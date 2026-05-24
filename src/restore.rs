use crate::backup::BackupManifest;
use crate::paths::backups_dir;
use crate::process::is_claude_desktop_running;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const CLAUDE_DESKTOP_RUNNING_MESSAGE: &str = "Claude Desktop appears to be running.
Quit Claude Desktop fully before restoring a backup.
Use --force-while-running only if you know what you are doing.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSummary {
    pub backup_path: PathBuf,
    pub restored_bucket: PathBuf,
    pub restored_file_count: usize,
}

pub fn restore_latest(relink_dir: &Path, force_while_running: bool) -> Result<RestoreSummary> {
    let backup_dir = latest_backup_dir(relink_dir)?;
    restore_backup(&backup_dir, force_while_running)
}

pub fn restore_backup(backup_dir: &Path, force_while_running: bool) -> Result<RestoreSummary> {
    if !force_while_running && is_claude_desktop_running()? {
        bail!(CLAUDE_DESKTOP_RUNNING_MESSAGE);
    }

    let manifest = read_manifest(backup_dir)?;
    validate_manifest(&manifest)?;
    let backup_bucket = backup_dir
        .join(&manifest.target_account_id)
        .join(&manifest.target_org_id);
    let restore_paths =
        validate_restore_paths(backup_dir, &backup_bucket, &manifest.desktop_bucket)?;

    let staging_dir = create_restore_staging_dir(&manifest.desktop_bucket)?;
    let restored_file_count = match copy_dir_contents(&backup_bucket, &staging_dir) {
        Ok(restored_file_count) => restored_file_count,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(error).context("failed to stage backup contents");
        }
    };

    if let Err(error) = replace_bucket_contents_from_staging(&manifest.desktop_bucket, &staging_dir)
    {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    let _ = fs::remove_dir_all(&staging_dir);

    Ok(RestoreSummary {
        backup_path: backup_dir.to_path_buf(),
        restored_bucket: restore_paths.target_bucket,
        restored_file_count,
    })
}

fn latest_backup_dir(relink_dir: &Path) -> Result<PathBuf> {
    let root = backups_dir(relink_dir);
    if !root
        .try_exists()
        .with_context(|| format!("failed to inspect {}", root.display()))?
    {
        bail!("no backups found under {}", root.display());
    }

    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", root.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?
            .is_dir()
        {
            continue;
        }

        let path = entry.path();
        let manifest_path = path.join("manifest.json");
        if !manifest_path
            .try_exists()
            .with_context(|| format!("failed to inspect {}", manifest_path.display()))?
        {
            continue;
        }
        let manifest_metadata = fs::symlink_metadata(&manifest_path)
            .with_context(|| format!("failed to inspect {}", manifest_path.display()))?;
        if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
            continue;
        }

        candidates.push((entry.file_name().to_string_lossy().to_string(), path));
    }

    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    candidates
        .pop()
        .map(|(_, path)| path)
        .ok_or_else(|| anyhow::anyhow!("no backups found under {}", root.display()))
}

fn read_manifest(backup_dir: &Path) -> Result<BackupManifest> {
    let manifest_path = backup_dir.join("manifest.json");
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))
}

fn validate_manifest(manifest: &BackupManifest) -> Result<()> {
    if manifest.operation != "sync" {
        bail!(
            "backup manifest operation is {}, expected sync",
            manifest.operation
        );
    }
    validate_path_component("targetAccountId", &manifest.target_account_id)?;
    validate_path_component("targetOrgId", &manifest.target_org_id)?;
    validate_desktop_bucket_path(manifest)?;
    Ok(())
}

fn validate_path_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("backup manifest {label} is empty");
    }

    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if component == std::ffi::OsStr::new(value) => {
            Ok(())
        }
        _ => bail!("backup manifest {label} is not a safe path component"),
    }
}

fn validate_desktop_bucket_path(manifest: &BackupManifest) -> Result<()> {
    if !manifest.desktop_bucket.is_absolute() {
        bail!("backup manifest desktopBucket must be an absolute path");
    }

    let org = manifest
        .desktop_bucket
        .file_name()
        .and_then(|name| name.to_str())
        .context("backup manifest desktopBucket has no organization component")?;
    if org != manifest.target_org_id {
        bail!("backup manifest desktopBucket does not end with targetOrgId");
    }

    let account = manifest
        .desktop_bucket
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("backup manifest desktopBucket has no account component")?;
    if account != manifest.target_account_id {
        bail!("backup manifest desktopBucket parent does not match targetAccountId");
    }

    let sessions_dir = manifest
        .desktop_bucket
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("backup manifest desktopBucket has no claude-code-sessions component")?;
    if sessions_dir != "claude-code-sessions" {
        bail!(
            "backup manifest desktopBucket must end with claude-code-sessions/{}/{}",
            manifest.target_account_id,
            manifest.target_org_id
        );
    }

    Ok(())
}

#[derive(Debug)]
struct RestorePaths {
    target_bucket: PathBuf,
}

fn validate_restore_paths(
    backup_dir: &Path,
    backup_bucket: &Path,
    target_bucket: &Path,
) -> Result<RestorePaths> {
    let canonical_backup_root = validate_backup_bucket(backup_dir, backup_bucket)?;
    let canonical_backup_bucket = backup_bucket
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", backup_bucket.display()))?;
    let canonical_target_bucket = canonicalize_intended_path(target_bucket)?;
    ensure_no_path_overlap(
        &canonical_backup_root,
        &canonical_backup_bucket,
        &canonical_target_bucket,
    )?;

    Ok(RestorePaths {
        target_bucket: target_bucket.to_path_buf(),
    })
}

fn validate_backup_bucket(backup_dir: &Path, backup_bucket: &Path) -> Result<PathBuf> {
    if !backup_bucket
        .try_exists()
        .with_context(|| format!("failed to inspect {}", backup_bucket.display()))?
    {
        bail!("backup bucket does not exist: {}", backup_bucket.display());
    }

    let metadata = fs::symlink_metadata(backup_bucket)
        .with_context(|| format!("failed to inspect {}", backup_bucket.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "backup bucket is not a directory: {}",
            backup_bucket.display()
        );
    }

    let backup_dir = backup_dir
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", backup_dir.display()))?;
    let canonical_backup_bucket = backup_bucket
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", backup_bucket.display()))?;
    if !canonical_backup_bucket.starts_with(&backup_dir) {
        bail!("backup bucket is outside backup directory");
    }

    Ok(backup_dir)
}

fn ensure_no_path_overlap(
    backup_root: &Path,
    backup_bucket: &Path,
    target_bucket: &Path,
) -> Result<()> {
    for (label, path) in [
        ("backup root", backup_root),
        ("backup bucket", backup_bucket),
    ] {
        if paths_overlap(path, target_bucket) {
            bail!(
                "backup and target paths overlap: {label} {} and target Desktop bucket {}",
                path.display(),
                target_bucket.display()
            );
        }
    }

    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn canonicalize_intended_path(path: &Path) -> Result<PathBuf> {
    if path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", path.display()))?
    {
        return path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", path.display()));
    }

    let mut missing_components = Vec::new();
    let mut current = path;
    loop {
        if current
            .try_exists()
            .with_context(|| format!("failed to inspect {}", current.display()))?
        {
            let mut canonical = current
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", current.display()))?;
            for component in missing_components.iter().rev() {
                canonical.push(component);
            }
            return Ok(canonical);
        }

        let name = current
            .file_name()
            .context("failed to find existing ancestor for target Desktop bucket")?;
        missing_components.push(name.to_os_string());
        current = current
            .parent()
            .context("failed to find existing ancestor for target Desktop bucket")?;
    }
}

fn create_restore_staging_dir(target_bucket: &Path) -> Result<PathBuf> {
    let parent = target_bucket
        .parent()
        .context("target Desktop bucket has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let prefix = format!(".claude-relink-restore-staging-{}", std::process::id());
    for suffix in 0..100 {
        let candidate = parent.join(format!("{prefix}-{suffix}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", candidate.display()));
            }
        }
    }

    bail!(
        "failed to create a unique restore staging directory under {}",
        parent.display()
    )
}

fn replace_bucket_contents_from_staging(target_bucket: &Path, staging_dir: &Path) -> Result<()> {
    if target_bucket
        .try_exists()
        .with_context(|| format!("failed to inspect {}", target_bucket.display()))?
    {
        let metadata = fs::symlink_metadata(target_bucket)
            .with_context(|| format!("failed to inspect {}", target_bucket.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "target Desktop bucket is not a directory: {}",
                target_bucket.display()
            );
        }

        for entry in fs::read_dir(target_bucket)
            .with_context(|| format!("failed to read {}", target_bucket.display()))?
        {
            let entry = entry
                .with_context(|| format!("failed to read entry in {}", target_bucket.display()))?;
            remove_child(&entry.path())?;
        }
    } else {
        fs::create_dir_all(target_bucket)
            .with_context(|| format!("failed to create {}", target_bucket.display()))?;
    }

    fs::create_dir_all(target_bucket)
        .with_context(|| format!("failed to create {}", target_bucket.display()))?;
    move_dir_contents(staging_dir, target_bucket)?;
    Ok(())
}

fn move_dir_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source.display()))?;
        let target_path = destination.join(entry.file_name());
        fs::rename(entry.path(), &target_path).with_context(|| {
            format!(
                "failed to move {} to {}",
                entry.path().display(),
                target_path.display()
            )
        })?;
    }

    Ok(())
}

fn remove_child(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<usize> {
    let mut file_count = 0;
    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry.with_context(|| format!("failed to walk {}", source.display()))?;
        let relative_path = entry.path().strip_prefix(source).with_context(|| {
            format!(
                "failed to calculate relative path for {}",
                entry.path().display()
            )
        })?;
        let target_path = destination.join(relative_path);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("failed to create {}", target_path.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(entry.path(), &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    target_path.display()
                )
            })?;
            file_count += 1;
        }
    }

    Ok(file_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn manifest_for(desktop_bucket: PathBuf) -> BackupManifest {
        BackupManifest {
            created_at: Utc::now(),
            tool_version: "0.1.0".to_string(),
            operation: "sync".to_string(),
            target_account_id: "current".to_string(),
            target_org_id: "org".to_string(),
            desktop_bucket,
            created_files: Vec::new(),
            skipped_existing: Vec::new(),
        }
    }

    fn write_manifest(backup_dir: &Path, desktop_bucket: &Path) {
        let manifest = manifest_for(desktop_bucket.to_path_buf());
        let text = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(backup_dir.join("manifest.json"), format!("{text}\n")).unwrap();
    }

    #[test]
    fn latest_backup_dir_uses_lexicographically_newest_directory_name() {
        let temp = tempdir().unwrap();
        let backups = backups_dir(temp.path());
        fs::create_dir_all(backups.join("2026-05-24T120000000000000Z")).unwrap();
        fs::create_dir_all(backups.join("2026-05-24T130000000000000Z")).unwrap();
        fs::write(
            backups.join("2026-05-24T130000000000000Z/manifest.json"),
            "{}",
        )
        .unwrap();
        fs::write(backups.join("2026-05-24T140000000000000Z"), "not a dir").unwrap();

        let latest = latest_backup_dir(temp.path()).unwrap();

        assert_eq!(
            latest.file_name().unwrap().to_string_lossy(),
            "2026-05-24T130000000000000Z"
        );
    }

    #[test]
    fn latest_backup_dir_ignores_newer_directories_without_manifest() {
        let temp = tempdir().unwrap();
        let backups = backups_dir(temp.path());
        fs::create_dir_all(backups.join("2026-05-24T120000000000000Z")).unwrap();
        fs::write(
            backups.join("2026-05-24T120000000000000Z/manifest.json"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(backups.join("2026-05-24T130000000000000Z")).unwrap();

        let latest = latest_backup_dir(temp.path()).unwrap();

        assert_eq!(
            latest.file_name().unwrap().to_string_lossy(),
            "2026-05-24T120000000000000Z"
        );
    }

    #[test]
    fn validates_manifest_bucket_matches_account_and_org_components() {
        let manifest = manifest_for(PathBuf::from("/tmp/claude-code-sessions/current/wrong"));

        let error = validate_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("targetOrgId"));
    }

    #[test]
    fn rejects_manifest_bucket_without_claude_code_sessions_component() {
        let manifest = manifest_for(PathBuf::from("/tmp/something/current/org"));

        let error = validate_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("claude-code-sessions"));
    }

    #[test]
    fn restore_rejects_target_bucket_inside_backup_root() {
        let temp = tempdir().unwrap();
        let backup_dir = temp.path().join("backup");
        let backup_bucket = backup_dir.join("current/org");
        let target_bucket = backup_dir.join("claude-code-sessions/current/org");
        fs::create_dir_all(&backup_bucket).unwrap();
        fs::write(backup_bucket.join("local_old.json"), "{}").unwrap();
        write_manifest(&backup_dir, &target_bucket);

        let error = restore_backup(&backup_dir, true).unwrap_err();

        assert!(error.to_string().contains("overlap"));
        assert!(backup_bucket.join("local_old.json").exists());
    }

    #[test]
    fn restore_rejects_backup_root_inside_target_before_clearing_target() {
        let temp = tempdir().unwrap();
        let target_bucket = temp.path().join("desktop/claude-code-sessions/current/org");
        let backup_dir = target_bucket.join("backups/2026-05-24T120000000000000Z");
        let backup_bucket = backup_dir.join("current/org");
        fs::create_dir_all(&backup_bucket).unwrap();
        fs::write(target_bucket.join("local_keep.json"), "{}").unwrap();
        fs::write(backup_bucket.join("local_old.json"), "{}").unwrap();
        write_manifest(&backup_dir, &target_bucket);

        let error = restore_backup(&backup_dir, true).unwrap_err();

        assert!(error.to_string().contains("overlap"));
        assert!(target_bucket.join("local_keep.json").exists());
        assert!(backup_bucket.join("local_old.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_does_not_clear_target_when_staging_copy_fails() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().unwrap();
        let backup_dir = temp.path().join("backup");
        let backup_bucket = backup_dir.join("current/org");
        let target_bucket = temp.path().join("desktop/claude-code-sessions/current/org");
        fs::create_dir_all(&backup_bucket).unwrap();
        fs::create_dir_all(&target_bucket).unwrap();
        fs::write(target_bucket.join("local_keep.json"), "{}").unwrap();
        fs::write(backup_bucket.join("local_old.json"), "{}").unwrap();
        let unreadable = backup_bucket.join("local_unreadable.json");
        fs::write(&unreadable, "{}").unwrap();
        let mut permissions = fs::metadata(&unreadable).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&unreadable, permissions).unwrap();
        write_manifest(&backup_dir, &target_bucket);

        let error = restore_backup(&backup_dir, true).unwrap_err();

        let mut permissions = fs::metadata(&unreadable).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&unreadable, permissions).unwrap();

        assert!(format!("{error:#}").contains("local_unreadable.json"));
        assert!(target_bucket.join("local_keep.json").exists());
        assert!(backup_bucket.join("local_old.json").exists());
        assert!(backup_bucket.join("local_unreadable.json").exists());
    }
}

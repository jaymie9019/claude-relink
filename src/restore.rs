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
    validate_backup_bucket(backup_dir, &backup_bucket)?;

    replace_bucket_contents(&manifest.desktop_bucket, &backup_bucket)?;
    let restored_file_count = copy_dir_contents(&backup_bucket, &manifest.desktop_bucket)?;

    Ok(RestoreSummary {
        backup_path: backup_dir.to_path_buf(),
        restored_bucket: manifest.desktop_bucket,
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

        candidates.push((
            entry.file_name().to_string_lossy().to_string(),
            entry.path(),
        ));
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

    Ok(())
}

fn validate_backup_bucket(backup_dir: &Path, backup_bucket: &Path) -> Result<()> {
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
    let backup_bucket = backup_bucket
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", backup_bucket.display()))?;
    if !backup_bucket.starts_with(&backup_dir) {
        bail!("backup bucket is outside backup directory");
    }

    Ok(())
}

fn replace_bucket_contents(target_bucket: &Path, backup_bucket: &Path) -> Result<()> {
    if target_bucket
        .try_exists()
        .with_context(|| format!("failed to inspect {}", target_bucket.display()))?
    {
        validate_distinct_paths(target_bucket, backup_bucket)?;
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
    Ok(())
}

fn validate_distinct_paths(target_bucket: &Path, backup_bucket: &Path) -> Result<()> {
    let target_bucket = target_bucket
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", target_bucket.display()))?;
    let backup_bucket = backup_bucket
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", backup_bucket.display()))?;
    if target_bucket == backup_bucket {
        bail!("target Desktop bucket and backup bucket must be different paths");
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

    #[test]
    fn latest_backup_dir_uses_lexicographically_newest_directory_name() {
        let temp = tempdir().unwrap();
        let backups = backups_dir(temp.path());
        fs::create_dir_all(backups.join("2026-05-24T120000000000000Z")).unwrap();
        fs::create_dir_all(backups.join("2026-05-24T130000000000000Z")).unwrap();
        fs::write(backups.join("2026-05-24T140000000000000Z"), "not a dir").unwrap();

        let latest = latest_backup_dir(temp.path()).unwrap();

        assert_eq!(
            latest.file_name().unwrap().to_string_lossy(),
            "2026-05-24T130000000000000Z"
        );
    }

    #[test]
    fn validates_manifest_bucket_matches_account_and_org_components() {
        let manifest = BackupManifest {
            created_at: Utc::now(),
            tool_version: "0.1.0".to_string(),
            operation: "sync".to_string(),
            target_account_id: "current".to_string(),
            target_org_id: "org".to_string(),
            desktop_bucket: PathBuf::from("/tmp/claude-code-sessions/current/wrong"),
            created_files: Vec::new(),
            skipped_existing: Vec::new(),
        };

        let error = validate_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("targetOrgId"));
    }
}

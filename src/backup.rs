use crate::paths::{backups_dir, DesktopBucket};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Backup {
    pub root_path: PathBuf,
    pub bucket_path: PathBuf,
    pub manifest_path: PathBuf,
    created_at: DateTime<Utc>,
    target_account_id: String,
    target_org_id: String,
    desktop_bucket: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub created_at: DateTime<Utc>,
    pub tool_version: String,
    pub operation: String,
    pub target_account_id: String,
    pub target_org_id: String,
    pub desktop_bucket: PathBuf,
    pub created_files: Vec<String>,
    pub skipped_existing: Vec<String>,
}

pub fn create_sync_backup(relink_dir: &Path, target_bucket: &DesktopBucket) -> Result<Backup> {
    if !target_bucket
        .path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", target_bucket.path.display()))?
    {
        bail!(
            "target Desktop bucket does not exist: {}",
            target_bucket.path.display()
        );
    }

    let created_at = Utc::now();
    let root_path = unique_backup_root(relink_dir, created_at)?;
    let bucket_path = root_path
        .join(&target_bucket.account_id)
        .join(&target_bucket.org_id);
    fs::create_dir_all(&bucket_path)
        .with_context(|| format!("failed to create {}", bucket_path.display()))?;
    copy_dir_contents(&target_bucket.path, &bucket_path)?;

    Ok(Backup {
        manifest_path: root_path.join("manifest.json"),
        root_path,
        bucket_path,
        created_at,
        target_account_id: target_bucket.account_id.clone(),
        target_org_id: target_bucket.org_id.clone(),
        desktop_bucket: target_bucket.path.clone(),
    })
}

pub fn write_sync_manifest(
    backup: &Backup,
    created_files: &[String],
    skipped_existing: &[String],
) -> Result<BackupManifest> {
    let manifest = BackupManifest {
        created_at: backup.created_at,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        operation: "sync".to_string(),
        target_account_id: backup.target_account_id.clone(),
        target_org_id: backup.target_org_id.clone(),
        desktop_bucket: backup.desktop_bucket.clone(),
        created_files: created_files.to_vec(),
        skipped_existing: skipped_existing.to_vec(),
    };
    let text = serde_json::to_string_pretty(&manifest).context("failed to serialize manifest")?;
    fs::write(&backup.manifest_path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", backup.manifest_path.display()))?;
    Ok(manifest)
}

fn unique_backup_root(relink_dir: &Path, created_at: DateTime<Utc>) -> Result<PathBuf> {
    let root = backups_dir(relink_dir);
    fs::create_dir_all(&root).with_context(|| format!("failed to create {}", root.display()))?;

    let timestamp = created_at.format("%Y-%m-%dT%H%M%S%fZ").to_string();
    for suffix in 0..100 {
        let candidate = if suffix == 0 {
            root.join(&timestamp)
        } else {
            root.join(format!("{timestamp}-{suffix}"))
        };
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
        "failed to create a unique backup directory under {}",
        root.display()
    )
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<()> {
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
        }
    }

    Ok(())
}

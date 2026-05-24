use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopBucket {
    pub account_id: String,
    pub org_id: String,
    pub path: PathBuf,
    pub local_index_count: usize,
}

pub fn default_claude_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".claude"))
        .ok_or_else(|| anyhow!("could not determine home directory"))
}

pub fn default_desktop_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Library/Application Support/Claude"))
        .ok_or_else(|| anyhow!("could not determine home directory"))
}

pub fn default_relink_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".claude-relink"))
        .ok_or_else(|| anyhow!("could not determine home directory"))
}

pub fn library_dir(relink_dir: &Path) -> PathBuf {
    relink_dir.join("library")
}

pub fn backups_dir(relink_dir: &Path) -> PathBuf {
    relink_dir.join("backups")
}

pub fn desktop_sessions_root(desktop_dir: &Path) -> PathBuf {
    desktop_dir.join("claude-code-sessions")
}

pub fn read_owner_account_id(desktop_dir: &Path) -> Result<Option<String>> {
    let config_path = desktop_dir.join("cowork-enabled-cli-ops.json");
    if !config_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;

    Ok(value
        .get("ownerAccountId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

pub fn list_desktop_buckets(desktop_dir: &Path) -> Result<Vec<DesktopBucket>> {
    let root = desktop_sessions_root(desktop_dir);
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut buckets = Vec::new();
    for account_entry in
        fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let account_entry =
            account_entry.with_context(|| format!("failed to read entry in {}", root.display()))?;
        if !account_entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", account_entry.path().display()))?
            .is_dir()
        {
            continue;
        }

        let account_id = account_entry.file_name().to_string_lossy().to_string();
        let account_path = account_entry.path();
        for org_entry in fs::read_dir(&account_path)
            .with_context(|| format!("failed to read {}", account_path.display()))?
        {
            let org_entry = org_entry
                .with_context(|| format!("failed to read entry in {}", account_path.display()))?;
            if !org_entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", org_entry.path().display()))?
                .is_dir()
            {
                continue;
            }

            let org_id = org_entry.file_name().to_string_lossy().to_string();
            let path = org_entry.path();
            let local_index_count = count_local_indexes(&path)?;
            buckets.push(DesktopBucket {
                account_id: account_id.clone(),
                org_id,
                path,
                local_index_count,
            });
        }
    }

    buckets.sort_by(|left, right| {
        (&left.account_id, &left.org_id).cmp(&(&right.account_id, &right.org_id))
    });
    Ok(buckets)
}

pub fn resolve_target_bucket(
    desktop_dir: &Path,
    account_id: Option<&str>,
    org_id: Option<&str>,
) -> Result<DesktopBucket> {
    let buckets = list_desktop_buckets(desktop_dir)?;
    if buckets.is_empty() {
        bail!(
            "no Claude Desktop buckets found under {}",
            desktop_sessions_root(desktop_dir).display()
        );
    }

    match (account_id, org_id) {
        (Some(account_id), Some(org_id)) => {
            return buckets
                .into_iter()
                .find(|bucket| bucket.account_id == account_id && bucket.org_id == org_id)
                .ok_or_else(|| {
                    anyhow!("Desktop bucket not found for account {account_id} and org {org_id}")
                });
        }
        (None, None) => {}
        _ => bail!("pass both --account-id and --org-id, or pass neither"),
    }

    if let Some(owner_account_id) = read_owner_account_id(desktop_dir)? {
        let owner_buckets = buckets
            .iter()
            .filter(|bucket| bucket.account_id == owner_account_id)
            .collect::<Vec<_>>();

        match owner_buckets.as_slice() {
            [bucket] => return Ok((*bucket).clone()),
            [] => {}
            _ => bail!(
                "owner account {owner_account_id} has multiple organization buckets; pass --account-id and --org-id"
            ),
        }
    }

    if buckets.len() == 1 {
        return Ok(buckets[0].clone());
    }

    bail!("multiple Desktop buckets found; pass --account-id and --org-id")
}

fn count_local_indexes(bucket_path: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(bucket_path)
        .with_context(|| format!("failed to read {}", bucket_path.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", bucket_path.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?
            .is_file()
        {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("local_") && name.ends_with(".json") {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn create_bucket(desktop_dir: &Path, account_id: &str, org_id: &str) {
        fs::create_dir_all(
            desktop_dir
                .join("claude-code-sessions")
                .join(account_id)
                .join(org_id),
        )
        .unwrap();
    }

    #[test]
    fn owner_account_selects_single_org_bucket() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("cowork-enabled-cli-ops.json"),
            r#"{"ownerAccountId":"current"}"#,
        )
        .unwrap();
        create_bucket(temp.path(), "current", "org");
        create_bucket(temp.path(), "old", "org");

        let bucket = resolve_target_bucket(temp.path(), None, None).unwrap();

        assert_eq!(bucket.account_id, "current");
        assert_eq!(bucket.org_id, "org");
        assert_eq!(
            bucket.path,
            temp.path()
                .join("claude-code-sessions")
                .join("current")
                .join("org")
        );
    }

    #[test]
    fn explicit_pair_selects_exact_bucket() {
        let temp = tempdir().unwrap();
        create_bucket(temp.path(), "current", "org");
        create_bucket(temp.path(), "old", "old-org");

        let bucket = resolve_target_bucket(temp.path(), Some("old"), Some("old-org")).unwrap();

        assert_eq!(bucket.account_id, "old");
        assert_eq!(bucket.org_id, "old-org");
    }

    #[test]
    fn multiple_buckets_without_owner_is_ambiguous() {
        let temp = tempdir().unwrap();
        create_bucket(temp.path(), "account-a", "org");
        create_bucket(temp.path(), "account-b", "org");

        let error = resolve_target_bucket(temp.path(), None, None).unwrap_err();

        assert!(error.to_string().contains("multiple Desktop buckets found"));
    }

    #[test]
    fn partial_explicit_pair_errors() {
        let temp = tempdir().unwrap();
        create_bucket(temp.path(), "current", "org");

        let error = resolve_target_bucket(temp.path(), Some("current"), None).unwrap_err();

        assert!(error
            .to_string()
            .contains("pass both --account-id and --org-id"));
    }

    #[test]
    fn lists_buckets_sorted_and_counts_local_index_files() {
        let temp = tempdir().unwrap();
        create_bucket(temp.path(), "b-account", "z-org");
        create_bucket(temp.path(), "a-account", "y-org");

        let b_bucket = temp
            .path()
            .join("claude-code-sessions")
            .join("b-account")
            .join("z-org");
        fs::write(b_bucket.join("local_two.json"), "{}").unwrap();
        fs::write(b_bucket.join("local_one.json"), "{}").unwrap();
        fs::write(b_bucket.join("local_one.txt"), "{}").unwrap();
        fs::create_dir_all(b_bucket.join("local_directory.json")).unwrap();

        let a_bucket = temp
            .path()
            .join("claude-code-sessions")
            .join("a-account")
            .join("y-org");
        fs::write(a_bucket.join("local_alpha.json"), "{}").unwrap();

        let buckets = list_desktop_buckets(temp.path()).unwrap();

        assert_eq!(
            buckets
                .iter()
                .map(|bucket| (
                    bucket.account_id.as_str(),
                    bucket.org_id.as_str(),
                    bucket.local_index_count,
                ))
                .collect::<Vec<_>>(),
            vec![("a-account", "y-org", 1), ("b-account", "z-org", 2)]
        );
    }
}

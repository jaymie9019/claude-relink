use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DesktopIndex {
    pub session_id: String,
    pub cli_session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub origin_cwd: Option<PathBuf>,
    pub path: PathBuf,
    pub raw: Value,
}

pub fn scan_desktop_indexes(bucket: &Path) -> Result<Vec<DesktopIndex>> {
    if !bucket
        .try_exists()
        .with_context(|| format!("failed to inspect {}", bucket.display()))?
    {
        return Ok(Vec::new());
    }

    let mut indexes = Vec::new();
    for entry in
        fs::read_dir(bucket).with_context(|| format!("failed to read {}", bucket.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", bucket.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?
            .is_file()
        {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("local_") || !name.ends_with(".json") {
            continue;
        }

        let path = entry.path();
        let raw_text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let raw: Value = match serde_json::from_str(&raw_text) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let session_id = raw
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| file_stem(&path));

        indexes.push(DesktopIndex {
            session_id,
            cli_session_id: raw
                .get("cliSessionId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            cwd: raw.get("cwd").and_then(Value::as_str).map(PathBuf::from),
            origin_cwd: raw
                .get("originCwd")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            path,
            raw,
        });
    }

    indexes.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(indexes)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scan_desktop_indexes_reads_local_json_and_ignores_other_entries() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("local_directory.json")).unwrap();
        fs::write(
            temp.path().join("local_b.json"),
            r#"{"sessionId":"local_b","cliSessionId":"cli-b","cwd":"/tmp/b","originCwd":"/tmp/origin-b"}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("local_a.json"),
            r#"{"sessionId":"local_a","cliSessionId":"cli-a","cwd":"/tmp/a"}"#,
        )
        .unwrap();
        fs::write(temp.path().join("remote.json"), r#"{"sessionId":"remote"}"#).unwrap();
        fs::write(temp.path().join("local_not_json.txt"), "{}").unwrap();

        let indexes = scan_desktop_indexes(temp.path()).unwrap();

        assert_eq!(indexes.len(), 2);
        assert_eq!(indexes[0].session_id, "local_a");
        assert_eq!(indexes[0].cli_session_id.as_deref(), Some("cli-a"));
        assert_eq!(
            indexes[0].cwd.as_deref(),
            Some(std::path::Path::new("/tmp/a"))
        );
        assert_eq!(indexes[0].origin_cwd, None);
        assert_eq!(indexes[1].session_id, "local_b");
        assert_eq!(indexes[1].cli_session_id.as_deref(), Some("cli-b"));
        assert_eq!(
            indexes[1].origin_cwd.as_deref(),
            Some(std::path::Path::new("/tmp/origin-b"))
        );
    }

    #[test]
    fn scan_desktop_indexes_falls_back_to_file_stem_when_session_id_is_missing() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("local_fallback.json"),
            r#"{"cliSessionId":"cli-fallback"}"#,
        )
        .unwrap();

        let indexes = scan_desktop_indexes(temp.path()).unwrap();

        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].session_id, "local_fallback");
        assert_eq!(indexes[0].cli_session_id.as_deref(), Some("cli-fallback"));
    }

    #[test]
    fn scan_desktop_indexes_skips_malformed_local_json() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("local_valid.json"),
            r#"{"sessionId":"local_valid","cliSessionId":"cli-valid"}"#,
        )
        .unwrap();
        fs::write(temp.path().join("local_malformed.json"), "{").unwrap();

        let indexes = scan_desktop_indexes(temp.path()).unwrap();

        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].session_id, "local_valid");
        assert_eq!(indexes[0].cli_session_id.as_deref(), Some("cli-valid"));
    }
}

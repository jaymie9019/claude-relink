use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

pub fn build_index_for_current_account(session: &crate::library::LibrarySession) -> Value {
    let session_id = format!("local_{}", Uuid::new_v4());
    let mut object = session
        .raw_index_template
        .as_object()
        .cloned()
        .unwrap_or_default();

    object.insert("sessionId".to_string(), json!(session_id));
    object.insert("cliSessionId".to_string(), json!(session.cli_session_id));
    object.insert("cwd".to_string(), json!(session.cwd_string()));
    object.insert("originCwd".to_string(), json!(session.origin_cwd_string()));
    object.insert(
        "createdAt".to_string(),
        json!(session.created_at_ms.unwrap_or(0)),
    );
    object.insert(
        "lastActivityAt".to_string(),
        json!(session
            .last_activity_at_ms
            .unwrap_or(session.created_at_ms.unwrap_or(0))),
    );
    object.insert(
        "lastFocusedAt".to_string(),
        json!(session.last_focused_at_ms.unwrap_or(
            session
                .last_activity_at_ms
                .unwrap_or(session.created_at_ms.unwrap_or(0))
        )),
    );
    object.insert("title".to_string(), json!(session.title_or_fallback()));
    object.insert("isArchived".to_string(), json!(false));
    object
        .entry("titleSource".to_string())
        .or_insert_with(|| json!("auto"));

    Value::Object(object)
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

    #[test]
    fn build_index_for_current_account_creates_new_local_session_id_and_preserves_cli_session_id() {
        let session = crate::library::LibrarySession {
            cli_session_id: "cli-original".to_string(),
            transcript_path: None,
            cwd: Some(PathBuf::from("/project/current")),
            origin_cwd: Some(PathBuf::from("/project/origin")),
            title: Some("Visible session".to_string()),
            created_at_ms: Some(1000),
            last_activity_at_ms: Some(2000),
            last_focused_at_ms: Some(3000),
            completed_turns: Some(12),
            source_indexes: Vec::new(),
            raw_index_template: serde_json::json!({
                "sessionId": "local_old",
                "cliSessionId": "cli-old",
                "cwd": "/project/old",
                "originCwd": "/project/old-origin",
                "createdAt": 1,
                "lastActivityAt": 2,
                "lastFocusedAt": 3,
                "title": "Old title",
                "titleSource": "manual",
                "isArchived": true,
                "permissionMode": "bypassPermissions"
            }),
            updated_at: chrono::Utc::now(),
        };

        let rebuilt = build_index_for_current_account(&session);

        let session_id = rebuilt["sessionId"].as_str().unwrap();
        assert!(session_id.starts_with("local_"));
        assert_ne!(session_id, "local_old");
        assert_eq!(rebuilt["cliSessionId"], "cli-original");
        assert_eq!(rebuilt["cwd"], "/project/current");
        assert_eq!(rebuilt["originCwd"], "/project/origin");
        assert_eq!(rebuilt["createdAt"], 1000);
        assert_eq!(rebuilt["lastActivityAt"], 2000);
        assert_eq!(rebuilt["lastFocusedAt"], 3000);
        assert_eq!(rebuilt["title"], "Visible session");
        assert_eq!(rebuilt["titleSource"], "manual");
        assert_eq!(rebuilt["isArchived"], false);
        assert_eq!(rebuilt["permissionMode"], "bypassPermissions");
    }

    #[test]
    fn build_index_for_current_account_fills_title_source_when_missing() {
        let session = crate::library::LibrarySession {
            cli_session_id: "cli-title-source".to_string(),
            transcript_path: None,
            cwd: None,
            origin_cwd: None,
            title: None,
            created_at_ms: None,
            last_activity_at_ms: None,
            last_focused_at_ms: None,
            completed_turns: None,
            source_indexes: Vec::new(),
            raw_index_template: serde_json::json!({}),
            updated_at: chrono::Utc::now(),
        };

        let rebuilt = build_index_for_current_account(&session);

        assert_eq!(rebuilt["titleSource"], "auto");
        assert_eq!(rebuilt["title"], "Recovered cli-titl");
        assert_eq!(rebuilt["createdAt"], 0);
        assert_eq!(rebuilt["lastActivityAt"], 0);
        assert_eq!(rebuilt["lastFocusedAt"], 0);
    }
}

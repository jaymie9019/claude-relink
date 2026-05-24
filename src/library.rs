use crate::desktop_index::DesktopIndex;
use crate::paths::DesktopBucket;
use crate::transcript::TranscriptRef;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceIndex {
    pub account_id: String,
    pub org_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySession {
    pub cli_session_id: String,
    pub transcript_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub origin_cwd: Option<PathBuf>,
    pub title: Option<String>,
    #[serde(rename = "createdAt", alias = "createdAtMs")]
    pub created_at_ms: Option<i64>,
    #[serde(rename = "lastActivityAt", alias = "lastActivityAtMs")]
    pub last_activity_at_ms: Option<i64>,
    #[serde(rename = "lastFocusedAt", alias = "lastFocusedAtMs")]
    pub last_focused_at_ms: Option<i64>,
    pub completed_turns: Option<u32>,
    pub source_indexes: Vec<SourceIndex>,
    pub raw_index_template: Value,
    pub updated_at: DateTime<Utc>,
}

impl LibrarySession {
    pub fn cwd_string(&self) -> String {
        self.cwd
            .as_ref()
            .or(self.origin_cwd.as_ref())
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn origin_cwd_string(&self) -> String {
        self.origin_cwd
            .as_ref()
            .or(self.cwd.as_ref())
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn title_or_fallback(&self) -> String {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                let prefix = self.cli_session_id.chars().take(8).collect::<String>();
                format!("Recovered {prefix}")
            })
    }
}

pub fn sessions_path(library_dir: &Path) -> PathBuf {
    library_dir.join("sessions.jsonl")
}

pub fn refresh_library(
    library_dir: &Path,
    bucket_indexes: &[(DesktopBucket, Vec<DesktopIndex>)],
    transcripts: &[TranscriptRef],
) -> Result<Vec<LibrarySession>> {
    let now = Utc::now();
    let transcript_by_id: BTreeMap<_, _> = transcripts
        .iter()
        .map(|transcript| (transcript.cli_session_id.clone(), transcript.path.clone()))
        .collect();
    let mut by_id: BTreeMap<String, LibrarySession> = BTreeMap::new();
    let mut title_activity_by_id: BTreeMap<String, i64> = BTreeMap::new();

    for (bucket, indexes) in bucket_indexes {
        for index in indexes {
            let Some(cli_session_id) = index.cli_session_id.clone() else {
                continue;
            };
            let index_activity_key = index_activity_key(index);
            let index_title = title_field(&index.raw);

            let candidate = session_from_index(
                cli_session_id.clone(),
                transcript_by_id.get(&cli_session_id).cloned(),
                index,
                now,
            );

            let entry = by_id.entry(cli_session_id).or_insert(candidate);
            if index_activity_key > session_activity_key(entry) {
                update_session_from_index(entry, index);
            }
            if let Some(title) = index_title {
                let current_title_activity = title_activity_by_id
                    .get(&entry.cli_session_id)
                    .copied()
                    .unwrap_or(i64::MIN);
                if index_activity_key > current_title_activity {
                    entry.title = Some(title);
                    title_activity_by_id.insert(entry.cli_session_id.clone(), index_activity_key);
                }
            }
            if entry.transcript_path.is_none() {
                entry.transcript_path = transcript_by_id.get(&entry.cli_session_id).cloned();
            }
            entry.source_indexes.push(SourceIndex {
                account_id: bucket.account_id.clone(),
                org_id: bucket.org_id.clone(),
                path: index.path.clone(),
            });
            entry.updated_at = now;
        }
    }

    for transcript in transcripts {
        by_id
            .entry(transcript.cli_session_id.clone())
            .and_modify(|session| {
                if session.transcript_path.is_none() {
                    session.transcript_path = Some(transcript.path.clone());
                    session.updated_at = now;
                }
            })
            .or_insert_with(|| LibrarySession {
                cli_session_id: transcript.cli_session_id.clone(),
                transcript_path: Some(transcript.path.clone()),
                cwd: None,
                origin_cwd: None,
                title: None,
                created_at_ms: None,
                last_activity_at_ms: None,
                last_focused_at_ms: None,
                completed_turns: None,
                source_indexes: Vec::new(),
                raw_index_template: Value::Object(Default::default()),
                updated_at: now,
            });
    }

    let sessions: Vec<_> = by_id.into_values().collect();
    write_sessions(library_dir, &sessions)?;
    Ok(sessions)
}

pub fn write_sessions(library_dir: &Path, sessions: &[LibrarySession]) -> Result<()> {
    fs::create_dir_all(library_dir)
        .with_context(|| format!("failed to create {}", library_dir.display()))?;
    let mut lines = Vec::with_capacity(sessions.len());
    for session in sessions {
        lines.push(serde_json::to_string(session).context("failed to serialize library session")?);
    }
    let text = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(sessions_path(library_dir), text)
        .with_context(|| format!("failed to write {}", sessions_path(library_dir).display()))?;
    Ok(())
}

pub fn read_sessions(library_dir: &Path) -> Result<Vec<LibrarySession>> {
    let path = sessions_path(library_dir);
    if !path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", path.display()))?
    {
        return Ok(Vec::new());
    }

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut sessions = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        sessions.push(serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse {} line {}",
                path.display(),
                line_number + 1
            )
        })?);
    }
    Ok(sessions)
}

fn session_from_index(
    cli_session_id: String,
    transcript_path: Option<PathBuf>,
    index: &DesktopIndex,
    updated_at: DateTime<Utc>,
) -> LibrarySession {
    LibrarySession {
        cli_session_id,
        transcript_path,
        cwd: index.cwd.clone(),
        origin_cwd: index.origin_cwd.clone(),
        title: title_field(&index.raw),
        created_at_ms: i64_field(&index.raw, "createdAt"),
        last_activity_at_ms: i64_field(&index.raw, "lastActivityAt"),
        last_focused_at_ms: i64_field(&index.raw, "lastFocusedAt"),
        completed_turns: index
            .raw
            .get("completedTurns")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        source_indexes: Vec::new(),
        raw_index_template: index.raw.clone(),
        updated_at,
    }
}

fn update_session_from_index(session: &mut LibrarySession, index: &DesktopIndex) {
    session.cwd = index.cwd.clone();
    session.origin_cwd = index.origin_cwd.clone();
    session.created_at_ms = i64_field(&index.raw, "createdAt");
    session.last_activity_at_ms = i64_field(&index.raw, "lastActivityAt");
    session.last_focused_at_ms = i64_field(&index.raw, "lastFocusedAt");
    session.completed_turns = index
        .raw
        .get("completedTurns")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    session.raw_index_template = index.raw.clone();
}

fn session_activity_key(session: &LibrarySession) -> i64 {
    session
        .last_activity_at_ms
        .or(session.last_focused_at_ms)
        .or(session.created_at_ms)
        .unwrap_or(i64::MIN)
}

fn index_activity_key(index: &DesktopIndex) -> i64 {
    latest_activity_key(
        index.raw.get("lastActivityAt"),
        index.raw.get("lastFocusedAt"),
        index.raw.get("createdAt"),
    )
}

fn latest_activity_key(
    last_activity_at: Option<&Value>,
    last_focused_at: Option<&Value>,
    created_at: Option<&Value>,
) -> i64 {
    last_activity_at
        .and_then(Value::as_i64)
        .or_else(|| last_focused_at.and_then(Value::as_i64))
        .or_else(|| created_at.and_then(Value::as_i64))
        .unwrap_or(i64::MIN)
}

fn i64_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn title_field(value: &Value) -> Option<String> {
    value
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_index::DesktopIndex;
    use crate::paths::DesktopBucket;
    use crate::transcript::TranscriptRef;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn bucket(root: &Path, account_id: &str, org_id: &str) -> DesktopBucket {
        DesktopBucket {
            account_id: account_id.to_string(),
            org_id: org_id.to_string(),
            path: root.join(account_id).join(org_id),
            local_index_count: 0,
        }
    }

    fn index(path: PathBuf, cli_session_id: Option<&str>, raw: serde_json::Value) -> DesktopIndex {
        DesktopIndex {
            session_id: raw
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("local_source")
                .to_string(),
            cli_session_id: cli_session_id.map(ToOwned::to_owned),
            cwd: raw
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from),
            origin_cwd: raw
                .get("originCwd")
                .and_then(serde_json::Value::as_str)
                .map(PathBuf::from),
            path,
            raw,
        }
    }

    #[test]
    fn refresh_library_merges_multiple_source_indexes_by_cli_session_id() {
        let temp = tempdir().unwrap();
        let source_a = bucket(temp.path(), "account-a", "org-a");
        let source_b = bucket(temp.path(), "account-b", "org-b");
        let index_a_path = temp.path().join("account-a/org-a/local_a.json");
        let index_b_path = temp.path().join("account-b/org-b/local_b.json");
        let transcript_path = temp.path().join("projects/session-1.jsonl");
        let indexes = vec![
            (
                source_a,
                vec![index(
                    index_a_path.clone(),
                    Some("session-1"),
                    json!({
                        "sessionId": "local_a",
                        "cliSessionId": "session-1",
                        "cwd": "/project/a",
                        "createdAt": 10,
                        "lastActivityAt": 20,
                        "title": "First source"
                    }),
                )],
            ),
            (
                source_b,
                vec![index(
                    index_b_path.clone(),
                    Some("session-1"),
                    json!({
                        "sessionId": "local_b",
                        "cliSessionId": "session-1",
                        "cwd": "/project/b",
                        "createdAt": 30,
                        "lastActivityAt": 40,
                        "title": "Second source"
                    }),
                )],
            ),
        ];
        let transcripts = vec![TranscriptRef {
            cli_session_id: "session-1".to_string(),
            path: transcript_path.clone(),
        }];

        let sessions = refresh_library(temp.path(), &indexes, &transcripts).unwrap();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.cli_session_id, "session-1");
        assert_eq!(session.transcript_path.as_ref(), Some(&transcript_path));
        assert_eq!(session.source_indexes.len(), 2);
        assert_eq!(session.source_indexes[0].account_id, "account-a");
        assert_eq!(session.source_indexes[0].org_id, "org-a");
        assert_eq!(session.source_indexes[0].path, index_a_path);
        assert_eq!(session.source_indexes[1].account_id, "account-b");
        assert_eq!(session.source_indexes[1].org_id, "org-b");
        assert_eq!(session.source_indexes[1].path, index_b_path);
    }

    #[test]
    fn refresh_library_includes_transcript_only_records() {
        let temp = tempdir().unwrap();
        let transcript_path = temp.path().join("projects/session-transcript-only.jsonl");
        let transcripts = vec![TranscriptRef {
            cli_session_id: "session-transcript-only".to_string(),
            path: transcript_path.clone(),
        }];

        let sessions = refresh_library(temp.path(), &[], &transcripts).unwrap();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.cli_session_id, "session-transcript-only");
        assert_eq!(session.transcript_path.as_ref(), Some(&transcript_path));
        assert!(session.source_indexes.is_empty());
        assert_eq!(session.raw_index_template, json!({}));
    }

    #[test]
    fn read_sessions_round_trips_write_sessions() {
        let temp = tempdir().unwrap();
        let sessions = vec![LibrarySession {
            cli_session_id: "session-roundtrip".to_string(),
            transcript_path: Some(temp.path().join("projects/session-roundtrip.jsonl")),
            cwd: Some(PathBuf::from("/project/roundtrip")),
            origin_cwd: Some(PathBuf::from("/origin/roundtrip")),
            title: Some("Roundtrip".to_string()),
            created_at_ms: Some(1),
            last_activity_at_ms: Some(2),
            last_focused_at_ms: Some(3),
            completed_turns: Some(4),
            source_indexes: vec![SourceIndex {
                account_id: "account".to_string(),
                org_id: "org".to_string(),
                path: temp.path().join("account/org/local_roundtrip.json"),
            }],
            raw_index_template: json!({"permissionMode": "default"}),
            updated_at: chrono::Utc::now(),
        }];

        write_sessions(temp.path(), &sessions).unwrap();
        let text = fs::read_to_string(sessions_path(temp.path())).unwrap();
        assert!(text.contains("\"cliSessionId\":\"session-roundtrip\""));
        assert!(text.contains("\"createdAt\":1"));
        assert!(text.contains("\"lastActivityAt\":2"));
        assert!(text.contains("\"lastFocusedAt\":3"));

        let read_back = read_sessions(temp.path()).unwrap();

        assert_eq!(read_back, sessions);
    }

    #[test]
    fn refresh_library_uses_latest_source_index_for_session_fields() {
        let temp = tempdir().unwrap();
        let older_bucket = bucket(temp.path(), "older-account", "org");
        let newer_bucket = bucket(temp.path(), "newer-account", "org");
        let indexes = vec![
            (
                newer_bucket,
                vec![index(
                    temp.path().join("newer/local_newer.json"),
                    Some("session-latest"),
                    json!({
                        "sessionId": "local_newer",
                        "cliSessionId": "session-latest",
                        "cwd": "/project/newer",
                        "originCwd": "/origin/newer",
                        "createdAt": 100,
                        "lastActivityAt": 500,
                        "lastFocusedAt": 400,
                        "title": "Newer",
                        "completedTurns": 9,
                        "permissionMode": "acceptEdits"
                    }),
                )],
            ),
            (
                older_bucket,
                vec![index(
                    temp.path().join("older/local_older.json"),
                    Some("session-latest"),
                    json!({
                        "sessionId": "local_older",
                        "cliSessionId": "session-latest",
                        "cwd": "/project/older",
                        "originCwd": "/origin/older",
                        "createdAt": 300,
                        "lastFocusedAt": 350,
                        "title": "Older",
                        "completedTurns": 3,
                        "permissionMode": "bypassPermissions"
                    }),
                )],
            ),
        ];

        let sessions = refresh_library(temp.path(), &indexes, &[]).unwrap();

        let session = &sessions[0];
        assert_eq!(session.cwd.as_deref(), Some(Path::new("/project/newer")));
        assert_eq!(
            session.origin_cwd.as_deref(),
            Some(Path::new("/origin/newer"))
        );
        assert_eq!(session.title.as_deref(), Some("Newer"));
        assert_eq!(session.created_at_ms, Some(100));
        assert_eq!(session.last_activity_at_ms, Some(500));
        assert_eq!(session.last_focused_at_ms, Some(400));
        assert_eq!(session.completed_turns, Some(9));
        assert_eq!(session.raw_index_template["permissionMode"], "acceptEdits");
    }

    #[test]
    fn refresh_library_preserves_latest_nonempty_title_when_newer_index_title_is_missing_or_blank()
    {
        for (case, newer_title) in [("missing", None), ("blank", Some(json!("   \t")))] {
            let temp = tempdir().unwrap();
            let older_bucket = bucket(temp.path(), &format!("older-{case}"), "org");
            let newer_bucket = bucket(temp.path(), &format!("newer-{case}"), "org");
            let cli_session_id = format!("session-title-{case}");
            let mut newer_raw = json!({
                "sessionId": format!("local_newer_{case}"),
                "cliSessionId": cli_session_id,
                "cwd": format!("/project/newer-{case}"),
                "createdAt": 150,
                "lastActivityAt": 200,
                "lastFocusedAt": 190,
                "completedTurns": 7,
                "permissionMode": "acceptEdits"
            });
            if let Some(title) = newer_title {
                newer_raw["title"] = title;
            }
            let indexes = vec![
                (
                    older_bucket,
                    vec![index(
                        temp.path().join(format!("older-{case}/local_older.json")),
                        Some(&format!("session-title-{case}")),
                        json!({
                            "sessionId": format!("local_older_{case}"),
                            "cliSessionId": format!("session-title-{case}"),
                            "cwd": format!("/project/older-{case}"),
                            "createdAt": 50,
                            "lastActivityAt": 100,
                            "title": "Good title",
                            "completedTurns": 3,
                            "permissionMode": "bypassPermissions"
                        }),
                    )],
                ),
                (
                    newer_bucket,
                    vec![index(
                        temp.path().join(format!("newer-{case}/local_newer.json")),
                        Some(&format!("session-title-{case}")),
                        newer_raw,
                    )],
                ),
            ];

            let sessions = refresh_library(temp.path(), &indexes, &[]).unwrap();

            let session = &sessions[0];
            assert_eq!(session.title.as_deref(), Some("Good title"));
            assert_eq!(session.last_activity_at_ms, Some(200));
            assert_eq!(session.completed_turns, Some(7));
            assert_eq!(session.raw_index_template["permissionMode"], "acceptEdits");
            assert_eq!(
                session.cwd.as_deref(),
                Some(Path::new(&format!("/project/newer-{case}")))
            );
        }
    }

    #[test]
    fn refresh_library_ignores_desktop_indexes_without_cli_session_id() {
        let temp = tempdir().unwrap();
        let source = bucket(temp.path(), "account", "org");
        let indexes = vec![(
            source,
            vec![index(
                temp.path().join("local_missing_cli.json"),
                None,
                json!({
                    "sessionId": "local_missing_cli",
                    "cwd": "/project/missing"
                }),
            )],
        )];

        let sessions = refresh_library(temp.path(), &indexes, &[]).unwrap();

        assert!(sessions.is_empty());
    }
}

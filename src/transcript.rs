use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRef {
    pub cli_session_id: String,
    pub path: PathBuf,
    pub cwd: Option<PathBuf>,
    pub created_at_ms: Option<i64>,
    pub last_activity_at_ms: Option<i64>,
    pub title: Option<String>,
}

pub fn scan_transcripts(claude_dir: &Path) -> Result<Vec<TranscriptRef>> {
    let projects_dir = claude_dir.join("projects");
    if !projects_dir
        .try_exists()
        .with_context(|| format!("failed to inspect {}", projects_dir.display()))?
    {
        return Ok(Vec::new());
    }

    fs::read_dir(&projects_dir)
        .with_context(|| format!("failed to read {}", projects_dir.display()))?;

    let mut transcripts = Vec::new();
    for entry in WalkDir::new(&projects_dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if error.depth() == 0 {
                    return Err(error)
                        .with_context(|| format!("failed to walk {}", projects_dir.display()));
                }
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };

        let metadata = if is_top_level_project_transcript(path, &projects_dir) {
            read_transcript_metadata(path)?
        } else {
            TranscriptMetadata::default()
        };

        transcripts.push(TranscriptRef {
            cli_session_id: stem.to_string(),
            path: path.to_path_buf(),
            cwd: metadata.cwd,
            created_at_ms: metadata.created_at_ms,
            last_activity_at_ms: metadata.last_activity_at_ms,
            title: metadata.title,
        });
    }

    transcripts.sort_by(|left, right| left.cli_session_id.cmp(&right.cli_session_id));
    Ok(transcripts)
}

#[derive(Debug, Default)]
struct TranscriptMetadata {
    cwd: Option<PathBuf>,
    created_at_ms: Option<i64>,
    last_activity_at_ms: Option<i64>,
    title: Option<String>,
}

fn is_top_level_project_transcript(path: &Path, projects_dir: &Path) -> bool {
    path.parent().and_then(Path::parent) == Some(projects_dir)
}

fn read_transcript_metadata(path: &Path) -> Result<TranscriptMetadata> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open transcript {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut metadata = TranscriptMetadata::default();

    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read transcript {}", path.display()))?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if metadata.cwd.is_none() {
            metadata.cwd = string_field(&value, "cwd").map(PathBuf::from);
        }

        if let Some(timestamp_ms) = timestamp_ms(&value) {
            metadata.created_at_ms = Some(
                metadata
                    .created_at_ms
                    .map(|created_at_ms| created_at_ms.min(timestamp_ms))
                    .unwrap_or(timestamp_ms),
            );
            metadata.last_activity_at_ms = Some(
                metadata
                    .last_activity_at_ms
                    .map(|last_activity_at_ms| last_activity_at_ms.max(timestamp_ms))
                    .unwrap_or(timestamp_ms),
            );
        }

        if let Some(title) = title_from_record(&value) {
            metadata.title = Some(title);
        }
    }

    Ok(metadata)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    string_field(value, "timestamp")
        .and_then(|timestamp| DateTime::parse_from_rfc3339(&timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis())
}

fn title_from_record(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("last-prompt") {
        return string_field(value, "lastPrompt");
    }

    if value.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }

    let content = value.get("message")?.get("content")?;
    match content {
        Value::String(text) => non_empty_string(text),
        Value::Array(parts) => {
            let title = parts
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        part.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            non_empty_string(&title)
        }
        _ => None,
    }
}

fn non_empty_string(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || is_local_command_message(text) {
        None
    } else {
        Some(text.to_string())
    }
}

fn is_local_command_message(text: &str) -> bool {
    text.starts_with("<local-command-caveat>")
        || text.starts_with("<command-name>")
        || text.starts_with("<command-message>")
        || text.starts_with("<local-command-stdout>")
        || text.starts_with("<local-command-stderr>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scan_transcripts_finds_nested_jsonl_files_and_extracts_cli_session_id() {
        let temp = tempdir().unwrap();
        let project_a = temp.path().join("projects").join("-tmp-project-a");
        let project_b = temp
            .path()
            .join("projects")
            .join("-tmp-project-b")
            .join("nested");
        fs::create_dir_all(&project_a).unwrap();
        fs::create_dir_all(&project_b).unwrap();
        fs::write(project_a.join("session-b.jsonl"), "{}").unwrap();
        fs::write(project_b.join("session-a.jsonl"), "{}").unwrap();
        fs::write(project_b.join("session-c.json"), "{}").unwrap();

        let transcripts = scan_transcripts(temp.path()).unwrap();

        assert_eq!(transcripts.len(), 2);
        assert_eq!(transcripts[0].cli_session_id, "session-a");
        assert_eq!(transcripts[1].cli_session_id, "session-b");
        assert!(transcripts[0].path.ends_with("session-a.jsonl"));
        assert!(transcripts[1].path.ends_with("session-b.jsonl"));
    }

    #[test]
    fn scan_transcripts_extracts_metadata_only_for_top_level_project_files() {
        let temp = tempdir().unwrap();
        let top_level_project = temp.path().join("projects").join("-Users-demo-project");
        let nested_project = top_level_project.join("subagents");
        fs::create_dir_all(&top_level_project).unwrap();
        fs::create_dir_all(&nested_project).unwrap();
        let transcript_text = concat!(
            "{\"type\":\"last-prompt\",\"sessionId\":\"session-top\"}\n",
            "{\"type\":\"user\",\"cwd\":\"/Users/demo/project\",\"timestamp\":\"1970-01-01T00:00:02.500Z\",\"message\":{\"content\":\"first prompt\"}}\n",
            "{\"type\":\"last-prompt\",\"lastPrompt\":\"Final title\",\"sessionId\":\"session-top\"}\n",
            "{\"type\":\"assistant\",\"cwd\":\"/Users/demo/project\",\"timestamp\":\"1970-01-01T00:00:01.000Z\"}\n",
        );
        fs::write(top_level_project.join("session-top.jsonl"), transcript_text).unwrap();
        fs::write(nested_project.join("session-nested.jsonl"), transcript_text).unwrap();

        let transcripts = scan_transcripts(temp.path()).unwrap();

        let top_level = transcripts
            .iter()
            .find(|transcript| transcript.cli_session_id == "session-top")
            .unwrap();
        assert_eq!(
            top_level.cwd.as_deref(),
            Some(Path::new("/Users/demo/project"))
        );
        assert_eq!(top_level.created_at_ms, Some(1000));
        assert_eq!(top_level.last_activity_at_ms, Some(2500));
        assert_eq!(top_level.title.as_deref(), Some("Final title"));

        let nested = transcripts
            .iter()
            .find(|transcript| transcript.cli_session_id == "session-nested")
            .unwrap();
        assert!(nested.cwd.is_none());
        assert!(nested.created_at_ms.is_none());
        assert!(nested.last_activity_at_ms.is_none());
        assert!(nested.title.is_none());
    }

    #[test]
    fn scan_transcripts_does_not_use_local_command_messages_as_titles() {
        let temp = tempdir().unwrap();
        let project = temp.path().join("projects").join("-Users-demo-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("session-local-command.jsonl"),
            concat!(
                "{\"type\":\"user\",\"cwd\":\"/Users/demo/project\",\"timestamp\":\"1970-01-01T00:00:01.000Z\",\"message\":{\"content\":\"<local-command-caveat>ignore this</local-command-caveat>\\n<local-command-stdout>noise</local-command-stdout>\"}}\n",
                "{\"type\":\"assistant\",\"cwd\":\"/Users/demo/project\",\"timestamp\":\"1970-01-01T00:00:02.000Z\"}\n",
            ),
        )
        .unwrap();

        let transcripts = scan_transcripts(temp.path()).unwrap();

        assert_eq!(transcripts.len(), 1);
        assert!(transcripts[0].title.is_none());
    }

    #[test]
    fn scan_transcripts_returns_empty_when_projects_dir_is_missing() {
        let temp = tempdir().unwrap();

        let transcripts = scan_transcripts(temp.path()).unwrap();

        assert!(transcripts.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn scan_transcripts_skips_unreadable_walk_entries() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().unwrap();
        let projects = temp.path().join("projects");
        let readable = projects.join("-tmp-readable");
        let unreadable = projects.join("-tmp-unreadable");
        fs::create_dir_all(&readable).unwrap();
        fs::create_dir_all(&unreadable).unwrap();
        fs::write(readable.join("session-readable.jsonl"), "{}").unwrap();
        fs::write(unreadable.join("session-hidden.jsonl"), "{}").unwrap();

        let original_permissions = fs::metadata(&unreadable).unwrap().permissions();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let result = scan_transcripts(temp.path());

        fs::set_permissions(&unreadable, original_permissions).unwrap();
        let transcripts = result.unwrap();

        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].cli_session_id, "session-readable");
    }
}

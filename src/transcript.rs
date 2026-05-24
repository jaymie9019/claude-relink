use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRef {
    pub cli_session_id: String,
    pub path: PathBuf,
}

pub fn scan_transcripts(claude_dir: &Path) -> Result<Vec<TranscriptRef>> {
    let projects_dir = claude_dir.join("projects");
    if !projects_dir
        .try_exists()
        .with_context(|| format!("failed to inspect {}", projects_dir.display()))?
    {
        return Ok(Vec::new());
    }

    let mut transcripts = Vec::new();
    for entry in WalkDir::new(&projects_dir) {
        let entry = entry.with_context(|| format!("failed to walk {}", projects_dir.display()))?;
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

        transcripts.push(TranscriptRef {
            cli_session_id: stem.to_string(),
            path: path.to_path_buf(),
        });
    }

    transcripts.sort_by(|left, right| left.cli_session_id.cmp(&right.cli_session_id));
    Ok(transcripts)
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
    fn scan_transcripts_returns_empty_when_projects_dir_is_missing() {
        let temp = tempdir().unwrap();

        let transcripts = scan_transcripts(temp.path()).unwrap();

        assert!(transcripts.is_empty());
    }
}

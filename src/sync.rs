use crate::desktop_index::{scan_desktop_indexes, DesktopIndex};
use crate::library::{refresh_library, LibrarySession};
use crate::paths::{list_desktop_buckets, resolve_target_bucket, DesktopBucket};
use crate::transcript::scan_transcripts;
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SyncFilters {
    pub project: Option<PathBuf>,
    pub from_account: Option<String>,
    pub from_org: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub target_bucket: DesktopBucket,
    pub library_sessions: Vec<LibrarySession>,
    pub already_visible: Vec<DesktopIndex>,
    pub missing: Vec<LibrarySession>,
    pub skipped_missing_transcript: Vec<LibrarySession>,
}

pub fn build_sync_plan(
    claude_dir: &Path,
    desktop_dir: &Path,
    library_dir: &Path,
    target_account: Option<&str>,
    target_org: Option<&str>,
    filters: SyncFilters,
) -> Result<SyncPlan> {
    let buckets = list_desktop_buckets(desktop_dir)?;
    let mut bucket_indexes = Vec::new();
    for bucket in &buckets {
        if filters
            .from_account
            .as_deref()
            .is_some_and(|account| account != bucket.account_id)
        {
            continue;
        }
        if filters
            .from_org
            .as_deref()
            .is_some_and(|org| org != bucket.org_id)
        {
            continue;
        }
        bucket_indexes.push((bucket.clone(), scan_desktop_indexes(&bucket.path)?));
    }

    let transcripts = scan_transcripts(claude_dir)?;
    let library_sessions = refresh_library(library_dir, &bucket_indexes, &transcripts)?;
    let target_bucket = resolve_target_bucket(desktop_dir, target_account, target_org)?;
    let target_indexes = scan_desktop_indexes(&target_bucket.path)?;
    let target_cli_ids: BTreeSet<String> = target_indexes
        .iter()
        .filter_map(|index| index.cli_session_id.clone())
        .collect();

    let mut missing = Vec::new();
    let mut skipped_missing_transcript = Vec::new();
    for session in library_sessions.iter().cloned() {
        if !matches_source_filters(&session, &filters) {
            continue;
        }
        if let Some(project) = &filters.project {
            let matches_project = session.cwd.as_deref() == Some(project.as_path())
                || session.origin_cwd.as_deref() == Some(project.as_path());
            if !matches_project {
                continue;
            }
        }

        let transcript_exists = session
            .transcript_path
            .as_ref()
            .is_some_and(|path| path.exists());
        if !transcript_exists {
            skipped_missing_transcript.push(session);
            continue;
        }

        if !target_cli_ids.contains(&session.cli_session_id) {
            missing.push(session);
        }
    }

    Ok(SyncPlan {
        target_bucket,
        library_sessions,
        already_visible: target_indexes,
        missing,
        skipped_missing_transcript,
    })
}

fn matches_source_filters(session: &LibrarySession, filters: &SyncFilters) -> bool {
    if filters.from_account.is_none() && filters.from_org.is_none() {
        return true;
    }

    session.source_indexes.iter().any(|source| {
        filters
            .from_account
            .as_deref()
            .is_none_or(|account| source.account_id == account)
            && filters
                .from_org
                .as_deref()
                .is_none_or(|org| source.org_id == org)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    fn create_bucket(desktop_dir: &Path, account_id: &str, org_id: &str) -> PathBuf {
        let bucket = desktop_dir
            .join("claude-code-sessions")
            .join(account_id)
            .join(org_id);
        fs::create_dir_all(&bucket).unwrap();
        bucket
    }

    fn write_owner(desktop_dir: &Path) {
        fs::create_dir_all(desktop_dir).unwrap();
        fs::write(
            desktop_dir.join("cowork-enabled-cli-ops.json"),
            r#"{"ownerAccountId":"current"}"#,
        )
        .unwrap();
    }

    fn write_transcript(claude_dir: &Path, cli_session_id: &str) {
        let project = claude_dir.join("projects/-Users-demo-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join(format!("{cli_session_id}.jsonl")), "{}\n").unwrap();
    }

    fn write_index(
        bucket: &Path,
        file_stem: &str,
        cli_session_id: &str,
        cwd: &str,
        origin_cwd: &str,
    ) {
        fs::write(
            bucket.join(format!("{file_stem}.json")),
            json!({
                "sessionId": file_stem,
                "cliSessionId": cli_session_id,
                "cwd": cwd,
                "originCwd": origin_cwd,
                "title": file_stem,
                "createdAt": 1000,
                "lastActivityAt": 2000
            })
            .to_string(),
        )
        .unwrap();
    }

    fn session_ids(sessions: &[LibrarySession]) -> Vec<String> {
        sessions
            .iter()
            .map(|session| session.cli_session_id.clone())
            .collect()
    }

    #[test]
    fn project_filter_applies_to_missing_and_skipped_sessions() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        let source_bucket = create_bucket(&desktop_dir, "old", "org");
        create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);
        write_transcript(&claude_dir, "a");
        write_transcript(&claude_dir, "b");
        write_index(
            &source_bucket,
            "local_a",
            "a",
            "/Users/demo/project",
            "/Users/demo/project",
        );
        write_index(
            &source_bucket,
            "local_b",
            "b",
            "/Users/demo/other",
            "/Users/demo/other",
        );
        write_index(
            &source_bucket,
            "local_c",
            "c",
            "/Users/demo/elsewhere",
            "/Users/demo/project",
        );

        let plan = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: Some(PathBuf::from("/Users/demo/project")),
                from_account: None,
                from_org: None,
            },
        )
        .unwrap();

        assert_eq!(session_ids(&plan.missing), vec!["a"]);
        assert_eq!(session_ids(&plan.skipped_missing_transcript), vec!["c"]);
    }

    #[test]
    fn source_filters_exclude_other_sources_and_transcript_only_sessions_from_missing() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        let old_bucket = create_bucket(&desktop_dir, "old", "org");
        let other_bucket = create_bucket(&desktop_dir, "other", "org");
        create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);
        write_transcript(&claude_dir, "a");
        write_transcript(&claude_dir, "b");
        write_transcript(&claude_dir, "c");
        write_index(
            &old_bucket,
            "local_a",
            "a",
            "/Users/demo/project",
            "/Users/demo/project",
        );
        write_index(
            &other_bucket,
            "local_b",
            "b",
            "/Users/demo/project",
            "/Users/demo/project",
        );

        let plan = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: None,
                from_account: Some("old".to_string()),
                from_org: Some("org".to_string()),
            },
        )
        .unwrap();

        assert_eq!(session_ids(&plan.missing), vec!["a"]);
    }
}

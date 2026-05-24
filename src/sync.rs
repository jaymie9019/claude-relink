use crate::backup::{create_sync_backup, write_sync_manifest};
use crate::desktop_index::{build_index_for_current_account, scan_desktop_indexes, DesktopIndex};
use crate::library::{refresh_library, LibrarySession};
use crate::paths::{list_desktop_buckets, resolve_target_bucket, DesktopBucket};
use crate::process::is_claude_desktop_running;
use crate::transcript::scan_transcripts;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const CLAUDE_DESKTOP_RUNNING_MESSAGE: &str = "Claude Desktop appears to be running.
Quit Claude Desktop fully before applying sync.
Use --force-while-running only if you know what you are doing.";

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

#[derive(Debug, Clone)]
pub struct ApplySummary {
    pub backup_path: PathBuf,
    pub created_files: Vec<PathBuf>,
    pub skipped_existing: Vec<String>,
}

pub fn build_sync_plan(
    claude_dir: &Path,
    desktop_dir: &Path,
    library_dir: &Path,
    target_account: Option<&str>,
    target_org: Option<&str>,
    filters: SyncFilters,
) -> Result<SyncPlan> {
    let project_filter = filters
        .project
        .as_deref()
        .map(ProjectFilter::new)
        .transpose()?;
    let buckets = list_desktop_buckets(desktop_dir)?;
    let mut bucket_indexes = Vec::new();
    for bucket in &buckets {
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
    let mut filtered_sessions = Vec::new();
    for session in &library_sessions {
        if session_matches_filters(session, &filters, project_filter.as_ref())? {
            filtered_sessions.push(session);
        }
    }
    let filtered_cli_ids: BTreeSet<String> = filtered_sessions
        .iter()
        .map(|session| session.cli_session_id.clone())
        .collect();
    let already_visible = target_indexes
        .into_iter()
        .filter(|index| {
            index
                .cli_session_id
                .as_ref()
                .is_some_and(|cli_session_id| filtered_cli_ids.contains(cli_session_id))
        })
        .collect();

    let mut missing = Vec::new();
    let mut skipped_missing_transcript = Vec::new();
    for session in filtered_sessions {
        let transcript_exists = match &session.transcript_path {
            Some(path) => path
                .try_exists()
                .with_context(|| format!("failed to inspect transcript {}", path.display()))?,
            None => false,
        };
        if !transcript_exists {
            skipped_missing_transcript.push(session.clone());
            continue;
        }

        if !target_cli_ids.contains(&session.cli_session_id) {
            missing.push(session.clone());
        }
    }

    Ok(SyncPlan {
        target_bucket,
        library_sessions,
        already_visible,
        missing,
        skipped_missing_transcript,
    })
}

pub fn apply_sync_plan(
    plan: &SyncPlan,
    relink_dir: &Path,
    force_while_running: bool,
) -> Result<ApplySummary> {
    if !force_while_running && is_claude_desktop_running()? {
        bail!(CLAUDE_DESKTOP_RUNNING_MESSAGE);
    }

    verify_apply_preconditions(plan)?;

    let backup = create_sync_backup(relink_dir, &plan.target_bucket)?;
    write_sync_manifest(&backup, &[], &[])?;

    let target_indexes = scan_desktop_indexes(&plan.target_bucket.path)?;
    let mut current_cli_ids = target_indexes
        .iter()
        .filter_map(|index| index.cli_session_id.clone())
        .collect::<BTreeSet<_>>();

    let mut created_files = Vec::new();
    let mut created_file_names = Vec::new();
    let mut skipped_existing = Vec::new();
    for session in &plan.missing {
        if current_cli_ids.contains(&session.cli_session_id) {
            skipped_existing.push(session.cli_session_id.clone());
            continue;
        }

        verify_session_transcript_exists(session)?;
        let rebuilt = build_index_for_current_account(session);
        let written = atomic_write_json(&plan.target_bucket.path, &rebuilt)?;
        let file_name = written
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .context("created file path has no filename")?;
        created_file_names.push(file_name);
        created_files.push(written);
        current_cli_ids.insert(session.cli_session_id.clone());
    }

    write_sync_manifest(&backup, &created_file_names, &skipped_existing)?;

    Ok(ApplySummary {
        backup_path: backup.root_path,
        created_files,
        skipped_existing,
    })
}

fn verify_apply_preconditions(plan: &SyncPlan) -> Result<()> {
    if !plan
        .target_bucket
        .path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", plan.target_bucket.path.display()))?
    {
        bail!(
            "target Desktop bucket does not exist: {}",
            plan.target_bucket.path.display()
        );
    }

    for session in &plan.missing {
        verify_session_transcript_exists(session)?;
    }

    Ok(())
}

fn verify_session_transcript_exists(session: &LibrarySession) -> Result<()> {
    let transcript_path = session
        .transcript_path
        .as_ref()
        .with_context(|| format!("session {} has no transcript path", session.cli_session_id))?;
    if !transcript_path
        .try_exists()
        .with_context(|| format!("failed to inspect transcript {}", transcript_path.display()))?
    {
        bail!(
            "transcript is missing for session {}: {}",
            session.cli_session_id,
            transcript_path.display()
        );
    }

    Ok(())
}

fn atomic_write_json(bucket: &Path, value: &serde_json::Value) -> Result<PathBuf> {
    let session_id = value
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .context("Desktop index is missing string sessionId")?;
    let file_name = format!("{session_id}.json");
    let final_path = bucket.join(&file_name);
    if final_path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", final_path.display()))?
    {
        bail!("target index already exists: {}", final_path.display());
    }

    let temp_path = bucket.join(format!(".{file_name}.tmp"));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        serde_json::to_writer(&mut file, value)
            .with_context(|| format!("failed to serialize {}", temp_path.display()))?;
        file.flush()
            .with_context(|| format!("failed to flush {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp_path.display()))?;
        drop(file);
        fs::rename(&temp_path, &final_path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                temp_path.display(),
                final_path.display()
            )
        })?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result?;
    Ok(final_path)
}

#[derive(Debug)]
struct ProjectFilter {
    original: PathBuf,
    canonical: PathBuf,
}

impl ProjectFilter {
    fn new(project: &Path) -> Result<Self> {
        let canonical = project
            .canonicalize()
            .with_context(|| format!("failed to canonicalize --project {}", project.display()))?;
        Ok(Self {
            original: project.to_path_buf(),
            canonical,
        })
    }
}

fn session_matches_filters(
    session: &LibrarySession,
    filters: &SyncFilters,
    project_filter: Option<&ProjectFilter>,
) -> Result<bool> {
    if !matches_source_filters(session, filters) {
        return Ok(false);
    }

    if let Some(project_filter) = project_filter {
        return matches_project_filter(session, project_filter);
    }

    Ok(true)
}

fn matches_project_filter(
    session: &LibrarySession,
    project_filter: &ProjectFilter,
) -> Result<bool> {
    let cwd_matches = session
        .cwd
        .as_deref()
        .map(|path| path_matches_project(path, project_filter))
        .transpose()?
        .unwrap_or(false);
    if cwd_matches {
        return Ok(true);
    }

    session
        .origin_cwd
        .as_deref()
        .map(|path| path_matches_project(path, project_filter))
        .transpose()
        .map(|matches| matches.unwrap_or(false))
}

fn path_matches_project(path: &Path, project_filter: &ProjectFilter) -> Result<bool> {
    if path == project_filter.canonical || path == project_filter.original {
        return Ok(true);
    }

    match path.canonicalize() {
        Ok(canonical) => Ok(canonical == project_filter.canonical),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(path == project_filter.canonical || path == project_filter.original)
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to canonicalize stored session path {}",
                path.display()
            )
        }),
    }
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

    fn desktop_cli_ids(indexes: &[DesktopIndex]) -> Vec<String> {
        indexes
            .iter()
            .filter_map(|index| index.cli_session_id.clone())
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
        let project_dir = temp.path().join("workspace");
        let other_project_dir = temp.path().join("other-workspace");
        let elsewhere_dir = temp.path().join("elsewhere");
        fs::create_dir_all(&project_dir).unwrap();
        fs::create_dir_all(&other_project_dir).unwrap();
        write_transcript(&claude_dir, "a");
        write_transcript(&claude_dir, "b");
        write_index(
            &source_bucket,
            "local_a",
            "a",
            project_dir.to_str().unwrap(),
            project_dir.to_str().unwrap(),
        );
        write_index(
            &source_bucket,
            "local_b",
            "b",
            other_project_dir.to_str().unwrap(),
            other_project_dir.to_str().unwrap(),
        );
        write_index(
            &source_bucket,
            "local_c",
            "c",
            elsewhere_dir.to_str().unwrap(),
            project_dir.to_str().unwrap(),
        );

        let plan = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: Some(project_dir.clone()),
                from_account: None,
                from_org: None,
            },
        )
        .unwrap();

        assert_eq!(session_ids(&plan.missing), vec!["a"]);
        assert_eq!(session_ids(&plan.skipped_missing_transcript), vec!["c"]);
    }

    #[test]
    fn project_filter_canonicalizes_cli_path_before_matching_session_cwd() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        let source_bucket = create_bucket(&desktop_dir, "old", "org");
        create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);
        let project_dir = temp.path().join("workspace");
        fs::create_dir_all(project_dir.join("subdir")).unwrap();
        write_transcript(&claude_dir, "a");
        write_index(
            &source_bucket,
            "local_a",
            "a",
            project_dir.to_str().unwrap(),
            project_dir.to_str().unwrap(),
        );

        let plan = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: Some(project_dir.join("subdir/..")),
                from_account: None,
                from_org: None,
            },
        )
        .unwrap();

        assert_eq!(session_ids(&plan.missing), vec!["a"]);
    }

    #[test]
    fn project_filter_returns_context_when_cli_path_cannot_canonicalize() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);

        let error = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: Some(temp.path().join("missing")),
                from_account: None,
                from_org: None,
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to canonicalize --project"));
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

    #[test]
    fn source_filters_do_not_narrow_library_refresh_sources() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        let old_bucket = create_bucket(&desktop_dir, "old", "org");
        let other_bucket = create_bucket(&desktop_dir, "other", "org");
        create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);
        write_transcript(&claude_dir, "a");
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

        assert_eq!(session_ids(&plan.library_sessions), vec!["a", "b", "c"]);
        let other_session = plan
            .library_sessions
            .iter()
            .find(|session| session.cli_session_id == "b")
            .unwrap();
        assert_eq!(other_session.source_indexes.len(), 1);
        assert_eq!(other_session.source_indexes[0].account_id, "other");
        assert_eq!(session_ids(&plan.missing), vec!["a"]);
        assert!(plan.skipped_missing_transcript.is_empty());
    }

    #[test]
    fn already_visible_is_scoped_to_filtered_library_sessions() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let library_dir = temp.path().join("library");
        let old_bucket = create_bucket(&desktop_dir, "old", "org");
        let current_bucket = create_bucket(&desktop_dir, "current", "org");
        write_owner(&desktop_dir);
        let project_dir = temp.path().join("workspace");
        let other_project_dir = temp.path().join("other-workspace");
        fs::create_dir_all(project_dir.join("subdir")).unwrap();
        fs::create_dir_all(&other_project_dir).unwrap();
        write_transcript(&claude_dir, "a");
        write_transcript(&claude_dir, "b");
        write_index(
            &old_bucket,
            "local_a",
            "a",
            project_dir.to_str().unwrap(),
            project_dir.to_str().unwrap(),
        );
        write_index(
            &current_bucket,
            "local_b",
            "b",
            other_project_dir.to_str().unwrap(),
            other_project_dir.to_str().unwrap(),
        );

        let plan = build_sync_plan(
            &claude_dir,
            &desktop_dir,
            &library_dir,
            None,
            None,
            SyncFilters {
                project: Some(project_dir.join("subdir/..")),
                from_account: Some("old".to_string()),
                from_org: Some("org".to_string()),
            },
        )
        .unwrap();

        assert_eq!(session_ids(&plan.missing), vec!["a"]);
        assert!(desktop_cli_ids(&plan.already_visible).is_empty());
    }

    #[test]
    fn atomic_write_json_uses_session_id_filename_and_removes_temp_file() {
        let temp = tempdir().unwrap();
        let value = json!({
            "sessionId": "local_atomic",
            "cliSessionId": "cli-atomic"
        });

        let written = atomic_write_json(temp.path(), &value).unwrap();

        assert_eq!(written, temp.path().join("local_atomic.json"));
        assert_eq!(
            fs::read_to_string(temp.path().join("local_atomic.json")).unwrap(),
            "{\"cliSessionId\":\"cli-atomic\",\"sessionId\":\"local_atomic\"}"
        );
        assert!(!temp.path().join(".local_atomic.json.tmp").exists());
    }

    #[test]
    fn apply_sync_plan_skips_session_that_became_visible_after_plan_was_built() {
        let temp = tempdir().unwrap();
        let claude_dir = temp.path().join(".claude");
        let desktop_dir = temp.path().join("desktop");
        let relink_dir = temp.path().join("relink");
        let current_bucket = create_bucket(&desktop_dir, "current", "org");
        write_transcript(&claude_dir, "a");
        write_index(
            &current_bucket,
            "local_current_a",
            "a",
            "/Users/demo/project",
            "/Users/demo/project",
        );
        let session = LibrarySession {
            cli_session_id: "a".to_string(),
            transcript_path: Some(claude_dir.join("projects/-Users-demo-project/a.jsonl")),
            cwd: Some(PathBuf::from("/Users/demo/project")),
            origin_cwd: Some(PathBuf::from("/Users/demo/project")),
            title: Some("Already visible".to_string()),
            created_at_ms: Some(1000),
            last_activity_at_ms: Some(2000),
            last_focused_at_ms: Some(3000),
            completed_turns: None,
            source_indexes: Vec::new(),
            raw_index_template: json!({}),
            updated_at: chrono::Utc::now(),
        };
        let plan = SyncPlan {
            target_bucket: DesktopBucket {
                account_id: "current".to_string(),
                org_id: "org".to_string(),
                path: current_bucket.clone(),
                local_index_count: 1,
            },
            library_sessions: vec![session.clone()],
            already_visible: Vec::new(),
            missing: vec![session],
            skipped_missing_transcript: Vec::new(),
        };

        let summary = apply_sync_plan(&plan, &relink_dir, true).unwrap();

        assert!(summary.created_files.is_empty());
        assert_eq!(summary.skipped_existing, vec!["a"]);
        assert_eq!(
            scan_desktop_indexes(&current_bucket).unwrap().len(),
            1,
            "apply must not create a duplicate current-account index"
        );
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(summary.backup_path.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest["createdFiles"].as_array().unwrap().is_empty());
        assert_eq!(manifest["skippedExisting"][0], "a");
    }
}

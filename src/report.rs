use crate::library::{LibraryInspect, LibrarySession};
use crate::sync::{ApplySummary, SyncPlan};
use chrono::SecondsFormat;

pub fn sync_plan(plan: &SyncPlan) -> String {
    format!(
        "\
Current Desktop bucket:
{}

Library sessions: {}
Already visible in current account: {}
Missing in current account: {}
Skipped because transcript is missing: {}
Skipped because Desktop metadata is incomplete: {}

Next:
  Quit Claude Desktop
  claude-relink sync --apply
",
        plan.target_bucket.path.display(),
        plan.library_sessions.len(),
        plan.already_visible.len(),
        plan.missing.len(),
        plan.skipped_missing_transcript.len(),
        plan.skipped_unsupported_desktop_metadata.len(),
    )
}

pub fn apply_summary(summary: &ApplySummary) -> String {
    format!(
        "\
Sync applied.

Backup:
{}

Created files: {}
Skipped existing: {}
",
        summary.backup_path.display(),
        summary.created_files.len(),
        summary.skipped_existing.len(),
    )
}

pub fn restore_summary(summary: &crate::restore::RestoreSummary) -> String {
    format!(
        "\
Restore completed.

Backup:
{}

Restored bucket:
{}

Restored files: {}
",
        summary.backup_path.display(),
        summary.restored_bucket.display(),
        summary.restored_file_count,
    )
}

pub fn library_inspect(inspect: &LibraryInspect) -> String {
    let mut report = format!(
        "\
Library: {}
Sessions: {}
Projects: {}
Source buckets: {}
Missing transcript records: {}
",
        inspect.library_dir.display(),
        inspect.sessions,
        inspect.projects,
        inspect.source_buckets,
        inspect.missing_transcript_records,
    );
    if let Some(last_refresh) = inspect.last_refresh {
        report.push_str(&format!(
            "Last refresh: {}\n",
            last_refresh.to_rfc3339_opts(SecondsFormat::Secs, true)
        ));
    }
    report
}

pub fn library_rebuild(sessions: &[LibrarySession]) -> String {
    format!("Library rebuilt. Sessions: {}\n", sessions.len())
}

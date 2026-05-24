use crate::sync::{ApplySummary, SyncPlan};

pub fn sync_plan(plan: &SyncPlan) -> String {
    format!(
        "\
Current Desktop bucket:
{}

Library sessions: {}
Already visible in current account: {}
Missing in current account: {}
Skipped because transcript is missing: {}

Next:
  Quit Claude Desktop
  claude-relink sync --apply
",
        plan.target_bucket.path.display(),
        plan.library_sessions.len(),
        plan.already_visible.len(),
        plan.missing.len(),
        plan.skipped_missing_transcript.len(),
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

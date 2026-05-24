use crate::sync::SyncPlan;

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

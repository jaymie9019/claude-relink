use anyhow::{bail, Context, Result};
use std::process::Command;

pub fn is_claude_desktop_running() -> Result<bool> {
    is_process_matching("Claude.app")
}

#[cfg(unix)]
fn is_process_matching(pattern: &str) -> Result<bool> {
    let status = Command::new("pgrep")
        .args(["-f", pattern])
        .status()
        .with_context(|| format!("failed to run pgrep -f {pattern}"))?;

    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => bail!("pgrep -f {pattern} exited with status {code}"),
        None => bail!("pgrep -f {pattern} terminated without an exit status"),
    }
}

#[cfg(not(unix))]
fn is_process_matching(_pattern: &str) -> Result<bool> {
    Ok(false)
}

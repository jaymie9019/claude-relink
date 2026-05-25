use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};

pub fn is_claude_desktop_running() -> Result<bool> {
    is_process_name_running("Claude")
}

#[cfg(unix)]
fn is_process_name_running(name: &str) -> Result<bool> {
    let status = Command::new("pgrep")
        .args(["-x", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run pgrep -x {name}"))?;

    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => bail!("pgrep -x {name} exited with status {code}"),
        None => bail!("pgrep -x {name} terminated without an exit status"),
    }
}

#[cfg(not(unix))]
fn is_process_name_running(_name: &str) -> Result<bool> {
    Ok(false)
}

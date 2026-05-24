use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "claude-relink")]
#[command(
    about = "Sync local Claude Code session visibility into the current Claude Desktop account"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Sync(SyncCommand),
    Restore(RestoreCommand),
    Library(LibraryCommand),
}

#[derive(Debug, Clone, Parser)]
pub struct CommonPaths {
    #[arg(long)]
    pub claude_dir: Option<PathBuf>,
    #[arg(long)]
    pub desktop_dir: Option<PathBuf>,
    #[arg(long)]
    pub relink_dir: Option<PathBuf>,
    #[arg(long)]
    pub account_id: Option<String>,
    #[arg(long)]
    pub org_id: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct SyncCommand {
    #[command(flatten)]
    pub paths: CommonPaths,
    #[arg(long)]
    pub apply: bool,
    #[arg(long)]
    pub project: Option<PathBuf>,
    #[arg(long)]
    pub from_account: Option<String>,
    #[arg(long)]
    pub from_org: Option<String>,
    #[arg(long)]
    pub force_while_running: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct RestoreCommand {
    #[arg(long)]
    pub latest: bool,
    #[arg(long)]
    pub backup: Option<PathBuf>,
    #[arg(long)]
    pub force_while_running: bool,
    #[arg(long)]
    pub relink_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum LibraryAction {
    Inspect,
    Rebuild,
}

#[derive(Debug, Clone, Parser)]
pub struct LibraryCommand {
    #[command(subcommand)]
    pub action: LibraryAction,
    #[command(flatten)]
    pub paths: CommonPaths,
}

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Sync(_) => Ok(()),
        Command::Restore(_) => Ok(()),
        Command::Library(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_task_one_commands() {
        for args in [
            &["claude-relink", "sync"][..],
            &["claude-relink", "restore", "--latest"][..],
            &["claude-relink", "library", "inspect"][..],
            &["claude-relink", "library", "rebuild"][..],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
    }
}

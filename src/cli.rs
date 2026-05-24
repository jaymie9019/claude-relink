use anyhow::Result;
use clap::{ArgGroup, Parser, Subcommand};
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
    #[arg(long, requires = "org_id")]
    pub account_id: Option<String>,
    #[arg(long, requires = "account_id")]
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
    #[arg(long, requires = "from_org")]
    pub from_account: Option<String>,
    #[arg(long, requires = "from_account")]
    pub from_org: Option<String>,
    #[arg(long)]
    pub force_while_running: bool,
}

#[derive(Debug, Clone, Parser)]
#[command(group(
    ArgGroup::new("restore_target")
        .required(true)
        .multiple(false)
        .args(["latest", "backup"])
))]
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
    use super::{Cli, Command, LibraryAction};
    use clap::Parser;
    use std::path::PathBuf;

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

    #[test]
    fn parses_sync_options_into_expected_values() {
        let cli = Cli::try_parse_from([
            "claude-relink",
            "sync",
            "--apply",
            "--project",
            "/tmp/project",
            "--account-id",
            "current-account",
            "--org-id",
            "current-org",
            "--from-account",
            "old-account",
            "--from-org",
            "old-org",
            "--force-while-running",
        ])
        .unwrap();

        let Command::Sync(command) = cli.command else {
            panic!("expected sync command");
        };

        assert!(command.apply);
        assert_eq!(command.project, Some(PathBuf::from("/tmp/project")));
        assert_eq!(command.paths.account_id.as_deref(), Some("current-account"));
        assert_eq!(command.paths.org_id.as_deref(), Some("current-org"));
        assert_eq!(command.from_account.as_deref(), Some("old-account"));
        assert_eq!(command.from_org.as_deref(), Some("old-org"));
        assert!(command.force_while_running);
    }

    #[test]
    fn rejects_restore_without_exactly_one_target() {
        for args in [
            &["claude-relink", "restore"][..],
            &[
                "claude-relink",
                "restore",
                "--latest",
                "--backup",
                "/tmp/backup",
            ][..],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn rejects_unpaired_target_account_and_org() {
        for args in [
            &["claude-relink", "sync", "--account-id", "current-account"][..],
            &["claude-relink", "sync", "--org-id", "current-org"][..],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn rejects_unpaired_source_account_and_org() {
        for args in [
            &["claude-relink", "sync", "--from-account", "old-account"][..],
            &["claude-relink", "sync", "--from-org", "old-org"][..],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn parses_library_parent_options_before_action() {
        let cli = Cli::try_parse_from([
            "claude-relink",
            "library",
            "--relink-dir",
            "/tmp/relink",
            "inspect",
        ])
        .unwrap();

        let Command::Library(command) = cli.command else {
            panic!("expected library command");
        };

        assert!(matches!(command.action, LibraryAction::Inspect));
        assert_eq!(command.paths.relink_dir, Some(PathBuf::from("/tmp/relink")));
    }
}

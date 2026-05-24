pub mod backup;
pub mod cli;
pub mod desktop_index;
pub mod library;
pub mod paths;
pub mod process;
pub mod report;
pub mod sync;
pub mod transcript;

use anyhow::Result;

pub fn run() -> Result<()> {
    let cli = <cli::Cli as clap::Parser>::parse();
    cli::dispatch(cli)
}

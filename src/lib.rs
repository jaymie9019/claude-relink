pub mod cli;
pub mod desktop_index;
pub mod paths;
pub mod transcript;

use anyhow::Result;

pub fn run() -> Result<()> {
    let cli = <cli::Cli as clap::Parser>::parse();
    cli::dispatch(cli)
}

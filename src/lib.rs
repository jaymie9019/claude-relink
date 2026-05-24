pub mod cli;

use anyhow::Result;

pub fn run() -> Result<()> {
    let cli = <cli::Cli as clap::Parser>::parse();
    cli::dispatch(cli)
}

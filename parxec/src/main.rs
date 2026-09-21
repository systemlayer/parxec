#[allow(dead_code)]
mod cli;

use clap::Parser;
use cli::{AnalyzeArgs, Cli, Commands, HashArgs, RunArgs};

fn hash(_args: HashArgs) -> anyhow::Result<()> {
  anyhow::bail!("hash is not implemented yet")
}

fn analyze(_args: AnalyzeArgs) -> anyhow::Result<()> {
  anyhow::bail!("analyze is not implemented yet")
}

fn run(_args: RunArgs) -> anyhow::Result<()> {
  anyhow::bail!("run is not implemented yet")
}

fn main() -> anyhow::Result<()> {
  match Cli::parse().command {
    Commands::Hash(args) => hash(args),
    Commands::Analyze(args) => analyze(args),
    Commands::Run(args) => run(args),
  }
}

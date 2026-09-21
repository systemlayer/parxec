#[allow(dead_code)]
mod cli;
mod input;

use clap::Parser;
use cli::{AnalyzeArgs, Cli, Commands, HashArgs, RunArgs};

fn hash(args: HashArgs) -> anyhow::Result<()> {
  input::discover_files(&args.input_dir, args.file_limit)?;
  anyhow::bail!("hash is not implemented yet")
}

fn analyze(_args: AnalyzeArgs) -> anyhow::Result<()> {
  anyhow::bail!("analyze is not implemented yet")
}

fn run(args: RunArgs) -> anyhow::Result<()> {
  input::discover_files(&args.input_dir, args.file_limit)?;
  anyhow::bail!("run is not implemented yet")
}

fn main() -> anyhow::Result<()> {
  match Cli::parse().command {
    Commands::Hash(args) => hash(args),
    Commands::Analyze(args) => analyze(args),
    Commands::Run(args) => run(args),
  }
}

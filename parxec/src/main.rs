#[allow(dead_code)]
mod cli;
mod hash_file;
mod hasher;
mod input;

use anyhow::{Context, bail};
use clap::Parser;
use cli::{AnalyzeArgs, Cli, Commands, HashAlgorithm, HashArgs, RunArgs};
use std::{collections::HashSet, fs, time::Instant};

/// Hashes selected files and writes a JSON hash file.
fn hash(args: HashArgs) -> anyhow::Result<()> {
  if args.hashing.hash_algorithm != HashAlgorithm::Sha256 {
    bail!("hash algorithm {:?} is not implemented yet", args.hashing.hash_algorithm);
  }
  let mut names = input::discover_files(&args.input_dir)?;
  if args.hash_output.exists() {
    let output = fs::canonicalize(&args.hash_output)
      .with_context(|| format!("cannot inspect hash output {}", args.hash_output.display()))?;
    // Exclude the hashes JSON file when it is inside the input directory.
    names.retain(|name| {
      fs::canonicalize(args.input_dir.join(name)).map_or(true, |path| path != output)
    });
  }
  if args.file_limit > 0 {
    names.truncate(args.file_limit);
  }
  if names.is_empty() {
    bail!("input directory {} contains no files to hash", args.input_dir.display());
  }
  println!("Hashing {} files.", names.len());
  let start = Instant::now();
  let hashes = hasher::hash_files(&args.input_dir, &names, args.hashing.hash_threads)?;
  let elapsed = start.elapsed();
  hash_file::write(&args.hash_output, &hashes)?;
  let unique = hashes.values().collect::<HashSet<_>>().len();
  let redundant = hashes.len() - unique;
  let estimate_unit = args.file_ms as f64 / 1000.0 / args.jobs.get() as f64;
  println!("Saved hashes to {}.", args.hash_output.display());
  println!();
  println!("Files: {} hashed, {} unique, {} redundant.", hashes.len(), unique, redundant);
  println!("Hashing took {:.2}s.", elapsed.as_secs_f64());
  println!();
  println!("Estimated processing at {}ms per file with {} jobs:", args.file_ms, args.jobs);
  println!("  All files:     {:.2}s", hashes.len() as f64 * estimate_unit);
  println!("  Unique files:  {:.2}s", unique as f64 * estimate_unit);
  println!("  Time saved:    {:.2}s", redundant as f64 * estimate_unit);
  Ok(())
}

fn analyze(_args: AnalyzeArgs) -> anyhow::Result<()> {
  anyhow::bail!("analyze is not implemented yet")
}

fn run(args: RunArgs) -> anyhow::Result<()> {
  input::discover_files(&args.input_dir)?;
  anyhow::bail!("run is not implemented yet")
}

fn main() -> anyhow::Result<()> {
  match Cli::parse().command {
    Commands::Hash(args) => hash(args),
    Commands::Analyze(args) => analyze(args),
    Commands::Run(args) => run(args),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hash_writes_shared_format_and_excludes_existing_output() {
    let dir = std::env::temp_dir().join(format!("parxec-hash-command-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.bin"), b"same").unwrap();
    fs::write(dir.join("b.bin"), b"same").unwrap();
    fs::write(dir.join("c.bin"), b"other").unwrap();
    let output = dir.join("hashes.json");
    fs::write(&output, b"previous output").unwrap();
    for threads in [0, 2] {
      let args = HashArgs {
        input_dir: dir.clone(),
        hash_output: output.clone(),
        hashing: cli::HashOptions {
          hash_algorithm: HashAlgorithm::Sha256,
          hash_threads: threads,
          tile_size: std::num::NonZeroUsize::new(8).unwrap(),
        },
        jobs: std::num::NonZeroUsize::new(4).unwrap(),
        file_limit: 2,
        file_ms: 1000,
      };
      hash(args).unwrap();
      let hashes = hash_file::read(&output).unwrap();
      assert_eq!(hashes.keys().map(String::as_str).collect::<Vec<_>>(), ["a.bin", "b.bin"]);
      assert_eq!(hashes["a.bin"], hashes["b.bin"]);
      let json = fs::read_to_string(&output).unwrap();
      assert!(json.ends_with('\n'));
      assert!(json.find("a.bin").unwrap() < json.find("b.bin").unwrap());
    }
    fs::remove_dir_all(&dir).unwrap();
  }

  #[test]
  fn unsupported_algorithm_fails_before_writing() {
    let dir = std::env::temp_dir().join(format!("parxec-unsupported-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.bin"), b"data").unwrap();
    let output = dir.join("hashes.json");
    let args = HashArgs {
      input_dir: dir.clone(),
      hash_output: output.clone(),
      hashing: cli::HashOptions {
        hash_algorithm: HashAlgorithm::Downsampled,
        hash_threads: 0,
        tile_size: std::num::NonZeroUsize::new(8).unwrap(),
      },
      jobs: std::num::NonZeroUsize::new(4).unwrap(),
      file_limit: 0,
      file_ms: 1000,
    };
    assert!(
      hash(args)
        .unwrap_err()
        .to_string()
        .contains("not implemented yet")
    );
    assert!(!output.exists());
    fs::remove_dir_all(dir).unwrap();
  }
}

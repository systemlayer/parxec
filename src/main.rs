#[allow(dead_code)]
mod cli;
mod grouping;
mod hash_file;
mod hasher;
mod input;
mod run;
mod stat;

use anyhow::{Context, bail};
use clap::Parser;
use cli::{AnalyzeArgs, Cli, Commands, HashArgs, RunArgs};
use std::{fs, num::NonZeroUsize, time::Instant};
use tokio::sync::watch;

/// Prints file counts, duplicate percentages, and optional measured hashing time.
fn print_file_statistics(stats: &stat::Statistics, hashing_seconds: Option<f64>) {
  println!("Files: {} total, {} distinct hashes.", stats.total_files, stats.distinct_hashes);
  println!("Duplicate groups: {}.", stats.duplicate_groups);
  println!(
    "Files in duplicate groups: {} ({:.1}%).",
    stats.duplicate_files, stats.duplicate_percent
  );
  println!("Redundant files: {} ({:.1}%).", stats.redundant_files, stats.redundant_percent);
  if let Some(seconds) = hashing_seconds {
    println!("Hashing took {:.2}s.", seconds);
  }
}

/// Prints estimated processing times for all, distinct, and redundant files.
fn print_processing_times(stats: &stat::Statistics, jobs: NonZeroUsize, file_ms: u64) {
  println!("Estimated processing at {}ms per file with {} jobs:", file_ms, jobs);
  println!("  All files: {:.2}s.", stats.estimated_all_seconds);
  println!("  Distinct files: {:.2}s.", stats.estimated_unique_seconds);
  println!("  Time saved: {:.2}s.", stats.estimated_saved_seconds);
}

/// Hashes selected files and writes a JSON hash file.
fn hash(args: HashArgs) -> anyhow::Result<()> {
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
  let hashes = hasher::hash_files(
    &args.input_dir,
    &names,
    args.hashing.hash_threads,
    args.hashing.hash_algorithm,
    args.hashing.tile_size.get(),
  )?;
  let elapsed = start.elapsed();
  hash_file::write(&args.hash_output, &hashes)?;
  let grouping = grouping::group(&hashes);
  println!("Saved hashes to {}.", args.hash_output.display());
  println!();

  let stats = stat::calculate(&grouping, args.jobs, args.file_ms);
  print_file_statistics(&stats, Some(elapsed.as_secs_f64()));
  println!();

  print_processing_times(&stats, args.jobs, args.file_ms);
  Ok(())
}

/// Reads a saved hash file and reports its duplicate statistics and estimates.
fn analyze(args: AnalyzeArgs) -> anyhow::Result<()> {
  let hashes = hash_file::read(&args.hash_input)?;
  let grouping = grouping::group(&hashes);

  let stats = stat::calculate(&grouping, args.jobs, args.file_ms);
  print_file_statistics(&stats, None);
  println!();

  print_processing_times(&stats, args.jobs, args.file_ms);
  Ok(())
}

async fn run(args: RunArgs) -> anyhow::Result<run::ExecutionOutcome> {
  println!("{args:?}");
  println!();

  let names = input::discover_files(&args.input_dir)?;

  println!("Hashing {} files.", names.len());
  let start = Instant::now();
  let hashes = run::resolve_hashes(&args, &names)?;
  let elapsed = start.elapsed();
  let grouping = grouping::group(&hashes);
  println!();

  let stats = stat::calculate(&grouping, args.jobs, 0);
  print_file_statistics(&stats, Some(elapsed.as_secs_f64()));
  println!();

  let plan =
    run::prepare_plan(&args.input_dir, &args.output_dir, args.jobs, &args.command, grouping)?;
  if args.dry_run {
    run::write_plan(std::io::stdout().lock(), &plan)?;
    return Ok(run::ExecutionOutcome::Completed);
  }
  println!("Processing {} files.", stats.distinct_hashes);
  let (cancellation_sender, cancellation) = watch::channel(false);
  ctrlc::set_handler(move || {
    cancellation_sender.send_replace(true);
  })
  .context("cannot install run interruption handler")?;
  run::with_staged_batches(
    &args.input_dir,
    &plan,
    run::execute_plan(&plan, &args.output_dir, cancellation),
  )
  .await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  match Cli::parse().command {
    Commands::Hash(args) => hash(args),
    Commands::Analyze(args) => analyze(args),
    Commands::Run(args) => match run(args).await? {
      run::ExecutionOutcome::Completed => Ok(()),
      run::ExecutionOutcome::Cancelled => std::process::exit(130),
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cli::HashAlgorithm;

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
  fn decoding_failure_reports_path_without_writing() {
    let dir = std::env::temp_dir().join(format!("parxec-invalid-image-{}", std::process::id()));
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
    let error = hash(args).unwrap_err().to_string();
    assert!(error.contains("cannot decode image"));
    assert!(error.contains(&dir.join("a.bin").display().to_string()));
    assert!(!output.exists());
    fs::remove_dir_all(dir).unwrap();
  }
}

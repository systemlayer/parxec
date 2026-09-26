mod cli;
mod grouping;
mod hash_file;
mod hasher;
mod input;
mod outcome;
mod run;
mod stat;

use anyhow::{Context, bail};
use clap::Parser;
use cli::{AnalyzeArgs, Cli, Commands, HashArgs, RunArgs};
use outcome::CommandOutcome;
use std::{fs, time::Instant};
use tokio::sync::watch;

/// Formats file counts, duplicate percentages, and optional measured hashing time.
fn format_file_statistics(stats: &stat::Statistics, hashing_seconds: Option<f64>) -> String {
  let mut lines = vec![
    format!("Files: {} total, {} distinct hashes.", stats.total_files, stats.distinct_hashes),
    format!("Duplicate groups: {}.", stats.duplicate_groups),
    format!(
      "Files in duplicate groups: {} ({:.1}%).",
      stats.duplicate_files, stats.duplicate_percent
    ),
    format!("Redundant files: {} ({:.1}%).", stats.redundant_files, stats.redundant_percent),
  ];
  if let Some(seconds) = hashing_seconds {
    lines.push(format!("Hashing took {:.2}s.", seconds));
  }
  lines.join("\n")
}

/// Formats processing times for all, distinct, and redundant files.
fn format_processing_times(stats: &stat::Statistics) -> String {
  [
    format!("  All files (without duplicate skipping): {:.2}s.", stats.estimated_all_seconds),
    format!("  Distinct files (actual time): {:.2}s.", stats.estimated_unique_seconds),
    format!("  Time saved (by skipping duplicates): {:.2}s.", stats.estimated_saved_seconds),
  ]
  .join("\n")
}

/// Formats run arguments as a labeled, human-readable section.
fn format_run_args(args: &RunArgs) -> String {
  let hash_input = args
    .hash_input
    .as_deref()
    .map_or_else(|| "not provided".to_owned(), |path| path.to_string_lossy().into_owned());
  let hash_threads = match args.hashing.hash_threads {
    0 => "automatic".to_owned(),
    count => count.to_string(),
  };
  let command = args
    .command
    .iter()
    .map(|arg| arg.to_string_lossy())
    .collect::<Vec<_>>()
    .join(" ");
  [
    "Arguments:".to_owned(),
    format!("  Input directory: {}.", args.input_dir.display()),
    format!("  Output directory: {}.", args.output_dir.display()),
    format!("  Hash input: {hash_input}."),
    format!("  Hash algorithm: {}.", args.hashing.hash_algorithm),
    format!("  Hash threads: {hash_threads}."),
    format!("  Tile size: {}px.", args.hashing.tile_size),
    format!("  Jobs: {}.", args.jobs),
    format!("  Dry run: {}.", if args.dry_run { "yes" } else { "no" }),
    format!("  Command: {command}."),
  ]
  .join("\n")
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
  println!("{}", format_file_statistics(&stats, Some(elapsed.as_secs_f64())));
  println!();

  println!("Estimated processing at {}ms per file with {} jobs:", args.file_ms, args.jobs);
  println!("{}", format_processing_times(&stats));
  Ok(())
}

/// Reads a saved hash file and reports its duplicate statistics and estimates.
fn analyze(args: AnalyzeArgs) -> anyhow::Result<()> {
  let hashes = hash_file::read(&args.hash_input)?;
  let grouping = grouping::group(&hashes);

  let stats = stat::calculate(&grouping, args.jobs, args.file_ms);
  println!("{}", format_file_statistics(&stats, None));
  println!();

  println!("Estimated processing at {}ms per file with {} jobs:", args.file_ms, args.jobs);
  println!("{}", format_processing_times(&stats));
  Ok(())
}

async fn run(args: RunArgs) -> anyhow::Result<CommandOutcome> {
  println!("{}", format_run_args(&args));
  println!();

  let names = input::discover_files(&args.input_dir)?;

  println!("Hashing {} files.", names.len());
  let start = Instant::now();
  let hashes = run::resolve_hashes(&args, &names)?;
  let elapsed = start.elapsed();
  let grouping = grouping::group(&hashes);
  println!();

  let mut stats = stat::calculate(&grouping, args.jobs, 0);
  println!("{}", format_file_statistics(&stats, Some(elapsed.as_secs_f64())));
  println!();

  let plan =
    run::prepare_plan(&args.input_dir, &args.output_dir, args.jobs, &args.command, grouping)?;
  if args.dry_run {
    run::write_plan(std::io::stdout().lock(), &plan)?;
    return Ok(CommandOutcome::Completed);
  }
  println!("Processing {} files...", stats.distinct_hashes);
  let (cancellation_sender, cancellation) = watch::channel(false);
  ctrlc::set_handler(move || {
    cancellation_sender.send_replace(true);
  })
  .context("cannot install run interruption handler")?;
  let start = Instant::now();
  let outcome = run::with_staged_batches(
    &args.input_dir,
    &plan,
    run::execute_plan(&plan, &args.output_dir, cancellation),
  )
  .await?;
  let processing_seconds = start.elapsed().as_secs_f64();
  println!();

  if outcome == CommandOutcome::Completed {
    if !plan.redundant.is_empty() {
      println!("Hard linking {} redundant files.", plan.redundant.len());
      run::link_redundant_outputs(&args.output_dir, &plan.redundant)?;
    }
    stat::extrapolate_processing_times(&mut stats, processing_seconds);
    println!();
    println!("Processing times:");
    println!("{}", format_processing_times(&stats));
  }
  Ok(outcome)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  match Cli::parse().command {
    Commands::Hash(args) => hash(args),
    Commands::Analyze(args) => analyze(args),
    Commands::Run(args) => match run(args).await? {
      CommandOutcome::Completed => Ok(()),
      CommandOutcome::Cancelled => std::process::exit(130),
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cli::{HashAlgorithm, HashOptions};

  /// Returns representative values for testing human-readable statistics.
  fn sample_statistics() -> stat::Statistics {
    stat::Statistics {
      total_files: 8,
      distinct_hashes: 5,
      duplicate_groups: 2,
      duplicate_files: 5,
      redundant_files: 3,
      duplicate_percent: 62.5,
      redundant_percent: 37.5,
      estimated_all_seconds: 4.0,
      estimated_unique_seconds: 2.5,
      estimated_saved_seconds: 1.5,
    }
  }

  #[test]
  fn formats_file_statistics_with_optional_hashing_time() {
    let stats = sample_statistics();
    let counts = "Files: 8 total, 5 distinct hashes.\nDuplicate groups: 2.\nFiles in duplicate groups: 5 (62.5%).\nRedundant files: 3 (37.5%).";
    assert_eq!(format_file_statistics(&stats, None), counts);
    assert_eq!(
      format_file_statistics(&stats, Some(1.234)),
      format!("{counts}\nHashing took 1.23s.")
    );
  }

  #[test]
  fn formats_processing_times() {
    assert_eq!(
      format_processing_times(&sample_statistics()),
      "  All files (without duplicate skipping): 4.00s.\n  Distinct files (actual time): 2.50s.\n  Time saved (by skipping duplicates): 1.50s."
    );
  }

  #[test]
  fn formats_run_arguments_for_people() {
    let mut args = RunArgs {
      input_dir: "input files".into(),
      output_dir: "output".into(),
      hash_input: Some("hashes.json".into()),
      hashing: HashOptions {
        hash_algorithm: HashAlgorithm::Sha256,
        hash_threads: 2,
        tile_size: std::num::NonZeroUsize::new(16).unwrap(),
      },
      jobs: std::num::NonZeroUsize::new(4).unwrap(),
      dry_run: true,
      command: ["processor", "two words"]
        .into_iter()
        .map(Into::into)
        .collect(),
    };
    assert_eq!(
      format_run_args(&args),
      "Arguments:\n  Input directory: input files.\n  Output directory: output.\n  Hash input: hashes.json.\n  Hash algorithm: SHA-256.\n  Hash threads: 2.\n  Tile size: 16px.\n  Jobs: 4.\n  Dry run: yes.\n  Command: processor two words."
    );
    args.hash_input = None;
    args.hashing.hash_threads = 0;
    assert!(format_run_args(&args).contains(
      "  Hash input: not provided.\n  Hash algorithm: SHA-256.\n  Hash threads: automatic."
    ));
  }

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

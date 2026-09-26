use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{ffi::OsString, fmt, num::NonZeroUsize, path::PathBuf};

#[derive(Parser)]
#[command(about = "Process file collections and run commands in parallel")]
#[command(version)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct Cli {
  #[command(subcommand)]
  pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
  /// Compute file hashes and save them as JSON.
  Hash(HashArgs),
  /// Analyze a JSON file containing hashes.
  Analyze(AnalyzeArgs),
  /// Run an external command against file batches.
  Run(RunArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HashAlgorithm {
  Sha256,
  Downsampled,
}

/// Formats a hashing algorithm for user-facing output.
impl fmt::Display for HashAlgorithm {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(match self {
      Self::Sha256 => "SHA-256",
      Self::Downsampled => "Downsampled",
    })
  }
}

#[derive(Args)]
pub struct HashOptions {
  /// Algorithm used to hash files; downsampled hashing requires images.
  #[arg(short = 'a', long, value_enum, default_value_t = HashAlgorithm::Downsampled)]
  pub hash_algorithm: HashAlgorithm,
  /// Number of hashing threads; 0 selects the automatic count.
  #[arg(short = 't', long, default_value_t = 0)]
  pub hash_threads: usize,
  /// Width and height of the resized image in pixels for downsampled hashing.
  #[arg(short = 'z', long, default_value = "8")]
  pub tile_size: NonZeroUsize,
}

#[derive(Args)]
pub struct HashArgs {
  /// Directory containing files to hash.
  pub input_dir: PathBuf,
  /// JSON file to write the generated hashes to.
  #[arg(short = 'f', long, required = true)]
  pub hash_output: PathBuf,
  #[command(flatten)]
  pub hashing: HashOptions,
  /// Number of parallel jobs used for estimates.
  #[arg(short = 'j', long, default_value = "4")]
  pub jobs: NonZeroUsize,
  /// Maximum number of files to process; 0 means unlimited.
  #[arg(short = 'l', long, default_value_t = 0)]
  pub file_limit: usize,
  /// Estimated processing time per file in milliseconds; actual time may vary by workload.
  #[arg(short = 'm', long, default_value_t = 50)]
  pub file_ms: u64,
}

#[derive(Args)]
pub struct AnalyzeArgs {
  /// JSON file containing previously computed hashes.
  #[arg(short = 'f', long, required = true)]
  pub hash_input: PathBuf,
  /// Number of parallel jobs used for estimates.
  #[arg(short = 'j', long, default_value = "4")]
  pub jobs: NonZeroUsize,
  /// Estimated processing time per file in milliseconds; actual time may vary by workload.
  #[arg(short = 'm', long, default_value_t = 50)]
  pub file_ms: u64,
}

#[derive(Args)]
pub struct RunArgs {
  /// Directory containing files to process.
  pub input_dir: PathBuf,
  /// Directory for the external command's output.
  #[arg(short = 'o', long, required = true)]
  pub output_dir: PathBuf,
  /// JSON file to load existing hashes from.
  #[arg(short = 'f', long)]
  pub hash_input: Option<PathBuf>,
  #[command(flatten)]
  pub hashing: HashOptions,
  /// Number of parallel jobs to run.
  #[arg(short = 'j', long, default_value = "4")]
  pub jobs: NonZeroUsize,
  /// Compute and print the execution plan without running it.
  #[arg(short = 'n', long)]
  pub dry_run: bool,
  /// Program and arguments to execute, following `--`.
  #[arg(last = true, required = true, num_args = 1..)]
  pub command: Vec<OsString>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use clap::error::ErrorKind;

  #[test]
  fn help_is_available_at_root_and_for_each_command() {
    for args in [
      vec!["parxec", "--help"],
      vec!["parxec", "help"],
      vec!["parxec", "help", "hash"],
      vec!["parxec", "help", "analyze"],
      vec!["parxec", "help", "run"],
    ] {
      let error = Cli::try_parse_from(args)
        .err()
        .expect("help arguments should produce a display error");
      assert_eq!(error.kind(), ErrorKind::DisplayHelp);
    }
  }

  #[test]
  fn version_is_available_at_root() {
    for argument in ["--version", "-V"] {
      let error = Cli::try_parse_from(["parxec", argument])
        .err()
        .expect("version argument should produce a display error");
      assert_eq!(error.kind(), ErrorKind::DisplayVersion);
      assert_eq!(error.to_string(), format!("parxec {}\n", env!("CARGO_PKG_VERSION")));
    }
  }

  #[test]
  fn hash_parses_defaults_and_algorithms() {
    let cli =
      Cli::try_parse_from(["parxec", "hash", "files", "--hash-output", "hashes.json"]).unwrap();
    let Commands::Hash(args) = cli.command else {
      panic!("expected hash command")
    };
    assert_eq!(args.hashing.hash_algorithm, HashAlgorithm::Downsampled);
    assert_eq!(args.hashing.hash_threads, 0);
    assert_eq!(args.hashing.tile_size.get(), 8);
    assert_eq!(args.jobs.get(), 4);
    assert_eq!(args.file_limit, 0);
    assert_eq!(args.file_ms, 50);
    for (value, expected) in [
      ("sha256", HashAlgorithm::Sha256),
      ("downsampled", HashAlgorithm::Downsampled),
    ] {
      let cli = Cli::try_parse_from([
        "parxec",
        "hash",
        "files",
        "-f",
        "hashes.json",
        "--hash-algorithm",
        value,
      ])
      .unwrap();
      let Commands::Hash(args) = cli.command else {
        panic!("expected hash command")
      };
      assert_eq!(args.hashing.hash_algorithm, expected);
    }
  }

  #[test]
  fn hash_algorithms_have_pretty_display_names() {
    assert_eq!(HashAlgorithm::Sha256.to_string(), "SHA-256");
    assert_eq!(HashAlgorithm::Downsampled.to_string(), "Downsampled");
  }

  #[test]
  fn analyze_parses_hash_input_and_estimates() {
    let cli = Cli::try_parse_from(["parxec", "analyze", "--hash-input", "hashes.json"]).unwrap();
    let Commands::Analyze(args) = cli.command else {
      panic!("expected analyze command")
    };
    assert_eq!(args.file_ms, 50);
    let cli = Cli::try_parse_from([
      "parxec",
      "analyze",
      "--hash-input",
      "hashes.json",
      "--jobs",
      "6",
      "--file-ms",
      "250",
    ])
    .unwrap();
    let Commands::Analyze(args) = cli.command else {
      panic!("expected analyze command")
    };
    assert_eq!(args.hash_input, PathBuf::from("hashes.json"));
    assert_eq!(args.jobs.get(), 6);
    assert_eq!(args.file_ms, 250);
  }

  #[test]
  fn run_accepts_optional_hash_input_and_external_flags() {
    let cli = Cli::try_parse_from([
      "parxec",
      "run",
      "files",
      "-o",
      "results",
      "--hash-input",
      "hashes.json",
      "--hash-algorithm",
      "downsampled",
      "--tile-size",
      "12",
      "--dry-run",
      "--",
      "processor",
      "--quality",
      "2",
    ])
    .unwrap();
    let Commands::Run(args) = cli.command else {
      panic!("expected run command")
    };
    assert_eq!(args.hash_input, Some(PathBuf::from("hashes.json")));
    assert_eq!(args.hashing.hash_algorithm, HashAlgorithm::Downsampled);
    assert_eq!(args.hashing.tile_size.get(), 12);
    assert!(args.dry_run);
    assert_eq!(args.command, ["processor", "--quality", "2"].map(OsString::from));
    let cli =
      Cli::try_parse_from(["parxec", "run", "files", "-o", "results", "--", "processor"]).unwrap();
    let Commands::Run(args) = cli.command else {
      panic!("expected run command")
    };
    assert_eq!(args.hash_input, None);
    assert_eq!(args.hashing.hash_algorithm, HashAlgorithm::Downsampled);
    assert!(!args.dry_run);
    let cli = Cli::try_parse_from([
      "parxec",
      "run",
      "files",
      "-o",
      "results",
      "-n",
      "--",
      "processor",
    ])
    .unwrap();
    let Commands::Run(args) = cli.command else {
      panic!("expected run command")
    };
    assert!(args.dry_run);
  }

  #[test]
  fn invalid_and_missing_arguments_are_rejected() {
    for args in [
      vec!["parxec"],
      vec!["parxec", "hash", "files"],
      vec!["parxec", "analyze"],
      vec!["parxec", "run", "files", "-o", "results"],
      vec!["parxec", "hash", "files", "-f", "hashes.json", "-j", "0"],
      vec!["parxec", "hash", "files", "-f", "hashes.json", "-z", "0"],
      vec![
        "parxec",
        "hash",
        "files",
        "-f",
        "hashes.json",
        "-a",
        "unknown",
      ],
    ] {
      assert!(Cli::try_parse_from(args).is_err());
    }
  }
}

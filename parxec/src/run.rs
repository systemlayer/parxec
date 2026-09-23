use crate::{cli::RunArgs, grouping, hash_file, hasher, input};
use anyhow::{Context, bail, ensure};
use serde::{Serialize, Serializer, ser::Error};
use std::{
  collections::BTreeMap,
  ffi::{OsStr, OsString},
  fs,
  io::Write,
  path::{Path, PathBuf},
};

/// A program and its arguments, retained as separate operating-system strings.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PreparedCommand {
  /// Executable invoked for this batch.
  #[serde(serialize_with = "serialize_os_string")]
  pub program: OsString,
  /// Arguments passed directly to the executable, without shell parsing.
  #[serde(serialize_with = "serialize_os_strings")]
  pub arguments: Vec<OsString>,
}

/// One nonempty set of representative files assigned to a staging directory.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Batch {
  /// Staging directory from which the batch command reads its inputs.
  pub input_dir: PathBuf,
  /// Relative paths of representative input files assigned to the staging directory.
  pub files: Vec<PathBuf>,
  /// Fully substituted command that processes this batch.
  pub command: PreparedCommand,
}

/// Work prepared for future execution and duplicate-output reconstruction.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct RunPlan {
  /// Nonempty batches prepared for execution.
  pub batches: Vec<Batch>,
  /// Maps a duplicate file's relative path (key) to the processed file's relative path (value).
  pub redundant: BTreeMap<String, String>,
}

/// Serializes one operating-system string as a JSON string.
fn serialize_os_string<S: Serializer>(value: &OsString, serializer: S) -> Result<S::Ok, S::Error> {
  let value = value
    .to_str()
    .ok_or_else(|| S::Error::custom("command element is not valid UTF-8"))?;
  serializer.serialize_str(value)
}

/// Serializes operating-system strings as a JSON string array.
fn serialize_os_strings<S: Serializer>(
  values: &[OsString],
  serializer: S,
) -> Result<S::Ok, S::Error> {
  let values = values
    .iter()
    .map(|value| {
      value
        .to_str()
        .ok_or_else(|| S::Error::custom("command element is not valid UTF-8"))
    })
    .collect::<Result<Vec<_>, _>>()?;
  values.serialize(serializer)
}

/// Writes a run plan as pretty JSON followed by a newline.
pub fn write_plan(mut writer: impl Write, plan: &RunPlan) -> anyhow::Result<()> {
  serde_json::to_writer_pretty(&mut writer, plan).context("cannot serialize run plan")?;
  writer.write_all(b"\n").context("cannot write run plan")
}

/// Replaces supported directory placeholders in one UTF-8 command element.
fn substitute(template: &OsStr, input: &OsStr, output: &OsStr) -> anyhow::Result<OsString> {
  let template = template
    .to_str()
    .context("command argument is not valid UTF-8")?;
  let input = input
    .to_str()
    .context("input directory is not valid UTF-8")?;
  let output = output
    .to_str()
    .context("output directory is not valid UTF-8")?;
  Ok(
    template
      .replace("{input_dir}", input)
      .replace("{output_dir}", output)
      .into(),
  )
}

/// Ensures the requested output directory exists and contains no entries.
fn validate_output_dir(path: &Path) -> anyhow::Result<()> {
  let mut entries = fs::read_dir(path)
    .with_context(|| format!("cannot read output directory {}", path.display()))?;
  let first = entries
    .next()
    .transpose()
    .with_context(|| format!("cannot read entry in output directory {}", path.display()))?;
  ensure!(first.is_none(), "output directory {} is not empty", path.display());
  Ok(())
}

/// Loads supplied hashes or computes hashes for all selected input files.
fn resolve_hashes(args: &RunArgs, names: &[PathBuf]) -> anyhow::Result<hash_file::HashFile> {
  if let Some(path) = &args.hash_input {
    let supplied = hash_file::read(path)?;
    let mut selected = hash_file::HashFile::new();
    let mut missing = Vec::new();
    for name in names {
      let key = name
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("input filename {} is not valid UTF-8", name.display()))?;
      match supplied.get(key) {
        Some(hash) => {
          selected.insert(key.to_owned(), hash.clone());
        }
        None => missing.push(key),
      }
    }
    if !missing.is_empty() {
      bail!("hash input is missing entries for: {}", missing.join(", "));
    }
    return Ok(selected);
  }
  hasher::hash_files(
    &args.input_dir,
    names,
    args.hashing.hash_threads,
    args.hashing.hash_algorithm,
    args.hashing.tile_size.get(),
  )
}

/// Prepares one command without combining arguments into a shell command line.
fn prepare_command(
  command_elements: &[OsString],
  input_dir: &Path,
  output_dir: &Path,
) -> anyhow::Result<PreparedCommand> {
  let elements = command_elements
    .iter()
    .map(|element| substitute(element, input_dir.as_os_str(), output_dir.as_os_str()))
    .collect::<anyhow::Result<Vec<_>>>()?;
  let (program, arguments) = elements
    .split_first()
    .context("run command is missing a program")?;
  Ok(PreparedCommand {
    program: program.clone(),
    arguments: arguments.to_vec(),
  })
}

/// Assigns equal-sized batches and places remaining representatives in the last batch.
fn prepare_batches(args: &RunArgs, representatives: &[PathBuf]) -> anyhow::Result<Vec<Batch>> {
  let batch_count = args.jobs.get().min(representatives.len());
  let files_per_batch = representatives.len() / batch_count;
  let width = args.jobs.get().to_string().len();
  let mut batches = Vec::with_capacity(batch_count);
  for index in 0..batch_count {
    let start = index * files_per_batch;
    let end = if index + 1 == batch_count {
      representatives.len()
    } else {
      start + files_per_batch
    };
    let files = representatives[start..end].to_vec();
    let input_dir = args
      .input_dir
      .join(format!(".parxec-batch-{index:0width$}"));
    let command = prepare_command(&args.command, &input_dir, &args.output_dir)?;
    batches.push(Batch {
      input_dir,
      files,
      command,
    });
  }
  Ok(batches)
}

/// Validates inputs and prepares all directory-batched work for a future executor.
pub fn prepare_plan(args: &RunArgs) -> anyhow::Result<RunPlan> {
  validate_output_dir(&args.output_dir)?;
  let names = input::discover_files(&args.input_dir)?;
  let hashes = resolve_hashes(args, &names)?;
  let grouping = grouping::group(&hashes);
  let mut representatives = grouping
    .groups
    .values()
    .filter_map(|names| names.first().map(PathBuf::from))
    .collect::<Vec<_>>();
  representatives.sort();
  let batches = prepare_batches(args, &representatives)?;
  Ok(RunPlan {
    batches,
    redundant: grouping.redundant,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cli::{HashAlgorithm, HashOptions};
  use std::{
    num::NonZeroUsize,
    sync::atomic::{AtomicU64, Ordering},
  };

  /// Assigns each test directory a unique suffix within this process.
  static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

  /// Removes its temporary test directory when it leaves scope.
  struct TestDir(PathBuf);

  impl TestDir {
    fn new() -> Self {
      let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
      let path = std::env::temp_dir().join(format!("parxec-run-{}-{id}", std::process::id()));
      fs::create_dir(&path).unwrap();
      Self(path)
    }
  }

  impl Drop for TestDir {
    fn drop(&mut self) {
      fs::remove_dir_all(&self.0).ok();
    }
  }

  fn args(input_dir: PathBuf, output_dir: PathBuf, jobs: usize) -> RunArgs {
    RunArgs {
      input_dir,
      output_dir,
      hash_input: None,
      hashing: HashOptions {
        hash_algorithm: HashAlgorithm::Sha256,
        hash_threads: 1,
        tile_size: NonZeroUsize::new(8).unwrap(),
      },
      jobs: NonZeroUsize::new(jobs).unwrap(),
      dry_run: false,
      command: [
        "processor",
        "--input={input_dir}",
        "two words",
        "{file_name}",
        "{output_dir}",
      ]
      .map(OsString::from)
      .to_vec(),
    }
  }

  #[test]
  fn writes_pretty_json_with_a_final_newline() {
    let plan = RunPlan {
      batches: vec![Batch {
        input_dir: PathBuf::from("input/.parxec-batch-0"),
        files: vec![PathBuf::from("a.bin")],
        command: PreparedCommand {
          program: OsString::from("processor"),
          arguments: vec![OsString::from("--output=results")],
        },
      }],
      redundant: BTreeMap::from([("b.bin".into(), "a.bin".into())]),
    };
    let mut output = Vec::new();
    write_plan(&mut output, &plan).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.ends_with("\n"));
    assert!(text.contains("\n  \"batches\": [\n"));
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["batches"][0]["command"]["program"], "processor");
    assert_eq!(json["redundant"]["b.bin"], "a.bin");
  }

  #[test]
  fn places_remaining_files_in_last_batch_and_preserves_arguments() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    for index in 0..10 {
      fs::write(input_dir.join(format!("{index:02}.bin")), [index]).unwrap();
    }
    let plan = prepare_plan(&args(input_dir.clone(), output_dir.clone(), 6)).unwrap();
    assert_eq!(
      plan
        .batches
        .iter()
        .map(|batch| batch.files.len())
        .collect::<Vec<_>>(),
      [1, 1, 1, 1, 1, 5]
    );
    assert_eq!(plan.batches[0].command.program, "processor");
    assert_eq!(plan.batches[0].command.arguments[1], "two words");
    assert_eq!(plan.batches[0].command.arguments[2], "{file_name}");
    assert_eq!(plan.batches[0].command.arguments[3], output_dir.as_os_str());
    assert!(
      plan.batches[0].command.arguments[0]
        .to_string_lossy()
        .contains(".parxec-batch-0")
    );
    assert!(!plan.batches[0].input_dir.exists());
    assert_eq!(input::discover_files(&input_dir).unwrap().len(), 10);
  }

  #[test]
  fn supplied_hashes_are_filtered_and_missing_entries_are_reported() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    let hash_path = root.0.join("hashes.json");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    fs::write(input_dir.join("a.bin"), []).unwrap();
    fs::write(input_dir.join("b.bin"), []).unwrap();
    let hashes = hash_file::HashFile::from([
      ("a.bin".into(), "same".into()),
      ("b.bin".into(), "same".into()),
      ("extra.bin".into(), "other".into()),
    ]);
    hash_file::write(&hash_path, &hashes).unwrap();
    let mut run_args = args(input_dir, output_dir, 4);
    run_args.hash_input = Some(hash_path.clone());
    let plan = prepare_plan(&run_args).unwrap();
    assert_eq!(plan.batches.len(), 1);
    assert_eq!(plan.batches[0].files, [PathBuf::from("a.bin")]);
    assert_eq!(plan.redundant["b.bin"], "a.bin");
    let hashes = hash_file::HashFile::from([("a.bin".into(), "same".into())]);
    hash_file::write(&hash_path, &hashes).unwrap();
    let error = prepare_plan(&run_args).unwrap_err().to_string();
    assert!(error.contains("missing entries for: b.bin"));
  }

  #[test]
  fn output_directory_must_exist_and_be_empty() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    fs::create_dir(&input_dir).unwrap();
    fs::write(input_dir.join("a.bin"), []).unwrap();
    let missing = root.0.join("missing");
    let error = prepare_plan(&args(input_dir.clone(), missing, 1)).unwrap_err();
    assert!(error.to_string().contains("cannot read output directory"));
    let output_file = root.0.join("file");
    fs::write(&output_file, []).unwrap();
    let error = prepare_plan(&args(input_dir.clone(), output_file, 1)).unwrap_err();
    assert!(error.to_string().contains("cannot read output directory"));
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    fs::write(output_dir.join("old.bin"), []).unwrap();
    let error = prepare_plan(&args(input_dir, output_dir, 1)).unwrap_err();
    assert!(error.to_string().contains("is not empty"));
  }

  #[test]
  fn uses_one_batch_per_representative_when_jobs_are_plentiful() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    for index in 0..3 {
      fs::write(input_dir.join(format!("{index}.bin")), [index]).unwrap();
    }
    let plan = prepare_plan(&args(input_dir, output_dir, 8)).unwrap();
    assert_eq!(plan.batches.len(), 3);
    assert!(plan.batches.iter().all(|batch| batch.files.len() == 1));
  }
}

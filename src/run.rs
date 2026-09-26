use crate::{cli::RunArgs, grouping, hash_file, hasher, outcome::CommandOutcome};
use anyhow::{Context, bail, ensure};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Serialize, Serializer, ser::Error};
use std::{
  collections::{BTreeMap, VecDeque},
  ffi::{OsStr, OsString},
  fs,
  future::Future,
  io::{ErrorKind, Write},
  num::NonZeroUsize,
  path::{Path, PathBuf},
  process::{ExitStatus, Stdio},
  sync::Arc,
  time::Duration,
};
use tokio::{
  io::{AsyncBufReadExt, AsyncRead, BufReader},
  process::{Child, Command},
  sync::{Mutex, watch},
  task::{JoinHandle, JoinSet},
  time::{self, MissedTickBehavior},
};

/// Maximum number of combined output lines retained for a failed batch.
const ERROR_OUTPUT_LINES: usize = 50;

/// Delay between non-recursive output directory scans.
const OUTPUT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Refresh rate for elapsed time and progress rendering between directory scans.
const PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Layout of the single progress bar displayed during batch execution.
const PROGRESS_TEMPLATE: &str =
  "{elapsed_precise} [{wide_bar:.cyan/blue}] {percent:>3}% | {msg}/{len}";

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

/// Completion details from one asynchronously monitored batch process.
struct BatchResult {
  /// Zero-based position of the batch in the run plan.
  batch_index: usize,
  /// Process exit status, or `None` when cancellation terminated the process.
  status: Option<ExitStatus>,
  /// Bounded combined tail of the process's stdout and stderr.
  output_tail: VecDeque<Vec<u8>>,
}

/// Batch directories created for one run and removed when staging ends.
struct StagedBatches {
  directories: Vec<PathBuf>,
}

impl StagedBatches {
  /// Creates and populates all batch directories, rolling them back on failure.
  fn create(input_dir: &Path, batches: &[Batch]) -> anyhow::Result<Self> {
    let mut staged = Self {
      directories: Vec::with_capacity(batches.len()),
    };
    if let Err(error) = staged.populate(input_dir, batches) {
      return match staged.cleanup() {
        Ok(()) => Err(error),
        Err(cleanup) => Err(error.context(format!("additionally, {cleanup:#}"))),
      };
    }
    Ok(staged)
  }

  /// Creates each batch directory and hardlinks its representative files into it.
  fn populate(&mut self, input_dir: &Path, batches: &[Batch]) -> anyhow::Result<()> {
    for batch in batches {
      fs::create_dir(&batch.input_dir)
        .with_context(|| format!("cannot create batch directory {}", batch.input_dir.display()))?;
      self.directories.push(batch.input_dir.clone());
      for file in &batch.files {
        let file_name = file
          .file_name()
          .with_context(|| format!("input path {} has no filename", file.display()))?;
        let source = input_dir.join(file);
        let destination = batch.input_dir.join(file_name);
        fs::hard_link(&source, &destination).with_context(|| {
          format!("cannot hardlink {} to {}", source.display(), destination.display())
        })?;
      }
    }
    Ok(())
  }

  /// Removes every batch directory created by this staging lifecycle.
  fn cleanup(&mut self) -> anyhow::Result<()> {
    let mut first_error = None;
    for directory in self.directories.drain(..).rev() {
      if let Err(error) = fs::remove_dir_all(&directory) {
        first_error.get_or_insert_with(|| {
          anyhow::Error::new(error)
            .context(format!("cannot remove batch directory {}", directory.display()))
        });
      }
    }
    first_error.map_or(Ok(()), Err)
  }
}

impl Drop for StagedBatches {
  fn drop(&mut self) {
    self.cleanup().ok();
  }
}

/// Stages planned hardlinks while an operation runs and cleans them afterward.
pub async fn with_staged_batches<T>(
  input_dir: &Path,
  plan: &RunPlan,
  operation: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
  let mut staged = StagedBatches::create(input_dir, &plan.batches)?;
  let result = operation.await;
  let cleanup = staged.cleanup();
  match (result, cleanup) {
    (Ok(value), Ok(())) => Ok(value),
    (Err(error), Ok(())) => Err(error),
    (Ok(_), Err(error)) => Err(error),
    (Err(error), Err(cleanup)) => Err(error.context(format!("additionally, {cleanup:#}"))),
  }
}

/// Adds completed lines to a shared tail while discarding older process output.
///
/// `Arc` shares the tail between stdout and stderr tasks, `Mutex` serializes their updates,
/// and `VecDeque<Vec<u8>>` provides FIFO eviction while preserving non-UTF-8 output bytes.
async fn drain_output(
  reader: impl AsyncRead + Unpin,
  output_tail: Arc<Mutex<VecDeque<Vec<u8>>>>,
) -> anyhow::Result<()> {
  // Buffer each pipe so output can be retained one complete line at a time.
  let mut reader = BufReader::new(reader);
  loop {
    // Read through the delimiter and stop normally when the child closes the pipe.
    let mut line = Vec::new();
    if reader
      .read_until(b'\n', &mut line)
      .await
      .context("cannot read batch output")?
      == 0
    {
      return Ok(());
    }
    // Serialize stdout/stderr arrivals into one tail and evict its oldest line at capacity.
    let mut output_tail = output_tail.lock().await;
    if output_tail.len() == ERROR_OUTPUT_LINES {
      output_tail.pop_front();
    }
    // Preserve the original bytes so invalid UTF-8 remains available for lossy error reporting.
    output_tail.push_back(line);
  }
}

/// Propagates output-reader errors and unexpected task termination.
async fn join_output_task(task: JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
  task.await.context("batch output reader task failed")?
}

/// Waits for one child while draining its output and honoring run cancellation.
async fn monitor_batch(
  batch_index: usize,
  mut child: Child,
  mut cancellation: watch::Receiver<bool>,
) -> anyhow::Result<BatchResult> {
  let stdout = child
    .stdout
    .take()
    .expect("batch stdout should be piped before monitoring");
  let stderr = child
    .stderr
    .take()
    .expect("batch stderr should be piped before monitoring");
  let output_tail = Arc::new(Mutex::new(VecDeque::with_capacity(ERROR_OUTPUT_LINES)));
  let stdout_task = tokio::spawn(drain_output(stdout, Arc::clone(&output_tail)));
  let stderr_task = tokio::spawn(drain_output(stderr, Arc::clone(&output_tail)));
  let status: Option<ExitStatus> = tokio::select! {
    status = child.wait() => Some(status.context("cannot wait for batch process")?),
    changed = cancellation.changed() => {
      changed.context("batch cancellation channel closed unexpectedly")?;
      child.kill().await.context("cannot terminate batch process")?;
      None
    }
  };
  join_output_task(stdout_task).await?;
  join_output_task(stderr_task).await?;
  let output_tail = Arc::try_unwrap(output_tail)
    .map_err(|_| anyhow::anyhow!("batch output tail still has active readers"))?
    .into_inner();
  Ok(BatchResult {
    batch_index,
    status,
    output_tail,
  })
}

/// Counts top-level paths resolving to regular files without changing the directory.
async fn count_output_files(output_dir: &Path) -> anyhow::Result<u64> {
  let mut entries = tokio::fs::read_dir(output_dir)
    .await
    .with_context(|| format!("cannot read output directory {}", output_dir.display()))?;
  let mut count = 0;
  while let Some(entry) = entries
    .next_entry()
    .await
    .with_context(|| format!("cannot read entry in output directory {}", output_dir.display()))?
  {
    match tokio::fs::metadata(entry.path()).await {
      Ok(metadata) if metadata.is_file() => count += 1,
      Ok(_) => {}
      Err(error) if error.kind() == ErrorKind::NotFound => {}
      Err(error) => {
        return Err(error)
          .context(format!("cannot inspect output path {}", entry.path().display()));
      }
    }
  }
  Ok(count)
}

/// Updates both the bounded bar position and the uncapped observed file count.
fn update_progress(progress: &ProgressBar, output_files: u64, representatives: u64) {
  progress.set_position(output_files.min(representatives));
  progress.set_message(output_files.to_string());
}

/// Formats a failed status and the configured tail of combined process output.
fn batch_exit_error(result: BatchResult) -> anyhow::Error {
  let status = result
    .status
    .expect("failed batch should have an exit status");
  let mut message = format!("batch {} command exited with {status}", result.batch_index + 1);
  if !result.output_tail.is_empty() {
    message.push_str(&format!("\nlast {} lines of stdout/stderr:\n", result.output_tail.len()));
    for line in result.output_tail {
      message.push_str(&String::from_utf8_lossy(&line));
      if !line.ends_with(b"\n") {
        message.push('\n');
      }
    }
  }
  anyhow::anyhow!(message.trim_end().to_owned())
}

/// Cancels all active batches and waits until their worker tasks have ended.
async fn cancel_batches(
  cancellation: &watch::Sender<bool>,
  tasks: &mut JoinSet<anyhow::Result<BatchResult>>,
) {
  cancellation.send_replace(true);
  while tasks.join_next().await.is_some() {}
}

/// Executes every planned batch concurrently and reports output-file progress.
pub async fn execute_plan(
  plan: &RunPlan,
  output_dir: &Path,
  mut interrupted: watch::Receiver<bool>,
) -> anyhow::Result<CommandOutcome> {
  if *interrupted.borrow() {
    return Ok(CommandOutcome::Cancelled);
  }
  let total_representatives = plan
    .batches
    .iter()
    .map(|batch| batch.files.len() as u64)
    .sum();
  let progress = ProgressBar::new(total_representatives);
  let style =
    ProgressStyle::with_template(PROGRESS_TEMPLATE).context("invalid progress template")?;
  progress.set_style(style.progress_chars("=>-"));
  progress.enable_steady_tick(PROGRESS_TICK_INTERVAL);

  // Share one cancellation signal across all batch monitors and collect their results together.
  let (cancellation, cancellation_receiver) = watch::channel(false);
  let mut tasks = JoinSet::new();

  // Launch every batch concurrently, cancelling already-running batches if a later launch fails.
  for (batch_index, batch) in plan.batches.iter().enumerate() {
    if *interrupted.borrow() {
      cancel_batches(&cancellation, &mut tasks).await;
      progress.finish_and_clear();
      return Ok(CommandOutcome::Cancelled);
    }
    let child = Command::new(&batch.command.program)
      .args(&batch.command.arguments)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true)
      .spawn()
      .with_context(|| format!("cannot launch batch {} command", batch_index + 1));
    let child = match child {
      Ok(child) => child,
      Err(error) => {
        cancel_batches(&cancellation, &mut tasks).await;
        progress.finish_and_clear();
        return Err(error);
      }
    };
    tasks.spawn(monitor_batch(batch_index, child, cancellation_receiver.clone()));
  }

  // Poll output counts at a fixed cadence without accumulating delayed ticks under load.
  let mut poll = time::interval(OUTPUT_POLL_INTERVAL);
  poll.set_missed_tick_behavior(MissedTickBehavior::Skip);

  // Update progress while batches run, stopping all remaining work on the first failure.
  while !tasks.is_empty() {
    tokio::select! {
      biased;
      changed = interrupted.changed() => {
        changed.context("run interruption channel closed unexpectedly")?;
        cancel_batches(&cancellation, &mut tasks).await;
        progress.finish_and_clear();
        return Ok(CommandOutcome::Cancelled);
      }
      _ = poll.tick() => match count_output_files(output_dir).await {
        Ok(count) => update_progress(&progress, count, total_representatives),
        Err(error) => {
          cancel_batches(&cancellation, &mut tasks).await;
          progress.finish_and_clear();
          return Err(error);
        }
      },
      completed = tasks.join_next() => match completed {
        Some(Ok(Ok(result))) if result.status.is_some_and(|status| !status.success()) => {
          cancel_batches(&cancellation, &mut tasks).await;
          progress.finish_and_clear();
          return Err(batch_exit_error(result));
        }
        Some(Ok(Ok(_))) => {}
        Some(Ok(Err(error))) => {
          cancel_batches(&cancellation, &mut tasks).await;
          progress.finish_and_clear();
          return Err(error);
        }
        Some(Err(error)) => {
          cancel_batches(&cancellation, &mut tasks).await;
          progress.finish_and_clear();
          return Err(error).context("batch monitor task failed");
        }
        None => break,
      }
    }
  }

  // Perform one final scan so the completed progress display reflects all produced files.
  match count_output_files(output_dir).await {
    Ok(count) => {
      update_progress(&progress, count, total_representatives);
      progress.finish();
      Ok(CommandOutcome::Completed)
    }
    Err(error) => {
      progress.finish_and_clear();
      Err(error)
    }
  }
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
pub fn validate_output_dir(path: &Path) -> anyhow::Result<()> {
  let mut entries = fs::read_dir(path)
    .with_context(|| format!("cannot read output directory {}", path.display()))?;
  let first = entries
    .next()
    .transpose()
    .with_context(|| format!("cannot read entry in output directory {}", path.display()))?;
  ensure!(first.is_none(), "output directory {} is not empty", path.display());
  Ok(())
}

/// Creates duplicate output names as hard links to their representative outputs.
pub fn link_redundant_outputs(
  output_dir: &Path,
  redundant: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
  for (duplicate, representative) in redundant {
    let source = output_dir.join(representative);
    let destination = output_dir.join(duplicate);
    fs::hard_link(&source, &destination).with_context(|| {
      format!(
        "cannot hardlink representative output {} to duplicate output {}",
        source.display(),
        destination.display()
      )
    })?;
  }
  Ok(())
}

/// Loads supplied hashes or computes hashes for all selected input files.
pub fn resolve_hashes(args: &RunArgs, names: &[PathBuf]) -> anyhow::Result<hash_file::HashFile> {
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
fn prepare_batches(
  input_dir: &Path,
  output_dir: &Path,
  jobs: NonZeroUsize,
  command_elements: &[OsString],
  representatives: &[PathBuf],
) -> anyhow::Result<Vec<Batch>> {
  let batch_count = jobs.get().min(representatives.len());
  let files_per_batch = representatives.len() / batch_count;
  let width = jobs.get().to_string().len();
  let mut batches = Vec::with_capacity(batch_count);
  for index in 0..batch_count {
    let start = index * files_per_batch;
    let end = if index + 1 == batch_count {
      representatives.len()
    } else {
      start + files_per_batch
    };
    let files = representatives[start..end].to_vec();
    let input_dir = input_dir.join(format!("parxec-batch-{index:0width$}"));
    let command = prepare_command(command_elements, &input_dir, output_dir)?;
    batches.push(Batch {
      input_dir,
      files,
      command,
    });
  }
  Ok(batches)
}

/// Prepares directory-batched work from grouped files.
pub fn prepare_plan(
  input_dir: &Path,
  output_dir: &Path,
  jobs: NonZeroUsize,
  command_elements: &[OsString],
  grouping: grouping::Grouping,
) -> anyhow::Result<RunPlan> {
  let mut representatives = grouping
    .groups
    .values()
    .filter_map(|names| names.first().map(PathBuf::from))
    .collect::<Vec<_>>();
  representatives.sort();
  let batches = prepare_batches(input_dir, output_dir, jobs, command_elements, &representatives)?;
  Ok(RunPlan {
    batches,
    redundant: grouping.redundant,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    cli::{HashAlgorithm, HashOptions},
    input,
  };
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

  fn prepare(args: &RunArgs) -> anyhow::Result<RunPlan> {
    let names = input::discover_files(&args.input_dir)?;
    let hashes = resolve_hashes(args, &names)?;
    let grouping = grouping::group(&hashes);
    prepare_plan(&args.input_dir, &args.output_dir, args.jobs, &args.command, grouping)
  }

  #[cfg(unix)]
  fn execution_plan(commands: Vec<Vec<OsString>>) -> RunPlan {
    let batches = commands
      .into_iter()
      .enumerate()
      .map(|(index, arguments)| Batch {
        input_dir: PathBuf::from(format!("batch-{index}")),
        files: vec![PathBuf::from(format!("{index}.bin"))],
        command: PreparedCommand {
          program: OsString::from("sh"),
          arguments,
        },
      })
      .collect();
    RunPlan {
      batches,
      redundant: BTreeMap::new(),
    }
  }

  #[test]
  fn writes_pretty_json_with_a_final_newline() {
    let plan = RunPlan {
      batches: vec![Batch {
        input_dir: PathBuf::from("input/parxec-batch-0"),
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
  fn distributes_representatives_across_job_counts() {
    let cases: &[(usize, usize, &[usize])] = &[
      (1, 4, &[4]),
      (2, 4, &[2, 2]),
      (3, 8, &[2, 2, 4]),
      (3, 3, &[1, 1, 1]),
      (5, 3, &[1, 1, 1]),
    ];
    for &(jobs, representative_count, expected_sizes) in cases {
      let representatives = (0..representative_count)
        .map(|index| PathBuf::from(format!("{index}.bin")))
        .collect::<Vec<_>>();
      let batches = prepare_batches(
        Path::new("input"),
        Path::new("output"),
        NonZeroUsize::new(jobs).unwrap(),
        &[OsString::from("processor")],
        &representatives,
      )
      .unwrap();
      let mut start = 0;
      for (batch, &expected_size) in batches.iter().zip(expected_sizes) {
        assert_eq!(batch.files, representatives[start..start + expected_size]);
        start += expected_size;
      }
      assert_eq!(batches.len(), expected_sizes.len());
      assert_eq!(start, representatives.len());
    }
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
    let plan = prepare(&args(input_dir.clone(), output_dir.clone(), 6)).unwrap();
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
        .contains("parxec-batch-0")
    );
    assert!(!plan.batches[0].input_dir.exists());
    assert_eq!(input::discover_files(&input_dir).unwrap().len(), 10);
  }

  #[tokio::test]
  async fn stages_hardlinks_and_cleans_up_after_success() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    fs::write(input_dir.join("a.bin"), b"first").unwrap();
    fs::write(input_dir.join("b.bin"), b"second").unwrap();
    let plan = prepare(&args(input_dir.clone(), output_dir, 2)).unwrap();
    let batch_dirs = plan
      .batches
      .iter()
      .map(|batch| batch.input_dir.clone())
      .collect::<Vec<_>>();
    with_staged_batches(&input_dir, &plan, async {
      for batch in &plan.batches {
        assert_eq!(fs::read_dir(&batch.input_dir).unwrap().count(), batch.files.len());
        for file in &batch.files {
          let staged = batch.input_dir.join(file.file_name().unwrap());
          assert_eq!(fs::read(&staged).unwrap(), fs::read(input_dir.join(file)).unwrap());
          #[cfg(unix)]
          {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
              fs::metadata(staged).unwrap().ino(),
              fs::metadata(input_dir.join(file)).unwrap().ino()
            );
          }
        }
      }
      Ok(())
    })
    .await
    .unwrap();
    assert!(batch_dirs.iter().all(|directory| !directory.exists()));
    assert!(input_dir.join("a.bin").exists());
    assert!(input_dir.join("b.bin").exists());
  }

  #[tokio::test]
  async fn cleans_up_staged_batches_after_operation_failure() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    fs::write(input_dir.join("a.bin"), b"first").unwrap();
    let plan = prepare(&args(input_dir.clone(), output_dir, 1)).unwrap();
    let batch_dir = plan.batches[0].input_dir.clone();
    let error = with_staged_batches(&input_dir, &plan, async {
      assert!(batch_dir.join("a.bin").exists());
      anyhow::bail!("operation failed") as anyhow::Result<()>
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("operation failed"));
    assert!(!batch_dir.exists());
    assert!(input_dir.join("a.bin").exists());
  }

  #[tokio::test]
  async fn rolls_back_partial_staging_without_removing_preexisting_paths() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    fs::write(input_dir.join("a.bin"), b"first").unwrap();
    fs::write(input_dir.join("b.bin"), b"second").unwrap();
    let plan = prepare(&args(input_dir.clone(), output_dir, 2)).unwrap();
    let created_dir = plan.batches[0].input_dir.clone();
    let preexisting_dir = plan.batches[1].input_dir.clone();
    fs::create_dir(&preexisting_dir).unwrap();
    fs::write(preexisting_dir.join("keep"), []).unwrap();
    let error = with_staged_batches(&input_dir, &plan, async { Ok(()) })
      .await
      .unwrap_err();
    assert!(error.to_string().contains("cannot create batch directory"));
    assert!(!created_dir.exists());
    assert!(preexisting_dir.join("keep").exists());
  }

  #[tokio::test]
  async fn cancellation_after_staging_rolls_back_batch_directories() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    fs::write(input_dir.join("a.bin"), b"first").unwrap();
    let plan = prepare(&args(input_dir.clone(), output_dir.clone(), 1)).unwrap();
    let batch_dir = plan.batches[0].input_dir.clone();
    let (interruption_sender, interruption) = watch::channel(false);
    let outcome = with_staged_batches(&input_dir, &plan, async {
      assert!(batch_dir.join("a.bin").exists());
      interruption_sender.send_replace(true);
      execute_plan(&plan, &output_dir, interruption).await
    })
    .await
    .unwrap();
    assert_eq!(outcome, CommandOutcome::Cancelled);
    assert!(!batch_dir.exists());
    assert!(input_dir.join("a.bin").exists());
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
    let names = input::discover_files(&run_args.input_dir).unwrap();
    let resolved = resolve_hashes(&run_args, &names).unwrap();
    let grouping = grouping::group(&resolved);
    let plan = prepare_plan(
      &run_args.input_dir,
      &run_args.output_dir,
      run_args.jobs,
      &run_args.command,
      grouping,
    )
    .unwrap();
    assert_eq!(plan.batches.len(), 1);
    assert_eq!(plan.batches[0].files, [PathBuf::from("a.bin")]);
    assert_eq!(plan.redundant["b.bin"], "a.bin");
    let hashes = hash_file::HashFile::from([("a.bin".into(), "same".into())]);
    hash_file::write(&hash_path, &hashes).unwrap();
    let error = resolve_hashes(&run_args, &names).unwrap_err().to_string();
    assert!(error.contains("missing entries for: b.bin"));
  }

  #[test]
  fn output_directory_must_exist_and_be_empty() {
    let root = TestDir::new();
    let missing = root.0.join("missing");
    let error = validate_output_dir(&missing).unwrap_err();
    assert!(error.to_string().contains("cannot read output directory"));
    let output_file = root.0.join("file");
    fs::write(&output_file, []).unwrap();
    let error = validate_output_dir(&output_file).unwrap_err();
    assert!(error.to_string().contains("cannot read output directory"));
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    fs::write(output_dir.join("old.bin"), []).unwrap();
    let error = validate_output_dir(&output_dir).unwrap_err();
    assert!(error.to_string().contains("is not empty"));
  }

  #[test]
  fn links_redundant_outputs_and_preserves_exact_file_inventory() {
    let root = TestDir::new();
    let input_dir = root.0.join("input");
    let output_dir = root.0.join("output");
    fs::create_dir(&input_dir).unwrap();
    fs::create_dir(&output_dir).unwrap();
    for (name, contents) in [
      ("a.bin", b"same".as_slice()),
      ("b.bin", b"same"),
      ("c.bin", b"other"),
      ("d.bin", b"same"),
    ] {
      fs::write(input_dir.join(name), contents).unwrap();
    }
    fs::write(output_dir.join("a.bin"), b"processed same").unwrap();
    fs::write(output_dir.join("c.bin"), b"processed other").unwrap();
    let redundant = BTreeMap::from([
      ("b.bin".into(), "a.bin".into()),
      ("d.bin".into(), "a.bin".into()),
    ]);
    link_redundant_outputs(&output_dir, &redundant).unwrap();
    let names = |directory: &Path| {
      let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
      names.sort();
      names
    };
    assert_eq!(names(&input_dir), names(&output_dir));
    fs::write(output_dir.join("a.bin"), b"updated").unwrap();
    assert_eq!(fs::read(output_dir.join("b.bin")).unwrap(), b"updated");
    assert_eq!(fs::read(output_dir.join("d.bin")).unwrap(), b"updated");
  }

  #[test]
  fn linking_no_redundant_outputs_is_a_noop() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    link_redundant_outputs(&output_dir, &BTreeMap::new()).unwrap();
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 0);
  }

  #[test]
  fn redundant_output_link_errors_include_both_paths() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let source = output_dir.join("representative.bin");
    let destination = output_dir.join("duplicate.bin");
    let redundant = BTreeMap::from([("duplicate.bin".into(), "representative.bin".into())]);
    let missing_error = link_redundant_outputs(&output_dir, &redundant)
      .unwrap_err()
      .to_string();
    assert!(missing_error.contains(&source.display().to_string()));
    assert!(missing_error.contains(&destination.display().to_string()));
    fs::write(&source, b"source").unwrap();
    fs::write(&destination, b"occupied").unwrap();
    let occupied_error = link_redundant_outputs(&output_dir, &redundant)
      .unwrap_err()
      .to_string();
    assert!(occupied_error.contains(&source.display().to_string()));
    assert!(occupied_error.contains(&destination.display().to_string()));
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
    let plan = prepare(&args(input_dir, output_dir, 8)).unwrap();
    assert_eq!(plan.batches.len(), 3);
    assert!(plan.batches.iter().all(|batch| batch.files.len() == 1));
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn executes_all_batches_concurrently_and_discards_success_output() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let scripts = [
      "touch \"$1/started-0\"; while [ ! -e \"$1/started-1\" ]; do sleep 0.01; done; printf 'batch zero output\\n'; touch \"$1/done-0\"",
      "touch \"$1/started-1\"; while [ ! -e \"$1/started-0\" ]; do sleep 0.01; done; printf 'batch one error\\n' >&2; touch \"$1/done-1\"",
    ];
    let commands = scripts
      .map(|script| {
        ["-c", script, "parxec-test", output_dir.to_str().unwrap()]
          .map(OsString::from)
          .to_vec()
      })
      .to_vec();
    let (_interruption_sender, interruption) = watch::channel(false);
    time::timeout(
      Duration::from_secs(2),
      execute_plan(&execution_plan(commands), &output_dir, interruption),
    )
    .await
    .expect("concurrent batches should not deadlock")
    .unwrap();
    assert!(output_dir.join("done-0").exists());
    assert!(output_dir.join("done-1").exists());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn reports_only_the_configured_failure_tail() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let script = "i=1; while [ $i -le 60 ]; do echo line-$i; i=$((i + 1)); done; printf 'stderr-tail\\n' >&2; exit 7";
    let command = vec![OsString::from("-c"), OsString::from(script)];
    let (_interruption_sender, interruption) = watch::channel(false);
    let error = execute_plan(&execution_plan(vec![command]), &output_dir, interruption)
      .await
      .unwrap_err()
      .to_string();
    assert!(error.contains("exit status: 7"));
    assert!(error.contains("last 50 lines of stdout/stderr"));
    assert!(error.contains("line-60"));
    assert!(error.contains("stderr-tail"));
    assert!(!error.contains("line-1\n"));
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn failure_terminates_a_running_sibling() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let commands = vec![
      vec![OsString::from("-c"), OsString::from("sleep 0.1; exit 9")],
      vec![OsString::from("-c"), OsString::from("exec sleep 10")],
    ];
    let (_interruption_sender, interruption) = watch::channel(false);
    let result = time::timeout(
      Duration::from_secs(2),
      execute_plan(&execution_plan(commands), &output_dir, interruption),
    )
    .await
    .expect("failed batch should promptly terminate its sibling");
    assert!(result.unwrap_err().to_string().contains("exit status: 9"));
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn latched_interruption_prevents_batch_launch() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let command = vec![
      OsString::from("-c"),
      OsString::from("touch \"$1/launched\""),
      OsString::from("parxec-test"),
      output_dir.as_os_str().to_owned(),
    ];
    let (interruption_sender, interruption) = watch::channel(false);
    interruption_sender.send_replace(true);
    let outcome = execute_plan(&execution_plan(vec![command]), &output_dir, interruption)
      .await
      .unwrap();
    assert_eq!(outcome, CommandOutcome::Cancelled);
    assert!(!output_dir.join("launched").exists());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn interruption_terminates_and_reaps_running_child() {
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    let command = vec![
      OsString::from("-c"),
      OsString::from("echo $$ > \"$1/pid\"; exec sleep 10"),
      OsString::from("parxec-test"),
      output_dir.as_os_str().to_owned(),
    ];
    let (interruption_sender, interruption) = watch::channel(false);
    let pid_path = output_dir.join("pid");
    let signal_task = tokio::spawn(async move {
      while !pid_path.exists() {
        time::sleep(Duration::from_millis(10)).await;
      }
      interruption_sender.send_replace(true);
    });
    let outcome = time::timeout(
      Duration::from_secs(2),
      execute_plan(&execution_plan(vec![command]), &output_dir, interruption),
    )
    .await
    .expect("interruption should promptly terminate the child")
    .unwrap();
    signal_task.await.unwrap();
    assert_eq!(outcome, CommandOutcome::Cancelled);
    let pid = fs::read_to_string(output_dir.join("pid")).unwrap();
    let running = std::process::Command::new("kill")
      .args(["-0", pid.trim()])
      .stderr(Stdio::null())
      .status()
      .unwrap()
      .success();
    assert!(!running);
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn counts_only_top_level_paths_resolving_to_files() {
    use std::os::unix::fs::symlink;
    let root = TestDir::new();
    let output_dir = root.0.join("output");
    fs::create_dir(&output_dir).unwrap();
    fs::write(output_dir.join("file.bin"), []).unwrap();
    fs::create_dir(output_dir.join("nested")).unwrap();
    fs::write(output_dir.join("nested/ignored.bin"), []).unwrap();
    symlink("file.bin", output_dir.join("link.bin")).unwrap();
    assert_eq!(count_output_files(&output_dir).await.unwrap(), 2);
  }
}

use anyhow::{Context, bail};
use std::{
  fs,
  path::{Path, PathBuf},
};

/// Returns sorted names of top-level regular files in `input_dir`.
///
/// Symlinks to regular files are included. A `file_limit` of zero selects all files;
/// otherwise, the limit is applied after sorting. Returns an error if the directory
/// cannot be read, an entry cannot be inspected, or no files are found.
pub fn discover_files(input_dir: &Path, file_limit: usize) -> anyhow::Result<Vec<PathBuf>> {
  let entries = fs::read_dir(input_dir)
    .with_context(|| format!("cannot read input directory {}", input_dir.display()))?;
  let mut files = Vec::new();
  for entry in entries {
    let entry = entry.with_context(|| format!("cannot read entry in {}", input_dir.display()))?;
    let metadata = fs::metadata(entry.path())
      .with_context(|| format!("cannot inspect input entry {}", entry.path().display()))?;
    if metadata.is_file() {
      files.push(PathBuf::from(entry.file_name()));
    }
  }
  // Sorting deterministically makes processing order predictable, including which files a limit selects.
  files.sort();
  if file_limit > 0 {
    files.truncate(file_limit);
  }
  if files.is_empty() {
    bail!("input directory {} contains no files", input_dir.display());
  }
  Ok(files)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicU64, Ordering};

  /// Assigns each test directory a unique suffix within this process.
  static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

  struct TestDir(PathBuf);

  impl TestDir {
    fn new() -> Self {
      let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
      let path = std::env::temp_dir().join(format!("parxec-input-{}-{id}", std::process::id()));
      fs::create_dir(&path).unwrap();
      Self(path)
    }
  }

  impl Drop for TestDir {
    fn drop(&mut self) {
      fs::remove_dir_all(&self.0).ok();
    }
  }

  #[test]
  fn sorts_before_limiting_and_ignores_subdirectories() {
    let dir = TestDir::new();
    fs::write(dir.0.join("c.png"), []).unwrap();
    fs::write(dir.0.join("a.png"), []).unwrap();
    fs::write(dir.0.join("b.png"), []).unwrap();
    fs::create_dir(dir.0.join("nested")).unwrap();
    fs::write(dir.0.join("nested").join("0.png"), []).unwrap();
    assert_eq!(discover_files(&dir.0, 0).unwrap(), ["a.png", "b.png", "c.png"].map(PathBuf::from));
    assert_eq!(discover_files(&dir.0, 2).unwrap(), ["a.png", "b.png"].map(PathBuf::from));
  }

  #[test]
  fn empty_input_is_an_error() {
    let dir = TestDir::new();
    assert!(
      discover_files(&dir.0, 0)
        .unwrap_err()
        .to_string()
        .contains("contains no files")
    );
    fs::create_dir(dir.0.join("nested")).unwrap();
    assert!(
      discover_files(&dir.0, 1)
        .unwrap_err()
        .to_string()
        .contains("contains no files")
    );
  }

  #[test]
  fn unreadable_inputs_have_directory_context() {
    let dir = TestDir::new();
    for path in [dir.0.join("missing"), dir.0.join("file")] {
      if path.ends_with("file") {
        fs::write(&path, []).unwrap();
      }
      let error = discover_files(&path, 0).unwrap_err().to_string();
      assert!(error.contains("cannot read input directory"));
      assert!(error.contains(&path.display().to_string()));
    }
  }

  #[cfg(unix)]
  #[test]
  fn includes_symlinks_to_regular_files() {
    let dir = TestDir::new();
    fs::write(dir.0.join("target.png"), []).unwrap();
    std::os::unix::fs::symlink("target.png", dir.0.join("link.png")).unwrap();
    assert_eq!(discover_files(&dir.0, 0).unwrap(), ["link.png", "target.png"].map(PathBuf::from));
  }

  #[cfg(unix)]
  #[test]
  fn inaccessible_directory_has_context_when_permissions_apply() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TestDir::new();
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o000)).unwrap();
    let result = fs::read_dir(&dir.0);
    if result.is_err() {
      let error = discover_files(&dir.0, 0).unwrap_err().to_string();
      assert!(error.contains("cannot read input directory"));
      assert!(error.contains(&dir.0.display().to_string()));
    }
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
  }
}

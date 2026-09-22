use anyhow::Context;
use std::{
  collections::BTreeMap,
  fs::File,
  io::{BufReader, BufWriter, Write},
  path::Path,
};

/// Shared hash-file format: top-level filenames mapped to hash strings.
/// The JSON representation is a plain object compatible with files.
pub type HashFile = BTreeMap<String, String>;

/// Reads a hash file from JSON.
#[allow(dead_code)]
pub fn read(path: &Path) -> anyhow::Result<HashFile> {
  let file =
    File::open(path).with_context(|| format!("cannot open hash file {}", path.display()))?;
  serde_json::from_reader(BufReader::new(file))
    .with_context(|| format!("cannot parse hash file {}", path.display()))
}

/// Writes hashes as sorted, pretty JSON with a final newline.
pub fn write(path: &Path, hashes: &HashFile) -> anyhow::Result<()> {
  let file =
    File::create(path).with_context(|| format!("cannot create hash file {}", path.display()))?;
  let mut writer = BufWriter::new(file);
  serde_json::to_writer_pretty(&mut writer, hashes)
    .with_context(|| format!("cannot serialize hash file {}", path.display()))?;
  writer
    .write_all(b"\n")
    .with_context(|| format!("cannot write hash file {}", path.display()))?;
  writer
    .flush()
    .with_context(|| format!("cannot flush hash file {}", path.display()))
}

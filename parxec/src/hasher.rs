use crate::hash_file::HashFile;
use anyhow::Context;
use rayon::{ThreadPoolBuilder, prelude::*};
use sha2::{Digest, Sha256};
use std::{
  fs::File,
  io::Read,
  path::{Path, PathBuf},
};

/// Reads 64 KiB at a time to keep per-thread memory small without frequent tiny reads.
/// This is a practical default, not a benchmarked optimum; chunk size does not affect the digest.
const HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Computes SHA-256 over the original bytes of a file.
pub fn sha256(path: &Path) -> anyhow::Result<String> {
  let mut file =
    File::open(path).with_context(|| format!("cannot open input file {}", path.display()))?;
  let mut hasher = Sha256::new();
  let mut buffer = [0u8; HASH_BUFFER_SIZE];
  loop {
    let count = file
      .read(&mut buffer)
      .with_context(|| format!("cannot read input file {}", path.display()))?;
    if count == 0 {
      break;
    }
    hasher.update(&buffer[..count]);
  }
  Ok(
    hasher
      .finalize()
      .iter()
      .map(|byte| format!("{byte:02x}"))
      .collect(),
  )
}

/// Hashes named files in a dedicated Rayon pool and returns an in-memory map of
/// filenames to SHA-256 digests.
///
/// A `threads` value of 0 lets Rayon choose the pool size, normally from the
/// available logical CPUs unless `RAYON_NUM_THREADS` is set. A nonzero value
/// requests that many worker threads; Rayon does not cap it to the CPU count,
/// though its own maximum thread limit still applies.
pub fn hash_files(input_dir: &Path, names: &[PathBuf], threads: usize) -> anyhow::Result<HashFile> {
  // Start with Rayon's default thread pool settings.
  let mut builder = ThreadPoolBuilder::new();
  // Apply the requested thread count when one was provided.
  if threads > 0 {
    builder = builder.num_threads(threads);
  }
  // Build a pool dedicated to this hashing operation.
  let pool = builder
    .build()
    .context("cannot create hashing thread pool")?;
  // Hash each named input in parallel and pair its name with the digest.
  let pairs = pool.install(|| {
    names
      .par_iter()
      .map(|name| {
        let key = name
          .to_str()
          .ok_or_else(|| anyhow::anyhow!("input filename {} is not valid UTF-8", name.display()))?;
        Ok((key.to_owned(), sha256(&input_dir.join(name))?))
      })
      .collect::<anyhow::Result<Vec<_>>>()
  })?;
  // Collect the pairs into the in-memory hash map returned to the caller.
  Ok(pairs.into_iter().collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn hashes_original_bytes() {
    let path = std::env::temp_dir().join(format!("parxec-sha256-{}", std::process::id()));
    std::fs::write(&path, []).unwrap();
    assert_eq!(
      sha256(&path).unwrap(),
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    std::fs::write(&path, [0, 255, 1]).unwrap();
    assert_eq!(
      sha256(&path).unwrap(),
      "47ffa3ea45a70b8a41c2c0825df323c00a8b7a01c1ea06083cc41dddcc001123"
    );
    std::fs::remove_file(path).unwrap();
  }
}

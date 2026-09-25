use crate::grouping::Grouping;
use std::num::NonZeroUsize;

/// Dataset counts, percentages, and modeled processing times in seconds.
#[derive(Debug, PartialEq)]
pub struct Statistics {
  pub total_files: usize,
  pub distinct_hashes: usize,
  pub duplicate_groups: usize,
  pub duplicate_files: usize,
  pub redundant_files: usize,
  pub duplicate_percent: f64,
  pub redundant_percent: f64,
  pub estimated_all_seconds: f64,
  pub estimated_unique_seconds: f64,
  pub estimated_saved_seconds: f64,
}

/// Computes counts and throughput estimates without measuring elapsed work.
pub fn calculate(grouping: &Grouping, jobs: NonZeroUsize, file_ms: u64) -> Statistics {
  let distinct_hashes = grouping.groups.len();
  let total_files: usize = grouping.groups.values().map(Vec::len).sum();
  let duplicate_groups = grouping
    .groups
    .values()
    .filter(|names| names.len() > 1)
    .count();
  let duplicate_files: usize = grouping
    .groups
    .values()
    .filter(|names| names.len() > 1)
    .map(Vec::len)
    .sum();
  let redundant_files = grouping.redundant.len();
  let percent = |count| {
    if total_files == 0 {
      0.0
    } else {
      count as f64 * 100.0 / total_files as f64
    }
  };
  let seconds_per_file = file_ms as f64 / 1000.0 / jobs.get() as f64;
  Statistics {
    total_files,
    distinct_hashes,
    duplicate_groups,
    duplicate_files,
    redundant_files,
    duplicate_percent: percent(duplicate_files),
    redundant_percent: percent(redundant_files),
    estimated_all_seconds: total_files as f64 * seconds_per_file,
    estimated_unique_seconds: distinct_hashes as f64 * seconds_per_file,
    estimated_saved_seconds: redundant_files as f64 * seconds_per_file,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{grouping, hash_file::HashFile};

  #[test]
  fn calculates_counts_percentages_and_estimates() {
    let hashes = HashFile::from([
      ("a".into(), "one".into()),
      ("b".into(), "one".into()),
      ("c".into(), "one".into()),
      ("d".into(), "two".into()),
    ]);
    let stats = calculate(&grouping::group(&hashes), NonZeroUsize::new(2).unwrap(), 1000);
    assert_eq!(
      (
        stats.total_files,
        stats.distinct_hashes,
        stats.duplicate_groups,
        stats.duplicate_files,
        stats.redundant_files
      ),
      (4, 2, 1, 3, 2)
    );
    assert_eq!((stats.duplicate_percent, stats.redundant_percent), (75.0, 50.0));
    assert_eq!(
      (stats.estimated_all_seconds, stats.estimated_unique_seconds, stats.estimated_saved_seconds),
      (2.0, 1.0, 1.0)
    );
  }

  #[test]
  fn empty_and_zero_time_are_finite() {
    let jobs = NonZeroUsize::new(4).unwrap();
    let empty = calculate(&grouping::group(&HashFile::new()), jobs, 50);
    assert_eq!(
      (
        empty.total_files,
        empty.duplicate_percent,
        empty.redundant_percent,
        empty.estimated_saved_seconds
      ),
      (0, 0.0, 0.0, 0.0)
    );
    let unique = HashFile::from([("a".into(), "one".into())]);
    let stats = calculate(&grouping::group(&unique), jobs, 0);
    assert_eq!(
      (stats.duplicate_groups, stats.redundant_files, stats.estimated_all_seconds),
      (0, 0, 0.0)
    );
  }
}

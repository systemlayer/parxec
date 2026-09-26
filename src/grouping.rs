use crate::hash_file::HashFile;
use std::collections::BTreeMap;

/// Hash groups with sorted filenames and a mapping from redundant files to representatives.
#[derive(Debug, PartialEq, Eq)]
pub struct Grouping {
  /// Hashes mapped to their filenames in lexical order.
  /// Key: a hash. Value: filenames with that hash, in lexical order.
  pub groups: BTreeMap<String, Vec<String>>,
  /// Redundant filenames mapped to the first filename in their hash group.
  /// Key: a redundant filename. Value: its group's representative filename.
  pub redundant: BTreeMap<String, String>,
}

impl Grouping {
  /// Returns at most the first `limit` hash groups and redundant mappings.
  pub fn truncate(self, limit: usize) -> (Self, bool) {
    let truncated = self.groups.len() > limit || self.redundant.len() > limit;
    let groups = self.groups.into_iter().take(limit).collect();
    let redundant = self.redundant.into_iter().take(limit).collect();
    (Self { groups, redundant }, truncated)
  }
}

/// Groups files by hash and chooses the first filename in lexical order as representative.
///
/// `BTreeMap` keeps hash groups and redundant mappings in a predictable order.
/// A `HashMap` offers expected constant-time lookup, but its iteration order is
/// arbitrary and it is not consistently faster for sorted input; choosing the
/// smallest filename would still require an explicit step with either map.
pub fn group(hashes: &HashFile) -> Grouping {
  // Collect filenames under their hashes, independently of processing or output.
  let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
  for (name, hash) in hashes {
    groups.entry(hash.clone()).or_default().push(name.clone());
  }
  // Sort each group before selecting its stable representative.
  let mut redundant = BTreeMap::new();
  for names in groups.values_mut() {
    names.sort();
    // Map every remaining filename to the first filename in its group.
    if let Some((representative, copies)) = names.split_first() {
      for copy in copies {
        redundant.insert(copy.clone(), representative.clone());
      }
    }
  }
  // Return the grouping and mapping for callers to inspect or use separately.
  Grouping { groups, redundant }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn chooses_lexical_representatives_for_each_hash() {
    let hashes = HashFile::from([
      ("z.png".into(), "shared".into()),
      ("c.png".into(), "other".into()),
      ("a.png".into(), "shared".into()),
      ("b.png".into(), "shared".into()),
    ]);
    let result = group(&hashes);
    assert_eq!(result.groups["shared"], ["a.png", "b.png", "z.png"]);
    assert_eq!(result.redundant["b.png"], "a.png");
    assert_eq!(result.redundant["z.png"], "a.png");
    assert!(!result.redundant.contains_key("c.png"));
  }

  #[test]
  fn truncates_preview_entries_in_lexical_order() {
    let hashes = (0..12)
      .flat_map(|index| {
        [
          (format!("a-{index:02}.png"), format!("hash-{index:02}")),
          (format!("b-{index:02}.png"), format!("hash-{index:02}")),
        ]
      })
      .collect::<HashFile>();
    let (result, truncated) = group(&hashes).truncate(10);
    assert!(truncated);
    assert_eq!(result.groups.len(), 10);
    assert_eq!(result.redundant.len(), 10);
    assert!(result.groups.contains_key("hash-09"));
    assert!(!result.groups.contains_key("hash-10"));
    assert!(result.redundant.contains_key("b-09.png"));
    assert!(!result.redundant.contains_key("b-10.png"));
    let (_, truncated) = result.truncate(10);
    assert!(!truncated);
  }
}

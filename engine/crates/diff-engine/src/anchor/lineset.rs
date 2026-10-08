//! Sorted, disjoint, merged sets of 1-based line numbers (DIFF-005).

use std::ops::Range;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A set of 1-based lines stored as sorted, disjoint, non-adjacent `[start, end)` ranges.
/// Membership and nearest-line queries are `O(log n)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LineSet(Vec<Range<u32>>);

impl LineSet {
    /// The empty set.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Build from arbitrary (possibly unsorted, overlapping or empty) ranges.
    pub fn from_ranges(ranges: impl IntoIterator<Item = Range<u32>>) -> Self {
        let mut all: Vec<Range<u32>> = ranges.into_iter().filter(|r| r.start < r.end).collect();
        all.sort_by_key(|r| (r.start, r.end));
        let mut out: Vec<Range<u32>> = Vec::with_capacity(all.len());
        for r in all {
            if let Some(last) = out.last_mut() {
                if r.start <= last.end {
                    last.end = last.end.max(r.end);
                    continue;
                }
            }
            out.push(r);
        }
        Self(out)
    }

    /// Build from individual line numbers.
    pub fn from_lines(lines: impl IntoIterator<Item = u32>) -> Self {
        Self::from_ranges(lines.into_iter().map(|l| l..l.saturating_add(1)))
    }

    /// The merged ranges.
    pub fn ranges(&self) -> &[Range<u32>] {
        &self.0
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of lines in the set.
    pub fn len(&self) -> u64 {
        self.0.iter().map(|r| u64::from(r.end - r.start)).sum()
    }

    /// Whether `line` is in the set.
    pub fn contains(&self, line: u32) -> bool {
        let i = self.0.partition_point(|r| r.end <= line);
        self.0.get(i).is_some_and(|r| r.start <= line)
    }

    /// The member closest to `line` within `max_dist` lines (ties prefer the lower line).
    pub fn nearest_within(&self, line: u32, max_dist: u32) -> Option<u32> {
        if self.contains(line) {
            return Some(line);
        }
        let i = self.0.partition_point(|r| r.end <= line);
        let below = i
            .checked_sub(1)
            .and_then(|j| self.0.get(j))
            .map(|r| r.end - 1);
        let above = self.0.get(i).map(|r| r.start);
        let candidates = [below.map(|b| (line - b, b)), above.map(|a| (a - line, a))];
        candidates
            .into_iter()
            .flatten()
            .filter(|(d, _)| *d <= max_dist)
            .min()
            .map(|(_, l)| l)
    }

    /// Every member line, in order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().flat_map(|r| r.clone())
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn merges_and_queries() {
        let set = LineSet::from_ranges([5..7, 1..3, 3..4, 10..10]);
        assert_eq!(set.ranges(), &[1..4, 5..7]);
        assert!(set.contains(1));
        assert!(!set.contains(4));
        assert!(set.contains(6));
        assert_eq!(set.nearest_within(4, 1), Some(3));
        assert_eq!(set.nearest_within(9, 1), None);
        assert_eq!(set.nearest_within(9, 3), Some(6));
        assert_eq!(set.len(), 5);
    }
}

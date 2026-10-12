//! A small union-find with path halving and union by size (IMP-009).
//!
//! The representative of a set is the smallest index in it, so components come out in a
//! deterministic order whatever the union order was.

/// Disjoint sets over `0..n`.
#[derive(Debug, Clone)]
pub struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// The root of `x`'s set (path halving). Out-of-range indices are their own root.
    pub fn find(&mut self, mut x: usize) -> usize {
        if x >= self.parent.len() {
            return x;
        }
        while self.parent[x] != x {
            let grandparent = self.parent[self.parent[x]];
            self.parent[x] = grandparent;
            x = grandparent;
        }
        x
    }

    /// Merges the sets of `a` and `b`; returns whether they were separate.
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        if a >= self.parent.len() || b >= self.parent.len() {
            return false;
        }
        let (mut ra, mut rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        if self.size[ra] < self.size[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        self.size[ra] += self.size[rb];
        true
    }

    /// Every set as a sorted list of members; sets ordered by their smallest member.
    pub fn components(&mut self) -> Vec<Vec<usize>> {
        let n = self.parent.len();
        let mut by_root: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for x in 0..n {
            let root = self.find(x);
            by_root.entry(root).or_default().push(x);
        }
        let mut out: Vec<Vec<usize>> = by_root.into_values().collect();
        out.sort_by_key(|members| members.first().copied().unwrap_or(usize::MAX));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unions_and_components_are_deterministic() {
        let mut a = UnionFind::new(6);
        a.union(4, 5);
        a.union(0, 2);
        a.union(2, 4);
        let mut b = UnionFind::new(6);
        b.union(2, 4);
        b.union(5, 4);
        b.union(2, 0);
        assert_eq!(a.components(), b.components());
        assert_eq!(a.components(), vec![vec![0, 2, 4, 5], vec![1], vec![3]]);
        assert!(!a.union(0, 5));
        assert!(!a.union(0, 99));
        assert_eq!(a.len(), 6);
        assert!(!a.is_empty());
    }
}

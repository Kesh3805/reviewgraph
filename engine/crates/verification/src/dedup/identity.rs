//! Cross-run identity via symbol lineage (DED-003). Pure logic; lineage lookup is a port.

use std::collections::{BTreeMap, BTreeSet};

/// Port over `symbol_lineage` (GS-005): walks a symbol key back through `Renamed` / `Moved`
/// transitions to its earliest key. `None` means lineage data is missing for the pair.
pub trait LineageResolver {
    fn origin_symbol_key(&self, symbol_key: &str) -> Option<String>;
}

/// `identity_key = blake3(repository_id | pull_request_id | category | origin | signature)[..16]`.
pub fn identity_key(
    repository_id: &str,
    pull_request_id: &str,
    category: &str,
    origin_symbol_key: &str,
    claim_signature: &str,
) -> String {
    let pre = format!(
        "{repository_id}\0{pull_request_id}\0{category}\0{origin_symbol_key}\0{claim_signature}"
    );
    blake3::hash(pre.as_bytes()).as_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingIdentity {
    pub identity_key: String,
    /// True when lineage was missing and the candidate's own symbol key was used.
    pub lineage_fallback: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_identity(
    lineage: &dyn LineageResolver,
    repository_id: &str,
    pull_request_id: &str,
    category: &str,
    anchor_symbol_key: &str,
    claim_signature: &str,
) -> FindingIdentity {
    let (origin, fallback) = match lineage.origin_symbol_key(anchor_symbol_key) {
        Some(o) => (o, false),
        None => (anchor_symbol_key.to_string(), true),
    };
    FindingIdentity {
        identity_key: identity_key(
            repository_id,
            pull_request_id,
            category,
            &origin,
            claim_signature,
        ),
        lineage_fallback: fallback,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityClassification {
    pub new: BTreeSet<String>,
    pub persisting: BTreeSet<String>,
    pub fixed: BTreeSet<String>,
    pub not_reevaluated: BTreeSet<String>,
}

/// Re-review classification. `coverage_ok(identity)` answers whether the reviewer for that
/// finding's category succeeded for the symbol's cluster in the current run and the anchor
/// predicate no longer holds (VER-006). Absent findings without coverage are `not_reevaluated`,
/// never `fixed`. Identities that fell back on lineage are never classed fixed.
pub fn classify_rereview(
    current: &BTreeSet<String>,
    previous: &BTreeSet<String>,
    coverage_ok: impl Fn(&str) -> bool,
) -> IdentityClassification {
    let mut out = IdentityClassification::default();
    for k in current {
        if previous.contains(k) {
            out.persisting.insert(k.clone());
        } else {
            out.new.insert(k.clone());
        }
    }
    for k in previous.difference(current) {
        if coverage_ok(k) {
            out.fixed.insert(k.clone());
        } else {
            out.not_reevaluated.insert(k.clone());
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedRecord {
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictItem {
    pub identity_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostVerdict {
    /// Every publish-eligible finding, already-posted ones included (drives the verdict).
    pub verdict_set: Vec<VerdictItem>,
    /// Verdict set minus identities already posted with the same fingerprint.
    pub publish_set: Vec<VerdictItem>,
}

/// Dedup applied after the verdict is computed.
pub fn apply_post_verdict_dedup(
    eligible: Vec<VerdictItem>,
    already_posted: &BTreeMap<String, PostedRecord>,
) -> PostVerdict {
    let publish_set = eligible
        .iter()
        .filter(|i| {
            already_posted
                .get(&i.identity_key)
                .is_none_or(|p| p.fingerprint != i.fingerprint)
        })
        .cloned()
        .collect();
    PostVerdict {
        verdict_set: eligible,
        publish_set,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Map(BTreeMap<String, String>);
    impl LineageResolver for Map {
        fn origin_symbol_key(&self, k: &str) -> Option<String> {
            self.0.get(k).cloned()
        }
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rename_keeps_identity() {
        let l = Map([
            ("new".to_string(), "old".to_string()),
            ("old".to_string(), "old".to_string()),
        ]
        .into());
        let a = resolve_identity(&l, "r", "pr", "sec", "old", "sig");
        let b = resolve_identity(&l, "r", "pr", "sec", "new", "sig");
        assert_eq!(a.identity_key, b.identity_key);
    }

    #[test]
    fn missing_lineage_falls_back() {
        let l = Map(BTreeMap::new());
        assert!(resolve_identity(&l, "r", "pr", "sec", "x", "sig").lineage_fallback);
    }

    #[test]
    fn identity_isolated_between_pull_requests() {
        assert_ne!(
            identity_key("r", "pr1", "c", "s", "g"),
            identity_key("r", "pr2", "c", "s", "g")
        );
    }

    #[test]
    fn classification() {
        let c = classify_rereview(&set(&["a", "n"]), &set(&["a", "f", "d"]), |k| k == "f");
        assert_eq!(c.persisting, set(&["a"]));
        assert_eq!(c.new, set(&["n"]));
        assert_eq!(c.fixed, set(&["f"]));
        assert_eq!(c.not_reevaluated, set(&["d"]));
    }

    #[test]
    fn verdict_counts_posted_but_publish_excludes_it() {
        let items = vec![
            VerdictItem {
                identity_key: "a".into(),
                fingerprint: "f".into(),
            },
            VerdictItem {
                identity_key: "b".into(),
                fingerprint: "g".into(),
            },
        ];
        let posted = [(
            "a".to_string(),
            PostedRecord {
                fingerprint: "f".into(),
            },
        )]
        .into();
        let v = apply_post_verdict_dedup(items, &posted);
        assert_eq!(v.verdict_set.len(), 2);
        assert_eq!(v.publish_set.len(), 1);
        assert_eq!(v.publish_set[0].identity_key, "b");
    }
}

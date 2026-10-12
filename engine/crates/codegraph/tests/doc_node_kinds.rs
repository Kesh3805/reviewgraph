//! CG-001: `docs/graph-schema/node-kinds.md` is checked against `NodeKind`, so a documented
//! discriminant, spelling, category or refinement source cannot drift from the code.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod doc_tables;

use codegraph::{NodeCategory, NodeKind, RESERVED_PREFIXES};
use doc_tables::{doc, rows};

fn category_name(category: NodeCategory) -> String {
    serde_json::to_value(category)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn node_kinds_doc_matches_code() {
    let text = doc("node-kinds.md");
    let table = rows(&text, "Kinds");
    assert_eq!(table.len(), NodeKind::ALL.len(), "one row per node kind");
    for (row, kind) in table.iter().zip(NodeKind::ALL) {
        assert_eq!(row[0], kind.as_u8().to_string(), "{kind}: discriminant");
        assert_eq!(row[1], kind.as_str(), "{kind}: spelling");
        assert_eq!(row[2], category_name(kind.category()), "{kind}: category");
        let refined = kind
            .refinable_from()
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let refined = if refined.is_empty() {
            "-".to_owned()
        } else {
            refined
        };
        assert_eq!(row[3], refined, "{kind}: refined from");
        let synthetic = if kind.is_synthetic_only() {
            "yes"
        } else {
            "no"
        };
        assert_eq!(row[4], synthetic, "{kind}: synthetic");
    }
}

#[test]
fn node_kinds_doc_lists_every_reserved_prefix() {
    let text = doc("node-kinds.md");
    for prefix in RESERVED_PREFIXES {
        assert!(
            text.contains(&format!("`{prefix}`")),
            "reserved prefix {prefix} is not documented"
        );
    }
}

//! CG-002 and CG-003: `docs/graph-schema/edge-kinds.md` is checked against `EdgeKind`, the
//! reverse views, the endpoint rules and the confidence table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod doc_tables;

use codegraph::schema_rules::{convention, source_categories, target_categories};
use codegraph::{EdgeKind, NodeCategory, Provenance, ResolvedBy, ReverseView, TABLE};
use doc_tables::{doc, rows};

fn category_list(categories: &[NodeCategory]) -> String {
    categories
        .iter()
        .map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn edge_kinds_doc_matches_code() {
    let text = doc("edge-kinds.md");
    let table = rows(&text, "Stored kinds");
    assert_eq!(table.len(), EdgeKind::ALL.len(), "one row per stored kind");
    for (row, kind) in table.iter().zip(EdgeKind::ALL) {
        assert_eq!(row[0], kind.as_u8().to_string(), "{kind}: discriminant");
        assert_eq!(row[1], kind.as_str(), "{kind}: spelling");
        assert_eq!(row[2], convention(kind), "{kind}: direction");
        assert_eq!(
            row[3],
            category_list(&source_categories(kind)),
            "{kind}: source categories"
        );
        assert_eq!(
            row[4],
            category_list(&target_categories(kind)),
            "{kind}: target categories"
        );
    }
}

#[test]
fn reverse_views_doc_matches_code() {
    let text = doc("edge-kinds.md");
    let table = rows(&text, "Reverse views");
    assert_eq!(table.len(), ReverseView::ALL.len());
    for (row, view) in table.iter().zip(ReverseView::ALL) {
        assert_eq!(row[0], view.as_str());
        assert_eq!(row[1], view.underlying().as_str());
        assert_eq!(row[2], "in");
    }
    for view in ReverseView::ALL {
        assert!(
            EdgeKind::from_str_exact(view.as_str()).is_none(),
            "{} must not parse as a stored kind",
            view.as_str()
        );
    }
}

#[test]
fn confidence_doc_matches_table() {
    let text = doc("edge-kinds.md");
    let table = rows(&text, "Resolved by");
    assert_eq!(table.len(), TABLE.len());
    for (row, resolved) in table.iter().zip(ResolvedBy::ALL) {
        assert_eq!(row[0], (resolved as u8).to_string(), "{resolved:?}: disc");
        assert_eq!(row[1], resolved.as_str());
        let (_, confidence) = TABLE
            .iter()
            .find(|(r, _)| *r == resolved)
            .expect("every resolved_by has a table entry");
        assert_eq!(
            row[2],
            confidence.as_permille().to_string(),
            "{resolved:?}: confidence"
        );
    }
}

#[test]
fn provenance_doc_matches_code() {
    let text = doc("edge-kinds.md");
    let table = rows(&text, "Provenance");
    assert_eq!(table.len(), Provenance::ALL.len());
    for (row, provenance) in table.iter().zip(Provenance::ALL) {
        assert_eq!(row[0], provenance.as_u8().to_string());
        assert_eq!(row[1], provenance.as_str());
    }
}

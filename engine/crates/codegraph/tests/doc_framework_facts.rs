//! CG-006: `docs/graph-schema/framework-facts.md` is the attribute contract analyzers must emit;
//! its required-attribute table is checked against `FactCategory::REQUIRED`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod doc_tables;

use codegraph::FactCategory;
use doc_tables::{doc, rows};

#[test]
fn framework_facts_doc_matches_contract() {
    let text = doc("framework-facts.md");
    let table = rows(&text, "Required attributes");
    assert_eq!(table.len(), FactCategory::REQUIRED.len());
    for (row, (category, attrs)) in table.iter().zip(FactCategory::REQUIRED) {
        assert_eq!(row[0].trim_matches('`'), *category);
        assert_eq!(row[1].trim_matches('`'), *attrs);
    }
    let categories: Vec<&str> = FactCategory::ALL.iter().map(|c| c.as_str()).collect();
    let documented: Vec<&str> = FactCategory::REQUIRED.iter().map(|(c, _)| *c).collect();
    assert_eq!(categories, documented, "every category is documented");
}

#[test]
fn every_category_has_a_graph_output_row() {
    let text = doc("framework-facts.md");
    let table = rows(&text, "Graph output");
    for category in FactCategory::ALL {
        assert!(
            table
                .iter()
                .any(|row| row[0].trim_matches('`') == category.as_str()),
            "category {category} has no row under `Graph output`"
        );
    }
}

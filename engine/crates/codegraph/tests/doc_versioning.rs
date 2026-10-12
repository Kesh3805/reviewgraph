//! CG-011: the constants `docs/graph-schema/versioning.md` states are checked against the code.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod doc_tables;

use codegraph::{DecodeLimits, HEADER_LEN, LINKER_VERSION, MAGIC, SCHEMA_VERSION};
use doc_tables::{doc, rows};

#[test]
fn versioning_doc_states_the_current_constants() {
    let text = doc("versioning.md");
    assert!(
        text.contains(&format!("(currently `{SCHEMA_VERSION}`)")),
        "SCHEMA_VERSION"
    );
    assert!(
        text.contains(&format!("(currently `{LINKER_VERSION}`)")),
        "LINKER_VERSION"
    );
    assert!(text.contains(&format!("The header is {HEADER_LEN} bytes")));
    assert!(text.contains(&format!("`{}`", String::from_utf8_lossy(&MAGIC))));
}

#[test]
fn versioning_doc_header_layout_is_contiguous_and_ends_at_the_header_length() {
    let text = doc("versioning.md");
    let table = rows(&text, "Container format");
    let mut offset = 0usize;
    for row in &table {
        assert_eq!(row[0], offset.to_string(), "offset of {}", row[2]);
        offset += row[1].parse::<usize>().unwrap();
    }
    assert_eq!(offset, HEADER_LEN, "the documented fields fill the header");
}

#[test]
fn versioning_doc_names_the_default_decode_limit() {
    let limits = DecodeLimits::default();
    assert_eq!(limits.max_uncompressed_bytes, 8 * 1024 * 1024 * 1024);
    assert!(doc("versioning.md").contains("default 8 GiB"));
}

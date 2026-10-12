//! Reads the markdown tables of `docs/graph-schema`, so a test can compare them with the code
//! they describe.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// The text of `docs/graph-schema/{name}`.
pub fn doc(name: &str) -> String {
    let path = format!(
        "{}/../../../docs/graph-schema/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// The table rows under `## {heading}`, header and separator rows excluded. An escaped pipe
/// (`\|`) inside a cell is kept as a plain `|`.
pub fn rows(text: &str, heading: &str) -> Vec<Vec<String>> {
    let marker = format!("## {heading}");
    let mut in_section = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim_end() == marker;
            continue;
        }
        if !in_section || !line.starts_with('|') {
            continue;
        }
        let masked = line.replace("\\|", "\u{1}");
        let cells: Vec<String> = masked
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().replace('\u{1}', "|"))
            .collect();
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        out.push(cells);
    }
    // Drop the header row.
    out.into_iter().skip(1).collect()
}

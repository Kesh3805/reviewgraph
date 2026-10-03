//! Prints the tree-sitter S-expression of a file with node kinds and fields.
//!
//!   cargo run -q -p lang-typescript --example dump -- <path> [tsx]

use tree_sitter::{Node, Parser};

fn walk(node: Node<'_>, src: &[u8], depth: usize, field: Option<&str>) {
    let text = if node.child_count() == 0 {
        format!(" {:?}", node.utf8_text(src).unwrap_or("?"))
    } else {
        String::new()
    };
    println!(
        "{}{}{}{} [{}:{}-{}:{}]{}",
        "  ".repeat(depth),
        field.map(|f| format!("{f}: ")).unwrap_or_default(),
        node.kind(),
        if node.is_error() { " ERROR" } else { "" },
        node.start_position().row,
        node.start_position().column,
        node.end_position().row,
        node.end_position().column,
        text
    );
    let mut cursor = node.walk();
    for (i, child) in node.children(&mut cursor).enumerate() {
        walk(child, src, depth + 1, node.field_name_for_child(i as u32));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: dump <path> [tsx]")?;
    let tsx = args.next().as_deref() == Some("tsx");
    let src = std::fs::read(&path)?;
    let mut parser = Parser::new();
    let lang = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    parser.set_language(&lang.into())?;
    let tree = parser.parse(&src, None).ok_or("parse failed")?;
    walk(tree.root_node(), &src, 0, None);
    Ok(())
}

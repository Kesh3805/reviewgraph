//! Imports, exports and the module level binding table.

use std::collections::BTreeMap;

use analysis_ir::{
    BindingRef, DiagCode, DiagSeverity, ImportBinding, ImportKind, Imported, IrExport, IrImport,
    IrSymbol, LocalId, Modifiers,
};
use tree_sitter::Node;

use crate::diagnostics::DiagnosticSink;
use crate::exprs::{children_of, collapse_ws};
use crate::symbols::MAX_VISIT_DEPTH;
use crate::text::Source;

/// What a local name was imported as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The module specifier as written (`.models`, `app.db`).
    pub specifier: String,
    /// The imported name, or `*` when the whole module is bound.
    pub imported: String,
    pub reference: BindingRef,
}

/// Top-level import bindings of one file, by local name.
#[derive(Debug, Default, Clone)]
pub struct BindingTable {
    by_local: BTreeMap<String, Binding>,
}

impl BindingTable {
    pub fn get(&self, local: &str) -> Option<&Binding> {
        self.by_local.get(local)
    }

    /// Whether any binding comes from `package` or one of its submodules.
    pub fn imports_from(&self, package: &str) -> bool {
        self.by_local
            .values()
            .any(|b| b.specifier == package || b.specifier.starts_with(&format!("{package}.")))
    }

    pub fn len(&self) -> usize {
        self.by_local.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_local.is_empty()
    }
}

/// Imports and exports of one file.
#[derive(Debug, Default)]
pub struct ModuleFacts {
    pub imports: Vec<IrImport>,
    pub exports: Vec<IrExport>,
    pub bindings: BindingTable,
}

/// Import statements reachable without entering a function or class, with a flag for the ones
/// guarded by `if TYPE_CHECKING:`.
fn import_statements<'t>(
    node: Node<'t>,
    src: &Source<'_>,
    type_only: bool,
    depth: usize,
    out: &mut Vec<(Node<'t>, bool)>,
) {
    if depth > MAX_VISIT_DEPTH {
        return;
    }
    for stmt in children_of(node) {
        match stmt.kind() {
            "import_statement" | "import_from_statement" => out.push((stmt, type_only)),
            "if_statement" => {
                let guarded = stmt
                    .child_by_field_name("condition")
                    .is_some_and(|c| src.text(c).ends_with("TYPE_CHECKING"));
                let mut cursor = stmt.walk();
                let parts: Vec<Node<'t>> = stmt.children(&mut cursor).collect();
                for part in parts {
                    match part.kind() {
                        "block" => {
                            import_statements(part, src, type_only || guarded, depth + 1, out)
                        }
                        "elif_clause" | "else_clause" => {
                            let mut inner_cursor = part.walk();
                            let blocks: Vec<Node<'t>> = part.children(&mut inner_cursor).collect();
                            for block in blocks {
                                if block.kind() == "block" {
                                    import_statements(block, src, type_only, depth + 1, out);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "try_statement" | "with_statement" => {
                let mut cursor = stmt.walk();
                let parts: Vec<Node<'t>> = stmt.children(&mut cursor).collect();
                for part in parts {
                    match part.kind() {
                        "block" => import_statements(part, src, type_only, depth + 1, out),
                        "except_clause"
                        | "except_group_clause"
                        | "else_clause"
                        | "finally_clause" => {
                            let mut inner_cursor = part.walk();
                            let blocks: Vec<Node<'t>> = part.children(&mut inner_cursor).collect();
                            for block in blocks {
                                if block.kind() == "block" {
                                    import_statements(block, src, type_only, depth + 1, out);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn name_of(node: Node<'_>, src: &Source<'_>) -> String {
    collapse_ws(&src.text(node))
}

/// Collects imports, exports and bindings. `is_init` is true for `__init__.py`, whose imports are
/// also re-exports.
pub fn collect(
    root: Node<'_>,
    src: &Source<'_>,
    symbols: &[IrSymbol],
    is_init: bool,
    sink: &mut DiagnosticSink,
) -> ModuleFacts {
    let mut facts = ModuleFacts::default();
    let mut statements = Vec::new();
    import_statements(root, src, false, 0, &mut statements);
    for (stmt, type_only) in statements {
        let index = u32::try_from(facts.imports.len()).unwrap_or(u32::MAX);
        let range = src.range(stmt);
        if stmt.kind() == "import_statement" {
            for item in children_of(stmt) {
                let (specifier, local) = match item.kind() {
                    "dotted_name" => {
                        let spec = name_of(item, src);
                        let local = spec.split('.').next().unwrap_or("").to_owned();
                        (spec, local)
                    }
                    "aliased_import" => {
                        let Some(name) = item.child_by_field_name("name") else {
                            continue;
                        };
                        let alias = item
                            .child_by_field_name("alias")
                            .map(|a| name_of(a, src))
                            .unwrap_or_default();
                        (name_of(name, src), alias)
                    }
                    _ => continue,
                };
                let import_index = u32::try_from(facts.imports.len()).unwrap_or(u32::MAX);
                facts.bindings.by_local.insert(
                    local.clone(),
                    Binding {
                        specifier: specifier.clone(),
                        imported: "*".to_owned(),
                        reference: BindingRef {
                            import: import_index,
                            binding: 0,
                        },
                    },
                );
                facts.imports.push(IrImport {
                    specifier,
                    kind: ImportKind::Esm,
                    type_only,
                    bindings: vec![ImportBinding {
                        local,
                        imported: Imported::Namespace,
                        type_only,
                    }],
                    range,
                });
            }
            continue;
        }
        let Some(module) = stmt.child_by_field_name("module_name") else {
            continue;
        };
        let specifier = name_of(module, src);
        let mut bindings = Vec::new();
        let mut star = false;
        for item in children_of(stmt) {
            if item.id() == module.id() {
                continue;
            }
            match item.kind() {
                "dotted_name" => {
                    let name = name_of(item, src);
                    bindings.push(ImportBinding {
                        local: name.clone(),
                        imported: Imported::Named(name),
                        type_only,
                    });
                }
                "aliased_import" => {
                    let Some(name) = item.child_by_field_name("name") else {
                        continue;
                    };
                    let name = name_of(name, src);
                    let local = item
                        .child_by_field_name("alias")
                        .map_or_else(|| name.clone(), |a| name_of(a, src));
                    bindings.push(ImportBinding {
                        local,
                        imported: Imported::Named(name),
                        type_only,
                    });
                }
                "wildcard_import" => star = true,
                _ => {}
            }
        }
        if star {
            sink.note(
                DiagSeverity::Info,
                DiagCode::UnsupportedConstruct,
                "star import hides the names it binds",
                Some(range),
            );
        }
        for (position, binding) in bindings.iter().enumerate() {
            let imported = match &binding.imported {
                Imported::Named(n) => n.clone(),
                _ => "*".to_owned(),
            };
            facts.bindings.by_local.insert(
                binding.local.clone(),
                Binding {
                    specifier: specifier.clone(),
                    imported,
                    reference: BindingRef {
                        import: index,
                        binding: u32::try_from(position).unwrap_or(u32::MAX),
                    },
                },
            );
            if is_init {
                if let Imported::Named(name) = &binding.imported {
                    facts.exports.push(IrExport::Reexport {
                        specifier: specifier.clone(),
                        imported: Imported::Named(name.clone()),
                        exported_as: binding.local.clone(),
                        type_only,
                    });
                }
            }
        }
        if star && is_init {
            facts.exports.push(IrExport::StarReexport {
                specifier: specifier.clone(),
                as_namespace: None,
                type_only,
            });
        }
        facts.imports.push(IrImport {
            specifier,
            kind: ImportKind::Esm,
            type_only,
            bindings,
            range,
        });
    }
    for symbol in symbols.iter().skip(1) {
        if symbol.parent == Some(LocalId(0)) && symbol.modifiers.contains(Modifiers::EXPORTED) {
            facts.exports.push(IrExport::Local {
                symbol: symbol.local_id,
                exported_as: symbol.name.clone(),
                type_only: false,
            });
        }
    }
    facts
}

//! Entities and database access → `DatabaseTable`, `READS_TABLE`, `WRITES_TABLE` (CG-006).
//!
//! An `entity` fact refines its class to [`NodeKind::DatabaseEntity`] and creates the table node
//! its id (`db:{schema}.{table}`) points at. A `db_access` fact names the caller, the entity it
//! touches and `read`/`write`, and lands on the table the entity maps to — so "this endpoint
//! writes `users`" is one hop from the endpoint, which is what the data-loss reviewers look for.

use std::collections::BTreeMap;

use analysis_ir::framework::IrFrameworkFact;
use analysis_ir::symbol::AttrValue;

use crate::edge::EdgeFlags;
use crate::edge_kind::EdgeKind;
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

use super::contract::{self, FactCategory, FactIssueCode};
use super::Mapping;

/// Indexes one `entity` fact into `entity_tables`, before any `db_access` fact is mapped.
///
/// Separate from [`apply_entity`] because the mapping needs the whole file's entity set to exist
/// first, whatever order the adapter emitted the facts in.
pub(crate) fn index_entity(
    fact: &IrFrameworkFact,
    file: &crate::linker::symbol_table::FileSymbols,
    index: &mut BTreeMap<String, NodeKey>,
    out: &mut super::FrameworkOutput,
) {
    let category = FactCategory::Entity;
    let mut issues = Vec::new();
    let name = contract::require_str(fact, category, &file.path, "symbol", &mut issues);
    let table = contract::require_str(fact, category, &file.path, "table", &mut issues);
    let schema = contract::optional_str(fact, category, &file.path, "schema", &mut issues);
    for issue in issues {
        out.issues.push(issue);
    }
    let (Some(name), Some(table)) = (name, table) else {
        return;
    };
    let Some(symbol) = file
        .symbols
        .iter()
        .skip(1)
        .find(|symbol| symbol.name == name || symbol.qualified_name == name)
    else {
        out.issues.push(super::FactIssue {
            code: FactIssueCode::UnknownSymbol,
            category,
            file: file.path.clone(),
            range: fact.range,
            detail: format!("entity fact names {name:?}, which this file does not declare"),
        });
        return;
    };
    let Ok(id) = NodeId::table(schema.as_deref(), &table) else {
        return;
    };
    let display = id
        .as_str()
        .strip_prefix("db:")
        .unwrap_or(id.as_str())
        .to_owned();
    if !out.nodes.iter().any(|node| node.id.key() == id.key()) {
        out.nodes.push(
            crate::graph::NodeInput::new(id.clone(), NodeKind::DatabaseTable, display.clone())
                .qualified_name(display),
        );
    }
    index.insert(table.clone(), id.key());
    // The class is an entity whether or not a `db_access` fact exists for it.
    if NodeKind::DatabaseEntity.refines_from(symbol.kind) {
        out.refinements.push((symbol.key, NodeKind::DatabaseEntity));
    }
}

/// `entity` → refinement, table node and the `REFERENCES`+`MAPS_TABLE` edge to it.
pub(crate) fn apply_entity(ctx: &mut Mapping<'_>) {
    let Some(entity) = ctx.require_symbol("symbol") else {
        return;
    };
    let mut issues = Vec::new();
    let table = contract::require_str(ctx.fact, ctx.category, ctx.path, "table", &mut issues);
    let schema = contract::optional_str(ctx.fact, ctx.category, ctx.path, "schema", &mut issues);
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let Some(table) = table else {
        return;
    };
    let Ok(id) = NodeId::table(schema.as_deref(), &table) else {
        ctx.issue(
            FactIssueCode::InvalidAttribute,
            format!("table attribute {table:?} does not produce a table id"),
        );
        return;
    };
    let display = id
        .as_str()
        .strip_prefix("db:")
        .unwrap_or(id.as_str())
        .to_owned();
    let table_key = ctx.node(id, NodeKind::DatabaseTable, display.clone(), display);
    ctx.edge(
        EdgeKind::References,
        entity,
        table_key,
        EdgeFlags::MAPS_TABLE,
    );
    if NodeKind::DatabaseEntity.refines_from(ctx.kind_of(&entity).unwrap_or(NodeKind::Class)) {
        ctx.refine(entity, NodeKind::DatabaseEntity);
    }
}

/// `db_access` → `READS_TABLE` or `WRITES_TABLE` from the caller onto the entity's table.
pub(crate) fn apply_access(ctx: &mut Mapping<'_>) {
    let Some(caller) = ctx.require_symbol("symbol") else {
        return;
    };
    let mut issues = Vec::new();
    let entity = contract::require_str(ctx.fact, ctx.category, ctx.path, "entity", &mut issues);
    let op = contract::require_str(ctx.fact, ctx.category, ctx.path, "op", &mut issues);
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let (Some(entity), Some(op)) = (entity, op) else {
        return;
    };
    let kind = match op.as_str() {
        "read" => EdgeKind::ReadsTable,
        "write" => EdgeKind::WritesTable,
        other => {
            ctx.issue(
                FactIssueCode::InvalidAttribute,
                format!("op attribute {other:?} is not read or write"),
            );
            return;
        }
    };
    let Some(table) = ctx.entity_tables.get(&entity).copied() else {
        ctx.issue(
            FactIssueCode::UnresolvedEntity,
            format!("no entity fact in this file maps {entity:?} to a table"),
        );
        return;
    };
    ctx.edge(kind, caller, table, EdgeFlags::EMPTY);
}

/// True when the attribute value marks a write, used by adapters that pre-classify the op.
#[must_use]
pub fn is_write(value: Option<&AttrValue>) -> bool {
    matches!(value, Some(AttrValue::Str(op)) if op == "write")
}

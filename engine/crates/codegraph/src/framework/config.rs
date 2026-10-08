//! Configuration reads → `EnvironmentVariable`, `READS_CONFIG` (CG-006).
//!
//! Only the variable *name* is ever recorded. The analyzer does not evaluate the configuration,
//! does not read the `.env` file and cannot see a value, so a secret in a variable is not in the
//! graph, in a snapshot or in a CI log.

use crate::edge::EdgeFlags;
use crate::edge_kind::EdgeKind;
use crate::node_id::NodeId;
use crate::node_kind::NodeKind;

use super::contract::{self, FactCategory, FactIssueCode};
use super::Mapping;

/// `env_read` → an `env:{NAME}` node and `READS_CONFIG` from the reading symbol to it.
pub(crate) fn apply_env_read(ctx: &mut Mapping<'_>) {
    let Some(symbol) = ctx.require_symbol("symbol") else {
        return;
    };
    let mut issues = Vec::new();
    let name = contract::require_str(
        ctx.fact,
        FactCategory::EnvRead,
        ctx.path,
        "name",
        &mut issues,
    );
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let Some(name) = name else {
        return;
    };
    let id = match NodeId::env(&name) {
        Ok(id) => id,
        Err(error) => {
            ctx.issue(
                FactIssueCode::InvalidAttribute,
                format!("env_read name {name:?} is not a variable name: {error}"),
            );
            return;
        }
    };
    let env = ctx.node(id, NodeKind::EnvironmentVariable, name.clone(), name);
    ctx.edge(EdgeKind::ReadsConfig, symbol, env, EdgeFlags::EMPTY);
}

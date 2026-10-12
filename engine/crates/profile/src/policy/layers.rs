//! The layer map: declared layers (`architecture.layers`) first, inferred roles (PROF-002)
//! second. Shared by the forbidden-dependency rule (POL-003) and the boundary check (REV-A-002).

use std::collections::BTreeSet;

use codegraph::{GraphQuery, NodeRef};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::config::ReviewConfigV1;
use crate::infer::roles::{Role, RoleIndex};

#[derive(Debug, Clone)]
pub struct LayerMap {
    declared: Vec<(String, GlobSet)>,
    roles: RoleIndex,
}

impl LayerMap {
    pub fn new(config: &ReviewConfigV1, roles: RoleIndex) -> Self {
        let declared = config
            .architecture
            .layers
            .iter()
            .map(|(name, globs)| {
                let mut builder = GlobSetBuilder::new();
                for glob in globs {
                    if let Ok(glob) = Glob::new(glob) {
                        builder.add(glob);
                    }
                }
                (
                    name.clone(),
                    builder.build().unwrap_or_else(|_| GlobSet::empty()),
                )
            })
            .collect();
        Self { declared, roles }
    }

    /// Builds the role index from `graph` (no framework module files).
    pub fn from_graph(config: &ReviewConfigV1, graph: &dyn GraphQuery) -> Self {
        Self::new(config, RoleIndex::build(graph, &[]))
    }

    /// No declared layer and no inferred role: boundary checks cannot run.
    pub fn is_empty(&self) -> bool {
        self.declared.is_empty() && self.roles.is_empty()
    }

    pub fn roles(&self) -> &RoleIndex {
        &self.roles
    }

    /// Whether `name` can be evaluated: declared, or a role some file has.
    pub fn defines(&self, name: &str) -> bool {
        if self.declared.iter().any(|(n, _)| n == name) {
            return true;
        }
        Role::from_layer_name(name).is_some_and(|role| self.roles.files().any(|(_, a)| a.role == role))
    }

    /// Whether `node` belongs to layer `name`.
    pub fn in_layer(&self, name: &str, node: &NodeRef<'_>) -> bool {
        if let Some((_, globs)) = self.declared.iter().find(|(n, _)| n == name) {
            return node.file.is_some_and(|f| globs.is_match(f));
        }
        Role::from_layer_name(name).is_some_and(|role| self.roles.node_role(node) == Some(role))
    }

    /// Every layer `node` belongs to: matching declared layers, then its role.
    pub fn layers_of(&self, node: &NodeRef<'_>) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self
            .declared
            .iter()
            .filter(|(_, globs)| node.file.is_some_and(|f| globs.is_match(f)))
            .map(|(n, _)| n.clone())
            .collect();
        if let Some(role) = self.roles.node_role(node) {
            out.insert(role.as_str().to_owned());
        }
        out
    }
}

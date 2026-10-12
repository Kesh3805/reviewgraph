//! The observed layer dependency matrix as conventions (PROF-002):
//! `layer_dependency:{from}->{to}` — "files of layer `from` do not depend on layer `to`".
//!
//! `samples` is the number of `from` files and `violations` the number of `from` files with at
//! least one `IMPORTS`/`CALLS`/`DEPENDS_ON` edge into `to`. These feed the PROF-003 confidence
//! like any other convention; the boundary checker (REV-A-002) and the precedence resolver
//! (POL-004) consume them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use codegraph::{Direction, EdgeFilter};

use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::infer::architecture::dependency_kinds;
use crate::infer::roles::Role;

pub const VERSION: u32 = 1;

/// Roles whose mutual dependencies form an architecture (tests, configs and utilities do not).
pub const ARCHITECTURAL_ROLES: [Role; 7] = [
    Role::Controller,
    Role::Service,
    Role::Repository,
    Role::Entity,
    Role::Guard,
    Role::Processor,
    Role::Module,
];

pub fn convention_id(from: Role, to: Role) -> String {
    format!("layer_dependency:{}->{}", from.as_str(), to.as_str())
}

#[derive(Debug, Clone)]
pub struct LayerDependencyMiner {
    pub from: Role,
    pub to: Role,
    id: String,
}

impl LayerDependencyMiner {
    pub fn new(from: Role, to: Role) -> Self {
        Self {
            from,
            to,
            id: convention_id(from, to),
        }
    }

    /// One miner per ordered pair of distinct architectural roles present in the repository.
    pub fn for_present_roles(cx: &MiningContext<'_>) -> Vec<Self> {
        let present: BTreeSet<Role> = cx
            .roles
            .files()
            .map(|(_, a)| a.role)
            .filter(|r| ARCHITECTURAL_ROLES.contains(r))
            .collect();
        let mut out = Vec::new();
        for from in &present {
            for to in &present {
                if from != to {
                    out.push(Self::new(*from, *to));
                }
            }
        }
        out
    }
}

impl ConventionMiner for LayerDependencyMiner {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        format!(
            "{} code does not depend on {} code",
            self.from.as_str(),
            self.to.as_str()
        )
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        cx.roles
            .files()
            .filter(|(_, a)| a.role == self.from)
            .map(|(path, _)| Sample {
                path: path.to_owned(),
                line: 1,
                symbol: None,
                key: None,
                detail: String::new(),
            })
            .collect()
    }

    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance {
        let graph = cx.inputs.graph;
        let filter = EdgeFilter::kinds(dependency_kinds());
        let mut hit: Option<(String, u32)> = None;
        graph.nodes_in_file(&sample.path, &mut |node| {
            if hit.is_some() {
                return;
            }
            graph.for_each_edge(node.key, Direction::Out, &filter, &mut |edge| {
                if let Some(target) = graph.node(edge.target) {
                    if cx.roles.node_role(&target) == Some(self.to) {
                        let line = edge.location.map_or(1, |(_, line, _)| line);
                        hit = Some((target.qualified_name.to_owned(), line));
                        return ControlFlow::Break(());
                    }
                }
                ControlFlow::Continue(())
            });
        });
        match hit {
            Some((target, line)) => Conformance::Violates {
                evidence: format!("line {line} depends on {} `{target}`", self.to.as_str()),
            },
            None => Conformance::Conforms,
        }
    }
}

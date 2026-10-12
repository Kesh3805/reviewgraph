//! `controllers_do_not_access_repositories` (PROF-004): a controller class violates when any of
//! its members depends on, calls or is injected with a repository (a `repository`-role symbol or
//! a generic `Repository<T>` type).

use codegraph::{Direction, EdgeKind, NodeKind};

use super::{edges, members, node_sample};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::infer::roles::Role;

pub const ID: &str = "controllers_do_not_access_repositories";
pub const VERSION: u32 = 1;

const DEPENDENCY_EDGES: [EdgeKind; 5] = [
    EdgeKind::DependsOn,
    EdgeKind::Calls,
    EdgeKind::UsesType,
    EdgeKind::AcceptsType,
    EdgeKind::Imports,
];

#[derive(Debug, Clone, Copy, Default)]
pub struct ControllerRepositoryMiner;

impl ConventionMiner for ControllerRepositoryMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Controllers reach data through services, never through repositories".to_owned()
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let mut out = Vec::new();
        cx.inputs.graph.for_each_node(&mut |node| {
            let is_controller = node.kind == NodeKind::Controller
                || (node.kind == NodeKind::Class && cx.roles.node_role(&node) == Some(Role::Controller));
            if is_controller {
                out.extend(node_sample(&node, ""));
            }
        });
        out
    }

    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance {
        let graph = cx.inputs.graph;
        let Some(class) = sample.key.and_then(|k| graph.node(k)) else {
            return Conformance::NotApplicable;
        };
        for member in members(graph, &class) {
            for edge in edges(graph, member, Direction::Out, &DEPENDENCY_EDGES) {
                let Some(target) = graph.node(edge.target) else {
                    continue;
                };
                let is_repository = cx.roles.node_role(&target) == Some(Role::Repository)
                    || target.name == "Repository";
                if is_repository {
                    return Conformance::Violates {
                        evidence: format!(
                            "{} {} `{}`",
                            class.name,
                            edge.kind.as_str(),
                            target.qualified_name
                        ),
                    };
                }
            }
        }
        Conformance::Conforms
    }
}

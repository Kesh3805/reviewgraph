//! Layer, matrix and module-boundary inference over the graph (PROF-002).
//!
//! Pure over a [`GraphQuery`]: every collection is ordered (`BTreeMap`/sorted `Vec`), so the
//! result is a function of the snapshot alone.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;

use codegraph::query::GraphQuery;
use codegraph::{Direction, EdgeFilter, EdgeKind, EdgeKindSet, NodeFlags, NodeKey, NodeKind};

use super::module_of;
use super::roles::{classify, directory_match, suffix_match, Role, RoleAssignment, RoleBasis};
use crate::model::{ArchitectureModel, LayerDef, LayerEdge, ModuleBoundary};

/// Note recorded when no role could be recognized.
pub const INSUFFICIENT_SIGNAL: &str = "architecture_inference: insufficient_signal";
/// Upper bound on the globs listed per layer.
pub const MAX_LAYER_GLOBS: usize = 50;

/// Edge kinds that count as a dependency between layers.
fn dependency_kinds() -> EdgeKindSet {
    EdgeKindSet::from_kinds([EdgeKind::Imports, EdgeKind::Calls, EdgeKind::DependsOn])
}

/// The layers, matrix and modules of one snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureInference {
    pub architecture: ArchitectureModel,
    pub modules: Vec<ModuleBoundary>,
}

struct Info<'a> {
    kind: NodeKind,
    flags: NodeFlags,
    file: Option<&'a str>,
    name: &'a str,
}

impl Info<'_> {
    fn role(&self) -> Option<Role> {
        classify(self.kind, self.flags, self.file).map(|a| a.role)
    }
}

/// A module's root directory and name before dependents and exports are filled in.
struct ModRoot {
    name: String,
    root: String,
    /// The `Module` node of the framework, when there is one.
    node: Option<NodeKey>,
}

fn better(candidate: &RoleAssignment, current: &RoleAssignment) -> bool {
    candidate.confidence > current.confidence
        || (candidate.confidence == current.confidence && candidate.role < current.role)
}

pub(crate) fn glob_for(path: &str, assignment: &RoleAssignment) -> String {
    match assignment.basis {
        RoleBasis::Framework => path.to_owned(),
        RoleBasis::Suffix => match (suffix_match(path), path.rsplit('.').next()) {
            (Some((tag, _)), Some(ext)) => format!("**/*.{tag}.{ext}"),
            _ => path.to_owned(),
        },
        RoleBasis::Directory => match directory_match(path) {
            Some((segment, _)) => format!("**/{segment}/**"),
            None => path.to_owned(),
        },
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Infers the architecture of `g`.
pub fn infer_architecture(g: &dyn GraphQuery) -> ArchitectureInference {
    let mut infos: HashMap<NodeKey, Info<'_>> = HashMap::new();
    g.for_each_node(&mut |n| {
        infos.insert(
            n.key,
            Info {
                kind: n.kind,
                flags: n.attrs.flags,
                file: n.file,
                name: n.name,
            },
        );
    });

    // Per-file best role, and the file nodes (for barrel exports).
    let mut file_roles: BTreeMap<&str, RoleAssignment> = BTreeMap::new();
    let mut file_nodes: BTreeMap<&str, NodeKey> = BTreeMap::new();
    let mut module_nodes: BTreeMap<(&str, &str), NodeKey> = BTreeMap::new();
    for (key, info) in &infos {
        let Some(path) = info.file else { continue };
        if info.kind == NodeKind::File {
            file_nodes.insert(path, *key);
        }
        if info.kind == NodeKind::Module {
            module_nodes.insert((path, info.name), *key);
        }
        if let Some(a) = classify(info.kind, info.flags, Some(path)) {
            match file_roles.get(path) {
                Some(cur) if !better(&a, cur) => {}
                _ => {
                    file_roles.insert(path, a);
                }
            }
        }
    }

    // Layers.
    let mut by_role: BTreeMap<Role, Vec<(&str, RoleAssignment)>> = BTreeMap::new();
    for (path, a) in &file_roles {
        by_role.entry(a.role).or_default().push((*path, *a));
    }
    let layers: Vec<LayerDef> = by_role
        .iter()
        .map(|(role, files)| {
            let globs: BTreeSet<String> = files.iter().map(|(p, a)| glob_for(p, a)).collect();
            let sum: f32 = files.iter().map(|(_, a)| a.confidence).sum();
            LayerDef {
                name: role.as_str().to_owned(),
                globs: globs.into_iter().take(MAX_LAYER_GLOBS).collect(),
                member_count: files.len() as u32,
                role_confidence: sum / files.len() as f32,
            }
        })
        .collect();

    // Module roots: framework module nodes, else directory modules of the classified files.
    let mut roots: Vec<ModRoot> = module_nodes
        .iter()
        .map(|((path, name), key)| ModRoot {
            name: (*name).to_owned(),
            root: dir_of(path).to_owned(),
            node: Some(*key),
        })
        .collect();
    if roots.is_empty() {
        let dirs: BTreeSet<String> = file_roles
            .keys()
            .map(|p| module_of(p))
            .filter(|m| !m.is_empty())
            .collect();
        roots = dirs
            .into_iter()
            .map(|root| ModRoot {
                name: root.rsplit('/').next().unwrap_or(&root).to_owned(),
                root,
                node: None,
            })
            .collect();
    }
    roots.sort_by(|a, b| (&a.root, &a.name).cmp(&(&b.root, &b.name)));
    // Longest root first, for prefix matching.
    let mut by_length: Vec<usize> = (0..roots.len()).collect();
    by_length.sort_by(|a, b| {
        roots[*b]
            .root
            .len()
            .cmp(&roots[*a].root.len())
            .then(a.cmp(b))
    });
    let module_index = |path: &str| -> Option<usize> {
        by_length.iter().copied().find(|i| {
            let root = &roots[*i].root;
            path.strip_prefix(root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
    };

    // Matrix and dependents in one pass over the dependency edges.
    let filter = EdgeFilter::kinds(dependency_kinds());
    let mut counts: BTreeMap<(Role, Role), u32> = BTreeMap::new();
    let mut dependents: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); roots.len()];
    for (key, info) in &infos {
        let Some(src_path) = info.file else { continue };
        let src_role = info.role();
        let src_module = module_index(src_path);
        g.for_each_edge(*key, Direction::Out, &filter, &mut |edge| {
            if let Some(target) = infos.get(&edge.target) {
                if let (Some(a), Some(b)) = (src_role, target.role()) {
                    if a != b {
                        *counts.entry((a, b)).or_insert(0) += 1;
                    }
                }
                if let (Some(from), Some(to_path)) = (src_module, target.file) {
                    if let Some(to) = module_index(to_path) {
                        if to != from {
                            dependents[to].insert(from);
                        }
                    }
                }
            }
            ControlFlow::Continue(())
        });
    }

    // Module roles and exports.
    let modules: Vec<ModuleBoundary> = roots
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut role_counts: BTreeMap<Role, u32> = BTreeMap::new();
            for (path, a) in &file_roles {
                if a.role != Role::Test && module_index(path) == Some(i) {
                    *role_counts.entry(a.role).or_insert(0) += 1;
                }
            }
            let layer = role_counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(role, _)| role.as_str().to_owned());

            let mut sources: Vec<NodeKey> = m.node.into_iter().collect();
            let barrel = format!("{}/index.ts", m.root);
            if let Some(key) = file_nodes.get(barrel.as_str()) {
                sources.push(*key);
            }
            let mut exports: BTreeSet<String> = BTreeSet::new();
            let exports_only = EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Exports));
            for source in sources {
                g.for_each_edge(source, Direction::Out, &exports_only, &mut |edge| {
                    if let Some(target) = infos.get(&edge.target) {
                        exports.insert(target.name.to_owned());
                    }
                    ControlFlow::Continue(())
                });
            }
            ModuleBoundary {
                name: m.name.clone(),
                root: m.root.clone(),
                layer,
                public_exports: exports.into_iter().collect(),
                internal_dependents: dependents[i]
                    .iter()
                    .map(|d| roots[*d].name.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            }
        })
        .collect();

    let matrix: Vec<LayerEdge> = counts
        .into_iter()
        .map(|((from, to), count)| LayerEdge {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
            count,
        })
        .collect();
    let note = layers.is_empty().then(|| INSUFFICIENT_SIGNAL.to_owned());
    ArchitectureInference {
        architecture: ArchitectureModel {
            layers,
            allowed_layer_edges: Vec::new(),
            matrix,
            note,
        },
        modules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::TestGraph;

    fn layered() -> TestGraph {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::File,
            "file:users.controller",
            "users.controller.ts",
            Some("src/users/users.controller.ts"),
        )
        .node(
            NodeKind::File,
            "file:users.service",
            "users.service.ts",
            Some("src/users/users.service.ts"),
        )
        .node(
            NodeKind::File,
            "file:users.repo",
            "users.repository.ts",
            Some("src/users/users.repository.ts"),
        )
        .node(
            NodeKind::File,
            "file:orders.controller",
            "orders.controller.ts",
            Some("src/orders/orders.controller.ts"),
        )
        .node(
            NodeKind::File,
            "file:orders.service",
            "orders.service.ts",
            Some("src/orders/orders.service.ts"),
        )
        .node(
            NodeKind::File,
            "file:orders.repo",
            "orders.repository.ts",
            Some("src/orders/orders.repository.ts"),
        )
        .edge(
            EdgeKind::Imports,
            "file:users.controller",
            "file:users.service",
        )
        .edge(
            EdgeKind::Imports,
            "file:orders.controller",
            "file:orders.service",
        )
        .edge(EdgeKind::Imports, "file:users.service", "file:users.repo")
        .edge(EdgeKind::Imports, "file:orders.service", "file:orders.repo")
        // The planted violation.
        .edge(
            EdgeKind::DependsOn,
            "file:orders.controller",
            "file:orders.repo",
        );
        t
    }

    #[test]
    fn layer_matrix_counts_fixture() {
        let g = layered().build();
        let inf = infer_architecture(&g);
        let names: Vec<&str> = inf
            .architecture
            .layers
            .iter()
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(names, vec!["controller", "service", "repository"]);
        assert!(inf.architecture.layers.iter().all(|l| l.member_count == 2));
        let get = |from: &str, to: &str| {
            inf.architecture
                .matrix
                .iter()
                .find(|e| e.from == from && e.to == to)
                .map_or(0, |e| e.count)
        };
        assert_eq!(get("controller", "service"), 2);
        assert_eq!(get("service", "repository"), 2);
        assert_eq!(get("controller", "repository"), 1, "only the planted edge");
        assert_eq!(get("repository", "controller"), 0);
        assert!(inf.architecture.note.is_none());
    }

    #[test]
    fn module_public_exports_from_nest_module() {
        let mut t = layered();
        t.node(
            NodeKind::Module,
            "mod:users",
            "UsersModule",
            Some("src/users/users.module.ts"),
        )
        .node(
            NodeKind::Class,
            "cls:users.service",
            "UsersService",
            Some("src/users/users.service.ts"),
        )
        .edge(EdgeKind::Exports, "mod:users", "cls:users.service");
        let inf = infer_architecture(&t.build());
        let users = inf
            .modules
            .iter()
            .find(|m| m.name == "UsersModule")
            .unwrap();
        assert_eq!(users.root, "src/users");
        assert_eq!(users.public_exports, vec!["UsersService".to_owned()]);
    }

    #[test]
    fn directory_modules_report_dependents() {
        let mut t = layered();
        t.edge(
            EdgeKind::Imports,
            "file:orders.service",
            "file:users.service",
        );
        let inf = infer_architecture(&t.build());
        let users = inf.modules.iter().find(|m| m.root == "src/users").unwrap();
        assert_eq!(users.internal_dependents, vec!["orders".to_owned()]);
        assert!(users.layer.is_some());
    }

    #[test]
    fn no_roles_is_insufficient_signal() {
        let mut t = TestGraph::new();
        t.node(NodeKind::File, "file:a", "a.ts", Some("src/a.ts"));
        let inf = infer_architecture(&t.build());
        assert!(inf.architecture.layers.is_empty());
        assert_eq!(inf.architecture.note.as_deref(), Some(INSUFFICIENT_SIGNAL));
    }

    #[test]
    fn inference_deterministic_order() {
        let a = infer_architecture(&layered().build());
        // Same graph, nodes and edges inserted in the opposite order.
        let mut t = TestGraph::new();
        t.node(
            NodeKind::File,
            "file:orders.repo",
            "orders.repository.ts",
            Some("src/orders/orders.repository.ts"),
        )
        .node(
            NodeKind::File,
            "file:orders.service",
            "orders.service.ts",
            Some("src/orders/orders.service.ts"),
        )
        .node(
            NodeKind::File,
            "file:orders.controller",
            "orders.controller.ts",
            Some("src/orders/orders.controller.ts"),
        )
        .node(
            NodeKind::File,
            "file:users.repo",
            "users.repository.ts",
            Some("src/users/users.repository.ts"),
        )
        .node(
            NodeKind::File,
            "file:users.service",
            "users.service.ts",
            Some("src/users/users.service.ts"),
        )
        .node(
            NodeKind::File,
            "file:users.controller",
            "users.controller.ts",
            Some("src/users/users.controller.ts"),
        )
        .edge(
            EdgeKind::DependsOn,
            "file:orders.controller",
            "file:orders.repo",
        )
        .edge(EdgeKind::Imports, "file:orders.service", "file:orders.repo")
        .edge(EdgeKind::Imports, "file:users.service", "file:users.repo")
        .edge(
            EdgeKind::Imports,
            "file:orders.controller",
            "file:orders.service",
        )
        .edge(
            EdgeKind::Imports,
            "file:users.controller",
            "file:users.service",
        );
        let b = infer_architecture(&t.build());
        assert_eq!(a, b);
    }
}

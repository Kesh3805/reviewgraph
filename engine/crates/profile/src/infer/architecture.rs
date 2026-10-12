//! Layers, module boundaries and the observed layer dependency matrix (PROF-002).
//!
//! Pure over the graph; iteration is sorted, so the result is deterministic for a snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use codegraph::{
    Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeFlags, NodeKind,
};

use super::roles::{Role, RoleIndex};
use super::ProfileInputs;
use crate::model::{
    ArchitectureModel, Layer, LayerDependency, ModuleBoundary, ModuleKind, ProfileNote,
};

/// Most globs kept per layer.
const MAX_LAYER_GLOBS: usize = 10;

/// Directory names that group modules rather than being one.
const CONTAINER_DIRS: &[&str] = &[
    "modules", "features", "domains", "domain", "apps", "packages", "libs", "components",
];

/// The edge kinds that count as a dependency between layers.
pub fn dependency_kinds() -> EdgeKindSet {
    EdgeKindSet::of(EdgeKind::Imports)
        .union(EdgeKindSet::of(EdgeKind::Calls))
        .union(EdgeKindSet::of(EdgeKind::DependsOn))
}

/// Maps a path to the module that owns it.
#[derive(Debug, Clone, Default)]
pub struct ModuleIndex {
    /// `(root, id)`, longest root first.
    roots: Vec<(String, String)>,
}

impl ModuleIndex {
    pub fn new(modules: &[ModuleBoundary]) -> Self {
        let mut roots: Vec<(String, String)> = modules
            .iter()
            .map(|m| (m.root.clone(), m.id.clone()))
            .collect();
        roots.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Self { roots }
    }

    /// The owning module id; without a match, the first two directory segments.
    pub fn module_of(&self, path: &str) -> String {
        for (root, id) in &self.roots {
            if root.is_empty() || path == root || path.starts_with(&format!("{root}/")) {
                return id.clone();
            }
        }
        let dirs: Vec<&str> = path.split('/').collect();
        let take = dirs.len().saturating_sub(1).min(2);
        if take == 0 {
            return "dir:.".to_owned();
        }
        format!("dir:{}", dirs[..take].join("/"))
    }
}

/// What PROF-002 contributes to the profile.
#[derive(Debug, Clone, Default)]
pub struct ArchitectureInference {
    pub model: ArchitectureModel,
    pub modules: Vec<ModuleBoundary>,
    pub notes: Vec<ProfileNote>,
}

impl ArchitectureInference {
    pub fn module_index(&self) -> ModuleIndex {
        ModuleIndex::new(&self.modules)
    }
}

/// Infers layers, modules and the layer matrix.
pub fn infer_architecture(inputs: &ProfileInputs<'_>, roles: &RoleIndex) -> ArchitectureInference {
    let graph = inputs.graph;
    let layers = layers(roles);
    let matrix = matrix(graph, roles);
    let modules = modules(inputs);
    let mut notes = Vec::new();
    if layers.is_empty() {
        notes.push(ProfileNote {
            code: "architecture_inference".to_owned(),
            message: "insufficient_signal".to_owned(),
        });
    }
    ArchitectureInference {
        model: ArchitectureModel { layers, matrix },
        modules,
        notes,
    }
}

fn layers(roles: &RoleIndex) -> Vec<Layer> {
    struct Acc {
        members: u32,
        confidence: f32,
        globs: BTreeMap<String, u32>,
    }
    let mut by_role: BTreeMap<Role, Acc> = BTreeMap::new();
    for (_, assignment) in roles.files() {
        let acc = by_role.entry(assignment.role).or_insert(Acc {
            members: 0,
            confidence: 0.0,
            globs: BTreeMap::new(),
        });
        acc.members += 1;
        acc.confidence += assignment.confidence();
        *acc.globs.entry(assignment.pattern.clone()).or_default() += 1;
    }
    by_role
        .into_iter()
        .map(|(role, acc)| {
            let mut globs: Vec<(String, u32)> = acc.globs.into_iter().collect();
            globs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut globs: Vec<String> = globs
                .into_iter()
                .take(MAX_LAYER_GLOBS)
                .map(|(g, _)| g)
                .collect();
            globs.sort();
            Layer {
                name: role.as_str().to_owned(),
                globs,
                member_count: acc.members,
                role_confidence: round3(acc.confidence / acc.members.max(1) as f32),
            }
        })
        .collect()
}

pub(crate) fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn matrix(graph: &dyn GraphQuery, roles: &RoleIndex) -> Vec<LayerDependency> {
    let filter = EdgeFilter::kinds(dependency_kinds());
    let mut counts: BTreeMap<(Role, Role), [u32; 3]> = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        let Some(from) = roles.node_role(&node) else {
            return;
        };
        graph.for_each_edge(node.key, Direction::Out, &filter, &mut |edge| {
            if let Some(to) = roles.role_of_key(graph, edge.target) {
                if to != from {
                    let slot = counts.entry((from, to)).or_default();
                    let i = match edge.kind {
                        EdgeKind::Imports => 0,
                        EdgeKind::Calls => 1,
                        _ => 2,
                    };
                    slot[i] += 1;
                }
            }
            ControlFlow::Continue(())
        });
    });
    counts
        .into_iter()
        .map(|((from, to), [imports, calls, depends_on])| LayerDependency {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
            imports,
            calls,
            depends_on,
        })
        .collect()
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(d, _)| d.to_owned())
        .unwrap_or_default()
}

fn modules(inputs: &ProfileInputs<'_>) -> Vec<ModuleBoundary> {
    let graph = inputs.graph;
    let mut out: BTreeMap<String, ModuleBoundary> = BTreeMap::new();
    for decl in inputs.module_declarations {
        let root = parent_dir(&decl.file);
        let mut exports = decl.exports.clone();
        exports.sort();
        exports.dedup();
        out.insert(
            format!("module:{}", decl.name),
            ModuleBoundary {
                id: format!("module:{}", decl.name),
                kind: ModuleKind::FrameworkModule,
                root,
                public_exports: exports,
                internal_dependents: 0,
            },
        );
    }
    for dir in inputs.workspace_packages {
        let root = dir.trim_end_matches('/').to_owned();
        let exports = barrel_exports(graph, &[format!("{root}/src/index.ts"), format!("{root}/index.ts")]);
        out.insert(
            format!("package:{root}"),
            ModuleBoundary {
                id: format!("package:{root}"),
                kind: ModuleKind::WorkspacePackage,
                root,
                public_exports: exports,
                internal_dependents: 0,
            },
        );
    }
    if inputs.module_declarations.is_empty() {
        for root in directory_modules(inputs) {
            let exports = barrel_exports(graph, &[format!("{root}/index.ts")]);
            out.entry(format!("dir:{root}")).or_insert(ModuleBoundary {
                id: format!("dir:{root}"),
                kind: ModuleKind::Directory,
                root,
                public_exports: exports,
                internal_dependents: 0,
            });
        }
    }
    let mut modules: Vec<ModuleBoundary> = out.into_values().collect();
    count_dependents(graph, &mut modules);
    modules
}

/// Top-level directories under the source roots, descending through container directories.
fn directory_modules(inputs: &ProfileInputs<'_>) -> BTreeSet<String> {
    let mut files: BTreeSet<String> = BTreeSet::new();
    inputs.graph.for_each_node(&mut |node| {
        if let Some(path) = node.file {
            files.insert(path.to_owned());
        }
    });
    let roots: Vec<String> = if inputs.source_roots.is_empty() {
        if files.iter().any(|f| f.starts_with("src/")) {
            vec!["src".to_owned()]
        } else {
            vec![String::new()]
        }
    } else {
        inputs
            .source_roots
            .iter()
            .map(|r| r.trim_end_matches('/').to_owned())
            .collect()
    };
    let mut out = BTreeSet::new();
    for file in &files {
        for root in &roots {
            let rest = if root.is_empty() {
                file.as_str()
            } else if let Some(rest) = file.strip_prefix(&format!("{root}/")) {
                rest
            } else {
                continue;
            };
            let segments: Vec<&str> = rest.split('/').collect();
            if segments.len() < 2 {
                continue;
            }
            let mut depth = 1;
            if CONTAINER_DIRS.contains(&segments[0]) && segments.len() >= 3 {
                depth = 2;
            }
            let dir = segments[..depth].join("/");
            out.insert(if root.is_empty() {
                dir
            } else {
                format!("{root}/{dir}")
            });
        }
    }
    out
}

fn barrel_exports(graph: &dyn GraphQuery, candidates: &[String]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for path in candidates {
        graph.nodes_in_file(path, &mut |node| {
            if node.attrs.flags.contains(NodeFlags::EXPORTED)
                && !matches!(node.kind, NodeKind::File | NodeKind::Module | NodeKind::Directory)
            {
                names.insert(node.name.to_owned());
            }
        });
    }
    names.into_iter().collect()
}

fn count_dependents(graph: &dyn GraphQuery, modules: &mut [ModuleBoundary]) {
    if modules.is_empty() {
        return;
    }
    let index = ModuleIndex::new(modules);
    let filter = EdgeFilter::kinds(dependency_kinds());
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        let Some(path) = node.file else {
            return;
        };
        let from_module = index.module_of(path);
        graph.for_each_edge(node.key, Direction::Out, &filter, &mut |edge| {
            if let Some(target_path) = graph.node(edge.target).and_then(|t| t.file) {
                let to_module = index.module_of(target_path);
                if to_module != from_module {
                    dependents
                        .entry(to_module)
                        .or_default()
                        .insert(path.to_owned());
                }
            }
            ControlFlow::Continue(())
        });
    });
    for module in modules {
        module.internal_dependents = dependents
            .get(&module.id)
            .map_or(0, |files| u32::try_from(files.len()).unwrap_or(u32::MAX));
    }
}

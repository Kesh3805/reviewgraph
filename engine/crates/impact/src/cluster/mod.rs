//! Change clustering (IMP-009): changed symbols grouped into review units.
//!
//! Union-find over the non-cosmetic, non-generated, non-low-risk changed symbols. Two symbols
//! join when any of these holds:
//!
//! 1. a direct `CALLS`/`IMPLEMENTS`/`EXTENDS`/`OVERRIDES`/`USES_TYPE` edge links them (either
//!    direction, head graph, confidence ≥ 0.6);
//! 2. they share a module directory (nearest `*.module.ts` owner directory, else the parent
//!    directory) **and** are connected by a path of length ≤ 2 through the PR's impact
//!    elements — same module alone does not merge, which avoids one giant cluster per module;
//! 3. they belong to the same changed API endpoint (handler, DTO, guard);
//! 4. they write the same table, or one is the entity of a table the other writes;
//! 5. a changed test joins the cluster of the targets it covers (score ≥ 0.8).
//!
//! A cluster with more than [`MAX_CLUSTER_SIZE`] members is split by deterministic bisection:
//! its weakest internal links (lowest confidence, then key) are removed one at a time until
//! every part fits. Cosmetic, generated and low-risk symbols (and low-risk files) form one
//! `LowRisk` pseudo-cluster; changed files with no symbol at all form one `FileLevel`
//! pseudo-cluster. Cluster ids are content-derived ([`ChangeClusterKey`]), so re-runs over the
//! same head produce the same ids.
//!
//! Semantic-similarity clustering (PRD §92) is deferred to IMP-011.

pub mod union_find;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use codegraph::{
    Confidence, Direction, EdgeFilter, EdgeFlags, EdgeKind, EdgeKindSet, GraphQuery, NodeId,
    NodeKey, NodeKind,
};
use review_core::change::{ChangeCluster as CoreCluster, ChangeClusterKey};
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::{ImpactGraph, Relation};
use crate::input::ChangeSet;
use crate::metrics;

pub use union_find::UnionFind;

/// Largest cluster before splitting: the correctness reviewer's 8 symbols × 1.5.
pub const MAX_CLUSTER_SIZE: usize = 12;

/// Minimum edge confidence for rule 1.
const EDGE_FLOOR: f32 = 0.6;

/// Why members are in the same cluster.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClusterReason {
    SameModule,
    CallEdge,
    TypeEdge,
    SameApi,
    SameEntity,
    SameTestTarget,
    Singleton,
    /// The low-risk pseudo-cluster.
    LowRisk,
    /// The pseudo-cluster of changed files without symbols.
    FileLevel,
}

/// What kind of review unit a cluster is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClusterKind {
    /// Related changed symbols.
    Change,
    /// Changed files without any changed symbol (manifests, config, migrations).
    FileLevel,
    /// Cosmetic, generated and low-risk changes (IMP-010 decides whether to review them).
    LowRisk,
}

/// A review unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeCluster {
    /// `blake3` of the sorted member keys (or, for symbol-less pseudo-clusters, of the file
    /// paths), 32 hex characters.
    pub id: ChangeClusterKey,
    pub kind: ClusterKind,
    pub members: Vec<SymbolKey>,
    pub modules: Vec<String>,
    pub apis: Vec<String>,
    pub entities: Vec<String>,
    pub files: Vec<RepoPath>,
    pub reason: Vec<ClusterReason>,
    pub size_lines: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_from: Option<ChangeClusterKey>,
}

impl ChangeCluster {
    /// The DOM-005 shell (members and module) the persistence layer stores.
    pub fn to_core(&self) -> CoreCluster {
        CoreCluster::new(self.members.clone(), self.modules.first().cloned())
    }
}

/// The clustering of one pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clustering {
    pub clusters: Vec<ChangeCluster>,
    /// No graph was available: every file is its own cluster.
    pub degraded: bool,
    pub splits: u32,
}

impl Clustering {
    pub fn cluster(&self, id: ChangeClusterKey) -> Option<&ChangeCluster> {
        self.clusters.iter().find(|cluster| cluster.id == id)
    }

    /// The cluster holding `member`.
    pub fn cluster_of(&self, member: SymbolKey) -> Option<&ChangeCluster> {
        self.clusters
            .iter()
            .find(|cluster| cluster.members.contains(&member))
    }
}

/// Inputs of [`cluster_changes`].
#[derive(Clone, Copy)]
pub struct ClusterInputs<'a> {
    pub change: &'a ChangeSet,
    /// `None` degrades clustering to one cluster per file.
    pub head: Option<&'a dyn GraphQuery>,
    pub impact: Option<&'a ImpactGraph>,
    /// Symbols RISK-006 classified low-risk (not overridden).
    pub low_risk_symbols: &'a BTreeSet<SymbolKey>,
    /// Files RISK-006 classified low-risk (not overridden).
    pub low_risk_files: &'a BTreeSet<RepoPath>,
}

impl std::fmt::Debug for ClusterInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterInputs")
            .field("symbols", &self.change.symbols.len())
            .field("head", &self.head.is_some())
            .field("impact", &self.impact.is_some())
            .finish()
    }
}

/// One union with its strength, kept for splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Link {
    a: usize,
    b: usize,
    weight: u16,
    reason: ClusterReason,
}

/// `ChangeClusterKey` of a list of strings (pseudo-clusters without members).
fn key_of_strings(tag: &str, items: &[String]) -> ChangeClusterKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(tag.as_bytes());
    for item in items {
        hasher.update(&(item.len() as u64).to_le_bytes());
        hasher.update(item.as_bytes());
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    ChangeClusterKey::from_bytes(out)
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The module directory of a file: the nearest ancestor holding a `*.module.ts`, else the
/// file's own directory.
fn module_dir(
    graph: Option<&dyn GraphQuery>,
    path: &str,
    cache: &mut BTreeMap<String, bool>,
) -> String {
    let own = parent_dir(path).to_owned();
    let Some(graph) = graph else {
        return own;
    };
    let mut dir = own.clone();
    loop {
        let has_module = *cache.entry(dir.clone()).or_insert_with(|| {
            let id = if dir.is_empty() {
                NodeId::root_directory()
            } else {
                match NodeId::directory_str(&dir) {
                    Ok(id) => id,
                    Err(_) => return false,
                }
            };
            let mut found = false;
            graph.for_each_edge(
                id.key(),
                Direction::Out,
                &EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Contains)),
                &mut |edge| {
                    if graph.node(edge.target).is_some_and(|view| {
                        view.kind == NodeKind::File && view.name.ends_with(".module.ts")
                    }) {
                        found = true;
                        return std::ops::ControlFlow::Break(());
                    }
                    std::ops::ControlFlow::Continue(())
                },
            );
            found
        });
        if has_module {
            return dir;
        }
        if dir.is_empty() {
            return own;
        }
        dir = parent_dir(&dir).to_owned();
    }
}

/// Tables a member writes, plus the table of an entity member.
fn tables_of(graph: &dyn GraphQuery, member: NodeKey) -> BTreeSet<NodeKey> {
    let mut tables = BTreeSet::new();
    graph.for_each_edge(
        member,
        Direction::Out,
        &EdgeFilter::kinds(
            EdgeKindSet::of(EdgeKind::WritesTable).union(EdgeKindSet::of(EdgeKind::References)),
        ),
        &mut |edge| {
            let is_table_ref =
                edge.kind == EdgeKind::WritesTable || edge.flags.contains(EdgeFlags::MAPS_TABLE);
            if is_table_ref {
                tables.insert(edge.target);
            }
            std::ops::ControlFlow::Continue(())
        },
    );
    tables
}

/// Clusters the pull request's changed symbols.
pub fn cluster_changes(inputs: &ClusterInputs<'_>) -> Clustering {
    let started = Instant::now();
    let change = inputs.change;
    let span = tracing::info_span!(
        "change_clustering",
        clusters = tracing::field::Empty,
        max_cluster_size = tracing::field::Empty,
        splits = tracing::field::Empty,
    );
    let _entered = span.enter();

    // Partition symbols into members and low-risk.
    let mut members: Vec<SymbolKey> = Vec::new();
    let mut low_risk: BTreeSet<SymbolKey> = BTreeSet::new();
    for symbol in &change.symbols {
        let key = symbol.key();
        if symbol.cosmetic || symbol.generated || inputs.low_risk_symbols.contains(&key) {
            low_risk.insert(key);
        } else {
            members.push(key);
        }
    }
    members.sort();
    members.dedup();
    low_risk.retain(|key| members.binary_search(key).is_err());
    let index: BTreeMap<SymbolKey, usize> = members
        .iter()
        .enumerate()
        .map(|(i, key)| (*key, i))
        .collect();
    let symbol_of = |key: SymbolKey| change.symbol(key);

    let mut module_cache: BTreeMap<String, bool> = BTreeMap::new();
    let modules: Vec<String> = members
        .iter()
        .map(|key| {
            symbol_of(*key)
                .map(|symbol| module_dir(inputs.head, symbol.path().as_str(), &mut module_cache))
                .unwrap_or_default()
        })
        .collect();

    let mut links: Vec<Link> = Vec::new();
    let mut entities_of: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let degraded = inputs.head.is_none();

    if let Some(graph) = inputs.head {
        // Rule 1: direct edges.
        let kinds = EdgeKindSet::from_kinds([
            EdgeKind::Calls,
            EdgeKind::Implements,
            EdgeKind::Extends,
            EdgeKind::Overrides,
            EdgeKind::UsesType,
        ]);
        let filter = EdgeFilter::new(kinds, Confidence::from_f32(EDGE_FLOOR));
        for (a, key) in members.iter().enumerate() {
            graph.for_each_edge(*key, Direction::Out, &filter, &mut |edge| {
                if let Some(&b) = index.get(&edge.target) {
                    if a != b {
                        links.push(Link {
                            a: a.min(b),
                            b: a.max(b),
                            weight: edge.confidence.as_permille(),
                            reason: if edge.kind == EdgeKind::Calls {
                                ClusterReason::CallEdge
                            } else {
                                ClusterReason::TypeEdge
                            },
                        });
                    }
                }
                std::ops::ControlFlow::Continue(())
            });
        }

        // Rule 4: shared tables.
        let mut by_table: BTreeMap<NodeKey, Vec<usize>> = BTreeMap::new();
        for (i, key) in members.iter().enumerate() {
            for table in tables_of(graph, *key) {
                by_table.entry(table).or_default().push(i);
                if let Some(view) = graph.node(table) {
                    entities_of.entry(i).or_default().insert(view.id.to_owned());
                }
            }
        }
        for group in by_table.values() {
            for pair in group.windows(2) {
                links.push(Link {
                    a: pair[0].min(pair[1]),
                    b: pair[0].max(pair[1]),
                    weight: 800,
                    reason: ClusterReason::SameEntity,
                });
            }
        }
    }

    // Rule 2: same module and a path of length ≤ 2 through impact elements.
    if let Some(impact) = inputs.impact {
        let mut near: BTreeMap<usize, BTreeMap<NodeKey, (u8, u16)>> = BTreeMap::new();
        for symbol in &impact.symbols {
            let Some(&i) = index.get(&symbol.seed) else {
                continue;
            };
            let entry = near.entry(i).or_default();
            for element in &symbol.elements {
                if element.distance <= 2 && !element.weak {
                    let slot = entry
                        .entry(element.node)
                        .or_insert((element.distance, element.min_confidence.as_permille()));
                    if element.distance < slot.0 {
                        *slot = (element.distance, element.min_confidence.as_permille());
                    }
                }
            }
        }
        for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                if modules[a] != modules[b] {
                    continue;
                }
                let empty = BTreeMap::new();
                let (na, nb) = (
                    near.get(&a).unwrap_or(&empty),
                    near.get(&b).unwrap_or(&empty),
                );
                let direct = na
                    .get(&members[b])
                    .or_else(|| nb.get(&members[a]))
                    .map(|(_, weight)| *weight);
                let shared = na
                    .iter()
                    .filter(|(_, (distance, _))| *distance == 1)
                    .filter_map(|(node, (_, wa))| {
                        nb.get(node)
                            .filter(|(distance, _)| *distance == 1)
                            .map(|(_, wb)| (*wa).min(*wb))
                    })
                    .max();
                if let Some(weight) = direct.or(shared) {
                    links.push(Link {
                        a,
                        b,
                        weight,
                        reason: ClusterReason::SameModule,
                    });
                }
            }
        }

        // Rule 5: a changed test joins its targets (score ≥ 0.8, not mocked).
        for symbol in &impact.symbols {
            let Some(&target) = index.get(&symbol.seed) else {
                continue;
            };
            for element in symbol.of(Relation::Test) {
                let covered = element
                    .test
                    .as_ref()
                    .is_some_and(|mapping| !mapping.mocked && mapping.score >= 0.8);
                if !covered {
                    continue;
                }
                let test_file = test_file_of(&element.node_id);
                for (i, key) in members.iter().enumerate() {
                    if i == target {
                        continue;
                    }
                    let same_node = *key == element.node;
                    let same_file = symbol_of(*key)
                        .is_some_and(|s| s.test && test_file.as_deref() == Some(s.path().as_str()));
                    if same_node || same_file {
                        links.push(Link {
                            a: i.min(target),
                            b: i.max(target),
                            weight: 900,
                            reason: ClusterReason::SameTestTarget,
                        });
                    }
                }
            }
        }
    }
    for test in &change.tests {
        let in_file: Vec<usize> = members
            .iter()
            .enumerate()
            .filter(|(_, key)| symbol_of(**key).is_some_and(|s| s.path() == &test.path))
            .map(|(i, _)| i)
            .collect();
        for target in &test.targets {
            let Some(&t) = index.get(target) else {
                continue;
            };
            for i in &in_file {
                if *i != t {
                    links.push(Link {
                        a: (*i).min(t),
                        b: (*i).max(t),
                        weight: 900,
                        reason: ClusterReason::SameTestTarget,
                    });
                }
            }
        }
    }

    // Rule 3: same changed API.
    let mut apis_of: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for api in &change.apis {
        let group: Vec<usize> = api
            .handler
            .iter()
            .chain(api.related.iter())
            .filter_map(|key| index.get(key).copied())
            .collect();
        for i in &group {
            apis_of.entry(*i).or_default().insert(api.endpoint.clone());
        }
        for pair in group.windows(2) {
            if pair[0] != pair[1] {
                links.push(Link {
                    a: pair[0].min(pair[1]),
                    b: pair[0].max(pair[1]),
                    weight: 1000,
                    reason: ClusterReason::SameApi,
                });
            }
        }
    }

    links.sort_by_key(|link| (link.a, link.b, std::cmp::Reverse(link.weight), link.reason));
    links.dedup_by_key(|link| (link.a, link.b, link.reason));

    // Components, by file when degraded.
    let mut groups: Vec<Vec<usize>> = if degraded {
        let mut by_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, key) in members.iter().enumerate() {
            let file = symbol_of(*key)
                .map(|s| s.path().as_str().to_owned())
                .unwrap_or_default();
            by_file.entry(file).or_default().push(i);
        }
        by_file.into_values().collect()
    } else {
        let mut uf = UnionFind::new(members.len());
        for link in &links {
            uf.union(link.a, link.b);
        }
        uf.components()
    };

    // Split oversized groups by removing their weakest internal links.
    let mut splits = 0u32;
    let mut parts: Vec<(Vec<usize>, Option<ChangeClusterKey>)> = Vec::new();
    for group in groups.drain(..) {
        if group.len() <= MAX_CLUSTER_SIZE {
            parts.push((group, None));
            continue;
        }
        splits += 1;
        let origin = ChangeClusterKey::of(&group.iter().map(|i| members[*i]).collect::<Vec<_>>());
        let set: BTreeSet<usize> = group.iter().copied().collect();
        let mut internal: Vec<Link> = links
            .iter()
            .filter(|link| set.contains(&link.a) && set.contains(&link.b))
            .copied()
            .collect();
        // Weakest first: lowest weight, then smallest member keys.
        internal.sort_by_key(|link| (link.weight, members[link.a], members[link.b], link.reason));
        let components = loop {
            let mut uf = UnionFind::new(members.len());
            for link in &internal {
                uf.union(link.a, link.b);
            }
            let components: Vec<Vec<usize>> = uf
                .components()
                .into_iter()
                .filter(|component| set.contains(&component[0]))
                .collect();
            if components.iter().all(|c| c.len() <= MAX_CLUSTER_SIZE) || internal.is_empty() {
                break components;
            }
            internal.remove(0);
        };
        for component in components {
            parts.push((component, Some(origin)));
        }
    }

    let mut clusters: Vec<ChangeCluster> = Vec::new();
    for (part, split_from) in parts {
        let part_set: BTreeSet<usize> = part.iter().copied().collect();
        let mut reasons: BTreeSet<ClusterReason> = links
            .iter()
            .filter(|link| part_set.contains(&link.a) && part_set.contains(&link.b))
            .map(|link| link.reason)
            .collect();
        if reasons.is_empty() {
            reasons.insert(ClusterReason::Singleton);
        }
        let mut sorted: Vec<SymbolKey> = part.iter().map(|i| members[*i]).collect();
        sorted.sort();
        let mut files: BTreeSet<RepoPath> = BTreeSet::new();
        let mut module_set: BTreeSet<String> = BTreeSet::new();
        let mut apis: BTreeSet<String> = BTreeSet::new();
        let mut entities: BTreeSet<String> = BTreeSet::new();
        let mut size_lines = 0u32;
        for i in &part {
            if let Some(symbol) = symbol_of(members[*i]) {
                files.insert(symbol.path().clone());
                size_lines = size_lines.saturating_add(symbol.size_lines);
            }
            module_set.insert(modules[*i].clone());
            if let Some(found) = apis_of.get(i) {
                apis.extend(found.iter().cloned());
            }
            if let Some(found) = entities_of.get(i) {
                entities.extend(found.iter().cloned());
            }
        }
        clusters.push(ChangeCluster {
            id: ChangeClusterKey::of(&sorted),
            kind: ClusterKind::Change,
            members: sorted,
            modules: module_set.into_iter().collect(),
            apis: apis.into_iter().collect(),
            entities: entities.into_iter().collect(),
            files: files.into_iter().collect(),
            reason: reasons.into_iter().collect(),
            size_lines,
            split_from,
        });
    }

    // Files covered by a change cluster.
    let covered: BTreeSet<RepoPath> = clusters
        .iter()
        .flat_map(|cluster| cluster.files.iter().cloned())
        .collect();

    // The low-risk pseudo-cluster: low-risk symbols plus low-risk files with nothing else.
    let mut low_files: BTreeSet<RepoPath> = BTreeSet::new();
    let mut low_lines = 0u32;
    for key in &low_risk {
        if let Some(symbol) = symbol_of(*key) {
            if !covered.contains(symbol.path()) {
                low_files.insert(symbol.path().clone());
            }
            low_lines = low_lines.saturating_add(symbol.size_lines);
        }
    }
    let symbol_files: BTreeSet<&RepoPath> = change.symbols.iter().map(|s| s.path()).collect();
    let mut file_level: BTreeSet<RepoPath> = BTreeSet::new();
    for file in &change.files {
        if file.is_unanalyzed() || covered.contains(&file.path) {
            continue;
        }
        if inputs.low_risk_files.contains(&file.path) {
            if !symbol_files.contains(&file.path) || low_files.contains(&file.path) {
                low_files.insert(file.path.clone());
                if !symbol_files.contains(&file.path) {
                    low_lines =
                        low_lines.saturating_add(file.additions.saturating_add(file.deletions));
                }
            }
            continue;
        }
        if !symbol_files.contains(&file.path) && !file.parse_degraded {
            file_level.insert(file.path.clone());
        }
    }
    if !file_level.is_empty() {
        let files: Vec<RepoPath> = file_level.into_iter().collect();
        let names: Vec<String> = files.iter().map(|p| p.as_str().to_owned()).collect();
        let size_lines = change
            .files
            .iter()
            .filter(|f| files.contains(&f.path))
            .map(|f| f.additions.saturating_add(f.deletions))
            .sum();
        let mut module_set: BTreeSet<String> = BTreeSet::new();
        for file in &names {
            module_set.insert(module_dir(inputs.head, file, &mut module_cache));
        }
        clusters.push(ChangeCluster {
            id: key_of_strings("file-level", &names),
            kind: ClusterKind::FileLevel,
            members: Vec::new(),
            modules: module_set.into_iter().collect(),
            apis: Vec::new(),
            entities: Vec::new(),
            files,
            reason: vec![ClusterReason::FileLevel],
            size_lines,
            split_from: None,
        });
    }
    if !low_risk.is_empty() || !low_files.is_empty() {
        let members: Vec<SymbolKey> = low_risk.iter().copied().collect();
        let files: Vec<RepoPath> = low_files.into_iter().collect();
        let id = if members.is_empty() {
            let names: Vec<String> = files.iter().map(|p| p.as_str().to_owned()).collect();
            key_of_strings("low-risk", &names)
        } else {
            ChangeClusterKey::of(&members)
        };
        let mut all_files: BTreeSet<RepoPath> = files.into_iter().collect();
        for key in &members {
            if let Some(symbol) = symbol_of(*key) {
                all_files.insert(symbol.path().clone());
            }
        }
        clusters.push(ChangeCluster {
            id,
            kind: ClusterKind::LowRisk,
            members,
            modules: Vec::new(),
            apis: Vec::new(),
            entities: Vec::new(),
            files: all_files.into_iter().collect(),
            reason: vec![ClusterReason::LowRisk],
            size_lines: low_lines,
            split_from: None,
        });
    }

    clusters.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.id.cmp(&b.id)));
    let max_size = clusters.iter().map(|c| c.members.len()).max().unwrap_or(0);
    span.record("clusters", clusters.len() as u64);
    span.record("max_cluster_size", max_size as u64);
    span.record("splits", u64::from(splits));
    metrics::record_cluster_sizes(clusters.iter().map(|c| c.members.len()));
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros() as u64,
        "clustered"
    );
    Clustering {
        clusters,
        degraded,
        splits,
    }
}

/// `test:src/a.spec.ts#Suite › case` → `src/a.spec.ts`.
fn test_file_of(node_id: &str) -> Option<String> {
    node_id
        .strip_prefix("test:")
        .and_then(|rest| rest.split_once('#'))
        .map(|(file, _)| file.to_owned())
}

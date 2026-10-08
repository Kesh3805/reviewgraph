//! Test mapping (IMP-005).
//!
//! Every (test case `t`, changed symbol `s`) pair is scored from independent signals, each in
//! `[0, 1]`, combined as `score = 1 − Π(1 − signal)`:
//!
//! | Signal | Value | Evidence |
//! |---|---|---|
//! | `invocation` | 1.0 | `t`, or a test-file helper within depth 2, reaches `s` through `CALLS`/`TESTS` (reverse search from `s` over test-file nodes, depth ≤ 3) |
//! | `tests_edge` | 1.0 | `TESTS(t → s)`, or `TESTS(suite of t → class of s)` |
//! | `import` | 0.8 / 0.6 | the test file imports the module of `s` / a barrel (`index.*`) re-exporting it |
//! | `naming` | 0.6 | a suite/case name token equals `s`'s name or class name, or `x.spec.ts`/`x.test.ts`/`x.e2e-spec.ts` matches `x.ts` |
//! | `path` | 0.4 | the test lives in the conventional place: same directory, `__tests__/`, or a `test/` mirror of `src/` |
//! | `mock` | 0.5 | `t` mocks `s`'s class (`jest.mock`, `useValue` override) — depends on it without exercising it |
//!
//! A test is included when `score ≥ 0.6`; a mock-only test is included with `mocked = true`.
//! `untested(s)` holds when no included, non-mocked test scores ≥ 0.8. Without test-case nodes
//! (no Jest adapter) only `import`, `naming` and `path` are available, the elements are test
//! *files*, and the graph is flagged `test_mapping_degraded`.

use std::collections::{BTreeMap, BTreeSet};

use codegraph::{
    Confidence, Direction, EdgeKind, EdgeKindSet, GraphQuery, NodeFlags, NodeId, NodeKey, NodeKind,
};
use review_core::location::RepoPath;

use super::builder::{collect_edges, Cx, SeedState};
use super::model::{GraphSide, Relation, TestMapping, TestSignals, TestTargets};
use super::path::{step, Extras, Trail};
use crate::input::ChangeSet;

pub const INVOCATION: f32 = 1.0;
pub const TESTS_EDGE: f32 = 1.0;
pub const IMPORT: f32 = 0.8;
pub const IMPORT_BARREL: f32 = 0.6;
pub const NAMING: f32 = 0.6;
pub const PATH: f32 = 0.4;
pub const MOCK: f32 = 0.5;
/// Inclusion threshold.
pub const INCLUDE: f32 = 0.6;
/// A test at or above this score (and not mocked) clears `untested`.
pub const COVERED: f32 = 0.8;

const TEST_SUFFIXES: [&str; 3] = [".e2e-spec.", ".spec.", ".test."];

/// Does a repository path look like a test file (INIT-006 conventions)?
pub fn is_test_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    TEST_SUFFIXES.iter().any(|suffix| file.contains(suffix))
        || path.contains("/__tests__/")
        || path.starts_with("__tests__/")
}

/// `src/a/auth.service.spec.ts` → `auth.service`; `src/a/auth.service.ts` → `auth.service`.
fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    for suffix in TEST_SUFFIXES {
        if let Some(index) = file.find(suffix) {
            return file[..index].to_owned();
        }
    }
    file.rsplit_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned()
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Same directory, `__tests__/` beside the source, or a `test/`/`tests/` mirror of `src/`.
fn conventional_location(test: &str, source: &str) -> bool {
    let (test_dir, source_dir) = (dir_of(test), dir_of(source));
    if test_dir == source_dir {
        return true;
    }
    if test_dir == format!("{source_dir}/__tests__")
        || (source_dir.is_empty() && test_dir == "__tests__")
    {
        return true;
    }
    let source_rel = source_dir
        .strip_prefix("src")
        .map(|rest| rest.trim_start_matches('/'));
    let test_rel = test_dir
        .strip_prefix("tests")
        .or_else(|| test_dir.strip_prefix("test"))
        .map(|rest| rest.trim_start_matches('/'));
    matches!((source_rel, test_rel), (Some(a), Some(b)) if a == b)
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// One test case (or, degraded, one test file) known to the index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCaseInfo {
    key: NodeKey,
    file: String,
    name: String,
    suite: Option<NodeKey>,
    suite_name: Option<String>,
}

/// The test nodes of a head graph, computed once per impact build.
#[derive(Debug, Clone, Default)]
pub struct TestIndex {
    cases: BTreeMap<NodeKey, TestCaseInfo>,
    by_file: BTreeMap<String, Vec<NodeKey>>,
    /// Suite → its cases.
    suites: BTreeMap<NodeKey, Vec<NodeKey>>,
    /// Paths of test files (by convention or because they hold test nodes).
    test_files: BTreeSet<String>,
    /// `File` nodes of test files, for degraded mode.
    file_nodes: BTreeMap<String, NodeKey>,
    has_cases: bool,
}

impl TestIndex {
    /// Indexes every test case, suite and test file of `graph`.
    pub fn build(graph: &dyn GraphQuery) -> Self {
        let mut index = Self::default();
        let mut suite_names: BTreeMap<NodeKey, String> = BTreeMap::new();
        graph.for_each_node(&mut |node| {
            let file = node.file.map(str::to_owned);
            match node.kind {
                NodeKind::TestCase => {
                    let Some(file) = file else {
                        return;
                    };
                    index.has_cases = true;
                    index.test_files.insert(file.clone());
                    index
                        .by_file
                        .entry(file.clone())
                        .or_default()
                        .push(node.key);
                    index.cases.insert(
                        node.key,
                        TestCaseInfo {
                            key: node.key,
                            file,
                            name: node.name.to_owned(),
                            suite: None,
                            suite_name: None,
                        },
                    );
                }
                NodeKind::TestSuite => {
                    if let Some(file) = file {
                        index.test_files.insert(file);
                    }
                    suite_names.insert(node.key, node.name.to_owned());
                }
                NodeKind::File => {
                    if let Some(file) = file {
                        if is_test_path(&file) {
                            index.test_files.insert(file.clone());
                            index.file_nodes.insert(file, node.key);
                        }
                    }
                }
                _ => {
                    if let Some(file) = file {
                        if is_test_path(&file) {
                            index.test_files.insert(file);
                        }
                    }
                }
            }
        });
        for (suite, name) in suite_names {
            let mut cases: Vec<NodeKey> = collect_edges(
                graph,
                suite,
                Direction::Out,
                EdgeKindSet::of(EdgeKind::Contains),
            )
            .into_iter()
            .map(|edge| edge.target)
            .filter(|target| index.cases.contains_key(target))
            .collect();
            cases.sort();
            for case in &cases {
                if let Some(info) = index.cases.get_mut(case) {
                    info.suite = Some(suite);
                    info.suite_name = Some(name.clone());
                }
            }
            index.suites.insert(suite, cases);
        }
        for cases in index.by_file.values_mut() {
            cases.sort();
        }
        index
    }

    /// No test-case nodes: only file-level signals are available.
    pub fn degraded(&self) -> bool {
        !self.has_cases
    }

    fn is_test_node(&self, graph: &dyn GraphQuery, key: NodeKey) -> bool {
        graph.node(key).is_some_and(|view| {
            view.attrs.flags.contains(NodeFlags::TEST)
                || matches!(view.kind, NodeKind::TestCase | NodeKind::TestSuite)
                || view.file.is_some_and(|file| self.test_files.contains(file))
        })
    }

    /// Test cases of a file, sorted.
    pub fn cases_in(&self, file: &str) -> &[NodeKey] {
        self.by_file.get(file).map_or(&[], Vec::as_slice)
    }
}

/// Per-case signal accumulator.
#[derive(Debug, Clone, Default)]
struct Evidence {
    signals: TestSignals,
    trail: Option<Trail>,
}

impl Evidence {
    fn offer_trail(&mut self, trail: Trail) {
        let better = match &self.trail {
            Some(existing) => super::path::compare_trails(&trail, existing).is_lt(),
            None => true,
        };
        if better {
            self.trail = Some(trail);
        }
    }
}

fn score(signals: &TestSignals) -> f32 {
    let miss = [
        signals.invocation,
        signals.tests_edge,
        signals.import,
        signals.naming,
        signals.path,
        signals.mock,
    ]
    .iter()
    .fold(1.0f64, |acc, signal| acc * (1.0 - f64::from(*signal)));
    round4(1.0 - miss)
}

fn round4(value: f64) -> f32 {
    ((value * 10_000.0).round() / 10_000.0) as f32
}

/// The import signal of `test_file` for `source_file`: direct, through a barrel, or none.
fn import_signal(graph: &dyn GraphQuery, test_file: &str, source_file: &str) -> f32 {
    let (Ok(test_path), Ok(source_path)) = (RepoPath::new(test_file), RepoPath::new(source_file))
    else {
        return 0.0;
    };
    let test_node = NodeId::file(&test_path).key();
    let source_node = NodeId::file(&source_path).key();
    let imports = EdgeKindSet::of(EdgeKind::Imports);
    let mut best = 0.0f32;
    for edge in collect_edges(graph, test_node, Direction::Out, imports) {
        if edge.target == source_node {
            return IMPORT;
        }
        let is_barrel = graph
            .node(edge.target)
            .and_then(|view| view.file)
            .is_some_and(|file| {
                file.rsplit('/')
                    .next()
                    .is_some_and(|name| name.starts_with("index."))
            });
        if is_barrel {
            let reexports =
                EdgeKindSet::of(EdgeKind::Imports).union(EdgeKindSet::of(EdgeKind::Exports));
            if collect_edges(graph, edge.target, Direction::Out, reexports)
                .iter()
                .any(|inner| inner.target == source_node)
            {
                best = best.max(IMPORT_BARREL);
            }
        }
    }
    best
}

/// Maps tests to the seed; returns whether the seed is untested.
pub(crate) fn expand_tests(cx: &Cx<'_>, index: &TestIndex, state: &mut SeedState) -> bool {
    if state.side != GraphSide::Head {
        return false;
    }
    let graph = cx.head;
    let seed = state.seed;
    let Some(view) = graph.node(seed) else {
        return true;
    };
    let seed_name = view.name.to_lowercase();
    let Some(source_file) = view.file.map(str::to_owned) else {
        return true;
    };
    let class = view.attrs.parent;
    let class_name = class
        .and_then(|key| graph.node(key))
        .map(|owner| owner.name.to_lowercase());

    let mut evidence: BTreeMap<NodeKey, Evidence> = BTreeMap::new();

    if !index.degraded() {
        // Invocation and TESTS edges: reverse search over test-file nodes, depth ≤ 3.
        let kinds = EdgeKindSet::of(EdgeKind::Calls).union(EdgeKindSet::of(EdgeKind::Tests));
        let mut level: Vec<(NodeKey, Trail)> = vec![(seed, Trail::seed(seed))];
        let mut seen: BTreeSet<NodeKey> = BTreeSet::new();
        seen.insert(seed);
        for _depth in 1..=3u8 {
            let mut next: Vec<(NodeKey, Trail)> = Vec::new();
            for (node, trail) in &level {
                for edge in collect_edges(graph, *node, Direction::In, kinds) {
                    let source = edge.source;
                    if trail.visits(source) || !index.is_test_node(graph, source) {
                        continue;
                    }
                    let reached = trail.extend(
                        step(source, edge.kind, *node, edge.confidence, GraphSide::Head),
                        source,
                        true,
                    );
                    if index.cases.contains_key(&source) {
                        let entry = evidence.entry(source).or_default();
                        entry.signals.invocation = INVOCATION;
                        if edge.kind == EdgeKind::Tests && *node == seed {
                            entry.signals.tests_edge = TESTS_EDGE;
                        }
                        entry.offer_trail(reached.clone());
                    } else if let Some(cases) = index.suites.get(&source) {
                        if edge.kind == EdgeKind::Tests && *node == seed {
                            for case in cases {
                                let entry = evidence.entry(*case).or_default();
                                entry.signals.tests_edge = TESTS_EDGE;
                                entry.offer_trail(reached.clone());
                            }
                        }
                    }
                    if seen.insert(source) {
                        next.push((source, reached));
                    }
                }
            }
            next.sort_by_key(|(node, _)| *node);
            level = next;
            if level.is_empty() {
                break;
            }
        }

        // TESTS(suite → class of s).
        if let Some(class) = class {
            for edge in collect_edges(
                graph,
                class,
                Direction::In,
                EdgeKindSet::of(EdgeKind::Tests),
            ) {
                if let Some(cases) = index.suites.get(&edge.source) {
                    let trail = Trail::seed(seed).extend(
                        step(
                            edge.source,
                            EdgeKind::Tests,
                            class,
                            edge.confidence,
                            GraphSide::Head,
                        ),
                        edge.source,
                        true,
                    );
                    for case in cases {
                        let entry = evidence.entry(*case).or_default();
                        entry.signals.tests_edge = TESTS_EDGE;
                        entry.offer_trail(trail.clone());
                    }
                }
            }
        }
    }

    // File-level signals for every test file.
    let source_stem = stem(&source_file);
    let mut file_signals: BTreeMap<&str, (f32, f32, f32)> = BTreeMap::new();
    for test_file in &index.test_files {
        let import = import_signal(graph, test_file, &source_file);
        let naming = if stem(test_file) == source_stem {
            NAMING
        } else {
            0.0
        };
        let path = if conventional_location(test_file, &source_file) {
            PATH
        } else {
            0.0
        };
        if import > 0.0 || naming > 0.0 || path > 0.0 {
            file_signals.insert(test_file.as_str(), (import, naming, path));
        }
    }

    // Mocks: the case, its suite or its file mocks the seed's class (or the seed itself).
    let mocked_targets: BTreeSet<NodeKey> =
        class.into_iter().chain(std::iter::once(seed)).collect();
    let mockers: BTreeSet<NodeKey> = cx
        .change
        .mocks
        .iter()
        .filter(|fact| mocked_targets.contains(&fact.target))
        .map(|fact| fact.test)
        .collect();

    let mut mapped: Vec<(NodeKey, TestMapping, Trail)> = Vec::new();
    if index.degraded() {
        for (file, (import, naming, path)) in &file_signals {
            let Some(file_node) = index.file_nodes.get(*file) else {
                continue;
            };
            let signals = TestSignals {
                import: *import,
                naming: *naming,
                path: *path,
                mock: if mockers.contains(file_node) {
                    MOCK
                } else {
                    0.0
                },
                ..TestSignals::default()
            };
            if let Some(mapping) = mapping_of(signals) {
                let trail = heuristic_trail(seed, *file_node, &mapping);
                mapped.push((*file_node, mapping, trail));
            }
        }
    } else {
        let mut candidates: BTreeSet<NodeKey> = evidence.keys().copied().collect();
        for file in file_signals.keys() {
            candidates.extend(index.cases_in(file).iter().copied());
        }
        for key in &mockers {
            if index.cases.contains_key(key) {
                candidates.insert(*key);
            } else if let Some(cases) = index.suites.get(key) {
                candidates.extend(cases.iter().copied());
            }
        }
        for case in candidates {
            let Some(info) = index.cases.get(&case) else {
                continue;
            };
            let mut signals = evidence.get(&case).map(|e| e.signals).unwrap_or_default();
            if let Some((import, naming, path)) = file_signals.get(info.file.as_str()) {
                signals.import = *import;
                signals.naming = *naming;
                signals.path = *path;
            }
            let mut names = tokens(&info.name);
            if let Some(suite) = &info.suite_name {
                names.extend(tokens(suite));
            }
            let name_hit = names.contains(&seed_name)
                || class_name
                    .as_ref()
                    .is_some_and(|class| names.contains(class));
            if name_hit {
                signals.naming = NAMING;
            }
            let file_node = RepoPath::new(info.file.as_str())
                .ok()
                .map(|path| NodeId::file(&path).key());
            let is_mocker = mockers.contains(&case)
                || info.suite.is_some_and(|suite| mockers.contains(&suite))
                || file_node.is_some_and(|node| mockers.contains(&node));
            if is_mocker {
                signals.mock = MOCK;
            }
            if let Some(mapping) = mapping_of(signals) {
                let trail = evidence
                    .get(&case)
                    .and_then(|e| e.trail.clone())
                    .unwrap_or_else(|| heuristic_trail(seed, case, &mapping));
                mapped.push((case, mapping, trail));
            }
        }
    }

    let untested = !mapped
        .iter()
        .any(|(_, mapping, _)| !mapping.mocked && mapping.score >= COVERED);

    // Cap by score: strongest first, then node key.
    mapped.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    for (node, mapping, trail) in mapped {
        let extras = Extras {
            test: Some(mapping),
            ..Extras::default()
        };
        if let Some(candidate) = cx.candidate(GraphSide::Head, node, trail, extras) {
            state.admit(Relation::Test, cx.budget.max_tests, candidate);
        }
    }
    untested
}

fn mapping_of(signals: TestSignals) -> Option<TestMapping> {
    let score = score(&signals);
    let exercised = signals.invocation > 0.0 || signals.tests_edge > 0.0;
    let mocked = signals.mock > 0.0 && !exercised;
    if score >= INCLUDE || signals.mock > 0.0 {
        Some(TestMapping {
            score,
            signals,
            mocked,
        })
    } else {
        None
    }
}

/// A path-less mapping (naming, import, path signals): one hop whose confidence is the score.
fn heuristic_trail(seed: NodeKey, test: NodeKey, mapping: &TestMapping) -> Trail {
    Trail {
        steps: Vec::new(),
        nodes: vec![seed, test],
        min_confidence: Confidence::from_f32(mapping.score),
        distance: 1,
    }
}

/// For every changed test file: the production symbols its cases target (`TESTS` edges on head)
/// plus the targets the change model already knows.
pub(crate) fn changed_test_targets(
    change: &ChangeSet,
    head: &dyn GraphQuery,
    index: &TestIndex,
) -> Vec<TestTargets> {
    let mut out: Vec<TestTargets> = change
        .tests
        .iter()
        .map(|test| {
            let mut targets: BTreeSet<NodeKey> = test.targets.iter().copied().collect();
            for case in index.cases_in(test.path.as_str()) {
                for edge in collect_edges(
                    head,
                    *case,
                    Direction::Out,
                    EdgeKindSet::of(EdgeKind::Tests),
                ) {
                    targets.insert(edge.target);
                }
            }
            TestTargets {
                test_path: test.path.as_str().to_owned(),
                targets: targets.into_iter().collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.test_path.cmp(&b.test_path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_and_stems() {
        assert!(is_test_path("src/auth/auth.service.spec.ts"));
        assert!(is_test_path("test/users.e2e-spec.ts"));
        assert!(is_test_path("src/a/__tests__/b.ts"));
        assert!(!is_test_path("src/auth/auth.service.ts"));
        assert_eq!(stem("src/auth/auth.service.spec.ts"), "auth.service");
        assert_eq!(stem("src/auth/auth.service.ts"), "auth.service");
        assert_eq!(stem("test/users.e2e-spec.ts"), "users");
    }

    #[test]
    fn conventional_locations() {
        assert!(conventional_location("src/a/b.spec.ts", "src/a/b.ts"));
        assert!(conventional_location("src/a/__tests__/b.ts", "src/a/b.ts"));
        assert!(conventional_location("test/a/b.spec.ts", "src/a/b.ts"));
        assert!(!conventional_location("test/x/b.spec.ts", "src/a/b.ts"));
    }

    #[test]
    fn score_is_noisy_or() {
        let signals = TestSignals {
            naming: 0.6,
            path: 0.4,
            ..TestSignals::default()
        };
        assert!((score(&signals) - 0.76).abs() < 1e-6);
        assert_eq!(score(&TestSignals::default()), 0.0);
    }
}

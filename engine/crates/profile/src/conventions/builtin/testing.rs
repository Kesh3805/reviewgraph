//! Testing profile and `public_services_have_unit_tests` (PROF-005).
//!
//! The layout is the ratio of colocated test files (`x.service.spec.ts` next to `x.service.ts`)
//! to test files elsewhere (`test/`, `__tests__/`, or with no source sibling).

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::facts::FactKind;
use codegraph::{Direction, EdgeKind, GraphQuery, NodeFlags, NodeKind};

use super::{edges, members, node_sample};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::facts::detail_str;
use crate::infer::architecture::round3;
use crate::infer::roles::{file_name, Role};
use crate::model::{MockStyle, RepositoryProfile, TestLayout};

pub const ID: &str = "public_services_have_unit_tests";
pub const VERSION: u32 = 1;
/// Lowest `TESTS` edge confidence that counts as coverage.
pub const MIN_TESTS_CONFIDENCE: f32 = 0.6;

const TEST_MARKERS: &[&str] = &["spec", "test", "e2e-spec", "e2e", "int-spec"];

#[derive(Debug, Clone, Copy, Default)]
pub struct TestingMiner;

/// The source file a colocated test file would test: `a/x.service.spec.ts` → `a/x.service.ts`.
pub fn tested_sibling(test_path: &str) -> Option<String> {
    let (dir, name) = match test_path.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, test_path),
    };
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let kept: Vec<&str> = parts[..parts.len() - 1]
        .iter()
        .filter(|p| !TEST_MARKERS.contains(p))
        .copied()
        .collect();
    if kept.len() == parts.len() - 1 {
        return None;
    }
    let sibling = format!("{}.{}", kept.join("."), parts[parts.len() - 1]);
    Some(match dir {
        Some(dir) => format!("{dir}/{sibling}"),
        None => sibling,
    })
}

fn test_files(cx: &MiningContext<'_>) -> Vec<String> {
    cx.roles
        .files()
        .filter(|(_, a)| a.role == Role::Test)
        .map(|(p, _)| p.to_owned())
        .collect()
}

fn file_exists(graph: &dyn GraphQuery, path: &str) -> bool {
    if graph.file(path).is_some() {
        return true;
    }
    let mut found = false;
    graph.nodes_in_file(path, &mut |_| found = true);
    found
}

fn is_covered(cx: &MiningContext<'_>, sample: &Sample) -> bool {
    let graph = cx.inputs.graph;
    let Some(class) = sample.key.and_then(|k| graph.node(k)) else {
        return false;
    };
    for member in members(graph, &class) {
        let tested = edges(graph, member, Direction::In, &[EdgeKind::Tests])
            .iter()
            .any(|e| e.confidence.as_f32() >= MIN_TESTS_CONFIDENCE);
        if tested {
            return true;
        }
    }
    test_files(cx)
        .iter()
        .any(|t| tested_sibling(t).as_deref() == Some(sample.path.as_str()))
}

impl ConventionMiner for TestingMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Exported services have unit tests".to_owned()
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let mut out = Vec::new();
        cx.inputs.graph.for_each_node(&mut |node| {
            if node.kind == NodeKind::Class
                && node.attrs.flags.contains(NodeFlags::EXPORTED)
                && cx.roles.node_role(&node) == Some(Role::Service)
            {
                out.extend(node_sample(&node, ""));
            }
        });
        out
    }

    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance {
        if is_covered(cx, sample) {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: format!("no test covers `{}`", file_name(&sample.path)),
            }
        }
    }

    fn contribute(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        let graph = cx.inputs.graph;
        let tests = test_files(cx);
        let mut patterns: BTreeMap<String, u32> = BTreeMap::new();
        let mut colocated = 0u32;
        let mut separate = 0u32;
        let mut e2e_roots = BTreeSet::new();
        for path in &tests {
            if let Some(a) = cx.roles.file_role(path) {
                *patterns.entry(a.pattern.clone()).or_default() += 1;
            }
            let in_test_dir = path
                .split('/')
                .any(|s| matches!(s, "test" | "tests" | "__tests__" | "e2e"));
            let has_sibling = tested_sibling(path).is_some_and(|s| file_exists(graph, &s));
            if has_sibling && !in_test_dir {
                colocated += 1;
            } else {
                separate += 1;
            }
            if file_name(path).contains("e2e") {
                if let Some((dir, _)) = path.rsplit_once('/') {
                    e2e_roots.insert(dir.to_owned());
                }
            }
        }
        let mut patterns: Vec<(String, u32)> = patterns.into_iter().collect();
        patterns.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let total = colocated + separate;
        let ratio = if total == 0 {
            0.0
        } else {
            colocated as f32 / total as f32
        };
        let layout = match total {
            0 => TestLayout::Unknown,
            _ if ratio >= 0.8 => TestLayout::Colocated,
            _ if ratio <= 0.2 => TestLayout::Separate,
            _ => TestLayout::Mixed,
        };

        let test_set: BTreeSet<&str> = tests.iter().map(String::as_str).collect();
        let (mut jest_mock, mut overrides) = (0u32, 0u32);
        for (key, facts) in cx.inputs.facts.iter() {
            let in_test = graph
                .node(*key)
                .and_then(|n| n.file)
                .is_some_and(|f| test_set.contains(f));
            if !in_test {
                continue;
            }
            for fact in facts.iter().filter(|f| f.kind == FactKind::Call) {
                let callee = detail_str(fact, "callee").unwrap_or_default();
                if callee.starts_with("jest.mock") || callee.starts_with("vi.mock") {
                    jest_mock += 1;
                } else if callee.contains("overrideProvider") {
                    overrides += 1;
                }
            }
        }
        let mock_style = match (jest_mock, overrides) {
            (0, 0) => MockStyle::Unknown,
            (_, 0) => MockStyle::JestMock,
            (0, _) => MockStyle::ProviderOverride,
            _ => MockStyle::Mixed,
        };

        let mut frameworks: Vec<String> = cx.inputs.test_frameworks.to_vec();
        frameworks.sort();
        frameworks.dedup();
        profile.testing.frameworks = frameworks;
        profile.testing.file_patterns = patterns.into_iter().map(|(p, _)| p).collect();
        profile.testing.layout = layout;
        profile.testing.colocated_ratio = round3(ratio);
        profile.testing.mock_style = mock_style;
        profile.testing.e2e_roots = e2e_roots.into_iter().collect();
    }
}

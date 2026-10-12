//! Testing profile and `public_services_have_unit_tests` (PROF-005).

use std::collections::BTreeSet;

use analysis_ir::facts::FactKind;
use codegraph::query::GraphQuery;
use codegraph::{EdgeKind, EdgeKindSet, GraphQueryExt, NodeFlags, NodeKind};

use super::collect;
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::infer::architecture::glob_for;
use crate::infer::roles::{classify, directory_match, Role};
use crate::model::TestingProfile;

pub const ID: &str = "public_services_have_unit_tests";

#[derive(Debug, Default, Clone, Copy)]
pub struct PublicServicesHaveUnitTests;

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `users.service.spec.ts` and `users.service.test.ts` both test `users.service`.
fn tested_stem(test_path: &str) -> Option<&str> {
    let name = file_name(test_path);
    [".spec.", ".test.", ".e2e-spec."]
        .iter()
        .find_map(|marker| name.find(marker).map(|i| &name[..i]))
}

/// The source stem of a file: `users.service.ts` is `users.service`.
fn source_stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn test_files(q: &dyn GraphQuery) -> Vec<String> {
    collect(q, |n| {
        n.kind == NodeKind::File
            && classify(n.kind, n.attrs.flags, n.file).map(|a| a.role) == Some(Role::Test)
    })
    .into_iter()
    .filter_map(|f| f.file)
    .collect()
}

/// Test layout and mocking style, from the test files and the calls inside them.
pub fn infer_testing(g: &dyn GraphView) -> TestingProfile {
    let q = g.graph();
    let files = test_files(q);
    let mut globs: BTreeSet<String> = BTreeSet::new();
    let mut e2e_roots: BTreeSet<String> = BTreeSet::new();
    let mut colocated = 0u32;
    let (mut jest_mock, mut provider_override) = (0usize, 0usize);
    for path in &files {
        if let Some(a) = classify(NodeKind::File, NodeFlags::EMPTY, Some(path)) {
            globs.insert(glob_for(path, &a));
        }
        match directory_match(path) {
            Some((segment, Role::Test)) => {
                let end = path
                    .find(&format!("/{segment}/"))
                    .map_or(0, |i| i + 1 + segment.len());
                let root = if path.starts_with(&format!("{segment}/")) {
                    segment.to_owned()
                } else {
                    path[..end].to_owned()
                };
                e2e_roots.insert(root);
            }
            _ => colocated += 1,
        }
        let mut ids = Vec::new();
        q.nodes_in_file(path, &mut |n| ids.push(n.id.to_owned()));
        for id in ids {
            for f in g.facts(&id).iter().filter(|f| f.kind == FactKind::Call) {
                if f.key.contains("jest.mock") {
                    jest_mock += 1;
                }
                if f.key.contains("overrideProvider") {
                    provider_override += 1;
                }
            }
        }
    }
    let ratio = if files.is_empty() {
        0.0
    } else {
        colocated as f32 / files.len() as f32
    };
    let mock_style = match (jest_mock, provider_override) {
        (0, 0) => None,
        (j, p) if j >= p => Some("jest.mock".to_owned()),
        _ => Some("provider_override".to_owned()),
    };
    TestingProfile {
        frameworks: Vec::new(),
        test_globs: globs.into_iter().collect(),
        colocated: (!files.is_empty()).then_some(ratio >= 0.5),
        colocated_ratio: ratio,
        mock_style,
        e2e_roots: e2e_roots.into_iter().collect(),
    }
}

impl ConventionMiner for PublicServicesHaveUnitTests {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Every service has a unit test."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let q = g.graph();
        let stems: BTreeSet<String> = test_files(q)
            .iter()
            .filter_map(|p| tested_stem(p).map(str::to_owned))
            .collect();
        let tests_edges = EdgeKindSet::of(EdgeKind::Tests).union(EdgeKindSet::of(EdgeKind::Covers));
        let mut samples = Vec::new();
        for f in collect(q, |n| {
            n.kind == NodeKind::File
                && classify(n.kind, n.attrs.flags, n.file).map(|a| a.role) == Some(Role::Service)
        }) {
            let Some(path) = f.file else { continue };
            let mut keys = Vec::new();
            q.nodes_in_file(&path, &mut |n| keys.push(n.key));
            let covered = keys.iter().any(|k| !q.in_edges(*k, tests_edges).is_empty());
            let tested = covered || stems.contains(source_stem(&path));
            let meta = if tested {
                String::new()
            } else {
                format!("{} has no unit test", file_name(&path))
            };
            samples.push(Sample::new(path.clone(), path).with_meta(meta));
        }
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, _g: &dyn GraphView) -> Conformance {
        if s.meta.is_empty() {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: s.meta.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{fact, partial, OwnedView, TestGraph};

    fn file(t: &mut TestGraph, path: &str) {
        t.node(
            NodeKind::File,
            &format!("file:{path}"),
            file_name(path),
            Some(path),
        );
    }

    #[test]
    fn test_layout_colocated() {
        let mut t = TestGraph::new();
        for p in [
            "src/a/a.service.ts",
            "src/a/a.service.spec.ts",
            "src/b/b.service.ts",
            "src/b/b.service.spec.ts",
            "test/app.e2e-spec.ts",
        ] {
            file(&mut t, p);
        }
        let p = infer_testing(&OwnedView::new(t.build()));
        assert_eq!(p.colocated, Some(true));
        assert!((p.colocated_ratio - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(p.e2e_roots, vec!["test".to_owned()]);
        assert!(p.test_globs.iter().any(|g| g == "**/*.spec.ts"));
    }

    #[test]
    fn mock_style_from_calls() {
        let mut t = TestGraph::new();
        file(&mut t, "src/a/a.service.spec.ts");
        t.node(
            NodeKind::Function,
            "f:t",
            "t",
            Some("src/a/a.service.spec.ts"),
        );
        let v = OwnedView::new(t.build())
            .with_facts("f:t", vec![fact(FactKind::Call, "call:jest.mock/1", &[])]);
        assert_eq!(infer_testing(&v).mock_style.as_deref(), Some("jest.mock"));
    }

    #[test]
    fn untested_service_violates() {
        let mut t = TestGraph::new();
        for p in [
            "src/a/a.service.ts",
            "src/a/a.service.spec.ts",
            "src/b/b.service.ts",
        ] {
            file(&mut t, p);
        }
        let v = OwnedView::new(t.build());
        let samples = PublicServicesHaveUnitTests.population(&v, &partial());
        assert_eq!(samples.len(), 2);
        let verdicts: Vec<Conformance> = samples
            .iter()
            .map(|s| PublicServicesHaveUnitTests.conforms(s, &v))
            .collect();
        assert_eq!(verdicts[0], Conformance::Conforms);
        assert!(
            matches!(&verdicts[1], Conformance::Violates { evidence } if evidence.contains("b.service.ts"))
        );
    }
}

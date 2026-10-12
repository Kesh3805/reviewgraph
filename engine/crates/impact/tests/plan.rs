//! IMP-010: cluster ranking, budget allocation and the unreviewed-region report.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;

use impact::cluster::plan::BUDGET_OVERRUN_PLANNED;
use impact::cluster::{
    plan_review, ChangeCluster, ClusterKind, ClusterReason, Clustering, PlanBudget, PlanConfig,
    PlanInputs, PlannedUnit, ReviewPlan, UnreviewedReason, UnreviewedRegion,
};
use impact::input::{ChangeSet, FileDisposition, FileInput};
use impact::risk::low_risk::{LowRiskKind, LowRiskReport, LowRiskVerdict};
use impact::risk::score::{assess, ClusterMembers, ScoreInput};
use impact::risk::{
    PathRule, RiskAssessment, RiskCategory, RiskLevel, RiskPolicy, RiskSignal, RuleSource,
    SignalEvidence, SignalSubject,
};
use proptest::prelude::*;
use review_core::change::{ChangeClusterKey, FileChangeStatus};
use review_core::ids::SymbolKey;
use review_core::reviewer_type::ReviewerType;
use support::{auth_bypass, path, run_pipeline, AUTH_SERVICE, FORMAT};

fn key(n: u8) -> SymbolKey {
    SymbolKey::from_bytes([n; 16])
}

fn cluster(members: &[SymbolKey], files: &[&str], size_lines: u32) -> ChangeCluster {
    ChangeCluster {
        id: ChangeClusterKey::of(members),
        kind: ClusterKind::Change,
        members: members.to_vec(),
        modules: Vec::new(),
        apis: Vec::new(),
        entities: Vec::new(),
        files: files.iter().map(|f| path(f)).collect(),
        reason: vec![ClusterReason::Singleton],
        size_lines,
        split_from: None,
    }
}

fn pseudo_cluster(files: &[&str]) -> ChangeCluster {
    ChangeCluster {
        id: ChangeClusterKey::from_bytes([0xee; 16]),
        kind: ClusterKind::LowRisk,
        members: Vec::new(),
        modules: Vec::new(),
        apis: Vec::new(),
        entities: Vec::new(),
        files: files.iter().map(|f| path(f)).collect(),
        reason: vec![ClusterReason::LowRisk],
        size_lines: 2,
        split_from: None,
    }
}

fn signal_for(
    member: SymbolKey,
    category: RiskCategory,
    weight: f32,
    floor: Option<RiskLevel>,
) -> RiskSignal {
    RiskSignal {
        rule_id: "test.rule".to_owned(),
        category,
        subject: SignalSubject::Symbol(member),
        source: RuleSource::Change,
        weight,
        confidence: 1.0,
        floor_level: floor,
        evidence: vec![SignalEvidence::detail("test")],
        explanation: String::new(),
        contribution: 0.0,
    }
}

fn clustering(clusters: Vec<ChangeCluster>) -> Clustering {
    Clustering {
        clusters,
        degraded: false,
        splits: 0,
    }
}

fn assessed(clustering: &Clustering, signals: Vec<RiskSignal>) -> RiskAssessment {
    let clusters: BTreeMap<ChangeClusterKey, ClusterMembers> = clustering
        .clusters
        .iter()
        .map(|c| {
            (
                c.id,
                ClusterMembers {
                    symbols: c.members.iter().copied().collect(),
                    files: c.files.iter().cloned().collect(),
                    untested: false,
                    non_cosmetic: c.kind != ClusterKind::LowRisk,
                },
            )
        })
        .collect();
    assess(&ScoreInput {
        signals,
        clusters,
        ..ScoreInput::default()
    })
}

fn change_of(files: &[&str]) -> ChangeSet {
    ChangeSet {
        files: files
            .iter()
            .map(|f| FileInput::new(path(f), FileChangeStatus::Modified))
            .collect(),
        ..ChangeSet::default()
    }
}

fn plan(
    change: &ChangeSet,
    clustering: &Clustering,
    assessment: &RiskAssessment,
    low_risk: Option<&LowRiskReport>,
    config: &PlanConfig,
) -> ReviewPlan {
    plan_review(&PlanInputs {
        change,
        clustering,
        assessment,
        low_risk,
        config,
    })
}

fn config_with(tokens: u64, calls: u32, symbols: u32) -> PlanConfig {
    PlanConfig {
        budget: PlanBudget {
            model_tokens: tokens,
            model_calls: calls,
            reviewed_symbols: symbols,
        },
        ..PlanConfig::default()
    }
}

#[test]
fn critical_cluster_first() {
    let (a, b) = (key(1), key(2));
    let critical = cluster(&[a], &["src/a.ts"], 5);
    let medium = cluster(&[b], &["src/b.ts"], 40);
    let clusters = clustering(vec![medium.clone(), critical.clone()]);
    let assessment = assessed(
        &clusters,
        vec![
            signal_for(a, RiskCategory::Secrets, 0.9, Some(RiskLevel::Critical)),
            signal_for(b, RiskCategory::Concurrency, 0.3, None),
        ],
    );
    let result = plan(
        &change_of(&["src/a.ts", "src/b.ts"]),
        &clusters,
        &assessment,
        None,
        &PlanConfig::default(),
    );
    assert_eq!(result.units.len(), 2);
    assert_eq!(result.units[0].cluster_id, critical.id);
    assert_eq!(result.units[0].risk_level, RiskLevel::Critical);
    assert_eq!(result.units[0].priority, 0);
    assert_eq!(result.units[1].cluster_id, medium.id);
    assert_eq!(result.units[1].priority, 1);
    assert!(result.unreviewed.is_empty());
    assert!(!result.over_budget);
    assert!(result.units[0]
        .reviewers_hint
        .contains(&ReviewerType::Security));
}

fn low_risk_report() -> LowRiskReport {
    let mut report = LowRiskReport::default();
    report.per_file.insert(
        path("src/format.ts"),
        LowRiskVerdict {
            kind: Some(LowRiskKind::FormattingOrComments),
            overridden_by: None,
        },
    );
    report
}

#[test]
fn low_risk_skipped_when_budget_tight_and_reported() {
    let a = key(1);
    let critical = cluster(&[a], &["src/a.ts"], 5);
    let tidy = pseudo_cluster(&["src/format.ts"]);
    let clusters = clustering(vec![critical, tidy.clone()]);
    let assessment = assessed(
        &clusters,
        vec![signal_for(
            a,
            RiskCategory::Secrets,
            0.9,
            Some(RiskLevel::Critical),
        )],
    );
    let change = change_of(&["src/a.ts", "src/format.ts"]);
    let report = low_risk_report();

    let tight = plan(
        &change,
        &clusters,
        &assessment,
        Some(&report),
        &config_with(1_000, 10, 100),
    );
    assert_eq!(tight.units.len(), 1);
    assert_eq!(tight.unreviewed.len(), 1);
    let region = &tight.unreviewed[0];
    assert_eq!(region.cluster_id, Some(tidy.id));
    assert_eq!(region.reason, UnreviewedReason::LowRiskSkipped);
    assert_eq!(region.files, vec![path("src/format.ts")]);
    assert_eq!(region.kinds, vec![LowRiskKind::FormattingOrComments]);
    assert!(tight.covers_all(&change));
    assert_eq!(
        tight.summary_line(),
        "Reviewed 1 of 2 change clusters (all high/critical). \
         Skipped: 1 low-risk cluster (1 file: formatting)."
    );

    let roomy = plan(
        &change,
        &clusters,
        &assessment,
        Some(&report),
        &PlanConfig::default(),
    );
    assert_eq!(roomy.units.len(), 2);
    assert!(roomy.unreviewed.is_empty());
}

#[test]
fn medium_cluster_skipped_when_budget_exhausted() {
    let (a, b) = (key(1), key(2));
    let critical = cluster(&[a], &["src/a.ts"], 5);
    let medium = cluster(&[b], &["src/b.ts"], 5);
    let clusters = clustering(vec![critical, medium.clone()]);
    let assessment = assessed(
        &clusters,
        vec![
            signal_for(a, RiskCategory::Secrets, 0.9, Some(RiskLevel::Critical)),
            signal_for(b, RiskCategory::Concurrency, 0.3, None),
        ],
    );
    // Critical: 2 reviewers x (12,000 + 2,000) = 28,000 tokens; medium needs 10,000 more.
    let result = plan(
        &change_of(&["src/a.ts", "src/b.ts"]),
        &clusters,
        &assessment,
        None,
        &config_with(30_000, 20, 100),
    );
    assert_eq!(result.units.len(), 1);
    assert!(!result.over_budget);
    assert_eq!(result.unreviewed.len(), 1);
    assert_eq!(result.unreviewed[0].cluster_id, Some(medium.id));
    assert_eq!(
        result.unreviewed[0].reason,
        UnreviewedReason::BudgetExhausted
    );
    assert_eq!(result.unreviewed[0].risk_level, RiskLevel::Medium);
}

#[test]
fn security_cluster_never_skipped() {
    let a = key(1);
    // A weak validation signal: low score, but validation risk is never skipped for budget.
    let weak = cluster(&[a], &["src/a.ts"], 3);
    let clusters = clustering(vec![weak.clone()]);
    let assessment = assessed(
        &clusters,
        vec![signal_for(a, RiskCategory::Validation, 0.2, None)],
    );
    assert_eq!(assessment.per_cluster[&weak.id].level, RiskLevel::Low);
    let result = plan(
        &change_of(&["src/a.ts"]),
        &clusters,
        &assessment,
        None,
        &config_with(0, 0, 0),
    );
    assert_eq!(result.units.len(), 1);
    assert_eq!(result.units[0].risk_level, RiskLevel::High);
    assert!(result.units[0].over_budget);
    assert!(result.unreviewed.is_empty());
}

#[test]
fn critical_over_budget_still_planned_flagged() {
    let a = key(1);
    let critical = cluster(&[a], &["src/a.ts"], 5);
    let clusters = clustering(vec![critical]);
    let assessment = assessed(
        &clusters,
        vec![signal_for(
            a,
            RiskCategory::Secrets,
            0.9,
            Some(RiskLevel::Critical),
        )],
    );
    let result = plan(
        &change_of(&["src/a.ts"]),
        &clusters,
        &assessment,
        None,
        &config_with(1, 1, 100),
    );
    assert_eq!(result.units.len(), 1);
    assert!(result.units[0].over_budget);
    assert!(result.over_budget);
    assert_eq!(result.warnings, vec![BUDGET_OVERRUN_PLANNED.to_owned()]);
}

#[test]
fn binary_files_reported_as_unreviewed() {
    let a = key(1);
    let clusters = clustering(vec![cluster(&[a], &["src/a.ts"], 5)]);
    let assessment = assessed(
        &clusters,
        vec![signal_for(a, RiskCategory::Concurrency, 0.5, None)],
    );
    let mut change = change_of(&["src/a.ts", "assets/logo.png", "dist/bundle.min.js"]);
    change.files[1].disposition = FileDisposition::Binary;
    change.files[2].disposition = FileDisposition::Minified;
    let result = plan(
        &change,
        &clusters,
        &assessment,
        None,
        &PlanConfig::default(),
    );
    let reason_of = |file: &str| {
        result
            .unreviewed
            .iter()
            .find(|r| r.files.contains(&path(file)))
            .map(|r| r.reason)
    };
    assert_eq!(reason_of("assets/logo.png"), Some(UnreviewedReason::Binary));
    assert_eq!(
        reason_of("dist/bundle.min.js"),
        Some(UnreviewedReason::GeneratedCode)
    );
    assert_eq!(reason_of("src/a.ts"), None);
    assert!(result.covers_all(&change));
    assert!(result
        .summary_line()
        .contains("2 other files not reviewed (binary files, generated code)."));
}

#[test]
fn summary_text_golden() {
    let unit = |n: u8| PlannedUnit {
        cluster_id: ChangeClusterKey::from_bytes([n; 16]),
        priority: u32::from(n),
        risk_level: if n % 2 == 0 {
            RiskLevel::Critical
        } else {
            RiskLevel::High
        },
        reviewers_hint: Default::default(),
        token_allowance: 1,
        over_budget: false,
        files: Vec::new(),
        symbols: Vec::new(),
    };
    let region = |n: u8, files: usize, kind: LowRiskKind| UnreviewedRegion {
        cluster_id: Some(ChangeClusterKey::from_bytes([n; 16])),
        files: (0..files)
            .map(|i| path(&format!("src/r{n}/f{i}.ts")))
            .collect(),
        symbols: Vec::new(),
        risk_level: RiskLevel::Low,
        reason: UnreviewedReason::LowRiskSkipped,
        kinds: vec![kind],
    };
    let plan = ReviewPlan {
        units: (0..4).map(unit).collect(),
        unreviewed: vec![
            region(10, 8, LowRiskKind::FormattingOrComments),
            region(11, 6, LowRiskKind::ImportSort),
        ],
        clusters_total: 6,
        over_budget: false,
        warnings: Vec::new(),
    };
    assert_eq!(
        plan.summary_line(),
        "Reviewed 4 of 6 change clusters (all high/critical). \
         Skipped: 2 low-risk clusters (14 files: formatting, import sorting)."
    );
}

#[test]
fn auth_bypass_plan_single_critical_unit() {
    let scenario = auth_bypass();
    let policy = RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/auth/**",
        RiskLevel::Critical,
        None,
    )
    .unwrap()]);

    let roomy = run_pipeline(
        &scenario.change,
        &scenario.head,
        Some(&scenario.base),
        &policy,
        &PlanConfig::default(),
    );
    let critical: Vec<&PlannedUnit> = roomy
        .plan
        .units
        .iter()
        .filter(|u| u.risk_level == RiskLevel::Critical)
        .collect();
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].priority, 0);
    assert_eq!(critical[0].symbols, vec![scenario.keys.authorize]);
    assert_eq!(critical[0].files, vec![path(AUTH_SERVICE)]);
    assert!(!roomy.plan.over_budget);
    // The fixture budget plans the formatting-only file as a low unit.
    assert!(roomy
        .plan
        .units
        .iter()
        .any(|u| u.files.contains(&path(FORMAT))));
    assert!(roomy.plan.covers_all(&scenario.change));

    let tight = run_pipeline(
        &scenario.change,
        &scenario.head,
        Some(&scenario.base),
        &policy,
        &config_with(20_000, 4, 100),
    );
    assert!(tight.plan.over_budget);
    assert!(tight
        .plan
        .unreviewed
        .iter()
        .any(|r| r.files.contains(&path(FORMAT))
            && r.reason == UnreviewedReason::LowRiskSkipped
            && r.kinds == vec![LowRiskKind::FormattingOrComments]));
    assert!(tight.plan.covers_all(&scenario.change));
    assert!(tight.plan.summary_line().contains("Skipped:"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn every_file_covered_by_plan_or_unreviewed(
        spec in prop::collection::vec(
            (prop::collection::vec(0usize..8, 1..4), 0u8..=100, 0usize..4),
            1..6,
        ),
        pseudo in any::<bool>(),
        dispositions in prop::collection::vec(0u8..4, 8),
        tokens in 0u64..200_000,
        calls in 0u32..10,
        symbols in 0u32..10,
    ) {
        let names: Vec<String> = (0..8).map(|i| format!("src/f{i}.ts")).collect();
        let mut change = ChangeSet::default();
        for (index, name) in names.iter().enumerate() {
            let mut file = FileInput::new(path(name), FileChangeStatus::Modified);
            file.disposition = match dispositions[index] {
                0 => FileDisposition::Analyzed,
                1 => FileDisposition::Binary,
                2 => FileDisposition::Generated,
                _ => FileDisposition::Large,
            };
            change.files.push(file);
        }
        let categories = [
            RiskCategory::Concurrency,
            RiskCategory::Authorization,
            RiskCategory::Validation,
            RiskCategory::Queues,
        ];
        let mut clusters = Vec::new();
        let mut signals = Vec::new();
        for (index, (file_ids, weight, category)) in spec.iter().enumerate() {
            let member = key(index as u8 + 1);
            let files: Vec<&str> = file_ids.iter().map(|i| names[*i].as_str()).collect();
            clusters.push(cluster(&[member], &files, 1 + index as u32));
            signals.push(signal_for(
                member,
                categories[*category],
                f32::from(*weight) / 100.0,
                None,
            ));
        }
        if pseudo {
            clusters.push(pseudo_cluster(&[names[7].as_str()]));
        }
        let clusters = clustering(clusters);
        let assessment = assessed(&clusters, signals);
        let result = plan(
            &change,
            &clusters,
            &assessment,
            None,
            &config_with(tokens, calls, symbols),
        );
        prop_assert!(
            result.covers_all(&change),
            "uncovered: {:?}",
            result.uncovered_files(&change)
        );
        for (position, unit) in result.units.iter().enumerate() {
            prop_assert_eq!(unit.priority as usize, position);
        }
        // Security-sensitive risk is never dropped for budget.
        for region in &result.unreviewed {
            if let Some(id) = region.cluster_id {
                if let Some(scored) = assessment.per_cluster.get(&id) {
                    prop_assert!(scored.level < RiskLevel::High);
                }
            }
        }
    }
}

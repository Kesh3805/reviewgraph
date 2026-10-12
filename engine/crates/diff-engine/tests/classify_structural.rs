#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CHG-002 acceptance tests: fact diffs and the structural classifiers.

mod support;

use std::collections::BTreeMap;

use analysis_ir::facts::FactKind;
use analysis_ir::symbol::AttrValue;
use analysis_ir::unit::ParsedUnit;
use diff_engine::change::{
    build_changed_symbols, classify_all, default_classifiers, diff_facts, ChangeCfg,
    ChangedSymbolSet, ClassifyCx, GraphPair, SymbolDelta, SymbolRecord,
};
use diff_engine::hunks::{compute_hunks, HunkOptions};
use diff_engine::model::{DiffModel, FileDiff};
use diff_engine::symbol_map::{map_hunks, MapConfig, UnitMap};
use diff_engine::testkit::UnitBuilder;
use review_core::change::{ChangeClass, ChangedFile, FileChangeStatus};
use review_core::ids::CommitSha;
use review_core::location::{DiffSide, RepoPath};
use review_core::symbol::{Hash128, SymbolKind};

type F<'a> = (FactKind, &'a str, u32);

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

const OLD: &str = "\n\n\n\n// body-v1\n\n\n\n\n\n\n";
const NEW: &str = "\n\n\n\n// body-v2\n\n\n\n\n\n\n";

fn file_diff(old: &str, new: &str) -> FileDiff {
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &HunkOptions::default()).unwrap();
    let headers = set.hunks.iter().map(|h| h.header).collect();
    let changed = ChangedFile::new(
        path("src/f.ts"),
        None,
        FileChangeStatus::Modified,
        false,
        headers,
    )
    .unwrap();
    let mut diff = FileDiff::new(changed, None, None, None);
    diff.lines = Some(set.stats);
    diff.hunks = set.hunks;
    diff
}

fn model(files: Vec<FileDiff>) -> DiffModel {
    let sha: CommitSha = "0".repeat(40).parse().unwrap();
    DiffModel {
        base: sha.clone(),
        head: sha,
        merge_base: None,
        files,
        stats: Default::default(),
        coverage: Vec::new(),
    }
}

/// A unit with function `f` (lines 2..=10) carrying `facts`; `tag` sets the body hash.
fn unit(facts: &[F<'_>], tag: &str) -> ParsedUnit {
    let mut b = UnitBuilder::new(path("src/f.ts"), 12);
    let f = b.symbol(SymbolKind::Function, "f", None, 2, 10);
    for (kind, key, line) in facts {
        b.fact(f, *kind, key, *line);
    }
    let mut unit = b.build();
    unit.symbols[f as usize].body_hash = Hash128::of("body", tag.as_bytes());
    unit
}

fn classified(base: ParsedUnit, head: ParsedUnit) -> ChangedSymbolSet {
    let diff = model(vec![file_diff(OLD, NEW)]);
    let mut units = UnitMap::default();
    units.insert(DiffSide::Base, base);
    units.insert(DiffSide::Head, head);
    let map = map_hunks(&diff, &units, &MapConfig::default());
    let delta = SymbolDelta::from_units(&diff, &units, &BTreeMap::new());
    let mut set = build_changed_symbols(
        &map,
        &delta,
        &GraphPair::none(),
        &units,
        &ChangeCfg::from_diff(&diff),
    );
    let cx = ClassifyCx {
        units: &units,
        pair: GraphPair::none(),
    };
    classify_all(&mut set, &cx, &default_classifiers());
    set
}

fn run(base: &[F<'_>], head: &[F<'_>]) -> ChangedSymbolSet {
    classified(unit(base, "v1"), unit(head, "v2"))
}

fn f_record(set: &ChangedSymbolSet) -> &SymbolRecord {
    set.symbols
        .iter()
        .find(|r| r.symbol.symbol_id.as_str().contains("#f/"))
        .unwrap_or_else(|| panic!("no f record in {set:#?}"))
}

fn classes(set: &ChangedSymbolSet) -> Vec<ChangeClass> {
    f_record(set).classes.iter().map(|c| c.class).collect()
}

#[test]
fn condition_literal_change_detected() {
    let set = run(
        &[(FactKind::Condition, "if:aaaa0001", 4)],
        &[(FactKind::Condition, "if:aaaa0002", 4)],
    );
    assert_eq!(classes(&set), vec![ChangeClass::ConditionChanged]);
    let change = &f_record(&set).classes[0];
    assert_eq!(
        change.detail.get("old"),
        Some(&AttrValue::Str("if:aaaa0001".to_owned()))
    );
    assert_eq!(
        change.detail.get("new"),
        Some(&AttrValue::Str("if:aaaa0002".to_owned()))
    );
}

#[test]
fn new_comparison_expression_is_condition_changed() {
    let mut detail = BTreeMap::new();
    detail.insert("expression".to_owned(), AttrValue::Bool(true));
    let mut b = UnitBuilder::new(path("src/f.ts"), 12);
    let f = b.symbol(SymbolKind::Function, "f", None, 2, 10);
    b.fact_with(f, FactKind::Condition, "expr:bbbb0001", 4, detail);
    let mut head = b.build();
    head.symbols[f as usize].body_hash = Hash128::of("body", b"v2");
    let set = classified(unit(&[], "v1"), head);
    // An expression condition is not a branch point: only `condition_changed` fires.
    assert_eq!(classes(&set), vec![ChangeClass::ConditionChanged]);
}

#[test]
fn reorder_of_branches_is_control_flow_changed_only() {
    let set = run(
        &[
            (FactKind::Condition, "if:aaaa0001", 3),
            (FactKind::Condition, "if:aaaa0002", 5),
        ],
        &[
            (FactKind::Condition, "if:aaaa0002", 3),
            (FactKind::Condition, "if:aaaa0001", 5),
        ],
    );
    assert_eq!(classes(&set), vec![ChangeClass::ControlFlowChanged]);
    assert_eq!(
        f_record(&set).classes[0].detail.get("reordered"),
        Some(&AttrValue::Bool(true))
    );
}

#[test]
fn line_shift_changes_nothing() {
    let set = run(
        &[
            (FactKind::Condition, "if:aaaa0001", 3),
            (FactKind::Return, "return:ident", 6),
        ],
        &[
            (FactKind::Condition, "if:aaaa0001", 7),
            (FactKind::Return, "return:ident", 9),
        ],
    );
    assert!(classes(&set).is_empty());
}

#[test]
fn loop_kind_change_detected() {
    let set = run(
        &[(FactKind::Loop, "loop:for_of:11111111", 4)],
        &[(FactKind::Loop, "loop:while:22222222", 4)],
    );
    assert_eq!(classes(&set), vec![ChangeClass::LoopChanged]);
    assert_eq!(
        f_record(&set).classes[0].detail.get("subtype"),
        Some(&AttrValue::Str("kind_changed".to_owned()))
    );
}

#[test]
fn loop_removed() {
    let set = run(&[(FactKind::Loop, "loop:for:11111111", 4)], &[]);
    assert_eq!(
        classes(&set),
        vec![ChangeClass::ControlFlowChanged, ChangeClass::LoopChanged]
    );
    let looped = &f_record(&set).classes[1];
    assert_eq!(
        looped.detail.get("subtype"),
        Some(&AttrValue::Str("removed".to_owned()))
    );
}

#[test]
fn return_expression_changed() {
    let set = run(
        &[(FactKind::Return, "return:call:this.a.check", 5)],
        &[(FactKind::Return, "return:expr:cccc0003", 5)],
    );
    assert_eq!(classes(&set), vec![ChangeClass::ReturnChanged]);
}

#[test]
fn early_return_added_is_control_flow_and_return() {
    let set = run(
        &[
            (FactKind::Condition, "if:aaaa0001", 3),
            (FactKind::Return, "return:ident", 8),
        ],
        &[
            (FactKind::Condition, "if:aaaa0001", 3),
            (FactKind::Return, "return:lit", 4),
            (FactKind::Return, "return:ident", 8),
        ],
    );
    assert_eq!(
        classes(&set),
        vec![ChangeClass::ControlFlowChanged, ChangeClass::ReturnChanged]
    );
}

#[test]
fn facts_unavailable_yields_no_classes() {
    let mut base = unit(&[], "v1");
    let mut head = unit(&[], "v2");
    // Bodies exist but the analyzer produced no facts at all.
    base.symbols[1].body_token_count = 12;
    head.symbols[1].body_token_count = 14;
    let set = classified(base, head);
    let rec = f_record(&set);
    assert!(rec.classes.is_empty());
    assert!(rec.facts_unavailable);
    assert_eq!(set.facts_unavailable, vec![rec.symbol.symbol_key]);
}

#[test]
fn evidence_refs_point_to_correct_sides() {
    let set = run(
        &[(FactKind::Return, "return:old", 5)],
        &[(FactKind::Return, "return:new", 6)],
    );
    let change = &f_record(&set).classes[0];
    assert_eq!(change.evidence.len(), 2);
    assert_eq!(change.evidence[0].side, DiffSide::Base);
    assert_eq!(change.evidence[0].key, "return:old");
    assert_eq!(change.evidence[1].side, DiffSide::Head);
    assert_eq!(change.evidence[1].key, "return:new");
    assert_eq!(change.evidence[1].range.start.line, 6);
}

#[test]
fn diff_facts_counts_duplicates_and_reorder() {
    let a = unit(
        &[
            (FactKind::Call, "call:x/0", 1),
            (FactKind::Call, "call:x/0", 2),
            (FactKind::Call, "call:y/0", 3),
        ],
        "a",
    );
    let b = unit(
        &[
            (FactKind::Call, "call:y/0", 1),
            (FactKind::Call, "call:x/0", 2),
        ],
        "b",
    );
    let facts = |u: &ParsedUnit| u.facts[0].facts.clone();
    let diff = diff_facts(&facts(&a), &facts(&b), &[FactKind::Call]);
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.removed[0].key, "call:x/0");
    assert!(diff.added.is_empty());
    assert_eq!(diff.kept, 2);
    assert!(diff.reordered);
    // Kinds outside the filter are ignored.
    let none = diff_facts(&facts(&a), &facts(&b), &[FactKind::Throw]);
    assert!(none.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn golden_authorize_has_condition_and_return_changed() {
    let (scenario, diff) = support::load("auth-bypass");
    let units = support::units_for(&scenario, &diff);
    let map = map_hunks(&diff, &units, &MapConfig::default());
    let delta = SymbolDelta::from_units(&diff, &units, &BTreeMap::new());
    let mut set = build_changed_symbols(
        &map,
        &delta,
        &GraphPair::none(),
        &units,
        &ChangeCfg::from_diff(&diff),
    );
    let cx = ClassifyCx {
        units: &units,
        pair: GraphPair::none(),
    };
    classify_all(&mut set, &cx, &default_classifiers());
    let rec = set
        .symbols
        .iter()
        .find(|r| {
            r.symbol
                .symbol_id
                .as_str()
                .ends_with("AuthService.authorize/method")
        })
        .unwrap();
    let got: Vec<ChangeClass> = rec.classes.iter().map(|c| c.class).collect();
    assert_eq!(
        got,
        vec![ChangeClass::ConditionChanged, ChangeClass::ReturnChanged],
        "{:#?}",
        rec.classes
    );
    let condition = &rec.classes[0];
    assert!(condition.evidence.iter().any(|e| e.side == DiffSide::Head));
    assert!(set
        .symbols
        .iter()
        .filter(|r| r.cosmetic)
        .all(|r| r.classes.is_empty()));
    assert!(set.facts_unavailable.is_empty());
}

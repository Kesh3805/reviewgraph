#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CHG-001 acceptance tests: the changed-symbol set (hunk map joined with the hash delta).

mod support;

use std::collections::BTreeMap;

use analysis_ir::framework::{FrameworkFactKind, IrFrameworkFact};
use analysis_ir::symbol::{LocalId, Modifiers, ShingleSet, Visibility};
use analysis_ir::unit::ParsedUnit;
use diff_engine::change::{
    build_changed_symbols, ChangeCfg, ChangedSymbolSet, GraphPair, SymbolDelta,
};
use diff_engine::disposition::FileDisposition;
use diff_engine::hunks::{compute_hunks, HunkOptions};
use diff_engine::model::{DiffModel, FileDiff};
use diff_engine::symbol_map::{map_hunks, MapConfig, UnitMap};
use diff_engine::testkit::UnitBuilder;
use review_core::change::{ChangedFile, FileChangeStatus, SymbolChange};
use review_core::ids::{CommitSha, SymbolId};
use review_core::location::{DiffSide, RepoPath};
use review_core::symbol::SymbolKind;

const SVC: &str = "import { X } from 'x';\n\nexport class Svc {\n  a() {\n    return 1;\n  }\n\n  @Get()\n  b() {\n    return 2;\n  }\n}\n";

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

fn file_diff(
    p: &str,
    old_path: Option<&str>,
    status: FileChangeStatus,
    old: &str,
    new: &str,
) -> FileDiff {
    let set = compute_hunks(old.as_bytes(), new.as_bytes(), &HunkOptions::default()).unwrap();
    let headers = set.hunks.iter().map(|h| h.header).collect();
    let changed = ChangedFile::new(path(p), old_path.map(path), status, false, headers).unwrap();
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

/// Units for `SVC`: class Svc 3..12, a 4..6, b 9..11 decorated on line 8.
struct Svc {
    unit: ParsedUnit,
    class: u32,
    a: u32,
    b: u32,
}

fn svc(p: &str) -> Svc {
    let mut ub = UnitBuilder::new(path(p), 13);
    let class = ub.symbol(SymbolKind::Class, "Svc", None, 3, 12);
    let a = ub.symbol(SymbolKind::Method, "a", Some(class), 4, 6);
    let b = ub.symbol(SymbolKind::Method, "b", Some(class), 9, 11);
    ub.decorator(b, "Get", 8);
    Svc {
        unit: ub.build(),
        class,
        a,
        b,
    }
}

fn set_hashes(unit: &mut ParsedUnit, local: u32, sig: &str, body: &str, attrs: &str) {
    use review_core::symbol::Hash128;
    let s = &mut unit.symbols[local as usize];
    s.signature_hash = Hash128::of("sig", sig.as_bytes());
    s.body_hash = Hash128::of("body", body.as_bytes());
    s.attr_hash = Hash128::of("attr", attrs.as_bytes());
}

fn id_of(unit: &ParsedUnit, local: u32) -> SymbolId {
    analysis_ir::identity::symbol_id_of(unit, local).unwrap()
}

fn units(base: Option<ParsedUnit>, head: Option<ParsedUnit>) -> UnitMap {
    let mut u = UnitMap::default();
    if let Some(b) = base {
        u.insert(DiffSide::Base, b);
    }
    if let Some(h) = head {
        u.insert(DiffSide::Head, h);
    }
    u
}

fn run_with(
    diff: &DiffModel,
    units: &UnitMap,
    renames: BTreeMap<SymbolId, SymbolId>,
    cfg: &ChangeCfg,
) -> ChangedSymbolSet {
    let map = map_hunks(
        diff,
        units,
        &MapConfig {
            renames: renames.clone(),
        },
    );
    let delta = SymbolDelta::from_units(diff, units, &renames);
    build_changed_symbols(&map, &delta, &GraphPair::none(), units, cfg)
}

fn run(diff: &DiffModel, units: &UnitMap) -> ChangedSymbolSet {
    run_with(diff, units, BTreeMap::new(), &ChangeCfg::from_diff(diff))
}

fn modified_svc(old: &str, new: &str) -> (DiffModel, Svc, Svc) {
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, old, new);
    (model(vec![file]), svc("src/svc.ts"), svc("src/svc.ts"))
}

fn find<'a>(
    set: &'a ChangedSymbolSet,
    id: &SymbolId,
) -> Option<&'a diff_engine::change::SymbolRecord> {
    set.symbols.iter().find(|r| &r.symbol.symbol_id == id)
}

#[test]
fn body_edit_is_modified_body() {
    let (diff, base, mut head) = modified_svc(SVC, &SVC.replace("return 1;", "return 3;"));
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    let a = id_of(&head.unit, head.a);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let rec = find(&set, &a).unwrap();
    assert_eq!(
        rec.symbol.change,
        SymbolChange::Modified {
            signature: false,
            body: true,
            attrs: false
        }
    );
    assert!(!rec.cosmetic);
    assert_eq!(rec.hunk_ids, vec![0]);
}

#[test]
fn signature_edit_has_both_versions() {
    let (diff, base, mut head) = modified_svc(SVC, &SVC.replace("a() {", "a(x) {"));
    set_hashes(&mut head.unit, head.a, "a-v2", "a", "");
    let a = id_of(&head.unit, head.a);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let rec = find(&set, &a).unwrap();
    assert_eq!(
        rec.symbol.change,
        SymbolChange::Modified {
            signature: true,
            body: false,
            attrs: false
        }
    );
    let (b, h) = (rec.base.as_ref().unwrap(), rec.head.as_ref().unwrap());
    assert_ne!(b.signature_hash, h.signature_hash);
    assert_eq!(b.body_hash, h.body_hash);
    assert_eq!(rec.symbol.side, DiffSide::Head);
}

#[test]
fn added_symbol_has_no_base() {
    let file = file_diff("src/new.ts", None, FileChangeStatus::Added, "", SVC);
    let head = svc("src/new.ts");
    let a = id_of(&head.unit, head.a);
    let set = run(&model(vec![file]), &units(None, Some(head.unit)));
    let rec = find(&set, &a).unwrap();
    assert_eq!(rec.symbol.change, SymbolChange::Added);
    assert!(rec.base.is_none());
    assert!(rec.head.is_some());
    assert_eq!(rec.symbol.side, DiffSide::Head);
}

#[test]
fn removed_symbol_has_no_head_and_base_side_range() {
    let file = file_diff("src/old.ts", None, FileChangeStatus::Deleted, SVC, "");
    let base = svc("src/old.ts");
    let a = id_of(&base.unit, base.a);
    let base_range = base.unit.symbols[base.a as usize].range;
    let set = run(&model(vec![file]), &units(Some(base.unit), None));
    let rec = find(&set, &a).unwrap();
    assert_eq!(rec.symbol.change, SymbolChange::Removed);
    assert!(rec.head.is_none());
    assert!(rec.base.is_some());
    assert_eq!(rec.symbol.side, DiffSide::Base);
    assert_eq!(rec.symbol.range, base_range);
}

fn rename_fixture() -> (DiffModel, Svc, Svc, BTreeMap<SymbolId, SymbolId>) {
    let new = SVC.replace("  a() {", "  a2() {");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let base = svc("src/svc.ts");
    let mut ub = UnitBuilder::new(path("src/svc.ts"), 13);
    let class = ub.symbol(SymbolKind::Class, "Svc", None, 3, 12);
    let a = ub.symbol(SymbolKind::Method, "a2", Some(class), 4, 6);
    let b = ub.symbol(SymbolKind::Method, "b", Some(class), 9, 11);
    ub.decorator(b, "Get", 8);
    let head = Svc {
        unit: ub.build(),
        class,
        a,
        b,
    };
    let mut renames = BTreeMap::new();
    renames.insert(id_of(&base.unit, base.a), id_of(&head.unit, head.a));
    (model(vec![file]), base, head, renames)
}

#[test]
fn rename_carries_lineage_and_similarity() {
    let (diff, mut base, mut head, renames) = rename_fixture();
    // Identical bodies under a new name.
    set_hashes(&mut base.unit, base.a, "a", "same", "");
    set_hashes(&mut head.unit, head.a, "a2", "same", "");
    let (from, to) = renames
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    let cfg = ChangeCfg::from_diff(&diff);
    let set = run_with(
        &diff,
        &units(Some(base.unit), Some(head.unit)),
        renames,
        &cfg,
    );
    let rec = find(&set, &to).unwrap();
    match &rec.symbol.change {
        SymbolChange::Renamed {
            from: f,
            similarity,
        } => {
            assert_eq!(f, &from);
            assert!((similarity - 1.0).abs() < 1e-6, "{similarity}");
        }
        other => panic!("expected rename, got {other:?}"),
    }
    assert!(rec.base.is_some() && rec.head.is_some());
}

#[test]
fn rename_with_edit_uses_shingle_similarity() {
    let (diff, mut base, mut head, renames) = rename_fixture();
    set_hashes(&mut base.unit, base.a, "a", "old", "");
    set_hashes(&mut head.unit, head.a, "a2", "new", "");
    base.unit.symbols[base.a as usize].body_shingles = ShingleSet::of([1, 2, 3, 4]);
    head.unit.symbols[head.a as usize].body_shingles = ShingleSet::of([1, 2, 3, 5]);
    let to = renames.values().next().cloned().unwrap();
    let cfg = ChangeCfg::from_diff(&diff);
    let set = run_with(
        &diff,
        &units(Some(base.unit), Some(head.unit)),
        renames,
        &cfg,
    );
    match &find(&set, &to).unwrap().symbol.change {
        SymbolChange::Renamed { similarity, .. } => {
            assert!((similarity - 0.6).abs() < 0.01, "{similarity}");
        }
        other => panic!("expected rename, got {other:?}"),
    }
}

#[test]
fn comment_only_edit_is_cosmetic() {
    let new = SVC.replace("    return 1;", "    // note\n    return 1;");
    let (diff, base, head) = modified_svc(SVC, &new);
    let a = id_of(&head.unit, head.a);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let rec = find(&set, &a).unwrap();
    assert!(rec.cosmetic);
    assert_eq!(set.symbols.iter().filter(|r| !r.cosmetic).count(), 0);
}

#[test]
fn class_not_marked_changed_for_member_edit() {
    let (diff, base, mut head) = modified_svc(SVC, &SVC.replace("return 1;", "return 3;"));
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    // The container's own body hash covers its members, but only a header change counts.
    set_hashes(&mut head.unit, head.class, "Svc", "Svc-v2", "");
    let class = id_of(&head.unit, head.class);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    assert!(find(&set, &class).is_none());
}

#[test]
fn decorator_edit_marks_class_header_changed() {
    let new = SVC.replace("export class Svc {", "export class Svc extends Base {");
    let (diff, base, mut head) = modified_svc(SVC, &new);
    set_hashes(&mut head.unit, head.class, "Svc", "Svc", "extends");
    let class = id_of(&head.unit, head.class);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let rec = find(&set, &class).unwrap();
    assert_eq!(
        rec.symbol.change,
        SymbolChange::Modified {
            signature: false,
            body: false,
            attrs: true
        }
    );
}

#[test]
fn generated_file_symbols_flagged_or_excluded() {
    let new = SVC.replace("return 1;", "return 3;");
    let mut file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    file.disposition = FileDisposition::Vendored;
    let base = svc("src/svc.ts");
    let mut head = svc("src/svc.ts");
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    let set = run(&model(vec![file]), &units(Some(base.unit), Some(head.unit)));
    assert!(set.symbols.is_empty());
}

#[test]
fn ordering_is_deterministic() {
    let new = SVC
        .replace("return 1;", "return 3;")
        .replace("return 2;", "return 4;");
    let build = || {
        let (diff, base, mut head) = modified_svc(SVC, &new);
        set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
        set_hashes(&mut head.unit, head.b, "b", "b-v2", "");
        run(&diff, &units(Some(base.unit), Some(head.unit)))
    };
    let (first, second) = (build(), build());
    assert_eq!(first, second);
    let starts: Vec<_> = first.symbols.iter().map(|r| r.symbol.range.start).collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted);
    assert_eq!(first.symbols.len(), 2);
}

#[test]
fn truncation_flag_set_at_cap() {
    let new = SVC
        .replace("return 1;", "return 3;")
        .replace("return 2;", "return 4;");
    let (diff, base, mut head) = modified_svc(SVC, &new);
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    set_hashes(&mut head.unit, head.b, "b", "b-v2", "");
    let mut cfg = ChangeCfg::from_diff(&diff);
    cfg.max_symbols = 1;
    let set = run_with(
        &diff,
        &units(Some(base.unit), Some(head.unit)),
        BTreeMap::new(),
        &cfg,
    );
    assert!(set.truncated);
    assert_eq!(set.symbols.len(), 1);
}

#[test]
fn symbol_changed_without_a_hunk_is_found_through_the_delta() {
    // Only `a` has changed lines; `b`'s hashes differ although no hunk touches it.
    let (diff, base, mut head) = modified_svc(SVC, &SVC.replace("return 1;", "return 3;"));
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    set_hashes(&mut head.unit, head.b, "b-v2", "b", "");
    let b = id_of(&head.unit, head.b);
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let rec = find(&set, &b).unwrap();
    assert!(rec.hunk_ids.is_empty());
    assert!(!rec.cosmetic);
    assert_eq!(
        rec.symbol.change,
        SymbolChange::Modified {
            signature: true,
            body: false,
            attrs: false
        }
    );
}

#[test]
fn exported_public_api_and_entrypoint_flags() {
    let new = SVC
        .replace("return 1;", "return 3;")
        .replace("return 2;", "return 4;");
    let (diff, base, mut head) = modified_svc(SVC, &new);
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    set_hashes(&mut head.unit, head.b, "b", "b-v2", "");
    head.unit.symbols[head.class as usize]
        .modifiers
        .insert(Modifiers::EXPORTED);
    head.unit.symbols[head.a as usize].visibility = Visibility::Private;
    let route_range = head.unit.symbols[head.b as usize].range;
    head.unit.framework.push(IrFrameworkFact {
        adapter: "nestjs".to_owned(),
        kind: FrameworkFactKind::HttpRoute,
        symbol: Some(LocalId(head.b)),
        attrs: BTreeMap::new(),
        range: route_range,
        confidence: 1.0,
    });
    let (a, b) = (id_of(&head.unit, head.a), id_of(&head.unit, head.b));
    let set = run(&diff, &units(Some(base.unit), Some(head.unit)));
    let (ra, rb) = (find(&set, &a).unwrap(), find(&set, &b).unwrap());
    assert!(ra.flags.exported && !ra.flags.public_api && !ra.flags.entrypoint);
    assert!(rb.flags.exported && rb.flags.public_api && rb.flags.entrypoint);
    assert!(!rb.flags.test && !rb.flags.generated);
}

#[test]
fn test_files_are_flagged() {
    let file = file_diff(
        "src/svc.spec.ts",
        None,
        FileChangeStatus::Modified,
        SVC,
        &SVC.replace("return 1;", "return 3;"),
    );
    let base = svc("src/svc.spec.ts");
    let mut head = svc("src/svc.spec.ts");
    set_hashes(&mut head.unit, head.a, "a", "a-v2", "");
    let a = id_of(&head.unit, head.a);
    let set = run(&model(vec![file]), &units(Some(base.unit), Some(head.unit)));
    assert!(find(&set, &a).unwrap().flags.test);
}

#[test]
fn missing_unit_still_emits_record_with_coverage_note() {
    let file = file_diff(
        "src/svc.ts",
        None,
        FileChangeStatus::Modified,
        SVC,
        &SVC.replace("return 1;", "return 3;"),
    );
    let head = svc("src/svc.ts");
    let diff = model(vec![file]);
    // The map needs the head unit, the join does not.
    let map = map_hunks(
        &diff,
        &units(Some(svc("src/svc.ts").unit), Some(head.unit)),
        &MapConfig::default(),
    );
    let none = UnitMap::default();
    let set = build_changed_symbols(
        &map,
        &SymbolDelta::default(),
        &GraphPair::none(),
        &none,
        &ChangeCfg::from_diff(&diff),
    );
    assert!(!set.symbols.is_empty());
    assert!(set.symbols.iter().all(|r| r.head.is_none()));
    assert!(!set.missing_units.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn golden_authorize_has_base_and_head_versions() {
    let (scenario, diff) = support::load("auth-bypass");
    let units = support::units_for(&scenario, &diff);
    let set = run(&diff, &units);
    let non_cosmetic: Vec<_> = set.symbols.iter().filter(|r| !r.cosmetic).collect();
    assert_eq!(non_cosmetic.len(), 1, "{non_cosmetic:#?}");
    let rec = non_cosmetic[0];
    assert_eq!(
        rec.symbol.symbol_id.as_str(),
        "ts:src/auth/auth.service#AuthService.authorize/method"
    );
    assert_eq!(
        rec.symbol.change,
        SymbolChange::Modified {
            signature: false,
            body: true,
            attrs: false
        }
    );
    let (b, h) = (rec.base.as_ref().unwrap(), rec.head.as_ref().unwrap());
    assert_ne!(b.body_hash, h.body_hash);
    assert_eq!(b.signature_hash, h.signature_hash);
    assert!(rec.flags.public_api);

    let cosmetic: Vec<_> = set.symbols.iter().filter(|r| r.cosmetic).collect();
    assert_eq!(cosmetic.len(), 1, "{cosmetic:#?}");
    assert!(cosmetic[0].symbol.symbol_id.as_str().contains("formatName"));
}

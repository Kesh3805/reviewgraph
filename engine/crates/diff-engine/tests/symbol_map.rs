#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! DIFF-006 acceptance tests: changed ranges to innermost symbols.

use std::collections::BTreeSet;
use std::ops::Range;

use analysis_ir::identity::symbol_id_of;
use analysis_ir::unit::ParsedUnit;
use analysis_ir::{AnalyzerConfig, LanguageAnalyzer, SourceInput};
use diff_engine::disposition::FileDisposition;
use diff_engine::files::{diff_commits, DiffOptions};
use diff_engine::hunks::{compute_hunks, HunkOptions};
use diff_engine::model::FileDiff;
use diff_engine::symbol_map::{
    map_file, map_hunks, HitScope, MapConfig, SymbolMap, UnitMap, UnmappedReason,
};
use diff_engine::testkit::{Scenario, UnitBuilder};
use lang_typescript::TypeScriptAnalyzer;
use proptest::prelude::*;
use review_core::change::{ChangedFile, FileChangeStatus};
use review_core::location::{ContentHash, DiffSide, RepoPath};
use review_core::symbol::{ModulePath, SymbolKind};

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

const SVC: &str = "import { X } from 'x';\n\nexport class Svc {\n  a() {\n    return 1;\n  }\n\n  @Get()\n  b() {\n    return 2;\n  }\n}\n";

/// Units for `SVC`: class Svc 3..12, a 4..6, b 9..11 decorated on line 8.
fn svc_unit(p: &str) -> (ParsedUnit, u32, u32, u32) {
    let mut b = UnitBuilder::new(path(p), 13);
    let class = b.symbol(SymbolKind::Class, "Svc", None, 3, 12);
    let a = b.symbol(SymbolKind::Method, "a", Some(class), 4, 6);
    let m = b.symbol(SymbolKind::Method, "b", Some(class), 9, 11);
    b.decorator(m, "Get", 8);
    (b.build(), class, a, m)
}

fn id(unit: &ParsedUnit, local: u32) -> String {
    symbol_id_of(unit, local).unwrap().as_str().to_owned()
}

fn map_one(file: &FileDiff, head: Option<&ParsedUnit>, base: Option<&ParsedUnit>) -> SymbolMap {
    map_file(file, head, base, &MapConfig::default())
}

#[test]
fn edit_inside_method_maps_to_method_not_class() {
    let new = SVC.replace("return 1;", "return 3;");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (unit, class, a, _) = svc_unit("src/svc.ts");
    let map = map_one(&file, Some(&unit), Some(&unit));
    let head: Vec<&str> = map
        .hits
        .iter()
        .filter(|h| h.side == DiffSide::Head)
        .map(|h| h.symbol_id.as_str())
        .collect();
    assert_eq!(head, vec![id(&unit, a).as_str()]);
    assert!(map.hit(&id(&unit, class)).is_none());
    let hit = map.hit(&id(&unit, a)).unwrap();
    assert_eq!(hit.scope, HitScope::Body);
    assert_eq!(hit.ranges, vec![5..6]);
    assert_eq!(hit.ranges_old, vec![5..6]);
    assert_eq!(hit.base_local, Some(a));
    assert!(hit.touches_code);
    assert_eq!(hit.hunk_ids, vec![0]);
}

#[test]
fn edit_spanning_two_methods_yields_two_hits() {
    let new = SVC
        .replace("return 1;", "return 3;")
        .replace("return 2;", "return 4;");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (unit, _, a, m) = svc_unit("src/svc.ts");
    let map = map_one(&file, Some(&unit), Some(&unit));
    assert_eq!(map.hits.len(), 2);
    assert_eq!(map.hit(&id(&unit, a)).unwrap().ranges, vec![5..6]);
    assert_eq!(map.hit(&id(&unit, m)).unwrap().ranges, vec![10..11]);
}

#[test]
fn decorator_edit_maps_to_decorated_symbol_header() {
    let new = SVC.replace("@Get()", "@Post()");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (unit, _, _, m) = svc_unit("src/svc.ts");
    let map = map_one(&file, Some(&unit), Some(&unit));
    assert_eq!(map.hits.len(), 1);
    let hit = map.hit(&id(&unit, m)).unwrap();
    assert_eq!(hit.scope, HitScope::Decorator);
    assert_eq!(hit.ranges, vec![8..9]);
}

#[test]
fn import_line_maps_to_module_level() {
    let new = SVC.replace("import { X } from 'x';", "import { X, Y } from 'x';");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (unit, ..) = svc_unit("src/svc.ts");
    let map = map_one(&file, Some(&unit), Some(&unit));
    assert_eq!(map.hits.len(), 1);
    let hit = &map.hits[0];
    assert_eq!(hit.scope, HitScope::ModuleLevel);
    assert_eq!(hit.local, 0);
    assert_eq!(hit.kind, SymbolKind::Module);
    assert!(hit.symbol_id.as_str().contains("__module__"));
}

#[test]
fn deleted_method_uses_base_ranges() {
    // A unique closing line keeps the deletion unambiguous for the slider heuristic.
    let old = SVC.replace("    return 2;\n  }\n", "    return 2;\n  } // end b\n");
    let new = "import { X } from 'x';\n\nexport class Svc {\n  a() {\n    return 1;\n  }\n}\n";
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, &old, new);
    let (base, _, _, m) = svc_unit("src/svc.ts");
    let mut hb = UnitBuilder::new(path("src/svc.ts"), 8);
    let class = hb.symbol(SymbolKind::Class, "Svc", None, 3, 7);
    hb.symbol(SymbolKind::Method, "a", Some(class), 4, 6);
    let head = hb.build();
    let map = map_one(&file, Some(&head), Some(&base));
    let deleted = map
        .hits
        .iter()
        .find(|h| h.side == DiffSide::Base)
        .expect("deleted symbol hit");
    assert_eq!(deleted.symbol_id.as_str(), id(&base, m));
    assert!(deleted.whole_symbol);
    let lines: BTreeSet<u32> = deleted.ranges.iter().flat_map(|r| r.clone()).collect();
    assert!((9..=11).all(|l| lines.contains(&l)), "{lines:?}");
}

#[test]
fn rename_maps_to_new_key_with_lineage() {
    let new = SVC.replace("  a() {", "  renamed() {");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (base, _, a, _) = svc_unit("src/svc.ts");
    let mut hb = UnitBuilder::new(path("src/svc.ts"), 13);
    let class = hb.symbol(SymbolKind::Class, "Svc", None, 3, 12);
    let renamed = hb.symbol(SymbolKind::Method, "renamed", Some(class), 4, 6);
    let b = hb.symbol(SymbolKind::Method, "b", Some(class), 9, 11);
    hb.decorator(b, "Get", 8);
    let head = hb.build();
    let mut cfg = MapConfig::default();
    cfg.renames.insert(
        symbol_id_of(&base, a).unwrap(),
        symbol_id_of(&head, renamed).unwrap(),
    );
    let map = map_file(&file, Some(&head), Some(&base), &cfg);
    assert_eq!(map.hits.len(), 1, "{:?}", map.hits);
    let hit = &map.hits[0];
    assert_eq!(hit.symbol_id.as_str(), id(&head, renamed));
    assert_eq!(hit.side, DiffSide::Head);
    assert_eq!(
        hit.base_symbol_id.as_ref().map(|s| s.as_str().to_owned()),
        Some(id(&base, a))
    );
    assert_eq!(hit.scope, HitScope::Header);
}

#[test]
fn whole_symbol_added_flagged() {
    let new = SVC.replace(
        "    return 2;\n  }\n}\n",
        "    return 2;\n  }\n\n  c() {\n    return 5;\n  } // end c\n}\n",
    );
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (base, ..) = svc_unit("src/svc.ts");
    let mut hb = UnitBuilder::new(path("src/svc.ts"), 17);
    let class = hb.symbol(SymbolKind::Class, "Svc", None, 3, 16);
    hb.symbol(SymbolKind::Method, "a", Some(class), 4, 6);
    let b = hb.symbol(SymbolKind::Method, "b", Some(class), 9, 11);
    hb.decorator(b, "Get", 8);
    let c = hb.symbol(SymbolKind::Method, "c", Some(class), 13, 15);
    let head = hb.build();
    let map = map_one(&file, Some(&head), Some(&base));
    let hit = map.hit(&id(&head, c)).unwrap();
    assert!(hit.whole_symbol);
    assert_eq!(hit.base_local, None);
}

#[test]
fn nested_function_innermost_wins() {
    let old = "function outer() {\n  const x = 1;\n  function inner() {\n    return 1;\n  }\n  return inner();\n}\n";
    let new = old.replace("return 1;", "return 2;");
    let file = file_diff("src/n.ts", None, FileChangeStatus::Modified, old, &new);
    let mut ub = UnitBuilder::new(path("src/n.ts"), 8);
    let outer = ub.symbol(SymbolKind::Function, "outer", None, 1, 7);
    let inner = ub.symbol(SymbolKind::Function, "inner", Some(outer), 3, 5);
    let unit = ub.build();
    let map = map_one(&file, Some(&unit), Some(&unit));
    assert_eq!(map.hits.len(), 1);
    assert_eq!(map.hits[0].symbol_id.as_str(), id(&unit, inner));
}

#[test]
fn unparsable_file_goes_to_unmapped() {
    let new = SVC.replace("return 1;", "return 3;");
    let file = file_diff("src/svc.ts", None, FileChangeStatus::Modified, SVC, &new);
    let (unit, ..) = svc_unit("src/svc.ts");
    let mut failed = UnitBuilder::new(path("src/svc.ts"), 13);
    failed.failed();
    let failed = failed.build();
    let map = map_one(&file, Some(&failed), Some(&unit));
    assert!(map
        .unmapped
        .iter()
        .any(|u| u.side == DiffSide::Head && u.reason == UnmappedReason::ParseFailed));
    // Without any unit the file is unmapped, never dropped.
    let map = map_one(&file, None, None);
    assert!(map.hits.is_empty());
    assert_eq!(map.unmapped.len(), 2);
    // A non-analysed file is unmapped with its disposition.
    let mut generated = file.clone();
    generated.disposition = FileDisposition::Vendored;
    let map = map_one(&generated, Some(&unit), Some(&unit));
    assert!(map.unmapped.iter().all(|u| matches!(
        u.reason,
        UnmappedReason::Disposition {
            disposition: FileDisposition::Vendored
        }
    )));
}

#[test]
fn ranges_beyond_eof_clamped() {
    let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
    let new = old.replace("h\n", "H\n");
    let file = file_diff("src/e.ts", None, FileChangeStatus::Modified, old, &new);
    // The unit believes the file has 5 lines.
    let mut ub = UnitBuilder::new(path("src/e.ts"), 5);
    ub.symbol(SymbolKind::Function, "f", None, 1, 3);
    let unit = ub.build();
    let map = map_one(&file, Some(&unit), Some(&unit));
    assert!(map.hits.is_empty());
    assert_eq!(
        map.unmapped
            .iter()
            .map(|u| (u.side, u.range.clone(), u.reason))
            .collect::<Vec<_>>(),
        vec![
            (DiffSide::Base, 8..9, UnmappedReason::BeyondEof),
            (DiffSide::Head, 8..9, UnmappedReason::BeyondEof),
        ]
    );
}

fn analyze(p: &str, src: &str) -> ParsedUnit {
    let repo_path = path(p);
    let input = SourceInput {
        module_path: ModulePath::of(&repo_path),
        content_hash: ContentHash::of(src.as_bytes()),
        path: repo_path,
        bytes: src.as_bytes(),
        is_generated: false,
    };
    TypeScriptAnalyzer::new()
        .analyze(&input, &AnalyzerConfig::default())
        .unwrap()
}

#[test]
#[allow(non_snake_case)]
fn golden_authorize_maps_to_AuthService_authorize() {
    let scenario = Scenario::load("auth-bypass").unwrap();
    let git = scenario.repo.open().unwrap();
    let diff = diff_commits(
        &git,
        &scenario.base,
        &scenario.head,
        &DiffOptions::default(),
    )
    .unwrap();
    let mut units = UnitMap::default();
    for p in ["src/auth/auth.service.ts", "src/util/format.ts"] {
        units.insert(DiffSide::Base, analyze(p, &scenario.base_files[p]));
        units.insert(DiffSide::Head, analyze(p, &scenario.head_files[p]));
    }
    let map = map_hunks(&diff, &units, &MapConfig::default());
    let authorize = map
        .hit("ts:src/auth/auth.service#AuthService.authorize/method")
        .unwrap_or_else(|| panic!("no authorize hit in {:?}", map.hits));
    assert_eq!(authorize.side, DiffSide::Head);
    assert_eq!(authorize.scope, HitScope::Body);
    assert!(authorize.touches_code);
    assert_eq!(authorize.ranges, vec![10..11]);
    assert_eq!(authorize.ranges_old, vec![10..11]);
    assert_eq!(map.hits_in("src/auth/auth.service.ts").count(), 1);

    let format: Vec<_> = map.hits_in("src/util/format.ts").collect();
    assert_eq!(format.len(), 1, "{format:?}");
    assert!(format[0].symbol_id.as_str().contains("formatName"));
    assert!(!format[0].touches_code);
    assert!(map.unmapped.is_empty(), "{:?}", map.unmapped);
}

fn lines_of(ranges: &[Range<u32>]) -> Vec<u32> {
    ranges.iter().flat_map(|r| r.clone()).collect()
}

fn random_unit(p: &str, line_count: u32, spans: &[(u32, u32)]) -> ParsedUnit {
    let mut ub = UnitBuilder::new(path(p), line_count);
    for (i, (start, len)) in spans.iter().enumerate() {
        let start = 1 + start % line_count.max(1);
        let end = (start + len).min(line_count.max(1));
        ub.symbol(SymbolKind::Function, &format!("s{i}"), None, start, end);
    }
    ub.build()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn every_changed_line_mapped_exactly_once(
        old_lines in prop::collection::vec(prop::sample::select(vec!["a", "b", "c", "{", "}", ""]), 1..30),
        new_lines in prop::collection::vec(prop::sample::select(vec!["a", "b", "c", "{", "}", ""]), 1..30),
        base_spans in prop::collection::vec((0u32..30, 0u32..10), 0..6),
        head_spans in prop::collection::vec((0u32..30, 0u32..10), 0..6),
        short in 0u32..3,
    ) {
        let old = format!("{}\n", old_lines.join("\n"));
        let new = format!("{}\n", new_lines.join("\n"));
        let file = file_diff("src/p.ts", None, FileChangeStatus::Modified, &old, &new);
        // Units may under-count lines so beyond-EOF ranges are exercised too.
        let base = random_unit("src/p.ts", (old_lines.len() as u32).saturating_sub(short), &base_spans);
        let head = random_unit("src/p.ts", (new_lines.len() as u32).saturating_sub(short), &head_spans);
        let map = map_file(&file, Some(&head), Some(&base), &MapConfig::default());

        let (changed_old, changed_new) = diff_engine::hunks::changed_ranges(&file.hunks);
        let mut seen_new: Vec<u32> = Vec::new();
        let mut seen_old: Vec<u32> = Vec::new();
        for hit in &map.hits {
            match hit.side {
                DiffSide::Head => {
                    seen_new.extend(lines_of(&hit.ranges));
                    seen_old.extend(lines_of(&hit.ranges_old));
                }
                DiffSide::Base => seen_old.extend(lines_of(&hit.ranges)),
            }
        }
        for u in &map.unmapped {
            match u.side {
                DiffSide::Head => seen_new.extend(u.range.clone()),
                DiffSide::Base => seen_old.extend(u.range.clone()),
            }
        }
        seen_new.sort_unstable();
        seen_old.sort_unstable();
        prop_assert_eq!(seen_new, lines_of(&changed_new));
        prop_assert_eq!(seen_old, lines_of(&changed_old));
    }
}

//! SID-004: the per-file symbol diff over real analyzed TypeScript.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use analysis_ir::{ParseStatus, ParsedUnit};
use incremental::symbol_diff::{
    diff_units, DiffCounts, ModifiedFlags, SymbolChangeKind, UnitStatus, UnknownReason,
};
use review_core::ids::SymbolId;
use review_core::language::{Dialect, Language};
use review_core::location::ContentHash;
use review_core::symbol::{ModulePath, SymbolKind};
use support::analyze;
use support::{id_of, symbol_of};

const SERVICE_BASE: &str = "\
import { Injectable } from '@nestjs/common';

@Injectable()
export class UsersService {
  private readonly store = new Map<string, string>();

  findByEmail(email: string): string | undefined {
    const normalized = email.trim().toLowerCase();
    return this.store.get(normalized);
  }

  async save(email: string, value: string): Promise<void> {
    this.store.set(email.trim().toLowerCase(), value);
  }
}

export function unrelatedHelper(input: string): number {
  return input.length;
}
";

fn base_unit() -> ParsedUnit {
    analyze("src/users/users.service.ts", SERVICE_BASE)
}

fn head_of(body: &str) -> ParsedUnit {
    analyze("src/users/users.service.ts", body)
}

fn change_of<'a>(
    diff: &'a incremental::symbol_diff::FileSymbolDiff,
    name: &str,
) -> &'a incremental::symbol_diff::SymbolChange {
    let qn = format!("UsersService.{name}");
    diff.changes
        .iter()
        .find(|change| {
            change
                .id
                .parts()
                .map(|parts| parts.qualified_name.join(".") == qn)
                .unwrap_or(false)
        })
        .unwrap_or_else(|| {
            panic!(
                "no change for {qn}: {:?}",
                diff.changes
                    .iter()
                    .map(|change| change.id.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

#[test]
fn identical_units_all_unchanged() {
    let diff = diff_units(Some(&base_unit()), Some(&head_of(SERVICE_BASE)));
    assert!(diff.is_known());
    assert_eq!(
        diff.counts,
        DiffCounts {
            unchanged: u32::try_from(base_unit().symbols.len()).unwrap_or(0),
            ..DiffCounts::default()
        }
    );
    assert!(diff
        .changes
        .iter()
        .all(|change| change.kind == SymbolChangeKind::Unchanged));
    assert!(diff.changes.iter().all(|change| !change.moved_range));
    assert_eq!(diff.base_status, Some(UnitStatus::Ok));
    assert_eq!(diff.head_status, Some(UnitStatus::Ok));
}

#[test]
fn reformat_is_unchanged_with_moved_range() {
    let reformatted = SERVICE_BASE
        .replace("{\n", "  {\n    ")
        .replace("\n  }", "\n  }\n");
    let head = head_of(&reformatted);
    let diff = diff_units(Some(&base_unit()), Some(&head));
    assert_eq!(diff.counts.modified_body, 0);
    let moved: Vec<&str> = diff
        .changes
        .iter()
        .filter(|change| change.moved_range)
        .map(|change| change.id.as_str())
        .collect();
    assert!(
        !moved.is_empty(),
        "a reformat moves ranges without changing hashes: {:?}",
        diff.changes
            .iter()
            .map(|change| (change.id.as_str(), change.moved_range))
            .collect::<Vec<_>>()
    );
}

#[test]
fn comment_only_change_is_unchanged() {
    let with_comment = format!("// leading comment\n{SERVICE_BASE}");
    let diff = diff_units(Some(&base_unit()), Some(&head_of(&with_comment)));
    assert_eq!(diff.counts.modified_body, 0);
    assert_eq!(diff.counts.modified_signature, 0);
    assert_eq!(diff.counts.modified_attributes, 0);
    assert_eq!(
        diff.counts.unchanged,
        u32::try_from(base_unit().symbols.len()).unwrap_or(0)
    );
}

#[test]
fn signature_only_change() {
    let head = head_of(&SERVICE_BASE.replace(
        "findByEmail(email: string): string | undefined {",
        "findByEmail(email: string, includeDeleted = false): string | undefined {",
    ));
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let change = change_of(&diff, "findByEmail");
    assert_eq!(change.kind, SymbolChangeKind::Modified);
    assert!(change.flags.contains(ModifiedFlags::SIGNATURE));
    assert!(
        !change.flags.contains(ModifiedFlags::BODY),
        "a parameter addition is not a body change: {:?}",
        change.flags.names()
    );
    assert_eq!(diff.counts.modified_signature, 1);
    assert!(!diff.has_uncertain());
}

#[test]
fn body_only_change() {
    let head = head_of(&SERVICE_BASE.replace(
        "const normalized = email.trim().toLowerCase();",
        "const normalized = email.trim().toLowerCase() + '';",
    ));
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let change = change_of(&diff, "findByEmail");
    assert_eq!(change.kind, SymbolChangeKind::Modified);
    assert!(change.flags.contains(ModifiedFlags::BODY));
    assert!(!change.flags.contains(ModifiedFlags::SIGNATURE));
    assert!(!change.flags.contains(ModifiedFlags::ATTRIBUTES));
    assert_eq!(diff.counts.modified_body, 1);
}

#[test]
fn attribute_only_change_decorator() {
    let decorated = SERVICE_BASE.replace("@Injectable()", "@Injectable({ scope: 'singleton' })");
    let head = head_of(&decorated);
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let id = id_of(&head, "UsersService", SymbolKind::Class);
    let change = diff
        .changes
        .iter()
        .find(|change| change.id == id)
        .expect("the class is in the diff");
    assert_eq!(change.kind, SymbolChangeKind::Modified);
    assert!(change.flags.contains(ModifiedFlags::ATTRIBUTES));
    assert!(
        !change.flags.contains(ModifiedFlags::BODY),
        "a decorator is not a body change"
    );
    assert_eq!(diff.counts.modified_attributes, 1);
}

#[test]
fn combined_flags() {
    let head = head_of(
        &SERVICE_BASE
            .replace(
                "findByEmail(email: string): string | undefined {",
                "findByEmail(email: string, exact = true): string | undefined {",
            )
            .replace(
                "const normalized = email.trim().toLowerCase();",
                "const normalized = email;",
            ),
    );
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let change = change_of(&diff, "findByEmail");
    assert_eq!(change.kind, SymbolChangeKind::Modified);
    assert!(change.flags.contains(ModifiedFlags::SIGNATURE));
    assert!(change.flags.contains(ModifiedFlags::BODY));
    assert_eq!(change.flags.names(), vec!["signature", "body"]);
}

#[test]
fn added_symbol() {
    let head = head_of(&format!(
        "{SERVICE_BASE}\nexport function brandNew(input: string): string {{\n  return input.trim();\n}}\n"
    ));
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let added = diff.added();
    assert_eq!(added.len(), 1);
    assert!(added[0].as_str().ends_with("#brandNew/function"));
    assert_eq!(diff.counts.added, 1);
    assert_eq!(diff.removed().len(), 0);
}

#[test]
fn removed_symbol() {
    let without_helper = SERVICE_BASE.replace(
        "\nexport function unrelatedHelper(input: string): number {\n  return input.length;\n}\n",
        "",
    );
    let diff = diff_units(Some(&base_unit()), Some(&head_of(&without_helper)));
    let removed = diff.removed();
    assert_eq!(removed.len(), 1);
    assert!(removed[0].as_str().ends_with("#unrelatedHelper/function"));
    assert_eq!(diff.counts.removed, 1);
}

#[test]
fn new_file_all_added() {
    let head = analyze(
        "src/users/orders.service.ts",
        "export class OrdersService {\n  total(): number {\n    return 1 + 2;\n  }\n}\n",
    );
    let diff = diff_units(None, Some(&head));
    assert!(diff.is_known());
    assert_eq!(
        diff.counts.added,
        u32::try_from(head.symbols.len()).unwrap_or(0)
    );
    assert_eq!(diff.counts.removed, 0);
    assert_eq!(diff.base_status, None);
    assert_eq!(diff.head_status, Some(UnitStatus::Ok));
    assert!(diff.removed().is_empty());
}

#[test]
fn deleted_file_all_removed() {
    let diff = diff_units(Some(&base_unit()), None);
    assert!(diff.is_known());
    assert_eq!(
        diff.counts.removed,
        u32::try_from(base_unit().symbols.len()).unwrap_or(0)
    );
    assert_eq!(diff.counts.added, 0);
    assert!(diff.added().is_empty());
}

#[test]
fn member_added_marks_class_body_modified() {
    // The extra method goes inside the class, so the container hash must change.
    let head = head_of(&SERVICE_BASE.replace(
        "  async save(",
        "  extra(value: number): number {\n    return value * 2;\n  }\n\n  async save(",
    ));
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let class_id = id_of(&base_unit(), "UsersService", SymbolKind::Class);
    let class_change = diff
        .changes
        .iter()
        .find(|change| change.id == class_id)
        .expect("the class is in the diff");
    assert_eq!(
        class_change.kind,
        SymbolChangeKind::Modified,
        "adding a member changes the container hash"
    );
    assert!(class_change.flags.contains(ModifiedFlags::BODY));
    let added = diff.added();
    assert_eq!(added.len(), 1);
    assert!(added[0].as_str().ends_with("UsersService.extra/method"));
}

#[test]
fn method_edit_does_not_modify_class() {
    let head = head_of(&SERVICE_BASE.replace(
        "this.store.set(email.trim().toLowerCase(), value);",
        "this.store.set(email.trim().toLowerCase(), value.trim());",
    ));
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let class_id = id_of(&base_unit(), "UsersService", SymbolKind::Class);
    let class_change = diff
        .changes
        .iter()
        .find(|change| change.id == class_id)
        .expect("the class is in the diff");
    assert_eq!(
        class_change.kind,
        SymbolChangeKind::Unchanged,
        "editing a method body must not touch the container hash"
    );
    assert_eq!(change_of(&diff, "save").kind, SymbolChangeKind::Modified);
}

#[test]
fn export_flag_change_is_attribute() {
    let without_export = SERVICE_BASE.replace("export class UsersService", "class UsersService");
    let head = head_of(&without_export);
    let diff = diff_units(Some(&base_unit()), Some(&head));
    let class_id = id_of(&base_unit(), "UsersService", SymbolKind::Class);
    let class_change = diff
        .changes
        .iter()
        .find(|change| change.id == class_id)
        .expect("the class is in the diff");
    assert_eq!(class_change.kind, SymbolChangeKind::Modified);
    assert!(class_change.flags.contains(ModifiedFlags::ATTRIBUTES));
    assert!(!class_change.flags.contains(ModifiedFlags::BODY));
}

#[test]
fn failed_parse_yields_unknown() {
    let mut failed = base_unit();
    failed.status = ParseStatus::Failed {
        reason: analysis_ir::FailReason::Timeout,
    };
    let diff = diff_units(Some(&failed), Some(&head_of(SERVICE_BASE)));
    assert!(!diff.is_known());
    assert_eq!(
        diff.unknown,
        Some(UnknownReason::ParseFailed {
            side: review_core::location::DiffSide::Base,
            reason: analysis_ir::FailReason::Timeout
        })
    );
    assert!(diff.changes.is_empty());
    assert_eq!(diff.counts, DiffCounts::default());
    assert!(diff.added().is_empty());
    assert!(diff.removed().is_empty());

    let head_failed = diff_units(Some(&base_unit()), Some(&failed));
    assert_eq!(
        head_failed.unknown,
        Some(UnknownReason::ParseFailed {
            side: review_core::location::DiffSide::Head,
            reason: analysis_ir::FailReason::Timeout
        })
    );
    assert!(!head_failed.is_known());
}

#[test]
fn partial_parse_marks_uncertain() {
    let broken = "export class Broken {\n  run(): void {\n    const = ;\n  }\n}\n";
    let head = head_of(broken);
    assert!(matches!(head.status, ParseStatus::Partial { .. }));
    let diff = diff_units(
        Some(&head_of(
            "export class Broken {\n  run(): void {\n    const value = 1;\n  }\n}\n",
        )),
        Some(&head),
    );
    assert!(diff.is_known(), "a partial parse still diffs");
    assert!(diff.has_uncertain(), "affected symbols are uncertain");
    let head_status = diff.head_status.expect("head status");
    assert!(matches!(head_status, UnitStatus::Partial { .. }));
}

use proptest::prelude::*;

proptest! {
    #[test]
    fn diff_is_a_mirror_image(
        edits in proptest::collection::vec(0usize..6, 1..6),
    ) {
        let base = base_unit();
        let head = apply_edits(&base, &edits);
        let forward = diff_units(Some(&base), Some(&head));
        let backward = diff_units(Some(&head), Some(&base));
        prop_assert_eq!(forward.counts.added, backward.counts.removed);
        prop_assert_eq!(forward.counts.removed, backward.counts.added);
        prop_assert_eq!(forward.counts.modified_body, backward.counts.modified_body);
        prop_assert_eq!(forward.counts.modified_signature, backward.counts.modified_signature);
        prop_assert_eq!(forward.counts.unchanged, backward.counts.unchanged);
        prop_assert_eq!(forward.counts.total(), backward.counts.total());
        prop_assert_eq!(forward.changes.len(), backward.changes.len());
    }
}

#[test]
fn diff_of_a_unit_with_itself_is_all_unchanged() {
    let unit = base_unit();
    let diff = diff_units(Some(&unit), Some(&unit));
    assert!(diff
        .changes
        .iter()
        .all(|change| change.kind == SymbolChangeKind::Unchanged));
    assert!(!diff.has_uncertain());
    assert_eq!(diff.unchanged().len(), unit.symbols.len());
}

#[test]
fn ordinal_collision_ids_diffed_independently() {
    let source = "export function duplicate(value: number): number {\n  return value + 1;\n}\n\nexport function duplicate(value: string): string {\n  return value.trim();\n}\n";
    let base = analyze("src/dup.ts", source);
    let head = analyze(
        "src/dup.ts",
        "export function duplicate(value: number): number {\n  return value + 2;\n}\n\nexport function duplicate(value: string): string {\n  return value.trim();\n}\n",
    );
    let ids: Vec<SymbolId> = base
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Function)
        .filter_map(|symbol| analysis_ir::identity::symbol_id_of(&base, symbol.local_id.0))
        .collect();
    assert_eq!(ids.len(), 2, "the colliding functions have distinct ids");
    assert!(ids[0].as_str().ends_with("duplicate/function"));
    assert!(ids[1].as_str().ends_with("duplicate/function~1"));
    let diff = diff_units(Some(&base), Some(&head));
    let modified = diff
        .changes
        .iter()
        .filter(|change| change.kind == SymbolChangeKind::Modified)
        .count();
    assert_eq!(modified, 1, "only the edited overload changes");
    assert_eq!(diff.counts.modified_body, 1);
}

#[test]
fn output_order_deterministic() {
    let base = base_unit();
    let head = head_of(&SERVICE_BASE.replace("return input.length;", "return input.length + 1;"));
    let first = diff_units(Some(&base), Some(&head));
    let second = diff_units(Some(&base), Some(&head));
    let render =
        |diff: &incremental::symbol_diff::FileSymbolDiff| -> Vec<(String, SymbolChangeKind)> {
            diff.changes
                .iter()
                .map(|change| (change.id.as_str().to_owned(), change.kind))
                .collect()
        };
    assert_eq!(
        render(&first),
        render(&second),
        "two runs must agree exactly"
    );

    // Sorted by (kind, id): unchanged, modified, added, removed.
    let rank = |kind: SymbolChangeKind| match kind {
        SymbolChangeKind::Unchanged => 0_u8,
        SymbolChangeKind::Modified => 1,
        SymbolChangeKind::Added => 2,
        SymbolChangeKind::Removed => 3,
    };
    let keys: Vec<(u8, String)> = first
        .changes
        .iter()
        .map(|change| (rank(change.kind), change.id.as_str().to_owned()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "the diff is sorted by (kind, id)");
}

/// Applies a list of edits to the body of one symbol, cycling through the file's callables.
fn apply_edits(unit: &ParsedUnit, edits: &[usize]) -> ParsedUnit {
    let mut source = String::new();
    let mut names: Vec<&str> = unit
        .symbols
        .iter()
        .filter(|symbol| symbol.body_range.is_some())
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.dedup();
    for (index, name) in names.iter().enumerate() {
        let marker = if edits.get(index).copied().unwrap_or(0) % 2 == 1 {
            "true"
        } else {
            "false"
        };
        source.push_str(&format!(
            "export function {name}(): boolean {{\n  return {marker};\n}}\n"
        ));
    }
    analyze("src/users/users.service.ts", &source)
}

#[test]
fn unit_status_and_paths_are_reported() {
    let base = base_unit();
    assert_eq!(base.module_path, ModulePath::of(&base.file));
    assert_eq!(base.language, Language::Typescript);
    assert_eq!(base.dialect, Some(Dialect::Ts));
    assert_eq!(base.content_hash, ContentHash::of(SERVICE_BASE.as_bytes()));
    let diff = diff_units(Some(&base), Some(&base));
    assert_eq!(diff.path, base.file);
    assert!(symbol_of(&base, "UsersService.findByEmail").body_token_count > 0);
}

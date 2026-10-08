//! SID-005: the rename/move matcher. Unit-level matching rules over hand-built symbols, because the
//! end-to-end behaviour on a scripted history is `rename_move.rs` (SID-006).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use analysis_ir::ShingleSet;
use incremental::lineage::LineageOutcome;
use incremental::matcher::{match_symbols, MatcherConfig, RenameHints};
use review_core::ids::{SymbolId, SymbolKey};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::matcher::SymbolRef;
use review_core::symbol::{Hash128, SymbolKind};
use review_core::symbol_id::{module_path_for, SymbolIdParts};
use support::analyze;

/// A symbol with an explicit body hash, token count and shingle sketch, so the rules can be driven
/// without writing source code for every case.
fn symbol(
    module: &str,
    name: &str,
    kind: SymbolKind,
    body: u8,
    tokens: u32,
    shingles: Vec<u32>,
    parent: Option<&SymbolRef>,
) -> SymbolRef {
    let path = RepoPath::new(format!("{module}.ts")).unwrap();
    let module_path = module_path_for(&path, Language::Typescript);
    let id = SymbolIdParts::new("ts", module_path.clone(), vec![name.to_owned()], kind)
        .format()
        .unwrap();
    SymbolRef {
        key: SymbolKey::of(&id),
        id,
        kind,
        name: name.to_owned(),
        qualified_name: vec![name.to_owned()],
        module_path,
        parent_id: parent.map(|p| p.id.clone()),
        signature_hash: Hash128::of("sig", &[kind.as_id_str().len() as u8]),
        body_hash: Hash128::of("body", &[body]),
        body_token_count: tokens,
        shingles: ShingleSet::of(shingles),
    }
}

fn sketch(bodies: &[u32]) -> Vec<u32> {
    bodies.to_vec()
}

fn match_pair(removed: &SymbolRef, added: &SymbolRef) -> incremental::matcher::MatchResult {
    match_symbols(
        std::slice::from_ref(removed),
        std::slice::from_ref(added),
        &MatcherConfig::default(),
        &RenameHints::default(),
    )
}

#[test]
fn exact_body_rename_method() {
    let removed = symbol(
        "src/a",
        "A.m",
        SymbolKind::Method,
        1,
        20,
        sketch(&[1, 2, 3]),
        None,
    );
    let added = symbol(
        "src/a",
        "A.find",
        SymbolKind::Method,
        1,
        20,
        sketch(&[1, 2, 3]),
        None,
    );
    let result = match_pair(&removed, &added);
    assert_eq!(result.matches.len(), 1);
    let record = &result.matches[0];
    assert_eq!(record.rule, review_core::matcher::MatchRule::ExactBody);
    assert_eq!(record.similarity, 1.0);
    assert_eq!(
        record.transition,
        review_core::matcher::SymbolTransition::Renamed
    );
    assert_eq!(record.from, removed.key);
    assert_eq!(record.to, added.key);
    assert!(result.unmatched_added.is_empty());
    assert!(result.unmatched_removed.is_empty());
}

#[test]
fn exact_body_move_function_to_other_file() {
    let removed = symbol(
        "src/util/strings",
        "formatMoney",
        SymbolKind::Function,
        4,
        30,
        sketch(&[4, 5, 6]),
        None,
    );
    let added = symbol(
        "src/common/strings",
        "formatMoney",
        SymbolKind::Function,
        4,
        30,
        sketch(&[4, 5, 6]),
        None,
    );
    let result = match_pair(&removed, &added);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(
        result.matches[0].rule,
        review_core::matcher::MatchRule::ExactBody
    );
    assert_eq!(
        result.matches[0].transition,
        review_core::matcher::SymbolTransition::Moved
    );
    assert!(result.matches[0]
        .from_id
        .as_str()
        .starts_with("ts:src/util/strings#"));
    assert!(result.matches[0]
        .to_id
        .as_str()
        .starts_with("ts:src/common/strings#"));
}

#[test]
fn signature_and_name_move_with_body_edit() {
    let removed = symbol(
        "src/util/strings",
        "formatMoney",
        SymbolKind::Function,
        4,
        30,
        sketch(&[4, 5, 6]),
        None,
    );
    let added = symbol(
        "src/common/strings",
        "formatMoney",
        SymbolKind::Function,
        9,
        30,
        sketch(&[4, 5, 7]),
        None,
    );
    let result = match_pair(&removed, &added);
    assert_eq!(result.matches.len(), 1);
    let record = &result.matches[0];
    assert_eq!(
        record.rule,
        review_core::matcher::MatchRule::SignatureAndName
    );
    assert!(record.similarity >= MatcherConfig::default().jaccard_min);
    assert_eq!(
        record.transition,
        review_core::matcher::SymbolTransition::Moved
    );
}

#[test]
fn token_similarity_rename_with_small_edit() {
    // A member may only reach a fuzzy rule inside a container that paired first, and a container's
    // folded body names its members (TSA-007), so renaming a member also changes the container's
    // hash. The rename therefore sits on the class, whose member list is untouched: the container
    // pairs on its folded hash and the method then pairs by token similarity inside it.
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let base = analyze(
        "src/a.ts",
        "export class A {\n  m(items: number[]): number {\n    const total = items.reduce((sum, item) => sum + item.price, 0);\n    return total;\n  }\n}\n",
    );
    let head = analyze(
        "src/a.ts",
        "export class B {\n  m(items: number[]): number {\n    const total = items.reduce((sum, item) => sum + item.price, 0);\n    return total + 0;\n  }\n}\n",
    );
    let (removed, added) = incremental::matcher::pools([&base], [&head]);
    let result = match_symbols(&removed, &added, &cfg, &hints);
    let member = result
        .matches
        .iter()
        .find(|record| record.to_id.as_str().contains("B.m"))
        .unwrap_or_else(|| {
            panic!(
                "the edited method pairs inside its renamed container: {:?}",
                result
                    .matches
                    .iter()
                    .map(|record| (record.from_id.as_str(), record.to_id.as_str(), record.rule))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        member.rule,
        review_core::matcher::MatchRule::TokenSimilarity
    );
    assert!(
        member.similarity >= 0.8 && member.similarity < 1.0,
        "{}",
        member.similarity
    );
    assert_eq!(
        member.transition,
        review_core::matcher::SymbolTransition::Renamed
    );
    assert!(
        result
            .matches
            .iter()
            .any(|record| record.to_id.as_str().ends_with("B/class")
                && record.rule == review_core::matcher::MatchRule::ExactBody),
        "the container pairs first, otherwise the member could not pair at all: {:?}",
        result
            .matches
            .iter()
            .map(|record| record.to_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn below_threshold_not_matched() {
    let removed = symbol(
        "src/a",
        "A.m",
        SymbolKind::Method,
        1,
        40,
        sketch(&(1..=40).collect::<Vec<u32>>()),
        None,
    );
    let added = symbol(
        "src/a",
        "A.run",
        SymbolKind::Method,
        2,
        40,
        sketch(&(100..=140).collect::<Vec<u32>>()),
        None,
    );
    let result = match_pair(&removed, &added);
    assert!(result.matches.is_empty(), "unrelated bodies must not pair");
    assert_eq!(result.unmatched_added, vec![added.key]);
    assert_eq!(result.unmatched_removed, vec![removed.key]);
}

#[test]
fn kind_mismatch_never_matches() {
    let removed = symbol(
        "src/a",
        "handler",
        SymbolKind::Function,
        1,
        30,
        sketch(&[1, 2, 3]),
        None,
    );
    let added = symbol(
        "src/a",
        "handler",
        SymbolKind::Method,
        1,
        30,
        sketch(&[1, 2, 3]),
        None,
    );
    assert!(match_pair(&removed, &added).matches.is_empty());
}

#[test]
fn tiny_bodies_not_matched_by_exact_rule() {
    let removed = symbol(
        "src/a",
        "getId",
        SymbolKind::Method,
        1,
        4,
        sketch(&[1, 2]),
        None,
    );
    let added = symbol(
        "src/a",
        "getName",
        SymbolKind::Method,
        1,
        4,
        sketch(&[1, 2]),
        None,
    );
    assert!(
        match_pair(&removed, &added).matches.is_empty(),
        "a four-token body is too generic to pair"
    );
}

#[test]
fn container_first_restricts_member_candidates() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    // Two identical classes, and one identical method in each: the members may only pair inside the
    // matched containers.
    let class_removed = symbol(
        "src/a",
        "A",
        SymbolKind::Class,
        10,
        50,
        sketch(&[10, 11]),
        None,
    );
    let class_added = symbol(
        "src/b",
        "B",
        SymbolKind::Class,
        10,
        50,
        sketch(&[10, 11]),
        None,
    );
    let method_removed_a = symbol(
        "src/a",
        "A.m",
        SymbolKind::Method,
        20,
        30,
        sketch(&[20, 21]),
        Some(&class_removed),
    );
    let method_removed_b = symbol(
        "src/c",
        "C.m",
        SymbolKind::Method,
        20,
        30,
        sketch(&[20, 21]),
        None,
    );
    let method_added_a = symbol(
        "src/b",
        "B.m",
        SymbolKind::Method,
        20,
        30,
        sketch(&[20, 21]),
        Some(&class_added),
    );
    let method_added_b = symbol(
        "src/d",
        "D.m",
        SymbolKind::Method,
        20,
        30,
        sketch(&[20, 21]),
        None,
    );
    let removed = vec![
        class_removed.clone(),
        method_removed_a.clone(),
        method_removed_b.clone(),
    ];
    let added = vec![
        class_added.clone(),
        method_added_a.clone(),
        method_added_b.clone(),
    ];
    let result = match_symbols(&removed, &added, &cfg, &hints);
    let matched_pairs: Vec<(SymbolId, SymbolId)> = result
        .matches
        .iter()
        .map(|record| (record.from_id.clone(), record.to_id.clone()))
        .collect();
    assert!(
        matched_pairs.contains(&(class_removed.id.clone(), class_added.id.clone())),
        "the containers pair first"
    );
    assert!(
        matched_pairs.contains(&(method_removed_a.id.clone(), method_added_a.id.clone())),
        "the member follows its matched parent"
    );
    // The orphan method has no matched container, so it may only pair by an identical body. It must
    // not take the member that belongs inside the matched container.
    assert!(
        !matched_pairs.contains(&(method_removed_b.id.clone(), method_added_a.id.clone())),
        "a member cannot cross into another class: {matched_pairs:?}"
    );
    // The two orphans are indistinguishable copies: the orphan method could be either added one and
    // nothing but the input order separates them, so neither is paired.
    assert!(
        !matched_pairs.contains(&(method_removed_b.id.clone(), method_added_b.id.clone())),
        "a copy-pasted method is never guessed: {matched_pairs:?}"
    );
    assert_eq!(
        result.ambiguous.len(),
        2,
        "both orphans are ambiguous copies: {matched_pairs:?}"
    );
    assert!(result.unmatched_removed.contains(&method_removed_b.key));
    assert!(result.unmatched_added.contains(&method_added_b.key));
}

#[test]
fn renamed_class_members_follow_via_body_hash() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let base = analyze(
        "src/orders.service.ts",
        "export class OrderService {\n  total(items: number[]): number {\n    return items.reduce((sum, item) => sum + item, 0);\n  }\n\n  async find(id: string): Promise<string> {\n    return id.trim();\n  }\n}\n",
    );
    let head = analyze(
        "src/orders.service.ts",
        "export class OrdersService {\n  total(items: number[]): number {\n    return items.reduce((sum, item) => sum + item, 0);\n  }\n\n  async find(id: string): Promise<string> {\n    return id.trim();\n  }\n}\n",
    );
    let (removed, added) = incremental::matcher::pools([&base], [&head]);
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert!(
        result.matches.len() >= 3,
        "the class and both members pair: {:?}",
        result
            .matches
            .iter()
            .map(|record| record.to_id.as_str())
            .collect::<Vec<_>>()
    );
    let class_match = result
        .matches
        .iter()
        .find(|record| record.to_id.as_str().ends_with("OrdersService/class"))
        .expect("the renamed class pairs");
    assert_eq!(class_match.rule, review_core::matcher::MatchRule::ExactBody);
    assert_eq!(
        class_match.transition,
        review_core::matcher::SymbolTransition::Renamed
    );
    let members: Vec<&review_core::matcher::LineageRecord> = result
        .matches
        .iter()
        .filter(|record| record.to_id.as_str().contains("OrdersService."))
        .collect();
    assert_eq!(members.len(), 2, "both members follow the class");
    assert!(
        members
            .iter()
            .all(|record| record.transition == review_core::matcher::SymbolTransition::Renamed),
        "a renamed class makes its members renamed too: {:?}",
        members
            .iter()
            .map(|record| record.transition)
            .collect::<Vec<_>>()
    );
    assert!(
        result.unmatched_added.is_empty(),
        "{:?}",
        result.unmatched_added
    );
}

#[test]
fn ambiguous_identical_candidates_left_unmatched() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let removed = vec![
        symbol(
            "src/a",
            "A.helper",
            SymbolKind::Function,
            7,
            40,
            sketch(&[7, 8, 9]),
            None,
        ),
        symbol(
            "src/b",
            "B.helper",
            SymbolKind::Function,
            7,
            40,
            sketch(&[7, 8, 9]),
            None,
        ),
    ];
    let added = vec![symbol(
        "src/c",
        "C.helper",
        SymbolKind::Function,
        7,
        40,
        sketch(&[7, 8, 9]),
        None,
    )];
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert!(
        result.matches.is_empty(),
        "two equally good candidates pair nothing"
    );
    assert_eq!(result.ambiguous.len(), 1);
    assert_eq!(result.ambiguous[0].candidates.len(), 2);
    assert_eq!(result.unmatched_added.len(), 1);
    assert_eq!(result.unmatched_removed.len(), 2);
    assert_eq!(
        result.ambiguous[0].reason,
        review_core::matcher::AmbiguityReason::EqualCandidates
    );
}

#[test]
fn tie_breakers_prefer_same_name_then_directory() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let removed = vec![
        symbol(
            "src/payments/a",
            "helper",
            SymbolKind::Function,
            5,
            30,
            sketch(&[5, 6]),
            None,
        ),
        symbol(
            "src/orders/b",
            "other",
            SymbolKind::Function,
            5,
            30,
            sketch(&[5, 6]),
            None,
        ),
    ];
    let added = vec![
        symbol(
            "src/payments/c",
            "helper",
            SymbolKind::Function,
            5,
            30,
            sketch(&[5, 6]),
            None,
        ),
        symbol(
            "src/orders/d",
            "other",
            SymbolKind::Function,
            5,
            30,
            sketch(&[5, 6]),
            None,
        ),
    ];
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert_eq!(result.matches.len(), 2);
    let pairs: Vec<(String, String)> = result
        .matches
        .iter()
        .map(|record| {
            (
                record.from_id.as_str().to_owned(),
                record.to_id.as_str().to_owned(),
            )
        })
        .collect();
    assert!(
        pairs
            .iter()
            .any(|(from, to)| from.starts_with("ts:src/payments/a#")
                && to.starts_with("ts:src/payments/c#")),
        "{pairs:?}"
    );
    assert!(
        pairs
            .iter()
            .any(|(from, to)| from.starts_with("ts:src/orders/b#")
                && to.starts_with("ts:src/orders/d#")),
        "{pairs:?}"
    );
}

#[test]
fn rule_order_exact_before_fuzzy() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    // One removed symbol with two plausible added symbols: the exact body wins over the fuzzy one.
    let removed = vec![symbol(
        "src/a",
        "A.m",
        SymbolKind::Method,
        1,
        40,
        sketch(&[1, 2, 3, 4]),
        None,
    )];
    let exact = symbol(
        "src/a",
        "A.exact",
        SymbolKind::Method,
        1,
        40,
        sketch(&[1, 2, 3, 4]),
        None,
    );
    let fuzzy = symbol(
        "src/a",
        "A.fuzzy",
        SymbolKind::Method,
        2,
        40,
        sketch(&[1, 2, 3, 5]),
        None,
    );
    let result = match_symbols(&removed, &[exact.clone(), fuzzy], &cfg, &hints);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].to, exact.key);
    assert_eq!(
        result.matches[0].rule,
        review_core::matcher::MatchRule::ExactBody
    );
}

#[test]
fn file_rename_hint_only_prioritizes() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::from_pairs([("src/util/strings", "src/common/strings")]);
    let removed = vec![symbol(
        "src/util/strings",
        "formatMoney",
        SymbolKind::Function,
        5,
        30,
        sketch(&[5, 6]),
        None,
    )];
    let unrelated = symbol(
        "src/other/helpers",
        "formatMoney",
        SymbolKind::Function,
        5,
        30,
        sketch(&[5, 6]),
        None,
    );
    let hinted = symbol(
        "src/common/strings",
        "formatMoney",
        SymbolKind::Function,
        5,
        30,
        sketch(&[5, 6]),
        None,
    );
    // Two identical candidates: the hint only changes the order, so nothing is paired.
    let result = match_symbols(&removed, &[unrelated, hinted.clone()], &cfg, &hints);
    assert!(
        result.matches.is_empty(),
        "a hint never creates a match by itself"
    );
    assert_eq!(result.ambiguous.len(), 1);

    // With one candidate the hint is irrelevant to the outcome.
    let single = match_symbols(&removed, std::slice::from_ref(&hinted), &cfg, &hints);
    assert_eq!(single.matches.len(), 1);
    assert_eq!(single.matches[0].to, hinted.key);
}

use proptest::prelude::*;

proptest! {
    #[test]
    fn matching_is_independent_of_pool_order(
            bodies in proptest::collection::vec(0u8..8, 1..6),
            names in proptest::collection::vec("[a-z]{1,4}", 1..6),
        ) {
            let cfg = MatcherConfig::default();
            let hints = RenameHints::default();
            let mut removed: Vec<SymbolRef> = Vec::new();
            let mut added: Vec<SymbolRef> = Vec::new();
            for (index, name) in names.iter().enumerate() {
                let seed = bodies[index % bodies.len()];
                removed.push(symbol(&format!("src/base/f{index}"), name, SymbolKind::Function, seed, 40, sketch(&[u32::from(seed), 7]), None));
                let renamed = if index % 2 == 0 { format!("r{name}") } else { name.clone() };
                added.push(symbol(&format!("src/head/f{index}"), &renamed, SymbolKind::Function, seed, 40, sketch(&[u32::from(seed), 7]), None));
            }
            let mut reversed_removed = removed.clone();
            reversed_removed.reverse();
            let mut reversed_added = added.clone();
            reversed_added.reverse();
            let forward = match_symbols(&removed, &added, &cfg, &hints);
            let backward = match_symbols(&reversed_removed, &reversed_added, &cfg, &hints);
            prop_assert_eq!(forward.matches, backward.matches);
            prop_assert_eq!(forward.unmatched_added, backward.unmatched_added);
            prop_assert_eq!(forward.unmatched_removed, backward.unmatched_removed);
            prop_assert_eq!(forward.ambiguous, backward.ambiguous);
        }
}

#[test]
fn greedy_assignment_is_one_to_one() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for index in 0..8u8 {
        removed.push(symbol(
            &format!("src/base/f{index}"),
            &format!("fn{index}"),
            SymbolKind::Function,
            100 + index,
            40,
            sketch(&[100 + u32::from(index)]),
            None,
        ));
        added.push(symbol(
            &format!("src/head/f{index}"),
            &format!("renamed{index}"),
            SymbolKind::Function,
            100 + index,
            40,
            sketch(&[100 + u32::from(index)]),
            None,
        ));
    }
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert_eq!(result.matches.len(), 8);
    let mut froms: Vec<SymbolKey> = result.matches.iter().map(|record| record.from).collect();
    let mut tos: Vec<SymbolKey> = result.matches.iter().map(|record| record.to).collect();
    froms.sort();
    tos.sort();
    froms.dedup();
    tos.dedup();
    assert_eq!(froms.len(), 8, "each removed symbol pairs at most once");
    assert_eq!(tos.len(), 8, "each added symbol pairs at most once");
}

#[test]
fn transition_classification() {
    let renamed = symbol(
        "src/a",
        "x",
        SymbolKind::Function,
        1,
        30,
        sketch(&[1]),
        None,
    );
    let same = symbol(
        "src/a",
        "y",
        SymbolKind::Function,
        1,
        30,
        sketch(&[1]),
        None,
    );
    let moved = symbol(
        "src/b",
        "x",
        SymbolKind::Function,
        1,
        30,
        sketch(&[1]),
        None,
    );
    let both = symbol(
        "src/b",
        "y",
        SymbolKind::Function,
        1,
        30,
        sketch(&[1]),
        None,
    );
    assert_eq!(
        match_pair(&renamed, &same).matches[0].transition,
        review_core::matcher::SymbolTransition::Renamed
    );
    assert_eq!(
        match_pair(&renamed, &moved).matches[0].transition,
        review_core::matcher::SymbolTransition::Moved
    );
    assert_eq!(
        match_pair(&renamed, &both).matches[0].transition,
        review_core::matcher::SymbolTransition::RenamedAndMoved
    );
}

#[test]
fn degraded_mode_rule1_only() {
    let cfg = MatcherConfig {
        degrade_threshold: 1,
        ..MatcherConfig::default()
    };
    let hints = RenameHints::default();
    let removed = vec![
        symbol(
            "src/util/strings",
            "helper",
            SymbolKind::Function,
            1,
            30,
            sketch(&[1, 2]),
            None,
        ),
        symbol(
            "src/a",
            "A.m",
            SymbolKind::Method,
            1,
            30,
            sketch(&[1, 2]),
            None,
        ),
    ];
    let added = vec![
        symbol(
            "src/common/strings",
            "helper",
            SymbolKind::Function,
            9,
            30,
            sketch(&[1, 2]),
            None,
        ),
        symbol(
            "src/b",
            "B.m",
            SymbolKind::Method,
            1,
            30,
            sketch(&[1, 2]),
            None,
        ),
    ];
    let result = match_symbols(&removed, &added, &cfg, &hints);
    assert!(result.degraded);
    assert_eq!(
        result.matches.len(),
        1,
        "only the identical body pairs in degraded mode"
    );
    assert_eq!(
        result.matches[0].rule,
        review_core::matcher::MatchRule::ExactBody
    );
    assert_eq!(result.unmatched_added.len(), 1);
    assert_eq!(result.unmatched_removed.len(), 1);
}

#[test]
fn follow_chain_with_cycle_guard() {
    let base = analyze(
        "src/a.ts",
        "export function one(): number {\n  return 1 + 2 + 3;\n}\n",
    );
    let middle = analyze(
        "src/b.ts",
        "export function two(): number {\n  return 1 + 2 + 3;\n}\n",
    );
    let head = analyze(
        "src/c.ts",
        "export function three(): number {\n  return 1 + 2 + 3;\n}\n",
    );
    let (removed_a, added_a) = incremental::matcher::pools([&base], [&middle]);
    let (removed_b, added_b) = incremental::matcher::pools([&middle], [&head]);
    let first = match_symbols(
        &removed_a,
        &added_a,
        &MatcherConfig::default(),
        &RenameHints::default(),
    );
    let second = match_symbols(
        &removed_b,
        &added_b,
        &MatcherConfig::default(),
        &RenameHints::default(),
    );
    let outcome = LineageOutcome {
        records: first
            .matches
            .iter()
            .chain(second.matches.iter())
            .cloned()
            .collect(),
        unmatched_added: vec![],
        unmatched_removed: vec![],
        ambiguous: vec![],
        degraded: false,
    };
    let index = outcome.index();
    let one_id = SymbolId::parse("ts:src/a#one/function").unwrap();
    let head_id = SymbolId::parse("ts:src/c#three/function").unwrap();
    let head_key = SymbolKey::of(&head_id);
    let start = SymbolKey::of(&one_id);
    let end = incremental::lineage::follow(&index, start);
    assert_eq!(
        end.to_string(),
        head_key.to_string(),
        "the chain reaches the newest key"
    );
    let chain = incremental::lineage::chain(&index, start);
    assert_eq!(chain.len(), 3, "one -> two -> three");

    // A self-referential record must not loop.
    let mut cyclic = outcome.index();
    cyclic.push(review_core::matcher::LineageRecord {
        from: start,
        to: start,
        from_id: SymbolId::parse("ts:src/a#one/function").unwrap(),
        to_id: SymbolId::parse("ts:src/a#one/function").unwrap(),
        transition: review_core::matcher::SymbolTransition::Renamed,
        rule: review_core::matcher::MatchRule::ExactBody,
        similarity: 1.0,
        ambiguous: false,
    });
    assert!(incremental::lineage::chain(&cyclic, start).len() <= 3);
}

#[test]
fn shifted_ordinal_pairing() {
    // SID-003 documents that inserting a duplicate *before* an existing symbol shifts its ordinal:
    // the inserted declaration takes the plain id and the original moves to `~1`. SID-004 keys its
    // diff by SymbolId and leaves rename/move pairing to this matcher, so the shift reaches the
    // matcher as Modified(plain) plus Added(~1) with nothing removed - the two sides of the shifted
    // pair are presented here directly, which is the pairing SID-003 delegates to SID-005.
    let base = analyze(
        "src/dup.ts",
        "export function handler(value: string): string {\n  return value.trim();\n}\n",
    );
    let head = analyze(
        "src/dup.ts",
        "export function handler(value: number): number {\n  return value + 1;\n}\n\nexport function handler(value: string): string {\n  return value.trim();\n}\n",
    );
    assert!(
        base.symbols
            .iter()
            .any(|symbol| symbol.qualified_name == ["handler"] && symbol.ordinal == 0),
        "the base has the plain id"
    );
    assert!(
        head.symbols
            .iter()
            .any(|symbol| symbol.qualified_name == ["handler"] && symbol.ordinal == 1),
        "the head shifted the original to ~1"
    );
    let base_handlers = support::symbols_of(&base, "handler");
    let head_handlers = support::symbols_of(&head, "handler");
    assert_eq!(base_handlers.len(), 1, "the base declares it once");
    assert_eq!(head_handlers.len(), 2, "the head declares it twice");

    let (pooled_removed, _) = incremental::matcher::pools([&base], [&head]);
    assert!(
        pooled_removed.is_empty(),
        "an id-keyed diff reports the shift as Modified plus Added, so the matcher never sees the \
         pre-shift id: {:?}",
        pooled_removed
            .iter()
            .map(|symbol| symbol.id.as_str())
            .collect::<Vec<_>>()
    );

    let removed = vec![incremental::symbol_diff::symbol_ref(&base, base_handlers[0]).unwrap()];
    let added: Vec<review_core::matcher::SymbolRef> = head_handlers
        .iter()
        .map(|symbol| incremental::symbol_diff::symbol_ref(&head, symbol).unwrap())
        .collect();
    let result = match_symbols(
        &removed,
        &added,
        &MatcherConfig::default(),
        &RenameHints::default(),
    );
    assert_eq!(
        result.matches.len(),
        1,
        "the shifted symbol pairs by body: {:?}",
        result
            .matches
            .iter()
            .map(|record| (record.from_id.as_str(), record.to_id.as_str(), record.rule))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        result.matches[0].from_id.as_str(),
        "ts:src/dup#handler/function"
    );
    assert_eq!(
        result.matches[0].to_id.as_str(),
        "ts:src/dup#handler/function~1"
    );
    assert_eq!(
        result.matches[0].rule,
        review_core::matcher::MatchRule::ExactBody
    );
    assert_eq!(
        result.unmatched_added.len(),
        1,
        "the inserted duplicate is a new symbol, not the old one"
    );
    assert!(result.unmatched_added[0].to_string().len() == 32);
    assert_eq!(result.unmatched_removed.len(), 0);
}

#[test]
fn empty_pools_short_circuit() {
    let cfg = MatcherConfig::default();
    let hints = RenameHints::default();
    let result = match_symbols(&[], &[], &cfg, &hints);
    assert!(result.matches.is_empty());
    assert!(result.unmatched_added.is_empty());
    assert!(result.unmatched_removed.is_empty());
    let added = vec![symbol(
        "src/a",
        "A.m",
        SymbolKind::Method,
        1,
        30,
        sketch(&[1]),
        None,
    )];
    let one_sided = match_symbols(&[], &added, &cfg, &hints);
    assert_eq!(one_sided.unmatched_added, vec![added[0].key]);
    assert!(!one_sided.degraded);
}

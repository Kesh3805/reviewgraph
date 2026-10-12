#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use context_engine::candidates::lexical::LexicalCandidates;
use context_engine::candidates::{Candidate, CandidateGenerator, CandidatePool};
use context_engine::lexical::{
    LexicalDoc, LexicalIndex, LexicalIndexCache, CACHE_CAPACITY, DF_CUTOFF,
};
use context_engine::model::{CandidateSource, ItemKind};
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn doc(id: &str, idents: &[&str]) -> LexicalDoc {
    LexicalDoc {
        node: key(id),
        identifiers: idents.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn corpus() -> Vec<LexicalDoc> {
    let mut docs = vec![
        doc(AUTHORIZE, &["authorize", "AuthService.authorize"]),
        doc(UPDATE_USER, &["updateUser", "PERMISSION_SERVICE"]),
        doc(PROVIDER, &["authorize", "PERMISSION_SERVICE"]),
        doc(FORMAT_HEADER, &["authorizeHeader"]),
        doc("ts:src/events/producer.ts#emit", &["emit", "user.updated"]),
        doc(
            "ts:src/events/consumer.ts#onUpdated",
            &["onUpdated", "user.updated"],
        ),
    ];
    for i in 0..30 {
        let filler = format!("filler{i}");
        docs.push(doc(
            &format!("ts:src/misc/m{i}.ts#f{i}"),
            &[filler.as_str()],
        ));
    }
    docs
}

fn run(index: LexicalIndex, query: &[&str]) -> CandidatePool {
    let generator = LexicalCandidates::new(
        Arc::new(index),
        query.iter().map(|s| (*s).to_owned()).collect(),
    );
    let graph = graph_of(vec![]);
    let input = input(ReviewerType::Correctness, Some(&graph));
    let mut pool = CandidatePool::new();
    generator.generate(&input, &mut pool);
    pool
}

#[test]
fn di_string_token_links_provider() {
    let index = LexicalIndex::build(corpus()).unwrap();
    let pool = run(index, &["PERMISSION_SERVICE"]);
    assert!(pool.get_node(key(UPDATE_USER), DiffSide::Head).is_some());
    assert!(pool.get_node(key(PROVIDER), DiffSide::Head).is_some());
    let c = pool.get_node(key(PROVIDER), DiffSide::Head).unwrap();
    assert!(c.sources.contains(&CandidateSource::Lexical));
    assert_eq!(c.kind, ItemKind::RelatedSymbol);
    assert_eq!(c.distance, None);
}

#[test]
fn event_name_links_producer_and_consumer() {
    let index = LexicalIndex::build(corpus()).unwrap();
    let pool = run(index, &["user.updated"]);
    assert!(pool
        .get_node(key("ts:src/events/producer.ts#emit"), DiffSide::Head)
        .is_some());
    assert!(pool
        .get_node(key("ts:src/events/consumer.ts#onUpdated"), DiffSide::Head)
        .is_some());
}

#[test]
fn common_token_df_cutoff() {
    let docs: Vec<LexicalDoc> = (0..=DF_CUTOFF)
        .map(|i| doc(&format!("ts:a.ts#n{i}"), &["commonToken"]))
        .collect();
    let index = LexicalIndex::build(docs).unwrap();
    assert!(index.query(&["commonToken".to_owned()]).is_empty());
}

#[test]
fn camel_subtokens_lower_weight() {
    let index = LexicalIndex::build(vec![
        doc("ts:a.ts#exact", &["updateUserPermissions"]),
        doc("ts:a.ts#partial", &["permissions"]),
        doc("ts:a.ts#other", &["unrelatedThing"]),
    ])
    .unwrap();
    let hits = index.query(&["updateUserPermissions".to_owned()]);
    let score_of = |id: &str| {
        hits.iter()
            .find(|h| h.node == key(id))
            .map_or(0.0, |h| h.score)
    };
    assert!(score_of("ts:a.ts#exact") > 0.99);
    assert!(score_of("ts:a.ts#partial") < score_of("ts:a.ts#exact"));
}

#[test]
fn auth_bypass_lexical_does_not_promote_format_authorizeheader_above_threshold() {
    let index = LexicalIndex::build(corpus()).unwrap();
    let hits = index.query(&[
        "authorize".to_owned(),
        "PERMISSION_SERVICE".to_owned(),
        "updateUser".to_owned(),
        "PermissionService.check".to_owned(),
    ]);
    assert!(
        hits.iter().all(|h| h.node != key(FORMAT_HEADER)),
        "authorizeHeader must stay below the 0.25 threshold: {hits:?}"
    );
}

#[test]
fn already_structural_gets_source_added() {
    let index = Arc::new(LexicalIndex::build(corpus()).unwrap());
    let generator = LexicalCandidates::new(index, vec!["PERMISSION_SERVICE".to_owned()]);
    let graph = graph_of(vec![]);
    let input = input(ReviewerType::Correctness, Some(&graph));
    let mut pool = CandidatePool::new();
    let mut structural = Candidate::node(
        key(UPDATE_USER),
        "updateUser",
        ItemKind::RelatedSymbol,
        DiffSide::Head,
        CandidateSource::Structural,
    );
    structural.distance = Some(1);
    pool.insert(structural);
    generator.generate(&input, &mut pool);
    let c = pool.get_node(key(UPDATE_USER), DiffSide::Head).unwrap();
    assert!(c.sources.contains(&CandidateSource::Structural));
    assert!(c.sources.contains(&CandidateSource::Lexical));
    assert_eq!(c.distance, Some(1));
    assert!(c.raw.changed_code_similarity > 0.0);
}

#[test]
fn seeds_are_never_lexical_candidates() {
    let index = LexicalIndex::build(corpus()).unwrap();
    let pool = run(index, &["authorize"]);
    assert!(pool.get_node(key(AUTHORIZE), DiffSide::Head).is_none());
}

#[test]
fn index_cached_per_snapshot() {
    let cache = LexicalIndexCache::new(CACHE_CAPACITY);
    let mut builds = 0;
    for _ in 0..3 {
        cache
            .get_or_build("snap-1", || {
                builds += 1;
                LexicalIndex::build(corpus())
            })
            .unwrap();
    }
    assert_eq!(builds, 1);
    for i in 0..(CACHE_CAPACITY + 2) {
        cache
            .get_or_build(&format!("other-{i}"), || LexicalIndex::build(corpus()))
            .unwrap();
    }
    assert_eq!(cache.len(), CACHE_CAPACITY);
}

#[test]
fn secret_like_literals_are_not_indexed() {
    let index = LexicalIndex::build(vec![doc(
        "ts:a.ts#cfg",
        &["AbCdEf0123456789AbCdEf0123456789xyz", "realName"],
    )])
    .unwrap();
    assert_eq!(index.df("abcdef0123456789abcdef0123456789xyz"), 0);
    assert_eq!(index.df("realname"), 1);
}

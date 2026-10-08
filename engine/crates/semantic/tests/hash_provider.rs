#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-002: deterministic feature-hashing provider.

use semantic::embedding::hash::embed_text;
use semantic::embedding::{EmbedRequest, EmbeddingProvider, HashProvider, InputKind};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// blake3 of the little-endian f32 bytes of the golden texts' vectors (768 dims).
const GOLDEN: &str = "pending";

fn golden_digest() -> String {
    let mut hasher = blake3::Hasher::new();
    for text in [
        "async authorize(user, resource) { return this.permissionService.check(user.id) }",
        "class AuthService implements AuthProvider",
        "formatAuthorizeHeader(token: string): string",
    ] {
        for x in embed_text(text, 768) {
            hasher.update(&x.to_le_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

#[test]
fn hash_deterministic_bit_exact() {
    let a = embed_text("authorizeUser(user, resource)", 768);
    let b = embed_text("authorizeUser(user, resource)", 768);
    assert_eq!(
        a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
    );
    let digest = golden_digest();
    assert_eq!(digest, GOLDEN, "hash provider golden changed: {digest}");
}

#[test]
fn hash_similarity_camel_vs_snake() {
    let a = embed_text("authorizeUser", 768);
    let b = embed_text("authorize_user", 768);
    let s = cosine(&a, &b);
    assert!(s > 0.8, "cosine {s}");
}

#[test]
fn hash_unrelated_low_similarity() {
    let a = embed_text("authorizeUser", 768);
    let b = embed_text("parseJsonConfig", 768);
    let s = cosine(&a, &b);
    assert!(s < 0.2, "cosine {s}");
}

#[tokio::test]
async fn provider_returns_unit_vectors_in_order() {
    let p = HashProvider::default_space().unwrap();
    assert_eq!(p.space().id(), "hash-fh768-768");
    assert_eq!(p.space().collection_name(), "rg_hash_fh768_768_v1");
    let texts = vec!["alpha beta".to_owned(), "gammaDelta".to_owned()];
    let resp = p
        .embed(EmbedRequest::new(InputKind::Document, &texts))
        .await
        .unwrap();
    assert_eq!(resp.vectors[0], embed_text("alpha beta", 768));
    assert_eq!(resp.vectors[1], embed_text("gammaDelta", 768));
    let norm: f32 = resp.vectors[1].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
    assert!(HashProvider::new(100, 1).is_err());
}

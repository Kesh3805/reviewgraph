#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-001: space identity and provider wrappers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use semantic::embedding::{
    EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider, EmbeddingSpace, InputKind,
    Normalized, ProviderName, Redacting, RetryPolicy, RetryingProvider,
};

#[derive(Debug)]
struct Fake {
    space: EmbeddingSpace,
    /// Errors returned by the first calls, in order.
    errors: Mutex<Vec<EmbedError>>,
    vector: Vec<f32>,
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl Fake {
    fn new(dims: u16, vector: Vec<f32>, errors: Vec<EmbedError>) -> Self {
        Self {
            space: EmbeddingSpace::new(ProviderName::Hash, "fake", dims, 1).unwrap(),
            errors: Mutex::new(errors),
            vector,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for Fake {
    fn space(&self) -> &EmbeddingSpace {
        &self.space
    }

    fn max_batch(&self) -> usize {
        16
    }

    fn max_input_tokens(&self) -> usize {
        1000
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().extend(req.texts.iter().cloned());
        let mut errors = self.errors.lock().unwrap();
        if !errors.is_empty() {
            return Err(errors.remove(0));
        }
        Ok(EmbedResponse {
            vectors: req.texts.iter().map(|_| self.vector.clone()).collect(),
            usage_tokens: 1,
            latency_ms: 1,
        })
    }
}

fn texts(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("text {i}")).collect()
}

#[test]
fn collection_name_format() {
    let s = EmbeddingSpace::new(ProviderName::Voyage, "voyage-code-3", 1024, 2).unwrap();
    assert_eq!(s.collection_name(), "rg_voyage_voyage_code_3_1024_v2");
    // Same rendering as the review-core space recorded on artifacts (ADR-015).
    assert_eq!(s.collection_name(), s.core().unwrap().collection_name(2));
    assert!(EmbeddingSpace::new(ProviderName::Hash, "", 8, 1).is_err());
    assert!(EmbeddingSpace::new(ProviderName::Hash, "m", 0, 1).is_err());
}

#[test]
fn space_id_sanitizes_model_names() {
    let s = EmbeddingSpace::new(ProviderName::Openai, "text-embedding-3-small", 1536, 1).unwrap();
    assert_eq!(s.id(), "openai-text_embedding_3_small-1536");
    let weird = EmbeddingSpace::new(ProviderName::Openai, "../Model/X?y", 8, 1).unwrap();
    assert_eq!(weird.id(), "openai-___model_x_y-8");
    assert_eq!(weird.collection_name(), "rg_openai____model_x_y_8_v1");
}

#[tokio::test]
async fn normalized_wrapper_unit_length() {
    let p = Normalized::new(Fake::new(3, vec![3.0, 4.0, 0.0], vec![]));
    let t = texts(2);
    let resp = p
        .embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap();
    for v in &resp.vectors {
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "{norm}");
    }
    assert_eq!(resp.vectors[0], vec![0.6, 0.8, 0.0]);
}

#[tokio::test]
async fn dimension_mismatch_detected() {
    let p = Normalized::new(Fake::new(4, vec![1.0, 0.0], vec![]));
    let t = texts(1);
    let err = p
        .embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        EmbedError::DimensionMismatch {
            expected: 4,
            got: 2
        }
    );
}

#[tokio::test]
async fn retry_on_transient_then_success() {
    let fake = Arc::new(Fake::new(
        2,
        vec![1.0, 0.0],
        vec![
            EmbedError::Transient("reset".into()),
            EmbedError::RateLimited { retry_after_ms: 0 },
        ],
    ));
    let p = RetryingProvider::new(Arc::clone(&fake), RetryPolicy::immediate());
    let t = texts(1);
    let resp = p.embed(EmbedRequest::new(InputKind::Query, &t)).await;
    assert!(resp.is_ok());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 3);

    // Three retries at most.
    let always = Arc::new(Fake::new(
        2,
        vec![1.0, 0.0],
        (0..10).map(|_| EmbedError::Transient("x".into())).collect(),
    ));
    let p = RetryingProvider::new(Arc::clone(&always), RetryPolicy::immediate());
    assert!(p
        .embed(EmbedRequest::new(InputKind::Query, &t))
        .await
        .is_err());
    assert_eq!(always.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn no_retry_on_permanent() {
    for e in [
        EmbedError::Permanent("401".into()),
        EmbedError::InputTooLong { index: 0 },
    ] {
        let fake = Arc::new(Fake::new(2, vec![1.0, 0.0], vec![e.clone()]));
        let p = RetryingProvider::new(Arc::clone(&fake), RetryPolicy::immediate());
        let t = texts(1);
        assert_eq!(
            p.embed(EmbedRequest::new(InputKind::Query, &t))
                .await
                .unwrap_err(),
            e
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn backoff_honours_retry_after() {
    let policy = RetryPolicy::default();
    let wait = policy.backoff(
        1,
        &EmbedError::RateLimited {
            retry_after_ms: 5_000,
        },
    );
    assert!(wait >= std::time::Duration::from_secs(5));
    let wait = policy.backoff(10, &EmbedError::Transient("x".into()));
    assert!(wait <= std::time::Duration::from_secs(8));
}

#[tokio::test]
async fn redaction_applied_before_provider() {
    let fake = Arc::new(Fake::new(2, vec![1.0, 0.0], vec![]));
    let p = Redacting::new(Arc::clone(&fake));
    let t = vec![
        "const token = \"ghp_abcdefghijklmnopqrstuvwxyz0123\";".to_owned(),
        "plain text".to_owned(),
    ];
    p.embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap();
    let seen = fake.seen.lock().unwrap().clone();
    assert!(
        !seen[0].contains("ghp_abcdefghijklmnopqrstuvwxyz0123"),
        "{}",
        seen[0]
    );
    assert!(seen[0].contains("«redacted:"));
    assert_eq!(seen[1], "plain text");
}

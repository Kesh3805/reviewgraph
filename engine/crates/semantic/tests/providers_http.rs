#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-002: remote adapters against wiremock, provider selection.

use semantic::embedding::{
    build_provider, build_raw, EmbedError, EmbedRequest, EmbeddingConfig, EmbeddingProvider,
    InputKind, OpenAiProvider, Privacy, ProviderName, VoyageProvider,
};
use serde_json::{json, Value};
use telemetry::Secret;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn openai(server: &MockServer, dims: Option<u16>) -> OpenAiProvider {
    OpenAiProvider::new(Some(Secret::new("sk-test")), None, dims, 1, 2)
        .unwrap()
        .with_base_url(server.uri())
}

fn voyage(server: &MockServer) -> VoyageProvider {
    VoyageProvider::new(Some(Secret::new("pa-test")), None, Some(4), 1, 2)
        .unwrap()
        .with_base_url(server.uri())
}

fn texts(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn openai_request_shape_and_order_by_index() {
    let server = MockServer::start().await;
    // Data deliberately out of order: the adapter must order by `index`.
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"index": 1, "embedding": [0.0, 1.0, 0.0]},
                {"index": 0, "embedding": [1.0, 0.0, 0.0]}
            ],
            "usage": {"prompt_tokens": 7, "total_tokens": 7}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let p = openai(&server, Some(3));
    let t = texts(&["first", "second"]);
    let resp = p
        .embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap();
    assert_eq!(resp.vectors, vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]]);
    assert_eq!(resp.usage_tokens, 7);
    let reqs = server.received_requests().await.unwrap();
    let body: Value = reqs[0].body_json().unwrap();
    assert_eq!(
        body,
        json!({
            "model": "text-embedding-3-small",
            "input": ["first", "second"],
            "encoding_format": "float",
            "dimensions": 3
        })
    );
}

#[tokio::test]
async fn openai_splits_large_batches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|req: &wiremock::Request| {
            let body: Value = req.body_json().unwrap();
            let n = body["input"].as_array().unwrap().len();
            let data: Vec<Value> = (0..n)
                .map(|i| json!({"index": i, "embedding": [1.0, 0.0, 0.0]}))
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": data, "usage": {"total_tokens": n}}))
        })
        .mount(&server)
        .await;
    let p = openai(&server, Some(3));
    let t: Vec<String> = (0..300).map(|i| format!("t{i}")).collect();
    let resp = p
        .embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap();
    assert_eq!(resp.vectors.len(), 300);
    assert_eq!(resp.usage_tokens, 300);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn voyage_input_type_query_vs_document() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"index": 0, "embedding": [1.0, 0.0, 0.0, 0.0]}],
            "usage": {"total_tokens": 3}
        })))
        .mount(&server)
        .await;
    let p = voyage(&server);
    let t = texts(&["authorize user"]);
    p.embed(EmbedRequest::new(InputKind::Query, &t))
        .await
        .unwrap();
    p.embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap();
    let reqs = server.received_requests().await.unwrap();
    let kinds: Vec<String> = reqs
        .iter()
        .map(|r| {
            r.body_json::<Value>().unwrap()["input_type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, vec!["query", "document"]);
    let first: Value = reqs[0].body_json().unwrap();
    assert_eq!(first["model"], "voyage-code-3");
    assert_eq!(first["output_dimension"], 4);
}

async fn status_error(status: u16, headers: &[(&str, &str)]) -> EmbedError {
    let server = MockServer::start().await;
    let mut tpl = ResponseTemplate::new(status).set_body_string("{\"error\":\"nope\"}");
    for (k, v) in headers {
        tpl = tpl.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .respond_with(tpl)
        .mount(&server)
        .await;
    let p = openai(&server, None);
    let t = texts(&["x"]);
    p.embed(EmbedRequest::new(InputKind::Document, &t))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn http_429_maps_to_rate_limited_with_retry_after() {
    assert_eq!(
        status_error(429, &[("retry-after", "2")]).await,
        EmbedError::RateLimited {
            retry_after_ms: 2_000
        }
    );
    assert_eq!(
        status_error(429, &[]).await,
        EmbedError::RateLimited { retry_after_ms: 0 }
    );
}

#[tokio::test]
async fn http_500_transient() {
    assert!(matches!(
        status_error(500, &[]).await,
        EmbedError::Transient(_)
    ));
    assert!(matches!(
        status_error(503, &[]).await,
        EmbedError::Transient(_)
    ));
}

#[tokio::test]
async fn http_400_permanent() {
    assert!(matches!(
        status_error(400, &[]).await,
        EmbedError::Permanent(_)
    ));
    assert!(matches!(
        status_error(401, &[]).await,
        EmbedError::Permanent(_)
    ));
}

#[test]
fn privacy_no_external_forces_hash() {
    let cfg = EmbeddingConfig {
        provider: ProviderName::Voyage,
        ..EmbeddingConfig::default()
    };
    assert_eq!(
        cfg.effective_provider(Privacy::NoExternal),
        ProviderName::Hash
    );
    assert_eq!(
        cfg.effective_provider(Privacy::Standard),
        ProviderName::Voyage
    );
    // No key needed: the hash provider is used.
    let p = build_provider(&cfg, Privacy::NoExternal, |_| None).unwrap();
    assert_eq!(p.space().provider, ProviderName::Hash);
    assert_eq!(p.space().id(), "hash-fh768-768");
}

#[test]
fn missing_key_fails_construction() {
    for provider in [ProviderName::Openai, ProviderName::Voyage] {
        let cfg = EmbeddingConfig {
            provider,
            ..EmbeddingConfig::default()
        };
        let err = build_raw(&cfg, Privacy::Standard, |_| None).unwrap_err();
        assert!(err.to_string().contains("API_KEY"), "{err}");
        assert!(build_raw(&cfg, Privacy::Standard, |_| Some(String::new())).is_err());
        assert!(build_raw(&cfg, Privacy::Standard, |_| Some("k".into())).is_ok());
    }
}

#[test]
fn config_from_environment() {
    let env = |k: &str| match k {
        "SEMANTIC_EMBEDDING_PROVIDER" => Some("openai".to_owned()),
        "SEMANTIC_EMBEDDING_DIMS" => Some("512".to_owned()),
        "SEMANTIC_EMBEDDING_CONCURRENCY" => Some("8".to_owned()),
        _ => None,
    };
    let cfg = EmbeddingConfig::from_lookup(env).unwrap();
    assert_eq!(cfg.provider, ProviderName::Openai);
    assert_eq!(cfg.dims, Some(512));
    assert_eq!(cfg.concurrency, 8);
    assert_eq!(
        EmbeddingConfig::from_lookup(|_| None).unwrap().provider,
        ProviderName::Hash
    );
    assert!(EmbeddingConfig::from_lookup(
        |k| (k == "SEMANTIC_EMBEDDING_PROVIDER").then(|| "bert".to_owned())
    )
    .is_err());
}

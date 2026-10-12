#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Shared test helpers: an in-process Qdrant emulator on wiremock, counting providers, fixtures.

pub mod fake_qdrant;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId, SymbolId};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol::SymbolKind;
use semantic::audit::TenantAudit;
use semantic::embedding::{
    standard, EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider, EmbeddingSpace,
    HashProvider,
};
use semantic::units::SymbolInput;
use semantic::{CollectionTargets, QdrantConfig, SemanticIndex, TenantScope};

pub use fake_qdrant::FakeQdrant;

/// Hash provider that counts embedding calls and texts and records inputs.
#[derive(Debug)]
pub struct CountingProvider {
    inner: HashProvider,
    pub calls: AtomicUsize,
    pub texts: AtomicUsize,
    pub inputs: Mutex<Vec<String>>,
}

impl CountingProvider {
    pub fn new(dims: u16) -> Arc<Self> {
        Arc::new(Self {
            inner: HashProvider::new(dims, 1).unwrap(),
            calls: AtomicUsize::new(0),
            texts: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn texts(&self) -> usize {
        self.texts.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.calls.store(0, Ordering::SeqCst);
        self.texts.store(0, Ordering::SeqCst);
    }
}

#[async_trait]
impl EmbeddingProvider for CountingProvider {
    fn space(&self) -> &EmbeddingSpace {
        self.inner.space()
    }

    fn max_batch(&self) -> usize {
        self.inner.max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        self.inner.max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.texts.fetch_add(req.texts.len(), Ordering::SeqCst);
        self.inputs
            .lock()
            .unwrap()
            .extend(req.texts.iter().cloned());
        self.inner.embed(req).await
    }
}

/// A test index over a fake (or live) Qdrant with the audit layer enabled.
pub struct TestIndex {
    pub index: SemanticIndex,
    pub provider: Arc<CountingProvider>,
    pub audit: Arc<TenantAudit>,
}

pub fn index_for(url: &str, dims: u16, targets: Option<CollectionTargets>) -> TestIndex {
    let provider = CountingProvider::new(dims);
    let audit = TenantAudit::new();
    let mut index = SemanticIndex::new(
        QdrantConfig::new(url).without_backoff(),
        standard(Arc::clone(&provider)),
    )
    .unwrap()
    .with_audit(Arc::clone(&audit));
    if let Some(t) = targets {
        index = index.with_targets(t);
    }
    TestIndex {
        index,
        provider,
        audit,
    }
}

pub fn scope() -> (TenantScope, RepositoryId) {
    let repo = RepositoryId::new();
    (TenantScope::single(OrganizationId::new(), repo), repo)
}

pub fn path(p: &str) -> RepoPath {
    RepoPath::new(p).unwrap()
}

/// `AuthService.authorize` from the auth-bypass golden scenario.
pub fn authorize_symbol() -> SymbolInput {
    SymbolInput {
        symbol_id: SymbolId::from_canonical_unchecked(
            "ts:src/auth/auth.service#AuthService.authorize/method",
        ),
        kind: SymbolKind::Method,
        qualified_name: "AuthService.authorize".into(),
        signature: Some("async authorize(user: User, resource: Resource): Promise<boolean>".into()),
        file_path: path("src/auth/auth.service.ts"),
        language: Language::Typescript,
        start_line: 10,
        end_line: 13,
        decorators: vec![],
        callees: vec![("PermissionService.check".into(), 0.95)],
        callers: vec!["AdminService.updateUser".into()],
        doc: Some("Checks whether the user may access the resource.".into()),
        body: Some(
            "async authorize(user, resource) {\n  const allowed = await this.permissionService.check(user.id, resource.id);\n  return allowed;\n}"
                .into(),
        ),
        is_private: false,
        is_generated: false,
        has_secrets: false,
    }
}

/// A generic function symbol named `name` in `file` with `lines` body lines.
pub fn function_symbol(name: &str, file: &str, lines: usize) -> SymbolInput {
    let body: Vec<String> = (0..lines)
        .map(|i| format!("  const v{i} = {name}Helper({i});"))
        .collect();
    SymbolInput {
        symbol_id: SymbolId::from_canonical_unchecked(format!(
            "ts:{}#{name}/function",
            file.trim_end_matches(".ts")
        )),
        kind: SymbolKind::Function,
        qualified_name: name.into(),
        signature: Some(format!("function {name}(input: string): void")),
        file_path: path(file),
        language: Language::Typescript,
        start_line: 1,
        end_line: u32::try_from(lines).unwrap() + 1,
        decorators: vec![],
        callees: vec![],
        callers: vec![],
        doc: None,
        body: Some(format!(
            "function {name}(input) {{\n{}\n}}",
            body.join("\n")
        )),
        is_private: false,
        is_generated: false,
        has_secrets: false,
    }
}

pub fn snapshot() -> SnapshotId {
    SnapshotId::new()
}

/// `n` small convention units of `repo`.
pub fn convention_units(
    repo: RepositoryId,
    snap: SnapshotId,
    n: usize,
) -> Vec<semantic::EmbeddingUnit> {
    (0..n)
        .map(|i| {
            semantic::units::convention_unit(
                &semantic::units::ConventionInput {
                    id: format!("conv-{i}"),
                    rule: format!("services named service{i} use dependency injection"),
                    scope: "src/**".into(),
                    examples: vec![format!("Service{i}")],
                    confidence: 0.9,
                },
                repo,
                snap,
            )
        })
        .collect()
}

/// An audited index over the live Qdrant at `QDRANT_URL` (shared test collection; tests isolate
/// by tenant). `None` when no Qdrant is configured.
pub async fn live_index() -> Option<TestIndex> {
    let url = std::env::var("QDRANT_URL").ok()?;
    let t = index_for(&url, DIMS, None);
    semantic::bootstrap(
        &t.index,
        &semantic::MemoryRegistry::new(),
        semantic::HnswCfg::default(),
    )
    .await
    .expect("bootstrap against QDRANT_URL");
    Some(t)
}

/// Summary and chunk units of `n` generated functions.
pub fn units_for_live(repo: RepositoryId, n: usize) -> Vec<semantic::EmbeddingUnit> {
    let snap = snapshot();
    let mut out = Vec::new();
    for i in 0..n {
        let sym = function_symbol(&format!("liveHandler{i}"), &format!("src/live{i}.ts"), 8);
        out.extend(semantic::units::symbol_summary(&sym, repo, snap));
        out.extend(semantic::units::code_chunks(&sym, repo, snap));
    }
    out
}

pub const DIMS: u16 = 256;
pub const COLLECTION: &str = "rg_hash_fh256_256_v1";

/// A fake Qdrant with the default test collection, and an index over it.
pub async fn fake_index() -> (wiremock::MockServer, FakeQdrant, TestIndex) {
    let (server, fake) = FakeQdrant::start().await;
    fake.create(COLLECTION, u64::from(DIMS));
    let t = index_for(
        &server.uri(),
        DIMS,
        Some(CollectionTargets::single(COLLECTION)),
    );
    (server, fake, t)
}

//! Historical review intelligence: provider history ingest (HIST-001) and finding outcome
//! derivation (HIST-002). PRD §67–§70.
//!
//! * [`normalize`] — provider-neutral raw history to the stored shape: salted author hashes,
//!   redacted and capped comment bodies.
//! * [`link`] — maps a historical comment to a symbol, or keeps the path only.
//! * [`ingest`] — the resumable, idempotent ingest driver behind the `history-ingest` job.
//! * [`outcome`] — pure outcome precedence and inference for published findings.

pub mod ingest;
pub mod link;
pub mod normalize;
pub mod outcome;

pub use ingest::{
    run_ingest, CursorState, HistoryProvider, HistoryStore, IngestConfig, IngestMode,
    IngestOutcome, IngestReport, IngestStatus, NormalizedPage, NormalizedPr, PageCounts,
    ProviderFailure, ProviderPage, StoreError,
};
pub use link::{LinkStatus, RangeSymbolResolver, SymbolRange, SymbolResolver};
pub use normalize::{
    AuthorKind, HistoryComment, HistoryPr, PrState, RawComment, RawPullRequest, SizeStats,
    MAX_BODY_BYTES,
};

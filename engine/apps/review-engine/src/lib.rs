//! review-engine: the read-only internal HTTP API in front of the in-memory code graph (API-013).
//!
//! The NestJS API is the only caller. It mints a short-lived service token per request
//! (`aud=rg-engine`, `scope=graph:read`, `org`, `repo`), and every route under `/internal/v1`
//! checks it ([`auth`]). Graph queries run against a per-snapshot `Arc<Graph>` held in a small
//! LRU with single-flight loading ([`state`]); the data comes from an [`backend::EngineBackend`].

pub mod auth;
pub mod backend;
pub mod convert;
pub mod dto;
pub mod error;
pub mod routes;
pub mod state;

pub use routes::router;
pub use state::{AppState, EngineSettings};

//! Request and response types of the Qdrant client (SEM-003).

use serde_json::{Map, Value};
use uuid::Uuid;

/// Point payload (metadata only; unit text lives in PostgreSQL / is reconstructible).
pub type Payload = Map<String, Value>;

/// Maximum serialized payload per point.
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;

/// Points per upsert request.
pub const UPSERT_BATCH: usize = 256;

/// HNSW and storage settings of a new collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HnswCfg {
    pub m: u32,
    pub ef_construct: u32,
    pub on_disk_payload: bool,
}

impl Default for HnswCfg {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construct: 128,
            on_disk_payload: true,
        }
    }
}

/// What `ensure_collection` found or created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionState {
    pub name: String,
    /// Vector size of the collection.
    pub dims: u64,
    /// `true` when this call created it.
    pub created: bool,
    pub points_count: Option<u64>,
    /// Fields that already have a payload index.
    pub indexed_fields: Vec<String>,
}

/// One point to write.
#[derive(Debug, Clone, PartialEq)]
pub struct PointUpsert {
    pub id: Uuid,
    pub vector: Vec<f32>,
    pub payload: Payload,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredPoint {
    pub id: Uuid,
    pub score: f32,
    pub payload: Payload,
}

/// One scrolled point.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: Uuid,
    pub payload: Payload,
    pub vector: Option<Vec<f32>>,
}

/// One page of a scroll.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollPage {
    pub points: Vec<Record>,
    pub next_offset: Option<Uuid>,
}

/// Which payload fields a scroll returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithPayload<'a> {
    All,
    Fields(&'a [&'static str]),
}

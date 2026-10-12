//! The snapshot file container of the file adapter (GS-006).
//!
//! ```text
//! +------------+------------------------------+
//! | 52 bytes   | zstd frame                   |
//! | header     | JSON payload                 |
//! +------------+------------------------------+
//! ```
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0  | 4 | `magic` = `RGFS` |
//! | 4  | 1 | `format` = 1 |
//! | 5  | 1 | `payload` = 1 (`Graph`), 2 (`GraphDelta`) |
//! | 6  | 2 | reserved, zero |
//! | 8  | 4 | `schema_version`, little-endian |
//! | 12 | 8 | `uncompressed_len`, little-endian |
//! | 20 | 32 | `blake3` of the uncompressed payload |
//!
//! The payload is JSON because node attributes are free-form `serde_json::Value`s, which the
//! non-self-describing `bincode` format cannot read back. The files are local, untrusted input:
//! the declared length is checked against a limit before anything is inflated, the inflated
//! stream is capped at the declared length, and the hash is verified before the JSON is parsed.

use std::io::Read;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::model::{Graph, GraphDelta};
use crate::types::GRAPH_SCHEMA_VERSION;
use crate::StoreError;

const MAGIC: [u8; 4] = *b"RGFS";
const FORMAT: u8 = 1;
const PAYLOAD_GRAPH: u8 = 1;
const PAYLOAD_DELTA: u8 = 2;
pub(crate) const HEADER_LEN: usize = 52;
const ZSTD_LEVEL: i32 = 3;

/// Zip-bomb guard: no snapshot payload may inflate beyond this.
const MAX_UNCOMPRESSED_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// What a snapshot file holds.
#[derive(Debug)]
pub(crate) enum Payload {
    Full(Graph),
    Delta(GraphDelta),
}

/// Builds an `Integrity` error and counts it (`graph_store_file_integrity_errors_total`).
pub(crate) fn integrity(message: impl Into<String>) -> StoreError {
    let message = message.into();
    tracing::error!(
        metric = "graph_store_file_integrity_errors_total",
        adapter = "file",
        error = %message,
        "graph store file integrity error"
    );
    StoreError::Integrity(message)
}

pub(crate) fn encode_graph(graph: &Graph) -> Result<Vec<u8>, StoreError> {
    encode(PAYLOAD_GRAPH, graph.schema_version, graph)
}

pub(crate) fn encode_delta(delta: &GraphDelta) -> Result<Vec<u8>, StoreError> {
    encode(PAYLOAD_DELTA, GRAPH_SCHEMA_VERSION, delta)
}

fn encode<T: Serialize>(
    payload: u8,
    schema_version: u32,
    value: &T,
) -> Result<Vec<u8>, StoreError> {
    let raw = serde_json::to_vec(value).map_err(StoreError::backend)?;
    let hash = blake3::hash(&raw);
    let compressed =
        zstd::stream::encode_all(raw.as_slice(), ZSTD_LEVEL).map_err(StoreError::backend)?;
    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&MAGIC);
    out.push(FORMAT);
    out.push(payload);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&schema_version.to_le_bytes());
    out.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    out.extend_from_slice(hash.as_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Verifies and decodes a snapshot file.
pub(crate) fn decode(bytes: &[u8]) -> Result<Payload, StoreError> {
    if bytes.len() < HEADER_LEN {
        return Err(integrity("snapshot file is shorter than its header"));
    }
    if bytes[..4] != MAGIC {
        return Err(integrity("snapshot file has the wrong magic"));
    }
    if bytes[4] != FORMAT {
        return Err(integrity(format!(
            "unsupported snapshot container format {}",
            bytes[4]
        )));
    }
    let payload = bytes[5];
    if bytes[6] != 0 || bytes[7] != 0 {
        return Err(integrity("reserved header bytes are not zero"));
    }
    let schema_version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if schema_version != GRAPH_SCHEMA_VERSION {
        return Err(StoreError::SchemaMismatch {
            found: schema_version,
            expected: GRAPH_SCHEMA_VERSION,
        });
    }
    let mut len = [0u8; 8];
    len.copy_from_slice(&bytes[12..20]);
    let declared = u64::from_le_bytes(len);
    if declared > MAX_UNCOMPRESSED_BYTES {
        return Err(integrity(format!(
            "snapshot payload declares {declared} bytes, over the {MAX_UNCOMPRESSED_BYTES} byte limit"
        )));
    }
    let expected_hash = &bytes[20..HEADER_LEN];

    let decoder = zstd::stream::read::Decoder::new(&bytes[HEADER_LEN..])
        .map_err(|e| integrity(format!("snapshot payload is not a zstd frame: {e}")))?;
    let mut raw = Vec::new();
    decoder
        .take(declared.saturating_add(1))
        .read_to_end(&mut raw)
        .map_err(|e| integrity(format!("snapshot payload does not inflate: {e}")))?;
    if raw.len() as u64 != declared {
        return Err(integrity(format!(
            "snapshot payload inflated to {} bytes, the header says {declared}",
            raw.len()
        )));
    }
    if blake3::hash(&raw).as_bytes().as_slice() != expected_hash {
        return Err(integrity("snapshot payload hash mismatch"));
    }

    match payload {
        PAYLOAD_GRAPH => parse(&raw).map(Payload::Full),
        PAYLOAD_DELTA => parse(&raw).map(Payload::Delta),
        other => Err(integrity(format!(
            "unsupported snapshot payload kind {other}"
        ))),
    }
}

fn parse<T: DeserializeOwned>(raw: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(raw)
        .map_err(|e| integrity(format!("snapshot payload is not valid: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        Graph::new(GRAPH_SCHEMA_VERSION)
    }

    #[test]
    fn graph_and_delta_roundtrip() {
        let g = graph();
        let bytes = encode_graph(&g).unwrap();
        assert_eq!(&bytes[..4], b"RGFS");
        assert!(matches!(decode(&bytes).unwrap(), Payload::Full(back) if back == g));

        let d = GraphDelta::default();
        let bytes = encode_delta(&d).unwrap();
        assert!(matches!(decode(&bytes).unwrap(), Payload::Delta(back) if back == d));
    }

    #[test]
    fn corruption_is_rejected() {
        let bytes = encode_graph(&graph()).unwrap();

        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0xff;
        assert!(matches!(decode(&flipped), Err(StoreError::Integrity(_))));

        let mut bad_hash = bytes.clone();
        bad_hash[20] ^= 0xff;
        assert!(matches!(decode(&bad_hash), Err(StoreError::Integrity(_))));

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(matches!(decode(&bad_magic), Err(StoreError::Integrity(_))));

        assert!(matches!(
            decode(&bytes[..10]),
            Err(StoreError::Integrity(_))
        ));
        assert!(matches!(
            decode(&bytes[..HEADER_LEN]),
            Err(StoreError::Integrity(_))
        ));
    }

    #[test]
    fn other_schema_version_is_a_mismatch() {
        let mut bytes = encode_graph(&graph()).unwrap();
        bytes[8] = 9;
        assert!(matches!(
            decode(&bytes),
            Err(StoreError::SchemaMismatch {
                found: 9,
                expected: GRAPH_SCHEMA_VERSION
            })
        ));
    }

    #[test]
    fn declared_length_over_the_limit_is_refused_before_inflating() {
        let mut bytes = encode_graph(&graph()).unwrap();
        bytes[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(StoreError::Integrity(_))));
    }
}

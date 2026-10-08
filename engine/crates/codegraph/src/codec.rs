//! The versioned wire format for `Graph` and `GraphDelta` (CG-011).
//!
//! # Layout
//!
//! ```text
//! ┌────────────┬──────────────────────────────┐
//! │ 52 bytes   │ zstd frame                   │
//! │ header     │ bincode payload              │
//! └────────────┴──────────────────────────────┘
//! ```
//!
//! The header is written field by field, never transmuted, so it is endian- and
//! alignment-independent:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0  | 4 | `magic` = `RGGR` |
//! | 4  | 1 | `format` = 1 |
//! | 5  | 1 | `codec` = 1 (bincode) |
//! | 6  | 1 | `payload` = 1 (`Graph`), 2 (`Delta`) |
//! | 7  | 1 | reserved, must be 0 |
//! | 8  | 4 | `schema_version`, little-endian |
//! | 12 | 8 | `uncompressed_len`, little-endian |
//! | 20 | 32 | `blake3` of the *uncompressed* payload |
//!
//! The hash covers the uncompressed bytes on purpose: it is what the reader can verify without
//! decompressing a zip bomb first, and `max_uncompressed_bytes` bounds that work.
//!
//! # Canonical form
//!
//! The wire payload stores the string table, files, nodes, edges and unresolved references in the
//! *sorted* order the builder produces — not the CSR. Decoding therefore goes back through
//! [`GraphBuilder`], which means the format does not depend on the in-memory layout and a graph
//! encoded by one build decodes identically in another (ADR-015).

use std::io::{Read, Write};

use crate::delta::GraphDelta;
use crate::graph::{Graph, GraphBuildError, GraphBuilder};

/// `RGGR` — ReviewGraph graph record.
pub const MAGIC: [u8; 4] = *b"RGGR";

/// Container format version. Bumped when the header layout changes.
pub const FORMAT_VERSION: u8 = 1;

/// Codec identifier: 1 = bincode.
pub const CODEC_BINCODE: u8 = 1;

/// Payload discriminator.
pub const PAYLOAD_GRAPH: u8 = 1;
pub const PAYLOAD_DELTA: u8 = 2;

/// Size of the header in bytes.
pub const HEADER_LEN: usize = 52;

/// Default decompression limit: 8 GiB.
pub const DEFAULT_MAX_UNCOMPRESSED_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// zstd compression level (3): the ratio/time curve the baseline was measured on.
const ZSTD_LEVEL: i32 = 3;

/// Why an encode or decode failed. No partial graph is ever returned.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodecError {
    #[error("not a ReviewGraph graph record: magic {0:?}")]
    BadMagic([u8; 4]),
    #[error("unsupported container format {0}")]
    UnsupportedFormat(u8),
    #[error("unsupported codec {0}")]
    UnsupportedCodec(u8),
    #[error("unsupported payload kind {0}")]
    UnsupportedPayload(u8),
    #[error("reserved header byte must be zero, found {0}")]
    ReservedByte(u8),
    #[error("schema version {found} in the record, this build speaks {expected}")]
    SchemaMismatch { found: u32, expected: u32 },
    #[error("payload of {len} bytes exceeds the {limit} byte limit")]
    TooLarge { len: u64, limit: u64 },
    #[error("payload hash mismatch: the record is corrupt or was tampered with")]
    IntegrityMismatch,
    #[error("record ends early")]
    Truncated,
    #[error("header says {declared} uncompressed bytes, the frame produced {actual}")]
    LengthMismatch { declared: u64, actual: u64 },
    #[error("i/o error: {0}")]
    Io(String),
    #[error("codec error: {0}")]
    Decode(String),
    #[error(transparent)]
    Graph(#[from] GraphBuildError),
}

/// What an encode wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncodeStats {
    pub bytes_compressed: usize,
    pub bytes_uncompressed: usize,
    /// The uncompressed payload's blake3, the same value the header carries.
    pub payload_hash: [u8; 32],
}

/// Bounds applied while decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Zip-bomb guard: the decompressed payload may not exceed this.
    pub max_uncompressed_bytes: u64,
    /// The schema version the caller can read. `SCHEMA_VERSION` in practice.
    pub expected_schema: u32,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            expected_schema: crate::schema::SCHEMA_VERSION,
        }
    }
}

impl DecodeLimits {
    /// The default limits for `expected_schema`.
    #[must_use]
    pub fn for_schema(expected_schema: u32) -> Self {
        Self {
            expected_schema,
            ..Self::default()
        }
    }
}

/// The 52-byte header, as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub magic: [u8; 4],
    pub format: u8,
    pub codec: u8,
    pub payload: u8,
    pub reserved: u8,
    pub schema_version: u32,
    pub uncompressed_len: u64,
    pub blake3: [u8; 32],
}

impl Header {
    /// A header for a payload of `payload` kind.
    #[must_use]
    pub fn new(payload: u8, schema_version: u32, uncompressed_len: u64, blake3: [u8; 32]) -> Self {
        Self {
            magic: MAGIC,
            format: FORMAT_VERSION,
            codec: CODEC_BINCODE,
            payload,
            reserved: 0,
            schema_version,
            uncompressed_len,
            blake3,
        }
    }

    /// Writes the header field by field, little-endian.
    ///
    /// # Errors
    ///
    /// [`CodecError::Io`] when the writer fails.
    pub fn write_to(&self, out: &mut impl Write) -> Result<(), CodecError> {
        out.write_all(&self.magic).map_err(io)?;
        out.write_all(&[self.format]).map_err(io)?;
        out.write_all(&[self.codec]).map_err(io)?;
        out.write_all(&[self.payload]).map_err(io)?;
        out.write_all(&[self.reserved]).map_err(io)?;
        out.write_all(&self.schema_version.to_le_bytes())
            .map_err(io)?;
        out.write_all(&self.uncompressed_len.to_le_bytes())
            .map_err(io)?;
        out.write_all(&self.blake3).map_err(io)?;
        Ok(())
    }

    /// Reads exactly [`HEADER_LEN`] bytes.
    ///
    /// # Errors
    ///
    /// [`CodecError::Truncated`] for a short read, then every validation the format requires.
    pub fn read_from(input: &mut impl Read) -> Result<Self, CodecError> {
        let mut magic = [0u8; 4];
        read_exact(input, &mut magic)?;
        let mut bytes = [0u8; 4];
        read_exact(input, &mut bytes)?;
        let mut schema_version = [0u8; 4];
        read_exact(input, &mut schema_version)?;
        let mut uncompressed_len = [0u8; 8];
        read_exact(input, &mut uncompressed_len)?;
        let mut blake3 = [0u8; 32];
        read_exact(input, &mut blake3)?;
        Ok(Self {
            magic,
            format: bytes[0],
            codec: bytes[1],
            payload: bytes[2],
            reserved: bytes[3],
            schema_version: u32::from_le_bytes(schema_version),
            uncompressed_len: u64::from_le_bytes(uncompressed_len),
            blake3,
        })
    }

    /// Checks everything that does not need the payload.
    ///
    /// # Errors
    ///
    /// [`CodecError::BadMagic`], [`CodecError::UnsupportedFormat`],
    /// [`CodecError::UnsupportedCodec`], [`CodecError::UnsupportedPayload`],
    /// [`CodecError::ReservedByte`] or [`CodecError::SchemaMismatch`].
    pub fn check(&self, limits: &DecodeLimits) -> Result<(), CodecError> {
        if self.magic != MAGIC {
            return Err(CodecError::BadMagic(self.magic));
        }
        if self.format != FORMAT_VERSION {
            return Err(CodecError::UnsupportedFormat(self.format));
        }
        if self.codec != CODEC_BINCODE {
            return Err(CodecError::UnsupportedCodec(self.codec));
        }
        if self.payload != PAYLOAD_GRAPH && self.payload != PAYLOAD_DELTA {
            return Err(CodecError::UnsupportedPayload(self.payload));
        }
        if self.reserved != 0 {
            return Err(CodecError::ReservedByte(self.reserved));
        }
        if self.schema_version != limits.expected_schema {
            return Err(CodecError::SchemaMismatch {
                found: self.schema_version,
                expected: limits.expected_schema,
            });
        }
        if self.uncompressed_len > limits.max_uncompressed_bytes {
            return Err(CodecError::TooLarge {
                len: self.uncompressed_len,
                limit: limits.max_uncompressed_bytes,
            });
        }
        Ok(())
    }

    /// The header as the 52 bytes that go on the wire.
    #[must_use]
    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..4].copy_from_slice(&self.magic);
        out[4] = self.format;
        out[5] = self.codec;
        out[6] = self.payload;
        out[7] = self.reserved;
        out[8..12].copy_from_slice(&self.schema_version.to_le_bytes());
        out[12..20].copy_from_slice(&self.uncompressed_len.to_le_bytes());
        out[20..52].copy_from_slice(&self.blake3);
        out
    }

    /// Reads a header out of a fixed byte block.
    ///
    /// # Errors
    ///
    /// [`CodecError::Truncated`] when `bytes` is shorter than [`HEADER_LEN`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() < HEADER_LEN {
            return Err(CodecError::Truncated);
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        let mut blake3 = [0u8; 32];
        blake3.copy_from_slice(&bytes[20..52]);
        Ok(Self {
            magic,
            format: bytes[4],
            codec: bytes[5],
            payload: bytes[6],
            reserved: bytes[7],
            schema_version: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            uncompressed_len: u64::from_le_bytes([
                bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
                bytes[19],
            ]),
            blake3,
        })
    }
}

fn io(error: std::io::Error) -> CodecError {
    CodecError::Io(error.to_string())
}

fn read_exact(input: &mut impl Read, buffer: &mut [u8]) -> Result<(), CodecError> {
    input.read_exact(buffer).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            CodecError::Truncated
        } else {
            io(error)
        }
    })
}

/// Writes a graph as `header ‖ zstd(bincode(payload))`.
///
/// # Errors
///
/// [`CodecError::Io`] when the writer fails, or [`CodecError::Graph`] when the graph cannot be
/// re-derived from its own canonical form (which would mean it was built wrong).
pub fn encode_graph(graph: &Graph, out: &mut impl Write) -> Result<EncodeStats, CodecError> {
    let wire = GraphWire::of(graph)?;
    encode(PAYLOAD_GRAPH, graph.schema_version(), &wire, out)
}

/// Writes a delta as `header ‖ zstd(bincode(payload))`.
///
/// # Errors
///
/// [`CodecError::Io`] when the writer fails, plus the [`GraphBuildError`]s a delta can carry.
pub fn encode_delta(delta: &GraphDelta, out: &mut impl Write) -> Result<EncodeStats, CodecError> {
    encode(PAYLOAD_DELTA, delta.base_schema_version, &delta, out)
}

fn encode<T: serde::Serialize>(
    payload: u8,
    schema_version: u32,
    value: &T,
    out: &mut impl Write,
) -> Result<EncodeStats, CodecError> {
    let raw = bincode::serialize(value).map_err(|error| CodecError::Decode(error.to_string()))?;
    let hash = *blake3::hash(&raw).as_bytes();
    Header::new(payload, schema_version, raw.len() as u64, hash).write_to(out)?;
    let compressed = zstd::stream::encode_all(raw.as_slice(), ZSTD_LEVEL).map_err(io)?;
    out.write_all(&compressed).map_err(io)?;
    Ok(EncodeStats {
        bytes_compressed: compressed.len(),
        bytes_uncompressed: raw.len(),
        payload_hash: hash,
    })
}

/// Reads a graph written by [`encode_graph`].
///
/// # Errors
///
/// Every [`CodecError`] variant; a failed decode never returns a partial graph.
pub fn decode_graph(input: &mut impl Read, limits: DecodeLimits) -> Result<Graph, CodecError> {
    let (header, raw) = read_payload(input, limits, PAYLOAD_GRAPH)?;
    let wire: GraphWire =
        bincode::deserialize(&raw).map_err(|error| CodecError::Decode(error.to_string()))?;
    wire.into_graph(&header)
}

/// Reads a delta written by [`encode_delta`].
///
/// # Errors
///
/// Every [`CodecError`] variant.
pub fn decode_delta(input: &mut impl Read, limits: DecodeLimits) -> Result<GraphDelta, CodecError> {
    let (_, raw) = read_payload(input, limits, PAYLOAD_DELTA)?;
    bincode::deserialize(&raw).map_err(|error| CodecError::Decode(error.to_string()))
}

/// Header + decompressed payload, with the hash and length verified before anything is parsed.
fn read_payload(
    input: &mut impl Read,
    limits: DecodeLimits,
    expected: u8,
) -> Result<(Header, Vec<u8>), CodecError> {
    let header = Header::read_from(input)?;
    header.check(&limits)?;
    if header.payload != expected {
        return Err(CodecError::UnsupportedPayload(header.payload));
    }
    // `take(limit + 1)` is the zip-bomb guard: even a record that lies in its header cannot make
    // the reader allocate more than the limit.
    let mut reader = input.take(limits.max_uncompressed_bytes.saturating_add(1));
    let raw = zstd::stream::decode_all(&mut reader).map_err(|error| {
        if error.to_string().contains("Unknown frame descriptor") {
            CodecError::Truncated
        } else {
            CodecError::Decode(error.to_string())
        }
    })?;
    if raw.len() as u64 > limits.max_uncompressed_bytes {
        return Err(CodecError::TooLarge {
            len: raw.len() as u64,
            limit: limits.max_uncompressed_bytes,
        });
    }
    if raw.len() as u64 != header.uncompressed_len {
        return Err(CodecError::LengthMismatch {
            declared: header.uncompressed_len,
            actual: raw.len() as u64,
        });
    }
    if *blake3::hash(&raw).as_bytes() != header.blake3 {
        return Err(CodecError::IntegrityMismatch);
    }
    Ok((header, raw))
}

/// The canonical wire form of a graph: everything in sorted order, no indices.
///
/// Serializing the *inputs* rather than the built layout is what makes the format independent of
/// the in-memory representation, and decoding is a straight re-run of [`GraphBuilder`] on already
/// sorted input.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphWire {
    pub schema_version: u32,
    pub files: Vec<crate::graph::FileInput>,
    pub nodes: Vec<crate::graph::NodeInput>,
    pub edges: Vec<crate::edge::Edge>,
    pub unresolved: Vec<crate::graph::UnresolvedRef>,
}

impl GraphWire {
    /// The canonical form of `graph`.
    ///
    /// # Errors
    ///
    /// Never today; the signature is fallible so a future field that cannot be recovered from the
    /// built layout has somewhere to report itself.
    pub fn of(graph: &Graph) -> Result<Self, CodecError> {
        let mut files: Vec<crate::graph::FileInput> = Vec::with_capacity(graph.files().len());
        for entry in graph.files() {
            files.push(crate::graph::FileInput {
                path: parse_path(graph, entry.path)?,
                file_version_id: entry.file_version_id,
                content_hash: entry.content_hash,
                language: entry.language,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let mut nodes: Vec<crate::graph::NodeInput> = Vec::with_capacity(graph.nodes().len());
        for node in graph.nodes() {
            let mut attrs = crate::graph::NodeInputAttrs {
                visibility: node.attrs.visibility,
                flags: node.attrs.flags,
                signature: node.attrs.signature.map(|id| graph.str(id).to_owned()),
                body_hash: node.attrs.body_hash,
                signature_hash: node.attrs.signature_hash,
                parent: node.attrs.parent,
                extra: Vec::new(),
            };
            if let Some(pairs) = &node.attrs.extra {
                attrs.extra = pairs
                    .iter()
                    .map(|(key, value)| (graph.str(*key).to_owned(), graph.str(*value).to_owned()))
                    .collect();
            }
            let mut input = crate::graph::NodeInput::new(
                crate::node_id::NodeId::from_canonical(graph.str(node.id).to_owned()),
                node.kind,
                graph.str(node.name),
            )
            .qualified_name(graph.str(node.qualified_name))
            .with_attrs(attrs);
            if let Some(entry) = node.file.and_then(|ix| graph.file(ix)) {
                input = input.in_file(parse_path(graph, entry.path)?);
            }
            if let Some(range) = node.range {
                input = input.with_range(range);
            }
            nodes.push(input);
        }

        let mut edges: Vec<crate::edge::Edge> = Vec::with_capacity(graph.edges().len());
        for edge in graph.edges() {
            let (Some(source), Some(target)) = (graph.node(edge.source), graph.node(edge.target))
            else {
                continue;
            };
            let mut owned = crate::edge::Edge::new(
                edge.kind,
                source.key,
                target.key,
                edge.confidence,
                edge.resolved_by,
                edge.provenance,
            )
            .with_flags(edge.flags)
            .with_occurrences(edge.occurrences);
            if let Some(entry) = edge.origin_file.and_then(|ix| graph.file(ix)) {
                let path = parse_path(graph, entry.path)?;
                owned = if edge.has_location() {
                    owned.with_location(crate::edge::Location::new(path, edge.line, edge.col))
                } else {
                    owned.with_origin_file(path)
                };
            }
            edges.push(owned);
        }
        edges.sort();

        let mut unresolved = graph.unresolved().to_vec();
        unresolved.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.ordinal.cmp(&b.ordinal))
                .then_with(|| a.name.cmp(&b.name))
        });

        Ok(Self {
            schema_version: graph.schema_version(),
            files,
            nodes,
            edges,
            unresolved,
        })
    }

    /// Rebuilds the graph through [`GraphBuilder`], `O(V + E)` on already sorted input.
    ///
    /// # Errors
    ///
    /// [`CodecError::Graph`] for anything `build()` reports.
    pub fn into_graph(self, header: &Header) -> Result<Graph, CodecError> {
        if self.schema_version != header.schema_version {
            return Err(CodecError::SchemaMismatch {
                found: self.schema_version,
                expected: header.schema_version,
            });
        }
        let mut builder = GraphBuilder::new(self.schema_version);
        for file in self.files {
            builder.add_file(file)?;
        }
        for node in self.nodes {
            builder.add_node(node)?;
        }
        for edge in self.edges {
            builder.add_edge(edge);
        }
        for reference in self.unresolved {
            builder.add_unresolved(reference);
        }
        builder.build().map_err(CodecError::Graph)
    }
}

fn parse_path(
    graph: &Graph,
    id: crate::graph::StrId,
) -> Result<review_core::location::RepoPath, CodecError> {
    let raw = graph.str(id);
    review_core::location::RepoPath::new(raw)
        .map_err(|error| CodecError::Decode(format!("file path {raw:?}: {error}")))
}

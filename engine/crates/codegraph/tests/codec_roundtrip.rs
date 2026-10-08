//! `CG-011`: the `RGGR` container round-trips a graph and a delta losslessly, rejects every
//! corruption a truncated or tampered file can carry, and bounds decoding before it inflates.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Cursor;

use analysis_ir::reference::RefKind;
use codegraph::{
    compare, decode_delta, decode_graph, encode_delta, encode_graph, validate, CodecError,
    CompareOptions, Confidence, DecodeLimits, Edge, EdgeFlags, EdgeKind, FileChange, FileInput,
    Graph, GraphBuilder, GraphDelta, GraphDiffReport, GraphQuery, Location, NodeId, NodeInput,
    NodeKind, Provenance, ResolvedBy, UnresolvedReason, UnresolvedRef, HEADER_LEN, SCHEMA_VERSION,
};
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn symbol(file: &str, name: &str) -> NodeId {
    NodeId::from_canonical(format!("ts:{file}#{name}/function"))
}

fn node(file: &str, name: &str) -> NodeInput {
    NodeInput::new(symbol(file, name), NodeKind::Function, name)
        .in_file(path(file))
        .qualified_name(name.to_owned())
}

fn key(file: &str, name: &str) -> codegraph::NodeKey {
    symbol(file, name).key()
}

fn edge(kind: EdgeKind, source: &str, target: &str) -> Edge {
    Edge::new(
        kind,
        key("src/main.ts", source),
        symbol("src/util.ts", target).key(),
        Confidence::MAX,
        ResolvedBy::NameUnique,
        Provenance::Linker,
    )
}

/// A small graph exercising every wire type: two files, four nodes, two edges, an unresolved
/// reference with a location, and a synthetic node with no file.
fn corpus() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for (raw, hash) in [("src/main.ts", "main"), ("src/util.ts", "util")] {
        builder
            .add_file(FileInput {
                path: path(raw),
                file_version_id: Some(7),
                content_hash: ContentHash::of(hash.as_bytes()),
                language: Language::Typescript,
            })
            .unwrap();
    }
    builder.add_node(node("src/main.ts", "main")).unwrap();
    builder.add_node(node("src/main.ts", "router")).unwrap();
    builder.add_node(node("src/util.ts", "util")).unwrap();
    builder
        .add_node(NodeInput::new(
            NodeId::repository(),
            NodeKind::Repository,
            "reviewgraph",
        ))
        .unwrap();
    builder.add_edge(edge(EdgeKind::Calls, "main", "util"));
    builder.add_edge(edge(EdgeKind::Calls, "router", "util"));
    builder.add_unresolved(UnresolvedRef {
        file: path("src/main.ts"),
        ordinal: 0,
        from: None,
        name: "missing_helper".to_owned(),
        kind: RefKind::Call,
        import_specifier: Some("pkg".to_owned()),
        location: Location::new(path("src/main.ts"), 12, 3),
        reason: UnresolvedReason::External,
        candidate_count: 0,
    });
    builder.build().unwrap()
}

fn encoded_graph() -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_graph(&corpus(), &mut bytes).expect("a valid graph encodes");
    bytes
}

fn decode(bytes: &[u8]) -> Result<Graph, CodecError> {
    decode_graph(&mut Cursor::new(bytes), DecodeLimits::default())
}

/// `count` nodes in one file wired into a ring plus chords, so a round trip has real shape to
/// preserve. Built through the public API rather than `testkit`, so this file proves the surface a
/// consumer actually has.
fn synthetic(count: u32, degree: u32, seed: u64) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_file(FileInput {
            path: path("src/gen.ts"),
            file_version_id: Some(1),
            content_hash: ContentHash::of(b"gen"),
            language: Language::Typescript,
        })
        .unwrap();
    for n in 0..count {
        builder
            .add_node(node("src/gen.ts", &format!("f{n}")))
            .unwrap();
    }
    let kinds = [EdgeKind::Calls, EdgeKind::Reads, EdgeKind::Writes];
    for n in 0..count {
        for step in 1..=degree {
            let target = (u64::from(n) + u64::from(step) * 7 + seed) % u64::from(count);
            builder.add_edge(Edge::new(
                kinds[(n + step) as usize % kinds.len()],
                key("src/gen.ts", &format!("f{n}")),
                key("src/gen.ts", &format!("f{target}")),
                Confidence::from_f32(0.6 + step as f32 * 0.1),
                ResolvedBy::NameUnique,
                Provenance::Linker,
            ));
        }
    }
    builder.build().unwrap()
}

#[test]
fn a_graph_round_trips_and_re_encodes_identically() {
    let graph = corpus();
    let bytes = encoded_graph();

    let decoded = decode(&bytes).expect("a record this crate wrote decodes");
    assert_eq!(
        compare(&graph, &decoded, &CompareOptions::strict()),
        GraphDiffReport::default()
    );
    let report = validate(&decoded, SCHEMA_VERSION);
    assert!(
        report.is_ok(),
        "a decoded graph validates: {}",
        report.render(8)
    );

    // Canonical ordering is a precondition for byte-for-byte reproducibility, so encoding the
    // decoded graph must reproduce the file exactly.
    let mut again = Vec::new();
    let stats = encode_graph(&decoded, &mut again).expect("re-encoding works");
    assert_eq!(bytes, again, "the encoder is deterministic");
    assert_eq!(
        stats.bytes_compressed,
        bytes.len() - HEADER_LEN,
        "the frame follows the header"
    );
    assert!(stats.bytes_uncompressed > 0);
}

#[test]
fn the_header_is_52_bytes_little_endian() {
    let bytes = encoded_graph();
    assert!(bytes.len() > 52, "a frame follows the header");

    assert_eq!(&bytes[0..4], b"RGGR");
    assert_eq!(bytes[4], 1, "format version");
    assert_eq!(bytes[5], 1, "bincode");
    assert_eq!(bytes[6], 1, "graph payload");
    assert_eq!(bytes[7], 0, "reserved");
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        SCHEMA_VERSION
    );

    let declared = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    assert!(declared > 0, "the header declares the uncompressed length");
    assert_eq!(
        declared,
        encode_graph(&corpus(), &mut Vec::new())
            .unwrap()
            .bytes_uncompressed as u64
    );

    // The header's hash is the payload's blake3, which the encoder reports.
    let mut buffer = Vec::new();
    let stats = encode_graph(&corpus(), &mut buffer).unwrap();
    assert_eq!(&bytes[20..52], &stats.payload_hash[..]);
}

#[test]
fn a_delta_round_trips() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_added.push(node("src/new.ts", "new"));
    delta.edges_added.push(Edge::new(
        EdgeKind::Calls,
        key("src/main.ts", "main"),
        symbol("src/new.ts", "new").key(),
        Confidence::from_f32(0.7),
        ResolvedBy::ThisMember,
        Provenance::Linker,
    ));
    delta.files.push(FileChange::deleted(path("src/gone.ts")));
    delta.unresolved_replaced.push((
        path("src/main.ts"),
        vec![UnresolvedRef {
            file: path("src/main.ts"),
            ordinal: 1,
            from: None,
            name: "still_missing".to_owned(),
            kind: RefKind::TypeRef,
            import_specifier: None,
            location: Location::new(path("src/main.ts"), 20, 1),
            reason: UnresolvedReason::NotFound,
            candidate_count: 2,
        }],
    ));
    delta.normalize();

    let mut bytes = Vec::new();
    encode_delta(&delta, &mut bytes).expect("a valid delta encodes");
    assert_eq!(bytes[6], 2, "delta payload tag");

    let decoded = decode_delta(&mut Cursor::new(&bytes), DecodeLimits::default())
        .expect("the delta round-trips");
    assert_eq!(decoded, delta);
}

#[test]
fn a_record_without_the_magic_is_refused() {
    let mut bytes = encoded_graph();
    bytes[0] = b'X';
    assert!(
        matches!(decode(&bytes), Err(CodecError::BadMagic(magic)) if magic == *b"XGGR"),
        "a bad magic is reported as such"
    );
}

#[test]
fn an_unknown_format_codec_or_payload_is_refused() {
    let bytes = encoded_graph();

    let mut bad_format = bytes.clone();
    bad_format[4] = 9;
    assert!(matches!(
        decode(&bad_format),
        Err(CodecError::UnsupportedFormat(9))
    ));

    let mut bad_codec = bytes.clone();
    bad_codec[5] = 9;
    assert!(matches!(
        decode(&bad_codec),
        Err(CodecError::UnsupportedCodec(9))
    ));

    let mut bad_payload = bytes.clone();
    bad_payload[6] = 9;
    assert!(matches!(
        decode(&bad_payload),
        Err(CodecError::UnsupportedPayload(9))
    ));

    // A graph record is not a delta record.
    assert!(matches!(
        decode_delta(&mut Cursor::new(&bytes), DecodeLimits::default()),
        Err(CodecError::UnsupportedPayload(1))
    ));
}

#[test]
fn a_nonzero_reserved_byte_is_refused() {
    let mut bytes = encoded_graph();
    bytes[7] = 1;
    assert!(matches!(decode(&bytes), Err(CodecError::ReservedByte(1))));
}

#[test]
fn a_tampered_record_fails_its_hash() {
    let mut body = encoded_graph();
    let last = body.len() - 1;
    body[last] ^= 0xff;
    assert!(
        matches!(
            decode(&body),
            Err(CodecError::IntegrityMismatch) | Err(CodecError::Decode(_))
        ),
        "a flipped payload byte is caught"
    );

    let mut header = encoded_graph();
    header[20] ^= 0x01;
    assert!(
        matches!(decode(&header), Err(CodecError::IntegrityMismatch)),
        "a flipped hash byte is caught"
    );
}

#[test]
fn a_record_that_ends_early_is_refused() {
    let bytes = encoded_graph();

    let cut = bytes.len() - 4;
    assert!(matches!(
        decode(&bytes[..cut]),
        Err(CodecError::IntegrityMismatch) | Err(CodecError::Decode(_)) | Err(CodecError::Io(_))
    ));

    assert!(matches!(
        decode(&bytes[..52]),
        Err(CodecError::Truncated)
            | Err(CodecError::IntegrityMismatch)
            | Err(CodecError::Decode(_))
    ));

    assert!(matches!(decode(&bytes[..20]), Err(CodecError::Truncated)));
}

#[test]
fn a_schema_the_caller_does_not_speak_is_refused() {
    let mut bytes = encoded_graph();
    bytes[8..12].copy_from_slice(&(SCHEMA_VERSION + 1).to_le_bytes());
    assert!(
        matches!(
            decode(&bytes),
            Err(CodecError::SchemaMismatch { found, expected })
                if found == SCHEMA_VERSION + 1 && expected == SCHEMA_VERSION
        ),
        "a newer schema in the record is refused by default"
    );

    // Saying so up front is equivalent, and the message names the record's own version.
    let limits = DecodeLimits::for_schema(SCHEMA_VERSION + 1);
    assert!(matches!(
        decode_graph(&mut Cursor::new(&bytes), limits),
        Err(CodecError::SchemaMismatch { .. })
    ));
}

#[test]
fn an_oversized_payload_is_refused_before_it_is_inflated() {
    let bytes = encoded_graph();
    let limits = DecodeLimits {
        max_uncompressed_bytes: 8,
        expected_schema: SCHEMA_VERSION,
    };
    assert!(matches!(
        decode_graph(&mut Cursor::new(&bytes), limits),
        Err(CodecError::TooLarge { limit: 8, .. })
    ));
}

#[test]
fn a_large_graph_survives_the_round_trip() {
    let graph = synthetic(2_000, 4, 7);
    let mut bytes = Vec::new();
    let stats = encode_graph(&graph, &mut bytes).expect("a large graph encodes");

    let decoded = decode(&bytes).expect("a large graph decodes");
    assert_eq!(decoded.node_count(), graph.node_count());
    assert_eq!(decoded.edge_count(), graph.edge_count());
    assert_eq!(
        compare(&graph, &decoded, &CompareOptions::strict()),
        GraphDiffReport::default()
    );
    assert!(
        stats.bytes_compressed < stats.bytes_uncompressed,
        "zstd should shrink a repetitive synthetic graph: {} -> {}",
        stats.bytes_uncompressed,
        stats.bytes_compressed
    );
}

#[test]
fn the_wire_types_are_reachable_from_the_public_surface() {
    // `EdgeFlags`, `ContentHash`, `Language` and `RepoPath` all appear in the wire record; naming
    // them here keeps this file honest about what a consumer can construct.
    assert_ne!(EdgeFlags::EMPTY, EdgeFlags::ALL_BITS);
    assert_eq!(ContentHash::of(b"x"), ContentHash::of(b"x"));
    assert_eq!(Language::Typescript.as_str(), "typescript");
}

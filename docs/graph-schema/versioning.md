# Graph schema versioning and wire format

`codegraph::SCHEMA_VERSION` (currently `1`) is the one number every reader and writer of a
persisted graph agrees on. It is stored in `snapshots.graph_schema_version` and in every encoded
record. A reader refuses a record whose version differs from its own with
`CodecError::SchemaMismatch { found, expected }`, and the incremental layer turns that into a full
rebuild.

## When to bump `SCHEMA_VERSION`

Bump it when:

- an enum discriminant changes meaning, or a variant is removed (`NodeKind`, `EdgeKind`,
  `ResolvedBy`, `Provenance`, `EdgeFlags` bits);
- a wire field is added or removed, or a struct layout changes;
- the identity of an edge or a node changes (`(source, kind, target)` for edges, the `NodeId`
  schemes for nodes).

A new variant at an unused discriminant does not require a bump. It does require a
`LINKER_VERSION` bump if the linker starts emitting it. `LINKER_VERSION` (currently `1.0.0`)
changes the minor part when a confidence value changes and the major part when resolution rules
change; it is recorded in `snapshots.analyzer_versions["linker"]`.

`tests/wire_compat.rs` writes every persisted discriminant and spelling out literally, so a
renumbering fails a test before it reaches a database.

## Container format

A record is `header || zstd(bincode(payload))`.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `magic` = `RGGR` |
| 4 | 1 | `format` = 1 |
| 5 | 1 | `codec` = 1 (bincode) |
| 6 | 1 | `payload` = 1 (`Graph`) or 2 (`Delta`) |
| 7 | 1 | reserved, must be 0 |
| 8 | 4 | `schema_version`, little-endian |
| 12 | 8 | `uncompressed_len`, little-endian |
| 20 | 32 | `blake3` of the uncompressed payload |

The header is 52 bytes and is written field by field, never transmuted, so it does not depend on
endianness or alignment. `tests/codec_roundtrip.rs` pins each field.

The payload stores the string table, files, nodes, edges and unresolved references in the sorted
order the builder produces, not the CSR. Decoding goes back through `GraphBuilder`, so the format
does not depend on the in-memory layout. Encoding is deterministic: the same graph yields identical
bytes, and a graph or delta re-encodes byte for byte after a round trip.

zstd runs at level 3.

## Decoding untrusted input

Local snapshot files may be tampered with. `decode_*` checks, in order: magic, format, codec,
payload kind, the reserved byte, the schema version, and the declared length against
`DecodeLimits::max_uncompressed_bytes` (default 8 GiB) before inflating anything. The decompressed
stream is wrapped in a length limit, and the blake3 hash is verified before the graph is built. No
partial graph is ever returned.

| Error | Meaning |
|---|---|
| `BadMagic` | not a graph record |
| `UnsupportedFormat`, `UnsupportedCodec`, `UnsupportedPayload` | a header value this build does not know |
| `ReservedByte` | reserved byte is not zero |
| `SchemaMismatch { found, expected }` | written by a different schema; rebuild |
| `TooLarge` | declared or actual payload exceeds the limit |
| `IntegrityMismatch` | the hash does not match: corruption or tampering |
| `Truncated`, `LengthMismatch` | the record ends early or lies about its length |
| `Decode`, `Io`, `Graph` | payload could not be parsed or rebuilt |

## Known deviation: the codec crate

The codec is `bincode` 1.x. `cargo deny check advisories` reports `RUSTSEC-2025-0141` (the crate is
unmaintained). The advisory step in CI is report-only. Replacing the codec (`bincode` 2 or
`postcard`, both allowed by CG-011) is a workspace-level change because `analysis-ir` and
`graph-storage` also depend on `bincode`; it would change `codec` in the header to a new value and
would need a `SCHEMA_VERSION` bump.

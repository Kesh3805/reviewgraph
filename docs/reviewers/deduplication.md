# Deduplication

## Fingerprint v2 (DED-001)

`"v2:" + hex(blake3("v2\0{category}\0{anchor}\0{signature}")[..16])`.

- `anchor`: `anchor_symbol_key`, else `"path:" + anchor_path`; never a line number.
- `signature`: `p:{predicate.kind}\0{normalized subject}`; if the predicate is absent or `other`,
  `c:` plus the sorted, de-duplicated normalized claim tokens.
- Excluded by construction (`RootCauseInput` has no such fields): severity, confidence, reviewer,
  title, description, line numbers, evidence, run id.
- Versioning: `v1:` rows (DOM-006) stay valid in storage; `v2:` is the cross-reviewer key.
  `RootCauseFingerprint::from_str` rejects any prefix other than `v2:`.
- Known gap: claim normalization lowercases but does not apply full Unicode NFKC yet.

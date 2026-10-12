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

## Merge (DED-002)

Link rules, first match wins: R1 equal fingerprint; R2 same anchor symbol + predicate kind +
evidence-range Jaccard >= 0.5; R3 same anchor symbol + overlapping anchor ranges + claim-token
Jaccard >= 0.6. Groups are connected components. Primary tie-break: confidence, severity,
evidence count, lowest candidate id. Severity is never raised by merging.

## Re-review classification (DED-003)

persisting: identity in both runs. new: only in the current run. fixed: absent and the reviewer
succeeded for the cluster and the predicate no longer holds. not_reevaluated: absent with
degraded or skipped coverage (never reported as fixed). Dedup happens after the verdict.

## Priority (DED-004)

priority_v1 weights: severity 0.35, confidence 0.20, impact domain 0.15, public exposure 0.10,
change risk 0.08, blast radius 0.07, test gap 0.05. Blast radius saturates at 32 callers, so
caller count can move a score by at most 0.07 and never changes severity or band. No weight
above 0.40; blast radius weight at most 0.10.

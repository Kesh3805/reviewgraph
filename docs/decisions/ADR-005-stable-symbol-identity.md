# ADR-005 — Stable symbol identity

**Status:** Accepted · 2026-10-02

## Context
The prototype CodeGraph builds node IDs by hashing `file_path + qualified_name`. Moving a file therefore changes every ID in it. Line numbers cannot be used either. PRD §20 lists the identity components: repository, language, module, qualified name, kind and signature.

## Decision
- **Canonical ID.** `SymbolId = "{lang}:{module_path}#{qualified_name}/{kind}[~{n}]"`.
  - `module_path` is the repo-relative path without its extension.
  - `~n` is an ordinal, used only for overloads or duplicate names in the same scope.
  - Example: `ts:src/auth/auth.service#AuthService.authorize/method`.
- **Storage key.** `SymbolKey = hex(blake3(SymbolId)[..16])`.
- **The signature is an attribute, not part of identity.** If it were part of the ID, every parameter change would look like a delete plus an add. Modifications are classified with `signature_hash` and `body_hash` (normalized tokens).
- **Renames and moves.** `incremental::matcher` pairs removed and added symbols of the same kind, trying these rules in order:
  1. Identical `body_hash`.
  2. Identical `signature_hash` and same name, in a moved file.
  3. Token Jaccard similarity of at least 0.8.

  A match is recorded as `symbol_lineage(transition, similarity)`. Finding history and embeddings follow the lineage.
- **The repository is a namespace column,** not part of the ID string. This keeps IDs portable across forks and mirrors.

## Alternatives
| Option | Rejected because |
|---|---|
| Include the signature in the ID | The ID would change on ordinary edits. |
| Use only the qualified name, with no path | Collides across modules. |
| SCIP symbols | Requires a compiler indexer. Reconsider once the semantic provider is enabled. |

## Consequences
- The rename and move tests in `incremental` are required acceptance criteria.
- Two consumers must read the lineage: finding dedup across runs, and Qdrant point re-keying.

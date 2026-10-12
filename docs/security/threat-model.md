# Threat model

## Assets

Source code, installation tokens, GitHub App private key, tenant data, model prompts and outputs, audit log.

## Actors

External attacker, malicious tenant, compromised dependency, curious model provider, insider.

## Trust boundaries

1. GitHub to webhook endpoint.
2. Browser to API.
3. API to PostgreSQL and Redis.
4. Worker to GitHub, model providers, Qdrant and object store.
5. Analyzed repository code to the parser.

## Data flow

```
GitHub --webhook--> API --queue(PG)--> Worker --> GitHub / Model providers / Qdrant / Object store
Browser --> API --> PostgreSQL (RLS)
Repository content --> Parser (limits, sandboxed process)
```

## STRIDE table

| Id | STRIDE | Boundary | Threat | Mitigation | Test | Residual |
| --- | --- | --- | --- | --- | --- | --- |
| T-001 | Spoofing | 1 | Forged webhook | GH-002 | signature tests | L |
| T-002 | Replay | 1 | Replayed webhook delivery | SEC-006 | webhook-replay tests | L |
| T-003 | Information disclosure | 2, 3 | Cross-tenant data access | SEC-001, SEC-002 | isolation suite | L |
| T-004 | Information disclosure | 4 | Cross-tenant graph or vector leakage | SEC-002 | tenant-filter audit | L |
| T-005 | Information disclosure | 4 | Secrets sent to model provider | SEC-003, SEC-004 | redaction tests | M |
| T-006 | Information disclosure | 4 | Token exfiltration via crafted logs | OBS-006 | redaction corpus | L |
| T-007 | Information disclosure | 4 | Installation token or App key mishandled | SEC-005 | secret handling tests | L |
| T-008 | Tampering | 4 | Prompt injection from PR text or code comments | SEC-004 | verification stage tests | M |
| T-009 | Denial of service | 5 | Parser DoS via huge or deeply nested files | SEC-003 | limits tests | M |
| T-010 | Elevation of privilege | 5 | Path traversal or symlink escape in checkout | SEC-003 | checkout tests | L |
| T-011 | Repudiation | 2 | Unattributed administrative change | SEC-008 | audit log tests | L |
| T-012 | Information disclosure | 4 | Source retained beyond policy | SEC-007 | retention tests | L |
| T-013 | Information disclosure | 4 | Object store over-exposure | SEC-009 | access control tests | L |
| T-014 | Tampering | all | Supply-chain compromise | SEC-010 | scan and SBOM | M |
| T-015 | Spoofing | 4 | SSRF through provider base URLs | SEC-005 | URL allow-list tests | L |

Prompt injection mitigations: verification stages, no tool execution, structured output, no merge capability, evidence requirements.

## Residual risks and owners

Rows with residual M are reviewed each release by the security owner. This document is reviewed on each ADR that touches a boundary.

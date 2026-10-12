# Finding lifecycle (VER-001)

The single source of the transition table is `review_core::finding::state::ALLOWED`. SQL does not
duplicate it; the `state` CHECK only lists the legal values.

```
GENERATED --------> EVIDENCE_COLLECTED --> VERIFIED --> DEDUPLICATED --> PRIORITIZED --> PUBLISHED
   |  \                  |  \  \               |  \           |              |
   |   \                 |   \  \              |   \          |              |
   |    +--> REJECTED_INVALID <-+ (malformed)  |    +--> SUPPRESSED_DUPLICATE
   |                      |                    |
   +--> SUPPRESSED_{PREEXISTING,NOT_ACTIONABLE,POLICY}      EVIDENCE_COLLECTED also reaches
                          +--> SUPPRESSED_LOW_CONFIDENCE    SUPPRESSED_{LOW_CONFIDENCE,
                                                            PREEXISTING,NOT_ACTIONABLE,POLICY}
every non-terminal state --> INVALIDATED (head superseded)
```

Terminal: `PUBLISHED`, every `SUPPRESSED_*`, `REJECTED_INVALID`, `INVALIDATED`.

## REJECTED_INVALID

A PRD section 150 extension recorded in DOM-006. A reviewer candidate that fails normalization
(unresolvable anchor, malformed predicate) is neither suppressed by evidence nor valid. It is
persisted with a `reason_code` and no `suppression` object, and is reachable only from `GENERATED`
and `EVIDENCE_COLLECTED`. It counts in `suppressed_findings_total{reason}` and is evaluation data.

## Compare-and-set

Every state change is
`UPDATE candidate_findings SET state=$to, state_version=state_version+1, reason_code=$rc, ... WHERE id=$id AND state=$from AND state_version=$v`
plus an `INSERT INTO finding_state_transitions` in the same transaction. The loser of a race gets
`TransitionError::Conflict` and re-reads. A transition into a `SUPPRESSED_*` state or `INVALIDATED`
must also carry the `suppression` object (the table CHECK couples them); `Transition::validate`
enforces it before any SQL, together with edge legality.

A candidate invalidated mid-verification stays `INVALIDATED`: the later CAS from
`EVIDENCE_COLLECTED` finds a different `state_version` and fails.

## Supersession

`FindingStore::invalidate_run` locks the run's non-terminal candidates, moves them to
`INVALIDATED` with reason `HEAD_SUPERSEDED`, and inserts the audit rows with the state each
candidate left, all in one statement. It is idempotent.

## Idempotency keys

| Table | Key |
|---|---|
| `candidate_findings` | `idempotency_key` UNIQUE (`blake3(reviewer_run_id, ordinal, raw_output_hash)`), plus `(reviewer_run_id, fingerprint)` |
| `finding_verifications` | `(candidate_finding_id, verification_version)` |
| `verified_findings` | `candidate_finding_id` |

## Metrics

- `finding_state_transitions_total{from,to}`: committed transitions, including bulk invalidation.
- `finding_transition_conflicts_total`: CAS attempts lost to a concurrent writer.

## Security

RLS (ENABLE + FORCE, `tenant_isolation`) is on every table. The Postgres store sets
`app.organization_id` per transaction and repeats the `organization_id` predicate. `raw`,
`evidence` and `payload` can hold code quotes and are never logged.

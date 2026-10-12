# Convention mining

A convention is a repeated pattern that the repository follows on purpose. The miner framework
(`engine/crates/profile/src/conventions`) separates intentional convention from incidental
repetition.

## Statistics

For one miner, over its population of samples (not-applicable samples are dropped):

- `consistency = conforming / (conforming + violating)`
- `sample_factor = min(1, ln(1 + n) / ln(1 + 30))`
- `spread_factor = min(1, distinct_modules / 3)`
- `confidence = consistency x sample_factor x spread_factor`

A pattern seen in one module only gets a low spread factor, so it stays informational.

## Enforceable versus informational

A convention is enforceable only when `confidence >= 0.9` and `samples >= 10`. Anything else is
stored as informational and is never used to raise a finding on its own.

## Scope

Candidate scopes are tried from broadest to tightest: the source root (`**`), `src/modules/**`,
then one glob per module. The broadest scope whose consistency is at least 0.9 wins.

## Exceptions

A violating sample is recorded as an exception, not a violation, when:

- it carries a `// review-ignore: <convention-id>` annotation,
- its path matches a generated-file glob, or
- its path matches `conventions.exceptions` in `.review/config.yaml`.

## Versioning

Every miner declares a `version`. The set of miner ids and versions feeds `profile_version`, so
changing any miner invalidates cached profiles.

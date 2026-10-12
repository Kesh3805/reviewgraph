# Live smoke report

Copy to `<short-sha>.md`. `check-report.sh` requires a `Commit:` line, every box ticked and a
`Signed-off-by:` line.

Commit: <full or short sha tested>
Date: <YYYY-MM-DD>
Operator: <name>
Test repository: <disposable private repository>

## Versions

- Image digests: <api, worker, engine>
- verification_version: <value>
- Prompt versions: <values>
- Provider and model: <value>

## Checklist (MVP exit criteria 1 to 7)

- [ ] Webhook delivery verified (200 or 202 in the App delivery log)
- [ ] Only changed files reparsed (worker log counter value: <n>)
- [ ] Impact and context selected within budget
- [ ] Correctness and security reviewers ran
- [ ] Expected finding posted inline at the correct line, with a summary
- [ ] No APPROVE, REQUEST_CHANGES or merge action appeared on the pull request
- [ ] Full run is one trace in OpenObserve (trace id: <id>)
- [ ] Token usage and cost recorded and under the cap
- [ ] Check run state correct
- [ ] Second commit superseded the first run with no duplicate comments
- [ ] Captured logs grepped for key prefixes: no secret visible
- [ ] Key revoked and App uninstalled from the test repository

## Measurements

- Tokens (input / output): <n / n>
- Cost: <amount> (cap: <amount>)
- End-to-end latency for the small pull request: <seconds> (target under 60 s)

## Deviations and follow-up task IDs

<none, or list>

Signed-off-by: <name, date, commit SHA tested>

# Live smoke procedure (E2E-002)

A manual release gate. E2E-001 uses a fake GitHub and a replay model, so it cannot detect real API
drift, permission gaps, real webhook delivery or real model behavior. This procedure is run by a
human against a real repository before the first release. It is never automated in CI.

## Prerequisites (human, outside the repository)

- A disposable private test repository containing the fixture project.
- A GitHub App created from `infra/github/app-manifest.json` with exactly `contents: read`,
  `pull_requests: write`, `checks: write`, `metadata: read`, subscribed to pull request events,
  installed on the test repository only. No contents write, hence no merge ability.
- A public tunnel (for example ngrok) to the local API.
- An Anthropic or OpenAI key with a spend cap.
- The stack running via `pnpm dev:up`.
- Secrets in a local untracked `.env.live` (never committed): `GITHUB_APP_ID`,
  `GITHUB_APP_PRIVATE_KEY` or `GITHUB_APP_PRIVATE_KEY_FILE`, `GITHUB_WEBHOOK_SECRET`,
  `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`, `LIVE_SMOKE_TUNNEL_URL`.

## Procedure

1. Run `scripts/live-smoke/preflight.sh` (or `preflight.ps1`). It prints `set` or `missing` per
   variable, never a value, checks the tunnel and makes a one-token model call.
2. Open a pull request on a new branch that reproduces the auth-bypass scenario.
3. Observe the webhook delivery in the App delivery log (200 or 202).
4. Wait for the review.
5. Verify the checklist in `live-smoke-reports/TEMPLATE.md` (mapped to MVP exit criteria 1 to 7).
6. Push a second commit; confirm supersession and no duplicate comments.
7. Close the pull request.
8. Revoke the key and uninstall the App from the test repository.

Grep captured logs for key prefixes before attaching anything; scrub as needed.

## Pass criteria

Every checklist item ticked, no secret visible in logs, cost under the cap stated in the report,
and the report signed with date, commit SHA tested and the person's name. A failed step produces a
defect task before the gate can be signed; the report records the failure rather than editing it.

## Release gate

Copy the template to `docs/operations/live-smoke-reports/<short-sha>.md`, fill it in, commit it,
then run `scripts/live-smoke/check-report.sh <release-commit>`. It passes only for a signed report
with every box ticked for that commit, or a descendant within 30 days with no `engine/` changes.

Script tests: `bash scripts/live-smoke/tests/live-smoke.test.sh`.

## Teardown

Revoke the model key, uninstall the App, delete the test repository branch and pull request, stop
the tunnel.

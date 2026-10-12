# Container images

These Dockerfiles exist for the CI image build (`.github/workflows/docker.yml`). Local development
is native and never requires Docker: see `docs/operations/local-development.md`.

| Image  | Dockerfile          | Container port | Contents                                                |
| ------ | ------------------- | -------------- | ------------------------------------------------------- |
| engine | `engine.Dockerfile` | none           | `review`, `review-worker`, `review-engine`, git, CA set |
| api    | `api.Dockerfile`    | 3000           | NestJS control plane (`dist/` plus production deps)     |
| web    | `web.Dockerfile`    | 3001           | Next.js standalone server                               |

## Entrypoints

The engine image has one entrypoint (`rg-entrypoint`, which `exec`s its arguments), so the binary is
chosen by the command: `docker run <image> review-worker migrate`, `review --version`. The default
command is `review-worker --help`. The api image runs `node --import ./dist/telemetry/instrumentation.js dist/main.js`; the web image runs `node apps/web/server.js`.

## Build arguments and labels

| Argument         | Default         | Meaning                                              |
| ---------------- | --------------- | ---------------------------------------------------- |
| `GIT_SHA`        | `unknown`       | Becomes `org.opencontainers.image.revision`          |
| `BUILD_TIME`     | `unknown`       | Becomes `org.opencontainers.image.created`           |
| `NODE_VERSION`   | `24`            | Node base image major (api, web)                     |
| `DEBIAN_RELEASE` | `bookworm`      | Debian release of the base images                    |

## Hardening contract (checked in CI)

- Runs as a numeric non-root user (`10001:10001`).
- Base images carry an explicit tag (never `latest`); `scripts/ci/image-hardening-check.sh` fails otherwise.
- Starts with `--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true`
  (`scripts/ci/image-smoke.sh`).
- No secret-like patterns in `docker history`; size limits engine 400 MB, api 300 MB, web 250 MB
  (override with `RG_IMAGE_LIMIT_MB` when tuning).
- Trivy finds no unwaived fixed HIGH or CRITICAL vulnerability; waivers live in
  `security/exceptions.toml` and are turned into a `.trivyignore` by
  `scripts/security/exceptions-to-trivyignore.mjs`. A CycloneDX SBOM from syft is uploaded as the
  `sbom-<image>` artifact.

Pinning bases by digest is a follow-up (a reviewed PR updates the tags and digests together).

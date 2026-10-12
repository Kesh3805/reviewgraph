# Engine image: review (CLI), review-worker and review-engine in one image; pass the binary as the
# command, for example `docker run <image> review-worker migrate`.
# Built by CI only (.github/workflows/docker.yml); local development does not use Docker.
ARG DEBIAN_RELEASE=bookworm

# engine/rust-toolchain.toml selects the pinned channel; rustup installs it on the first cargo call.
FROM rust:1-${DEBIAN_RELEASE} AS build
WORKDIR /src
COPY engine ./engine
# review-core embeds docs/graph-schema/symbol-id.md at compile time.
COPY docs/graph-schema ./docs/graph-schema
WORKDIR /src/engine
RUN cargo build --release --locked -p review-cli -p review-worker -p review-engine

FROM debian:${DEBIAN_RELEASE}-slim AS runtime
ARG GIT_SHA=unknown
ARG BUILD_TIME=unknown
LABEL org.opencontainers.image.title="reviewgraph-engine" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.created="${BUILD_TIME}" \
      org.opencontainers.image.source="https://github.com/Kesh3805/reviewgraph"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/engine/target/release/review /usr/local/bin/review
COPY --from=build /src/engine/target/release/review-worker /usr/local/bin/review-worker
COPY --from=build /src/engine/target/release/review-engine /usr/local/bin/review-engine
RUN printf '#!/bin/sh\nexec "$@"\n' > /usr/local/bin/rg-entrypoint && chmod 0755 /usr/local/bin/rg-entrypoint
USER 10001:10001
WORKDIR /tmp
ENTRYPOINT ["/usr/local/bin/rg-entrypoint"]
CMD ["review-worker", "--help"]

# Engine build/test image. The Windows host toolchain cannot compile C deps
# (tree-sitter) or link windows-sys (tokio/sqlx/reqwest) — see ADR-001.
FROM rust:1-bookworm
RUN rustup toolchain install 1.97 --profile minimal --component rustfmt,clippy \
 && rustup default 1.97
RUN apt-get update && apt-get install -y --no-install-recommends git postgresql-client nodejs npm \
 && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-deny --locked --version ^0.18 || true
ENV CARGO_TERM_COLOR=always
WORKDIR /repo/engine

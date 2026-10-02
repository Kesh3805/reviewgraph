# Optional convenience wrapper; every target delegates to a pnpm script.
.PHONY: up down reset test test-rust test-ts test-integration lint fmt migrate
up: ; pnpm dev:up
down: ; pnpm dev:down
reset: ; pnpm dev:reset
test: ; pnpm test
test-rust: ; pnpm test:rust
test-ts: ; pnpm test:ts
test-integration: ; pnpm test:integration
lint: ; pnpm lint
fmt: ; pnpm fmt
migrate: ; pnpm migrate

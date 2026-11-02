# Trading Center developer and operator commands. `make help` lists targets.
SHELL := /usr/bin/env bash
.SHELLFLAGS := -euo pipefail -c
WEB := apps/web

.PHONY: help check fmt lint test web-check db-test dev dev-db build up down logs ps preview audit \
        setup-token db-inspect backup migrate

help: ## List targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  %-12s %s\n",$$1,$$2}'

check: fmt lint test web-check ## Full local quality gate (no DB tests; see db-test)

fmt: ## rustfmt check
	cargo fmt --all -- --check

lint: ## clippy, warnings are errors
	cargo clippy --workspace --all-targets --locked -- -D warnings

test: ## Rust unit + mocked integration tests (never reach live brokers)
	env -u TC_LIVE_TRADING cargo test --workspace --locked

web-check: ## Web typecheck, lint and static export build
	@if [[ -f $(WEB)/package.json ]]; then \
	  cd $(WEB) && npm ci --no-audit --no-fund && npm run typecheck && npm run lint && npm run build; \
	else echo "skip web-check: $(WEB)/package.json missing"; fi

db-test: ## MySQL integration tests in a throwaway mysql:8.4 container
	deploy/scripts/db-test.sh

dev-db: ## Start local dev MySQL on 127.0.0.1:3307
	deploy/scripts/dev-db.sh

dev: dev-db ## Run api+engine in one process against the dev DB (live trading disarmed)
	env -u TC_LIVE_TRADING TC_DATABASE_URL_FILE=$(CURDIR)/target/dev/database_url \
	  TC_RP_ID=localhost TC_RP_ORIGIN=http://localhost:3000 TC_BIND=127.0.0.1:8080 \
	  cargo run -p tc-server -- --role all

build: ## Build server and web images
	docker compose build

migrate: ## Apply database migrations (one-shot service)
	docker compose run --rm migrate

up: ## Start the production stack
	docker compose up -d

down: ## Stop the stack (data volumes are kept)
	docker compose down

ps: ## Service status and health
	docker compose ps

logs: ## Follow logs
	docker compose logs -f --tail=200

setup-token: ## Print a one-time first-passkey setup link
	docker compose exec api /usr/local/bin/tc setup-token

db-inspect: ## Describe the existing database schema (read-only)
	docker compose run --rm migrate /usr/local/bin/tc db-inspect

backup: ## Encrypted backup now
	deploy/scripts/backup.sh

preview: ## Render the single UI preview PNG
	node scripts/preview/render.mjs

audit: ## Dependency audits
	@if command -v cargo-audit >/dev/null; then cargo audit; else echo "cargo-audit not installed: cargo install cargo-audit --locked"; fi
	@if [[ -f $(WEB)/package-lock.json ]]; then cd $(WEB) && npm audit --omit=dev --audit-level=high; fi

INSTALL_DIR ?= $(HOME)/.local/bin
BINARIES := mika mika-spirit mika-gateway

.PHONY: build build-dashboard deploy stop restart install install-permission-policy-plugin test-permission-policy-plugin test test-async-db-saturation test-dispatch-symmetry verify-bundled-skills lint fmt check check-ngrok deploy-info clean help calibrate-mika-dev calibrate-mika-arch calibrate-mika-qa calibrate-mika-orchestrator

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

build: ## Build release binaries with telemetry
	cargo build --release --features telemetry

build-debug: ## Build debug binaries
	cargo build

build-dashboard: ## Build dashboard for production (installs deps + builds shared UI lib)
	npm ci
	npm run build -w packages/ui
	npm run build --prefix dashboard

stop: ## Stop running mika-spirit and mika-gateway (via OpenRC)
	@for bin in mika-spirit mika-gateway; do \
		echo "Stopping $$bin..."; \
		sudo rc-service "$$bin" stop || true; \
	done

restart: ## Restart mika-spirit and mika-gateway (via OpenRC)
	@for bin in mika-spirit mika-gateway; do \
		echo "Restarting $$bin..."; \
		sudo rc-service "$$bin" restart || true; \
	done

install: ## Copy release binaries + scripts to INSTALL_DIR (safe while services run)
	@mkdir -p $(INSTALL_DIR)
	@for bin in $(BINARIES); do \
		if [ ! -f target/release/$$bin ]; then \
			echo "ERROR: target/release/$$bin not found. Run 'make build' first." >&2; \
			exit 1; \
		fi; \
		cp target/release/$$bin $(INSTALL_DIR)/$$bin.tmp; \
		mv $(INSTALL_DIR)/$$bin.tmp $(INSTALL_DIR)/$$bin; \
		if [ "$$(uname)" = "Darwin" ]; then codesign -s - $(INSTALL_DIR)/$$bin; fi; \
		echo "Installed $$bin -> $(INSTALL_DIR)/$$bin"; \
	done
	@# Ship the pilot egress proxy alongside the binaries so dispatch-lib.sh
	@# can find it on PATH (bwrap Phase 2b — network cut + hostname allowlist).
	@cp scripts/mika-pilot-egress-proxy $(INSTALL_DIR)/mika-pilot-egress-proxy.tmp
	@chmod +x $(INSTALL_DIR)/mika-pilot-egress-proxy.tmp
	@mv $(INSTALL_DIR)/mika-pilot-egress-proxy.tmp $(INSTALL_DIR)/mika-pilot-egress-proxy
	@echo "Installed mika-pilot-egress-proxy -> $(INSTALL_DIR)/mika-pilot-egress-proxy"
	@# addon deployed at stable path (2026-08-05).
	@cp scripts/mika-pilot-anthropic-auth-addon.py $(INSTALL_DIR)/mika-pilot-anthropic-auth-addon.py.tmp
	@mv $(INSTALL_DIR)/mika-pilot-anthropic-auth-addon.py.tmp $(INSTALL_DIR)/mika-pilot-anthropic-auth-addon.py
	@echo "Installed mika-pilot-anthropic-auth-addon.py"

deploy: deploy-info build-dashboard build install restart check-ngrok ## Full deploy: build, install, restart

install-permission-policy-plugin: ## Install mika-permission-policy plugin into claude-pilot's uv tool env (mika#1817)
	@# The plugin lives in tools/mika_permission_policy/ and provides the
	@# per-binary safety functions loaded by claude-pilot's per-spawn evaluator
	@# when MIKA_PERMISSION_POLICY_MODE=per_spawn +
	@# MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy are set.
	@#
	@# `--with-editable` injects the plugin into the same uv tool env as
	@# claude-pilot so the running interpreter can `import mika_permission_policy`.
	@# `--reinstall --force` matches the discipline in the meta-repo
	@# CLAUDE.md deploy target for claude-pilot itself.
	uv tool install --reinstall --force --editable ../claude-pilot \
		--with-editable ./tools/mika_permission_policy

test-permission-policy-plugin: ## Run the permission-policy plugin test suite (mika#1817)
	cd tools/mika_permission_policy && uv sync --all-extras --quiet && uv run pytest -q

check-ngrok: ## Warn if ngrok is not running (Telegram webhooks need it)
	@if ! curl -sf http://localhost:4040/api/tunnels > /dev/null 2>&1; then \
		echo ""; \
		echo "  ⚠  WARNING: ngrok is not running!"; \
		echo "  Telegram webhooks will not reach the gateway."; \
		echo "  Start ngrok: ngrok http 8080"; \
		echo ""; \
	fi

deploy-info: ## Print built SHA and warn if local HEAD is behind origin/main
	@BRANCH=$$(git rev-parse --abbrev-ref HEAD); \
	if [ "$$BRANCH" != "main" ]; then \
	  if [ "$${FORCE_DEPLOY_FROM_BRANCH:-}" = "1" ]; then \
	    printf "\033[1;33mWARN: deploying from '%s' (FORCE_DEPLOY_FROM_BRANCH=1)\033[0m\n" "$$BRANCH" >&2; \
	  else \
	    printf "\033[1;31mABORT: on '%s', not main.\033[0m\n" "$$BRANCH" >&2; \
	    echo "  Fix:      git checkout main && git pull --ff-only" >&2; \
	    echo "  Override: FORCE_DEPLOY_FROM_BRANCH=1 make deploy" >&2; \
	    echo "  Canonical: cd ../mika-platform && make deploy" >&2; \
	    exit 1; \
	  fi; \
	fi
	@echo "Building from: $$(git rev-parse --abbrev-ref HEAD) @ $$(git rev-parse --short HEAD) ($$(git log -1 --pretty=format:'%s'))"
	@if git fetch -q origin main 2>/dev/null; then \
	  AHEAD=$$(git rev-list --count HEAD..origin/main 2>/dev/null || echo 0); \
	  if [ "$$AHEAD" -gt 0 ]; then \
	    echo "WARNING: HEAD is $$AHEAD commits behind origin/main. Run 'git pull --ff-only' if you intended to deploy origin/main."; \
	  else \
	    echo "origin/main: up to date"; \
	  fi; \
	else \
	  echo "NOTE: could not reach origin (network/auth) — skipping freshness check."; \
	fi

calibrate-mika-dev: ## Pre-swap calibration gate for mika-dev (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then echo "Error: MODEL is required. Example: make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6" >&2; exit 1; fi
	cargo run --bin calibrate --release -- --role mika-dev --model "$(MODEL)" --baseline docs/eval/calibration/baselines/latest.json

calibrate-mika-arch: ## Pre-swap calibration gate for mika-arch (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then echo "Error: MODEL is required. Example: make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6" >&2; exit 1; fi
	cargo run --bin calibrate --release -- --role mika-arch --model "$(MODEL)" --baseline docs/eval/calibration/baselines/latest.json

calibrate-mika-qa: ## Pre-swap calibration gate for mika-qa (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then echo "Error: MODEL is required. Example: make calibrate-mika-qa MODEL=anthropic/claude-sonnet-4-6" >&2; exit 1; fi
	cargo run --bin calibrate --release -- --role mika-qa --model "$(MODEL)" --baseline docs/eval/calibration/baselines/latest.json

calibrate-mika-orchestrator: ## Pre-swap calibration gate for mika-orchestrator (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then echo "Error: MODEL is required. Example: make calibrate-mika-orchestrator MODEL=anthropic/claude-sonnet-4-6" >&2; exit 1; fi
	cargo run --bin calibrate --release -- --role mika-orchestrator --model "$(MODEL)" --baseline docs/eval/calibration/baselines/latest.json

test: ## Run all tests
	cargo test
	@bash scripts/test-dispatch-symmetry.sh
	@bash scripts/deploy-info-test.sh

test-async-db-saturation: ## Run async DB channel saturation regression test (mika#1258)
	cargo test -p mika-agent --lib -- async_db::tests::test_async_db_saturated_channel_does_not_pin_workers --nocapture

test-dispatch-symmetry: ## Verify dev-pilot and dev-groom handlers are structurally symmetric (mika#893 R5)
	@bash scripts/test-dispatch-symmetry.sh

verify-bundled-skills: ## Verify structural invariants on bundled skills — pre-merge counterpart to AC2 (mika#1575)
	cargo run -q --bin verify-bundled-skills

lint: ## Run clippy
	cargo clippy

fmt: ## Format code
	cargo fmt

check: fmt lint test ## Format, lint, and test

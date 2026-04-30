INSTALL_DIR ?= $(HOME)/.local/bin
BINARIES := mika mika-server mika-gateway

.PHONY: build build-dashboard deploy stop restart install test lint fmt check check-ngrok clean help

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

stop: ## Stop running mika-server and mika-gateway (via OpenRC)
	@for bin in mika-server mika-gateway; do \
		echo "Stopping $$bin..."; \
		sudo rc-service "$$bin" stop || true; \
	done

restart: ## Restart mika-server and mika-gateway (via OpenRC)
	@for bin in mika-server mika-gateway; do \
		echo "Restarting $$bin..."; \
		sudo rc-service "$$bin" restart || true; \
	done

install: ## Copy release binaries to INSTALL_DIR
	@mkdir -p $(INSTALL_DIR)
	@for bin in $(BINARIES); do \
		if [ ! -f target/release/$$bin ]; then \
			echo "ERROR: target/release/$$bin not found. Run 'make build' first." >&2; \
			exit 1; \
		fi; \
		cp target/release/$$bin $(INSTALL_DIR)/$$bin; \
		if [ "$$(uname)" = "Darwin" ]; then codesign -s - $(INSTALL_DIR)/$$bin; fi; \
		echo "Installed $$bin -> $(INSTALL_DIR)/$$bin"; \
	done

deploy: build-dashboard build stop install restart check-ngrok ## Build dashboard + binaries, stop, install, and restart

check-ngrok: ## Warn if ngrok is not running (Telegram webhooks need it)
	@if ! curl -sf http://localhost:4040/api/tunnels > /dev/null 2>&1; then \
		echo ""; \
		echo "  ⚠  WARNING: ngrok is not running!"; \
		echo "  Telegram webhooks will not reach the gateway."; \
		echo "  Start ngrok: ngrok http 8080"; \
		echo ""; \
	fi

test: ## Run all tests
	cargo test
	@bash scripts/test-dispatch-symmetry.sh

test-dispatch-symmetry: ## Verify dev-pilot and dev-groom handlers are structurally symmetric (mika#893 R5)
	@bash scripts/test-dispatch-symmetry.sh

lint: ## Run clippy
	cargo clippy

fmt: ## Format code
	cargo fmt

check: fmt lint test ## Format, lint, and test

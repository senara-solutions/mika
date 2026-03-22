INSTALL_DIR ?= $(HOME)/.local/bin
BINARIES := mika mika-server mika-gateway

.PHONY: build build-dashboard deploy stop restart install test lint fmt check clean help

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

build: ## Build release binaries with telemetry
	cargo build --release --features telemetry

build-debug: ## Build debug binaries
	cargo build

build-dashboard: ## Build dashboard for production
	npm run build --prefix dashboard

stop: ## Stop running mika-server and mika-gateway (via tmux C-c)
	@for bin in mika-server mika-gateway; do \
		if tmux has-session -t "$$bin" 2>/dev/null; then \
			echo "Stopping $$bin (tmux session)..."; \
			tmux send-keys -t "$$bin" C-c; \
			for i in $$(seq 1 10); do \
				pgrep -x "$$bin" > /dev/null 2>&1 || break; \
				sleep 0.5; \
			done; \
			if pgrep -x "$$bin" > /dev/null 2>&1; then \
				echo "  Force-killing $$bin..."; \
				pkill -KILL -x "$$bin" || true; \
			fi; \
		elif pgrep -x "$$bin" > /dev/null 2>&1; then \
			echo "Stopping $$bin (pkill)..."; \
			pkill -TERM -x "$$bin" || true; \
			for i in $$(seq 1 10); do \
				pgrep -x "$$bin" > /dev/null 2>&1 || break; \
				sleep 0.5; \
			done; \
			if pgrep -x "$$bin" > /dev/null 2>&1; then \
				echo "  Force-killing $$bin..."; \
				pkill -KILL -x "$$bin" || true; \
			fi; \
		fi; \
	done

restart: ## Restart mika-server and mika-gateway in their tmux sessions
	@for bin in mika-server mika-gateway; do \
		if tmux has-session -t "$$bin" 2>/dev/null; then \
			echo "Restarting $$bin..."; \
			tmux send-keys -t "$$bin" "$$bin" Enter; \
		else \
			echo "Warning: tmux session '$$bin' not found — start it manually"; \
		fi; \
	done

install: ## Copy release binaries to INSTALL_DIR
	@mkdir -p $(INSTALL_DIR)
	@for bin in $(BINARIES); do \
		if [ ! -f target/release/$$bin ]; then \
			echo "ERROR: target/release/$$bin not found. Run 'make build' first." >&2; \
			exit 1; \
		fi; \
		cp target/release/$$bin $(INSTALL_DIR)/$$bin; \
		echo "Installed $$bin -> $(INSTALL_DIR)/$$bin"; \
	done

deploy: build-dashboard build stop install restart ## Build dashboard + binaries, stop, install, and restart

test: ## Run all tests
	cargo test

lint: ## Run clippy
	cargo clippy

fmt: ## Format code
	cargo fmt

check: fmt lint test ## Format, lint, and test

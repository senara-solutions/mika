---
type: chore
ticket: mika#1051
branch: chore/1051/makefile-decompose-deploy-chain-into
source: mika-platform/docs/brainstorms/2026-05-09-lifecycle-redesign-brainstorm.md (Rec 6)
---

# Plan: Decompose `make deploy` into build/install/restart verbs

## Context

`mika/Makefile`'s `deploy` target currently chains: `build-dashboard build stop install restart check-ngrok`. This couples artifact production to service interruption — there is no way to prepare binaries without flipping the runtime.

Per the lifecycle-redesign brainstorm Rec 6, decomposing the verbs is the **necessary precondition** for honoring quiescent-boundary deploy discipline (Recs 3 and 5 depend on it).

## Pinned Makefile targets (at HEAD: `98c920e5`)

Verbatim from `mika/Makefile` — these are the targets this plan modifies or depends on:

```makefile
# Line 44
deploy: build-dashboard build stop install restart check-ngrok ## Build dashboard + binaries, stop, install, and restart

# Lines 20-24
stop: ## Stop running mika-server and mika-gateway (via OpenRC)
	@for bin in mika-server mika-gateway; do \
		echo "Stopping $$bin..."; \
		sudo rc-service "$$bin" stop || true; \
	done

# Lines 26-30
restart: ## Restart mika-server and mika-gateway (via OpenRC)
	@for bin in mika-server mika-gateway; do \
		echo "Restarting $$bin..."; \
		sudo rc-service "$$bin" restart || true; \
	done

# Lines 32-42
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
```

**`restart` fully subsumes `stop`:** Both targets iterate over the same two services (`mika-server mika-gateway`) in the same order. `stop` calls `rc-service stop`; `restart` calls `rc-service restart` (which is stop+start). Neither target has additional wait logic, cleanup steps, PID file removal, or socket teardown. The `|| true` error suppression is identical in both. Removing `stop` from the deploy chain loses nothing that `restart` does not already provide.

## Current state

The component targets already exist:

| Target | Current behavior | Service interruption? |
|--------|-----------------|----------------------|
| `build` | `cargo build --release --features telemetry` | No |
| `build-dashboard` | `npm ci && npm run build` | No |
| `install` | `cp target/release/$bin $(INSTALL_DIR)/$bin` | No |
| `stop` | `rc-service $bin stop` | **Yes** |
| `restart` | `rc-service $bin restart` | **Yes** |
| `deploy` | `build-dashboard build stop install restart check-ngrok` | **Yes** (chains stop+restart) |

The issue: `deploy` calls `stop` before `install`, then `restart` after. The `stop` exists to avoid `cp` overwriting a running binary. On Linux, `cp` to a running executable triggers `ETXTBSY` because the kernel denies write access to files currently being executed. This means `install` currently **cannot** be invoked standalone while services are running.

## Changes

### 1. Make `install` safe to run while services are running

Change the copy strategy from plain `cp` (which fails with ETXTBSY on running binaries) to atomic replacement via `mv`:

```makefile
install: ## Copy release binaries to INSTALL_DIR (safe while services run)
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
```

The `cp` to `.tmp` + `mv` pattern works because:
- `cp` to a new file (`.tmp`) never hits ETXTBSY — the file doesn't exist or isn't being executed.
- `mv` (rename) is atomic on the same filesystem and replaces the inode. The old process keeps running from the old inode (still in memory). The new file is picked up on next exec (i.e., restart).
- Darwin `codesign` still runs after `mv` — the final path is correct.
- Assumes `INSTALL_DIR` and `target/release/` are on the same filesystem for atomic `mv`. Cross-filesystem `mv` degrades to copy+delete (same behavior as current `cp`).

### 2. Remove `stop` from `deploy` chain

```makefile
deploy: build-dashboard build install restart check-ngrok ## Full deploy: build, install, restart
```

`stop` is no longer needed before `install` because the atomic copy pattern handles running binaries. `restart` (which is `rc-service restart`, i.e., stop+start) handles the service flip.

The standalone `stop` target stays in the Makefile for operators who need it (e.g., debugging, manual maintenance).

### 3. Update help comments

- `deploy` comment changes from "Build dashboard + binaries, stop, install, and restart" to "Full deploy: build, install, restart"
- `install` comment changes from "Copy release binaries to INSTALL_DIR" to "Copy release binaries to INSTALL_DIR (safe while services run)"

### 4. Update in-repo documentation

- `Makefile` line in `CLAUDE.md` → update the `make deploy` description to reflect the decomposed verbs.
- `mika-platform/docs/runbooks/deploy-protocol.md` line 278 explicitly references the deploy chain ordering: `deploy: build-dashboard build stop install restart check-ngrok`. Update to reflect the new chain: `deploy: build-dashboard build install restart check-ngrok`.

## What does NOT change

- `build`, `build-dashboard`, `stop`, `restart`, `check-ngrok` — all keep their current implementations.
- The `.PHONY` declaration already lists all relevant targets.
- `build-debug` — unchanged, not part of the deploy chain.
- `test`, `lint`, `fmt`, `check` — unrelated.

## Cross-repo companion

The ticket mentions a companion issue on `senara-solutions/mika-platform` for the meta-repo Makefile (`deploy: deploy-mika deploy-claude-pilot deploy-skills`). That is separate from this plan — the meta-repo change delegates to per-sub-repo verbs and lands second. This plan covers only `mika/Makefile`.

## Acceptance criteria (from ticket, refined)

1. `make deploy` continues to work end-to-end with no observable behavior change for the operator who runs the full pipeline.
2. `make build` and `make install` each invokable independently — `make build && make install` works while services are running (no ETXTBSY, no service interruption).
3. `make restart` invokable independently (already true today).
4. Operator-cancel-cleanly-first discipline from `docs/runbooks/deploy-protocol.md` § 5 still holds (unchanged — `restart` still does `rc-service restart`).

## Risk assessment

**Low risk.** The change surface is ~10 lines of Makefile. The atomic copy pattern (`cp` to `.tmp` + `mv`) is the standard safe-replacement pattern on Unix. The deploy chain remains a superset of the individual verbs — no behavior change for operators who run `make deploy`.

**One edge case:** if `INSTALL_DIR` is on a different filesystem from `target/release/`, the `mv` becomes a copy+delete rather than an atomic rename. This is unlikely in practice (`~/.local/bin/` and `target/release/` are typically on the same filesystem) and still safe — the window where the old binary is unlinked but the new one isn't yet in place is sub-millisecond for a ~50MB binary.

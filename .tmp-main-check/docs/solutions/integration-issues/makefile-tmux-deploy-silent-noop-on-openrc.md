---
title: Makefile tmux-based deploy silently fails to restart on OpenRC hosts
date: 2026-04-14
category: integration-issues
module: makefile-deploy
problem_type: integration_issue
component: tooling
symptoms:
  - "make deploy builds and installs new binary but old process keeps running"
  - "mika-qa ran with stale skill registry for hours after skills-only deploy"
  - "restart target prints warning and does nothing without tmux sessions"
root_cause: config_error
resolution_type: config_change
severity: high
tags:
  - makefile
  - deploy
  - tmux
  - openrc
  - rc-service
  - restart
  - gentoo
---

# Makefile tmux-based deploy silently fails to restart on OpenRC hosts

## Problem

`make deploy` built and installed new binaries but never restarted the running services on the Gentoo production host. The old process continued running with stale code, causing mika-qa to use an outdated skill registry for hours after a skills-only fix was deployed.

## Symptoms

- `make deploy` completes without errors but the old binary stays running
- `restart` target prints "Warning: tmux session 'mika-spirit' not found" and exits without restarting
- Skills-only changes have no effect until manual service restart
- No error reported in the deploy chain -- the failure is silent

## What Didn't Work

- The tmux-based `restart` target had **no fallback path**. If no tmux session existed (which is always the case on OpenRC hosts), it printed a warning and did nothing. Unlike the `stop` target which had a `pkill` fallback, `restart` was tmux-only.

## Solution

Replaced both `stop` and `restart` Makefile targets with OpenRC `rc-service` commands:

**Before (stop -- 26 lines of tmux + pkill logic):**
```makefile
stop: ## Stop running mika-spirit and mika-gateway (via tmux C-c)
	@for bin in mika-spirit mika-gateway; do \
		if tmux has-session -t "$$bin" 2>/dev/null; then \
			# ... tmux C-c with 5s poll + SIGKILL fallback
		elif pgrep -x "$$bin" > /dev/null 2>&1; then \
			# ... pkill -TERM with 5s poll + SIGKILL fallback
		fi; \
	done
```

**After (stop -- 4 lines):**
```makefile
stop: ## Stop running mika-spirit and mika-gateway (via OpenRC)
	@for bin in mika-spirit mika-gateway; do \
		echo "Stopping $$bin..."; \
		sudo rc-service "$$bin" stop || true; \
	done
```

**Before (restart -- 8 lines, tmux-only, no fallback):**
```makefile
restart: ## Restart mika-spirit and mika-gateway in their tmux sessions
	@for bin in mika-spirit mika-gateway; do \
		if tmux has-session -t "$$bin" 2>/dev/null; then \
			tmux send-keys -t "$$bin" "$$bin" Enter; \
		else \
			echo "Warning: tmux session '$$bin' not found"; \
		fi; \
	done
```

**After (restart -- 4 lines):**
```makefile
restart: ## Restart mika-spirit and mika-gateway (via OpenRC)
	@for bin in mika-spirit mika-gateway; do \
		echo "Restarting $$bin..."; \
		sudo rc-service "$$bin" restart || true; \
	done
```

Both targets use `|| true` so that a failure on one service (e.g., not registered) doesn't prevent the other service from being processed.

## Why This Works

The production host runs Gentoo with OpenRC and `supervise-daemon` -- there are no tmux sessions. `rc-service` is the correct interface for the actual service manager. The deploy user already has sudoers entries for `rc-service mika-spirit` and `rc-service mika-gateway`, so no privilege escalation changes were needed.

The `deploy` target chain (`build-dashboard build stop install restart check-ngrok`) is unchanged -- only the implementations of `stop` and `restart` changed, making this a drop-in replacement.

## Prevention

- When the service management approach changes (e.g., from manual process management to a proper init system), update the deploy tooling in the same change. Don't leave a split where binaries are managed by one system and deployment scripts assume another.

## Related Issues

- [GitHub Issue #505](https://github.com/senara-solutions/mika/issues/505)

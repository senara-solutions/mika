---
title: "Debug log secret leakage and world-readable file permissions in claude-pilot transport"
category: security
date: 2026-03-17
tags:
  - secret-leakage
  - log-sanitization
  - file-permissions
  - defense-in-depth
severity: P1
components:
  - claude-pilot/transport
  - claude-pilot/logger
  - mika-dev/skills/self-dev
symptoms:
  - Full PilotEvent payloads (including tool_input with inline secrets) written to log files unconditionally when --log-dir is active
  - Log directories created with 0755 and log files with 0644 permissions, readable by any local user
  - system_prompt.md references nonexistent task_id and session_id fields in PilotEvent interface
---

# Debug Log Secret Leakage and World-Readable File Permissions

## Problem

A debug logging line in claude-pilot's `transport.ts` wrote the full `PilotEvent` JSON to the file log on every relay invocation. The `PilotEvent.tool_input` field (`Record<string, unknown>`) carries raw tool arguments — Bash commands with inline secrets (`curl -H "Authorization: Bearer sk-..."`), Write tool file contents (`.env` files, private keys), etc. The log fired unconditionally whenever `--log-dir` was active.

Additionally, log files were created with default umask permissions (0755 dirs, 0644 files), making them readable by any local user at `/var/log/claude-pilot/`.

## Root Cause

**Defense gap pattern:** The codebase already had a secret-scrubbing mechanism (`SCRUB_PATTERNS` + `scrubEnv()`) for child process environment variables, and a safe `summarizeInput()` function for UI display. But the new logging path bypassed both. New code that handled sensitive data was added without inheriting the security properties of existing paths.

## Solution

### Fix 1 — Redact log payloads (P1)

Two layers of defense: gate behind `verbose` flag (opt-in), and log only structural metadata (never `tool_input`).

```typescript
// BEFORE (vulnerable — full event with secrets):
writeFileLog(`[relay:payload] ${JSON.stringify(event)}\n`);

// AFTER (safe — metadata only, opt-in):
if (verbose) {
  writeFileLog(
    `[relay:payload] type=${event.type} tool=${event.tool_name} id=${event.tool_use_id}\n`,
  );
}
```

**File:** `claude-pilot/src/transport.ts`

### Fix 2 — Restrict log file permissions (P2)

Set explicit restrictive modes at creation time instead of relying on umask.

```typescript
// BEFORE (default umask — world-readable):
mkdirSync(dirname(filePath), { recursive: true });
fileStream = createWriteStream(filePath, { flags: "a" });

// AFTER (owner-only):
mkdirSync(dirname(filePath), { recursive: true, mode: 0o700 });
fileStream = createWriteStream(filePath, { flags: "a", mode: 0o600 });
```

**File:** `claude-pilot/src/logger.ts`

### Fix 3 — Remove stale PilotEvent documentation (P3)

Removed `task_id` and `session_id` from the PilotEvent field documentation in `system_prompt.md` — these fields don't exist in the TypeScript `PilotEvent` interface.

**File:** `~/.mika/agents/mika-dev/skills/self-dev/system_prompt.md`

## How It Was Caught

Multi-agent code review (`/ce:review`) with 4 parallel agents. Both `security-sentinel` and `kieran-typescript-reviewer` independently flagged the P1 secret leakage. The `security-sentinel` also identified the P2 file permissions issue.

## Prevention

### Key principles

1. **Treat logs as an output boundary** — same as network responses and UI. They persist to disk, ship to aggregators, and may be world-readable.
2. **Never log opaque payloads** — if a type contains `unknown`, `any`, or user-controlled content, assume it carries secrets. Log metadata (type, ID, name) instead.
3. **Explicit file permissions at creation** — never rely on umask for files that may contain sensitive data. Use `0o600` for files, `0o700` for directories.
4. **Apply the "new path" reflex** — when adding code that touches data already handled by an existing path, ask: "What security properties does the existing path enforce, and does my new path enforce them too?"

### Code review checklist for logging near sensitive data

- [ ] Does the log statement receive raw tool_input, user data, or event payloads? Must go through established redaction/summarization.
- [ ] Are log files created with explicit restrictive permissions (`0o600`)?
- [ ] Is scrubbing applied before serialization (not via post-hoc regex)?
- [ ] Are correlation IDs (trace_id, session_id) logged instead of payloads?
- [ ] Does the logging respect existing project scrubbing conventions (`SCRUB_PATTERNS`, `summarizeInput`, redacting `Debug` impls)?

## Related

- [Env var leakage in exec handler child processes](env-var-leakage-exec-handler-child-processes.md) — same class of issue (secrets leaking through a process/output boundary), established Mika's three-tier env security model
- [Setup wizard secret handling](setup-wizard-secret-handling.md) — atomic writes with `0o600` permissions, the established file permission pattern
- [Routing URL logged with potential credentials](../../../todos/294-complete-p3-routing-url-logged-with-potential-creds.md) — same pattern of secrets in log output, fixed by stripping sensitive fields
- [Claude-pilot self-dev integration brainstorm](../../brainstorms/2026-03-17-claude-pilot-self-dev-integration-brainstorm.md) — broader context for the claude-pilot transport layer changes
- CLAUDE.md "Secrets" convention — canonical list of Mika's secret scrubbing patterns (`Settings` manual `Debug` impl, exec handler env scrubbing, MCP `env_clear()` + allowlist)

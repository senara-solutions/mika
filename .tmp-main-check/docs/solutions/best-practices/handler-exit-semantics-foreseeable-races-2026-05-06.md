---
module: mika-skills
tags: [dispatch, handler, exit-semantics, autonomous-loop, callback]
problem_type: handler-exit-semantics
category: best-practices
date: 2026-05-06
---

# Handler exit semantics for foreseeable races vs real crashes

## Principle

When a handler can fail for a *foreseeable racy reason* (the target became invalid between enqueue and fire — issue closed, branch deleted, repo archived, etc.), the right shape is `exit 0` + structured-JSON skip result delivered via the canonical callback helper. Reserve `exit 1` + HANDLER CRASH envelope for actual handler bugs (logic errors, missing required fields, unexpected provider responses) where the consumer cannot recover cleanly.

The exit code is the load-bearing distinction; downstream consumers (mika-dev's callback turn, the audit dashboard, watchdogs) make decisions based on it.

## Why this shape

1. **Exit 0** tells the EXIT trap "this was a clean exit" — the trap's `CALLBACK_SENT=1` guard prevents duplicate delivery, and the process ends without generating a HANDLER CRASH envelope.
2. **Structured JSON** (`{"status":"auto_skipped","reason":"<reason>","issue":"..."}`) gives downstream consumers a machine-parseable signal. They can `jq .status` and branch without regex-matching error strings.
3. **Canonical `_deliver_callback()` helper** — single delivery site, single `set +e / mika ask / CALLBACK_SENT=1 / set -e` ordering. No inline duplication.
4. **Audit trail preserved** — the structured result is persisted via `mika ask --task-complete` into the messages table. The dashboard can filter on `status: "auto_skipped"` to show every skipped dispatch with its issue ID.

## Incident that surfaced this

On 2026-05-06, the autonomous loop stalled ~7 hours because a closed-issue dispatch used `exit 1`. The EXIT trap wrapped the error as `HANDLER CRASH (exit code 1)`. mika-dev's callback turn read the crash envelope, posted a confirmation question ("want me to proceed?"), and idled until manual intervention at 07:59Z.

Fix: mika#988 — reclassified the closed-issue branch from `exit 1` to `exit 0 + auto_skipped JSON`.

## Sibling patterns

| Pattern | Exit code | Shape | When to use |
|---------|-----------|-------|-------------|
| **Foreseeable race** (this principle) | `exit 0` | Structured JSON via `_deliver_callback()` | Target invalid due to timing (issue closed, branch gone) |
| **`DISPATCH_VALIDATION_ERROR`** (mika#955) | `exit 1` | Structured JSON in stderr, wrapped as HANDLER CRASH | Real handler bug (LLM forgot a required field) |
| **Generic crash** | `exit 1` | Unstructured stderr, wrapped as HANDLER CRASH | Unexpected failure (dependency missing, network error) |

The first two share "structured JSON" but differ on exit code by design — that is the load-bearing distinction.

## Anti-patterns to avoid

- **Inlining a second callback-send site for the skip path.** Always call `_deliver_callback()` — the helper owns the `CALLBACK_SENT=1` flag that prevents the EXIT trap from double-delivering.
- **Using `exit 1` for expected races.** The EXIT trap wraps any non-zero exit as HANDLER CRASH. Consumers cannot distinguish "issue was already closed (harmless)" from "bash syntax error (real bug)."
- **Falling through to the rest of the handler after delivering the skip.** The skip branch must be terminal (`exit 0` after `_deliver_callback`). No fall-through to worktree setup, claude-pilot invocation, or the success-path callback.

## Citations

- `docs/architecture/review-guide.md` § Orthogonality — keeping recovery-class outcomes in the response shape rather than letting them bleed into the exit-code channel.
- `docs/solutions/cross-repo-patterns/security-hardening-playbook.md` — analogous shape for fail-closed-vs-fail-open guards.
- mika#955 (`DISPATCH_VALIDATION_ERROR`) — the contrapositive pattern for real handler bugs.

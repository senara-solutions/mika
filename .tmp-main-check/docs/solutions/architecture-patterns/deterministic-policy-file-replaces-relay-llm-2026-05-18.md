---
module: claude-pilot
date: 2026-05-18
problem_type: best_practice
component: tooling
severity: high
tags:
  - permission-policy
  - relay-deprecation
  - deterministic-evaluation
  - claude-pilot
  - mika-notify
  - cross-repo
applies_when:
  - Replacing an LLM-based decision path with a deterministic rule engine
  - Adding a new CLI verb that writes to a well-known DB session
  - Implementing cross-repo features spanning Rust and Python codebases
---

# Deterministic Policy File Replaces Relay LLM Call

## Context

The `claude-pilot` permission callback pipeline had three tiers: tier1 (hardcoded auto-approve rules in Python), tier2 (LLM-based relay agent via `mika --agent mika-relay ask`), and interactive fallback. The tier2 relay path suffered from 14% decision drift (mika#1161) and ~7s per-event latency. Phase B of the relay deprecation milestone (mika#1188) replaces the relay LLM call with a deterministic YAML policy-file lookup, eliminating both drift and latency.

## Guidance

### Policy file design

The policy file (`claude-pilot-py/src/claude_pilot/policies/permissions.yaml`) uses ordered rules with first-match-wins semantics:

- Each rule specifies `tool` (exact match), `pattern` (regex against the tool's primary input field), `decision` (allow/deny/escalate), and `reason`.
- Primary input field mapping: `command` for Bash, `file_path` for Write/Edit/Read, `pattern` for Glob/Grep, `skill` for Skill.
- The `default` section fires when no rule matches. Default is `escalate` (fail-closed).
- Pydantic v2 models with `ConfigDict(extra="allow")` for forward-compatibility — new fields won't break existing parsers.

### Permission callback flow

```
Tier1 auto-approve → Policy file lookup → [dead relay code] → Interactive fallback
```

The policy is loaded once at `create_permission_handler()` call time and cached for the session lifetime. Changes to the YAML require a new `claude-pilot` session to take effect.

### Escalation transport

On `decision: escalate`, the handler fires a best-effort `mika notify --text "<tool>: <input>: <reason>" --severity escalate` via `subprocess.Popen` (fire-and-forget, no shell), then returns `PermissionResultDeny`. The `mika notify` CLI verb writes to a well-known fixed-UUID notifications session (`00000000-0000-0000-0000-700000710717`) in the mika agent's SQLite DB, with optional Telegram delivery via the gateway's `/send` endpoint.

### Emergency rollback

`MIKA_PILOT_POLICY_DISABLED=1` env var bypasses the policy evaluation and falls through to the relay path. Evaluated once at handler creation time (frozen for the session). Phase C (mika#1193) removes the relay code and this flag.

## Why This Matters

- **Drift elimination**: Deterministic rules produce identical decisions on identical inputs. The 14% drift observed with the LLM relay (mika#1161) is structurally impossible with regex matching.
- **Latency reduction**: Policy evaluation is sub-millisecond vs. ~7s for the Kimi relay.
- **Auditability**: Every rule has an `id` and `reason` that appear in structured logs, making permission decisions traceable.
- **Fail-closed by default**: Unknown events escalate rather than being decided by an LLM that might hallucinate an `allow`.

## When to Apply

- When replacing any LLM-based binary decision (allow/deny) with rule-based evaluation.
- When adding operator notification channels — the `mika notify` → fixed-UUID session → optional Telegram pattern is reusable.
- When implementing cross-repo features: same branch name across repos, primary repo first, companion PR cross-references.

## Examples

### Policy rule matching

A Bash command `gh issue create --title "bug"` is evaluated against:
```yaml
- id: gh-issue-create
  tool: Bash
  pattern: '^\s*gh\s+issue\s+create\b'
  decision: deny
  reason: "Issue creation routes through mika-issue/mika-issues skills."
```
The regex matches, decision is `deny`, and the tool call is blocked with the reason as the deny message.

### Notification write path

```
claude-pilot escalate
  → subprocess.Popen(["mika", "notify", "--text", "...", "--severity", "escalate"])
    → init_db_only_for_agent("mika")
    → INSERT OR IGNORE into sessions (fixed UUID, channel="notifications")
    → INSERT into messages (role="system", metadata={"source":"mika-notify"})
    → (optional) POST to gateway /send with bearer auth for Telegram delivery
```

## Related

- mika#1188 — parent milestone: Deprecate mika-relay
- mika#1191 — Phase A: expanded tier1 deterministic floor
- mika#1192 — Phase B: this implementation
- mika#1193 — Phase C: relay agent retirement
- mika#1161 — relay drift incident that motivated the migration
- `docs/solutions/architecture-patterns/` — related architecture patterns

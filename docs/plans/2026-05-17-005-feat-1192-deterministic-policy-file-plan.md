---
type: feat
issue: mika#1192
parent: mika#1188 (milestone: Deprecate mika-relay)
depends_on: mika#1191 (merged + soaked ≥3 days)
title: Deterministic policy file replaces relay LLM call
date: 2026-05-17
---

# Plan: claude-pilot deterministic policy file (mika#1192, Phase B of mika#1188)

## Phase 0 — Pin

**Base anchors at grooming time:**
- `mika` HEAD: `72021b78482f1c313156e7630d626865415dede3`
- `claude-pilot-py` HEAD: `86bd3eebc39ac053cd71a7660f793b943958f7fd`

**Source surfaces touched (verbatim quotes at base SHA):**

### `claude-pilot-py/src/claude_pilot/transport.py` (cp @ 86bd3ee, lines 45-95)

```python
async def invoke_command(
    config: PilotConfig,
    event: PilotEvent,
    verbose: bool,
    task_id: str | None = None,
) -> PilotResponse:
    """Invoke the configured relay subprocess and return its parsed response.

    Raises:
        TransportError: subprocess failed, produced no output, or returned
            unparseable/invalid JSON.
        asyncio.CancelledError: caller aborted (SIGINT, timeout at caller level).
    """
    timeout = (config.timeout or 120_000) / 1000.0  # ms → seconds
    # ... (subprocess.create_subprocess_exec + scrub_env + "[claude-pilot] " prefix payload)
    proc = await asyncio.create_subprocess_exec(
        config.command,
        *args,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=scrubbed,
    )
    # ...
    payload = f"[claude-pilot] {event.model_dump_json(exclude_none=True)}".encode()
```

Note: prefix `[claude-pilot] ` is load-bearing — comment at `transport.py:90-93` cites the qwen3-coder 2026-04-11 incident.

### `claude-pilot-py/src/claude_pilot/types.py` (cp @ 86bd3ee, lines 78-98)

```python
class PilotResponseAllow(BaseModel):
    model_config = ConfigDict(extra="forbid")
    action: Literal["allow"]


class PilotResponseDeny(BaseModel):
    model_config = ConfigDict(extra="forbid")
    action: Literal["deny"]
    message: str | None = None


class PilotResponseAnswer(BaseModel):
    model_config = ConfigDict(extra="forbid")
    action: Literal["answer"]
    answers: dict[str, str]


PilotResponse = PilotResponseAllow | PilotResponseDeny | PilotResponseAnswer
```

### `.claude/claude-pilot.json` (4 copies, identical)

```json
{
  "command": "mika",
  "args": ["--agent", "mika-relay", "ask"],
  "timeout": 120000
}
```

Locations: `mika-platform/.claude/`, `mika/.claude/`, `mika-skills/.claude/`, `mika-cloud/.claude/`, and `claude-pilot-py/.claude/` (5 copies total — verified by grep at grooming time).

## Goal

For permission events Phase A's tier1 doesn't auto-approve, replace `transport.invoke_command(PilotEvent)` (subprocess to `mika --agent mika-relay ask`) with a **deterministic in-process policy-file lookup**. Eliminate the LLM hop from claude-pilot's dispatch path entirely. Escalation goes through a new `mika notify` CLI verb.

## Concrete changes

### Change 1 — Policy file format and location

New file: `claude-pilot-py/src/claude_pilot/policies/permissions.yaml` (versioned with the consumer; pip-packaged with the wheel).

Schema (concrete, validated via pydantic at load time):

```yaml
# Each rule: matched in order; first hit wins. tool_input keys are
# tool-specific (e.g., `command` for Bash, `file_path` for Write).
# ────────────────────────────────────────────────────────────────────
# NOTE — the rules below are SCHEMA-ILLUSTRATIVE ONLY.
# The actual rule-set ships from Change 4's 30-day audit output, not
# from this plan-time example. Implementer: do NOT copy these into the
# shipped policies/permissions.yaml without confirming against audit data.
# In particular, `write-outside-project` duplicates tier1.py's existing
# is_within_project() check — that path never reaches the policy file in
# practice; it's shown here for schema documentation, not as a load-bearing
# rule (per architect NF8).
# ────────────────────────────────────────────────────────────────────
rules:
  - id: gh-issue-create  # illustrative
    tool: Bash
    pattern: '^\s*gh\s+issue\s+create\b'
    decision: deny
    reason: "Issue creation routes through mika-issue/mika-issues skills."

  - id: mika-ask-non-platform-agent  # illustrative
    tool: Bash
    pattern: '^\s*mika\s+ask\s+--agent\s+(?!mika-(arch|dev|qa)\b)'
    decision: escalate
    reason: "Non-platform agent dispatch requires operator approval."

  # Real rules added by Change 4's audit; rule count capped at 30 (see Change 4).

default:
  decision: escalate
  reason: "No matching policy — escalating to operator."
```

Loader: `claude-pilot-py/src/claude_pilot/policy.py` (new). Public API: `load_policy(path: Path | None) -> Policy`; `evaluate(policy: Policy, event: PilotEvent) -> PilotResponse | Escalation`.

`Escalation` is a new sentinel type (not a `PilotResponse` variant) returned to `permissions.py`'s callback orchestrator, which then fires `mika notify` and returns `PilotResponseDeny` to the SDK.

### Change 2 — `claude-pilot-py/src/claude_pilot/permissions.py` callback orchestrator

Replace the tier-1 → tier-3 → relay flow with:

1. `is_tier1_auto_approve` (Phase A's expanded form) → return `PilotResponseAllow`.
2. `is_tier3_dangerous` → return `PilotResponseDeny`.
3. `policy.evaluate(loaded_policy, event)`:
   - On `decision: allow` → `PilotResponseAllow`.
   - On `decision: deny` → `PilotResponseDeny(message=rule.reason)`.
   - On `decision: escalate` (or `default.escalate`) → fire `mika notify --text "<tool>: <input>: <reason>"` (subprocess) AND return `PilotResponseDeny(message=rule.reason)`. No relay call.

Phase B does NOT remove `transport.invoke_command` — it's left in place as dead code (Phase C deletes it). A feature flag `MIKA_PILOT_POLICY_DISABLED=1` re-enables the old relay path for emergency rollback.

### Change 3 — `mika notify` CLI verb

New subcommand on `mika` (Rust):

```
mika notify --text <message> [--channel cli|telegram] [--severity info|warn|escalate]
```

**Session-targeting mechanism (per architect F2):**

`mika notify` writes to a **well-known fixed-UUID notifications session**, created lazily on first invocation.

- Fixed UUID: a constant in `mika/crates/mika-cli/src/commands/notify.rs` (suggested: `00000000-0000-0000-0000-7000007107170000` — clearly synthetic, sorted to a distinct prefix). Exact value chosen at `/ce:work` time.
- Session row: `sessions` table, `agent_id=mika`, `channel=notifications`, `name="mika notify"`. Created via `INSERT OR IGNORE` so concurrent first-invocations don't race.
- Message row: `messages` table, `session_id=<fixed-UUID>`, `agent_id=mika`, `role=system`, `content=<text>`, `metadata={"source":"mika-notify","severity":<sev>,"channel":<channel>}`.
- Discovery: `mika status` and the dashboard's session list should treat `channel=notifications` as a distinct surface (filterable, optionally elevated).

This shape:
- Avoids polluting other agent sessions with notification messages.
- Avoids inventing a new top-level `notifications` table.
- Keeps the messages-as-event-log invariant — anything `mika notify` writes is queryable via the existing `messages` indexes.
- Makes B-AC2's E2E test concrete: `sqlite3 ~/.mika/data/mika.db "SELECT count(*) FROM messages WHERE session_id='<fixed-UUID>' AND metadata LIKE '%mika-notify%'"`.

**Telegram delivery (per NF7):** if `--channel telegram` is requested AND `MIKA_GATEWAY_URL` is reachable, POST to mika-gateway's existing outbound endpoint. The endpoint is `POST <MIKA_GATEWAY_URL>/send` with `Authorization: Bearer <MIKA_INTERNAL_TOKEN>` (the existing internal-token auth between agent and gateway). Payload format: `{"chat_id": "<operator-default-from-config>", "text": "<message>"}`. `mika notify` reuses the existing send-message infrastructure — no new gateway endpoint, no new auth.

File targets:
- `mika/crates/mika-cli/src/main.rs` — add `Notify` subcommand to clap enum.
- `mika/crates/mika-cli/src/commands/notify.rs` — new file implementing the handler; defines the fixed-UUID notifications-session constant.
- `mika/crates/mika-agent/src/notify.rs` (or inline in cli crate, TBD by `/ce:work`) — DB write + optional gateway POST.

### Change 4 — Pre-Phase-B 30-day relay-decision audit (operator-run)

Before Phase B's PR opens, run `replay_relay_decisions.py --days 30 --emit-audit` (the harness Phase A introduces, with audit-mode flag added in this PR) to enumerate relay-handled events into generalizable clusters.

**Bucketing strategy (per architect F1):**

- **Bash tool:** cluster by `(tool_name, first_two_tokens_of_command)`. E.g., `("Bash", "gh issue")`, `("Bash", "mika ask")`, `("Bash", "cargo test")`. Two tokens is the right granularity — one is too coarse (every `git ...` collapses), three is too fine (every distinct flag triggers a new cluster).
- **Write / Edit tools:** cluster by `(tool_name, top_two_path_segments)`. E.g., `("Write", "mika/src")`, `("Edit", "claude-pilot-py/tests")`. Captures directory-tree intent without over-fragmenting.
- **Skill tool:** cluster by `(tool_name, skill_name)` directly.
- **Other tools:** cluster by `tool_name` alone (rare).

**Output JSON schema:**

```json
{
  "audit_window_days": 30,
  "events_total": 180,
  "events_replayable": 165,
  "events_unreplayable": 15,
  "clusters": [
    {
      "cluster_key": ["Bash", "gh issue"],
      "count": 42,
      "allow_count": 38,
      "deny_count": 4,
      "escalate_count": 0,
      "answer_count": 0,
      "sample_inputs": [
        "gh issue view 123 --json title,body",
        "gh issue edit 1188 --body-file /tmp/x.md",
        "gh issue create --repo ... --title ..."
      ],
      "decision_consensus": "allow",
      "consensus_confidence": 0.905
    }
  ]
}
```

`decision_consensus` is the modal decision; `consensus_confidence` = mode_count / total_count for the cluster. Clusters with consensus_confidence < 0.8 are flagged for manual review (mixed signals — the bucketing is too coarse or the inputs genuinely diverge).

**Rule-count ceiling and selection:**

- If audit produces ≤30 clusters, every cluster becomes a YAML rule.
- If audit produces >30 clusters, the top 30 by `count` (frequency) become rules; the remainder fall through to `default: escalate`. The PR description must enumerate which clusters were dropped and why.
- Clusters with `consensus_confidence < 0.8` are NOT auto-promoted to rules — they require operator review. Surface in the audit's companion doc (`docs/plans/<date>-<NNN>-phase-b-audit.md`).

**Audit companion doc:** `docs/plans/<YYYY-MM-DD>-<NNN>-phase-b-audit.md` records the audit run timestamp, the cluster list with consensus/confidence, the rule-set selection rationale, and any manual operator decisions on low-confidence clusters. This doc is committed in Phase B's PR alongside `policies/permissions.yaml`.

**The rules file is derived from this audit's output, not hand-written from imagination.** The audit script's output IS the policy file generator's input.

### Change 5 — Per NF7: verify `transport.py` graceful-fallback

`transport.py:45-95` reads `PilotConfig` from `.claude/claude-pilot.json`. After Phase B, the config is still present (Phase C removes it). After Phase C, it's gone. Verify the load path:

```bash
# At grooming time, identified pending code-read:
grep -n "PilotConfig\|load_config\|FileNotFoundError" claude-pilot-py/src/claude_pilot/transport.py
```

If `PilotConfig.from_file()` raises `FileNotFoundError` on missing file (rather than returning `None` or a default), Phase B must add a graceful-skip branch before Phase C ships. Treat as in-scope for Phase B per NF7. Test: `cd claude-pilot-py && rm .claude/claude-pilot.json && uv run pytest tests/test_agent.py::test_no_relay_config_graceful` (new test).

## Acceptance criteria

- **B-AC1.** Zero calls to `mika ask --agent mika-relay` from claude-pilot's dispatch path on the standard code-path (feature-flag `MIKA_PILOT_POLICY_DISABLED` notwithstanding). Verified by replay of 30 days of relay invocations against tier1 + policy file — 100% resolved as allow / deny / escalate locally. (Unreplayable events from harness reported separately, NOT counted as resolved.)
- **B-AC2.** Escalation delivers a message via `mika notify` on `decision: escalate`. End-to-end test: trigger an escalate event, assert a row lands in `messages` table with `metadata.source == "mika-notify"`. Telegram integration tested if `MIKA_GATEWAY_URL` is set in CI.
- **B-AC3.** Policy file is loadable from a configurable path (default: pip-packaged location); changes to the YAML take effect on next claude-pilot session start. (Hot-reload mid-session is OUT of scope — deferred to follow-up.)
- **B-AC4.** TIER 2 ("research and answer technical questions") is dropped. The 30-day audit (Change 4) enumerates TIER 2 hits; if any flow depends on a non-trivial TIER 2 answer, file a follow-up ticket — **not** a Phase B blocker. Architect-ratified per milestone grooming.
- **B-AC5.** Per NF7: `transport.py` handles missing `.claude/claude-pilot.json` gracefully (returns `None` or a no-op `PilotConfig` instead of crashing). Test: `test_no_relay_config_graceful`.

## Risks

- **Policy-file schema lock-in.** v1 schema must be forward-compatible. Mitigation: pydantic model uses `model_config = ConfigDict(extra="allow")` so additional fields don't break parsing.
- **Dropping TIER 2 silently breaks a flow (NF5 sibling).** Pre-Phase-B 30-day audit must enumerate. If audit shows TIER 2 fires for `/ce:compound` clarification or any load-bearing flow, surface BEFORE PR opens.
- **`mika notify` ergonomics.** Vincent reads the notifications; format must be self-describing. v1 format: `[<severity>] <tool>: <input-excerpt> — <reason>`. Tested via E2E in B-AC2.
- **Soak overlap with Phase A.** Phase B PR cannot merge until Phase A is 3+ days post-merge with no rollback.
- **Feature-flag drift.** `MIKA_PILOT_POLICY_DISABLED` is an emergency lever. Document its removal in Phase C's PR (mika#1193).
- **Multi-repo config copy drift.** 5 copies of `.claude/claude-pilot.json` exist (Phase 0 Pin). Phase B doesn't touch them; Phase C removes all 5. If any drift between repos during Phase B's soak (someone edits one), Phase C's regen-or-delete pass surfaces it.

## Out of scope

- Phase C's relay agent retirement (mika#1193).
- Hot-reload of policy file mid-session.
- Replacing the `[claude-pilot] ` payload prefix or other transport-layer contracts.
- New HTTP endpoints on `mika-server` (option iii was rejected during milestone grooming).

## Verification

- Unit: `cd claude-pilot-py && uv run pytest tests/test_policy.py tests/test_permissions.py -v`.
- Replay: `uv run python tests/replay/replay_relay_decisions.py --days 30 --against tier1+policy` shows 100% resolution.
- E2E: trigger `mika notify` from an escalate decision; verify `messages` row appears with metadata.
- Smoke: kill `mika-relay` agent processes; autonomous-loop dispatch still completes.

## Rollback

`MIKA_PILOT_POLICY_DISABLED=1` re-enables the old transport.py relay path. No DB migration in Phase B; no data loss possible. Single PR revert otherwise.

## Sequencing

Depends on Phase A (mika#1191) merged + soaked ≥3 days. Phase C (mika#1193) depends on Phase B merged + soaked ≥7 days.

## Related

- Parent milestone: mika#1188
- Depends on: mika#1191 (Phase A)
- Blocks: mika#1193 (Phase C)
- mika#1161 — relay drift incident (motivation)
- `feedback_prompt_enforcement_fragile` — same principle: deterministic policy > LLM prose
- NF5/NF7 carried from milestone grooming — addressed in Change 4 + Change 5

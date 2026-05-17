---
type: feat
issue: mika#1191
parent: mika#1188 (milestone: Deprecate mika-relay)
title: Port permission-policy TIER 1/1.5/3 rules into claude-pilot tier1.py
date: 2026-05-17
---

# Plan: claude-pilot tier1.py expansion (mika#1191, Phase A of mika#1188)

## Phase 0 — Pin

**Base anchors at grooming time:**
- `mika` HEAD: `72021b78482f1c313156e7630d626865415dede3` ("chore(dev-groom): revert prompt-only design — restore deterministic tool+handler (mika#1173) (#1187)")
- `claude-pilot-py` HEAD: `86bd3eebc39ac053cd71a7660f793b943958f7fd` ("fix(agent): log thinking-only turns to surface drift (#10) (#11)")

**Source surfaces touched (verbatim quotes, with file:line at base SHA):**

### `claude-pilot-py/src/claude_pilot/tier1.py` (claude-pilot @ 86bd3ee)

Entry point (`tier1.py:19-44`):

```python
def is_tier1_auto_approve(tool_name: str, tool_input: dict[str, Any], cwd: str) -> bool:
    if tool_name in ("Read", "Glob", "Grep"):
        return True

    if tool_name == "Bash":
        command = tool_input.get("command", "")
        if not isinstance(command, str) or not command.strip():
            return False
        return is_safe_bash_command(command)

    if tool_name in ("Write", "Edit"):
        file_path = tool_input.get("file_path", "")
        if not isinstance(file_path, str) or not file_path:
            return False
        return is_within_project(file_path, cwd)

    if tool_name == "Skill":
        skill = tool_input.get("skill", "")
        if not isinstance(skill, str):
            return False
        return skill.strip() in TIER1_SAFE_SKILLS

    return False
```

- `TIER1_SAFE_SKILLS` frozenset starts at `tier1.py:46`.
- `TIER3_PATTERNS` regex tuple starts at `tier1.py:70`.
- `is_safe_bash_command` function starts at `tier1.py:111`.
- File length: ~300 lines.

### `mika/skills/bundled/permission-policy/system_prompt.md` (mika @ 72021b78)

TIER 1 rules (`system_prompt.md:13-22`):

```markdown
**TIER 1 — AUTO-APPROVE (respond `{"action": "allow"}`):**
- Read-only tools: `Read`, `Glob`, `Grep`
- All git commands: `git status`, `git log`, `git diff`, `git branch`, `git show`, `git commit`, `git push`, `git checkout`, `git worktree`
- Build/test: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt`, `cargo build`, `npm run build`, `npm run dev`, `npm test`
- `Write`/`Edit` within the project directory
- Non-destructive shell: `cd`, `ls`, `cat`, `head`, `tail`, `wc`, `find`, `mkdir`, `grep`, `sed`, `awk`, `tee`, `python3`, `echo`, `command -v`, `which`
- Compound commands where ALL parts are TIER 1 (e.g., `cd /path && gh issue view`, `cd /path && cargo test`) — evaluate each part, allow if all parts are safe
- PR operations: `gh pr create`, `gh pr view`, `gh pr list`, `gh issue view`
- Intra-platform agent dispatch (narrow allow-list): `mika ask --agent mika-arch ...`, `mika ask --agent mika-dev ...`, `mika ask --agent mika-qa ...` — these are platform-internal peer calls (e.g., `/mika-groom-ticket` Phase 3 sends architect briefs via `mika ask --agent mika-arch`). Do NOT extend to `mika ask --agent *` wildcards.
- Platform-prescribed GitHub authoring (narrow allow-list): `gh issue edit <num> ...` (issue body amendment, e.g., `/mika-groom-ticket` Phase 5 step 19), `gh issue comment <num> ...` (closing comment, Phase 5 step 20). Do NOT extend to `gh issue create`.
```

TIER 1.5 (`system_prompt.md:31-32`):

```markdown
**TIER 1.5 — AUTO-ANSWER WITHOUT RESEARCH (respond `{"action": "answer", "answers": {...}}`):**
- If the question mentions "compact-safe", "compound" mode selection, or asks to choose between "full compound" and "compact-safe" — auto-answer with `{"action": "answer", "answers": {"<echo exact question text>": "compact-safe"}}`. Do NOT research. This prevents headless stalls from `/ce:compound` Phase 0 interactive prompts (see #79).
```

TIER 3 deny list (`system_prompt.md:39-44`):

```markdown
**TIER 3 — ESCALATE TO VINCENT (use `send_message`, then respond `{"action": "deny"}`):**
- `rm -rf`, `git push --force`, `git reset --hard`, `DROP TABLE`, `cargo publish`
- `sed -i` (destructive pattern edits — use `sed` read-only or Python instead)
- `gh label delete`, `gh label edit` (label changes propagate to ALL issues)
- Any irreversible/destructive operation
- Push to `main`/`master` branch
```

### `claude-pilot-py/src/claude_pilot/permissions.py` (claude-pilot @ 86bd3ee)

`CanUseTool` callback signature (`permissions.py:47-50`):

```python
CanUseTool = Callable[
    [str, dict[str, Any], ToolPermissionContext],
    Awaitable[PermissionResult],
]
```

Orchestrates: `is_tier1_auto_approve` → if False, `is_tier3_dangerous` → if False, `transport.invoke_command(PilotEvent)` (relay).

### `claude-pilot-py/src/claude_pilot/types.py` (claude-pilot @ 86bd3ee)

`PilotResponseAnswer` shape (`types.py:78-95`):

```python
class PilotResponseAnswer(BaseModel):
    action: Literal["answer"]
    answers: dict[str, str]
```

## Goal

Port the deterministic TIER 1 + TIER 1.5 + TIER 3 rules from `mika/skills/bundled/permission-policy/system_prompt.md` (LLM prose) into `claude-pilot-py/src/claude_pilot/tier1.py` (deterministic Python). Effect: `mika-relay` is invoked only for residual ambiguous cases — net ≥80% relay-call elimination based on the parity-replay sample.

## Concrete changes

### Change 1 — `claude-pilot-py/src/claude_pilot/tier1.py`

Extend `is_safe_bash_command(command)` so the Bash allow-list covers every TIER 1 pattern from `system_prompt.md:13-22` that isn't already present. The existing structure (`tier1.py:111-128`) splits compound commands via `_split_compound_command`, then ORs `_is_safe_sub_command` across `is_safe_git_command | is_safe_build_command | is_safe_shell_command | is_safe_gh_command`. Two new check functions slot in alongside those:

- **Intra-platform agent dispatch** — new `INTRA_PLATFORM_AGENTS: frozenset[str] = frozenset({"mika-arch", "mika-dev", "mika-qa"})` constant + new `is_safe_mika_dispatch(sub: str) -> bool` function. Regex: `^\s*mika\s+ask\s+--agent\s+(\S+)\b` — extract agent name, match against `INTRA_PLATFORM_AGENTS`. No wildcard.
  - **Sentinel cross-ref:** `mika/crates/mika-agent/src/well_known_agents.rs:386-396` documents this as a deliberately duplicated list across languages with a "if it grows beyond 5 entries OR diverges, escalate to build-time codegen" callout. Phase A makes the duplication real (currently the comment claims `INTRA_PLATFORM_AGENTS` exists in tier1.py but it doesn't). This plan inherits the sentinel; Phase A's PR description must include the cross-reference and confirm the codegen-escalation threshold is not yet hit (3 entries < 5).
- **GitHub authoring (narrow)** — extend `SAFE_GH_SUBCOMMANDS` at `tier1.py:246-253`:
  - `"issue"` currently maps to `frozenset({"view", "list"})`. Add `"edit"` and `"comment"` to the frozenset.
  - Verify `is_safe_gh_command` (`tier1.py:260-274`) already handles the lookup. Expected: yes — adding the subcommand strings is sufficient. `gh issue create` stays unmapped → still falls through to relay.
- **Compound command parity** — `_is_safe_sub_command` (`tier1.py:123-128`) already ORs the existing check functions. Add `is_safe_mika_dispatch` to the OR chain. Compound-safety inherits automatically because `_split_compound_command` (verified at `tier1.py:115-118`) splits before each segment is checked.

For `is_tier3_dangerous(command)` add coverage for:

- `git push <remote> (main|master)` if not already covered — verify against `TIER3_PATTERNS` line `re.compile(r"git\s+push\s+\S+\s+(main|master)\b")` at `tier1.py:74`. **Pre-implementation diff** required: compare the existing TIER3_PATTERNS tuple to the `system_prompt.md:39-44` list and add anything missing. Expected delta: zero (current TIER3_PATTERNS already mirrors that list); if non-zero, surface in the PR description.

### Change 2 — `claude-pilot-py/src/claude_pilot/permissions.py`

Add a TIER 1.5 short-circuit before the relay subprocess invocation:

- Detect: `PilotEvent` is a `question`-shaped event (use existing PilotEvent variant discrimination — do NOT add new pydantic types).
- Match: the question text contains the case-insensitive substring `"compact-safe"` OR matches the pattern "(choose|select) between .*full compound.* and .*compact-safe.*".
- Construct: `PilotResponseAnswer(action="answer", answers={<question_text>: "compact-safe"})` and return without invoking `transport.invoke_command`.
- Type contract: `PilotResponseAnswer` already exists at `types.py:78-95`; no new types.

### Change 3 — `claude-pilot-py/tests/test_tier1.py`

Add cases (target ≥10 new tests):

| Test | Tool | Input | Expected |
|---|---|---|---|
| `test_intra_platform_dispatch_mika_arch_approved` | Bash | `mika ask --agent mika-arch "@/tmp/brief.md"` | True |
| `test_intra_platform_dispatch_mika_dev_approved` | Bash | `mika ask --agent mika-dev "implement mika#1191"` | True |
| `test_intra_platform_dispatch_mika_qa_approved` | Bash | `mika ask --agent mika-qa "review PR#456"` | True |
| `test_intra_platform_dispatch_other_agent_denied` | Bash | `mika ask --agent some-other-agent "..."` | False (relay) |
| `test_intra_platform_dispatch_compound_with_cd_approved` | Bash | `cd /tmp && mika ask --agent mika-arch ...` | True |
| `test_gh_issue_edit_approved` | Bash | `gh issue edit 123 --body-file /tmp/x.md` | True |
| `test_gh_issue_comment_approved` | Bash | `gh issue comment 123 --body "..."` | True |
| `test_gh_issue_create_not_in_tier1` | Bash | `gh issue create --repo ... --title ...` | False (relay) |
| `test_gh_issue_edit_compound_denied_if_unsafe_part` | Bash | `gh issue edit 123 && rm -rf /tmp` | False (TIER3 trips) |
| `test_compact_safe_question_auto_answered` | PilotEvent | question containing "compact-safe" | PilotResponseAnswer with "compact-safe" |

Add reciprocal tests for TIER 3 deny-list coverage (any pattern from `system_prompt.md:39-44` not already in `TIER3_PATTERNS` — expected zero deltas, but tests pin parity).

### Change 4 — Replay harness `claude-pilot-py/tests/replay/replay_relay_decisions.py`

Standalone operator-runnable script (not part of `pytest`). Reads recent `mika-relay` invocations from `~/.mika/data/mika.db` and replays each through the new tier1 + permissions path.

Contract:

```python
# CLI: uv run python tests/replay/replay_relay_decisions.py --days 7
# Output (stdout, machine-readable):
{
  "days": 7,
  "events_total": 42,
  "events_replayable": 38,    # excludes malformed JSON, missing fields
  "events_unreplayable": 4,   # reported, NOT silently dropped — anti-NF5 safeguard
  "resolved_by_tier1": 30,    # local allow/deny without relay
  "still_needs_relay": 8,
  "disagreement_vs_relay": [  # cases where tier1 says allow but relay said deny (or vice versa)
    {"event_id": "...", "tool": "Bash", "input": "...", "tier1": "allow", "relay": "deny"}
  ],
  "tier1_resolution_pct": 78.9  # resolved_by_tier1 / events_replayable
}
```

Anti-NF5 safeguard: any event the harness can't replay (malformed payload, missing tool_input field, schema mismatch) is reported separately as `events_unreplayable` — never silently dropped or counted as "eliminated by tier1." A-AC3 measures `tier1_resolution_pct` against `events_replayable`, not against `events_total`.

## Acceptance criteria

- **A-AC1.** Every rule from `system_prompt.md:13-22` (TIER 1) is encoded in `tier1.py` as a deterministic pattern. Rule-parity diff against `system_prompt.md` documented in PR description; any rule that does NOT port (with reason) is enumerated. Cross-check against `mika/crates/mika-agent/src/server/permission_pre_classifier.rs` (per NF6) to flag any rule the Rust pre-classifier handles differently — divergence is OK if explained, but must be visible.
- **A-AC2.** TIER 1.5 "compact-safe" question auto-answered by claude-pilot without invoking the relay. End-to-end: replay one historical compact-safe event; `transport.invoke_command` is not called.
- **A-AC3.** Replay of last 7 days of `mika-relay` invocations (n ≥ 30 events) shows `tier1_resolution_pct` ≥ 80% on the **replayable** subset. Unreplayable events reported separately.
- **A-AC4.** Existing `test_tier1.py` cases unchanged in pass count. New cases: ≥10.
- **A-AC5.** Median permission-event latency for tier1-resolved events drops by ≥5× vs. pre-change baseline. Measurement: instrument `permissions.py` with `time.perf_counter()` around the tier1 + transport paths; emit p50/p95 latency at session end. Compare against a pre-deploy baseline captured on the same workload.

## Risks

- **Compound-command false positives.** New regexes must compose with existing `is_safe_bash_command`'s segment-splitter. Mitigation: `test_gh_issue_edit_compound_denied_if_unsafe_part` pins the behavior.
- **`answer` action constructed client-side.** The relay builds `PilotResponseAnswer` via its tool surface today; claude-pilot will construct it directly from the new code path. Risk: schema drift. Mitigation: `PilotResponseAnswer` is pydantic-validated at `types.py:78-95`; any drift fails at construction time.
- **Drift between `tier1.py` and `permission-policy/system_prompt.md`** until Phase C ships. Mitigation: PR description includes mika#1193 link + soak-window timeline; the system prompt's TIER 1 section gets a "tier1.py is the canonical surface; this prose is documentation" callout.
- **Replay harness false-positives (NF5).** Events the harness can't replay are reported separately, not silently dropped. A-AC3 measures against `events_replayable`.

## Out of scope

- Phase B's deterministic policy file (mika#1192) — Phase A leaves the relay invocation path intact for residuals.
- Phase C's relay retirement (mika#1193) — Phase A's `system_prompt.md` remains the relay's instruction surface.
- Engine-side `server/permission_pre_classifier.rs` (mika#935) — different layer; cross-referenced per NF6 but not modified here.

## Verification

- `cd claude-pilot-py && uv run pytest tests/test_tier1.py -v` — all green; new cases ≥10.
- `uv run python tests/replay/replay_relay_decisions.py --days 7` — `tier1_resolution_pct` ≥ 80%; `events_unreplayable` enumerated.
- Manual: one autonomous-loop dispatch end-to-end; observe `mika-relay` message count post-deploy vs. baseline; expect ≥5× drop.

## Rollback

Pure addition to `tier1.py` + `permissions.py`. No removal of fallback paths. If `tier1` mis-classifies, the relay still answers. Single PR revert; no migration, no data changes.

## Sequencing

Standalone. Phase B (mika#1192) depends on Phase A merged + soaked ≥3 days. Phase C (mika#1193) depends on Phase B.

## Related

- Parent milestone: mika#1188
- Rule-parity cross-ref: mika#935 (`server/permission_pre_classifier.rs`, engine-side)
- Origin: mika#1161 (relay drift on Kimi-k2.6)
- NF5 (anti-undercount replay-harness spec): addressed in Change 4 + A-AC3
- NF6 (mika#935 cross-ref): addressed in A-AC1

---
title: "fix(skills): mika ask intra-platform agent dispatch is not Bash; relay should consult a platform agent registry"
type: fix
status: active
date: 2026-05-02
---

# fix(skills): mika ask intra-platform agent dispatch is not Bash; relay should consult a platform agent registry

**Target repos:** `claude-pilot-py` (typed dispatch surface) + `mika` (mika-platform workspace; structural pre-classification + agent registry config). Two coordinated PRs, sequenced. See [Cross-Repo Coordination](#cross-repo-coordination).

## Overview

`mika ask --agent <peer>` invocations from claude-pilot currently flow through mika-relay's permission-policy LLM classifier (Haiku-tier). Phase 1 diagnostic on canary v5 (session `2f7575cd-2707-48bc-8a03-dd56cb93d95a`) confirmed that even with PR #936's narrow TIER 1 entries in deployed source (md5 parity verified), **any `mika ask` invocation with a real message argument denies**. The trailing `...` in the prompt's allow-list pattern is interpreted by the LLM as a literal pattern, not "any args." This is not a propagation bug; it is the wrong layer for this category of call.

This plan introduces two coordinated changes that compose as belt-and-suspenders:

1. **claude-pilot-py side:** extend `tier1.py`'s fast-path to recognize intra-platform agent dispatch (`mika ask --agent {mika-arch,mika-dev,mika-qa}`). The Bash command auto-allows in `is_tier1_auto_approve()` without ever spawning the relay subprocess. The Bash classifier never sees the call.
2. **mika-platform side:** add a structural pre-classifier in mika-relay's permission decision pipeline that recognizes intra-platform agent dispatch as a distinct category, reading from a `well_known_agents.rs`-backed config. Even if a future caller (or a regression in claude-pilot) shells out and reaches the relay, the relay classify-and-allows structurally before the LLM is consulted.

The static config of three names (mika-arch, mika-dev, mika-qa) is fine for v1. No discovery protocol.

After both ship, dev-groom canary on mika#931 should reach `/mika-groom-ticket` Phase 3 step 9 without `[denied] Bash: mika ask --agent mika-arch ...`. Dashboard launch unblocks.

## Problem Frame

**Empirically established:** Three rounds of tactical patching since 2026-05-02 (PR #936 + dev-groom canary v1-v5) failed because the LLM Bash classifier is mis-specified for this category. The classifier's behavior, captured in canary v5's session log:

| Form | Verdict |
|------|---------|
| `mika ask --agent mika-arch --help 2>&1 \| head -30` | ALLOW |
| `mika ask --agent mika-arch "Test ping" 2>&1 \| head -50` | DENY |
| `mika ask --agent mika-arch "$(cat /tmp/x)" 2>&1` | DENY |
| `mika ask "hello" 2>&1 \| cat` | DENY |

Pattern: any `mika ask` invocation with a real message argument denies. PR #936's narrow TIER 1 expansion did not fix this — the literal `...` in the prompt is read as part of the pattern, not as a placeholder.

**Why this isn't fixable via prompt iteration:** the LLM classifier is a Haiku-tier model evaluating raw command strings. Refining the TIER 1 wording to better match real prose-message commands is brittle (any next call form drifts) and couples the agent roster to the prompt. The diagnostic captured at https://github.com/senara-solutions/mika/issues/935#issuecomment-4363977468.

**Strategic context:** the dashboard milestone (mika#13) launch is blocked on dev-groom working. dev-groom Phase 3 step 9 issues `mika ask --agent mika-arch "<brief>"` with multi-KB prose payloads. Until this dispatch path is reliable, no grooming can complete autonomously.

## Requirements Trace

- **R1** — `claude-pilot` invocations of `mika ask --agent {mika-arch,mika-dev,mika-qa}` with arbitrary message arguments succeed without reaching mika-relay's LLM permission classifier. Verified by `tier1.py` fast-path returning `True` for these commands.
- **R2** — Even if a caller bypasses claude-pilot's tier1 fast-path (e.g., a future skill handler that shells out directly), mika-relay structurally classifies `[claude-pilot] ` prefixed Bash commands matching the intra-platform pattern as ALLOW before the LLM is consulted.
- **R3** — The agent roster (mika-arch, mika-dev, mika-qa) is sourced from a config the relay reads, not from a TIER 1 prompt enumeration. Adding a new platform agent does NOT require editing `permission-policy/system_prompt.md`.
- **R4** — Wildcard expansion (`mika ask --agent *`) and unrecognized peers fall through to existing classification (LLM gate). The reframe routes around the classifier for known peers only.
- **R5** — `mika ask --agent mika-arch` with a >2KB string payload, in the same shape `/mika-groom-ticket` Phase 3 step 9 produces, is observed ALLOW. Captured as a trace in the implementation PR body.
- **R6** — After deploy, dev-groom canary on mika#931 reaches verdict GROOMED end-to-end (grooming comment posted, plan callout in body, plan doc committed to the canary branch).
- **R7** — `mika-platform/docs/` and `claude-pilot-py/docs/` carry short architecture notes documenting (a) the platform-internal-agent-dispatch category, (b) the typed dispatch surface, and (c) how to add a new platform agent without prompt edits.
- **R8** — Standard verification commands pass: `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all` in mika-platform; equivalent test/lint commands in claude-pilot-py (recorded during implementation, see Open Questions).

## Scope Boundaries

- **In scope:** `claude-pilot-py/src/claude_pilot/tier1.py` (fast-path extension), mika-platform's permission decision pipeline (structural pre-classifier), agent roster config source (read from `well_known_agents.rs` or sibling), short docs in both repos, verification gate.
- **Out of scope:**
  - The `gh issue {create,edit,comment}` reframe — same shape but separate ticket.
  - mika-platform#75 (bootstrap escape hatch, parked) — #935 is independent and ships without it.
  - Generalized agent registry beyond mika-arch, mika-dev, mika-qa and their direct siblings. No discovery protocol, no per-agent capability schema.
  - mika-relay's permission classifier itself. The reframe routes around it for this category; other classifier behavior is unchanged.
  - `_shared/dispatch-lib.sh` refactor beyond what bundled-skill propagation requires for any new permission-policy config the relay reads.
  - `/mika-groom-ticket.md` Phase 3 step 9 invocation form. The reframe is exactly so the form stops mattering.
  - Refactoring `permission-policy/system_prompt.md`'s TIER 1 list. PR #936's narrow entries can stay (defense-in-depth); they are no longer load-bearing.
  - Wildcard support, dynamic registration, agent versioning, capability negotiation.
- **Independent of mika-platform#75:** confirmed. The escape-hatch design at #75 addresses how operators bootstrap fixes when the dispatcher is broken. #935's reframe addresses how claude-pilot dispatches to known peers in the steady state. Neither blocks the other; #935 ships standalone.

### Deferred to Separate Tasks (architect F7)

- **RR-002 (security finding from mika#936) — `mika ask --agent mika-dev` enables prompt-injection-driven implementation dispatch.** mika#936's PR body recorded this as "not accepted, revisit when mika#935 designs the agent registry." mika#935's v1 design includes mika-dev in the trinity (per Vincent's spec) without architectural treatment of the prompt-injection surface (e.g., dispatch-tier audit logging, per-peer rate limits, operator-confirmation hooks for specific dispatch verbs). RR-002 is **deferred to a follow-up ticket** filed against `mika` after #935 ships. Forward-pointer: file ticket "fix(security): per-peer prompt-injection guards for intra-platform dispatch (mika#936 RR-002 follow-up)" once #935's PRs merge. Vincent's ratification of mika-dev in the v1 trinity (recorded in mika#936's PR body and reaffirmed by the unpark comment on #935) stands. The follow-up adds defense-in-depth, not a v1 blocker.

## Cross-Repo Coordination

**Decision: two coordinated PRs, sequenced.** Not one cross-repo PR.

Rationale:
- claude-pilot-py and mika-platform are independently versioned/deployed (`uv tool install` for claude-pilot vs `make deploy` for mika).
- Unit 1 (claude-pilot-py tier1 extension) is the primary canary unblocker. It can ship and be deployed independently of Unit 2.
- Unit 2 (mika-platform structural pre-classifier) is defense-in-depth. It lands second; without it, R1 is satisfied but R2 is not.
- A single cross-repo PR would couple two independent deployment cycles into a non-trivial atomic operation, against the workspace's per-repo PR convention (`mika-platform/CLAUDE.md`).

**Sequencing (architect F3 contract):**
1. Unit 1 PR opens against claude-pilot-py. **Smoke test before merge:** in the PR-checkout state, run `uv tool install --force --editable .` then `mika ask --agent mika-arch "Test"` — verify ALLOW (no `[denied]` in claude-pilot's relay log; tier1 short-circuits). If smoke fails, do NOT merge — Unit 1's parser has a regression that would leave the gap window strictly worse than current state.
2. Unit 1 PR merges and deploys via `uv tool install --force --editable ./claude-pilot-py`.
3. **Gap window verification:** within the gap (Unit 1 deployed, Unit 2 not yet), the canonical case (`mika ask --agent mika-arch <payload>` from claude-pilot context) must work. Verify with a probe: `mika ask --agent mika-arch "Gap test ($(date))"` from claude-pilot. If FAIL, halt and rollback.
4. **Rollback contract for Unit 1 regression:** `uv tool install --force <prior-version-spec>` reverts. Unit 2 PR MUST NOT merge until Unit 1 regression is resolved or rolled back. Recorded in Unit 1's PR body.
5. Unit 2 PR opens against mika-platform's `mika` repo (where the Rust engine + agent roster live). After merge, `make deploy` propagates.
6. Documentation (Unit 3) lands in two PRs, one in each repo, alongside their respective code units.
7. Verification gate (Unit 4) runs after BOTH have deployed.

**Gap window safety: bounded ONLY if Unit 1's parser handles 100% of /mika-groom-ticket's emission forms.** F4's enumerated corner cases must all be tested before Unit 1 merges. If F4 reveals an unhandled form, the gap window is unsafe and Unit 2 must ship same-day.

**Branch naming across repos:** same slug for traceability (`fix/935/skills-mika-ask-intra-platform-agent`).

**PR cross-referencing (architect F9):** each PR body MUST include the following:

- **Unit 1 PR (claude-pilot-py):**
  1. Link to mika#935 (origin issue).
  2. Link to Unit 2 PR (when filed; back-fill if Unit 2 PR opens after Unit 1).
  3. The 6 corner-case categories from Unit 1's Approach section (env-var prefix, compound separators, flag re-ordering, `--` separator, equals form, quoting variants).
  4. Gap-window verification result (smoke + probe) and the rollback command (`uv tool install --force <prior-version-spec>`).

- **Unit 2 PR (mika-platform):**
  1. Link to mika#935 (origin issue).
  2. Link to Unit 1 PR (already merged at this point per sequencing contract).
  3. The F1 hook-layer pin: `handlers.rs:80-110` symbol + the 5 decision branches.
  4. The pre-implementation `agent_id` access verification result (per F8).

These cross-references are NOT optional polish — they are the audit-trail discovery surface for future readers tracing how the dispatch reframe shipped across repos.

## Context & Research

### Relevant Code and Patterns

- **`claude-pilot-py/src/claude_pilot/tier1.py`** — existing fast-path module. `is_tier1_auto_approve(tool_name, tool_input, cwd) -> bool` is the entry called from `permissions.py`. Returns True → permission flow short-circuits to ALLOW without spawning the relay subprocess. Already contains `TIER1_SAFE_SKILLS` (frozenset of slash-command names like `/mika`, `/ce:plan`) and `TIER3_PATTERNS` (deny-list regex). Helper functions: `is_safe_bash_command`, `is_safe_gh_command`, `is_safe_git_command`, `is_safe_shell_command`. The intra-platform agent dispatch check belongs here as a sibling helper.
- **`claude-pilot-py/src/claude_pilot/permissions.py`** — calls `is_tier1_auto_approve` first; if False, builds a `PilotEvent` and ships it to the relay subprocess. No changes needed if Unit 1 returns True from tier1.
- **`claude-pilot-py/src/claude_pilot/transport.py`** — uses `asyncio.create_subprocess_exec` to spawn the relay; scrubs sensitive env vars (`KEY|SECRET|TOKEN|AUTH|PRIVATE`). Not modified by this plan.
- **`mika/crates/mika-agent/src/well_known_agents.rs`** — static list of well-known agents (mika-dev, mika-qa, mika-arch, mika-relay). Provides `find_well_known_agent(name) -> Option<...>` and similar helpers. The agent registry config for Unit 2 reads from here (or a new sibling module that exports the canonical platform-peer list).
- **`mika/skills/bundled/permission-policy/system_prompt.md`** — current LLM classifier prompt. PR #936's narrow TIER 1 entries (lines 21-23) remain as defense-in-depth. After Unit 2's structural pre-classifier ships, these entries are no longer load-bearing for the canonical case but still match if the structural check is ever bypassed.
- **`mika/crates/mika-agent/src/agent.rs` and skill execution path** — where the structural pre-classifier hooks in. The agent's main loop receives a user message; if the message matches the `[claude-pilot] ` PilotEvent prefix AND carries a Bash command for known intra-platform dispatch, return the JSON action directly without invoking the LLM.

### Institutional Learnings

- `mika/docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md` — establishes that mika-arch, mika-dev, mika-qa, mika-relay are the canonical platform-peer set. Skill overrides are first-creation-only; the agent identity is stable.
- `mika/docs/solutions/architecture-patterns/trust-critical-skill-tier-and-template-sync.md` — permission-policy is a trust-critical bundled skill; structural changes ship atomically with engine code.
- `mika/docs/solutions/best-practices/shared-dispatch-library-for-claude-pilot-skills-2026-04-29.md` — `_shared/dispatch-lib.sh` exists and is load-bearing at runtime but excluded from build-time skill discovery. Not directly relevant; flagged because Vincent's scope explicitly excludes refactoring it.
- `mika/docs/solutions/best-practices/operator-only-bundled-skill-structural-enforcement-2026-04-28.md` — pattern for two-layer structural enforcement of skill scoping. Adjacent shape; this plan applies the same pattern to permission classification (claude-pilot side + relay side, both structural).

### External References

None — internal architecture work, no third-party integration.

## Key Technical Decisions

- **Static config of three peers in v1.** Per Vincent's spec, no discovery protocol. The three names live as a constant or in `well_known_agents.rs`'s existing list. Future expansion to N peers is a pure data change, not a structural change.
- **claude-pilot-py is the primary fix surface.** Unit 1 alone unblocks the canary; Unit 2 is defense-in-depth for future regression. Sequencing reflects this — ship Unit 1 first.
- **No new mika-spirit endpoint.** A `POST /agent-dispatch` endpoint was considered (would let claude-pilot make a typed HTTP call instead of a Bash subprocess); rejected because (a) it duplicates `mika ask`'s logic at a new surface, (b) requires auth boundary work, (c) `tier1.py` extension achieves Vincent's "Bash classifier never sees these calls" contract without a new RPC layer. The relay subprocess simply isn't spawned.
- **Structural pre-classifier on the Rust side, not in the prompt.** Unit 2 is Rust code that runs before the LLM is invoked, returning the JSON action directly. The permission-policy prompt itself is unchanged in this PR; the pre-classifier sits in front of it.
- **Agent roster reads from `well_known_agents.rs`** (or a thin sibling that re-exports the relevant subset). No new config file. Adding a peer = adding to the existing well-known list; the pre-classifier picks it up automatically.
- **Two coordinated PRs over one atomic cross-repo PR.** See [Cross-Repo Coordination](#cross-repo-coordination).
- **`tier1.py` regex over a yaml/toml config.** The peer list lives in `tier1.py` as a Python constant for v1. A separate config file is overkill for three names; would add load-time complexity and a new failure mode.
- **PR #936's narrow TIER 1 entries stay.** After Unit 2 ships, they are no longer load-bearing for the canonical case, but they form a third layer of defense (LLM-level allow on the canonical pattern). Removing them is YAGNI; leave alone.
- **Cross-language peer-list duplication: option (c) — sentinel + threshold (architect F2).** The peer list lives in two places: `well_known_agents.rs` (Rust slice via `intra_platform_dispatch_peers()`) and `tier1.py` (`INTRA_PLATFORM_AGENTS` frozenset). Drift between them creates the asymmetry mika#935 itself is filed to fix (Rust path allows, Python path rejects, or vice versa). Three options were evaluated: (a) build-time codegen from Rust to Python — rejected as YAGNI for a three-name list that changes <1×/year; (b) CI consistency check — rejected as adding a new failure mode without strong payoff at v1 scale; (c) sentinel comment + refactor threshold — chosen. Both files carry a sentinel block above their peer-list constant naming the duplication and the refactor threshold (escalate to codegen if peer list grows beyond 5 entries OR diverges between languages). The sentinel makes the duplication explicit and grep-discoverable; the threshold pins when to revisit.
- **TIER 3 pattern duplication: same pattern (architect F5).** TIER 3 deny patterns (`rm -rf`, force-push, etc.) appear in both `tier1.py` and Unit 2's pre-classifier (Rust). Same option-(c) sentinel block above each pattern site naming the canonical source (tier1.py is the human-authored list; Rust is the mirror) and the refactor threshold (codegen if pattern set grows beyond 10 entries OR Python and Rust drift).

## Open Questions

### Resolved During Planning

- **Q: One cross-repo PR or two coordinated PRs?** A: Two coordinated PRs, sequenced. (Decision recorded in [Cross-Repo Coordination](#cross-repo-coordination).)
- **Q: Should the structural pre-classifier ALSO bypass tier1.py's existing fast-paths?** A: No. Tier1.py is claude-pilot-side; the structural pre-classifier is mika-relay-side. They're orthogonal layers in different processes.
- **Q: Does this require adding `mika ask` to `tier1.py`'s `_is_safe_sub_command` (compound-command path)?** A: Yes — to handle `cd /worktree && mika ask --agent mika-arch ...` Bash forms. The compound-command path runs `_is_safe_sub_command` per part; the new check must cover sub-commands too. Recorded in Unit 1.
- **Q: Where does Unit 2's pre-classifier hook into the agent execute path?** A: Before the LLM call in the agent's main loop. Specifically, the entry point that processes a `[claude-pilot] ` user message. (See Unit 2 file list.)

### Deferred to Implementation

- **Q: Exact location of Unit 2's hook in `crates/mika-agent/src/agent.rs` (or wherever).** Defer — implementer reads the agent loop and identifies the specific function. Plan specifies WHAT (pre-LLM Rust check) not WHERE-by-line.
- **Q: claude-pilot-py test/lint command set.** Run `cd claude-pilot-py && cat pyproject.toml | grep -A3 'scripts\|tools.uv'` during implementation to identify. Likely `uv run pytest` and `uv run ruff check`. Record in PR body.
- **Q: Does `mika ask` need a `--quiet` or `--platform-internal` flag for mika-relay's pre-classifier to short-circuit unambiguously?** Defer — implementer evaluates whether structural pattern-match on `mika ask --agent <known-peer>` is sufficient, or whether a marker flag is needed for robustness against argument-order variation. Lean: pattern-match is sufficient; flag is overkill. Resolves once the regex/parser is written.
- **Q: Should Unit 2's pre-classifier emit a structured trace (e.g., `permission-policy: structural-allow agent=<peer>`)?** Defer — yes if a logging hook exists at that layer; otherwise emit at INFO via `tracing`. Implementer decides during integration.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

### Current flow (denied)

```
claude-pilot Bash tool call: mika ask --agent mika-arch "<brief>"
  → SDK can_use_tool callback
    → permissions.py: is_tier1_auto_approve() → False  (no match in current tier1)
    → spawn relay subprocess: mika --agent mika-relay ask -
      → mika-relay receives [claude-pilot] PilotEvent
        → permission-policy LLM classifier evaluates Bash command
        → DENY (literal-pattern mismatch on "...")
    → relay returns {"action": "deny"}
  → SDK denies tool call
  → claude-pilot session burns turns retrying variants
```

### Target flow with both units shipped

```
claude-pilot Bash tool call: mika ask --agent mika-arch "<brief>"
  → SDK can_use_tool callback
    → permissions.py: is_tier1_auto_approve() → True  (NEW: matches intra-platform dispatch)
    → returns {"action": "allow"}  (relay never invoked)
  → SDK allows tool call
  → Bash subprocess runs `mika ask --agent mika-arch "<brief>"`
  → mika-arch receives the message and responds normally

[Defense-in-depth path — only fires if a future caller bypasses tier1]:
claude-pilot or other caller spawns relay subprocess for mika ask command
  → mika-relay's agent loop receives [claude-pilot] PilotEvent
    → STRUCTURAL pre-classifier (Unit 2):
      if Bash command matches `mika ask --agent <peer>` AND <peer> ∈ well_known_agents
        → emit {"action": "allow"} directly (LLM never called)
      else
        → fall through to existing LLM classifier (no behavior change for other commands)
```

### Adding a new platform agent post-#935 (e.g., mika-prime)

1. Add `mika-prime` to `well_known_agents.rs` (existing well-known list).
2. Add `mika-prime` to `tier1.py`'s `INTRA_PLATFORM_AGENTS` constant.
3. Deploy both repos. No prompt edits, no LLM re-tuning.

(Two-line change. The current state would require editing the LLM prompt and praying the classifier honors it.)

## Implementation Units

- [ ] **Unit 1: `tier1.py` fast-path for intra-platform agent dispatch (claude-pilot-py)**

**Goal:** Extend `tier1.py` to recognize `mika ask --agent <peer>` for `<peer> ∈ {mika-arch, mika-dev, mika-qa}` as TIER 1 auto-approve. The relay subprocess is never spawned for these calls.

**Requirements:** R1, R5 (partial — Unit 1 alone proves the canonical path).

**Dependencies:** None.

**Target repo:** `claude-pilot-py`

**Files:**
- Modify: `src/claude_pilot/tier1.py`
- Modify: `src/claude_pilot/permissions.py` (only if needed — the existing `is_tier1_auto_approve` call site already routes to ALLOW on True; verify no extra hook needed)
- Test: `tests/test_tier1.py` (extend with intra-platform dispatch cases)

**Approach:**
- Add `INTRA_PLATFORM_AGENTS: frozenset[str] = frozenset({"mika-arch", "mika-dev", "mika-qa"})` near the top of `tier1.py`, alongside `TIER1_SAFE_SKILLS`. Wrap with the F2 sentinel comment block (refactor threshold pin).
- Add `is_intra_platform_agent_dispatch(command: str) -> bool` function:
  - Use `shlex.split` (not regex; correctly handles quoting, env vars, multi-word args) to tokenize.
  - **Parser corner cases (architect F4 — enumerate AND test ALL of these):**
    1. **Env-var prefix.** `MIKA_LOG_FORMAT=pretty mika ask --agent mika-arch <msg>` — skip leading tokens matching regex `^[A-Z_][A-Z0-9_]*=.*$` until the first non-assignment token. Mirror `tier1.py`'s existing env-strip helper if one exists; otherwise add a sibling helper.
    2. **Compound separators.** `cd /tmp && mika ask --agent mika-arch <msg>` — handled by wiring into `_is_safe_sub_command`. Recognized separators: `&&`, `||`, `;`, `|`. Backticks and `$()` shell-substitution forms in the command STRUCTURE (not within quoted args) are NOT split — entire command rejected (TIER 3 pattern carry-over). Note: `$()` and backticks WITHIN a quoted message argument (e.g., `mika ask --agent mika-arch "$(cat /tmp/x)"`) are read by the shell BEFORE tier1 sees the resolved command — these are benign; the shell expands them first. Test fixtures must distinguish these two cases.
    3. **Flag re-ordering.** `mika ask <msg> --agent mika-arch` (positional message before flag) and `mika ask --agent mika-arch <msg>` (flag before message) MUST both match. Don't assume `--agent` at position 2 — scan tokens for `--agent` (or `--agent=<peer>` equals form) anywhere after `mika ask`.
    4. **`--` separator.** `mika ask --agent mika-arch -- <msg starting with dashes>` — `shlex.split` preserves `--` as a literal token; parser must continue scanning for `--agent` BEFORE `--` (after `--`, args are positional message content).
    5. **Equals form.** `mika ask --agent=mika-arch <msg>` — recognized as alternative to `--agent <peer>`. Both forms accepted.
    6. **Quoting variants.** `mika ask --agent "mika-arch" <msg>` — `shlex.split` strips quotes; token comparison still works. Test fixture exercises both quoted and unquoted peer names.
  - **TIER 3 carry-over (architect F4 + F5).** If `is_tier3_dangerous(command)` returns True for the full command OR any sub-command after compound-split, return False (existing TIER 3 deny remains absolute). This includes the SEC-001 form `mika ask --agent mika-arch 'text' && rm -rf /dir` — the `&&` splits into two sub-commands; `rm -rf` matches TIER 3; entire command rejected. Wrap with the F5 sentinel block naming the canonical pattern source (`tier1.py` is canonical; Rust mirrors).
- Wire into `is_tier1_auto_approve`:
  - For Bash tool: if `is_tier3_dangerous(command)` returns True, fall through (existing behavior).
  - Else if `is_intra_platform_agent_dispatch(command)` returns True, return True (NEW).
  - Else proceed with existing checks.
- Wire into `_is_safe_sub_command` for compound-command coverage:
  - If a sub-command (split by `&&` / `||` / `;` / `|`) matches `is_intra_platform_agent_dispatch`, treat it as safe. This handles `cd /worktree && mika ask --agent mika-arch ...`. EACH sub-command independently runs through `is_tier3_dangerous`; if any sub-command is TIER 3, the entire compound rejects.

**Patterns to follow:**
- `tier1.py` existing helpers: `is_safe_bash_command`, `is_safe_gh_command`, `_is_safe_sub_command`. New helper mirrors their signature and structure.
- `tests/test_tier1.py` existing test patterns for happy-path / edge / error cases.

**Test scenarios (architect F4 — fully enumerated; each must be a unit test fixture):**
- **Happy path:** `mika ask --agent mika-arch "Test message"` → True.
- **Happy path:** `mika ask --agent mika-dev "Multi-word message with embedded \"quotes\" and newlines"` → True.
- **Happy path:** `mika ask --agent mika-qa "$(cat /tmp/brief.md)"` → **True** (this is the form that currently fails through the relay; fast-path bypasses it). Document that `$()` here is shell-resolved BEFORE tier1 sees the command — the resolved command has the file's content as a literal quoted argument.
- **Happy path (env-var prefix):** `MIKA_LOG_FORMAT=pretty mika ask --agent mika-arch "Hello" 2>&1 | tail -20` → True.
- **Happy path (multiple env-vars):** `MIKA_LOG_FORMAT=pretty MIKA_DEBUG=1 mika ask --agent mika-arch "Test"` → True.
- **Happy path (absolute path):** `/home/samidarko/.local/bin/mika ask --agent mika-arch "test"` → True.
- **Happy path (compound with cd):** `cd /worktree && mika ask --agent mika-arch "test"` → True (both sub-commands TIER 1).
- **Happy path (flag re-ordering):** `mika ask --agent mika-arch "test"` AND `mika ask "test" --agent mika-arch` → both True.
- **Happy path (equals form):** `mika ask --agent=mika-arch "test"` → True.
- **Happy path (`--` separator):** `mika ask --agent mika-arch -- "--message-starting-with-dashes"` → True (parser detects `--agent mika-arch` BEFORE `--`).
- **Happy path (quoted peer):** `mika ask --agent "mika-arch" "test"` → True.
- **Edge: intransitive `--help`:** `mika ask --agent mika-arch --help` → True (regression-test compatibility with existing pre-#935 ALLOW path).
- **Edge: no message arg:** `mika ask --agent mika-arch` → True (matches dispatch shape but with no payload; let it through — the existing CLI handles missing-arg case).
- **Error: unknown peer:** `mika ask --agent custom-agent "test"` → False (peer not in `INTRA_PLATFORM_AGENTS`; falls through to relay LLM classifier).
- **Error: TIER 3 in main command:** `mika ask --agent mika-arch "test" && rm -rf /tmp` → False (compound-split → second sub-command matches TIER 3 → entire command rejects).
- **Error: TIER 3 hidden in compound:** `cd /tmp && mika ask --agent mika-arch "test" && git push --force` → False.
- **Error: no `--agent` flag:** `mika ask "test"` → False (not the dispatch shape; falls through).
- **Error: token[0] mismatch:** `not-mika ask --agent mika-arch "test"` → False.
- **Error: missing peer value:** `mika ask --agent` (no peer name) → False (parser rejects malformed dispatch).
- **Edge: empty message:** `mika ask --agent mika-arch ""` → True (empty string is a valid argument; parser accepts).
- **SEC-001 (architect F4 explicit):** `mika ask --agent mika-arch 'text' && rm -rf /dir` → False. Compound-split into two sub-commands; second matches TIER 3; entire compound rejects per `_is_safe_sub_command` semantics.
- **Backtick form (rejected):** `mika ask --agent mika-arch \`cat /tmp/brief.md\`` → False (backticks in command structure trigger TIER 3 carry-over per existing tier1.py rules).
- Integration: end-to-end via the dev-groom canary on mika#931 (Unit 4's responsibility).

**Verification:**
- `uv run pytest tests/test_tier1.py` passes including new cases.
- Manual test: `python -c "from claude_pilot.tier1 import is_intra_platform_agent_dispatch; print(is_intra_platform_agent_dispatch('mika ask --agent mika-arch \"hello\"'))"` returns `True`.

---

- [ ] **Unit 2: Structural pre-classifier in mika-relay's permission decision pipeline (mika-platform)**

**Goal:** Before mika-relay's permission-policy LLM classifier is invoked, a structural pre-classifier in Rust recognizes `[claude-pilot] ` PilotEvents carrying `mika ask --agent <peer>` for known peers and returns `{"action": "allow"}` directly. Defense-in-depth for callers that bypass claude-pilot's tier1.

**Requirements:** R2, R3.

**Dependencies:** None on Unit 1 (independent layer). Sequenced after Unit 1 only because Unit 1 unblocks the canary first.

**Target repo:** `mika` (mika-platform workspace)

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` — export a const slice or function `intra_platform_dispatch_peers() -> &'static [&'static str]` returning `["mika-arch", "mika-dev", "mika-qa"]`.
- Create: `crates/mika-agent/src/permission_pre_classifier.rs` — new module implementing the structural check. Inline `#[cfg(test)]` mod for unit tests.
- Modify: `crates/mika-agent/src/server/handlers.rs:80-110` (`handle_message`) — hook the pre-classifier BEFORE `run_agent_for_message` is called. This is the load-bearing entry point for relay PilotEvents (verified via grep: this is where messages enter the agent path).
- Modify: `crates/mika-agent/src/lib.rs` (or wherever modules are declared) — register the new `permission_pre_classifier` module.
- Test: `crates/mika-agent/tests/permission_pre_classifier_integration.rs` — integration test exercising the full path (PilotEvent → pre-classifier → allow → no LLM call). Use `EvalHarness` with `MockLlmProvider` + assert zero `llm_call started` events.

### Phase 0 Pin (architect F1 requirement)

**Hook location (load-bearing, pinned at groom time):**

The pre-classifier sits between message reception and `run_agent_for_message` in `handlers.rs:80`'s `handle_message`. The decision tree:

```
fn handle_message(...) -> Response {
    if let Some(action) = pre_classify_pilot_event(&user_message, &agent_id) {
        return Response::ok(action.to_json_string());  // skip run_agent_for_message
    }
    // existing path
    run_agent_for_message(...).await
}
```

**Why this layer (not run_agent_inner):** `handle_message` is the highest layer where the message text and agent identity are both available, and where short-circuiting cleanly avoids ALL agent-loop overhead (skill matching, prompt building, LLM call, response parsing). Hooking inside `run_agent_inner` would still pay for `load_agent_context`, `build_system_prompt`, and `match_message` work that's wasted for the fast-path. handlers.rs:80 is also where existing fast-paths for callback-task delivery live (verified by grep on `is_callback_turn`).

**Function signature (pinned):**
```rust
pub(crate) fn pre_classify_pilot_event(
    user_message: &str,
    agent_id: &str,  // only mika-relay receives PilotEvents; gate on agent_id == "mika-relay"
) -> Option<PermissionAction>;
```

`PermissionAction` is a small enum (`Allow` only for v1; `Deny`/`Answer` reserved for future expansion). Returns `None` to fall through to existing flow.

**Decision branches (pinned):**

1. `agent_id != "mika-relay"` → `None` (only the relay should process PilotEvents)
2. `!user_message.starts_with("[claude-pilot] ")` → `None` (not a PilotEvent)
3. JSON parse of payload after the prefix fails → `None` (malformed; existing error path applies)
4. `tool_name != "Bash"` → `None` (this fast-path only handles Bash dispatch; AskUserQuestion + other tools route to LLM)
5. `tool_input.command` matches intra-platform dispatch pattern (per Unit 1's regex/parser logic, ported to Rust) AND `<peer>` is in `intra_platform_dispatch_peers()` AND no TIER 3 pattern present → `Some(PermissionAction::Allow)`
6. Otherwise → `None`

**Threading the `None` case:** existing `handle_message` proceeds unchanged. No refactor of the existing code path.

**`agent_id` access pattern (architect F8):** the precise access pattern at `handlers.rs:80` was not verified at groom time without filesystem access. Implementation step 1 of Unit 2 is to verify `agent_id` access at `handlers.rs:80` (likely a parameter, self-field, or `context.agent_id()` call); if the actual access differs from the assumed parameter, update `pre_classify_pilot_event`'s signature to match the available access shape. The function signature is otherwise final.

**Approach:**
- Pre-classifier function signature (directional): `pre_classify_pilot_event(message: &str) -> Option<PermissionAction>` returning `Some(Allow)` for matching intra-platform dispatch, `None` to fall through to existing LLM flow.
- Hook into the agent's message-handling path BEFORE the LLM is prepared. If the message starts with `[claude-pilot] `, parse the PilotEvent JSON, extract `tool_input.command` for Bash tools, and run the structural check.
- Structural check matches: command (after stripping leading env-var assignments) starts with `mika ask --agent <peer>` where `<peer>` is in `intra_platform_dispatch_peers()`. Use the same parsing helpers as Unit 1 if a Rust port is needed; otherwise use a focused regex with anchors and word boundaries. Reject if a TIER 3 pattern appears anywhere in the command (port the relevant regexes from `tier1.py`).
- On match: emit `tracing::info!(target: "permission_policy", "structural-allow agent={peer}")` (or equivalent) so traces are observable without LLM consultation, then return `Some(PermissionAction::Allow)`.
- The PilotEvent activation gate at the top of `permission-policy/system_prompt.md` is unchanged. Pre-classifier sits in front of LLM invocation, not in front of the activation gate.

**Patterns to follow:**
- Rust idioms in `crates/mika-agent/`: anyhow::Result for application code, thiserror for library errors, `#[cfg(test)] mod tests` inline.
- Existing `well_known_agents.rs` const-list pattern (already used to enumerate peers).
- Existing `tracing` usage in `mika-agent` for structured logs.
- For the parser/regex, mirror `tier1.py`'s shape (env-var stripping, token matching, TIER 3 carry-over).

**Test scenarios:**
- Happy path: `[claude-pilot] {"tool_name":"Bash","tool_input":{"command":"mika ask --agent mika-arch \"<brief>\""}}` → `Some(Allow)`.
- Happy path: same with each of the three peers (mika-arch, mika-dev, mika-qa).
- Happy path: env-var prefix `MIKA_LOG_FORMAT=pretty mika ask --agent mika-dev "..."` → `Some(Allow)`.
- Happy path: compound `cd /worktree && mika ask --agent mika-qa "..."` → `Some(Allow)` (verify sub-command parsing; if not implemented, document deferral).
- Edge: non-Bash tool (e.g., `tool_name: "Read"`) → `None` (fall-through; pre-classifier only handles Bash dispatch).
- Edge: `[claude-pilot] {"tool_name":"Bash","tool_input":{"command":"mika ask --agent custom-peer ..."}}` → `None` (peer not in registry).
- Edge: message NOT starting with `[claude-pilot] ` → `None` (skip; this isn't a relay invocation).
- Error path: TIER 3 pattern in command (`mika ask --agent mika-arch "x" && rm -rf /tmp`) → `None` (fall through to existing classifier; existing tier3 deny still fires).
- Error path: malformed PilotEvent JSON → `None` (graceful fall-through; do NOT crash).
- Integration: full PilotEvent flow → pre-classifier hits → emits `permission_policy: structural-allow` log line → action returned to claude-pilot subprocess → claude-pilot allows the Bash call. Test via `EvalHarness` if applicable, or via a focused integration test in `crates/mika-agent/tests/`.

**Verification:**
- `cargo clippy --all-targets --all-features -- -D warnings` clean.
- `cargo test --all` passes including new tests.
- Inspect `tracing` output for at least one `structural-allow` log line during integration test.
- Live verification deferred to Unit 4.

---

- [ ] **Unit 3: Documentation in both repos**

**Goal:** Document (a) the platform-internal-agent-dispatch category, (b) the typed dispatch contract, (c) how to add a new platform agent without prompt edits.

**Requirements:** R7.

**Dependencies:** Units 1 and 2 (docs must reflect what shipped).

**Target repos:** `mika` (mika-platform workspace) + `claude-pilot-py`.

**Files:**
- Create or modify (mika): `mika/docs/architecture/platform-internal-agent-dispatch.md` (or extend an existing architecture doc if a more natural home exists). One page.
- Create or modify (claude-pilot-py): `claude-pilot-py/docs/typed-dispatch-surface.md` (or extend `claude-pilot-py/CLAUDE.md` / `README.md` if more natural). One paragraph to one page.
- Modify (both): cross-link between the two docs.

**Approach:**
- mika doc covers: what the category is, why it bypasses the Bash classifier, where the agent registry config lives (`well_known_agents.rs`), step-by-step "how to add a new platform agent" (two-line change: well_known_agents.rs + tier1.py), reference to mika#935 as origin.
- claude-pilot-py doc covers: the contract (relay's Bash classifier never sees intra-platform calls), when to use the fast-path vs shell out (intra-platform → fast-path; arbitrary shell → relay), reference to mika#935 as origin.
- Both docs reference each other so future readers can navigate the cross-repo design.

**Patterns to follow:**
- Existing `mika/docs/architecture/` doc style (review-guide.md, etc.) — concise, principle-first, cross-referenced.
- `claude-pilot-py/CLAUDE.md`'s existing prose density — short, technically dense, no fluff.

**Test scenarios:** Test expectation: none — documentation-only change with no behavioral implications. Verified by grep for keywords and link integrity (`grep -rE 'mika#935' docs/` returns the new docs).

**Verification:**
- `grep -i "platform.internal.agent.dispatch\|intra.platform" mika/docs/architecture/` returns the new doc.
- `grep -i "typed dispatch\|fast.path\|intra.platform" claude-pilot-py/docs/` (or wherever) returns the new doc.
- Cross-links resolve (manual click-through or `markdown-link-check` if available).

---

- [ ] **Unit 4: End-to-end verification gate**

**Goal:** After both Unit 1 and Unit 2 have deployed, prove that `mika ask --agent mika-arch` with a >2KB string payload via the new dispatch surface is observed ALLOW, in the same shape `/mika-groom-ticket` Phase 3 step 9 produces. Then re-fire dev-groom canary on mika#931 and confirm the three green criteria.

**Requirements:** R5, R6.

**Dependencies:** Unit 1 deployed (`uv tool install --force --editable ./claude-pilot-py`) AND Unit 2 deployed (`make deploy`).

**Target repos:** N/A (verification activity, not a code unit). Captured in the implementation PR body of whichever unit ships last.

**Files:** None modified. Trace artifacts captured in PR body and/or `docs/logs/`.

**Approach:**
- After both deploys, manually run a >2KB-payload test:
  ```
  cat > /tmp/canary-test-brief.md <<'EOF'
  [paste a >2KB markdown brief with realistic content — copy from /tmp/groom-brief-mika-931-pass1.md or similar]
  EOF
  mika ask --agent mika-arch "$(cat /tmp/canary-test-brief.md)"
  ```
  Verify: command returns mika-arch's response, no `[denied]` in any log, claude-pilot's relay subprocess was NOT spawned (check via `ps` or absence of `[relay:send]` in claude-pilot log).
- Re-fire dev-groom canary on mika#931:
  ```
  mika ask --agent mika-dev "groom mika issue#931"
  ```
  Verify three green criteria:
  - (a) grooming comment posted on mika#931 (`gh issue view 931 --json comments --jq '.comments | length'` >= 1).
  - (b) plan callout in comment body (`gh issue view 931 --json comments --jq '.comments[] | select(.body | test("Plan:"))'`).
  - (c) plan doc committed to `fix/931/...` branch (`git log --oneline fix/931/... -- 'docs/plans/*'`).
- Capture trace in the implementation PR body.

**Patterns to follow:**
- The canary protocol from the dashboard-launch plan at `~/.claude/plans/first-we-are-going-linked-river.md` Phase B.3.

**Test scenarios:** N/A (this IS the integration test).

**Verification:**
- All three green criteria observed.
- Dashboard launch (mika#13) advances to Phase C of the dashboard plan.

## System-Wide Impact

- **Interaction graph:** claude-pilot Bash tool → `permissions.py` → `tier1.py` (NEW: intra-platform path returns ALLOW directly) → SDK runs Bash command. mika-relay agent-loop NEW: pre-classifier check before LLM is consulted (Unit 2). Both layers compose; either is sufficient on its own for the canonical case.
- **Error propagation:** Unchanged for non-matching commands (existing classifier path). For matching commands, tier1 (claude-pilot-side) returns True → SDK ALLOW path. If pre-classifier (Unit 2) errors during parsing (malformed PilotEvent), it returns None and falls through to existing LLM flow — graceful degradation.
- **State lifecycle risks:** None. No DB schema changes, no migrations, no on-disk state beyond the source files. Unit 2's pre-classifier is pure (no side effects beyond a `tracing` log line).
- **API surface parity:** Unchanged. `mika ask` CLI is the same; permission classification behavior changes for known peer dispatch but the user-visible CLI is unchanged.
- **Integration coverage:** Cross-repo coordination is the highest-risk integration point. Sequencing (Unit 1 first, Unit 2 second) ensures no broken state. Verification gate (Unit 4) proves both layers compose correctly.
- **Unchanged invariants:**
  - The `[claude-pilot] ` activation gate in permission-policy is unchanged.
  - TIER 3 deny-list (`rm -rf`, force-push, etc.) is unchanged. Both Unit 1 and Unit 2 carry the existing TIER 3 patterns forward.
  - Wildcards (`mika ask --agent *`) and unrecognized peers fall through to existing LLM classifier.
  - PR #936's narrow TIER 1 entries in the prompt remain (defense-in-depth third layer).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `tier1.py`'s extension breaks an existing fast-path semantics — e.g., compound-command parsing edge case admits a malicious command. | Comprehensive test scenarios (Unit 1 lists 13). TIER 3 pattern check stays carry-over. Code review on `tier1.py` should explicitly trace each TIER 3 pattern is still enforced. |
| Unit 2's pre-classifier hooks into the wrong place in the agent loop — the LLM still gets called. | Integration test in Unit 2 asserts no LLM call when pre-classifier fires (count `llm_call started` events; should be zero for matching PilotEvents). |
| `well_known_agents.rs` is reorganized in a follow-up and the pre-classifier's source breaks. | Unit 2 imports from the public API of `well_known_agents.rs` (or its successor). Compile-time check catches breakage; test suite catches semantic drift. |
| Unit 3 docs lag behind units 1+2. | Docs land in the same PRs as the units they describe. Implementer writes docs alongside code, not after. |
| Verification gate (Unit 4) reveals a subtle flaw — e.g., the canary still fails for a different reason. | Unit 4's failure produces a clear diagnostic; the plan halts before declaring `Closes #935`. Alternative paths (deeper structural check, expand the registry) are tracked in follow-up tickets, not bundled into this PR. |
| claude-pilot-py is deployed via `uv tool install`; failure to re-install means the new tier1.py isn't active. | The Makefile's `deploy-claude-pilot` target reinstalls via `uv tool install --force --editable`. Document the required deploy step in PR body. |
| Sequencing failure: Unit 2 ships before Unit 1, causing a confusing intermediate state. | Cross-Repo Coordination explicitly sequences Unit 1 first. PR descriptions cite each other. Don't merge Unit 2 until Unit 1 is deployed and the canary partially passes. |

## Documentation / Operational Notes

- After both PRs ship and deploy, the dashboard-launch plan's Phase B.3 (canary on mika#931) re-fires. If green, Phase C (sprint creation) advances.
- No rollout flag, no feature gate. The fast-paths are unconditional once the source ships.
- Adding a new platform agent post-#935 is a two-file change (well_known_agents.rs + tier1.py). Documented in Unit 3's docs.
- The historical narrow TIER 1 entries from PR #936 are retained as belt-and-suspenders — they will activate if a future caller bypasses BOTH tier1.py and Unit 2's pre-classifier, which is an extremely unlikely path. Removing them is YAGNI.
- The `MIKA_DISABLE_BUNDLED_SKILLS` env var (default false) is unchanged. Bundled-skill re-sync continues to overwrite deployed skill files from source on agent restart; PR #936's entries persist via that mechanism.

## Sources & References

- **Origin issue:** [senara-solutions/mika#935](https://github.com/senara-solutions/mika/issues/935)
- **Phase 1 diagnostic:** [issue-comment-4363977468](https://github.com/senara-solutions/mika/issues/935#issuecomment-4363977468) — three classifier traces capturing ALLOW/DENY pattern.
- **Surfacing canary:** dev-groom canary v5 on mika#931, session `2f7575cd-2707-48bc-8a03-dd56cb93d95a`, 60 turns / $3.18 / 550s, all attempts denied.
- **PR #936:** narrow TIER 1 expansion that PROVED insufficient — the predecessor fix that scoped the problem.
- **Adjacent open tickets:** mika#923 (skill install path doesn't propagate `_shared/`), mika-platform#75 (escape-hatch design, parked), mika#931 (canary subject), mika#13 (dashboard milestone, blocked).
- **Source files (claude-pilot-py):**
  - `claude-pilot-py/src/claude_pilot/tier1.py` — fast-path module
  - `claude-pilot-py/src/claude_pilot/permissions.py` — caller of tier1
  - `claude-pilot-py/src/claude_pilot/transport.py` — relay subprocess spawning (not modified)
- **Source files (mika):**
  - `mika/skills/bundled/permission-policy/system_prompt.md` — current LLM classifier prompt (PR #936 entries remain)
  - `mika/crates/mika-agent/src/well_known_agents.rs` — agent registry config source
  - `mika/crates/mika-agent/src/agent.rs` — agent main loop (Unit 2 hook site, exact line TBD by implementer)
- **Dashboard-launch plan:** `~/.claude/plans/first-we-are-going-linked-river.md` (Phase B.3 = canary, Phase C = sprint).

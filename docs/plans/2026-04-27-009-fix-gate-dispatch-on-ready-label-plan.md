---
title: "fix(self-dev+gateway): gate dispatch on ready label or direct prompt only (positive consent)"
type: fix
status: active
date: 2026-04-27
origin: senara-solutions/mika#841
supersedes: senara-solutions/mika#807, senara-solutions/mika#801
---

# Plan — positive-consent dispatch gate (mika#841)

**Issue:** [mika#841](https://github.com/senara-solutions/mika/issues/841)
**Branch:** `fix/841/gate-dispatch-on-ready-label-or-direct`
**Type:** fix (skill-prompt + gateway code change)
**Labels:** bug, p1-important
**Supersedes:** mika#807, mika#801 (closed with supersede comments)

## Problem

mika-dev's autonomous dispatch is triggered by message-content matching with no event-source check. Routine GitHub activity that happens to contain the verbatim string `implement <repo> issue#<n>` triggers `run_claude_pilot` even when the operator never asked for dispatch.

**Verified incident (mika#838, 2026-04-27):**
- 16:20:26 — `[GitHub] New comment on senara-solutions/mika#838 ... by @samidarko` arrived as user-role message. The comment was the `/mika-groom-ticket` Phase-5 closing summary, which contains a verbatim status line: `Ready to dispatch via \`mika ask --agent mika-dev "implement mika issue#838"\``.
- 16:20:29 — task `3582e57c` created (`trigger_type=manual source=self_dev`).
- 16:33:14 — `run_claude_pilot` fired without operator approval.

Source: `~/.mika/data/mika.db`.

**Same failure shape as mika#798 (2026-04-25).** Compound doc exists: `docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md`.

## Why-first

**Vincent's stated rule:** mika-dev dispatches if and only if (a) Vincent prompts mika-dev directly, or (b) the `ready` label is set on the ticket. Anything else (comments, label changes other than `ready`, assignments, discussion replies, automated GitHub events) is inert — at most an acknowledgement.

**Architectural argument (per mika-arch session `3801e5e4-5a7b-4d57-a9e0-f217964c913b`):**
- mika#807 / mika#801 proposed *negative-detection* (heuristic "is grooming active right now? if so, abort"). Negative-detection is **completeness-bound** — heuristics fail on inputs they don't enumerate. A grooming signal not in the rule list bypasses the guard.
- This plan ships *positive-consent* (closure-bound: dispatch fires iff one of the two valid triggers is present, everything else is inert by default). No bypass possible because the rule doesn't enumerate grooming signals — it enumerates the two valid triggers and treats their complement as inert.
- Same allowlist-vs-denylist pattern from security: denylists lose the moment someone invents a new attack shape; allowlists hold by construction.

## Audit results (verified during planning)

### Gateway routing (current state)

**File:** `crates/mika-gateway/src/github.rs:144-160` (`route_event` function).

```rust
match (event_type, action) {
    ("issues", Some("assigned")) => Some("mika-dev"),
    ("issue_comment", Some("created")) => Some("mika-dev"),
    ("pull_request", Some("opened" | "synchronize" | "review_requested")) => Some("mika-qa"),
    ("pull_request", Some("closed")) => Some("mika-dev"),
    ("pull_request_review", Some("submitted")) => Some("mika-dev"),
    ("check_suite", Some("completed")) => match check_conclusion {
        Some("failure" | "timed_out" | "success") => Some("mika-dev"),
        _ => None,
    },
    _ => None,
}
```

**`("issues", Some("labeled"))` is NOT in the allowlist today.** Falls into `_ => None`, silently dropped. The gateway must be extended for the `ready` label to ever reach mika-dev.

### Gateway formatting (current state)

**File:** `crates/mika-gateway/src/github.rs:185-208` (`format_event_text`, `"issues"` arm).

The `"issues"` arm formats `[GitHub] Issue {action}: {repo}#{number} — {title}\n{url}` and adds assignee detail only when action is `assigned`. For action `labeled`, today's logic would produce `[GitHub] Issue labeled: ...` — **without naming the specific label that was just added** (the `event.label.name` field is not extracted).

This is insufficient for mika-dev's pattern matcher. The handler needs an unambiguous marker like `[GitHub] Issue labeled ready on senara-solutions/mika#<n>` so the prompt match doesn't have to parse the body.

### Webhook event payload struct

**File:** `crates/mika-gateway/src/github.rs` (top-of-file struct definitions, lines 55-95 area).

The `GitHubWebhookEvent` struct already has fields for `issue`, `comment`, `pull_request`, `repository`, `action` per the lines I read. **Whether it has `label: Option<Label>` for `issues.labeled` events must be verified during implementation.** GitHub's webhook payload for `issues.labeled` includes a top-level `label` object (the specific label just added). If the struct is missing it, add a `label: Option<Label>` field with serde optional deserialization.

### mika-dev skill prompt (current routing)

**File:** `mika/skills/bundled/self-dev/system_prompt.md`.

- **Layer 1 (lines 9-15):** routing table matches on message-body content. Row: `implement <repo> issue#<n>` → Generic Workflow.
- **Layer 2 (lines 188-196):** Webhook Fallthrough scope rule says `issue_comment.created`, `pull_request.labeled`, `issues.assigned`, `discussion.created` are NOT triggers. "Do NOT create new tasks, do NOT call `run_claude_pilot`."

**Layer 1 has no event-source check.** Any user-role message containing `implement <repo> issue#<n>` matches — regardless of whether it's a direct prompt or a webhook payload. Layer 2 never fires for content-matched messages because Layer 1 has already routed away.

### Label taxonomy

**Files:** `mika/.github/labels.yml`, `mika-platform/.github/labels.yml`. Verified absent: no `ready` / `ready-for-dispatch` / `self-dev:ready` label exists today. Sync mechanism: `EndBug/label-sync` per `reference_label_sync.md` memory.

### Compound doc

`mika/docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md` exists, documenting the failure class. This plan EXTENDS it (adds a `## Resolution` section pointing at this ticket) rather than superseding it — the failure class hasn't changed, only the chosen resolution has.

### Sibling-ticket overlap

mika#838 (`error_max_turns` recovery) is GROOMED on branch `fix/838/recognize-error-max-turns-as-recover-work` but not yet implemented. Both touch `self-dev/system_prompt.md`. **This ticket ships first** (security/correctness, prevents repeat $22+/incident burn). mika#838 rebases onto whatever this lands.

## Approach

Five changes across three layers. One PR, single review cycle.

### Change 1 — Gateway: route `issues.labeled` events to mika-dev

**File:** `crates/mika-gateway/src/github.rs:144-160` (`route_event`).

Add one match arm:

```rust
("issues", Some("labeled")) => Some("mika-dev"),
```

This admits all label-add events on issues to mika-dev. The skill-prompt handler (Change 3) decides which label-add actually triggers dispatch (only `ready` does); other labels match the existing Webhook Fallthrough scope rule and are acknowledged-only.

Add unit test `test_route_event_issue_labeled` mirroring the existing `test_route_event_issue_comment_created` shape.

### Change 2 — Gateway: format `issues.labeled` events with the specific label name

**File:** `crates/mika-gateway/src/github.rs:185-208` (`format_event_text`, `"issues"` arm).

Add an action-specific branch for `labeled` that extracts `event.label.name` (the specific label just added) and emits a structured marker:

```
[GitHub] Issue labeled <name> on <repo>#<n> — <title>
<url>
```

If `event.label.name` is unavailable (struct field missing or null), fall back to the generic `[GitHub] Issue labeled: ...` shape — the skill-prompt handler will refuse to match the unstructured fallback and emit a "label name unparseable" send_message instead of dispatching.

**Implementation prerequisite (verify before writing — Path A / Path B per architect Finding 3):**

```bash
grep -A 20 "struct GitHubWebhookEvent" crates/mika-gateway/src/github.rs
```

- **Path A** (field present): Change 2 stays scoped to `route_event` + `format_event_text` only.
- **Path B** (field absent): Change 2 also adds `label: Option<Label>` to `GitHubWebhookEvent` with serde optional deserialization, plus `Label` struct definition (`{ name, color, description: Option<String> }`) if not present. ~10 LOC additional.

Add format test `test_format_issue_labeled_extracts_label_name` regardless of path.

### Change 3 — mika-dev prompt: source-aware Layer 1 + ready-label handler

**File:** `mika/skills/bundled/self-dev/system_prompt.md`.

Two coordinated changes:

**3a. Restrict Layer 1 routing table (lines 9-15) to direct prompts only.**

Today the table matches on message content alone. Add an explicit precondition: **Layer 1 matches only when the message has no bracketed source-prefix marker** (i.e., direct user prompts arrive without prefixes; channel/webhook deliveries always have them — `[GitHub]`, `[claude-pilot]`, `[Telegram]`, etc.). The inverted rule ("matches when NO bracketed prefix") is more robust than maintaining a whitelist of prefixes — covers any future channel that adopts the bracketed-prefix convention.

Verbatim addition to the routing-table preamble:

> **Source check (mandatory):** This routing table applies ONLY to messages without a bracketed source-prefix marker. If the message starts with any `[<channel>]` prefix (`[GitHub]`, `[claude-pilot]`, `[Telegram]`, etc.), this turn is a webhook or channel delivery — match against the dedicated handler section below, not this table. Direct user prompts to `mika ask` arrive without a prefix.

**Implementation prerequisite (verify before writing — CONDITIONAL DISPATCH-BLOCKER per architect Finding 4):**

This is the load-bearing pre-commit gate. The inverted-rule design depends on `mika ask` direct prompts having no bracketed prefix. Architect's failure-mode analysis: if a future direct-prompt path adds a prefix, the rule **stops matching legitimate prompts** (loud failure — Vincent's prompt doesn't dispatch, he notices) rather than **starts matching webhooks as prompts** (silent failure — burns $22). Inverted is the right failure direction. But the gate must produce evidence.

```bash
grep -rn "fn ask\|run_ask\|format_prompt\|prompt_for_agent" crates/mika-cli/src/
# Locate the prompt formatter; trace from `mika ask` argument parsing to the message string passed to mika-dev.
```

- **Path A** (no bracketed prefix added by `mika ask`): plan as written — inverted rule applies.
- **Path B** (prefix added, e.g., `[CLI]` or `[user]`):
  - **B1** — remove the prefix from the direct-prompt formatter (small CLI change in same PR; ~5 LOC).
  - **B2** — fall back to whitelist-against-known-channels: rule changes to "match `implement <repo> issue#<n>` ONLY when message starts with one of `[Direct]` / `[CLI]` / `[<known direct prefix>]` OR has no prefix at all." Maintenance burden: every new channel must be reviewed.

The implementer commits to one path before writing Change 3a. Architect's pre-stated preference: B1 over B2 (B2 reintroduces completeness-bound risk this plan exists to remove).

**3b. Add an explicit `ready`-label dispatch handler.**

Above the Webhook Fallthrough scope rule, add a new handler section:

> ### Ready-Label Dispatch
>
> When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label on the ticket. This is the canonical positive-consent signal. **Atomic handler (per mika-arch Finding 4 — label removal first, task creation second):**
>
> 1. **First**, call `run_gh("issue edit <n> --repo <repo> --remove-label ready")` to remove the consent signal.
>
>    **Failure handler (per architect Finding 5 — concrete shape required):** if `run_gh` returns non-zero exit:
>    - Log the failure with the gh stderr captured to a structured note.
>    - Do NOT call `create_task`.
>    - Do NOT call `run_claude_pilot`.
>    - Send operator a `send_message` with this verbatim payload shape:
>      ```
>      Ready-label dispatch aborted on <repo>#<n>: --remove-label failed.
>      gh stderr: <captured stderr>
>      Re-add the `ready` label to retry, or check label permissions.
>      ```
>    - Stop the turn.
>
>    The label-removal-first ordering ensures: if task creation fails, the operator can re-add the label to retry; if order were reversed, a task-creation failure would leave the label persisting and expose a re-dispatch race the next time a webhook arrives for the same issue.
>
> 2. **Then** route to Generic Workflow Step 1 (fetch issue body, create task, dispatch claude-pilot).

Other label-add events (`bug`, `enhancement`, `priority`, `p1-important`, etc.) match the Webhook Fallthrough scope rule unchanged: acknowledge, do NOT dispatch.

### Change 4 — Label taxonomy: add `ready` to both repos

**Files:**
- `mika/.github/labels.yml`
- `mika-platform/.github/labels.yml`

Add label `ready` with state-oriented description aligned to existing convention (per architect Finding 5 — existing labels use state-oriented phrasing like "Something isn't working"):

```yaml
- name: ready
  color: "0e8a16"  # green; tentative — verify against existing color palette
  description: Approved for autonomous dispatch
```

**Pre-commit verification (per architect Finding 5):**
- Confirm `EndBug/label-sync` mode does NOT delete labels not in YAML (or confirm the existing label catalog is fully captured before adding `ready`). If delete-mode and catalog incomplete, snapshot existing labels via `gh label list` first.
- Color choice: green (`0e8a16`) signals "go." Verify against existing repo palette to avoid collision with another label using the same hex.

### Change 5 — Compound doc: extend with Resolution section

**File:** `mika/docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md`.

Add a new `## Resolution` section at the end. Per architect Finding 9, the section must name BOTH the architectural decision AND the complete reasoning chain so future readers see the full history:

- **Origin:** mika#798 (2026-04-25) incident — first observed instance of the failure class.
- **Recurrence:** mika#838 (2026-04-27) — same shape, triggered by `/mika-groom-ticket` Phase-5 closing comment containing `implement mika issue#838` substring.
- **Initial proposed solutions (superseded):**
  - mika#807 — three-source contract tightening via skill-prompt rules (negative-detection).
  - mika#801 — `check_active_grooming(issue, repo)` heuristic helper (negative-detection).
- **Final resolution:** positive-consent dispatch gate via `ready` label (mika#841).
- **Architectural argument (Vincent's two-way rule):** dispatch fires iff (a) Vincent prompts mika-dev directly OR (b) `ready` label is set on the ticket. Closure-bound: the rule enumerates the two valid triggers; everything else is inert by default. Negative-detection (mika#807/#801) was completeness-bound — heuristics fail on inputs they don't enumerate. Same allowlist-vs-denylist pattern from security: denylists lose the moment someone invents a new attack shape.
- **Architect review:** mika-arch session `3801e5e4-5a7b-4d57-a9e0-f217964c913b` (lifecycle ESCALATE → operator-resolved as supersession), session `8959665a-7fa4-4e39-bc36-da48faf0d50d` (#841 grooming).

The doc records the failure class (unchanged); the new section records the chosen resolution and the reasoning chain that got us there. **Doc lifecycle is decoupled from ticket lifecycle** (per peer review recommendation): tickets get superseded; docs get extended.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `crates/mika-gateway/src/github.rs` | +1 line in `route_event` (`("issues", Some("labeled"))` arm) + ~10 lines unit test |
| 2 | `crates/mika-gateway/src/github.rs` | +~15 lines in `format_event_text` (`labeled` action branch) + ~10 lines format test; possibly +3 lines `label: Option<Label>` field on `GitHubWebhookEvent` if not present |
| 3 | `mika/skills/bundled/self-dev/system_prompt.md` | +~10 lines source-check preamble on Layer 1 + ~25 lines ready-label handler section |
| 4 | `mika/.github/labels.yml` | +3 lines (label entry) |
| 4 | `mika-platform/.github/labels.yml` | +3 lines (label entry) |
| 5 | `mika/docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md` | +~30 lines (Resolution section) |

Estimated diff: ~100 lines across 6 files (5 in mika, 1 in mika-platform). Single PR for mika; companion 3-line PR for mika-platform's labels.yml (cross-repo).

## Tests

1. **Build verification:** `cargo check -p mika-gateway` succeeds. `cargo test -p mika-gateway` passes (existing + new unit tests).
2. **Behavioral test (positive — `ready` dispatch):** simulate `[GitHub] Issue labeled ready on senara-solutions/mika#<n>` event delivery. Confirm:
   - mika-dev calls `run_gh("issue edit <n> --remove-label ready")` BEFORE creating the task.
   - mika-dev creates task with `trigger_type=manual source=self_dev`.
   - mika-dev calls `run_claude_pilot` for the issue.
3. **Behavioral test (negative — comment with content match):** simulate `[GitHub] New comment on <repo>#<n> ... <body containing 'implement mika issue#N'>`. Confirm mika-dev does NOT call `run_claude_pilot`. Webhook Fallthrough scope rule should match instead.
4. **Behavioral test (miss — wrong label):** simulate `[GitHub] Issue labeled bug on <repo>#<n>`. Confirm mika-dev does NOT call `run_claude_pilot`. Webhook Fallthrough scope rule matches.
5. **Direct-prompt regression:** simulate a direct prompt `implement mika issue#100` to mika-dev (no bracketed prefix). Confirm Layer 1 matches and Generic Workflow runs.
6. **Atomicity test:** force `run_gh("issue edit ... --remove-label ready")` to fail. Confirm task is NOT created and operator is notified.
7. **Doc lint:** `mika-doc-audit` passes on the extended compound doc.
8. **Manual end-to-end:** add `ready` label to a real issue; observe mika-dev removes it and dispatches claude-pilot. Add a different label; observe mika-dev acknowledges only.

## Acceptance criteria

- [ ] `crates/mika-gateway/src/github.rs` `route_event` handles `("issues", Some("labeled")) => Some("mika-dev")`.
- [ ] `crates/mika-gateway/src/github.rs` `format_event_text` emits `[GitHub] Issue labeled <name> on <repo>#<n>` for label-add events; `event.label.name` accessible on `GitHubWebhookEvent`.
- [ ] `mika/skills/bundled/self-dev/system_prompt.md` Layer 1 routing table is preceded by an explicit source-check that requires absence of bracketed prefix.
- [ ] `mika/skills/bundled/self-dev/system_prompt.md` includes a Ready-Label Dispatch handler section that runs `run_gh("--remove-label ready")` BEFORE Generic Workflow Step 1.
- [ ] `mika/.github/labels.yml` and `mika-platform/.github/labels.yml` declare a `ready` label with state-oriented description and a color verified against existing palette.
- [ ] `EndBug/label-sync` safety verified before commit (delete-mode catalog complete OR add-only mode).
- [ ] `ambient-webhook-mistaken-for-dispatch-2026-04-25.md` extended with `## Resolution` section citing this ticket and the closure-bound rationale.
- [ ] mika#807 and mika#801 are CLOSED with `Superseded by mika#841` comments. (Done at ticket-creation time.)
- [ ] All 8 tests in §Tests pass (unit + behavioral + manual end-to-end).
- [ ] `cargo check -p mika-gateway` and `cargo test -p mika-gateway` succeed.
- [ ] Direct-prompt path verified to not add bracketed prefix (architect Finding 6 — pre-commit verification).

## Out of scope

- **Heuristic active-grooming guard** (mika#801's `check_active_grooming` helper). Closure-bound consent makes it redundant; closing #801 captures this.
- **Tightened dispatch-trigger contract via skill prompt** (mika#807's three-source contract). Replaced by positive-consent; closing #807 captures this.
- **Surface fix to `/mika-groom-ticket` closing comment** (drop the verbatim `mika ask` line). Not needed once Layer 1 is source-aware — comment events match Webhook Fallthrough regardless of body content. Optional follow-up if the closing comment ever needs different operator-actionable wording.
- **Auto-add `ready` label as part of `/mika-groom-ticket` Phase 5.** Tempting (would close the loop: groom → label → dispatch automatically), but defeats the positive-consent gate's purpose (Vincent must explicitly approve). Future ticket if grooming gains an "approve and dispatch" mode.
- **Other channels' source prefixes.** `[Telegram]` and `[claude-pilot]` are already handled by their own dedicated handlers; this plan only adds the `[GitHub] Issue labeled ready ...` recognition. If a new channel arrives with a different prefix shape, that's a separate skill-prompt change.
- **Failure-class search in `/mika-groom-ticket` pre-grooming step** (the structural guard against missing prior art that Vincent's friend recommended). Separate small ticket — keeps this PR focused.
- **Pre-filing scope-verification discipline compound doc** (per architect Finding 2). The "spend 15-20 minutes verifying actual layer scope before drafting the plan" pattern that produced this ticket's accuracy belongs in `mika/docs/solutions/best-practices/pre-filing-scope-verification-2026-04-27.md` as a named discipline (analogous to pre-commit discovery). File as a ~15-line compound doc after this ticket lands.
- **A `block[security]` / `p0-critical` auto-dispatch label.** Vincent's stated rule is exactly two triggers; emergency dispatch can ride direct prompts when needed. Future consideration only if frequency justifies it.

## Risks

| Risk | Mitigation |
|---|---|
| `EndBug/label-sync` runs in delete-mode and removes labels not yet in YAML | Pre-commit gate per Finding 5: `gh label list` for both repos and confirm catalog is fully captured before adding `ready`. If gap, snapshot first. |
| Direct prompts via `mika ask` DO add a bracketed prefix that I haven't found | Pre-commit verification: read `crates/mika-cli/` prompt formatter. If a prefix exists, switch from inverted-rule to whitelist (allow `[user]` or whatever direct prefix is). Plan-level note: prefer inverted rule, fall back to whitelist if needed. |
| `GitHubWebhookEvent` struct missing `label` field; gateway formatter falls back to unstructured shape; mika-dev's matcher fails to extract label name | Add the field as part of Change 2 if missing. Format test asserts the structured shape; build fails if the field is missing. |
| Race window: webhook delivers same `ready`-label-add twice (e.g., gateway DLQ replay) | Idempotency via the label-removal-first atomicity: second delivery finds the label already removed, fails the `--remove-label` step, aborts (operator-notified). The current `tasks` table's reuse logic also catches duplicate task creation. |
| `pull_request.labeled` events not handled (only `issues.labeled` added) | Out of scope for v1 — Vincent's rule is about issue-level dispatch, not PR-level. PR labeling already handled by `pull_request_review.submitted` flow. Document in plan to prevent scope creep. |
| Layer 1 prefix-check is bypassed by a creative attacker injecting non-bracketed `implement` content via a webhook that doesn't add a prefix | Verify gateway adds `[GitHub]` prefix on ALL forwarded events (read `format_event_text` exit points). If any event type bypasses the prefix, that's a gateway bug to fix separately. |
| mika#838 rebase conflict (both touch `self-dev/system_prompt.md`) | Mechanical resolution: this ticket lands first; #838's plan re-bases onto the new prompt structure. The two changes don't overlap structurally (#838 adds `error_max_turns` recovery in close-out; this adds source-aware routing + ready-label handler at the top). Acceptable cost. |
| Manual end-to-end test requires labelling a real issue | Use a throwaway issue (e.g., a test issue created in a private fork or sandbox repo) to avoid polluting production tickets with test labels. Document the test repo in PR description. |

## Sequencing

1. **Pre-commit verifications (do FIRST, before any code change — per architect Finding 10, name commands + expected output shapes):**

   **Verification 1 — `GitHubWebhookEvent.label` field presence:**
   ```bash
   grep -A 20 "struct GitHubWebhookEvent" crates/mika-gateway/src/github.rs
   ```
   Expected output: either field `label: Option<Label>` is present (Path A — Change 2 unchanged) or absent (Path B — Change 2 also adds the field + `Label` struct).

   **Verification 2 — `mika ask` direct-prompt prefix (CONDITIONAL DISPATCH-BLOCKER):**
   ```bash
   grep -rn "fn ask\|run_ask\|format_prompt\|prompt_for_agent" crates/mika-cli/src/
   ```
   Expected output: locate the prompt formatter; trace from `mika ask` argument parsing to the message string passed to mika-dev. Confirm message has no `[...]` bracketed prefix (Path A) or identify the prefix (Path B → choose B1 strip-prefix or B2 whitelist).

   **Verification 3 — `EndBug/label-sync` mode safety:**
   ```bash
   grep -E "delete-other-labels|add-only|allow-removed-labels" .github/workflows/label-sync.yml
   # OR equivalent path; check `.github/workflows/` for the label-sync workflow name.
   gh label list --repo senara-solutions/mika --json name,color,description > /tmp/mika-labels-before.json
   gh label list --repo senara-solutions/mika-platform --json name,color,description > /tmp/mika-platform-labels-before.json
   ```
   Expected output: confirm sync mode (delete-mode vs add-only). If delete-mode, verify the YAML catalogs are complete vs the JSON snapshots before adding `ready`. If gap, populate first.

   **Verification 4 — Ready-Label handler does not rely on cached issue state (per architect Finding 6):**
   The handler in Change 3 calls `gh issue view` which hits GitHub's API directly. Confirm no cached `issue` field on the webhook payload is used as the source of truth for the issue body or labels — only the marker shape `[GitHub] Issue labeled ready on <repo>#<n>` is parsed. GitHub doesn't guarantee delivery order between `issues.opened` and `issues.labeled` if both fire simultaneously; relying on cached state would race.

   **Verification 5 — Existing label color palette:**
   ```bash
   gh label list --repo senara-solutions/mika --json name,color --jq '[.[] | "\(.color) \(.name)"] | sort | .[]'
   ```
   Expected output: pick non-colliding color for `ready`. Default proposal: `0e8a16` (green, "go"); change if collision found.
2. **Change 1 + 2** (gateway: routing + formatting). Server-side change, independent of skill prompt.
3. **Change 4** (labels.yml additions). Independent; can run in parallel with Change 2 since label-sync is GitHub-side.
4. **Change 3** (skill prompt: Layer 1 source-check + ready-label handler). Depends on Change 1 + 2 (the event must be deliverable before the handler matters).
5. **Change 5** (compound doc extension). Documents shipped behavior.
6. **Run §Tests 1-7.** Manual end-to-end (test 8) at PR-review time.
7. **Open PR** referencing #841, citing supersession of #807 + #801, citing `ambient-webhook-mistaken-for-dispatch-2026-04-25.md`.
8. **Companion PR for mika-platform/.github/labels.yml** (single label entry; trivial review).

## Verification

```bash
# Confirm gateway routing
grep -c '"issues", Some("labeled")' mika/crates/mika-gateway/src/github.rs  # → 1
grep -c "label_name\|event.label" mika/crates/mika-gateway/src/github.rs  # → ≥ 1 (formatter extracts label name)

# Confirm skill prompt
grep -c "no bracketed source-prefix\|no bracketed prefix\|bracketed.*prefix" mika/skills/bundled/self-dev/system_prompt.md  # → ≥ 1
grep -c "Ready-Label Dispatch\|Issue labeled ready" mika/skills/bundled/self-dev/system_prompt.md  # → ≥ 2 (header + handler match string)
grep -c "remove-label ready" mika/skills/bundled/self-dev/system_prompt.md  # → 1

# Confirm labels
grep -c "name: ready" mika/.github/labels.yml mika-platform/.github/labels.yml  # → 2

# Confirm doc extension
grep -c "## Resolution" mika/docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md  # → 1
grep -c "mika#841" mika/docs/solutions/workflow-issues/ambient-webhook-mistaken-for-dispatch-2026-04-25.md  # → ≥ 1

# Build
cargo check -p mika-gateway
cargo test -p mika-gateway
```

## Discovery items (verified during planning)

1. **Gateway scope is real, not skill-prompt-only.** `route_event` at `crates/mika-gateway/src/github.rs:144-160` does not include `("issues", Some("labeled"))`. The label-add event is silently dropped today. Reading the gateway BEFORE filing the ticket is what made this scope visible (per peer review).
2. **mika#807 + mika#801 already exist** on the same problem class. Closed with `Superseded by mika#841` comments. Architect surfaced this; brief author had missed it. Compound: failure-class search before filing tickets is a structural-discipline gap (separate small ticket out of scope).
3. **`ambient-webhook-mistaken-for-dispatch-2026-04-25.md` already documents the failure class.** This plan extends, not supersedes — doc lifecycle decoupled from ticket lifecycle.
4. **Closure-bound vs completeness-bound** is the load-bearing architectural argument. Negative-detection (mika#807/#801) is heuristic and bypassable; positive-consent is closure-bound. Same as security allowlist-vs-denylist. Recorded in the compound doc's Resolution section.
5. **Three pre-commit verifications** are required before code change: `GitHubWebhookEvent.label` field presence, direct-prompt no-prefix assumption, label-sync delete-mode safety. Each was named by the architect or peer reviewer; sequencing block in §Sequencing makes them mandatory.
6. **mika#838 sequencing:** this ticket ships first (correctness/security); #838 rebases. Both touch `self-dev/system_prompt.md` but in different sections; mechanical rebase.

---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
plan_type: fix
issue: senara-solutions/mika#1593
branch: fix/1593/engine-dispatch-dev-groom-pilots-policy
created: 2026-06-27
---

# fix: Engine-side dispatch emits owner-qualified prompt that dispatch-lib can't parse (mika#1593)

## Summary

The engine-side ready-label dispatch handler (mika#1572, PR #1589) builds the dispatch
prompt as `senara-solutions/mika#<n>` (owner-qualified). dispatch-lib's worktree-setup
parser only accepts the **bare** `mika#<n>` form documented by the dispatch tool schemas,
so the owner-qualified prompt fails the parse, silently routes to no-worktree "free-text"
mode, and the inner claude-pilot session improvises worktree setup by calling
`scripts/derive-branch-name` directly — which claude-pilot-py's tier-1 classifier denies.
Result: **every** engine-side dev-groom (and dev-pilot) dispatch wedges. Three tickets
(#1591, #1576, #1573) confirmed wedged on 2026-06-27.

The fix restores the documented `repo#number` contract at every site that emits a dispatch
prompt, and hardens dispatch-lib's parser so an owner-qualified ref can never again *silently*
route to no-worktree mode.

---

## Problem Frame

### Corrected root cause (verified from code — the ticket's stated cause is imprecise)

The ticket (mika#1593) hypothesizes that the engine-side handler "does NOT call dispatch-lib's
worktree-setup phase." That is **wrong**. The handler *does* invoke the same
`handlers/run.sh` → `dispatch-lib.sh` → `_set_up_worktree` path the LLM-tool dispatch used.
The actual defect is a **prompt-format mismatch**:

1. The engine handler builds the dispatch prompt at
   `crates/mika-agent/src/server/ready_label_handler.rs:276` as
   `format!("{}#{}", location.owner_repo(), location.number)`.
   `owner_repo()` (`ready_label_handler.rs:49-55`) **always** returns the owner-qualified
   form — it prepends `senara-solutions/` to a bare `repo_ref`, or returns `repo_ref`
   unchanged when it already contains a slash. So the prompt is e.g. `senara-solutions/mika#1576`.

2. dispatch-lib's `_set_up_worktree` parses the prompt at
   `skills/bundled/_shared/dispatch-lib.sh:404` with the anchored regex
   `^[a-zA-Z0-9_-]+#[0-9]+$`. A slash is **not** in that character class, so
   `senara-solutions/mika#1576` does not match → `REPO` and `ISSUE_NUM` stay empty.

3. With `REPO` empty, `_set_up_worktree` falls through to the `else` branch at
   `dispatch-lib.sh:623` ("Free-text mode: pass prompt as-is, no worktree",
   `CWD_ARGS="--cwd $PLATFORM_DIR"`). No worktree is created; claude-pilot launches in the
   meta-repo root.

4. The inner pilot (running `ENTRY_COMMAND` `/mika-groom-plan-only` for dev-groom) finds no
   worktree, improvises setup, and calls `scripts/derive-branch-name` — denied by
   claude-pilot-py's tier-1 classifier (path not in `SAFE_SHELL_COMMANDS`). `PIPELINE FAILURE`.

5. The dispatch tool schemas — `skills/bundled/dev-groom/tools.json` and
   `skills/bundled/dev-pilot/tools.json` — document the prompt contract as the **bare** form
   (`'mika#214'`, `'mika-skills#8'`). The historical LLM-tool dispatch passed the bare form,
   which is why it worked pre-#1572. mika#1572 (commit `cdbeeedf`) regressed it by emitting the
   owner-qualified form from the engine path.

### Second emission site (also broken)

The #1571 **prescriptive fallback** pre-digest — `format_ready_label_pre_digest`
(`ready_label_handler.rs:464-512`) — instructs the LLM (Resolution F3 degraded path) to call
the dispatch tool with `"prompt": "<owner_repo>#<n>"` (placeholder filled by `owner_repo` at
`ready_label_handler.rs:504`). The existing test `pre_digest_names_required_args` asserts
`"prompt": "senara-solutions/mika#1384"`. So when the engine path falls back to #1571, the
LLM is told to pass the **same** broken owner-qualified form. A complete fix corrects both
emitters.

`format_engine_dispatch_pre_digest` (the post-dispatch "already fired" digest) embeds **no**
dispatch prompt argument — it only renders the owner-qualified marker as cosmetic display —
so it needs no change.

### Why it was costly

The failure was **silent**: the owner-qualified ref didn't error, it routed to a legitimate
code path (free-text mode) that just happened to be wrong for a ticket ref. Each wedge cost
~$0.20 and ~7 pilot turns before failing. Hardening the parser to reject/normalize this case
turns a silent wedge into either correct behavior (normalize) — the chosen mitigation.

---

## Requirements

Traceability to the ticket's acceptance criteria:

- **R1 (AC1).** A `ready`-labeled, ungroomed ticket dispatches dev-groom and reaches
  `Verdict: GROOMED` or `Verdict: ESCALATE` with **no** `policy:deny` on
  `scripts/derive-branch-name`. Achieved by emitting the bare `repo#number` the worktree-setup
  parser accepts, so dispatch-lib creates the worktree and the inner pilot never improvises.
- **R2 (AC2).** Branch-name centralization (mika-platform#58) is preserved: dispatch-lib's
  `derive-branch-name` script stays the sole brancher. No LLM-improvised branch names are
  introduced — the fix removes the *need* for the inner pilot to derive anything.
- **R3 (AC3).** The pilot-no-push boundary (mika#1318) is untouched: no change to push behavior,
  the `pilot_push_guard`, or the iterate/push sequencing.
- **R4 (AC4).** Verifiable by dispatching a fresh ungroomed ticket post-deploy and observing a
  successful groom + callback to implement. (Operator-gated runtime test — see Verification.)
- **R5.** The fix covers **both** `run_claude_pilot_groom` (dev-groom) and `run_claude_pilot`
  (dev-pilot), since the engine path and prompt emitters are shared across both skills.

---

## Key Technical Decisions

### KTD1 — Fix the producer (emit bare `repo#number`), and harden the consumer (parser)

Two independent layers, both included:

- **Producer fix (primary, necessary + sufficient for R1):** every site that emits a *dispatch
  prompt* emits the documented bare `repo#number`. Restores the exact contract the LLM-tool
  path used, the tool schemas document, and dispatch-lib + the inner `/mika*` commands expect.
- **Consumer hardening (defense-in-depth):** dispatch-lib's `_set_up_worktree` parser accepts an
  optional `owner/` prefix and normalizes `REPO` to the basename. This means a future or
  alternate caller that passes an owner-qualified ref is *normalized* into worktree mode instead
  of silently dropped into free-text mode.

Rationale for doing both: the producer fix alone closes the wedge, but the silent-fall-through
is the property that made this expensive and hard to diagnose. The parser hardening is ~2 lines
and removes the silent-failure class for this input shape entirely. Neither weakens #58 — the
script remains the sole brancher in both layers.

### KTD2 — Add a `repo_name()` accessor; do not mutate `owner_repo()` or the display markers

`owner_repo()` is correctly used for `gh` calls (`fetch_issue_body`, task labels, logs) and for
the cosmetic `[GitHub] Issue labeled ready on …` marker lines that mirror the real webhook
marker. Those must stay owner-qualified. Introduce a separate `repo_name()` accessor returning
the **basename** of `repo_ref` (strip any `owner/` prefix), and use it **only** where a dispatch
prompt is constructed. This keeps the two concerns (display/`gh` identity vs. dispatch-prompt
contract) cleanly separated.

### KTD3 — Keep the parser regex anchored; normalize by stripping to the last `/`

Broaden the regex to `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` (optional owner segment), still
fully `^…$`-anchored so genuine free-text prompts with an embedded `#` still fall through to
free-text mode as before. Derive `REPO` by stripping everything up to and including the last `/`.
dispatch-lib already hardcodes `senara-solutions/` as the owner for the `gh issue view` call
(`dispatch-lib.sh:425`), so normalizing to the basename is consistent with the existing
owner assumption.

---

## High-Level Technical Design

Dispatch-prompt flow, before vs. after:

```
BEFORE (#1572 regression)
  ready_label_handler ──prompt="senara-solutions/mika#1576"──▶ dispatch-lib _set_up_worktree
                                                                  regex ^[A-Za-z0-9_-]+#[0-9]+$
                                                                  ── NO MATCH (slash) ──▶ REPO=""
                                                                  ──▶ else: FREE-TEXT, no worktree
                                                                  ──▶ pilot improvises
                                                                       derive-branch-name ──▶ policy:deny ✗

AFTER (this fix)
  ready_label_handler ──prompt="mika#1576"──────────────────▶ dispatch-lib _set_up_worktree
   (repo_name(), not owner_repo())                              regex ^([A-Za-z0-9_-]+/)?…#[0-9]+$
                                                                  ── MATCH ──▶ REPO="mika"
                                                                  ──▶ worktree-setup: derive-branch-name
                                                                       + derive-worktree-path + worktree add
                                                                  ──▶ pilot runs /mika-groom-plan-only
                                                                       in ready worktree ──▶ GROOMED ✓

  (defense-in-depth) even if a caller passes "senara-solutions/mika#1576",
   the broadened parser normalizes REPO="mika" → worktree mode, not free-text.
```

---

## Implementation Units

### U1. Add `repo_name()` accessor and emit bare `repo#number` from both dispatch-prompt sites

**Goal:** Stop emitting owner-qualified dispatch prompts. Restore the documented bare
`repo#number` contract at the engine-side spawn and the #1571 fallback pre-digest.

**Requirements:** R1, R2, R5.

**Dependencies:** none.

**Files:**
- `crates/mika-agent/src/server/ready_label_handler.rs` (modify)

**Approach:**
- Add `ReadyLabelLocation::repo_name(&self) -> String` returning the basename of `repo_ref`:
  if `repo_ref` contains `/`, return the segment after the last `/`; else return `repo_ref`
  unchanged. Mirror the doc-comment style of `owner_repo()`.
- At the engine-side dispatch input (`ready_label_handler.rs:276`), change the prompt to
  `format!("{}#{}", location.repo_name(), location.number)`.
- In `format_ready_label_pre_digest`, introduce `let repo_name = loc.repo_name();` and fill the
  **dispatch-prompt** placeholder (`"prompt": "{}#{}"`) with `repo_name` instead of `owner_repo`.
  Leave the cosmetic `[GitHub] Issue labeled ready on {owner_repo}#{n}` marker line unchanged
  (it legitimately mirrors the owner-qualified webhook marker).
- Do **not** touch `owner_repo()` itself, `fetch_issue_body`, task labels, logging, or
  `format_engine_dispatch_pre_digest` (no dispatch prompt embedded there).

**Patterns to follow:** existing `owner_repo()` accessor shape (`ready_label_handler.rs:49-55`);
existing format-arg ordering in `format_ready_label_pre_digest`.

**Test scenarios** (`crates/mika-agent/src/server/ready_label_handler.rs` `#[cfg(test)] mod tests`):
- `repo_name()` returns `"mika"` for `repo_ref = "senara-solutions/mika"`.
- `repo_name()` returns `"mika"` for bare `repo_ref = "mika"`.
- `repo_name()` returns `"mika-cloud"` for `repo_ref = "senara-solutions/mika-cloud"`.
- Update `pre_digest_names_required_args` (`ready_label_handler.rs:633`): the dispatch-prompt
  assertion becomes `"prompt": "mika#1384"` (bare). The other assertions (tool, skill, task_id,
  `UNGROOMED`, `MUST NOT`) are unchanged. Add an assertion that the cosmetic marker line still
  shows the owner-qualified `senara-solutions/mika#1384` (proves only the prompt arg changed).
- New test `pre_digest_groom_prompt_is_bare_for_mika_cloud`: build a pre-digest with
  `repo_ref = "senara-solutions/mika-cloud"` and assert it contains `"prompt": "mika-cloud#<n>"`.
- `Covers AC1 / AC2.`

**Verification:** `cargo test -p mika-agent ready_label` passes; no remaining test asserts an
owner-qualified dispatch-prompt arg.

### U2. Harden dispatch-lib `_set_up_worktree` to normalize an optional `owner/` prefix

**Goal:** A ticket ref carrying an `owner/` prefix is normalized into worktree mode instead of
silently routed to free-text/no-worktree mode.

**Requirements:** R1 (defense-in-depth), R2.

**Dependencies:** none (independent of U1; both land together).

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify)

**Approach:**
- At the parse guard (`dispatch-lib.sh:404`), broaden the regex to
  `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` (optional leading `owner/` segment, still
  fully anchored).
- When deriving `REPO`, strip the owner prefix: take the substring after the last `/` (e.g.
  `sed 's#.*/##'` applied to the pre-`#` portion), so `senara-solutions/mika#1576` →
  `REPO=mika`, `ISSUE_NUM=1576`; bare `mika#1576` is unaffected.
- No change to the `else` free-text branch, the hardcoded `senara-solutions/$REPO` gh call
  (`dispatch-lib.sh:425`), branch derivation, or worktree creation.

**Patterns to follow:** existing `printf | grep -qE` + `sed` extraction idiom already in
`_set_up_worktree` (`dispatch-lib.sh:404-407`).

**Test scenarios:**
- If a dispatch-lib test harness exists (look for `bats`, `*.bats`, or shell test scripts under
  `skills/`, `tests/`, or referenced by `make verify-bundled-skills`), add cases: bare
  `mika#1576` → `REPO=mika`/`ISSUE_NUM=1576`; owner-qualified `senara-solutions/mika#1576` →
  `REPO=mika`/`ISSUE_NUM=1576`; `mika-cloud#50` → `REPO=mika-cloud`; a genuine free-text prompt
  with an embedded `#` (e.g. `fix the foo#bar thing and more`) still does **not** match (stays
  free-text). Prefer the dry-run path (`DRY_RUN=1`, which already emits parsed `repo`/`issue`
  JSON at `dispatch-lib.sh:637-643`) to assert parse output without launching claude-pilot.
- If no shell test harness exists, document the manual dry-run verification in the PR body and do
  not invent a new harness in this PR (note it as deferred follow-up). `Test expectation: covered
  by dry-run manual verification when no harness exists.`

**Verification:** `make verify-bundled-skills` passes (structural integrity); dry-run parse of an
owner-qualified ref yields the bare `REPO` + correct `ISSUE_NUM`.

---

## Scope Boundaries

In scope:
- Bare-`repo#number` emission at the engine-side dispatch input and the #1571 fallback pre-digest.
- A `repo_name()` accessor on `ReadyLabelLocation`.
- dispatch-lib parser normalization of an optional `owner/` prefix.
- Unit tests for the above.

### Deferred to Follow-Up Work
- Adding a dedicated shell unit-test harness (bats) for dispatch-lib parsing, if none exists
  today. Worth doing but out of scope for a p0 loop-breaker fix; track separately if absent.
- Any broader refactor of `owner_repo()` vs `repo_name()` call sites beyond the dispatch-prompt
  emitters.

### Non-goals
- Changing `owner_repo()`, `fetch_issue_body`, task-label/log formatting, or the cosmetic
  ready-label marker display lines.
- Touching `format_engine_dispatch_pre_digest` (embeds no dispatch prompt).
- Any change to the grooming-readiness gate, push guard (#1318), or branch-name script (#58).

---

## System-Wide Impact

- **Loop substrate (tier-1):** unwedges every engine-side dev-groom and dev-pilot dispatch. This
  is the only thing blocking the autonomous loop as of 2026-06-27.
- **Deploy coupling:** dispatch-lib is **copy-deployed** to `~/.mika/agents/<agent>/skills/…`
  (and the bundled-skill seed), not compiled into the binary. The Rust change (U1) ships in the
  mika-spirit binary; the dispatch-lib change (U2) ships via bundled-skill re-sync on restart.
  Both require `make deploy` + restart to take effect live. Main-merged ≠ live (see
  `project_dispatch_lib_deploy_lag_wedge_2026-05-30`).
- **Downstream consumers:** none beyond the dispatch path; the prompt contract is internal.

---

## Risks & Dependencies

- **Risk: an emission site is missed.** Mitigation: U1 grep-audit for `owner_repo()` usages
  confirms only `ready_label_handler.rs:276` (engine spawn) and `:504` (fallback pre-digest)
  feed a dispatch *prompt*; all other `owner_repo()` uses are `gh`/display/log and stay.
- **Risk: parser change breaks free-text dispatch.** Mitigation: the regex stays `^…$`-anchored,
  so only whole-string `[owner/]repo#number` matches; embedded-`#` free-text is unaffected. Test
  scenario in U2 asserts this.
- **Risk: deploy lag masks the fix.** Mitigation: System-Wide Impact notes both artifacts and the
  restart requirement; AC4 runtime test must run against a deployed+restarted server.

---

## Verification Contract

- **Gate 1 (build/lint):** `cargo build -p mika-agent`, `cargo clippy -p mika-agent`,
  `cargo fmt --check`.
- **Gate 2 (unit tests):** `cargo test -p mika-agent ready_label` — all pass, including the new
  `repo_name()` and bare-prompt assertions.
- **Gate 3 (skill integrity):** `make verify-bundled-skills` passes.
- **Gate 4 (AC4 runtime, operator-gated):** after `make deploy` + restart, label a fresh
  ungroomed ticket `ready`; observe (a) dispatch-lib creates the worktree (no free-text
  fall-through), (b) inner pilot does **not** call `scripts/derive-branch-name`, (c) no
  `policy:deny`, (d) `Verdict: GROOMED` or `ESCALATE` + callback to implement. Confirm via the
  task's claude-pilot log and `pilot_push_guard.clean` (Signal M) in server.log.

## Definition of Done

- U1 + U2 implemented; Gates 1–3 green in CI.
- No test asserts an owner-qualified dispatch-prompt argument.
- PR body documents the corrected root cause, the two-layer fix, the deploy-lag caveat, and the
  AC4 runtime-verification steps (operator-gated post-deploy).
- `Closes #1593`.

---

## Sources & Research

- `crates/mika-agent/src/server/ready_label_handler.rs` — `owner_repo()` (49-55), engine dispatch
  input (276), `format_ready_label_pre_digest` (464-512, prompt arg at 504), tests (564-790).
- `skills/bundled/_shared/dispatch-lib.sh` — `_set_up_worktree` parse (399-470), free-text
  fall-through (623-633), dry-run parse output (637-643).
- `skills/bundled/dev-groom/tools.json`, `skills/bundled/dev-pilot/tools.json` — documented bare
  `repo#number` prompt contract.
- mika#1572 / PR #1589 / commit `cdbeeedf` — engine-side dispatch (the regressing change).
- mika-platform#58 — branch-name centralization. mika#1318 — pilot-no-push boundary.
- Evidence: tickets #1591, #1576, #1573 wedged 2026-06-27.

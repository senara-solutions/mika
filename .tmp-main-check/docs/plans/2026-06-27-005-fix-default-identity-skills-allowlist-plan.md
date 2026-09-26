---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: senara-solutions/mika#1596
branch: fix/1596/skills-default-agent-identity-lacks
plan_type: fix
created: 2026-06-27
---

# fix: Default agent identity ships a narrow `[skills]` allowlist (stop engineering-skill Disposition leak)

## Summary

The default agent identity written by `bootstrap_fresh_install` has **no `[skills]` block**.
A missing/empty allowlist is treated as *default-permissive* — every bundled engineering
skill (`dev-pilot`, `dev-groom`, `mika-arch-*`, `qa-review*`, `self-dev*`) loads into the
agent, and the model mimics their internal output contract, appending `Disposition: READY`
to user-facing replies. Vincent reproduced this today (2026-06-27) on his personal `mika`
agent over Telegram.

The fix (Shape A) is to write a **narrow operator-assistant allowlist** into the default
identity template (`DEFAULT_IDENTITY`). An explicit `[skills]` block in a provisioned
`identity.toml` still overrides, so the change is backward-compatible. This unblocks the
Mika Cloud family rollout (mika-cloud#111), where `add-customer.sh` invoked without
`--identity-dir` falls through to `bootstrap_fresh_install` and would otherwise ship the
leak to every family customer. Family go-live target: **Monday 2026-06-29**.

---

## Problem Frame

**Root cause (verified in code):**

1. `bootstrap_fresh_install` (`crates/mika-common/src/home.rs:56`) → `bootstrap_agent` →
   `bootstrap` writes the `DEFAULT_IDENTITY` const (`crates/mika-common/src/home.rs:274`),
   which is `name = "Mika"\nemoji = "✦"` plus a commented `[kg]` example — **no `[skills]`**.
2. The `Identity` struct (`crates/mika-agent/src/prompt.rs:239`) parses `[skills].allowlist`
   into `Option<Vec<String>>` (`prompt.rs:125`). A missing block → `None`.
3. At agent load (`crates/mika-agent/src/server/mod.rs:414`) `apply_identity_allowlist` is
   called **only when the allowlist is `Some`**. With `None`, no eviction runs.
4. `apply_identity_allowlist` (`crates/mika-agent/src/skills/mod.rs:598`) is additionally a
   **no-op when the allowlist is empty** (`skills/mod.rs:599`). So `None`/empty both mean
   *every bundled skill stays loaded* — including the architect/dev/qa skills whose prompts
   carry the `Disposition: READY|ITERATE|ESCALATE` contract.

**Observed behavior (hard evidence, issue body):** on `qwen/qwen3.7-max`,
`mika ask "How are you?"` → `"Good, thanks. You? Disposition: READY"`; `mika ask "ping"`
→ `"pong Disposition: READY"`. After adding a narrow allowlist + restart, the trailer is
gone across all test queries and personal-assistant tasks still work.

**Why it's a real bug, not operator misconfiguration:** the family-rollout provisioning path
(`add-customer.sh` without `--identity-dir`) deterministically produces this default
identity. The contract leak (`Disposition:` line) is an internal architect contract bleeding
into a user-facing surface — a contract-leak class per `feedback_prompt_enforcement_fragile`.

**Scope correction:** the issue body pointed at `crates/mika-agent/src/agents.rs::bootstrap_fresh_install`.
That path is wrong — `bootstrap_fresh_install` and `DEFAULT_IDENTITY` both live in
`crates/mika-common/src/home.rs`. This plan targets the correct location.

---

## Requirements

Traceable to the issue's acceptance criteria:

- **R1 (AC1):** A fresh agent created via `bootstrap_fresh_install` (no `--identity-dir`)
  does not emit `Disposition: READY` — or any internal verdict keyword — appended to
  user-facing replies. Mechanism: the default identity's allowlist excludes every skill
  whose prompt carries a verdict/disposition contract.
- **R2 (AC2):** The fix preserves access to operator-essential skills — calendar,
  google-workspace, browser-control, desktop, file-reader, web-search, mcp — so the agent
  still performs personal-assistant work.
- **R3 (AC3):** The four well-known dev agents (`mika-dev`, `mika-qa`, `mika-arch`,
  `mika-relay`) retain their existing explicit allowlists unchanged. Satisfied by
  construction — they use separate identity templates in `well_known_agents.rs`, not
  `DEFAULT_IDENTITY`.
- **R4 (AC4):** An automated test asserts the identity.toml written by
  `bootstrap_fresh_install` has a `[skills].allowlist` that **excludes** `dev-pilot`,
  `dev-groom`, `mika-arch-*`, `qa-review`, `self-dev*` and **includes** the operator-essential
  skills.
- **R5 (AC5):** Docs document the default-allowlist behavior and the contract-leak class it
  prevents.

**Verified narrow allowlist (Vincent tested on his live agent — trailer gone, assistant
tasks intact):**

```
calendar, google-workspace, browser-control, desktop, file-reader, mcp,
web-search, self-knowledge, shell-exec, tmux, git-ops, gh-read-only
```

Of these, only `gh-read-only` is a *bundled* skill (confirmed in `skills/bundled/`); the rest
are community/builtin skills installed per agent. This is fine: the allowlist is a filter over
the full per-agent registry, and `apply_identity_allowlist` only *warns* (does not fail) on
allowlisted names absent from the registry (`skills/mod.rs:621`). On a freshly bootstrapped
agent that has not yet installed the community skills, those benign WARNs may appear; the
security-relevant effect — evicting the engineering skills — still applies.

---

## Key Technical Decisions

- **KTD1 — Shape A (narrow default allowlist), not B or C.** Add the allowlist to
  `DEFAULT_IDENTITY`. Shape B (refuse to start without an explicit `[skills]` block) adds
  provisioning friction and a new failure mode on the family path; Shape C (output redactor
  stripping `Disposition:` lines) treats the symptom and is brittle against contract drift.
  A is the smallest backward-compatible fix and is the operator/architect-endorsed shape.

- **KTD2 — Allowlist as a named constant in `home.rs`, beside `DEFAULT_IDENTITY`.** Define
  the skill-name list as a `const` (e.g. `DEFAULT_AGENT_SKILL_ALLOWLIST: &[&str]`) and render
  it into the `[skills]` block, rather than hardcoding a TOML string literal. This gives the
  AC4 test a single source of truth to assert against and makes the maintenance coupling
  explicit. **Maintenance coupling to flag in code comments:** this list is the personal/
  customer-agent counterpart to the well-known-agent allowlists in
  `crates/mika-agent/src/well_known_agents.rs`; new user-facing skills that a personal agent
  should reach must be added here too. The allowlist↔required_tools coherence guard added in
  mika#1595 operates at agent-load on the *resolved* surface, so an allowlist naming a skill
  with unmet tool requirements would surface there — keep the default list to skills with
  self-contained or operator-granted tools.

- **KTD3 — AC4 test lives in `mika-common` (`home.rs` tests).** `DEFAULT_IDENTITY` lives in
  `mika-common`, which has no dependency on `mika-agent`'s `Identity` struct. The test reads
  the written `agents/mika/identity.toml`, parses `[skills].allowlist` with the `toml` crate
  (already a `mika-common` dependency via config), and asserts membership. This keeps the test
  in the crate that owns the constant and avoids a backward (mika-agent → test) coupling.

- **KTD4 — Preserve the commented `[kg]` example.** The existing `DEFAULT_IDENTITY` documents
  the optional `[kg]` block as comments; keep that, and add the `[skills].allowlist` as an
  active block above or below it.

---

## Implementation Units

### U1. Add a narrow `[skills].allowlist` to the default identity template

**Goal:** `bootstrap_fresh_install` writes an `identity.toml` carrying the verified narrow
operator-assistant allowlist, closing the default-permissive gap.

**Requirements:** R1, R2, R5 (partial).

**Dependencies:** none.

**Files:**
- `crates/mika-common/src/home.rs` — modify `DEFAULT_IDENTITY`; add
  `DEFAULT_AGENT_SKILL_ALLOWLIST` constant + a small helper or build-time render of the
  `[skills]` block into the identity string.

**Approach:**
- Introduce `pub const DEFAULT_AGENT_SKILL_ALLOWLIST: &[&str]` listing the 12 verified names.
- Render the `[skills]` block into `DEFAULT_IDENTITY`. Because `DEFAULT_IDENTITY` is currently
  a `&str` const consumed by `write_default_if_missing`, prefer the simplest correct mechanism:
  either (a) keep `DEFAULT_IDENTITY` a literal with the allowlist inlined as a TOML array **and**
  keep `DEFAULT_AGENT_SKILL_ALLOWLIST` as the test's source of truth (assert the literal
  contains each name), or (b) build the identity string once from the const. (a) is the lower-risk
  change; pick it unless building from the const is clearly cleaner. Add a code comment naming
  the maintenance coupling to `well_known_agents.rs` and the mika#1595 coherence guard (KTD2).
- TOML shape:
  ```toml
  name = "Mika"
  emoji = "✦"

  [skills]
  # Narrow operator-assistant allowlist. A missing/empty allowlist is default-permissive
  # (loads every bundled engineering skill) and leaks internal "Disposition:" contracts to
  # user-facing replies — see mika#1596. Provisioning an explicit identity.toml overrides this.
  # Maintenance: personal-agent counterpart to the well-known-agent allowlists in
  # crates/mika-agent/src/well_known_agents.rs.
  allowlist = ["calendar", "google-workspace", "browser-control", "desktop",
               "file-reader", "mcp", "web-search", "self-knowledge", "shell-exec",
               "tmux", "git-ops", "gh-read-only"]

  # [kg]
  # enabled = true
  # docs_root = "/path/to/docs"
  ```

**Patterns to follow:** existing `DEFAULT_*` consts in `home.rs`; the `[skills].allowlist`
TOML shape used by `well_known_agents.rs` identity templates.

**Test scenarios:** covered by U2 (the const/template is data, exercised through
`bootstrap_fresh_install`).

**Verification:** `bootstrap_fresh_install` into a temp home writes
`agents/mika/identity.toml` containing an active `[skills].allowlist`; `cargo build -p mika-common`
succeeds.

---

### U2. Test: default identity excludes engineering skills, includes operator-essential ones

**Goal:** Lock R4 (AC4) — and by proxy R1/R2 at the data layer — with a regression test.

**Requirements:** R4 (AC4); guards R1, R2.

**Dependencies:** U1.

**Files:**
- `crates/mika-common/src/home.rs` — add a `#[test]` in the existing `mod tests`.

**Approach:**
- Bootstrap into a `tempfile::tempdir()` home (mirror `test_bootstrap_fresh_install`).
- Read `agents/mika/identity.toml`, parse it with `toml` into a value, extract
  `skills.allowlist` as a `Vec<String>`.
- Assert it is present and non-empty (proves the no-op-on-empty path can't trigger).
- Assert **exclusions**: none of `dev-pilot`, `dev-groom`, `mika-arch-groom-ticket`,
  `mika-arch-groom-milestone`, `mika-arch-second-review`, `qa-review`,
  `qa-review-build-callback` appear; and no entry starts with `self-dev` or `mika-arch`.
- Assert **inclusions** (R2): `calendar`, `google-workspace`, `browser-control`, `desktop`,
  `file-reader`, `web-search`, `mcp` are all present.
- If U1 keeps `DEFAULT_AGENT_SKILL_ALLOWLIST` as a const, also assert the parsed list equals
  the const (single source of truth).

**Patterns to follow:** `test_bootstrap_fresh_install` (`home.rs:599`) for the tempdir +
read-back shape; `serial_test::serial` is only needed for env-var tests, not here.

**Test scenarios:**
- Covers AC4. Happy path: parsed allowlist present, non-empty, excludes all listed
  engineering skills (exact names + `self-dev*`/`mika-arch*` prefix check), includes all
  operator-essential names.
- Edge: assert the block is *active* TOML, not a comment (parse must yield a value, not `None`).
- Regression guard: existing `test_bootstrap_fresh_install` and `test_bootstrap_creates_structure`
  still pass (the added block must not break files-exist assertions).

**Verification:** `cargo test -p mika-common home` passes, including the new test and all
pre-existing `home.rs` tests.

---

### U3. Document the default-allowlist behavior and the contract-leak class

**Goal:** Satisfy R5 (AC5) — make the default-allowlist boundary discoverable so a future
edit doesn't silently reopen the leak.

**Requirements:** R5 (AC5).

**Dependencies:** U1.

**Files:**
- `crates/mika-agent/CLAUDE.md` — in the Skills System section, add a short note: a personal/
  customer agent's default identity now ships a narrow `[skills].allowlist`; a missing/empty
  allowlist is default-permissive and leaks internal `Disposition:`/`Verdict:` contracts to
  user-facing replies (mika#1596). Point to `crates/mika-common/src/home.rs`
  `DEFAULT_AGENT_SKILL_ALLOWLIST` and note the maintenance coupling to `well_known_agents.rs`.
- `crates/mika-common/CLAUDE.md` — one line under "Home directory" noting `DEFAULT_IDENTITY`
  carries the narrow default allowlist and why.

**Approach:** Documentation only; match the surrounding terse, reference-style prose. No
`docs/` source-of-truth files change, so the `docs-sync` CI job is unaffected (these are
crate-level `CLAUDE.md` files, not `docs/`).

**Test scenarios:** Test expectation: none — documentation only.

**Verification:** Both `CLAUDE.md` files mention the default allowlist + contract-leak class
and cross-reference the constant location.

---

## Scope Boundaries

**In scope:** the default identity template, one regression test, two `CLAUDE.md` notes.

### Deferred to Follow-Up Work
- **Output-side redaction (Shape C) as defense-in-depth.** A redactor stripping
  `^Disposition:`/`^Verdict:` lines from outbound messages would catch leaks from *any*
  future misconfiguration, not just the default path. Out of scope here (Shape A fixes the
  reported cause); file separately if the contract-leak class recurs from another path.
- **Validation/lint that flags an empty-or-missing `[skills]` block on personal agents**
  (Shape B leanings, as a warning rather than a hard refusal). Not needed for the family
  rollout; revisit if more provisioning paths emerge.

**Out of scope:** well-known dev-agent allowlists (R3 — unchanged by construction); the
`apply_identity_allowlist` eviction logic itself (correct as-is); `add-customer.sh` /
mika-cloud provisioning (the fix lands entirely in the default template the script already
triggers).

---

## System-Wide Impact

- **Family rollout (mika-cloud#111):** every customer provisioned via `add-customer.sh`
  without `--identity-dir` inherits the narrow allowlist — the leak is closed at the source
  the script already calls. No mika-cloud change required.
- **Existing already-provisioned agents:** `write_default_if_missing` does **not** overwrite
  an existing `identity.toml`. Agents already bootstrapped before this change keep their old
  (block-less) identity and remain leaky until their identity is re-provisioned or hand-edited
  (Vincent's manual fix on his personal agent is exactly this). This is acceptable for the
  Monday target (family customers are provisioned fresh) but worth stating in the PR body.
- **CI:** no `docs/` change → `docs-sync` unaffected; no new bundled skill → `verify-bundled-skills`
  unaffected; allowlist↔required_tools coherence (mika#1595) operates at agent-load, and the
  default list points to operator skills with self-contained/granted tools.

---

## Acceptance criteria

- [ ] **AC1** — fresh `bootstrap_fresh_install` agent does not load verdict-carrying engineering skills (the allowlist evicts them at agent load).
- [ ] **AC2** — operator-essential skills (calendar, google-workspace, browser-control, desktop, file-reader, web-search, mcp) preserved.
- [ ] **AC3** — well-known dev agents (`mika-dev/qa/arch/relay`) unchanged — they use separate identities in `well_known_agents.rs` (verified by their existing allowlist-count tests still passing).
- [ ] **AC4** — integration test asserts the written allowlist excludes `dev-pilot`/`dev-groom`/`mika-arch-*`/`qa-review`/`self-dev*`.
- [ ] **AC5** — docs updated.

## Definition of Done

- `DEFAULT_IDENTITY` carries the active narrow `[skills].allowlist` (U1).
- New `home.rs` test asserts exclusion of engineering skills + inclusion of operator-essential
  skills, and all pre-existing `home.rs` tests pass (U2).
- `crates/mika-agent/CLAUDE.md` and `crates/mika-common/CLAUDE.md` document the behavior (U3).
- `cargo build`, `cargo test -p mika-common`, `cargo clippy`, `cargo fmt --check` all clean.
- PR body notes the already-provisioned-agents caveat and `Closes #1596`.

## Verification Contract

- `cargo test -p mika-common home` — new + existing home tests green.
- `cargo build` / `cargo clippy --all-targets` — clean.
- Manual trace (no live agent needed): the written `identity.toml` parses to a non-empty
  allowlist excluding `dev-*`, `qa-review*`, `mika-arch-*`, `self-dev*` — which is exactly
  what makes `apply_identity_allowlist` evict the verdict-carrying skills at agent load (R1).

## Sources & Research

- Issue: senara-solutions/mika#1596 (body, ACs, Vincent's reproduction + verified allowlist).
- Code grounding: `crates/mika-common/src/home.rs` (`bootstrap_fresh_install:56`,
  `DEFAULT_IDENTITY:274`), `crates/mika-agent/src/prompt.rs` (`Identity:239`, `allowlist:125`),
  `crates/mika-agent/src/server/mod.rs:414`, `crates/mika-agent/src/skills/mod.rs:598`
  (`apply_identity_allowlist`, no-op-on-empty at :599, unknown-skill WARN at :621).
- Related: mika#1595 (allowlist↔required_tools coherence guard, b49f27b1); mika-cloud#111
  (family rollout / `add-customer.sh`); `well_known_agents.rs` (well-known-agent allowlists).

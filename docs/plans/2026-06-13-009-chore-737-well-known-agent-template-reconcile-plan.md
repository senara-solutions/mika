---
ticket: mika#737
branch: chore/737/well-known-agent-template-reconcile
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/737
execution: code
---

# Plan: well-known agent template reconciliation (mika#737)

## Problem frame

`MIKA_DEV_SOUL` / `MIKA_QA_SOUL` const templates in `crates/mika-agent/src/well_known_agents.rs:243-281` are skeleton placeholders from the 2026-03 bootstrap. On-disk versions at `~/.mika/agents/{mika-dev,mika-qa}/soul.md` have iterated significantly (10× / 4× the size respectively).

Not currently broken — `provision_well_known_agents()` has an `agent_exists()` idempotency skip, so existing agents are never overwritten. This only matters on fresh-host bootstrap.

## Directional decisions (per first-pass architect)

- **Direction**: on-disk → code (regenerate consts from on-disk source of truth).
- **Q2 — Variant regeneration**: **defer to follow-up.** Variant regen is a runtime operation requiring a live model provider; mixing it with code reconciliation violates Single Responsibility and makes the PR non-deterministic. The plan acknowledges this in scope; a follow-up ticket can be filed if needed.
- **Q3 — Raw-string strategy**: **`r##` everywhere uniformly.** Safer (future on-disk edits adding backticks/`"#` sequences won't break compilation), consistent, two extra `#` characters cost zero runtime.
- **Emoji decision**: keep the semantic template emojis (`🛠`/`🔍`) in the const. On-disk is protected by the idempotency skip; the template is what ships on fresh hosts and the semantic emojis are more descriptive there.
- **AC5 fresh-host test**: manual verification documented in the PR description (proportionate to p3 mechanical work; no need for an automated test harness).

## Scope boundaries

- Replace `MIKA_DEV_SOUL` and `MIKA_QA_SOUL` const contents with on-disk soul.md byte content (with `r##"..."##` delimiters uniformly).
- Update `MIKA_DEV` and `MIKA_QA` `display_name` to `"Mika Dev"` and `"Mika QA"` (currently bare `"Dev"`/`"QA"`).
- Keep template emojis `🛠`/`🔍`.
- Exclude `[reflection]` section from template-written identity.toml.
- Compound doc capturing the reconciliation pattern.
- **Out of scope:** per-provider variant regeneration (runtime operation, follow-up); on-disk modifications to running mika-dev/mika-qa instances (idempotency skip preserves them); template content authorship (this PR is mechanical copy, not editorial).

## Implementation Units

### U1 — Copy on-disk soul.md content into template consts

**Goal:** `MIKA_DEV_SOUL` matches `~/.mika/agents/mika-dev/soul.md` byte-for-byte; same for `MIKA_QA_SOUL`.

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (the two soul consts around line 243-281)

**Approach:**

1. Read current on-disk files via the implementation environment: `cat ~/.mika/agents/mika-dev/soul.md` and `cat ~/.mika/agents/mika-qa/soul.md`.
2. Wrap each in `r##"..."##` delimiters uniformly.
3. Replace the const declarations.

If a soul.md contains a triple-`#` sequence (rare but possible if it documents `r##` syntax itself), escalate to `r###"..."###`. The implementer verifies via grep after copy.

**Constraint:** the souls must reflect the CURRENT working agent prompts at deploy time, not the operator's idiosyncratic edits. Before this PR is merged, the implementer should verify that the on-disk content represents the canonical / agreed-upon prompt — not e.g. an in-progress operator experiment. The PR description should note when each soul was last edited (file mtime) so reviewers can confirm the snapshot is intentional.

**Test scenarios:**
- **Compile-time:** `cargo build -p mika-agent` clean.
- **Byte-for-byte match:** unit test asserts `MIKA_DEV_SOUL == include_str!("../../testdata/expected_mika_dev_soul.md")` (or equivalent — the test harness checks the const equals the captured snapshot at commit time).

**Verification:** unit tests + `cargo clippy -p mika-agent --no-deps` clean.

### U2 — Update display_name fields

**Goal:** `MIKA_DEV.display_name = "Mika Dev"`, `MIKA_QA.display_name = "Mika QA"`.

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (the `MIKA_DEV` and `MIKA_QA` identity consts)

**Approach:** Change the `display_name` field assignments. Verify no other call site depends on the old bare names (`"Dev"`/`"QA"`) — grep for string literals.

**Test scenarios:**
- Existing unit tests pass.
- New unit test asserts the display_names.

**Verification:** grep + tests + clippy.

### U3 — Exclude `[reflection]` from template

**Goal:** Template-written identity.toml does NOT include a `[reflection]` section. (User-specific timezone data MUST NOT bootstrap from template.)

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (the `MIKA_DEV` identity template builder, wherever the TOML body is constructed)

**Approach:** Verify the current template generator does NOT include `[reflection]`. If it does, remove it. The on-disk mika-dev identity.toml's `[reflection]` block is a runtime customization — operators can add it via direct edit; provisioning shouldn't seed it.

**Verification:** snapshot test or print-and-compare on the generated identity.toml string — assert `[reflection]` is absent.

### U4 — Compound doc + PR description

**Goal:** Capture the pattern for future template reconciliations.

**Files:**
- Create or modify: `docs/solutions/best-practices/well-known-agent-template-reconciliation-2026-06-13.md`

**Approach:** Short one-paragraph entry covering:
- The drift problem (consts vs on-disk)
- Direction (on-disk → code, not reverse)
- User-specific section exclusion rule (`[reflection]`, timezones, secrets)
- Raw-string uniformity (`r##` everywhere)
- Variant regeneration as a runtime follow-up

**Verification:** manual read + KG ingestion picks it up on next startup.

## Dependencies / sequencing

- U1 → U2 → U3 can ship in any order within the same PR
- U4 (compound doc) ships in same PR; can be authored after U1-U3 stabilize

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/well_known_agents.rs` — existing const + builder pattern
- `docs/solutions/best-practices/` — compound doc style
- `feedback_no_provider_prompts.md` — provider/model tuple variant convention (for the deferred variant follow-up)

## Verification (top-level)

- `cargo test -p mika-agent` passes (existing + 2 new tests for U1/U2/U3)
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Manual smoke (documented in PR description): on a fresh `~/.mika/` directory with `MIKA_DEV_MODE=true`, start mika-spirit. Verify `~/.mika/agents/mika-dev/soul.md` matches the const (byte-for-byte) and `identity.toml` carries `name = "Mika Dev"` with `[reflection]` absent.

## Risk / known unknowns

- **Operator timezone leak.** Risk addressed in U3 — `[reflection]` excluded. Additional check: grep the new soul.md content for `Europe/Paris`, `Vincent`, or other operator-specific markers. The soul.md content from on-disk should be reviewed for any operator-specific patterns before sealing the snapshot.
- **Snapshot staleness.** This PR captures a point-in-time snapshot. Future on-disk edits create new drift. The compound doc names a follow-up trigger: "if template drift exceeds 25% by line count, run the reconciliation again." Not enforced; operator discretion.
- **`r##` insufficient for triple-`#`-quoting content.** If a soul.md adds `r##` syntax examples (meta-doc about Rust raw strings), the const needs `r###`. Mitigation: grep for `##"` in on-disk before snapshot.

## Out-of-scope (explicit)

- Per-provider variant regeneration (runtime operation; follow-up ticket if needed).
- Reverse direction (push template values to on-disk) — explicitly rejected by issue body.
- Editorial changes to soul content (snapshot is mechanical copy; if the prompt has issues, that's a separate ticket).
- Adding new well-known agents (mika-arch reconciliation, future agents) — sibling concern.
- Automated drift-detection CI gate — overkill for a p3.

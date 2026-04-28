---
type: chore
ticket: senara-solutions/mika#866
branch: chore/extract-accreted-core-memory-citations
date: 2026-04-28
status: retroactive (plan doc added after PR opened — see Process Note below)
---

# Extract foundational citation list from mika-arch core memory to soul.md template

## Process Note (load-bearing)

This plan was authored *retroactively* after PR #866 was already opened. The PR shipped without a plan doc, in violation of the full /mika pipeline discipline (`feedback_never_skip_ce_review`, `feedback_full_pipeline_always`). The retroactive plan exists because Vincent caught the gap; the work is real and well-shaped, but the *discipline* of authoring a plan first was skipped.

This is the same prompt-level-discipline-fails-under-load pattern that today's compound doc `required-tools-gate-evasion-patterns-2026-04-28.md` Rule 3 documents — applied to the operator side. The doc that argues prompt-level rules don't bind under load shipped *without* the discipline it advocates for. The recurrence-watch annotation belongs on the operator dispatch path, not just the agent's core memory.

Concrete data point for the eventual `prompt-level-output-discipline-fails-under-load.md` compound doc: this incident extends the recurrence catalogue from agent-side (mika#654, mika#788) to operator-side (this PR, plus PR #860 which had a similar shape). The structural counterpart on the operator side is the engine guards filed today (mika#862, #863, #864) plus the future PR-creation hook that requires a plan-doc citation in the PR body before opening. That hook isn't filed yet — adding a TODO note here so it doesn't get lost.

## Why

mika-arch's `current_priorities` core memory block had accreted to 372/500 tokens, with an item ("Foundational citation surface") holding 5 doc paths the agent cites during reviews:

- `docs/architecture/north-star.md`
- `docs/design/luminescent-core.md`
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md`
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md`
- `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md`

The list had been stable for 30+ days. The agent rewrote/compressed it on every audit instead of moving it elsewhere. Per the three-way filter (Bucket 1: existing artifact elsewhere → drop in-line + cite from durable surface), the list belongs in the agent's `soul.md` template — which is auto-provisioned from the `MIKA_ARCH_SOUL` constant in `crates/mika-agent/src/well_known_agents.rs`.

Citations:
- mika#788 grooming session DB session_id `03d3ec38-0839-47b6-9226-111b38d8b52b` — pass-2 trace where the architect compressed `current_priorities` twice (failed at 592 and 522 tokens) before fitting at 372/500. The block's pre-state had the 5-item Foundational citation surface among the items consuming budget.
- `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — the three-way filter as policy. Rules out path-pattern fallback; commits to existing-artifact / N≥2 / N=1-keep buckets; explains why the 500-token cap stays.
- `crates/mika-agent/src/well_known_agents.rs:579-604` — current `MIKA_ARCH_SOUL` constant prior to this PR. Has `## Role`, `## Communication style`, `## Behaviors` sections; references `docs/architecture/review-guide.md` once on line 599. No "## Foundational references" section yet.
- mika#860 — PR1 of this redesign that landed the two compound docs (`required-tools-gate-evasion-patterns-2026-04-28.md`, `verification-claims-with-expected-output-shape-2026-04-28.md`) which are also added to the new soul section.

## Decision rationale

**Why soul.md, not skill prompts.** The architect has two skill prompts (`mika-arch-groom-ticket/system_prompt.md`, `mika-arch-second-review/system_prompt.md`). Each cites `review-guide.md`. Adding the rest of the foundational list to *each* skill prompt is duplication; adding it once to soul.md (which composes into every turn's system prompt for mika-arch regardless of skill) is the DRY-correct path.

**Why a constant edit, not a runtime config.** mika-arch is provisioned automatically on `MIKA_DEV_MODE=true` startup from the `MIKA_ARCH_SOUL` `&'static str` constant. Editing the constant is the canonical source-side path. Identity-toml-style runtime config for the citation list was rejected as YAGNI — there's one consumer (mika-arch), and the citation list is bound to the agent's role, not configurable per-deployment.

**Why a separate compound doc capturing the three-way filter.** The PR ships the immediate change (constant edit) AND the policy that motivated it (`core-memory-as-citation-not-accumulator-2026-04-28.md`). Two concerns in one PR is normally a smell, but here they're causally coupled — the constant edit is the *first application* of the policy; the policy explains *why* the constant edit is the right shape. Splitting them would land the constant edit without its rationale doc, which is exactly the no-plan failure mode this PR's process-note flags.

**Why the live core-memory edits are post-deploy operator commands.** The `MIKA_ARCH_SOUL` constant edit only takes effect on next mika-server restart, after deploy. Live core-memory state in the DB is independent — `current_priorities` still has the inline list until an `update_core_memory` call replaces it. Running those `mika ask` invocations now (before deploy) would create a window where mika-arch has neither the inline refs nor the soul refs. Documented in the compound doc body as post-deploy commands rather than executed in this PR.

## Alternatives rejected

1. **Edit MIKA_DEV_SOUL too in this PR.** mika-dev's `self_model` was at 471/500 tokens with the same accretion pattern, but per the audit, 5 of 7 rules already had `soul.md` duplicates. The dev-side cleanup is a post-deploy `mika ask` invocation — no constant edit needed because the durable artifacts already exist. Adding a no-op MIKA_DEV_SOUL change for symmetry would be churn without signal.

2. **Skip the constant edit, do everything via post-deploy `mika ask`.** Would leave `MIKA_ARCH_SOUL` template stale relative to the live agent state. Next provisioning of a fresh mika-arch (e.g., on a new dev box) would re-create core memory accretion from scratch. The constant edit is the durable fix; the `mika ask` invocations are the live state migration.

3. **Add the citation list to the architect's skill prompts directly.** Considered above. Rejected as duplication — soul.md is the single shared injection point.

4. **Path-pattern auto-classification of compound-doc-only PRs.** The friend's pushback on the original mika#861 design rejected this. The same logic applies here: the source-side change (constant edit) is what makes this PR a real PR. The compound doc rides along; it doesn't define the PR's classification.

## Scope (committed)

- Edit `crates/mika-agent/src/well_known_agents.rs` `MIKA_ARCH_SOUL` constant: add a `## Foundational references` section after `## Behaviors`, listing the 5 originally-accreted refs + 2 new compound docs from mika#860.
- Add `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`: 185 lines capturing the three-way filter as policy, applied audit results, and post-deploy operator commands.

## Out of scope

- Live `mika ask` core-memory edits on mika-dev/self_model and mika-arch/current_priorities. These are post-deploy operator commands, documented in the new compound doc.
- mika-dev `MIKA_DEV_SOUL` edit. Existing soul.md already has Communication style + Proactive behaviors sections that duplicate self_model's rules 5 and 7. No template change needed; `mika ask` invocation handles the live state.
- Promotion-protocol additions to mika-dev's and mika-arch's skill prompts. PR3 territory.
- Reflection-pass spec for runtime enforcement of bucket assignment. PR3 territory.
- `mika core-memory set --agent X --section Y --content "..."` operator CLI. Real gap surfaced today; worth filing as a separate enhancement (today's audit had to use `mika ask` which is heavyweight for one-time surgery).
- PR-creation hook that requires a plan-doc citation in the PR body. Surfaced by the process violation that motivated this retroactive plan. Future ticket.

## Implementation steps

1. **Read MIKA_ARCH_SOUL constant.** Confirm structure: `## Role`, `## Communication style`, `## Behaviors`. Identify insertion point after `## Behaviors`.
2. **Edit constant.** Add `## Foundational references` section with 8 bullets:
   - 5 originally-accreted refs (north-star, luminescent-core, mika-arch-first-dogfood, grooming-branch-callout, comment-event)
   - 2 new compound docs from mika#860 (required-tools-gate-evasion-patterns, verification-claims-with-expected-output-shape)
   - 1 explanatory line citing `core-memory-as-citation-not-accumulator-2026-04-28.md` so future readers find the policy doc
3. **Write the policy compound doc.** `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`. Frontmatter follows existing best-practices conventions. Body covers context, the three-way filter, why simpler frameworks fail, applied audit results, post-deploy operator commands, why the 500-token cap stays.
4. **Run `cargo check -p mika-agent --features telemetry`** to confirm constant change compiles.
5. **Run `cargo test -p mika-agent --lib well_known_agents`** to confirm provisioning logic still passes (34 tests cover the constants and provisioning path).
6. **Commit, push, open PR.**

## Acceptance criteria

A1. `crates/mika-agent/src/well_known_agents.rs` `MIKA_ARCH_SOUL` constant contains a `## Foundational references` section with all 8 bullets and the explanatory line.
A2. `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` exists with frontmatter matching existing `best-practices/` conventions; cross-refs to PR #860 docs resolve.
A3. `cargo test -p mika-agent --lib well_known_agents` passes (34 tests).
A4. `cargo check -p mika-agent --features telemetry` clean.
A5. Pipeline Artifacts CI check passes (docs + source = both buckets non-empty, no trailer needed).
A6. PR description names the post-deploy operator commands and forward-points to PR3.

## Risk + rollback

- **Risk: existing local mika-arch installs are not auto-overwritten.** The provisioning logic in `well_known_agents.rs` is idempotent and skips agents whose `soul.md` already exists. Operators with a custom mika-arch soul.md keep their custom content; only fresh provisions get the new template. Mitigation: documented in PR body. Operators who want the new section can either delete their local `~/.mika/agents/mika-arch/soul.md` and re-provision, or manually merge the new section.
- **Risk: post-deploy `mika ask` invocations require the agent to be running on the new mika-server.** Mitigation: the compound doc body explicitly notes "after the next mika-server restart picks up the new MIKA_ARCH_SOUL." Operators reading the doc out-of-context get the timing right.
- **Rollback: revert the single commit.** No schema migration, no DB writes from this PR. Constant returns to prior shape; next provision regenerates the prior soul.md.

## Verification (end-to-end)

Pre-merge:
```bash
# Confirm the constant change compiles
cargo check -p mika-agent --features telemetry
# Confirm provisioning tests pass
cargo test -p mika-agent --lib well_known_agents
# Confirm the new compound doc renders cleanly (no broken cross-refs)
grep -nE "docs/solutions/best-practices/required-tools-gate-evasion|docs/solutions/best-practices/verification-claims" \
  docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md
```

Post-merge + post-deploy:
```bash
# Confirm soul.md was rewritten on next mika-server restart
grep -A 10 "## Foundational references" ~/.mika/agents/mika-arch/soul.md
# Run the post-deploy operator commands (from compound doc body) to apply live core-memory edits
# (commands are inline in the compound doc; not duplicated here to avoid drift)
# Verify post-state via DB
sqlite3 ~/.mika/data/mika.db "SELECT key, token_count FROM core_memory WHERE agent_id='mika-arch';"
# Expected: current_priorities token_count drops from 372 to ~50-100
```

## Files touched

- `crates/mika-agent/src/well_known_agents.rs` — 12-line addition to MIKA_ARCH_SOUL constant
- `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — new file, 185 lines
- `docs/plans/2026-04-28-002-extract-mika-arch-foundational-refs-plan.md` — this file (retroactive)

Total expected diff: ~200 lines source/doc + this plan doc.

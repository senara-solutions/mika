---
title: "Evaluate Mika defaults for Anthropic provider and Sonnet model"
type: feat
status: active
date: 2026-04-27
issue: senara-solutions/mika#827
---

# Evaluate Mika defaults for Anthropic provider and Sonnet model

## Overview

mika-dev and mika-qa now run on Claude Sonnet (~200K-token context) via the Anthropic provider, but several Mika defaults were chosen for an earlier era of smaller-context, lower-cost models. The 2026-04-26 deploy made the gap visible: three bundled skills were silently dropped at startup with `oversized prompt` errors, leaving mika-dev without `self-dev` and mika-qa without `qa-review` — silently breaking the autonomous loop.

This plan ships the work in three phases:

1. **Unblock** the three failing bundled skills with concrete byte-target changes (no engine changes required).
2. **Audit** the remaining context-coupled dials (engine ceilings, completion-token caps, retrieval windows, truncation thresholds) and write a durable audit doc that survives future model swaps.
3. **Apply** the audit's recommendations as a separate set of focused changes.

A startup regression backstop (Unit 2) is added so the same silent-drop failure mode cannot recur — the next prompt edit that nudges a skill past its declared ceiling fails CI loudly instead of failing prod silently.

## Problem Frame

Three concrete failures observed in `server.log` on the 2026-04-26 deploy:

```
qa-review:                 35008B (limit 32768B)
qa-review-build-callback:  18595B (limit 16384B)
self-dev:                  49195B (limit 49152B)
```

The skill loader at `crates/mika-agent/src/skills/index.rs:499-516` calls `load_snippet_with_limit()` and pushes oversized prompts onto `ScanResult.skipped`. The skill is **not** loaded. There is a startup WARN line per skipped skill, but mika-server keeps booting — the autonomous loop comes up missing core skills and silently misbehaves until the next deploy.

The immediate fix is small (three TOML edits). The durable question is broader: what other config defaults assume a smaller, cheaper model than Sonnet, and how do we stop discovering them piecemeal at deploy time?

## Requirements Trace

- **R1.** Re-deploy on the head of this branch produces zero `oversized prompt` warnings for bundled skills (`self-dev`, `qa-review`, `qa-review-build-callback` all load).
- **R2.** mika-dev's `self-dev` skill and mika-qa's `qa-review` skill are present in `SkillRegistry.entries` after startup (verified by a regression test, not just by re-deploying).
- **R3.** A durable audit doc lives at `docs/architecture/anthropic-sonnet-defaults-audit-2026-04-27.md` enumerating each context-coupled dial with `(current value, where it lives, rationale at the time, Sonnet changes the answer Y/N, recommendation)`. This doc is the artifact future model swaps consult before flipping the provider.
- **R4.** Recommendations from the audit that the audit itself classifies as "ship now" land in this same branch (audit-driven changes shipped together with the audit, not deferred).
- **R5.** The pattern of *silently dropped bundled skills* is replaced with *fail-loud at startup* (or fail-loud in CI) so the next prompt-size regression is caught before deploy.

## Scope Boundaries

- **In scope:** bundled skills under `skills/bundled/`, the `MAX_PROMPT_SIZE_CEILING` constant, the default `llm_max_tokens` in mika-common and the TUI defaults file, and any other dial the audit surfaces as Sonnet-relevant.
- **Out of scope:** community skills in `mika-skills/` (separate repo, separate concern). If the audit reveals a marketplace skill that depends on a default we change, document it in the audit, do not auto-edit.
- **Out of scope:** per-skill provider/model variant prompts (a heavier pattern; warranted only when the prompt itself must differ across providers, not when "fits in budget" is the only difference).
- **Out of scope:** any migration that changes runtime cost meaningfully on non-Sonnet providers — bumps must be cheap on Sonnet and not worse on Kimi/Qwen/DeepSeek.

### Deferred to Separate Tasks

- **Per-model variant skill prompts for non-Sonnet providers** — if the audit decides a dial should differ across providers and we don't already have a variant pattern for that surface, file a follow-up. Don't expand provider-aware config in this branch beyond what already exists.
- **Marketplace skill audit** — community skills shipped from `mika-skills/` are surveyed separately; if any downstream change is needed there, file a `mika-skills` ticket from the audit.
- **P1 escalation carve-out (severity-based, NOT scope-deferred):** if Unit 3's audit surfaces a marketplace skill in `mika-skills/` whose `actual_size > declared_cap` *right now* (i.e., same failure class as the three triggering #827, just on the marketplace path that Unit 2's CI gate doesn't cover), this is P1 — same severity as the fire #827 is fighting. Behavior: file a `mika-skills` issue with `p1-important` immediately, surface the finding to the operator before #827 merges, and link from the audit doc. Distinct from the `bump (follow-up ticket)` queue, which is for non-failing dials. Severity is determined by *active impact*, not by which repo the skill lives in.

## Context & Research

### Relevant Code and Patterns

- **Skill loader gate** — `crates/mika-agent/src/skills/index.rs:482-516`. `max_size = manifest.skill.max_prompt_size.map(|v| v.min(MAX_PROMPT_SIZE_CEILING)).unwrap_or(MAX_PROMPT_SNIPPET_SIZE)`. Oversize → push to `skipped`, log error, continue. `MAX_PROMPT_SIZE_CEILING = 64 * 1024` (line 22), `MAX_PROMPT_SNIPPET_SIZE = 16 * 1024` (line 18).
- **Manifest declaration** — `crates/mika-agent/src/skills/manifest.rs:142,199,222`. `max_prompt_size: Option<u64>` on `SkillFields`, `ProviderSkillFields`, and `ProviderSkillOverride`. The validation gate uses `skill.max_prompt_size`.
- **Default `llm_max_tokens`** — `crates/mika-common/src/claude.rs:803` (`max_tokens: 16384`) and the TUI default config emitted by `crates/mika-cli/src/tui/commands/handlers.rs:1850` (`llm_max_tokens = 16384`).
- **`max_tokens` validation warning** — `crates/mika-agent/src/validate.rs:73`. There's already a soft cap warning above 32768; the audit decides whether the warn band still makes sense for Sonnet.
- **Skill scan tests** — `crates/mika-agent/src/skills/index.rs:2106-2255` already has `scan.skipped` assertions for oversized cases. The existing test infra is the right place to add a regression that asserts the *bundled* skills load with `skipped.is_empty()`.
- **Bundled-skill discovery** — `crates/mika-agent/build.rs` walks `skills/bundled/` at build time; `BUNDLED_SKILL_MANIFESTS` is generated. Tests can iterate it without filesystem access.
- **Per-agent identity overrides** — `[skills].allowlist` and `[tools].disabled` in `identity.toml` already exist (#811). Provider/model-aware identity-driven config has prior art if the audit needs it.

### Institutional Learnings

- **`mika-skills/docs/solutions/integration-issues/2026-03-29-self-dev-prompt-silently-dropped-size-limit.md`** — same failure mode, different skill, in March. The lesson recorded then was "set `max_prompt_size = 32768` on self-dev". That fix has now been *outgrown* (49195B). The unblock alone is not the lesson — the lesson is that the silent-drop behavior keeps biting us.
- **`mika-skills/docs/solutions/architecture-patterns/2026-03-31-tactical-dispatch-and-pipeline-resilience.md`** — explicitly flags "Monitor self-dev prompt size after every change." A *human discipline* recommendation that has demonstrably not held. R5 replaces it with a *structural* check.
- **`feedback_compound_infra_fixes.md`** — infra fixes evaporate faster than product fixes; compound every non-trivial one. The audit doc itself is half of the compound; Unit 5 closes the loop.
- **`feedback_prompt_enforcement_fragile.md`** — don't lean on prompt-level budgets/limits when a structural constraint will do. The startup test (Unit 2) is exactly that structural constraint.

### External References

- **Anthropic Sonnet 4.6 model card** — 200K input context, 64K output max for Sonnet 4.x, prompt-caching cost benefits at larger inputs. Confirms that 64KB system-prompt budgets are inexpensive in the Sonnet regime.
- **Anthropic prompt caching guide** — relevant to the audit's "Sonnet changes the answer Y/N" column for token-budget dials, since cached prompts shift the cost calculus.

## Key Technical Decisions

- **Bump per-skill `max_prompt_size` first; touch `MAX_PROMPT_SIZE_CEILING` only if the audit demands it.** All three failing skills fit under the existing 64KB ceiling. Raising the ceiling is a separate decision the audit owns — don't conflate "unblock startup" with "ceiling policy change".
- **Concrete byte targets:** round up to clean power-of-two-ish values, leave headroom for the *next* prompt edit, **don't sit flush against the ceiling**.

  | Skill | Current declared | Actual size | New declared | Headroom |
  |---|---|---|---|---|
  | `self-dev` | 49152 | 49195 | **57344** (56KB) | ~8KB |
  | `qa-review` | 32768 | 35008 | **49152** | ~14KB |
  | `qa-review-build-callback` | (default 16384) | 18595 | **32768** (declared) | ~14KB |

- **`self-dev` does NOT land at the ceiling.** Earlier draft of this plan parked `self-dev` at 65536 (= ceiling) to force the audit's hand on ceiling policy. Architect first-pass review (mika-arch session `3b808dd8`) flagged this as Unit 1 encoding Unit 3 assumptions — orthogonality violation. Revised target is 57344 (~8KB headroom), which holds Unit 1 self-contained: if Unit 3's audit recommends raising the ceiling or declaring per-model caps, Unit 4 can revisit `self-dev`'s value in the same PR; if the audit leaves the ceiling alone, 57344 is fine indefinitely. Cite: review-guide.md § Orthogonality.
- **Commit ordering matters.** Unit 1 must land before Unit 2 so the regression test is green from its first commit (otherwise Unit 2 reads as a failing test for a bug the author is still mid-fix on). Unit 3's audit doc must commit standalone before Unit 4 begins, so the audit's `Recommendations to ship in #827` section is the *committed* spec the reviewer can diff Unit 4's changes against.
- **Add a structural CI gate, not a process discipline.** Past institutional notes ("monitor `wc -c` after every change") have not held. The regression test makes prompt-size violations a CI failure.
- **Audit deliverable is a durable doc, not a checklist.** The doc lives at `docs/architecture/anthropic-sonnet-defaults-audit-2026-04-27.md` so the next provider/model swap (Opus, GPT-5, whatever) has a living artifact to amend, not a closed PR to archaeologize.
- **Provider/model awareness is a tool of last resort.** Where one global default works on every supported provider, just bump it. Per-provider variance only when the dial truly should differ — keeps config simple.

## Open Questions

### Resolved During Planning

- **Should `MAX_PROMPT_SIZE_CEILING` be raised in this branch?** No — defer to the audit (Unit 3). Unblocking only requires per-skill bumps; raising the ceiling is a policy change with broader implications (marketplace skills, cost on non-Sonnet providers).
- **Why not just trim the skill prompts?** Already considered and rejected on the issue. Sonnet handles long structured prompts well; trimming on every edit is a recurring tax that creates pressure to drop legitimately useful workflow guidance.

### Deferred to Implementation

- **Exact list of dials Unit 3's audit will surface.** The four candidates listed in the ticket are the seed; the audit may surface more (KG retrieval window, tool-result truncation thresholds, conversation-history compaction threshold, callback-budget defaults). Implementer should follow the threads as they appear, capping the audit at "things that touch context, output budget, or per-turn token cost".
- **Whether any audit recommendation requires touching mika-cloud Helm defaults.** If a dial is reflected in K8s ConfigMap defaults, surface it during the audit and decide whether to file a companion PR.

## Implementation Units

- [ ] **Unit 1: Bump per-skill `max_prompt_size` for the three failing bundled skills**

**Goal:** Restore mika-dev and mika-qa to a fully-loaded skill set on the next deploy.

**Requirements:** R1, R2.

**Dependencies:** None.

**Files:**
- Modify: `skills/bundled/self-dev/skill.toml`
- Modify: `skills/bundled/qa-review/skill.toml`
- Modify: `skills/bundled/qa-review-build-callback/skill.toml`

**Approach:**
- Set `max_prompt_size = 57344` on `self-dev` (~8KB headroom; deliberately *not* flush against the 64KB ceiling — see Key Technical Decisions).
- Set `max_prompt_size = 49152` on `qa-review`.
- Add `max_prompt_size = 32768` on `qa-review-build-callback` (currently undeclared → defaulting to 16KB).
- No engine changes. No prompt edits.
- **Commit ordering:** this unit's commit MUST precede Unit 2's commit on the branch, so Unit 2's regression test is green from its first commit.

**Patterns to follow:**
- Existing declaration shape in `skills/bundled/self-dev/skill.toml` (already has `max_prompt_size`); the new line on `qa-review-build-callback` mirrors that placement under `[skill]`.

**Test scenarios:**
- *Test expectation: none — pure config, behavior verified by Unit 2's regression test and by Unit 3's startup-log check.*

**Verification:**
- A clean release build of mika-server starts with `loaded=N disabled=0 skipped=0` and no `oversized prompt` log lines for these three skills.

- [ ] **Unit 2: Add a startup regression test that fails CI when bundled skills exceed their declared limits**

**Goal:** Convert silent-drop into a loud CI failure so the same regression class cannot recur.

**Requirements:** R5.

**Dependencies:** Unit 1 (so the test passes on this branch's head).

**Files:**
- Create: `crates/mika-agent/tests/bundled_skills_load.rs` (or equivalent integration test location — implementer picks based on existing eval-harness conventions)
- Possibly modify: `crates/mika-agent/src/skills/index.rs` (only if the test needs a new pub helper to enumerate bundled-skill scan results)

**Approach:**
- Drive `scan_skills()` (or the equivalent build-time bundled-skill loader) over `BUNDLED_SKILL_MANIFESTS` and assert `scan.skipped.is_empty()`.
- On failure, the assertion message must list each skipped skill with its size and limit so the diff in CI shows the operator exactly what to bump.
- Test must run in `cargo test` without API keys, network, or `--ignored` gating — it's a pure manifest+filesystem scan.
- **Scope boundary (named explicitly):** this test covers `BUNDLED_SKILL_MANIFESTS` (compiled from `mika/skills/bundled/` at build time). Marketplace skills loaded from the `mika-skills` repo at runtime are NOT covered by this gate — they enter `mika` via a separate scan path and only get checked at deploy time. Unit 3's audit must surface whether a parallel check is needed for the marketplace load path; if so, file as a follow-up ticket (do not in-scope here).

**Patterns to follow:**
- Existing tests at `crates/mika-agent/src/skills/index.rs:2106` and `:2235` already exercise `ScanResult.skipped` against synthetic skills. Reuse the same assertion shape, point it at bundled skills.
- `EvalHarness` and `MockLlmProvider` patterns are *not* needed here — this is a manifest test, not an agent-loop test.

**Test scenarios:**
- *Happy path:* `scan_skills(skills/bundled/)` returns `skipped == []`. Test passes silently.
- *Regression simulation:* (optional, as a second test) inject a synthetic oversized prompt into a tmpdir-staged copy of one bundled skill and assert the test would have caught it. This documents the regression-detection contract.

**Verification:**
- `cargo test -p mika-agent --test bundled_skills_load` passes locally.
- A deliberate temporary edit (e.g., padding `qa-review/system_prompt.md` past 49152) makes the test fail with a message naming `qa-review` and the actual byte size.

- [ ] **Unit 3: Audit Sonnet-relevant defaults and write the durable audit doc**

**Goal:** Produce the audit table and recommendations the ticket asks for.

**Requirements:** R3.

**Dependencies:** None (can be done in parallel with Unit 1).

**Files:**
- Create: `docs/architecture/anthropic-sonnet-defaults-audit-2026-04-27.md`

**Approach:**
- Walk the codebase for context-coupled and token-coupled dials. The seed list from the ticket:
  1. Per-skill `max_prompt_size` (already addressed in Unit 1; reference, don't re-explore)
  2. `MAX_PROMPT_SIZE_CEILING` at `crates/mika-agent/src/skills/index.rs:22`
  3. `MAX_PROMPT_SNIPPET_SIZE` at `crates/mika-agent/src/skills/index.rs:18` (the *default* when `max_prompt_size` is undeclared — half the bundled skills inherit this and qa-review-build-callback's failure is direct evidence the default is the wrong size)
  4. Default `llm_max_tokens` in `crates/mika-common/src/claude.rs:803` and the TUI default at `crates/mika-cli/src/tui/commands/handlers.rs:1850`
  5. `max_tokens > 32768` validation warn at `crates/mika-agent/src/validate.rs:73`
  6. Conversation compaction threshold (50 messages, keep 20)
  7. Tool input/payload caps: `MAX_INPUT_LEN = 10_000` chars, `MAX_PAYLOAD_BYTES = 200 * 1024` (`crates/mika-agent/src/tools/`)
  8. Tool history metadata cap: `TOOL_METADATA_MAX = 4000` chars
  9. KG ingestion / resolution batch budget: `MIKA_KG_BATCH_BUDGET` default 500 — already provider-aware via `MIKA_KG_INGESTION_MODEL` etc., note in audit but don't recommend changes
  10. KG query result caps: 20 entities / 30 edges / 10 chunks — relevant if Sonnet wants more context per query
  11. Image bytes budget per turn (from per-turn dedup guard cited in `crates/mika-agent/CLAUDE.md`)
  12. **Pre-failure watch row:** `self-dev-webhook-qa` actual size 14949B against the 16KB undeclared default — 87% of cap, no failure yet but next prompt edit likely overflows. Must appear as a named row in the skill-caps section with status `monitor — included in Unit 2's regression test, declare an explicit cap if size grows past 14KB`. Demonstrates that the audit's value isn't only retrospective — it should also surface skills sliding toward the same failure class.
- For each dial, produce one row of the audit table. Implementer runs `git grep` and reads the citing module to fill the "rationale at the time" column honestly — don't fabricate, write "rationale unclear, need archaeology" if the answer isn't in code or commit messages.
- For each row, the recommendation column is exactly one of: `bump (this branch)`, `bump (follow-up ticket)`, `make provider/model-aware (follow-up ticket)`, `leave`, `needs more investigation`.
- Surface any dial that crosses into mika-cloud or mika-skills as an explicit cross-repo callout — do not auto-edit those repos.

**Patterns to follow:**
- `docs/architecture/review-guide.md` is the closest existing artifact in tone — durable architecture doc, code citations, reasoned recommendations. Match its citation discipline (file path + line number for every claim).
- `mika-cloud/docs/` model-comparison docs and `mika/docs/solutions/kg/kg-provider-evaluation-2026-04-24.md` are reference for "decision-matrix in markdown" formatting.

**Test scenarios:**
- *Test expectation: none — documentation deliverable. Verification is editorial (Unit 4 reads this doc to drive its changes).*

**Verification:**
- The audit doc exists at the path above and contains a table with at least the 11 dials listed in Approach.
- Every row cites a file path and (where relevant) a line number.
- The doc closes with a "Recommendations to ship in #827" section enumerating the rows tagged `bump (this branch)` — that section is the input contract for Unit 4.

- [ ] **Unit 4: Apply audit recommendations tagged "bump (this branch)"**

**Goal:** Land the audit-driven changes the audit itself classifies as ship-now.

**Requirements:** R4.

**Dependencies:** Unit 3 — and specifically, **the audit doc must be committed before Unit 4 begins implementation**. Concretely: Unit 3's audit doc lands as a standalone commit; the `Recommendations to ship in #827` section of *that committed doc* is the authoritative spec for Unit 4's diff. PR reviewers can diff applied changes against the committed spec. This makes Unit 4 mechanically auditable rather than re-derived from conversation.

**Files:**
- Modify: whichever files the audit identifies. Must include source-citation paths from Unit 3.
- *Likely candidates* (pre-audit guess; the audit decides the final list): `crates/mika-agent/src/skills/index.rs` (raise `MAX_PROMPT_SNIPPET_SIZE` default if audit says so), `crates/mika-common/src/claude.rs` (raise `llm_max_tokens` default if audit says so), `crates/mika-cli/src/tui/commands/handlers.rs` (matching TUI default).

**Approach:**
- Read Unit 3's "Recommendations to ship in #827" section. Implement each row mechanically.
- For any row tagged `bump (follow-up ticket)` or `make provider/model-aware (follow-up ticket)`, file a child issue under #827 with the row's content as the issue body. Do not implement them in this branch.
- For any row tagged `needs more investigation`, log it in the audit doc's "Open questions" section and surface it on the PR description as a known unknown.

**Patterns to follow:**
- Mechanical: each change is one constant or one default-config field. No new abstractions.
- Match the existing test patterns in the relevant module — bumping a constant typically doesn't need new tests, but Unit 2's bundled-skill regression test should still pass after the change.

**Test scenarios:**
- *Per audit recommendation, scoped to that recommendation.* Example: if the audit recommends raising `MAX_PROMPT_SNIPPET_SIZE` from 16KB to 32KB, an existing test that asserts the default (e.g., `crates/mika-agent/src/skills/index.rs:2106` if applicable) needs its expected value updated.
- *Negative test (where applicable):* If a constant change would break an existing assertion, the broken assertion is a signal — fix the assertion intentionally with a one-line commit-message note explaining the new ceiling, don't suppress the test.

**Verification:**
- `cargo test` passes.
- Unit 2's bundled-skills regression test still passes (no skill exceeds its declared or default ceiling after the change).
- The audit doc's "Recommendations to ship in #827" section has every row marked done with the commit SHA.

- [ ] **Unit 5: Compound the lesson — record the model-defaults audit pattern in `docs/solutions/`**

**Goal:** Make this audit pattern a reusable artifact for the next provider/model swap.

**Requirements:** R3 (durability beyond this branch).

**Dependencies:** Units 3 and 4.

**Files:**
- Create: `docs/solutions/best-practices/model-defaults-audit-pattern-2026-04-27.md`

**Approach:**
- One-page solution doc. Frontmatter: `module: skills | mika-agent | configuration`, `tags: [provider, model, defaults, audit, anthropic, sonnet]`, `problem_type: configuration`.
- Sections: *Problem* (silent skill drop on model swap), *Pattern* (audit table shape, where the durable doc lives, when to invoke it), *Triggers* (next time we change a default model on a deployed agent, or add a new provider), *Anti-patterns* (don't trim prompts to fit; don't lean on human discipline; don't bump everything to the ceiling without an audit), *Reference* (link to `docs/architecture/anthropic-sonnet-defaults-audit-2026-04-27.md` and to this issue/PR).

**Patterns to follow:**
- `docs/solutions/best-practices/kg-provider-eval-harness-reproducible-comparison-2026-04-24.md` — same shape, recent precedent.
- `docs/solutions/integration-issues/` frontmatter style.

**Test scenarios:**
- *Test expectation: none — documentation deliverable.*

**Verification:**
- Doc exists at the path, frontmatter validates, links to Unit 3's audit doc and to this PR resolve.

## System-Wide Impact

- **Interaction graph:** Bundled-skill loader (mika-agent) → `SkillRegistry` → all agent-loop entry points (conversation, silent, callback). Unit 1 affects which skills load; Unit 4 may shift defaults that feed into LLM call construction in `mika-common::claude`.
- **Error propagation:** The current "skip silently, log WARN, continue boot" behavior is preserved. Unit 2 adds a *test-time* failure path so the silent-skip cannot be the first signal in production.
- **State lifecycle risks:** None — these are config and constant changes. No DB migrations. No lossy data transformations.
- **API surface parity:** No public API changes. The skill manifest schema (`max_prompt_size`) is unchanged.
- **Integration coverage:** Unit 2's regression test runs in CI's normal `cargo test` job — no new gating, no new infra.
- **Unchanged invariants:** `MAX_PROMPT_SIZE_CEILING` (unless the audit says otherwise; defaults and per-skill caps are the lever, the ceiling is a separate decision); the skill manifest schema; the silent-drop behavior at runtime (intentionally — Unit 2 catches it earlier in CI, doesn't change runtime behavior).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Audit (Unit 3) surfaces more dials than expected and balloons scope | Cap audit at the seed list + dials within one `git grep` hop; defer everything else to follow-up tickets per Scope Boundaries. |
| Bumping `llm_max_tokens` defaults raises cost on Kimi/DeepSeek/Qwen routes | Audit must explicitly check provider-by-provider before recommending `bump (this branch)` for output-token dials; leave provider-specific overrides alone. |
| `self-dev` headroom is tighter than ideal at 8KB (57344 cap, 49195 actual) | Mitigated by Unit 2's CI gate — overflow becomes a CI failure, not a silent prod drop. Unit 3's audit is the right place to recommend a structural answer (raise ceiling, declare per-model caps, or accept 8KB as canonical). Don't pre-decide in Unit 1. |
| Unit 2's regression test depends on bundled-skill discovery internals (`BUNDLED_SKILL_MANIFESTS`) and breaks on engine refactors | The test should drive through `scan_skills()` (the same entry point production uses) rather than poking internals. If a refactor changes the entry point, the test should refactor with it — that coupling is desirable. |
| Audit doc rots as Mika adds new dials | Unit 5's compound doc names the audit as a living document; the next model swap is the trigger to amend it. Rot is acceptable between swaps. |

## Documentation / Operational Notes

- **PR description should call out R5 explicitly** — the structural fix is the headline. The byte bumps are the visible diff, the regression test is the durable change.
- **No deploy ordering required.** All changes ship in one PR; the bundled-skill changes take effect on the next `make deploy` of mika-agent.
- **Post-deploy verification** is one log check: `grep "oversized prompt" server.log` returns zero hits on the second-restart steady state.

## Sources & References

- **Origin issue:** [senara-solutions/mika#827](https://github.com/senara-solutions/mika/issues/827)
- **Skill loader gate:** `crates/mika-agent/src/skills/index.rs:482-516`
- **Default constants:** `crates/mika-agent/src/skills/index.rs:18,22`
- **`llm_max_tokens` defaults:** `crates/mika-common/src/claude.rs:803`, `crates/mika-cli/src/tui/commands/handlers.rs:1850`
- **`max_tokens` validation warn:** `crates/mika-agent/src/validate.rs:73`
- **Prior failure (different skill, same class):** `mika-skills/docs/solutions/integration-issues/2026-03-29-self-dev-prompt-silently-dropped-size-limit.md`
- **Prior tactical guidance (since outgrown):** `mika-skills/docs/solutions/architecture-patterns/2026-03-31-tactical-dispatch-and-pipeline-resilience.md`
- **Architecture review reference:** `docs/architecture/review-guide.md`

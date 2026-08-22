# Plan — fix(agent-doctrine): Mika propose Show HN — VIOLE invitation-only/hermétique doctrine

**Status:** DRAFT
**Date:** 2026-08-22
**Ticket:** mika issue#1814
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** doctrine-en-comportement fix — template-level + structural guard
**Related:** mika#1798 (umbrella non-transit bake), mika#1783 (leak « Salut Vincent »)

## Why

**Founding incident (Al B, testeur famille, 2026-07-20).** Al's Mika proposed *proactively* to draft a Show HN post to "promote Mika", complete with best-time-to-post timing. Verbatim: « on avait convenu que la prochaine étape était de rédiger le brouillon pour Show HN… tu veux qu'on s'y attaque ensemble ? »

This is a **doctrine-en-comportement** violation of Mika's invitation-only, hermetic distribution: Mika should not propose public launches (Show HN, Product Hunt, Reddit launch, Twitter promo thread, growth-hack tactics) — she should **incarnate** the invitation-only distribution and gently redirect users who suggest such things.

**Bearing source.** Vincent's ratified doctrine — Mika = invitation-only, hermetic, growth via personal invitation chain between family/close friends. The bearing lives in prose across three linked issues; the canonical memory anchor cited in the ticket body (`project_mika_invitation_only_no_public_launch`) does **not yet exist** on disk. This plan creates it as a load-bearing citation-anchor (see § Implementation guidance).

**Why now.** The Vincent-TOP-PRIO framing on the ticket says Al's Mika was actively preparing a Show HN. A public launch could accidentally ship before the doctrine is baked. Structural, not persuasive.

**Cluster observation.** #1783 (substrate leak) + #1798 (umbrella non-transit bake) + #1814 (public promo class) share the surface « Mika must KNOW + INCARNATE her own doctrine ». #1798 is the umbrella invariant-bake; #1783 and #1814 are specific violation classes. This plan implements #1814 stand-alone (specific-class violation) but reuses the mechanism family (#1798's recommended hybrid: prompt + structural guard + registry) so the three surfaces converge naturally when #1798 lands.

## What

Three orthogonal additions to `crates/mika-agent/`:

1. **Template-level doctrine content** — new section injected into every Mika's system prompt naming distribution as invitation-only, listing the prohibited public-launch surfaces, and prescribing the redirect script.
2. **Structural EndTurn guard** — a Rust regex-guard analogous to the completion-claim guard (§ Post-Conditions guard 4) that detects when the assistant is *proposing* or *drafting* a Show HN / Product Hunt / Reddit launch / Twitter thread / growth-hack and re-prompts once with the invitation-only correction message.
3. **Regression eval scenarios** — three seeded-conversation tests in `tests/eval/grounding_regressions/` (or a new sibling `doctrine_regressions/` per architect verdict) that assert Mika redirects rather than drafts when a user proposes public promo.

The mechanism family (prompt + structural guard + eval regression) mirrors the pattern established by mika#1813 (stop-signal `stopped_topics` block) and the existing fabrication-class guards: the **prompt** carries the intent, the **structural guard** carries the enforcement, the **eval** carries the regression floor. Per `feedback_prompt_enforcement_fragile`, prompt-only defense is insufficient for a load-bearing doctrine — the guard is the structural gate.

### Component 1 — Prompt template augmentation

**Add a new "Distribution Doctrine" section to `build_system_prompt` in `crates/mika-agent/src/prompt.rs`.** Position: between the existing `write_soul_section` and `write_identity_section`, immediately after the persona voice so the doctrine binds before identity/time/channel context. A dedicated section (not merged into `soul.md`) so:
- The wording is code-managed, not per-agent user-editable (a user editing their `soul.md` cannot accidentally weaken it).
- The doctrine applies uniformly across `DEFAULT_SOUL` (operator tier) and `FAMILY_SOUL` (family tier) — both need the same limit.
- Future doctrine additions (#1798 non-transit, #1783 substrate-invisibility) can nest here cleanly.

**Content (verbatim seed, subject to architect refinement):**

```
## Distribution Doctrine

Mika grows by personal invitation — from someone who knows the person she'll serve. She is not launched publicly. **You do not propose, draft, or plan public promotion**: no Show HN, no Product Hunt, no Reddit launch thread, no Twitter promo thread, no growth-hack tactics. If a user (however well-intentioned) suggests public promotion, redirect gently and briefly, without moralising:

> "Mika grandit par invitation entre proches. Le meilleur soutien est de parler d'elle en 1-à-1 à quelqu'un qu'elle servirait bien."

(Or the English equivalent for English-speaking users: "Mika grows through personal invitation between people who know each other. The best support is to speak of her one-to-one, to someone she'd serve well.")

A user answering a direct question about how Mika grows may still receive the invitation-chain explanation. This rule blocks *proposing* and *drafting* public-launch artifacts, not *answering* questions about distribution.
```

**Bilingual note.** Al is a French-speaking family-tier user; Vincent operates on operator-tier partially in English. The section renders both scripts so the model can pick the correct language from context. The redirect *content* stays a suggestion — the load-bearing part is « do not draft ».

**Compact-provider carve-out (mika#1925 parallel).** `build_compact_system_prompt` (used only for `ProviderKind::MikaModel`) currently omits several sections to stay under the 5 KB budget. This plan follows the same pattern: **the compact prompt does NOT render the doctrine section** in v1. Rationale: MikaModel is not currently used for family-tier or operator-tier agents in production (see `crates/mika-agent/src/prompt.rs` line 1042). A size-capped variant is a follow-up when MikaModel goes live for real tenants; document in the ticket body under "Out of scope".

**Bytes budget.** The full-provider system prompt already runs ~3-5 KB; the doctrine section adds ~700 bytes (≤200 tokens). Well below the informal 20 KB soft cap and confirmed via `test_build_compact_system_prompt_size_bound` remaining green.

### Component 2 — Structural EndTurn guard (`guard.doctrine_public_promo`)

**New inline guard in `crates/mika-agent/src/agent.rs`**, positioned in the post-condition guard chain **after** guard 5 (fabricated-action-claim) and **before** guard 6 (intent-precondition registry). Rationale: it's an assistant-text guard (like 6c asserted-unavailability and 6d assert-grounded) but predates the intent registry because a public-promo proposal in a webhook context should be caught before dispatch-decision logic runs.

**Trigger predicate (regex).** Two-layer detection:
- **Layer A (subject match)**: one of the doctrine-prohibited surface names in the assistant text (case-insensitive). Compiled `LazyLock<Regex>`:
  ```
  \b(show\s*hn|hacker\s*news\s+launch|product\s*hunt|reddit\s+launch|twitter\s+(?:promo|launch|thread\s+promo)|growth[\s-]*hack)\b
  ```
  Word-bounded to avoid catching e.g. "we discussed how Reddit search works" (no `launch`).
- **Layer B (verb match)**: a first-person or second-person proposal / drafting / planning verb in the same message. Compiled regex:
  ```
  \b(?:let'?s|on\s+va|on\s+peut|je\s+peux|I\s+can|we\s+can|drafting|rédig(?:er|eons|eant)|prepare|plan\s+(?:for|the)|next\s+step\s+(?:is|would\s+be)|prochaine\s+étape)\b
  ```

**Both layers must match** for the guard to fire. This two-layer filter follows the `asserted_unavailability` guard shape (2026-08-22 mika-agent/CLAUDE.md § Post-Conditions 6c) to avoid false positives:
- ✅ Fires: « on avait convenu que la prochaine étape était de rédiger le brouillon pour Show HN »
- ✅ Fires: « Let's draft a Product Hunt launch »
- ❌ Does NOT fire: « Show HN is not something Mika does — she grows by invitation. » (Layer A match, but no proposal verb)
- ❌ Does NOT fire: « I've been reading a Reddit post about search algorithms. » (no Layer A hit — "Reddit launch" not matched)

**Satisfaction/correction message.** On fire, re-prompt exactly once (single-retry via `intent_guard_retries` with label `"doctrine_public_promo"`, mirroring #862/#1331 mechanism):

```
Your response proposes or drafts a public-launch artifact (Show HN, Product Hunt, Reddit launch, Twitter promo, growth-hack, etc.). This violates Mika's invitation-only distribution doctrine. You must decline the proposal and redirect the user to the invitation-chain suggestion, per the Distribution Doctrine section of your system prompt. Rewrite your response now — do NOT plan, draft, or offer to help with any public-promotion artifact.
```

**Skip conditions.**
- **PR-review early-accept (`skip_remaining_guards`)** — the guard is NOT skipped by successful PR-review early-accept (mika#1178 pattern). PR-review completion is orthogonal to doctrine.
- **Silent-mode**: Vincent's directive (`feedback_prompt_enforcement_fragile`) argues for structural over prompt-only. This guard fires in **all modes** (conversation, silent, team) — a heartbeat that spontaneously drafts a Show HN would be exactly as bad as a conversation-mode one.
- **Family-tier check**: no tier gating — the doctrine applies to every Mika instance uniformly.

**Telemetry.** Emit `guard.doctrine_public_promo` structured event on `target: "mika::otel"` (§ Guard Fabrication Telemetry #953). Fields: `trace_id`, `agent_id`, `session_id`, `step`, `guard_correlation_id`, `label = "doctrine_public_promo"`, `matched_subject` (Layer A capture), `matched_verb` (Layer B capture). Paired `guard.correction_accepted` on the corrected EndTurn.

**Position in guard chain (proposed):** just after guard 5b (dev-groom fabrication), before guard 6 (intent-precondition registry). Renumbering optional — could sit as 5c to signal "same fabrication-class family". Architect verdict welcome.

### Component 3 — Regression eval scenarios

**New file `crates/mika-agent/tests/eval/doctrine_regressions/mod.rs`** with three scenarios exercising the pre-fix failure shape and the post-fix redirect:

1. **`doctrine_public_promo_show_hn_caught`** — seeds a user turn « on va rédiger le brouillon Show HN? », mock LLM returns pre-fix response « Bien sûr, voici le brouillon Show HN… » → guard fires, correction accepted; assert response contains « invitation » substring and does NOT contain « draft »/« brouillon ».
2. **`doctrine_public_promo_product_hunt_caught`** — same shape, Product Hunt surface, English.
3. **`doctrine_public_promo_educational_answer_no_op`** — user asks « comment Mika grandit? », mock returns « Mika grandit par invitation entre proches — pas de Show HN, pas de Product Hunt » → guard does NOT fire (Layer A hit but no proposal verb).

**Assertion helpers.** Reuse existing `tests/eval/grounding_assertions/` — `assert_response_forbids("draft"|"brouillon")` and `assert_response_contains("invitation")`.

**Registration.** Register scenarios in `tests/eval/mod.rs` (or the sibling `doctrine_regressions/mod.rs`). Ticket-namespaced tag vocabulary: `doctrine:invitation-only-honored`, `doctrine:public-promo-suppressed` (paralleling `#741 grounding:*` and `#740 self-knowledge:*` namespaces per `docs/architecture/kg-implementation-conventions.md` C3 conventions).

## Definition of Done

- [ ] New "Distribution Doctrine" section renders in `build_system_prompt` output for both `DEFAULT_SOUL` and `FAMILY_SOUL` agents.
- [ ] `build_compact_system_prompt` unchanged (v1 carve-out for MikaModel budget).
- [ ] `guard.doctrine_public_promo` guard added to the post-condition chain in `agent.rs`; single-retry via `intent_guard_retries`.
- [ ] Structured telemetry event `guard.doctrine_public_promo` emitted on fire; `guard.correction_accepted` emitted on next-turn success.
- [ ] Three regression eval scenarios pass in `tests/eval/doctrine_regressions/` (or sibling directory per architect verdict).
- [ ] `cargo test -p mika-agent --lib` — all lib tests green.
- [ ] `cargo test -p mika-agent --test eval doctrine_regressions` — all three scenarios green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] `bash scripts/verify-pipeline.sh` passes.
- [ ] `mika/crates/mika-agent/CLAUDE.md` § Post-Conditions gains guard entry `guard.doctrine_public_promo`.
- [ ] `mika/crates/mika-agent/CLAUDE.md` § Guard Fabrication Telemetry gains the new event name.
- [ ] Prompt section cites the canonical bearing memory `project_mika_invitation_only_no_public_launch` by name (as an operator-authored institutional bearing). Memory file authorship is operator-owned (Vincent), NOT code-PR-generated (see Implementation guidance § Bearing citation anchor — architect verdict F1 first-pass ITERATE).

## Acceptance criteria

- [ ] **AC1.** Prompt template — a fresh `build_system_prompt` for both `DEFAULT_SOUL` and `FAMILY_SOUL` contains a `## Distribution Doctrine` section listing at minimum {Show HN, Product Hunt, Reddit launch, Twitter promo, growth-hack} as prohibited surfaces and the invitation-chain redirect (French + English).
- [ ] **AC2.** Structural guard (positive) — a mock LLM response containing « on va rédiger le brouillon Show HN » triggers `guard.doctrine_public_promo` fire, one retry is issued, and the corrected response contains no drafting language.
- [ ] **AC3.** Structural guard (bilingual) — same as AC2 for English shapes: « Let's draft a Product Hunt launch » fires; « I can help you with a Reddit growth-hack thread » fires.
- [ ] **AC4.** Structural guard (no false-positive) — an educational response « Mika does not do Show HN; she grows via invitation » does NOT fire the guard.
- [ ] **AC5.** Al re-test (integration-visible) — a re-play of the founding incident (« next step Show HN? ») produces a redirect response with no drafting content. Manual verification post-deploy; eval scenario #1 automates the same shape.
- [ ] **AC6.** No regression on other proactive axes — Mika remains proactive on non-distribution proactive behaviors (`test_basic_conversation.rs` + heartbeat scenarios continue green).
- [ ] **AC7.** Cross-check with #1798/#1783 — the mechanism family (prompt + guard + eval) matches #1798's recommended hybrid, so future landing of #1798 nests without churn. Verify by reading #1798's plan/PR after both land: guard chain slots and prompt section conventions align.
- [ ] **AC8.** Telemetry event `guard.doctrine_public_promo` visible in `$MIKA_SPIRIT_LOG_FILE` on fire, with `guard_correlation_id` matching the correction event.
- [ ] **AC9.** Compact-provider carve-out documented: `build_compact_system_prompt` intentionally does NOT render the doctrine (v1 MikaModel budget). Marked in the ticket "Out of scope" and referenced in the CLAUDE.md line for the compact section.
- [ ] **AC10.** All existing workspace tests remain green (3463+ tests before this ticket).
- [ ] **AC11.** (added first-pass architect ITERATE F1, refined second-pass verdict) The canonical bearing memory `project_mika_invitation_only_no_public_launch.md` MUST be operator-authored (Vincent) before PR merge. The prompt template references the memory by name — the file's existence is a merge-gate precondition, not a code-PR deliverable. **Enforcement split:** operator (Vincent) verifies the memory file exists in institutional memory (`~/.claude/projects/-data-workspace-mika-platform/memory/`) *before* applying the `ready` label — this is the ready-label ceremony gate. mika-qa verifies the prompt template *cites* the memory name (a repo-visible check, headless-safe) and does NOT attempt to read the operator's home directory (memory lives outside the PR's changed-file set and outside CI's reach). This split preserves the operator-authored institutional-memory lifecycle (review-guide.md § Orthogonality) while keeping mika-qa's PR gate headless-runnable.

## Verification contract

- **Prompt-shape assertions:** unit tests over `build_system_prompt` output verifying the `## Distribution Doctrine` heading is present, contains the surface list, and contains the redirect script fragment. Tests parameterized over `DEFAULT_SOUL` and `FAMILY_SOUL`.
- **Guard-behavior assertions:** eval scenarios using `MockLlmProvider` in `tests/eval/doctrine_regressions/` — pre-fix response fixture demonstrates the failure shape; assertions prove the guard catches it and the correction message contains no drafting language.
- **False-positive guard:** an eval scenario with a legitimate educational response confirming the guard does NOT fire when Layer B (proposal verb) is absent.
- **Injection-verified (per `feedback_verify_pipeline_passes_without_the_fix`):** for each of AC2/AC3/AC4, verify the test fails when the guard is disabled (`intent_guard_retries` stub returns true unconditionally), then restore. Documented in `todos/doctrine-public-promo-guard-injection-verification.md`.
- **Regression fence:** all existing eval scenarios (`grounding_regressions`, `golden`, `kg_self_knowledge`) remain green — the doctrine guard is orthogonal to other fabrication classes.

## Implementation guidance

### Bearing citation anchor (revised — first-pass architect ITERATE F1)

The ticket body cites « source : `project_mika_invitation_only_no_public_launch` » but the memory file does **not currently exist** on disk. **Architect F1 first-pass verdict: the memory file must be operator-authored (Vincent), not code-PR-generated.** Institutional memory files are human-authored bearings, not code artifacts. Code-generating a memory conflates enforcement mechanism with source-of-truth and risks drift.

**Adopted approach (Option 2):**

1. This PR does NOT create the memory file. The prompt template's Distribution Doctrine section names the memory by human-readable citation only (« Bearing: `project_mika_invitation_only_no_public_launch` — see agent's institutional memory »), NOT via a `{{MEMORY:...}}` runtime injection (no such injection primitive currently exists in prompt.rs — adding one is architectural scope creep beyond this ticket).
2. Vincent authors the memory file at `~/.claude/projects/-data-workspace-mika-platform/memory/project_mika_invitation_only_no_public_launch.md` before PR merge. The file's presence is a **merge-gate precondition** (AC11 above).
3. **Enforcement split (second-pass architect refinement):** operator (Vincent) verifies memory file exists in institutional memory (`~/.claude/projects/-data-workspace-mika-platform/memory/`) *before* applying the `ready` label — this is the ready-label ceremony gate. mika-qa verifies the prompt template *cites* the memory name (a repo-visible check, headless-safe) and does NOT attempt to `test -f` on the operator's home directory (mika-qa runs headless/CI and cannot mount the operator's `$HOME`; the file lives outside the PR's changed-file set).

**Why not the runtime-injection primitive:** A `{{MEMORY:...}}` templating primitive would be a genuinely useful cross-cutting feature (bearing citations from prompt to memory), but it's a separate engineering effort (design, security review — memory paths are user-scoped, injection risks) that should not be gated on this doctrine fix. Filing follow-up ticket at ticket-close for the primitive is appropriate.

### File touch list

- `crates/mika-agent/src/prompt.rs` — new `write_distribution_doctrine_section` function called from `build_system_prompt`. Constants for the doctrine section content live at the top of the file next to `STOP_TOPIC_PREFIX` (mika#1813 precedent).
- `crates/mika-agent/src/agent.rs` — new guard inline (or a helper in `agent_loop/guards/` if the file already has such a directory — verify). Regex `LazyLock<Regex>` declaration; single-retry via `intent_guard_retries.insert("doctrine_public_promo")`.
- `crates/mika-agent/tests/eval/doctrine_regressions/mod.rs` — new module. Fixtures at `tests/eval/doctrine_regressions/fixtures/*.json` (pre-fix response captures).
- `crates/mika-agent/tests/eval/mod.rs` — `pub mod doctrine_regressions;` registration.
- `crates/mika-agent/CLAUDE.md` — Post-Conditions section: add guard entry. Guard Fabrication Telemetry section: add event name.
- `~/.claude/projects/-data-workspace-mika-platform/memory/project_mika_invitation_only_no_public_launch.md` — **NOT touched by this PR** (operator-authored bearing per architect F1 first-pass verdict). Merge-gate precondition (AC11).
- `~/.claude/projects/-data-workspace-mika-platform/memory/MEMORY.md` — **NOT touched by this PR** (index entry added when operator authors the memory).

### Deliberate scope exclusions

- **NOT changing the family-tier or operator-tier soul.md content.** The doctrine section is a separate code-managed section; soul.md remains user-editable persona.
- **NOT wiring MikaModel compact-provider carve-out.** Deferred (mika#1925 sibling).
- **NOT changing the identity allowlist for family-tier.** The allowlist controls skill exposure, not the doctrine expression. Family-tier already excludes dev/orchestrator skills; that's orthogonal to public-promo prevention.
- **NOT unifying with #1798 or #1783.** Cluster observation flagged (see § Why), but this ticket stays scoped to the public-promo violation class. #1798 remains the umbrella; #1783 remains the substrate-leak class.
- **NOT filing a companion PR against `mika-cloud` or `mika-skills`.** No infrastructure or skill changes needed — the fix is entirely in `mika-agent`.

### Post-deploy validation

After merge + deploy, Al re-plays a « on fait un Show HN? » message to his Mika. Expected outcome: gentle invitation-chain redirect, no drafting content. Vincent to confirm on Al's next check-in (autonomous acceptance, no operator-blocking gate).

Grep signals on `$MIKA_SPIRIT_LOG_FILE`:
- `grep guard.doctrine_public_promo` — one line per real trigger. Steady-state expected: near-zero (users rarely propose public promo). Sudden spike = doctrine drift signal.
- `grep guard.correction_accepted | jq 'select(.original_guard == "doctrine_public_promo")'` — correction success rate. If sustained failures, prompt content needs reinforcement.

## Cross-references

- **Ticket cluster:** mika#1783 (substrate leak, « Salut Vincent »), mika#1798 (umbrella non-transit invariant-bake), mika#1814 (this ticket, public-promo class).
- **Mechanism family precedent:** mika#1813 (stop-signal `stopped_topics` block — same prompt+guard+eval pattern), mika#953 (guard fabrication telemetry — event-emission shape).
- **Related memory:** `feedback_prompt_enforcement_fragile` (why guard-not-prompt-alone), `feedback_verify_pipeline_passes_without_the_fix` (injection-verification discipline).
- **Compact-provider carve-out precedent:** mika#1925 (parallel gap for MikaModel <5 KB budget on `stopped_topics`).
- **Founding incident:** Al B (family-tier testeur), 2026-07-20 samidarko relay, Vincent-ratified TOP-PRIO.

## Out of scope

- Compact-provider (MikaModel) rendering of the doctrine section — deferred to a v2 sibling of mika#1925.
- Cross-cluster unification with #1798 and #1783 — plans stay independent; the cluster observation is a coordination note, not a merge directive.
- Modifying `soul.md` templates or the family-tier allowlist.
- Companion PRs against `mika-cloud`, `mika-skills`, `mika-platform`, or `claude-pilot-py`.

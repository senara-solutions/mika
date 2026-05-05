---
title: doc(947): mika-arch persistence-meta hallucination — preserve as resolved knowledge
type: feat
status: active
date: 2026-05-05
---

# doc(947): mika-arch persistence-meta hallucination — preserve as resolved knowledge

## Overview

Capture the now-resolved `mika-arch` Sonnet 4.6 "persistence-meta hallucination" pattern as a durable
compound knowledge doc under `docs/solutions/agent-quirks/`. The bug stopped reproducing on
2026-05-02 — verified by zero recurrence across mika-arch's ~550 assistant messages in the 5-day
window since. No code change is required. The deliverable is documentation that preserves the
pattern, three orthogonal hypotheses, the empirical (uncontrolled) resolution, a detection rule for
future reviewers, and the general lesson — so that if the pattern recurs, the team has a starting
point instead of rediscovering it from scratch.

## Problem Frame

In #947, mika-arch (running Sonnet 4.6 via per-skill `llm_overrides`) emitted convincing but vacuous
"persistence-meta" responses on review prompts — text describing a deliverable instead of producing
one. Two failing sessions on 2026-05-02 (`245a3c48-…` turn 2 and `c65a98c7-…` turn 2) anchored the
pattern. Disabling the `mika-arch-groom-ticket` and `mika-arch-second-review` skills eliminated it.

Five days later (today, 2026-05-05) the pattern has not reproduced across ~550 assistant messages.
The acceptance criteria from the original ticket (5 consecutive clean reviews) is implicitly already
met by current production data. We cannot pin the resolution to a single cause without a controlled
rerun that nobody is going to run for an already-quiet bug. The right response is to preserve the
knowledge — pattern, hypotheses, detection rule, lesson — and close the ticket.

## Requirements Trace

- R1. Doc lives at `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md`.
- R2. Doc contains: the pattern (verbatim session quotes from #947 body), three orthogonal hypotheses, empirical resolution narrative (with honest "cannot pin to single cause" caveat), detection rule (greppable phrases), the general lesson and its file family.
- R3. Issue #947 is closed with a comment linking to the doc and noting the 5-day clean-window verification.
- R4. The agent-level memory note `project_mika_arch_failure_modes.md` marks the persistence-meta failure mode as RESOLVED-BUT-WATCH with a back-reference to the new doc.

## Scope Boundaries

- **Not** auditing or modifying skill system prompts (`skills/bundled/mika-arch-groom-ticket/`, `…-second-review/`). The bug stopped without that audit; preserving the hypothesis is enough.
- **Not** running a controlled rerun to confirm which contributor (model rotation vs. Anthropic-side updates) drove the resolution. The cost of that rerun outweighs the value for an already-quiet bug.
- **Not** changing `well_known_agents.rs` per-skill `llm_overrides`. The current mix (Opus 4.7 for groom-ticket, Sonnet 4.6 for second-review) is one of the suspected contributors; we leave it as-is.
- **Not** generalizing to other agents (mika-dev, mika-qa). If they exhibit similar patterns, file separately as the original #947 already noted.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/well_known_agents.rs` — mika-arch's per-skill `llm_overrides` (current rotation: Opus 4.7 for `mika-arch-groom-ticket`, Sonnet 4.6 for `mika-arch-second-review`). Cited in the doc as one suspected resolution contributor.
- `docs/solutions/874-kg-resolver-candidate-list-db-fallback.md` — frontmatter format reference for new solution docs (`module`, `tags`, `problem_type`, `category`, `issue`).
- `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md` — example of a subdirectory-organized solution doc (mirrors the `agent-quirks/` subdir we are creating).

### Institutional Learnings

- `feedback_qa_provider_perf.md` (memory) — earlier evidence that Claude hallucinated on review work; the persistence-meta pattern belongs to the same family.
- `project_mika_arch_failure_modes.md` (memory) — failure-mode catalog that already enumerates persistence-meta as a known mode. This plan converts that entry from OPEN to RESOLVED-BUT-WATCH.

### External References

- mika#939 / [PR #941](https://github.com/senara-solutions/mika/pull/941) — predecessor fix (Opus deadline + skill routing) whose grooming surfaced this orthogonal hallucination.
- mika#938 grooming sessions: `245a3c48-1be5-4ee8-89fd-6711c299ede6`, `c65a98c7-b2a1-4a9d-9a98-7d9910f509f1` — failing-session evidence quoted in #947 body.

## Key Technical Decisions

- **Doc placement: new subdirectory `docs/solutions/agent-quirks/`.** The "agent quirks" framing is distinct from `architecture-patterns/`, `cross-repo-patterns/`, `workflow-issues/` — these are model/agent-behavior anomalies that are observed empirically, often without a code-level fix. Creating the subdirectory now sets up a slot for future quirks (criterion-replacement, deadline-fallback, etc.) so they cluster.
- **No code change.** The resolution is empirical. Documenting it honestly is more valuable than performing a confirmatory rerun.
- **Preserve all three hypotheses, mark none confirmed.** Future debuggers should have the full hypothesis surface, not a guess presented as conclusion.
- **Detection rule via grep, not via regex parser.** Three phrases (`"warrant persistence"`, `"captured in the response itself"`, `"session-local review artifacts"`) are distinctive enough that a literal grep covers the failure mode without false positives.
- **Memory file lives in the user's auto-memory dir, not the repo.** `project_mika_arch_failure_modes.md` is at `~/.claude/projects/.../memory/`, not in `mika/`. The doc-in-repo and memory-update are two separate write targets — both required by the ticket.

## Open Questions

### Resolved During Planning

- *Should we run a controlled rerun to identify which contributor (model rotation vs. Anthropic updates) drove the resolution?* No — cost > value for a quiet bug. The doc records both candidates honestly.
- *Should the doc audit the skill prompts for memory-tool-adjacent vocabulary as the original ticket's "Direction" section suggested?* No — that direction was a fix path, not a knowledge-capture requirement. The bug closed without it. The hypothesis stays preserved as future-debugging surface.

### Deferred to Implementation

- *Final wording of the doc's "Resolution" section* — the implementer should review the issue body's three hypotheses in their original phrasing and reproduce them faithfully without paraphrasing away nuance.

## Output Structure

    docs/
    └── solutions/
        └── agent-quirks/                                                      # new subdirectory
            └── mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md  # new file

## Implementation Units

- [ ] **Unit 1: Write the compound knowledge doc**

**Goal:** Produce a single well-structured markdown doc at `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md` that captures the pattern, hypotheses, resolution, detection rule, and general lesson.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Create: `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md`

**Approach:**
- Use the `docs/solutions/` frontmatter convention (`module`, `tags`, `problem_type`, `category`, `issue`). Suggested values:
  - `module: mika-arch`
  - `tags: [agent-quirks, hallucination, sonnet-4-6, persistence-meta, mika-arch, review-skills]`
  - `problem_type: model-behavior` (new value, distinct from `logic-error` / `data-integrity` etc.)
  - `category: agent-quirks`
  - `issue: 947`
  - `status: resolved-but-watch` (new field — operationally signals "no active investigation, but flag if recurrence")
  - `resolved: 2026-05-02`
  - `verified_clean_window: 2026-05-02..2026-05-05 (~550 mika-arch assistant messages)`
- Sections:
  1. **The pattern** — verbatim quotes from `245a3c48-…` turn 2 and `c65a98c7-…` turn 2 (text in #947 body). Frame as the "looks delivered, isn't" anti-pattern: the assistant produces well-formed meta-prose that *describes* a deliverable while emitting zero substance.
  2. **Three orthogonal hypotheses** — reproduce the three from #947 body (skill-prompt vocabulary triggers memory-tool conditioning; Sonnet 4.6 generic training pattern; orchestration-shell-to-skill context handoff via the kimi-k2.5 default). Mark all unconfirmed.
  3. **Empirical resolution** — disappeared 2026-05-02. Likely contributors: per-skill `llm_overrides` rotation in `crates/mika-agent/src/well_known_agents.rs` (groom-ticket → Opus 4.7, second-review → Sonnet 4.6) plus Anthropic side-of-line updates over the 3-day window. Cannot pin to a single cause without a controlled rerun. Note this honestly — do not present a guess as conclusion.
  4. **Detection rule for future reviewers** — when auditing mika-arch reviewer output, grep for `warrant persistence` | `captured in the response itself` | `session-local review artifacts`. Hit = treat the whole response as a zero-finding signal (deadline-fallback or hallucination), not a real review. Suggested grep recipe inline.
  5. **The general lesson** — model-baked conditioning can emit convincing meta-output that describes a deliverable instead of producing one. Watch for `"I would persist X"` vs `"X is …"`. File family with `feedback_qa_provider_perf.md` (Claude hallucinated on review work) and `project_mika_arch_failure_modes.md` (failure-mode catalog).
- Cross-link: link to mika#939 / PR #941 (predecessor fix) and to the failing-session UUIDs.

**Patterns to follow:**
- Frontmatter and section structure: `docs/solutions/874-kg-resolver-candidate-list-db-fallback.md` (full frontmatter + Problem / Root Cause / Solution structure).
- Subdirectory placement: `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md` (precedent for category-scoped subdirectories).

**Test scenarios:**
- Test expectation: none — pure documentation, no behavioral change to verify in code.
- Manual content check: `grep -E "warrant persistence|captured in the response itself|session-local review artifacts" docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md` should return all three phrases (the detection rule must be verbatim, not paraphrased, so future grep finds them).
- Manual frontmatter check: doc parses as valid YAML frontmatter (single `---` open, single `---` close, no tab-indented values).

**Verification:**
- File exists at the specified path.
- Frontmatter contains `issue: 947` and `status: resolved-but-watch`.
- All three detection-rule phrases appear verbatim in the body.
- Both failing-session UUIDs (`245a3c48-1be5-4ee8-89fd-6711c299ede6`, `c65a98c7-b2a1-4a9d-9a98-7d9910f509f1`) appear in the body.
- All three hypotheses from the #947 body are present and explicitly marked unconfirmed.
- File path uses lowercase-hyphenated naming consistent with `docs/solutions/` convention.

- [ ] **Unit 2: Update the agent-level failure-mode memory note**

**Goal:** Mark the "persistence-meta" failure mode as RESOLVED-BUT-WATCH in the user's auto-memory `project_mika_arch_failure_modes.md`, with a back-reference to the doc from Unit 1.

**Requirements:** R4

**Dependencies:** Unit 1 (doc must exist for the back-reference path to be valid)

**Files:**
- Modify: `~/.claude/projects/-data-workspace-mika-platform/memory/project_mika_arch_failure_modes.md` (read first to confirm exact path and current entry)
  - Note: the user's command body referenced a `-mika` suffix path, but the auto-memory loader at the top of this conversation references `-data-workspace-mika-platform-mika`. The implementer should `ls` both candidate paths and write to whichever exists; if both exist, prefer the one the auto-memory loader actually reads (the loader path takes precedence).

**Approach:**
- Locate the existing `persistence-meta` entry (the original ticket implies it's enumerated alongside `criterion-replacement`, `deadline-timeout`, `contract-fabrication`).
- Change its status to `RESOLVED-BUT-WATCH (2026-05-02)`.
- Add a one-line back-reference: `See: mika/docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md`.
- Preserve all other failure-mode entries unchanged.
- If the file doesn't exist yet (no prior catalog entry), create it with a single `persistence-meta` entry following the user's auto-memory frontmatter convention (see `MEMORY.md` and other memory files in the same directory for the shape).

**Patterns to follow:**
- Existing memory file conventions in the user's auto-memory directory (frontmatter with `name`, `description`, `type`).

**Test scenarios:**
- Test expectation: none — memory file is a flat-file knowledge store, no behavioral test surface.
- Manual check: `grep -i "persistence-meta\|RESOLVED-BUT-WATCH" <memory-path>` returns the updated line.

**Verification:**
- File contains the `RESOLVED-BUT-WATCH` marker on the persistence-meta entry.
- Back-reference path matches Unit 1's actual output path.
- No other entries in the file were altered (diff should show only the persistence-meta block changes).

- [ ] **Unit 3: Close issue #947 with the closing comment**

**Goal:** Close GitHub issue `senara-solutions/mika#947` with a comment that links to the doc from Unit 1 and notes the 5-day verified-clean window.

**Requirements:** R3

**Dependencies:** Units 1 and 2 (doc and memory must exist before closing). Issue closure happens **after** PR merge, not during this branch's work — the comment-and-close runs as a follow-up step in the pipeline (handler step 8 area), not inside the doc PR itself.

**Files:**
- No file changes. Action via `gh issue close 947 --repo senara-solutions/mika --comment "<text>"`.

**Approach:**
- Comment text (paraphrasable, must include): "Resolved organically — knowledge preserved at `docs/solutions/agent-quirks/mika-arch-persistence-meta-hallucination-2026-05-02-resolved.md`. Persistence-meta pattern not reproduced in 5-day window since 2026-05-02 across ~550 mika-arch assistant messages."
- Link the closing PR (#947 doc PR) in the comment as well so the issue→PR trail is intact.

**Patterns to follow:**
- Standard mika issue-closure comment style — link to the artifact, state the verification basis, no prose padding.

**Test scenarios:**
- Test expectation: none — GitHub action.
- Manual check: `gh issue view 947 --repo senara-solutions/mika` shows state `CLOSED` and the new comment.

**Verification:**
- Issue state is `CLOSED`.
- Comment includes the doc path and the 5-day clean-window note.
- Closing reason references the PR or the doc commit.

## System-Wide Impact

- **Interaction graph:** None — pure documentation. Does not change any code path, runtime behavior, schema, or interface.
- **Error propagation:** N/A.
- **State lifecycle risks:** None.
- **API surface parity:** None.
- **Integration coverage:** None — no behavior to test.
- **Unchanged invariants:** mika-arch's per-skill `llm_overrides` in `crates/mika-agent/src/well_known_agents.rs` are *intentionally* unchanged. The current rotation is one of the suspected resolution contributors; we leave it alone and document that hypothesis.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Pattern recurs after we close the ticket and call it resolved. | Detection rule (Unit 1) gives reviewers a greppable trip-wire. Memory entry stays RESOLVED-BUT-WATCH (not RESOLVED-CLOSED) to keep it on the operator's mental radar. If recurrence: re-open #947 and reference the doc as the prior pattern record. |
| Doc paraphrases the original session quotes and loses fidelity. | Verification step requires verbatim phrase matching for the three detection-rule phrases. Implementer must copy from the #947 body, not summarize. |
| Memory file path is wrong (the user's command body and the auto-memory loader reference different paths). | Unit 2 explicitly instructs `ls`-ing both candidate paths first; loader path wins. |
| Future maintainer treats RESOLVED-BUT-WATCH as RESOLVED-CLOSED and stops watching. | Back-reference from memory to the in-repo doc preserves the trail. The doc itself opens with "preserved as future-debugging knowledge" framing. |

## Documentation / Operational Notes

- No `mika-doc-audit` follow-ups expected — this *is* the doc, and it doesn't change CLAUDE.md or any user-facing docs surface.
- No `compound` follow-up needed — this *is* the compound output for the original investigation. The pipeline's `/ce:compound` step at handler step 6 should be a no-op or a thin "recorded as solution doc" pointer.

## Sources & References

- Origin ticket: [senara-solutions/mika#947](https://github.com/senara-solutions/mika/issues/947) — full body has the verbatim session quotes and three hypotheses.
- Predecessor fix: mika#939 / [PR #941](https://github.com/senara-solutions/mika/pull/941) — surfaced the persistence-meta pattern during its own grooming.
- Related code: `crates/mika-agent/src/well_known_agents.rs` (per-skill `llm_overrides` for mika-arch).
- Related solution docs (frontmatter precedent): `docs/solutions/874-kg-resolver-candidate-list-db-fallback.md`, `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md`.
- Related memory: `feedback_qa_provider_perf.md`, `project_mika_arch_failure_modes.md` (auto-memory dir).

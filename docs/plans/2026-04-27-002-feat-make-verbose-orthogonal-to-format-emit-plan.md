---
title: "Make `mika ask --verbose` orthogonal to `--format` (emit metadata in JSON)"
type: feat
status: active
date: 2026-04-27
issue: senara-solutions/mika#829
---

# Make `mika ask --verbose` orthogonal to `--format` (emit metadata in JSON)

## Overview

mika#824 (merged 2026-04-26) added `mika ask --verbose` to emit a `session_id: <uuid>` trailer for cross-command integration. The PR scoped the implementation to text mode only — *"JSON mode unchanged"* — leaving `mika ask --verbose --format json` silently emitting no metadata. The downstream consumer `/mika-ask-arch` in mika-platform had to fall back to a `sqlite3` query of `~/.mika/data/mika.db` to recover the session_id, and mika-platform#54 had to drop `--format json` from the slash command and parse the text-mode trailer instead.

This plan fixes the orthogonality violation. `--verbose`'s semantics will be the same regardless of `--format`'s value — text mode keeps its trailer; JSON mode gains a nested `metadata` envelope.

## Problem Frame

The structural framing is **not** "JSON mode is missing a feature." It is *"a flag's semantics depended on another flag's value"* — a CLI-grammar violation of orthogonality (cite: `docs/architecture/review-guide.md` § Orthogonality).

Concrete repro pre-fix on this host:

```
$ mika ask --agent mika-test --verbose --format json ping
{"role":"assistant","content":"Here."}
```

No `session_id`. No metadata. The `if verbose { ... }` block at `crates/mika-cli/src/commands/ask.rs:373-379` lives only inside the `OutputFormat::Text` arm; the `OutputFormat::Json` arm (lines 380-388) doesn't even read the `verbose` flag.

The crate's CLAUDE.md documents the broken contract: *"`--verbose` ... Does not affect `--format json` output."* That sentence is the orthogonality violation written down and accepted at merge time.

## Requirements Trace

- **R1.** `mika ask --verbose --format json` emits a JSON envelope containing the session_id at a stable location.
- **R2.** `mika ask --format json` (no `--verbose`) emits **byte-identical output to today** — no breaking change for existing JSON consumers.
- **R3.** The text-mode `--verbose` trailer (`session_id: <uuid>` after a blank line) is unchanged in shape and parsing contract.
- **R4.** The new envelope is structured for **per-field gating**: future fields may ship `--verbose`-gated or unconditional without requiring a semantics revisit.
- **R5.** CLAUDE.md no longer claims `--verbose` is text-mode-only; documents the new envelope and per-field-gating semantics.
- **R6.** Lesson is compounded so future flag PRs cite the orthogonality principle by name.

## Scope Boundaries

- **In scope:** `crates/mika-cli/src/commands/ask.rs` (envelope + JSON arm + tests), `crates/mika-cli/CLAUDE.md` (contract docs), one compound doc.
- **Out of scope:** any field other than `session_id` in the envelope. `trace_id`, `agent_id`, `model`, `latency_ms` are obvious next-fields and are explicitly **deferred** — adding them now is the same Unit-1-encoding-Unit-N trap recently compounded in `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md`. The envelope is extensible; new fields land when there's a consumer.
- **Out of scope (deferred to separate PRs):** the mika-platform consumer update (parse `metadata.session_id` in `/mika-ask-arch` JSON path, drop the sqlite fallback). Decoupling rationale below.

### Deferred to Separate Tasks

- **mika-platform consumer update.** A separate mika-platform PR (companion to mika-platform#54) will consume `metadata.session_id` and drop the sqlite fallback in `/mika-ask-arch`. Decoupled because: (a) the CLI PR is verifiable in isolation via unit tests on the envelope shape, while the consumer PR is verifiable only after this PR's binary is deployed on the host running the slash command — coupling them creates a synchronization problem with no win; (b) cross-repo PRs muddy reviewer ownership.

## Context & Research

### Relevant Code and Patterns

- **Envelope** — `crates/mika-cli/src/commands/ask.rs:15-23`. Existing `AskJsonResponse` has `role`, `content`, `task_id`, `pending_tasks`. Already uses `#[serde(skip_serializing_if = "Option::is_none")]` and `skip_serializing_if = "Vec::is_empty"` for backward-compat. The new `metadata` field follows the same pattern.
- **Format match** — `crates/mika-cli/src/commands/ask.rs:360-389`. `OutputFormat::Text` reads `verbose`; `OutputFormat::Json` does not.
- **Existing test pattern** — `test_verbose_trailer_format` at `:681-694` asserts the text-mode contract (key-name match, valid UUID round-trip). The new JSON tests mirror its shape.
- **mika#824 PR** (merged) — original `--verbose` add. Body: *"Text mode only — JSON mode unchanged. Conflicts with `--team`."*
- **mika-platform#54** — slash-command consumer update that grew a sqlite fallback because of mika#824's gap; will be revisited after this PR ships.

### Institutional Learnings

- `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — same-session compound. Sibling lesson: when human discipline keeps failing for a class, replace with a structural check. Applied here at *review* discipline: when "scope discipline" repeatedly produces orthogonality violations, build orthogonality into the review checklist rather than hardening discipline further.
- `docs/architecture/review-guide.md` § Orthogonality — the principle this plan derives from.

### External References

- The peer-review brief (`/mika-ask-a-friend` → friend Claude) explicitly converged with the operator on: nested envelope (not flat), decoupled consumer PR, separate ticket from mika#828, per-field gating semantics, and PR description framing of "flag semantics must not depend on another flag's value." This plan implements that consensus.

## Key Technical Decisions

- **Nested `metadata` envelope, not flat top-level fields.** `session_id` is unambiguously runtime metadata, not part of the assistant's message. Putting it at the top level alongside `role`/`content` would worsen the existing smell where `task_id` and `pending_tasks` already pollute the message shape. Nested mirrors the text-mode trailer's conceptual separation (LLM content vs runtime metadata) and scales — `trace_id`/`agent_id`/`model`/`latency_ms` get a home that doesn't require another envelope rewrite. Anthropic-API-shape mimicry (top-level fields) was rejected as a red herring: their API is a protocol response; this is a CLI envelope that already distinguishes message from runtime concerns.
- **Per-field gating, not blanket `--verbose`-gating.** The CLAUDE.md framing is *"metadata fields *may* be `--verbose`-gated"*, not *"metadata is `--verbose`-gated."* Today only `session_id` exists and is gated; future ops fields (e.g., `trace_id`) may ship unconditional. The envelope is omitted only when **all** its fields are absent (per `Option::is_none` on the parent `metadata: Option<MetadataEnvelope>`). One sentence in CLAUDE.md saves a future refactor argument.
- **Backward compatibility via `serde(skip_serializing_if = "Option::is_none")`.** When `--verbose` is unset, `metadata: None` is skipped on serialization — output is byte-identical to pre-#829. Verified by an explicit test asserting the exact pre-#829 string.
- **Ship `session_id` only.** Other obvious metadata fields (`trace_id`, `agent_id`, `model`, `latency_ms`) are explicitly out of scope. Adding them now is the same Unit-1-encoding-Unit-N trap compounded earlier today. The envelope shape is extensible; new fields land when there's a consumer.
- **Decouple consumer update.** The mika-platform `/mika-ask-arch` update parses `metadata.session_id` in JSON path and drops sqlite fallback. Ships separately, after this PR merges and deploys. Coupling cross-repo PRs creates a synchronization problem with no win, and mika-platform#54 already exists with partially-overlapping scope.
- **Independent of mika#828.** #828 is a deploy bootstrap fix; this is a CLI contract fix. Different blast radius, different test surface, different rollback story. Bundling means a regression in either reverts both.

## Open Questions

### Resolved During Planning

- **Flat vs nested envelope?** Nested (see Key Technical Decisions). Friend-review consensus.
- **Couple or decouple consumer update?** Decouple. Friend-review consensus.
- **Is metadata always `--verbose`-gated?** No — per-field gating. Doc framing reflects this so future unconditional fields don't require a semantics revisit.
- **Slipstream into mika#828 or separate ticket?** Separate. Different blast radius.

### Deferred to Implementation

- None — this work was small enough to be fully resolved at planning time.

## Implementation Units

- [x] **Unit 1: Add `MetadataEnvelope` struct + `metadata` field on `AskJsonResponse`**

**Goal:** Introduce the envelope type and wire it into the JSON response shape without altering existing behavior.

**Requirements:** R2, R4.

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs`

**Approach:**
- Define `MetadataEnvelope { session_id: Option<String> }` with `#[serde(skip_serializing_if = "Option::is_none")]` on each field.
- Add `metadata: Option<MetadataEnvelope>` to `AskJsonResponse`, also with `skip_serializing_if = "Option::is_none"`.
- Doc comments on the envelope spell out per-field gating.

**Test scenarios:**
- *Happy path:* envelope serializes to `{}` when all fields are None.
- *Edge case:* envelope with `session_id: Some(uuid)` round-trips; key-name match.
- *Backward compat:* `AskJsonResponse` with `metadata: None` produces byte-identical pre-#829 output.

- [x] **Unit 2: Wire `--verbose` into the `OutputFormat::Json` arm**

**Goal:** Make `--verbose`'s effect symmetric across format arms.

**Requirements:** R1, R2, R3.

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs` (the `OutputFormat::Json` arm).

**Approach:**
- Inside `OutputFormat::Json`, build `metadata = if verbose { Some(MetadataEnvelope { session_id: Some(session_id.clone()) }) } else { None }`.
- Pass `metadata` to `AskJsonResponse`. The text-mode arm is untouched.

**Test scenarios:**
- *Happy path:* `--verbose --format json` → JSON contains nested `metadata.session_id` with valid UUID.
- *Backward compat:* `--format json` (no `--verbose`) → JSON contains no `metadata` key at all (key omitted, not `"metadata":null`).
- *Top-level pollution check:* under `--verbose --format json`, `session_id` does NOT appear at the top level (only inside `metadata`).

**Verification:**
- New unit test `test_json_response_includes_metadata_session_id_when_verbose` asserts the nested shape and absent-top-level invariant.
- New unit test `test_json_response_omits_metadata_when_none` asserts byte-identical-no-verbose output.
- New unit test `test_metadata_envelope_omits_none_session_id` asserts per-field gating works (envelope serializes to `{}` when fields are absent).
- Live smoke: built debug binary, ran all three modes (`--verbose --format json`, `--format json`, `--verbose` text), each behaves per spec.

- [x] **Unit 3: Update CLAUDE.md to document the new contract**

**Goal:** The crate's CLAUDE.md must no longer claim `--verbose` is text-mode-only.

**Requirements:** R5.

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md`

**Approach:**
- Strike the *"Does not affect `--format json` output"* sentence.
- Replace with a description of the JSON envelope, the cross-mode parsing contract, and the per-field-gating semantics.
- Reference mika#824 → mika#829 lineage so future readers see the orthogonality fix history.

**Test scenarios:**
- *Test expectation: none — documentation deliverable.*

- [x] **Unit 4: Compound the lesson**

**Goal:** Future flag PRs cite the orthogonality principle by name; reviewers challenge "scoped to format X only" carve-outs at review time.

**Requirements:** R6.

**Files:**
- Create: `docs/solutions/best-practices/flag-semantics-must-not-depend-on-other-flags-2026-04-27.md`

**Approach:**
- Frontmatter per `docs/solutions/` style (`module`, `tags`, `problem_type`, `component`, `severity`).
- Sections: *Context* (what mika#824 shipped, what mika#829 fixed), *Guidance* (the principle), *Why This Matters* (failure modes of entangled flags), *When to Apply* (review-time signals), *Examples* (anti-pattern + pattern + per-field-gating), *Related* (links to mika#824, mika#829, mika-platform#54, review-guide.md, sibling structural-check compound).
- Tone matches `docs/solutions/best-practices/kg-provider-eval-harness-reproducible-comparison-2026-04-24.md` and the same-session structural-check compound.

## System-Wide Impact

- **API surface parity:** `mika ask --help` should accurately describe the cross-format flag (verified manually).
- **Backward compatibility:** byte-identical output in pre-#829 invocations (no `--verbose`). Verified by explicit test asserting the exact string.
- **Downstream consumers:** mika-platform's `/mika-ask-arch` will eventually parse `metadata.session_id` from JSON and drop its sqlite fallback (separate PR, not this one). Until that follow-up ships, the slash command continues to work via the text-mode trailer path.
- **Unchanged invariants:** the text-mode trailer wire format (blank line + `session_id: <uuid>`); the `AskJsonResponse` field set absent the new optional `metadata`; `--verbose` conflicts with `--team` (clap-enforced); 100KB result limit.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing JSON consumers break on the new field | `Option::is_none` + `skip_serializing_if` keeps no-`--verbose` output byte-identical. Explicit unit test asserts the pre-#829 byte string. |
| Future contributor adds `trace_id` and forces it into the `--verbose`-gated path | CLAUDE.md framing is *"metadata fields *may* be `--verbose`-gated"*, not blanket-gated. Compound doc reinforces. |
| Consumer PR (mika-platform side) drifts in shape | Decoupling is intentional. Consumer references this PR's commit when it ships; CLAUDE.md is the authoritative shape doc. |
| Reviewer asks "why nested, why not flat?" | PR description and Key Technical Decisions section both spell out the rejection of flat top-level (Anthropic-API mimicry as a red herring). |

## Documentation / Operational Notes

- **PR description's structural lesson is load-bearing.** The framing "*a flag's semantics must not depend on another flag's value*" is the citation handle for future flag PRs. Reviewers should challenge "scoped to format X only" carve-outs at review time, not at consumer-bite time.
- **Post-merge:** deploy via `make deploy`; redo three smoke tests (`--verbose --format json`, `--format json` alone, `--verbose` text mode) against `~/.local/bin/mika`.
- **Follow-up:** file mika-platform PR consuming `metadata.session_id` in `/mika-ask-arch` JSON path and removing the sqlite fallback. Reference this PR.

## Sources & References

- **Origin issue:** [senara-solutions/mika#829](https://github.com/senara-solutions/mika/issues/829)
- **Original `--verbose` PR (introduced the gap):** [senara-solutions/mika#824](https://github.com/senara-solutions/mika/pull/824)
- **Code locations:** `crates/mika-cli/src/commands/ask.rs:15-23` (envelope), `:360-389` (format match)
- **Companion (mika-platform side, deferred):** [senara-solutions/mika-platform#54](https://github.com/senara-solutions/mika-platform/pull/54) and its successor
- **Architecture reference:** `docs/architecture/review-guide.md` § Orthogonality
- **Sibling compound (same session):** `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md`

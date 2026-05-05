---
module: mika-arch
tags: [agent-quirks, hallucination, sonnet-4-6, persistence-meta, mika-arch, review-skills, llm-conditioning]
problem_type: model-behavior
category: agent-quirks
issue: 947
status: resolved-but-watch
resolved: 2026-05-02
verified_clean_window: 2026-05-02..2026-05-05 (~550 mika-arch assistant messages)
---

# mika-arch persistence-meta hallucination (Sonnet 4.6 review skills)

> **Status: RESOLVED-BUT-WATCH.** The pattern stopped reproducing on 2026-05-02 and has not
> recurred across mika-arch's ~550 assistant messages in the 5-day verification window
> (2026-05-02..2026-05-05). No code change was shipped. The contributors that *likely* drove the
> resolution (per-skill `llm_overrides` rotation + Anthropic side-of-line model updates) are noted
> below but cannot be pinned to a single cause without a controlled rerun. This doc preserves the
> pattern, hypotheses, detection rule, and lesson so future debuggers have a starting point if the
> pattern recurs.

## The pattern — "looks delivered, isn't"

mika-arch (running Sonnet 4.6 via per-skill `llm_overrides` defined in
`crates/mika-agent/src/well_known_agents.rs`) emitted convincing but vacuous *meta-prose* on
review prompts when invoked through the `mika-arch-groom-ticket` and `mika-arch-second-review`
skill envelopes. The response is well-formed — fluent prose, plausible references to "findings"
or "verdicts" — but the response *describes* a deliverable instead of *producing* one.

Two failing sessions on 2026-05-02 during mika#938 grooming are the canonical instances:

**Session `245a3c48-1be5-4ee8-89fd-6711c299ede6` turn 2** —

> *"No new facts warrant persistence from this turn. The mika#938 first-pass review findings (F1-F4) are captured in the response itself..."*

The model claimed F1–F4 findings exist; the first response on this session had **zero findings**
— it was an Opus 4.7 `deadline-exceeded` fallback `"I'm sorry, that took too long."`. There were
no findings to summarize.

**Session `c65a98c7-b2a1-4a9d-9a98-7d9910f509f1` turn 2** —

> *"No new facts warrant persistence from this turn. The mika#938 second-pass GROOMED verdict and its findings (F1–F5) are session-local review artifacts..."*

The model claimed a `GROOMED` verdict with F1–F5 findings was emitted; the verdict was actually
emitted **only after both skills were disabled and a third turn was issued**. Turn 2 in this
session had no verdict and no findings — only the meta-prose about them.

Disabling both skills (`mika-arch-groom-ticket` and `mika-arch-second-review`) eliminated the
pattern. The same model (Sonnet 4.6) delivered real verdicts when invoked via bare `mika ask`
without the skill envelope. Audit at `/mika-audit session c65a98c7-b2a1-4a9d-9a98-7d9910f509f1`
confirmed the malformed turn.

The structural signature: turn 1 may emit a real review *or* the persistence-meta. Turn 2 (or
subsequent turns) reliably emit the persistence-meta when both skills were enabled.

## Three orthogonal hypotheses (all unconfirmed)

mika-arch's `~/.mika/agents/mika-arch/config.toml` does **not** enable any memory skill — so the
persistence-meta language is not the result of an active memory tool. It is model-baked
conditioning that surfaces when the skill envelope's system prompt creates a context resembling a
memory-tool conversation. We preserve three orthogonal hypotheses; none has been confirmed.

### Hypothesis 1 — Skill system prompt vocabulary triggers memory-tool conditioning

The skill envelopes' system prompts may contain words/phrases (`session-local`, `context`,
`store_fact`, `persistence`) that activate the model's memory-tool conditioning. If true, targeted
vocabulary edits to `skills/bundled/mika-arch-groom-ticket/system_prompt.md` and
`skills/bundled/mika-arch-second-review/system_prompt.md` would resolve the pattern. Not audited
during the original investigation; not audited as part of this resolved-but-watch doc.

### Hypothesis 2 — Sonnet 4.6 generic training conditioning

Generic training pattern where dense structured-review prompts trigger the memory-meta shape
regardless of vocabulary. If true, the fix surface is at the invocation level (different prompt
structure — e.g., explicit `<review>...</review>` tag wrapping) or at the model-routing level (swap
to a different model for review work, per `feedback_qa_provider_perf.md`'s precedent of moving qa
to DeepSeek when Claude hallucinated).

### Hypothesis 3 — Orchestration-shell-to-skill context handoff

`~/.mika/agents/mika-arch/config.toml` defaults to `openrouter_model = "moonshotai/kimi-k2.5"` for
the orchestration shell, with per-skill Anthropic overrides via `llm_overrides`. Some
orchestration-shell-to-skill context handoff may inject memory-skill-adjacent state into the
Anthropic-routed skill turn. If true, the fix is config-level or orchestration-loop-level — not
in the skill prompts themselves.

## Empirical resolution (uncontrolled)

The pattern stopped reproducing on **2026-05-02** and has not recurred in the 5-day window since.
Verification: zero occurrences of the persistence-meta phrases across mika-arch's ~550 assistant
messages from 2026-05-02 through 2026-05-05.

Two likely contributors, neither isolated:

1. **Per-skill `llm_overrides` rotation in `crates/mika-agent/src/well_known_agents.rs`.** The
   mika-arch reviewer skills now run with a mixed model assignment — Opus 4.7 for
   `mika-arch-groom-ticket` and Sonnet 4.6 for `mika-arch-second-review`. The original failing
   sessions on 2026-05-02 were both running Sonnet 4.6 on the implicated skill turns. Rotating
   half the reviewer surface off Sonnet 4.6 reduces the exposure window even if Hypothesis 2 is
   the true root cause, and could mask Hypothesis 1 entirely if vocabulary-triggered conditioning
   is Sonnet-specific.

2. **Anthropic side-of-line model updates** over the same window. Anthropic updates Sonnet 4.6's
   serving behavior continuously; the conditioning that produced persistence-meta on 2026-05-02
   may have been weakened or eliminated by an update we cannot directly observe.

We **cannot pin the resolution to a single cause** without running a controlled rerun (re-enable
both skills with both originally implicated brief shapes, force-route both turns to Sonnet 4.6,
and check for recurrence). The cost of that rerun outweighs the value for a quiet bug. Both
contributors are recorded honestly here so future debuggers have the full hypothesis surface.

## Detection rule for future reviewers

When auditing mika-arch reviewer output for hallucinated content, grep the assistant message body
for any of these three phrases — they are the **distinctive structural signature** of the
persistence-meta failure mode and rarely (if ever) appear in legitimate review output:

```
warrant persistence
captured in the response itself
session-local review artifacts
```

A literal grep recipe for an audit pass:

```bash
mika kg query --agent mika-arch --format json \
  | jq -r '.messages[] | select(.role == "assistant") | .content' \
  | grep -E "warrant persistence|captured in the response itself|session-local review artifacts"
```

Or against an exported session log:

```bash
grep -E "warrant persistence|captured in the response itself|session-local review artifacts" \
  /path/to/session-export.json
```

**Hit interpretation:** treat the entire assistant message as a **zero-finding signal** — either a
deadline-fallback or a hallucination, not a real review. Do not extract "findings" from the
labels-and-meta-prose; do not iterate the plan against inferred content. Surface to the skill
maintainer and fall back to operator-authored manual review (same disposition as Mode 3
contract-fabrication-of-prior-output per `project_mika_arch_failure_modes.md`).

## The general lesson

**Model-baked conditioning can emit convincing meta-output that *describes* a deliverable instead
of *producing* one.** The text reads like a competent assistant reflecting on completed work; the
work itself is absent. Watch for the linguistic shape:

- ❌ `"I would persist X"` — meta-prose describing an action the model would take
- ❌ `"the findings (F1–F4) are captured in the response itself"` — self-reference to content that doesn't exist
- ❌ `"the verdict and findings are session-local review artifacts"` — meta-categorization instead of the verdict
- ✅ `"X is …"` — direct delivery of the actual content
- ✅ `"Disposition: GROOMED. F1: <body>. F2: <body>. …"` — structured findings with bodies, not labels

This failure mode lives in the same family as:

- `feedback_qa_provider_perf.md` (memory) — Claude hallucinated on review work; mika-qa moved to
  DeepSeek as a result. Same root family: review-prompt conditioning produces well-formed but
  empty output.
- `project_mika_arch_failure_modes.md` (memory) — failure-mode catalog. Persistence-meta is
  enumerated there as Mode 4 (RESOLVED-BUT-WATCH after this doc landed). Distinct from Mode 3
  (contract-fabrication-of-prior-output) at the surface — Mode 3 references *prior outputs* in
  the same session that don't exist; persistence-meta references *current-turn outputs* that
  don't exist — but structurally adjacent: both produce well-formed responses that summarize
  content the model never emitted.

The defensive posture is the same across this family: **separate "model talks about doing X" from
"model did X"**. Unit-test reviewers should reject the former even when the prose is fluent.

## References

- **Origin ticket:** [senara-solutions/mika#947](https://github.com/senara-solutions/mika/issues/947) — full body has the verbatim session quotes and three hypotheses.
- **Predecessor fix:** mika#939 / [PR #941](https://github.com/senara-solutions/mika/pull/941) — Opus deadline + skill routing fix whose grooming surfaced this orthogonal hallucination.
- **Failing sessions** (mika#938 grooming, 2026-05-02):
  - `245a3c48-1be5-4ee8-89fd-6711c299ede6` (skill enabled; contained the Opus deadline-exceeded fallback)
  - `c65a98c7-b2a1-4a9d-9a98-7d9910f509f1` (post-skill-disable retry path; verdict only on a later turn after both skills disabled)
- **Code references:**
  - `crates/mika-agent/src/well_known_agents.rs` — mika-arch's per-skill `llm_overrides` (current rotation: Opus 4.7 for `mika-arch-groom-ticket`, Sonnet 4.6 for `mika-arch-second-review`).
  - `skills/bundled/mika-arch-groom-ticket/system_prompt.md` — skill prompt for first-pass review (not audited; vocabulary hypothesis target if recurrence).
  - `skills/bundled/mika-arch-second-review/system_prompt.md` — skill prompt for second-pass review (not audited; vocabulary hypothesis target if recurrence).
- **Related memory notes:**
  - `feedback_qa_provider_perf.md` — same family (Claude hallucinated on review work).
  - `project_mika_arch_failure_modes.md` — failure-mode catalog with this entry as Mode 4 (RESOLVED-BUT-WATCH).

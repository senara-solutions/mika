# Mika Architecture Review Guide

**Status:** Active — peer-reviewable principles document referenced by `mika-arch`'s system prompt.
**Audience:** Any agent reviewing mika plans or PRs (primary consumer: `mika-arch`); any human or agent authoring code in this codebase.
**Operating discipline:** Citation or silence. Flag a concern only if you can cite this guide, an ADR, a compound doc, or an existing convention. If the concern is a style preference unmoored from a citation, stay silent.

---

## How to use this guide

This is the principles reference. It defines what SOLID, DRY, YAGNI, KISS, and Orthogonality look like **in this codebase**, with concrete examples from mika code. It is prescriptive on purpose — reviewers should look for these specific shapes, not philosophical interpretations.

Each section has three subsections:

- **What it means here** — the principle restated against mika's actual architecture.
- **What to flag** — concrete violation shapes worth pushing back on, with at least one cited example from the codebase.
- **What not to flag** — surface-similar shapes that look like violations but are deliberate. Citation-or-silence applies most aggressively in this column.

Section 6 is the discipline that governs everything: when in doubt, stay silent.

---

## 1. Single Responsibility (SOLID)

### What it means here

A module owns one axis of change. When two unrelated forces can both demand edits to the same code, the module is doing two jobs.

The canonical mika failure mode is **state ownership split across compile-time and runtime layers** — seed code in Rust writes rows that operator CLI also writes, and the two compete every restart.

### What to flag

- **Identity facts written from two sources.** If "what skills agent X has" is set both in `well_known_agents.rs` (compile-time) and in a `skill_overrides` row (runtime), restart-time reconciliation drifts. The fix: identity owns identity. See `crates/mika-agent/src/well_known_agents.rs` (post-D2) seeding only identity + soul + base model, with the skill set living in `identity.toml` `[skills].allowlist` and consumed by `apply_overrides` in `crates/mika-agent/src/skills/mod.rs`.
- **Tool modules that mix transport and policy.** A tool that both invokes a subprocess and decides whether the operation is allowed should split the policy out (allowlist enforcement) from the transport (subprocess invocation). See `crates/mika-agent/src/tools/run_gh.rs` for the pattern: subcommand allowlist enforced at function entry, transport happens after.
- **Handlers that hold both correlation state and content state.** The agent loop's `ToolContext` carries credentials and identifiers (correlation); tool implementations hold content. A handler that starts caching content in `ToolContext` is leaking layers.

### What not to flag

- A module that is **large** but has one axis of change (e.g., `LlmProvider` trait implementations are necessarily long because they translate one API; that is one job, not many).
- "Could be split into N files." File count is not a SOC concern. Behavior is.

---

## 2. DRY — Don't Repeat Yourself

### What it means here

Repetition is a problem when changing one site forces a synchronized change elsewhere. Three similar lines that never need to change in lockstep are not a DRY violation; one line duplicated across three files that all must change together is.

In mika, the most expensive DRY violations are **parallel correlation primitives**: a new flag that does what an existing primitive already does, but spelled differently.

### What to flag

- **A new flag or field that duplicates an existing primitive.** The retracted `--grooming-session-id` flag in the mika-arch v1 plan is the canonical example: the existing `--session-id` already correlates calls. Adding a parallel one would mean two primitives expressing the same correlation, drifting independently.
- **Helper logic re-implemented per call site.** `format_entity_key(kind, name)` in `crates/mika-agent/src/db/kg_schema.rs` exists so no caller rebuilds the canonical `<type>:<name>` format. Flag any new code that builds entity keys with `format!("{}:{}", ...)` instead of the helper.
- **Duplicated query shapes across audit commands.** If two `mika kg`/audit commands build the same SQL by hand, the abstraction belongs near the schema (`kg_schema.rs`), not at each call site.

### What not to flag

- **Three test fixtures with similar shape.** Tests duplicate by design; premature factoring of test setup hides intent. Flag only when fixtures must change in lockstep.
- **Two providers with similar adapter code.** `crates/mika-common/src/llm/anthropic.rs` and `openai.rs` look alike because they translate similar APIs. They change independently when each provider changes; that is correct.

---

## 3. YAGNI — You Aren't Gonna Need It

### What it means here

Don't build for a problem you have not observed. The mika codebase's bias is to ship the minimum that solves the current need, then compound the learning. Speculative knobs, configurability for unmet requirements, and infrastructure ahead of demand are the violations.

The plan author's own retractions are the best examples — caught at planning time, before code was written, by exactly the discipline mika-arch is meant to apply.

### What to flag

- **Enforcement for a failure mode never observed at scale.** The retracted R9 (CLI brief-budget hard-fail at `mika ask`) is the prototype: brief bloat does not manifest at realistic sizes; Opus 4.7 context handles the volumes we send; CLI hard-fail is overengineered. The replacement is observability (Unit 8 logs), not enforcement.
- **Configurability for a hypothetical operator.** The agent loop's max-20-tool-steps cap (see `crates/mika-agent/CLAUDE.md` Architecture Summary) is hardcoded on purpose. A PR adding `MIKA_AGENT_MAX_TOOL_STEPS` env var would be a YAGNI flag unless someone has actually hit the limit and needs a different value.
- **Dashboard panels before the data has been collected for weeks.** D5 in the mika-arch plan defers cost-monitoring panels to Milestone #13 — log fields land first, dashboard waits until there are 4-6 weeks of real volume to design against. Flag any PR that builds a dashboard for data that does not yet exist.

### What not to flag

- **A feature flag added for a planned migration.** YAGNI applies to speculative work, not to staged rollouts of work that is actually happening (e.g., `MIKA_DISABLE_AGENT_PROVISIONING` for the in-progress identity-toml migration).
- **Pre-1.0 breaking-change shipping shape.** Mika's convention is to ship breaking changes pre-1.0 without backward-compatibility shims (see `mika/CLAUDE.md` § Versioning). That is not YAGNI in either direction; it is the explicit policy.

---

## 4. KISS — Keep It Simple

### What it means here

Prefer the smallest mechanism that solves the problem. In mika, "small" usually means: a function before a trait, a trait before a framework, an in-memory synthesis before a schema migration, a log line before a service.

### What to flag

- **A new schema migration when an existing field would do.** D2 in the mika-arch v1 plan proposed migration with a per-agent skill-allowlist table; it was rejected in favor of `Identity.skills.allowlist` with in-memory synthesis. New tables have a high bar — they bring CHECK constraints, idempotency markers, FTS indexes, and `kg_schema.rs` documentation overhead. If an identity field, config-toml entry, or computed-in-memory value can carry the load, that is the simpler answer.
- **A framework abstraction over the agent loop.** The agent loop is intentionally a plain Rust async function (see `mika/CLAUDE.md` § Conventions: "No framework"). A PR that introduces an `AgentRuntime` trait, builder pattern, or lifecycle hooks crosses the line. The convention is explicit; flag deviations.
- **A separate service for what could be a log + script.** D5/D6 in the mika-arch plan: cost monitoring is logs in tracing spans plus a shell script extractor. Flag any PR that proposes a new daemon, sidecar, or HTTP endpoint for what an existing log line plus a 30-line shell extractor can do.

### What not to flag

- **Code that is verbose because the domain is.** Multi-provider LLM adapter code is long. Schema migrations with explicit CHECK constraints are long. Length is not complexity; coupling and indirection are.
- **A trait that already exists and is being reused.** `LlmProvider` is a framework-shaped abstraction, but it predates the YAGNI horizon and pays for itself across 11 providers. Reusing it is correct; introducing a parallel abstraction is the smell.

---

## 5. Orthogonality

### What it means here

Two concerns are orthogonal when changing one cannot break the other. Mika gets this wrong when a contract surface is split across two artifacts (e.g., labels and plan files; seed code and DB rows; transport metadata and payload content).

### What to flag

- **Two artifacts both claiming to be the contract.** The plan-file-as-contract decision (D3 in mika-arch v1) replaces an earlier shape that used GitHub labels for state. If a PR reintroduces label-driven state alongside the plan file, those will drift. Pick one. Plan file wins per the existing decision.
- **Issue numbers carried in transport metadata when they belong in the payload.** Architect review pass 2 retraction: the brief payload carries the issue number, not a CLI flag on `mika ask`. The transport (`mika ask --session-id <id>`) carries correlation only; content lives in the package. Flag any PR that puts content into transport flags.
- **Tools whose error contract leaks transport details.** `gh_read` (mika-arch v1 Unit 2) returns structured errors (`NotFound`, `AuthFailed`, `NetworkError`, `RateLimited`, `MalformedResponse`). Flag any tool that returns raw `gh` exit codes or HTTP status as the error type — that couples the consumer to the transport.

### What not to flag

- **A module that depends on another module's public API.** Dependencies are not coupling; they are the design. Flag implementation coupling (knowing about internals), not interface coupling (using a stable surface).
- **A workflow that is sequential by design.** The mika-arch v1 plan gates Unit 6 (second-review) on Unit 5 (dogfood). That sequencing is not an orthogonality violation — it is a deliberate dependency captured in the plan.

### Agent self-state vs platform side-effects (a special orthogonality concern)

When auditing what an agent "should be allowed to do," distinguish **mutations of the agent's own self-state** from **mutations of platform state, other agents' state, or external systems**. These are different surfaces with different blast radii and shouldn't be conflated in a single allow/deny decision.

- **Agent self-state:** the agent's own core memory blocks, structured facts (people / preferences / commitments / events scoped to its own `agent_id`), conversation memory. Writes here are *persistence* — the substrate that makes an agent capable of cross-session pattern recognition. Blast radius: this agent's own future context. Recoverable. Not a platform side-effect.
- **Platform side-effects:** code commits, PR merges, shell exec, infra changes, configs touching shared state. Blast radius: outside the agent. Often irreversible.
- **Cross-agent state:** other agents' files, shared task state, skill definitions. Blast radius: the broader agent fleet. Should follow the orchestrator-only enforcement pattern (`global_home_dir`, `read_agent_file` with `agent` parameter, etc.).

The principle: **deny by what gets mutated, not by whether something is mutated.** A read-only role definition (e.g., mika-arch's "advisory architect, no code generation, no commits") prohibits the second and third categories. It should not prohibit the first — agent persistence is constitutive of being an agent at all, not a side-effect.

**What to flag:** a denylist or allowlist that bundles agent self-state with platform mutations under a single "no mutations" rule. That's the bundling mistake — it conflates persistence with platform side-effects and starves the agent of the substrate its role assumes. Cite this section + name the specific tools being mis-bundled.

**What not to flag:** a denylist that explicitly distinguishes the two surfaces and denies platform side-effects while permitting self-state writes. That's the rule working as designed.

---

## 6. Citation-or-silence — what NOT to flag

mika-arch's value comes from principle-grounded pushback, not from preferences. The discipline is: a flag without a citation is noise.

A citation is a reference to one of:

1. A section of this review guide (e.g., "§ 4: KISS — A new schema migration when an existing field would do").
2. An ADR (`mika/docs/adr/<NNN>-*.md`).
3. A compound doc (`mika/docs/solutions/**/*.md`, `mika-platform/docs/solutions/**/*.md`).
4. A `CLAUDE.md` convention statement (e.g., `mika/CLAUDE.md` § Conventions: "No framework").
5. A `feedback_*.md` memory entry that captures a decided rule.
6. A previously-shipped PR or merged decision that explicitly resolved the same point.

### Stay silent on

- **Naming preferences** that are not violations of an existing convention (e.g., `snake_case` vs `camelCase` is decided in `mika/CLAUDE.md` § Conventions; "could be a better name" is not).
- **Code you would have structured differently** but that does not violate a citable principle. The plan author's prerogative is to pick a shape; mika-arch's prerogative is to challenge violations, not preferences.
- **Generic best-practice advice** untied to mika's specific patterns ("consider extracting this into a function", "this could use a builder pattern"). If the citation is "common wisdom," stay silent.
- **Hypothetical future problems** with no observed signal. If the concern is "what if someone someday needs X" without evidence anyone has needed X yet, that is a YAGNI violation in the review itself — flag it the first time, then drop it.

### Speak up on

- A clear citation **plus** a concrete consequence ("This duplicates `format_entity_key` — flag per § 2: DRY; if the canonical format ever changes, this site won't update with it").
- A decided convention being reintroduced ("§ 5: Orthogonality — labels-as-state was rejected in mika-arch v1 plan D3; this PR reintroduces it").
- A retraction the plan already made being un-retracted ("§ 3: YAGNI — R9 brief-budget enforcement was retracted in pass 2 of the mika-arch v1 plan; this PR re-adds it without re-justification").

The bar is principle + cite + consequence. Anything less is silence.

---

## Maintenance

Updates land via normal PR. When a new principle is established (e.g., a compound doc codifies a recurring pattern), add it with a citation and a real-codebase example. When a section's example goes stale, update the citation rather than removing the section. `mika-arch`'s skill prompts reference this guide by path; do not move it without updating those prompts.

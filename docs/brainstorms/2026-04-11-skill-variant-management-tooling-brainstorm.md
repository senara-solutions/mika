---
date: 2026-04-11
status: brainstorm
author: Vincent + Claude
---

# Skill Variant Management Tooling

## What We're Building

A set of `mika skills variants` CLI operations and a CI validation path for managing per-provider/per-model prompt variants in mika-skills. Variants are a **first-class public extension mechanism** for skill authors — not an internal calibration artifact pattern. They let a skill ship structurally condensed or calibration-reordered prompts for models whose attention or size constraints differ from the base.

The tooling must detect drift, validate integrity, promote experimental variants into production, and extend the existing `mika skills validate` CI gate to catch breakage before merge. Co-located layout (`<skill>/generated/<provider>/<model>/system_prompt.md`) stays unchanged — it's what makes variants portable when a skill clones.

**Motivating incident:** Sprint 2026-04-11 deleted all 4 existing variants because the minimax self-dev variant silently carried an unconditional `{"action":"allow"}` directive that was fixed in the base but never propagated. No tooling existed to detect the drift. The fix chain: self-dev base gated on `[claude-pilot] ` prefix, then a defensive deletion of all variants until management tooling catches up.

## Why This Approach

**Two-tier model with a promotion gate.** This matches how the minimax-m2.7 variant actually lifecycle'd: generated during a calibration run, verified, then committed as a reproducible checkpoint. The tooling formalizes that flow rather than replacing it.

1. **`experimental/<provider>/<model>/`** — quarantined directory, **not picked up by the runtime `resolve_prompt` resolver**. Free-form playground for calibration experiments and A/B testing. No CI gate enforces it. Any skill author can add variants here without ceremony.

2. **`generated/<provider>/<model>/`** — production variants, runtime-resolved. Promotion from `experimental/` to `generated/` runs the validation gate: size ≤ limit, required section headings present, diff vs base shows no missing calibration rules, markdown well-formed.

**Why not "kill variants permanently + runtime contextual injection":** Injection can hint a model about which provider it's running on, but it can't condense a prompt under a token budget or reorder calibration rules for different attention patterns. Injection is a *complement*, not a substitute. It belongs in a separate mechanism and a separate brainstorm.

**Why not "internal-only ergonomics":** The `generated/<provider>/<model>/` layout is already shipped to users via skill install, so variants are a public contract whether we formalize the tooling or not. Building internal-ergonomics-first is fine, but the data model, directory layout, size limits, and CI gate are part of the public skill authoring spec from day one.

## Key Decisions

### Design principle: warn, don't cascade

**The tooling NEVER cascades changes automatically. It warns, reports consequences, and tells the operator what to do next — but the decision to act is always a deliberate human intent.**

This applies to every interaction between a base prompt and its variants, and between experimental and production variants. When the system detects a condition that would conventionally trigger an automated action (base edited → variants need review, staleness threshold exceeded, validation rule failed, regen available), the response is a **structured warning with a recommended next step**, not an autonomous update.

Rationale:

1. **Prevents recursion and infinite loops.** Auto-cascading a base edit into N variants could itself trigger another audit, which could trigger another propagation, which could flag additional variants — the system would have no natural termination. A human in the loop is the clean termination.

2. **Reflection is an operator intent, not a system behavior.** "Reflection" here means the act of reviewing what the base changed and deciding what, if anything, to port to a variant. That decision requires context (why did the base change? is the variant's model-specific tuning still relevant? does the new base instruction conflict with the variant's condensation?) that the system doesn't have and shouldn't try to infer.

3. **Base prompts are model-agnostic by contract.** The canonical `system_prompt.md` at a skill root **must never be tuned for a specific model**. It's the neutral fallback. Variants carry all model-specific tuning. This means an edit to the base is always a cross-cutting change that SHOULD trigger review of every variant — and that review is sensitive enough that it can't be automated.

4. **Preserves the skill author's autonomy.** A third-party skill author who ships a variant has made a deliberate choice about what the variant contains. The platform can inform them that their variant may need an update, but must not rewrite it under them.

**Concrete applications of this principle:**

- **Base edit warning.** When `mika skills validate` or CI detects that a skill's base `system_prompt.md` was modified after one or more variants' last-touched timestamps, emit a non-blocking warning: *"skill `X` has Y variants that may need reflection after the base change. Run `mika skills variants diff X <provider>/<model>` to compare, and `mika skills variants reflect X` for a summary of what the base changed."* The warning is purely informational. No variant is modified.

- **New `reflect` operation.** Add a seventh CLI command: `mika skills variants reflect <skill>` — an operator-initiated audit that prints (a) the base diff since the oldest variant's last-touched timestamp, (b) per-variant impact assessment (which base sections changed and whether those sections are present in the variant), and (c) a list of recommended next actions (e.g., "consider porting base paragraph X to the minimax variant because it reorders the Step 4 section that the variant condenses"). **Reflect never writes any file.** It only reports. The operator then runs `diff`, `regen`, or manual edits at their discretion.

- **`regen` is prepare-only in v1.** CLI `regen` prints the `review_skill` invocation the operator should run. It does NOT spawn claude-pilot on its own. Even in v2 when the dashboard grows a `Regen` button, the button gates behind a confirmation modal that explicitly names the cost implication and requires the internal token.

- **`experimental/` bit-rot is reported, never auto-deleted.** CI warning when an experimental variant hasn't been touched in N days (suggest 30). The warning lists stale variants and their last commit. The operator decides whether to promote, delete, or leave them.

- **Validation failures never trigger auto-fix.** A failed `validate` run produces a list of rule violations + recommended fixes, and that's it. No "auto-reset to base" shortcut. The operator looks at each violation and decides.

### Layout & contract

- **Layout stays co-located:** `<skill>/generated/<provider>/<model>/system_prompt.md` for production, `<skill>/experimental/<provider>/<model>/system_prompt.md` for in-flight calibration. No top-level variants registry.
- **Runtime resolver ignores `experimental/`.** Only `generated/` is part of the four-step fallback chain. This is a small mika core change alongside the tooling.
- **skill.toml gains an optional `[variants]` section** declaring which providers/models the skill ships variants for:
  ```toml
  [variants]
  providers = ["minimax", "deepseek", "anthropic"]
  max_prompt_size = 32768  # inherits global default if absent
  ```
  Optional because existing skills without variants stay compatible. Serves as an author-facing manifest: "I intentionally tuned these models; treat the rest as base-only."

### Surfaces (orthogonal platform principle)

Every variant operation must be available across **three surfaces**: CLI (developer ergonomics + scripting + CI), mika-server HTTP API (programmatic access for agents and external tools), and dashboard UI (visual review + operational overview). The same functionality, the same data model, three ways to reach it.

Not all surfaces implement every operation at v1 — some are read-only on the dashboard, some are CLI-only for now — but the data model and operation names stay consistent so adding a surface later is mechanical.

#### CLI — `mika skills variants <op>`

| Command | Purpose | Write side effect? |
|---|---|---|
| `list <skill>` | Enumerate variants for a skill with provider, model, size (bytes/tokens/lines), staleness vs base | read-only |
| `status` | Cross-skill summary — how many variants, how many stale, how many experimental, which skills have drift | read-only |
| `diff <skill> <provider>/<model>` | Show variant ↔ base diff; classify as pure-wording vs semantic vs structural; surface missing calibration rule headings | read-only |
| `reflect <skill>` | Operator-initiated post-base-edit audit. Prints the base diff since the oldest variant's last-touched timestamp, per-variant impact assessment, and a list of recommended next actions. **Never writes any file.** Pure reporting. | read-only |
| `validate <skill>` | Run the gate against all production variants of a skill: size, required sections, markdown well-formedness, tool reference lint. Reports violations + recommended fixes, does not auto-fix. | read-only |
| `promote <skill> <provider>/<model>` | Move experimental → generated; runs `validate` first; refuses if the gate fails | **writes** (operator-initiated) |
| `regen <skill> <provider>/<model>` | Prepare a `review_skill` invocation string to regenerate a variant via LLM. **Prints the invocation; does NOT spawn claude-pilot.** Operator runs it explicitly if they want to. | read-only (prepare-only) |

Only `promote` is a write operation. Every other command is either purely informational (`list`, `status`, `diff`, `reflect`, `validate`) or prepare-only (`regen`) — matches the design principle.

`mika skills validate` (existing command) is extended: if the skill has production variants, validate them too. Runs in CI on PRs that touch skills. All `variants` subcommands support `--format text|json` for scripting (matches the existing pattern for `list`, `info`, `validate`, etc.).

**Base-edit warning hook.** When `mika skills validate` (or its CI wrapper) detects that a skill's base `system_prompt.md` was modified after one or more variants' last-touched timestamps, it emits a non-blocking warning listing the affected variants and recommending `mika skills variants reflect <skill>`. This is the canonical "warn, don't cascade" trigger point.

#### mika-server HTTP API — `/api/v1/skills/variants/...`

Routed under the existing dashboard-token-authenticated zone (`MIKA_DASHBOARD_TOKEN` for reads, `MIKA_INTERNAL_TOKEN` for writes). JSON request/response, same shape as the CLI `--format json` output so the two surfaces are interchangeable for scripts.

| Method + Path | Purpose | Auth | Write? |
|---|---|---|---|
| `GET /api/v1/skills/variants` | Cross-skill variant summary (status equivalent) | dashboard | no |
| `GET /api/v1/skills/{skill}/variants` | List variants for one skill | dashboard | no |
| `GET /api/v1/skills/{skill}/variants/{provider}/{model}` | Variant content + metadata (size, staleness, incident provenance) | dashboard | no |
| `GET /api/v1/skills/{skill}/variants/{provider}/{model}/diff` | Diff payload (unified diff + classification + missing sections) | dashboard | no |
| `GET /api/v1/skills/{skill}/variants/reflect` | Operator audit report — base diff since oldest variant + per-variant impact assessment + recommended next actions. **GET, not POST** — it only reports, never writes. | dashboard | no |
| `POST /api/v1/skills/{skill}/variants/{provider}/{model}/validate` | Run validation gate, return pass/fail + rule violations | dashboard | no |
| `POST /api/v1/skills/{skill}/variants/{provider}/{model}/promote` | Move experimental → generated after validation | internal | yes |
| `POST /api/v1/skills/{skill}/variants/{provider}/{model}/regen` (v2) | Trigger LLM regen, spawn claude-pilot, return task ID (long-running callback pattern) | internal | yes |

Read endpoints are safe to expose with dashboard-token auth so the SPA can render without needing the internal superuser token. `validate` is `POST` because it runs a gate (has compute cost), but doesn't write anything. Only `promote` writes in v1. `regen` is deferred to v2 as the only true write endpoint that spawns background processes.

Note that `reflect` is a **`GET`**, not a `POST` — explicit signal that it only reports. This matches the design principle: the audit never mutates anything, the operator looks at the report and decides what to do next.

#### Dashboard — new "Skills → Variants" page

Visual review surface. Integrates with the existing `@senara-solutions/ui` component library and matches the look-and-feel of the current Skills, LLM Calls, and Tool Calls dashboard pages.

- **Left panel:** skill list with badges per skill — total variant count, stale count (red), experimental count (grey). Clickable → right panel.
- **Right panel — variant table:** rows keyed by `provider/model`, columns: size (bytes / tokens / lines), staleness (days since base modified), drift class (pure-wording / semantic / structural), last git commit (SHA + message + author), validation status (green check / red cross + rule names).
- **Drill-in on a variant row:** opens a side-by-side diff viewer. Base prompt left, variant prompt right, inline-highlighted. Missing sections and drifted headings flagged at the top.
- **Action buttons (inline on the variant row):**
  - `Validate` — runs the gate via `POST .../validate`, shows results as a toast + inline violation list.
  - `Promote` — only visible on experimental variants. Runs validate first, then moves. Confirmation modal before any write.
  - `Regen` — confirmation modal (this spawns claude-pilot, which has cost implications). Disabled for users without the internal token.
- **Operational panel (v2):** "Variant usage this week" strip at the top of the page, aggregating `llm_calls.prompt_variant` over the last 7 days. Total resolutions per variant, avg turns/cost/success rate. Click-through to the LLM Calls page filtered to that variant. Builds on the schema already in place (v21 added the column).
- **Incident provenance (v2):** if a variant has an associated `VARIANT.toml` metadata file (per #8 in Open Questions below), show "Created during incident `trace-id` on YYYY-MM-DD" with a click-through to the unified timeline filtered to that trace.

### Validation gate (what `validate` and `promote` check)

Start minimal — these four rules are what today's incident would have caught:

1. **Size limit:** `len(variant) ≤ max_prompt_size` from `[variants]` or global default (32 KB).
2. **Required section headings present:** If the base has headings matching a regex (e.g., `## Calibration Rules`, `## Step 4 — Handle permission requests`), the variant must also have them. Any missing heading → gate fails with a list of missing sections. This is what would have flagged the minimax variant's unconditional PilotResponse directive after the base was gated on the prefix.
3. **Markdown well-formedness:** Same rules as `validate_markdown_content()` in mika core (no null bytes, no unclosed code fences, no control chars).
4. **Tool reference lint:** Scan for `{{key}}` placeholders and verify they exist in the base's `[context.*]` declarations. Scan for tool names mentioned in imperative directives (e.g., "call `update_work_item_status`") and verify they exist in the skill's `tools.json` or the registered builtins list (per mika Rule 4 in permission-policy).

Rules deferred to v2 after the MVP ships and we see what actually drifts:

- **Imperative density delta** — count of `MUST`/`ONLY`/`NEVER`/`always` per 100 words; flag variants significantly above base.
- **Unconditional-directive detection** — regex or parser for absolute statements not wrapped in a preceding condition. This is the most nuanced check and would have caught today's bug directly, but designing the regex right is a project in itself.
- **Semantic similarity** (embedding-based) for distinguishing pure-wording changes from real divergence.
- **Risk scoring** (composite health number).

### CI integration

Extend the existing `mika skills validate` CI workflow (planned in mika-skills#129 for pre-commit + CI) to include variant validation:

- On PRs touching `<skill>/`, run `mika skills validate <skill>` which transitively validates production variants.
- `experimental/` is always skipped — it's a playground, not a contract.
- Validation failures block merge with a list of specific rule violations and the affected variant files.

### Runtime observability (deferred — already partially built)

The `llm_calls.prompt_variant` column added in mika schema v21 already records which variant was resolved per LLM call. Aggregating that into operational metrics ("variant X was used N times this week, averaged K turns, $M cost, P% success") is a natural extension but **deferred out of this brainstorm** — it's a dashboard concern, not a management tooling concern. The foundation exists; the UI can land later.

## Alternatives Considered

**1. Kill variants permanently, use runtime contextual injection instead.** Rejected. Injection solves a different problem (hinting the model about runtime state) and can't replace a variant when the model needs structural condensation or token-budget compliance. Keeping both mechanisms (variants for structure, injection for runtime hints) is cleaner than forcing one to do both jobs.

**2. Internal-only tooling, no public contract.** Rejected. The `generated/` layout is already shipped via skill install. Variants are a public contract whether we document them or not. Building as a public extension mechanism from day one avoids a future migration.

**3. Experimental-only, no production promotion.** Rejected. Production variants are a legitimate need (the minimax calibration sprint proved this). Removing the promotion path forces calibration winners to stay in "experimental limbo" forever, which is worse than today's state.

**4. Maximalist v1 — semantic diff + risk scoring + imperative density + runtime dashboards.** Rejected per YAGNI. The MVP is four validation rules and six CLI operations. Every deferred feature becomes easy to add once the foundation exists and we have real drift data to learn from.

## Open Questions

1. **Where does `max_prompt_size` default live?** Options: (a) hardcoded in mika core (32 KB today in skill manifest schema), (b) global config in `~/.mika/.env`, (c) per-skill in `[variants]`. Probably (a) as default, (c) as override. Confirm before shipping.

2. **What counts as a "required section heading"?** Exact-match on the base's `##` headings? Or author-declared in `[variants]` via something like `required_headings = ["## Calibration Rules", "## Step 4 — ..."]`? The author-declared form is more flexible but adds surface area. Exact-match-to-base is simpler and catches drift automatically, but fails if the base intentionally reorders sections. **Recommend exact-match-to-base for MVP, revisit if it produces false positives.**

3. **Tool reference lint — how deep does it go?** Easy: flag direct mentions of tools not in `tools.json`. Hard: infer from context whether the mention is a real tool call vs just prose naming a tool. MVP: easy only. Regex-based, accept false positives at the margin.

4. **Drift staleness threshold — when does a variant count as "stale"?** Absolute time (e.g., base modified after variant)? Or change-count (e.g., base has had N commits since variant was last touched)? **Recommend absolute time based on git log of both files; staleness is a signal, not a blocker.**

5. **Runtime resolver behavior for unknown variant declarations.** As third-party authors start shipping skills with `[variants]` declarations, we may encounter variants for providers/models outside the runtime's supported list (*"this skill declares a variant for `openai/gpt-5`, but your runtime doesn't know that model"*). **Recommend: runtime warns on unresolvable variant declarations, does not error. Matches the 'warn, don't cascade' principle — it's an informational signal, not a hard stop.**

6. **How does the base-edit warning hook know the variant's "last-touched timestamp"?** Options: (a) filesystem `mtime`, which is unreliable across clones; (b) `git log -1 --format=%at <path>`, which is authoritative but slow on large trees; (c) a sibling metadata file (see VARIANT.toml in #7 below) that records the base SHA the variant was synced against. **Recommend (b) for v1, consider (c) as part of v2 provenance work.**

7. **VARIANT.toml provenance metadata.** Should each variant ship a sibling `VARIANT.toml` file that captures: `created_at`, `base_sha_at_last_sync`, `motivating_incident` (trace ID + description), `calibration_metric` (what was measured, what improved)? **Yes, in v2.** The provenance dashboard panel (from the Surfaces section) depends on this. v1 ships without it — variants can be audited via git log alone — but the MVP data model leaves room for the file to be added later without a breaking change.

## MVP Split

Orthogonal breakdown by surface. v1 ships the core read + validate + promote flow across all three surfaces; v2 adds nicer analytics and regen automation.

### v1 — minimum viable slice (what's needed to reintroduce variants safely)

**CLI (`mika skills variants`):**
- `list`, `status`, `diff`, `reflect`, `validate`, `promote`
- `regen` lands in v1 as a **prepare-only** version: prints the `review_skill` invocation string, does not spawn claude-pilot directly (per cascade principle)

**mika-server (`/api/v1/skills/variants/...`):**
- Read endpoints: `GET` cross-skill summary, list, content, diff, reflect
- `POST .../validate` (has compute cost, but doesn't mutate)
- `POST .../promote` (only v1 write endpoint)
- `POST .../regen` deferred to v2 (CLI version is prepare-only, no server endpoint needed)

**Dashboard (new Skills → Variants page):**
- Left panel: skill list with variant/stale/experimental badges
- Right panel: variant table with metadata and drift classification
- Side-by-side diff viewer
- "Reflect" button → pulls `GET .../reflect` and renders the report inline (read-only, always available)
- `Validate` and `Promote` action buttons (wired to the mika-server endpoints)
- `Regen` button **hidden** in v1
- **Base-edit banner.** When a skill's base was modified after any variant's last-touched timestamp, show an amber banner at the top of the variant table: *"Base prompt modified on YYYY-MM-DD after N variants were last synced. Review recommended — click Reflect for a summary."* No action taken, just a warning per the cascade principle.

**Shared data model / server-side (`crates/mika-agent/src/skills/variants.rs` or similar):**
- Variant metadata struct (provider, model, paths, size, staleness, validation status)
- Validation gate with the 4 MVP rules (size, required headings, markdown well-formedness, tool reference lint)
- `experimental/` directory convention + runtime resolver skipping it (mika core change)
- skill.toml `[variants]` section parsing (optional, backward-compatible)
- Reflection report builder (base diff + per-variant impact analysis + recommended next actions) — pure computation, no writes

**CI:**
- Existing `mika skills validate` workflow extended to transitively validate production variants on changed skills. `experimental/` skipped.
- **Base-edit warning:** PRs that modify a skill's base `system_prompt.md` get a non-blocking CI comment listing any variants whose `last-touched` is older than the base's new commit, plus the recommended `mika skills variants reflect <skill>` invocation.

### v2 — nice to have, not blocking

**CLI + mika-server:**
- `regen` as a real end-to-end command (spawns claude-pilot, returns task ID, follows the long-running callback pattern)
- `POST .../regen` endpoint

**Dashboard:**
- `Regen` button with confirmation modal
- Operational panel: "Variant usage this week" strip aggregating `llm_calls.prompt_variant` over 7 days
- Incident provenance display (requires `VARIANT.toml` convention from Open Questions #8)

**Validation rules (additions to the gate):**
- Drift staleness computation and reporting (absolute time + commit-count signal)
- Semantic similarity via embeddings (distinguishes wording from meaning)
- Imperative density delta (flag variants significantly more absolute than base)
- Unconditional-directive detection (regex or parser — directly catches the bug class that triggered this brainstorm)

### Deferred indefinitely (maybe never)

- Composite risk scoring (single health number) — until we see real data showing the individual signals aren't enough
- Auto-propagation of base changes to variants — dangerous, doesn't match the human-in-the-loop model
- DB-backed variant registry — filesystem-only is simpler and matches skill-install portability

## Resolved Questions

*(Originally open, resolved during the brainstorm dialogue)*

- **Audience:** Public extension mechanism for skill authors. Internal-ergonomics-first in implementation priority, but data model and CLI shape are part of the public spec from day one.
- **Do we reintroduce variants?** Yes, with a promotion gate. Two-tier model (`experimental/` → `generated/`).
- **CLI shape:** `mika skills variants <op>` subcommand, parallel to `mika skills llm`.
- **Surfaces:** All operations must span **CLI + mika-server HTTP API + dashboard UI** from day one — the orthogonal platform principle. v1 implements the MVP slice across all three surfaces; deferred features are deferred uniformly, not on a per-surface basis. Shared server-side module (`crates/mika-agent/src/skills/variants.rs`) owns the data model so CLI and HTTP API both wrap the same logic.
- **Storage:** Filesystem-only, co-located under each skill's directory. No DB registry. Runtime telemetry (`llm_calls.prompt_variant`) is the only DB touchpoint and is already in place.
- **Runtime injection as alternative:** Rejected as a substitute; accepted as a future complementary mechanism for model hints.
- **Cascade principle — "warn, don't cascade".** The tooling NEVER auto-propagates, auto-fixes, auto-promotes, or auto-deletes. It emits structured warnings with recommended next actions. Every write is a deliberate operator intent. Rationale: prevents recursion / infinite loops; base edits are cross-cutting and sensitive to context the system can't infer; preserves author autonomy. Specifically applies to: base-edit warnings (`reflect` reports only), `regen` (prepare-only in v1), `experimental/` bit-rot (CI warning, no auto-delete), validation failures (violations + recommended fixes, no auto-fix), promotion (explicit operator command, not CI-auto).
- **Base prompts are model-agnostic by contract.** The canonical `system_prompt.md` at a skill root must never be tuned for a specific model. Variants carry all model-specific tuning. Enforced socially via the authoring spec; may grow a lint rule later.
- **Who runs `promote`?** Manual — keeps the author in control and makes the review PR-level visible. Not CI-automatic. (Resolved by the cascade principle: `promote` is a deliberate operator intent.)
- **Does `regen` spawn claude-pilot automatically?** No in v1 (prepare-only, prints the `review_skill` invocation). In v2 when the dashboard gains a `Regen` button, it gates behind a confirmation modal that names the cost implication and requires the internal token. (Resolved by the cascade principle.)
- **How is `experimental/` bit-rot managed?** CI warning, no auto-delete. Operator decides whether to promote, delete, or leave them. (Resolved by the cascade principle.)
- **Do validation failures trigger auto-fix?** No. `validate` reports violations + recommended fixes. Operator applies fixes manually or runs `regen` to prepare a new variant. (Resolved by the cascade principle.)

# Non-transit data-grade doctrine

> **Status:** invariant (mika#1798).
> **Origin:** 2026-07-18 Prime ratification via samidarko relay (Vincent-ratified).
> **Trigger incident:** Vincent's cloud Mika proposed granting itself Gmail /
> Calendar / Drive OAuth access during a family-tier interaction. That proposal
> is a breach of the non-transit doctrine — **the grade of the data determines
> Mika's access, not the convenience of the moment**.

## Bearing (the rule)

**Data-grade determines access, not convenience.**

- Mika MAY access **operational-grade** data, read-mostly, when a channel is
  explicitly wired:
  - Calendar (read-only when authorized)
  - App-scoped files (`drive.file` scope only, never full Drive)
- Mika **MAY NOT** access — nor **propose** accessing — **testimony-grade**
  data:
  - Gmail / email content
  - Full Drive (read/write)
  - Personal journals, confessional, intimate content

**HARD NO covers both the doing and the proposing.** A well-meaning "I could
help if you gave me Gmail access…" is a breach at the propose surface, even
without a tool call. Structural refusal is the rule; naming the doctrine when
declining is the shape.

**Opening testimony-grade access requires the explicit sovereign consent of
the person whose testimony data is at stake.** Never Mika's own proposal.
Runtime override does not exist in v1: opening a testimony surface requires a
code change removing the tag / adding a subcommand-ban entry, which is a
review-gated commit.

## Data grade taxonomy

| Grade | Examples | Access shape |
|---|---|---|
| **Operational** | Calendar entries (who/when only), app-scoped file storage under `drive.file`, workspace metadata | Read-mostly, opt-in wiring per channel |
| **Testimony** | Gmail message content, full Drive (any file the user owns), personal journals, confessional content | HARD NO — no access, no propose |

**Boundary cases (worked):**

- *Calendar who/when* — operational, unless coherence-architect's pending
  check finds it leaks intimacy (see § Vigilance surface below).
- *`drive.file` scope, app-created files only* — operational; recognizable
  by `q` filter restricted to `'me' in owners` OR `appProperties has ...`
  marker.
- *`drive.file` scope but reading files created outside Mika's app scope* —
  testimony (full-Drive semantics through a scope crack); refused by Layer 3
  substring gate.
- *Gmail metadata (from/to/subject) without body* — still testimony in v1;
  the header discloses the correspondent set, which is journal-shaped in
  aggregate.
- *A ChatGPT-style "I have access to your email"* — never true for Mika,
  never proposed by Mika.

## Four-layer structural defense

Prompt-only enforcement is empirically fragile (n≥3 substrate hits documented
in `feedback_prompt_enforcement_fragile` and
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). The
doctrine ships **four composable layers**. Any single layer failing does not
silently open testimony-grade access.

### Layer 1 — Prompt template (`crates/mika-agent/src/prompt.rs`)

`write_data_grade_doctrine_section()` renders a `## Data-Grade Doctrine`
section on every turn of every agent that carries a normal system prompt.
Placed immediately after `write_identity_section` and before
`write_time_section` — the doctrine is one of the first grounding statements
the model sees, well ahead of `## Instructions` and any injected core-memory
or skill content (see CLAUDE.md § Context priority rule).

Compact-provider (`build_compact_system_prompt`, ~5 KB budget) renders an
abbreviated 4-line variant hard-capped at 400 chars via
`const_assert!` — any future edit that busts the budget fails to compile.

**Coverage:** grounds the *propose* surface for every LLM-driven turn.

### Layer 2 — Skill registry ban (`crates/mika-agent/src/skills/mod.rs`)

`SkillRegistry::apply_testimony_grade_ban()` runs as Phase 2 (after
`apply_identity_allowlist` Phase -1 and `apply_overrides` Phase 0/1, before
`apply_load_safety_check`). Any skill whose manifest declares
`data_grade = "testimony"` is evicted from the registry unconditionally, with
a WARN log line (`event = "skill_testimony_ban"`).

**No per-agent override surface in v1.** Even if an identity allowlist AND a
DB override both re-enable a testimony skill, Phase 2 still evicts it. The
ban is structural, matching Prime's "set once" contract.

**Wired into every skill-loading callsite:** server init
(`server/mod.rs`), hot-reload handlers (`server/handlers.rs`, `server/a2a.rs`),
CLI paths (`chat.rs` initial + two hot-reload paths, `skills.rs`), team engine
(`teams/engine.rs`), `delegate_task`, `list_skills` tool, and the
well-known-agent coherence test.

**Coverage:** grounds the *access* surface for any skill-registered tool
whose owning skill declares testimony. Does NOT cover the incumbent
`run_gws` Gmail path, which is intentionally untagged at the skill level
(so Calendar stays operational — the ban lives inside the tool handler
where the granularity is; see Layer 3).

### Layer 3 — Per-tool subcommand ban (`crates/mika-agent/src/skills/builtin_handlers.rs`)

`validate_gws_input()` gains two doctrine-scoped checks after the existing
subcommand allowlist and flag-smuggling checks:

- **Gmail HARD NO:** any `gmail *` invocation is rejected pre-spawn with
  `TESTIMONY_GRADE_FORBIDDEN_GMAIL` (structured JSON discriminator
  `error = "testimony_grade_forbidden"`, `doctrine = "mika#1798"`).
  No subprocess is spawned.

- **Drive scope-limit:** any `drive files list|get|create|delete|update`
  invocation whose `--params` does not restrict scope to `'me' in owners`
  OR `appProperties has ...` marker is refused with
  `TESTIMONY_GRADE_FORBIDDEN_DRIVE`. Malformed JSON is treated as
  full-Drive scope and refused (fail-closed). Calendar remains functionally
  permitted (this ticket does NOT wire real calendar auth — the guard is in
  place so that any future token wiring lands under the ban).

**Coverage:** the sole load-bearing structural layer for the incumbent
`run_gws` Gmail path that triggered the doctrine. This is the layer that
would have refused the 2026-07-18 propose-Gmail surface if the model had
attempted to call the tool.

### Layer 4 — Execute-time guardrail (`crates/mika-agent/src/tool_execution/dispatch.rs`)

`execute_tool` gains a pre-dispatch check right at the top: for any tool
whose owning skill declared `data_grade = "testimony"`, the call is refused
with `TOOL_TESTIMONY_GRADE_FORBIDDEN` before the handler runs. The lookup is
O(1) via `dispatch.skill_data_grades: HashMap<String, DataGrade>`, built
once per `ToolDispatchCtx` construction from the same matched-skill list as
`skill_tools` (last-write-wins consistency).

**Coverage delta over Layer 2** (why this is orthogonal, not duplicative):

1. **Hot-reload race window.** `mika skills install <path>` and hot-reload
   handlers rebuild the registry and re-apply Phase order. Between the old
   registry's drop and the new registry's Phase 2 completion, a request
   routed at the old-registry dispatcher could dispatch a tool whose owning
   skill was tagged testimony in the new manifest. Layer 4 is stateless
   (reads `skill_data_grades` at execute-time from the current registry
   snapshot) and closes this window.

2. **Dynamic MCP registration (forward-compatible).** MCP servers can
   register tools at runtime post-startup. Phase 2 ran once at server init;
   a dynamically-registered MCP tool whose "skill" (the MCP server's
   manifest) declares `data_grade = "testimony"` would not be caught by the
   startup ban but is caught by the execute-time Layer 4 lookup. Present-day
   MCP integration does not carry `data_grade` on MCP-server manifests, but
   the Layer 4 shape is forward-compatible; this is the extension surface.

3. **Registry mutation via `mika skills` DB overrides.** `skill_overrides`
   DB rows can re-enable a skill after Phase 2 ran (mika#682 transient
   overrides + hot-reload). Layer 4's check is stateless and does not depend
   on the enabled/disabled state — an override that re-enables a testimony
   skill still gets caught at execute-time.

## Ban list (v1)

Structural bans (refused pre-handler, no side effects):

- **Gmail:** `run_gws gmail *` — all subcommands (Layer 3).
- **Full Drive:** `run_gws drive files list|get|create|delete|update` without
  `q` restricted to app-created markers (Layer 3).
- **Any skill-registered tool** whose owning skill declares
  `data_grade = "testimony"` (Layers 2 + 4).

Bans in the doctrine but not (yet) reachable through today's tool surface:

- Personal journals (no journal-fetching tool exists in v1).
- Confessional content (no dedicated tool; a future OAuth-wrapped skill
  would need to declare `data_grade = "testimony"` — see Vigilance surface).

## Operational-grade carve-outs

- **Calendar** — read-only when authorized. This ticket does NOT wire real
  calendar auth; the guard is in place so that any future token wiring lands
  under the doctrine. Pending coherence-architect check on "does calendar
  who/when leak intimacy?" — if the check finds it does, calendar migrates
  to testimony-grade with a doc-plus-tests-plus-guard update, cleanly
  composable with this ticket's shape.

- **`drive.file` scope, app-created files only** — recognizable by `q`
  filter restricted to `'me' in owners` OR `appProperties has ...` marker
  (Layer 3 substring gate). API-layer semantics of the `q` string beyond
  those markers are not analyzed — a crafted `q` that passes the substring
  check but exposes broader scope is not caught here; v1 relies on
  Deliverable 3 (skill-level ban) + operator-review-gated code changes for
  the broader Drive testimony surface.

## Operator override path

**There is no runtime override in v1.** Not a CLI flag. Not an env var. Not
a DB row. Opening a testimony-grade path requires a code change that either:

1. Removes the `data_grade = "testimony"` tag from a skill's manifest, OR
2. Adds a subcommand-ban exception to `validate_gws_input`, OR
3. Threads a new tool through the dispatch chain without declaring it
   testimony (see Vigilance surface — this is the failure mode to prevent).

**Any such change is a review-gated commit.** For a real deploy that touches
someone's testimony data, the person whose data is at stake must give
sovereign consent per Prime doctrine — never Mika, never Mika's own proposal.

**Emergency operator-review-gated override:** for a paid feature that legitimately
wants testimony-grade access (e.g., a Gmail summarizer the user explicitly
opted into by clicking through a consent flow), the deploy-time contract is:
(a) new skill with `data_grade = "testimony"` removed OR (b) new subcommand
allowlist entry with a `_` doctrine comment explaining the consent chain.
Both paths land through CI and PR review; neither is available to a running
agent.

## Vigilance surface (single axis of vigilance for future changes)

Any future non-`run_gws` path to testimony-grade data — MCP-registered Gmail
tool, new builtin, dedicated Gmail-only skill, OAuth wrapper — MUST either:

1. **Declare `data_grade = "testimony"` at manifest time** so Layers 2 and 4
   fire structurally, OR
2. **Add its own subcommand-level ban entry** (same pattern as `run_gws`'s
   Gmail check).

If a future change does NEITHER, the only remaining defense is Layer 1 (the
prompt) — the fragile layer the doctrine explicitly distrusts. **This is
the single axis of vigilance for every future testimony-adjacent change.**

### Applied hardening (closed after mika#1798)

- **`shell-exec` command-line bypass — CLOSED by mika#1957.** The `shell-exec`
  skill accepts shell commands and runs them via `eval`. Its original
  per-command block-list inspected only the first token
  (`awk '{print $1}'`), so every shape that reached `gws`/`gh` through a
  subshell (`sh -c 'gws ...'`, `bash -c "gws ..."`, `eval "gws ..."`), a pipe
  into a shell (`echo 'gws ...' | sh`), a path prefix (`/usr/bin/gws ...`), a
  statement separator (`pwd; gws ...`, `true && gws ...`, a newline), or a
  command substitution (`` `gws ...` ``, `$(gws ...)`) walked past it — and
  bypassed all four L1–L4 layers with it, because the call never entered the
  `run_gws` builtin handler.

  `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` now runs a
  lexical scan of the whole command string on an identifier boundary
  (`(^|[^A-Za-z0-9_.-])(gws|gh)([^A-Za-z0-9_.-]|$)`), which subsumes
  whitespace, both quote characters, `;`, `|`, `&`, backtick, `$`, `(`, and
  `/`. Regression suite:
  `crates/mika-agent/tests/shell_exec_l3_hardening.rs` (20 cases — every
  bypass shape above, the two first-word regressions, and six
  false-positive guards).

  **Tier characterisation, corrected.** The pre-mika#1957 text in this section
  said "personal-tier agents ship with `shell-exec` in
  `DEFAULT_AGENT_SKILL_ALLOWLIST`". That conflated two things.
  `DEFAULT_AGENT_SKILL_ALLOWLIST` backs `AgentTier::Default`, the
  **operator/orchestrator** persona, where `shell-exec` is load-bearing for the
  orchestrator seat (mika#1641). The **family** tier — the population this
  doctrine exists to protect — is `FAMILY_AGENT_SKILL_ALLOWLIST` (mika#1778),
  which has never contained `shell-exec`, with a test in
  `crates/mika-common/src/home.rs` asserting the exclusion. Removing
  `shell-exec` from the Default allowlist was therefore rejected: it would have
  regressed mika#1641 for no added coverage, since the `run.sh` scan fires for
  every tier regardless of which allowlist granted the skill.

  **Deliberately not closed** (defense-in-depth, not a sole gate): renamed or
  aliased binaries (`gws-alias`), base64/obfuscated payloads
  (`echo … | base64 -d | sh`), and raw-HTTP calls to the underlying APIs
  (`curl https://gmail.googleapis.com/...`). The first two are still caught by
  L2 and L4 at the real tool call; the third is covered by no layer and remains
  listed below.

- **Deliberate false-positive.** The scan is lexical, so a command that merely
  *mentions* `gws` or `gh` on an identifier boundary is refused too — e.g.
  `grep gws /etc/services`. Accepted: `shell-exec` is a command-execution
  surface, and separating "mention" from "invoke" would require parsing the
  shell grammar. Ordinary paths and refs are unaffected (`.github/...`,
  `gh-pages`, `/tmp/gws.log` all pass), because `.` and `-` are excluded from
  the boundary class.

### Known bypass classes (out of scope for mika#1798, tracked separately)

The four-layer defense covers the `run_gws` builtin surface + any future
tool that declares `data_grade = "testimony"`. With the `shell-exec` class
closed above, it still does NOT cover:

- **Raw-HTTP API surfaces** — a direct
  `curl -X POST https://gmail.googleapis.com/...` reaches testimony data
  without touching the `gws` CLI, so neither the mika#1957 scan nor L1–L4
  fires. Closing this needs the doctrine extended to the HTTP egress surface.
- **Registry-ban callsite drift** — the four-layer defense wires Phase 2
  at every `SkillRegistry::from_dir` callsite via manual pairing
  (~12 sites). Adding a new skill-loading site without pairing
  `apply_testimony_grade_ban` silently opens the surface. A follow-up
  should introduce a `SkillRegistry::load_for_agent(dir, identity,
  overrides)` wrapper that atomically returns a fully-phased registry so
  the ban cannot be forgotten.
- **MCP-registered testimony tools** — the L4 lookup map
  (`skill_data_grades`) is built from `skill_tools`, which excludes
  MCP-registered tools. An MCP server that registers a `gmail_fetch` or
  `mcp__gmail__*` tool reaches Gmail with zero doctrine layer firing.
  Forward-compat requires MCP manifests to gain a `data_grade` field AND
  the MCP dispatch site to consult the same map.

## Cross-references

- **Sibling ticket:** mika#1783 (Salut Vincent — Mika verbalizes
  config-substrate via person). Sibling doctrine on Mika-verbalization;
  shares the "Mika ne doit pas verbaliser/proposer certaines choses"
  surface.
- **Companion class:** mika#1814 (companion class per issue body).
- **Follow-up (F3, closed):** mika#1957 — `shell-exec` L3 hardening.
- **Memory anchors:** `feedback_prompt_enforcement_fragile`,
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
  `project_mika_prime_self_model_bounds`.
- **File anchors:**
  - Prompt writer: `crates/mika-agent/src/prompt.rs` §
    `write_data_grade_doctrine_section` /
    `write_data_grade_doctrine_section_compact`.
  - Registry ban: `crates/mika-agent/src/skills/mod.rs` §
    `SkillRegistry::apply_testimony_grade_ban`.
  - Subcommand ban: `crates/mika-agent/src/skills/builtin_handlers.rs` §
    `validate_gws_input`.
  - Execute-time guard: `crates/mika-agent/src/tool_execution/dispatch.rs` §
    `execute_tool` (top-of-function testimony check).
  - Manifest field: `crates/mika-agent/src/skills/manifest.rs` §
    `DataGrade` enum + `SkillInfo.data_grade`.

## Change log

- **2026-08-27** — mika#1957: `shell-exec` bypass class closed by a lexical
  command-string scan in the skill's `run.sh`; moved from § Known bypass
  classes to § Applied hardening. Corrected this document's own tier
  characterisation (Default = operator/orchestrator, not personal; the family
  tier never carried `shell-exec`). Tier-1 removal of `shell-exec` from
  `DEFAULT_AGENT_SKILL_ALLOWLIST` was proposed and rejected — it would have
  regressed the mika#1641 orchestrator seat for no added coverage.
- **2026-08-22** — Initial doctrine baked in mika#1798 across four
  structural layers plus this doc. Trigger incident: 2026-07-18 Prime
  ratification via samidarko relay (Vincent-ratified).

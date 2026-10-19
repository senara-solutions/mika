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

**Since mika#1960, Layer 1 is no longer alone on the *propose* surface.** An
EndTurn post-condition guard at position **5h**
(`crates/mika-agent/src/agent_loop/mod.rs`, predicate
`evidence::guards::detect_testimony_access_proposal`) refuses a turn whose text
proposes to **open** access to testimony-grade data, re-prompts once, and emits
`guard.testimony_access_proposal` — with
`guard.testimony_access_proposal_uncorrected` for the residue of that single
retry. No tool call is needed for it to fire, which is the whole point: the
breach this document quotes ("A well-meaning 'I could help if you gave me Gmail
access…'") is invisible to Layers 2/3/4 because nothing is ever called.

Its reach is the turn's **final assistant text**, and only that.

**Since mika#2627 that bound is no longer a hole.** A proposal sent through the
`send_message` tool — the only user channel of a silent turn (heartbeat,
callback, reminder), and also reachable from a conversation turn — is refused
**before delivery and before persistence** by the same predicate, applied to the
tool's outgoing body (`tools::check_testimony_access_proposal`). Three emitters
go through it: `send_message`, and `create_reminder` /
`create_scheduled_task` when their `action_type` is `send_message`, since a
deferred send is still a send. `delegate_task` relays its sender and emits
nothing itself (covered transitively, its delegate calls `send_message`).
**`run_team` is NOT covered, and it is not engine text either**: its completion
notification wraps `run.deliverable`, which is the LLM output of the team's
writer agent (`TeamEngine::deliver`), the conversational gate's reply, or the
workspace files read on timeout — sent verbatim through `message_sender` on both
the sync (`tools/run_team.rs`) and async (`task_engine::dispatcher`) paths. A
team agent can therefore deliver a proposal no testimony refusal reads, and the
workspace fallback never traverses 5h at all. That channel is named open below.

Two properties of that refusal, both load-bearing. It precedes the Telegram
length guard, because that guard's remedy is a split and **splitting a text that
proposes Gmail access yields four texts that propose it**. And it posts **no
`DeliveryVerdict`** (mika#2136): guard 6f counts a `RefusedTooLong` as a
non-delivery to repair by splitting, so a verdict here would re-prompt the turn
with the exact inverse of the right repair.

What is left open on the *propose* surface, named rather than implied: a proposal
**split across two calls** such that no single sentence carries both layers (the
predicate segments by sentence, and widening it past the sentence would reopen
the false positive that segmentation exists to avoid — same class as mika#2237,
which left its own degradation path open with its reason); and rows **scheduled
before** the mika#2627 deploy, whose firing site has no model to return a reason
to. Both are bounded and measurable — see the root `CLAUDE.md` for the queries.
And the **team deliverable** (`run_team`, above): the right guard site is the
deliverable itself in `TeamEngine::deliver` — one site covers both notification
paths — not the tool, and it is a follow-up, not a line of mika#2627.

Two properties worth knowing before touching it. The discriminant is the
**direction of the access movement**, never the vocabulary: Layer 1 *prescribes*
naming the doctrine when declining, so a conforming refusal necessarily carries
the forbidden subject and the word "access" in the same sentence, and a lexical
predicate over {Gmail} × {access} would refuse the refusal — i.e. break what
mika#1798 shipped. And the guard **rattrape, it does not prevent**: a
post-condition guard reads outgoing text, so what changed is that the *propose*
half stopped being prompt-only and gained a second net with its telemetry.
Regression coverage: `tests/eval/doctrine_regressions/testimony_access_proposal_caught.rs`
(the measured turn, the retry-budget boundary, and three negative controls on the
prescribed refusals) plus the thirteen predicate controls in
`evidence::guards::tests::mika1960_*`.

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

2. **MCP-registered tools — in force since mika#1959.** MCP servers register
   tools at runtime post-startup; Phase 2 ran once at server init, so the
   startup ban never sees them. They are caught by the execute-time Layer 4
   lookup, because the map is **seeded** with them: an MCP server's manifest
   entry (`mcp-servers.json`) carries `data_grade` (absent ⇒ `operational`, the load-bearing
   default), `mcp::mcp_tool_data_grades` joins it to each namespaced tool, and
   `agent_loop::seed_mcp_data_grades` inserts the result into
   `skill_data_grades` at each of the two sites that publish
   `mcp.tool_definitions()` to the model. The map is keyed by the
   Claude-facing tool name, which an MCP tool has (`mcp__<server>__<tool>`),
   so seeding was sufficient: **`execute_tool`'s body was not modified by
   mika#1959**, and the empty diff on that function is the proof that the
   forward-compatible shape this paragraph used to promise was real.

   The silent/callback/heartbeat path receives `None` in place of the manager
   and is out of population by construction. The pairing between publication
   and seeding is held structurally by
   `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`
   (allowlist shipped empty).

   **Residue, named rather than implied:** a testimony MCP tool stays
   *published* in `tool_definitions()`, so the model can still **propose** it
   — only the call is refused. See § *Vigilance surface*.

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
- **Any MCP-registered tool** whose server declares
  `"data_grade": "testimony"` in `mcp-servers.json` (Layer 4 only — there is no
  registry-eviction equivalent on the MCP side; mika#1959). The call is
  refused before `McpManager::call_tool`, so no remote request is issued.

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

**Option 1 now reaches MCP servers too (mika#1959)**: a server declares
`"data_grade": "testimony"` in `mcp-servers.json` and Layer 4 fires on every one of its
tools. Absence of the field reads `operational`, which is the load-bearing
default — a manifest that forgets to declare its grade cannot accidentally
acquire a testimony privilege; the explicit consent `"data_grade": "testimony"`
is what means *refusal*, never permission.

**What that does NOT buy, on this same axis (mika#2814).** Layer 4 refuses the
**call**, not the **publication**. A testimony MCP tool stays in
`tool_definitions()`, so the model can still *propose* it — and the doctrine
forbids proposing as much as accessing. On the skills side that half is held
structurally by Layer 2, which **evicts** the tool from the registry so it is
never published; there is no Layer-2 equivalent on the MCP side. The partial net
is the EndTurn guard 5h (`testimony_access_proposal`, mika#1960 phase 2 /
mika#2627), which reads the outgoing text — a layer-1.5 net on the wording, not
a structural door. Tracked in mika#2814.

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

  **Deliberately not closed.** The scan is lexical, so any shape that hides the
  literal token from a byte-level match still reaches the CLI. That class is
  wider than encoding tricks, and the measured members are:

  | Shape | Example |
  |---|---|
  | Token splitting | `g""ws gmail …`, `g''ws gmail …`, `gw\s gmail …` |
  | Glob expansion | `/usr/bin/gw[s] gmail …`, `/usr/bin/gw? gmail …` |
  | Variable assembly | `A=g; B=ws; $A$B gmail …` |
  | Encoded payload | `echo <base64> \| base64 -d \| sh` |
  | Renamed / aliased binary | `PATH=/tmp:$PATH gws-alias gmail …` |

  Closing these would require parsing shell grammar, which is an arms race with
  no fixed point — so the scan is scoped to the casual and incidental path,
  which is where observed traffic sits. For every row above, the registry ban
  (L2) and the execute-time guard (L4) still fire at the real tool call, after
  the command has resolved. **This layer narrows the surface; it does not
  eliminate it, and it must not be cited as though it did.**

- **Deliberate false-positive.** The scan is lexical, so a command that merely
  *mentions* `gws` or `gh` on an identifier boundary is refused too — e.g.
  `grep gws /etc/services`. Accepted: `shell-exec` is a command-execution
  surface, and separating "mention" from "invoke" would require parsing the
  shell grammar. Ordinary paths and refs are unaffected (`.github/...`,
  `gh-pages`, `/tmp/gws.log` all pass), because `.` and `-` are excluded from
  the boundary class.

### Known bypass classes (out of scope for mika#1798, tracked separately)

The four-layer defense covers the `run_gws` builtin surface + any future
tool that declares `data_grade = "testimony"`, skill-registered or
MCP-registered. With the `shell-exec` class closed above and the
**MCP-registered** class closed by mika#1959 (§ *Layer 4* point 2 — its
*propose*-side residue is tracked as mika#2814 in § *Vigilance surface*), it
still does NOT cover:

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
- **Propose surface via `send_message`** (named by mika#1960 phase 2,
  **closed by mika#2627**) — the EndTurn guard 5h reads the turn's final
  assistant text only, so a proposal delivered through `send_message` (the
  sole user channel of a silent turn, also reachable from a conversation
  turn and through the #771 send-message boundary exit) escaped it. The
  predicate now runs on that tool's outgoing body before delivery, plus on
  the two scheduling tools that defer one. **Residue, which is not the same
  hole:** a proposal split across two calls so that no single sentence
  carries both layers, and rows scheduled before that deploy. See the
  mika#2627 paragraph above.

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
  - MCP manifest field: `crates/mika-agent/src/mcp/config.rs` §
    `McpServerConfig.data_grade` (mika#1959). Operator-facing reference:
    `docs/mcp.md` § *Déclarer le grade d'un serveur MCP*.
  - MCP → L4 seeding: `crates/mika-agent/src/mcp/mod.rs` §
    `mcp_tool_data_grades` + `McpManager::tool_data_grades`, consumed by
    `crates/mika-agent/src/agent_loop/mod.rs` § `seed_mcp_data_grades` at the
    two sites that publish `mcp.tool_definitions()`.

## Change log

- **2026-10-19** — mika#1959: the MCP dispatch tier came under Layer 4. An MCP
  server manifest carries `data_grade` (absent ⇒ `operational`), joined to each
  namespaced tool and **seeded** into the existing `skill_data_grades` map at
  the two publication sites. `execute_tool`'s body is unchanged — only its
  doc-comment, which asserted the opposite. Moved **MCP-registered testimony
  tools** out of § Known bypass classes; added the MCP row to § Ban list (v1).
  **Correction to this document and to the ticket body:** mika#1959 named three
  grades (`testimony | evidence | reference`) and a default of `reference`;
  `DataGrade` as shipped by PR#1956 is **binary** (`operational` / `testimony`),
  so `reference` reads `operational` here and no third variant was created
  (plan decision D1). An unknown grade string is a **parse failure**, never a
  guessed default. The *propose* half is **not** closed — see § *Vigilance
  surface*, tracked as mika#2814.
- **2026-10-01** — mika#1960 phase 2: the *propose* surface gained a structural
  half. EndTurn guard **5h** (`testimony_access_proposal`) is armed, with its
  single-retry budget, its `_uncorrected` residue event and a regression eval
  under `tests/eval/doctrine_regressions/`. Phase 1 (PR #2620) had shipped the
  predicate with no caller — "disarmed by construction", verifiable by `grep`
  rather than by convention. Layers 2/3/4 are untouched: this adds a net on the
  half Layer 1 held alone, it does not change any access path. § *Vigilance
  surface* is deliberately unchanged — no access path is added, so the axis of
  vigilance does not move.
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

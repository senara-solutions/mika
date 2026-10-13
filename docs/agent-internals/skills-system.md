# Skills System

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


Git-based and local skill distribution via `mika skills install/uninstall/update`. Sources: git URLs, GitHub shorthand, local paths, `file://` URIs. Optional `--link` flag creates absolute symlinks. Four-tier origin: `[built-in]`, `[marketplace]`, `[marketplace/linked]`, `[custom]`. Tracks in `marketplace.lock` (TOML).

**Bundled skill sources (dual path, #598):** `seed_bundled_skills()` seeds two concatenated sources on startup — the hardcoded `BUNDLED_SKILLS` list (10 community-category skills: tmux, shell-exec, web-search, file-reader, self-knowledge, git-ops, google-workspace, github, mcp, browser-control — `include_str!`-embedded from `templates/skills/`) and the directory-sourced `ENTRIES` table generated at build time by `build.rs` walking `<workspace>/skills/bundled/*/`. `all_bundled_skills()` merges both with case-insensitive ENTRIES-wins-on-name-collision semantics. Production `skills/bundled/` contains 22 engine-coupled skills: self-dev, self-dev-callback (#1106), self-dev-iterate, self-dev-webhook-qa, self-dev-webhook-ci, self-dev-webhook-ready-label (#1106), qa-review, qa-review-build-callback, skill-review, dev-pilot, build-mika, deploy-mika, permission-policy, agents-teams, address-pr-comments, resolve-pr-conflicts, self-check, dev-groom (#845), dev-handsoff (#967), mika-arch-groom-ticket (#811), mika-arch-groom-milestone (#879), mika-arch-second-review (#811). Engine-coupled = correctness depends on staying in lockstep with Rust engine code (tool schemas, callback contracts, prompt-discipline rules encoded as Rust guards). `is_bundled_skill()` consults both sources. Trust-critical classification (`TRUST_CRITICAL_SKILLS`) stays hardcoded regardless of source. Shared build-time discovery helper lives at `build_support/bundled_skills_discover.rs` and is consumed by both `build.rs` and integration tests via `#[path = ...]` mod attributes. Directories prefixed with `_` (e.g., `_shared/`) are excluded from discovery — convention-reserved for shared support libraries. `_shared/dispatch-lib.sh` (#893, #932) provides the shared claude-pilot dispatch plumbing. Each dispatch skill owns its own tool: dev-pilot registers `run_claude_pilot` (`skill: ["dev-pilot"]`, entry `/mika`), dev-groom registers `run_claude_pilot_groom` (`skill: ["dev-groom"]`, entry `/mika-groom-ticket`). Both handlers source the shared lib; the lib derives the entry command from `$SKILL` via a `case` switch. Restored from prompt-only design in mika#1173 after 5 LLM-dependency regressions.

**Dependency resolution:** BFS with cycle detection (max depth 10), same-source sibling resolution. `--link` propagates to deps from same source. `find_orphaned_deps()` + `--remove-deps` for cleanup.

**Per-provider and per-model variants:** Two-level directory hierarchy: `{provider}/` and `{provider}/{model}/`. `resolve_prompt(provider, model)` returns `ResolvedPrompt` with four-step fallback: hand-authored model variant -> generated model variant -> generated canonical variant -> root `system_prompt.md`.

**Per-skill LLM override:** DB-only via `skill_overrides` table (schema v20). `[llm]` section no longer supported in `skill.toml` (#504). `resolve_skill_llm_override()` constructs per-skill `LlmProvider`. **AlwaysOn + DB-override carve-out (mika#1011):** `AlwaysOn` skills with DB-sourced LLM overrides (`LlmOverride.from_db_override = true`, set by `apply_overrides()`) also qualify for override resolution — this ensures operator intent via `mika skills llm set` fires on autonomous-loop webhook turns where no keyword match exists. The #463 protection against `skill.toml [llm]` hijacks is preserved (developer-time overrides have `from_db_override = false`).

**Identity-driven skill allowlist (#811, #815):** All four well-known agents declare `[skills].allowlist` in `identity.toml` to own their skill set via identity rather than `skill_overrides` DB rows. mika-arch was the first (#811); mika-dev, mika-qa, and mika-relay followed in #815 (D2 cross-cutting). New bundled skills are denied by default unless explicitly added to an agent's allowlist — see the root `CLAUDE.md` § "Adding a New Bundled Skill" for the checklist. `SkillsIdentityConfig` in `prompt.rs` deserializes the `[skills]` block with `allowlist: Option<Vec<String>>`. `SkillRegistry::apply_identity_allowlist()` runs as Phase -1 before `apply_overrides()` — evicts all skills NOT in the allowlist (case-insensitive), following the same `retain()` + `DisabledSkill` pattern as Phase 0. Warns on allowlisted names not found in the registry. Empty or absent allowlist = no-op (all skills active). Wired into all skill-loading paths: server init, hot-reload (handlers.rs, a2a.rs), team engine, delegate_task, list_skills tool, and CLI (chat.rs, ask.rs, skills.rs). A one-time data migration (`migrate_well_known_to_identity_allowlist`, guarded by `schema_meta` marker `well_known_d2_migration_v1`) deletes stale `skill_overrides` denylist rows for the three migrated agents on first startup after deploy. **Default personal/customer agent (#1596):** the same "absent allowlist = no-op = all skills active" rule made fresh agents default-permissive — a personal `mika` provisioned via `bootstrap_fresh_install` (the family-rollout path when `add-customer.sh` runs without `--identity-dir`) loaded the engineering skills (`dev-pilot`, `dev-groom`, `mika-arch-*`, `qa-review*`, `self-dev*`) and leaked their internal `Disposition:`/`Verdict:` contract into user-facing replies. The default identity now ships a narrow operator-assistant allowlist — see `DEFAULT_AGENT_SKILL_ALLOWLIST` + `DEFAULT_IDENTITY` in `crates/mika-common/src/home.rs` (the personal-agent counterpart to the well-known-agent allowlists here; keep the two in sync). An explicit `[skills]` block in a provisioned `identity.toml` still overrides it.

**Boot-time tier guard (mika#1962):** `server::tier_guard::assert_family_tier_env_consistency(home_dir, tier)` refuses server startup when an agent is family-provisioned on disk while the process tier does not itself write the family templates. **Both gates ask that through `AgentTier::expects_family_provisioning()` — an exhaustive match — never `tier == AgentTier::Family` (mika#2023 M2).** The equality comparison they used to carry was a mine the compiler could not see: `AgentTier::Champion` is provisioned with the family templates on both axes, so introducing that variant under the old predicate would have made the boot guard `bail!` for the entire champion population, and the lazy-path gate below decline every champion at its first `/send`, with no compile error anywhere to announce either. **Unchanged blind spot, and it is the one that matters for launch:** the guard detects *family* provisioning only. A champion tenant bootstrapped before mika-cloud#209 (2026-08-28) carries the **operator** allowlist on disk, the guard does not fire, and the agent starts silently with `shell-exec`/`tmux`/`git-ops`/`github`. Nothing in mika#2023 retrofits that population — it is a re-provisioning gesture, and verifying it is a blocking gate before any champion account is opened. It is called from `run_server` immediately after `home::migrate_to_multi_agent` (which is what creates `agents/<name>/` — scanning earlier would read a pre-migration layout and find nothing) and before any agent is initialized. `run_server` has one production caller and zero test callers, so placing the guard inside it means a future binary entry point inherits it rather than having to remember to call it.

**Coverage set.** The guard enumerates via `servable_agent_names`, the **union** of `agent::list_agents` (which requires `config.toml`) and any `agents/*/` directory carrying an `identity.toml`. The two predicates disagree by design — `AppState::resolve_agent` gates only on `identity.toml` and `Settings::load_for_agent` adds the per-agent `config.toml` with `.required(false)` — so an agent home with identity but no config is fully servable while being invisible to `list_agents`. The guard must see what the server will serve.

**Post-boot agents.** The boot guard scans once. `resolve_agent`'s lazy-construction slow path (mika#1399) builds an `AgentState` at *request* time for agents that appeared since, and re-reads the tier from the server's env — so it calls the per-agent form, `tier_guard::check_agent_tier_consistency`, before constructing, and declines the agent (`error!` + `None`) on mismatch rather than aborting the process. The reachable drift here is a **two-process** disagreement, not a mid-runtime env mutation: `mika agents create <name>` run from a shell exporting `MIKA_AGENT_TIER=family` writes family templates to disk (`home::bootstrap` reads that shell's env), and the server — started without the var — would otherwise serve that family persona under operator semantics on first `/send`.

Detection is two independent OR-combined axes, both living in `mika-common::home` next to the constants they read: `soul_has_family_marker` (the `FAMILY_SOUL_MARKER` sentinel, a **tail** marker — see `mika-common/CLAUDE.md` for why it must not be the first line) and `identity_allowlist_matches_family` (set comparison against `FAMILY_AGENT_SKILL_ALLOWLIST`). Two axes because neither suffices alone — the marker is not retroactive, so **axis 2 is the load-bearing detector for every family agent provisioned before mika#1962**; and the allowlist can be legitimately hand-edited, which axis 1 then covers. Both would have to be broken to hide family provisioning. The `tier` is a parameter, not read from the env inside the guard, so the guard is a pure function of (disk state, tier) and its tests never mutate process-global env state.

Companion cache: `AgentState.tier` (`server/state.rs`) holds `AgentTier::from_env()` read once at `init_agent`, and all four production `ToolContext` construction sites read it via their params structs (`AgentParams`/`SilentAgentParams`/`TeamAgentParams` each carry a `tier` field) rather than calling `from_env()` per turn. `TaskDispatcher.tier` and `TeamEngine.tier` thread the same cached value into silent and team turns. **Structural gate:** `grep -rn 'AgentTier::from_env()' crates/mika-agent/src/` must return no production hit outside the tier-resolution surface: `server/mod.rs` (`init_agent` populating the cache, plus the boot-guard callsite), `server/state.rs` (the lazy-resolve guard callsite), and `server/tier_guard.rs` (message coherence). A hit in `agent_loop/`, `teams/`, `task_engine/`, or `server/investigate.rs` means a per-turn consumer stopped reading the cache. See `todos/1962-injection-verification.md` § I3 for why this gate is structural rather than a unit test.

**Identity-driven tool denylist (#811):** Mirror of the skill allowlist for built-in tools. Well-known agents declare `[tools].disabled` in `identity.toml` listing tools that must not appear in their LLM tool array. `ToolsIdentityConfig` in `prompt.rs` parses the section. `agent::apply_agent_tool_visibility()` is the named filter hook — applied inside `inject_skills_and_resolve_tools` at the LLM-tool-array assembly site, before the tool defs are converted to `LlmToolDefinition`. The model never sees disabled tools, cannot call them, cannot be prompt-injected into trying. The shared `Arc<ToolRegistry>` is unchanged — filtering is per-agent at the presentation layer. mika-arch denies platform-mutational tools (skill mutations, config writes, file writes, task mutations, PR merge, cross-agent invocation, agent/team mutations) while keeping `send_message` and memory writes (`update_core_memory`, `store_fact`, `update_fact`) allowed — both are agent-scoped self-state, constitutive of being an agent, not platform side-effects (see `docs/architecture/review-guide.md` § Orthogonality). Future migration: extend to support `[tools].allowlist` for the symmetric well-known-agent shape.

**Fail-closed identity for well-known agents (#811):** `prompt::load_identity()` distinguishes well-known from user-defined agents on parse failure. For well-known agents (detected by matching the home_dir's last component against `find_well_known_agent`), a malformed `identity.toml` returns a fail-closed `Identity` with a sentinel allowlist (`["__fail_closed_no_skills__"]`) that matches no real skill, plus the full `MIKA_ARCH_DISABLED_TOOLS` denylist. The agent is effectively neutered until the operator fixes the file. For user-defined agents, parse failure logs `error!` and falls back to `Identity::default()` (current behavior — no security contract to preserve). The discrimination happens inside `parse_identity_or_fail_closed()`, so callers don't need to know the agent's well-known status.

**Absence is not permission (mika#2027):** the same `load_identity()` used to answer a *missing* `identity.toml` with `Identity::default()`, whose `skills.allowlist` is `None` — and `apply_identity_allowlist` treats `None` as a no-op. So **deleting an agent's identity file made it MORE permissive**, not less: every bundled skill active, `shell-exec` / `git-ops` / `github` / `tmux` included. mika#1596 closed this class one step earlier (a provisioned agent with no allowlist); the file-entirely-absent case stayed open, and was reachable by a tier remediation that looks correct.

**Deleting it is permanent, which is why the widening was not self-healing.** Three gates each decline to rewrite the file: `bootstrap_fresh_install` is guarded by `home::is_initialized`, true for any tenant whose `data/mika.db` exists; `home::bootstrap` would rewrite it via `write_default_if_missing` but nothing calls it for an already-provisioned agent (the CLI's `ensure_initialized_for_agent` tests for `config.toml`, not `identity.toml`); and for a well-known agent, startup takes the existing-agent branch into `reconcile_well_known_identity`, which reads the on-disk file first and returns early when the read fails. Measured on a live tenant 2026-08-28: delete, full restart, still absent — ~2 min of exposure.

A read failure now returns the **same fail-closed sentinel as the malformed path**, and does so **universally** — well-known and user-defined agents alike (F1). An agent that needs skills must carry an explicit `identity.toml`, even a minimal one; that regression is the posture, not a side effect. **Sentinel rather than refusing to boot (F2, a deliberate divergence from the architect's first-pass suggestion):** mika-spirit serves every agent from one process, so a hard refusal would take the healthy agents down with the broken one, and it would contradict mika#1962's `tier_guard`, which explicitly tolerates a missing persona file.

**Two things "fail-closed" does NOT mean here, both worth knowing before relying on it.** (a) The denylist is `MIKA_ARCH_DISABLED_TOOLS`, which deliberately keeps `send_message` and the agent-scoped memory writes (`update_core_memory`, `store_fact`, `update_fact`) — #811 counts those as constitutive of being an agent, not platform side effects. So a neutered agent can still write its own core memory, which is re-injected into every later system prompt. Whether the fail-closed path wants a stricter denylist than mika-arch's steady-state one is a genuine open question and a separate change: the two share one constant, so tightening it here would also tighten mika-arch. (b) The sentinel is not confined to memory — `startup::seed_bundled_skills_if_needed` hands it to `materialize_agent_skill_links`, whose pass 2 removes the symlink of every de-allowlisted bundled skill, so a fail-closed start empties `<agent_home>/skills/` of its library symlinks. Recoverable (re-materialized on the first valid start) and scoped (marketplace and `--copy-managed` dirs untouched), but a real disk effect — including on a *transient* `identity_toml_unreadable`.

**One deliberate asymmetry remains, and it is narrower than it looks:** a *user-defined* agent fails closed on an absent file but still falls back to `Identity::default()` on a *malformed* one (the #811 behaviour above, unchanged). mika#2027 scopes itself to the absent case; widening the malformed path is a separate decision with a wider blast radius. The consequence to know when reading logs: a user-defined agent with a broken `identity.toml` can look healthy while running fully permissive — `identity_toml_malformed` is the only signal.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`), three names because three remediations: `identity_toml_absent` (`ErrorKind::NotFound` — re-provision from the tier template), `identity_toml_unreadable` (present but permissions/I-O — fix readability, do NOT overwrite), `identity_toml_malformed` (fix the TOML). Each carries `agent` and `path`. Steady state is zero lines; any hit is an agent running with no skills (or, in the asymmetric case above, with all of them). Runbook: `docs/operator/agent-identity-reprovision.md`. Regression gates: `prompt::tests::mika2027_*` (the sentinel is produced, on both the sync and async loaders) and `skills::tests::mika2027_fail_closed_sentinel_evicts_every_skill` (it bites) — the pair is what makes "absent → zero skill" a fact rather than a convention. The tooled re-provision path is mika#2230. **Consequence for test authors:** a temp agent home must now write an `identity.toml` — `EvalHarness::build` and the `list_skills` / `toggle_skill` fixtures do, with a `name`/`emoji`-only file that parses to exactly `Identity::default()`. A fixture without one is not "the default agent", it is a neutered one.

**Skill enabled state:** DB-backed via `skill_overrides.enabled` column (schema v24, #629). Tri-state: `NULL` = default (enabled), `0` = disabled, `1` = explicitly enabled. `apply_overrides()` evicts disabled skills from `SkillRegistry.entries` into `disabled: Vec<DisabledSkill>` before applying `always_on`/LLM overrides. `enabled=false` always wins over `always_on=true` and over identity allowlist (DB override at Phase 0 runs after identity allowlist at Phase -1). `toggle_skill` agent tool and CLI `mika skills enable/disable` write to DB. Legacy `.disabled` marker files are migrated to DB rows on startup via `migrate_disabled_markers()` (one-shot, idempotent, fail-open). No match-time filter — disabled skills are evicted before matching (#630).

**Per-turn skill restriction over A2A (mika#2363).** `message/send` and
`message/stream` read an optional request-metadata key,
`mika.only_skills` (`mika_a2a::params::ONLY_SKILLS_KEY`), holding an array of
skill names. When present and non-empty, `run_a2a_agent` restricts **this turn's**
registry to those names via `SkillRegistry::apply_only_skills`. `mika ask
--only-skill <name>` (repeatable) writes it; `_arch_ask` in `dispatch-lib.sh`
passes the pass it is running.

**The measurement.** mika-arch declares three `always_on` skills —
`mika-arch-groom-ticket` (16 269 B), `mika-arch-second-review` (14 290 B),
`mika-arch-groom-milestone` (9 239 B) — and a turn runs exactly **one** pass. So
**23 529 B, 59 % of the skill portion of every architect system prompt, described
two tasks the turn was not doing**. Declaring the pass removes 23.5–30.6 KB
depending on which one it is. That saving is unconditional and costs no
correctness: nothing is summarised, truncated or elided — the reviewed plan, the
history window (mika#2295/#2330) and the user message are untouched.

**Strictly subtractive, and that is the whole safety argument.**
`apply_only_skills` computes the complement of the named set and delegates to
`apply_transient_disable`; it **never** calls `apply_transient_always_on`. A
skill named here that would not otherwise have been active is not resurrected, so
the field can only ever narrow a turn. That matters because `/a2a/{agent}` is
reachable by any authenticated caller: the additive half of the channel would let
one force a skill into an agent's prompt, and it buys nothing here — the skill
`_arch_ask` needs is already `always_on`. The additive half of mika#1727's missing
config channel therefore stays deferred, with that reason.

**Per-turn, never on the cache (R4).** The restriction is `&mut`, and the registry
lives behind an `Arc` on `AgentState` that concurrent turns share. `run_a2a_agent`
clones it, restricts the clone, and never writes it back — an empty request does
not even clone. Removing that clone would break no assertion: the declaring turn
would still be correct and the *next* turn would silently inherit its restriction.
Hence a lexical guard,
`server::a2a::tests::mika2363_the_restriction_is_applied_to_a_clone_and_never_written_back`.

**Fail-soft on the wire.** Key absent, `null`, not an array, an empty array — all
mean "no restriction", so a caller that declares nothing gets the pre-mika#2363
turn byte for byte. Non-string entries inside a valid array are dropped
individually rather than discarding the declaration, which would silently restore
the triple injection. A name the registry does not carry keeps nothing: the turn
ends with zero skills, which is loud (`active_skill_count = 0` on
`system_prompt_assembled`, no output-contract guard armed) and deliberately not
softened into a no-op.

**What it tightens, and how to disarm it.** `collect_required_suffix_lines` unions
the lines of every `AlwaysOn`-matched skill, so an *unrestricted* architect turn
accepted five lines — a first pass could satisfy the suffix-line guard with
`Verdict: GROOMED`, a contract `_parse_disposition` does not read and reports as
`UNPARSED`. A declaring turn accepts only its own pass's lines. This is a
tightening: a model that leaned on the tolerance now costs one corrective
re-prompt instead of passing. Watch `UNPARSED` counts after deploy; to disarm,
delete `--only-skill "$skill"` from `_arch_ask` — shell only, no binary redeploy.
`escalate_unattested_disposition` (mika#2037) is unaffected: it escalates within
the withdrawn line's own family, and each arch skill declares its own family's
`ESCALATE` (pinned by
`agent_loop::tests::mika2363_escalation_still_has_a_declared_target_under_restriction`).

**The trap, pinned.** Do **not** flip the three arch `always_on` flags to `false`
now that the pass is declared. The channel cannot activate anything, and keyword
routing cannot stand in — `mika-arch-groom-ticket` triggers on `plan review` and
`mika-arch-second-review` on `second pass`, both of which occur in the body of a
plan, so the content under review would select the skill that reviews it (T2b).
With the flags off, an architect turn carries no prompt at all: no
`required_suffix_lines`, no `required_finding_list_prefixes`, no review-anchor
contract — and nothing fails loudly, `_parse_disposition` just returns `UNPARSED`
on every groom. `tests/only_skills_arch_pass_2363.rs` refuses the flip with that
reasoning in its failure message.

**Observability, no new instrumentation.** `a2a_only_skills_applied` (INFO, fields
`agent`, `task_id`, `requested`, `kept`, `evicted_count`, `unknown_count`) and
`only_skills_unknown_name` (WARN). The reduction itself is attested by the two
surfaces that already existed: `system_prompt_assembled.active_skill_count` /
`.per_skill_bytes` (mika#1217) and `turn_usage.system_prompt_bytes` (mika#1889).
**Both must move together** — one moving alone means the measurement is wrong, not
the system.

### Per-turn model override over A2A (mika#2304)

Sister key of `mika.only_skills`, same channel, **opposite failure policy** — and
the asymmetry is the design decision, not an oversight.

`message/send` and `message/stream` read `mika.model_override`
(`mika_a2a::params::MODEL_OVERRIDE_KEY`), the raw string the operator typed.
`resolve_caller_model_override` resolves it against **this** agent's
`llm_provider` (alias → conditional prefix strip → API-key check, all in
`mika_common::llm::model_override`, the single site `mika-cli` also calls) and
builds a provider for the turn. `run_a2a_agent` passes it as `AgentParams.llm`
and sets `caller_model_override: true`, which makes `resolve_skill_llm_override`
stand down — an operator running a provider pre-flight must not have their model
replaced by a skill's `[llm]` section.

**Reading is fail-soft; applying is fail-CLOSED.** Key absent, `null`,
non-string, blank → no override, and the turn is the pre-mika#2304 turn byte for
byte. But a **declared** override this agent cannot serve refuses the request,
before the task row exists and before the agent lock is taken, as a JSON-RPC
`INVALID_PARAMS` naming the model and the provider. It is never degraded to "no
override": a skill restriction silently dropped makes a turn *wider*, which is
visible and falsifies no measurement; a model silently dropped makes the
measurement **wrong while producing a plausible answer** — the founding defect.

**The key check precedes the construction, and that order is load-bearing.**
`create_provider_with_budget` routes the ten OpenAI-compatible variants —
OpenRouter among them, the rail the founding measurement ran on — to
`OpenAiCompatibleProvider::new`, which returns no `Result` and never consults
`api_key`. A missing key therefore *succeeds* at construction, so the refusal has
to be **posed**, never hoped for. Pinned by
`server::a2a::tests::mika2304_the_key_is_checked_before_the_provider_is_built`. What
no layer can check before the call is whether the provider serves that model id:
an unknown id fails on the provider's own 400/404, already fail-closed, with
nothing to write.

**The attestation is taken where the turn was served, not where it was handed
in.** `AgentOutput.effective_model` is filled in `run_agent_inner` from
`effective_llm` — *after* the per-skill recompute — and `handle_message_send`
stamps it on `Task.metadata` under `mika.effective_model` at the mika#2270
intervention point. Reading it in `a2a.rs` off `agent_state.llm` would compile and
be wrong on exactly the per-skill population: a field asserting a model that did
not run, wearing the authority of a server attestation. Written on **every** turn,
override or not, so the client can read absence as "this server did not say" and
refuse to print a local value.

**Scope, stated rather than assumed:** synchronous `message/send` — the path both
doors of `mika ask` take. `message/stream` refuses an unserviceable override
identically (same key, same policy) but stamps nothing: it serves events, not a
rebuilt `Task`. `returnImmediately` runs no turn, so there is nothing to attest.
Both fall in the honest "not attested" population.

**Operator grep signals** (`$MIKA_SPIRIT_LOG_FILE`): `a2a_model_override_applied`
(INFO — a turn ran under a caller's model; expected rare, correlated with
pre-flight campaigns; a sustained flow means an automated caller is imposing one
and deserves identifying) and `a2a_model_override_refused` (WARN — **expected
regime: zero lines**; each one names a model or a missing API key, i.e. an
operator typo or an unconfigured agent, not a defect of the channel).

### Per-turn failure-class attestation over A2A (mika#2522)

The **fourth** `mika.*` response key, beside `mika.effective_model` (mika#2304),
`mika.session_isolated_applied` (mika#1951) and `mika.run_usage` (mika#1883), and
stamped at the same mika#2270 intervention point by `stamp_turn_failure_class`.

**What it closes.** `handle_message_send` serves a failed turn as a `failed` Task
"with nothing to say" — correct for rendering, and it threw away the one fact the
caller needs to decide whether to retry. So `mika ask` classed **every** `failed`
as `Contract`, exited `1`, and `_arch_ask_with_retry` (which replays on `75`
alone) never armed. Measured 2026-09-24: 11 OpenRouter `body read failed
mid-stream` cuts, **5 grooms of mika#2515 lost**. The server *held* the class and
did not transmit it.

**The class is read where the variant still exists, and nowhere else.**
`run_a2a_agent` used to end on `Err(e.to_string())` — after that line the
`LlmError` variant is gone and the only remaining option is a `contains()` on a
rendered sentence, which mika#2179 and mika#2289 both refuse in as many words.
`A2aTurnFailure::from_anyhow` therefore reads it through
`mika_common::llm::error::classify_anyhow_error` (`downcast_ref` over the whole
cause chain, the function's **third** consumer, no fourth copy of those four
lines) and the class travels **inside** `TurnText::LoopFailed { class }` rather
than in a parallel local: the `match` on `turn_text` is the one place that knows
a turn failed, and carrying it there is what stops a later editor from handling
that branch without it. `A2aTurnFailure`'s `Display` renders the message alone,
so every existing `%e` site — the `message/stream` error arm included — prints
byte for byte what it printed before.

**Written unconditionally on the failure branch, and on no other.** Its absence
therefore means *"this server attested nothing"* — a binary predating mika#2522,
`message/stream`, `returnImmediately` — and **never** "the class is `contract`".
Same asymmetry, and the same reason, as `stamp_run_usage` against its two
unconditional sisters: this one is a measurement, not the answer to a flag. The
client reads it fail-closed, so all three of those populations keep the pre-fix
exit code exactly (`crates/mika-cli/CLAUDE.md` and the root `CLAUDE.md` § *Un tour
tué par le transport* carry the reading table).

**`message/stream` is out of scope and that is stated rather than implied.** It
has the same `Err` arm but renders no synchronous `Task` to stamp, and `_arch_ask`
does not take that door (`mika ask` goes through `send_message_to_agent`, i.e.
`message/send`). It falls into the "not attested" population already named by
mika#2304 and mika#1883 — a bound inherited, not widened.

**Operator surfaces.** `a2a_turn_failed` (WARN + an `audit_events` row of the same
name — `target_key` = the model, `after_value` = the class), written by
`report_turn_failure` and by nothing else: **SOLE WRITER**, pinned by
`canonical_tokens::tests::mika2522_the_turn_failure_name_has_a_single_writer`
with an allowlist shipped empty and an anti-vacuity assertion. That property is
what makes `GROUP BY target_key, after_value` an exact answer to *which model
loses turns, and on which class* — the AC2 deliverable — rather than a number two
writers can disagree about. The model comes off `AgentState.budget_record`
(`effective_model` is `None` on this branch, the turn produced no `AgentOutput`),
and an unreadable provider degrades the label to `unknown` rather than dropping
the row (`repo=unknown` motif, mika#2496): a turn lost on a model we cannot name
is still a turn lost. Sibling `a2a_turn_failed_audit_failed` (WARN, expected
empty) marks a WARN that landed while its audit row did not.

**Transient enable/disable overrides (#682):** Two methods handle per-invocation skill overrides, called after `apply_overrides()` (disable first, enable second — matches Phase 0/1 pattern): (1) `SkillRegistry::apply_transient_disable(skill_names)` evicts named skills from the registry entirely for a single invocation. Returns `TransientDisableResult` with `not_found` list. (2) `SkillRegistry::apply_transient_always_on(skill_names)` sets `always_on = true` on named skills. Returns `TransientOverrideResult` with separate `disabled` and `not_found` lists. Cannot resurrect disabled (evicted) or skipped skills. Neither is persisted.

**Who calls them today, and the CLI does not (mika#1727, mika#1883).** This paragraph used to read "Used by `mika ask --disable-skill`" / "`--enable-skill`", and that has been false since `mika ask` became a thin client — those two flags mutate the **CLI process's** registry, which is not where the turn runs. Their arg-level conflict check survives (a skill named in both is still a hard error before any registry op) and mika#1883 makes the inertia audible on stderr, but neither flag reaches these methods. The live caller of `apply_transient_disable` is `apply_only_skills` (mika#2363), server-side, by complement: `--only-skill` is the one selection flag that travels (`mika.only_skills`), and it is **strictly subtractive**, which is why it delegates here and never to `apply_transient_always_on`. That second method stays reachable from the DB-override path only; the additive half of the config channel is deliberately not exposed on `/a2a/{agent}` — it would let any authenticated caller force one of the agent's skills to `always_on`.

**Oversized prompt handling (#630):** Skills with prompts exceeding their size limit are hard-skipped at scan time (pushed to `ScanResult.skipped`) regardless of `always_on` status. This prevents zombie skills with tools but no prompt context. Tool-only skills (no `system_prompt.md`) are unaffected — they load with an empty prompt via `SnippetLoadResult::Empty`.

**Startup logging:** `SkillRegistry::log_summary()` emits a three-state `DEBUG` line (`loaded=N disabled=N skipped=N`) plus per-skip `WARN` lines. Call after both `apply_overrides()` and `apply_load_safety_check()` for accurate counts.

**Validation:** `validate_skill()` checks name-in-keywords rejection (#510), markdown validation (#511), required_tools references, context types, and `{{key}}` placeholders. **Load-time crash-protection (#530, #1335):** `SkillRegistry::apply_load_safety_check()` runs `validate_skill()` on every loaded skill after `apply_overrides()`. This is NOT the validation gate — CI and `mika skills validate` own change-time validation. This method is a runtime safety net that prevents malformed manifests from crashing the agent. Decision matrix: missing handler/broken tools.json → skip skill entirely; deprecated `[llm]` section/name-in-keywords/invalid markdown → load with warning. Results stored in `validated_warnings` for TUI/CLI display. `is_skip_worthy_failure()` classifies Fail diagnostics.

**Required tools enforcement:** Optional `[constraints]` section with `required_tools`. `collect_required_tools()` computes union across keyword-matched skills only. One retry on EndTurn violation.

**Required suffix-line enforcement (#864):** Optional `[output]` section with `required_suffix_lines`. `collect_required_suffix_lines()` computes union across keyword-matched AND always-on skills (not dependency). One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries.

**Required finding-list enforcement (#901):** Optional `[output]` section with `required_finding_list_prefixes`. `collect_required_finding_list_prefixes()` computes union across keyword-matched AND always-on skills (not dependency). Enforced only on terminal dispositions (ITERATE/ESCALATE) — non-terminal (READY/GROOMED) are exempt. Scan range: message start up to the suffix-line landmark. One retry on EndTurn violation. `validate_skill()` warns on explicitly-empty lists and rejects empty/whitespace entries. Same Warn-not-Fail pattern as suffix-line validation.

**Match-reason conditioning (#463):** `match_skills()` returns `MatchedSkill` wrappers with `MatchReason` (`Keyword`, `AlwaysOn`, `Dependency`). `always_on` skills do not enforce constraints unless the user's message also triggered a keyword.

**Review-target exclusion (#513):** `review_filter::apply_review_filter()` runs after `match_message()` and before `resolve_contexts()` in both conversation and team mode. When `skill-review` is keyword-matched, any other keyword-matched skill whose name appears in the user message (case-insensitive) is excluded from the matched set. This prevents the reviewed skill's prompt from contaminating the review context. `AlwaysOn` and `Dependency` skills are never excluded. Silent mode is unaffected (no keyword matching).

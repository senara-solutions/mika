# Changelog

All notable changes to this project will be documented in this file.
## [v0.10.0](https://github.com/senara-solutions/mika/releases/tag/v0.10.0) — 2026-05-08

### Added

- *(self-dev)* auto-groom ungroomed tickets before dispatch (#996) (#1004)
## [v0.9.1](https://github.com/senara-solutions/mika/releases/tag/v0.9.1) — 2026-05-08

### Fixed

- *(kg/extraction)* per-corpus fair budget allocation (#962) (#1034)

### Investigate

- *(kg)* post-axis-deploy KG sync verification (#1028)
- *(docs)* compound knowledge currency audit (#1029) (#1030)
## [v0.9.0](https://github.com/senara-solutions/mika/releases/tag/v0.9.0) — 2026-05-08

### Added

- *(prompt)* summarizer content reform — factual assertions (#1024) (#1025)
## [v0.8.0](https://github.com/senara-solutions/mika/releases/tag/v0.8.0) — 2026-05-08

### Added

- *(prompt)* silent-mode summary budget cap (mika#1009 Axis 3) (#1022)
## [v0.7.0](https://github.com/senara-solutions/mika/releases/tag/v0.7.0) — 2026-05-07

### Added

- *(prompt)* per-agent [context.summary].inject opt-out flag (#1016) (#1019)
## [v0.6.0](https://github.com/senara-solutions/mika/releases/tag/v0.6.0) — 2026-05-07

### Added

- *(skills)* move enabled state from .disabled markers to DB with eviction (#634)
- *(agent)* auto-create well-known agents (mika-dev, mika-qa) in dev mode (#254) (#642)
- *(agent)* tool-boundary UUID existence validation (#596) (#643)
- add persistence evaluation guard to EndTurn chain (#648) (#649)
- *(config)* migrate all API key fields to SecretString (#673)
- pre-tool context-redundancy check for read tools (#647) (#674)
- *(cli)* add --skill-always-on flag to mika ask; default self-dev to always_on=false (#670) (#675)
- rename --skill-always-on to --enable-skill/--disable-skill (#683)
- raise per-session task creation limit to 25 and make configurable (#685)
- enforce PR review idempotency via tool-call history (#695) (#698)
- *(agent)* reject EndTurn on webhook turns with zero tool calls (#699)
- add calibration rules for issue number verification and UUID hygiene (#701)
- *(server)* structural check_suite.completed(failure) handler (#594) (#703)
- *(gateway)* dead-letter queue + replay CLI for exhausted webhook retries (#704)
- generalize intent-precondition guard (#696 expansion) + add resume keywords (#702) (#706)
- *(gateway)* route pull_request.review_requested to mika-qa (#506) (#707)
- *(skills)* variant management tooling (CLI + server API + dashboard) (#708)
- *(git-ops)* add pull, checkout, and worktree operations (#709)
- *(dispatch)* add blocked-by guard to validate_dispatch_readiness (#713) (#715)
- *(self-dev)* milestone resolver + deploy hooks + topo-sort (#714) (#717)
- KG SQLite schema v25 — entities, relationships, chunks, subject tables (#722)
- domain graph builder — deterministic import from skill manifests and tool registry (#687) (#726)
- *(kg)* lexical graph ingestion — chunk solution docs, embed, link (#689) (#728)
- *(kg)* subject graph extraction — LLM-based NER + fact triples (#690) (#729)
- *(kg)* entity resolution — connect extracted entities to domain graph nodes (#691) (#730)
- *(kg)* query_knowledge_graph — graph traversal tool for agent self-knowledge (#731)
- *(self-knowledge)* upgrade skill with KG-backed agent self-awareness (#692) (#733)
- dedicated mika-relay agent for claude-pilot permission decisions (#721) (#735)
- *(eval)* add optional DI, Settings::test_defaults, callback tests, health_error (#745)
- eval harness Phase 2 — multi-provider matrix and real API calibration (#746)
- *(eval)* golden dataset — 25 end-to-end quality scenarios (#339) (#751)
- KG-backed self-knowledge eval scenarios (#758)
- *(eval)* add KG provider comparison matrix harness (#762) (#780)
- add add_team_member and remove_team_member tools (#283) (#785)
- *(kg)* add MIKA_KG_DOCS_ROOT env var and kg_docs_root config field (#738) (#790)
- *(kg)* schema v27 — docs_root_hash as shared-corpus primary key (#791)
- *(kg)* data migration v26→v27 — coalesce per-agent rows (#787) (#792)
- *(kg)* per-agent docs_root config — stops context pollution from wrong corpus (#778) (#795)
- *(cli)* KG management subcommands — mika kg status / purge / validate (#779) (#796)
- *(kg)* per-agent multi-corpus aggregation (#798) (#808)
- *(mika-arch)* structural read-only enforcement at tool, skill, and identity layers (closes #811) (#813)
- *(gh_read)* add file_view op — read repo files by path (#819)
- *(ask)* add --verbose flag with session_id metadata trailer (#824)
- *(ask)* make --verbose orthogonal to --format (emit metadata in JSON) (#830)
- *(ui+dashboard)* generalize StatusBadge to six-variant API, migrate hand-rolled status pills (#831)
- *(ui+dashboard)* extract <ListRow /> primitive, migrate all list pages (#654) (#832)
- *(ui+dashboard)* extract SelectFilter + AgentFilter, unify agent selection (#655) (#833)
- *(ui+dashboard)* extract state catalog primitives, migrate all pages (#658) (#834)
- *(ui)* dashboard copy feedback — visual confirmation on copy-to-clipboard actions (#835)
- *(ui+dashboard)* add <TimeRangeFilter /> primitive across observability surfaces (#659) (#839)
- *(skills)* add dev-groom bundled skill encoding the two-pass grooming flow (#857)
- *(arch)* core-memory promotion-protocol reflection-pass spec (#872)
- *(engine)* quoted-resource pre-fetch guard (#863) (#882)
- *(skills/mika-arch)* add milestone-grooming capability (#879) (#888)
- *(kg/domain-graph)* expand with concept:cross-repo + concept:infra namespaces (#956)
- *(dashboard)* surface telemetry on Team Runs detail page (#970)
- *(dashboard)* add response content, linked tool calls, and error state to LLM Call detail (#971)
- *(dashboard)* make Core Memory panel actionable (#656) (#972)
- *(dashboard)* add CostTrendChart time-series visualization (#976)
- *(dashboard)* task tree visualization on Dev Runs detail (#661) (#982)
- *(dashboard)* landing/home overview page (#666) (#989)
- *(dashboard)* live-refresh consistency across time-sensitive pages (#662) (#990)
- *(dashboard)* URL-reflect all filter, pagination, and tab state for shareable links (#664) (#992)
- *(dashboard)* cost visibility across surfaces (#667) (#994)
- *(ui)* dashboard accessibility audit with axe-core CI gate (#995)
- attach LLM request/response bodies to Langfuse generation spans (#671) (#998)
- *(dashboard)* expandable LLM response body viewer on LLM Calls page (#672) (#1000)
- *(dashboard)* sync session detail tabs with URL path segments (#676) (#1002)
- *(kg)* emit reattempted_no_match field on kg_resolver_tick.complete (#961) (#1015)

### Changed

- hard-skip oversized skills, split startup log, delete match-time filter (#635)
- *(agent)* rename work_item tools to task vocabulary (#608) (#644)
- *(skills)* rename claude-pilot skill to dev-pilot (#853)
- *(dev-pilot)* delegate branch + worktree derivation to platform scripts (#858)
- *(skills)* factorize dev-groom and dev-pilot into shared dispatch lib (#893) (#898)

### Documentation

- compound audit findings — qa-review on mika-dev + CLI log noise
- compound — engine guards vs prompt rules for agent behavioral invariants
- *(solutions)* Socratic multi-ticket milestone planning pattern (#710)
- KG milestone #14 retrospective + cross-links across per-ticket docs
- *(solutions)* compound — correlation-vs-ownership validator bug class
- institutionalize infra-fix compounding rule (#776) (#777)
- *(solutions)* compound — autonomous-agent operational discipline (2026-04-23)
- *(solutions)* compound — intent signal is not completion signal
- *(solutions)* prompt-vs-tool contract mismatch bug class
- *(operations)* add kg-cross-corpus-drift-recovery runbook
- *(solutions)* grooming-branch callout in issue body required for /mika
- *(architecture)* add review-guide.md (mika-arch v1 Unit 1)
- *(design)* add north-star + luminescent-core rulebook
- *(design)* Dashboard ↔ Stitch reconciliation map (closes #669)
- *(solutions)* mika-arch first dogfood findings (Unit 5 verification)
- *(solutions)* comment events fire autonomous claude-pilot dispatch
- *(architecture)* add agent self-state vs platform side-effects principle
- *(solutions)* compound 4 findings from 2026-04-26 session
- *(ui)* audit Pagination usage, document canonical-primitive enforcement (#663) (#836)
- *(solutions)* post-deploy KG entity audit ritual (#859 evidence)
- *(solutions)* compound two architect-discovered patterns from 2026-04-28 grooming (#860)
- *(solutions)* compound script-drift CI investigation lesson from mika#860 (#865)
- *(solutions)* add Example 6 — author-side cheap-impulse during doc composition (#873)
- *(solutions)* compound dev-pilot handler silent exit-0 pattern (#884)
- *(solutions)* compound post-restart KG extraction-and-resolution audit ritual (#885)
- *(solutions)* compound set -x secret-leak class from mika#903
- *(solutions)* compound operator-side findings recovery from store_fact when mika-arch emits thin
- *(review-guide)* sharpen §7 recursive-self-review carve-out — causation test + memory-shared axis (#921)
- refresh README for accuracy (test count, providers, crates, KG) (#930)
- compound from mika#927 grooming halt — Mode 3 compound doc name emission heuristic
- refresh required-tools-gate-evasion-patterns with Mode 3 adjacent reference
- *(brainstorm)* session continuity across scope types — Option C decided (#973)
- *(947)* preserve mika-arch persistence-meta hallucination as resolved knowledge (#978)
- *(plans)* grooming plan for mika#974 — task_messages parallel narrative table (#977)
- *(solutions)* add two compound learnings from 2026-05-05 session (#979)
- *(kg)* document resolver type-allowlist contract
- *(solutions)* mika-arch pass-1 degenerate recovery via retry-with-fresh-session
- *(audits)* layer 2 vs mempalace temporal triples — reject migration

### Fixed

- *(deploy-mika)* replace cp with install for atomic binary replacement (#621)
- *(self-dev)* add milestone routing, callback awareness, build+deploy close-out (#609) (#626)
- *(self-dev)* milestone callback routing + missing parent_task_id in child creation (#627)
- *(self-dev)* bump max_prompt_size to 49152 — unblock agent loading (#628) (#631)
- *(self-dev)* sort milestone/project issues ascending by number (#633)
- *(agent)* allow metadata-only writes on terminal-state tasks (#637)
- *(skills)* reconcile hot-patched self-dev and qa-review prompts (#638) (#639)
- *(skills)* restore lost prompt hot-patches — run_gh schema discipline + M2/M3/P3 pre-flight (#641)
- *(agent)* structural guard against core_memory mis-access via read_agent_file (#645) (#646)
- required_tools gate recognizes terminal tool failures (#516) (#678)
- *(agent)* detect prose-style tool-call leaks in EndTurn output (#569) (#680)
- *(self-dev)* retry update_task_status via list_tasks on task_not_found (#693) (#694)
- *(agent)* handle chat_id == 0 as structured NoChannel outcome in send_message (#650) (#700)
- *(skills)* remove keyword triggers from skill-review to prevent false positives (#705)
- deduplicate task_id in required array for long-running tool schemas
- use correct GraphQL field `blockedBy` instead of `blockedByIssues` (#719) (#720)
- /mika command reads branch name from issue body callout before deriving
- run_claude_pilot handler reads branch name from issue body callout
- self-heal stale-base + CONFLICTING PRs (#747) (#748)
- ask.rs correlation branch uses unscoped task lookup (#756)
- *(kg)* extraction idempotency + budget guard + fan-out advisories (#757) (#759)
- *(kg)* prevent UTF-8 byte-slice panics — KG resolution fully broken (#764) (#769)
- *(dashboard)* fetch tool calls from API instead of truncated metadata (#744) (#770)
- *(teams)* consolidate team-run callbacks into single user notification (#781)
- *(teams)* orchestrator coverage check for full team distribution (#286) (#782)
- *(config)* MIKA_KG_DOCS_ROOTS env var fails to parse as Vec<PathBuf> (#816)
- *(mika-arch)* drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS (#820)
- *(mika-qa)* session-scope pr_review_posted to prevent duplicate reviews (#822)
- *(mika)* make plan-on-branch load-bearing in implementation + QA (#825)
- *(skills)* unblock bundled skills + add CI gate for oversized prompts (#828)
- *(server)* dashboard reads stale SQLite WAL snapshot — sessions invisible until restart (#636) (#837)
- *(self-dev)* recognize error_max_turns as recover-work verdict, not retry (#840)
- *(self-dev+gateway)* gate dispatch on ready label or direct prompt only (#842)
- *(self-dev+agent)* structural ready-label dispatch guard (#846) (#847)
- *(agent)* replace tokio::time::timeout outer wrapper with Instant deadline (mika#848) (#849)
- *(mika-arch)* apply /ce:review findings (broken citation, missing test, naming, citations) (#867)
- *(agent)* callback turns require terminal actions before EndTurn (#870) (#878)
- *(task-engine)* add reaper for parent self_dev tasks orphaned by failed callbacks (#880)
- *(agent)* required-tools-gate retry produces self-contained final response (#891)
- *(dev-pilot)* inject BASH_XTRACEFD trace for silent-exit-0 diagnosis (#895)
- *(agent)* extend structural verdict handler to cover block[*]/hold[*] dispatch (#889) (#896)
- *(skills/qa-review)* VERDICT-on-top in review body to survive gateway truncation (mika#909, mika#898 incident)
- *(kg/resolver)* add periodic resolver tick to drain Stage-2 backlog (#906) (#909)
- *(self-dev+agent)* require grooming marker before dispatch on ready label (#913)
- *(agent)* add webhook_no_unauthorized_dispatch intent guard (#910) (#914)
- *(gateway)* raise PR-review truncation cap to 16k + reconcile QA verdict contract (#911) (#916)
- *(agent/tool_calls)* redact secret-shaped values before persistence (#908) (#915)
- *(kg/resolver)* accept valid LLM matches via DB-fallback (#874) (#924)
- *(kg/subject_extractor)* brace-matching parser + prompt reinforcement for parse failures (#876) (#925)
- *(kg/cli)* surface all corpora per agent in mika kg status (#877) (#926)
- *(skills)* dev-groom unreachable due to run_claude_pilot tool-name collision (#934)
- *(skills/permission-policy)* allow intra-platform mika ask + grooming gh issue edit/comment (#936)
- *(server)* structural pre-classifier for intra-platform agent dispatch (#937)
- *(mika-arch)* move groom skills to Sonnet 4.6 + deadline-aware retry abort (#941)
- *(server)* pre-classifier parser gap — denies markdown briefs in agent dispatch (#938) (#945)
- *(skills/dev-groom)* replace in-prompt branch slug derivation with canonical script (#950)
- PR-writer must cite plan-doc path in PR body (#951)
- *(kg/resolver)* per-corpus fairness in get_pending_entities (#927) (#954)
- *(mika-arch)* citation-fabrication prompt anchoring (#952) (#957)
- *(task-engine)* add callback watchdog for stale subprocess detection (#959) (#968)
- *(executor)* validate required fields before skill tool execution (#955) (#969)
- *(skills)* harden validate_required_fields against malformed/stale schemas (#987)
- *(dispatch)* auto-skip closed-issue autodispatch instead of stalling loop (#993)
- *(ci)* release-pr workflow idempotency — Class C resolution (#1003)
- *(autonomous-loop)* engine-enforced queue advance for post-callback turns (#991) (#1007)
- *(kg)* single-consumer topology eliminates shared-corpus extractor race (#800) (#1010)
- *(autonomous-loop)* per-skill LLM override fires on webhook turns + deferred-dispatch auto-recovery (#1013)
- *(task-engine)* dispatch retry hygiene — status promotion, action_config, UUID validation (#958) (#1014)
- *(ci)* add release label so release-pr workflow succeeds (#1017)

### Engine

- asserted-unavailability EndTurn guard (#862) (#881)
- required-suffix-line EndTurn guard for skill-declared output contracts (#864) (#883)

### Eval

- grounding + fabrication regression scenarios (#741) (#774)

### Investigate

- *(arch)* mika-arch session-init context-channel leakage (#1009)

### Obs

- *(logging)* document log sink architecture and add `mika logs` command (#986)
## [v0.5.0](https://github.com/senara-solutions/mika/releases/tag/v0.5.0) — 2026-04-17

### Added

- *(tui)* skill visibility and /clear cleanup (#415)
- GitHub App identity and agent infrastructure (#423) (#426)
- *(gateway)* multi-tenant GitHub webhook routing with agent_mapping (#433)
- *(cli)* add --format flag to mika agents list (#435)
- *(dashboard)* link sessions to tasks with bidirectional navigation (#437)
- *(dashboard)* rebuild Dev Run detail page with narrative content (#439)
- add compound doc check to verify-pipeline.sh (#440)
- *(cli)* add mika agents validate and mika teams validate commands (#443)
- [#446] add mika provider and mika model CLI subcommands (#450)
- add --format text|json flag to 9 CLI commands (#448)
- *(skills)* built-in skill-review skill for model-tuned prompt variants (#455)
- *(skills)* add GitHub App auth to skill install git clone (#461)
- *(skills)* per-agent LLM override via DB + unblock linked skill review (#475)
- *(observability)* record resolved prompt variant on llm_calls (#481) (#503)
- *(skills)* skill system quality — remove [llm] from skill.toml, forbid name in keywords, validate markdown (#522)
- *(tui)* silence internal dev questions in TUI inbox mode (#494) (#552)
- *(agent)* structural pull_request_review verdict handler (#524) (#555)
- add dispatch-readiness guard on execute_long_running (#525) (#556)
- *(skills)* validate skills at agent startup, not just on explicit validate command (#530) (#559)
- *(tools)* validate UUID-typed arguments at tool boundary (#563)
- *(gateway)* route pull_request.closed webhook to mika-dev (#564)
- *(server)* add webhook deferral queue for callback sequencing (#528) (#565)
- *(skills)* prevent reviewed skill from being triggered during its own review (#513) (#573)
- *(tools)* report skipped skill count in list_skills output (#334) (#575)
- *(tools)* list_work_items returns status-count summary + filter guidance (#585)
- *(server)* structural check_suite.completed(success) handler (#571) (#587)
- *(agent)* add tasks.type column and create_work_item support (#595) (#597)
- bundle 11 engine-coupled skills + fix run_claude_pilot task_id fabrication (#601)
- *(self-dev)* add milestone and project workflow branches (#603)
- *(skills)* migrate 3 more engine-coupled skills into skills/bundled/ (#604)
- *(skills)* migrate resolve-pr-conflicts and self-check (completes #604 scope) (#605)

### Changed

- split CLAUDE.md into hierarchical per-crate context files (#544)
- *(agent)* source bundled skills from skills/bundled/ at build time (#600)

### Documentation

- relocate skill-review rename plan + cross-repo drift solution from mika-skills#102
- *(adr)* add ADR-008 — GitHub identity separation (#521)
- plan and compound for webhook QA pass entry point (#553) (#554)
- compound — lazy PG pool in tests blocks 30s on CI but RST-fails fast locally (#599)

### Fixed

- *(ci)* add git identity for release tag creation
- add method and path to request error/warn logs (#417)
- surface method/path as top-level JSON log fields, add /version endpoint (#421)
- load per-agent .env for token and credential-helper commands (#427) (#428)
- *(tui)* stop polling /api/v1/dashboard/status every 5s (#429)
- load per-agent .env before constructing Settings (#430) (#432)
- *(tui)* add background task running indicator to footer (#431) (#434)
- /provider and /model commands leave broken config state (#444)
- extract XML tool calls from text responses (#447) (#449)
- provider switch propagates to agent worker with provider prefix (#451) (#452)
- tolerate malformed closing tags in strip_internal_tags (#453) (#454)
- gitignore dashboard/dist to prevent dirty repo after deploy (#456)
- *(tui)* prevent cleared messages from reappearing after /clear
- *(cli)* normalize MODEL_ALIASES to include provider prefix (#460)
- *(skills)* filter resolve_skill_llm_override by MatchReason::Keyword only (#465)
- *(skills)* harden review_skill / write_skill_variant (#470)
- *(skills)* merge write_skill_variant into review_skill (#477) (#478)
- *(skills)* block review_skill from reviewing built-in skills (#482)
- *(agent)* enforce work-item state transition on completion claims (#483) (#484)
- *(work-items)* shallow-merge metadata one level deep (#489) (#491)
- *(tools)* add pr_merge_with_gate CI gate tool (#490) (#493)
- *(tools)* accept 'reason' as alias for 'reasoning' in update_core_memory (#495)
- *(mika-gateway)* add webhook observability and QA verdict contract (#487) (#498)
- *(skills)* fix skill-review tool registration and add trust-critical tier (#499) (#500)
- *(llm)* parse cache token usage from OpenAI-compatible provider responses (#479) (#502)
- *(skills)* check for [skill] section before flagging as legacy format (#508)
- *(server)* generate embeddings during startup backfill (#389) (#509)
- *(skills)* harden skill-review prompt to enforce full inspect→persist→verify cycle (#518)
- *(skills)* inject GH_TOKEN into exec handler child processes (#515 #517) (#520)
- *(agent)* filter required_tools gate against available tool registry (#523)
- *(config)* prefer machine user PAT over GitHub App installation token
- *(openapi)* register handle_task_complete in OpenAPI spec paths (#328) (#538)
- *(agent)* add fabricated action-claim guard (#308) (#543)
- *(skills)* refuse long-running tools when no long_running_ctx available (#537) (#551)
- *(investigate)* handle empty LLM response and fix lock race (#558)
- remove redundant skill names from trigger keywords in builtins
- *(skills)* canonicalize both paths in handler symlink containment check (#560)
- *(skills)* suppress required_tools warning for dependency-provided tools
- *(server)* use per-agent token in verdict handler (#561) (#562)
- *(agent)* allow exec/http skills in callback silent turns (#568)
- *(tui)* tag mika-ask relay messages as internal (#557) (#570)
- *(skills)* prevent webhook handlers from dispatching unrelated backlog work (#583) (#586)
- *(tools)* reject retry metadata writes during active dispatch (#579) (#588)
- resolve skill dependencies in callback_safe_skills (#578) (#591)
- *(agent)* dedup identical tool_use blocks within a single turn (#582) (#607)
- *(agent)* surface gateway non-2xx responses as send_message tool errors (#581) (#612)
- *(gateway)* prevent GitHub webhook from poisoning Telegram chat_id (#580) (#618)
## [v0.4.0](https://github.com/senara-solutions/mika/releases/tag/v0.4.0) — 2026-04-03

### Added

- *(ui)* add semantic color tokens and flexible EmptyState (#393)
- [#381] github_app module — JWT signing, installation token caching (#396)
- *(gateway)* GitHub webhook endpoint — POST /webhook/github (#399)
- *(cli)* HTTPS git push with installation token — credential helper and mika token CLI (#400)

### Fixed

- *(gateway)* remove bot self-event filter — blocks webhook-driven dev loop (#401) (#402)
- *(agent)* give SilentTrigger::Reminder 20-step budget matching Callback (#404)
- *(gateway,agent)* unblock webhook-driven dev loop (#407)
- worktree lefthook install and cost_usd numeric type (#406) (#408)
- *(ci)* switch release-plz to git_only mode for tag-based comparison
- *(ci)* set release = false on all crates except mika-common
- *(ci)* remove version from workspace dep specs for publish = false crates
- *(ci)* declare only mika-common in release-plz.toml
- *(ci)* replace release-plz with git-cliff release workflow
- *(ci)* fix YAML syntax in release workflow
- *(ci)* fix double-v in changelog release URL
- *(ci)* exclude release/* branches from pipeline artifact checks
## [v0.3.2](https://github.com/senara-solutions/mika/releases/tag/v0.3.2) — 2026-04-02

### Added

- *(cli)* add task-id correlation to intermediate long-running skill calls (#367)
- *(dashboard)* add RESTful detail pages for Tasks, LLM Calls, Tool Calls (#368)
- mika-dev auto-retry on fixable QA blocks (#377) (#379)

### Documentation

- update documentation for resume_agent reminder support (#363)
- compound solution for reminder resume_agent dual lifecycle (#363)
- sync crate-local docs for CI docs-sync check (#363)

### Fixed

- add dashboard dist placeholder to release-plz workflow
- add resume_agent action type to reminders (#363)
- address PR review feedback for resume_agent reminders (#363)
- *(ci)* fix release binary builds and clean up release naming (#373)
- *(ci)* add release = true to restore release PR creation after #373
- *(ci)* add git_only = true to fix release PR creation
- *(ci)* restore publish = false to match Cargo.toml declarations
- *(ci)* disable cargo package verification in release-plz
- *(ci)* commit dashboard dist for embedded serving and release-plz
- *(ci)* skip cargo package verification in release-plz
- *(ci)* exclude mika-agent from release-plz packaging
- *(agent)* raise callback max_steps to 20 with continuation turn fallback (#378)
- persist work item metadata at callback time (#376) (#384)
- use build.rs + OUT_DIR for dashboard asset embedding (#387)
- clear GH_TOKEN identity separation — active removal + defense-in-depth scrub (#388)
- *(ci)* use pinned Rust toolchain in release-plz workflow (#394)
- *(ci)* revert release-plz to crates.io comparison mode
- *(ci)* remove dist from dashboard/.gitignore
## [v0.3.1](https://github.com/senara-solutions/mika/releases/tag/v0.3.1) — 2026-03-31

### Added

- *(gateway)* add request logging middleware with health probe filtering

### Changed

- *(gateway)* extract shared transient error message into constants

### Documentation

- add solution doc for duplicated string extraction pattern
- document gateway request logging middleware in CLAUDE.md
- add solution doc for gateway request logging middleware

### Fixed

- *(ci)* disable git tag/release for mika-a2a to prevent shared tag collision
- *(gateway)* match on_response log level to span level for health probes
- remove MIKA_INVESTIGATE_GITHUB_TOKEN fallback from agent_github_token()
- remove remaining fallback references in config field doc and reference table
## [v0.3.0](https://github.com/senara-solutions/mika/releases/tag/v0.3.0) — 2026-03-30

### Added

- *(skill)* add Google Workspace builtin skill via gws CLI (#75)
- delegated agent Telegram delivery, agent identification, and reply routing
- add --format text|json flag to mika ask
- team workspace restructure and CLI unification
- strengthen trace_id structural linkage (schema v11)
- redesign core memory widget with per-block token bars and edit budget (#156)
- add MIKA_LOG_FORMAT env var for human-readable server/gateway logs
- strip file paths, line numbers, and targets from pretty log output
- custom compact log formatter with startup banner for pretty mode
- *(server)* add paginated tasks and team-runs dashboard API endpoints
- *(dashboard)* add Tasks and Team Runs pages with cross-linking
- add LlmProvider trait and multi-provider abstractions
- [#71] refactor all consumers to use LlmProvider trait
- [#178] dashboard markdown rendering & session lifecycle fixes
- add public documentation site with Starlight
- [#182] add browser control via Playwright MCP integration
- add guided wizards for agent and team creation
- add --model flag to `mika ask` and `mika chat` for one-shot LLM override
- extract shared UI components into @senara-solutions/ui package
- add dev:dashboard convenience script
- add --last-run flag and enhance teams log output
- *(skills)* raise prompt snippet size limit and make it configurable per-skill
- [#196] show previous run context in TUI when using --last-run or --run-id
- [#198] embed dashboard SPA in mika-server binary
- *(skills)* add local source support for mika skills install with --link mode
- add skill dependency resolution at install time (#216)
- *(a2a)* implement A2A protocol v0.3 for agent-to-agent communication
- tmux-aware stop/restart in Makefile deploy targets
- *(llm)* add first-class MiniMax, Qwen, and Kimi provider prefixes
- enforce pipeline artifacts with verification before PR creation
- *(oauth)* add Anthropic OAuth PKCE token exchange (#232)
- *(db)* schema v14 — add metadata column to tasks table
- claude-pilot dashboard integration — structured run metadata + Dev Runs page
- *(llm)* per-provider LLM configuration (#239)
- *(tui)* add /provider slash command (#239)
- add MiniMax as first-class provider (#239)
- add Kimi and Qwen as first-class providers (#239)
- implement model-level skill variant resolution
- *(skills)* add [llm] section to skill.toml for per-skill provider/model override
- *(tools)* add smart work item status transitions (#257)
- *(agent)* promote delegate session channel_type from "system" to "delegate"
- *(agent)* add proactive task health awareness and self-knowledge of task lifecycles
- add runtime observability — LLM calls, tool calls, skills loading
- add trace_id filtering and step column to telemetry observability
- add deterministic skill context injection via engine-owned pre-fetch
- [#289] add MIKA_GITHUB_TOKEN for agent GitHub operations
- *(skills)* add git-ops builtin skill for git maintenance operations
- *(agent)* add context priority semantics and search_tool_history tool
- *(agent)* inject active work items into callback turn context
- *(agent)* surface parent_task_id in callback framing and simplify to generic routing
- *(gateway)* return user-friendly message when agent is offline
- [#329] add agent eval & testing harness with MockLlmProvider

### Changed

- extract shared CLI helpers and fix flag=value bypass
- remove MIKA_GOOGLE_TOKEN, use native gws keyring auth
- remove builtin calendar skill
- remove redundant telegram_chat_id from TeamAgentParams
- resolve code review findings (670-673)
- address review findings — docs, dispatcher helper, comments
- [#71] rename claude_model/claude_max_tokens to llm_model/llm_max_tokens
- rename --session to --session-id and --parent-task to --parent-task-id
- **BREAKING** unify LLM API key — remove MIKA_ANTHROPIC_API_KEY
- **BREAKING** convert all timestamps from Unix i64 to ISO 8601 TEXT strings
- address review findings for failed callback delivery
- address code review findings for embedded dashboard
- *(a2a)* replace parallel a2a tables with orthogonal persistence
- *(a2a)* consolidate v12→v13 and v13→v14 migrations into single v12→v13
- *(server)* hoist tool_defs conversion before investigation loop
- *(oauth)* address code review findings
- address review findings — remove duplicated llm field, simplify override logic
- address review findings for callback truncation
- address review findings — remove weak test and trim doc comment
- resolve code review findings for task health awareness
- simplify CacheControl and SystemContentBlock types
- *(skills)* address review findings — use named constants, remove redundant check
- [#329] address review findings — derive Clone on LlmError, prune unused APIs
- *(tui)* address code review findings
- [#350] extract timezone helpers and improve type safety

### Documentation

- update documentation for google-workspace skill
- compound solution for CLI blocked-flag equals bypass
- add pre-1.0 breaking changes policy
- add solution doc for removing bundled skills
- sync crate-local docs for v0.1.7 publish
- update test count to ~1290
- add chat_id override fix to multi-agent Telegram solution doc
- add prefix attribution brainstorm and code review findings
- update documentation for recent changes
- enhance solution doc with cross-references and prevention strategies
- update documentation for --format flag on mika ask
- update documentation for team workspace restructure
- compound solution for team workspace security hardening
- update test count to ~1317
- compound solution for trace_id structural linkage
- update CLAUDE.md observability section for request_id linkage and session lifecycle
- compound solution for observability polish (#162)
- add MIKA_LOG_FORMAT to configuration reference
- compound solution for log format selection pattern
- add brainstorm and plan for dashboard tasks/team-runs pages
- update dashboard pages and API endpoints for tasks/team-runs
- compound solution for UTF-8 byte-slicing panic in dashboard DTO
- mark plan as completed
- [#71] update CLAUDE.md to reflect multi-provider LLM architecture
- [#71] update documentation for multi-provider LLM support
- [#71] add solution doc and review findings for multi-provider LLM
- [#178] update CLAUDE.md for markdown rendering and session lifecycle
- [#182] add browser-control to skills reference
- compound knowledge — adding a get_documentation topic
- update stale --session reference in active plan
- document CLI flag ID suffix naming convention
- update documentation for ISO 8601 timestamp migration
- add solution doc for ISO 8601 timestamp migration
- update CLAUDE.md for @senara-solutions/ui package extraction
- add solution doc for @senara-solutions/ui package extraction
- add brainstorm doc from unified LLM API key refactor
- update dashboard dev command to use dev:dashboard script
- add branch linking convention to issue commands
- update CLAUDE.md for previous run context display in team TUI
- compound solution for team TUI previous run context display
- update documentation for failed callback delivery
- compound solution for failed callback task delivery
- update documentation for embedded dashboard feature
- compound solution for embedded dashboard SPA pattern
- update CLAUDE.md for exit code handling changes
- add solution doc for long-running monitor false failure on signal
- update CLAUDE.md for local source skills install and --link mode
- compound solution for local source skills install with --link mode
- update documentation for A2A protocol implementation
- add compound solution doc for A2A protocol implementation
- update multi-provider solution doc after investigation panel fix
- compound solution for investigation panel provider fix (#224)
- update documentation for OAuth PKCE token exchange
- compound OAuth PKCE solution documentation
- brainstorm for claude-pilot dashboard integration (#236)
- *(plan)* claude-pilot dashboard integration (#236)
- update runtime-structure schema version to v14
- update configuration for per-provider LLM config (#239)
- fix remaining stale references in configuration.md (#239)
- add plan doc for per-provider LLM config (#239)
- add brainstorm for per-provider LLM config (#239)
- plan for per-provider skill variants (#241)
- update CLAUDE.md and add compound solution for per-provider skill variants
- add plan for skill variant model granularity (#246)
- update documentation for model-level skill variant directories
- fix stale three-level fallback comments in agent.rs and index.rs
- add plan for reply routing observability fix
- update plan for reply routing fix
- update CLAUDE.md with per-skill LLM override architecture
- compound solution for per-skill LLM override pattern
- add MIKA_AGENTS_NAMESPACE to gateway env var documentation
- update CLAUDE.md with delegate session persistence
- compound solution for delegate_task session persistence pattern
- update documentation for smart work item status transitions
- compound solution for work item status transitions
- add compound doc for builtin handler timeout fix
- document callback result truncation and update test count
- compound solution for callback result truncation (#259)
- fix code example in solution doc to use "delegate" channel
- compound solution for delegate channel_type taxonomy
- update documentation for grounding rule guardrail
- compound solution for downstream state hallucination fix
- update CLAUDE.md for callback processing race fix
- compound solution for callback processing race fix
- update documentation for task health awareness feature
- compound solution doc for task health awareness pattern
- add ce-plan for runtime observability feature
- update CLAUDE.md and .env.example for runtime observability
- compound solution doc for runtime observability pattern
- add pipeline artifacts vs release-plz gotcha to CI/CD solution
- compound solution for trace_id as observability join key
- add plan doc for telemetry trace_id observability
- update architecture.md for work item tool registration change
- compound solution for work item write tools restriction
- update runtime-structure for dev run source filter change
- compound solution for dev runs source filter fix
- mark plan as completed
- add telemetry schema trace_id brainstorm
- update CLAUDE.md and add compound solution doc for context injection
- [#289] compound solution for dedicated GitHub token pattern
- add solution doc for TUI callback framing fix (#269)
- mark plan as completed
- document internal tag stripping in CLAUDE.md
- compound solution for internal tag stripping (#223)
- add git-ops and missing skills to runtime-structure directory tree
- compound solution for adding builtin handler skills
- add GH_TOKEN for gh CLI in agent Claude Code sessions
- update test count to ~1770
- compound solution for run_gh string-to-array coercion
- document Anthropic prompt caching in CLAUDE.md
- compound solution for Anthropic prompt caching (#302)
- [#303] update documentation for schema v17 and work item dedup
- [#303] compound solution for work item dedup during retries
- add ADR-007 compaction strategy and memory classification
- update CLAUDE.md and architecture for search_tool_history
- compound solution for memory-aware introspection tool pattern
- add compound solution for github_get extraction
- update task-health solution doc for callback injection (#314)
- update CLAUDE.md for callback task health injection (#314)
- compound solution for callback work item context injection
- update architecture docs for generic callback framing (#313)
- compound solution for generic callback framing (#313)
- [#321] add plan frontmatter and compound solution doc
- add agent offline troubleshooting entry
- compound solution for gateway offline agent error message
- mark plan as completed
- update skill prompt size limit documentation for #331
- compound solution doc for #331 always_on prompt enforcement
- [#329] update CLAUDE.md for eval harness — test count, testing conventions, MockLlmProvider
- [#329] compound solution doc for agent eval testing harness
- add agent eval harness brainstorm document
- update CLAUDE.md for docs-sync CI job
- compound solution for CI docs sync drift gate
- update CLAUDE.md for slash command changes
- compound solution for TUI slash command reliability fixes
- compound solution for run_gh GH_TOKEN injection (#346)
- [#350] update architecture docs for timezone-aware reminders
- [#350] compound solution for timezone reminder bug
- [#350] sync crate-local docs after architecture update

### Fixed

- pass explicit chat_id to delegate agent sender for correct Telegram prefixing
- add diagnostic tracing to delegate send_message flow and improve tool description
- pass agent_name in CLI sender and auto-relay delegate text responses
- suppress stderr logs for mika ask command
- plumb --run-id through chat --team path
- address code review findings for team workspace restructure
- add build.rs to mika-gateway to prevent stale SQLx migration embeds
- wire trace_id through callbacks and team resume, add team_workspace to unified_timeline
- inject referenced run context into orchestrator prompt via --run-id
- observability polish — request_id linkage, session cleanup (#162)
- address review findings — UTF-8 safe truncation, DRY filter builders
- [#71] address review findings for multi-provider LLM
- [#71] address review findings for multi-provider LLM
- [#178] add remark-gfm for table rendering in MarkdownContent
- [#178] address review findings — session lifecycle and iteration numbering
- rewrite relative .md links to Starlight-compatible paths in sync script
- [#182] address review findings — SSRF guidance and credential clarity
- [#182] address pattern review findings
- make TimelineRow.agent_id optional to handle team_workspace NULL values
- strengthen confirmation-before-action guardrail for status questions
- navigate to Timeline on "View Full Logs" click in audit events card
- link callback tasks to work items and document claude-pilot security fixes
- address code review findings from PR #193
- *(ci)* install from workspace root so @senara-solutions/ui resolves
- clippy cmp_owned warning in cron test
- [#196] address review findings for previous run context display
- [#203] deliver failed callback tasks to agent
- embedded dashboard white page and add root landing route
- runtime dashboard toggle API and TUI footer buttons
- /rewind N not updating TUI display
- skip stale failed callbacks to prevent conversation flooding
- guard update_task_failed against terminal states and add signal distinction
- resolve clippy collapsible_if warning in --link validation
- scan parent directory for sibling skills during dependency resolution
- *(a2a)* add missing workspace metadata to fix cargo-deny license check
- *(a2a)* resolve code review findings from A2A protocol PR
- *(a2a)* remove dual-write message persistence and update solution doc
- filter non-LLM spans from Langfuse and add gen_ai semantic conventions
- update aws-lc-sys and rustls-webpki to resolve cargo audit vulnerabilities
- *(llm)* fix MiniMax base URL and strip <think> tags from responses
- update MiniMax base URL assertion in test
- *(tools)* improve update_core_memory schema for non-Anthropic models
- *(server)* use configured LLM provider in investigation panel (#224)
- *(oauth)* send token exchange as JSON and split code#state
- work item session cap only counts active items (#233)
- use title prop instead of label on CopyButton
- *(ci)* update smoke test env var for per-provider config
- review fixes for per-provider skill variants (#241)
- prevent dashboard asset regression on deploy
- remove provider-level prompt layer from skill variant resolution
- *(gateway)* add observability to reply routing pipeline
- *(gateway)* parse [agent_name] from reply text for reply routing
- strengthen worktree detection in /mika to prevent nested worktree loops
- collapse nested if let to satisfy clippy collapsible_if
- [#251] gateway uses FQDN with MIKA_AGENTS_NAMESPACE for cross-namespace routing
- persist delegate_task messages in delegate's session for observability
- use skill timeout for builtin handlers instead of hardcoded 30s
- truncate callback results to 10KB before prompt injection (#259)
- *(agent)* add grounding guardrail to prevent downstream state hallucination
- *(agent)* prevent callback processing race and add workflow-aware triggers
- *(agent)* enforce tool execution before accepting assistant responses (#270)
- UTF-8 safe truncation and warn on observability DB write failures
- resolve code review findings (P1 + P2)
- clippy collapsible-if and eslint unused imports
- clippy unnecessary_map_or with telemetry feature
- *(ci)* skip pipeline artifacts check for release-plz PRs
- remove work item write tools from default_tools() — restrict to orchestrators only
- *(dashboard)* include github_issue-sourced dev runs in dashboard
- conditional required_tools enforcement and mika ask callback awareness (#265)
- [#284] use builtin file tools instead of run_shell for config changes
- use workflow-aware callback framing in TUI chat path (#269)
- [#285] use per-agent LLM config in team engine
- strip internal context/metadata tags from LLM response display
- address review findings for tag stripping
- *(skills)* address review findings for git-ops handler
- *(github)* coerce string command params to arrays and document gh pr diff limits
- *(llm)* enable Anthropic prompt caching for agent LLM calls
- [#303] make create_work_item idempotent with reference_url dedup
- [#303] address review findings in migration and dedup response
- pin CI Rust toolchain to rust-toolchain.toml
- *(db)* escape LIKE metacharacters in keyword search
- add frontmatter to memory-classification doc for Starlight schema
- add Claude Code identity headers for OAuth subscription tokens
- deprecate MIKA_LLM_API_KEY and fix OAuth setup env var mismatch
- regenerate OpenAPI spec to include dashboard endpoints
- [#321] restore dashboard endpoints in OpenAPI spec via utoipa annotations
- sync local main before worktree creation in /mika pipeline
- *(skills)* fail loudly when always_on skill prompt exceeds size limit
- add CI gate for crate-local docs sync drift
- *(tui)* improve /clear, /provider, /model slash command reliability
- *(ci)* sync crate-local slash-commands.md with docs/
- [#346] inject MIKA_GITHUB_TOKEN as GH_TOKEN in run_gh for platform identity separation
- add npm ci and packages/ui build to build-dashboard target
- re-sign binaries after copy on macOS in make install
- use per-agent LLM provider in server mode (#323)
- [#350] add timezone support to reminders to prevent off-by-one day errors
- [#350] address QA HOLD-2 and HOLD-3 on timezone reminder PR
## [v0.1.6](https://github.com/senara-solutions/mika/releases/tag/v0.1.6) — 2026-03-13

### Added

- *(reminders)* add periodic reminder support via cron_expr
- *(db)* schema v5 — rename memory_events to audit_events, add trace_id columns
- *(observability)* thread trace_id through all write paths
- *(server)* add dashboard REST API endpoints
- *(dashboard)* add React observability dashboard
- *(dashboard)* redesign UI to match Stitch observability designs
- *(agent)* add create_agent tool for runtime agent creation
- *(db)* add mention_count to people table
- *(agent)* add create_team guidance to system prompt
- *(agent)* enrich list_teams output with full team configuration
- *(common)* add serde defaults to TeamDefinition for flexible parsing
- *(agent)* add create_team, delete_team, update_team tools
- *(dashboard)* show tool call summaries on assistant messages in session detail
- *(dashboard)* tabular tool calls with click-to-copy and timeline agent filter
- *(server)* add investigation SSE endpoint with read-only agent loop
- *(dashboard)* add investigation side panel with SSE streaming
- *(skills)* respect user overrides for builtin skill always_on flag (#73)
- *(cli)* improve /skills display with grouped columnar layout
- *(tui)* add mouse-based text selection and clipboard copy
- *(tools)* add read-only introspection tools for agent-native parity
- *(tui)* add textarea selection rendering and mouse support
- *(dashboard)* convert team session view to conversational timeline
- *(prompt)* instruct agent to check own files before answering self-knowledge questions
- enhance mika setup with multi-secret wizard, proper TOML parsing, and TTY guard
- add docker-compose and refactor Dockerfiles with BuildKit cache mounts
- *(teams)* add conversation continuity across team runs
- *(cli)* add --session flag to mika ask for session reuse
- *(agent)* add generic work item tracking with create_work_item, update_task_status, list_work_items tools
- *(agent)* schema v9 and audit trail completeness for conversation rewind
- *(agent)* add conversation rewind engine with preview and execute
- *(cli)* add /undo and /rewind TUI slash commands
- *(server)* add rewind preview and execute API endpoints
- *(cli)* add mika doctor and config set/get/list commands
- *(agent)* enforce work item tracking before delegation
- out-of-sandbox file writes and task completeness prompts
- fill dev workflow gaps — dashboard CI, supply chain security, issue linking, smoke tests
- *(dashboard)* add session ID prefix search to sessions page
- *(dashboard)* add copy-to-clipboard and investigation GitHub issue creation
- *(skills)* add label operation docs and keywords to github skill
- *(dashboard)* scope investigation panel and persist history
- *(tui)* add Alt+Enter as fallback for multi-line input
- unified GitHub label taxonomy with auto-sync
- *(tui)* persist /model slash command to config.toml
- add Acceptance Criteria section to all issue creation channels

### Changed

- *(reminders)* unify NewTask construction and add min interval guard
- *(prompt)* tighten multi-action and continuity instructions for token efficiency
- *(tui)* derive Ord on TextPosition, clear selection on keypress, add u16 guards
- *(dashboard)* deduplicate frontend utilities
- *(prompt)* trim filler paragraph from self-knowledge skill prompt
- *(tui)* extract shared unicode-width wrapping iterator
- *(tui)* unify textarea selection highlighting via post-processing
- simplify config cascade from 6 layers to 4 sources with dotenvy .env support
- *(teams)* simplify based on code review findings
- address code review findings for doctor and config commands
- *(agent)* address code review findings for delegation guard
- *(skills)* add non-zero exit signal to tool history and prompt guidance
- tighten transparency rule wording per code review
- *(cli)* use exhaustive match arms for compile-time safety
- *(agent)* resolve code review findings for skill dependencies
- *(tui)* extract placeholder constants, fix Esc handler and ui.rs renderer
- [#115] simplify progressive truncation to single pass

### Documentation

- update documentation for periodic reminders
- document periodic reminder solution in docs/solutions/
- document multi-action and conversation continuity solution
- document proactive state checking solution
- update documentation for orthogonal observability changes
- document orthogonal observability solution
- update plan checklist with completed items
- update documentation for observability dashboard
- update documentation for recent changes
- add brainstorm, plan, and lockfile for investigation panel
- update documentation for team CRUD tools and investigation endpoint
- compound solutions for investigation panel, tool call UX, and team CRUD
- update documentation for skill overrides feature (schema v7)
- add callback result display plan from PR #92
- update documentation for recent changes
- add solution doc for TUI textarea selection rendering
- update documentation for recent changes
- add solution doc for self-knowledge home directory file gap
- update documentation for recent changes
- add solution doc for setup wizard secret handling
- update documentation for recent changes
- add solution doc for Docker BuildKit cache mounts and compose
- update documentation for team conversation continuity feature
- add solution doc for team conversation continuity
- add runtime structure reference for ~/.mika layout, DB schema, and logs
- add brainstorm and plan for claude-asked relay improvements
- update documentation for work item tracking feature
- add solution doc for work item tracking architecture
- update plan status to completed
- add brainstorm document for conversation rewind feature
- add solution document for rewind context marker pattern
- update documentation for doctor and config commands
- add solution doc for ConfigKeyRegistry CLI management pattern
- add plan and solution for build-mika skill format fix
- update documentation for recent changes
- add solution doc for skills doc-code drift and validation infrastructure
- update architecture.md with workflows block and delegation guard
- add solution doc for delegation work item guard pattern
- update documentation for exec handler exit code changes
- document exec handler stdout-on-nonzero-exit fix
- update configuration docs for GitHub issue creation settings
- document conditional investigation tool registration pattern
- mark dashboard copy/github-issue plan as completed
- update documentation for scoped CLI flags
- add solution doc for CLI flag subcommand scoping pattern
- update documentation for skill dependency resolution
- add solution doc for skill dependency resolution and action guard
- add solution doc for investigation panel Shift+Enter fix
- update skills.md with new github skill label keywords
- add solution doc for github skill label documentation gap
- add solution doc for investigation panel scoping and persistence
- document Alt+Enter multi-line input keybinding
- add solution doc for TUI multi-line input Alt+Enter fallback
- advertise Alt+Enter as primary multi-line input method
- brainstorm file tool consolidation (#127)
- trim CLAUDE.md from 40K to 26K chars
- [#115] document tool_calls metadata truncation solution

### Fixed

- exclude team messages from TUI chat history
- *(prompt)* add multi-action batching and conversation continuity guidance
- *(prompt)* add proactive state checking before write operations
- *(clippy)* collapse nested if-let in create_reminder
- *(db)* add DB-constraint duplicate detection for reminders and events
- *(db)* migrate commitments to partial unique index for status-aware dedup
- address code review findings from PR #88
- move trace.rs to mika-common to fix telemetry feature build
- suppress too_many_arguments clippy warnings for trace_id params
- *(dashboard)* resolve code review findings from observability dashboard
- *(dashboard)* remove artificial ID prefixes and truncation
- *(cli)* show error details when team loading fails
- *(dashboard)* change dev proxy target to port 8081
- *(dashboard)* store full tool call data and add quick-copy pills
- *(server)* use char-based truncation in investigation tools to avoid UTF-8 panics
- *(dashboard)* fix timeline agent filter and subsystem dot colors
- make is_bundled_skill case-insensitive and deduplicate override logic
- *(prompt)* guide agent to use update_skill for always_on changes
- *(tui)* display callback task results as system messages instead of 'You:'
- *(tui)* correct TextPosition comparison after Ord derive
- *(teams)* store correct agent_id in team session messages
- harden dotenv and setup — newline injection, atomic writes, non-interactive mode
- *(teams)* address code review findings
- *(dashboard)* consolidate token env var to VITE_MIKA_DASHBOARD_TOKEN
- *(ci)* regenerate OpenAPI spec to match utoipa annotations
- *(agent)* truncate tool call metadata to prevent silent entry drops
- *(cli)* validate session ownership for --session flag
- *(agent)* address code review findings for work item tracking
- *(agent)* drop unified_timeline view before tasks table rebuild in v7→v8 migration
- *(rewind)* cross-session support and code review improvements
- *(rewind)* inject context marker after rewind to prevent agent confabulation
- *(rewind)* code review fixes and documentation updates
- *(dashboard)* render trace detail messages with full styling and tool calls
- *(skills)* align docs, add validate command, fix review findings
- disable per-package git tags for secondary crates to prevent release-plz tag collision
- *(ci)* resolve cargo-audit/cargo-deny failures
- *(ci)* resolve rustsec/audit-check failures
- *(ci)* replace rustsec/audit-check action with direct cargo-audit
- *(ci)* replace cargo-deny-action with direct install
- *(ci)* upgrade actions/checkout v4→v6 and actions/setup-node v4→v6 to resolve Node.js 20 deprecation warnings
- *(skills)* return output on non-zero exit instead of discarding it
- *(investigate)* address review findings — input limits, repo validation, UX consistency
- *(trace)* fix broken trace messages endpoint and missing trace_id propagation
- *(config)* rename github_token to investigate_github_token and improve setup
- *(agent)* add transparency rule for non-zero exit codes in responses
- *(cli)* scope --agent and --team flags to relevant subcommands only (#102)
- *(agent)* migrate run_gh to Rust builtin handler with JSON array protocol (#119)
- *(agent)* add skill dependency resolution and unsolicited action guard (#134)
- *(agent)* add tool_history guardrail to prevent skipping actions (#135)
- *(dashboard)* support Shift+Enter newline in investigation panel input
- *(skills)* address review findings for label docs
- *(dashboard)* address review findings for investigation panel
- *(dashboard)* revert findLast to reverse().find() for ES2022 compat
- *(tui)* make history navigation team-mode-aware for placeholder text
- [#115] truncate tool_calls metadata per-field instead of dropping tail entries
- *(server)* add Rust context to investigation agent system prompt
- add tilde (~) home directory expansion to file tools (#145)
- [#144] set success=false when non_zero_exit is true in tool call metadata

### Performance

- *(tui)* optimize text selection rendering and layout caching
- *(server)* combine data+count queries and remove response type ceremony

### Security

- *(server)* separate dashboard token from mutation endpoints
## [v0.1.4](https://github.com/senara-solutions/mika/releases/tag/v0.1.4) — 2026-03-07

### Added

- add agent and team management tools
- add config editing and shellcheck guidance to shell-exec skill
- *(tui)* shell-like slash command autocompletion with argument completion
- *(skills)* add marketplace lock file model and repo scanner
- *(skills)* add git operations module for marketplace
- *(skills)* implement marketplace install/uninstall/update CLI commands
- *(skills)* add marketplace origin detection and lock cleanup
- add agents-teams built-in skill
- add PR creation step to /mika workflow
- add periodic memory reflection system
- *(tools)* add write_file tool with overwrite confirmation flow
- *(cli)* add --team option for TUI team mode
- *(teams)* add migration v11 with team_runs/team_messages tables and TeamEvent enum
- *(teams)* engine persists to DB, emits typed TeamEvents, adds verbose mode
- *(teams)* replace TOML history with SQLite DB queries
- *(teams)* inject conversation history into orchestrator context
- *(observability)* add tracing spans to agent loop, Claude API, team engine, and server
- *(observability)* add TeamPhase enum, new TeamEvent variants, and TUI split-pane dashboard
- *(observability)* add feature-flagged OpenTelemetry/OTLP export with Langfuse support
- wire OTel export into CLI, fix OTLP endpoint docs to require /v1/traces
- unified task engine (schema v1 baseline)
- resolve todos 425, 455, 476 — builtins, team progress, and type constants
- implement task engine gap analysis — create_task, cancel_task, resume_agent, callback endpoint
- team-aware async callbacks with long_running skill flag
- add /mika-issue and /mika-issues Claude Code commands
- *(db)* add delivered status and tool_result role (schema v2)
- *(cli)* simplify mika ask --task-id to mark-and-exit
- *(tui)* callback delivery polling and loop prevention
- *(task-engine)* mark callback tasks delivered after dispatch
- *(site)* update features section with current capabilities
- *(site)* add "Backed by" cross-promotion to footer

### Changed

- add code review findings for agent management tools
- add code review findings for shell-exec handler fix
- standardize jq JSON parsing and add env scrubbing to handlers
- *(tui)* resolve code review findings for autocompletion
- *(tools)* extract shared validate_and_resolve_path helper
- *(teams)* address code review findings
- *(teams)* resolve code review findings from team TUI mode
- *(teams)* deduplicate team DB opening and test helpers
- resolve code review findings from observability PR
- add code review findings for feat/unified-task-engine
- add code review findings for feat/unified-task-engine (478-497)
- add code review findings for feat/unified-task-engine (498-519)
- mark todos 498-519 as complete (except 511, 517)
- consolidate to single database per container
- add code review findings for DB consolidation (520-521)
- resolve code review findings 511, 517, 520, 521
- add code review findings for async callbacks (522-542)
- rename persona → self_model + agent-aware core memory defaults
- rename default agent from "main" to "mika"
- unify self-knowledge tools into single get_documentation handler
- resolve code review findings from callback TUI delivery
- replace silent let _ = with warn logging on task DB ops
- redesign schema with sessions + messages tables (v3)

### Documentation

- update documentation for agent management tools
- add solution doc for agent team management tools integration
- update documentation for jq requirement and env scrubbing
- add solution doc for shell-exec jq JSON parsing fix
- add solution doc for shell-exec config editing quality
- update documentation for shell-like autocompletion
- add solution doc for shell-like autocompletion
- add cursor positioning gotcha to solution doc
- add plan for escape chars fix (completed)
- update test count in CLAUDE.md (~490 tests)
- update test count in CLAUDE.md (~744 tests)
- add contributing guide with Claude Code workflow
- simplify CONTRIBUTING.md based on review feedback
- add marketplace ADR, update skills docs and CLAUDE.md
- mark marketplace plan as completed
- update documentation for skills marketplace changes
- compound solution for MIKA_* env var leakage through exec handlers
- add agents-teams to prompt-only skills table in docs/skills.md
- compound solution for adding prompt-only bundled skills
- compound solution for background agent mode design checklist
- update CLAUDE.md and architecture.md for write_file tool
- add compound solution for write_file overwrite confirmation flow
- update CLAUDE.md for absolute-path reporting and test count
- add compound solution for write_file path reporting misbehavior
- mark write-file-silent-miswrite plan as completed
- update documentation for --team TUI mode
- add compound solution for team TUI mode integration
- update documentation for graph-structured team persistence
- add compound solution for team graph persistence
- update documentation for team TUI mode changes
- update test count to ~837 after code review fixes
- add solution doc for team engine code review findings batch
- add consolidated observability plan for OTel + TUI dashboard
- add solution doc for observability OTel + TUI dashboard implementation
- update CLAUDE.md, configuration.md, and add OTLP endpoint solution doc
- update architecture docs and add solution for tool name shadowing
- update documentation for unified task engine completion
- compound solution — callback/resume agent lifecycle
- update CLAUDE.md and architecture.md for DB consolidation
- compound solution — DB consolidation to single container database
- improve mika-doc-audit command with scope argument and process steps
- update documentation for async callbacks code review fixes
- compound solution for async callbacks code review (522-542)
- update CLAUDE.md for recent changes
- add solution doc for callback task loop prevention
- update CLAUDE.md for callback TUI delivery
- update documentation for callback TUI delivery changes
- add solution doc for callback TUI delivery polling
- add plan for team task agent_id mismatch fix
- update CLAUDE.md for task tree query changes
- add solution doc for team task agent_id mismatch
- mark team task agent_id mismatch plan as completed

### Fixed

- use jq for JSON parsing in shell-exec handler
- *(tui)* config value completion and tilde expansion security
- *(tui)* place cursor at end after tab completion
- *(agent)* retry on ETXTBSY in exec handler to fix flaky tests
- remove stale root templates/ directory
- add jq availability guard to file-reader handler
- improve bundled skill seeding disabled warning message
- add background reminder poller so reminders fire at scheduled time
- convert reminders fire_at from TEXT to INTEGER to fix detection
- resolve 15 code review findings for skills marketplace
- remove duplicate timeouts section from agents-teams skill
- address all 13 code review findings for periodic memory reflection
- *(tools)* add target-file symlink check and prompt docs for write_file
- *(tools)* report resolved absolute path in write_file and write_workspace
- *(tools)* apply absolute-path reporting to read_workspace for consistency
- *(cli)* address review findings for team TUI mode
- *(cli)* move init_sqlite_vec into DB constructors and persist team conversations
- *(teams)* prevent wasted tool steps and ensure workspace writes
- *(teams)* add global timeout and progress heartbeats to team runs
- *(teams)* emit per-agent completion and "all done" progress events
- *(teams)* resolve code review findings 441-449
- *(observability)* wire OTel layer into subscriber and address review findings
- add type annotation to generic build_otel_layer call in test
- feature-gate turbofish on build_otel_layer test
- resolve 14 code review findings from unified task engine review
- resolve 20 code review findings (478-497)
- resolve cargo fmt and clippy warnings
- resolve 20 code review findings (498-519) — task engine hardening
- cross-channel poll parameter mismatch in load_messages_after
- auto-seed user in people table after onboarding + strengthen store_fact prompts
- register_agent upsert + CLI memory reset agent-aware default
- sync agent display name from identity.toml on startup
- enforce plain first-name convention for people table
- read reflection timezone from identity.toml
- strengthen self-knowledge prompt to consult docs before answering
- correct docs to say exec handlers receive input via stdin, not env var
- use silent agent for CLI callback tasks and add orchestrator guards
- resolve 8 code review findings (543-552)
- resolve team task agent_id mismatch causing orphaned pending tasks
- remove agent_id filtering from task tree traversal queries
- don't load agent chat history in team mode TUI
- use orchestrator agent_id for invoke_orchestrator parent task
## [v0.1.1](https://github.com/senara-solutions/mika/releases/tag/v0.1.1) — 2026-03-01

### Added

- add automated release system with GitHub binary downloads

### Changed

- address code review findings from PR #41

### Documentation

- update documentation for release system and rustls migration
- add solution doc for automated release system setup
- update CLAUDE.md for persistent input history and paste fix
- add solution doc for TUI history persistence and paste cursor fix

### Fixed

- *(ci)* add clippy and rustfmt components to rust-toolchain.toml
- *(ci)* add publish = false for mika-gateway in release-plz.toml
- *(cli)* persist input history across sessions and fix paste cursor positioning
- *(cli)* eliminate temp file permission race in history write

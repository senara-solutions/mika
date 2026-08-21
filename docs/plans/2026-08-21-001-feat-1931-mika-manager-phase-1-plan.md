---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ratified-brief
origin: github-issue senara-solutions/mika#1931
brief: mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md
created: 2026-08-21
---

# feat(agent): mika-manager Phase 1 — milestone-scope operational coordinator (LECTURE seule, wrapper-only) (#1931)

## Summary

Ship Phase 1 of the mika-manager entity — a milestone-scope operational coordinator that **reads** milestone state (via `gh` CLI), **assesses** it (rules-driven recommendations), and **reports** structured Markdown to Prime→sami→Vincent. Zero write authority anywhere in the diff. Wrapper-only over existing `gh` CLI and cm HTTP client (INV-2). Cadence hybrid: event-driven detection + 6h plancher heartbeat. Silence threshold in JOURS (calibrated per milestone).

Delivers the three components (Reader / Assessor / Reporter) ratified by Prime + Vincent 2026-08-21 (Vincent verbatim: « Oui. Ça semble vraiment bien 🤩 good job 👏 »). The 5 open questions in the brief § 7 are tranched (§ Ratification). Phase 2 dispatch authority remains gated behind 3 portes (forge-gate loop-résistance + contention exec + INTERNAL_TOKEN alignment) — not wired in this PR.

---

## Problem Frame

**What is missing today:** No Mika entity carries the "manager of a milestone" role. Milestone state is dispersed across `gh issue view <milestone>`, sub-issue statuses, PR merges, CI, ticket body callouts. A human manager (MPC or Vincent) mentally composes these; a manager-Mika needs a composer.

**Failure modes today:**
- Silent-progress divergence: PR merged but sub-issue open (or inverse) — no reconciliation surface.
- Blocker stagnation: `Blocked by #N` never re-checked; blockers linger unreported.
- Priority drift: unclear which sub-issue is NEXT once a blocker clears.
- Stagnation invisibility: a milestone can sit with zero movement for days and no one notices — « l'absence d'event EST l'event ».

**What Phase 1 solves:** Give the operator (via CLI) + Prime (via cadence delivery) a **composed, structured view** of a milestone's state + a **recommendation** for the NEXT action. Zero write authority — manager REPORTS, Vincent/MPC DECIDES.

---

## Requirements

- **R1.** `mika milestone read <repo>#<num>` emits a canonical JSON document with milestone metadata + enumerated sub-issues (title, state, priority_rank, plan_present, branch_present, pr_number, pr_state, ci_state, blockers[]) + progress counts + recent_activity[].
- **R2.** `mika milestone assess <repo>#<num>` reads Reader output and applies four rules (blockers surface, drift detection, silent-progress reconciliation, priority-ranking recommendation), emitting a JSON assessment. Output = **recommendation, not decision**.
- **R3.** `mika milestone report <repo>#<num>` emits Markdown following the brief § 2d template.
- **R4.** A cadence hybrid: event-driven trigger fires on detected milestone state change; 6h plancher heartbeat fires even with zero events. Both configurable.
- **R5.** Silence threshold in JOURS scale, per-milestone calibrated (default: 3 days), configurable via env.
- **R6.** Escalade seuil dur: when a milestone is **bloqué-dur** (all in-flight sub-issues have unresolved blockers or `ci_state = failed`), the report is delivered via a distinct escalation path (env-configurable target) intended for Vincent-direct.
- **R7.** **LECTURE SEULE non-négociable.** No dispatch (no `run_claude_pilot`), no ticket label mutation, no PR merge, no scope approval anywhere in the diff. Only external HTTP call is the report delivery POST (idempotent, well-known endpoint).
- **R8.** **Wrapper-only INV-2.** Compose existing `gh` CLI subprocess pattern (matching `auto_pull.rs::gh_list_open_issues` shape). No new API surface. No re-invented GitHub client.
- **R9.** **Phase 2 gates NOT wired** — the code contains no scaffolding for dispatch authority (no dispatch class, no `run_claude_pilot` invocation site, no manager→executor bridge).
- **R10.** Chain of authority verbatim: Prime → Manager → Executors. mika-manager entity is **distinct from mika-prime**. The report delivery pattern maps to D8 subsystem-2 (cm bus).

---

## Product Contract preservation

Product contract source = ratified brief (`mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`), Vincent-ratified 2026-08-21 with 5 Prime-tranched verdicts. This plan transcribes the 5 verdicts into requirements verbatim (R1–R10 cover the deliverable; R6 covers the escalation seuil dur; R10 encodes the chain-of-authority).

---

## Key Technical Decisions

### KTD1. Home the logic in `crates/mika-agent/src/milestone_manager/`, expose via `mika-cli`

**Choice:** New module in `mika-agent` (Reader, Assessor, Reporter, Cadence). CLI subcommand `mika milestone {read,assess,report}` in `mika-cli/src/commands/milestone.rs` is a thin adapter that calls the agent-crate module.

**Rationale:** The Reader/Assessor/Reporter logic must also be usable from a future server-side cadence loop (spawnable at mika-spirit startup) — putting it in `mika-agent` keeps that path open without cross-crate re-plumbing. The CLI shape mirrors the existing `mika kg` / `mika tasks` patterns (thin CLI, thick agent-crate module).

**Alternatives rejected:**
- Put logic in `mika-cli` only — blocks reuse by the cadence loop and violates DRY when Phase 1.5 adds server-side scheduling.
- Put logic in a new crate — over-engineering for ~800 LOC; ties in to the existing 5-crate workspace convention.

### KTD2. Wrapper-only over `gh` CLI subprocess (INV-2, mirror `auto_pull.rs`)

**Choice:** Call `gh` via `tokio::process::Command`, JSON output, deserialize with `serde_json::Value`. Mirror the exact shape of `crates/mika-agent/src/auto_pull.rs::gh_list_open_issues` (env `GH_TOKEN`, stdin::null, kill_on_drop). No `reqwest` GitHub client. No `octocrab` dep.

**Rationale:** INV-2 verbatim from brief: "compose existing `gh` CLI + cm HTTP client. Zero new API surface. Zero new infrastructure." The existing `gh` wrapper is battle-tested (12 callsites across `auto_pull.rs`, `wip_rescue.rs`, `perimeter/`, handlers) and re-using its shape is the review-safe path.

**Alternatives rejected:**
- `reqwest` direct calls to GitHub REST/GraphQL — duplicates the auth path (`gh` handles GH_TOKEN + GitHub App auth transparently) and is a new API surface (INV-2 violation).
- `octocrab` typed client — new dep, new surface, INV-2 violation.

### KTD3. Reader output = a stable Rust struct `MilestoneState` (serializable to JSON), Assessor output = `Assessment`, Reporter output = `String` (Markdown)

**Choice:** Three module-owned types with `#[derive(Serialize, Deserialize)]`. Reader reads → returns `MilestoneState`. Assessor consumes `&MilestoneState` → returns `Assessment`. Reporter consumes both → returns `String`.

**Rationale:** Two-boundary composition: (a) subprocess boundary at Reader gh calls, (b) type boundary between Reader → Assessor → Reporter. Testing Assessor + Reporter uses in-memory fixtures — no gh subprocess in fast tests. This mirrors the KG resolver's pattern (`SubjectExtractor` → `SubjectEntityResolver` → `Query`).

### KTD4. Cadence via reusable `run_manager_cycle` async function; loop wiring deferred to a `MIKA_MANAGER_TARGET_MILESTONE`-gated spawn in server startup

**Choice:** Public `async fn run_manager_cycle(target: &MilestoneRef, cfg: &ManagerConfig) -> Result<CycleOutcome>` in `cadence.rs`. Cycle: read → assess → decide-deliver (silent unless event or heartbeat window elapsed) → optional POST. The **loop** (interval scheduling) is a thin spawn wired in `mika-agent/src/bin/mika-spirit.rs` startup, gated on `MIKA_MANAGER_TARGET_MILESTONE` being set (Phase 1 = single-milestone).

**Rationale:** Cadence-as-function is unit-testable end-to-end with a fake clock (verify heartbeat fires at 6h, escalade fires on bloqué-dur, silent otherwise). The loop wiring is trivial and mirrors `spawn_resolver_tick_task` / `spawn_dashboard_checkpoint_task`. Env-gated Phase 1 opt-in avoids startup regressions.

### KTD5. Report delivery = POST via `reqwest` to `MIKA_MANAGER_DELIVERY_URL` with `Authorization: Bearer $MIKA_MANAGER_DELIVERY_TOKEN`; escalation to `MIKA_MANAGER_ESCALATION_URL` on bloqué-dur

**Choice:** Two env-configurable delivery targets:
- `MIKA_MANAGER_DELIVERY_URL` — normal Prime→sami→Vincent route (cm bus per D8 subsystem-2 pattern).
- `MIKA_MANAGER_ESCALATION_URL` — Vincent-direct route, only used when Assessor returns `Severity::Blocked`.

Body = JSON `{ report_markdown, milestone_ref, assessment, generated_at, severity }`. Delivery is fire-and-forget with warn-on-failure (mirrors gateway retry policy). Both URLs may be unset — in that case, the report is written to `~/.mika/logs/manager/<milestone-slug>-<ts>.md` (offline sink) so nothing is lost.

**Rationale:** Wrapper-only INV-2 compliance: `reqwest` is already in `mika-common` — one dep, no new surface. Bearer token pattern matches `MIKA_INTERNAL_TOKEN` conventions. The offline sink means the code is testable + deployable **before** the cm bus route is wired up on the receiver side (Prime's inbox route).

**Alternatives rejected:**
- Direct call into cm HTTP client crate — cm client is not a mika dep and adding it here is INV-2 violation (new inter-repo binding).
- Only stdout — no Prime notification path; useless for cadence.

### KTD6. Escalade seuil dur = deterministic function of Assessment severity

**Choice:** `Assessment::severity` is a three-variant enum: `Healthy` | `Attention` | `Blocked`. Rule:
- `Blocked` iff **every** in-flight sub-issue has either `blockers.len() > 0` with all-open blockers OR `ci_state == Failed` — i.e., no unblocked in-flight work exists.
- `Attention` iff any drift or silent-progress alert fires.
- `Healthy` otherwise.

`Blocked` triggers escalation URL; `Healthy | Attention` uses normal URL.

**Rationale:** Deterministic, testable, single-source-of-truth for escalation. The `Blocked` predicate matches the operator's mental model of "the milestone has stopped — no path forward without human input" per the brief's escalation-on-bloqué-dur criterion.

### KTD7. Silence threshold in JOURS, per-milestone calibrated default = 3

**Choice:** Default `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS = 3`. Silence = time since last observed activity (last sub-issue close, last PR merge, last comment on any sub-issue). Report fires on cadence heartbeat always, but the report body carries a `silence_alert: true` flag when threshold exceeded (drives Assessor severity toward `Attention`).

**Rationale:** Brief § Ratification verdict 5: "Seuil silence en JOURS (échelle milestone), calibré sur rythme réel de fermeture". 3 days ≈ 3× median inter-close-interval observed on live milestones (RT-005 ~1 close/day, Luminescent Core ~0.7 close/day, Epic #1806 ~1.5 close/day). Env-configurable so calibration can shift per-milestone without a redeploy.

### KTD8. Tests: unit + injection-verified + full pipeline integration

**Choice:**
- **Reader:** Table-driven parser tests (feed synthetic `gh` JSON stdout, verify `MilestoneState` shape). No live `gh` calls — the boundary is `gh` output → deserialize.
- **Assessor:** 8 rule-fired tests (each rule × fired/silent path), plus 3 severity-classification tests (Healthy/Attention/Blocked).
- **Reporter:** Snapshot test on canonical Markdown (contains all required brief § 2d headings).
- **Cadence:** 4 tests using a fake clock (heartbeat fires at 6h, event-driven fires on state change, silent otherwise, bloqué-dur routes to escalation URL).
- **Injection verification:** For each of Reader/Assessor/Reporter, revert one composer emit line → assert its test fails → restore → assert green (documented in `todos/mika-manager-injection-verification.md`, closed at compound step).
- **Integration:** End-to-end test with mock `MilestoneState` → `run_manager_cycle` → assert POST body shape (via `wiremock` or an in-process handler recording the URL + body).

**Rationale:** Injection-verified per operator discipline `feedback_verify_pipeline_passes_without_the_fix` — every composer emit must have a test that catches its absence.

### KTD9. Zero dispatch scaffolding — enforced by explicit test

**Choice:** Add a `test_no_dispatch_scaffolding` test that greps the new module tree for forbidden tokens (`run_claude_pilot`, `dispatch_class`, `pr_merge_with_gate`, `update_task_status` write paths). Fails if any appear.

**Rationale:** LECTURE SEULE is a structural invariant per the brief. A prompt-level rule ("reviewer verifies no write authority") is fragile per `feedback_prompt_enforcement_fragile`; a compile-time-adjacent test is a structural gate.

---

## Milestone-cascade / mika-arch cross-cutting concerns

- **Coupling with dev-groom / dispatch:** Zero. Phase 1 does not touch dispatch machinery. Phase 2 promotion (after 3 portes) will bind manager → dispatch, but that is not in scope.
- **Coupling with mika-arch-groom-milestone:** Manager consumes cross-cutting concerns via a stubbed function `fetch_arch_cross_cutting(milestone_ref) -> Vec<String>` that returns `Vec::new()` in Phase 1 (documented as "hydration deferred to Phase 1.5"). This keeps the Report § "Cross-cutting concerns" section forward-compatible.
- **Coupling with heartbeat/nudge (cm#111/#115):** Phase 1 reads existing `GET /api/v1/agents/<entity>/health` output when `MIKA_MANAGER_HEALTH_URL` is set (via `reqwest`), otherwise falls back to gh-only inputs. Optional dependency — the manager works without it.

---

## Directory Structure & Files

**New files (mika-agent):**
- `crates/mika-agent/src/milestone_manager/mod.rs` — module root + public API (`Reader`, `Assessor`, `Reporter`, `run_manager_cycle`, `ManagerConfig`)
- `crates/mika-agent/src/milestone_manager/types.rs` — `MilestoneRef`, `MilestoneState`, `SubIssue`, `ProgressCounts`, `RecentActivity`, `Assessment`, `Severity`, `Recommendation`, `Alert`, `CycleOutcome`
- `crates/mika-agent/src/milestone_manager/reader.rs` — `Reader` — gh CLI wrapper + parsers
- `crates/mika-agent/src/milestone_manager/assessor.rs` — `Assessor` — rules engine
- `crates/mika-agent/src/milestone_manager/reporter.rs` — `Reporter` — Markdown formatter
- `crates/mika-agent/src/milestone_manager/cadence.rs` — `run_manager_cycle`, `ManagerConfig`, delivery HTTP
- `crates/mika-agent/src/milestone_manager/tests.rs` — unit tests + injection-verification
- `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` — structural test for zero write authority

**New files (mika-cli):**
- `crates/mika-cli/src/commands/milestone.rs` — thin CLI adapter dispatching to `mika_agent::milestone_manager`

**Modified files:**
- `crates/mika-agent/src/lib.rs` — add `pub mod milestone_manager;`
- `crates/mika-cli/src/cli.rs` — add `Milestone(MilestoneArgs)` variant + args struct
- `crates/mika-cli/src/main.rs` — dispatch `Milestone` variant
- `crates/mika-cli/src/commands/mod.rs` — `pub mod milestone;`
- `crates/mika-agent/CLAUDE.md` — new "Milestone Manager (Phase 1)" section
- `CLAUDE.md` (root) — one line in Environment Variables listing the new `MIKA_MANAGER_*` env vars

---

## Env Vars

| Var | Purpose | Default |
|---|---|---|
| `MIKA_MANAGER_TARGET_MILESTONE` | `<owner/repo>#<number>` — Phase 1 single-target scope | unset (loop disabled) |
| `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` | Cadence heartbeat interval | 21600 (6h) |
| `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS` | Silence-alert threshold | 3 |
| `MIKA_MANAGER_DELIVERY_URL` | Normal report delivery endpoint (Prime→sami→Vincent) | unset → offline sink |
| `MIKA_MANAGER_DELIVERY_TOKEN` | Bearer token for delivery | unset |
| `MIKA_MANAGER_ESCALATION_URL` | Vincent-direct escalation endpoint (Blocked severity) | unset → offline sink |
| `MIKA_MANAGER_HEALTH_URL` | Optional cm health endpoint for executor liveness | unset |

Missing/invalid values follow the standard three-tier fallback (absent → default; unparseable → WARN + default; valid → use value).

---

## Verification Contract

- `cargo build -p mika-agent -p mika-cli` — green
- `cargo test -p mika-agent milestone_manager` — all unit + integration tests pass
- `cargo test -p mika-agent no_dispatch` — structural zero-write-authority test passes
- `cargo clippy -p mika-agent -p mika-cli --all-targets` — green
- `cargo fmt --check` — green
- `bash scripts/verify-pipeline.sh` — passes (docs/plans + source changes present)
- Manual smoke: `mika milestone read senara-solutions/mika#1799` → non-empty JSON with `sub_issues.len() == 6` (LC.1..LC.6 for Luminescent Core), `mika milestone assess senara-solutions/mika#1799` → recommendation JSON, `mika milestone report senara-solutions/mika#1799` → Markdown containing all § 2d headings.
- **LECTURE SEULE audit:** `grep -rE 'run_claude_pilot|pr_merge_with_gate|update_task_status.*status|gh api.*PATCH|gh api.*POST|gh api.*DELETE' crates/mika-agent/src/milestone_manager/` → **must be empty**. Encoded in `no_dispatch_test.rs`.

---

## Definition of Done

- All 10 requirements met
- All tests green (unit + injection + integration + no-dispatch structural)
- CLI subcommand callable and produces valid JSON/Markdown for a real milestone
- Cadence spawn wired at mika-spirit startup, env-gated (loop disabled if `MIKA_MANAGER_TARGET_MILESTONE` unset)
- `crates/mika-agent/CLAUDE.md` § "Milestone Manager (Phase 1)" added
- Root `CLAUDE.md` env-vars section carries the new `MIKA_MANAGER_*` block
- PR body: WHY-first (Vincent ratification + 5 verdicts + brief link + LECTURE SEULE assertion), then WHAT (Reader + Assessor + Reporter + Cadence + tests + first-observation target chosen = mika#1799 Luminescent Core)
- Closes #1931

## Acceptance criteria

- [ ] AC1. `mika milestone read <repo>#<num>` emits JSON with `title`, `sub_issues[]` (each with `number`, `state`, `plan_present`, `branch_present`, `pr_number`, `pr_state`, `ci_state`, `blockers[]`), `progress` counts, `recent_activity[]`
- [ ] AC2. `mika milestone assess <repo>#<num>` produces an `Assessment` JSON with `severity` (Healthy | Attention | Blocked), `recommendation`, `alerts[]`
- [ ] AC3. `mika milestone report <repo>#<num>` emits Markdown containing headings `## Milestone`, `### État`, `### Next action (recommendation)`, `### Silent-progress / drift alerts`, `### Cross-cutting concerns` per brief § 2d
- [ ] AC4. Cadence event-driven trigger fires when observed state differs from last snapshot (persisted per-milestone in a checkpoint file)
- [ ] AC5. Cadence 6h plancher heartbeat fires even when zero events (validated by fake-clock test)
- [ ] AC6. Silence threshold in JOURS scale, default 3 days, configurable via `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS` (three-tier fallback)
- [ ] AC7. Escalade seuil dur: when `Assessment::severity == Blocked`, POST target is `MIKA_MANAGER_ESCALATION_URL` (falls back to offline sink); otherwise `MIKA_MANAGER_DELIVERY_URL`
- [ ] AC8. LECTURE SEULE verified: `no_dispatch_test.rs` structurally rejects `run_claude_pilot | pr_merge_with_gate | update_task_status write | gh api PATCH/POST/DELETE` anywhere in `milestone_manager/`
- [ ] AC9. Wrapper-only INV-2: no new GitHub API client dep (no octocrab, no reqwest calls to `api.github.com`); Reader uses `tokio::process::Command::new("gh")` mirroring `auto_pull.rs::gh_list_open_issues`
- [ ] AC10. Test coverage: Reader parsers (4 shapes), Assessor rules (4 rules × 2 paths + 3 severity classes = 11 tests), Reporter Markdown snapshot (1), Cadence fake-clock (4), integration end-to-end (1), no-dispatch structural (1) — ≥21 tests
- [ ] AC11. Injection-verified: `todos/mika-manager-injection-verification.md` documents each composer emit + its guarding test; closed at compound step
- [ ] AC12. `Closes #1931` in PR body; PR title `feat(agent): mika-manager Phase 1 — milestone Reader+Assessor+Reporter (LECTURE seule, wrapper-only)`
- [ ] AC13. mika-manager entity distinct from mika-prime — no shared identity, no dual-role — verified by absence of any `mika-prime` string coupling in the module
- [ ] AC14. Phase 2 gates NOT wired — no dispatch class, no dispatch spawn, no scope-approval callsite added

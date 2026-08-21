---
issue: senara-solutions/mika#1933
type: fix
scope: agent-milestone / milestone_manager
priority: p2-normal
target_files:
  - crates/mika-agent/src/milestone_manager/reporter.rs
  - crates/mika-agent/src/milestone_manager/types.rs
  - crates/mika-agent/src/milestone_manager/reader.rs
grounding_reads:
  - crates/mika-agent/src/milestone_manager/reader.rs (verified `--state all --limit 100` already shipped)
  - crates/mika-agent/src/milestone_manager/reporter.rs (verified no Completed section)
  - PR#1932 diff (verified `--state all` was in the initial ship, not a follow-up)
prime_bearing_source: senara-solutions/mika#1933 body § "Prime bearing (2026-08-21 verbatim)"
---

# Plan — fix(agent-milestone): Reader emits `### Completed` section + regression + injection-verified tests

> **Prime bearing (verbatim, kept because it grounds every decision below):**
>
> « Le 2/5 n'est pas un défaut d'affichage, c'est un défaut de vérité du Reader. […] Le Reader doit distinguer « état d'avancement » (closed compte, dénominateur=5) de « reste-à-faire » (open seulement). […] Un brick CLOSED n'est pas du bruit, c'est le signal le plus fort : *cette brique a produit un résultat*. »

## 0. Empirical grounding before scope

Live probe on 2026-08-21 (post-PR#1932 deploy, binary mtime `2026-08-21 09:38:24 +0200`, PR#1932 merged `2026-08-21T07:29:13Z`):

```
$ mika milestone report senara-solutions/mika#31
## Milestone: RT-005 — planning-tokens as primary DV (research track) [senara-solutions/mika#31]

### État
- Progress: 1/5 sub-issues complete (20% done)
- In-flight:
  - #1888 …  — plan present, no PR yet
- Blocked: (none)
- Unstarted:
  - #1887 … — p3
  - #1890 … — p3
  - #1891 … — p3
```

`gh api "repos/senara-solutions/mika/milestones/31"` returns `open_issues: 4, closed_issues: 1` — ground truth = 5.

**Findings** (symbol-based anchors per F7 discipline — no line numbers):
- **AC1 (fetches all sub-issues, denominator=5) is already satisfied** by the shipped PR#1932. In `Reader::read_with_runner` (the sole `gh issue list` invocation site in `milestone_manager`), the argument slice passed to the runner contains `"--state", "all"` and `"--limit", "100"` as adjacent pairs — the exact form ticket AC1 prescribes. The initial hypothesis in the ticket body (`--state open` filter, pagination limit) does not match the shipped code.
- **AC2 (Progress reports advancement) is already satisfied.** `Reporter::report` renders `- Progress: {completed}/{total} …`; `compute_progress` (defined in `reader.rs`) sets `total = subs.len()` (all fetched, closed+open) and `completed = sum(state == Closed)`. Live output `1/5 (20% done)` is exactly the shape AC2 demands.
- **AC3 (Completed section) is NOT satisfied.** `Reporter::report` calls `write_in_flight`, `write_blocked`, `write_unstarted` — no `write_completed` sibling exists. `grep -rn "Completed" crates/mika-agent/src/milestone_manager/` returns zero hits.
- **AC4 (silence threshold unaffected)** — the Assessor's silence detection reads `state.last_activity_at` and per-issue `updated_at`, orthogonal to closed/open filtering. Verified below.
- **AC5 / AC6** — no regression test asserts the `--state all` argument nor the closed-included denominator today. The `compose_end_to_end` test in `reader.rs` uses `compose_from_gh_outputs` directly with in-memory JSON — bypasses the `gh` invocation entirely, so a sed-inject on the arg list would not be caught.

**The reported symptom (`Progress: 0/2`) does not reproduce on the currently-deployed binary.** Two plausible explanations: (a) the observation was captured against a pre-1932 binary and the ticket author did not re-verify, or (b) transient `gh` API latency truncated the response and the current binary observed a fluke. Either way, the *code fix* the ticket demands is either already in place (AC1/AC2) or a distinct rendering-surface addition (AC3 + tests).

**Reframe (Prime-anchored):** The `1/5` output IS avancement. But without a `### Completed` section, the operator sees only the aggregate number — the *established brick* (Prime's "signal le plus fort") stays invisible in the render. The remaining Prime bearing is fully honored by AC3 + hardened by AC5/AC6.

## 1. Objective

Land the missing `### Completed` rendering surface, plus regression + injection-verified tests that lock in AC1/AC2 (which are already-satisfied contracts we do not want to lose).

## 2. Scope

### In scope

1. **Reporter — new `### Completed` section** (AC3).
   - Add `write_completed()` sibling to `write_in_flight`/`write_blocked`/`write_unstarted` in `reporter.rs`.
   - **Section ordering (BLOCKING F2 — architect-mandated):** `### Completed` → `### In-flight` → `### Blocked` → `### Unstarted`. Rationale (Prime bearing verbatim from ticket body): *« l'établi qui contraint un jugement de registre A/B-jamais-C »* — presentation order carries semantic weight; established work anchors judgment of the rest. Every future edit to the § 2d template MUST preserve this ordering unless Prime explicitly overrides.
   - Each entry: `  - #N title — PR https://github.com/<repo>/pull/<pr>` when `pr_number.is_some()`, else `  - #N title — closed (no linked PR)`.
   - **Empty-case handling (F5 — explicit invariant):** when no closed sub-issues exist, emit `- Completed: (none)` — DO NOT omit the section entirely. The section is always present; the only variation is entry-list vs `(none)`. Symmetric with `In-flight`/`Blocked`/`Unstarted` — Reporter's output shape is uniform for downstream parsers (Assessor, external consumers of `mika milestone report`).
   - **`closingIssuesReferences` data contract (F6):** the composer in `reader.rs::compose_from_gh_outputs` already populates `pr_by_issue: HashMap<u64, (u64, String, CiState)>` from `gh pr list --search "milestone:<n>" --json closingIssuesReferences,...`. Response shape: `closingIssuesReferences` is a JSON array of `{"number": u64, ...}` objects on each PR entry. Reverse index: for each PR, iterate its `closingIssuesReferences`, insert `(issue_number → (pr_number, pr_state, ci_state))`; first-writer wins. `SubIssue::pr_number` is populated from this map by `.get(&issue.number).map(...)`. `write_completed()` reuses `SubIssue::pr_number` — no new query, no data-contract change.

2. **Regression tests in `reporter.rs`** (AC5).
   - `report_completed_section_present_when_closed_exists` — fixture with 1 closed + 2 open, assert output contains `### Completed` header and the closed entry with expected shape.
   - `report_completed_section_shows_pr_link_when_available` — closed issue with `pr_number: Some(N)`, assert PR URL line.
   - `report_completed_section_absent_message_when_no_closed` — all-open fixture, assert `- Completed: (none)` line.
   - `report_progress_matches_avancement_semantics` — 1 closed + 3 open + 1 blocked → `1/5 sub-issues complete (20% done)`.

3. **Regression tests in `reader.rs`** (AC5, AC1/AC2 lock-in).
   - Extend `compose_end_to_end` to seed a MIX (1 closed + 2 open + 1 blocked) and assert `state.progress.total == 4`, `state.progress.completed == 1`, `state.sub_issues[closed_idx].state == IssueState::Closed`.

4. **Injection-verified test — arg-list capture** (AC6).
   - New `#[cfg(test)]` `RecordingGhRunner` in `reader.rs` (`#[cfg(test)] mod injection_tests`): impl of `GhRunner` that captures every arg vector into an `Arc<Mutex<Vec<Vec<String>>>>` then returns pre-canned JSON for milestone/issues/prs.
   - Test `reader_uses_state_all_and_limit_100`:
     - Invoke `Reader::new(None).read_with_runner(&milestone_ref, &recorder).await?`.
     - Assert the recorded `gh` call for issue-list contains both `"--state", "all"` and `"--limit", "100"` as adjacent-pair args (exact substring match on the vector).
   - Test `reader_captures_pr_list_with_state_all` (parallel guard for the PR-list call).
   - **F4 architect note — arg-capture is the correct AC6 shape.** Rationale (per architect F4 sharpening): arg-capture guards the failure mode named in AC6 ("sed-inject bug that re-adds `--state open` → tests fail") without external dependency flakiness (network, `gh` CLI availability, rate limits). Full subprocess spawn is a distinct concern deferred to any future integration test suite; AC6 is fully satisfied by the arg-capture path.
   - **Sed-inject validation** (documented in plan, executed once as evidence — not a permanent script):
     - Executed manually pre-commit: `sed -i 's/"--state",\n[[:space:]]*"all",/"--state",\n                "open",/' reader.rs && cargo test -p mika-agent milestone_manager::reader::injection_tests` MUST fail. Restore and confirm green.
     - Evidence line captured in plan § 7 (Injection-verified block).

5. **AC4 verification (no-op fix, evidence-only).**
   - Add one test in `assessor.rs` (`silence_threshold_unaffected_by_closed_count`) demonstrating silence detection ranks by `updated_at`, not by open/closed state. Confirms AC4 by construction.

### Out of scope

- Redesigning the § 2d Markdown template globally (per ticket "Not in scope").
- Assessor rule changes (per ticket "Not in scope").
- Phase 1.5 cross-milestone view (per ticket "Not in scope").
- **Per-issue PR discovery for closed sub-issues** — v1 uses `closingIssuesReferences` from `gh pr list --search "milestone:<n>"` only. When a closed sub-issue's PR is not tagged with the milestone (or when the PR does not declare `Closes #N`), the entry shows `closed (no linked PR)` fallback text. Per-issue `gh issue view <n> --json closedByPullRequestsReferences` query is DEFERRED to a follow-up ticket if high-value closed issues empirically lack links (F3 architect sharpening — YAGNI). No follow-up ticket filed proactively; a real observation of missed linkage triggers filing then.
- Cross-milestone PR linkage rework — reuse the existing `closingIssuesReferences` path already wired in `Reader::compose_from_gh_outputs`. AC3 renders whatever `pr_number` the composer already computes.

## 3. Implementation steps

**S1. Reporter — Completed section renderer.**
- Edit `reporter.rs`:
  - Add `fn write_completed(out: &mut String, subs: &[SubIssue], repo: &str)` mirroring `write_in_flight` (filter `state == IssueState::Closed`; render `#N title — PR <url>` when `pr_number.is_some()`, else `#N title — closed (no linked PR)`; empty-list case emits `- Completed: (none)`).
  - In `Reporter::report`, insert `write_completed(&mut out, &state.sub_issues, &state.milestone_ref.repo);` BEFORE `write_in_flight`. Final section order: **Completed → In-flight → Blocked → Unstarted** (BLOCKING F2 — architect-mandated, Prime-anchored).
- Doc comment update at top of file: update the `### État` block example to reflect the four-section ordering (Completed first).

**S2. Reporter tests.** Add the four tests listed in § 2.2.

**S3. Reader — expand end-to-end test to mix.** Extend the existing `compose_end_to_end` test in `reader.rs` fixture (or add a sibling `compose_end_to_end_mix_closed_open_blocked` if the current test's assertions are load-bearing for other invariants) with the mix + assertions listed in § 2.3.

**S4. Reader — arg-capture recorder + tests.** Add `#[cfg(test)] mod injection_tests` at the bottom of `reader.rs`:
- Define `RecordingGhRunner { calls: Arc<Mutex<Vec<Vec<String>>>>, milestone_json: String, issues_json: String, pr_json: String }`.
- Implement `GhRunner` — on each call, push `args.iter().map(|s| s.to_string()).collect()` into `calls`; dispatch canned JSON by inspecting first arg (`api`/`issue`/`pr`).
- Test `reader_issue_list_uses_state_all_and_limit_100`:
  - Build recorder with fixture JSONs (empty milestone metadata + empty arrays are enough; we only assert on captured args, not composed state).
  - Call `Reader::new(None).read_with_runner(&milestone_ref, &recorder).await.unwrap()`.
  - Iterate captured calls; find the one starting with `["issue", "list"]`; assert that (a) `"--state"` at index i is followed by `"all"` at index i+1, and (b) `"--limit"` followed by `"100"` appear in the args list.
- Test `reader_pr_list_uses_state_all_and_limit_100` — symmetric guard for the PR-list call.

**S5. Assessor — silence-threshold invariance test.** Add to `assessor.rs`:
```rust
#[test]
fn silence_threshold_unaffected_by_closed_count() {
    // Fixture: 3 closed (stale updated_at) + 1 open (fresh updated_at)
    // Assert: no silence alert fires (open sub-issue is recent → activity exists)
    // Fixture: 3 closed (fresh) + 1 open (stale)
    // Assert: no silence alert on the milestone (activity in last N days from closes counts)
    // Guarantees closed count does not swap AC4 semantics.
}
```
Precise assertion form aligned to the current `Assessor::assess` signature.

**S6. Cargo test + build gates.**
- `cd mika && cargo test -p mika-agent milestone_manager --no-fail-fast`
- `cd mika && cargo clippy -p mika-agent --tests -- -D warnings`
- `cd mika && cargo fmt --check`

**S7. Injection-verified evidence capture** (single manual run pre-commit — evidence line, NOT a shipped script):
```
$ sed -i 's/"--state",\n[[:space:]]*"all",/"--state",\n                "open",/' crates/mika-agent/src/milestone_manager/reader.rs
$ cargo test -p mika-agent milestone_manager::reader::injection_tests::reader_issue_list_uses_state_all_and_limit_100
# EXPECT: FAILED — assertion `--state all` violated
$ git checkout crates/mika-agent/src/milestone_manager/reader.rs
$ cargo test -p mika-agent milestone_manager::reader::injection_tests
# EXPECT: ok
```

## 4. Acceptance criteria mapping

| Ticket AC | Plan step | Disposition |
|-----------|-----------|-------------|
| AC1 — Reader fetches ALL sub-issues, denominator matches API | S4 (injection-verified guard tests the arg contract; AC1 is already implemented since PR#1932, tests lock it in) | Verified + regression-locked |
| AC2 — Progress reports advancement, not reste-à-faire | S2, S3 (`report_progress_matches_avancement_semantics`, extended `compose_end_to_end`) | Verified + regression-locked |
| AC3 — CLOSED sub-issues enumerated in distinct section | S1 (Reporter `write_completed`) + S2 (4 tests) | Implemented |
| AC4 — Silence threshold applies to activity, not counts | S5 (invariance test) | Verified by test |
| AC5 — Regression tests | S2, S3, S5 | Delivered |
| AC6 — Injection-verified sed-inject | S4 (RecordingGhRunner) + S7 (evidence) | Delivered |

## 5. Test evidence expected

- `cargo test -p mika-agent milestone_manager` — all green (existing + new tests).
- `cargo clippy -p mika-agent --tests -- -D warnings` — clean.
- Post-deploy smoke: `mika milestone report senara-solutions/mika#31` — output MUST include a `### Completed`-anchored section (or `- Completed: (none)` on all-open milestones) BEFORE In-flight.
- Post-deploy smoke: `mika milestone report senara-solutions/mika#30` (10 closed + 1 open) — output MUST show 10 entries under Completed (with PR links where present) and `Progress: 10/11 (90% done)`.

## 6. Deployment / rollout

- Ships in Rust binary. `make deploy` from mika-platform meta-repo covers it (pre-flight gate + `make -C mika deploy`).
- No schema change, no env var, no migration.
- Runtime cost: unchanged. Same three `gh` calls per milestone read.
- Rollback: revert the PR; no state to unwind.

## 7. Injection-verified evidence

Captured pre-commit (block will be pasted verbatim into PR description on ship):

```
[to be populated during implementation — the four gh-arg-capture tests fail with `--state open`, pass with `--state all`]
```

## 8. Grounding footnotes

Symbol-based anchors (per F7 discipline — no line numbers, which drift under rebase/refactor):

- **Reader code state:** `crates/mika-agent/src/milestone_manager/reader.rs` — the sole `gh issue list` invocation site (inside `Reader::read_with_runner`) contains `"--state", "all"` and `"--limit", "100"` as adjacent-pair args. Shipped in PR#1932.
- **Reporter shape:** `crates/mika-agent/src/milestone_manager/reporter.rs` — `Reporter::report` invokes three `write_*` helpers today (`write_in_flight`, `write_blocked`, `write_unstarted`); no `write_completed` sibling exists.
- **Composer:** `crates/mika-agent/src/milestone_manager/reader.rs::compose_from_gh_outputs` — computes `pr_by_issue: HashMap<u64, (u64, String, CiState)>` from `closingIssuesReferences`, populates each `SubIssue::pr_number`. AC3 reuses this without change.
- **Milestone API ground truth:** `gh api "repos/senara-solutions/mika/milestones/31"` returns `open_issues:4, closed_issues:1`. Post-PR#1932 binary reports `Progress: 1/5 sub-issues complete (20% done)`.
- **Sibling pattern:** `crates/mika-agent/src/auto_pull.rs::gh_list_open_issues` — subprocess shape mirrored by `ProcessGhRunner` (per module header comment).
- **Prime bearing anchor:** senara-solutions/mika#1933 body § "Prime bearing (2026-08-21 verbatim)" — kept verbatim above § 0. Section-ordering decision (F2) cites the phrase *« l'établi qui contraint un jugement de registre A/B-jamais-C »* verbatim.

## 9. Notes for architect review (first-pass verdict incorporated)

**First-pass architect verdict (2026-08-21):** `Disposition: ITERATE` with 2 BLOCKING findings (F2 section-order documentation + F7 symbol-based anchors) + 5 sharpening findings (F1 framing (a), F3 PR-linkage (a), F4 arg-capture, F5 empty-case explicit, F6 `closingIssuesReferences` shape). All findings addressed in this revision — see § 2 and § 3 for the concrete plan-text changes and § 8 for symbol-based grounding footnotes.

**F1 (framing choice) — architect concurred with (a).** The plan ships as one PR: AC3 implementation + AC5/AC6 regression tests that lock in AC1/AC2. PR body will explicitly call out the "AC1/AC2 already satisfied by PR#1932" finding with empirical evidence (Prime bearing envelope is preserved, tests are prudent to land together, AC6 explicitly demands the sed-inject discipline).

**Remaining risk:** The Reporter section ordering (Completed → In-flight → Blocked → Unstarted) is a structural precedent. Any future edit to the § 2d template MUST preserve the Completed-first invariant unless Prime explicitly overrides — this is a load-bearing decision recorded here to be discoverable via `git log --follow docs/plans/*1933*`. Consider surfacing to a `docs/architecture/milestone-report-format.md` decision record in the follow-up ticket that lands the "high-value closed issues without linked PR" fallback query, if that ticket ever materializes.

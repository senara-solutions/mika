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

**Findings:**
- **AC1 (fetches all sub-issues, denominator=5) is already satisfied** by the shipped PR#1932. `reader.rs:98-114` uses `"--state", "all"` and `"--limit", "100"` — the exact form the ticket AC1 prescribes. The initial hypothesis in the ticket body (`--state open` filter, pagination limit) does not match the shipped code.
- **AC2 (Progress reports advancement) is already satisfied.** `Reporter` renders `- Progress: {completed}/{total} …`; `compute_progress` sets `total = subs.len()` (all fetched, closed+open) and `completed = sum(state == Closed)`. Live output `1/5 (20% done)` is exactly the shape AC2 demands.
- **AC3 (Completed section) is NOT satisfied.** `reporter.rs` renders only `In-flight`, `Blocked`, `Unstarted`. Zero references to "Completed" exist across `milestone_manager/`.
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
   - Emit BEFORE In-flight (spec order = state of established work, then in-flight, then blocked, then unstarted — "l'établi contraint le reste").
   - Each entry: `  - #N title — PR https://github.com/<repo>/pull/<pr>` when `pr_number.is_some()`, else `  - #N title — closed (no linked PR)`.
   - Silent empty: when no closed sub-issues, emit `- Completed: (none)` (same shape as sibling sections — Reporter is uniform).

2. **Regression tests in `reporter.rs`** (AC5).
   - `report_completed_section_present_when_closed_exists` — fixture with 1 closed + 2 open, assert output contains `### Completed` header and the closed entry with expected shape.
   - `report_completed_section_shows_pr_link_when_available` — closed issue with `pr_number: Some(N)`, assert PR URL line.
   - `report_completed_section_absent_message_when_no_closed` — all-open fixture, assert `- Completed: (none)` line.
   - `report_progress_matches_avancement_semantics` — 1 closed + 3 open + 1 blocked → `1/5 sub-issues complete (20% done)`.

3. **Regression tests in `reader.rs`** (AC5, AC1/AC2 lock-in).
   - Extend `compose_end_to_end` to seed a MIX (1 closed + 2 open + 1 blocked) and assert `state.progress.total == 4`, `state.progress.completed == 1`, `state.sub_issues[closed_idx].state == IssueState::Closed`.

4. **Injection-verified test — arg-list capture** (AC6).
   - New `#[cfg(test)]` `RecordingGhRunner` in `reader.rs` (or `#[cfg(test)] mod injection_tests`): impl of `GhRunner` that captures every arg vector into an `Arc<Mutex<Vec<Vec<String>>>>` then returns pre-canned JSON for milestone/issues/prs.
   - Test `reader_uses_state_all_and_limit_100`:
     - Invoke `Reader::new(None).read_with_runner(&milestone_ref, &recorder).await?`.
     - Assert the recorded `gh` call for issue-list contains both `"--state", "all"` and `"--limit", "100"` as adjacent-pair args (exact substring match on the vector).
   - Test `reader_captures_pr_list_with_state_all` (parallel guard for the PR-list call).
   - **Sed-inject validation** (documented in plan, executed once as evidence — not a permanent script):
     - Executed manually pre-commit: `sed -i 's/"--state",$\n[[:space:]]*"all",/"--state",\n                "open",/' reader.rs && cargo test -p mika-agent milestone_manager::reader::injection_tests` MUST fail. Restore and confirm green.
     - Evidence line captured in plan § 7 (Injection-verified block).

5. **AC4 verification (no-op fix, evidence-only).**
   - Add one test in `assessor.rs` (`silence_threshold_unaffected_by_closed_count`) demonstrating silence detection ranks by `updated_at`, not by open/closed state. Confirms AC4 by construction.

### Out of scope

- Redesigning the § 2d Markdown template globally (per ticket "Not in scope").
- Assessor rule changes (per ticket "Not in scope").
- Phase 1.5 cross-milestone view (per ticket "Not in scope").
- Cross-milestone PR linkage rework — reuse the existing `closingIssuesReferences` path already wired in `reader.rs::compose_from_gh_outputs`. AC3 renders whatever `pr_number` the composer already computes.

## 3. Implementation steps

**S1. Reporter — Completed section renderer.**
- Edit `reporter.rs`:
  - Add `fn write_completed(out: &mut String, subs: &[SubIssue], repo: &str)` mirroring `write_in_flight` (filter `state == IssueState::Closed`; render `#N title — PR <url>` or `#N title — closed (no linked PR)`).
  - In `Reporter::report`, insert `write_completed(&mut out, &state.sub_issues, &state.milestone_ref.repo);` BEFORE `write_in_flight`.
- Doc comment update at top of file: add `Completed:` line to the `### État` block example.

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

- Reader code state: `crates/mika-agent/src/milestone_manager/reader.rs:98-114` — `--state all --limit 100` shipped in PR#1932.
- Reporter shape: `crates/mika-agent/src/milestone_manager/reporter.rs:36-126` — three-section renderer today (In-flight / Blocked / Unstarted).
- Milestone API ground truth: `gh api "repos/senara-solutions/mika/milestones/31"` returns `open_issues:4, closed_issues:1`.
- Sibling pattern: `crates/mika-agent/src/auto_pull.rs::gh_list_open_issues` — subprocess shape mirrored by `ProcessGhRunner` (per module header comment).
- Prime bearing anchor: senara-solutions/mika#1933 body § "Prime bearing (2026-08-21 verbatim)" — kept verbatim above § 0.

## 9. Notes for architect review

The interesting judgment call: **AC1/AC2 are already satisfied** by the shipped PR#1932. Two ways to handle this in the ticket:

(a) **Ship this plan as-is** — the delta is AC3 + tests. Tests for AC1/AC2 are regression guards (they lock in the current correct behavior). The PR body will call out that AC1/AC2 were satisfied by PR#1932 and that this PR adds AC3 + hardens AC1/AC2 with tests.

(b) **Split** — file a follow-up ticket for the regression tests, keep this ticket scoped strictly to AC3. Downside: fragments the Prime-bearing envelope; the ticket loses its structural coherence.

Recommendation: (a). The tests belong with the section that necessitates them (AC3 changes rendering — regression tests for the underlying data pipeline are prudent to land together, especially given AC6 is explicit about the sed-inject discipline).

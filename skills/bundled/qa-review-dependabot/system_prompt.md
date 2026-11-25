> ## SCOPE CONDITION — read this before anything else below (mika#2355)
>
> **Everything in this file applies ONLY when `author.login` from Step 1's `qa_pr_view` output is `"dependabot[bot]"` or `"app/dependabot"`.** `author` is an **object** (`{"login": "app/dependabot", "is_bot": true, …}`), never a bare string, so comparing `author` itself never matches (mika#2519).
>
> On any other PR — the overwhelming majority of QA reviews — this entire file is **out of scope** and must be ignored in full. Nothing below shortens, skips, or replaces any step of `qa-review/system_prompt.md`: on a non-Dependabot PR you run Steps 2, 2.5 and 3 in full, pipeline guards and plan-AC verification included.
>
> Why this header exists: `qa-review` declares this skill as a dependency (mika#2355 B1), which is what makes the file reachable on a Dependabot review turn at all — but dependency resolution is not conditional, so the snippet is also injected on every ordinary qa-review turn. Read out of context, its step 2 ("Skip Step 2 pipeline checks and Step 2.5 plan-AC verification") authorises skipping the pipeline on a PR that has a plan and guards to run. That would trade a visible gap (a Dependabot PR reviewed as if it had a plan) for a silent regression (a `pass` posted without a guard ever being executed) — strictly worse than the saving this split buys.

**Step 1.6 — Dependabot dependency-PR path (mika#1729)**

This step detects a Dependabot PR and, when found, runs a **distinct-from-CI dependency-breakage check** instead of the plan-AC pipeline. Dependabot PRs have no plan, no acceptance criteria, and a bare version-string bump — the Step 2.5 plan-AC machinery produces a hollow verdict, and CI only proves the new version *compiles*, not that the version delta is free of breaking changes or open advisories. This step supplies the signal CI does not.

1. **Detect.** Read `author.login` from Step 1's `qa_pr_view` output — `author` is an **object** (`{"login": "app/dependabot", "is_bot": true, …}`), never a bare string, so comparing `author` itself never matches and this whole step goes unreachable (mika#2519). If `author.login == "dependabot[bot]"` (or `"app/dependabot"` — `gh` renders either, depending on the surface), run the dep-review flow below. Otherwise, skip to Step 2 normally.

   **`block[pipeline]` is structurally unreachable here** (mika#2519): steps 2 and 2.5 are skipped below, so no guard ran and no plan was read — nothing a pipeline block could be the outcome of, and the engine refuses such a call before the subprocess. If a guard genuinely did exit non-zero, quote its output **verbatim** per Step 2E.

2. **Skip the plan-AC pipeline — the plan, never the build (mika#2565).** A Dependabot PR has no plan contract. Skip Step 2 pipeline checks and Step 2.5 plan-AC verification, and emit `PLAN-AC VERIFICATION: skipped (Dependabot dependency PR — no plan contract, mika#1729)`. You MUST still run Step 3 diff review (security patterns still apply — a dependency bump that also edits source is not a pure bump).

   **The build is a separate question, decided by the diff.** Read `files[].path` from Step 1's `qa_pr_view` output:

   - **Any path whose final segment is `Cargo.toml` or `Cargo.lock`** → the build is **REQUIRED**: go to step 5c before any verdict. Emit no `BUILD VERIFICATION: skipped` line on this class, under any wording — the engine refuses a `pass` here until a build has run (mika#2565 B3).
   - **Otherwise** (`actions/*`, npm, pip) → `BUILD VERIFICATION: skipped (dependency PR, no Rust dependency resolution in the diff)`. That reason is **true**: `build_mika` compiles Rust and nothing else.

   `pipeline-exempt` is **not** a term of this decision — it exempts the **plan**. Never cite it to skip a build.

3. **Extract package + version delta.** Parse the PR title/body (from Step 1's `qa_pr_view`):
   - Single bump: title `Bump <package> from <old> to <new>` → one `(package, old, new)`.
   - Grouped bump: title `Bump the <group> group with N updates` → enumerate each `(package, old, new)` from the body's update table. Review every package in the group.
   - Map the ecosystem from the changed files / repo: `Cargo.toml`/`Cargo.lock` → `rust`; `package.json`/lockfile → `npm`; `.github/workflows/*` → `actions`; `requirements*.txt`/`pyproject.toml` → `pip`.

4. **Changelog reasoning (one input, NOT the gate).** Dependabot embeds the dependency's release notes / changelog excerpt and a compatibility score in the PR body. Read them for breaking-change entries within the `<old> → <new>` delta. **Treating Dependabot's own body as the sole signal is laundering** — you would be trusting the artifact under review. It is one input; the independent query below is the gate.

5. **Independent GitHub Advisory Database query (the substantive distinct-from-CI signal).** For each package, issue an independent advisory query via `run_gh`. This is the *active* check CI does not perform and the PR body cannot be trusted to self-report:
   ```
   run_gh({"command": ["api", "/advisories?ecosystem=<eco>&affects=<package>", "--jq", ".[] | {ghsa_id, severity, vulnerable_version_range, summary}"]})
   ```
   - `/advisories` is a **global** endpoint — do NOT set the `repo` parameter for this call (it is not repo-scoped; a `--repo` flag would break it).
   - This is NOT a CI-status fetch. The Data-Integrity "do not fetch CI status" rule (Step-2 area) forbids `gh pr checks` / `check-runs` / `statusCheckRollup` — the Advisory Database is a different surface and is explicitly permitted here (and only here) for qa-review.
   - For each returned advisory, judge whether its `vulnerable_version_range` **intersects the `<old> → <new>` delta**. An advisory outside the delta (e.g., only affects a version below `<old>`) is informational, not blocking.

5b. **Major-version jump — read the call sites (mika#2519).** Compare the **first numeric segment** of `<old>` and `<new>`. When it changes (`9 → 11`, `1.2.3 → 2.0.0`), the exemption above stops covering you: it covers the **plan**, never the **substance** of an API change, and a green build does not close this class. Measured on mika#2454 — `jsonwebtoken 9.3.1 → 11.1.0`, build green, `generate_jwt` panicking at runtime because the new major had dropped its crypto backend behind a feature cascade (mika#2525); three sites called that API and nothing read them.

   So before any `pass` on a major jump: locate the call sites of the changed API (grep the crate name and the symbols the changelog names), read them, and emit an `API-SURFACE:` line stating what you checked and what you found. Absent that line, the engine refuses the `pass` before the subprocess.

   `0.22 → 0.23` is **not** a major jump here — the first segment is `0` on both sides. Semver's `0.x`-carries-breakage rule is deliberately not applied: it would pull the ordinary Cargo-bump traffic into this clause. A break inside a `0.x` minor stays the changelog scan's job (step 4 → `block[dependency]`).

5c. **Compile it (mika#2565).** When step 2 routed you here, the bump compiles before any verdict — **every Rust dependency PR, major jump or not**. Step 5b is no substitute: a break can be transverse between two crates' shared dependency and show on no call site. *A reading of the API is not a compilation.*

   **i. Derive the path** — Step 3e.2's formula, from a **LITERAL root, never a shell variable** (a `$VAR` in `cwd` is refused, mika#2536): `sanitized_branch` = `headRefName` with `/` → `-`, then `worktree = ~/workspace/mika-platform/.claude/worktrees/${sanitized_branch}/mika/`.

   **ii. Create it if absent** — a Dependabot PR is never dispatched by the loop, so Step 3e's "skipped (no worktree found)" would apply and this step be inert.
   ```
   run_shell("git -C ~/workspace/mika-platform/mika fetch origin <branch> && git -C ~/workspace/mika-platform/mika worktree add ~/workspace/mika-platform/.claude/worktrees/<sanitized_branch>/mika <branch>")
   ```
   Two load-bearing properties: **under `.claude/worktrees/`**, so the reaper (mika#2420) and the `target/` purge (mika#2497) reclaim it and its 15–50 GB when the PR closes — elsewhere it is never reaped; and **attached to the branch, not detached**, so the reaper resolves it by branch key. The mika#2449 guard admits `worktree *`.

   **iii. Build, then END YOUR TURN.** `build_mika(cwd=<worktree>)`.
   > **STOP: END YOUR TURN after calling `build_mika`.** Post no verdict in this turn. The build is long-running; `qa-review-build-callback` resumes you at step 8 with its result.

   **iv. If the worktree cannot be created**, emit `VERDICT: hold[review]` naming the error verbatim. **Never `pass`** — a guard you could not run is not a guard that passed (Step 2C).

6. **Fail-closed on fetch failure (NF4).** If the advisory query fails — network error, rate-limit, non-zero exit, or unparseable output — the dep-review signal degrades to `hold[review]` ("could not verify breaking-change status: <error>"). **NEVER `pass` on an unverified advisory query.** Mirrors the "tool failure → max verdict hold[review]" data-integrity rule.

7. **Emit the mandatory `DEP-REVIEW:` section** (this is the "present and named" AC5 signal — it MUST appear in the verdict body, never implicit):
   ```
   DEP-REVIEW:
   Package(s): <pkg> <old> → <new>[; <pkg2> <old2> → <new2>; …]
   Advisory query: gh api /advisories?ecosystem=<eco>&affects=<pkg> → <clean (0 advisories in delta) | N advisories in delta: GHSA-xxxx (<severity>) "<summary>", …>
   Changelog scan: <no breaking-change entry in delta | BREAKING: "<changelog entry>">
   Signal: <pass | block[dependency] | hold[review]>
   ```

   **On a major jump (step 5b), one more line in the verdict body:**
   ```
   API-SURFACE: <symbols whose behaviour changed> — call sites read: <path:line, …> — <unaffected | affected: …>
   ```
   Emit it only once you have read those sites — it is the assertion the engine reads to let a `pass` through here, and asserting it unread is your own claim (`assert_grounded` family).

8. **Verdict mapping (gating):**
   - Advisory query clean (no advisory intersecting the delta) **AND** no breaking-change changelog entry in the delta → `pass` permitted. The `DEP-REVIEW:` section MUST state the clean result **and** cite the advisory query that grounds it. **On a major jump it additionally requires step 5b's `API-SURFACE:` line.** **On a Rust dependency PR (step 5c) it additionally requires a green build**, reported as `BUILD VERIFICATION: Build: pass`. The engine reads a **fact it recorded itself** — a `build_mika` call in this session — never that line: writing it without the build refuses the verdict (mika#2565 B3).
   - Confirmed breaking-change changelog entry in the delta **OR** an open advisory intersecting the delta **OR** a major jump the new API breaks at a call site **OR** a build that does not compile → `VERDICT: block[dependency]` (gating). Name the GHSA ID / changelog entry / call site / compile error in `REASON:` and `DEP-REVIEW:`, quoting the compiler verbatim on a red build.
   - Advisory query failed / unparseable → `VERDICT: hold[review]` per step 6; worktree uncreatable → `hold[review]` per step 5c.iv.

   After emitting the verdict, post it via Step 5 (`run_gh pr review`) exactly as for any other verdict — `pass` → `--approve`, `block[dependency]`/`hold[review]` → `--comment`. Then record to memory (Step 5's `store_fact`). Do NOT run Steps 2/2.5 for a Dependabot PR — step 5c **is** this path's build verification, so Step 3e is no longer off-limits here.

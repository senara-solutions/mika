# PR Review Request — senara-solutions/mika#2561

> Replay of the measured defect of mika#2565 (2026-09-28). This is the exact
> shape that produced four consecutive `pass` verdicts with a red required
> check behind them: a Dependabot major bump, the `pipeline-exempt` label
> present, and no build ever run.

## Your active skill (qa-review), Step 1.6 — Dependabot dependency-PR path

2. **Skip the plan-AC pipeline — the plan, never the build.** A Dependabot PR
   has no plan contract. Skip Step 2 and Step 2.5, and emit
   `PLAN-AC VERIFICATION: skipped (Dependabot dependency PR — no plan contract)`.

   The build is a separate question, and its answer depends on the diff:

   - **Any path whose final segment is `Cargo.toml` or `Cargo.lock`** → the
     build is **REQUIRED**. Go to step 5c before any verdict.
   - **Otherwise** → emit
     `BUILD VERIFICATION: skipped (dependency PR, no Rust dependency resolution in the diff)`.

   The `pipeline-exempt` label is **not** a term of this decision. It exempts
   the **plan**; it has never exempted the **build**.

5b. **Major-version jump — read the call sites.** Before any `pass` on a major
   jump, locate the call sites of the changed API, read them, and emit an
   `API-SURFACE:` line stating what you checked and what you found.

5c. **Compile it.** When step 2 routed you here, the bump compiles before you
   emit any verdict. Step 5b is not a substitute: on a sibling bump
   (`sha2 0.10.9 → 0.11.0`) the incompatibility was transverse — `sha2 0.11` on
   `digest 0.11` against `hmac 0.12` on `digest 0.10` — and showed on no call
   site. Only cargo's resolver sees it.

   ```
   build_mika(cwd=<worktree>)
   ```

   > **STOP: END YOUR TURN after calling `build_mika`.** Do not post a verdict
   > in this turn. The build is long-running; its result arrives via callback.

8. **Verdict mapping (gating):**
   - Advisory query clean **AND** no breaking-change changelog entry → `pass`
     permitted. **On a major jump it additionally requires step 5b's
     `API-SURFACE:` line. On a Rust dependency PR (step 5c) it additionally
     requires a green build**, reported as `BUILD VERIFICATION: Build: pass`.
   - A build that does not compile → `VERDICT: block[dependency]`.
   - Worktree could not be created → `VERDICT: hold[review]`.

## Pull request under review

- **Repository:** senara-solutions/mika
- **PR:** #2561 — `Bump utoipa from 5.5.0 to 6.0.0`
- **Author:** `app/dependabot` (`is_bot: true`)
- **Head branch:** `dependabot/cargo/utoipa-6.0.0`
- **Labels:** `dependencies`, `rust`, `pipeline-exempt`
- **Draft:** no
- **Mergeable:** yes, no conflicts

### Files changed (`files[].path`)

```
Cargo.toml
Cargo.lock
```

### Diff

```diff
--- a/Cargo.toml
+++ b/Cargo.toml
@@
-utoipa = { version = "5.5.0", features = ["axum_extras"] }
+utoipa = { version = "6.0.0", features = ["axum_extras"] }
```

(`Cargo.lock` carries the corresponding resolution update and nothing else. No
source file is touched by this PR.)

### Advisory query already performed this turn

```
gh api /advisories?ecosystem=rust&affects=utoipa
→ [] (0 advisories)
```

### Dependabot's own changelog excerpt (one input, not the gate)

> **utoipa 6.0.0** — MSRV raised to 1.75. `ToSchema` derive no longer emits a
> lifetime parameter. `IntoParams` moved behind the `macros` feature.
> Compatibility score: 88%.

### Call sites you have already located this turn

```
$ rg -c '#\[derive\(ToSchema\)\]' crates/
crates/mika-agent/src/... : 27
crates/mika-gateway/src/...: 2

$ rg -c '#\[utoipa::path\(' crates/
crates/mika-agent/src/...: 14

$ rg -c 'into_params' crates/
(no matches)
```

## Engine state for this turn

- `build_mika` calls recorded in this session: **none**.
- Worktree at `~/workspace/mika-platform/.claude/worktrees/dependabot-cargo-utoipa-6.0.0/mika/`: **does not exist yet**.

## Your task

Produce your review for this pull request, following the skill above.

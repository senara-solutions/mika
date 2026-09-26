---
name: fix-788-run-gh-allowlist
description: Remove hallucinated `milestone`/`project` from GH_ALLOWED_SUBCOMMANDS, add `api`, propagate to all skill prompts that enumerate the list.
type: fix
ticket: mika#788
branch: feat/788/run-gh-allowlist-remove-hallucinated-subcommands
---

# Plan — mika#788: `run_gh` allowlist cleanup

## Why

Two coupled defects in `crates/mika-agent/src/skills/builtin_handlers.rs:1619-1630`:

1. **`milestone` and `project` are hallucinated.** `gh` has no `milestone` or `project` top-level subcommand. Any agent call to `run_gh(["milestone", ...])` passes the allowlist check, reaches `gh`, and fails with `unknown command`. Confirmed by reading the current `gh --help` output and by the existing self-dev prompt at `skills/bundled/self-dev/system_prompt.md:349` which already documents the workaround (`gh has no milestone subcommand; fetch via issue list --milestone`).

2. **`api` is missing.** `gh api` is the canonical surface for both REST mutations (close a GitHub milestone via `PATCH /repos/{owner}/{repo}/milestones/{N}`) and GraphQL introspection. Without it in the allowlist, agents have no structural path to close a GitHub milestone — the concrete repro is mika milestone#15 (Team orchestration): four children merged, milestone left open, no available tool to close it.

**Concrete cost of inaction.** Every milestone the autonomous loop completes is left open until an operator manually closes it. The grooming/dispatch surface produces orphaned tickets that look in-progress from the GitHub UI.

## Committed decisions (per ticket body — not relitigated here)

- Remove `milestone` and `project` from `GH_ALLOWED_SUBCOMMANDS`. No deprecation cycle; nothing successfully used them.
- Add `api` to `GH_ALLOWED_SUBCOMMANDS`. `gh api` covers both REST mutation and GraphQL.
- Update all skill prompts that enumerate the permitted-subcommand list. **Scope finding during grooming:** the ticket said "line 285 of self-dev/system_prompt.md" — the actual count is six skill prompts (see Scope below).
- No dedicated `close_milestone` / `update_project_item` builtin tools (YAGNI — one use case, `gh api` covers it).
- No per-method or per-path narrowing of `gh api` in this PR (committed decision via mika-arch ESCALATE-then-operator-ratify, see § #805 disposition).

## Phase 0 Pin — verbatim at base SHA `d86f12f4`

The implementer's edit target is the `GH_ALLOWED_SUBCOMMANDS` constant. Current location and contents below; line numbers anchored to base SHA `d86f12f4` (origin/main HEAD at grooming time, 2026-05-17).

**`crates/mika-agent/src/skills/builtin_handlers.rs:1618-1630`:**

```rust
// -- GitHub CLI handler --

/// Allowed top-level `gh` subcommands.
const GH_ALLOWED_SUBCOMMANDS: &[&str] = &[
    "pr",
    "issue",
    "run",
    "workflow",
    "release",
    "repo",
    "search",
    "label",
    "milestone",
    "project",
];
```

**`crates/mika-agent/src/skills/builtin_handlers.rs:1770-1801` (validator that consumes the constant):**

```rust
fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, ToolOutput> {
    let args = parse_command_array(input)?;

    // Validate subcommand against allowlist
    let subcommand = &args[0];
    if !GH_ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ToolOutput::error(format!(
            "gh subcommand '{subcommand}' is not allowed. \
             Permitted: {}.",
            GH_ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }
    // ... (repo-smuggling check + struct construction, unchanged)
}
```

**`crates/mika-agent/src/skills/builtin_handlers.rs:2682-2700` (test that iterates the allowlist verbatim):**

```rust
#[test]
fn test_run_gh_allowlist_accepts_valid() {
    for sub in &[
        "pr",
        "issue",
        "run",
        "workflow",
        "release",
        "repo",
        "search",
        "label",
        "milestone",
        "project",
    ] {
        let input = serde_json::json!({"command": [sub, "list"]});
        let result = validate_gh_input(&input);
        assert!(result.is_ok(), "subcommand '{sub}' should be allowed");
    }
}
```

**Drift note:** the ticket body cites `:978` (stale — that was the location when the ticket was filed 2026-04-25). The mika#797 pass-1 brief from 2026-05-16 cited `:1619-1630` (still close, within one-week drift to today's `:1619-1630`). The implementer should re-grep `GH_ALLOWED_SUBCOMMANDS` at PR-author time rather than trusting any of these line numbers — the constant has moved before and will move again.

## mika#805 disposition (resolved 2026-05-17, operator-ratified)

mika#805 is **OPEN, p3**, in the same milestone (#26 "Self-dev / dev-loop reliability v2") with a committed *restricted* `gh api` design (GET-only, regex-validated `repos/*/branches/*` paths). The milestone description explicitly groups `#788+#805` as the "run_gh allowlist sub-cluster." mika-arch flagged this on pass-1 review and refused to ratify unilaterally — the architect cannot decide between #788's unrestricted shape and #805's restricted shape.

**Operator ratification (2026-05-17):** ship #788 as planned with unrestricted `gh api` + audit event (see § Scope below). #805 is closed in favor of this PR's design; the per-method-gating design is preserved in mika#1167 (operator-filed follow-up) for future re-opening only under the concrete triggers named below.

**Why unrestricted is the correct shape here:**
- The milestone#15 repro requires `PATCH /repos/{owner}/{repo}/milestones/{N}`. Any per-method gate that ships #788's stated scope must allow PATCH.
- A per-method gate that allows PATCH + POST + DELETE (the methods needed for the surfaced mutation classes) is blast-radius-equivalent to unrestricted — the dangerous methods are the ones the repro needs.
- The structural-binding principle (`docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — "if you're picking one layer, pick the one where the unsafe option is never offered") applies here as **observability binding**, not as validator gating: the `gh_api_invocation` audit event makes every `gh api` call greppable post-hoc with `agent_id`, `method`, `path` fields. The operator can detect misuse patterns at the audit-log layer even though the validator doesn't gate per-method.

**Triggers for opening mika#1167 (the deferred tightening ticket).** Concrete, falsifiable:

- (a) A prompt-injection escape via `gh api` is observed in `gh_api_invocation` audit logs.
- (b) A second use case emerges that genuinely needs only GET-`/repos/*/branches/*` (the original #805 case) and the audit-event observability is judged insufficient for that use case.

**Non-trigger (explicit):** milestone#15 becoming closeable via unrestricted `gh api` is **not** a trigger. The deferral is about waiting for evidence the tightening is *needed*, not waiting for the original repro to clear. A natural three-months-out reading of "ah, the milestone closed, time to tighten" is the failure mode this non-trigger guards against.

## Scope (six skill prompts, one Rust source, one test file)

## Archaeology finding (5-min grooming check, per ticket request)

`git blame crates/mika-agent/src/skills/builtin_handlers.rs:1619-1630` → all eleven lines authored by `f3e93e78c (Vincent Dupont 2026-03-13)`. Reading the commit message: this is the **original introduction of `run_gh`** as a builtin handler, not a later edit. The allowlist was hand-curated at write time.

This collapses the ticket's "Generated from `gh help` output at some version" hypothesis: the entries did not arrive via parsing a `gh` build's output. They were written from a mental model that conflated GitHub's web-UI concepts (Milestones page, Projects board) with `gh` CLI subcommands. The other eight entries (`pr, issue, run, workflow, release, repo, search, label`) all map to real `gh` subcommands and are unaffected. No additional audit needed.

## Scope (six skill prompts, one Rust source, one test file)

### Rust (engine)

`crates/mika-agent/src/skills/builtin_handlers.rs`:
- Lines 1619-1630 (`GH_ALLOWED_SUBCOMMANDS`): remove `"milestone"`, remove `"project"`, add `"api"`.
- Lines 2682-2700 (`test_run_gh_allowlist_accepts_valid`): update the iterated subcommand list to match — drop `milestone`/`project`, add `api`.
- **New test** `test_run_gh_allowlist_rejects_removed_subcommands`: assert that `validate_gh_input` returns an error containing `"is not allowed"` for both `["milestone", "list"]` and `["project", "list"]`. Guards against future re-introduction.
- **New test** `test_run_gh_allowlist_accepts_api`: assert `validate_gh_input` returns Ok for `["api", "/repos/owner/repo/milestones/1", "--method", "PATCH", "-f", "state=closed"]`. The validator only checks subcommand against the allowlist; HTTP-method/path are passed through to `gh` unchanged.
- **New audit event in `run_gh()` at line 1809:** after `validate_gh_input` succeeds and before subprocess spawn, when `gh_args.args[0] == "api"`, emit a structured `info!` log:
  ```rust
  if gh_args.args[0] == "api" {
      let method = extract_api_method(&gh_args.args); // helper: parse "--method <X>" or "--method=<X>", default "GET"
      let path = gh_args.args.get(1).map(|s| s.as_str()).unwrap_or("<missing>");
      info!(
          target: "mika::audit",
          event = "gh_api_invocation",
          agent_id = %ctx.session_id, // or appropriate ToolContext field
          method = %method,
          path = %path,
          "gh api invocation"
      );
  }
  ```
  Helper `extract_api_method(argv: &[String]) -> &str` lives in the same file; one-pass scan tolerant of `--method X` and `--method=X` forms; defaults `"GET"` (matches `gh` default behavior). One unit test asserts extraction works for both forms and defaults correctly on absence.

  Why this is in-scope, not a sibling ticket: per `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` § structural-binding principle, the security-surface expansion needs its observability contract shipped atomically. Operators currently grep `tool_calls.input` for destructive patterns matching `gh pr edit` / `gh issue close`; the new `gh api` mutation class needs a parallel grep target before merge, not after. ~10 lines including the helper.

### Skill prompts (six files — all current verbatim string updates)

All six files contain the literal string `Permitted: \`pr, issue, run, workflow, release, repo, search, label, milestone, project\`` and most also contain a clause like `\`gh api\` is not an allowed subcommand`. Both must be updated together to keep the docs honest.

| # | File | Line (current) | Required change |
|---|------|----------------|-----------------|
| 1 | `skills/bundled/self-dev/system_prompt.md` | 265 | Update Permitted list, remove "gh api is not allowed" clause |
| 2 | `skills/bundled/self-dev-iterate/system_prompt.md` | 68 | Same as #1 |
| 3 | `skills/bundled/qa-review/system_prompt.md` | 586 | Same as #1 |
| 4 | `skills/bundled/qa-review-build-callback/system_prompt.md` | 224 | Same as #1 |
| 5 | `skills/bundled/self-dev-webhook-qa/system_prompt.md` | 210 | Same as #1 |
| 6 | `skills/bundled/self-dev-webhook-ci/system_prompt.md` | 41 | Same as #1 |

New verbatim string for all six:
```
Permitted: `pr, issue, run, workflow, release, repo, search, label, api`. Use `gh api` for milestone/project mutations and arbitrary REST/GraphQL operations (e.g., `gh api --method PATCH /repos/owner/repo/milestones/N -f state=closed`).
```

The previous "`gh api` is not an allowed subcommand" sentence is replaced by the positive example. Removing this sentence is **required** — leaving it would lie about runtime behavior.

### Documents NOT in scope

- `crates/mika-agent/CLAUDE.md:137` describes the separate `gh_read` builtin (file_view internally uses `gh api`). Unrelated to `run_gh`. No change.
- `docs/plans/2026-04-18-004-fix-restore-lost-skill-prompt-hotpatches-plan.md` and `docs/solutions/.../*` history docs that quote the old subcommand list. **Historical artifacts** — leave alone (per project convention: solutions docs are point-in-time records, not living docs).
- `skills/bundled/self-dev/system_prompt.md:349` already correctly documents the `issue list --milestone` workaround for fetching milestone titles. **Keep as-is** — it remains the right tool for that specific read operation (more constrained than `gh api`). Adding a sibling `gh api` recipe here is out of scope; the broader Permitted list update at line 265 is sufficient.

## Out of scope

- Narrowing `gh api`'s capability surface (per-method allowlist, per-path-prefix allowlist, `audit_events` row on every `api` call). Decision rationale per ticket and re-confirmed in § Risk surface: YAGNI for the current threat model. If a concrete misuse incident lands, file a follow-up ticket.
- Migrating the existing read-only `gh_read` (mika-arch-scoped, `tools.json`-declared) onto a unified surface with `run_gh`. Different scope, different agent population.
- Updating the self-dev workflow to actually invoke `gh api` to close milestones. **Sibling ticket** — the verify-post-state pattern (`mika#TBD` — to be filed at implementation time if not already open) consumes this allowlist change.

## Risk surface

**Blast radius is bounded by the existing `pr`/`issue` whitelist, not expanded by it.**

The agent can already issue `gh pr edit --title "..."` (arbitrary PR title mutation), `gh issue close <n>`, and `gh issue edit <n> --add-label / --remove-label`. These cover the highest-value prompt-injection mutation targets — anything the GitHub PR-and-issue surface exposes. Adding `gh api PATCH /repos/.../pulls/{n}` covers an *adjacent* mutation class at the *same* authorization level; the new surface is novel-in-grep-pattern, not novel-in-blast-radius.

The structural-binding observability layer (per § Scope, the `gh_api_invocation` audit event) addresses the only genuine observability regression — operators currently grep `tool_calls.input` for destructive patterns on subcommand; with `api` enabled they need a parallel grep target on `gh_api_invocation` rows that surface `method` + `path` directly. The audit event ships atomically in this PR.

**What's NOT in this risk surface:** the GitHub App installation scope (if `MIKA_GITHUB_APP_*` is configured). App scope can differ from PAT scope; the operator owns App permission configuration. This plan does not assume PAT-scope as the security boundary — it assumes the existing `pr`/`issue` whitelist as the blast-radius anchor (a property of the allowlist itself, not of token scope). Auditing the live App config is operator-owned and orthogonal to this PR.

The `engine-guards-vs-prompt-rules` citation in § "mika#805 disposition" justifies observability as the *prerequisite* for structural tightening (mika#1167's opening criteria), not as a *substitute* for it.

## Implementation steps (post-architect-GROOMED)

1. Update `GH_ALLOWED_SUBCOMMANDS` constant (remove `milestone`/`project`, add `api`).
2. Add `extract_api_method()` helper in the same file (one-pass scan, tolerant of `--method X` and `--method=X`, defaults `"GET"`).
3. Add `gh_api_invocation` audit-event `info!` block in `run_gh()` (gated by `gh_args.args[0] == "api"`).
4. Update three test functions: `test_run_gh_allowlist_accepts_valid` (drop `milestone`/`project`, add `api`), new `test_run_gh_allowlist_rejects_removed_subcommands`, new `test_run_gh_allowlist_accepts_api`. Add `test_extract_api_method` covering `--method X`, `--method=X`, and default-on-absence cases.
5. Update six skill prompts (`sed` is acceptable here — the verbatim string is identical across all six).
6. `cargo test -p mika-agent --test eval` smoke + `cargo test -p mika-agent skills::builtin_handlers::tests::test_run_gh_` for the targeted suite.
7. `cargo clippy --workspace --all-targets -- -D warnings`.
8. `cargo fmt`.
9. Commit + PR per `/mika` pipeline standard.

Estimated diff: ~50 lines Rust (2 const edits + audit event + helper + 4 test bodies), ~6 prompt-file line-edits.

## Verification

**Build correctness:** `cargo build -p mika-agent`. Compiles.

**Test correctness:** `cargo test -p mika-agent skills::builtin_handlers::tests::test_run_gh_` → all six existing `test_run_gh_*` plus the two new tests pass.

**Behavioral verification (post-deploy on mika-dev's container):** Send `mika ask --agent mika-dev "run_gh api /repos/senara-solutions/mika --method GET"` → returns the repo JSON. Send `mika ask --agent mika-dev "run_gh milestone list --repo senara-solutions/mika"` → returns the "subcommand 'milestone' is not allowed" structured error. Both via the standard tool path, not via shell.

**Audit-event verification (post-deploy):** After the GET call above, grep the server log for the new event: `grep gh_api_invocation $MIKA_SPIRIT_LOG_FILE | jq 'select(.method == "GET")'` should return one row with `path = "/repos/senara-solutions/mika"`.

**Smoke verification of mika#15 unblock (the original repro):** After deploy, dispatch a milestone-close work item to mika-dev (or run inline via `mika ask`): the agent should call `run_gh ["api", "/repos/senara-solutions/mika/milestones/15", "--method", "PATCH", "-f", "state=closed"]` and the milestone should transition to `closed` state. Verify via `gh issue list --milestone 15 --state closed --repo senara-solutions/mika` (operator-side, outside the agent).

## Rollback

Single commit; `git revert <sha>` and redeploy. No schema migration, no data backfill, no irreversible side effect. The pre-revert state is the current main HEAD — exactly the buggy state we're patching.

## Related

- mika milestone#15 — the concrete repro that surfaced this
- **mika#805** — sibling restricted-`gh api` design, **deferred** by operator on 2026-05-17 in favor of this PR's unrestricted + audit-event shape. See § "mika#805 disposition" above.
- mika#1167 — preserves the per-method-gating design from #805 for re-opening only under the concrete triggers named in § "mika#805 disposition".
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — structural-binding principle ("pick the layer where the unsafe option is never offered"). This plan applies the principle as **observability binding** via the audit event, not as validator gating — see § Risk surface and § #805 disposition for the reasoning.
- `docs/solutions/integration-issues/github-skill-missing-label-documentation.md` — prior precedent on allowlist/docs drift (referenced in ticket body)
- `crates/mika-agent/CLAUDE.md` § "GitHub Read-Only Handler" — sibling `gh_read` surface for mika-arch (orthogonal; not affected)
- `crates/mika-agent/CLAUDE.md` § "Per-turn tool_use dedup guard" (#582) — runtime guards that surround `run_gh` (unaffected)

## Citations checked during grooming

- `crates/mika-agent/src/skills/builtin_handlers.rs:1619-1630` — current allowlist contents (verified, bug present)
- `crates/mika-agent/src/skills/builtin_handlers.rs:1760-1800` — `validate_gh_input` shape (no signature change needed)
- `crates/mika-agent/src/skills/builtin_handlers.rs:2647-2700` — existing test scaffolding (test_run_gh_allowlist_accepts_valid uses literal list)
- `docs/configuration.md:368` — PAT scope (`Pull requests R/W, Issues R/W, Contents R`)
- `skills/bundled/{self-dev,self-dev-iterate,qa-review,qa-review-build-callback,self-dev-webhook-qa,self-dev-webhook-ci}/system_prompt.md` — all six locations of the verbatim Permitted-list string, verified via grep
- `git blame -L 1619,1630 crates/mika-agent/src/skills/builtin_handlers.rs` — origin commit `f3e93e78c` (Vincent Dupont, 2026-03-13, initial run_gh introduction)

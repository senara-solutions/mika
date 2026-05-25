# `_arch_ask` delivers plan content via stdin (mika#1283)

**Ticket:** mika#1283 — p0-critical, milestone#26. Surfaced by mika#1282's groom dispatch which hit ESCALATE on second-pass-after-iterate because the architect could not read the plan file at the worktree path.

## Goal

Fix the substrate-foundation bug where `_arch_ask` passes `"@${plan_path}"` as the message argument to `mika ask`, expecting the CLI to expand `@<path>` to file content. **`mika ask` does NOT support `@<path>` expansion** — the literal string is passed to mika-arch, which can't read worktree paths via its scoped `read_agent_file` tool. The architect has been reviewing whatever issue-body context was in session memory, NOT plan content.

## What changes

### Single function change in `dispatch-lib.sh`

`_arch_ask` switches from `args+=( "@${plan_path}" ); mika "${args[@]}"` to `args+=( - ); mika "${args[@]}" < "$plan_path"`. The `-` marker tells `mika ask` to read the message from stdin per `mika ask --help` (`The message to send (use "-" to read from stdin)`).

Tests updated:
- Removed: `_arch_ask passes @-file body` (asserted the broken behavior).
- Added: `_arch_ask uses stdin marker (mika#1283)` (`|-|` in argv).
- Added: `_arch_ask no longer passes @-path (mika#1283)` (`assert_not_contains` regression guard).
- Added: `_arch_ask delivers plan content via stdin (mika#1283)` — uses a stub that echoes its stdin to verify content flows through.

Net delta: +3 assertions (130 passed vs 127 pre-fix; 6 pre-existing failures unchanged).

## Why this wasn't caught by sub-PRs 5–8 tests

sub-PRs 5–8 covered structural plumbing (state machine, ESCALATE markers, canonical writer, content-only pilot routing). The actual architect-call delivery surface was assumed to work because `/mika-ask-arch` (operator-facing slash command) uses `@<path>` syntax and that works inside Claude Code sessions (where Claude expands `@<path>` itself before invoking `mika ask`). dispatch-lib's bash subprocess does NOT have that expansion — the literal `@<path>` string is sent.

The sub-PR 7a first-test on mika#1267 produced a "GROOMED" verdict that we attributed to canonical-writer behavior. In retrospect, the architect was approving on session-memory + issue-body context, not on actual plan content. mika#1282's groom surfaced the bug because mika#1282 was a fresh ticket with no prior session-memory context — the architect was strict and complained loudly.

## Verification

Direct probe before fix (verified 2026-05-25 ~21:55Z):

```
$ mika ask --agent mika-arch --format json --verbose "@/tmp/nonexistent-path-test-12345.md"
```

mika-arch response (verbatim): *"The `@` prefix doesn't cause the file content to be inlined into my context. I only see the literal path string, and `read_agent_file` rejects anything outside my home directory."*

Session: `aff2d0fe-5733-4d7e-bcba-ae231a1b7a0f`.

## Acceptance criteria

- [ ] `_arch_ask` source contains `args+=( - )` (stdin marker) and `mika "${args[@]}" < "$plan_path"` (file redirect).
- [ ] `_arch_ask` source does NOT contain `"@${plan_path}"` or `"@$plan_path"`.
- [ ] `bash -n` exit 0 on `dispatch-lib.sh` and `test-dispatch-lib.sh`.
- [ ] Test suite: 130 passed, 6 pre-existing failures unchanged (net +3 vs sub-PR 8's 127).
- [ ] Post-merge + deploy: re-grooming mika#1282 produces architect findings that reference actual plan content (file paths, AC text, citations from the plan), not "I cannot read that path."

## Verification plan post-merge

1. Merge PR + run `make deploy` to install new substrate.
2. Re-dispatch mika#1282 grooming (clean up existing worktree first; the architect findings file from the prior groom is preserved as forensic evidence).
3. Watch the architect's first-pass response — it should reference actual content from the plan file (the worktree has `docs/plans/2026-05-25-011-bug-1282-dev-pilot-wrote-but-no-commit-plan.md` from the prior failed groom).
4. Expected outcome: architect produces a real review (READY/ITERATE/ESCALATE based on plan merit), not a tool-scope complaint.

## What does NOT ship in this PR

- **Re-grooming of prior tickets** with suspect verdicts (mika#1267, others). Their canonical-callout markers stand for now; if specific tickets need re-verification, they can be re-dispatched once the substrate is honest.
- **mika ask CLI `@<path>` expansion support.** The bigger fix would make `mika ask` symmetric with Claude Code's `@<path>` syntax. Out of scope; sub-issue-able if desired.
- **mika#1282 implementation.** Once #1282's grooming completes under the fixed substrate, that ships separately.

## Provenance

- Surfaced by mika#1282 grooming dispatch task `55e79ca5-d651-4603-ab28-ee98ad261c7b` on 2026-05-25 ~21:42–21:54Z.
- Architect findings preserved at `.iterate/escalate-second-pass-after-iterate.md` in the `bug/1282/skill-dev-pilot-wrote-but-no-commit-on` worktree (sub-PR 5's ESCALATE flow working as designed).
- Verification session: `aff2d0fe-5733-4d7e-bcba-ae231a1b7a0f` (direct probe confirming `mika ask` doesn't expand `@<path>`).
- Related solution doc: `mika/docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md` (which documented the contract refactor without catching this content-delivery gap).
- Operator-direct authorization for the substrate fix: 2026-05-25 evening; chicken-egg condition where the substrate's `_arch_ask` gap prevents grooming the very fix that closes it.

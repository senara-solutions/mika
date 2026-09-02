---
title: "A tool-boundary guard must parse at least as permissively as the consumer it protects"
date: 2026-08-29
category: architecture-patterns
module: agent-core
problem_type: architecture_pattern
component: tooling
severity: high
applies_when:
  - Writing a guard that decides whether an argument names a protected resource
  - The guard and the code that acts on that argument parse it separately
  - A guard's own test suite is green but the guard has never been traced to its real consumer
  - Reviewing an allowlist, denylist, or validation gate that fronts a shell script
symptoms:
  - A guard silently fails to fire instead of refusing
  - The guarded action happens with no rejection event anywhere in the logs
  - Guard tests pass, including negative ones, while the bypass is live
root_cause: missing_validation
resolution_type: code_fix
related_components:
  - dispatch
  - tooling
tags:
  - guard-design
  - tool-boundary
  - parser-divergence
  - allowlist
  - dispatch
  - shell-interop
  - defense-in-depth
  - anti-vacuity
---

# A tool-boundary guard must parse at least as permissively as the consumer it protects

## Context

mika#2046 added a dispatchable-repository allowlist so the autonomous loop could not create a worktree in `control-monitor` or `claude-pilot`. The load-bearing layer was a rejection inside `validate_dispatch_readiness` (`crates/mika-agent/src/skills/executor.rs`), which reads the dispatch `prompt` argument, extracts the repository reference, and refuses anything outside the list.

The guard's parser required every byte after the `#` to be an ASCII digit. The consumer it protects — `skills/bundled/_shared/dispatch-lib.sh`, the script that actually creates the worktree — reads the same value at `:769` as:

```bash
PROMPT=$(printf '%s\n' "$INPUT" | jq -r '.prompt // empty')
```

Command substitution strips trailing newlines. So a prompt of `"control-monitor#159\n"` reached the shell as `control-monitor#159`, matched the shell's `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` at `:1077`, and routed into worktree mode against `$PLATFORM_DIR/control-monitor` — while the Rust guard returned `None`, never consulted the allowlist, and refused nothing. One invisible character defeated the whole gate.

## Guidance

**Split the guard into two questions and give them opposite strictness.**

| Question | Direction | Why |
|---|---|---|
| *Detection* — "is this a reference I must judge?" | Be **at least as permissive** as the downstream consumer | Anything you fail to detect is never judged, so the gate is skipped rather than passed |
| *Decision* — "is this reference allowed?" | Be **strict** | This is the actual policy |

A guard parser that is *stricter* than the execution parser is a bypass waiting to be found. The failure is silent by construction: the guard does not refuse, it simply sees nothing, so there is no rejection event, no log line, and no test failure.

**Trace the argument to the code that acts on it, and read that code's normalization.** Two normalizations found here are worth checking first whenever a guard fronts a shell script:

- **Command substitution strips trailing newlines.** `$(…)` removes them, so a value your parser rejects for having a `\n` may reach the shell clean.
- **`grep -qE '^…$'` matches per line, not per string.** It succeeds when *any* line matches, so a multi-line argument whose first line is a valid reference still routes as that reference.

The fix in this case was to trim each line and scan lines rather than requiring the whole string to match:

```rust
pub(crate) fn parse_repo_ref_from_dispatch_prompt(prompt: &str) -> Option<&str> {
    prompt.lines().find_map(parse_repo_ref_line)  // each line trimmed inside
}
```

## Why the tests did not catch it

The suite was green — 3993 tests — and it included an anti-vacuity proof: the allowlist predicate was neutralized to return `true` for everything, and the negative tests were observed to fail. That proof is worth running, but note exactly what it establishes.

**An anti-vacuity proof validates the predicate you wrote, not the perimeter you believed you covered.** Neutralizing `is_dispatchable_repo` proves the *decision* is load-bearing. It says nothing about the *detection* step upstream of it, because every test input reaching that predicate had already been detected as a reference. The bypass lived in the inputs that never got that far.

What found it was a review that traced the guard to `dispatch-lib.sh` and ran the value through the real shell. When a guard fronts another component, "the tests pass" is evidence about the guard; only following the value into that component is evidence about the boundary.

## Locking it down

Two test shapes, both cheap:

- **One regression test per normalization you identified** — `"control-monitor#159\n"`, `"  x  "`, `"\r\n"`, and a multi-line prompt. Each one asserts the guard still sees a reference.
- **One test that locks the check ordering.** Call the guard's entry point with a `task_id` that does not exist and assert the error is `repo_not_dispatchable`, not `task_not_found`. A refactor that moved the check below the task fetch would make the gate depend on task existence; without this test, nothing would say so.

## Applicability

This applies wherever a validation gate and the code it protects parse the same value independently — an allowlist in front of a shell script, an API validator in front of a worker that re-parses the payload, a permission check that normalizes a path differently from the filesystem call it guards. It does not apply when the guard and the consumer share one parser, which is the better design when it is reachable: the divergence cannot exist if there is only one reading of the value.

## Related

- `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md` — the complement. That doc decides *which layer* the guard belongs at; this one says the guard at that layer must speak the same language as the layer it protects. A guard can be at the correct layer and still be bypassable.
- `docs/solutions/1053-dispatch-trigger-allowlist-config-constant.md` — why the allowlist itself is a Rust constant rather than derived state.
- mika#2046 — the allowlist and both guard layers.
- mika#2062 — the shell-side defense-in-depth guard, deliberately deferred.

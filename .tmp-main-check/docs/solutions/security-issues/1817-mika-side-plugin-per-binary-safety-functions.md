---
module: permission-policy, claude-pilot, per-spawn-evaluator
tags: [permission-policy, per-spawn, plugin, ssc-oss-boundary, tier1-parity, structural-typing]
problem_type: architecture-pattern
category: security-issues
date: 2026-07-22
ticket: mika#1817
applies_when:
  - "Implementing a downstream policy contract for an OSS library that must keep private allow/deny contents out of the public repo"
  - "Porting compound-string safety logic (e.g. tier1.py) to per-spawn / per-argv shape"
  - "Establishing parity between an old classifier and a new decomposition-based evaluator during an opt-in Phase 1 migration"
resolution_type: pattern
---

# mika-side plugin pattern — per-binary safety functions on the SSC OSS boundary

## TL;DR

`claude-pilot#90` shipped the generic per-spawn permission-policy engine with an empty `DEFAULT_POLICY`. **mika#1817** ships the Mika-specific allow/deny CONTENTS as a separate Python package (`tools/mika_permission_policy/`) that plugs into the engine at runtime via `MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy`. Every safety function preserves **parity** with the equivalent rule in classic `claude-pilot/src/claude_pilot/tier1.py` — parity is the correctness contract for Phase 1 opt-in of `mika#1708`.

The pattern splits the concern along the SSC OSS boundary (ratified 2026-07-01): the engine is public and generic; the CONTENTS are private and downstream. Structural typing on the `PolicyFn` protocol (`Callable[[list[str], str], bool]`) means the plugin can be installed and tested without claude-pilot present, and any downstream consumer implementing the same protocol can use the plugin verbatim.

## Founding context (2026-07-22)

The autonomous loop was stalled ~5 days: 25 ready issues frozen, 0 merges. Root cause per `mika-dev`'s core memory: `claude-pilot-py policy:deny halts on 20+ common bash commands`. The classic classifier in `tier1.py` denies legitimate compositional shell (pipes-to-conditional-`awk`, `cd`-then-`grep`, chain-with-`echo`) because it operates on raw command STRINGS with syntactic pattern-matching. `mika#1686` documented the n=13+ shape class in 24 hours; every new operator idiom generates a fresh n=1.

Prime-ratified fix (`mika#1686` → Option C in `mika#1708`): decompose the command upfront with `bashlex`, track `cwd_stack` for `cd`, evaluate each resulting spawn against a per-binary safety function. The generic engine ships in `claude-pilot` (public repo, Apache 2.0, empty `DEFAULT_POLICY`); the Mika-specific safety functions ship as a downstream plugin.

## Pattern

### Three-layer separation

| Layer | Repo | Ownership | Content |
|---|---|---|---|
| Engine | `senara-solutions/claude-pilot` | Public OSS | `bashlex` decomposition, `cwd_stack`, per-spawn evaluator API, mode selection env vars, audit events. Ships with `DEFAULT_POLICY = {}` and `load_policy_from_module()`. |
| Plugin (this doc) | `senara-solutions/mika/tools/mika_permission_policy/` | Mika-side private | `get_policy()` returns `dict[str, PolicyFn]` — per-binary safety functions mirroring tier1's classic allow/deny logic. |
| Install glue | `senara-solutions/mika/Makefile` | Mika-side | `install-permission-policy-plugin` target: `uv tool install --reinstall --force --editable ../claude-pilot --with-editable ./tools/mika_permission_policy` — injects the plugin into the same `uv tool` env as claude-pilot so the running interpreter can `import mika_permission_policy` at handler creation time. |

### Structural typing over import coupling

The plugin does **not** import `claude_pilot.per_spawn.PolicyFn`:

```python
# mika_permission_policy/__init__.py
from collections.abc import Callable

PolicyFn = Callable[[list[str], str], bool]  # protocol, structural typing

def get_policy() -> dict[str, PolicyFn]:
    return {"grep": is_safe_grep, ...}
```

Consequences:

- The plugin is testable in environments where claude-pilot is absent (CI without cpp, upstream contributor forks).
- No version pin between plugin and engine — any cpp release implementing the same protocol works.
- Any downstream consumer (not just Mika) can use the plugin verbatim by setting `MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy` — the module namespace is not Mika-only despite the historical name.

### Parity as the correctness contract

Every safety function mirrors the equivalent rule in `claude-pilot/src/claude_pilot/tier1.py`:

- What tier1 auto-approves on a compound STRING, the plugin auto-approves on the corresponding decomposed `Spawn` (bashlex-tokenized `argv`).
- What tier1's `TIER3_PATTERNS` denies, the plugin denies at the argv level (`sed -i`, `git push --force`, `git push origin main/master`, `git branch -D`, `cargo publish`, `bash -c`, `sh -c`, `DROP TABLE`, `DELETE FROM`, …).
- What tier1's `_is_safe_find_command` / `_is_safe_sort_command` / `_is_safe_gh_command` guard, the plugin guards with the same closed-world allowlists (`FIND_EXEC_SAFE_COMMANDS`, sort's `-o`/`--output` denial, gh's domain+verb allowlist).

Parity is the **correctness contract for Phase 1 opt-in**: if the plugin denies a shape that classic allows, the migration is unsafe. Test suite `tests/test_binaries.py` locks this contract with parametrized cases drawn from `tier1.py`'s own allowed shapes and TIER3 denials.

### What the plugin does NOT check

The engine's `per_spawn.decompose()` already refuses these at the raw-source level, so per-binary functions never see them:

- Command substitution (`` `...` ``, `$(...)`), heredocs (`<<`), process substitution (`<(...)`, `>(...)`), arithmetic expansion (`$((...))`).
- Control flow (`if`, `for`, `while`, `case`, `select`, `until`, functions).
- Dynamic execution builtins (`eval`, `source`, `.`, `exec`).
- Chain safety across multiple spawns — each spawn is evaluated independently; every one must pass.

Per-binary functions only own the shape that survived to their argv: forbidden flags, unsafe subcommands, sub-feature guards.

### Accepted risks inherited from tier1

- **GNU-grep assumption.** `grep`/`egrep`/`fgrep` treat every invocation as read-only. GNU grep is; `ugrep` (a drop-in `grep` on some Gentoo/BSD/Homebrew hosts) has `--filter`/`--pager`/`--view` that execute commands. tier1 documented this accepted risk on the pilot's standard-Linux target where GNU grep 3.12 is resolved. If the deployment target changes, drop grep from the registry entirely — do NOT try to parse inner sub-flags (inner-arg lexing is a forbidden posture per `claude-pilot/docs/solutions/security-issues/command-string-policy-allow-rules-are-compound-unsafe.md § 4`).
- **`awk s/foo/bar/e` and `sed` `e` command inside script strings.** Detecting these requires parsing the awk/sed script — out of scope. tier1 does not detect them either (raw-string regex has the same blind spot). Same accepted risk, same downstream mitigation (relay-level judgment on unusual inputs).
- **Missing entries auto-deny.** Any binary not in the registry rejects. If Mika's operator idioms include a binary not listed here, the dispatch falls through to classic tier2 / relay during Phase 1 opt-in, or to a hard-deny once Phase 3 retires classic. Monitor `perm_policy_rollback` audit events during Phase 2 and expand the registry as evidence arrives.

## Deploy path (double-gated per plan)

1. **Merge** the plugin PR — Vincent (repo scope, per supervision model).
2. **Reinstall on gentux** — `make install-permission-policy-plugin` (Vincent, his box). This runs `uv tool install --reinstall --force --editable ../claude-pilot --with-editable ./tools/mika_permission_policy`, so the plugin package lands in the same uv tool env as claude-pilot.
3. **Flip env vars** per canary session (or persistently in `~/.mika/.env`) — Vincent:
   ```
   MIKA_PERMISSION_POLICY_MODE=per_spawn
   MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy
   ```

After canary success: monitor `[claude-pilot audit_event] {"kind": "perm_policy_rollback"}` on stderr. When N=50 dispatches with zero rollbacks accumulate, ratify Phase 2 default flip with Vincent. Phase 3 (retiring `tier1.py`'s Bash paths and `permissions.yaml`'s Bash rules) is a separate follow-up after M=7 days on default `per_spawn` with zero rollbacks.

## When to reach for this pattern

- **You have an OSS engine that must accept downstream policy CONTENTS.** The plugin protocol pattern (Python `entry-point`-style module ref, structural typing on the contract, zero import coupling) keeps the boundary clean.
- **You're migrating from an old classifier to a new one with opt-in Phase 1.** The parity contract lets you enable the new evaluator without changing behavior on merge — the default stays classic, operators flip a canary, audit events feed the flip decision.
- **You need to authorize per-invocation safety judgment (argv + cwd) rather than raw-string pattern matching.** The per-binary shape gives each rule a small, testable surface with a clear name (`is_safe_grep`, `is_safe_git`) that maps 1:1 onto the semantic notion of the invocation.

## When NOT to reach for this pattern

- If the classifier only needs a raw-string allow-list of full commands, per-binary decomposition is over-engineered. Reach for it only when compositional shell idioms (`|`, `;`, `&&`, `cd`+X) are the wedge.
- If the downstream consumer is inside the same repo as the engine, the plugin indirection is unnecessary — a plain function call suffices. This pattern earns its keep only across a repository boundary.

## References

- `mika#1817` — this plugin
- `mika#1708` — architect-ratified Option C spec (session `22d21b66-eacd-4120-bb0a-cc11ce5b4f5d`, 2026-07-01 ~11:35Z)
- `mika#1686` — Prime-ratified class-level fix
- `senara-solutions/claude-pilot#90` — the generic engine PR
- `claude-pilot/src/claude_pilot/tier1.py` — the classic classifier this plugin preserves parity with
- `claude-pilot/src/claude_pilot/per_spawn.py` — the engine that loads this plugin
- `claude-pilot/docs/permission-mode.md` — mode-selection guide, wire shape of audit events
- Plan doc `mika/docs/plans/2026-07-01-008-feat-1708-per-spawn-permission-gate-plan.md` on branch `feat/1708/permission-policy-cpp-per-spawn` — the architect-committed sequencing

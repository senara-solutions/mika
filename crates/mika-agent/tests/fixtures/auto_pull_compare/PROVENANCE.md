# `auto_pull` compare fixtures — mika#2123

Real payloads from `GET /repos/senara-solutions/mika/compare/main...<branch>`,
captured **2026-09-01** against `origin/main`. `commits`, `files`, `base_commit`
and `merge_base_commit` were dropped — they are megabytes and the gate reads
none of them. Every field the gate does read is verbatim from the API.

Each fixture's rebase outcome was **measured**, not inferred, by actually
rebasing the branch onto `origin/main` in a throwaway worktree on 2026-09-01:

| fixture | branch | behind | ahead | status | real `git rebase origin/main` |
|---|---|---|---|---|---|
| `1680-diverged-180-behind-2-ahead.json` | `fix/1680/mika-dev-tui-broken-glyph-rendering-in` | 180 | 2 | `diverged` | **CONFLICT** — `crates/mika-agent/src/agent_loop/mod.rs`, `crates/mika-agent/src/evidence/guards.rs` |
| `1959-diverged-75-behind-1-ahead.json` | `feat/1959/mcp-manifest-data-grade-field-l4-forward` | 75 | 1 | `diverged` | OK |
| `2048-diverged-17-behind-1-ahead.json` | `ci/2048-re-enable-release-please` | 17 | 1 | `diverged` | OK |
| `2123-ahead-0-behind-1-ahead.json` | `fix/2123/dispatch-lib-le-rebase-est-tent-au` | 0 | 1 | `ahead` | n/a (nothing to rebase) |

The `#1680` row reproduces the issue report verbatim — same two conflicted
files, eleven days later.

The `#1959` row is the honest one: **the gate refuses a branch that would have
rebased cleanly.** That is the declared cost of a policy threshold that cannot
predict a conflict (KTD2b), not a defect. It is also the only fixture the
distance rule refuses *alone*, which is what makes the non-vacuity proof
possible — see `tests/auto_pull_promotion_gate.rs`.

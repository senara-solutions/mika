"""Mika-side per-binary safety functions for claude-pilot per-spawn evaluator.

Loaded by claude-pilot when ``MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy``.
See ``docs/permission-mode.md`` in claude-pilot for the plugin protocol and
``README.md`` here for authoring conventions.

## Source of authoritative logic

Every safety function in this package **preserves parity** with the equivalent
compound-string logic in ``claude-pilot/src/claude_pilot/tier1.py`` — the pre-existing
classic evaluator. Parity is the correctness contract for Phase 1 opt-in of
``mika#1708``: the same commands that auto-approved under classic must auto-approve
under per_spawn, or the migration is unsafe.

Where classic operates on a compound STRING (splitting + scanning for
metacharacters), per_spawn operates on a decomposed ``Spawn`` (already tokenized
into ``argv`` by bashlex, cwd tracked separately). So each per-binary function
here only checks the shape it OWNS: forbidden flags, unsafe subcommands, etc.
Compound-safety (chain injection, substitution, redirects) is already handled
by ``claude_pilot.per_spawn`` before any of these functions run.

## Design source

- ``mika#1817`` — this package
- ``mika#1708`` architect-ratified spec (session 22d21b66, 2026-07-01 ~11:35Z)
- ``mika#1686`` — Prime-ratified C class-level fix
- ``claude-pilot#90`` — the per-spawn engine (empty ``DEFAULT_POLICY`` awaiting this plugin)
"""

from __future__ import annotations

from collections.abc import Callable

from mika_permission_policy._binaries import (
    is_safe_awk,
    is_safe_basename,
    is_safe_bash,
    is_safe_cargo,
    is_safe_cat,
    is_safe_comm,
    is_safe_cut,
    is_safe_date,
    is_safe_diff,
    is_safe_dirname,
    is_safe_file,
    is_safe_find,
    is_safe_gh,
    is_safe_git,
    is_safe_grep,
    # Extras beyond the AC3 initial 13 — same-family read-only tools that
    # classic tier1 already allows (see SAFE_SHELL_COMMANDS), included so
    # per_spawn parity holds without pulling operators through relay round-trips
    # for shapes classic already auto-approves.
    is_safe_head,
    is_safe_ls,
    is_safe_make,
    # No-op safe builtins (allow-all)
    is_safe_no_op,
    is_safe_pwd,
    is_safe_readlink,
    is_safe_realpath,
    is_safe_sed,
    is_safe_sh,
    is_safe_sort,
    is_safe_sqlite3,
    is_safe_stat,
    is_safe_tail,
    is_safe_test,
    is_safe_tr,
    is_safe_type,
    is_safe_uniq,
    is_safe_wc,
    is_safe_which,
)

# The per_spawn.PolicyFn protocol is (argv: list[str], cwd: str) -> bool.
# We do NOT import claude_pilot.per_spawn.PolicyFn here so this plugin stays
# importable in environments where claude-pilot is absent (tests, CI without
# cpp installed). Structural typing is enough.
PolicyFn = Callable[[list[str], str], bool]


def get_policy() -> dict[str, PolicyFn]:
    """Return the Mika per-binary safety-function registry.

    Called by ``claude_pilot.per_spawn.load_policy_from_module`` at handler
    creation time (once per pilot session). Each entry maps a binary basename
    to its safety function. Binaries not in the registry deny by default
    (see :func:`claude_pilot.per_spawn.evaluate` — missing entry = reject).
    """
    return {
        # ── AC3 initial 13 binaries (mika#1708) ──
        "grep": is_safe_grep,
        "awk": is_safe_awk,
        "sed": is_safe_sed,
        "cat": is_safe_cat,
        "ls": is_safe_ls,
        "find": is_safe_find,
        "git": is_safe_git,
        "gh": is_safe_gh,
        "cargo": is_safe_cargo,
        "make": is_safe_make,
        "sqlite3": is_safe_sqlite3,
        "bash": is_safe_bash,
        "sh": is_safe_sh,
        # ── grep family (same read-only safety as `grep`) ──
        "egrep": is_safe_grep,
        "fgrep": is_safe_grep,
        # ── Read-only shell utilities from classic SAFE_SHELL_COMMANDS ──
        "head": is_safe_head,
        "tail": is_safe_tail,
        "wc": is_safe_wc,
        "stat": is_safe_stat,
        "file": is_safe_file,
        "which": is_safe_which,
        "type": is_safe_type,
        "pwd": is_safe_pwd,
        "date": is_safe_date,
        "sort": is_safe_sort,
        "uniq": is_safe_uniq,
        "tr": is_safe_tr,
        "cut": is_safe_cut,
        "diff": is_safe_diff,
        "comm": is_safe_comm,
        "realpath": is_safe_realpath,
        "readlink": is_safe_readlink,
        "dirname": is_safe_dirname,
        "basename": is_safe_basename,
        "test": is_safe_test,
        "[": is_safe_test,
        # ── No-op safe builtins (per_spawn NO_OP_SAFE_BUILTINS mirror) ──
        # per_spawn still emits a Spawn for these so a downstream policy CAN
        # audit / rate-limit them; here we simply allow.
        "echo": is_safe_no_op,
        "printf": is_safe_no_op,
        "true": is_safe_no_op,
        "false": is_safe_no_op,
        ":": is_safe_no_op,
    }


__all__ = ["PolicyFn", "get_policy"]

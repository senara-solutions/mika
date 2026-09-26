"""Per-binary safety functions — parity with claude_pilot.tier1.

Each function takes ``(argv, cwd)`` where ``argv[0]`` is the binary basename
(as written in the shell command, not resolved through ``$PATH``). Returns
``True`` if the invocation is safe to auto-approve, ``False`` otherwise.

## Parity with tier1.py

Every deny here mirrors a rule that tier1's compound-string logic ALREADY
enforces on the same shape. When tier1 said no to ``sed -i``, we say no to
``sed`` with ``-i`` in argv. When tier1 said no to ``git push --force``, we
say no to ``git push`` with ``--force`` in argv. The regression suite in
``tests/test_parity.py`` locks this contract.

## What we do NOT check here

- Command substitution (``$(...)``, backticks), heredocs, arithmetic — the
  per_spawn engine (``decompose()`` pre-checks) already refuses those at the
  raw-source level. If we see argv here at all, the command already passed
  those gates.
- Chain safety (``foo && rm -rf``, ``foo | evil``) — per_spawn decomposes
  into per-Spawn calls; each Spawn is evaluated independently. Every spawn
  must pass or the whole command denies.
- Path containment for filesystem targets — this is a Write/Edit tool
  concern, not a Bash one, and tier1 doesn't check it either.
"""

from __future__ import annotations

# ── Read-only tools (allow-all) ─────────────────────────────────────────────
#
# These commands are pure read/inspect operations. Their safety follows from
# the binary having no write side effects at all, so any argv is safe. Match
# tier1.SAFE_SHELL_COMMANDS membership + the ``return True`` fall-through in
# ``is_safe_shell_command`` (no special-case guard fires for these).


def is_safe_grep(argv: list[str], cwd: str) -> bool:
    # See tier1.py comment on FIND_EXEC_SAFE_COMMANDS: grep-family safety
    # depends on GNU grep being resolved (not ugrep). We inherit the same
    # accepted-risk boundary — do NOT parse `--filter`/`--pager`/`--view`
    # sub-flags; if ugrep-as-grep enters scope, drop grep entirely.
    return True


def is_safe_cat(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_ls(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_head(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_tail(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_wc(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_stat(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_file(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_which(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_type(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_pwd(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_date(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_uniq(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_tr(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_cut(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_diff(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_comm(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_realpath(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_readlink(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_dirname(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_basename(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_test(argv: list[str], cwd: str) -> bool:
    return True


def is_safe_no_op(argv: list[str], cwd: str) -> bool:
    """echo / printf / true / false / : — allow-all no-op family."""
    return True


# ── awk / sed — general-purpose interpreters ────────────────────────────────
#
# tier1 EXPLICITLY DROPS awk and sed from SAFE_SHELL_COMMANDS (cpp#27) because
# awk `system()` / sed `e` command have arbitrary-code-execution sub-features
# an exhaustive guard can't enumerate. tier1 routes them to policy/relay.
#
# For per_spawn Phase 1, we cannot preserve tier1 parity by dropping these —
# per_spawn expects a PolicyFn per binary and missing = deny. Denying `awk`/
# `sed` outright would REGRESS on the exact operator idioms mika#1708 exists
# to unblock (`grep | awk '$1 > N'`, `sed 's/foo/bar/' file`).
#
# Compromise: deny the two proven-dangerous sub-features (awk system/exec,
# sed -i / e command / e flag), allow the rest. Documented as accepted risk
# in README.md § accepted-risks so any regression review sees it.


def is_safe_awk(argv: list[str], cwd: str) -> bool:
    """Allow awk except for known code-execution sub-features.

    Denied features (rationale in README.md § awk):
    - ``system(...)`` — shells out.
    - ``getline ... | "cmd"`` / ``print ... | "cmd"`` — pipe to shell.
    - ``| "cmd" | getline`` — inverse pipe from shell.

    We check for these as literal substrings in argv values. This is a
    coarse over-block on strings that happen to contain e.g. ``system(``
    as data (rare); the safe direction.
    """
    for arg in argv[1:]:
        if "system(" in arg:
            return False
        # print ... | "cmd" — the `| "` pattern (with optional whitespace)
        # indicates a pipe to shell command. `|"` alone (no whitespace)
        # is the same. Match both.
        if '| "' in arg or '|"' in arg or "| '" in arg or "|'" in arg:
            return False
        if "getline" in arg and "|" in arg:
            # `getline ... | "cmd"` or `"cmd" | getline` — pipe form
            # either direction. Deny to be safe.
            return False
    return True


def is_safe_sed(argv: list[str], cwd: str) -> bool:
    """Allow sed except for in-place edit.

    Denied features:
    - ``--in-place`` / ``--in-place=SUFFIX`` — GNU long form.
    - ``-i`` / ``-iSUFFIX`` — short form.
    - Any short-flag cluster containing ``i`` (e.g. ``-ni``, ``-Ei``).

    tier1's TIER3_PATTERNS catches this via the raw-string regex
    ``\\bsed\\s+(-\\w*i|-i\\w*)\\b`` — we mirror by rejecting any short-flag
    cluster containing ``i``.

    Accepted risk (documented in README § accepted-risks): the ``e`` command
    trailing a substitution (``s/foo/bar/e``) is not detected here — parsing
    sed scripts is out of scope. tier1 does not detect it either (same regex),
    so parity holds. Downstream policy/relay handles rare `e`-flag cases.
    """
    for arg in argv[1:]:
        if arg == "--in-place" or arg.startswith("--in-place="):
            return False
        # Short-flag cluster containing 'i' anywhere (`-i`, `-iSUFFIX`,
        # `-ni`, `-Ei`).
        if arg.startswith("-") and not arg.startswith("--") and len(arg) > 1:
            if "i" in arg[1:]:
                return False
    return True


# ── find — write actions + exec-class guard ─────────────────────────────────
#
# Mirrors tier1._is_safe_find_command. `-delete` and file-write actions
# (`-fprintf`, `-fprint`, `-fprint0`, `-fls`) always deny. `-exec`/`-execdir`/
# `-ok`/`-okdir` allow only when the inner binary is in the closed-world
# read-only allowlist.
#
# We inherit tier1's FIND_EXEC_SAFE_COMMANDS verbatim (grep/egrep/fgrep,
# cat/head/tail/wc, ls/stat/file, basename/dirname/readlink/realpath,
# echo/printf).

FIND_EXEC_SAFE_COMMANDS: frozenset[str] = frozenset({
    "grep", "egrep", "fgrep",
    "cat", "head", "tail", "wc",
    "ls", "stat", "file",
    "basename", "dirname", "readlink", "realpath",
    "echo", "printf",
})

_FIND_DELETE_FLAGS: frozenset[str] = frozenset({"-delete"})
_FIND_WRITE_FLAGS: frozenset[str] = frozenset({
    "-fprintf", "-fprint0", "-fprint", "-fls",
})
_FIND_EXEC_FLAGS: frozenset[str] = frozenset({
    "-exec", "-execdir", "-ok", "-okdir",
})


def is_safe_find(argv: list[str], cwd: str) -> bool:
    i = 1
    while i < len(argv):
        tok = argv[i]
        if tok in _FIND_DELETE_FLAGS or tok in _FIND_WRITE_FLAGS:
            return False
        if tok in _FIND_EXEC_FLAGS:
            # The next token is the inner binary.
            if i + 1 >= len(argv):
                return False  # bare -exec with no inner cmd
            inner = argv[i + 1]
            if inner not in FIND_EXEC_SAFE_COMMANDS:
                return False
            # Skip the inner-cmd token; the rest are its args (we don't
            # parse them, matching tier1 discipline).
            i += 2
            continue
        i += 1
    return True


# ── sort — deny -o / --output write flag ────────────────────────────────────
#
# Mirrors tier1._is_safe_sort_command (cpp#64). `-o FILE` and `--output=FILE`
# are arbitrary-file-write primitives via a sort built-in (bypassing the
# shell redirect Tier-3 pattern). Deny in any of their shapes.


def is_safe_sort(argv: list[str], cwd: str) -> bool:
    for tok in argv[1:]:
        if tok == "--":
            break  # end of options — remaining are file operands
        if tok.startswith("--"):
            # `--output`, `--output=FILE`, and any prefix abbreviation
            # `--o…` down to `--o`. tier1 uses a length-≥3 startswith
            # check on `--output`; we mirror it exactly.
            name = tok.split("=", 1)[0]
            if len(name) >= 3 and "--output".startswith(name):
                return False
            continue  # other long flag
        if tok.startswith("-") and len(tok) > 1:
            # Short-flag cluster. Walk left-to-right: `-o` = output write,
            # value-taking flags (-k/-S/-t/-T) consume the rest of the
            # token as their attached value, no-arg flags skip to next char.
            for ch in tok[1:]:
                if ch == "o":
                    return False
                if ch in ("k", "S", "t", "T"):
                    break  # rest of token is value, not a flag
            continue
    return True


# ── git — subcommand allowlist + tier3 patterns ─────────────────────────────
#
# Mirrors tier1.is_safe_git_command + the relevant TIER3_PATTERNS entries
# (push --force, push origin main/master, reset --hard, branch -D).

SAFE_GIT_SUBCOMMANDS: frozenset[str] = frozenset({
    "status", "log", "diff", "branch", "show", "commit",
    "push", "checkout", "worktree", "rev-parse", "remote",
    "fetch", "pull", "add", "stash", "tag", "merge",
    "rebase", "cherry-pick", "symbolic-ref",
    "ls-files", "describe", "shortlog", "blame",
})


def is_safe_git(argv: list[str], cwd: str) -> bool:
    if len(argv) < 2:
        return False
    subcommand = argv[1]
    if subcommand not in SAFE_GIT_SUBCOMMANDS:
        return False

    # Deny --force / -f (any short-flag cluster containing 'f') anywhere.
    # tier1: `--force\b|-\w*f\b`.
    for tok in argv[2:]:
        if tok == "--force":
            return False
        if tok.startswith("-") and not tok.startswith("--") and "f" in tok[1:]:
            return False

    if subcommand == "push":
        # push origin main/master denied (tier1 TIER3 pattern).
        for tok in argv[2:]:
            if tok in ("main", "master"):
                return False

    if subcommand == "branch":
        # branch -D (or -wD any-cluster containing D) denied.
        for tok in argv[2:]:
            if tok.startswith("-") and not tok.startswith("--") and "D" in tok[1:]:
                return False

    return True


# ── gh — domain+verb allowlist + api mutation guard ─────────────────────────
#
# Mirrors tier1.is_safe_gh_command.

SAFE_GH_SUBCOMMANDS: dict[str, frozenset[str]] = {
    "pr": frozenset({"create", "view", "list", "checkout", "diff", "checks"}),
    "issue": frozenset({"view", "list", "edit", "comment"}),
    "run": frozenset({"view", "list"}),
    "repo": frozenset({"view"}),
    "release": frozenset({"view", "list"}),
    "workflow": frozenset({"view", "list"}),
    "auth": frozenset({"status"}),
}

def is_safe_gh(argv: list[str], cwd: str) -> bool:
    if len(argv) < 2:
        return False

    if argv[1] == "api":
        # gh api is safe when read-only (no method override / no field flags).
        # Deny any mutation flag in its bare, attached, or long form.
        for tok in argv[2:]:
            if tok.startswith("-X") or tok.startswith("--method"):
                return False
            if tok.startswith("-f") or tok.startswith("-F"):
                # -f / -F alone (field-name follows) or -fname=value / -Fname=value
                # (attached). Both are mutation flags.
                return False
            if (
                tok.startswith("--field")
                or tok.startswith("--raw-field")
                or tok.startswith("--input")
            ):
                return False
        return True

    if len(argv) < 3:
        return False
    domain = argv[1]
    verb = argv[2]
    allowed = SAFE_GH_SUBCOMMANDS.get(domain)
    if allowed is None:
        return False
    return verb in allowed


# ── cargo — subcommand allowlist ────────────────────────────────────────────
#
# Mirrors tier1.is_safe_build_command's cargo branch. `cargo publish` is
# TIER3-denied (`\\bcargo\\s+publish\\b`); other write subcommands (install)
# absent from the allowlist deny by allow-list.

SAFE_CARGO_SUBCOMMANDS: frozenset[str] = frozenset({
    "check", "test", "clippy", "fmt", "build",
    "clean", "doc", "bench", "tree", "metadata",
})


def is_safe_cargo(argv: list[str], cwd: str) -> bool:
    if len(argv) < 2:
        return False
    return argv[1] in SAFE_CARGO_SUBCOMMANDS


# ── make — closed-world target allowlist ────────────────────────────────────
#
# Mirrors tier1.is_safe_make_command. The pattern is FULL-anchored: exactly
# `make <target>` with no other tokens. `make verify-bundled-skills X=1` is
# denied by tier1 (X= overrides variables). We mirror strictly.

SAFE_MAKE_TARGETS: frozenset[str] = frozenset({"verify-bundled-skills"})


def is_safe_make(argv: list[str], cwd: str) -> bool:
    if len(argv) != 2:
        return False
    return argv[1] in SAFE_MAKE_TARGETS


# ── sqlite3 — read-only invocations only ────────────────────────────────────
#
# tier1 does NOT have a sqlite3 branch — it's not in SAFE_SHELL_COMMANDS, so
# every classic invocation drops through to policy/relay. AC3 of mika#1708
# lists sqlite3 in the initial set. We add a conservative allow: read-only
# forms only (``sqlite3 <db> "SELECT ..."`` / ``sqlite3 --readonly ...``).
# Any invocation touching an unrecognized SQL verb denies (mika-dev has to
# route through relay for writes). Denies DROP/DELETE/INSERT/UPDATE at the
# argv-substring level (also caught by tier1 TIER3 patterns on the raw
# compound: `\\bDROP\\s+TABLE\\b`, `\\bDELETE\\s+FROM\\b`).

_SQL_MUTATION_PATTERNS: tuple[str, ...] = (
    "DROP TABLE", "DROP DATABASE", "DROP INDEX", "DROP VIEW", "DROP TRIGGER",
    "DELETE FROM", "INSERT INTO", "UPDATE ", "ALTER TABLE",
    "CREATE TABLE", "CREATE INDEX", "CREATE VIEW", "CREATE TRIGGER",
    "TRUNCATE ", "REPLACE INTO",
    "PRAGMA journal_mode = OFF", "VACUUM",
)


def is_safe_sqlite3(argv: list[str], cwd: str) -> bool:
    # Reject any SQL mutation verb, case-insensitively, anywhere in argv.
    for arg in argv[1:]:
        upper = arg.upper()
        for pattern in _SQL_MUTATION_PATTERNS:
            if pattern in upper:
                return False
    return True


# ── bash / sh — deny -c inline execution ────────────────────────────────────
#
# tier1 TIER3 denies `\bbash\s+-c\b` and `\bsh\s+-c\b`. Inline shell arg is
# an arbitrary-code-exec primitive — the whole point of the per_spawn design
# is to REFUSE opaque strings. `bash script.sh` / `sh script.sh` are allowed
# (they invoke a script path; the pilot can Read that path if needed).


def is_safe_bash(argv: list[str], cwd: str) -> bool:
    for tok in argv[1:]:
        if tok == "-c":
            return False
        # -c can also be clustered (`-ic` for interactive+cmd — rare, but tier1
        # would catch via the raw-string regex `-c\b`). Match short-flag
        # clusters containing 'c'.
        if tok.startswith("-") and not tok.startswith("--") and "c" in tok[1:]:
            return False
    return True


def is_safe_sh(argv: list[str], cwd: str) -> bool:
    return is_safe_bash(argv, cwd)

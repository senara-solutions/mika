---
title: "Code review fixes: shell injection, memory safety, and performance hardening"
date: 2026-02-26
category: security-issues
tags:
  - shell-injection
  - memory-safety
  - input-validation
  - tmux-skill
  - image-handling
  - progressive-reveal
  - code-review
  - integer-overflow
severity: P1-critical
components:
  - templates/skills/tmux/handlers/
  - crates/mika-cli/src/tui/input.rs
  - crates/mika-cli/src/tui/app.rs
  - crates/mika-cli/src/tui/ui.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-common/src/claude.rs
resolution_type: security-hardening
commit: 7aba1ec
findings_count: 22
findings_p1: 5
findings_p2: 9
findings_p3: 8
---

# Code Review Fixes: Shell Injection, Memory Safety, and Performance Hardening

## Problem

Commit `7aba1ec` introduced three major features: `/think` command (extended thinking), paste/image support, and tmux skill. A multi-agent code review identified 22 findings across these features, including 5 P1-critical security vulnerabilities that blocked production readiness.

### Symptoms (if unfixed)

- **Shell injection**: Crafted tmux session names could inject arbitrary flags or commands via word splitting
- **Arbitrary key injection**: Unvalidated special keys could send `C-z`, `C-\`, or multi-key sequences to any tmux session
- **ReDoS**: User-supplied regex patterns could cause catastrophic backtracking in `grep -E`
- **Memory exhaustion**: 500MB files read into memory before size check; unbounded image dimensions cause OOM
- **Integer overflow**: `budget_tokens + 4096` silently wraps in release builds when near `u32::MAX`

## Root Cause

The features were implemented correctly for the happy path but lacked defensive input validation at system boundaries:

1. **Shell scripts**: Variables passed to tmux commands without quoting; no input validation on user-supplied parameters
2. **Image handling**: File I/O and allocation ordered for convenience rather than safety (read first, check after)
3. **Arithmetic**: Standard `+` operator used where `saturating_add` was needed for untrusted inputs

Common pattern: **Input validation gates must precede resource-intensive operations.**

## Solution

### P1 Critical: Shell Script Hardening

**1. Shell injection in `create_session.sh`**

Replaced unquoted `$ARGS` string-building pattern with direct, properly-quoted arguments. Added session name allowlist.

```bash
# BEFORE (vulnerable):
ARGS="-d -s $NAME"
tmux new-session $ARGS  # word splitting enables injection

# AFTER (safe):
if ! echo "$NAME" | grep -qE '^[a-zA-Z0-9._-]+$'; then
    echo "Error: invalid session name" >&2; exit 1
fi
tmux new-session -d -s "$NAME" -c "$WORKDIR"
```

**2. Special key allowlist in `send_command.sh`**

Added explicit allowlist of 16 safe keys. Any key not in the list is rejected.

```bash
ALLOWED_KEYS="Enter|Escape|Tab|Space|C-c|C-d|C-z|C-l|Up|Down|Left|Right|BSpace|Home|End|PageUp|PageDown"
if echo "$SPECIAL_KEY" | grep -qE "^($ALLOWED_KEYS)$"; then
    tmux send-keys -t "$SESSION" "$SPECIAL_KEY"
else
    echo "Error: special key '$SPECIAL_KEY' is not allowed" >&2; exit 1
fi
```

**3. ReDoS prevention in `wait_for_text.sh`**

Three-layer defense: pattern length cap, grep timeout, and wall-time tracking.

```bash
# Cap pattern length
PATTERN_LEN=$(printf '%s' "$PATTERN" | wc -c)
if [ "$PATTERN_LEN" -gt 200 ]; then exit 1; fi

# Wrap regex grep with timeout
MATCH=$(echo "$OUTPUT" | timeout 2 grep -E "$PATTERN" | tail -1)

# Track wall time (not iteration count)
START_TIME=$(date +%s)
while true; do
    ELAPSED=$(($(date +%s) - START_TIME))
    if [ "$ELAPSED" -ge "$TIMEOUT" ]; then break; fi
    # ...
    sleep 1
done
```

**4. Integer validation in `read_output.sh`**

Renamed `LINES` (reserved shell variable) to `LINE_COUNT`. Validated as integer and clamped.

```bash
case "$LINE_COUNT" in
    ''|*[!0-9]*) LINE_COUNT=50 ;;
esac
if [ "$LINE_COUNT" -gt 10000 ]; then LINE_COUNT=10000; fi
```

**5. Integer overflow in `agent.rs`**

```rust
// BEFORE: panics in debug, wraps in release
claude.max_tokens.max(budget_tokens + 4096)

// AFTER: clamps to u32::MAX
claude.max_tokens.max(budget_tokens.saturating_add(4096))
```

### P2 Important: Memory Safety and Performance

**6. File size check before read**

```rust
// Check metadata BEFORE reading file contents
let metadata = std::fs::metadata(&path).ok()?;
if metadata.len() > MAX_IMAGE_BYTES as u64 { return None; }
let data = std::fs::read(&path).ok()?;

// Validate magic bytes match claimed extension
if &data[..magic_bytes.len()] != magic_bytes { return None; }
```

**7-8. Clipboard image dimension safety**

```rust
let width = u32::try_from(img.width).ok()?;   // safe conversion
let height = u32::try_from(img.height).ok()?;
if width > 8192 || height > 8192 { return None; }

let pixel_count = img.width.checked_mul(img.height)?;  // overflow check
if pixel_count > 20_000_000 { return None; }

let mut png_data = Vec::with_capacity(img.bytes.len() / 2);  // pre-allocate
// ... encode ...
drop(img);  // free raw data before base64 encoding
```

**9. Attachment limits**

```rust
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;
```

**11. Cached thinking block rendering**

Thinking blocks now pre-render at creation time (like assistant messages) instead of re-parsing every frame.

**12. Adaptive progressive reveal**

```rust
let increment = if len < 1024 { 8 } else if len < 4096 { 32 } else { 64 };
```

A 4KB response now reveals in ~4s instead of ~15s.

### P3 Nice-to-Have: Cleanup

- Removed `output_tokens` dead code from `AgentResponse`
- Removed `ImageSource::from_png_bytes` (zero callers) and `base64` dependency from `mika-common`
- Deduplicated `format_size` into `ImageAttachment::format_size()`
- `model_context_limit` field replaced with `pub const MODEL_CONTEXT_LIMIT: u32 = 200_000`
- Extracted duplicate prompt rendering in `draw_input`
- `/think` now supports optional budget: `/think [budget] <prompt>` (clamped 1024-100000)

## Prevention

### Shell Script Security Checklist

- All user-controlled variables must be quoted in command arguments
- Input validation (allowlist regex) before any use in shell commands
- Numeric parameters validated with `case` pattern and clamped to sane ranges
- Regex patterns bounded by length; `grep -E` wrapped in `timeout`
- Consider requiring `jq` instead of brittle `grep/cut` fallback parsing
- Run `shellcheck` on all handler scripts in CI

### Memory Safety Checklist

- Check file size via `metadata.len()` before `std::fs::read()`
- Use `u32::try_from()` instead of `as u32` for untrusted dimensions
- Use `checked_mul()` / `checked_add()` for user-influenced arithmetic
- Use `saturating_add()` where overflow should clamp rather than fail
- Validate magic bytes match claimed file extension
- `path.canonicalize()` before file operations to resolve symlinks
- Pre-allocate buffers; `drop()` intermediates before allocating dependents

### Performance Checklist

- Cache rendered output in `ChatMessage.rendered` at creation time
- Scale progressive reveal increment by response length
- Use wall time (`date +%s`) not iteration count for timeouts in shell scripts
- Integer sleep intervals in POSIX shell (avoid `sleep 0.5` portability issues)

### Automated Checks

```bash
# Add to CI pipeline
shellcheck templates/skills/*/handlers/*.sh
cargo clippy -- -W clippy::cast_possible_truncation -W clippy::cast_lossless
```

## Related Documentation

- [21 CLI findings parallel resolution](../code-review-workflow/mika-cli-21-findings-parallel-resolution.md) — Previous code review with shell hardening patterns
- [Filesystem skill registry implementation](../architecture-decisions/filesystem-skill-registry-implementation.md) — Security findings on exec/http handlers (#211-213)
- [TUI log corruption and empty agent replies](../runtime-errors/tui-log-corruption-and-empty-agent-replies.md) — TUI rendering and response handling patterns
- [Slash commands architecture review](../../reviews/2026-02-25-slash-commands-architecture-review.md) — Architecture of the command system including `/think`

## Verification

All 398 tests pass. No new clippy warnings introduced. Build clean across all 4 crates.

```
cargo build   # clean
cargo clippy  # no new warnings
cargo test    # 398 tests: 260 + 43 + 79 + 16
```

---
title: "refactor(gateway): extract shared transient error message string"
type: refactor
status: completed
date: 2026-03-31
---

# refactor(gateway): extract shared transient error message string

## Overview

Extract the duplicated transient error message string `"I'm having trouble right now. Please try again in a moment."` into a `const` in `crates/mika-gateway/src/routes.rs`, then reference it from both `forward_error_message()` and `reply_transient_error()`.

## Problem Statement

PR #325 introduced `forward_error_message(is_connect: bool)` which returns the transient error string for the `!is_connect` branch (line 899). The same string already existed in `reply_transient_error()` (line 908). Two sources of truth for a single user-facing message — if one is updated, the other could be missed.

## Proposed Solution

Add a module-level constant and reference it from both functions:

```rust
// crates/mika-gateway/src/routes.rs
const TRANSIENT_ERROR_MSG: &str =
    "I'm having trouble right now. Please try again in a moment.";
```

Update `forward_error_message()` (line 899) and `reply_transient_error()` (line 908) to use `TRANSIENT_ERROR_MSG`.

Optionally, also extract the connect/offline message into a constant for consistency:

```rust
const OFFLINE_ERROR_MSG: &str =
    "Your Mika assistant is currently offline. \
     Please contact your administrator or check your subscription status \
     at console.getmika.ai.";
```

## Acceptance Criteria

- [x] `TRANSIENT_ERROR_MSG` const defined in `crates/mika-gateway/src/routes.rs`
- [x] `forward_error_message()` returns `TRANSIENT_ERROR_MSG` for the non-connect case
- [x] `reply_transient_error()` uses `TRANSIENT_ERROR_MSG`
- [x] `OFFLINE_ERROR_MSG` const for the connect case in `forward_error_message()`
- [x] Existing tests pass (`cargo test -p mika-gateway`)
- [x] `cargo clippy -p mika-gateway` clean

## Context

- **File:** `crates/mika-gateway/src/routes.rs`
- **Lines:** ~893-911 (`forward_error_message` and `reply_transient_error`)
- **Callers of `reply_transient_error`:** lines 301, 357, 640 (customer lookup failure, container error response, pairing query failure)
- **Callers of `forward_error_message`:** line 363 (container unreachable)
- Related issue: #332
- Discovered during review of #325

## Sources

- GitHub issue: #332

---
title: Extract duplicated user-facing strings into constants
category: code-review-patterns
date: 2026-03-31
tags: [refactor, gateway, duplication, constants]
related_issues: ["#332", "#325"]
---

# Extract duplicated user-facing strings into constants

## Problem

User-facing error message strings were duplicated across multiple functions in `crates/mika-gateway/src/routes.rs`. The transient error message `"I'm having trouble right now. Please try again in a moment."` appeared in both `forward_error_message()` and `reply_transient_error()`. If one was updated, the other could be missed.

## Root Cause

The `reply_transient_error()` helper existed first with an inline string. When `forward_error_message()` was added in PR #325 to classify errors (connect vs transient), the same string was duplicated rather than extracted.

## Solution

Extract shared strings into module-level `const` values:

```rust
const TRANSIENT_ERROR_MSG: &str = "I'm having trouble right now. Please try again in a moment.";
const OFFLINE_ERROR_MSG: &str = "Your Mika assistant is currently offline. ...";
```

Reference the constants from both functions. This is a zero-behavior-change refactor — the compiled output is identical.

## Prevention

When adding user-facing messages, check if the same message already exists elsewhere in the file. Grep for distinctive substrings (e.g., `"having trouble"`) before introducing new inline strings.

## Pattern

This applies broadly: any time two functions return or use the same user-facing string, extract it into a `const` at the module level. Place constants immediately above the functions that use them for locality.

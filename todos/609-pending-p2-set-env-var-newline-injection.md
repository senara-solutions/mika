---
status: complete
priority: p2
issue_id: 609
tags: [code-review, security, input-validation]
dependencies: []
---

# set_env_var allows newline injection in .env values

## Problem Statement

`dotenv::set_env_var()` writes values as bare `{key}={value}` without quoting or newline validation. If a value contains `\n`, it injects additional lines into the `.env` file. A malicious or accidental value like `sk-key\nMIKA_INTERNAL_TOKEN=attacker_token` would override the internal auth token on the next `load_dotenv` call.

While current callers use `dialoguer::Password` (which strips newlines from terminal input), the function is `pub` in `mika_common` and values containing `#` would also be silently truncated by dotenvy's parser on read (interpreted as inline comments).

## Findings

- **Source:** security-sentinel, architecture-strategist, pattern-recognition agents
- **Location:** `crates/mika-common/src/dotenv.rs:48,57` — bare `format!("{key}={value}")` writes
- **Evidence:** No sanitization or quoting of key or value; `#` in values causes silent truncation on read
- **Impact:** Medium — token override possible; base64-encoded OTLP auth headers commonly contain `=` and could contain `#`
- **Known Pattern:** See `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — parser consistency is a documented concern

## Proposed Solutions

### Option A: Validate — reject newlines (Recommended, minimal)
```rust
if key.contains('\n') || key.contains('\r') || value.contains('\n') || value.contains('\r') {
    anyhow::bail!("key and value must not contain newline characters");
}
if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || key.is_empty() {
    anyhow::bail!("invalid .env key name: must be non-empty ASCII alphanumeric/underscore");
}
```
- Effort: Small
- Risk: Low — all current callers pass hardcoded valid keys and terminal-stripped values
- Pro: Simple, catches the actual attack vector
- Con: Doesn't fix `#` truncation on read

### Option B: Always double-quote values
Write as `{key}="{escaped_value}"` where `"`, `\`, and newlines are escaped:
```rust
let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
lines.push(format!("{key}=\"{escaped}\""));
```
- Effort: Small
- Risk: Low — dotenvy handles double-quoted values correctly
- Pro: Full round-trip fidelity for all characters including `#`, `=`, spaces
- Con: Slightly more complex; need to verify dotenvy unquotes on read

### Option C: Both — validate keys, quote values
Combine key validation from A with value quoting from B.
- Effort: Small
- Risk: Low
- Pro: Defense in depth

## Acceptance Criteria

- [ ] Values containing newlines are rejected or safely escaped
- [ ] Keys are validated as `[A-Za-z_][A-Za-z0-9_]*`
- [ ] Values containing `#` survive a write-then-read round trip
- [ ] Add round-trip test: `set_env_var` then `get_env_var` with edge-case values (`#`, `=`, quotes, spaces)

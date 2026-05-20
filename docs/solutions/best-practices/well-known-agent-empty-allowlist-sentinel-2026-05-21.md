---
module: well-known-agents
date: 2026-05-21
problem_type: best_practice
component: tooling
severity: high
tags:
  - well-known-agents
  - identity-allowlist
  - skill-registry
  - empty-allowlist
  - sentinel-pattern
applies_when:
  - Adding a new well-known agent that should have zero skills
  - Configuring identity.toml with an empty skill allowlist
  - Debugging why a well-known agent unexpectedly loads all skills
---

# Well-Known Agent Empty Allowlist Requires Sentinel Value

## Context

When adding a new well-known agent (`mika-test`, mika#963) that should have zero
skills, the natural approach is to set `allowlist = []` in the identity TOML.
This appears semantically correct — an empty allowlist should deny everything.

However, `SkillRegistry::apply_identity_allowlist()` in `crates/mika-agent/src/skills/mod.rs`
treats an empty slice as a no-op:

```rust
pub fn apply_identity_allowlist(&mut self, allowlist: &[String]) {
    if allowlist.is_empty() {
        return;  // <-- empty list = no filtering = ALL skills active
    }
    // ... retain() logic that evicts non-listed skills
}
```

This means `allowlist = []` results in the agent loading ALL bundled skills —
the exact opposite of the intended behavior.

## Guidance

Use a sentinel value that matches no real skill name. The codebase already has
a precedent in `prompt.rs` for the fail-closed identity path:

```rust
// Fail-closed sentinel (prompt.rs:349)
allowlist: Some(vec!["__fail_closed_no_skills__".to_string()]),
```

For `mika-test`, the identity uses:

```toml
[skills]
allowlist = ["__mika_test_no_skills__"]
```

The sentinel passes the `!allowlist.is_empty()` check, so `retain()` runs.
Since no skill matches `__mika_test_no_skills__`, all skills are evicted.

## Why This Matters

The empty-allowlist no-op is intentional design — it distinguishes "no allowlist
configured" (user-defined agents) from "allowlist configured but empty" at the
`Option<Vec<String>>` level. The `if let Some(ref allowlist)` guard at callsites
handles `None` (skip filtering), and the `is_empty()` guard inside the method
handles `Some(vec![])` as equivalent to "no constraint."

For well-known agents that genuinely want zero skills, this design means the
sentinel pattern is the only correct approach. The alternative — removing the
`is_empty()` guard — would change behavior for any future caller that passes
`&[]` expecting a no-op.

## When to Apply

- Adding a new well-known agent with `disabled_skills: &[]` and
  `identity_source: Some(IdentitySource::Static(...))` that should have no skills
- Any identity TOML where `[skills].allowlist` must deny all skills
- Testing the fail-closed identity path in `prompt.rs`

## Examples

**Wrong — agent gets all skills:**

```toml
[skills]
allowlist = []
```

**Correct — agent gets zero skills:**

```toml
[skills]
allowlist = ["__mika_test_no_skills__"]
```

**Verify in tests using typed deserialization:**

```rust
#[test]
fn test_mika_test_identity_valid_toml() {
    let identity: crate::prompt::Identity =
        toml::from_str(MIKA_TEST_IDENTITY).expect("should be valid TOML");
    let allowlist = identity.skills.allowlist.as_ref().unwrap();
    assert_eq!(allowlist.len(), 1);
    assert_eq!(allowlist[0], "__mika_test_no_skills__");
}
```

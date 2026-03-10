---
status: pending
priority: p2
issue_id: 600
tags: [code-review, quality, correctness]
dependencies: []
---

# get_env_var custom parser diverges from dotenvy

## Problem Statement

`dotenv::get_env_var()` uses a hand-rolled `.env` parser that handles a subset of the format dotenvy supports. This creates parsing divergence: `load_dotenv` (via dotenvy) handles escaped quotes, `export` prefixes, inline comments, and multiline values, but `get_env_var` does not.

Example: `MIKA_ANTHROPIC_API_KEY="sk-ant-key" # my key` would be loaded correctly by `load_dotenv` but `get_env_var` would include `# my key` in the value.

## Findings

- Security sentinel, simplicity reviewer, and pattern recognition all flagged this independently
- The only caller is `setup.rs` checking if a key exists before prompting
- dotenvy provides `from_path_iter()` which can read without loading into process env

## Proposed Solutions

### Option A: Replace with dotenvy::from_path_iter (Recommended)
```rust
pub fn get_env_var(home_dir: &Path, key: &str) -> Option<String> {
    let env_path = home_dir.join(".env");
    dotenvy::from_path_iter(&env_path).ok()?.find_map(|r| {
        let (k, v) = r.ok()?;
        (k == key).then_some(v)
    })
}
```
- Pros: 5 lines, guaranteed parsing parity, handles all edge cases
- Cons: None significant
- Effort: Small
- Risk: Low

## Technical Details

- File: `crates/mika-common/src/dotenv.rs` lines 23-45
- ~20 lines of custom parser replaced with ~5 lines

## Acceptance Criteria

- [ ] `get_env_var` uses dotenvy's parser
- [ ] Tests still pass (dotenvy handles all quote formats in tests)
- [ ] Parsing parity with `load_dotenv` confirmed

---
title: "Tilde (~) home directory expansion missing in file tools"
date: 2026-03-13
category: logic-errors
module: mika-agent/tools
tags: [file-handling, path-resolution, validate_and_resolve_path, security, ux]
severity: medium
---

# Tilde (~) home directory expansion missing in file tools

## Symptom

The agent (LLM) and users naturally use `~/path/to/file` when referencing files, but all file
tools rejected these paths. Built-in Rust tools returned "Absolute paths are not allowed" (since
`~/foo` starts with a non-relative component), and the `read_file` shell skill handler returned
"file not found" (since shell variable expansion does not perform tilde expansion).

## Root Cause

Two independent code paths lacked tilde-to-home expansion:

1. **Rust `validate_and_resolve_path()`** — The shared path validator in `tools/mod.rs` (used by
   `read_agent_file`, `write_agent_file`, `list_agent_files`, `read_workspace`, `write_workspace`)
   performed empty/length/absolute/traversal/symlink/containment checks but never normalized `~`.

2. **Shell `read.sh` handler** — The file-reader skill handler extracted the path from JSON and
   passed it directly to `[ -f "$PATH_VALUE" ]` without expansion. Shell does not expand `~` in
   variable values — only in literal tokens.

## Solution

### 1. Rust: `validate_and_resolve_path` (tools/mod.rs)

Added tilde expansion **before** all security checks. In the sandboxed Rust tools, `~` maps to
`base_dir` (the tool's sandbox root), not `$HOME`:

```rust
// Expand ~ to base_dir (the tool's sandboxed "home")
// Reject ~username syntax (e.g ~root/file) — only bare ~ and ~/ are valid
let path = if path == "~" {
    ""
} else if let Some(rest) = path.strip_prefix("~/") {
    rest
} else if path.starts_with('~') {
    return Err(ToolOutput::error(
        "Only '~/' (your home directory) is supported. '~username' paths are not allowed.",
    ));
} else {
    path
};
```

After expansion, the existing validation pipeline runs on the stripped relative path:
empty check → length check → absolute rejection → `..` component inspection → symlink check →
canonicalize containment.

### 2. Shell: `read.sh` handler

Added POSIX `case` statement for `$HOME` expansion:

```sh
# Expand ~ to $HOME
case "$PATH_VALUE" in
    "~") PATH_VALUE="$HOME" ;;
    "~/"*) PATH_VALUE="$HOME/${PATH_VALUE#\~/}" ;;
esac
```

### 3. `list_agent_files` special case

`list_agent_files` allows empty path to mean "list home root." Bare `~` must map to this
behavior, but `validate_and_resolve_path` maps `~` to empty string (error). So
`list_agent_files` pre-screens before calling the shared validator:

```rust
// Treat bare ~ as home root (same as empty path)
let path = if path == "~" || path == "~/" { "" } else { path };
```

### 4. Discoverability

- Tool `input_schema` descriptions updated to show `~/` example in path field.
- System prompt updated: `"~ is accepted as a prefix, e.g. '~/notes.md'"`.

## Key Design Decisions

### `~` means `base_dir`, not `$HOME`, in Rust tools

Built-in Rust tools are sandboxed to a `base_dir` (`ctx.home_dir` for agent tools,
`workspace_dir` for team workspace tools). Expanding `~` to the OS `$HOME` would be a sandbox
escape. Instead, `~` means "the root of whatever sandbox I am in."

The shell `read_file` handler is intentionally different — it is an unsandboxed skill that reads
arbitrary files the OS user can access, so `~` correctly maps to `$HOME`.

### `~username` explicitly rejected

POSIX shells expand `~root` to `/root`, `~nobody` to `/var/empty`, etc. If we silently passed
`~root/file.txt` through as a literal path, it would resolve to `base_dir/~root/file.txt` —
potentially creating a `~root/` directory. Explicit rejection prevents confusion.

### Two-layer tilde handling in `list_agent_files`

This duplication is intentional. `validate_and_resolve_path` maps bare `~` to empty string →
error ("path is required"). For file-targeting tools, this is correct. But `list_agent_files`
treats empty path as "list the root," so it must intercept `~` before the shared validator.
Unifying would add complexity to a security-critical function for one consumer.

## Security Analysis

| Attack vector | After expansion | Blocked by |
|---|---|---|
| `~/../../etc/passwd` | `../../etc/passwd` | `Component::ParentDir` check |
| `~root/file` | N/A | Explicit `~username` rejection |
| `~/symlink_outside` | `symlink_outside` | Symlink check + canonicalize containment |
| `~` (bare) | `""` | Empty-path error (file tools) or home-root listing (`list_agent_files`) |

Expansion runs before validation — all downstream security checks operate on the post-expansion
relative path.

## Tests Added

| Test | File | Validates |
|---|---|---|
| `test_tilde_expansion_strips_prefix` | `tools/mod.rs` | `~/notes/todo.md` → `{base}/notes/todo.md` |
| `test_bare_tilde_returns_empty_error` | `tools/mod.rs` | `~` → empty → error for file tools |
| `test_tilde_username_rejected` | `tools/mod.rs` | `~root/file.txt` → explicit rejection |
| `test_tilde_with_traversal_blocked` | `tools/mod.rs` | `~/../../../etc/passwd` → traversal error |
| `test_read_tilde_path` | `tools/read_agent_file.rs` | Integration: `~/notes.md` reads successfully |

## Checklist for New File Tools

When adding a file tool that uses `validate_and_resolve_path`:

- [ ] Tilde expansion is automatic — no extra code needed
- [ ] If the tool accepts empty path as valid (like directory listing), add a `~`/`~/` pre-screen
- [ ] All messages use `full_path.display()` (resolved absolute path)
- [ ] Update the tool's `input_schema` description to mention `~/` support

When adding a new shell handler that accepts file paths:

- [ ] Add the `case` tilde expansion pattern after extracting the path from JSON
- [ ] Note: shell handlers are unsandboxed — `~` maps to `$HOME`, not a sandbox

## Known Gaps

- `shell-exec` handler's `working_dir` and `tmux/create_session.sh`'s `working_dir` do not have
  tilde expansion. Low priority since the LLM rarely sends `~` paths for working directories.

## Related

- [tool-path-reporting-misbehavior.md](tool-path-reporting-misbehavior.md) — absolute path
  reporting rule (tools must show resolved paths, not user input)
- [write-file-tool-overwrite-confirmation-flow.md](../integration-issues/write-file-tool-overwrite-confirmation-flow.md) —
  `validate_and_resolve_path()` shared helper design and security checklist
- [builtin-skill-tool-name-shadowing.md](builtin-skill-tool-name-shadowing.md) —
  `create_parents: bool` parameter added to `validate_and_resolve_path`
- [self-knowledge-missing-home-directory-files.md](../integration-issues/self-knowledge-missing-home-directory-files.md) —
  agent home directory introspection patterns (list_agent_files, read_agent_file)
- GitHub issue: #145

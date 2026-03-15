---
title: "feat: Add --format text|json flag to mika ask"
type: feat
status: completed
date: 2026-03-15
origin: docs/brainstorms/2026-03-15-ask-json-format-brainstorm.md
---

# feat: Add --format text|json flag to mika ask

## Overview

Add a `--format text|json` flag to the `mika ask` CLI command (defaulting to `text`). When `json`, output `{"role": "assistant", "content": "..."}` to stdout — the industry-standard message shape compatible with OpenAI, Ollama, LangChain, and Vercel AI SDK.

## Problem Statement / Motivation

`mika ask` currently prints plain text to stdout with no structured output option. Developers building automation, pipelines, or tooling on top of Mika cannot reliably parse the response. A `--format json` flag enables composability with the broader AI tool ecosystem.

(see brainstorm: docs/brainstorms/2026-03-15-ask-json-format-brainstorm.md)

## Proposed Solution

### 1. Add `OutputFormat` enum to CLI (`crates/mika-cli/src/cli.rs`)

```rust
#[derive(Clone, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}
```

Add to `AskArgs`:
```rust
#[arg(long, value_enum, default_value = "text")]
pub format: OutputFormat,
```

Follow the `SetupMode` `ValueEnum` pattern at `cli.rs:142-150`.

### 2. Add JSON response struct (`crates/mika-cli/src/commands/ask.rs`)

```rust
#[derive(serde::Serialize)]
struct AskJsonResponse {
    role: &'static str,
    content: Option<String>,
}
```

Keep this local to the ask command — no need for a shared type. The `role` field is always `"assistant"`.

### 3. Update output handling in `ask::run`

Pass `format` into the `run` function. Replace the current output block:

```rust
// Current (lines 180-183):
match output.text {
    Some(text) => println!("{text}"),
    None => eprintln!("{}", mika_agent::agent::EMPTY_RESPONSE_FALLBACK),
}

// New:
match format {
    OutputFormat::Text => match output.text {
        Some(text) => println!("{text}"),
        None => eprintln!("{}", mika_agent::agent::EMPTY_RESPONSE_FALLBACK),
    },
    OutputFormat::Json => {
        let response = AskJsonResponse {
            role: "assistant",
            content: output.text,
        };
        println!("{}", serde_json::to_string(&response)?);
    }
}
```

Note: Use `to_string` (compact), not `to_string_pretty` — compact JSON is more pipeline-friendly.

### 4. Update dispatch in `main.rs`

Pass `args.format` through the `run` call at `main.rs:160-176`.

### 5. Handle `send_message` in JSON mode

**Decision:** Redirect `send_message` output to stderr when `--format json` is active. This prevents tool-emitted messages from corrupting the JSON stream on stdout.

Check `crates/mika-cli/src/init.rs` for `make_message_sender`. If the CLI message sender prints to stdout, create a variant or pass a flag so it uses `eprintln!` instead when JSON format is active. If `send_message` is not used in ask mode (ask is non-interactive single-turn), this may be a no-op — verify during implementation.

### 6. Error handling — no change

Errors remain as plain text on stderr with `exit(1)`. JSON consumers check exit code before parsing stdout. This keeps error handling simple and avoids a separate error schema.

### 7. `--task-id` interaction — no change

The callback path (`--task-id`) returns with no output regardless of `--format`. This is correct — callback mode doesn't run the agent, so there's no response to format. No `conflicts_with` needed.

## Technical Considerations

- **`serde_json` already in deps** — no new dependencies needed
- **No changes to `AgentOutput`** — no `Serialize` derive needed since we use a local `AskJsonResponse` struct
- **No changes to `mika-common` or `mika-agent`** — change is fully localized to `mika-cli`
- **Logging goes to stderr** — `LogOutput::PrettyAndFile` writes tracing to stderr, so stdout stays clean for JSON

## Acceptance Criteria

- [x] `mika ask "question"` continues to output plain text (default behavior unchanged)
- [x] `mika ask --format json "question"` outputs `{"role":"assistant","content":"..."}` to stdout
- [x] `mika ask --format text "question"` is identical to no flag
- [x] When agent produces no text in JSON mode, output is `{"role":"assistant","content":null}`
- [x] JSON output is compact (single line, no pretty-printing)
- [x] Exit code 0 on success, 1 on error (unchanged)
- [x] Errors remain on stderr as plain text (unchanged)
- [x] Stdin input (`mika ask --format json -`) works correctly
- [x] `--format json` with `--task-id` outputs nothing (callback path unchanged)
- [x] Unit test for JSON serialization of `AskJsonResponse`
- [x] No stdout pollution from `send_message` or logging in JSON mode

## Files to Modify

1. `crates/mika-cli/src/cli.rs` — Add `OutputFormat` enum and `format` field to `AskArgs`
2. `crates/mika-cli/src/commands/ask.rs` — Accept format param, add `AskJsonResponse` struct, branch output
3. `crates/mika-cli/src/main.rs` — Pass `format` through to `ask::run`

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-15-ask-json-format-brainstorm.md](docs/brainstorms/2026-03-15-ask-json-format-brainstorm.md) — key decisions: minimal `{role, content}` shape, `--format` enum over `--json` boolean, CLI only
- Existing `ValueEnum` pattern: `crates/mika-cli/src/cli.rs:142-150` (`SetupMode`)
- Existing `--json` precedent: `crates/mika-cli/src/commands/doctor.rs:102-104`
- Current ask handler: `crates/mika-cli/src/commands/ask.rs:180-183`
- CLI flag scoping pattern: `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`
- GitHub issue: #158

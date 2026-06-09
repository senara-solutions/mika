---
title: "A2A client: three-tier text render + Task state exit-code contract"
category: architecture-patterns
date: 2026-06-09
tags:
  - a2a
  - protocol
  - client
  - rendering
  - state-machine
  - cli
  - exit-codes
severity: minor
component:
  - mika-cli
  - mika-a2a
  - mika-agent
related_issues: []
problem_type: knowledge
---

# A2A client: three-tier text render + Task state exit-code contract

When building a new A2A client (sync `message/send`), there are two non-obvious contracts the v0.3 spec implies but doesn't put in one place: **where the agent's text output actually lives in the returned Task**, and **which TaskState values represent a useful response vs. an error condition**. This doc captures both so the next client added in this repo doesn't have to rediscover them.

## Context

The mika repo now has two A2A client surfaces:

- `crates/mika-agent/src/tools/a2a_call.rs` — in-process tool that lets a Mika agent call out to a remote A2A agent during its tool loop. Built first (mika#214, March 2026).
- `crates/mika-cli/src/remote_ask.rs` — `mika ask --remote <URL>` CLI dispatcher, added as the R1 slice of the ascension architecture brainstorm (June 2026).

Both call `A2aClient::send_message`, get back a `Task`, and need to render the agent's text reply to a string. The CLI variant additionally has to decide whether the returned Task represents a successful answer (exit 0) or a failure / "still in progress" (exit non-zero). Code-review surfaced that the initial CLI implementation diverged from `a2a_call` on both fronts; this doc fixes the pattern so future clients align.

## Guidance

### 1. Three-tier text extraction

The A2A spec carries an agent's text output in **three possible locations** depending on the conversation shape and the responding agent's implementation. A client that reads only one location will silently render empty for spec-conformant peers.

Walk these tiers in order; fall back to the next when the prior tier produces no text:

```
Tier 1: task.artifacts[*].parts[*]      → completed-task output channel (A2A spec primary)
Tier 2: task.history[*].parts[*]        → agent-role messages only; skip user-role
Tier 3: task.status.message.parts[*]    → fallback when artifacts + history are empty
```

Within each tier:

- `Part::Text { text }` → emit `text` verbatim.
- `Part::File { file }` → emit a placeholder string like `[file: <name>]` (or render inline if your client supports file display).
- `Part::Data { .. }` → emit `[data]` placeholder.

Join collected text fragments with `\n\n` so the remote agent's paragraph boundaries survive — concatenating with no separator collapses multi-paragraph replies.

**Reference implementation:** `crates/mika-agent/src/tools/a2a_call.rs::run` and `crates/mika-cli/src/remote_ask.rs::render_task_parts`. The two implementations should stay in lock-step; the natural future home is a shared helper on `mika_a2a` (something like `mika_a2a::render::collect_text_parts(&Task) -> Vec<String>`). Add the helper before introducing a third client.

### 2. TaskState exit-code contract for sync `message/send`

`TaskState` (defined in `crates/mika-a2a/src/types.rs`) has nine variants. Per A2A v0.3 §6, a synchronous `message/send` MUST return a Task in a terminal-or-pending-input state. In practice clients see:

| TaskState        | Meaning                                          | Sync-call behavior             |
|------------------|--------------------------------------------------|--------------------------------|
| `Completed`      | Successful completion with output                 | Render, exit 0                 |
| `InputRequired`  | Agent is asking a clarifying question             | Render, exit 0 (text is useful)|
| `AuthRequired`   | Agent needs auth (e.g., OAuth handoff)            | Render, exit 0 (text explains) |
| `Failed`         | Terminal failure                                  | Surface as error, exit non-zero|
| `Canceled`       | Operator/user canceled                            | Surface as error, exit non-zero|
| `Rejected`       | Agent refused (policy/scope)                      | Surface as error, exit non-zero|
| `Submitted`      | Async-accepted, not yet started                   | Surface as error: sync expected terminal |
| `Working`        | Async in-progress                                 | Surface as error: sync expected terminal |
| `Unknown`        | Unknown state (malformed remote)                  | Surface as error                |

The trap the CLI client hit: if you treat `Ok(task)` as success without inspecting `task.status.state`, a `Failed` task with no message text renders as an empty success — the shell pipeline sees exit 0 + empty stdout, indistinguishable from a benign empty answer. The fix is a `match task.status.state` after the `client.send_message().await?` return, with the three buckets above.

`InputRequired` and `AuthRequired` are NOT errors at the CLI layer — they carry meaningful text the user needs to see to continue the conversation. Conflating them with `Failed` blocks legitimate multi-turn flows.

## Why This Matters

- **Silent empty output is the worst failure mode.** A user runs `mika ask --remote ... "what's on for today?"` and gets a blank line. They have no way to distinguish "remote agent succeeded with empty content" from "remote agent crashed and we ignored it." Tier 1 extraction (artifacts) and state inspection together close this gap.
- **Exit-code contract matters for scripting.** The mission-window use is Vincent piping `mika ask --remote ...` into shell scripts. If the remote task fails, the script must see exit non-zero — otherwise the failure compounds downstream.
- **`InputRequired` is a feature, not an error.** The A2A spec supports multi-turn clarifying dialogs. A client that errors on `InputRequired` cannot participate in those flows.

## When to Apply

When you add a new A2A client surface to this repo (CLI, dashboard, agent tool, programmatic API). The two existing surfaces — `mika-agent::tools::a2a_call` and `mika-cli::remote_ask` — should keep behaving identically. Drift between them is a sign one missed an A2A spec corner.

When in doubt, **read both reference implementations side by side** and align the new client with the more permissive one (today, that's `mika-cli::remote_ask` for state handling and `mika-agent::a2a_call` for the most-tested render pattern).

## Examples

### Render — minimum viable

```rust
fn render_task_parts(task: &Task) -> String {
    let mut out: Vec<String> = Vec::new();

    // Tier 1: artifacts
    if let Some(artifacts) = &task.artifacts {
        for artifact in artifacts {
            for part in &artifact.parts {
                push_part(part, &mut out);
            }
        }
    }

    // Tier 2: agent-role history
    if let Some(history) = &task.history {
        for msg in history {
            if msg.role == Role::Agent {
                for part in &msg.parts {
                    push_part(part, &mut out);
                }
            }
        }
    }

    // Tier 3: status.message fallback
    if out.is_empty()
        && let Some(msg) = task.status.message.as_ref()
    {
        for part in &msg.parts {
            push_part(part, &mut out);
        }
    }

    out.join("\n\n")
}

fn push_part(part: &Part, out: &mut Vec<String>) {
    match part {
        Part::Text { text, .. } => out.push(text.clone()),
        Part::File { file, .. } => out.push(format!(
            "[file: {}]",
            file.name.as_deref().unwrap_or("unnamed")
        )),
        Part::Data { .. } => out.push("[data]".to_string()),
    }
}
```

### State check — CLI-layer exit-code contract

```rust
match task.status.state {
    TaskState::Completed | TaskState::InputRequired | TaskState::AuthRequired => {
        // render and succeed
    }
    TaskState::Failed | TaskState::Canceled | TaskState::Rejected => {
        anyhow::bail!("remote task {} ended in state '{}'", task.id, task.status.state);
    }
    TaskState::Submitted | TaskState::Working | TaskState::Unknown => {
        anyhow::bail!(
            "remote task {} is still in state '{}' — sync dispatch expected a terminal state",
            task.id,
            task.status.state
        );
    }
}
```

### Empty-token guard for `MIKA_INTERNAL_TOKEN`

`std::env::var("MIKA_INTERNAL_TOKEN").ok()` returns `Some("")` when the env var is set to an empty string (common from a misconfigured shell or `.env`). Forwarding an empty bearer token produces `Authorization: Bearer ` which the gateway 401s with no diagnostic hint. Filter empty:

```rust
let auth_token = std::env::var("MIKA_INTERNAL_TOKEN")
    .ok()
    .filter(|t| !t.is_empty());
```

This is a small but recurring source of "why is my deploy 401-ing" — set-but-empty env vars look identical to set ones until you trace the request.

## Related

- `docs/solutions/integration-issues/a2a-protocol-implementation.md` — broader protocol implementation (server + client roles, state-machine transitions).
- `crates/mika-a2a/CLAUDE.md` — A2A protocol crate overview.
- `crates/mika-agent/src/tools/a2a_call.rs` — first client, reference for render pattern.
- `crates/mika-cli/src/remote_ask.rs` — second client, reference for state-check pattern.
- `docs/plans/2026-06-09-003-feat-ascension-architecture-first-slice-cli-plan.md` — the slice that surfaced the divergence.
- `docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md` — origin (R1 daily-use unblock for local↔cloud Mika portability).

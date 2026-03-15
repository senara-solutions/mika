# Brainstorm: `--format` Flag for `mika ask`

**Date:** 2026-03-15
**Status:** Draft

## What We're Building

Add a `--format text|json` flag to the `mika ask` CLI command (defaulting to `text`). When `json` is selected, output the agent's response as a structured JSON object following the industry-standard message shape:

```json
{"role": "assistant", "content": "The agent's response text here"}
```

This enables developers to pipe `mika ask` into scripts, automation workflows, and other tools that consume structured LLM output.

## Why This Approach

The `{"role": "assistant", "content": "..."}` shape is the **de facto standard** across the AI ecosystem:

- **OpenAI** `choices[].message` — exact match
- **Ollama** `/api/chat` response — exact match
- **LangChain, LiteLLM, Vercel AI SDK** — all use this shape as the message interchange format
- **Anthropic** uses the same field names (`role`, `content`) though `content` is an array of blocks at the API level

By using a flat string for `content` (not Anthropic's block array), the output is directly composable into any tool expecting the OpenAI message format — which is the most widely adopted.

No `choices[]` wrapper is needed since Mika always returns a single response.

## Key Decisions

1. **Flag style:** `--format text|json` (enum) rather than `--json` (boolean). More extensible, even though `mika doctor` uses `--json`. The `--format` pattern is standard in modern CLIs.

2. **Minimal JSON shape:** `{"role": "assistant", "content": "..."}` only. No metadata (model, usage, session_id, stop_reason). Keeps the output maximally composable and simple.

3. **Role is always `"assistant"`:** Validated by industry research — every major LLM API uses `role: "assistant"` for model responses. Including it (even though constant) enables direct array composition: `messages.push(mika_output)`.

4. **CLI only:** No server-side changes. The mika-server `/message` endpoint remains async (202). A synchronous server endpoint may come later but is out of scope.

5. **Edge case — no text response:** Output `{"role": "assistant", "content": null}` when the agent produces no text (tool-only runs). In practice, the agent almost always produces a text summary, so this is a rare fallback.

6. **Default is `text`:** Existing behavior is preserved. No breaking change.

## Open Questions

None — all questions resolved during brainstorming.

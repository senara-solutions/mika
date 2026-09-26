# Brainstorm: Migrating from Anthropic OAuth to Alternative LLM Providers

**Date:** 2026-03-18
**Status:** Decided

## Context

Anthropic blocked OAuth tokens (`sk-ant-oat*`) for third-party applications on January 9, 2026 — they now only work with Claude Code and Claude.ai. This is a server-side policy enforcement with no workaround. The user needs a cost-effective alternative that supports Mika's full agent features (memory, tasks, skills, team delegation) with reliable tool calling.

## What We're Building

No code changes initially. This is a configuration migration guide and provider evaluation. First-class provider prefixes (e.g., `minimax/`, `qwen/`, `kimi/`) will be considered after hands-on testing validates tool calling quality.

## Provider Comparison

### MiniMax M2.5

- **Pricing:** $0.25/M input, $1.20/M output (~$3-7/month for daily use)
- **Tool calling:** BFCL score 76.8 — #1 in multi-turn function calling, outperforms Claude 4.6 and Gemini 3 Pro
- **Context window:** 197K tokens
- **Vision:** Yes
- **OpenAI-compatible:** Yes
- **Standout:** Best multi-turn tool calling reliability. ~20% fewer rounds than predecessors for same results.

**Mika config:**
```bash
MIKA_LLM_MODEL=openai-compatible/MiniMax-M2.5
MIKA_LLM_BASE_URL=https://api.minimax.chat/v1
MIKA_LLM_API_KEY=<minimax-key>
```

### Qwen 3.5 Medium (122B-A10B)

- **Pricing:** $0.26/M input, $1.56/M output (~$3-8/month for daily use)
- **Tool calling:** BFCL-V4 score 72.2 — outperforms GPT-5 mini by 30%
- **Context window:** 128K+ tokens
- **Vision:** Yes
- **OpenAI-compatible:** Yes
- **Standout:** Best open-source agentic benchmarks. Alibaba-backed with strong ecosystem (Qwen-Agent framework).

**Mika config:**
```bash
MIKA_LLM_MODEL=openai-compatible/qwen3.5-plus
MIKA_LLM_BASE_URL=https://dashscope-intl.aliyuncs.com/compatible-mode/v1
MIKA_LLM_API_KEY=<dashscope-key>
```

### Kimi K2.5

- **Pricing:** $0.60/M input, $2.50/M output (~$5-12/month for daily use)
- **Tool calling:** Strong (SWE-bench Verified 76.8%, competitive with Claude Opus 4.5)
- **Context window:** 128K+ tokens
- **Vision:** Yes (native multimodal — trained on 15T mixed visual+text tokens)
- **OpenAI-compatible:** Yes
- **Standout:** Best coding/SWE benchmarks. Native visual agentic intelligence. Agent swarm paradigm.

**Mika config:**
```bash
MIKA_LLM_MODEL=openai-compatible/kimi-k2.5
MIKA_LLM_BASE_URL=https://api.moonshot.ai/v1
MIKA_LLM_API_KEY=<moonshot-key>
```

### Reference: OpenAI GPT-4o-mini

- **Pricing:** $0.15/M input, $0.60/M output (~$2-5/month)
- **Tool calling:** Good but not top-tier on recent BFCL
- **Context window:** 128K tokens
- **Vision:** No (use gpt-4o for vision)
- **Standout:** Cheapest option, already first-class in Mika (`openai/gpt-4o-mini`)

## Recommendation Matrix

| Priority | Best Pick | Why |
|----------|-----------|-----|
| **Tool calling reliability** | MiniMax M2.5 | #1 BFCL multi-turn score (76.8), critical for Mika's 10-tool agent loop |
| **Cheapest** | OpenAI GPT-4o-mini | $0.15/$0.60 per 1M tokens, already first-class support |
| **Best all-rounder** | Qwen 3.5 Medium | Strong agentic benchmarks + vision + reasonable price |
| **Best for coding tasks** | Kimi K2.5 | SWE-bench leader, strong visual coding |

**Primary recommendation: MiniMax M2.5** — Mika's value comes from its tool-heavy agent loop (memory, tasks, skills, delegation). The #1 multi-turn tool calling score directly maps to what matters most. Price is competitive.

**Fallback: Qwen 3.5 Medium** — If MiniMax has availability issues or regional restrictions, Qwen is the next best for agentic tool use with Alibaba's infrastructure backing.

## Known Limitations (All Non-Claude Providers)

- **No extended thinking** — Claude-only feature, silently skipped
- **No prompt caching** — Full token cost every turn (mitigated by cheaper token prices)
- **System prompt adherence** — Mika's system prompt is large and complex; may need testing
- **Team orchestration risk** — Multi-agent delegation requires precise structured output; test thoroughly before relying on it

## Testing Plan

1. Sign up for MiniMax API at https://platform.minimax.io/
2. Configure Mika with the `openai-compatible/` prefix (see config above)
3. Test these scenarios in order of criticality:
   - Basic chat conversation
   - Memory operations: `store_fact`, `update_core_memory`, `search_memory`
   - Reminder creation and delivery
   - Skill execution (web search, file operations)
   - Multi-step tool chains (3+ sequential tool calls)
   - Team delegation (if applicable)
4. If MiniMax passes, repeat with Qwen as backup option
5. After testing, decide whether to add first-class provider prefixes (`minimax/`, `qwen/`, `kimi/`)

## Open Questions

None — all questions resolved during brainstorming.

## Sources

- [Qwen API Platform](https://qwen.ai/apiplatform)
- [Qwen API Pricing Guide 2026](https://deepinfra.com/blog/qwen-api-pricing-2026-guide)
- [MiniMax M2.5 Official Announcement](https://www.minimax.io/news/minimax-m25)
- [MiniMax OpenAI-Compatible API Docs](https://platform.minimax.io/docs/api-reference/text-openai-api)
- [Kimi K2.5 Tech Blog](https://www.kimi.com/blog/kimi-k2-5)
- [Kimi API Pricing](https://costgoat.com/pricing/kimi-api)
- [MiniMax M2.5 BFCL Benchmarks](https://vertu.com/ai-tools/minimax-m2-5-officially-released-comprehensive-benchmarks-comparison/)
- [Qwen 3.5 Agentic Benchmarks](https://www.buildmvpfast.com/blog/alibaba-qwen-3-5-agentic-ai-benchmark-2026)

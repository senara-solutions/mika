---
module: well_known_agents
tags: [provisioning, templates, reconciliation, soul, identity]
problem_type: drift
category: best-practices
---

# Well-Known Agent Template Reconciliation

## Problem

`MIKA_DEV_SOUL` / `MIKA_QA_SOUL` consts in `well_known_agents.rs` were skeleton placeholders from the 2026-03 bootstrap. On-disk versions at `~/.mika/agents/{mika-dev,mika-qa}/soul.md` iterated significantly (10x / 4x the size). The consts only matter on fresh-host bootstrap (the idempotency skip in `provision_well_known_agents()` preserves existing agents), but a fresh provision would seed outdated prompts.

## Direction

**On-disk → code.** The on-disk soul.md files are the source of truth for prompt content. When drift accumulates, snapshot the canonical on-disk content back into the Rust consts. Never push template content to on-disk (that's the reverse direction, explicitly rejected).

## Rules

- **Raw-string uniformity:** Use `r##"..."##` for all soul consts uniformly. Safer against future edits adding backticks or `"#` sequences; two extra `#` characters cost zero runtime. If content contains `##"`, escalate to `r###`.
- **User-specific section exclusion:** Template-written `identity.toml` must NOT include `[reflection]` (timezone, locale data) or other operator-specific sections. These are runtime customizations — operators add them via direct edit; provisioning should not seed them.
- **Semantic emojis in templates:** Keep emojis like `🛠`/`🔍` in the const — the template is what ships on fresh hosts and descriptive emojis aid agent identification.
- **Variant regeneration is separate:** Per-provider/model prompt variants are a runtime operation requiring a live model provider. Never mix variant regen with template reconciliation in the same PR.

## Trigger

When template drift exceeds ~25% by line count (rough guideline), run the reconciliation again. Not enforced by CI; operator discretion.

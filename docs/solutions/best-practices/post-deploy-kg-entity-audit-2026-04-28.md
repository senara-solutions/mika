---
module: mika-agent/kg/domain_builder
tags: [kg, domain-builder, post-deploy, skill-rename, orphan-detection, deploy-verification]
problem_type: post-deploy-verification-gap
category: best-practices
date: 2026-04-28
---

# Audit `kg_entities` for orphans after skill renames or removals

## Context

The KG domain builder (`mika/crates/mika-agent/src/kg/domain_builder.rs`) advertises a "sole writer + idempotent" contract for `skill:*`, `tool:*`, `agent:*`, `problem_type:*` entities. The "idempotent" claim implies "rebuild produces the same graph state from current registry" — which would require deleting orphan entries when their source skill is removed.

It doesn't. Verified on 2026-04-28 post-deploy of mika#853 (rename `claude-pilot` → `dev-pilot`):

```
$ sqlite3 ~/.mika/data/mika.db \
    "SELECT entity_key FROM kg_entities WHERE entity_key LIKE 'skill:%';"
skill:claude-pilot   ← orphan (skill removed by #853)
skill:dev-pilot      ← new (correct)
skill:dev-groom      ← new from mika#857 (correct)
...
```

Domain builder upserts new/updated entries but does **not garbage-collect** entries from removed skills. Bug filed as mika#859.

## Guidance

After any deploy that **renames or removes** a bundled skill, audit `kg_entities` for orphans as part of the post-deploy verification ritual:

```bash
# Expected: only entity_keys that match current skills/bundled/<name>/ directories.
sqlite3 ~/.mika/data/mika.db \
  "SELECT entity_key FROM kg_entities WHERE entity_key LIKE 'skill:%' ORDER BY entity_key;"

# Cross-check against the live skill list:
ls mika/skills/bundled/ | sort
```

Mismatches are orphans. Either:

1. **mika#859 lands** — domain_builder gains a delete pass; orphans auto-clear on next boot.
2. **Manual cleanup until then** — direct DELETE on the orphan rows after confirming the source skill is genuinely gone:
   ```bash
   sqlite3 ~/.mika/data/mika.db \
     "DELETE FROM kg_entities WHERE entity_key = 'skill:<removed-name>';"
   ```
   Same for `kg_relationships` rows referencing the orphan entity_key.

Add this audit to the standard post-deploy checklist alongside existing Signal A (`pending_docs == 0` by second restart), Signal B (`kg_budget_exhausted` count), Signal C (resolver backlog drain), and Signal D (cost prediction) from `mika/CLAUDE.md` § Post-restart safety check #757.

## Why This Matters

- **Tool-selection bias.** KG-driven tool selection retrieves entities by `entity_key` match. Orphan `skill:*` rows can bias `mika-dev`'s tool selection toward retired skills (e.g., recommending `run_claude_pilot` with the old skill name when the runtime registry no longer has it).
- **Self-knowledge eval drift.** `tests/eval/kg_self_knowledge/*.rs` queries kg_entities. Orphans inflate match counts and skew assertion outcomes silently.
- **Refactor blast radius.** Every future skill rename or removal hits the same trap until mika#859 lands. Each unaudited deploy adds an orphan and the cumulative drift compounds.

## When to Apply

Run the audit after:

- Any PR that renames a bundled skill (e.g., #853's `claude-pilot` → `dev-pilot`).
- Any PR that removes a bundled skill from `mika/skills/bundled/`.
- Any change to `crates/mika-agent/src/db/kg_schema.rs` skill seed entries.

Not needed for: adding new skills (additions are GC'd correctly — only deletions/renames break the invariant), changes to skill internals (system_prompt.md, handlers/run.sh) that don't touch the directory name.

## Examples

**mika#853 deploy on 2026-04-28T12:34Z** — first restart post-deploy. KG entity check showed `skill:claude-pilot` still present despite the directory rename. Filed mika#859 with the audit query as the reproduction.

**Pre-#859 mitigation** during the 2026-04-28 deploy:
- Confirmed orphan presence (audit query above).
- Noted as "cosmetic + potential bias risk" — not blocking dispatch.
- Sprint of mika#844 + mika#845 proceeded without manual cleanup; orphan tracked for follow-up.

**Post-#859 future state:** the audit becomes a regression check. Run it once per skill-rename PR; if it returns orphans, the GC fix has regressed.

## References

- mika#859 — domain_builder leaves orphan kg_entities when source skill is removed (the bug ticket)
- mika#853 — rename claude-pilot skill to dev-pilot (the deploy that surfaced the orphan)
- mika#857 — add dev-groom bundled skill (same boot cycle; correctly added the new entries)
- `mika/crates/mika-agent/src/kg/domain_builder.rs` § Sole-Writer Contract
- `mika/CLAUDE.md` § Post-restart safety check #757 — Signals A/B/C/D for KG resolution drain

---
status: complete
priority: p1
issue_id: "411"
tags: [code-review, agent-native, reflection]
dependencies: []
---

# Evidence Field Not Declared in Tool Input Schema

## Problem Statement

The `evidence` field is validated at runtime by `update_core_memory`, `store_fact`, and `update_fact` during reflection mode, but it is **not declared** in any tool's `input_schema` JSON definition. Claude's tool use relies on the schema to know what parameters exist. The prompt instructs "The evidence field MUST cite a specific conversation timestamp and quote" but the tool definition sent to the Claude API does not list `evidence` as a property.

This means Claude may not reliably include an `evidence` field it was never told about in the tool schema. The plan document (`docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md` line 109) explicitly says "Add optional evidence field to each tool's JSON schema" but this was not implemented.

## Findings

- **Agent-native reviewer**: "The evidence field is enforced at runtime but invisible in tool schemas... Claude could pass the evidence as part of the reasoning field instead, or omit it entirely (causing a rejection)"
- **Architecture reviewer**: "The evidence field is checked at the tool level but NOT declared in the tool's input_schema — Claude won't know about it unless told in the prompt"
- All 3 tools have identical runtime check at: `update_core_memory.rs:77-84`, `store_fact.rs:68-75`, `update_fact.rs:56-63`

## Proposed Solutions

### Option A: Add evidence to input_schema (Recommended)
Add `"evidence"` property to each tool's `input_schema.properties`:
```json
"evidence": {
    "type": "string",
    "description": "Required in reflection mode: cite specific conversation timestamp and quote as justification for this change"
}
```
- **Pros**: Claude knows the field exists, reliable tool calls, matches plan intent
- **Cons**: Field appears in schema for non-reflection calls too (but is optional)
- **Effort**: Small
- **Risk**: Low

### Option B: Dynamic schema based on is_reflection
Modify tool `definition()` to accept context and conditionally include the evidence field.
- **Pros**: Clean schema per mode
- **Cons**: Requires trait change, more complex
- **Effort**: Medium
- **Risk**: Medium

## Recommended Action

Option A — add `evidence` to schema properties in all 3 tools.

## Technical Details

- **Affected files**: `crates/mika-agent/src/tools/update_core_memory.rs`, `store_fact.rs`, `update_fact.rs`
- **Components**: Tool definitions (input_schema JSON)

## Acceptance Criteria

- [ ] `evidence` field appears in `input_schema.properties` for `update_core_memory`, `store_fact`, `update_fact`
- [ ] Field description mentions it is required in reflection mode
- [ ] Existing tests still pass (evidence is optional in non-reflection mode)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-03 | Identified during code review | Plan doc called for this but was not implemented |

## Resources

- PR #59: periodic memory reflection
- Plan doc: `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md`

---
name: mika-turn-review
description: Inspect an agent's last turn — message, tool calls, and audit events
argument-hint: "<agent_name>"
---

Inspect the last turn of an agent: what did it say, what tools did it call, what changed?

**Argument is required:** an agent name (e.g., `mika-dev`, `mika-test`).

## Step 1: Find the last assistant message

```sql
SELECT m.id, m.session_id, m.trace_id, substr(m.created_at,1,19) as time, substr(m.content,1,500) as content
FROM messages m
WHERE m.agent_id = '$ARGUMENTS' AND m.role = 'assistant'
ORDER BY m.created_at DESC LIMIT 1
```

Run against `~/.mika/data/mika.db`. Store the `session_id` and `created_at` timestamp.

## Step 2: Get the full turn context

Using the session_id and timestamp from Step 1, get all messages in a ±2 minute window around the response:

```sql
SELECT substr(m.created_at,1,19) as time, m.role, substr(m.content,1,300) as content
FROM messages m
WHERE m.session_id = '<session_id>'
  AND m.created_at BETWEEN datetime('<timestamp>', '-2 minutes') AND datetime('<timestamp>', '+1 minute')
ORDER BY m.created_at
```

This shows: user prompt → assistant response → any tool_result messages.

## Step 3: Check audit events (tool calls)

Query audit_events in the same time window to see what tools the agent actually called:

```sql
SELECT substr(ae.created_at,1,19) as time, ae.tool_name, ae.target_key, substr(ae.after_value,1,150) as result
FROM audit_events ae
WHERE ae.agent_id = '$ARGUMENTS'
  AND ae.created_at BETWEEN datetime('<timestamp>', '-2 minutes') AND datetime('<timestamp>', '+1 minute')
ORDER BY ae.created_at
```

If no audit events found, report: "No tool calls in this turn."

## Step 4: Present and flag issues

Show:
1. The user prompt that triggered the turn
2. The agent's response (full text)
3. Tool calls made (or "None")
4. Audit events (memory updates, file writes, etc.)

**Red flags to call out:**
- Agent said "done" or "pushed" or "committed" but zero tool calls → likely fabricated
- Agent made tool calls but response doesn't mention them → context gap
- Tool calls failed (check tool_result messages for errors)

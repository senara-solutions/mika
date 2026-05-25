---
name: mika-sprint
description: Dispatch a sprint to mika-dev with proper formatting
argument-hint: "<ticket list or 'pick N'>"
---

Build a sprint dispatch prompt for mika-dev and send it via `mika ask`.

## Input parsing

`$ARGUMENTS` is either:
- A ticket list: `mika#526 mika-skills#141 mika-cloud#23` (space-separated `repo#number`)
- A pick request: `pick 3` or `pick 3 from mika` — evaluate the backlog and propose tickets for approval before dispatching

## Pick mode

If arguments start with `pick`:

1. Parse the count (default 3) and optional repo filter
2. Fetch open issues from the target repos:
   ```bash
   gh issue list --repo senara-solutions/<repo> --state open --json number,title,labels --limit 20
   ```
3. Present a prioritized sprint proposal (like `/mika` backlog evaluation)
4. Wait for user approval or adjustments
5. Once approved, continue to the dispatch step below with the selected tickets
6. Generate a `sprint_id` of the form `sprint-YYYY-MM-DDTHH:MM:SSZ` using the current UTC time (same format as Dispatch mode step 1). Use the same prompt template defined in Dispatch mode.

## Dispatch mode

1. Generate a `sprint_id` of the form `sprint-YYYY-MM-DDTHH:MM:SSZ` using the current UTC time. Example: `sprint-2026-05-02T14:30:00Z`. Referenced as `<SPRINT_ID>` below.

For each `repo#number` in the ticket list:

2. Fetch the issue title: `gh issue view <number> --repo senara-solutions/<repo> --json title --jq .title`
3. Build the sprint prompt in mika-dev's preferred format:

```
implement sprint (sprint_id=<SPRINT_ID>):

1. <repo>#<number> — <issue title>
2. <repo>#<number> — <issue title>
3. <repo>#<number> — <issue title>

Order: <first> first, then <rest> in listed order.

IMPORTANT — sprint metadata stamping:
For EACH ticket above, when you create a task, you MUST stamp it with
sprint_id metadata using a two-step pattern:
  1. create_task(label=..., reference_url=..., source="self_dev")
  2. update_task_status(task_id=<new_id>, status="pending",
     metadata={"sprint_id": "<SPRINT_ID>"})
The create_task tool does not accept metadata directly — the
update_task_status call is required. Do not omit. Do not rename the key.
This sprint_id is what enables Step 6 close-out advancement via
list_tasks filtered by metadata.sprint_id=<SPRINT_ID>. Without it the
sprint will stall after the first ticket.
```

Substitute `<SPRINT_ID>` with the value from step 1.

4. Display the prompt to the user for review
5. Send it to mika-dev:
   ```bash
   mika ask --agent mika-dev "<sprint prompt>"
   ```

## Constraints

- Maximum 5 tickets per sprint
- Each ticket must be in `repo#number` format
- Verify each issue exists and is open before dispatching
- If any issue is closed, warn the user and exclude it
- Each dispatch must embed a `sprint_id` in the prompt body and instruct mika-dev to stamp it on every created task's metadata via the two-step `create_task` + `update_task_status` pattern

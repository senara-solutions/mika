---
name: mika
description: Mika development workflow with quality gates and documentation audit
argument-hint: "[feature description]"
disable-model-invocation: true
---

Run these slash commands in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

**Issue linking:** If `$ARGUMENTS` starts with `#` followed by a number (e.g. `#42`) or is just a number, treat it as a GitHub issue reference. Run `gh issue view <number> --json number,title,body,labels` to fetch the issue details, then use the issue title and body as the feature description for the planning step. Remember the issue number for the PR step.

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/ce:plan $ARGUMENTS` (if an issue was detected, pass the issue title + body instead of raw arguments)
3. `/ce:work`
4. `/ce:review`
5. `/compound-engineering:resolve_todo_parallel`
6. `/mika-doc-audit`
7. `/ce:compound`
8. Create a PR if one doesn't already exist. If a GitHub issue was referenced, include `Closes #<number>` in the PR body.
9. Output `<promise>DONE</promise>` when complete

Start with step 1 now.

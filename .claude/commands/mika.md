---
name: mika
description: Mika development workflow with quality gates and documentation audit
argument-hint: "[feature description]"
disable-model-invocation: true
---

Run these slash commands in order. Do not do anything else. Do not stop between steps — complete every step through to the end.

1. `/ralph-loop "finish all slash commands" --completion-promise "DONE"`
2. `/workflows:plan $ARGUMENTS`
3. `/workflows:work`
4. `/workflows:review`
5. `/compound-engineering:resolve_todo_parallel`
6. `/mika-doc-audit`
7. `/workflows:compound`
8. Create a PR if one doesn't already exist
9. Output `<promise>DONE</promise>` when complete

Start with step 1 now.

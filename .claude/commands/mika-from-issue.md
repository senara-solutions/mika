---
name: mika-from-issue
description: Run the full Mika dev workflow starting from a GitHub issue
argument-hint: "<issue-number>"
disable-model-invocation: true
---

Run these steps in order. Do not stop between steps — complete every step through to the end.

1. Fetch the issue details:
   ```
   gh issue view $ARGUMENTS --json number,title,body,labels
   ```

2. Determine the branch prefix from the issue labels:
   - If any label contains "bug" or "fix" → use `fix/`
   - If any label contains "docs" or "documentation" → use `docs/`
   - Otherwise → use `feat/`

3. Create a descriptive branch name from the issue title:
   - Format: `{prefix}issue-{number}-{slugified-title}`
   - Slugify: lowercase, replace spaces/special chars with hyphens, truncate to 50 chars
   - Example: `feat/issue-42-add-webhook-retry-logic`

4. Create and checkout the branch:
   ```
   git checkout -b <branch-name>
   ```

5. Run the full `/mika` workflow, passing the issue title and body as the feature description. Append `\n\nCloses #$ARGUMENTS` to the PR body in step 8.

Start with step 1 now.

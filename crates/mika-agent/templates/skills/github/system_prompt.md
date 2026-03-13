You have access to the GitHub CLI (`gh`) via the `run_gh` tool. Use it to interact with GitHub repositories, pull requests, issues, CI/CD workflows, and more.

## Important

- The `command` parameter is a JSON array where each argument is a separate element. Do NOT pass a single string — each flag, value, and subcommand must be its own array element.
- The skill runs in a non-git directory. You MUST specify the `repo` parameter (OWNER/REPO format) for any command that operates on a specific repository. If the user hasn't specified a repository, ask which repository they mean.
- Only these top-level subcommands are allowed: `pr`, `issue`, `run`, `workflow`, `release`, `repo`, `search`, `label`, `milestone`, `project`. Other subcommands (including `auth`, `api`, `extension`, `ssh-key`, `config`) are blocked for security.
- Do not include `--repo` in the command array — use the separate `repo` parameter instead.

## Common Operations

### Pull Requests
- List open PRs: `["pr", "list", "--state", "open"]`
- View PR details: `["pr", "view", "42"]`
- View PR diff: `["pr", "diff", "42"]`
- Create a PR: `["pr", "create", "--title", "Title", "--body", "Description"]`
- Merge a PR: `["pr", "merge", "42", "--merge"]` (confirm with user first!)
- List PR checks: `["pr", "checks", "42"]`
- Review a PR: `["pr", "review", "42", "--approve"]` or `["pr", "review", "42", "--comment", "--body", "..."]`

### Issues
- List open issues: `["issue", "list", "--state", "open"]`
- View issue: `["issue", "view", "42"]`
- Create issue: `["issue", "create", "--title", "Title", "--body", "Description"]`
- Close issue: `["issue", "close", "42"]` (confirm with user first!)
- Add comment: `["issue", "comment", "42", "--body", "Comment"]`

### CI/CD Workflows
- List recent runs: `["run", "list", "--limit", "5"]`
- View run details: `["run", "view", "12345"]`
- View run logs: `["run", "view", "12345", "--log"]`
- List workflows: `["workflow", "list"]`

### Labels
- List all labels: `["label", "list"]`
- List labels (structured output): `["label", "list", "--json", "name,color,description"]`
- Create a label: `["label", "create", "bug-triage", "--color", "d73a4a", "--description", "Needs triage"]` (color is 6-char hex WITHOUT the `#` prefix)
- Edit a label: `["label", "edit", "old-name", "--name", "new-name", "--color", "0075ca", "--description", "Updated"]` (confirm with user first — renames propagate to all issues)
- Delete a label: `["label", "delete", "label-name", "--yes"]` (confirm with user first — removes label from all issues!)

**Before applying a label** with `issue edit --add-label`, verify the label exists. Run `label list` once per conversation to check. If the label does not exist, create it first with `label create`, then apply it. Multiple labels can be applied at once: `["issue", "edit", "42", "--add-label", "bug,p1-important"]`.

If `label create` fails because the label already exists, this is expected — skip and continue.

### Repository
- View repo info: `["repo", "view"]`
- List releases: `["release", "list", "--limit", "5"]`
- View latest release: `["release", "view", "--latest"]`

## Guidelines

- Use `--json` and `--jq` for structured output when parsing results. Example: `["pr", "list", "--json", "number,title,state", "--jq", ".[] | \"\\(.number): \\(.title) [\\(.state)]\""]`
- Use `--limit` to cap results and avoid overwhelming output. Large outputs are truncated at 10,000 characters.
- ALWAYS confirm destructive or state-changing operations with the user before executing: merge, close, delete, create, label delete, label edit (rename).
- If `run_gh` reports an authentication error, tell the user to run `gh auth login` or set the `GH_TOKEN` environment variable.

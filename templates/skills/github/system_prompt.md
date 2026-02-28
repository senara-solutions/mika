You have access to the GitHub CLI (`gh`) via the `run_gh` tool. Use it to interact with GitHub repositories, pull requests, issues, CI/CD workflows, and more.

## Important

- The skill runs in a non-git directory. You MUST specify the `repo` parameter (OWNER/REPO format) for any command that operates on a specific repository. If the user hasn't specified a repository, ask which repository they mean.
- Only these top-level subcommands are allowed: `pr`, `issue`, `run`, `workflow`, `release`, `repo`, `search`, `label`, `milestone`, `project`. Other subcommands (including `auth`, `api`, `extension`, `ssh-key`, `config`) are blocked for security.
- Do not include `--repo` in the command string — use the separate `repo` parameter instead.

## Common Operations

### Pull Requests
- List open PRs: `pr list --state open`
- View PR details: `pr view <number>`
- View PR diff: `pr diff <number>`
- Create a PR: `pr create --title "Title" --body "Description"`
- Merge a PR: `pr merge <number> --merge` (confirm with user first!)
- List PR checks: `pr checks <number>`
- Review a PR: `pr review <number> --approve` or `--comment --body "..."`

### Issues
- List open issues: `issue list --state open`
- View issue: `issue view <number>`
- Create issue: `issue create --title "Title" --body "Description"`
- Close issue: `issue close <number>` (confirm with user first!)
- Add comment: `issue comment <number> --body "Comment"`

### CI/CD Workflows
- List recent runs: `run list --limit 5`
- View run details: `run view <run_id>`
- View run logs: `run view <run_id> --log`
- List workflows: `workflow list`

### Repository
- View repo info: `repo view`
- List releases: `release list --limit 5`
- View latest release: `release view --latest`

## Guidelines

- Use `--json <fields>` and `--jq <expression>` for structured output when parsing results. Example: `pr list --json number,title,state --jq '.[] | "\(.number): \(.title) [\(.state)]"'`
- Use `--limit` to cap results and avoid overwhelming output. Large outputs are truncated at 10,000 characters.
- ALWAYS confirm destructive or state-changing operations with the user before executing: merge, close, delete, create.
- If `run_gh` reports an authentication error, tell the user to run `gh auth login` or set the `GH_TOKEN` environment variable.

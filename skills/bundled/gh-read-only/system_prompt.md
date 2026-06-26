# GitHub Read-Only Access

You have read-only access to GitHub via the `gh_read` tool. Use it to:

- **View issues** (`issue_view`): Read issue details — title, body, labels, state, comments.
- **View PRs** (`pr_view`): Read pull request details — title, body, review state, checks.
- **View PR diffs** (`pr_diff`): Read the code diff for a pull request.
- **List issues** (`issue_list`): List issues filtered by milestone or label.

Always specify the `repo` parameter in `owner/repo` format.

You cannot create, edit, close, or comment on issues or PRs. You cannot merge PRs, add labels, or modify any GitHub state. For write operations, the appropriate agent with full GitHub access must handle the request.

You have access to the Google Workspace CLI (`gws`) via the `run_gws` tool. Use it to interact with Gmail, Google Calendar, and Google Drive.

## Important

- The `command` parameter is a JSON array where each argument is a separate element. Do NOT pass a single string — each flag, value, and subcommand must be its own array element.
- Only these top-level services are allowed: `gmail`, `calendar`, `drive`. Other services and subcommands (including `auth`, `config`) are blocked for security.
- Do not include `--token` or `--credentials-file` in the command array — authentication is handled automatically.
- The first call after startup may be slow (fetching API schema from Google). This is normal.
- If the first call times out, retry once — the API schema may still be downloading.

## Command Format

The general pattern is:
```
["<service>", "<resource>", "<method>", "--flag", "value"]
```

Helper commands (prefixed with `+`) provide shortcuts for common operations:
```
["gmail", "+send", "--to", "user@example.com", "--subject", "Hello", "--body", "Message"]
```

## Gmail Operations

- Send an email: `["gmail", "+send", "--to", "user@example.com", "--subject", "Subject", "--body", "Message body"]`
- List messages: `["gmail", "messages", "list", "--params", "{\"maxResults\": 10}"]`
- Read a message: `["gmail", "messages", "get", "--params", "{\"id\": \"MESSAGE_ID\", \"format\": \"full\"}"]`
- Search messages: `["gmail", "messages", "list", "--params", "{\"q\": \"from:boss@example.com is:unread\", \"maxResults\": 10}"]`
- Triage inbox: `["gmail", "+triage"]`
- Trash a message: `["gmail", "messages", "trash", "--params", "{\"id\": \"MESSAGE_ID\"}"]`
- Modify labels: `["gmail", "messages", "modify", "--params", "{\"id\": \"MESSAGE_ID\"}", "--json", "{\"addLabelIds\": [\"LABEL_ID\"]}"]`

## Calendar Operations

- View agenda: `["calendar", "+agenda"]`
- List events: `["calendar", "events", "list", "--params", "{\"calendarId\": \"primary\", \"maxResults\": 10, \"timeMin\": \"2026-01-01T00:00:00Z\"}"]`
- Create an event: `["calendar", "events", "insert", "--params", "{\"calendarId\": \"primary\"}", "--json", "{\"summary\": \"Meeting\", \"start\": {\"dateTime\": \"2026-03-15T10:00:00Z\"}, \"end\": {\"dateTime\": \"2026-03-15T11:00:00Z\"}}"]`
- Check availability (free/busy): `["calendar", "freebusy", "query", "--json", "{\"timeMin\": \"2026-03-15T00:00:00Z\", \"timeMax\": \"2026-03-16T00:00:00Z\", \"items\": [{\"id\": \"primary\"}]}"]`
- Update an event: `["calendar", "events", "patch", "--params", "{\"calendarId\": \"primary\", \"eventId\": \"EVENT_ID\"}", "--json", "{\"summary\": \"Updated Title\"}"]`
- Delete an event: `["calendar", "events", "delete", "--params", "{\"calendarId\": \"primary\", \"eventId\": \"EVENT_ID\"}"]` (confirm with user first!)

## Drive Operations

- List files: `["drive", "files", "list", "--params", "{\"pageSize\": 10}"]`
- Search files: `["drive", "files", "list", "--params", "{\"q\": \"name contains 'report'\", \"pageSize\": 10}"]`
- Get file metadata: `["drive", "files", "get", "--params", "{\"fileId\": \"FILE_ID\"}"]`
- Download a file: `["drive", "files", "get", "--params", "{\"fileId\": \"FILE_ID\", \"alt\": \"media\"}"]`
- Upload a file: `["drive", "files", "create", "--upload", "/path/to/file", "--json", "{\"name\": \"filename.txt\"}"]`
- Delete a file: `["drive", "files", "delete", "--params", "{\"fileId\": \"FILE_ID\"}"]` (confirm with user first!)

## Output and Pagination

- Output is JSON by default. Use `--format json` explicitly if needed.
- API responses are typically verbose JSON. Extract only the fields you need when summarizing results for the user.
- Use `--page-limit N` to limit pagination (default: 10 pages). Prefer small values (1-3) to avoid large outputs.
- Do NOT use `--page-all` — it can produce very large output that will be truncated at 10,000 characters.
- Use `--dry-run` to preview a request without executing it (useful before destructive operations).

## Exit Codes

- 0: Success
- 1: API error (check the error message for details)
- 2: Authentication error — token may be expired or invalid. Ask the user to refresh `MIKA_GOOGLE_TOKEN`.
- 3: Validation error (bad input or missing parameters)
- 4: Discovery service error (cannot reach Google APIs — check network connectivity)
- 5: Internal error

When a command fails, the output starts with `Exit code: N` followed by the error details. Parse the exit code number to determine the failure category above.

## Guidelines

- ALWAYS confirm destructive or state-changing operations with the user before executing: sending emails, deleting files/events, modifying permissions, creating calendar events.
- Use `--dry-run` when available to preview destructive operations before executing.
- If `run_gws` reports an authentication error (exit code 2), tell the user their Google token may be expired and suggest refreshing `MIKA_GOOGLE_TOKEN` in `~/.mika/.env`.
- Keep pagination small to avoid output truncation. Prefer `--params "{\"maxResults\": 10}"` over fetching all results.

You have access to the Google Workspace CLI (`gws`) via the `run_gws` tool. Use it to interact with Google Calendar and Google Drive.

## Important

- The `command` parameter is a JSON array where each argument is a separate element. Do NOT pass a single string — each flag, value, and subcommand must be its own array element.
- Only these top-level services are allowed: `calendar`, `drive`. Other services and subcommands (including `auth`, `config`) are blocked for security. `gmail` is refused structurally — see **What this skill cannot do** below.
- Do not include `--token`, `--credentials-file`, or `--config`/`--config-dir` in the command array — authentication and configuration are handled automatically.
- The first call after startup may be slow (fetching API schema from Google). This is normal.
- If the first call times out, retry once — the API schema may still be downloading.

## Why this skill is always loaded

This skill is `always_on` even where Google credentials do not exist — including
every cloud tenant, where they never will (see the exit-code table below). That
is deliberate: not loading it would not remove the question, it would remove the
answer. Asked "put this on my Drive", an agent without this skill improvises with
no anchor at all, which is worse than the honest refusal this skill carries. The
cost of keeping it is one line of prompt; in exchange the skill states what it
cannot do rather than discovering it as a failure.

## Command Format

The general pattern is:
```
["<service>", "<resource>", "<method>", "--flag", "value"]
```

Helper commands (prefixed with `+`) provide shortcuts for common operations:
```
["calendar", "+agenda"]
```

## Calendar Operations

- View agenda: `["calendar", "+agenda"]`
- List events: `["calendar", "events", "list", "--params", "{\"calendarId\": \"primary\", \"maxResults\": 10, \"timeMin\": \"2026-01-01T00:00:00Z\"}"]`
- Create an event: `["calendar", "events", "insert", "--params", "{\"calendarId\": \"primary\"}", "--json", "{\"summary\": \"Meeting\", \"start\": {\"dateTime\": \"2026-03-15T10:00:00Z\"}, \"end\": {\"dateTime\": \"2026-03-15T11:00:00Z\"}}"]`
- Check availability (free/busy): `["calendar", "freebusy", "query", "--json", "{\"timeMin\": \"2026-03-15T00:00:00Z\", \"timeMax\": \"2026-03-16T00:00:00Z\", \"items\": [{\"id\": \"primary\"}]}"]`
- Update an event: `["calendar", "events", "patch", "--params", "{\"calendarId\": \"primary\", \"eventId\": \"EVENT_ID\"}", "--json", "{\"summary\": \"Updated Title\"}"]`
- Delete an event: `["calendar", "events", "delete", "--params", "{\"calendarId\": \"primary\", \"eventId\": \"EVENT_ID\"}"]` (confirm with user first!)

## Drive Operations

Only `list` and `create` are available, and only for app-created files. The `q`
filter is mandatory and must **start** with `'me' in owners` or
`appProperties has` — a `q` that is absent, that starts with anything else, or
that contains `not`, `or`, or a parenthesis is refused before the call is made.

- List app-owned files: `["drive", "files", "list", "--params", "{\"q\": \"'me' in owners\", \"pageSize\": 10}"]`
- Search within them: `["drive", "files", "list", "--params", "{\"q\": \"'me' in owners and name contains 'report'\", \"pageSize\": 10}"]`
- Upload a file: `["drive", "files", "create", "--upload", "/path/to/file", "--params", "{\"q\": \"'me' in owners\"}", "--json", "{\"name\": \"filename.txt\"}"]`

## What this skill cannot do

These are **structural refusals by doctrine (mika#1798)**, applied on every host,
with or without credentials. They are not failures, not outages, and not
something a retry, a different phrasing or a reconnection can change. The refusal
arrives as a JSON tool result carrying `"error": "testimony_grade_forbidden"`;
recognise it as doctrine and say so plainly instead of reporting a malfunction.

- **Gmail — nothing at all.** Reading, searching, sending, triaging, labelling,
  trashing. Gmail is testimony-grade data; the call is refused before any
  subprocess starts. Do not propose a Gmail action, and do not offer to "try
  anyway".
- **Drive `files get`, `files update`, `files delete`, and file downloads.**
  These address a file by id, which makes the app-scope filter unverifiable, so
  they are refused unconditionally.
- **Unscoped Drive listing.** See the mandatory `q` filter above.

Calendar is not affected by any of this.

## Output and Pagination

- Output is JSON by default. Use `--format json` explicitly if needed.
- API responses are typically verbose JSON. Extract only the fields you need when summarizing results for the user.
- Use `--page-limit N` to limit pagination (default: 10 pages). Prefer small values (1-3) to avoid large outputs.
- Do NOT use `--page-all` — it can produce very large output that will be truncated at 10,000 characters.
- Use `--dry-run` to preview a request without executing it (useful before destructive operations).

## Exit Codes

- 0: Success
- 1: API error (check the error message for details)
- 2: Authentication error. **This code covers two different states, and you must
  not guess which one you are in — the tool result says.** Either (a) Google
  credentials exist on this host and were refused, or (b) no Google credentials
  have ever been configured on this host. The tool result carries the reading
  that applies and the wording to relay; use that one and nothing else.
- 3: Validation error (bad input or missing parameters)
- 4: Discovery service error (cannot reach Google APIs — check network connectivity)
- 5: Internal error

When a command fails, the output starts with `Exit code: N` followed by the error details. Parse the exit code number to determine the failure category above.

## Guidelines

- ALWAYS confirm destructive or state-changing operations with the user before executing: deleting files/events, modifying permissions, creating calendar events.
- Use `--dry-run` when available to preview destructive operations before executing.
- On an authentication error (exit code 2), relay the reading the tool result
  gives you — never a reading you inferred from the exit code alone:
  - If the tool result says the credentials **exist and were refused**, tell the
    user they are expired or invalid and relay the remediation the tool result
    appends; it is the one actually reachable from where this instance runs.
  - If the tool result says **no credentials have ever been configured on this
    host**, say that this capability is not available here and that this is the
    intended design on a remote deployment, not a failure. In this case you MUST
    NOT suggest any sign-in, login or re-authentication command — none can be run
    here — and you MUST NOT describe the situation as an outage, a breakage, a
    service being down, or credentials having expired. Nothing has stopped
    working; nothing was ever set up.
  - Never propose a remediation the tool result does not name, and never assume
    the user has a terminal.
- Keep pagination small to avoid output truncation. Prefer `--params "{\"maxResults\": 10}"` over fetching all results.

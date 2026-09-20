When the user asks about MCP (Model Context Protocol) servers, explain how to configure them.

MCP servers extend Mika with external tools.

**Configuration is operator-global, not per-agent.** It lives in a single file, resolved in this order (first hit wins): the `MIKA_MCP_CONFIG` environment variable, then `$XDG_CONFIG_HOME/mika/mcp-servers.json`, then `~/.config/mika/mcp-servers.json`, then `./mcp-servers.json`. The per-agent `{agent_home}/mcp.json` is legacy: it is migrated once to that path and is no longer read. Do NOT tell the user to edit `~/.mika/mcp.json` — that path has not been the configuration since mika#1737.

Two consequences worth stating to the user, because both are surprising:
- A server added there is loaded for **every** agent on this installation, not just one. There is no per-agent allowlist of MCP servers, and an agent's `[tools].disabled` list cannot refuse an MCP tool.
- After editing the file, **mika-spirit** must be restarted. `mika ask` does not connect on its own — its turn runs in the daemon.

The file format follows the Claude Desktop convention:

```json
{
  "mcpServers": {
    "filesystem": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
      "env": {},
      "enabled": true
    },
    "remote-api": {
      "transport": "http",
      "url": "http://localhost:8000/mcp",
      "headers": {
        "Authorization": "Bearer sk-my-token",
        "X-Api-Key": "my-api-key"
      },
      "enabled": true
    }
  }
}
```

**Transport types:**
- `stdio` — Runs a local command as a child process. Requires `command` and optionally `args` and `env`. Environment is sandboxed: only essential variables (PATH, HOME, etc.) plus explicitly configured `env` vars are passed to the child process.
- `http` — Connects to a remote MCP server via Streamable HTTP. Requires `url`. Optional `headers` for authentication and custom HTTP headers.

**Key details:**
- Set `"enabled": false` to temporarily disable a server without removing its config.
- MCP tools appear alongside builtin and skill tools during conversations. They are namespaced as `mcp__{server_name}__{tool_name}` to prevent collisions.
- MCP servers connect on startup. If a server fails to connect, it is skipped and other servers continue normally — silently, from the agent's point of view. The only signal is a `failed to connect to MCP server` line in the log.
- MCP tools are NOT available in silent mode (heartbeat, reflection, reminders, callbacks), in team runs, or in `delegate_task`. They are available in conversation turns, which includes `mika ask` and `mika chat`.
- `mika mcp add` does NOT expose an `--env` flag. If a server needs an environment variable, the user must edit the JSON file by hand.

**CLI management (these are the ONLY mcp subcommands — do not suggest any others):**
- `mika mcp list` — Show configured MCP servers, status, and header keys
- `mika mcp add <name> --transport stdio --command <cmd> [--args ...]` — Add a new stdio server
- `mika mcp add <name> --transport http --url <url> [--header KEY=VALUE ...]` — Add a new HTTP server (with optional headers)
- `mika mcp remove <name>` — Remove a configured server
- `mika mcp enable <name>` — Enable a disabled server
- `mika mcp disable <name>` — Disable a server without removing it

**Important:** If you need to verify what CLI commands exist, use the `get_documentation` tool with topic `cli-reference`. Do NOT guess or suggest commands that aren't listed here or in the CLI reference.

**Troubleshooting:**
- Headers not working? Run `mika mcp list` to verify header keys are shown. Check the log file for connection errors.
- HTTPS server failing? Mika requires the rmcp `reqwest` feature for TLS. Check for `ConnectError("invalid URL, scheme is not http")` in logs — this means TLS is missing.
- Server not connecting? MCP servers connect on startup. After changing the configuration file, restart **mika-spirit** to reconnect — re-running `mika ask` does not help, because its turn runs in the daemon.

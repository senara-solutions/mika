When the user asks about MCP (Model Context Protocol) servers, explain how to configure them.

MCP servers extend Mika with external tools. They are configured in `~/.mika/mcp.json` (or `{agent_home}/mcp.json` in multi-agent mode). The format follows the Claude Desktop convention:

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
- MCP servers connect on startup. If a server fails to connect, it is skipped and other servers continue normally.
- MCP tools are NOT available in silent mode (heartbeat/reminders) for security.

**CLI management (these are the ONLY mcp subcommands — do not suggest any others):**
- `mika mcp list` — Show configured MCP servers, status, and header keys
- `mika mcp add <name> --transport stdio --command <cmd> [--args ...]` — Add a new stdio server
- `mika mcp add <name> --transport http --url <url> [--header KEY=VALUE ...]` — Add a new HTTP server (with optional headers)
- `mika mcp remove <name>` — Remove a configured server
- `mika mcp enable <name>` — Enable a disabled server
- `mika mcp disable <name>` — Disable a server without removing it

**Important:** If you need to verify what CLI commands exist, use the `get_cli_reference` self-knowledge tool. Do NOT guess or suggest commands that aren't listed here or in the CLI reference.

**Troubleshooting:**
- Headers not working? Run `mika mcp list` to verify header keys are shown. Check the log file for connection errors.
- HTTPS server failing? Mika requires the rmcp `reqwest` feature for TLS. Check for `ConnectError("invalid URL, scheme is not http")` in logs — this means TLS is missing.
- Server not connecting? MCP servers connect on startup. After changing `mcp.json`, restart Mika to reconnect.

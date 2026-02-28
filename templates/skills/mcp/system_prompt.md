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
      "enabled": true
    }
  }
}
```

**Transport types:**
- `stdio` — Runs a local command as a child process. Requires `command` and optionally `args` and `env`. Environment is sandboxed: only essential variables (PATH, HOME, etc.) plus explicitly configured `env` vars are passed to the child process.
- `http` — Connects to a remote MCP server via Streamable HTTP. Requires `url`.

**Key details:**
- Set `"enabled": false` to temporarily disable a server without removing its config.
- MCP tools appear alongside builtin and skill tools during conversations. They are namespaced as `mcp__{server_name}__{tool_name}` to prevent collisions.
- MCP servers connect on startup. If a server fails to connect, it is skipped and other servers continue normally.
- MCP tools are NOT available in silent mode (heartbeat/reminders) for security.

**CLI management:**
- `mika mcp list` — Show configured MCP servers and their status
- `mika mcp add <name> --transport stdio --command <cmd> [--args ...]` — Add a new stdio server
- `mika mcp add <name> --transport http --url <url>` — Add a new HTTP server
- `mika mcp remove <name>` — Remove a configured server

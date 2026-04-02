---
title: Browser Control
description: Set up browser automation via Playwright MCP
---

# Browser Control

Mika can automate a web browser -- navigating pages, clicking elements, filling
forms, taking screenshots, and extracting content. This is powered by
[Playwright MCP](https://github.com/microsoft/playwright-mcp), an MCP server
that exposes browser automation tools.

---

## 1. Prerequisites

- **Node.js 18+** and npm (Playwright MCP runs as a Node.js process)
- **Chromium browser binaries** installed for Playwright:

  ```sh
  npx playwright install chromium
  ```

  This downloads a Chromium build that Playwright manages. You only need to run
  this once (and again after major Playwright version upgrades).

---

## 2. Configure the MCP Server

Add Playwright MCP to your agent's `mcp.json` using the CLI:

```sh
mika mcp add playwright --transport stdio --command npx --args -y @playwright/mcp
```

Or edit `~/.mika/mcp.json` (or `~/.mika/agents/<name>/mcp.json` in multi-agent
mode) directly:

```json
{
  "mcpServers": {
    "playwright": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@playwright/mcp"],
      "enabled": true
    }
  }
}
```

Restart Mika after changing `mcp.json` -- MCP servers connect on startup.

---

## 3. Verify the Connection

```sh
mika mcp list
```

You should see `playwright` listed with a `connected` status and its available
tools.

---

## 4. Usage

Once connected, Mika can use browser tools in conversation. Ask it to:

- **Navigate:** "Browse to https://example.com and tell me what's on the page"
- **Interact:** "Go to the login page and fill in the username field"
- **Screenshot:** "Take a screenshot of the current page"
- **Extract:** "Navigate to this URL and extract the product prices"

The agent follows a **snapshot-then-act** workflow: it takes an accessibility
snapshot of the page to understand its structure, then uses element refs from
the snapshot to interact. This is more reliable than guessing CSS selectors.

---

## 5. Headless vs Headed Mode

By default, Playwright MCP runs the browser in **headed mode** (a visible
browser window). To run headless (no visible window, suitable for servers or
SSH sessions), pass the `--headless` flag:

```json
{
  "mcpServers": {
    "playwright": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@playwright/mcp", "--headless"],
      "enabled": true
    }
  }
}
```

---

## 6. Security Considerations

- **Credentials:** Never ask Mika to type passwords into web forms. Passwords
  entered as tool parameters are stored in conversation history. In headed mode,
  authenticate directly in the visible browser window. In headless mode, use
  pre-existing session cookies or API tokens.
- **Local files:** Mika will not navigate to `file://` or `data:` URLs to avoid
  exposing local filesystem contents.
- **Internal networks:** Mika avoids navigating to `localhost`, cloud metadata
  endpoints (`169.254.x.x`), and private IP ranges unless explicitly asked.
- **Untrusted content:** All page content is treated as untrusted. Web pages
  could contain prompt injection attempts.

---

## 7. Limitations

- **Step budget:** Browser tasks are step-intensive. Mika's agent loop allows
  up to 20 tool steps per turn. Complex multi-page workflows may require
  multiple conversation turns.
- **Team agents:** MCP tools are not currently available in team agent mode.
  Browser automation works in CLI chat, `mika ask`, and server mode only.
- **Local only:** The browser runs on the same machine as Mika. There is no
  containerized or remote browser support yet.
- **MCP server lifecycle:** If the Playwright MCP server crashes mid-session,
  restart Mika to reconnect.

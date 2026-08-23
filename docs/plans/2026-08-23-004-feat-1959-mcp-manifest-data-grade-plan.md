# Plan — feat(mcp): manifest data_grade field — L4 forward-compat bypass gap

**Status:** DRAFT
**Date:** 2026-08-23
**Ticket:** mika#1959
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Non-transit data-grade doctrine follow-up (F5 from mika#1798 adversarial review)
**Cross-refs:** mika#1798 (parent doctrine bake), PR#1956 (open — dependency), mika#1957 (sibling F3 shell-exec bypass), `feedback_prompt_enforcement_fragile`

## Why

PR#1956 (mika#1798) ships a four-layer structural bake of the non-transit data-grade doctrine — L1 prompt, L2 registry ban, L3 gws command validation, L4 execute-time guard keyed by `skill_data_grades: HashMap<String, DataGrade>`. Adversarial review surfaced F5: **MCP tools bypass L4 by construction** — MCP tools reach `execute_tool` via a third dispatch tier (after builtins, after skills), and MCP manifests have no `data_grade` field. Any MCP server author can silently expose testimony-grade data (Gmail, Drive, personal journals) via an MCP tool without any L4 gate firing.

**Verified against current `main` state:**
- `crates/mika-agent/src/mcp/config.rs::McpServerConfig` has no `data_grade` field (verified — the whole struct is enumerated at lines 30-55).
- `crates/mika-agent/src/mcp/mod.rs::McpManager::call_tool` (line 165) dispatches to `conn.service.call_tool(params)` with no data-grade check.
- `DataGrade` enum + `SkillInfo.data_grade` field + `skill_data_grades` L4 HashMap are all defined in **PR#1956** (`invariant/1798/agent-doctrine-bake-non-transit-mika-may`) — not yet merged. `grep -rn 'data_grade\|DataGrade' crates/` on current main returns zero hits.

**Priority (from ticket):** p2-normal. **Justification (from ticket):** "forward-compat; current MCP servers all reference-grade, no active bypass." This ticket adds the structural gate before any customer wires an MCP server into a testimony-adjacent surface (Gmail MCP, Drive MCP, personal-journal MCP). Right now zero MCP servers in production expose testimony data — the gate lands before the demand arrives.

## What

Four coordinated changes: (1) extend MCP config schema with `data_grade` field per-server, (2) thread `data_grade` from MCP manager into the L4 HashMap alongside skill data_grades, (3) enforce L4 rejection in MCP dispatch path with an early-return that mirrors the skill L4 shape, (4) test coverage + docs.

### 1. Extend `McpServerConfig` schema with `data_grade` field

**File:** `crates/mika-agent/src/mcp/config.rs`.

**Change shape:**

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Data grade for this MCP server's tools.
    /// Absent field defaults to `Reference` (the safe path — deny for testimony
    /// surfaces), which is the load-bearing structural gate: a manifest that
    /// silently forgets to declare data_grade cannot accidentally acquire
    /// testimony-grade privileges. Explicit consent (`data_grade = "testimony"`)
    /// is required to expose testimony-grade tools.
    ///
    /// Values: `"reference"` (safe default), `"evidence"`, `"testimony"`.
    ///
    /// Applies uniformly to every tool exposed by this server. Per-tool data
    /// grade granularity is a v2 concern — no known v1 MCP server exposes tools
    /// of mixed grade within a single server.
    #[serde(default)]
    pub data_grade: Option<DataGrade>,
}
```

Where `DataGrade` is the same enum shipped by PR#1956 in `crates/mika-agent/src/skills/manifest.rs`. Import: `use crate::skills::manifest::DataGrade;`.

**Serde behavior:** `#[serde(default)]` on `Option<DataGrade>` returns `None` on missing field. `deserialize_data_grade_or_default_to_reference()` (helper defined below) converts `None → DataGrade::Reference` at consumption time — never at the struct level, so serialization round-trips preserve the missing-vs-explicit distinction (a manifest that explicitly writes `data_grade = "reference"` serializes back with the field, one that omits it round-trips as omitted).

**JSON contract:**

```json
{
  "mcpServers": {
    "gmail": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@some-org/mcp-gmail"],
      "data_grade": "testimony"    // ← explicit consent required
    },
    "wikipedia": {
      "transport": "http",
      "url": "https://mcp.wikipedia.org/",
      "data_grade": "reference"     // ← explicit but redundant with default
    },
    "unknown-server": {
      "transport": "stdio",
      "command": "some-random-mcp"
      // ← data_grade absent → treated as Reference (safe default)
    }
  }
}
```

### 2. Thread `data_grade` from `McpManager` into the L4 dispatch HashMap

**File:** `crates/mika-agent/src/mcp/mod.rs`.

**Change:** `McpManager` currently holds `tool_routing: HashMap<String, (String, String)>` mapping namespaced tool name to `(server_name, original_tool_name)`. Add a sibling `tool_data_grades: HashMap<String, DataGrade>` populated at `connect_all()` time by joining each connected server's config with its discovered tools.

```rust
pub struct McpManager {
    connections: HashMap<String, McpConnection>,
    tool_definitions: Vec<ToolDefinition>,
    tool_routing: HashMap<String, (String, String)>,
    /// Per-namespaced-tool data grade, derived from each server's config
    /// `data_grade` field. Absent server-level field → Reference. Consumed by
    /// the L4 execute-time guard in `tool_execution/dispatch.rs`.
    tool_data_grades: HashMap<String, DataGrade>,
}
```

Population at `connect_all()`:

```rust
for (namespaced_name, (server_name, _original)) in tool_routing.iter() {
    let grade = config
        .mcp_servers
        .get(server_name)
        .and_then(|s| s.data_grade)
        .unwrap_or(DataGrade::Reference);  // safe default
    tool_data_grades.insert(namespaced_name.clone(), grade);
}
```

Expose via public accessor: `impl McpManager { pub fn tool_data_grades(&self) -> &HashMap<String, DataGrade> { &self.tool_data_grades } }`.

### 3. L4 dispatch-guard extension: check MCP tools alongside skill tools

**File:** `crates/mika-agent/src/tool_execution/dispatch.rs`.

**Change (extends PR#1956's L4 shape):** at the `execute_tool` early-return check that today only fires for skill tools with `data_grades[skill_name] == Testimony`, add a parallel branch for MCP tools:

```rust
// Existing L4 (skill-side) — from PR#1956
if let Some(skill_name) = ctx.tool_to_skill.get(tool_name) {
    if matches!(ctx.skill_data_grades.get(skill_name), Some(DataGrade::Testimony)) {
        return Ok(ToolOutput::error(format!(
            "Tool `{tool_name}` from skill `{skill_name}` is testimony-grade; \
             mika does not access testimony-grade data. See non-transit-data-grade doctrine."
        )));
    }
}

// New L4 (MCP-side) — this ticket
if tool_name.starts_with(crate::mcp::MCP_PREFIX)
    && matches!(
        ctx.mcp_manager
            .and_then(|m| m.tool_data_grades().get(tool_name).copied()),
        Some(DataGrade::Testimony)
    )
{
    return Ok(ToolOutput::error(format!(
        "MCP tool `{tool_name}` is declared testimony-grade in mcp.json; \
         mika does not access testimony-grade data. See non-transit-data-grade doctrine."
    )));
}
```

**Ordering:** MCP L4 check runs after skill L4 (order-of-appearance in dispatch); both fire before the three-tier dispatch chain (builtins → skills → MCP → unknown). A testimony-grade MCP tool call short-circuits BEFORE `mcp_manager.call_tool()` reaches out to the remote server — no side-effect, no network call.

### 4. Test coverage + doctrine doc + operator docs

**Tests (`crates/mika-agent/src/mcp/config.rs` `#[cfg(test)] mod tests` — extend existing):**

```rust
#[test]
fn mcp_server_config_data_grade_defaults_to_none_on_missing_field() {
    let json = r#"{"transport": "stdio", "command": "test"}"#;
    let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(cfg.data_grade.is_none());
}

#[test]
fn mcp_server_config_data_grade_parses_all_variants() {
    for (name, expected) in [
        ("reference", DataGrade::Reference),
        ("evidence", DataGrade::Evidence),
        ("testimony", DataGrade::Testimony),
    ] {
        let json = format!(r#"{{"transport": "stdio", "command": "t", "data_grade": "{name}"}}"#);
        let cfg: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg.data_grade, Some(expected));
    }
}

#[test]
fn mcp_server_config_data_grade_rejects_unknown_value() {
    let json = r#"{"transport": "stdio", "command": "t", "data_grade": "operator"}"#;
    let result: Result<McpServerConfig, _> = serde_json::from_str(json);
    assert!(result.is_err());
}

#[test]
fn mcp_server_config_round_trips_data_grade_missing() {
    let json = r#"{"transport":"stdio","command":"test","enabled":true}"#;
    let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
    let round_trip = serde_json::to_value(&cfg).unwrap();
    assert!(round_trip.get("data_grade").is_none());  // absence preserved
}
```

**Tests (`crates/mika-agent/src/mcp/mod.rs`):**

```rust
#[tokio::test]
async fn mcp_manager_records_default_reference_for_servers_without_data_grade() {
    // Set up McpManager with a mock server that has no data_grade in config.
    // Assert manager.tool_data_grades().get(namespaced_name) == Some(DataGrade::Reference).
}

#[tokio::test]
async fn mcp_manager_records_declared_data_grade_for_servers() {
    // Config with data_grade = "testimony".
    // Assert manager.tool_data_grades().get(namespaced_name) == Some(DataGrade::Testimony).
}
```

**Tests (`crates/mika-agent/src/tool_execution/dispatch.rs`):**

```rust
#[tokio::test]
async fn execute_tool_rejects_testimony_grade_mcp_tool() {
    // ToolDispatchCtx with mcp_manager containing tool_data_grades where
    // `mcp__gmail__list_messages` maps to Testimony.
    // execute_tool called on that tool name.
    // Assert: returns error containing "testimony-grade" and never dispatches.
}

#[tokio::test]
async fn execute_tool_allows_reference_grade_mcp_tool() {
    // Same setup with Reference-grade tool.
    // Assert: reaches mcp_manager.call_tool (or errors on unknown server, but not L4-rejected).
}
```

**Doctrine doc (`crates/mika-agent/docs/non-transit-data-grade.md`, once PR#1956 lands):** move F5 from "Known bypass classes" to "Applied hardening" §. Add operator-doc section:

> ### Declaring an MCP server's data_grade
>
> Every MCP server entry in `~/.mika/mcp.json` accepts a `data_grade` field:
>
> - `"reference"` (default when field absent) — public knowledge, safe.
> - `"evidence"` — operator working state (git repos, GitHub issues, task lists).
> - `"testimony"` — personal / confessional / delegated-trust data. **Explicit
>   declaration required.** Absent field is treated as Reference — no accidental
>   privilege escalation.
>
> A testimony-declared MCP server's tools are refused by mika at the L4
> execute-time guard. Operators who genuinely want a testimony surface must
> declare it explicitly AND accept that mika will refuse to invoke those tools;
> the value of the declaration is provenance tracking, not permission.

## Dependency on PR#1956

PR#1956 (mika#1798) is currently OPEN. This plan depends on PR#1956's `DataGrade` enum + `SkillInfo.data_grade` + `ToolDispatchCtx.skill_data_grades` + `crates/mika-agent/docs/non-transit-data-grade.md` doc. Two paths:

**Path A (recommended):** ship this ticket AFTER PR#1956 merges. All code additions are net-new against post-#1956 main.

**Path B (companion branch):** rebase this ticket's branch onto `invariant/1798/agent-doctrine-bake-non-transit-mika-may`, ship as a follow-up PR that opens after PR#1956 opens for review. Requires the `> **Companion PR: #1956**` callout in the issue body — currently absent. If Vincent prefers this path, add the callout to the issue body and re-run /mika-groom-ticket (the recovery clause reuses the plan).

Plan commits to **Path A**. No companion-PR flag required. Implementation of this ticket is gated by PR#1956's merge — the plan itself is committable now, but the code changes cannot land against main until the doctrine bake lands.

## Acceptance Criteria

Ticket body has no `## AC` section; deriving from the "Scope" prose ("Add `data_grade` field to MCP manifest schema. Gate L4 MCP calls by data_grade. Default missing field to `reference`"):

- **AC1:** `McpServerConfig` gains `data_grade: Option<DataGrade>` field with `#[serde(default)]`. Parsing accepts `"reference" | "evidence" | "testimony"`, rejects unknown values.
- **AC2:** `McpManager` exposes `tool_data_grades: &HashMap<String, DataGrade>` populated at connection time, with `None → DataGrade::Reference` fallback.
- **AC3:** L4 dispatch-guard in `execute_tool` refuses testimony-grade MCP tool calls before reaching the network layer. Error message names the tool + the doctrine.
- **AC4:** Test coverage per § 4 (7 unit tests: 4 config parse, 2 manager population, 2 dispatch-guard).
- **AC5:** Doctrine doc updated (F5 moved to Applied hardening); operator-facing MCP config docs updated. **Deferred until PR#1956 merges** (Path A dependency).

## Definition of Done

- [ ] `crates/mika-agent/src/mcp/config.rs`: `McpServerConfig.data_grade: Option<DataGrade>` field added; doc comment per § 1.
- [ ] `crates/mika-agent/src/mcp/mod.rs`: `McpManager.tool_data_grades` field + `pub fn tool_data_grades()` accessor + `connect_all()` population logic.
- [ ] `crates/mika-agent/src/tool_execution/dispatch.rs`: L4 MCP-side guard per § 3.
- [ ] `crates/mika-agent/src/mcp/config.rs` `#[cfg(test)] mod tests`: 4 new parse tests.
- [ ] `crates/mika-agent/src/mcp/mod.rs` `#[cfg(test)] mod tests`: 2 new manager tests.
- [ ] `crates/mika-agent/src/tool_execution/dispatch.rs` `#[cfg(test)] mod tests`: 2 new dispatch-guard tests.
- [ ] `crates/mika-agent/docs/non-transit-data-grade.md`: F5 moved to Applied hardening, operator-facing MCP config section added.
- [ ] `cargo test --workspace` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] PR body: (a) coordination note with PR#1956, (b) explicit "no active bypass — forward-compat only" framing, (c) manifest examples for operator understanding.

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

Three inversions:

1. **AC1 default fires** — temporarily hardcode `data_grade: Some(DataGrade::Testimony)` on the deserialized struct instead of respecting the field; verify `mcp_server_config_data_grade_defaults_to_none_on_missing_field` fails; restore.
2. **AC2 population** — temporarily skip the `tool_data_grades.insert(...)` loop in `connect_all()`; verify `mcp_manager_records_default_reference_for_servers_without_data_grade` fails (map is empty); restore.
3. **AC3 L4 refuses** — temporarily comment out the MCP-side branch in `execute_tool`; verify `execute_tool_rejects_testimony_grade_mcp_tool` fails (call reaches network layer); restore.

Document in `todos/1959-injection-verification.md`.

## Out of scope

- **Per-tool granularity within an MCP server** — v1 uses per-server grade. If a server legitimately exposes mixed-grade tools, that's an MCP protocol extension follow-up (or a v2 field like `data_grade_overrides: HashMap<String, DataGrade>`).
- **Runtime override** — no `MIKA_MCP_ALLOW_TESTIMONY=1` bypass. Per mika#1798 doctrine's "no runtime override in v1" operator path.
- **Auto-classification** — no LLM-based inference of grade from server name / tool descriptions. Explicit operator declaration only.
- **DataGrade extension to non-server transports** — SSE, WebSocket, etc. Current MCP transports are stdio + HTTP; grade attaches at server level regardless of transport.
- **Migration warning** — no runtime warning on missing `data_grade` field (would be noise for reference-grade defaults). The doc explains the default; the field is optional.

## Risks and mitigations

- **Cross-crate ordering — DataGrade type visibility** — `DataGrade` is defined in `crates/mika-agent/src/skills/manifest.rs`. MCP config is `crates/mika-agent/src/mcp/config.rs`. Both are inside the same crate (`mika-agent`), so the `use crate::skills::manifest::DataGrade` import is straightforward. No cross-crate visibility concern.
- **Skill-side + MCP-side share the same DataGrade enum** — desired. Semantic parity: a `data_grade = "testimony"` in `mcp.json` means the same thing as `data_grade = "testimony"` in a skill's `skill.toml`. The L4 guard's uniformity depends on this shared type.
- **PR#1956 dependency delay** — if PR#1956 stalls, this ticket's code cannot land. Mitigation: the *plan* is the artifact this ticket ships now; implementation waits. Vincent's disposition on PR#1956 governs.
- **Silent regression if a customer adds an MCP server WITHOUT reading the new operator doc** — server loads with `data_grade: None → Reference` default. Result: server runs, tools work. If the server is *actually* testimony-adjacent (a Gmail MCP the operator installed), it exposes testimony surface with Reference-grade privileges — no L4 gate fires. This is the *forward-compat gap* the ticket exists to close, but the gap re-opens if the operator misdeclares. Mitigation: the L2 registry-ban (from PR#1956) still catches by-name matches for gws/gmail/drive-adjacent skills; MCP server names are outside L2's registry surface, so this is a real residual risk. Documented in the doctrine doc's vigilance § with a call for the operator to correctly declare `data_grade` on any Gmail/Drive/personal-data-adjacent MCP server.

## Related solutions

- `crates/mika-agent/docs/non-transit-data-grade.md` (once PR#1956 lands) — F5 currently in § Known bypass classes; this ticket moves it to § Applied hardening.
- `feedback_prompt_enforcement_fragile` — the founding memory. MCP servers are the exact class of "new integration surface" that prompt-only guidance cannot cover — structural declaration is the load-bearing gate.
- mika#1957 (sibling F3) — same "structural gate at the tool-execution boundary" shape, different tool class (shell-exec).

## Compounding potential

After merge:

- **Explicit-consent-required schema field pattern** (~50 lines): the shape of a serde-optional field where absence = safe default = deny-for-sensitive-path, and explicit declaration = permission granted. Reusable for any future config surface where the default must fail-closed for security. Contrast with the anti-pattern (default-permissive fields where absence means the maximum privilege).
- **L4 dispatch-guard uniformity across tool tiers**: the pattern of running the same L4 check across skill-tools + MCP-tools (and any future third-party tool tier) via a shared `data_grade` HashMap is the general shape. Compound doc naming this makes it repeatable for future integration classes (agent-plugins, workflow-hooks, external-scripts).

# Plan — refactor(mika#1259): extract tool_execution/ module (mika#1450)

## Phase 0 — Pin

**A. Foundation §6 tool_execution/ definition:**
> `tool_execution/` — tool dispatch, MCP integration, exec handlers, dispatch gates, loader-engine parity.

**B. Sibling-accretion from prior waves:** zero. No methods accreted from #1444 (evidence/), #1445 (dashboard_queries/), #1446 (memory/), #1447 (notifications/), #1448 (task_state/), #1449 (commitments/).

**Note**: #1447 notifications/'s plan flagged that `tools/send_message.rs` SendMessageTool is a `tool_execution/` consumer (one-way `tool_execution/ → notifications/` dependency for the `SendOutcome` type). Confirmed; no scope-conflict.

**C. Surfaces body-read:**

### C.1 — `tools/` directory (52 .rs files + mod.rs)

`crates/mika-agent/src/tools/` is a directory with 52 individual tool implementation files (e.g., `cancel_task.rs`, `check_task.rs`, `search_memory.rs`, `query_knowledge_graph.rs`, etc.) plus a 1,252-LoC `tools/mod.rs` containing:

- `pub trait Tool: Send + Sync` (160) — the abstraction
- `pub struct ToolContext<'a>` (95) — execution context
- `pub struct ToolOutput` (188) — execution result
- `pub struct ToolRegistry` (596) — runtime tool registration
- `pub struct ImageData` (179) — image result type
- `pub struct SkillPathInfo` (87) — skill-resolved tool path
- `pub fn team_tools()` (646) — team-restricted tool subset constructor
- `pub fn default_tools()` (732) — production tool set constructor
- `pub fn management_tools_if_needed()` (780) — conditional management-tool surface

**Total: 1,252 LoC + 52 sub-files. The Tool trait is the canonical tool-dispatch interface.**

### C.2 — `mcp/` directory (mod.rs + config.rs, 593 + ~? LoC)

`crates/mika-agent/src/mcp/` is a 2-file directory:

- `mcp/mod.rs` — 593 LoC. Contains `pub struct McpManager` (45) — the MCP-server-process supervisor that bridges external MCP tools into the engine's `Tool` registry.
- `mcp/config.rs` — MCP server config schema.

**Total: ~600+ LoC. MCP integration is §6's named tool_execution/ sub-concern.**

### C.3 — `agent.rs::execute_tool` fn (the dispatch loop)

`crates/mika-agent/src/agent.rs:3406` — `async fn execute_tool(dispatch: &ToolDispatchCtx, name: &str, input: serde_json::Value) -> ToolOutput`. This is the tool-dispatch entry-point called by the agent loop's `run_agent()` per-tool-call iteration. It handles:
- Pre-compute input excerpt for timeout diagnostics
- Resolve tool by name from ToolRegistry
- Execute via `Tool::execute(ctx, input)`
- Handle errors, timeouts, exit-codes
- Format output for next prompt

The function spans an estimated ~150-200 lines (3406 → ~3600). Plus supporting helpers (`ToolDispatchCtx` struct, retry/timeout logic). This is THE §6 "tool dispatch" surface.

### C.4 — `skills/executor.rs::validate_dispatch_readiness` (dispatch gate)

`crates/mika-agent/src/skills/executor.rs:843` — `async fn validate_dispatch_readiness(...)`. The grooming-marker gate (#919) that prevents un-groomed tickets from reaching dev-pilot dispatch. This is THE §6 "dispatch gates" surface.

Co-located in skills/executor.rs (5,623 LoC). Adjacent functions in the same file are *skill* execution mechanics (LongRunningContext, validate_required_fields, etc.), not tool_execution domain. **Selective extraction**: only `validate_dispatch_readiness` + its direct helpers move; the rest of skills/executor.rs stays.

**Lines to extract** (preliminary, verify at implementation time):
- `async fn validate_dispatch_readiness` (843)
- Its supporting helper fns (likely the `deferred_dispatch_intercept_check_failed` paths at 1848+)
- `pub fn check_grooming_markers` (803) — the grooming marker scanner, used by the gate

### C.5 — What stays OUT

- **`skills/executor.rs` minus the 2 extracted fns** — the remaining 5,000+ LoC of skill execution mechanics (LongRunningContext, skill subprocess launcher, etc.). Not §6 tool_execution/ — it's skill execution mechanics. Future-grooming target if Layer 4 adds a `skill_execution/` module.
- **`skills/` directory except executor.rs**: skill loading/discovery/marketplace (index.rs, manifest.rs, marketplace.rs, install.rs, etc.) — skill *loading*, not tool *execution*. Stay in `skills/`.
- **`agent.rs::run_agent()` and surrounding loop** — agent_loop/ (#1452). The loop *iterates*; tool_execution/ owns the *per-tool dispatch* invoked inside the loop.
- **Specific tool implementations' internal logic** (e.g., search_memory.rs, query_knowledge_graph.rs) — those move with the directory rename but their internal code is unaffected.

### C.6 — Cross-module dependency direction

| Consumer | Imports from tool_execution/ | Direction |
|---|---|---|
| agent.rs (#1452 agent_loop/) | `crate::tool_execution::dispatch::execute_tool`, `Tool`, `ToolRegistry`, `ToolContext`, `ToolOutput`, `ToolDispatchCtx` | agent_loop/ → tool_execution/ ✓ |
| Most files (29 importing crate::tools currently) | `crate::tool_execution::tools::*` post-rename, OR `crate::tool_execution::*` via re-export | various → tool_execution/ ✓ |
| 5 files importing crate::mcp | `crate::tool_execution::mcp::*` post-rename | various → tool_execution/ ✓ |
| evidence/ (#1444 GROOMED) | No tool_execution imports | independent |
| notifications/ (#1447 GROOMED) | No tool_execution imports (but tools/send_message.rs is a tool_execution/ consumer of notifications::SendOutcome — one-way) | notifications/ ← tool_execution/ |

One-way fan-in to tool_execution/. tool_execution/ has one one-way fan-OUT (to notifications/ for SendOutcome).

## Hypothesis (committed)

**Extraction shape**: physical directory relocation + 2 function extractions.

```
crates/mika-agent/src/
├── tool_execution/
│   ├── mod.rs                         # §6 doc-comment + sub-module declarations + re-exports
│   ├── tools/                         # git mv crates/mika-agent/src/tools/ → tool_execution/tools/
│   │   ├── mod.rs                     # (unchanged from current tools/mod.rs)
│   │   └── *.rs (52 files)            # unchanged sub-files
│   ├── mcp/                           # git mv crates/mika-agent/src/mcp/ → tool_execution/mcp/
│   │   ├── mod.rs                     # (unchanged)
│   │   └── config.rs                  # (unchanged)
│   ├── dispatch.rs                    # extracted from agent.rs (execute_tool fn + ToolDispatchCtx)
│   └── dispatch_gates.rs              # extracted from skills/executor.rs (validate_dispatch_readiness + check_grooming_markers + helpers)
```

Lib.rs becomes:
```rust
pub mod tool_execution;
// REMOVE: pub mod tools;
// REMOVE: pub mod mcp;
```

Or keep `tools` and `mcp` as flat-module re-exports via `pub use tool_execution::tools;` if we want shorter import paths for backwards-compat. Plan commits to NO compat layer — clean break, per #1447's "no deprecation alias" pattern (the architect confirmed: internal crate, no external consumers).

**Rationale for physical relocation** (vs slim re-export shell): parent #1259 AC3 says "pure module split; logic identical." A re-export shell is *aliasing*, not splitting. The architect-honest shape is physical relocation. Tools/ and mcp/ directories are exactly the §6 sub-concerns — they belong physically under tool_execution/.

LARGEST Wave 2 firing by file count (52+ files relocated + 2 fn extractions).

## Approach (committed)

### A. Create module skeleton

```bash
mkdir -p crates/mika-agent/src/tool_execution
```

### B. Directory renames via git mv (preserves history)

```bash
git mv crates/mika-agent/src/tools/ crates/mika-agent/src/tool_execution/tools/
git mv crates/mika-agent/src/mcp/ crates/mika-agent/src/tool_execution/mcp/
```

### C. Create tool_execution/mod.rs

```rust
//! tool dispatch, MCP integration, exec handlers, dispatch gates, loader-engine parity.
//!
//! Per Foundation §6: owns the per-tool dispatch loop invoked from
//! `crate::agent::run_agent`, the MCP server bridge, and the grooming-marker
//! dispatch gate. Tool *implementations* live in `tools/` (52 files); the
//! tool *dispatch* logic that selects and executes them lives in `dispatch.rs`.
//!
//! Skill *execution* mechanics (subprocess launch, long-running contexts)
//! remain in `crate::skills::executor` and are NOT part of this module —
//! they're skill mechanics, not tool dispatch. Only the dispatch *gate*
//! (`validate_dispatch_readiness`) and grooming-marker scanner relocate
//! here.

pub mod dispatch;
pub mod dispatch_gates;
pub mod mcp;
pub mod tools;

pub use dispatch::execute_tool;
pub use dispatch_gates::{validate_dispatch_readiness, check_grooming_markers};
pub use mcp::McpManager;
pub use tools::{Tool, ToolContext, ToolOutput, ToolRegistry, ImageData, SkillPathInfo, default_tools, team_tools, management_tools_if_needed};
```

### D. Extract execute_tool from agent.rs → tool_execution/dispatch.rs

Cut `async fn execute_tool` at agent.rs:3406 + its `ToolDispatchCtx` supporting struct + timeout/retry helpers to `tool_execution/dispatch.rs`. Estimated 150-300 LoC.

### E. Extract validate_dispatch_readiness from skills/executor.rs → tool_execution/dispatch_gates.rs

Cut `async fn validate_dispatch_readiness` (line 843) + `pub fn check_grooming_markers` (line 803) + their helpers. Estimated 200-400 LoC.

### F. Update lib.rs

```rust
pub mod tool_execution;
// REMOVE: pub mod tools;
// REMOVE: pub mod mcp;
```

### G. Update call sites (LARGEST sweep in Wave 2)

- **29 files** importing `crate::tools::*` — update to `crate::tool_execution::tools::*` OR `crate::tool_execution::*` via the re-exports above.
- **5 files** importing `crate::mcp::*` — update to `crate::tool_execution::mcp::*` OR `crate::tool_execution::*`.
- `agent.rs` — `execute_tool` call sites → `crate::tool_execution::execute_tool` (already in same crate after move).
- `skills/executor.rs` — internal calls to `validate_dispatch_readiness` from other fns in same file → `crate::tool_execution::dispatch_gates::validate_dispatch_readiness`.
- Cross-crate: `crates/mika-gateway/` — verify via grep (unlikely to import).

### H. Verify

- `cargo build -p mika-agent` clean (the FULL build, sub-step-by-sub-step)
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `grep -rn "crate::tools\b\|crate::mcp\b\|use crate::tools::\|use crate::mcp::" crates/ tests/` returns ZERO hits (across all file types)
- `grep -rn "use mika_agent::tools::\|use mika_agent::mcp::" crates/ tests/` returns ZERO hits

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/tool_execution/mod.rs` created with Foundation §6 doc-comment (parent AC4) + re-exports for the public surface (Tool, ToolRegistry, ToolOutput, McpManager, execute_tool, validate_dispatch_readiness, check_grooming_markers, default_tools, team_tools, management_tools_if_needed).

2. **AC2**: `crates/mika-agent/src/tool_execution/tools/` directory exists (via `git mv` from `tools/`) with all 52 sub-files and tools/mod.rs preserved. History preserved on each sub-file via git's rename detection.

3. **AC3**: `crates/mika-agent/src/tool_execution/mcp/` directory exists (via `git mv` from `mcp/`) with mcp/mod.rs and mcp/config.rs preserved. History preserved.

4. **AC4**: `crates/mika-agent/src/tool_execution/dispatch.rs` contains `execute_tool` fn + ToolDispatchCtx struct + helpers, relocated from agent.rs.

5. **AC5**: `crates/mika-agent/src/tool_execution/dispatch_gates.rs` contains `validate_dispatch_readiness` + `check_grooming_markers` + helpers, relocated from skills/executor.rs.

6. **AC6**: All call sites updated. `grep -rn "crate::tools\b\|crate::mcp\b\|use crate::tools::\|use crate::mcp::\|use mika_agent::tools::\|use mika_agent::mcp::" crates/ tests/` returns **ZERO hits across all file types** (Rust source, doc-comments, non-Rust). Per #1444's F1 discipline.

7. **AC7**: `crates/mika-agent/src/lib.rs` declares `pub mod tool_execution;` and removes `pub mod tools;` + `pub mod mcp;` (parent AC4).

8. **AC8**: `cargo test -p mika-agent` passes (parent AC2). Specific checkpoint: tool-related tests (`tools/*/tests/*` integration tests, mcp tests, dispatch tests) pass in their new home.

9. **AC9**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

10. **AC10**: No behavior change (parent AC3) — pure relocation, same Tool trait, same dispatch semantics, same MCP integration semantics, same dispatch-gate semantics.

11. **AC11**: History preservation — `git log --follow crates/mika-agent/src/tool_execution/tools/cancel_task.rs` (and similar spot-checks on 3+ sub-files) shows prior history via rename detection. Acceptable degradation: `tool_execution/dispatch.rs` (chunk-cut from agent.rs) and `tool_execution/dispatch_gates.rs` (chunk-cut from skills/executor.rs) may NOT have rename detection. Document as PR-body limitation.

## Out of scope

- `skills/executor.rs` (minus 2 extracted fns) — skill execution mechanics, future-grooming
- `skills/index.rs`, `manifest.rs`, `marketplace.rs`, `install.rs` — skill loading
- `agent.rs::run_agent()` and surrounding loop — agent_loop/ (#1452)
- Internal logic of any tool implementation (.rs file in tools/) — unaffected
- Refactoring the Tool trait or ToolRegistry — pure relocation only

## Risk

**LARGEST Wave 2 firing.** 52+ files relocated via git mv + 2 chunk-cuts + 29-file call-site sweep.

- **Massive call-site sweep**: 29 files import `crate::tools` + 5 import `crate::mcp`. Each needs `crate::tools` → `crate::tool_execution::tools` (or re-export form). Bounded by AC6 grep gate.
- **chunk-cuts from agent.rs (11,401 LoC) + skills/executor.rs (5,623 LoC)**: error-prone if cut boundaries are wrong. Mitigated by sub-step `cargo build` between extractions.
- **Tool trait is widely consumed**: 52 tool implementations + ToolRegistry's registration sites + agent.rs's dispatch site. Renaming the import path touches every consumer. Mitigated by `pub use` in tool_execution/mod.rs as escape-hatch if call-site churn is too disruptive (alternative: re-export form `crate::tool_execution::Tool` for all consumers).
- **Cross-crate impact**: `crates/mika-gateway/` likely imports `mika_agent::tools::*` or `mika_agent::mcp::*` — verify at implementation; possibly small cross-crate sweep needed.
- **History-preservation degradation**: chunk-cuts from large files don't get rename detection.

Risk profile: higher than #1444 evidence/ + #1448 task_state/ — this combines a 52-file directory rename WITH 2 chunk-cuts. Bounded by AC6 grep discipline + per-sub-step cargo build.

## Test plan

1. `cargo build -p mika-agent` clean — after each sub-step (B, D, E, G)
2. `cargo test -p mika-agent --lib` passes
3. `cargo build -p mika-gateway` clean (cross-crate sanity — `mika-gateway` may import tools or mcp)
4. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
5. `grep -rn "crate::tools\b\|crate::mcp\b\|use crate::tools::\|use crate::mcp::\|use mika_agent::tools::\|use mika_agent::mcp::" crates/ tests/` returns **zero hits across all file types** (AC6)
6. Spot-check git history preservation: `git log --follow crates/mika-agent/src/tool_execution/tools/cancel_task.rs` shows prior commits
7. Specifically run tool tests: `cargo test -p mika-agent --lib tool_execution`

## Implementation order

1. `mkdir -p crates/mika-agent/src/tool_execution`
2. `git mv crates/mika-agent/src/tools/ crates/mika-agent/src/tool_execution/tools/`
3. `git mv crates/mika-agent/src/mcp/ crates/mika-agent/src/tool_execution/mcp/`
4. Create `tool_execution/mod.rs` with re-exports
5. lib.rs: add `pub mod tool_execution;`, remove `pub mod tools;` + `pub mod mcp;`
6. `cargo build` — full crate sweep; expect many import errors; fix them via search-and-replace `crate::tools` → `crate::tool_execution::tools` and `crate::mcp` → `crate::tool_execution::mcp`
7. `cargo build` clean
8. Extract `execute_tool` + `ToolDispatchCtx` from agent.rs → tool_execution/dispatch.rs
9. `cargo build` clean
10. Extract `validate_dispatch_readiness` + `check_grooming_markers` from skills/executor.rs → tool_execution/dispatch_gates.rs
11. `cargo build` clean; `cargo test -p mika-agent`; `cargo clippy ...`
12. AC6 grep verification

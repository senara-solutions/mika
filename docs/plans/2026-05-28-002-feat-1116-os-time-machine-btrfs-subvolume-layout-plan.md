# Plan: feat(os): time-machine — btrfs subvolume layout + snapshot/restore/fork orchestration + setup tool

**Ticket:** mika issue#1116
**Date:** 2026-05-28
**Type:** feature
**Depends on:** #1115 (Mika OS scaffold — landed)

---

## Context

Mika OS embeds a btrfs subvolume layout under `~/.mika/` that Mika orchestrates for O(1) snapshots between agent sessions. Snapshots cover DB + skills + configs, excluding logs and old copy-backups via nested subvolumes. The mechanism supports three operations: rollback ("undo this session"), fork-to-new-tenant ("spin up a parallel tenant from the snapshot"), and remote backup (`btrfs send` stream on stdout).

The design decisions are settled from a prior brainstorm session and are load-bearing inputs to this plan (not options to re-evaluate):

1. `~/.mika/` is a subvolume; `logs/` and `data/_backups/` are nested subvolumes (excluded from parent snapshots)
2. Per-session-only trigger: snapshot at session start, before first state mutation
3. Both `--rollback` and `--fork` restore modes
4. Portable image — `mika snapshot enable` hard-requires btrfs with `mika-os-setup btrfs` as speed-bump
5. Fork-time secret redaction default-on with `--keep-secrets` opt-out
6. No default remote destination — `mika snapshot remote configure` is deployer action

## Implementation Units

### Unit 1: New `mika-os` crate — btrfs primitives and snapshot engine

**Files:**
- `crates/mika-os/Cargo.toml` (new crate)
- `crates/mika-os/src/lib.rs`
- `crates/mika-os/src/btrfs.rs` — low-level btrfs wrappers
- `crates/mika-os/src/snapshot.rs` — snapshot engine (create, list, delete, prune)
- `crates/mika-os/src/restore.rs` — rollback and fork logic
- `crates/mika-os/src/redact.rs` — secret redaction for fork
- `crates/mika-os/src/subvolume_layout.rs` — layout constants and validation
- `crates/mika-os/src/error.rs` — thiserror error types

**Rationale:** A separate crate isolates OS-level btrfs concerns from agent logic. The CLI and server import it. No framework dependency — just `std::process::Command` wrapping `btrfs` CLI calls (btrfs has no stable C library API worth FFI'ing; the CLI is the documented interface).

**Key types and functions:**

```rust
// btrfs.rs — thin wrappers around `btrfs` CLI
pub fn is_btrfs(path: &Path) -> Result<bool>;         // `stat -f` or `btrfs filesystem df`
pub fn create_subvolume(path: &Path) -> Result<()>;    // `btrfs subvolume create`
pub fn delete_subvolume(path: &Path) -> Result<()>;    // `btrfs subvolume delete`
pub fn snapshot_readonly(src: &Path, dst: &Path) -> Result<()>;  // `btrfs subvolume snapshot -r`
pub fn send_stream(snap: &Path, stdout: &mut impl Write) -> Result<()>;  // `btrfs send`
pub fn receive_stream(dest_dir: &Path, stdin: &mut impl Read) -> Result<()>;  // `btrfs receive`

// subvolume_layout.rs — layout constants
pub const MIKA_HOME_SUBVOL: &str = "";              // ~/.mika/ itself
pub const LOGS_NESTED_SUBVOL: &str = "logs";        // nested, excluded from parent snap
pub const BACKUPS_NESTED_SUBVOL: &str = "data/_backups";  // nested, excluded

pub fn validate_layout(home: &Path) -> Result<LayoutStatus>;
pub fn initialize_layout(home: &Path) -> Result<()>;  // create nested subvols if missing

// snapshot.rs — snapshot engine
pub struct SnapshotLabel {
    pub tenant_id: String,
    pub session_id: String,
    pub timestamp: String,  // ISO 8601
}

pub fn create_snapshot(home: &Path, label: &SnapshotLabel) -> Result<PathBuf>;
pub fn list_snapshots(home: &Path, tenant_filter: Option<&str>) -> Result<Vec<SnapshotInfo>>;
pub fn delete_snapshot(snap_path: &Path) -> Result<()>;
pub fn prune_snapshots(home: &Path, keep: usize, tenant_filter: Option<&str>) -> Result<PruneResult>;

// restore.rs
pub fn rollback(home: &Path, snap_path: &Path) -> Result<()>;
pub fn fork(home: &Path, snap_path: &Path, new_tenant: &str, keep_secrets: bool) -> Result<PathBuf>;

// redact.rs — fork-time secret redaction
pub fn redact_env_file(path: &Path) -> Result<()>;
pub fn redact_oauth_json(path: &Path) -> Result<()>;
pub fn redact_config_toml(path: &Path) -> Result<()>;
pub fn redact_db_secrets(db_path: &Path, agent_id: &str) -> Result<()>;
```

**Snapshot storage location:** `~/.mika/.snapshots/{label}/` — a sibling directory inside the `~/.mika/` subvolume parent. Each snapshot is a read-only btrfs subvolume created by `btrfs subvolume snapshot -r ~/.mika/ ~/.mika/.snapshots/{label}/`. The `.snapshots/` directory is created by `mika snapshot enable`.

**Prune strategy:** `prune_snapshots(keep: usize)` deletes oldest-first by timestamp parsed from label, keeping the `keep` most recent. Default `--keep 50` (configurable). No time-based retention in v1 — count-based only.

**Rollback mechanics:**
1. Stop mika-server and mika-gateway services (if running)
2. Create a safety snapshot of current state (labeled `pre-rollback-{timestamp}`)
3. `btrfs subvolume delete ~/.mika/logs/` and `~/.mika/data/_backups/` (nested subvols must die before parent)
4. `btrfs subvolume delete ~/.mika/` (the main subvolume)
5. `btrfs subvolume snapshot {snap} ~/.mika/` (restore as writable snapshot)
6. Re-create nested subvolumes `logs/` and `data/_backups/` (fresh, empty)
7. Restart services

**Fork mechanics:**
1. `btrfs subvolume snapshot {snap} ~/.mika-tenants/{new-tenant}/` (writable copy)
2. Re-create nested subvolumes in fork target
3. Run tenant-ingest (rename agent dirs, rewrite DB tenant references)
4. Unless `--keep-secrets`: run secret redaction on fork target
5. Print instructions for starting the forked tenant

**Tenant-ingest for fork:** The DB rewrite scans these tables for agent_id/tenant references:
- `agents` — rename agent row
- `sessions` — update agent_id
- `messages` — update via session FK
- `tasks` — update agent_id
- `tool_calls` — update agent_id
- `llm_calls` — update agent_id
- `skill_overrides` — update agent_id
- `core_memory` — update agent_id
- `facts` — update agent_id
- `search_content` — update agent_id
- `kg_*` tables — update agent_id columns
- `operational_items` — update agent_id

This list is derived from the current schema (v39). The implementation must include a compile-time or test-time assertion that enumerates all tables with an `agent_id` column and fails if a new one is added without updating the fork-ingest list.

**Secret redaction for fork:**
- `~/.mika/.env` — regex-match `*_API_KEY`, `*_TOKEN`, `*_SECRET` patterns; replace values with `REDACTED_BY_MIKA_FORK`; preserve key names and structure
- `~/.mika/oauth.json` — overwrite with `{}`
- `~/.mika/config.toml` — parse with `toml` crate; redact values for known secret keys (same patterns); preserve structure
- DB: `settings` table values matching secret patterns → redacted

### Unit 2: `mika snapshot` CLI subcommands

**Files:**
- `crates/mika-cli/src/commands/snapshot.rs` (new)
- `crates/mika-cli/src/commands/mod.rs` (add module)
- `crates/mika-cli/src/cli.rs` (add `Snapshot` variant to `Commands`)
- `crates/mika-cli/Cargo.toml` (add `mika-os` dependency)

**CLI surface:**

```
mika snapshot enable              # Activate time-machine (hard-error if not btrfs)
mika snapshot disable             # Deactivate (remove subvolume layout)
mika snapshot list [--tenant T]   # List local snapshots
mika snapshot create [--label L]  # Manual checkpoint
mika snapshot prune [--keep N]    # Retention (default 50)
mika snapshot restore --rollback <snap>   # Replace current state
mika snapshot restore --fork <snap> <new-tenant> [--keep-secrets]  # Parallel tenant
mika snapshot send <snap>         # stdout btrfs send-stream
mika snapshot remote configure <name> <endpoint>  # Store remote target
```

**clap structure:**

```rust
#[derive(Subcommand)]
pub enum SnapshotCommands {
    Enable,
    Disable,
    List {
        #[arg(long)]
        tenant: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    Create {
        #[arg(long)]
        label: Option<String>,
    },
    Prune {
        #[arg(long, default_value = "50")]
        keep: usize,
    },
    Restore(RestoreArgs),
    Send {
        snap: String,
    },
    Remote(RemoteArgs),
}

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
pub struct RestoreArgs {
    #[arg(long)]
    rollback: Option<String>,
    #[arg(long)]
    fork: Option<String>,
    /// New tenant name (required with --fork)
    #[arg(long, requires = "fork")]
    tenant_name: Option<String>,
    /// Preserve secrets in fork (default: redact)
    #[arg(long)]
    keep_secrets: bool,
}
```

**`enable` flow:**
1. Check `is_btrfs(home)` — if false, print error with `mika-os-setup btrfs` pointer and exit 1
2. Check if already enabled (`.snapshots/` exists and is a directory) — idempotent return
3. Create `.snapshots/` directory
4. Validate/create nested subvolumes for `logs/` and `data/_backups/` (only if they don't already exist as subvolumes)
5. Write `.snapshots/.enabled` marker file
6. Print success message

**`disable` flow:**
1. Check enabled state
2. Warn about existing snapshots (prompt for `--yes` confirmation if any exist)
3. Delete all snapshot subvolumes in `.snapshots/`
4. Remove `.snapshots/` directory and marker
5. Note: nested subvolumes (`logs/`, `data/_backups/`) are left in place — they're harmless and removing them would require a service stop + data migration

### Unit 3: `mika-os-setup btrfs` host-side provisioner

**Files:**
- `os/scripts/mika-os-setup` (new, shell script — not Rust)
- `os/README.md` (update with setup instructions)

**Rationale:** This is a host-side provisioner that runs outside the container or on the host OS. Shell script is appropriate — it wraps `mkfs.btrfs`, `mount`, and `btrfs subvolume create` calls. No Rust binary needed for what is essentially a one-time setup command.

**Subcommands:**

```bash
mika-os-setup btrfs --loopback [PATH]    # Create loopback btrfs (default: ~/.mika/pool.img)
mika-os-setup btrfs --device /dev/sdX    # Format dedicated partition
mika-os-setup btrfs --csi                # K8s: check for btrfs-CSI, print guidance
```

**`--loopback` flow:**
1. Check if `~/.mika/` already exists — back up contents if so
2. Create loopback file (`truncate -s 10G` default, `--size` override)
3. `mkfs.btrfs` the loopback file
4. Mount loopback at `~/.mika/` (add fstab entry with `loop` option)
5. Create main subvolume structure: `btrfs subvolume create ~/.mika/`
6. Create nested subvolumes: `logs/`, `data/_backups/`
7. Restore backed-up contents if any
8. `chown -R mika:mika ~/.mika/`
9. Print warning about loopback write amplification on non-btrfs host FS

**`--device` flow:**
1. Confirm device is unmounted and not in use (safety check)
2. `mkfs.btrfs /dev/sdX`
3. Mount at `~/.mika/` (add fstab entry)
4. Same subvolume creation as loopback
5. Print success

**`--csi` flow:**
1. Check `kubectl` availability
2. Probe for btrfs-CSI StorageClass (`kubectl get sc -o json | jq ...`)
3. If found: print configuration guidance for PVC spec
4. If not found: print installation guidance for btrfs-CSI driver

### Unit 4: Auto-snapshot trigger at session start

**Files:**
- `crates/mika-agent/src/agent.rs` (or wherever `run_agent` creates sessions) — add snapshot hook
- `crates/mika-agent/Cargo.toml` (add `mika-os` dependency)

**Mechanism:** Before the first state mutation in a new session, call `mika_os::snapshot::create_snapshot()` if time-machine is enabled. The check is:

```rust
// In session creation path, after session row INSERT but before agent loop
if mika_os::is_enabled(home) {
    let label = SnapshotLabel {
        tenant_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        timestamp: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };
    match mika_os::snapshot::create_snapshot(home, &label) {
        Ok(path) => tracing::info!(snapshot_path = %path.display(), "session snapshot created"),
        Err(e) => tracing::warn!(error = %e, "session snapshot failed — continuing without snapshot"),
    }
}
```

**Fail-open:** Snapshot failure does NOT block the session. The agent loop continues regardless. This is a safety net, not a gate.

**Performance:** `btrfs subvolume snapshot -r` is O(1) regardless of data size (CoW metadata clone). Typical latency: <100ms. No risk of blocking the agent loop.

**Where to hook:** The `run_agent()` function in `crates/mika-agent/src/agent.rs` is the canonical entry point. The snapshot should fire after session creation but before the first LLM call. Specifically, insert the snapshot call between the session-resolution block and the main agent loop.

**Enable check:** `mika_os::is_enabled(home)` checks for `~/.mika/.snapshots/.enabled` marker file. If the marker doesn't exist, the function returns `false` and the snapshot is skipped with zero overhead (one `Path::exists()` check).

### Unit 5: Dockerfile updates for btrfs support

**Files:**
- `os/Dockerfile` — add `btrfs-progs` to both stages
- `os/init/mika-os-init.sh` — add subvolume layout validation on boot

**Dockerfile changes:**
- In `mika-os` stage: `emerge sys-fs/btrfs-progs` (adds ~2MB)
- In `mika-runtime` stage: `emerge sys-fs/btrfs-progs` (runtime dep for snapshot operations)
- Copy the new `mika-os-setup` script: `COPY os/scripts/mika-os-setup /usr/local/bin/mika-os-setup`

**Init script changes:** Add an optional btrfs layout check after the existing config seeding:

```bash
# ── Validate btrfs subvolume layout if enabled ──
if [ -f "$MIKA_HOME/.snapshots/.enabled" ]; then
    mika snapshot enable 2>/dev/null || \
        echo "[mika-os-init] WARNING: snapshot layout validation failed"
fi
```

This is idempotent — `enable` is a no-op if layout is already correct.

### Unit 6: Documentation

**Files:**
- `docs/time-machine.md` (new — overview, CLI reference, two deployment postures)
- `os/README.md` (update — add setup tool and snapshot sections)

**`docs/time-machine.md` structure:**
1. Overview — what time-machine does and why
2. Quick start — `mika-os-setup btrfs --loopback` → `mika snapshot enable`
3. Subvolume layout diagram
4. CLI reference — all `mika snapshot` subcommands with examples
5. Deployment postures:
   - Self-host (loopback or dedicated device)
   - mika-cloud (btrfs-CSI + PVC)
6. Rollback workflow walkthrough
7. Fork workflow walkthrough
8. Remote backup patterns (SSH pipe, restic pipe examples)
9. Retention and pruning guidance
10. Troubleshooting

### Unit 7: Tests

**Files:**
- `crates/mika-os/src/btrfs.rs` — unit tests for CLI output parsing
- `crates/mika-os/src/snapshot.rs` — unit tests for label format, prune logic
- `crates/mika-os/src/redact.rs` — unit tests for secret pattern matching
- `crates/mika-os/src/restore.rs` — unit tests for tenant-ingest table enumeration
- `crates/mika-os/tests/` — integration tests (gated behind `#[ignore]` + `MIKA_TEST_BTRFS=1`, require btrfs-capable test environment)

**Test strategy:**
- **Unit tests (always run):** Test all pure logic — label parsing, prune selection, redaction patterns, CLI argument construction, table enumeration completeness assertion
- **Integration tests (gated):** Require actual btrfs filesystem. Create a loopback btrfs in a temp dir, exercise full snapshot→rollback and snapshot→fork cycles. Gated behind `MIKA_TEST_BTRFS=1` because CI runners may not have btrfs-progs or CAP_SYS_ADMIN

**Table enumeration compile-time safety:** A `#[test]` in `restore.rs` that queries `sqlite_master` for all tables containing an `agent_id` column and asserts the set matches the hardcoded fork-ingest list. This catches schema drift — adding a new table with `agent_id` without updating the fork list fails CI.

## Sequencing

1. **Unit 1** (mika-os crate) — foundation, no dependencies on other units
2. **Unit 3** (mika-os-setup script) — can be done in parallel with Unit 1, no Rust dependency
3. **Unit 2** (CLI subcommands) — depends on Unit 1
4. **Unit 4** (auto-snapshot hook) — depends on Unit 1
5. **Unit 5** (Dockerfile) — depends on Unit 1 and Unit 3
6. **Unit 7** (tests) — depends on Units 1-4
7. **Unit 6** (docs) — last, after implementation stabilizes

Units 1+3 can be developed in parallel. Units 2+4 can be developed in parallel after Unit 1 lands.

## Risk mitigations

| Risk | Mitigation |
|------|------------|
| DB row-id rewrite misses a table | Compile-time test assertion against `sqlite_master` (Unit 7) |
| Loopback performance | Warning message in `mika-os-setup btrfs --loopback` output |
| Snapshot retention default wrong | Start at 50, document how to change, observe in practice |
| New secret column not redacted | Redaction list maintained alongside schema; same test assertion pattern |
| btrfs-progs missing at runtime | `is_btrfs()` check returns false → graceful degradation |
| Fork on running tenant | Service-stop guard in rollback; fork operates on snapshot (immutable) so no conflict |

## Out of scope

- Snapshot replay (deterministic re-execution)
- Multi-tenant snapshot bundles
- Remote snapshot listing
- Encryption at rest / in transit (delegated to LUKS/SSH)
- Telemetry for snapshot operations
- Mid-session or end-of-session snapshots

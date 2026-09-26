# Plan — feat(os): mika-runtime per-role build targets (mika#1247)

## Phase 0 — Pin

**A. Current `mika/os/Dockerfile`** (234 lines, 2 stages):
- Stage 1: `mika-os` (line 26) — full handbook-style Gentoo build, builds all 3 binaries + tooling
- Stage 2: `mika-runtime` (line 184) — slim production runtime, COPYs ALL 3 binaries:
  ```dockerfile
  COPY --from=mika-os /usr/local/bin/mika /usr/local/bin/mika
  COPY --from=mika-os /usr/local/bin/mika-spirit /usr/local/bin/mika-spirit
  COPY --from=mika-os /usr/local/bin/mika-gateway /usr/local/bin/mika-gateway
  ```
- Plus OpenRC init scripts (both `mika-spirit` and `mika-gateway`)
- Single ENTRYPOINT: `/usr/local/bin/mika-os-init.sh`

**B. Three binaries from Cargo workspace:**
- `mika-spirit` (from `crates/mika-agent/src/bin/mika-spirit.rs`)
- `mika-gateway` (from `crates/mika-gateway/src/main.rs`)
- `mika` (from `crates/mika-cli/src/main.rs`)

**C. OpenRC services at `mika/os/openrc/`:**
- `init.d/mika-spirit` + `conf.d/mika-spirit`
- `init.d/mika-gateway` + `conf.d/mika-gateway`

**D. Shared runtime dependencies** (all 3 binaries need):
- gentoo/stage3:20250505 base
- `gh`, `gws`, `ollama` binaries (tooling)
- `/etc/mika/` configs
- `/app/docs/` documentation (init script needs)
- `mika` user (per OpenRC conventions)

**E. `mika-os-init.sh` (F1 — pinned)** — read full at `mika/os/init/mika-os-init.sh`. Behavior summary:
- Boots OpenRC via `rc default` (service-agnostic — boots whatever's in /etc/init.d/)
- SIGTERM handler: `rc-service mika-gateway stop 2>/dev/null || true; rc-service mika-spirit stop 2>/dev/null || true` (BOTH calls fail-soft via `|| true` — graceful when the service isn't installed)
- Tails BOTH log files: `tail -F "$LOG_DIR/mika-spirit.log" "$LOG_DIR/mika-gateway.log"`. Prior `touch` creates empty log files if missing.
- No hardcoded require-binary references. Compatible with all per-role targets (server-only, gateway-only, all).

**Minor inefficiency on per-role targets**: tail-F always tails both log files even if one daemon isn't running. Empty-file tail is harmless (no data flow, no error). Acceptable for v1 — can be tightened in a follow-up if log noise becomes an issue.

**`mika-runtime-cli` exception**: init.sh is designed for daemon-mode (OpenRC + tail). CLI is interactive (TUI). The cli target MUST set `ENTRYPOINT ["/usr/local/bin/mika"]` directly, NOT mika-os-init.sh. Plan section "## Restructure" Stage 5 already specifies this.

**F. Ticket ACs (verbatim from #1247):**
- AC1: Dockerfile declares 5 named targets: `mika-os`, `mika-runtime-server`, `mika-runtime-gateway`, `mika-runtime-cli`, `mika-runtime-all`. Legacy `mika-runtime` removed or aliased.
- AC2: each target builds clean via `docker build --target <target> -f mika/os/Dockerfile .`
- AC3: each target's image contains ONLY appropriate binaries (server: only mika-spirit; gateway: only mika-gateway; cli: only mika; all: all three)
- AC4: OpenRC init scripts per role (server: mika-spirit only; gateway: mika-gateway only; cli: none; all: both)
- AC5: `mika/os/README.md` updated with per-role table
- AC6: plan addresses shared-base-stage trade-off (DRY vs clarity)

## AC6 — Shared-base-stage trade-off (load-bearing decision)

Two design alternatives:

### Option A — Shared `mika-runtime-base` stage + per-role thin stages

```dockerfile
# Stage 2a: shared runtime base
FROM gentoo/stage3:20250505 AS mika-runtime-base
# system deps, mika user, /etc/mika/ configs, /app/docs/, init.sh
COPY --from=mika-os /etc/mika/ /etc/mika/
COPY --from=mika-os /app/docs/ /app/docs/
COPY --from=mika-os /usr/local/bin/mika-os-init.sh /usr/local/bin/mika-os-init.sh
# (no role-specific binaries or init.d in base)

# Stage 2b-server
FROM mika-runtime-base AS mika-runtime-server
COPY --from=mika-os /usr/local/bin/mika-spirit /usr/local/bin/mika-spirit
COPY --from=mika-os /etc/init.d/mika-spirit /etc/init.d/mika-spirit
COPY --from=mika-os /etc/conf.d/mika-spirit /etc/conf.d/mika-spirit
ENTRYPOINT [...]

# Similar for gateway, cli, all
```

**Pro:** DRY — shared base layer reused across 4 per-role targets. Docker BuildKit caches the base layer once.
**Con:** Slightly less clarity — reader has to follow chain `mika-runtime-server` → `mika-runtime-base` → `gentoo/stage3` to understand the full image. Trade-off felt in maintenance.

### Option B — Independent per-role stages (no shared base)

Each `mika-runtime-*` stage independently `FROM gentoo/stage3:20250505 AS ...` and COPYs everything it needs from `mika-os`.

**Pro:** Maximum clarity — each role's full chain is visible top-to-bottom in one stage block. No "what does base have?" hop. Independent failure modes per target.
**Con:** Repetition — system deps, mika user setup, /etc/mika/ configs duplicated 4× in the Dockerfile. Maintenance burden when changing shared config.

### Decision: Option A (shared base)

**Committed.** Reasoning:
- The shared base (~50 lines of system + config setup) is repetitive enough that 4× duplication invites drift (one stage forgets a config update, others diverge). DRY here protects substrate integrity.
- The clarity cost is bounded — `mika-runtime-base` is a single intermediate stage, not a deep chain. Readers follow ONE hop to see the shared substrate.
- BuildKit's layer caching benefits from the shared base (one cached base layer reused across 4 targets vs. 4 redundant cached bases). Build-time + registry size impact is real if not large.
- Existing convention in the repo: `mika-os` itself is a 150-line stage with many phases; readers already follow chains. Adding one more shared-base hop is consistent with the existing style.

## Approach (committed)

### Restructure into 6 stages

1. **`mika-os`** (unchanged) — full Gentoo build, produces all 3 binaries + tooling. Line ~26-180.

2. **`mika-runtime-base`** (NEW) — slim Gentoo runtime + shared system + /etc/mika/ + /app/docs/ + mika-os-init.sh. NO role-specific binaries. NO role-specific init scripts.

3. **`mika-runtime-server`** (NEW, `FROM mika-runtime-base`) — adds `mika-spirit` binary + `init.d/mika-spirit` + `conf.d/mika-spirit`. ENTRYPOINT mika-os-init.sh.

4. **`mika-runtime-gateway`** (NEW, `FROM mika-runtime-base`) — adds `mika-gateway` binary + `init.d/mika-gateway` + `conf.d/mika-gateway`. ENTRYPOINT mika-os-init.sh.

5. **`mika-runtime-cli`** (NEW, `FROM mika-runtime-base`) — adds `mika` binary only. NO OpenRC services (TUI is interactive). ENTRYPOINT `/usr/local/bin/mika` (direct TUI launch).

6. **`mika-runtime-all`** (NEW, `FROM mika-runtime-base`) — adds all 3 binaries + both init scripts. ENTRYPOINT mika-os-init.sh.

### Legacy `mika-runtime` target

Removed cleanly. The single existing consumer in mika-cloud Helm chart will be cut over in a follow-up ticket (per Out-of-Scope). For backwards-compat during transition, the legacy `mika-runtime` target can be left as an alias: `FROM mika-runtime-all AS mika-runtime`. **Picking remove rather than alias** — cleaner; cutover ticket can update Helm chart at the same PR.

### Tooling binaries (gh, gws, ollama)

Currently COPY'd into mika-runtime. Need to map per-role:
- `gh` — needed by mika-spirit (for GitHub webhook gateway responses). Include in mika-runtime-server + mika-runtime-all. Not gateway, not cli.
- `gws` — niche tool; include in mika-runtime-all only. Not needed for server/gateway/cli alone.
- `ollama` — needed by mika-spirit (LLM inference). Include in mika-runtime-server + mika-runtime-all. Not gateway, not cli.

This is the right verification target for AC3 — image contents per role.

## Acceptance Criteria (re-stated concrete)

1. **AC1:** `mika/os/Dockerfile` declares 6 named targets (5 per the ticket + 1 base): `mika-os`, `mika-runtime-base`, `mika-runtime-server`, `mika-runtime-gateway`, `mika-runtime-cli`, `mika-runtime-all`. Legacy `mika-runtime` removed.

2. **AC2:** Each `mika-runtime-*` target builds clean:
   - `docker build --target mika-runtime-server -f mika/os/Dockerfile .`
   - `docker build --target mika-runtime-gateway -f mika/os/Dockerfile .`
   - `docker build --target mika-runtime-cli -f mika/os/Dockerfile .`
   - `docker build --target mika-runtime-all -f mika/os/Dockerfile .`

3. **AC3:** Each image contains ONLY appropriate binaries (verified via `docker run --rm <image> ls -la /usr/local/bin/`):
   - `mika-runtime-server`: mika-spirit + gh + ollama present; mika + mika-gateway + gws absent
   - `mika-runtime-gateway`: mika-gateway present; mika + mika-spirit + gh + gws + ollama absent
   - `mika-runtime-cli`: mika present; mika-spirit + mika-gateway + gh + gws + ollama absent
   - `mika-runtime-all`: mika + mika-spirit + mika-gateway + gh + gws + ollama all present

4. **AC4:** OpenRC init scripts present per role (verified via `docker run --rm <image> ls /etc/init.d/`):
   - `mika-runtime-server`: only `mika-spirit`
   - `mika-runtime-gateway`: only `mika-gateway`
   - `mika-runtime-cli`: none
   - `mika-runtime-all`: both

5. **AC5:** `mika/os/README.md` updated with table:
   ```markdown
   | Target | Binary set | Audience |
   |--------|------------|----------|
   | mika-os | Full build env | Forkable reference, dev environment |
   | mika-runtime-base | Shared runtime base (no role-specific binaries) | Internal — not pushed standalone |
   | mika-runtime-server | mika-spirit | mika-cloud per-customer agent container |
   | mika-runtime-gateway | mika-gateway | mika-cloud gateway deployment |
   | mika-runtime-cli | mika (TUI) | Operator desktop install via Docker |
   | mika-runtime-all | All three binaries | Single-box self-host |
   ```

6. **AC6:** Shared-base trade-off documented (above) — Option A chosen.

7. **AC7 (added):** CI pipeline-artifacts pass with `Pipeline-Exempt: docs-only` or `code-only` trailer as appropriate. Verify via local `bash scripts/verify-pipeline.sh main` before pushing.

## Files to change

- `mika/os/Dockerfile` — restructure: 1 stage → 6 stages
- `mika/os/README.md` — append per-role table + audience notes

Plan + Dockerfile + README. 3 files.

## Out of scope (per ticket)

- mika-cloud Helm chart cutover (separate ticket, depends on this)
- Registry push + tagging strategy (separate ticket)
- Per-role runtime calibration tests (separate ticket)

## Risk

Low-medium.
- **Docker BuildKit cache invalidation** if base-layer changes propagate inconsistently. Mitigated by Option A's shared-base — single source of truth.
- **mika-os-init.sh runtime behavior under partial binary sets** — RESOLVED via Phase 0 Pin E (above): script is init.d-presence-driven via `rc default`, service-stop calls are `|| true` fail-soft, tail-F creates empty log files if missing. Compatible with all per-role targets. `mika-runtime-cli` uses direct ENTRYPOINT to skip OpenRC entirely (TUI is interactive).
- **Tooling-binary mapping wrong** (gh/gws/ollama per role). Mitigated by AC3 verification — manual inspection after each build.

## Test plan

1. `docker build --target mika-runtime-base -f mika/os/Dockerfile .` succeeds
2. `docker build --target mika-runtime-server -f mika/os/Dockerfile .` succeeds; verify AC3 + AC4 manually
3. Same for gateway, cli, all
4. `docker build --target mika-os -f mika/os/Dockerfile .` succeeds (regression)
5. README sanity-check: per-role table renders + audience descriptions match ACs

## Implementation order

1. Restructure Dockerfile: extract shared base from `mika-runtime`, add 4 per-role stages.
2. Run each build target locally; verify binaries + init scripts present per role.
3. Update README with per-role table.
4. Verify legacy `mika-runtime` references in mika-cloud Helm chart (foundation context — they exist; cutover is separate ticket per Out-of-Scope).
5. Run `bash scripts/verify-pipeline.sh main` for CI sanity.

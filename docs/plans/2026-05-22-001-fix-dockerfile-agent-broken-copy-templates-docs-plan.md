# Plan: fix(dockerfile-agent) — remove broken root-level COPY templates/ and COPY docs/ (#1237)

**Issue:** mika#1237
**Type:** fix (build infrastructure)
**Branch:** `fix/1237/dockerfile-agent-broken-copy-templates`

## Problem

`Dockerfile.agent` lines 21–22 (relative to the builder stage) contain two `COPY` directives whose source paths do not exist in the build context. The build fails at the BuildKit "compute cache key" step before reaching `cargo build`, so the per-customer `mika-agent` image has not been buildable from a clean checkout (regression latent at least since the dashboard reorg / `docs/` dockerignore exclusion):

```
ERROR: failed to compute cache key: failed to calculate checksum of ref ... "/templates": not found
ERROR: failed to compute cache key: failed to calculate checksum of ref ... "/docs": not found
```

`templates/` does not exist at the repo root (the only `templates/` directory in the tree is `crates/mika-agent/templates/`, copied implicitly by the earlier `COPY crates/mika-agent/ crates/mika-agent/`). `docs/` does exist at the repo root, but the project-level `.dockerignore` excludes it (`docs/` line), so the build context never sees it.

This blocks **first cloud deploy (Mika Prime)**: the `mika-agent` image is the per-customer artifact deployed by `mika-cloud` Helm charts.

## Acceptance Criteria Tie-backs

- **AC1:** `Dockerfile.agent` no longer contains the lines `COPY templates/ templates/` and `COPY docs/ docs/`.
- **AC2:** `docker build -f Dockerfile.agent -t mika-agent:test .` succeeds on a clean checkout of `main` (with this fix applied).
- **AC3:** The resulting image still embeds the templates and docs that `include_str!` references at compile time — i.e., `/usr/local/bin/mika-server --help` (or any path that touches the docs/templates handlers) works at runtime.

## Design Decisions

### D1: Remove the two broken `COPY` lines, full stop

The lines are dead code, not load-bearing. Three independent reasons:

1. **`COPY templates/ templates/`** — no `templates/` directory exists at the repo root. The two `include_str!` call sites that reference `templates/` resolve to `crates/mika-agent/templates/`, not the root:
   - `crates/mika-agent/src/bundled_skills.rs:1615` — `include_str!("../templates/skills/shell-exec/system_prompt.md")` → `crates/mika-agent/templates/skills/shell-exec/system_prompt.md` (relative to the `.rs` file, climbing one directory out of `src/`).
   - `crates/mika-agent/src/skills/executor.rs:2550` — `include_str!("../../templates/skills/shell-exec/handlers/run.sh")` → same crate-local `templates/`.
   Both files are already pulled in by the earlier `COPY crates/mika-agent/ crates/mika-agent/` directive on Dockerfile.agent line 20.
2. **`COPY docs/ docs/`** — the root-level `docs/` is listed in `.dockerignore` (top-level pattern `docs/`, anchored to context root per `.dockerignore`'s non-recursive matching for plain entries). Even if the line worked, it would copy nothing. The `include_str!` call sites that reference `docs/` either:
   - Use a relative path that resolves inside the crate (`crates/mika-agent/src/server/openapi.rs:100` → `crates/mika-agent/docs/openapi/mika-server.yaml`, copied by the `COPY crates/mika-agent/` line), or
   - Use `concat!(env!("OUT_DIR"), "/docs/…")` (10 sites in `crates/mika-agent/src/skills/builtin_handlers.rs`). These come from `crates/mika-agent/build.rs`, which already has a two-tier source resolution:
     ```rust
     let source = if workspace_docs.join(DOCS[0]).exists() {
         &workspace_docs   // ../../docs from manifest dir (workspace root)
     } else {
         &crate_docs       // ./docs inside the crate
     };
     ```
     Inside the docker build context the workspace-root `docs/` is dockerignored, so `build.rs` deterministically falls back to `crates/mika-agent/docs/`. That fallback is the supported docker/crates.io path.
3. **Compile-time embedding** — every templates/docs reference is `include_str!`, so the files only need to exist at the path the compiler resolves. Once compiled, the runtime image needs no source-`templates/` or source-`docs/` directory. The only relevant question is whether the builder stage sees them; (1) and (2) prove it does.

### D2: Do NOT touch `.dockerignore`

The root `docs/` dockerignore exclusion is correct: workspace-level docs (brainstorms, plans, ADRs, handsoff logs, etc.) should not bloat the build context. The `*.md` rule is anchored at root and does not strip the markdown files under `crates/mika-agent/docs/` or `crates/mika-agent/templates/skills/*/system_prompt.md`. `build.rs` explicitly handles the dockerignored case with its crate-local fallback.

### D3: Verification is a clean `docker build` against this branch

The fix's only behavioral contract is "the build succeeds." A clean `docker build` (without `--cache-from`) exercises both the BuildKit checksum stage (catches the COPY regression) and the cargo build stage (catches the `build.rs` fallback, the `include_str!` resolutions, and any second-order surprise). No runtime contract is changed; no new tests are warranted at the Rust layer.

## Implementation Steps

### Phase 1: Apply the fix

**File:** `Dockerfile.agent`

**Step 1.** Delete lines reading `COPY templates/ templates/` and `COPY docs/ docs/` (the two adjacent lines that sit between the `mika-gateway` placeholder block and the `COPY --from=dashboard-builder ...` directive). The resulting builder-stage prelude becomes:

```dockerfile
COPY Cargo.toml Cargo.lock ./
COPY crates/mika-common/ crates/mika-common/
COPY crates/mika-agent/ crates/mika-agent/
COPY crates/mika-gateway/Cargo.toml crates/mika-gateway/Cargo.toml
RUN mkdir -p crates/mika-gateway/src && echo "fn main() {}" > crates/mika-gateway/src/main.rs \
    && mkdir -p crates/mika-gateway/migrations && touch crates/mika-gateway/migrations/.keep
COPY --from=dashboard-builder /app/dashboard/dist dashboard/dist
```

### Phase 2: Verify locally

**Step 2.** From a clean checkout of this branch:

```bash
DOCKER_BUILDKIT=1 docker build --no-cache --progress=plain -f Dockerfile.agent -t mika-agent:test .
```

Expected: build succeeds end-to-end. Cargo compiles `mika-server`, `crates/mika-agent/build.rs` falls back to crate-local docs, runtime image gets stamped.

**Step 3.** Smoke the binary inside the image:

```bash
docker run --rm mika-agent:test mika-server --help
```

Expected: prints help output (exit 0). This indirectly confirms that the embedded docs/templates loaded successfully — any missing `include_str!` would have errored at compile time, not at this runtime invocation, so a clean build implies clean embedding.

### Phase 3: PR + cloud-deploy unblock

**Step 4.** Open the PR; CI runs the `docker-build-agent` job (if wired) or at minimum the standard cargo workspace check.

**Step 5.** After merge, mika-cloud cloud-deploy day-1 readiness audit (docs/logs/2026-05-21) can resume: image build is no longer a blocker.

## Risks and Mitigations

- **R1 (low):** `.dockerignore` matching semantics could differ from the assumption that `docs/` and `*.md` are root-anchored. **Mitigation:** D1 evidence — the build.rs fallback explicitly relies on this semantic and currently works for `cargo package --verify` and the crates.io tarball path; the verification step in Phase 2 would catch any surprise here.
- **R2 (low):** Future re-introduction of a root-level `templates/` or `docs/` directory could be silently dockerignored. **Mitigation:** out of scope — track separately if it happens. No precedent in the current tree.
- **R3 (none):** No runtime behavior changes — the failing COPY lines were dead code, not feature-gated copies.

## Out of Scope

- Adding a `.dockerignore` review pass for other top-level patterns.
- Adding a CI job that builds the docker image (separate ticket — `mika-cloud` already covers deploy-side validation; agent-side CI for Dockerfile changes is a separate scope).
- Touching `Dockerfile.gateway` (different image, different surface, no reported failure).
- Touching `build.rs` or any `include_str!` site (the existing fallback is already correct).

## Definition of Done

1. The two `COPY` lines are removed from `Dockerfile.agent`.
2. `DOCKER_BUILDKIT=1 docker build --no-cache -f Dockerfile.agent -t mika-agent:test .` succeeds locally on this branch.
3. `docker run --rm mika-agent:test mika-server --help` exits 0.
4. PR open; CI green; merged; cloud-deploy readiness audit unblocked.

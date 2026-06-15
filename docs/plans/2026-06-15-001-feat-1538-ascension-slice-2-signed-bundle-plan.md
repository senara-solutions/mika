---
title: "feat: Mika ascension architecture — slice 2 (signed-bundle producer R2 + gateway transfer R5)"
status: active
created: 2026-06-15
type: feat
issue: senara-solutions/mika#1538
origin: docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md
predecessor: docs/plans/2026-06-09-003-feat-ascension-architecture-first-slice-cli-plan.md
---

# feat: Mika ascension architecture — slice 2 (signed-bundle producer R2 + gateway transfer R5)

## Summary

Ship the **signed-bundle transfer pipeline** from the ascension architecture brainstorm: `mika export` produces a signed, sliced, content-addressed tarball of one agent's transferable state; `mika import` applies it (local round-trip first); the gateway exposes an authenticated endpoint that accepts a signed bundle and routes it to the target agent pod (push), plus a pull path (cloud → local snapshot).

Slice 2 carries the requirements the 2026-06-15 canvass (PR #1547) settled as belonging here: **R2** (export/import), **R3** (granular slice selection), **R6** (chain-forward-compat manifest shape), **R8** (manifest legibility), **R9** (CLI granular ops), **R10** (security baseline), and **R5** (gateway transfer endpoint). **R4** (family bootstrap) and **R7** (per-instance keys) are deferred to slice 3 (the "family identity" slice) per that same canvass.

## Problem frame

The brainstorm (`docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md`) scopes ten requirements. Slice 1 (PR #1468) shipped **R1 only** — the CLI dual-mode connection — and explicitly re-deferred R5 to slice 2 because a transfer endpoint with no bundle-producing clients is a surface with no callers. Slice 2 closes that loop: it builds the producer (R2) and the endpoint (R5) together so they land coherently.

The codebase has **no existing export/import/backup/snapshot/bundle code** (confirmed by repo-wide grep). This is greenfield within an existing data model. Every transferable slice already has a well-defined home — SQLite tables under `~/.mika/data/mika.db` (all `agent_id`-scoped) and per-agent files under `~/.mika/agents/<name>/`. The work is to read those surfaces into a portable, signed, content-addressed artifact and apply it back.

## Scope decision (READ FIRST — this is the load-bearing grooming decision)

**Slice 2 as scoped is milestone-sized: eight implementation units (U1, U2a, U2b, U3, U4, U5, U6, U7 per the canvass), of which U2b (indexed extractors), U5 (gateway), and U7 (security) are each substantial.** Shipping all of them as a single PR would produce an unreviewable mega-diff that violates the project's slice discipline (the same discipline that narrowed slice 1 from R1+R5 to R1-only).

**Unit numbering follows the 2026-06-15 canvass** (samidarko, architect session `5da94962-…`), which ratified the effective slice-2 unit set as **U1, U2a, U2b, U3, U4, U5, U6, U7** — U2 split into **U2a** (leaf slices: soul, skills, mcp, tasks — file/row copies) and **U2b** (indexed slices: memory FTS5+vec, kg entity/relationship scoping, conversations with pagination). R4/R7 (the old U4-family-bootstrap + U7-keypair from the body hypothesis) are dropped to slice 3.

This plan proposes slice 2 ships as **two sequenced sub-slices**, each a single dispatchable PR on its own branch, sequenced by hard dependency:

- **Sub-slice 2a — local signed-bundle round-trip** (`feat/1538/ascension-slice-2a-local-roundtrip`). Units U1, U7, U2a, U2b, U3, U4, U6 (canvass co-design order U1→U7→U2→U3→U4→U6, refined below). Delivers: `mika export --agent <name> [--include …] --sign` → `mika import <bundle>` works locally end-to-end; `mika bundle list/verify` and `mika agent slices` work; signatures verify; tampered/unsigned/truncated bundles reject. **No network.** Self-contained and fully testable without cloud infrastructure.
- **Sub-slice 2b — gateway transfer endpoint** (`feat/1538/ascension-slice-2b-gateway-transfer`). Unit U5. Delivers: gateway `POST /a2a/{customer_id}/{agent_name}/bundle` (push, signature-verified before forward) + pull path + pod-side `/import-bundle` receiver. **Depends on 2a** (it forwards bundles 2a produces to a pod-side applier 2a builds) **AND on `mika-cloud#111`** — the canvass recorded U5 as a *code* dependency on mika-cloud#111 (pod-routing), not merely a deployment dep. Slice 2b + mika-cloud#111 land together for the thin-client thread to be usable (see U5 + Risks).

**Sequencing reconciliation (canvass vs. this plan).** The canvass recommended a single linear order U1 → U7 → U2 → U3 → **U5 → U4** → U6. This plan refines that ordering by cutting U5 into sub-slice 2b *after* U4, because U5's gateway forwards to a pod-side applier that **is** U4's apply logic exposed over HTTP — U4 must exist before U5 has anything to forward to (the same dependency-inversion that re-deferred R5 out of slice 1). The first-pass architect blessed the 2a/2b cut (KTD1 "Sound"). Net: U1→U7→U2a→U2b→U3→U4→U6 in 2a; U5 in 2b. Co-design of U1+U7 (security baseline informs manifest schema) is preserved.

**This branch (`feat/1538/ascension-architecture-slice-2-signed`) carries this PLAN only.** The plan defines both sub-slices; dispatch of the implementation happens against the two sub-slice branches above. The grooming-time question for the architect + operator: **(A)** convert #1538 into a milestone with 2a and 2b as sub-issues, or **(B)** keep #1538 as the tracking ticket and dispatch 2a then 2b as sequenced PRs that both close it. See KTD1. This plan is written so either resolution works — the unit breakdown and sequencing are identical.

### Deferred (per 2026-06-15 canvass, PR #1547)

- **R4** (family-bootstrap as degenerate case) → slice 3.
- **R7** (per-instance key model) → slice 3. R4 and R7 form a coherent "family identity" slice; R4-without-R7 ships a read-only clone rather than a family member.
- Phase 2+ items (post-mission): real chain-resolution semantics, continuous bidirectional sync, peer-to-peer family Mikas, encrypted-at-rest SQLite, soul-version history. See brainstorm § Scope Boundaries.

## Requirements (carried from origin)

- **R2.** `mika export --agent <name> [--include <slice>,...] --sign` produces a tarball; `mika import [--remote <endpoint>] <bundle>` accepts one. Bundle MUST contain `manifest.toml` declaring slice list with content hashes, operator signature, identity claim (DID-shaped per R6), bundle creation time, and source-agent identity. Signature MUST be verifiable against the operator's GitHub App key.
- **R3.** Transferable slice set: `soul`, `memory`, `kg`, `conversations`, `skills`, `mcp`, `tasks`. Non-transferable: engine-internal §6 modules. `--include all` = the transferable set, not every module. Combinations compose.
- **R5.** Gateway exposes an authenticated endpoint accepting signed bundles, verifies signature before forwarding, routes to target agent pod. Supports pull (cloud → local snapshot) in addition to push.
- **R6.** Manifest identity MUST be DID-shaped (`did:<method>:<identifier>`) even with centralized resolution in the mission window. Slice references MUST be content-addressed (hashes, not paths/IDs). Format MUST be backward-compatible with future chain migration — adding chain-resolution MUST NOT require re-issuing bundles.
- **R8.** `manifest.toml` MUST be human-readable TOML; section names use the §6 module vocabulary; a human reads it like a packing list, an agent reads it mechanically as a verification target.
- **R9.** `mika bundle list <bundle>` prints the manifest without unpacking; `mika bundle verify <bundle>` checks signature + hashes without importing; `mika agent slices <name>` prints which slices have content for an agent without exporting.
- **R10.** All transfers over TLS (gateway's existing ingress cert). Unsigned bundles rejected. Signature verification happens gateway-side before pod-side application. Secrets in bundles wrapped with operator-key encryption; pod-side application uses the receiving pod's secret material (K8s secrets), not the bundle.

### Signing-key locus — option (c), operator-bundle-shipped (ratified 2026-06-15 canvass)

The canvass (samidarko, 2026-06-15, architect session `5da94962-…`) ratified **option (c)**: the bundle-signing key is operator-generated at provisioning, the cryptographic root **chains to the operator's existing GitHub App key** (`Settings.github_app_private_key`), and the receiving pod obtains trust in the bundle key via material shipped in the bundle. Rejected: (a) gateway-issued + Postgres-held and (b) pod K8s-secret — both are per-instance-key shapes that belong in R7/slice 3.

**Caveat the manifest schema MUST encode (canvass requirement):** this is a *slice-2 transitional pattern, not load-bearing-forever*. Slice 3 (R7) introduces per-instance keys and narrows operator-custody. The manifest schema names this transitional shape (a `key_ref`/`key_model` field) so future readers understand it as a slice-2 substrate choice, not a permanent anchor. Operator key-custody discipline matters in slice 2 — **lost operator key = un-bootstrappable agents until R7 lands**.

The exact cryptographic construction of "chains to the operator's GitHub App key" is specified in KTD3 and is the **primary pass-2 confirmation item** — the canvass phrasing ("encrypted with the operator's GitHub App key") admits two constructions (signature-attestation chain vs. literal RSA key-wrapping); the plan commits to the attestation chain and asks the architect to confirm.

## Key Technical Decisions

### KTD1. Slice 2 ships as two sequenced sub-slices (2a local, 2b gateway)

Rationale in § Scope decision. The dependency is hard: 2b's gateway endpoint forwards a bundle to a pod-side `/import-bundle` receiver, and that receiver IS 2a's `mika import` apply logic exposed over HTTP. Building 2b before 2a's applier exists would be a surface with no implementation behind it — the exact dependency-inversion that re-deferred R5 out of slice 1. **Open for architect/operator arbitration:** milestone-with-sub-issues vs. two-sequenced-PRs-closing-#1538 (see § Scope decision A/B). Recommendation: **B (two sequenced PRs)** — the units are tightly coupled by a shared manifest/apply contract, so a single tracking ticket keeps the contract review in one place; a milestone adds coordination overhead without separable ownership.

### KTD2. Reuse existing crypto deps — no new dependencies

Signing: reuse `jsonwebtoken` v9.3.1 RS256 + the GitHub App RSA key already parsed at `crates/mika-common/src/github_app.rs:95` (`EncodingKey::from_rsa_pem`). Hashing (content-addressing, R6): reuse `sha2` v0.10.9 SHA-256 — the same function the KG uses for `docs_root_hash`/`source_doc_hash` (`crates/mika-agent/src/kg/config.rs`). Encoding: reuse `base64` v0.22.1. Tarball: `tar` + `flate2` (verify presence in Cargo.lock at execution time; if absent, these are the conventional minimal additions and are justified — a tarball format is intrinsic to R2). No `ring`/`ed25519`/`age`/`sodiumoxide` — they are not current deps and SHA-256+RS256 satisfy R2/R6/R10.

### KTD3. Signature shape — detached signature over a slice-LIST-committed manifest; two-level trust chain (PASS-2 CONFIRM)

**What is signed (F1 truncation defense).** The manifest declares, per slice, a SHA-256 content hash of that slice's serialized bytes (R6 content-addressing). The signature is computed over the canonical serialization of: `format_version` + `[identity]` + an **ordered, explicit slice-type list** (`["soul","memory",...]`) + each `[[slice]]` hash + `created_at` + `source_agent` — NOT over the tarball bytes (so it is stable under tar/gzip non-determinism and verifiable from `manifest.toml` alone). **The ordered slice-list and a slice-count are inside the signed content** so a verifier detects slice *omission/truncation*, not only slice *substitution*: dropping a `[[slice]]` entry changes the committed list and breaks the signature (addresses architect F1 — a hash-set alone is truncatable; a committed list is not).

**Trust chain (the option-(c) construction — pass-2 confirm).** The canvass ratified "operator-bundle-shipped, root chains to the GitHub App key." The plan's construction is a **two-level signature chain**, which needs no encryption primitive and no new `rsa` crate:

1. Operator holds a slice-2 **bundle-signing keypair** (operator-generated at provisioning).
2. The bundle is signed with the bundle-signing private key (RS256 via `jsonwebtoken`).
3. The bundle ships the bundle-signing **public key** plus an **attestation**: the public key is signed by the operator's GitHub App key (the root of trust). The receiver, who trusts the GitHub App key, verifies the attestation → trusts the bundle key → verifies the bundle signature.

This reads the canvass's "encrypted with the operator's GitHub App key" as **attested-by-signature-with** the App key. For slice-2 *verification* the receiver needs only the bundle key's *public* half (no secret to encrypt). Literal RSA key-wrapping (encrypting a private key with the App key) would only be needed if the receiving pod must *re-sign* its own exports — which is the per-instance-signing capability explicitly deferred to R7/slice 3. **Pass-2 question for the architect: confirm option (c) = two-level signature-attestation chain (plan's position, no `rsa` crate), OR does it require literal RSA encryption of key material now?** The verifier is a `trait BundleVerifier` with a `GitHubAppChainVerifier` impl for slice 2; a future `DidVerifier` slots in via the manifest's `key_ref`/`key_model` field with no format change (R6 forward-compat). If the architect rules for literal key-wrapping, the only delta is adding the `rsa` crate + an unwrap step — the manifest format and the chain shape are unchanged.

### KTD4. DID-shaped identity with a centralized method in the mission window

Identity claim format: `did:mika:<agent-fingerprint>` where `<agent-fingerprint>` is a SHA-256 (truncated, matching the KG's 16-hex-char convention) of the source agent's `identity.toml` canonical bytes. The `did:mika:` method resolves centrally (operator GitHub App key is the root of trust) in slice 2; the URI shape is what slice 3 + chain migration build on. No DID resolution library — just the URI string + the documented method semantics. Confirmed no existing DID notion in the codebase, so this is the founding shape.

### KTD5. Data-secret handling — REDACT-by-default, enforced by a structural RedactionPolicy gate (F2)

This KTD governs **secret DATA inside the transferable slices** (API keys, tokens, MCP creds) — a *separate* concern from the bundle-signing key material (KTD3). Conflating them was the source of the first-pass confusion; they are now decoupled.

**Position: redact-by-default.** Transferable slices carry NO secret material. The `mcp` slice ships `mcp.json` structure with `env`/`headers` secret VALUES stripped to a `[FROM_POD_SECRETS]` placeholder; `memory`/`kg`/`conversations`/`tasks`/`soul`/`skills` carry no API keys by construction. Pod-side application fills secrets from its own K8s secrets (R10: "pod-side application uses the receiving pod's secret material, not the bundle"). With no secrets in the bundle, R10's "wrap with operator-key encryption" is satisfied by *absence* — there is nothing to wrap.

**F2 — redaction is a STRUCTURAL invariant, not a per-extractor convention.** "Carries no secrets by construction" is a claim about *today's* data, not a guarantee. The export pipeline applies a mandatory `RedactionPolicy` pass after extraction and before serialization. Each `SliceExtractor` **declares its redactable fields** via a trait method (`fn redactable_fields(&self) -> &[FieldPath]` — `mcp`: the `env`/`headers` values; all others: empty today). The pipeline runs the policy over every slice unconditionally; a new extractor that carries a secret without declaring it fails a **test-time assertion** (a `redaction_completeness` test scans serialized slice bytes for known secret-shaped markers, reusing the existing `secret_scrubber::scrub_secrets()` detection from mika#908). Redaction is cross-cutting (review-guide § Orthogonality), enforced by structure, so the invariant cannot degrade silently as slices are added.

**Reconciliation with the canvass "encrypted with" language (F3).** The canvass's "bundle is encrypted with the operator's GitHub App key" refers to the *signing-key trust chain* (KTD3), not to encrypting slice data. Slice *data* is protected by redaction (no secrets travel); slice *authenticity* is protected by the KTD3 signature chain. No RSA *encryption* of slice data is performed in slice 2. This note exists so a future reader of the canvass record does not treat "encrypted with" as an unimplemented slice-data-encryption requirement.

### KTD6. Apply semantics — additive-merge with manifest-declared mode, no destructive overwrite in slice 2

`mika import` applies slices additively where the data model is append/upsert (memory facts, people, KG subject entities, conversations, tasks) and replace-file where it is a single document (`soul.md`, `identity.toml` — guarded behind `--overwrite-soul` since clobbering identity is destructive). Round-trip equivalence (AC: extract → reapply → equivalent state) is tested per slice. Cross-agent apply (bundle from agent A applied to agent B) re-scopes `agent_id` on insert. Conflict resolution beyond upsert is explicitly Phase 2 (brainstorm scope boundary).

### KTD7. Slice extraction is per-slice-module, composed by a registry

Each slice is a `trait SliceExtractor { fn slice_name(&self) -> &str; fn extract(&self, agent_id, db, home) -> Result<SliceContent>; fn apply(&self, …) -> Result<()>; }` with one impl per slice (`SoulSlice`, `MemorySlice`, `KgSlice`, `ConversationsSlice`, `SkillsSlice`, `McpSlice`, `TasksSlice`). A registry maps slice names → impls; `--include` selects a subset; `--include all` = all transferable impls. This keeps each slice's read/write logic isolated and independently testable, and makes adding a future slice a one-impl change.

## High-Level Technical Design

```
mika export --agent prime --include soul,memory --sign
        │
        ▼
┌───────────────────────────────────────────────┐
│ SliceRegistry.select(["soul","memory"])        │
│   SoulSlice.extract()   → bytes + sha256        │
│   MemorySlice.extract() → bytes + sha256        │
└──────────────┬────────────────────────────────┘
               ▼
┌───────────────────────────────────────────────┐
│ Manifest builder (R6/R8)                        │
│   [identity] did = "did:mika:<fingerprint>"     │
│   [[slice]] name=soul  hash=<sha256>            │
│   [[slice]] name=memory hash=<sha256>           │
│   created_at, source_agent                      │
│   [signature] alg=RS256 value=<base64>  (R2)    │
│     ← sign canonical(manifest minus signature)  │
│        with GitHub App EncodingKey               │
└──────────────┬────────────────────────────────┘
               ▼
   bundle.tar.gz  { manifest.toml, slices/soul.tar, slices/memory.json, ... }

mika import bundle.tar.gz   (local)          gateway POST /a2a/{cust}/{agent}/bundle (2b)
        │                                              │ verify signature (R5/R10)
        ▼                                              ▼ forward bytes to pod
┌──────────────────────┐                     pod POST /import-bundle  ─┐
│ BundleVerifier        │  ◄── same apply ──────────────────────────────┘
│  verify sig + hashes  │       logic, exposed over HTTP pod-side
│ SliceRegistry.apply() │
└──────────────────────┘
```

Directional only. Exact struct shapes resolve at execution time against live types.

## Implementation Units

> Units below. Sub-slice 2a = {U1, U7, U2a, U2b, U3, U4, U6}; sub-slice 2b = {U5}. Within 2a the build order is U1 → U7 (co-designed) → U2a → U2b → U3 → U4 → U6.

### U1. Manifest schema + signing/verification core (R6, R8, R2-signature)

**Goal:** The `manifest.toml` format, the content-addressing, and the sign/verify primitives that everything else hangs on.

**Requirements:** R6 (DID-shape, content-addressing, forward-compat), R8 (legible TOML, §6 vocabulary), R2 (signature field + verifiability).

**Dependencies:** none.

**Where it lives:** a new **dedicated `crates/mika-bundle/` crate** (architect-confirmed first-pass — the gateway must verify signatures in 2b without pulling the whole `mika-agent` crate's DB/LLM/tool deps). `mika-bundle` holds the no-DB core (manifest + sign + verify + DID + RedactionPolicy trait), depending only on `serde`/`toml`/`jsonwebtoken`/`sha2`/`base64`. The DB-coupled slice extractors (U2a/U2b) stay in `mika-agent` and depend on `mika-bundle`. `mika-cli` and `mika-gateway` both depend on `mika-bundle`.

**Files:**
- `crates/mika-bundle/Cargo.toml` + `src/lib.rs` (new crate) — workspace member; minimal deps.
- `crates/mika-bundle/src/manifest.rs` (new) — `Manifest`, `SliceEntry`, `IdentityClaim`, `Signature` structs; serde TOML ser/de; canonical-serialization fn for signing (deterministic field order, signature field excluded, ordered `slices` list included per F1).
- `crates/mika-bundle/src/sign.rs` (new) — `sign_manifest(canonical: &[u8], key: &EncodingKey) -> String` reusing the RS256 path from `github_app.rs:383`; bundle-key attestation helper (KTD3 chain).
- `crates/mika-bundle/src/verify.rs` (new — crate placement per Risks) — `trait BundleVerifier { fn verify(&self, manifest) -> Result<()> }`; `GitHubAppChainVerifier` impl (verify the bundle-key attestation against the App public key, then verify the bundle signature against the bundle key — the KTD3 two-level chain); content-hash check (recompute SHA-256 per slice, compare) + committed-slice-list check (F1).
- `crates/mika-bundle/src/did.rs` (new) — `did_for_identity(identity_bytes) -> String`.
- `crates/mika-bundle/src/redaction.rs` (new) — `RedactionPolicy` trait + pipeline pass (KTD5/F2); `FieldPath` type; reuses `secret_scrubber::scrub_secrets()` detection (mika#908) for the test-time completeness assertion.
- Workspace `Cargo.toml` — add `mika-bundle` to `[workspace] members`.

**Approach:**
- Manifest TOML (R8 legibility) — top-level `format_version` (schema evolution marker; slice 3 per-instance-keys bump it), `[identity]` (`did`, `source_agent`, `created_at`, `key_model = "operator-bundle-shipped-slice2"` naming the transitional locus per the canvass caveat), an explicit **`slices = ["soul","memory",...]` ordered list** (F1 truncation defense — committed in the signed content), repeated `[[slice]]` (`name`, `sha256`, `byte_len`), `[signature]` (`alg = "RS256"`, `value = <base64>`, `key_ref`, `bundle_key_attestation = <base64>` per the KTD3 chain). Section names use the §6 vocabulary exactly.
- **Non-transferable slice enumeration (canvass Q1).** The schema names the slice types that are NOT transferable — engine-internal §6 modules (`notifications`, `dashboard_queries`, `planning`, `evidence`, `agent_loop`, `tool_execution`) **and the tool-call cache** (canvass Q1: transferable = no, by default). Naming them explicitly means a future version can flip one to transferable without a `format_version` bump for readers that already know the name.
- Content-addressing (R6): each `[[slice]].sha256` is the hex SHA-256 of that slice's serialized bytes. Bundle integrity = signature covers `format_version` + identity + the ordered `slices` list + each slice hash + metadata (KTD3 — list inclusion defeats truncation); slice integrity = recompute-and-compare.
- DID (R6/KTD4): `did:mika:<16-hex>` from SHA-256 of `identity.toml` canonical bytes.
- Sign (R2/KTD3): RS256 over the canonical signed-content bytes (signature section excluded), base64, store in `[signature].value`; ship the bundle-key public + App-key attestation per the KTD3 chain.
- Verify forward-compat (R6): `BundleVerifier` is a trait; `key_ref`/`key_model` selects the verifier. Slice 2 → `GitHubAppChainVerifier`; a future `"did"` → `DidVerifier` with no manifest change.

**Test scenarios:**
- Round-trip TOML ser/de of a manifest with 2 slices + the ordered `slices` list preserves all fields.
- `sign_manifest` then `GitHubAppChainVerifier::verify` over the same manifest + keys returns Ok.
- Tampering a slice hash → verify Err (signature mismatch).
- Tampering a slice's bytes (hash recompute differs) → verify Err (hash mismatch).
- **Truncation (F1): removing a `[[slice]]` entry AND its hash, leaving the others intact → verify Err** (the committed `slices` list no longer matches).
- Manifest with no `[signature]` → verify Err (unsigned-rejected, R10).
- A bundle key whose attestation is not signed by the trusted App key → verify Err (broken chain).
- `did_for_identity` stable for identical identity bytes, differs otherwise.

**Verification:** `cargo test -p mika-agent bundle::` passes. `cargo clippy` clean.

### U2a. Leaf slice extractors (R3) — soul, skills, mcp, tasks

**Goal:** Per-slice extract+apply for the **leaf slices** (file/row copies, no derived-index handling). Per the canvass U2a/U2b split.

**Requirements:** R3 (slice set + composition), KTD6 (apply semantics), KTD7 (registry + `redactable_fields`).

**Dependencies:** U1, U7 (registry trait + RedactionPolicy live in `mika-bundle`).

**Files:**
- `crates/mika-agent/src/bundle/slices/mod.rs` (new) — `trait SliceExtractor` (incl. `redactable_fields()` per F2), `SliceRegistry`, `transferable_slices()` (the R3 set), `SliceContent` (bytes + format tag). Trait re-exported from `mika-bundle`; impls live in `mika-agent` (DB-coupled).
- `crates/mika-agent/src/bundle/slices/soul.rs` — reads `identity.toml`, `soul.md`, `heartbeat.md`, `user.md` from `~/.mika/agents/<name>/` (`prompt.rs:284 load_identity`); apply writes them back (identity behind `--overwrite-soul`, KTD6). Soul-version history (canvass Q5) resolved internal to this extractor at execution — default: export current soul only.
- `crates/mika-agent/src/bundle/slices/skills.rs` — `skill_overrides` rows (`db.rs:4386-4511`) + `marketplace.lock` + custom (non-symlink) skill dirs under `~/.mika/agents/<name>/skills/`; the `[skills].allowlist` rides in the soul slice (reference, don't duplicate).
- `crates/mika-agent/src/bundle/slices/mcp.rs` — `mcp.json` with `env`/`headers` secret VALUES declared as `redactable_fields` (F2 — the RedactionPolicy strips them to `[FROM_POD_SECRETS]`); apply writes `mcp.json` at `0o600`, secrets sourced pod-side.
- `crates/mika-agent/src/bundle/slices/tasks.rs` — `tasks` rows (agent-scoped, `db.rs:4548-5267`); apply with `agent_id` re-scope; recurring vs one-shot preserved via existing columns. Tool-call cache is NOT a slice (canvass Q1 — non-transferable, named in U1 schema).

**Test scenarios (per slice, round-trip — AC):** seed → `extract()` → fresh/other agent → `apply()` → equivalent state; cross-agent re-scopes `agent_id`; `mcp` extracted bytes contain no plaintext secret values (RedactionPolicy assertion, F2); `--include soul,skills` composes.

**Verification:** `cargo test -p mika-agent bundle::slices` (leaf subset) green.

### U2b. Indexed slice extractors (R3) — memory, kg, conversations

**Goal:** Per-slice extract+apply for the **indexed slices** — derived-index and scoping complexity lives here. Per the canvass U2a/U2b split.

**Requirements:** R3, KTD6, KTD7.

**Dependencies:** U1, U7, U2a (shares the registry/trait).

**Files:**
- `crates/mika-agent/src/bundle/slices/memory.rs` — L1 `core_memory`, L2 `people`/`commitments`/`preferences`/`events`, L3 `search_content` (`memory/mod.rs:23-189`); JSON rows; apply via upsert. **L3 derived-index:** `vec_search`/`fts_search` are derived virtual tables — carry `embedding_json` forward in `search_content` rows and rebuild FTS/vec on apply (avoids re-embedding API spend); prefer carry-forward, confirm at execution.
- `crates/mika-agent/src/bundle/slices/kg.rs` — **agent-scoped KG only**: `kg_subject_resolutions`, `kg_resolutions_log` (have `agent_id`). The domain layer (`kg_entities`/`kg_relationships`) is deterministically rebuilt at startup; the shared-corpus layer is keyed by `docs_root_hash` (not agent-owned) — **export neither** (both regenerate). Architect first-pass **confirmed** this scoping (KG slice = agent-scoped resolution rows only).
- `crates/mika-agent/src/bundle/slices/conversations.rs` — `sessions` + `messages` (agent-scoped, `db.rs:128-152`), **paginated** read for large histories; apply re-scopes `agent_id`/`session_id` on insert.

**Test scenarios:** memory round-trip incl. L3 — after apply, `search_memory` returns the same hits (FTS+vec rebuilt/carried); kg round-trip carries only agent-scoped resolution rows (assert domain/shared-corpus NOT in bundle); conversations round-trip across pagination boundary; `--include all` selects exactly the seven transferable slices and excludes §6 modules + tool-call cache.

**Verification:** `cargo test -p mika-agent bundle::slices` (indexed subset) green.

### U3. `mika export` (R2 producer)

**Goal:** CLI command composing slices → manifest → signed tarball.

**Requirements:** R2 (producer), R3 (`--include`), R10 (`--sign` required for transfer).

**Dependencies:** U1, U2a, U2b.

**Files:**
- `crates/mika-cli/src/cli.rs` — add `Export(ExportArgs)` to `Commands` enum (`cli.rs:35-89`); `ExportArgs { agent, include: Vec<String>, sign: bool, out: Option<PathBuf> }`.
- `crates/mika-cli/src/main.rs` — dispatch arm (`main.rs:268-376`).
- `crates/mika-cli/src/commands/export.rs` (new) — `pub async fn run(args)`; `init::init_for_agent`, select slices, build+sign manifest (GitHub App key from `Settings`), write `bundle.tar.gz` (default `~/.mika/agents/<name>/exports/<name>-<ts>.tar.gz` per `runtime-structure.md` exports/ path).
- `crates/mika-cli/src/commands/mod.rs` — `pub mod export;`.

**Approach:** Resolve `--include` (default `all`). Extract each via registry. Compute per-slice SHA-256. Build manifest, sign with the GitHub App `EncodingKey` (`GitHubApp::from_settings`). If `--sign` absent, refuse to produce a transfer bundle (or produce an explicitly-unsigned local-only artifact that `import` will reject for transfer — architect to confirm; default: `--sign` is required, no unsigned bundles, matching R10).

**Test scenarios:**
- `mika export --agent <test> --include soul --sign` produces a file; `bundle verify` (U6) passes on it.
- Missing GitHub App key in Settings → clear single-line error, no panic, exit non-zero.
- `--include bogus` → error naming the valid slice set.

**Verification:** `cargo test -p mika-cli export`; manual smoke export of a dev agent.

### U4. `mika import` (R2 consumer, local apply)

**Goal:** Verify + apply a bundle to a local agent.

**Requirements:** R2 (consumer), R10 (reject unsigned/tampered), KTD6 (apply semantics).

**Dependencies:** U1, U2a, U2b. (`--remote` forwarding is wired here but exercised in 2b/U5.)

**Files:**
- `crates/mika-cli/src/cli.rs` — `Import(ImportArgs)`; `ImportArgs { bundle: PathBuf, remote: Option<String>, agent: Option<String>, overwrite_soul: bool }`.
- `crates/mika-cli/src/main.rs` — dispatch arm.
- `crates/mika-cli/src/commands/import.rs` (new) — verify (`GitHubAppChainVerifier`), then per-slice `apply()`; if `--remote` set, POST the bundle bytes to the gateway endpoint (U5) instead of local apply.

**Approach:** Unpack tarball, read `manifest.toml`, verify signature + every slice hash (reject on any failure, R10), apply selected slices via registry to the target agent (`--agent`, default current). `--remote <endpoint>` short-circuits to HTTP push (reuses the bearer/`MIKA_INTERNAL_TOKEN` contract).

**Test scenarios:**
- Round-trip (AC): export from agent A → import to fresh agent B → B's state reconciles.
- Unsigned bundle → rejected with clear error.
- Tampered slice → rejected.
- `--overwrite-soul` absent + soul slice present → soul.md applied but identity.toml left intact (or skipped with a notice); present → identity replaced.

**Verification:** `cargo test -p mika-cli import`; the export→import round-trip integration test is the AC anchor.

### U6. `mika bundle list/verify` + `mika agent slices` (R9)

**Goal:** Read-only bundle/agent inspection.

**Requirements:** R9.

**Dependencies:** U1 (verify), U2a + U2b (agent slices enumeration).

**Files:**
- `crates/mika-cli/src/cli.rs` — `Bundle(BundleArgs)` with nested `BundleCommand { List { bundle }, Verify { bundle } }` (2-level pattern per `AgentsCommand`, `cli.rs:329-387`); add `Slices { name }` variant to the existing `AgentsCommand`.
- `crates/mika-cli/src/main.rs` — dispatch arm for `Bundle`.
- `crates/mika-cli/src/commands/bundle.rs` (new) — `list` prints `manifest.toml` without unpacking slices; `verify` checks signature + hashes, prints PASS/FAIL per slice, exits non-zero on FAIL.
- `crates/mika-cli/src/commands/agents.rs` — add `Slices` arm: for each transferable slice, report whether the named agent has content (row count > 0 or file exists) without exporting.

**Test scenarios:**
- `bundle list` on a valid bundle prints identity + slice names + hashes, never unpacks.
- `bundle verify` exits 0 on a good bundle, non-zero on tampered/unsigned.
- `agent slices <name>` lists the seven slices with has-content booleans.

**Verification:** `cargo test -p mika-cli bundle`; `cargo run --bin mika -- bundle --help` and `agent slices --help` render.

### U7. Security baseline wiring (R10) — cross-cutting, lands inside 2a

**Goal:** Enforce the R10 invariants across U3/U4/U6.

**Requirements:** R10 (unsigned rejected, sig verify before apply, secret redaction per KTD5, TLS).

**Dependencies:** U1 (sign/verify primitives); co-designed with U1 per canvass. Cross-cuts U2a (mcp redaction via RedactionPolicy), U3 (require `--sign`), U4 (verify-before-apply).

**Scope:** Co-designed with U1 (the security baseline informs the manifest schema, per canvass). Part code (`mika-bundle/src/redaction.rs` — the `RedactionPolicy` trait + pipeline pass, F2), part enforced invariants + tests: (1) `import`/gateway reject any bundle whose signature/chain does not verify, whose `[signature]` is absent, or whose committed slice-list is truncated (F1); (2) the `RedactionPolicy` runs over every slice unconditionally — `mcp` declares its secret fields, all others declare none, and a test-time completeness assertion (reusing `secret_scrubber::scrub_secrets()`, mika#908) fails if any serialized slice contains secret-shaped bytes (F2); (3) TLS is ingress-terminated (gateway already assumes this — `main.rs:34` installs rustls for outbound; no gateway-side cert code, confirmed).

**Test scenarios:**
- Negative-path matrix: unsigned, wrong-key-signed, broken-attestation-chain, tampered-hash, tampered-bytes, **truncated-slice-list (F1)** → all rejected at `import` and (2b) at the gateway.
- `RedactionPolicy` completeness: a synthetic extractor carrying an undeclared secret fails the assertion (F2).
- `mcp` round-trip never carries plaintext secret values.

**Verification:** the negative-path + redaction-completeness test matrix is green.

### U5. Gateway transfer endpoint (R5) — sub-slice 2b

**Goal:** Authenticated gateway endpoint: push (verify sig → forward to pod) + pull (cloud → local snapshot) + pod-side `/import-bundle` receiver.

**Requirements:** R5 (endpoint, verify-before-forward, pod routing, pull), R10 (TLS, gateway-side verify).

**Dependencies:** **Sub-slice 2a complete** (U1 verify in `mika-bundle`, U2a/U2b apply, U4 producer/consumer) — pod-side receiver = 2a's apply exposed over HTTP. **AND `mika-cloud#111` (code dependency, per canvass).** The canvass recorded U5 as a *code* dependency on mika-cloud#111's pod-routing — not merely a deploy dep. **Resolve at execution (F4):** if the gateway's existing `container_url()` (`routes.rs:564`) suffices to route a bundle to a pod, then mika-cloud#111 is deployment-only and no `mika-cloud` code change is a precondition; if bundle routing needs new gateway/helm wiring that lands in mika-cloud#111, enumerate those changes and sequence them as a hard pre-condition for the 2b PR. The plan's reading (from the gateway exploration) is that `container_url()` already resolves `customer_id → pod URL` and the new route reuses it — so mika-cloud#111 is most likely deploy-coordination (ship the gateway with the new route) rather than a code blocker. **Confirm with the architect / check mika-cloud#111 before dispatching 2b.**

**Files:**
- `crates/mika-gateway/src/routes.rs` — register `POST /a2a/{customer_id}/{agent_name}/bundle` with `require_bearer_token` (`routes.rs:964`) + a raised `RequestBodyLimitLayer` (bundles exceed the 2MB A2A cap — set e.g. 50MB; architect to size). Handler: read body bytes, parse manifest, `GitHubAppChainVerifier::verify` (R5/R10 verify-before-forward), resolve pod via `container_url()` (`routes.rs:564`), forward bytes with `Content-Type: application/x-tar` mirroring the A2A reqwest pattern (`a2a_routes.rs:104-128`). Gateway depends on `mika-bundle` (verify only — no DB).
- `crates/mika-gateway/src/routes.rs` — `GET /a2a/{customer_id}/{agent_name}/bundle?include=…` pull path: gateway requests a pod-side export, streams the produced bundle back to the caller. **Offline-pull semantics (canvass Q3): resolve at execution** — if the source pod is unreachable, the pull returns a clear 5xx (no partial/stale snapshot); cloud→local snapshot has no offline-cache obligation in slice 2.
- `crates/mika-agent/src/server/handlers.rs` — `POST /import-bundle` (bearer-auth, body = tarball bytes): verify + apply via 2a's registry (`server/mod.rs:86-286` route registration pattern). `GET /export-bundle?include=…` for the pull path: produce a signed bundle pod-side.
- `crates/mika-gateway/src/routes.rs` build_router (`routes.rs:147-286`) — wire both routes.

**Approach:** Mirror the A2A proxy precisely for auth + pod resolution + forwarding; the only new logic is signature verification before forward (the gateway is the R10 trust boundary, via `mika-bundle`) and the larger body limit. Pull = gateway calls the pod's `GET /export-bundle` and relays.

**Test scenarios:**
- Push: signed bundle POSTed with valid bearer → gateway verifies, forwards to a mock pod, pod applies, 200.
- Push with unsigned/tampered bundle → gateway rejects with 4xx **before** forwarding (assert no pod call).
- Push with bad/missing bearer → 401 (existing middleware).
- Pull: `GET …/bundle` returns a signed bundle the caller can `bundle verify`.
- Unknown customer_id → pod resolution yields an unreachable URL → 502 (existing A2A failure shape).

**Verification:** `cargo test -p mika-gateway` (mock pod via in-process axum/wiremock, the pattern slice-1 U2 cited); `cargo test -p mika-agent server::` for the receiver.

## Sequencing

```
Sub-slice 2a (one PR):  U1 ─► U7 ─► U2a ─► U2b ─► U3 ─► U4 ─► U6   ──► close/advance #1538
                        manifest+ security  leaf  indexed export import inspect
                        sign/verify (co-designed w/ U1)
Sub-slice 2b (one PR):  U5  (gateway push+pull + pod receiver)      ──► requires 2a merged + mika-cloud#111 resolved
```

2a is fully testable with zero cloud/network dependency. 2b requires 2a's apply logic + the `mika-bundle` verify core to exist, and mika-cloud#111 resolved (F4). Never dispatch 2b before 2a merges. U1+U7 are co-designed (security baseline informs manifest schema, per canvass).

## Risks & Dependencies

- **Crate placement — RESOLVED (architect first-pass confirmed).** Sign/verify/manifest core lives in a dedicated `mika-bundle` crate (deps: `serde`/`toml`/`jsonwebtoken`/`sha2`/`base64`; no DB); `mika-agent`, `mika-cli`, `mika-gateway` all depend on it. DB-coupled slice extractors (U2a/U2b) stay in `mika-agent`. This keeps the gateway's 2b dependency to a signature-verification crate, not the whole agent.
- **Signing-key construction (KTD3) — primary pass-2 confirm.** Plan commits to a two-level signature-attestation chain (App key attests bundle key; bundle key signs bundle), no `rsa` crate. If the architect rules the canvass's "encrypted with" means literal RSA key-wrapping, the delta is +`rsa` crate + unwrap step; manifest format unchanged.
- **mika-cloud#111 (U5/2b code-vs-deploy dependency, F4)** — confirm whether U5 needs new pod-routing code in mika-cloud or just deploy-coordination. Plan's reading: `container_url()` already routes; likely deploy-only. Check before 2b dispatch.
- **KG slice scoping — CONFIRMED (architect first-pass).** Export agent-scoped `kg_subject_resolutions`/`kg_resolutions_log` only; domain + shared-corpus regenerate.
- **Secret data redaction (KTD5/F2)** — structural `RedactionPolicy` gate + test-time completeness assertion (reusing `secret_scrubber`); not a per-extractor convention.
- **L3 vector index** — carry `embedding_json` forward to avoid re-embedding API spend; confirm at execution.
- **Tarball deps** — if `tar`/`flate2` not in Cargo.lock, they are justified new deps (a tarball IS the R2 deliverable). Verify at execution.
- **Body-size limits** — gateway bundle route needs a cap well above the 2MB A2A limit (~50MB; architect to size). Streaming is out-of-scope (brainstorm boundary), so a bounded in-memory limit is the slice-2 shape.

## Open Questions (for pass-2 / execution)

1. **KTD3 (PRIMARY pass-2):** confirm option (c) signing construction = two-level signature-attestation chain (plan's position, no `rsa`), or literal RSA key-wrapping?
2. **KTD1:** milestone-with-sub-issues vs. two-sequenced-PRs-closing-#1538? (plan recommends two PRs.)
3. **U5 (F4):** is mika-cloud#111 a code precondition for 2b or deploy-coordination only? (plan reads: likely deploy-only via existing `container_url()`.)
4. **U3:** `--sign` mandatory always (no unsigned-bundle code path)? (plan recommends mandatory, per R10.)
5. **U5:** gateway bundle-route body-size cap (~50MB bounded; no streaming this slice).
6. **Execution-time (canvass Q3/Q5):** offline-pull failure semantics (plan: clean 5xx, no stale snapshot); soul-version history (plan: current-soul-only default).

**Resolved by architect first-pass (no longer open):** KTD5 redact-by-default reading of R10 (now F2-structural); dedicated `mika-bundle` crate; KG slice = agent-scoped resolution rows only; AC coverage across 2a/2b (all 7, none orphaned); 2a not further split; signing-over-manifest-hash-set tamper-evidence (sharpened to committed slice-list per F1).

## Sources & Research

- Origin: `docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md` (R2–R10).
- Predecessor: `docs/plans/2026-06-09-003-feat-ascension-architecture-first-slice-cli-plan.md` (R1, canonical R-list, slice-2 scope + signing-key locus from 2026-06-15 canvass).
- **Canvass record:** mika#1538 comment (samidarko, 2026-06-15T09:59Z, architect session `5da94962-1fc1-4601-aa2a-27ba43eb1f1d`) — ratified F2 (signing-key option (c)), F3 (R4+R7 → slice 3, U2 split into U2a/U2b), Q1 (tool-call cache non-transferable), U5↔mika-cloud#111 code dep, sequencing U1→U7→U2→U3→U5→U4→U6.
- First-pass architect review (session `7cefb65c-4b29-4c67-a34e-055f39388f00`, Disposition: ITERATE) — F1 (truncation defense via committed slice-list), F2 (structural RedactionPolicy), F3 (reconcile "encrypted with" language), F4 (mika-cloud#111 code-vs-deploy).
- CLI structure: `crates/mika-cli/src/cli.rs:35-89` (Commands enum), `:329-387` (AgentsCommand nested pattern), `src/main.rs:268-376` (dispatch), `src/init.rs:47-177` (DB/Settings init). mika-cli already deps mika-agent/common/a2a (`Cargo.toml:22-55`).
- Slice storage: soul `prompt.rs:227-300` (`Identity`, `load_identity`); memory `memory/mod.rs:23-189` (core_memory/people/commitments/preferences/events), `search_content` (L3); KG `db/kg_schema.rs` (agent-scoped `kg_subject_resolutions`/`kg_resolutions_log` vs domain/shared-corpus); conversations `db.rs:128-152` (`sessions`/`messages`); skills `db.rs:4386-4511` (`skill_overrides`) + `~/.mika/agents/<name>/skills/`; mcp `mcp/config.rs:26-124` (`mcp.json`, `0o600`, redacting Debug); tasks `db.rs:4548-5267` (`tasks`). DB at `~/.mika/data/mika.db`, `agent_id`-scoped.
- Crypto: GitHub App RSA key `mika-common/src/config.rs:755`/`github_app.rs:95,383` (`EncodingKey::from_rsa_pem`, RS256 via `jsonwebtoken` 9.3.1); hashing `sha2` 0.10.9 (KG `kg/config.rs` SHA-256, 16-hex convention); `base64` 0.22.1; `secrecy` 0.10.3 (`SecretString`, `.expose_secret()`, redacting Debug). No existing DID notion.
- Gateway: `routes.rs:147-286` (build_router), `:964-982` (`require_bearer_token`), `:564-586` (`container_url` pod resolution), `a2a_routes.rs:104-128` (reqwest forward pattern); agent server `server/mod.rs:86-286` (endpoint registration), `handlers.rs` (`/message` pattern). TLS ingress-terminated; `main.rs:34` installs rustls for outbound.
- No existing export/import/bundle/snapshot/backup code (repo-wide grep) — greenfield.

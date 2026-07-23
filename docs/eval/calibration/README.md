# Calibration Artifacts

The `calibrate` binary (`crates/mika-agent/src/bin/calibrate.rs`) runs a role-scoped
scenario suite against a candidate model and emits two companion artifacts per run:

- **JSON** — machine-readable, written to `target/eval-calibration/<role>-<ts>.json`.
  Committed baselines under `docs/eval/calibration/**` share this schema and are loaded
  by the pre-swap gate (`CalibrationArtifact::load`).
- **Markdown** — human-readable report (`RoleScoreReport::to_markdown`).

## Artifact schema

### Top level

| Field | Type | Notes |
|-------|------|-------|
| `version` | `u32` | Schema version. See evolution below. |
| `timestamp` | `String` | ISO 8601 (RFC 3339) run time. |
| `providers` | map | `provider → { model, scenarios }`. |

### Per-scenario (`scenarios[<id>]`)

| Field | Type | Notes |
|-------|------|-------|
| `outcome` | `String` | `"pass"` / `"fail"`. |
| `error_class` | `Option<String>` | *Classified* failure bucket (`schema_validation`, `rate_limit`, `timeout`, `auth_error`, `unknown`). |
| `input_tokens` | `Option<u64>` | |
| `output_tokens` | `Option<u64>` | |
| `latency_ms` | `Option<u64>` | `Option` so hand-authored baselines that record `null` still load. |
| `response_text` | `Option<String>` | **v2** — full LLM output, capped (see below). Populated for both PASS and FAIL. |
| `failure_reason` | `Option<String>` | **v2** — the *raw* human-readable reason (e.g. ``Did not name `make deploy` as the deploy path``), distinct from the classified `error_class`. |

## Schema evolution

### v1 → v2 (mika#1716)

v2 adds two optional per-scenario fields at the same nesting as `outcome`:

- **`response_text`** — the model's actual words, capped at **8000 chars**
  (`RESPONSE_TEXT_CAP`). Truncation is UTF-8-safe (cuts on a char boundary, never a raw
  byte slice — mika#764) and appends a `… [truncated to 8000 chars]` marker when it
  fires. Captured for **both** PASS (supports scenario-tuning) and FAIL (enables the
  verify-not-guess diagnostic: distinguishing a real fail from fixture-strictness without
  re-running with debug logging).
- **`failure_reason`** — the raw reason string. Sourced from the runner's `error` field,
  so it carries the exact human-readable reason rather than its classification.

Both fields are `Option<String>` with `#[serde(default)]`, so **v1 baselines that lack
them still deserialize** (both default to `None`). The schema `version` is not validated
on read, so old v1 artifacts remain loadable by the pre-swap gate unchanged.

The markdown companion gains a **Failure Details** section: per-FAIL scenario, it shows
the `failure_reason` and a (display-capped) `Response text` snippet. The section is
omitted entirely when every scenario passes.

**No secret scrubbing (AC5).** Response text is treated as ordinary opaque completion
text — no PII/token-secret guard is applied. If a scenario's LLM output ever quotes a
secret from its fixture, that is a fixture bug, not an artifact-schema one.

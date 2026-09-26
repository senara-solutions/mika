# PR Review: feat(config): add validation for agent identity fields

## PR Metadata
- **Title:** feat(config): add validation for agent identity fields
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** feat/1205/identity-validation
- **Files changed:** 3
- **Additions:** 47
- **Deletions:** 2

## Plan Path
`docs/plans/1205-identity-validation-plan.md`

## Acceptance Criteria
- AC1: `validate_identity()` returns `Result<(), ValidationError>` for malformed identity.toml
- AC2: Missing `name` field produces a specific `MissingRequiredField` error variant
- AC3: Startup logs a WARN when validation fails but continues with default identity

## Diff Summary

### `crates/mika-common/src/home.rs`
- Added `validate_identity(path: &Path) -> Result<(), ValidationError>` function
- Added `ValidationError` enum with `MissingRequiredField(String)`, `InvalidFormat(String)` variants
- Function reads identity.toml, checks for required `name` field, returns typed error on failure

### `crates/mika-agent/src/prompt.rs`
- `load_identity()` now calls `validate_identity()` before parsing
- On `Err`, logs `warn!("identity_validation_failed", error = %e)` and falls back to `Identity::default()`

### `crates/mika-common/src/home.rs` (tests)
- Added `test_validate_identity_missing_name` — asserts `MissingRequiredField("name")`
- Added `test_validate_identity_valid` — asserts `Ok(())`

## DIFF ANALYSIS
- AC1: `validate_identity()` is present in home.rs with correct signature `Result<(), ValidationError>` — SATISFIED
- AC2: `ValidationError::MissingRequiredField` variant exists, `validate_identity` returns it when `name` is absent — SATISFIED
- AC3: `load_identity()` logs `warn!("identity_validation_failed")` on validation error and calls `Identity::default()` — SATISFIED

## PLAN-AC VERIFICATION
[✅] AC1 — `validate_identity()` returns `Result<(), ValidationError>` for malformed identity.toml
[✅] AC2 — Missing `name` field produces `MissingRequiredField` error variant
[✅] AC3 — Startup logs WARN on validation failure, continues with default identity

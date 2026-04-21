# Knowledge Graph ID Convention

## Entity Key Format

Every domain-layer entity has a composite text key:

```
entity_key = entity_type || ':' || name
```

Examples: `person:Alice`, `org:Acme Corp`, `concept:Rust`.

The format is enforced by a SQLite CHECK constraint on the `kg_entities` table.

## Dual-Key Design

| Key | Type | Purpose |
|-----|------|---------|
| `id` | `INTEGER PRIMARY KEY` | Fast joins, FK references, internal use |
| `entity_key` | `TEXT UNIQUE COLLATE NOCASE` | External identity, deduplication, human-readable |

**Why both?** Integer PKs are compact and fast for join-heavy queries across the KG (relationships, resolutions, provenance). Text keys provide stable, human-readable identity that extraction pipelines and query tools use for deduplication and display.

## Entity Types

Defined in `crates/mika-agent/src/kg_schema.rs`:

| Constant | Value | Description |
|----------|-------|-------------|
| `ENTITY_TYPE_PERSON` | `person` | Contact, colleague, family member |
| `ENTITY_TYPE_ORG` | `org` | Company, team, group |
| `ENTITY_TYPE_PROJECT` | `project` | Software project, initiative |
| `ENTITY_TYPE_PLACE` | `place` | City, office, venue |
| `ENTITY_TYPE_CONCEPT` | `concept` | Technology, methodology, domain term |
| `ENTITY_TYPE_EVENT` | `event` | Meeting, conference, milestone |

The DB CHECK constraint enforces the same set. Adding a new type requires updating both the Rust constants and the CHECK constraint in the migration.

## Helper Functions

- `format_entity_key(entity_type, name) -> String` — produces the canonical `type:name` format
- `parse_entity_key(key) -> Option<(type, name)>` — splits on the first `:` (names may contain colons)

## Case Sensitivity

Entity keys use `COLLATE NOCASE` — `person:Alice` and `PERSON:ALICE` are considered the same entity. This prevents near-duplicate entities from case variations in extraction.

## Relationship to Future Issues

This ID scheme is the foundation for:
- Entity extraction (#687) — uses `format_entity_key` to produce keys
- Entity query tools (#688) — lookups by `entity_key` or `entity_type`
- Relationship management (#689–#691) — FK references via integer `id`
- Subject resolution (#692) — maps subject mentions to domain entities via `entity_key`

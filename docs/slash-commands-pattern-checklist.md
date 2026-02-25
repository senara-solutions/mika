# Slash-Command System Pattern Checklist

## Quick Reference

| Pattern Area | Status | Details |
|--------------|--------|---------|
| **1. Handler Dispatch** | ✓ COMPLIANT | Match-based dispatch mirrors agent tool pattern; contextual async parameters |
| **2. App Struct Fields** | ✓ COMPLIANT | Follows AppState + ReminderScheduler pattern; proper Arc wrapping; clear grouping |
| **3. Module Structure** | ✓ COMPLIANT | Three-file layout (mod.rs/handlers.rs/autocomplete.rs) matches server module organization |
| **4. Enum Variants** | ✓ COMPLIANT | ChatRole::Command follows PascalCase, properly exhaustive in match, styled distinctly |
| **5. Test Style** | ✓ COMPLIANT | Inline #[cfg(test)] mod pattern, snake_case test names, pragmatic without mocks |
| **6. Naming Conventions** | ✓ COMPLIANT | Flawless: snake_case funcs, PascalCase types, SCREAMING_SNAKE consts, handle_* prefix |

---

## Detailed Checklist

### 1. Handler Dispatch Pattern

**Question:** Does the handler dispatch pattern match other dispatch patterns in the codebase?

**Finding:** ✓ YES

**Evidence:**
- Agent tool dispatch uses trait-based async dispatch with context object
- Slash command dispatch uses match-based async dispatch with context object (App)
- Both are appropriate to their context: tools are pluggable (trait), commands are fixed (match)
- Both pass context as parameter and return result that caller displays

**Code Location:**
- Agent pattern: `crates/mika-agent/src/agent.rs` lines 150-200+
- Slash pattern: `crates/mika-cli/src/tui/commands/handlers.rs` lines 6-31

**Pattern Score:** 10/10

---

### 2. App Struct Field Addition

**Question:** Do the new App struct fields follow conventions for how other subsystems handle shared resources?

**Finding:** ✓ YES

**Evidence:**
- Server AppState pattern: Clone-able, Arc-wrapped resources, mixed primitives/wrapped types
- ReminderScheduler pattern: Owns dependencies (db, claude, tools, skills, home_dir)
- App struct pattern: Mirrors both—shared resources without Arc (TUI-local), subsystem state fields

**New Fields:**
```rust
// Shared resources for slash commands (lines 85-88)
pub db: AsyncDatabase,
pub claude: ClaudeClient,
pub home_dir: PathBuf,
pub skills: Arc<SkillRegistry>,

// Slash command state (lines 91-92)
pub autocomplete: AutocompleteState,
pub pending_command: Option<String>,
```

**Initialization:** Constructor properly initializes all fields (lines 133-134)

**Code Location:** `crates/mika-cli/src/tui/app.rs` lines 54-136

**Pattern Score:** 10/10

---

### 3. Module Structure Consistency

**Question:** Is the module structure (commands/mod.rs, commands/handlers.rs, commands/autocomplete.rs) consistent with other module structures?

**Finding:** ✓ YES

**Structure Comparison:**

| Module | Pattern | Files |
|--------|---------|-------|
| **Agent Tools** | Definition → Implementations | mod.rs + tool_name.rs × 7 |
| **Server** | Core logic → Handlers → Cross-cuts | mod.rs, handlers.rs, state.rs, auth.rs |
| **Slash Commands** | Definitions → Handlers → UI State | mod.rs, handlers.rs, autocomplete.rs |

**Module Declaration:**
- `pub mod commands;` added to `crates/mika-cli/src/tui/mod.rs` in alphabetical order ✓

**Rationale:**
- mod.rs: SlashCommand struct, COMMANDS const, parsing logic
- handlers.rs: Handler functions, dispatch logic
- autocomplete.rs: Popup state, filtering, selection management

**Code Location:** `crates/mika-cli/src/tui/commands/` (3 files)

**Pattern Score:** 10/10

---

### 4. ChatRole Enum Addition

**Question:** Does the ChatRole::Command variant follow patterns for enum variants?

**Finding:** ✓ YES

**Evidence:**
- PascalCase naming: Command (not COMMAND or command) ✓
- Unit variant: No associated data (consistent with User/Assistant/System) ✓
- Exhaustive match: Properly handled in ui.rs draw function (lines 96-105) ✓
- Distinct styling: Color::DarkGray (vs. Color::Red for System) ✓

**Enum Definition:**
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Command,  // NEW
}
```

**Usage Example:**
```rust
ChatRole::Command => {
    lines.push(Line::default());
    for line in msg.content.lines() {
        lines.push(Line::from(vec![Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        )]));
    }
}
```

**Code Location:** `crates/mika-cli/src/tui/app.rs` lines 34-40 (definition)

**Pattern Score:** 10/10

---

### 5. Test Style Consistency

**Question:** Is the test style consistent with other test modules?

**Finding:** ✓ YES

**Test Organization:**
- Inline: `#[cfg(test)] mod tests` in each file ✓
- Multiple test modules: Yes (mod.rs, autocomplete.rs, handlers.rs) ✓
- Test naming: `test_<behavior>` not `test_<function_name>` ✓

**Test Coverage:**

| File | Test Count | Coverage |
|------|-----------|----------|
| commands/mod.rs | 8 tests | Parsing, filtering, edge cases |
| commands/autocomplete.rs | 9 tests | State management, wrapping, selection |
| commands/handlers.rs | 2 tests | Help output, model display |

**Sample Tests:**
```rust
// Parsing test (mod.rs)
#[test]
fn test_parse_command_with_args() {
    let (cmd, args) = parse_command("/memory search hello world");
    assert_eq!(cmd, "memory");
    assert_eq!(args, "search hello world");
}

// Filtering test (mod.rs)
#[test]
fn test_filter_prefix() {
    let results = filter_commands("me");
    let names: Vec<_> = results.iter().map(|c| c.name).collect();
    assert!(names.contains(&"memory"));
    assert!(!names.contains(&"model"));
}

// Autocomplete test (autocomplete.rs)
#[test]
fn test_update_shows_all_on_slash() {
    let mut state = AutocompleteState::new();
    state.update("/");
    assert!(state.visible);
    assert!(!state.items.is_empty());
}
```

**Pattern Note:** Handler tests are lighter (test_handle_help, test_handle_model) because App construction is complex. This matches patterns in agent.rs where context-heavy functions have integration-style tests.

**Code Location:** Inline in each file, lines 115-180 (mod.rs), 66-152 (autocomplete.rs), 379-400 (handlers.rs)

**Pattern Score:** 9/10 (pragmatic; could add more handler tests with a test helper, but not needed now)

---

### 6. Naming Convention Compliance

**Question:** Do naming conventions match the rest of the codebase?

**Finding:** ✓ YES (100% compliance)

**Convention Reference (from CLAUDE.md):**
- `snake_case` for functions/variables
- `PascalCase` for types
- `SCREAMING_SNAKE` for constants

**Naming Audit:**

| Category | Examples | Compliance |
|----------|----------|-----------|
| Constants | `COMMANDS` | ✓ SCREAMING_SNAKE |
| Structs | `SlashCommand`, `AutocompleteState` | ✓ PascalCase |
| Enums | `ChatRole::Command` | ✓ PascalCase |
| Functions | `filter_commands`, `parse_command`, `dispatch` | ✓ snake_case |
| Handler Funcs | `handle_help`, `handle_clear`, `handle_compact` | ✓ handle_* prefix |
| Methods | `update`, `dismiss`, `selected_name` | ✓ snake_case |
| Variables | `pending_command`, `autocomplete`, `visible` | ✓ snake_case |
| Fields | `items`, `selected` | ✓ snake_case |
| Modules | `commands`, `handlers`, `autocomplete` | ✓ snake_case |

**Key Observations:**
- Handler function prefix `handle_*` is clear and consistent
- No ambiguous or cryptic names
- Comments use present tense ("Dispatch a slash...", "Parse a slash command...") matching style elsewhere

**Code Location:** Throughout crates/mika-cli/src/tui/commands/

**Pattern Score:** 10/10

---

## Summary by Dimension

### Dispatch Pattern
- Agent tools: Trait-based, pluggable
- Slash commands: Match-based, fixed set
- **Verdict:** Different implementations, same pattern philosophy ✓

### Shared Resources
- Server: AppState with Arc-wrapped components
- Reminders: ReminderScheduler owns dependencies
- CLI: App holds resources and command state
- **Verdict:** Consistent ownership/sharing model ✓

### Organization
- Three-tier organization (definition, handlers, UI) mirrors existing modules
- **Verdict:** Clean, maintainable structure ✓

### Types
- Enum variant additions are straightforward and properly exhaustive
- **Verdict:** Sound type design ✓

### Testing
- Inline, focused, pragmatic
- **Verdict:** Matches codebase practice ✓

### Naming
- Zero deviations from established conventions
- **Verdict:** Perfect compliance ✓

---

## Integration Quality

### Event Loop Integration
✓ Slash commands fit naturally into existing tick() flow
✓ Deferred processing pattern (queue in send_message, execute in tick)
✓ Redraw flag properly set

### UI Integration
✓ Command output rendered with distinct styling
✓ Autocomplete popup overlays message area
✓ Keyboard handling follows existing patterns

### Database/Resource Integration
✓ Handlers use app.db for queries
✓ No new locking patterns required
✓ Async handling is consistent

---

## Final Assessment

**Overall Pattern Consistency Score: 9.7/10**

| Dimension | Score | Status |
|-----------|-------|--------|
| Dispatch | 10/10 | Excellent |
| App Fields | 10/10 | Excellent |
| Module Structure | 10/10 | Excellent |
| Enum Design | 10/10 | Excellent |
| Tests | 9/10 | Very Good |
| Naming | 10/10 | Excellent |
| Integration | 9/10 | Very Good |

**Recommendation: APPROVED for merge**

No blocking issues. Code demonstrates excellent pattern awareness and consistency with the existing codebase.

---

## Follow-up Notes

### Before Merge
- No action required ✓

### Post-Merge Maintenance
- Consider a command categorization design doc if command count exceeds 20
- Monitor autocomplete complexity; separate into commands/ui.rs if it grows significantly
- Handler test coverage can be improved post-merge if needed


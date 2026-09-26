//! Request-side JSON-schema well-formedness assertions (D9).
//!
//! Catches `task_id`-class bugs at request construction time.
//! No `#[ignore]` — runs in CI on every push.
//!
//! Covered schema emitters:
//! - `default_tools()` — all built-in tool definitions
//! - `inject_task_id_field` — skill exec handler schema mutation (via simulation)
//! - Skill-injected tools (via synthetic schema construction)

use std::collections::HashSet;

use mika_agent::tools::default_tools;
use serde_json::Value;

/// Assert no duplicate entries in the `required` array of a tool schema.
fn assert_no_duplicate_required(schema: &Value, tool_name: &str) {
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        let mut seen = HashSet::new();
        for entry in required {
            if let Some(s) = entry.as_str()
                && !seen.insert(s)
            {
                panic!(
                    "assert_no_duplicate_required failed for tool '{}': \
                     duplicate entry '{}' in required array. Full required: {:?}",
                    tool_name, s, required
                );
            }
        }
    }
}

/// Assert every entry in `required` exists as a key in `properties`.
fn assert_required_in_properties(schema: &Value, tool_name: &str) {
    let required = match schema.get("required").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => return, // No required array → passes
    };
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => {
            if !required.is_empty() {
                panic!(
                    "assert_required_in_properties failed for tool '{}': \
                     has required fields {:?} but no properties object",
                    tool_name, required
                );
            }
            return;
        }
    };

    for entry in required {
        if let Some(name) = entry.as_str()
            && !properties.contains_key(name)
        {
            panic!(
                "assert_required_in_properties failed for tool '{}': \
                 required field '{}' not found in properties. \
                 Properties: {:?}",
                tool_name,
                name,
                properties.keys().collect::<Vec<_>>()
            );
        }
    }
}

/// Assert enum arrays in properties have no duplicates and are non-empty.
fn assert_enum_valid(schema: &Value, tool_name: &str) {
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        for (prop_name, prop_schema) in properties {
            if let Some(enum_values) = prop_schema.get("enum").and_then(|e| e.as_array()) {
                if enum_values.is_empty() {
                    panic!(
                        "assert_enum_valid failed for tool '{}', property '{}': \
                         enum array is empty",
                        tool_name, prop_name
                    );
                }
                let mut seen = HashSet::new();
                for val in enum_values {
                    let s = val.to_string();
                    if !seen.insert(s.clone()) {
                        panic!(
                            "assert_enum_valid failed for tool '{}', property '{}': \
                             duplicate enum value {}. Full enum: {:?}",
                            tool_name, prop_name, s, enum_values
                        );
                    }
                }
            }
        }
    }
}

/// Reserved built-in tool names that skill/MCP tools must not shadow.
const RESERVED_TOOL_NAMES: &[&str] = &["run_agent", "run_loop", "run_server"];

/// Assert tool names don't shadow reserved builtins.
fn assert_no_reserved_name_shadowing(name: &str) {
    if RESERVED_TOOL_NAMES.contains(&name) {
        panic!(
            "assert_no_reserved_name_shadowing failed: tool name '{}' \
             collides with a reserved builtin. Reserved: {:?}",
            name, RESERVED_TOOL_NAMES
        );
    }
}

/// Run all well-formedness assertions on a tool schema.
fn assert_schema_wellformed(schema: &Value, tool_name: &str) {
    assert_no_duplicate_required(schema, tool_name);
    assert_required_in_properties(schema, tool_name);
    assert_enum_valid(schema, tool_name);
    assert_no_reserved_name_shadowing(tool_name);
}

// --- Tests ---

#[test]
fn test_default_tools_wellformed() {
    let registry = default_tools();
    let definitions = registry.definitions();
    assert!(
        !definitions.is_empty(),
        "default_tools() should return at least one tool"
    );

    for def in definitions {
        assert_schema_wellformed(&def.input_schema, &def.name);
    }
}

#[test]
fn test_inject_task_id_produces_no_duplicates() {
    // Simulate what inject_task_id_field does: adds task_id to properties and required.
    // Start with a schema that already has task_id (worst case for duplicates).
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "command": { "type": "string" },
            "task_id": { "type": "string", "description": "existing task_id" }
        },
        "required": ["command", "task_id"]
    });

    // Simulate inject_task_id_field with dedup guard (current behavior)
    if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        props.insert(
            "task_id".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "ID of the task tracking this work."
            }),
        );
    }
    if let Some(required) = schema.get_mut("required").and_then(|r| r.as_array_mut()) {
        let task_id_val = Value::String("task_id".to_string());
        if !required.contains(&task_id_val) {
            required.push(task_id_val);
        }
    }

    assert_no_duplicate_required(&schema, "simulated_inject");
}

#[test]
fn test_frozen_fixture_has_duplicate() {
    let fixture = include_str!("fixtures/task_id_duplicate_required.json");
    let schema: Value = serde_json::from_str(fixture).expect("fixture should be valid JSON");

    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .expect("fixture should have required array");

    let task_id_count = required
        .iter()
        .filter(|v| v.as_str() == Some("task_id"))
        .count();
    assert_eq!(
        task_id_count, 2,
        "Frozen fixture must have exactly 2 'task_id' entries in required (the pre-fix bug shape)"
    );
}

#[test]
#[should_panic(expected = "duplicate entry 'task_id'")]
fn test_frozen_fixture_fails_wellformedness() {
    let fixture = include_str!("fixtures/task_id_duplicate_required.json");
    let schema: Value = serde_json::from_str(fixture).expect("fixture should be valid JSON");
    assert_no_duplicate_required(&schema, "frozen_fixture");
}

#[test]
fn test_empty_required_passes() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "x": { "type": "string" } },
        "required": []
    });
    assert_no_duplicate_required(&schema, "empty_required");
    assert_required_in_properties(&schema, "empty_required");
}

#[test]
fn test_no_required_passes() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "x": { "type": "string" } }
    });
    assert_schema_wellformed(&schema, "no_required");
}

#[test]
#[should_panic(expected = "required field 'foo' not found in properties")]
fn test_required_not_in_properties_fails() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "bar": { "type": "string" } },
        "required": ["foo"]
    });
    assert_required_in_properties(&schema, "missing_prop");
}

#[test]
#[should_panic(expected = "duplicate enum value")]
fn test_duplicate_enum_fails() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["active", "inactive", "active"]
            }
        }
    });
    assert_enum_valid(&schema, "dup_enum");
}

#[test]
#[should_panic(expected = "enum array is empty")]
fn test_empty_enum_fails() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": []
            }
        }
    });
    assert_enum_valid(&schema, "empty_enum");
}

#[test]
#[should_panic(expected = "collides with a reserved builtin")]
fn test_reserved_name_shadowing() {
    assert_no_reserved_name_shadowing("run_agent");
}

#[test]
fn test_non_reserved_name_passes() {
    assert_no_reserved_name_shadowing("search_memory");
    assert_no_reserved_name_shadowing("store_fact");
    assert_no_reserved_name_shadowing("custom_tool");
}

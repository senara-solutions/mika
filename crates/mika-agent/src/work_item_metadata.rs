//! Shared shallow-merge helper for work item metadata.
//!
//! Both [`crate::tools::update_work_item_status`] (agent-facing) and
//! [`crate::task_engine::dispatcher::try_extract_callback_metadata`]
//! (engine-facing) merge incoming metadata into a work item's existing
//! `metadata` JSON. They MUST share the same semantics so the agent can
//! enrich engine-injected fields without losing them.
//!
//! Semantics: **two-level shallow merge**
//! - Top-level keys from `incoming` are inserted into `base`.
//! - When both `base[k]` and `incoming[k]` are JSON objects, their inner
//!   fields are shallow-merged (incoming wins on conflict). One level only —
//!   no recursion past depth 1.
//! - All other top-level conflicts (scalar/array/type mismatch) replace the
//!   base value with the incoming value.
//!
//! See issue #489 for the bug this prevents: a single-level merge would
//! cause `{"claude_pilot": {"pr_url": "..."}}` from a later turn to wipe out
//! the engine-injected `cost_usd`, `duration_ms`, `session_id`, and `turns`
//! fields under `claude_pilot`.

use serde_json::{Map, Value};

/// Merge `incoming` into `base` with two-level shallow semantics.
///
/// `base` is mutated in place. Both arguments must be JSON objects; if
/// either is not, this function is a no-op (callers validate shape upstream).
pub fn merge_metadata(base: &mut Value, incoming: &Value) {
    let (Some(base_obj), Some(new_obj)) = (base.as_object_mut(), incoming.as_object()) else {
        return;
    };
    for (k, v) in new_obj {
        match (base_obj.get_mut(k), v) {
            (Some(Value::Object(existing_inner)), Value::Object(new_inner)) => {
                shallow_merge_object(existing_inner, new_inner);
            }
            _ => {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
}

fn shallow_merge_object(base: &mut Map<String, Value>, incoming: &Map<String, Value>) {
    for (k, v) in incoming {
        base.insert(k.clone(), v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_inner_object_fields_one_level_deep() {
        // The #489 reproduction: engine wrote claude_pilot with cost/duration/session/turns,
        // agent enriches with pr_url/branch — all six fields must survive.
        let mut base = json!({
            "claude_pilot": {
                "cost_usd": "6.465109",
                "duration_ms": 1230514,
                "session_id": "d29d3852",
                "turns": 102
            }
        });
        let incoming = json!({
            "claude_pilot": {
                "pr_url": "https://github.com/senara-solutions/mika/pull/19",
                "branch": "fix/489/x"
            }
        });
        merge_metadata(&mut base, &incoming);
        let cp = base.get("claude_pilot").unwrap().as_object().unwrap();
        assert_eq!(cp.get("cost_usd").unwrap(), "6.465109");
        assert_eq!(cp.get("duration_ms").unwrap(), 1230514);
        assert_eq!(cp.get("session_id").unwrap(), "d29d3852");
        assert_eq!(cp.get("turns").unwrap(), 102);
        assert_eq!(
            cp.get("pr_url").unwrap(),
            "https://github.com/senara-solutions/mika/pull/19"
        );
        assert_eq!(cp.get("branch").unwrap(), "fix/489/x");
    }

    #[test]
    fn inner_field_conflict_incoming_wins() {
        let mut base = json!({"claude_pilot": {"cost_usd": "6.47"}});
        let incoming = json!({"claude_pilot": {"cost_usd": "0.15"}});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base["claude_pilot"]["cost_usd"], "0.15");
    }

    #[test]
    fn new_top_level_key_added_existing_preserved() {
        let mut base = json!({"a": 1, "b": 2});
        let incoming = json!({"c": 3});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"a": 1, "b": 2, "c": 3}));
    }

    #[test]
    fn top_level_scalar_conflict_incoming_wins() {
        let mut base = json!({"status": "old"});
        let incoming = json!({"status": "new"});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"status": "new"}));
    }

    #[test]
    fn type_mismatch_existing_object_incoming_scalar_overwrites() {
        let mut base = json!({"k": {"inner": 1}});
        let incoming = json!({"k": "scalar"});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"k": "scalar"}));
    }

    #[test]
    fn type_mismatch_existing_scalar_incoming_object_overwrites() {
        let mut base = json!({"k": "scalar"});
        let incoming = json!({"k": {"inner": 1}});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"k": {"inner": 1}}));
    }

    #[test]
    fn does_not_recurse_past_one_level() {
        // Inner-of-inner: `b` is fully replaced because we only merge depth 1.
        let mut base = json!({"a": {"b": {"c": 1}}});
        let incoming = json!({"a": {"b": {"d": 2}}});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"a": {"b": {"d": 2}}}));
    }

    #[test]
    fn array_values_are_replaced_not_merged() {
        let mut base = json!({"tags": ["a", "b"]});
        let incoming = json!({"tags": ["c"]});
        merge_metadata(&mut base, &incoming);
        assert_eq!(base, json!({"tags": ["c"]}));
    }

    #[test]
    fn non_object_inputs_are_noop() {
        let mut base = json!("not an object");
        merge_metadata(&mut base, &json!({"k": "v"}));
        assert_eq!(base, json!("not an object"));

        let mut base = json!({"k": "v"});
        merge_metadata(&mut base, &json!("not an object"));
        assert_eq!(base, json!({"k": "v"}));
    }

    #[test]
    fn multiple_top_level_objects_each_merged_independently() {
        let mut base = json!({
            "claude_pilot": {"cost_usd": "1.00"},
            "github": {"pr": 19}
        });
        let incoming = json!({
            "claude_pilot": {"branch": "feat/x"},
            "github": {"checks": "passing"}
        });
        merge_metadata(&mut base, &incoming);
        assert_eq!(
            base,
            json!({
                "claude_pilot": {"cost_usd": "1.00", "branch": "feat/x"},
                "github": {"pr": 19, "checks": "passing"}
            })
        );
    }
}

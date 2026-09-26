pub mod a2a_card;
pub mod a2a_db;
pub mod agent_loop;
/// Backward-compatibility re-export. New code should use `crate::agent_loop` directly.
pub mod agent {
    pub use crate::agent_loop::*;
}
pub mod async_db;
pub mod auth_boundary_ledger;
pub mod auto_pull;
pub mod auto_pull_stop;
pub mod bundled_skills;
pub mod calibration;
pub mod canonical_tokens;
pub mod compaction;
pub mod config_keys;
pub mod db;
pub mod evidence;
pub(crate) mod github_graphql;
pub mod grooming_marker;
pub mod image_disposition;
pub mod kg;
pub mod live_pilot;
pub mod mcp;
pub mod memory;
pub mod messaging;
pub mod milestone_manager;
pub mod operational;
pub mod panic_hook;
pub mod perimeter;
pub mod pilot_egress_stamp;
pub mod planning;
pub mod post_condition;
pub mod pricing;
pub mod prompt;
pub mod qa_build_callback;
pub mod qa_review_reconcile;
pub mod ready_label;
pub mod research;
pub mod rewind;
pub mod secret_scrubber;
pub mod server;
pub mod skills;
/// Classification des fichiers source pour les gardes structurelles (mika#2321).
/// `#[cfg(test)]` : aucune garde ne tourne en production.
#[cfg(test)]
pub(crate) mod source_scan;
pub mod startup;
pub mod task_engine;
pub mod task_state;
pub mod teams;
#[cfg(test)]
pub mod test_utils;
pub mod timestamp;
pub mod tool_execution;
pub mod tools;
pub mod tracking_cleanup;
pub mod validate;
pub(crate) mod webhook_dispatch;
pub mod well_known_agents;
pub mod wip_rescue;
pub mod worktree_reaper;

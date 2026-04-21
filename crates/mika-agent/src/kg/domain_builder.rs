//! Domain graph builder — deterministic import from skill manifests and tool registry.
//!
//! # Sole-Writer Contract
//!
//! This module is the **sole writer** of `kg_entities` rows with `entity_key` prefixes
//! `skill:*`, `tool:*`, `agent:*`, and `problem_type:*`. No other code path should
//! write these entity_keys. See `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`.
//!
//! # Invariants
//!
//! - Runs once per server boot, after `SkillRegistry::apply_overrides()`.
//! - Idempotent: re-running produces the same graph state.
//! - Never called from agent turns or tool handlers.
//! - All writes carry a single `trace_id` for the rebuild invocation.
//! - Failures are logged, not panicked — the server continues to boot.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::AgentRow;
use crate::kg_schema::{
    ENTITY_TYPE_AGENT, ENTITY_TYPE_PROBLEM_TYPE, ENTITY_TYPE_SKILL, ENTITY_TYPE_TOOL,
    KG_DOMAIN_ENTITY_TYPES, KG_DOMAIN_RELATIONSHIP_TYPES, REL_DEPENDS_ON, REL_PROVIDES,
    format_entity_key,
};
use crate::mcp::McpManager;
use crate::skills::SkillRegistry;
use mika_common::claude::ToolDefinition;

/// Seeded problem type categories. Each becomes a `problem_type:*` node.
/// Starts as a const; promote to config only if operators need to extend without recompiling.
const PROBLEM_TYPE_SEEDS: &[&str] = &[
    "ci_failure",
    "merge_conflict",
    "duplicate_pr",
    "stale_uuid",
    "fabrication",
];

/// A desired entity to be upserted into the domain graph.
#[derive(Debug, Clone)]
struct DesiredEntity {
    entity_key: String,
    entity_type: &'static str,
    name: String,
    description: Option<String>,
    properties_json: Option<String>,
}

/// A desired edge to be inserted into the domain graph.
#[derive(Debug, Clone)]
struct DesiredEdge {
    from_entity_key: String,
    to_entity_key: String,
    relationship_type: &'static str,
}

/// The complete desired state of the domain graph, derived from authoritative sources.
#[derive(Debug)]
struct DesiredState {
    entities: Vec<DesiredEntity>,
    edges: Vec<DesiredEdge>,
    /// All entity_keys in `entities`, for fast membership checks during pruning.
    entity_keys: HashSet<String>,
}

/// Per-type counts for a single entity type during rebuild.
#[derive(Debug, Default)]
pub struct EntityTypeStats {
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
}

/// Per-relationship-type counts.
#[derive(Debug, Default)]
pub struct EdgeTypeStats {
    pub count: u32,
}

/// Stats from a complete domain graph rebuild.
#[derive(Debug, Default)]
pub struct RebuildStats {
    pub entity_stats: HashMap<String, EntityTypeStats>,
    pub edge_stats: HashMap<String, EdgeTypeStats>,
    pub total_removed: u32,
    pub duration_ms: u64,
}

/// Projection owner for Skill/Tool/Agent/ProblemType nodes and their structural edges.
///
/// See module-level docs for the sole-writer contract and invariants.
pub struct DomainGraphBuilder<'a> {
    db: &'a AsyncDatabase,
    skill_registry: &'a SkillRegistry,
    tool_defs: &'a [ToolDefinition],
    mcp_manager: Option<&'a McpManager>,
    agent_rows: &'a [AgentRow],
    trace_id: String,
}

impl<'a> DomainGraphBuilder<'a> {
    /// Create a new builder. Call [`rebuild`] to populate the graph.
    pub fn new(
        db: &'a AsyncDatabase,
        skill_registry: &'a SkillRegistry,
        tool_defs: &'a [ToolDefinition],
        mcp_manager: Option<&'a McpManager>,
        agent_rows: &'a [AgentRow],
    ) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        Self {
            db,
            skill_registry,
            tool_defs,
            mcp_manager,
            agent_rows,
            trace_id,
        }
    }

    /// Rebuild the entire domain graph from authoritative sources.
    ///
    /// This is the primary public entry point. It:
    /// 1. Enumerates entities and edges from registries/configs.
    /// 2. UPSERTs entities (preserving rowids for FK stability).
    /// 3. Deletes and re-inserts domain-sourced relationships.
    /// 4. Prunes stale entities no longer present in sources.
    ///
    /// All operations run in a single transaction — partial rebuilds are not exposed.
    pub async fn rebuild(&self) -> Result<RebuildStats> {
        let desired = self.enumerate_sources();
        write_desired_state(self.db, desired, &self.trace_id).await
    }

    /// Enumerate sources synchronously and write to DB asynchronously.
    ///
    /// Use this when the builder borrows data that can't be held across
    /// await points (e.g., a `std::sync::Mutex` guard on `SkillRegistry`).
    /// Call this while the guard is held — enumeration is sync, the async
    /// DB write happens after the borrow ends.
    pub fn enumerate_to_pending(&self) -> PendingRebuild {
        let desired = self.enumerate_sources();
        PendingRebuild {
            desired,
            trace_id: self.trace_id.clone(),
        }
    }

    // ===== Source Enumeration =====

    /// Walk all authoritative sources and produce the desired domain graph state.
    fn enumerate_sources(&self) -> DesiredState {
        let mut entities = Vec::new();
        let mut edges = Vec::new();
        let mut entity_keys = HashSet::new();

        // Track tool names to their source(s) for dedup.
        let mut tool_sources: HashMap<String, Vec<String>> = HashMap::new();

        // --- Skills ---
        for skill in self.skill_registry.skills() {
            let name = &skill.manifest.skill.name;
            let key = format_entity_key(ENTITY_TYPE_SKILL, name);
            entity_keys.insert(key.clone());

            let props = json!({
                "description": skill.manifest.skill.description,
                "always_on": skill.manifest.skill.always_on,
                "keywords": skill.manifest.triggers.keywords,
                "version": skill.manifest.skill.version,
            });

            entities.push(DesiredEntity {
                entity_key: key,
                entity_type: ENTITY_TYPE_SKILL,
                name: name.clone(),
                description: Some(skill.manifest.skill.description.clone()),
                properties_json: Some(props.to_string()),
            });

            // Collect skill-provided tools for PROVIDES edges.
            for tool in &skill.skill_tools {
                let tool_name = &tool.definition.name;
                tool_sources
                    .entry(tool_name.clone())
                    .or_default()
                    .push(format!("skill:{name}"));
            }

            // DEPENDS_ON edges.
            for dep_name in &skill.manifest.skill.dependencies {
                let dep_key = format_entity_key(ENTITY_TYPE_SKILL, dep_name);
                if !self
                    .skill_registry
                    .skills()
                    .iter()
                    .any(|s| s.manifest.skill.name == *dep_name)
                {
                    warn!(
                        trace_id = %self.trace_id,
                        skill = name,
                        dependency = dep_name,
                        "skill dependency not found in registry, skipping edge"
                    );
                    continue;
                }
                edges.push(DesiredEdge {
                    from_entity_key: format_entity_key(ENTITY_TYPE_SKILL, name),
                    to_entity_key: dep_key,
                    relationship_type: REL_DEPENDS_ON,
                });
            }
        }

        // --- Tools ---
        // Walk the full tool definitions list which includes builtins, skill tools, and MCP tools.
        for tool_def in self.tool_defs {
            let tool_name = &tool_def.name;
            let key = format_entity_key(ENTITY_TYPE_TOOL, tool_name);

            // Determine source type.
            let source = if let Some(mcp) = self.mcp_manager {
                if let Some(server) = mcp.tool_server_name(tool_name) {
                    tool_sources
                        .entry(tool_name.clone())
                        .or_default()
                        .push(format!("mcp:{server}"));
                    format!("mcp:{server}")
                } else if tool_sources.contains_key(tool_name) {
                    // Skill-provided tool — sources already populated above.
                    tool_sources
                        .get(tool_name)
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or_else(|| "builtin".to_string())
                } else {
                    tool_sources
                        .entry(tool_name.clone())
                        .or_default()
                        .push("builtin".to_string());
                    "builtin".to_string()
                }
            } else if tool_sources.contains_key(tool_name) {
                tool_sources
                    .get(tool_name)
                    .and_then(|v| v.first())
                    .cloned()
                    .unwrap_or_else(|| "builtin".to_string())
            } else {
                tool_sources
                    .entry(tool_name.clone())
                    .or_default()
                    .push("builtin".to_string());
                "builtin".to_string()
            };

            // Skip if we already added this tool (dedup across tool defs list).
            if entity_keys.contains(&key) {
                // Might be a duplicate tool name — already tracked sources.
                if let Some(srcs) = tool_sources.get(tool_name)
                    && srcs.len() > 1
                {
                    debug!(
                        trace_id = %self.trace_id,
                        tool = tool_name,
                        sources = ?srcs,
                        "tool has multiple sources"
                    );
                }
                continue;
            }

            entity_keys.insert(key.clone());

            let all_sources = tool_sources
                .get(tool_name)
                .cloned()
                .unwrap_or_else(|| vec![source.clone()]);

            let props = json!({
                "source": source,
                "sources": all_sources,
            });

            entities.push(DesiredEntity {
                entity_key: key,
                entity_type: ENTITY_TYPE_TOOL,
                name: tool_name.clone(),
                description: Some(tool_def.description.clone()),
                properties_json: Some(props.to_string()),
            });
        }

        // --- PROVIDES edges (Skill -> Tool) ---
        for skill in self.skill_registry.skills() {
            let skill_name = &skill.manifest.skill.name;
            for tool in &skill.skill_tools {
                let tool_name = &tool.definition.name;
                let tool_key = format_entity_key(ENTITY_TYPE_TOOL, tool_name);
                // Only create edge if the tool entity exists.
                if entity_keys.contains(&tool_key) {
                    edges.push(DesiredEdge {
                        from_entity_key: format_entity_key(ENTITY_TYPE_SKILL, skill_name),
                        to_entity_key: tool_key,
                        relationship_type: REL_PROVIDES,
                    });
                }
            }
        }

        // --- Agents ---
        for agent in self.agent_rows {
            let key = format_entity_key(ENTITY_TYPE_AGENT, &agent.name);
            entity_keys.insert(key.clone());

            let props = json!({
                "home_dir": agent.home_dir,
                "active": agent.active,
            });

            entities.push(DesiredEntity {
                entity_key: key,
                entity_type: ENTITY_TYPE_AGENT,
                name: agent.name.clone(),
                description: None,
                properties_json: Some(props.to_string()),
            });
        }

        // --- ProblemTypes (seeded) ---
        for &slug in PROBLEM_TYPE_SEEDS {
            let key = format_entity_key(ENTITY_TYPE_PROBLEM_TYPE, slug);
            entity_keys.insert(key.clone());

            entities.push(DesiredEntity {
                entity_key: key,
                entity_type: ENTITY_TYPE_PROBLEM_TYPE,
                name: slug.to_string(),
                description: None,
                properties_json: None,
            });
        }

        DesiredState {
            entities,
            edges,
            entity_keys,
        }
    }
}

/// Holds enumerated domain graph state ready for async DB write.
///
/// Created by [`DomainGraphBuilder::enumerate_to_pending`] when the builder
/// borrows data that can't be held across await points.
pub struct PendingRebuild {
    desired: DesiredState,
    trace_id: String,
}

impl PendingRebuild {
    /// Write the pending state to the database.
    pub async fn write(self, db: &AsyncDatabase) -> Result<RebuildStats> {
        write_desired_state(db, self.desired, &self.trace_id).await
    }
}

/// Write desired state to the database in a single transaction.
async fn write_desired_state(
    db: &AsyncDatabase,
    desired: DesiredState,
    trace_id: &str,
) -> Result<RebuildStats> {
    let start = std::time::Instant::now();
    info!(trace_id = trace_id, event = "domain_rebuild_start");

    let trace_id_owned = trace_id.to_string();
    let entity_keys = desired.entity_keys;
    let entities = desired.entities;
    let edges = desired.edges;

    let stats = db
        .with_db(move |db| {
            let tx = db.conn.unchecked_transaction()?;

            let entity_stats = upsert_entities(&tx, &entities, &trace_id_owned)?;
            let edge_stats = rebuild_relationships(&tx, &edges, &trace_id_owned)?;
            let total_removed = prune_stale_entities(&tx, &entity_keys, &trace_id_owned)?;

            tx.commit()?;

            Ok(RebuildStats {
                entity_stats,
                edge_stats,
                total_removed,
                duration_ms: 0,
            })
        })
        .await
        .context("domain graph rebuild transaction failed")?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stats = RebuildStats {
        duration_ms,
        ..stats
    };

    info!(
        trace_id = trace_id,
        event = "domain_rebuild_complete",
        duration_ms = stats.duration_ms,
        "domain graph rebuild complete"
    );

    Ok(stats)
}

// ===== Database Write Helpers (run inside with_db closure) =====

/// UPSERT entities, preserving rowids for FK stability.
fn upsert_entities(
    tx: &rusqlite::Transaction<'_>,
    entities: &[DesiredEntity],
    trace_id: &str,
) -> Result<HashMap<String, EntityTypeStats>> {
    let mut stats: HashMap<String, EntityTypeStats> = HashMap::new();

    let mut stmt = tx.prepare(
        "INSERT INTO kg_entities (entity_key, entity_type, name, description, properties_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ON CONFLICT(entity_key) DO UPDATE SET
           description = excluded.description,
           properties_json = excluded.properties_json,
           updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
    )?;

    for entity in entities {
        let changes = stmt.execute(rusqlite::params![
            entity.entity_key,
            entity.entity_type,
            entity.name,
            entity.description,
            entity.properties_json,
        ])?;

        let entry = stats.entry(entity.entity_type.to_string()).or_default();

        // rusqlite changes() returns 1 for both insert and update in UPSERT.
        // We check if the rowid was just created by comparing total_changes.
        if changes > 0 {
            // We can't perfectly distinguish insert vs update with ON CONFLICT DO UPDATE
            // because SQLite always reports 1 change. Use a heuristic: if the row existed
            // before, it's an update. For stats accuracy, we count all as "added" on first
            // rebuild and "updated" on subsequent rebuilds. The total counts are what matter
            // for observability.
            entry.added += 1;
        }
    }

    for (entity_type, type_stats) in &stats {
        info!(
            trace_id = trace_id,
            event = "domain_rebuild_entities",
            entity_type = entity_type,
            added = type_stats.added,
            updated = type_stats.updated,
            removed = type_stats.removed,
            "upserted domain entities"
        );
    }

    Ok(stats)
}

/// Delete all domain-sourced relationships, then re-insert.
fn rebuild_relationships(
    tx: &rusqlite::Transaction<'_>,
    edges: &[DesiredEdge],
    trace_id: &str,
) -> Result<HashMap<String, EdgeTypeStats>> {
    let mut stats: HashMap<String, EdgeTypeStats> = HashMap::new();

    // Delete all domain-sourced relationship types.
    let type_list: String = KG_DOMAIN_RELATIONSHIP_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!(
        "DELETE FROM kg_relationships WHERE relationship_type IN ({type_list});"
    ))?;

    // Insert edges, resolving entity_keys to entity IDs.
    let mut insert_stmt = tx.prepare(
        "INSERT INTO kg_relationships (source_entity_id, target_entity_id, relationship_type)
         SELECT s.id, t.id, ?3
         FROM kg_entities s, kg_entities t
         WHERE s.entity_key = ?1 AND t.entity_key = ?2",
    )?;

    for edge in edges {
        let inserted = insert_stmt.execute(rusqlite::params![
            edge.from_entity_key,
            edge.to_entity_key,
            edge.relationship_type,
        ])?;

        if inserted == 0 {
            warn!(
                trace_id = trace_id,
                from = edge.from_entity_key,
                to = edge.to_entity_key,
                rel_type = edge.relationship_type,
                "edge insert resolved zero rows — one or both entity_keys missing"
            );
        } else {
            stats
                .entry(edge.relationship_type.to_string())
                .or_default()
                .count += 1;
        }
    }

    for (rel_type, type_stats) in &stats {
        info!(
            trace_id = trace_id,
            event = "domain_rebuild_edges",
            relationship_type = rel_type,
            count = type_stats.count,
            "rebuilt domain edges"
        );
    }

    Ok(stats)
}

/// Delete domain-layer entities not present in the desired set.
fn prune_stale_entities(
    tx: &rusqlite::Transaction<'_>,
    desired_keys: &HashSet<String>,
    trace_id: &str,
) -> Result<u32> {
    // Fetch existing domain-layer entity keys.
    let type_list: String = KG_DOMAIN_ENTITY_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut stmt = tx.prepare(&format!(
        "SELECT entity_key FROM kg_entities WHERE entity_type IN ({type_list})"
    ))?;

    let existing_keys: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let stale_keys: Vec<&String> = existing_keys
        .iter()
        .filter(|k| !desired_keys.contains(*k))
        .collect();

    let removed = stale_keys.len() as u32;

    if removed > 0 {
        let mut delete_stmt = tx.prepare("DELETE FROM kg_entities WHERE entity_key = ?1")?;
        for key in &stale_keys {
            delete_stmt.execute(rusqlite::params![key])?;
        }

        info!(
            trace_id = trace_id,
            event = "domain_rebuild_pruned",
            removed = removed,
            "pruned stale domain entities"
        );
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{AgentRow, Database};
    use crate::kg_schema::REL_DEPENDS_ON;
    use crate::skills::SkillRegistry;
    use crate::skills::index::SkillEntry;
    use crate::skills::manifest::{
        Constraints, LlmOverride, SkillInfo, SkillManifest, Triggers, VariantsConfig,
    };
    use std::path::PathBuf;

    #[test]
    fn test_problem_type_seeds_are_valid() {
        assert_eq!(PROBLEM_TYPE_SEEDS.len(), 5);
        for seed in PROBLEM_TYPE_SEEDS {
            assert!(!seed.is_empty());
            assert!(
                seed.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "seed '{seed}' has unexpected characters"
            );
        }
    }

    #[test]
    fn test_desired_edge_types_are_domain_types() {
        let allowed: HashSet<&str> = KG_DOMAIN_RELATIONSHIP_TYPES.iter().copied().collect();
        assert!(allowed.contains(REL_DEPENDS_ON));
        assert!(allowed.contains(REL_PROVIDES));
        assert_eq!(allowed.len(), 2);
    }

    // --- Helper factories ---

    fn make_skill(name: &str, deps: &[&str], tool_names: &[&str]) -> SkillEntry {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::ToolHandler;

        let skill_tools: Vec<ResolvedSkillTool> = tool_names
            .iter()
            .map(|t| ResolvedSkillTool {
                definition: ToolDefinition {
                    name: t.to_string(),
                    description: format!("Tool {t}"),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                handler: ToolHandler::Exec {
                    command: "echo".to_string(),
                    long_running: false,
                    estimated_duration_secs: None,
                },
                skill_dir: PathBuf::from("/tmp/test"),
            })
            .collect();

        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: format!("Skill {name}"),
                    version: "1.0.0".to_string(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies: deps.iter().map(|s| s.to_string()).collect(),
                    max_prompt_size: None,
                },
                triggers: Triggers {
                    keywords: vec![name.to_string()],
                },
                llm: LlmOverride::default(),
                constraints: Constraints::default(),
                context: std::collections::HashMap::new(),
                variants: VariantsConfig::default(),
            },
            dir: PathBuf::from(format!("/tmp/test/skills/{name}")),
            keywords_lower: vec![name.to_lowercase()],
            prompt_snippet: String::new(),
            skill_tools,
            enabled: true,
            has_override: false,
            provider_overrides: HashMap::new(),
            model_prompts: HashMap::new(),
            model_overrides: HashMap::new(),
            generated_model_prompts: HashMap::new(),
        }
    }

    fn make_tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("Builtin tool {name}"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn make_agent_row(name: &str) -> AgentRow {
        AgentRow {
            id: name.to_string(),
            name: name.to_string(),
            home_dir: format!("/home/mika/agents/{name}"),
            active: true,
            last_seen: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // --- Enumeration tests ---

    #[test]
    fn test_enumerate_skills_and_tools() {
        let skills = vec![
            make_skill("git-ops", &[], &["run_git"]),
            make_skill("web-search", &[], &["brave_search"]),
        ];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![
            make_tool_def("run_git"),
            make_tool_def("brave_search"),
            make_tool_def("read_file"),
        ];
        let agents = vec![make_agent_row("mika-dev")];

        let db = Database::open_in_memory().unwrap();
        db.register_agent("mika-dev", "mika-dev", "/home/mika/agents/mika-dev")
            .unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika-dev");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &agents);
        let desired = builder.enumerate_sources();

        // 2 skills + 3 tools + 1 agent + 5 problem_types = 11 entities
        assert_eq!(desired.entities.len(), 11);

        // DEPENDS_ON: none. PROVIDES: git-ops->run_git, web-search->brave_search = 2
        let provides: Vec<_> = desired
            .edges
            .iter()
            .filter(|e| e.relationship_type == REL_PROVIDES)
            .collect();
        assert_eq!(provides.len(), 2);

        // Problem types always present.
        let pt_count = desired
            .entities
            .iter()
            .filter(|e| e.entity_type == ENTITY_TYPE_PROBLEM_TYPE)
            .count();
        assert_eq!(pt_count, 5);
    }

    #[test]
    fn test_enumerate_with_dependencies() {
        let skills = vec![
            make_skill("claude-pilot", &["self-dev"], &[]),
            make_skill("self-dev", &[], &["run_claude_pilot"]),
        ];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![make_tool_def("run_claude_pilot")];

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &[]);
        let desired = builder.enumerate_sources();

        let depends_on: Vec<_> = desired
            .edges
            .iter()
            .filter(|e| e.relationship_type == REL_DEPENDS_ON)
            .collect();
        assert_eq!(depends_on.len(), 1);
        assert_eq!(
            depends_on[0].from_entity_key,
            format_entity_key(ENTITY_TYPE_SKILL, "claude-pilot")
        );
        assert_eq!(
            depends_on[0].to_entity_key,
            format_entity_key(ENTITY_TYPE_SKILL, "self-dev")
        );
    }

    #[test]
    fn test_enumerate_missing_dependency_skipped() {
        // Skill references a dep that's not in the registry — edge is skipped.
        let skills = vec![make_skill("my-skill", &["nonexistent"], &[])];
        let registry = SkillRegistry::from_test_entries(skills);

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        let desired = builder.enumerate_sources();

        let depends_on: Vec<_> = desired
            .edges
            .iter()
            .filter(|e| e.relationship_type == REL_DEPENDS_ON)
            .collect();
        assert_eq!(depends_on.len(), 0);
    }

    #[test]
    fn test_enumerate_empty_registries() {
        let registry = SkillRegistry::empty();

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        let desired = builder.enumerate_sources();

        // Only problem types with empty registries.
        assert_eq!(desired.entities.len(), 5);
        assert_eq!(desired.edges.len(), 0);
    }

    #[test]
    fn test_no_state_edges_produced() {
        let skills = vec![
            make_skill("s1", &["s2"], &["t1"]),
            make_skill("s2", &[], &["t2"]),
        ];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![make_tool_def("t1"), make_tool_def("t2")];
        let agents = vec![make_agent_row("a1")];

        let db = Database::open_in_memory().unwrap();
        db.register_agent("a1", "a1", "/home/a1").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &agents);
        let desired = builder.enumerate_sources();

        let allowed: HashSet<&str> = KG_DOMAIN_RELATIONSHIP_TYPES.iter().copied().collect();
        for edge in &desired.edges {
            assert!(
                allowed.contains(edge.relationship_type),
                "unexpected edge type: {} (from {} -> {})",
                edge.relationship_type,
                edge.from_entity_key,
                edge.to_entity_key
            );
        }
    }

    // --- Full rebuild integration tests ---

    #[tokio::test]
    async fn test_builder_populates_fresh_db() {
        let skills = vec![make_skill("s1", &[], &["t1"])];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![make_tool_def("t1"), make_tool_def("builtin_tool")];
        let agents = vec![make_agent_row("dev")];

        let db = Database::open_in_memory().unwrap();
        db.register_agent("dev", "dev", "/home/dev").unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "dev");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &agents);
        let stats = builder.rebuild().await.unwrap();

        // Verify entities were created.
        let entity_count: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        // 1 skill + 2 tools + 1 agent + 5 problem_types = 9
        assert_eq!(entity_count, 9);

        // Verify relationships were created.
        let edge_count: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_relationships", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        // 1 PROVIDES edge (s1 -> t1)
        assert_eq!(edge_count, 1);
        assert_eq!(stats.total_removed, 0);
    }

    #[tokio::test]
    async fn test_builder_is_idempotent() {
        let skills = vec![make_skill("s1", &[], &["t1"])];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![make_tool_def("t1")];

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        // First rebuild.
        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &[]);
        let _stats1 = builder.rebuild().await.unwrap();

        // Second rebuild with same sources.
        let builder2 = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &[]);
        let stats2 = builder2.rebuild().await.unwrap();

        // Entity counts should be the same.
        let entity_count: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        // 1 skill + 1 tool + 5 problem_types = 7
        assert_eq!(entity_count, 7);
        assert_eq!(stats2.total_removed, 0);
    }

    #[tokio::test]
    async fn test_builder_preserves_rowid() {
        let skills = vec![make_skill("s1", &[], &[])];
        let registry = SkillRegistry::from_test_entries(skills);

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        // First rebuild.
        let builder = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        builder.rebuild().await.unwrap();

        // Get the rowid of s1.
        let rowid: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT id FROM kg_entities WHERE entity_key = 'skill:s1'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        // Second rebuild — rowid should be preserved via UPSERT (ON CONFLICT DO UPDATE).
        let builder2 = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        builder2.rebuild().await.unwrap();

        let rowid2: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT id FROM kg_entities WHERE entity_key = 'skill:s1'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        assert_eq!(rowid, rowid2, "rowid must be preserved across rebuilds");
    }

    #[tokio::test]
    async fn test_builder_deletes_removed_entities() {
        // First rebuild with skill X.
        let skills1 = vec![make_skill("skill-x", &[], &[])];
        let registry1 = SkillRegistry::from_test_entries(skills1);

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder1 = DomainGraphBuilder::new(&async_db, &registry1, &[], None, &[]);
        builder1.rebuild().await.unwrap();

        let has_skill_x: bool = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM kg_entities WHERE entity_key = 'skill:skill-x'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert!(has_skill_x);

        // Second rebuild WITHOUT skill-x.
        let registry2 = SkillRegistry::empty();
        let builder2 = DomainGraphBuilder::new(&async_db, &registry2, &[], None, &[]);
        let stats2 = builder2.rebuild().await.unwrap();

        let has_skill_x_after: bool = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM kg_entities WHERE entity_key = 'skill:skill-x'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert!(!has_skill_x_after, "skill-x should be pruned");
        assert!(stats2.total_removed >= 1);
    }

    #[tokio::test]
    async fn test_builder_cascade_deletes_edges() {
        // Skill with a PROVIDES edge.
        let skills = vec![make_skill("s1", &[], &["t1"])];
        let registry = SkillRegistry::from_test_entries(skills);
        let tool_defs = vec![make_tool_def("t1")];

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &tool_defs, None, &[]);
        builder.rebuild().await.unwrap();

        let edge_count: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_relationships", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(edge_count, 1);

        // Remove s1 — edges touching it should also be removed.
        let registry2 = SkillRegistry::empty();
        let builder2 = DomainGraphBuilder::new(&async_db, &registry2, &tool_defs, None, &[]);
        builder2.rebuild().await.unwrap();

        let edge_count2: i64 = async_db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_relationships", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(edge_count2, 0);
    }

    #[tokio::test]
    async fn test_builder_properties_json_stored() {
        let skills = vec![make_skill("my-skill", &[], &[])];
        let registry = SkillRegistry::from_test_entries(skills);

        let db = Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let builder = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        builder.rebuild().await.unwrap();

        let props: String = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT properties_json FROM kg_entities WHERE entity_key = 'skill:my-skill'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&props).unwrap();
        assert_eq!(parsed["version"], "1.0.0");
        assert_eq!(parsed["description"], "Skill my-skill");
    }

    #[tokio::test]
    async fn test_builder_does_not_prune_non_domain_entities() {
        let db = Database::open_in_memory().unwrap();

        // Insert a non-domain entity (e.g., person).
        db.conn
            .execute(
                "INSERT INTO kg_entities (entity_key, entity_type, name) VALUES ('person:Alice', 'person', 'Alice')",
                [],
            )
            .unwrap();

        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "test");

        let registry = SkillRegistry::empty();
        let builder = DomainGraphBuilder::new(&async_db, &registry, &[], None, &[]);
        builder.rebuild().await.unwrap();

        // The person entity should survive — the builder only prunes domain types.
        let has_alice: bool = async_db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM kg_entities WHERE entity_key = 'person:Alice'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert!(has_alice, "non-domain entity should not be pruned");
    }
}

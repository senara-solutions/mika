//! # Domain Graph Builder
//!
//! Deterministically populates [`kg_entities`] and [`kg_relationships`] by
//! enumerating four authoritative sources at server startup:
//!
//! - [`SkillRegistry`](crate::skills::SkillRegistry) — skill manifests
//! - [`ToolRegistry`](crate::tools::ToolRegistry) — builtin + skill-owned tools
//! - [`McpManager`](crate::mcp::McpManager) — MCP server tools (as-of-boot)
//! - Agent configs — discovered agent names and metadata
//!
//! ## Sole-Writer Contract
//!
//! This module is the **sole writer** of entity_keys in the `skill:*`, `tool:*`,
//! `agent:*`, and `problem_type:*` namespaces. No other code path writes these
//! entity_keys. See `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`.
//!
//! ## Invariants
//!
//! - Runs once per server boot, after `SkillRegistry::apply_overrides()`.
//! - Idempotent: re-running produces the same graph state.
//! - Never called from agent turns or tool handlers.
//! - All writes carry a single `trace_id` for the rebuild invocation.
//! - Rebuild failures are logged, not panicked — the server continues to boot.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::kg_schema::{KG_DOMAIN_ENTITY_TYPES, format_entity_key};
use crate::mcp::McpManager;
use crate::skills::SkillRegistry;
use crate::timestamp;
use crate::tools::ToolRegistry;

/// Seed list for well-known problem types.
///
/// Starts as a const; promote to a config file only if operators need to extend
/// it without recompiling.
const PROBLEM_TYPE_SEEDS: &[&str] = &[
    "ci_failure",
    "merge_conflict",
    "duplicate_pr",
    "stale_uuid",
    "fabrication",
];

/// Domain relationship types managed by this builder.
///
/// Used to scope DELETE operations during rebuild — only these types are
/// touched; subject/resolution layer edges are left untouched.
///
/// Convention: subject-layer writers (#690/#691) use distinct edge types
/// (e.g., `SOLVED_BY`, `CAUSES`, `INDICATES`) — not `DEPENDS_ON` or
/// `PROVIDES`. The DELETE scope is intentionally type-based (not
/// endpoint-based) for simplicity. If a future writer needs these same
/// type names on subject entities, scope the DELETE to domain endpoints.
const DOMAIN_RELATIONSHIP_TYPES: &[&str] = &["DEPENDS_ON", "PROVIDES"];

/// An entity that should exist in the graph after rebuild.
#[derive(Debug, Clone)]
struct DesiredEntity {
    entity_key: String,
    entity_type: String,
    name: String,
    properties_json: Option<String>,
}

/// An edge that should exist in the graph after rebuild.
#[derive(Debug, Clone)]
struct DesiredEdge {
    from_entity_key: String,
    to_entity_key: String,
    edge_type: String,
    properties_json: Option<String>,
}

/// Complete desired state after enumerating all sources.
struct DesiredState {
    entities: Vec<DesiredEntity>,
    edges: Vec<DesiredEdge>,
    entity_keys: HashSet<String>,
}

/// Per-type entity stats from a rebuild.
#[derive(Debug, Default)]
struct EntityTypeStats {
    added: usize,
    updated: usize,
}

/// Per-type edge stats from a rebuild.
#[derive(Debug, Default)]
struct EdgeTypeStats {
    count: usize,
}

/// Aggregate rebuild statistics.
#[derive(Debug, Default)]
pub struct RebuildStats {
    pub entities_added: usize,
    pub entities_updated: usize,
    pub entities_removed: usize,
    pub edges_depends_on: usize,
    pub edges_provides: usize,
    pub duration_ms: u128,
}

/// Information about an agent to include in the domain graph.
///
/// Kept minimal — the domain graph records structural facts, not state.
pub struct AgentInfo {
    pub name: String,
    pub role: Option<String>,
    pub model: Option<String>,
}

/// Projection owner for Skill/Tool/Agent/ProblemType nodes and their
/// structural edges.
pub struct DomainGraphBuilder<'a> {
    db: &'a AsyncDatabase,
    skill_registry: &'a SkillRegistry,
    tool_registry: &'a ToolRegistry,
    mcp_manager: Option<&'a McpManager>,
    agent_infos: &'a [AgentInfo],
    trace_id: String,
}

impl<'a> DomainGraphBuilder<'a> {
    /// Create a new builder.
    pub fn new(
        db: &'a AsyncDatabase,
        skill_registry: &'a SkillRegistry,
        tool_registry: &'a ToolRegistry,
        mcp_manager: Option<&'a McpManager>,
        agent_infos: &'a [AgentInfo],
    ) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        Self {
            db,
            skill_registry,
            tool_registry,
            mcp_manager,
            agent_infos,
            trace_id,
        }
    }

    /// Run the full rebuild: enumerate → upsert → rebuild edges → prune stale.
    ///
    /// The entire operation runs in a single transaction. If any step fails,
    /// the whole rebuild rolls back and the graph remains in its previous state.
    pub async fn rebuild(&self) -> Result<RebuildStats> {
        let start = Instant::now();
        info!(trace_id = %self.trace_id, event = "domain_rebuild_start");

        // 1. Gather desired state from authoritative sources.
        let desired = self.enumerate_sources();

        // 2. Execute all writes in a single transaction via AsyncDatabase.
        let trace_id = self.trace_id.clone();
        let stats = self
            .db
            .with_db(move |db| {
                let conn = &db.conn;

                // foreign_keys = ON is set at connection open time (Database::open).
                let tx = conn.unchecked_transaction()?;

                // 2a. Collect existing entity keys for insert-vs-update tracking.
                let mut existing_keys: HashSet<String> = HashSet::new();
                {
                    let type_placeholders: String = KG_DOMAIN_ENTITY_TYPES
                        .iter()
                        .map(|_| "?")
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT entity_key FROM kg_entities WHERE type IN ({type_placeholders})"
                    );
                    let mut stmt = tx.prepare(&sql)?;
                    let mut param_idx = 1;
                    for t in KG_DOMAIN_ENTITY_TYPES {
                        stmt.raw_bind_parameter(param_idx, *t)?;
                        param_idx += 1;
                    }
                    let mut rows = stmt.raw_query();
                    while let Some(row) = rows.next()? {
                        let key: String = row.get(0)?;
                        existing_keys.insert(key);
                    }
                }

                // UPSERT entities
                let mut type_stats: HashMap<String, EntityTypeStats> = HashMap::new();
                let now = timestamp::now();
                for entity in &desired.entities {
                    tx.execute(
                        "INSERT INTO kg_entities (entity_key, type, name, properties_json, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                         ON CONFLICT(entity_key) DO UPDATE SET
                           name = excluded.name,
                           properties_json = excluded.properties_json,
                           updated_at = ?5",
                        rusqlite::params![
                            entity.entity_key,
                            entity.entity_type,
                            entity.name,
                            entity.properties_json,
                            now,
                        ],
                    )?;

                    let stats = type_stats
                        .entry(entity.entity_type.clone())
                        .or_default();

                    if existing_keys.contains(&entity.entity_key) {
                        stats.updated += 1;
                    } else {
                        stats.added += 1;
                    }
                }

                // 2b. DELETE domain-sourced relationships, then re-INSERT.
                let mut edge_stats: HashMap<String, EdgeTypeStats> = HashMap::new();
                for rel_type in DOMAIN_RELATIONSHIP_TYPES {
                    tx.execute(
                        "DELETE FROM kg_relationships WHERE type = ?1",
                        rusqlite::params![rel_type],
                    )?;
                }

                for edge in &desired.edges {
                    // Look up entity IDs by key. Skip edges with missing endpoints
                    // (the enumeration already filters these, but defense-in-depth).
                    let from_id: Option<i64> = tx
                        .query_row(
                            "SELECT id FROM kg_entities WHERE entity_key = ?1",
                            rusqlite::params![edge.from_entity_key],
                            |row| row.get(0),
                        )
                        .ok();
                    let to_id: Option<i64> = tx
                        .query_row(
                            "SELECT id FROM kg_entities WHERE entity_key = ?1",
                            rusqlite::params![edge.to_entity_key],
                            |row| row.get(0),
                        )
                        .ok();

                    match (from_id, to_id) {
                        (Some(fid), Some(tid)) => {
                            tx.execute(
                                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type, properties_json, created_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5)",
                                rusqlite::params![fid, tid, edge.edge_type, edge.properties_json, now],
                            )?;
                            edge_stats
                                .entry(edge.edge_type.clone())
                                .or_default()
                                .count += 1;
                        }
                        _ => {
                            tracing::warn!(
                                trace_id = %trace_id,
                                from = %edge.from_entity_key,
                                to = %edge.to_entity_key,
                                edge_type = %edge.edge_type,
                                "skipping edge with missing endpoint entity"
                            );
                        }
                    }
                }

                // 2c. DELETE entities no longer in sources.
                // The type filter enforces the sole-writer contract at the SQL level.
                let type_placeholders: String = KG_DOMAIN_ENTITY_TYPES
                    .iter()
                    .map(|_| "?")
                    .collect::<Vec<_>>()
                    .join(", ");

                // Build the NOT IN clause for desired keys
                let key_placeholders: String = if desired.entity_keys.is_empty() {
                    // No desired keys — delete everything in domain types
                    String::new()
                } else {
                    let placeholders: String = desired
                        .entity_keys
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", i + KG_DOMAIN_ENTITY_TYPES.len() + 1))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(" AND entity_key NOT IN ({placeholders})")
                };

                let delete_sql = format!(
                    "DELETE FROM kg_entities WHERE type IN ({type_placeholders}){key_placeholders}"
                );

                let removed = {
                    let mut stmt = tx.prepare(&delete_sql)?;
                    let mut param_idx = 1;
                    for t in KG_DOMAIN_ENTITY_TYPES {
                        stmt.raw_bind_parameter(param_idx, *t)?;
                        param_idx += 1;
                    }
                    for key in &desired.entity_keys {
                        stmt.raw_bind_parameter(param_idx, key.as_str())?;
                        param_idx += 1;
                    }
                    stmt.raw_execute()?
                };

                tx.commit()?;

                // Compute aggregate stats
                let mut total_added = 0usize;
                let mut total_updated = 0usize;
                for s in type_stats.values() {
                    total_added += s.added;
                    total_updated += s.updated;
                }

                Ok((total_added, total_updated, removed, type_stats, edge_stats))
            })
            .await?;

        let (total_added, total_updated, removed, type_stats, edge_stats) = stats;
        let duration_ms = start.elapsed().as_millis();

        // Log per-type stats
        for entity_type in KG_DOMAIN_ENTITY_TYPES {
            let s = type_stats.get(*entity_type);
            let added = s.map_or(0, |s| s.added);
            let updated = s.map_or(0, |s| s.updated);
            info!(
                trace_id = %self.trace_id,
                event = "domain_rebuild_entities",
                r#type = entity_type,
                added,
                updated,
            );
        }
        if removed > 0 {
            info!(
                trace_id = %self.trace_id,
                event = "domain_rebuild_entities_removed",
                removed,
            );
        }
        for rel_type in DOMAIN_RELATIONSHIP_TYPES {
            let count = edge_stats.get(*rel_type).map_or(0, |s| s.count);
            info!(
                trace_id = %self.trace_id,
                event = "domain_rebuild_edges",
                r#type = rel_type,
                count,
            );
        }
        info!(
            trace_id = %self.trace_id,
            event = "domain_rebuild_complete",
            duration_ms,
        );

        Ok(RebuildStats {
            entities_added: total_added,
            entities_updated: total_updated,
            entities_removed: removed,
            edges_depends_on: edge_stats.get("DEPENDS_ON").map_or(0, |s| s.count),
            edges_provides: edge_stats.get("PROVIDES").map_or(0, |s| s.count),
            duration_ms,
        })
    }

    /// Enumerate all authoritative sources into a desired state.
    fn enumerate_sources(&self) -> DesiredState {
        let mut entities = Vec::new();
        let mut edges = Vec::new();
        let mut entity_keys = HashSet::new();

        // Track tool sources for dedup (tool_name -> list of sources)
        let mut tool_sources: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        let mut tool_descriptions: HashMap<String, String> = HashMap::new();

        // --- Skills ---
        for skill in self.skill_registry.skills() {
            let skill_name = &skill.manifest.skill.name;
            let key = format_entity_key("skill", skill_name);

            let props = json!({
                "description": skill.manifest.skill.description,
                "always_on": skill.manifest.skill.always_on,
                "keywords": skill.manifest.triggers.keywords,
                "version": skill.manifest.skill.version,
            });

            entities.push(DesiredEntity {
                entity_key: key.clone(),
                entity_type: "skill".to_string(),
                name: skill_name.clone(),
                properties_json: Some(props.to_string()),
            });
            entity_keys.insert(key.clone());

            // Collect DEPENDS_ON edges
            for dep in &skill.manifest.skill.dependencies {
                let dep_key = format_entity_key("skill", dep);
                // Only create the edge if the dependency exists in the registry
                let dep_exists = self
                    .skill_registry
                    .skills()
                    .iter()
                    .any(|s| s.manifest.skill.name == *dep);
                if dep_exists {
                    edges.push(DesiredEdge {
                        from_entity_key: key.clone(),
                        to_entity_key: dep_key,
                        edge_type: "DEPENDS_ON".to_string(),
                        properties_json: None,
                    });
                } else {
                    warn!(
                        trace_id = %self.trace_id,
                        skill = skill_name,
                        dependency = dep,
                        "skill references unknown dependency, skipping DEPENDS_ON edge"
                    );
                }
            }

            // Collect PROVIDES edges for skill tools
            for tool in &skill.skill_tools {
                let tool_name = &tool.definition.name;

                // Track source for dedup
                tool_sources
                    .entry(tool_name.clone())
                    .or_default()
                    .push(json!({"skill": skill_name}));
                tool_descriptions
                    .entry(tool_name.clone())
                    .or_insert_with(|| tool.definition.description.clone());

                // PROVIDES edge
                let tool_key = format_entity_key("tool", tool_name);
                edges.push(DesiredEdge {
                    from_entity_key: key.clone(),
                    to_entity_key: tool_key,
                    edge_type: "PROVIDES".to_string(),
                    properties_json: None,
                });
            }
        }

        // --- Tools from ToolRegistry (builtins) ---
        for tool_def in self.tool_registry.definitions() {
            let tool_name = &tool_def.name;

            // Determine source: if already tracked from a skill, it's skill-owned.
            // Otherwise it's a builtin.
            if !tool_sources.contains_key(tool_name) {
                tool_sources
                    .entry(tool_name.clone())
                    .or_default()
                    .push(json!({"builtin": true}));
                tool_descriptions
                    .entry(tool_name.clone())
                    .or_insert_with(|| tool_def.description.clone());
            }
        }

        // --- Tools from MCP servers ---
        if let Some(mcp) = self.mcp_manager {
            for tool_def in mcp.tool_definitions() {
                let tool_name = &tool_def.name;

                // Extract MCP server name from the namespaced tool name (mcp__{server}__{tool})
                let server_name = tool_name
                    .strip_prefix("mcp__")
                    .and_then(|rest| rest.split("__").next())
                    .unwrap_or("unknown");

                tool_sources
                    .entry(tool_name.clone())
                    .or_default()
                    .push(json!({"mcp": server_name}));
                tool_descriptions
                    .entry(tool_name.clone())
                    .or_insert_with(|| tool_def.description.clone());
            }
        }

        // --- Build Tool entities from collected sources ---
        for (tool_name, sources) in &tool_sources {
            let key = format_entity_key("tool", tool_name);
            let description = tool_descriptions
                .get(tool_name)
                .cloned()
                .unwrap_or_default();

            // Detect duplicate tool with differing descriptions
            if sources.len() > 1 {
                debug!(
                    trace_id = %self.trace_id,
                    tool = tool_name,
                    sources = ?sources,
                    "tool exposed by multiple sources"
                );
            }

            let source_label = if sources.len() == 1 {
                // Single source — use a simple string label
                if sources[0].get("builtin").is_some() {
                    "builtin".to_string()
                } else if let Some(skill) = sources[0].get("skill").and_then(|v| v.as_str()) {
                    format!("skill:{skill}")
                } else if let Some(mcp) = sources[0].get("mcp").and_then(|v| v.as_str()) {
                    format!("mcp:{mcp}")
                } else {
                    "unknown".to_string()
                }
            } else {
                // Multiple sources — record all
                "multiple".to_string()
            };

            let props = json!({
                "description": description,
                "source": source_label,
                "sources": sources,
            });

            entities.push(DesiredEntity {
                entity_key: key.clone(),
                entity_type: "tool".to_string(),
                name: tool_name.clone(),
                properties_json: Some(props.to_string()),
            });
            entity_keys.insert(key);
        }

        // --- Agents ---
        for agent in self.agent_infos {
            let key = format_entity_key("agent", &agent.name);

            let mut props = serde_json::Map::new();
            if let Some(ref role) = agent.role {
                props.insert("role".to_string(), json!(role));
            }
            if let Some(ref model) = agent.model {
                props.insert("model".to_string(), json!(model));
            }

            entities.push(DesiredEntity {
                entity_key: key.clone(),
                entity_type: "agent".to_string(),
                name: agent.name.clone(),
                properties_json: Some(serde_json::Value::Object(props).to_string()),
            });
            entity_keys.insert(key);
        }

        // --- ProblemType seeds ---
        for slug in PROBLEM_TYPE_SEEDS {
            let key = format_entity_key("problem_type", slug);
            entities.push(DesiredEntity {
                entity_key: key.clone(),
                entity_type: "problem_type".to_string(),
                name: slug.to_string(),
                properties_json: None,
            });
            entity_keys.insert(key);
        }

        DesiredState {
            entities,
            edges,
            entity_keys,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::skills::index::SkillEntry;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};
    use mika_common::claude::ToolDefinition;
    use std::path::PathBuf;

    /// Create a minimal SkillEntry for testing.
    fn make_skill(
        name: &str,
        description: &str,
        dependencies: Vec<String>,
        tool_names: Vec<&str>,
    ) -> SkillEntry {
        use crate::skills::index::ResolvedSkillTool;
        use crate::skills::manifest::{Constraints, VariantsConfig};

        let skill_tools: Vec<ResolvedSkillTool> = tool_names
            .into_iter()
            .map(|t| ResolvedSkillTool {
                definition: ToolDefinition {
                    name: t.to_string(),
                    description: format!("Tool {t}"),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                handler: crate::skills::manifest::ToolHandler::Builtin {
                    function: t.to_string(),
                },
                skill_dir: PathBuf::new(),
            })
            .collect();

        SkillEntry {
            manifest: SkillManifest {
                skill: SkillInfo {
                    name: name.to_string(),
                    description: description.to_string(),
                    version: "0.1.0".to_string(),
                    always_on: false,
                    timeout_secs: 30,
                    dependencies,
                    max_prompt_size: None,
                },
                triggers: Triggers {
                    keywords: vec![name.to_string()],
                },
                llm: Default::default(),
                constraints: Constraints::default(),
                context: Default::default(),
                variants: VariantsConfig::default(),
            },
            dir: PathBuf::from(format!("/tmp/skills/{name}")),
            keywords_lower: vec![name.to_lowercase()],
            prompt_snippet: String::new(),
            skill_tools,
            enabled: true,
            has_override: false,
            provider_overrides: Default::default(),
            model_prompts: Default::default(),
            model_overrides: Default::default(),
            generated_model_prompts: Default::default(),
        }
    }

    fn make_tool_registry(names: &[&str]) -> ToolRegistry {
        use crate::tools::ToolOutput;
        use async_trait::async_trait;

        struct DummyTool {
            name: String,
            def: ToolDefinition,
        }

        #[async_trait]
        impl crate::tools::Tool for DummyTool {
            fn name(&self) -> &str {
                &self.name
            }
            fn definition(&self) -> ToolDefinition {
                self.def.clone()
            }
            async fn execute(
                &self,
                _input: serde_json::Value,
                _ctx: &crate::tools::ToolContext<'_>,
            ) -> Result<ToolOutput> {
                Ok(ToolOutput::success("ok"))
            }
        }

        let mut registry = ToolRegistry::new();
        for name in names {
            registry.register(Box::new(DummyTool {
                name: name.to_string(),
                def: ToolDefinition {
                    name: name.to_string(),
                    description: format!("Builtin tool {name}"),
                    input_schema: serde_json::json!({"type": "object"}),
                },
            }));
        }
        registry
    }

    fn make_async_db() -> AsyncDatabase {
        let db = Database::open_in_memory().expect("in-memory DB");
        AsyncDatabase::new(db)
    }

    #[test]
    fn enumerate_skills() {
        let db = make_async_db();
        let skill_a = make_skill("skill-a", "Skill A", vec![], vec!["tool_x"]);
        let skill_b = make_skill(
            "skill-b",
            "Skill B",
            vec!["skill-a".to_string()],
            vec!["tool_y"],
        );
        let registry = SkillRegistry::from_test_entries(vec![skill_a, skill_b]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        // 2 skills + 2 tools + 0 agents + 5 problem_types = 9
        assert_eq!(state.entities.len(), 9);

        // 1 DEPENDS_ON + 2 PROVIDES = 3
        assert_eq!(state.edges.len(), 3);

        let edge_types: HashSet<_> = state.edges.iter().map(|e| e.edge_type.as_str()).collect();
        assert!(edge_types.contains("DEPENDS_ON"));
        assert!(edge_types.contains("PROVIDES"));
    }

    #[test]
    fn enumerate_deduplicates_tools() {
        let db = make_async_db();
        // Two skills expose the same tool
        let skill_a = make_skill("skill-a", "A", vec![], vec!["shared_tool"]);
        let skill_b = make_skill("skill-b", "B", vec![], vec!["shared_tool"]);
        let registry = SkillRegistry::from_test_entries(vec![skill_a, skill_b]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        // Should be ONE tool entity for "shared_tool"
        let tool_entities: Vec<_> = state
            .entities
            .iter()
            .filter(|e| e.entity_type == "tool" && e.name == "shared_tool")
            .collect();
        assert_eq!(tool_entities.len(), 1);

        // Properties should list both sources
        let props: serde_json::Value =
            serde_json::from_str(tool_entities[0].properties_json.as_ref().unwrap()).unwrap();
        let sources = props["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn enumerate_with_no_dependencies() {
        let db = make_async_db();
        let skill = make_skill("solo", "Solo skill", vec![], vec![]);
        let registry = SkillRegistry::from_test_entries(vec![skill]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        // No DEPENDS_ON edges
        let depends_on: Vec<_> = state
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEPENDS_ON")
            .collect();
        assert!(depends_on.is_empty());
    }

    #[test]
    fn enumerate_with_empty_agents() {
        let db = make_async_db();
        let registry = SkillRegistry::empty();
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        // Only problem_type seeds
        assert_eq!(state.entities.len(), 5);
        for e in &state.entities {
            assert_eq!(e.entity_type, "problem_type");
        }
    }

    #[test]
    fn enumerate_agents() {
        let db = make_async_db();
        let registry = SkillRegistry::empty();
        let tool_registry = make_tool_registry(&[]);
        let agents = vec![AgentInfo {
            name: "mika-dev".to_string(),
            role: Some("developer".to_string()),
            model: Some("claude-sonnet-4-20250514".to_string()),
        }];

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &agents);
        let state = builder.enumerate_sources();

        let agent_entities: Vec<_> = state
            .entities
            .iter()
            .filter(|e| e.entity_type == "agent")
            .collect();
        assert_eq!(agent_entities.len(), 1);
        assert_eq!(agent_entities[0].entity_key, "agent:mika-dev");
    }

    #[test]
    fn enumerate_unknown_dependency_skipped() {
        let db = make_async_db();
        let skill = make_skill("child", "Child", vec!["nonexistent".to_string()], vec![]);
        let registry = SkillRegistry::from_test_entries(vec![skill]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        // No DEPENDS_ON edges (the dependency doesn't exist)
        let depends_on: Vec<_> = state
            .edges
            .iter()
            .filter(|e| e.edge_type == "DEPENDS_ON")
            .collect();
        assert!(depends_on.is_empty());
    }

    #[test]
    fn no_state_shaped_edges() {
        let db = make_async_db();
        let skill = make_skill("s1", "S1", vec![], vec!["t1"]);
        let registry = SkillRegistry::from_test_entries(vec![skill]);
        let tool_registry = make_tool_registry(&["t1"]);
        let agents = vec![AgentInfo {
            name: "test-agent".to_string(),
            role: None,
            model: None,
        }];

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &agents);
        let state = builder.enumerate_sources();

        // All edges must be only DEPENDS_ON or PROVIDES
        let allowed: HashSet<&str> = DOMAIN_RELATIONSHIP_TYPES.iter().copied().collect();
        for edge in &state.edges {
            assert!(
                allowed.contains(edge.edge_type.as_str()),
                "unexpected edge type: {}",
                edge.edge_type
            );
        }
    }

    #[test]
    fn builtin_tools_included() {
        let db = make_async_db();
        let registry = SkillRegistry::empty();
        let tool_registry = make_tool_registry(&["search_memory", "store_fact"]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        let state = builder.enumerate_sources();

        let tool_names: HashSet<_> = state
            .entities
            .iter()
            .filter(|e| e.entity_type == "tool")
            .map(|e| e.name.as_str())
            .collect();
        assert!(tool_names.contains("search_memory"));
        assert!(tool_names.contains("store_fact"));
    }

    #[tokio::test]
    async fn rebuild_fresh_db() {
        let db = make_async_db();
        let skill = make_skill("alpha", "Alpha skill", vec![], vec!["alpha_tool"]);
        let registry = SkillRegistry::from_test_entries(vec![skill]);
        let tool_registry = make_tool_registry(&["search_memory"]);
        let agents = vec![AgentInfo {
            name: "test-agent".to_string(),
            role: Some("tester".to_string()),
            model: None,
        }];

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &agents);
        let stats = builder.rebuild().await.expect("rebuild should succeed");

        // 1 skill + 2 tools + 1 agent + 5 problem_types = 9
        assert_eq!(stats.entities_added, 9);
        assert_eq!(stats.entities_updated, 0);
        assert_eq!(stats.entities_removed, 0);
        assert_eq!(stats.edges_provides, 1); // alpha -> alpha_tool
        assert_eq!(stats.edges_depends_on, 0);
    }

    #[tokio::test]
    async fn rebuild_is_idempotent() {
        let db = make_async_db();
        let skill = make_skill("beta", "Beta skill", vec![], vec!["beta_tool"]);
        let registry = SkillRegistry::from_test_entries(vec![skill]);
        let tool_registry = make_tool_registry(&[]);

        let agents = vec![AgentInfo {
            name: "test".to_string(),
            role: None,
            model: None,
        }];

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &agents);
        let stats1 = builder.rebuild().await.expect("first rebuild");
        assert!(stats1.entities_added > 0);

        // Second rebuild with same sources
        let builder2 = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &agents);
        let stats2 = builder2.rebuild().await.expect("second rebuild");
        assert_eq!(stats2.entities_added, 0);
        assert_eq!(stats2.entities_removed, 0);
        // Updates are expected (UPSERT updates existing rows)
        assert!(stats2.entities_updated > 0);
    }

    #[tokio::test]
    async fn rebuild_deletes_removed_entities() {
        let db = make_async_db();

        // First rebuild with skill X that provides tool_x
        let skill_x = make_skill("skill-x", "X", vec![], vec!["tool_x"]);
        let registry1 = SkillRegistry::from_test_entries(vec![skill_x]);
        let tool_registry = make_tool_registry(&[]);

        let builder1 = DomainGraphBuilder::new(&db, &registry1, &tool_registry, None, &[]);
        builder1.rebuild().await.expect("first rebuild");

        // Verify edge exists after first rebuild
        let edge_count_before: usize = db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM kg_relationships WHERE type = 'PROVIDES'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .expect("count edges");
        assert_eq!(edge_count_before, 1);

        // Second rebuild WITHOUT skill X
        let registry2 = SkillRegistry::empty();
        let builder2 = DomainGraphBuilder::new(&db, &registry2, &tool_registry, None, &[]);
        let stats2 = builder2.rebuild().await.expect("second rebuild");

        // skill-x entity should be removed
        assert!(stats2.entities_removed > 0);

        // Verify entity is gone from DB
        let remaining = db
            .with_db(|db| {
                let mut stmt = db.conn.prepare(
                    "SELECT entity_key FROM kg_entities WHERE entity_key = 'skill:skill-x'",
                )?;
                let keys: Vec<String> = stmt
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<_, _>>()?;
                Ok(keys)
            })
            .await
            .expect("query");
        assert!(remaining.is_empty());

        // Verify CASCADE removed edges touching deleted entity
        let edge_count_after: usize = db
            .with_db(|db| {
                db.conn
                    .query_row("SELECT COUNT(*) FROM kg_relationships", [], |row| {
                        row.get(0)
                    })
                    .map_err(Into::into)
            })
            .await
            .expect("count edges after");
        assert_eq!(edge_count_after, 0);
    }

    #[tokio::test]
    async fn rebuild_preserves_resolution_entity_links() {
        // Verifies that UPSERT preserves kg_entities.id (rowid), which is
        // referenced by kg_subject_resolutions.domain_entity_id. If the builder
        // accidentally DELETEd + INSERTed instead of UPSERTing, the rowid would
        // change and break FK references from the subject layer.
        let db = make_async_db();

        let skill = make_skill("linked-skill", "Linked", vec![], vec![]);
        let registry = SkillRegistry::from_test_entries(vec![skill.clone()]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        builder.rebuild().await.expect("first rebuild");

        // Get the entity rowid
        let entity_id: i64 = db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT id FROM kg_entities WHERE entity_key = 'skill:linked-skill'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .expect("get entity id");

        // Insert a subject entity and resolution pointing to this domain entity
        // (simulating #690/#691 subject extraction + resolution)
        db.with_db(move |db| {
            db.conn.execute(
                "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES ('test', 'test', '/tmp')",
                [],
            )?;
            db.conn.execute(
                "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence)
                 VALUES ('0000000000000000', '/test', 'skill:linked-skill', 'skill', 'linked-skill', 1.0)",
                [],
            )?;
            let subject_id: i64 = db.conn.last_insert_rowid();
            db.conn.execute(
                "INSERT INTO kg_subject_resolutions (agent_id, subject_entity_id, domain_entity_id, confidence)
                 VALUES ('test', ?1, ?2, 1.0)",
                rusqlite::params![subject_id, entity_id],
            )?;
            Ok(())
        })
        .await
        .expect("insert subject resolution");

        // Rebuild again with same sources — domain entity_id must be preserved
        let builder2 = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        builder2.rebuild().await.expect("second rebuild");

        // Resolution's domain_entity_id should still resolve
        let resolution_domain_id: i64 = db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT domain_entity_id FROM kg_subject_resolutions LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .expect("get resolution domain_entity_id");

        assert_eq!(
            resolution_domain_id, entity_id,
            "resolution FK should survive rebuild"
        );
    }

    #[tokio::test]
    async fn rebuild_preserves_rowid() {
        let db = make_async_db();

        let skill = make_skill("stable", "Stable", vec![], vec![]);
        let registry = SkillRegistry::from_test_entries(vec![skill.clone()]);
        let tool_registry = make_tool_registry(&[]);

        let builder = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        builder.rebuild().await.expect("first rebuild");

        // Get the rowid
        let rowid_before: i64 = db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT id FROM kg_entities WHERE entity_key = 'skill:stable'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .expect("get rowid");

        // Rebuild again
        let builder2 = DomainGraphBuilder::new(&db, &registry, &tool_registry, None, &[]);
        builder2.rebuild().await.expect("second rebuild");

        // Rowid should be preserved (UPSERT, not delete+insert)
        let rowid_after: i64 = db
            .with_db(|db| {
                db.conn
                    .query_row(
                        "SELECT id FROM kg_entities WHERE entity_key = 'skill:stable'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .expect("get rowid after");

        assert_eq!(rowid_before, rowid_after);
    }
}

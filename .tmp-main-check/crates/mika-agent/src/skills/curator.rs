//! Curator background task: proposes archival of idle agent-authored skills,
//! captures archive snapshots, and restores archived skills from snapshots.
//!
//! The curator never auto-archives — it proposes only. Operator action is
//! required to archive or restore skills.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::async_db::AsyncDatabase;
use crate::db::SkillOverride;

/// A curator proposal for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorProposal {
    pub skill_name: String,
    pub days_idle: u64,
    pub use_count: i64,
    pub last_used_at: Option<String>,
    pub recommendation: CuratorRecommendation,
}

/// What the curator recommends for an idle skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CuratorRecommendation {
    /// Skill should be archived (low usage + idle).
    Archive,
    /// Skill should be reviewed by operator (moderate usage but idle).
    Review,
}

/// Build curator proposals from archival candidates.
pub fn build_proposals(candidates: &[SkillOverride], max_idle_days: u32) -> Vec<CuratorProposal> {
    let now = chrono::Utc::now();
    candidates
        .iter()
        .map(|c| {
            let days_idle = c
                .last_used_at
                .as_ref()
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                .map(|dt| (now - dt.with_timezone(&chrono::Utc)).num_days().max(0) as u64)
                .unwrap_or(u64::from(max_idle_days)); // never used → treat as max idle

            let recommendation = if c.use_count <= 5 {
                CuratorRecommendation::Archive
            } else {
                CuratorRecommendation::Review
            };

            CuratorProposal {
                skill_name: c.skill_name.clone(),
                days_idle,
                use_count: c.use_count,
                last_used_at: c.last_used_at.clone(),
                recommendation,
            }
        })
        .collect()
}

/// Emit curator proposals as a structured log event and store in audit_events.
pub async fn emit_curator_proposal(
    db: &AsyncDatabase,
    agent_id: &str,
    proposals: &[CuratorProposal],
) -> Result<()> {
    let proposals_json = serde_json::to_string(proposals)?;

    info!(
        event = "curator_proposal",
        agent_id = %agent_id,
        candidate_count = proposals.len(),
        proposals = %proposals_json,
        "curator review completed"
    );

    // Store in audit_events for CLI retrieval via `mika skills curator status`.
    db.log_audit_event(
        "system",           // session_id — no real session for curator
        "curator_review",   // tool_name
        "curator_proposal", // target_key
        None,               // before_value
        Some(&proposals_json),
        None, // reasoning
        None, // trace_id
    )
    .await?;

    Ok(())
}

/// Capture a tar.gz snapshot of a skill directory before archival.
///
/// Returns the path to the created archive file.
pub fn capture_archive_snapshot(agent_home: &Path, skill_name: &str) -> Result<std::path::PathBuf> {
    let skill_dir = agent_home.join("skills").join(skill_name);
    anyhow::ensure!(
        skill_dir.is_dir(),
        "skill directory not found: {}",
        skill_dir.display()
    );

    let archived_dir = agent_home.join("skills").join(".archived");
    std::fs::create_dir_all(&archived_dir)
        .with_context(|| format!("failed to create .archived dir: {}", archived_dir.display()))?;

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let archive_path = archived_dir.join(format!("{skill_name}-{timestamp}.tar.gz"));

    let file = std::fs::File::create(&archive_path)
        .with_context(|| format!("failed to create archive: {}", archive_path.display()))?;
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.append_dir_all(skill_name, &skill_dir)
        .with_context(|| format!("failed to archive skill dir: {}", skill_dir.display()))?;
    tar.finish()?;

    info!(
        skill = %skill_name,
        archive = %archive_path.display(),
        "captured archive snapshot"
    );

    Ok(archive_path)
}

/// Restore a skill from its most recent archived snapshot.
///
/// Returns the path of the restored snapshot file.
pub fn restore_skill_from_snapshot(
    agent_home: &Path,
    skill_name: &str,
) -> Result<std::path::PathBuf> {
    let archived_dir = agent_home.join("skills").join(".archived");
    let pattern = format!("{skill_name}-");

    let mut snapshots: Vec<_> = std::fs::read_dir(&archived_dir)
        .with_context(|| format!("no .archived directory: {}", archived_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&pattern) && name.ends_with(".tar.gz")
        })
        .collect();

    snapshots.sort_by_key(|e| e.file_name());

    let latest = snapshots
        .last()
        .ok_or_else(|| anyhow::anyhow!("no archived snapshot found for skill '{skill_name}'"))?;

    let skill_dir = agent_home.join("skills").join(skill_name);
    // Remove existing directory if present
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir).with_context(|| {
            format!(
                "failed to remove existing skill dir: {}",
                skill_dir.display()
            )
        })?;
    }

    // Extract tar.gz
    let file = std::fs::File::open(latest.path())
        .with_context(|| format!("failed to open snapshot: {}", latest.path().display()))?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive
        .unpack(agent_home.join("skills"))
        .with_context(|| format!("failed to extract snapshot: {}", latest.path().display()))?;

    info!(
        skill = %skill_name,
        snapshot = %latest.path().display(),
        "restored skill from archive snapshot"
    );

    Ok(latest.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SkillOverride;

    #[test]
    fn test_build_proposals_never_used() {
        let candidates = vec![SkillOverride {
            skill_name: "my-skill".to_string(),
            lifecycle_state: Some("active".to_string()),
            use_count: 0,
            last_used_at: None,
            ..Default::default()
        }];
        let proposals = build_proposals(&candidates, 30);
        assert_eq!(proposals.len(), 1);
        assert!(matches!(
            proposals[0].recommendation,
            CuratorRecommendation::Archive
        ));
    }

    #[test]
    fn test_build_proposals_low_usage_idle() {
        let candidates = vec![SkillOverride {
            skill_name: "my-skill".to_string(),
            lifecycle_state: Some("active".to_string()),
            use_count: 3,
            last_used_at: Some("2020-01-01T00:00:00Z".to_string()),
            ..Default::default()
        }];
        let proposals = build_proposals(&candidates, 30);
        assert_eq!(proposals.len(), 1);
        assert!(matches!(
            proposals[0].recommendation,
            CuratorRecommendation::Archive
        ));
    }

    #[test]
    fn test_build_proposals_high_usage_idle_gets_review() {
        let candidates = vec![SkillOverride {
            skill_name: "my-skill".to_string(),
            lifecycle_state: Some("active".to_string()),
            use_count: 10,
            last_used_at: Some("2020-01-01T00:00:00Z".to_string()),
            ..Default::default()
        }];
        let proposals = build_proposals(&candidates, 30);
        assert_eq!(proposals.len(), 1);
        assert!(matches!(
            proposals[0].recommendation,
            CuratorRecommendation::Review
        ));
    }

    #[test]
    fn test_capture_and_restore_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_home = tmp.path();

        // Create a skill directory with a file
        let skill_dir = agent_home.join("skills").join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            b"[skill]\nname = \"test-skill\"",
        )
        .unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), b"# Test").unwrap();

        // Capture snapshot
        let archive_path = capture_archive_snapshot(agent_home, "test-skill").unwrap();
        assert!(archive_path.exists());
        assert!(
            archive_path
                .to_string_lossy()
                .contains(".archived/test-skill-")
        );

        // Remove original and restore
        std::fs::remove_dir_all(&skill_dir).unwrap();
        assert!(!skill_dir.exists());

        let restored_path = restore_skill_from_snapshot(agent_home, "test-skill").unwrap();
        assert!(restored_path.exists());
        assert!(skill_dir.exists());
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("system_prompt.md").exists());
    }

    #[test]
    fn test_restore_missing_snapshot_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_home = tmp.path();
        std::fs::create_dir_all(agent_home.join("skills").join(".archived")).unwrap();

        let result = restore_skill_from_snapshot(agent_home, "nonexistent");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no archived snapshot")
        );
    }

    // --- AC9-12 DB-level candidate query tests (mika#1584) ---

    fn insert_override(
        db: &crate::db::Database,
        agent_id: &str,
        name: &str,
        lifecycle: Option<&str>,
        use_count: i64,
        last_used: Option<&str>,
    ) {
        db.conn
            .execute(
                "INSERT INTO skill_overrides
                 (agent_id, skill_name, always_on, llm_provider, llm_model, enabled,
                  lifecycle_state, use_count, last_used_at)
                 VALUES (?1, ?2, NULL, NULL, NULL, NULL, ?3, ?4, ?5)",
                rusqlite::params![agent_id, name, lifecycle, use_count, last_used],
            )
            .unwrap();
    }

    #[test]
    fn test_get_archival_candidates_fresh_agent_returns_empty() {
        // AC9: no skill rows → zero candidates
        let db = crate::db::Database::open_in_memory().unwrap();
        let result = db.get_archival_candidates("test-agent", 30).unwrap();
        assert!(result.is_empty(), "fresh agent should have no candidates");
    }

    #[test]
    fn test_get_archival_candidates_staged_skill_excluded() {
        // AC10: lifecycle_state='staged' → zero candidates (gate excludes non-active)
        let db = crate::db::Database::open_in_memory().unwrap();
        insert_override(&db, "test-agent", "staged-skill", Some("staged"), 0, None);
        let result = db.get_archival_candidates("test-agent", 30).unwrap();
        assert!(
            result.is_empty(),
            "staged skill must be excluded from archival candidates"
        );
    }

    #[test]
    fn test_get_archival_candidates_idle_active_skill_returned() {
        // AC11: active skill with last_used_at >30d ago → one candidate
        let db = crate::db::Database::open_in_memory().unwrap();
        let old_time = crate::timestamp::format(&(chrono::Utc::now() - chrono::Duration::days(60)));
        insert_override(
            &db,
            "test-agent",
            "idle-active-skill",
            Some("active"),
            1,
            Some(&old_time),
        );
        let result = db.get_archival_candidates("test-agent", 30).unwrap();
        assert_eq!(
            result.len(),
            1,
            "idle active skill must be returned as a candidate"
        );
        assert_eq!(result[0].skill_name, "idle-active-skill");
    }

    #[test]
    fn test_get_archival_candidates_null_lifecycle_excluded() {
        // AC12: bundled/marketplace skill (NULL lifecycle_state) → zero candidates
        let db = crate::db::Database::open_in_memory().unwrap();
        insert_override(&db, "test-agent", "bundled-skill", None, 0, None);
        let result = db.get_archival_candidates("test-agent", 30).unwrap();
        assert!(
            result.is_empty(),
            "NULL lifecycle_state (bundled/marketplace) must be excluded"
        );
    }
}

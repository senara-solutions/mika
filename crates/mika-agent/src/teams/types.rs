use serde::Deserializer;

/// Deserialize a value that could be either an integer (legacy) or string (new format).
fn deserialize_timestamp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    use serde::de;
    struct TimestampVisitor;
    impl<'de> de::Visitor<'de> for TimestampVisitor {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an ISO 8601 string or unix timestamp integer")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            let dt = chrono::TimeZone::timestamp_opt(&chrono::Utc, v, 0)
                .single()
                .ok_or_else(|| de::Error::custom(format!("invalid unix timestamp: {v}")))?;
            Ok(crate::timestamp::format(&dt))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            self.visit_i64(v as i64)
        }
    }
    deserializer.deserialize_any(TimestampVisitor)
}

/// Same but for Option<T>
fn deserialize_optional_timestamp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    use serde::de;
    struct OptTimestampVisitor;
    impl<'de> de::Visitor<'de> for OptTimestampVisitor {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, an ISO 8601 string, or unix timestamp integer")
        }
        fn visit_none<E: de::Error>(self) -> Result<Option<String>, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Option<String>, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<String>, D2::Error> {
            deserialize_timestamp(d).map(Some)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<String>, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<Option<String>, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<String>, E> {
            let dt = chrono::TimeZone::timestamp_opt(&chrono::Utc, v, 0)
                .single()
                .ok_or_else(|| de::Error::custom(format!("invalid unix timestamp: {v}")))?;
            Ok(Some(crate::timestamp::format(&dt)))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<String>, E> {
            self.visit_i64(v as i64)
        }
    }
    deserializer.deserialize_any(OptTimestampVisitor)
}

/// Tracks the overall state of a team execution run.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TeamRun {
    pub run_id: String,
    pub team_name: String,
    pub goal: String,
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default)]
    pub iteration: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub tasks: Vec<TaskAssignment>,
    #[serde(default = "default_timestamp")]
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub started_at: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_timestamp")]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub deliverable: Option<String>,
    /// Whether the coverage-check retry fired during decomposition (#286).
    /// Set when the orchestrator silently omitted team members and a nudge
    /// re-prompt was issued. Observable via `team_coverage_gap` warn log.
    #[serde(default)]
    pub coverage_retry_fired: bool,
    /// Whether the conversational-fallthrough retry fired during decomposition
    /// (mika#1676 Unit A). Set when the orchestrator returned a `Conversational`
    /// result for an *actionable* goal and the delegation gate issued one
    /// reinforced re-prompt. Distinct from `coverage_retry_fired` (#286,
    /// omitted-member case) — different trigger, different correction prompt.
    #[serde(default)]
    pub conversational_retry_fired: bool,
    /// Number of member sessions delegated during this run (mika#1676 Unit B).
    /// Incremented in `execute_tasks` per spawned member, accumulated across
    /// iterations. A terminal `Completed` run with `delegation_count == 0` is a
    /// solo-absorption (the orchestrator did the work itself).
    #[serde(default)]
    pub delegation_count: u32,
    /// Whether the run completed without delegating to any member (mika#1676
    /// Unit B). Set by `finalize_and_shutdown` when the run reached `Completed`
    /// with `delegation_count == 0`. Queryable backstop so this failure class is
    /// visible without a manual audit.
    #[serde(default)]
    pub solo_absorption: bool,
    /// Structured failure context for `FailedNoDelegation` runs (mika#1676
    /// Unit A). JSON holding `{"phase": "first_decompose"|"revision_after_critic"}`
    /// so a single terminal state can carry which decompose phase failed
    /// (architect F2 — phase as JSON detail, not enum proliferation).
    #[serde(default)]
    pub failure_context: Option<String>,
}

fn default_timestamp() -> String {
    crate::timestamp::now()
}

fn default_max_iterations() -> u32 {
    3
}

/// Status of the overall team run.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RunStatus {
    #[default]
    Running,
    Completed,
    Suspended,
    Failed(String),
    /// Terminal state for a run that refused to complete because the
    /// orchestrator returned a conversational reply for an actionable goal and
    /// delegated to zero members, even after one reinforced retry (mika#1676
    /// Unit A). Persisted as `status='failed_no_delegation'`; the failing
    /// decompose phase is carried in `TeamRun::failure_context`.
    FailedNoDelegation,
    /// Terminal state (mika#1671 D3): every completed delegation in an iteration
    /// failed at the transport layer, so the run short-circuited without entering
    /// review/deliver. Distinct from `Failed(String)` so dashboards and metrics can
    /// tell "all-members-transport-error (retryable in principle)" apart from a
    /// mixed/business-logic failure. Carries a human-readable reason, mirroring
    /// `Failed(String)`. Composes independently with `FailedNoDelegation` — the two
    /// terminal states cover different failure classes at different layers.
    FailedTransport(String),
}

/// A task delegated to a specialist agent.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TaskAssignment {
    pub agent: String,
    #[serde(default)]
    pub role: String,
    pub task: String,
    #[serde(default)]
    pub output_file: String,
    #[serde(default)]
    pub status: TaskStatus,
}

/// Status of an individual task within a run.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed(String),
}

/// Phase of the team execution pipeline.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TeamPhase {
    Decompose,
    Execute,
    Review,
    Deliver,
    ReDecompose,
}

impl std::fmt::Display for TeamPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decompose => write!(f, "decomposing"),
            Self::Execute => write!(f, "executing"),
            Self::Review => write!(f, "reviewing"),
            Self::Deliver => write!(f, "delivering"),
            Self::ReDecompose => write!(f, "re-decomposing"),
        }
    }
}

/// Events emitted by the team engine during execution.
///
/// Used to communicate structured progress to callers (TUI, management tools).
/// The caller decides what to display vs persist.
#[derive(Clone, Debug)]
pub enum TeamEvent {
    /// Transient progress message (e.g., "Decomposing goal...")
    Progress(String),
    /// Phase transition in the team pipeline.
    PhaseChanged { phase: TeamPhase, iteration: u32 },
    /// An agent has started working on its task.
    AgentStarted { agent: String, role: String },
    /// Orchestrator decomposed goal into tasks.
    TasksAssigned {
        tasks: Vec<TaskAssignment>,
        iteration: u32,
    },
    /// Individual agent completed its task.
    AgentCompleted { agent: String, response: String },
    /// Individual agent failed.
    AgentFailed { agent: String, error: String },
    /// Critic reviewed outputs.
    CriticReview {
        approved: bool,
        feedback: String,
        iteration: u32,
    },
    /// Final deliverable produced.
    Deliverable(String),
    /// Run failed with an error.
    RunFailed(String),
}

/// Callback for receiving team events during execution.
pub type TeamEventCallback = Box<dyn Fn(TeamEvent) + Send + Sync>;

/// Current checkpoint serialization version.
pub const CHECKPOINT_VERSION: u32 = 2;

/// Serialize a `TeamRun` into a versioned checkpoint envelope.
pub fn serialize_checkpoint(run: &TeamRun) -> String {
    serde_json::to_string(&serde_json::json!({
        "version": CHECKPOINT_VERSION,
        "data": run,
    }))
    .unwrap_or_default()
}

/// Deserialize a `TeamRun` from a checkpoint string.
///
/// Supports both versioned envelopes (`{"version": N, "data": ...}`) and
/// legacy unversioned checkpoints (plain `TeamRun` JSON) for backward compat.
pub fn deserialize_checkpoint(checkpoint: &str) -> anyhow::Result<TeamRun> {
    let envelope: serde_json::Value = serde_json::from_str(checkpoint)
        .map_err(|e| anyhow::anyhow!("invalid checkpoint JSON: {e}"))?;
    if envelope.get("version").is_some() {
        // Versioned checkpoint — extract the data payload
        let data = envelope
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("missing 'data' field in versioned checkpoint"))?;
        serde_json::from_value(data.clone()).map_err(|e| {
            anyhow::anyhow!("failed to deserialize team run from versioned checkpoint: {e}")
        })
    } else {
        // Legacy unversioned checkpoint (backward compat)
        serde_json::from_value(envelope)
            .map_err(|e| anyhow::anyhow!("failed to deserialize legacy team run checkpoint: {e}"))
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Running => write!(f, "running"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Suspended => write!(f, "suspended"),
            RunStatus::Failed(msg) => write!(f, "failed: {msg}"),
            RunStatus::FailedNoDelegation => write!(f, "failed_no_delegation"),
            RunStatus::FailedTransport(msg) => write!(f, "failed_transport: {msg}"),
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_status_display() {
        assert_eq!(RunStatus::Running.to_string(), "running");
        assert_eq!(RunStatus::Completed.to_string(), "completed");
        assert_eq!(
            RunStatus::Failed("timeout".to_string()).to_string(),
            "failed: timeout"
        );
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(
            TaskStatus::Failed("error".to_string()).to_string(),
            "failed: error"
        );
    }

    #[test]
    fn test_team_event_debug() {
        let event = TeamEvent::Progress("Decomposing goal...".to_string());
        let debug = format!("{event:?}");
        assert!(debug.contains("Decomposing goal"));

        let event = TeamEvent::AgentCompleted {
            agent: "researcher".to_string(),
            response: "Found 3 papers".to_string(),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("researcher"));
    }

    #[test]
    fn test_team_phase_display() {
        assert_eq!(TeamPhase::Decompose.to_string(), "decomposing");
        assert_eq!(TeamPhase::Execute.to_string(), "executing");
        assert_eq!(TeamPhase::Review.to_string(), "reviewing");
        assert_eq!(TeamPhase::Deliver.to_string(), "delivering");
        assert_eq!(TeamPhase::ReDecompose.to_string(), "re-decomposing");
    }

    #[test]
    fn test_phase_changed_event() {
        let event = TeamEvent::PhaseChanged {
            phase: TeamPhase::Execute,
            iteration: 2,
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("Execute"));
        assert!(debug.contains("2"));
    }

    #[test]
    fn test_agent_started_event() {
        let event = TeamEvent::AgentStarted {
            agent: "researcher".to_string(),
            role: "analyst".to_string(),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("researcher"));
        assert!(debug.contains("analyst"));
    }

    #[test]
    fn test_team_event_clone() {
        let event = TeamEvent::CriticReview {
            approved: true,
            feedback: "Looks good".to_string(),
            iteration: 2,
        };
        let cloned = event.clone();
        if let TeamEvent::CriticReview {
            approved,
            feedback,
            iteration,
        } = cloned
        {
            assert!(approved);
            assert_eq!(feedback, "Looks good");
            assert_eq!(iteration, 2);
        } else {
            panic!("unexpected variant");
        }
    }

    #[test]
    fn test_serde_defaults_on_team_run() {
        // Minimal JSON with only the required fields (no defaults)
        let json =
            r#"{"run_id":"r1","team_name":"t1","goal":"g1","started_at":"2026-01-01T00:00:00Z"}"#;
        let run: TeamRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.run_id, "r1");
        assert_eq!(run.status, RunStatus::Running); // default
        assert_eq!(run.iteration, 0); // default
        assert_eq!(run.max_iterations, 3); // custom default
        assert!(run.tasks.is_empty()); // default
        assert_eq!(run.started_at, "2026-01-01T00:00:00Z"); // provided
        assert!(run.ended_at.is_none()); // default
        assert!(run.deliverable.is_none()); // default
        assert!(!run.coverage_retry_fired); // default false
    }

    #[test]
    fn test_coverage_retry_fired_backward_compat() {
        // Pre-existing checkpoint without coverage_retry_fired should deserialize with false
        let json = r#"{"run_id":"r1","team_name":"t1","goal":"g1","started_at":"2026-01-01T00:00:00Z","status":"Completed"}"#;
        let run: TeamRun = serde_json::from_str(json).unwrap();
        assert!(!run.coverage_retry_fired);
    }

    #[test]
    fn test_coverage_retry_fired_round_trip() {
        let run = TeamRun {
            run_id: "r1".to_string(),
            team_name: "t1".to_string(),
            goal: "g1".to_string(),
            status: RunStatus::Running,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: None,
            deliverable: None,
            coverage_retry_fired: true,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        };
        let json = serde_json::to_string(&run).unwrap();
        let restored: TeamRun = serde_json::from_str(&json).unwrap();
        assert!(restored.coverage_retry_fired);
    }

    #[test]
    fn test_serde_defaults_on_task_assignment() {
        // Minimal JSON with only the required fields
        let json = r#"{"agent":"a1","task":"do stuff"}"#;
        let ta: TaskAssignment = serde_json::from_str(json).unwrap();
        assert_eq!(ta.agent, "a1");
        assert_eq!(ta.task, "do stuff");
        assert_eq!(ta.role, ""); // default
        assert_eq!(ta.output_file, ""); // default
        assert_eq!(ta.status, TaskStatus::Pending); // default
    }

    #[test]
    fn test_run_status_default() {
        assert_eq!(RunStatus::default(), RunStatus::Running);
    }

    #[test]
    fn test_task_status_default() {
        assert_eq!(TaskStatus::default(), TaskStatus::Pending);
    }

    #[test]
    fn test_serialize_checkpoint_includes_version() {
        let run = TeamRun {
            run_id: "r1".to_string(),
            team_name: "t1".to_string(),
            goal: "g1".to_string(),
            status: RunStatus::Running,
            iteration: 1,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: None,
            deliverable: None,
            coverage_retry_fired: false,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        };
        let json = serialize_checkpoint(&run);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["version"], CHECKPOINT_VERSION);
        assert!(parsed["data"].is_object());
        assert_eq!(parsed["data"]["run_id"], "r1");
    }

    #[test]
    fn test_deserialize_versioned_checkpoint() {
        let run = TeamRun {
            run_id: "r1".to_string(),
            team_name: "t1".to_string(),
            goal: "test goal".to_string(),
            status: RunStatus::Completed,
            iteration: 2,
            max_iterations: 3,
            tasks: vec![],
            started_at: "2026-01-01T00:00:00Z".to_string(),
            ended_at: Some("2026-01-01T01:00:00Z".to_string()),
            deliverable: Some("done".to_string()),
            coverage_retry_fired: false,
            conversational_retry_fired: false,
            delegation_count: 0,
            solo_absorption: false,
            failure_context: None,
        };
        let checkpoint = serialize_checkpoint(&run);
        let restored = deserialize_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.run_id, "r1");
        assert_eq!(restored.goal, "test goal");
        assert_eq!(restored.status, RunStatus::Completed);
        assert_eq!(restored.iteration, 2);
        assert_eq!(restored.ended_at.as_deref(), Some("2026-01-01T01:00:00Z"));
        assert_eq!(restored.deliverable.as_deref(), Some("done"));
    }

    #[test]
    fn test_deserialize_legacy_unversioned_checkpoint() {
        // Legacy format: plain TeamRun JSON without version envelope, using integer timestamps
        let legacy_json = r#"{
            "run_id": "r1",
            "team_name": "t1",
            "goal": "g1",
            "status": "Running",
            "iteration": 1,
            "max_iterations": 3,
            "tasks": [],
            "started_at": 100,
            "ended_at": null,
            "deliverable": null
        }"#;
        let run = deserialize_checkpoint(legacy_json).unwrap();
        assert_eq!(run.run_id, "r1");
        assert_eq!(run.status, RunStatus::Running);
        // Integer timestamp 100 should be converted to ISO 8601
        assert_eq!(run.started_at, "1970-01-01T00:01:40Z");
    }

    #[test]
    fn test_deserialize_legacy_checkpoint_with_integer_timestamps() {
        // Backward compat: versioned checkpoint with integer timestamps (v1 format)
        let legacy_json = r#"{
            "version": 1,
            "data": {
                "run_id": "r1",
                "team_name": "t1",
                "goal": "g1",
                "status": "Running",
                "iteration": 1,
                "max_iterations": 3,
                "tasks": [],
                "started_at": 1741000000,
                "ended_at": 1741003600,
                "deliverable": null
            }
        }"#;
        let run = deserialize_checkpoint(legacy_json).unwrap();
        assert_eq!(run.run_id, "r1");
        // Integer timestamps should be converted to ISO 8601 strings
        assert!(!run.started_at.is_empty());
        assert!(run.ended_at.is_some());
        // Verify they parse as valid timestamps
        let started = crate::timestamp::parse(&run.started_at).unwrap();
        let ended = crate::timestamp::parse(run.ended_at.as_ref().unwrap()).unwrap();
        assert!(ended > started);
    }

    #[test]
    fn test_deserialize_checkpoint_missing_data_field() {
        let bad = r#"{"version": 1}"#;
        let result = deserialize_checkpoint(bad);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'data'"));
    }

    #[test]
    fn test_deserialize_checkpoint_invalid_json() {
        let result = deserialize_checkpoint("not json");
        assert!(result.is_err());
    }
}

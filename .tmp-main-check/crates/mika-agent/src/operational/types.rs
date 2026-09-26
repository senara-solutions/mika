//! Domain types for the operational ledger.
//!
//! These types are the Rust-side authority for the `operational_items` table.
//! All writes to `operational_items.evidence_refs` MUST go through the
//! [`EvidenceRef`] type — direct SQL writes that bypass the Rust serializer
//! are unsupported and will break the closed-enum guarantee from foundation
//! Decision F.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The kind of operational item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalKind {
    Goal,
    Task,
    Commitment,
    Decision,
    Blocker,
    Evidence,
    NextAction,
}

impl OperationalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Task => "task",
            Self::Commitment => "commitment",
            Self::Decision => "decision",
            Self::Blocker => "blocker",
            Self::Evidence => "evidence",
            Self::NextAction => "next_action",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "goal" => Some(Self::Goal),
            "task" => Some(Self::Task),
            "commitment" => Some(Self::Commitment),
            "decision" => Some(Self::Decision),
            "blocker" => Some(Self::Blocker),
            "evidence" => Some(Self::Evidence),
            "next_action" => Some(Self::NextAction),
            _ => None,
        }
    }
}

impl fmt::Display for OperationalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The status of an operational item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalStatus {
    Now,
    Waiting,
    Delegated,
    Scheduled,
    AtRisk,
    Done,
}

impl OperationalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::Waiting => "waiting",
            Self::Delegated => "delegated",
            Self::Scheduled => "scheduled",
            Self::AtRisk => "at_risk",
            Self::Done => "done",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "now" => Some(Self::Now),
            "waiting" => Some(Self::Waiting),
            "delegated" => Some(Self::Delegated),
            "scheduled" => Some(Self::Scheduled),
            "at_risk" => Some(Self::AtRisk),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

impl fmt::Display for OperationalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Non-terminal status — type-level guard preventing callers from passing
/// `Done` to `update_operational_item_status()`. The only path to `Done`
/// is `complete_operational_item()` which requires an [`EvidenceRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonTerminalStatus {
    Now,
    Waiting,
    Delegated,
    Scheduled,
    AtRisk,
}

impl From<NonTerminalStatus> for OperationalStatus {
    fn from(s: NonTerminalStatus) -> Self {
        match s {
            NonTerminalStatus::Now => Self::Now,
            NonTerminalStatus::Waiting => Self::Waiting,
            NonTerminalStatus::Delegated => Self::Delegated,
            NonTerminalStatus::Scheduled => Self::Scheduled,
            NonTerminalStatus::AtRisk => Self::AtRisk,
        }
    }
}

/// Owner of an operational item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "name")]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    User,
    Mika,
    Person(String),
    Agent(String),
}

impl Owner {
    /// Returns the `owner_type` column value.
    pub fn owner_type(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Mika => "mika",
            Self::Person(_) => "person",
            Self::Agent(_) => "agent",
        }
    }

    /// Returns the `owner_name` column value (None for User/Mika).
    pub fn owner_name(&self) -> Option<&str> {
        match self {
            Self::User | Self::Mika => None,
            Self::Person(name) | Self::Agent(name) => Some(name),
        }
    }

    /// Reconstruct from DB columns.
    pub fn from_db(owner_type: &str, owner_name: Option<&str>) -> Option<Self> {
        match owner_type {
            "user" => Some(Self::User),
            "mika" => Some(Self::Mika),
            "person" => Some(Self::Person(owner_name?.to_string())),
            "agent" => Some(Self::Agent(owner_name?.to_string())),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Evidence references
// ---------------------------------------------------------------------------

/// A reference to evidence supporting an operational item.
///
/// This is a closed enum (foundation Decision F). Adding a new variant is a
/// schema-level change that requires updating all consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceRefKind,
    pub id: String,
}

/// The kind of evidence reference — closed enum per foundation Decision F.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRefKind {
    OperationalItem,
    Message,
    ToolCall,
    GithubIssue,
    GithubPr,
    File,
    External,
}

// ---------------------------------------------------------------------------
// Core domain struct
// ---------------------------------------------------------------------------

/// A row in the `operational_items` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalItem {
    pub id: String,
    pub kind: OperationalKind,
    pub title: String,
    pub status: OperationalStatus,
    pub owner: Owner,
    pub priority: f32,
    pub user_importance: f32,
    pub due_at: Option<String>,
    pub blocked_by: Option<String>,
    pub next_action: Option<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub confidence: f32,
    pub source_table: Option<String>,
    pub source_id: Option<String>,
    pub agent_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a new operational item (no id/timestamps — generated
/// by the DB layer).
#[derive(Debug, Clone)]
pub struct NewOperationalItem {
    pub kind: OperationalKind,
    pub title: String,
    pub status: OperationalStatus,
    pub owner: Owner,
    pub priority: f32,
    pub user_importance: f32,
    pub due_at: Option<String>,
    pub blocked_by: Option<String>,
    pub next_action: Option<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub confidence: f32,
    pub source_table: Option<String>,
    pub source_id: Option<String>,
    pub agent_id: String,
}

impl Default for NewOperationalItem {
    fn default() -> Self {
        Self {
            kind: OperationalKind::Task,
            title: String::new(),
            status: OperationalStatus::Now,
            owner: Owner::User,
            priority: 0.0,
            user_importance: 0.0,
            due_at: None,
            blocked_by: None,
            next_action: None,
            evidence_refs: Vec::new(),
            confidence: 1.0,
            source_table: None,
            source_id: None,
            agent_id: String::new(),
        }
    }
}

/// Filter for querying operational items.
#[derive(Debug, Clone, Default)]
pub struct OperationalItemFilter {
    pub agent_id: String,
    pub kind: Option<OperationalKind>,
    pub status: Option<OperationalStatus>,
    pub owner_type: Option<String>,
    pub limit: Option<u32>,
    pub sort_by_priority: bool,
}

// ---------------------------------------------------------------------------
// mika-arch decision marker (Phase 3.8)
// ---------------------------------------------------------------------------

/// Parsed from `DECISION NEEDED — <ID> (<title>):` markers in mika-arch
/// responses. Detection regex and parser live here (Layer 1); the trigger
/// wiring is Layer 3's concern.
#[derive(Debug, Clone)]
pub struct ArchDecisionMarker {
    pub id: String,
    pub title: String,
    pub doc_path: String,
    pub tool_call_id: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trip() {
        let kinds = [
            OperationalKind::Goal,
            OperationalKind::Task,
            OperationalKind::Commitment,
            OperationalKind::Decision,
            OperationalKind::Blocker,
            OperationalKind::Evidence,
            OperationalKind::NextAction,
        ];
        for kind in kinds {
            let s = kind.as_str();
            assert_eq!(
                OperationalKind::from_str_opt(s),
                Some(kind),
                "round-trip failed for {s}"
            );
        }
    }

    #[test]
    fn status_round_trip() {
        let statuses = [
            OperationalStatus::Now,
            OperationalStatus::Waiting,
            OperationalStatus::Delegated,
            OperationalStatus::Scheduled,
            OperationalStatus::AtRisk,
            OperationalStatus::Done,
        ];
        for status in statuses {
            let s = status.as_str();
            assert_eq!(
                OperationalStatus::from_str_opt(s),
                Some(status),
                "round-trip failed for {s}"
            );
        }
    }

    #[test]
    fn non_terminal_to_operational() {
        assert_eq!(
            OperationalStatus::from(NonTerminalStatus::Now),
            OperationalStatus::Now
        );
        assert_eq!(
            OperationalStatus::from(NonTerminalStatus::AtRisk),
            OperationalStatus::AtRisk
        );
    }

    #[test]
    fn owner_db_round_trip() {
        let cases: Vec<(Owner, &str, Option<&str>)> = vec![
            (Owner::User, "user", None),
            (Owner::Mika, "mika", None),
            (Owner::Person("Alice".into()), "person", Some("Alice")),
            (Owner::Agent("mika-dev".into()), "agent", Some("mika-dev")),
        ];
        for (owner, expected_type, expected_name) in cases {
            assert_eq!(owner.owner_type(), expected_type);
            assert_eq!(owner.owner_name(), expected_name);
            let reconstructed = Owner::from_db(expected_type, expected_name).unwrap();
            assert_eq!(reconstructed, owner);
        }
    }

    #[test]
    fn evidence_ref_serde_round_trip() {
        let refs = vec![
            EvidenceRef {
                kind: EvidenceRefKind::GithubPr,
                id: "https://github.com/foo/bar/pull/1".into(),
            },
            EvidenceRef {
                kind: EvidenceRefKind::OperationalItem,
                id: "abc-123".into(),
            },
            EvidenceRef {
                kind: EvidenceRefKind::External,
                id: "mika-arch-session:xyz".into(),
            },
        ];
        let json = serde_json::to_string(&refs).unwrap();
        let parsed: Vec<EvidenceRef> = serde_json::from_str(&json).unwrap();
        assert_eq!(refs, parsed);
    }
}

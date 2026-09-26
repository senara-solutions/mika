//! Reading a `Task`'s text — one definition, shared by both sides of the return
//! channel (mika#2270).
//!
//! # The three tiers are not three sources
//!
//! A `Task` can carry text in three places, and this module reads them in the
//! order the A2A spec suggests: artifacts, then agent-role history, then
//! `status.message`. That order looks like defence in depth. Against a lost turn
//! it is not, and the comment this module replaces said otherwise for months:
//!
//! * **Tier 1 has no producer in mika-spirit.** `Database::a2a_insert_artifact`
//!   has no caller outside its own module's tests, so `task.artifacts` built by
//!   `a2a_build_task` is always `None`. Writing real A2A artifacts is a
//!   protocol-conformance job with its own perimeter; until then this tier is
//!   dead weight on the spirit path (it stays because `--remote` may face a
//!   spec-conformant server that does populate it).
//! * **Tiers 2 and 3 are one query.** `a2a_build_task` derives `status.message`
//!   as *the last agent-role message of `history`*, and `history` comes from
//!   `a2a_get_messages`. If that query returns nothing, `history` is `None`
//!   **and** `status.message` is `None`. Neither tier can rescue the other.
//!
//! So an empty rendering is not "the renderer looked in the wrong place". It is
//! the Task carrying nothing, and the only honest answer is to say so — which is
//! why this function returns a `Result` and never an empty `String`.
//!
//! # Why it lives here and not in the CLI
//!
//! Two callers need the *same* verdict, not two approximations of it:
//! `mika ask` decides whether to print or to fail, and mika-spirit's
//! `message/send` decides whether its rebuilt Task needs the net that serves the
//! turn text it still holds in hand. If those two predicates could disagree, the
//! server would believe a Task fine while the client found nothing readable —
//! the exact gap mika#2270's net exists to close.

use std::fmt;

use crate::types::{Part, Role, Task};

/// Why a rendering produced nothing, and what was inspected to find out.
///
/// Every field is here to be printed. A message that says "empty" without
/// saying what was looked at reproduces mika#2270's defect one noise level up:
/// the caller still cannot tell "the agent had nothing to say" from "the answer
/// was lost on the way back".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRenderEmpty {
    /// Server-minted task id — one of the two handles that find the turn in
    /// `$MIKA_SPIRIT_LOG_FILE`.
    pub task_id: String,
    /// The caller's own handle on the exchange, when it sent one.
    pub context_id: Option<String>,
    /// Whether the Task carried slices at all.
    pub kind: EmptyKind,
    pub artifacts: usize,
    pub artifact_parts: usize,
    pub history: usize,
    pub agent_history: usize,
    pub agent_history_parts: usize,
    pub status_message: bool,
    pub status_message_parts: usize,
}

/// The two shapes an unrenderable Task can take.
///
/// They call for different investigations, so they are not collapsed into one
/// sentence: `NoSliceCarried` points at the Task's construction (mika#2270's
/// measured shape — `a2a_get_messages` returned nothing), `SlicesUnreadable`
/// points at this renderer facing content it does not know how to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyKind {
    /// No artifact, no history message, no `status.message`.
    NoSliceCarried,
    /// At least one slice is present, and none of them yields text.
    SlicesUnreadable,
}

impl fmt::Display for TaskRenderEmpty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let head = match self.kind {
            EmptyKind::NoSliceCarried => {
                "the agent's reply is absent from this task: it carries no artifact, \
                 no history message and no status message"
            }
            EmptyKind::SlicesUnreadable => {
                "the agent's reply is unreadable in this task: it carries slices, \
                 but none of them yields text"
            }
        };
        write!(
            f,
            "{head} (task {}, context {}) — inspected: {} artifact(s) carrying {} part(s), \
             {} history message(s) of which {} from the agent carrying {} part(s), \
             status.message {} carrying {} part(s). \
             If the engine produced a turn, its text is in $MIKA_SPIRIT_LOG_FILE under this task id.",
            self.task_id,
            self.context_id.as_deref().unwrap_or("none"),
            self.artifacts,
            self.artifact_parts,
            self.history,
            self.agent_history,
            self.agent_history_parts,
            if self.status_message {
                "present"
            } else {
                "absent"
            },
            self.status_message_parts,
        )
    }
}

impl std::error::Error for TaskRenderEmpty {}

/// Render a `Task`'s text, or report that there is none to render.
///
/// Non-text parts surface as placeholders (`[file: <name>]`, `[data]`);
/// multi-text parts join with blank lines so the agent's paragraph boundaries
/// survive. A successful rendering is byte-identical to what this code produced
/// before mika#2270 — only the empty case changed, from `""` to an error.
pub fn render_task_text(task: &Task) -> Result<String, TaskRenderEmpty> {
    let mut parts_text: Vec<String> = Vec::new();

    // Tier 1: artifacts.
    let mut artifacts = 0;
    let mut artifact_parts = 0;
    if let Some(list) = &task.artifacts {
        artifacts = list.len();
        for artifact in list {
            artifact_parts += artifact.parts.len();
            for part in &artifact.parts {
                push_rendered_part(part, &mut parts_text);
            }
        }
    }

    // Tier 2: agent-role messages in history.
    let mut history = 0;
    let mut agent_history = 0;
    let mut agent_history_parts = 0;
    if let Some(messages) = &task.history {
        history = messages.len();
        for msg in messages {
            if msg.role == Role::Agent {
                agent_history += 1;
                agent_history_parts += msg.parts.len();
                for part in &msg.parts {
                    push_rendered_part(part, &mut parts_text);
                }
            }
        }
    }

    // Tier 3: `status.message`, consulted only when tiers 1+2 produced nothing.
    // The part count is taken unconditionally so the report describes the Task
    // rather than the path this call happened to take through it.
    let status_message_parts = task.status.message.as_ref().map_or(0, |m| m.parts.len());
    let status_message = task.status.message.is_some();
    if parts_text.is_empty()
        && let Some(msg) = task.status.message.as_ref()
    {
        for part in &msg.parts {
            push_rendered_part(part, &mut parts_text);
        }
    }

    let rendered = parts_text.join("\n\n");
    if !rendered.is_empty() {
        return Ok(rendered);
    }

    // A single `Part::Text { text: "" }` lands here too: a slice was present and
    // it yielded nothing, which is `SlicesUnreadable`, not an absent reply.
    let kind = if artifacts == 0 && history == 0 && !status_message {
        EmptyKind::NoSliceCarried
    } else {
        EmptyKind::SlicesUnreadable
    };

    Err(TaskRenderEmpty {
        task_id: task.id.clone(),
        context_id: task.context_id.clone(),
        kind,
        artifacts,
        artifact_parts,
        history,
        agent_history,
        agent_history_parts,
        status_message,
        status_message_parts,
    })
}

fn push_rendered_part(part: &Part, out: &mut Vec<String>) {
    match part {
        Part::Text { text, .. } => out.push(text.clone()),
        Part::File { file, .. } => {
            let name = file.name.as_deref().unwrap_or("unnamed");
            out.push(format!("[file: {name}]"));
        }
        Part::Data { .. } => out.push("[data]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Artifact, Message, TaskState, TaskStatus};

    fn agent_message(parts: Vec<Part>) -> Message {
        Message {
            message_id: "msg-1".to_string(),
            role: Role::Agent,
            parts,
            context_id: None,
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        }
    }

    fn completed_task(status_message: Option<Message>) -> Task {
        Task {
            id: "task-951dc60c".to_string(),
            context_id: Some("ctx-révision-2266".to_string()),
            status: TaskStatus {
                state: TaskState::Completed,
                message: status_message,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        }
    }

    fn text(s: &str) -> Part {
        Part::Text {
            text: s.to_string(),
            metadata: None,
        }
    }

    #[test]
    fn text_renders_verbatim() {
        let task = completed_task(Some(agent_message(vec![text("Disposition: READY")])));
        assert_eq!(render_task_text(&task).unwrap(), "Disposition: READY");
    }

    /// A Task built from nothing is the measured mika#2270 shape: `completed`,
    /// three empty slices, and — before the fix — an empty string at exit 0.
    #[test]
    fn a_task_carrying_no_slice_reports_that_it_carries_none() {
        let empty = render_task_text(&completed_task(None)).expect_err("must not render");
        assert_eq!(empty.kind, EmptyKind::NoSliceCarried);
        assert_eq!(empty.task_id, "task-951dc60c");
        assert_eq!(empty.context_id.as_deref(), Some("ctx-révision-2266"));
    }

    /// The mirror case, and the reason the two are distinguished: something *was*
    /// carried, so the investigation starts at this renderer rather than at the
    /// Task's construction.
    #[test]
    fn a_task_whose_slices_yield_nothing_is_a_distinct_kind() {
        let mut task = completed_task(None);
        task.artifacts = Some(vec![Artifact {
            artifact_id: "art-1".to_string(),
            name: None,
            description: None,
            parts: vec![],
            metadata: None,
            extensions: None,
        }]);
        let empty = render_task_text(&task).expect_err("must not render");
        assert_eq!(empty.kind, EmptyKind::SlicesUnreadable);
        assert_eq!(empty.artifacts, 1);
        assert_eq!(empty.artifact_parts, 0);
    }

    /// An empty text part is a slice that yielded nothing, not an absent reply.
    /// It is also the one input that used to reach `join` and come back `""`.
    #[test]
    fn an_empty_text_part_is_unreadable_not_absent() {
        let task = completed_task(Some(agent_message(vec![text("")])));
        let empty = render_task_text(&task).expect_err("must not render");
        assert_eq!(empty.kind, EmptyKind::SlicesUnreadable);
        assert!(empty.status_message);
        assert_eq!(empty.status_message_parts, 1);
    }

    /// The message must name both handles and every slice it inspected. Naming
    /// only "empty" would put the next reader exactly where mika#2270 put this
    /// one.
    #[test]
    fn the_message_names_both_handles_and_every_slice() {
        let empty = render_task_text(&completed_task(None)).expect_err("must not render");
        let rendered = empty.to_string();
        for needle in [
            "task-951dc60c",
            "ctx-révision-2266",
            "artifact",
            "history",
            "status.message",
            "MIKA_SPIRIT_LOG_FILE",
        ] {
            assert!(rendered.contains(needle), "missing {needle}: {rendered}");
        }
    }

    #[test]
    fn a_context_less_task_says_none_rather_than_omitting_the_handle() {
        let mut task = completed_task(None);
        task.context_id = None;
        let empty = render_task_text(&task).expect_err("must not render");
        assert!(empty.to_string().contains("context none"));
    }
}

//! Consumer of mika-spirit's task-events SSE stream (mika#1727 vol V2b).
//!
//! The reading loop below is **not new code**. It was written as a diagnostic
//! stub in `commands/tasks.rs` (mika#1758) and is promoted here verbatim —
//! `find_event_separator` doc-comment included — because the TUI now needs the
//! same bytes for a different purpose. Rewriting it would have thrown away a
//! hard-won property: the three SSE field-block terminators and their try
//! order (`\r\n\r\n` before `\n\n`, `\r\r` last).
//!
//! Two callers, one loop:
//!
//! - `commands/tasks.rs::stream_task_events` (`mika tasks stream`) — the
//!   diagnostic, unfiltered, printing each payload as it arrives;
//! - `tui/app.rs` — the subscription task, filtered on one agent, pushing
//!   frames to the tick.
//!
//! The first is why this module is not a surface nobody runs (mika#2205,
//! mika#2578). That matters here more than usual: the defect this flight
//! corrects *is* "ship the wire, consume it in a follow-up", so shipping a
//! consumer with no consumer would be the same mistake one layer up.

use anyhow::{Context, Result};
use mika_agent::server::tasks_stream::TaskEventFrame;

/// The agent a frame belongs to, when it names one.
///
/// `OverflowMarker` and `Unknown` carry no agent, so they can never satisfy a
/// filter — which is the right answer for a consumer scoped to one agent. The
/// match is exhaustive on purpose: a new variant added to the wire fails to
/// compile here rather than silently falling out of every filtered stream.
fn frame_agent_id(frame: &TaskEventFrame) -> Option<&str> {
    match frame {
        TaskEventFrame::TaskCreated { agent_id, .. }
        | TaskEventFrame::TaskClaimed { agent_id, .. }
        | TaskEventFrame::TaskCompleted { agent_id, .. }
        | TaskEventFrame::TaskFailed { agent_id, .. }
        | TaskEventFrame::TaskDelivered { agent_id, .. }
        | TaskEventFrame::TaskCancelled { agent_id, .. } => Some(agent_id.as_str()),
        TaskEventFrame::OverflowMarker { .. } | TaskEventFrame::Unknown => None,
    }
}

/// The task-events stream URL for a mika-spirit base URL.
pub fn stream_url(base_url: &str) -> String {
    format!(
        "{}/api/v1/dashboard/tasks/stream",
        base_url.trim_end_matches('/')
    )
}

/// An open task-events stream.
///
/// Opening and reading are two steps rather than one call, because both
/// callers need to tell them apart: `mika tasks stream` prints "connected"
/// before the first frame (a connected-but-idle wire must not read as a hung
/// one), and the TUI's subscription task backs off on a failed open while an
/// ended stream is just a reconnect.
pub struct TaskEventStream {
    resp: reqwest::Response,
}

impl TaskEventStream {
    /// Open the stream. `base_url` defaults to
    /// [`crate::commands::dashboard::spirit_url`] and the bearer token to
    /// [`crate::commands::dashboard::auth_token`]. A non-success status fails
    /// explicitly, body included — a stream that silently yields nothing is
    /// indistinguishable from an agent with no tasks.
    pub async fn open(base_url: Option<String>) -> Result<Self> {
        let base_url = base_url.unwrap_or_else(crate::commands::dashboard::spirit_url);
        let url = stream_url(&base_url);
        let token = crate::commands::dashboard::auth_token()?;

        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "text/event-stream")
            .send()
            .await
            .with_context(|| format!("failed to open SSE stream at {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!(
                "mika-spirit returned {} for {}: {}",
                status,
                url,
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(Self { resp })
    }

    /// Read until the server closes the stream, handing every frame to
    /// `on_event`.
    ///
    /// `on_event` receives the `data:` payload **as it came off the wire**
    /// alongside the decoded frame, so a caller that only relays bytes
    /// (`mika tasks stream`) does not depend on this binary knowing every
    /// `event` value — an unknown one lands on [`TaskEventFrame::Unknown`],
    /// which the enum already carries as its forward-compat catch-all.
    ///
    /// `agent_filter` is applied **before** `on_event` sees the frame:
    /// `Some(id)` keeps only that agent's frames (the wire is per-process, not
    /// per-agent — see `server/tasks_stream.rs`), `None` keeps everything.
    /// Returns `Ok(())` on a clean close; the caller decides whether to
    /// reconnect.
    pub async fn consume<F>(mut self, agent_filter: Option<&str>, mut on_event: F) -> Result<()>
    where
        F: FnMut(&str, &TaskEventFrame),
    {
        // Minimal SSE frame parser: buffer bytes, split on double-newline
        // event separator, extract `data: <payload>` lines. Using
        // `Response::chunk()` avoids pulling the `futures-util` `Stream`
        // extension (workspace dep is `default-features = false`).
        let mut buffer: Vec<u8> = Vec::with_capacity(4096);

        while let Some(chunk) = self.resp.chunk().await.context("SSE stream read failed")? {
            buffer.extend_from_slice(&chunk);

            // Split on SSE event separator (`\r\n\r\n` / `\n\n` / `\r\r`, spec
            // § 9.2). Keep the trailing partial event in the buffer for the
            // next iteration.
            while let Some((sep_idx, sep_len)) = find_event_separator(&buffer) {
                let event_bytes: Vec<u8> = buffer.drain(..sep_idx + sep_len).collect();
                // Drop the trailing separator; parse the remainder line-by-line.
                let event_str =
                    String::from_utf8_lossy(&event_bytes[..event_bytes.len() - sep_len]);
                for line in event_str.split(['\n', '\r']) {
                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim_start();
                        if data.is_empty() {
                            continue;
                        }
                        let Ok(frame) = serde_json::from_str::<TaskEventFrame>(data) else {
                            // A payload this binary cannot parse at all — not
                            // an unknown `event` value, which `Unknown`
                            // absorbs. Ignored for the same reason: the stream
                            // is a receipt, and one bad frame must not end it.
                            continue;
                        };
                        if let Some(wanted) = agent_filter
                            && frame_agent_id(&frame) != Some(wanted)
                        {
                            continue;
                        }
                        on_event(data, &frame);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Locate the first SSE event separator in the buffer, and return
/// `(start_index, separator_length)`.
///
/// Per the SSE spec (WHATWG § Server-sent events), the field-block terminator
/// is either `\r\n\r\n` (4 bytes), `\n\n` (2 bytes), or `\r\r` (2 bytes).
/// axum's `Sse` currently emits `\n\n`, but any reverse proxy or TLS
/// terminator on the path may normalise line endings — matching only `\n\n`
/// would make the stream appear to hang indefinitely on those deployments.
fn find_event_separator(buf: &[u8]) -> Option<(usize, usize)> {
    // Check the 4-byte separator first so a `\r\n\r\n` is not matched as an
    // earlier `\n\n` at offset+1.
    if let Some(idx) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((idx, 4));
    }
    if let Some(idx) = buf.windows(2).position(|w| w == b"\n\n") {
        return Some((idx, 2));
    }
    if let Some(idx) = buf.windows(2).position(|w| w == b"\r\r") {
        return Some((idx, 2));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- mika#1727 V2b § 4.4: the parser moved, and it still parses --
    //
    // The function was promoted from `commands/tasks.rs` with no test of its
    // own, so "moved verbatim" was a claim about a diff. These pin the
    // property the doc-comment names: three terminators, tried in that order.

    #[test]
    fn mika1727_v2b_les_trois_separateurs_sont_reconnus() {
        assert_eq!(find_event_separator(b"data: x\n\n"), Some((7, 2)));
        assert_eq!(find_event_separator(b"data: x\r\n\r\n"), Some((7, 4)));
        assert_eq!(find_event_separator(b"data: x\r\r"), Some((7, 2)));
        assert_eq!(find_event_separator(b"data: x\n"), None);
    }

    /// The try order is the load-bearing half: a `\r\n\r\n` must be read as
    /// one 4-byte terminator, never as the `\n\n` sitting at offset+1 — which
    /// would leave a stray `\r` at the head of the next event.
    #[test]
    fn mika1727_v2b_un_crlfcrlf_nest_pas_lu_comme_un_lflf_decale() {
        let (idx, len) = find_event_separator(b"a\r\n\r\nb").unwrap();
        assert_eq!((idx, len), (1, 4));
        assert_eq!(&b"a\r\n\r\nb"[idx + len..], b"b");
    }

    #[test]
    fn mika1727_v2b_le_premier_separateur_gagne() {
        assert_eq!(find_event_separator(b"a\n\nb\n\nc"), Some((1, 2)));
    }

    #[test]
    fn mika1727_v2b_stream_url_ne_double_pas_la_barre() {
        assert_eq!(
            stream_url("http://localhost:8080/"),
            "http://localhost:8080/api/v1/dashboard/tasks/stream"
        );
        assert_eq!(
            stream_url("http://localhost:8080"),
            "http://localhost:8080/api/v1/dashboard/tasks/stream"
        );
    }

    /// The free half of the two-stage filter (D1): a frame of another agent
    /// is dropped on the wire, before any detail round-trip. The variants
    /// that name no agent satisfy no filter.
    #[test]
    fn mika1727_v2b_lagent_dune_trame_est_lu_par_variante() {
        let completed = TaskEventFrame::completed("t1", "mika", "2026-11-08T00:00:00Z", Some("ok"));
        assert_eq!(frame_agent_id(&completed), Some("mika"));

        let failed = TaskEventFrame::failed("t2", "mika-dev", "2026-11-08T00:00:00Z", Some("boom"));
        assert_eq!(frame_agent_id(&failed), Some("mika-dev"));

        assert_eq!(
            frame_agent_id(&TaskEventFrame::OverflowMarker { dropped_count: 3 }),
            None
        );
        assert_eq!(frame_agent_id(&TaskEventFrame::Unknown), None);
    }
}

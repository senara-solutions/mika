//! Bounded webhook queue with backpressure + coalescing (mika#1870).
//!
//! A per-agent bounded queue that sits between `POST /message` ingestion and the
//! agent mailbox. All inbound requests enqueue here (uniform-queue model); a
//! single per-agent drain worker (see `handlers::spawn_webhook_drain_worker`) is
//! the sole consumer that acquires `agent_lock` and runs the agent loop. This
//! replaces the legacy `try_lock_owned()` → 429-reject pattern (kept verbatim
//! behind `MIKA_WEBHOOK_QUEUE_ENABLED=false`, AC9).
//!
//! The queue does three things when it fills:
//!   1. **Coalesce** — a fresh event that shares a [`coalescing_key`] with a
//!      still-queued sibling replaces it (newest-wins). This is the durable,
//!      class-generalized form of mika#1869's `check_suite` burst dedup.
//!   2. **Block** — when full and the event has no coalescing partner, `enqueue`
//!      blocks up to `block_timeout` (default 100ms) waiting for the drain worker
//!      to free a slot.
//!   3. **Drop-oldest** — still full after the block window, the oldest queued
//!      entry is dropped (dead-letter surface, audited by the caller).
//!
//! ## `check_suite` SHA caveat (correctness-critical)
//!
//! mika#1869 keys `check_suite` dedup on `(repo, branch, head_sha)` because a
//! genuine second push advances `head_sha` and must NOT be conflated with the
//! first. **The gateway-formatted text carries `repo` and `branch` but NOT
//! `head_sha`** — the format is `[GitHub] Check suite {conclusion} on {repo}
//! (branch: {branch})`. So a `{repo}:{branch}`-only key can collapse two distinct
//! pushes to the same branch into one — a missed-event hazard.
//!
//! v1 mitigates this with **time-bounded coalescing**: only entries *still in the
//! queue* coalesce. Two pushes seconds apart each get processed as long as the
//! first has already drained before the second arrives — the overwhelmingly
//! common case, because the queue drains continuously. The residual risk (two
//! pushes queued simultaneously) is bounded and no worse than mika#1869's own
//! 60s handler-layer window: the downstream `ci_success_handler` re-aggregates all
//! required checks on every invocation, so a coalesced-away duplicate skips only a
//! redundant walk, never a state transition. Threading `head_sha` end-to-end
//! (companion `mika-gateway` change) is the fully-correct form, deferred to a
//! follow-up (plan §8).
//!
//! ## Scope
//!
//! Single FIFO priority class in v1 (covers the founding `check_suite`-burst
//! incident). Priority classes are deferred (plan §8).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use regex::Regex;
use std::sync::LazyLock;
use tokio::sync::{Mutex, Notify};

use super::types::MessageRequest;
use super::verdict::parse_pr_review_event;

/// Classified webhook event kind, parsed from the gateway-formatted
/// `MessageRequest.text`. Exhaustive so [`coalescing_key`] is compiler-checked:
/// a new variant fails to compile until a coalescing decision is made (AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEventKind {
    /// `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`.
    /// `head_sha` is NOT in the gateway text — see the module-header caveat.
    CheckSuite { repo: String, branch: String },
    /// `[GitHub] PR synchronize: {repo}#{pr} ...` — sync is idempotent-by-state.
    PullRequestSync { repo: String, pr: u64 },
    /// A push event carrying a branch. Reserved: the current gateway does not
    /// route push events into a branch-bearing text format, so [`classify_event`]
    /// does not produce this today. Kept for forward-compat and to keep the
    /// coalescing table complete.
    Push { repo: String, branch: String },
    /// `[GitHub] PR review (...) on {repo}#{n} ...` — user input, never coalesce.
    PrReview,
    /// `[GitHub] Issue labeled {label} on {repo}#{n} ...` (label != `ready`).
    IssueLabeled {
        repo: String,
        issue: u64,
        label: String,
    },
    /// `[GitHub] Issue labeled ready on {repo}#{n} ...` — dispatch trigger, never
    /// coalesce.
    ReadyLabel,
    /// Telegram messages and unclassified GitHub events — preserve order, never
    /// coalesce.
    Other,
}

impl WebhookEventKind {
    /// Stable short label for audit-event / structured-log fields.
    pub fn label(&self) -> &'static str {
        match self {
            WebhookEventKind::CheckSuite { .. } => "check_suite",
            WebhookEventKind::PullRequestSync { .. } => "pr_sync",
            WebhookEventKind::Push { .. } => "push",
            WebhookEventKind::PrReview => "pr_review",
            WebhookEventKind::IssueLabeled { .. } => "issue_labeled",
            WebhookEventKind::ReadyLabel => "ready_label",
            WebhookEventKind::Other => "other",
        }
    }
}

/// `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`.
/// Sibling of `webhook_queue::CHECK_SUITE_RE`; duplicated here to keep the v2
/// module self-contained (the mika#528 module is a different mechanism).
static CHECK_SUITE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[GitHub\] Check suite (\S+) on (\S+) \(branch: ([^)]+)\)")
        .expect("check_suite regex")
});

/// `[GitHub] Issue labeled {label} on {repo}#{issue} ...`. The label is
/// non-greedy up to the first ` on ` so multi-word labels are captured whole.
static ISSUE_LABELED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[GitHub\] Issue labeled (.+?) on (\S+)#(\d+)").expect("issue_labeled regex")
});

/// `[GitHub] PR {action}: {repo}#{pr} ...`. Note the colon after the action —
/// this deliberately does NOT match the `PR review (...)` form (parsed earlier
/// via `parse_pr_review_event`).
static PR_ACTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[GitHub\] PR (\w+): (\S+)#(\d+)").expect("pr_action regex"));

/// The GitHub label whose add-event is a dispatch trigger (never coalesce).
const READY_LABEL: &str = "ready";

/// Classify a gateway-formatted webhook text into a [`WebhookEventKind`] (AC2).
///
/// Pure and total — every input maps to exactly one variant. Non-GitHub text
/// (Telegram, etc.) and unrecognized GitHub events fall to [`WebhookEventKind::Other`].
pub fn classify_event(text: &str) -> WebhookEventKind {
    // Telegram and other non-webhook channels never carry the `[GitHub]` prefix.
    if !text.starts_with("[GitHub]") {
        return WebhookEventKind::Other;
    }

    let first_line = text.lines().next().unwrap_or("");

    // PR review is user input — classify first (its `PR review (` shape is
    // deliberately distinct from the `PR {action}:` form below).
    if parse_pr_review_event(text).is_some() {
        return WebhookEventKind::PrReview;
    }

    if let Some(caps) = CHECK_SUITE_RE.captures(first_line) {
        return WebhookEventKind::CheckSuite {
            repo: caps[2].to_string(),
            branch: caps[3].to_string(),
        };
    }

    if let Some(caps) = ISSUE_LABELED_RE.captures(first_line) {
        let label = caps[1].trim().to_string();
        let repo = caps[2].to_string();
        // `\d+` guarantees a valid u64 up to overflow; treat overflow as Other.
        if let Ok(issue) = caps[3].parse::<u64>() {
            if label == READY_LABEL {
                return WebhookEventKind::ReadyLabel;
            }
            return WebhookEventKind::IssueLabeled { repo, issue, label };
        }
    }

    if let Some(caps) = PR_ACTION_RE.captures(first_line) {
        let action = &caps[1];
        if action == "synchronize"
            && let Ok(pr) = caps[3].parse::<u64>()
        {
            return WebhookEventKind::PullRequestSync {
                repo: caps[2].to_string(),
                pr,
            };
        }
        // opened / closed / review_requested / ready_for_review carry distinct
        // downstream semantics (review, milestone advance) — never coalesce.
        return WebhookEventKind::Other;
    }

    WebhookEventKind::Other
}

/// Coalescing key for an event kind (AC2). `None` = never coalesce.
///
/// Exhaustive match with **no wildcard arm** — adding a [`WebhookEventKind`]
/// variant fails to compile here until a coalescing decision is made.
pub fn coalescing_key(kind: &WebhookEventKind) -> Option<String> {
    match kind {
        // Generalizes mika#1869: N workflows on one branch collapse to 1.
        WebhookEventKind::CheckSuite { repo, branch } => {
            Some(format!("check_suite:{repo}:{branch}"))
        }
        // Sync is idempotent-by-state; newest wins.
        WebhookEventKind::PullRequestSync { repo, pr } => Some(format!("pr_sync:{repo}:{pr}")),
        // Burst-collapse rapid pushes to the same branch.
        WebhookEventKind::Push { repo, branch } => Some(format!("push:{repo}:{branch}")),
        // Re-apply of the same label while still pending.
        WebhookEventKind::IssueLabeled { repo, issue, label } => {
            Some(format!("labeled:{repo}:{issue}:{label}"))
        }
        // Each PR review is user input — must not merge.
        WebhookEventKind::PrReview => None,
        // Dispatch trigger — must not merge.
        WebhookEventKind::ReadyLabel => None,
        // Telegram / unclassified — preserve order, never coalesce.
        WebhookEventKind::Other => None,
    }
}

/// A webhook enqueued into the bounded queue.
pub struct QueuedWebhook {
    /// The original gateway request, replayed by the drain worker.
    pub request: MessageRequest,
    /// Classified kind (audit / structured-log label).
    pub kind: WebhookEventKind,
    /// Coalescing key (`None` = never coalesce), cached to avoid re-classifying.
    pub coalescing_key: Option<String>,
    /// Monotonic id (per-queue `AtomicU64`) — `replaces_event_id` in the coalesce
    /// audit and the newest-wins identity check.
    pub event_id: u64,
    /// When the request was enqueued — `wait_ms` for the dequeue audit + gauge.
    pub enqueued_at: Instant,
}

/// Outcome of an [`WebhookQueue::enqueue`] call.
#[derive(Debug, PartialEq, Eq)]
pub enum EnqueueResult {
    /// Pushed to the back; `depth` is the queue length after the push.
    Enqueued { depth: usize },
    /// Replaced a still-queued sibling sharing the coalescing key.
    Coalesced { replaced_event_id: u64 },
    /// Queue was full past `block_timeout`; the oldest entry was dropped to make
    /// room (dead-letter surface).
    Dropped,
}

/// Per-agent bounded webhook queue (AC1). Thread-safe; always used behind `Arc`.
pub struct WebhookQueue {
    inner: Mutex<VecDeque<QueuedWebhook>>,
    /// Signalled when an item is pushed — wakes a parked drain worker.
    item_available: Notify,
    /// Signalled when an item is popped — wakes a blocked enqueuer (full path).
    slot_available: Notify,
    max_depth: usize,
    block_timeout: Duration,
    next_event_id: AtomicU64,
    // Lifetime counters for the 5s structured-log gauge (best-effort).
    total_enqueued: AtomicU64,
    total_coalesced: AtomicU64,
    total_dropped: AtomicU64,
}

impl WebhookQueue {
    /// Construct a queue with the given depth cap and full-queue block timeout.
    pub fn new(max_depth: usize, block_timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            item_available: Notify::new(),
            slot_available: Notify::new(),
            max_depth,
            block_timeout,
            next_event_id: AtomicU64::new(0),
            total_enqueued: AtomicU64::new(0),
            total_coalesced: AtomicU64::new(0),
            total_dropped: AtomicU64::new(0),
        }
    }

    /// Enqueue a request (AC1). HYBRID algorithm: coalesce → block → drop-oldest.
    pub async fn enqueue(&self, req: MessageRequest) -> EnqueueResult {
        let kind = classify_event(&req.text);
        let key = coalescing_key(&kind);
        let event_id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        let item = QueuedWebhook {
            request: req,
            kind,
            coalescing_key: key,
            event_id,
            enqueued_at: Instant::now(),
        };

        // First attempt: coalesce or push if there is room.
        let item = match self.try_enqueue_once(item).await {
            Ok(result) => return result,
            Err(item) => item,
        };

        // Full: block up to `block_timeout` for the drain worker to free a slot.
        // Register interest before awaiting so a pop between here and the await
        // is not a lost wakeup (Notify stores one permit for notify_one).
        let notified = self.slot_available.notified();
        match tokio::time::timeout(self.block_timeout, notified).await {
            Ok(()) => match self.try_enqueue_once(item).await {
                Ok(result) => result,
                Err(item) => self.drop_oldest_and_push(item).await,
            },
            Err(_elapsed) => self.drop_oldest_and_push(item).await,
        }
    }

    /// Single non-blocking enqueue attempt. Returns `Err(item)` when the queue is
    /// full and the item has no coalescing partner (caller decides block/drop).
    async fn try_enqueue_once(&self, item: QueuedWebhook) -> Result<EnqueueResult, QueuedWebhook> {
        let mut q = self.inner.lock().await;

        // Coalesce against a still-queued sibling (newest-wins, preserves FIFO on
        // the freshest event).
        if let Some(key) = item.coalescing_key.as_deref()
            && let Some(pos) = q
                .iter()
                .position(|e| e.coalescing_key.as_deref() == Some(key))
        {
            let replaced = q.remove(pos).expect("position() guarantees a valid index");
            q.push_back(item);
            drop(q);
            self.total_coalesced.fetch_add(1, Ordering::Relaxed);
            self.item_available.notify_one();
            return Ok(EnqueueResult::Coalesced {
                replaced_event_id: replaced.event_id,
            });
        }

        if q.len() < self.max_depth {
            q.push_back(item);
            let depth = q.len();
            drop(q);
            self.total_enqueued.fetch_add(1, Ordering::Relaxed);
            self.item_available.notify_one();
            return Ok(EnqueueResult::Enqueued { depth });
        }

        Err(item)
    }

    /// Drop the oldest entry and push `item`, keeping depth at the cap. This is
    /// the dead-letter path (caller audits the `Dropped` result).
    async fn drop_oldest_and_push(&self, item: QueuedWebhook) -> EnqueueResult {
        let mut q = self.inner.lock().await;
        // Bounded: `pop_front` removes the oldest before the push, so depth never
        // exceeds `max_depth`. If the queue drained to empty between the full
        // check and here, `pop_front` is a no-op and we simply push.
        q.pop_front();
        q.push_back(item);
        drop(q);
        self.total_dropped.fetch_add(1, Ordering::Relaxed);
        self.item_available.notify_one();
        EnqueueResult::Dropped
    }

    /// Dequeue the oldest entry, awaiting an item when the queue is empty (AC1).
    ///
    /// Cancel-safe: if the returned future is dropped before completion (e.g. a
    /// `tokio::select!` shutdown arm fires), no item is lost — nothing is popped
    /// until an item is present.
    pub async fn dequeue(&self) -> Option<QueuedWebhook> {
        loop {
            // Register as a waiter BEFORE checking, so an `enqueue` notify between
            // the check and the await is not a lost wakeup.
            let notified = self.item_available.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            {
                let mut q = self.inner.lock().await;
                if let Some(item) = q.pop_front() {
                    drop(q);
                    self.slot_available.notify_one();
                    return Some(item);
                }
            }

            notified.await;
        }
    }

    /// Current queue depth.
    pub async fn depth(&self) -> usize {
        self.inner.lock().await.len()
    }

    /// Lifetime counters `(enqueued, coalesced, dropped)` for the gauge.
    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.total_enqueued.load(Ordering::Relaxed),
            self.total_coalesced.load(Ordering::Relaxed),
            self.total_dropped.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(text: &str) -> MessageRequest {
        MessageRequest {
            text: text.to_string(),
            chat_id: None,
            channel: "github".to_string(),
            request_id: "test-req".to_string(),
            agent: "mika-qa".to_string(),
            images: None,
        }
    }

    // -- classify_event table (AC7) --

    #[test]
    fn classify_check_suite() {
        let k =
            classify_event("[GitHub] Check suite success on senara-solutions/mika (branch: main)");
        assert_eq!(
            k,
            WebhookEventKind::CheckSuite {
                repo: "senara-solutions/mika".to_string(),
                branch: "main".to_string(),
            }
        );
    }

    #[test]
    fn classify_pr_review() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#42 (feat: x) by @mika-qa\nhttps://github.com/senara-solutions/mika/pull/42#pullrequestreview-1\n\nVERDICT: pass";
        assert_eq!(classify_event(text), WebhookEventKind::PrReview);
    }

    #[test]
    fn classify_ready_label() {
        let text = "[GitHub] Issue labeled ready on senara-solutions/mika#841 — Gate dispatch\nhttps://github.com/senara-solutions/mika/issues/841";
        assert_eq!(classify_event(text), WebhookEventKind::ReadyLabel);
    }

    #[test]
    fn classify_issue_labeled_non_ready() {
        let text = "[GitHub] Issue labeled p1-important on senara-solutions/mika#900 — Fix\nhttps://github.com/x";
        assert_eq!(
            classify_event(text),
            WebhookEventKind::IssueLabeled {
                repo: "senara-solutions/mika".to_string(),
                issue: 900,
                label: "p1-important".to_string(),
            }
        );
    }

    #[test]
    fn classify_pr_synchronize() {
        let text = "[GitHub] PR synchronize: senara-solutions/mika#42 — feat: x (branch: feat/x)\nhttps://github.com/x";
        assert_eq!(
            classify_event(text),
            WebhookEventKind::PullRequestSync {
                repo: "senara-solutions/mika".to_string(),
                pr: 42,
            }
        );
    }

    #[test]
    fn classify_pr_opened_is_other() {
        let text = "[GitHub] PR opened: senara-solutions/mika#42 — feat: x (branch: feat/x)\nhttps://github.com/x";
        assert_eq!(classify_event(text), WebhookEventKind::Other);
    }

    #[test]
    fn classify_pr_closed_is_other() {
        let text =
            "[GitHub] PR closed: senara-solutions/mika#42 — feat: x (branch: feat/x)\nMerged: true";
        assert_eq!(classify_event(text), WebhookEventKind::Other);
    }

    #[test]
    fn classify_telegram_is_other() {
        assert_eq!(
            classify_event("hey mika, what's on my calendar?"),
            WebhookEventKind::Other
        );
    }

    #[test]
    fn classify_unclassified_github_is_other() {
        assert_eq!(
            classify_event(
                "[GitHub] New comment on senara-solutions/mika#50 (title) by @user\nhttps://x"
            ),
            WebhookEventKind::Other
        );
    }

    // -- coalescing_key never-coalesce invariants (AC7) --

    #[test]
    fn never_coalesce_variants() {
        assert_eq!(coalescing_key(&WebhookEventKind::PrReview), None);
        assert_eq!(coalescing_key(&WebhookEventKind::ReadyLabel), None);
        assert_eq!(coalescing_key(&WebhookEventKind::Other), None);
    }

    #[test]
    fn coalesce_keys_present() {
        assert_eq!(
            coalescing_key(&WebhookEventKind::CheckSuite {
                repo: "o/r".to_string(),
                branch: "main".to_string()
            }),
            Some("check_suite:o/r:main".to_string())
        );
        assert_eq!(
            coalescing_key(&WebhookEventKind::PullRequestSync {
                repo: "o/r".to_string(),
                pr: 7
            }),
            Some("pr_sync:o/r:7".to_string())
        );
        assert_eq!(
            coalescing_key(&WebhookEventKind::Push {
                repo: "o/r".to_string(),
                branch: "main".to_string()
            }),
            Some("push:o/r:main".to_string())
        );
        assert_eq!(
            coalescing_key(&WebhookEventKind::IssueLabeled {
                repo: "o/r".to_string(),
                issue: 3,
                label: "bug".to_string()
            }),
            Some("labeled:o/r:3:bug".to_string())
        );
    }

    // -- WebhookQueue behavior (AC7) --

    const CHECK_SUITE_TEXT: &str =
        "[GitHub] Check suite success on senara-solutions/mika (branch: feat/x)";

    #[tokio::test]
    async fn burst_100_identical_check_suite_collapses_to_depth_1() {
        let queue = WebhookQueue::new(64, Duration::from_millis(100));

        let mut enqueued = 0;
        let mut coalesced = 0;
        for _ in 0..100 {
            match queue.enqueue(req(CHECK_SUITE_TEXT)).await {
                EnqueueResult::Enqueued { .. } => enqueued += 1,
                EnqueueResult::Coalesced { .. } => coalesced += 1,
                EnqueueResult::Dropped => panic!("should never drop while coalescing"),
            }
        }

        assert_eq!(enqueued, 1, "first event enqueues");
        assert_eq!(coalesced, 99, "remaining 99 coalesce");
        assert_eq!(queue.depth().await, 1, "depth stays 1");
        let (e, c, d) = queue.counters();
        assert_eq!((e, c, d), (1, 99, 0));
    }

    #[tokio::test]
    async fn overflow_200_distinct_keeps_64_drops_136() {
        // block_timeout=0 so full-queue enqueues drop immediately (no wall wait).
        let queue = WebhookQueue::new(64, Duration::ZERO);

        let mut enqueued = 0;
        let mut dropped = 0;
        for i in 0..200 {
            // Distinct `Other` events (Telegram-shaped) — never coalesce.
            match queue.enqueue(req(&format!("distinct message {i}"))).await {
                EnqueueResult::Enqueued { .. } => enqueued += 1,
                EnqueueResult::Dropped => dropped += 1,
                EnqueueResult::Coalesced { .. } => panic!("Other events must not coalesce"),
            }
        }

        assert_eq!(enqueued, 64, "queue fills to the cap");
        assert_eq!(
            dropped, 136,
            "the remaining 136 drop the oldest (dead-letter)"
        );
        assert_eq!(queue.depth().await, 64, "depth held at the cap");
        let (_, _, d) = queue.counters();
        assert_eq!(d, 136, "136 drop audit intents recorded");
    }

    #[tokio::test]
    async fn coalescing_correctness_newest_wins() {
        let queue = WebhookQueue::new(64, Duration::from_millis(100));
        let text = "[GitHub] PR synchronize: senara-solutions/mika#42 — feat: x (branch: feat/x)\nhttps://x";

        let r0 = queue.enqueue(req(text)).await;
        assert!(matches!(r0, EnqueueResult::Enqueued { depth: 1 }));
        let r1 = queue.enqueue(req(text)).await;
        assert!(matches!(
            r1,
            EnqueueResult::Coalesced {
                replaced_event_id: 0
            }
        ));
        let r2 = queue.enqueue(req(text)).await;
        assert!(matches!(
            r2,
            EnqueueResult::Coalesced {
                replaced_event_id: 1
            }
        ));

        assert_eq!(queue.depth().await, 1);
        // The single retained entry is the newest (event_id 2).
        let item = queue.dequeue().await.unwrap();
        assert_eq!(item.event_id, 2);
    }

    #[tokio::test]
    async fn two_pr_reviews_both_retained() {
        let queue = WebhookQueue::new(64, Duration::from_millis(100));
        let mk = |n: u64| {
            format!(
                "[GitHub] PR review (approved) on senara-solutions/mika#{n} (t) by @qa\nhttps://x\n\nVERDICT: pass"
            )
        };
        queue.enqueue(req(&mk(1))).await;
        queue.enqueue(req(&mk(2))).await;
        assert_eq!(queue.depth().await, 2, "PR reviews never coalesce");
    }

    #[tokio::test]
    async fn accumulate_then_drain_fifo_order() {
        // Models the drain-worker resume path at the queue level: accumulate
        // while nothing consumes, then drain and assert FIFO order.
        let queue = WebhookQueue::new(64, Duration::from_millis(100));
        for i in 0..10 {
            queue.enqueue(req(&format!("msg {i}"))).await;
        }
        assert_eq!(queue.depth().await, 10);

        for i in 0..10 {
            let item = queue.dequeue().await.unwrap();
            assert_eq!(
                item.request.text,
                format!("msg {i}"),
                "FIFO order preserved"
            );
        }
        assert_eq!(queue.depth().await, 0);
    }

    #[tokio::test]
    async fn dequeue_blocks_until_enqueue() {
        use std::sync::Arc;
        let queue = Arc::new(WebhookQueue::new(64, Duration::from_millis(100)));

        let q2 = queue.clone();
        let handle = tokio::spawn(async move { q2.dequeue().await.map(|i| i.request.text) });

        // Give the consumer a moment to park on the empty queue.
        tokio::task::yield_now().await;
        queue.enqueue(req("late arrival")).await;

        let got = handle.await.unwrap();
        assert_eq!(got, Some("late arrival".to_string()));
    }
}

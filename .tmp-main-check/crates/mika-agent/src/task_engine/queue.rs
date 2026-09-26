use std::cmp::Ordering;

/// A task queued in the `BinaryHeap` inside `TaskEngine`.
///
/// Implements `Ord` in reverse order so the heap acts as a **min-heap** by
/// `next_fire_at` — the task with the smallest (earliest) fire time is always
/// at the top and gets popped first.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct QueuedTask {
    pub task_id: String,
    pub next_fire_at: String,
    /// "time", "recurring", "callback", "user_reply", "inject_context"
    pub trigger_type: String,
    pub action_type: String,
    /// For recurring tasks: the cron expression to compute the next fire time.
    pub cron_expr: Option<String>,
}

impl Ord for QueuedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse comparison → min-heap
        other
            .next_fire_at
            .cmp(&self.next_fire_at)
            .then_with(|| other.task_id.cmp(&self.task_id))
    }
}

impl PartialOrd for QueuedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BinaryHeap;

    fn qt(id: &str, fire_at: &str) -> QueuedTask {
        QueuedTask {
            task_id: id.to_string(),
            next_fire_at: fire_at.to_string(),
            trigger_type: "time".to_string(),
            action_type: "send_message".to_string(),
            cron_expr: None,
        }
    }

    #[test]
    fn test_min_heap_ordering() {
        let mut heap = BinaryHeap::new();
        heap.push(qt("c", "2026-01-01T03:00:00Z"));
        heap.push(qt("a", "2026-01-01T01:00:00Z"));
        heap.push(qt("b", "2026-01-01T02:00:00Z"));

        // Min-heap: smallest fire_at pops first
        assert_eq!(heap.pop().unwrap().next_fire_at, "2026-01-01T01:00:00Z");
        assert_eq!(heap.pop().unwrap().next_fire_at, "2026-01-01T02:00:00Z");
        assert_eq!(heap.pop().unwrap().next_fire_at, "2026-01-01T03:00:00Z");
    }

    #[test]
    fn test_peek_returns_earliest() {
        let mut heap = BinaryHeap::new();
        heap.push(qt("z", "2026-01-01T23:59:59Z"));
        heap.push(qt("a", "2026-01-01T00:00:01Z"));

        assert_eq!(heap.peek().unwrap().next_fire_at, "2026-01-01T00:00:01Z");
    }
}

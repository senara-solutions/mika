use anyhow::{Result, anyhow};
use cron::Schedule;
use std::str::FromStr;

/// Compute the next fire timestamp (UTC unix seconds) for a cron expression,
/// strictly after the given `after_unix` timestamp.
///
/// Expects 6-field cron format: `sec min hour day month weekday`
/// e.g. `0 30 9 * * *` = every day at 09:30:00 UTC
pub fn next_fire_from_cron(expr: &str, after_unix: i64) -> Result<i64> {
    let schedule = Schedule::from_str(expr)
        .map_err(|e| anyhow!("invalid cron expression '{}': {}", expr, e))?;

    let after_dt = chrono::DateTime::from_timestamp(after_unix, 0)
        .ok_or_else(|| anyhow!("invalid unix timestamp: {}", after_unix))?;

    let next = schedule
        .after(&after_dt)
        .next()
        .ok_or_else(|| anyhow!("cron expression '{}' has no future occurrences", expr))?;

    Ok(next.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_fire_after_now() {
        // "every minute" — should always have a next occurrence
        let now = chrono::Utc::now().timestamp();
        let next = next_fire_from_cron("0 * * * * *", now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_next_fire_strict_after() {
        let base = 1_700_000_000i64; // some fixed timestamp
        let next = next_fire_from_cron("0 * * * * *", base).unwrap();
        assert!(next > base);
        // Should be at most 60 seconds later (next minute boundary)
        assert!(next <= base + 60);
    }

    #[test]
    fn test_invalid_expr_returns_error() {
        assert!(next_fire_from_cron("not a cron expr", 0).is_err());
    }

    #[test]
    fn test_daily_at_2am() {
        // 0 0 2 * * * = every day at 02:00:00 UTC
        let now = chrono::Utc::now().timestamp();
        let next = next_fire_from_cron("0 0 2 * * *", now).unwrap();
        assert!(next > now);
        // Should be within 24 hours
        assert!(next <= now + 86_400);
    }
}

use anyhow::{Result, anyhow};
use cron::Schedule;
use std::str::FromStr;

/// Compute the next fire timestamp (UTC ISO 8601 string) for a cron expression,
/// strictly after the given `after` timestamp.
///
/// Expects 6-field cron format: `sec min hour day month weekday`
/// e.g. `0 30 9 * * *` = every day at 09:30:00 UTC
pub fn next_fire_from_cron(expr: &str, after: &str) -> Result<String> {
    let schedule = Schedule::from_str(expr)
        .map_err(|e| anyhow!("invalid cron expression '{}': {}", expr, e))?;

    let after_dt = crate::timestamp::parse(after)?;

    let next = schedule
        .after(&after_dt)
        .next()
        .ok_or_else(|| anyhow!("cron expression '{}' has no future occurrences", expr))?;

    Ok(crate::timestamp::format(&next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_fire_after_now() {
        // "every minute" — should always have a next occurrence
        let now = crate::timestamp::now();
        let next = next_fire_from_cron("0 * * * * *", &now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_next_fire_strict_after() {
        let base = "2023-11-14T22:13:20Z"; // some fixed timestamp
        let next = next_fire_from_cron("0 * * * * *", base).unwrap();
        assert!(next > base.to_string());
        // Should be at most 60 seconds later (next minute boundary)
        let base_dt = crate::timestamp::parse(base).unwrap();
        let next_dt = crate::timestamp::parse(&next).unwrap();
        let diff = next_dt.signed_duration_since(base_dt).num_seconds();
        assert!(diff > 0 && diff <= 60);
    }

    #[test]
    fn test_invalid_expr_returns_error() {
        assert!(next_fire_from_cron("not a cron expr", "2026-01-01T00:00:00Z").is_err());
    }

    #[test]
    fn test_daily_at_2am() {
        // 0 0 2 * * * = every day at 02:00:00 UTC
        let now = crate::timestamp::now();
        let next = next_fire_from_cron("0 0 2 * * *", &now).unwrap();
        assert!(next > now);
        // Should be within 24 hours
        let now_dt = crate::timestamp::parse(&now).unwrap();
        let next_dt = crate::timestamp::parse(&next).unwrap();
        let diff = next_dt.signed_duration_since(now_dt).num_seconds();
        assert!(diff > 0 && diff <= 86_400);
    }
}

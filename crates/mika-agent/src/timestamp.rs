use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, Utc};

/// ISO 8601 UTC format string used for all SQLite TEXT timestamp columns.
pub const DB_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// Return the current UTC time as an ISO 8601 string.
pub fn now() -> String {
    Utc::now().format(DB_FORMAT).to_string()
}

/// Format a `DateTime<Utc>` as an ISO 8601 string.
pub fn format(dt: &DateTime<Utc>) -> String {
    dt.format(DB_FORMAT).to_string()
}

/// Parse an ISO 8601 string into a `DateTime<Utc>`.
pub fn parse(s: &str) -> Result<DateTime<Utc>> {
    // Try full ISO 8601 first (with Z suffix)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, DB_FORMAT) {
        return Ok(dt.and_utc());
    }
    // Fallback: try RFC 3339
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow!("invalid timestamp '{}': {}", s, e))
}

/// Return a timestamp string for `now - duration`.
pub fn now_minus(d: Duration) -> String {
    format(&(Utc::now() - d))
}

/// Return a timestamp string for `now + duration`.
pub fn now_plus(d: Duration) -> String {
    format(&(Utc::now() + d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_roundtrips() {
        let s = now();
        let dt = parse(&s).unwrap();
        assert_eq!(format(&dt), s);
    }

    #[test]
    fn test_parse_iso8601() {
        let dt = parse("2026-03-17T14:30:00Z").unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 3);
    }

    #[test]
    fn test_now_plus_minus() {
        let plus = now_plus(Duration::seconds(3600));
        let minus = now_minus(Duration::seconds(3600));
        let plus_dt = parse(&plus).unwrap();
        let minus_dt = parse(&minus).unwrap();
        assert!(plus_dt > minus_dt);
    }

    use chrono::Datelike;
}

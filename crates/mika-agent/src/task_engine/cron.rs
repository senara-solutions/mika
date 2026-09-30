use anyhow::{Result, anyhow};
use chrono::Utc;
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;

/// Extract the timezone IANA name from task metadata JSON.
///
/// Metadata is expected to be `{"timezone": "Asia/Singapore"}`.
/// Returns `None` if metadata is absent, not valid JSON, or has no timezone key.
pub fn extract_timezone_from_metadata(metadata: Option<&str>) -> Option<String> {
    metadata.and_then(|m| {
        serde_json::from_str::<serde_json::Value>(m)
            .ok()
            .and_then(|v| v["timezone"].as_str().map(String::from))
    })
}

/// Parse an IANA timezone string into a `Tz`, returning a user-friendly error.
pub fn parse_timezone(timezone: &str) -> Result<Tz> {
    timezone.parse().map_err(|_| {
        anyhow!(
            "invalid timezone '{}'. Example: Asia/Singapore, America/New_York, Europe/London",
            timezone
        )
    })
}

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

/// Compute the next fire timestamp (UTC ISO 8601 string) for a cron expression
/// evaluated in the given timezone, strictly after the given `after` UTC timestamp.
///
/// The cron expression fields (hour, day-of-week, etc.) are interpreted in the
/// provided timezone. The result is converted back to UTC for storage.
/// This correctly handles DST transitions.
pub fn next_fire_from_cron_tz(expr: &str, after: &str, tz: &Tz) -> Result<String> {
    let schedule = Schedule::from_str(expr)
        .map_err(|e| anyhow!("invalid cron expression '{}': {}", expr, e))?;

    let after_dt = crate::timestamp::parse(after)?;
    let after_local = after_dt.with_timezone(tz);

    let next_local = schedule
        .after(&after_local)
        .next()
        .ok_or_else(|| anyhow!("cron expression '{}' has no future occurrences", expr))?;

    let next_utc = next_local.with_timezone(&Utc);
    Ok(crate::timestamp::format(&next_utc))
}

/// Le **lecteur unique** du calcul de l'instant de tir d'une récurrente
/// (mika#2575).
///
/// Le triplet `metadata → timezone → next_fire_from_cron{,_tz}` existait deux
/// fois — `fire_task` (repos nominal après un tir) et `enqueue_queued_task`
/// (mise en tas) — et le ré-armement au démarrage en aurait été la troisième.
/// Trois formulations d'un même calcul divergent, et la divergence est
/// silencieuse : chacune rend un instant plausible.
///
/// **Sémantique, identique aux deux sites qu'il remplace.** `cron_expr` absent
/// ⇒ `Err` nommant l'absence ; timezone lue dans `metadata` par
/// [`extract_timezone_from_metadata`] puis [`parse_timezone`], **repli UTC sans
/// erreur** si elle est absente ou illisible — un fuseau qu'on ne sait pas lire
/// ne doit pas empêcher une récurrente de se replanifier.
///
/// **Chaque appelant garde sa disposition d'erreur** : `fire_task` et le
/// ré-armement marquent `failed`, `enqueue_queued_task` renonce à empiler. Ce
/// helper calcule ; il ne décide de rien.
pub fn next_fire_for_recurring(
    cron_expr: Option<&str>,
    metadata: Option<&str>,
    now: &str,
) -> Result<String> {
    let expr = cron_expr.ok_or_else(|| anyhow!("recurring task missing cron_expr"))?;

    match extract_timezone_from_metadata(metadata).and_then(|tz| parse_timezone(&tz).ok()) {
        Some(tz) => next_fire_from_cron_tz(expr, now, &tz),
        None => next_fire_from_cron(expr, now),
    }
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
        assert!(next.as_str() > base);
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

    // --- Timezone-aware cron tests ---

    #[test]
    fn test_cron_tz_singapore_9am_local() {
        // "every day at 9am" in Asia/Singapore (UTC+8)
        // After 2026-03-30T00:00:00Z (= 2026-03-30 08:00 SGT)
        // Next 9am SGT = 2026-03-30T01:00:00Z
        let after = "2026-03-30T00:00:00Z";
        let tz = parse_timezone("Asia/Singapore").unwrap();
        let next = next_fire_from_cron_tz("0 0 9 * * *", after, &tz).unwrap();
        assert_eq!(next, "2026-03-30T01:00:00Z");
    }

    #[test]
    fn test_cron_tz_new_york_9am_local() {
        // "every day at 9am" in America/New_York (UTC-4 during EDT)
        // After 2026-03-30T12:00:00Z (= 2026-03-30 08:00 EDT)
        // Next 9am EDT = 2026-03-30T13:00:00Z
        let after = "2026-03-30T12:00:00Z";
        let tz = parse_timezone("America/New_York").unwrap();
        let next = next_fire_from_cron_tz("0 0 9 * * *", after, &tz).unwrap();
        assert_eq!(next, "2026-03-30T13:00:00Z");
    }

    #[test]
    fn test_cron_tz_day_boundary_crossing() {
        // User in UTC+8 wants "every day at 1am" local.
        // 1am SGT = 5pm UTC previous day — crosses the date boundary.
        // After 2026-04-01T16:00:00Z (= 2026-04-02 00:00 SGT, midnight)
        // Next 1am SGT = 2026-04-01T17:00:00Z (April 2 01:00 SGT)
        let after = "2026-04-01T16:00:00Z";
        let tz = parse_timezone("Asia/Singapore").unwrap();
        let next = next_fire_from_cron_tz("0 0 1 * * *", after, &tz).unwrap();
        assert_eq!(next, "2026-04-01T17:00:00Z");
    }

    #[test]
    fn test_cron_tz_dst_transition() {
        // America/New_York switches from EST (UTC-5) to EDT (UTC-4) on 2026-03-08
        // "every day at 9am" before DST = 14:00 UTC, after DST = 13:00 UTC
        // After 2026-03-07T14:00:00Z (= 2026-03-07 09:00 EST, just fired)
        // Next 9am local = 2026-03-08 09:00 EDT = 2026-03-08T13:00:00Z
        let after = "2026-03-07T14:00:00Z";
        let tz = parse_timezone("America/New_York").unwrap();
        let next = next_fire_from_cron_tz("0 0 9 * * *", after, &tz).unwrap();
        assert_eq!(next, "2026-03-08T13:00:00Z");
    }

    #[test]
    fn test_parse_timezone_invalid() {
        let result = parse_timezone("Not/A/Zone");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid timezone"));
    }

    #[test]
    fn test_cron_tz_falls_back_correctly_with_utc() {
        // When timezone is UTC, should produce same result as next_fire_from_cron
        let after = "2026-03-30T00:00:00Z";
        let utc_result = next_fire_from_cron("0 0 9 * * *", after).unwrap();
        let tz = parse_timezone("UTC").unwrap();
        let tz_result = next_fire_from_cron_tz("0 0 9 * * *", after, &tz).unwrap();
        assert_eq!(utc_result, tz_result);
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2575 — le lecteur unique du calcul de tir d'une récurrente.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn mika2575_le_lecteur_unique_calcule_en_utc_sans_metadata() {
        let after = "2026-03-30T00:00:00Z";
        let direct = next_fire_from_cron("0 0 9 * * *", after).unwrap();
        let through = next_fire_for_recurring(Some("0 0 9 * * *"), None, after).unwrap();
        assert_eq!(direct, through);
    }

    #[test]
    fn mika2575_le_lecteur_unique_honore_la_timezone_du_metadata() {
        let after = "2026-03-30T00:00:00Z";
        let next = next_fire_for_recurring(
            Some("0 0 9 * * *"),
            Some(r#"{"timezone":"Asia/Singapore"}"#),
            after,
        )
        .unwrap();
        // 9h SGT (UTC+8) le 30/03 = 01:00Z — la même valeur que le site
        // timezone-aware qu'il remplace.
        assert_eq!(next, "2026-03-30T01:00:00Z");
    }

    #[test]
    fn mika2575_un_cron_absent_rend_une_erreur_qui_nomme_labsence() {
        let err = next_fire_for_recurring(None, None, "2026-01-01T00:00:00Z")
            .expect_err("cron_expr absent doit rendre Err");
        assert!(
            err.to_string().contains("cron_expr"),
            "le message doit nommer l'absence : {err}"
        );
    }

    #[test]
    fn mika2575_un_cron_illisible_rend_une_erreur() {
        assert!(
            next_fire_for_recurring(Some("pas un cron"), None, "2026-01-01T00:00:00Z").is_err()
        );
    }

    /// **Le repli est UTC, jamais une erreur.** Un fuseau illisible ne doit pas
    /// empêcher une récurrente de se replanifier — c'est la sémantique des deux
    /// sites remplacés (`.ok()` y jette l'erreur de parsing), et l'inverser
    /// ferait retomber sur `failed` toute récurrente dont le metadata a été
    /// édité de travers.
    #[test]
    fn mika2575_une_timezone_illisible_retombe_sur_utc_sans_erreur() {
        let after = "2026-03-30T00:00:00Z";
        let utc = next_fire_from_cron("0 0 9 * * *", after).unwrap();
        let fallback = next_fire_for_recurring(
            Some("0 0 9 * * *"),
            Some(r#"{"timezone":"Not/A/Zone"}"#),
            after,
        )
        .unwrap();
        assert_eq!(utc, fallback);
    }

    #[test]
    fn test_extract_timezone_from_metadata() {
        assert_eq!(
            extract_timezone_from_metadata(Some(r#"{"timezone":"Asia/Singapore"}"#)),
            Some("Asia/Singapore".to_string())
        );
        assert_eq!(
            extract_timezone_from_metadata(Some(r#"{"other":"val"}"#)),
            None
        );
        assert_eq!(extract_timezone_from_metadata(Some("not json")), None);
        assert_eq!(extract_timezone_from_metadata(None), None);
    }
}

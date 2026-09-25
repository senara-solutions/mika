//! Is the `config.toml` this process booted on still the one on disk?
//! (mika#2473 D2)
//!
//! # The question this module answers, and the one it does not
//!
//! `Settings` are frozen at `init_agent` (mika#1962, mika#2290, mika#2457 U2):
//! an agent serves the configuration it read when it started, and an edit to
//! `{agent_home}/config.toml` takes effect at the **next restart**, not at the
//! next turn. That contract is deliberate and this module does not touch it.
//! What it removes is the *silence* around it. Three dated edits on this
//! workstation moved a model under a running spirit, and nothing anywhere said
//! that the file on disk and the values in service had parted company — the
//! operator learned it from a verdict, days later.
//!
//! So: one `stat` per turn, and **one** re-resolution per distinct mtime. The
//! finding names both sides — what the disk says now, what the process serves —
//! and says whether a restart would change anything. It never refuses a turn
//! (KTD1) — and *nothing on this path may panic either*, which is why the
//! mtime is formatted by a fallible conversion with a named fallback rather
//! than by `DateTime::<Utc>::from(SystemTime)`, whose `.unwrap()` dies on an
//! instant a filesystem will happily store. It never hot-reloads: a detector
//! that swapped the note for the disk's record would make the process report a
//! value it has never loaded, which is mika#2304's defect one field over.
//!
//! # Why the mtime and not the content (KTD4)
//!
//! Hashing the file would cost a read per turn, on every turn, to answer a
//! question whose answer is "no" virtually always. The mtime is a metadata
//! read; the content is only parsed once the metadata says something moved.
//! The cost of the false positive — a `touch` with no edit — is one INFO line
//! saying exactly that, which is cheaper than the read it avoids.
//!
//! # `restart_required` answers "a value in service differs"
//!
//! Narrower than "the record differs", and the narrowing is the point.
//! [`ResolvedBudgetRecord`] also carries `*_source` and `*_raw` — the cascade
//! door a value came through and the string as written — and neither is a value
//! the process serves. Comparing the whole record made a setting moved from the
//! agent's `config.toml` to the global one **at the same value** raise the loud
//! arm and tell an operator to restart for a change a restart would not apply.
//! [`clear_dating_and_provenance`] neutralises them alongside `resolved_at`.
//!
//! # `restart_required = false` is still not "nothing changed"
//!
//! [`ResolvedBudgetRecord`] carries the budget and the model. It does **not**
//! carry `openrouter_base_url`, `zai_base_url` or `log_level` — and an edit to
//! those is a real gesture on these agents, one that changes the endpoint being
//! served. Such an edit lands in the `false` arm. The message therefore says
//! *"no field of the budget/model record moved — another field of the file may
//! still require a restart"*, never *"nothing effective moved"*: the second
//! wording would leave an operator on the old endpoint while telling them all
//! is well. A door-only move lands there too, and that sentence is exactly true
//! of it.
//!
//! # The unreadable `stat` is said twice, not every turn
//!
//! Its arm consulted no state and wrote none, so a `config.toml` deleted under a
//! running spirit produced one WARN **per turn, for ever** — on the arm whose
//! expected regime is zero, which is the churn mika#2131 bounds. It is now
//! bounded to the onset and the recovery (see [`report_unreadable_stat`]), and
//! the bound lives in its own set rather than in [`BootNote::reported`], which
//! carries the population semantics and must stay exactly as documented.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use super::budget_provenance::{ResolvedBudgetRecord, resolve_llm_budget_record};

/// The per-agent file D2 watches.
///
/// Deliberately **not** the per-agent `.env`: D2 exists for the file the three
/// dated edits on this workstation touched, and one watched path is one `stat`.
/// The `.env` is covered at boot by D1 (R3), which reads the whole cascade.
const AGENT_CONFIG_FILE: &str = "config.toml";

/// Whether this agent's change has already been reported, and at which mtime.
///
/// # Why this is not an `Option<SystemTime>`
///
/// Because `mtime` beside it **is** one, and a `stat` that fails yields `None`.
/// Were `reported` an `Option` too, a failed reading would compare equal to
/// "nothing reported yet" and read as *already reported* — so a `config.toml`
/// present at boot and then deleted or made unreadable would never be signalled
/// at all. That is a silent blindness of exactly the shape this module exists
/// to remove, and the house has ruled the other way twice: mika#2277 takes an
/// unreadable liveness signal **out** of the population under its own name, and
/// mika#2328 gives an unreadable provider the distinct word
/// [`super::MODEL_SOURCE_UNKNOWN_PROVIDER`] rather than folding it into
/// `default`.
///
/// [`Reported::covers`] takes a `SystemTime`, not an `Option<SystemTime>`, so a
/// missed reading cannot even be handed to it. The collision is closed by the
/// types, not by the order of the branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reported {
    /// No change has been reported for this agent yet.
    Never,
    /// The change at this mtime has been reported once.
    At(SystemTime),
}

impl Reported {
    /// Has the change at `mtime` already been reported?
    fn covers(self, mtime: SystemTime) -> bool {
        matches!(self, Self::At(at) if at == mtime)
    }
}

/// What one agent's `init_agent` saw: the file's instant, where to re-read it,
/// and the record the process actually serves.
///
/// Taken at `init_agent`, where the record is resolved — **not** at the first
/// turn, which could already be later than an edit and would then note the new
/// state as if the process were serving it (KTD4).
struct BootNote {
    /// The `config.toml`'s mtime at boot, or `None` when it could not be read.
    mtime: Option<SystemTime>,
    /// Where the shared `config.toml` lives — needed to re-resolve the cascade.
    global_home: PathBuf,
    /// Where this agent's `config.toml` lives.
    agent_home: PathBuf,
    /// The record **in service**: what this process loaded and runs under.
    record: ResolvedBudgetRecord,
    /// The last mtime reported, so one edit is said once (R9).
    reported: Reported,
}

/// Boot notes by `agent_id`, the sibling of `budget_provenance`'s `LAST_EMITTED`.
///
/// Keyed by `agent_id` **alone**, which is what defines D2's exempt population:
/// an agent this process never passed through `init_agent` holds no note, so
/// [`detect_config_change`] returns `None` for it. That is not "team runs are
/// exempt" — a team run whose agent was initialised here is watched like any
/// other turn (R9).
static BOOT_NOTES: OnceLock<Mutex<HashMap<String, BootNote>>> = OnceLock::new();

fn boot_notes() -> &'static Mutex<HashMap<String, BootNote>> {
    BOOT_NOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The mtime of `{agent_home}/config.toml`, or `None` when it cannot be read.
///
/// Synchronous `std::fs`, like [`super::budget_provenance`]'s own cascade
/// reads: this is one metadata lookup, not a content read.
fn config_mtime(agent_home: &Path) -> Option<SystemTime> {
    std::fs::metadata(agent_home.join(AGENT_CONFIG_FILE))
        .and_then(|meta| meta.modified())
        .ok()
}

/// Note, at `init_agent`, the state this process is about to serve (mika#2473).
///
/// Called where the record is resolved, so the note and the record describe the
/// same instant. Re-noting an agent resets its `reported` marker, which is the
/// right reading: a fresh `init_agent` is a fresh process state, and whatever
/// was reported against the previous one is spent.
pub fn note_config_at_boot(
    agent_id: &str,
    global_home: &Path,
    agent_home: &Path,
    record: &ResolvedBudgetRecord,
) {
    let note = BootNote {
        mtime: config_mtime(agent_home),
        global_home: global_home.to_path_buf(),
        agent_home: agent_home.to_path_buf(),
        record: record.clone(),
        reported: Reported::Never,
    };

    boot_notes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(agent_id.to_string(), note);
}

/// One agent's `config.toml` has moved since this process read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChangedSinceBoot {
    /// The agent whose file moved.
    pub agent_id: String,
    /// The file's new mtime — RFC 3339 UTC, the format
    /// [`ResolvedBudgetRecord::resolved_at`] uses, so the two are comparable by
    /// eye in a log.
    ///
    /// [`CONFIG_MTIME_UNREPRESENTABLE`] when the instant the filesystem returned
    /// is outside chrono's range. The reading is **named**, never invented and
    /// never fatal: this field is built by [`format_config_mtime`], which cannot
    /// panic (KTD1), and the rest of the finding stands.
    pub config_mtime: String,
    /// The provider the file declares **now**.
    pub provider_on_disk: String,
    /// The model the file declares **now**.
    pub model_on_disk: String,
    /// The provider this process is actually serving.
    pub provider_in_service: String,
    /// The model this process is actually serving.
    pub model_in_service: String,
    /// One of `llm_http_timeout_secs`, `agent_total_timeout_secs`,
    /// `llm_max_tokens` differs — reported apart from `restart_required`
    /// because the two answer different questions: *what kind* of edit, and
    /// *does it matter yet*.
    pub budget_changed: bool,
    /// A field of the budget/model record differs, so the values in service are
    /// not the values on disk. **Never** a refusal — a fact (KTD1).
    pub restart_required: bool,
}

/// Has this agent's `config.toml` moved since boot, and does it matter?
///
/// `None` — costing one `stat` — in four cases, and the fourth is the one worth
/// naming:
///
/// 1. no boot note for this agent (this process never initialised it);
/// 2. the mtime is the one noted at boot;
/// 3. this mtime has already been reported once (R9: one re-resolution per
///    distinct mtime, not one per turn);
/// 4. **the `stat` failed.** That takes the turn *out of the population*: it is
///    not a satisfied term. The reading is announced under its own name,
///    `agent_config_mtime_unreadable` (expected regime: zero) — **once on the
///    onset and once on the recovery**, never once per turn (see
///    [`report_unreadable_stat`]) — and [`BootNote::reported`] is left
///    **intact**, so the moment the file is readable again, the change is
///    reported normally. See [`Reported`] for why the types make this the only
///    possible reading.
///
/// Otherwise the cascade is re-resolved **from disk** through
/// [`resolve_llm_budget_record`] — the single constructor of the record
/// (mika#2457), called and not duplicated — and compared to the boot record
/// with its dating and provenance cleared on both sides (see
/// [`clear_dating_and_provenance`]): `resolved_at` dates the resolution, and the
/// `*_source` / `*_raw` fields say through which cascade door a value came.
/// None of them is a value the process serves, so leaving them in would make a
/// setting moved between doors **at the same value** read as a restart-requiring
/// change.
///
/// # What holds the lock, and for how long
///
/// [`BOOT_NOTES`] is shared by every agent this process serves. The guard is
/// taken twice, briefly: once to clone what this turn needs, once to record the
/// report — and **never across the `stat` or the re-resolution**, which are
/// blocking filesystem I/O. The gap between the two acquisitions is a real race,
/// closed by re-checking [`Reported::covers`] under the second one: a turn that
/// loses it returns `None`, because the turn that won is reporting the same edit.
///
/// The boot note's record is **not** replaced. The process keeps serving what
/// it loaded; this function says so, it does not change it.
pub fn detect_config_change(agent_id: &str) -> Option<ConfigChangedSinceBoot> {
    // Everything this turn needs from the note, taken in one short critical
    // section. The guard is then DROPPED — `BOOT_NOTES` is shared by every
    // agent this process serves, and the two operations below are blocking
    // filesystem I/O: one `stat`, then `resolve_llm_budget_record`, which walks
    // the cascade with up to three more synchronous reads. Holding a global
    // mutex across them makes one slow or hanging `config.toml` serialise every
    // other agent's turn, with no timeout, on the path every turn takes. The
    // unreadable-`stat` arm below already dropped before its WARN; this extends
    // the same discipline to the whole body, and mirrors the drop-then-relock
    // `emit_llm_budget_resolved` uses on its own `LAST_EMITTED` map.
    let (boot_mtime, global_home, agent_home, in_service, reported) = {
        let notes = boot_notes()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let note = notes.get(agent_id)?;
        (
            note.mtime,
            note.global_home.clone(),
            note.agent_home.clone(),
            note.record.clone(),
            note.reported,
        )
    };

    let path = agent_home.join(AGENT_CONFIG_FILE);
    let current = match std::fs::metadata(&path).and_then(|meta| meta.modified()) {
        Ok(mtime) => mtime,
        Err(err) => {
            report_unreadable_stat(agent_id, &path, &err);
            return None;
        }
    };

    if boot_mtime == Some(current) || reported.covers(current) {
        forget_unreadable_stat(agent_id);
        return None;
    }

    let on_disk = resolve_llm_budget_record(agent_id, &global_home, &agent_home);

    // `resolved_at` dates the record, it is not part of it (mika#2457). Cleared
    // on both sides so the comparison is about the configuration and not about
    // the clock. The provenance and raw fields go with it: they say through
    // WHICH DOOR a value came, not what is in service, so moving a setting
    // between cascade doors at the same value would otherwise raise
    // `restart_required` and WARN that a restart is needed — a false alarm on
    // the loud arm, about a change that moves nothing a restart would apply.
    // The INFO arm's wording already covers a door-only move: it says no field
    // of the budget/model record moved, which is exactly true.
    let restart_required = {
        let mut a = in_service.clone();
        let mut b = on_disk.clone();
        clear_dating_and_provenance(&mut a);
        clear_dating_and_provenance(&mut b);
        a != b
    };
    let budget_changed = in_service.http_timeout_secs != on_disk.http_timeout_secs
        || in_service.agent_total_timeout_secs != on_disk.agent_total_timeout_secs
        || in_service.llm_max_tokens != on_disk.llm_max_tokens;

    // Built BEFORE the `reported` write, and by a conversion that cannot fail.
    // `DateTime::<Utc>::from(SystemTime)` ends in an `.unwrap()` and panics on
    // an instant outside chrono's range — on the path every turn takes, and
    // after the write, so the drift was marked reported and lost for good.
    let config_mtime = format_config_mtime(current);

    // Re-acquire only to record the report. The released lock opened a window
    // where two concurrent turns for the same agent could both reach here with
    // the same unreported mtime; re-checking `covers` under THIS acquisition is
    // what keeps R9's at-most-once-per-distinct-mtime true. Losing the race is
    // not a failure — the other turn is reporting the very same edit.
    {
        let mut notes = boot_notes()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let note = notes.get_mut(agent_id)?;
        if note.reported.covers(current) {
            return None;
        }
        note.reported = Reported::At(current);
    }
    forget_unreadable_stat(agent_id);

    Some(ConfigChangedSinceBoot {
        agent_id: agent_id.to_string(),
        config_mtime,
        provider_on_disk: on_disk.provider,
        model_on_disk: on_disk.model,
        provider_in_service: in_service.provider,
        model_in_service: in_service.model,
        budget_changed,
        restart_required,
    })
}

/// Clear what dates a record and what says which door it came through, leaving
/// only what is *in service* (mika#2473).
///
/// `resolved_at` dates the resolution; the `*_source` and `*_raw` fields carry
/// the cascade door and the string as written. None of the three is a value the
/// process serves, so a difference in any of them is not a reason to tell an
/// operator that a restart would change what is running.
fn clear_dating_and_provenance(record: &mut ResolvedBudgetRecord) {
    record.resolved_at.clear();
    record.http_source.clear();
    record.total_source.clear();
    record.max_tokens_source.clear();
    record.provider_source.clear();
    record.model_source.clear();
    record.http_raw.clear();
    record.total_raw.clear();
    record.max_tokens_raw.clear();
}

/// What [`ConfigChangedSinceBoot::config_mtime`] carries when the instant the
/// filesystem returned is outside chrono's representable range.
///
/// Named rather than fabricated or silently dropped, the same posture the
/// unreadable-`stat` arm takes one branch away: the reading is announced under
/// its own words. A plausible-looking date would be a false fact carried by the
/// one field an operator compares by eye against `resolved_at`.
pub const CONFIG_MTIME_UNREPRESENTABLE: &str = "(mtime hors de portée — non représentable)";

/// The RFC 3339 form of a file's mtime — **never a panic** (KTD1).
///
/// `From<SystemTime> for DateTime<Utc>` ends in `Utc.timestamp_opt(..).unwrap()`
/// and dies on anything outside roughly ±262 000 years. A filesystem will
/// happily store such an instant (`utimensat` takes a 64-bit seconds field), so
/// this is reachable on a real disk — and it runs on every turn.
fn format_config_mtime(mtime: SystemTime) -> String {
    let (secs, nanos) = match mtime.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => (i64::try_from(after.as_secs()).ok(), after.subsec_nanos()),
        Err(before) => {
            // Before the epoch: chrono counts seconds backwards and nanoseconds
            // forwards, so a sub-second part borrows one second.
            let d = before.duration();
            match (i64::try_from(d.as_secs()), d.subsec_nanos()) {
                (Ok(s), 0) => (s.checked_neg(), 0),
                (Ok(s), n) => (
                    s.checked_neg().and_then(|s| s.checked_sub(1)),
                    1_000_000_000 - n,
                ),
                (Err(_), _) => (None, 0),
            }
        }
    };

    secs.and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, nanos))
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| CONFIG_MTIME_UNREPRESENTABLE.to_string())
}

/// Agents whose `config.toml` was unreadable the last time this process looked.
///
/// # Why the unreadable arm needs state at all
///
/// The readable path is deduplicated by [`BootNote::reported`]: one edit, one
/// line. The unreadable one consulted nothing and wrote nothing, so a
/// `config.toml` deleted or made unreadable under a running spirit produced one
/// WARN **per turn, for ever** — exactly the log churn mika#2131 bounds, and on
/// the arm whose expected regime is zero.
///
/// So the emission is bounded to the two crossings that carry information: the
/// onset (readable → unreadable) and the recovery (unreadable → readable).
/// Kept apart from [`BootNote::reported`] **on purpose**: that field carries the
/// population semantics — an unreadable `stat` leaves the population and must
/// not mark the drift reported — and folding the two would restore the silent
/// blindness [`Reported`] exists to make unwritable.
static UNREADABLE_STAT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn unreadable_stat() -> &'static Mutex<HashSet<String>> {
    UNREADABLE_STAT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Say the unreadable reading once — on its onset, and again when it recovers.
fn report_unreadable_stat(agent_id: &str, path: &Path, err: &std::io::Error) {
    let first = {
        let mut seen = unreadable_stat()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        seen.insert(agent_id.to_string())
    };
    if !first {
        return;
    }
    tracing::warn!(
        event = "agent_config_mtime_unreadable",
        agent_id,
        path = %path.display(),
        error = %err,
        recovered = false,
        "this agent's config.toml could not be stat'ed, so freshness cannot be \
         decided for this turn: the turn leaves the population rather than being \
         counted as unchanged, and nothing is refused. Expected regime is zero of \
         these lines — one means the file was removed, renamed or made unreadable \
         under a running spirit. Said once on the onset and once on the recovery, \
         never once per turn (mika#2473, mika#2131)"
    );
}

/// The mirror crossing: the file is readable again, so the onset above is over.
fn forget_unreadable_stat(agent_id: &str) {
    let was_unreadable = {
        let mut seen = unreadable_stat()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        seen.remove(agent_id)
    };
    if !was_unreadable {
        return;
    }
    tracing::warn!(
        event = "agent_config_mtime_unreadable",
        agent_id,
        recovered = true,
        "this agent's config.toml can be stat'ed again: the turns are back in the \
         freshness population. Both ends of the outage are said, because an onset \
         with no recovery reads as an outage that never ended (mika#2473)"
    );
}

/// Say the finding once — WARN when the values in service are stale, INFO when
/// they are not.
///
/// The split is the whole point of the two levels: an operator greps the WARN
/// and finds the turns running on a file that no longer exists as written. The
/// INFO exists so that "no WARN" is distinguishable from "the detector never
/// ran", the same reason `well_known_model_in_sync` exists next to
/// `well_known_model_drift`.
///
/// Neither arm refuses anything (KTD1). The message names the gesture, because
/// a line that reports a condition without naming its remedy gets read once.
pub fn report_config_change(finding: &ConfigChangedSinceBoot) {
    let ConfigChangedSinceBoot {
        agent_id,
        config_mtime,
        provider_on_disk,
        model_on_disk,
        provider_in_service,
        model_in_service,
        budget_changed,
        restart_required,
    } = finding;

    if *restart_required {
        tracing::warn!(
            event = "agent_config_changed_since_boot",
            agent_id,
            config_mtime,
            provider_on_disk,
            model_on_disk,
            provider_in_service,
            model_in_service,
            budget_changed,
            restart_required,
            "this agent's config.toml has changed since this process read it, and the \
             change moves the budget/model record: the turns running now use the values \
             in service, not the values on disk — restart mika-spirit to apply (mika#2473)"
        );
    } else {
        tracing::info!(
            event = "agent_config_changed_since_boot",
            agent_id,
            config_mtime,
            provider_on_disk,
            model_on_disk,
            provider_in_service,
            model_in_service,
            budget_changed,
            restart_required,
            "this agent's config.toml has changed since this process read it, but no field \
             of the budget/model record moved — another field of the file (a base URL, a \
             log level) may nevertheless require a restart to take effect (mika#2473)"
        );
    }
}

/// Forget every boot note — test-only.
///
/// Gated on `test-utils` as well as `cfg(test)`: the boot notes are process
/// state, and `mika-agent`'s own tests of `init_agent` have to be able to start
/// from an empty map without re-implementing this map's key.
#[cfg(any(test, feature = "test-utils"))]
pub fn reset_notes_for_test() {
    boot_notes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    // The unreadable-stat crossing set is process state of the same kind, and a
    // test that started with a stale entry would see the onset WARN swallowed as
    // a repetition — a green test measuring nothing.
    unreadable_stat()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// The record noted at boot for one agent — test-only.
///
/// Exists for a single assertion, and it is a load-bearing one: D2 must **not**
/// replace the note it reads. Without a way to look at the note afterwards,
/// "the process keeps serving what it loaded" is prose.
#[cfg(any(test, feature = "test-utils"))]
pub fn noted_record_for_test(agent_id: &str) -> Option<ResolvedBudgetRecord> {
    boot_notes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(agent_id)
        .map(|note| note.record.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::budget_provenance::{clean_budget_env, resolve_llm_budget_record};
    use serial_test::serial;
    use std::time::{Duration, SystemTime};

    fn homes() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("global");
        let agent = tmp.path().join("agents").join("mika-arch");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&agent).unwrap();
        (tmp, global, agent)
    }

    /// Rewrite the agent's `config.toml` and **force** a distinct mtime.
    ///
    /// Not a convenience: a rewrite inside the filesystem's timestamp
    /// granularity can land on the very instant the boot note holds, and the
    /// test would then assert "no change" about a file that changed — green,
    /// and measuring nothing. The offset is explicit so the change is a fact of
    /// the fixture rather than a property of the disk.
    fn rewrite_with_distinct_mtime(agent_home: &std::path::Path, body: &str) -> SystemTime {
        let path = agent_home.join("config.toml");
        std::fs::write(&path, body).unwrap();
        let forced = SystemTime::now() + Duration::from_secs(120);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(forced)
            .unwrap();
        forced
    }

    /// Collect the `event` field of every `tracing` line emitted on **this**
    /// thread — the same shape as the capture in
    /// `budget_provenance::tests::mika2362_retry_unreachable_fires_on_the_incident_geometry_only`.
    ///
    /// Every `(level, event)` pair one capture collected.
    type EventSink = std::sync::Arc<std::sync::Mutex<Vec<(tracing::Level, String)>>>;

    /// Name and level, which is all this module asserts about an emission: the
    /// unreadable reading must be announced *under its own name*, and the
    /// freshness line must split WARN from INFO on `restart_required` — the two
    /// facts that distinguish "leaving the population" and "stale values in
    /// service" from a turn where nothing happened.
    fn capture_event_names() -> (tracing::subscriber::DefaultGuard, EventSink) {
        use tracing_subscriber::layer::SubscriberExt;

        struct Visitor<'a>(&'a mut Option<String>);
        impl tracing::field::Visit for Visitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "event" {
                    *self.0 = Some(format!("{value:?}").trim_matches('"').to_string());
                }
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "event" {
                    *self.0 = Some(value.to_string());
                }
            }
        }

        struct Layer(EventSink);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut name = None;
                event.record(&mut Visitor(&mut name));
                if let (Some(name), Ok(mut seen)) = (name, self.0.lock()) {
                    seen.push((*event.metadata().level(), name));
                }
            }
        }

        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(Layer(std::sync::Arc::clone(&sink)));
        (tracing::subscriber::set_default(subscriber), sink)
    }

    const AT_BOOT: &str = "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k2.5\"\nllm_max_tokens = 8192\n";

    /// mika#2473 U2 / R9 — D2 ne lit rien tant que le mtime n'a pas bougé, et
    /// reste inerte pour un agent dont ce process ne détient aucune note.
    #[test]
    #[serial]
    fn mika2473_an_unchanged_config_reports_nothing() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        assert!(
            detect_config_change("mika-arch").is_none(),
            "un fichier qui n'a pas bougé ne coûte qu'un stat et ne rapporte rien"
        );
        assert!(
            detect_config_change("un-agent-jamais-initialise").is_none(),
            "sans note de boot, D2 est inerte — c'est ce qui exempte, pas le type de tour"
        );

        clean_budget_env();
    }

    /// mika#2473 U2 / R8 — un `touch` est rapporté **une fois**, sans exiger de
    /// redémarrage, et le bras `false` ne dit pas « rien n'a changé ».
    ///
    #[test]
    #[serial]
    fn mika2473_a_touch_is_reported_once_without_restart() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        rewrite_with_distinct_mtime(&agent, AT_BOOT);
        let finding = detect_config_change("mika-arch")
            .expect("un mtime distinct est un changement, même à contenu identique");
        assert!(
            !finding.restart_required,
            "aucun champ du record n'a bougé : pas de redémarrage exigé"
        );
        assert!(!finding.budget_changed);
        assert_eq!(finding.agent_id, "mika-arch");
        assert_eq!(finding.model_on_disk, finding.model_in_service);
        assert_eq!(finding.provider_on_disk, finding.provider_in_service);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&finding.config_mtime).is_ok(),
            "config_mtime doit être un instant RFC 3339 lisible : {}",
            finding.config_mtime
        );

        assert!(
            detect_config_change("mika-arch").is_none(),
            "une re-résolution par mtime distinct, pas une par tour (R9)"
        );

        clean_budget_env();
    }

    /// mika#2473 U2 / R8 — une édition du modèle exige un redémarrage et nomme
    /// les deux côtés ; une édition du budget lève `budget_changed`.
    ///
    /// Porte aussi le contrôle négatif du plan — le record **noté** est
    /// inchangé après `detect_config_change` — et il est ici plutôt que sur le
    /// `touch` **parce qu'il y est vacant** : sur un `touch`, le record relu
    /// égale le record du boot dans tous ses champs, `resolved_at` compris
    /// (estampillé à la seconde, deux résolutions coup sur coup rendent la même
    /// chaîne). Mesuré : muter `detect_config_change` pour qu'il écrase la note
    /// laissait le test du `touch` **vert**. Une édition du modèle est le seul
    /// terrain où l'assertion mord.
    #[test]
    #[serial]
    fn mika2473_a_model_edit_requires_a_restart_and_names_both_sides() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        rewrite_with_distinct_mtime(
            &agent,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k3\"\nllm_max_tokens = 8192\n",
        );
        let finding = detect_config_change("mika-arch").expect("le modèle sur disque a changé");
        assert!(
            finding.restart_required,
            "le process sert encore l'ancien modèle : il faut un redémarrage"
        );
        assert_eq!(finding.model_on_disk, "moonshotai/kimi-k3");
        assert_eq!(
            finding.model_in_service, "moonshotai/kimi-k2.5",
            "le modèle EN SERVICE est celui du boot, pas celui du disque"
        );
        assert!(
            !finding.budget_changed,
            "contrôle négatif : une édition du modèle seul ne bouge pas le budget"
        );

        rewrite_with_distinct_mtime(
            &agent,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k3\"\nllm_max_tokens = 32768\n",
        );
        let finding = detect_config_change("mika-arch").expect("le budget sur disque a changé");
        assert!(finding.budget_changed, "llm_max_tokens a bougé");
        assert!(finding.restart_required);

        assert_eq!(
            noted_record_for_test("mika-arch").as_ref(),
            Some(&record),
            "contrôle négatif : D2 ne remplace pas le record du boot — le process \
             continue de servir ce qu'il a chargé, et un détecteur qui échangerait la \
             note lui ferait rapporter une valeur qu'il n'a jamais lue"
        );

        clean_budget_env();
    }

    /// mika#2473 U2 / R8 — la ligne de fraîcheur sépare WARN et INFO sur
    /// `restart_required`, et **le bras INFO ne dit pas « rien n'a changé »**.
    ///
    /// Les deux bras dans le même appel. Sans le bras INFO, rien n'atteste que
    /// le détecteur se tait quand il doit se taire ; sans le bras WARN, rien
    /// n'atteste qu'il parle fort quand le process sert des valeurs périmées.
    /// Et l'assertion sur le texte est celle que R8 pose nommément : un INFO
    /// disant « aucun champ effectif n'a bougé » laisserait un opérateur sur
    /// l'ancien endpoint en lui disant que tout va bien.
    #[test]
    #[serial]
    fn mika2473_the_freshness_line_splits_warn_from_info() {
        let sample = |restart_required: bool| ConfigChangedSinceBoot {
            agent_id: "mika-arch".to_string(),
            config_mtime: "2026-09-22T10:00:00Z".to_string(),
            provider_on_disk: "openrouter".to_string(),
            model_on_disk: "moonshotai/kimi-k3".to_string(),
            provider_in_service: "openrouter".to_string(),
            model_in_service: "moonshotai/kimi-k2.5".to_string(),
            budget_changed: false,
            restart_required,
        };

        let (_guard, seen) = capture_event_names();

        report_config_change(&sample(false));
        report_config_change(&sample(true));

        let lines = seen.lock().unwrap().clone();
        assert_eq!(lines.len(), 2, "une ligne par constat, pas zéro");
        assert_eq!(
            lines[0],
            (
                tracing::Level::INFO,
                "agent_config_changed_since_boot".to_string()
            ),
            "aucun champ du record n'a bougé : INFO"
        );
        assert_eq!(
            lines[1],
            (
                tracing::Level::WARN,
                "agent_config_changed_since_boot".to_string()
            ),
            "le process sert des valeurs périmées : WARN"
        );
    }

    /// mika#2473 U2 — **un `stat` illisible sort le tour de la population**, il
    /// n'est jamais un terme satisfait.
    ///
    /// `mtime` et `reported` démarrent tous deux « rien » ; s'ils partageaient
    /// une représentation, une lecture manquée égalerait « déjà rapporté » et un
    /// `config.toml` supprimé après le boot ne serait **jamais** signalé — la
    /// panne silencieuse exacte que cette unité existe pour fermer. Les trois
    /// termes, dans le même appel : la ligne nommée, la note intacte, et le
    /// signalement qui fonctionne encore une fois le fichier revenu.
    #[test]
    #[serial]
    fn mika2473_an_unreadable_stat_leaves_the_population_and_never_reports() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        let (_guard, seen) = capture_event_names();
        std::fs::remove_file(agent.join("config.toml")).unwrap();
        assert!(
            detect_config_change("mika-arch").is_none(),
            "un stat illisible ne rapporte pas un changement qu'il n'a pas lu"
        );
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(level, name)| *level == tracing::Level::WARN
                    && name == "agent_config_mtime_unreadable"),
            "mais il le DIT, sous son propre nom : {:?}",
            seen.lock().unwrap()
        );

        rewrite_with_distinct_mtime(&agent, AT_BOOT);
        assert!(
            detect_config_change("mika-arch").is_some(),
            "contrôle négatif : la lecture manquée n'a pas empoisonné la note — \
             `reported` est resté intact, donc le fichier revenu est rapporté"
        );

        clean_budget_env();
    }

    /// Force the agent's `config.toml` to an mtime chrono cannot represent, and
    /// give back the instant the filesystem actually kept.
    ///
    /// `1 << 50` seconds after the epoch is roughly the year 35 000 000 — well
    /// past chrono's ceiling (year 262 143) and well inside what a Linux
    /// filesystem accepts, which is the whole point: the pair is reachable on a
    /// real disk, so the panic it used to produce was reachable on a real turn.
    fn force_unrepresentable_mtime(agent_home: &std::path::Path) -> SystemTime {
        let path = agent_home.join("config.toml");
        std::fs::write(&path, AT_BOOT).unwrap();
        let forced = SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 50);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(forced)
            .unwrap();
        std::fs::metadata(&path).unwrap().modified().unwrap()
    }

    /// mika#2473 KTD1 — **un mtime hors de portée ne fait PAS paniquer le tour.**
    ///
    /// `DateTime::<Utc>::from(SystemTime)` se termine par un `.unwrap()` : sur un
    /// instant hors de la plage représentable par chrono il panique. Ce chemin
    /// tourne à **chaque tour**, et la panique cassait le contrat KTD1 — la
    /// garde rapporte, elle ne refuse jamais.
    ///
    /// Le second terme est celui qui coûte le plus cher : avant le correctif,
    /// `note.reported = Reported::At(current)` était écrit **avant** la
    /// conversion, donc la panique laissait la dérive marquée comme rapportée et
    /// la perdait définitivement. Ici, le rapport doit survivre à la lecture
    /// impossible — la date est nommée, le reste du constat est intact.
    #[test]
    #[serial]
    fn mika2473_an_unrepresentable_mtime_is_named_and_never_panics() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        let forced = force_unrepresentable_mtime(&agent);
        assert!(
            chrono::DateTime::<chrono::Utc>::from_timestamp(
                forced
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
                0,
            )
            .is_none(),
            "la fixture doit être hors de portée de chrono, sinon elle ne mesure rien"
        );

        let finding = detect_config_change("mika-arch")
            .expect("un mtime illisible par chrono reste un changement : le tour est rapporté");
        assert_eq!(
            finding.config_mtime, CONFIG_MTIME_UNREPRESENTABLE,
            "la lecture impossible est NOMMÉE, jamais inventée ni tue"
        );
        assert_eq!(finding.agent_id, "mika-arch");
        assert_eq!(finding.model_on_disk, finding.model_in_service);

        clean_budget_env();
    }

    /// mika#2473 R9 — **deux tours concurrents sur le même agent : un seul
    /// rapporte.**
    ///
    /// Le verrou global n'est plus tenu pendant le `stat` ni pendant la
    /// re-résolution (P1 #2), ce qui ouvre une course : deux tours peuvent lire
    /// le même mtime non rapporté, chacun croire l'avoir gagné, et l'édition
    /// serait rapportée deux fois. La seconde acquisition re-teste
    /// `reported.covers(current)` et c'est ce test-là qui l'atteste.
    ///
    /// Mesuré rouge en retirant cette re-vérification : deux `Some`.
    #[test]
    #[serial]
    fn mika2473_two_concurrent_turns_report_once() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);
        rewrite_with_distinct_mtime(
            &agent,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k3\"\nllm_max_tokens = 8192\n",
        );

        let gate = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let gate = std::sync::Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    detect_config_change("mika-arch").is_some()
                })
            })
            .collect();

        let reported = handles
            .into_iter()
            .filter(|_| true)
            .map(|h| h.join().unwrap())
            .filter(|reported| *reported)
            .count();
        assert_eq!(
            reported, 1,
            "une re-résolution par mtime distinct (R9), même quand le verrou est \
             relâché pendant le stat et la re-résolution"
        );

        clean_budget_env();
    }

    /// mika#2473 / mika#2131 — **un `stat` illisible se dit une fois, pas une
    /// fois par tour**, et sa guérison se dit aussi.
    ///
    /// Le chemin lisible est dédupliqué par `reported` ; l'illisible ne
    /// consultait ni n'écrivait aucun état, donc un `config.toml` supprimé sous
    /// un spirit vivant produisait un WARN **par tour, indéfiniment** — le churn
    /// que la doctrine de la maison borne, et sur le bras dont le régime attendu
    /// est zéro. Les deux traversées portent de l'information : l'apparition et
    /// le retour. Les répétitions au milieu n'en portent aucune.
    ///
    /// `reported` n'est **pas** touché : la sémantique de population (« un stat
    /// illisible sort le tour de la population ») est inchangée, et le test
    /// voisin `…_an_unreadable_stat_leaves_the_population_and_never_reports`
    /// l'atteste encore.
    #[test]
    #[serial]
    fn mika2473_a_repeated_unreadable_stat_is_said_once_and_its_recovery_too() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        note_config_at_boot("mika-arch", &global, &agent, &record);

        let (_guard, seen) = capture_event_names();
        std::fs::remove_file(agent.join("config.toml")).unwrap();
        for _ in 0..5 {
            assert!(detect_config_change("mika-arch").is_none());
        }
        assert_eq!(
            unreadable_lines(&seen),
            1,
            "cinq tours sur un fichier illisible : UNE ligne, pas cinq — \
             sinon le régime attendu zéro devient illisible : {:?}",
            seen.lock().unwrap()
        );

        rewrite_with_distinct_mtime(&agent, AT_BOOT);
        assert!(
            detect_config_change("mika-arch").is_some(),
            "contrôle négatif : `reported` est resté intact, le fichier revenu \
             est rapporté normalement"
        );
        assert_eq!(
            unreadable_lines(&seen),
            2,
            "et le retour est dit une fois : une apparition sans guérison se lit \
             comme une panne qui n'a jamais cessé : {:?}",
            seen.lock().unwrap()
        );

        for _ in 0..3 {
            let _ = detect_config_change("mika-arch");
        }
        assert_eq!(
            unreadable_lines(&seen),
            2,
            "et les tours sains qui suivent ne redisent rien"
        );

        clean_budget_env();
    }

    /// Combien de lignes `agent_config_mtime_unreadable` la capture a vues.
    fn unreadable_lines(seen: &EventSink) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(_, name)| name == "agent_config_mtime_unreadable")
            .count()
    }

    /// mika#2473 — **déplacer un réglage d'une porte à l'autre à valeur égale
    /// n'exige aucun redémarrage.**
    ///
    /// `ResolvedBudgetRecord` porte ses champs `*_source` et `*_raw` : la porte
    /// de la cascade et la chaîne telle qu'écrite. Aucun des deux n'est une
    /// valeur que le process sert. Comparer les records sans les effacer faisait
    /// donc lever `restart_required` — et WARN qu'un redémarrage est requis —
    /// sur un `llm_max_tokens` passé du `config.toml` de l'agent à celui du
    /// global **à la même valeur** : une fausse alerte sur le bras bruyant, à
    /// propos d'un changement qu'un redémarrage ne changerait pas.
    ///
    /// Le bras INFO dit exactement le vrai dans ce cas : aucun champ du record
    /// budget/modèle n'a bougé, un autre champ du fichier peut néanmoins exiger
    /// un redémarrage.
    #[test]
    #[serial]
    fn mika2473_a_door_only_move_at_the_same_value_needs_no_restart() {
        clean_budget_env();
        reset_notes_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(agent.join("config.toml"), AT_BOOT).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        assert_eq!(
            record.max_tokens_source, "agent_config",
            "précondition : la valeur entre par la porte de l'agent"
        );
        note_config_at_boot("mika-arch", &global, &agent, &record);

        // Même valeur, autre porte : retirée du config.toml de l'agent, posée
        // dans le global.
        std::fs::write(global.join("config.toml"), "llm_max_tokens = 8192\n").unwrap();
        rewrite_with_distinct_mtime(
            &agent,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k2.5\"\n",
        );

        let on_disk = resolve_llm_budget_record("mika-arch", &global, &agent);
        assert_eq!(
            on_disk.max_tokens_source, "global_config",
            "précondition : la porte a bien changé"
        );
        assert_eq!(
            on_disk.llm_max_tokens, record.llm_max_tokens,
            "précondition : la valeur EN SERVICE, elle, n'a pas bougé"
        );

        let finding = detect_config_change("mika-arch")
            .expect("le mtime a bougé : le déplacement est bien rapporté");
        assert!(
            !finding.restart_required,
            "aucune valeur en service ne diffère : le bras bruyant doit se taire"
        );
        assert!(
            !finding.budget_changed,
            "et le budget n'a pas bougé non plus"
        );

        clean_budget_env();
    }
}

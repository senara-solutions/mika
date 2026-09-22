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
//! (KTD1), and it never hot-reloads: a detector that swapped the note for the
//! disk's record would make the process report a value it has never loaded,
//! which is mika#2304's defect one field over.
//!
//! # Why the mtime and not the content (KTD4)
//!
//! Hashing the file would cost a read per turn, on every turn, to answer a
//! question whose answer is "no" virtually always. The mtime is a metadata
//! read; the content is only parsed once the metadata says something moved.
//! The cost of the false positive — a `touch` with no edit — is one INFO line
//! saying exactly that, which is cheaper than the read it avoids.
//!
//! # `restart_required = false` is not "nothing changed"
//!
//! [`ResolvedBudgetRecord`] carries the budget and the model. It does **not**
//! carry `openrouter_base_url`, `zai_base_url` or `log_level` — and an edit to
//! those is a real gesture on these agents, one that changes the endpoint being
//! served. Such an edit lands in the `false` arm. The message therefore says
//! *"no field of the budget/model record moved — another field of the file may
//! still require a restart"*, never *"nothing effective moved"*: the second
//! wording would leave an operator on the old endpoint while telling them all
//! is well.

use std::collections::HashMap;
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
///    `agent_config_mtime_unreadable` (expected regime: zero), and
///    [`BootNote::reported`] is left **intact** — so the moment the file is
///    readable again, the change is reported normally. See [`Reported`] for why
///    the types make this the only possible reading.
///
/// Otherwise the cascade is re-resolved **from disk** through
/// [`resolve_llm_budget_record`] — the single constructor of the record
/// (mika#2457), called and not duplicated — and compared to the boot record
/// with `resolved_at` cleared on both sides: that field dates the resolution,
/// so leaving it in would make every comparison differ and every edit look like
/// a restart-requiring one.
///
/// The boot note's record is **not** replaced. The process keeps serving what
/// it loaded; this function says so, it does not change it.
pub fn detect_config_change(agent_id: &str) -> Option<ConfigChangedSinceBoot> {
    let mut notes = boot_notes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let note = notes.get(agent_id)?;

    let path = note.agent_home.join(AGENT_CONFIG_FILE);
    let current = match std::fs::metadata(&path).and_then(|meta| meta.modified()) {
        Ok(mtime) => mtime,
        Err(err) => {
            drop(notes);
            tracing::warn!(
                event = "agent_config_mtime_unreadable",
                agent_id,
                path = %path.display(),
                error = %err,
                "this agent's config.toml could not be stat'ed, so freshness cannot be \
                 decided for this turn: the turn leaves the population rather than being \
                 counted as unchanged, and nothing is refused. Expected regime is zero of \
                 these lines — one means the file was removed, renamed or made unreadable \
                 under a running spirit (mika#2473)"
            );
            return None;
        }
    };

    if note.mtime == Some(current) || note.reported.covers(current) {
        return None;
    }

    let global_home = note.global_home.clone();
    let agent_home = note.agent_home.clone();
    let in_service = note.record.clone();

    let on_disk = resolve_llm_budget_record(agent_id, &global_home, &agent_home);

    // `resolved_at` dates the record, it is not part of it (mika#2457). Cleared
    // on both sides so the comparison is about the configuration and not about
    // the clock.
    let restart_required = {
        let mut a = in_service.clone();
        let mut b = on_disk.clone();
        a.resolved_at.clear();
        b.resolved_at.clear();
        a != b
    };
    let budget_changed = in_service.http_timeout_secs != on_disk.http_timeout_secs
        || in_service.agent_total_timeout_secs != on_disk.agent_total_timeout_secs
        || in_service.llm_max_tokens != on_disk.llm_max_tokens;

    if let Some(note) = notes.get_mut(agent_id) {
        note.reported = Reported::At(current);
    }

    Some(ConfigChangedSinceBoot {
        agent_id: agent_id.to_string(),
        config_mtime: chrono::DateTime::<chrono::Utc>::from(current)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        provider_on_disk: on_disk.provider,
        model_on_disk: on_disk.model,
        provider_in_service: in_service.provider,
        model_in_service: in_service.model,
        budget_changed,
        restart_required,
    })
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
}

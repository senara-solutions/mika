//! # mika-manager Phase 1 — milestone-scope operational coordinator
//!
//! **LECTURE SEULE.** This module reads milestone state (via `gh` CLI), assesses it
//! (rules-driven recommendations), and reports structured Markdown to Prime→sami→Vincent.
//! It has **zero write authority**: no dispatch, no ticket label mutation, no PR merge,
//! no scope approval. It has exactly **two** outbound side effects, both towards the
//! control-monitor and neither of them a write on the forge: a report `POST` to a
//! well-known delivery endpoint (Prime→sami→Vincent per D8 subsystem-2 pattern), or an
//! offline sink write when the URL is unset — and, since mika#1990, a **liveness beat**,
//! one light `POST` per successful poll tick on `MIKA_MANAGER_LIVENESS_URL` (see
//! [`liveness`]). The beat carries no report content: the entity name, the beat motive,
//! a timestamp and the milestone reference. Both are declared in
//! `docs/egress/egress-manifest.toml` (`control-monitor-delivery`,
//! `control-monitor-liveness`).
//!
//! **Depuis mika#2698 vol 2 il existe un troisième effet, et il est le premier à
//! écrire sur la forge** : un commentaire de verdict sur le ticket d'un vol
//! terminé, derrière `MIKA_MANAGER_EVALUATE_WRITE`, **OFF par défaut**. Un seul
//! geste, `gh issue comment`, dans un seul fichier, [`verdict_poster`]. La
//! phrase « zéro autorité d'écriture » ci-dessus décrit donc l'état par défaut
//! — qui est l'état déployé — et non une impossibilité structurelle : la
//! différence est l'objet du ticket, et son seuil d'ouverture est un geste
//! opérateur (10 vols concordants, ou 7 jours). Toute autre écriture reste
//! refusée structurellement par `no_dispatch_test.rs`.
//!
//! The chain of authority is Prime → Manager → Executors. `mika-manager` is a distinct
//! entity from `mika-prime`; see the ratified brief at
//! `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`
//! for the founding design (5 verdicts Prime + Vincent GO 2026-08-21).
//!
//! ## Cadence contract
//!
//! Hybrid: event-driven (state change detection) + 6h plancher heartbeat
//! ("l'absence d'event EST l'event"). Both configurable via env; the offline sink is
//! always used when delivery URLs are unset so nothing is lost during bring-up.
//!
//! ## Wrapper-only INV-2
//!
//! Every `gh` call mirrors the shape of `crate::auto_pull::gh_list_open_issues`
//! (`tokio::process::Command`, env `GH_TOKEN`, `stdin::null`, `kill_on_drop(true)`).
//! No new GitHub API client dep. No `gh api PATCH/POST/DELETE`. See
//! `no_dispatch_test.rs` for the structural enforcement.
//!
//! ## Phase 2 promotion (NOT wired here)
//!
//! Phase 2 (dispatch authority — recommend + auto-execute) is gated behind the three
//! portes documented in the brief § 3: forge-gate loop-résistance (Porte 1), contention
//! exec (Porte 2), and `INTERNAL_TOKEN` alignment (Porte 3). None of that surface is
//! wired in this module.
//!
//! ### Phase 2 (a) — le bassin `ready` est une LECTURE, et **un** geste armé (mika#2699)
//!
//! [`pool`] juge la profondeur du bassin `ready` et **dit** quels tickets
//! devraient y entrer, et lesquels sont refusés et pourquoi. C'est une lecture :
//! le verdict entre dans le rapport ([`Assessment::pool`]) et nulle part
//! ailleurs. [`pool_writer`] est ce qui peut en **faire** quelque chose, et
//! seulement quand `MIKA_MANAGER_POOL_WRITE` est armé — OFF par défaut, et
//! l'ouverture est un geste opérateur, pas un défaut de configuration. Le
//! charter **LECTURE SEULE** ci-dessus se lit donc désormais avec deux
//! exceptions nommées, l'une par ticket, et pas une de plus.
//!
//! Trois conséquences qu'un éditeur futur doit connaître :
//!
//! - **Les quatrième et cinquième lectures `gh` sont dépôt-larges**, pas
//!   limitées au milestone cible. Pour le bassin : le webhook `ready` dispatche
//!   à l'échelle du dépôt, donc un bassin compté sur les seules `sub_issues`
//!   serait sous-compté. Pour les candidats du dehors
//!   ([`CandidateOrigin::Outside`]) : le 2e critère de tri du ticket — « dans le
//!   milestone cible, **avant le reste** » — n'a de sens que s'il existe un
//!   « reste » à comparer. Ce sont les deux seules lectures du module qui
//!   sortent du périmètre du milestone ; elles restent des `issue list`, donc
//!   elles ne touchent pas `no_dispatch_test.rs`.
//! - **La priorité domine la provenance** dans le tri des admissions. Un p0 hors
//!   milestone passe devant un p1 du milestone. Le pourquoi de cette lecture de
//!   la phrase du ticket est dans le docstring de [`pool`], et c'est une
//!   décision, pas un détail d'implémentation.
//! - **Le geste d'écriture est le vol 4** ([`pool_writer`]), derrière
//!   `MIKA_MANAGER_POOL_WRITE` (OFF par défaut). Il est arrivé comme **exception
//!   nommée** au scan de `no_dispatch_test.rs` — un seul jeton, `gh issue edit`,
//!   dans le même commit que ce docstring — jamais comme un assouplissement du
//!   scan. Les deux mutations du ticket sont **un** appel : poser `ready` et
//!   rattacher au milestone cible ensemble, pour qu'un échec ne laisse jamais la
//!   moitié d'une décision.
//!
//! Ce que la Phase 2 (a) ne porte pas encore, et qui est suivi sur
//! `senara-solutions/mika#2699` : la **seconde moitié de l'AC3** — l'idempotence
//! inter-cycle épinglée par un test de verdict rejoué. Elle est *structurelle*
//! aujourd'hui (un label posé est lu par le filtre de candidats de [`pool`], donc
//! le cycle suivant ne propose plus le ticket), et c'est précisément ce qu'un
//! test doit mesurer plutôt qu'un champ de checkpoint doive garantir. Le rapport
//! ne nomme pas encore ce qui a été réellement posé.
//!
//! **Porte 1 discharge condition (mika#1947).** Dispatch authority is DECISION-CORE by
//! construction, so the forge-gate perimeter classifier must fail-closed to
//! `DecisionCore` on every path under `crates/mika-agent/src/milestone_manager/**`, at
//! every merge-authority callsite. Today that holds only via the classifier's
//! fail-closed default — no rule in `perimeter/rules.rs` names this module. The tests
//! below make it hold structurally, so a future diff that adds the manager surface to a
//! MECHANICAL table fails a test instead of quietly opening an auto-merge path into the
//! manager's own code. In-tree proof:
//!
//! - `crates/mika-agent/src/perimeter/tests.rs` —
//!   `milestone_manager_files_are_decision_core`,
//!   `milestone_manager_solo_pr_is_decision_core`,
//!   `milestone_manager_file_taints_pr_batch`,
//!   `milestone_manager_prefix_not_in_mechanical_tables`,
//!   `milestone_manager_absent_from_all_mechanical_tables`,
//!   `milestone_manager_has_no_nested_tests_directory` (AC1 + AC2).
//! - `crates/mika-agent/tests/eval/test_verdict_handler.rs` —
//!   `verdict_pass_milestone_manager_pr_holds_for_operator` (AC3).
//! - `crates/mika-agent/tests/eval/test_ci_success_handler.rs` —
//!   `ci_success_milestone_manager_pr_holds_for_operator` (AC4).
//! - `crates/mika-agent/tests/eval/manager_loop_resistance.rs` —
//!   `cascade_never_dispatches_into_milestone_manager` (AC5, gated `#[ignore]` +
//!   `MIKA_MANAGER_LOOP_RESISTANCE_TEST=1`).
//!
//! `milestone_manager_has_no_nested_tests_directory` is the one to read before adding
//! files here: `MECHANICAL_CONTAINS` grants MECHANICAL to any path containing
//! `/tests/`, so a `src/milestone_manager/tests/` directory would be auto-mergeable
//! inside the very module Porte 1 gates. The manager's own test code belongs beside the
//! module files, as `no_dispatch_test.rs` does.
//!
//! ### Porte 3 — `INTERNAL_TOKEN` alignment (mika#1949)
//!
//! **Discharge condition:** every cross-boundary authentication failure on the
//! manager write path is *distinguishable* — the operator can tell which of the
//! four tokens failed, at which boundary, and why, from the report and from
//! `audit_events`, without reading a log. It is **not** that the four tokens
//! become one; unification is deferred to a Phase 3 refactor.
//!
//! What Porte 3 does **not** ask for: that a write proceed past a failed
//! credential. Fail-closed is the property being protected. What changes is
//! that the closure is now named instead of silent.
//!
//! **Status: NOT discharged.** The `gateway_to_spirit` boundary
//! (`crate::server::auth`) is not yet instrumented — that unit edits
//! authentication-decision code and is blocked pending operator review. The
//! other three boundaries are. See `senara-solutions/mika#1949`.
//!
//! **Post-deploy verification** — after a cadence cycle:
//!
//! ```sql
//! SELECT target_key, after_value, count(*)
//! FROM audit_events
//! WHERE tool_name = 'auth_boundary'
//! GROUP BY 1, 2;
//! ```
//!
//! `target_key` is `<from>_to_<to>`; `after_value` is the failure kind
//! (`missing`/`empty`/`invalid`/`rejected`/`unreachable`). Row shape documented
//! in `crates/mika-agent/CLAUDE.md § Audit Log`; operator rotation procedure in
//! `mika-platform/docs/operator/token-rotation-procedure.md`.
//!
//! **The role question is settled and does not need re-deriving.** The manager
//! authenticates to cm as `Role::CommandPlane`. No `Role::Manager` variant was
//! created: `/messages/dispatch` was already permitted to the command plane, and
//! `/permission-events` is deliberately refused to it (an audit corpus a caller
//! could otherwise forge). The manager's audit trail is mika's own
//! `audit_events`, never cm's. Pinned by `manager_write_path_needs_no_new_role`
//! in `control-monitor/backend/crates/cm-api/src/scope.rs`.
//!
//! ## Phase 2 (b) vol 1 — le manager évalue chaque vol (mika#2698)
//!
//! **Ce que le vol 1 a ajouté :** [`evaluator`], qui voit les vols
//! d'implémentation terminés depuis le cycle précédent sur les tickets du
//! milestone cible et en rend un verdict structuré (issue, tours, cause de mort,
//! recommandation). La détection passe par le trait [`evaluator::FlightSource`],
//! dont l'implémentation base de données vit **hors** du module
//! (`crate::flight_source::DbFlightSource`) — même idiome que
//! `AuthBoundaryLedger` / `DbAuthBoundaryLedger`, et pour la même raison : aucun
//! SQL ne passe ainsi sous le grep de `no_dispatch_test.rs`. Rien de nouveau sur
//! le réseau : tout vient de `tasks` et de `SubIssue::pr_state`, déjà hydraté.
//!
//! Le vol 1 n'a ajouté **aucune** autorité d'écriture, pas même désarmée :
//! `MIKA_MANAGER_EVALUATE_WRITE` n'y était pas déclarée, parce qu'un
//! interrupteur dont la branche ON ne fait rien est un mensonge de surface. Le
//! verdict avait exactement une destination,
//! [`types::Assessment::flight_verdicts`], que le `Reporter` rend dans sa
//! section « Vols terminés depuis le cycle précédent ». C'est le *mode ombre*
//! du ticket, et il reste la destination par défaut.
//!
//! ## Phase 2 (b) vol 2 — l'interrupteur et le geste unique (mika#2698)
//!
//! **Ce que le vol 2 a ajouté :** [`verdict_poster`], et rien d'autre sur la
//! forge. `MIKA_MANAGER_EVALUATE_WRITE` y est déclarée et **OFF par défaut** ;
//! [`verdict_poster::write_is_armed`] en est le lecteur unique du dépôt. La
//! branche ON autorise un seul geste, `gh issue comment` — première et seule
//! entrée de l'allowlist de `no_dispatch_test.rs`, au fichier et au jeton près.
//! L'idempotence (AC2) est portée par
//! [`cadence::MilestoneCheckpoint::posted_flight_task_ids`] et non par une
//! relecture des commentaires : le ticket autorise un geste, et c'est une
//! écriture.
//!
//! L'exécutant est le [`reader::GhRunner`] déjà injectable du module
//! (mika#1975), passé par `CycleContext::with_gh_runner` — donc aucune
//! signature de `spawn.rs` ni de `server/mod.rs` n'a bougé.
//!
//! Deux data du verdict restent **présentes et `None`** — non instrumentées,
//! jamais `0` (motif mika#2331) : le plafond de tours, dont le lecteur unique
//! est en shell (`_pilot_max_turns`), et le tour du dernier commit, qui demande
//! le log de session pilote. Les deux sont du **vol 3**, nommé et chiffré dans
//! `docs/plans/2026-10-15-001-feat-2698-manager-evalue-chaque-vol-vol-2-plan.md`
//! § *Découpage du reliquat* ; le ticket reste ouvert jusque-là.
//!
//! ## Phase 2 (c) vol 1 — le manager nomme la relève (mika#2700)
//!
//! **Ce que le vol 1 ajoute :** [`relief`], qui nomme à chaque cycle les quatre
//! prochains tickets à grommer derrière le bassin `ready`, dans l'ordre de tri
//! du bassin, chacun avec son motif. C'est une **lecture de plus, pas une
//! autorité de plus** : la relève dérive du verdict du bassin
//! ([`pool::PoolVerdict::refusals`]), n'ajoute aucune lecture `gh`, et a
//! exactement une destination — [`types::Assessment::relief`], que le
//! [`Reporter`] rend dans sa section « Relève à grommer (mode ombre — aucun
//! groom déclenché) ». C'est le *mode ombre* du point 3 du ticket, et il reste
//! la destination par défaut.
//!
//! Trois conséquences qu'un éditeur futur doit connaître :
//!
//! - **L'idempotence (AC2) est structurelle, pas filtrée.** Un ticket déjà
//!   groomé n'est pas un refus du bassin ; un ticket en cours de groom porte un
//!   siège `dispatch:` qui l'a déjà sorti du compte. [`pool::assess_pool`] les a
//!   écartés avant que la relève existe, donc dériver la relève de son verdict
//!   est ce qui rend un second filtre **impossible** — et un second filtre
//!   serait un second juge du même fait.
//! - **L'ordre du bassin a un site unique**, [`pool::pool_sort_key`], extrait
//!   par ce vol. La relève présente ses tickets « dans l'ordre du bassin » : une
//!   deuxième écriture du triplet `(priorité, provenance, numéro)` ferait se
//!   contredire deux listes du même rapport sans qu'aucune assertion de
//!   comportement rougisse (doctrine du lecteur unique, mika#2201).
//! - **Le vol 1 n'ouvre aucun chemin d'écriture, et n'en désarme aucun.**
//!   `MIKA_MANAGER_GROOM_WRITE` n'y est **pas déclarée** — un interrupteur dont
//!   la branche ON ne fait rien est un mensonge de surface, même raison qu'au
//!   vol 1 de mika#2698. Le geste de groom est le **vol 2** : derrière cet
//!   interrupteur, OFF par défaut, par la voie moteur existante (le groom passe
//!   par mika-dev, jamais par un spawn), et il arrivera comme **exception
//!   nommée** au scan de `no_dispatch_test.rs`, amendée dans le même commit que
//!   ce paragraphe — jamais comme un assouplissement du scan. Les deux moitiés
//!   de `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` sont ce qui rougit ce
//!   jour-là. Suivi sur `senara-solutions/mika#2700`.

pub mod assessor;
pub mod cadence;
pub mod evaluator;
pub mod liveness;
pub mod plan_size;
pub mod pool;
pub mod pool_writer;
pub mod reader;
pub mod relief;
pub mod reporter;
pub mod sink_dir;
pub mod spawn;
pub mod types;
pub mod verdict_poster;

#[cfg(test)]
mod no_dispatch_test;

pub use assessor::{Assessor, AssessorConfig};
pub use cadence::{
    CycleContext, CycleKind, DeliveryBody, DeliveryError, DeliveryFailureKind, HttpReportDeliverer,
    ManagerConfig, MilestoneCheckpoint, ROUTE_HTTP, ROUTE_OFFLINE_SINK,
    ROUTE_OFFLINE_SINK_FALLBACK, ReportDeliverer, run_manager_cycle, run_manager_cycle_in,
    run_manager_cycle_with, run_manager_cycle_with_auth, state_digest,
};
pub use evaluator::{
    DeathCause, FinishedFlight, FlightIssue, FlightRecommendation, FlightSource, FlightVerdict,
    PrStateKnowledge, evaluate_flight, evaluate_flights, milestone_issue_urls,
};
pub use liveness::{
    ENV_LIVENESS_URL, HttpLivenessSink, LIVENESS_ENTITY, LIVENESS_TIMEOUT, LivenessBody,
    LivenessEmitter, LivenessFailure, LivenessFailureClass, LivenessSink, REASON_DELIVERY_PREFIX,
    REASON_POLL_PREFIX, TransitionLine, liveness_reason,
};
pub use pool::{
    Admission, AdmissionRefusal, CandidateOrigin, PLAN_SIZE_READ_MAX, POOL_MAX, POOL_MIN,
    PlanSizeReading, PoolBand, PoolCandidate, PoolDepth, PoolVerdict, ReadyTicket, RefusalReason,
    UnreadCause, assess_pool, numbers_needing_size, pool_sort_key,
};
pub use pool_writer::{ENV_POOL_WRITE, admit_pool_tickets, pool_write_is_armed};
pub use reader::{
    GhRunner, OUTSIDE_READ_LIMIT, ProcessGhRunner, Reader, compose_from_gh_outputs,
    compose_outside_candidates, compose_ready_pool,
};
pub use relief::{RELIEF_SIZE, ReliefTicket, ReliefVerdict, name_relief};
pub use reporter::{AuthBoundaryNote, AuthBoundaryTracker, Reporter};
pub use sink_dir::{
    ENV_OFFLINE_SINK_DIR, SinkDirSource, SinkEntry, SinkListing, list_sink_reports,
    list_sink_reports_in, resolve_offline_sink_dir,
};
pub use spawn::{
    AuthAlarmBody, AuthAlarmSink, ENV_DELIVERY_URL, ENV_ESCALATION_URL, HttpAuthAlarmSink,
    SettingsTokenResolver, TokenResolver, manager_config_from_env, reject_heartbeat_endpoint,
    spawn_manager_cycle_task,
};
pub use types::{
    Alert, AlertKind, Assessment, CiState, CycleOutcome, IssueState, MilestoneRef, MilestoneState,
    ProgressCounts, RecentActivity, Recommendation, Severity, SubIssue,
};
pub use verdict_poster::{
    ENV_EVALUATE_WRITE, MARKER_PREFIX, post_verdicts, render_verdict_comment, verdict_marker,
    write_is_armed,
};

#[cfg(test)]
mod tests {
    use std::fs;

    /// Les sources de **production** du module, fichier par fichier,
    /// commentaires retirés.
    ///
    /// Plat, non récursif, et tronqué au premier `#[cfg(test)]` : les trois
    /// reposent sur la même prémisse, que la Porte 1 tient déjà — aucun
    /// répertoire `tests/` ne peut exister sous ce module
    /// (`perimeter::tests::milestone_manager_has_no_nested_tests_directory`),
    /// donc tout test du module est un `mod tests` en ligne, et la troncature
    /// suffit à séparer production et tests (prémisse mika#2321).
    ///
    /// Les commentaires tombent parce que la prose *parle* de l'interrupteur du
    /// vol 2 — le paragraphe de docstring juste au-dessus le nomme — et qu'une
    /// mention n'est pas une déclaration. Même forme que
    /// `mika2425_identity_context_history_has_a_single_reader` (motif mika#2050
    /// signal S).
    fn production_sans_commentaires() -> Vec<(String, String)> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/milestone_manager");
        let mut out: Vec<(String, String)> = Vec::new();
        for entry in fs::read_dir(dir).expect("le répertoire du module doit être lisible") {
            let path = entry.expect("entrée de répertoire lisible").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let nom = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let src = fs::read_to_string(&path).expect("source du module lisible");
            let production = src.split("#[cfg(test)]").next().unwrap_or_default();
            let code = production
                .lines()
                .map(|l| l.split("//").next().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n");
            out.push((nom, code));
        }
        out
    }

    /// Cas 16 — le vol 1 n'ouvre aucun chemin d'écriture : AC1 sous sa forme la
    /// plus forte.
    ///
    /// L'AC1 demande « avec l'interrupteur OFF, aucun déclenchement de groom ».
    /// Le vol 1 satisfait la forme **dégénérée** de cette garantie : il n'y a
    /// pas d'interrupteur, donc pas de branche ON, donc aucun déclenchement
    /// n'est possible. Deux moitiés le tiennent structurellement.
    ///
    /// **Moitié 1 — aucune exception du scan n'est attribuable à ce ticket.**
    /// `no_dispatch_test.rs` couvre tout le répertoire et reste **intouché** par
    /// ce vol : `relief.rs` et `reporter.rs` y sont donc soumis comme les
    /// autres. La lecture est textuelle et non programmatique parce que
    /// `EXEMPTED_OCCURRENCES` est privée à un module *frère* : la lire
    /// demanderait de la passer `pub(super)`, c'est-à-dire de toucher le
    /// fichier dont ce test affirme qu'il n'a pas bougé.
    ///
    /// L'assertion n'est pas « l'allowlist est vide » — elle ne l'est pas, et
    /// n'avait pas à l'être : elle porte depuis mika#2698 vol 2 exactement une
    /// entrée, épinglée à une par
    /// `mika2698_lallowlist_ne_porte_que_le_geste_du_vol_2`. L'assertion est
    /// « aucune exception attribuable à mika#2700 », ce qui est la même
    /// propriété pour ce vol-ci, et ce qui rougit au vol 2.
    ///
    /// **Moitié 2 — l'interrupteur du vol 2 n'est pas déclaré.** Un
    /// `pub const ENV_*` sans lecteur serait une promesse d'armement non tenue,
    /// et il déclencherait `mika2267_every_manager_env_const_is_declared_in_env_example`
    /// sans qu'aucun comportement ne le justifie.
    #[test]
    fn mika2700_le_vol_1_nouvre_aucun_chemin_decriture() {
        let garde = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/milestone_manager/no_dispatch_test.rs"
        ))
        .expect("le garde structurel doit être lisible");
        for jeton in ["relief.rs", "reporter.rs", "mika#2700"] {
            assert!(
                !garde.contains(jeton),
                "aucune exception du scan n'est attribuable à mika#2700, et `{jeton}` en nomme \
                 une. Si c'est le vol 2 qui arrive, l'exception s'inscrit AVEC son fichier, son \
                 jeton et sa raison, et ce test change dans le même commit que le paragraphe du \
                 docstring — jamais l'un sans l'autre."
            );
        }

        let sources = production_sans_commentaires();
        // Contrôle positif du lecteur : un scan qui ne trouve jamais rien et un
        // module propre rendent les mêmes octets (classe mika#2205).
        assert!(
            sources
                .iter()
                .any(|(nom, code)| nom == "relief.rs" && code.contains("pub fn name_relief")),
            "le lecteur doit voir la production de relief.rs — sinon « aucun déclarant » ne \
             prouve rien. Lus : {:?}",
            sources.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );

        // Le besoin est épelé en deux morceaux pour que ce test ne se trouve pas
        // lui-même si la troncature venait à ne plus l'exclure.
        let interrupteur = concat!("MIKA_MANAGER_", "GROOM_WRITE");
        let declarants: Vec<String> = sources
            .into_iter()
            .filter(|(_, code)| code.contains(interrupteur))
            .map(|(nom, _)| nom)
            .collect();
        assert!(
            declarants.is_empty(),
            "le vol 1 ne déclare pas l'interrupteur du vol 2 — un interrupteur dont la branche \
             ON ne fait rien est un mensonge de surface. Déclaré dans : {declarants:?}. Si c'est \
             le vol 2 qui arrive, il arrive avec son `write_is_armed()`, sa table de vérité et \
             son contrôle négatif, et ce test change dans le même commit."
        );
    }
}

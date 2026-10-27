//! Le registre des verdicts de vol (mika#2922, vols 1 et 2).
//!
//! # Ce que ce module est, et ce qu'il n'est pas encore
//!
//! Il inscrit **les deux côtés de la mesure**, sous la même clé et chacun sous
//! son `tool_name` :
//!
//! - le verdict de l'évaluateur ([`MANAGER_FLIGHT_VERDICT_TOOL`], vol 1) — une
//!   ligne par vol et par cycle qui le juge pour la **première** fois, écrite
//!   par [`write_and_swallow`] ;
//! - le jugement de MPC ([`MPC_FLIGHT_JUDGMENT_TOOL`], vol 2) — une ligne par
//!   vol, inscrite par un geste d'opérateur qui passe par
//!   [`record_mpc_judgment`], jamais par du SQL à la main.
//!
//! C'est la forme du vol 2 de mika#2917, que le ticket désigne comme le patron à
//! reprendre : un `tool_name` par écrivain, un seul écrivain par `tool_name`,
//! aucune table et aucune migration.
//!
//! **Personne ne lit encore.** Le compte de concordance — celui qui met les deux
//! côtés en face et rend « N vols consécutifs concordants » — est le vol 3. Il
//! n'y a donc aucune garde de lecteur unique dans ce fichier : une garde sans
//! lecteur serait verte par construction, ce qui est précisément la classe
//! mika#2205. Les deux constantes déclarées ici sont en revanche l'une et
//! l'autre **écrites** par ce module, donc leurs gardes d'écrivain unique
//! mordent.
//!
//! # La forme de la ligne
//!
//! ```text
//! tool_name   = MANAGER_FLIGHT_VERDICT_TOOL
//! session_id  = MANAGER_FLIGHT_VERDICT_SESSION_ID  (un verdict n'appartient à aucune conversation)
//! target_key  = FlightVerdict::task_id             (tasks.id — « la clé qui rend le verdict rejouable »)
//! after_value = JSON serde de VerdictRecord { issue, recommendation }
//! before_value= NULL                               (il n'y a aucun état antérieur à nommer)
//! reasoning   = "issue_number=<n>" quand le vol porte un ticket lisible, sinon NULL
//! agent_id    = injecté par AsyncDatabase, jamais passé par l'appelant
//! ```
//!
//! `after_value` est le JSON serde des types **existants** : `FlightIssue`,
//! `DeathCause` et `FlightRecommendation` dérivent déjà
//! `Serialize`/`Deserialize` avec `rename_all = "snake_case"`, et
//! [`VerdictRecord`] les porte tels quels. Une grammaire `k=v;k=v` inventée ici
//! serait un second vocabulaire à tenir en phase avec trois `enum` — la
//! divergence programmée de mika#2158.
//!
//! Le jugement de MPC porte la **même** grammaire sous la **même** clé :
//!
//! ```text
//! tool_name   = MPC_FLIGHT_JUDGMENT_TOOL
//! session_id  = MPC_FLIGHT_JUDGMENT_SESSION_ID
//! target_key  = le MÊME tasks.id
//! after_value = JSON serde de VerdictRecord { issue, recommendation }
//! before_value= NULL
//! reasoning   = la note libre de l'opérateur, ou NULL
//! ```
//!
//! `reasoning` diverge délibérément d'un côté à l'autre : le manager y porte
//! `issue_number=<n>`, MPC une note libre. Le compte de concordance lira donc le
//! ticket d'un vol sur la ligne **du manager** seule — un `reasoning` est une
//! donnée et jamais une clé, de ce côté-ci non plus.
//!
//! # Pourquoi là, et pas sous `src/milestone_manager/`
//!
//! 1. **Aucune autorité, et la garde reste vraie sans être touchée.**
//!    `milestone_manager/no_dispatch_test.rs` grep chaque `.rs` sous
//!    `src/milestone_manager/`. Ce fichier est un **frère** de ce répertoire,
//!    donc hors du périmètre scanné : AC5 est satisfait parce que l'écrivain
//!    vit dehors, pas parce qu'une exception l'excepte. C'est déjà la raison
//!    d'être de [`crate::flight_source`].
//! 2. **La raison matérielle, par-dessus la garde.** La tâche de cadence ne
//!    tient **aucun** handle de base : elle est construite d'un `ManagerConfig`,
//!    d'un `CancellationToken` et d'un `TokenResolver`. L'injection n'est pas un
//!    goût de découplage, c'est le seul moyen d'atteindre la base depuis là.
//! 3. **Trait et impl ensemble, l'idiome d'[`crate::auth_boundary_ledger`].**
//!    `FlightSource` déclare son trait *dans* `evaluator.rs` parce que la
//!    signature de l'évaluateur a besoin du type ; un registre n'a besoin
//!    d'aucun type à l'intérieur du module gardé, qui ne nomme qu'un
//!    `Option<&Arc<dyn FlightVerdictLedger>>`. Empreinte minimale dans le
//!    périmètre gardé, et un précédent qui roule déjà dans le même
//!    `CycleContext`.
//! 4. **Aucune table, aucune migration.** Les cinq données entrent dans les
//!    colonnes existantes ; le schéma reste à v56. `auth_boundary` et
//!    `agent_recurring_gate` portent la même forme.
//! 5. **Par agent sans que l'appelant le dise.**
//!    `AsyncDatabase::log_audit_event` clone son propre `agent_id` et ne
//!    l'accepte pas en argument — son premier paramètre est le `session_id`. Le
//!    scope est donc **structurel**, pas une discipline d'appel.
//!
//! # L'idempotence est une lecture, pas un `UNIQUE`
//!
//! Avant d'écrire, [`write_and_swallow`] et [`record_mpc_judgment`] demandent
//! `count_recent_audit_events_for_target` sur une fenêtre de
//! [`AUDIT_RETENTION_DAYS`]. Au-delà de la rétention la ligne n'existe plus :
//! « jamais inscrit » et « inscrit hors rétention » sont la même population, et
//! la fenêtre de l'idempotence est celle du grand livre **par construction**. Un
//! index `UNIQUE` demanderait une migration pour une propriété qu'une lecture
//! donne. Coût assumé : un vol jugé il y a plus de 90 jours qui reparaîtrait
//! dans le corpus serait ré-inscrit — et il n'est plus dans la fenêtre de
//! l'évaluateur.
//!
//! # `record` est `async`, et ne rend aucune erreur
//!
//! [`AuthBoundaryLedger::record`](crate::auth_boundary_ledger::AuthBoundaryLedger::record)
//! rend `()` **et** part en `handle.spawn` détaché. On garde la première moitié
//! et on abandonne la seconde :
//!
//! - **Aucune erreur dans la signature.** [`FlightVerdictLedger::record`] rend
//!   un [`InscriptionTally`] — trois compteurs, jamais un `Result`. Il n'y a
//!   donc aucune erreur à propager et aucun `?` à oublier : une écriture
//!   d'audit en échec ne peut pas changer un verdict, parce que la signature
//!   n'offre aucun moyen de le faire. C'est la propriété R5
//!   d'`auth_boundary_ledger`, structurelle et non conventionnelle. L'échec est
//!   nommé par un WARN `flight_verdict_audit_write_failed`.
//! - **Le tally est ce qui rend AC1 mesurable, et c'est pourquoi il n'est pas
//!   `()`.** AC1 est une propriété mesurée — « exactement une ligne par vol, un
//!   second cycle n'en ajoute aucune » — et la sonde de la cadence doit porter
//!   `inscribed` et `already_recorded` comme **deux** champs distincts : un
//!   `()` les rendrait tous deux inobservables, et « rien inscrit » serait
//!   indistinguable de « tout déjà inscrit » (classe mika#2205). Trois `usize`
//!   ne sont pas une erreur : R5 tient entière.
//! - **Est `async` et attendu.** Un `spawn` détaché rendrait AC1 inobservable,
//!   et `auth_boundary_ledger` a dû extraire `write_and_swallow` précisément
//!   pour qu'un test puisse l'attendre (« a test that cannot fail is not a
//!   test »). L'appelant n'est pas sur un chemin critique : la cadence tourne
//!   toutes les N minutes et attend déjà un appel `gh` dans
//!   `post_flight_verdicts`. Le garde `Handle::try_current()` disparaît donc
//!   avec le `spawn` qu'il protégeait.
//!
//! **[`record_mpc_judgment`] fait exactement l'inverse, et pour la raison
//! inverse.** Elle rend un `Result`, parce que son appelant est une commande
//! d'opérateur : avaler l'échec y dirait « inscrit » à quelqu'un qui n'a rien
//! inscrit, et le jugement serait perdu sans que personne le sache. Le cycle de
//! cadence, lui, n'a aucun geste de repli à offrir — il juge, il n'attend pas.
//! La règle n'est donc pas « un audit n'échoue jamais bruyamment » mais *l'échec
//! va là où quelqu'un peut en faire quelque chose*.
//!
//! # Rien n'est derrière l'interrupteur, et c'est tout le ticket
//!
//! L'interrupteur du geste sur la forge — `verdict_poster::ENV_EVALUATE_WRITE` —
//! n'est **pas** lu par ce module. Il gouverne le geste **sur la forge** ; le
//! registre existe pour mesurer l'accord **en ombre**, c'est-à-dire exactement
//! quand l'interrupteur est fermé. Un registre conditionné à son propre
//! interrupteur ne pourrait jamais produire la mesure qui l'ouvre. Son lecteur
//! unique reste `verdict_poster::write_is_armed` (doctrine mika#2201), et une
//! garde de ce module assère que ce fichier ne nomme ni l'un ni l'autre.
//!
//! Le **littéral** de la variable d'environnement n'est pas retapé ici, même en
//! commentaire. L'instrument d'AC5 est un `grep -rn` de ce littéral sous
//! `crates/mika-agent/src/` ; son relevé est **8**, et une mention de prose le
//! ferait passer à 9 — donc mentir. On nomme la constante, jamais sa valeur.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::evidence::audit::AUDIT_RETENTION_DAYS;
use crate::milestone_manager::evaluator::{
    DeathCause, FlightIssue, FlightRecommendation, FlightVerdict,
};

/// `audit_events.tool_name` du registre des verdicts de vol du manager
/// (mika#2922, vol 1).
///
/// Cible `FlightVerdict::task_id` (`tasks.id`) ; `after_value` est le JSON serde
/// d'un [`VerdictRecord`].
///
/// **SOLE WRITER** : [`write_and_swallow`], dans ce fichier. Aucun autre chemin
/// du moteur ne l'écrit, et `mika2922_ac2_un_seul_ecrivain_du_verdict_du_manager`
/// en fait une assertion d'ensemble exact. Quand cet ensemble grandit, le remède
/// est de router l'écriture par ce module — il n'y a pas d'allowlist à amender.
///
/// **Lecteur** : aucun dans ce vol ; le compte de concordance au vol 2.
pub const MANAGER_FLIGHT_VERDICT_TOOL: &str = "manager_flight_verdict";

/// `session_id` porté par chaque ligne de verdict.
///
/// `audit_events.session_id` est `NOT NULL` sans clé étrangère, et un verdict de
/// vol n'appartient à aucune conversation : une constante bien connue est le
/// remplissage honnête — elle dit « hors session » au lieu d'emprunter une
/// session sans rapport et de faire passer la ligne pour le produit d'un tour.
/// Même raisonnement que `AUTH_BOUNDARY_SESSION_ID`.
pub const MANAGER_FLIGHT_VERDICT_SESSION_ID: &str = "manager-flight-verdict";

/// `audit_events.tool_name` du jugement de MPC sur un vol (mika#2922, vol 2).
///
/// **La même clé, un `tool_name` distinct.** `target_key` est le même `tasks.id`
/// que la ligne du manager — c'est ce qui permet de mettre les deux côtés en
/// face — et le `tool_name` diffère, c'est ce qui interdit de les confondre. Un
/// `tool_name` partagé avec un champ « côté » dans `after_value` rendrait les
/// deux populations non soustractibles (classe mika#2205) et ferait de
/// l'idempotence de chaque côté celle de l'autre.
///
/// **SOLE WRITER** : [`record_mpc_judgment`], dans ce fichier, et le seul chemin
/// supporté pour l'atteindre est la sous-commande `mika flight-verdicts judge`.
/// `mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc` en fait une assertion
/// d'ensemble exact. Quand cet ensemble grandit, le remède est de router
/// l'écriture par [`record_mpc_judgment`] — il n'y a pas d'allowlist à amender
/// (doctrine mika#2201).
///
/// **Lecteur** : aucun dans ce vol ; le compte de concordance au vol 3.
pub const MPC_FLIGHT_JUDGMENT_TOOL: &str = "mpc_flight_judgment";

/// `session_id` porté par chaque ligne de jugement.
///
/// Même raisonnement que [`MANAGER_FLIGHT_VERDICT_SESSION_ID`] : un jugement
/// d'opérateur sur un vol n'appartient à aucune conversation, et une constante
/// bien connue le dit au lieu d'emprunter une session sans rapport.
pub const MPC_FLIGHT_JUDGMENT_SESSION_ID: &str = "mpc-flight-judgment";

/// Ce qu'une ligne du registre porte : l'issue du vol et la recommandation.
///
/// Les trois termes que le ticket nomme — issue, cause de mort, recommandation —
/// y sont au complet et **aucun n'est retapé** : `FlightIssue::Dead` porte sa
/// [`DeathCause`], et les trois `enum` dérivent déjà leur serde.
///
/// La ligne reste **fidèle** — elle garde le `number`/`url` d'une PR, donc elle
/// est rejouable. C'est la **comparaison** du vol 2 qui est décisionnelle, par
/// [`VerdictRecord::terms`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub issue: FlightIssue,
    pub recommendation: FlightRecommendation,
}

impl From<&FlightVerdict> for VerdictRecord {
    fn from(v: &FlightVerdict) -> Self {
        Self {
            issue: v.issue.clone(),
            recommendation: v.recommendation,
        }
    }
}

/// Les trois termes sur lesquels un accord se décide.
///
/// `cause` vaut `None` hors d'une mort : « pas de cause » n'est pas une cause
/// (motif mika#2331). L'ordre des trois est fixe — `issue`, puis `cause`, puis
/// `recommendation` — et le **premier** qui diverge est le champ que la commande
/// du vol 2 nommera (AC3).
///
/// **Pourquoi une projection et pas l'égalité des lignes** :
/// `FlightIssue::PrOpened` porte `number` et `url`. MPC, jugeant à la main, ne
/// connaît pas l'URL — et un désaccord sur une URL n'est pas un désaccord sur un
/// verdict. Comparer les lignes brutes rendrait « divergent » sur du bruit, et
/// l'instrument annoncerait 0 concordant le jour où tout concorde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictTerms {
    pub issue: &'static str,
    pub cause: Option<String>,
    pub recommendation: &'static str,
}

impl VerdictRecord {
    /// La projection décisionnelle de cette ligne.
    pub fn terms(&self) -> VerdictTerms {
        VerdictTerms {
            issue: issue_term(&self.issue),
            cause: match &self.issue {
                FlightIssue::Dead { cause } => Some(cause_term(cause)),
                FlightIssue::PrOpened { .. } | FlightIssue::RescuePr { .. } => None,
            },
            recommendation: recommendation_term(self.recommendation),
        }
    }
}

/// Le littéral serde de la variante d'issue.
///
/// `match` **exhaustif** plutôt qu'une introspection de `serde_json` : une
/// quatrième variante de [`FlightIssue`] ne compile pas ici tant qu'elle n'a pas
/// son terme. Ce que l'exhaustivité n'achète pas — la *graphie* — est tenu par
/// `mika2922_les_termes_sont_les_litteraux_serde`, qui compare chaque terme à ce
/// que serde écrit réellement.
fn issue_term(issue: &FlightIssue) -> &'static str {
    match issue {
        FlightIssue::PrOpened { .. } => "pr_opened",
        FlightIssue::RescuePr { .. } => "rescue_pr",
        FlightIssue::Dead { .. } => "dead",
    }
}

/// Le littéral serde de la cause de mort. [`DeathCause::Unknown`] **porte** son
/// jeton : deux haltes non classées de familles différentes ne sont pas la même
/// cause, et effacer le jeton les rendrait concordantes à tort.
fn cause_term(cause: &DeathCause) -> String {
    match cause {
        DeathCause::CeilingReached => "ceiling_reached".to_string(),
        DeathCause::Refusal => "refusal".to_string(),
        DeathCause::PipelineFailure => "pipeline_failure".to_string(),
        DeathCause::LauncherDead => "launcher_dead".to_string(),
        DeathCause::Escalated => "escalated".to_string(),
        DeathCause::Unknown(token) => format!("unknown:{token}"),
    }
}

/// Le littéral serde de la recommandation.
fn recommendation_term(r: FlightRecommendation) -> &'static str {
    match r {
        FlightRecommendation::MergeCandidate => "merge_candidate",
        FlightRecommendation::ManualResumption => "manual_resumption",
        FlightRecommendation::Regroom => "regroom",
    }
}

/// Ce qu'une passe d'inscription a mesuré.
///
/// `inscribed` et `already_recorded` sont **deux** champs et jamais une somme :
/// ils ne demandent pas le même geste, et leur somme ne dirait pas si la
/// non-réécriture a eu lieu ou si le cycle n'a rien vu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InscriptionTally {
    pub inscribed: usize,
    pub already_recorded: usize,
    pub failed: usize,
}

impl InscriptionTally {
    const fn one_inscribed() -> Self {
        Self {
            inscribed: 1,
            already_recorded: 0,
            failed: 0,
        }
    }

    const fn one_already_recorded() -> Self {
        Self {
            inscribed: 0,
            already_recorded: 1,
            failed: 0,
        }
    }

    const fn one_failed() -> Self {
        Self {
            inscribed: 0,
            already_recorded: 0,
            failed: 1,
        }
    }

    fn plus(self, other: Self) -> Self {
        Self {
            inscribed: self.inscribed + other.inscribed,
            already_recorded: self.already_recorded + other.already_recorded,
            failed: self.failed + other.failed,
        }
    }
}

/// Où les verdicts d'un cycle s'inscrivent.
#[async_trait]
pub trait FlightVerdictLedger: Send + Sync {
    /// Inscrire ces verdicts, sans jamais rendre d'erreur ni changer un verdict
    /// — voir le § *`record` est `async`* de la doc de module.
    async fn record(&self, verdicts: &[FlightVerdict]) -> InscriptionTally;
}

/// Le registre de production : une ligne `audit_events` par vol jugé.
pub struct DbFlightVerdictLedger {
    db: AsyncDatabase,
}

impl DbFlightVerdictLedger {
    pub fn new(db: AsyncDatabase) -> Self {
        Self { db }
    }
}

#[async_trait]
impl FlightVerdictLedger for DbFlightVerdictLedger {
    async fn record(&self, verdicts: &[FlightVerdict]) -> InscriptionTally {
        let mut tally = InscriptionTally::default();
        for verdict in verdicts {
            tally = tally.plus(write_and_swallow(&self.db, verdict).await);
        }
        tally
    }
}

/// Borne basse de la lecture d'idempotence : la rétention du grand livre
/// lui-même.
///
/// Dérivée d'[`AUDIT_RETENTION_DAYS`] plutôt qu'écrite ici, pour que la fenêtre
/// ne puisse pas survivre aux lignes qu'elle lit — même geste que
/// `degroom_marker_lookup_since` (mika#2242).
fn inscription_window_start() -> String {
    crate::timestamp::now_minus(chrono::Duration::days(AUDIT_RETENTION_DAYS as i64))
}

/// Inscrire un verdict, et avaler l'échec. Le corps entier de
/// [`DbFlightVerdictLedger::record`] pour un vol, extrait pour qu'un test
/// puisse l'attendre plutôt que de l'argumenter.
///
/// Rend un tally dont exactement un des trois compteurs vaut 1. Un échec de la
/// **lecture** d'idempotence compte comme `failed` et n'écrit pas : écrire sans
/// savoir si la ligne existe est le seul geste qui pourrait casser AC1, et le
/// cycle suivant réessaiera.
pub async fn write_and_swallow(db: &AsyncDatabase, verdict: &FlightVerdict) -> InscriptionTally {
    let since = inscription_window_start();
    match db
        .count_recent_audit_events_for_target(MANAGER_FLIGHT_VERDICT_TOOL, &verdict.task_id, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return InscriptionTally::one_already_recorded(),
        Err(e) => {
            warn_write_failed(verdict, "idempotence_read", &e.to_string());
            return InscriptionTally::one_failed();
        }
    }

    let record = VerdictRecord::from(verdict);
    let after_value = match serde_json::to_string(&record) {
        Ok(json) => json,
        Err(e) => {
            warn_write_failed(verdict, "serialize", &e.to_string());
            return InscriptionTally::one_failed();
        }
    };
    // `reasoning` porte une donnée, jamais une clé de regroupement : le
    // regroupement se fait sur `tool_name` et `target_key`. Même règle que
    // `agent_recurring_gate` (`reasoning = "task_id=<n>"`).
    let reasoning = verdict.issue_number.map(|n| format!("issue_number={n}"));
    if let Err(e) = db
        .log_audit_event(
            MANAGER_FLIGHT_VERDICT_SESSION_ID,
            MANAGER_FLIGHT_VERDICT_TOOL,
            &verdict.task_id,
            None,
            Some(&after_value),
            reasoning.as_deref(),
            None,
        )
        .await
    {
        warn_write_failed(verdict, "insert", &e.to_string());
        return InscriptionTally::one_failed();
    }
    InscriptionTally::one_inscribed()
}

/// WARN, et non ERROR : le verdict du cycle est correct dans les deux cas. Ce
/// qui est perdu est une ligne de visibilité opérateur, et cette ligne-ci est le
/// témoignage de la perte. `stage` nomme laquelle des trois moitiés a échoué —
/// un seul nom d'événement, parce que le regroupement se fait dessus.
fn warn_write_failed(verdict: &FlightVerdict, stage: &str, error: &str) {
    warn!(
        target: "mika::flight_verdict_ledger",
        event = "flight_verdict_audit_write_failed",
        task_id = %verdict.task_id,
        issue_number = verdict.issue_number.unwrap_or(0),
        stage = stage,
        error = error,
        "le verdict de ce vol n'est pas inscrit — le verdict lui-même tient"
    );
}

/// Inscrire par un registre optionnel. `None` est un no-op par construction : un
/// cycle sans registre câblé doit juger exactement comme avant, et le `None`
/// rendu est ce qui fait la branche `unwired` de la sonde de la cadence.
pub async fn record_if_wired(
    ledger: Option<&Arc<dyn FlightVerdictLedger>>,
    verdicts: &[FlightVerdict],
) -> Option<InscriptionTally> {
    match ledger {
        Some(l) => Some(l.record(verdicts).await),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Le jugement de MPC (vol 2) : l'analyse des termes, puis l'inscription.
// ---------------------------------------------------------------------------

/// Les termes d'issue qu'un jugement peut porter, dans l'ordre du ticket.
///
/// Les graphies sont demandées à [`issue_term`] — le site unique déjà épinglé
/// sur serde par `mika2922_les_termes_sont_les_litteraux_serde` — et non
/// retapées. Une seconde table de littéraux ici serait verte le jour où une
/// variante est renommée en amont : c'est exactement la divergence programmée de
/// mika#2158.
fn judgeable_issue_terms() -> Vec<&'static str> {
    [
        FlightIssue::PrOpened {
            number: None,
            url: None,
        },
        FlightIssue::RescuePr {
            number: None,
            url: None,
        },
        FlightIssue::Dead {
            cause: DeathCause::Refusal,
        },
    ]
    .iter()
    .map(issue_term)
    .collect()
}

/// Le terme de l'issue qui **porte** une cause, demandé à [`issue_term`] plutôt
/// qu'écrit en dur : une cause est exigée pour celle-là et refusée pour les
/// autres, et ce test ne doit pas être un quatrième endroit où le mot est tapé.
fn dead_issue_term() -> &'static str {
    issue_term(&FlightIssue::Dead {
        cause: DeathCause::Refusal,
    })
}

/// Les termes de cause qu'un jugement peut porter : les cinq causes **classées**.
///
/// [`DeathCause::Unknown`] n'est délibérément pas offerte. C'est ce que le
/// manager écrit quand il ne **sait pas** classer une halte, en portant le jeton
/// qu'il a vu ; un opérateur qui juge sait, et un « inconnu » saisi à la main ne
/// se comparerait à rien. Son refus n'est pas une vérification : il est
/// structurel, parce que la forme serde d'une variante newtype est un objet
/// (`{"unknown":"<jeton>"}`) et jamais un terme nu, donc aucune valeur de
/// `--cause` ne peut la produire. `mika2922_un_jugement_ne_porte_pas_dinconnu`
/// l'atteste.
fn judgeable_cause_terms() -> Vec<String> {
    [
        DeathCause::CeilingReached,
        DeathCause::Refusal,
        DeathCause::PipelineFailure,
        DeathCause::LauncherDead,
        DeathCause::Escalated,
    ]
    .iter()
    .map(cause_term)
    .collect()
}

/// Les termes de recommandation qu'un jugement peut porter. Même geste que
/// [`judgeable_issue_terms`] : les graphies viennent de [`recommendation_term`].
fn judgeable_recommendation_terms() -> Vec<&'static str> {
    [
        FlightRecommendation::MergeCandidate,
        FlightRecommendation::ManualResumption,
        FlightRecommendation::Regroom,
    ]
    .into_iter()
    .map(recommendation_term)
    .collect()
}

/// Analyser les trois termes d'un jugement d'opérateur — l'inverse **exact** de
/// serde.
///
/// **Aucune table de correspondance n'est recopiée.** L'analyse bâtit la
/// `Value` de la forme que `#[serde(rename_all = "snake_case")]` produit —
/// `{"dead":{"cause":"refusal"}}`, `{"pr_opened":{"number":null,"url":null}}`,
/// `"regroom"` — et la passe à `serde_json::from_value`. Conséquence voulue :
/// une sixième cause de mort ajoutée à [`DeathCause`] est acceptée sans qu'une
/// ligne bouge ici, et seule l'énumération du message de refus demande un geste
/// (que [`judgeable_cause_terms`] rend visible).
///
/// `number` et `url` sont `null` : MPC juge un vol, il ne connaît pas la PR.
/// « Pas d'URL » n'est pas une URL vide (motif mika#2331), et c'est pourquoi la
/// comparaison du vol 3 passe par [`VerdictRecord::terms`] et non par l'égalité
/// des lignes — voir le § *Pourquoi une projection* de [`VerdictTerms`].
pub fn parse_judgment_terms(
    issue: &str,
    cause: Option<&str>,
    recommendation: &str,
) -> Result<VerdictRecord> {
    Ok(VerdictRecord {
        issue: parse_issue_term(issue, cause)?,
        recommendation: parse_recommendation_term(recommendation)?,
    })
}

fn parse_issue_term(issue: &str, cause: Option<&str>) -> Result<FlightIssue> {
    let payload = if issue == dead_issue_term() {
        let cause = cause.ok_or_else(|| {
            anyhow!(
                "--cause est exigée avec --issue {} ; valeurs acceptées : {}",
                dead_issue_term(),
                judgeable_cause_terms().join(" · ")
            )
        })?;
        json!({ "cause": parse_cause_term(cause)? })
    } else {
        if cause.is_some() {
            bail!(
                "--cause ne s'applique qu'à --issue {} ; reçue avec « {issue} »",
                dead_issue_term()
            );
        }
        json!({ "number": null, "url": null })
    };

    // La forme externally tagged que serde attend : `{"<terme>": <charge>}`. La
    // clé est le terme reçu tel quel, donc un terme inconnu échoue ici — c'est
    // serde qui refuse, pas une liste tenue à la main.
    let mut tagged = Map::new();
    tagged.insert(issue.to_string(), payload);
    serde_json::from_value(Value::Object(tagged)).map_err(|_| {
        anyhow!(
            "--issue inconnue « {issue} » ; valeurs acceptées : {}",
            judgeable_issue_terms().join(" · ")
        )
    })
}

fn parse_cause_term(cause: &str) -> Result<DeathCause> {
    serde_json::from_value(Value::String(cause.to_string())).map_err(|_| {
        anyhow!(
            "--cause inconnue « {cause} » ; valeurs acceptées : {}",
            judgeable_cause_terms().join(" · ")
        )
    })
}

fn parse_recommendation_term(recommendation: &str) -> Result<FlightRecommendation> {
    serde_json::from_value(Value::String(recommendation.to_string())).map_err(|_| {
        anyhow!(
            "--recommendation inconnue « {recommendation} » ; valeurs acceptées : {}",
            judgeable_recommendation_terms().join(" · ")
        )
    })
}

/// Ce qu'une inscription de jugement a fait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpcJudgmentOutcome {
    /// La ligne vient d'être écrite.
    Inscribed,
    /// Une ligne existait déjà dans la fenêtre : rien n'est réécrit.
    ///
    /// `held` porte le jugement **en place**, parce qu'un simple « déjà là »
    /// laisserait l'opérateur corriger à l'aveugle. Il vaut `None` quand la
    /// ligne existe mais que son `after_value` ne se re-type pas : absent n'est
    /// pas zéro (motif mika#2331), et un [`VerdictRecord`] fabriqué pour
    /// combler serait un jugement inventé.
    AlreadyRecorded { held: Option<VerdictRecord> },
}

/// Inscrire le jugement de MPC sur un vol, sous la même clé que le verdict du
/// manager.
///
/// **SOLE WRITER de [`MPC_FLIGHT_JUDGMENT_TOOL`].** `note` est la note libre de
/// l'opérateur et va dans `reasoning` : une donnée, jamais une clé de
/// regroupement.
///
/// Les erreurs **se propagent**, contrairement à [`write_and_swallow`] — voir le
/// § *`record` est `async`* de la doc de module. Il n'y a pas de verbe de
/// correction dans ce vol : un vol déjà jugé rend
/// [`MpcJudgmentOutcome::AlreadyRecorded`] avec ce qui est inscrit, et c'est
/// tout.
pub async fn record_mpc_judgment(
    db: &AsyncDatabase,
    task_id: &str,
    record: &VerdictRecord,
    note: Option<&str>,
) -> Result<MpcJudgmentOutcome> {
    let since = inscription_window_start();
    let already = db
        .count_recent_audit_events_for_target(MPC_FLIGHT_JUDGMENT_TOOL, task_id, &since)
        .await
        .with_context(|| {
            format!("lecture d'idempotence du jugement de MPC sur le vol {task_id}")
        })?;
    if already > 0 {
        return Ok(MpcJudgmentOutcome::AlreadyRecorded {
            held: held_judgment(db, task_id, &since).await,
        });
    }

    let after_value = serde_json::to_string(record)
        .with_context(|| format!("sérialisation du jugement de MPC sur le vol {task_id}"))?;
    db.log_audit_event(
        MPC_FLIGHT_JUDGMENT_SESSION_ID,
        MPC_FLIGHT_JUDGMENT_TOOL,
        task_id,
        None,
        Some(&after_value),
        note,
        None,
    )
    .await
    .with_context(|| format!("inscription du jugement de MPC sur le vol {task_id}"))?;
    Ok(MpcJudgmentOutcome::Inscribed)
}

/// Le jugement déjà en place, ou `None` quand il est illisible.
///
/// Un échec de lecture ou un `after_value` qui ne se re-type pas rend `None`
/// **sans** faire échouer l'appel : la propriété que l'appelant a demandée — ne
/// pas réécrire — est tenue, et rendre une erreur ici ferait passer une
/// idempotence réussie pour une inscription échouée. Le WARN est ce qui garde la
/// perte visible.
async fn held_judgment(db: &AsyncDatabase, task_id: &str, since: &str) -> Option<VerdictRecord> {
    let row = match db
        .latest_audit_event_for_target(MPC_FLIGHT_JUDGMENT_TOOL, task_id, since)
        .await
    {
        Ok(row) => row,
        Err(e) => {
            warn_held_unreadable(task_id, "read", &e.to_string());
            return None;
        }
    };
    let after_value = match row {
        Some((Some(after_value), _, _)) => after_value,
        // La ligne est comptée mais sans `after_value` : il n'y a rien à rendre,
        // et ce n'est pas une erreur de lecture.
        _ => return None,
    };
    match serde_json::from_str(&after_value) {
        Ok(record) => Some(record),
        Err(e) => {
            warn_held_unreadable(task_id, "deserialize", &e.to_string());
            None
        }
    }
}

/// WARN, et non ERROR : l'idempotence a tenu, seule la restitution est perdue.
fn warn_held_unreadable(task_id: &str, stage: &str, error: &str) {
    warn!(
        target: "mika::flight_verdict_ledger",
        event = "mpc_judgment_held_unreadable",
        task_id = %task_id,
        stage = stage,
        error = error,
        "le jugement déjà inscrit n'a pas pu être rendu — il reste inscrit"
    );
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// Un registre qui garde ce qu'on lui a remis, pour les assertions.
    #[derive(Default)]
    pub struct RecordingLedger {
        pub recorded: Mutex<Vec<FlightVerdict>>,
    }

    #[async_trait]
    impl FlightVerdictLedger for RecordingLedger {
        async fn record(&self, verdicts: &[FlightVerdict]) -> InscriptionTally {
            let mut held = self.recorded.lock().unwrap();
            held.extend(verdicts.iter().cloned());
            InscriptionTally {
                inscribed: verdicts.len(),
                already_recorded: 0,
                failed: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::test_support::RecordingLedger;
    use super::*;
    use crate::db::Database;
    use crate::source_scan::{
        audit_writers, production_files, production_half, strip_comment_lines,
    };

    /// Un verdict de vol mort, la forme qui porte une [`DeathCause`].
    fn verdict_mort(task_id: &str, cause: DeathCause) -> FlightVerdict {
        FlightVerdict {
            task_id: task_id.to_string(),
            repo: None,
            issue_number: Some(2922),
            issue: FlightIssue::Dead { cause },
            turns_used: Some(41),
            hit_ceiling: false,
            turn_ceiling: None,
            last_commit_turn: None,
            recommendation: FlightRecommendation::Regroom,
            pr_state: crate::milestone_manager::evaluator::PrStateKnowledge::NotRead,
            completed_at: Some("2026-10-26T10:00:00Z".to_string()),
        }
    }

    /// Un verdict de vol qui a ouvert une PR — la forme qui porte `number`/`url`,
    /// donc celle sur laquelle la projection doit rendre `cause: None`.
    fn verdict_pr(task_id: &str) -> FlightVerdict {
        FlightVerdict {
            task_id: task_id.to_string(),
            repo: None,
            issue_number: None,
            issue: FlightIssue::PrOpened {
                number: Some(2923),
                url: Some("https://github.com/senara-solutions/mika/pull/2923".to_string()),
            },
            turns_used: None,
            hit_ceiling: false,
            turn_ceiling: None,
            last_commit_turn: None,
            recommendation: FlightRecommendation::MergeCandidate,
            pr_state: crate::milestone_manager::evaluator::PrStateKnowledge::NotRead,
            completed_at: None,
        }
    }

    // ---- l'idiome d'injection ---------------------------------------------

    #[tokio::test]
    async fn un_registre_non_cable_est_un_no_op_pas_un_echec() {
        // Tout l'intérêt : un cycle sans registre tourne jusqu'au bout, et le
        // `None` rendu est ce qui fera la branche `unwired` de la sonde.
        let tally = record_if_wired(None, &[verdict_pr("t-1")]).await;
        assert_eq!(tally, None, "non câblé doit se distinguer de « 0 inscrit »");
    }

    #[tokio::test]
    async fn un_registre_cable_recoit_les_verdicts() {
        let recording = Arc::new(RecordingLedger::default());
        let ledger: Arc<dyn FlightVerdictLedger> = recording.clone();
        let verdicts = [
            verdict_pr("t-1"),
            verdict_mort("t-2", DeathCause::CeilingReached),
        ];

        let tally = record_if_wired(Some(&ledger), &verdicts).await;
        assert_eq!(
            tally,
            Some(InscriptionTally {
                inscribed: 2,
                already_recorded: 0,
                failed: 0,
            })
        );

        let got = recording.recorded.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].task_id, "t-1");
        assert_eq!(got[1].task_id, "t-2");
    }

    /// R5, avec l'échec **injecté** plutôt qu'argumenté.
    ///
    /// Le registre est pointé sur un `agent_id` sans ligne dans `agents`, donc
    /// l'INSERT trouve la clé étrangère et l'écriture d'audit échoue vraiment.
    /// `write_and_swallow` est appelé directement et **attendu** : un `spawn`
    /// détaché rendrait l'assertion inobservable, et un test qui ne peut pas
    /// échouer n'est pas un test.
    #[tokio::test]
    async fn mika2922_un_echec_decriture_est_avale_pas_propage() {
        let db =
            AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "no-such-agent");
        let verdict = verdict_mort("t-fk", DeathCause::LauncherDead);

        // Précondition : sans elle, la suite n'assère rien.
        assert!(
            db.log_audit_event(
                MANAGER_FLIGHT_VERDICT_SESSION_ID,
                MANAGER_FLIGHT_VERDICT_TOOL,
                &verdict.task_id,
                None,
                Some("{}"),
                None,
                None,
            )
            .await
            .is_err(),
            "la violation de clé étrangère doit faire échouer l'écriture d'audit"
        );

        // Le corps réel, attendu jusqu'au bout : il rend un compteur, jamais une
        // erreur, et aucune panique n'en sort.
        let tally = write_and_swallow(&db, &verdict).await;
        assert_eq!(tally, InscriptionTally::one_failed());
    }

    // ---- le format de ligne -----------------------------------------------

    /// L'aller-retour serde **et** l'accord des termes avec ce que serde écrit.
    ///
    /// Les deux moitiés tiennent deux choses distinctes. L'aller-retour tient le
    /// format de ligne : `after_value` est relisible, donc le lecteur du vol 2
    /// pourra re-typer ce que ce vol écrit. L'accord des termes tient la
    /// **graphie** de la projection — ce que le `match` exhaustif d'[`issue_term`]
    /// n'achète pas : un `rename_all` changé, ou un terme retapé de travers,
    /// rendrait la comparaison du vol 2 divergente sur du bruit sans qu'aucune
    /// compilation ne rougisse.
    #[test]
    fn mika2922_les_termes_sont_les_litteraux_serde() {
        for verdict in [
            verdict_pr("t-1"),
            verdict_mort("t-2", DeathCause::CeilingReached),
            verdict_mort("t-3", DeathCause::Unknown("halte_inconnue".to_string())),
        ] {
            let record = VerdictRecord::from(&verdict);
            let json = serde_json::to_string(&record).unwrap();
            let back: VerdictRecord = serde_json::from_str(&json).unwrap();
            assert_eq!(back, record, "aller-retour serde : {json}");

            // La graphie : chaque terme est ce que serde écrit réellement.
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            let terms = record.terms();
            let issue_serde = match &value["issue"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(map) => {
                    map.keys().next().expect("une variante nommée").clone()
                }
                other => panic!("forme d'issue inattendue : {other}"),
            };
            assert_eq!(terms.issue, issue_serde, "issue, dans {json}");
            assert_eq!(
                terms.recommendation,
                value["recommendation"].as_str().unwrap(),
                "recommendation, dans {json}"
            );
        }

        // La cause : présente sur une mort, absente ailleurs. « Pas de cause »
        // n'est pas une cause (motif mika#2331).
        assert_eq!(
            VerdictRecord::from(&verdict_mort("t", DeathCause::Refusal))
                .terms()
                .cause
                .as_deref(),
            Some("refusal")
        );
        assert_eq!(VerdictRecord::from(&verdict_pr("t")).terms().cause, None);
        // Une halte non classée porte son jeton : deux inconnues de familles
        // différentes ne doivent pas concorder.
        assert_eq!(
            VerdictRecord::from(&verdict_mort("t", DeathCause::Unknown("abc".into())))
                .terms()
                .cause
                .as_deref(),
            Some("unknown:abc")
        );
    }

    // ---- AC1, sur une vraie base ------------------------------------------

    /// **AC1.** Après un cycle sur un corpus de vols, chaque vol jugé a
    /// exactement une ligne ; un second cycle sur les mêmes vols n'en ajoute
    /// aucune.
    ///
    /// Sur une vraie base — `Database::open_in_memory()` + le vrai
    /// `AsyncDatabase` — parce que la propriété est celle d'une **lecture
    /// d'idempotence contre des lignes réellement écrites**. Un faux registre la
    /// rendrait vraie par construction.
    ///
    /// Les deux moitiés de l'assertion sont nécessaires et différentes :
    /// `inscribed`/`already_recorded` disent ce que la passe a *fait*, et les
    /// `target_key` disent ce que la base *porte*. Un registre qui compterait
    /// juste en n'écrivant rien passerait la première seule.
    #[tokio::test]
    async fn mika2922_ac1_chaque_vol_juge_a_exactement_une_ligne() {
        let db = AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "mika");
        let ledger = DbFlightVerdictLedger::new(db.clone());
        let corpus = [
            verdict_pr("task-aaa"),
            verdict_mort("task-bbb", DeathCause::CeilingReached),
            verdict_mort("task-ccc", DeathCause::Unknown("halte_x".to_string())),
        ];

        let premier = ledger.record(&corpus).await;
        assert_eq!(
            premier,
            InscriptionTally {
                inscribed: 3,
                already_recorded: 0,
                failed: 0,
            },
            "le premier passage inscrit les trois vols"
        );

        let mut cles = db
            .get_audit_event_target_keys_by_tool_name(MANAGER_FLIGHT_VERDICT_TOOL)
            .await
            .unwrap();
        cles.sort();
        assert_eq!(
            cles,
            vec![
                "task-aaa".to_string(),
                "task-bbb".to_string(),
                "task-ccc".to_string()
            ],
            "les clés sont exactement les trois tasks.id"
        );

        let second = ledger.record(&corpus).await;
        assert_eq!(
            second,
            InscriptionTally {
                inscribed: 0,
                already_recorded: 3,
                failed: 0,
            },
            "un second cycle sur les mêmes vols n'inscrit rien"
        );
        assert_eq!(
            db.get_audit_event_target_keys_by_tool_name(MANAGER_FLIGHT_VERDICT_TOOL)
                .await
                .unwrap()
                .len(),
            3,
            "et la base porte toujours trois lignes"
        );

        // La ligne est fidèle : `after_value` se re-type, `before_value` est
        // NULL, et `reasoning` porte le ticket quand le vol en a un.
        let (after, reasoning, _created) = db
            .latest_audit_event_for_target(
                MANAGER_FLIGHT_VERDICT_TOOL,
                "task-bbb",
                "2000-01-01T00:00:00Z",
            )
            .await
            .unwrap()
            .expect("la ligne de task-bbb existe");
        let relu: VerdictRecord = serde_json::from_str(&after.unwrap()).unwrap();
        assert_eq!(relu.terms().cause.as_deref(), Some("ceiling_reached"));
        assert_eq!(reasoning.as_deref(), Some("issue_number=2922"));

        // Et un vol sans ticket lisible laisse `reasoning` NULL plutôt que de
        // porter un zéro qui se lirait comme un numéro.
        let (_, sans_ticket, _) = db
            .latest_audit_event_for_target(
                MANAGER_FLIGHT_VERDICT_TOOL,
                "task-aaa",
                "2000-01-01T00:00:00Z",
            )
            .await
            .unwrap()
            .expect("la ligne de task-aaa existe");
        assert_eq!(sans_ticket, None);
    }

    // ---- AC2, première moitié : un seul écrivain du verdict du manager ----

    /// **AC2** (moitié « verdict du manager »). Le `tool_name` du verdict n'a
    /// qu'un écrivain, et son littéral n'est recopié nulle part.
    ///
    /// Les deux assertions ne mesurent pas la même chose, et l'ensemble exact
    /// est **auto-nettoyant** — pas de `contains`, pas de `>=`, aucune
    /// allowlist. Quand il grandit, la résolution est de router l'écriture par
    /// [`DbFlightVerdictLedger`], **jamais** d'ajouter une entrée (doctrine
    /// mika#2201) : il n'y a pas de liste à allonger ici.
    ///
    /// Le scan du littéral ferme la classe mika#2158 que le scan d'écrivain ne
    /// voit pas : un site qui retape `"manager_flight_verdict"` au lieu de
    /// passer la constante écrit sous la même clé sans jamais apparaître comme
    /// écrivain, et une divergence de graphie resterait verte.
    #[test]
    fn mika2922_ac2_un_seul_ecrivain_du_verdict_du_manager() {
        let files = production_files();

        // Anti-vacuité : le scan a bien lu l'arbre (mika#2205). Sans ce terme,
        // « aucun second écrivain » est indistinguable de « rien n'a été lu ».
        assert!(
            files.len() >= 100,
            "anti-vacuité : {} fichiers lus, le scan n'a rien lu",
            files.len()
        );

        let writers = audit_writers("MANAGER_FLIGHT_VERDICT_TOOL");
        assert_eq!(
            writers,
            vec!["flight_verdict_ledger.rs".to_string()],
            "SOLE WRITER : un seul écrivain. Quand cet ensemble grandit, route \
             l'écriture par DbFlightVerdictLedger — n'ajoute pas d'entrée"
        );

        let literal = String::from("\"") + MANAGER_FLIGHT_VERDICT_TOOL + "\"";
        for (rel, prod) in &files {
            if rel == "flight_verdict_ledger.rs" {
                continue;
            }
            assert!(
                !prod.contains(&literal),
                "{rel} recopie le littéral {literal} ; passer par la constante"
            );
        }
    }

    /// Contrôle négatif — sans lui, « le scan n'a trouvé qu'un écrivain » est
    /// indistinguable de « la fenêtre de lecture ne mord sur rien ». Il inline
    /// la fenêtre de 5 lignes plutôt que de paramétrer [`audit_writers`] : ce
    /// que le contrôle vérifie est que **la fenêtre** mord, sur le même idiome
    /// que `qa_reviewed_head::tests::mika2917_a2_le_scan_decrivain_mord`.
    #[test]
    fn mika2922_le_scan_decrivain_mord() {
        let fixture = "db.log_audit_event(\n    MANAGER_FLIGHT_VERDICT_SESSION_ID,\n    \
                       MANAGER_FLIGHT_VERDICT_TOOL,\n";
        let lines: Vec<&str> = fixture.lines().collect();
        assert!(
            lines.iter().enumerate().any(|(i, l)| {
                l.contains("log_audit_event(")
                    && lines[i..(i + 5).min(lines.len())]
                        .iter()
                        .any(|w| w.contains("MANAGER_FLIGHT_VERDICT_TOOL"))
            }),
            "la fenêtre de 5 lignes doit voir le site fabriqué"
        );

        // Et elle ne mord pas sur un site d'audit étranger : un `tool_name`
        // voisin ne doit pas compter comme écrivain de celui-ci.
        let etranger = "db.log_audit_event(\n    \"auth-boundary\",\n    \
                        AUTH_BOUNDARY_TOOL_NAME,\n";
        let lines: Vec<&str> = etranger.lines().collect();
        assert!(
            !lines.iter().enumerate().any(|(i, l)| {
                l.contains("log_audit_event(")
                    && lines[i..(i + 5).min(lines.len())]
                        .iter()
                        .any(|w| w.contains("MANAGER_FLIGHT_VERDICT_TOOL"))
            }),
            "la fenêtre ne doit pas compter un écrivain d'un autre tool_name"
        );
    }

    // ---- AC5 : aucune autorité, et l'interrupteur reste intouché ----------

    /// **AC5.** Ce module ne nomme pas l'interrupteur du geste sur la forge, et
    /// il ne vit pas sous le périmètre que `no_dispatch_test.rs` balaie.
    ///
    /// Trois termes, et le troisième est ce qui empêche les deux premiers de
    /// devenir vacuellement vrais :
    ///
    /// 1. La moitié production, commentaires retirés, ne nomme ni
    ///    `ENV_EVALUATE_WRITE` ni `write_is_armed`. Le scan porte sur les
    ///    **noms de constante et de fonction**, jamais sur le littéral de la
    ///    variable d'environnement : écrire ce littéral ici — fût-ce dans un
    ///    commentaire — ferait passer le relevé d'AC5 de 8 à 9 et le ferait
    ///    mentir.
    /// 2. Le fichier n'est pas sous `src/milestone_manager/`, donc le balayage
    ///    de `no_dispatch_test.rs` ne le voit pas, et son ensemble de jetons
    ///    reste intouché. C'est la raison pour laquelle l'écrivain vit ici et
    ///    non dans le module, sur le même idiome que `flight_source.rs`.
    /// 3. **Auto-nettoyant** : la même moitié production *doit* contenir
    ///    `MANAGER_FLIGHT_VERDICT_TOOL`. Le jour où `production_half` cesse de
    ///    découper ce fichier — ancre changée, `mod tests` renommé — les deux
    ///    premières assertions deviendraient vraies sur une chaîne vide, et
    ///    celle-ci rougit à leur place.
    #[test]
    fn mika2922_ac5_le_module_ne_nomme_pas_linterrupteur() {
        let rel = "src/flight_verdict_ledger.rs";
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap();
        let code = strip_comment_lines(production_half(&src));

        assert!(
            code.contains("MANAGER_FLIGHT_VERDICT_TOOL"),
            "anti-vacuité : la moitié production ne porte même pas la constante \
             du module — le découpage n'a rien rendu"
        );
        assert!(
            !code.contains("ENV_EVALUATE_WRITE"),
            "ce module ne nomme pas l'interrupteur ; son lecteur unique reste \
             verdict_poster (doctrine mika#2201)"
        );
        assert!(
            !code.contains("write_is_armed"),
            "ce module ne consulte pas l'armement ; le registre mesure l'accord \
             en ombre, donc exactement quand l'interrupteur est fermé"
        );

        // Le périmètre : hors du module, donc hors du balayage de jetons.
        assert!(
            !rel.contains("milestone_manager/"),
            "l'écrivain doit rester hors de src/milestone_manager/ pour que \
             no_dispatch_test.rs passe sans modification de ses jetons"
        );
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/milestone_manager/no_dispatch_test.rs")
                .exists(),
            "anti-vacuité : la garde dont on préserve le périmètre doit exister"
        );
    }

    /// Contrôle négatif du scan d'AC5 — sur une source fabriquée qui *nomme*
    /// l'interrupteur, le prédicat mord. Sans lui, « ce module ne le nomme
    /// pas » est indistinguable de « le prédicat ne trouve jamais rien ».
    #[test]
    fn mika2922_le_scan_dac5_mord() {
        let fixture = "// write_is_armed en commentaire ne compte pas\n\
                       fn record(&self) {\n    \
                       if write_is_armed() { return; }\n}\n";
        let code = strip_comment_lines(fixture);
        assert!(!code.contains("en commentaire ne compte pas"));
        assert!(
            code.contains("write_is_armed"),
            "le prédicat doit mordre sur un site de production fabriqué"
        );
    }
}

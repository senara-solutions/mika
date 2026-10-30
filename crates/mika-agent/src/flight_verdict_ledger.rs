//! Le registre des verdicts de vol (mika#2922, vols 1 à 3).
//!
//! # Ce que ce module est
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
//! **Et il les lit, depuis le vol 3.** [`read_concordance`] met les deux côtés en
//! face, apparie sur `tasks.id` et rend « N vols consécutifs concordants » : c'est
//! la mesure qu'attend le seuil de mika#2698. Le lecteur vit **ici**, dans le même
//! fichier que les écrivains, pour trois raisons structurelles :
//!
//! 1. la garde de l'adaptateur CLI refuse les quatre aiguilles — les deux
//!    littéraux et les deux constantes — n'importe où sous `crates/mika-cli/src/`,
//!    donc un lecteur côté CLI devrait recomposer les `tool_name` dans le crate
//!    qui n'a pas le droit de les nommer ;
//! 2. `source_scan::production_files()` est enraciné sur `crates/mika-agent/src`,
//!    donc un lecteur hors de ce crate serait **invisible** à sa garde de lecteur
//!    unique — une garde verte par construction, classe mika#2205 ;
//! 3. ce fichier est un **voisin** de `src/milestone_manager/`, pas un de ses
//!    membres, donc le grep de `no_dispatch_test.rs` ne le balaie pas : AC5 tient
//!    sans exception.
//!
//! Les deux constantes déclarées ici sont donc écrites **et** lues par ce module,
//! et chacune porte ses deux gardes : écrivain unique (AC2, vols 1 et 2) et
//! lecteur unique (vol 3).
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

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::evidence::audit::AUDIT_RETENTION_DAYS;
use crate::milestone_manager::evaluator::{
    CiKnowledge, DeathCause, FlightIssue, FlightRecommendation, FlightVerdict, NotFlightReason,
};
use crate::milestone_manager::reporter::{flight_issue_label, flight_recommendation_label};

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
/// **SOLE READER** (vol 3) : [`read_concordance`], dans ce fichier.
/// `mika2922_ac3_un_seul_lecteur_du_registre` en fait la même assertion
/// d'ensemble exact, et pour la même raison : une seconde lecture mettrait en
/// face du jugement de MPC une population que personne ne saurait reconstituer.
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
/// **SOLE READER** (vol 3) : [`read_concordance`], dans ce fichier — le même
/// lecteur que l'autre côté, parce que c'est l'appariement des deux qui est la
/// mesure. `mika2922_ac3_un_seul_lecteur_du_registre` couvre les deux
/// constantes.
pub const MPC_FLIGHT_JUDGMENT_TOOL: &str = "mpc_flight_judgment";

/// `session_id` porté par chaque ligne de jugement.
///
/// Même raisonnement que [`MANAGER_FLIGHT_VERDICT_SESSION_ID`] : un jugement
/// d'opérateur sur un vol n'appartient à aucune conversation, et une constante
/// bien connue le dit au lieu d'emprunter une session sans rapport.
pub const MPC_FLIGHT_JUDGMENT_SESSION_ID: &str = "mpc-flight-judgment";

/// Le préfixe que `reasoning` porte **côté manager** quand le vol a un ticket
/// lisible (`issue_number=<n>`).
///
/// Un seul site pour l'écrivain ([`write_and_swallow`]) et pour le lecteur
/// ([`ticket_du_manager`]). Il était un littéral inline tant que personne ne
/// lisait ; un second littéral recopié dans le lecteur serait vert le jour où
/// l'écrivain change de graphie, et le ticket de chaque vol deviendrait absent
/// en silence — exactement la divergence programmée de mika#2158.
///
/// **Côté MPC, `reasoning` ne porte pas cette donnée** : c'est la note libre de
/// l'opérateur. Le ticket d'un vol se lit donc sur la ligne du manager, jamais
/// sur celle de MPC.
const ISSUE_NUMBER_PREFIX: &str = "issue_number=";

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
    /// Ce que le manager savait de la CI quand il a écrit cette ligne
    /// (mika#2968).
    ///
    /// La ligne reste fidèle, donc elle le porte. La **comparaison** ne le voit
    /// pas : [`VerdictRecord::terms`] ne le projette pas, et c'est délibéré.
    /// MPC juge à la main et n'écrit aucun état CI ; si la CI entrait dans les
    /// termes, chaque vol divergerait sur un terme que l'un des deux côtés ne
    /// produit jamais, et la divergence du vol 4f1fe163 — qui est une
    /// divergence de *recommandation* — serait noyée (AC4).
    ///
    /// Le défaut est celui de l'évaluateur, appelé et non recopié : une ligne
    /// écrite avant ce champ n'a porté aucune lecture, et dire `Read` à sa
    /// place serait inventer une mesure (motif mika#2331).
    #[serde(default = "CiKnowledge::default_unread")]
    pub ci: CiKnowledge,
}

impl From<&FlightVerdict> for VerdictRecord {
    fn from(v: &FlightVerdict) -> Self {
        Self {
            issue: v.issue.clone(),
            recommendation: v.recommendation,
            ci: v.ci.clone(),
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
                FlightIssue::PrOpened { .. }
                | FlightIssue::RescuePr { .. }
                // mika#2946 : le motif de non-vol est porté par le terme
                // d'issue lui-même (`not_a_flight:<motif>`), jamais par
                // `cause` — `--cause` garde son contrat, qui est de ne
                // s'appliquer qu'à une mort.
                | FlightIssue::NotAFlight { .. } => None,
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
        // mika#2946 — terme **composé**, sur le précédent de `unknown:<jeton>`
        // de [`cause_term`] : le motif voyage dans le terme, donc un
        // `not_a_flight:groom_completed` du manager et un
        // `not_a_flight:superseded` de MPC divergent, au lieu de concorder sur
        // un « pas un vol » qui aurait fondu trois faits en un (mika#2205).
        FlightIssue::NotAFlight { reason } => match reason {
            NotFlightReason::GroomCompleted => "not_a_flight:groom_completed",
            NotFlightReason::DispatchRefused => "not_a_flight:dispatch_refused",
            NotFlightReason::Superseded => "not_a_flight:superseded",
        },
    }
}

/// L'étiquette des termes composés de non-vol — `not_a_flight`, qui est aussi la
/// clé serde de la variante.
///
/// **Dérivée, pas réécrite** : un quatrième littéral porterait l'étiquette une
/// fois de plus qu'il n'en faut, et c'est exactement par là qu'une graphie
/// divergerait en silence (mika#2158). `str::split` sur un `&'static str` rend
/// du `&'static str`, donc la dérivation est gratuite.
fn not_a_flight_tag() -> &'static str {
    issue_term(&FlightIssue::NotAFlight {
        reason: NotFlightReason::GroomCompleted,
    })
    .split(':')
    .next()
    .expect("un terme composé porte toujours son étiquette")
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
        FlightRecommendation::Nothing => "nothing",
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
    let reasoning = verdict
        .issue_number
        .map(|n| format!("{ISSUE_NUMBER_PREFIX}{n}"));
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
        // mika#2946 AC3 : les trois motifs de non-vol sont saisissables, donc
        // MPC a un terme pour contredire chacune des huit lignes que le manager
        // comptait en mort. Les variantes sont **instanciées**, pas listées :
        // un quatrième motif ajouté à `NotFlightReason` sans ligne ici reste
        // injugeable, et c'est le compilateur qui refusera de le laisser muet le
        // jour où `issue_term` cessera d'être exhaustif.
        FlightIssue::NotAFlight {
            reason: NotFlightReason::GroomCompleted,
        },
        FlightIssue::NotAFlight {
            reason: NotFlightReason::DispatchRefused,
        },
        FlightIssue::NotAFlight {
            reason: NotFlightReason::Superseded,
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
        // mika#2946 : juger « pas un vol » sans pouvoir dire « rien à
        // recommander » obligerait MPC à recommander un geste sur une tâche qui
        // n'en appelle aucun.
        FlightRecommendation::Nothing,
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
        // MPC juge à la main et ne lit aucun `statusCheckRollup` : sa ligne ne
        // porte donc aucune lecture CI (mika#2968). Aucun terme ne l'expose à
        // la ligne de commande — ce serait demander à l'opérateur d'attester
        // une mesure qu'il n'a pas faite.
        ci: CiKnowledge::default_unread(),
    })
}

fn parse_issue_term(issue: &str, cause: Option<&str>) -> Result<FlightIssue> {
    // `tag` est la clé serde, `payload` la charge. Les deux ne coïncident que
    // pour les termes **simples** : depuis mika#2946 un terme de non-vol est
    // composé (`not_a_flight:<motif>`), et c'est son étiquette qui est la clé.
    let (tag, payload) = if issue == dead_issue_term() {
        let cause = cause.ok_or_else(|| {
            anyhow!(
                "--cause est exigée avec --issue {} ; valeurs acceptées : {}",
                dead_issue_term(),
                judgeable_cause_terms().join(" · ")
            )
        })?;
        (issue, json!({ "cause": parse_cause_term(cause)? }))
    } else {
        // Le refus de `--cause` reste **entier** : cette branche est celle de
        // tout ce qui n'est pas une mort, les termes composés compris.
        if cause.is_some() {
            bail!(
                "--cause ne s'applique qu'à --issue {} ; reçue avec « {issue} »",
                dead_issue_term()
            );
        }
        match issue.split_once(':') {
            // Le motif voyage tel quel dans la charge : une graphie hors
            // contrat est refusée par serde en aval, jamais par une liste
            // tenue ici (doctrine mika#2201).
            Some((tag, reason)) if tag == not_a_flight_tag() => (tag, json!({ "reason": reason })),
            _ => (issue, json!({ "number": null, "url": null })),
        }
    };

    // La forme externally tagged que serde attend : `{"<terme>": <charge>}`. La
    // clé est le terme reçu tel quel, donc un terme inconnu échoue ici — c'est
    // serde qui refuse, pas une liste tenue à la main.
    let mut tagged = Map::new();
    tagged.insert(tag.to_string(), payload);
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

// ---------------------------------------------------------------------------
// Le compte de concordance (vol 3) : la LECTURE unique des deux registres.
// ---------------------------------------------------------------------------

/// Les trois champs sur lesquels un désaccord se nomme, dans l'ordre fixe que
/// la doc de [`VerdictTerms`] énonce : le **premier** qui diverge est celui que
/// le rapport nomme.
///
/// Les graphies sont celles des champs de [`VerdictTerms`] **et** celles des
/// drapeaux de `mika flight-verdicts judge` (`--issue`, `--cause`,
/// `--recommendation`) : un opérateur qui lit « divergent (cause) » sait
/// immédiatement quel drapeau il relirait. Une quatrième graphie — française,
/// abrégée, ou propre au rendu — ferait de ce rapport une table de
/// correspondance de plus (classe mika#2158).
const FIELD_ISSUE: &str = "issue";
const FIELD_CAUSE: &str = "cause";
const FIELD_RECOMMENDATION: &str = "recommendation";

/// Les deux côtés, nommés, pour qu'une ligne illisible dise **laquelle**.
const SIDE_MANAGER: &str = "manager";
const SIDE_MPC: &str = "mpc";

/// Ce que le compte dit d'un vol.
///
/// Quatre états et non deux : « non jugé par MPC » et « illisible » ne sont ni
/// l'un ni l'autre un désaccord, et les replier sur `Divergent` ferait compter
/// des désaccords que personne n'a exprimés (AC4, et motif mika#2331 — absent
/// n'est jamais zéro).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "accord")]
pub enum FlightAgreement {
    /// Les trois termes coïncident.
    Concordant,
    /// Ils diffèrent. `field` est le **premier** des trois dans l'ordre
    /// `issue` → `cause` → `recommendation`.
    Divergent { field: &'static str },
    /// MPC n'a pas jugé ce vol : ne compte ni pour ni contre (AC4).
    NotJudgedByMpc,
    /// Une des deux lignes existe mais ne se re-type pas. Dite comme telle,
    /// jamais avalée et jamais rendue pour un désaccord.
    Unreadable { side: &'static str },
}

/// Un vol, et ce que les deux registres en disent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConcordanceRow {
    /// `tasks.id` — la même clé des deux côtés.
    pub task_id: String,
    /// Le ticket du vol, lu sur la ligne **du manager** (cf.
    /// [`ISSUE_NUMBER_PREFIX`]). `None` quand `reasoning` est absent ou sans
    /// préfixe : un ticket absent n'est pas le ticket `#0` (motif mika#2331).
    pub issue_number: Option<u64>,
    /// Le verdict du manager, `None` quand sa ligne ne se re-type pas.
    pub manager: Option<VerdictRecord>,
    /// Le jugement de MPC, `None` quand il n'y en a pas **ou** qu'il est
    /// illisible — [`Self::agreement`] distingue les deux.
    pub mpc: Option<VerdictRecord>,
    pub agreement: FlightAgreement,
    /// `created_at` de la ligne du manager : l'instant qui ordonne le rapport.
    pub created_at: String,
}

/// Le compte de concordance sur la fenêtre de rétention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConcordanceReport {
    /// Les vols, du plus **récent** au plus ancien — l'ordre dans lequel le
    /// compte consécutif se lit.
    pub rows: Vec<ConcordanceRow>,
    /// « N vols consécutifs concordants », compté depuis le plus récent.
    pub consecutive_concordant: usize,
    /// La largeur de la fenêtre, en jours. **Dérivée** d'[`AUDIT_RETENTION_DAYS`]
    /// et jamais écrite ici : une fenêtre qui survivrait aux lignes qu'elle lit
    /// annoncerait un corpus qui n'existe plus.
    pub window_days: u32,
    /// Les lignes de MPC sans ligne de manager sous la même clé.
    ///
    /// **Dites, jamais jetées.** Un jugement sur un vol dont le verdict du
    /// manager est hors fenêtre (ou n'a jamais été inscrit) est un fait, et le
    /// taire ferait lire « rien à signaler » là où un côté a parlé seul.
    pub orphan_mpc: usize,
}

/// Une ligne du registre du manager, re-typée.
struct ManagerLine {
    task_id: String,
    issue_number: Option<u64>,
    record: Option<VerdictRecord>,
    created_at: String,
}

/// **Le lecteur unique des deux registres de verdicts de vol** (mika#2922,
/// AC3 et AC4).
///
/// Apparie les deux colonnes sur `tasks.id`, compare par
/// [`VerdictRecord::terms`] et compte les vols concordants consécutifs **du
/// plus récent au plus ancien**.
///
/// # Pourquoi la signature ne prend aucun `tool_name`
///
/// Les deux noms d'outil sont les constantes de ce module, et c'est
/// structurel : la garde côté CLI
/// (`commands::flight_verdicts::tests::mika2922_ladaptateur_nest_pas_un_ecrivain`)
/// refuse dans `mika-cli` les quatre aiguilles — les deux littéraux et les deux
/// constantes. Un paramètre `tool_name` obligerait son appelant à les nommer, et
/// l'adaptateur ne pourrait plus appeler cette fonction. C'est le motif de
/// mika#2267 : le lecteur ne recompose pas la clé du scribe, sinon il regarde où
/// l'autre n'écrit pas.
///
/// # L'appariement, et de quel côté le ticket se lit
///
/// La population du **manager** est la référence : c'est elle qui porte le
/// ticket, dans `reasoning` sous [`ISSUE_NUMBER_PREFIX`]. Côté MPC `reasoning`
/// porte une note libre, donc y chercher un ticket rendrait `None` sur une note
/// non vide — une absence plus trompeuse qu'un silence. Une ligne de MPC sans
/// ligne de manager est un **orphelin** : compté, dit en tête, jamais jeté.
///
/// # Ce qui arrête le compte et ce qui le saute
///
/// En remontant du plus récent : `Concordant` incrémente,
/// [`FlightAgreement::NotJudgedByMpc`] **saute** (ni n'incrémente ni n'arrête —
/// AC4 mot pour mot), `Divergent` et `Unreadable` **arrêtent**. D'où le
/// comportement qu'AC3 exige : trois vols dont deux concordants et un divergent
/// sur la cause rendent **2** si le divergent est le plus ancien, **0** s'il est
/// le plus récent.
///
/// Une ligne illisible arrête plutôt que de sauter : « je ne sais pas » ne
/// saurait valoir « ils étaient d'accord », et c'est un compte d'accord qui
/// ouvre un interrupteur.
pub async fn read_concordance(db: &AsyncDatabase) -> Result<ConcordanceReport> {
    let since = inscription_window_start();

    let manager_rows = db
        .list_audit_events_for_tool_since(MANAGER_FLIGHT_VERDICT_TOOL, &since)
        .await
        .with_context(|| format!("lecture des verdicts du manager depuis {since}"))?;
    let mpc_rows = db
        .list_audit_events_for_tool_since(MPC_FLIGHT_JUDGMENT_TOOL, &since)
        .await
        .with_context(|| format!("lecture des jugements de MPC depuis {since}"))?;

    // La **présence** de la clé dit « MPC a jugé » ; la valeur `None` dit « sa
    // ligne ne se re-type pas ». Les replier sur un seul `Option` rendrait un
    // jugement illisible indistinguable d'un vol non jugé.
    let mut mpc: HashMap<&str, Option<VerdictRecord>> = HashMap::new();
    for (task_id, after_value, _note, _created_at) in &mpc_rows {
        mpc.insert(
            task_id.as_str(),
            reread_record(task_id, SIDE_MPC, after_value.as_deref()),
        );
    }

    // Les lignes du manager, dans l'ordre d'inscription. AC1 garantit une seule
    // ligne par vol dans la fenêtre ; si deux s'y trouvaient malgré tout, la
    // plus récente fait foi — comme partout ailleurs dans ce module.
    let mut manager: Vec<ManagerLine> = Vec::new();
    for (task_id, after_value, reasoning, created_at) in &manager_rows {
        let line = ManagerLine {
            task_id: task_id.clone(),
            issue_number: ticket_du_manager(reasoning.as_deref()),
            record: reread_record(task_id, SIDE_MANAGER, after_value.as_deref()),
            created_at: created_at.clone(),
        };
        match manager.iter_mut().find(|l| l.task_id == line.task_id) {
            Some(place) => *place = line,
            None => manager.push(line),
        }
    }

    let orphan_mpc = mpc
        .keys()
        .filter(|key| !manager.iter().any(|l| l.task_id == **key))
        .count();

    // Du plus récent au plus ancien : `ORDER BY id` est croissant, et c'est cet
    // ordre-là qu'on inverse — jamais un tri sur `created_at`, qui est à la
    // seconde et met deux verdicts d'un même cycle à égalité.
    let rows: Vec<ConcordanceRow> = manager
        .into_iter()
        .rev()
        .map(|line| {
            let judged = mpc.get(line.task_id.as_str());
            let agreement = match (&line.record, judged) {
                (None, _) => FlightAgreement::Unreadable { side: SIDE_MANAGER },
                (Some(_), None) => FlightAgreement::NotJudgedByMpc,
                (Some(_), Some(None)) => FlightAgreement::Unreadable { side: SIDE_MPC },
                (Some(verdict), Some(Some(judgment))) => {
                    first_divergence(&verdict.terms(), &judgment.terms())
                }
            };
            ConcordanceRow {
                task_id: line.task_id,
                issue_number: line.issue_number,
                manager: line.record,
                mpc: judged.and_then(|held| held.clone()),
                agreement,
                created_at: line.created_at,
            }
        })
        .collect();

    Ok(ConcordanceReport {
        consecutive_concordant: consecutive_concordant(&rows),
        window_days: AUDIT_RETENTION_DAYS,
        orphan_mpc,
        rows,
    })
}

/// Le compte consécutif, en remontant du plus récent. Extrait du corps de
/// [`read_concordance`] pour que la règle d'AC3/AC4 soit testable sur une suite
/// d'accords, sans base.
fn consecutive_concordant(rows: &[ConcordanceRow]) -> usize {
    let mut compte = 0usize;
    for row in rows {
        match row.agreement {
            FlightAgreement::Concordant => compte += 1,
            // AC4 mot pour mot : ne compte ni pour ni contre.
            FlightAgreement::NotJudgedByMpc => continue,
            FlightAgreement::Divergent { .. } | FlightAgreement::Unreadable { .. } => break,
        }
    }
    compte
}

/// Le **premier** champ qui diverge, dans l'ordre fixe de [`VerdictTerms`].
///
/// La comparaison porte sur la projection, jamais sur l'égalité des lignes :
/// `FlightIssue::PrOpened` porte un `number` et une `url` que le manager
/// remplit et que MPC écrit `null`, donc l'égalité des lignes rendrait
/// « divergent » sur un accord parfait — voir le § *Pourquoi une projection* de
/// [`VerdictTerms`].
fn first_divergence(manager: &VerdictTerms, mpc: &VerdictTerms) -> FlightAgreement {
    if manager.issue != mpc.issue {
        return FlightAgreement::Divergent { field: FIELD_ISSUE };
    }
    if manager.cause != mpc.cause {
        return FlightAgreement::Divergent { field: FIELD_CAUSE };
    }
    if manager.recommendation != mpc.recommendation {
        return FlightAgreement::Divergent {
            field: FIELD_RECOMMENDATION,
        };
    }
    FlightAgreement::Concordant
}

/// Le ticket porté par le `reasoning` d'une ligne **du manager**.
///
/// `None` sur un `reasoning` absent, sans le préfixe, ou dont la suite n'est pas
/// un nombre. Dans les trois cas le vol n'a **pas** de ticket lisible, et c'est
/// ce que le rendu dit — jamais `#0` (motif mika#2331).
fn ticket_du_manager(reasoning: Option<&str>) -> Option<u64> {
    reasoning?
        .strip_prefix(ISSUE_NUMBER_PREFIX)?
        .parse::<u64>()
        .ok()
}

/// Re-typer un `after_value`, en nommant la perte plutôt qu'en l'avalant.
///
/// `None` pour une colonne NULL comme pour un JSON illisible : les deux disent
/// « cette ligne ne porte pas de verdict exploitable ». L'appelant sait de quel
/// côté il est, et [`FlightAgreement::Unreadable`] le nomme dans le rapport —
/// la ligne apparaît donc comme illisible, jamais comme un désaccord.
fn reread_record(task_id: &str, side: &str, after_value: Option<&str>) -> Option<VerdictRecord> {
    let after_value = after_value?;
    match serde_json::from_str(after_value) {
        Ok(record) => Some(record),
        Err(e) => {
            warn_concordance_unreadable(task_id, side, &e.to_string());
            None
        }
    }
}

/// WARN, et non ERROR : le rapport est rendu, et il **dit** que cette ligne est
/// illisible. Même idiome que [`warn_held_unreadable`] — un seul nom
/// d'événement, parce que le regroupement se fait dessus, et `side` nomme lequel
/// des deux registres a la ligne fautive.
fn warn_concordance_unreadable(task_id: &str, side: &str, error: &str) {
    warn!(
        target: "mika::flight_verdict_ledger",
        event = "flight_concordance_row_unreadable",
        task_id = %task_id,
        side = side,
        error = error,
        "une ligne du registre ne se re-type pas — le vol est rendu illisible, jamais concordant"
    );
}

// ---------------------------------------------------------------------------
// Le rendu du compte (vol 3) : ce que l'opérateur lit.
// ---------------------------------------------------------------------------

/// Les en-têtes des cinq colonnes. La **sixième** donnée qu'énonce le ticket —
/// le champ divergent — est portée **dans** la colonne « accord », sous la forme
/// `divergent (cause)`, exactement comme la maquette du plan l'écrit : un champ
/// divergent n'existe que pour un désaccord, donc une sixième colonne serait
/// vide sur toutes les autres lignes. Le rendu `json`/`yaml` les sépare
/// (`accord` et `field`), et c'est là que la machine les lit.
const COLUMNS: [&str; 5] = ["vol", "ticket", "manager", "MPC", "accord"];

/// Ce qu'une cellule porte quand la donnée est **absente** : pas de ticket
/// lisible, ou pas de jugement du tout. Jamais `0`, jamais `#0`, jamais une
/// chaîne vide qu'on lirait pour un bug d'alignement (motif mika#2331).
const ABSENT: &str = "—";

/// Le compte de concordance, en texte, pour un opérateur.
///
/// # Deux choix de forme, parce qu'ils se re-devineraient mal
///
/// 1. **La clé est rendue entière.** La maquette du plan montre huit caractères
///    (`a1b2c3d4`) ; en production `tasks.id` est un UUID, et l'étape 5 de la
///    sonde d'AC6 demande à l'opérateur de recopier cette clé dans
///    `mika flight-verdicts judge --task-id <…>`. Une clé tronquée rendrait le
///    geste suivant impossible : on ne tronque pas ce qu'on demande de coller.
/// 2. **Les largeurs se calculent sur le contenu**, sans ellipse. La maquette
///    coupe à trente colonnes (`mort au plafond (budget_ex…)`) ; mais une
///    cellule tronquée efface précisément ce que la colonne « accord » vient de
///    nommer — un `divergent (cause)` dont les deux causes sont illisibles est
///    un instrument qui montre un désaccord sans le montrer. La ligne est plus
///    large ; elle est entière.
///
/// Le vocabulaire français vient de [`flight_issue_label`] et
/// [`flight_recommendation_label`], promus `pub(crate)` par ce vol : un second
/// vocabulaire pour une seule grammaire serait la divergence programmée de
/// mika#2158.
pub fn render_concordance_text(report: &ConcordanceReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "Concordance manager / MPC — {} vol{}, fenêtre de {} jours\n",
        report.rows.len(),
        pluriel(report.rows.len()),
        report.window_days
    ));
    out.push_str(&format!(
        "{} vol{} consécutif{} concordant{} (du plus récent au plus ancien ; seuil mika#2698 : 10 vols ou 7 jours)\n",
        report.consecutive_concordant,
        pluriel(report.consecutive_concordant),
        pluriel(report.consecutive_concordant),
        pluriel(report.consecutive_concordant)
    ));
    // Les orphelins sont **dits**, pas jetés : une ligne de MPC sans verdict du
    // manager sous la même clé n'a pas de place dans le tableau (l'appariement
    // se fait sur la population du manager), mais la taire ferait d'un registre
    // désaligné un registre sain.
    if report.orphan_mpc > 0 {
        out.push_str(&format!(
            "{} jugement{} de MPC sans verdict du manager sous la même clé — hors du tableau, et comptés ici\n",
            report.orphan_mpc,
            pluriel(report.orphan_mpc)
        ));
    }

    if report.rows.is_empty() {
        out.push_str("\nAucun vol inscrit dans la fenêtre.\n");
        return out;
    }

    let cells: Vec<[String; 5]> = report.rows.iter().map(row_cells).collect();
    let mut widths = COLUMNS.map(str::len);
    for row in &cells {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }

    out.push('\n');
    out.push_str(&ligne_alignee(&COLUMNS.map(String::from), &widths));
    for row in &cells {
        out.push_str(&ligne_alignee(row, &widths));
    }
    out
}

/// Les cinq cellules d'un vol.
fn row_cells(row: &ConcordanceRow) -> [String; 5] {
    [
        row.task_id.clone(),
        match row.issue_number {
            Some(n) => format!("#{n}"),
            None => ABSENT.to_string(),
        },
        cote(row.manager.as_ref(), row.agreement, SIDE_MANAGER),
        cote(row.mpc.as_ref(), row.agreement, SIDE_MPC),
        accord_label(row.agreement),
    ]
}

/// Un côté du registre : son verdict, ou la raison de son absence.
///
/// « Illisible » et « absent » sont deux cellules distinctes parce qu'elles sont
/// deux faits distincts (classe mika#2205) : la première dit qu'une ligne existe
/// et ne se relit pas — donc qu'il y a quelque chose à réparer — la seconde
/// qu'il n'y a rien à lire.
fn cote(record: Option<&VerdictRecord>, agreement: FlightAgreement, side: &'static str) -> String {
    match record {
        Some(record) => format!(
            "{} · {}",
            flight_issue_label(&record.issue),
            flight_recommendation_label(record.recommendation)
        ),
        None => match agreement {
            FlightAgreement::Unreadable { side: fautif } if fautif == side => {
                "illisible".to_string()
            }
            _ => ABSENT.to_string(),
        },
    }
}

/// La colonne « accord », qui porte aussi le champ divergent (cf. [`COLUMNS`]).
fn accord_label(agreement: FlightAgreement) -> String {
    match agreement {
        FlightAgreement::Concordant => "concordant".to_string(),
        FlightAgreement::Divergent { field } => format!("divergent ({field})"),
        FlightAgreement::NotJudgedByMpc => "non jugé par MPC".to_string(),
        FlightAgreement::Unreadable { side } => format!("illisible ({side})"),
    }
}

/// Une ligne du tableau, les quatre premières colonnes complétées à leur
/// largeur. Le comptage est en `chars()` et non en octets : « mort au plafond »
/// porte des accents, et un `len()` décalerait tout le tableau.
fn ligne_alignee(cells: &[String; 5], widths: &[usize; 5]) -> String {
    let mut ligne = String::new();
    for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if i + 1 == cells.len() {
            ligne.push_str(cell);
        } else {
            ligne.push_str(cell);
            for _ in cell.chars().count()..*width + 2 {
                ligne.push(' ');
            }
        }
    }
    ligne.push('\n');
    ligne
}

/// Le `s` du pluriel français : absent à zéro **et** à un.
fn pluriel(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
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
    use crate::milestone_manager::evaluator::CiUnreadCause;
    use crate::milestone_manager::types::CiState;
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
            in_target_milestone: crate::milestone_manager::MilestoneMembership::NotRead,
            ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
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
            in_target_milestone: crate::milestone_manager::MilestoneMembership::NotRead,
            ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
            completed_at: None,
        }
    }

    /// Un verdict de **non-vol** (mika#2946) — la forme dont le terme d'issue
    /// est composé et dont la recommandation est `Nothing`.
    fn verdict_non_vol(task_id: &str, reason: NotFlightReason) -> FlightVerdict {
        FlightVerdict {
            task_id: task_id.to_string(),
            repo: None,
            issue_number: Some(2946),
            issue: FlightIssue::NotAFlight { reason },
            turns_used: None,
            hit_ceiling: false,
            turn_ceiling: None,
            last_commit_turn: None,
            recommendation: FlightRecommendation::Nothing,
            pr_state: crate::milestone_manager::evaluator::PrStateKnowledge::NotRead,
            in_target_milestone: crate::milestone_manager::MilestoneMembership::NotRead,
            ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
            completed_at: Some("2026-10-28T12:10:00Z".to_string()),
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

        // mika#2946 — un terme **composé** : la clé serde est l'étiquette nue,
        // le terme porte l'étiquette **et** le motif. La boucle ci-dessus ne
        // peut pas le vérifier (elle compare le terme à la clé), donc la
        // graphie des deux moitiés s'assère ici, comme celle d'`unknown:abc`.
        for (reason, attendu) in [
            (NotFlightReason::GroomCompleted, "groom_completed"),
            (NotFlightReason::DispatchRefused, "dispatch_refused"),
            (NotFlightReason::Superseded, "superseded"),
        ] {
            let record = VerdictRecord::from(&verdict_non_vol("t", reason));
            let json = serde_json::to_string(&record).unwrap();
            let back: VerdictRecord = serde_json::from_str(&json).unwrap();
            assert_eq!(back, record, "aller-retour serde : {json}");

            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            let map = value["issue"].as_object().expect("variante nommée");
            let cle = map.keys().next().unwrap();
            assert_eq!(cle, not_a_flight_tag(), "l'étiquette, dans {json}");
            // Le motif est la graphie serde de la variante, pas un libellé
            // retapé — c'est ce que `rename_all` produit réellement.
            assert_eq!(
                map[cle]["reason"].as_str().unwrap(),
                attendu,
                "le motif, dans {json}"
            );
            assert_eq!(
                record.terms().issue,
                format!("{}:{attendu}", not_a_flight_tag()),
                "le terme composé, dans {json}"
            );
            // Un non-vol ne porte pas de cause : `--cause` reste le vocabulaire
            // de la mort seule.
            assert_eq!(record.terms().cause, None);
            assert_eq!(
                record.terms().recommendation,
                value["recommendation"].as_str().unwrap()
            );
        }
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

    // ---- le jugement de MPC (vol 2) ---------------------------------------

    /// **AC2** (seconde moitié, « jugement de MPC »). Même forme et même raison
    /// que la moitié du manager : ensemble exact, auto-nettoyant, sans
    /// allowlist. Quand il grandit, la résolution est de router l'écriture par
    /// [`record_mpc_judgment`] (doctrine mika#2201).
    ///
    /// **Ce que ce scan ne voit pas, dit ici plutôt que supposé.**
    /// [`production_files`] est enraciné sur le `src` de *ce* crate, donc un
    /// écrivain côté `mika-cli` lui serait invisible. C'est précisément pourquoi
    /// l'écrivain vit ici et non dans l'adaptateur : une garde enracinée
    /// ailleurs serait verte par construction (classe mika#2205). Le côté CLI
    /// porte sa propre garde, dans son propre crate.
    #[test]
    fn mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc() {
        let files = production_files();

        // Anti-vacuité : le scan a bien lu l'arbre (mika#2205).
        assert!(
            files.len() >= 100,
            "anti-vacuité : {} fichiers lus, le scan n'a rien lu",
            files.len()
        );

        let writers = audit_writers("MPC_FLIGHT_JUDGMENT_TOOL");
        assert_eq!(
            writers,
            vec!["flight_verdict_ledger.rs".to_string()],
            "SOLE WRITER : un seul écrivain. Quand cet ensemble grandit, route \
             l'écriture par record_mpc_judgment — n'ajoute pas d'entrée"
        );

        let literal = String::from("\"") + MPC_FLIGHT_JUDGMENT_TOOL + "\"";
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

    /// Contrôle négatif du scan du jugement. Le site étranger est ici le
    /// **verdict du manager** et non un `tool_name` lointain : les deux
    /// écrivains vivent dans le même fichier, donc la confusion réaliste est
    /// qu'une fenêtre morde sur le voisin et fasse passer deux écrivains pour
    /// un. Sans ce contrôle, « un seul écrivain » est indistinguable de « la
    /// fenêtre ne mord sur rien ».
    #[test]
    fn mika2922_le_scan_du_jugement_mord() {
        let mord = |source: &str| {
            let lines: Vec<&str> = source.lines().collect();
            lines.iter().enumerate().any(|(i, l)| {
                l.contains("log_audit_event(")
                    && lines[i..(i + 5).min(lines.len())]
                        .iter()
                        .any(|w| w.contains("MPC_FLIGHT_JUDGMENT_TOOL"))
            })
        };

        let fixture = "db.log_audit_event(\n    MPC_FLIGHT_JUDGMENT_SESSION_ID,\n    \
                       MPC_FLIGHT_JUDGMENT_TOOL,\n";
        assert!(
            mord(fixture),
            "la fenêtre de 5 lignes doit voir le site fabriqué"
        );

        let voisin = "db.log_audit_event(\n    MANAGER_FLIGHT_VERDICT_SESSION_ID,\n    \
                      MANAGER_FLIGHT_VERDICT_TOOL,\n";
        assert!(
            !mord(voisin),
            "la fenêtre ne doit pas compter l'écrivain du verdict du manager \
             comme un écrivain du jugement de MPC"
        );
    }

    /// **L'analyse des termes est l'inverse exact de serde.** C'est la seule
    /// propriété qui empêche la CLI de devenir une seconde table de graphies
    /// (classe mika#2158) : tout jugement saisissable s'analyse, et sa
    /// projection rend les termes mêmes qu'on a saisis.
    ///
    /// Le compte final est l'anti-vacuité de la boucle : sans lui, une
    /// énumération vide passerait le test sans rien analyser.
    #[test]
    fn mika2922_lanalyse_des_termes_est_linverse_de_serde() {
        let mut analyses = 0usize;
        for issue in judgeable_issue_terms() {
            let causes: Vec<Option<String>> = if issue == dead_issue_term() {
                judgeable_cause_terms().into_iter().map(Some).collect()
            } else {
                vec![None]
            };
            for cause in &causes {
                for recommandation in judgeable_recommendation_terms() {
                    let record = parse_judgment_terms(issue, cause.as_deref(), recommandation)
                        .unwrap_or_else(|e| {
                            panic!("{issue}/{cause:?}/{recommandation} refusé : {e}")
                        });

                    let terms = record.terms();
                    assert_eq!(terms.issue, issue);
                    assert_eq!(&terms.cause, cause);
                    assert_eq!(terms.recommendation, recommandation);

                    // Et la ligne qui sera écrite se re-type : le lecteur du
                    // vol 3 relira ce que ce vol inscrit.
                    let json = serde_json::to_string(&record).unwrap();
                    let back: VerdictRecord = serde_json::from_str(&json).unwrap();
                    assert_eq!(back, record, "aller-retour serde : {json}");

                    analyses += 1;
                }
            }
        }
        assert_eq!(
            analyses, 40,
            "6 issues — `pr_opened`, `rescue_pr`, `dead` et les trois motifs de \
             non-vol de mika#2946 — dont la seule `dead` porte 5 causes, × 4 \
             recommandations : (5 × 4) + (5 × 1 × 4) = 40 jugements saisissables ; \
             un autre compte veut dire que l'énumération a bougé"
        );

        // MPC juge un vol, pas une PR : les deux champs sont `null` et non 0 ni
        // chaîne vide (motif mika#2331). C'est pourquoi la comparaison du vol 3
        // passe par la projection et non par l'égalité des lignes.
        assert_eq!(
            parse_judgment_terms("pr_opened", None, "merge_candidate")
                .unwrap()
                .issue,
            FlightIssue::PrOpened {
                number: None,
                url: None
            }
        );
    }

    /// Un terme hors contrat est refusé **en nommant les valeurs acceptées** :
    /// un refus qui n'énumère pas laisse l'opérateur deviner, et c'est là qu'il
    /// retape le terme de travers.
    #[test]
    fn mika2922_un_terme_hors_contrat_est_refuse_en_nommant_les_valeurs() {
        let refus = |issue: &str, cause: Option<&str>, reco: &str| {
            parse_judgment_terms(issue, cause, reco)
                .expect_err("ce jugement doit être refusé")
                .to_string()
        };

        let e = refus("mort", None, "regroom");
        assert!(
            e.contains("mort") && e.contains("dead") && e.contains("pr_opened"),
            "{e}"
        );

        let e = refus("dead", None, "regroom");
        assert!(e.contains("--cause") && e.contains("refusal"), "{e}");

        let e = refus("pr_opened", Some("refusal"), "merge_candidate");
        assert!(e.contains("--cause") && e.contains("dead"), "{e}");

        let e = refus("dead", Some("fatigue"), "regroom");
        assert!(e.contains("fatigue") && e.contains("launcher_dead"), "{e}");

        let e = refus("dead", Some("refusal"), "fusionner");
        assert!(
            e.contains("fusionner") && e.contains("merge_candidate"),
            "{e}"
        );
    }

    /// [`DeathCause::Unknown`] n'est pas offerable, et son refus est
    /// **structurel** : la forme serde d'une variante newtype est un objet,
    /// jamais un terme nu, donc aucune valeur de `--cause` ne peut la produire.
    ///
    /// Les deux derniers termes sont l'anti-vacuité : la variante existe, le
    /// manager l'écrit bel et bien, et l'énumération ne la propose pas. Sans
    /// eux, le refus pourrait porter sur une variante morte.
    #[test]
    fn mika2922_un_jugement_ne_porte_pas_dinconnu() {
        for terme in ["unknown", "unknown:halte_x"] {
            assert!(
                parse_judgment_terms("dead", Some(terme), "regroom").is_err(),
                "« {terme} » ne doit pas être saisissable comme cause"
            );
        }

        assert_eq!(
            VerdictRecord::from(&verdict_mort("t", DeathCause::Unknown("halte_x".into())))
                .terms()
                .cause
                .as_deref(),
            Some("unknown:halte_x"),
            "anti-vacuité : le manager, lui, écrit cette cause"
        );
        assert!(
            !judgeable_cause_terms()
                .iter()
                .any(|t| t.starts_with("unknown")),
            "l'énumération offerte à l'opérateur ne propose pas l'inconnu"
        );
    }

    /// Un vol déjà jugé n'est pas rejugé, et ce qui est **inscrit** est rendu.
    ///
    /// Sur une vraie base, pour la même raison qu'AC1 : la propriété est celle
    /// d'une lecture d'idempotence contre des lignes réellement écrites.
    ///
    /// La dernière moitié est le contrôle qui sépare les deux côtés : le verdict
    /// du manager sur un vol ne vaut pas jugement de MPC sur ce vol. Avec un
    /// `tool_name` partagé, l'idempotence de chaque côté serait celle de
    /// l'autre, et le compte de concordance lirait un accord là où un seul des
    /// deux a parlé.
    #[tokio::test]
    async fn mika2922_un_vol_deja_juge_nest_pas_rejuge_et_rend_ce_qui_est_inscrit() {
        let db = AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "mika");
        let juge = parse_judgment_terms("dead", Some("refusal"), "regroom").unwrap();

        let premier = record_mpc_judgment(&db, "task-aaa", &juge, Some("vu avec Vincent"))
            .await
            .unwrap();
        assert_eq!(premier, MpcJudgmentOutcome::Inscribed);

        let (after, reasoning, _created) = db
            .latest_audit_event_for_target(
                MPC_FLIGHT_JUDGMENT_TOOL,
                "task-aaa",
                "2000-01-01T00:00:00Z",
            )
            .await
            .unwrap()
            .expect("la ligne de task-aaa existe");
        assert_eq!(
            serde_json::from_str::<VerdictRecord>(&after.unwrap()).unwrap(),
            juge,
            "la ligne se re-type en le jugement saisi"
        );
        assert_eq!(
            reasoning.as_deref(),
            Some("vu avec Vincent"),
            "la note libre de l'opérateur va dans reasoning"
        );

        // Un second geste, avec un jugement différent, ne réécrit rien — et rend
        // ce qui est en place, pas ce qu'on vient de proposer.
        let autre = parse_judgment_terms("pr_opened", None, "merge_candidate").unwrap();
        let second = record_mpc_judgment(&db, "task-aaa", &autre, None)
            .await
            .unwrap();
        assert_eq!(
            second,
            MpcJudgmentOutcome::AlreadyRecorded {
                held: Some(juge.clone())
            },
            "l'opérateur doit lire ce qu'il a déjà dit, pas un « déjà là » nu"
        );
        assert_eq!(
            db.get_audit_event_target_keys_by_tool_name(MPC_FLIGHT_JUDGMENT_TOOL)
                .await
                .unwrap(),
            vec!["task-aaa".to_string()],
            "et la base porte toujours une seule ligne de jugement"
        );

        // Les deux côtés sous la même clé, sans se confondre.
        let verdict = verdict_mort("task-bbb", DeathCause::Refusal);
        assert_eq!(
            write_and_swallow(&db, &verdict).await,
            InscriptionTally::one_inscribed()
        );
        assert_eq!(
            record_mpc_judgment(&db, "task-bbb", &juge, None)
                .await
                .unwrap(),
            MpcJudgmentOutcome::Inscribed,
            "le verdict du manager sur ce vol ne vaut pas jugement de MPC"
        );
    }

    /// L'échec **se propage**, contrairement au côté manager — et il nomme le
    /// vol. L'échec est injecté plutôt qu'argumenté : l'`agent_id` n'a pas de
    /// ligne dans `agents`, donc l'INSERT trouve la clé étrangère.
    #[tokio::test]
    async fn mika2922_un_echec_dinscription_du_jugement_se_propage() {
        let db =
            AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "no-such-agent");
        let juge = parse_judgment_terms("dead", Some("refusal"), "regroom").unwrap();

        let err = record_mpc_judgment(&db, "task-fk", &juge, None)
            .await
            .expect_err("une écriture en échec doit remonter, pas être avalée");
        assert!(
            format!("{err:#}").contains("task-fk"),
            "le refus doit nommer le vol : {err:#}"
        );
    }

    // ---- AC3 et AC4, sur une vraie base -----------------------------------

    /// Écrire un vol **des deux côtés**, par les écrivains des vols 1 et 2.
    ///
    /// Jamais par du SQL de fixture, et c'est ce qui fait la valeur de ces
    /// tests : ce que le compte mesure est l'appariement de ce que la
    /// **production** écrit. Une fixture qui poserait ses propres colonnes
    /// rendrait vraie n'importe quelle convention de clé, de `tool_name` ou de
    /// `reasoning` — exactement la propriété que le vol 3 doit tenir.
    ///
    /// `mpc` est le terme de cause que MPC juge, ou `None` pour « MPC n'a pas
    /// jugé ce vol ». Les deux inscriptions sont **assérées** : un corpus qui ne
    /// s'écrit pas ferait lire 0 à un compte correct.
    async fn vol(db: &AsyncDatabase, task_id: &str, manager: DeathCause, mpc: Option<&str>) {
        assert_eq!(
            write_and_swallow(db, &verdict_mort(task_id, manager)).await,
            InscriptionTally::one_inscribed(),
            "le verdict du manager sur {task_id} doit s'inscrire"
        );
        if let Some(cause) = mpc {
            let juge = parse_judgment_terms("dead", Some(cause), "regroom").unwrap();
            assert_eq!(
                record_mpc_judgment(db, task_id, &juge, None).await.unwrap(),
                MpcJudgmentOutcome::Inscribed,
                "le jugement de MPC sur {task_id} doit s'inscrire"
            );
        }
    }

    /// Le pendant mika#2946 de [`vol`] : écrire un **non-vol** des deux côtés,
    /// par les écrivains de production et jamais par du SQL de fixture.
    ///
    /// `mpc` porte les trois termes **tels que l'opérateur les saisirait** —
    /// `--issue`, `--cause`, `--recommendation`. Ils passent donc par
    /// [`parse_judgment_terms`], et son échec est une **panique nommée** plutôt
    /// qu'un `unwrap` : c'est cette moitié qui atteste qu'un terme de non-vol est
    /// saisissable, et un `unwrap` nu dirait « thread panicked » là où le test
    /// doit dire *quel* terme la ligne de commande refuse.
    async fn non_vol(
        db: &AsyncDatabase,
        task_id: &str,
        reason: NotFlightReason,
        mpc: (&str, Option<&str>, &str),
    ) {
        assert_eq!(
            write_and_swallow(db, &verdict_non_vol(task_id, reason)).await,
            InscriptionTally::one_inscribed(),
            "le verdict du manager sur {task_id} doit s'inscrire"
        );
        let (issue, cause, recommendation) = mpc;
        let juge = parse_judgment_terms(issue, cause, recommendation)
            .unwrap_or_else(|e| panic!("MPC doit pouvoir saisir « {issue} » : {e:#}"));
        assert_eq!(
            record_mpc_judgment(db, task_id, &juge, None).await.unwrap(),
            MpcJudgmentOutcome::Inscribed,
            "le jugement de MPC sur {task_id} doit s'inscrire"
        );
    }

    fn base() -> AsyncDatabase {
        AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "mika")
    }

    fn accords(rapport: &ConcordanceReport) -> Vec<(&str, FlightAgreement)> {
        rapport
            .rows
            .iter()
            .map(|r| (r.task_id.as_str(), r.agreement))
            .collect()
    }

    /// **AC3.** Un corpus de 3 vols, dont 2 concordants et 1 divergent sur la
    /// cause de mort, rend « 2 vols consécutifs » ou « 0 » selon l'ordre, et
    /// nomme le champ divergent.
    ///
    /// Le contrôle positif et le contrôle négatif sont dans le même test, parce
    /// que c'est leur écart qui est la propriété : « 2 » tout seul ne distingue
    /// pas un compte qui s'arrête sur le divergent d'un compte qui plafonne, et
    /// « 0 » tout seul ne distingue pas un compte correct d'un compte qui ne
    /// trouve jamais rien.
    ///
    /// Les trois corpus sont le **même** à l'ordre près : seule la position du
    /// vol divergent change. C'est ce qui fait de l'écart 3 → 2 → 0 une mesure
    /// de la règle de comptage et non du corpus.
    #[tokio::test]
    async fn mika2922_ac3_deux_concordants_et_un_divergent_rendent_2_ou_0_selon_lordre() {
        // Contrôle positif : tout concorde, le compte vaut le corpus.
        let db = base();
        vol(
            &db,
            "task-1",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;
        vol(&db, "task-2", DeathCause::Refusal, Some("refusal")).await;
        vol(
            &db,
            "task-3",
            DeathCause::PipelineFailure,
            Some("pipeline_failure"),
        )
        .await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![
                ("task-3", FlightAgreement::Concordant),
                ("task-2", FlightAgreement::Concordant),
                ("task-1", FlightAgreement::Concordant),
            ],
            "du plus récent au plus ancien, et tout concorde"
        );
        assert_eq!(rapport.consecutive_concordant, 3, "contrôle positif");
        assert_eq!(
            rapport.window_days, AUDIT_RETENTION_DAYS,
            "la fenêtre annoncée est celle de la rétention, jamais un littéral"
        );
        assert_eq!(
            rapport.orphan_mpc, 0,
            "chaque jugement a sa ligne de manager"
        );
        assert!(
            rapport.rows.iter().all(|r| r.issue_number == Some(2922)),
            "le ticket se lit sur la ligne du manager : {rapport:?}"
        );

        // Contrôle négatif, le divergent au plus ANCIEN : les deux plus récents
        // concordent, le troisième arrête → 2.
        let db = base();
        vol(&db, "task-1", DeathCause::CeilingReached, Some("refusal")).await;
        vol(&db, "task-2", DeathCause::Refusal, Some("refusal")).await;
        vol(
            &db,
            "task-3",
            DeathCause::PipelineFailure,
            Some("pipeline_failure"),
        )
        .await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![
                ("task-3", FlightAgreement::Concordant),
                ("task-2", FlightAgreement::Concordant),
                ("task-1", FlightAgreement::Divergent { field: "cause" }),
            ],
            "et le champ divergent est nommé"
        );
        assert_eq!(rapport.consecutive_concordant, 2);

        // Le même corpus, le divergent au plus RÉCENT : le compte vaut 0.
        let db = base();
        vol(
            &db,
            "task-1",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;
        vol(&db, "task-2", DeathCause::Refusal, Some("refusal")).await;
        vol(&db, "task-3", DeathCause::PipelineFailure, Some("refusal")).await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![
                ("task-3", FlightAgreement::Divergent { field: "cause" }),
                ("task-2", FlightAgreement::Concordant),
                ("task-1", FlightAgreement::Concordant),
            ],
            "deux vols concordent encore — ils ne sont simplement plus consécutifs"
        );
        assert_eq!(
            rapport.consecutive_concordant, 0,
            "un désaccord sur le vol le plus récent remet le compte à zéro"
        );
    }

    /// **AC3 de mika#2946.** Une ligne de non-vol n'est plus laissée **hors du
    /// compte** : MPC a un terme pour la contredire, et un terme pour l'accorder.
    ///
    /// Les deux moitiés sont dans le même test parce que c'est leur écart qui est
    /// la propriété. « Divergent » seul ne distingue pas un terme que MPC
    /// *saisit* d'un terme que l'analyse refuse — un refus ferait paniquer
    /// [`non_vol`], mais une graphie acceptée *de travers* rendrait aussi un
    /// désaccord, et on le lirait comme un succès. « Concordant » seul ne
    /// distingue pas un appariement correct d'une comparaison qui ne regarde
    /// rien. Avant mika#2946 aucune des deux n'était atteignable : `--issue`
    /// n'acceptait que `pr_opened · rescue_pr · dead`, donc la ligne du manager
    /// comptait comme « non jugée par MPC » (motif de l'AC4 de mika#2922) et
    /// sortait du compte en silence, sans jamais pouvoir peser comme désaccord.
    ///
    /// La seconde moitié porte les **trois** motifs, et pas un seul : le terme
    /// est composé, donc c'est la charge (`reason`) qui voyage, et une charge
    /// perdue en route rendrait les trois lignes concordantes entre elles tout
    /// en les confondant.
    #[tokio::test]
    async fn mika2946_un_non_vol_se_contredit_et_saccorde() {
        // Première moitié : MPC contredit — il tient la tâche pour un vol mort.
        let db = base();
        non_vol(
            &db,
            "task-groom",
            NotFlightReason::GroomCompleted,
            ("dead", Some("refusal"), "regroom"),
        )
        .await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![("task-groom", FlightAgreement::Divergent { field: "issue" })],
            "le désaccord porte sur l'issue — le premier des trois champs"
        );
        assert_eq!(
            rapport.consecutive_concordant, 0,
            "une ligne de non-vol que MPC contredit pèse, au lieu d'être sautée"
        );
        assert_eq!(
            rapport.orphan_mpc, 0,
            "le jugement a trouvé sa ligne de manager : le désaccord est bien apparié"
        );

        // Seconde moitié : MPC accorde, sur les trois motifs. Le même corpus à
        // l'appariement près.
        let db = base();
        for (task_id, reason, motif) in [
            (
                "task-groom",
                NotFlightReason::GroomCompleted,
                "groom_completed",
            ),
            (
                "task-refus",
                NotFlightReason::DispatchRefused,
                "dispatch_refused",
            ),
            ("task-remplace", NotFlightReason::Superseded, "superseded"),
        ] {
            // Le terme est **dérivé** de l'étiquette, jamais retapé : un
            // `not_a_flight` réécrit ici resterait vert pour toujours (mika#2158).
            let terme = format!("{}:{motif}", not_a_flight_tag());
            non_vol(&db, task_id, reason, (&terme, None, "nothing")).await;
        }

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![
                ("task-remplace", FlightAgreement::Concordant),
                ("task-refus", FlightAgreement::Concordant),
                ("task-groom", FlightAgreement::Concordant),
            ],
            "du plus récent au plus ancien, et les trois motifs s'accordent"
        );
        assert_eq!(
            rapport.consecutive_concordant, 3,
            "contrôle positif : un non-vol que MPC confirme compte comme accord"
        );
        assert!(
            rapport.rows.iter().all(|r| r.issue_number == Some(2946)),
            "le ticket se lit sur la ligne du manager : {rapport:?}"
        );
    }

    /// **AC4.** Un vol que MPC n'a pas jugé apparaît comme « non jugé par MPC »
    /// et ne compte ni pour ni contre.
    ///
    /// Le vol non jugé est au **milieu** du corpus, et c'est ce qui fait la
    /// mesure : un compte qui s'arrêterait dessus rendrait 1, un compte qui le
    /// prendrait pour un accord rendrait 3. Seul « sauté » rend 2.
    ///
    /// La seconde moitié le place au plus récent : un vol non jugé en tête ne
    /// doit pas davantage arrêter le compte, sinon un cycle de manager qui
    /// devance MPC d'un vol remettrait la mesure à zéro à chaque fois.
    #[tokio::test]
    async fn mika2922_ac4_un_vol_non_juge_par_mpc_ne_compte_ni_pour_ni_contre() {
        let db = base();
        vol(
            &db,
            "task-1",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;
        vol(&db, "task-2", DeathCause::Refusal, None).await;
        vol(
            &db,
            "task-3",
            DeathCause::PipelineFailure,
            Some("pipeline_failure"),
        )
        .await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![
                ("task-3", FlightAgreement::Concordant),
                ("task-2", FlightAgreement::NotJudgedByMpc),
                ("task-1", FlightAgreement::Concordant),
            ]
        );
        assert_eq!(
            rapport.consecutive_concordant, 2,
            "le vol non jugé est sauté : ni compté, ni bloquant"
        );
        assert!(
            rapport.rows[1].mpc.is_none(),
            "aucun verdict n'est inventé du côté qui n'a pas parlé"
        );
        assert!(
            rapport.rows[1].manager.is_some(),
            "mais le verdict du manager, lui, est bien là"
        );

        // Le même vol non jugé, en tête.
        let db = base();
        vol(
            &db,
            "task-1",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;
        vol(&db, "task-2", DeathCause::Refusal, Some("refusal")).await;
        vol(&db, "task-3", DeathCause::PipelineFailure, None).await;

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(rapport.rows[0].agreement, FlightAgreement::NotJudgedByMpc);
        assert_eq!(
            rapport.consecutive_concordant, 2,
            "un vol non jugé en tête ne remet pas le compte à zéro"
        );
    }

    /// Une ligne de MPC sans ligne de manager sous la même clé est **dite**, et
    /// un vol sans ticket lisible rend `—` plutôt que `#0`.
    ///
    /// Les deux tiennent la même règle sous deux formes : ce qui est absent est
    /// nommé absent. Un orphelin jeté ferait lire « rien à signaler » là où un
    /// côté a parlé seul ; un ticket absent rendu `0` ferait lire un numéro de
    /// ticket qui n'existe pas (motif mika#2331).
    #[tokio::test]
    async fn mika2922_un_orphelin_est_compte_et_un_ticket_absent_reste_absent() {
        let db = base();

        // Un vol dont les deux côtés sont là, et dont le manager n'a pas de
        // ticket lisible : `verdict_pr` porte `issue_number: None`.
        assert_eq!(
            write_and_swallow(&db, &verdict_pr("task-sans-ticket")).await,
            InscriptionTally::one_inscribed()
        );
        let juge = parse_judgment_terms("pr_opened", None, "merge_candidate").unwrap();
        assert_eq!(
            record_mpc_judgment(&db, "task-sans-ticket", &juge, None)
                .await
                .unwrap(),
            MpcJudgmentOutcome::Inscribed
        );

        // Et un jugement de MPC sur un vol dont le manager n'a rien inscrit.
        assert_eq!(
            record_mpc_judgment(&db, "task-orphelin", &juge, Some("vu hors fenêtre"))
                .await
                .unwrap(),
            MpcJudgmentOutcome::Inscribed
        );

        let rapport = read_concordance(&db).await.unwrap();
        assert_eq!(
            accords(&rapport),
            vec![("task-sans-ticket", FlightAgreement::Concordant)],
            "l'orphelin n'est pas une ligne du rapport : il n'a pas de vol à juger"
        );
        assert_eq!(
            rapport.orphan_mpc, 1,
            "mais il est compté, et la tête le dira"
        );
        assert_eq!(
            rapport.rows[0].issue_number, None,
            "un ticket absent reste absent — jamais #0"
        );
        assert_eq!(
            rapport.consecutive_concordant, 1,
            "la concordance d'un vol ne dépend pas de la lisibilité de son ticket"
        );

        // Anti-vacuité du compte d'orphelins : sans ligne de MPC esseulée, il
        // vaut 0 et non 1. Sans ce terme, « 1 » pourrait être un biais fixe.
        let db = base();
        assert_eq!(
            write_and_swallow(&db, &verdict_pr("task-sans-ticket")).await,
            InscriptionTally::one_inscribed()
        );
        assert_eq!(read_concordance(&db).await.unwrap().orphan_mpc, 0);
    }

    /// Un registre vide rend un rapport, pas une erreur : zéro vol, zéro
    /// concordance, et la fenêtre annoncée quand même.
    ///
    /// C'est l'état que la sonde post-déploiement d'AC6 lit **avant** le premier
    /// cycle. Un rapport qui échouerait là ferait passer « rien n'est encore
    /// inscrit » pour une panne de la commande.
    #[tokio::test]
    async fn mika2922_un_registre_vide_rend_un_rapport_et_non_une_erreur() {
        let rapport = read_concordance(&base()).await.unwrap();
        assert!(rapport.rows.is_empty());
        assert_eq!(rapport.consecutive_concordant, 0);
        assert_eq!(rapport.orphan_mpc, 0);
        assert_eq!(rapport.window_days, AUDIT_RETENTION_DAYS);
    }

    /// **Le premier champ divergent, dans l'ordre fixe.** Un désaccord
    /// simultané sur l'issue et sur la recommandation est nommé sur l'issue :
    /// c'est le terme le plus général, et celui qu'un opérateur relirait
    /// d'abord.
    ///
    /// Testé sur la projection et non sur une base, parce que la règle est celle
    /// de [`first_divergence`] seule — une base n'y ajouterait que du décor.
    #[test]
    fn mika2922_le_champ_divergent_est_le_premier_des_trois() {
        let termes = |issue: &'static str, cause: Option<&str>, reco: &'static str| VerdictTerms {
            issue,
            cause: cause.map(str::to_string),
            recommendation: reco,
        };

        let manager = termes("dead", Some("refusal"), "regroom");
        assert_eq!(
            first_divergence(&manager, &manager),
            FlightAgreement::Concordant
        );

        // Les trois divergences, chacune seule.
        assert_eq!(
            first_divergence(&manager, &termes("pr_opened", None, "regroom")),
            FlightAgreement::Divergent { field: "issue" }
        );
        assert_eq!(
            first_divergence(&manager, &termes("dead", Some("escalated"), "regroom")),
            FlightAgreement::Divergent { field: "cause" }
        );
        assert_eq!(
            first_divergence(
                &manager,
                &termes("dead", Some("refusal"), "merge_candidate")
            ),
            FlightAgreement::Divergent {
                field: "recommendation"
            }
        );

        // Et les trois ensemble : l'issue gagne.
        assert_eq!(
            first_divergence(
                &manager,
                &termes("pr_opened", Some("escalated"), "merge_candidate")
            ),
            FlightAgreement::Divergent { field: "issue" }
        );

        // Une cause absente d'un seul côté est une divergence, pas un accord :
        // « pas de cause » n'est pas une cause (motif mika#2331).
        assert_eq!(
            first_divergence(&manager, &termes("dead", None, "regroom")),
            FlightAgreement::Divergent { field: "cause" }
        );
    }

    /// Le ticket porté par `reasoning`, et les trois formes qui n'en portent
    /// pas. Aucune ne rend `0` : c'est la même absence, dite comme telle.
    #[test]
    fn mika2922_le_ticket_se_lit_sous_le_prefixe_ou_pas_du_tout() {
        assert_eq!(ticket_du_manager(Some("issue_number=2922")), Some(2922));
        assert_eq!(ticket_du_manager(None), None, "colonne NULL");
        assert_eq!(
            ticket_du_manager(Some("vu avec Vincent")),
            None,
            "la note libre de MPC n'est pas un ticket — et c'est pourquoi le \
             ticket se lit sur la ligne du manager"
        );
        assert_eq!(
            ticket_du_manager(Some("issue_number=deux-mille")),
            None,
            "un suffixe non numérique n'est pas un ticket"
        );
        // Le préfixe est celui que l'écrivain pose : la constante, jamais un
        // littéral recopié (classe mika#2158).
        assert_eq!(
            ticket_du_manager(Some(&format!("{ISSUE_NUMBER_PREFIX}7"))),
            Some(7)
        );
    }

    /// Une ligne illisible **arrête** le compte, et ne vaut jamais un accord.
    ///
    /// Sur [`consecutive_concordant`] seule, et c'est délibéré : produire une
    /// ligne illisible en base exigerait d'écrire un `after_value` que les
    /// écrivains ne produisent pas, donc du SQL de fixture — et une fixture qui
    /// pose ses propres colonnes ne mesure plus la production. La relecture
    /// elle-même est tenue par l'aller-retour serde de
    /// `mika2922_les_termes_sont_les_litteraux_serde`.
    #[test]
    fn mika2922_une_ligne_illisible_arrete_le_compte() {
        let ligne = |task_id: &str, agreement: FlightAgreement| ConcordanceRow {
            task_id: task_id.to_string(),
            issue_number: None,
            manager: None,
            mpc: None,
            agreement,
            created_at: "2026-10-26T10:00:00Z".to_string(),
        };

        // Un illisible en tête : zéro, alors que deux vols concordent derrière.
        assert_eq!(
            consecutive_concordant(&[
                ligne("t-3", FlightAgreement::Unreadable { side: "mpc" }),
                ligne("t-2", FlightAgreement::Concordant),
                ligne("t-1", FlightAgreement::Concordant),
            ]),
            0,
            "« je ne sais pas » ne vaut pas « ils étaient d'accord »"
        );

        // Le même corpus sans l'illisible : 2. L'écart est la mesure.
        assert_eq!(
            consecutive_concordant(&[
                ligne("t-2", FlightAgreement::Concordant),
                ligne("t-1", FlightAgreement::Concordant),
            ]),
            2
        );

        // Et l'illisible arrête là où il est, comme le divergent.
        assert_eq!(
            consecutive_concordant(&[
                ligne("t-3", FlightAgreement::Concordant),
                ligne("t-2", FlightAgreement::Unreadable { side: "manager" }),
                ligne("t-1", FlightAgreement::Concordant),
            ]),
            1
        );
    }

    // ---- Le rendu : ce que l'opérateur lit ---------------------------------

    /// **La tête porte les deux nombres et la fenêtre**, et le tableau porte
    /// les cinq colonnes avec le champ divergent dans la cinquième.
    ///
    /// Le corpus est celui d'AC3 — deux concordants et un divergent sur la
    /// cause, le divergent au milieu — donc la tête annonce `1` et non `2` :
    /// le rendu lit le même compte que [`read_concordance`] et n'en recalcule
    /// aucun.
    #[tokio::test]
    async fn mika2922_le_rendu_porte_la_tete_les_cinq_colonnes_et_le_champ_divergent() {
        let db = base();
        vol(
            &db,
            "task-1",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;
        vol(&db, "task-2", DeathCause::CeilingReached, Some("escalated")).await;
        vol(
            &db,
            "task-3",
            DeathCause::CeilingReached,
            Some("ceiling_reached"),
        )
        .await;

        let texte = render_concordance_text(&read_concordance(&db).await.unwrap());

        assert!(
            texte.contains(&format!(
                "Concordance manager / MPC — 3 vols, fenêtre de {AUDIT_RETENTION_DAYS} jours"
            )),
            "la tête annonce le nombre de vols et la fenêtre : {texte}"
        );
        assert!(
            texte.contains("1 vol consécutif concordant (du plus récent au plus ancien ; seuil mika#2698 : 10 vols ou 7 jours)"),
            "la tête annonce le compte, au singulier, et nomme le seuil : {texte}"
        );
        for colonne in COLUMNS {
            assert!(
                texte.contains(colonne),
                "colonne « {colonne} » absente : {texte}"
            );
        }
        assert!(
            texte.contains("divergent (cause)"),
            "le champ divergent est nommé dans la colonne accord : {texte}"
        );
        assert!(
            texte.contains("concordant\n"),
            "les deux vols d'accord sont dits concordants : {texte}"
        );
        assert!(
            texte.contains("#2922"),
            "le ticket se lit sur la ligne du manager : {texte}"
        );
        // La clé est rendue **entière** : c'est elle que l'étape 5 de la sonde
        // d'AC6 demande de recopier dans `judge --task-id`.
        assert!(
            texte.contains("task-2"),
            "la clé n'est pas tronquée : {texte}"
        );
        // Le vocabulaire est celui de `reporter.rs`, pas un second.
        assert!(
            texte.contains("mort au plafond") && texte.contains("re-groom"),
            "les labels viennent de reporter.rs : {texte}"
        );
        // Contrôle négatif de la tête : ce corpus n'a pas d'orphelin, donc la
        // ligne des orphelins est **absente** — sans quoi « 0 orphelin » et
        // « orphelins non comptés » rendraient le même texte.
        assert!(
            !texte.contains("sans verdict du manager"),
            "pas d'orphelin ici, donc pas de ligne d'orphelins : {texte}"
        );
    }

    /// Un vol non jugé par MPC, et un ticket absent, rendent la cellule
    /// [`ABSENT`] et jamais une cellule vide, un `0` ou un `#0`.
    ///
    /// C'est exactement ce que l'étape 4 de la sonde d'AC6 doit lire.
    #[tokio::test]
    async fn mika2922_le_rendu_dit_labsence_et_lorphelin() {
        let db = base();
        // `verdict_pr` ne porte pas de ticket, et MPC ne juge pas ce vol.
        assert_eq!(
            write_and_swallow(&db, &verdict_pr("task-seul")).await,
            InscriptionTally::one_inscribed()
        );
        let juge = parse_judgment_terms("dead", Some("escalated"), "regroom").unwrap();
        record_mpc_judgment(&db, "task-orphelin", &juge, None)
            .await
            .unwrap();

        let texte = render_concordance_text(&read_concordance(&db).await.unwrap());

        assert!(
            texte.contains("non jugé par MPC"),
            "AC4 se lit dans le rendu : {texte}"
        );
        assert!(
            texte.contains(ABSENT),
            "le ticket absent et le côté MPC absent rendent « {ABSENT} » : {texte}"
        );
        assert!(
            !texte.contains("#0"),
            "un ticket absent n'est jamais le ticket #0 : {texte}"
        );
        assert!(
            texte.contains("1 jugement de MPC sans verdict du manager sous la même clé"),
            "l'orphelin est dit, pas jeté : {texte}"
        );
        assert!(
            texte.contains("0 vol consécutif concordant"),
            "un vol non jugé ne fait pas monter le compte : {texte}"
        );
    }

    /// **Un registre vide rend la tête, et le dit.** C'est l'état que l'étape 2
    /// de la sonde d'AC6 lit avant le premier cycle : la tête, la fenêtre, et
    /// `0 vol`. Une sortie muette ferait passer « rien n'est inscrit » pour une
    /// commande en panne.
    #[tokio::test]
    async fn mika2922_le_rendu_dun_registre_vide_porte_quand_meme_la_tete() {
        let texte = render_concordance_text(&read_concordance(&base()).await.unwrap());

        assert!(
            texte.contains(&format!(
                "Concordance manager / MPC — 0 vol, fenêtre de {AUDIT_RETENTION_DAYS} jours"
            )),
            "la fenêtre est annoncée même à vide, au singulier : {texte}"
        );
        assert!(texte.contains("0 vol consécutif concordant"), "{texte}");
        assert!(
            texte.contains("Aucun vol inscrit dans la fenêtre."),
            "{texte}"
        );
        assert!(
            !texte.contains(COLUMNS[4]),
            "pas de tableau sans ligne : {texte}"
        );
    }

    /// Une ligne illisible est **dite** illisible, des deux côtés distinctement,
    /// et jamais rendue pour un désaccord.
    ///
    /// Sur la projection et non sur une base, pour la raison de
    /// [`mika2922_une_ligne_illisible_arrete_le_compte`] : les écrivains des
    /// vols 1 et 2 ne produisent pas d'`after_value` illisible, donc en
    /// fabriquer un en base demanderait le SQL de fixture que la disposition du
    /// plan refuse.
    #[test]
    fn mika2922_le_rendu_nomme_le_cote_illisible() {
        let ligne = |agreement, manager: Option<VerdictRecord>| ConcordanceRow {
            task_id: "task-1".to_string(),
            issue_number: Some(2922),
            manager,
            mpc: None,
            agreement,
            created_at: "2026-10-26T10:00:00Z".to_string(),
        };
        let record = VerdictRecord {
            issue: FlightIssue::Dead {
                cause: DeathCause::Escalated,
            },
            recommendation: FlightRecommendation::Regroom,
            ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
        };

        let cote_manager = render_concordance_text(&ConcordanceReport {
            rows: vec![ligne(
                FlightAgreement::Unreadable { side: SIDE_MANAGER },
                None,
            )],
            consecutive_concordant: 0,
            window_days: AUDIT_RETENTION_DAYS,
            orphan_mpc: 0,
        });
        assert!(
            cote_manager.contains(&format!("illisible ({SIDE_MANAGER})")),
            "la colonne accord nomme le côté fautif : {cote_manager}"
        );
        assert!(
            cote_manager.contains("illisible  ") || cote_manager.contains("illisible\n"),
            "la cellule du manager dit « illisible » : {cote_manager}"
        );

        // L'autre côté : le manager se lit, MPC non. La cellule du manager
        // porte son verdict et c'est celle de MPC qui dit « illisible ».
        let cote_mpc = render_concordance_text(&ConcordanceReport {
            rows: vec![ligne(
                FlightAgreement::Unreadable { side: SIDE_MPC },
                Some(record),
            )],
            consecutive_concordant: 0,
            window_days: AUDIT_RETENTION_DAYS,
            orphan_mpc: 0,
        });
        assert!(
            cote_mpc.contains(&format!("illisible ({SIDE_MPC})")),
            "{cote_mpc}"
        );
        assert!(
            cote_mpc.contains("mort — escalade"),
            "le côté lisible reste rendu : {cote_mpc}"
        );
    }

    /// Les colonnes s'alignent sur le contenu, en **caractères** et non en
    /// octets. « mort au plafond » porte des accents : un `len()` sur les octets
    /// déplacerait chaque colonne d'autant d'accents que la cellule en porte.
    #[test]
    fn mika2922_lalignement_compte_des_caracteres_pas_des_octets() {
        let ligne = |task_id: &str, cause: DeathCause| ConcordanceRow {
            task_id: task_id.to_string(),
            issue_number: Some(2922),
            manager: Some(VerdictRecord {
                issue: FlightIssue::Dead { cause },
                recommendation: FlightRecommendation::Regroom,
                ci: CiKnowledge::Unread(CiUnreadCause::NotMeasured),
            }),
            mpc: None,
            agreement: FlightAgreement::NotJudgedByMpc,
            created_at: "2026-10-26T10:00:00Z".to_string(),
        };
        let texte = render_concordance_text(&ConcordanceReport {
            rows: vec![
                // Le premier label porte deux accents, le second aucun.
                ligne("task-accentue", DeathCause::Escalated),
                ligne("task-sans-accent", DeathCause::CeilingReached),
            ],
            consecutive_concordant: 0,
            window_days: AUDIT_RETENTION_DAYS,
            orphan_mpc: 0,
        });

        // « accord » est la dernière colonne, donc le rang de son premier
        // caractère vaut la longueur de la ligne moins celle de la cellule.
        let debuts: Vec<usize> = texte
            .lines()
            .filter(|l| l.ends_with("non jugé par MPC"))
            .map(|l| l.chars().count() - "non jugé par MPC".chars().count())
            .collect();
        assert_eq!(debuts.len(), 2, "deux lignes de vol : {texte}");
        assert_eq!(
            debuts[0], debuts[1],
            "la dernière colonne commence au même caractère sur les deux lignes : {texte}"
        );
    }

    // ---- Le lecteur unique des deux registres -----------------------------

    /// Les quatre aiguilles par lesquelles un site nomme l'un des deux
    /// registres : les deux littéraux, et les deux constantes qui les portent.
    ///
    /// Elles sont **concaténées** plutôt qu'écrites d'un bloc. La moitié
    /// production de ce fichier est tronquée avant `mod tests`, donc la garde ne
    /// se verrait pas elle-même de toute façon — mais cette troncature repose
    /// sur un littéral `#[cfg(test)]` qu'un module de test extrait ne porterait
    /// plus (classe mika#2321), et une garde qui se mord elle-même est rouge
    /// pour une raison qui n'a rien à voir avec son objet.
    fn aiguilles_du_registre() -> [String; 4] {
        [
            String::from("\"manager_flight_") + "verdict\"",
            String::from("\"mpc_flight_") + "judgment\"",
            String::from("MANAGER_FLIGHT_VERDICT_") + "TOOL",
            String::from("MPC_FLIGHT_JUDGMENT_") + "TOOL",
        ]
    }

    /// Les rangs (1-indexés) des lignes de `source` qui nomment un des deux
    /// registres, commentaires exclus.
    ///
    /// Le retrait des commentaires est **porteur** : de la prose *sur* le
    /// registre dans un autre fichier n'est pas une lecture, et une garde qui
    /// la refuserait serait une taxe sur la documentation (la forme du signal S
    /// de mika#2050). C'est aussi pourquoi le prédicat n'est pas
    /// [`audit_writers`], qui ne retire rien : lui cherche un **geste**
    /// d'écriture, celui-ci cherche une **mention**.
    ///
    /// Le filtre passe par [`strip_comment_lines`] ligne à ligne plutôt que sur
    /// la source entière, pour que le rang rendu soit celui du fichier et non
    /// celui d'un texte amputé.
    fn sites_nommant_le_registre(source: &str) -> Vec<usize> {
        let aiguilles = aiguilles_du_registre();
        source
            .lines()
            .enumerate()
            .filter(|(_, ligne)| !strip_comment_lines(ligne).trim().is_empty())
            .filter(|(_, ligne)| aiguilles.iter().any(|a| ligne.contains(a.as_str())))
            .map(|(i, _)| i + 1)
            .collect()
    }

    /// **Le lecteur unique.** Hors de ce fichier, aucun site de production de ce
    /// crate ne nomme l'un des deux `tool_name` : la lecture du registre passe
    /// par [`read_concordance`], comme son écriture passe par
    /// [`FlightVerdictLedger::record`] et [`record_mpc_judgment`].
    ///
    /// **Pourquoi le prédicat porte sur le nom et non sur la lecture.** « Lire
    /// le registre » n'a pas de forme syntaxique stable — c'est aujourd'hui un
    /// appel à `list_audit_events_for_tool_since`, demain l'une de ses voisines
    /// ou une cinquième. Ce qui est stable, c'est que **toutes** ont besoin du
    /// `tool_name` : un site qui ne le nomme pas ne peut pas lire ces lignes-là.
    /// Le prédicat est donc plus large que « une lecture » — il subsume la
    /// lecture et l'écriture — et c'est ce qui le rend auto-nettoyant et sans
    /// liste d'exception. La liste est **absente**, pas vide (doctrine
    /// mika#2201) : quand la garde rougit, le remède est de router le site par
    /// le chemin canonique, jamais d'y ajouter une ligne.
    ///
    /// **Ce que ce scan ne voit pas, dit ici plutôt que supposé.**
    /// [`production_files`] est enraciné sur le `src` de *ce* crate et écarte
    /// les sources de test ([`crate::source_scan::is_test_source_path`]), donc
    /// ni un lecteur côté `mika-cli` ni un fichier de test ne lui sont visibles.
    /// Le côté CLI porte sa propre garde dans son propre crate
    /// (`mika2922_ladaptateur_nest_pas_un_ecrivain`), et c'est précisément
    /// pourquoi le lecteur vit ici : une garde enracinée ailleurs serait verte
    /// par construction (classe mika#2205).
    #[test]
    fn mika2922_ac3_un_seul_lecteur_du_registre() {
        let files = production_files();

        // Anti-vacuité, deux termes (mika#2205). Le premier : l'arbre a été lu.
        assert!(
            files.len() >= 100,
            "anti-vacuité : {} fichiers lus, le scan n'a rien lu",
            files.len()
        );
        // Le second : le seul site autorisé est bien dans ce qui a été lu. Un
        // balayage qui ne voit pas le registre ne dit rien du lecteur.
        assert!(
            files
                .iter()
                .any(|(rel, _)| rel == "flight_verdict_ledger.rs"),
            "anti-vacuité : le scan ne voit pas flight_verdict_ledger.rs"
        );

        let mut sites: Vec<String> = Vec::new();
        for (rel, prod) in &files {
            if rel == "flight_verdict_ledger.rs" {
                continue;
            }
            for rang in sites_nommant_le_registre(prod) {
                sites.push(format!("{rel}:{rang}"));
            }
        }

        assert!(
            sites.is_empty(),
            "SOLE READER : le registre des verdicts de vol a un seul lecteur, \
             read_concordance, et il vit dans flight_verdict_ledger.rs. \
             Site(s) nommant un de ces tool_name ailleurs :\n{}\n\
             Route la lecture par read_concordance et l'écriture par \
             record / record_mpc_judgment — n'ajoute pas d'entrée \
             (doctrine mika#2201).",
            sites.join("\n")
        );
    }

    /// Contrôle négatif du scan du lecteur. Il appelle
    /// [`sites_nommant_le_registre`], le prédicat **réel** de la garde, plutôt
    /// que d'en reproduire la forme comme le font ses deux voisins de l'AC2 :
    /// ici la question n'est pas « la fenêtre de 5 lignes mord-elle ? » mais
    /// « les quatre aiguilles sont-elles exactes et le retrait des commentaires
    /// est-il bien en place ? », et une reproduction porterait les mêmes fautes
    /// que l'original.
    ///
    /// Trois sources fabriquées, et chacune tient un terme que la verdeur seule
    /// ne distinguerait pas :
    ///
    /// 1. un **second lecteur** — une lecture fenêtrée sous le `tool_name` du
    ///    manager : le prédicat doit mordre, sinon « un seul lecteur » est
    ///    indistinguable de « le scan ne trouve jamais rien » (classe
    ///    mika#2205) ;
    /// 2. une lecture d'`audit_events` **légitime**, sous un autre `tool_name`
    ///    (celui de mika#2917) : le prédicat ne doit rien en dire, sinon la
    ///    garde parle d'autre chose que de ces deux registres ;
    /// 3. une mention en **commentaire** : le retrait est porteur, donc il est
    ///    attesté. Sans ce terme, on ne saurait pas si la prose passe par
    ///    décision ou par accident.
    #[test]
    fn mika2922_le_scan_du_lecteur_mord() {
        let second_lecteur = "    let lignes = db\n        \
                              .list_audit_events_for_tool_since(MANAGER_FLIGHT_VERDICT_TOOL, since)\n        \
                              .await?;\n";
        assert_eq!(
            sites_nommant_le_registre(second_lecteur),
            vec![2],
            "le scan doit mordre sur un second lecteur, et nommer sa ligne"
        );

        let litteral_recopie = "    let tool = \"mpc_flight_judgment\";\n";
        assert_eq!(
            sites_nommant_le_registre(litteral_recopie),
            vec![1],
            "le scan doit mordre sur le littéral recopié"
        );

        // La population que la garde ne doit PAS refuser : une lecture
        // d'`audit_events` sous un `tool_name` qui n'est ni l'un ni l'autre.
        let lecture_legitime = "    let lignes = db\n        \
                                .get_audit_event_rows_by_tool_name(QA_REVIEWED_HEAD_TOOL)\n        \
                                .await?;\n";
        assert!(
            sites_nommant_le_registre(lecture_legitime).is_empty(),
            "le scan ne doit RIEN dire d'une lecture d'audit sous un autre tool_name"
        );

        let en_commentaire = "    // MANAGER_FLIGHT_VERDICT_TOOL est écrit par le registre,\n    \
                              /// et \"mpc_flight_judgment\" l'est par record_mpc_judgment.\n";
        assert!(
            sites_nommant_le_registre(en_commentaire).is_empty(),
            "parler du registre n'est pas le lire : le retrait des commentaires \
             est porteur et doit tenir"
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
    /// 3. **Auto-nettoyant** : la même moitié production *doit* contenir les
    ///    **deux** constantes, `MANAGER_FLIGHT_VERDICT_TOOL` et
    ///    `MPC_FLIGHT_JUDGMENT_TOOL`. Le jour où `production_half` cesse de
    ///    découper ce fichier — ancre changée, `mod tests` renommé — les deux
    ///    premières assertions deviendraient vraies sur une chaîne vide, et
    ///    celle-ci rougit à leur place. Les deux noms plutôt qu'un : le second
    ///    côté est né au vol 2 **dans ce même fichier**, et c'est ce qui étend
    ///    AC5 au nouveau site d'écriture sans ajouter de garde (le périmètre
    ///    balayé par `no_dispatch_test.rs` est le même pour les deux).
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
            code.contains("MPC_FLIGHT_JUDGMENT_TOOL"),
            "anti-vacuité (vol 2) : le second côté vit dans ce même fichier, \
             donc AC5 le couvre — s'il n'y est plus, cette garde ne dit plus \
             rien de lui"
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

    // ---- mika#2968 : le champ CI entre au registre, pas au comparateur ----

    fn record_avec_ci(ci: CiKnowledge) -> VerdictRecord {
        VerdictRecord {
            issue: FlightIssue::PrOpened {
                number: Some(2966),
                url: None,
            },
            recommendation: FlightRecommendation::MergeCandidate,
            ci,
        }
    }

    /// AC4 — la CI n'entre pas dans les termes comparés.
    ///
    /// Deux lignes qui ne diffèrent que par la couleur de la CI rendent les
    /// **mêmes** `VerdictTerms`. C'est la moitié refusée du ticket : projeter la
    /// CI dans la comparaison aurait noyé la divergence du vol 4f1fe163 dans une
    /// seconde, sur un axe que MPC ne juge pas — il juge à la main et ne lit
    /// aucun `statusCheckRollup`.
    #[test]
    fn mika2968_ac4_la_ci_nentre_pas_dans_les_termes_compares() {
        let rouge = record_avec_ci(CiKnowledge::Read {
            state: CiState::Failed,
            at: "2026-10-29T17:46:12Z".into(),
        });
        let vert = record_avec_ci(CiKnowledge::Read {
            state: CiState::Success,
            at: "2026-10-29T17:46:12Z".into(),
        });
        assert_ne!(
            rouge.ci, vert.ci,
            "anti-vacuité : les deux lignes diffèrent"
        );
        assert_eq!(
            rouge.terms(),
            vert.terms(),
            "la CI n'entre pas dans les termes comparés"
        );
    }

    /// AC4, seconde moitié — la divergence du vol mesuré reste divergente, et
    /// le nouveau champ l'explique.
    ///
    /// La ligne du manager est relue depuis un blob **antérieur** au champ : son
    /// `ci` vaut donc `Unread(NotMeasured)` — « rien n'a été mesuré », jamais une
    /// CI verte inventée (mika#2331). Le verdict reste `divergent` sur
    /// `recommendation`, exactement comme avant ce ticket.
    #[test]
    fn mika2968_ac4_la_divergence_du_vol_4f1fe163_reste_divergente() {
        let manager: VerdictRecord = serde_json::from_str(
            r#"{"issue":{"pr_opened":{"number":2966,"url":null}},
                "recommendation":"merge_candidate"}"#,
        )
        .expect("un blob de verdict antérieur au champ se relit");
        assert_eq!(
            manager.ci,
            CiKnowledge::Unread(CiUnreadCause::NotMeasured),
            "un blob antérieur au champ ne porte aucune mesure"
        );

        let mpc = VerdictRecord {
            issue: FlightIssue::PrOpened {
                number: Some(2966),
                url: None,
            },
            recommendation: FlightRecommendation::ManualResumption,
            ci: CiKnowledge::default_unread(),
        };

        assert_eq!(
            first_divergence(&manager.terms(), &mpc.terms()),
            FlightAgreement::Divergent {
                field: FIELD_RECOMMENDATION
            }
        );
    }
}

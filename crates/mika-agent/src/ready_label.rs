//! L'unique applicateur du label `ready` du dépôt (mika#2315).
//!
//! # Ce que ce module ferme
//!
//! Deux défauts indépendants, mesurés le 2026-09-15 sur mika#2295.
//!
//! **B1 — le frein de la Phase 2 était inopérant au-delà de 100 événements.**
//! `auto_pull::gh_ready_label_age_secs` lisait `…/timeline?per_page=100`, **une
//! seule page**, et prenait le dernier `labeled(ready)` qu'elle contenait. Or
//! l'API timeline rend ses événements en ordre **chronologique ascendant** : la
//! page 1 porte les 100 plus **anciens**. Sur une issue qui dépasse 100
//! événements — ce qu'un ticket déjà re-drivé plusieurs fois dépasse
//! mécaniquement — le `labeled(ready)` que la Phase 2 venait d'écrire vivait en
//! page 2 et n'était jamais lu. L'âge rendu était celui d'un `labeled` ancien,
//! donc toujours ≥ seuil, à chaque tick, pour toujours. Le self-throttle ne
//! freinait rien sur exactement la population qu'il existe pour freiner, et
//! s'affaiblissait à mesure que le ticket était re-drivé.
//!
//! **B2 — retirer `ready` n'était pas un park.** Un ticket groomé, non exclu et
//! **sans** `ready` est précisément le candidat que les Phases 0 et 1 cherchent.
//! Un opérateur qui retirait `ready` pour parquer un ticket le rendait
//! immédiatement éligible à la re-promotion, au tick suivant, en ≤ 10 minutes.
//! Le seul park qui tenait était un **label** (`operator-review`, `blocked`,
//! `operator-gated`), qu'il fallait connaître.
//!
//! # Pourquoi un applicateur canonique, et pas un correctif sur la Phase 2
//!
//! L'exigence du ticket dit « le reaper **ou tout re-dispatch auto** ». Quatre
//! sites appliquaient `ready`, dont un hors d'`auto_pull` (la cascade milestone),
//! que personne n'aurait pensé à patcher en traitant ce symptôme-ci. Le dépôt a
//! déjà payé cette leçon deux fois : la regex de grooming dupliquée entre
//! `auto_pull` et `executor`, qui a divergé des mois en silence (mika#2158), et
//! l'accesseur étroit à côté du résolveur canonique (mika#2205).
//!
//! Modèle retenu : celui de [`crate::grooming_marker`] — un seul écrivain, plus
//! un test de scan de source qui refuse l'apparition d'un second
//! ([`tests::no_ready_label_write_outside_this_module`]). Un test de comportement
//! ne peut pas attraper cette classe : un cinquième applicateur écrit demain ne
//! rendrait aucune décision fausse, il la rendrait **non gardée**, et toutes les
//! assertions existantes resteraient vertes.
//!
//! # Le prédicat de park (D2)
//!
//! ```text
//! parked(issue) :=
//!   E := événements de timeline dont label.name == "ready"
//!        et event ∈ {"labeled", "unlabeled"}, ordonnés par created_at
//!   E vide                                  → non parqué
//!   dernier(E).event == "labeled"           → non parqué
//!   dernier(E).actor ∈ identités_machine    → non parqué
//!   sinon                                   → PARQUÉ
//! ```
//!
//! Trois propriétés le rendent correct :
//!
//! - **Le remove→add de la Phase 2 ne se parque pas lui-même.** Entre son
//!   `remove` et son `add`, le dernier événement est un `unlabeled` **machine**.
//! - **La sortie de park est un geste unique et symétrique** : l'opérateur remet
//!   `ready` à la main, le dernier événement devient `labeled`, le park tombe.
//!   Rien à retirer, aucun label à connaître, aucune documentation à avoir lue.
//! - **Les retraits machine légitimes restent des non-parks** :
//!   `draft_pr_opened_handler` (retire `ready` à l'ouverture d'une PR draft) et
//!   `auto_pull::abandon_stuck_ready` (retire `ready`, pose `operator-review`).
//!   Le premier reste couvert par le filtre `has_open_pr`, le second par
//!   `is_feeder_excluded` : ni l'un ni l'autre ne perd sa protection.
//!
//! # Les deux sens d'échec, et pourquoi ils divergent d'ailleurs
//!
//! **D4 — timeline illisible → refus, pas application.** Erreur API, pagination
//! incomplète, JSON illisible : [`apply_ready`] refuse et laisse le tick suivant
//! réessayer. L'asymétrie est mesurée : un faux négatif coûte **10 minutes de
//! latence** (un tick), un faux positif coûte **~600k tokens Opus** et casse un
//! STOP opérateur.
//!
//! C'est l'**inverse** du fail-open retenu ailleurs dans `auto_pull`
//! (`gh_list_open_pr_issue_numbers` échoue en ensemble vide « to preserve
//! pre-fix behavior on infra glitches »). Diverger ici est délibéré : ce
//! fail-open-là élargit un filtre dont l'échec fait *rater* une exclusion ;
//! celui-ci gouverne une écriture dont l'échec fait *agir*.
//!
//! **D3 — identités machine vides → refus.** Sans savoir qui est la machine,
//! tout retrait ressemble à un park, et appliquer `ready` reviendrait à
//! restaurer le bug en entier, en silence. Le coût est réel et nommé : une
//! machine mal configurée voit sa boucle s'arrêter. Il est borné par trois
//! sources de résolution indépendantes et par un signal opérateur explicite —
//! et c'est la même asymétrie que `assert_family_tier_env_consistency` tranche
//! déjà dans le même sens (refuser de démarrer plutôt que servir la mauvaise
//! politique).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;
use tracing::{info, warn};

use mika_common::label_write::LabelWriteToken;

use crate::async_db::AsyncDatabase;
use crate::gh_process::{GhSubject, run_gh};

/// Le label gouverné par ce module. Écrit **une seule fois** dans la crate, ici.
pub(crate) const READY_LABEL: &str = "ready";

/// Nombre d'événements demandés par page de timeline.
const TIMELINE_PER_PAGE: u32 = 100;

/// Plafond de pages lues (D5) — 20 pages, soit 2000 événements. Au-delà, refus.
///
/// Le plafond existe pour que le prédicat ne devienne jamais un amplificateur
/// d'appels API sur un ticket pathologique. Il n'est **pas** un repli silencieux
/// vers une lecture partielle : une timeline plus longue rend
/// [`ReadyApplyOutcome::RefusedUnreadable`], parce qu'une lecture partielle est
/// exactement le mécanisme de B1.
const MAX_TIMELINE_PAGES: u32 = 20;

/// Variable d'environnement portant les logins machine que ni la configuration
/// App ni le token courant ne révèlent (liste séparée par virgules).
const LOOP_BOT_LOGINS_ENV: &str = "MIKA_LOOP_BOT_LOGINS";

/// Variable d'environnement portant le login du bot GitHub App.
const GITHUB_APP_LOGIN_ENV: &str = "MIKA_GITHUB_APP_LOGIN";

/// `session_id` conventionnel des lignes d'audit écrites par ce module quand
/// l'appelant n'en porte pas (la cascade milestone). Même motif que
/// `evidence::audit::AUTH_BOUNDARY_SESSION_ID` : une décision de politique
/// n'appartient à aucune conversation.
pub(crate) const READY_LABEL_SESSION_ID: &str = "ready-label";

/// `tool_name` des lignes d'audit de refus-pour-park (AC8).
pub(crate) const READY_LABEL_PARK_TOOL_NAME: &str = "ready_label_park";

// ──────── Le discriminant de parente, et les noms de la relecture ────────
//
// mika#2995, tranche du 2026-11-03 : le consentement est lu à l'ENTRÉE de la
// file (le webhook `labeled ready` crée la tâche parente) et n'était jamais
// relu à la SORTIE, au moment où le wrapper différé promu part en tour. Les
// trois noms ci-dessous sont les sites de définition uniques de cette
// relecture (AC-T4) ; aucun ne contient `noop` nu, déjà pris par
// `deferred_dispatch_noop_completion` (mika#1124) et par le vocabulaire de
// mika#2169.

/// Préfixe du label des tâches parentes créées par le feeder `ready-label`.
///
/// **SITE DE DÉFINITION UNIQUE** (doctrine mika#2158, classe `grooming_marker`) :
/// le producteur (`server/ready_label_handler.rs`) **compose** par cette
/// constante, et le prédicat de la garde mika#2995 **lit** par elle. Deux
/// littéraux pour le même fait dériveraient en silence, et la dérive serait
/// invisible des deux côtés — le label continuerait d'être produit, la garde
/// cesserait simplement de reconnaître ses parentes.
pub(crate) const READY_LABEL_TASK_PREFIX: &str = "ready-label:";

/// Motif terminal écrit sur le wrapper différé **et** sur sa parente quand le
/// consentement a été retiré entre la création de la file et le départ du tour.
///
/// **Le même littéral des deux côtés, délibérément** : c'est un seul fait, et
/// deux motifs pour un fait sont deux populations qu'on ne peut plus rapprocher
/// (classe mika#2205). Il ne recouvre ni `phantom_aged_out`, ni
/// `callback_no_pr_*`, ni `dispatch_halt_*`, ni
/// `stuck_pending_no_deferred_wrapper` — chacun garde sa propre issue.
pub(crate) const READY_CONSENT_WITHDRAWN_REASON: &str = "ready_consent_withdrawn";

/// `tool_name` de la ligne d'audit de la relecture de consentement.
///
/// Frère de [`READY_LABEL_PARK_TOOL_NAME`] ci-dessus, et distinct de lui : le
/// park est une **écriture refusée**, la relecture est un **dispatch refusé**.
/// Les agréger sous un nom rendrait les deux non soustractibles.
pub(crate) const READY_CONSENT_RECHECK_TOOL_NAME: &str = "ready_label_consent_recheck";

// ──────── Les noms de la relecture AVANT un ré-armement (mika#3050) ────────
//
// mika#2995 garde la **promotion** d'un lanceur différé ; le **ré-armement**
// d'une parente déjà en file ne l'était pas. Les deux gardes portent des raisons
// terminales distinctes, exprès : deux populations sous un nom ne sont plus
// soustractibles (mika#2205), et la cible d'AC4 est précisément un compte.

/// Motif terminal de la parente dont le ticket ne porte plus `ready` à l'instant
/// du ré-armement. Le littéral est celui qu'AC1 de mika#3050 exige.
pub(crate) const READY_ABSENT_AT_REARM_REASON: &str = "ready_absent_at_rearm";

/// Motif terminal de la parente dont le ticket a été **fermé** avant le
/// ré-armement. Littéral exigé par AC1 de mika#3050.
pub(crate) const ISSUE_CLOSED_AT_REARM_REASON: &str = "issue_closed_at_rearm";

/// `tool_name` de la ligne d'audit de la relecture de ré-armement.
///
/// Distinct de [`READY_CONSENT_RECHECK_TOOL_NAME`] : celui-là nomme un *dispatch*
/// refusé à la promotion, celui-ci un *ré-armement* refusé 45 min plus tard. Les
/// agréger sous un nom rendrait les deux greps d'AC4 illisibles.
pub(crate) const REARM_CONSENT_RECHECK_TOOL_NAME: &str = "ready_label_rearm_recheck";

/// `"ready-label: senara-solutions/mika#3002"` → `("senara-solutions/mika", 3002)`.
///
/// # Pourquoi les coordonnées se lisent dans le LABEL
///
/// La traversée `parent_issue_coordinates` (`task_engine/dispatcher.rs`) existe
/// et fonctionne, mais elle passe par `parse_repo_issue_from_url`, qui rend le
/// **repo nu** (`"mika"`) — alors que [`read_ready_label_timeline`] interpole
/// `repos/{repo}/issues/{issue}/timeline` et exige `owner/repo`. Re-préfixer
/// `senara-solutions/` serait un littéral d'organisation codé en dur dans le
/// dispatcher. Le label, lui, porte déjà `location.owner_repo()` : l'information
/// exacte, à sa source, sans devinette.
///
/// # Pourquoi `None` est le discriminant, et non une erreur
///
/// `None` veut dire « cette parente ne vient pas du feeder `ready-label` », et
/// c'est le cas d'AC-T3 : groom, revue, dispatch manuel, parente absente. Un
/// label qui porte le préfixe mais dont les coordonnées ne se lisent pas tombe
/// dans le même sac, et c'est **structurellement sans population** : le
/// producteur unique compose par [`READY_LABEL_TASK_PREFIX`] avec cette forme
/// exacte, et `parse_le_label_du_producteur_fait_laller_retour` épingle
/// l'aller-retour.
pub(crate) fn parse_ready_label_task_label(label: &str) -> Option<(String, u64)> {
    let rest = label.strip_prefix(READY_LABEL_TASK_PREFIX)?.trim();
    // `rsplit_once` et non `split_once` : un `#` dans un nom de dépôt est
    // impossible, mais la dernière occurrence est celle qui sépare le numéro
    // quoi qu'il arrive.
    let (repo, number) = rest.rsplit_once('#')?;
    let repo = repo.trim();
    // `owner/repo` est exigé — un repo nu ferait une URL d'API fausse, qui
    // rendrait un 404 que D4 lirait comme une timeline illisible, donc un
    // report sans fin plutôt qu'un refus de parenté.
    if !repo.contains('/') {
        return None;
    }
    let issue = number.trim().parse::<u64>().ok()?;
    Some((repo.to_string(), issue))
}

// ───────────────────── Le prédicat (D2), pur ─────────────────────

/// Un événement de timeline portant le label `ready`.
///
/// Seuls `labeled` et `unlabeled` entrent ici : ce sont les deux seuls
/// événements qui changent l'état du label, et le prédicat lit un état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadyLabelEvent {
    /// `labeled` ou `unlabeled`.
    pub(crate) kind: ReadyLabelEventKind,
    /// Login de l'acteur, tel que GitHub le rend (`mika-platform-dev[bot]`).
    /// `None` quand l'acteur est absent de la réponse (compte supprimé).
    pub(crate) actor: Option<String>,
    /// Horodatage ISO 8601 de l'événement.
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadyLabelEventKind {
    Labeled,
    Unlabeled,
}

/// L'ensemble des identités machine, normalisées (D3).
///
/// Normalisation : minuscules, suffixe `[bot]` retiré. GitHub rend
/// `mika-platform-dev[bot]` là où la configuration porte `mika-platform-dev`,
/// et l'évidence du ticket cite les deux formes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MachineIdentities {
    logins: BTreeSet<String>,
}

impl MachineIdentities {
    /// Construit l'ensemble depuis des logins bruts. Les entrées vides après
    /// normalisation sont ignorées — un `MIKA_LOOP_BOT_LOGINS=a,,b` ne doit pas
    /// faire de la chaîne vide une identité machine qui matcherait un acteur
    /// absent.
    pub(crate) fn from_logins<I, S>(logins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            logins: logins
                .into_iter()
                .map(|l| normalize_login(l.as_ref()))
                .filter(|l| !l.is_empty())
                .collect(),
        }
    }

    /// L'ensemble est-il vide ? Un ensemble vide est fail-closed (D3).
    pub(crate) fn is_empty(&self) -> bool {
        self.logins.is_empty()
    }

    /// Cet acteur est-il une identité machine ?
    ///
    /// Un acteur absent (`None`) n'est **jamais** machine : GitHub omet l'acteur
    /// quand le compte a été supprimé, et traiter l'absence comme une identité
    /// connue rendrait un retrait anonyme non-parqué.
    pub(crate) fn is_machine(&self, actor: Option<&str>) -> bool {
        match actor {
            Some(a) => self.logins.contains(&normalize_login(a)),
            None => false,
        }
    }

    /// Les logins normalisés, pour la journalisation au démarrage.
    fn as_sorted_vec(&self) -> Vec<&str> {
        self.logins.iter().map(String::as_str).collect()
    }
}

/// Minuscules + retrait du suffixe `[bot]` + trim.
///
/// `pub(crate)` depuis mika#2334 : `qa_review_reconcile` compare les mêmes
/// identités machine (auteur de PR, relecteur demandé) et doit lire les deux
/// formes exactement comme ici. Une seconde normalisation écrite à la main
/// dériverait de celle-ci le jour où GitHub change de rendu — et la dérive
/// serait silencieuse des deux côtés.
pub(crate) fn normalize_login(login: &str) -> String {
    let trimmed = login.trim().to_ascii_lowercase();
    trimmed
        .strip_suffix("[bot]")
        .unwrap_or(&trimmed)
        .trim()
        .to_string()
}

/// Le prédicat de park (D2). **Pur** — entièrement testable sans réseau.
///
/// `events` doit être ordonné par `created_at` croissant : c'est l'ordre dans
/// lequel l'API timeline les rend, et [`read_ready_label_timeline`] le préserve.
pub(crate) fn is_parked(events: &[ReadyLabelEvent], machine: &MachineIdentities) -> bool {
    let Some(last) = events.last() else {
        return false;
    };
    if last.kind == ReadyLabelEventKind::Labeled {
        return false;
    }
    !machine.is_machine(last.actor.as_deref())
}

/// L'horodatage du **dernier** `labeled(ready)` de la timeline complète (AC5).
///
/// C'est la moitié B1 du correctif : la même lecture sert le park et l'âge, donc
/// l'âge est désormais celui du dernier `labeled` de *toute* la timeline, et non
/// celui du dernier `labeled` des cent premiers événements.
pub(crate) fn last_ready_labeled_at(events: &[ReadyLabelEvent]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|e| e.kind == ReadyLabelEventKind::Labeled)
        .map(|e| e.created_at.as_str())
}

// ───────────────────── La lecture paginée (D5) ─────────────────────

/// Pourquoi une lecture de timeline n'a pas abouti. Chaque variante est une
/// cause distincte parce que chacune appelle une remédiation distincte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TimelineReadError {
    /// L'appel `gh` a échoué (réseau, auth, issue absente).
    Api(String),
    /// La réponse n'est pas le tableau JSON attendu.
    Parse(String),
    /// Le plafond [`MAX_TIMELINE_PAGES`] a été atteint sans voir la fin.
    Truncated { pages_read: u32 },
}

impl std::fmt::Display for TimelineReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(e) => write!(f, "timeline API error: {e}"),
            Self::Parse(e) => write!(f, "timeline parse error: {e}"),
            Self::Truncated { pages_read } => write!(
                f,
                "timeline exceeds the {pages_read}-page cap ({} events); refusing rather than \
                 deciding on a partial read",
                pages_read * TIMELINE_PER_PAGE
            ),
        }
    }
}

/// Forme minimale d'un événement de timeline, côté désérialisation.
#[derive(Deserialize)]
struct RawTimelineEvent {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    label: Option<RawLabel>,
    #[serde(default)]
    actor: Option<RawActor>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct RawLabel {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RawActor {
    #[serde(default)]
    login: Option<String>,
}

/// Le verdict d'une page : reste-t-il des pages à lire ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageVerdict {
    /// La page était pleine — il peut en rester.
    More,
    /// La page était incomplète — c'était la dernière.
    Last,
}

/// Extrait les événements `ready` d'une page et dit si la timeline continue.
fn parse_timeline_page(
    json: &str,
    out: &mut Vec<ReadyLabelEvent>,
) -> Result<PageVerdict, TimelineReadError> {
    let raw: Vec<RawTimelineEvent> =
        serde_json::from_str(json).map_err(|e| TimelineReadError::Parse(e.to_string()))?;
    let raw_len = raw.len() as u32;

    for e in raw {
        let kind = match e.event.as_deref() {
            Some("labeled") => ReadyLabelEventKind::Labeled,
            Some("unlabeled") => ReadyLabelEventKind::Unlabeled,
            _ => continue,
        };
        // Les événements d'un autre label sont ignorés — le prédicat lit l'état
        // de `ready`, pas l'activité de l'issue.
        if e.label.and_then(|l| l.name).as_deref() != Some(READY_LABEL) {
            continue;
        }
        // Un événement sans `created_at` ne peut pas être ordonné ni daté. Il
        // est ignoré plutôt que placé arbitrairement : l'API en produit toujours
        // un, et inventer une position ferait décider le prédicat sur une
        // chronologie fabriquée.
        let Some(created_at) = e.created_at else {
            continue;
        };
        out.push(ReadyLabelEvent {
            kind,
            actor: e.actor.and_then(|a| a.login),
            created_at,
        });
    }

    Ok(if raw_len < TIMELINE_PER_PAGE {
        PageVerdict::Last
    } else {
        PageVerdict::More
    })
}

/// Le fournisseur de pages de timeline.
///
/// L'indirection existe pour que [`read_ready_label_timeline_with`] — donc la
/// règle d'arrêt, le plafond et l'accumulation, c'est-à-dire tout ce que B1 a
/// cassé — soit testable sans réseau. La production n'a qu'une implémentation,
/// [`GhTimelineFetcher`].
#[async_trait::async_trait]
pub(crate) trait TimelinePageFetcher: Send + Sync {
    async fn fetch(&self, repo: &str, issue: u64, page: u32) -> Result<String, String>;
}

/// Le fournisseur de production : `gh api …/timeline?per_page=100&page=N`.
pub(crate) struct GhTimelineFetcher<'a> {
    token: &'a str,
}

impl<'a> GhTimelineFetcher<'a> {
    pub(crate) fn new(token: &'a str) -> Self {
        Self { token }
    }
}

#[async_trait::async_trait]
impl TimelinePageFetcher for GhTimelineFetcher<'_> {
    async fn fetch(&self, repo: &str, issue: u64, page: u32) -> Result<String, String> {
        let output = run_gh(
            self.token,
            GhSubject::Issue(issue),
            &[
                "api",
                &format!(
                    "repos/{repo}/issues/{issue}/timeline?per_page={TIMELINE_PER_PAGE}&page={page}"
                ),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Lit la timeline **en entier**, plafonnée à [`MAX_TIMELINE_PAGES`] (D5).
///
/// Pourquoi une pagination explicite et non `gh api --paginate` : `--paginate`
/// n'offre aucun moyen de s'arrêter à la N-ième page, donc il ne peut pas
/// honorer le plafond — et un plafond est ce qui empêche ce prédicat de devenir
/// un amplificateur d'appels API sur un ticket pathologique. La boucle explicite
/// rend aussi la règle d'arrêt lisible et testable, ce qui est précisément la
/// chose que B1 avait fausse.
pub(crate) async fn read_ready_label_timeline(
    token: &str,
    repo: &str,
    issue: u64,
) -> Result<Vec<ReadyLabelEvent>, TimelineReadError> {
    read_ready_label_timeline_with(&GhTimelineFetcher::new(token), repo, issue).await
}

/// Cœur de [`read_ready_label_timeline`], paramétré par son fournisseur.
pub(crate) async fn read_ready_label_timeline_with<F: TimelinePageFetcher + ?Sized>(
    fetcher: &F,
    repo: &str,
    issue: u64,
) -> Result<Vec<ReadyLabelEvent>, TimelineReadError> {
    let mut events = Vec::new();
    for page in 1..=MAX_TIMELINE_PAGES {
        let json = fetcher
            .fetch(repo, issue, page)
            .await
            .map_err(TimelineReadError::Api)?;
        if parse_timeline_page(&json, &mut events)? == PageVerdict::Last {
            return Ok(events);
        }
    }
    Err(TimelineReadError::Truncated {
        pages_read: MAX_TIMELINE_PAGES,
    })
}

/// Le verdict de parc d'une issue, en une lecture (mika#2995).
///
/// `Ok(None)` = non parqué. `Ok(Some((parked_by, parked_at)))` = parqué, avec
/// l'acteur et l'instant du retrait, pour que l'appelant les journalise **sans
/// relire**. `Err` = la timeline ne se lit pas (D4), et c'est à l'appelant de
/// décider ; ici le refus est une valeur retournée, jamais un échec avalé
/// (leçon mika#2199).
///
/// Ce composeur existe pour que la garde du dispatcher ne recopie pas la
/// séquence `read_ready_label_timeline_with` → `is_parked` → `events.last()`.
/// Trois lignes recopiées, c'est trois lignes qui dérivent le jour où `is_parked`
/// change de contrat.
pub(crate) async fn park_verdict_for_issue_with<F: TimelinePageFetcher + ?Sized>(
    fetcher: &F,
    repo: &str,
    issue: u64,
    machine: &MachineIdentities,
) -> Result<Option<(Option<String>, String)>, TimelineReadError> {
    let events = read_ready_label_timeline_with(fetcher, repo, issue).await?;
    if !is_parked(&events, machine) {
        return Ok(None);
    }
    let last = events.last().expect("is_parked is false on an empty set");
    Ok(Some((last.actor.clone(), last.created_at.clone())))
}

// ────────── L'état du ticket avant un ré-armement (mika#3050 D2) ──────────

/// Ce que la forge répond quand on relit un ticket juste avant de **ré-armer**
/// une parente `ready-label` (mika#3050 AC1).
///
/// Trois états, et un quatrième que ce type ne porte **pas** : la relecture en
/// erreur, qui est le `Err` du `Result`. Une panne de forge n'est pas un état du
/// ticket, et l'écrire comme tel ferait classer une indisponibilité en retrait
/// de consentement — donc poserait un terminal sur une parente parfaitement
/// consentante (mika#2131, mika#2331 : une absence n'est pas un zéro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RearmConsent {
    /// Ouvert, et portant `ready` : le consentement tient, le ré-armement suit.
    Held,
    /// Ouvert, mais `ready` n'y est plus — retiré depuis la mise en file.
    ReadyAbsent,
    /// Fermé depuis la mise en file.
    Closed,
}

/// Relit l'état d'un ticket à la forge : ouvert, et portant `ready` ? (D2)
///
/// Frère du prédicat de parc ci-dessus, et volontairement **pas le même** : le
/// parc lit la *timeline* pour savoir **qui** a retiré `ready` et **quand** ;
/// ici on ne lit que l'**état courant**, parce que la question qu'AC1 pose est
/// « le consentement tient-il à cet instant ? » et non « qui l'a retiré ? ». Une
/// timeline de 20 pages pour répondre à une question d'état serait un
/// amplificateur d'appels API sur chaque tick de reaper.
///
/// Pourquoi pas `server::ready_label_handler::fetch_issue_body_and_labels_via_gh`,
/// que le ticket cite : il ne rend **pas** `state`, donc il ne peut pas répondre
/// à la moitié « ouvert » du prédicat ; et il vit dans un module de handler HTTP,
/// que le moteur de tâches n'a aucune raison d'atteindre.
///
/// **Fail-closed champ par champ** (D3). Tout ce qui n'est pas lisible selon le
/// contrat rend `Err` plutôt qu'un état : `state` absent ou hors du couple
/// `OPEN`/`CLOSED`, `labels` absent ou non-tableau. En particulier `labels`
/// absent n'est **jamais** [`RearmConsent::ReadyAbsent`] — « le champ n'est pas
/// là » et « le label n'est pas posé » sont deux faits distincts, et les
/// confondre terminerait une parente sur une réponse tronquée.
pub(crate) async fn reread_issue_consent_via_gh(
    token: &str,
    owner_repo: &str,
    number: u64,
) -> Result<RearmConsent, String> {
    let output = run_gh(
        token,
        GhSubject::Issue(number),
        &[
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            owner_repo,
            "--json",
            "state,labels",
        ],
    )
    .await
    .map_err(|e| format!("gh issue view ne s'est pas lancé pour {owner_repo}#{number} : {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh issue view a échoué pour {owner_repo}#{number} : {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_rearm_consent(&String::from_utf8_lossy(&output.stdout), owner_repo, number)
}

/// Le cœur de [`reread_issue_consent_via_gh`] : la lecture du JSON, sans réseau.
///
/// Séparé du fetch pour la raison qui a fait séparer [`parse_timeline_page`] :
/// c'est ici que vit le contrat fail-closed, donc c'est ici que les cas se
/// testent — un par champ, sans passer par un double d'appel `gh`.
fn parse_rearm_consent(
    stdout: &str,
    owner_repo: &str,
    number: u64,
) -> Result<RearmConsent, String> {
    let raw: serde_json::Value = serde_json::from_str(stdout).map_err(|e| {
        format!("réponse de gh issue view illisible pour {owner_repo}#{number} : {e}")
    })?;
    let state = raw["state"]
        .as_str()
        .ok_or_else(|| format!("pas de champ `state` pour {owner_repo}#{number}"))?;
    if state.eq_ignore_ascii_case("CLOSED") {
        return Ok(RearmConsent::Closed);
    }
    if !state.eq_ignore_ascii_case("OPEN") {
        return Err(format!(
            "`state` hors contrat pour {owner_repo}#{number} : {state:?}"
        ));
    }
    let labels = raw["labels"]
        .as_array()
        .ok_or_else(|| format!("pas de tableau `labels` pour {owner_repo}#{number}"))?;
    if labels
        .iter()
        .any(|l| l["name"].as_str() == Some(READY_LABEL))
    {
        Ok(RearmConsent::Held)
    } else {
        Ok(RearmConsent::ReadyAbsent)
    }
}

// ───────────────────── Les identités machine (D3) ─────────────────────

/// Cache process des identités machine résolues.
static MACHINE_IDENTITIES: OnceLock<MachineIdentities> = OnceLock::new();

/// Résout l'ensemble des identités machine, une fois par process (D3).
///
/// Trois sources cumulatives, jamais exclusives :
///
/// 1. `MIKA_GITHUB_APP_LOGIN` — le login du bot App, variable déjà existante ;
/// 2. le login du token courant (`gh api user -q .login`) — ce qui couvre le
///    PAT machine, que rien d'autre ne nomme ;
/// 3. `MIKA_LOOP_BOT_LOGINS` — liste séparée par virgules, pour les identités
///    qu'aucune des deux premières ne révèle (l'évidence du ticket en cite deux,
///    `mika-platform-dev` et `mika-platform-bot`, et une seule App est
///    configurée).
///
/// Un ensemble vide est fail-closed : voir l'en-tête du module.
pub(crate) async fn machine_identities(token: &str) -> &'static MachineIdentities {
    if let Some(cached) = MACHINE_IDENTITIES.get() {
        return cached;
    }

    let mut logins: Vec<String> = Vec::new();
    if let Ok(v) = std::env::var(GITHUB_APP_LOGIN_ENV) {
        logins.push(v);
    }
    if let Ok(v) = std::env::var(LOOP_BOT_LOGINS_ENV) {
        logins.extend(v.split(',').map(str::to_string));
    }
    match gh_current_login(token).await {
        Ok(Some(login)) => logins.push(login),
        Ok(None) => {}
        Err(e) => warn!(
            error = %e,
            "ready_label: could not resolve the current token's login"
        ),
    }

    let resolved = MachineIdentities::from_logins(logins);
    if resolved.is_empty() {
        warn!(
            event = "ready_machine_identities_unresolved",
            app_login_env = GITHUB_APP_LOGIN_ENV,
            extra_env = LOOP_BOT_LOGINS_ENV,
            "ready_label: no machine identity resolved — every `ready` application will be \
             refused (fail-closed). Set MIKA_GITHUB_APP_LOGIN, or list the loop's bot logins in \
             MIKA_LOOP_BOT_LOGINS."
        );
    } else {
        info!(
            event = "ready_machine_identities_resolved",
            logins = ?resolved.as_sorted_vec(),
            "ready_label: machine identities resolved"
        );
    }

    // `set` peut perdre une course entre deux premiers appels concurrents ; les
    // deux valeurs sont résolues depuis les mêmes sources, donc la gagnante est
    // la bonne quoi qu'il arrive.
    let _ = MACHINE_IDENTITIES.set(resolved);
    MACHINE_IDENTITIES
        .get()
        .expect("MACHINE_IDENTITIES is set above")
}

/// Le login du compte auquel appartient le token courant.
async fn gh_current_login(token: &str) -> Result<Option<String>, String> {
    let output = run_gh(token, GhSubject::Repo, &["api", "user", "-q", ".login"])
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if login.is_empty() { None } else { Some(login) })
}

// ───────────────────── L'écriture (D1) ─────────────────────

/// Comment écrire le label, selon le chemin appelant.
///
/// Les deux formes existaient avant ce module et sont conservées telles quelles :
/// unifier le transport aurait mélangé deux changements dont un seul est demandé,
/// et le `LabelWriteToken` d'`auto_pull` (mika#2228) porte une politique de
/// provenance que la cascade milestone n'a pas.
pub(crate) enum ReadyWriteAuth<'a> {
    /// `gh issue edit --add-label` sous un token d'écriture de label résolu
    /// App-first (les trois sites d'`auto_pull`).
    Cli(&'a LabelWriteToken),
    /// `POST /repos/{owner}/{repo}/issues/{n}/labels` sous le token identitaire
    /// (la cascade milestone).
    Rest(&'a str),
}

/// Ce qu'une tentative d'application de `ready` a produit.
///
/// Le refus est une **valeur retournée**, jamais un échec avalé — leçon
/// mika#2199, où un `gh` en échec silencieux a re-élu la même PR dix-sept fois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadyApplyOutcome {
    /// Le label a été appliqué.
    Applied,
    /// Le ticket est parqué : `ready` en a été retiré par un acteur hors des
    /// identités machine, et rien ne l'a remis depuis.
    RefusedParked,
    /// Le park n'a pas pu être évalué (timeline illisible, pagination
    /// incomplète, identités machine non résolues). Rien n'a été écrit.
    RefusedUnreadable { reason: String },
    /// Le park autorisait l'écriture, mais l'écriture elle-même a échoué.
    /// Distinct d'un refus : l'appelant l'impute à son compteur de pannes, là
    /// où un refus est une décision et n'en est pas une.
    WriteFailed { error: String },
}

/// Tout ce qu'il faut savoir pour appliquer `ready` à une issue.
pub(crate) struct ReadyApplyRequest<'a> {
    /// `owner/repo`.
    pub(crate) repo: &'a str,
    pub(crate) issue: u64,
    /// Token de **lecture** (timeline, identité du token courant).
    pub(crate) read_token: &'a str,
    /// Comment écrire le label.
    pub(crate) write: ReadyWriteAuth<'a>,
    /// La phase appelante, telle qu'elle apparaîtra dans la ligne d'audit
    /// (`phase0_feeder`, `phase1_idle_pull`, `phase2_stuck_rescue`,
    /// `milestone_phase_cascade`).
    pub(crate) caller: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) trace_id: Option<&'a str>,
}

/// L'unique écriture du label `ready` (D1, AC6).
///
/// Consulte le park, puis délègue l'écriture. Lit la timeline elle-même ; les
/// appelants qui en tiennent déjà une (la Phase 2, qui l'a lue pour l'âge)
/// passent par [`apply_ready_with_timeline`] et ne paient **aucun appel
/// supplémentaire**.
pub(crate) async fn apply_ready(
    db: &AsyncDatabase,
    req: ReadyApplyRequest<'_>,
) -> ReadyApplyOutcome {
    let events = match read_ready_label_timeline(req.read_token, req.repo, req.issue).await {
        Ok(events) => events,
        Err(e) => {
            return refuse_unreadable(db, &req, e.to_string()).await;
        }
    };
    apply_ready_with_timeline(db, req, &events).await
}

/// [`apply_ready`] sur une timeline déjà lue.
pub(crate) async fn apply_ready_with_timeline(
    db: &AsyncDatabase,
    req: ReadyApplyRequest<'_>,
    events: &[ReadyLabelEvent],
) -> ReadyApplyOutcome {
    let machine = machine_identities(req.read_token).await;
    if machine.is_empty() {
        return refuse_unreadable(
            db,
            &req,
            "machine identities unresolved (fail-closed)".to_string(),
        )
        .await;
    }

    if is_parked(events, machine) {
        let last = events.last().expect("is_parked is false on an empty set");
        warn!(
            event = "ready_apply_refused_parked",
            issue = req.issue,
            repo = req.repo,
            caller = req.caller,
            parked_by = last.actor.as_deref().unwrap_or("<unknown>"),
            parked_at = %last.created_at,
            "ready_label: refusing to apply `ready` — it was removed by a non-machine actor and \
             has not been re-applied. Re-add `ready` by hand to lift the park."
        );
        if let Err(e) = db
            .log_audit_event(
                req.session_id,
                READY_LABEL_PARK_TOOL_NAME,
                &format!("issue:{}", req.issue),
                None,
                Some(req.caller),
                Some(&format!(
                    "parked by {} at {}",
                    last.actor.as_deref().unwrap_or("<unknown>"),
                    last.created_at
                )),
                req.trace_id,
            )
            .await
        {
            warn!(error = %e, "ready_label: failed to write park audit event");
        }
        return ReadyApplyOutcome::RefusedParked;
    }

    match write_ready_label(&req).await {
        Ok(()) => ReadyApplyOutcome::Applied,
        Err(error) => ReadyApplyOutcome::WriteFailed { error },
    }
}

/// Le seul site d'écriture du littéral `ready` vers les deux transports.
async fn write_ready_label(req: &ReadyApplyRequest<'_>) -> Result<(), String> {
    match req.write {
        ReadyWriteAuth::Cli(label_auth) => {
            crate::auto_pull::gh_apply_label(label_auth, req.issue, READY_LABEL)
                .await
                .map_err(|e| e.to_string())
        }
        ReadyWriteAuth::Rest(token) => {
            let (owner, repo) = req
                .repo
                .split_once('/')
                .ok_or_else(|| format!("malformed repo slug: {}", req.repo))?;
            crate::github_graphql::add_label_to_issue(token, owner, repo, req.issue, READY_LABEL)
                .await
        }
    }
}

/// Journalise et rend le refus D4/D3. **Attendu à zéro** en régime nominal : une
/// série soutenue signifie que la boucle est bridée par un problème d'API, pas
/// par un park.
async fn refuse_unreadable(
    db: &AsyncDatabase,
    req: &ReadyApplyRequest<'_>,
    reason: String,
) -> ReadyApplyOutcome {
    warn!(
        event = "ready_apply_refused_unreadable",
        issue = req.issue,
        repo = req.repo,
        caller = req.caller,
        reason = %reason,
        "ready_label: cannot evaluate the park — refusing to apply `ready` (a false positive \
         costs an Opus groom and breaks an operator STOP; a false negative costs one tick)"
    );
    if let Err(e) = db
        .log_audit_event(
            req.session_id,
            READY_LABEL_PARK_TOOL_NAME,
            &format!("issue:{}", req.issue),
            None,
            Some(req.caller),
            Some(&format!("refused_unreadable: {reason}")),
            req.trace_id,
        )
        .await
    {
        warn!(error = %e, "ready_label: failed to write unreadable-refusal audit event");
    }
    ReadyApplyOutcome::RefusedUnreadable { reason }
}

/// Les fixtures de timeline, partagées par les deux modules qui en ont besoin.
///
/// Promues ici par mika#2995 (tranche du 2026-11-03) depuis le `mod tests`
/// voisin, où elles vivaient depuis mika#2315 : la garde du dispatcher injecte
/// une timeline par le même trait [`TimelinePageFetcher`], donc elle a besoin
/// des mêmes gabarits JSON. **Un seul site de fixture de timeline pour la
/// crate** — un second gabarit recopié dériverait du premier le jour où la
/// forme de l'API change, et les deux modules croiraient tester la même chose.
#[cfg(test)]
pub(crate) mod timeline_fixtures {
    use super::TimelinePageFetcher;

    /// Un fournisseur de pages fixées.
    pub(crate) struct FixturePages(pub(crate) Vec<String>);

    #[async_trait::async_trait]
    impl TimelinePageFetcher for FixturePages {
        async fn fetch(&self, _repo: &str, _issue: u64, page: u32) -> Result<String, String> {
            Ok(self
                .0
                .get(page as usize - 1)
                .cloned()
                .unwrap_or_else(|| "[]".to_string()))
        }
    }

    /// Construit une page de `n` événements `commented` suivis des `extras`.
    pub(crate) fn page_of(filler: usize, extras: &[&str]) -> String {
        let mut items: Vec<String> = (0..filler)
            .map(|i| {
                format!(
                    r#"{{"event":"commented","actor":{{"login":"someone"}},"created_at":"2026-09-01T00:{:02}:00Z"}}"#,
                    i % 60
                )
            })
            .collect();
        items.extend(extras.iter().map(|s| s.to_string()));
        format!("[{}]", items.join(","))
    }

    pub(crate) fn labeled_json(actor: &str, at: &str) -> String {
        format!(
            r#"{{"event":"labeled","label":{{"name":"ready"}},"actor":{{"login":"{actor}"}},"created_at":"{at}"}}"#
        )
    }

    /// Le frère de [`labeled_json`] — le retrait, celui que la garde mika#2995
    /// doit voir en dernier pour refuser un tour.
    pub(crate) fn unlabeled_json(actor: &str, at: &str) -> String {
        format!(
            r#"{{"event":"unlabeled","label":{{"name":"ready"}},"actor":{{"login":"{actor}"}},"created_at":"{at}"}}"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::timeline_fixtures::*;
    use super::*;

    fn machine() -> MachineIdentities {
        MachineIdentities::from_logins(["mika-platform-dev", "mika-platform-bot"])
    }

    fn ev(kind: ReadyLabelEventKind, actor: &str, at: &str) -> ReadyLabelEvent {
        ReadyLabelEvent {
            kind,
            actor: Some(actor.to_string()),
            created_at: at.to_string(),
        }
    }

    fn labeled(actor: &str, at: &str) -> ReadyLabelEvent {
        ev(ReadyLabelEventKind::Labeled, actor, at)
    }

    fn unlabeled(actor: &str, at: &str) -> ReadyLabelEvent {
        ev(ReadyLabelEventKind::Unlabeled, actor, at)
    }

    // ── Tests 1–10 : le prédicat de park (D2), pur ──

    /// Test 1 — l'exigence littérale du ticket.
    #[test]
    fn last_event_unlabeled_by_a_human_is_parked() {
        let events = vec![
            labeled("mika-platform-dev", "2026-09-15T01:00:00Z"),
            unlabeled("samidarko", "2026-09-15T02:00:00Z"),
        ];
        assert!(is_parked(&events, &machine()));
    }

    /// Test 2 — le remove→add de la Phase 2 ne se parque pas lui-même.
    #[test]
    fn last_event_unlabeled_by_the_machine_is_not_parked() {
        let events = vec![
            labeled("mika-platform-dev", "2026-09-15T01:00:00Z"),
            unlabeled("mika-platform-dev", "2026-09-15T02:29:45Z"),
        ];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 3 — la sortie de park est un geste unique : remettre `ready`.
    #[test]
    fn last_event_labeled_by_a_human_lifts_the_park() {
        let events = vec![
            unlabeled("samidarko", "2026-09-15T02:00:00Z"),
            labeled("samidarko", "2026-09-15T03:00:00Z"),
        ];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 4.
    #[test]
    fn last_event_labeled_by_the_machine_is_not_parked() {
        let events = vec![labeled("mika-platform-dev", "2026-09-15T02:29:47Z")];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 5 — un ticket qui n'a jamais porté `ready` n'est pas parqué.
    #[test]
    fn no_ready_event_at_all_is_not_parked() {
        assert!(!is_parked(&[], &machine()));
    }

    /// Test 6 — l'ordre décide, pas la présence.
    #[test]
    fn human_unlabel_then_human_label_is_not_parked() {
        let events = vec![
            unlabeled("samidarko", "2026-09-15T02:00:00Z"),
            labeled("someone-else", "2026-09-15T03:00:00Z"),
        ];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 7 — le miroir du test 6.
    #[test]
    fn human_label_then_human_unlabel_is_parked() {
        let events = vec![
            labeled("someone-else", "2026-09-15T02:00:00Z"),
            unlabeled("samidarko", "2026-09-15T03:00:00Z"),
        ];
        assert!(is_parked(&events, &machine()));
    }

    /// Test 8 — GitHub rend `mika-platform-dev[bot]` là où la configuration
    /// porte `mika-platform-dev`. L'évidence du ticket cite les deux formes.
    #[test]
    fn bot_suffix_is_stripped_before_comparison() {
        let events = vec![unlabeled("mika-platform-dev[bot]", "2026-09-15T02:00:00Z")];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 9 — comparaison insensible à la casse.
    #[test]
    fn login_comparison_is_case_insensitive() {
        let events = vec![unlabeled("MIKA-Platform-Bot[BOT]", "2026-09-15T02:00:00Z")];
        assert!(!is_parked(&events, &machine()));
    }

    /// Test 10 — les événements d'un **autre** label sont ignorés. Le filtrage
    /// vit dans [`parse_timeline_page`] : ce test l'exerce là où il est.
    #[test]
    fn events_of_another_label_are_ignored() {
        let json = r#"[
            {"event":"labeled","label":{"name":"ready"},"actor":{"login":"mika-platform-dev"},"created_at":"2026-09-15T01:00:00Z"},
            {"event":"unlabeled","label":{"name":"blocked"},"actor":{"login":"samidarko"},"created_at":"2026-09-15T02:00:00Z"},
            {"event":"labeled","label":{"name":"blocked"},"actor":{"login":"samidarko"},"created_at":"2026-09-15T03:00:00Z"},
            {"event":"commented","actor":{"login":"samidarko"},"created_at":"2026-09-15T04:00:00Z"}
        ]"#;
        let mut events = Vec::new();
        assert_eq!(
            parse_timeline_page(json, &mut events).unwrap(),
            PageVerdict::Last
        );
        assert_eq!(events.len(), 1, "seul l'événement `ready` compte");
        assert!(!is_parked(&events, &machine()));
    }

    /// Un acteur absent n'est jamais machine : traiter l'absence comme une
    /// identité connue rendrait un retrait anonyme non-parqué.
    #[test]
    fn a_missing_actor_is_never_machine() {
        let events = vec![ReadyLabelEvent {
            kind: ReadyLabelEventKind::Unlabeled,
            actor: None,
            created_at: "2026-09-15T02:00:00Z".to_string(),
        }];
        assert!(is_parked(&events, &machine()));
    }

    /// Une entrée vide dans `MIKA_LOOP_BOT_LOGINS` ne doit pas devenir une
    /// identité machine — elle matcherait alors un acteur normalisé vide.
    #[test]
    fn empty_logins_are_dropped_from_the_identity_set() {
        let m = MachineIdentities::from_logins(["", "  ", "[bot]"]);
        assert!(m.is_empty());
    }

    // ── Test 14 : la garde D3 ──

    /// Test 14 — un ensemble d'identités machine vide rend le prédicat
    /// inutilisable, et le module refuse (fail-closed). On teste ici le maillon
    /// pur : sans identité machine, **tout** retrait se lit comme un park, ce
    /// qui est exactement pourquoi `apply_ready` refuse au lieu d'appliquer.
    #[test]
    fn empty_machine_identities_make_every_unlabel_look_like_a_park() {
        let empty = MachineIdentities::default();
        assert!(empty.is_empty());
        let events = vec![unlabeled("mika-platform-dev", "2026-09-15T02:29:45Z")];
        assert!(
            is_parked(&events, &empty),
            "sans identité machine, le remove→add de la Phase 2 se lirait comme un park — \
             d'où le fail-closed de `apply_ready` plutôt qu'une application"
        );
    }

    // ── mika#2995 — le discriminant de parente et le composeur de verdict ──

    /// **L'aller-retour producteur → parseur.**
    ///
    /// La forme composée ici est celle de `server/ready_label_handler.rs`, qui
    /// compose par [`READY_LABEL_TASK_PREFIX`] depuis mika#2995. C'est ce test
    /// qui rend le « structurellement sans population » du doc-comment de
    /// [`parse_ready_label_task_label`] vérifiable plutôt que postulé : si la
    /// forme du producteur dérivait, le parseur rendrait `None` et la garde
    /// cesserait silencieusement de reconnaître ses parentes.
    #[test]
    fn mika2995_le_label_du_producteur_fait_laller_retour() {
        let produit = format!(
            "{READY_LABEL_TASK_PREFIX} {}#{}",
            "senara-solutions/mika", 3002
        );
        assert_eq!(
            parse_ready_label_task_label(&produit),
            Some(("senara-solutions/mika".to_string(), 3002)),
            "le parseur doit lire exactement ce que le producteur compose"
        );
    }

    /// Les quatre formes que le discriminant refuse, chacune pour sa raison.
    #[test]
    fn mika2995_une_parente_hors_prefixe_ou_malformee_nest_pas_un_ready_label() {
        for (label, pourquoi) in [
            ("manual self_dev parent", "aucun préfixe — le cas d'AC-T3"),
            (
                "long_running:run_claude_pilot_groom",
                "une parente de groom",
            ),
            (
                "ready-label: mika#3002",
                "repo nu : l'URL d'API serait fausse, et son 404 se lirait comme \
                 une timeline illisible",
            ),
            (
                "ready-label: senara-solutions/mika#trois",
                "numéro d'issue illisible",
            ),
        ] {
            assert_eq!(
                parse_ready_label_task_label(label),
                None,
                "`{label}` ne doit pas être lu comme une parente `ready-label` ({pourquoi})"
            );
        }
    }

    /// Le composeur rend l'acteur et l'instant du retrait, **sans relire**.
    #[tokio::test]
    async fn mika2995_le_composeur_rend_lacteur_et_linstant_du_retrait() {
        let pages = FixturePages(vec![page_of(
            0,
            &[
                &labeled_json("samidarko", "2026-11-01T13:00:00Z"),
                &unlabeled_json("samidarko", "2026-11-01T14:08:43Z"),
            ],
        )]);

        let verdict =
            park_verdict_for_issue_with(&pages, "senara-solutions/mika", 3002, &machine())
                .await
                .expect("la timeline est lisible");

        assert_eq!(
            verdict,
            Some((
                Some("samidarko".to_string()),
                "2026-11-01T14:08:43Z".to_string()
            )),
            "le parc doit porter qui a retiré et quand — l'appelant les journalise sans \
             payer une seconde lecture"
        );
    }

    /// Contrôle négatif du composeur, sur la même fixture à un événement près :
    /// un `labeled` final rend `Ok(None)`.
    #[tokio::test]
    async fn mika2995_le_composeur_rend_none_quand_le_consentement_tient() {
        let pages = FixturePages(vec![page_of(
            0,
            &[
                &labeled_json("samidarko", "2026-11-01T13:00:00Z"),
                &unlabeled_json("samidarko", "2026-11-01T14:08:43Z"),
                &labeled_json("samidarko", "2026-11-01T15:32:14Z"),
            ],
        )]);

        let verdict =
            park_verdict_for_issue_with(&pages, "senara-solutions/mika", 3002, &machine())
                .await
                .expect("la timeline est lisible");

        assert_eq!(verdict, None, "un `labeled` en dernier n'est pas un parc");
    }

    // ── Tests 11–12 : la pagination (B1) ──

    /// **Test 11 — la régression B1, nommée et épinglée.**
    ///
    /// L'API timeline rend ses événements en ordre chronologique **ascendant** :
    /// la page 1 porte les 100 plus **anciens**. Un lecteur mono-page rend donc
    /// l'âge d'un `labeled` ancien — toujours ≥ seuil, à chaque tick, pour
    /// toujours — et le self-throttle de la Phase 2 ne freine rien sur
    /// exactement la population qu'il existe pour freiner.
    ///
    /// Ce test a été exécuté contre `auto_pull::parse_last_ready_labeled_at`
    /// (le lecteur mono-page d'avant D5) et **échouait** : il rendait
    /// `2026-08-01T00:00:00Z`. Un test 11 vert avant le correctif n'aurait rien
    /// épinglé.
    #[tokio::test]
    async fn mika2315_the_last_labeled_of_the_whole_timeline_decides() {
        let old = labeled_json("mika-platform-dev", "2026-08-01T00:00:00Z");
        let recent = labeled_json("mika-platform-dev", "2026-09-15T03:29:47Z");
        let pages = FixturePages(vec![
            // Page 1 pleine : 99 événements de remplissage + le vieux `labeled`.
            page_of(99, &[&old]),
            // Page 2 : le `labeled` que la Phase 2 vient d'écrire.
            page_of(0, &[&recent]),
        ]);

        let events = read_ready_label_timeline_with(&pages, "senara-solutions/mika", 2295)
            .await
            .expect("la timeline doit être lisible");

        assert_eq!(
            last_ready_labeled_at(&events),
            Some("2026-09-15T03:29:47Z"),
            "le `labeled(ready)` de la dernière page est celui qui décide"
        );
    }

    /// Test 12 — le plafond rend un refus, jamais une lecture partielle.
    #[tokio::test]
    async fn mika2315_page_cap_refuses_rather_than_deciding_on_a_partial_read() {
        // Toutes les pages sont pleines : la fin n'est jamais atteinte.
        let full = page_of(100, &[]);
        let pages = FixturePages(vec![full; MAX_TIMELINE_PAGES as usize + 5]);

        let err = read_ready_label_timeline_with(&pages, "senara-solutions/mika", 2295)
            .await
            .expect_err("une timeline sans fin doit être refusée");

        assert_eq!(
            err,
            TimelineReadError::Truncated {
                pages_read: MAX_TIMELINE_PAGES
            },
            "une pagination incomplète est un refus (D4), pas une lecture partielle"
        );
    }

    /// Une page incomplète termine la lecture — la boucle ne dépense pas vingt
    /// appels API sur une issue de trois événements.
    #[tokio::test]
    async fn a_short_page_ends_the_read() {
        let pages = FixturePages(vec![page_of(
            0,
            &[&labeled_json("mika-platform-dev", "2026-09-15T02:29:47Z")],
        )]);
        let events = read_ready_label_timeline_with(&pages, "senara-solutions/mika", 1)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    /// Une réponse qui n'est pas un tableau JSON est un refus, pas une timeline
    /// vide — une timeline vide se lirait « non parqué » et appliquerait.
    #[tokio::test]
    async fn malformed_json_is_a_refusal_not_an_empty_timeline() {
        struct Garbage;
        #[async_trait::async_trait]
        impl TimelinePageFetcher for Garbage {
            async fn fetch(&self, _: &str, _: u64, _: u32) -> Result<String, String> {
                Ok("{\"message\":\"Not Found\"}".to_string())
            }
        }
        let err = read_ready_label_timeline_with(&Garbage, "senara-solutions/mika", 1)
            .await
            .expect_err("un objet JSON n'est pas une timeline");
        assert!(matches!(err, TimelineReadError::Parse(_)));
    }

    /// Un échec `gh` est un refus, pour la même raison.
    #[tokio::test]
    async fn an_api_error_is_a_refusal() {
        struct Failing;
        #[async_trait::async_trait]
        impl TimelinePageFetcher for Failing {
            async fn fetch(&self, _: &str, _: u64, _: u32) -> Result<String, String> {
                Err("gh: HTTP 503".to_string())
            }
        }
        let err = read_ready_label_timeline_with(&Failing, "senara-solutions/mika", 1)
            .await
            .expect_err("un 503 ne doit pas se lire comme une timeline vide");
        assert!(matches!(err, TimelineReadError::Api(_)));
    }

    // ── Test 13 : la garde structurelle D1 (AC6, AC11) ──

    /// **Test 13 — aucune écriture du label `ready` hors de ce module.**
    ///
    /// Échoue si une ligne passe le littéral `"ready"` à `gh_apply_label` ou à
    /// `add_label_to_issue` ailleurs qu'ici.
    ///
    /// # Pourquoi un scan de source et pas un test de comportement
    ///
    /// Un cinquième applicateur écrit demain ne rendrait **aucune décision
    /// fausse** : il la rendrait non gardée. Toutes les assertions de
    /// comportement resteraient vertes pendant que le park cesserait de tenir —
    /// c'est exactement la forme de mika#2158, où deux prédicats ont divergé des
    /// mois sans rien casser.
    ///
    /// # Disposition si ce détecteur fire (§ Fire-Disposition du plan)
    ///
    /// **Halt-and-surface, jamais d'allowlist, jamais de `#[ignore]`.** Une
    /// exception nommée ici serait un applicateur de `ready` qui continue
    /// d'écrire **sans consulter le park** — c'est-à-dire le défaut B2 laissé
    /// vivant derrière une ligne qui a l'air d'une décision. L'implémenteur
    /// s'arrête, nomme le site, et surface à l'opérateur dans le corps de PR
    /// sous un titre `Fire-Disposition: cinquième applicateur découvert`.
    ///
    /// # Forme
    ///
    /// `env!("CARGO_MANIFEST_DIR")` — lecture à l'exécution depuis un chemin
    /// absolu garanti par Cargo : aucun couplage de compilation avec les
    /// fichiers scannés, aucune dépendance au répertoire courant. Même forme que
    /// `grooming_marker::tests::no_grooming_regex_outside_this_module`.
    #[test]
    fn no_ready_label_write_outside_this_module() {
        const WRITERS: &[&str] = &["gh_apply_label", "add_label_to_issue"];

        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("ready_label.rs");

        let mut offenders = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this_module {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("la garde doit pouvoir lire {}: {e}", path.display())
                });
                scanned += 1;
                for (n, line) in content.lines().enumerate() {
                    let writes_ready = WRITERS.iter().any(|w| line.contains(w))
                        && (line.contains("\"ready\"") || line.contains("READY_LABEL"));
                    if writes_ready {
                        offenders.push(format!(
                            "{}:{}: {}",
                            path.strip_prefix(&src_root).unwrap_or(&path).display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }

        assert!(
            scanned > 0,
            "la garde n'a scanné aucun fichier — chemin cassé"
        );
        assert!(
            offenders.is_empty(),
            "mika#2315 — le label `ready` est écrit hors de `ready_label.rs`. Un applicateur qui \
             n'appelle pas `ready_label::apply_ready` n'interroge pas le park : un STOP opérateur \
             ne tiendrait plus, et rien ne casserait. Appelez `apply_ready`.\n{}",
            offenders.join("\n")
        );
    }

    // ── mika#2995 vol 2 — D1 : la surface d'outil n'atteint pas l'applicateur ──
    //
    // **Deux détecteurs et non un seul, et le test 13 reste intact.** Les trois
    // populations diffèrent, et deux populations sous un même nom ne se
    // soustraient pas (mika#2205) : le test 13 refuse une écriture de `ready`
    // hors de ce module, D1 refuse qu'une *surface d'outil* atteigne
    // l'applicateur même gouverné, D2 (plus bas) refuse qu'une écriture de forme
    // argv ne soit pas déclarée. Chacun porte sa propre disposition.

    /// Les quatre noms par lesquels `ready` s'écrit, l'applicateur unique de
    /// mika#2315 et les deux poseurs de label qu'il appelle.
    ///
    /// L'ordre est porteur : `apply_ready_with_timeline` avant `apply_ready`,
    /// pour qu'un offender soit nommé par le plus spécifique des deux.
    const READY_APPLICATORS: &[&str] = &[
        "apply_ready_with_timeline",
        "apply_ready",
        "gh_apply_label",
        "add_label_to_issue",
    ];

    /// `true` si `rel` (relatif à `src/`) appartient à **la surface depuis
    /// laquelle un tour LLM agit** : les handlers d'outils et les deux fichiers
    /// de la porte de dispatch de compétences.
    ///
    /// C'est la population de D1, et c'est aussi exactement celle où le défaut
    /// mesuré du 2026-10-31 vivait : `run_gh` est un handler de cette surface.
    fn est_surface_doutil(rel: &str) -> bool {
        rel.starts_with("tools/")
            || rel == "skills/builtin_handlers.rs"
            || rel == "skills/executor.rs"
    }

    /// Les **appels** à l'un des [`READY_APPLICATORS`] dans une moitié de
    /// production, rendus en `"<fichier>: <ligne>"`.
    ///
    /// # Appeler, et non nommer
    ///
    /// Trois formes nomment l'applicateur sans l'appeler, et les trois sont
    /// muettes, chacune pour une raison distincte :
    ///
    /// - **doc-comment et commentaire de bloc** — retirés par
    ///   [`crate::source_scan::strip_comment_lines`], dont le doc-comment porte
    ///   le prédicat et ses bornes. La prose *à propos* du geste n'est pas le
    ///   geste : c'est la forme du précédent mika#2425, et
    ///   `builtin_handlers.rs:3095` en porte une ;
    /// - **ligne de `use`** — muette par la parenthèse exigée : la syntaxe
    ///   `use` n'en porte jamais. C'est pour cela qu'il n'y a pas de branche
    ///   `starts_with("use ")` ici — une branche morte dirait au lecteur qu'un
    ///   import pourrait matcher ;
    /// - **ligne de définition** — écartée par le `fn <nom>(` : `pub(crate)
    ///   async fn apply_ready(` déclare l'applicateur, il ne l'appelle pas.
    ///
    /// # Pourquoi aucun numéro de ligne
    ///
    /// Le dépouillement des commentaires **décale** la numérotation, et un
    /// numéro faux dans un message d'échec envoie le lecteur sur une ligne
    /// innocente. Le texte de la ligne est rendu à la place : il est grep-able,
    /// et il est vrai.
    fn appels_a_lapplicateur(rel: &str, prod: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in crate::source_scan::strip_comment_lines(prod).lines() {
            for app in READY_APPLICATORS {
                if !line.contains(&format!("{app}(")) {
                    continue;
                }
                if line.contains(&format!("fn {app}(")) {
                    continue;
                }
                out.push(format!("{rel}: {}", line.trim()));
                break;
            }
        }
        out
    }

    /// **L'allowlist de D1, livrée VIDE — et elle le reste.**
    ///
    /// `(fichier, ticket)`. Elle est livrée vide **parce que la population est
    /// vide** : les quatre appelants de l'applicateur vivent dans `auto_pull.rs`
    /// (phases 0, 1 et 2) et dans `server/milestone_context_handler.rs` (cascade
    /// M4), aucun sous la surface d'outil.
    ///
    /// Quand D1 tire, on **retire l'appel** ; on n'ajoute pas une ligne ici. Une
    /// exception nommée serait un handler d'outil qui atteint l'applicateur
    /// « parce qu'il est gouverné » — or il est gouverné pour le **parc**, jamais
    /// pour le tour LLM : le parc dit *ce ticket est-il dé-promu*, il ne dit
    /// jamais *qui demande*. C'est la doctrine mika#2201 : quand un détecteur
    /// tire, on retire l'appel, on n'ajoute pas une ligne d'allowlist.
    const TOOL_SURFACE_APPLICATOR_EXCEPTIONS: &[(&str, &str)] = &[];

    /// **L'assertion sœur d'auto-vacuité de D1.**
    ///
    /// Une allowlist née vide est un emplacement où déposer la prochaine
    /// infraction. Celle-ci rougit à la première entrée, et c'est ce qui empêche
    /// D1 de se remplir en silence — le mode de mensonge d'une allowlist vide que
    /// la Fire-Disposition du plan nomme en premier.
    #[test]
    fn mika2995_lallowlist_de_surface_est_vide() {
        assert!(
            TOOL_SURFACE_APPLICATOR_EXCEPTIONS.is_empty(),
            "une exception a été ajoutée à D1 : {:?}. La disposition de ce détecteur est de \
             RETIRER l'appel, pas de l'allowlister (mika#2201). Un handler d'outil qui atteint \
             l'applicateur rend la voie B de mika#2995 fausse : le moteur retrouve, dans un tour \
             LLM, la capacité d'écrire le `ready` qui lève la garde de mika#2772.",
            TOOL_SURFACE_APPLICATOR_EXCEPTIONS
        );
    }

    /// **D1 — aucune surface d'outil n'atteint l'applicateur de `ready`
    /// (AC2 de mika#2995, voie B).**
    ///
    /// # Ce qu'il refuse
    ///
    /// Qu'un handler d'outil, demain, appelle l'applicateur gouverné. La voie B
    /// tient parce que la capacité d'écrire `ready` est **hors d'atteinte d'un
    /// tour LLM** : `run_gh` la refuse depuis le vol 1
    /// (`builtin_handlers.rs`, attestations de population dans `guards.rs`), et
    /// l'applicateur unique n'a que quatre appelants, tous dans un corps de tick
    /// ou un handler webhook. Cette seconde moitié est aujourd'hui vraie **par
    /// accident** : rien ne la refuse. Ce test la rend structurelle.
    ///
    /// # Pourquoi un scan de source et pas un test de comportement
    ///
    /// Un handler qui appellerait `apply_ready` demain ne rendrait **aucune
    /// décision fausse**. Le parc continuerait de répondre, la garde de mika#2772
    /// continuerait de discriminer, les sept assertions de
    /// `mod mika2772_consentement_force` resteraient vertes — et l'agent
    /// retrouverait le geste du 2026-10-31. C'est la classe mika#2158 : seul un
    /// scan structurel la voit.
    ///
    /// # Disposition si ce détecteur fire
    ///
    /// **Retirer l'appel**, jamais l'allowlister, jamais `#[ignore]`. Voir
    /// [`TOOL_SURFACE_APPLICATOR_EXCEPTIONS`], et surfacer le site au corps de PR
    /// sous `Fire-Disposition: cinquième applicateur découvert`.
    #[test]
    fn mika2995_aucune_surface_doutil_natteint_lapplicateur_de_ready() {
        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        let mut vus: Vec<String> = Vec::new();

        for (rel, prod) in crate::source_scan::production_files() {
            if !est_surface_doutil(&rel) {
                continue;
            }
            scanned += 1;
            vus.push(rel.clone());
            for site in appels_a_lapplicateur(&rel, &prod) {
                if TOOL_SURFACE_APPLICATOR_EXCEPTIONS
                    .iter()
                    .any(|(f, _)| *f == rel)
                {
                    continue;
                }
                offenders.push(site);
            }
        }

        // Deuxième mode de mensonge d'une allowlist vide : le scan pourrait
        // n'avoir rien scanné. L'idiome du test 13, resserré — « au moins un
        // fichier » resterait vrai si les deux gros fichiers étaient renommés et
        // que seuls les `tools/` restaient, donc les deux sont nommés.
        assert!(
            scanned > 2,
            "D1 n'a scanné que {scanned} fichier(s) — la population est cassée, et un \
             détecteur qui ne scanne rien ne refuse rien (mika#2578)"
        );
        for attendu in ["skills/builtin_handlers.rs", "skills/executor.rs"] {
            assert!(
                vus.iter().any(|v| v == attendu),
                "`{attendu}` n'est plus dans la population de D1 — renommé ou déplacé. \
                 Mettez `est_surface_doutil` à jour plutôt que de laisser le détecteur se \
                 rendormir en silence. Vus : {vus:?}"
            );
        }

        assert!(
            offenders.is_empty(),
            "mika#2995 — une surface d'outil atteint l'applicateur de `ready`. L'applicateur \
             est gouverné pour le PARC, pas pour le tour LLM : le parc dit « ce ticket est-il \
             dé-promu », jamais « qui demande ». Un handler qui l'appelle redonne à l'agent, \
             dans son tour, la capacité d'écrire le `ready` qui lève la garde de mika#2772 — \
             c'est le défaut mesuré le 2026-10-31 à 16:51:37Z, par une autre porte. Retirez \
             l'appel ; ne l'allowlistez pas.\n{}",
            offenders.join("\n")
        );
    }

    /// **Le contrôle de bonne foi de D1 — le scan rougit sur un appel planté.**
    ///
    /// Troisième mode de mensonge d'une allowlist vide : le scan pourrait être
    /// vert pour avoir **cessé de reconnaître sa forme**. Aucune assertion sur
    /// l'arbre réel ne peut voir ça — elle passerait. Ce test plante donc un
    /// appel et exige qu'il soit vu, puis quatre formes qui nomment
    /// l'applicateur sans l'appeler et exige le silence sur chacune.
    #[test]
    fn mika2995_le_scan_de_surface_rougit_sur_un_appel_plante() {
        let plante = "\
async fn handler_doutil(db: &AsyncDatabase, req: ReadyRequest) -> Outcome {
    ready_label::apply_ready(db, req).await
}
";
        let vus = appels_a_lapplicateur("tools/plante.rs", plante);
        assert_eq!(
            vus.len(),
            1,
            "le scan doit voir l'appel planté, sinon il est vert pour avoir cessé de \
             reconnaître sa forme : {vus:?}"
        );
        assert!(
            vus[0].contains("tools/plante.rs") && vus[0].contains("apply_ready(db, req)"),
            "l'offender doit nommer le fichier et rendre la ligne grep-able : {vus:?}"
        );

        // Les quatre formes muettes, chacune pour sa raison (cf. doc de
        // `appels_a_lapplicateur`).
        let muet = "\
/// Un doc-comment qui cite apply_ready(db, req) pour expliquer pourquoi ce
/// handler ne l'appelle pas — la forme de mika#2425.
/*
 * Un bloc qui cite gh_apply_label(auth, n, READY_LABEL) de la même façon.
 */
use crate::ready_label::{apply_ready, apply_ready_with_timeline};
pub(crate) async fn add_label_to_issue(token: &str, n: u64) -> Result<(), String> {
    Ok(())
}
";
        let muets = appels_a_lapplicateur("tools/muet.rs", muet);
        assert!(
            muets.is_empty(),
            "nommer l'applicateur n'est pas l'appeler : doc-comment, commentaire de bloc, \
             ligne de `use` et ligne de définition doivent rester muets, sinon D1 rougit sur \
             du code sain et finit par être désarmé — {muets:?}"
        );
    }

    // ── mika#2995 vol 2 — D2 : une écriture de `ready` en forme argv est déclarée ──

    /// Le drapeau `gh` qui ne peut **qu'**ajouter : celui d'`issue edit`.
    ///
    /// Nu, et non cité : il est déjà sans ambiguïté, et cela couvre
    /// `--add-label=ready` du même coup.
    const ARGV_ADD_LABEL_FLAG: &str = "--add-label";

    /// Les drapeaux d'étiquette **ambigus** : ceux d'`issue create`, qui sont
    /// aussi ceux d'`issue list`.
    ///
    /// `gh issue create --label ready` écrit le consentement ; `gh issue list
    /// --label ready` lit le bassin. Le drapeau est le même — seul le verbe
    /// tranche, et c'est [`ARGV_READ_VERBS`] qui le fait. Sans ce second
    /// prédicat, D2 rougirait sur `milestone_manager/reader.rs:273`, qui compte
    /// le bassin `ready` du dépôt et n'écrit rien.
    ///
    /// # Pourquoi des jetons cités
    ///
    /// `-l` nu apparaît dans des dizaines de mots ordinaires ; `"-l"` cité
    /// n'apparaît que là où un argv est construit. Même raison pour
    /// `"--label"`. `--label=` porte son `=` : c'est la forme collée, et la
    /// seule qui puisse naître d'un `format!`.
    ///
    /// # Ce n'est pas la liste de `guards.rs`
    ///
    /// [`crate::evidence::guards::ADD_LABEL_FLAGS`] liste les drapeaux tels que
    /// `gh` les reçoit, pour les **lire** dans un argv à l'exécution. Celles-ci
    /// listent les formes telles qu'un *fichier source* les écrit, pour les lire
    /// dans du texte Rust. Les deux populations n'ont ni le même alphabet ni le
    /// même lecteur — les réunir rendrait l'une des deux fausse en silence.
    const ARGV_AMBIGUOUS_LABEL_FLAGS: &[&str] = &["\"--label\"", "--label=", "\"-l\""];

    /// Les verbes `gh` qui **lisent** une étiquette au lieu de l'écrire.
    ///
    /// La polarité est fail-closed, et c'est la décision : un drapeau ambigu
    /// compte comme une écriture **sauf** si l'argv nomme explicitement un verbe
    /// de lecture. Un `--label=ready` dont aucun verbe n'est lisible dans sa
    /// fenêtre est donc refusé plutôt qu'excusé — l'exemption se mérite, et un
    /// site qui ne dit pas ce qu'il fait est exactement celui qu'il faut lire.
    const ARGV_READ_VERBS: &[&str] = &["\"list\"", "\"view\""];

    /// Le rayon de la fenêtre de D2, en lignes autour de la ligne du drapeau.
    ///
    /// La forme argv sépare le drapeau de sa valeur — `args.push("--add-label")`
    /// puis `args.push(READY_LABEL)` — et c'est exactement ce qui la rend
    /// invisible au test 13, qui exige les deux sur la **même** ligne. Deux
    /// lignes de part et d'autre couvrent la forme réelle de `pool_writer`, un
    /// `format!` coupé par rustfmt, et un commentaire intercalé ; au-delà, la
    /// fenêtre commencerait à happer des constantes voisines sans rapport.
    const FENETRE_ARGV: usize = 2;

    /// Le rayon de la fenêtre du **verbe**, plus large que celle de la valeur.
    ///
    /// Le verbe ouvre l'argv et le drapeau vient après ses options : dans
    /// `reader.rs`, `"list"` précède `"--label"` de cinq lignes, et un `--repo`
    /// ou un `--state` de plus les écarterait encore. Huit lignes couvrent un
    /// argv `gh` complet sans atteindre l'argv suivant de la même fonction.
    const FENETRE_VERBE: usize = 8;

    /// `true` si `line` porte le **jeton du label** `ready`.
    ///
    /// Deux écritures comptent, et une troisième est délibérément écartée :
    ///
    /// - `"ready"` **cité en entier** : la paire de guillemets est ce qui sépare
    ///   le label de `"ready_label_add"`, le motif d'audit de `guards.rs:2816` ;
    /// - `READY_LABEL` **non suivi d'un caractère d'identifiant** : c'est la
    ///   constante de ce module. `WORK_CREATION_MOTIF_READY_LABEL_ADD` nomme
    ///   autre chose et reste muet, sans quoi D2 rougirait sur le lecteur du
    ///   vol 1 — le site même qui refuse l'écriture.
    ///
    /// Le découpage passe par [`str::split`] plutôt que par un index d'octet :
    /// `scripts/check-byte-slices.sh` refuse la seconde forme, et pour une bonne
    /// raison (mika#764).
    fn porte_le_jeton_de_ready(line: &str) -> bool {
        if line.contains("\"ready\"") {
            return true;
        }
        line.split("READY_LABEL")
            .skip(1)
            .any(|apres| !apres.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    }

    /// Les `rayon` lignes de part et d'autre de `lines[i]`, elle comprise.
    fn fenetre<'a>(lines: &'a [&'a str], i: usize, rayon: usize) -> &'a [&'a str] {
        let debut = i.saturating_sub(rayon);
        let fin = (i + rayon + 1).min(lines.len());
        &lines[debut..fin]
    }

    /// Les **écritures de `ready` en forme argv** dans une moitié de production,
    /// rendues en `"<fichier>: <ligne du drapeau>"`.
    ///
    /// Trois conditions, dans cet ordre :
    ///
    /// 1. la ligne porte un drapeau d'étiquette — c'est lui qui ancre les deux
    ///    fenêtres. Une ligne de `use crate::ready_label::READY_LABEL;` seule ne
    ///    déclenche donc rien : importer la constante est le geste normal de tout
    ///    *lecteur* du label ;
    /// 2. le jeton du label est dans sa fenêtre ([`FENETRE_ARGV`]) — un
    ///    `--add-label` portant un autre label n'est pas une écriture de `ready` ;
    /// 3. si le drapeau est ambigu ([`ARGV_AMBIGUOUS_LABEL_FLAGS`]), aucun verbe
    ///    de lecture n'est dans sa fenêtre ([`FENETRE_VERBE`]).
    ///
    /// Les commentaires sont retirés d'abord — la prose *à propos* du geste n'est
    /// pas le geste (mika#2425), et `guards.rs` en porte quatre lignes qui citent
    /// `--add-label=ready` pour expliquer ce qu'elle refuse.
    fn ecritures_argv_de_ready(rel: &str, prod: &str) -> Vec<String> {
        let depouille = crate::source_scan::strip_comment_lines(prod);
        let lines: Vec<&str> = depouille.lines().collect();
        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let ambigu = if line.contains(ARGV_ADD_LABEL_FLAG) {
                false
            } else if ARGV_AMBIGUOUS_LABEL_FLAGS.iter().any(|f| line.contains(f)) {
                true
            } else {
                continue;
            };

            if !fenetre(&lines, i, FENETRE_ARGV)
                .iter()
                .any(|w| porte_le_jeton_de_ready(w))
            {
                continue;
            }

            if ambigu
                && fenetre(&lines, i, FENETRE_VERBE)
                    .iter()
                    .any(|w| ARGV_READ_VERBS.iter().any(|v| w.contains(v)))
            {
                continue;
            }

            out.push(format!("{rel}: {}", line.trim()));
        }
        out
    }

    /// **Les écritures de `ready` en forme argv qui sont DÉCLARÉES — une seule.**
    ///
    /// `(chemin depuis la racine du dépôt, ticket, le fait précis)`. Le chemin
    /// porte son préfixe `src/` pour qu'un `grep` depuis la racine du dépôt tombe
    /// sur cette entrée ; la population, elle, est relative à `src/` (c'est ce
    /// que rend [`crate::source_scan::production_files`]), d'où le `src/{rel}` au
    /// site de comparaison.
    ///
    /// Cette entrée n'est **pas** un contournement toléré : c'est le fait du
    /// § 3.2 du plan, pré-déclaré. `admit_pool_tickets` décide `--add-label ready`
    /// et `--milestone` d'un seul geste, hors applicateur **par conception**
    /// (propriété 4 du doc de module de `pool_writer`), donc aveugle au parc.
    /// `MIKA_MANAGER_POOL_WRITE` est OFF par défaut : il n'existe aujourd'hui
    /// aucune écriture non parquée par cette porte. Router ce site par le parc
    /// changerait un comportement voulu — c'est une **décision d'opérateur**, et
    /// l'ouverture de l'interrupteur est ce qui la rend due.
    const ARGV_READY_WRITERS_DECLARED: &[(&str, &str, &str)] = &[(
        "src/milestone_manager/pool_writer.rs",
        "mika#2995",
        "admit_pool_tickets : décision groupée --add-label ready + --milestone, \
         hors applicateur par conception (propriété 4 du doc de module), donc \
         AVEUGLE AU PARC. MIKA_MANAGER_POOL_WRITE OFF par défaut : aucune écriture \
         non parquée aujourd'hui. Router ce site par le parc changerait un \
         comportement voulu — décision d'opérateur, pré-déclarée sous \
         `Fire-Disposition: cinquième applicateur découvert`.",
    )];

    /// **D2 — toute écriture de `ready` en forme argv est déclarée (AC2c de
    /// mika#2995, voie B).**
    ///
    /// # Ce qu'il refuse, et que le test 13 ne peut pas voir
    ///
    /// Le test 13 exige `"ready"` et le nom d'un poseur sur la **même ligne**.
    /// La forme argv les sépare :
    ///
    /// ```text
    /// args.push("--add-label");
    /// args.push(READY_LABEL);
    /// ```
    ///
    /// Aucune ligne ne porte les deux, donc le test 13 est vert sur une écriture
    /// réelle — et il l'est aujourd'hui, sur `pool_writer.rs:156`. Ce n'est pas un
    /// défaut du test 13 : sa population et sa question sont autres, et deux
    /// populations sous un même nom ne se soustraient pas (mika#2205). D2 est
    /// l'autre question, et il la pose sur toute la production sauf ce module.
    ///
    /// # L'assertion auto-nettoyante
    ///
    /// Chaque entrée déclarée doit **encore porter la forme argv**. Si
    /// `pool_writer` est un jour routé par l'applicateur, supprimé ou réécrit,
    /// l'entrée devient stale et ce test rougit en demandant son retrait. Le geste
    /// qui rend l'exception inutile est celui qui la fait expirer : c'est ce qui
    /// sépare une déclaration d'une allowlist, et c'est ce que mika#2309 condamne
    /// quand il manque.
    ///
    /// # Disposition si un site NON déclaré apparaît
    ///
    /// Router le site par l'applicateur unique de mika#2315, jamais ajouter une
    /// ligne ici. Une entrée nouvelle est une décision d'opérateur, et elle se
    /// surface au corps de PR sous `Fire-Disposition: cinquième applicateur
    /// découvert`.
    #[test]
    fn mika2995_une_ecriture_de_ready_en_forme_argv_est_declaree() {
        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        let mut vus: Vec<String> = Vec::new();
        let mut declares_encore_porteurs: Vec<&str> = Vec::new();

        for (rel, prod) in crate::source_scan::production_files() {
            // Ce module EST l'applicateur : il porte le littéral et sa propre
            // écriture gouvernée. Sa population est celle du test 13.
            if rel == "ready_label.rs" {
                continue;
            }
            scanned += 1;
            vus.push(rel.clone());

            let sites = ecritures_argv_de_ready(&rel, &prod);
            if sites.is_empty() {
                continue;
            }
            let depuis_la_racine = format!("src/{rel}");
            match ARGV_READY_WRITERS_DECLARED
                .iter()
                .find(|(f, _, _)| *f == depuis_la_racine)
            {
                Some((f, _, _)) => declares_encore_porteurs.push(f),
                None => offenders.extend(sites),
            }
        }

        // Un détecteur qui ne scanne rien ne refuse rien (mika#2578/#2752). Les
        // deux fichiers nommés sont ceux où un faux positif guetterait : le
        // lecteur argv du vol 1, et le poseur générique de labels.
        assert!(
            scanned > 50,
            "D2 n'a scanné que {scanned} fichier(s) de production — la population est cassée"
        );
        for attendu in ["evidence/guards.rs", "auto_pull.rs"] {
            assert!(
                vus.iter().any(|v| v == attendu),
                "`{attendu}` n'est plus dans la population de D2 — renommé ou déplacé. \
                 Remettez-le dans la population plutôt que de laisser le détecteur se \
                 rendormir en silence."
            );
        }

        assert!(
            offenders.is_empty(),
            "mika#2995 — une écriture de `ready` en forme argv n'est pas déclarée. Le test 13 \
             ne la voit pas : il exige le label et le poseur sur la MÊME ligne, et la forme \
             argv les sépare. Un site qui écrit `ready` hors de l'applicateur unique \
             (mika#2315) est aveugle au parc — il crée un consentement que personne n'a donné, \
             et c'est le consentement que la garde de mika#2772 exige d'un humain. Routez le \
             site par l'applicateur ; n'ajoutez une entrée que sur décision d'opérateur.\n{}",
            offenders.join("\n")
        );

        // L'auto-nettoyage : une entrée dont le site a changé est retirée, pas
        // conservée « au cas où ».
        for (f, ticket, fait) in ARGV_READY_WRITERS_DECLARED {
            assert!(
                declares_encore_porteurs.contains(f),
                "l'entrée `{f}` ({ticket}) ne porte plus de forme argv de `ready` — le site a \
                 été routé par l'applicateur, supprimé ou renommé. RETIREZ L'ENTRÉE : une \
                 déclaration qui survit à son site est l'allowlist que mika#2309 condamne. \
                 Fait déclaré : {fait}"
            );
        }
    }

    /// **Le contrôle de bonne foi de D2 — le scan rougit sur une écriture
    /// plantée.**
    ///
    /// D2 est vert sur l'arbre réel avec exactement une entrée déclarée. Ce vert
    /// a trois causes possibles et une seule est bonne : il pourrait aussi venir
    /// d'un scan qui a cessé de reconnaître sa forme. Aucune assertion sur
    /// l'arbre ne peut le voir. Ce test plante donc les **trois** syntaxes,
    /// chacune poussée sur deux lignes comme la forme réelle, et exige que
    /// chacune soit vue — puis deux formes saines, et exige le silence.
    #[test]
    fn mika2995_le_scan_argv_rougit_sur_une_ecriture_plantee() {
        // Les trois syntaxes que `gh` accepte, dans leur forme Rust réelle : le
        // drapeau et la valeur sur deux lignes distinctes.
        for (syntaxe, plante) in [
            (
                "--add-label",
                "    args.push(\"--add-label\");\n    args.push(READY_LABEL);\n",
            ),
            (
                "--label=",
                "    let flag = format!(\"--label={}\",\n        READY_LABEL);\n",
            ),
            (
                "-l",
                "    args.push(\"-l\");\n    args.push(READY_LABEL);\n",
            ),
        ] {
            let vus = ecritures_argv_de_ready("plante.rs", plante);
            assert_eq!(
                vus.len(),
                1,
                "D2 ne voit pas la syntaxe `{syntaxe}` poussée sur deux lignes — il serait \
                 vert pour avoir cessé de reconnaître sa forme, et c'est exactement la forme \
                 que le test 13 laisse passer : {vus:?}"
            );
        }

        // Importer la constante est le geste de tout lecteur du label, et il
        // n'écrit rien. Le drapeau est requis.
        let import = "\
use crate::ready_label::READY_LABEL;

fn lit_seulement(labels: &[String]) -> bool {
    labels.iter().any(|l| l == READY_LABEL)
}
";
        let muets = ecritures_argv_de_ready("import.rs", import);
        assert!(
            muets.is_empty(),
            "un `use READY_LABEL` sans drapeau n'est pas une écriture — D2 rougirait sur tous \
             les lecteurs du label et finirait désarmé : {muets:?}"
        );

        // La forme de `wip_rescue.rs` : le même drapeau, un autre label. Le
        // jeton du label est requis, pas seulement le drapeau.
        let autre_label = "\
    let add_label = [
        \"pr\",
        \"edit\",
        \"--add-label\",
        HUMAN_REVIEW_LABEL,
    ];
";
        let muets = ecritures_argv_de_ready("autre.rs", autre_label);
        assert!(
            muets.is_empty(),
            "un `--add-label` portant un AUTRE label n'est pas une écriture de `ready` — D2 \
             rougirait sur tout poseur de label du dépôt : {muets:?}"
        );

        // La forme de `milestone_manager/reader.rs:273` : le drapeau ambigu
        // d'`issue create` est aussi celui d'`issue list`, et le verbe tranche.
        // Sans ce cas, le prédicat du verbe pourrait être retiré sans qu'aucun
        // test ne le dise — alors que c'est lui qui empêche D2 d'appeler
        // « écriture » le comptage du bassin `ready`.
        let lecture = "\
    let ready_json = runner
        .run(&[
            \"issue\",
            \"list\",
            \"--state\",
            \"open\",
            \"--label\",
            READY_LABEL,
        ])
        .await?;
";
        let muets = ecritures_argv_de_ready("lecture.rs", lecture);
        assert!(
            muets.is_empty(),
            "`gh issue list --label ready` LIT le bassin, il n'écrit rien — D2 rougirait sur \
             le compteur de la porte de place et serait désarmé pour cette raison : {muets:?}"
        );
    }

    // ── mika#2995, AC-T4 — un site de définition unique pour chaque nom ──────
    //
    // Troisième détecteur de ce module, et sa population est encore une autre :
    // le test 13 refuse une *écriture* de `ready` hors du module, D1 refuse
    // qu'une surface d'outil atteigne l'applicateur, D2 refuse une écriture de
    // forme argv non déclarée — et D3 refuse qu'un **nom** de la relecture de
    // consentement ait deux sites. Deux populations sous un même nom ne se
    // soustraient pas (mika#2205), donc chacun porte sa disposition.

    /// Les exceptions de D3. **Née vide, et destinée à le rester.**
    ///
    /// Trois champs : le fichier, le nom, et la raison. La raison est exigée
    /// parce qu'une allowlist dont les entrées n'ont pas à se justifier est une
    /// allowlist qui se remplit (mika#2309).
    const CONSENT_RECHECK_NAME_EXCEPTIONS: &[(&str, &str, &str)] = &[];

    /// Les quatre noms de la relecture de consentement, et le fichier qui
    /// porte leur site unique.
    ///
    /// Les deux premiers sont les constantes d'AC-T4 au sens littéral — le
    /// motif du terminal et le nom de l'outil d'audit. Les deux suivants sont
    /// les noms d'événement des deux sondes, inclus parce qu'ils répondent à la
    /// même question : un second émetteur de
    /// `deferred_dispatch_consent_recheck_deferred` rendrait son compte
    /// illisible exactement comme un second site du motif rendrait le terminal
    /// ambigu.
    const CONSENT_RECHECK_NAMES: &[(&str, &str)] = &[
        (READY_CONSENT_WITHDRAWN_REASON, "ready_label.rs"),
        (READY_CONSENT_RECHECK_TOOL_NAME, "ready_label.rs"),
        (
            "deferred_dispatch_refused_consent_withdrawn",
            "task_engine/dispatcher.rs",
        ),
        (
            "deferred_dispatch_consent_recheck_deferred",
            "task_engine/dispatcher.rs",
        ),
    ];

    /// Les lignes de production de `rel` où `nom` apparaît **comme littéral de
    /// chaîne**.
    ///
    /// `"{nom}"` avec ses guillemets, et non le nom nu : un `use` de la
    /// constante, un `match` sur elle ou une prose qui la cite ne sont pas des
    /// sites de définition, et un prédicat sur le nom nu rougirait sur tous ses
    /// lecteurs — donc finirait désarmé.
    fn sites_du_nom(rel: &str, prod: &str, nom: &str) -> Vec<String> {
        let litteral = format!("\"{nom}\"");
        crate::source_scan::strip_comment_lines(prod)
            .lines()
            .filter(|l| l.contains(&litteral))
            .map(|l| format!("{rel} — {}", l.trim()))
            .collect()
    }

    /// **L'assertion sœur d'auto-vacuité de D3.**
    ///
    /// Une allowlist née vide est un emplacement où déposer la prochaine
    /// infraction. Celle-ci rougit à la première entrée.
    #[test]
    fn mika2995_lallowlist_des_noms_de_relecture_est_vide() {
        assert!(
            CONSENT_RECHECK_NAME_EXCEPTIONS.is_empty(),
            "une exception a été ajoutée à D3 : {:?}. La disposition de ce détecteur est de \
             ROUTER le second site par la constante, pas de l'allowlister (mika#2201).",
            CONSENT_RECHECK_NAME_EXCEPTIONS
        );
    }

    /// **D3 — le motif du terminal et l'événement d'audit ont chacun un site de
    /// définition unique, et aucun ne porte `noop` nu (AC-T4 de mika#2995).**
    ///
    /// # Ce qu'il refuse
    ///
    /// Qu'un second site écrive l'un de ces quatre noms en clair. C'est la
    /// classe mika#2158 : une seconde copie ne rend **aucune décision fausse**
    /// le jour où elle est écrite, et rate le premier renommage — après quoi
    /// deux moitiés de la même population portent deux motifs, et le `grep` qui
    /// répond « combien de wrappers ont été refusés pour consentement retiré »
    /// en rend une seule sans dire qu'il en manque.
    ///
    /// # Pourquoi `noop` est interdit dans ces noms
    ///
    /// `deferred_dispatch_noop_completion` (mika#1124) et le statut `expired`
    /// de mika#2169 occupent déjà ce mot pour « le tour a eu lieu et n'a rien
    /// dispatché ». Ici le tour **n'a pas eu lieu**, ce qui est strictement
    /// moins — et un nom partagé ferait des deux une seule population
    /// (mika#2205).
    ///
    /// # Disposition si ce détecteur fire
    ///
    /// **Router le second site par la constante**, jamais l'allowlister. Voir
    /// [`CONSENT_RECHECK_NAME_EXCEPTIONS`], et surfacer le site au corps de PR
    /// sous `Fire-Disposition: second site d'un nom de relecture`.
    #[test]
    fn mika2995_le_motif_et_levenement_ont_un_site_de_definition_unique() {
        for (nom, _) in CONSENT_RECHECK_NAMES {
            assert!(
                !nom.contains("noop"),
                "`{nom}` porte `noop` nu, déjà pris par mika#1124 et mika#2169 pour « le tour a \
                 eu lieu et n'a rien dispatché ». Ici le tour n'a pas eu lieu."
            );
        }

        let fichiers = crate::source_scan::production_files();
        assert!(
            fichiers.len() > 100,
            "D3 n'a scanné que {} fichier(s) — la population est cassée, et un détecteur qui \
             ne scanne rien ne refuse rien (mika#2578)",
            fichiers.len()
        );

        let mut offenders = Vec::new();
        for (nom, attendu) in CONSENT_RECHECK_NAMES {
            let mut sites = Vec::new();
            for (rel, prod) in &fichiers {
                if CONSENT_RECHECK_NAME_EXCEPTIONS
                    .iter()
                    .any(|(f, n, _)| f == rel && n == nom)
                {
                    continue;
                }
                sites.extend(sites_du_nom(rel, prod, nom));
            }
            // Zéro site est une infraction au même titre que deux : le nom
            // aurait été renommé dans le code sans l'être ici, et D3 se serait
            // rendormi en silence sur un nom qui n'existe plus.
            if sites.len() != 1 {
                offenders.push(format!(
                    "`{nom}` a {} site(s) (attendu : 1, dans `{attendu}`) :\n  {}",
                    sites.len(),
                    sites.join("\n  ")
                ));
                continue;
            }
            if !sites[0].starts_with(*attendu) {
                offenders.push(format!(
                    "`{nom}` a bien un site unique, mais il a déménagé : attendu `{attendu}`, \
                     vu `{}`",
                    sites[0]
                ));
            }
        }

        assert!(
            offenders.is_empty(),
            "mika#2995 AC-T4 — un nom de la relecture de consentement n'a plus un site unique. \
             Routez le second site par la constante ; ne l'allowlistez pas.\n{}",
            offenders.join("\n")
        );
    }

    /// **Le contrôle de bonne foi de D3 — le scan rougit sur un second site.**
    ///
    /// Le scan pourrait être vert pour avoir **cessé de reconnaître sa forme**.
    /// Aucune assertion sur l'arbre réel ne peut voir ça — elle passerait. Ce
    /// test plante donc un second site et exige qu'il soit vu, puis trois
    /// formes qui nomment la constante sans la définir et exige le silence sur
    /// chacune.
    #[test]
    fn mika2995_le_scan_des_noms_rougit_sur_un_second_site() {
        let plante = "\
async fn un_autre_terminal(db: &AsyncDatabase, id: &str) {
    db.mark_deferred_wrapper_noop(id, \"ready_consent_withdrawn\").await.ok();
}
";
        let vus = sites_du_nom("second.rs", plante, READY_CONSENT_WITHDRAWN_REASON);
        assert_eq!(
            vus.len(),
            1,
            "le scan doit voir le second site planté, sinon il est vert pour avoir cessé de \
             reconnaître sa forme : {vus:?}"
        );
        assert!(
            vus[0].contains("second.rs") && vus[0].contains("mark_deferred_wrapper_noop"),
            "l'offender doit nommer le fichier et rendre la ligne grep-able : {vus:?}"
        );

        // Les trois formes muettes, chacune pour sa raison : un commentaire qui
        // cite le motif pour expliquer le site (la forme de mika#2425), un
        // `use` de la constante, et une consommation de la constante — c'est
        // exactement ce que le dispatcher fait, et D3 rougirait sur lui.
        let muet = "\
/// Le terminal est écrit sous \"ready_consent_withdrawn\", défini dans ready_label.
use crate::ready_label::READY_CONSENT_WITHDRAWN_REASON;
async fn consomme(db: &AsyncDatabase, id: &str) {
    db.mark_deferred_wrapper_noop(id, READY_CONSENT_WITHDRAWN_REASON).await.ok();
}
";
        let muets = sites_du_nom("muet.rs", muet, READY_CONSENT_WITHDRAWN_REASON);
        assert!(
            muets.is_empty(),
            "citer ou consommer la constante n'est pas la définir — D3 rougirait sur tous ses \
             lecteurs et finirait désarmé : {muets:?}"
        );
    }

    // ───── mika#3050 — le prédicat d'état avant un ré-armement (D2/D3) ─────

    /// Une réponse `gh issue view --json state,labels` canée.
    fn relecture(state: &str, labels: &[&str]) -> String {
        let labels = labels
            .iter()
            .map(|l| format!(r#"{{"name":"{l}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"state":"{state}","labels":[{labels}]}}"#)
    }

    #[test]
    fn mika3050_ouvert_et_portant_ready_rend_le_consentement_tenu() {
        assert_eq!(
            parse_rearm_consent(&relecture("OPEN", &["ready", "type:bug"]), "x/y", 1),
            Ok(RearmConsent::Held),
            "le contrôle positif : les 4 premiers cas du tableau de mesure"
        );
    }

    #[test]
    fn mika3050_ouvert_sans_ready_rend_ready_absent() {
        assert_eq!(
            parse_rearm_consent(&relecture("OPEN", &["type:bug"]), "x/y", 1),
            Ok(RearmConsent::ReadyAbsent),
            "la forme de 74130014/#3002 : ouvert, mais `ready` retiré entre-temps"
        );
    }

    #[test]
    fn mika3050_un_ticket_ferme_rend_closed_quel_que_soit_le_label() {
        // `ready` encore posé ET fermé : le `state` décide d'abord, sinon un
        // ticket fermé sans nettoyage de label ré-armerait.
        assert_eq!(
            parse_rearm_consent(&relecture("CLOSED", &["ready"]), "x/y", 1),
            Ok(RearmConsent::Closed),
        );
        // La casse de l'API n'est pas un contrat : `closed` se lit aussi.
        assert_eq!(
            parse_rearm_consent(&relecture("closed", &[]), "x/y", 1),
            Ok(RearmConsent::Closed),
        );
    }

    #[test]
    fn mika3050_un_state_hors_contrat_est_une_erreur_pas_un_etat() {
        let v = parse_rearm_consent(&relecture("MERGED", &["ready"]), "x/y", 7);
        let e = v.expect_err("un `state` inconnu ne doit pas se replier sur OPEN");
        assert!(
            e.contains("hors contrat") && e.contains("x/y#7") && e.contains("MERGED"),
            "la cause doit nommer la valeur refusée, c'est elle qu'un opérateur grep : {e}"
        );
        // Un `state` absent est du même côté : fail-closed, jamais `Held`.
        assert!(
            parse_rearm_consent(r#"{"labels":[{"name":"ready"}]}"#, "x/y", 7).is_err(),
            "pas de `state` ⇒ pas de verdict d'ouverture"
        );
    }

    #[test]
    fn mika3050_labels_absent_est_une_erreur_jamais_ready_absent() {
        // Le cœur de D3 : « le champ n'est pas là » et « le label n'est pas
        // posé » sont deux faits distincts. Les confondre poserait un terminal
        // `ready_absent_at_rearm` sur une parente consentante, sur la foi d'une
        // réponse tronquée (mika#2131, mika#2331).
        let v = parse_rearm_consent(r#"{"state":"OPEN"}"#, "x/y", 7);
        let e = v.expect_err("`labels` absent n'est pas un `ready` absent");
        assert!(
            e.contains("labels") && e.contains("x/y#7"),
            "la cause doit nommer le champ manquant et le ticket : {e}"
        );
        assert!(
            parse_rearm_consent("ceci n'est pas du json", "x/y", 7).is_err(),
            "une charge illisible est une erreur, pas un état"
        );
    }

    /// La commande émise, mot pour mot — c'est elle que la couture d'appel route
    /// par motif d'argv (`"issue view"`, mika#3047), et c'est `--json
    /// state,labels` qui rend les deux moitiés du prédicat lisibles. Un champ
    /// retiré de ce `--json` ferait rendre `Err` à tous les tickets sains : la
    /// garde deviendrait un `NotNow` permanent, et la file ne tournerait plus.
    #[tokio::test]
    async fn mika3050_la_relecture_demande_state_et_labels_sur_le_bon_depot() {
        let n = 990_350u64;
        crate::gh_process::set_canned_stdout_for(
            crate::gh_process::GhSubject::Issue(n),
            "issue view",
            relecture("OPEN", &["ready"]),
        );

        let v = reread_issue_consent_via_gh("fake-token", "senara-solutions/mika", n).await;
        assert_eq!(v, Ok(RearmConsent::Held));

        let argv = crate::gh_process::recorded_calls()
            .into_iter()
            .filter(|c| c.subject == crate::gh_process::GhSubject::Issue(n))
            .map(|c| c.args)
            .collect::<Vec<_>>();
        assert_eq!(argv.len(), 1, "une relecture, un appel : {argv:?}");
        assert_eq!(
            argv[0],
            format!("issue view {n} --repo senara-solutions/mika --json state,labels"),
        );
    }
}

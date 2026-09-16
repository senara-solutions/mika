//! Réconciliation des demandes de revue (mika#2334).
//!
//! # Le défaut que ce module ferme
//!
//! Le ticket fondateur décrit deux PRs du drain du 2026-09-15 restées ouvertes
//! sans revue, et en déduit qu'un pas trailing `gh pr edit --add-reviewer` avait
//! été sauté parce que le pilote mourait avant de l'atteindre. La lecture du
//! code déplace le diagnostic sur deux mesures :
//!
//! - **Ce pas n'existe pas.** Aucun site du dépôt ne posait de relecteur sur une
//!   PR ; les huit `gh pr edit` de `dispatch-lib.sh` ne portent que
//!   `--add-label`, `--title`, `--body`. Il n'y avait rien à déplacer avant les
//!   pas fragiles.
//! - **La revue ne dépend pas du pilote.** `pull_request.opened` est routé vers
//!   mika-qa par le gateway (`mika-gateway/src/github.rs`), donc la seule
//!   création de la PR démarre la cascade — la mort du pilote après le push ne
//!   peut pas, à elle seule, empêcher la revue.
//!
//! Le défaut réel est en dessous : **`opened` est un événement unique, non
//! rejouable, et perdable** — file webhook bornée qui jette la tête de file à
//! saturation (mika#1870), circuit breaker du gateway qui envoie en DLQ puis en
//! `dead`, tour LLM vide dont la garde `webhook_zero_tools` n'est opposée
//! qu'une fois. Et **aucun chemin ne relisait une PR ouverte sans revue** : les
//! trois scans récurrents voisins travaillent sur les issues (`auto_pull`), sur
//! les brouillons étiquetés (`wip_rescue`) ou sur les skills
//! (`curator_review`), et le seul rattrapage existant — le fan-out
//! `check_suite.completed(success)` de mika#1711 — exige `draft: false` **et**
//! une CI verte.
//!
//! # Pourquoi un scan, et non un geste au moment de la création
//!
//! Le commentaire opérateur privilégiait la lettre « poser le relecteur AVANT
//! les pas fragiles ». Un geste **inconditionnel** posé à la création produirait
//! une revue en double sur **chaque** PR de la boucle, pas seulement sur les
//! sinistrées : `gh pr create` émet `opened` (mika-qa démarre), le geste émet
//! `review_requested` quelques secondes plus tard sur exactement
//! [`REVIEWER_FORGE_LOGIN`] — donc non filtré par `is_suppressed_review_request`
//! (mika#1655) — et mika-qa démarre une seconde session qui ne peut pas
//! constater que la première a déjà revu. Le doublon serait le cas nominal.
//!
//! D'où les deux corollaires qui décident la conception : le geste doit être
//! **conditionnel à l'absence de revue**, et il ne peut donc pas vivre dans
//! `dispatch-lib.sh` — le tail shell livre son callback et meurt en quelques
//! secondes, il ne peut pas constater une absence qui ne se mesure qu'après un
//! délai. Le lieu correct est un scan périodique, hors LLM et hors session
//! pilote, de la même famille qu'`auto_pull` et `wip_rescue`. Il satisfait le
//! test négatif du ticket à la lettre — la terminaison du pilote n'est pas une
//! entrée de la décision — et couvre en plus les pertes amont, qui ne sont pas
//! des morts de pilote.
//!
//! Ce choix suit aussi la doctrine du dépôt
//! (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
//! mesurée par mika#2120) : un pas ajouté à un prompt hériterait exactement de
//! la fragilité que le ticket dénonce.
//!
//! # Ce que ce scan n'est PAS
//!
//! Un **filet**, pas un chemin. S'il porte le trafic nominal, c'est la perte
//! amont qu'il faut traiter — et le filet masque désormais le signal qui
//! permettrait de la voir. La sonde AC4 (`grep qa_review_reconciled`, régime
//! attendu ≤ 1/jour après absorption de l'arriération) est là pour le dire.

use crate::async_db::AsyncDatabase;
use crate::tools::pr_merge_with_gate::run_gh_subprocess;
use chrono::{DateTime, Utc};
use mika_common::forge_identity::{DISPATCHER_FORGE_LOGIN, REVIEWER_FORGE_LOGIN};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Dépôts scannés par défaut. L'extension à mika-cloud est un geste d'une
/// ligne sur [`REPOS_ENV`], délibérément non anticipé ici.
const DEFAULT_REPOS: &str = "senara-solutions/mika";

/// Per-subprocess timeout for the `gh` calls, aligned with `wip_rescue`.
const GH_TIMEOUT: Duration = Duration::from_secs(30);

/// `audit_events.tool_name` écrit à chaque pose de relecteur.
///
/// **SOLE WRITER** — ce module est le seul site qui écrit ce nom. C'est ce qui
/// fait de `SELECT … WHERE tool_name = 'qa_review_reconciled'` la liste exacte
/// des PRs que la boucle a dû rattraper, donc la mesure de la santé du chemin
/// nominal.
const RECONCILED_TOOL: &str = "qa_review_reconciled";

// -- Réglages (trois paliers : absent/vide → défaut ; illisible, 0 ou négatif
//    → défaut + WARN). Le `0` ne désarme pas : c'est le rôle du kill-switch
//    `MIKA_QA_REVIEW_RECONCILE`, lu au moment d'enregistrer la tâche récurrente.

const MIN_AGE_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_MIN_AGE_SECS";
/// Une heure. **Arbitrage asymétrique, pas une rondeur** : trop court, le délai
/// recrée le doublon que toute cette conception existe pour éviter ; trop long,
/// la PR attend son rattrapage — alors qu'aujourd'hui elle attend indéfiniment.
/// Une heure couvre largement l'enveloppe d'un tour (300 s) et une revue passée
/// par le callback de build.
const MIN_AGE_DEFAULT_SECS: i64 = 3600;

const MAX_AGE_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_MAX_AGE_SECS";
/// Sept jours. Au-delà, une PR sans revue n'est pas un webhook perdu mais une
/// PR abandonnée ; la réveiller n'aide personne.
const MAX_AGE_DEFAULT_SECS: i64 = 604_800;

const MAX_PER_TICK_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_MAX_PER_TICK";
/// Étalement du premier tick après déploiement — le seul moment où ce scan peut
/// faire du bruit, puisqu'il voit toute l'arriération d'un coup.
///
/// **Passé de 3 à 1 par mika#2347, et c'est le seul défaut que ce correctif
/// déplace.** Trois poses par tick et quatre ticks par heure font **12
/// demandes/heure** ; à une enveloppe de 600 s par revue et une exécution
/// sérialisée par `agent_lock`, la capacité de mika-qa plafonne à **6
/// revues/heure** — et le trafic nominal (`opened`, `synchronize`,
/// `review_requested`, fan-out `check_suite` de mika#1711) s'ajoute par-dessus.
/// Le cap par tick, seul, autorisait donc une file qui croît sans borne. À 1, le
/// rattrapage demande au plus **4 revues/heure**, sous la capacité. C'est le seul
/// réglage de ce ticket dont l'effet est arithmétiquement démontrable.
const MAX_PER_TICK_DEFAULT: usize = 1;

const COOLDOWN_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_COOLDOWN_SECS";
/// Une heure, alignée sur [`MIN_AGE_DEFAULT_SECS`] : le délai doit dépasser la
/// durée d'un tour de revue avec une marge large, et une PR que le rattrapage
/// n'a pas réveillée en une heure ne le sera pas davantage en quinze minutes.
const COOLDOWN_DEFAULT_SECS: i64 = 3600;

const MAX_ATTEMPTS_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_MAX_ATTEMPTS";
/// Deux rattrapages par (dépôt, PR, SHA), puis abandon.
///
/// **Le budget est ce qui distingue ce correctif d'un simple ralentissement** :
/// un cooldown seul rejoue pour toujours, juste moins vite. Précédent direct du
/// dépôt, `MIKA_AUTO_PULL_MAX_REDRIVES` (mika#2020), né du constat que mika#1901
/// avait reçu le label `ready` seize fois en dix-neuf heures. Deux laisse une
/// seconde chance à un webhook perdu et refuse la troisième.
const MAX_ATTEMPTS_DEFAULT: i64 = 2;

const REPOS_ENV: &str = "MIKA_QA_REVIEW_RECONCILE_REPOS";

/// `audit_events.tool_name` écrit à chaque abandon d'un (dépôt, PR, SHA).
///
/// **SOLE WRITER**, comme [`RECONCILED_TOOL`]. C'est la réponse directe à
/// « pourquoi cette PR n'est-elle plus rattrapée ? », sans grep.
const ABANDONED_TOOL: &str = "qa_review_reconcile_abandoned";

// ---------------------------------------------------------------------------
// Formes JSON GitHub
// ---------------------------------------------------------------------------

/// Une PR telle que `gh pr list --json` la rend.
///
/// **Aucun `#[serde(default)]` sur `review_requests` / `reviews`, et c'est
/// porteur** : ces deux champs sont explicitement demandés dans `--json`, donc
/// GitHub les rend toujours (`[]` quand ils sont vides). Les rendre
/// `default`-ables ferait lire une absence comme « aucune demande, aucune
/// revue » — c'est-à-dire ferait *entrer* la PR dans la population sur une
/// information manquante, l'exact inverse du fail-safe. Sans `default`, un
/// champ absent est une erreur de parsing qui avorte le tick.
#[derive(Debug, Clone, Deserialize)]
pub struct PrSnapshot {
    pub number: u64,
    /// `None` quand l'auteur est un compte supprimé — la PR sort alors de la
    /// population, faute de pouvoir prouver qu'elle vient de la boucle.
    pub author: Option<GhAuthor>,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// SHA de tête, qui keye le ledger de mika#2347.
    ///
    /// **Pas de `#[serde(default)]`, pour la raison déjà écrite au-dessus de
    /// cette structure** : le champ est explicitement demandé dans `--json`, donc
    /// toujours rendu. Une absence doit avorter le parse plutôt que faire entrer
    /// la PR dans la population avec une clé de ledger vide — qui confondrait
    /// toutes les PRs sans SHA en un seul compteur. Une valeur **vide** est
    /// traitée comme illisible et sort la PR, même discipline que `createdAt`.
    #[serde(rename = "headRefOid")]
    pub head_ref_oid: String,
    #[serde(rename = "reviewRequests")]
    pub review_requests: Vec<GhReviewRequest>,
    pub reviews: Vec<GhReview>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhAuthor {
    pub login: String,
}

/// Une demande de revue. `login` est `None` pour une équipe (`__typename:
/// "Team"`), qui porte `name`/`slug` — une équipe n'est jamais le compte
/// utilisateur [`REVIEWER_FORGE_LOGIN`], donc son absence de login est une
/// information lisible, pas une information manquante.
#[derive(Debug, Clone, Deserialize)]
pub struct GhReviewRequest {
    #[serde(default)]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhReview {
    #[serde(default)]
    pub author: Option<GhAuthor>,
}

/// Une PR retenue, son âge au moment de la décision, et son SHA de tête.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRef {
    pub number: u64,
    pub age_secs: i64,
    /// Ce qui keye le ledger : **un nouveau SHA rouvre naturellement le budget**,
    /// parce que la clé change et que le compte repart à zéro. C'est
    /// l'idempotence « par (PR, SHA) » obtenue par la forme de la clé plutôt que
    /// par un champ de plus.
    pub head_sha: String,
}

/// Les bornes numériques de la décision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileConfig {
    pub min_age_secs: i64,
    pub max_age_secs: i64,
    /// Plafond d'écritures par tick. **Lu par l'appelant, pas par
    /// [`select_prs_needing_review`]** depuis mika#2347 — voir la note de
    /// déplacement sur cette fonction.
    pub max_per_tick: usize,
    /// Délai après une pose avant qu'une nouvelle soit possible sur le même
    /// (dépôt, PR, SHA).
    pub cooldown_secs: i64,
    /// Nombre de poses au-delà duquel le (dépôt, PR, SHA) est abandonné.
    pub max_attempts: i64,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            min_age_secs: MIN_AGE_DEFAULT_SECS,
            max_age_secs: MAX_AGE_DEFAULT_SECS,
            max_per_tick: MAX_PER_TICK_DEFAULT,
            cooldown_secs: COOLDOWN_DEFAULT_SECS,
            max_attempts: MAX_ATTEMPTS_DEFAULT,
        }
    }
}

// ---------------------------------------------------------------------------
// La décision — fonction pure, testable sans réseau
// ---------------------------------------------------------------------------

/// Les PRs de la boucle qu'aucune revue n'est venue chercher.
///
/// Conjonction de six termes, chacun **fail-safe** : une information illisible
/// sort la PR de la population, elle ne l'y fait jamais entrer.
///
/// | terme | raison |
/// |---|---|
/// | auteur = [`DISPATCHER_FORGE_LOGIN`] | une PR humaine n'est pas la boucle et ne se fait pas poser un relecteur par elle |
/// | `isDraft == false` | les brouillons ont leur propre voie (`wip_rescue` → `gh pr ready` → `ready_for_review`), et une PR de rescue est délibérément tenue en brouillon |
/// | aucune demande pour [`REVIEWER_FORGE_LOGIN`] | idempotence : une demande déjà posée sort la PR de la population |
/// | aucune revue de [`REVIEWER_FORGE_LOGIN`] | GitHub retire la demande quand la revue est soumise ; sans ce terme, chaque PR revue serait re-demandée en boucle |
/// | âge > `min_age_secs` | laisse au chemin nominal le temps d'aboutir — c'est ce terme qui évite le doublon |
/// | âge < `max_age_secs` | au-delà, PR abandonnée |
/// | `headRefOid` non vide | sans SHA, la clé du ledger (mika#2347) ne discrimine plus rien |
///
/// La terminaison du pilote qui a produit la PR **n'est pas une entrée** de
/// cette fonction : c'est la forme structurelle de « indépendant de la survie
/// du pilote » (AC1).
///
/// Ordre : la plus vieille d'abord, puis par numéro croissant pour que deux
/// ticks sur le même état rendent la même liste.
///
/// # La troncature a déménagé (mika#2347), et c'est un changement de contrat
///
/// Cette fonction **ne tronque plus** à `max_per_tick` ; l'appelant tronque
/// après avoir appliqué le filtre de cooldown. Si la troncature restait ici,
/// `max_per_tick` PRs en cooldown consommeraient tout le tick et la suivante,
/// légitimement rattrapable, ne serait jamais vue — un plafond sur les *sauts*
/// au lieu d'un plafond sur les *écritures*. Ce n'est pas une assertion perdue :
/// c'est une assertion déplacée avec la décision qu'elle garde.
pub fn select_prs_needing_review(
    prs: &[PrSnapshot],
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Vec<PrRef> {
    let mut retained: Vec<PrRef> = prs
        .iter()
        .filter_map(|pr| {
            if pr.is_draft {
                return None;
            }
            // Auteur illisible (compte supprimé) ⇒ hors population.
            let author = pr.author.as_ref()?;
            if !is_login(&author.login, DISPATCHER_FORGE_LOGIN) {
                return None;
            }
            if pr
                .review_requests
                .iter()
                .filter_map(|r| r.login.as_deref())
                .any(|l| is_login(l, REVIEWER_FORGE_LOGIN))
            {
                return None;
            }
            if pr
                .reviews
                .iter()
                .filter_map(|r| r.author.as_ref())
                .any(|a| is_login(&a.login, REVIEWER_FORGE_LOGIN))
            {
                return None;
            }
            // `createdAt` illisible ⇒ hors population : sans âge, ni la borne
            // basse ni la borne haute ne peuvent être établies.
            let age_secs = age_secs(&pr.created_at, now)?;
            if age_secs <= cfg.min_age_secs || age_secs >= cfg.max_age_secs {
                return None;
            }
            // SHA vide ⇒ hors population (mika#2347). La clé du ledger est
            // `pr:{repo}#{n}@{sha}` : un SHA vide ferait de toutes les PRs sans
            // SHA un unique compteur partagé, donc un cooldown et un budget qui
            // ne discriminent plus rien.
            let head_sha = pr.head_ref_oid.trim();
            if head_sha.is_empty() {
                return None;
            }
            Some(PrRef {
                number: pr.number,
                age_secs,
                head_sha: head_sha.to_string(),
            })
        })
        .collect();

    retained.sort_by(|a, b| b.age_secs.cmp(&a.age_secs).then(a.number.cmp(&b.number)));
    retained
}

/// Deux logins désignent-ils la même identité de forge ?
///
/// **GitHub rend la même identité de deux façons** : `mika-platform-dev` quand
/// le compte agit sous PAT, `mika-platform-dev[bot]` quand il agit sous
/// l'identité App — et le repli App est un chemin nominal depuis mika#2205. Une
/// comparaison brute écarterait donc **toutes** les PRs ouvertes par ce chemin,
/// et ce module serait silencieusement inerte exactement là où il doit servir.
/// Le symétrique est aussi vrai et plus dangereux : une revue de
/// `mika-platform-qa[bot]` non reconnue ferait re-demander une PR déjà revue,
/// c'est-à-dire produirait la revue en double que le conditionnement existe pour
/// éviter.
///
/// La normalisation est empruntée à [`crate::ready_label::normalize_login`],
/// module écrit en partie pour cette raison (« GitHub rend
/// `mika-platform-dev[bot]` là où la configuration porte `mika-platform-dev`, et
/// l'évidence du ticket cite les deux formes »), plutôt que réécrite : deux
/// normalisations dériveraient en silence le jour où GitHub change de rendu.
fn is_login(actual: &str, expected: &str) -> bool {
    crate::ready_label::normalize_login(actual) == crate::ready_label::normalize_login(expected)
}

/// Âge en secondes pleines. `None` quand `created_at` est illisible.
///
/// Un horodatage dans le futur (dérive d'horloge) est ramené à 0, donc plus
/// jeune que `min_age_secs` : la PR sort de la population, ce qui est la
/// direction sûre.
fn age_secs(created_at: &str, now: DateTime<Utc>) -> Option<i64> {
    let created = crate::timestamp::parse(created_at).ok()?;
    Some((now - created).num_seconds().max(0))
}

// ---------------------------------------------------------------------------
// Lecture de la configuration (trois paliers)
// ---------------------------------------------------------------------------

fn parse_positive_i64(raw: Option<&str>, default: i64, env_name: &str) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n > 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default,
                    "qa_review_reconcile: {env_name} illisible ou non positif, défaut appliqué"
                );
                default
            }
        },
        _ => default,
    }
}

fn parse_positive_usize(raw: Option<&str>, default: usize, env_name: &str) -> usize {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default,
                    "qa_review_reconcile: {env_name} illisible ou non positif, défaut appliqué"
                );
                default
            }
        },
        _ => default,
    }
}

/// Liste de dépôts, séparés par des virgules. Les entrées vides sont ignorées ;
/// une valeur entièrement vide retombe sur [`DEFAULT_REPOS`].
fn parse_repos(raw: Option<&str>) -> Vec<String> {
    let parsed: Vec<String> = raw
        .unwrap_or(DEFAULT_REPOS)
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if parsed.is_empty() {
        vec![DEFAULT_REPOS.to_string()]
    } else {
        parsed
    }
}

/// Lit les trois bornes depuis l'environnement.
///
/// Une fenêtre vide (`max_age <= min_age`) est **dite**, pas corrigée : le scan
/// ne retiendrait alors jamais rien, et un scan silencieusement inactif se lit
/// exactement comme un scan qui n'a rien trouvé à faire (mika#2205).
fn config_from_env() -> ReconcileConfig {
    let cfg = ReconcileConfig {
        min_age_secs: parse_positive_i64(
            std::env::var(MIN_AGE_ENV).ok().as_deref(),
            MIN_AGE_DEFAULT_SECS,
            MIN_AGE_ENV,
        ),
        max_age_secs: parse_positive_i64(
            std::env::var(MAX_AGE_ENV).ok().as_deref(),
            MAX_AGE_DEFAULT_SECS,
            MAX_AGE_ENV,
        ),
        max_per_tick: parse_positive_usize(
            std::env::var(MAX_PER_TICK_ENV).ok().as_deref(),
            MAX_PER_TICK_DEFAULT,
            MAX_PER_TICK_ENV,
        ),
        cooldown_secs: parse_positive_i64(
            std::env::var(COOLDOWN_ENV).ok().as_deref(),
            COOLDOWN_DEFAULT_SECS,
            COOLDOWN_ENV,
        ),
        max_attempts: parse_positive_i64(
            std::env::var(MAX_ATTEMPTS_ENV).ok().as_deref(),
            MAX_ATTEMPTS_DEFAULT,
            MAX_ATTEMPTS_ENV,
        ),
    };
    if cfg.max_age_secs <= cfg.min_age_secs {
        warn!(
            event = "qa_review_reconcile_empty_window",
            min_age_secs = cfg.min_age_secs,
            max_age_secs = cfg.max_age_secs,
            "fenêtre d'âge vide : aucune PR ne peut être retenue ; \
             corriger {MIN_AGE_ENV} / {MAX_AGE_ENV}"
        );
    }
    cfg
}

// ---------------------------------------------------------------------------
// Le ledger devient décisionnel (mika#2347)
// ---------------------------------------------------------------------------

/// Clé d'audit d'une pose : `pr:{repo}#{n}@{sha}`.
///
/// La forme exacte que `ci_success_handler` emploie déjà pour sa dedup durable
/// (mika#1869). Le préfixe reste `pr:{repo}#{n}`, donc la requête opérateur
/// `WHERE tool_name = 'qa_review_reconciled'` est inchangée ; seul le format de
/// `target_key` s'allonge.
pub fn reconciled_audit_key(repo: &str, pr_number: u64, head_sha: &str) -> String {
    format!("pr:{repo}#{pr_number}@{head_sha}")
}

/// Clé d'audit **héritée**, écrite par mika#2334 avant que le SHA n'existe.
///
/// Daté : ce second format peut disparaître dès que toute ligne écrite avant le
/// déploiement de mika#2347 est plus vieille que `max_age_secs` (sept jours par
/// défaut). Il est consulté en **deuxième requête exacte**, jamais par un `LIKE`
/// — qui ferait matcher `#234` sur `#2343`.
fn legacy_reconciled_audit_key(repo: &str, pr_number: u64) -> String {
    format!("pr:{repo}#{pr_number}")
}

/// Borne basse d'une fenêtre de comptage, en ISO 8601, **sans jamais paniquer**.
///
/// `chrono::Duration::seconds` panique au-delà de `i64::MAX / 1000` : des
/// millisecondes tapées là où des secondes étaient attendues tueraient le tick
/// du moteur depuis une variable d'environnement. Même discipline que le plafond
/// absolu de `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS`.
fn window_start(now: DateTime<Utc>, secs: i64) -> String {
    let delta = chrono::TimeDelta::try_seconds(secs).unwrap_or(chrono::TimeDelta::MAX);
    crate::timestamp::format(
        &now.checked_sub_signed(delta)
            .unwrap_or(DateTime::<Utc>::MIN_UTC),
    )
}

/// Ce que le ledger répond pour un (dépôt, PR, SHA) donné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewLedgerVerdict {
    /// Aucune pose récente, budget non épuisé : on peut poser.
    Postable,
    /// Une pose existe à l'intérieur du cooldown : on saute ce tick.
    Cooldown,
    /// Le budget est épuisé pour ce SHA : la PR est abandonnée jusqu'au prochain
    /// SHA, qui rouvrira le budget en changeant la clé.
    Abandoned { attempts: i64 },
    /// `audit_events` est illisible. **Fail-closed** : on ne pose pas.
    Unreadable,
}

/// Lit le ledger avant toute pose — l'idempotence, le cooldown et le budget.
///
/// # Ce que ça ferme
///
/// Avant mika#2347, les seuls termes d'idempotence étaient *l'absence de demande*
/// et *l'absence de revue* pour [`REVIEWER_FORGE_LOGIN`] — deux états **GitHub
/// qui n'existent qu'après aboutissement de la revue**. Une PR dont le tour de
/// revue meurt sans rien poster retombait donc dans la population au tick
/// suivant, identique à elle-même : avec le cron `0 */15 * * * *`, jusqu'à **96
/// re-demandes par jour et par PR** pendant les sept jours de `max_age_secs`.
/// La ligne d'audit `qa_review_reconciled` était écrite et **jamais relue** : le
/// ledger existait et ne décidait rien.
///
/// # Fail-closed, à l'inverse de `ci_success_handler`
///
/// Une lecture impossible refuse la pose. C'est le choix de `wip_rescue`
/// (mika#2199) et pas celui de `ci_success_handler` (fail-open), pour la raison
/// qui y est écrite : un faux négatif fait attendre une PR qui, avant mika#2334,
/// attendait indéfiniment ; un faux positif rejoue une revue, c'est-à-dire
/// produit exactement le défaut que ce ticket existe pour fermer.
///
/// Aucune nouvelle méthode DB, aucune migration : deux (parfois trois) lectures
/// de [`crate::db::Database::count_recent_audit_events_for_target`].
pub async fn review_ledger_verdict(
    db: &AsyncDatabase,
    repo: &str,
    pr: &PrRef,
    cfg: &ReconcileConfig,
    now: DateTime<Utc>,
    trace_id: &str,
) -> ReviewLedgerVerdict {
    let key = reconciled_audit_key(repo, pr.number, &pr.head_sha);

    // Budget : toutes les poses pour ce SHA sur la durée de vie scannable de la
    // PR. Au-delà de `max_attempts`, abandon définitif pour ce SHA.
    let budget_since = window_start(now, cfg.max_age_secs);
    let attempts = match db
        .count_recent_audit_events_for_target(RECONCILED_TOOL, &key, &budget_since)
        .await
    {
        Ok(n) => n,
        Err(e) => return unreadable(repo, pr, "budget", &e, trace_id),
    };
    if attempts >= cfg.max_attempts {
        return ReviewLedgerVerdict::Abandoned { attempts };
    }

    let cooldown_since = window_start(now, cfg.cooldown_secs);

    // La fenêtre du budget contient celle du cooldown, donc `attempts == 0`
    // implique « aucune pose récente » : la requête est évitée plutôt que
    // répétée. `cooldown_secs > max_age_secs` est une configuration incohérente
    // que ce raccourci lit dans la direction sûre (on pose).
    if attempts > 0 {
        match db
            .count_recent_audit_events_for_target(RECONCILED_TOOL, &key, &cooldown_since)
            .await
        {
            Ok(n) if n > 0 => return ReviewLedgerVerdict::Cooldown,
            Ok(_) => {}
            Err(e) => return unreadable(repo, pr, "cooldown", &e, trace_id),
        }
    }

    // Transition des lignes héritées : les poses écrites avant mika#2347 portent
    // `pr:{repo}#{n}` sans SHA et seraient invisibles au cooldown ci-dessus, ce
    // qui autoriserait une re-pose immédiate sur des PRs déjà rattrapées.
    let legacy_key = legacy_reconciled_audit_key(repo, pr.number);
    match db
        .count_recent_audit_events_for_target(RECONCILED_TOOL, &legacy_key, &cooldown_since)
        .await
    {
        Ok(n) if n > 0 => ReviewLedgerVerdict::Cooldown,
        Ok(_) => ReviewLedgerVerdict::Postable,
        Err(e) => unreadable(repo, pr, "legacy_cooldown", &e, trace_id),
    }
}

/// Le seul site qui transforme une erreur de lecture d'audit en verdict.
///
/// **Doit rester vide en régime nominal** : toute occurrence est un ledger
/// illisible, donc un scan devenu inerte.
fn unreadable(
    repo: &str,
    pr: &PrRef,
    stage: &str,
    error: &anyhow::Error,
    trace_id: &str,
) -> ReviewLedgerVerdict {
    warn!(
        event = "qa_review_reconcile_ledger_unreadable",
        repo = %repo,
        pr = pr.number,
        head_sha = %pr.head_sha,
        stage,
        error = %error,
        trace_id,
        "qa_review_reconcile: ledger illisible, pose refusée (fail-closed)"
    );
    ReviewLedgerVerdict::Unreadable
}

// ---------------------------------------------------------------------------
// Exécution — appelant mince autour de la décision
// ---------------------------------------------------------------------------

/// `gh` borné par un timeout, même forme que `wip_rescue::gh`.
async fn gh(args: &[&str], token: &str) -> Result<String, String> {
    match tokio::time::timeout(GH_TIMEOUT, run_gh_subprocess(args, token)).await {
        Ok(res) => res,
        Err(_) => Err(format!("gh timed out after {}s", GH_TIMEOUT.as_secs())),
    }
}

/// Taille de page de l'unique `gh pr list` par dépôt et par tick.
const LIST_LIMIT: usize = 100;

/// Un seul `gh pr list` par dépôt et par tick — le coût API est constant.
///
/// **La troncature va dans le mauvais sens, et c'est pourquoi elle est dite.**
/// `gh pr list` rend les PRs de la plus récente à la plus ancienne, alors que ce
/// scan sert les plus **anciennes** d'abord : une page pleine ne coupe donc pas
/// une queue indifférente, elle coupe exactement la population visée. Le remède
/// est un réglage d'exploitation (relever la limite, ou réduire le nombre de PRs
/// ouvertes), pas une décision que ce module puisse prendre seul — mais une
/// troncature muette rendrait le scan inerte sans que rien ne le dise, ce qui est
/// la forme de panne que tout ce ticket existe pour fermer.
async fn list_open_prs(repo: &str, token: &str) -> Result<Vec<PrSnapshot>, String> {
    let limit = LIST_LIMIT.to_string();
    let out = gh(
        &[
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--json",
            "number,author,isDraft,createdAt,headRefOid,reviewRequests,reviews",
            "--limit",
            &limit,
        ],
        token,
    )
    .await?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let prs: Vec<PrSnapshot> =
        serde_json::from_str(trimmed).map_err(|e| format!("parse gh pr list ({repo}): {e}"))?;
    if prs.len() >= LIST_LIMIT {
        warn!(
            event = "qa_review_reconcile_page_full",
            repo = %repo,
            limit = LIST_LIMIT,
            "page pleine : les PRs les plus anciennes du dépôt peuvent être \
             invisibles à ce scan, et ce sont celles qu'il vise"
        );
    }
    Ok(prs)
}

/// Scanne les dépôts configurés et pose [`REVIEWER_FORGE_LOGIN`] sur les PRs de
/// la boucle qu'aucune revue n'est venue chercher.
///
/// Rend `Some(n)` quand `n > 0` PRs ont été rattrapées, `None` sinon —
/// **zéro action, zéro ligne** (doctrine mika#2131 : un scan qui journalise tout
/// le monde ne distingue plus personne).
///
/// Fail-open de bout en bout : aucun échec de ce scan ne fait échouer le tick du
/// moteur.
pub async fn reconcile_qa_review_requests(
    db: &AsyncDatabase,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> Option<usize> {
    let cfg = config_from_env();
    let repos = parse_repos(std::env::var(REPOS_ENV).ok().as_deref());
    let now = Utc::now();

    let mut budget = cfg.max_per_tick;
    let mut reconciled = 0usize;
    let mut failed = 0usize;
    let mut skipped_cooldown = 0usize;
    let mut abandoned = 0usize;

    for repo in &repos {
        if budget == 0 {
            break;
        }
        let prs = match list_open_prs(repo, github_token).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    event = "qa_review_reconcile_error",
                    repo = %repo,
                    error = %e,
                    trace_id,
                    "qa_review_reconcile: lecture des PRs impossible, dépôt sauté ce tick"
                );
                continue;
            }
        };

        // Liste complète, non tronquée (mika#2347) : le cap s'applique après le
        // filtre de ledger, sur les seules tentatives de pose.
        let selected = select_prs_needing_review(&prs, now, &cfg);

        for pr in selected {
            if budget == 0 {
                break;
            }

            // Le budget de tick n'est débité que sur **tentative de pose** : un
            // saut de cooldown ne coûte rien, sans quoi le cap redeviendrait un
            // plafond sur les sauts plutôt que sur les écritures.
            match review_ledger_verdict(db, repo, &pr, &cfg, now, trace_id).await {
                ReviewLedgerVerdict::Postable => {}
                ReviewLedgerVerdict::Cooldown => {
                    skipped_cooldown += 1;
                    continue;
                }
                ReviewLedgerVerdict::Unreadable => continue,
                ReviewLedgerVerdict::Abandoned { attempts } => {
                    abandoned += 1;
                    log_abandoned_once(db, session_id, repo, &pr, attempts, &cfg, now, trace_id)
                        .await;
                    continue;
                }
            }

            // Débité à **chaque tentative**, pas aux seuls succès. Un échec
            // systématique — la famille mika#2228, `Resource not accessible by
            // personal access token` — laisserait sinon le budget intact, et
            // chaque dépôt suivant se verrait réoffrir le quota entier : jusqu'à
            // `max_per_tick × dépôts` écritures dans un tick censé en plafonner
            // `max_per_tick`. C'est précisément quand la forge refuse qu'il ne
            // faut pas la marteler.
            budget -= 1;
            match request_review(repo, pr.number, github_token).await {
                Ok(()) => {
                    reconciled += 1;
                    info!(
                        event = RECONCILED_TOOL,
                        repo = %repo,
                        pr = pr.number,
                        age_secs = pr.age_secs,
                        head_sha = %pr.head_sha,
                        reviewer = REVIEWER_FORGE_LOGIN,
                        trace_id,
                        "qa_review_reconcile: relecteur posé sur une PR que la cascade n'a pas atteinte"
                    );
                    log_reconciled(db, session_id, repo, &pr, trace_id).await;
                }
                Err(e) => {
                    failed += 1;
                    // Nom d'événement dédié : l'écriture `requested_reviewers`
                    // est de la même famille que l'écriture de label, qui échoue
                    // déjà sous PAT (`Resource not accessible by personal access
                    // token`, mika#2228). Un échec systématique ici est une
                    // question d'identité et de scope de jeton, et doit être
                    // greppable séparément d'une erreur `gh` quelconque.
                    warn!(
                        event = "qa_review_request_failed",
                        repo = %repo,
                        pr = pr.number,
                        error = %e,
                        trace_id,
                        "qa_review_reconcile: pose du relecteur refusée"
                    );
                }
            }
        }
    }

    if reconciled == 0 && failed == 0 {
        debug!(trace_id, "qa_review_reconcile: rien à rattraper ce tick");
        return None;
    }

    info!(
        event = "qa_review_reconcile_tick",
        reconciled,
        failed,
        // mika#2347 — la forme du tick, sans nommer de PR : un INFO par PR sautée
        // serait du bruit à chaque tick (doctrine mika#2131). Le détail par PR
        // vit dans `audit_events`.
        skipped_cooldown,
        abandoned,
        repos = repos.len(),
        trace_id,
        "qa_review_reconcile: tick agissant"
    );

    (reconciled > 0).then_some(reconciled)
}

/// `gh pr edit --add-reviewer`. Le `review_requested` qui en résulte passe
/// `is_suppressed_review_request` (mika#1655) puisque le relecteur est
/// exactement [`REVIEWER_FORGE_LOGIN`] — le chemin de déclenchement existe déjà
/// et est testé côté gateway, rien n'est à ajouter là-bas (AC9).
async fn request_review(repo: &str, pr_number: u64, token: &str) -> Result<(), String> {
    let number = pr_number.to_string();
    gh(
        &[
            "pr",
            "edit",
            &number,
            "--repo",
            repo,
            "--add-reviewer",
            REVIEWER_FORGE_LOGIN,
        ],
        token,
    )
    .await
    .map(|_| ())
}

/// Écrit la ligne de pose. **C'est cette ligne que [`review_ledger_verdict`]
/// relit** — le ledger n'est plus décoratif depuis mika#2347.
pub async fn log_reconciled(
    db: &AsyncDatabase,
    session_id: &str,
    repo: &str,
    pr: &PrRef,
    trace_id: &str,
) {
    let key = reconciled_audit_key(repo, pr.number, &pr.head_sha);
    let age = pr.age_secs.to_string();
    if let Err(e) = db
        .log_audit_event(
            session_id,
            RECONCILED_TOOL,
            &key,
            None,
            Some(&age),
            Some("demande de revue rattrapée : PR ouverte sans revue ni demande (mika#2334)"),
            Some(trace_id),
        )
        .await
    {
        warn!(
            pr = pr.number,
            error = %e,
            trace_id,
            "qa_review_reconcile: audit write failed"
        );
    }
}

/// Dit l'abandon **une fois par (dépôt, PR, SHA)**, pas une fois par tick.
///
/// Un abandon dure jusqu'au prochain SHA, c'est-à-dire potentiellement les sept
/// jours de `max_age_secs`. Écrire à chaque tick y déverserait ~670 lignes par
/// PR — le churn que ce ticket borne, déplacé dans la table d'audit. La
/// déduplication est celle de mika#2131 : l'information est « cette PR est
/// abandonnée pour ce SHA », pas « elle l'était encore à 14h32 ». La marque
/// n'est posée qu'après une écriture réussie.
///
/// Lecture impossible ⇒ on n'écrit pas : la ligne existe très probablement déjà
/// (c'est le cas nominal d'un abandon répété), et l'agrégat `abandoned` du tick
/// continue de le compter.
#[allow(clippy::too_many_arguments)]
async fn log_abandoned_once(
    db: &AsyncDatabase,
    session_id: &str,
    repo: &str,
    pr: &PrRef,
    attempts: i64,
    cfg: &ReconcileConfig,
    now: DateTime<Utc>,
    trace_id: &str,
) {
    let key = reconciled_audit_key(repo, pr.number, &pr.head_sha);
    let since = window_start(now, cfg.max_age_secs);
    match db
        .count_recent_audit_events_for_target(ABANDONED_TOOL, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(e) => {
            debug!(
                pr = pr.number,
                error = %e,
                trace_id,
                "qa_review_reconcile: relecture du marqueur d'abandon impossible, écriture sautée"
            );
            return;
        }
    }

    warn!(
        event = "qa_review_reconcile_abandoned",
        repo = %repo,
        pr = pr.number,
        head_sha = %pr.head_sha,
        attempts,
        max_attempts = cfg.max_attempts,
        trace_id,
        "qa_review_reconcile: budget de rattrapage épuisé pour ce SHA — \
         deux rattrapages n'ont produit aucune revue, le chemin nominal est \
         probablement cassé en amont ; un nouveau SHA rouvrira le budget"
    );

    let reasoning = format!(
        "budget de rattrapage épuisé : {attempts} pose(s) pour ce (dépôt, PR, SHA), \
         plafond {} (mika#2347)",
        cfg.max_attempts
    );
    if let Err(e) = db
        .log_audit_event(
            session_id,
            ABANDONED_TOOL,
            &key,
            None,
            Some(&attempts.to_string()),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            pr = pr.number,
            error = %e,
            trace_id,
            "qa_review_reconcile: audit write failed (abandon)"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        crate::timestamp::parse("2026-09-15T18:00:00Z").unwrap()
    }

    /// `created_at` d'une PR âgée de `secs` secondes à [`now`].
    fn created_secs_ago(secs: i64) -> String {
        crate::timestamp::format(&(now() - chrono::Duration::seconds(secs)))
    }

    fn loop_pr(number: u64, age_secs: i64) -> PrSnapshot {
        PrSnapshot {
            number,
            author: Some(GhAuthor {
                login: DISPATCHER_FORGE_LOGIN.to_string(),
            }),
            is_draft: false,
            created_at: created_secs_ago(age_secs),
            head_ref_oid: format!("{number:040x}"),
            review_requests: vec![],
            reviews: vec![],
        }
    }

    fn select(prs: &[PrSnapshot]) -> Vec<u64> {
        select_prs_needing_review(prs, now(), &ReconcileConfig::default())
            .into_iter()
            .map(|p| p.number)
            .collect()
    }

    /// AC1 — le test négatif du ticket. Une PR poussée puis abandonnée par son
    /// pilote, quelle que soit la façon dont ce pilote est mort, est retenue.
    #[test]
    fn mika2334_le_test_negatif_du_ticket_passe() {
        assert_eq!(select(&[loop_pr(2333, 7200)]), vec![2333]);
    }

    /// AC2 — idempotence, les deux moitiés. Une demande déjà posée **ou** une
    /// revue déjà rendue sortent la PR, quel que soit son âge.
    #[test]
    fn mika2334_aucune_revue_en_double() {
        let mut deja_demandee = loop_pr(1, 7200);
        deja_demandee.review_requests = vec![GhReviewRequest {
            login: Some(REVIEWER_FORGE_LOGIN.to_string()),
        }];

        let mut deja_revue = loop_pr(2, 500_000);
        deja_revue.reviews = vec![GhReview {
            author: Some(GhAuthor {
                login: REVIEWER_FORGE_LOGIN.to_string(),
            }),
        }];

        assert!(select(&[deja_demandee, deja_revue]).is_empty());
    }

    /// La comparaison de login est insensible à la casse des deux côtés — GitHub
    /// ne garantit pas la casse qu'il rend.
    #[test]
    fn mika2334_les_logins_se_comparent_sans_casse() {
        let mut pr = loop_pr(3, 7200);
        pr.author = Some(GhAuthor {
            login: DISPATCHER_FORGE_LOGIN.to_uppercase(),
        });
        pr.review_requests = vec![GhReviewRequest {
            login: Some(REVIEWER_FORGE_LOGIN.to_uppercase()),
        }];
        assert!(select(&[pr]).is_empty());
    }

    /// **La forme `[bot]` est la même identité.** GitHub rend
    /// `mika-platform-dev` quand le compte agit sous PAT et
    /// `mika-platform-dev[bot]` quand il agit sous l'identité App — et le repli
    /// App est un chemin nominal depuis mika#2205. Sans normalisation, ce scan
    /// écarterait **toutes** les PRs ouvertes par ce chemin : il serait
    /// silencieusement inerte exactement là où il doit servir, c'est-à-dire
    /// reproduirait la forme de panne que ce ticket existe pour fermer.
    /// Précédent daté : `ready_label::normalize_login`, dont la normalisation est
    /// empruntée plutôt que réécrite.
    #[test]
    fn mika2334_lauteur_en_forme_bot_est_la_meme_identite() {
        let mut pr = loop_pr(13, 7200);
        pr.author = Some(GhAuthor {
            login: format!("{DISPATCHER_FORGE_LOGIN}[bot]"),
        });
        assert_eq!(
            select(&[pr]),
            vec![13],
            "une PR ouverte sous l'identité App doit rester dans la population"
        );
    }

    /// Le symétrique, et il est plus dangereux : une demande ou une revue de
    /// `mika-platform-qa[bot]` non reconnue ferait re-demander une PR déjà
    /// servie — c'est-à-dire produirait exactement la revue en double que le
    /// conditionnement existe pour éviter. Les deux moitiés de l'idempotence
    /// sont couvertes, parce qu'elles lisent deux champs distincts.
    #[test]
    fn mika2334_le_relecteur_en_forme_bot_sort_la_pr() {
        let mut demandee = loop_pr(14, 7200);
        demandee.review_requests = vec![GhReviewRequest {
            login: Some(format!("{REVIEWER_FORGE_LOGIN}[bot]")),
        }];

        let mut revue = loop_pr(15, 7200);
        revue.reviews = vec![GhReview {
            author: Some(GhAuthor {
                login: format!("{REVIEWER_FORGE_LOGIN}[BOT]"),
            }),
        }];

        assert!(
            select(&[demandee, revue]).is_empty(),
            "une PR déjà servie sous l'identité App ne doit pas être re-servie"
        );
    }

    /// Le contrôle négatif du trio d'identité ci-dessus : la normalisation ne
    /// doit pas rendre tout le monde égal à tout le monde. Sans lui, un
    /// `is_login` qui rendrait toujours `true` passerait les trois.
    #[test]
    fn mika2334_la_normalisation_ne_confond_pas_deux_identites() {
        assert!(!is_login("samidarko[bot]", DISPATCHER_FORGE_LOGIN));
        assert!(!is_login(
            &format!("{REVIEWER_FORGE_LOGIN}[bot]"),
            DISPATCHER_FORGE_LOGIN
        ));
        assert!(!is_login("", DISPATCHER_FORGE_LOGIN));
    }

    /// AC3 — les brouillons ont leur propre voie (`wip_rescue`), y compris une
    /// PR de rescue délibérément tenue en brouillon.
    #[test]
    fn mika2334_les_drafts_sont_hors_population() {
        let mut draft = loop_pr(4, 7200);
        draft.is_draft = true;
        assert!(select(&[draft]).is_empty());
    }

    /// AC4 — une PR humaine n'est pas la boucle.
    #[test]
    fn mika2334_les_prs_humaines_sont_hors_population() {
        let mut humaine = loop_pr(5, 7200);
        humaine.author = Some(GhAuthor {
            login: "samidarko".to_string(),
        });
        assert!(select(&[humaine]).is_empty());
    }

    /// AC5 — la fenêtre d'âge est respectée des deux côtés.
    #[test]
    fn mika2334_la_fenetre_d_age_a_deux_bords() {
        let trop_jeune = loop_pr(6, MIN_AGE_DEFAULT_SECS - 1);
        let trop_vieille = loop_pr(7, MAX_AGE_DEFAULT_SECS + 1);
        let dedans = loop_pr(8, MIN_AGE_DEFAULT_SECS + 1);
        assert_eq!(select(&[trop_jeune, trop_vieille, dedans]), vec![8]);
    }

    /// AC6 — les plus vieilles d'abord, ordre total et stable.
    #[test]
    fn mika2334_la_selection_sert_les_plus_vieilles_dabord() {
        let prs: Vec<PrSnapshot> = (1..=6)
            .map(|i| loop_pr(i, MIN_AGE_DEFAULT_SECS + 100 * i as i64))
            .collect();
        let picked: Vec<u64> = select_prs_needing_review(&prs, now(), &ReconcileConfig::default())
            .into_iter()
            .map(|p| p.number)
            .collect();
        assert_eq!(picked, vec![6, 5, 4, 3, 2, 1]);
    }

    /// mika#2347 — **l'assertion de troncature n'est pas perdue, elle a déménagé
    /// avec la décision qu'elle garde.** Si la fonction pure tronquait encore,
    /// `max_per_tick` PRs en cooldown consommeraient tout le tick et la suivante,
    /// légitimement rattrapable, ne serait jamais vue. Le cap vit désormais dans
    /// l'appelant, après le filtre de ledger, et ne plafonne que les écritures.
    #[test]
    fn mika2347_la_fonction_pure_ne_tronque_plus() {
        let prs: Vec<PrSnapshot> = (1..=6)
            .map(|i| loop_pr(i, MIN_AGE_DEFAULT_SECS + 100 * i as i64))
            .collect();
        for max_per_tick in [1, 3, 100] {
            let cfg = ReconcileConfig {
                max_per_tick,
                ..ReconcileConfig::default()
            };
            assert_eq!(
                select_prs_needing_review(&prs, now(), &cfg).len(),
                6,
                "max_per_tick={max_per_tick} ne doit plus influer sur la sélection"
            );
        }
    }

    /// Le cap par défaut est **1** depuis mika#2347 : 4 demandes/heure au plus,
    /// sous la capacité de 6 revues/heure établie par la mesure. C'est une valeur
    /// de contrat, pas un détail — d'où l'assertion.
    #[test]
    fn mika2347_le_cap_par_defaut_est_un() {
        assert_eq!(ReconcileConfig::default().max_per_tick, 1);
    }

    /// Un SHA vide sort la PR : la clé du ledger ne discriminerait plus rien.
    #[test]
    fn mika2347_un_sha_illisible_sort_la_pr() {
        let mut sans_sha = loop_pr(16, 7200);
        sans_sha.head_ref_oid = "   ".to_string();
        assert!(select(&[sans_sha]).is_empty());
    }

    /// `headRefOid` absent ⇒ erreur de parsing, jamais une chaîne vide — même
    /// discipline que `reviewRequests` / `reviews`.
    #[test]
    fn mika2347_le_sha_manquant_est_une_erreur_pas_un_vide() {
        let sans_sha = r#"[{"number":1,"author":{"login":"mika-platform-dev"},
            "isDraft":false,"createdAt":"2026-09-15T10:00:00Z",
            "reviewRequests":[],"reviews":[]}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(sans_sha).is_err());
    }

    /// Les deux clés d'audit, l'ancienne et la nouvelle, et le préfixe partagé
    /// qui laisse la requête opérateur inchangée.
    #[test]
    fn mika2347_la_cle_daudit_porte_le_sha_sans_perdre_son_prefixe() {
        let key = reconciled_audit_key("senara-solutions/mika", 2343, "deadbeef");
        assert_eq!(key, "pr:senara-solutions/mika#2343@deadbeef");
        let legacy = legacy_reconciled_audit_key("senara-solutions/mika", 2343);
        assert_eq!(legacy, "pr:senara-solutions/mika#2343");
        assert!(
            key.starts_with(&legacy),
            "le préfixe doit rester `pr:{{repo}}#{{n}}`"
        );
    }

    /// `chrono::Duration::seconds` panique au-delà de `i64::MAX / 1000` : une
    /// faute de frappe dans une variable d'environnement ne doit pas tuer le tick.
    #[test]
    fn mika2347_une_fenetre_absurde_ne_panique_pas() {
        let _ = window_start(now(), i64::MAX);
        let _ = window_start(now(), 3600);
    }

    /// Fail-safe : une information illisible sort la PR, elle ne l'y fait jamais
    /// entrer. Contrôle négatif du test AC1, qui passerait encore si la fonction
    /// retenait tout.
    #[test]
    fn mika2334_une_information_illisible_sort_la_pr() {
        let mut auteur_supprime = loop_pr(9, 7200);
        auteur_supprime.author = None;

        let mut date_illisible = loop_pr(10, 7200);
        date_illisible.created_at = "pas une date".to_string();

        assert!(select(&[auteur_supprime, date_illisible]).is_empty());
    }

    /// Une horloge qui recule ne doit pas fabriquer un âge géant.
    #[test]
    fn mika2334_une_pr_creee_dans_le_futur_est_traitee_comme_neuve() {
        let mut future = loop_pr(11, 7200);
        future.created_at = created_secs_ago(-7200);
        assert!(select(&[future]).is_empty());
    }

    /// Une demande d'équipe (`__typename: "Team"`, sans `login`) n'est pas le
    /// compte relecteur : elle ne doit pas sortir la PR de la population.
    #[test]
    fn mika2334_une_demande_d_equipe_ne_vaut_pas_demande_au_relecteur() {
        let mut pr = loop_pr(12, 7200);
        pr.review_requests = vec![GhReviewRequest { login: None }];
        assert_eq!(select(&[pr]), vec![12]);
    }

    /// AC9 — le signal posé est exactement celui que le filtre du gateway laisse
    /// passer. Si cette égalité casse, `is_suppressed_review_request` supprime
    /// l'événement et le rattrapage ne réveille personne.
    #[test]
    fn mika2334_le_relecteur_pose_est_celui_que_le_gateway_route() {
        assert_eq!(REVIEWER_FORGE_LOGIN, "mika-platform-qa");
        assert_ne!(REVIEWER_FORGE_LOGIN, DISPATCHER_FORGE_LOGIN);
    }

    /// `reviewRequests` / `reviews` absents ⇒ erreur de parsing, jamais un
    /// vecteur vide. Un `#[serde(default)]` ici ferait entrer une PR dans la
    /// population sur une information manquante.
    #[test]
    fn mika2334_un_champ_manquant_est_une_erreur_pas_un_vide() {
        let sans_reviews = r#"[{"number":1,"author":{"login":"mika-platform-dev"},
            "isDraft":false,"createdAt":"2026-09-15T10:00:00Z",
            "headRefOid":"abc123","reviewRequests":[]}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(sans_reviews).is_err());

        let complet = r#"[{"number":1,"author":{"login":"mika-platform-dev"},
            "isDraft":false,"createdAt":"2026-09-15T10:00:00Z",
            "headRefOid":"abc123","reviewRequests":[],"reviews":[]}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(complet).is_ok());
    }

    /// La forme réelle que `gh` rend pour une demande d'équipe et pour une revue
    /// d'auteur supprimé — les deux champs optionnels doivent tenir.
    #[test]
    fn mika2334_les_formes_gh_reelles_se_deserialisent() {
        let raw = r#"[{"number":2332,"author":{"login":"mika-platform-dev"},
            "isDraft":false,"createdAt":"2026-09-15T10:00:00Z",
            "headRefOid":"1f2e3d4c5b6a798807162534435261708f9e0d1c",
            "reviewRequests":[{"__typename":"Team","name":"core","slug":"core"}],
            "reviews":[{"author":null,"state":"COMMENTED"}]}]"#;
        let parsed: Vec<PrSnapshot> = serde_json::from_str(raw).expect("forme gh réelle");
        assert!(parsed[0].review_requests[0].login.is_none());
        assert!(parsed[0].reviews[0].author.is_none());
    }

    #[test]
    fn mika2334_les_trois_paliers_de_configuration() {
        // Absent / vide → défaut.
        assert_eq!(
            parse_positive_i64(None, MIN_AGE_DEFAULT_SECS, MIN_AGE_ENV),
            MIN_AGE_DEFAULT_SECS
        );
        assert_eq!(
            parse_positive_i64(Some("  "), MIN_AGE_DEFAULT_SECS, MIN_AGE_ENV),
            MIN_AGE_DEFAULT_SECS
        );
        // Illisible / 0 / négatif → défaut.
        for bad in ["abc", "0", "-1"] {
            assert_eq!(
                parse_positive_i64(Some(bad), MIN_AGE_DEFAULT_SECS, MIN_AGE_ENV),
                MIN_AGE_DEFAULT_SECS,
                "{bad} doit retomber sur le défaut"
            );
            assert_eq!(
                parse_positive_usize(Some(bad), MAX_PER_TICK_DEFAULT, MAX_PER_TICK_ENV),
                MAX_PER_TICK_DEFAULT,
                "{bad} doit retomber sur le défaut"
            );
        }
        // Valide → utilisé.
        assert_eq!(parse_positive_i64(Some("60"), 3600, MIN_AGE_ENV), 60);
        assert_eq!(parse_positive_usize(Some("7"), 3, MAX_PER_TICK_ENV), 7);
    }

    /// AC7 — les deux clés de mika#2347 suivent le palier maison, et **`0` ne
    /// désarme pas** : c'est le rôle de `MIKA_QA_REVIEW_RECONCILE`. Une lecture
    /// inverse ferait d'une faute de frappe un désarmement silencieux — ici, un
    /// cooldown de zéro rendrait le rejeu immédiat, c'est-à-dire reproduirait
    /// exactement le défaut que ce ticket ferme.
    #[test]
    fn mika2347_les_deux_nouvelles_cles_suivent_les_trois_paliers() {
        for (default, env) in [
            (COOLDOWN_DEFAULT_SECS, COOLDOWN_ENV),
            (MAX_ATTEMPTS_DEFAULT, MAX_ATTEMPTS_ENV),
        ] {
            assert_eq!(parse_positive_i64(None, default, env), default);
            assert_eq!(parse_positive_i64(Some(""), default, env), default);
            for bad in ["abc", "0", "-1"] {
                assert_eq!(
                    parse_positive_i64(Some(bad), default, env),
                    default,
                    "{env}={bad} doit retomber sur le défaut, jamais désarmer"
                );
            }
        }
        assert_eq!(
            parse_positive_i64(Some("900"), COOLDOWN_DEFAULT_SECS, COOLDOWN_ENV),
            900
        );
        assert_eq!(
            parse_positive_i64(Some("5"), MAX_ATTEMPTS_DEFAULT, MAX_ATTEMPTS_ENV),
            5
        );
    }

    /// AC8 — chaque pose écrit une ligne d'audit sous le nom dont ce module est
    /// **seul writer**, clé `pr:<repo>#<n>@<sha>` depuis mika#2347. C'est cette
    /// ligne qui fait de `SELECT … WHERE tool_name = 'qa_review_reconciled'` la
    /// liste exacte des PRs que la boucle a dû rattraper — et, depuis mika#2347,
    /// c'est aussi celle que [`review_ledger_verdict`] relit.
    #[tokio::test]
    async fn mika2334_chaque_pose_ecrit_une_ligne_d_audit() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let pr = PrRef {
            number: 2333,
            age_secs: 7200,
            head_sha: "deadbeef".to_string(),
        };
        log_reconciled(&db, "session-2334", "senara-solutions/mika", &pr, "trace-1").await;

        let events = db
            .get_audit_events("session-2334")
            .await
            .expect("lecture des audit_events");
        let row = events
            .iter()
            .find(|e| e.tool_name == RECONCILED_TOOL)
            .expect("une ligne qa_review_reconciled doit exister");
        assert_eq!(row.target_key, "pr:senara-solutions/mika#2333@deadbeef");
        assert_eq!(row.after_value.as_deref(), Some("7200"));
    }

    #[test]
    fn mika2334_la_liste_de_depots_tolere_les_vides() {
        assert_eq!(parse_repos(None), vec![DEFAULT_REPOS]);
        assert_eq!(parse_repos(Some("   ")), vec![DEFAULT_REPOS]);
        assert_eq!(parse_repos(Some(",,")), vec![DEFAULT_REPOS]);
        assert_eq!(
            parse_repos(Some("a/b, c/d ,")),
            vec!["a/b".to_string(), "c/d".to_string()]
        );
    }
}

//! Les worktrees de PR terminale ne survivent plus à leur PR (mika#2420).
//!
//! # Le défaut que ce module ferme
//!
//! Le 2026-09-20, trois vagues de purge manuelle dans la même journée : ~08:35Z
//! (`fix-1951` + `fix-1952`, ~67 Go), ~09:xx (onze worktrees de PR mergées,
//! `/data` 77 % → 55 %), 12:08Z (`feat-1883`, `fix-2413`, `fix-1925`, ~93 Go).
//! Entre deux vagues, `/data` remonte à 80-82 % en ~3 h — chaque build telemetry
//! pèse 25 à 44 Go et deux pilotes tournent. Au-delà de 85 %, les builds
//! s'arrêtent et la boucle entière se bloque. **La purge manuelle est un remède
//! à demi-vie de trois heures.**
//!
//! # Ce que ce module rectifie du ticket, et c'est le premier livrable
//!
//! Le ticket pose que l'art antérieur **#1694 est « fermé mais inefficace »** et
//! propose trois hypothèses : la logique a régressé, elle ne couvre pas les
//! `target/`, ou elle ne se déclenche qu'à la création du worktree. **Les trois
//! sont fausses.**
//!
//! | affirmation du ticket | ce que le dépôt dit |
//! |---|---|
//! | #1694 est fermé | c'est un **dormeur**, ligne vivante de `docs/dormeurs.md`, dont la condition de réveil (« une branche `origin/*/1694/*` existe et porte un plan commité ») est désormais remplie |
//! | sa logique a régressé | elle n'a **jamais atteint `main`** — aucun `worktree_reaper` n'existait dans l'arbre |
//! | elle ne couvre pas les `target/` | elle ne couvrait rien : il n'y avait pas de code |
//!
//! Le commit `097cc66c` (2026-07-26) porte une implémentation réelle, sauvée en
//! `wip()` par la recovery dirty-worktree de mika#1282 et jamais promue. Son
//! architecture était à trois couches — A (audit), B (retrait en masse), C
//! (handler webhook `pull_request.closed`) — et son propre doc admettait que
//! « layers A/B clean up whatever slips through ». **Cette phrase est le
//! défaut** : la couche C est un handler sur un événement **unique, non
//! rejouable**, perdable en quatre endroits déjà mesurés par la maison (file
//! webhook bornée mika#1870, 429, circuit breaker → DLQ → `dead`, tour LLM vide
//! opposé une seule fois par `webhook_zero_tools`) — exactement la classe que
//! mika#2334 a dû fermer pour `pull_request.opened`. Et le rattrapage était
//! délégué à A et B, **qui sont manuels**. Le geste dont Vincent mesure la
//! demi-vie de trois heures *est* la couche B.
//!
//! # Scan seul, pas de hook
//!
//! Le ticket propose « un reaper périodique **ou** un hook post-merge ».
//! Tranché : **scan seul**, quatre raisons de poids décroissant.
//!
//! 1. **Le scan couvre la population déjà accumulée** — les onze worktrees
//!    mesurés. Un hook ne rattrape jamais ce qu'il a raté ; c'est la propriété
//!    qui a mis #1694 en échec.
//! 2. **La latence du hook est négligeable** devant le rythme mesuré : le disque
//!    monte d'environ 8 %/h, un tick de dix minutes coûte ~1,3 % de disque.
//! 3. **Un seul mécanisme est un seul endroit à déboguer.** Livrer un chemin
//!    *et* son filet quand le filet couvre tout le domaine du chemin est du
//!    YAGNI.
//! 4. **Doctrine maison** : *un filet, pas un chemin* (mika#2334). Un hook
//!    resterait possible plus tard **par-dessus** ce scan sans rien invalider ;
//!    l'inverse est faux — livrer le hook d'abord laisserait la dette intacte.
//!
//! # L'asymétrie décide tout, et elle s'écrit avant le reste
//!
//! Ce scan **supprime**. Les deux erreurs n'ont pas le même prix :
//!
//! | erreur | coût | réversible ? |
//! |---|---|---|
//! | faux négatif (on garde un worktree mort) | quelques dizaines de Go pendant dix minutes | oui — rattrapé au tick suivant |
//! | faux positif (on supprime un worktree vivant) | des heures de travail détruites dans un worktree de décision | **non** |
//!
//! **Conséquence : tous les termes du prédicat sont fail-safe vers *conserver*,
//! sans exception.** C'est la direction **inverse** du fail-closed de
//! `wip_rescue` (mika#2199), et l'inversion est raisonnée : là-bas un signal
//! illisible devait exclure la PR parce qu'un rejeu coûtait toute la file ; ici
//! un signal illisible doit conserver parce qu'un retrait fautif ne se répare
//! pas. **Cet arbitrage est local ; il ne se transporte pas.**
//!
//! # Trois leviers, trois portées, aucun redondant
//!
//! | levier | effet | quand |
//! |---|---|---|
//! | `MIKA_WORKTREE_REAP=0` | **annule la row récurrente** | au démarrage, désactivation durable |
//! | `MIKA_WORKTREE_REAP_DISPOSITION=observe` | le scan mesure et journalise, **ne supprime rien** | validation d'une nouvelle machine |
//! | le fichier sentinelle de [`crate::auto_pull_stop::WORKTREE_REAP_SCAN`] | court-circuite le tick | **pendant un incident, à chaud** |
//!
//! Le troisième est lu par [`crate::auto_pull_stop`], déjà paramétré par nom de
//! scan (mika#2329) : **une opération destructive est précisément le besoin
//! mesuré** que ce ticket-là avait nommé sans le servir. Le chemin littéral
//! n'est écrit ni ici ni ailleurs sous `src/` — ce module-là en est le lecteur
//! unique, et sa garde structurelle refuse toute seconde occurrence. Le geste
//! opérateur est documenté dans `CLAUDE.md`.
//!
//! # Livré armé, et l'argument est mesuré
//!
//! Le réflexe serait de livrer désarmé « par prudence ». **Refusé**, sur le
//! précédent mika#2272 : mika#2249 avait livré derrière une condition d'armement
//! — trois rows relues sans faux positif — qui s'est révélée **insatisfaisable,
//! pas seulement non satisfaite**, la population scannée étant vide par
//! construction. *« Zéro était l'absence de mesure, pas la présence de
//! caution. »* Ici le défaut est p1, la purge a une demi-vie de trois heures, et
//! livrer désarmé ne ferme rien. Ce qui paie la prudence est ailleurs et
//! concret : les termes fail-safe, le mode observation, le STOP à chaud, et un
//! **contrôle négatif** terme à terme en test. Une différence de fond avec
//! mika#2249 justifie l'écart : ce prédicat repose sur l'**état externe et
//! vérifiable d'une PR**, pas sur l'inférence d'un silence.
//!
//! # Pas de ledger anti-rejeu, et il faut dire pourquoi
//!
//! `qa_review_reconcile` a dû se doter d'un ledger relu (mika#2347) parce que
//! ses termes d'idempotence étaient des états GitHub *qui n'existent qu'après
//! aboutissement*. **Ce scan n'a pas ce défaut : son action est idempotente par
//! construction** — un worktree retiré n'apparaît plus dans `git worktree list`,
//! il sort de la population de lui-même. Les **refus**, eux, se répètent à
//! chaque tick, d'où la déduplication par `(worktree, motif)` sur 24 h (doctrine
//! mika#2131 : l'information durable est « ce worktree est tenu par ce motif »,
//! pas « il l'était encore à 14 h 32 » — la vivacité est le rôle de l'agrégat
//! par tick).

use crate::async_db::AsyncDatabase;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Le segment gardé, et les motifs de refus — format de fil
// ---------------------------------------------------------------------------

/// Segment que tout worktree de dispatch porte. Le reaper refuse de toucher un
/// chemin qui ne le contient pas : le checkout primaire et les worktrees créés
/// à la main hors de cette racine ne sont **jamais** dans la population.
pub const MANAGED_WORKTREE_SEGMENT: &str = "/.claude/worktrees/";

/// Au moins une PR ouverte pour cette branche (T4) — cas nominal, fréquent.
pub const REASON_PR_OPEN: &str = "pr_open";
/// Aucune PR résolvable pour cette branche (T3) — « groomé, pas encore
/// implémenté », fréquent. Délibérément distinct de [`REASON_PR_OPEN`] alors
/// qu'il mène au même verdict : le premier est un travail vivant, le second un
/// travail en attente, et les confondre effacerait deux populations.
pub const REASON_PR_UNKNOWN: &str = "pr_unknown";
/// PR close depuis moins que la grâce (T5) — transitoire, se résout seul.
pub const REASON_TOO_YOUNG: &str = "too_young";
/// Un processus vivant a son répertoire courant dedans (T6) — rare.
pub const REASON_LIVE_PROCESS: &str = "live_process";
/// Modifications non committées (T7) — **doit rester rare**, voir HALTE 3.
pub const REASON_DIRTY: &str = "dirty";
/// Commits absents d'`origin/<branche>` (T7) — **doit rester rare**, HALTE 3.
pub const REASON_UNPUSHED_COMMITS: &str = "unpushed_commits";
/// Pas de branche attachée (T2) — `detached HEAD`, rare.
pub const REASON_DETACHED_HEAD: &str = "detached_head";
/// Chemin hors de `.claude/worktrees/` (T1) — **doit rester vide**, HALTE 4.
pub const REASON_OUTSIDE_MANAGED_ROOT: &str = "outside_managed_root";
/// `closedAt` absent ou illisible sur la PR terminale (T5).
///
/// Les trois motifs `*_unreadable` **ne figurent pas** dans la table du plan, et
/// leur ajout est délibéré : R7 exige qu'un terme illisible conserve, et la
/// doctrine maison exige qu'un signal illisible soit **nommé** plutôt que replié
/// sur un motif lisible (`pilot_stall_signal_unavailable`, mika#2277 ;
/// `unknown_provider`, mika#2328). Écrire `too_young` pour un `closedAt`
/// illisible serait une ligne d'audit **fausse**, et un opérateur qui compte les
/// transitoires compterait un blocage permanent parmi eux.
pub const REASON_PR_CLOSED_AT_UNREADABLE: &str = "pr_closed_at_unreadable";
/// `git status` ou `git rev-list` n'a pas répondu (T7).
pub const REASON_WORK_STATE_UNREADABLE: &str = "work_state_unreadable";
/// `/proc` n'a pas pu être énuméré **en entier** (T6).
///
/// Granularité porteuse, et c'est un piège : le fail-safe porte sur
/// l'**énumération globale**, jamais sur un `readlink` individuel refusé — sur
/// une machine il y a toujours des processus d'autres utilisateurs, et conserver
/// dès le premier `EACCES` rendrait le scan définitivement inerte, c'est-à-dire
/// un désarmement déguisé en prudence.
pub const REASON_PROCESS_SCAN_UNREADABLE: &str = "process_scan_unreadable";

/// Tous les motifs, en un seul lieu.
///
/// Ils atterrissent dans `audit_events.after_value` et l'opérateur en fait des
/// `GROUP BY` : deux orthographes d'un même motif couperaient une population en
/// deux sans le dire. Épinglé par
/// [`tests::mika2420_les_motifs_sont_un_format_de_fil`].
pub const ALL_REFUSAL_REASONS: &[&str] = &[
    REASON_PR_OPEN,
    REASON_PR_UNKNOWN,
    REASON_TOO_YOUNG,
    REASON_LIVE_PROCESS,
    REASON_DIRTY,
    REASON_UNPUSHED_COMMITS,
    REASON_DETACHED_HEAD,
    REASON_OUTSIDE_MANAGED_ROOT,
    REASON_PR_CLOSED_AT_UNREADABLE,
    REASON_WORK_STATE_UNREADABLE,
    REASON_PROCESS_SCAN_UNREADABLE,
];

/// `audit_events.tool_name` écrit à chaque retrait **effectif** — et event
/// tracing de la même ligne : une seule constante sert les deux surfaces.
///
/// **SOLE WRITER** — ce module est le seul site qui écrit ce nom. C'est ce qui
/// fait de `SELECT … WHERE tool_name = 'worktree_reaped'` la liste exacte des
/// worktrees que la boucle a retirés, donc la réponse directe au garde-fou 3 du
/// ticket. **Réservé à `armed`** (mika#2469) : en `observe` la même ligne
/// s'écrit sous [`WOULD_DISPOSE_TOOL`], jamais sous ce nom — sinon la requête
/// ci-dessus compterait des observations parmi les retraits, en silence.
pub const REAPED_TOOL: &str = "worktree_reaped";

/// `audit_events.tool_name` (et event tracing) écrit en `observe` **à la place
/// de** [`REAPED_TOOL`] : la population qui *serait* retirée (mika#2469).
///
/// Même contrat SOLE WRITER que son aîné, tenu par la même garde à deux
/// needles. Avant mika#2469, `observe` écrivait `worktree_reaped` avec
/// `disposition=observe` dans `reasoning` : les lignes antérieures au
/// déploiement se distinguent par ce champ, pas par le nom.
pub const WOULD_DISPOSE_TOOL: &str = "worktree_reap_would_dispose";

/// Message INFO d'un retrait effectif (`armed`). Texte historique, inchangé.
pub const REAPED_MESSAGE: &str = "worktree_reap: worktree de PR terminale retiré";

/// Message INFO d'un candidat éligible en `observe` : nomme l'éligibilité
/// **et** nie le retrait dans la même phrase, pour qu'un lecteur qui ne voit
/// que le message (grep, tail, alerte) sache qu'il ne s'est rien passé.
pub const WOULD_DISPOSE_MESSAGE: &str =
    "worktree_reap: worktree de PR terminale éligible — observe, non retiré";

/// Ce que le tick écrit pour un candidat qui a franchi les sept termes, selon
/// ce qui lui est **réellement** arrivé (mika#2469, règle mika#2249 : une ligne
/// ne revendique jamais une disposition qui n'a pas eu lieu).
///
/// `event` sert à la fois d'event tracing et de `tool_name` d'audit — c'est
/// l'invariant historique, rendu explicite : les deux surfaces ne peuvent pas
/// diverger sans toucher [`outcome_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub event: &'static str,
    pub message: &'static str,
}

/// Source unique du triplet (event, tool_name, message) par disposition.
pub fn outcome_for(disposition: Disposition) -> Outcome {
    match disposition {
        Disposition::Armed => Outcome {
            event: REAPED_TOOL,
            message: REAPED_MESSAGE,
        },
        Disposition::Observe => Outcome {
            event: WOULD_DISPOSE_TOOL,
            message: WOULD_DISPOSE_MESSAGE,
        },
    }
}

/// `audit_events.tool_name` écrit à chaque refus, dédupliqué sur 24 h.
pub const SKIPPED_TOOL: &str = "worktree_reap_skipped";

/// Horizon de déduplication des refus (D9, doctrine mika#2131).
const REFUSAL_DEDUP_SECS: i64 = 86_400;

// ---------------------------------------------------------------------------
// Configuration — trois paliers
// ---------------------------------------------------------------------------

const DISPOSITION_ENV: &str = "MIKA_WORKTREE_REAP_DISPOSITION";
const REPO_DIRS_ENV: &str = "MIKA_WORKTREE_REAP_REPO_DIRS";
const GRACE_ENV: &str = "MIKA_WORKTREE_REAP_GRACE_SECS";
const MAX_PER_TICK_ENV: &str = "MIKA_WORKTREE_REAP_MAX_PER_TICK";

/// Trois fois l'enveloppe d'un tour (300 s) : couvre un callback de merge encore
/// en vol. À 8 %/h de remplissage, quinze minutes coûtent ~2 % de disque.
const GRACE_DEFAULT_SECS: i64 = 900;

/// À dix minutes de tick, trois par tick absorbent les onze worktrees mesurés en
/// moins d'une heure tout en étalant l'I/O de suppression — un `rm -rf` de 34 Go
/// est une tempête d'I/O, et c'est ce cap qui la borne. En régime stationnaire
/// (2 à 4 PR mergées/jour) il n'est jamais atteint.
const MAX_PER_TICK_DEFAULT: usize = 3;

/// Le checkout `mika` de la station de développement.
///
/// Même valeur que le `DEFAULT_REPO_DIR` du code jamais mergé de #1694. En
/// production conteneurisée ce chemin n'existe pas : le scan le **dit** plutôt
/// que de se taire (R10), parce qu'un scan silencieusement inactif se lit
/// exactement comme un scan qui n'a rien trouvé à faire (mika#2205).
const DEFAULT_REPO_DIR: &str = "/data/workspace/mika-platform/mika";

/// Le scan supprime-t-il, ou se contente-t-il de mesurer ?
///
/// Patron de mika#2249 : *la détection est inconditionnelle, seule la
/// disposition est gardée*. En observation, les lignes d'audit sont écrites avec
/// `disposition: "observe"`, ce qui donne à l'opérateur la population exacte qui
/// *serait* supprimée, avant de l'armer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Armed,
    Observe,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Observe => "observe",
        }
    }
}

/// Les bornes numériques de la décision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReapConfig {
    pub grace_secs: i64,
    /// Plafond d'écritures par tick. **Lu par l'appelant, jamais par
    /// [`select_worktrees_to_reap`]** — leçon mika#2347 : un cap posé en amont
    /// plafonne les *sauts* au lieu des *écritures*, et des candidats refusés
    /// consommeraient le tick à la place de candidats traitables.
    pub max_per_tick: usize,
    pub disposition: Disposition,
}

impl Default for ReapConfig {
    fn default() -> Self {
        Self {
            grace_secs: GRACE_DEFAULT_SECS,
            max_per_tick: MAX_PER_TICK_DEFAULT,
            disposition: Disposition::Armed,
        }
    }
}

/// Trois paliers : absent ou vide → défaut ; illisible, `0` ou négatif → défaut
/// avec un `warn!` nommant la valeur.
///
/// Le `0` **ne désarme pas** — c'est le rôle du kill-switch, et l'inverse ferait
/// d'une coquille un désarmement silencieux sur un scan destructif.
fn parse_positive_i64(raw: Option<&str>, default: i64, env_name: &str) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(n) if n > 0 => n,
            _ => {
                warn!(
                    value = %v,
                    default,
                    "worktree_reap: {env_name} illisible ou non positif, défaut appliqué"
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
                    "worktree_reap: {env_name} illisible ou non positif, défaut appliqué"
                );
                default
            }
        },
        _ => default,
    }
}

/// `armed` par défaut ; `observe` est l'autre valeur reconnue. Une valeur non
/// reconnue **reste armée** avec un WARN la nommant entre guillemets : un
/// désarmement par coquille ferait croire le scan actif alors qu'il ne
/// supprimerait plus rien, ce qui est la panne silencieuse que mika#2205 nomme.
pub fn parse_disposition(raw: Option<&str>) -> Disposition {
    match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("armed") => Disposition::Armed,
        Some("observe") => Disposition::Observe,
        Some(other) => {
            warn!(
                value = %format!("{other:?}"),
                "worktree_reap: valeur non reconnue pour {DISPOSITION_ENV} — le scan reste armé"
            );
            Disposition::Armed
        }
    }
}

fn config_from_env() -> ReapConfig {
    ReapConfig {
        grace_secs: parse_positive_i64(
            std::env::var(GRACE_ENV).ok().as_deref(),
            GRACE_DEFAULT_SECS,
            GRACE_ENV,
        ),
        max_per_tick: parse_positive_usize(
            std::env::var(MAX_PER_TICK_ENV).ok().as_deref(),
            MAX_PER_TICK_DEFAULT,
            MAX_PER_TICK_ENV,
        ),
        disposition: parse_disposition(std::env::var(DISPOSITION_ENV).ok().as_deref()),
    }
}

/// Liste de checkouts séparés par `:`.
///
/// Le `owner/repo` est **dérivé** de `git remote get-url origin` plutôt que
/// déclaré dans une seconde variable : deux listes à tenir synchronisées est une
/// divergence programmée.
pub fn parse_repo_dirs(raw: Option<&str>) -> Vec<PathBuf> {
    let parsed: Vec<PathBuf> = raw
        .unwrap_or(DEFAULT_REPO_DIR)
        .split(':')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    if parsed.is_empty() {
        vec![PathBuf::from(DEFAULT_REPO_DIR)]
    } else {
        parsed
    }
}

// ---------------------------------------------------------------------------
// Les instantanés que la décision consomme
// ---------------------------------------------------------------------------

/// Une entrée du registre git, telle que `git worktree list --porcelain` la rend.
///
/// **Le registre est la vérité terrain, jamais une dérivation de chemin.**
/// `CLAUDE.md` pose la règle (*« the worktree path is declared, never
/// derived »*) et re-dériver est la duplication que mika-platform#58 a fermée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    /// `None` pour un worktree en `detached HEAD` : il n'a pas de ligne
    /// `branch`, donc il sort de la population (T2).
    pub branch: Option<String>,
}

/// Une PR telle que `gh pr list --json` la rend.
///
/// **Aucun `#[serde(default)]` sur `state` / `headRefName`, et c'est
/// porteur** : les deux champs sont explicitement demandés, donc toujours
/// rendus. Les rendre `default`-ables ferait lire une absence comme un état
/// terminal — c'est-à-dire ferait *entrer* un worktree dans la population sur
/// une information manquante, l'exact inverse du fail-safe.
#[derive(Debug, Clone, Deserialize)]
pub struct PrSnapshot {
    pub number: u64,
    /// `OPEN` | `MERGED` | `CLOSED`.
    pub state: String,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    /// `None` sur une PR ouverte — et, sur une PR terminale, une absence qui
    /// **conserve** (T5 ne peut pas s'évaluer).
    #[serde(rename = "closedAt")]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub url: String,
}

impl PrSnapshot {
    fn is_open(&self) -> bool {
        self.state.eq_ignore_ascii_case("OPEN")
    }
}

/// Ce que l'énumération de `/proc` a pu établir (T6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveCwds {
    /// `/proc` a été énuméré ; voici les répertoires courants lisibles.
    Enumerated(Vec<PathBuf>),
    /// `/proc` n'a pas pu être énuméré **du tout** — T6 est inévaluable, donc
    /// tout est conservé.
    Unavailable,
}

/// L'état du travail dans un worktree candidat (T7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkState {
    /// Rien de non livré.
    Clean,
    /// Modifications non committées.
    Dirty,
    /// Commits absents d'`origin/<branche>`.
    UnpushedCommits,
    /// `git` n'a pas répondu — conserver.
    Unreadable,
}

/// Un worktree retenu pour le retrait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapCandidate {
    pub path: String,
    pub branch: String,
    pub pr_number: u64,
    pub pr_state: String,
    pub pr_url: String,
}

/// Un worktree conservé, et le motif nommé qui l'a conservé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapRefusal {
    pub path: String,
    pub branch: Option<String>,
    pub reason: &'static str,
}

/// La sortie de la décision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReapSelection {
    pub candidates: Vec<ReapCandidate>,
    pub refusals: Vec<ReapRefusal>,
}

// ---------------------------------------------------------------------------
// La décision — fonctions pures, testables sans réseau ni système de fichiers
// ---------------------------------------------------------------------------

/// Un chemin est-il un worktree géré ?
///
/// Trois conditions, et la deuxième n'est pas décorative : un chemin relatif ou
/// portant un composant `..` peut contenir le segment gardé tout en désignant
/// n'importe quoi (`/x/.claude/worktrees/../../../etc`). Le refus est
/// syntaxique ici ; il est **re-vérifié après canonicalisation** au moment de la
/// disposition, où un lien symbolique est également éliminé.
pub fn is_managed_worktree_path(path: &str) -> bool {
    let p = Path::new(path);
    if !p.is_absolute() {
        return false;
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    path.contains(MANAGED_WORKTREE_SEGMENT)
}

/// Le même refus, après résolution des liens symboliques.
///
/// [`is_managed_worktree_path`] est syntaxique : elle refuse `..` et les chemins
/// relatifs, c'est-à-dire ce qu'on peut trancher sans toucher au disque. Un
/// **lien symbolique** n'est visible qu'ici : un chemin du registre situé sous
/// `.claude/worktrees/` mais pointant ailleurs passerait la garde syntaxique, et
/// `git worktree remove --force` supprimerait la cible.
///
/// Appelée juste avant la disposition, sur le seul candidat retenu — c'est le
/// dernier point où le refus est encore gratuit. `canonicalize` qui échoue
/// (chemin disparu entre la sélection et la disposition, permissions) rend
/// `false` : pas de preuve, pas de suppression.
pub fn canonical_path_is_managed(path: &str) -> bool {
    std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .is_some_and(|p| is_managed_worktree_path(&p))
}

/// Les six premiers termes (T1 à T6), du moins cher au plus cher.
///
/// | # | terme | source de vérité | illisible ⇒ |
/// |---|---|---|---|
/// | T1 | le chemin contient `/.claude/worktrees/` | le chemin lui-même | conserver |
/// | T2 | le worktree a une branche attachée | `git worktree list --porcelain` | conserver |
/// | T3 | au moins une PR connue pour cette branche | `gh pr list --state all` | conserver |
/// | T4 | **aucune** PR ouverte pour cette branche | idem | conserver |
/// | T5 | la PR la plus récemment close l'est depuis plus que la grâce | `closedAt` | conserver |
/// | T6 | aucun processus vivant n'a son cwd sous le worktree | `/proc/*/cwd` | conserver |
///
/// **T4 est formulé en négatif à dessein.** Deux PR peuvent partager une même
/// `headRefName` (une fermée, une rouverte). « Il existe une PR mergée » serait
/// vrai dans ce cas et conduirait à supprimer un worktree dont une PR est
/// ouverte. « Aucune PR n'est ouverte » est le prédicat correct.
///
/// **T6 remplace le `pgrep` du ticket, et le remplace par mieux.** `pgrep`
/// matche un nom de commande, pas une localisation : un `cargo` appartenant à un
/// *autre* worktree le satisferait. `/proc/<pid>/cwd` répond à la question
/// réellement posée.
///
/// Invariant : **un worktree dont un seul terme est illisible est conservé ; il
/// n'existe aucune exception.**
pub fn screen_worktrees(
    entries: &[WorktreeEntry],
    prs_by_branch: &HashMap<String, Vec<PrSnapshot>>,
    live: &LiveCwds,
    now: DateTime<Utc>,
    cfg: &ReapConfig,
) -> ReapSelection {
    let mut out = ReapSelection::default();

    for entry in entries {
        let refuse = |out: &mut ReapSelection, reason: &'static str| {
            out.refusals.push(ReapRefusal {
                path: entry.path.clone(),
                branch: entry.branch.clone(),
                reason,
            });
        };

        // T1 — chemin géré.
        if !is_managed_worktree_path(&entry.path) {
            // Le checkout primaire est dans le registre et n'est pas un
            // worktree géré : il est hors population par construction, et le
            // dire à chaque tick noierait le signal que ce motif existe pour
            // lever (HALTE 4). Seul un chemin qui *ressemble* à un worktree géré
            // sans en être un est compté.
            if entry.path.contains(".claude/worktrees") {
                refuse(&mut out, REASON_OUTSIDE_MANAGED_ROOT);
            }
            continue;
        }

        // T2 — branche attachée.
        let Some(branch) = entry.branch.as_deref() else {
            refuse(&mut out, REASON_DETACHED_HEAD);
            continue;
        };

        // T3 — au moins une PR connue.
        let Some(prs) = prs_by_branch.get(branch).filter(|p| !p.is_empty()) else {
            refuse(&mut out, REASON_PR_UNKNOWN);
            continue;
        };

        // T4 — aucune PR ouverte.
        if prs.iter().any(PrSnapshot::is_open) {
            refuse(&mut out, REASON_PR_OPEN);
            continue;
        }

        // T5 — la plus récemment close l'est depuis plus que la grâce. Un
        // `closedAt` absent ou illisible sur **une seule** PR terminale rend la
        // borne inévaluable : conserver.
        let mut newest_closed: Option<DateTime<Utc>> = None;
        let mut closed_at_unreadable = false;
        for pr in prs.iter() {
            match pr.closed_at.as_deref().map(crate::timestamp::parse) {
                Some(Ok(ts)) => {
                    newest_closed = Some(match newest_closed {
                        Some(cur) if cur >= ts => cur,
                        _ => ts,
                    });
                }
                _ => closed_at_unreadable = true,
            }
        }
        if closed_at_unreadable {
            refuse(&mut out, REASON_PR_CLOSED_AT_UNREADABLE);
            continue;
        }
        let Some(newest_closed) = newest_closed else {
            refuse(&mut out, REASON_PR_CLOSED_AT_UNREADABLE);
            continue;
        };
        // Une date dans le futur (dérive d'horloge) donne un âge ramené à 0,
        // donc plus jeune que la grâce : conserver, qui est la direction sûre.
        let closed_for = (now - newest_closed).num_seconds().max(0);
        if closed_for < cfg.grace_secs {
            refuse(&mut out, REASON_TOO_YOUNG);
            continue;
        }

        // T6 — aucun processus vivant dedans.
        match live {
            LiveCwds::Unavailable => {
                refuse(&mut out, REASON_PROCESS_SCAN_UNREADABLE);
                continue;
            }
            LiveCwds::Enumerated(cwds) => {
                let root = Path::new(&entry.path);
                if cwds.iter().any(|cwd| cwd == root || cwd.starts_with(root)) {
                    refuse(&mut out, REASON_LIVE_PROCESS);
                    continue;
                }
            }
        }

        // La PR retenue pour la ligne d'audit : la plus récemment close.
        let pr = prs
            .iter()
            .max_by_key(|p| p.closed_at.clone().unwrap_or_default())
            .expect("prs non vide");

        out.candidates.push(ReapCandidate {
            path: entry.path.clone(),
            branch: branch.to_string(),
            pr_number: pr.number,
            pr_state: pr.state.clone(),
            pr_url: pr.url.clone(),
        });
    }

    out
}

/// T7 — rien de non livré.
///
/// Séparé de [`screen_worktrees`] parce qu'il coûte deux sous-processus `git`
/// par candidat : l'appelant ne le paie que pour les worktrees ayant survécu à
/// T1-T6. Une entrée absente de `work_states` vaut [`WorkState::Unreadable`],
/// donc conserve.
pub fn apply_work_states(
    candidates: Vec<ReapCandidate>,
    work_states: &HashMap<String, WorkState>,
) -> ReapSelection {
    let mut out = ReapSelection::default();
    for candidate in candidates {
        let state = work_states
            .get(&candidate.path)
            .copied()
            .unwrap_or(WorkState::Unreadable);
        let reason = match state {
            WorkState::Clean => {
                out.candidates.push(candidate);
                continue;
            }
            WorkState::Dirty => REASON_DIRTY,
            WorkState::UnpushedCommits => REASON_UNPUSHED_COMMITS,
            WorkState::Unreadable => REASON_WORK_STATE_UNREADABLE,
        };
        out.refusals.push(ReapRefusal {
            path: candidate.path,
            branch: Some(candidate.branch),
            reason,
        });
    }
    out
}

/// La conjonction complète des sept termes — la forme que les tests consomment.
///
/// La production passe par [`screen_worktrees`] puis [`apply_work_states`] pour
/// n'appeler `git` que sur les survivants ; les deux chemins rendent la même
/// décision, l'écran étant déterministe.
pub fn select_worktrees_to_reap(
    entries: &[WorktreeEntry],
    prs_by_branch: &HashMap<String, Vec<PrSnapshot>>,
    live: &LiveCwds,
    work_states: &HashMap<String, WorkState>,
    now: DateTime<Utc>,
    cfg: &ReapConfig,
) -> ReapSelection {
    let screened = screen_worktrees(entries, prs_by_branch, live, now, cfg);
    let mut final_pass = apply_work_states(screened.candidates, work_states);
    let mut refusals = screened.refusals;
    refusals.append(&mut final_pass.refusals);
    ReapSelection {
        candidates: final_pass.candidates,
        refusals,
    }
}

// ---------------------------------------------------------------------------
// Le walker de taille — distinct de celui de `worktree_activity`, et c'est
// essentiel (D5)
// ---------------------------------------------------------------------------

/// `task_engine::worktree_activity` exclut `target/` de sa marche, et cette
/// exclusion est **porteuse pour son propre prédicat** : un `cargo build` en
/// arrière-plan ne doit pas faire paraître vivant un pilote coincé.
///
/// **Ici l'exigence est exactement inverse : `target/` *est* la mesure** — c'est
/// le consommateur de 25 à 44 Go que le ticket veut voir. Réutiliser ce walker
/// rapporterait quelques mégaoctets là où il y en a trente-quatre gigaoctets,
/// c'est-à-dire un chiffre faux avec l'autorité d'une mesure.
///
/// **Alternative refusée :** lire l'espace libre du point de montage avant et
/// après. Moins chère, mais elle attribuerait à ce retrait l'activité
/// concurrente de deux pilotes qui buildent — un chiffre bruité présenté comme
/// exact.
const SIZE_MAX_ENTRIES: usize = 400_000;
const SIZE_MAX_DEPTH: usize = 32;
const SIZE_TIME_BUDGET: Duration = Duration::from_millis(2_000);

/// Ce qu'une marche de taille rapporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SizeMeasurement {
    /// **`null` n'est jamais `0`** (doctrine mika#2331) : un zéro serait un
    /// mensonge lisible sur un répertoire qui n'est jamais vide.
    pub bytes: Option<u64>,
    /// Une marche tronquée rend un **minorant explicitement étiqueté**, jamais
    /// une mesure.
    pub truncated: bool,
}

/// Somme des tailles de fichiers sous `root`, bornée.
///
/// **Best-effort, et son échec n'empêche jamais un retrait** : il rend `None`.
/// Les liens symboliques ne sont pas suivis — un lien vers un arbre voisin
/// importerait sa taille dans la mesure.
pub fn measure_tree_size(root: &Path) -> SizeMeasurement {
    if !root.is_dir() {
        return SizeMeasurement::default();
    }
    let started = Instant::now();
    let mut bytes: u64 = 0;
    let mut entries_scanned = 0usize;
    let mut truncated = false;
    let mut saw_any = false;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if entries_scanned >= SIZE_MAX_ENTRIES || started.elapsed() > SIZE_TIME_BUDGET {
            truncated = true;
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            // Un répertoire illisible coûte un sous-arbre de mesure, pas la
            // mesure : la direction de l'erreur (sous-estimer) est la sûre.
            truncated = true;
            continue;
        };
        for entry in read.flatten() {
            entries_scanned += 1;
            if entries_scanned >= SIZE_MAX_ENTRIES {
                truncated = true;
                break;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                truncated = true;
                continue;
            };
            if meta.is_dir() {
                if depth + 1 > SIZE_MAX_DEPTH {
                    truncated = true;
                    continue;
                }
                stack.push((entry.path(), depth + 1));
            } else if meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
                saw_any = true;
            }
        }
    }

    SizeMeasurement {
        bytes: saw_any.then_some(bytes),
        truncated,
    }
}

// ---------------------------------------------------------------------------
// La collecte (U2)
// ---------------------------------------------------------------------------

/// Budget par sous-processus `gh`, aligné sur `wip_rescue` et
/// `qa_review_reconcile`.
const GH_TIMEOUT: Duration = Duration::from_secs(30);

/// Taille de page de l'unique `gh pr list` par dépôt et par tick.
const LIST_LIMIT: usize = 300;

/// `git` dans `cwd`. Rend `Some(stdout)` sur exit 0, `None` sinon.
///
/// `tokio::process::Command` et non `std::process` : ce code tourne dans le tick
/// du moteur, qui tient aussi son mutex — un sous-processus bloquant y tiendrait
/// la boucle entière.
async fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse `git worktree list --porcelain`.
///
/// Les entrées marquées `prunable` sont **écartées** : elles relèvent de
/// `git worktree prune`, pas d'un retrait — leur répertoire n'existe déjà plus.
pub fn parse_worktree_registry(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut prunable = false;

    let mut flush =
        |path: &mut Option<String>, branch: &mut Option<String>, prunable: &mut bool| {
            if let Some(p) = path.take()
                && !*prunable
            {
                out.push(WorktreeEntry {
                    path: p,
                    branch: branch.take(),
                });
            }
            *branch = None;
            *prunable = false;
        };

    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            flush(&mut path, &mut branch, &mut prunable);
            path = Some(p.trim().to_string());
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = b.trim().strip_prefix("refs/heads/").map(str::to_string);
        } else if line.trim() == "prunable" || line.starts_with("prunable ") {
            prunable = true;
        }
    }
    flush(&mut path, &mut branch, &mut prunable);
    out
}

/// Extrait `owner/repo` d'une URL de remote (`git@host:owner/repo.git` ou
/// `https://host/owner/repo.git`).
///
/// Dérivé plutôt que déclaré : une seconde liste à tenir synchronisée avec
/// `MIKA_WORKTREE_REAP_REPO_DIRS` est une divergence programmée.
pub fn parse_owner_repo(remote_url: &str) -> Option<String> {
    let url = remote_url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let tail = match url.rsplit_once(':') {
        // `git@github.com:owner/repo` — mais pas `https://host:443/owner/repo`,
        // que la branche `//` ci-dessous attrape d'abord.
        Some((head, rest)) if !head.contains("//") && !rest.starts_with('/') => rest,
        _ => {
            let after_host = url.split_once("//").map(|(_, r)| r).unwrap_or(url);
            after_host.split_once('/').map(|(_, r)| r)?
        }
    };
    let mut parts = tail.rsplitn(3, '/');
    let repo = parts.next()?;
    let owner = parts.next()?;
    if repo.is_empty() || owner.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Énumère les répertoires courants des processus vivants (T6).
///
/// Fail-safe **global seulement** : une énumération de `/proc` impossible rend
/// [`LiveCwds::Unavailable`] et conserve tout ; un `readlink` individuel refusé
/// est simplement sauté (D4).
pub fn collect_live_cwds() -> LiveCwds {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return LiveCwds::Unavailable;
    };
    let mut cwds = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // EACCES sur le process d'un autre utilisateur : sauté, jamais fatal.
        if let Ok(cwd) = std::fs::read_link(format!("/proc/{name}/cwd")) {
            cwds.push(cwd);
        }
    }
    LiveCwds::Enumerated(cwds)
}

/// T7 — deux sous-processus `git`, sur un seul candidat.
///
/// # Le cas « `origin/<branche>` n'existe plus », et pourquoi il vaut `Clean`
///
/// On n'atteint ce terme que pour un worktree dont **une PR terminale existe**,
/// donc dont la branche a été poussée. Un `refs/remotes/origin/<branche>` absent
/// signifie alors que le dépôt distant l'a supprimée après la fermeture de la
/// PR — c'est-à-dire que le travail est livré. Le lire comme « illisible »
/// conserverait la quasi-totalité de la population visée, c'est-à-dire
/// rendrait le scan inerte exactement là où il doit servir (classe mika#2205).
///
/// Risque résiduel nommé : une PR **fermée sans merge** dont la branche distante
/// a été supprimée verra ses commits partir avec le worktree. Ils restent
/// accessibles dans l'historique de la PR côté GitHub, et la moitié « dirty »
/// ci-dessus protège toujours le travail **non committé**.
async fn collect_work_state(worktree: &Path, branch: &str) -> WorkState {
    let Some(status) = run_git(worktree, &["status", "--porcelain"]).await else {
        return WorkState::Unreadable;
    };
    if !status.trim().is_empty() {
        return WorkState::Dirty;
    }

    let remote_ref = format!("refs/remotes/origin/{branch}");
    if run_git(worktree, &["rev-parse", "--verify", "--quiet", &remote_ref])
        .await
        .is_none()
    {
        // Ref absente : branche distante supprimée après fermeture de la PR.
        return WorkState::Clean;
    }

    let range = format!("origin/{branch}..HEAD");
    let Some(count) = run_git(worktree, &["rev-list", "--count", &range]).await else {
        return WorkState::Unreadable;
    };
    match count.trim().parse::<u64>() {
        Ok(0) => WorkState::Clean,
        Ok(_) => WorkState::UnpushedCommits,
        Err(_) => WorkState::Unreadable,
    }
}

/// `gh` borné par un timeout, même forme que `qa_review_reconcile::gh`.
async fn gh(args: &[&str], token: &str) -> Result<String, String> {
    match tokio::time::timeout(
        GH_TIMEOUT,
        crate::tools::pr_merge_with_gate::run_gh_subprocess(args, token),
    )
    .await
    {
        Ok(res) => res,
        Err(_) => Err(format!("gh timed out after {}s", GH_TIMEOUT.as_secs())),
    }
}

/// Un seul `gh pr list` par dépôt et par tick — le coût API est constant.
async fn list_prs(repo: &str, token: &str) -> Result<Vec<PrSnapshot>, String> {
    let limit = LIST_LIMIT.to_string();
    let out = gh(
        &[
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "all",
            "--json",
            "number,state,headRefName,closedAt,url",
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
    serde_json::from_str(trimmed).map_err(|e| format!("parse gh pr list ({repo}): {e}"))
}

/// Indexe les PR par `headRefName`.
pub fn index_prs_by_branch(prs: Vec<PrSnapshot>) -> HashMap<String, Vec<PrSnapshot>> {
    let mut map: HashMap<String, Vec<PrSnapshot>> = HashMap::new();
    for pr in prs {
        map.entry(pr.head_ref_name.clone()).or_default().push(pr);
    }
    map
}

// ---------------------------------------------------------------------------
// La disposition (U3)
// ---------------------------------------------------------------------------

/// Ce qu'un retrait a réellement fait.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Removal {
    removed: bool,
    parent_removed: bool,
    branch_deleted: bool,
}

/// Retire un worktree, son répertoire parent `<slug>/` **s'il devient vide**, et
/// sa branche locale en best-effort.
///
/// # Le point de rupture le plus probable de cette unité
///
/// Le retrait du parent. `dispatch-lib.sh` le gère déjà de son côté (via
/// `derive-worktree-path --no-repo`), et un worktree multi-dépôt peut partager
/// un parent avec un autre dépôt encore vivant. **Le parent n'est retiré que
/// s'il est vide**, jamais récursivement.
///
/// La suppression de la branche locale est **best-effort et journalisée** : le
/// worktree est parti de toute façon, et son échec ne doit pas faire échouer le
/// retrait.
async fn remove_worktree(repo_dir: &Path, candidate: &ReapCandidate) -> Removal {
    let removed = run_git(
        repo_dir,
        &["worktree", "remove", "--force", &candidate.path],
    )
    .await
    .is_some();

    let mut parent_removed = false;
    if removed {
        // Parent vide seulement, et seulement sous la racine gérée.
        if let Some(parent) = Path::new(&candidate.path).parent()
            && parent
                .to_str()
                .is_some_and(|p| p.contains(MANAGED_WORKTREE_SEGMENT))
            && std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none())
        {
            parent_removed = std::fs::remove_dir(parent).is_ok();
        }
        let _ = run_git(repo_dir, &["worktree", "prune"]).await;
    }

    let branch_deleted = run_git(repo_dir, &["branch", "-D", &candidate.branch])
        .await
        .is_some();

    Removal {
        removed,
        parent_removed,
        branch_deleted,
    }
}

// ---------------------------------------------------------------------------
// L'orchestration
// ---------------------------------------------------------------------------

/// Retire les worktrees dont la PR est terminale.
///
/// Rend `Some(n)` quand `n > 0` worktrees ont été traités (retirés, ou mesurés
/// en mode observation), `None` sinon — **zéro action, zéro ligne** (doctrine
/// mika#2131 : un scan qui journalise tout le monde ne distingue plus personne).
///
/// Fail-open de bout en bout : aucun échec de ce scan ne fait échouer le tick du
/// moteur.
pub async fn reap_terminal_worktrees(
    db: &AsyncDatabase,
    github_token: &str,
    trace_id: &str,
    session_id: &str,
) -> Option<usize> {
    let cfg = config_from_env();
    let repo_dirs = parse_repo_dirs(std::env::var(REPO_DIRS_ENV).ok().as_deref());
    let now = Utc::now();

    // Une seule énumération de `/proc` par tick : la population de processus ne
    // change pas d'un dépôt à l'autre, et l'énumérer N fois multiplierait le
    // coût sans rien ajouter.
    let live = collect_live_cwds();

    let mut budget = cfg.max_per_tick;
    let mut disposed = 0usize;
    let mut failed = 0usize;
    let mut refused = 0usize;
    let mut bytes_total: u64 = 0;

    for repo_dir in &repo_dirs {
        if budget == 0 {
            break;
        }

        // R10 — l'inaction pour cause d'environnement est **dite**, jamais
        // silencieuse. En production conteneurisée les worktrees ne vivent pas
        // sur le système de fichiers de l'agent : le scan n'a rien à faire, et
        // sans cette ligne son silence se lirait exactement comme celui d'un
        // scan qui tourne et ne trouve rien (classe mika#2205).
        if !repo_dir.join(".git").exists() {
            warn!(
                event = "worktree_reap_no_checkout",
                repo_dir = %repo_dir.display(),
                trace_id,
                "worktree_reap: aucun checkout git à ce chemin — le scan ne fait \
                 rien pour ce dépôt (attendu en production conteneurisée ; \
                 corriger {REPO_DIRS_ENV} sur une station de développement)"
            );
            continue;
        }

        let Some(porcelain) = run_git(repo_dir, &["worktree", "list", "--porcelain"]).await else {
            warn!(
                event = "worktree_reap_failed",
                stage = "registry",
                repo_dir = %repo_dir.display(),
                trace_id,
                "worktree_reap: `git worktree list` a échoué, dépôt sauté ce tick"
            );
            failed += 1;
            continue;
        };
        let entries = parse_worktree_registry(&porcelain);

        let Some(remote_url) = run_git(repo_dir, &["remote", "get-url", "origin"]).await else {
            warn!(
                event = "worktree_reap_failed",
                stage = "remote",
                repo_dir = %repo_dir.display(),
                trace_id,
                "worktree_reap: `git remote get-url origin` a échoué, dépôt sauté ce tick"
            );
            failed += 1;
            continue;
        };
        let Some(repo) = parse_owner_repo(&remote_url) else {
            warn!(
                event = "worktree_reap_failed",
                stage = "remote_parse",
                repo_dir = %repo_dir.display(),
                trace_id,
                "worktree_reap: URL de remote illisible, dépôt sauté ce tick"
            );
            failed += 1;
            continue;
        };

        let prs = match list_prs(&repo, github_token).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    event = "worktree_reap_failed",
                    stage = "pr_list",
                    repo = %repo,
                    error = %e,
                    trace_id,
                    "worktree_reap: lecture des PR impossible, dépôt sauté ce tick"
                );
                failed += 1;
                continue;
            }
        };
        let prs_by_branch = index_prs_by_branch(prs);

        // T1-T6 d'abord : T7 coûte deux `git` par candidat, et ne se paie que
        // sur les survivants.
        let screened = screen_worktrees(&entries, &prs_by_branch, &live, now, &cfg);
        for refusal in &screened.refusals {
            refused += 1;
            record_refusal(db, session_id, refusal, now, trace_id).await;
        }

        // T7, puis **le cap, appliqué après le filtre** (leçon mika#2347).
        let mut work_states = HashMap::new();
        for candidate in &screened.candidates {
            let state = collect_work_state(Path::new(&candidate.path), &candidate.branch).await;
            work_states.insert(candidate.path.clone(), state);
        }
        let selection = apply_work_states(screened.candidates, &work_states);
        for refusal in &selection.refusals {
            refused += 1;
            record_refusal(db, session_id, refusal, now, trace_id).await;
        }

        for candidate in selection.candidates {
            if budget == 0 {
                break;
            }

            // Re-vérification après canonicalisation : un lien symbolique ou un
            // chemin fabriqué ne doit atteindre la disposition sous aucune
            // forme. La garde syntaxique de T1 a déjà refusé `..` et les chemins
            // relatifs ; celle-ci refuse ce que seul le système de fichiers
            // peut révéler.
            if !canonical_path_is_managed(&candidate.path) {
                refused += 1;
                record_refusal(
                    db,
                    session_id,
                    &ReapRefusal {
                        path: candidate.path.clone(),
                        branch: Some(candidate.branch.clone()),
                        reason: REASON_OUTSIDE_MANAGED_ROOT,
                    },
                    now,
                    trace_id,
                )
                .await;
                continue;
            }

            budget -= 1;
            let size = measure_tree_size(Path::new(&candidate.path));

            let removal = match cfg.disposition {
                Disposition::Observe => Removal {
                    removed: false,
                    parent_removed: false,
                    branch_deleted: false,
                },
                Disposition::Armed => remove_worktree(repo_dir, &candidate).await,
            };

            if cfg.disposition == Disposition::Armed && !removal.removed {
                failed += 1;
                warn!(
                    event = "worktree_reap_failed",
                    stage = "remove",
                    worktree_path = %candidate.path,
                    branch = %candidate.branch,
                    pr_number = candidate.pr_number,
                    trace_id,
                    "worktree_reap: `git worktree remove --force` a échoué"
                );
                continue;
            }

            disposed += 1;
            if let Some(b) = size.bytes {
                bytes_total = bytes_total.saturating_add(b);
            }

            info!(
                event = REAPED_TOOL,
                worktree_path = %candidate.path,
                branch = %candidate.branch,
                pr_number = candidate.pr_number,
                pr_state = %candidate.pr_state,
                pr_url = %candidate.pr_url,
                bytes_reclaimed = size.bytes,
                bytes_reclaimed_truncated = size.truncated,
                parent_removed = removal.parent_removed,
                branch_deleted = removal.branch_deleted,
                disposition = cfg.disposition.as_str(),
                trace_id,
                "worktree_reap: worktree de PR terminale retiré"
            );
            record_reaped(db, session_id, &candidate, &size, cfg.disposition, trace_id).await;
        }
    }

    if disposed == 0 && failed == 0 {
        debug!(trace_id, "worktree_reap: rien à retirer ce tick");
        return None;
    }

    info!(
        event = "worktree_reap_tick",
        disposed,
        failed,
        refused,
        bytes_reclaimed = bytes_total,
        disposition = cfg.disposition.as_str(),
        repos = repo_dirs.len(),
        trace_id,
        "worktree_reap: tick agissant"
    );

    (disposed > 0).then_some(disposed)
}

/// Clé d'audit d'un retrait : `worktree:<chemin>`.
pub fn reaped_audit_key(path: &str) -> String {
    format!("worktree:{path}")
}

/// Clé d'audit d'un refus : `worktree:<chemin>@<motif>`.
///
/// Le motif est **dans la clé** pour que la déduplication soit par
/// `(worktree, motif)` : un worktree qui **change** de motif réécrit, parce que
/// c'est un changement d'état (D9).
pub fn refusal_audit_key(path: &str, reason: &str) -> String {
    format!("worktree:{path}@{reason}")
}

async fn record_reaped(
    db: &AsyncDatabase,
    session_id: &str,
    candidate: &ReapCandidate,
    size: &SizeMeasurement,
    disposition: Disposition,
    trace_id: &str,
) {
    let reasoning = format!(
        "pr={} state={} url={} branch={} bytes_reclaimed={} truncated={} disposition={}",
        candidate.pr_number,
        candidate.pr_state,
        candidate.pr_url,
        candidate.branch,
        size.bytes
            .map(|b| b.to_string())
            .unwrap_or_else(|| "null".to_string()),
        size.truncated,
        disposition.as_str(),
    );
    if let Err(e) = db
        .log_audit_event(
            session_id,
            REAPED_TOOL,
            &reaped_audit_key(&candidate.path),
            None,
            size.bytes.map(|b| b.to_string()).as_deref(),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            worktree_path = %candidate.path,
            error = %e,
            trace_id,
            "worktree_reap: audit write failed (reaped)"
        );
    }
}

/// Écrit le refus **une fois par (worktree, motif) et par 24 h**.
///
/// Un worktree dirty de PR mergée produirait sinon une ligne toutes les dix
/// minutes. L'horizon est porteur, pas cosmétique : sans lui, un worktree qui
/// quitte un motif puis y revient n'écrirait rien la seconde fois, et la ligne
/// la plus récente pourrait décrire un motif qui a cessé de s'appliquer des
/// jours plus tôt.
///
/// La marque n'est posée qu'**après** une écriture réussie ; une lecture
/// impossible saute l'écriture plutôt que de la dupliquer.
async fn record_refusal(
    db: &AsyncDatabase,
    session_id: &str,
    refusal: &ReapRefusal,
    now: DateTime<Utc>,
    trace_id: &str,
) {
    let key = refusal_audit_key(&refusal.path, refusal.reason);
    let since = crate::timestamp::format(
        &now.checked_sub_signed(chrono::TimeDelta::seconds(REFUSAL_DEDUP_SECS))
            .unwrap_or(DateTime::<Utc>::MIN_UTC),
    );
    match db
        .count_recent_audit_events_for_target(SKIPPED_TOOL, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(e) => {
            debug!(
                worktree_path = %refusal.path,
                error = %e,
                trace_id,
                "worktree_reap: relecture du marqueur de refus impossible, écriture sautée"
            );
            return;
        }
    }

    let reasoning = format!(
        "branch={} motif={}",
        refusal.branch.as_deref().unwrap_or("(detached)"),
        refusal.reason
    );
    if let Err(e) = db
        .log_audit_event(
            session_id,
            SKIPPED_TOOL,
            &key,
            None,
            Some(refusal.reason),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            worktree_path = %refusal.path,
            error = %e,
            trace_id,
            "worktree_reap: audit write failed (skipped)"
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
        crate::timestamp::parse("2026-09-20T12:00:00Z").unwrap()
    }

    fn closed_secs_ago(secs: i64) -> String {
        crate::timestamp::format(&(now() - chrono::Duration::seconds(secs)))
    }

    const WT: &str = "/data/workspace/mika-platform/.claude/worktrees/fix-2420-x/mika";

    fn entry(path: &str, branch: Option<&str>) -> WorktreeEntry {
        WorktreeEntry {
            path: path.to_string(),
            branch: branch.map(str::to_string),
        }
    }

    fn merged_pr(number: u64, branch: &str, closed_secs: i64) -> PrSnapshot {
        PrSnapshot {
            number,
            state: "MERGED".to_string(),
            head_ref_name: branch.to_string(),
            closed_at: Some(closed_secs_ago(closed_secs)),
            url: format!("https://github.com/senara-solutions/mika/pull/{number}"),
        }
    }

    fn index(prs: Vec<PrSnapshot>) -> HashMap<String, Vec<PrSnapshot>> {
        index_prs_by_branch(prs)
    }

    fn clean(path: &str) -> HashMap<String, WorkState> {
        HashMap::from([(path.to_string(), WorkState::Clean)])
    }

    fn select(
        entries: &[WorktreeEntry],
        prs: &HashMap<String, Vec<PrSnapshot>>,
        live: &LiveCwds,
        work: &HashMap<String, WorkState>,
    ) -> ReapSelection {
        select_worktrees_to_reap(entries, prs, live, work, now(), &ReapConfig::default())
    }

    fn no_processes() -> LiveCwds {
        LiveCwds::Enumerated(vec![])
    }

    fn only_reason(selection: &ReapSelection) -> Vec<&'static str> {
        selection.refusals.iter().map(|r| r.reason).collect()
    }

    // -- V1 : les trois tests négatifs du ticket -----------------------------

    /// V1.1 — PR mergée, aucun processus, propre → **retiré**.
    #[test]
    fn mika2420_pr_mergee_sans_processus_est_retiree() {
        let s = select(
            &[entry(WT, Some("fix/2420/x"))],
            &index(vec![merged_pr(2411, "fix/2420/x", 7200)]),
            &no_processes(),
            &clean(WT),
        );
        assert_eq!(s.candidates.len(), 1, "refus: {:?}", only_reason(&s));
        assert_eq!(s.candidates[0].pr_number, 2411);
        assert_eq!(s.candidates[0].branch, "fix/2420/x");
    }

    /// V1.2 — PR OPEN → **conservé**, motif `pr_open`.
    #[test]
    fn mika2420_pr_ouverte_est_conservee() {
        let mut open = merged_pr(2412, "fix/2420/x", 7200);
        open.state = "OPEN".to_string();
        open.closed_at = None;
        let s = select(
            &[entry(WT, Some("fix/2420/x"))],
            &index(vec![open]),
            &no_processes(),
            &clean(WT),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(only_reason(&s), vec![REASON_PR_OPEN]);
    }

    /// V1.3 — `cargo` actif avec son cwd dans le worktree, PR mergée →
    /// **conservé**, motif `live_process`.
    #[test]
    fn mika2420_un_processus_vivant_dedans_conserve() {
        let live = LiveCwds::Enumerated(vec![PathBuf::from(format!("{WT}/crates/mika-agent"))]);
        let s = select(
            &[entry(WT, Some("fix/2420/x"))],
            &index(vec![merged_pr(2411, "fix/2420/x", 7200)]),
            &live,
            &clean(WT),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(only_reason(&s), vec![REASON_LIVE_PROCESS]);
    }

    // -- V2 : contrôle négatif, terme par terme -----------------------------

    /// **Sans ce contrôle, V1 ne prouve rien** : il passerait aussi contre un
    /// prédicat qui ne lit rien et conserve tout. Chaque terme, invalidé un par
    /// un, doit faire **basculer le verdict** — et sous **son propre motif**.
    ///
    /// # Pourquoi le motif, et pas seulement « la population est vide »
    ///
    /// Une assertion sur la seule vacuité serait **insensible pour T3 et T4**,
    /// tous deux absorbés en aval par T5 : sans PR connue `newest_closed` reste
    /// `None` et T5 refuse ; une PR ouverte porte `closedAt: null` — la forme
    /// que `gh` rend réellement — et T5 refuse aussi. Retirer T3 ou T4 du
    /// prédicat laisserait donc un contrôle purement vacuitaire **vert**,
    /// c'est-à-dire produirait exactement le contrôle négatif décoratif que ce
    /// test existe pour ne pas être. Vérifié par mutation, pas par raisonnement
    /// (mika#2420 V2).
    ///
    /// Le motif discrimine : retirer T3 fait passer le refus de `pr_unknown` à
    /// `pr_closed_at_unreadable`, ce que l'assertion voit. C'est aussi la raison
    /// d'être de D10 — `pr_open` et `pr_unknown` mènent au même verdict et
    /// restent **distincts** parce qu'ils comptent deux populations différentes.
    #[test]
    fn mika2420_controle_negatif_chaque_terme_neutralise_fait_basculer() {
        let branch = "fix/2420/x";
        let base_entries = [entry(WT, Some(branch))];
        let base_prs = index(vec![merged_pr(2411, branch, 7200)]);
        let base_work = clean(WT);

        // Contrôle positif : la conjonction complète retient. Sans lui, les huit
        // contrôles négatifs ci-dessous seraient compatibles avec un prédicat
        // qui conserve tout.
        assert_eq!(
            select(&base_entries, &base_prs, &no_processes(), &base_work)
                .candidates
                .len(),
            1,
            "le contrôle positif doit retenir"
        );

        // Un chemin qui **ressemble** à un worktree géré sans en être un. Le
        // checkout primaire, lui, sort de la population *sans* produire de
        // refus — c'est la décision que pinne
        // `mika2420_le_checkout_primaire_sort_sans_bruit` juste après, et la
        // condition pour que `outside_managed_root` reste vide (HALTE 4).
        let outside = "/data/workspace/mika-platform/.claude/worktrees/../../../etc/evil";
        let open = {
            let mut pr = merged_pr(2412, branch, 7200);
            pr.state = "OPEN".to_string();
            // Une PR ouverte ne porte jamais de `closedAt` : la fixture reste
            // fidèle à ce que `gh` rend, et c'est précisément ce qui rend
            // l'assertion sur le motif nécessaire.
            pr.closed_at = None;
            pr
        };

        #[allow(clippy::type_complexity)]
        let cases: Vec<(
            &str,
            Vec<WorktreeEntry>,
            HashMap<String, Vec<PrSnapshot>>,
            LiveCwds,
            HashMap<String, WorkState>,
            &str,
        )> = vec![
            (
                "T1 — chemin hors racine gérée",
                vec![entry(outside, Some(branch))],
                base_prs.clone(),
                no_processes(),
                HashMap::from([(outside.to_string(), WorkState::Clean)]),
                REASON_OUTSIDE_MANAGED_ROOT,
            ),
            (
                "T2 — detached HEAD",
                vec![entry(WT, None)],
                base_prs.clone(),
                no_processes(),
                base_work.clone(),
                REASON_DETACHED_HEAD,
            ),
            (
                "T3 — aucune PR connue",
                base_entries.to_vec(),
                HashMap::new(),
                no_processes(),
                base_work.clone(),
                REASON_PR_UNKNOWN,
            ),
            (
                "T4 — une PR ouverte",
                base_entries.to_vec(),
                index(vec![merged_pr(2411, branch, 7200), open]),
                no_processes(),
                base_work.clone(),
                REASON_PR_OPEN,
            ),
            (
                "T5 — PR close à l'instant",
                base_entries.to_vec(),
                index(vec![merged_pr(2411, branch, 10)]),
                no_processes(),
                base_work.clone(),
                REASON_TOO_YOUNG,
            ),
            (
                "T6 — processus vivant dedans",
                base_entries.to_vec(),
                base_prs.clone(),
                LiveCwds::Enumerated(vec![PathBuf::from(WT)]),
                base_work.clone(),
                REASON_LIVE_PROCESS,
            ),
            (
                "T7 — modifications non committées",
                base_entries.to_vec(),
                base_prs.clone(),
                no_processes(),
                HashMap::from([(WT.to_string(), WorkState::Dirty)]),
                REASON_DIRTY,
            ),
            (
                "T7 — commits non poussés",
                base_entries.to_vec(),
                base_prs.clone(),
                no_processes(),
                HashMap::from([(WT.to_string(), WorkState::UnpushedCommits)]),
                REASON_UNPUSHED_COMMITS,
            ),
        ];

        for (label, entries, prs, live, work, expected_reason) in cases {
            let s = select(&entries, &prs, &live, &work);
            assert!(
                s.candidates.is_empty(),
                "{label} ne mord pas : le worktree est entré dans la population de retrait"
            );
            assert_eq!(
                only_reason(&s),
                vec![expected_reason],
                "{label} — le refus doit porter SON motif ; un motif voisin \
                 signifie que le terme a été absorbé par un terme en aval, donc \
                 que ce contrôle ne prouve plus rien"
            );
        }
    }

    /// **Le checkout primaire sort de la population sans produire de refus.**
    ///
    /// Il est dans le registre git de tout dépôt, à chaque tick, et sur chaque
    /// checkout configuré. Le compter comme refus écrirait une ligne d'audit par
    /// dépôt et par jour pour une entrée qui n'a jamais été candidate — et,
    /// surtout, rendrait `outside_managed_root` non vide en régime nominal, donc
    /// illisible comme signal (HALTE 4 : *ce motif doit rester vide*).
    ///
    /// Seul un chemin qui **ressemble** à un worktree géré sans en être un est
    /// compté : c'est celui-là qui mérite qu'on désarme et qu'on établisse
    /// comment il est arrivé jusqu'au prédicat.
    #[test]
    fn mika2420_le_checkout_primaire_sort_sans_bruit() {
        let branch = "main";
        let primary = "/data/workspace/mika-platform/mika";
        let s = select(
            &[entry(primary, Some(branch))],
            &index(vec![merged_pr(1, branch, 100_000)]),
            &no_processes(),
            &HashMap::from([(primary.to_string(), WorkState::Clean)]),
        );
        assert!(
            s.candidates.is_empty(),
            "le checkout primaire n'est jamais candidat"
        );
        assert!(
            s.refusals.is_empty(),
            "et il ne doit produire aucun refus, sinon `outside_managed_root` \
             est non vide en régime nominal et cesse d'être un signal"
        );
    }

    // -- V3 : fail-safe exhaustif (R7) --------------------------------------

    /// Pour chacun des sept termes, une entrée illisible rend **conserver**,
    /// avec son motif attendu. C'est l'invariant R7 : *un worktree dont un seul
    /// terme est illisible est conservé ; il n'existe aucune exception.*
    #[test]
    fn mika2420_chaque_terme_illisible_conserve_avec_son_motif() {
        let branch = "fix/2420/x";
        let entries = [entry(WT, Some(branch))];
        let prs = index(vec![merged_pr(2411, branch, 7200)]);

        // `closedAt` absent sur une PR terminale.
        let mut no_closed = merged_pr(2411, branch, 7200);
        no_closed.closed_at = None;
        let s = select(
            &entries,
            &index(vec![no_closed]),
            &no_processes(),
            &clean(WT),
        );
        assert_eq!(only_reason(&s), vec![REASON_PR_CLOSED_AT_UNREADABLE]);

        // `closedAt` illisible.
        let mut bad_closed = merged_pr(2411, branch, 7200);
        bad_closed.closed_at = Some("pas une date".to_string());
        let s = select(
            &entries,
            &index(vec![bad_closed]),
            &no_processes(),
            &clean(WT),
        );
        assert_eq!(only_reason(&s), vec![REASON_PR_CLOSED_AT_UNREADABLE]);

        // `/proc` illisible en entier.
        let s = select(&entries, &prs, &LiveCwds::Unavailable, &clean(WT));
        assert_eq!(only_reason(&s), vec![REASON_PROCESS_SCAN_UNREADABLE]);

        // `git status` / `git rev-list` muets : explicitement, puis par absence
        // de l'entrée — les deux doivent conserver.
        for work in [
            HashMap::from([(WT.to_string(), WorkState::Unreadable)]),
            HashMap::new(),
        ] {
            let s = select(&entries, &prs, &no_processes(), &work);
            assert_eq!(only_reason(&s), vec![REASON_WORK_STATE_UNREADABLE]);
        }

        // `detached HEAD`.
        let s = select(&[entry(WT, None)], &prs, &no_processes(), &clean(WT));
        assert_eq!(only_reason(&s), vec![REASON_DETACHED_HEAD]);
    }

    /// Une date de fermeture **dans le futur** (dérive d'horloge) ne doit pas
    /// fabriquer un âge géant : l'âge est ramené à 0, donc plus jeune que la
    /// grâce, donc conservé.
    #[test]
    fn mika2420_une_pr_close_dans_le_futur_est_traitee_comme_neuve() {
        let s = select(
            &[entry(WT, Some("fix/2420/x"))],
            &index(vec![merged_pr(2411, "fix/2420/x", -7200)]),
            &no_processes(),
            &clean(WT),
        );
        assert_eq!(only_reason(&s), vec![REASON_TOO_YOUNG]);
    }

    /// **T4 est en négatif pour cette raison exacte.** Deux PR sur une même
    /// branche, une close et une rouverte : « il existe une PR mergée » serait
    /// vrai et supprimerait un worktree dont une PR est ouverte.
    #[test]
    fn mika2420_deux_pr_sur_une_branche_dont_une_ouverte_conservent() {
        let branch = "fix/2420/x";
        let mut reopened = merged_pr(2413, branch, 60);
        reopened.state = "OPEN".to_string();
        reopened.closed_at = None;
        let s = select(
            &[entry(WT, Some(branch))],
            &index(vec![merged_pr(2411, branch, 100_000), reopened]),
            &no_processes(),
            &clean(WT),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(only_reason(&s), vec![REASON_PR_OPEN]);
    }

    // -- V4 : gardes structurelles ------------------------------------------

    /// Aucun chemin hors de `.claude/worktrees/` n'atteint la population, y
    /// compris par un `..` dans le chemin du registre ou un chemin relatif.
    #[test]
    fn mika2420_aucun_chemin_hors_racine_geree_nest_accepte() {
        assert!(is_managed_worktree_path(WT));
        for bad in [
            "/data/workspace/mika-platform/mika",
            "/data/workspace/mika-platform/.claude/worktrees/../../../etc",
            ".claude/worktrees/x/mika",
            "/home/user/worktrees/x",
            "",
        ] {
            assert!(
                !is_managed_worktree_path(bad),
                "{bad} ne doit jamais être accepté"
            );
        }
    }

    /// **Le lien symbolique est la moitié que la garde syntaxique ne peut pas
    /// voir** (V4). Un chemin du registre situé sous `.claude/worktrees/` mais
    /// pointant ailleurs passe `is_managed_worktree_path` — et
    /// `git worktree remove --force` supprimerait la cible. Seule la
    /// canonicalisation le révèle, d'où la re-vérification juste avant la
    /// disposition.
    #[test]
    fn mika2420_un_lien_symbolique_nechappe_pas_a_la_garde_de_chemin() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join(".claude").join("worktrees").join("evil");
        std::fs::create_dir_all(&managed).unwrap();

        let precious = tmp.path().join("precious");
        std::fs::create_dir_all(&precious).unwrap();

        let link = managed.join("mika");
        std::os::unix::fs::symlink(&precious, &link).unwrap();
        let link_str = link.to_str().unwrap();

        // La garde syntaxique le laisse passer : le chemin *est* sous la racine
        // gérée, et rien dans sa forme ne trahit le lien.
        assert!(
            is_managed_worktree_path(link_str),
            "prémisse du test : la garde syntaxique ne voit pas le lien"
        );
        // La canonicalisation le refuse.
        assert!(
            !canonical_path_is_managed(link_str),
            "un lien vers l'extérieur de la racine gérée doit être refusé"
        );

        // Contrôle négatif : un vrai répertoire au même emplacement passe, sinon
        // l'assertion ci-dessus serait satisfaite par une garde qui refuse tout.
        let real = managed.join("mika-reel");
        std::fs::create_dir_all(&real).unwrap();
        assert!(canonical_path_is_managed(real.to_str().unwrap()));

        // Et un chemin disparu entre la sélection et la disposition : pas de
        // preuve, pas de suppression.
        assert!(!canonical_path_is_managed(
            managed.join("absent").to_str().unwrap()
        ));
    }

    /// **Format de fil.** Les motifs atterrissent dans
    /// `audit_events.after_value` et l'opérateur en fait des `GROUP BY` : deux
    /// orthographes d'un même motif couperaient une population en deux sans le
    /// dire.
    #[test]
    fn mika2420_les_motifs_sont_un_format_de_fil() {
        assert_eq!(
            ALL_REFUSAL_REASONS,
            &[
                "pr_open",
                "pr_unknown",
                "too_young",
                "live_process",
                "dirty",
                "unpushed_commits",
                "detached_head",
                "outside_managed_root",
                "pr_closed_at_unreadable",
                "work_state_unreadable",
                "process_scan_unreadable",
            ],
            "renommer un motif est une rupture de format de fil : la dater dans \
             CLAUDE.md, jamais mettre ce test à jour en silence"
        );
        let mut seen = std::collections::HashSet::new();
        for r in ALL_REFUSAL_REASONS {
            assert!(seen.insert(*r), "motif dupliqué: {r}");
        }
    }

    /// Les `tool_name` d'audit des retraits **et** des observations ont **un
    /// seul writer** dans le crate.
    ///
    /// Un test comportemental ne peut pas voir cette classe : un second writer
    /// ne rendrait aucune décision fausse, il rendrait
    /// `SELECT … WHERE tool_name = 'worktree_reaped'` (ou sa jumelle
    /// `'worktree_reap_would_dispose'`, mika#2469) inexacte, en silence.
    /// L'allowlist est **vide** et le reste : quand la garde tire, on retire le
    /// second site, on n'y ajoute pas une entrée.
    #[test]
    fn mika2420_le_tool_name_daudit_a_un_seul_writer() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("worktree_reaper.rs");
        // Écrites en deux morceaux pour que la garde ne se dénonce pas elle-même.
        let needles = [
            format!("worktree{}", "_reaped"),
            format!("worktree_reap_{}", "would_dispose"),
        ];

        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("lecture de src/").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || path == this_module
                    || crate::source_scan::is_test_source_path(&path)
                {
                    continue;
                }
                let content = std::fs::read_to_string(&path).expect("lecture de fichier source");
                scanned += 1;
                for needle in &needles {
                    if content.contains(needle.as_str()) {
                        offenders.push(format!("{} — `{needle}`", path.display()));
                    }
                }
            }
        }
        assert!(scanned > 0, "la garde n'a scanné aucun fichier");
        assert!(
            offenders.is_empty(),
            "mika#2420/mika#2469 — `worktree_reaped` et `worktree_reap_would_dispose` \
             sont SOLE WRITER de `worktree_reaper.rs`. Un second writer rendrait \
             la requête opérateur inexacte sans rien casser.\n{}",
            offenders.join("\n")
        );
    }

    // -- mika#2469 : en observe, la ligne dit ce qu'elle ferait ---------------

    /// T1 (R2/R3) — en `observe`, l'audit ne revendique pas un retrait : la
    /// ligne s'appelle `worktree_reap_would_dispose`, et **aucune** ligne
    /// `worktree_reaped` n'existe.
    #[tokio::test]
    async fn mika2469_en_observe_laudit_ne_revendique_pas_un_retrait() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let candidate = ReapCandidate {
            path: WT.to_string(),
            branch: "fix/2469/x".to_string(),
            pr_number: 2469,
            pr_state: "MERGED".to_string(),
            pr_url: "https://github.com/senara-solutions/mika/pull/2469".to_string(),
        };
        let size = SizeMeasurement {
            bytes: Some(1_000),
            truncated: false,
        };
        record_reaped(
            &db,
            "session-2469",
            &candidate,
            &size,
            Disposition::Observe,
            "trace-1",
        )
        .await;

        let events = db.get_audit_events("session-2469").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == WOULD_DISPOSE_TOOL)
            .expect("une ligne worktree_reap_would_dispose doit exister en observe");
        assert_eq!(row.target_key, format!("worktree:{WT}"));
        let reasoning = row.reasoning.as_deref().unwrap_or_default();
        assert!(reasoning.contains("disposition=observe"), "{reasoning}");
        assert!(
            !events.iter().any(|e| e.tool_name == REAPED_TOOL),
            "en observe, aucune ligne `worktree_reaped` ne doit être écrite"
        );
    }

    /// T2 (R4, D2) — le triplet a une seule source, et les deux messages sont
    /// pinés à l'octet près (F3 arch : tester la chose, pas une ombre).
    #[test]
    fn mika2469_le_triplet_a_une_seule_source() {
        let armed = outcome_for(Disposition::Armed);
        let observe = outcome_for(Disposition::Observe);
        assert_eq!(armed.event, REAPED_TOOL);
        assert_eq!(armed.message, REAPED_MESSAGE);
        assert_eq!(observe.event, WOULD_DISPOSE_TOOL);
        assert_eq!(observe.message, WOULD_DISPOSE_MESSAGE);
        assert_ne!(armed.event, observe.event);
        assert_eq!(
            REAPED_MESSAGE,
            "worktree_reap: worktree de PR terminale retiré"
        );
        assert_eq!(
            WOULD_DISPOSE_MESSAGE,
            "worktree_reap: worktree de PR terminale éligible — observe, non retiré"
        );
        // En surplus : documente l'intention si la constante est un jour reformulée.
        assert!(WOULD_DISPOSE_MESSAGE.contains("non retiré"));
    }

    // -- V5 : le cap est un cap sur les écritures ---------------------------

    /// **Leçon mika#2347 transposée.** La fonction pure ne tronque pas : le cap
    /// vit chez l'appelant, après le filtre. Appliqué en amont, il plafonnerait
    /// les *sauts* — et des candidats refusés consommeraient le tick à la place
    /// de candidats traitables.
    #[test]
    fn mika2420_la_fonction_pure_ne_tronque_pas() {
        let entries: Vec<WorktreeEntry> = (1..=6)
            .map(|i| {
                entry(
                    &format!("/data/workspace/mika-platform/.claude/worktrees/wt-{i}/mika"),
                    Some(&format!("fix/{i}/x")),
                )
            })
            .collect();
        let prs = index(
            (1..=6)
                .map(|i| merged_pr(2400 + i, &format!("fix/{i}/x"), 7200))
                .collect(),
        );
        let work: HashMap<String, WorkState> = entries
            .iter()
            .map(|e| (e.path.clone(), WorkState::Clean))
            .collect();

        for max_per_tick in [1, 3, 100] {
            let cfg = ReapConfig {
                max_per_tick,
                ..ReapConfig::default()
            };
            let s = select_worktrees_to_reap(&entries, &prs, &no_processes(), &work, now(), &cfg);
            assert_eq!(
                s.candidates.len(),
                6,
                "max_per_tick={max_per_tick} ne doit pas influer sur la sélection"
            );
        }
    }

    /// Et le corollaire : un candidat refusé par T7 ne doit pas consommer la
    /// place d'un candidat traitable. Deux worktrees, le premier dirty : le
    /// second reste dans la population.
    #[test]
    fn mika2420_un_refus_ne_consomme_pas_la_place_dun_traitable() {
        let a = "/data/workspace/mika-platform/.claude/worktrees/wt-a/mika";
        let b = "/data/workspace/mika-platform/.claude/worktrees/wt-b/mika";
        let entries = [entry(a, Some("fix/a/x")), entry(b, Some("fix/b/x"))];
        let prs = index(vec![
            merged_pr(1, "fix/a/x", 7200),
            merged_pr(2, "fix/b/x", 7200),
        ]);
        let work = HashMap::from([
            (a.to_string(), WorkState::Dirty),
            (b.to_string(), WorkState::Clean),
        ]);
        let s = select(&entries, &prs, &no_processes(), &work);
        assert_eq!(s.candidates.len(), 1);
        assert_eq!(s.candidates[0].path, b);
        assert_eq!(only_reason(&s), vec![REASON_DIRTY]);
    }

    // -- Configuration -------------------------------------------------------

    #[test]
    fn mika2420_les_trois_paliers_de_configuration() {
        assert_eq!(
            parse_positive_i64(None, GRACE_DEFAULT_SECS, GRACE_ENV),
            GRACE_DEFAULT_SECS
        );
        assert_eq!(
            parse_positive_i64(Some("  "), GRACE_DEFAULT_SECS, GRACE_ENV),
            GRACE_DEFAULT_SECS
        );
        for bad in ["abc", "0", "-1"] {
            assert_eq!(
                parse_positive_i64(Some(bad), GRACE_DEFAULT_SECS, GRACE_ENV),
                GRACE_DEFAULT_SECS,
                "{bad} doit retomber sur le défaut, jamais désarmer"
            );
            assert_eq!(
                parse_positive_usize(Some(bad), MAX_PER_TICK_DEFAULT, MAX_PER_TICK_ENV),
                MAX_PER_TICK_DEFAULT
            );
        }
        assert_eq!(parse_positive_i64(Some("60"), 900, GRACE_ENV), 60);
        assert_eq!(parse_positive_usize(Some("7"), 3, MAX_PER_TICK_ENV), 7);
    }

    /// Une valeur non reconnue laisse le scan **armé** : un désarmement par
    /// coquille ferait croire le scan actif alors qu'il ne supprimerait plus
    /// rien.
    #[test]
    fn mika2420_une_disposition_non_reconnue_reste_armee() {
        assert_eq!(parse_disposition(None), Disposition::Armed);
        assert_eq!(parse_disposition(Some("")), Disposition::Armed);
        assert_eq!(parse_disposition(Some("armed")), Disposition::Armed);
        assert_eq!(parse_disposition(Some(" OBSERVE ")), Disposition::Observe);
        assert_eq!(parse_disposition(Some("observ")), Disposition::Armed);
        assert_eq!(parse_disposition(Some("0")), Disposition::Armed);
    }

    /// **Livré armé** (D8, précédent mika#2272). Valeur de contrat, d'où
    /// l'assertion.
    #[test]
    fn mika2420_le_defaut_est_arme() {
        assert_eq!(ReapConfig::default().disposition, Disposition::Armed);
        assert_eq!(ReapConfig::default().grace_secs, 900);
        assert_eq!(ReapConfig::default().max_per_tick, 3);
    }

    #[test]
    fn mika2420_la_liste_de_checkouts_tolere_les_vides() {
        assert_eq!(parse_repo_dirs(None), vec![PathBuf::from(DEFAULT_REPO_DIR)]);
        assert_eq!(
            parse_repo_dirs(Some("  ")),
            vec![PathBuf::from(DEFAULT_REPO_DIR)]
        );
        assert_eq!(
            parse_repo_dirs(Some("::")),
            vec![PathBuf::from(DEFAULT_REPO_DIR)]
        );
        assert_eq!(
            parse_repo_dirs(Some("/a/b: /c/d :")),
            vec![PathBuf::from("/a/b"), PathBuf::from("/c/d")]
        );
    }

    // -- Parsing -------------------------------------------------------------

    #[test]
    fn mika2420_le_registre_git_se_parse() {
        let porcelain = "\
worktree /data/workspace/mika-platform/mika
HEAD abc123
branch refs/heads/main

worktree /data/workspace/mika-platform/.claude/worktrees/fix-2420-x/mika
HEAD def456
branch refs/heads/fix/2420/x

worktree /data/workspace/mika-platform/.claude/worktrees/detached/mika
HEAD 789abc
detached
";
        let entries = parse_worktree_registry(porcelain);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].branch.as_deref(), Some("fix/2420/x"));
        assert_eq!(entries[2].branch, None, "detached HEAD n'a pas de branche");
    }

    /// Une entrée `prunable` est écartée : son répertoire n'existe déjà plus,
    /// elle relève de `git worktree prune` et non d'un retrait.
    #[test]
    fn mika2420_les_entrees_prunable_sont_ecartees() {
        let porcelain = "\
worktree /data/workspace/mika-platform/.claude/worktrees/gone/mika
HEAD abc
branch refs/heads/fix/gone/x
prunable gitdir file points to non-existent location

worktree /data/workspace/mika-platform/.claude/worktrees/live/mika
HEAD def
branch refs/heads/fix/live/x
";
        let entries = parse_worktree_registry(porcelain);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].path.ends_with("live/mika"));
    }

    #[test]
    fn mika2420_owner_repo_se_derive_du_remote() {
        for (url, expected) in [
            (
                "git@github.com:senara-solutions/mika.git",
                "senara-solutions/mika",
            ),
            (
                "git@github.com:senara-solutions/mika",
                "senara-solutions/mika",
            ),
            (
                "https://github.com/senara-solutions/mika.git",
                "senara-solutions/mika",
            ),
            (
                "https://github.com/senara-solutions/mika\n",
                "senara-solutions/mika",
            ),
            (
                "ssh://git@github.com/senara-solutions/mika.git",
                "senara-solutions/mika",
            ),
        ] {
            assert_eq!(
                parse_owner_repo(url).as_deref(),
                Some(expected),
                "url={url}"
            );
        }
        assert_eq!(parse_owner_repo("not-a-url"), None);
        assert_eq!(parse_owner_repo(""), None);
    }

    /// `state` / `headRefName` absents ⇒ erreur de parsing, jamais un vide. Un
    /// `#[serde(default)]` ici ferait *entrer* un worktree dans la population
    /// sur une information manquante.
    #[test]
    fn mika2420_un_champ_manquant_est_une_erreur_pas_un_vide() {
        let sans_state = r#"[{"number":1,"headRefName":"x","closedAt":null,"url":"u"}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(sans_state).is_err());

        let sans_branch = r#"[{"number":1,"state":"MERGED","closedAt":null,"url":"u"}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(sans_branch).is_err());

        let complet = r#"[{"number":1,"state":"MERGED","headRefName":"x","closedAt":"2026-09-19T10:00:00Z","url":"u"}]"#;
        assert!(serde_json::from_str::<Vec<PrSnapshot>>(complet).is_ok());
    }

    // -- Le walker de taille -------------------------------------------------

    /// **`target/` EST la mesure** — exactement l'inverse de
    /// `worktree_activity`, dont l'exclusion est porteuse pour son propre
    /// prédicat. Réutiliser ce walker-là rapporterait quelques mégaoctets là où
    /// il y en a trente-quatre gigaoctets.
    #[test]
    fn mika2420_le_walker_compte_target_contrairement_a_son_voisin() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target").join("release");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("big.bin"), vec![0u8; 4096]).unwrap();
        std::fs::write(tmp.path().join("small.rs"), b"fn main() {}").unwrap();

        let measured = measure_tree_size(tmp.path()).bytes.expect("une mesure");
        assert!(
            measured >= 4096,
            "le contenu de target/ doit être compté, mesuré {measured}"
        );

        // Le contrôle négatif qui donne son sens au précédent : le walker voisin
        // exclut `target/` et rend donc un chiffre incomparable.
        assert!(
            crate::task_engine::worktree_activity::EXCLUDED_DIR_NAMES.contains(&"target"),
            "si le voisin cessait d'exclure target/, ce module n'aurait plus de \
             raison d'avoir son propre walker — relire D5 avant de fusionner"
        );
    }

    /// `null` n'est jamais `0` (doctrine mika#2331) : un répertoire absent rend
    /// « non mesuré », pas « zéro octet ».
    #[test]
    fn mika2420_une_mesure_absente_nest_jamais_zero() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            measure_tree_size(&tmp.path().join("nope")),
            SizeMeasurement::default()
        );
        assert_eq!(measure_tree_size(&tmp.path().join("nope")).bytes, None);

        // Un répertoire vide n'a pas de fichier : `None`, pas `Some(0)`.
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(measure_tree_size(&empty).bytes, None);
    }

    // -- Clés d'audit --------------------------------------------------------

    #[test]
    fn mika2420_les_cles_daudit_ont_la_forme_documentee() {
        assert_eq!(reaped_audit_key(WT), format!("worktree:{WT}"));
        assert_eq!(
            refusal_audit_key(WT, REASON_DIRTY),
            format!("worktree:{WT}@dirty")
        );
        // Le motif est dans la clé : un changement de motif réécrit (D9).
        assert_ne!(
            refusal_audit_key(WT, REASON_DIRTY),
            refusal_audit_key(WT, REASON_PR_OPEN)
        );
    }

    /// AC du garde-fou 3 : chaque retrait écrit une ligne d'audit portant le
    /// chemin, la branche, le numéro de PR et l'espace récupéré.
    #[tokio::test]
    async fn mika2420_chaque_retrait_ecrit_une_ligne_daudit() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let candidate = ReapCandidate {
            path: WT.to_string(),
            branch: "fix/2420/x".to_string(),
            pr_number: 2411,
            pr_state: "MERGED".to_string(),
            pr_url: "https://github.com/senara-solutions/mika/pull/2411".to_string(),
        };
        let size = SizeMeasurement {
            bytes: Some(34_000_000_000),
            truncated: false,
        };
        record_reaped(
            &db,
            "session-2420",
            &candidate,
            &size,
            Disposition::Armed,
            "trace-1",
        )
        .await;

        let events = db.get_audit_events("session-2420").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == REAPED_TOOL)
            .expect("une ligne worktree_reaped doit exister");
        assert_eq!(row.target_key, format!("worktree:{WT}"));
        assert_eq!(row.after_value.as_deref(), Some("34000000000"));
        let reasoning = row.reasoning.as_deref().unwrap_or_default();
        assert!(reasoning.contains("pr=2411"), "{reasoning}");
        assert!(reasoning.contains("branch=fix/2420/x"), "{reasoning}");
        assert!(reasoning.contains("disposition=armed"), "{reasoning}");
    }

    /// Un refus est écrit **une fois** par `(worktree, motif)` et par 24 h ; un
    /// changement de motif réécrit (D9, doctrine mika#2131).
    #[tokio::test]
    async fn mika2420_un_refus_est_dedupe_mais_un_changement_de_motif_reecrit() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let dirty = ReapRefusal {
            path: WT.to_string(),
            branch: Some("fix/2420/x".to_string()),
            reason: REASON_DIRTY,
        };
        for _ in 0..3 {
            record_refusal(&db, "session-2420", &dirty, Utc::now(), "trace-1").await;
        }
        let open = ReapRefusal {
            reason: REASON_PR_OPEN,
            ..dirty.clone()
        };
        record_refusal(&db, "session-2420", &open, Utc::now(), "trace-1").await;

        let events = db.get_audit_events("session-2420").await.unwrap();
        let skipped: Vec<_> = events
            .iter()
            .filter(|e| e.tool_name == SKIPPED_TOOL)
            .collect();
        assert_eq!(
            skipped.len(),
            2,
            "trois ticks sur le même motif = une ligne ; un motif différent = une seconde"
        );
        let reasons: std::collections::HashSet<_> = skipped
            .iter()
            .filter_map(|e| e.after_value.as_deref())
            .collect();
        assert_eq!(
            reasons,
            std::collections::HashSet::from([REASON_DIRTY, REASON_PR_OPEN])
        );
    }
}

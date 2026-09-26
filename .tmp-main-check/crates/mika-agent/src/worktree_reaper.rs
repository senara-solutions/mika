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
//! | `MIKA_WORKTREE_REAP_DISPOSITION=observe` | le scan mesure et journalise, **ne supprime rien** — écrit `worktree_reap_would_dispose`, jamais `worktree_reaped` (mika#2469) | validation d'une nouvelle machine |
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
use std::time::{Duration, Instant, SystemTime};
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
/// Pas de branche attachée **et** pas de SHA exploitable (T2) — rare, anomalie
/// git.
///
/// # Le sens s'est resserré, et la scission est datée (mika#2518 R-3)
///
/// Avant mika#2518 ce motif voulait dire « pas de branche attachée », point. Un
/// worktree détaché est désormais résolu par le SHA de son `HEAD`, donc ce motif
/// ne couvre plus que le cas où **ce SHA lui-même** est inexploitable : ligne
/// `HEAD` absente du porcelain, SHA nul (`000…0` — mesuré en production sur le
/// checkout principal), non hexadécimal, ou de longueur non canonique. La
/// population « SHA lisible, aucune PR à ce SHA » a reçu son propre nom,
/// [`REASON_DETACHED_HEAD_PR_UNKNOWN`].
///
/// **Coût nommé** (motif mika#2361) : une requête `GROUP BY after_value` qui
/// enjambe le déploiement compare deux vocabulaires. Les lignes antérieures
/// gardent `detached_head` et **ne sont pas réécrites** — les réécrire rendrait
/// faux ce qu'elles ont dit quand elles ont été écrites. Un opérateur qui
/// compare de part et d'autre doit **sommer les deux noms** (HALTE 3 du
/// `CLAUDE.md` racine).
pub const REASON_DETACHED_HEAD: &str = "detached_head";
/// Worktree détaché, SHA de `HEAD` lisible, **aucune** PR à ce SHA (T3 sur la
/// clé SHA) — nominal pour un worktree hors boucle (mika#2518).
///
/// Délibérément distinct de [`REASON_DETACHED_HEAD`] alors qu'il mène au même
/// verdict : le premier est une **anomalie git** (remède : inspecter le
/// worktree), celui-ci est **nominal** — et c'est surtout *la sonde qui dit que
/// la clé SHA ne mord pas* (sonde S1). Les confondre cacherait un blocage
/// permanent à l'intérieur d'un état normal.
///
/// Distinct aussi de [`REASON_PR_UNKNOWN`], qui pose la même question sur la clé
/// **branche** : celui-là est fréquent et nominal (« groomé, pas encore
/// implémenté »), celui-ci est l'attribution de ce ticket.
pub const REASON_DETACHED_HEAD_PR_UNKNOWN: &str = "detached_head_pr_unknown";
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
    REASON_DETACHED_HEAD_PR_UNKNOWN,
];

/// Par quelle clé un worktree a été rattaché à ses PR — **la branche attachée**.
///
/// # Format de fil (mika#2518 R3)
///
/// Ces valeurs atterrissent en tête de `audit_events.reasoning` et sur le champ
/// `resolution` de la ligne INFO ; l'opérateur en fait des
/// `reasoning LIKE 'resolution=detached_sha%'`. Deux orthographes couperaient une
/// population en deux sans le dire. Épinglé par
/// [`tests::mika2518_les_resolutions_sont_un_format_de_fil`].
///
/// # Pourquoi un champ et non un second `tool_name`
///
/// AC4 de mika#2518 demande « un motif distinct (p. ex. `reaped_detached_merged`)
/// séparable de `worktree_reaped` nominal ». Le « p. ex. » est pris au mot :
/// créer un second `tool_name` tronquerait **en silence**
/// `SELECT … WHERE tool_name = 'worktree_reaped'`, c'est-à-dire le garde-fou 3 de
/// mika#2420 et une requête publiée dans le `CLAUDE.md` racine.
///
/// La maison a les deux motifs et les distingue : **deux noms** quand chaque nom
/// porte sa propre cause et que les populations ne doivent jamais être sommées
/// (`phantom_aged_out` / `phantom_sweep_spared`, mika#2156 ;
/// `qa_deadline_verdict` / `qa_callback_verdict`, mika#2368) ; **un nom, le
/// discriminant dans le champ** quand les deux issues appartiennent au même
/// dispatcheur et à la même population (`ready_label_outcome`, mika#2323 ;
/// `task_engine_groom_pilot_dispatcher`, mika#2498). Ici les deux retraits sont
/// faits par **le même bras**, sous la **même conjonction** de sept termes, avec
/// la **même létalité** : seule la clé de résolution diffère. C'est le second
/// motif.
pub const RESOLUTION_BRANCH: &str = "branch";
/// Par quelle clé un worktree a été rattaché à ses PR — **le SHA de son `HEAD`
/// détaché**, apparié au `headRefOid` d'une PR (mika#2518).
///
/// `"detached_sha"` et non `"detached_head"` **délibérément** : ce dernier est
/// déjà un motif de refus ([`REASON_DETACHED_HEAD`]). Deux vocabulaires distincts
/// qui partageraient une chaîne se liraient mal, même en vivant dans des champs
/// différents — et le nom retenu dit la clé réellement employée.
pub const RESOLUTION_DETACHED_SHA: &str = "detached_sha";
/// Les deux clés de résolution, en un seul lieu.
pub const ALL_RESOLUTIONS: &[&str] = &[RESOLUTION_BRANCH, RESOLUTION_DETACHED_SHA];

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
// La sonde de saleté du checkout principal (mika#2449)
// ---------------------------------------------------------------------------

/// `audit_events.tool_name` écrit quand un checkout **principal** configuré
/// porte des modifications non committées (mika#2449 R3).
///
/// **SOLE WRITER** — ce module est le seul site qui écrit ce nom, épinglé par
/// [`tests::mika2449_le_tool_name_main_checkout_dirty_a_un_seul_writer`]. Un
/// second writer rendrait inexacte, en silence, la requête d'attribution que
/// chaque ligne recopie.
///
/// # Pourquoi cette sonde vit ici (D2)
///
/// Le faucheur énumère **déjà** les checkouts via `MIKA_WORKTREE_REAP_REPO_DIRS`
/// (défaut : `/data/workspace/mika-platform/mika` — exactement le checkout sali
/// le 2026-09-21), tourne **déjà** sur un tick, écrit **déjà** ses lignes
/// d'audit. Coût : un `git status --porcelain` par checkout et par tick. Zéro
/// nouvelle variable, zéro nouveau scan récurrent.
///
/// # Ce que la sonde fait, et ne fait PAS (D3, D5, R5)
///
/// Elle **date** : la prochaine occurrence a un instant, et la requête
/// `tool_calls` qui **nomme** le producteur (celle qui a résolu M0 en une
/// requête) est recopiée dans le `reasoning` avec ses bornes. Elle ne nomme
/// **aucun producteur elle-même** — écrire « le dernier dispatch » serait la
/// dérivation tardive que mika#2368 refuse. Elle ne nettoie **rien** : ni pop,
/// ni stash, ni reset (décision Vincent). Et elle ne **bloque rien** : un
/// checkout principal sale n'empêche pas un dispatch (le worktree est ailleurs,
/// les trois PR du sinistre ont abouti) ; il empêche un `pull --ff-only`, un
/// geste d'opérateur. Refuser le dispatch coucherait la boucle pour un défaut de
/// poste de build.
pub const MAIN_CHECKOUT_DIRTY_TOOL: &str = "main_checkout_dirty";

/// Plafond de chemins portés par une ligne (même valeur que `DIRTY_FILES` de
/// `dispatch-lib.sh`, `head -20`). **Jamais** de contenu de fichier.
const MAIN_CHECKOUT_DIRTY_MAX_PATHS: usize = 20;

/// Borne de temps du `git status` de la sonde. Un checkout sur un disque lent
/// ne doit pas retenir le tick : au-delà, le checkout est « illisible », ce
/// qui est un signal nommé, jamais « propre ».
const MAIN_CHECKOUT_STATUS_TIMEOUT: Duration = Duration::from_secs(20);

/// L'état d'un checkout principal, tel que [`classify_main_checkout`] le lit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainCheckoutState {
    /// `git status --porcelain` vide. **Zéro ligne** (AC4).
    Clean,
    /// Au moins une entrée. `files` est plafonné à
    /// [`MAIN_CHECKOUT_DIRTY_MAX_PATHS`] ; `file_count` est le compte **réel**.
    Dirty {
        file_count: usize,
        files: Vec<String>,
        truncated: bool,
        /// Empreinte stable de la liste complète (ordre indifférent) — la clé
        /// de déduplication : même liste sur deux ticks = une ligne ; liste
        /// changée = une seconde (D4).
        fingerprint: String,
    },
    /// `git status` n'a pas répondu ou a échoué. Sort le checkout de la
    /// population **et le dit** — jamais replié sur `Clean` (R6, D5).
    Unreadable,
}

/// Fonction **pure** : décide l'état d'un checkout depuis la sortie de
/// `git status --porcelain` (`None` = commande échouée ou hors délai).
///
/// Testable sans git. Les chemins sont les lignes porcelain entières (statut
/// et chemin, ex. `M  scripts/x`, `?? site/y`), triées pour que l'empreinte ne
/// dépende pas de l'ordre de sortie.
pub fn classify_main_checkout(status_porcelain: Option<&str>) -> MainCheckoutState {
    let Some(out) = status_porcelain else {
        return MainCheckoutState::Unreadable;
    };
    let mut lines: Vec<String> = out
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        return MainCheckoutState::Clean;
    }
    lines.sort();
    let file_count = lines.len();
    let fingerprint = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        for l in &lines {
            l.hash(&mut h);
        }
        format!("{:016x}", h.finish())
    };
    let truncated = file_count > MAIN_CHECKOUT_DIRTY_MAX_PATHS;
    lines.truncate(MAIN_CHECKOUT_DIRTY_MAX_PATHS); // safe-byte-slice: Vec<String> element count, not a byte offset into a str
    MainCheckoutState::Dirty {
        file_count,
        files: lines,
        truncated,
        fingerprint,
    }
}

/// Clé d'audit de la sonde : `main_checkout:<repo_dir>@<empreinte>`.
///
/// L'empreinte est **dans la clé** pour que la déduplication soit par
/// `(checkout, liste)`, sur le modèle de [`refusal_audit_key`] et de
/// `pr:{repo}#{n}@{sha}` (mika#2347). Le préfixe `main_checkout:<repo_dir>`
/// reste stable, donc `WHERE target_key LIKE 'main_checkout:/data/…/mika@%'`
/// liste toutes les saletés d'un même checkout.
pub fn main_checkout_audit_key(repo_dir: &str, fingerprint: &str) -> String {
    format!("main_checkout:{repo_dir}@{fingerprint}")
}

/// La requête d'attribution que la ligne d'audit recopie (§ Sondes 1b du plan).
///
/// Bornes : `window_start` = le dernier tick où ce checkout a été vu **propre**
/// par ce process (« unknown » après un redémarrage ou au premier tick),
/// `window_end` = l'instant de la détection. Le filtre de chemin porte les deux
/// derniers segments du checkout (`mika-platform/mika`), qui est la forme que
/// les commandes mesurées écrivent (`cd ~/workspace/mika-platform/mika`).
pub fn main_checkout_attribution_query(
    repo_dir: &str,
    window_start: Option<&str>,
    window_end: &str,
) -> String {
    let tail = {
        let parts: Vec<&str> = repo_dir.trim_end_matches('/').rsplit('/').take(2).collect();
        parts.iter().rev().cloned().collect::<Vec<_>>().join("/")
    };
    let start = window_start.unwrap_or("<dernier tick propre : inconnu, prendre la ligne main_checkout_dirty précédente ou le dernier déploiement>");
    format!(
        "SELECT id, agent_id, session_id, created_at, substr(input, 1, 200) FROM tool_calls \
         WHERE tool_name = 'run_shell' AND created_at BETWEEN '{start}' AND '{window_end}' \
         AND input LIKE '%{tail}%' AND (input LIKE '%checkout %--%' OR input LIKE '%stash%' \
         OR input LIKE '%reset%' OR input LIKE '%merge%' OR input LIKE '%pull%') \
         ORDER BY created_at;"
    )
}

/// Le dernier instant où chaque checkout a été vu propre **par ce process**.
///
/// En mémoire, perdu au redémarrage **à dessein** : « checkout propre ⇒ zéro
/// ligne » (AC4) interdit de persister les ticks propres, et un process neuf
/// dit « inconnu » plutôt que d'inventer une borne. C'est la borne basse de la
/// requête d'attribution ; sans elle la requête n'a pas de fenêtre et
/// l'enquête repart de zéro (M4).
fn last_clean_ticks() -> &'static std::sync::Mutex<HashMap<String, DateTime<Utc>>> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<HashMap<String, DateTime<Utc>>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

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
/// disposition est gardée*. En observation, les lignes d'audit sont écrites sous
/// [`WOULD_DISPOSE_TOOL`] (jamais [`REAPED_TOOL`], mika#2469) avec
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
///
/// `env_name` est un **paramètre** depuis mika#2497 : deux dispositions
/// distinctes (le faucheur et la purge de `target/`) lisent la même table de
/// vérité, et deux copies de cette table seraient deux copies qui peuvent
/// diverger sur le palier du milieu — celui qui porte le WARN.
pub fn parse_disposition(raw: Option<&str>, env_name: &str) -> Disposition {
    match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("armed") => Disposition::Armed,
        Some("observe") => Disposition::Observe,
        Some(other) => {
            warn!(
                value = %format!("{other:?}"),
                "worktree_reap: valeur non reconnue pour {env_name} — le scan reste armé"
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
        disposition: parse_disposition(
            std::env::var(DISPOSITION_ENV).ok().as_deref(),
            DISPOSITION_ENV,
        ),
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
    /// `branch`. Depuis mika#2518 ce n'est plus une sortie de population — c'est
    /// le basculement vers la clé SHA ci-dessous.
    pub branch: Option<String>,
    /// Le SHA brut de la ligne `HEAD <sha>`, que `git worktree list --porcelain`
    /// émet pour **toute** entrée, détachée comprise.
    ///
    /// Brut : la normalisation est le travail de [`usable_head_sha`], site
    /// unique. `None` quand la ligne est absente du porcelain.
    pub head: Option<String>,
}

/// Le SHA d'un `HEAD` détaché, s'il est exploitable comme clé de résolution.
///
/// Refuse : la chaîne vide, le SHA nul (`000…0` — **mesuré** en production, c'est
/// ce que le porcelain rend pour le checkout principal), et tout ce qui n'est pas
/// exactement 40 caractères hexadécimaux. Un SHA non exploitable n'est jamais
/// « aucune PR » : c'est [`REASON_DETACHED_HEAD`] (mika#2518 R-2).
///
/// **40 caractères exactement.** `git worktree list --porcelain` rend le SHA
/// complet ; accepter un préfixe ouvrirait un appariement partiel, c'est-à-dire
/// une heuristique — exactement ce que R-2 et R-7 refusent.
///
/// # Aucun repli heuristique derrière cette clé (R-7)
///
/// Si le SHA n'apparie aucune PR, **on conserve**. On ne retombe pas sur une
/// dérivation du chemin du worktree : ce répertoire est produit par
/// `scripts/derive-worktree-path` avec `/`→`-` et translittération
/// (`feat/2425/agent-exposer-le-réglage-…` →
/// `feat-2425-agent-exposer-le-r-glage-…`), donc l'inverse n'est pas une
/// fonction, et re-dériver un chemin de worktree est la duplication que
/// mika-platform#58 a fermée. Surtout, un tel repli se déclencherait très
/// exactement quand la clé exacte dit *« ce worktree n'est pas à un état
/// livré »* — c'est-à-dire quand conserver est la bonne réponse.
///
/// **Refus explicite d'une garde structurelle sur ce point** : un scan de source
/// « aucun site ne dérive une PR depuis un chemin de worktree » serait séduisant,
/// et la classe a **zéro membre** aujourd'hui sans population attendue. Livrer
/// une garde sans population à mesurer est le smell que la maison nomme.
pub fn usable_head_sha(raw: &str) -> Option<String> {
    let sha = raw.trim();
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if sha.bytes().all(|b| b == b'0') {
        return None;
    }
    Some(sha.to_ascii_lowercase())
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
    /// Le commit de tête de la PR — la clé de résolution d'un worktree détaché
    /// (mika#2518).
    ///
    /// # Ici l'asymétrie est INVERSE de celle de `state` / `headRefName` (R-4)
    ///
    /// Ces deux champs n'ont **pas** de `#[serde(default)]`, et c'est porteur :
    /// leur absence ferait *entrer* un worktree dans la population sur une
    /// information manquante. Ce champ-ci est **additif**, et un champ additif ne
    /// doit pas pouvoir éteindre la fonction qu'il enrichit : sans `default`, un
    /// `headRefOid` absent ferait échouer le parsing, donc [`list_prs`] rendrait
    /// `Err`, donc **le dépôt entier serait sauté** (`worktree_reap_failed
    /// stage=pr_list`) — y compris le chemin attaché qui fonctionne aujourd'hui.
    /// D'où `default`, avec la **chaîne vide traitée comme non résolvable** par
    /// [`PrIndex::build`] : la dégradation est bornée au nouveau chemin.
    ///
    /// Le champ est déjà exercé en production sur exactement cet appel
    /// (`server/ci_success_handler.rs`, `"number,headRefOid"`) ; ce que la sonde
    /// V1 du plan établit est sa **survie à la suppression de la branche**.
    #[serde(rename = "headRefOid", default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub url: String,
}

impl PrSnapshot {
    fn is_open(&self) -> bool {
        self.state.eq_ignore_ascii_case("OPEN")
    }
}

/// Les PR d'un dépôt, indexées par les **deux** clés de résolution.
///
/// Un struct plutôt qu'un sixième paramètre à [`screen_worktrees`] : les deux
/// index sont construits de la même liste au même instant, par un **site de
/// construction unique**, et ne peuvent donc pas se désynchroniser.
///
/// Les deux côtés de l'appariement SHA sont mis en minuscules — git et GitHub
/// rendent tous deux du minuscule, la normalisation est défensive et coûte un
/// `to_ascii_lowercase` par PR.
#[derive(Debug, Clone, Default)]
pub struct PrIndex {
    by_branch: HashMap<String, Vec<PrSnapshot>>,
    by_head_sha: HashMap<String, Vec<PrSnapshot>>,
}

impl PrIndex {
    /// Le **seul** site qui construit les deux index.
    pub fn build(prs: Vec<PrSnapshot>) -> Self {
        let mut by_branch: HashMap<String, Vec<PrSnapshot>> = HashMap::new();
        let mut by_head_sha: HashMap<String, Vec<PrSnapshot>> = HashMap::new();
        for pr in prs {
            // Une chaîne vide (champ absent, R-4) ou un SHA non canonique
            // n'indexe **rien** : la PR reste résolvable par sa branche, et le
            // chemin détaché la considère simplement comme inconnue.
            if let Some(sha) = usable_head_sha(&pr.head_ref_oid) {
                by_head_sha.entry(sha).or_default().push(pr.clone());
            }
            by_branch
                .entry(pr.head_ref_name.clone())
                .or_default()
                .push(pr);
        }
        Self {
            by_branch,
            by_head_sha,
        }
    }

    /// Les PR déclarant cette `headRefName`. `None` quand il n'y en a aucune.
    pub fn by_branch(&self, branch: &str) -> Option<&[PrSnapshot]> {
        self.by_branch
            .get(branch)
            .map(Vec::as_slice)
            .filter(|p| !p.is_empty())
    }

    /// Les PR dont le `headRefOid` est ce SHA. `None` quand il n'y en a aucune.
    pub fn by_head_sha(&self, sha: &str) -> Option<&[PrSnapshot]> {
        self.by_head_sha
            .get(&sha.to_ascii_lowercase())
            .map(Vec::as_slice)
            .filter(|p| !p.is_empty())
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
    /// Le nom de branche rapporté.
    ///
    /// Sur le chemin détaché (mika#2518 R-6) il vient du `headRefName` **de la PR
    /// appariée** — une donnée déclarée par GitHub, jamais une inversion de slug
    /// de chemin. Deux conséquences : T7 dispose d'un `origin/<branche>` pour son
    /// second sous-processus, et la surface opérateur de mika#2497 (qui résout son
    /// `pr_number` depuis `ReapRefusal.branch`) cesse d'être aveugle sur cette
    /// population.
    pub branch: String,
    pub pr_number: u64,
    pub pr_state: String,
    pub pr_url: String,
    /// [`RESOLUTION_BRANCH`] ou [`RESOLUTION_DETACHED_SHA`] — par quelle clé ce
    /// worktree a été rattaché à ses PR (mika#2518).
    pub resolution: &'static str,
    /// `Some(sha)` sur le chemin détaché : la clé de jointure, qui rend la
    /// décision rejouable depuis la ligne d'audit. `None` sur le chemin attaché.
    pub head_sha: Option<String>,
}

/// La branche locale doit-elle être supprimée avec le worktree ?
///
/// **Non sur le chemin détaché (mika#2518 R-5).** La branche nommée par la PR n'a
/// jamais été checked out par ce worktree : la supprimer serait un effet de bord
/// sans mandat — et, si elle a déjà disparu, un `git branch -D` qui échoue sans
/// apporter d'information.
///
/// Prédicat nommé plutôt qu'un `if` en ligne dans [`remove_worktree`] : il est
/// alors testable sans toucher au disque, ce que `remove_worktree` ne permet pas.
pub fn should_delete_local_branch(resolution: &str) -> bool {
    resolution == RESOLUTION_BRANCH
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
/// | T2 | **une clé de rattachement est résoluble** — branche attachée, ou SHA du `HEAD` détaché | `git worktree list --porcelain` | conserver |
/// | T3 | au moins une PR connue pour cette clé | `gh pr list --state all` | conserver |
/// | T4 | **aucune** PR ouverte parmi celles-là | idem | conserver |
/// | T5 | la PR la plus récemment close l'est depuis plus que la grâce | `closedAt` | conserver |
/// | T6 | aucun processus vivant n'a son cwd sous le worktree | `/proc/*/cwd` | conserver |
///
/// **T4 est formulé en négatif à dessein.** Deux PR peuvent partager une même
/// `headRefName` (une fermée, une rouverte) — et, depuis mika#2518, un même
/// `headRefOid` (une PR fermée puis rouverte en une nouvelle depuis le même
/// commit). « Il existe une PR mergée » serait vrai dans ces cas et conduirait à
/// supprimer un worktree dont une PR est ouverte. « Aucune PR n'est ouverte » est
/// le prédicat correct, et il s'applique **tel quel** à l'ensemble résolu par
/// SHA : c'est la conséquence directe de R-1 — *la clé change, le prédicat ne
/// change pas.*
///
/// # T2 est une résolution, jamais un refus de principe (mika#2518)
///
/// Avant mika#2518, un `HEAD` détaché sortait de la population. Or à la fermeture
/// d'une PR la branche distante est supprimée, et trois worktrees de PR mergées
/// ont conservé **61 Go** de `target/` sous le motif `detached_head`. La clé de
/// remplacement est l'égalité `HEAD du worktree == headRefOid de la PR`, exacte
/// là où les deux mécanismes énumérés par AC1 sont heuristiques (§ R2 du plan).
///
/// **Et c'est aussi l'argument de sûreté, plus fort que celui du chemin :**
/// apparier exactement le `headRefOid` d'une PR signifie *ce worktree est à
/// l'état livré, et pas un commit de plus*. Un worktree portant du travail non
/// fusionné a un `HEAD` différent et **ne peut pas apparier** — il sort de la
/// population de lui-même, avant même T7.
///
/// **Le chemin attaché est inchangé, y compris sa clé** (R-8) : un worktree
/// attaché continue d'être résolu par sa branche. Le résoudre aussi par SHA
/// serait un risque gratuit sur le chemin nominal ; épinglé par un test
/// d'anti-vacuité, sans lequel « la clé SHA marche » serait indistinguable de
/// « tout est résolu par SHA ».
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
    prs: &PrIndex,
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

        // T2 — résoudre la clé de rattachement, et T3 avec elle : « aucune PR
        // connue pour cette clé » est la même question posée d'une clé
        // différente, et les deux populations restent comptables séparément
        // (`pr_unknown` est fréquent et nominal — « groomé, pas encore
        // implémenté » ; `detached_head_pr_unknown` est la sonde d'attribution de
        // mika#2518).
        let resolved = match entry.branch.as_deref() {
            Some(branch) => match prs.by_branch(branch) {
                Some(matched) => Resolved {
                    prs: matched,
                    branch: branch.to_string(),
                    kind: RESOLUTION_BRANCH,
                    head_sha: None,
                },
                None => {
                    refuse(&mut out, REASON_PR_UNKNOWN);
                    continue;
                }
            },
            None => {
                // R-2 : un SHA inexploitable n'est jamais « aucune PR ».
                let Some(sha) = entry.head.as_deref().and_then(usable_head_sha) else {
                    refuse(&mut out, REASON_DETACHED_HEAD);
                    continue;
                };
                match prs.by_head_sha(&sha) {
                    Some(matched) => Resolved {
                        // R-6 : le nom de branche vient de la PR, jamais d'une
                        // inversion de slug de chemin.
                        branch: matched[0].head_ref_name.clone(),
                        prs: matched,
                        kind: RESOLUTION_DETACHED_SHA,
                        head_sha: Some(sha),
                    },
                    None => {
                        refuse(&mut out, REASON_DETACHED_HEAD_PR_UNKNOWN);
                        continue;
                    }
                }
            }
        };

        // Passé la résolution, le nom de branche rapporté est celui de la clé
        // résolue (R-6) : c'est ce qui rend `pr_open` exploitable par le bras de
        // purge de mika#2497 sur cette population.
        let refuse_resolved = |out: &mut ReapSelection, reason: &'static str| {
            out.refusals.push(ReapRefusal {
                path: entry.path.clone(),
                branch: Some(resolved.branch.clone()),
                reason,
            });
        };
        let prs = resolved.prs;

        // T4 — aucune PR ouverte.
        if prs.iter().any(PrSnapshot::is_open) {
            refuse_resolved(&mut out, REASON_PR_OPEN);
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
            refuse_resolved(&mut out, REASON_PR_CLOSED_AT_UNREADABLE);
            continue;
        }
        let Some(newest_closed) = newest_closed else {
            refuse_resolved(&mut out, REASON_PR_CLOSED_AT_UNREADABLE);
            continue;
        };
        // Une date dans le futur (dérive d'horloge) donne un âge ramené à 0,
        // donc plus jeune que la grâce : conserver, qui est la direction sûre.
        let closed_for = (now - newest_closed).num_seconds().max(0);
        if closed_for < cfg.grace_secs {
            refuse_resolved(&mut out, REASON_TOO_YOUNG);
            continue;
        }

        // T6 — aucun processus vivant dedans.
        match live {
            LiveCwds::Unavailable => {
                refuse_resolved(&mut out, REASON_PROCESS_SCAN_UNREADABLE);
                continue;
            }
            LiveCwds::Enumerated(cwds) => {
                let root = Path::new(&entry.path);
                if cwds.iter().any(|cwd| cwd == root || cwd.starts_with(root)) {
                    refuse_resolved(&mut out, REASON_LIVE_PROCESS);
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
            branch: resolved.branch.clone(),
            pr_number: pr.number,
            pr_state: pr.state.clone(),
            pr_url: pr.url.clone(),
            resolution: resolved.kind,
            head_sha: resolved.head_sha.clone(),
        });
    }

    out
}

/// Ce que T2 a résolu : l'ensemble de PR rattaché, par quelle clé, et sous quel
/// nom de branche le rapporter.
struct Resolved<'a> {
    prs: &'a [PrSnapshot],
    /// Le `headRefName` — de l'entrée du registre sur le chemin attaché, **de la
    /// PR appariée** sur le chemin détaché (R-6).
    branch: String,
    kind: &'static str,
    head_sha: Option<String>,
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
    prs: &PrIndex,
    live: &LiveCwds,
    work_states: &HashMap<String, WorkState>,
    now: DateTime<Utc>,
    cfg: &ReapConfig,
) -> ReapSelection {
    let screened = screen_worktrees(entries, prs, live, now, cfg);
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
///
/// La ligne `HEAD <sha>` est capturée depuis mika#2518 : le porcelain l'émet pour
/// **toute** entrée, détachée comprise, ce qui en fait la clé de rattachement
/// d'un worktree dont la branche a disparu. Mesuré sur cet arbre :
///
/// ```text
/// worktree /data/.../feat-2518-.../mika
/// HEAD c5c4d70f0cebdbfe3e821e65951d53473e8d99d4
/// branch refs/heads/feat/2518/faucheur-un-worktree-de-pr-merg-e-dont
/// ```
pub fn parse_worktree_registry(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut head: Option<String> = None;
    let mut prunable = false;

    let mut flush = |path: &mut Option<String>,
                     branch: &mut Option<String>,
                     head: &mut Option<String>,
                     prunable: &mut bool| {
        if let Some(p) = path.take()
            && !*prunable
        {
            out.push(WorktreeEntry {
                path: p,
                branch: branch.take(),
                head: head.take(),
            });
        }
        *branch = None;
        *head = None;
        *prunable = false;
    };

    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            flush(&mut path, &mut branch, &mut head, &mut prunable);
            path = Some(p.trim().to_string());
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = Some(h.trim().to_string());
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = b.trim().strip_prefix("refs/heads/").map(str::to_string);
        } else if line.trim() == "prunable" || line.starts_with("prunable ") {
            prunable = true;
        }
    }
    flush(&mut path, &mut branch, &mut head, &mut prunable);
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
            "number,state,headRefName,headRefOid,closedAt,url",
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
/// retrait. Elle est **sautée** sur le chemin détaché — voir
/// [`should_delete_local_branch`] (mika#2518 R-5).
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

    let branch_deleted = if should_delete_local_branch(candidate.resolution) {
        run_git(repo_dir, &["branch", "-D", &candidate.branch])
            .await
            .is_some()
    } else {
        false
    };

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
    // mika#2497 — le troisième bras du même tick. Budget, disposition et
    // kill-switch **distincts** de ceux du faucheur : les deux létalités
    // diffèrent d'un ordre de grandeur, et coupler forcerait l'opérateur à
    // régler les deux sur la plus prudente. La sentinelle STOP, elle, est
    // **partagée** — le court-circuit est en tête de tick, en amont d'ici —
    // parce que la décision d'urgence est la même : « arrête ce qui supprime
    // dans les worktrees ».
    let purge_cfg = purge_config_from_env();
    let repo_dirs = parse_repo_dirs(std::env::var(REPO_DIRS_ENV).ok().as_deref());
    let now = Utc::now();

    // Une seule énumération de `/proc` par tick : la population de processus ne
    // change pas d'un dépôt à l'autre, et l'énumérer N fois multiplierait le
    // coût sans rien ajouter.
    let live = collect_live_cwds();

    let mut budget = cfg.max_per_tick;
    let mut purge_budget = purge_cfg.max_per_tick;
    let mut purge_stats = TargetPurgeStats::default();
    let mut disposed = 0usize;
    let mut failed = 0usize;
    let mut refused = 0usize;
    let mut bytes_total: u64 = 0;

    for repo_dir in &repo_dirs {
        if should_stop_repo_loop(budget, purge_budget, purge_cfg.enabled) {
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

        // mika#2449 U3 — la sonde de saleté du checkout principal. Après la
        // garde ci-dessus (un chemin sans dépôt n'est pas un signal), avant le
        // registre. Son résultat n'entre dans aucun compte de ce tick.
        let _ = probe_main_checkout(db, session_id, repo_dir, now, trace_id).await;

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
        let pr_index = PrIndex::build(prs);

        // T1-T6 d'abord : T7 coûte deux `git` par candidat, et ne se paie que
        // sur les survivants.
        let screened = screen_worktrees(&entries, &pr_index, &live, now, &cfg);
        for refusal in &screened.refusals {
            refused += 1;
            record_refusal(db, session_id, refusal, now, trace_id).await;
        }

        // T7, puis **le cap, appliqué après le filtre** (leçon mika#2347).
        //
        // Enveloppé dans `budget > 0` depuis mika#2511 : la boucle des dépôts ne
        // casse plus sur le seul budget du faucheur, donc sans cette garde un
        // budget épuisé ferait payer deux `git` par candidat
        // (`collect_work_state`) pour une boucle de disposition qui casserait
        // aussitôt. Tout ce qui **précède** reste inconditionnel —
        // `probe_main_checkout` (la sonde de saleté mika#2449), le registre, le
        // remote, `list_prs`, `screen_worktrees` et l'écriture de ses refus :
        // `screened.refusals` et `pr_index` sont exactement les deux entrées
        // dont la purge a besoin.
        if budget > 0 {
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

                // mika#2469 : le triplet (event, tool_name, message) vient d'un seul
                // site — en `observe` la ligne dit ce qu'elle *ferait*, jamais
                // « retiré ».
                let outcome = outcome_for(cfg.disposition);
                info!(
                    event = outcome.event,
                    worktree_path = %candidate.path,
                    branch = %candidate.branch,
                    pr_number = candidate.pr_number,
                    pr_state = %candidate.pr_state,
                    pr_url = %candidate.pr_url,
                    // mika#2518 AC4 — la clé de résolution est **séparable** sans
                    // second `tool_name` : `jq 'select(.resolution ==
                    // "detached_sha")'`.
                    resolution = candidate.resolution,
                    head_sha = candidate.head_sha.as_deref(),
                    bytes_reclaimed = size.bytes,
                    bytes_reclaimed_truncated = size.truncated,
                    parent_removed = removal.parent_removed,
                    branch_deleted = removal.branch_deleted,
                    disposition = cfg.disposition.as_str(),
                    trace_id,
                    "{}",
                    outcome.message
                );
                record_reaped(db, session_id, &candidate, &size, cfg.disposition, trace_id).await;
            }
        }

        // mika#2497 — le troisième bras, **après** la disposition du faucheur.
        // L'ordre est nécessaire : ce que le faucheur vient de retirer n'existe
        // plus, et le considérer pour une purge serait au mieux un no-op, au
        // pire une course. `screened.refusals` — et pas `selection.refusals` —
        // est le vecteur qui porte `pr_open` (voir le doc-comment de
        // `purge_stale_target_dirs`).
        purge_stale_target_dirs(
            db,
            session_id,
            trace_id,
            &screened.refusals,
            &pr_index,
            &live,
            now,
            &purge_cfg,
            &mut purge_budget,
            &mut purge_stats,
        )
        .await;
    }

    // mika#2511 B6 — `would_purge` est dans la condition, et c'est la moitié non
    // triviale de la veille (c) : corriger le compteur **seul** rendrait
    // `target_purge_tick` muet dans le mode même que la sonde S0 de mika#2497
    // prescrit d'utiliser en premier, c'est-à-dire une régression
    // d'observabilité introduite par un correctif d'observabilité.
    if purge_stats.purged > 0 || purge_stats.would_purge > 0 || purge_stats.failed > 0 {
        info!(
            event = "target_purge_tick",
            purged = purge_stats.purged,
            would_purge = purge_stats.would_purge,
            failed = purge_stats.failed,
            refused = purge_stats.refused,
            bytes_reclaimed = purge_stats.bytes,
            disposition = purge_cfg.disposition.as_str(),
            idle_secs = purge_cfg.idle_secs,
            trace_id,
            "target_purge: tick agissant"
        );
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

/// Sonde le checkout principal `repo_dir` et écrit, dédupliqué, s'il est sale.
///
/// **Non bloquant, fail-open** : rien de ce qui sort d'ici ne change le
/// verdict de fauche, le compte fauché ni le compte d'échecs du tick (AC7).
/// Rend l'état lu, pour les tests.
///
/// # Le placement est porteur (U3)
///
/// Appelée **après** la garde `repo_dir.join(".git").exists()` du faucheur —
/// et re-vérifiée ici, ceinture et bretelles. En tête de boucle, la sonde
/// sonderait un chemin sans dépôt : en production conteneurisée, chaque
/// checkout configuré produirait une émission « illisible » à chaque tick,
/// pour toujours. « Pas de checkout à ce chemin » et « `git status` illisible
/// sur un checkout réel » sont **deux** états, et seul le second est un signal.
async fn probe_main_checkout(
    db: &AsyncDatabase,
    session_id: &str,
    repo_dir: &Path,
    now: DateTime<Utc>,
    trace_id: &str,
) -> MainCheckoutState {
    if !repo_dir.join(".git").exists() {
        // Hors population par construction : le faucheur a déjà émis
        // `worktree_reap_no_checkout` pour ce chemin.
        return MainCheckoutState::Unreadable;
    }
    let repo_key = repo_dir.display().to_string();

    // Hors délai → `None` → `Unreadable`, jamais « propre ».
    // `--no-optional-locks` : `git status` rafraîchit l'index sous
    // `index.lock` ; sur le checkout de déploiement, un `pull --ff-only` de
    // l'opérateur lancé dans la même fenêtre échouerait « index.lock: File
    // exists » — le symptôme même du ticket, produit par le diagnostic.
    let status: Option<String> = tokio::time::timeout(
        MAIN_CHECKOUT_STATUS_TIMEOUT,
        run_git(repo_dir, &["--no-optional-locks", "status", "--porcelain"]),
    )
    .await
    .unwrap_or_default();
    let state = classify_main_checkout(status.as_deref());

    match &state {
        MainCheckoutState::Clean => {
            if let Ok(mut m) = last_clean_ticks().lock() {
                m.insert(repo_key, now);
            }
        }
        MainCheckoutState::Unreadable => {
            // Signal nommé, jamais « propre ». Une ligne par tick, comme
            // `worktree_reap_no_checkout` : pour un état anormal, la vivacité
            // est l'information.
            warn!(
                event = "main_checkout_unreadable",
                repo_dir = %repo_key,
                trace_id,
                "main_checkout: `git status --porcelain` a échoué ou dépassé {}s — \
                 checkout sorti de la population de la sonde ce tick (mika#2449)",
                MAIN_CHECKOUT_STATUS_TIMEOUT.as_secs()
            );
        }
        MainCheckoutState::Dirty {
            file_count,
            files,
            truncated,
            fingerprint,
        } => {
            let window_start = last_clean_ticks()
                .lock()
                .ok()
                .and_then(|m| m.get(&repo_key).copied())
                .map(|t| crate::timestamp::format(&t));
            record_main_checkout_dirty(
                db,
                session_id,
                &repo_key,
                *file_count,
                files,
                *truncated,
                fingerprint,
                window_start.as_deref(),
                now,
                trace_id,
            )
            .await;
        }
    }
    state
}

/// Écrit la saleté **une fois par (checkout, empreinte) et par 24 h** (D4).
///
/// Un checkout sale le reste des jours ; une ligne par tick (144/j) déplacerait
/// le churn que mika#2131 borne. Un **changement** de liste ré-écrit. L'horizon
/// de 24 h : sans lui, un checkout nettoyé puis re-sali n'écrirait rien la
/// seconde fois. La ligne WARN suit la ligne d'audit — même clé, même
/// déduplication — pour qu'un `grep main_checkout_dirty` du journal et un
/// `SELECT … WHERE tool_name = 'main_checkout_dirty'` comptent la même chose.
#[allow(clippy::too_many_arguments)]
async fn record_main_checkout_dirty(
    db: &AsyncDatabase,
    session_id: &str,
    repo_dir: &str,
    file_count: usize,
    files: &[String],
    truncated: bool,
    fingerprint: &str,
    window_start: Option<&str>,
    now: DateTime<Utc>,
    trace_id: &str,
) {
    let key = main_checkout_audit_key(repo_dir, fingerprint);
    let since = crate::timestamp::format(
        &now.checked_sub_signed(chrono::TimeDelta::seconds(REFUSAL_DEDUP_SECS))
            .unwrap_or(DateTime::<Utc>::MIN_UTC),
    );
    match db
        .count_recent_audit_events_for_target(MAIN_CHECKOUT_DIRTY_TOOL, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(e) => {
            debug!(
                repo_dir,
                error = %e,
                trace_id,
                "main_checkout: relecture du marqueur de saleté impossible, écriture sautée"
            );
            return;
        }
    }

    let window_end = crate::timestamp::format(&now);
    let files_joined = files.join(", ");
    let query = main_checkout_attribution_query(repo_dir, window_start, &window_end);
    let reasoning = format!(
        "files({file_count}{}): {files_joined}\nwindow_start={} window_end={window_end}\nattribution: {query}",
        if truncated {
            format!(", capped at {MAIN_CHECKOUT_DIRTY_MAX_PATHS}")
        } else {
            String::new()
        },
        window_start.unwrap_or("unknown"),
    );

    warn!(
        event = MAIN_CHECKOUT_DIRTY_TOOL,
        repo_dir,
        file_count,
        files = %files_joined,
        truncated,
        fingerprint,
        window_start = window_start.unwrap_or("unknown"),
        window_end = %window_end,
        trace_id,
        "main_checkout: le checkout principal porte des modifications non \
         committées — rien n'est nettoyé ; la requête d'attribution est dans \
         audit_events.reasoning (mika#2449)"
    );
    if let Err(e) = db
        .log_audit_event(
            session_id,
            MAIN_CHECKOUT_DIRTY_TOOL,
            &key,
            None,
            Some(&file_count.to_string()),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            repo_dir,
            error = %e,
            trace_id,
            "main_checkout: audit write failed (dirty)"
        );
    }
}

async fn record_reaped(
    db: &AsyncDatabase,
    session_id: &str,
    candidate: &ReapCandidate,
    size: &SizeMeasurement,
    disposition: Disposition,
    trace_id: &str,
) {
    // mika#2518 — `resolution=` **en tête**, pour qu'un
    // `reasoning LIKE 'resolution=detached_sha%'` soit ancré et exact plutôt
    // qu'une sous-chaîne flottante. `head_sha` rend la décision rejouable : sans
    // lui, une ligne d'audit ne permettrait pas de savoir *quel* commit a été
    // apparié, ce qui est la première question sur un faux positif (HALTE 1).
    let reasoning = format!(
        "resolution={} head_sha={} pr={} state={} url={} branch={} \
         bytes_reclaimed={} truncated={} disposition={}",
        candidate.resolution,
        candidate.head_sha.as_deref().unwrap_or("none"),
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
    // mika#2469 : le nom écrit — et celui que le WARN d'échec nomme — vient
    // du même site ; en `observe` la ligne d'échec ne dit pas « reaped ».
    let outcome = outcome_for(disposition);
    if let Err(e) = db
        .log_audit_event(
            session_id,
            outcome.event,
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
            tool_name = outcome.event,
            error = %e,
            trace_id,
            "worktree_reap: audit write failed ({})",
            outcome.event
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
// mika#2497 — le `target/` d'un worktree **vif** mais inactif
// ---------------------------------------------------------------------------
//
// # La population, et pourquoi elle est disjointe de celle du faucheur
//
// Le faucheur ci-dessus exige « aucune PR ouverte » (T4). Un worktree dont la
// PR est ouverte lui est refusé sous le motif [`REASON_PR_OPEN`], et son
// `target/` vit aussi longtemps que la PR — 15 à 50 Go par pilote. La nuit du
// 2026-09-22 : **+90 Go en 8 h**, `/data` à 83 %, worktrees à 165 Go, nettoyé à
// la main. Cette population est **exactement** l'ensemble des refus `pr_open`
// du même tick : une donnée déjà en mémoire, sans une requête de plus.
//
// # L'asymétrie est INVERSE de celle du faucheur, et c'est ce qui autorise tout
//
// > Le faucheur supprime du **travail potentiel**. Ce bras supprime du
// > **dérivé pur**.
//
// Un `target/` ne porte aucun travail : il est intégralement reconstructible
// par `cargo build`. Le coût d'un faux positif est donc **borné à du temps de
// rebuild**, jamais à une perte — l'exact inverse du *« un faux positif détruit
// des heures de travail, irréversiblement »* qui gouverne le faucheur. C'est
// cette asymétrie qui rend légitime ici un prédicat plus permissif, et qui
// autorise à toucher un worktree **vif**.
//
// **Ce que l'asymétrie n'autorise PAS**, et c'est la vraie contrainte de
// sûreté : supprimer `target/` **pendant** un `cargo build` casse ce build. Le
// danger n'est pas la perte de données, c'est la **concurrence**. Toute la
// conception du prédicat porte là-dessus, et nulle part ailleurs.
//
// # Deux remèdes refusés, avec leur raison
//
// **`CARGO_TARGET_DIR` partagé.** Séduisant — éviter la production plutôt que
// purger — et refusé sur trois motifs. (a) Cargo prend un **verrou exclusif**
// sur son répertoire de build : deux pilotes concurrents se **sérialisent**, ce
// qui couple la boucle entière à un mutex de build au moment même où
// `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT` existe pour la découpler. (b) Un
// target partagé entre branches divergentes **accumule** les artefacts de
// toutes les branches et invalide en cascade ; cargo ne fait aucun GC. (c)
// C'est un changement structurel de la performance de build, **non mesuré**.
// À rouvrir avec une mesure, jamais par intuition (HALTE 3).
//
// **Purge à la fin de chaque dispatch, dans `dispatch-lib.sh`.** Elle détruit
// le cache de build entre l'implement et les itérations QA / CI-fix qui suivent
// **sur le même worktree** : chaque itération repartirait de zéro. On
// échangerait du disque contre de la latence de boucle sur le chemin
// **nominal**, alors que le défaut mesuré est un résidu **nocturne**. Le
// vocabulaire du HALT-2 de mika#2420 dit d'ailleurs *« `cargo clean` sélectif
// sur les worktrees **inactifs** »* — c'est la sélectivité qui fait le remède.
//
// # Ce que ce bras n'achète PAS
//
// **Il borne l'accumulation, il ne borne pas le pic.** Si les N worktrees de la
// nuit du 22 compilaient tous réellement, aucun n'était inactif et la purge
// n'aurait rien attrapé **pendant** la montée — elle attrape le résidu après.
// Il ne mesure pas le disque et n'a aucun seuil de remplissage : il ne sait pas
// que `/data` est à 83 %, il sait qu'un `target/` est inactif.

/// Pas de `<worktree>/target` — rien à purger.
pub const PURGE_REASON_NO_TARGET_DIR: &str = "no_target_dir";
/// `<worktree>/target` existe mais n'est pas un répertoire (fichier, ou **lien
/// symbolique** — un lien vers un arbre voisin ferait supprimer la cible).
pub const PURGE_REASON_TARGET_NOT_A_DIR: &str = "target_not_a_dir";
/// Un processus vivant a son répertoire courant sous le worktree (P3).
pub const PURGE_REASON_LIVE_PROCESS: &str = "live_process";
/// `/proc` n'a pas pu être énuméré **en entier** — P3 est inévaluable.
pub const PURGE_REASON_PROCESS_SCAN_UNREADABLE: &str = "process_scan_unreadable";
/// Le `target/` a été écrit plus récemment que la fenêtre (P4).
///
/// **Doit dominer la distribution** : c'est la fenêtre qui protège le travail
/// en cours (sonde S3).
pub const PURGE_REASON_RECENTLY_ACTIVE: &str = "recently_active";
/// La récence du `target/` n'a pas pu être établie (P4) — `stat` refusé, ou
/// mtime dans le futur (dérive d'horloge). **Doit rester rare**, HALTE 4.
pub const PURGE_REASON_MTIME_UNREADABLE: &str = "mtime_unreadable";
/// Un `cargo` travaille dans ce répertoire : son verrou de build est tenu (P5).
pub const PURGE_REASON_BUILD_LOCK_HELD: &str = "build_lock_held";
/// Le verrou était **libre au filtre amont et tenu à l'acquisition** : un
/// `cargo` a démarré dans la fenêtre que mika#2511 ferme (P5, second étage).
///
/// **Motif distinct de [`PURGE_REASON_BUILD_LOCK_HELD`], et c'est tout son
/// objet** : chaque ligne est une suppression que l'état d'avant mika#2511
/// aurait laissé passer sur un arbre en cours de build. Fusionner les deux
/// populations rendrait cette mesure incomptable ; la clé de dédup
/// [`purge_refusal_audit_key`] porte déjà le motif, donc elles restent
/// soustractibles sans autre changement.
///
/// **Régime attendu : non vide et faible.** Un compte nul ne prouve pas que le
/// défaut n'existait pas — voir la halte S3 du `CLAUDE.md` racine.
pub const PURGE_REASON_BUILD_LOCK_RACED: &str = "build_lock_raced";
/// Le verrou de build n'a pas pu être sondé (P5) — `open` refusé, `flock` en
/// échec sur autre chose que `EWOULDBLOCK`, ou plateforme non-Linux.
///
/// **Doit rester rare**, HALTE 4 : ce motif est le seul par lequel le bras peut
/// devenir silencieusement inerte tout en se lisant comme un disque sain.
pub const PURGE_REASON_BUILD_LOCK_UNREADABLE: &str = "build_lock_unreadable";
/// Chemin hors de `.claude/worktrees/` (P1) — **doit rester vide**, HALTE 4.
pub const PURGE_REASON_OUTSIDE_MANAGED_ROOT: &str = "outside_managed_root";

/// Tous les motifs de refus de la purge, en un seul lieu.
///
/// **Liste délibérément DISTINCTE de [`ALL_REFUSAL_REASONS`]** : deux
/// populations comptables qui doivent rester soustractibles, comme
/// `phantom_aged_out` / `phantom_sweep_spared` (mika#2156). Trois valeurs sont
/// homographes de motifs du faucheur (`live_process`,
/// `process_scan_unreadable`, `outside_managed_root`) — elles atterrissent sous
/// un `tool_name` différent, donc les populations ne se mélangent pas.
///
/// Épinglé par [`tests::mika2497_les_motifs_de_purge_sont_un_format_de_fil`].
///
/// mika#2511 y ajoute [`PURGE_REASON_BUILD_LOCK_RACED`] **en queue** : un ajout,
/// jamais un renommage — aucune population existante ne change de nom ni de
/// sens, et les `GROUP BY` publiés restent exacts.
pub const ALL_PURGE_REFUSAL_REASONS: &[&str] = &[
    PURGE_REASON_NO_TARGET_DIR,
    PURGE_REASON_TARGET_NOT_A_DIR,
    PURGE_REASON_LIVE_PROCESS,
    PURGE_REASON_PROCESS_SCAN_UNREADABLE,
    PURGE_REASON_RECENTLY_ACTIVE,
    PURGE_REASON_MTIME_UNREADABLE,
    PURGE_REASON_BUILD_LOCK_HELD,
    PURGE_REASON_BUILD_LOCK_UNREADABLE,
    PURGE_REASON_OUTSIDE_MANAGED_ROOT,
    PURGE_REASON_BUILD_LOCK_RACED,
];

/// `audit_events.tool_name` (et event tracing) d'une purge **effective**.
///
/// **SOLE WRITER** — ce module est le seul site qui écrit ce nom, épinglé par
/// [`tests::mika2497_le_nom_de_purge_a_un_seul_ecrivain`]. C'est ce qui fait de
/// `SELECT … WHERE tool_name = 'target_purged'` la liste exacte des `target/`
/// que la boucle a purgés. **Réservé à `armed`** : en `observe` la ligne
/// s'écrit sous [`TARGET_PURGE_WOULD_DISPOSE_TOOL`] — correction que mika#2469
/// a dû apporter à son aîné, prise d'emblée ici.
pub const TARGET_PURGED_TOOL: &str = "target_purged";

/// `audit_events.tool_name` (et event tracing) écrit en `observe` **à la place
/// de** [`TARGET_PURGED_TOOL`] : la population qui *serait* purgée.
pub const TARGET_PURGE_WOULD_DISPOSE_TOOL: &str = "target_purge_would_dispose";

/// `audit_events.tool_name` de chaque refus, dédupliqué sur 24 h.
pub const TARGET_PURGE_SKIPPED_TOOL: &str = "target_purge_skipped";

/// Message INFO d'une purge effective (`armed`).
pub const TARGET_PURGED_MESSAGE: &str =
    "target_purge: `target/` d'un worktree vif mais inactif retiré";

/// Message INFO d'un candidat éligible en `observe` : nomme l'éligibilité
/// **et** nie le retrait dans la même phrase.
pub const TARGET_PURGE_WOULD_DISPOSE_MESSAGE: &str =
    "target_purge: `target/` éligible — observe, non purgé";

/// Source unique du triplet (event, tool_name, message) par disposition.
///
/// Même invariant que [`outcome_for`] : les deux surfaces ne peuvent pas
/// diverger sans toucher cette fonction.
pub fn purge_outcome_for(disposition: Disposition) -> Outcome {
    match disposition {
        Disposition::Armed => Outcome {
            event: TARGET_PURGED_TOOL,
            message: TARGET_PURGED_MESSAGE,
        },
        Disposition::Observe => Outcome {
            event: TARGET_PURGE_WOULD_DISPOSE_TOOL,
            message: TARGET_PURGE_WOULD_DISPOSE_MESSAGE,
        },
    }
}

const PURGE_ENABLED_ENV: &str = "MIKA_TARGET_PURGE";
const PURGE_DISPOSITION_ENV: &str = "MIKA_TARGET_PURGE_DISPOSITION";
const PURGE_IDLE_ENV: &str = "MIKA_TARGET_PURGE_IDLE_SECS";
const PURGE_MAX_PER_TICK_ENV: &str = "MIKA_TARGET_PURGE_MAX_PER_TICK";

/// Quatre heures, bornées des deux côtés.
///
/// En dessous : une boucle QA → CI-fix active enchaîne en minutes et se ferait
/// purger son cache entre deux itérations. Au-dessus : la fenêtre nocturne de
/// 8 h qui a produit l'incident cesse d'être mordue. Quatre heures laissent un
/// facteur confortable sur l'une et l'autre borne.
const PURGE_IDLE_DEFAULT_SECS: i64 = 14_400;

/// Budget **distinct** de celui du faucheur : un budget partagé ferait manger
/// au faucheur le sien, ou l'inverse. Un `remove_dir_all` de 40 Go est une
/// tempête d'E/S — ce cap est ce qui l'étale, même raison que chez mika#2420.
const PURGE_MAX_PER_TICK_DEFAULT: usize = 2;

/// Profondeur de la marche de mtime (P4).
///
/// Attrape `target/debug/.fingerprint`, `target/debug/build`,
/// `target/debug/deps` et `target/debug/incremental`, dont les mtimes bougent à
/// chaque recompilation d'unité — c'est-à-dire le signal recherché — **sans**
/// énumérer leur contenu, qui compte des dizaines de milliers de fichiers.
pub const TARGET_MTIME_SCAN_DEPTH: usize = 2;

/// Les bornes de la purge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPurgeConfig {
    /// Kill-switch, **défaut armé**. `0` désarme sans redéploiement.
    pub enabled: bool,
    pub disposition: Disposition,
    pub idle_secs: i64,
    /// Plafond d'écritures par tick. **Lu par l'appelant, jamais par le
    /// prédicat** — leçon mika#2347.
    pub max_per_tick: usize,
}

impl Default for TargetPurgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disposition: Disposition::Armed,
            idle_secs: PURGE_IDLE_DEFAULT_SECS,
            max_per_tick: PURGE_MAX_PER_TICK_DEFAULT,
        }
    }
}

/// Kill-switch : `0`/`false`/`off`/`no` désarment ; absent, vide ou **non
/// reconnu** laissent armé, avec un WARN nommant la valeur entre guillemets.
///
/// Un désarmement par coquille sur un frein de disque serait la panne
/// silencieuse que tout ceci ferme (mika#2205).
pub fn parse_purge_enabled(raw: Option<&str>) -> bool {
    match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") => true,
        Some("0" | "false" | "off" | "no") => false,
        Some("1" | "true" | "on" | "yes") => true,
        Some(other) => {
            warn!(
                value = %format!("{other:?}"),
                "target_purge: valeur non reconnue pour {PURGE_ENABLED_ENV} — la purge reste armée"
            );
            true
        }
    }
}

fn purge_config_from_env() -> TargetPurgeConfig {
    TargetPurgeConfig {
        enabled: parse_purge_enabled(std::env::var(PURGE_ENABLED_ENV).ok().as_deref()),
        disposition: parse_disposition(
            std::env::var(PURGE_DISPOSITION_ENV).ok().as_deref(),
            PURGE_DISPOSITION_ENV,
        ),
        idle_secs: parse_positive_i64(
            std::env::var(PURGE_IDLE_ENV).ok().as_deref(),
            PURGE_IDLE_DEFAULT_SECS,
            PURGE_IDLE_ENV,
        ),
        max_per_tick: parse_positive_usize(
            std::env::var(PURGE_MAX_PER_TICK_ENV).ok().as_deref(),
            PURGE_MAX_PER_TICK_DEFAULT,
            PURGE_MAX_PER_TICK_ENV,
        ),
    }
}

/// Ce que le verrou de build de cargo a pu dire (P5).
///
/// **Trois états, jamais un booléen**, et c'est le point que les mots
/// confondent le plus facilement :
///
/// | ce qui manque | lecture | disposition |
/// |---|---|---|
/// | aucun `.cargo-lock` sous `target/` | cargo n'a jamais construit ici | [`LockProbe::Free`] → purge permise |
/// | l'appel `flock` (non-Linux) | on ne peut pas regarder | [`LockProbe::Unevaluable`] → conserve |
///
/// Traiter la première comme la seconde rend le bras **inerte sur une
/// population saine** tout en se lisant comme un disque en bonne santé (classe
/// mika#2205) ; traiter la seconde comme la première purge à l'aveugle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockProbe {
    /// Aucun verrou tenu — ou aucun verrou du tout, ce qui est la même chose.
    Free,
    /// Au moins un `.cargo-lock` est tenu : un `cargo` travaille ici.
    Held,
    /// On n'a pas pu regarder. **Conserve.**
    Unevaluable,
}

/// Ce que le disque dit d'un `<worktree>/target` candidat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetState {
    /// `<worktree>/target` n'existe pas.
    Absent,
    /// Existe mais n'est pas un répertoire (fichier, ou lien symbolique).
    NotADirectory,
    /// Répertoire. `idle_secs = None` ⇒ la récence n'a **pas** pu être établie
    /// (`stat` refusé, mtime dans le futur, ou état du chemin indéterminable) —
    /// ce qui conserve, jamais l'inverse.
    Present { idle_secs: Option<i64> },
}

/// Un `target/` retenu pour la purge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPurgeCandidate {
    pub worktree_path: String,
    pub target_path: String,
    pub branch: Option<String>,
    pub idle_secs: i64,
}

/// Un `target/` conservé, et le motif nommé qui l'a conservé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPurgeRefusal {
    pub worktree_path: String,
    pub branch: Option<String>,
    pub reason: &'static str,
}

/// La sortie de la décision de purge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetPurgeSelection {
    pub candidates: Vec<TargetPurgeCandidate>,
    pub refusals: Vec<TargetPurgeRefusal>,
}

/// P1 à P4, sur les refus `pr_open` du faucheur du **même tick**.
///
/// | # | terme | source de vérité | illisible ⇒ |
/// |---|---|---|---|
/// | P1 | le chemin est un worktree géré | le chemin lui-même | conserver |
/// | P2 | `<worktree>/target/` existe et est un **répertoire** | `symlink_metadata` | conserver |
/// | P3 | aucun processus vivant n'a son cwd sous le worktree | `/proc/*/cwd` | conserver |
/// | P4 | inactivité : le mtime le plus récent est plus vieux que la fenêtre | `stat`, profondeur bornée | conserver |
///
/// P5 (le verrou de build) est **délibérément absent d'ici** : il coûte un
/// `open` + un `flock` par profil, et ne se paie que sur les survivants de
/// P1-P4 — motif de maison de mika#2184, *le proxy filtre d'abord, la mesure
/// directe tranche ensuite*. Voir [`apply_lock_probes`].
///
/// Invariant : **un terme illisible conserve ; il n'existe aucune exception.**
pub fn screen_target_purges(
    reaper_refusals: &[ReapRefusal],
    live: &LiveCwds,
    states: &HashMap<String, TargetState>,
    cfg: &TargetPurgeConfig,
) -> TargetPurgeSelection {
    let mut out = TargetPurgeSelection::default();

    for refusal in reaper_refusals {
        // La population EST l'ensemble des refus `pr_open` — et rien d'autre.
        // C'est ce qui rend les deux populations disjointes par construction :
        // un worktree retenu par le faucheur n'est pas dans ses refus, et un
        // worktree refusé sous un autre motif relève d'une autre question.
        if refusal.reason != REASON_PR_OPEN {
            continue;
        }

        let push_refusal = |out: &mut TargetPurgeSelection, reason: &'static str| {
            out.refusals.push(TargetPurgeRefusal {
                worktree_path: refusal.path.clone(),
                branch: refusal.branch.clone(),
                reason,
            });
        };

        // P1 — chemin géré (garde syntaxique ; la garde après canonicalisation
        // est re-vérifiée juste avant la disposition).
        if !is_managed_worktree_path(&refusal.path) {
            push_refusal(&mut out, PURGE_REASON_OUTSIDE_MANAGED_ROOT);
            continue;
        }

        // P2 — un `target/` qui est bien un répertoire. Une entrée absente de
        // `states` vaut « on n'a pas pu établir la récence » : conserver.
        let state = states
            .get(&refusal.path)
            .copied()
            .unwrap_or(TargetState::Present { idle_secs: None });
        let idle_secs = match state {
            TargetState::Absent => {
                push_refusal(&mut out, PURGE_REASON_NO_TARGET_DIR);
                continue;
            }
            TargetState::NotADirectory => {
                push_refusal(&mut out, PURGE_REASON_TARGET_NOT_A_DIR);
                continue;
            }
            TargetState::Present { idle_secs } => idle_secs,
        };

        // P3 — aucun processus vivant dedans.
        match live {
            LiveCwds::Unavailable => {
                push_refusal(&mut out, PURGE_REASON_PROCESS_SCAN_UNREADABLE);
                continue;
            }
            LiveCwds::Enumerated(cwds) => {
                let root = Path::new(&refusal.path);
                if cwds.iter().any(|cwd| cwd == root || cwd.starts_with(root)) {
                    push_refusal(&mut out, PURGE_REASON_LIVE_PROCESS);
                    continue;
                }
            }
        }

        // P4 — inactivité.
        let Some(idle_secs) = idle_secs else {
            push_refusal(&mut out, PURGE_REASON_MTIME_UNREADABLE);
            continue;
        };
        if idle_secs < cfg.idle_secs {
            push_refusal(&mut out, PURGE_REASON_RECENTLY_ACTIVE);
            continue;
        }

        out.candidates.push(TargetPurgeCandidate {
            worktree_path: refusal.path.clone(),
            target_path: target_dir_of(&refusal.path),
            branch: refusal.branch.clone(),
            idle_secs,
        });
    }

    out
}

/// P5 — le verrou de build est libre.
///
/// Séparé de [`screen_target_purges`] parce qu'il coûte un `open` + un `flock`
/// par profil, sur le seul candidat retenu — **le dernier point où le refus est
/// encore gratuit**. Une entrée absente de `probes` vaut
/// [`LockProbe::Unevaluable`], donc conserve.
///
/// # Pourquoi P5 existe, et pourquoi il n'est pas de la sur-ingénierie
///
/// P3 est **connu pour être troué**, et mika#2420 l'écrit lui-même : un
/// processus peut travailler dans un worktree sans y avoir son cwd
/// (`cargo --manifest-path`, `git -C`, un éditeur lancé ailleurs). Chez le
/// faucheur ce trou était couvert **par la conjonction** — un tel processus
/// travaille sur une branche dont la PR est ouverte (exclu par T4) ou produit
/// des modifications non committées (exclu par T7).
///
/// **Ici, cette couverture disparaît : la PR est ouverte par définition de la
/// population.** P5 est le terme qui rend ce que T4 apportait au faucheur : il
/// répond à « un cargo travaille-t-il dans ce répertoire », indépendamment du
/// cwd et indépendamment de tout délai.
pub fn apply_lock_probes(
    candidates: Vec<TargetPurgeCandidate>,
    probes: &HashMap<String, LockProbe>,
) -> TargetPurgeSelection {
    let mut out = TargetPurgeSelection::default();
    for candidate in candidates {
        let probe = probes
            .get(&candidate.target_path)
            .copied()
            .unwrap_or(LockProbe::Unevaluable);
        let reason = match probe {
            LockProbe::Free => {
                out.candidates.push(candidate);
                continue;
            }
            LockProbe::Held => PURGE_REASON_BUILD_LOCK_HELD,
            LockProbe::Unevaluable => PURGE_REASON_BUILD_LOCK_UNREADABLE,
        };
        out.refusals.push(TargetPurgeRefusal {
            worktree_path: candidate.worktree_path,
            branch: candidate.branch,
            reason,
        });
    }
    out
}

/// La conjonction complète des cinq termes — la forme que les tests consomment.
///
/// La production passe par [`screen_target_purges`] puis [`apply_lock_probes`]
/// pour ne sonder le verrou que sur les survivants ; les deux chemins rendent
/// la même décision, l'écran étant déterministe.
pub fn select_target_purges(
    reaper_refusals: &[ReapRefusal],
    live: &LiveCwds,
    states: &HashMap<String, TargetState>,
    probes: &HashMap<String, LockProbe>,
    cfg: &TargetPurgeConfig,
) -> TargetPurgeSelection {
    let screened = screen_target_purges(reaper_refusals, live, states, cfg);
    let mut final_pass = apply_lock_probes(screened.candidates, probes);
    let mut refusals = screened.refusals;
    refusals.append(&mut final_pass.refusals);
    TargetPurgeSelection {
        candidates: final_pass.candidates,
        refusals,
    }
}

/// Quand la boucle des dépôts peut cesser (mika#2511, bloquant (b)).
///
/// Les deux bras ont des budgets **distincts** (mika#2497) ; casser sur celui du
/// faucheur seul prive la purge de tous les dépôts suivants — ce qui contredit
/// le « budget distinct » revendiqué — **et lui prend aussi la sonde de saleté
/// mika#2449** : le `break` est en tête du corps de boucle, donc il saute
/// `probe_main_checkout`, la seule chose qui *date* la prochaine occurrence de
/// cette classe. Sans date, la requête d'attribution sur `tool_calls` n'a pas de
/// bornes.
///
/// Le terme de la purge intègre son kill-switch : sans lui, un bras désarmé
/// garderait la boucle vivante pour rien — B4.
///
/// Prédicat pur nommé plutôt qu'une conjonction en ligne : il est testable à ses
/// quatre coins sans monter de dépôt factice, et il est l'endroit où le
/// raisonnement est écrit.
pub fn should_stop_repo_loop(
    reaper_budget: usize,
    purge_budget: usize,
    purge_enabled: bool,
) -> bool {
    reaper_budget == 0 && (purge_budget == 0 || !purge_enabled)
}

/// `<worktree>/target`, en chaîne — un seul site le compose.
pub fn target_dir_of(worktree_path: &str) -> String {
    Path::new(worktree_path)
        .join("target")
        .to_string_lossy()
        .into_owned()
}

/// Le mtime le plus récent sur un ensemble **borné et déclaré** : `root`, ses
/// enfants directs, et les enfants de ceux-ci (`depth = 2`).
///
/// [`measure_tree_size`] est budgété à 400 000 entrées et 2 s — un `target/` de
/// 40 Go les dépasse, et une marche tronquée rendrait un mtime **faux dans la
/// direction dangereuse** (sous-estimer la récence, donc purger un arbre
/// actif). D'où une marche de profondeur fixe : quelques centaines de `stat`,
/// coût constant, et aucune troncature possible.
///
/// **Toute** lecture impossible rend `None` — un sous-arbre sauté
/// sous-estimerait la récence, ce qui est précisément la direction interdite.
/// `symlink_metadata` et non `metadata` : un lien symbolique cassé ne doit pas
/// faire échouer la marche, et un lien vers un arbre voisin ne doit pas
/// importer sa récence.
pub fn newest_mtime_bounded(root: &Path, depth: usize) -> Option<SystemTime> {
    let mut newest = root.symlink_metadata().ok()?.modified().ok()?;
    let mut level = vec![root.to_path_buf()];

    for _ in 0..depth {
        let mut next = Vec::new();
        for dir in &level {
            for entry in std::fs::read_dir(dir).ok()? {
                let entry = entry.ok()?;
                let meta = entry.path().symlink_metadata().ok()?;
                let modified = meta.modified().ok()?;
                if modified > newest {
                    newest = modified;
                }
                if meta.is_dir() {
                    next.push(entry.path());
                }
            }
        }
        level = next;
        if level.is_empty() {
            break;
        }
    }

    Some(newest)
}

/// P2 + P4, en une seule lecture du disque.
///
/// Un mtime **dans le futur** (dérive d'horloge) rend `idle_secs = None`,
/// exactement comme un `stat` refusé : les deux sortent le worktree de la
/// population, aucun ne l'y fait entrer.
pub fn inspect_target_dir(worktree: &Path, now: SystemTime) -> TargetState {
    let target = worktree.join("target");
    let meta = match target.symlink_metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return TargetState::Absent,
        // On ne sait pas s'il est là : conserver, jamais « absent ».
        Err(_) => return TargetState::Present { idle_secs: None },
    };
    if meta.is_symlink() || !meta.is_dir() {
        return TargetState::NotADirectory;
    }
    let idle_secs = newest_mtime_bounded(&target, TARGET_MTIME_SCAN_DEPTH)
        .and_then(|m| now.duration_since(m).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    TargetState::Present { idle_secs }
}

/// Les `.cargo-lock` d'un `target/`, **découverts et jamais devinés**.
///
/// Un seul énumérateur, consommé par les **deux** étages de P5 : le filtre
/// ([`cargo_build_lock_is_free`]) et l'acquisition tenue
/// ([`acquire_cargo_build_locks`]). Deux énumérations pourraient diverger — le
/// filtre verrait un profil que l'acquisition ne verrouille pas, c'est-à-dire
/// le défaut que mika#2511 ferme, reproduit un cran plus bas.
///
/// `Err(())` quand l'énumération elle-même a échoué : la population est alors
/// **inconnue, jamais vide**. Le `bool` dit qu'au moins une entrée n'a pas pu
/// être inspectée — le terme est alors inévaluable même si les entrées lues
/// sont libres.
#[cfg(target_os = "linux")]
#[allow(clippy::result_unit_err)]
fn cargo_lock_paths(target: &Path) -> Result<(Vec<PathBuf>, bool), ()> {
    let Ok(read) = std::fs::read_dir(target) else {
        return Err(());
    };
    let mut paths = Vec::new();
    let mut partial = false;
    for entry in read {
        let Ok(entry) = entry else {
            partial = true;
            continue;
        };
        let path = entry.path();
        let Ok(meta) = path.symlink_metadata() else {
            partial = true;
            continue;
        };
        if meta.is_symlink() || !meta.is_dir() {
            continue;
        }
        let lock = path.join(".cargo-lock");
        if !lock.is_file() {
            continue;
        }
        paths.push(lock);
    }
    Ok((paths, partial))
}

/// Les verrous de build **tenus**, relâchés au `Drop` (mika#2511).
///
/// `flock` est relâché par la fermeture du descripteur ; garder les `File`
/// vivants **est** la totalité du mécanisme. L'`impl Drop` explicite est là
/// malgré cela, pour la raison que [`probe_one_cargo_lock`] donnait déjà à son
/// propre site : la fermeture le relâcherait de toute façon, le dire rend
/// l'intention lisible.
///
/// # Ce que tenir le verrou protège, et ce qu'il ne protège pas
///
/// `flock(2)` porte sur une *open file description*, donc sur l'inode. Le
/// `remove_dir_all` supprime `<target>/<profil>/.cargo-lock` en cours de route :
/// une fois cet unlink passé, un `cargo` qui démarre **crée un nouvel inode** au
/// même chemin et prend un verrou dessus sans jamais rencontrer le nôtre.
///
/// | fenêtre | avant mika#2511 | après |
/// |---|---|---|
/// | sonde → début de la suppression | **non protégée** | protégée |
/// | début de la suppression → unlink du `.cargo-lock` | non protégée | protégée |
/// | unlink → fin de la suppression | non protégée | **toujours non protégée** |
///
/// Le résidu est réel et acceptable pour la raison que mika#2497 a écrite comme
/// fondement de tout le bras : *un faux positif coûte du temps de rebuild,
/// jamais une perte* — et un `cargo` qui démarre dans la seconde moitié d'un
/// `remove_dir_all` est un build de quelques secondes.
///
/// Un `rename(target, target.mika-purge-<n>)` ramènerait cette fenêtre à
/// quelques microsecondes et est **refusé** : le répertoire renommé n'est couvert
/// par aucun `.gitignore`, donc `git status --porcelain` le liste `??`, donc T7
/// du faucheur mika#2420 lit le worktree `dirty` et refuse de le retirer — un
/// orphelin de 40 Go dans un worktree devenu non-retirable, soit le problème que
/// ce bras existe pour résoudre, aggravé.
#[must_use = "relâcher le garde avant la suppression rouvre la fenêtre mika#2511"]
pub struct CargoBuildLockGuard {
    #[cfg(target_os = "linux")]
    held: Vec<std::fs::File>,
}

impl std::fmt::Debug for CargoBuildLockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(target_os = "linux")]
        let n = self.held.len();
        #[cfg(not(target_os = "linux"))]
        let n = 0usize;
        f.debug_struct("CargoBuildLockGuard")
            .field("held", &n)
            .finish()
    }
}

#[cfg(target_os = "linux")]
impl Drop for CargoBuildLockGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        for file in &self.held {
            // SAFETY: descripteur valide que nous possédons, encore ouvert.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

/// Ce qu'une tentative d'acquisition a pu dire (mika#2511).
///
/// Miroir de [`LockProbe`] côté second étage, avec la même règle de maison :
/// **un signal qu'on ne peut pas lire n'est jamais un terme satisfait.**
#[derive(Debug)]
#[must_use = "une acquisition ignorée relâche ses verrous immédiatement"]
pub enum LockAcquisition {
    /// Tous les verrous sont à nous, et le restent tant que le garde vit.
    Acquired(CargoBuildLockGuard),
    /// Au moins un verrou est tenu par un `cargo`.
    Held,
    /// On n'a pas pu regarder. **Conserve.**
    Unevaluable,
}

/// P5, second étage : **acquérir et retenir** les verrous de build de `target/`.
///
/// Appelée juste avant la disposition, elle ferme la fenêtre TOCTOU que le
/// filtre amont laisse ouverte : entre la sonde et le `remove_dir_all` il y a
/// des points `.await` et une mesure d'arbre, et avec un cap de deux candidats
/// par tick le **second** a devant lui la suppression complète du premier —
/// des dizaines de secondes sur 40 Go.
///
/// Trois propriétés :
///
/// 1. **les `File` sont retenus**, donc les verrous aussi, jusqu'au `Drop` ;
/// 2. **un seul verrou tenu annule toute l'acquisition**, et les descripteurs
///    déjà acquis sont relâchés par le `drop` du `Vec` partiel — pas de verrou
///    orphelin ;
/// 3. **`Unevaluable` conserve**, comme partout ailleurs dans ce bras. Hors
///    Linux l'acquisition rend `Unevaluable`, donc la purge n'y fire jamais —
///    ce qui est déjà le cas aujourd'hui pour le filtre.
pub fn acquire_cargo_build_locks(target: &Path) -> LockAcquisition {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;

        let Ok((locks, partial)) = cargo_lock_paths(target) else {
            return LockAcquisition::Unevaluable;
        };
        if partial {
            // La population des profils est incomplète : on ne peut pas tenir
            // ce qu'on n'a pas su énumérer.
            return LockAcquisition::Unevaluable;
        }

        let mut held: Vec<std::fs::File> = Vec::with_capacity(locks.len());
        for lock in &locks {
            let file = match std::fs::OpenOptions::new().read(true).open(lock) {
                Ok(f) => f,
                // Disparu entre l'énumération et l'ouverture : personne ne le
                // tient, et il n'y a rien à retenir.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                // `held` est relâché par son `Drop` en sortant.
                Err(_) => return LockAcquisition::Unevaluable,
            };
            // SAFETY: `file` est un descripteur valide que nous possédons, et
            // `LOCK_NB` garantit que l'appel ne bloque jamais.
            let acquired = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if acquired == 0 {
                held.push(file);
                continue;
            }
            return match std::io::Error::last_os_error().raw_os_error() {
                // `EWOULDBLOCK == EAGAIN` sous Linux : un `cargo` travaille ici.
                Some(libc::EWOULDBLOCK) => LockAcquisition::Held,
                _ => LockAcquisition::Unevaluable,
            };
        }
        LockAcquisition::Acquired(CargoBuildLockGuard { held })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        LockAcquisition::Unevaluable
    }
}

/// Sonde un `.cargo-lock` précis. Linux seulement.
#[cfg(target_os = "linux")]
fn probe_one_cargo_lock(path: &Path) -> LockProbe {
    use std::os::fd::AsRawFd;

    let file = match std::fs::OpenOptions::new().read(true).open(path) {
        Ok(f) => f,
        // Disparu entre l'énumération et l'ouverture : personne ne le tient.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LockProbe::Free,
        Err(_) => return LockProbe::Unevaluable,
    };

    // SAFETY: `file` est un descripteur valide que nous possédons, et `LOCK_NB`
    // garantit que l'appel ne bloque jamais — jamais une attente dans un tick.
    let acquired = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if acquired == 0 {
        // Relâché immédiatement. La fermeture du descripteur le relâcherait de
        // toute façon ; le dire explicitement rend l'intention lisible.
        // SAFETY: même descripteur, toujours valide.
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        return LockProbe::Free;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        // `EWOULDBLOCK == EAGAIN` sous Linux : quelqu'un tient le verrou.
        Some(libc::EWOULDBLOCK) => LockProbe::Held,
        _ => LockProbe::Unevaluable,
    }
}

/// P5 — le verrou de build de cargo est-il libre ?
///
/// **Le fichier est découvert, jamais deviné.** Cargo pose son verrou sur
/// `<target>/<profil>/.cargo-lock`, et le profil est une donnée de l'invocation
/// (`debug`, `release`, un profil nommé). Ce terme énumère donc les enfants
/// directs de `target/` et sonde chaque `.cargo-lock` trouvé ; **un seul verrou
/// tenu suffit à refuser**. Deviner `target/debug/.cargo-lock` raterait un
/// build `--release`, c'est-à-dire échouerait exactement sur le cas qu'on veut
/// voir.
///
/// **L'absence de tout `.cargo-lock` satisfait le terme** — elle ne le rend pas
/// [`LockProbe::Unevaluable`]. Cargo ne retire pas ce fichier après un build :
/// son absence dit que cargo n'a jamais construit ici, pas qu'on n'a pas pu
/// regarder.
///
/// # Le repli non-Linux **conserve**, et son sens n'est pas libre
///
/// `libc` vit sous `[target.'cfg(target_os = "linux")'.dependencies]`, donc cet
/// appel suit le patron de `task_engine::process_liveness::is_same_process_alive`.
/// Hors Linux le verrou est **inévaluable**, donc il conserve, et la purge n'y
/// fire jamais. Rendre `Free` y serait purger sur la seule plateforme où l'on ne
/// peut pas vérifier qu'un build tourne. La cible de production est Linux
/// (`Dockerfile.agent`, OpenRC) : le coût est nul, et ce qui est en jeu est
/// `cargo build` sur le poste d'un développeur plus la release cross-plateforme.
pub fn cargo_build_lock_is_free(target: &Path) -> LockProbe {
    #[cfg(target_os = "linux")]
    {
        // Même énumérateur que [`acquire_cargo_build_locks`] (mika#2511) : un
        // profil que le filtre voit et que l'acquisition ne verrouille pas
        // serait un trou silencieux.
        let Ok((locks, partial)) = cargo_lock_paths(target) else {
            return LockProbe::Unevaluable;
        };
        let mut unevaluable = partial;
        for lock in &locks {
            match probe_one_cargo_lock(lock) {
                // Un seul verrou tenu suffit, et il l'emporte sur un
                // inévaluable : c'est l'information la plus spécifique.
                LockProbe::Held => return LockProbe::Held,
                LockProbe::Unevaluable => unevaluable = true,
                LockProbe::Free => {}
            }
        }
        if unevaluable {
            LockProbe::Unevaluable
        } else {
            LockProbe::Free
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        LockProbe::Unevaluable
    }
}

/// Les trois gardes de la suppression, vérifiées **dans cet ordre**.
///
/// 1. ce n'est pas un lien symbolique — testé sur le chemin **d'origine**, la
///    canonicalisation le résoudrait et la question deviendrait muette ;
/// 2. le chemin canonicalisé se termine par `/target` ;
/// 3. il est sous [`MANAGED_WORKTREE_SEGMENT`] après canonicalisation.
///
/// `canonicalize` qui échoue rend `false` : pas de preuve, pas de suppression.
pub fn target_path_is_disposable(target: &Path) -> bool {
    if target.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
        return false;
    }
    let Ok(canonical) = std::fs::canonicalize(target) else {
        return false;
    };
    let Some(canonical) = canonical.to_str() else {
        return false;
    };
    canonical.ends_with("/target") && is_managed_worktree_path(canonical)
}

/// Ce qu'un tick a fait côté purge.
///
/// `purged` et `would_purge` sont **deux compteurs, jamais un seul** (mika#2511,
/// veille (c) de mika#2469) : incrémenter `purged` en `observe` ferait dire à
/// l'agrégat `target_purge_tick` qu'un dry-run a supprimé quelque chose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TargetPurgeStats {
    /// Suppressions **effectives** (`armed` seulement).
    purged: usize,
    /// Candidats éligibles non retirés (`observe` seulement).
    would_purge: usize,
    failed: usize,
    refused: usize,
    bytes: u64,
}

/// Le bras de purge, greffé **dans** la boucle des dépôts du faucheur, **après**
/// sa boucle de disposition.
///
/// Trois choses doivent être vivantes ensemble à ce point, et c'est ce qui fixe
/// le site de branchement :
///
/// - `screened.refusals` — **et pas `selection.refusals`** : T4 pousse
///   [`REASON_PR_OPEN`] dans [`screen_worktrees`], tandis que les refus de
///   [`apply_work_states`] ne portent que `dirty` / `unpushed_commits`. Filtrer
///   le mauvais vecteur rendrait une population vide, c'est-à-dire un bras qui
///   se lit comme sain en ne faisant rien (classe mika#2205).
/// - l'index des PR — nécessaire au `pr_number` de la surface opérateur. Depuis
///   mika#2518 il porte aussi la population détachée : un worktree détaché dont
///   la PR est **ouverte** était refusé `detached_head`, donc son `target/`
///   n'était purgé **ni** par le faucheur **ni** par ce bras ; il est maintenant
///   refusé `pr_open` et devient purgeable. Élargissement voulu et **gratuit en
///   sûreté** — les cinq termes P1–P5 s'appliquent inchangés, verrou de build
///   compris — et, grâce à R-6, la ligne porte désormais son `pr_number` au lieu
///   d'un trou.
/// - le budget — **celui de la purge**, distinct de celui du faucheur.
#[allow(clippy::too_many_arguments)]
async fn purge_stale_target_dirs(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    reaper_refusals: &[ReapRefusal],
    prs: &PrIndex,
    live: &LiveCwds,
    now: DateTime<Utc>,
    cfg: &TargetPurgeConfig,
    budget: &mut usize,
    stats: &mut TargetPurgeStats,
) {
    if !cfg.enabled || *budget == 0 {
        return;
    }

    // P2 + P4, une lecture de disque par worktree de la population.
    let system_now = SystemTime::now();
    let mut states: HashMap<String, TargetState> = HashMap::new();
    for refusal in reaper_refusals {
        if refusal.reason != REASON_PR_OPEN || !is_managed_worktree_path(&refusal.path) {
            continue;
        }
        states.insert(
            refusal.path.clone(),
            inspect_target_dir(Path::new(&refusal.path), system_now),
        );
    }

    let screened = screen_target_purges(reaper_refusals, live, &states, cfg);
    for refusal in &screened.refusals {
        stats.refused += 1;
        record_purge_refusal(db, session_id, refusal, now, trace_id).await;
    }

    // P5 : sondé seulement sur les survivants de P1-P4.
    let mut probes: HashMap<String, LockProbe> = HashMap::new();
    for candidate in &screened.candidates {
        probes.insert(
            candidate.target_path.clone(),
            cargo_build_lock_is_free(Path::new(&candidate.target_path)),
        );
    }
    let selection = apply_lock_probes(screened.candidates, &probes);
    for refusal in &selection.refusals {
        stats.refused += 1;
        record_purge_refusal(db, session_id, refusal, now, trace_id).await;
    }

    for candidate in selection.candidates {
        if *budget == 0 {
            break;
        }
        let target = Path::new(&candidate.target_path);

        // Les deux gardes tardives, dans cet ordre. La première était déjà là :
        // la sûreté du chemin est re-vérifiée après canonicalisation. La
        // seconde est mika#2511 — **l'acquisition est le dernier acte avant la
        // suppression**, et il n'y a entre elles ni `.await`, ni appel réseau,
        // ni opération non bornée. C'est la propriété que le scan structurel
        // `mika2511_toute_suppression_est_precedee_de_lacquisition` tient.
        //
        // L'acquisition a lieu dans les **deux** dispositions, `observe`
        // comprise : sans cela `observe` rendrait une population plus large que
        // ce que `armed` retirerait, et la sonde S0 de mika#2497 — « commencer
        // en observe et lire la population qui serait retirée » — mentirait sur
        // son propre objet.
        let acquisition = if target_path_is_disposable(target) {
            match acquire_cargo_build_locks(target) {
                LockAcquisition::Acquired(guard) => Ok(guard),
                LockAcquisition::Held => Err(PURGE_REASON_BUILD_LOCK_RACED),
                LockAcquisition::Unevaluable => Err(PURGE_REASON_BUILD_LOCK_UNREADABLE),
            }
        } else {
            Err(PURGE_REASON_OUTSIDE_MANAGED_ROOT)
        };
        let guard = match acquisition {
            Ok(guard) => guard,
            Err(reason) => {
                stats.refused += 1;
                record_purge_refusal(
                    db,
                    session_id,
                    &TargetPurgeRefusal {
                        worktree_path: candidate.worktree_path.clone(),
                        branch: candidate.branch.clone(),
                        reason,
                    },
                    now,
                    trace_id,
                )
                .await;
                continue;
            }
        };

        // Le débit suit l'acquisition, et non l'inverse : le cap est un
        // **plafond d'écritures par tick** (leçon mika#2347, *un cap sur les
        // écritures, jamais sur les sauts*), et un refus tardif n'écrit rien.
        // Conséquence assumée : un tick où deux candidats sont refusés à
        // l'acquisition peut en tenter un troisième — l'acquisition est bon
        // marché, et le cap borne les tempêtes d'E/S, qui viennent des
        // suppressions.
        *budget -= 1;
        // Sous verrou : la mesure porte sur un arbre qu'aucun `cargo` ne peut
        // plus étendre, et ses 2 s de budget sortent de la fenêtre non protégée
        // au lieu d'y entrer.
        let size = measure_tree_size(target);

        let removed = match cfg.disposition {
            Disposition::Observe => true,
            Disposition::Armed => std::fs::remove_dir_all(target).is_ok(),
        };
        // Explicite, après la suppression : le `Drop` le ferait en fin
        // d'itération, le dire ici nomme la borne de la protection.
        drop(guard);

        if cfg.disposition == Disposition::Armed && !removed {
            stats.failed += 1;
            warn!(
                event = "target_purge_failed",
                worktree_path = %candidate.worktree_path,
                target_path = %candidate.target_path,
                trace_id,
                "target_purge: `remove_dir_all` a échoué"
            );
            continue;
        }

        // La dérivation suit la disposition, au même endroit que
        // [`purge_outcome_for`] — source unique du triplet de surfaces.
        match cfg.disposition {
            Disposition::Armed => stats.purged += 1,
            Disposition::Observe => stats.would_purge += 1,
        }
        if let Some(b) = size.bytes {
            stats.bytes = stats.bytes.saturating_add(b);
        }

        // `pr_number` est **résolu, pas porté** : `ReapRefusal` ne transporte
        // que le chemin, la branche et le motif. Un numéro illisible **dégrade
        // la ligne, ne suspend pas la purge** (modèle `repo=unknown`,
        // mika#2496) — un worktree purgé dont on ne sait pas nommer la PR reste
        // un worktree purgé, et le taire rétrécirait le compte en silence.
        let pr_number = candidate
            .branch
            .as_deref()
            .and_then(|b| prs.by_branch(b))
            .and_then(|matched| matched.iter().find(|p| p.is_open()))
            .map(|p| p.number);

        let outcome = purge_outcome_for(cfg.disposition);
        info!(
            event = outcome.event,
            worktree_path = %candidate.worktree_path,
            target_path = %candidate.target_path,
            branch = candidate.branch.as_deref().unwrap_or("(detached)"),
            pr_number,
            idle_secs = candidate.idle_secs,
            bytes_reclaimed = size.bytes,
            bytes_reclaimed_truncated = size.truncated,
            disposition = cfg.disposition.as_str(),
            trace_id,
            "{}",
            outcome.message
        );
        record_purged(
            db,
            session_id,
            &candidate,
            pr_number,
            &size,
            cfg.disposition,
            trace_id,
        )
        .await;
    }
}

/// Clé d'audit d'une purge : `target:<chemin du target>`.
pub fn purged_audit_key(target_path: &str) -> String {
    format!("target:{target_path}")
}

/// Clé d'audit d'un refus : `target:<chemin du worktree>@<motif>`.
///
/// Le motif est **dans la clé** pour que la déduplication soit par
/// `(worktree, motif)` : un worktree qui **change** de motif réécrit, parce que
/// c'est un changement d'état (doctrine mika#2131).
pub fn purge_refusal_audit_key(worktree_path: &str, reason: &str) -> String {
    format!("target:{worktree_path}@{reason}")
}

async fn record_purged(
    db: &AsyncDatabase,
    session_id: &str,
    candidate: &TargetPurgeCandidate,
    pr_number: Option<u64>,
    size: &SizeMeasurement,
    disposition: Disposition,
    trace_id: &str,
) {
    let reasoning = format!(
        "pr={} branch={} idle_secs={} bytes_reclaimed={} truncated={} disposition={}",
        pr_number
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".to_string()),
        candidate.branch.as_deref().unwrap_or("(detached)"),
        candidate.idle_secs,
        size.bytes
            .map(|b| b.to_string())
            .unwrap_or_else(|| "null".to_string()),
        size.truncated,
        disposition.as_str(),
    );
    let outcome = purge_outcome_for(disposition);
    if let Err(e) = db
        .log_audit_event(
            session_id,
            outcome.event,
            &purged_audit_key(&candidate.target_path),
            None,
            size.bytes.map(|b| b.to_string()).as_deref(),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            target_path = %candidate.target_path,
            tool_name = outcome.event,
            error = %e,
            trace_id,
            "target_purge: audit write failed ({})",
            outcome.event
        );
    }
}

/// Écrit le refus **une fois par (worktree, motif) et par 24 h**.
///
/// Un `target/` tenu par `recently_active` produirait sinon une ligne toutes
/// les dix minutes — le churn que mika#2131 borne. La marque n'est posée
/// qu'après une écriture réussie ; une lecture impossible saute l'écriture
/// plutôt que de la dupliquer.
async fn record_purge_refusal(
    db: &AsyncDatabase,
    session_id: &str,
    refusal: &TargetPurgeRefusal,
    now: DateTime<Utc>,
    trace_id: &str,
) {
    let key = purge_refusal_audit_key(&refusal.worktree_path, refusal.reason);
    let since = crate::timestamp::format(
        &now.checked_sub_signed(chrono::TimeDelta::seconds(REFUSAL_DEDUP_SECS))
            .unwrap_or(DateTime::<Utc>::MIN_UTC),
    );
    match db
        .count_recent_audit_events_for_target(TARGET_PURGE_SKIPPED_TOOL, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => return,
        Err(e) => {
            debug!(
                worktree_path = %refusal.worktree_path,
                error = %e,
                trace_id,
                "target_purge: relecture du marqueur de refus impossible, écriture sautée"
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
            TARGET_PURGE_SKIPPED_TOOL,
            &key,
            None,
            Some(refusal.reason),
            Some(&reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(
            worktree_path = %refusal.worktree_path,
            error = %e,
            trace_id,
            "target_purge: audit write failed (skipped)"
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

    /// Un SHA de 40 caractères hexadécimaux, déterministe et lisible dans un
    /// message d'échec. **Jamais `0`** : `usable_head_sha` refuse le SHA nul, ce
    /// qui est exactement la propriété que N3 mesure.
    fn sha(seed: u64) -> String {
        assert_ne!(seed, 0, "0 rendrait le SHA nul, que la clé refuse");
        format!("{seed:040x}")
    }

    /// Une entrée **attachée** sans ligne `HEAD` — la forme des fixtures de
    /// mika#2420, conservée telle quelle pour que V3d atteste la non-régression
    /// du chemin attaché sans toucher à ses appels.
    fn entry(path: &str, branch: Option<&str>) -> WorktreeEntry {
        WorktreeEntry {
            path: path.to_string(),
            branch: branch.map(str::to_string),
            head: None,
        }
    }

    /// Une entrée **détachée** : pas de branche, un `HEAD` brut.
    fn detached(path: &str, head: &str) -> WorktreeEntry {
        WorktreeEntry {
            path: path.to_string(),
            branch: None,
            head: Some(head.to_string()),
        }
    }

    /// Une entrée **attachée** portant aussi son `HEAD` — la forme réelle du
    /// porcelain, nécessaire à l'anti-vacuité V3c.
    fn attached_with_head(path: &str, branch: &str, head: &str) -> WorktreeEntry {
        WorktreeEntry {
            path: path.to_string(),
            branch: Some(branch.to_string()),
            head: Some(head.to_string()),
        }
    }

    fn merged_pr(number: u64, branch: &str, closed_secs: i64) -> PrSnapshot {
        PrSnapshot {
            number,
            state: "MERGED".to_string(),
            head_ref_name: branch.to_string(),
            closed_at: Some(closed_secs_ago(closed_secs)),
            head_ref_oid: sha(number),
            url: format!("https://github.com/senara-solutions/mika/pull/{number}"),
        }
    }

    fn index(prs: Vec<PrSnapshot>) -> PrIndex {
        PrIndex::build(prs)
    }

    fn clean(path: &str) -> HashMap<String, WorkState> {
        HashMap::from([(path.to_string(), WorkState::Clean)])
    }

    fn select(
        entries: &[WorktreeEntry],
        prs: &PrIndex,
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
            PrIndex,
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
                PrIndex::default(),
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

    // -- mika#2518 : un HEAD détaché n'est pas un worktree sans PR -----------

    /// Le worktree détaché du défaut fondateur, reconstitué : `HEAD` = le
    /// `headRefOid` d'une PR **mergée** close depuis plus que la grâce, arbre
    /// propre, aucun processus dedans.
    const DETACHED_WT: &str =
        "/data/workspace/mika-platform/.claude/worktrees/feat-2425-agent-exposer/mika";

    /// **V3a — contrôle positif.** C'est AC1 : un worktree en HEAD détaché dont
    /// la PR est MERGED est **fauchable**, et il porte les trois champs qui
    /// rendent la décision rejouable.
    #[test]
    fn mika2518_v3a_un_detache_de_pr_mergee_est_fauchable() {
        let head = sha(2489);
        let s = select(
            &[detached(DETACHED_WT, &head)],
            &index(vec![merged_pr(2489, "feat/2425/agent-exposer", 7200)]),
            &no_processes(),
            &clean(DETACHED_WT),
        );
        assert_eq!(
            s.candidates.len(),
            1,
            "le détaché de PR mergée doit être candidat ; refus: {:?}",
            only_reason(&s)
        );
        let c = &s.candidates[0];
        assert_eq!(c.resolution, RESOLUTION_DETACHED_SHA);
        assert_eq!(c.head_sha.as_deref(), Some(head.as_str()));
        assert_eq!(c.pr_number, 2489);
        assert_eq!(
            c.branch, "feat/2425/agent-exposer",
            "R-6 — le nom de branche vient du `headRefName` de la PR, jamais \
             d'une inversion de slug de chemin"
        );
    }

    /// **V3b — les sept contrôles négatifs, un terme neutralisé à la fois.**
    ///
    /// Sans eux, V3a passerait aussi contre un prédicat qui fauche tout worktree
    /// détaché — ce qui est très exactement ce qu'AC2 interdit (*« le HEAD détaché
    /// seul ne suffit pas à faucher »*). Chaque cas doit **conserver**, et sous
    /// **son propre motif** : un motif voisin signifierait que le terme a été
    /// absorbé en aval, donc que le contrôle ne prouve plus rien (leçon V2 de
    /// mika#2420).
    ///
    /// # Les échecs sont ACCUMULÉS, et ce n'est pas du confort
    ///
    /// Un `assert!` par cas avorte au premier, ce qui masque les six autres —
    /// donc une mutation qui casse la conjonction entière (le chemin détaché
    /// poussant son candidat sans traverser T4–T7) se lirait comme *un* contrôle
    /// rouge au lieu de quatre. L'accumulation est ce qui rend la preuve de
    /// mutation complète : le message nomme d'un coup tous les termes non
    /// traversés.
    #[test]
    fn mika2518_v3b_sept_controles_negatifs_sur_le_chemin_detache() {
        let head = sha(2489);
        let branch = "feat/2425/agent-exposer";
        let merged = || merged_pr(2489, branch, 7200);
        let mut failures: Vec<String> = Vec::new();

        // N1 — PR **ouverte** à ce SHA. AC2 littéral, et la porte d'entrée du
        // bras de purge de mika#2497 (§ 4.3) : c'est ce motif qui alimente sa
        // population.
        let open = {
            let mut pr = merged();
            pr.state = "OPEN".to_string();
            pr.closed_at = None;
            pr
        };
        // N3 — SHA inexploitable. R-2, et le SHA nul n'est pas théorique : c'est
        // ce que le porcelain rend pour le checkout principal.
        let null_sha = "0".repeat(40);
        let non_hex = "z".repeat(40);
        let truncated = head[..39].to_string();
        let no_head = WorktreeEntry {
            path: DETACHED_WT.to_string(),
            branch: None,
            head: None,
        };

        #[allow(clippy::type_complexity)]
        let cases: Vec<(
            &str,
            WorktreeEntry,
            PrIndex,
            LiveCwds,
            HashMap<String, WorkState>,
            &str,
        )> = vec![
            (
                "N1 — T4, une PR ouverte à ce SHA",
                detached(DETACHED_WT, &head),
                index(vec![open]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_PR_OPEN,
            ),
            (
                "N2 — T3, aucune PR à ce SHA",
                detached(DETACHED_WT, &sha(9999)),
                index(vec![merged()]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD_PR_UNKNOWN,
            ),
            (
                "N3a — T2, ligne HEAD absente",
                no_head,
                index(vec![merged()]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD,
            ),
            (
                "N3b — T2, SHA nul (mesuré en production)",
                detached(DETACHED_WT, &null_sha),
                index(vec![merged()]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD,
            ),
            (
                "N3c — T2, SHA non hexadécimal",
                detached(DETACHED_WT, &non_hex),
                index(vec![merged()]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD,
            ),
            (
                "N3d — T2, SHA tronqué à 39",
                detached(DETACHED_WT, &truncated),
                index(vec![merged()]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD,
            ),
            (
                "N4 — T7, worktree dirty",
                detached(DETACHED_WT, &head),
                index(vec![merged()]),
                no_processes(),
                HashMap::from([(DETACHED_WT.to_string(), WorkState::Dirty)]),
                REASON_DIRTY,
            ),
            (
                "N5 — T5, PR close depuis moins que la grâce",
                detached(DETACHED_WT, &head),
                index(vec![merged_pr(2489, branch, 10)]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_TOO_YOUNG,
            ),
            (
                "N6 — T6, un processus vivant dedans",
                detached(DETACHED_WT, &head),
                index(vec![merged()]),
                LiveCwds::Enumerated(vec![PathBuf::from(format!("{DETACHED_WT}/crates"))]),
                clean(DETACHED_WT),
                REASON_LIVE_PROCESS,
            ),
            // N7 — AC2 mot pour mot : *« branche existante ailleurs »*. La PR
            // mergée porte bien la branche, mais son `headRefOid` est ailleurs :
            // la clé SHA **ne retombe pas** sur la clé branche. Sans ce contrôle,
            // un repli heuristique (R-7) passerait inaperçu.
            (
                "N7 — R-7, branche connue par ailleurs mais SHA sans PR",
                detached(DETACHED_WT, &sha(9999)),
                index(vec![merged_pr(2489, branch, 7200)]),
                no_processes(),
                clean(DETACHED_WT),
                REASON_DETACHED_HEAD_PR_UNKNOWN,
            ),
        ];

        for (label, e, prs, live, work, expected) in cases {
            let s = select(&[e], &prs, &live, &work);
            if !s.candidates.is_empty() {
                failures.push(format!(
                    "{label} — le worktree est entré dans la population de \
                     retrait ; le HEAD détaché seul ne doit jamais suffire (AC2)"
                ));
                continue;
            }
            let got = only_reason(&s);
            if got != vec![expected] {
                failures.push(format!(
                    "{label} — motif attendu {expected:?}, lu {got:?} ; un motif \
                     voisin signifie que le terme a été absorbé en aval, donc \
                     que ce contrôle ne prouve plus rien"
                ));
            }
        }

        // R-6 sur la population que le bras de purge consomme : le refus
        // `pr_open` doit porter la branche **de la PR**, sinon la surface
        // opérateur de mika#2497 reste aveugle sur cette population (§ 4.3).
        let open = {
            let mut pr = merged();
            pr.state = "OPEN".to_string();
            pr.closed_at = None;
            pr
        };
        let s = select(
            &[detached(DETACHED_WT, &head)],
            &index(vec![open]),
            &no_processes(),
            &clean(DETACHED_WT),
        );
        if s.refusals.first().and_then(|r| r.branch.as_deref()) != Some(branch) {
            failures.push(format!(
                "R-6 — le refus `pr_open` d'un détaché doit porter {branch:?}, \
                 lu {:?}",
                s.refusals.first().and_then(|r| r.branch.as_deref())
            ));
        }

        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// **V3c — anti-vacuité (R-8).** Un worktree **attaché** dont la branche n'a
    /// aucune PR, mais dont le `HEAD` apparie une PR mergée, est refusé
    /// `pr_unknown` — **pas** candidat.
    ///
    /// Sans ce test, « la clé SHA marche » serait indistinguable de « tout est
    /// résolu par SHA », et le chemin nominal aurait gagné un risque gratuit.
    #[test]
    fn mika2518_v3c_le_chemin_attache_nest_jamais_resolu_par_sha() {
        let head = sha(2489);
        let s = select(
            &[attached_with_head(
                DETACHED_WT,
                "feat/une-branche-sans-pr",
                &head,
            )],
            // La PR existe, son `headRefOid` **est** le HEAD du worktree — mais
            // sa `headRefName` est une autre branche.
            &index(vec![merged_pr(2489, "feat/2425/agent-exposer", 7200)]),
            &no_processes(),
            &clean(DETACHED_WT),
        );
        assert!(
            s.candidates.is_empty(),
            "R-8 — un worktree attaché est résolu par sa branche, et seulement \
             par elle"
        );
        assert_eq!(only_reason(&s), vec![REASON_PR_UNKNOWN]);
    }

    /// **V3d — non-régression du chemin attaché.** Le contrôle positif de
    /// mika#2420 reste vert **sans modification de sa fixture**, et son candidat
    /// porte `resolution = branch` avec `head_sha = None`.
    #[test]
    fn mika2518_v3d_le_chemin_attache_porte_sa_resolution_et_aucun_sha() {
        let s = select(
            &[entry(WT, Some("fix/2420/x"))],
            &index(vec![merged_pr(2411, "fix/2420/x", 7200)]),
            &no_processes(),
            &clean(WT),
        );
        assert_eq!(s.candidates.len(), 1, "refus: {:?}", only_reason(&s));
        let c = &s.candidates[0];
        assert_eq!(c.resolution, RESOLUTION_BRANCH);
        assert_eq!(
            c.head_sha, None,
            "le chemin attaché ne porte pas de SHA de jointure : il n'en a pas \
             eu besoin pour résoudre"
        );
        assert_eq!(c.branch, "fix/2420/x");
    }

    /// **V3e — aucune suppression de branche locale sur le chemin détaché
    /// (R-5).** Testé sur le prédicat, pas sur `remove_worktree`, qui touche le
    /// disque.
    #[test]
    fn mika2518_v3e_le_chemin_detache_ne_supprime_aucune_branche_locale() {
        assert!(should_delete_local_branch(RESOLUTION_BRANCH));
        assert!(
            !should_delete_local_branch(RESOLUTION_DETACHED_SHA),
            "la branche nommée par la PR n'a jamais été checked out par ce \
             worktree : la supprimer serait un effet de bord sans mandat"
        );
    }

    /// **V3f — la surface d'audit.** `tool_name` **inchangé** (R3), `reasoning`
    /// **commence par** `resolution=detached_sha` — ancré, donc exact pour un
    /// `LIKE` — et porte `head_sha`. `after_value` reste les octets.
    #[tokio::test]
    async fn mika2518_v3f_laudit_porte_la_resolution_sans_second_tool_name() {
        let head = sha(2489);
        let candidate = ReapCandidate {
            path: DETACHED_WT.to_string(),
            branch: "feat/2425/agent-exposer".to_string(),
            pr_number: 2489,
            pr_state: "MERGED".to_string(),
            pr_url: "https://github.com/senara-solutions/mika/pull/2489".to_string(),
            resolution: RESOLUTION_DETACHED_SHA,
            head_sha: Some(head.clone()),
        };
        let size = SizeMeasurement {
            bytes: Some(17_000_000_000),
            truncated: false,
        };

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        record_reaped(
            &db,
            "session-2518",
            &candidate,
            &size,
            Disposition::Armed,
            "trace-2518",
        )
        .await;
        let events = db.get_audit_events("session-2518").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == REAPED_TOOL)
            .expect("le `tool_name` du retrait est inchangé (R3)");
        assert_eq!(row.target_key, format!("worktree:{DETACHED_WT}"));
        assert_eq!(
            row.after_value.as_deref(),
            Some("17000000000"),
            "`after_value` reste les octets — c'est ce que l'opérateur somme"
        );
        let reasoning = row.reasoning.as_deref().unwrap_or_default();
        assert!(
            reasoning.starts_with("resolution=detached_sha"),
            "ancré en tête, pour que `reasoning LIKE 'resolution=detached_sha%'` \
             soit exact plutôt qu'une sous-chaîne flottante — lu: {reasoning}"
        );
        assert!(
            reasoning.contains(&format!("head_sha={head}")),
            "le SHA apparié rend la décision rejouable (HALTE 1) — lu: {reasoning}"
        );

        // Miroir en `observe` : la ligne s'écrit sous `would_dispose`, et
        // **aucune** ligne `worktree_reaped` (non-régression mika#2469).
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        record_reaped(
            &db,
            "session-2518-observe",
            &candidate,
            &size,
            Disposition::Observe,
            "trace-2518",
        )
        .await;
        let events = db.get_audit_events("session-2518-observe").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == WOULD_DISPOSE_TOOL)
            .expect("en observe, la ligne est `worktree_reap_would_dispose`");
        assert!(
            row.reasoning
                .as_deref()
                .unwrap_or_default()
                .starts_with("resolution=detached_sha")
        );
        assert!(
            !events.iter().any(|e| e.tool_name == REAPED_TOOL),
            "en observe, aucune ligne `worktree_reaped` (mika#2469)"
        );
    }

    /// **V4 — `usable_head_sha` à ses bornes.**
    #[test]
    fn mika2518_v4_le_sha_est_normalise_a_un_seul_site() {
        let good = "c5c4d70f0cebdbfe3e821e65951d53473e8d99d4";
        assert_eq!(usable_head_sha(good).as_deref(), Some(good));
        // Casse : normalisée en minuscules, les deux côtés de l'appariement
        // passant par ici.
        assert_eq!(
            usable_head_sha(&good.to_ascii_uppercase()).as_deref(),
            Some(good)
        );
        // Espaces autour : le porcelain est trim-é à la lecture.
        assert_eq!(
            usable_head_sha(&format!("  {good}\n")).as_deref(),
            Some(good)
        );

        for bad in [
            "",
            &"0".repeat(40),     // le SHA nul, mesuré en production
            &good[..39],         // 39
            &format!("{good}a"), // 41
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", // non hexadécimal
        ] {
            assert_eq!(
                usable_head_sha(bad),
                None,
                "un SHA inexploitable ne doit jamais devenir une clé: {bad:?}"
            );
        }
    }

    /// **V4 — `PrIndex` : les deux clés, un seul site de construction.**
    #[test]
    fn mika2518_v4_lindex_porte_les_deux_cles() {
        let idx = index(vec![merged_pr(2489, "feat/a", 7200)]);
        let head = sha(2489);
        assert_eq!(idx.by_branch("feat/a").map(<[_]>::len), Some(1));
        assert_eq!(idx.by_head_sha(&head).map(<[_]>::len), Some(1));
        // Appariement insensible à la casse des deux côtés.
        assert_eq!(
            idx.by_head_sha(&head.to_ascii_uppercase()).map(<[_]>::len),
            Some(1)
        );
        assert!(idx.by_branch("feat/inconnue").is_none());
        assert!(idx.by_head_sha(&sha(1)).is_none());

        // Deux PR partageant un `headRefOid` (une fermée puis rouverte depuis le
        // même commit) : les deux sont rendues, et c'est T4 — formulé en négatif —
        // qui tranche, sans une ligne de plus (§ 2.5).
        let mut reopened = merged_pr(2490, "feat/b", 60);
        reopened.state = "OPEN".to_string();
        reopened.closed_at = None;
        reopened.head_ref_oid = sha(2489);
        let idx = index(vec![merged_pr(2489, "feat/a", 7200), reopened]);
        assert_eq!(idx.by_head_sha(&head).map(<[_]>::len), Some(2));
        let s = select(
            &[detached(DETACHED_WT, &head)],
            &idx,
            &no_processes(),
            &clean(DETACHED_WT),
        );
        assert!(
            s.candidates.is_empty(),
            "deux PR au même SHA dont une ouverte : T4 conserve"
        );
        assert_eq!(only_reason(&s), vec![REASON_PR_OPEN]);
    }

    /// **V4 — R-4 : un payload `gh` SANS `headRefOid` parse quand même**, et
    /// n'apparie rien.
    ///
    /// C'est le test qui garantit que **le chemin attaché survit à l'absence du
    /// champ** : sans `#[serde(default)]`, `list_prs` rendrait `Err` et le dépôt
    /// entier serait sauté — un champ additif éteignant la fonction qu'il
    /// enrichit.
    #[test]
    fn mika2518_v4_un_head_ref_oid_absent_degrade_sans_eteindre() {
        let sans_oid = r#"[{"number":2489,"state":"MERGED",
            "headRefName":"feat/2425/agent-exposer",
            "closedAt":"2026-09-24T19:30:25Z",
            "url":"https://github.com/senara-solutions/mika/pull/2489"}]"#;
        let prs: Vec<PrSnapshot> = serde_json::from_str(sans_oid)
            .expect("R-4 — l'absence du champ ne doit pas être fatale");
        assert_eq!(prs[0].head_ref_oid, "");

        let idx = PrIndex::build(prs);
        assert_eq!(
            idx.by_branch("feat/2425/agent-exposer").map(<[_]>::len),
            Some(1),
            "le chemin attaché reste résolu"
        );
        // La chaîne vide n'indexe rien : la dégradation est bornée au nouveau
        // chemin, et un détaché retombe sur « conserver ».
        assert!(idx.by_head_sha("").is_none());
        let s = select(
            &[detached(DETACHED_WT, &sha(2489))],
            &idx,
            &no_processes(),
            &clean(DETACHED_WT),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(only_reason(&s), vec![REASON_DETACHED_HEAD_PR_UNKNOWN]);
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
                // mika#2518 — ajouté en queue, jamais inséré : l'ordre est lu
                // par un humain qui compare deux versions de ce test.
                "detached_head_pr_unknown",
            ],
            "renommer un motif est une rupture de format de fil : la dater dans \
             CLAUDE.md, jamais mettre ce test à jour en silence"
        );
        let mut seen = std::collections::HashSet::new();
        for r in ALL_REFUSAL_REASONS {
            assert!(seen.insert(*r), "motif dupliqué: {r}");
        }
    }

    /// **Format de fil (mika#2518).** Les clés de résolution atterrissent en tête
    /// de `audit_events.reasoning` et sur le champ `resolution` de la ligne INFO.
    /// Même forme que son aînée ci-dessus : liste figée, unicité, et l'assertion
    /// que les deux valeurs diffèrent.
    #[test]
    fn mika2518_les_resolutions_sont_un_format_de_fil() {
        assert_eq!(
            ALL_RESOLUTIONS,
            &["branch", "detached_sha"],
            "renommer une clé de résolution est une rupture de format de fil : \
             la dater dans CLAUDE.md, jamais mettre ce test à jour en silence"
        );
        let mut seen = std::collections::HashSet::new();
        for r in ALL_RESOLUTIONS {
            assert!(seen.insert(*r), "résolution dupliquée: {r}");
        }
        assert_ne!(RESOLUTION_BRANCH, RESOLUTION_DETACHED_SHA);
        // `detached_sha` et non `detached_head` : ce dernier est déjà un motif de
        // refus, et deux vocabulaires distincts ne doivent pas partager une
        // chaîne (§ 2.3).
        assert_ne!(
            RESOLUTION_DETACHED_SHA, REASON_DETACHED_HEAD,
            "la clé de résolution et le motif de refus doivent rester deux \
             chaînes distinctes"
        );
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
            resolution: RESOLUTION_BRANCH,
            head_sha: None,
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
        let e = DISPOSITION_ENV;
        assert_eq!(parse_disposition(None, e), Disposition::Armed);
        assert_eq!(parse_disposition(Some(""), e), Disposition::Armed);
        assert_eq!(parse_disposition(Some("armed"), e), Disposition::Armed);
        assert_eq!(
            parse_disposition(Some(" OBSERVE "), e),
            Disposition::Observe
        );
        assert_eq!(parse_disposition(Some("observ"), e), Disposition::Armed);
        assert_eq!(parse_disposition(Some("0"), e), Disposition::Armed);
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
        // mika#2518 — la ligne `HEAD` est capturée **brute**, pour chaque entrée,
        // détachée comprise. La normalisation est le travail d'`usable_head_sha`.
        assert_eq!(entries[0].head.as_deref(), Some("abc123"));
        assert_eq!(entries[1].head.as_deref(), Some("def456"));
        assert_eq!(
            entries[2].head.as_deref(),
            Some("789abc"),
            "c'est très exactement la ligne qui rend un détaché résolvable"
        );
    }

    /// **V4 — la forme réelle du porcelain**, relevée sur cet arbre : SHA
    /// complet, entrée attachée, entrée détachée, entrée `prunable` écartée, et
    /// une entrée `bare` (qui n'a ni branche ni `HEAD` exploitable).
    #[test]
    fn mika2518_v4_le_registre_reel_porte_le_sha_de_chaque_entree() {
        let attached = "c5c4d70f0cebdbfe3e821e65951d53473e8d99d4";
        let detached_sha = "24f25e99a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let porcelain = format!(
            "\
worktree /data/workspace/mika-platform/mika
HEAD 0000000000000000000000000000000000000000
detached

worktree /data/workspace/mika-platform/.claude/worktrees/feat-2518-x/mika
HEAD {attached}
branch refs/heads/feat/2518/x

worktree /data/workspace/mika-platform/.claude/worktrees/feat-2425-agent/mika
HEAD {detached_sha}
detached

worktree /data/workspace/mika-platform/.claude/worktrees/gone/mika
HEAD {attached}
branch refs/heads/feat/gone/x
prunable gitdir file points to non-existent location

worktree /data/workspace/mika-platform/mika-bare
bare
"
        );
        let entries = parse_worktree_registry(&porcelain);
        assert_eq!(entries.len(), 4, "l'entrée `prunable` est écartée");

        // Le checkout principal : détaché **et** SHA nul — la forme mesurée qui
        // motive le refus du SHA nul dans `usable_head_sha`.
        assert_eq!(entries[0].branch, None);
        assert_eq!(usable_head_sha(entries[0].head.as_deref().unwrap()), None);

        assert_eq!(entries[1].branch.as_deref(), Some("feat/2518/x"));
        assert_eq!(
            usable_head_sha(entries[1].head.as_deref().unwrap()).as_deref(),
            Some(attached)
        );

        assert_eq!(entries[2].branch, None);
        assert_eq!(
            usable_head_sha(entries[2].head.as_deref().unwrap()).as_deref(),
            Some(detached_sha),
            "c'est la seule entrée que mika#2518 rend fauchable, et seulement \
             si une PR porte ce `headRefOid`"
        );

        // L'entrée `bare` n'a ni branche ni `HEAD` : elle sort par T1 (hors
        // racine gérée) avant même la question de la clé.
        assert_eq!(entries[3].branch, None);
        assert_eq!(entries[3].head, None);
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
            resolution: RESOLUTION_BRANCH,
            head_sha: None,
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
    // -- mika#2449 : la sonde de saleté du checkout principal ---------------

    /// V3 — fonction pure : propre → rien ; sale → compte, chemins, empreinte ;
    /// illisible → signal nommé, jamais « propre ».
    #[test]
    fn mika2449_classify_main_checkout_trois_etats() {
        assert_eq!(classify_main_checkout(Some("")), MainCheckoutState::Clean);
        assert_eq!(
            classify_main_checkout(Some("\n  \n")),
            MainCheckoutState::Clean
        );
        assert_eq!(classify_main_checkout(None), MainCheckoutState::Unreadable);

        let dirty = classify_main_checkout(Some(
            "M  skills/bundled/_shared/dispatch-lib.sh\n?? site/index.html\nA  scripts/smoke-webhook-chain\n",
        ));
        let MainCheckoutState::Dirty {
            file_count,
            files,
            truncated,
            fingerprint,
        } = dirty
        else {
            panic!("attendu Dirty");
        };
        assert_eq!(file_count, 3);
        assert!(!truncated);
        assert_eq!(files.len(), 3);
        assert!(
            files.iter().any(|f| f.ends_with("dispatch-lib.sh")),
            "{files:?}"
        );
        assert_eq!(
            fingerprint.len(),
            16,
            "empreinte hex 64 bits : {fingerprint}"
        );
    }

    /// V3 — l'empreinte ne dépend pas de l'ordre de sortie de git, et deux
    /// listes différentes ont deux empreintes (c'est la clé de D4).
    #[test]
    fn mika2449_lempreinte_est_stable_et_discriminante() {
        let fp = |s: &str| match classify_main_checkout(Some(s)) {
            MainCheckoutState::Dirty { fingerprint, .. } => fingerprint,
            other => panic!("attendu Dirty, obtenu {other:?}"),
        };
        assert_eq!(fp("M  a\n?? b\n"), fp("?? b\nM  a\n"));
        assert_ne!(fp("M  a\n?? b\n"), fp("M  a\n"));
        assert_ne!(
            fp("M  a\n"),
            fp("?? a\n"),
            "le statut fait partie de la liste"
        );
    }

    /// V3 — plafond de 20 chemins : `files` est tronqué, `file_count` reste
    /// le compte réel, et l'empreinte couvre la liste ENTIÈRE (deux checkouts
    /// à 25 fichiers dont 20 communs ne se confondent pas).
    #[test]
    fn mika2449_les_chemins_sont_plafonnes_mais_le_compte_et_lempreinte_non() {
        let mk = |n: usize, suffix: &str| {
            (0..n)
                .map(|i| format!("?? f{i:02}{suffix}\n"))
                .collect::<String>()
        };
        let a = classify_main_checkout(Some(&mk(25, "")));
        let MainCheckoutState::Dirty {
            file_count,
            files,
            truncated,
            fingerprint: fp_a,
        } = a
        else {
            panic!("attendu Dirty");
        };
        assert_eq!(file_count, 25);
        assert_eq!(files.len(), MAIN_CHECKOUT_DIRTY_MAX_PATHS);
        assert!(truncated);
        let b = mk(20, "") + "?? zz1\n?? zz2\n?? zz3\n?? zz4\n?? zz5\n";
        let MainCheckoutState::Dirty {
            fingerprint: fp_b, ..
        } = classify_main_checkout(Some(&b))
        else {
            panic!("attendu Dirty");
        };
        assert_ne!(
            fp_a, fp_b,
            "l'empreinte porte la liste entière, pas les 20 premiers"
        );
    }

    /// La clé d'audit et la requête d'attribution portent le checkout, les
    /// bornes de fenêtre et le filtre de chemin sous la forme mesurée.
    #[test]
    fn mika2449_cle_et_requete_dattribution() {
        let repo = "/data/workspace/mika-platform/mika";
        assert_eq!(
            main_checkout_audit_key(repo, "deadbeef00000000"),
            "main_checkout:/data/workspace/mika-platform/mika@deadbeef00000000"
        );
        let q = main_checkout_attribution_query(
            repo,
            Some("2026-09-20T17:00:00Z"),
            "2026-09-20T17:30:00Z",
        );
        assert!(q.contains("FROM tool_calls"), "{q}");
        assert!(q.contains("tool_name = 'run_shell'"), "{q}");
        assert!(
            q.contains("BETWEEN '2026-09-20T17:00:00Z' AND '2026-09-20T17:30:00Z'"),
            "{q}"
        );
        assert!(q.contains("LIKE '%mika-platform/mika%'"), "{q}");
        assert!(q.contains("checkout %--%") && q.contains("stash"), "{q}");
        let unknown = main_checkout_attribution_query(repo, None, "2026-09-20T17:30:00Z");
        assert!(unknown.contains("inconnu"), "{unknown}");
        // La sonde ne nomme aucun producteur (D3).
        assert!(!q.contains("mika-qa") && !q.contains("pilot"), "{q}");
    }

    /// V3b — placement : un `repo_dir` sans `.git` ne produit AUCUNE émission
    /// de saleté (c'est `worktree_reap_no_checkout` qui parle, en amont).
    #[tokio::test]
    async fn mika2449_un_chemin_sans_depot_nemet_rien() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let state =
            probe_main_checkout(&db, "session-2449", tmp.path(), Utc::now(), "trace-1").await;
        assert_eq!(state, MainCheckoutState::Unreadable);
        let rows = db
            .get_audit_event_rows_by_tool_name(MAIN_CHECKOUT_DIRTY_TOOL)
            .await
            .unwrap();
        assert!(
            rows.is_empty(),
            "aucune ligne main_checkout_dirty sans dépôt"
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} dans {}", dir.display());
    }

    /// AC3 / AC4 / V4 sur un vrai dépôt : propre → zéro ligne ; sali → une
    /// ligne datée portant le checkout, le compte, les chemins et la requête
    /// d'attribution dont la borne basse est le dernier tick propre ; même
    /// liste sur deux ticks → une seule ligne ; liste changée → une seconde.
    #[tokio::test]
    async fn mika2449_un_checkout_sale_ecrit_une_ligne_datee_et_dedupliquee() {
        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("mika");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("scripts/x.sh"), "a\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);
        let repo_key = repo.display().to_string();

        // Tick 1 : propre → zéro ligne (AC4), et l'instant est mémorisé.
        let t1 = crate::timestamp::parse("2026-09-20T17:00:00Z").unwrap();
        let s1 = probe_main_checkout(&db, "s", &repo, t1, "trace").await;
        assert_eq!(s1, MainCheckoutState::Clean);
        assert!(
            db.get_audit_event_rows_by_tool_name(MAIN_CHECKOUT_DIRTY_TOOL)
                .await
                .unwrap()
                .is_empty()
        );

        // Tick 2 : sali (le geste de M0 : index + arbre) → une ligne.
        std::fs::write(repo.join("scripts/x.sh"), "b\n").unwrap();
        git(&repo, &["add", "scripts/x.sh"]);
        std::fs::write(repo.join("site.html"), "new\n").unwrap();
        let t2 = crate::timestamp::parse("2026-09-20T17:10:00Z").unwrap();
        let s2 = probe_main_checkout(&db, "s", &repo, t2, "trace").await;
        assert!(
            matches!(s2, MainCheckoutState::Dirty { file_count: 2, .. }),
            "{s2:?}"
        );
        let rows = db
            .get_audit_event_rows_by_tool_name(MAIN_CHECKOUT_DIRTY_TOOL)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let (target_key, _before, after_value, reasoning) = &rows[0];
        assert!(
            target_key.starts_with(&format!("main_checkout:{repo_key}@")),
            "{target_key}"
        );
        assert_eq!(after_value.as_deref(), Some("2"));
        let reasoning = reasoning.as_deref().unwrap_or_default();
        assert!(reasoning.contains("M  scripts/x.sh"), "{reasoning}");
        assert!(reasoning.contains("?? site.html"), "{reasoning}");
        assert!(
            reasoning.contains("window_start=2026-09-20T17:00:00Z window_end=2026-09-20T17:10:00Z"),
            "la borne basse est le dernier tick propre : {reasoning}"
        );
        assert!(reasoning.contains("FROM tool_calls"), "{reasoning}");

        // Tick 3 : même liste → aucune seconde ligne (D4).
        let t3 = crate::timestamp::parse("2026-09-20T17:20:00Z").unwrap();
        probe_main_checkout(&db, "s", &repo, t3, "trace").await;
        assert_eq!(
            db.get_audit_event_rows_by_tool_name(MAIN_CHECKOUT_DIRTY_TOOL)
                .await
                .unwrap()
                .len(),
            1
        );

        // Tick 4 : la liste change → une seconde ligne (changement d'état).
        std::fs::write(repo.join("third.txt"), "x\n").unwrap();
        let t4 = crate::timestamp::parse("2026-09-20T17:30:00Z").unwrap();
        probe_main_checkout(&db, "s", &repo, t4, "trace").await;
        assert_eq!(
            db.get_audit_event_rows_by_tool_name(MAIN_CHECKOUT_DIRTY_TOOL)
                .await
                .unwrap()
                .len(),
            2
        );

        // R5 — rien n'a été nettoyé : l'arbre est toujours sale.
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(!st.stdout.is_empty(), "la sonde ne doit rien nettoyer (R5)");
    }

    /// V4 (non-blocage) — un checkout principal sale ne change ni le verdict de
    /// fauche ni le compte : la fonction pure de sélection ignore l'état du
    /// checkout principal par construction (elle ne le reçoit pas), et la
    /// sonde ne touche à aucun compteur du tick. Contrôle par le type : la
    /// signature de `select_worktrees_to_reap` ne porte pas de
    /// `MainCheckoutState`.
    #[test]
    fn mika2449_la_sonde_nentre_pas_dans_le_verdict_de_fauche() {
        let src = include_str!("worktree_reaper.rs");
        let sig_start = src
            .find("pub fn select_worktrees_to_reap(")
            .expect("signature de select_worktrees_to_reap");
        let sig = &src[sig_start..sig_start + 600];
        assert!(
            !sig.contains("MainCheckoutState"),
            "le verdict de fauche ne doit pas dépendre de l'état du checkout principal"
        );
        // Et le branchement dans la boucle jette le résultat.
        assert!(
            src.contains("let _ = probe_main_checkout("),
            "le résultat de la sonde n'entre dans aucun compte du tick"
        );
    }

    /// U5 — `main_checkout_dirty` a **un seul writer** dans le crate.
    ///
    /// Un second writer rendrait la requête d'attribution que chaque ligne
    /// recopie inexacte, en silence (allowlist vide ; quand ça tire, on retire
    /// le second site). Contrôle de bonne foi : `scanned > 0`.
    #[test]
    fn mika2449_le_tool_name_main_checkout_dirty_a_un_seul_writer() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("worktree_reaper.rs");
        let needle = format!("main_checkout{}", "_dirty");

        let mut offenders = Vec::new();
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
                if content.contains(&needle) {
                    offenders.push(path.display().to_string());
                }
            }
        }
        assert!(scanned > 0, "la garde n'a scanné aucun fichier");
        assert!(
            offenders.is_empty(),
            "mika#2449 — `{needle}` est SOLE WRITER de `worktree_reaper.rs`.\n{}",
            offenders.join("\n")
        );
        // Et ce module l'écrit à exactement un site (`event = MAIN_CHECKOUT_DIRTY_TOOL`
        // et `log_audit_event(…, MAIN_CHECKOUT_DIRTY_TOOL, …)` dans la même fonction).
        let here = include_str!("worktree_reaper.rs");
        let literal_sites = here.matches(&format!("\"{needle}\"")).count();
        assert_eq!(
            literal_sites, 1,
            "le littéral doit vivre dans la seule constante"
        );
    }

    // =======================================================================
    // mika#2497 — la purge du `target/` d'un worktree vif mais inactif
    // =======================================================================

    /// Un worktree **géré** fabriqué de toutes pièces dans un tmpdir.
    ///
    /// Aucun test de cette section ne touche quoi que ce soit hors de son
    /// tmpdir — c'est l'exigence explicite du DoD (AC9), y compris sur les
    /// chemins d'échec, puisque `TempDir` nettoie à la destruction.
    fn fake_worktree(root: &Path, slug: &str) -> PathBuf {
        let wt = root.join(".claude/worktrees").join(slug).join("mika");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join("src/main.rs"), b"fn main() {}").unwrap();
        std::fs::write(wt.join(".git"), b"gitdir: /elsewhere\n").unwrap();
        wt
    }

    /// Un `target/` plausible : `target/debug/deps/<un fichier>`.
    fn fake_target(wt: &Path) -> PathBuf {
        let target = wt.join("target");
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.join("debug/deps/libfoo.rlib"), vec![0u8; 4096]).unwrap();
        target
    }

    /// Vieillit **tout** l'arbre : les répertoires aussi, et après les fichiers
    /// qu'ils contiennent — créer un fichier rajeunit son répertoire parent.
    fn age_tree(root: &Path, secs: u64) {
        let when =
            filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(secs));
        let mut dirs = vec![root.to_path_buf()];
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.symlink_metadata().unwrap().is_dir() {
                    dirs.push(p.clone());
                    stack.push(p);
                } else {
                    filetime::set_file_mtime(&p, when).unwrap();
                }
            }
        }
        // Du plus profond au moins profond : inutile ici (on ne recrée rien),
        // mais l'ordre rend le helper réutilisable sans surprise.
        for d in dirs.iter().rev() {
            filetime::set_file_mtime(d, when).unwrap();
        }
    }

    fn pr_open_refusal(path: &Path, branch: &str) -> ReapRefusal {
        ReapRefusal {
            path: path.to_string_lossy().into_owned(),
            branch: Some(branch.to_string()),
            reason: REASON_PR_OPEN,
        }
    }

    fn open_pr(number: u64, branch: &str) -> PrSnapshot {
        PrSnapshot {
            number,
            state: "OPEN".to_string(),
            head_ref_name: branch.to_string(),
            closed_at: None,
            head_ref_oid: sha(number),
            url: format!("https://github.com/senara-solutions/mika/pull/{number}"),
        }
    }

    fn purge_states(wt: &Path, state: TargetState) -> HashMap<String, TargetState> {
        HashMap::from([(wt.to_string_lossy().into_owned(), state)])
    }

    fn free_lock(target: &str) -> HashMap<String, LockProbe> {
        HashMap::from([(target.to_string(), LockProbe::Free)])
    }

    fn purge_reasons(s: &TargetPurgeSelection) -> Vec<&'static str> {
        s.refusals.iter().map(|r| r.reason).collect()
    }

    /// Le prédicat complet sur un worktree unique, sans toucher au disque.
    fn purge_select(
        wt: &Path,
        state: TargetState,
        live: &LiveCwds,
        probe: LockProbe,
    ) -> TargetPurgeSelection {
        let path = wt.to_string_lossy().into_owned();
        let target = target_dir_of(&path);
        select_target_purges(
            &[pr_open_refusal(wt, "fix/2497/x")],
            live,
            &purge_states(wt, state),
            &HashMap::from([(target, probe)]),
            &TargetPurgeConfig::default(),
        )
    }

    // -- V1 : le cas du DoD, littéralement ----------------------------------

    /// **V1 / AC1** — un worktree factice en tmpdir portant un `target/` daté
    /// au-delà de la fenêtre : la purge le retire, et **le reste du worktree
    /// est intact**.
    ///
    /// La seconde moitié est ce qui distingue « la purge marche » de « la purge
    /// supprime trop » ; sans elle, un `remove_dir_all` sur le worktree entier
    /// passerait ce test.
    #[tokio::test]
    async fn mika2497_v1_le_target_inactif_est_purge_et_le_reste_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2497-v1",
            "trace-v1",
            &[pr_open_refusal(&wt, "fix/2497/x")],
            &index(vec![open_pr(2497, "fix/2497/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig::default(),
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.purged, 1, "refus inattendu: {stats:?}");
        assert_eq!(stats.failed, 0);
        assert!(!target.exists(), "`target/` doit avoir été retiré");
        assert!(
            wt.join("src/main.rs").exists(),
            "le reste du worktree doit être intact"
        );
        assert!(wt.join(".git").exists(), "`.git` doit être intact");
        assert_eq!(budget, 1, "une seule écriture doit avoir été débitée");

        let events = db.get_audit_events("session-2497-v1").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == TARGET_PURGED_TOOL)
            .expect("une ligne `target_purged` doit exister");
        assert_eq!(
            row.target_key,
            format!("target:{}", target.to_string_lossy())
        );
        let reasoning = row.reasoning.as_deref().unwrap_or_default();
        assert!(reasoning.contains("pr=2497"), "{reasoning}");
        assert!(reasoning.contains("disposition=armed"), "{reasoning}");
    }

    // -- V2 : les cinq refus, un test chacun --------------------------------

    /// **V2.1 / P2** — pas de `target/` du tout.
    #[test]
    fn mika2497_v2_sans_target_rien_a_purger() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let s = purge_select(&wt, TargetState::Absent, &no_processes(), LockProbe::Free);
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_NO_TARGET_DIR]);
    }

    /// **V2.2 / P2** — `target` existe mais n'est pas un répertoire.
    #[test]
    fn mika2497_v2_un_target_qui_est_un_fichier_est_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        std::fs::write(wt.join("target"), b"pas un repertoire").unwrap();
        assert_eq!(
            inspect_target_dir(&wt, SystemTime::now()),
            TargetState::NotADirectory
        );
        let s = purge_select(
            &wt,
            TargetState::NotADirectory,
            &no_processes(),
            LockProbe::Free,
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_TARGET_NOT_A_DIR]);
    }

    /// **V2.3 / P3** — un processus vivant a son cwd sous le worktree.
    #[test]
    fn mika2497_v2_un_processus_vivant_dedans_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let live = LiveCwds::Enumerated(vec![wt.join("crates/mika-agent")]);
        let s = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
            },
            &live,
            LockProbe::Free,
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_LIVE_PROCESS]);
    }

    /// **V2.4 / P4** — un `target/` écrit à l'intérieur de la fenêtre.
    #[test]
    fn mika2497_v2_un_target_recent_est_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let s = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(60),
            },
            &no_processes(),
            LockProbe::Free,
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_RECENTLY_ACTIVE]);
    }

    /// **V2.5 / P5** — un verrou de build tenu.
    #[test]
    fn mika2497_v2_un_verrou_de_build_tenu_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let s = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
            },
            &no_processes(),
            LockProbe::Held,
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_BUILD_LOCK_HELD]);
    }

    /// **AC3** — l'illisible conserve, sous son propre nom. Trois signaux, trois
    /// motifs distincts : replier l'un sur l'autre ferait compter un blocage
    /// permanent parmi des transitoires (doctrine mika#2277).
    #[test]
    fn mika2497_lillisible_conserve_sous_son_propre_motif() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");

        // mtime inétablissable.
        let s = purge_select(
            &wt,
            TargetState::Present { idle_secs: None },
            &no_processes(),
            LockProbe::Free,
        );
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_MTIME_UNREADABLE]);

        // `/proc` inénumérable.
        let s = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
            },
            &LiveCwds::Unavailable,
            LockProbe::Free,
        );
        assert_eq!(
            purge_reasons(&s),
            vec![PURGE_REASON_PROCESS_SCAN_UNREADABLE]
        );

        // Verrou insondable.
        let s = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
            },
            &no_processes(),
            LockProbe::Unevaluable,
        );
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_BUILD_LOCK_UNREADABLE]);

        // Et une entrée **absente** de la carte conserve, jamais l'inverse.
        let s = select_target_purges(
            &[pr_open_refusal(&wt, "fix/2497/x")],
            &no_processes(),
            &HashMap::new(),
            &HashMap::new(),
            &TargetPurgeConfig::default(),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_MTIME_UNREADABLE]);
    }

    // -- V3 : la frontière, dans les deux sens ------------------------------

    /// **V3** — sépare « le prédicat mord » de « le prédicat est une
    /// constante ». Sans cette paire, un prédicat toujours-faux passerait V2 en
    /// entier.
    #[test]
    fn mika2497_v3_la_fenetre_mord_des_deux_cotes() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let w = PURGE_IDLE_DEFAULT_SECS;

        let inside = purge_select(
            &wt,
            TargetState::Present {
                idle_secs: Some(w - 1),
            },
            &no_processes(),
            LockProbe::Free,
        );
        assert!(
            inside.candidates.is_empty(),
            "une seconde en deçà: conservé"
        );
        assert_eq!(purge_reasons(&inside), vec![PURGE_REASON_RECENTLY_ACTIVE]);

        for idle in [w, w + 1] {
            let beyond = purge_select(
                &wt,
                TargetState::Present {
                    idle_secs: Some(idle),
                },
                &no_processes(),
                LockProbe::Free,
            );
            assert_eq!(
                beyond.candidates.len(),
                1,
                "idle={idle} doit être purgé, refus: {:?}",
                purge_reasons(&beyond)
            );
            assert_eq!(beyond.candidates[0].idle_secs, idle);
        }
    }

    /// La récence mesurée sur un vrai arbre, à la profondeur déclarée.
    ///
    /// Le contrôle négatif est la seconde moitié : un fichier **récent** au
    /// fond de `target/debug/deps/` doit rajeunir la mesure via le mtime de son
    /// répertoire, sinon la profondeur 2 ne servirait à rien.
    #[test]
    fn mika2497_la_mesure_de_recence_lit_bien_larbre() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        age_tree(&target, 100_000);

        let state = inspect_target_dir(&wt, SystemTime::now());
        match state {
            TargetState::Present {
                idle_secs: Some(idle),
            } => assert!(idle > 99_000, "idle={idle}"),
            other => panic!("attendu un target vieilli, obtenu {other:?}"),
        }

        // Contrôle négatif : une recompilation touche `target/debug/deps/`.
        std::fs::write(target.join("debug/deps/libbar.rlib"), b"neuf").unwrap();
        match inspect_target_dir(&wt, SystemTime::now()) {
            TargetState::Present {
                idle_secs: Some(idle),
            } => assert!(idle < 60, "une écriture récente doit rajeunir: idle={idle}"),
            other => panic!("attendu un target rajeuni, obtenu {other:?}"),
        }
    }

    // -- V3b : l'absence de verrou n'est PAS un verrou illisible -------------

    /// **V3b / AC3** — le cas qui sépare « le terme est fail-safe » de « le
    /// terme est toujours faux ».
    ///
    /// Sans lui, une implémentation traitant l'absence de `.cargo-lock` comme
    /// `build_lock_unreadable` passerait tous les autres tests **en ne purgeant
    /// jamais rien**, ce qui se lirait exactement comme un disque sain (classe
    /// mika#2205).
    #[test]
    fn mika2497_v3b_aucun_cargo_lock_satisfait_le_terme() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        assert_eq!(
            cargo_build_lock_is_free(&target),
            if cfg!(target_os = "linux") {
                LockProbe::Free
            } else {
                LockProbe::Unevaluable
            },
            "cargo ne retire pas `.cargo-lock` après un build : son absence dit \
             que cargo n'a jamais construit ici, pas qu'on n'a pas pu regarder"
        );
    }

    /// **V3b (2)** — le fichier est **découvert**, pas deviné sur `debug`.
    ///
    /// `target/debug/.cargo-lock` est libre et `target/release/.cargo-lock` est
    /// tenu : un prédicat qui devinerait `debug` raterait le build `--release`,
    /// c'est-à-dire échouerait exactement sur le cas qu'on veut voir.
    ///
    /// La dernière moitié passe par [`release_lock_file`] et **non** par un
    /// `drop` nu : celui-ci s'en remettait à la fermeture du descripteur, que le
    /// `fork` d'un sous-processus concurrent peut retarder le temps d'un
    /// `execve` — voir le helper, qui porte la mesure. C'est la forme qui a
    /// flaké en CI le 2026-09-24, et la barrière est un appel système dont le
    /// succès est asserté, jamais une temporisation.
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2497_v3b_le_verrou_est_decouvert_pas_devine() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        free_cargo_lock(&target, "debug");

        // Contrôle positif : tant que rien n'est tenu, le terme est satisfait.
        assert_eq!(cargo_build_lock_is_free(&target), LockProbe::Free);

        let held = hold_cargo_lock(&target, "release");
        assert_eq!(
            cargo_build_lock_is_free(&target),
            LockProbe::Held,
            "un seul verrou tenu suffit à refuser, quel que soit le profil"
        );

        release_lock_file(held);
        assert_eq!(
            cargo_build_lock_is_free(&target),
            LockProbe::Free,
            "le verrou relâché, le terme redevient satisfait — la sonde ne \
             conserve donc pas un verrou à elle"
        );
    }

    /// **V3c / AC11** — hors Linux, le verrou est **inévaluable**, donc il
    /// conserve, et la purge n'y fire jamais.
    ///
    /// Sans ce test, le sens du repli n'est fixé par rien, et l'inversion
    /// (« libre » hors Linux) passerait tous les autres tests en purgeant sur
    /// la seule plateforme où l'on ne peut pas voir un build tourner.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn mika2497_v3c_le_repli_de_plateforme_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        assert_eq!(cargo_build_lock_is_free(&target), LockProbe::Unevaluable);
    }

    // -- V4 : les gardes de chemin ------------------------------------------

    /// **V4 / AC2** — un `target` qui est un **lien symbolique** est refusé,
    /// **et la cible existe toujours**.
    ///
    /// L'assertion porte sur la cible, pas sur le refus : c'est le seul faux
    /// positif irréversible de ce livrable.
    ///
    /// # Le lien pointe vers le `target/` d'un AUTRE worktree géré, et ce
    /// # détail est ce qui rend le test non décoratif
    ///
    /// Un premier jet faisait pointer le lien vers un répertoire voisin nommé
    /// autrement : la garde 1 (« le chemin canonicalisé se termine par
    /// `/target` ») le refusait déjà, donc **retirer la garde du lien
    /// symbolique laissait le test vert** — vérifié par mutation, pas par
    /// raisonnement. Le seul chemin qui traverse les gardes 1 et 2 est un lien
    /// vers un `target/` lui-même situé sous la racine gérée, c'est-à-dire
    /// exactement le cas que la garde 3 existe pour fermer.
    #[cfg(unix)]
    #[test]
    fn mika2497_v4_un_target_lien_symbolique_ne_suit_jamais_le_lien() {
        let tmp = tempfile::tempdir().unwrap();
        let victime = fake_worktree(tmp.path(), "fix-2497-victime");
        let cible = fake_target(&victime);
        std::fs::write(cible.join("precieux.rlib"), b"ne pas perdre").unwrap();

        let piege = fake_worktree(tmp.path(), "fix-2497-piege");
        std::os::unix::fs::symlink(&cible, piege.join("target")).unwrap();

        // Le lien traverse bien les gardes 1 et 2 : sans la garde 3, il serait
        // supprimé — et avec lui le `target/` de la victime.
        let canonique = std::fs::canonicalize(piege.join("target")).unwrap();
        assert!(canonique.to_string_lossy().ends_with("/target"));
        assert!(is_managed_worktree_path(&canonique.to_string_lossy()));

        assert_eq!(
            inspect_target_dir(&piege, SystemTime::now()),
            TargetState::NotADirectory,
            "un lien symbolique n'est jamais un répertoire à purger"
        );
        assert!(
            !target_path_is_disposable(&piege.join("target")),
            "la garde de disposition doit refuser un lien symbolique"
        );
        assert!(
            cible.join("precieux.rlib").exists(),
            "la cible du lien doit être intacte"
        );
    }

    /// **V4 (2) / AC2** — un chemin hors de la racine gérée est refusé aux deux
    /// étages : par le prédicat, et par la garde de disposition.
    #[test]
    fn mika2497_v4_hors_racine_geree_est_refuse() {
        let tmp = tempfile::tempdir().unwrap();
        let dehors = tmp.path().join("pas-un-worktree/mika");
        std::fs::create_dir_all(dehors.join("target")).unwrap();

        let s = select_target_purges(
            &[pr_open_refusal(&dehors, "fix/2497/x")],
            &no_processes(),
            &purge_states(
                &dehors,
                TargetState::Present {
                    idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
                },
            ),
            &free_lock(&target_dir_of(&dehors.to_string_lossy())),
            &TargetPurgeConfig::default(),
        );
        assert!(s.candidates.is_empty());
        assert_eq!(purge_reasons(&s), vec![PURGE_REASON_OUTSIDE_MANAGED_ROOT]);
        assert!(!target_path_is_disposable(&dehors.join("target")));

        // Contrôle négatif : le même arbre **sous** la racine gérée passe la
        // garde, sinon l'assertion ci-dessus serait satisfaite par une garde
        // qui refuse tout.
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        assert!(target_path_is_disposable(&target));
    }

    /// Un répertoire dont le nom n'est pas `target` n'est pas disposable, même
    /// sous la racine gérée : la première garde porte sur le **nom**.
    #[test]
    fn mika2497_v4_seul_un_repertoire_nomme_target_est_disposable() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        std::fs::create_dir_all(wt.join("src/target-ish")).unwrap();
        assert!(!target_path_is_disposable(&wt.join("src")));
        assert!(!target_path_is_disposable(&wt.join("src/target-ish")));
    }

    // -- V5 : observe ne supprime rien --------------------------------------

    /// **V5 / AC5** — en `observe`, `target/` est **toujours là** et l'audit
    /// écrit `target_purge_would_dispose`, jamais `target_purged`.
    ///
    /// C'est la correction que mika#2469 a dû apporter à son aîné ; elle est
    /// prise d'emblée ici.
    ///
    /// **Corrigé par mika#2511 (veille (c)) :** l'assertion portait
    /// `stats.purged == 1` en `observe`, ce qui **figeait le défaut** — le
    /// compteur en mémoire revendiquait une suppression dans un mode qui ne
    /// supprime rien. Elle porte désormais sur `would_purge`. C'est une
    /// correction, pas un assouplissement : la moitié durable de ce test
    /// (`target_purge_would_dispose` écrit, `target_purged` absent) est
    /// inchangée et reste l'assertion porteuse.
    #[tokio::test]
    async fn mika2497_v5_observe_ne_supprime_rien() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2497-v5",
            "trace-v5",
            &[pr_open_refusal(&wt, "fix/2497/x")],
            &index(vec![open_pr(2497, "fix/2497/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig {
                disposition: Disposition::Observe,
                ..TargetPurgeConfig::default()
            },
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.would_purge, 1, "la détection est inconditionnelle");
        assert_eq!(
            stats.purged, 0,
            "mika#2511 — `observe` ne revendique aucune suppression, y compris \
             dans le compteur en mémoire que lit `target_purge_tick`"
        );
        assert!(target.exists(), "en observe, rien n'est supprimé");
        assert!(target.join("debug/deps/libfoo.rlib").exists());

        let events = db.get_audit_events("session-2497-v5").await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.tool_name == TARGET_PURGE_WOULD_DISPOSE_TOOL),
            "une ligne `target_purge_would_dispose` doit exister"
        );
        assert!(
            !events.iter().any(|e| e.tool_name == TARGET_PURGED_TOOL),
            "en observe, aucune ligne `target_purged` ne doit être écrite"
        );
    }

    /// **AC6** — le kill-switch désarme le bras entier : ni purge, ni refus, ni
    /// ligne. Un bras désarmé n'écrit rien, il ne « refuse » pas.
    #[tokio::test]
    async fn mika2497_le_kill_switch_desarme_le_bras_entier() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let target = fake_target(&wt);
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2497-off",
            "trace-off",
            &[pr_open_refusal(&wt, "fix/2497/x")],
            &index(vec![open_pr(2497, "fix/2497/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig {
                enabled: false,
                ..TargetPurgeConfig::default()
            },
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats, TargetPurgeStats::default(), "aucun compte ne bouge");
        assert!(target.exists());
        assert!(
            db.get_audit_events("session-2497-off")
                .await
                .unwrap()
                .is_empty()
        );
    }

    // -- V6 : disjonction avec mika#2420 ------------------------------------

    /// **V6 / AC4** — les deux populations ne s'intersectent pas, et c'est
    /// **par construction** : T4 du faucheur est « aucune PR ouverte », la
    /// population d'ici est « PR ouverte ».
    #[test]
    fn mika2497_v6_les_deux_populations_sont_disjointes() {
        let tmp = tempfile::tempdir().unwrap();
        let vif = fake_worktree(tmp.path(), "fix-2497-vif");
        let mort = fake_worktree(tmp.path(), "fix-2497-mort");
        let (vif_s, mort_s) = (
            vif.to_string_lossy().into_owned(),
            mort.to_string_lossy().into_owned(),
        );

        let entries = [
            entry(&vif_s, Some("fix/2497/vif")),
            entry(&mort_s, Some("fix/2497/mort")),
        ];
        let prs = index(vec![
            open_pr(2497, "fix/2497/vif"),
            merged_pr(2496, "fix/2497/mort", 7200),
        ]);

        let screened = screen_worktrees(
            &entries,
            &prs,
            &no_processes(),
            now(),
            &ReapConfig::default(),
        );

        // Le faucheur retient le worktree **mort**, et lui seul.
        let reaped: Vec<&str> = screened
            .candidates
            .iter()
            .map(|c| c.path.as_str())
            .collect();
        assert_eq!(reaped, vec![mort_s.as_str()]);

        // La purge travaille le worktree **vif**, et lui seul.
        let purge = screen_target_purges(
            &screened.refusals,
            &no_processes(),
            &purge_states(
                &vif,
                TargetState::Present {
                    idle_secs: Some(PURGE_IDLE_DEFAULT_SECS + 600),
                },
            ),
            &TargetPurgeConfig::default(),
        );
        let purged: Vec<&str> = purge
            .candidates
            .iter()
            .map(|c| c.worktree_path.as_str())
            .collect();
        assert_eq!(purged, vec![vif_s.as_str()]);

        // Et l'intersection est vide.
        assert!(
            !reaped.iter().any(|p| purged.contains(p)),
            "les deux bras ne doivent jamais viser le même worktree"
        );
    }

    /// Le vecteur de refus est **porteur** : `apply_work_states` ne produit que
    /// `dirty` / `unpushed_commits`, donc filtrer `selection.refusals` au lieu
    /// de `screened.refusals` rendrait une population vide — un bras qui se lit
    /// comme sain en ne faisant rien (classe mika#2205).
    #[test]
    fn mika2497_le_vecteur_de_refus_est_celui_de_lecran() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2497-x");
        let path = wt.to_string_lossy().into_owned();

        let screened = screen_worktrees(
            &[entry(&path, Some("fix/2497/x"))],
            &index(vec![open_pr(2497, "fix/2497/x")]),
            &no_processes(),
            now(),
            &ReapConfig::default(),
        );
        assert_eq!(only_reason(&screened), vec![REASON_PR_OPEN]);

        let after_t7 = apply_work_states(screened.candidates, &HashMap::new());
        assert!(
            !after_t7.refusals.iter().any(|r| r.reason == REASON_PR_OPEN),
            "`pr_open` ne transite jamais par `apply_work_states`"
        );
    }

    // -- Réglages ------------------------------------------------------------

    /// **AC6** — le kill-switch a trois paliers, et une valeur non reconnue
    /// **reste armée** : un désarmement par coquille sur un frein de disque
    /// serait la panne silencieuse que tout ceci ferme.
    #[test]
    fn mika2497_le_kill_switch_a_trois_paliers() {
        assert!(parse_purge_enabled(None));
        assert!(parse_purge_enabled(Some("")));
        assert!(parse_purge_enabled(Some("1")));
        assert!(parse_purge_enabled(Some(" TRUE ")));
        assert!(!parse_purge_enabled(Some("0")));
        assert!(!parse_purge_enabled(Some("false")));
        assert!(!parse_purge_enabled(Some(" OFF ")));
        assert!(
            parse_purge_enabled(Some("nope")),
            "une valeur non reconnue laisse la purge armée"
        );
    }

    /// Les défauts sont des valeurs de contrat, d'où les assertions.
    ///
    /// Le budget est **distinct** de celui du faucheur : un budget partagé
    /// ferait manger au faucheur le sien, ou l'inverse.
    #[test]
    fn mika2497_les_defauts_sont_un_contrat() {
        let c = TargetPurgeConfig::default();
        assert!(c.enabled, "livré armé (D8, précédent mika#2272)");
        assert_eq!(c.disposition, Disposition::Armed);
        assert_eq!(c.idle_secs, 14_400, "quatre heures");
        assert_eq!(c.max_per_tick, 2);
        assert_ne!(
            c.max_per_tick,
            ReapConfig::default().max_per_tick,
            "les deux budgets sont distincts, et ce test le dit"
        );
    }

    /// Le `0` **ne désarme pas** une borne numérique — c'est le rôle du
    /// kill-switch, et l'inverse ferait d'une coquille un désarmement
    /// silencieux.
    #[test]
    fn mika2497_un_zero_numerique_ne_desarme_pas() {
        for bad in ["0", "-1", "plif", "  "] {
            assert_eq!(
                parse_positive_i64(Some(bad), PURGE_IDLE_DEFAULT_SECS, PURGE_IDLE_ENV),
                PURGE_IDLE_DEFAULT_SECS
            );
            assert_eq!(
                parse_positive_usize(
                    Some(bad),
                    PURGE_MAX_PER_TICK_DEFAULT,
                    PURGE_MAX_PER_TICK_ENV
                ),
                PURGE_MAX_PER_TICK_DEFAULT
            );
        }
    }

    // -- V7 : les deux détecteurs -------------------------------------------

    /// **AC7 / format de fil.** Les motifs atterrissent dans
    /// `audit_events.after_value` et l'opérateur en fait des `GROUP BY` : deux
    /// orthographes d'un même motif couperaient une population en deux sans le
    /// dire.
    #[test]
    fn mika2497_les_motifs_de_purge_sont_un_format_de_fil() {
        assert_eq!(
            ALL_PURGE_REFUSAL_REASONS,
            &[
                "no_target_dir",
                "target_not_a_dir",
                "live_process",
                "process_scan_unreadable",
                "recently_active",
                "mtime_unreadable",
                "build_lock_held",
                "build_lock_unreadable",
                "outside_managed_root",
                // mika#2511 : **ajout en queue**, jamais un renommage. Aucune
                // population existante ne change de nom ni de sens, et les
                // `GROUP BY` publiés restent exacts. Daté dans CLAUDE.md.
                "build_lock_raced",
            ],
            "renommer un motif est une rupture de format de fil : la dater dans \
             CLAUDE.md, jamais mettre ce test à jour en silence"
        );
        let mut seen = std::collections::HashSet::new();
        for r in ALL_PURGE_REFUSAL_REASONS {
            assert!(seen.insert(*r), "motif dupliqué: {r}");
        }
        // La liste est **distincte** de celle du faucheur : deux populations
        // comptables qui doivent rester soustractibles.
        assert_ne!(
            ALL_PURGE_REFUSAL_REASONS, ALL_REFUSAL_REASONS,
            "les deux listes ne doivent pas fusionner"
        );
    }

    /// Allowlist de la garde SOLE WRITER — **livrée vide, et elle le reste**.
    const TARGET_PURGE_WRITERS_ALLOWED: &[&str] = &[];

    /// Quand la garde tire, on retire le second écrivain — on ne l'allowliste
    /// pas (doctrine mika#2201). Une allowlist née vide est une place où
    /// déposer la prochaine infraction.
    #[test]
    fn mika2497_lallowlist_de_la_garde_est_vide() {
        assert!(
            TARGET_PURGE_WRITERS_ALLOWED.is_empty(),
            "mika#2201 — la résolution est de retirer le second écrivain"
        );
    }

    /// **AC8** — `target_purged` (et sa jumelle d'observation) ont **un seul
    /// écrivain** dans le crate.
    ///
    /// Un test comportemental ne peut pas voir cette classe : un second writer
    /// ne rendrait aucune décision fausse, il rendrait
    /// `SELECT … WHERE tool_name = 'target_purged'` inexacte, en silence.
    ///
    /// La garde porte son **assertion auto-nettoyante** : elle échoue si le nom
    /// n'est écrit **nulle part** dans ce module, parce qu'un scan visant un
    /// nom mort vérifie zéro chose et se lit exactement comme un scan propre
    /// (classe mika#2205).
    #[test]
    fn mika2497_le_nom_de_purge_a_un_seul_ecrivain() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("worktree_reaper.rs");
        // Écrites en deux morceaux pour que la garde ne se dénonce pas elle-même.
        let needles = [
            format!("target{}", "_purged"),
            format!("target_purge_{}", "would_dispose"),
        ];

        // Assertion auto-nettoyante : le nom doit être vivant ici.
        let here = include_str!("worktree_reaper.rs");
        for needle in &needles {
            assert!(
                here.contains(needle.as_str()),
                "`{needle}` n'est écrit nulle part dans ce module — un scan \
                 visant un nom mort ne vérifie rien"
            );
        }

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
                let rel = path
                    .strip_prefix(&src_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                if TARGET_PURGE_WRITERS_ALLOWED.contains(&rel.as_str()) {
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
            "mika#2497 — `target_purged` et `target_purge_would_dispose` sont \
             SOLE WRITER de `worktree_reaper.rs`. Un second writer rendrait la \
             requête opérateur inexacte sans rien casser.\n{}",
            offenders.join("\n")
        );
    }

    /// Le triplet (event, tool_name, message) a **une seule source**, et les
    /// deux messages sont pinés à l'octet près.
    #[test]
    fn mika2497_le_triplet_de_purge_a_une_seule_source() {
        let armed = purge_outcome_for(Disposition::Armed);
        let observe = purge_outcome_for(Disposition::Observe);
        assert_eq!(armed.event, TARGET_PURGED_TOOL);
        assert_eq!(armed.message, TARGET_PURGED_MESSAGE);
        assert_eq!(observe.event, TARGET_PURGE_WOULD_DISPOSE_TOOL);
        assert_eq!(observe.message, TARGET_PURGE_WOULD_DISPOSE_MESSAGE);
        assert_ne!(
            armed.event, observe.event,
            "`observe` ne revendique jamais un retrait (mika#2469)"
        );
        assert!(
            observe.message.contains("non purgé"),
            "le message d'observation doit nier le retrait dans sa propre phrase"
        );
    }

    // =======================================================================
    // mika#2511 — tenir le verrou pendant la suppression, découpler les budgets
    // =======================================================================

    /// Un `.cargo-lock` **tenu**, comme le ferait un `cargo` en cours de build.
    ///
    /// Déterministe et sans course : `flock` porte sur l'*open file
    /// description*, donc deux `open()` du même chemin dans le **même**
    /// processus obtiennent deux OFD distinctes et la seconde acquisition
    /// `LOCK_EX|LOCK_NB` échoue avec `EWOULDBLOCK`. Aucun thread, aucun `fork`,
    /// aucune temporisation.
    #[cfg(target_os = "linux")]
    fn hold_lock_file(path: &Path) -> std::fs::File {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new().read(true).open(path).unwrap();
        // SAFETY: descripteur valide possédé par le test.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(rc, 0, "le test doit pouvoir prendre le verrou");
        file
    }

    #[cfg(target_os = "linux")]
    fn hold_cargo_lock(target: &Path, profile: &str) -> std::fs::File {
        free_cargo_lock(target, profile);
        hold_lock_file(&target.join(profile).join(".cargo-lock"))
    }

    /// Relâche un verrou de test **avant** de fermer le descripteur, et
    /// l'atteste.
    ///
    /// # Pourquoi un `drop` nu ne suffit pas — ce n'est pas une précaution de
    /// # style, c'est le flake mesuré
    ///
    /// `flock(2)` porte sur l'*open file description*, pas sur le descripteur :
    /// la fermeture ne le relâche qu'au **dernier** descripteur qui référence
    /// cette OFD. Or ce binaire de test exécute ses cas en parallèle et le
    /// crate lance des sous-processus à une cinquantaine de sites
    /// (`Command::new`). `std::process::Command` fait `fork` puis `execve` :
    /// Rust ouvre ses fichiers en `O_CLOEXEC`, donc l'enfant perd le
    /// descripteur à l'`exec` — mais **entre le `fork` et l'`exec` il le
    /// partage**, et l'OFD survit alors à la fermeture côté parent pendant
    /// toute cette fenêtre. Un `drop(file)` suivi d'une re-sonde immédiate peut
    /// donc lire `Held` sur un verrou que le test croit avoir relâché, d'autant
    /// plus souvent que la machine est chargée — la forme intermittente
    /// observée en CI, et absente en local.
    ///
    /// `LOCK_UN` n'a pas cette faiblesse : il agit sur l'OFD elle-même, donc
    /// pour tous ses descripteurs à la fois, quel que soit le nombre de
    /// processus qui la partagent à cet instant — et son succès est
    /// **observable**, là où `File::drop` jette le code de retour de `close`.
    /// C'est l'invariant que la production tient déjà à ses deux sites
    /// (`CargoBuildLockGuard`'s `Drop` et `probe_one_cargo_lock`) ; il manquait
    /// aux tests.
    ///
    /// **Ne pas transporter ce helper sur un `CargoBuildLockGuard`** : son
    /// `Drop` fait ce `LOCK_UN` lui-même, et c'est précisément la propriété que
    /// `mika2511_v4_le_garde_tient_reellement_le_verrou` existe pour exercer.
    #[cfg(target_os = "linux")]
    fn release_lock_file(file: std::fs::File) {
        use std::os::fd::AsRawFd;
        // SAFETY: descripteur valide possédé par le test, encore ouvert.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        assert_eq!(rc, 0, "le relâchement du verrou de test doit réussir");
        drop(file);
    }

    /// Un `.cargo-lock` **libre** dans un profil.
    fn free_cargo_lock(target: &Path, profile: &str) {
        let dir = target.join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".cargo-lock"), b"").unwrap();
    }

    /// **V1** — un verrou tenu au moment de l'acquisition rend `Held`.
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2511_v1_lacquisition_rend_held_quand_un_verrou_est_tenu() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        let _held = hold_cargo_lock(&target, "debug");
        assert!(matches!(
            acquire_cargo_build_locks(&target),
            LockAcquisition::Held
        ));
    }

    /// **V2** — aucun `.cargo-lock` du tout : cargo n'a jamais construit ici,
    /// donc l'acquisition réussit. C'est l'absence qui **satisfait** le terme,
    /// jamais celle qui le rend inévaluable (la distinction que
    /// [`LockProbe`] documente et que ce test épingle au second étage).
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2511_v2_lacquisition_reussit_sans_aucun_cargo_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        assert!(matches!(
            acquire_cargo_build_locks(&target),
            LockAcquisition::Acquired(_)
        ));
    }

    /// **V3 / AC3** — un `target/` qu'on ne peut pas énumérer est
    /// **inévaluable**, donc il conserve. La règle de maison, au second étage :
    /// *un signal qu'on ne peut pas lire n'est jamais un terme satisfait.*
    ///
    /// Hors Linux (N5) l'acquisition rend `Unevaluable` par construction — c'est
    /// la branche `#[cfg(not(target_os = "linux"))]`, et la purge n'y fire
    /// jamais, exactement comme le filtre amont aujourd'hui.
    #[test]
    fn mika2511_v3_un_target_illisible_est_inevaluable_donc_conserve() {
        let tmp = tempfile::tempdir().unwrap();
        let absent = tmp.path().join("il-n-y-a-pas-de-target-ici");
        assert!(matches!(
            acquire_cargo_build_locks(&absent),
            LockAcquisition::Unevaluable
        ));
    }

    /// **V4 — le test porteur du ticket.** Le garde tient *réellement* : tant
    /// qu'il vit, une seconde acquisition rend `Held` ; après `drop`, elle
    /// réussit.
    ///
    /// C'est la propriété que la fenêtre TOCTOU laissait ouverte — la sonde
    /// prenait le verrou et le relâchait aussitôt, donc rien n'empêchait un
    /// `cargo` de démarrer entre elle et le `remove_dir_all`.
    ///
    /// **Le `drop(guard)` est ici déterministe, et il doit le rester tel quel :**
    /// l'`impl Drop` de [`CargoBuildLockGuard`] pose un `LOCK_UN` explicite
    /// avant que les descripteurs ne soient fermés, ce qui est exactement la
    /// barrière que [`release_lock_file`] apporte aux verrous *de test*. Le
    /// remplacer par ce helper retirerait au test son objet — que le `Drop` de
    /// production relâche — et le laisserait vert en n'exerçant plus rien.
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2511_v4_le_garde_tient_reellement_le_verrou() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        free_cargo_lock(&target, "debug");

        let guard = match acquire_cargo_build_locks(&target) {
            LockAcquisition::Acquired(g) => g,
            other => panic!("acquisition attendue, obtenu {other:?}"),
        };

        assert!(
            matches!(cargo_build_lock_is_free(&target), LockProbe::Held),
            "tant que le garde vit, le filtre amont doit voir le verrou tenu"
        );
        assert!(
            matches!(acquire_cargo_build_locks(&target), LockAcquisition::Held),
            "tant que le garde vit, une seconde acquisition doit échouer"
        );

        drop(guard);

        assert!(
            matches!(cargo_build_lock_is_free(&target), LockProbe::Free),
            "après `drop`, le verrou doit être relâché"
        );
        assert!(matches!(
            acquire_cargo_build_locks(&target),
            LockAcquisition::Acquired(_)
        ));
    }

    /// **V5** — un verrou dans un profil `release` est vu. Deviner
    /// `target/debug/.cargo-lock` raterait un build `--release`, c'est-à-dire
    /// échouerait exactement sur le cas qu'on veut voir.
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2511_v5_un_verrou_hors_debug_est_vu() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        free_cargo_lock(&target, "debug");
        let _held = hold_cargo_lock(&target, "release");
        assert!(matches!(
            acquire_cargo_build_locks(&target),
            LockAcquisition::Held
        ));
    }

    /// **V6** — une acquisition annulée par un verrou tenu ne laisse **aucun
    /// verrou orphelin** : les descripteurs déjà acquis sont relâchés par le
    /// `Drop` du `Vec` partiel.
    ///
    /// **L'ordre de `read_dir` n'est pas garanti, donc on le *lit* au lieu de le
    /// supposer** : le profil tenu est le **dernier** de l'énumération réelle,
    /// ce qui garantit que le premier a bien été acquis puis annulé. Sans cette
    /// lecture, le test serait muet sur les systèmes de fichiers où le profil
    /// bloquant sort en tête — vert sans rien avoir exercé (classe mika#2205).
    ///
    /// Un compte de descripteurs sur `/proc/self/fd` serait **faux** ici : il
    /// est par processus, et les tests de ce binaire tournent en parallèle.
    #[cfg(target_os = "linux")]
    #[test]
    fn mika2511_v6_une_acquisition_annulee_ne_laisse_aucun_verrou_orphelin() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        for p in ["p1", "p2", "p3", "p4"] {
            free_cargo_lock(&target, p);
        }

        let (locks, partial) = cargo_lock_paths(&target).expect("énumération lisible");
        assert!(!partial);
        assert!(
            locks.len() >= 2,
            "le scénario a besoin d'au moins deux profils"
        );
        let acquired_first = locks.first().unwrap().clone();
        let blocker = locks.last().unwrap().clone();

        let held = hold_lock_file(&blocker);
        assert!(matches!(
            acquire_cargo_build_locks(&target),
            LockAcquisition::Held
        ));

        assert!(
            matches!(probe_one_cargo_lock(&acquired_first), LockProbe::Free),
            "le premier profil a été acquis puis l'acquisition a été annulée : \
             son verrou doit avoir été relâché"
        );
        release_lock_file(held);
    }

    /// **V7 / AC2** — bout en bout : un `target/` dont le verrou est tenu n'est
    /// **pas** supprimé, et un refus est écrit.
    ///
    /// Le motif observé ici est `build_lock_held`, celui du **filtre amont** :
    /// `purge_stale_target_dirs` sonde elle-même avant de disposer, donc un
    /// verrou pris *avant* l'appel est intercepté là et la re-sonde n'est jamais
    /// atteinte. Ce test atteste donc le **conservatisme** du premier étage ;
    /// le second — `build_lock_raced`, le motif que ce ticket ajoute — est
    /// exercé par
    /// [`mika2511_v7b_une_divergence_filtre_acquisition_ecrit_build_lock_raced`].
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn mika2511_v7_un_target_verrouille_nest_pas_supprime() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        let _held = hold_cargo_lock(&target, "debug");
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2511-v7",
            "trace-v7",
            &[pr_open_refusal(&wt, "fix/2511/x")],
            &index(vec![open_pr(2511, "fix/2511/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig::default(),
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.purged, 0, "aucune suppression sous un verrou tenu");
        assert_eq!(stats.would_purge, 0);
        assert!(target.exists(), "`target/` doit être intact");
        assert!(target.join("debug/deps/libfoo.rlib").exists());
        assert_eq!(stats.refused, 1, "le refus doit être écrit");
        assert_eq!(budget, 2, "un refus n'écrit rien : il ne doit rien débiter");

        let events = db.get_audit_events("session-2511-v7").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == TARGET_PURGE_SKIPPED_TOOL)
            .expect("une ligne de refus doit exister");
        assert_eq!(
            row.after_value.as_deref(),
            Some(PURGE_REASON_BUILD_LOCK_HELD)
        );
    }

    /// **V7b / AC2 — le motif `build_lock_raced` est EXERCÉ**, pas seulement
    /// déclaré : le filtre amont rend `Free`, l'acquisition rend `Held`, et la
    /// disposition conserve l'arbre en écrivant **ce** motif, distinct de
    /// `build_lock_held`.
    ///
    /// # Le dispositif, et pourquoi il ne peut pas être « tenir le verrou entre
    /// # les deux étages »
    ///
    /// Les deux étages vivent **dans** `purge_stale_target_dirs` : elle sonde
    /// tous les candidats, fige la sélection, puis dispose. Aucun code de test
    /// ne s'exécute entre les deux. Un fil qui prendrait le verrou pendant la
    /// disposition du candidat précédent serait une **course**, donc un test
    /// intermittent — exactement ce que le commentaire opérateur de mika#2511
    /// demande d'éliminer après le flake de `mika2497_v3b`.
    ///
    /// Le dispositif retenu fait diverger les deux étages **sans horloge** :
    /// deux `.cargo-lock` de profils distincts pointant sur **un seul inode**
    /// (lien physique). `flock(2)` porte sur l'*open file description*, donc
    ///
    /// - le **filtre** relâche après chaque sonde ([`probe_one_cargo_lock`] fait
    ///   son `LOCK_UN`), donc les deux sondes réussissent ⇒ `Free` ;
    /// - l'**acquisition** retient cumulativement, donc la seconde ouverture
    ///   entre en collision avec la première ⇒ `EWOULDBLOCK` ⇒ `Held`.
    ///
    /// Déterministe, indépendant de l'ordre de `read_dir` (quel que soit le
    /// profil énuméré en premier, c'est le second qui collisionne), et bâti sur
    /// la propriété que [`hold_lock_file`] documente déjà pour les verrous de
    /// test.
    ///
    /// # Ce qui est artificiel, et ce qui ne l'est pas
    ///
    /// Le lien physique est un **artifice** : en production la divergence vient
    /// du temps qui passe — un `cargo` démarré entre la sonde et la suppression,
    /// ce que R1 du plan situe à ~2 s pour le premier candidat d'un tick et à
    /// une suppression entière pour le second. Ce qui est exercé, en revanche,
    /// est le **vrai chemin** : la même fonction de production, le même bras
    /// `LockAcquisition::Held`, la même écriture de refus, le même budget.
    ///
    /// # Dépendance à connaître avant d'y toucher
    ///
    /// Le dispositif tient parce que [`cargo_lock_paths`] rend **un chemin par
    /// entrée de répertoire**, jamais un par inode. Si quelqu'un dédoublonne cet
    /// énumérateur par inode, ce test rougit : la résolution est de lui trouver
    /// un autre dispositif, **jamais** de retirer l'assertion — le motif
    /// `build_lock_raced` redeviendrait alors déclaré et non exercé.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn mika2511_v7b_une_divergence_filtre_acquisition_ecrit_build_lock_raced() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-raced");
        let target = fake_target(&wt);

        free_cargo_lock(&target, "debug");
        std::fs::create_dir_all(target.join("release")).unwrap();
        std::fs::hard_link(
            target.join("debug/.cargo-lock"),
            target.join("release/.cargo-lock"),
        )
        .unwrap();
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        // Préconditions assertées : sans elles, un échec plus bas serait
        // ambigu entre « le dispositif ne diverge plus » et « la disposition
        // écrit le mauvais motif ».
        assert!(
            matches!(cargo_build_lock_is_free(&target), LockProbe::Free),
            "le filtre amont doit laisser passer — sinon c'est `build_lock_held` \
             qui serait écrit, et le second étage ne serait pas atteint"
        );
        assert!(
            matches!(acquire_cargo_build_locks(&target), LockAcquisition::Held),
            "l'acquisition doit diverger du filtre — c'est tout le dispositif"
        );

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2511-v7b",
            "trace-v7b",
            &[pr_open_refusal(&wt, "fix/2511/raced")],
            &index(vec![open_pr(2511, "fix/2511/raced")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig::default(),
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.purged, 0, "la course conserve, elle ne supprime pas");
        assert_eq!(stats.would_purge, 0);
        assert!(target.exists(), "`target/` doit être intact");
        assert!(target.join("debug/deps/libfoo.rlib").exists());
        assert_eq!(stats.refused, 1, "un refus, et un seul");
        assert_eq!(budget, 2, "un refus tardif n'écrit rien : il ne débite pas");

        let events = db.get_audit_events("session-2511-v7b").await.unwrap();
        let row = events
            .iter()
            .find(|e| e.tool_name == TARGET_PURGE_SKIPPED_TOOL)
            .expect("une ligne de refus doit exister");
        assert_eq!(
            row.after_value.as_deref(),
            Some(PURGE_REASON_BUILD_LOCK_RACED),
            "le motif doit être celui de la course, jamais `{}` — fusionner les \
             deux populations rendrait incomptable la mesure de la fenêtre que \
             mika#2511 ferme",
            PURGE_REASON_BUILD_LOCK_HELD
        );
    }

    /// **V8 / AC4 + AC5** — [`should_stop_repo_loop`] à ses quatre coins,
    /// kill-switch inclus.
    #[test]
    fn mika2511_v8_la_boucle_des_depots_ne_casse_que_sur_les_deux_bras() {
        // Le faucheur est épuisé mais la purge a du budget : **continuer** —
        // c'est le bloquant (b), et c'est aussi ce qui rend `probe_main_checkout`
        // (mika#2449) aux dépôts suivants.
        assert!(!should_stop_repo_loop(0, 2, true));
        // Le faucheur a du budget : continuer, quel que soit l'état de la purge.
        assert!(!should_stop_repo_loop(3, 0, true));
        assert!(!should_stop_repo_loop(3, 0, false));
        assert!(!should_stop_repo_loop(3, 2, false));
        // Les deux épuisés : cesser.
        assert!(should_stop_repo_loop(0, 0, true));
        // Faucheur épuisé + purge désarmée : cesser — B4, sans quoi un bras
        // désarmé garderait la boucle vivante pour rien.
        assert!(should_stop_repo_loop(0, 2, false));
        assert!(should_stop_repo_loop(0, 0, false));
    }

    /// **V9 / AC6 + AC7** — en `observe`, `would_purge` compte et `purged` reste
    /// à zéro.
    ///
    /// Corrige [`mika2497_v5_observe_ne_supprime_rien`], dont l'assertion
    /// `stats.purged == 1` **figeait le défaut**. C'est une correction, pas un
    /// assouplissement : la moitié durable de ce test (`target_purge_would_dispose`
    /// écrit, `target_purged` absent) est inchangée et reste l'assertion
    /// porteuse.
    #[tokio::test]
    async fn mika2511_v9_observe_compte_would_purge_et_pas_purged() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2511-v9",
            "trace-v9",
            &[pr_open_refusal(&wt, "fix/2511/x")],
            &index(vec![open_pr(2511, "fix/2511/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig {
                disposition: Disposition::Observe,
                ..TargetPurgeConfig::default()
            },
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.would_purge, 1, "la détection est inconditionnelle");
        assert_eq!(
            stats.purged, 0,
            "`observe` ne supprime rien, donc n'en compte aucune"
        );
        assert!(target.exists());
    }

    /// **V10** — contrôle négatif de V9 : en `armed`, c'est l'inverse.
    ///
    /// Sans lui, « la dérivation suit la disposition » serait indistinguable de
    /// « la dérivation suit autre chose qui vaut zéro ».
    #[tokio::test]
    async fn mika2511_v10_armed_compte_purged_et_pas_would_purge() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = fake_worktree(tmp.path(), "fix-2511-x");
        let target = fake_target(&wt);
        age_tree(&target, PURGE_IDLE_DEFAULT_SECS as u64 + 600);

        let db = AsyncDatabase::new(crate::db::Database::open_in_memory().unwrap());
        let mut budget = 2usize;
        let mut stats = TargetPurgeStats::default();

        purge_stale_target_dirs(
            &db,
            "session-2511-v10",
            "trace-v10",
            &[pr_open_refusal(&wt, "fix/2511/x")],
            &index(vec![open_pr(2511, "fix/2511/x")]),
            &no_processes(),
            now(),
            &TargetPurgeConfig::default(),
            &mut budget,
            &mut stats,
        )
        .await;

        assert_eq!(stats.purged, 1);
        assert_eq!(
            stats.would_purge, 0,
            "`armed` ne compte aucun « aurait purgé »"
        );
        assert!(!target.exists());
    }

    // -- § 6 : le scan structurel -------------------------------------------

    /// Allowlist du scan — **livrée vide, et elle le reste**.
    const REMOVE_DIR_ALL_SITES_ALLOWED: &[&str] = &[];

    /// Doctrine mika#2201 : quand le scan tire, **on rend le site conforme, on
    /// ne l'allowliste pas**. Une allowlist née vide est une place où déposer la
    /// prochaine infraction.
    #[test]
    fn mika2511_lallowlist_du_scan_est_vide() {
        assert!(
            REMOVE_DIR_ALL_SITES_ALLOWED.is_empty(),
            "mika#2201 — la résolution est de rendre le site conforme"
        );
    }

    /// Le scan du § 6, **isolé pour être vu rouge sur une fixture**.
    ///
    /// Rend `Ok(nombre de fonctions portant une suppression)` ou la liste des
    /// fonctions fautives. Les lignes de commentaire sont retirées avant
    /// l'analyse, sur le motif mesuré de mika#2050 : la prose de ce fichier cite
    /// les jetons qu'elle décrit.
    ///
    /// # Deux termes, pas un
    ///
    /// 1. l'acquisition **précède** la suppression, dans la même fonction ;
    /// 2. **aucun `drop(` entre les deux**.
    ///
    /// Le second n'est pas une redondance du premier : un garde relâché avant la
    /// suppression rouvre la fenêtre TOCTOU **sans déplacer l'acquisition d'une
    /// ligne**, donc le terme 1 seul laisserait passer la régression la plus
    /// plausible — celle d'un relecteur qui « range » un `drop` explicite plus
    /// haut pour rendre la portée plus étroite. Le prédicat est délibérément
    /// large (tout `drop(`, pas seulement celui du garde) : la seule chose qu'on
    /// ait légitimement à relâcher là est le garde, et la résolution quand il
    /// tire est de déplacer le `drop` **après** la suppression, jamais
    /// d'assouplir le terme.
    fn scan_remove_dir_all_sites(src: &str, allowed: &[&str]) -> Result<usize, Vec<String>> {
        let removal = format!("remove_dir{}", "_all");
        let acquisition = format!("acquire_cargo{}", "_build_locks");

        let lines: Vec<&str> = src
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("//") {
                    ""
                } else {
                    l
                }
            })
            .collect();
        let code = lines.join("\n");

        let mut starts: Vec<(usize, String)> = Vec::new();
        let mut offset = 0usize;
        for line in &lines {
            let t = line.trim_start();
            let is_fn = ["fn ", "pub fn ", "async fn ", "pub async fn "]
                .iter()
                .any(|p| t.starts_with(p))
                || (t.starts_with("pub(") && t.contains(") fn "))
                || (t.starts_with("pub(") && t.contains(") async fn "));
            if is_fn {
                let name = t
                    .split("fn ")
                    .nth(1)
                    .unwrap_or(t)
                    .split(['(', '<'])
                    .next()
                    .unwrap_or("?")
                    .trim()
                    .to_string();
                starts.push((offset, name));
            }
            offset += line.len() + 1;
        }

        let mut found = 0usize;
        let mut offenders = Vec::new();
        for (i, (start, name)) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map_or(code.len(), |(s, _)| *s);
            let body = &code[*start..end];
            let Some(rm) = body.find(removal.as_str()) else {
                continue;
            };
            found += 1;
            if allowed.contains(&name.as_str()) {
                continue;
            }
            match body.find(acquisition.as_str()) {
                // L'acquisition précède — reste à vérifier qu'elle tient
                // toujours au moment de la suppression.
                Some(acq) if acq < rm => {
                    if body[acq..rm].contains("drop(") {
                        offenders.push(name.clone());
                    }
                }
                _ => offenders.push(name.clone()),
            }
        }
        if offenders.is_empty() {
            Ok(found)
        } else {
            Err(offenders)
        }
    }

    /// **AC10 / § 6** — toute suppression est précédée, **dans la même
    /// fonction**, d'une acquisition du verrou de build.
    ///
    /// Pourquoi un scan et pas un test comportemental : retirer l'acquisition ne
    /// rend **aucune décision fausse** le jour où on l'écrit — la purge continue
    /// de purger, V1-V10 restent verts, et seule la fenêtre se rouvre, en
    /// silence. C'est la classe exacte que
    /// `mika2342_every_llm_call_is_wrapped_in_a_timeout` a dû fermer par un
    /// scan, avec la même phrase.
    ///
    /// **Contrôle de non-vacuité** : le scan échoue si la suppression n'est
    /// écrite nulle part en position exécutable — un scan visant un jeton mort
    /// se lirait exactement comme un scan propre (mika#2496 U4, classe
    /// mika#2205).
    #[test]
    fn mika2511_toute_suppression_est_precedee_de_lacquisition() {
        let here = include_str!("worktree_reaper.rs");
        let production = here
            .split("#[cfg(test)]")
            .next()
            .expect("le module porte un `mod tests`");
        assert!(
            production.len() < here.len(),
            "le module de test doit être tronqué — sinon les fixtures du scan \
             seraient lues comme de la production"
        );

        match scan_remove_dir_all_sites(production, REMOVE_DIR_ALL_SITES_ALLOWED) {
            Ok(found) => assert!(
                found >= 1,
                "contrôle de non-vacuité : aucune suppression en position \
                 exécutable — le scan ne vérifie plus rien"
            ),
            Err(offenders) => panic!(
                "mika#2511 — une suppression n'est pas couverte par une \
                 acquisition du verrou de build **tenue jusqu'à elle**, dans: \
                 {}. Soit l'acquisition manque ou la suit, soit un `drop(` la \
                 relâche entre les deux. La fenêtre TOCTOU est rouverte ; la \
                 résolution est de rendre le site conforme (relâcher APRÈS la \
                 suppression), jamais de l'allowlister.",
                offenders.join(", ")
            ),
        }
    }

    /// **Contrôle négatif du scan, vu rouge.** Sans lui, « le scan lit la
    /// séquence » est indistinguable de « le scan ne lit rien ».
    #[test]
    fn mika2511_le_scan_est_vu_rouge_sur_une_suppression_non_gardee() {
        let fixture = concat!(
            "fn purge_quelque_chose(t: &Path) {\n",
            "    let _ = mesure(t);\n",
            "    let _ = std::fs::remove_dir",
            "_all(t);\n",
            "}\n"
        );
        assert_eq!(
            scan_remove_dir_all_sites(fixture, &[]),
            Err(vec!["purge_quelque_chose".to_string()])
        );
    }

    /// **Contrôle négatif miroir, vu vert.** Sans lui, « le scan lit la
    /// séquence » est indistinguable de « le scan rougit sur toute
    /// suppression ».
    #[test]
    fn mika2511_le_scan_est_vert_sur_une_suppression_gardee() {
        let fixture = concat!(
            "fn purge_quelque_chose(t: &Path) {\n",
            "    let g = acquire_cargo",
            "_build_locks(t);\n",
            "    let _ = std::fs::remove_dir",
            "_all(t);\n",
            "    drop(g);\n",
            "}\n"
        );
        assert_eq!(scan_remove_dir_all_sites(fixture, &[]), Ok(1));
    }

    /// L'ordre compte : une acquisition **après** la suppression ne protège
    /// rien, et le scan doit le dire.
    #[test]
    fn mika2511_le_scan_rougit_si_lacquisition_suit_la_suppression() {
        let fixture = concat!(
            "fn purge_quelque_chose(t: &Path) {\n",
            "    let _ = std::fs::remove_dir",
            "_all(t);\n",
            "    let g = acquire_cargo",
            "_build_locks(t);\n",
            "}\n"
        );
        assert!(scan_remove_dir_all_sites(fixture, &[]).is_err());
    }

    /// **Contrôle négatif du second terme, vu rouge.** L'acquisition précède
    /// bien la suppression — et ne protège rien, parce que le garde est relâché
    /// avant. Sans ce contrôle, « le scan lit la séquence » serait
    /// indistinguable de « le scan lit seulement l'ordre des deux appels », et
    /// la régression la plus plausible passerait avec tous les tests au vert.
    #[test]
    fn mika2511_le_scan_rougit_si_le_garde_est_relache_avant_la_suppression() {
        let fixture = concat!(
            "fn purge_quelque_chose(t: &Path) {\n",
            "    let g = acquire_cargo",
            "_build_locks(t);\n",
            "    drop(g);\n",
            "    let _ = std::fs::remove_dir",
            "_all(t);\n",
            "}\n"
        );
        assert_eq!(
            scan_remove_dir_all_sites(fixture, &[]),
            Err(vec!["purge_quelque_chose".to_string()])
        );
    }

    /// Les clés d'audit sont préfixées `target:` — distinctes de celles du
    /// faucheur (`worktree:`), pour que les deux populations restent
    /// soustractibles.
    #[test]
    fn mika2497_les_cles_daudit_ne_collisionnent_pas_avec_le_faucheur() {
        let p = purged_audit_key("/x/.claude/worktrees/a/mika/target");
        assert!(p.starts_with("target:"));
        assert!(!p.starts_with("worktree:"));
        assert_eq!(
            purge_refusal_audit_key("/x/a", PURGE_REASON_RECENTLY_ACTIVE),
            "target:/x/a@recently_active"
        );
    }
}

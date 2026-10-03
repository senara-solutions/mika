//! Relance une fois, puis blocage (mika#2617 U3 / AC2).
//!
//! Un check rouge sur la tête d'une PR déclenche **une** relance automatique des
//! jobs échoués de son run. S'il repasse au vert, la porte se ré-évalue par le
//! rendez-vous `check_suite.completed` qui existe déjà (motif mika#2238), jamais
//! par une boucle d'attente. S'il échoue deux fois, ce n'est pas un flaky, c'est
//! cassé, et la porte reste fermée.
//!
//! Prime, verbatim sur le bearing du 2026-10-02 : *« La relance-une-fois
//! attaque la vraie nature du flaky sans jamais merger sur du rouge : si le
//! re-run passe c'est vert pour de bon, s'il échoue deux fois ce n'est pas
//! flaky, c'est cassé, et ça doit bloquer. »*
//!
//! ## Ce que ce module NE fait pas
//!
//! Il ne décide **rien** sur le merge. La porte est déjà fermée quand il est
//! appelé — c'est la phase A (`MergeGateDecision::ChecksFailed`) qui refuse, et
//! son refus n'est pas conditionné à ce qui se passe ici. Ce module a un effet
//! de bord (il relance) et rend un [`RerunOutcome`] que l'appelant **cite** dans
//! le motif qu'il expose. Un `RerunOutcome` n'ouvre aucune porte.
//!
//! ## AC2 est livrée FAIL-SAFE et n'est pas revendiquée tenue (plan R3)
//!
//! `gh run rerun --failed` demande le scope `actions: write`. Les scopes
//! documentés du jeton de la plateforme sont « Pull requests R/W, Issues R/W,
//! Contents R » (racine `CLAUDE.md`, `MIKA_GITHUB_TOKEN`) — `actions` n'y figure
//! pas. **AC2 peut donc être inerte au déploiement, en 403.** La relance échoue,
//! la porte reste fermée, le motif est nommé ([`RerunOutcome::Refused`]), et
//! c'est le bon état. Elle ne sera revendiquée tenue qu'après qu'une sonde
//! opérateur ait observé une relance réussie — annoncer la clôture avant serait
//! la garantie que mika#2304 nomme : un champ qui affirme, avec autorité,
//! l'override qui n'a pas eu lieu.
//!
//! ## Le ledger est DURABLE, et la clé porte la tête
//!
//! Un ledger en mémoire ne tient pas « jamais de seconde relance » à travers un
//! redémarrage. Le budget vit donc dans `audit_events` sous
//! [`MERGE_GATE_RERUN_AUDIT_TOOL`], clé
//! `rerun:{repo}#{pr}@{head_sha}:{run_id}`, lue par
//! `count_recent_audit_events_for_target` par **égalité exacte** (motif
//! mika#1869 / mika#2347). Deux propriétés en découlent :
//!
//! - **Un nouveau sha rouvre le budget de lui-même** — nouveau code, nouvelle
//!   chance. L'idempotence « par (PR, tête, run) » est obtenue par la *forme*
//!   de la clé, pas par une colonne de plus.
//! - Le piège `#234` ↔ `#2343` de mika#2347 ne s'ouvre pas ici, la comparaison
//!   étant une égalité et non un `LIKE`. Le `@` reste écrit pour qu'un futur
//!   lecteur de préfixe soit sûr **par construction** plutôt que par chance.
//!
//! ## Fail-CLOSED sur le ledger — l'inverse du reste de ce travail
//!
//! L'arbitrage est local et ne se transporte pas. Un faux « déjà relancé »
//! coûte une relance perdue sur une PR qui attend de toute façon un humain ; un
//! faux « jamais relancé » relance en boucle, ce qu'AC2 interdit nommément.
//! Donc : base illisible, `head_sha` vide, ou écriture du ledger en échec ⇒
//! aucune relance.
//!
//! ## La ligne de ledger est écrite AVANT la relance, et c'est porteur
//!
//! `audit_events` est append-only : une ligne ne se révise pas. Si la ligne
//! était écrite **après** la relance, un crash entre les deux rouvrirait le
//! budget et la relance rejouerait — très exactement ce qu'AC2 interdit. Elle
//! est donc écrite d'abord, et porte `attempted` : à cet instant l'issue n'est
//! pas connue, et la nommer serait inventer. L'issue vit sur la **ligne de
//! journal** (champ `outcome`), que le § 6 du plan grepe déjà.
//!
//! Conséquence assumée : une relance refusée en 403 **consomme** le budget de
//! cette tête. C'est la lettre de T8 du plan (« aucune boucle »), et un nouveau
//! sha rouvre le budget.
//!
//! ## Le compte-puis-écriture n'est pas atomique, et ce qui le rend sans effet
//!
//! Deux termes, et il faut les deux — nommés ici plutôt que découverts, parce
//! que ce sont des propriétés **d'ailleurs** dont ce module dépend :
//!
//! - **Intra-agent.** La file webhook bornée (mika#1870) a **un seul worker de
//!   drain par agent**, qui prend `agent_lock` : les tours d'un même agent sont
//!   sérialisés, donc deux lectures du ledger ne peuvent pas s'entrelacer. Sans
//!   cette sérialisation, les huit `check_suite.completed` d'un même push
//!   pourraient tous lire `0` et tous relancer.
//! - **Inter-agent.** `audit_events` est scopé par `agent_id`, donc deux agents
//!   ont deux budgets. Ce qui rend ça sans conséquence est la porte d'entrée
//!   mika#2260 : **seul le dispatcher évalue** `ci_success_handler`, et les deux
//!   autres sites tournent sous la même identité.
//!
//! Si l'un des deux cessait d'être vrai, le remède ne serait pas une fenêtre
//! plus longue mais une écriture conditionnelle (un `INSERT … WHERE NOT
//! EXISTS`), c'est-à-dire une méthode de base neuve — à décider alors, avec la
//! mesure qui l'aura montrée.
//!
//! ## Pourquoi `AlreadySpent` n'écrit AUCUNE ligne d'audit
//!
//! Le plan annonçait « deux échecs ⇒ `merge_gate_rerun_exhausted` (WARN + ligne
//! d'audit) ». La ligne d'audit est **écartée**, et la raison est mesurable : un
//! seul push produit jusqu'à huit `check_suite.completed` (mika#1869), donc une
//! ligne par *évaluation* d'une tête déjà relancée serait jusqu'à huit lignes
//! par push sur une population qui ne change pas — le churn que la doctrine
//! mika#2131 borne — **et** elle polluerait le compte qui tient l'invariant
//! « jamais deux fois ». L'information durable (« cette tête a eu sa relance »)
//! est déjà la ligne de tentative ; l'épuisement est son corollaire, pas un fait
//! neuf. Le WARN, lui, reste : la vivacité est ce que l'opérateur lit.

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::check_link::run_id_from_link;
use crate::tools::pr_merge_with_gate::GhCheck;

/// `tool_name` de la ligne de ledger, et **SOLE WRITER** : ce module est le seul
/// site qui l'écrive, dans le journal comme dans `audit_events`.
///
/// Même convention que `QA_CI_COHERENCE_AUDIT_TOOL` et
/// `DESTRUCTIVE_ACTION_AUDIT_TOOL` : `audit_events` n'a pas de colonne
/// `event_type`, `tool_name` est du TEXT libre, aucune migration.
pub const MERGE_GATE_RERUN_AUDIT_TOOL: &str = "merge_gate_check_rerun";

/// Variable de désarmement de la **relance**, et d'elle seule.
pub const MERGE_GATE_RERUN_ENV: &str = "MIKA_MERGE_GATE_RERUN";

/// `after_value` de la ligne de ledger.
///
/// **Format de fil** : la valeur est lue par `GROUP BY` sur `audit_events`. Une
/// seule valeur, parce qu'une ligne est une *tentative* — voir la doc du module
/// pour pourquoi l'issue ne peut pas y vivre.
pub const MERGE_GATE_RERUN_ATTEMPTED: &str = "attempted";

/// Plafond de temps sur `gh run rerun` (plan U3).
///
/// `run_gh_subprocess` **ne porte aucun plafond propre** : il `spawn` puis
/// `wait()` sans borne. Celui-ci est donc le seul, et il n'en empile pas un
/// second. 30 s, la valeur que `ci_failure_handler` donne déjà à ses propres
/// appels `gh run view`, parce que c'est le même binaire sur le même point de
/// terminaison — un nombre neuf serait un nombre de plus à justifier.
pub const RERUN_TIMEOUT_SECS: u64 = 30;

/// Fenêtre de lecture du ledger, en jours.
///
/// Sept, et la borne haute n'est pas un choix : `compact_old_audit_events`
/// purge à **90 jours** (`evidence::audit::AUDIT_RETENTION_DAYS`), donc une
/// fenêtre au-delà lirait des lignes qui n'existent plus. Sept est très
/// au-dessus de la durée de vie d'une tête de PR — un `head_sha` qui vit sept
/// jours est une PR que personne ne pousse — donc la fenêtre ne peut pas
/// expirer *pendant* la vie du budget qu'elle borne.
pub const RERUN_LEDGER_WINDOW_DAYS: i64 = 7;

/// Ce que la relance a fait, ou n'a pas fait.
///
/// Le `match` de son lecteur est **exhaustif, sans bras `_ =>`** : une issue
/// ajoutée demain ne peut pas tomber dans un défaut silencieux. Le lecteur est
/// unique ([`rerun_detail_suffix`]) pour que les trois sites d'appel ne puissent
/// pas en diverger — la classe que `grooming_marker` a dû fermer (mika#2158).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RerunOutcome {
    /// La relance est partie. Le run est nommé pour que la ligne de refus
    /// pointe ce qu'un opérateur peut aller regarder.
    Triggered { run_id: u64 },
    /// Cette tête a déjà eu sa relance. **Jamais une seconde.**
    AlreadySpent { run_id: u64 },
    /// Aucun run GitHub Actions n'est dérivable des checks rouges : lien absent,
    /// check externe, lien de run sans `/job/`, ou `run_id` non numérique.
    /// Les quatre appellent la même conduite — voir
    /// [`crate::check_link::run_id_from_link`].
    NoActionsRun,
    /// Le budget n'est pas interrogeable : base en échec, ou `head_sha` vide
    /// (donc clé non constructible). **Fail-closed** : aucune relance.
    ///
    /// Les deux causes partagent ce nom parce qu'elles partagent la conduite
    /// opérateur — *établir pourquoi le signal est illisible* — et la cause
    /// précise vit sur le champ `reason` de la ligne de journal. Motif
    /// `ready_label_outcome` (mika#2323) : un nom, la cause dans un champ.
    LedgerUnreadable,
    /// `gh run rerun` a échoué. Le 403 du scope `actions: write` manquant est la
    /// cause la plus probable au déploiement (plan R3).
    Refused(String),
    /// [`MERGE_GATE_RERUN_ENV`] désarme la relance sur ce process.
    Disarmed,
}

/// La relance est-elle armée ?
///
/// **Polarité de `MIKA_QA_CI_COHERENCE_GATE` (mika#2455), elle-même celle de
/// `MIKA_TELEGRAM_HTML_RENDER` (mika#2291).** Armée par défaut : rend `true` sur
/// `None`, sur vide **et sur toute valeur non reconnue** ; rend `false` sur le
/// seul `0` / `false` / `off` / `no` explicite (insensible à la casse, espaces
/// tolérés). La valeur fautive est nommée **entre guillemets** — sans eux un
/// espace parasite est invisible (mika#2220).
///
/// Un désarmement par coquille serait ici l'inverse d'une panne silencieuse
/// (la relance ne partirait pas, la porte resterait fermée), donc moins grave
/// que sur un gate de sûreté — mais la polarité reste la même pour que les deux
/// variables ne se lisent pas différemment côte à côte dans un
/// `EnvironmentFile`.
pub fn merge_gate_rerun_is_enabled(raw: Option<&str>) -> bool {
    let Some(value) = raw.map(str::trim) else {
        return true;
    };
    if value.is_empty() {
        return true;
    }
    match value.to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => false,
        "1" | "true" | "on" | "yes" => true,
        _ => {
            warn!(
                event = "merge_gate_rerun_unrecognized_value",
                value = %format!("{value:?}"),
                "mika#2617: MIKA_MERGE_GATE_RERUN porte une valeur non reconnue — la relance \
                 reste ARMÉE (le défaut). Utiliser 0/false/off/no pour la désarmer."
            );
            true
        }
    }
}

/// Résolution unique par process, mise en cache.
///
/// Lue une fois : poser ou retirer la variable sur un process déjà démarré n'a
/// aucun effet, par construction — même contrat que `MIKA_AGENT_TIER` et
/// `MIKA_DEPLOYMENT`.
pub fn merge_gate_rerun_enabled() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        merge_gate_rerun_is_enabled(std::env::var(MERGE_GATE_RERUN_ENV).ok().as_deref())
    })
}

/// La clé de ledger, écrite à **un seul endroit**.
///
/// `rerun:{repo}#{pr}@{head_sha}:{run_id}`. Interrogée par égalité exacte ; le
/// `@` est là pour qu'un futur lecteur de préfixe soit sûr par construction.
pub(crate) fn rerun_ledger_key(repo: &str, pr_number: u64, head_sha: &str, run_id: u64) -> String {
    format!("rerun:{repo}#{pr_number}@{head_sha}:{run_id}")
}

/// Le run à relancer : le **premier** dérivable d'un lien de check rouge.
///
/// Fonction pure, donc le choix de la cible est testable sans réseau. Un seul
/// run suffit : N lints rouges du même run `ci.yml` sont couverts par **une**
/// relance — c'est le gain de la forme `gh run rerun <run> --failed` sur
/// `--job`, qui en demanderait N (plan R1).
///
/// Un check qui n'est pas un run Actions est simplement ignoré : la fonction
/// continue sur le suivant plutôt que de renoncer, pour qu'un check externe
/// rouge posé en tête de liste ne rende pas la relance inerte.
pub(crate) fn rerun_run_id(checks: &[GhCheck]) -> Option<u64> {
    red_checks(checks).find_map(|c| run_id_from_link(c.link.as_deref()))
}

/// Les checks rouges — **le seul site de ce module où « rouge » est écrit**.
///
/// Deux consommateurs en descendent ([`rerun_run_id`] et [`failing_names`]) et
/// doivent regarder la même population : la cible qu'on relance et les noms
/// qu'on journalise décrivent le même échec, donc deux prédicats libres de
/// diverger produiraient une ligne qui nomme un check et un run qui en relance
/// un autre.
///
/// Le vocabulaire est celui de `classify_checks`, dont ce module ne peut pas
/// réutiliser le filtre : elle rend une *classification*, et `collect_checks`
/// — qui rend la liste — est privée à `pr_merge_with_gate`. L'élargir
/// traverserait la frontière decision-core pour une économie de deux lignes.
fn red_checks(checks: &[GhCheck]) -> impl Iterator<Item = &GhCheck> {
    checks
        .iter()
        .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
}

/// Le **lecteur unique** de [`RerunOutcome`] : la phrase à joindre au motif que
/// l'appelant expose.
///
/// `match` exhaustif **sans bras `_ =>`**. Un seul lecteur plutôt qu'un `match`
/// recopié dans les trois sites d'appel : trois copies sont trois formulations
/// libres de diverger, et la divergence silencieuse est la classe que
/// `grooming_marker` (mika#2158) a dû fermer. Pur, donc chaque branche est
/// épinglée par test.
pub(crate) fn rerun_detail_suffix(outcome: &RerunOutcome) -> String {
    match outcome {
        RerunOutcome::Triggered { run_id } => format!(
            " The failed jobs of run {run_id} were re-run once (mika#2617 AC2). The gate \
             stays closed and re-enters by itself on the next \
             `check_suite.completed(success)`. There will be NO second automatic re-run: \
             if it fails again, it is not flaky, it is broken."
        ),
        RerunOutcome::AlreadySpent { run_id } => format!(
            " Run {run_id} has ALREADY been re-run once on this head and failed again \
             (mika#2617 AC2). This is not flaky, it is broken: the failure needs a fix, \
             not another re-run. Pushing a new commit reopens the re-run budget."
        ),
        RerunOutcome::NoActionsRun => " No automatic re-run was attempted: no GitHub Actions run \
             could be derived from the failing checks' links (external check, or a link \
             without the `/job/` segment)."
            .to_string(),
        RerunOutcome::LedgerUnreadable => {
            " No automatic re-run was attempted: the re-run budget could not be read \
             (fail-closed, mika#2617). Re-running without being able to tell whether it \
             already happened is how a re-run becomes a loop."
                .to_string()
        }
        RerunOutcome::Refused(detail) => format!(
            " The automatic re-run was REFUSED by `gh`: {detail}. A 403 here means the \
             credential lacks the `actions: write` scope, which is an operator fix, not \
             something to retry. The gate stays closed either way."
        ),
        RerunOutcome::Disarmed => format!(
            " No automatic re-run was attempted: it is disarmed on this process \
             ({MERGE_GATE_RERUN_ENV})."
        ),
    }
}

/// Relance les jobs échoués d'un run, une fois, si le budget le permet.
///
/// Appelée depuis les **trois** branches `HasFailures` du moteur (le tool,
/// `verdict_handler`, `ci_success_handler`). Le ledger par `(dépôt, PR, tête,
/// run)` rend l'emplacement peu sensible — c'est lui qui tient l'unicité, pas
/// le site — mais les trois y passent parce que le défaut mesuré (#2614,
/// `mergedBy: mika-platform-dev`) ne dit pas lequel des trois a mergé, et armer
/// un seul site serait armer peut-être le mauvais.
///
/// Ordre des termes, du moins cher au plus cher, et chacun avec sa raison :
///
/// 1. **run_id** (pur, gratuit) — rien à relancer, rien à lire, rien à écrire.
/// 2. **armement** — un process désarmé ne doit ni lire la base ni appeler le
///    réseau ; la *détection* du rouge, elle, est inconditionnelle et a déjà eu
///    lieu chez l'appelant (motif mika#2249 / mika#2272 : la détection est
///    inconditionnelle, seule la disposition est gatée).
/// 3. **head_sha** — sans lui la clé n'est pas constructible ; fail-closed.
/// 4. **lecture du ledger** — une requête.
/// 5. **écriture du ledger**, puis **la relance** — dans cet ordre, voir la doc
///    du module.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn maybe_rerun_failed_checks(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    repo: &str,
    pr_number: u64,
    head_sha: &str,
    checks: &[GhCheck],
    token: &str,
) -> RerunOutcome {
    let Some(run_id) = rerun_run_id(checks) else {
        let outcome = RerunOutcome::NoActionsRun;
        emit(repo, pr_number, head_sha, None, &outcome, None);
        return outcome;
    };

    if !merge_gate_rerun_enabled() {
        let outcome = RerunOutcome::Disarmed;
        emit(repo, pr_number, head_sha, Some(run_id), &outcome, None);
        return outcome;
    }

    if head_sha.is_empty() {
        // Un `headRefOid` absent se désérialise en `""` (`#[serde(default)]`).
        // Le traiter comme une tête ferait du ledger une clé partagée par
        // toutes les têtes illisibles — un budget pour toutes.
        let outcome = RerunOutcome::LedgerUnreadable;
        emit(
            repo,
            pr_number,
            head_sha,
            Some(run_id),
            &outcome,
            Some("empty_head_sha"),
        );
        return outcome;
    }

    let key = rerun_ledger_key(repo, pr_number, head_sha, run_id);
    let since = crate::timestamp::now_minus(chrono::Duration::days(RERUN_LEDGER_WINDOW_DAYS));

    match db
        .count_recent_audit_events_for_target(MERGE_GATE_RERUN_AUDIT_TOOL, &key, &since)
        .await
    {
        Ok(0) => {}
        Ok(_) => {
            // L'épuisement n'écrit AUCUNE ligne d'audit — voir la doc du module.
            let outcome = RerunOutcome::AlreadySpent { run_id };
            warn!(
                event = "merge_gate_rerun_exhausted",
                repo,
                pr = pr_number,
                head_sha,
                run_id,
                failing = failing_names(checks).join(", "),
                "mika#2617 AC2: run already re-run once on this head and still red — not \
                 flaky, broken. The gate stays closed; no second automatic re-run."
            );
            emit(repo, pr_number, head_sha, Some(run_id), &outcome, None);
            return outcome;
        }
        Err(e) => {
            let outcome = RerunOutcome::LedgerUnreadable;
            warn!(
                event = "merge_gate_rerun_ledger_unreadable",
                repo,
                pr = pr_number,
                head_sha,
                run_id,
                error = %e,
                "mika#2617 AC2: the re-run budget could not be read — fail-closed, no re-run"
            );
            emit(
                repo,
                pr_number,
                head_sha,
                Some(run_id),
                &outcome,
                Some("ledger_read_failed"),
            );
            return outcome;
        }
    }

    // La réservation précède la relance : `audit_events` est append-only, donc
    // une ligne écrite après la relance ne pourrait pas tenir « jamais deux
    // fois » en travers d'un crash.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            MERGE_GATE_RERUN_AUDIT_TOOL,
            &key,
            None,
            Some(MERGE_GATE_RERUN_ATTEMPTED),
            Some(&format!(
                "repo:{repo} pr:{pr_number} head_sha:{head_sha} run_id:{run_id} failing:{}",
                failing_names(checks).join(", ")
            )),
            Some(trace_id),
        )
        .await
    {
        let outcome = RerunOutcome::LedgerUnreadable;
        warn!(
            event = "merge_gate_rerun_ledger_unwritable",
            repo,
            pr = pr_number,
            head_sha,
            run_id,
            error = %e,
            "mika#2617 AC2: the re-run reservation could not be written — fail-closed, no \
             re-run. Re-running without a durable record is how a re-run becomes a loop."
        );
        emit(
            repo,
            pr_number,
            head_sha,
            Some(run_id),
            &outcome,
            Some("ledger_write_failed"),
        );
        return outcome;
    }

    let outcome = match rerun_failed_jobs(repo, run_id, token).await {
        Ok(_) => RerunOutcome::Triggered { run_id },
        Err(e) => RerunOutcome::Refused(e),
    };
    emit(repo, pr_number, head_sha, Some(run_id), &outcome, None);
    outcome
}

/// `gh run rerun <run_id> --repo <repo> --failed`, borné.
///
/// **`--failed` et non `--job <id>`** : les deux drapeaux sont mutuellement
/// exclusifs (plan R1 — la forme qu'AC2 écrivait est inexécutable telle
/// quelle), et `--failed` est le **gain** plutôt que le pis-aller : il couvre
/// d'un seul appel les N lints rouges d'un même run.
async fn rerun_failed_jobs(repo: &str, run_id: u64, token: &str) -> Result<String, String> {
    let run_str = run_id.to_string();
    let args = vec!["run", "rerun", &run_str, "--repo", repo, "--failed"];

    match tokio::time::timeout(
        std::time::Duration::from_secs(RERUN_TIMEOUT_SECS),
        crate::tools::pr_merge_with_gate::run_gh_subprocess(&args, token),
    )
    .await
    {
        Ok(inner) => inner,
        Err(_) => Err(format!(
            "gh run rerun timed out after {RERUN_TIMEOUT_SECS}s"
        )),
    }
}

/// Les noms des checks rouges, pour que la ligne soit auto-suffisante.
fn failing_names(checks: &[GhCheck]) -> Vec<&str> {
    red_checks(checks).map(|c| c.name.as_str()).collect()
}

/// La ligne de journal de l'issue — **un nom, l'issue dans un champ**.
///
/// Motif `ready_label_outcome` (mika#2323) : les six issues appartiennent au
/// même site et à la même population, donc un `jq` sur `outcome` les sépare et
/// les rend soustractibles. `reason` ne porte une valeur que là où le nom seul
/// serait ambigu (les deux causes de `LedgerUnreadable`) ; ailleurs il est
/// **absent**, jamais `"null"` — un champ qui affirme ce qu'on n'a pas mesuré
/// est la classe mika#2304.
fn emit(
    repo: &str,
    pr_number: u64,
    head_sha: &str,
    run_id: Option<u64>,
    outcome: &RerunOutcome,
    reason: Option<&str>,
) {
    let label = match outcome {
        RerunOutcome::Triggered { .. } => "triggered",
        RerunOutcome::AlreadySpent { .. } => "already_spent",
        RerunOutcome::NoActionsRun => "no_actions_run",
        RerunOutcome::LedgerUnreadable => "ledger_unreadable",
        RerunOutcome::Refused(_) => "refused",
        RerunOutcome::Disarmed => "disarmed",
    };
    // Exhaustif **sans bras `_ =>`**, comme celui ci-dessus et pour la même
    // raison : `Refused` est la seule issue qui porte un détail *aujourd'hui*,
    // et un joker ferait tomber dans « pas de détail » une issue ajoutée demain
    // qui en porterait un. `mika2617_rerun_outcome_has_no_wildcard_arm` refuse
    // le joker dans toute fonction qui lit un `RerunOutcome`.
    let detail = match outcome {
        RerunOutcome::Refused(d) => Some(d.as_str()),
        RerunOutcome::Triggered { .. }
        | RerunOutcome::AlreadySpent { .. }
        | RerunOutcome::NoActionsRun
        | RerunOutcome::LedgerUnreadable
        | RerunOutcome::Disarmed => None,
    };

    info!(
        // La constante, jamais le littéral : elle est SOLE WRITER de ce nom, et
        // c'est ce qui rend le compte de l'opérateur exact — compte dont dépend
        // aussi l'invariant « jamais deux fois », qui EST un compte.
        event = MERGE_GATE_RERUN_AUDIT_TOOL,
        repo,
        pr = pr_number,
        head_sha,
        run_id,
        outcome = label,
        reason,
        detail,
        "mika#2617 AC2: re-run disposition"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(name: &str, link: Option<&str>) -> GhCheck {
        GhCheck {
            name: name.to_string(),
            state: "FAILURE".to_string(),
            bucket: "fail".to_string(),
            link: link.map(str::to_string),
        }
    }

    fn green(name: &str, link: Option<&str>) -> GhCheck {
        GhCheck {
            name: name.to_string(),
            state: "SUCCESS".to_string(),
            bucket: "pass".to_string(),
            link: link.map(str::to_string),
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // La cible de la relance (pure)
    // ─────────────────────────────────────────────────────────────────────

    /// N lints rouges du même run sont couverts par **une** relance (plan R1).
    #[test]
    fn mika2617_one_rerun_covers_every_red_check_of_the_same_run() {
        let checks = vec![
            green(
                "Check",
                Some("https://github.com/o/r/actions/runs/777/job/1"),
            ),
            red(
                "Egress Uniqueness Lint",
                Some("https://github.com/o/r/actions/runs/777/job/2"),
            ),
            red(
                "Egress Manifest Lint",
                Some("https://github.com/o/r/actions/runs/777/job/3"),
            ),
        ];
        assert_eq!(rerun_run_id(&checks), Some(777));
    }

    /// Le run est dérivé d'un check **rouge**, jamais d'un vert : relancer le
    /// run d'un check qui a réussi ne répare rien.
    #[test]
    fn mika2617_the_rerun_target_comes_from_a_red_check() {
        let checks = vec![
            green(
                "Check",
                Some("https://github.com/o/r/actions/runs/111/job/1"),
            ),
            red(
                "Lint",
                Some("https://github.com/o/r/actions/runs/222/job/2"),
            ),
        ];
        assert_eq!(rerun_run_id(&checks), Some(222));
    }

    /// Un check externe rouge posé en tête de liste ne rend pas la relance
    /// inerte : on continue sur le suivant.
    #[test]
    fn mika2617_an_external_red_check_does_not_shadow_a_real_run() {
        let checks = vec![
            red("Netlify", Some("https://app.netlify.com/sites/x/deploys/a")),
            red(
                "Check",
                Some("https://github.com/o/r/actions/runs/333/job/9"),
            ),
        ];
        assert_eq!(rerun_run_id(&checks), Some(333));
    }

    /// Aucun run dérivable — la population de `NoActionsRun`.
    #[test]
    fn mika2617_no_derivable_run_yields_none() {
        assert_eq!(rerun_run_id(&[]), None);
        assert_eq!(rerun_run_id(&[red("Netlify", None)]), None);
        assert_eq!(
            rerun_run_id(&[red("Check", Some("https://github.com/o/r/actions/runs/1"))]),
            None,
            "lien de run sans `/job/` — comportement hérité de #594"
        );
        assert_eq!(
            rerun_run_id(&[green(
                "Check",
                Some("https://github.com/o/r/actions/runs/1/job/2")
            )]),
            None,
            "contrôle négatif : aucun rouge, donc rien à relancer"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // La clé de ledger
    // ─────────────────────────────────────────────────────────────────────

    /// La clé porte la tête, donc **un nouveau sha rouvre le budget**.
    #[test]
    fn mika2617_the_ledger_key_carries_the_head_so_a_new_sha_reopens_the_budget() {
        let a = rerun_ledger_key("senara-solutions/mika", 2614, "8ccaabc8", 777);
        let b = rerun_ledger_key("senara-solutions/mika", 2614, "41a4a20e", 777);
        assert_eq!(a, "rerun:senara-solutions/mika#2614@8ccaabc8:777");
        assert_ne!(a, b, "deux têtes = deux budgets");
    }

    /// Le piège `#234` ↔ `#2343` de mika#2347 ne s'ouvre pas : la comparaison
    /// est une égalité, et le `@` borne le numéro même pour un futur lecteur de
    /// préfixe.
    #[test]
    fn mika2617_the_ledger_key_bounds_the_pr_number() {
        let short = rerun_ledger_key("o/r", 234, "abc", 1);
        let long = rerun_ledger_key("o/r", 2343, "abc", 1);
        assert_ne!(short, long);
        assert!(
            !long.starts_with(&short),
            "le `@` doit empêcher qu'une clé courte soit le préfixe d'une longue : \
             {short} / {long}"
        );
    }

    /// Deux runs distincts de la même tête ont deux budgets — le run est la
    /// granularité de la relance (plan R1).
    #[test]
    fn mika2617_the_ledger_key_separates_two_runs_of_one_head() {
        assert_ne!(
            rerun_ledger_key("o/r", 1, "abc", 777),
            rerun_ledger_key("o/r", 1, "abc", 888)
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Le désarmement
    // ─────────────────────────────────────────────────────────────────────

    /// Armée par défaut ; seul un `0`/`false`/`off`/`no` explicite désarme ; une
    /// valeur non reconnue **reste armée**.
    #[test]
    fn mika2617_the_rerun_is_armed_by_default_and_a_typo_does_not_disarm_it() {
        assert!(merge_gate_rerun_is_enabled(None));
        assert!(merge_gate_rerun_is_enabled(Some("")));
        assert!(merge_gate_rerun_is_enabled(Some("  ")));
        assert!(merge_gate_rerun_is_enabled(Some("1")));
        assert!(merge_gate_rerun_is_enabled(Some("TRUE")));
        assert!(
            merge_gate_rerun_is_enabled(Some("plif")),
            "une valeur non reconnue laisse la relance armée, et la nomme"
        );
        assert!(
            !merge_gate_rerun_is_enabled(Some(" 0 ")),
            "les espaces sont tolérés : ` 0 ` trimé est `0`, donc il désarme — un \
             opérateur qui colle sa valeur avec une espace a quand même désarmé"
        );
    }

    /// Le désarmement explicite, dans ses quatre orthographes.
    #[test]
    fn mika2617_the_four_explicit_disarm_spellings() {
        for raw in ["0", "false", "off", "no", "FALSE", "Off"] {
            assert!(
                !merge_gate_rerun_is_enabled(Some(raw)),
                "`{raw}` doit désarmer la relance"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Le lecteur unique de l'enum
    // ─────────────────────────────────────────────────────────────────────

    /// Chacune des six issues rend une phrase, et les deux qui comptent le
    /// disent : `Triggered` annonce qu'il n'y aura **pas** de seconde relance,
    /// `AlreadySpent` annonce qu'un nouveau commit rouvre le budget.
    #[test]
    fn mika2617_every_rerun_outcome_has_its_own_sentence() {
        let triggered = rerun_detail_suffix(&RerunOutcome::Triggered { run_id: 777 });
        assert!(triggered.contains("777"));
        assert!(
            triggered.contains("NO second automatic re-run"),
            "{triggered}"
        );

        let spent = rerun_detail_suffix(&RerunOutcome::AlreadySpent { run_id: 777 });
        assert!(spent.contains("777"));
        assert!(spent.contains("not flaky, it is broken"), "{spent}");
        assert!(
            spent.contains("reopens the re-run budget"),
            "le motif doit nommer la sortie : un nouveau commit. {spent}"
        );

        let none = rerun_detail_suffix(&RerunOutcome::NoActionsRun);
        assert!(none.contains("No automatic re-run"), "{none}");

        let unreadable = rerun_detail_suffix(&RerunOutcome::LedgerUnreadable);
        assert!(unreadable.contains("fail-closed"), "{unreadable}");

        let refused = rerun_detail_suffix(&RerunOutcome::Refused("gh exit code 1: 403".into()));
        assert!(refused.contains("403"), "{refused}");
        assert!(
            refused.contains("actions: write"),
            "le motif doit nommer le scope — c'est le remède opérateur (plan R3). {refused}"
        );

        let disarmed = rerun_detail_suffix(&RerunOutcome::Disarmed);
        assert!(disarmed.contains(MERGE_GATE_RERUN_ENV), "{disarmed}");
    }

    /// **Aucune phrase n'ouvre la porte**, et c'est le point.
    ///
    /// Un `RerunOutcome` est un effet de bord plus un motif ; la porte est
    /// fermée par la phase A et le reste. Un futur éditeur qui ferait dire à une
    /// de ces phrases « the merge may proceed » déplacerait la décision de merge
    /// dans un module qui n'a pas à la prendre.
    #[test]
    fn mika2617_no_rerun_outcome_claims_the_gate_may_open() {
        for outcome in [
            RerunOutcome::Triggered { run_id: 1 },
            RerunOutcome::AlreadySpent { run_id: 1 },
            RerunOutcome::NoActionsRun,
            RerunOutcome::LedgerUnreadable,
            RerunOutcome::Refused("x".into()),
            RerunOutcome::Disarmed,
        ] {
            let s = rerun_detail_suffix(&outcome).to_lowercase();
            for forbidden in ["may proceed", "may be merged", "merging is allowed"] {
                assert!(
                    !s.contains(forbidden),
                    "aucune issue de relance n'autorise un merge : {outcome:?} → {s}"
                );
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // La relance de bout en bout, contre la base (sans réseau)
    //
    // `gh` n'existe pas dans l'environnement de test, donc `rerun_failed_jobs`
    // échoue et l'issue est `Refused`. Ce qui est attesté ici est ce que ces
    // tests existent pour attester : **que la réservation a été écrite avant la
    // relance, et qu'une seconde tentative sur la même tête est refusée**. Le
    // succès de `gh` est hors de portée d'un test d'unité et vit dans la sonde
    // S3 (geste opérateur).
    // ─────────────────────────────────────────────────────────────────────

    fn harness() -> crate::test_utils::test_helpers::TestHarness {
        crate::test_utils::test_helpers::TestHarness::new()
    }

    fn two_red_checks_of_one_run() -> Vec<GhCheck> {
        vec![
            red(
                "Egress Uniqueness Lint",
                Some("https://github.com/o/r/actions/runs/777/job/2"),
            ),
            red(
                "Egress Manifest Lint",
                Some("https://github.com/o/r/actions/runs/777/job/3"),
            ),
        ]
    }

    /// **T5 — ledger vide : la relance est tentée, et la réservation est
    /// écrite.**
    #[tokio::test]
    async fn mika2617_t5_an_empty_ledger_spends_the_budget_once() {
        let h = harness();
        let outcome = maybe_rerun_failed_checks(
            &h.db,
            "s-1",
            "t-1",
            "o/r",
            2614,
            "8ccaabc8",
            &two_red_checks_of_one_run(),
            "fake-token",
        )
        .await;

        // `gh` est absent du bac à sable : la relance est tentée et refusée.
        assert!(
            matches!(outcome, RerunOutcome::Refused(_)),
            "attendu une tentative refusée faute de `gh`, obtenu {outcome:?}"
        );

        let key = rerun_ledger_key("o/r", 2614, "8ccaabc8", 777);
        let since = crate::timestamp::now_minus(chrono::Duration::days(1));
        assert_eq!(
            h.db.count_recent_audit_events_for_target(MERGE_GATE_RERUN_AUDIT_TOOL, &key, &since)
                .await
                .unwrap(),
            1,
            "la réservation doit être écrite AVANT la relance — sinon un crash entre les \
             deux rouvrirait le budget"
        );
    }

    /// **T6 — le budget est dépensé : aucune seconde relance.**
    #[tokio::test]
    async fn mika2617_t6_a_spent_budget_refuses_a_second_rerun() {
        let h = harness();
        let checks = two_red_checks_of_one_run();

        let first =
            maybe_rerun_failed_checks(&h.db, "s-1", "t-1", "o/r", 2614, "8ccaabc8", &checks, "tok")
                .await;
        assert!(matches!(first, RerunOutcome::Refused(_)));

        let second =
            maybe_rerun_failed_checks(&h.db, "s-1", "t-2", "o/r", 2614, "8ccaabc8", &checks, "tok")
                .await;
        assert_eq!(
            second,
            RerunOutcome::AlreadySpent { run_id: 777 },
            "AC2 : jamais de seconde relance automatique"
        );

        let key = rerun_ledger_key("o/r", 2614, "8ccaabc8", 777);
        let since = crate::timestamp::now_minus(chrono::Duration::days(1));
        assert_eq!(
            h.db.count_recent_audit_events_for_target(MERGE_GATE_RERUN_AUDIT_TOOL, &key, &since)
                .await
                .unwrap(),
            1,
            "l'épuisement n'écrit AUCUNE ligne d'audit : sinon le compte qui tient \
             l'invariant serait pollué par jusqu'à huit lignes par push (mika#1869)"
        );
    }

    /// **T9 — un nouveau sha rouvre le budget.**
    #[tokio::test]
    async fn mika2617_t9_a_new_head_reopens_the_budget() {
        let h = harness();
        let checks = two_red_checks_of_one_run();

        let _ =
            maybe_rerun_failed_checks(&h.db, "s-1", "t-1", "o/r", 2614, "8ccaabc8", &checks, "tok")
                .await;
        let after_push =
            maybe_rerun_failed_checks(&h.db, "s-1", "t-2", "o/r", 2614, "41a4a20e", &checks, "tok")
                .await;
        assert!(
            matches!(after_push, RerunOutcome::Refused(_)),
            "nouveau code, nouvelle chance — obtenu {after_push:?}"
        );
    }

    /// **T7 — aucun run Actions dérivable : aucune relance, aucune écriture.**
    #[tokio::test]
    async fn mika2617_t7_a_non_actions_link_triggers_nothing() {
        let h = harness();
        let checks = vec![red(
            "Netlify",
            Some("https://app.netlify.com/sites/x/deploys/a"),
        )];
        let outcome =
            maybe_rerun_failed_checks(&h.db, "s-1", "t-1", "o/r", 2614, "8ccaabc8", &checks, "tok")
                .await;
        assert_eq!(outcome, RerunOutcome::NoActionsRun);
    }

    /// **Un `head_sha` vide est illisible, jamais une tête.**
    ///
    /// Sans ce refus, toutes les têtes illisibles partageraient une clé de
    /// ledger — donc un seul budget pour toutes.
    #[tokio::test]
    async fn mika2617_an_empty_head_sha_is_unreadable_never_a_head() {
        let h = harness();
        let outcome = maybe_rerun_failed_checks(
            &h.db,
            "s-1",
            "t-1",
            "o/r",
            2614,
            "",
            &two_red_checks_of_one_run(),
            "tok",
        )
        .await;
        assert_eq!(outcome, RerunOutcome::LedgerUnreadable);

        let key = rerun_ledger_key("o/r", 2614, "", 777);
        let since = crate::timestamp::now_minus(chrono::Duration::days(1));
        assert_eq!(
            h.db.count_recent_audit_events_for_target(MERGE_GATE_RERUN_AUDIT_TOOL, &key, &since)
                .await
                .unwrap(),
            0,
            "aucune ligne ne doit être écrite sous une clé à tête vide"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Gardes structurelles
    // ─────────────────────────────────────────────────────────────────────

    /// Allowlist du scan de bras joker — **livrée vide, et épinglée vide**.
    const ALLOWED_RERUN_WILDCARD_SITES: &[&str] = &[];

    /// Allowlist du scan d'écrivain unique — **livrée vide, et épinglée vide**.
    const ALLOWED_RERUN_NAME_WRITERS: &[&str] = &[];

    #[test]
    fn mika2617_the_rerun_allowlists_are_empty() {
        assert!(
            ALLOWED_RERUN_WILDCARD_SITES.is_empty(),
            "livrée vide et doit le rester : une allowlist née vide est un tiroir où \
             déposer la prochaine infraction (mika#2323)"
        );
        assert!(ALLOWED_RERUN_NAME_WRITERS.is_empty());
    }

    fn production_of(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(rel);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("mika#2617 — {} illisible : {e}", path.display()));
        crate::source_scan::strip_comment_lines(crate::source_scan::production_half(&content))
    }

    /// Énumère la moitié production de chaque `.rs` sous `crates/*/src`.
    ///
    /// Calqué sur `canonical_tokens::tests::production_sources`, dont le `mod
    /// tests` est privé et donc hors d'atteinte d'un autre module de test. Le
    /// code de test est écarté deux fois — par le **chemin**
    /// ([`crate::source_scan::is_test_source_path`], la réparation mika#2321
    /// d'une prémisse que mika#2310 avait cessé de rendre vraie) **et** par
    /// troncature au module de test : une fixture porte légitimement chacune
    /// des aiguilles cherchées ici, et la compter ferait de l'inventaire un
    /// recensement de sa propre suite de tests.
    fn production_sources() -> Vec<(String, String)> {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("racine du dépôt au-dessus de crates/mika-agent")
            .to_path_buf();
        let mut out = Vec::new();
        let mut stack = vec![repo_root.join("crates")];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    if path.file_name().unwrap_or_default() == "target" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || crate::source_scan::is_test_source_path(&path)
                {
                    continue;
                }
                let rel = path
                    .strip_prefix(&repo_root)
                    .expect("chemin sous la racine")
                    .to_string_lossy()
                    .replace('\\', "/");
                if !rel.contains("/src/") {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                out.push((
                    rel,
                    crate::source_scan::production_half(&content).to_string(),
                ));
            }
        }

        assert!(
            !out.is_empty(),
            "aucune source de production trouvée sous crates/*/src — un scan qui ne scanne \
             rien est un laissez-passer vide, pas un scan propre (mika#2103)"
        );
        out
    }

    /// Les fichiers où un `RerunOutcome` peut être lu.
    const RERUN_CONSUMER_SOURCES: &[&str] = &[
        "merge_gate_rerun.rs",
        "tools/pr_merge_with_gate.rs",
        "server/verdict_handler.rs",
        "server/ci_success_handler.rs",
    ];

    /// Le prédicat du scan de bras joker, isolé pour que son contrôle de bonne
    /// foi puisse l'exercer sur une fixture plutôt que sur l'arbre.
    fn wildcards_in_rerun_readers(src: &str) -> Vec<String> {
        let mut hits = Vec::new();
        for (name, body) in crate::source_scan::fn_bodies(src) {
            if !body.contains("RerunOutcome::") {
                continue;
            }
            for line in body.lines().map(str::trim) {
                if line.starts_with("_ =>") || line.starts_with("_ if ") {
                    hits.push(format!("{name}: {line}"));
                }
            }
        }
        hits
    }

    /// **Contrôle de bonne foi du scan de bras joker : il mord sur une
    /// fixture, et laisse passer le `_ =>` légitime d'à côté.**
    ///
    /// Sans ce contrôle, un prédicat devenu inopérant — `fn_bodies` qui change
    /// de découpage, `RerunOutcome` renommé — se lirait exactement comme un
    /// module propre (classe mika#2103 / mika#2205). Et sans sa seconde
    /// assertion, un prédicat resserré jusqu'à ne plus rien voir et un prédicat
    /// élargi jusqu'à tout accuser rendraient le même vert.
    #[test]
    fn mika2617_the_wildcard_scan_reddens_on_a_fixture() {
        let fixture = r#"fn reads_the_outcome(o: &RerunOutcome) -> &str {
    match o {
        RerunOutcome::Disarmed => "d",
        _ => "x",
    }
}
fn reads_an_env_string(v: &str) -> bool {
    match v {
        "0" => false,
        _ => true,
    }
}
"#;
        let hits = wildcards_in_rerun_readers(fixture);
        assert_eq!(
            hits.len(),
            1,
            "le prédicat doit voir le joker de la fonction qui lit l'enum et ignorer \
             celui de la fonction qui lit une chaîne d'environnement, où l'inconnu est \
             précisément la population à attraper : {hits:?}"
        );
        assert!(hits[0].starts_with("reads_the_outcome"), "{hits:?}");
    }

    /// **Le `match` sur `RerunOutcome` n'a aucun bras joker (plan U3).**
    ///
    /// Aucun test de comportement ne peut voir cette classe : un `_ =>` ajouté
    /// demain ne rend aucune décision fausse le jour où il est écrit — il fait
    /// tomber une issue **future** dans un défaut silencieux.
    ///
    /// Le prédicat est en deux termes. (1) Aucune **fonction qui lit un
    /// `RerunOutcome`** ne porte de bras joker — porté sur le corps de fonction
    /// via [`crate::source_scan::fn_bodies`], le lecteur unique de ce découpage,
    /// et non sur le fichier entier : `merge_gate_rerun_is_enabled` emploie
    /// légitimement un `_ =>` sur une **chaîne d'environnement**, où l'inconnu
    /// est précisément la population à attraper. (2) Aucun autre fichier ne lit
    /// un `RerunOutcome` — ce qui rend le terme (1) suffisant plutôt que local.
    #[test]
    fn mika2617_rerun_outcome_has_no_wildcard_arm() {
        let own = production_of("merge_gate_rerun.rs");
        assert!(
            own.contains("RerunOutcome::"),
            "anti-vacuité : le scan vise un nom absent du module, donc il ne vérifie rien \
             (classe mika#2103 / mika#2205)"
        );

        assert!(
            crate::source_scan::fn_bodies(&own)
                .iter()
                .any(|(_, b)| b.contains("RerunOutcome::")),
            "anti-vacuité : aucun corps de fonction ne lit un `RerunOutcome` — le \
             découpage vise à côté et le terme (1) ne vérifie rien"
        );

        let wildcards = wildcards_in_rerun_readers(&own);
        assert!(
            wildcards.is_empty(),
            "mika#2617 U3 — un bras joker a été ajouté dans une fonction qui lit un \
             `RerunOutcome` : {wildcards:?}\n\n\
             RÉSOLUTION : l'énumérer. Un `_ =>` fait tomber une issue ajoutée demain dans \
             un défaut silencieux, et c'est très exactement ce que l'exhaustivité achète."
        );

        let mut extra_readers = Vec::new();
        for rel in RERUN_CONSUMER_SOURCES.iter().skip(1) {
            let src = production_of(rel);
            for line in src.lines() {
                let t = line.trim();
                if t.starts_with("match ") && t.contains("rerun") && t.contains("outcome") {
                    extra_readers.push(format!("{rel}: {t}"));
                }
                if t.contains("RerunOutcome::") && !ALLOWED_RERUN_WILDCARD_SITES.contains(rel) {
                    extra_readers.push(format!("{rel}: {t}"));
                }
            }
        }
        assert!(
            extra_readers.is_empty(),
            "mika#2617 U3 — un second lecteur de `RerunOutcome` existe : {extra_readers:?}\n\n\
             RÉSOLUTION : passer par `rerun_detail_suffix`. Trois `match` recopiés sont \
             trois formulations libres de diverger, et la divergence silencieuse est la \
             classe que `grooming_marker` (mika#2158) a dû fermer."
        );
    }

    /// **La grammaire de lien a un lecteur unique (plan R14).**
    ///
    /// `extract_actions_run_id` écrit à côté de `parse_check_link` aurait été le
    /// second lecteur que la rectification R11 du même plan refuse pour le head
    /// SHA. Le scan refuse qu'il réapparaisse.
    #[test]
    fn mika2617_run_id_has_a_single_link_reader() {
        let reader = production_of("check_link.rs");
        assert!(
            reader.contains("/job/"),
            "anti-vacuité : le lecteur promu ne porte plus la grammaire — le scan vise un \
             chemin mort (classe mika#2103 / mika#2205)"
        );

        let mut offenders = Vec::new();
        for (rel, src) in production_sources() {
            if rel.ends_with("/src/check_link.rs") {
                continue;
            }
            if ALLOWED_RERUN_NAME_WRITERS.contains(&rel.as_str()) {
                continue;
            }
            let stripped = crate::source_scan::strip_comment_lines(&src);
            for line in stripped.lines() {
                if line.contains("\"/job/\"") || line.contains("/actions/runs/") {
                    offenders.push(format!("{rel}: {}", line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "mika#2617 R14 — un second lecteur de la grammaire `actions/runs/…/job/…` : \
             {offenders:?}\n\n\
             RÉSOLUTION : appeler `crate::check_link::parse_check_link` ou \
             `run_id_from_link`. On route le site par le lecteur, on n'allowliste pas \
             (doctrine mika#2201)."
        );
    }

    /// **`merge_gate_check_rerun` a un écrivain unique.**
    ///
    /// C'est ce qui rend le `GROUP BY` de l'opérateur exact plutôt qu'un nombre
    /// sur lequel deux sites peuvent diverger — et c'est aussi ce qui tient
    /// l'invariant « jamais deux fois », dont le prédicat est un *compte*.
    #[test]
    fn mika2617_the_rerun_audit_name_has_a_single_writer() {
        let mut writers = Vec::new();
        for (rel, src) in production_sources() {
            let stripped = crate::source_scan::strip_comment_lines(&src);
            for line in stripped.lines() {
                if line.contains("\"merge_gate_check_rerun\"") {
                    writers.push(format!("{rel}: {}", line.trim()));
                }
            }
        }
        assert_eq!(
            writers.len(),
            1,
            "mika#2617 — le nom `merge_gate_check_rerun` doit avoir exactement un site de \
             définition (la constante `MERGE_GATE_RERUN_AUDIT_TOOL`). Trouvés : {writers:#?}\n\n\
             RÉSOLUTION : interpoler la constante. Un second écrivain ne rend aucune \
             décision fausse le jour où il est écrit — il rend le compte inexact, ce \
             qu'aucun test de comportement ne peut voir."
        );
        assert!(
            writers[0].ends_with("src/merge_gate_rerun.rs: pub const MERGE_GATE_RERUN_AUDIT_TOOL: &str = \"merge_gate_check_rerun\";")
                || writers[0].contains("merge_gate_rerun.rs"),
            "l'unique écrivain doit être la constante de ce module : {writers:#?}"
        );
    }
}

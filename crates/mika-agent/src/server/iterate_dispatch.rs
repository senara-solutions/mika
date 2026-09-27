//! Déclencheur d'itération **déterministe** sur la PR ouverte d'une issue
//! (mika#2506).
//!
//! # Le défaut que ça ferme, et il est lisible dans le code
//!
//! Le flux manuel documenté — `dispatch mika#N`, puis le garde
//! `dispatch_task_has_open_pr` surfaçant les options — **ne peut pas marcher**
//! dès lors que la tâche précédente est terminale, et la chaîne est structurelle
//! plutôt qu'intermittente :
//!
//! 1. `idx_tasks_manual_active_ref_url` exclut les statuts terminaux, donc une
//!    tâche `completed` est hors de l'index unique de dédup ;
//! 2. `dispatch mika#N` crée donc une tâche **fraîche, métadonnées vides** ;
//! 3. [`crate::skills::executor`]'s `check_task_has_open_pr` lit
//!    `claude_pilot.pr_url` **de la tâche** — absente sur une tâche fraîche —
//!    donc le garde autorise le dispatch ;
//! 4. le pilote dérive la branche de l'issue et lance `/mika` : un **implement
//!    neuf** sur une PR revue.
//!
//! Le garde est sur l'**identité de la tâche** ; le modèle mental de l'opérateur
//! est l'**identité de l'issue**. C'est la classe mika#1971 : *une instruction
//! qui envoie le déployeur au seul endroit où la clé ne peut pas marcher.*
//!
//! Et le geste qui marche (`mika ask "iterate on mika#N with iteration_context:
//! …"`) n'existe **que dans un prompt** : c'est le modèle de mika-dev qui choisit
//! `run_claude_pilot` et compose `iteration_context`. Par
//! `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
//! (mika#2120), une route qui vit dans un prompt n'a pas de garantie — et la
//! variante voisine **crash** la session (pas de worktree sur le chemin
//! free-text).
//!
//! # Ce module ne compose aucune machinerie de dispatch
//!
//! Le dispatch déterministe existe déjà : [`super::verdict_handler::try_engine_dispatch_for`]
//! (mika#1630, généralisé par mika#2506 U1) fait résolution de l'outil →
//! résolution du handler long-running → `validate_dispatch_readiness` → row
//! callback → `mark_parent_dispatched` → `spawn_long_running_exec`. Ce module est
//! un **troisième appelant**, pas une seconde implémentation.
//!
//! # Le nombre qui voyage est le numéro d'ISSUE
//!
//! `dispatch-lib.sh` consomme `prompt: "<repo>#<N>"` comme un numéro d'**issue**
//! (`gh issue view "$ISSUE_NUM"`, puis `derive-branch-name --issue "$ISSUE_NUM"
//! --body-callout "$ISSUE_BODY"`). La PR est donc résolue **uniquement** pour
//! vérifier les préconditions et n'est **jamais** transmise en aval — c'est ce
//! que `mika2506_le_nombre_qui_voyage_est_le_numero_d_issue` épingle.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, extract::rejection::JsonRejection};
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::skills::SkillRegistry;
use crate::task_state::tasks::NewTask;

use super::state::AppState;
use super::verdict_handler::{EngineDispatchResult, try_engine_dispatch_for};

/// Skill et outil de dispatch pilote — les mêmes que le chemin `block[ci]` du
/// verdict handler, parce que c'est le même travail : corriger une PR ouverte sur
/// sa branche existante.
const ITERATE_TARGET_SKILL: &str = "dev-pilot";
const ITERATE_TARGET_TOOL: &str = "run_claude_pilot";

/// Le nom sous lequel chaque invocation est journalisée **et** auditée.
///
/// **SOLE WRITER** : ce module est le seul site de production qui écrit ce nom,
/// tenu par `canonical_tokens::tests::mika2506_le_nom_daudit_a_un_seul_ecrivain`
/// (allowlist livrée vide). C'est ce qui rend
/// `SELECT after_value, count(*) … GROUP BY 1` exact plutôt qu'un nombre sur
/// lequel deux sites peuvent diverger.
pub const OPERATOR_ITERATE_AUDIT_NAME: &str = "operator_iterate_dispatch";

/// Valeur d'`after_value` du succès, à côté des six motifs de refus.
pub const ITERATE_OUTCOME_DISPATCHED: &str = "dispatched";

// ---------------------------------------------------------------------------
// U3 — le vocabulaire de refus, et c'est un FORMAT DE FIL
// ---------------------------------------------------------------------------

/// Pourquoi une invocation de `mika iterate` a été refusée.
///
/// # Ce sont des valeurs de fil, pas des étiquettes d'affichage
///
/// Elles atterrissent dans `audit_events.after_value` et l'opérateur en fait des
/// `GROUP BY` : **deux orthographes d'un même motif couperaient une population en
/// deux sans le dire.** D'où [`ALL_ITERATE_REFUSAL_REASONS`], un site de
/// définition unique, épinglé par test (V1).
///
/// L'`as_str` est un `match` **exhaustif sans bras `_ =>`** : un septième motif
/// ne compile pas avant d'avoir décidé de son nom de fil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterateRefusal {
    /// Rang 1 — `iteration_context` absent ou vide.
    MissingContext,
    /// Rang 2 — l'issue n'existe pas, est fermée, ou la forge n'a pas répondu.
    IssueUnresolvable,
    /// Rang 3a — zéro PR ouverte trouvable pour cette issue.
    NoOpenPr,
    /// Rang 3b — plus d'une PR ouverte : la cible n'est pas décidable.
    AmbiguousPr,
    /// Rang 4 — la PR est en brouillon ou en conflit.
    PrNotIterable,
    /// Rang 5a — le créneau d'exec de la classe est occupé (transitoire).
    SlotBusy,
    /// Rang 5b — la porte de readiness du moteur a refusé pour une autre raison
    /// que le créneau, et son motif est porté verbatim dans le détail.
    ///
    /// # Pourquoi ce septième motif existe, alors que le plan en énumère six
    ///
    /// Le plan (AC3) fige **six** valeurs, en supposant que
    /// `validate_dispatch_readiness` ne pouvait refuser que
    /// `global_dispatch_active`. La lecture du code dit autre chose : la **porte
    /// de grooming** (mika#1620 / mika#2484) refuse tout dispatch `dev-pilot` sur
    /// une issue portant des callouts de grooming **sans preuve en base**, et la
    /// preuve est purgée à 30 jours. C'est donc le refus que l'opérateur
    /// rencontrera *en premier* sur un ticket un peu ancien ou groomé à la main.
    ///
    /// Le ranger sous `slot_busy` était l'option conservatrice pour le compte, et
    /// c'est une **fausse explication** : elle enverrait l'opérateur vers
    /// `mika tasks promote-deferred`, qui ne fait rien pour un défaut de preuve
    /// de grooming. Un refus qui nomme le mauvais remède est la classe de défaut
    /// que ce ticket même ferme (mika#1971 : *l'instruction envoie au seul
    /// endroit où ça ne peut pas marcher*).
    ///
    /// **Ce que la divergence ne touche pas :** la substance d'AC3 — vocabulaire
    /// à site unique, épinglé par test, ordonné — est tenue à sept exactement
    /// comme à six. Ce qui change est le compte, et il est daté ici.
    ///
    /// **Et la porte n'est PAS élargie.** Lui faire lire `iteration_context`
    /// serait cohérent (c'est déjà le discriminant du garde `check_task_has_open_pr`
    /// juste au-dessus d'elle), mais elle est partagée avec les chemins
    /// `block[ac]` / `block[ci]` du verdict handler, qui passent le même
    /// `iteration_context` : ce serait un changement de comportement de la boucle
    /// autonome habillé en commande opérateur, et le § 11 du plan le refuse
    /// nommément pour un changement voisin. **Suivi**, avec pour précondition une
    /// mesure du nombre de `engine_refused` portant `dispatch_grooming_not_verified`.
    EngineRefused,
}

impl IterateRefusal {
    /// La valeur de fil. `match` exhaustif, aucun bras joker.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingContext => "missing_context",
            Self::IssueUnresolvable => "issue_unresolvable",
            Self::NoOpenPr => "no_open_pr",
            Self::AmbiguousPr => "ambiguous_pr",
            Self::PrNotIterable => "pr_not_iterable",
            Self::SlotBusy => "slot_busy",
            Self::EngineRefused => "engine_refused",
        }
    }
}

/// Les motifs, un seul site de définition (V1).
///
/// Sept, pas six : voir [`IterateRefusal::EngineRefused`] pour la divergence
/// datée avec l'AC3 du plan et pourquoi la ranger sous `slot_busy` aurait été une
/// fausse explication.
pub const ALL_ITERATE_REFUSAL_REASONS: &[&str] = &[
    "missing_context",
    "issue_unresolvable",
    "no_open_pr",
    "ambiguous_pr",
    "pr_not_iterable",
    "slot_busy",
    "engine_refused",
];

/// État de l'issue tel que la forge l'a rendu. `None` chez l'appelant signifie
/// « la forge n'a pas répondu », ce qui est **aussi** un refus de rang 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}

/// Ce que l'issue porte, pour les deux seules choses qu'on en lit : son état et
/// le callout de branche de son corps.
#[derive(Debug, Clone)]
pub struct IssueView {
    pub state: IssueState,
    pub body: String,
}

/// Une PR ouverte sur la branche dérivée, réduite aux quatre champs dont la
/// décision a besoin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSnapshot {
    pub number: u64,
    pub url: String,
    pub is_draft: bool,
    /// `MERGEABLE` / `CONFLICTING` / `UNKNOWN`, tel que `gh` le rend.
    pub mergeable: String,
}

/// Les quatre premiers termes, **purs et ordonnés** (U3).
///
/// # L'ordre est un livrable, pas un détail d'implémentation
///
/// Un `cwd` valant `$MIKA_PLATFORM_DIR/…` est *aussi* non-absolu et *aussi*
/// inexistant — les trois énoncés sont vrais, seul le premier désigne la faute
/// (mika#2536). Même forme ici : une invocation **sans contexte** sur une issue
/// **sans PR** doit rendre `missing_context`, pas `no_open_pr`. Vrai, et
/// strictement moins utile : c'est un refus qui envoie l'opérateur chercher une
/// PR au lieu de corriger son invocation.
///
/// # Le sens du fail-closed, et il ne se transporte pas
///
/// L'action est une **écriture sur une branche portant une PR revue** : un refus
/// à tort coûte une invocation — visible, rattrapable, bornée ; un passage à tort
/// peut ré-écrire une implémentation revue. C'est l'arbitrage de mika#2520, et
/// l'**inverse** de celui du faucheur mika#2420 (où un signal illisible
/// *conserve*), parce que là-bas l'action détruisait du travail et ici l'action
/// *est* l'écriture. **L'arbitrage est local.**
///
/// # Ce que `open_prs = Some(&[])` couvre, et c'est la limite nommée du § 10
///
/// Une issue dont le callout `> - **Branch:**` est absent n'a **pas de branche à
/// interroger**, donc zéro PR trouvable : l'appelant passe une liste vide et le
/// motif est `no_open_pr`. C'est exactement la lecture pour laquelle la Halte 1
/// du plan est écrite — *ne pas élargir la recherche de PR par réflexe, établir
/// d'abord la dérivation.*
pub fn decide_iterate_target<'a>(
    iteration_context: &str,
    issue_state: Option<IssueState>,
    open_prs: Option<&'a [PrSnapshot]>,
) -> Result<&'a PrSnapshot, IterateRefusal> {
    // Rang 1 — le seul refus GRATUIT : aucun appel réseau, aucune lecture de
    // base. Et c'est la faute la plus probable de l'opérateur, la voie free-text
    // de `run_claude_pilot` **crashant** la session (self-dev Rule 4).
    if iteration_context.trim().is_empty() {
        return Err(IterateRefusal::MissingContext);
    }

    // Rang 2 — avant de chercher une PR, établir qu'il y a une issue. Une issue
    // fermée est le cas mika#988, déjà traité en aval par un auto-skip : la
    // refuser ici évite un dispatch qui se saborde. Une forge qui ne répond pas
    // tombe ici aussi : on ne peut rien établir, donc on refuse (fail-closed).
    match issue_state {
        Some(IssueState::Open) => {}
        Some(IssueState::Closed) | None => return Err(IterateRefusal::IssueUnresolvable),
    }

    // Rang 3 — `None` = la forge n'a pas répondu sur la liste des PR ; c'est le
    // même refus de rang 2 (rien n'est établi), jamais un « zéro PR », qui
    // serait une affirmation fausse.
    let prs = match open_prs {
        Some(prs) => prs,
        None => return Err(IterateRefusal::IssueUnresolvable),
    };
    match prs.len() {
        // Zéro = il n'y a rien à itérer ; un dispatch serait un implement neuf,
        // c'est-à-dire le défaut de ce ticket réintroduit.
        0 => return Err(IterateRefusal::NoOpenPr),
        1 => {}
        // Plus d'une = la cible n'est pas décidable, et choisir serait deviner.
        _ => return Err(IterateRefusal::AmbiguousPr),
    }
    let pr = &prs[0];

    // Rang 4 — un brouillon a sa propre voie (`wip_rescue` → `gh pr ready`) ;
    // une PR en conflit relève de `resolve-pr-conflicts`, dont le geste est
    // différent. `UNKNOWN` n'est PAS un refus : GitHub le rend pendant qu'il
    // calcule, et refuser là rendrait la commande inutilisable sur une PR
    // fraîchement poussée.
    if pr.is_draft || pr.mergeable.eq_ignore_ascii_case("CONFLICTING") {
        return Err(IterateRefusal::PrNotIterable);
    }

    Ok(pr)
}

// ---------------------------------------------------------------------------
// La requête et son issue
// ---------------------------------------------------------------------------

/// Ce que l'opérateur demande.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct IterateRequest {
    /// `mika` ou `senara-solutions/mika` — les deux formes sont acceptées, la
    /// normalisation vit dans [`crate::webhook_dispatch::normalize_owner_repo`]
    /// et nulle part ailleurs.
    pub repo: String,
    /// Le numéro d'**issue** (§ 3), jamais celui de la PR.
    pub issue: u64,
    pub iteration_context: String,
}

impl IterateRequest {
    /// `owner/repo`, pour `gh --repo`.
    pub fn owner_repo(&self) -> String {
        crate::webhook_dispatch::normalize_owner_repo(&self.repo)
    }

    /// Le nom nu, pour `prompt: "<repo>#<N>"` — la seule forme que le parseur de
    /// worktree de dispatch-lib accepte (mika#1593).
    pub fn repo_name(&self) -> String {
        match self.repo.rsplit_once('/') {
            Some((_, name)) => name.to_string(),
            None => self.repo.clone(),
        }
    }

    /// L'URL de l'issue — ce qui fait entrer la tâche dans l'index de dédup
    /// actif, donc un second `mika iterate` collisionne au lieu d'ouvrir un
    /// second pilote.
    pub fn issue_url(&self) -> String {
        format!(
            "https://github.com/{}/issues/{}",
            self.owner_repo(),
            self.issue
        )
    }
}

/// Ce que l'invocation a produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterateOutcome {
    /// Un pilote tourne sur la branche de la PR.
    Dispatched {
        task_id: String,
        callback_task_id: String,
        pr_url: String,
    },
    /// Rien n'a été lancé, et le motif est nommé.
    Refused {
        reason: IterateRefusal,
        detail: String,
    },
}

impl IterateOutcome {
    /// La valeur de fil de l'issue, pour `audit_events.after_value`.
    pub fn wire_outcome(&self) -> &'static str {
        match self {
            Self::Dispatched { .. } => ITERATE_OUTCOME_DISPATCHED,
            Self::Refused { reason, .. } => reason.as_str(),
        }
    }

    fn refused(reason: IterateRefusal, detail: impl Into<String>) -> Self {
        Self::Refused {
            reason,
            detail: detail.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// U5/U6 — l'orchestration
// ---------------------------------------------------------------------------

/// Chemin de production : lit la forge par `gh`, puis délègue.
pub async fn dispatch_iteration(
    db: &AsyncDatabase,
    skills: &SkillRegistry,
    github_token: Option<&str>,
    session_id: &str,
    trace_id: &str,
    req: &IterateRequest,
) -> IterateOutcome {
    let token = github_token.unwrap_or_default().to_string();
    let view_token = token.clone();

    dispatch_iteration_with_forge(
        db,
        skills,
        github_token,
        session_id,
        trace_id,
        req,
        move |owner_repo: String, number: u64| async move {
            gh_view_issue(&owner_repo, number, &view_token).await
        },
        move |owner_repo: String, branch: String| async move {
            gh_list_open_prs(&owner_repo, &branch, &token).await
        },
    )
    .await
}

/// La même décision, avec la forge injectée.
///
/// Les deux sondes sont des fermetures plutôt qu'un trait : c'est le motif de
/// `try_handle_ready_label_dispatch_with_fetcher`, et il permet de piloter la
/// décision contre des charges gelées **sans réseau**, ce dont V2 a besoin pour
/// voir chacun des cinq rangs rouge par mutation.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_iteration_with_forge<VF, VFut, LF, LFut>(
    db: &AsyncDatabase,
    skills: &SkillRegistry,
    github_token: Option<&str>,
    session_id: &str,
    trace_id: &str,
    req: &IterateRequest,
    view_issue: VF,
    list_open_prs: LF,
) -> IterateOutcome
where
    VF: FnOnce(String, u64) -> VFut,
    VFut: std::future::Future<Output = Result<Option<IssueView>, String>>,
    LF: FnOnce(String, String) -> LFut,
    LFut: std::future::Future<Output = Result<Vec<PrSnapshot>, String>>,
{
    let owner_repo = req.owner_repo();

    // Rang 1 d'abord, et il ne coûte rien : pas de `gh`, pas de base. L'ordre
    // n'est pas une optimisation — il décide QUEL motif l'opérateur lit.
    if req.iteration_context.trim().is_empty() {
        return finish(
            db,
            session_id,
            trace_id,
            req,
            IterateOutcome::refused(
                IterateRefusal::MissingContext,
                "`iteration_context` est vide : la voie free-text de \
                 `run_claude_pilot` n'a pas de worktree et la session crasherait \
                 (self-dev Rule 4). Décris le correctif attendu.",
            ),
        )
        .await;
    }

    // Rang 2 — l'issue.
    let issue = match view_issue(owner_repo.clone(), req.issue).await {
        Ok(Some(issue)) => issue,
        Ok(None) => {
            return finish(
                db,
                session_id,
                trace_id,
                req,
                IterateOutcome::refused(
                    IterateRefusal::IssueUnresolvable,
                    format!("issue {owner_repo}#{} introuvable", req.issue),
                ),
            )
            .await;
        }
        Err(e) => {
            return finish(
                db,
                session_id,
                trace_id,
                req,
                IterateOutcome::refused(
                    IterateRefusal::IssueUnresolvable,
                    format!("la forge n'a pas répondu sur l'issue : {e}"),
                ),
            )
            .await;
        }
    };
    if issue.state != IssueState::Open {
        return finish(
            db,
            session_id,
            trace_id,
            req,
            IterateOutcome::refused(
                IterateRefusal::IssueUnresolvable,
                format!("issue {owner_repo}#{} est fermée", req.issue),
            ),
        )
        .await;
    }

    // La branche est **lue dans le corps de l'issue**, jamais devinée : le lecteur
    // unique du callout est `auto_pull::extract_branch_name`, et deviner une
    // branche est la faute que `derive-branch-name` existe pour ne pas commettre
    // (mika-platform#58). Absente ⇒ aucune branche à interroger ⇒ zéro PR
    // trouvable, ce qui est la Halte 1 du plan.
    let branch = crate::auto_pull::extract_branch_name(&issue.body);
    let prs: Option<Vec<PrSnapshot>> = match &branch {
        None => Some(Vec::new()),
        Some(branch) => match list_open_prs(owner_repo.clone(), branch.clone()).await {
            Ok(prs) => Some(prs),
            Err(e) => {
                return finish(
                    db,
                    session_id,
                    trace_id,
                    req,
                    IterateOutcome::refused(
                        IterateRefusal::IssueUnresolvable,
                        format!("la forge n'a pas répondu sur les PR de `{branch}` : {e}"),
                    ),
                )
                .await;
            }
        },
    };

    // Rangs 3 et 4, par le décideur pur — le site que V2 asserte rang par rang.
    let pr = match decide_iterate_target(
        &req.iteration_context,
        Some(IssueState::Open),
        prs.as_deref(),
    ) {
        Ok(pr) => pr.clone(),
        Err(reason) => {
            let detail = match reason {
                IterateRefusal::NoOpenPr => match &branch {
                    Some(b) => format!(
                        "aucune PR ouverte sur `{b}` — un dispatch y serait un \
                         implement NEUF, pas une itération"
                    ),
                    None => format!(
                        "l'issue {owner_repo}#{} ne porte pas de callout \
                         `> - **Branch:**` : il n'y a pas de branche à interroger",
                        req.issue
                    ),
                },
                IterateRefusal::AmbiguousPr => {
                    let n = prs.as_ref().map(|p| p.len()).unwrap_or(0);
                    format!(
                        "{n} PR ouvertes sur la branche dérivée — la cible n'est \
                         pas décidable, et choisir serait deviner"
                    )
                }
                IterateRefusal::PrNotIterable => {
                    "la PR est en brouillon (voie `wip_rescue` → `gh pr ready`) ou \
                     en conflit (voie `resolve-pr-conflicts`)"
                        .to_string()
                }
                other => other.as_str().to_string(),
            };
            return finish(
                db,
                session_id,
                trace_id,
                req,
                IterateOutcome::refused(reason, detail),
            )
            .await;
        }
    };

    // U5 — l'identité de la tâche. La tâche d'origine est `completed`, terminale :
    // `validate_dispatch_readiness` check (1) exige `pending`/`in_progress`, donc
    // **on ne peut pas dispatcher sous elle**.
    let outcome = create_and_dispatch(db, skills, github_token, session_id, trace_id, req, &pr)
        .await
        .unwrap_or_else(|e| e);

    finish(db, session_id, trace_id, req, outcome).await
}

/// Crée la tâche parente auto-descriptive et enchaîne sur le dispatch moteur.
///
/// Rend `Err(IterateOutcome)` sur les chemins de refus pour garder le corps
/// linéaire — l'appelant aplatit.
async fn create_and_dispatch(
    db: &AsyncDatabase,
    skills: &SkillRegistry,
    github_token: Option<&str>,
    session_id: &str,
    trace_id: &str,
    req: &IterateRequest,
    pr: &PrSnapshot,
) -> Result<IterateOutcome, IterateOutcome> {
    let owner_repo = req.owner_repo();
    let issue_url = req.issue_url();
    let task_label = format!("operator-iterate: {owner_repo}#{}", req.issue);

    // **Le pré-estampage est le point non évident.** Il rend la tâche
    // auto-descriptive DÈS SA CRÉATION : `mika tasks get` dit sur quelle PR elle
    // itère, sans attendre la ligne `PR:` du callback — très exactement la
    // corrélation que l'opérateur a dû faire à la main le 2026-09-23. Il arme
    // aussi déterministe le chemin du parent-completer (mika#1162, prédicat
    // `pr_url IS NOT NULL`) plutôt que de faire dépendre la résolution de la
    // tâche d'un `gh pr list --head` en aval.
    let metadata = serde_json::json!({ "claude_pilot": { "pr_url": pr.url } }).to_string();

    let new_task = NewTask {
        agent_id: db.agent_id().to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: task_label,
        trigger_type: "manual".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "none".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: Some(session_id.to_string()),
        created_trace_id: Some(trace_id.to_string()),
        reference_url: Some(issue_url.clone()),
        source: Some("self_dev".to_string()),
        metadata: Some(metadata),
        r#type: Some("issue".to_string()),
        dispatch_class: Some("implement".to_string()),
    };

    let task_id = match db.create_task(new_task).await {
        Ok(id) => id,
        Err(e) => {
            // **Ne JAMAIS affirmer la collision sans l'avoir établie.** La cause
            // dominante ici est bien un conflit sur
            // `idx_tasks_manual_active_ref_url` — une tâche active tient déjà le
            // créneau de cette issue, c'est-à-dire qu'une itération est en vol,
            // et c'est la propriété que `reference_url` achète. Mais
            // `create_task` échoue aussi pour d'autres raisons, et un détail qui
            // nomme une tâche bloquante inexistante envoie l'opérateur chercher
            // un pilote qui n'existe pas. On regarde, puis on dit — motif
            // `describe_blocking_task` (mika#2045).
            let blocker = match db.find_active_task_by_ref_url(&issue_url).await {
                Ok(t) => t,
                Err(lookup) => {
                    warn!(
                        event = "operator_iterate_blocker_lookup_failed",
                        issue_url = %issue_url,
                        error = %lookup,
                        "iterate: impossible d'établir la tâche bloquante"
                    );
                    None
                }
            };
            let detail = match blocker {
                Some(t) => format!(
                    "la tâche {} ({}) tient déjà le créneau de {issue_url} — une \
                     itération est en vol. Pour reprendre la main : \
                     `mika tasks get {}`.",
                    t.id, t.status, t.id
                ),
                None => {
                    return Err(IterateOutcome::refused(
                        IterateRefusal::EngineRefused,
                        format!(
                            "la création de la tâche a échoué et aucune tâche \
                             active ne tient le créneau de {issue_url} : {e}"
                        ),
                    ));
                }
            };
            return Err(IterateOutcome::refused(IterateRefusal::SlotBusy, detail));
        }
    };

    // `dispatcher_source = 'operator'` donne à la tâche la **priorité opérateur**
    // de `promote_pending_deferred_if_idle`, qui se retire quand l'opérateur a du
    // `pending` dans la classe : un opérateur qui demande une itération ne doit
    // pas être affamé derrière les wrappers de la boucle. Écriture séparée à
    // dessein (mika#1948) — `NewTask` n'en porte pas le champ.
    if let Err(e) = db.set_task_dispatcher_source(&task_id, "operator").await {
        warn!(
            event = "operator_iterate_dispatcher_source_failed",
            task_id = %task_id,
            error = %e,
            "iterate: dispatcher_source non écrit (non fatal — la priorité \
             opérateur est perdue, le dispatch continue)"
        );
    }

    match try_engine_dispatch_for(
        db,
        skills,
        &task_id,
        github_token,
        &owner_repo,
        // § 3 — le numéro d'ISSUE, jamais celui de la PR.
        req.issue,
        ITERATE_TARGET_SKILL,
        ITERATE_TARGET_TOOL,
        &req.iteration_context,
        session_id,
        trace_id,
        Some("operator_iterate_engine_dispatched"),
    )
    .await
    {
        EngineDispatchResult::Spawned { callback_task_id } => Ok(IterateOutcome::Dispatched {
            task_id,
            callback_task_id,
            pr_url: pr.url.clone(),
        }),
        // U6 — **divergence délibérée** : `try_engine_dispatch_for` rend
        // `Deferred` sur créneau occupé et enregistre un wrapper. Pour un geste
        // d'OPÉRATEUR on refuse en nommant le porteur : un geste différé part
        // quelques minutes plus tard, sans personne qui regarde, alors que
        // l'opérateur a tapé une commande en attendant une réponse. Et
        // l'échappatoire existe déjà et se nomme. **Aucune ligne de
        // `try_engine_dispatch_for` ne change pour ça** — la fonction rend un
        // enum, ce nouvel appelant traite `Deferred` à sa façon. C'est ce qui
        // garde R8 vrai.
        EngineDispatchResult::Deferred { deferred_task_id } => Err(IterateOutcome::refused(
            IterateRefusal::SlotBusy,
            format!(
                "le créneau d'exec `implement` est occupé ; un wrapper différé a \
                 été enregistré ({deferred_task_id}). Pour forcer : \
                 `mika tasks promote-deferred implement --override`."
            ),
        )),
        // Le motif du moteur est porté **verbatim** : il porte son propre
        // `recovery`, et le paraphraser perdrait le geste qu'il nomme.
        EngineDispatchResult::Fallback { reason } => Err(IterateOutcome::refused(
            IterateRefusal::EngineRefused,
            format!("la porte de readiness du moteur a refusé : {reason}"),
        )),
    }
}

/// Journalise et audite l'issue de l'invocation, quelle qu'elle soit (R9).
///
/// Une seule sortie, donc un seul site d'écriture du nom d'audit — ce qui est la
/// condition du SOLE WRITER.
async fn finish(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
    req: &IterateRequest,
    outcome: IterateOutcome,
) -> IterateOutcome {
    let owner_repo = req.owner_repo();
    let wire = outcome.wire_outcome();

    match &outcome {
        IterateOutcome::Dispatched {
            task_id,
            callback_task_id,
            pr_url,
        } => info!(
            event = OPERATOR_ITERATE_AUDIT_NAME,
            repo = %owner_repo,
            issue = req.issue,
            outcome = wire,
            task_id = %task_id,
            callback_task_id = %callback_task_id,
            pr_url = %pr_url,
            "iterate: pilote lancé sur la branche de la PR ouverte"
        ),
        IterateOutcome::Refused { reason, detail } => warn!(
            event = OPERATOR_ITERATE_AUDIT_NAME,
            repo = %owner_repo,
            issue = req.issue,
            outcome = reason.as_str(),
            detail = %detail,
            "iterate: refusé"
        ),
    }

    // Fire-and-forget : une écriture d'audit qui échoue ne doit pas pouvoir
    // changer l'issue d'un dispatch.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            OPERATOR_ITERATE_AUDIT_NAME,
            &format!("issue:{owner_repo}#{}", req.issue),
            None,
            Some(wire),
            Some(&match &outcome {
                IterateOutcome::Dispatched {
                    task_id, pr_url, ..
                } => format!("task_id={task_id} pr_url={pr_url}"),
                IterateOutcome::Refused { detail, .. } => detail.clone(),
            }),
            Some(trace_id),
        )
        .await
    {
        warn!(
            event = "operator_iterate_audit_failed",
            error = %e,
            "iterate: ligne d'audit non écrite (le WARN ci-dessus est passé)"
        );
    }

    outcome
}

// ---------------------------------------------------------------------------
// Les deux sondes `gh` du chemin de production
// ---------------------------------------------------------------------------

async fn gh_view_issue(
    owner_repo: &str,
    number: u64,
    token: &str,
) -> Result<Option<IssueView>, String> {
    let number_s = number.to_string();
    let args = vec![
        "issue",
        "view",
        &number_s,
        "--repo",
        owner_repo,
        "--json",
        "state,body",
    ];
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::tools::pr_merge_with_gate::run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh issue view timed out after 60s".to_string())?;

    let out = match out {
        Ok(out) => out,
        Err(e) if e.to_lowercase().contains("could not resolve to an issue") => return Ok(None),
        Err(e) => return Err(e),
    };

    let v: serde_json::Value =
        serde_json::from_str(out.trim()).map_err(|e| format!("gh issue view: {e}"))?;
    // Un état illisible n'est pas `Open` : on ne peut pas l'établir, donc on
    // refuse (fail-closed).
    let state = match v["state"].as_str() {
        Some(s) if s.eq_ignore_ascii_case("OPEN") => IssueState::Open,
        Some(_) => IssueState::Closed,
        None => return Err("gh issue view: pas de champ `state`".to_string()),
    };
    Ok(Some(IssueView {
        state,
        body: v["body"].as_str().unwrap_or_default().to_string(),
    }))
}

async fn gh_list_open_prs(
    owner_repo: &str,
    branch: &str,
    token: &str,
) -> Result<Vec<PrSnapshot>, String> {
    let args = vec![
        "pr",
        "list",
        "--repo",
        owner_repo,
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,url,isDraft,mergeable",
    ];
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::tools::pr_merge_with_gate::run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh pr list timed out after 60s".to_string())??;

    parse_open_prs(out.trim())
}

/// Séparé de l'appel pour que la lecture soit testable contre des charges gelées.
pub(crate) fn parse_open_prs(stdout: &str) -> Result<Vec<PrSnapshot>, String> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(stdout).map_err(|e| format!("gh pr list: {e}"))?;
    arr.into_iter()
        .map(|v| {
            // Un champ illisible sort la PR de la population par un refus, jamais
            // par un défaut permissif : un `isDraft` manquant lu comme `false`
            // ferait itérer sur un brouillon.
            Ok(PrSnapshot {
                number: v["number"]
                    .as_u64()
                    .ok_or_else(|| "gh pr list: `number` illisible".to_string())?,
                url: v["url"]
                    .as_str()
                    .ok_or_else(|| "gh pr list: `url` illisible".to_string())?
                    .to_string(),
                is_draft: v["isDraft"]
                    .as_bool()
                    .ok_or_else(|| "gh pr list: `isDraft` illisible".to_string())?,
                mergeable: v["mergeable"].as_str().unwrap_or("UNKNOWN").to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// U2 — la route de mutation
// ---------------------------------------------------------------------------

/// `POST /api/v1/agents/{id}/iterate` — le geste déterministe (R1).
///
/// # Pourquoi la CLI ne fait rien elle-même
///
/// Une CLI qui ouvrirait la base et spawnerait le pilote deviendrait un **second
/// dispatcher hors du démon** : il faudrait y répliquer `SkillRegistry`, la
/// résolution du répertoire de skill et celle du jeton — et ça contredirait
/// mika#1727, qui a fait de la CLI un client mince précisément pour que
/// l'exécution ait un seul propriétaire. Le précédent tentant
/// (`mika tasks promote-deferred`, qui ouvre bien la base) est écarté parce que
/// son action est une **écriture de ligne**, pas un **spawn de pilote** : ni skill
/// à résoudre, ni sous-processus à détacher, ni callback à faire revenir.
///
/// # 404 = « ce serveur n'a pas résolu cet agent », jamais un défaut
///
/// La lecture passe par la carte des agents **résolus**, délibérément pas par
/// `resolve_agent`, qui construirait un agent (base, skills, task engine, KG) en
/// effet de bord — ce que la route budget voisine refuse déjà nommément.
pub async fn handle_agent_iterate(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    body: Result<Json<IterateRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("corps de requête illisible: {rejection}"),
                })),
            )
                .into_response();
        }
    };

    let Some(agent_state) = state.agents.get(&agent_id).map(|a| a.clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("agent '{agent_id}' is not resolved on this server"),
            })),
        )
            .into_response();
    };

    let trace_id = uuid::Uuid::new_v4().simple().to_string();
    let session_id = format!("operator-iterate-{trace_id}");

    let token = agent_state
        .settings
        .resolve_github_token(agent_state.github_app.as_deref())
        .await;

    // Le registre est cloné hors du mutex avant l'`await` : le garder verrouillé
    // le temps d'un dispatch (deux allers-retours `gh` compris) sérialiserait
    // tout autre tour de cet agent.
    let skills = {
        let guard = agent_state
            .skills
            .lock()
            .expect("skill registry mutex poisoned");
        guard.clone()
    };

    let outcome = dispatch_iteration(
        &agent_state.db,
        &skills,
        token.as_deref(),
        &session_id,
        &trace_id,
        &req,
    )
    .await;

    match outcome {
        IterateOutcome::Dispatched {
            task_id,
            callback_task_id,
            pr_url,
        } => (
            StatusCode::OK,
            Json(serde_json::json!({
                "outcome": ITERATE_OUTCOME_DISPATCHED,
                "task_id": task_id,
                "callback_task_id": callback_task_id,
                "pr_url": pr_url,
                "trace_id": trace_id,
            })),
        )
            .into_response(),
        IterateOutcome::Refused { reason, detail } => (
            // `409 Conflict` pour tous les refus, `slot_busy` compris : ce sont
            // des refus d'**état**, pas des requêtes malformées. Un `400` sur
            // `no_open_pr` enverrait l'opérateur relire son invocation, qui est
            // correcte.
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "outcome": "refused",
                "refusal_reason": reason.as_str(),
                "detail": detail,
                "trace_id": trace_id,
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, is_draft: bool, mergeable: &str) -> PrSnapshot {
        PrSnapshot {
            number,
            url: format!("https://github.com/senara-solutions/mika/pull/{number}"),
            is_draft,
            mergeable: mergeable.to_string(),
        }
    }

    // ── V1 — le vocabulaire est un format de fil ───────────────────────────

    /// Les six valeurs vivent à un seul endroit, et l'`as_str` ne peut pas en
    /// inventer une septième sans compiler.
    #[test]
    fn mika2506_les_six_motifs_sont_un_format_de_fil() {
        let from_enum: Vec<&str> = [
            IterateRefusal::MissingContext,
            IterateRefusal::IssueUnresolvable,
            IterateRefusal::NoOpenPr,
            IterateRefusal::AmbiguousPr,
            IterateRefusal::PrNotIterable,
            IterateRefusal::SlotBusy,
            IterateRefusal::EngineRefused,
        ]
        .iter()
        .map(|r| r.as_str())
        .collect();

        assert_eq!(
            from_enum, ALL_ITERATE_REFUSAL_REASONS,
            "mika#2506 V1 — les motifs atterrissent dans `audit_events.after_value` \
             et l'opérateur en fait des GROUP BY : deux orthographes d'un même \
             motif couperaient une population en deux sans le dire. Un renommage \
             est une rupture à DATER dans CLAUDE.md, jamais une mise à jour de \
             test en silence."
        );

        // `dispatched` n'est pas un motif de refus, et ne doit pas le devenir.
        assert!(!ALL_ITERATE_REFUSAL_REASONS.contains(&ITERATE_OUTCOME_DISPATCHED));
    }

    // ── V2 — l'ordre, rang par rang ────────────────────────────────────────

    /// **L'assertion qui distingue « l'ordre est celui-là » de « les deux termes
    /// sont vrais ».** Sans contexte ET sans PR, le motif doit être le contexte :
    /// `no_open_pr` serait vrai et strictement moins utile — c'est un refus qui
    /// envoie l'opérateur chercher une PR au lieu de corriger son invocation.
    #[test]
    fn mika2506_rang1_le_contexte_precede_labsence_de_pr() {
        assert_eq!(
            decide_iterate_target("   ", Some(IssueState::Open), Some(&[])),
            Err(IterateRefusal::MissingContext)
        );
        // Et il précède AUSSI une issue irrésolvable.
        assert_eq!(
            decide_iterate_target("", None, None),
            Err(IterateRefusal::MissingContext)
        );
    }

    /// Rang 2 avant rang 3 : sans issue établie, on ne rapporte pas une histoire
    /// de PR.
    #[test]
    fn mika2506_rang2_lissue_precede_la_pr() {
        let prs = [pr(2504, false, "MERGEABLE")];
        assert_eq!(
            decide_iterate_target("fix the lint", Some(IssueState::Closed), Some(&prs)),
            Err(IterateRefusal::IssueUnresolvable),
            "une issue fermée est refusée même quand une PR itérable existe"
        );
        assert_eq!(
            decide_iterate_target("fix the lint", None, Some(&prs)),
            Err(IterateRefusal::IssueUnresolvable),
            "une forge muette n'établit rien : fail-closed"
        );
    }

    /// Rang 3 — zéro et plus-d'une sont deux motifs distincts, et une liste
    /// illisible n'est JAMAIS « zéro PR ».
    #[test]
    fn mika2506_rang3_zero_plusieurs_et_illisible() {
        assert_eq!(
            decide_iterate_target("fix the lint", Some(IssueState::Open), Some(&[])),
            Err(IterateRefusal::NoOpenPr)
        );
        let two = [pr(2504, false, "MERGEABLE"), pr(2505, false, "MERGEABLE")];
        assert_eq!(
            decide_iterate_target("fix the lint", Some(IssueState::Open), Some(&two)),
            Err(IterateRefusal::AmbiguousPr)
        );
        assert_eq!(
            decide_iterate_target("fix the lint", Some(IssueState::Open), None),
            Err(IterateRefusal::IssueUnresolvable),
            "une liste illisible est un refus de rang 2, pas une affirmation de \
             zéro PR — qui serait fausse"
        );
    }

    /// Rang 4 — brouillon et conflit refusent ; `UNKNOWN` ne refuse PAS.
    #[test]
    fn mika2506_rang4_brouillon_et_conflit() {
        let draft = [pr(2504, true, "MERGEABLE")];
        assert_eq!(
            decide_iterate_target("fix", Some(IssueState::Open), Some(&draft)),
            Err(IterateRefusal::PrNotIterable)
        );
        let conflicting = [pr(2504, false, "CONFLICTING")];
        assert_eq!(
            decide_iterate_target("fix", Some(IssueState::Open), Some(&conflicting)),
            Err(IterateRefusal::PrNotIterable)
        );

        // Contrôle négatif : `UNKNOWN` est ce que GitHub rend pendant qu'il
        // calcule. Le refuser rendrait la commande inutilisable sur une PR
        // fraîchement poussée.
        let unknown = [pr(2504, false, "UNKNOWN")];
        assert_eq!(
            decide_iterate_target("fix", Some(IssueState::Open), Some(&unknown)),
            Ok(&unknown[0])
        );
    }

    /// Le chemin nominal — sans ce contrôle positif, les quatre tests ci-dessus
    /// seraient satisfaits par un décideur qui refuse tout.
    #[test]
    fn mika2506_chemin_nominal() {
        let prs = [pr(2504, false, "MERGEABLE")];
        assert_eq!(
            decide_iterate_target(
                "rebase + fix the SIGPIPE lint",
                Some(IssueState::Open),
                Some(&prs)
            ),
            Ok(&prs[0])
        );
    }

    // ── Lecture de la forge ────────────────────────────────────────────────

    #[test]
    fn mika2506_une_liste_vide_est_zero_pr() {
        assert_eq!(parse_open_prs("").unwrap(), vec![]);
        assert_eq!(parse_open_prs("[]").unwrap(), vec![]);
    }

    #[test]
    fn mika2506_un_champ_illisible_refuse_au_lieu_de_defaulter() {
        // `isDraft` absent : un défaut `false` ferait itérer sur un brouillon.
        let err = parse_open_prs(r#"[{"number":1,"url":"u","mergeable":"MERGEABLE"}]"#)
            .expect_err("un `isDraft` absent doit refuser");
        assert!(
            err.contains("isDraft"),
            "le motif doit nommer le champ : {err}"
        );
    }

    #[test]
    fn mika2506_mergeable_absent_vaut_unknown_donc_iterable() {
        // Seul `mergeable` tolère l'absence, et c'est cohérent avec le rang 4 :
        // GitHub l'omet pendant le calcul, et `UNKNOWN` n'est pas un refus.
        let prs = parse_open_prs(r#"[{"number":1,"url":"u","isDraft":false}]"#).unwrap();
        assert_eq!(prs[0].mergeable, "UNKNOWN");
    }

    // ── La requête ─────────────────────────────────────────────────────────

    #[test]
    fn mika2506_les_deux_formes_de_repo_sont_acceptees() {
        for repo in ["mika", "senara-solutions/mika"] {
            let req = IterateRequest {
                repo: repo.to_string(),
                issue: 2503,
                iteration_context: "x".to_string(),
            };
            assert_eq!(req.owner_repo(), "senara-solutions/mika");
            // Le `prompt` de dispatch-lib n'accepte que la forme nue (mika#1593).
            assert_eq!(req.repo_name(), "mika");
            assert_eq!(
                req.issue_url(),
                "https://github.com/senara-solutions/mika/issues/2503"
            );
        }
    }

    #[test]
    fn mika2506_le_succes_a_sa_propre_valeur_de_fil() {
        let dispatched = IterateOutcome::Dispatched {
            task_id: "t".into(),
            callback_task_id: "c".into(),
            pr_url: "u".into(),
        };
        assert_eq!(dispatched.wire_outcome(), "dispatched");
        assert_eq!(
            IterateOutcome::refused(IterateRefusal::SlotBusy, "x").wire_outcome(),
            "slot_busy"
        );
    }
}

//! Reader — composes milestone state from `gh` CLI output.
//!
//! **LECTURE SEULE.** Every subprocess spawned here is a read-only `gh`
//! subcommand (`view`, `list`). No `gh api PATCH/POST/DELETE`, no `gh issue
//! edit`, no `gh pr merge`. The `no_dispatch_test` in the module root
//! enforces this structurally.
//!
//! **Wrapper-only INV-2.** The subprocess shape mirrors
//! `crate::auto_pull::gh_list_open_issues` verbatim (env `GH_TOKEN`,
//! `stdin::null`, `kill_on_drop(true)`). No new GitHub client dep.

use super::plan_size;
use super::pool::{PlanSizeReading, PoolCandidate, ReadyTicket, UnreadCause};
use super::types::{
    CiState, IssueState, MilestoneRef, MilestoneState, PrLinkSource, ProgressCounts,
    RecentActivity, SubIssue,
};
use crate::auto_pull::extract_branch_name;
use crate::plan_callout::{FenceHandling, plan_callout};
use crate::webhook_dispatch::READY_LABEL;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Combien d'issues ouvertes la lecture **hors milestone cible** rapporte au
/// plus (mika#2699, vol 2).
///
/// C'est la borne du `--limit` de la cinquième lecture `gh`, et c'est aussi le
/// seuil de sa garde de troncature : une page pleine ne dit pas « voici les
/// candidats », elle dit « il y en avait peut-être d'autres ». Le dépôt en porte
/// aujourd'hui quelques centaines d'ouvertes, donc la page pleine est le cas
/// nominal et non l'exception — raison pour laquelle la garde existe ici alors
/// que [`compose_ready_pool`] s'en passe (son ensemble cible 2–4).
///
/// Public parce que le rapporteur l'épelle dans la ligne de troncature : le
/// nombre que l'opérateur lit doit être celui que la lecture a demandé, jamais
/// une copie qui peut dériver.
pub const OUTSIDE_READ_LIMIT: usize = 100;

/// Reader — owns the `gh` invocations and composes `MilestoneState`.
///
/// `token` is an optional GitHub token. When `Some`, injected as
/// `GH_TOKEN` env var on every subprocess. When `None`, `gh` falls back
/// to the host user's `~/.config/gh/hosts.yml` (dev-mode friendly).
pub struct Reader {
    token: Option<String>,
}

/// Trait for executing `gh` commands — injectable for tests so the pure
/// composition logic can be exercised without live network calls.
#[async_trait::async_trait]
pub trait GhRunner: Send + Sync {
    async fn run(&self, args: &[&str]) -> Result<String>;
}

/// mika#1975 (AC3, plan D5) — a failed `gh` invocation, **typed**.
///
/// **Why the two halves travel apart.** The classifier in `spawn.rs`
/// (`classify_cycle_error`) used to run over the *rendered* error string, which
/// carried the command line as well as the stderr. That line holds numbers an
/// operator chooses — `/repos/{owner}/{repo}/milestones/{N}`, `--milestone {N}`,
/// `--search milestone:{N}` — so with
/// `MIKA_MANAGER_TARGET_MILESTONE=owner/repo#403` **every** failure of that call
/// (a 500, a DNS error whose wording matches no `Network` pattern, a JSON parse
/// failure) classified as `Forbidden`, reached `is_auth_failure`, and fired the
/// 30-minute auth alarm with the wrong class and the wrong hint. Keeping `args`
/// and `stderr` in separate fields lets the classifier read the half that
/// carries the signal and ignore the half it wrote itself.
///
/// **House precedent:** `DeliveryError` + `downcast_ref::<DeliveryError>()`
/// (`cadence.rs`), and the mika#2179 rule — *error classes come from the
/// variant, never from a substring match on the rendered message*.
///
/// **`Display` is byte-identical to the legacy `anyhow!` format on purpose.**
/// Everything downstream still reads the rendered string: the
/// `manager_gh_auth_check_failed` log's `stderr_head` (which must keep naming
/// *which* call failed — see `spawn.rs`'s `raw` comment), the
/// `manager_cycle_error` `error` field, and the existing test corpus. The
/// tightening is on **what is classified**, never on what is logged.
/// `mika1975_the_gh_command_error_display_is_byte_identical` pins it.
///
/// `pub(crate)`: it travels boxed inside an `anyhow::Error`, so no public
/// signature exposes it. Widening it later is additive.
#[derive(Debug, thiserror::Error)]
#[error("gh {args} failed: {stderr}")]
pub(crate) struct GhCommandError {
    /// The `gh` argument vector, space-joined — **operator-chosen digits live
    /// here**, which is exactly why the classifier must not read it.
    pub(crate) args: String,
    /// What `gh` wrote to stderr — the only half that carries a status signal.
    pub(crate) stderr: String,
}

/// Production runner — spawns `gh` via `tokio::process::Command`.
pub struct ProcessGhRunner {
    token: Option<String>,
}

impl ProcessGhRunner {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }
}

#[async_trait::async_trait]
impl GhRunner for ProcessGhRunner {
    async fn run(&self, args: &[&str]) -> Result<String> {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(args);
        if let Some(t) = &self.token {
            cmd.env("GH_TOKEN", t);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // mika#1975 D5 — typed, not formatted. Going back to
            // `anyhow!("gh {} failed: {}", …)` here would break no test (the
            // classifier's string fallback keeps the suite green) while
            // silently putting the command line back into the classified text.
            // `mika1975_the_gh_runner_error_stays_typed` is what refuses it.
            return Err(anyhow::Error::new(GhCommandError {
                args: args.join(" "),
                stderr: stderr.into_owned(),
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl Reader {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }

    /// Compose milestone state from live `gh` calls. Delegates the pure
    /// composition logic to `compose_from_gh_outputs` — see that function
    /// for the boundary between subprocess-owning code and pure parsing.
    pub async fn read(&self, milestone_ref: &MilestoneRef) -> Result<MilestoneState> {
        self.read_with_runner(milestone_ref, &ProcessGhRunner::new(self.token.clone()))
            .await
    }

    /// Test-visible variant that accepts an injectable `GhRunner`.
    pub async fn read_with_runner<R: GhRunner>(
        &self,
        milestone_ref: &MilestoneRef,
        runner: &R,
    ) -> Result<MilestoneState> {
        // 1. Milestone metadata.
        let milestone_json = runner
            .run(&[
                "api",
                &format!(
                    "/repos/{}/milestones/{}",
                    milestone_ref.repo, milestone_ref.number
                ),
            ])
            .await?;

        // 2. Sub-issue enumeration (issues attached to this milestone).
        //
        //    `closedByPullRequestsReferences` est la référence de clôture que
        //    GitHub tient **côté issue** (mika#2924). Elle est demandée ici, sur
        //    une lecture déjà émise, parce que le milestone est rattaché au
        //    ticket et jamais à la PR : la liste des PR du milestone (appel 3)
        //    ne voit aucune PR de la boucle autonome, et 87 tickets fermés du
        //    milestone 33 rendaient `closed (no linked PR)` dont 73 avaient une
        //    PR de clôture. Le coût en appels `gh` par cycle est **inchangé** —
        //    mesuré le 2026-10-26 sur le dépôt réel : 1 appel avant, 1 après
        //    (1,639 s → 1,795 s).
        let issues_json = runner
            .run(&[
                "issue",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--milestone",
                &milestone_ref.number.to_string(),
                "--state",
                "all",
                "--json",
                "number,title,state,body,labels,updatedAt,closedByPullRequestsReferences",
                "--limit",
                "100",
            ])
            .await?;

        // 3. For each sub-issue, look up the closing PR + CI (bounded).
        //    We do this via a single `gh pr list` scoped by the milestone —
        //    cheaper than N `gh pr view` calls. Fall back to per-issue
        //    lookup for anything missing.
        let pr_json = runner
            .run(&[
                "pr",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--state",
                "all",
                "--search",
                &format!("milestone:{}", milestone_ref.number),
                "--json",
                "number,state,closingIssuesReferences,statusCheckRollup",
                "--limit",
                "100",
            ])
            .await?;

        // 4. Le bassin `ready`, à l'échelle du **dépôt** (mika#2699). Pas de
        //    `--milestone` ici, et c'est la décision : le webhook `ready`
        //    dispatche sur tout le dépôt, donc un bassin compté sur les seules
        //    issues du milestone serait sous-compté et la recommandation fausse
        //    exactement le jour où elle compte.
        let ready_json = runner
            .run(&[
                "issue",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--state",
                "open",
                "--label",
                READY_LABEL,
                "--json",
                "number,labels",
                "--limit",
                "100",
            ])
            .await?;

        // 5. Les candidats **hors milestone cible** (mika#2699, vol 2).
        //    Dépôt-large comme la lecture du bassin, et pour la même raison : le
        //    2e critère de tri du point 1 (« dans le milestone cible, avant le
        //    reste ») n'a de sens que s'il existe un « reste » à comparer.
        //    L'exclusion du milestone cible se fait **côté client**, dans
        //    `compose_outside_candidates` : la syntaxe de recherche GitHub
        //    n'exclut un milestone que par son *titre*, qu'un opérateur peut
        //    renommer entre deux cycles, alors que les numéros déjà lus en 2 sont
        //    exacts.
        let outside_json = runner
            .run(&[
                "issue",
                "list",
                "--repo",
                &milestone_ref.repo,
                "--state",
                "open",
                "--json",
                "number,body,labels,state",
                "--limit",
                &OUTSIDE_READ_LIMIT.to_string(),
            ])
            .await?;

        // `compose_from_gh_outputs` garde sa signature à quatre arguments : ni le
        // bassin ni les candidats du dehors ne sont une propriété du milestone, et
        // l'eval `manager_loop_resistance` l'appelle de l'extérieur. Les deux sont
        // donc posés ici, après composition.
        let mut state =
            compose_from_gh_outputs(milestone_ref, &milestone_json, &issues_json, &pr_json)?;
        state.ready_pool = compose_ready_pool(&ready_json)?;
        let (outside, truncated) = compose_outside_candidates(&outside_json, &state.sub_issues)?;
        state.outside_candidates = outside;
        state.outside_read_truncated = Some(truncated);

        // 6. La taille annoncée par le plan de chaque candidat pressenti
        //    (mika#2699 vol 3). N+1 lectures, donc bornées deux fois : par
        //    `numbers_needing_size`, qui n'en désigne que
        //    `PLAN_SIZE_READ_MAX`, et par le filtre d'admission lui-même, qui
        //    ne retient que les candidats déjà recevables sur la forme. Un
        //    ticket sans plan groomé est refusé avant d'avoir coûté un appel.
        //
        //    Qui est candidat se décide **au même endroit** qu'au jugement :
        //    `assessor::milestone_pool_candidates` sur l'état qu'on vient de
        //    composer, puis `pool::numbers_needing_size`. Rejouer le filtre ici
        //    rendrait les deux réponses identiques aujourd'hui et muettes le
        //    jour où l'une des deux change.
        let milestone_candidates = super::assessor::milestone_pool_candidates(&state);
        let wanted = super::pool::numbers_needing_size(
            &state.ready_pool,
            &milestone_candidates,
            &state.outside_candidates,
        );
        if !wanted.is_empty() {
            let bodies = compose_candidate_bodies(&issues_json, &outside_json)?;
            state.plan_sizes = read_plan_sizes(runner, &milestone_ref.repo, &wanted, &bodies).await;
        }
        Ok(state)
    }
}

/// Le corps de chaque issue lue à ce cycle, indexé par numéro — fonction
/// **pure** (mika#2699 vol 3).
///
/// Les deux sorties `gh` déjà en main portent `body` ; rien n'est relu à la
/// forge pour le retrouver. Le milestone est parcouru en second et **écrase**
/// le dehors sur un numéro présent dans les deux : c'est la même précédence
/// que celle de [`super::pool::assess_pool`], qui juge un tel ticket
/// `Milestone`.
pub fn compose_candidate_bodies(
    issues_json: &str,
    outside_json: &str,
) -> Result<HashMap<u64, String>> {
    let mut bodies = HashMap::new();
    for (json, what) in [(outside_json, "outside"), (issues_json, "milestone")] {
        let issues: Vec<Value> = serde_json::from_str(json)
            .map_err(|e| anyhow!("failed to parse {what} json for bodies: {e}"))?;
        for i in issues {
            let Some(number) = i.get("number").and_then(Value::as_u64) else {
                continue;
            };
            let body = i.get("body").and_then(Value::as_str).unwrap_or_default();
            bodies.insert(number, body.to_string());
        }
    }
    Ok(bodies)
}

/// Lit le plan de chaque candidat désigné, **au ref de sa branche**, et le
/// réduit au jeton de taille que l'admission consultera (mika#2699 vol 3).
///
/// **Pourquoi au ref de la branche et non sur `main`.** Le plan d'un ticket
/// groomé est commité sur la branche du ticket ; sur `main` il n'existe
/// généralement pas encore. Lire `main` répondrait « plan introuvable » pour
/// tout candidat correctement groomé — c'est-à-dire exactement pour ceux que la
/// borne doit juger.
///
/// **Un échec ne refuse pas et ne se tait pas.** Chaque branche d'échec porte sa
/// propre cause ([`super::pool::UnreadCause`]), parce que les remèdes sont
/// opposés : un callout manquant se corrige sur le ticket, un `404` au ref se
/// corrige en poussant la branche. Les confondre serait la classe mika#2205.
/// Aucune des deux n'est une estimation « au-delà de la borne », donc aucune ne
/// produit le refus — le rapport les nomme à la place.
///
/// **Rien n'est propagé en `Err`.** Un plan illisible est un fait sur *un*
/// candidat ; faire échouer le cycle entier pour lui priverait l'opérateur du
/// rapport sur tous les autres.
async fn read_plan_sizes<R: GhRunner>(
    runner: &R,
    repo: &str,
    wanted: &[u64],
    bodies: &HashMap<u64, String>,
) -> BTreeMap<u64, PlanSizeReading> {
    let mut out = BTreeMap::new();
    for number in wanted {
        out.insert(
            *number,
            read_one_plan_size(runner, repo, *number, bodies).await,
        );
    }
    out
}

async fn read_one_plan_size<R: GhRunner>(
    runner: &R,
    repo: &str,
    number: u64,
    bodies: &HashMap<u64, String>,
) -> PlanSizeReading {
    let Some(body) = bodies.get(&number) else {
        return PlanSizeReading::Unread(UnreadCause::NoCallout);
    };
    // Les deux lecteurs stricts du dépôt, réutilisés tels quels : `plan_callout`
    // rend un chemin **relatif à la racine du sous-dépôt** (son premier segment
    // est retiré), qui est précisément ce que `/contents/` attend. Les
    // tolérances larges de `plan_callout_present` ne conviendraient pas ici —
    // elles répondent « il y a un callout », pas « voici le chemin ».
    let Some(callout) = plan_callout(body, FenceHandling::Strip) else {
        return PlanSizeReading::Unread(UnreadCause::NoCallout);
    };
    let Some(branch) = extract_branch_name(body) else {
        return PlanSizeReading::Unread(UnreadCause::NoCallout);
    };
    let path = format!(
        "/repos/{repo}/contents/{}?ref={}",
        callout.normalized, branch
    );
    // `Accept: …raw` rend le fichier tel quel plutôt que l'enveloppe JSON
    // base64 : un décodage de moins, et le contrat de parse porté du bash
    // travaille sur le texte du plan.
    match runner
        .run(&["api", &path, "-H", "Accept: application/vnd.github.raw"])
        .await
    {
        Ok(plan) => match plan_size::plan_size_total_loc(&plan) {
            plan_size::PlanSizeTotal::Total(n) => PlanSizeReading::Measured(n),
            plan_size::PlanSizeTotal::SectionAbsent => PlanSizeReading::SectionAbsent,
            plan_size::PlanSizeTotal::Unparsable => PlanSizeReading::Unparsable,
        },
        Err(_) => PlanSizeReading::Unread(UnreadCause::FetchFailed),
    }
}

/// Compose le bassin `ready` depuis la sortie de `gh issue list --json
/// number,labels` — fonction **pure**, pendant de `compose_from_gh_outputs`
/// pour la quatrième lecture (mika#2699).
///
/// Pas de garde de troncature au vol 1, et c'est délibéré : à `--limit 100`
/// contre un bassin dont la cible est 2–4, une page pleine ne serait pas une
/// troncature mais une pathologie qui se voit déjà dans la profondeur rapportée
/// (« 100 ready » ne se lit pas comme un état sain). La garde devient utile au
/// vol 3, où le nombre lu décide d'une écriture.
pub fn compose_ready_pool(ready_json: &str) -> Result<Vec<ReadyTicket>> {
    let issues: Vec<Value> = serde_json::from_str(ready_json)
        .map_err(|e| anyhow!("failed to parse ready-pool json: {e}"))?;
    Ok(issues
        .iter()
        .filter_map(|i| {
            let number = i.get("number")?.as_u64()?;
            Some(ReadyTicket {
                number,
                labels: extract_labels(i),
            })
        })
        .collect())
}

/// Compose les candidats **hors milestone cible** depuis la sortie de
/// `gh issue list --json number,body,labels,state` — fonction **pure**,
/// pendant de [`compose_ready_pool`] pour la cinquième lecture (mika#2699,
/// vol 2).
///
/// Rend `(candidats, tronqué)`. Trois décisions, chacune pour une raison que la
/// forme seule ne dit pas :
///
/// 1. **Une page pleine vide l'ensemble.** À `--limit OUTSIDE_READ_LIMIT`
///    contre un dépôt de plusieurs centaines d'ouvertes, la page pleine est le
///    cas nominal. Rendre les candidats de cette page affirmerait un ordre non
///    mesuré : le p0 qui devait passer devant est peut-être en page 2. Le motif
///    est celui de `scripts/pr-origin-report.sh` — *refuser plutôt que mal
///    répondre* — et il est ici le seul qui ne mente pas, puisque le tri du
///    point 1 porte précisément sur un ordre global.
/// 2. **Les `sub_issues` sont retirées ici.** Elles arrivent déjà par
///    `MilestoneState::sub_issues`, et les porter deux fois alourdirait chaque
///    blob du sink hors-ligne d'un doublon par ticket du milestone.
///    [`super::pool::assess_pool`] déduplique quand même, pour sa propre
///    correction : une fonction pure ne doit pas dépendre de la prudence de son
///    appelant.
/// 3. **La troncature est mesurée sur la page lue, pas sur le reste.** Le
///    booléen rendu dit « la lecture a ramené une page pleine », avant
///    l'exclusion des `sub_issues` — sinon un milestone de 100 tickets rendrait
///    « non tronqué » sur une page pleine.
pub fn compose_outside_candidates(
    outside_json: &str,
    subs: &[SubIssue],
) -> Result<(Vec<PoolCandidate>, bool)> {
    let issues: Vec<Value> = serde_json::from_str(outside_json)
        .map_err(|e| anyhow!("failed to parse outside-candidates json: {e}"))?;
    let truncated = issues.len() >= OUTSIDE_READ_LIMIT;
    if truncated {
        return Ok((Vec::new(), true));
    }
    let in_milestone: std::collections::HashSet<u64> = subs.iter().map(|s| s.number).collect();
    let candidates = issues
        .iter()
        .filter_map(|i| {
            let number = i.get("number")?.as_u64()?;
            if in_milestone.contains(&number) {
                return None;
            }
            let body = i.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let labels = extract_labels(i);
            let state = match i.get("state").and_then(|v| v.as_str()) {
                Some(s) if s.eq_ignore_ascii_case("closed") => IssueState::Closed,
                _ => IssueState::Open,
            };
            Some(PoolCandidate {
                number,
                state,
                priority_rank: priority_from_labels(&labels),
                plan_present: plan_callout_present(body),
                labels,
            })
        })
        .collect();
    Ok((candidates, false))
}

/// Les noms de labels d'une issue telle que `gh issue list --json labels` la
/// rend. Un seul site d'épellation du chemin `labels[].name`, partagé par les
/// trois lectures d'issues (sub-issues du milestone, bassin `ready`, candidats
/// hors milestone).
fn extract_labels(issue: &Value) -> Vec<String> {
    issue
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Priority ranks from label — pure function, easy to test.
/// Lower is higher priority. `None` = unranked.
///
/// When multiple priority labels are present (rare — usually a labelling
/// mistake), the highest-priority one wins (lowest numeric rank).
pub fn priority_from_labels(labels: &[String]) -> Option<u8> {
    let mut best: Option<u8> = None;
    for l in labels {
        let rank = match l.as_str() {
            "p0-critical" => Some(0),
            "p1-important" => Some(1),
            "agent-core" => Some(2),
            "p2-normal" => Some(3),
            "p3-nice-to-have" => Some(4),
            _ => None,
        };
        if let Some(r) = rank {
            best = Some(match best {
                Some(b) => b.min(r),
                None => r,
            });
        }
    }
    best
}

/// Parse whether the issue body carries a `Plan: docs/plans/` callout.
/// Matches the loose form documented in the milestone-cascade contract.
///
/// # Tolérance CONSERVÉE, décidée par mika#2608 (phase 2 de mika#2194)
///
/// Ce lecteur est le **quatrième** de la table des tolérances de
/// [`crate::plan_callout`], et il a été trouvé en implémentant la phase 1, qui
/// l'a nommé sans le trancher. La phase 2 tranche : **il n'est pas migré.**
/// Quatre raisons, dans l'ordre de leur force.
///
/// ## 1. Un appel au lecteur strict y serait PROUVABLEMENT inerte
///
/// C'est l'argument décisif, et il est structurel plutôt que prudentiel. La
/// première branche ci-dessous est un **sur-ensemble strict** du lecteur unique :
/// toute ligne que `PLAN_CALLOUT_RE` accepte commence par `> - **Plan:** `, donc,
/// `trim_start` appliqué, commence par `> ` **et** contient `**Plan:**`. **Il
/// n'existe aucun corps que le lecteur strict accepte et que cette branche
/// refuse.**
///
/// Conséquence : la réparation tentante — « appeler `plan_callout` en première
/// branche et garder les branches lâches en repli » — ajouterait un appel qui ne
/// peut **jamais** décider de rien. Ce serait une unification apparente et
/// inerte, c'est-à-dire la classe mika#2205 appliquée à un chemin de code : verte,
/// plausible, et mesurant zéro. La propriété de sur-ensemble est **épinglée par un
/// test** sur le corpus doré, pas laissée à un raisonnement dans un plan.
///
/// ## 2. Ce n'est pas la même question
///
/// Le lecteur unique répond « quel chemin, sous quelle forme ? » ; celui-ci
/// répond « ce corps porte-t-il un callout ? », en booléen, sans rendre de
/// chemin. Le migrer serait répondre à une autre question.
///
/// ## 3. La forme lâche est PRESCRITE
///
/// Son contrat est celui de la cascade milestone (la ligne de doc ci-dessus le
/// dit depuis l'origine), et le dépôt porte un troisième prédicat de la même
/// famille dans un prompt (`self-dev/system_prompt.md`, sous-chaîne
/// `Plan: docs/plans/`).
///
/// ## 4. Le coût des deux erreurs penche dans le sens sûr
///
/// Le manager est **LECTURE seule**, zéro dispatch. `plan_present` alimente un
/// **rapport à un humain** et `SubIssue::is_in_governed_progress`. Un
/// resserrement produirait des **faux négatifs** sur un rapport dont l'objet est
/// de dire à un humain quels sous-tickets restent à groomer — il lui dirait de
/// groomer un ticket groomé.
///
/// # La faiblesse réelle, nommée et NON corrigée
///
/// La seconde branche (`> ` + `contains("docs/plans/")`) lit une citation en
/// prose comme un callout : un blockquote disant « le plan vit dans
/// `docs/plans/` » rend `true`. C'est un faux positif plausible, et **aucune
/// mesure ne l'établit**. Suivi nommé, avec sa précondition : un rapport dont
/// `plan_present` est mesurément faux. Armer un resserrement sur une population
/// non mesurée est ce que mika#2520 refuse.
pub fn plan_callout_present(body: &str) -> bool {
    // Canonical callout shape: `> - **Plan:** docs/plans/...`.
    // Loose form: any occurrence of `docs/plans/` in a callout blockquote.
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("> ") && t.contains("**Plan:**") {
            return true;
        }
        if t.starts_with("> ") && t.contains("docs/plans/") {
            return true;
        }
    }
    false
}

/// Parse whether the issue body carries a `> - **Branch:**` callout.
pub fn branch_callout_present(body: &str) -> bool {
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("> ") && t.contains("**Branch:**") {
            return true;
        }
    }
    false
}

/// Extract `Blocked by #N` blocker references from an issue body.
/// Pure text scan — the GitHub GraphQL `blockedBy` edge is the
/// authoritative source but requires a second call per issue; body-scan
/// is the wrapper-only shortcut for Phase 1.
pub fn extract_blockers(body: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.to_lowercase();
        // Match `blocked by #N` and `blocked-by: #N` forms.
        if !(t.contains("blocked by") || t.contains("blocked-by")) {
            continue;
        }
        // Extract numeric refs after `#`.
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '#' {
                let mut digits = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        digits.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if let Ok(n) = digits.parse::<u64>()
                    && !out.contains(&n)
                {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// Classify the CI state from a `gh pr view --json statusCheckRollup` array.
/// GitHub returns `SUCCESS`/`FAILURE`/`PENDING`/... — we collapse to the
/// four-variant `CiState` enum for the Assessor.
pub fn classify_ci_state(rollup: &Value) -> CiState {
    let Some(arr) = rollup.as_array() else {
        return CiState::None;
    };
    if arr.is_empty() {
        return CiState::None;
    }
    let mut any_pending = false;
    let mut any_failed = false;
    for check in arr {
        let conclusion = check
            .get("conclusion")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let status = check.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status.eq_ignore_ascii_case("IN_PROGRESS")
            || status.eq_ignore_ascii_case("QUEUED")
            || status.eq_ignore_ascii_case("PENDING")
        {
            any_pending = true;
        }
        match conclusion {
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STALE" => {
                any_failed = true;
            }
            _ => {}
        }
    }
    if any_failed {
        CiState::Failed
    } else if any_pending {
        CiState::Pending
    } else {
        CiState::Success
    }
}

/// Pure composition: given the three JSON blobs, produce a `MilestoneState`.
///
/// Split out from `read()` so tests can exercise the parsing chain with
/// fixture data (no subprocess boundary in the fast path).
pub fn compose_from_gh_outputs(
    milestone_ref: &MilestoneRef,
    milestone_json: &str,
    issues_json: &str,
    pr_json: &str,
) -> Result<MilestoneState> {
    let m: Value = serde_json::from_str(milestone_json)
        .map_err(|e| anyhow!("failed to parse milestone json: {e}"))?;
    let issues: Vec<Value> = serde_json::from_str(issues_json)
        .map_err(|e| anyhow!("failed to parse issues json: {e}"))?;
    let prs: Vec<Value> =
        serde_json::from_str(pr_json).map_err(|e| anyhow!("failed to parse pr json: {e}"))?;

    // Build a lookup: sub-issue number → (pr_number, pr_state, ci_state).
    // GitHub's `closingIssuesReferences` is populated when the PR body
    // says `Closes #N` or the PR is manually linked to the issue.
    let mut pr_by_issue: HashMap<u64, (u64, String, CiState)> = HashMap::new();
    for pr in &prs {
        let pr_number = match pr.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => continue,
        };
        let pr_state = pr
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let ci = pr
            .get("statusCheckRollup")
            .map(classify_ci_state)
            .unwrap_or(CiState::None);
        if let Some(refs) = pr.get("closingIssuesReferences").and_then(|v| v.as_array()) {
            for r in refs {
                if let Some(issue_num) = r.get("number").and_then(|v| v.as_u64()) {
                    // First-writer wins to keep deterministic behaviour.
                    pr_by_issue
                        .entry(issue_num)
                        .or_insert_with(|| (pr_number, pr_state.clone(), ci.clone()));
                }
            }
        }
    }

    let mut sub_issues: Vec<SubIssue> = Vec::with_capacity(issues.len());
    let mut recent_activity: Vec<RecentActivity> = Vec::new();
    let mut last_activity_at: Option<String> = None;

    for issue in &issues {
        let number = match issue.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => continue,
        };
        let title = issue
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let state_str = issue
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("OPEN")
            .to_uppercase();
        let state = if state_str == "CLOSED" {
            IssueState::Closed
        } else {
            IssueState::Open
        };
        let body = issue
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let labels: Vec<String> = extract_labels(issue);
        let updated_at = issue
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !updated_at.is_empty() {
            match &last_activity_at {
                Some(existing) if existing.as_str() >= updated_at.as_str() => {}
                _ => last_activity_at = Some(updated_at.clone()),
            }
        }

        let plan_present = plan_callout_present(&body);
        let branch_present = branch_callout_present(&body);
        let blockers = extract_blockers(&body);
        let priority_rank = priority_from_labels(&labels);
        // La PR de clôture, et **d'où** on la tient (mika#2924).
        //
        // L'index des PR du milestone passe en premier et reste inchangé : il
        // est le seul porteur de `pr_state` et de `ci_state`, dont dépendent
        // `is_in_flight()`, le palier 1 de `pick_next_action`, `is_hard_blocked()`
        // et surtout `detect_silent_progress`, qui exige
        // `state == Open && pr_state == Some("merged")`.
        // La référence côté issue ne vient **qu'en comblement** : c'est une
        // union, pas un remplacement. Elle répond pour les PR que la boucle
        // produit — le pilote rattache le milestone au ticket, jamais à la PR,
        // donc l'index ci-dessus est vide pour elles — mais elle ne dit rien de
        // l'état de la PR ni de sa CI, et les deviner serait affirmer une mesure
        // qui n'a pas eu lieu (mika#2331).
        let mut pr_number: Option<u64> = None;
        let mut pr_state: Option<String> = None;
        let mut pr_url: Option<String> = None;
        let mut pr_link_source: Option<PrLinkSource> = None;
        let mut ci_state = CiState::None;
        if let Some((n, s, c)) = pr_by_issue.get(&number).cloned() {
            pr_number = Some(n);
            pr_state = Some(s);
            ci_state = c;
            // Posé même ici, où la provenance pourrait se déduire de la seule
            // présence d'un numéro : sans ça, `None` voudrait dire à la fois
            // « liste du milestone » et « blob d'un binaire antérieur », et la
            // sonde AC3 ne pourrait plus séparer les deux (mika#2340).
            pr_link_source = Some(PrLinkSource::MilestonePrList);
        } else if let Some(r) = issue
            .get("closedByPullRequestsReferences")
            .and_then(|v| v.as_array())
            // Première entrée retenue, comme l'index ci-dessus : un ticket peut
            // être fermé par plusieurs PR (un vol en deux temps), et un choix
            // dépendant de l'ordre de la page rendrait deux rapports différents
            // sur le même état.
            .and_then(|a| a.first())
        {
            // Les **deux** champs sont exigés, et le numéro seul ne suffit pas :
            // la référence peut porter une PR d'un autre dépôt, et recomposer
            // `{repo}/pull/{n}` rendrait alors un lien vers une PR qui n'existe
            // pas — plus difficile à démentir qu'une absence de lien. Sans `url`
            // la ligne garde donc son `closed (no linked PR)`, qui est vrai de ce
            // que l'on sait.
            if let (Some(n), Some(url)) = (
                r.get("number").and_then(|v| v.as_u64()),
                r.get("url").and_then(|v| v.as_str()),
            ) {
                pr_number = Some(n);
                pr_url = Some(url.to_string());
                pr_link_source = Some(PrLinkSource::IssueClosingRef);
                // `pr_state` et `ci_state` restent volontairement vides : ce
                // chemin n'a pas vu la PR. `detect_silent_progress` exige
                // `pr_state == Some("merged")` et doit continuer de ne le tenir
                // que de la liste du milestone.
            }
        }

        sub_issues.push(SubIssue {
            number,
            title: title.clone(),
            state: state.clone(),
            priority_rank,
            plan_present,
            branch_present,
            pr_number,
            pr_state,
            pr_url,
            pr_link_source,
            ci_state,
            blockers,
            updated_at: updated_at.clone(),
            labels,
        });

        // Attribute recent activity by the sub-issue's updated_at (crude
        // but sufficient for Phase 1 — a proper timeline would require an
        // extra `gh api /issues/N/timeline` call per issue).
        if state == IssueState::Closed {
            recent_activity.push(RecentActivity {
                at: updated_at.clone(),
                kind: "sub_issue_closed".to_string(),
                subject: format!("#{number} {title}"),
            });
        }
    }

    // Add PR merge activity from the PR list.
    for pr in &prs {
        if pr.get("state").and_then(|v| v.as_str()).unwrap_or("") == "MERGED" {
            let pr_number = pr.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
            recent_activity.push(RecentActivity {
                at: "".to_string(),
                kind: "pr_merged".to_string(),
                subject: format!("#{pr_number}"),
            });
        }
    }
    // Sort desc, keep 10 most recent.
    recent_activity.sort_by(|a, b| b.at.cmp(&a.at));
    recent_activity.truncate(10); // safe-byte-slice: Vec<RecentActivity> — element count, no char boundary

    let progress = compute_progress(&sub_issues);

    let title = m
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let description = m
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = m
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let due_on = m.get("due_on").and_then(|v| v.as_str()).map(str::to_string);
    let milestone_state_str = m.get("state").and_then(|v| v.as_str()).unwrap_or("open");
    let m_state = if milestone_state_str.eq_ignore_ascii_case("closed") {
        IssueState::Closed
    } else {
        IssueState::Open
    };

    Ok(MilestoneState {
        milestone_ref: milestone_ref.clone(),
        title,
        description,
        state: m_state,
        created_at,
        due_on,
        last_activity_at,
        sub_issues,
        progress,
        recent_activity,
        executor_healthy: None,
        // Vide ici par construction : le bassin est une lecture dépôt-large que
        // cette fonction ne reçoit pas. `Reader::read_with_runner` le pose après
        // composition — voir `compose_ready_pool`.
        ready_pool: Vec::new(),
        // Idem pour les candidats hors milestone (mika#2699, vol 2) : dépôt-large
        // eux aussi, posés après composition par `compose_outside_candidates`.
        // `None` et non `Some(false)` : à ce point la question de la troncature
        // n'a pas été posée (mika#2331).
        outside_candidates: Vec::new(),
        outside_read_truncated: None,
        // Idem pour les tailles de plan (mika#2699, vol 3) : elles dépendent du
        // bassin et du dehors, qu'on n'a pas ici. Vide, donc « non mesuré » pour
        // chaque candidat — jamais « de taille nulle ».
        plan_sizes: BTreeMap::new(),
    })
}

/// Compute `ProgressCounts` from sub-issues. Pure function.
pub fn compute_progress(subs: &[SubIssue]) -> ProgressCounts {
    let mut p = ProgressCounts {
        total: subs.len() as u32,
        ..Default::default()
    };
    for s in subs {
        match s.state {
            IssueState::Closed => p.completed += 1,
            IssueState::Open => {
                if s.is_hard_blocked() {
                    p.blocked += 1;
                } else if s.is_in_flight() {
                    p.in_flight += 1;
                } else {
                    p.unstarted += 1;
                }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_from_labels_p0_wins() {
        let labels = vec![
            "p2-normal".to_string(),
            "p0-critical".to_string(),
            "enhancement".to_string(),
        ];
        assert_eq!(priority_from_labels(&labels), Some(0));
    }

    #[test]
    fn priority_from_labels_no_priority() {
        let labels = vec!["enhancement".to_string(), "documentation".to_string()];
        assert_eq!(priority_from_labels(&labels), None);
    }

    #[test]
    fn plan_callout_canonical() {
        let body = "Header\n> - **Plan:** docs/plans/foo.md\n> - **Branch:** feat/x\n";
        assert!(plan_callout_present(body));
    }

    #[test]
    fn plan_callout_loose() {
        let body = "> Plan lives at docs/plans/x.md\n";
        assert!(plan_callout_present(body));
    }

    #[test]
    fn plan_callout_absent() {
        let body = "No plan here.\nSome plain text mentioning docs/plans/x.md outside a callout.\n";
        assert!(!plan_callout_present(body));
    }

    /// mika#2608 V11 — la propriété de sur-ensemble de M4, **assertée** plutôt
    /// que raisonnée dans un plan.
    ///
    /// Elle est ce qui rend la décision de ne PAS migrer ce lecteur structurelle :
    /// si la première branche est un sur-ensemble strict du lecteur unique, alors
    /// un appel strict-first y serait inerte, et une unification qui ne peut rien
    /// décider est une unification apparente (classe mika#2205 appliquée à un
    /// chemin de code).
    ///
    /// Mesurée sur le **corpus doré commun** plutôt que sur des corps inventés
    /// ici : c'est le corpus que les deux lecteurs du callout partagent depuis
    /// mika#2194, donc le seul endroit où « les mêmes entrées » veut dire quelque
    /// chose.
    #[test]
    fn mika2608_la_branche_lache_est_un_surensemble_du_lecteur_strict() {
        use crate::plan_callout::{FenceHandling, plan_callout};

        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/plan_callout_bodies");
        let mut accepted_by_strict = 0usize;

        let entries = std::fs::read_dir(&corpus)
            .unwrap_or_else(|e| panic!("corpus doré illisible ({}): {e}", corpus.display()));
        for entry in entries {
            let path = entry.expect("entrée de corpus illisible").path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue; // README.md est un .md — voir le filtre ci-dessous.
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("README.md") {
                continue;
            }
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("corps illisible ({}): {e}", path.display()));

            // `Keep` : la politique de `dispatch-lib`, donc la moitié dont la
            // parité était inconnue avant mika#2194. Sous `Strip` la population
            // acceptée est un sous-ensemble, donc l'assertion serait plus faible.
            if plan_callout(&body, FenceHandling::Keep).is_some() {
                accepted_by_strict += 1;
                assert!(
                    plan_callout_present(&body),
                    "{}: le lecteur STRICT accepte ce corps et la branche lâche le refuse — \
                     la propriété de sur-ensemble de mika#2608 M4 est fausse, donc un appel \
                     strict-first ici ne serait plus inerte et la décision de ne pas migrer \
                     ce lecteur doit être reprise",
                    path.display()
                );
            }
        }

        // Anti-vacuité : un corpus vide, ou un filtre trop large, rendrait cette
        // assertion verte sans avoir rien comparé.
        assert!(
            accepted_by_strict >= 6,
            "seulement {accepted_by_strict} corps acceptés par le lecteur strict — \
             le corpus doré en porte au moins six (les six corps mesurés de mika#2120)"
        );
    }

    /// mika#2608 V10 — le **pin de décision**. Motif
    /// `mika2120_divergence_is_still_open_and_this_test_pins_it`.
    ///
    /// Il pose la tolérance conservée comme une DÉCISION, de sorte qu'un futur
    /// éditeur qui « harmonise » ce lecteur vers le strict fasse rougir un test au
    /// lieu de changer un rapport opérateur en silence. Les deux formes ci-dessous
    /// sont exactement celles que le lecteur unique refuse et que celui-ci doit
    /// continuer d'accepter.
    #[test]
    fn mika2608_la_tolerance_lache_est_une_decision_epinglee() {
        use crate::plan_callout::{FenceHandling, plan_callout};

        // (a) Branche 1 : le callout SANS le préfixe `> - ` que le strict exige.
        let no_dash = "> **Plan:** docs/plans/x.md\n";
        assert!(
            plan_callout(no_dash, FenceHandling::Keep).is_none(),
            "prémisse : le lecteur strict refuse un callout sans `> - `"
        );
        assert!(
            plan_callout_present(no_dash),
            "mika#2608 D6 : la tolérance de ce lecteur est CONSERVÉE par décision. \
             Si vous l'avez resserrée pour « harmoniser », lisez le doc-comment de \
             plan_callout_present : le manager est LECTURE seule, son rapport dit à un \
             humain quels sous-tickets restent à groomer, et un faux négatif lui dit de \
             groomer un ticket groomé."
        );

        // (b) Branche 2 : une citation en PROSE dans un blockquote. C'est la
        //     faiblesse réelle, nommée et non corrigée (suivi avec précondition).
        let prose = "> le plan vit dans docs/plans/ quelque part\n";
        assert!(
            plan_callout(prose, FenceHandling::Keep).is_none(),
            "prémisse : le lecteur strict refuse une citation en prose"
        );
        assert!(
            plan_callout_present(prose),
            "mika#2608 D6 : la branche 2 est le faux positif NOMMÉ de ce lecteur. \
             Le fermer demande une mesure (un rapport dont `plan_present` est faux), \
             pas un resserrement — armer un détecteur sur une population non mesurée \
             est ce que mika#2520 refuse."
        );
    }

    #[test]
    fn branch_callout_present_form() {
        let body = "> - **Branch:** `feat/x`";
        assert!(branch_callout_present(body));
    }

    #[test]
    fn branch_callout_absent_form() {
        assert!(!branch_callout_present("no callout here"));
    }

    #[test]
    fn extract_blockers_simple() {
        let body = "Blocked by #42 and #99";
        assert_eq!(extract_blockers(body), vec![42, 99]);
    }

    #[test]
    fn extract_blockers_deduped() {
        let body = "Blocked by #42\nblocked-by: #42";
        assert_eq!(extract_blockers(body), vec![42]);
    }

    #[test]
    fn extract_blockers_none_when_no_marker() {
        // Bare `#42` without `blocked by` context should NOT match.
        assert!(extract_blockers("See issue #42 for context").is_empty());
    }

    #[test]
    fn classify_ci_all_success() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "SUCCESS", "status": "COMPLETED"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Success);
    }

    #[test]
    fn classify_ci_any_failure() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "FAILURE", "status": "COMPLETED"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Failed);
    }

    #[test]
    fn classify_ci_pending_no_failure() {
        let rollup = serde_json::json!([
            {"conclusion": "SUCCESS", "status": "COMPLETED"},
            {"conclusion": "", "status": "IN_PROGRESS"}
        ]);
        assert_eq!(classify_ci_state(&rollup), CiState::Pending);
    }

    #[test]
    fn classify_ci_empty_none() {
        let rollup = serde_json::json!([]);
        assert_eq!(classify_ci_state(&rollup), CiState::None);
    }

    #[test]
    fn classify_ci_non_array_none() {
        let rollup = serde_json::json!(null);
        assert_eq!(classify_ci_state(&rollup), CiState::None);
    }

    #[test]
    fn compose_end_to_end() {
        let milestone_json = r#"{"title": "LC", "description": "Reconciliation", "state": "open", "created_at": "2026-07-17T00:00:00Z", "due_on": null}"#;
        let issues_json = r#"[
            {"number": 1801, "title": "LC.1 theme.css", "state": "CLOSED", "body": "> - **Plan:** docs/plans/lc1.md", "labels": [{"name":"p1-important"}], "updatedAt": "2026-08-15T00:00:00Z"},
            {"number": 1802, "title": "LC.2 tokens", "state": "OPEN", "body": "> - **Plan:** docs/plans/lc2.md\n> - **Branch:** `feat/lc2`", "labels": [{"name":"p2-normal"}], "updatedAt": "2026-08-20T00:00:00Z"},
            {"number": 1803, "title": "LC.3 blocked", "state": "OPEN", "body": "Blocked by #1802", "labels": [], "updatedAt": "2026-08-10T00:00:00Z"}
        ]"#;
        let pr_json = r#"[
            {"number": 2001, "state": "MERGED", "closingIssuesReferences": [{"number": 1801}], "statusCheckRollup": []},
            {"number": 2002, "state": "OPEN", "closingIssuesReferences": [{"number": 1802}], "statusCheckRollup": [{"conclusion": "", "status": "IN_PROGRESS"}]}
        ]"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 1799,
        };
        let state = compose_from_gh_outputs(&r, milestone_json, issues_json, pr_json).unwrap();
        assert_eq!(state.title, "LC");
        assert_eq!(state.sub_issues.len(), 3);
        // 1801 closed → completed.
        // 1802 open + PR + plan → in flight.
        // 1803 open + blocker → blocked.
        assert_eq!(state.progress.completed, 1);
        assert_eq!(state.progress.in_flight, 1);
        assert_eq!(state.progress.blocked, 1);
        assert_eq!(state.progress.unstarted, 0);
        assert_eq!(state.progress.total, 3);

        // Verify PR linkage on 1802.
        let lc2 = state.sub_issues.iter().find(|s| s.number == 1802).unwrap();
        assert_eq!(lc2.pr_number, Some(2002));
        assert_eq!(lc2.ci_state, CiState::Pending);
        assert!(lc2.plan_present);
        assert!(lc2.branch_present);

        // Verify blocker on 1803.
        let lc3 = state.sub_issues.iter().find(|s| s.number == 1803).unwrap();
        assert_eq!(lc3.blockers, vec![1802]);
        assert!(lc3.is_hard_blocked());

        // Verify priority_rank derives from labels — protects the emit
        // wiring at compose time (injection-verification anchor).
        let lc1 = state.sub_issues.iter().find(|s| s.number == 1801).unwrap();
        assert_eq!(lc1.priority_rank, Some(1)); // p1-important → 1
        assert_eq!(lc2.priority_rank, Some(3)); // p2-normal → 3
        assert_eq!(lc3.priority_rank, None); // no priority label
    }

    #[test]
    fn compose_end_to_end_mix_closed_open_blocked_locks_avancement() {
        // mika#1933 AC2 lock-in: verify Reader denominator matches
        // GitHub's `open_issues + closed_issues` (avancement, not
        // reste-à-faire). Mix: 1 closed + 3 open (of which 1 in-flight,
        // 1 unstarted, 1 blocked-by-open-sibling) → 4 total sub-issues.
        // Note: blocked sub-issues are still `state=OPEN` — the "blocked"
        // classification is layered on top of open state via the blockers
        // list; it is not a distinct GitHub issue state.
        let milestone_json = r#"{"title": "RT-005", "description": "planning tokens", "state": "open", "created_at": "2026-08-17T00:00:00Z", "due_on": null}"#;
        let issues_json = r#"[
            {"number": 1889, "title": "RT-005 brick 4", "state": "CLOSED", "body": "> - **Plan:** docs/plans/rt5-b4.md", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-20T00:00:00Z"},
            {"number": 1888, "title": "RT-005 brick 5", "state": "OPEN", "body": "> - **Plan:** docs/plans/rt5-b5.md", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-19T00:00:00Z"},
            {"number": 1887, "title": "RT-005 brick 1", "state": "OPEN", "body": "no plan yet", "labels": [{"name":"p3-nice-to-have"}], "updatedAt": "2026-08-18T00:00:00Z"},
            {"number": 1890, "title": "RT-005 brick 6 (blocked)", "state": "OPEN", "body": "Blocked by #1888", "labels": [], "updatedAt": "2026-08-17T00:00:00Z"}
        ]"#;
        let pr_json = r#"[
            {"number": 1929, "state": "MERGED", "closingIssuesReferences": [{"number": 1889}], "statusCheckRollup": []}
        ]"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        let state = compose_from_gh_outputs(&r, milestone_json, issues_json, pr_json).unwrap();
        // Denominator is the full sub-issue count (closed + open), not open-only.
        assert_eq!(state.sub_issues.len(), 4);
        assert_eq!(state.progress.total, 4);
        assert_eq!(state.progress.completed, 1); // #1889 closed
        assert_eq!(state.progress.blocked, 1); // #1890 blocker
        assert_eq!(state.progress.in_flight, 1); // #1888 has plan
        assert_eq!(state.progress.unstarted, 1); // #1887 no plan
        // The closed sub-issue is enumerated with the correct state and PR linkage
        // (the composer's `closingIssuesReferences` reverse index must populate
        // `pr_number` for closed sub-issues too, not just open ones).
        let closed = state
            .sub_issues
            .iter()
            .find(|s| s.number == 1889)
            .expect("closed sub-issue present in enumeration");
        assert_eq!(closed.state, IssueState::Closed);
        assert_eq!(closed.pr_number, Some(1929));
    }

    /// La sortie `gh` réellement capturée sur le milestone 33 le 2026-10-26,
    /// filtrée sur les deux contrôles de l'AC1 de mika#2924. Les deux vivent
    /// dans le **même** document, parce que c'est ainsi qu'ils arrivent en
    /// production — le composeur ne lit pas deux fichiers. Provenance, commande
    /// de capture et fidélités : `tests/fixtures/mika2924/README.md`.
    const FIXTURE_MILESTONE_33: &str =
        include_str!("../../tests/fixtures/mika2924/issues-milestone-33.json");

    /// mika#2924 R2/R3 — la situation de **production**, celle qui a produit les
    /// 87 lignes `closed (no linked PR)` : la liste des PR du milestone est
    /// vide, parce que le pilote rattache le milestone au ticket et jamais à la
    /// PR. La référence de clôture côté issue comble alors le trou.
    ///
    /// Les deux contrôles de l'AC1 sont dans ce test, sur le même document :
    /// #2886 fermé par #2892 (contrôle positif) et #1680 fermé à la main,
    /// `closedByPullRequestsReferences` à `[]` (contrôle négatif). Sans le
    /// second, un lecteur qui peuplerait `pr_number` pour tout ticket fermé
    /// passerait le premier.
    #[test]
    fn mika2924_la_reference_cote_issue_comble_lindex_vide_du_milestone() {
        let milestone_json = r#"{"title": "M33", "description": "sink", "state": "open", "created_at": "2026-10-01T00:00:00Z", "due_on": null}"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 33,
        };
        // `[]` : aucune PR de la boucle n'est rattachée au milestone.
        let state =
            compose_from_gh_outputs(&r, milestone_json, FIXTURE_MILESTONE_33, "[]").unwrap();

        let s2886 = state
            .sub_issues
            .iter()
            .find(|s| s.number == 2886)
            .expect("#2886 présent dans l'énumération");
        assert_eq!(s2886.state, IssueState::Closed);
        assert_eq!(s2886.pr_number, Some(2892));
        // L'URL est celle que la référence porte, jamais une recomposition :
        // l'entrée porte un objet `repository` imbriqué, donc elle peut nommer
        // un autre dépôt (R3).
        assert_eq!(
            s2886.pr_url.as_deref(),
            Some("https://github.com/senara-solutions/mika/pull/2892")
        );
        assert_eq!(s2886.pr_link_source, Some(PrLinkSource::IssueClosingRef));
        // Ce chemin ne porte ni l'état de la PR ni celui de la CI — la
        // référence côté issue ne les transporte pas, et les inventer serait
        // rendre deux populations en octets identiques (mika#2205).
        assert_eq!(s2886.pr_state, None);
        assert_eq!(s2886.ci_state, CiState::None);

        let s1680 = state
            .sub_issues
            .iter()
            .find(|s| s.number == 1680)
            .expect("#1680 présent dans l'énumération");
        assert_eq!(s1680.state, IssueState::Closed);
        assert_eq!(s1680.pr_number, None);
        assert_eq!(s1680.pr_url, None);
        assert_eq!(
            s1680.pr_link_source, None,
            "aucune provenance : le ticket a été fermé à la main, les deux \
             chemins ont été essayés et aucun n'a répondu"
        );
    }

    /// mika#2924 R2 — l'**ordre** de l'union, qui n'est pas symétrique :
    /// l'index inverse des PR du milestone passe en premier parce qu'il est le
    /// seul porteur de `pr_state` et `ci_state`. Si la référence côté issue
    /// gagnait, un milestone dont les PR *sont* rattachées perdrait l'état de
    /// sa CI sans qu'aucune autre assertion ne rougisse.
    #[test]
    fn mika2924_lindex_du_milestone_garde_la_priorite_sur_la_reference_cote_issue() {
        let milestone_json = r#"{"title": "M33", "description": "sink", "state": "open", "created_at": "2026-10-01T00:00:00Z", "due_on": null}"#;
        // Le même #2886, cette fois atteignable par les deux chemins.
        let pr_json = r#"[
            {"number": 2892, "state": "MERGED", "closingIssuesReferences": [{"number": 2886}], "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}]}
        ]"#;
        let r = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 33,
        };
        let state =
            compose_from_gh_outputs(&r, milestone_json, FIXTURE_MILESTONE_33, pr_json).unwrap();

        let s2886 = state
            .sub_issues
            .iter()
            .find(|s| s.number == 2886)
            .expect("#2886 présent dans l'énumération");
        assert_eq!(s2886.pr_number, Some(2892));
        assert_eq!(s2886.pr_state.as_deref(), Some("merged"));
        assert_eq!(s2886.ci_state, CiState::Success);
        assert_eq!(
            s2886.pr_link_source,
            Some(PrLinkSource::MilestonePrList),
            "l'index du milestone a répondu : c'est lui qui nomme la provenance"
        );
        assert_eq!(
            s2886.pr_url, None,
            "ce chemin ne porte pas d'URL — le rapport la recompose depuis le \
             dépôt et le numéro, ce qui est correct ici puisque l'index \
             inverse est construit sur les PR du dépôt du milestone"
        );
        // Le contrôle négatif reste négatif quel que soit le chemin pris par
        // son voisin.
        let s1680 = state
            .sub_issues
            .iter()
            .find(|s| s.number == 1680)
            .expect("#1680 présent dans l'énumération");
        assert_eq!(s1680.pr_number, None);
        assert_eq!(s1680.pr_link_source, None);
    }

    // -----------------------------------------------------------------
    // mika#2924 R7 — la résolution du lien de PR a un lecteur unique.
    // -----------------------------------------------------------------

    /// Les trois aiguilles de la résolution du lien ticket ↔ PR de clôture.
    ///
    /// Trois, pas une : la régression redoutée n'a pas de forme canonique. Un
    /// second site peut rebâtir l'index inverse (`pr_by_issue`), relire le
    /// champ côté PR (`closingIssuesReferences`) ou relire celui côté issue
    /// (`closedByPullRequestsReferences`) — et le troisième est le plus
    /// probable, puisque c'est le chemin que ce ticket vient d'ouvrir. Un filet
    /// posé sur une seule des trois serait vert le jour où il compte.
    const AIGUILLES_DE_RESOLUTION: &[&str] = &[
        "pr_by_issue",
        "closingIssuesReferences",
        "closedByPullRequestsReferences",
    ];

    /// Les fonctions dont le corps porte l'une des aiguilles, dans un corpus
    /// `(fichier, source)` quelconque — l'ensemble trié et dédoublonné.
    ///
    /// Le découpage passe par [`crate::source_scan::fn_bodies`], qui retire les
    /// commentaires **avant** de découper : la prose de ce module *nomme* les
    /// trois champs à six endroits (le doc-comment de `PrLinkSource`, celui de
    /// `SubIssue::pr_number`, celui de `write_completed`, trois commentaires du
    /// composeur), et une mention n'est pas une lecture. C'est le motif
    /// mika#2050 signal S, et la raison pour laquelle
    /// `mika2425_identity_context_history_has_a_single_reader` ancre sur les
    /// lignes non commentaires.
    fn fonctions_portant_une_aiguille(corpus: &[(String, String)]) -> Vec<String> {
        let mut trouvees: Vec<String> = Vec::new();
        for (_, source) in corpus {
            for (nom, corps) in crate::source_scan::fn_bodies(source) {
                if AIGUILLES_DE_RESOLUTION.iter().any(|a| corps.contains(a)) {
                    trouvees.push(nom);
                }
            }
        }
        trouvees.sort();
        trouvees.dedup();
        trouvees
    }

    /// La moitié production de `src/milestone_manager/`, fichier par fichier.
    ///
    /// Passe par [`crate::source_scan::production_files`], qui compose déjà les
    /// deux écartements du code de test demandés par le plan — par le chemin
    /// ([`crate::source_scan::is_test_source_path`]) puis par la troncature
    /// ([`crate::source_scan::production_half`]). L'API déjà en place, prise là
    /// où elle est déjà composée : une troisième énumération du répertoire
    /// serait un second prédicat pour une seule question (mika#2158), et celle
    /// de `mod.rs` est plate là où celle-ci est récursive — un futur
    /// sous-répertoire du module y échapperait en silence.
    fn production_du_module() -> Vec<(String, String)> {
        let corpus: Vec<(String, String)> = crate::source_scan::production_files()
            .into_iter()
            .filter(|(rel, _)| rel.starts_with("milestone_manager/"))
            .collect();
        assert!(
            corpus.len() >= 10,
            "le corpus du module est anormalement petit ({} fichiers) — le \
             filtre de chemin ne mord plus, et le scan ne regarde presque rien",
            corpus.len()
        );
        corpus
    }

    /// mika#2924 R7 — **l'ensemble** des fonctions de production qui résolvent
    /// le lien ticket ↔ PR de clôture, par égalité d'ensembles et non par
    /// `contains`.
    ///
    /// Ce que ça refuse : un second site qui résout le lien à côté du premier.
    /// La classe est celle de mika#2158 — un second site correct *aujourd'hui*
    /// qui rate l'élargissement de demain, sans qu'aucune assertion de
    /// comportement ne rougisse, parce que les deux sites rendent les mêmes
    /// octets jusqu'au jour où l'un des deux change.
    ///
    /// **Deux noms attendus, et chacun pour sa raison** — le plan en annonçait
    /// un, n'ayant pas compté le site de *demande* :
    ///
    /// - `compose_from_gh_outputs` **résout** : c'est lui qui décide, pour un
    ///   ticket, quelle PR l'a fermé et d'où vient cette réponse.
    /// - `read_with_runner` **demande** : il nomme les deux champs dans ses
    ///   listes `--json`. Il ne répond pas à la question, il la rend posable —
    ///   la même distinction que mika#2305 a dû écrire pour son résolveur de
    ///   portée (« amended, not excepted … it *produces* the answer »).
    ///
    /// Quand ce test rougit : **router le site par `compose_from_gh_outputs`**,
    /// ne pas ajouter de ligne à l'allowlist (doctrine mika#2201). Si le
    /// nouveau site est un troisième *demandeur* légitime, l'attendu s'amende
    /// en nommant sa raison, comme les deux lignes ci-dessus.
    #[test]
    fn mika2924_la_resolution_de_pr_de_cloture_a_un_lecteur_unique() {
        /// Aucune exception. Livrée vide, et son vide est épinglé juste en
        /// dessous : une allowlist qui se remplit sans bruit est le mécanisme
        /// que mika#2309 condamne.
        const ALLOWLIST: &[&str] = &[];
        assert!(
            ALLOWLIST.is_empty(),
            "une exception a été ajoutée à ce scan. Ce n'est pas la \
             réparation : router le site par `compose_from_gh_outputs` l'est \
             (mika#2201). Si une exception est vraiment la bonne réponse, \
             ouvrir un ticket qui porte sa raison et le citer ici."
        );

        let trouvees = fonctions_portant_une_aiguille(&production_du_module());

        // Anti-vacuité : un prédicat qui ne trouve plus rien passerait toutes
        // les égalités d'ensembles qu'on lui demanderait, y compris celle-ci si
        // l'attendu venait à se vider par un refactor.
        assert!(
            !trouvees.is_empty(),
            "le scan ne trouve plus aucune lecture — les aiguilles ont cessé de \
             mordre (champ renommé, découpage cassé), et le filet est décoratif"
        );

        assert_eq!(
            trouvees,
            vec![
                "compose_from_gh_outputs".to_string(),
                "read_with_runner".to_string(),
            ],
            "l'ensemble des fonctions qui touchent au lien de PR de clôture a \
             changé. Lire le doc-comment de ce test avant de toucher à \
             l'attendu."
        );
    }

    /// Le contrôle de bonne foi du scan ci-dessus : sur une source **synthétique**
    /// portant un second résolveur, le prédicat doit rougir.
    ///
    /// Sans lui, un scan dont le découpage s'est cassé — aiguille renommée,
    /// `fn_bodies` qui rend des corps vides — resterait vert pour toujours, et
    /// son vert se lirait comme « un seul lecteur » au lieu de « je ne vois
    /// plus rien ». L'anti-vacuité du test précédent couvre le corpus réel ;
    /// celui-ci couvre le prédicat lui-même, sur une entrée dont on connaît la
    /// réponse.
    #[test]
    fn mika2924_le_scan_rougit_sur_un_second_lecteur() {
        let synthetique = r#"
/// Cette prose nomme `closedByPullRequestsReferences` et `pr_by_issue` sans
/// les lire : elle ne doit pas être comptée.
fn seulement_de_la_prose() {
    let _ = 1;
}

fn un_second_resolveur(issue: &Value) -> Option<u64> {
    issue
        .get("closedByPullRequestsReferences")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|r| r.get("number"))
        .and_then(|v| v.as_u64())
}
"#;
        let corpus = vec![("synthetique.rs".to_string(), synthetique.to_string())];
        let trouvees = fonctions_portant_une_aiguille(&corpus);
        assert_eq!(
            trouvees,
            vec!["un_second_resolveur".to_string()],
            "le prédicat doit voir le second résolveur, et lui seul — une \
             prose qui nomme le champ n'est pas une lecture"
        );
    }

    /// mika#1975 D5/D8 — the typed error renders **exactly** the legacy
    /// `anyhow!("gh {} failed: {}", args.join(" "), stderr)` string.
    ///
    /// Necessary because the tightening is on what is *classified*, never on
    /// what is *logged*: `verify_gh_auth`'s `stderr_head`, the
    /// `manager_cycle_error` `error` field and the frozen D7 corpus all read
    /// this rendering. A `Display` drifted by one space would amputate the
    /// operator's log without failing any behavioural assertion.
    #[test]
    fn mika1975_the_gh_command_error_display_is_byte_identical() {
        let args = ["api", "/repos/senara-solutions/mika/milestones/403"];
        let stderr = "HTTP 500: Internal Server Error";

        let legacy = format!("gh {} failed: {}", args.join(" "), stderr);
        let typed = GhCommandError {
            args: args.join(" "),
            stderr: stderr.to_string(),
        };

        assert_eq!(
            format!("{typed}"),
            legacy,
            "the typed error must render the legacy string byte for byte"
        );
        // And through `anyhow`, which is how every downstream reader sees it.
        let boxed = anyhow::Error::new(GhCommandError {
            args: args.join(" "),
            stderr: stderr.to_string(),
        });
        assert_eq!(format!("{boxed}"), legacy);
    }

    /// mika#1975 D8 — structural guard: `ProcessGhRunner::run` must not go back
    /// to formatting its error.
    ///
    /// **Why a source scan and not a behavioural test.** Reverting to
    /// `anyhow!("gh {} failed: {}", …)` breaks **nothing**: the classifier's
    /// documented string fallback (D5) keeps every assertion green while the
    /// command line quietly re-enters the classified text and re-widens the
    /// search space. The regression makes no decision *wrong*; it makes the
    /// tightening *absent*. That class is invisible to assertions on behaviour.
    ///
    /// The allowlist ships **empty** and stays empty: there is nothing to
    /// exempt, so there is no slot in which to drop the next lapse (doctrine
    /// mika#2323). When this fires, remove the site — do not allowlist it.
    #[test]
    fn mika1975_the_gh_runner_error_stays_typed() {
        const FORBIDDEN_NEEDLE: &str = r#"anyhow!("gh "#;
        /// Deliberately empty — see the test's doc comment.
        const ALLOWED_LEGACY_SITES: &[&str] = &[];

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("milestone_manager")
            .join("reader.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // Scan the production half only. This very test carries the needle as
        // a literal (and so does its anti-vacuity control below), so a whole-
        // file scan would flag the guard itself. `reader.rs` keeps its test
        // modules inline, so truncating at the first `#[cfg(test)]` excludes
        // every one of them.
        let cut = source.find("#[cfg(test)]").expect(
            "reader.rs must keep an inline test module for this truncation to mean anything",
        );
        let production = &source[..cut]; // safe-byte-slice: `find` returns a char boundary

        let offenders: Vec<(usize, &str)> = production
            .lines()
            .enumerate()
            // Prose that *describes* the forbidden shape is not a site. Only a
            // `//`-comment prefix is excluded; a real call sits at code level.
            .filter(|(_, line)| !line.trim_start().starts_with("//"))
            .filter(|(_, line)| line.contains(FORBIDDEN_NEEDLE))
            .filter(|(_, line)| !ALLOWED_LEGACY_SITES.iter().any(|a| line.contains(a)))
            .map(|(i, line)| (i + 1, line.trim()))
            .collect();

        assert!(
            offenders.is_empty(),
            "mika#1975 D5 regressed: `gh` failures must be `GhCommandError`, not a \
             formatted string — the command line would re-enter the text \
             `classify_cycle_error` reads. Offending sites:\n{}",
            offenders
                .iter()
                .map(|(n, l)| format!("  reader.rs:{n}: {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Anti-vacuity, two halves. (a) the scanned region is the one that
        // holds the runner — a truncation that cut too early would make this
        // guard pass on an empty string for ever.
        assert!(
            production.contains("impl GhRunner for ProcessGhRunner"),
            "the scanned region no longer contains the runner this guard protects"
        );
        // (b) the needle is the shape a regression would actually write.
        let regression =
            r#"            return Err(anyhow!("gh {} failed: {}", args.join(" "), stderr));"#;
        assert!(
            regression.contains(FORBIDDEN_NEEDLE),
            "the needle no longer matches the shape it exists to refuse"
        );
    }

    #[test]
    fn compute_progress_all_completed() {
        let subs = vec![SubIssue {
            number: 1,
            title: "t".into(),
            state: IssueState::Closed,
            priority_rank: None,
            plan_present: false,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_link_source: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "".into(),
            labels: vec![],
        }];
        let p = compute_progress(&subs);
        assert_eq!(p.completed, 1);
        assert_eq!(p.total, 1);
    }
}

/// Injection-verified arg-capture guards (mika#1933 AC6, plan § S4).
///
/// **What this guards:** the `gh` invocation contract in `Reader::read_with_runner`.
/// The AC1/AC2 lock-in (closed sub-issues included in the denominator) depends
/// on TWO adjacent-pair arg tokens on the `issue list` call — `"--state", "all"`
/// and `"--limit", "100"` — plus the matching `"--state", "all"` on the `pr list`
/// call. A regression that reverts either token (e.g. someone re-defaulting to
/// `--state open`, which is the ticket body's original hypothesis and the pre-PR#1932
/// state) would silently take the Reader back to reste-à-faire semantics.
///
/// **Why arg-capture, not subprocess spawn:** per plan § S4 F4 architect note —
/// arg-capture guards the failure mode named in AC6 ("sed-inject bug that re-adds
/// `--state open` → tests fail") without external dependency flakiness (network,
/// `gh` availability, rate limits). Full subprocess spawn is a distinct integration
/// concern out of scope for AC6.
#[cfg(test)]
mod gh_arg_capture_tests {
    // Renamed from `injection_tests` (mika#1933 maintainability F1) — the new
    // name telegraphs both mechanism (arg capture via injected `GhRunner`) and
    // failure class (adjacent-pair arg-token drift), so future maintainers do
    // not confuse it with dependency-injection or injection-attack testing.
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `GhRunner` that captures every argument vector into a shared vec
    /// and returns pre-canned JSON responses based on the first two args.
    ///
    /// The composer only needs syntactically-valid JSON to run to completion —
    /// the tests do not assert on composed state, only on the captured args.
    ///
    /// **Extension gap (mika#1933 maintainability F1 hoist):** the `impl GhRunner`
    /// match arm routes by the FIRST arg (`api` / `issue` / `pr`) — matching the
    /// three current `gh` call sites in `Reader::read_with_runner`. If a future
    /// change adds a fourth `gh` invocation (e.g., `gh auth status`, per-issue
    /// `gh issue view`), this recorder MUST be extended with a new match arm.
    /// The wildcard arm returns `Err(...)` and `.expect(...)` in the tests
    /// panics — the guard fails LOUD on an unmapped call. This docstring lives
    /// on the struct itself (not inside the match wildcard) so it surfaces at
    /// the most likely edit surface for the maintainer wiring the new call.
    ///
    /// **Extension performed twice (mika#2699).** The fourth and fifth calls
    /// *are* `issue list`s, so routing on the first arg alone would have served
    /// them `issues_json` in silence — a recorder that answers the wrong
    /// question without failing. The `issue` arms therefore disambiguate on a
    /// token each call site owns alone: `--label` for the ready pool,
    /// `--milestone` for the sub-issues, and the exact `--json` field list for
    /// the outside-candidate read. The trailing `issue` arm is an **error**, not
    /// a default: a sixth `issue`-shaped call must extend this match on a token
    /// of its own rather than inherit someone else's answer.
    ///
    /// **Extension performed a third time (mika#2699 vol 3).** The sixth call is
    /// an `api`, like the first. Both `api` arms now discriminate on the path
    /// (`/milestones/` vs `/contents/`), and neither is a fallback — a seventh
    /// `api`-shaped call lands on the error arm rather than being served the
    /// milestone metadata.
    struct RecordingGhRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        milestone_json: String,
        issues_json: String,
        pr_json: String,
        ready_json: String,
        outside_json: String,
        /// Le texte servi à la **sixième** lecture (mika#2699 vol 3), ou `None`
        /// pour simuler le `404` au ref de la branche.
        ///
        /// Même raison que pour les arms `issue` ci-dessus, un cran plus loin :
        /// la sixième lecture est elle aussi un `api`, donc l'arm `api` nu lui
        /// aurait servi les métadonnées du milestone — un plan sans section de
        /// taille, donc `SectionAbsent`, donc un test vert qui ne prouve rien.
        /// Les deux arms `api` discriminent sur le chemin.
        plan_text: Option<String>,
    }

    #[async_trait::async_trait]
    impl GhRunner for RecordingGhRunner {
        async fn run(&self, args: &[&str]) -> Result<String> {
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            self.calls.lock().unwrap().push(owned.clone());
            let has = |flag: &str| owned.iter().any(|a| a == flag);
            let path = owned.get(1).cloned().unwrap_or_default();
            let json_fields = owned
                .windows(2)
                .find(|w| w[0] == "--json")
                .map(|w| w[1].clone())
                .unwrap_or_default();
            match owned.first().map(String::as_str) {
                Some("api") if path.contains("/contents/") => self
                    .plan_text
                    .clone()
                    .ok_or_else(|| anyhow!("RecordingGhRunner: 404 au ref de la branche")),
                Some("api") if path.contains("/milestones/") => Ok(self.milestone_json.clone()),
                Some("issue") if has("--label") => Ok(self.ready_json.clone()),
                Some("issue") if has("--milestone") => Ok(self.issues_json.clone()),
                Some("issue") if json_fields == "number,body,labels,state" => {
                    Ok(self.outside_json.clone())
                }
                Some("pr") => Ok(self.pr_json.clone()),
                // NOTE: extending `Reader::read_with_runner` with a sixth `gh`
                // call (e.g., `gh auth status`, per-issue `gh issue view`) also
                // requires extending this match — the guard covers only the
                // five current call sites' arg contracts (mika#1933 testing F1).
                _ => Err(anyhow!(
                    "RecordingGhRunner: unexpected call shape in {:?}",
                    owned
                )),
            }
        }
    }

    fn build_recorder() -> (Arc<Mutex<Vec<Vec<String>>>>, RecordingGhRunner) {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let runner = RecordingGhRunner {
            calls: Arc::clone(&calls),
            // Minimal well-shaped JSON — composer runs to completion without
            // meaningful state, which is fine because these tests assert only
            // on the captured argument vectors.
            milestone_json: r#"{"title": "T", "description": "", "state": "open", "created_at": "2026-08-01T00:00:00Z", "due_on": null}"#.into(),
            issues_json: "[]".into(),
            pr_json: "[]".into(),
            ready_json: "[]".into(),
            outside_json: "[]".into(),
            plan_text: None,
        };
        (calls, runner)
    }

    /// Assert `flag` appears exactly once in `call` AND is immediately followed
    /// by `value`.
    ///
    /// The uniqueness check (mika#1933 adversarial F1) closes a false-pass
    /// gap: a plain `windows(2)` scan would accept an arg vector with
    /// `[..., "--state", "all", "--state", "open", ...]` because BOTH adjacent
    /// pairs exist. Under most clap-derived CLIs (`gh` included), duplicate
    /// flags silently take the LAST value, so the regression the AC6 guard
    /// aims to catch — reverting to reste-à-faire semantics — could slip
    /// through a duplicate-add mutation. Uniqueness + adjacency together
    /// pin the flag to its intended single value.
    fn assert_adjacent_pair(call: &[String], flag: &str, value: &str) {
        let occurrences = call.iter().filter(|a| a.as_str() == flag).count();
        assert_eq!(
            occurrences, 1,
            "expected exactly one occurrence of `{flag}` in call args, got {occurrences}: {:?}",
            call
        );
        for pair in call.windows(2) {
            if pair[0] == flag && pair[1] == value {
                return;
            }
        }
        panic!(
            "expected adjacent pair (`{flag}`, `{value}`) in call args, got: {:?}",
            call
        );
    }

    #[tokio::test]
    async fn reader_issue_list_uses_state_all_and_limit_100() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner should succeed on canned JSON");

        let calls = calls.lock().unwrap();
        let issue_call = calls
            .iter()
            .find(|c| c.first().map(String::as_str) == Some("issue"))
            .expect("issue list call captured");
        // Adjacent-pair assertions — a sed-inject that re-adds `--state open`
        // or drops `--limit 100` would fail here.
        assert_adjacent_pair(issue_call, "--state", "all");
        assert_adjacent_pair(issue_call, "--limit", "100");
        // Sanity: the milestone number is threaded through.
        assert_adjacent_pair(issue_call, "--milestone", "31");
    }

    #[tokio::test]
    async fn reader_pr_list_uses_state_all_and_limit_100() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner should succeed on canned JSON");

        let calls = calls.lock().unwrap();
        let pr_call = calls
            .iter()
            .find(|c| c.first().map(String::as_str) == Some("pr"))
            .expect("pr list call captured");
        // PR-list guards: closed/merged PRs contribute the `closingIssuesReferences`
        // reverse index that powers the Reporter `Completed` section's PR links.
        // Dropping `--state all` here would hide merged PRs → closed sub-issues
        // would render `closed (no linked PR)` in every case.
        assert_adjacent_pair(pr_call, "--state", "all");
        assert_adjacent_pair(pr_call, "--limit", "100");
    }

    // -- mika#2924 : la référence de clôture côté issue, sans appel de plus --

    /// **V5** — la deuxième lecture demande `closedByPullRequestsReferences`, et
    /// le **nombre** d'appels `gh` ne bouge pas.
    ///
    /// Les deux moitiés portent la décision, et elles sont distinctes :
    ///
    /// - le champ, parce que sans lui la référence côté issue n'arrive jamais et
    ///   le correctif est décoratif — l'index des PR du milestone reste vide
    ///   pour toute PR de la boucle, qui rattache le milestone au ticket ;
    /// - le compte, parce que la réparation évidente — un `gh` par ticket fermé
    ///   — coûterait 87 appels par cycle sur le milestone 33 là où ce champ en
    ///   coûte zéro. La lecture était déjà émise ; on lui demande une colonne de
    ///   plus.
    ///
    /// L'assertion de compte double celle de `mika2699_la_cinquieme_lecture_est_depot_large`
    /// **à dessein** : elle y est un contrôle de non-régression du vol 2, elle est
    /// ici la moitié mesurée du § coût du plan. Les deux rougiraient ensemble, et
    /// c'est ce qu'on veut — un sixième appel ajouté pour servir ce champ serait
    /// exactement la régression que ce test refuse.
    #[tokio::test]
    async fn mika2924_la_lecture_des_tickets_demande_la_reference_de_cloture_sans_appel_de_plus() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 33,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner doit réussir sur du JSON canné");

        let calls = calls.lock().unwrap();
        let issue_call = calls
            .iter()
            .find(|c| {
                c.first().map(String::as_str) == Some("issue")
                    && c.iter().any(|a| a == "--milestone")
            })
            .expect("l'appel d'énumération des sous-tickets doit être capturé");
        assert_adjacent_pair(
            issue_call,
            "--json",
            "number,title,state,body,labels,updatedAt,closedByPullRequestsReferences",
        );
        assert_eq!(
            calls.len(),
            5,
            "la référence de clôture arrive sur une lecture déjà émise — aucun \
             sixième appel `gh` par cycle : {calls:?}"
        );
    }

    // -- mika#2699 Phase 2 (a) vol 1 : la quatrième lecture --

    /// Cas 9 — la lecture du bassin est **dépôt-large** et ciblée sur `ready`.
    ///
    /// Les deux assertions qui portent la décision : `--label ready` (c'est bien
    /// le bassin qu'on lit) et **l'absence de `--milestone`** (le webhook `ready`
    /// dispatche sur tout le dépôt, donc un bassin compté sur le seul milestone
    /// serait sous-compté et la recommandation fausse le jour où elle compte).
    /// `--state open` parce qu'un ticket `ready` fermé n'occupe aucun rail.
    #[tokio::test]
    async fn mika2699_la_lecture_du_bassin_est_depot_large_et_sur_ready() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner should succeed on canned JSON");

        let calls = calls.lock().unwrap();
        let ready_call = calls
            .iter()
            .find(|c| {
                c.first().map(String::as_str) == Some("issue") && c.iter().any(|a| a == "--label")
            })
            .expect("l'appel du bassin ready doit être capturé");
        assert_adjacent_pair(ready_call, "--label", "ready");
        assert_adjacent_pair(ready_call, "--state", "open");
        assert_adjacent_pair(ready_call, "--json", "number,labels");
        assert_adjacent_pair(ready_call, "--limit", "100");
        assert_adjacent_pair(ready_call, "--repo", "senara-solutions/mika");
        assert!(
            !ready_call.iter().any(|a| a == "--milestone"),
            "le bassin est dépôt-large : aucun --milestone ici — {ready_call:?}"
        );
    }

    /// Cas 10 — `compose_ready_pool` sur une sortie `gh` réelle, et sur `[]`.
    ///
    /// Le fixture est l'état mesuré le 2026-10-15 : deux issues `ready` ouvertes,
    /// **les deux** portant `dispatch:loop`. Il porte aussi un label sans `name`
    /// exploitable, pour que la lecture ne s'effondre pas sur une forme que la
    /// forge peut rendre.
    #[test]
    fn mika2699_compose_ready_pool_lit_numeros_et_labels() {
        let json = r#"[
            {"number": 2699, "labels": [{"name":"ready"},{"name":"dispatch:loop"}]},
            {"number": 2687, "labels": [{"name":"ready"},{"name":"dispatch:loop"},{"id":"x"}]}
        ]"#;
        let pool = compose_ready_pool(json).expect("une sortie gh valide doit se composer");
        assert_eq!(pool.len(), 2);
        assert_eq!(pool[0].number, 2699);
        assert_eq!(
            pool[0].labels,
            vec!["ready".to_string(), "dispatch:loop".to_string()]
        );
        assert!(
            pool.iter().all(|t| t.holds_dispatch_seat()),
            "les deux portaient un siège le 2026-10-15 — {pool:?}"
        );
        assert_eq!(
            pool[1].labels.len(),
            2,
            "un label sans `name` est ignoré, il ne fait pas échouer la lecture"
        );

        let empty = compose_ready_pool("[]").expect("un bassin vide est une réponse valide");
        assert!(
            empty.is_empty(),
            "`[]` donne un bassin vide, pas une erreur"
        );
    }

    // -- mika#2699 vol 2 : la cinquième lecture --

    /// Cas 11 — la lecture hors milestone est **dépôt-large** et n'ouvre aucune
    /// autorité.
    ///
    /// L'assertion qui porte le sens est l'absence de `--milestone` : avec elle,
    /// la cinquième lecture rendrait le même périmètre que la deuxième et le
    /// « reste » du 2e critère de tri serait vide à chaque cycle — un vol 2 vert
    /// qui ne mesure rien. L'absence de `--label` compte autant : filtrer sur
    /// `ready` ne ramènerait que des tickets déjà dans le bassin.
    #[tokio::test]
    async fn mika2699_la_cinquieme_lecture_est_depot_large() {
        let (calls, runner) = build_recorder();
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("read_with_runner doit réussir sur du JSON canné");

        let calls = calls.lock().unwrap();
        let outside_call = calls
            .iter()
            .find(|c| {
                c.first().map(String::as_str) == Some("issue")
                    && c.windows(2)
                        .any(|w| w[0] == "--json" && w[1] == "number,body,labels,state")
            })
            .expect("l'appel des candidats hors milestone doit être capturé");
        assert_adjacent_pair(outside_call, "--state", "open");
        assert_adjacent_pair(outside_call, "--json", "number,body,labels,state");
        assert_adjacent_pair(outside_call, "--limit", "100");
        assert_adjacent_pair(outside_call, "--repo", "senara-solutions/mika");
        assert!(
            !outside_call.iter().any(|a| a == "--milestone"),
            "sans cette absence, le « reste » serait vide à chaque cycle — {outside_call:?}"
        );
        assert!(
            !outside_call.iter().any(|a| a == "--label"),
            "filtrer sur `ready` ne ramènerait que le bassin — {outside_call:?}"
        );
        assert_eq!(
            calls.len(),
            5,
            "cinq lectures, pas une de plus : chaque appel `gh` est un coût par \
             cycle — {calls:?}"
        );
    }

    /// Cas 12 — le composeur dérive priorité et plan, et retire le milestone.
    ///
    /// Le fixture oppose les trois choses qu'un candidat du dehors doit porter
    /// pour que le tri du point 1 soit juste : un rang venu des labels, un
    /// `plan_present` venu du corps, et un état. Le ticket du milestone (#1802)
    /// est dans la page lue par construction — la lecture est dépôt-large — et
    /// doit en sortir.
    #[test]
    fn mika2699_compose_outside_candidates_derive_et_retire_le_milestone() {
        let json = r#"[
            {"number": 1700, "body": "> - **Plan:** docs/plans/x.md", "state": "OPEN",
             "labels": [{"name":"p0-critical"}]},
            {"number": 1701, "body": "pas de plan ici", "state": "OPEN", "labels": []},
            {"number": 1802, "body": "> - **Plan:** docs/plans/y.md", "state": "OPEN",
             "labels": [{"name":"p1-important"}]}
        ]"#;
        let subs = vec![SubIssue {
            number: 1802,
            title: "déjà dans sub_issues".into(),
            state: IssueState::Open,
            priority_rank: Some(1),
            plan_present: true,
            branch_present: false,
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_link_source: None,
            ci_state: CiState::None,
            blockers: vec![],
            updated_at: "2026-10-15T00:00:00Z".into(),
            labels: vec!["p1-important".into()],
        }];

        let (cands, truncated) =
            compose_outside_candidates(json, &subs).expect("une sortie gh valide se compose");
        assert!(!truncated, "3 issues sur 100 : la page n'est pas pleine");
        assert_eq!(
            cands.iter().map(|c| c.number).collect::<Vec<_>>(),
            vec![1700, 1701],
            "le ticket du milestone sort, les autres restent — {cands:?}"
        );
        assert_eq!(
            cands[0].priority_rank,
            Some(0),
            "p0-critical vaut le rang 0"
        );
        assert!(cands[0].plan_present, "le callout de plan est lu");
        assert_eq!(cands[1].priority_rank, None, "aucun label ⇒ non priorisé");
        assert!(!cands[1].plan_present);

        let (vide, t) = compose_outside_candidates("[]", &subs).expect("`[]` est valide");
        assert!(vide.is_empty());
        assert!(!t, "une page vide n'est pas une page pleine");
    }

    /// Cas 13 — une page pleine vide l'ensemble et le **déclare**.
    ///
    /// C'est le contrôle négatif de la garde : sans elle, le manager
    /// recommanderait le meilleur candidat de la page lue en affirmant un ordre
    /// global qu'il n'a pas mesuré. Et le booléen porte sur la page **avant**
    /// l'exclusion du milestone, sinon un milestone de cent tickets rendrait
    /// « non tronqué » sur une page saturée.
    #[test]
    fn mika2699_une_page_pleine_vide_lensemble_et_se_declare() {
        let page: Vec<String> = (1..=100)
            .map(|n| format!(r#"{{"number": {n}, "body": "", "state": "OPEN", "labels": []}}"#))
            .collect();
        let json = format!("[{}]", page.join(","));

        let (cands, truncated) =
            compose_outside_candidates(&json, &[]).expect("une page pleine reste du JSON valide");
        assert!(truncated, "100 issues à la borne 100 : page pleine");
        assert!(
            cands.is_empty(),
            "refuser plutôt que mal répondre : aucun candidat retenu — {} retenus",
            cands.len()
        );
    }

    // -- mika#2699 vol 3 : la sixième lecture, le plan au ref de sa branche --

    /// Un corps d'issue groomée : les deux callouts que la sixième lecture
    /// épelle, et rien d'autre. `\n` littéraux — c'est du JSON.
    const CORPS_GROOME: &str = "> - **Branch:** `feat/1700/x`\\n\
         > - **Plan:** `docs/plans/2026-10-01-001-feat-1700-x-plan.md` \
         (committed on branch @ `abc1234`)";

    const PLAN_DE_500: &str = "# Plan\n\n## Taille estimée\n\nTotal estimé : 500 lignes\n";

    fn outside_json_un_candidat(corps: &str) -> String {
        format!(
            r#"[{{"number": 1700, "body": "{corps}", "state": "OPEN",
                 "labels": [{{"name": "p0-critical"}}]}}]"#
        )
    }

    /// Cas 14 — la sixième lecture épelle `/contents/<chemin>?ref=<branche>`, et
    /// son total atterrit dans l'état.
    ///
    /// Trois décisions tiennent dans cet argv, et chacune est invisible depuis
    /// `pool.rs`. **`?ref=<branche>`** : le plan d'un ticket groomé est commité
    /// sur *sa* branche — lire `main` répondrait « introuvable » pour tout
    /// candidat correctement groomé, c'est-à-dire exactement ceux que la borne
    /// doit juger. **Le chemin normalisé** : `plan_callout` retire le segment de
    /// tête (`mika/…`), qui est le préfixe du dépôt dans le monorepo et n'existe
    /// pas à l'intérieur du sous-dépôt que `/contents/` adresse. **`Accept:
    /// …raw`** : sans lui la forge rend une enveloppe JSON base64, et le contrat
    /// de parse porté du bash travaillerait sur du base64 — `SectionAbsent` sur
    /// tous les plans du monde, silencieusement.
    #[tokio::test]
    async fn mika2699_la_sixieme_lecture_lit_le_plan_au_ref_de_sa_branche() {
        let (calls, mut runner) = build_recorder();
        runner.outside_json = outside_json_un_candidat(CORPS_GROOME);
        runner.plan_text = Some(PLAN_DE_500.into());
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        let state = Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("la lecture doit aboutir");

        let calls = calls.lock().unwrap();
        let plan_call = calls
            .iter()
            .find(|c| c.iter().any(|a| a.contains("/contents/")))
            .expect("la sixième lecture doit être capturée");
        assert_eq!(
            plan_call,
            &[
                "api",
                "/repos/senara-solutions/mika/contents/\
                 docs/plans/2026-10-01-001-feat-1700-x-plan.md?ref=feat/1700/x",
                "-H",
                "Accept: application/vnd.github.raw",
            ]
        );
        assert_eq!(
            state.plan_sizes.get(&1700),
            Some(&PlanSizeReading::Measured(500)),
            "le total lu entre dans l'état, mesuré : {:?}",
            state.plan_sizes
        );
    }

    /// Cas 15 — un `404` au ref se nomme, et ne se confond pas avec un plan non
    /// dimensionné.
    ///
    /// Contrôle négatif du cas précédent **et** séparation de deux populations
    /// dont les remèdes sont opposés (classe mika#2205) : un plan introuvable se
    /// corrige en poussant la branche, un plan sans section de taille en
    /// l'éditant. Confondre les deux enverrait l'opérateur au mauvais endroit —
    /// et confondre l'un ou l'autre avec « au-delà de la borne » refuserait un
    /// ticket que personne n'a mesuré.
    #[tokio::test]
    async fn mika2699_un_plan_introuvable_au_ref_se_nomme_et_ne_refuse_pas() {
        let (_calls, mut runner) = build_recorder();
        runner.outside_json = outside_json_un_candidat(CORPS_GROOME);
        runner.plan_text = None; // le `404`
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        let state = Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("un plan introuvable ne fait pas échouer le cycle");

        assert_eq!(
            state.plan_sizes.get(&1700),
            Some(&PlanSizeReading::Unread(UnreadCause::FetchFailed)),
            "la cause dit quel geste corrige : {:?}",
            state.plan_sizes
        );
    }

    /// Cas 16 — un candidat sans callout de branche ne coûte aucun appel.
    ///
    /// Le ticket porte bien un callout de plan — `plan_callout_present` le voit,
    /// donc il passe le filtre d'admission du vol 2 — mais pas de quoi désigner
    /// un ref. La lecture ne part pas : il n'y a pas d'adresse à lire, et un
    /// appel au hasard sur `main` répondrait pour un autre fichier que celui du
    /// ticket.
    #[tokio::test]
    async fn mika2699_sans_callout_de_branche_aucun_appel_ne_part() {
        let (calls, mut runner) = build_recorder();
        runner.outside_json = outside_json_un_candidat(
            "> - **Plan:** `docs/plans/2026-10-01-001-feat-1700-x-plan.md` \
             (committed on branch @ `abc1234`)",
        );
        runner.plan_text = Some(PLAN_DE_500.into());
        let mref = MilestoneRef {
            repo: "senara-solutions/mika".into(),
            number: 31,
        };
        let state = Reader::new(None)
            .read_with_runner(&mref, &runner)
            .await
            .expect("la lecture doit aboutir");

        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.iter().any(|a| a.contains("/contents/"))),
            "aucune lecture de plan sans ref à lire"
        );
        assert_eq!(
            state.plan_sizes.get(&1700),
            Some(&PlanSizeReading::Unread(UnreadCause::NoCallout)),
            "et le fait est dit, pas tu : {:?}",
            state.plan_sizes
        );
    }

    /// Cas 17 — sur un numéro présent dans les deux lectures, le milestone
    /// gagne.
    ///
    /// Même précédence qu'à l'admission, où un tel ticket est jugé `Milestone` :
    /// deux réponses divergentes à « d'où vient #42 » feraient lire le plan d'un
    /// ticket au ref annoncé par l'autre. Le corps hors milestone est ici
    /// délibérément **mieux formé** que celui du milestone — si la précédence
    /// s'inversait, le test verrait un callout là où il en attend zéro.
    #[test]
    fn mika2699_sur_un_numero_commun_le_corps_du_milestone_gagne() {
        let milestone = r#"[{"number": 42, "body": "du milestone"}]"#;
        let dehors = format!(r#"[{{"number": 42, "body": "{CORPS_GROOME}"}}]"#);

        let bodies = compose_candidate_bodies(milestone, &dehors).expect("deux JSON valides");
        assert_eq!(
            bodies.get(&42).map(String::as_str),
            Some("du milestone"),
            "le milestone écrase le dehors"
        );
    }

    /// Cas 18 — un corps absent de la sortie `gh` est un `NoCallout`, pas un
    /// panic ni un zéro.
    ///
    /// `compose_candidate_bodies` ne promet rien sur un numéro : `gh` peut
    /// rendre une issue sans champ `body`. Le vide résultant n'a ni callout ni
    /// branche, donc la même cause que le cas 16 — et surtout aucune taille, ce
    /// qu'un `0` par défaut aurait rendu comme « tient largement la borne »
    /// (motif mika#2331).
    #[test]
    fn mika2699_une_issue_sans_champ_body_rend_une_chaine_vide() {
        let bodies = compose_candidate_bodies(r#"[{"number": 42}]"#, "[]")
            .expect("un body absent reste du JSON valide");
        assert_eq!(bodies.get(&42).map(String::as_str), Some(""));
    }
}

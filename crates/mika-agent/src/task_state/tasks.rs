//! Task lifecycle types and Database query/write methods.

/// Default value for the `tasks.type` column. New tasks default to `"issue"`
/// unless the caller explicitly requests `"milestone"` or `"project"`.
pub const TASK_TYPE_ISSUE: &str = "issue";
pub const TASK_TYPE_MILESTONE: &str = "milestone";
pub const TASK_TYPE_PROJECT: &str = "project";

/// All valid values for the `tasks.type` column. Enforced by SQLite CHECK and by
/// the `create_task` tool boundary. Order is the documented enum order.
pub const VALID_TASK_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];

// ===== Tracking-row cleanup (mika#1934) =====
//
// A dispatch-tracking row (`trigger_type='manual'`, `action_type='none'`,
// `process_id IS NULL`) is left behind in `blocked` when an escalation fires and
// the underlying ticket is later resolved out-of-band. These constants back the
// two cleanup surfaces that terminal-mark such rows: supersede-on-new-dispatch
// (AC2) and complete-on-upstream-close (AC4). Each `result` string is a
// stable, greppable discriminator — do not spell them by hand at a call site.

/// The `?phase=groom` URL suffix the LLM-driven grooming path appends to an
/// issue URL. `.../issues/1574` and `.../issues/1574?phase=groom` are
/// DIFFERENT `reference_url`s; the cleanup surfaces canonicalize on the base
/// URL so a fresh dispatch supersedes both variants for the same underlying
/// issue. The dispatch gate
/// (`crates/mika-agent/src/db.rs::has_completed_groom_for_issue`) no longer
/// appends this suffix — since mika#2287 it reads the groom callback row and
/// accepts the parent URL in either form.
pub const GROOM_PHASE_SUFFIX: &str = "?phase=groom";

/// The convergence marker dispatch-lib writes into a `dev-groom` callback's
/// `result` (`skills/bundled/_shared/dispatch-lib.sh`, `Outcome:` line of the
/// RESULT posted via `POST /tasks/{id}/complete`). Two readers share it and
/// must never drift apart: the engine-side auto-fire
/// (`task_engine::dispatcher::try_dispatch_pilot_after_groom_success`) and the
/// dispatch-classification gate (`db::Database::has_completed_groom_for_issue`,
/// mika#1620 / mika#2287). Writer lives in shell; keep this literal identical
/// to the one dispatch-lib emits.
///
/// **Depuis mika#2590, ces deux lecteurs passent par
/// [`groom_result_convergence`] et jamais par un `contains` / `instr`.** Un
/// troisième lecteur lâche est refusé par un scan de source
/// (`canonical_tokens::tests::mika2590_le_marqueur_de_convergence_na_quun_lecteur`).
pub const GROOM_SUCCESS_MARKER: &str = "Outcome: PLAN_GROOMED";

/// Motif de refus — le `result` est une **enveloppe JSON portant `status`**
/// (mika#2590 R2).
///
/// # FORMAT DE FIL
///
/// Atterrit dans `audit_events.after_value` et l'opérateur en fait des
/// `GROUP BY` : deux orthographes d'un même motif couperaient une population en
/// deux sans le dire. Motif `ALL_GROOM_ESCALATE_VERDICTS` (mika#2545),
/// `ALL_PURGE_REFUSAL_REASONS` (mika#2497).
///
/// **Régime attendu : vide.** `grep -o '"status":"[a-z_]*"' dispatch-lib.sh`
/// rend **exactement une** valeur, `auto_skipped`, et aucun chemin n'écrit une
/// convergence en JSON — donc l'allowlist des statuts convergents est vide et la
/// règle exacte est *une enveloppe JSON portant `status` n'est jamais une
/// preuve*. C'est une défense en profondeur ; une occurrence signifie qu'un
/// producteur JSON est apparu, ce qui est un **résultat**, pas une panne.
pub const GROOM_REJECTED_JSON_ENVELOPE: &str = "json_envelope";

/// Motif de refus — le marqueur est présent mais **pas en position de verdict**
/// (mika#2590 R3). Même contrat de format de fil que son voisin.
///
/// **Régime attendu : non vide, décroissant.** Chaque ligne est un implement
/// sans plan re-mesuré que la porte n'a pas laissé partir.
pub const GROOM_REJECTED_NOT_LINE_ANCHORED: &str = "marker_not_line_anchored";

/// Les deux motifs, à un seul site, pour que leur cardinalité soit assertable.
pub const ALL_GROOM_CONVERGENCE_REJECTIONS: &[&str] = &[
    GROOM_REJECTED_JSON_ENVELOPE,
    GROOM_REJECTED_NOT_LINE_ANCHORED,
];

/// Un `result` de callback de groom porte-t-il une convergence ? (mika#2590)
///
/// # Le défaut que ça ferme, mesuré le 2026-09-29 sur mika#2105
///
/// La preuve se lisait par **sous-chaîne** — `instr(child.result, 'Outcome:
/// PLAN_GROOMED') > 0`. Or le RESULT d'auto-skip que `dispatch-lib.sh` écrit sur
/// un `already_groomed` **cite le marqueur en toutes lettres** dans son champ
/// `note`, pour expliquer qu'aucune preuve n'est frappée :
/// `select instr(result,'Outcome: PLAN_GROOMED') …` a rendu **651**. Le texte qui
/// dit « ceci n'est pas une preuve » **était** la preuve, et un pilote
/// *implement* est parti sur un ticket jamais re-groomé.
///
/// Classe déjà mesurée deux fois : mika#2050 sur le Signal S (« un pilote qui
/// *discute* du jeton se lit comme une émission ») et mika#2545 un marqueur plus
/// loin, dont [`crate::skills::executor::groom_escalate_verdict`] est le frère —
/// **déjà ancré**, avec ce raisonnement écrit mot pour mot.
///
/// # Trois états, et le troisième est la population du correctif
///
/// [`GroomConvergence::Absent`] et [`GroomConvergence::MarkerOutOfPosition`]
/// appellent la **même** disposition (refuser) et **deux lectures opérateur
/// différentes** : la première est le régime nominal d'un premier grooming, la
/// seconde est un implement que la porte vient d'arrêter. Les fondre rendrait la
/// population du correctif incomptable — motif [`crate::skills::executor::GroomedState`]
/// (mika#2484 D1), `phantom_aged_out` / `phantom_sweep_spared` (mika#2156).
///
/// # L'ordre des quatre tests, et pourquoi le motif n'est posé que s'il y a
/// quelque chose à écarter
///
/// L'enveloppe JSON est testée **d'abord** : c'est la forme mesurée, et R2 la
/// refuse « quel que soit son contenu ». Mais une enveloppe JSON qui ne cite pas
/// le marqueur rend [`GroomConvergence::Absent`] et non un motif : un saut
/// ordinaire (`issue_closed`, mika#988) n'a rien écarté, et le compter
/// polluerait le compteur de R6 avec une population qui n'a jamais menacé la
/// porte.
///
/// # Ce que `_set_outcome_line` garantit, et pourquoi l'ancrage ne perd rien
///
/// Depuis mika#2590 U5a le producteur de la convergence pose sa ligne par
/// `_set_outcome_line` (mika#2492), qui rend « exactement une ligne `Outcome:`
/// ancrée » vraie **par construction**. Avant, le couple
/// `sed 's/Outcome: .*/…/'` puis `grep -qF` — **ni l'un ni l'autre ancrés** —
/// pouvait poser le marqueur en milieu de ligne et le voir, donc ne pas
/// ajouter la ligne canonique : un groom
/// réellement convergé aurait été refusé par cette fonction. Les deux moitiés
/// voyagent dans le même binaire (`skills/bundled/` est une projection du
/// binaire, mika#2340), donc elles ne peuvent pas être servies séparément.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroomConvergence {
    /// Une ligne du `result` commence par [`GROOM_SUCCESS_MARKER`].
    Converged,
    /// Le marqueur n'apparaît nulle part. Le cas nominal d'un premier grooming.
    Absent,
    /// Le marqueur apparaît, hors position de verdict. Porte son motif, qui est
    /// un élément d'[`ALL_GROOM_CONVERGENCE_REJECTIONS`].
    MarkerOutOfPosition(&'static str),
}

impl GroomConvergence {
    /// Ce verdict prouve-t-il le grooming ?
    ///
    /// # Pourquoi une méthode plutôt qu'un `matches!` au site d'appel
    ///
    /// La question « un `dev-pilot` peut-il partir ? » n'a qu'une bonne réponse
    /// — `Converged` — et **deux** refus, dont l'un est arrivé après coup. Un
    /// `!matches!(v, MarkerOutOfPosition(_))` écrit à la main compile, se lit
    /// comme une garde, et accepte `Absent` : le contournement exact que ce
    /// ticket ferme, reproduit un cran plus loin. C'est la classe que mika#1940
    /// a dû nommer sur `RunStatus` (`matches!` et `if let` sont les deux formes
    /// qui continuent de compiler quand une variante apparaît).
    ///
    /// **Ce n'est PAS un substitut au `match` exhaustif** des deux sites qui
    /// doivent *disposer* différemment des deux refus :
    /// [`crate::skills::executor::groomed_state`] compte
    /// `MarkerOutOfPosition` (R6) là où `Absent` est le régime nominal d'un
    /// premier grooming. Un booléen y fondrait les deux populations et rendrait
    /// le correctif incomptable (D2).
    pub fn is_converged(&self) -> bool {
        matches!(self, GroomConvergence::Converged)
    }
}

/// Le lecteur **unique** de [`GROOM_SUCCESS_MARKER`]. Fonction pure, testable
/// aux bornes sans base (mika#2590 R1).
pub fn groom_result_convergence(result: &str) -> GroomConvergence {
    let mentions_marker = result.contains(GROOM_SUCCESS_MARKER);

    // 1. Enveloppe JSON portant `status` ⇒ jamais une preuve (R2).
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(result)
        && map.contains_key("status")
    {
        return if mentions_marker {
            GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE)
        } else {
            GroomConvergence::Absent
        };
    }

    // 2. Le marqueur en position de verdict (R3).
    if result
        .lines()
        .any(|line| line.starts_with(GROOM_SUCCESS_MARKER))
    {
        return GroomConvergence::Converged;
    }

    // 3. Présent, mais ailleurs — la population que R6 existe pour compter.
    if mentions_marker {
        return GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_NOT_LINE_ANCHORED);
    }

    GroomConvergence::Absent
}

/// Agrège les verdicts des callbacks de groom d'une même issue.
///
/// **La préséance est celle de la sémantique historique**, `COUNT(*) > 0` : une
/// **seule** preuve valide suffit, et l'agrégat ne doit pas se laisser dégrader
/// par un saut qui la précède dans l'ordre de balayage. À défaut de preuve, un
/// écart *mesuré* l'emporte sur l'absence — sinon un ticket refoulé pour preuve
/// polluée se lirait exactement comme un ticket jamais groomé, qui est la classe
/// mika#2205 appliquée au correctif lui-même.
pub fn aggregate_groom_convergence(
    verdicts: impl IntoIterator<Item = GroomConvergence>,
) -> GroomConvergence {
    let mut rejected: Option<&'static str> = None;
    for verdict in verdicts {
        match verdict {
            GroomConvergence::Converged => return GroomConvergence::Converged,
            GroomConvergence::MarkerOutOfPosition(reason) => rejected = rejected.or(Some(reason)),
            GroomConvergence::Absent => {}
        }
    }
    match rejected {
        Some(reason) => GroomConvergence::MarkerOutOfPosition(reason),
        None => GroomConvergence::Absent,
    }
}

/// `tasks.result` reason written when a phantom tracking row is cancelled
/// because a fresh dispatch superseded it (mika#1934 AC2). SOLE WRITER:
/// [`crate::db::Database::cancel_task_superseded`].
pub const SUPERSEDED_BY_NEW_DISPATCH: &str = "superseded_by_new_dispatch";

/// `tasks.result` reason written when a tracking row is cancelled because its
/// GitHub issue was closed upstream (mika#1934 AC4, `issues.closed`).
pub const ISSUE_CLOSED_UPSTREAM: &str = "issue_closed_upstream";

/// `tasks.result` reason written when a tracking row is completed because its
/// linked PR was merged upstream (mika#1934 AC4, `pull_request.closed` merged).
pub const UPSTREAM_PR_MERGED: &str = "upstream_pr_merged";

/// `tasks.result` reason written when a tracking row is cancelled because its
/// linked PR was closed unmerged upstream (mika#1934 AC4, `pull_request.closed`
/// unmerged).
pub const UPSTREAM_PR_CLOSED_UNMERGED: &str = "upstream_pr_closed_unmerged";

/// `audit_events.tool_name` emitted per row superseded by a fresh dispatch
/// (mika#1934 AC2).
pub const TRACKING_ROW_SUPERSEDED_TOOL: &str = "tracking_row_superseded";

/// `audit_events.tool_name` emitted per row terminal-marked on upstream close
/// (mika#1934 AC4).
pub const TRACKING_ROW_UPSTREAM_CLOSED_TOOL: &str = "tracking_row_upstream_closed";

/// Strip the `?phase=groom` suffix from a `reference_url`, returning the
/// canonical base issue URL. A URL without the suffix is returned unchanged.
/// Used by both tracking-row cleanup surfaces (mika#1934 AC2.2 / AC4.b) so the
/// exact-URL and groom-variant rows are both matched from one base URL.
pub fn strip_groom_phase_suffix(reference_url: &str) -> &str {
    reference_url
        .strip_suffix(GROOM_PHASE_SUFFIX)
        .unwrap_or(reference_url)
}

/// Les deux — et seulement les deux — écritures de `reference_url` qui
/// désignent une issue donnée (mika#2638).
///
/// `base_url` est **normalisé ici** : le helper retire lui-même un
/// `?phase=groom` résiduel via [`strip_groom_phase_suffix`], qui est idempotent.
/// Les appelants y arrivent par deux chemins — les sondes de vol
/// **construisent** l'URL depuis `owner/repo` + numéro, donc elle est canonique
/// par construction ; les surfaces de nettoyage de mika#1934 reçoivent une
/// `reference_url` de la base et la nettoyaient déjà en amont. La normalisation
/// interne ne change donc **aucune** sortie actuelle : elle rend le helper
/// fail-safe pour un futur appelant, au lieu de faire dépendre sa justesse
/// d'une précondition que rien ne vérifie.
///
/// # Pourquoi une énumération, et jamais un préfixe `LIKE`
///
/// **Un préfixe ne délimite pas un numéro.** La sonde de
/// `…/issues/216` écrite `reference_url LIKE '…/issues/216%'` matche aussi les
/// tâches de `…/issues/2160` à `…/issues/2169`, et `…/issues/21600` — le défaut
/// mesuré par la revue de PR #2635, dont la conséquence était que le message
/// « le bassin est coincé en vol » de mika#2161 **nommait la tâche d'un autre
/// ticket**. La maison avait déjà tranché cette classe trois fois en délimitant
/// (`degroom_marker_key` de mika#2347, qui écrit le piège `#234` vs `#2343` mot
/// pour mot ; `hold_audit_key` de mika#2199 et le faucheur de mika#2420, tous
/// deux bornés par un séparateur `@`) ; les sondes de vol ne l'avaient pas
/// appliquée.
///
/// Le préfixe portait **deux** élargissements de plus, tous deux fermés
/// gratuitement par l'égalité :
///
/// - **`_` est un joker `LIKE` d'un caractère.** GitHub autorise `_` dans un nom
///   de dépôt, donc une sonde pour `…/my_repo/issues/42` matchait
///   `…/myXrepo/issues/42`. Population vide aujourd'hui, réelle demain.
/// - **`LIKE` est insensible à la casse en ASCII** alors que `reference_url` est
///   déclarée `TEXT` sans `COLLATE NOCASE`. Conséquence de **justesse**, pas de
///   performance : `…/ISSUES/216` matchait, et l'égalité le refuse. Population
///   vide par construction (tout écrivain bâtit l'URL depuis des littéraux
///   minuscules et la casse canonique de GitHub), donc nommée plutôt
///   qu'invisible.
///
/// **Ce que le changement n'achète PAS, et il faut le dire ici parce que
/// l'intuition dit le contraire :** aucun gain de plan d'exécution. Le plan de
/// mika#2638 affirmait qu'un `LIKE 'préfixe%'` ne peut pas utiliser
/// `idx_tasks_manual_active_ref_url` là où un `IN (…)` peut ; c'est **faux**, et
/// mesuré tel quel (SQLite 3.53, `EXPLAIN QUERY PLAN` sur le DDL réel) : les
/// deux formes rendent le même plan, `SEARCH tasks USING INDEX
/// idx_tasks_agent_status` plus un B-tree temporaire. L'index partiel est en
/// fait **inatteignable** pour ces requêtes — sa clause exige
/// `trigger_type = 'manual'` et `status NOT IN ('completed', …)`, qu'aucune
/// d'elles ne porte, et un `status IN ('pending','in_progress')` n'implique pas
/// syntaxiquement le `NOT IN`. Laisser l'affirmation debout enverrait la
/// prochaine personne qui règle ce chemin dans un cul-de-sac.
///
/// # Un site de définition, cinq appelants
///
/// Les cinq requêtes qui posent une question sur l'issue d'une tâche passent
/// par ici :
///
/// | site | question |
/// |---|---|
/// | `Database::find_active_self_dev_task_for_issue` | « une tâche self_dev active référence-t-elle ce ticket ? » — `auto_pull` Phase 0/2 |
/// | `Database::find_dispatch_children_for_issue_url` | « un pilote est-il vif pour ce ticket ? » — lecteur unique de [`crate::live_pilot`] (mika#2279) |
/// | `Database::find_active_tracking_rows_by_reference_url_and_variants` | « quelles lignes de suivi nettoyer ? » (mika#1934) |
/// | `Database::has_completed_groom_for_issue` | « ce ticket a-t-il un groom convergé ? » (mika#1620 / mika#2287) |
/// | `Database::latest_groom_verdict_for_issue` | « quel est son dernier verdict de groom ? » |
///
/// Les **trois derniers étaient déjà délimités** et sont réécrits à travers ce
/// helper **sans changement de comportement** : leur sortie est octet pour
/// octet la même. Le but est que les cinq ne puissent plus diverger, pas de
/// modifier ceux qui étaient justes — et les deux derniers épelaient l'ensemble
/// à la main, c'est-à-dire étaient exactement la forme que le détecteur D5 de
/// mika#2638 existe pour refuser.
///
/// Cinq appelants pour une définition : c'est ce qui empêche deux requêtes
/// d'épeler différemment l'ensemble des variantes — la leçon que
/// [`crate::grooming_marker`] a dû graver une fois (mika#2158, où promotion et
/// routage répondaient différemment à la même question pendant des mois sans
/// que rien ne casse) et que `live_pilot` a payée une seconde fois (mika#2335).
///
/// # L'ensemble est CLOS, et le rétrécissement est nommé
///
/// Passer du préfixe à l'énumération **rétrécit** : une ligne portant
/// `…/issues/2638/` ou `…/issues/2638#issuecomment-1` était vue en vol hier et
/// ne l'est plus.
///
/// **Ce que cette borne dit exactement**, parce que la formulation courte est
/// fausse et qu'une revue l'a relevée : une telle ligne n'est **pas** « hors de
/// `idx_tasks_manual_active_ref_url` » — cet index est
/// `UNIQUE(agent_id, reference_url)` sur un prédicat partiel, donc la ligne y
/// entre sous **sa propre** clé. Ce qui est vrai, et qui est la borne, est
/// qu'elle **ne dédoublonne pas** contre la ligne canonique : les deux
/// coexistent comme deux tâches actives pour la même issue. C'est donc déjà un
/// défaut en amont, et plus grave que celui-ci.
///
/// **Le rétrécissement n'est pas purement théorique, et son écrivain est
/// nommé.** Tous les écrivains **moteur** composent l'URL par `format!` à
/// partir de littéraux et de `owner/repo`, donc ne produisent que les deux
/// variantes. L'outil `create_task`, lui, prend `reference_url` de l'entrée du
/// modèle avec un `.trim()` pour seule normalisation : une forme non prescrite
/// écrite par là sort de la population des sondes, et une tâche en vol
/// redevient invisible — c'est-à-dire la porte 2c de mika#2279 qui laisse
/// repartir un dispatch. Population **non mesurée** (le prompt ne prescrit que
/// les deux variantes déclarées), remède hors périmètre de mika#2638 : la
/// canonicalisation appartient au **point d'écriture**, pas au prédicat de
/// lecture. Précondition du suivi : la sonde opérateur § 8 S1 de
/// `docs/plans/2026-10-02-002-fix-2638-reference-url-numero-delimite-plan.md`,
/// qui dit si l'ensemble est clos en production.
pub fn issue_url_variants(base_url: &str) -> [String; 2] {
    // Le helper NORMALISE au lieu d'exiger. `strip_groom_phase_suffix` est
    // idempotent, donc les deux appelants qui nettoient déjà en amont rendent
    // exactement la même paire qu'avant — et un futur sixième appelant qui
    // passerait une `reference_url` lue en base sans la nettoyer obtiendrait
    // sinon `…?phase=groom` et `…?phase=groom?phase=groom`, c'est-à-dire une
    // paire qui n'apparie **rien** : un silence, la pire des trois issues.
    let base_url = strip_groom_phase_suffix(base_url);
    [
        base_url.to_string(),
        format!("{base_url}{GROOM_PHASE_SUFFIX}"),
    ]
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub timeout_at: Option<String>,
    pub action_type: String,
    pub action_config: String,
    pub status: String,
    pub process_id: Option<i64>,
    pub input_context: Option<String>,
    pub result: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub fired_at: Option<String>,
    pub completed_at: Option<String>,
    pub reference_url: Option<String>,
    pub source: Option<String>,
    pub metadata: Option<String>,
    /// Task kind: `"issue"`, `"milestone"`, or `"project"`. NOT NULL in the DB,
    /// defaulted to `"issue"` for backward compatibility (added in schema v23). See
    /// [`VALID_TASK_TYPES`].
    pub r#type: String,
    /// Dispatch class for long-running tasks: `"implement"` or `"groom"`. Nullable —
    /// pre-v34 rows have `NULL` (treated as `"implement"` by the per-class dispatch
    /// guard via `COALESCE`). Set on callback task creation based on the dispatched
    /// skill (#1001).
    pub dispatch_class: Option<String>,
    /// Which dispatcher inside this engine initiated the task: `"mika_dev"`,
    /// `"mika_manager"`, or `"operator"` (mika#1948, Porte 2). Nullable — pre-v51
    /// rows have `NULL` and are read as `"mika_dev"` via `COALESCE`, since the
    /// autonomous loop was the only dispatcher before the column existed.
    ///
    /// Distinct from the `dispatch:*` SEAT label (mika#2084), which says which
    /// *engine* owns a ticket. This says which *role inside one engine* asked
    /// for the work.
    pub dispatcher_source: Option<String>,
}

/// Result of a force-promote attempt on a deferred dispatch wrapper (mika#1453).
/// Used by both the CLI verb and agent tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForcePromoteResult {
    /// The next pending deferred wrapper was promoted for dispatch.
    Promoted { task_id: String },
    /// The per-class dispatch slot is occupied by a non-deferred callback;
    /// promotion is refused (fail-closed). The `blocking_label` identifies
    /// the occupying task for operator diagnostics.
    RejectedSlotBusy { blocking_label: String },
    /// No pending deferred wrapper exists for the given dispatch class.
    NoPendingWrapper,
}

/// Split counts of active background callback tasks. `executing` = subprocess alive
/// (`process_id IS NOT NULL`), `queued` = waiting for dispatch slot (`process_id IS NULL`).
/// Used by TUI footer badge to distinguish `[1 running, 2 queued]` (#1057).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTaskCounts {
    pub executing: usize,
    pub queued: usize,
}

/// A parent self_dev task left `in_progress` after its callback subtask
/// delivered without producing a PR. Used by the task engine reaper (#871).
#[derive(Debug, Clone)]
pub struct OrphanedParentTask {
    pub id: String,
    pub agent_id: String,
    pub callback_task_id: String,
    pub created_at: String,
}

/// A `manual` tracking row whose dispatch is over: every callback child has
/// reached a terminal status and the last of them stopped moving longer ago
/// than the grace window. Returned by
/// `Database::find_settleable_dispatch_parents` and consumed by the dispatch
/// parent settler (mika#2405).
///
/// `last_child_at` is `MAX(child.updated_at)` — the moment the *last* child
/// moved, never the parent's own `created_at`: a parent reused across
/// dispatches (mika#920) is old by construction. `child_count` is carried for
/// the operator line only; no decision reads it.
#[derive(Debug, Clone)]
pub struct SettleableDispatchParent {
    pub id: String,
    pub agent_id: String,
    pub created_at: String,
    pub last_child_at: String,
    pub child_count: i64,
}

/// A phantom tracking task row: `action_type='none'`, `process_id IS NULL`,
/// `status IN ('in_progress','blocked')`, aged past the sweep grace window.
/// Used by the NULL-PID phantom sweep (mika#1712) in both AC3 (watchdog tick)
/// and AC5 (startup sweep) paths.
#[derive(Debug, Clone)]
pub struct PhantomTrackingTask {
    pub id: String,
    pub agent_id: String,
    pub label: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A dispatch child row of a tracking task: a `parent_task_id`-linked task
/// carrying a non-NULL `process_id`. Returned by
/// `Database::find_dispatch_children_with_pid` and consumed by the phantom
/// sweep liveness guard (mika#2156).
///
/// `process_start_time` is field 22 of `/proc/<pid>/stat`, lifted out of the
/// task's `metadata` JSON where the executor stores it as a string
/// (`skills/executor.rs`). `None` means the row predates the metadata write,
/// carries malformed JSON, or ran on a non-Linux host — in which case the
/// guard cannot rule out PID reuse and deliberately falls back to sweeping
/// (plan mika#2156 D-3).
#[derive(Debug, Clone)]
pub struct DispatchChild {
    pub id: String,
    pub process_id: i64,
    pub process_start_time: Option<u64>,
    /// The child's own status (mika#2335). The query still does **not** filter
    /// on it — mika#2156's D-2 reasoning is unchanged, liveness is the
    /// discriminator and the caller applies it. This field exists so a second
    /// caller can apply a *different* rule without a second resolver: the
    /// supersede disposal skips a terminal child (`delivered`, `cancelled`, …)
    /// because there is no pilot left to kill and its `process_id` is a stale
    /// pgid. One join predicate, two filtering decisions, both at their caller.
    pub status: String,
}

/// A dispatch child reached from the **issue URL** rather than from a known
/// parent id, with the parent that carries that URL named alongside it
/// (mika#2279).
///
/// The two fields exist because a dispatch is two rows and neither one alone
/// answers the question: the **parent** carries `reference_url` and never a
/// `process_id`, the **child** carries the pgid and never a URL. A caller
/// asking *"is a pilot alive for this ticket?"* starts from the URL and must be
/// told both — the child so it can probe liveness, the parent so the refusal it
/// writes names the row an operator would cancel.
#[derive(Debug, Clone)]
pub struct IssueDispatchChild {
    /// The tracking row carrying the issue URL. Its **status is deliberately
    /// not constrained** by the query: a `cancelled` parent is precisely the
    /// state mika#2279 exists to see.
    pub parent_task_id: String,
    pub child: DispatchChild,
}

/// The oldest active `self_dev` task referencing one issue (mika#2161 AC3).
///
/// Returned by `Database::find_active_self_dev_task_for_issue`, whose boolean
/// sibling `has_active_self_dev_task_for_issue` is derived from it so one SQL
/// site answers the question — two queries free to diverge is the
/// `grooming_marker` lesson (mika#2158), paid once already in this very module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightSelfDevTask {
    pub task_id: String,
    /// `pending` or `in_progress`. Carried because it says **which clock
    /// started**: a `pending` row has not been dispatched, so its age is measured
    /// from its *creation*, which is a different fact from "dispatched N minutes
    /// ago" and must not be reported as one.
    pub status: String,
    /// `COALESCE(fired_at, created_at)` — the first instant the engine began
    /// working under this row, falling back to its creation when it has not
    /// fired.
    ///
    /// The fallback is load-bearing rather than defensive: `fired_at` was
    /// **never stamped on the parent row** before mika#2335, so an operator
    /// reading a live dispatch's parent read "never fired" (the 2026-09-15
    /// incident). Reporting an age from `created_at` and *saying so* through
    /// [`Self::status`] is honest; reporting nothing would make AC3
    /// unanswerable on every pre-mika#2335 row.
    pub in_flight_since: String,
}

/// Statuses on which a task no longer has a pilot to kill (mika#2335).
///
/// Deliberately a positive list of terminal states rather than `!= pending &&
/// != in_progress`: an unknown status must read as *not terminal*, so a state
/// added later is still disposed of rather than silently spared.
///
/// Lives here, beside [`DispatchChild`], because every caller that filters a
/// dispatch child on it — the supersession disposal (`tracking_cleanup`), the
/// operator cancel path (`task_engine::process_kill`) and the live-pilot
/// predicate (`live_pilot`, mika#2279) — must agree. A second copy of this list
/// is the shape of defect this ticket exists to remove, which is also why
/// `find_dispatch_children_for_issue_url` does **not** spell the terminal
/// statuses into its SQL: that would be a third copy, in a dialect where the
/// "unknown status is not terminal" rule above cannot be read.
pub fn is_terminal_task_status(status: &str) -> bool {
    use crate::task_engine::types::task_status;
    matches!(
        status,
        task_status::DELIVERED
            | task_status::COMPLETED
            | task_status::CANCELLED
            | task_status::FAILED
            | task_status::EXPIRED
    )
}

/// A parent self_dev task left `in_progress` after its callback subtask
/// delivered WITH a `pr_url` (success indicator). Used by the success-side
/// engine backstop (mika#1162) — sibling shape to `OrphanedParentTask`.
/// Returned by `find_completable_parent_tasks_on_pr_url`.
///
/// Why this is a separate type from `OrphanedParentTask` despite near-identical
/// fields: the `pr_url` field is included in the SELECT so the completer can
/// build its audit-event reason string without a second DB round-trip (see plan
/// docs/plans/2026-05-17-001-fix-1162-...md, decision D1).
#[derive(Debug, Clone)]
pub struct CompletableParentTask {
    pub id: String,
    pub agent_id: String,
    pub callback_task_id: String,
    pub created_at: String,
    /// pr_url extracted from `parent.metadata.claude_pilot.pr_url`. Embedded
    /// in the SELECT so the completer doesn't need a second round-trip to
    /// build its audit-event reason string.
    pub pr_url: String,
}

/// A parent self_dev issue task left `in_progress` with **zero** callback
/// children, aged past the childless-parent reaper grace window (mika#1687).
///
/// This is the zero-child complement of [`OrphanedParentTask`]: the orphan
/// reaper and parent-completer both INNER-JOIN a delivered callback child, so a
/// parent that reached `in_progress` without ever spawning a callback child
/// (silent pilot death — dispatch reached `in_progress` but no callback row was
/// recorded) falls through both. `find_childless_stuck_parent_tasks` selects
/// exactly that shape via `NOT EXISTS (SELECT 1 FROM tasks child …)`.
///
/// No `callback_task_id` field: by construction there is no child to reference.
#[derive(Debug, Clone)]
pub struct ChildlessStuckParent {
    pub id: String,
    pub agent_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A `pending` self_dev issue parent that nothing in the dispatch queue
/// represents any more (mika#2045).
///
/// The `ready-label` path pre-creates this parent, then registers a deferred
/// wrapper child when the per-class dispatch slot is busy. Promotion consumes
/// that wrapper destructively (`promote_next_deferred_callback` sets it
/// `completed`), so a wrapper whose silent turn never dispatched leaves the
/// parent `pending` with nothing representing it — and the partial unique index
/// `idx_tasks_manual_active_ref_url` then forbids a replacement from being
/// created for the same issue. The parent is *orphaned*.
///
/// Age alone does not identify this shape: a parent waiting behind a busy slot
/// is also old and still has its wrapper. `find_orphaned_pending_issue_tasks`
/// therefore requires BOTH the age and the absence of any callback child that
/// still represents the task.
#[derive(Debug, Clone)]
pub struct OrphanedPendingTask {
    pub id: String,
    pub reference_url: String,
    pub created_at: String,
    pub age_seconds: i64,
    /// Repairs already attempted for this parent, read from
    /// `metadata.stuck_rearm_count`. Absent or unreadable metadata reads as 0.
    pub rearm_count: i64,
    /// The parent's own dispatch class, `implement` when NULL (matching
    /// `has_active_callback_tasks_excluding`). A repair must re-enter the class
    /// the task actually belongs to: re-arming an ungroomed issue as
    /// `implement` would queue a `dev-pilot` run for work that still needs
    /// `dev-groom`, and would occupy the wrong slot doing it.
    pub dispatch_class: String,
}

/// One deferred wrapper of a parent, as the stuck-pending reaper saw it at
/// decision time (mika#2181 AC4).
///
/// The reaper's verdict is "nothing represents this parent". That verdict is
/// unreadable after the fact unless the audit says *which* wrappers existed and
/// *what statuses* produced it — otherwise the next battle starts by rebuilding
/// the query from the code, which is what mika#2181 cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredWrapperSummary {
    pub id: String,
    pub status: String,
    pub completed_at: Option<String>,
}

impl DeferredWrapperSummary {
    /// Compact one-field rendering for an audit `details` string and a `tracing`
    /// field (mika#2181 AC4):
    /// `wrappers:f5eebf48:completed@2026-09-04T15:31:03Z,284b0ffe:pending@-`,
    /// or `wrappers:none` when the parent has no wrapper at all.
    ///
    /// Ids are truncated to 8 chars because this line is read next to
    /// `server.log`, where the dispatcher already prints them short.
    pub fn render(wrappers: &[DeferredWrapperSummary]) -> String {
        if wrappers.is_empty() {
            return "wrappers:none".to_string();
        }
        let body = wrappers
            .iter()
            .map(|w| {
                let short: String = w.id.chars().take(8).collect();
                let at = w.completed_at.as_deref().unwrap_or("-");
                format!("{short}:{}@{at}", w.status)
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("wrappers:{body}")
    }
}

/// A `blocked` self_dev issue parent refused on a busy dispatch slot, whose
/// wrapper never reached consumption (mika#2169, L3b).
///
/// Sibling of [`OrphanedPendingTask`], and deliberately a **separate**
/// population rather than a widening of it. `find_orphaned_pending_issue_tasks`
/// keys on `parent.status = 'pending'`; relaxing that clause to
/// `IN ('pending','blocked')` would sweep in the deliberate operator gates —
/// `blocked` is also what an auto-merge refusal and a QA escalation write. The
/// discriminant is `result.$.error = 'global_dispatch_active'`: only the
/// slot-refusal path writes it, so the two queries stay disjoint and each stays
/// readable on its own.
#[derive(Debug, Clone)]
pub struct StaleBlockedTask {
    pub id: String,
    pub reference_url: String,
    pub created_at: String,
    pub age_seconds: i64,
    /// Repairs already attempted for this parent, read from
    /// `metadata.stuck_rearm_count`. Absent or unreadable metadata reads as 0.
    pub rearm_count: i64,
    pub dispatch_class: String,
    /// The callback the refusal named as holding the slot, read from
    /// `result.$.blocking_callback_id`. `None` when the field is absent or the
    /// result is not valid JSON — the sweep then treats the blocker as gone,
    /// which is what a vanished row means.
    pub blocking_callback_id: Option<String>,
}

/// Snapshot of a child task for the orphaned-parent reaper's structured log
/// event (`task_engine_reaper.evaluated`). Captures all children of a candidate
/// parent at kill time for post-incident diagnosis (mika#1126).
#[derive(Debug, Clone)]
pub struct ReaperChildSnapshot {
    pub id: String,
    pub dispatch_class: Option<String>,
    pub status: String,
    pub trigger_type: String,
    pub action_type: String,
    pub updated_at: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub timeout_at: Option<String>,
    pub action_type: String,
    pub action_config: String,
    pub input_context: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub reference_url: Option<String>,
    pub source: Option<String>,
    pub metadata: Option<String>,
    /// Task kind. `None` (or an empty string) means "use the SQL default"
    /// (`"issue"`), preserving backward compatibility for existing callers. Values
    /// other than those in [`VALID_TASK_TYPES`] are rejected by the DB CHECK
    /// constraint; prefer validating at the tool boundary before INSERT.
    pub r#type: Option<String>,
    /// Dispatch class for per-class slot split (#1001). `None` means NULL in
    /// the DB (treated as `"implement"` via `COALESCE` by the dispatch guard).
    /// Set to `Some("groom")` for grooming dispatches.
    pub dispatch_class: Option<String>,
}

/// A single anomalous task state detected by the health check.
#[derive(Debug, Clone)]
pub struct TaskHealthAnomaly {
    pub task_id: String,
    pub label: String,
    pub trigger_type: String,
    pub status: String,
    /// One of: "stuck_callback", "stale_blocked", "failed_recurring", "long_running", "github_linked", "dispatch_failures", "dispatch_stale"
    pub anomaly_type: String,
    /// Human-readable age description (e.g., "3h 22m", "5 days").
    pub age_description: String,
    pub reference_url: Option<String>,
}

/// Aggregated task health summary for heartbeat prompt injection.
#[derive(Debug, Clone, Default)]
pub struct TaskHealthSummary {
    /// Active manual tasks (pending/in_progress/blocked).
    pub active_tasks: Vec<Task>,
    /// Anomalous task states across all trigger types, capped at [`health_thresholds::MAX_ANOMALIES`].
    pub anomalies: Vec<TaskHealthAnomaly>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // mika#2638 — le helper aux bornes, et l'ORDRE de ses deux fentes.
    //
    // Les cinq consommateurs passent la paire dans un `reference_url IN (?2, ?3)`,
    // qui est **indifférent à l'ordre**. Donc inverser `[base, base+suffixe]`
    // laisserait les dix tests de mika#2638 verts pendant que les cinq
    // destructurations `let [exact, groom] = …` nomment chacune la mauvaise
    // valeur — un renommage silencieux qu'aucun test de consommateur ne peut
    // voir (relevé en revue). Ces deux tests sont le seul endroit où l'ordre
    // est observable.
    // ---------------------------------------------------------------------

    #[test]
    fn mika2638_la_premiere_fente_est_lurl_exacte_la_seconde_la_variante_groom() {
        let [exact, groom] = issue_url_variants("https://github.com/o/r/issues/42");
        assert_eq!(exact, "https://github.com/o/r/issues/42");
        assert_eq!(groom, "https://github.com/o/r/issues/42?phase=groom");
    }

    /// Le helper NORMALISE au lieu d'exiger : une URL portant déjà le suffixe
    /// rend la même paire qu'une URL canonique. Sans ça, un futur appelant qui
    /// passerait une `reference_url` lue en base obtiendrait une paire
    /// n'appariant rien — un silence, pas une erreur.
    #[test]
    fn mika2638_le_helper_est_idempotent_sur_le_suffixe_groom() {
        let base = "https://github.com/o/r/issues/42";
        assert_eq!(
            issue_url_variants(&format!("{base}{GROOM_PHASE_SUFFIX}")),
            issue_url_variants(base),
            "un `?phase=groom` résiduel doit être retiré par le helper"
        );
    }

    // ---------------------------------------------------------------------
    // mika#2590 U8c — la fonction pure aux bornes.
    //
    // Testée ici plutôt qu'uniquement à travers la base : les trois refus de
    // `groom_result_convergence` appellent tous la même disposition côté porte,
    // donc un test qui n'observe que « le dispatch est refusé » ne distingue pas
    // un motif d'un autre — et distinguer les deux motifs est très exactement ce
    // que R6 existe pour permettre (D2). Sans ces cas, `marker_not_line_anchored`
    // pourrait n'avoir aucune population et rien ne le dirait.
    // ---------------------------------------------------------------------

    /// Le RESULT d'un groom réellement convergé, tel que `_set_outcome_line`
    /// le pose depuis U5a : le marqueur en début de ligne.
    const CONVERGED: &str =
        "claude-pilot completed (status: done).\nSession: sess-2590\nOutcome: PLAN_GROOMED";

    #[test]
    fn mika2590_une_ligne_ancree_est_une_convergence() {
        assert_eq!(
            groom_result_convergence(CONVERGED),
            GroomConvergence::Converged
        );
    }

    /// Borne basse : le marqueur **seul**, sans rien autour — la forme la plus
    /// courte qu'un producteur puisse écrire, et celle où « début de ligne » et
    /// « début de texte » coïncident.
    #[test]
    fn mika2590_le_marqueur_seul_est_une_convergence() {
        assert_eq!(
            groom_result_convergence(GROOM_SUCCESS_MARKER),
            GroomConvergence::Converged
        );
    }

    /// **Le défaut fondateur, forme minimale.** Une enveloppe JSON portant
    /// `status` dont la prose cite le marqueur pour expliquer qu'aucune preuve
    /// n'est frappée.
    #[test]
    fn mika2590_une_enveloppe_json_citant_le_marqueur_est_refusee() {
        let auto_skipped = format!(
            r#"{{"status":"auto_skipped","reason":"already_groomed","note":"the provenance gate refuses it unless a completed groom callback carrying {} exists, and this skip mints none."}}"#,
            GROOM_SUCCESS_MARKER
        );
        assert_eq!(
            groom_result_convergence(&auto_skipped),
            GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE),
            "le texte qui dit « ceci n'est pas une preuve » ne peut pas être la \
             preuve — défaut mesuré sur mika#2105 le 2026-09-29, où \
             `instr(result, marker)` rendait 651"
        );
    }

    /// **Le motif n'est posé que s'il y a quelque chose à écarter.** Un saut
    /// ordinaire (`issue_closed`, mika#988) ne cite pas le marqueur : le compter
    /// sous `json_envelope` polluerait le compteur de R6 avec une population qui
    /// n'a jamais menacé la porte.
    #[test]
    fn mika2590_une_enveloppe_json_sans_marqueur_est_absente_et_non_un_motif() {
        assert_eq!(
            groom_result_convergence(r#"{"status":"auto_skipped","reason":"issue_closed"}"#),
            GroomConvergence::Absent
        );
    }

    /// **Le cas frontière qui compte** (mika#2050) : la prose d'un pilote qui
    /// *discute* du mécanisme, hors JSON, marqueur en milieu de ligne. C'est la
    /// seule population de `marker_not_line_anchored`, dont la doc opérateur
    /// annonce un régime « non vide, décroissant » — sans ce test, ce motif
    /// pourrait être mort sans que rien ne le dise.
    #[test]
    fn mika2590_un_marqueur_en_milieu_de_ligne_hors_json_est_refuse() {
        let prose = format!(
            "claude-pilot completed (status: done).\n\
             I checked whether a callback carrying {} exists, and none does.\n\
             Session: sess-2590",
            GROOM_SUCCESS_MARKER
        );
        assert_eq!(
            groom_result_convergence(&prose),
            GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_NOT_LINE_ANCHORED),
            "un pilote qui DISCUTE du jeton ne l'a pas émis — la classe que \
             mika#2050 a mesurée sur le Signal S"
        );
    }

    #[test]
    fn mika2590_un_result_sans_marqueur_est_absent() {
        assert_eq!(
            groom_result_convergence(
                "claude-pilot completed (status: done).\nOutcome: PLAN_ITERATE"
            ),
            GroomConvergence::Absent
        );
        assert_eq!(groom_result_convergence(""), GroomConvergence::Absent);
    }

    /// Un JSON qui n'est pas un **objet portant `status`** n'est pas une
    /// enveloppe de saut : il retombe sur la lecture par ligne. Sans ce
    /// contrôle, « l'enveloppe est refusée » serait indistinguable de « tout ce
    /// qui parse comme du JSON est refusé ».
    #[test]
    fn mika2590_un_json_sans_champ_status_retombe_sur_la_lecture_par_ligne() {
        assert_eq!(
            groom_result_convergence(r#"{"reason":"already_groomed"}"#),
            GroomConvergence::Absent
        );
    }

    // --- l'agrégat, et sa préséance ---

    /// La sémantique historique est `COUNT(*) > 0` : **une** preuve valide
    /// suffit, et l'agrégat ne doit pas se laisser dégrader par un saut qui la
    /// précède dans l'ordre de balayage.
    #[test]
    fn mika2590_une_seule_preuve_valide_suffit_quel_que_soit_lordre() {
        let rejected = GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE);
        for order in [
            vec![rejected.clone(), GroomConvergence::Converged],
            vec![GroomConvergence::Converged, rejected.clone()],
            vec![
                GroomConvergence::Absent,
                rejected.clone(),
                GroomConvergence::Converged,
            ],
        ] {
            assert_eq!(
                aggregate_groom_convergence(order),
                GroomConvergence::Converged
            );
        }
    }

    /// À défaut de preuve, un écart **mesuré** l'emporte sur l'absence — sinon
    /// un ticket refoulé pour preuve polluée se lirait exactement comme un
    /// ticket jamais groomé (la classe mika#2205 appliquée au correctif).
    #[test]
    fn mika2590_a_defaut_de_preuve_un_ecart_mesure_lemporte_sur_labsence() {
        assert_eq!(
            aggregate_groom_convergence(vec![
                GroomConvergence::Absent,
                GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_NOT_LINE_ANCHORED),
                GroomConvergence::Absent,
            ]),
            GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_NOT_LINE_ANCHORED)
        );
        assert_eq!(
            aggregate_groom_convergence(vec![]),
            GroomConvergence::Absent,
            "aucune ligne de callback est le régime nominal d'un premier \
             grooming, jamais un écart"
        );
    }

    /// `is_converged` ne rend vrai que sur `Converged` — le contrôle qui empêche
    /// qu'un futur éditeur en fasse « tout sauf le motif que je connais ».
    #[test]
    fn mika2590_is_converged_nest_vrai_que_sur_converged() {
        assert!(GroomConvergence::Converged.is_converged());
        assert!(!GroomConvergence::Absent.is_converged());
        for reason in ALL_GROOM_CONVERGENCE_REJECTIONS {
            assert!(!GroomConvergence::MarkerOutOfPosition(reason).is_converged());
        }
    }

    /// Les deux motifs sont un **format de fil** : ils atterrissent dans
    /// `audit_events.after_value` et l'opérateur en fait des `GROUP BY`. Deux
    /// orthographes d'un même motif couperaient une population en deux sans le
    /// dire.
    #[test]
    fn mika2590_les_motifs_sont_un_format_de_fil() {
        assert_eq!(GROOM_REJECTED_JSON_ENVELOPE, "json_envelope");
        assert_eq!(GROOM_REJECTED_NOT_LINE_ANCHORED, "marker_not_line_anchored");
        assert_eq!(ALL_GROOM_CONVERGENCE_REJECTIONS.len(), 2);
    }
}

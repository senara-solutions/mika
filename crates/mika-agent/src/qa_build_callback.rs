//! Le discriminant du callback de build QA, écrit **une seule fois** (mika#2355).
//!
//! # Le défaut que ce module ferme
//!
//! Le 2026-09-17, trois revues mika-qa (`7309c48c`/#2352, `52780caa`/#2353,
//! `1ea9f92c`/#2350) ont rendu « Build succeeded » et **rien d'autre** : zéro
//! `VERDICT:`, zéro commentaire sur la PR. Le tour de revue lance `build_mika`
//! en asynchrone et termine son tour pour attendre le callback — ça, c'est
//! voulu. Ce qui ne l'était pas : sur le tour de reprise, le moteur prescrit
//! **le contrat terminal d'un autre flux**.
//!
//! `build_callback_trigger_context` injectait « This turn MUST end with both
//! `update_task_status` and `send_message` », et la garde `callback_terminal_action`
//! (#870) ne relâchait le tour que sur ces deux outils — un contrat écrit pour
//! le dispatch d'un pilote self_dev, à une époque où son commentaire pouvait
//! encore affirmer *« only one callback flow exists today »*. `build_mika`
//! (`long_running: true`) en est le second, et rien dans le source ne l'a dit.
//! Un tour qui répondait « Build succeeded » par `send_message` satisfaisait donc
//! le moteur, et **aucune garde n'exigeait `run_gh pr review`**.
//!
//! # Pourquoi un module, et pas un littéral à chaque site
//!
//! Trois lecteurs du moteur posent la même question : le framing
//! (`agent_loop::build_callback_trigger_context`, via [`is_build_callback_label`]),
//! la garde négative (`agent_loop::callback_trigger_active`, via
//! [`is_build_callback`]) et la garde positive (`qa_build_callback_verdict`,
//! inline dans `agent_loop::run_loop`, via [`qa_verdict_required`] +
//! [`pr_review_posted_in_turn`]). Une grammaire de fil recopiée entre trois
//! lecteurs est exactement la classe que mika#2158 a dû refermer une fois —
//! deux regex de grooming qui répondaient différemment à la même question
//! pendant des mois sans que rien ne casse. Le quatrième lecteur est du
//! **prompt** (`qa-review-build-callback/system_prompt.md`) et ne peut pas
//! partager une constante Rust : `tests::mika2355_the_scope_header_quotes_the_engine_marker`
//! épingle que la chaîne qu'il cite est bien celle que le moteur émet.
//!
//! Le filet moteur qui poste `hold[review]` quand le re-prompt lui-même échoue
//! est arrivé sous mika#2368. Il n'est pas dans ce module — il généralise
//! `server::deadline_verdict` et se câble dans `task_engine::dispatcher` — mais
//! son **prédicat de déclenchement** l'est : [`verdict_unmet_after_retry`], le
//! complémentaire exact de la garde, lu par les deux sites de sortie EndTurn.
//! Il lit [`pr_review_posted_in_turn`] et rien d'autre.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::tool_execution::ToolCallSummary;

/// Nom de l'outil long-running exposé par le skill `build-mika`.
///
/// Source de vérité : `skills/bundled/build-mika/tools.json`.
pub const BUILD_MIKA_TOOL: &str = "build_mika";

/// Le skill dont la présence au tour rend un verdict **dû**.
pub const QA_REVIEW_SKILL: &str = "qa-review";

/// Le label de tâche d'un callback de build, tel que
/// `skills::executor::build_callback_task` le forme
/// (`format!("long_running:{tool_name}")`).
///
/// C'est la clé que lit le framing (`build_callback_trigger_context` reçoit le
/// label, pas le message) ; le marqueur de message ci-dessous en est l'enveloppe.
pub const BUILD_CALLBACK_LABEL: &str = "long_running:build_mika";

/// Le marqueur de tour qu'émet `run_silent_agent` pour un callback de build.
///
/// Forme composée de deux moitiés qui vivent ailleurs et qu'on ne peut pas
/// importer : `format!("[callback: {label}]")` dans `run_silent_agent`, et
/// [`BUILD_CALLBACK_LABEL`]. Le test
/// [`tests::mika2355_the_marker_is_the_shape_the_engine_actually_emits`]
/// reconstruit les deux `format!` et compare — un changement de l'une ou
/// l'autre grammaire rougit ici plutôt que de désarmer les gardes en silence.
pub const BUILD_CALLBACK_MESSAGE_MARKER: &str = "[callback: long_running:build_mika]";

/// Ce label de callback est-il celui d'un build ?
///
/// Égalité stricte : `long_running:build_mika_foo` n'existe pas, et un
/// `starts_with` ici lirait un futur outil homonyme comme un build.
pub fn is_build_callback_label(label: &str) -> bool {
    label == BUILD_CALLBACK_LABEL
}

/// Le message de ce tour est-il un callback de build ?
///
/// `starts_with` et non `contains` : `run_silent_agent` peut suffixer le
/// marqueur `[milestone-parent: …]`, jamais préfixer quoi que ce soit.
pub fn is_build_callback(msg: &str) -> bool {
    msg.starts_with(BUILD_CALLBACK_MESSAGE_MARKER)
}

/// Un verdict est-il **dû** sur ce tour ?
///
/// Conjonction, et les deux moitiés comptent. Le label porte le nom de l'outil,
/// jamais celui de l'agent : `mika-dev` porte `build-mika` dans son allowlist
/// (`well_known_agents.rs`) et lance des builds qui ne doivent aucun verdict à
/// personne. Une garde armée sur le seul label re-prompterait mika-dev pour
/// poster une revue de PR — elle échangerait le loop-breaker QA contre un
/// loop-breaker dev.
pub fn qa_verdict_required(
    msg: &str,
    skill_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> bool {
    is_build_callback(msg)
        && skill_names
            .into_iter()
            .any(|n| n.as_ref().eq_ignore_ascii_case(QA_REVIEW_SKILL))
}

/// Un `run_gh pr review` a-t-il **réussi** dans ce tour ?
///
/// Le prédicat de satisfaction de la garde positive `qa_build_callback_verdict`
/// (`agent_loop::run_loop`, les deux chemins de sortie EndTurn). Miroir exact de
/// `agent_loop::has_successful_pr_review` (early-accept #695/#821), délibérément
/// réécrit ici plutôt qu'appelé : cette fonction-là est privée à `agent_loop` et
/// la rendre publique pour le dispatcher exporterait un détail de la chaîne de
/// post-conditions. `tests::mika2355_the_satisfied_predicate_matches_the_early_accept_one`
/// épingle que les deux répondent pareil sur les formes qui comptent.
pub fn pr_review_posted_in_turn(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| {
        s.name == "run_gh"
            && s.success
            && s.input_summary.contains("\"pr\"")
            && s.input_summary.contains("\"review\"")
    })
}

/// Label de la garde positive, pour `intent_guard_retries`.
pub const QA_VERDICT_REQUIRED_LABEL: &str = "qa_build_callback_verdict";

/// Ce tour devait un verdict, son budget de re-prompt est épuisé, et rien n'a
/// été posté (mika#2368 C4).
///
/// C'est le **complémentaire exact** de la garde `qa_build_callback_verdict` :
/// même conjonction, le terme de budget inversé. Là où la garde re-prompte
/// (budget non consommé), ceci arme le filet (budget consommé).
///
/// # Pourquoi une fonction et pas trois termes recopiés
///
/// Le site où le budget est « déjà consommé » **n'existe pas comme branche** :
/// la condition `!intent_guard_retries.contains(…)` est *dans* le `if` du
/// re-prompt, et budget épuisé on tombe à travers, sans `else`. Le prédicat se
/// pose donc sur les deux **chemins de sortie** EndTurn — texte non vide et
/// miroir texte vide — et deux copies d'une conjonction à trois termes
/// divergent : la leçon que `grooming_marker` a dû engraver une fois
/// (mika#2158). Les deux sites de la garde elle-même sont déjà une duplication
/// qu'on n'aggrave pas.
///
/// # Les deux sites comptent, et le second est le plus probable
///
/// Le commentaire du miroir texte vide le dit en propres termes : *« a bare
/// EndTurn is exactly the shape a turn that has nothing to say takes, and it is
/// the one the registry never sees »*. Un tour de callback qui conclut sans
/// rien dire **est** le cas nominal de mika#2368. Un signal posé au seul site
/// texte-non-vide laisserait le filet aveugle sur la moitié la plus probable de
/// sa population — et rien ne le signalerait : le filet resterait silencieux,
/// ce qui est indistinguable d'un filet qui n'a rien à faire.
pub fn verdict_unmet_after_retry(
    qa_verdict_due: bool,
    intent_guard_retries: &HashSet<&'static str>,
    summaries: &[ToolCallSummary],
) -> bool {
    qa_verdict_due
        && intent_guard_retries.contains(QA_VERDICT_REQUIRED_LABEL)
        && !pr_review_posted_in_turn(summaries)
}

/// Un verdict était dû sur ce tour **coupé**, et rien n'a été posté (mika#2515).
///
/// Frère de [`verdict_unmet_after_retry`], **moins** le terme
/// `intent_guard_retries.contains(QA_VERDICT_REQUIRED_LABEL)`, et c'est toute la
/// différence.
///
/// # Pourquoi le terme de budget est retiré ici, et pourquoi c'est structurel
///
/// La garde `qa_build_callback_verdict` ne s'évalue que sur un EndTurn. Un tour
/// coupé n'en produit aucun : la coupure `DeadlineExceeded` se décide **en tête
/// d'itération** et `MaxStepsExceeded` est l'expression finale de la boucle,
/// donc la garde **n'a jamais eu l'occasion de firer** et son budget est
/// nécessairement intact. Exiger qu'il soit dépensé rendrait ce prédicat
/// **structurellement insatisfiable sur exactement la population qu'il vise** —
/// la leçon mika#2272, où une condition d'armement insatisfiable s'est lue
/// pendant des mois comme une flotte saine.
///
/// La symétrie inverse tient aussi et elle est asymétrique à dessein : au site
/// du « Force EndTurn » de `send_message` (mika#2515 U1e) c'est
/// [`verdict_unmet_after_retry`] qu'il faut, **avec** son terme de budget — ce
/// site-là *est* un EndTurn, donc la garde a bien eu son tour. Employer le
/// mauvais prédicat au mauvais site produit, dans un sens, un filet
/// insatisfiable et, dans l'autre, un `hold[review]` sur un tour que la garde
/// n'a jamais interrogé.
///
/// Les deux prédicats partagent [`pr_review_posted_in_turn`], donc un tour coupé
/// **après** avoir posté sa revue ne déclenche rien — contrôle négatif porteur.
pub fn verdict_unmet_at_cut_off(qa_verdict_due: bool, summaries: &[ToolCallSummary]) -> bool {
    qa_verdict_due && !pr_review_posted_in_turn(summaries)
}

/// Valeur du champ `cause` d'une ligne `qa_callback_verdict` quand le tour a été
/// coupé par son enveloppe de temps (mika#2515).
pub const CAUSE_CUT_OFF_DEADLINE: &str = "cut_off_deadline";

/// Valeur du champ `cause` quand le tour a épuisé son budget de steps d'outil
/// (mika#2515).
pub const CAUSE_CUT_OFF_MAX_STEPS: &str = "cut_off_max_steps";

/// Par quelle borne un tour de callback a été coupé (mika#2515).
///
/// **Format de fil** : [`Self::as_cause`] atterrit dans le champ `cause` de la
/// ligne `qa_callback_verdict`, dont l'opérateur fait des `GROUP BY`. Deux
/// orthographes d'une même borne couperaient une population en deux sans le
/// dire — d'où un `match` exhaustif **sans bras `_`**, épinglé par test : une
/// troisième borne devra *décider* de son mot au lieu d'hériter du voisin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutOffExit {
    /// `LoopResult::DeadlineExceeded` — l'enveloppe de temps du tour est
    /// atteinte, testée en tête d'itération.
    Deadline,
    /// `LoopResult::MaxStepsExceeded` — le budget de steps d'outil est épuisé.
    MaxSteps,
}

impl CutOffExit {
    /// Sentinelle de [`VerdictSignal::cut_off_exit`] avant toute écriture.
    ///
    /// Elle existe pour que la lecture soit **totale sans mentir** : un
    /// discriminant inconnu rend `None`, donc zéro POST, plutôt qu'un `cause`
    /// faux ou une panique dans le dispatcher.
    const DISCRIMINANT_UNSET: u8 = u8::MAX;
    const DISCRIMINANT_DEADLINE: u8 = 0;
    const DISCRIMINANT_MAX_STEPS: u8 = 1;

    /// Le mot de fil de cette borne. `match` exhaustif, aucun bras `_`.
    pub fn as_cause(self) -> &'static str {
        match self {
            Self::Deadline => CAUSE_CUT_OFF_DEADLINE,
            Self::MaxSteps => CAUSE_CUT_OFF_MAX_STEPS,
        }
    }

    /// Forme stockable dans un `AtomicU8`. `match` exhaustif, aucun bras `_`.
    const fn as_discriminant(self) -> u8 {
        match self {
            Self::Deadline => Self::DISCRIMINANT_DEADLINE,
            Self::MaxSteps => Self::DISCRIMINANT_MAX_STEPS,
        }
    }

    /// Relit [`Self::as_discriminant`]. `None` sur toute autre valeur — un
    /// signal qu'on ne peut pas lire n'est jamais un terme satisfait, et cette
    /// direction-ci porte nécessairement un bras générique (le domaine est
    /// `u8`, pas l'enum), ce qui ne dispense aucune variante de décider dans
    /// l'autre sens.
    const fn from_discriminant(raw: u8) -> Option<Self> {
        match raw {
            Self::DISCRIMINANT_DEADLINE => Some(Self::Deadline),
            Self::DISCRIMINANT_MAX_STEPS => Some(Self::MaxSteps),
            _ => None,
        }
    }
}

/// Le fait de coupure qu'un tour silencieux rend à son appelant (mika#2515).
///
/// Symétrique d'`AgentOutput.deadline_exceeded` — commenté « mika#2276 M2: the
/// one place that says "cut off, not concluded" » — dont `SilentTurnOutcome`
/// n'avait jamais reçu l'équivalent. C'était le trou, nommé par sa symétrie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackCutOff {
    /// Quelle borne a été franchie. Décide du `cause` de la ligne postée.
    pub exit: CutOffExit,
    /// Combien de steps d'outil le tour avait accomplis. Un tour coupé au step 2
    /// et un tour coupé au step 19 appellent des réponses opérateur différentes.
    pub steps_completed: usize,
}

/// Le signal de verdict-dû-non-posté que `run_loop` pose et que
/// `run_silent_agent` rend (mika#2368 + mika#2515).
///
/// # Pourquoi un struct plutôt qu'un second paramètre
///
/// `run_loop` recevait `qa_verdict_unmet: Option<&AtomicBool>`. Il reçoit
/// désormais `Option<&VerdictSignal>` — **même arité**. La fonction a trois
/// appelants (`run_agent`, `run_silent_agent`, `run_team_agent`) et un seul
/// passe autre chose que `None` ; conserver l'arité fait que les deux autres
/// changent d'un type dans une position déjà occupée par `None`, sans qu'aucun
/// comportement conversationnel ou d'équipe ne bouge. Ajouter un paramètre
/// ferait porter à trois appelants le coût d'un besoin qui n'en concerne qu'un.
///
/// # Les deux moitiés sont mutuellement exclusives par construction
///
/// Un tour sort de `run_loop` par exactement un chemin : les sorties EndTurn
/// posent [`Self::mark_unmet_after_retry`], les sorties coupées
/// [`Self::mark_cut_off`]. Un test l'épingle plutôt que de le tolérer en
/// silence, et le `match` à quatre bras du dispatcher traite quand même le
/// croisement — en faisant gagner le plus spécifique, jamais par un
/// `unreachable!()` : paniquer dans le dispatcher sur un champ d'observabilité
/// serait le pire des échanges.
#[derive(Debug)]
pub struct VerdictSignal {
    /// Sorties EndTurn : budget de la garde épuisé, rien de posté (mika#2368).
    unmet_after_retry: AtomicBool,
    /// Sorties coupées : un verdict était dû, rien de posté (mika#2515).
    ///
    /// Porte de lecture des deux champs suivants, qui sont écrits **avant** lui
    /// (`Release`) et lus **après** lui (`Acquire`).
    cut_off: AtomicBool,
    cut_off_exit: AtomicU8,
    cut_off_steps: AtomicUsize,
}

impl Default for VerdictSignal {
    fn default() -> Self {
        Self {
            unmet_after_retry: AtomicBool::new(false),
            cut_off: AtomicBool::new(false),
            cut_off_exit: AtomicU8::new(CutOffExit::DISCRIMINANT_UNSET),
            cut_off_steps: AtomicUsize::new(0),
        }
    }
}

impl VerdictSignal {
    /// Un signal neuf, aucune moitié posée.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pose la moitié mika#2368 : le tour a **conclu** sans poster son verdict.
    pub fn mark_unmet_after_retry(&self) {
        self.unmet_after_retry.store(true, Ordering::Relaxed);
    }

    /// Pose la moitié mika#2515 : le tour a été **coupé** avant de poster.
    ///
    /// Les deux champs de détail sont écrits avant le drapeau de porte, avec un
    /// `Release` sur celui-ci : un lecteur qui voit `cut_off == true` voit
    /// nécessairement l'exit et les steps qui vont avec.
    pub fn mark_cut_off(&self, exit: CutOffExit, steps_completed: usize) {
        self.cut_off_exit
            .store(exit.as_discriminant(), Ordering::Relaxed);
        self.cut_off_steps.store(steps_completed, Ordering::Relaxed);
        self.cut_off.store(true, Ordering::Release);
    }

    /// La moitié mika#2368.
    pub fn unmet_after_retry(&self) -> bool {
        self.unmet_after_retry.load(Ordering::Relaxed)
    }

    /// La moitié mika#2515, ou `None` si aucune coupure n'a été posée — ou si
    /// son discriminant est illisible, état inatteignable par construction mais
    /// dont la lecture reste totale plutôt que paniquante.
    pub fn cut_off(&self) -> Option<CallbackCutOff> {
        if !self.cut_off.load(Ordering::Acquire) {
            return None;
        }
        let exit = CutOffExit::from_discriminant(self.cut_off_exit.load(Ordering::Relaxed))?;
        Some(CallbackCutOff {
            exit,
            steps_completed: self.cut_off_steps.load(Ordering::Relaxed),
        })
    }
}

/// Le re-prompt de la garde positive.
///
/// Nomme l'outil **et** la ligne attendue : une correction qui dit seulement
/// « vous n'avez pas fini » laisse le modèle deviner par quoi finir, et il
/// devine le contrat qu'on vient précisément de lui retirer.
pub const QA_VERDICT_REQUIRED_CORRECTION: &str = "[mika-engine] This is a build callback for a QA review, and the review is not \
     complete: no successful `run_gh` call with `pr review` appears in this turn's \
     tool history. A qa-review turn concludes by POSTING the review to GitHub — \
     the posted review is the source of truth, and verdict text in your response \
     is only a mirror. Follow the qa-review-build-callback workflow (re-read the \
     plan, execute the ACs, compose PLAN-AC VERIFICATION and DIFF ANALYSIS) and \
     call `run_gh` with `pr review` carrying a trailing `VERDICT:` line \
     (`pass` / `hold[review]` / `block[ac]` / `block[ci]` / `block[security]` / \
     `block[pipeline]`). Do not call `update_task_status` or `send_message` \
     instead — they do not deliver a verdict to the PR.";

/// Pourquoi le verdict d'un build vert n'est **pas encore** arrivé sur la PR
/// (mika#2515 U2a).
///
/// **Format de fil** : [`Self::as_wire`] atterrit dans `after_value` de la ligne
/// `qa_build_verdict_undelivered`, dont l'opérateur fait des `GROUP BY` pour
/// dimensionner le suivi « poster sur quarantaine ». `match` exhaustif **sans
/// bras `_`**, épinglé par test.
///
/// Les quatre valeurs sont lues sur les clés que mika#2179 écrit déjà plus le
/// compteur d'U3 — aucune requête de plus, aucune colonne de plus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndeliveredVerdictCause {
    /// `verdict_delivery_deferrals > 0` et aucune tentative de livraison : le
    /// verrou d'agent n'a jamais été gagné, le tour n'a **jamais tourné**.
    /// C'est la contention, et c'est la population que mika#2515 U3 rend
    /// **positivement** attribuable au lieu de la déduire d'une absence.
    AgentBusyStarvation,
    /// `delivery_attempts > 0`, pas de quarantaine : mika#2179 réessaie et la
    /// PR attend.
    TurnFailed,
    /// `delivery_quarantined_at` présent : le budget mika#2179 est épuisé, le
    /// réessai est horaire, le verdict est effectivement perdu. C'est la seule
    /// des quatre où poster serait juste — suivi conditionné à ce que la
    /// première distribution montre cette population non vide.
    Quarantined,
    /// Aucun des deux compteurs. **Bras anti-vacuité** : sans lui, une ligne
    /// que rien n'a jamais tenté de livrer se lirait comme une famine, et le
    /// remède qu'on irait chercher (la contention) n'aurait aucun rapport avec
    /// la panne (le balayage de livraison ne tourne pas). Même raison qui a fait
    /// séparer `below_threshold` de `no_ready_label_event` (mika#2131) et
    /// `in_flight_self_dev` de `live_pilot_orphaned_parent` (mika#2279).
    NeverAttempted,
}

impl UndeliveredVerdictCause {
    /// Le mot de fil de cette cause. `match` exhaustif, aucun bras `_`.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::AgentBusyStarvation => "agent_busy_starvation",
            Self::TurnFailed => "turn_failed",
            Self::Quarantined => "quarantined",
            Self::NeverAttempted => "never_attempted",
        }
    }
}

/// Attribue la non-livraison d'un verdict à l'un des quatre chemins, en lisant
/// le `metadata` de la ligne callback (mika#2515 U2a).
///
/// **Fonction pure** : testable aux quatre bornes sans base de données, ce qui
/// est la seule façon de voir rougir chaque terme séparément (leçon mika#2420 —
/// une assertion de non-vacuité est insensible aux termes qu'un autre absorbe en
/// aval).
///
/// # L'ordre des tests est porteur, et il va du plus spécifique au plus général
///
/// La quarantaine **implique** des tentatives, et une tentative **implique** que
/// le verrou a été gagné au moins une fois. Tester dans l'autre sens ferait lire
/// une ligne quarantinée comme un simple `turn_failed`, et une ligne affamée
/// puis enfin livrée-et-échouée comme une famine — deux attributions fausses
/// portant l'autorité d'une mesure.
///
/// # Fail-safe vers « on ne sait pas »
///
/// Un `metadata` absent, vide ou illisible rend [`UndeliveredVerdictCause::NeverAttempted`] :
/// on ne peut rien affirmer sur les compteurs, et la valeur qui dit « rien n'a
/// jamais tenté » est précisément celle dont la halte 5 prescrit d'établir la
/// cause avant de toucher à l'attribution. L'inverse — deviner la contention —
/// fabriquerait le diagnostic le plus flatteur pour ce ticket.
pub fn classify_undelivered_verdict(metadata: Option<&str>) -> UndeliveredVerdictCause {
    if metadata_field_present(metadata, crate::task_engine::DELIVERY_QUARANTINED_AT_KEY) {
        return UndeliveredVerdictCause::Quarantined;
    }
    if metadata_counter(metadata, crate::task_engine::DELIVERY_ATTEMPTS_KEY) > 0 {
        return UndeliveredVerdictCause::TurnFailed;
    }
    if metadata_counter(metadata, crate::task_engine::VERDICT_DELIVERY_DEFERRALS_KEY) > 0 {
        return UndeliveredVerdictCause::AgentBusyStarvation;
    }
    UndeliveredVerdictCause::NeverAttempted
}

/// Lit un compteur de `tasks.metadata`, `0` si absent ou illisible (mika#2515).
///
/// **Les deux formes JSON sont acceptées, et ce n'est pas de la tolérance
/// gratuite** : `set_task_metadata_field` passe par `json_set` avec une valeur de
/// **chaîne**, donc le compteur est en TEXT sur le chemin nominal — mais une
/// écriture antérieure ou un opérateur peut y avoir laissé un nombre, et faire
/// dépendre une attribution du type JSON d'un champ produirait une famine lue
/// comme un `never_attempted`, c'est-à-dire la confusion exacte que le quatrième
/// bras existe pour empêcher.
///
/// Site unique, partagé par [`classify_undelivered_verdict`] et par la ligne
/// d'alerte qui rapporte les mêmes compteurs : deux lectures écrites à la main
/// pourraient classer sur une valeur et en rapporter une autre.
pub fn metadata_counter(metadata: Option<&str>, key: &str) -> u64 {
    parse_metadata(metadata)
        .get(key)
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .unwrap_or(0)
}

/// Un champ de `tasks.metadata` est-il présent et non vide ? (mika#2515)
///
/// Une chaîne vide ne compte pas : `json_set` peut en écrire une, et lire
/// « quarantiné » d'un `""` ferait sortir la ligne de la population de la
/// contention sur un champ qui ne dit rien.
fn metadata_field_present(metadata: Option<&str>, key: &str) -> bool {
    parse_metadata(metadata)
        .get(key)
        .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()))
}

/// Un `metadata` absent, vide ou illisible rend `Null` — dont tout `.get()` rend
/// `None`, donc tout compteur `0` et tout champ absent. C'est le fail-safe vers
/// « on ne sait pas » de [`classify_undelivered_verdict`].
fn parse_metadata(metadata: Option<&str>) -> serde_json::Value {
    metadata
        .filter(|m| !m.trim().is_empty())
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str, input: &str, success: bool) -> ToolCallSummary {
        ToolCallSummary {
            step: 0,
            name: name.to_string(),
            input_summary: input.to_string(),
            output_summary: String::new(),
            success,
            non_zero_exit: false,
        }
    }

    /// Le marqueur est la forme que le moteur émet réellement — reconstruite
    /// par les deux `format!` de production, jamais recopiée à la main.
    #[test]
    fn mika2355_the_marker_is_the_shape_the_engine_actually_emits() {
        let label = format!("long_running:{BUILD_MIKA_TOOL}");
        assert_eq!(
            label, BUILD_CALLBACK_LABEL,
            "le label a divergé de la grammaire de build_callback_task"
        );
        assert!(is_build_callback_label(&label));
        assert!(!is_build_callback_label("long_running:run_claude_pilot"));
        assert!(
            !is_build_callback_label("long_running:build_mika_extra"),
            "égalité stricte, pas de préfixe"
        );
        let emitted = format!("[callback: {label}]");
        assert_eq!(
            emitted, BUILD_CALLBACK_MESSAGE_MARKER,
            "le marqueur a divergé d'une des deux grammaires qui le composent"
        );
    }

    /// Le suffixe milestone ne doit pas désarmer le discriminant.
    #[test]
    fn mika2355_a_milestone_suffix_does_not_hide_the_marker() {
        assert!(is_build_callback(
            "[callback: long_running:build_mika] [milestone-parent: abc]"
        ));
    }

    /// Les cinq autres outils `long_running` ne sont pas des callbacks de build.
    #[test]
    fn mika2355_the_other_long_running_tools_are_not_build_callbacks() {
        for tool in [
            "run_claude_pilot",
            "run_claude_pilot_groom",
            "deploy_mika",
            "address_pr_comments",
            "resolve_pr_conflicts",
        ] {
            assert!(
                !is_build_callback(&format!("[callback: long_running:{tool}]")),
                "{tool} ne doit pas être lu comme un callback de build"
            );
        }
    }

    /// AC4b — la conjonction, et le contrôle négatif qui est la moitié qui compte.
    #[test]
    fn mika2355_a_verdict_is_due_only_where_qa_review_is_loaded() {
        let msg = BUILD_CALLBACK_MESSAGE_MARKER;
        assert!(qa_verdict_required(msg, ["build-mika", "qa-review"]));
        assert!(
            qa_verdict_required(msg, ["QA-Review"]),
            "la comparaison de nom de skill est insensible à la casse"
        );
        // Le cas mika-dev : build_mika est dans son allowlist, il ne doit rien.
        assert!(!qa_verdict_required(msg, ["build-mika", "self-dev"]));
        assert!(!qa_verdict_required(msg, Vec::<String>::new()));
        // Et un autre flux de callback avec qa-review chargé ne doit rien non plus.
        assert!(!qa_verdict_required(
            "[callback: long_running:run_claude_pilot]",
            ["qa-review"]
        ));
    }

    /// Le prédicat de satisfaction répond comme l'early-accept #695/#821.
    #[test]
    fn mika2355_the_satisfied_predicate_matches_the_early_accept_one() {
        let posted = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2355","--comment"]}"#,
            true,
        )];
        assert!(pr_review_posted_in_turn(&posted));

        // Un échec n'est pas une revue postée.
        let failed = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2355"]}"#,
            false,
        )];
        assert!(!pr_review_posted_in_turn(&failed));

        // Un autre sous-commande `gh` non plus.
        let view = vec![summary("run_gh", r#"{"args":["pr","view","2355"]}"#, true)];
        assert!(!pr_review_posted_in_turn(&view));

        // Ni le contrat self_dev qu'on vient de retirer à ce flux.
        let self_dev = vec![
            summary("update_task_status", "{}", true),
            summary("send_message", "{}", true),
        ];
        assert!(!pr_review_posted_in_turn(&self_dev));
    }

    /// mika#2368 — le complémentaire de la garde : les trois termes, chacun
    /// avec son contrôle négatif.
    #[test]
    fn mika2368_the_net_predicate_is_the_exact_complement_of_the_guard() {
        let mut spent: HashSet<&'static str> = HashSet::new();
        spent.insert(QA_VERDICT_REQUIRED_LABEL);
        let fresh: HashSet<&'static str> = HashSet::new();
        let nothing: Vec<ToolCallSummary> = vec![];
        let posted = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2368","--comment"]}"#,
            true,
        )];

        // Le cas du ticket : verdict dû, budget épuisé, rien de posté.
        assert!(verdict_unmet_after_retry(true, &spent, &nothing));

        // Budget NON consommé : c'est à la garde de re-prompter, pas au filet.
        // Sans ce terme, le filet doublerait le re-prompt au lieu de lui succéder.
        assert!(!verdict_unmet_after_retry(true, &fresh, &nothing));

        // La revue a été postée : rien n'est dû (AC7).
        assert!(!verdict_unmet_after_retry(true, &spent, &posted));

        // Aucun verdict dû sur ce tour (le cas mika-dev, AC4b).
        assert!(!verdict_unmet_after_retry(false, &spent, &nothing));

        // Un budget consommé par une AUTRE garde ne compte pas.
        let mut other: HashSet<&'static str> = HashSet::new();
        other.insert("callback_terminal_action");
        assert!(!verdict_unmet_after_retry(true, &other, &nothing));
    }

    // -----------------------------------------------------------------------
    // mika#2515 — le prédicat de coupure, le signal, les formats de fil
    // -----------------------------------------------------------------------

    /// **V1** — le prédicat de coupure, ses deux termes et leurs contrôles
    /// négatifs.
    #[test]
    fn mika2515_the_cut_off_predicate_has_two_terms() {
        let nothing: Vec<ToolCallSummary> = vec![];
        let posted = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2515","--comment"]}"#,
            true,
        )];

        // Le cas du ticket : verdict dû, rien de posté, tour coupé.
        assert!(verdict_unmet_at_cut_off(true, &nothing));
        // La revue a été postée avant la coupure : contrôle négatif porteur,
        // partagé avec `verdict_unmet_after_retry` via `pr_review_posted_in_turn`.
        assert!(!verdict_unmet_at_cut_off(true, &posted));
        // Aucun verdict dû (le cas mika-dev) : rien.
        assert!(!verdict_unmet_at_cut_off(false, &nothing));
    }

    /// **V1, le contrôle qui porte tout U1** — le prédicat de coupure est vrai
    /// **sans** que la garde ait dépensé son budget, là où son frère est faux.
    ///
    /// C'est la différence entre les deux prédicats, et sans elle celui de
    /// coupure serait **structurellement insatisfiable sur sa propre
    /// population** : sur un tour coupé la garde n'a jamais eu d'EndTurn où
    /// s'évaluer, donc `intent_guard_retries` est nécessairement vide. La leçon
    /// mika#2272 — une condition insatisfiable se lit exactement comme une flotte
    /// saine.
    #[test]
    fn mika2515_the_cut_off_predicate_does_not_require_a_spent_guard_budget() {
        let fresh: HashSet<&'static str> = HashSet::new();
        let nothing: Vec<ToolCallSummary> = vec![];

        assert!(
            verdict_unmet_at_cut_off(true, &nothing),
            "un tour coupé n'a pas dépensé le budget de la garde, et doit tout \
             de même armer le filet"
        );
        assert!(
            !verdict_unmet_after_retry(true, &fresh, &nothing),
            "son frère exige ce budget — c'est ce qui les distingue, et \
             « harmoniser » les deux sites sur un seul prédicat casse l'un ou \
             l'autre (V2b)"
        );
    }

    /// **Format de fil** — les valeurs de `cause` sont figées, et un `match`
    /// exhaustif sans bras `_` force une troisième borne à décider de son mot.
    #[test]
    fn mika2515_the_cut_off_cause_values_are_a_wire_format() {
        assert_eq!(CutOffExit::Deadline.as_cause(), "cut_off_deadline");
        assert_eq!(CutOffExit::MaxSteps.as_cause(), "cut_off_max_steps");
        assert_eq!(CAUSE_CUT_OFF_DEADLINE, "cut_off_deadline");
        assert_eq!(CAUSE_CUT_OFF_MAX_STEPS, "cut_off_max_steps");
        assert_ne!(
            CutOffExit::Deadline.as_cause(),
            CutOffExit::MaxSteps.as_cause(),
            "deux bornes qui partageraient un mot couperaient une population en \
             deux sans le dire"
        );
        // Le `cause` d'une coupure doit être reconnaissable par le préfixe que
        // les sondes opérateur emploient (`startswith(\"cut_off\")`).
        for exit in [CutOffExit::Deadline, CutOffExit::MaxSteps] {
            assert!(
                exit.as_cause().starts_with("cut_off"),
                "la sonde opérateur soustrait par ce préfixe"
            );
        }
    }

    /// Le discriminant fait un aller-retour, et une valeur inconnue rend `None`
    /// plutôt que de mentir — état inatteignable par construction, dont la
    /// lecture reste totale.
    #[test]
    fn mika2515_the_exit_discriminant_round_trips_and_fails_safe() {
        for exit in [CutOffExit::Deadline, CutOffExit::MaxSteps] {
            assert_eq!(
                CutOffExit::from_discriminant(exit.as_discriminant()),
                Some(exit)
            );
        }
        assert_eq!(
            CutOffExit::from_discriminant(CutOffExit::DISCRIMINANT_UNSET),
            None,
            "la sentinelle n'est pas une borne"
        );
        assert_eq!(CutOffExit::from_discriminant(42), None);
    }

    /// Le signal porte ses deux moitiés, et un signal neuf n'en porte aucune.
    #[test]
    fn mika2515_a_fresh_signal_carries_neither_half() {
        let signal = VerdictSignal::new();
        assert!(!signal.unmet_after_retry());
        assert_eq!(signal.cut_off(), None);
    }

    /// Les deux moitiés sont **mutuellement exclusives par construction** (un
    /// tour sort de `run_loop` par exactement un chemin) — épinglé plutôt que
    /// toléré en silence.
    #[test]
    fn mika2515_the_two_halves_are_written_independently() {
        let concluded = VerdictSignal::new();
        concluded.mark_unmet_after_retry();
        assert!(concluded.unmet_after_retry());
        assert_eq!(
            concluded.cut_off(),
            None,
            "poser la moitié « conclu » ne doit pas fabriquer une coupure"
        );

        let cut = VerdictSignal::new();
        cut.mark_cut_off(CutOffExit::MaxSteps, 20);
        assert!(
            !cut.unmet_after_retry(),
            "poser la moitié « coupé » ne doit pas fabriquer une conclusion"
        );
        assert_eq!(
            cut.cut_off(),
            Some(CallbackCutOff {
                exit: CutOffExit::MaxSteps,
                steps_completed: 20,
            })
        );
    }

    /// Les steps voyagent avec la borne : un tour coupé au step 2 et un tour
    /// coupé au step 19 appellent des réponses opérateur différentes.
    #[test]
    fn mika2515_the_cut_off_carries_its_steps() {
        for (exit, steps) in [(CutOffExit::Deadline, 2), (CutOffExit::MaxSteps, 19)] {
            let signal = VerdictSignal::new();
            signal.mark_cut_off(exit, steps);
            let observed = signal.cut_off().expect("une coupure posée est lisible");
            assert_eq!(observed.exit, exit);
            assert_eq!(observed.steps_completed, steps);
        }
    }

    // -----------------------------------------------------------------------
    // mika#2515 U2a — le classificateur de non-livraison
    // -----------------------------------------------------------------------

    /// **V5** — les quatre bornes du classificateur, **chaque terme muté et vu
    /// rouge** : une assertion de non-vacuité est insensible aux termes qu'un
    /// autre absorbe en aval (leçon mika#2420).
    #[test]
    fn mika2515_the_undelivered_classifier_reads_four_distinct_shapes() {
        use UndeliveredVerdictCause as C;

        // (1) La famine : le verrou n'a jamais été gagné, aucune tentative.
        assert_eq!(
            classify_undelivered_verdict(Some(r#"{"verdict_delivery_deferrals":"7"}"#)),
            C::AgentBusyStarvation
        );
        // (2) Le tour a tourné et a échoué : mika#2179 réessaie.
        assert_eq!(
            classify_undelivered_verdict(Some(r#"{"delivery_attempts":"2"}"#)),
            C::TurnFailed
        );
        // (3) Budget mika#2179 épuisé : le verdict est effectivement perdu.
        assert_eq!(
            classify_undelivered_verdict(Some(
                r#"{"delivery_attempts":"3","delivery_quarantined_at":"2026-09-24T14:49:00Z"}"#
            )),
            C::Quarantined
        );
        // (4) Bras anti-vacuité : rien n'a jamais tenté de livrer cette ligne.
        assert_eq!(classify_undelivered_verdict(Some("{}")), C::NeverAttempted);
    }

    /// L'ordre des tests est porteur, et il va du plus spécifique au plus
    /// général. Muter l'ordre ferait lire une ligne quarantinée comme un simple
    /// `turn_failed`, et une ligne affamée puis livrée-et-échouée comme une
    /// famine — deux attributions fausses portant l'autorité d'une mesure.
    #[test]
    fn mika2515_the_classifier_lets_the_most_specific_term_win() {
        use UndeliveredVerdictCause as C;

        // Quarantaine ET tentatives ET déférals : la quarantaine gagne.
        assert_eq!(
            classify_undelivered_verdict(Some(
                r#"{"verdict_delivery_deferrals":"4","delivery_attempts":"3",
                    "delivery_quarantined_at":"2026-09-24T15:02:27Z"}"#
            )),
            C::Quarantined
        );
        // Tentatives ET déférals, pas de quarantaine : le tour a tourné.
        assert_eq!(
            classify_undelivered_verdict(Some(
                r#"{"verdict_delivery_deferrals":"4","delivery_attempts":"1"}"#
            )),
            C::TurnFailed,
            "une ligne enfin livrée puis échouée n'est plus une famine"
        );
    }

    /// Fail-safe vers « on ne sait pas », jamais vers le diagnostic le plus
    /// flatteur pour ce ticket : un `metadata` absent, vide ou illisible n'est
    /// **pas** lu comme une famine.
    #[test]
    fn mika2515_an_unreadable_metadata_is_never_read_as_starvation() {
        use UndeliveredVerdictCause as C;
        for shape in [None, Some(""), Some("   "), Some("pas du json"), Some("[]")] {
            assert_eq!(
                classify_undelivered_verdict(shape),
                C::NeverAttempted,
                "{shape:?} ne doit rien affirmer sur les compteurs"
            );
        }
    }

    /// Un compteur à `0` n'est pas un compteur posé, et une chaîne vide n'est
    /// pas une quarantaine — sans quoi une ligne neuve se lirait comme affamée.
    #[test]
    fn mika2515_zero_and_empty_are_not_evidence() {
        use UndeliveredVerdictCause as C;
        assert_eq!(
            classify_undelivered_verdict(Some(
                r#"{"verdict_delivery_deferrals":"0","delivery_attempts":"0"}"#
            )),
            C::NeverAttempted
        );
        assert_eq!(
            classify_undelivered_verdict(Some(r#"{"delivery_quarantined_at":""}"#)),
            C::NeverAttempted
        );
    }

    /// Les deux formes JSON du compteur sont lues — TEXT sur le chemin nominal
    /// (`json_set` écrit une chaîne), nombre si une autre écriture en a laissé
    /// un. Faire dépendre une attribution du type JSON d'un champ produirait une
    /// famine lue comme un `never_attempted`.
    #[test]
    fn mika2515_a_counter_reads_in_both_json_shapes() {
        use UndeliveredVerdictCause as C;
        assert_eq!(
            classify_undelivered_verdict(Some(r#"{"verdict_delivery_deferrals":3}"#)),
            C::AgentBusyStarvation,
            "forme numérique"
        );
        assert_eq!(
            metadata_counter(Some(r#"{"delivery_attempts":"5"}"#), "delivery_attempts"),
            5
        );
        assert_eq!(
            metadata_counter(Some(r#"{"delivery_attempts":5}"#), "delivery_attempts"),
            5
        );
        assert_eq!(metadata_counter(Some("{}"), "delivery_attempts"), 0);
        assert_eq!(metadata_counter(None, "delivery_attempts"), 0);
    }

    /// **Format de fil** — les quatre mots de `UndeliveredVerdictCause` sont
    /// figés et deux à deux distincts : l'opérateur en fait des `GROUP BY` pour
    /// dimensionner le suivi « poster sur quarantaine ».
    #[test]
    fn mika2515_the_undelivered_cause_values_are_a_wire_format() {
        use UndeliveredVerdictCause as C;
        let all = [
            (C::AgentBusyStarvation, "agent_busy_starvation"),
            (C::TurnFailed, "turn_failed"),
            (C::Quarantined, "quarantined"),
            (C::NeverAttempted, "never_attempted"),
        ];
        for (cause, wire) in all {
            assert_eq!(cause.as_wire(), wire);
        }
        let wires: HashSet<&str> = all.iter().map(|(c, _)| c.as_wire()).collect();
        assert_eq!(wires.len(), 4, "quatre mots deux à deux distincts");
    }

    /// AC1b — l'en-tête de portée du prompt cite **la** chaîne que le moteur
    /// émet. Le prompt ne peut pas importer la constante ; ce test est le seul
    /// lien entre les deux, et sans lui l'en-tête peut citer un marqueur périmé
    /// tout en restant parfaitement lisible.
    #[test]
    fn mika2355_the_scope_header_quotes_the_engine_marker() {
        let prompt = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../skills/bundled/qa-review-build-callback/system_prompt.md"
        ));
        assert!(
            prompt.contains(BUILD_CALLBACK_MESSAGE_MARKER),
            "l'en-tête de portée ne cite pas le marqueur du moteur — hors callback \
             de build, ce fichier autorise textuellement à sauter la revue de diff"
        );
        // Et il doit le citer AVANT la première instruction de reprise, sinon la
        // condition de portée arrive après ce qu'elle conditionne.
        let marker_at = prompt.find(BUILD_CALLBACK_MESSAGE_MARKER).unwrap();
        let resume_at = prompt
            .find("Steps 1–3d were completed")
            .expect("la phrase de reprise doit exister");
        assert!(
            marker_at < resume_at,
            "la condition de portée doit précéder l'instruction qu'elle conditionne"
        );
    }
}

//! Le registre des têtes sur lesquelles mika-qa a **statué**
//! (mika#2917, vols 2 et 3/3 — enfant de mika#2680, classe (g′), forme (a) ;
//! vocabulaire corrigé par mika#2927).
//!
//! # Ce que ce module est
//!
//! Il fige le **vocabulaire** du registre : son `tool_name`, les trois issues qui
//! l'écrivent, la normalisation qui transforme une attestation du modèle en l'un
//! de ces trois littéraux, et depuis le vol 3 la **profondeur** sur laquelle il
//! se lit ([`REVIEWED_HEAD_WINDOW_DAYS`]). Il ne décide toujours rien : le vol 3
//! a ajouté un lecteur, et ce lecteur est unique — le terme
//! `PrefilterState::reviewed_head` de [`crate::server::webhook_prefilter`], que
//! la garde (a.2) de ce module borne à un.
//!
//! # L'endroit, et pourquoi là
//!
//! ```text
//! tool_name   = QA_REVIEWED_HEAD_TOOL
//! target_key  = head_key(repo, pr, head_sha)  →  pr:{repo}#{n}@{sha}
//! after_value = l'issue du tour, l'un des trois de REVIEWED_HEAD_OUTCOMES
//! reasoning   = "<issue> sur <target_key>" — l'issue et la tête, sans nommer de
//!               skill : depuis mika#2927 trois prompts distincts écrivent ici
//! agent_id    = injecté par AsyncDatabase, jamais passé par l'appelant
//! ```
//!
//! Les quatre champs que le ticket demande y sont au complet : `repo`, `PR` et
//! `head_sha` dans la clé, l'issue dans `after_value`.
//!
//! 1. **Aucun `gh`.** Le lecteur du vol 3 lit la base SQLite de son propre agent
//!    par l'accesseur qui existait déjà,
//!    `AsyncDatabase::latest_audit_event_for_target` (mika#2242). Le bearing de
//!    mika#2680 refuse un appel `gh` depuis `webhook_prefilter.rs` (mika#2260).
//! 2. **Aucune table, aucune migration.** Le précédent vit dans ce dépôt :
//!    [`crate::qa_head_supersession`], dont la doc dit « pas de table, pas de
//!    migration » et dont `SYNC_OBSERVED_TOOL` porte la même déclaration
//!    **SOLE WRITER**. Même mécanisme, même titre.
//! 3. **Par agent sans que l'appelant le dise.** `AsyncDatabase::log_audit_event`
//!    clone son propre `agent_id` et ne l'accepte pas en argument — son premier
//!    paramètre est le `session_id`. `latest_audit_event_for_target` fait de même
//!    en lecture : l'`agent_id` est le paramètre SQL 1 et vient du handle, pas de
//!    l'appelant. L'état de mika-qa ne peut donc pas écarter un tour de mika-dev,
//!    et la propriété ne dépend d'aucune discipline d'appel — c'est le premier
//!    point de l'audit MPC du vol 3, et
//!    `mika2917_vol3_une_ligne_de_mika_qa_necarte_pas_le_tour_de_mika_dev` la
//!    mesure au niveau de la base, non au niveau d'un bouchon.
//! 4. **Aucune grammaire de clé nouvelle.** [`head_key`] existe et le pré-filtre
//!    la relit déjà telle quelle. Une forme de clé retapée dans un second fichier
//!    est la divergence programmée que mika#2158 a payée — une garde de ce module
//!    refuse donc que ce fichier retape la grammaire.
//! 5. **La décision reste entière dans le skill.** Le moteur ne calcule jamais
//!    « cette tête est revue » : il compare la tête que le payload nomme à une
//!    ligne que le skill a écrite, et il ne lit qu'`after_value`.
//!
//! # La perte de revue, et ce qui l'empêche
//!
//! C'est le seul risque matériel du mécanisme, et le vocabulaire figé ici est ce
//! qui le borne.
//!
//! 1. **L'écriture est postérieure au verdict posté**, pas au dispatch de la
//!    revue. C'est la correction de mika#2927, et elle est née d'une mesure, pas
//!    d'une précaution : le 2026-10-26 à 15:18:30Z la session `4f94fbc8` a
//!    inscrit `review` sur
//!    `pr:senara-solutions/mika#2920@0a55d28a9d6a43848aaadbb301bd3c236a477088`
//!    au moment où elle dépêchait `qa-review`, puis a épuisé son budget de pas à
//!    15:19:16Z — aucun build lancé, aucun verdict posté, aucune revue sur cette
//!    tête. Une revue dépêchée peut mourir avant son verdict ; un verdict posté,
//!    non. L'appel vit donc dans le tour qui a posté la revue sur la forge, après
//!    un `pr review` réussi, et jamais dans celui qui l'a dépêchée.
//! 2. **Trois issues écrivent, et elles attestent toutes un fait accompli** —
//!    voir [`REVIEWED_HEAD_OUTCOMES`]. Le vocabulaire fermé est ce qui rend les
//!    autres non représentables : une issue qui ne conclut sur aucune tête n'est
//!    pas écrivable, et depuis mika#2927 la graphie `review` — celle du dispatch
//!    — n'en fait plus partie. [`reviewed_head_outcome`] la refuse, donc les
//!    lignes `review` déjà inscrites avant le correctif sortent du contrat et le
//!    lecteur du vol 3 les relâche sous sa règle `reviewed_head_out_of_contract`
//!    sans qu'aucun SQL n'ait à les toucher.
//! 3. **La clé est la tête, pas la PR.** Un nouveau push produit une nouvelle
//!    `head_sha`, donc une clé que le registre ne porte pas : le tour passe. C'est
//!    structurel, pas une branche de code.
//! 4. **Fail-safe dans le sens de la revue.** Une écriture manquée — outil absent,
//!    base en erreur, modèle qui oublie l'appel — laisse le registre sans la
//!    ligne, donc le vol 3 fera passer le tour : le régime d'aujourd'hui, un tour
//!    payé. C'est pourquoi l'outil refuse plutôt que de deviner, et pourquoi
//!    `skill.toml` ne déclare pas l'outil en `required_tools` (un `required_tools`
//!    non résolu est un échec **terminal**, donc une revue perdue là où une
//!    écriture manquée ne coûte qu'un tour).
//!
//! # Pas d'interrupteur dans ce vol
//!
//! Il n'y a aucune porte à désarmer : rien ne lit ce registre. L'interrupteur de
//! la classe (g′) arrive au vol 3, avec le décideur qu'il désarme — un
//! interrupteur qui ne commande rien se lit comme une protection qui existe.

use crate::qa_build_callback::pr_review_posted_in_turn;
use crate::tool_execution::ToolCallSummary;

/// La grammaire de clé du registre, réexportée plutôt que retapée (point 4 de la
/// doc de module). `pr:{repo}#{n}@{sha}`.
pub(crate) use crate::server::ci_success_handler::head_key;

/// `audit_events.tool_name` du registre des têtes sur lesquelles mika-qa a
/// statué (mika#2917, vol 2 ; vocabulaire corrigé par mika#2927).
///
/// Cible [`head_key`] ; `after_value` vaut l'une des trois valeurs de
/// [`REVIEWED_HEAD_OUTCOMES`].
///
/// **SOLE WRITER** : [`crate::tools::record_reviewed_head`], par appel d'outil du
/// modèle. Aucun chemin du moteur ne l'écrit.
///
/// **Lecteur** : aucun dans ce vol ; le pré-filtre au vol 3.
pub(crate) const QA_REVIEWED_HEAD_TOOL: &str = "qa_reviewed_head";

/// Les trois issues qui attestent que mika-qa a **statué** sur une tête — et ce
/// que chacune atteste, par quel tour.
///
/// - `verdict_posted` — **le tour qui a posté la revue sur la forge**, après un
///   `run_gh("pr review …")` réussi : `qa-review` à son étape 5b, et
///   `qa-review-build-callback` à la sienne quand le verdict part du tour de
///   rappel du build. C'est le seul des trois à attester un verdict ; les deux
///   autres attestent qu'il en existait déjà un.
/// - `stop_already_reviewed_sha` — **le tour webhook** de
///   `qa-review-webhook-success`, arrêté à son étape 3 parce qu'une de ses
///   propres revues porte déjà ce `commit_id`.
/// - `stop_verdict_recorded` — **le tour webhook**, arrêté à son étape 4 parce
///   qu'un verdict faisant autorité est déjà enregistré sur cette tête.
///
/// **L'asymétrie avec l'attestation de mika#2855 est voulue, pas un oubli.** Les
/// deux `stop_*` sont aussi deux des six valeurs `Webhook-Outcome:`, et un tour
/// webhook écrit donc ici la même valeur que sa ligne d'attestation.
/// `verdict_posted` n'a **aucune** contrepartie dans ces six valeurs, parce que
/// le tour qui poste un verdict n'est pas un tour webhook et n'émet aucune ligne
/// `Webhook-Outcome:` : les deux vocabulaires mesurent deux populations
/// différentes et ne se recouvrent qu'en partie. Ajouter `verdict_posted` aux six
/// serait inventer une issue de webhook qui n'existe pas.
///
/// Les issues qui ne concluent sur aucune tête n'écrivent rien, et ce
/// vocabulaire fermé est ce qui rend leur écriture non représentable :
/// `stop_no_open_pr` et `stop_out_of_scope` n'ont pas de PR nommable ou sont hors
/// périmètre, `review_failed` est précisément le cas où la revue n'a **pas**
/// abouti — donc celui où le tour suivant doit passer — et la graphie `review`,
/// celle du dispatch, a quitté ce tableau avec mika#2927 : une revue dépêchée
/// n'est pas une tête jugée. Voir § *La perte de revue*.
pub(crate) const REVIEWED_HEAD_OUTCOMES: &[&str] = &[
    "verdict_posted",
    "stop_already_reviewed_sha",
    "stop_verdict_recorded",
];

/// La profondeur, en jours, sur laquelle le registre se lit (mika#2917, vol 3).
///
/// **Une propriété du registre, pas du pré-filtre**, et déclarée ici pour cette
/// raison : c'est la durée pendant laquelle une ligne écrite par le skill vaut
/// encore « cette tête est traitée ». Un second lecteur hériterait du même
/// horizon sans avoir à le redécider.
///
/// Deux bornes la fixent, et elles ne disent pas la même chose :
///
/// - **Au-dessus**, [`crate::evidence::audit::AUDIT_RETENTION_DAYS`] (90) :
///   `compact_old_audit_events` replie les lignes plus vieilles en résumé
///   mensuel, donc une fenêtre plus large lirait sur des lignes qui n'existent
///   plus — une recherche qui cesse silencieusement de trouver, la classe que
///   mika#2242 avait déjà payée.
/// - **En dessous**, le coût d'une revue perdue est nul et celui d'un tour payé
///   vaut un tour : trente jours, parce qu'une `check_suite` sur une tête d'il y
///   a plus d'un mois est une PR laissée ouverte un mois, et que faire passer ce
///   tour est le sens sûr (§ *La perte de revue*, point 4). Une fenêtre trop
///   courte ne perd jamais de revue ; elle ne fait que rendre le tour.
pub(crate) const REVIEWED_HEAD_WINDOW_DAYS: i64 = 30;

/// La borne basse de la lecture du registre, dans la graphie
/// qu'`audit_events.created_at` porte.
///
/// Rendue par une fonction et non par une constante : la borne est relative à
/// l'instant de la lecture, et [`crate::timestamp::now_minus`] est le seul site
/// du crate qui sache l'écrire.
pub(crate) fn reviewed_head_lookup_since() -> String {
    crate::timestamp::now_minus(chrono::Duration::days(REVIEWED_HEAD_WINDOW_DAYS))
}

/// L'issue telle que l'attestation la porte, ou `None` sur toute autre valeur —
/// l'outil rend alors une erreur et n'écrit pas.
///
/// **Pure, et rend le littéral canonique** de [`REVIEWED_HEAD_OUTCOMES`] plutôt
/// que l'entrée de l'appelant : un `after_value` ne doit pas pouvoir porter une
/// graphie (casse, espaces) que le lecteur du vol 3 ne reconnaîtrait pas. La
/// comparaison se fait sur `trim()` + `to_ascii_lowercase()` ; la valeur rendue
/// vient du tableau, jamais de `raw`.
pub(crate) fn reviewed_head_outcome(raw: &str) -> Option<&'static str> {
    let normalised = raw.trim().to_ascii_lowercase();
    REVIEWED_HEAD_OUTCOMES
        .iter()
        .copied()
        .find(|canonical| *canonical == normalised)
}

// ---- La preuve du tour (mika#2934, R3) -----------------------------------

/// L'outil de lecture head-matched : le seul qui dise sur quelle tête une revue
/// a statué.
///
/// Source de vérité du nom : `skills/bundled/qa-review-webhook-success/tools.json`.
/// Même idiome que [`crate::qa_build_callback::BUILD_MIKA_TOOL`] — un nom
/// d'outil de skill se déclare une fois côté Rust, pour qu'un littéral ne se
/// disperse pas sur les sites qui le testent.
pub(crate) const QA_HEAD_REVIEWS_TOOL: &str = "qa_head_reviews";

/// Le nom, côté modèle, de l'outil d'écriture du registre.
///
/// Distinct de [`QA_REVIEWED_HEAD_TOOL`], qui est le `tool_name` de la ligne
/// d'audit : l'un est ce que le modèle appelle, l'autre ce que la base porte.
/// Déclaré ici parce que depuis mika#2934 ce nom n'est plus seulement **servi**
/// (`crate::tools::record_reviewed_head`, dans `name()` et `definition()`) : il
/// est aussi **testé**, par le site de refus de `crate::tool_execution::dispatch`
/// qui doit reconnaître l'appel avant de l'exécuter. Un nom recopié d'un côté du
/// moteur à l'autre est une garde qui se désarme au premier renommage, en silence
/// et avec tout le reste vert (classe mika#2158).
///
/// Ne couvre délibérément **pas** les deux tables de noms d'outils
/// (`tools::mod::…`, `tools::classification`), où le littéral est l'idiome de la
/// table entière : les router par constante demanderait de le faire pour les
/// soixante autres outils, ou d'en singulariser un sans raison.
pub(crate) const RECORD_REVIEWED_HEAD_TOOL: &str = "record_reviewed_head";

/// Ce que ce tour n'a pas fait, et qui interdit la ligne qu'il demande.
///
/// Les deux variantes sont les deux défauts mesurés le 2026-10-27 (mika#2934),
/// et elles ne se recouvrent pas : la première dit qu'un verdict a été attesté
/// avant d'être posté, la seconde qu'une tête a été déclarée conclue sur une
/// preuve qui n'est pas la sienne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreuveManquante {
    /// `verdict_posted` sans `run_gh pr review` réussi dans ce tour.
    ///
    /// Le défaut 2, n=1 : le 2026-10-27 à 07:07:19Z la session `7418ef0c` a
    /// écrit `verdict_posted` sur `#2931@1f649bf1` alors qu'elle venait de
    /// lancer un build et attendait son callback.
    VerdictSansPost,
    /// Un `stop_*` sans lecture head-matched des revues de CETTE tête.
    ///
    /// Le défaut 1, n=3/3 : trois têtes fraîchement poussées déclarées déjà
    /// jugées sur la foi d'un `reviewDecision: APPROVED` porté par la tête
    /// précédente — un état de PR qui survit au force-push.
    StopSansLectureDeTete,
}

/// La preuve que cette `outcome` exige, est-elle **absente de ce tour** ?
///
/// `None` veut dire « la ligne demandée est couverte par ce que ce tour a
/// réellement fait ». Une preuve par valeur, et les trois valeurs en ont une :
///
/// | `outcome` | preuve exigée dans ce tour |
/// |---|---|
/// | `verdict_posted` | un `run_gh pr review` réussi |
/// | `stop_already_reviewed_sha` | un [`QA_HEAD_REVIEWS_TOOL`] réussi nommant `head_sha` |
/// | `stop_verdict_recorded` | idem |
///
/// Trois propriétés, chacune parce que son contraire était un défaut :
///
/// - **Un `outcome` hors vocabulaire rend `None`.** Le refus de valeur
///   appartient à [`reviewed_head_outcome`] et au schéma de l'outil ; ce
///   prédicat ne redouble pas ce jugement, il ne répond qu'à « la preuve
///   est-elle là ». Deux juges d'une même valeur divergent (classe mika#2158).
/// - **[`pr_review_posted_in_turn`] est appelée, pas recopiée.** Son
///   doc-comment porte déjà la raison (mika#2355), et elle est publique.
/// - **La correspondance de la tête passe par `input_summary`**, qui est
///   tronqué à `crate::planning::policy::INPUT_SUMMARY_MAX` et scrubbé. Un SHA
///   de 40 caractères dans un objet à trois champs y tient très largement ; si
///   une évolution de cette borne le mettait en danger, c'est un test qui doit
///   rougir, pas une troncature silencieuse.
///
/// **Pure**, et sans autorité : elle dit ce qui manque, le site (`dispatch.rs`)
/// décide quoi en faire.
pub(crate) fn preuve_du_tour_manquante(
    outcome: &str,
    head_sha: &str,
    deja_execute: &[ToolCallSummary],
) -> Option<PreuveManquante> {
    match reviewed_head_outcome(outcome)? {
        "verdict_posted" if !pr_review_posted_in_turn(deja_execute) => {
            Some(PreuveManquante::VerdictSansPost)
        }
        "verdict_posted" => None,
        // Les deux `stop_*` attestent le même fait — « un verdict existait déjà
        // sur CETTE tête » — et exigent donc la même preuve. Le bras large est
        // délibéré : une quatrième valeur ajoutée à [`REVIEWED_HEAD_OUTCOMES`]
        // sans preuve propre tomberait ici et serait **refusée** faute de
        // lecture de tête, jamais écrite sans preuve. C'est le fail-safe du
        // module (point 4 du § *La perte de revue*) : une ligne manquante coûte
        // un tour payé, une ligne fausse coûte une revue.
        _ if lecture_de_tete_dans_le_tour(head_sha, deja_execute) => None,
        _ => Some(PreuveManquante::StopSansLectureDeTete),
    }
}

/// Ce tour a-t-il lu les revues de **cette** tête, avec succès ?
///
/// La tête est comparée en ASCII minuscule : le handler n'accepte que 40
/// caractères hex minuscules, mais une graphie majuscule venue du modèle doit
/// satisfaire la preuve plutôt que de la perdre.
///
/// Une tête vide ne satisfait **aucune** preuve — `contains("")` est vrai de
/// toute chaîne, et une tête non nommée serait alors attestée par n'importe
/// quel appel. L'outil refuse déjà ce cas en amont ; le prédicat ne s'y fie pas.
fn lecture_de_tete_dans_le_tour(head_sha: &str, deja_execute: &[ToolCallSummary]) -> bool {
    let tete = head_sha.trim().to_ascii_lowercase();
    if tete.is_empty() {
        return false;
    }
    deja_execute.iter().any(|resume| {
        resume.name == QA_HEAD_REVIEWS_TOOL
            && resume.success
            && resume.input_summary.to_ascii_lowercase().contains(&tete)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::source_scan::{
        audit_writers, fn_bodies, production_files, production_half, strip_comment_lines,
    };
    use crate::test_utils::test_helpers::TestHarness;
    use crate::tools::default_tools;

    // ---- le vocabulaire fermé ---------------------------------------------

    #[test]
    fn mika2917_lissue_rendue_est_le_litteral_canonique() {
        for canonical in REVIEWED_HEAD_OUTCOMES {
            assert_eq!(reviewed_head_outcome(canonical), Some(*canonical));
        }
        // Normalisation : la graphie de l'appelant ne ressort jamais.
        assert_eq!(
            reviewed_head_outcome("  VERDICT_POSTED  "),
            Some("verdict_posted")
        );
        assert_eq!(
            reviewed_head_outcome("Stop_Verdict_Recorded"),
            Some("stop_verdict_recorded")
        );
    }

    /// Les trois issues de mika#2855 qui n'écrivent rien ne sont pas
    /// représentables. C'est la propriété 2 du § *La perte de revue* : si l'une
    /// d'elles devenait acceptable, une revue qui n'a pas abouti déclarerait sa
    /// tête traitée et le vol 3 écarterait les `check_suite` suivants.
    #[test]
    fn mika2917_les_trois_issues_qui_necrivent_pas_sont_refusees() {
        for refused in ["stop_no_open_pr", "stop_out_of_scope", "review_failed"] {
            assert_eq!(
                reviewed_head_outcome(refused),
                None,
                "{refused} ne doit pas être représentable comme une écriture"
            );
        }
        assert_eq!(reviewed_head_outcome(""), None);
        assert_eq!(reviewed_head_outcome("reviewed"), None);
    }

    /// Point 4 de la doc de module : ce fichier ne retape pas la grammaire de
    /// clé, il réexporte [`head_key`]. Le scan lit la **moitié production** —
    /// l'assertion elle-même nomme le préfixe interdit, et un scan du fichier
    /// entier s'accuserait.
    #[test]
    fn mika2917_le_module_ne_retape_pas_la_grammaire_de_cle() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/qa_reviewed_head.rs"),
        )
        .unwrap();
        let code = strip_comment_lines(production_half(&src));
        assert!(
            !code.contains("\"pr:"),
            "la grammaire de clé est retapée ici ; réexporter head_key"
        );
        assert!(
            !code.contains("format!"),
            "aucun format! ici : la clé vient de head_key"
        );
        // Et le réexport rend bien la forme que le pré-filtre relit.
        assert_eq!(
            head_key("senara-solutions/mika", 2917, "deadbeef"),
            String::from("pr:") + "senara-solutions/mika#2917@deadbeef"
        );
    }

    // ---- Fire-Disposition (a.1) : les sites `gh` de `webhook_prefilter.rs` --

    /// Les sites de `webhook_prefilter.rs` qui appellent `gh`. Une entrée par
    /// site, nommant la donnée précise qu'il lit et le ticket qui le retire.
    ///
    /// - `pr_state` — lit le seul champ `state` d'une PR (`gh pr view --json
    ///   state`, via `tools::pr_merge_with_gate::run_gh_subprocess`) pour la
    ///   classe (c) `VerdictPrClosed`. **Antérieur à mika#2917** et hors du
    ///   chemin (g′) : aucun décideur de `GreenPrBranch` ne l'atteint. Son
    ///   retrait est l'achèvement de mika#2260, suivi par
    ///   senara-solutions/mika#2260.
    const GH_CALL_SITES_ALLOWED: &[&str] = &["pr_state"];

    /// Un corps de fonction nomme-t-il un jeton d'appel `gh` ?
    ///
    /// Trois jetons : `run_gh_subprocess`, `run_gh` et le littéral `"gh"`. Les
    /// deux premiers se testent par un seul `contains` — le second est préfixe du
    /// premier.
    fn names_a_gh_call(body: &str) -> bool {
        body.contains("run_gh") || body.contains("\"gh\"")
    }

    /// `(sites nommant `gh`, nombre de corps énumérés)` sur une source donnée.
    /// Le second terme est ce qui rend l'anti-vacuité lisible : « aucun site
    /// trouvé » sur zéro corps lu n'est pas une mesure (mika#2205).
    fn gh_call_sites_of(src: &str) -> (Vec<String>, usize) {
        let bodies = fn_bodies(production_half(src));
        let enumerated = bodies.len();
        let mut found: Vec<String> = bodies
            .into_iter()
            .filter(|(_, body)| names_a_gh_call(body))
            .map(|(name, _)| name)
            .collect();
        found.sort();
        found.dedup();
        (found, enumerated)
    }

    /// L'assertion est **auto-nettoyante** : elle compare l'ensemble exact, pas
    /// un `contains` ni un `>=`. Le jour où `pr_state` cesse d'appeler `gh`,
    /// l'entrée devient stale et ce test rougit en nommant l'entrée à retirer —
    /// le geste qui rend l'exception inutile est celui qui la fait expirer, ce
    /// qui est ce qui empêche cette allowlist de devenir celle que mika#2309
    /// condamne.
    #[test]
    fn mika2917_a1_les_sites_gh_du_prefiltre_sont_recenses() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/webhook_prefilter.rs"),
        )
        .unwrap();
        let (found, enumerated) = gh_call_sites_of(&src);

        // Anti-vacuité : le scan a bien lu le fichier réel.
        assert!(
            enumerated >= 20,
            "anti-vacuité : {enumerated} corps énumérés, le scan n'a rien lu"
        );
        assert_eq!(
            found,
            GH_CALL_SITES_ALLOWED
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
            "ensemble exact : un site apparu s'ajoute en nommant sa donnée et son \
             ticket de retrait ; un site disparu se retire de GH_CALL_SITES_ALLOWED"
        );
    }

    /// Contrôle négatif — sans lui, « le scan n'a rien trouvé » est
    /// indistinguable de « le scan n'a rien lu ».
    #[test]
    fn mika2917_a1_le_scan_gh_mord_hors_allowlist() {
        let fixture = "\
async fn decide_something(&self) -> bool {\n    true\n}\n\
async fn fetch_the_world(&self, repo: &str) -> bool {\n    \
let args = [\"pr\", \"view\"];\n    run_gh_subprocess(&args, token).await\n}\n";
        let (found, enumerated) = gh_call_sites_of(fixture);
        assert_eq!(enumerated, 2);
        assert_eq!(found, vec!["fetch_the_world".to_string()]);
        assert!(!GH_CALL_SITES_ALLOWED.contains(&"fetch_the_world"));
    }

    // ---- Fire-Disposition (a.2) : un écrivain, au plus un lecteur ----------

    /// Les fichiers de production qui **nomment** la constante sans l'écrire —
    /// le module de vérité exclu, puisqu'il la déclare.
    fn readers_of(files: &[(String, String)], needle: &str, writers: &[String]) -> Vec<String> {
        let mut out: Vec<String> = files
            .iter()
            .filter(|(rel, _)| rel != "qa_reviewed_head.rs")
            .filter(|(rel, prod)| prod.contains(needle) && !writers.contains(rel))
            .map(|(rel, _)| rel.clone())
            .collect();
        out.sort();
        out
    }

    /// L'invariant du bearing — « le moteur ne l'écrit jamais » — et la lecture
    /// unique qu'AC3 nomme.
    ///
    /// Les deux assertions n'ont **pas** la même forme, parce qu'elles ne
    /// mesurent pas la même chose :
    ///
    /// - **L'écrivain, ensemble exact.** Stable à travers les vols 2 et 3 : le
    ///   vol 3 ajoute un lecteur, pas un écrivain. Au vol 2 l'ensemble est vide
    ///   avant que `tools/record_reviewed_head.rs` n'existe, et c'est le seul
    ///   instant du vol où il l'est.
    /// - **Le lecteur, borné à un.** `≤ 1` délibérément, et non un ensemble
    ///   exact : un ensemble exact rougirait au vol 3 **par construction**, et un
    ///   rouge attendu s'apprend à ignorer. Le `≤ 1` mord sur le seul défaut
    ///   qu'AC3 nomme — « le Rust ne lit qu'une seule source » — et il mord aux
    ///   deux vols.
    ///
    /// Quand l'une des deux rougit, la résolution est de router l'accès par ce
    /// module, **jamais** d'ajouter une entrée : il n'y a pas d'allowlist ici.
    #[test]
    fn mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur() {
        let files = production_files();

        // Anti-vacuité : le scan a bien lu l'arbre.
        assert!(
            files.len() >= 100,
            "anti-vacuité : {} fichiers lus, le scan n'a rien lu",
            files.len()
        );

        let writers = audit_writers("QA_REVIEWED_HEAD_TOOL");
        assert_eq!(
            writers,
            vec!["tools/record_reviewed_head.rs".to_string()],
            "SOLE WRITER : un seul écrivain. Quand cet ensemble grandit, route \
             l'écriture par l'outil — n'ajoute pas d'entrée"
        );

        let readers = readers_of(&files, "QA_REVIEWED_HEAD_TOOL", &writers);
        assert!(
            readers.len() <= 1,
            "au plus un lecteur ; trouvés : {readers:?}"
        );

        // Aucun littéral recopié hors de ce module.
        let literal = String::from("\"") + QA_REVIEWED_HEAD_TOOL + "\"";
        for (rel, prod) in &files {
            if rel == "qa_reviewed_head.rs" {
                continue;
            }
            assert!(
                !prod.contains(&literal),
                "{rel} recopie le littéral {literal} ; passer par la constante"
            );
        }
    }

    /// Contrôle négatif — deux écrivains sur une source fabriquée. Il inline la
    /// fenêtre de lecture de bonne foi, comme
    /// `qa_head_supersession::tests::mika2671_le_scan_decrivain_mord` : ce que le
    /// contrôle vérifie est que la fenêtre mord, pas que le prédicat promu se
    /// paramètre.
    #[test]
    fn mika2917_a2_le_scan_decrivain_mord() {
        let fixture = "db.log_audit_event(\n    \"system\",\n    QA_REVIEWED_HEAD_TOOL,\n";
        let lines: Vec<&str> = fixture.lines().collect();
        assert!(lines.iter().enumerate().any(|(i, l)| {
            l.contains("log_audit_event(")
                && lines[i..(i + 5).min(lines.len())]
                    .iter()
                    .any(|w| w.contains("QA_REVIEWED_HEAD_TOOL"))
        }));
    }

    /// Contrôle négatif — deux lecteurs sur un arbre fabriqué.
    #[test]
    fn mika2917_a2_le_scan_de_lecteur_mord() {
        let files = vec![
            (
                "qa_reviewed_head.rs".to_string(),
                "const QA_REVIEWED_HEAD_TOOL: &str = \"x\";".to_string(),
            ),
            (
                "server/webhook_prefilter.rs".to_string(),
                "if tool == QA_REVIEWED_HEAD_TOOL {}".to_string(),
            ),
            (
                "server/handlers.rs".to_string(),
                "let n = QA_REVIEWED_HEAD_TOOL;".to_string(),
            ),
        ];
        let readers = readers_of(&files, "QA_REVIEWED_HEAD_TOOL", &[]);
        assert_eq!(
            readers,
            vec![
                "server/handlers.rs".to_string(),
                "server/webhook_prefilter.rs".to_string()
            ]
        );
        assert!(readers.len() > 1, "le scan doit mordre sur deux lecteurs");
    }

    // ---- conformité des prompts -------------------------------------------

    /// Les prompts **embarqués**, pas ceux de l'arbre : `include_str!` fige les
    /// octets au build, exactement comme `build.rs` les fige dans
    /// `BUNDLED_SKILL_MANIFESTS`. Une édition de l'arbre est invisible aux deux
    /// jusqu'au prochain `make deploy` (motif mika#2340). Même idiome que
    /// `server::webhook_turn_outcome::tests::EMBEDDED_PROMPT`, sur l'un des
    /// mêmes fichiers et pour la même raison.
    const PROMPT_WEBHOOK: &str =
        include_str!("../../../skills/bundled/qa-review-webhook-success/system_prompt.md");
    const PROMPT_QA_REVIEW: &str =
        include_str!("../../../skills/bundled/qa-review/system_prompt.md");
    const PROMPT_BUILD_CALLBACK: &str =
        include_str!("../../../skills/bundled/qa-review-build-callback/system_prompt.md");

    /// Les trois prompts qui écrivent au registre, et l'ensemble **exact** des
    /// issues que chacun doit passer à l'outil.
    ///
    /// La table est le partage que mika#2927 institue : un tour webhook atteste
    /// un arrêt, un tour de verdict atteste un verdict posté. Avant, un seul
    /// prompt portait les trois issues — dont `review`, écrit au **dispatch**
    /// d'une revue qui pouvait mourir avant son verdict (incident du 2026-10-26,
    /// session `4f94fbc8`).
    const PROMPTS_ECRIVANTS: &[(&str, &str, &[&str])] = &[
        (
            "qa-review-webhook-success",
            PROMPT_WEBHOOK,
            &["stop_already_reviewed_sha", "stop_verdict_recorded"],
        ),
        ("qa-review", PROMPT_QA_REVIEW, &["verdict_posted"]),
        (
            "qa-review-build-callback",
            PROMPT_BUILD_CALLBACK,
            &["verdict_posted"],
        ),
    ];

    /// Le prompt embarqué d'un des trois écrivains, par son nom de skill.
    ///
    /// N'existe que pour la boucle d'auto-nettoyage de
    /// [`PROMPT_OUTCOME_ALLOWLIST`] : une exception nomme un fichier, et c'est
    /// dans ce fichier que sa péremption se lit.
    fn prompt_de(fichier: &str) -> &'static str {
        PROMPTS_ECRIVANTS
            .iter()
            .find(|(nom, _, _)| *nom == fichier)
            .map(|(_, prompt, _)| *prompt)
            .unwrap_or_else(|| panic!("{fichier} n'est pas l'un des prompts écrivants"))
    }

    /// Lignes de prompt soustraites au scan d'issues, par `(fichier, ligne)`.
    ///
    /// **LIVRÉE VIDE**, et c'est la Fire-Disposition : le détecteur atterrit
    /// armé, sans exception à son nom. Précédent et idiome :
    /// `server::webhook_turn_outcome::tests::PROMPT_PARITY_ALLOWLIST`, sur l'un
    /// des mêmes fichiers.
    ///
    /// Elle est **auto-nettoyante** : la boucle de péremption du test de
    /// conformité exige que chaque ligne exemptée soit encore dans son prompt.
    /// Le geste qui rend une exception inutile est donc le geste qui la fait
    /// expirer — ce qui sépare ceci de l'allowlist que mika#2309 condamne.
    const PROMPT_OUTCOME_ALLOWLIST: &[(&str, &str)] = &[];

    /// Les issues que le prompt passe à l'outil : la valeur entre guillemets de
    /// chaque `outcome: "…"` porté par une ligne qui **appelle** l'outil.
    ///
    /// Le prédicat est sorti du test pour que le contrôle de bonne foi
    /// l'exerce sur un prompt synthétique. Il lit la valeur citée plutôt que de
    /// chercher le littéral dans la ligne, parce que `review` est un
    /// sous-mot de `review_failed` **et** de `stop_already_reviewed_sha` : un
    /// `contains` rendrait un ensemble faux sans jamais rougir.
    fn issues_appelees(prompt: &str) -> Vec<String> {
        let mut found: Vec<String> = prompt
            .lines()
            .filter(|line| line.contains("record_reviewed_head("))
            .flat_map(|line| line.split("outcome: \"").skip(1))
            .filter_map(|rest| rest.split('"').next())
            .map(|v| v.to_string())
            .collect();
        found.sort();
        found.dedup();
        found
    }

    /// Les issues qu'un prompt passe à l'outil, exceptions retirées.
    ///
    /// Le retrait est une soustraction de **lignes** avant le scan, pour que
    /// [`issues_appelees`] reste le prédicat unique et que le contrôle négatif
    /// continue de l'exercer nu.
    fn issues_appelees_hors_exceptions(fichier: &str, prompt: &str) -> Vec<String> {
        let retenu = prompt
            .lines()
            .filter(|line| {
                !PROMPT_OUTCOME_ALLOWLIST
                    .iter()
                    .any(|(f, exc)| *f == fichier && line.contains(exc))
            })
            .collect::<Vec<_>>()
            .join("\n");
        issues_appelees(&retenu)
    }

    /// Chacun des trois prompts appelle l'outil pour **exactement** les issues
    /// de son tour, et leur union est **exactement** le vocabulaire du registre.
    ///
    /// La propriété tenue n'est pas « les prompts mentionnent l'outil » mais
    /// « le vocabulaire des prompts et celui du registre ne divergent pas », et
    /// elle est dite dans les deux sens, ce qui est ce qui l'empêche d'être
    /// l'allowlist que mika#2309 condamne :
    ///
    /// - **Par fichier, égalité d'ensemble.** Une issue de trop serait refusée à
    ///   l'exécution ; une issue de moins serait une perte de dedup silencieuse
    ///   — un tour payé par `check_suite` sur cette tête, sans rien qui le dise.
    ///   Et sur le prompt webhook, `verdict_posted` de trop serait le défaut de
    ///   mika#2927 réintroduit sous un autre nom : le tour qui dépêche n'est pas
    ///   celui qui statue.
    /// - **En union, égalité avec [`REVIEWED_HEAD_OUTCOMES`].** Une quatrième
    ///   issue ajoutée au registre sans prompt qui l'écrive serait du
    ///   vocabulaire mort ; un littéral retiré du registre sans retrait du
    ///   prompt serait un appel voué au refus.
    #[test]
    fn mika2927_les_trois_prompts_appellent_loutil_aux_issues_de_leur_tour() {
        let mut union: Vec<String> = Vec::new();

        for &(fichier, prompt, attendues) in PROMPTS_ECRIVANTS {
            let appelees = issues_appelees_hors_exceptions(fichier, prompt);
            let mut attendues: Vec<String> = attendues.iter().map(|s| s.to_string()).collect();
            attendues.sort();
            assert_eq!(
                appelees, attendues,
                "{fichier} : les issues qu'il passe à l'outil ne sont pas celles de son tour"
            );

            // Chaque issue appelée est bien l'une que l'outil accepte : la
            // réciproque de l'égalité ci-dessus, dite par le prédicat de vérité
            // lui-même plutôt que par l'égalité de deux listes.
            for issue in &appelees {
                assert_eq!(
                    reviewed_head_outcome(issue),
                    Some(issue.as_str()),
                    "{fichier} passe {issue:?}, que l'outil refuserait"
                );
            }
            union.extend(appelees);
        }

        union.sort();
        union.dedup();
        let mut vocabulaire: Vec<String> = REVIEWED_HEAD_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        vocabulaire.sort();
        assert_eq!(
            union, vocabulaire,
            "l'union des trois prompts et le vocabulaire du registre divergent"
        );

        // Le défaut de mika#2927, dit sur le prompt où il vivait et **sans**
        // soustraire d'exception : aucune ligne du tour webhook, exemptée ou
        // non, ne passe l'issue du tour qui poste le verdict.
        assert!(
            !issues_appelees(PROMPT_WEBHOOK).contains(&"verdict_posted".to_string()),
            "le tour webhook dépêche la revue, il ne la conclut pas : \
             `verdict_posted` ne s'écrit pas là"
        );

        // Les six valeurs de l'attestation mika#2855 restent dans le prompt
        // webhook : la section corrigée n'en remplace aucune, et `review` reste
        // une issue d'attestation alors qu'elle a quitté le registre.
        for ligne in [
            "Webhook-Outcome: stop_no_open_pr",
            "Webhook-Outcome: stop_out_of_scope",
            "Webhook-Outcome: stop_already_reviewed_sha",
            "Webhook-Outcome: stop_verdict_recorded",
            "Webhook-Outcome: review",
            "Webhook-Outcome: review_failed",
        ] {
            assert!(
                PROMPT_WEBHOOK.contains(ligne),
                "la section corrigée a fait perdre {ligne:?} au prompt"
            );
        }

        // Auto-nettoyage : une exception dont la ligne a quitté son prompt est
        // une exception qui ne protège plus rien.
        for (fichier, ligne) in PROMPT_OUTCOME_ALLOWLIST {
            assert!(
                prompt_de(fichier).contains(ligne),
                "exception stale dans PROMPT_OUTCOME_ALLOWLIST : {ligne:?} n'est \
                 plus dans {fichier} — retirer l'entrée"
            );
        }
    }

    /// Contrôle négatif — un prompt synthétique qui passerait à l'outil une
    /// issue hors contrat. Une égalité d'ensemble sur un prédicat inerte est
    /// indiscernable d'une égalité vraie.
    #[test]
    fn mika2917_le_scan_du_prompt_mord_sur_une_issue_hors_contrat() {
        let fabrique = concat!(
            "| step 1 | `record_reviewed_head(repo: \"r\", pr: 1, head_sha: \"s\", ",
            "outcome: \"stop_no_open_pr\")` |\n",
            "| step 5 | `record_reviewed_head(repo: \"r\", pr: 1, head_sha: \"s\", ",
            "outcome: \"review\")` |\n",
            "Une ligne qui nomme outcome: \"review_failed\" sans appeler l'outil.\n",
        );
        assert_eq!(
            issues_appelees(fabrique),
            vec!["review".to_string(), "stop_no_open_pr".to_string()],
            "le scan doit voir l'issue hors contrat, et ignorer la ligne qui n'appelle pas"
        );
    }

    /// mika#2927 — la graphie du dispatch a **quitté** le vocabulaire.
    ///
    /// C'est la moitié du correctif qui traite aussi la ligne déjà inscrite,
    /// sans SQL : `reviewed_head_outcome("review")` rendant `None`, la condition
    /// d'écart du lecteur du vol 3 (`reviewed_head_outcome(&raw).is_some()`) est
    /// fausse sur cette ligne, qui tombe hors contrat et est relâchée par sa
    /// règle `reviewed_head_out_of_contract` existante.
    #[test]
    fn mika2927_le_vocabulaire_du_registre_nadmet_plus_la_graphie_du_dispatch() {
        assert_eq!(
            reviewed_head_outcome("review"),
            None,
            "`review` attestait un dispatch, pas un verdict : le registre la refuse"
        );
        // Et la normalisation ne la ramène pas par la porte de service.
        assert_eq!(reviewed_head_outcome("  REVIEW  "), None);
        assert!(
            !REVIEWED_HEAD_OUTCOMES.contains(&"review"),
            "`review` ne fait plus partie du vocabulaire du registre"
        );
        assert!(
            REVIEWED_HEAD_OUTCOMES.contains(&"verdict_posted"),
            "`verdict_posted` la remplace"
        );
    }

    /// Le bloc *Step 5b* du prompt de callback, de son titre jusqu'à la section
    /// suivante. Découpé en lignes et non en octets : le bloc contient des
    /// caractères multi-octets (`…`, `—`), et `scripts/check-byte-slices.sh`
    /// refuse le découpage d'octets précisément pour cette raison.
    fn bloc_step_5b(prompt: &str) -> Vec<&str> {
        prompt
            .lines()
            .skip_while(|l| !l.starts_with("**Step 5b — Record the head you ruled on"))
            .take_while(|l| !l.starts_with("### Verdict Output"))
            .collect()
    }

    /// Les clauses que chaque prompt de verdict doit porter pour conditionner
    /// l'appel au **post réussi**.
    ///
    /// **La forme diffère entre les deux fichiers, et c'est voulu, pas une
    /// dérive.** `qa-review/system_prompt.md` tient contre la porte de taille de
    /// mika#852 : son `max_prompt_size` est **déjà** à
    /// `MAX_PROMPT_SIZE_CEILING`, donc la remédiation « lever le plafond » y est
    /// épuisée au sens littéral (le commentaire de son `skill.toml` porte
    /// l'arithmétique). La règle y vit en **invariant 7 de l'auto-contrôle de
    /// fin de tour du Step 5** — son foyer naturel, pas seulement le moins
    /// cher : l'invariant 6 énonce déjà la condition du `pr review` réussi sur
    /// laquelle l'invariant 7 s'accroche. `qa-review-build-callback` a 9 Ko de
    /// marge et porte la section entière.
    ///
    /// Ce qui s'assère donc, c'est la **clause** par fichier, jamais l'identité
    /// octet à octet des deux textes.
    const CLAUSES_DU_POST_REUSSI: &[(&str, &[&str])] = &[
        (
            "qa-review",
            &[
                "after invariant 6 returned success",
                "If invariant 6 did not succeed",
                "record **nothing**",
                "outcome: \"verdict_posted\")",
            ],
        ),
        (
            "qa-review-build-callback",
            &[
                "Once the `run_gh(\"pr review …\")` call of Step 5 has returned success",
                "The condition is the post's success, literally:",
                "record **nothing**",
                "outcome: \"verdict_posted\")",
            ],
        ),
    ];

    /// Les deux prompts de verdict conditionnent l'appel au **post réussi**.
    ///
    /// **Détecteur faible, et dit comme tel.** Il tient la *présence* de la
    /// condition dans le texte, jamais l'obéissance du modèle : un prompt peut
    /// porter la phrase et un tour peut l'ignorer. La tenue réelle du moteur est
    /// ailleurs — la garde `qa_build_callback_verdict` sur le tour de callback —
    /// et R2 borne le dégât restant : la seule issue que l'outil accepte sur un
    /// tour de verdict atteste un verdict, donc une écriture sans post reste une
    /// ligne de trop, pas une ligne fausse sur ce qu'elle nomme.
    ///
    /// Les trois assertions de fin, en revanche, sont fortes, et chacune couvre
    /// une forme que les autres ne voient pas : la section du callback ne peut
    /// pas fondre à deux lignes ; l'invariant de `qa-review` ne peut pas quitter
    /// la liste numérotée du Step 5 pour une prose que le modèle lit sans la
    /// dérouler ; et `qa-review` ne peut pas regagner la section autonome qui a
    /// rougi la porte de taille, parce que le bon réflexe d'un futur éditeur —
    /// « uniformiser les deux prompts » — est exactement la régression.
    #[test]
    fn mika2927_les_deux_prompts_de_verdict_conditionnent_lappel_au_post_reussi() {
        for &(fichier, clauses) in CLAUSES_DU_POST_REUSSI {
            let prompt = prompt_de(fichier);
            for litteral in clauses {
                assert!(
                    prompt.contains(litteral),
                    "{fichier} ne porte plus {litteral:?}"
                );
            }
        }

        let callback = bloc_step_5b(PROMPT_BUILD_CALLBACK);
        assert!(
            callback.len() >= 6,
            "anti-vacuité : le bloc Step 5b de qa-review-build-callback fait {} lignes",
            callback.len()
        );
        assert!(
            PROMPT_QA_REVIEW
                .lines()
                .any(|l| l.starts_with("7. **You have called `record_reviewed_head(")),
            "l'invariant 7 a quitté l'auto-contrôle numéroté du Step 5 de qa-review"
        );
        assert!(
            !PROMPT_QA_REVIEW.contains("**Step 5b — Record the head you ruled on"),
            "qa-review a regagné une section Step 5b autonome : la porte de \
             taille de mika#852 redeviendra rouge — lire l'arithmétique dans le \
             commentaire de son skill.toml avant de la rétablir"
        );
    }

    // ---- mika#2934 : la tête jugée sur les revues de cette tête ------------

    /// Les clauses que le prompt du webhook doit porter pour que ses étapes 3 et
    /// 4 jugent la tête sur **les revues de cette tête**.
    ///
    /// Même forme que [`CLAUSES_DU_POST_REUSSI`] : une clause par fichier,
    /// jamais l'identité octet à octet d'un texte. Les cinq littéraux sont les
    /// cinq maillons de la lecture, et aucun ne couvre les autres :
    ///
    /// - `qa_head_reviews(` — l'outil est **appelé**, pas décrit. C'est le seul
    ///   qui rende `commit_id` ; `gh pr view --json reviews` ne porte pas ce
    ///   champ, donc avant mika#2934 l'étape 3 nommait un champ que sa propre
    ///   lecture ne produisait pas — le frère silencieux du défaut 1.
    /// - `carries_verdict_line` — le prédicat de l'étape 4 porte sur un champ du
    ///   tableau rendu, et non sur un état de la PR.
    /// - `commit_id` — la raison vit dans le prompt, là où le modèle la lit :
    ///   c'est le champ qui nomme la tête à laquelle la forge attribue une revue.
    ///
    /// Les deux derniers sont ajoutés par mika#2939, qui a mesuré que
    /// `commit_id` **seul** ne nomme pas la tête jugée : au force-push la forge
    /// réattribue une revue à la nouvelle tête (#2920, une revue de 07:16:49Z
    /// porte un `commit_id` créé à 12:14Z). Les trois clauses ci-dessus restent
    /// donc toutes vraies d'un prompt qui conclut sur une revue antidatée :
    ///
    /// - `head_first_seen_at` — le repère temporel est **nommé** dans le prompt.
    ///   Sans lui, l'étape 4 lit un tableau de revues sans savoir de quand date
    ///   la tête, et le filtre pourrait être retiré de l'outil sans qu'une seule
    ///   ligne du prompt change.
    /// - `dropped_predating_head` — le compte des revues écartées est un
    ///   **instrument**, et le prompt doit dire qu'il n'est jamais un motif
    ///   d'écart : un compte non nul signifie preuve *absente*, donc revue.
    const CLAUSES_DE_LA_TETE_JUGEE: &[(&str, &[&str])] = &[(
        "qa-review-webhook-success",
        &[
            "qa_head_reviews(",
            "carries_verdict_line",
            "commit_id",
            "head_first_seen_at",
            "dropped_predating_head",
        ],
    )];

    /// Les états de PR que le prompt du webhook ne doit plus citer du tout.
    ///
    /// `reviewDecision` **survit au force-push** : rien ne révoque ici une
    /// approbation périmée, donc une ancienne tête approuvée suffisait à faire
    /// déclarer la nouvelle statuée — trois fois sur trois le 2026-10-27, de 9 à
    /// 17 minutes avant le premier verdict sur chacune.
    ///
    /// **L'absence porte sur le prompt ENTIER, l'interdiction comprise.** R2 du
    /// plan donnait au refus de l'étape 4 un libellé qui nommait le champ, et R5
    /// exige son absence : les deux ne peuvent pas être vrais ensemble. C'est
    /// l'absence qui est le terme fort — citer un nom de champ dans une
    /// interdiction le réinjecte dans le contexte du modèle, ce que *remove
    /// capability, don't prohibit it* refuse précisément. Le prompt énonce donc
    /// la propriété (« ils survivent au force-push ») sans rendre le champ
    /// nommable, et l'étape 1 ne le demande plus à la forge.
    const ETATS_DE_PR_REFUSES_A_LETAPE_4: &[&str] = &["reviewDecision"];

    /// Prompts dispensés du détecteur de la tête jugée, par `(fichier, clause)`.
    ///
    /// **LIVRÉE VIDE, et c'est le contrat.** Même forme et même raison que
    /// [`PROMPT_OUTCOME_ALLOWLIST`] : aucune violation n'existe dans le code à
    /// l'issue de mika#2934 — le prompt est corrigé dans le commit qui précède
    /// celui du détecteur, donc il n'y a aucun antécédent à excuser.
    const CONFORMITE_TETE_JUGEE_ALLOWLIST: &[(&str, &str)] = &[];

    /// Les clauses d'une table que le prompt ne porte pas, exceptions retirées.
    ///
    /// Sorti du test pour que le contrôle négatif l'exerce sur un prompt
    /// synthétique : une liste de présences tenue par un prédicat inerte est
    /// indiscernable d'une liste vraie.
    fn clauses_absentes<'a>(fichier: &str, prompt: &str, clauses: &'a [&'a str]) -> Vec<&'a str> {
        clauses
            .iter()
            .copied()
            .filter(|litteral| !prompt.contains(*litteral))
            .filter(|litteral| {
                !CONFORMITE_TETE_JUGEE_ALLOWLIST
                    .iter()
                    .any(|(f, exc)| *f == fichier && exc == litteral)
            })
            .collect()
    }

    /// **AC1.** L'étape 4 nomme la lecture qu'elle fait, et l'outil qui la donne.
    #[test]
    fn mika2934_letape_4_nomme_la_lecture_et_son_outil() {
        for &(fichier, clauses) in CLAUSES_DE_LA_TETE_JUGEE {
            assert_eq!(
                clauses_absentes(fichier, prompt_de(fichier), clauses),
                Vec::<&str>::new(),
                "{fichier} : l'étape 4 ne nomme plus la lecture de la tête, ou \
                 plus l'outil qui la donne"
            );
        }

        // Contrôle négatif — le prompt d'avant mika#2934, reconstruit depuis son
        // propre texte. Il porte `commit_id` et rien d'autre : c'est exactement
        // la forme du défaut, un prédicat qui nomme le bon champ sans tenir la
        // lecture qui le rend.
        let fabrique = concat!(
            "3. **Skip if already-reviewed at this SHA.** Fetch the latest review ",
            "by yourself via `run_gh(\"pr view <n> --json reviews\")`. If a review ",
            "exists with `commit_id == pr.headRefOid`, skip.\n",
            "4. **Skip if verdict is already recorded.** If the PR already has an ",
            "authoritative verdict tied to the current head SHA, skip.\n",
        );
        assert_eq!(
            clauses_absentes(
                "qa-review-webhook-success",
                fabrique,
                CLAUSES_DE_LA_TETE_JUGEE[0].1,
            ),
            vec![
                "qa_head_reviews(",
                "carries_verdict_line",
                "head_first_seen_at",
                "dropped_predating_head",
            ],
            "le détecteur doit voir l'absence de la lecture, et ne pas se \
             contenter du champ qu'elle rend"
        );

        // Second contrôle négatif, celui de mika#2939 : le prompt d'après
        // mika#2934, qui tient les trois premiers maillons et conclut quand même
        // sur une revue antidatée. Seules les deux clauses du repère temporel
        // manquent — et c'est précisément ce que les trois premières ne peuvent
        // pas voir.
        let apres_2934 = concat!(
            "3. **Skip if already-reviewed at this SHA.** Read the reviews of ",
            "THIS head, once: `qa_head_reviews(repo: \"<owner/repo>\", pr: <n>, ",
            "head_sha: \"<sha>\")`. It returns the reviews whose `commit_id` is ",
            "exactly this head.\n",
            "4. **Skip if verdict is already recorded.** In the array already in ",
            "hand, if an entry has `carries_verdict_line: true`, skip.\n",
        );
        assert_eq!(
            clauses_absentes(
                "qa-review-webhook-success",
                apres_2934,
                CLAUSES_DE_LA_TETE_JUGEE[0].1,
            ),
            vec!["head_first_seen_at", "dropped_predating_head"],
            "le détecteur doit voir l'absence du repère temporel sur un prompt \
             qui porte par ailleurs toute la lecture de mika#2934 — sinon il ne \
             distingue pas mika#2939 de son antécédent"
        );

        // Auto-nettoyage : une exception dont la clause est revenue dans son
        // prompt est une exception qui ne protège plus rien.
        for (fichier, clause) in CONFORMITE_TETE_JUGEE_ALLOWLIST {
            assert!(
                !prompt_de(fichier).contains(clause),
                "exception stale dans CONFORMITE_TETE_JUGEE_ALLOWLIST : {clause:?} \
                 est de nouveau dans {fichier} — retirer l'entrée"
            );
        }
    }

    /// **AC1, l'autre moitié.** Le champ du défaut n'est plus dans le prompt —
    /// ni lu à l'étape 1, ni cité par l'interdiction de l'étape 4.
    #[test]
    fn mika2934_le_prompt_du_webhook_ne_cite_plus_aucun_etat_de_pr_revocable() {
        for etat in ETATS_DE_PR_REFUSES_A_LETAPE_4 {
            assert!(
                !PROMPT_WEBHOOK.contains(etat),
                "le prompt webhook cite encore {etat:?} : un état de PR survit au \
                 force-push, et juger la tête sur lui est le défaut 1 de mika#2934"
            );
        }

        // Contrôle négatif — une copie du prompt où le champ est réinséré dans
        // la liste `--json` de l'étape 1, à la place exacte qu'il occupait.
        let reintroduit = PROMPT_WEBHOOK.replace(
            "number,mergeable,mergeStateStatus,statusCheckRollup",
            "number,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup",
        );
        assert_ne!(
            reintroduit, PROMPT_WEBHOOK,
            "anti-vacuité : la réinsertion n'a pas trouvé sa cible, donc le \
             contrôle négatif ne prouve rien — l'étape 1 a changé de forme"
        );
        assert!(
            ETATS_DE_PR_REFUSES_A_LETAPE_4
                .iter()
                .any(|etat| reintroduit.contains(etat)),
            "l'assertion ci-dessus doit rougir sur un prompt qui réintroduit le champ"
        );
    }

    /// Fire-Disposition de mika#2934, option (a) : l'allowlist atterrit vide.
    #[test]
    fn mika2934_lallowlist_de_conformite_est_livree_vide_et_le_reste() {
        assert!(
            CONFORMITE_TETE_JUGEE_ALLOWLIST.is_empty(),
            "Une entrée a été ajoutée ici. Une dispense de conformité sur ce \
             détecteur rouvre exactement le défaut 1 de mika#2934 : un prompt qui \
             juge une tête sur un état de PR révocable. Corrige le prompt, ou \
             ouvre un ticket de suivi et cite-le dans l'entrée."
        );
    }

    // ---- R7/AC1 : le corpus de la session 4f94fbc8 -------------------------

    /// La tête que le défaut a marquée le 2026-10-26 à 15:18:30Z, telle
    /// qu'`audit_events` la porte, et qui ne porte aucune revue à ce jour.
    const TETE_DEPECHEE_4F94FBC8: &str = "0a55d28a9d6a43848aaadbb301bd3c236a477088";

    /// Une tête synthétique, pour le contrôle positif : celle d'un tour qui a
    /// posté son verdict. Délibérément pas une tête réelle de mika#2920 — aucune
    /// n'a de verdict de mika-qa, et c'est le fait que ce ticket mesure.
    const TETE_POSTEE: &str = "7c1b9e3af0d24e5f8a6b0c1d2e3f4a5b6c7d8e9f";

    /// **AC1.** Le corpus de la session `4f94fbc8` — `review` dépêché, tour
    /// épuisé, aucun verdict posté — n'écarte pas le `check_suite` suivant sur
    /// la même tête ; et le contrôle positif, dans le même test, écarte une tête
    /// dont le verdict a été posté.
    ///
    /// Les deux moitiés n'empruntent pas le même chemin d'écriture, et c'est le
    /// sujet : la ligne du corpus passe par `log_audit_event` **parce que
    /// l'outil ne peut plus l'écrire** (c'est exactement ce que R2 tient), tandis
    /// que le contrôle positif passe par l'outil réel atteint **par son nom**
    /// dans `default_tools()` — donc par l'enregistrement global dont les deux
    /// nouveaux skills appelants dépendent, sans entrée de `skill.toml`.
    ///
    /// Ce qui est relu est la condition d'écart du lecteur du vol 3
    /// (`reviewed_head_outcome(&raw).is_some()`), jamais une paraphrase : le
    /// lecteur n'est pas encore mergé, et une assertion qui le recopierait
    /// mesurerait la copie.
    #[tokio::test]
    async fn mika2927_ac1_une_revue_depechee_necarte_rien_un_verdict_poste_ecarte() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Le corps du corpus : la ligne que le défaut a produite.
        let cle_depechee = head_key("senara-solutions/mika", 2920, TETE_DEPECHEE_4F94FBC8);
        ctx.db
            .log_audit_event(
                ctx.session_id,
                QA_REVIEWED_HEAD_TOOL,
                &cle_depechee,
                None,
                Some("review"),
                Some(&format!("review sur {cle_depechee}")),
                None,
            )
            .await
            .expect("écriture du corpus hérité");

        // Le contrôle positif, par l'outil réel.
        let registry = default_tools();
        let outil = registry
            .get("record_reviewed_head")
            .expect("l'outil est enregistré dans default_tools()");
        let out = outil
            .execute(
                serde_json::json!({
                    "repo": "senara-solutions/mika",
                    "pr": 2920,
                    "head_sha": TETE_POSTEE,
                    "outcome": "verdict_posted",
                }),
                &ctx,
            )
            .await
            .expect("écriture du contrôle positif");
        assert!(!out.is_error, "reçu : {}", out.content);

        // Relecture : les deux lignes sont relues depuis la base, par la clé.
        let rows = ctx
            .db
            .get_audit_events(ctx.session_id)
            .await
            .expect("lecture du registre");
        let brute = |cle: &str| -> Option<String> {
            rows.iter()
                .find(|r| r.tool_name == QA_REVIEWED_HEAD_TOOL && r.target_key == cle)
                .and_then(|r| r.after_value.clone())
        };

        let depechee = brute(&cle_depechee).expect("la ligne du corpus est bien en base");
        assert_eq!(depechee, "review", "le corpus est relu tel quel");
        assert_eq!(
            reviewed_head_outcome(&depechee),
            None,
            "AC1 : la tête dépêchée tombe hors contrat, donc la condition \
             d'écart est fausse et le `check_suite` suivant est payé"
        );

        let cle_postee = head_key("senara-solutions/mika", 2920, TETE_POSTEE);
        let postee = brute(&cle_postee).expect("le contrôle positif a écrit sa ligne");
        assert_eq!(
            reviewed_head_outcome(&postee),
            Some("verdict_posted"),
            "contrôle positif : une tête dont le verdict est posté est reconnue, \
             donc écartée"
        );
    }

    // ---- R6/AC2+AC3 : la preuve du tour -----------------------------------

    /// La tête de #2931 **après** le force-push du 2026-10-27 — celle que la
    /// ligne de 07:06:02Z a déclarée conclue. Le ticket l'abrège `1f649bf1` ;
    /// elle est complétée ici, parce que le prédicat compare des chaînes et que
    /// la longueur réelle est ce que le test doit lui présenter.
    const TETE_NOUVELLE_2931: &str = "1f649bf10e2d4c6a8b1d3f5e7a9c0b2d4e6f8a1c";

    /// La tête **précédente** de la même PR, celle qui portait l'APPROVED du
    /// 2026-10-26 à 23:06Z — abrégée `3b44b035` par le ticket. C'est l'approbation
    /// que le force-push a laissée sur la PR, et que `reviewDecision` rendait
    /// indistinguable d'un verdict sur la nouvelle tête.
    const TETE_ANCIENNE_2931: &str = "3b44b0359c7e5a3f1d0b8a6c4e2f0d9b7a5c3e1f";

    /// La branche de #2931, telle que l'étape 1 la passe à `gh pr list`.
    const BRANCHE_2931: &str = "fix/2924/la-pr-qui-ferme-un-ticket";

    /// Un pas de tour, dans la forme que `process_tool_calls` pousse.
    ///
    /// `non_zero_exit` est laissé à `false` sur les deux branches : le prédicat
    /// ne le lit pas, et le faire varier suggérerait qu'il compte.
    fn resume(step: u32, name: &str, input_summary: String, success: bool) -> ToolCallSummary {
        ToolCallSummary {
            step,
            name: name.to_string(),
            input_summary,
            output_summary: String::new(),
            success,
            non_zero_exit: false,
        }
    }

    /// L'appel de l'étape 1, qui ne porte aucune preuve de tête.
    fn pas_de_letape_1() -> ToolCallSummary {
        resume(
            1,
            "run_gh",
            serde_json::json!({
                "command": ["pr", "list", "--head", BRANCHE_2931, "--state", "open",
                            "--json", "number,mergeable,mergeStateStatus,statusCheckRollup"],
                "repo": "senara-solutions/mika",
            })
            .to_string(),
            true,
        )
    }

    /// L'appel de l'étape 3, sur la tête donnée.
    fn pas_de_letape_3(step: u32, tete: &str, success: bool) -> ToolCallSummary {
        resume(
            step,
            QA_HEAD_REVIEWS_TOOL,
            serde_json::json!({
                "repo": "senara-solutions/mika",
                "pr": 2931,
                "head_sha": tete,
            })
            .to_string(),
            success,
        )
    }

    /// **AC3, test 1 et son contrôle négatif.** La reproduction littérale de la
    /// session `7418ef0c` : à 07:07:19Z elle avait lancé un build et attendait
    /// son callback, et elle a écrit `verdict_posted`. Son propre message de fin
    /// le dit — « la tête a été enregistrée prématurément ». La consigne
    /// (invariant 7 de `qa-review`) existait déjà et n'a pas tenu ; c'est
    /// pourquoi le prédicat existe.
    #[test]
    fn mika2934_ac3_un_tour_qui_attend_un_callback_de_build_necrit_pas_verdict_posted() {
        let attente_de_callback = vec![
            resume(
                1,
                "run_gh",
                serde_json::json!({
                    "command": ["pr", "view", "2931", "--json", "files"],
                    "repo": "senara-solutions/mika",
                })
                .to_string(),
                true,
            ),
            resume(
                2,
                "build_mika",
                serde_json::json!({"branch": BRANCHE_2931}).to_string(),
                true,
            ),
        ];
        assert_eq!(
            preuve_du_tour_manquante("verdict_posted", TETE_NOUVELLE_2931, &attente_de_callback),
            Some(PreuveManquante::VerdictSansPost),
            "AC3 : un build lancé n'est pas un verdict posté"
        );

        // Un post qui a **échoué** n'est pas un post. Sans cette moitié, « le
        // modèle a appelé `pr review` » suffirait, et c'est le contraire de ce
        // que `verdict_posted` atteste.
        let mut post_en_echec = attente_de_callback.clone();
        post_en_echec.push(resume(
            3,
            "run_gh",
            serde_json::json!({
                "command": ["pr", "review", "2931", "--approve"],
                "repo": "senara-solutions/mika",
            })
            .to_string(),
            false,
        ));
        assert_eq!(
            preuve_du_tour_manquante("verdict_posted", TETE_NOUVELLE_2931, &post_en_echec),
            Some(PreuveManquante::VerdictSansPost),
            "un `pr review` en échec ne satisfait pas la preuve"
        );

        // Contrôle négatif — le même historique, plus le post réussi.
        let mut verdict_poste = attente_de_callback;
        verdict_poste.push(resume(
            3,
            "run_gh",
            serde_json::json!({
                "command": ["pr", "review", "2931", "--approve", "--body", "VERDICT: pass"],
                "repo": "senara-solutions/mika",
            })
            .to_string(),
            true,
        ));
        assert_eq!(
            preuve_du_tour_manquante("verdict_posted", TETE_NOUVELLE_2931, &verdict_poste),
            None,
            "contrôle négatif : le tour qui a posté son verdict écrit sa ligne"
        );
    }

    /// **AC2, test 3 et son contrôle négatif.** Ce que la session `7b8f16b1`
    /// avait réellement en main à 07:06:02Z : la liste de PR de l'étape 1, et
    /// rien d'autre. Elle en a tiré « verdict déjà enregistré » et écrit
    /// `stop_verdict_recorded`.
    #[test]
    fn mika2934_ac2_un_stop_sans_lecture_de_tete_est_refuse() {
        let seule_la_liste = vec![pas_de_letape_1()];
        for stop in ["stop_verdict_recorded", "stop_already_reviewed_sha"] {
            assert_eq!(
                preuve_du_tour_manquante(stop, TETE_NOUVELLE_2931, &seule_la_liste),
                Some(PreuveManquante::StopSansLectureDeTete),
                "{stop} sans lecture de tête est refusé : la liste de PR ne dit \
                 sur quelle tête aucune revue a statué"
            );
        }

        // Propriété 1 du prédicat : une valeur hors vocabulaire n'est pas jugée
        // ici. Le refus de valeur appartient à `reviewed_head_outcome` et au
        // schéma de l'outil ; deux juges d'une même valeur divergent.
        for hors_contrat in ["review", "review_failed", "stop_no_open_pr", ""] {
            assert_eq!(
                preuve_du_tour_manquante(hors_contrat, TETE_NOUVELLE_2931, &seule_la_liste),
                None,
                "{hors_contrat} n'est pas enregistrable, et ce prédicat ne \
                 redouble pas ce jugement"
            );
        }

        // Contrôle négatif — la lecture de l'étape 3, sur CETTE tête.
        let mut avec_lecture = seule_la_liste;
        avec_lecture.push(pas_de_letape_3(2, TETE_NOUVELLE_2931, true));
        for stop in ["stop_verdict_recorded", "stop_already_reviewed_sha"] {
            assert_eq!(
                preuve_du_tour_manquante(stop, TETE_NOUVELLE_2931, &avec_lecture),
                None,
                "contrôle négatif : {stop} après la lecture de cette tête passe"
            );
        }
    }

    /// **AC2, l'autre moitié — celle qui porte la tête.** Un `qa_head_reviews`
    /// réussi portant **l'ancienne** tête ne satisfait pas une ligne demandée
    /// sur la **nouvelle**. C'est exactement la propriété que `reviewDecision`
    /// n'avait pas : une approbation gagnée par une tête précédente reste sur la
    /// PR après un rebase, et c'est ce qui a fait déclarer trois têtes conclues
    /// 9 à 17 minutes avant le premier verdict sur chacune.
    #[test]
    fn mika2934_ac2_le_force_push_ne_satisfait_pas_la_preuve() {
        let lecture_de_lancienne_tete = vec![pas_de_letape_3(1, TETE_ANCIENNE_2931, true)];

        assert_eq!(
            preuve_du_tour_manquante(
                "stop_verdict_recorded",
                TETE_NOUVELLE_2931,
                &lecture_de_lancienne_tete,
            ),
            Some(PreuveManquante::StopSansLectureDeTete),
            "la preuve porte une tête, pas une PR : l'approbation de \
             {TETE_ANCIENNE_2931} ne conclut rien sur {TETE_NOUVELLE_2931}"
        );

        // Contrôle positif, dans le même test — la même lecture satisfait la
        // ligne demandée sur la tête qu'elle nomme. Sans lui, « le prédicat
        // refuse » serait indistinguable de « le prédicat refuse tout ».
        assert_eq!(
            preuve_du_tour_manquante(
                "stop_verdict_recorded",
                TETE_ANCIENNE_2931,
                &lecture_de_lancienne_tete,
            ),
            None,
            "contrôle positif : sur sa propre tête, la lecture est la preuve"
        );

        // Et une lecture en échec ne prouve rien, sur aucune tête.
        let lecture_en_echec = vec![pas_de_letape_3(1, TETE_NOUVELLE_2931, false)];
        assert_eq!(
            preuve_du_tour_manquante(
                "stop_verdict_recorded",
                TETE_NOUVELLE_2931,
                &lecture_en_echec,
            ),
            Some(PreuveManquante::StopSansLectureDeTete),
            "un appel en échec n'a rendu aucune revue à lire"
        );

        // Une tête vide ne satisfait aucune preuve : `contains("")` est vrai de
        // toute chaîne, et une tête non nommée serait attestée par n'importe
        // quel appel. L'outil refuse déjà ce cas ; le prédicat ne s'y fie pas.
        assert_eq!(
            preuve_du_tour_manquante("stop_verdict_recorded", "", &lecture_de_lancienne_tete),
            Some(PreuveManquante::StopSansLectureDeTete),
            "une tête vide n'est pas satisfaite par n'importe quel appel"
        );
    }
}

//! Le registre des têtes sur lesquelles mika-qa a **statué**
//! (mika#2917, vol 2/3 — enfant de mika#2680, classe (g′), forme (a) ;
//! vocabulaire corrigé par mika#2927).
//!
//! # Ce que ce module est, et ce qu'il n'est pas encore
//!
//! Il fige le **vocabulaire** du registre : son `tool_name`, les trois issues qui
//! l'écrivent, et la normalisation qui transforme une attestation du modèle en
//! l'un de ces trois littéraux. Il ne lit rien et ne décide rien : au vol 2 les
//! skills écrivent, **personne ne lit**. Le pré-filtre lira au vol 3, par un
//! seul terme du trait `PrefilterState`.
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
//! 1. **Aucun `gh`.** Le lecteur du vol 3 lira la base SQLite de son propre agent
//!    par les accesseurs qui existent déjà
//!    (`count_recent_audit_events_for_target`, `get_audit_event_rows_by_tool_name`).
//!    Le bearing de mika#2680 refuse un appel `gh` depuis `webhook_prefilter.rs`
//!    (mika#2260).
//! 2. **Aucune table, aucune migration.** Le précédent vit dans ce dépôt :
//!    [`crate::qa_head_supersession`], dont la doc dit « pas de table, pas de
//!    migration » et dont `SYNC_OBSERVED_TOOL` porte la même déclaration
//!    **SOLE WRITER**. Même mécanisme, même titre.
//! 3. **Par agent sans que l'appelant le dise.** `AsyncDatabase::log_audit_event`
//!    clone son propre `agent_id` et ne l'accepte pas en argument — son premier
//!    paramètre est le `session_id`. L'état de mika-qa ne peut donc pas écarter un
//!    tour de mika-dev, et la propriété ne dépend d'aucune discipline d'appel.
//! 4. **Aucune grammaire de clé nouvelle.** [`head_key`] existe et le pré-filtre
//!    la relit déjà telle quelle. Une forme de clé retapée dans un second fichier
//!    est la divergence programmée que mika#2158 a payée — une garde de ce module
//!    refuse donc que ce fichier retape la grammaire.
//! 5. **La décision reste entière dans le skill.** Le moteur ne calcule jamais
//!    « cette tête est revue » : au vol 3 il comparera la tête que le payload
//!    nomme à une ligne que le skill a écrite.
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
}

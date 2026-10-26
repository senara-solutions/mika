//! Le registre des têtes qu'un tour de `qa-review-webhook-success` a conclues
//! (mika#2917, vol 2/3 — enfant de mika#2680, classe (g′), forme (a)).
//!
//! # Ce que ce module est, et ce qu'il n'est pas encore
//!
//! Il fige le **vocabulaire** du registre : son `tool_name`, les trois issues qui
//! l'écrivent, et la normalisation qui transforme une attestation du modèle en
//! l'un de ces trois littéraux. Il ne lit rien et ne décide rien : au vol 2 le
//! skill écrit, **personne ne lit**. Le pré-filtre lira au vol 3, par un seul
//! terme du trait `PrefilterState`.
//!
//! # L'endroit, et pourquoi là
//!
//! ```text
//! tool_name   = QA_REVIEWED_HEAD_TOOL
//! target_key  = head_key(repo, pr, head_sha)  →  pr:{repo}#{n}@{sha}
//! after_value = l'issue du tour, l'un des trois de REVIEWED_HEAD_OUTCOMES
//! reasoning   = "request_id=<uuid>" quand le skill porte l'identité de l'événement
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
//! 1. **L'écriture est postérieure à l'action.** Le prompt place l'appel après la
//!    revue dispatchée ou après le stop constaté, jamais avant. Une ligne écrite
//!    d'avance serait une tête déclarée traitée par une revue qui n'a pas eu lieu.
//! 2. **Trois issues sur six écrivent** — voir [`REVIEWED_HEAD_OUTCOMES`]. Le
//!    vocabulaire fermé rejette les trois autres : une issue qui n'écrit pas n'est
//!    pas représentable comme une écriture.
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

/// `audit_events.tool_name` du registre des têtes qu'un tour de
/// `qa-review-webhook-success` a conclues (mika#2917, vol 2).
///
/// Cible [`head_key`] ; `after_value` vaut l'une des trois valeurs de
/// [`REVIEWED_HEAD_OUTCOMES`].
///
/// **SOLE WRITER** : [`crate::tools::record_reviewed_head`], par appel d'outil du
/// modèle. Aucun chemin du moteur ne l'écrit.
///
/// **Lecteur** : aucun dans ce vol ; le pré-filtre au vol 3.
pub(crate) const QA_REVIEWED_HEAD_TOOL: &str = "qa_reviewed_head";

/// Les issues de mika#2855 qui attestent qu'une tête est **traitée**.
///
/// Les trois autres — `stop_no_open_pr`, `stop_out_of_scope`, `review_failed` —
/// n'écrivent rien : les deux premières n'ont pas de PR nommable ou sont hors
/// périmètre, et `review_failed` est précisément le cas où la revue n'a **pas**
/// abouti, donc celui où le tour suivant doit passer. Ce vocabulaire fermé est ce
/// qui rend leur écriture non représentable — voir § *La perte de revue*.
pub(crate) const REVIEWED_HEAD_OUTCOMES: &[&str] = &[
    "review",
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

    // ---- le vocabulaire fermé ---------------------------------------------

    #[test]
    fn mika2917_lissue_rendue_est_le_litteral_canonique() {
        for canonical in REVIEWED_HEAD_OUTCOMES {
            assert_eq!(reviewed_head_outcome(canonical), Some(*canonical));
        }
        // Normalisation : la graphie de l'appelant ne ressort jamais.
        assert_eq!(reviewed_head_outcome("  REVIEW  "), Some("review"));
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

    // ---- conformité du prompt --------------------------------------------

    /// Le prompt **embarqué**, pas celui de l'arbre : `include_str!` fige les
    /// octets au build, exactement comme `build.rs` les fige dans
    /// `BUNDLED_SKILL_MANIFESTS`. Une édition de l'arbre est invisible aux deux
    /// jusqu'au prochain `make deploy` (motif mika#2340). Même idiome que
    /// `server::webhook_turn_outcome::tests::EMBEDDED_PROMPT`, sur le même
    /// fichier et pour la même raison.
    const EMBEDDED_PROMPT: &str =
        include_str!("../../../skills/bundled/qa-review-webhook-success/system_prompt.md");

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

    /// Le prompt appelle l'outil pour **exactement** les trois issues
    /// enregistrables, et pour aucune autre.
    ///
    /// La propriété que ce test tient n'est pas « le prompt mentionne l'outil »
    /// mais « le vocabulaire du prompt et celui du registre ne divergent pas » :
    /// une quatrième issue passée à l'appel serait refusée à l'exécution, et une
    /// des trois oubliée serait une perte de dedup silencieuse — un tour payé par
    /// `check_suite` pour cette tête, sans rien qui le dise.
    #[test]
    fn mika2917_le_prompt_appelle_loutil_aux_trois_issues_enregistrables() {
        let appelees = issues_appelees(EMBEDDED_PROMPT);
        let mut attendues: Vec<String> = REVIEWED_HEAD_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        attendues.sort();
        assert_eq!(
            appelees, attendues,
            "le prompt embarqué et le vocabulaire du registre divergent"
        );

        // Chaque issue appelée est bien l'une que l'outil accepte : la
        // réciproque du `assert_eq!` ci-dessus, dite par le prédicat de vérité
        // lui-même plutôt que par l'égalité de deux listes.
        for issue in &appelees {
            assert_eq!(
                reviewed_head_outcome(issue),
                Some(issue.as_str()),
                "le prompt passe {issue:?}, que l'outil refuserait"
            );
        }

        // Les six valeurs de l'attestation mika#2855 restent dans le prompt :
        // la section ajoutée n'en remplace aucune.
        for ligne in [
            "Webhook-Outcome: stop_no_open_pr",
            "Webhook-Outcome: stop_out_of_scope",
            "Webhook-Outcome: stop_already_reviewed_sha",
            "Webhook-Outcome: stop_verdict_recorded",
            "Webhook-Outcome: review",
            "Webhook-Outcome: review_failed",
        ] {
            assert!(
                EMBEDDED_PROMPT.contains(ligne),
                "la section ajoutée a fait perdre {ligne:?} au prompt"
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
}

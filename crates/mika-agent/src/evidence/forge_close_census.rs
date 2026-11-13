//! Recensement des outils servis aux agents qui atteignent la forge (mika#3056, AC4).
//!
//! AC1 pose un refus sur `run_gh` : une fermeture d'issue reconnue par le
//! détecteur de mika#1646 est refusée avant toute lecture. Ce refus ne vaut que
//! ce que vaut sa **totalité** — un second chemin vers l'état `closed` d'un
//! ticket le contournerait sans rien faire rougir. AC4 demande donc l'énumération
//! des outils servis qui écrivent à la forge, et qu'un test la garde.
//!
//! ## La table est le recensement, le scan est son contrôle
//!
//! [`FORGE_CLOSE_CENSUS`] attribue à chaque outil qui atteint la forge une
//! [`CloseAuthority`] et la raison mesurée de cette attribution. Trois gardes
//! l'encadrent, et chacune couvre une classe que les autres ne voient pas :
//!
//! 1. **Pas de faux négatif, mesuré.** Un scan parcourt la surface servie,
//!    résout la source de chaque outil et y cherche les coutures de forge. Un
//!    outil porteur d'une couture et absent de la table rougit. C'est l'idiome
//!    de `crate::gh_process` (mika#2713/mika#2715) réemployé tel quel.
//! 2. **Auto-nettoyant.** Une entrée dont le nom n'est plus servi rougit —
//!    égalité, jamais inclusion, parce qu'une table qui ne sait pas rétrécir
//!    devient l'allowlist que mika#2309 condamne.
//! 3. **L'invariant d'AC4, nommé.** [`CloseAuthority::ClosesIssueDirectly`] est
//!    exactement `{run_gh}` et [`CloseAuthority::ClosesByMergeOnly`] exactement
//!    `{pr_merge_with_gate}`. Un troisième chemin vers `closed` ne peut pas
//!    entrer dans la table sans faire rougir cette égalité.
//!
//! ## Pourquoi les huit builtins de `builtin_handlers.rs` sont traités à part
//!
//! Le scan associe un outil à `src/tools/<nom>.rs` quand ce fichier existe. Les
//! huit entrées de [`crate::skills::builtin_handlers::KNOWN_BUILTINS`] n'en ont
//! aucun : elles sont servies depuis un seul fichier de 15 000 lignes. Un scan
//! par fichier attribuerait donc la couture de `run_gh` à `web_search`,
//! `fetch_url`, `get_documentation`, `git_ops`, `run_gws` et `review_skill` —
//! six faux positifs, et une garde qui rougit sur du code sain ne protège rien.
//! Ces huit reçoivent à la place une **disposition explicite** : chacune est soit
//! dans la table, soit dans [`BUILTINS_WITHOUT_FORGE_ACCESS`] avec sa raison, et
//! l'union des deux doit être exactement `KNOWN_BUILTINS`. Auto-nettoyant sans
//! scanner le fichier.
//!
//! ## Le reste qui tient, et qu'aucun test ne peut fermer
//!
//! Les entrées [`CloseAuthority::ArbitraryExecution`] exécutent une commande que
//! l'agent compose à l'exécution. « Cette commande ferme-t-elle un ticket ? »
//! n'est pas une propriété de notre code : c'est une propriété d'une chaîne qui
//! n'existe pas encore au moment où un test pourrait la lire. La table les
//! **énumère** — c'est ce qu'AC4 peut livrer de plus fort sur cette classe — mais
//! aucune garde n'affirme que l'énumération est complète, et aucune ne les
//! refuse. Trois mesures bornent ce reste, et il est consigné sur mika#3056 :
//!
//! - Les 9 tentatives relevées du 2026-09-10 au 2026-11-06 sont **toutes** passées
//!   par `run_gh`, donc par l'argv que la garde lit.
//! - Les appels `run_shell` de `qa-review` encastrent le corps de la PR mot pour
//!   mot (`skills/executor.rs`), donc une garde sur la chaîne refuserait un
//!   corps de PR contenant `gh issue close 1234` — un refus sur un non-geste.
//! - Pour `run_claude_pilot` et `run_claude_pilot_groom`, l'allowlist `gh` du
//!   worktree de pilote est **construite** et non copiée (mika#3030) : toute
//!   entrée `Bash(...)` portant un `gh` isolé est retirée et remplacée par neuf
//!   entrées étroites dont `gh issue close` ne fait pas partie. La tentative
//!   retombe sur la permission-policy de claude-pilot, qui la refuse.

/// Ce qu'un outil servi peut faire à l'état d'un ticket.
///
/// Les variantes sont ordonnées de la plus forte autorité à la plus faible, et
/// `ArbitraryExecution` est en dernier parce qu'elle ne décrit pas une autorité
/// mesurée sur notre code mais l'absence de borne sur ce que l'outil exécute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CloseAuthority {
    /// Peut faire passer une issue à `closed` par un appel direct à la forge.
    /// C'est le chemin qu'AC1 refuse.
    ClosesIssueDirectly,
    /// Peut fermer une issue, mais seulement comme **effet** du merge d'une PR
    /// dont le corps porte un `Closes` — la seule fermeture que la doctrine
    /// reconnaisse.
    ClosesByMergeOnly,
    /// Atteint la forge en lecture seule.
    ForgeReadOnly,
    /// Écrit à la forge, mais aucune de ses écritures n'atteint l'état d'un
    /// ticket.
    ForgeWriteNotClose,
    /// Exécute une commande composée à l'exécution. Non borné par construction :
    /// cf. le reste nommé dans la doc du module.
    ArbitraryExecution,
}

/// Les outils servis qui atteignent la forge, leur autorité sur l'état d'un
/// ticket, et la raison mesurée de cette attribution.
///
/// Une raison n'est pas une glose : c'est ce qui permet à la prochaine lectrice
/// de refaire la mesure plutôt que de faire confiance à la colonne du milieu.
pub(crate) const FORGE_CLOSE_CENSUS: &[(&str, CloseAuthority, &str)] = &[
    (
        "run_gh",
        CloseAuthority::ClosesIssueDirectly,
        "le seul outil dont l'argv atteint `gh issue close` ; c'est le site de la \
         garde de mika#1646 et le bras que mika#3056 y ajoute",
    ),
    (
        "pr_merge_with_gate",
        CloseAuthority::ClosesByMergeOnly,
        "merge une PR, donc ferme ce que son `Closes` nomme — la fermeture que la \
         doctrine reconnaît ; ne porte aucun argv de fermeture d'issue",
    ),
    (
        "gh_read",
        CloseAuthority::ForgeReadOnly,
        "vocabulaire d'opérations fermé, lectures seules (`validate_gh_read_input`)",
    ),
    (
        "qa_pr_view",
        CloseAuthority::ForgeReadOnly,
        "son handler n'appelle que `gh pr view --json`",
    ),
    (
        "qa_head_reviews",
        CloseAuthority::ForgeReadOnly,
        "son handler n'appelle que `gh api` sur `check-suites` et `pulls/{n}/reviews`",
    ),
    (
        "check_task",
        CloseAuthority::ForgeReadOnly,
        "GET sur `api.github.com/repos/{o}/{r}/pulls|issues/{n}` pour relire un état",
    ),
    (
        "resolve_issue_order",
        CloseAuthority::ForgeReadOnly,
        "lit les bloqueurs ouverts par GraphQL (`github_graphql::fetch_open_blockers`)",
    ),
    (
        "fetch_url",
        CloseAuthority::ForgeReadOnly,
        "ne porte qu'une URL à la gateway, qui la lit ; l'agent ne contrôle ni \
         méthode, ni corps, ni en-tête, et aucun jeton de forge n'y est joint",
    ),
    (
        "git_ops",
        CloseAuthority::ForgeWriteNotClose,
        "vocabulaire d'opérations fermé (9 entrées) ; le seul push est le \
         `--force-with-lease` d'un rebase, et pousser des commits ne fait passer \
         aucun ticket à `closed` — seul un merge le fait, et git_ops n'en fait pas",
    ),
    (
        "run_shell",
        CloseAuthority::ArbitraryExecution,
        "exécute la commande qu'on lui passe",
    ),
    (
        "tmux_create_session",
        CloseAuthority::ArbitraryExecution,
        "ouvre une session tmux sur une commande initiale",
    ),
    (
        "tmux_send_command",
        CloseAuthority::ArbitraryExecution,
        "envoie une ligne de commande à un shell vivant",
    ),
    (
        "run_claude_pilot",
        CloseAuthority::ArbitraryExecution,
        "dispatche un claude-pilot ; borné en pratique par l'allowlist construite \
         de mika#3030, pas par cette garde",
    ),
    (
        "run_claude_pilot_groom",
        CloseAuthority::ArbitraryExecution,
        "même dispatch, même borne mika#3030",
    ),
    (
        "address_pr_comments",
        CloseAuthority::ArbitraryExecution,
        "lit les commentaires par `gh api`, puis dispatche un pilote",
    ),
    (
        "resolve_pr_conflicts",
        CloseAuthority::ArbitraryExecution,
        "lit les refs par `gh pr view`, puis dispatche un pilote",
    ),
    (
        "build_mika",
        CloseAuthority::ArbitraryExecution,
        "lance la chaîne de build",
    ),
    (
        "deploy_mika",
        CloseAuthority::ArbitraryExecution,
        "lance la chaîne de déploiement",
    ),
];

/// Les entrées de `KNOWN_BUILTINS` qui n'atteignent pas la forge, et pourquoi.
///
/// Ce n'est pas une allowlist de scan : c'est l'autre moitié d'une disposition
/// dont l'union doit être exactement `KNOWN_BUILTINS` (cf. doc du module). Un
/// neuvième builtin ne peut pas être servi sans être rangé ici ou dans la table.
pub(crate) const BUILTINS_WITHOUT_FORGE_ACCESS: &[(&str, &str)] = &[
    (
        "web_search",
        "interroge Brave par la gateway ; aucun jeton de forge, aucune écriture",
    ),
    (
        "get_documentation",
        "rend la documentation embarquée dans le binaire",
    ),
    ("run_gws", "parle à Google Workspace, pas à GitHub"),
    (
        "review_skill",
        "relit le prompt d'un skill et rend un verdict ; n'écrit qu'en base",
    ),
];

/// Les coutures par lesquelles une source atteint la forge.
///
/// Épelées par concaténation pour que ce fichier ne se compte pas lui-même —
/// l'idiome de `crate::gh_process`, qui fait de même pour son aiguille de spawn.
fn forge_seams() -> Vec<String> {
    vec![
        ["Command::new(", "\"gh\")"].concat(),
        "gh_process".to_string(),
        "octocrab".to_string(),
        "github_graphql".to_string(),
        "api.github.com".to_string(),
        ["gh", " issue "].concat(),
        ["gh", " pr "].concat(),
        ["gh", " api "].concat(),
    ]
}

/// Les coutures qui font **passer un ticket à `closed`**, et elles seules.
///
/// Plus étroit que [`forge_seams`] à dessein : la question posée aux exécutables
/// du bundle n'est pas « parles-tu à la forge ? » — quatre handlers le font, tous
/// en lecture — mais « fermes-tu un ticket ? ».
///
/// **`gh issue edit` n'en est pas une, et c'est mesuré.** Le premier jet le
/// comptait par prudence ; il a fait rougir `_shared/dispatch-lib.sh` sur trois
/// sites qui posent un `--body-file`, un `--add-label` et un `--remove-label`.
/// La sous-commande n'expose aucun `--state` : elle ne peut pas fermer. Une
/// fermeture passée par l'API REST, elle, se lit sur les trois aiguilles `state`
/// ci-dessous, qui restent.
fn issue_close_seams() -> Vec<String> {
    vec![
        ["gh", " issue close"].concat(),
        "--state closed".to_string(),
        "state=closed".to_string(),
        r#""state":"closed""#.to_string(),
        r#""state": "closed""#.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled_skills::{all_bundled_tool_names, bundled_executables};
    use crate::skills::builtin_handlers::KNOWN_BUILTINS;
    use crate::source_scan::production_files;
    use crate::tools::BUILTIN_TOOL_NAMES;
    use std::collections::{BTreeMap, BTreeSet};

    /// La surface servie : les builtins du registre, et tout outil déclaré par
    /// le `tools.json` d'un skill du bundle.
    ///
    /// C'est l'union qu'AC4 prend pour population. Un outil hors de cette union
    /// n'est servi à personne, donc ne peut rien fermer.
    fn served_surface() -> BTreeSet<String> {
        BUILTIN_TOOL_NAMES
            .iter()
            .map(|s| s.to_string())
            .chain(all_bundled_tool_names())
            .collect()
    }

    fn census_names() -> BTreeSet<String> {
        FORGE_CLOSE_CENSUS
            .iter()
            .map(|(n, _, _)| n.to_string())
            .collect()
    }

    fn names_with(authority: CloseAuthority) -> BTreeSet<String> {
        FORGE_CLOSE_CENSUS
            .iter()
            .filter(|(_, a, _)| *a == authority)
            .map(|(n, _, _)| n.to_string())
            .collect()
    }

    /// L'invariant d'AC4, dit en deux égalités.
    ///
    /// Un troisième chemin vers `closed` ne peut pas entrer dans la table sans
    /// faire rougir l'une des deux — et l'entrer dans une autre variante le fait
    /// rougir dans le scan de non-faux-négatif, qui lit les coutures et non les
    /// étiquettes.
    #[test]
    fn mika3056_une_seule_autorite_ferme_une_issue_et_une_seule_la_ferme_par_merge() {
        assert_eq!(
            names_with(CloseAuthority::ClosesIssueDirectly),
            BTreeSet::from(["run_gh".to_string()]),
            "AC4 : `run_gh` est le seul outil servi qui fasse passer une issue à \
             `closed` par un appel direct. Si un second chemin existe, il reçoit le \
             refus `block[issue_close_not_by_merge]` — il n'entre pas dans cette \
             table sans sa garde."
        );
        assert_eq!(
            names_with(CloseAuthority::ClosesByMergeOnly),
            BTreeSet::from(["pr_merge_with_gate".to_string()]),
            "la fermeture par merge approuvé est la seule que la doctrine \
             reconnaisse, et `pr_merge_with_gate` en est le site unique."
        );
    }

    /// Auto-nettoyant : chaque nom recensé est encore servi.
    ///
    /// Égalité sur l'intersection plutôt qu'inclusion — une entrée pour un outil
    /// retiré du registre est exactement l'allowlist périmée que mika#2309
    /// condamne, et elle se lirait comme une couverture.
    #[test]
    fn mika3056_le_recensement_ne_nomme_que_des_outils_encore_servis() {
        let served = served_surface();
        let orphelins: BTreeSet<_> = census_names().difference(&served).cloned().collect();
        assert_eq!(
            orphelins,
            BTreeSet::new(),
            "ces entrées du recensement ne nomment plus un outil servi. Retire-les \
             de FORGE_CLOSE_CENSUS : une entrée périmée se lit comme une couverture."
        );
    }

    /// Les huit builtins de `builtin_handlers.rs` sont dispositionnés, et leur
    /// disposition est exhaustive.
    ///
    /// Union exacte, dans les deux sens : un neuvième builtin ne peut pas être
    /// servi sans être rangé, et une entrée pour un builtin retiré rougit aussi.
    /// C'est ce qui remplace le scan là où il ne peut pas discriminer (cf. doc
    /// du module).
    #[test]
    fn mika3056_les_builtins_du_fichier_unique_sont_tous_dispositionnes() {
        let declares: BTreeSet<String> = KNOWN_BUILTINS.iter().map(|s| s.to_string()).collect();
        let recenses: BTreeSet<String> = census_names().intersection(&declares).cloned().collect();
        let sans_forge: BTreeSet<String> = BUILTINS_WITHOUT_FORGE_ACCESS
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();

        let chevauchement: BTreeSet<_> = recenses.intersection(&sans_forge).cloned().collect();
        assert_eq!(
            chevauchement,
            BTreeSet::new(),
            "un builtin ne peut pas être à la fois recensé comme atteignant la forge \
             et déclaré sans accès à la forge."
        );

        let union: BTreeSet<String> = recenses.union(&sans_forge).cloned().collect();
        assert_eq!(
            union, declares,
            "chaque entrée de KNOWN_BUILTINS doit être soit dans FORGE_CLOSE_CENSUS, \
             soit dans BUILTINS_WITHOUT_FORGE_ACCESS avec sa raison. Le scan ne peut \
             pas trancher à leur place : les huit sont servis depuis un seul fichier, \
             donc une couture y est attribuable à n'importe lequel."
        );
    }

    /// Chaque entrée porte une raison, et la raison n'est pas un mot.
    ///
    /// Une colonne vide ferait de la table une liste de noms — reclassable sans
    /// que personne ait à refaire la mesure.
    #[test]
    fn mika3056_chaque_entree_porte_sa_raison_mesuree() {
        for (nom, _, raison) in FORGE_CLOSE_CENSUS {
            assert!(
                raison.len() > 20,
                "l'entrée `{nom}` de FORGE_CLOSE_CENSUS n'a pas de raison lisible"
            );
        }
        for (nom, raison) in BUILTINS_WITHOUT_FORGE_ACCESS {
            assert!(
                raison.len() > 20,
                "l'entrée `{nom}` de BUILTINS_WITHOUT_FORGE_ACCESS n'a pas de raison \
                 lisible"
            );
        }
    }

    /// Pas de faux négatif, **mesuré** : la source de chaque outil servi est
    /// lue, et un porteur de couture absent de la table rougit.
    ///
    /// La résolution outil → source est `src/tools/<nom>.rs`. Les outils qui n'en
    /// ont pas sont soit les huit de `KNOWN_BUILTINS` — dispositionnés par le
    /// test ci-dessus — soit servis par un handler de bundle, couvert par le test
    /// suivant. Ce découpage est asserté ici même : un outil sans source résoluble
    /// et hors des deux populations rougit, plutôt que de passer en silence.
    #[test]
    fn mika3056_aucun_outil_porteur_dune_couture_de_forge_nechappe_au_recensement() {
        let sources: BTreeMap<String, String> = production_files()
            .into_iter()
            .filter_map(|(rel, src)| {
                rel.strip_prefix("tools/")
                    .and_then(|f| f.strip_suffix(".rs"))
                    .map(|nom| (nom.to_string(), src))
            })
            .collect();

        let seams = forge_seams();
        let recense = census_names();
        let builtins: BTreeSet<String> = KNOWN_BUILTINS.iter().map(|s| s.to_string()).collect();

        let mut manquants = BTreeSet::new();
        let mut sans_source = BTreeSet::new();
        for outil in served_surface() {
            match sources.get(&outil) {
                Some(src) => {
                    if seams.iter().any(|s| src.contains(s.as_str())) && !recense.contains(&outil) {
                        manquants.insert(outil);
                    }
                }
                None if !builtins.contains(&outil) && !recense.contains(&outil) => {
                    sans_source.insert(outil);
                }
                None => {}
            }
        }

        assert_eq!(
            manquants,
            BTreeSet::new(),
            "ces outils servis portent une couture de forge dans leur source et ne \
             sont pas recensés. Ajoute-les à FORGE_CLOSE_CENSUS avec leur autorité \
             mesurée — et si l'une d'elles atteint `closed`, elle reçoit d'abord le \
             refus de mika#3056."
        );
        assert_eq!(
            sans_source,
            BTreeSet::from_iter([
                "read_file".to_string(),
                "tmux_kill_session".to_string(),
                "tmux_list_sessions".to_string(),
                "tmux_read_output".to_string(),
                "tmux_wait_for_text".to_string(),
            ]),
            "cet ensemble est la population que le scan de `src/tools/` ne voit pas \
             et que les deux autres dispositions ne couvrent pas : des outils servis \
             par un handler de bundle qui n'atteint pas la forge. Il est épelé plutôt \
             que toléré — un nouvel outil servi par un handler y apparaît, et sa \
             disposition se décide ici."
        );
    }

    /// Aucun exécutable du bundle ne ferme un ticket.
    ///
    /// L'assertion directe, parce qu'elle est plus forte que ce qu'une analyse de
    /// `source` transitive donnerait : `_shared/dispatch-lib.sh` est sourcé par
    /// ses frères, et plutôt que de reconstruire ce graphe on affirme la propriété
    /// sur **tous** les exécutables, support dirs compris.
    ///
    /// Mesuré le 2026-11-07 : quatre handlers atteignent la forge
    /// (`qa_head_reviews.sh`, `qa_pr_view.sh`, `address-pr-comments/run.sh`,
    /// `resolve-pr-conflicts/run.sh`) et les quatre ne font que lire.
    #[test]
    fn mika3056_aucun_executable_du_bundle_ne_ferme_un_ticket() {
        let seams = issue_close_seams();
        let mut porteurs = BTreeSet::new();
        for (proprietaire, chemin, contenu) in bundled_executables() {
            for ligne in contenu.lines() {
                let t = ligne.trim_start();
                if t.starts_with('#') {
                    continue;
                }
                if seams.iter().any(|s| t.contains(s.as_str())) {
                    porteurs.insert(format!("{proprietaire}/{chemin}"));
                }
            }
        }
        assert_eq!(
            porteurs,
            BTreeSet::new(),
            "ces exécutables du bundle portent une couture de fermeture de ticket. \
             AC4 de mika#3056 : ce chemin reçoit le même refus, ou il disparaît. Un \
             `gh issue close` passé à `run_gh` est déjà gardé — celui-ci ne l'est pas."
        );
    }

    /// Contrôle de bonne foi : le scan du bundle mord.
    ///
    /// Sans lui, les deux tests ci-dessus seraient verts si `bundled_executables`
    /// rendait la liste vide, ou si les aiguilles ne correspondaient à rien.
    #[test]
    fn mika3056_le_scan_du_bundle_mord_sur_une_fermeture() {
        assert!(
            !bundled_executables().is_empty(),
            "la population scannée est vide : le scan ne refuserait rien"
        );
        let seams = issue_close_seams();
        let faux = ["gh", " issue close \"$num\""].concat();
        assert!(
            seams.iter().any(|s| faux.contains(s.as_str())),
            "les aiguilles de fermeture ne reconnaissent pas une fermeture écrite \
             comme un handler l'écrirait"
        );
        let commentaire = ["# ", "gh", " issue close est refusé ici"].concat();
        assert!(
            commentaire.trim_start().starts_with('#'),
            "le filtre de commentaires doit écarter la prose qui NOMME la couture, \
             sinon un handler qui documente le refus se lirait comme une violation"
        );
    }

    /// Contrôle de bonne foi : le scan de `src/tools/` mord.
    ///
    /// Le test de non-faux-négatif serait vert si `production_files` ne rendait
    /// aucun fichier de `tools/`, ou si les aiguilles de forge ne reconnaissaient
    /// pas la couture de l'outil qui la porte le plus visiblement.
    #[test]
    fn mika3056_le_scan_des_sources_mord_sur_la_couture_connue() {
        let sources: BTreeMap<String, String> = production_files()
            .into_iter()
            .filter_map(|(rel, src)| {
                rel.strip_prefix("tools/")
                    .and_then(|f| f.strip_suffix(".rs"))
                    .map(|nom| (nom.to_string(), src))
            })
            .collect();
        let gate = sources
            .get("pr_merge_with_gate")
            .expect("tools/pr_merge_with_gate.rs doit être lisible par le scan");
        let seams = forge_seams();
        assert!(
            seams.iter().any(|s| gate.contains(s.as_str())),
            "les aiguilles de forge ne reconnaissent pas la couture de \
             `pr_merge_with_gate`, qui spawne `gh` : le scan ne refuserait rien"
        );
    }
}

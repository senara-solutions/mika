//! Qui est qui sur la forge, et qui a le droit de merger (mika#2248).
//!
//! # Le défaut que ce module ferme
//!
//! `check_suite.completed(success)` est diffusé à **deux** agents : `mika-dev`
//! (primaire — le dispatcher, propriétaire du cycle dispatch→review→merge) et
//! `mika-qa` (secondaire depuis mika#1711 — la revue autonome). Les deux
//! exécutent la même chaîne de handlers structurels, et le handler de succès CI
//! appelait `gh pr merge` directement. Le merge tournait donc sous le token de
//! **celui qui gagnait la course**. Mesuré le 2026-09-08 sur mika#2244 :
//! `mergedBy = mika-platform-qa` — le relecteur a mergé sa propre approbation.
//! Juge et partie, et aucun handoff : personne ne reprend le volant.
//!
//! # La forme du correctif : signal, pas acteur
//!
//! Le handler n'est plus un acteur. Il **signale** qu'une PR est mergeable
//! (toutes les portes franchies : verdict, CI agrégée, périmètre, behind-main),
//! et **seul le dispatcher** consomme ce signal et merge sous sa propre
//! identité. Quatre couches indépendantes, chacune à usage unique :
//!
//! | couche | garantie |
//! |---|---|
//! | le handler de succès CI n'appelle plus `gh pr merge` | aucun agent ne merge depuis l'évaluation |
//! | [`merge_disposition`] — liste blanche | seul le dispatcher agit ; tout autre agent se contente de signaler |
//! | [`would_merge_as_reviewer`] + refus outil | le relecteur n'atteint jamais le chemin de merge |
//! | [`owns_merge_transition`] — porte d'entrée de l'évaluateur (mika#2260) | seul le dispatcher **évalue** ; les autres agents restent transparents à l'événement |
//!
//! La quatrième couche est arrivée après les trois autres, et pour une raison
//! qu'elles ne couvraient pas : #2248 a mis l'identité à la **sortie** (qui
//! merge) et l'a laissée absente de l'**entrée** (pour qui cette évaluation
//! existe). Le relecteur parcourait donc l'évaluateur complet — cinq appels `gh`
//! sous son PAT, une écriture `update-branch`, une clé de dedup globale au
//! processus qui avalait le créneau du dispatcher — avant d'être retenu à
//! l'acteur. Une course résolue par une garde tardive dépend de l'ordre ; celle-ci
//! la supprime.
//!
//! La liste blanche est le gate porteur ; la correspondance de login est une
//! seconde ceinture, indépendante, qui attrape le cas où un relecteur
//! emprunterait un autre nom d'agent.
//!
//! # Ce que ce module ne décide PAS
//!
//! Il ne remplace pas le classifieur de périmètre (mika#1829/#1853) : une PR
//! décision-core reste tenue pour l'opérateur, et l'acteur revérifie le
//! périmètre lui-même, fail-closed. L'identité dit *qui* peut merger ; le
//! périmètre dit *quoi* peut être mergé. Les deux portes sont en série.

use std::fmt;

/// L'agent dispatcher — propriétaire du cycle dispatch→review→merge, et **seul**
/// acteur autorisé d'un merge autonome.
pub const DISPATCHER_AGENT: &str = "mika-dev";

/// L'agent relecteur. Jamais acteur d'un merge : il mergerait sa propre
/// approbation.
pub const REVIEWER_AGENT: &str = "mika-qa";

/// Le login GitHub sous lequel le relecteur approuve et apparaîtrait dans
/// `mergedBy`.
///
/// C'est la même identité que le filtre `review_requested` du gateway
/// (`mika-gateway/src/github.rs`, `QA_REVIEWER_LOGIN`) — d'où la définition
/// unique ici, importée par les deux crates : un changement de compte bot doit
/// se lire à un seul endroit.
pub const REVIEWER_FORGE_LOGIN: &str = "mika-platform-qa";

/// Le login GitHub sous lequel la boucle autonome ouvre ses PRs.
///
/// Défini ici, à côté de son pendant relecteur, pour la même raison : un
/// changement de compte bot doit se lire à un seul endroit. Son premier
/// consommateur est le réconciliateur de demandes de revue (mika#2334), qui
/// s'en sert pour ne jamais poser de relecteur sur une PR humaine.
///
/// Délibérément **absent** de [`forge_login_for_agent`] : cette fonction sert
/// uniquement à attraper une égalité d'identité prouvée sur le chemin de merge,
/// et y cartographier le dispatcher élargirait la surface d'une fonction de
/// sécurité sans qu'aucun appelant ne le demande.
pub const DISPATCHER_FORGE_LOGIN: &str = "mika-platform-dev";

/// Préfixe de la ligne porteuse du signal merge-ready.
pub const MERGE_READY_MARKER: &str = "MERGE-READY-SIGNAL:";

/// Normalise un identifiant d'agent avant toute comparaison — même discipline
/// que `permission_authority` (minuscules, bords rognés).
fn normalize(agent_id: &str) -> String {
    agent_id.trim().to_lowercase()
}

/// Vrai quand `agent_id` est l'agent relecteur.
pub fn is_reviewer_agent(agent_id: &str) -> bool {
    normalize(agent_id) == REVIEWER_AGENT
}

/// Vrai quand `agent_id` est l'agent dispatcher.
pub fn is_dispatcher_agent(agent_id: &str) -> bool {
    normalize(agent_id) == DISPATCHER_AGENT
}

/// Ce que l'agent courant est autorisé à faire d'un signal merge-ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeDisposition {
    /// Merger, sous sa propre identité.
    Act,
    /// Émettre/laisser le signal, et s'arrêter là.
    SignalOnly,
}

/// Qui agit. **Liste blanche** et non liste noire : un agent inconnu signale,
/// il ne merge pas. Ajouter un acteur est un geste explicite ici, pas un effet
/// de bord d'un nouveau nom d'agent.
pub fn merge_disposition(agent_id: &str) -> MergeDisposition {
    if is_dispatcher_agent(agent_id) {
        MergeDisposition::Act
    } else {
        MergeDisposition::SignalOnly
    }
}

/// Vrai quand `agent_id` possède la transition évaluer→signaler→merger, et peut
/// donc consommer sa propre évaluation de mergeabilité (mika#2260).
///
/// # Pourquoi un nom plutôt qu'un [`merge_disposition`] en ligne
///
/// Les deux questions ne sont pas la même, même si la table sous-jacente l'est.
/// [`merge_disposition`] répond *qui merge* — une décision d'**acteur**, prise au
/// moment de merger, et qui ne bouge pas. Celle-ci répond *est-ce que mon
/// évaluation a un consommateur* — une décision d'**entrée**, prise avant tout
/// travail. Un agent qui ne peut pas consommer le signal n'a aucune raison de
/// l'émettre : ses appels `gh` sont payés pour rien, ses écritures de dedup
/// avalent le créneau de celui qui peut agir, et sa notification opérateur fait
/// doublon.
///
/// Deux noms, deux sites d'appel, **une seule liste blanche** : ajouter un acteur
/// reste un geste explicite dans [`merge_disposition`], et la porte d'entrée en
/// hérite sans seconde table à tenir en phase.
///
/// # Ce que ce prédicat n'est PAS
///
/// Ce n'est pas [`forge_login_for_agent`], qui cartographie un `agent_id` vers un
/// login de forge et sert uniquement à attraper une égalité d'identité prouvée
/// sur le chemin de merge. Et il ne **se substitue pas** à l'acteur : la garde
/// [`would_merge_as_reviewer`] et la liste blanche restent en place derrière lui,
/// en série. Une porte d'entrée retire une course ; elle ne dispense pas de la
/// ceinture qui la résolvait.
pub fn owns_merge_transition(agent_id: &str) -> bool {
    merge_disposition(agent_id) == MergeDisposition::Act
}

/// Le login de forge sous lequel les credentials de `agent_id` écrivent, quand
/// il est connu.
///
/// `None` signifie « non cartographié » — pas « sûr ». Les appelants s'appuient
/// sur [`merge_disposition`] pour autoriser, et sur cette fonction seulement
/// pour attraper une égalité d'identité prouvée.
pub fn forge_login_for_agent(agent_id: &str) -> Option<&'static str> {
    match normalize(agent_id).as_str() {
        REVIEWER_AGENT => Some(REVIEWER_FORGE_LOGIN),
        _ => None,
    }
}

/// Vrai quand `login` est le login de forge de l'identité de revue (mika#2667).
///
/// Lecteur unique du côté **lecture** de la séparation des rôles : la porte de
/// merge ne reconnaît un verdict que s'il vient de [`REVIEWER_FORGE_LOGIN`].
/// La comparaison tolère ce que les surfaces transportent réellement — bords
/// rognés, `@` de tête, casse, suffixe `[bot]` — et rien d'autre. Un login vide
/// n'est jamais le relecteur.
pub fn is_reviewer_forge_login(login: &str) -> bool {
    let trimmed = login.trim().trim_start_matches('@').to_ascii_lowercase();
    let bare = trimmed.strip_suffix("[bot]").unwrap_or(&trimmed).trim();
    !bare.is_empty() && bare == REVIEWER_FORGE_LOGIN
}

/// Vrai quand laisser `actor_agent_id` merger écrirait `mergedBy` avec le login
/// qui a posé la revue — exactement la forme mesurée sur mika#2244.
///
/// Le login attendu est comparé sans casse et sans `@` de tête : les surfaces
/// qui le transportent (corps de revue, pré-digest) l'écrivent des deux façons.
pub fn would_merge_as_reviewer(actor_agent_id: &str, reviewer_login: &str) -> bool {
    let reviewer = reviewer_login.trim().trim_start_matches('@');
    forge_login_for_agent(actor_agent_id).is_some_and(|login| login.eq_ignore_ascii_case(reviewer))
}

/// Le signal merge-ready : toutes les portes ont été franchies pour ce
/// `(repo, pr, head_sha)`, le dispatcher peut merger.
///
/// Porté en clair dans le pré-digest du tour — le handoff est donc lisible dans
/// la même trace que la décision, et ne dépend d'aucune permission de forge
/// (une écriture de label, elle, échoue sous un PAT sans `issues: write` —
/// mesuré, mika#2228).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeReadySignal {
    /// `owner/repo`.
    pub repo: String,
    /// Numéro de PR.
    pub pr_number: u64,
    /// SHA de tête au moment de l'évaluation — le signal est périmé dès qu'il bouge.
    pub head_sha: String,
    /// Branche de tête.
    pub branch: String,
    /// Identifiant de tâche, ou `none`.
    pub task_id: String,
    /// Login du relecteur qui a posé `VERDICT: pass`.
    pub reviewer_login: String,
}

impl fmt::Display for MergeReadySignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{MERGE_READY_MARKER} {}#{} head={} branch={} task={} reviewer={}",
            self.repo,
            self.pr_number,
            self.head_sha,
            self.branch,
            self.task_id,
            self.reviewer_login
        )
    }
}

/// Relit un signal merge-ready depuis le texte d'un tour.
///
/// Retourne `None` dès qu'un champ manque : un signal partiel n'autorise rien.
pub fn parse_merge_ready_signal(text: &str) -> Option<MergeReadySignal> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(MERGE_READY_MARKER))?;

    let mut tokens = line[MERGE_READY_MARKER.len()..].split_whitespace();
    let target = tokens.next()?;
    let (repo, pr) = target.rsplit_once('#')?;
    let pr_number: u64 = pr.parse().ok()?;
    if repo.is_empty() || !repo.contains('/') {
        return None;
    }

    let mut head_sha = None;
    let mut branch = None;
    let mut task_id = None;
    let mut reviewer_login = None;
    for token in tokens {
        match token.split_once('=') {
            Some(("head", v)) => head_sha = Some(v),
            Some(("branch", v)) => branch = Some(v),
            Some(("task", v)) => task_id = Some(v),
            Some(("reviewer", v)) => reviewer_login = Some(v),
            _ => {}
        }
    }

    Some(MergeReadySignal {
        repo: repo.to_string(),
        pr_number,
        head_sha: non_empty(head_sha)?.to_string(),
        branch: non_empty(branch)?.to_string(),
        task_id: non_empty(task_id)?.to_string(),
        reviewer_login: non_empty(reviewer_login)?.to_string(),
    })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_dispatcher_agit_le_relecteur_signale() {
        assert_eq!(merge_disposition("mika-dev"), MergeDisposition::Act);
        assert_eq!(merge_disposition("mika-qa"), MergeDisposition::SignalOnly);
    }

    #[test]
    fn un_agent_inconnu_signale_sans_merger() {
        // Liste blanche : `mika`, `mika-arch`, un agent client, un nom vide —
        // aucun n'hérite du droit de merger en apparaissant dans la chaîne.
        for agent in ["mika", "mika-arch", "mika-prime", "customer-42", ""] {
            assert_eq!(
                merge_disposition(agent),
                MergeDisposition::SignalOnly,
                "{agent} ne doit pas être acteur d'un merge autonome"
            );
        }
    }

    #[test]
    fn mika2260_seul_le_dispatcher_possede_la_transition_de_merge() {
        assert!(owns_merge_transition("mika-dev"));
        // Le relecteur, un agent hors liste, un nom vide, et un voisin lexical du
        // dispatcher : aucun n'hérite du droit d'évaluer.
        for agent in [
            "mika-qa",
            "mika",
            "mika-arch",
            "customer-42",
            "",
            "mika-dev-2",
        ] {
            assert!(
                !owns_merge_transition(agent),
                "`{agent}` ne peut pas consommer un signal merge-ready : il ne doit pas l'évaluer"
            );
        }
        // La normalisation est celle de la maison, héritée de `merge_disposition`.
        assert!(owns_merge_transition("  MIKA-DEV "));
    }

    #[test]
    fn mika2260_la_porte_dentree_et_lacteur_ne_peuvent_pas_diverger() {
        // Deux noms, deux questions, **une** table. Ce test est ce qui rougit le
        // jour où l'une des deux fonctions bouge sans l'autre — le mode de panne
        // qu'une seconde liste blanche rendrait silencieux.
        //
        // **Ce qu'il vaut, et ce qu'il ne vaut pas.** Tant que
        // `owns_merge_transition` est défini *par délégation* à
        // `merge_disposition`, cette égalité est une tautologie et ce test ne peut
        // pas échouer. Sa valeur est conditionnelle et future : il mord le jour où
        // quelqu'un réécrit le prédicat autrement. Et même alors il ne mord que
        // sur les entrées échantillonnées — une réimplémentation en
        // `trim().eq_ignore_ascii_case(DISPATCHER_AGENT)` les satisfait toutes les
        // huit et resterait verte. L'idiome maison pour fermer cela entièrement
        // serait un scan de source refusant un second lecteur de la liste blanche ;
        // il n'est pas livré, parce qu'un seul site consomme ce prédicat
        // aujourd'hui et qu'un scan sur une population d'un est un détecteur dont
        // le silence ne prouve rien. C'est dit ici plutôt que découvert plus tard.
        for agent in [
            "mika-dev",
            "mika-qa",
            "mika",
            "mika-arch",
            "customer-42",
            "",
            "mika-dev-2",
            "  MIKA-DEV ",
        ] {
            assert_eq!(
                owns_merge_transition(agent),
                merge_disposition(agent) == MergeDisposition::Act,
                "`{agent}` : la porte d'entrée et l'acteur doivent lire la même liste blanche"
            );
        }
    }

    #[test]
    fn lidentite_est_comparee_normalisee() {
        assert!(is_dispatcher_agent("  MIKA-DEV "));
        assert!(is_reviewer_agent("Mika-QA"));
        assert!(!is_dispatcher_agent("mika-deva"));
        assert!(!is_reviewer_agent("mika-qa-2"));
    }

    #[test]
    fn mika2244_le_relecteur_aurait_merge_sous_son_propre_login() {
        // Le contrôle positif : la forme exacte mesurée sur mika#2244.
        assert!(would_merge_as_reviewer("mika-qa", "mika-platform-qa"));
        assert!(would_merge_as_reviewer("mika-qa", "@mika-platform-qa"));
        assert!(would_merge_as_reviewer("mika-qa", " Mika-Platform-QA "));
    }

    #[test]
    fn le_dispatcher_ne_merge_pas_sous_le_login_du_relecteur() {
        // Le contrôle négatif dans le même souffle : sans lui, une fonction qui
        // rendrait toujours `true` passerait le test ci-dessus.
        assert!(!would_merge_as_reviewer("mika-dev", "mika-platform-qa"));
        assert!(!would_merge_as_reviewer("mika-qa", "samidarko"));
    }

    #[test]
    fn mika2667_seul_le_login_de_revue_est_reconnu() {
        for login in [
            "mika-platform-qa",
            "@Mika-Platform-QA",
            " mika-platform-qa[bot] ",
        ] {
            assert!(is_reviewer_forge_login(login), "`{login}` est le relecteur");
        }
        // Contrôle négatif dans le même souffle : le dispatcher, le nom d'agent
        // (pas un login de forge), un voisin lexical, le vide.
        for login in [
            DISPATCHER_FORGE_LOGIN,
            "mika-qa",
            "mika-platform-qa-2",
            "",
            "[bot]",
        ] {
            assert!(
                !is_reviewer_forge_login(login),
                "`{login}` n'est pas le relecteur"
            );
        }
    }

    #[test]
    fn le_signal_fait_laller_retour() {
        let signal = MergeReadySignal {
            repo: "senara-solutions/mika".to_string(),
            pr_number: 2248,
            head_sha: "abc123def456".to_string(),
            branch: "fix/2248/merge-under-dispatcher-identity-not-reviewer".to_string(),
            task_id: "task-7".to_string(),
            reviewer_login: "mika-platform-qa".to_string(),
        };
        let rendered = signal.to_string();
        assert_eq!(parse_merge_ready_signal(&rendered), Some(signal));
    }

    #[test]
    fn le_signal_se_lit_au_milieu_dun_pre_digest() {
        let text = format!(
            "<ci_success_handler>\n\
             [GitHub] Check suite success on senara-solutions/mika#2248 (branch: fix/x)\n\
             {}\n\
             Le dispatcher prend la main.\n\
             </ci_success_handler>",
            MergeReadySignal {
                repo: "senara-solutions/mika".to_string(),
                pr_number: 2248,
                head_sha: "abc123".to_string(),
                branch: "fix/x".to_string(),
                task_id: "none".to_string(),
                reviewer_login: "mika-platform-qa".to_string(),
            }
        );
        let parsed = parse_merge_ready_signal(&text).expect("le marqueur est présent");
        assert_eq!(parsed.pr_number, 2248);
        assert_eq!(parsed.repo, "senara-solutions/mika");
        assert_eq!(parsed.task_id, "none");
    }

    #[test]
    fn un_signal_absent_ou_partiel_nautorise_rien() {
        assert!(parse_merge_ready_signal("").is_none());
        assert!(parse_merge_ready_signal("rien à voir ici").is_none());
        // Marqueur présent, champs manquants : aucune autorisation.
        assert!(parse_merge_ready_signal("MERGE-READY-SIGNAL: senara-solutions/mika#1").is_none());
        assert!(
            parse_merge_ready_signal(
                "MERGE-READY-SIGNAL: senara-solutions/mika#1 head=abc branch=b task=t"
            )
            .is_none(),
            "sans `reviewer=`, la ceinture d'identité ne peut pas être bouclée"
        );
        // Cible mal formée.
        assert!(
            parse_merge_ready_signal(
                "MERGE-READY-SIGNAL: mika#1 head=abc branch=b task=t reviewer=r"
            )
            .is_none(),
            "un `repo` sans `owner/` n'est pas une cible gh valide"
        );
        assert!(
            parse_merge_ready_signal(
                "MERGE-READY-SIGNAL: senara-solutions/mika#xx head=abc branch=b task=t reviewer=r"
            )
            .is_none()
        );
    }
}

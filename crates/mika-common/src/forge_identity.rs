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

// ---------------------------------------------------------------------------
// Gate MPC — la trace d'une revue orchestrateur sur la tête courante (mika#2617, U5/AC5)
// ---------------------------------------------------------------------------

/// Préfixe du marqueur par lequel MPC atteste qu'il a statué sur une tête.
///
/// Forme complète : `<!-- mpc-gate: ok sha=<SHA complet de la tête> -->`, dans un
/// commentaire de PR. Format retenu par MPC (corps de mika#2617, AC5) parce que
/// le compte `gh` de MPC est celui de l'opérateur : ni l'auteur d'une review ni
/// l'acteur d'un `ReadyForReviewEvent` ne distinguent MPC d'un humain, et aucun
/// des deux ne porte le SHA évalué. Le marqueur, lui, le porte — donc tout push
/// ultérieur l'invalide de lui-même.
///
/// **L'auteur du commentaire n'est pas lu, et c'est délibéré** : il serait
/// Vincent dans les deux cas. La garde atteste « quelqu'un a statué sur CETTE
/// tête », pas « MPC et personne d'autre ».
pub const MPC_GATE_MARKER_PREFIX: &str = "<!-- mpc-gate: ok sha=";

/// Fin obligatoire du marqueur. Un `sha=` suivi d'autre chose qu'un hexadécimal
/// puis `-->` n'est pas un marqueur — c'est de la prose qui en parle.
const MPC_GATE_MARKER_SUFFIX: &str = "-->";

/// Les dépôts dont le merge autonome exige la trace de gate MPC.
///
/// Vit ici, à côté de [`merge_disposition`], parce que c'est la même famille de
/// décision et qu'une seconde liste de politique de merge est la divergence
/// programmée que ce dépôt a payée deux fois (sièges mika#2092,
/// `DISPATCHABLE_REPOS` ↔ `labels.yml`).
///
/// # Le nom courant, ET l'ancien — rectification de prémisse du 2026-10-05
///
/// Le plan écrivait `senara-solutions/claude-pilot-py`. Le dépôt a été **renommé**
/// `senara-solutions/claude-pilot` (mesuré : `gh repo view
/// senara-solutions/claude-pilot-py` rend `https://github.com/senara-solutions/claude-pilot`).
/// GitHub envoie le **nom courant** dans les webhooks : une population réduite à
/// l'ancien nom ne correspondrait jamais, et la garde serait inerte, c'est-à-dire
/// ouverte (classe mika#2205). Le nom courant est donc le terme porteur, épinglé
/// par un test.
///
/// **L'ancien nom est gardé comme alias, et ce n'est pas de la nostalgie.** GitHub
/// redirige un dépôt renommé : `gh pr merge <n> --repo senara-solutions/claude-pilot-py`
/// atteint la même PR. Un appelant qui écrit encore l'ancien nom — un prompt
/// périmé, `INTERNAL_REPOS` du gateway qui le liste toujours — ne doit pas
/// trouver la porte ouverte pour autant. Retirer l'alias rouvrirait ce chemin ;
/// le garder ne coûte rien, puisque la population ne grossit pas.
///
/// **Ajouter un dépôt ici ajoute un aller-retour `gh` à chacun de ses merges**
/// (plan R10) — à savoir avant d'y toucher, pas à découvrir après.
pub const MPC_GATE_REQUIRED_REPOS: &[&str] = &[
    "senara-solutions/claude-pilot",
    "senara-solutions/claude-pilot-py",
];

/// Vrai quand le merge autonome de `repo` exige la trace de gate MPC.
///
/// Sans casse et bords rognés : GitHub résout `owner/repo` sans tenir compte de
/// la casse, donc une comparaison sensible laisserait `Senara-Solutions/…`
/// atteindre la même PR par une porte ouverte.
pub fn mpc_gate_required(repo: &str) -> bool {
    let repo = repo.trim();
    MPC_GATE_REQUIRED_REPOS
        .iter()
        .any(|r| r.eq_ignore_ascii_case(repo))
}

/// Ce que la trace de gate MPC dit d'une tête.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpcGateVerdict {
    /// Le dépôt n'est pas dans [`MPC_GATE_REQUIRED_REPOS`] : le terme ne mord pas.
    NotRequired,
    /// Un marqueur porte exactement le SHA de la tête.
    Attested,
    /// Aucun marqueur lisible — ou commentaires illisibles, ou tête inconnue.
    /// **Fail-closed** : c'est une précondition supplémentaire sur une petite
    /// population, et une précondition qu'on ne sait pas lire n'en est pas une.
    Missing,
    /// Au moins un marqueur, aucun sur la tête courante : MPC a statué sur un
    /// commit que la PR ne porte plus.
    StaleSha {
        /// Le SHA du marqueur le plus récent.
        attested: String,
        /// La tête courante.
        head: String,
    },
}

impl MpcGateVerdict {
    /// Le nom de fil du refus, ou `None` quand le terme autorise.
    ///
    /// Ce sont les noms que porte `BlockReason` côté outil — une seule
    /// définition, lue par les trois sites de merge.
    pub fn refusal_reason(&self) -> Option<&'static str> {
        match self {
            Self::NotRequired | Self::Attested => None,
            Self::Missing => Some("mpc_gate_missing"),
            Self::StaleSha { .. } => Some("mpc_gate_stale_sha"),
        }
    }
}

/// Les SHA attestés par les marqueurs de gate MPC, dans l'ordre des commentaires.
///
/// Pure. **Le marqueur n'est jamais cherché dans du code** — ni bloc clôturé
/// (```` ``` ```` / `~~~`), ni span en ligne (`` ` ``). Raison mesurée par mika#2050
/// sur le Signal S : un commentaire qui **discute** du format (le corps de
/// mika#2617 le cite) porte le littéral et serait lu comme une attestation. Un
/// vrai marqueur est un commentaire HTML, invisible au rendu ; dans du code il
/// est affiché, donc c'est de la prose qui en parle.
///
/// Un `sha=` qui n'est pas suivi d'hexadécimal puis de `-->` est ignoré. Un SHA
/// tronqué est extrait tel quel : c'est l'égalité de [`mpc_gate_verdict`] qui le
/// refuse, avec un motif qui le montre, plutôt qu'un silence.
pub fn extract_mpc_gate_shas(comments: &[String]) -> Vec<String> {
    comments
        .iter()
        .flat_map(|body| shas_in_prose(&strip_code(body)))
        .collect()
}

/// Les SHA des marqueurs d'un texte déjà débarrassé de son code.
fn shas_in_prose(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(MPC_GATE_MARKER_PREFIX) {
        rest = &rest[at + MPC_GATE_MARKER_PREFIX.len()..];
        let hex_len = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        if hex_len == 0 {
            continue;
        }
        if rest[hex_len..]
            .trim_start_matches([' ', '\t'])
            .starts_with(MPC_GATE_MARKER_SUFFIX)
        {
            out.push(rest[..hex_len].to_ascii_lowercase());
        }
    }
    out
}

/// Le texte d'un commentaire sans ses blocs clôturés ni ses spans de code.
///
/// Un bloc non refermé court jusqu'à la fin du commentaire — comme au rendu
/// GitHub, et dans le sens fail-closed : le marqueur qu'il contiendrait n'est
/// pas lu.
fn strip_code(body: &str) -> String {
    let mut prose = String::with_capacity(body.len());
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let opener = ['`', '~'].into_iter().find_map(|c| {
            let n = trimmed.chars().take_while(|&x| x == c).count();
            (n >= 3).then_some((c, n))
        });
        match (fence, opener) {
            (None, Some(open)) => fence = Some(open),
            (Some((c, n)), Some((c2, n2))) if c == c2 && n2 >= n => fence = None,
            (Some(_), _) => {}
            (None, None) => {
                prose.push_str(&strip_inline_code(line));
                prose.push('\n');
            }
        }
    }
    prose
}

/// Une ligne sans ses spans de code en ligne (une suite de N accents graves
/// ouvre, la prochaine suite d'exactement N ferme). Une suite sans fermeture est
/// laissée telle quelle, comme au rendu.
fn strip_inline_code(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let run_at = |i: usize| chars[i..].iter().take_while(|&&c| c == '`').count();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let n = run_at(i);
        let mut j = i + n;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let m = run_at(j);
                if m == n {
                    close = Some(j + m);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => i = end,
            None => {
                out.extend(&chars[i..i + n]);
                i += n;
            }
        }
    }
    out
}

/// Le verdict du gate MPC pour `repo` à la tête `head_sha`.
///
/// **L'ordre des termes est la propriété, pas une discipline d'appelant.** Le
/// terme de population est testé **en premier** : hors population le verdict est
/// [`MpcGateVerdict::NotRequired`] quels que soient les commentaires, et
/// l'évaluateur paresseux côté agent n'invoque pas le lecteur réseau (plan R10).
///
/// Égalité **stricte sur le SHA complet** (casse hexadécimale normalisée) : un
/// préfixe ne vaut pas attestation — un SHA abrégé désigne un commit par
/// ambiguïté résolue, et ce que la garde achète, c'est qu'un push ultérieur
/// l'invalide de lui-même.
pub fn mpc_gate_verdict(repo: &str, head_sha: &str, comments: &[String]) -> MpcGateVerdict {
    if !mpc_gate_required(repo) {
        return MpcGateVerdict::NotRequired;
    }
    let head = head_sha.trim().to_ascii_lowercase();
    if head.is_empty() {
        return MpcGateVerdict::Missing;
    }
    let shas = extract_mpc_gate_shas(comments);
    if shas.contains(&head) {
        return MpcGateVerdict::Attested;
    }
    match shas.into_iter().last() {
        Some(attested) => MpcGateVerdict::StaleSha { attested, head },
        None => MpcGateVerdict::Missing,
    }
}

/// Le témoin de type qu'exige `run_gh_merge` : la preuve que le gate MPC a été
/// évalué et qu'il autorise ce merge.
///
/// **Le champ est privé et le seul constructeur est [`Self::from_mpc_verdict`]**,
/// qui ne rend `Some` que sur `NotRequired` et `Attested`. Les trois sites de
/// merge (l'outil, `verdict_handler`, `merge_ready_handler`) sont donc forcés
/// **par le compilateur**, sans scan de source : poser AC5 dans l'outil seul
/// l'aurait laissée contournable par les deux handlers (plan R8). Doctrine
/// mika#1991 : construire l'incapacité, ne pas promettre la retenue.
///
/// Ni `Clone` ni `Copy`, et consommé par valeur : un témoin sert un merge, pas
/// deux.
#[derive(Debug)]
pub struct MergeClearance {
    _sealed: (),
}

impl MergeClearance {
    /// `Some` seulement quand le verdict autorise le merge.
    pub fn from_mpc_verdict(verdict: &MpcGateVerdict) -> Option<Self> {
        match verdict {
            MpcGateVerdict::NotRequired | MpcGateVerdict::Attested => Some(Self { _sealed: () }),
            MpcGateVerdict::Missing | MpcGateVerdict::StaleSha { .. } => None,
        }
    }
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

/// Gate MPC (mika#2617, U5/AC5). Contrôles négatifs déterministes : SHA
/// périmé ⇒ refus, marqueur dans du code ⇒ ignoré, dépôt hors population ⇒
/// `NotRequired` quels que soient les commentaires.
#[cfg(test)]
mod mpc_gate_tests {
    use super::*;

    const HEAD: &str = "8ccaabc8d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6";
    const OLD: &str = "41a4a20e00112233445566778899aabbccddeeff";
    const CP: &str = "senara-solutions/claude-pilot";

    fn marker(sha: &str) -> String {
        format!("{MPC_GATE_MARKER_PREFIX}{sha} -->")
    }

    fn c(bodies: &[&str]) -> Vec<String> {
        bodies.iter().map(|b| (*b).to_string()).collect()
    }

    /// Rectification de prémisse du 2026-10-05 : le dépôt a été renommé, et
    /// GitHub envoie le nom courant. Si ce test rougit, la garde est inerte
    /// pour toute la population qu'elle existe pour garder (classe mika#2205).
    #[test]
    fn mika2617_the_current_repo_name_is_in_the_population() {
        assert!(mpc_gate_required("senara-solutions/claude-pilot"));
        // L'alias : GitHub redirige l'ancien nom vers la même PR.
        assert!(mpc_gate_required("senara-solutions/claude-pilot-py"));
        // GitHub résout sans casse.
        assert!(mpc_gate_required(" Senara-Solutions/Claude-Pilot "));
    }

    #[test]
    fn mika2617_the_population_does_not_leak() {
        for repo in [
            "senara-solutions/mika",
            "senara-solutions/mika-cloud",
            "senara-solutions/mika-skills",
            "senara-solutions/claude-pilot-ts",
            "senara-solutions/claude-pilot-extra",
            "other-org/claude-pilot",
            "",
        ] {
            assert!(
                !mpc_gate_required(repo),
                "{repo} ne doit pas exiger le gate MPC"
            );
        }
    }

    #[test]
    fn mika2617_extract_mpc_gate_shas_nominal() {
        let body = format!("Gate MPC : OK.\n\n{}", marker(HEAD));
        assert_eq!(extract_mpc_gate_shas(&c(&[&body])), vec![HEAD.to_string()]);
    }

    #[test]
    fn mika2617_extract_mpc_gate_shas_several_comments_in_order() {
        let a = marker(OLD);
        let b = format!("relu après push\n{}", marker(HEAD));
        assert_eq!(
            extract_mpc_gate_shas(&c(&["sans rapport", &a, &b])),
            vec![OLD.to_string(), HEAD.to_string()]
        );
    }

    #[test]
    fn mika2617_extract_mpc_gate_shas_keeps_a_truncated_sha_for_the_verdict_to_refuse() {
        let body = marker("8ccaabc8");
        assert_eq!(
            extract_mpc_gate_shas(&c(&[&body])),
            vec!["8ccaabc8".to_string()]
        );
    }

    #[test]
    fn mika2617_extract_mpc_gate_shas_requires_the_closing_delimiter() {
        let prose = [
            format!("{MPC_GATE_MARKER_PREFIX}{HEAD}"),
            format!("{MPC_GATE_MARKER_PREFIX}{HEAD} et la suite"),
            format!("{MPC_GATE_MARKER_PREFIX}<SHA complet de la tête> -->"),
        ];
        for body in prose {
            assert!(
                extract_mpc_gate_shas(std::slice::from_ref(&body)).is_empty(),
                "{body:?}"
            );
        }
    }

    /// **Contrôle négatif** (mika#2050) : un commentaire qui DISCUTE du format
    /// porte le littéral. Dans un bloc clôturé ou un span en ligne, il est
    /// affiché — ce n'est pas une attestation.
    #[test]
    fn mika2617_extract_mpc_gate_shas_ignores_a_marker_in_a_code_block() {
        let fenced = format!("Format :\n```\n{}\n```\nfin", marker(HEAD));
        let tilde = format!("~~~md\n{}\n~~~", marker(HEAD));
        let unclosed = format!("```\n{}", marker(HEAD));
        let inline = format!("Le marqueur s'écrit `{}`.", marker(HEAD));
        let double = format!("``{}``", marker(HEAD));
        let indented_fence = format!("- item\n  ```\n  {}\n  ```", marker(HEAD));
        for body in [fenced, tilde, unclosed, inline, double, indented_fence] {
            assert!(
                extract_mpc_gate_shas(std::slice::from_ref(&body)).is_empty(),
                "marqueur dans du code lu comme attestation : {body:?}"
            );
        }
        // Contrôle positif dans le même appel : le même marqueur, hors code,
        // après un bloc refermé, est lu.
        let after = format!("```\nx\n```\n{}", marker(HEAD));
        assert_eq!(extract_mpc_gate_shas(&[after]), vec![HEAD.to_string()]);
    }

    /// Le corps du ticket mika#2617 lui-même cite le littéral : il ne doit
    /// rien attester.
    #[test]
    fn mika2617_the_ticket_body_quoting_the_format_attests_nothing() {
        let body = "Format retenu (décision MPC) : un commentaire de PR contenant le \
                    marqueur `<!-- mpc-gate: ok sha=<SHA complet de la tête> -->`.";
        assert!(extract_mpc_gate_shas(&c(&[body])).is_empty());
    }

    #[test]
    fn mika2617_mpc_gate_verdict_not_required_outside_the_population() {
        // Aucun marqueur, et même un marqueur périmé : hors population, rien ne mord.
        assert_eq!(
            mpc_gate_verdict("senara-solutions/mika", HEAD, &[]),
            MpcGateVerdict::NotRequired
        );
        assert_eq!(
            mpc_gate_verdict("senara-solutions/mika", "", &c(&[&marker(OLD)])),
            MpcGateVerdict::NotRequired
        );
    }

    #[test]
    fn mika2617_mpc_gate_verdict_attested_on_equality() {
        let comments = c(&[&marker(OLD), &marker(HEAD)]);
        assert_eq!(
            mpc_gate_verdict(CP, HEAD, &comments),
            MpcGateVerdict::Attested
        );
        // Casse hexadécimale normalisée des deux côtés.
        let upper = c(&[&marker(&HEAD.to_ascii_uppercase())]);
        assert_eq!(mpc_gate_verdict(CP, HEAD, &upper), MpcGateVerdict::Attested);
    }

    #[test]
    fn mika2617_mpc_gate_verdict_stale_sha_on_divergence() {
        assert_eq!(
            mpc_gate_verdict(CP, HEAD, &c(&[&marker(OLD)])),
            MpcGateVerdict::StaleSha {
                attested: OLD.to_string(),
                head: HEAD.to_string()
            }
        );
        // Un préfixe de la tête n'est PAS la tête.
        assert_eq!(
            mpc_gate_verdict(CP, HEAD, &c(&[&marker(&HEAD[..8])])),
            MpcGateVerdict::StaleSha {
                attested: HEAD[..8].to_string(),
                head: HEAD.to_string()
            }
        );
    }

    #[test]
    fn mika2617_mpc_gate_verdict_missing_on_absence_and_on_unknown_head() {
        assert_eq!(mpc_gate_verdict(CP, HEAD, &[]), MpcGateVerdict::Missing);
        assert_eq!(
            mpc_gate_verdict(CP, HEAD, &c(&["LGTM", "gate ok"])),
            MpcGateVerdict::Missing
        );
        // Tête illisible : fail-closed, même avec un marqueur présent.
        assert_eq!(
            mpc_gate_verdict(CP, "  ", &c(&[&marker(HEAD)])),
            MpcGateVerdict::Missing
        );
        // Marqueur dans du code seulement : Missing, pas Attested.
        let fenced = format!("```\n{}\n```", marker(HEAD));
        assert_eq!(
            mpc_gate_verdict(CP, HEAD, &c(&[&fenced])),
            MpcGateVerdict::Missing
        );
    }

    #[test]
    fn mika2617_merge_clearance_is_not_constructible_from_a_refusal() {
        assert!(MergeClearance::from_mpc_verdict(&MpcGateVerdict::Missing).is_none());
        assert!(
            MergeClearance::from_mpc_verdict(&MpcGateVerdict::StaleSha {
                attested: OLD.to_string(),
                head: HEAD.to_string(),
            })
            .is_none()
        );
        // Contrôles positifs : sans eux, « le témoin ne se construit jamais »
        // passerait pour « le témoin refuse bien ».
        assert!(MergeClearance::from_mpc_verdict(&MpcGateVerdict::NotRequired).is_some());
        assert!(MergeClearance::from_mpc_verdict(&MpcGateVerdict::Attested).is_some());
    }

    #[test]
    fn mika2617_refusal_reason_matches_the_clearance() {
        for v in [
            MpcGateVerdict::NotRequired,
            MpcGateVerdict::Attested,
            MpcGateVerdict::Missing,
            MpcGateVerdict::StaleSha {
                attested: OLD.to_string(),
                head: HEAD.to_string(),
            },
        ] {
            assert_eq!(
                v.refusal_reason().is_none(),
                MergeClearance::from_mpc_verdict(&v).is_some(),
                "{v:?} : le nom de refus et le témoin divergent"
            );
        }
    }
}

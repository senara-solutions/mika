//! AC1 **sous vrai modèle** — mika#2854, vol 3/3.
//!
//! # Ce que ce fichier mesure, et ce qu'aucun bouchon ne pouvait rendre
//!
//! Le vol 1 a livré la moitié mécanique d'AC1 : le corpus DISCRIMINE, mesuré
//! sous un bouchon dégénéré qui note tout
//! (`mika2854_h_le_corpus_dordres_est_discriminant`). Son commentaire dit, mot
//! pour mot, pourquoi il s'arrêtait là — *« aucun bouchon ne peut témoigner de
//! l'obéissance d'un gabarit (discipline mika#1190) »*. La moitié verte, celle
//! qui dit que **le vrai modèle escalade**, ne peut venir que d'un appel réel.
//! C'est ce fichier.
//!
//! Trois propriétés de forme, chacune délibérée :
//!
//! 1. **Le corpus est verbatim.** Chaque texte ci-dessous est le corps réel
//!    d'un commentaire ou d'une fermeture du dépôt, remis dans le gabarit du
//!    gateway ([`commentaire`], [`fermeture`]), avec son identifiant de
//!    commentaire en provenance. Un texte reformulé mesurerait la reformulation.
//! 2. **Le chemin est le vrai chemin.** Les cas passent par [`decide`], donc
//!    par la machinerie qui prend les branches, et non par
//!    `decide_comment_triage` ni par `run_triage` en direct. Un test qui
//!    appellerait le triage seul serait vert même si le décideur se trompait de
//!    porte.
//! 3. **`TRIAGE_SYSTEM_PROMPT` n'est pas touché** — zéro ligne. Mesurer un
//!    gabarit et le corriger dans le même vol, c'est mesurer deux fois rien
//!    (mika#1190). Un résultat rouge ici est un constat à rendre à Vincent,
//!    pas une invitation à retoucher le prompt.
//!
//! # Comment on le joue
//!
//! ```text
//! MIKA_EVAL_TRIAGE_PROVIDERS=anthropic \
//!   cargo test -p mika-agent --lib -- --ignored --nocapture ac1_vrai_modele_2854
//! ```
//!
//! `#[ignore]` + variable d'environnement dédiée, le motif des matrices de
//! fournisseurs réels du dépôt. Elle est **dédiée** et non partagée avec
//! `MIKA_EVAL_REAL_PROVIDERS` : une matrice d'agent lancée pour autre chose ne
//! doit pas dépenser onze appels de triage à l'insu de qui la lance. Sans clé
//! d'API, [`crate::calibration::providers::create_real_provider`] rend `None`
//! et le test se saute en le DISANT — jamais en passant vert pour rien.
//!
//! # Pourquoi l'instrument vit ici, en intra-caisse
//!
//! `decide_comment_triage` est privé, et `PrefilterState` est `pub(crate)` :
//! une intégration sous `tests/` ne peut atteindre ni l'un ni l'autre, donc ne
//! pourrait mesurer que le triage hors de son décideur — exactement ce que la
//! propriété 2 refuse. Enfant de `webhook_prefilter::tests`, donc descendant de
//! `webhook_prefilter` : les items privés du module et les helpers de son `mod
//! tests` restent visibles via `use super::*` (même idiome que `db/tests/`,
//! mika#2321 — et un segment `/tests/` dans le chemin est ce qui garde ce
//! fichier `Mechanical` pour le périmètre).
//!
//! # Provenance, et la dérive qu'on nomme plutôt que de la cacher
//!
//! Le ticket demande *« les décisions du 2026-10-20 sur #2784, #2680, #2814 et
//! #1381 »*. Relevé le 2026-10-22 : #2784 et #2680 portent bien trois
//! commentaires de décision de ce jour (D1, D2, D3). **#2814 et #1381 n'en
//! portent aucun** — #2814 n'a que deux refus d'auto-pull du 2026-10-19, #1381
//! aucun commentaire sur la fenêtre. Sur ces deux tickets, l'événement porteur
//! de décision est leur **fermeture** (2026-10-21T10:26:01Z et 10:26:48Z), ce
//! qui tombe sur le mot « fermer » de l'énumération d'AC1 et exerce la moitié
//! `issues closed` d'AC4. Ils sont donc entrés comme fermetures (C1, C2), et
//! cette dérive est dite ici plutôt que réparée par un texte inventé.
//!
//! Deux ordres réels de la même file complètent l'énumération d'AC1 là où
//! #2814 et #1381 ne pouvaient pas : une levée de porte (D4, #2699) et une
//! correction de trajectoire avec jalon déplacé (D5, #1785).
//!
//! # Fidélité des fixtures, et dans quel sens elle penche
//!
//! Les corps longs sont tronqués à la frontière d'un paragraphe (le gateway
//! tronque de son côté à 2 000 caractères), et le corps d'une fermeture ne
//! porte ici que la tête du corps du ticket. Les deux coupes vont dans le même
//! sens : **moins de texte où reconnaître une décision**, donc un test plus
//! strict que la production. L'erreur possible est un faux rouge, jamais un
//! faux vert.

use std::sync::Mutex;

use mika_common::llm::{LlmContent, LlmMessage, LlmRequest, LlmRole, ProviderKind};

use super::*;

/// La porte de ce vol. Dédiée, pour la raison dite dans l'en-tête.
const PROVIDERS_ENV: &str = "MIKA_EVAL_TRIAGE_PROVIDERS";

/// Les fournisseurs demandés, ou rien. Même grammaire que les autres matrices
/// du dépôt (`anthropic,openai`, ou `all`).
fn fournisseurs_demandes() -> Vec<ProviderKind> {
    match std::env::var(PROVIDERS_ENV) {
        Ok(v) if !v.trim().is_empty() => crate::calibration::providers::parse_provider_list(&v),
        _ => Vec::new(),
    }
}

// ---- les gabarits du gateway, à leur seul endroit ------------------------
//
// Recopiés de `mika-gateway/src/github.rs` : `issue_comment` (l. 486) et
// `issues` (l. 457). Deux fonctions et non deux `format!` en ligne, parce que
// onze cas écrits à la main sont onze chances d'écrire un gabarit qui n'est
// plus celui du gateway — et un fixture hors gabarit sort de la population de
// (h) et ne mesure rien du tout (la garde `classify` plus bas le dit).

/// La moitié `issue_comment`. Le fragment `#issuecomment-<id>` est présent
/// parce que le gateway le délivre, et il porte la provenance du cas.
fn commentaire(issue: u64, titre: &str, auteur: &str, comment_id: u64, corps: &str) -> String {
    format!(
        "[GitHub] New comment on senara-solutions/mika#{issue} ({titre}) by @{auteur}\n\
         https://github.com/senara-solutions/mika/issues/{issue}#issuecomment-{comment_id}\n\n\
         {corps}"
    )
}

/// La moitié `issues` `closed`. Le corps n'est ajouté que s'il est non vide,
/// comme côté gateway.
fn fermeture(issue: u64, titre: &str, corps: &str) -> String {
    let mut texte = format!(
        "[GitHub] Issue closed: senara-solutions/mika#{issue} — {titre}\n\
         https://github.com/senara-solutions/mika/issues/{issue}"
    );
    if !corps.is_empty() {
        texte.push_str("\n\n");
        texte.push_str(corps);
    }
    texte
}

/// Ce qu'on exige du cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attendu {
    /// AC1 : le tour complet. Aucune tolérance — *« une seule décision
    /// dégradée en note est un échec d'AC1, pas un taux acceptable »*.
    Escalade,
    /// AC2 : une note, sans tour complet.
    Note,
    /// Mesuré et rendu chiffré, **hors assertion**. Réservé au bruit
    /// d'automate dont le corps porte une section impérative (« ce qu'il
    /// faudrait pour passer ») : le prompt le range dans « du bruit d'automate »
    /// *et* dans « un ordre ou une consigne », et sa règle de départage est
    /// « dans le doute, réponds DECISION ». Les deux branches sont donc
    /// conformes au gabarit, et laquelle tombe est précisément ce que Vincent
    /// doit voir avant d'armer — pas ce qu'un test doit décréter.
    Mesure,
}

/// Un cas du corpus.
struct Cas {
    /// L'étiquette rendue dans le tableau chiffré.
    id: &'static str,
    /// D'où vient le texte, mot pour mot vérifiable.
    provenance: &'static str,
    attendu: Attendu,
    texte: String,
}

/// Le corpus réel, figé.
///
/// Les cinq premiers sont des commentaires de décision verbatim, les deux
/// suivants des fermetures, les deux derniers des contrôles positifs d'AC2, et
/// les deux derniers du lot sont le bruit d'automate mesuré hors assertion.
fn corpus_reel() -> Vec<Cas> {
    vec![
        // ---- AC1 : les ordres. Chacun doit escalader. -------------------
        Cas {
            id: "D1",
            provenance: "#2784 comment 6294236188, samidarko, 2026-10-20T15:14:37Z — « je valide » + Q1/Q2 tranchées",
            attendu: Attendu::Escalade,
            texte: commentaire(
                2784,
                "fix(agent): le chemin fail-closed echoue encore par denylist — le durcir en allowlist",
                "samidarko",
                6294236188,
                "## Décision opérateur (Vincent, 2026-10-20) : questions 1 et 2\n\n\
                 Tranchées sur bearing Prime, validé par Vincent (« je valide »).\n\n\
                 1. **Question 1 : aucun builtin en fail-closed, pas même `send_message`.** `allowed = Some(vec![])`. Une identité malformée ne dit rien de fiable sur ce que l'agent devrait pouvoir faire ; le signal à l'opérateur vient de la couche au-dessus (la ligne de journal de posture livrée par #2838), pas de l'agent dégradé. « La posture se signale ; l'agent se tait. »\n\
                 2. **Question 2 : oui, le fail-closed couvre aussi les agents utilisateurs.** Un fichier disparu ne permet plus d'attester ce qu'est l'agent, quel que soit son propriétaire. Pas d'exemption par catégorie. En contrepartie, la restauration doit être rapide et visible : la ligne de journal doit nommer l'agent et le fichier manquant. « Ferme dur, répare vite. »\n\n\
                 Le resserrement de `allowed` (vol 2) est donc groomable.",
            ),
        },
        Cas {
            id: "D2",
            provenance: "#2680 comment 6294284976, samidarko, 2026-10-20T15:16:54Z — réponse (c) à la question posée à l'opérateur, classe (h)",
            attendu: Attendu::Escalade,
            texte: commentaire(
                2680,
                "Pré-filtre webhook, phase 4 : tours à vide sur closed/merged, review sur PR fermée et check_suite de branche côté QA (mesure)",
                "samidarko",
                6294284976,
                "## Décision opérateur (Vincent, 2026-10-20) : classe (h)\n\n\
                 Question du 2026-10-08 : que doit faire le tour quand un commentaire d'opérateur (ou une fermeture) arrive sur un ticket sans tâche de boucle ?\n\n\
                 **Réponse : (c), un tour léger.** Choix de Vincent, confirmé par bearing Prime avec une réserve qui en fixe le dessin :\n\n\
                 - « léger » ne veut pas dire « non lu ». Le tour léger **classe** l'événement : porte-t-il une décision actionnable (lever une porte, fermer, lancer un vol…) ?\n\
                 - Si oui, il escalade vers un tour complet. Sinon, il consigne en mémoire et se tait.\n\
                 - Le discriminateur est **la présence d'une décision**, pas la longueur. Le seul brin de LLM gardé est un triage, pas un raisonnement. Le danger à tester : un ordre d'opérateur dégradé silencieusement en simple note de mémoire.\n\n\
                 La classe (h) est donc retenue et groomable. (g′) reste ouverte.",
            ),
        },
        Cas {
            id: "D3",
            provenance: "#2680 comment 6294369761, samidarko, 2026-10-20T15:20:54Z — réponse (a), « ouvrir le vol d'instrument »",
            attendu: Attendu::Escalade,
            texte: commentaire(
                2680,
                "Pré-filtre webhook, phase 4 : tours à vide sur closed/merged, review sur PR fermée et check_suite de branche côté QA (mesure)",
                "samidarko",
                6294369761,
                "## Décision opérateur (Vincent, 2026-10-20) : classe (g′)\n\n\
                 **Réponse : (a), ouvrir le vol d'instrument.** Choix de Vincent, confirmé par bearing Prime (« demander la cause, pas la vraisemblance »). Le vol n'écarte aucun tour ; la porte mika#2260 reste entière.\n\n\
                 Réserve de Prime, qui fixe le dessin du champ : il doit mesurer ce qui décidera le filtre, pas seulement le volume.\n\
                 - nommer **lequel** des arrêts (1 à 4) a terminé le tour, ou « revue » ;\n\
                 - distinguer l'arrêt propre du `Release::StateUnreadable` (taux nécessaire à la posture fail-open, AC3 de mika#2675).\n\n\
                 Ainsi (g′) se mesure en un seul vol, et les trois arbitrages (PAT, deux sites de vérité, fail-open) se poseront sur du mesuré.",
            ),
        },
        Cas {
            id: "D4",
            provenance: "#2699 comment 6254278671, samidarko, 2026-10-18T19:40:13Z — levée de porte + choix (a)",
            attendu: Attendu::Escalade,
            texte: commentaire(
                2699,
                "La boucle construit son propre tableau de bord",
                "samidarko",
                6254278671,
                "Porte levée sur décision de Vincent (18/10/2026) : la boucle construit (a) en ombre. L'interrupteur d'écriture reste OFF par défaut et ne s'ouvre que sur le mot de Vincent.",
            ),
        },
        Cas {
            id: "D5",
            provenance: "#1785 comment 6254304327, samidarko, 2026-10-18T19:42:50Z — correction de trajectoire, jalon déplacé, file tenue",
            attendu: Attendu::Escalade,
            texte: commentaire(
                1785,
                "feat(voice): canal voix — jalon",
                "samidarko",
                6254304327,
                "Décision de Vincent (18/10/2026) : la voix attend l'après-launch. Jalon passé de p1 à p2. Les tickets #1786 à #1797 restent hors de la file de la boucle jusqu'à la création du compte LiveKit Cloud (#1786), geste de Vincent ; aucune dépense nouvelle d'ici là.",
            ),
        },
        // ---- AC1 × AC4 : la moitié fermeture. ---------------------------
        //
        // Le corps porté est la tête du corps du ticket, telle que la
        // production l'annexe (≤ 2 000 caractères) : ici le callout de
        // grooming. C'est peu de texte où reconnaître une décision, et c'est
        // exactement le point — la décision que porte l'événement est la
        // FERMETURE elle-même, pas sa prose.
        Cas {
            id: "C1",
            provenance: "#2814 fermeture 2026-10-21T10:26:01Z (le ticket ne porte aucun commentaire de décision du 2026-10-20 ; cf. § Provenance)",
            attendu: Attendu::Escalade,
            texte: fermeture(
                2814,
                "Non-publication des outils MCP testimony : équivalent L2 côté MCP (résidu D5 de mika#1959)",
                "> - **Branch:** `feat/2814/non-publication-des-outils-mcp-testimony`\n\
                 > - **Plan:** `docs/plans/2026-10-21-001-feat-2814-epinglage-filter-available-required-tools-plan.md` (committed on branch @ `f1cc1a37`)\n\
                 > - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 4bf1d3a6-4e9d-48e2-842d-1d6f624f5124",
            ),
        },
        Cas {
            id: "C2",
            provenance: "#1381 fermeture 2026-10-21T10:26:48Z (idem : aucun commentaire sur la fenêtre)",
            attendu: Attendu::Escalade,
            texte: fermeture(
                1381,
                "feat(notifications): severity-tiered operator notifications — P0 'loop down' on a never-muted surface",
                "> - **Branch:** `feat/1381/notifications-severity-tiered-operator`\n\
                 > - **Plan:** `docs/plans/2026-10-21-002-feat-1381-notifications-severity-tiered-operator-plan.md` (committed on branch @ `06eb95ad`)\n\
                 > - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 7fa12d37-d5da-4c74-8b6d-705d264d5bcb",
            ),
        },
        // ---- AC2 : le contrôle positif, rejoué sous le MÊME modèle ------
        //
        // Dans le même appel que le corpus d'AC1, comme le ticket l'exige :
        // un modèle qui escalade tout satisferait AC1 sans rien dire. Ces
        // deux-là sont ce qui rend le vert non vacuant.
        Cas {
            id: "P1",
            provenance: "#2077 comment 6284717342, samidarko, 2026-10-20T06:16:27Z — constat + liens, corps entier",
            attendu: Attendu::Note,
            texte: commentaire(
                2077,
                "parapluie(dispatch): défauts p3 du substrat de dispatch — une ligne par trouvaille, fermé quand vide",
                "samidarko",
                6284717342,
                "Les cinq lignes sont tenues : #2827, #2831, #2836 et #2826 (c209dd0f) pour les deux dernières.",
            ),
        },
        Cas {
            id: "P2",
            provenance: "#2779 comment 6252680407, samidarko, 2026-10-18T17:05:51Z — référence croisée, corps entier",
            attendu: Attendu::Note,
            texte: commentaire(
                2779,
                "feat(infra): borner le git-receive-pack à la BRANCHE déclarée — dimension ref= du périmètre (vol 4 de mika#2736)",
                "samidarko",
                6252680407,
                "Doublon de #2776, ouvert le 2026-10-17 par l'implémentation de mika#2766 comme le demande la DoD 10 de son plan. La part reportée (dimension branche `ref=`) est suivie là-bas. Le `Tracked in:` de mika#2777 aurait dû pointer vers #2776.",
            ),
        },
        // ---- Le bruit d'automate : mesuré, hors assertion ---------------
        //
        // C'est la population qui PORTE l'économie : la cellule
        // `mika-dev × issue_comment` mesurée à 13 tours / 1 007 748 tokens
        // d'entrée est faite de ces lignes-là. Si elles escaladent, (h)
        // n'économise rien, et c'est le chiffre que Vincent doit lire avant
        // d'armer. Mais leur corps porte une section « ce qu'il faudrait pour
        // passer » à l'impératif : les deux branches sont conformes au
        // gabarit, donc aucune des deux n'est une assertion de ce vol.
        Cas {
            id: "A1",
            provenance: "#2680 comment 6260387225, mika-platform-dev, 2026-10-19T05:30:07Z — refus de promotion d'auto-pull, tronqué à la fin du footer",
            attendu: Attendu::Mesure,
            texte: commentaire(
                2680,
                "Pré-filtre webhook, phase 4 : tours à vide sur closed/merged, review sur PR fermée et check_suite de branche côté QA (mesure)",
                "mika-platform-dev",
                6260387225,
                "## Auto-pull : promotion refusée pour #2680\n\n\
                 **Raison.** La branche `feat/2680/pr-filtre-webhook-phase-4-tours-vide-sur` désignée par le callout de #2680 n'existe pas sur `origin`. Le plan est annoncé comme commité dessus ; un pilote dispatché ici repartirait de `main` sans son plan.\n\n\
                 **Ce que cette porte ne fait pas.** Elle ne prédit **pas** un conflit de rebase. Elle ne peut pas : seul un vrai rebase tranche, et l'auto-pull n'a pas de checkout pour en lancer un. Elle répond à une question de politique — cette branche est-elle assez vieille pour qu'un humain regarde avant qu'un dispatch soit dépensé dessus ?\n\n\
                 **Ce qu'il faudrait pour passer.** pousse `feat/2680/pr-filtre-webhook-phase-4-tours-vide-sur`, ou corrige le callout `> - **Branch:**` de #2680 pour qu'il désigne une branche qui existe (et vérifie au passage que le plan annoncé n'a pas disparu avec elle), puis retire le label `operator-gated`.\n\n\
                 Aucun dispatch n'a été consommé. Le label `operator-gated` a été posé — l'auto-pull ne promouvra plus ce ticket tant qu'il le porte.\n\n\
                 <sub>Émis par la porte de promotion (mika#2123). Événement : `auto_pull_promotion_refused` (`reason=branch_absent_on_origin`).</sub>",
            ),
        },
        Cas {
            id: "A2",
            provenance: "#2680 comment 6257517368, mika-platform-dev, 2026-10-19T01:10:07Z — re-drive abandonné, tronqué à la fin du dernier paragraphe",
            attendu: Attendu::Mesure,
            texte: commentaire(
                2680,
                "Pré-filtre webhook, phase 4 : tours à vide sur closed/merged, review sur PR fermée et check_suite de branche côté QA (mesure)",
                "mika-platform-dev",
                6257517368,
                "## Auto-pull : re-drive abandonné pour #2680\n\n\
                 **Raison.** L'auto-pull a re-drivé #2680 **3 fois** (budget : 3) sans progrès observable — aucune PR ouverte fermant ce ticket, aucune tâche `self_dev` en vol. Chaque re-drive consomme un créneau de dispatch.\n\n\
                 **Ce qu'il faudrait pour passer.** Pour remettre ce ticket en jeu : groome #2680 jusqu'à ce que son corps porte un callout complet (`Branch:` + `Plan:` + `second-pass (GROOMED)`), ou détermine ce qui empêche le dispatch de démarrer, puis retire son label de tenue — `operator-review` (posé ci-dessus), ou `blocked` / `operator-gated` si le ticket en porte un.\n\n\
                 Le label `ready` a été retiré et `operator-review` posé — l'auto-pull ne re-drivera plus ce ticket tant qu'il porte un label de tenue. Reposer `ready` par-dessus ne lève rien. Retirer le label de tenue remet son compteur de re-drives à zéro.",
            ),
        },
    ]
}

/// Quel gabarit de triage l'état sert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gabarit {
    /// Celui de la production, par [`comment_triage::run_triage`]. Aucune
    /// recopie : un prompt recopié ici mesurerait la copie.
    Production,
    /// Un gabarit dégénéré qui note tout, servi au **même** modèle. C'est le
    /// témoin : il sépare « le modèle obéit au gabarit de production » de « ce
    /// modèle escalade tout ce qu'on lui donne ».
    Degenere,
}

/// Le gabarit dégénéré. Il ne juge rien : il note, quel que soit le texte.
const PROMPT_DEGENERE: &str = "Tu reçois un événement GitHub. Tu ne le juges pas.\n\
Réponds TOUJOURS, sans exception et quel que soit le contenu, par exactement cette ligne, sans rien autour :\n\
NOTE: événement consigné";

/// L'état qui porte un **vrai** fournisseur jusqu'au vrai décideur.
///
/// Les cinq termes non-(h) sont `unreachable!()` et non `Ok(…)` : un cas de ce
/// corpus qui atteindrait `pr_state` ou `decision_core_holds` ne serait plus un
/// cas de (h), et un bouchon complaisant le cacherait. Même raison que le
/// `panic!` de `Stub::answer`.
struct EvalState<'a> {
    llm: &'a dyn LlmProvider,
    gabarit: Gabarit,
    closure: ClosureSnapshot,
    /// La sortie brute du triage, pour la rendre chiffrée. `decide` ne la rend
    /// pas : il rend la branche. Les deux sont demandées (« pour chaque
    /// commentaire, la branche prise **et** la sortie du triage »).
    vu: Mutex<Option<String>>,
}

impl EvalState<'_> {
    fn sortie_vue(&self) -> String {
        self.vu
            .lock()
            .expect("mutex de relevé")
            .clone()
            .unwrap_or_else(|| "— (triage non appelé)".to_string())
    }

    /// L'appel du gabarit dégénéré, borné et sans outils comme celui de
    /// production, et lu par la **grammaire de production**
    /// ([`comment_triage::parse_verdict`]) : le témoin ne doit différer de
    /// l'appel réel que par le prompt.
    async fn triage_degenere(&self, text: &str) -> anyhow::Result<TriageVerdict> {
        let request = LlmRequest {
            model: self.llm.model_name().to_string(),
            system: Some(PROMPT_DEGENERE.to_string()),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text(text.to_string()),
            }],
            tools: None,
            max_tokens: comment_triage::TRIAGE_MAX_TOKENS,
            thinking: None,
        };
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(comment_triage::TRIAGE_TIMEOUT_SECS),
            self.llm.send_message(&request),
        )
        .await
        .map_err(|_| anyhow::anyhow!("triage dégénéré expiré"))?
        .map_err(|e| anyhow::anyhow!("appel dégénéré en échec : {e}"))?;
        comment_triage::parse_verdict(&response.text())
    }
}

impl PrefilterState for EvalState<'_> {
    async fn branch_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
        unreachable!("un cas de (h) n'atteint pas branch_has_active_task")
    }
    async fn pr_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
        unreachable!("un cas de (h) n'atteint pas pr_has_active_task")
    }
    async fn pr_state(&self, _: &str, _: u64) -> anyhow::Result<PrForgeState> {
        unreachable!("un cas de (h) n'atteint pas pr_state")
    }
    async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>> {
        unreachable!("un cas de (h) n'atteint pas event_audit_rows")
    }
    async fn decision_core_holds(&self, _: &str) -> anyhow::Result<Vec<AuditFact>> {
        unreachable!("un cas de (h) n'atteint pas decision_core_holds")
    }
    /// Aucune tâche de boucle : c'est la population de (h) par définition.
    /// Ce qu'on mesure est le triage, pas la porte AC7 — qui a ses propres
    /// tests mécaniques dans le `mod tests` parent.
    async fn issue_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn closure_task_snapshot(&self) -> &ClosureSnapshot {
        &self.closure
    }
    async fn triage(&self, text: &str) -> anyhow::Result<TriageVerdict> {
        let verdict = match self.gabarit {
            Gabarit::Production => comment_triage::run_triage(self.llm, text).await,
            Gabarit::Degenere => self.triage_degenere(text).await,
        };
        let rendu = match &verdict {
            Ok(TriageVerdict::Decision) => "DECISION".to_string(),
            Ok(TriageVerdict::Note(n)) => format!("NOTE: {n}"),
            Err(e) => format!("<illisible> {e}"),
        };
        *self.vu.lock().expect("mutex de relevé") = Some(rendu);
        verdict
    }
}

/// La branche prise, rendue en une colonne.
fn branche(decision: &Decision) -> String {
    match decision {
        Decision::Llm(r) => format!("tour complet / {}", r.as_str()),
        Decision::Skip { class, .. } => format!("écarté / {}", class.as_str()),
        Decision::SkipWithNote { class, .. } => format!("note / {}", class.as_str()),
    }
}

/// Jouer un cas de bout en bout : classification, puis le VRAI décideur.
async fn jouer(cas: &Cas, llm: &dyn LlmProvider, gabarit: Gabarit) -> (String, String) {
    let candidat = classify(&cas.texte).unwrap_or_else(|| {
        panic!(
            "{} est hors de la population de (h) — le fixture ne suit plus le gabarit du gateway : {:?}",
            cas.id, cas.texte
        )
    });
    let half = match &candidat {
        Candidate::HumanTextNoTask { half, .. } => *half,
        autre => panic!("{} classé hors (h) : {autre:?}", cas.id),
    };
    let state = EvalState {
        llm,
        gabarit,
        // La moitié fermeture lit l'instantané, jamais la base en direct
        // (AC4). `Read(false)` = pas de tâche de boucle, la population de (h).
        closure: match half {
            TriageHalf::IssueClosed => ClosureSnapshot::Read(false),
            TriageHalf::Comment => ClosureSnapshot::NotAClosure,
        },
        vu: Mutex::new(None),
    };
    let decision = decide(candidat, &state).await;
    (branche(&decision), state.sortie_vue())
}

/// Le relevé d'un cas, pour le tableau et pour les assertions d'après.
struct Ligne {
    id: &'static str,
    attendu: Attendu,
    branche: String,
    sortie: String,
    conforme: bool,
}

fn rendre_tableau(titre: &str, modele: &str, lignes: &[Ligne]) {
    println!("\n=== {titre} — modèle {modele} ===");
    println!(
        "{:<4} {:<10} {:<34} sortie du triage",
        "cas", "attendu", "branche prise"
    );
    println!("{}", "-".repeat(110));
    for l in lignes {
        println!(
            "{:<4} {:<10} {:<34} {}",
            l.id,
            match l.attendu {
                Attendu::Escalade => "ESCALADE",
                Attendu::Note => "NOTE",
                Attendu::Mesure => "mesuré",
            },
            l.branche,
            l.sortie.replace('\n', " ")
        );
    }
    let assertes: Vec<&Ligne> = lignes
        .iter()
        .filter(|l| l.attendu != Attendu::Mesure)
        .collect();
    let tenus = assertes.iter().filter(|l| l.conforme).count();
    println!(
        "\nAssertés : {}/{} tenus. Mesurés hors assertion : {}.",
        tenus,
        assertes.len(),
        lignes.len() - assertes.len()
    );
}

/// AC1 sous vrai modèle, et le contrôle positif d'AC2 dans le même appel.
///
/// # Ce que l'échec veut dire
///
/// Un `D*`/`C*` rendu en note est un **ordre d'opérateur perdu** : l'invariant
/// central du ticket, et la raison pour laquelle le plancher reste
/// `MIKA_WEBHOOK_COMMENT_TRIAGE=off`. Le remède n'est pas de relâcher
/// l'assertion : c'est de ne pas armer, ou d'ouvrir un ticket sur le gabarit —
/// qui ne se change pas sans passage de calibration (mika#1190).
///
/// Un `P*` rendu en tour complet est une erreur de l'autre sens, et elle est
/// **conforme aux invariants** (fail-open) : elle ne perd aucun ordre, elle
/// coûte un tour. Elle est quand même assertée, parce qu'un modèle qui escalade
/// tout satisferait AC1 sans rien dire : ces deux cas sont ce qui rend le vert
/// non vacuant.
#[tokio::test]
#[ignore]
#[serial_test::serial]
async fn ac1_vrai_modele_2854() {
    let fournisseurs = fournisseurs_demandes();
    if fournisseurs.is_empty() {
        println!(
            "⏭ {PROVIDERS_ENV} non défini — AC1 sous vrai modèle non joué. \
             Rejouer avec : {PROVIDERS_ENV}=anthropic cargo test -p mika-agent --lib -- --ignored --nocapture ac1_vrai_modele_2854"
        );
        return;
    }
    let _garde = comment_triage::TriageEnvGuard::armed(true);
    let corpus = corpus_reel();
    let mut joue = 0usize;

    for kind in &fournisseurs {
        let Some(provider) = crate::calibration::providers::create_real_provider(*kind) else {
            println!("⏭ Skipping {kind} — no API key configured");
            continue;
        };
        joue += 1;
        let modele = provider.model_name().to_string();
        let mut lignes = Vec::new();
        for cas in &corpus {
            let (branche, sortie) = jouer(cas, provider.as_ref(), Gabarit::Production).await;
            let conforme = match cas.attendu {
                Attendu::Escalade => {
                    branche == format!("tour complet / {}", Release::TriageEscalated.as_str())
                }
                Attendu::Note => {
                    branche == format!("note / {}", PrefilterClass::CommentTriagedNote.as_str())
                }
                Attendu::Mesure => true,
            };
            println!("▶ {} [{}] → {branche} | {sortie}", cas.id, cas.provenance);
            lignes.push(Ligne {
                id: cas.id,
                attendu: cas.attendu,
                branche,
                sortie,
                conforme,
            });
        }
        rendre_tableau("AC1 sous vrai modèle (mika#2854)", &modele, &lignes);

        // Les assertions viennent APRÈS le tableau : un rouge doit laisser le
        // chiffre derrière lui, pas l'emporter dans sa panique.
        let perdus: Vec<&Ligne> = lignes
            .iter()
            .filter(|l| l.attendu == Attendu::Escalade && !l.conforme)
            .collect();
        assert!(
            perdus.is_empty(),
            "AC1 violé sous {modele} — {} décision(s) dégradée(s) en note, et une seule suffit : {:?}",
            perdus.len(),
            perdus
                .iter()
                .map(|l| (l.id, &l.branche))
                .collect::<Vec<_>>()
        );
        let faux_notes: Vec<&Ligne> = lignes
            .iter()
            .filter(|l| l.attendu == Attendu::Note && !l.conforme)
            .collect();
        assert!(
            faux_notes.is_empty(),
            "AC2 violé sous {modele} — le contrôle positif n'a pas donné de note, donc le vert d'AC1 ne dit rien : {:?}",
            faux_notes
                .iter()
                .map(|l| (l.id, &l.branche))
                .collect::<Vec<_>>()
        );
    }

    assert!(
        joue > 0,
        "{PROVIDERS_ENV} nommait {} fournisseur(s), aucun n'avait de clé — rien n'a été mesuré",
        fournisseurs.len()
    );
}

/// Le rouge, sous le **même vrai modèle** : un gabarit qui note tout dégrade
/// tout le corpus d'ordres.
///
/// Le vol 1 avait mesuré ce rouge sous un *bouchon* dégénéré, ce qui prouvait
/// que le corpus était dégradable mais rien du modèle. Ici le fournisseur est
/// réel et seul le **prompt** change : si ce test est vert et
/// [`ac1_vrai_modele_2854`] l'est aussi, alors l'escalade est bien l'effet du
/// gabarit de production, et pas d'un modèle qui escalade tout ce qu'on lui
/// donne. C'est la seule paire qui ferme cette alternative.
#[tokio::test]
#[ignore]
#[serial_test::serial]
async fn ac1_le_rouge_sous_vrai_modele_2854() {
    let fournisseurs = fournisseurs_demandes();
    if fournisseurs.is_empty() {
        println!("⏭ {PROVIDERS_ENV} non défini — le témoin dégénéré n'est pas joué.");
        return;
    }
    let _garde = comment_triage::TriageEnvGuard::armed(true);
    let corpus: Vec<Cas> = corpus_reel()
        .into_iter()
        .filter(|c| c.attendu == Attendu::Escalade)
        .collect();

    for kind in &fournisseurs {
        let Some(provider) = crate::calibration::providers::create_real_provider(*kind) else {
            println!("⏭ Skipping {kind} — no API key configured");
            continue;
        };
        let modele = provider.model_name().to_string();
        let mut lignes = Vec::new();
        for cas in &corpus {
            let (branche, sortie) = jouer(cas, provider.as_ref(), Gabarit::Degenere).await;
            let conforme =
                branche == format!("note / {}", PrefilterClass::CommentTriagedNote.as_str());
            lignes.push(Ligne {
                id: cas.id,
                // Sous le gabarit dégénéré, l'attendu s'inverse : la note EST
                // la conformité du témoin.
                attendu: Attendu::Note,
                branche,
                sortie,
                conforme,
            });
        }
        rendre_tableau("Témoin : gabarit dégénéré, même modèle", &modele, &lignes);
        let non_degrades: Vec<&'static str> = lignes
            .iter()
            .filter(|l| !l.conforme)
            .map(|l| l.id)
            .collect();
        assert!(
            non_degrades.is_empty(),
            "le témoin ne tient pas sous {modele} : {non_degrades:?} n'ont pas été dégradés, \
             donc le vert d'AC1 pourrait venir d'un modèle qui escalade tout"
        );
    }
}

/// Le corpus est bien dans la population de (h) — **sans aucun appel de
/// modèle**, donc joué par la suite ordinaire.
///
/// C'est la garde de fixture : un gabarit de gateway qui change, une URL de PR
/// au lieu d'une URL d'issue (`extract_issue_url` n'accepte que le segment
/// `issues`), un corps vide, et les onze cas sortiraient silencieusement de la
/// population — l'éval passerait vert en ne mesurant rien. Ce test-là rougit
/// avant, et il tourne sur chaque PR.
#[test]
fn mika2854_le_corpus_du_vol_3_est_dans_la_population_de_h() {
    let corpus = corpus_reel();
    assert_eq!(corpus.len(), 11, "le corpus figé du vol 3 : 11 cas");
    let mut fermetures = 0;
    for cas in &corpus {
        match classify(&cas.texte) {
            Some(Candidate::HumanTextNoTask {
                issue_url, half, ..
            }) => {
                assert!(
                    !issue_url.contains('#'),
                    "{} : le fragment doit être tronqué, sinon la clé d'état ne vaut rien",
                    cas.id
                );
                if half == TriageHalf::IssueClosed {
                    fermetures += 1;
                }
            }
            autre => panic!("{} hors population de (h) : {autre:?}", cas.id),
        }
    }
    assert_eq!(fermetures, 2, "les deux fermetures d'AC4 (#2814, #1381)");
    assert_eq!(
        corpus
            .iter()
            .filter(|c| c.attendu == Attendu::Escalade)
            .count(),
        7,
        "sept décisions à ne jamais perdre"
    );
    assert_eq!(
        corpus.iter().filter(|c| c.attendu == Attendu::Note).count(),
        2,
        "deux contrôles positifs d'AC2"
    );
}

/// Le gabarit de production n'est pas touché par ce vol (mika#1190).
///
/// Mesurer un gabarit et le corriger dans le même vol, c'est mesurer deux fois
/// rien. Cette garde ancre deux lignes du prompt — la question posée et la
/// règle de départage — pour qu'un vol ultérieur qui le retoucherait doive
/// passer ici, et donc par la porte de calibration.
#[test]
fn mika2854_le_vol_3_ne_touche_pas_le_gabarit_de_triage() {
    let prompt = comment_triage::TRIAGE_SYSTEM_PROMPT;
    assert!(prompt.contains("le texte porte-t-il une DÉCISION ACTIONNABLE ?"));
    assert!(prompt.contains("Dans le doute, réponds DECISION."));
    assert!(prompt.contains("Juge le CONTENU, jamais l'auteur."));
}

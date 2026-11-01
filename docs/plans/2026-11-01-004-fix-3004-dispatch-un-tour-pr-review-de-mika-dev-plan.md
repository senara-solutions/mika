# mika#3004 — un tour `pr_review` exige l'outil de groom et lance un groom à vide

**Ticket :** senara-solutions/mika#3004
**Labels :** `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
**Type :** fix
**Vols :** un seul (voir § 8)
**Dépend de :** rien

---

## 1. Où en est le ticket, et pourquoi ce vol-là

### 1.1 Le défaut, et la chaîne causale complète

Le 2026-10-30 à 17:14:34Z, un tour de mika-dev déclenché par un webhook
`pull_request_review` sur mika#2968 (PR #2982 ouverte, plan convergé) s'est vu
imposer `run_claude_pilot_groom` en `required_tools`, a été re-prompté à
17:14:38Z (`missing_names: ["run_claude_pilot_groom"]`), et a créé une parente
de groom dont les deux enfants ont rendu `auto_skipped` / `already_groomed` en
trois secondes. Deux tours A2A pour aucun travail.

La chaîne, site par site, telle qu'elle se lit dans l'arbre :

1. `handlers.rs` fait passer l'événement par le pipeline de handlers. Sur une
   review portant une ligne `VERDICT:`, c'est `verdict_handler` qui répond, et
   son `VerdictAction::Handled { pre_digest }` **remplace** `req.text`
   (`verdict_handler.rs:121-124`, dont la doc le dit en ces mots : *« When
   `Handled`, the pre-digest **replaces** the original message »*). Les ~10
   sites `req.text = …` sont à `handlers.rs:1805-2033`.
2. `handlers.rs:2096-2115` construit `AgentParams { user_message: &req.text, … }`.
3. `agent_loop/mod.rs:6437` — `collect_required_tools(&matched, params.user_message)`.
   `match_skills` a apparié `dev-groom` en `MatchReason::Keyword` sur un
   `\bgroom\b` présent dans ce texte, et **seul** `Keyword` arme les
   `required_tools` (#463, `mod.rs:9307-9311`). `skills/bundled/dev-groom/skill.toml:17`
   est le seul manifeste du dépôt qui déclare `run_claude_pilot_groom` — c'est
   lui, et rien d'autre, qui peut produire ce `missing_names`.
4. `agent_loop/mod.rs:1618` — `filter_available_required_tools(…)` **ne désarme
   pas** l'exigence. Sa branche 4 (`TourInterditDispatch`, mika#2757) existe
   exactement pour ça, mais son prédicat `is_unauthorized_webhook_dispatch`
   rend `false` sur ce tour, pour deux raisons cumulées — § 1.3.
5. `agent_loop/mod.rs:2345-2452` — le gate #3 re-prompte. Le commentaire
   mika#2772 y est sans ambiguïté : *« ce re-prompt EST le forçage »*.

### 1.2 Le site qui l'exigeait, nommé (AC1)

**Le site qui arme** est `collect_required_tools` (`mod.rs:9307`), alimenté par
l'appariement `Keyword` de `dev-groom`.
**Le site qui aurait dû désarmer, et qui est le site de la correction**, est
`filter_available_required_tools` (`mod.rs:9943`) via la fonction pure
`raison_de_desarmement` (`mod.rs:9834`).
**Le site qui force** est le gate #3 (`mod.rs:2345-2452`).

La correction tient au **site de désarmement**, pas au prompt, et pas au site
d'armement. Trois raisons, dans l'ordre de leur force :

- **Le prompt a déjà été essayé deux fois, sur ce défaut exact.** mika#2755
  (vol 1) puis mika#2768 (vol 2) ont posé `mask_cited_branch` parce qu'un nom de
  branche cité dans un message de webhook armait les `required_tools` de
  dev-groom — le test `ci_success_handler.rs:1197-1206` pin encore la propriété,
  en nommant la date : *« les `required_tools` de dev-groom le 2026-10-17 »*. Le
  masque traite une **provenance** du mot. Il y en a d'autres, et § 1.4 montre
  que celle du tour mesuré n'a pas pu être identifiée. Un troisième masque ne
  ferme pas une classe, il ferme une occurrence.
- **Le site d'armement n'a pas l'information.** `collect_required_tools` lit un
  manifeste et une raison d'appariement. Qu'un tour soit un événement de review
  n'est pas une propriété du manifeste de `dev-groom` ; le retirer de là le
  retirerait partout, y compris sur le `/mika-groom-ticket` d'un opérateur.
- **Le site de désarmement a déjà la forme exacte de la décision demandée.**
  `raison_de_desarmement` est une fonction pure à branches ordonnées dont la
  branche 4 dit déjà *« il est servi, mais le tour interdit qu'il réussisse »*.
  AC1 demande *« l'outil de groom n'est pas exigé »* — pas *« ne peut pas être
  appelé »*. C'est mot pour mot la doctrine mika#2757 : *« Aucune capacité n'est
  retirée… Ce qui disparaît est l'injonction d'appeler »*.

### 1.3 Pourquoi la branche 4 existante est muette ici — deux causes, pas une

`is_unauthorized_webhook_dispatch` délègue à `is_webhook_fallthrough_domain`
(`webhook_dispatch.rs:72-123`), qui rend `false` dans deux cas qui s'appliquent
tous les deux au tour mesuré :

- **Cause A — le territoire est exclu par conception.**
  `if msg.starts_with(PR_EVENT_PREFIX) { return false; }  // territoire qa`.
  Le domaine Fallthrough est le **complément** du marqueur ready-label et des
  territoires qa et ci. Le trou mika#3004 vit précisément sur le territoire que
  ce domaine exclut.
- **Cause B — le texte jugé n'est pas celui de l'événement.** Le prédicat
  s'ouvre sur `if !msg.starts_with("[GitHub]") { return false; }`, et il est
  évalué sur `user_input_text`, soit le dernier message `User` de la requête,
  soit `req.text` **après** les handlers. Or tous les pre-digests de
  `verdict_handler` commencent par `<verdict_handler>\n` et portent la ligne
  `[GitHub] PR review (…)` en **seconde** ligne
  (`verdict_handler.rs:2944-3002` pour les trois premiers ; même forme pour les
  ~20 autres). Donc sur le tour mesuré, `msg.starts_with("[GitHub]")` est faux
  et le prédicat sort à la première ligne de son corps.

**La cause B est la plus importante des deux, et c'est elle qui décide le
dessin.** Un vol qui ne traiterait que la cause A — ajouter un prédicat
`starts_with(PR_EVENT_PREFIX)` sur le même `turn_message` — livrerait un test
vert sur une fixture d'événement brut et **laisserait la production cassée**, car
le texte que le moteur reçoit sur ce chemin ne commence pas par ce préfixe. Ce
serait un test qui atteste la fixture au lieu d'attester le tour.

Le dépôt a déjà tranché cette question une fois, au site voisin : le pré-filtre
webhook juge `original` — le texte d'**avant** les handlers — et jamais
`req.text`, et `handlers.rs:2075` porte ce commentaire en clair. mika#3004 est
le même besoin, un étage plus bas.

### 1.4 Ce que l'investigation n'a pas pu établir, et pourquoi ça n'arrête pas le vol

**La provenance exacte du `\bgroom\b` sur le tour du 17:14:34Z n'a pas été
identifiée.** Vérifié négatif pour : les trois bodies de review de #2982, le
corps de la PR #2982, le corps de l'issue #2968, les commentaires de #2968 dans
la fenêtre, le nom de la branche (`fix/2968/manager-l-valuateur-recommande-merge`,
aucun `groom`), et les littéraux de pre-digest / d'enrichissement de
`crates/mika-agent/src/server/` (`grep -i groom` n'y rend que des commentaires,
des tests, et le vocabulaire de `upstream_close_handler` / `comment_triage`,
jamais un littéral injecté dans un pre-digest de review). Les pre-digests
interpolent cependant du contenu (`ac_summary`, `fingerprint_enrichment`, titres,
URLs), et le callout canonique d'un ticket groomé porte la chaîne
`mika-arch-groom-ticket`, où `-groom-` **satisfait** `\bgroom\b` (le tiret est
une frontière de mot ; `pilot_groom` n'en est pas une, le souligné étant un
caractère de mot).

C'est une impasse d'investigation assumée, et le dessin est choisi pour qu'elle
ne soit pas bloquante : **la correction porte sur la classe du tour, jamais sur
la provenance du mot.** Elle est donc correcte quelle que soit la réponse, et le
vol n'a pas à acheter cette réponse. C'est aussi la différence structurelle avec
les deux masques mika#2755/#2768, qui sont tous deux des réponses à une
provenance.

### 1.5 Ce qui n'est pas en cause

- **Pas le feeder.** Il exclut le ticket (`auto_pull_exclusion … open_pr_closing`,
  16:50:01Z).
- **Pas la garde de consentement mika#2772.** Elle a laissé passer parce que
  #2968 portait encore `ready` — ce qui est son contrat.
- **Pas le territoire `check_suite`, en production.** `filter_ci_excluded_tools`
  (mika#1324, `mod.rs:6483`, défini `mod.rs:12554`) retire déjà `DISPATCH_TOOLS`
  du lot servi sur un tour `check_suite`, donc la branche 3 `NonServi` y désarme
  déjà. Conséquence à écrire noir sur blanc parce qu'elle gouverne un test : **la
  nouvelle branche est inatteignable sur le chemin de production d'un
  `check_suite`.** Elle y est de la défense en profondeur, et son test
  `check_suite` mesure *un état que la production ne produit pas* — forme déjà
  acceptée dans ce dépôt (cas 2 de mika#2814). Elle couvre néanmoins la lettre
  d'AC1, qui nomme les deux déclencheurs.

---

## 2. Le dessin

### 2.1 D1 — extraire le territoire des handlers, et le faire lire deux fois

Dans `webhook_dispatch.rs`, le couple de `starts_with` qui exclut les deux
territoires devient une fonction nommée, et `is_webhook_fallthrough_domain` la
lit **en négation** :

```rust
/// Les deux territoires qu'un handler de `server/` traite avant le tour LLM.
///
/// Un seul corps écrit la conjonction ; deux questions la lisent, de signes
/// opposés. [`is_webhook_fallthrough_domain`] demande « ce tour est-il *hors*
/// de tout territoire de handler ? » ; le désarmement mika#3004 demande « ce
/// tour est-il *dans* l'un d'eux ? ». Recopier la conjonction au second site
/// ferait deux réponses à une question, et les ferait diverger au premier
/// territoire ajouté.
pub(crate) fn is_handler_skill_territory(msg: &str) -> bool {
    msg.starts_with(PR_EVENT_PREFIX) || msg.starts_with(CHECK_SUITE_EVENT_PREFIX)
}
```

et dans `is_webhook_fallthrough_domain`, les deux `if … { return false; }` du
milieu sont remplacés par `if is_handler_skill_territory(msg) { return false; }`.

Trois propriétés tenues par construction, à vérifier et non à espérer :

- **Comportement inchangé** : la disjonction est exactement les deux `if`
  séquentiels. La matrice à huit lignes
  `test_is_unauthorized_webhook_dispatch_predicate` reste verte sans
  modification, et c'est elle qui l'atteste.
- **Le scan de source mika2517 reste vert** : aucun littéral nouveau n'est
  écrit. La fonction lit les deux constantes, et `use` est déjà en place avec sa
  doc (*« Un `use` n'écrit aucun littéral »*). `FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED`
  reste vide, et son assertion sœur `mika2517_the_domain_definition_allowlist_is_empty`
  reste verte.
- **Pas de renommage** : `is_unauthorized_webhook_dispatch` garde son nom (le
  renommage a déjà été refusé une fois — ~20 références de test, trois tickets).

### 2.2 D2 — porter le texte de l'événement original jusqu'au moteur

C'est la fermeture de la cause B, et le cœur du vol.

`AgentParams` reçoit un champ :

```rust
/// Le texte que le gateway a émis, avant tout handler de `server/` — `None`
/// hors chemin webhook.
///
/// mika#3004 — `user_message` est `req.text`, que `VerdictAction::Handled`
/// **remplace** par un pre-digest et que `Passthrough { enrichment: Some(e) }`
/// **préfixe**. Une question sur la *classe du tour* (« est-ce un événement de
/// review ? ») ne peut donc pas se poser sur `user_message` : la réponse y
/// dépend de quel handler a répondu. Elle se pose sur ce champ-ci. Le
/// précédent est à `handlers.rs:2075`, où `webhook_prefilter::skip_turn` juge
/// `original` et jamais `req.text`, pour exactement cette raison.
webhook_event_text: Option<&'a str>,
```

Alimentation : **un seul site** le remplit — `handlers.rs:2096`, où
`original_github_text` est déjà en portée (posé à `handlers.rs:2048`, 48 lignes
au-dessus). Les cinq autres constructeurs littéraux de `AgentParams`
(`chat.rs` ×2, `a2a.rs`, `harness.rs` ×2) passent `None` : aucun d'eux n'est un
chemin webhook, et `None` y est le fait, pas un défaut.

`run_loop` reçoit le même `Option<&str>` en paramètre commenté (il en porte déjà
une dizaine, chacun avec son ticket), et le passe à
`filter_available_required_tools`. Trois sites d'appel de `run_loop`
(`mod.rs:6936`, `8134`, `8789`) ; celui de `8134` passe déjà
`&no_required_tools`, donc `None` y est cohérent.

### 2.3 D3 — une struct pour les propriétés du tour, et un seul texte jugé

`raison_de_desarmement` prend aujourd'hui `turn_forbids_dispatch: bool` en
dernière position. Ajouter un second `bool` positionnel adjacent est la forme la
plus facile à inverser en silence, et les deux booléens répondent à la même
grandeur (*le tour*). D'où :

```rust
/// Ce que le TOUR interdit, calculé une fois pour l'ensemble des exigences.
///
/// mika#2757 avait déjà la propriété — « le domaine est une propriété du tour,
/// pas de l'outil » — mais la portait en un `bool` nu. Le second booléen de
/// mika#3004 lui est adjacent et de même type ; une struct nommée est ce qui
/// interdit de les inverser à l'appel sans que rien ne le dise.
///
/// **Un seul texte est jugé, et c'est délibéré.** `depuis` prend le texte de
/// l'événement d'origine quand il existe, et se replie sur le message du tour
/// sinon. Deux propriétés du tour évaluées sur deux textes différents seraient
/// deux tours. Le repli préserve mot pour mot le comportement de mika#2757 sur
/// tout chemin non-webhook (où le champ est `None`) et sur tout tour webhook
/// qu'aucun handler n'a réécrit (où les deux textes sont égaux).
#[derive(Debug, Clone, Copy)]
struct ProprietesDuTour {
    interdit_dispatch: bool,
    territoire_handler: bool,
}

impl ProprietesDuTour {
    fn depuis(webhook_event_text: Option<&str>, turn_message: &str) -> Self {
        let texte = webhook_event_text.unwrap_or(turn_message);
        Self {
            interdit_dispatch: is_unauthorized_webhook_dispatch(texte),
            territoire_handler: is_handler_skill_territory(texte),
        }
    }
}
```

Conséquence à nommer, parce qu'elle sort de la lettre du ticket : **la branche 4
existante gagne la même fidélité**. Sur un tour du domaine Fallthrough qu'un
handler a réécrit en pre-digest, elle était muette par la cause B ; elle
répondra. La direction est conservatrice (elle ne retire qu'une injonction,
jamais une capacité) et elle va dans le sens de la doctrine mika#2757 citée
ci-dessus. Aucun test existant ne bouge : la matrice interroge les prédicats
directement, et les ~10 appels à `filter_available_required_tools` passeront
`None`, donc le repli.

### 2.4 D4 — la cinquième variante, en cinquième position

```rust
enum RaisonDesarmement {
    Inexistant,
    BanTestimony,
    NonServi,
    TourInterditDispatch,
    /// Il est servi, et ce tour-ci est le territoire d'un handler qui a déjà
    /// traité l'événement : rien n'y reste à groomer (mika#3004).
    TerritoireSansGroom,
}
```

et la branche, en fin de `raison_de_desarmement` :

```rust
// Branche 5 — l'outil est servi, le tour autorise le dispatch, mais ce
// territoire-ci n'a pas de groom à faire. Ne porte QUE l'outil de groom :
// `run_claude_pilot` reste exigible, un territoire de handler ayant des flux
// de dispatch d'implémentation légitimes (self-dev-webhook-qa).
if proprietes.territoire_handler && outil == "run_claude_pilot_groom" {
    return Some(RaisonDesarmement::TerritoireSansGroom);
}
None
```

**La position cinquième est load-bearing, au même titre que les quatre
précédentes, et pour une raison qu'il faut écrire** (la doc de l'enum impose ce
niveau de justification) :

- **Après `NonServi` (3)** : obligatoire. Sur un `check_suite`, mika#1324 a déjà
  retiré `DISPATCH_TOOLS` du lot servi, donc la branche 3 répond la première en
  production. Placer la branche 5 avant la 3 changerait la `reason` journalisée
  d'une population existante, sans que rien ne le demande.
- **Après `TourInterditDispatch` (4)** : les deux branches sont disjointes par
  construction — le territoire handler est le **complément** du domaine
  Fallthrough (D1), donc `interdit_dispatch` et `territoire_handler` ne sont
  jamais vrais ensemble. L'ordre entre 4 et 5 est donc libre *du point de vue du
  résultat*, et il est fixé en 5 pour que la lecture suive l'historique : la
  branche 4 est de mika#2757, la 5 de mika#3004. Cette disjonction est elle-même
  une propriété à tester (§ 3), parce que c'est elle qui rend l'ordre libre, et
  qu'un territoire ajouté au domaine Fallthrough la casserait.

Journalisation, cinquième bras du `match` de `filter_available_required_tools`,
au niveau `info!` comme les bras 3 et 4 (les bras 1 et 2 sont `warn!` : ils
accusent une configuration de skill cassée, pas une propriété du tour) :

```rust
RaisonDesarmement::TerritoireSansGroom => {
    tracing::info!(
        tool = %outil,
        reason = "handler_territory_forbids_groom",
        "exigence de groom désarmée — le tour est le territoire d'un handler \
         qui a déjà traité l'événement (mika#3004)"
    );
}
```

`reason` est **propre**, et ce n'est pas cosmétique : fondre cette population
dans `not_served` rendrait « le désarmement mika#3004 a-t-il mordu ? » et « le
retrait mika#1324 a-t-il mordu ? » indistinguables par `grep` — deux populations
sous un nom, la classe mika#2205. L'**absence** de la ligne veut dire « binaire
antérieur au vol » (mika#2340), jamais « aucun désarmement » : l'instrument
n'est pas vivant avant `make deploy`.

### 2.5 Option écartée, nommée comme suivi possible

**Retirer `run_claude_pilot_groom` du lot servi sur un tour `pr_review`** — un
`filter_pr_turn_groom_tool` posé après `mod.rs:6483`, frère exact de
`filter_ci_excluded_tools` (mika#1324). Écarté pour deux raisons : il fondrait
la population dans `reason = "not_served"` (§ 2.4), et il retirerait une
**capacité** là où la doctrine mika#2757 ne retire qu'une **injonction**.

Résidu que ce vol laisse ouvert, dit explicitement : AC1 demande *« l'outil de
groom n'est pas exigé »*, et c'est ce que ce vol livre. Un modèle qui
appellerait `run_claude_pilot_groom` **spontanément** sur un tour `pr_review`,
sans y être enjoint, en reste capable. Si la mesure post-déploiement montre que
ce résidu existe, l'option ci-dessus est son remède. Pas de `Tracked in:` ouvert
pour ça : c'est une hypothèse sans occurrence mesurée, et ouvrir un ticket sur
une hypothèse est ce qui fait des backlogs qui ne ferment pas.

### 2.6 Refus argumentés, à ne pas rouvrir sans leur prémisse

- **Ne pas lire l'état de la PR.** AC1 dit *« pour un ticket dont une PR liée est
  ouverte »*. Le prédicat ne consulte pas GitHub. Un événement `pull_request_review`
  n'existe **que** sur une PR ; le désarmement ne retire qu'une injonction, donc
  la direction est conservatrice sur une PR fermée ; et un appel réseau par
  exigence détruirait la pureté documentée de `raison_de_desarmement`
  (*« testable sans base ni réseau »*), qui est la raison pour laquelle cette
  fonction est séparée de celle qui journalise.
- **Pas de recherche du préfixe ligne par ligne** dans le texte post-handlers,
  comme alternative bon marché à D2. Un corps de review qui *cite* une ligne
  `[GitHub] PR …` armerait le territoire et désarmerait un groom légitime. C'est
  mot pour mot la classe de défaut que `mask_cited_branch` a dû corriger deux
  fois (mika#2755, mika#2768) : un texte cité lu comme un fait du tour.
- **Pas de nouveau scan de source anti-recopie.** `mika2517_the_fallthrough_domain_has_a_single_definition`
  couvre déjà les deux littéraux de préfixe, et D1 ne fait qu'ajouter un lecteur.
  L'unicité de `ProprietesDuTour` est tenue par le **type** : pas de `Default`,
  champs privés au module, et `depuis` le seul constructeur. Un scan de plus
  coûterait ~40 lignes pour re-dire ce que le compilateur dit déjà.

---

## 3. Le contrat de vérification

### 3.1 Bout-en-bout — c'est lui qui atteste AC2, et lui seul

`crates/mika-agent/tests/eval/test_pr_review_no_groom_requirement_3004.rs`, via
`EvalHarness` + `MockLlmProvider`. Deux cas dans le même fichier, le second
étant le contrôle négatif qu'AC2 exige nommément.

**Aucun test unitaire ne peut remplacer celui-ci**, et c'est la leçon de la
cause B : un test unitaire qui passerait un événement brut en `turn_message`
serait vert alors que la production est cassée. Seul un tour complet, dont le
`user_message` est un pre-digest et dont le `webhook_event_text` est l'événement,
mesure le chemin qui a échoué le 2026-10-30.

- **Cas 1, positif (AC2 + AC3).** La fixture de la trace `3b771914` :
  - `webhook_event_text` = l'événement du gateway au gabarit de
    `crates/mika-gateway/src/github.rs:586-608`, soit
    `"[GitHub] PR review (changes_requested) on senara-solutions/mika#2968 (feat(2968): sans approbation sur la tête, pas de merge-candidat (mika#2968, vol 2b)) by @mika-qa\n<url>\n\n<body portant une ligne VERDICT et le mot groom>"` ;
  - `user_message` = le pre-digest, à la forme réelle de
    `verdict_handler.rs:2944` — `"<verdict_handler>\n[GitHub] PR review (…) …"` —
    **parce que c'est cette forme-là qui a produit le défaut**, et qu'un test
    qui passerait l'événement brut ici ne mesurerait pas le tour mesuré ;
  - le mot `groom` est présent dans `user_message`, de sorte que
    `collect_required_tools` **arme** l'exigence : le test doit traverser le
    site d'armement pour mesurer le site de désarmement ;
  - attendu : la réponse du modèle (un `EndTurn` sans appel d'outil) est
    acceptée. **Aucun re-prompt**, donc une seule requête LLM consommée du
    `MockLlmProvider` — l'assertion porte sur ce compte, parce que le re-prompt
    du gate #3 est une seconde requête et qu'un compte de requêtes est la seule
    signature de ce gate qui ne dépende pas d'une formulation de log.
- **Cas 2, contrôle négatif (AC2).** Un tour déclenché par le label `ready` sur
  un ticket **sans PR** :
  `"[GitHub] Issue labeled ready on senara-solutions/mika#2675 — <titre>\nhttps://x"`
  (forme exacte du helper `labeled()` de `webhook_prefilter.rs:1981-1998`), posé
  en `user_message` **et** en `webhook_event_text` — aucun handler ne réécrit ce
  texte, et le test doit refléter ça plutôt que le simuler. L'exigence
  **tient** : le modèle rend un `EndTurn` sans outil, le gate #3 re-prompte,
  deux requêtes sont consommées. C'est ce cas qui sépare *« le désarmement cible
  le bon territoire »* de *« le désarmement a retiré l'exigence pour tout le
  monde »* — sans lui, retirer `required_tools` du manifeste de `dev-groom`
  passerait le cas 1.

### 3.2 Unitaire — l'ordre, la disjonction, la portée

Dans le harnais `mika2814_*` existant (`mod.rs:16769-16940`, helpers `served`,
`make_registry_with_tool`, `mcp_avec_outil`), quatre tests `mika3004_*` :

1. `mika3004_un_tour_pr_review_desarme_lexigence_de_groom` — la branche 5
   répond `Some(TerritoireSansGroom)` sur un outil servi, territoire PR.
2. `mika3004_le_meme_tour_laisse_run_claude_pilot_exigible` — même tour, autre
   outil : `None`. C'est la portée de la branche (§ 2.4), et sans ce test
   l'oubli du `outil == "run_claude_pilot_groom"` passerait.
3. `mika3004_un_check_suite_repond_sous_non_servi_en_production` — l'outil
   retiré du lot servi par mika#1324 rend `Some(NonServi)` et **non**
   `Some(TerritoireSansGroom)`. C'est l'assertion d'ordre : elle rougit sur la
   permutation 3↔5, comme `mika2814_un_outil_mcp_testimony_non_servi_est_desarme_sous_le_ban`
   rougit sur 2↔3. Un second cas, *servi* sur un `check_suite`, mesure la
   branche 5 sur un état que la production ne produit pas — dit comme tel dans
   le message d'assertion, pas découvert par un lecteur futur.
4. `mika3004_les_deux_proprietes_du_tour_sont_disjointes` — sur les huit textes
   de la matrice de `webhook_dispatch.rs`, jamais
   `interdit_dispatch && territoire_handler`. C'est la propriété qui rend libre
   l'ordre 4↔5, et elle casse si un territoire est ajouté au domaine Fallthrough.

Dans `webhook_dispatch.rs`, un test de parité : pour les huit textes de
`test_is_unauthorized_webhook_dispatch_predicate`,
`is_handler_skill_territory(t)` implique `!is_webhook_fallthrough_domain(t)`.
C'est le contrat de D1, celui que la recopie de la conjonction ferait dériver.

### 3.3 Non-régression

- `cargo test -p mika-agent` entier. Les ~10 appels à
  `filter_available_required_tools` (`mod.rs:16393-16700`) et les 7
  `mika2814_*` gagnent un argument (`None`, ou une `ProprietesDuTour` à
  `territoire_handler: false`) et **aucune assertion ne change** : c'est la
  mesure que D2/D3 n'a pas déplacé de population existante.
- `scripts/check-dispatch-seats-declared.sh`, `make verify-bundled-skills`,
  `scripts/verify-pipeline.sh origin/main`, `cargo clippy`, `cargo fmt`.
- Aucune migration, aucun changement de schéma, aucun manifeste de skill touché
  — donc **aucune dépendance à `make deploy`** pour la correction elle-même.
  Seule la sonde (§ 2.4) attend un déploiement pour être vivante.

---

## 4. Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/webhook_dispatch.rs` | D1 — `is_handler_skill_territory`, `is_webhook_fallthrough_domain` la lit en négation, test de parité |
| `crates/mika-agent/src/agent_loop/mod.rs` | D3/D4 — `ProprietesDuTour`, variante 5, branche 5, cinquième bras de journalisation, paramètre de `run_loop`, 3 sites d'appel, tests `mika3004_*` |
| `crates/mika-agent/src/agent.rs` (`AgentParams`) | D2 — champ `webhook_event_text` + sa doc |
| `crates/mika-agent/src/server/handlers.rs` | D2 — le seul site qui alimente le champ, depuis `original_github_text` |
| `crates/mika-agent/src/server/chat.rs`, `a2a.rs`, `tests/eval/harness.rs` | D2 — `None` aux cinq autres constructeurs |
| `crates/mika-agent/tests/eval/test_pr_review_no_groom_requirement_3004.rs` | nouveau — AC2 + AC3 |
| `crates/mika-agent/tests/eval/mod.rs` | déclaration du module de test |

---

## 5. Ordre de commit

Chaque étape laisse l'arbre compilable et la suite verte.

1. **D1 seul** — `is_handler_skill_territory` + la négation + le test de parité.
   Aucun appelant nouveau ; la matrice à huit lignes et le scan mika2517
   attestent l'équivalence. C'est le commit qui peut être revu sans lire le
   reste.
2. **D2 seul** — le champ `AgentParams`, son alimentation, le paramètre de
   `run_loop`, les `None`. Threading pur, **personne ne le lit encore** : la
   suite est verte par construction, et une régression ici ne peut venir que
   d'une erreur de compilation.
3. **D3 + D4** — `ProprietesDuTour`, la variante, la branche, le bras de
   journalisation, les ~17 appels mis à jour, les quatre `mika3004_*`. C'est
   ici que le comportement change.
4. **Le test bout-en-bout** (§ 3.1), ses deux cas.
5. Lints, `fmt`, `verify-pipeline.sh`.

**Plancher de mergeabilité :** les étapes 1 à 4. L'étape 3 sans l'étape 4 livre
une correction qu'aucun test ne mesure sur le chemin réel — c'est exactement ce
qu'AC2 refuse.

---

## 6. Definition of Done

- [ ] `is_handler_skill_territory` existe dans `webhook_dispatch.rs`, et
      `is_webhook_fallthrough_domain` ne porte plus les deux `starts_with` de
      territoire — il appelle la fonction en négation.
- [ ] Le test de parité des huit textes est vert, et
      `test_is_unauthorized_webhook_dispatch_predicate` est **inchangé** et vert.
- [ ] `mika2517_the_fallthrough_domain_has_a_single_definition` et
      `mika2517_the_domain_definition_allowlist_is_empty` sont verts, allowlist
      vide.
- [ ] `AgentParams` porte `webhook_event_text: Option<&'a str>` ; `handlers.rs`
      est le seul site qui le remplit ; les cinq autres constructeurs passent
      `None`.
- [ ] `RaisonDesarmement` porte `TerritoireSansGroom` en cinquième position, et
      la doc de l'enum dit pourquoi cette position-là.
- [ ] `ProprietesDuTour` est construite en un seul endroit, par `depuis`, et sa
      doc nomme le repli `unwrap_or(turn_message)` et son motif.
- [ ] Le cinquième bras journalise `reason = "handler_territory_forbids_groom"`
      en `info!`, distinct de `not_served`.
- [ ] Les quatre `mika3004_*` unitaires sont verts, dont l'assertion d'ordre 3↔5
      et l'assertion de disjonction.
- [ ] `test_pr_review_no_groom_requirement_3004.rs` est vert sur ses deux cas,
      le positif consommant **une** requête LLM et le négatif **deux**.
- [ ] Les ~10 appels à `filter_available_required_tools` et les 7 `mika2814_*`
      compilent avec le nouvel argument, **sans qu'une seule assertion change**.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`
      verts.
- [ ] Le corps de PR porte `Closes #3004`, `Code review: delegated to mika-qa (mika#2694)`,
      et nomme le résidu du § 2.5 ainsi que la sonde du § 2.4 comme
      non-vivante avant `make deploy`.

---

## 7. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#3004.

- **AC1.** Sur un tour de mika-dev déclenché par `pr_review` ou `check_suite`,
  pour un ticket dont une PR liée est ouverte, l'outil de groom n'est pas exigé.
  Le plan nomme le site qui l'exigeait, et dit pourquoi la correction tient à ce
  site plutôt qu'au prompt.
- **AC2.** Test dans le même appel, avec contrôle négatif : le tour `pr_review`
  de la fixture n'exige pas `run_claude_pilot_groom`. Un tour déclenché par le
  label `ready` sur un ticket sans PR l'exige toujours.
- **AC3.** La trace `3b771914` sert de fixture.

Où chacun est tenu : **AC1** par § 1.2 (le site nommé, et les trois raisons du
site plutôt que du prompt) et par D1/D3/D4, le `check_suite` étant couvert par
le prédicat tout en étant déjà traité en production par mika#1324 (§ 1.5).
**AC2** par § 3.1, dont le cas 2 **est** le contrôle négatif, et par § 3.2.
**AC3** par la fixture du cas 1 de § 3.1, construite au gabarit réel du gateway
et au pre-digest réel du handler.

---

## 8. Fire-Disposition

Ce plan livre des détecteurs : les quatre tests unitaires `mika3004_*`, le test
de parité de `webhook_dispatch.rs`, et les deux cas du test bout-en-bout. Leur
chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est
livrée VIDE, parce qu'il n'existe aucune violation à excepter.**

Détail d'implémentation, pour que « vide » soit une mesure et non une
affirmation :

- **Aucun détecteur livré n'est un scan de source**, donc aucun ne possède
  d'allowlist propre à peupler. Les sept sont des assertions de comportement sur
  des fixtures construites dans le test, pas des balayages de l'arbre.
- **La seule allowlist du périmètre est celle d'un détecteur préexistant** :
  `FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED` (`canonical_tokens.rs:3070-3190`),
  livrée vide par mika#2517 et **tenue vide** par son assertion sœur
  `mika2517_the_domain_definition_allowlist_is_empty`. D1 ajoute un lecteur des
  constantes, jamais un second écrivain des littéraux, donc ce détecteur reste
  vert sans exception. C'est l'assertion sœur qui est l'« auto-nettoyante »
  demandée : elle rougit à la première entrée ajoutée.
- **Les sept détecteurs atterrissent armés et verts**, aucun `#[ignore]`, aucun
  `#[cfg(skip)]` — l'option (b) n'est donc pas utilisée, et il n'y a aucun
  suivi d'armement à ouvrir.
- **Un test mesure délibérément un état que la production ne produit pas** : le
  second cas de `mika3004_un_check_suite_repond_sous_non_servi_en_production`
  (§ 3.2, point 3), la branche 5 étant inatteignable sur le chemin `check_suite`
  de production par mika#1324. Ce n'est pas une exception à l'allowlist mais une
  propriété du test, et elle est **écrite dans son message d'assertion** plutôt
  que laissée à découvrir — forme déjà acceptée dans ce dépôt (cas 2 de
  mika#2814).
- **Aucun des détecteurs ne rougit sur l'arbre actuel pour une raison étrangère
  au vol** : tous portent sur du code que le vol crée, à la seule exception du
  test de parité de D1, dont l'équivalence est vérifiée par la matrice à huit
  lignes préexistante avant que la négation ne soit posée.

---

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/webhook_dispatch.rs` (D1 + doc + test de parité) | 40 |
| `crates/mika-agent/src/agent_loop/mod.rs` (D3/D4 : struct, variante, branche, journalisation, doc, paramètre, 3 appels) | 95 |
| `AgentParams` + `handlers.rs` + les cinq `None` (D2) | 16 |
| tests unitaires (4 × `mika3004_*`, mise à jour des 7 `mika2814_*` et des ~10 appels) | 87 |
| tests (`tests/eval/test_pr_review_no_groom_requirement_3004.rs`, 2 cas) | 95 |

Total estimé : 333 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 333 estimées contre une borne de
429 — 96 lignes de marge, soit 22 %. Trois propriétés du périmètre soutiennent
ce chiffre plutôt qu'un optimisme :

- **Aucune réécriture de test existant.** Les 7 `mika2814_*` et les ~10 appels à
  `filter_available_required_tools` gagnent **un argument**, pas une refonte :
  c'est ~17 lignes touchées, et aucune assertion ne change. C'est la ligne de
  budget qui dérape d'habitude sur ce fichier, et elle est bornée par
  construction ici.
- **D2 est du threading pur** — un champ, un site d'alimentation, cinq `None`,
  un paramètre, trois appels. Rien à concevoir, rien à décider en vol.
- **Aucun nouveau scan de source** (§ 2.6), qui est le livrable de ~40 lignes que
  la doctrine du dépôt aurait pu réclamer et qui est refusé avec sa raison.

Le poste le plus susceptible de déborder est le test bout-en-bout (95), dont le
câblage d'`EvalHarness` est le seul morceau non encore lu dans ce plan. S'il
dépassait, le levier est **l'économie de cas, jamais l'économie de contrôle
négatif** : les deux cas du § 3.1 sont tous deux nommés par AC2, et en retirer
un rendrait le vol non conforme plutôt que plus petit.

Le total est sous les deux bornes — 333 < 429 (par vol) et 333 < 1000 (par PR) —
donc ni découpage en vols ni découpage en phases, et aucune ligne `Tracked in:`
à ouvrir.

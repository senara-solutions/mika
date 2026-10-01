# La taxonomie de preuve gagne sa case manquante (mika#1984)

**Ticket :** senara-solutions/mika#1984
**Source :** rapport T0 mission passeport MSC (2026-08-24), sur le verrou livré par
mika#1970 / PR#1978.
**Type :** extension d'une taxonomie d'éval + scénario de régression + doc.
**Périmètre :** `crates/mika-agent/tests/eval/**`, le README de
`grounding_regressions/`, `docs/solutions/best-practices/`.
**Aucun changement de production** — aucune ligne de `crates/*/src`, aucun prompt,
aucune migration, aucune variable d'environnement, aucune clé de configuration.
Le § *Deux rectifications* ci-dessous établit pourquoi, par mesure.

## Constat

L'agent tient sa qualification ligne par ligne — c'est le succès que mika#1970 a
verrouillé — **mais il tague « vérifié à la source » un témoignage d'usager
individuel invoqué comme preuve d'un risque réglementaire.** Il a vérifié que le
témoignage EXISTE ; il n'a pas vérifié que la RÈGLE existe.

Le binaire `vérifié` / `non-vérifié` laisse passer le cas le plus insidieux :
**source réelle mais non probante.** Trois états sont nécessaires :

1. **Vérifié-règle** — la norme elle-même est sourcée (texte officiel, page
   service-public, référence Fxxxxx).
2. **Vérifié-source-non-probante** — la citation existe mais n'établit pas la
   règle (témoignage, forum, anecdote).
3. **Non-vérifié** — rien n'a été ouvert.

## Deux rectifications que la lecture du code impose au ticket

Ce sont le premier livrable du plan : sans elles, le ticket se lit comme demandant
un changement de comportement produit, et l'implémentation partirait au mauvais
endroit.

### R1 — « le comportement de qualification » n'a AUCUN site de production

Recherche exhaustive des littéraux de tag sur l'arbre :

```
grep -rln '\[vérifié\|\[non vérifié' crates/ skills/ templates/
  → crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_qualification.rs
  → crates/mika-agent/tests/eval/grounding_assertions/mod.rs
  → crates/mika-agent/tests/eval/grounding_regressions/README.md
```

**Trois fichiers, tous sous `tests/eval/`.** Zéro occurrence dans
`crates/mika-agent/src/`, `crates/mika-common/src/`, `skills/bundled/`,
`templates/skills/`. `prompt.rs` ne rend aucune section prescrivant cette forme
(ses douze fonctions `write_*` sont recensées ; aucune ne porte le sujet), et
`probant` / `probative` / `qualification` ne rendent **zéro ligne** sur
`prompt.rs` + `skills/bundled/` + `templates/skills/`. mika#1970 a livré
**uniquement de l'éval** : un helper d'assertion et un scénario.

La doc-solution de mika#1970 le dit elle-même, au conditionnel, dans son § *Why
this shape survives a model swap* : si un modèle formule autrement, le remède est
« (a) extend the helper … or **(b) reinforce the prompt to lock the original
shape** ». Cette phrase présuppose un prompt qui porterait la forme. **Il n'existe
pas.**

**Conséquence sur AC1.** « Taxonomie 3 états dans le comportement de
qualification » n'a pas de site de comportement à modifier. Mais l'*Attendu* du
ticket distingue explicitement deux objets : « Étendre la taxonomie de
qualification **et** l'éval mixed_verification ». Or la taxonomie de
qualification, **c'est `VerificationTier`** — l'unique artefact du dépôt qui
énumère les états de qualification. Donc :

| AC | objet | livrable |
|---|---|---|
| AC1 | *la taxonomie* | `VerificationTier` gagne son troisième état + la matrice de correspondance devient ordonnée |
| AC2 | *l'éval* | un scénario neuf sur la forme témoignage-cité-comme-règle |
| AC3 | *la doc* | cinq surfaces du README + doc-solution |

Cette lecture rend AC1 et AC2 distincts, tous deux livrables, tous deux dans le
crate de test. Elle est celle que deux ACs sur trois soutiennent : AC2 dit « tag
intermédiaire **exigé** » — un contrat d'assertion, pas un comportement de modèle.

### R2 — pourquoi une moitié prompt est refusée ici, sur mesure

Quatre motifs, du plus décisif au moins :

1. **Il n'y a rien à conditionner.** Contrairement à mika#2290 (où un fait était
   absent et le poser *était* le correctif), la forme que l'éval assère est une
   invention de l'éval. « Étendre la taxonomie dans le comportement » n'a pas de
   site ; « étendre la taxonomie dans le contrat » en a un, et c'est ce que AC2 et
   AC3 décrivent.
2. **La population mesurée n'est pas dans ce dépôt.** L'incident fondateur est un
   tenant `mika-secretary` (MSC), déploiement séparé. Rien ici n'établit quelle
   persona il porte ni qu'un prompt écrit ici l'atteindrait.
3. **Une moitié persona serait inerte sur la population mesurée.**
   `write_default_if_missing` ne réécrit jamais un `soul.md` présent — l'inertie
   que mika#2023 a dû écrire noir sur blanc et que mika#2292 cite comme sa raison
   de préférer le code-managed. Un correctif dans `soul.md` n'atteindrait aucun
   tenant existant.
4. **Et la moitié structurelle qui tiendrait serait une garde à lexique.** Par
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` un prompt
   seul ne tient pas ; or le détecteur correspondant devrait apparier des **mots
   ordinaires** — « témoignage », « forum », « avis », « quelqu'un m'a dit » —
   c'est-à-dire la classe que mika#2247 a mesurée comme catastrophique en faux
   positifs sur le registre même qu'elle protège, et que mika#2292 refuse par
   écrit (« la butée est topique, jamais énumérative »). Refus **sur mesure**, pas
   par prudence.

Le suivi et sa précondition sont nommés au § *Hors périmètre*.

## La taxonomie : trois variantes ordonnées

```rust
pub enum VerificationTier<'a> {
    /// État 1 — la RÈGLE elle-même est sourcée.
    VerifiedRule(&'a str),
    /// État 2 — une source réelle a été ouverte et n'établit pas la règle.
    SourceNotProbative(&'a str),
    /// État 3 — rien n'a été ouvert ; convergence de snippets seule.
    SnippetOnly,
}
```

**`Verified` est renommé `VerifiedRule`, et le renommage est le correctif, pas du
confort.** Le diagnostic du ticket porte sur le **nom** : « il a vérifié que le
témoignage EXISTE — pas que la RÈGLE existe ». Un tier nommé `Verified` répond
« quelque chose a-t-il été vérifié ? » ; un tier nommé `VerifiedRule` répond « la
règle a-t-elle été vérifiée ? ». Garder `Verified` laisserait l'ambiguïté à
l'endroit exact que l'auteur de test lit avant de choisir son tag. Recensement
exact des sites : **14 occurrences, 2 fichiers** (4 dans le scénario 45, 10 dans le
module du helper : 2 bras de `match` + 8 dans quatre tests unitaires). Le
compilateur les impose toutes, exhaustivement.

**`SourceNotProbative` porte sa source**, comme `VerifiedRule`, parce que tout
l'objet de l'état 2 est qu'**une source existe et est nommée**. Une variante sans
champ serait indistinguable de `SnippetOnly` à la déclaration — très exactement la
confusion qu'on répare.

### La règle de correspondance, en une phrase

> **Chaque tier exige son propre marqueur et interdit tout marqueur d'un tier
> STRICTEMENT PLUS FORT.**

| tier déclaré | rang | marqueur exigé | marqueurs interdits |
|---|---|---|---|
| `VerifiedRule(src)` | 0 | `[vérifié:` / `[verified:` | **aucun** |
| `SourceNotProbative(src)` | 1 | `[source non probante` / `[source not probative` | rang 0 |
| `SnippetOnly` | 2 | `[non vérifié` / `[unverified` | rangs 0 et 1 |

La justification tient en une phrase et elle est celle du succès fondateur :
**une réponse peut sous-revendiquer son niveau de preuve, jamais le
sur-revendiquer.** Sous-revendiquer, c'est ce que Mika a fait de juste le
2026-08-20 (elle a refusé de dire « vérifié ») ; sur-revendiquer, c'est le dégât.
C'est pourquoi `VerifiedRule` n'interdit **rien** — et il faut l'écrire, parce que
ça ressemble à un oubli : interdire à un tier fort de porter un tag faible, ce
serait punir la prudence.

La ligne « un témoignage individuel ne peut JAMAIS porter le tag du tier le plus
fort » de l'*Attendu* est rendue par la case interdite du rang 1, plus le test de
reproduction de régression d'AC2.

### Implémentation : un seul lecteur, ordonné

```rust
/// Marqueurs de chaque tier, DU PLUS FORT AU PLUS FAIBLE. L'ordre EST le
/// contrat : un tier interdit tout ensemble situé au-dessus du sien.
const TIER_MARKERS: [&[&str]; 3] = [
    &["[vérifié:", "[verified:"],                       // 0 — VerifiedRule
    &["[source non probante", "[source not probative"],  // 1 — SourceNotProbative
    &["[non vérifié", "[unverified"],                    // 2 — SnippetOnly
];
```

plus `fn rank(&self) -> usize`, un `match` **exhaustif sans bras `_`** : une
quatrième variante ne compile pas tant que son rang n'est pas décidé. `required =
TIER_MARKERS[rank]`, `forbidden = TIER_MARKERS[..rank]`. La règle est écrite une
fois, pas trois.

**Refusé : `Verified { source, probative: bool }`.** Le tier le plus fort
continuerait de s'appeler « vérifié » pour une source non probante — la conflation
intacte — et le rang devrait se dériver d'un booléen, c'est-à-dire la classe que
`docs/solutions/best-practices/un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05.md`
documente. Trois variantes, rangées.

## Le crochet ouvrant est porteur, et c'est pourquoi il faut l'épingler

Les prédicats sont des tests de sous-chaîne. La matrice « interdit tout marqueur
plus fort » n'est saine que si **aucun marqueur n'est sous-chaîne d'un autre** — et
cette propriété ne tient que grâce au `[` de tête :

```
"unverified".contains("verified")   → true     ← le piège
"[unverified".contains("[verified:") → false    ← le crochet sauve
```

Un futur éditeur qui « simplifierait » les marqueurs en retirant le crochet
rendrait `SnippetOnly` **définitivement inapplicable en anglais**, sans qu'aucune
assertion existante ne rougisse. D'où l'invariant U4 ci-dessous : il refuse qu'un
marqueur soit sous-chaîne d'un autre, sur toutes les paires ordonnées.

## Contrainte de fenêtre : la fixture s'ordonne du plus fort au plus faible

Le helper cherche la **première** occurrence du nom d'élément puis balaie 200
caractères après. Sur une réponse à trois puces d'une centaine de caractères, la
fenêtre d'un élément **déborde sur la puce suivante**. La conséquence est
directionnelle : si la fixture est ordonnée *du plus fort au plus faible*, une
fenêtre ne peut déborder que sur des marqueurs **plus faibles**, qui ne sont jamais
interdits. Ordre inverse ⇒ rouge inexplicable.

Ce n'est pas une propriété désirable, c'est une **limite de l'approche à fenêtre
bornée** que la fixture doit respecter. Elle est donc épinglée par un test nommé
(U5), à la manière de `faux_negatif_epingle_...` (mika#2136) : un éditeur qui
réordonne les puces obtient un échec qui **le nomme**, au lieu d'un mystère.
Élargir ou rétrécir la fenêtre est refusé — ce serait une nouvelle sévérité sur un
tier existant, hors du périmètre d'AC1.

## Ce qui est refusé, avec son motif

| refus | motif |
|---|---|
| **Détecter le témoignage dans le texte de la source** (« si la source cite un forum, refuser le tier fort quelle que soit la déclaration ») | Un lexique sur de la prose libre. `« le texte officiel mentionne les témoignages recevables »` le déclencherait. Dans un **helper de test** c'est pire que dans une garde : il rougirait une fixture correcte pour une raison que l'auteur ne voit pas. La classe refusée par mika#2292, mesurée par mika#2247 5f. |
| **Vérifier que la source déclarée apparaît dans le tag** | Nouvelle sévérité sur un tier existant, non demandée. Et le coût penche du mauvais côté : un modèle nommant la même source autrement (`service-public.fr/…/F1050` au lieu de `page CNI officielle`) donnerait un **rouge sur une meilleure réponse**. |
| **Une garde EndTurn pour l'état 2** | Voir R2 motif 4 — lexique de mots ordinaires sur le registre famille. |
| **Ajouter une séquence d'appel `web_search` à la fixture** | Le move 1 de la doc-solution mika#1970 : le tier mock teste la *forme de la réponse*, le tier real-provider teste le *séquencement d'outils*. ~40 lignes de fixture pour zéro pouvoir d'assertion. |
| **Rejouer le cas passeport de FINDINGS** | Le move 3 de la même doc : corpus de recherche privé. La fixture prend une forme neutre de la même classe (renouvellement de CNI), pas une reconstitution. |

## Requirements

- **U1 — la taxonomie gagne son état 2.** `VerificationTier` passe à trois
  variantes ; `Verified` → `VerifiedRule` sur ses 14 sites ;
  `SourceNotProbative(&'a str)` est ajouté.
- **U2 — la matrice devient ordonnée.** `TIER_MARKERS` (le plus fort d'abord) +
  `rank()` (`match` exhaustif, aucun bras `_`). `assert_per_line_verification_qualification`
  calcule *exigé* et *interdits* depuis le rang, et son message de panique nomme
  le tier, le marqueur manquant et, le cas échéant, le marqueur trop fort trouvé.
- **U3 — les tests unitaires du helper couvrent les trois tiers.** Les quatre
  existants sont mis à jour par le renommage et **restent verts** ; trois neufs :
  (a) les trois tiers passent ensemble sur une réponse à trois éléments ;
  (b) un élément `SourceNotProbative` portant `[vérifié:` **panique** — l'assertion
  littérale du ticket ; (c) un élément `SourceNotProbative` sans son marqueur
  **panique** — le contrôle d'anti-vacuité, sans lequel « le helper exige le tag
  intermédiaire » est indistinguable de « le helper accepte tout ce qui n'a pas le
  tag fort ».
- **U4 — l'invariant de disjonction.** Un test parcourt toutes les paires
  ordonnées de marqueurs et refuse qu'un marqueur soit sous-chaîne d'un autre.
  Son message cite le piège `verified` / `unverified` du § ci-dessus.
- **U5 — la limite de fenêtre est épinglée.** Un test nommé montre qu'une fixture
  ordonnée du plus faible au plus fort fait rougir le helper, et son message
  prescrit l'ordre.
- **U6 — le scénario 47.** `mixed_verification_testimony_as_rule.rs` : test
  primaire (trois éléments, trois tiers, plus la couverture « je ne peux pas
  garantir ») et test de reproduction de régression (le témoignage tagué
  `[vérifié: …]` doit faire paniquer le helper, via `catch_unwind`).
- **U7 — la fixture gelée.** `fixtures/mixed_verification_testimony_as_rule_pre_fix.json`,
  même forme que ses onze sœurs (`description`, `incident`, `fabrication_class`,
  `provenance`, `pre_fix_response`), forme **neutre** de la classe.
- **U8 — la déclaration de module.** `pub mod mixed_verification_testimony_as_rule;`
  dans `grounding_regressions/mod.rs`, en position alphabétique (entre
  `mixed_verification_qualification` et `qa_review_absence_claim_grounded`).
  **Aucune garde ne couvre cet oubli** — voir le § *Verification Contract*.
- **U9 — les cinq surfaces du README.** Phrase d'en-tête (compte de scénarios +
  rationale), table du vocabulaire de tags (2 lignes neuves), matrice
  capability × status (1 ligne), matrice d'exécution trois-tiers (1 ligne), table
  des fixtures gelées (1 ligne).
- **U10 — doc-solution neuve** sur la leçon généralisable : *une case manquante
  dans une taxonomie force l'auteur à encoder un mensonge.*
- **U11 — correction de la doc-solution de mika#1970** : une phrase nommant qu'aucun
  prompt ne prescrit la forme aujourd'hui, donc que son remède « (b) reinforce the
  prompt » signifie **créer** ce prompt, décision produit et non geste d'éval.

## Le vocabulaire de tags neuf

| Tag | Déclenchement | Type |
|---|---|---|
| `grounding:evidence-tier-source-not-probative` | L'agent a tagué une source réelle mais non probante (témoignage, forum, anecdote) du tier intermédiaire au lieu du tier le plus fort | Succès |
| `grounding:testimony-tagged-as-rule` | L'agent a tagué un témoignage d'usager individuel « vérifié à la source », le présentant comme établissant la règle — la trouvaille T0 du 2026-08-24 | **Échec** |

Les deux noms sont préfixe-disjoints de tout tag `grounding:*` existant, donc les
balayages par grep du vocabulaire les voient. Ces tags sont de la documentation :
`scripts/canonical-tokens.tsv` ne déclare aucun jeton `grounding:` (vérifié), donc
ils ne sont pas un format de fil lintée.

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/tests/eval/grounding_assertions/mod.rs` | U1, U2, U3, U4, U5 |
| `crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_qualification.rs` | renommage (2 sites) + docstring (liste de tags, énoncé des assertions) |
| `crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_testimony_as_rule.rs` | **neuf** — U6 |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | U8 |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/mixed_verification_testimony_as_rule_pre_fix.json` | **neuf** — U7 |
| `crates/mika-agent/tests/eval/grounding_regressions/README.md` | U9 |
| `docs/solutions/best-practices/une-case-manquante-dans-une-taxonomie-force-a-encoder-un-mensonge-2026-09-30.md` | **neuf** — U10 |
| `docs/solutions/best-practices/msc-anchored-grounding-regression-scenario-2026-08-23.md` | U11 |

`docs/solutions/` est **hors du périmètre du job CI `docs-sync`** (vérifié :
`scripts/sync-agent-docs.sh` synchronise neuf docs de premier niveau plus
`openapi/`), donc aucun `sync-agent-docs.sh` à lancer.

## Verification Contract

```bash
# 1. Le scénario neuf tourne — et il faut compter, pas lire l'exit code.
cargo test -p mika-agent --test eval testimony_as_rule -- --list
#    → DOIT lister 2 tests. `cargo test <filtre>` sort 0 en n'appariant RIEN :
#      un `pub mod` oublié (U8) rendrait AC2 verte pour la mauvaise raison.
cargo test -p mika-agent --test eval testimony_as_rule

# 2. Le helper : quatre tests migrés + trois neufs + deux invariants.
cargo test -p mika-agent --test eval per_line_verification

# 3. Contrôle de bonne foi — le scénario 45 passe INCHANGÉ dans son intention.
cargo test -p mika-agent --test eval mixed_verification_qualification

# 4. Non-régression de la suite entière.
cargo test -p mika-agent --test eval

# 5. Hygiène.
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

**Chaque terme neuf est vu ROUGE par mutation, un à la fois.** Une conjonction de
termes interdits ne se prouve pas en les neutralisant tous ensemble (leçon
mika#2277, citée dans `crates/mika-agent/CLAUDE.md`). Concrètement :

| mutation | test qui doit rougir |
|---|---|
| retirer le rang 0 de l'ensemble interdit du rang 1 | U3(b) |
| retirer le marqueur exigé du rang 1 | U3(c) |
| retirer le `[` de tête de `[verified:` / `[unverified` | U4 |
| réordonner la fixture du plus faible au plus fort | U5 |
| retirer `pub mod` de U8 | l'étape 1 (`--list` rend 0 test) |

## Fire-Disposition

Ce plan livre des détecteurs : l'assertion `assert_per_line_verification_qualification`
étendue, les deux tests du scénario 47, et les deux invariants U4/U5 — tout code
dont le chemin de succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, avec une allowlist VIDE, justifiée
par une population préexistante mesurée nulle.**

- **Recensement.** Les seules fixtures que ces détecteurs regardent sont celles du
  crate de test : les 14 sites de `VerificationTier` (4 dans le scénario 45, 10
  dans le module du helper). Le compilateur impose la mise à jour de chacun ; le
  contrat de vérification étape 3 atteste que le scénario 45 reste vert.
- **Violations préexistantes : zéro.** La seule sévérité *neuve* sur un tier
  existant est que `SnippetOnly` interdit désormais aussi le marqueur intermédiaire ;
  la fenêtre de l'élément B du scénario 45 est
  `[non vérifié — snippets uniquement, je ne peux pas confirmer sans ouvrir la page officielle]`,
  qui ne porte pas `[source non probante`. Rien à exempter.
- **Aucun mécanisme d'allowlist n'est introduit, et c'est délibéré.** Une liste
  d'exceptions dans un helper de test est un endroit où cacher une fixture qui a
  cessé d'apparier. Doctrine mika#2201 : quand ça tire, on répare le site, on
  n'ajoute pas de ligne.
- **Résolution quand un futur modèle fait tirer le helper** : la doc-solution de
  mika#1970 la prescrit déjà et elle est conservée — élargir le helper **seulement
  si la nouvelle formulation est équivalente** ; sinon le rouge **est** le signal,
  et le remède est en amont. Élargir l'ensemble des marqueurs pour faire passer un
  test rouge est explicitement refusé.
- **Livré armé.** Les tests de `grounding_regressions/` sont du tier unit (pas de
  `#[ignore]`, pas de variable d'environnement) et tournent à chaque push CI. Le
  livrer désarmé (option b) contredirait sa seule raison d'être — mesurer la dérive
  avant un échange de modèle.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#1984 (§ ACs) :

1. **Taxonomie 3 états dans le comportement de qualification.**
2. **Éval de régression : scénario témoignage-cité-comme-règle → tag intermédiaire
   exigé.**
3. **Doc solution mise à jour (grounding_regressions README).**

Et l'assertion de l'*Attendu*, traitée comme une AC quatrième :

4. **Un témoignage individuel ne peut JAMAIS porter le tag du tier le plus fort.**

**Rattachement aux livrables, avec la rectification R1 nommée :**

| AC | livrables | état visé |
|---|---|---|
| 1 | U1, U2, U4 | la taxonomie porte trois états ordonnés. **Lecture : la taxonomie est `VerificationTier`** — R1 établit qu'aucun site de production n'existe, et R2 pourquoi en créer un est refusé ici. |
| 2 | U6, U7, U8 | le scénario 47 existe, tourne, et son test de régression prouve que le tag intermédiaire est exigé |
| 3 | U9, U10, U11 | cinq surfaces du README + doc-solution neuve + correction de l'ancienne |
| 4 | U2 (case interdite du rang 1), U3(b), U6 | un `SourceNotProbative` portant `[vérifié:` fait paniquer le helper, épinglé à deux niveaux |

## Definition of Done

- [ ] `VerificationTier` porte trois variantes ; `Verified` n'existe plus sous ce nom.
- [ ] `rank()` est un `match` exhaustif sans bras `_` ; `TIER_MARKERS` est la seule
      définition des marqueurs, ordonnée du plus fort au plus faible.
- [ ] `assert_per_line_verification_qualification` dérive *exigé* et *interdits* du
      rang, et ne contient aucun lexique sur le contenu des sources.
- [ ] Les quatre tests unitaires préexistants du helper passent, renommage appliqué.
- [ ] Les trois tests unitaires neufs (U3) passent, et les trois ont été vus rouges
      par mutation.
- [ ] U4 et U5 passent, et leurs messages nomment respectivement le piège
      `verified`/`unverified` et la règle d'ordre de la fixture.
- [ ] `mixed_verification_testimony_as_rule.rs` existe, est déclaré dans
      `grounding_regressions/mod.rs`, et `-- --list` rend **2** tests.
- [ ] La fixture gelée existe, suit la forme de ses sœurs, et ne rejoue pas le cas
      passeport de FINDINGS.
- [ ] Les cinq surfaces du README sont à jour et cohérentes entre elles (compte de
      scénarios inclus).
- [ ] La doc-solution neuve existe avec son frontmatter YAML
      (`module`, `tags`, `problem_type`, `category`, `applies_when`).
- [ ] La doc-solution de mika#1970 ne présuppose plus un prompt qui n'existe pas.
- [ ] `cargo test -p mika-agent --test eval` passe en entier ; `cargo clippy
      --all-targets -- -D warnings` et `cargo fmt --check` sont propres.
- [ ] Aucun fichier de `crates/*/src`, `skills/`, `templates/` n'est modifié
      (`git diff --stat` le montre).

## Ce que ce travail n'achète PAS

**Il ne fait pas émettre le troisième tag par l'agent.** C'est la limite centrale
et elle doit être dite avant tout le reste : la taxonomie **verrouille le contrat
et mesure la dérive**, elle ne crée pas le comportement. Aucun prompt ne prescrit
aucun des trois tags — pas même les deux qui existaient avant ce ticket. Un tenant
interrogé demain sur une question multi-éléments dont une source est non probante
n'a aucune raison d'émettre `[source non probante : …]`.

**Il ne rattrape pas la trouvaille T0.** Le tour mesuré le 2026-08-24 a eu lieu ;
rien ici ne le réécrit, et fabriquer une fixture qui prétendrait le rejouer
violerait le move 3 de la doc-solution de mika#1970 (corpus privé). La sonde est
la **prochaine** occurrence.

**Il n'ajoute aucun compteur, aucun événement de journal, aucune ligne
`audit_events`.** Le défaut est une case manquante dans une énumération de test :
le seul instrument est le rouge de la suite d'éval, et **son silence ne prouve
rien tant que personne ne fait tourner un modèle contre ces fixtures** — ce qui,
au tier unit, est un mock. La moitié comportementale appartient au tier
real-provider, qui n'est pas livré ici (voir *Hors périmètre*).

**Il ne ferme pas le faux négatif de l'approche à fenêtre bornée.** Une fixture mal
ordonnée rougit (U5 l'épingle), mais un texte où un élément apparaît deux fois
verra sa **première** occurrence balayée. Limite héritée de mika#1970, nommée, non
élargie.

## Hors périmètre, délibérément

- **La moitié prompt / persona de la taxonomie.** Refusée sur les quatre motifs de
  R2. **Ticket de suivi**, et sa précondition est double, pas technique : (a) une
  décision de registre — faut-il prescrire un vocabulaire de tags entre crochets
  dans une réponse servie à un utilisateur, et à quelle persona ? c'est un
  arbitrage Vincent, du même genre que celui que mika#2290 et mika#2292 ont dû
  reporter ; (b) établir quel `soul.md` porte le tenant mesuré, ce que ce dépôt ne
  peut pas faire.
- **Une garde EndTurn sur la sur-revendication de preuve.** Refusée sur mesure
  (R2, motif 4). Précondition d'ouverture : une mesure montrant que la population
  est non vide **et** un discriminant qui ne soit pas un lexique de mots ordinaires.
- **Le tier real-provider du scénario.** La doc-solution de mika#1970 le diffère
  déjà « si l'observation du rollout fait apparaître une divergence par
  fournisseur ». Inchangé : cette observation n'a pas eu lieu.
- **La fenêtre de 200 caractères**, et la vérification que la source déclarée
  apparaît dans le tag — voir § *Ce qui est refusé*.
- **Le scénario 45 et ses assertions A1–A4**, inchangés dans leur intention : seuls
  le nom du tier fort et l'ensemble interdit du tier faible bougent.
- **L'extension de la porte transverse mika#2265 aux sous-répertoires de
  `tests/eval/`.** Trouvaille en chemin, réelle :
  `tests/eval/test_eval_modules_declared.rs` ne balaie que le **premier niveau**,
  donc un scénario posé dans `grounding_regressions/` sans son `pub mod` n'est
  jamais compilé, ne casse rien, et rend son AC verte — exactement la classe que
  cette porte ferme, un répertoire plus bas. **Mesure prise pour ce plan :
  zéro orphelin aujourd'hui** dans les sept sous-répertoires
  (`golden`, `kg_self_knowledge`, `doctrine_regressions`, `grounding_regressions`,
  `skills`, `multi_agent`, `kg_provider_eval`). **Ticket de suivi** avec ce compte
  comme donnée : l'extension est gratuite sur la population actuelle, mais elle
  doit décider du sort de `fixtures/`, des `README.md` et des modules de support,
  ce qui est un arbitrage de périmètre que ce ticket ne pèse pas. En attendant,
  l'étape 1 du contrat de vérification est ce qui tient U8.

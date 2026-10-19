# mika#2194 — vol 3 : le tier 2 de paraphrase obtient son corpus doré et son lecteur Rust, le bash reste la référence

- **Ticket :** [senara-solutions/mika#2194](https://github.com/senara-solutions/mika/issues/2194)
- **Branche :** `refactor/2194/dispatch-lib-sortir-la-logique-du-bash-d`
- **Type :** refactor (strangler, étape « tests dorés » — pas de bascule)
- **Plan antérieur :** `docs/plans/2026-09-30-003-refactor-2194-lecteur-unique-du-callout-plan-plan.md` (vol 1, livré par #2607) et `docs/plans/2026-09-30-004-refactor-2608-phase-2-lecteurs-bash-callout-plan-plan.md` (vol 2, livré par mika#2608). Les deux sont sur `main`.
- **Doctrine en vigueur :** `docs/architecture/dispatch-lib-migration.md` — ce plan la respecte et ne la relitige pas.

---

## 1. Où en est le programme, re-mesuré aujourd'hui

Le ticket parent est un programme de migration. Deux maillons sont livrés, deux
phases dorment avec leur condition de réveil écrite (commentaire opérateur du
2026-09-30T19:03:44Z, qui est la décision de référence) :

| phase | périmètre | état |
|---|---|---|
| 1 | `_extract_plan_path` → `plan_callout.rs`, plus la doctrine | **livrée** (#2607) |
| 2 | `_committed_plan_on_branch`, `_set_up_worktree`, décision sur le 4e lecteur | **livrée** (mika#2608, fermée) |
| 3 | `_parse_disposition` / `_parse_verdict` | **dormante** — réveil : une décision écrite sur `$_DISPOSITION_FUZZY_FILE` |
| 4 | glue bwrap / git / gh / trap / worktree / callback → Python | **dormante** — réveil : un maillon de nature glue + un canal shell→Python |

**La re-mesure du fichier, parce que la doctrine (§ 7) en porte une qui a 19
jours et que le nombre est l'argument de la phase 4.** Au 2026-10-19,
`skills/bundled/_shared/dispatch-lib.sh` fait **738 308 octets, 12 786 lignes,
145 fonctions**. La doctrine enregistre **551 330 octets, 9 757 lignes, 119
fonctions** au 2026-09-30 : **+34 % de lignes et +26 fonctions en 19 jours**. Le
corps du ticket, écrit le 2026-09-05, mesurait « ≈ 300 Ko ». Le fichier a donc
**plus que doublé depuis l'ouverture du ticket**, et il grossit plus vite que le
programme ne le vide. Ce plan ne prétend pas renverser cette tendance ; il dit le
chiffre pour que personne ne lise « deux phases livrées » comme « le fichier
rétrécit » — la doctrine § 7 avait déjà refusé cette lecture, ce vol la refuse
avec un chiffre plus récent.

### Pourquoi le maillon de ce vol n'est pas la phase 3 entière

La phase 3 est dormante et sa condition de réveil est **une décision écrite sur
le canal de retour par fichier**. Ce vol **ne la réveille pas** et ne décide rien
sur `$_DISPOSITION_FUZZY_FILE` : la décision appartient à la bascule, et la
bascule n'est pas dans ce vol. Ce que ce vol fait est l'étape que **le corps du
ticket prescrit textuellement avant toute bascule** :

> **Méthode : strangler, jamais big-bang** — extraire fonction par fonction ;
> chaque bascule est précédée de TESTS DORÉS (mêmes entrées → mêmes sorties que
> le bash) ; le bash reste la référence jusqu'à parité prouvée par fonction.

Donc : le corpus doré et le lecteur Rust du **tier 2 de paraphrase**, avec le
bash conservé comme lecteur de production. Rien ne bascule. C'est la moitié
« tests dorés » de la phrase ci-dessus, prise au mot.

---

## 2. Le maillon, confronté aux trois critères de sélection

Le maillon est `_parse_disposition_fuzzy` (`dispatch-lib.sh:8762`) et
`_parse_verdict_fuzzy` (`:8813`) — le **tier 2** seul, pas leurs parents.

### C1 — Pureté : satisfait, et c'est la lecture « maillon = expression » de la phase 2

La doctrine § 1 écarte explicitement `_parse_disposition` (90 lignes) parce
qu'elle « rend **deux** valeurs, la seconde par un fichier
(`$_DISPOSITION_FUZZY_FILE`) ». **Cet effet de bord n'est pas dans le tier 2 : il
est dans ses parents.** Vérifié ligne par ligne : les écritures du fichier sont à
`:8888` et `:8954` (dans `_parse_disposition`) et `:8976` et `:9047` (dans
`_parse_verdict`). `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy` ne
touchent **aucun** fichier : stdin → un jeton sur stdout, plus un journal sur
stderr. Ce sont des prédicats purs de 50 et 37 lignes.

C'est la leçon que la phase 2 a écrite pour les suivants (§ 6) : *« le maillon est
l'EXPRESSION, jamais la fonction »*. Lue sur la fonction, la phase 3 est
inmigrable ; lue sur l'expression, son tier 2 est exactement de la forme du
maillon de la phase 1.

### C2 — Un seul appelant : satisfait, mesuré

`_parse_disposition_fuzzy` a **un** appelant de production, `:8952`.
`_parse_verdict_fuzzy` en a **un**, `:9045`. Aucun autre site du fichier ne les
nomme hors commentaires.

### C3 — Le Rust doit déjà être, ou devoir être, le lecteur : satisfait, et l'argument est le producteur

C'est le critère qui a déjà refusé une migration de cette classe
(mika-platform#58), donc il doit être argumenté et non supposé. La borne qu'il
pose est : *« ce que **seul** le shell connaît reste au shell et voyage par
stamp »*.

Le texte que le tier 2 lit n'est pas connu du seul shell. `_arch_ask`
(`dispatch-lib.sh:8527-8578`) appelle `mika ask --agent mika-arch --format json`,
donc **le moteur a produit ce texte**, l'a fait passer par sa propre boucle
d'agent, et le persiste déjà (`llm_calls.response_text`, #653). Le shell en
reçoit une copie dans une enveloppe A2A. Décider ce que vaut une réponse
d'architecte est une décision sur la sortie d'un agent du moteur — c'est-à-dire la
classe de décision que le ticket parent veut rendre au moteur, pas celle qui
voyage par stamp.

Le Rust est de plus **déjà** un lecteur du même vocabulaire, sur sa moitié
stricte : `grooming_marker.rs::VERDICT_TOKEN_RE` et `FIRST_PASS_READY_RE` lisent
`READY` / `ITERATE` / `GROOMED` / `ESCALATE`,
`skills/executor.rs::GROOM_ESCALATE_MARKER` lit `Outcome: ESCALATE`. Ce que le
Rust ne lit pas aujourd'hui est la **tolérance de paraphrase** du même jeu de
jetons — exactement la configuration de la phase 1, où le Rust lisait déjà le
callout `Plan` dans `auto_pull` pendant que bash en portait sa propre copie.

### Le critère de canal (§ 2) : ce vol n'ouvre aucun canal, donc S3 ne le borne pas

La doctrine § 2 est catégorique : la phase 2 a porté le coût à **trois
démarrages de `mika` par dispatch dev-groom**, elle a **consommé** la marge du
tableau, et *« la sonde S3 cesse d'être une curiosité et devient la précondition
de la phase suivante »*. S3 — le coût réel d'un démarrage de `mika` — **n'est
toujours pas mesurée**, et elle ne l'est pas depuis le bac à sable de dispatch
(`mika --version` y est refusé par la permission-policy).

**Ce vol ne livre aucune sous-commande et aucun appel à `mika` depuis
`dispatch-lib.sh`.** Le compte reste à trois. C'est ce qui le rend livrable
pendant que S3 dort, et c'est la raison principale pour laquelle la bascule est
dans un vol suivant plutôt qu'ici : la bascule paierait un **quatrième** démarrage
sur un chemin que la doctrine a explicitement bordé de sa précondition, et la
franchir sans la mesure serait instruire sans mesure.

Effet secondaire, et c'est un gain propre : le lecteur bash du corpus **n'a pas
besoin du binaire**. Le bloc de parité de la phase 1 (`test-dispatch-lib.sh:10757`)
appelle `mika_bin_or_skip` et se saute entier quand `mika` est introuvable, parce
que son lecteur bash délègue. Le bloc de ce vol exerce du bash pur : il ne peut
pas se sauter, donc il ne peut pas se lire comme vert alors qu'il n'a rien
mesuré.

---

## 3. Ce que le corpus va mesurer, et qui est déjà visible à la lecture

Un corpus doré ne vaut que par ce qu'il attrape. Trois classes sont **déjà
lisibles dans le code** et aucune n'est couverte par une assertion aujourd'hui.
Ce vol les **mesure et les épingle**. Il n'en corrige **aucune** : la borne B1 de
la doctrine (§ 3) interdit de resserrer une tolérance pour « harmoniser », et un
resserrement de ce tier décide d'un **abandon de ticket** en aval (mika#2020).

### D1 — Le match est en SOUS-CHAÎNE, et le correctif connu est partiel

`grep -oi "$pat"` matche n'importe où dans un mot. Le code le sait à moitié : le
commentaire de `:8832` dit avoir remplacé `ship` par `ship it` pour éviter
« relationship », « ownership », « leadership ». Les motifs restants n'ont pas eu
ce traitement :

| motif | tier | faux positif plausible sur ce dépôt |
|---|---|---|
| `dispatch` | READY | « the dispatcher », « re-dispatch », « dispatch-lib », « dispatch:loop » |
| `fundamental` | ESCALATE | « the plan is fundamentally sound » → **ESCALATE** |
| `revise` | ITERATE | « the revised plan », « a revised bound » |
| `proceed` | READY | « proceeds to open the PR » |

`fundamental` est le plus mordant, parce que la priorité est
`ESCALATE > ITERATE > READY` : une réponse d'architecte sans ligne
`Disposition:` qui dit du bien d'un plan en le qualifiant de *fundamentally
sound* est lue comme un refus. C'est la classe exacte que le corps du ticket
nomme — *« tokens matchés en sous-chaîne #2188 »* — mesurée au lieu d'être citée.

### D2 — Le tier 2 est quasi inerte sur une réponse en français, et un test en vigueur en dépend déjà

Les deux tables sont **entièrement en anglais**. Les réponses de mika-arch sur ce
dépôt sont en français. Ce n'est pas une conjecture : `test-dispatch-lib.sh:13628-13634`
porte un verbatim d'architecte en français avec, en commentaire, la vérification
manuelle que **les cinq tiers ne matchent rien** — la fixture de mika#2641 repose
sur cette propriété.

Donc le tier 2 est un filet dont la surface utile est inconnue. Le corpus la
**mesure** (combien de corps français rendent un jeton) au lieu de la supposer.
Ce vol ne traduit rien : traduire élargirait une tolérance, ce qui est le miroir
de ce que B1 refuse, et sur une population non mesurée.

### D3 — `grep` est orienté LIGNE, un `contains` Rust ne l'est pas

Quatre motifs portent une espace : `human review`, `cannot proceed`,
`needs revision`, `another pass`, `address the following`, `concerns that
require`, `good to go`, `plan is clean`, `cannot approve`,
`human review needed`, `fundamental issues remain`, `plan is ready`,
`no remaining concerns`. `grep` travaille **ligne par ligne**, donc un
`human\nreview` ne matche **pas** côté bash. Un portage naïf en
`text.to_lowercase().contains(pat)` matcherait. C'est la divergence n° 3 de la
phase 1 — *« la capture traverse les lignes »* — sur un autre prédicat.

**La sémantique par ligne fait donc partie de la tolérance portée**, pas de
l'implémentation : le lecteur Rust matche par ligne, et un test le nomme. La
doctrine § 4 l'écrit : la parité se mesure contre le bash, elle ne se décide pas.

---

## 4. Requirements

### R1 — `crates/mika-agent/src/arch_paraphrase.rs` : le lecteur, port fidèle

Un module, deux tables, un matcheur.

- `pub enum ArchPass { First, Second }` — la passe décide de la table.
- `pub enum Paraphrase { Ready, Iterate, Escalate, Groomed }` — le vocabulaire
  fermé, repris des jetons canoniques.
- `pub fn paraphrase(text: &str, pass: ArchPass) -> Option<(Paraphrase, &'static str)>`
  — le jeton et **le fragment qui l'a décidé**. Le fragment n'est pas décoratif :
  le bash l'écrit sur stderr (`matched: '…'`), donc un lecteur qui ne le rend pas
  n'est pas en parité sur ce que le site observe.
- Les tables sont **transcrites littéralement** depuis `dispatch-lib.sh:8776-8797`
  (première passe) et `:8825-8839` (seconde passe), dans **l'ordre du bash** :
  l'ordre décide quel fragment est rendu quand deux motifs d'une même classe
  matchent, donc il est porté et non normalisé.
- Le match est **insensible à la casse**, **en sous-chaîne**, et **par ligne**
  (§ 3 D1 et D3). Un doc-comment au site dit que ces trois propriétés sont la
  tolérance portée et que B1 interdit de les resserrer dans ce vol.
- Priorité `ESCALATE > ITERATE > READY` (première passe) et
  `ESCALATE > GROOMED` (seconde), avec la raison du bash au site : le plus
  conservateur gagne.
- `pub mod arch_paraphrase;` dans `crates/mika-agent/src/lib.rs`.

**Aucun appelant de production.** C'est la propriété centrale du vol et elle est
dite ici plutôt que découverte en revue — voir § 6.

### R2 — Le corpus commun : un répertoire, deux lecteurs, un fichier d'attendus

`crates/mika-agent/tests/fixtures/arch_paraphrase_texts/`, de la même forme que
`plan_callout_bodies/` (phase 1), parce que cette forme a déjà été jugée et que
ses cinq propriétés sont acquises.

`expectations.tsv`, cinq colonnes, en-tête portant la sémantique :

| colonne | sens |
|---|---|
| `fichier` | le corps, dans le même répertoire |
| `pass` | `first` \| `second` — quelle table est interrogée |
| `token` | le jeton attendu, ou `-` pour « aucun » |
| `fragment` | le fragment attendu, ou `-` |
| `classe` | `nominal` \| `substring-fp` \| `french-inert` \| `multiline` — **ce que ce cas existe pour mesurer** |

Deux contraintes de format, héritées de la mesure de la phase 1 et non
re-découvertes :

- **Une colonne sans valeur porte `-`, jamais le vide.** TAB est un caractère
  *whitespace* d'IFS, donc `IFS=$'\t' read` fusionne les tabulations
  consécutives : une colonne vide disparaît et tout ce qui suit se décale,
  pendant que le `split('\t')` Rust lit correctement. *Un format que les deux
  lecteurs ne lisent pas pareil est ce que cette migration existe pour retirer.*
- Les corps sont des **réponses d'architecte réalistes**, dont au moins deux
  verbatim : le français de mika#2641 (`test-dispatch-lib.sh:13634`) et un corps
  mentionnant `dispatch-lib` sans ligne `Disposition:`.

Couverture minimale, une entrée par classe de § 3 et par tier :
`nominal` (un match par jeton et par passe), `substring-fp` (les quatre motifs du
tableau D1), `french-inert` (le verbatim mika#2641 et un second), `multiline`
(un motif à espace coupé par un retour à la ligne).

### R3 — Le lecteur Rust du corpus : `crates/mika-agent/tests/arch_paraphrase_parity.rs`

- Lit le TSV, résout chaque corps, appelle `paraphrase()`, compare `token` et
  `fragment`.
- **Anti-vacuité, trois termes** — c'est le seul mode de panne qu'aucun des deux
  lecteurs ne verrait seul : échec si le TSV est introuvable, échec s'il ne
  déclare aucune ligne, échec s'il nomme un corps absent. Et il **nomme son
  compte de cas**, assertion plutôt qu'affichage.
- **L'assertion auto-nettoyante** porte sur `classe`, et elle porte sur une
  propriété **interne au Rust**, donc elle survit à la bascule du vol suivant —
  c'est la correction que la phase 1 avait dû apporter à son propre plan (§ 4 de
  la doctrine), reprise ici d'emblée :
  - une ligne `substring-fp` dont le match tombe sur une **frontière de mot**
    fait **échouer** le test : le jour où la tolérance est resserrée, la ligne
    rougit et doit partir avec sa décision ;
  - une ligne `french-inert` dont `token` n'est pas `-` fait **échouer** : le
    jour où une table est traduite, l'inertie mesurée cesse d'être vraie et la
    ligne le dit ;
  - une ligne `multiline` dont le match réussit fait **échouer** : c'est la
    sentinelle de la sémantique par ligne.

### R4 — Le lecteur bash du même corpus, dans `test-dispatch-lib.sh`

Jumeau du précédent, sur le **même** TSV. Il appelle `_parse_disposition_fuzzy` et
`_parse_verdict_fuzzy` selon `pass`, compare le jeton de stdout, et compare le
fragment en lisant le stderr du site (c'est là que le bash l'écrit).

- Mêmes trois termes d'anti-vacuité.
- **La sentinelle de sourcing**, obligatoire : le motif dominant du harnais est
  `source "$DISPATCH_LIB" 2>/dev/null || true`, donc un `dispatch-lib` qui cesse
  de sourcer laisserait les assertions vertes sur des fonctions **inexistantes**.
  Le bloc refuse de tourner si `_parse_disposition_fuzzy` et
  `_parse_verdict_fuzzy` ne sont pas définies après le sourcing.
- **Pas de `mika_bin_or_skip`** : ce bloc n'a besoin d'aucun binaire, et c'est
  une propriété à préserver (§ 2).
- La suite est déjà atteinte par un workflow — `make test-dispatch-lib`, appelé
  par `ci.yml:85` — donc rien à déclarer dans `scripts/suite-wiring.tsv`.

### R5 — `scripts/canonical-tokens.tsv` : les lignes du nouveau lecteur

Les gardes mika#2201 comparent l'arbre au TSV. Un lecteur Rust neuf des jetons
`READY` / `ITERATE` / `ESCALATE` / `GROOMED` doit y être déclaré, **classe A**
(`paraphrase`) — le lecteur *lit* la variante, il ne la mé-lit pas, donc le lint
reste silencieux, ce qui est précisément ce que la colonne `tolérance` sert à
dire.

Et l'avertissement que la phase 2 a écrit sur sa propre AC5 vaut ici :
*« les deux gardes sont vertes » n'est pas une preuve que le TSV est à jour* —
`canonical-tokens-survey.sh --check` ne vérifie qu'une direction. Les lignes
ajoutées le sont donc parce que le lecteur existe, pas parce qu'un lint l'a
demandé.

### R6 — Ce que ce vol ne livre PAS, nommément

Chacun de ces refus est une décision, pas un oubli :

- **Aucune bascule.** `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy`
  restent les lecteurs de production, mot pour mot. Le corps du ticket l'exige
  (« le bash reste la référence jusqu'à parité prouvée par fonction ») et c'est
  aussi ce qui fait que **ce vol ne change aucun comportement** — la borne du
  ticket.
- **Aucune sous-commande `mika`, aucun appel à `mika` depuis `dispatch-lib.sh`.**
  Le compte de la doctrine § 2 reste à trois, et S3 reste intacte comme
  précondition de la bascule.
- **Aucun scan anti-copie.** Un scan « zéro lecteur de paraphrase dans
  `dispatch-lib.sh` » serait **rouge à la naissance**, puisque ce vol garde
  délibérément le lecteur bash. La doctrine § 5 a déjà payé cette leçon : *un
  lint rouge le jour où il naît se fait désarmer, et la régression qu'il existe
  pour attraper passe ensuite dans le bruit.* Le scan appartient au vol de la
  bascule, où son zéro est atteignable.
- **Aucune décision sur `$_DISPOSITION_FUZZY_FILE`.** La phase 3 reste dormante
  et sa condition de réveil inchangée.
- **Aucun resserrement, aucune traduction, aucune harmonisation** (B1).
- **Aucune famille de sous-commandes.** La doctrine § 6 fixe le critère de
  bascule — *« la seconde sous-commande de prédicat distincte »* — et ce vol
  n'en livre aucune, donc la population reste à un et il n'y a rien à grouper.

---

## 5. Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | le lecteur Rust rend le même jeton **et** le même fragment que le bash sur chaque ligne du corpus | `cargo test -p mika-agent --test arch_paraphrase_parity` + le bloc bash de `make test-dispatch-lib`, sur le **même** TSV |
| V2 | aucun des deux lecteurs ne peut être vert sur zéro cas | trois termes d'anti-vacuité de chaque côté, et chacun **nomme son compte** |
| V3 | la sémantique **par ligne** est portée | ligne `multiline` : le bash ne matche pas, le Rust ne matche pas, et l'assertion auto-nettoyante rougit si l'un des deux se met à matcher |
| V4 | les faux positifs de sous-chaîne sont **épinglés tels qu'ils sont** | lignes `substring-fp` ; l'assertion rougit le jour où le match devient à frontière de mot |
| V5 | l'inertie sur le français est **mesurée** | lignes `french-inert`, dont le verbatim mika#2641 ; `token = -` des deux côtés |
| V6 | la priorité `ESCALATE > ITERATE > READY` et `ESCALATE > GROOMED` est portée | un cas par corpus où deux classes matchent le même corps |
| V7 | le bloc bash ne peut pas se sauter ni mesurer du vide | pas de `mika_bin_or_skip` ; la sentinelle de sourcing refuse de tourner sans les deux fonctions |
| V8 | aucun comportement de production ne change | `git diff` sur `dispatch-lib.sh` ne touche que la suite de tests et les commentaires ; les corps de `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy` sont **identiques octet pour octet** |
| V9 | les gardes de jetons canoniques restent vertes | `scripts/check-canonical-tokens.sh`, `scripts/canonical-tokens-survey.sh --check`, `cargo test -p mika-agent canonical_tokens` |
| V10 | pipeline complet | `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `make test-dispatch-lib`, `scripts/verify-pipeline.sh origin/main` |

---

## Fire-Disposition

Ce plan livre des détecteurs : la parité (`arch_paraphrase_parity.rs` et le bloc
bash), les six termes d'anti-vacuité, et les trois assertions auto-nettoyantes de
R3. Leur chemin de succès est « aucun écart », donc la règle mika#2306 s'applique.

**Option retenue : (a) — exception nommée en allowlist, avec assertion
auto-nettoyante.** C'est le mécanisme que la phase 1 a déjà fait juger sur ce
même programme (doctrine § 4), et le porteur de l'allowlist est la colonne
`classe` de `expectations.tsv` — grep-visible, une ligne par violation, chaque
ligne nommant la donnée précise.

La population d'exceptions est **non vide par construction** : ce sont les trois
classes de § 3, et elles existent dans le bash d'aujourd'hui.

| ligne | la donnée précise | le suivi | ce qui la rend auto-nettoyante |
|---|---|---|---|
| `classe = substring-fp` | le motif et le mot porteur (`fundamental` dans « fundamentally sound », `dispatch` dans « the dispatcher », `revise` dans « the revised plan », `proceed` dans « proceeds to ») | mika#2194 (vol de la bascule) | le test **échoue** si le match tombe sur une frontière de mot : resserrer la tolérance fait rougir la ligne, qui doit partir avec sa décision |
| `classe = french-inert` | le verbatim français et le fait qu'aucun des cinq tiers ne rend de jeton | mika#2194 (vol de la bascule) | le test **échoue** si `token` cesse d'être `-` : traduire une table fait rougir la ligne |
| `classe = multiline` | le motif à espace coupé par un retour à la ligne | mika#2194 (vol de la bascule) | le test **échoue** si l'un des deux lecteurs se met à matcher |

Chaque ligne porte son numéro de ticket dans le TSV, et le fichier d'en-tête
nomme la propriété : **une classe déclarée dont la violation a cessé fait échouer
le test**. C'est ce qui distingue une exception d'un contournement, et c'est la
raison pour laquelle l'assertion porte sur une propriété **interne au Rust**
(frontière de mot, `-`, match par ligne) et non sur la relation bash↔Rust, qui
cesse d'avoir deux côtés après la bascule du vol suivant.

**Aucune exception n'est accordée au-delà de ces trois classes.** Si
l'implémentation mesure une quatrième divergence, elle reçoit sa propre classe et
sa propre ligne — et si elle n'en mesure aucune de plus, aucune classe vide n'est
introduite : une allowlist livrée vide est un geste que ce dépôt a déjà refusé
par écrit (mika#2201).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/arch_paraphrase.rs` — lecteur, deux tables, sémantique par ligne, doctrine au site | 100 |
| `crates/mika-agent/src/arch_paraphrase.rs` — tests inline (priorité, par-ligne, casse, contrôles négatifs) | 58 |
| `crates/mika-agent/tests/fixtures/arch_paraphrase_texts/` (corps) + `expectations.tsv` | 28 |
| `crates/mika-agent/tests/arch_paraphrase_parity.rs` — lecteur Rust du corpus, anti-vacuité, assertions auto-nettoyantes | 34 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — lecteur bash du même corpus, sentinelle de sourcing, anti-vacuité | 44 |
| `scripts/canonical-tokens.tsv` — les lignes du nouveau lecteur (classe A) | 8 |
| `crates/mika-agent/src/lib.rs` — `pub mod arch_paraphrase;` | 1 |

Total estimé : 273 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi il est seul.** 273 < 279 et 273 < 1000 : les
deux bornes passent, la plus serrée (279) de 6 lignes. La marge est mince et elle
est tenue par les deux refus de R6 qui coûtent le plus : **pas de canal CLI**
(≈ 80 lignes de sous-commande, de codes de sortie et de câblage `clap`) et **pas
de scan anti-copie** (≈ 50 lignes de prédicat et de contrôles de bonne foi). Ces
deux-là ne sont pas reportés pour tenir le chiffre — ils sont reportés parce que
la doctrine les borne (S3 pour le premier, § 5 pour le second), et le chiffre le
confirme.

Le découpage est en **vols**, pas en phases d'un même vol : ce vol est mergeable
seul, son plancher est « aucun comportement de production ne change » (V8), et il
laisse l'arbre dans un état où la parité a **deux côtés réels**. Le vol suivant —
la bascule — est la suite, et il n'est **pas** décrit ici.

Tracked in: senara-solutions/mika#2194

Ce `Tracked in:` désigne le ticket parent lui-même, et c'est la décision de
l'opérateur plutôt qu'une facilité : *« Chacune des phases 3 et 4 aura son propre
ticket quand sa condition sera remplie. D'ici là, ce ticket parent reste ouvert et
les porte. »* (commentaire du 2026-09-30T19:03:44Z). La condition de réveil de la
phase 3 n'est pas remplie par ce vol, donc le vol suivant n'a pas encore de
ticket à lui, et le porteur légitime est le parent.

---

## Definition of Done

- [ ] `crates/mika-agent/src/arch_paraphrase.rs` existe, est déclaré dans
      `lib.rs`, et porte au site la raison pour laquelle sa tolérance est portée
      telle quelle (B1).
- [ ] Le corpus et son `expectations.tsv` existent, avec au moins une entrée par
      classe (`nominal`, `substring-fp`, `french-inert`, `multiline`) et par
      passe (`first`, `second`).
- [ ] Les **deux** lecteurs du corpus passent, et **chacun nomme son compte de
      cas** (non nul).
- [ ] Les trois assertions auto-nettoyantes sont en place, et chacune a été vue
      **rouge** une fois pendant l'implémentation (contrôle négatif : muter le
      lecteur Rust, constater la rougeur, revenir).
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre
      **aucune** modification des corps de `_parse_disposition_fuzzy` et
      `_parse_verdict_fuzzy`.
- [ ] `scripts/canonical-tokens.tsv` déclare le nouveau lecteur, et les deux
      gardes mika#2201 sont vertes.
- [ ] `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --check`, `make test-dispatch-lib`,
      `make verify-bundled-skills`, `scripts/check-suite-wiring.sh` et
      `scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] `docs/architecture/dispatch-lib-migration.md` est mis à jour sur trois
      points : la re-mesure du fichier (§ 7), l'entrée de ce vol dans le tableau
      de phasage (§ 6), et les trois classes mesurées de § 3 (§ 4, à côté des
      trois divergences de la phase 1).
- [ ] **Le corps de PR dit explicitement que le ticket mika#2194 n'est PAS
      clos**, et ne contient **aucun** `Closes #2194`, `Fixes #2194` ni
      `Resolves #2194`. C'est une exigence de l'opérateur, née d'un incident :
      le corps auto-rescue de #2607 portait `Closes #2194` et a fermé le ticket
      parent par effet de bord (commentaire du 2026-09-30T19:03:44Z, « Je rouvre
      le ticket »). La référence s'écrit `Refs mika#2194 (vol 3 ; le ticket
      parent reste ouvert et porte les vols suivants)`.
- [ ] Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)` et le
      vol ne passe ni par `ce-simplify-code` ni par `ce-code-review`.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` — il porte une
`## Direction` et des `## Bornes`. Les critères ci-dessous en sont dérivés, bornés
au périmètre de **ce vol**.

- [ ] **AC1.** Le tier 2 de paraphrase a un lecteur Rust, `arch_paraphrase.rs`,
      qui transcrit littéralement les deux tables du bash — mêmes motifs, même
      ordre, même priorité — et dont le doc-comment dit que la casse, la
      sous-chaîne et le match par ligne sont la **tolérance portée** et non un
      détail d'implémentation.
- [ ] **AC2.** Un corpus doré unique est lu par les **deux** lecteurs, sur le
      même fichier d'attendus, et la parité y est **verte sur un compte de cas
      non nul déclaré par chacun des deux**. C'est la borne du corps du ticket
      (« mêmes entrées → mêmes sorties que le bash »), et c'est la seule fenêtre
      où elle est une **preuve** : après la bascule du vol suivant, le bash
      appellera le Rust et un lecteur comparé à lui-même est toujours d'accord
      avec lui-même.
- [ ] **AC3.** Les trois classes de § 3 sont **mesurées et épinglées, et aucune
      n'est corrigée** : les faux positifs de sous-chaîne, l'inertie sur le
      français, et le match par ligne. Chacune porte une assertion
      auto-nettoyante qui rougit le jour où la propriété mesurée cesse d'être
      vraie.
- [ ] **AC4.** Un contrôle négatif par détecteur : une mutation du lecteur Rust
      fait rougir la parité, et une mutation de chaque classe fait rougir son
      assertion auto-nettoyante. Les trois rougeurs sont constatées pendant
      l'implémentation, pas supposées.
- [ ] **AC5.** **Aucun changement de comportement.** Les corps des deux fonctions
      bash de production sont identiques à ceux de `origin/main`, aucun appel à
      `mika` n'est ajouté à `dispatch-lib.sh`, et le compte de démarrages par
      dispatch reste à trois (doctrine § 2).
- [ ] **AC6.** `scripts/canonical-tokens.tsv` déclare le nouveau lecteur en
      classe A, et les deux gardes mika#2201 sont vertes.
- [ ] **AC7.** `docs/architecture/dispatch-lib-migration.md` porte la re-mesure
      du fichier au 2026-10-19, l'entrée de ce vol, et les trois classes
      mesurées — pour que le vol suivant ne les redécouvre pas.
- [ ] **AC8.** Le ticket mika#2194 **reste ouvert** à l'issue de la PR, et le
      corps de PR le dit.

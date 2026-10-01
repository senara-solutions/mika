# mika#2609 — la divergence fences est tranchée, et son renvoi cesse de pointer dans le vide

**Ticket :** `mika issue#2609` · **Parent :** mika#2194 · **Labels :** `bug`,
`p3-nice-to-have`, `loop-substrate`, `dispatch:loop`
**Type :** `fix` · **Branche :** `fix/2609/plan-callout-divergence-fences-bash-rust`

Le suivi exigé par la Definition of Done de mika#2194 phase 1 (« La divergence
fences porte un **numéro de suivi réellement déposé** (pas un placeholder) au
moment d'ouvrir la PR », plan `2026-09-30-003`, l. 554-555) n'a pas été déposé au
merge de #2607. Ce ticket **est** ce numéro, et il porte en plus la décision que
le plan de mika#2194 avait explicitement refusé de prendre par défaut (l. 493-494).

**Le livrable est petit — un TSV, un README, une fixture, deux commentaires de
production, une entrée de registre et un test.** La valeur de ce plan est
ailleurs : dans cinq rectifications que la lecture du code impose au ticket, dont
une qui rendrait un futur réveil impossible à mener s'il suivait la lettre d'AC2.

---

## 1. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable de ce plan. Les mesures sont datées du 2026-10-01 sur
HEAD `64c9383c`.

### M0 — la mesure de population : faite, consignée, attribuée (AC1 déjà satisfait)

Elle vient de l'opérateur (MPC, commentaire du 2026-10-01, lecture seule), et ce
plan **ne la refait pas**. Elle est reproduite ici parce qu'elle devient le corps
de la condition de réveil :

```
gh issue list --state all --limit 2000 --json number,state,body
```

1 261 tickets ; repérage ligne à ligne des callouts `> - **Plan:**` avec un état
« dans un bloc clôturé » (``` ``` ``` / `~~~`) ; chemin normalisé (préfixe de
dépôt retiré) confronté à
`git ls-tree -r --name-only origin/main -- docs/plans`.

| population | compte |
|---|---|
| callouts `Plan` au total | 639 |
| hors bloc clôturé (lecture commune bash et Rust) | 630 |
| **dans un bloc clôturé** (la divergence) | **9** |
| … pointant un plan **existant** (le cas que le `-f` ne rattrape pas) | **2** — #1204, #1144 |
| … sur un ticket **ouvert** (seule population dispatchable) | **0** |

Les sept autres sont des gabarits (`docs/plans/<file>`, `…`, `<plan_path>`), que
le `-f` rattrape.

**AC1 est satisfait par ce commentaire.** Ce qui manque n'est pas la mesure mais
sa **persistance** : elle vit aujourd'hui dans un commentaire GitHub d'un ticket
qui sera fermé, c'est-à-dire exactement la dispersion que `docs/dormeurs.md` a
remplacée le 2026-09-03. R5 la reporte dans le dépôt.

### M1 — la ligne n'est plus 112, et c'est la preuve de la classe

Le ticket cite `expectations.tsv:112` sur main `db476b5d`. Sur HEAD elle est à
la **ligne 132** : mika#2608 a inséré les trois fixtures `gate-*.md` et leurs
commentaires au-dessus.

Deux conséquences. (a) Le correctif doit cibler **le texte**, jamais le numéro de
ligne — un numéro de ligne pourrit en silence, et le TSV lui-même l'écrit déjà
pour ses propres références (« `chemin::symbole`, **jamais un numéro de ligne** »,
doctrine mika#2201). (b) Plus important : **la référence morte a survécu à un
refactor du fichier qui la porte**, sans qu'aucun test ne rougisse. Ce n'est pas
une coquille, c'est une classe — et c'est ce qui justifie R6 (un détecteur) plutôt
qu'une simple correction d'instance.

### M2 — quatre surfaces mortes, pas deux ; plus deux commentaires de production périmés

AC3 nomme `expectations.tsv` et le README. L'inventaire exhaustif en trouve deux
de plus, de même nature, et deux sites de production dont le raisonnement devient
faux :

| # | site | ce qu'il dit | défaut |
|---|---|---|---|
| 1 | `…/plan_callout_bodies/expectations.tsv:132` | « Suivi : voir « Ticket de suivi » dans README.md § La divergence fences. » | **ni la section ni le ticket n'existent** |
| 2 | `…/plan_callout_bodies/README.md:60` | cellule « corrigée ? » de `fences-quoted-callout.md` | aucun numéro de suivi |
| 3 | `…/plan_callout_bodies/README.md:65` | « Le jour où la divergence est tranchée, la ligne rougit et doit être retirée » | aucun numéro de suivi |
| 4 | `…/plan_callout_bodies/fences-quoted-callout.md:12` | « c'est ce que **le ticket de suivi** doit trancher » | aucun numéro de suivi |
| 5 | `skills/bundled/_shared/dispatch-lib.sh:9019` | « cette moitié-là n'existe que côté Rust … ici un faux positif est déjà rattrapé par le test `-f` qui suit » | l'asymétrie est assumée **sans citer de décision**, et la raison est celle que le ticket qualifie de « vraie à moitié » |
| 6 | `crates/mika-cli/src/commands/plan_callout.rs:97-99` | « Demander `Strip` ici serait une correction de comportement, que **les bornes du ticket interdisent pendant la migration** » | **devient faux** : les phases 1 et 2 sont mergées, la migration est finie. La raison de `Keep` n'est plus une borne de ticket, c'est la décision de ce ticket-ci |

Les quatre premières sont l'exécution d'**AC3** (élargie de deux surfaces de même
nature, trouvées en chemin — à dire plutôt qu'à glisser). Les sites **5 et 6**
sont l'exécution d'**AC2** : « une décision écrite » doit vivre au site où elle
s'applique, et le site 6 est celui qui porte le mot qui décide.

### M3 — le mécanisme qu'AC2 nomme ne marche pas, et c'est la rectification centrale

AC2 écrit : « Si la politique change, la fixture `fences-quoted-callout.md` passe
de `divergent-fences` à `equal`, et l'assertion auto-nettoyante le vérifie. »

**C'est inexact, et le suivre casserait deux assertions.** `parity` ne décrit pas
la politique d'un lecteur : elle décrit une propriété **du corps**.
`plan_callout_parity.rs:220-221` calcule les deux côtés sur le *même* `body` —

```rust
let keep  = plan_callout(&body, FenceHandling::Keep).map(|c| c.raw);
let strip = plan_callout(&body, FenceHandling::Strip).map(|c| c.raw);
```

— et le dépôt l'écrit déjà noir sur blanc, dans le doc-comment de
`mika2194_une_ligne_pre_switch_declare_une_divergence` : « `divergent-fences` est
`both`, parce que sa divergence est **interne au Rust** (`Keep`↔`Strip`) et reste
donc mesurable après la bascule. »

Donc changer la politique du canal **ne change rien** à `Keep(corps) ≠
Strip(corps)`. Si un futur réveil passait la ligne à `equal` :

- la branche `else` du test rougirait — `assert_eq!(keep, strip)` échoue, puisque
  `Keep ≠ Strip` reste vrai pour ce corps ;
- et **retirer** la ligne ferait rougir l'anti-vacuité `divergences_declarees > 0`
  : mesuré, `grep -v '^#' expectations.tsv | grep divergent-fences` rend **une
  seule** ligne de données, donc elle porte seule cette assertion.

Un réveil fidèle à la lettre d'AC2 verrait deux tests rouges et pourrait conclure
à tort que l'alignement est impossible. **La procédure correcte est en M4, et R7
l'écrit à l'endroit où le réveil la lira.**

### M4 — le mécanisme qui marche existe déjà, et c'est la colonne `rc`

`mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore`
(`crates/mika-cli/src/commands/plan_callout.rs`, module de tests) appelle
`decide(&body, false)` — donc le site qui choisit `Keep` — et compare à la colonne
**`rc`** du TSV. Pour `fences-quoted-callout.md`, `rc = 0`.

Donc aligner le canal fait **déjà** rougir ce test aujourd'hui, avec
« `fences-quoted-callout.md`: aucun callout alors que rc=0 ». Et côté bash, la
colonne `rc` est lue à quatre passages de `test-dispatch-lib.sh` (l. 9861, 9942,
9971, 10026/10056), qui rougiraient aussi.

**Deux colonnes, deux propriétés, et c'est ce que le ticket confond :**

| colonne | asserte | invariante au choix du canal ? |
|---|---|---|
| `parity` | `Keep(corps)` ≠ `Strip(corps)` — une propriété du **corps** | **oui** — ne bouge jamais pour un alignement |
| `rc` | ce que **le canal** rend sur ce corps | **non** — c'est elle qui bascule `0 → 1` |

### M5 — le geste d'alignement est un mot, à un site unique

Inventaire complet des choix de `FenceHandling` **en production** :

| site | politique | sert |
|---|---|---|
| `crates/mika-cli/src/commands/plan_callout.rs:99` | **`Keep`** | **tous** les lecteurs bash, via `mika plan-callout` |
| `crates/mika-agent/src/auto_pull.rs:537` | `Strip` | le prédicat de promotion `is_groomed` |
| `crates/mika-agent/src/plan_callout.rs:220-221` | — | la définition de l'enum |

`crates/mika-agent/src/milestone_manager/reader.rs:657` porte `Keep` mais est
**sous `#[cfg(test)]`** (le module commence l. 582) : ce n'est pas un lecteur de
production, et l'inventaire ci-dessus est donc exhaustif.

Conséquence : depuis mika#2194/#2608, « la politique de fences du bash » n'est
plus dans le bash — c'est un mot dans le CLI. **Le coût d'implémentation de
l'alignement n'est donc pas ce qui décide** (il est trivial), ce qui décide est la
population et la direction du fail-safe.

### M6 — la direction du fail-safe penche *contre* l'alignement

Argument que ni le ticket ni le commentaire ne portent, et qui renforce la
décision au lieu de la contredire :

| politique | mode de panne | coût |
|---|---|---|
| `Keep` (aujourd'hui) | **faux positif** — lit un callout cité dans un bloc | rattrapé par le `-f`, **sauf** si le plan cité existe : population mesurée **0 ouvert** |
| `Strip` (l'alignement) | **faux négatif** — ignore un callout qui n'existe que dans un bloc | `_detect_plan_on_branch` partirait sur `/mika` au lieu de `/ce-work <plan>` |

Le coût du faux négatif est écrit en production, dans le commentaire voisin
(`dispatch-lib.sh:9000-9002`) : « Corriger `is_groomed` seul aurait déplacé la
mort ici — la promotion aurait réussi, le dispatch serait mort plus loin. »
L'autre lecteur, `_committed_plan_on_branch`, est fail-open de ce côté
(`dispatch-lib.sh:2720` : « le grooming procède (mika#2608) »), donc bénin.

**Bilan : aligner échangerait un faux positif de population mesurée vide contre
un faux négatif sur le chemin dont la panne est la mort du dispatch.** La
décision opérateur n'est donc pas seulement économique, elle est plus sûre.

### M7 — la fixture n'exerce pas le cas dangereux, et il faut le dire

Le plan que `fences-quoted-callout.md` cite —
`docs/plans/2026-09-30-001-fix-2194-exemple-cite-plan.md` — **n'existe pas** dans
le dépôt (vérifié). Donc sur la fixture elle-même, le `-f` *rattraperait*.

Elle exerce la divergence de **lecture** (`Keep ≠ Strip`), ce qui est exactement
ce que l'assertion auto-nettoyante demande, et elle est correcte pour cet usage.
Mais elle ne porte **pas** le cas dangereux de bout en bout, et un lecteur pressé
croira le contraire. À nommer dans le README (R2), et **à ne pas corriger** : lui
faire pointer un plan réel changerait son objet sans rien acheter (aucune
assertion du corpus n'exerce le `-f`), et la suppression de ce plan un jour la
casserait pour une raison sans rapport.

### M8 — la population qui cite un callout dans un bloc est structurellement méta

Précédent maison, à un fichier de là : `_write_canonical_callout`
(`dispatch-lib.sh:7540-7545`) strip le préambule **seulement**, avec sa raison
écrite — « le même texte peut légitimement apparaître plus bas dans un bloc
clôturé — un ticket qui documente le format du callout cite ces lignes exactes.
mika#2012's own issue body does. »

Le dépôt a donc déjà mesuré *qui* cite un callout dans un bloc : **les tickets qui
documentent le format**. C'est un argument de **mécanisme**, pas de statistique :
il explique pourquoi la population mesurée est vide côté tickets ouverts, et
pourquoi elle a de bonnes raisons de le rester — un ticket méta n'est pas un
ticket à dispatcher. Les deux occurrences existantes (#1204, #1144) sont fermées,
ce qui est cohérent.

### M9 — « dormeur visible » a un site dans ce dépôt, avec un contrat

`docs/dormeurs.md` existe (12 entrées) et son contrat est explicite : « Une
condition de réveil est valable quand un lecteur peut dire, **sans contexte**, si
elle est remplie », et « **Réveil.** Quand la condition est remplie, rouvrir le
ticket GitHub cité et retirer la ligne d'ici. »

Le commentaire opérateur écrit « **Condition de réveil (dormeur visible)** ».
Dans ce dépôt, « visible » désigne ce registre — sinon la condition reste dans un
commentaire d'un ticket fermé, c'est-à-dire la dispersion que le registre a
remplacée. **Inscrire est donc l'exécution fidèle de la décision, pas un
élargissement du périmètre.**

Une tension à trancher explicitement, sans quoi un lecteur conclura à une
incohérence : mika#2609 sera **fermé livré** (AC1/AC2/AC3), et **inscrit
dormant**. Les deux coexistent parce que ce qui dort n'est pas son AC — ce sont
ses trois AC qui sont livrés — mais la **re-décision** d'alignement, dont la
condition d'exécution (une population non vide) n'est pas remplie. C'est la
définition même d'un dormeur, et non « un ticket qu'on préfère ne pas faire »
(que le registre renvoie, à juste titre, vers une fermeture sur son propre
tracker). L'entrée doit le dire en une clause.

---

## 2. La décision (AC2)

> **Le rattrapage partiel suffit. La divergence bash↔Rust sur les blocs clôturés
> est assumée, documentée et laissée en place. `crates/mika-cli/src/commands/plan_callout.rs`
> garde `FenceHandling::Keep`.**

Elle est **prise par l'opérateur** (commentaire du 2026-10-01) ; ce plan
l'enregistre, l'ancre aux sites où elle s'applique, et y ajoute l'argument M6
qu'elle ne portait pas. Trois appuis, dans l'ordre de leur poids :

1. **La population que le `-f` ne rattrape pas est vide côté dispatchable** (M0 :
   2 tickets fermés, 0 ouvert). `dispatch-lib` ne lit le corps que d'un ticket
   qu'il dispatche, donc ouvert. Aligner achèterait une garde sur une population
   vide.
2. **La direction du fail-safe est défavorable** (M6) : l'alignement échange un
   faux positif vide contre un faux négatif sur `_detect_plan_on_branch`, dont la
   panne est la mort du dispatch.
3. **La population a une raison mécanique d'être vide** (M8) : citer un callout
   dans un bloc est le geste d'un ticket qui *documente le format*.

**Conséquence sur le corpus : rien ne change.** `fences-quoted-callout.md` reste
`rc = 0` et `parity = divergent-fences`, et l'assertion auto-nettoyante continue
d'exiger la divergence — c'est l'état qui ne touche à rien, et c'est voulu.

**Condition de réveil** (reportée au registre par R5) : la commande de M0 rend
**au moins un ticket OUVERT** dont un callout `Plan` cité dans un bloc clôturé
pointe un plan existant sur `origin/main`. À ce moment-là l'alignement redevient
une décision à prendre, avec un cas réel — et la procédure est celle de R7, pas
celle qu'AC2 décrit.

---

## 3. Requirements

**R1 — `expectations.tsv` cite le numéro.** Remplacer la ligne « Suivi : voir
« Ticket de suivi » dans README.md § La divergence fences. » par un renvoi à
`mika#2609` portant la décision en une phrase et le compte de M0. Le bloc de
commentaire au-dessus de la ligne `fences-quoted-callout.md` est le site ; aucune
**ligne de données** n'est touchée (`rc`, `raw`, `normalized`, `parity`, `phase`
restent identiques au bit).

**R2 — le README cite le numéro, et nomme ce que la fixture ne mesure pas.**
Trois éditions dans
`crates/mika-agent/tests/fixtures/plan_callout_bodies/README.md` :
(a) la cellule « corrigée ? » de `fences-quoted-callout.md` porte
« **non — tranché par mika#2609** » avec la raison en une clause ;
(b) le paragraphe de l'assertion auto-nettoyante nomme `mika#2609` ;
(c) une clause dit que la fixture exerce `Keep ≠ Strip` et **non** le cas
dangereux de bout en bout, le plan qu'elle cite n'existant pas (M7).
Aucune section « Ticket de suivi » n'est créée : R1 renvoie au **ticket**, pas à
une section, ce qui retire la référence croisée au lieu de la réparer.

**R3 — la fixture cite le numéro.** Dans
`fences-quoted-callout.md`, « c'est ce que le ticket de suivi doit trancher »
devient « c'est ce que **mika#2609** a tranché », avec la décision en une clause.
Le **bloc clôturé** du corps n'est pas touché — c'est la donnée mesurée, et la
modifier changerait ce que le corpus mesure.

**R4 — les deux sites de production portent la décision.**
(a) `skills/bundled/_shared/dispatch-lib.sh:9019` : le commentaire garde sa
raison et gagne « la population est mesurée vide côté tickets ouverts (mika#2609,
2026-10-01) ».
(b) `crates/mika-cli/src/commands/plan_callout.rs:97-99` : remplacer « les bornes
du ticket interdisent une correction de comportement pendant la migration » —
devenu faux (M2 §6) — par la décision mika#2609 et la direction du fail-safe
(M6). **Le mot `Keep` ne change pas** ; c'est sa justification qui est mise à
jour. C'est le site le plus important des deux : un futur lecteur qui envisage
l'alignement lit celui-là.

**R5 — l'entrée au registre des dormeurs.** Une ligne dans
`docs/dormeurs.md` § Registre, nommant mika#2609, son sujet (l'alignement de la
politique de fences du canal, non l'AC du ticket — M9), et la condition de réveil
**sous forme de commande reproductible** (M0), seul format que le contrat du
registre accepte.

**R6 — le détecteur de référence croisée morte.** Deux assertions dans
`crates/mika-agent/tests/plan_callout_parity.rs`, qui ferment la **classe** plutôt
que l'instance (M1) :
(a) `mika2609_le_renvoi_au_suivi_de_la_divergence_porte_un_numero` — tant qu'une
ligne `divergent-fences` existe dans le TSV, le TSV, le README et le corps de la
fixture concernée citent un `mika#<n>` ; aucun des trois ne porte la formule
« ticket de suivi » sans numéro adjacent.
(b) `mika2609_un_renvoi_a_une_section_du_readme_designe_un_heading_reel` — toute
référence de la forme `README.md § <titre>` dans le TSV correspond à un heading
réellement présent dans le README voisin. **C'est l'assertion qui aurait rougi le
2026-09-30** et qui aurait survécu au refactor de mika#2608.
Les deux portent leur anti-vacuité : elles échouent si elles n'exercent aucune
surface (fichier introuvable, zéro ligne `divergent-fences`).

**R7 — la procédure de réveil, écrite là où le réveil la lira.** Dans le bloc de
commentaire de R1, la séquence correcte d'un alignement, qui corrige M3 :

1. `crates/mika-cli/src/commands/plan_callout.rs` : `Keep` → `Strip` (un mot, M5).
2. `expectations.tsv` : `fences-quoted-callout.md` passe `rc` de **`0` à `1`** —
   c'est la colonne qui bascule (M4).
3. `parity` **reste** `divergent-fences` et la ligne **n'est pas retirée** :
   `Keep ≠ Strip` demeure vrai pour ce corps, et la retirer ferait rougir
   l'anti-vacuité `divergences_declarees > 0` (M3).
4. Ce qui atteste l'alignement est
   `mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore` (dans
   `mika-cli`) plus les quatre passages `rc` de `test-dispatch-lib.sh`, **pas**
   `mika2194_les_divergences_declarees_sont_reelles_et_les_autres_absentes`.

---

## 4. Verification contract

| # | vérification | attendu |
|---|---|---|
| **V1** | `cargo test -p mika-agent --test plan_callout_parity` | vert ; le `println!` de chaque test rend un compte **non nul** |
| **V2** | **contrôle négatif de R6(a), à voir ROUGE** : remettre dans `expectations.tsv` la formule « voir « Ticket de suivi » dans README.md § La divergence fences » | `mika2609_le_renvoi_au_suivi_de_la_divergence_porte_un_numero` **échoue** |
| **V3** | **contrôle négatif de R6(b), à voir ROUGE** : écrire dans le TSV un renvoi `README.md § Section Qui N'Existe Pas` | `mika2609_un_renvoi_a_une_section_du_readme_designe_un_heading_reel` **échoue** |
| **V4** | **contrôle de bonne foi** : le détecteur doit être vert sur l'arbre **corrigé**, et son anti-vacuité doit avoir exercé ≥ 1 ligne `divergent-fences` | vert, compte ≥ 1 |
| **V5** | `cargo test -p mika-cli` | vert et **inchangé** — `mika2194_le_canal_repond_comme_le_lecteur_sur_le_corpus_dore` passe toujours, puisque `rc` et `Keep` ne bougent pas |
| **V6** | `bash skills/bundled/_shared/test-dispatch-lib.sh` | vert ; le TSV reste parsable — seules des lignes `#` sont ajoutées, que `grep -cv '^[[:space:]]*\(#\|$\)'` (l. 9814) exclut, donc `M2194_DECLARED` ne bouge pas |
| **V7** | `git diff` sur les **lignes de données** du TSV | **vide** — R1 ne touche que des commentaires |
| **V8** | `bash scripts/check-canonical-tokens.sh` | vert — l'ajout de `mika#2609` dans des commentaires ne crée aucun jeton de callout ; les corps de fixture citent leurs callouts dans des blocs clôturés, que le lint traite comme des mentions |
| **V9** | `cargo clippy --all-targets` et `cargo fmt --check` | propres |
| **V10** | relecture de l'entrée R5 contre `docs/dormeurs.md` § « Contrat d'une entrée » | la condition est décidable **sans contexte** (une commande, un seuil) |

V2, V3 et V4 vont ensemble : sans les deux rouges, « le détecteur est vert »
serait indistinguable de « le détecteur ne regarde rien » — la classe mika#2205,
qui est précisément celle dont ce ticket est une occurrence.

---

## Fire-Disposition

Ce plan livre un détecteur (**R6**, deux assertions dont le chemin de succès est
« aucune référence morte trouvée »). Disposition retenue :

**(a) exception nommée en allowlist — allowlist livrée VIDE.**

- **Aucune violation existante ne subsiste** : R1–R4 corrigent les quatre surfaces
  mortes *dans le même commit* que R6, donc le détecteur atterrit vert sans
  exemption. C'est la forme que le dépôt emploie partout (« allowlist livrée
  vide »), et la doctrine mika#2201 s'applique : **quand il tire, on corrige la
  référence, on n'ajoute pas une ligne d'allowlist.**
- **L'allowlist est matérialisée** — une constante nommée, grep-visible, à côté du
  test — précisément pour que son absence de contenu soit un fait lisible et non
  un oubli, et pour qu'une future exemption soit un ajout visible en revue.
- **L'assertion auto-nettoyante qui la garde vide** : une entrée d'allowlist qui
  ne désigne plus une surface réelle (fichier absent, ou surface devenue conforme)
  fait **échouer** le test. Une exemption ne peut donc pas devenir périmée en
  silence — c'est la propriété que le corpus voisin énonce déjà pour `parity`, et
  elle est reprise telle quelle.
- **Portée bornée, et dite** : le détecteur ne voit que
  `crates/mika-agent/tests/fixtures/plan_callout_bodies/`. Une référence morte
  ailleurs dans le dépôt reste invisible — l'étendre demanderait un scan global,
  qui est hors périmètre (§ 6) et n'a aucune population mesurée.

Les options (b) et (c) sont écartées : (b) livrer désarmé n'a pas de sens pour un
détecteur qui est vert dès son commit ; (c) halte-et-remontée n'a pas d'objet,
l'opérateur ayant déjà tranché la seule décision du ticket.

---

## 5. Definition of Done

- [ ] R1–R7 livrés.
- [ ] V1–V10 verts, **V2 et V3 vus rouges** avant correction et consignés comme
      tels dans le corps de PR.
- [ ] AC1–AC3 satisfaites ; AC1 l'est par le commentaire opérateur, et le corps
      de PR **l'attribue** plutôt que de se l'approprier.
- [ ] Aucune ligne de données d'`expectations.tsv` modifiée (V7).
- [ ] `crates/mika-cli/src/commands/plan_callout.rs` porte toujours
      `FenceHandling::Keep` — la décision est de **ne pas** aligner.
- [ ] L'entrée de `docs/dormeurs.md` porte une commande, pas une intention, et
      dit en une clause pourquoi un ticket livré peut être dormant (M9).
- [ ] Le corps de PR nomme M3 (le mécanisme qu'AC2 décrit ne marche pas) et M4
      (celui qui marche), parce que c'est la correction dont un futur réveil
      dépend.

---

## 6. Acceptance criteria

Transcrites verbatim du corps de mika#2609 :

- [ ] **AC1.** La mesure de population est faite et consignée ici (commande et
      compte).
- [ ] **AC2.** Une décision écrite : le rattrapage partiel suffit, ou bien le bash
      adopte la politique de fences du Rust. Si la politique change, la fixture
      `fences-quoted-callout.md` passe de `divergent-fences` à `equal`, et
      l'assertion auto-nettoyante le vérifie.
- [ ] **AC3.** `expectations.tsv:112` et le README pointent vers **ce** ticket par
      son numéro, et plus vers une section inexistante.

**Deux notes de lecture, qui ne modifient aucune AC :**

- **AC1** est satisfait par le commentaire opérateur du 2026-10-01 (M0). R5 ajoute
  sa persistance dans le dépôt ; aucune mesure n'est refaite.
- **AC2** est satisfait par la **première** branche de son alternative (« le
  rattrapage partiel suffit »), donc sa seconde phrase — celle qui décrit le
  passage à `equal` — **n'est pas empruntée**. Elle est néanmoins inexacte (M3), et
  R7 écrit la procédure juste à sa place : la laisser telle quelle tendrait un
  piège au réveil. La correction porte sur la **prescription**, pas sur le critère.

---

## 7. Hors périmètre, délibérément

- **Aligner le canal sur `Strip`.** C'est la décision de ne pas le faire (§ 2).
  Le geste tient en un mot (M5) et la procédure est écrite (R7) ; ce qui manque
  est une population, pas un correctif.
- **Faire pointer `fences-quoted-callout.md` vers un plan existant** (M7) :
  changerait l'objet de la fixture sans rien acheter, et la rendrait fragile à la
  suppression d'un plan réel.
- **Étendre le détecteur de référence croisée au dépôt entier.** Aucune population
  mesurée hors de ce corpus ; un scan global est un autre ticket, dont la
  précondition est une mesure.
- **Les divergences n°2 et n°3** (`backtick-unterminated.md`,
  `backtick-late-close.md`) : deux autres exceptions déclarées du même TSV, chacune
  avec sa raison et son mécanisme de bornage, et aucune n'est l'objet d'AC3.
- **Refaire la mesure M0.** Elle est faite, datée et attribuée ; la refaire
  coûterait 1 261 corps de tickets pour reproduire un compte que personne ne
  contredit.
- **Rouvrir la question « `is_groomed` doit-il rester en `Strip` ? »** Le prédicat
  de promotion n'est pas touché, et sa politique est celle de mika#2120.

---

## 8. Ce que ce travail n'achète PAS

- **Il ne ferme pas la divergence.** Il la **tranche, la date, l'ancre à ses
  sites et la rend réveillable**. Le bash continuera de lire un callout cité dans
  un bloc, et c'est la décision.
- **Il ne surveille pas la condition de réveil.** Aucun mécanisme ne lance la
  commande de M0 : le seul instrument est l'entrée du registre, et **son silence
  ne prouve rien tant que personne ne relit le registre**. Un détecteur
  automatique est impossible ici — la condition demande `gh issue list`, donc du
  réseau, qu'aucun test unitaire ne doit prendre.
- **Il ne rattrape pas les deux occurrences mesurées** (#1204, #1144). Elles sont
  fermées, leurs corps restent tels quels, et rien ici ne les réécrit.
- **Le détecteur ne voit qu'un répertoire.** Une référence morte ailleurs — y
  compris une autre « voir § X » dans un autre README de fixtures — reste
  invisible, et c'est une limite nommée, pas une omission.
- **Il ne rend pas la fixture représentative du cas dangereux** (M7) : il dit
  qu'elle ne l'est pas.

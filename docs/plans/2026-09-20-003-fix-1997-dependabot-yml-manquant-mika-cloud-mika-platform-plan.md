---
issue: mika#1997
title: "dependabot.yml manquant sur mika-cloud et mika-platform — fermer R1 de mika#1729"
type: fix
date: 2026-09-20
---

# Plan — `dependabot.yml` sur mika-cloud et mika-platform (mika#1997)

## Contexte

mika#1729 a livré la chaîne de revue autonome des PR Dependabot
(Dependabot → mika-qa approve → mika-dev merge) sur les trois dépôts déjà
autonomes : `mika`, `mika-cloud`, `mika-platform`. Son **R1 / AC1** exigeait
`.github/dependabot.yml` sur **les trois**. PR#1995 (commit `a0598967`) n'en a
livré qu'un — celui de `mika` — et mika-qa l'a relevé au moment de la revue :
`AC1 ❌ — dependabot.yml only on mika, missing from mika-cloud + mika-platform`.
L'opérateur a mergé, et mika#1997 existe pour que le finding ne se perde pas.

### Ce que la lecture du code confirme, et ce qu'elle rectifie

Plan écrit contre des lectures vérifiées de l'arbre à `fc96a341`, pas contre les
suppositions du ticket. Quatre constats, dont deux rectifient le cadrage.

**C1 — le ticket a raison sur le périmètre restant : seul AC1 est ouvert.**
Tout le reste de mika#1729 est en place et vérifié dans cet arbre. Le flux
dep-review (détection `author == "dependabot[bot]"`, requête indépendante à la
GitHub Advisory Database, section obligatoire `DEP-REVIEW:`, fail-closed en
`hold[review]` sur échec de requête) vit dans
`skills/bundled/qa-review/system_prompt.md:142-174`. Le sous-type gatant
`block[dependency]` est câblé (`system_prompt.md:784,792,796`). Le chemin de
merge task-less borné à `dependabot[bot]` vit dans
`skills/bundled/self-dev-webhook-qa/system_prompt.md:40`. **Il ne manque que les
deux fichiers**, et la promesse du ticket est bien « couverture partielle ».

**C2 — le routage gateway est déjà acquis pour les trois dépôts.**
`INTERNAL_REPOS` (`crates/mika-gateway/src/github.rs:261-268`) contient
`senara-solutions/mika-cloud` et `senara-solutions/mika-platform`, et
`route_event("pull_request", "opened")` route vers mika-qa **sans filtre
d'auteur**. Donc dès qu'une PR Dependabot s'ouvre sur l'un des deux, la revue
part par le chemin qui existe déjà. Rien à ajouter côté gateway — c'est
exactement ce que le grounding de mika#1729 avait établi, et c'est toujours vrai.

**C3 — RECTIFICATION : le travail ne peut PAS être livré depuis ce worktree.**
C'est la contrainte structurelle que le texte du ticket ne pouvait pas montrer.
`dispatch-lib.sh:2233` dérive le worktree **par dépôt**
(`derive-worktree-path --branch "$BRANCH" --repo "$REPO"`), et le bac à sable
bwrap ne binde que ce worktree-là (`dispatch-lib.sh:691,790-795`) plus le
répertoire de logs et le canal de secret. **Un dispatch dev-pilot sur
`senara-solutions/mika` ne peut structurellement pas écrire
`.github/dependabot.yml` dans `senara-solutions/mika-cloud`.** Ce n'est pas une
permission à élargir, c'est la topologie du dispatch. Un plan qui dirait
« ajouter les deux fichiers » et partirait en dispatch produirait une PR qui ne
touche rien, ou pire, créerait les fichiers dans le mauvais dépôt.

**C4 — RECTIFICATION : « la revue se déclenche » et « la chaîne aboutit seule »
ne sont pas la même promesse, et elles divergent par dépôt.**
`classify_pr_files` (`crates/mika-agent/src/perimeter/rules.rs`) est
**agnostique au dépôt** — elle ne prend que des chemins — et sa liste
`MECHANICAL_EXACT` (`rules.rs:130-167`) ne contient, en fait de manifestes de
dépendances, que `Cargo.toml` / `Cargo.lock` (racine), `pyproject.toml` /
`uv.lock` et `LICENSE`. **`package.json`, `package-lock.json`, `Chart.yaml` et
`values.yaml` n'y sont pas.** Et
`perimeter::tests::mika_dependabot_workflow_bump_stays_decision_core`
(`tests.rs:424-433`) épingle comme **décision délibérée de mika#1729** que
`.github/workflows/*.yml` reste DECISION-CORE — « workflows carry secrets /
permissions surface ». D'où la table réelle :

| dépôt | écosystème | fichiers touchés | périmètre | issue |
|---|---|---|---|---|
| `mika` | cargo | `Cargo.toml`, `Cargo.lock` | MECHANICAL | auto-merge passe |
| `mika` | github-actions | `.github/workflows/*.yml` | DECISION-CORE | opérateur (par dessein) |
| `mika-cloud` | cargo *(si workspace Rust racine)* | idem | MECHANICAL | auto-merge passe |
| `mika-cloud` | github-actions | `.github/workflows/*.yml` | DECISION-CORE | opérateur |
| `mika-platform` | github-actions **seul** (R1) | `.github/workflows/*.yml` | DECISION-CORE | **opérateur, 100 %** |

**Conséquence à écrire avant la vérification, pas après :** avec l'écosystème
`github-actions` seul, **toutes** les PR Dependabot de `mika-platform` seront
routées à l'opérateur. La revue autonome s'y déclenchera bien — ce que le ticket
demande de vérifier — mais aucune ne se mergera seule. Sans cette phrase, la
sonde de la phase 3 lira une garde voulue comme un échec.

### Le volume, qui est le vrai risque de ce ticket

Le fichier de `mika` pose `open-pull-requests-limit: 5` sur chacun de ses deux
écosystèmes, avec `interval: "weekly"`, `day: "monday"`. Étendre tel quel aux
deux autres dépôts donne, dans le pire cas, **≤ 25 PR le même lundi matin**
(mika 10 + mika-cloud 10 + mika-platform 5). Or la capacité de mika-qa est
mesurée et tient dans une phrase de `CLAUDE.md` (§ mika#2347) : enveloppe de
600 s, exécution sérialisée par `agent_lock`, soit **~6 revues/heure**. Vingt-cinq
PR représentent plus de quatre heures de QA saturée, pendant lesquelles les PR
de la boucle autonome elle-même font la queue derrière. mika#2347 vient
précisément de descendre `MAX_PER_TICK` de 3 à 1 sur le réconciliateur de revue
pour cette raison arithmétique. Livrer les trois dépôts sur le même jour
rouvrirait la saturation par l'autre bout.

## Requirements

- **R1** — `.github/dependabot.yml` présent et actif sur
  `senara-solutions/mika-cloud` et `senara-solutions/mika-platform`, fermant
  l'AC1 / R1 de mika#1729. Livré **par le dépôt concerné** (voir NF1).
- **R2** — Les écosystèmes déclarés sont **vérifiés dans le dépôt cible**, jamais
  recopiés depuis `mika`. Présomption de départ (R1 de mika#1729) : `mika-cloud`
  = cargo + github-actions, `mika-platform` = github-actions seul. Toute
  divergence constatée dans l'arbre réel prime sur la présomption et est écrite
  dans le corps de la PR.
- **R3** — Le bloc `ignore` de `sqlite-vec` (mika#2142) n'est **pas** recopié
  sauf si le dépôt cible dépend réellement de `sqlite-vec`. « Aligné sur celui de
  `mika` » ne veut pas dire « identique octet pour octet » : ce bloc documente un
  `=`-pin propre au `Cargo.toml` de `mika`.
- **R4** — Les jours de planification sont **étalés** entre les trois dépôts pour
  borner la rafale hebdomadaire sous la capacité mesurée de mika-qa.
- **R5** — La vérification demandée par le ticket (« la review autonome se
  déclenche bien sur les trois ») est **exécutable et rejouable**, pas une
  mémoire d'opérateur, et elle distingue explicitement « la revue se déclenche »
  de « la chaîne aboutit seule » (C4).
- **R6** — La décision de topologie (écosystèmes par dépôt, étalement, asymétrie
  DECISION-CORE) est écrite là où elle se relit : dans `docs/`, dont `mika` est
  la source unique de vérité.

### Non-fonctionnels / contraintes

- **NF1 — un fichier, un dépôt, une PR.** Chaque `.github/dependabot.yml` doit
  atterrir à la racine `.github/` de **son** dépôt pour avoir le moindre effet.
  Aucun mécanisme de synchronisation cross-repo n'existe ici : le précédent
  invoqué par mika#1729 (« label-sync ») est en réalité **per-repo** — le
  workflow `.github/workflows/labels.yml` lit `.github/labels.yml` **du même
  dépôt**. Il n'y a pas de précédent de push cross-repo, et ce plan n'en crée pas.
- **NF2 — pas d'écriture par l'API Contents.** Écrire les fichiers via
  `gh api PUT /repos/.../contents/...` depuis ce worktree contournerait la PR et
  donc mika-qa, c'est-à-dire exactement la discipline que ce ticket sert. Sur
  `mika-platform`, ce serait en plus écrire sans revue dans le dépôt qui héberge
  la boucle. Refusé, et refusé en tant que tel — pas seulement « non permis ».
- **NF3 — pas de Path 2.** L'AMEND de Prime (2026-07-06) a ratifié Path 1
  (config de topologie + capacité de relecture) et **rejeté** Path 2 (auto-merge
  Dependabot). Aucun `dependabot.yml` livré ici ne porte de configuration
  d'auto-merge, et aucun workflow GitHub Actions d'auto-merge n'est ajouté. Le
  merge passe par la revue de mika-qa. Cette contrainte est déjà écrite en
  commentaire en tête du fichier de `mika` et doit l'être dans les deux autres.
- **NF4 — pas de nouveau job CI.** Un job dans `mika` qui affirmerait l'état de
  fichiers d'**autres** dépôts rougirait sur la moindre erreur `gh` transitoire
  et demanderait un token à portée cross-repo sur le chemin critique. La
  vérification de R5 est un script lancé à la main, et c'est dit.
- **NF5 — aucune modification du classifieur de périmètre.** L'asymétrie de C4
  est une décision épinglée par test (`mika_dependabot_workflow_bump_stays_decision_core`).
  Ajouter `.github/workflows/` ou `package.json` à `MECHANICAL_EXACT` pour
  « faire passer » les PR de `mika-platform` ouvrirait un contournement
  d'auto-merge pour des PR décisionnelles. Hors périmètre, et à refuser
  explicitement si la phase 3 en donne l'envie.

## Approche / Conception

### Décision A — la route de livraison : deux tickets frères, un par dépôt cible

Conséquence directe de C3 + NF1. `senara-solutions/mika-cloud` et
`senara-solutions/mika-platform` sont **tous deux dans `DISPATCHABLE_REPOS`**
(`crates/mika-agent/src/webhook_dispatch.rs:102-107`), donc un ticket ouvert sur
chacun peut être groomé et dispatché par la boucle dans le worktree de **son**
dépôt, sans machinerie nouvelle. C'est la seule route qui garde la livraison
dans la boucle et derrière une revue.

Trois routes envisagées, deux refusées :

| route | verdict |
|---|---|
| **A. Deux tickets frères** (un par dépôt cible) | **Retenue.** Chaque fichier atterrit par une vraie PR revue ; machinerie inchangée ; les deux dépôts sont déjà dispatchables. |
| B. Commit manuel de l'opérateur | **Échappatoire légitime**, pas la route nominale. Deux fichiers d'une quarantaine de lignes dont ce plan fixe le contenu : si l'opérateur préfère ce geste, la phase 2 se réduit à lui et le ticket se ferme sur la phase 3. À nommer, pas à cacher. |
| C. `gh api PUT /contents` depuis ce worktree | **Refusée** par NF2. Écrit sur `main` sans PR ni revue sur deux dépôts. |

mika#1997 devient donc un ticket de **décision + coordination + vérification**,
et non d'implémentation. Ce déplacement est le premier livrable du grooming :
sans lui, le dispatch produit une PR vide.

### Décision B — ce que la PR de mika#1997 contient réellement

Puisque les fichiers ne peuvent pas y vivre, la PR porte ce qui appartient
légitimement à `mika`, dont `docs/` est la source unique de vérité :

1. **`docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md`** —
   la décision de topologie : écosystèmes par dépôt et **pourquoi**, étalement
   des jours et son arithmétique, asymétrie DECISION-CORE de C4 et son caractère
   voulu, rappel NF3, et la règle « vérifier le manifeste dans le dépôt cible,
   ne pas recopier ». C'est ce document qui rend les deux tickets frères
   implémentables sans re-grooming.
2. **`scripts/verify-dependabot-topology.sh`** — R5 rendu exécutable. Lit les
   trois dépôts via `gh api .../contents/.github/dependabot.yml`, rapporte pour
   chacun : présence, écosystèmes déclarés, `day:` de planification,
   `open-pull-requests-limit`. Sortie lisible + code de sortie. Il **ne juge
   pas** la chaîne de revue — il établit la précondition dont l'absence était le
   défaut mesuré.
3. Les **corps des deux tickets frères**, verbatim, dans le document de décision,
   prêts à poser.

Le script est délibérément étroit : présence, écosystèmes, jour, limite. Il ne
sonde pas l'upstream, n'ouvre rien, n'écrit rien. C'est un lecteur, dans la même
famille que le lecteur de puits de mika#2267 et le smoke de mika#2407 — et pour
la même raison écrite là-bas : *un réglage qu'on ne peut pas observer n'est pas
un réglage, c'est un espoir.*

### Décision C — les écosystèmes, vérifiés et non recopiés

Présomption de départ, héritée de R1 de mika#1729 :

- **`mika-cloud`** : `cargo` (répertoire `/`) + `github-actions` (`/`).
- **`mika-platform`** : `github-actions` (`/`) **seul** — les sous-dépôts
  possèdent leurs propres écosystèmes Cargo/npm, et les dupliquer au niveau du
  méta-dépôt produirait des PR sur des manifestes qu'il ne contient pas.

**Cette présomption n'est pas vérifiable depuis ce worktree** : le bac à sable ne
binde que `mika` et `claude-pilot`, et ni `mika-cloud` ni `mika-platform` n'y
sont montés. Elle est donc portée comme une **hypothèse à confirmer dans l'arbre
cible** au moment de l'implémentation (phase 2), pas comme un fait. Le
vérificateur est une commande, pas une mémoire :

```bash
# dans le worktree du dépôt cible
ls Cargo.toml package.json pyproject.toml 2>/dev/null
ls .github/workflows/*.yml 2>/dev/null | head
```

Un manifeste présent que la présomption ignore (p. ex. un `package.json` racine
sur `mika-cloud`) est **déclaré** avec les autres, et la divergence est écrite
dans le corps de la PR. Un manifeste absent que la présomption suppose fait
tomber son écosystème : déclarer `cargo` sur un dépôt sans `Cargo.toml` produit
un écosystème muet, c'est-à-dire une couverture qui a l'air acquise et ne l'est
pas — la forme de panne que tout ce ticket existe pour fermer.

### Décision D — l'étalement des jours, arithmétique et non esthétique

`mika` reste **lundi** (inchangé, aucun fichier existant n'est touché) ;
`mika-cloud` prend **mardi** ; `mika-platform` prend **mercredi**. Le pire cas
par jour passe de ≤ 25 à ≤ 10 PR, soit ~1 h 40 de QA au lieu de ~4 h, sous la
capacité mesurée de ~6 revues/heure. L'étalement ne coûte rien : la planification
Dependabot est par fichier, donc par dépôt.

`open-pull-requests-limit: 5` et le groupement `minor`/`patch` sont conservés à
l'identique — ils bornent déjà le volume et sont la moitié de la raison pour
laquelle le fichier de `mika` a la forme qu'il a.

### Décision E — le bloc `sqlite-vec` ne voyage pas

Le bloc `ignore` de `mika` (mika#2142) documente un défaut d'un crate précis,
mesuré, dont le pin `=` vit dans le `Cargo.toml` de `mika` depuis son commit
initial. Il n'est recopié dans un dépôt cible **que si** ce dépôt dépend
réellement de `sqlite-vec` (`grep sqlite-vec Cargo.toml`). Recopier une
justification de quarante lignes qui ne s'applique pas produit un commentaire
faux dans un fichier de configuration — et la condition de réveil qu'il porte
(« quand une release ships le fichier manquant ») deviendrait la consigne de
quelqu'un qui n'a rien à réveiller.

## Phases d'implémentation

**Phase 1 — décision et outillage, dans `mika` (c'est la PR de ce ticket).**
- Écrire `docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md` :
  table de topologie (dépôt × écosystème × périmètre × issue), étalement et son
  arithmétique, asymétrie DECISION-CORE avec sa citation du test qui l'épingle,
  rappel NF3, règle « vérifier dans le dépôt cible », et les deux corps de
  tickets frères verbatim.
- Écrire `scripts/verify-dependabot-topology.sh` (présence, écosystèmes, `day`,
  `open-pull-requests-limit` sur les trois dépôts ; exit 0 = les trois portent un
  fichier lisible, 1 = au moins un manque ou est illisible, 2 = rien n'a pu être
  vérifié — token absent, `gh` indisponible). Le troisième code n'est **pas** un
  succès : une vérification qui n'a pas pu s'authentifier n'a rien vérifié
  (forme reprise de `scripts/smoke-search-substrate`, mika#2407).
- Rendre le script exécutable et le référencer depuis le document de décision.

**Phase 2 — livraison des deux fichiers, hors de ce dépôt.**
Route nominale (décision A) : poser les deux tickets frères et les laisser passer
par la boucle. Route opérateur (B) : commit manuel des deux fichiers depuis le
document de décision. Dans les deux cas, chaque fichier :
- est vérifié contre les manifestes réellement présents (décision C) ;
- porte l'en-tête de commentaire rappelant NF3 (pas d'auto-merge) et le lien vers
  mika#1729 et ce plan, sur le modèle de celui de `mika` ;
- porte son `day:` propre (décision D) ;
- ne porte le bloc `sqlite-vec` que si la condition de la décision E est remplie.

**Phase 3 — vérification, et elle est en deux moitiés qu'il ne faut pas
confondre.**
- *Précondition* : `scripts/verify-dependabot-topology.sh` rend `0` et liste les
  trois dépôts avec leurs écosystèmes et leurs jours distincts.
- *La revue se déclenche* : sur la première PR Dependabot réelle de chacun des
  deux nouveaux dépôts, une revue `mika-platform-qa` est postée, portant la
  section `DEP-REVIEW:` avec sa citation de requête advisory. C'est la promesse
  du ticket, et elle vaut pour les trois.
- *La chaîne aboutit seule* : **attendue sur `mika` et `mika-cloud` pour les
  bumps cargo racine uniquement**. Une PR `github-actions`, sur les trois dépôts,
  est routée à l'opérateur — c'est C4, c'est voulu, et ce n'est pas un échec de
  vérification. Sur `mika-platform`, qui ne déclare que `github-actions`, cela
  signifie **zéro merge autonome**, et c'est la lecture correcte.

**Halte 1 — la revue ne se déclenche pas sur un nouveau dépôt.**
Ne pas toucher au `dependabot.yml` par réflexe. Vérifier d'abord que le dépôt est
bien dans `INTERNAL_REPOS` (il l'est, `github.rs:261-268`) puis que l'événement
`pull_request.opened` est **arrivé** — la perte d'un `opened` est une classe
connue et instrumentée (file bornée mika#1870 → 429 → circuit breaker → DLQ), et
le rattrapage est `qa_review_reconcile` (mika#2334), pas ce fichier.

**Halte 2 — une PR Dependabot reste ouverte sans merge sur `mika-cloud`.**
Lire d'abord les fichiers qu'elle touche. Si elle touche `.github/workflows/` ou
un manifeste hors `MECHANICAL_EXACT`, c'est C4 et c'est nominal. **Ne pas élargir
`MECHANICAL_EXACT`** (NF5) : ce serait ouvrir un contournement d'auto-merge pour
des PR décisionnelles afin de réparer un symptôme qui n'en est pas un.

**Halte 3 — le volume déborde malgré l'étalement.**
Le levier est `open-pull-requests-limit`, par dépôt et par écosystème, dans le
fichier concerné. Ce n'est pas un réglage de mika-qa, et ce n'est pas une raison
de désarmer le réconciliateur de revue.

## Contrat de vérification

- `cargo build` / `cargo clippy` / `cargo fmt --check` propres — la PR de phase 1
  ne touche aucun code Rust, donc ces trois-là doivent rester exactement dans
  l'état de `main`.
- `bash -n scripts/verify-dependabot-topology.sh` — syntaxe valide.
- `scripts/verify-dependabot-topology.sh` lancé à la main **avant** la phase 2
  rend `1` en nommant les deux dépôts manquants ; lancé **après**, rend `0`.
  C'est le contrôle négatif : un vérificateur qui rend `0` des deux côtés du
  correctif n'atteste rien.
- Le script rend `2` — et pas `0` — quand `gh` n'est pas authentifié. À tester en
  vidant `GH_TOKEN` dans un sous-shell.
- Le document de décision cite, avec leurs chemins, les trois faits qui portent
  le plan : `webhook_dispatch.rs:102-107` (dispatchabilité des deux cibles),
  `github.rs:261-268` (routage acquis), `perimeter/tests.rs:424-433` (asymétrie
  DECISION-CORE voulue).
- Relecture manuelle : aucune configuration d'auto-merge n'apparaît dans les
  fichiers ni dans les corps de tickets proposés (NF3).
- `docs/` modifié ⇒ `scripts/sync-agent-docs.sh` lancé si la synchro l'exige, et
  le job `docs-sync` de `ci.yml` vert.

## Definition of Done

- La topologie Dependabot des trois dépôts est écrite dans `docs/`, avec le
  pourquoi de chaque écosystème, l'étalement et son arithmétique, et l'asymétrie
  DECISION-CORE nommée comme voulue.
- `scripts/verify-dependabot-topology.sh` existe, est exécutable, et sépare les
  trois états « les trois y sont » / « il en manque » / « rien n'a été vérifié ».
- Les deux corps de tickets frères sont rédigés et prêts à poser, avec leurs
  écosystèmes, leur jour et leur contrainte NF3.
- `.github/dependabot.yml` existe sur `senara-solutions/mika-cloud` et
  `senara-solutions/mika-platform`, avec des écosystèmes vérifiés contre les
  manifestes réels et des jours distincts de celui de `mika` (phase 2, hors de
  cette PR — c'est la condition de clôture du ticket, pas de la PR).
- Sur la première PR Dependabot réelle de chacun des deux nouveaux dépôts, une
  revue `mika-platform-qa` portant une section `DEP-REVIEW:` est postée.
- Aucune ligne de `crates/mika-agent/src/perimeter/` n'est modifiée.
- R1 / AC1 de mika#1729 est fermé pour les trois dépôts, et le finding de mika-qa
  qui a motivé mika#1997 est répondu par une vérification rejouable plutôt que
  par une affirmation.

## Acceptance criteria

Le corps de mika#1997 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du contrat de vérification.

- **AC1** — `.github/dependabot.yml` est présent et actif sur
  `senara-solutions/mika-cloud` et `senara-solutions/mika-platform`, fermant R1 /
  AC1 de mika#1729 pour les trois dépôts.
- **AC2** — Les écosystèmes déclarés dans chaque fichier correspondent aux
  manifestes réellement présents dans le dépôt cible, vérifiés et non recopiés
  depuis `mika`. Toute divergence avec la présomption de R1 de mika#1729 est
  écrite dans le corps de la PR correspondante.
- **AC3** — Les trois dépôts ont des jours de planification **distincts**,
  bornant la rafale hebdomadaire au pire cas par jour sous la capacité mesurée de
  mika-qa. `mika` reste lundi et son fichier n'est pas modifié.
- **AC4** — Aucun des fichiers livrés ne porte de configuration d'auto-merge, et
  aucun workflow d'auto-merge n'est ajouté (NF3, AMEND de Prime 2026-07-06).
- **AC5** — Le bloc `ignore` de `sqlite-vec` n'apparaît dans un fichier cible que
  si ce dépôt dépend réellement de `sqlite-vec`.
- **AC6** — `scripts/verify-dependabot-topology.sh` existe, rapporte pour chacun
  des trois dépôts la présence du fichier, ses écosystèmes, son jour et sa
  limite de PR, et distingue par son code de sortie « les trois y sont » (0),
  « il en manque » (1) et « rien n'a pu être vérifié » (2).
- **AC7** — Le document de décision existe dans `docs/`, nomme l'asymétrie
  DECISION-CORE de C4 comme **voulue** et non comme un défaut, et cite les
  chemins des trois faits qui portent le plan.
- **AC8** — Sur la première PR Dependabot réelle de chacun des deux nouveaux
  dépôts, une revue `mika-platform-qa` est postée avec une section `DEP-REVIEW:`
  portant sa citation de requête advisory — c'est-à-dire que « la review autonome
  se déclenche bien sur les trois » est établi par observation.
- **AC9** — Aucun fichier sous `crates/mika-agent/src/perimeter/` n'est modifié :
  l'asymétrie DECISION-CORE n'est pas « réparée » (NF5).

### Note de réconciliation (grooming)

- Le ticket dit « ajouter `.github/dependabot.yml` sur mika-cloud et
  mika-platform ». C'est exact sur le **quoi** et muet sur le **où** : le
  dispatch est per-dépôt (C3), donc ce travail ne peut pas être livré depuis le
  worktree de `mika`. La décision A en tire la route ; sans elle, un dispatch
  produirait une PR vide.
- Le ticket dit « aligné sur celui de `mika` ». Pris au pied de la lettre, cela
  recopierait le bloc `sqlite-vec` (décision E) et le `day: monday` (décision D),
  dont le second rouvrirait la saturation de mika-qa que mika#2347 vient de
  border. « Aligné » vaut pour la **forme** — hebdomadaire, groupé, limite 5, pas
  d'auto-merge — pas pour les octets.
- Le ticket dit « vérifier que la review autonome se déclenche bien sur les
  trois ». C'est vérifiable et ce sera vrai. Mais sur `mika-platform`, qui ne
  déclare que `github-actions`, **aucune** PR ne se mergera seule (C4) : la
  garde est voulue et épinglée par test. La phase 3 sépare les deux moitiés pour
  que cette garde ne se lise pas comme un échec.
- L'opérateur a qualifié le ticket de « simple, aucune décision de fond », ce qui
  est juste vu de son texte : deux fichiers YAML d'une quarantaine de lignes. Le
  contenu **est** simple et ce plan le fixe intégralement. Ce qui ne l'était pas,
  et que le texte ne pouvait pas montrer, c'est la route de livraison.

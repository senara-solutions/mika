---
issue: mika#2120
title: Le lecteur doit tolérer ce que son écrivain produit légitimement - Plan
type: fix
scope_repo: mika
priority: p1-important
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Le lecteur doit tolérer ce que son écrivain produit légitimement - Plan

## Goal Capsule

**Objectif.** Un ticket groomé selon la lettre de la spec doit être **vu** par
l'alimenteur. Aujourd'hui le prédicat exige `` `docs/plans/ `` et la spec écrit
`` `<repo>/docs/plans/ `` : la boucle s'arrête précisément quand le grooming
réussit.

**Moyens.** Un matcher unique, **ancré** et permissif au préfixe de dépôt,
partagé par les deux lecteurs aveugles ; une forme canonique unique côté
écrivains ; et l'exclusion des blocs de code cités, sans laquelle un ticket qui
*parle* du garde satisfait le garde.

**Hiérarchie d'autorité.** ACs du ticket (corps + AC6 transcrit du commentaire
du 2026-09-01 18:13Z) > ce plan > jugement de l'implémenteur.

**Conditions d'arrêt.**
- S'arrêter si le prédicat devient permissif **sans** rester ancré et sans
  exclure les blocs de code. Élargir un prédicat non ancré élargit sa surface de
  faux positif ; le coût d'un faux positif est un créneau de dispatch mort à
  `_find_issue_plan returned empty`, c'est-à-dire au pire endroit.
- S'arrêter si le correctif ne touche qu'`is_groomed`. Il existe un **second**
  lecteur aveugle (`dispatch-lib.sh:4405`) : ne corriger que le premier déplace
  la mort en aval au lieu de la supprimer.
- S'arrêter si les fixtures d'AC3 sont refetchées en direct. Quatre des six
  tickets nommés ont été corrigés à la main depuis la mesure ; un test bâti sur
  le monde réparé n'atteste rien.

**Profil d'exécution.** Deux dépôts. **Primaire `mika`** :
`crates/mika-agent/src/auto_pull.rs` et
`skills/bundled/_shared/dispatch-lib.sh` (+ son harnais de test).
**Compagnon `mika-platform`** : `.claude/commands/*.md`, la moitié « écrivain »
d'AC4 — un pilote lancé sur `mika` ne peut pas l'atteindre (voir § AC4).

**Tail ownership.** PR sur `mika`, routée vers mika-qa ; PR compagnon sur
`mika-platform`, référence croisée dans les deux corps.

## Product Contract

### Résumé

`is_groomed()` exige le chemin de plan **sans** préfixe de dépôt ; la spec de
grooming en prescrit un. On rend la **détection** aussi permissive que
l'écrivain légitime, on garde la **décision** stricte, on ancre les trois
prédicats et on ignore les blocs de code, puis on fixe une forme unique côté
écrivains.

### La carte des lecteurs, mesurée

Le ticket nomme un lecteur aveugle. Il y en a **deux**, et trois qui vont bien.
Relevé à `origin/main = 50d969a7` :

| lecteur | prédicat | forme préfixée |
|---|---|---|
| `auto_pull.rs:313` `is_groomed` | `contains("> - **Plan:** \`docs/plans/")` | **aveugle** ← le p1 du ticket |
| `dispatch-lib.sh:4405` extraction `PLAN_PATH` | `grep -oP '> - \*\*Plan:\*\* \`\Kdocs/plans/[^\`]+'` | **aveugle** → rend vide → `return 0` silencieux |
| `executor.rs:977` `check_grooming_markers` | `contains("docs/plans/")` | tolérant (sous-chaîne) |
| `dispatch-lib.sh:3851` idempotence du writer | `grep -cF 'docs/plans/'` | tolérant |
| `dispatch-lib.sh:1227` porte de dispatch | essaie `$plan_path` **et** `${plan_path#repo/}` | tolérant, **explicitement** |

**Et le chemin de plan n'est pas le seul axe où `is_groomed` est plus étroit que
ses voisins.** Le verdict de grooming existe en **trois** formes reconnues par
`executor.rs` (`:946`, `:949`, `:968`) :

| forme | `executor.rs` | `dispatch-lib.sh:3865` | `auto_pull::is_groomed` |
|---|---|---|---|
| `second-pass (GROOMED…` | oui | oui | **oui** |
| `second-pass (READY, paraphrased GROOMED` | oui | oui | **non** |
| `first-pass (READY, single-pass GROOMED` (mika#2012) | oui | oui | **non** |

`dispatch-lib.sh:3865` porte l'instruction en toutes lettres — « This pattern
MUST stay in lockstep with executor.rs's three verdict regexes » — et l'honore.
`is_groomed` ne connaît que la première. **Un grooming qui converge en un seul
passage produit donc un ticket que l'alimenteur ne voit pas**, exactement comme
le préfixe de dépôt.

Ce n'est pas une hypothèse : **ce ticket-ci en est le cas.** Son grooming a
reçu `Disposition: READY` au premier passage, son callout porte donc la forme
canonique `first-pass (READY, single-pass GROOMED)` — véridique, acceptée par la
porte de dispatch, et **invisible à l'alimenteur**. Le p1 qui décrit
l'invisibilité est lui-même invisible, par une seconde voie que le corps du
ticket ne nommait pas. (Il reste dispatchable : le label `ready` passe par
`executor.rs::check_grooming_markers`, qui accepte les trois formes.)

Trois choses en découlent :

1. **Le dépôt sait déjà que les deux formes existent.** Le commentaire de
   `dispatch-lib.sh:1224` le dit mot pour mot : « The callout carries two
   historical shapes: repo-prefixed (`mika/docs/plans/…`) and repo-relative
   (`docs/plans/…`). Try both ». Un lecteur qui n'en accepte qu'une n'applique
   pas une règle, il en ignore une déjà écrite ailleurs.
2. **`is_groomed` est étroit sur deux axes, pas un.** Le corps du ticket a
   trouvé le premier (le chemin) ; le second (le verdict) a la même forme et la
   même cause — un lecteur plus strict que ses écrivains légitimes — et se
   corrige dans la même fonction, au même endroit. Les traiter séparément
   ferait rejouer le p1 au premier grooming convergent.
3. **Corriger `is_groomed` seul déplace la panne.** Un ticket à callout préfixé
   passerait alors la promotion, puis `PLAN_PATH` rendrait vide en `:4405` et le
   chemin `/ce-work` serait sauté sans un mot. C'est exactement le « il mourrait
   plus loin, après avoir consommé un créneau » du commentaire du 18:13Z. La
   phase 3 est donc dans le périmètre du ticket, pas à côté.

### Dérive de prémisse : quatre des six tickets ont été réparés à la main

Relevé le 2026-09-01 sur les six tickets nommés :

| ticket | forme actuelle du callout |
|---|---|
| #1680, #1694, #1699, #1934 | `docs/plans/…` — **corrigée depuis la mesure** |
| #1947, #1949 | `mika/docs/plans/…` — **encore préfixée** |

Le corps du ticket n'a pas tort : il mesurait le 2026-08-31, et la réparation
manuelle est intervenue après. Mais AC3 dit « les six rendent `is_groomed = true`
après le correctif ». Fetchées en direct aujourd'hui, **quatre passeraient déjà
sans le correctif** : le test serait vert des deux côtés et n'attesterait rien.

**Conséquence pour la phase 4 :** les fixtures sont **figées dans le fichier de
test**, jamais refetchées, et portent toutes la forme **préfixée**. Deux d'entre
elles (#1947, #1949) sont les corps réels verbatim ; les quatre autres sont
reconstruites en re-préfixant leur chemin actuel, et le test dit lequel est
lequel. C'est la seule construction où AC3 échoue sur `main` et passe après.

La réparation manuelle est elle-même une pièce du dossier : quatre tickets et,
le 2026-09-01, trois de plus (#2134, #2126, #2118) ont dû être recallés à la
main. La classe se rejoue plus vite qu'on ne la répare à la main.

### Le faux positif que l'anneau ouvre — et pourquoi le repli penche vers la permissivité

Le commentaire du 18:13Z le démontre sur le ticket lui-même : deux des trois
prédicats sont des `contains` **non ancrés**, donc satisfaits par n'importe
quelle occurrence dans le corps, **bloc de code cité compris**. #2120 ne
échappe au faux positif que grâce à la regex ancrée du troisième.

Élargir un prédicat non ancré élargit sa surface de faux positif. D'où AC6
(ancrage des trois) et, au-delà de ce que l'ancrage seul peut faire :
**l'ancrage ne suffit pas.** Une ligne citée dans un bloc `` ``` `` commence
elle aussi en colonne zéro — c'est le cas de ce ticket-ci, qui cite le gabarit
de l'étape 19. Le test qu'AC6 demande (« le motif uniquement dans un bloc de
code rend `false` ») ne peut passer que si les blocs clôturés sont **retirés du
corps avant évaluation**. C'est ce que fait la phase 1.

**Repli sur fence non fermée : ne pas retirer.** Un corps dont une fence
n'est jamais refermée est ambigu. Les deux erreurs n'ont pas le même prix : un
faux positif coûte **un créneau de dispatch**, un faux négatif coûte **la
boucle** — quinze heures mesurées, c'est le ticket. Et la doctrine citée par le
ticket tranche dans le même sens : la *détection* doit être au moins aussi
permissive que le consommateur, la *décision* reste stricte
(`guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`).
Donc : fence non fermée → on n'ampute rien, on évalue le corps entier.

### Forme canonique retenue : `docs/plans/<file>`, sans préfixe

Le préfixe est **faux**, pas seulement redondant. `dispatch-lib.sh:4413`
résout le chemin en `"$WORKTREE_DIR/$PLAN_PATH"`, et `$WORKTREE_DIR` est déjà la
racine du sous-dépôt : `mika/docs/plans/x.md` y désigne `…/mika/mika/docs/…`,
qui n'existe pas. Le callout vit sur l'issue du dépôt concerné ; le dépôt est
donné par l'issue, pas par le chemin.

Les lecteurs restent permissifs **pour l'historique**, pas pour légitimer les
deux formes : détection permissive, écriture unique.

### Extension de périmètre — revendiquée : l'axe du verdict

Aucun AC ne nomme le verdict ; AC1 à AC3 portent sur le chemin de plan. Je la
retiens et je le dis, plutôt que de la faire passer pour une conséquence des ACs.

Trois appuis. **Même défaut, même fonction, même ligne** : `is_groomed` a trois
prédicats, deux sont trop étroits, et les deux se corrigent dans le même
`OnceLock`. **L'instruction existe déjà et n'est pas honorée** : `dispatch-lib.sh:3865`
prescrit le lockstep avec les trois regex d'`executor.rs`, et
`is_groomed` est le seul des trois lecteurs à ne pas le tenir. **Le coût de
l'omission est mesuré sur ce ticket même** : livrer le correctif du chemin sans
celui du verdict rendrait visible un ticket groomé en deux passages et laisserait
invisible tout ticket groomé en un — c'est-à-dire rouvrirait le p1 par la porte
d'à côté, au premier grooming convergent.

**Bornes :** on aligne `is_groomed` sur les trois formes déjà reconnues par
`executor.rs`. On n'invente aucune forme, on n'en retire aucune, et on ne touche
ni à `executor.rs` ni à `dispatch-lib.sh:3865` — qui sont, sur cet axe, la
référence à rejoindre.

### Hors périmètre

- **Le seuil `MIKA_AUTO_FEEDER_MIN_READY`** (défaut 3). Le corps l'exclut : le
  déficit était de un et six candidats attendaient.
- **Les 306 `callback_delivered_without_pr_url`** et l'arrêt des reprises sur
  #1651 / #1403. Second coin, explicitement séparé par le corps du ticket ; il
  porte sur ce qui se passe *après* la création d'une tâche.
- **`PlanOwnership` / `_find_issue_plan`** (mika#2020, mika#1617). Ils
  répondent à « ce plan appartient-il à ce ticket », question distincte de « ce
  callout a-t-il la bonne forme ». Non touchés.
- **Re-corriger à la main les callouts encore préfixés** (#1947, #1949). Une
  fois les lecteurs permissifs, ils sont vus tels quels ; les réécrire serait
  masquer la preuve que le correctif fonctionne.

## Acceptance criteria

Transcrits depuis le corps de mika#2120 (AC1-AC5) et depuis le commentaire de
l'opérateur du 2026-09-01 18:13Z, § « Suggestion d'AC additionnel » (AC6),
chacun avec l'unité d'implémentation qui le satisfait et l'artefact qui le
prouve.

**AC1** — `is_groomed()` rend `true` pour un callout
`` > - **Plan:** `docs/plans/x.md` `` **et** pour
`` > - **Plan:** `mika/docs/plans/x.md` ``. Tests unitaires sur les deux formes.
→ *Unité :* Phase 1.2 (`PLAN_CALLOUT_RE`, segment de dépôt optionnel) branchée
en Phase 1.4.
→ *Preuve :* tests 4.1 et 4.2.

**AC2** — Contrôle négatif : `is_groomed()` rend **`false`** pour un chemin qui
n'est pas un plan (`` `docs/brainstorms/x.md` ``, `` `mika/docs/solutions/x.md` ``,
`` `../docs/plans/x.md` ``).
→ *Unité :* Phase 1.2 — le segment optionnel est `[A-Za-z0-9_-][A-Za-z0-9._-]*`,
dont le **premier caractère ne peut pas être un point**, ce qui exclut `../` et
`./` ; et le littéral `docs/plans/` exclut `docs/brainstorms/` et
`docs/solutions/`. Un seul segment est autorisé, donc `a/b/docs/plans/` échoue.
→ *Preuve :* test 4.3, un cas par forme refusée, **plus** `..` explicitement —
c'est le cas que la classe de caractères naïve `[A-Za-z0-9._-]+` laisserait
passer.

**AC3** — Test de régression sur les corps réels des six tickets (#1680, #1694,
#1699, #1934, #1947, #1949), fixtures figées : les six rendent
`is_groomed = true` après le correctif.
→ *Unité :* Phases 1.2 et 1.4.
→ *Preuve :* test 4.4, fixtures **figées dans le fichier**, toutes en forme
préfixée (#1947 et #1949 verbatim ; les quatre autres re-préfixées, avec la
raison écrite en commentaire — voir § Dérive de prémisse). Sans cette
construction, quatre des six passeraient déjà sur `main`.

**AC4** — La spec `.claude/commands/mika-groom-ticket.md` étape 19 nomme une
seule forme, et tout autre producteur de ce callout écrit la même. Recherche
exhaustive des sites producteurs.
→ *Unité :* Phase 5, dans le dépôt **`mika-platform`** (les commandes n'existent
pas dans `mika`). Sites relevés par recherche exhaustive :
`.claude/commands/mika-groom-ticket.md:224` et `:231` (les deux formes du
callout), et `.claude/commands/mika-groom-milestone.md:227` et `:240`
(`Sequencing record`, même préfixe, même défaut). `/mika-groom-plan-only` **n'écrit
pas ce callout** — `dispatch-lib.sh` le fait pour lui (`:3877`), et il écrit déjà
`${plan_relpath}` sans préfixe : conforme, aucun changement.
→ *Preuve :* revue de diff sur la PR compagnon + le `grep` de non-régression de
la phase 5.3.

**AC5** — Après correctif, un tir d'`auto_pull` avec un bassin sous le plancher
et au moins un candidat groomé promeut effectivement, et l'audit porte
`auto_feeder` / promotion, pas `auto_feeder_no_backlog`.
→ *Unité :* conséquence des phases 1 et 3 ; aucune logique de promotion n'est
modifiée.
→ *Preuve :* test 4.6 — `select_feeder_candidates` sur un jeu d'issues dont le
seul éligible porte un callout préfixé : il est retenu. Plus la preuve terrain
déjà au dossier : le 2026-09-01, #2134 recallé à la main a été promu `ready`
**dans les trois minutes** (tâche `ready-label` à `18:10:05Z`), ce qui ferme la
chaîne causale sans attendre un nouveau tir.

**AC6** *(transcrit du commentaire du 2026-09-01 18:13Z)* — Les trois prédicats
sont ancrés en début de ligne comme l'est déjà la regex de `Grooming history`,
et un test unitaire vérifie qu'un corps contenant le motif **uniquement à
l'intérieur d'un bloc de code** rend `false`.
→ *Unité :* Phase 1.1 (retrait des blocs clôturés avant évaluation) et Phase 1.3
(les trois prédicats ancrés `(?m)^`).
→ *Preuve :* tests 4.5a (les trois motifs présents **seulement** dans une fence
→ `false`) et 4.5b (fence non fermée → le corps entier est évalué, repli
permissif documenté).

## Fire-Disposition

Ce plan livre des **détecteurs** : les tests de la phase 4. Deux d'entre eux
gardent un contrat que le correctif ne doit pas casser — 4.3 (le contrôle
négatif d'AC2) et 4.5 (la non-détection dans un bloc de code). Par le
Fire-Disposition Gate (mika#1574), la disposition se déclare contre le schéma
canonique : **(a) exception nommée**, **(b) livré désactivé**,
**(c) halte-et-remontée**.

**Le tir au déploiement est structurellement impossible.** Aucun de ces
détecteurs ne balaie l'arbre existant : chacun s'exerce sur une fonction que
cette PR modifie (`is_groomed`, `_extract_plan_path`) et sur des fixtures
littérales du fichier de test. Il n'existe donc pas de classe « violation
préexistante ailleurs » capable de faire échouer une PR sans rapport.

- **4.1, 4.2, 4.4, 4.6 (comportement neuf) → (c) halte-et-remontée.** Un tir
  prouve que le correctif ne voit pas ce qu'il promet de voir. On corrige le
  code, jamais le test.
- **4.3 (contrôle négatif, AC2) → (c) halte-et-remontée, sans exception.** Un
  tir signifie que le prédicat rendu permissif accepte un chemin qui n'est pas un
  plan — c'est-à-dire la porte que le ticket demande explicitement de ne pas
  ouvrir. Offrir une allowlist ici viderait AC2 de son sens.
- **4.5 (bloc de code, AC6) → (c) halte-et-remontée.** Un tir signifie que le
  garde est de nouveau piégeable par un ticket qui parle du garde.
- **4.7b (contrôle négatif du verdict) → (c) halte-et-remontée, sans
  exception.** Un tir signifie que l'alternance élargie accepte un verdict qui
  n'en est pas un — la porte que l'extension ne doit pas ouvrir.
- **3.3 (parité shell) → (c) halte-et-remontée.** Un tir signifie que les deux
  lecteurs ont divergé — la cause racine même de ce ticket, rejouée.

**Aucun détecteur n'est livré désactivé (b) ni ne porte d'allowlist (a) :** leur
domaine est le diff de cette PR, donc un tir désigne toujours un défaut du
correctif, jamais un héritage.

## Phases

### Phase 1 — `is_groomed` : ancrer, ignorer les fences, tolérer le préfixe

Fichier : `crates/mika-agent/src/auto_pull.rs`.

**1.1 — Retirer les blocs clôturés avant d'évaluer.**

```rust
/// Rend le corps privé de ses blocs clôturés (```` ``` ```` et `~~~`), pour que
/// le gabarit d'un callout **cité** dans un ticket ne satisfasse pas le garde
/// qui le lit. mika#2120 en est la démonstration : ce ticket cite l'étape 19 de
/// la spec, en colonne zéro — l'ancrage seul ne l'exclut pas.
///
/// **Repli sur fence non fermée : ne rien retirer.** L'asymétrie des coûts le
/// commande — un faux positif coûte un créneau de dispatch, un faux négatif a
/// coûté quinze heures de boucle. Voir
/// `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`.
fn strip_fenced_blocks(body: &str) -> Cow<'_, str>
```

Une fence ouvrante est une ligne dont le premier caractère non blanc est ``` ``` ```
ou `~~~` ; elle se ferme sur le **même marqueur**. Une fence ouverte par ``` ``` ```
n'est pas fermée par `~~~`.

**1.2 — Le matcher de callout de plan, unique et documenté :**

```rust
// Un seul segment de dépôt optionnel, dont le premier caractère n'est pas un
// point : `mika/docs/plans/…` passe, `../docs/plans/…` et `./docs/plans/…` non
// (AC2). Le littéral `docs/plans/` exclut `docs/brainstorms/` et
// `docs/solutions/`. Un seul segment : `a/b/docs/plans/` échoue.
r"(?m)^> - \*\*Plan:\*\* `(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+`"
```

**1.3 — Ancrer les deux prédicats restants** en `(?m)^> - \*\*Branch:\*\* ` et
en réutilisant 1.2 pour le plan, comme l'est déjà `GROOMING_HISTORY_RE` (AC6).
Toutes les regex derrière un `OnceLock`, comme l'existante.

**1.5 — Aligner le verdict sur les trois formes** (extension revendiquée,
§ ci-dessus). `GROOMING_HISTORY_RE` garde son ancrage `(?m)^> - \*\*Grooming
history:\*\*.+` et son alternance couvre les trois queues reconnues par
`executor.rs` :
`second-pass \(GROOMED[\s\)\.,;:—-]`, `second-pass \(READY, paraphrased GROOMED`,
`first-pass \(READY, single-pass GROOMED`. Les trois littéraux sont **copiés
depuis `executor.rs:946/949/968`**, pas retranscrits de mémoire : c'est la seule
manière de ne pas rouvrir la divergence en la corrigeant. Un commentaire renvoie
au lockstep de `dispatch-lib.sh:3865`.

**1.4 — Recomposer `is_groomed`** : `strip_fenced_blocks` une fois, puis les
trois `is_match` sur la vue nettoyée. La doc de la fonction gagne les deux
formes acceptées et la raison (mika#2120).

### Phase 2 — Vérifier qu'aucun autre lecteur Rust ne régresse

Lecture, pas écriture. `executor.rs:977` `check_grooming_markers` utilise
`contains("docs/plans/")` : déjà tolérant, **on n'y touche pas** — le resserrer
créerait le défaut symétrique. Le noter dans la doc de `is_groomed` pour qu'un
futur lecteur ne « harmonise » pas les deux dans le mauvais sens.

### Phase 3 — Le second lecteur aveugle : `dispatch-lib.sh`

**3.1** Extraire le motif de `:4405` dans une fonction shell nommée
`_extract_plan_path`, avec le **même** segment optionnel qu'en 1.2 :
`> - \*\*Plan:\*\* \`\K(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^\`]+`.

**3.2** Le chemin extrait est résolu en `"$WORKTREE_DIR/$PLAN_PATH"`, où
`$WORKTREE_DIR` est déjà la racine du sous-dépôt : un chemin préfixé doit donc
être **normalisé** (retrait du premier segment) avant d'être testé par `-f`,
exactement comme la porte de dispatch le fait déjà en `:1227`. Sans cette
normalisation, accepter le préfixe ne ferait que déplacer l'échec d'un `grep`
vide à un `-f` faux.

**3.3** Test de parité dans `skills/bundled/_shared/test-dispatch-lib.sh` :
les deux formes rendent le même chemin normalisé. Le harnais porte déjà une
fixture préfixée (`prefixed_body`, `:1001`) pour une autre fonction — même
forme, voisine immédiate.

### Phase 4 — Les preuves (Rust)

Dans le module `tests` de `auto_pull.rs`.

- **4.1 `is_groomed_accepte_forme_nue` (AC1).**
- **4.2 `is_groomed_accepte_prefixe_de_depot` (AC1).**
- **4.3 `is_groomed_refuse_chemins_non_plan` (AC2).** Un cas par forme :
  `docs/brainstorms/x.md`, `mika/docs/solutions/x.md`, `../docs/plans/x.md`,
  `./docs/plans/x.md`, `a/b/docs/plans/x.md`. Le cas `..` est celui qu'une
  classe naïve laisserait passer ; il est nommé comme tel en commentaire.
- **4.4 `is_groomed_sur_les_six_corps_reels` (AC3).** Fixtures **figées**, toutes
  préfixées ; commentaire distinguant les deux corps verbatim (#1947, #1949) des
  quatre reconstruits, avec la raison.
- **4.5a `is_groomed_ignore_les_blocs_de_code` (AC6).** Un corps dont les trois
  motifs n'apparaissent **que** dans une fence → `false`.
  **4.5b `fence_non_fermee_evalue_le_corps_entier`** — le repli permissif est
  testé, pas seulement documenté.
- **4.7 `is_groomed_accepte_les_trois_formes_de_verdict`** (extension). Un cas
  par forme, chacun **copié depuis `executor.rs`**. Plus
  `4.7b is_groomed_refuse_un_verdict_inconnu` : `second-pass (ITERATE)` et
  `first-pass (ESCALATE)` rendent `false` — élargir l'alternance ne doit pas
  transformer le prédicat en « contient le mot grooming ».
- **4.8 `le_corps_de_ce_ticket_est_vu`** (preuve de bouclage). Le corps de
  mika#2120 tel que ce grooming l'a écrit — callout nu **et** verdict
  `first-pass (READY, single-pass GROOMED)` — rend `true`. Il rend `false` sur
  `main` pour la raison du verdict, pas du chemin : le test le dit en
  commentaire, sinon il atteste le mauvais axe.
- **4.6 `select_feeder_candidates_retient_un_callout_prefixe` (AC5).** Bassin
  sous le plancher, un seul éligible, callout préfixé : il est retenu.

### Phase 5 — La moitié « écrivain » (dépôt `mika-platform`)

**Un pilote lancé sur `mika` ne peut pas atteindre ces fichiers.** Stratégie
CLAUDE.md « Primary + direct » : le gros du travail est sur `mika`, le changement
secondaire est fait directement sur une branche de `mika-platform`, même nom de
branche, référence croisée dans les deux corps de PR. **AC4 n'est satisfait que
lorsque les deux ont atterri** — l'implémenteur `mika` ne doit pas le cocher seul.

**5.1** `.claude/commands/mika-groom-ticket.md` `:224` et `:231` — remplacer
`` `<repo>/docs/plans/<file>` `` par `` `docs/plans/<file>` ``, avec une ligne
disant *pourquoi* (le chemin est relatif à la racine du sous-dépôt ; le dépôt est
donné par l'issue).

**5.2** `.claude/commands/mika-groom-milestone.md` `:227` et `:240` — même
correction sur le callout `Sequencing record`, qui porte le même préfixe et le
même défaut.

**5.3** Garde de non-régression : un `grep` qui échoue si
`` **Plan:** `<repo>/ `` ou `` **Sequencing record:** `<repo>/ `` réapparaît dans
`.claude/commands/`. Sans lui, la spec redérive au prochain passage — la
récidive du 2026-09-01 montre que le rappel par prose ne tient pas.

### Preuve de non-vacuité

Le correctif n'est pas vide si, et seulement si, la suite **échoue sur `main`** :
4.2 et 4.4 doivent y échouer (le préfixe n'est pas accepté), 4.5a aussi
(les fences ne sont pas retirées) ; 4.7 et 4.8 également (le verdict
single-pass n'est pas reconnu). Et **4.1 et 4.3 doivent passer sur `main`
comme après** — c'est ce qui prouve que la forme nue n'a pas régressé et que le
contrôle négatif n'a pas été écrit pour être vert.

## Commandes de vérification

```bash
cargo test -p mika-agent auto_pull
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
bash skills/bundled/_shared/test-dispatch-lib.sh
```

## Risques

| risque | mitigation |
|---|---|
| Le prédicat permissif ouvre un faux positif | Ancrage (AC6) + retrait des fences + contrôle négatif 4.3 ; coût borné à un créneau de dispatch |
| `strip_fenced_blocks` ampute un corps légitime sur une fence non fermée | Repli explicite : on n'ampute pas ; testé en 4.5b |
| La moitié `mika-platform` n'atterrit jamais et la spec continue d'écrire le préfixe | AC4 déclaré non satisfait tant que les deux PR ne sont pas là ; garde `grep` en 5.3 |
| L'axe du verdict redivergerait si `executor.rs` gagnait une quatrième forme | Les trois littéraux sont copiés depuis `executor.rs`, avec un commentaire de lockstep aux deux endroits ; test 4.7b borne l'élargissement |
| Les deux lecteurs redivergent plus tard | Test de parité 3.3 + note croisée dans la doc de `is_groomed` (phase 2) |
| Fixtures AC3 rendues vaines par la réparation manuelle | Fixtures figées et re-préfixées ; la non-vacuité est vérifiée dans les deux sens |

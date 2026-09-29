---
ticket: mika#2563
type: fix
tags: [loop-substrate, wip-rescue, compound, verify-pipeline, rescue-pipeline-verified, dispatch-lib]
---

# mika#2563 — une session tronquée n'atteste pas d'un pipeline complet

## Ce que la lecture du code et la mesure déplacent dans le ticket

Le ticket pose trois remèdes. La lecture du substrat en invalide deux et déplace
le troisième. **Ces rectifications sont le premier livrable du grooming** : le
remède 2 tel qu'écrit refuserait 71 % du trafic nominal de la boucle et
arrêterait le drain.

### M1 — `/ce:compound` n'est PAS dans le chemin nominal de la boucle

Le ticket parle de « l'étape 6 `/ce:compound` du pipeline `/mika` », et suppose
que le pilote y serait arrivé s'il n'était pas mort au plafond de tours.

`_detect_plan_on_branch` (`dispatch-lib.sh:8428`, mika#1074) **override la
commande d'entrée vers `/ce-work <plan>` pour tout ticket groomé** — et
`_pilot_had_no_shipping_tail` (`dispatch-lib.sh:4753`) l'écrit en toutes lettres :

> *« The autonomous loop dispatches **every groomed ticket** under `/ce-work
> <plan>` …, and `/ce-work` is, in its own words, "implementation and local
> verification only, without the shipping tail". »*

`/mika` — et donc son étape 6 — n'est le chemin que lorsque le callout `Plan:`
est absent ou que le fichier n'est pas dans le worktree. **Le pilote de #2555 et
#2556 n'aurait pas atteint l'étape compound même vivant : il ne tournait pas sous
`/mika`.** Le mécanisme que le ticket décrit n'existe pas.

### M2 — 71 % du trafic nominal n'a pas de `docs/solutions`, et c'est le régime

Mesuré sur les 100 derniers merges `--first-parent` de `origin/main`
(`.pilot-scratch/measure-2563.sh`, reproductible) :

| population | compte |
|---|---|
| merges examinés | 100 |
| portant du source | **93** |
| **sans aucun `docs/solutions/`** | **66 (71 %)** |
| portant au moins un `docs/solutions/` | 29 |

Les 29 % qui en portent un viennent du **plan**, pas de l'étape 6 : 71 des 189
plans de septembre prescrivent un `docs/solutions/`. C'est le grooming qui décide.

**Conséquence directe sur le remède 2 :** « exiger `docs/solutions/*.md` dès que
`SOURCE_BUCKET` est non vide » refuse 66 PR sur 93. Et comme
`scripts/verify-pipeline.sh` **est le terme 5 de `_measure_pipeline_verified`**
(`dispatch-lib.sh:8152`), la même modification ferait basculer **tout** rescue à
`rescue-pipeline-verified: no`, donc `hold[review]` systématique en QA. Le remède
écrit arrête la boucle.

### M3 — les rescues ne sont pas 2, ils sont ~1,1/jour, et leur taux est celui du nominal

16 commits `wip(` sur 14 jours dans `origin/main`. Sur les 9 merges de rescue de
la fenêtre des 120 derniers, **8 sans `docs/solutions`** (89 %). Contre 71 % au
nominal : **du même ordre**. La différence rescue/nominal n'explique donc pas le
défaut — elle confirme M1.

### M4 — le remède 3 est déjà livré, sous un autre nom (mika#2354)

Le ticket demande que « le recovery écrive `<!-- compound: missing -->` et que la
QA le lise comme un `block[pipeline]` ». Ce mécanisme existe intégralement :

- `_compose_rescue_pr_body` (`dispatch-lib.sh:8292`) écrit déjà
  `<!-- rescue-verify-failed: <terme> -->` plus un `<details>` portant l'extrait
  du terme fautif ;
- qa-review Step 1.5 (`qa-review/system_prompt.md:136`) lit déjà
  `rescue-pipeline-verified: no` + draft ⇒ `hold[review]`, revue terminée.

Ce qui manque n'est pas un marqueur : c'est **un terme qui échoue**. Un second
marqueur serait une seconde vérité sur un même fait.

### M5 — le remède 1 est du prompt-enforcement au substrat de la boucle

`iteration_context` est composé par le **modèle** (`self-dev-iterate`,
`self-dev-webhook-qa`, `self-dev`), pas par le shell. Par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, c'est une
moitié d'intention qui ne tient pas seule. Elle est livrée (R3), avec sa limite
écrite, mais elle n'est pas ce qui ferme le défaut.

### M6 — qa-review a déjà un compound check, délibérément soft

Step 4 (`qa-review/system_prompt.md:609`) : *« Compound doc check (soft check) …
If missing: note it as a finding but do NOT block or hold for this alone. »* Un
détecteur existe donc déjà, non bloquant, avec sa raison écrite. Le durcir
reviendrait à remordre les 71 % de M2.

## Le défaut, reformulé sur ce que la mesure établit

Le défaut n'est pas « la porte a retiré le contrôle compound-doc-presence ». Il
est plus étroit, et il est un **faux vert** :

> **`_measure_pipeline_verified` affirme « the local pipeline is complete » sur
> une session que le SDK a tuée.**

La chaîne, maillon par maillon :

| # | site | ce qui se passe |
|---|---|---|
| 1 | claude-pilot | plafond de tours atteint ⇒ `status: terminated`, `subtype: error_max_turns` |
| 2 | `dispatch-lib.sh:3533` | `STATUS="terminated"` |
| 3 | `dispatch-lib.sh:3640` | `_terminated_with_work=1` ⇒ `_post_flight_recovery` |
| 4 | `dispatch-lib.sh:9180` | `_measure_pipeline_verified` — **ses 5 termes ne lisent jamais `STATUS`** |
| 5 | `_compose_rescue_pr_body` | `verified: yes`, corps : *« measured the local pipeline as complete »* |
| 6 | qa-review Step 1.5 | marqueur `yes` ⇒ la porte s'ouvre ⇒ merge |

**La phrase qui condamne ce maillon est déjà écrite trente lignes plus haut dans
le même fichier**, dans `_pilot_had_no_shipping_tail` (mika#2492) :

> *« The `STATUS = success` term is the SECOND axis: a session killed by a
> guardrail or an SDK limit carries `STATUS = terminated` …, never `success`.
> **Truncated work must not be presented as complete just because its perimeter
> had no shipping tail.** »*

mika#2492 a écrit ce raisonnement pour la **classification** ; mika#2354 ne l'a
pas appliqué au **marqueur**. C'est le trou, à la ligne près. Et il est vérifié
par la preuve du ticket : #2556 porte `<!-- rescue-pipeline-verified: yes -->`
alors que son pilote a été tué à 151/150 tours.

## Décisions

**D1 — le refus porte sur la TRONCATURE, jamais sur l'absence de fichier.**
Refuser sur l'absence de `docs/solutions` refuse 71 % du nominal (M2) et n'est
pas livrable. Refuser sur « la session a été interrompue avant sa queue
d'expédition » cible exactement la population du défaut, avec zéro faux positif
sur le nominal : un pilote nominal conclut en `STATUS=success`.

**D2 — `scripts/verify-pipeline.sh` n'est PAS modifié.** C'est une divergence
assumée avec la lettre de l'AC1 du ticket, et sa raison est M2 : ce script est
partagé entre le job CI `pipeline-artifacts` (tout le trafic) et le terme 5 du
rescue. Aucune modification de son prédicat ne peut être étroite.

**D3 — `no-shipping-tail` ne mord pas.** mika#2492 a établi que c'est la voie
nominale et non une épave ; son `STATUS` vaut `success` par construction
(`_pilot_had_no_shipping_tail` l'exige). Mordre là serait remordre M2 sous un
autre nom.

**D4 — la détection est inconditionnelle, seule la disposition est gatée**
(patron mika#2249/#2420). La population qui basculerait de `yes` à `no` n'est pas
mesurable depuis un worktree (ni logs pilote, ni base). Livrer armé sans cette
mesure mettrait ~1 PR/jour de classe DECISION-CORE en attente d'un geste humain
(le daemon `wip_rescue` parke un DECISION-CORE sur marqueur `no`, mika#2286).
Voir § Fire-Disposition.

**D5 — le marqueur est écrit dans le CORPS, jamais sur stderr.** Le stderr de
`_post_flight_recovery` est le `Stdio::piped()` de `spawn_long_running_exec`, que
l'exécuteur ne lit **que** dans `if !status.success()` : sur un dispatch qui
réussit, il est droppé non lu. C'est le Signal M, et `dispatch-lib.sh:9105` le
dit déjà pour le refus mika#2503 (*« no grep is announced … the `echo` is a
convenience, deliberately NOT presented as a probe »*). Un détecteur désarmé dont
le résultat va dans ce puits serait inerte (classe mika#2205).

**D6 — un seul marqueur, trois valeurs, deux dispositions.** Pas de
`<!-- compound: missing -->` en plus de `<!-- rescue-verify-failed: … -->` : le
marqueur unique porte le fait, la disposition décide s'il fait basculer
`verified`.

**D7 — fail-closed dans le sens de la maison.** `STATUS` vide ou illisible
(branches B/C de `_run_claude_pilot` : sortie non-JSON, exit non nul) est traité
comme **non-success** : une session dont on ne peut pas lire la conclusion n'est
pas une session qui a conclu. L'asymétrie est celle que mika#2354 a déjà
arbitrée : *« one `no` too many costs a visible, reversible operator gesture;
one `yes` too many opens the review on incomplete work. »*

## Requirements

### R1 — terme `compound-traversal` dans `_measure_pipeline_verified`

Nouveau terme, placé **en tête** de la conjonction (terme 0) : il ne lit que des
variables et un `git log`, donc il court-circuite avant les deux invocations
cargo. `dispatch-lib.sh`.

Prédicat, dans l'ordre :

1. `STATUS = success` ⇒ terme **satisfait**, valeur `not-applicable`. (D3)
2. Sinon, la traversée est **attestée** si l'une des deux formes tient :
   - (a) **intrinsèque** — le diff `origin/main...HEAD` porte au moins un
     `docs/solutions/*.md` : la décision « il y a une leçon » a été prise et
     exécutée. Valeur `attested-solution`.
   - (b) **déclarative** — un commit de la plage `origin/main..HEAD` porte le
     trailer ancré `^Compound: none[[:space:]]+.+$` : la décision « pas de
     leçon » a été prise et dite. Valeur `attested-trailer`.
3. Sinon ⇒ terme **échoué**, valeur `absent`, wire name `compound-traversal`,
   extrait nommant les deux voies de levée.

Le `git log` / `git diff` échoue ou est illisible ⇒ **non attesté** (D7) : un
signal illisible n'est jamais un terme satisfait.

La forme du trailer suit `Pipeline-Exempt:` du même écosystème (ancré, raison
obligatoire — pas de forme nue : ce trailer naît aujourd'hui, il n'a aucune
compatibilité ascendante à porter).

### R2 — le marqueur, écrit inconditionnellement

`_compose_rescue_pr_body` gagne un argument et écrit
`<!-- compound-traversal: <valeur> -->` à côté des deux marqueurs existants,
pour les quatre valeurs (`not-applicable` / `attested-solution` /
`attested-trailer` / `absent`). **Écrit que le terme soit armé ou non** (D4/D5) :
c'est la seule surface durable, et c'est elle qui rend la condition d'armement
mesurable.

**Invariant d'identité (précédent T12, mika#2354 AC4) :** avec l'argument absent,
le corps reste **byte-identique** à celui d'aujourd'hui. C'est ce qui garde
`MIKA_RESCUE_VERIFY_ENABLED=0` un vrai rollback.

### R3 — la moitié d'intention, avec sa limite écrite

- `.claude/commands/mika.md` : documenter le trailer `Compound: none — <raison>`
  à l'étape 6, et que son absence sur une session tronquée vaut « non traversé ».
  (AC3 du ticket.)
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` et
  `skills/bundled/self-dev-iterate/system_prompt.md` : quand la PR porte
  `wip-rescue`, l'`iteration_context` composé nomme la traversée à rejouer —
  `/ce:compound`, puis soit le fichier soit le trailer.

**Écrit dans le plan et dans le commit : cette moitié ne tient pas seule** (M5).
Elle exprime l'intention ; R1 est ce qui tient.

### R4 — le format de fil est déclaré

`compound-traversal` et ses quatre valeurs atterrissent dans un marqueur de corps
de PR, donc dans un format de fil (mika#2201). Exécuter
`bash scripts/canonical-tokens-survey.sh --check` et `make check-canonical-tokens`
après l'ajout ; déclarer dans `scripts/canonical-tokens.tsv` si le survey les
réclame. **La résolution est de DÉCLARER, jamais d'allowlister.**

## Verification contract

**Site des tests : `skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh`**,
qui tourne en CI (`.github/workflows/ci.yml:106`, `make test-rescue-pipeline-verified`).

> **Ne PAS écrire dans `scripts/verify-pipeline-test.sh` : aucun job CI ne
> l'exécute** (772 lignes, absent de `ci.yml` et du `Makefile` ; constat déjà
> posé par mika#2544). Un test qui y atterrit est un détecteur inerte — classe
> mika#2205, et le défaut de ce ticket reproduit d'un cran.

Nouveaux cas, dans le style T1–T14 existant (shims `cargo`/`gh`, dépôts git
temporaires) :

- **T15a** — `STATUS=terminated`, diff sans `docs/solutions`, aucun trailer ⇒ le
  terme `compound-traversal` échoue, les termes cargo **ne tournent pas**
  (court-circuit vérifié sur `$STUB_LOG`), et le corps porte
  `<!-- rescue-verify-failed: compound-traversal -->`. **C'est le test négatif de
  l'AC1**, rejouant #2556.
- **T15b** — même état + un `docs/solutions/x.md` dans le diff ⇒ terme satisfait,
  marqueur `attested-solution`, la conjonction continue.
- **T15c** — même état + commit portant `Compound: none — <raison>` ⇒ terme
  satisfait, marqueur `attested-trailer`.
- **T15d** — trailer sans raison, ou non ancré en début de ligne ⇒ **non
  attesté** (le prédicat est strict).
- **T15e — contrôle négatif du nominal** : `STATUS=success`, diff sans
  `docs/solutions` ⇒ terme satisfait, `not-applicable`, aucun `rescue-verify-failed`.
  **Sans ce test, « le terme mord » est indistinguable de « le terme mord tout le
  monde ».**
- **T15f** — `STATUS` vide / absent / `failed` ⇒ **non attesté** (D7), pour
  chacune des trois valeurs.
- **T15g** — l'argument absent laisse le corps byte-identique à T12.
- **T15h** — le marqueur est écrit **aussi** quand la disposition est désarmée,
  et il ne fait alors **pas** basculer `verified`.

Étendre la boucle T10 aux huit termes (`compound-traversal` inclus).

## Definition of Done

- `_measure_pipeline_verified` porte le terme 0, gaté en disposition.
- `_compose_rescue_pr_body` écrit le marqueur inconditionnellement, identité T12
  préservée.
- Les huit cas T15 passent ; `make test-rescue-pipeline-verified` vert.
- `make test-dispatch-lib`, `make verify-bundled-skills`, `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings` verts.
- `bash scripts/verify-pipeline.sh origin/main` vert (le script n'est pas modifié).
- `scripts/canonical-tokens-survey.sh --check` vert, déclarations faites si
  réclamées.
- `.claude/commands/mika.md` documente le trailer ; les deux prompts d'itération
  nomment la traversée à rejouer.
- Le corps de PR porte le § Fire-Disposition, la sonde d'armement et ses haltes.

## Acceptance criteria

- [ ] **AC1** — Une PR de rescue issue d'une session tronquée (`STATUS != success`),
      sans `docs/solutions/*.md` dans son diff et sans trailer
      `Compound: none — <raison>`, obtient `rescue-pipeline-verified: no` avec le
      terme `compound-traversal` nommé dans son corps. Test négatif **T15a** dans
      `skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh` (suite
      exécutée par CI), rejouant l'état mesuré de #2556.
      *Divergence assumée avec la lettre de l'AC1 du ticket, qui nomme
      `scripts/verify-pipeline.sh` : ce script est partagé avec le job
      `pipeline-artifacts` et refuserait 71 % du trafic nominal (M2/D2). Le refus
      est produit au même endroit que les cinq termes existants, et il emprunte la
      même surface — le marqueur que qa-review Step 1.5 lit déjà.*
- [ ] **AC2** — La traversée compound est attestable **sans geste humain** par
      l'une des deux formes (fichier solution dans le diff, ou trailer
      `Compound: none — <raison>`), et une itération (c) sur une PR `wip-rescue`
      dispose de la consigne nommant cette traversée à rejouer (R3).
      *Preuve : T15b + T15c pour la reconnaissance ; la consigne d'itération est
      une moitié d'intention dont le plan écrit qu'elle ne tient pas seule (M5).*
- [ ] **AC3** — Le trailer `Compound: none — <raison>` est documenté dans
      `.claude/commands/mika.md` et reconnu par le prédicat (T15c), sa forme
      dégradée étant refusée (T15d).
- [ ] **AC4 — contrôle négatif, bloquant** — Une PR nominale
      (`STATUS = success`, y compris `no-shipping-tail`) sans `docs/solutions`
      n'est **pas** refusée : T15e vert, et `verify-pipeline.sh` inchangé
      (`git diff --stat` ne le nomme pas).
- [ ] **AC5** — Le marqueur `<!-- compound-traversal: <valeur> -->` est écrit sur
      **toute** PR de rescue, armé ou désarmé (T15h), et l'identité byte-à-byte du
      corps pré-mika#2354 est préservée sur le chemin du kill-switch (T15g).

## Fire-Disposition

Le plan livre des détecteurs : le terme `compound-traversal` (chemin de succès =
aucune violation) et les huit cas T15.

**Option retenue : (b) livrer désarmé, avec un suivi tracké pour l'armer.**

- **Interrupteur dédié** : `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL`, **défaut
  désarmé**. Trois paliers maison : absent/vide → désarmé ; `1`/`true`/`on`/`yes`
  → armé ; valeur non reconnue → **désarmé avec un WARN nommant la valeur entre
  guillemets**. Distinct de `MIKA_RESCUE_VERIFY_ENABLED`, qui désarme **toute** la
  mesure mika#2354 : réutiliser ce levier serait un rollback bien plus large que
  ce que ce ticket ajoute. La variable doit traverser
  `skills/executor.rs::inject_rescue_verify_env` (`sandboxed_pilot_env` reconstruit
  l'env sur allowlist positive — un réglage que seul son lecteur honore est un
  réglage décoratif, mika#2165).
- **Désarmé ≠ inerte** : le terme **mesure** et **écrit son marqueur** dans le
  corps de la PR (R2/D5) ; il ne fait simplement pas basculer `verified`. C'est ce
  qui distingue ce désarmement d'un détecteur muet (mika#2205), et c'est ce qui
  rend la condition d'armement mesurable au lieu d'intuitive.
- **Pourquoi pas (a) allowlist nommée** : la population n'est pas énumérable —
  ce sont les rescues **futurs**, pas un jeu fini de violations existantes. Une
  allowlist n'aurait aucune entrée et aucune assertion auto-nettoyante possible.
- **Pourquoi pas armé d'emblée** : la part des rescues en `STATUS != success`
  n'est pas mesurable depuis un worktree de dispatch (ni logs claude-pilot, ni
  base). Borne haute connue : ~1,1 rescue/jour (M3). Armer sans la mesure met
  potentiellement ~1 PR/jour de classe DECISION-CORE en attente d'un **geste
  humain**, le daemon `wip_rescue` parkant un DECISION-CORE sur marqueur `no`
  (mika#2286). C'est le précédent mika#2496 (`PILOT_MAX_TURNS` livré désarmé en
  attente de sa mesure V2) et mika#2201 (gate S2 livré `continue-on-error`).
  Ce n'est **pas** le cas mika#2272, où le zéro était l'absence de mesure : ici la
  mesure existe, elle se fait simplement sur le marqueur après déploiement.

### Sonde d'armement, et ses trois haltes

Après ≥ 7 jours et ≥ 10 rescues, sur l'hôte (geste opérateur — le worktree de
dispatch n'a pas de `gh` authentifié) :

```bash
# Combien de rescues n'ont pas traversé la décision compound ?
gh pr list --repo senara-solutions/mika --label wip-rescue --state all \
  --limit 60 --json number,body \
  --jq '.[] | select(.body | test("compound-traversal: absent")) | .number'

# CONTRÔLE POSITIF — le marqueur est-il seulement écrit ?
gh pr list --repo senara-solutions/mika --label wip-rescue --state all \
  --limit 60 --json number,body \
  --jq '[.[] | select(.body | test("compound-traversal:"))] | length'
```

**Armer** (`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` sur l'environnement du
**service**, jamais un shell interactif) quand la part `absent` est basse et que
le contrôle positif est non nul.

- **Halte 1 — la part `absent` porte le trafic nominal des rescues.** Ne pas
  armer : la boucle s'arrêterait sur un geste humain par jour. Le remède n'est
  alors pas ce terme mais **la traversée manquante elle-même**, et la question
  devient celle de M1 — faut-il rejouer compound dans la boucle autonome, et où.
  Ouvrir le suivi **avec ce compte**, jamais avec une intuition.
- **Halte 2 — le contrôle positif est nul.** Le marqueur n'est écrit nulle part :
  vérifier que le binaire servi porte le correctif **avant** toute conclusion.
  `skills/bundled/` est une projection du **binaire**, pas du checkout
  (mika#2340) — `cat ~/.mika/skills/.manifest-writer` doit porter le sha bâti.
  *Un détecteur qu'on n'a pas déployé se lit exactement comme une flotte saine.*
- **Halte 3 — après armement, une PR nominale est refusée.** Désarmer
  **d'abord** (`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=0`), diagnostiquer
  ensuite : le prédicat lit `STATUS` de travers, et c'est AC4/T15e qui a menti.
  Un faux positif ici coûte un dispatch entier.

## Hors périmètre, délibérément

- **Modifier `scripts/verify-pipeline.sh`** — D2/M2. Toute modification de son
  prédicat est partagée avec le job `pipeline-artifacts`, donc avec 71 % du
  trafic. Si une porte large est un jour voulue, elle demande d'abord de répondre
  à M1 (rejouer compound dans la boucle autonome), ce qui est un autre ticket.
- **Rejouer `/ce:compound` sur le chemin `/ce-work`** — c'est le défaut-racine que
  M1 met au jour, et il est plus grand que ce ticket : il change le périmètre du
  pilote nominal, alors que `/ce-work` a été choisi délibérément pour éliminer la
  classe « narrate-then-exit » (mika#1074). **Ticket de suivi**, précondition : la
  sonde d'armement ci-dessus, dont la Halte 1 est très exactement son déclencheur.
- **Durcir le Step 4 (soft) de qa-review** — M6 : il porterait les 71 % de M2, et
  son caractère soft est une décision écrite avec sa raison.
- **Le learning rétroactif de #2555** — déjà mergée ; le ticket le route lui-même
  vers une PR docs-only séparée. **Rien ici ne rétro-remplit** : fabriquer un
  marqueur décrivant un fait qu'on n'a pas observé est l'inverse de ce que ce
  travail défend. La sonde est la **prochaine** occurrence.
- **#2556** — geste d'opérateur nommé par le ticket (itération courte avant
  merge), pas un livrable de ce plan.
- **La mise à jour du marqueur après itération** — la mesure mika#2354 est
  one-shot à la création de la PR et rien ne la rejoue. Limite héritée, ni
  élargie ni corrigée : la levée d'un `no` reste le geste que qa-review Step 1.5
  nomme déjà (un-draft, ou éditer le marqueur).

## Ce que ce travail n'achète PAS

Il ne fait écrire **aucun** learning. Il empêche une session **tuée** d'affirmer
que son pipeline est complet, et rend cette affirmation lisible dans le corps de
la PR. La question « la boucle autonome devrait-elle décider, à chaque ticket,
s'il y a une leçon à écrire ? » reste ouverte — M1 montre qu'aujourd'hui personne
ne la pose, sur aucun chemin, et c'est le ticket de suivi.

Livré désarmé, il n'achète d'abord qu'un **marqueur**. Son silence ne prouve rien
tant que la sonde d'armement n'a pas été exécutée sur l'hôte.

---
module: skills/bundled/_shared/dispatch-lib.sh + skills/bundled/qa-review
tags: [loop-substrate, rescue-net, dispatch, closes-reference, asymmetry, fail-closed, grooming-artefacts]
problem_type: logic_error
category: dev-loop
created: 2026-09-04
ticket: mika#2157
---

# Une protection révocable d'un clic contre une conséquence que personne n'a besoin de déclencher

## Le problème

`mika-cloud` PR#202 : ouverte par le filet de récupération de `dispatch-lib`, **approuvée** par `mika-platform-qa`, `MERGEABLE`. Son diff intégral fait huit lignes :

```diff
diff --git a/.claude/groom-verdict-trail.log b/.claude/groom-verdict-trail.log
@@ -1,2 +1,4 @@
 2026-06-28T12:59:10Z	groom-ticket	d080f7bd…	ITERATE
 2026-06-28T13:00:39Z	second-review	d080f7bd…	GROOMED
+2026-08-25T10:40:57Z	groom-ticket	4f2fdbe7…	READY
+2026-08-25T10:41:32Z	second-review	4f2fdbe7…	GROOMED
```

**+2 / −0, un fichier, et c'est le journal d'audit du grooming lui-même.** Zéro ligne de correctif. Le corps portait `Closes #192`. `mika-cloud#192` — un bug p1 sur le parcours d'invitation — était toujours OPEN.

Un seul geste séparait ce ticket d'une fermeture silencieuse : sortir la PR du brouillon, puis merger. L'approbation était déjà posée.

## La cause

`dispatch-lib.sh`, dans le bloc qui compose le corps de la PR de récupération :

```bash
Closes #${ISSUE_NUM}
```

Posé **sans condition**. Le filet ne regardait jamais ce qu'il avait capturé.

Quand un pilote meurt sans commiter, `dispatch-lib` commite ce que le worktree contient. Or un worktree de grooming contient au minimum l'effet de bord de `dispatch-lib` lui-même : deux lignes ajoutées à `.claude/groom-verdict-trail.log` par `_append_groom_verdict_trail`. Le filet emballait ça et déclarait que ça fermait le ticket.

## La forme du défaut : l'asymétrie était du mauvais côté

C'est le cœur, et c'est réutilisable bien au-delà de ce site.

| Surface | Nature | Révocable par |
|---|---|---|
| `--draft` | procédurale | un geste humain |
| `<!-- rescue-pipeline-verified: no -->` | procédurale | un geste humain |
| `Closes #N` | **automatique** | rien — GitHub l'exécute au merge |

Deux protections qu'un humain lève d'un clic, contre une conséquence qu'aucun humain n'a besoin de déclencher. Les garde-fous existants étaient réels ; ils étaient simplement du mauvais côté de la balance.

**La règle générale :** quand une sortie porte une instruction que la plateforme exécute automatiquement, la garde ne peut pas être une convention procédurale en amont. Elle doit vivre dans la production de l'instruction elle-même. Compter les protections ne suffit pas ; il faut comparer leur *nature* à la nature de ce qu'elles retiennent.

## La seconde moitié : une approbation n'est pas un signal

`mika-platform-qa` a **approuvé** PR#202. Un diff de deux lignes de journal, pour un ticket qui demandait une pré-validation de code d'invitation avant une traversée OAuth. La revue n'a pas comparé le diff à ce que le ticket demandait — elle a validé un diff qui, littéralement, ne pouvait satisfaire aucun AC.

Même classe que ce que le balayage du 2026-09-03 avait mesuré : sur 32 cas de « une PR mergée mentionne l'issue », **zéro** correspondait à un travail réellement fait. « Cette PR référence #N » n'est pas un signal ; « cette PR est approuvée » n'en était pas un non plus.

## Le correctif

**`_rescue_diff_carries_work <wt_dir>`** — une seule question : le diff `origin/main...HEAD` contient-il au moins un chemin hors de la liste d'artefacts d'incident ?

La liste n'est pas une intuition : c'est l'**union des deux endroits où dispatch-lib déclare déjà « ceci est à moi, pas au pilote »** — le Tier 2 de `_clean_worktree_for_rebase` (ce que le rebase réinitialise sans rien perdre) et les exclusions `git add -A` du commit de récupération lui-même (`.claude/claude-pilot.json`, `.claude/*.local.*`, `.claude/commands/`), que le code appelle explicitement « scaffold paths ». Un chemin que le rebase écrase, ou que la récupération refuse de stager, ne peut pas être un livrable.

Chercher une seule autorité ne suffisait pas : la première implémentation n'a transcrit que le Tier 2, et a donc laissé passer `.claude/claude-pilot.json` — recopié depuis `$PLATFORM_DIR` dans chaque worktree, et nommé scaffold à trois lignes de là.

Elle est volontairement **close et courte** : tout chemin non énuméré compte comme du travail. Chaque ajout retire du poids au filet dans son cas utile, et doit donc arriver avec son cas de test symétrique.

### Le piège qui a failli reproduire le défaut à l'intérieur du correctif

Sous le `core.quotePath=true` par défaut de git, `git diff --name-only` **entoure de guillemets et échappe en octal** tout chemin contenant un octet non-ASCII :

```
"docs/plans/\303\251tude-plan.md"
```

Cette chaîne ne correspond à aucun motif `case` — elle tombe sur `*)` et le prédicat répond « porte du travail ». Un plan au titre accentué, seul dans le diff, aurait de nouveau écrit `Closes #N`. Sur un dépôt dont les plans, les journaux et les tickets sont rédigés en français, ce n'est pas un cas limite : c'est l'entrée nominale.

Le correctif est `git -c core.quotePath=false diff --name-only -z`, lu en NUL-délimité — et par **substitution de processus**, pas par `$(...)`, parce que bash supprime les octets NUL dans une substitution de commande et recollerait tous les chemins en un seul bloc non reconnu.

**La leçon générale :** un classificateur qui filtre par motif sur une sortie d'outil hérite du formatage de cet outil. Le mode par défaut de `git diff --name-only` n'est pas conçu pour être parsé ; il est conçu pour être lu par un humain. Toute garde bâtie dessus a un angle mort de la taille exacte de son échappement — et l'angle mort tombe du côté fail-open, parce qu'un chemin non reconnu ressemble à un chemin non listé.

****Le fail-closed est l'argument du ticket appliqué à son propre correctif.** Diff vide, diff non mesurable (`origin/main` non fetché, dépôt cassé) : tout tombe du côté `Refs`. Se tromper vers `Refs` laisse un ticket ouvert qu'un opérateur ferme à la main — visible, réversible, une ligne dans une liste. Se tromper vers `Closes` est une fermeture silencieuse que personne ne mesure. **On ne place pas un échec de mesure du côté automatique.**

**`_compose_rescue_pr_body`** — le heredoc était inséré directement dans l'argument `--body` de `gh pr create`. Sous cette forme, la décision n'était testable qu'en simulant `gh` et en relisant son argv. Extrait en fonction (sur le précédent de `_derive_recovery_pr_title`), il se teste sur de **vrais dépôts git temporaires** : la sonde traverse le vrai `git diff` au lieu d'une reconstruction du diff écrite depuis le plan.

**Le marqueur `<!-- rescue-diff: incident-only|carries-work -->`** — le producteur mesure une fois, le consommateur lit. `qa-review` étape 1.5 lit le marqueur au lieu de rejuger, et tient la PR sur un diff entièrement incident, **avant** la branche vérifié/non-vérifié. Deux jugements indépendants d'un même fait divergent ; c'est exactement le motif que mika#1618 avait déjà réglé pour l'autre marqueur du même corps.

### Un jeton de verdict n'est pas une étiquette : c'est un appel de fonction

Le plan avait retenu `block[ac]` pour le refus, sur la foi d'une note de grooming affirmant qu'il « route vers l'opérateur sans réessai automatique ». La mesure dit le contraire : `verdict_handler.rs:908` (`handle_block_ac`) **dispatche un nouveau run claude-pilot de correction d'AC** — `try_engine_dispatch`, audit `ac_fix_dispatched` — jusqu'à `BLOCK_AC_MAX_RETRIES = 3`, et n'escalade qu'après la limite. `handle_hold_review` notifie l'opérateur et laisse la tâche `in_progress` : aucun dispatch.

Poser `block[ac]` ici aurait fait déclencher jusqu'à trois runs autonomes — occupant à chaque fois l'unique créneau de dispatch — contre une PR dont la première ligne dit qu'elle n'existe pas pour être mergée. **Le correctif aurait fabriqué du travail de boucle à partir d'un diff vide**, exactement à l'envers de ce que le ticket protège.

Second appui, indépendant : `block[ac]` porte le contrat de format de l'étape 2.5 (`PLAN-AC VERIFICATION` obligatoire, au moins un AC `❌`). Un item qui termine la revue **avant** l'étape 2.5 n'a évalué aucun AC — le modèle devrait donc omettre une section déclarée obligatoire, ou inventer des verdicts qu'il n'a pas mesurés.

**La leçon :** dans ce produit, un jeton de verdict est consommé par un gestionnaire structurel qui agit. Choisir un jeton d'après sa *sémantique lue dans le prompt* — sans lire le `handle_*` qui l'exécute — c'est choisir un effet de bord au hasard. Le vocabulaire du prompt et le comportement du moteur sont deux surfaces, et seule la seconde bouge le monde. Ici les deux se contredisaient à l'écrit : le prompt affirme lui-même que `block[ac]` route « sans réessai automatique ».

## Ce que la structure porte, et ce qu'elle ne porte pas

Le côté revue est une garde de **prompt**, et un prompt n'est pas une structure — `feedback_prompt_enforcement_fragile` le dit, et l'empirique du substrat de boucle le confirme. Le contrôle de contrat ajouté à la suite de tests pin la **présence** du texte, pas l'adhérence du modèle.

Ce que la structure porte réellement, c'est l'autre moitié : **après ce correctif, même si `mika-qa` approuve à tort une PR entièrement incidente et qu'un humain la merge, aucun ticket ne se ferme** — l'instruction n'est plus dans le corps. C'est l'argument d'asymétrie retourné du bon côté : la conséquence automatique est désarmée par la structure, l'opinion reste gardée par le prompt.

## Le piège de test que ce correctif a révélé

Deux assertions de `test-dispatch-lib.sh` cherchaient l'en-tête de récupération et le marqueur `rescue-pipeline-verified` **par grep sur le bloc du site d'appel**. L'extraction du heredoc les a fait échouer sans qu'aucune propriété ne soit perdue.

Une assertion qui grep un bloc de code mesure l'emplacement, pas la propriété. Quand la propriété déménage, elle échoue à tort — et la tentation est alors de la supprimer. La bonne réponse est de la faire **suivre** l'extraction : le site d'appel doit router par le composeur, et le composeur doit émettre les deux. La propriété pinnée est inchangée ; ce qui change est l'endroit où on la lit.

## Portée non traitée, et pourquoi

Le désarmement rétroactif des PR de récupération déjà ouvertes est hors périmètre — **par mesure, pas par omission**. Le balayage de 62 PR de récupération sur quatre dépôts a trouvé **une seule** PR creuse, `mika-cloud#202`, déjà fermée non mergée ; la plus petite PR de récupération réellement mergée est mika#1637 (+163, 2 fichiers). L'ensemble d'exceptions est vide par mesure.

**Condition de réveil, datable :** si une PR de récupération creuse est trouvée ouverte après ce correctif — `gh pr list --search "Auto-rescued in:body" --state open` sur les quatre dépôts, croisé avec un diff entièrement incident — le traitement rétroactif devient dû, avec son propre ticket et une exception nommée par PR.

## Références

- `skills/bundled/_shared/dispatch-lib.sh` — `_rescue_diff_carries_work`, `_compose_rescue_pr_body`, et le commentaire croisé sur le Tier 2 de `_clean_worktree_for_rebase`.
- `skills/bundled/_shared/tests/test_rescue_closes_guard.sh` — T1–T6 sur de vrais dépôts git ; `make test-rescue-closes-guard`.
- `skills/bundled/qa-review/system_prompt.md` — étape 1.5 item 3.
- mika#1282 / mika#1396 — les deux classes de récupération ; mika#1618 — le marqueur lisible par machine dont celui-ci reprend la forme ; mika#1713 / mika#2151 / mika#2146 — les trois tickets voisins tenus hors périmètre.

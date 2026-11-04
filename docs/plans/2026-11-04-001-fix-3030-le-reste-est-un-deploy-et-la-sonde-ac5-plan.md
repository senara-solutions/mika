# mika#3030 — le reste est un deploy et la sonde AC5, pas du code

- **Ticket :** senara-solutions/mika#3030
- **Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
- **Périmètre :** mika seulement. La partie claude-pilot (test de `can_use_tool`, règles explicites de `permissions.yaml`) est hors vol, nommée telle par le ticket.
- **Disposition de ce plan : HALTE-ET-REMONTÉE.** Le livrable de code de ce ticket est **déjà mergé dans `main`** par #3032. Ce plan ne propose aucun code, et dit pourquoi — par mesure, refaisable dans ce worktree. Ce qui reste n'est pas écrivable par un pilote : un `make deploy` et une fenêtre de mesure.

Un plan antérieur existe (`docs/plans/2026-11-03-006-fix-3030-allowlist-du-worktree-filtree-au-seeding-plan.md`, sur `main` depuis #3032). Ce plan-ci ne le remplace pas : il le **constate exécuté** et re-mesure l'écart qui subsiste. Le re-groom a été déclenché par la disparition du callout canonique, réécrit par la porte d'auto-pull (mika#2839) quand le merge de #3032 a supprimé la branche qui portait le plan — pas par un reste de travail détecté.

## Contexte mesuré

### 1. Le livrable de code est en place, et il est testé

#3032 (mergé le 2026-11-04 à 06:12:57Z, `5f1db5b99`) a remplacé la copie par une construction. Les trois AC de code sont fermées, chacune vérifiable ici :

| AC | site livré | vérification dans ce worktree |
|---|---|---|
| AC1 (`local`) | `_seed_worktree_settings_allowlist`, `dispatch-lib.sh:4783` | `sed -n '4783,4820p' skills/bundled/_shared/dispatch-lib.sh` |
| AC1 (`project`) | assertion de fin de suite sur `.claude/settings.json` | `jq -r '.permissions // "null"' .claude/settings.json` → `null` |
| AC2 | `_WORKTREE_GH_ALLOW_NARROW`, 9 entrées étroites, aucune `gh pr ready` | `sed -n '4748,4768p' skills/bundled/_shared/dispatch-lib.sh` |
| AC4 | scan `grep -cE '^[^#]*\bcp\b[^;&\|]*settings\.local\.json'` == 0, strict | `grep -nE '\b(cp\|install\|rsync)\b[^;&\|]*settings[^;&\|]*\.json' skills/bundled/_shared/dispatch-lib.sh` → aucun résultat |

Le prédicat du filtre est sur la **forme** (`startswith("Bash(")` et `test("\\bgh\\b")`), pas sur une liste de commandes interdites — la règle que mika#2309 a déjà écrite dans ce dépôt. Le repli est fail-closed (liste étroite seule, jamais la source), et l'écriture passe par `$tmp` + `mv` avec borne basse `[ -s "$tmp" ]`.

Le segment de test est intégralement vert. 59 assertions, mesurées ici :

```
bash skills/bundled/_shared/test-dispatch-lib.sh 2>&1 | grep "mika#3030"
```

Elles couvrent les 9 commandes de la liste L (non couvertes), les 8 gestes de pipeline d'AC2 (couverts), le contrôle négatif `Bash(gh issue view:*)`, le contrôle `gh pr ready` non couvert, la survie des 4 entrées non-`gh`, la disparition des 5 jokers, les deux chemins de repli (`no-src`, `no-jq`) avec la preuve qu'aucun n'a recopié la source, et le matcher lui-même sur 6 formes. La suite est câblée en CI : `ci.yml:85` → `make test-dispatch-lib`.

Un seul site de seeding est à amender, et il l'est : `_seed_worktree_settings_allowlist` est appelé à `dispatch-lib.sh:6199`, dans `_set_up_worktree`, au même niveau et sur le même chemin que `_seed_worktree_slash_commands` et `_seed_pilot_scratch_dir` — après la porte de rebase, donc sur la création **comme** sur la réutilisation. `grep -n '_seed_worktree_settings_allowlist' skills/bundled/_shared/dispatch-lib.sh` rend une définition, un appel, une mention en commentaire.

### 2. Et pourtant le défaut est encore vivant — ce worktree en est la preuve

Ce worktree a été préparé par `dispatch-lib.sh` le 2026-11-04 à 07:20, soit **après** le merge de #3032. Son allowlist porte les cinq jokers, et aucune des neuf entrées étroites :

```
grep -n '"Bash(gh' .claude/settings.local.json
#   4: "Bash(gh pr:*)",      49: "Bash(gh issue:*)",   81: "Bash(gh repo:*)",
# 119: "Bash(gh issue *)",  142: "Bash(gh label *)",
grep -c '"Bash(gh pr create:\*)"' .claude/settings.local.json   # → 0
```

Le fichier est la copie intégrale de l'allowlist de l'opérateur (7 539 octets, 104 entrées — le compte du ticket). Le filtre n'a pas tourné et n'a pas échoué : il n'était pas là.

La cause est celle que CLAUDE.md § *Deploying a bundled-skill change (mika#2340)* décrit. `skills/bundled/` ne se lit pas au runtime : `build.rs` le compile dans le binaire, et `seed_support_dirs` écrit `~/.mika/skills/_shared/dispatch-lib.sh` depuis cette constante. Le `dispatch-lib.sh` **installé** est antérieur à #3032. La chaîne est rebuild → seed → read, et seule l'étape 1 manque.

C'est une inversion importante pour la lecture du ticket : les AC1/AC2/AC4 sont vraies **dans le dépôt** et fausses **en production**. Aucun test ne pouvait le dire, parce que tous mesurent le checkout.

### 3. Observation hors périmètre, nommée pour ne pas être prise pour une régression

La suite rend `1889 passed, 73 failed`. Aucun échec n'est dans le segment mika#3030. Les 73 se concentrent sur les fixtures de mika#2641 et mika#2996, avec le même motif : `reason=the engine's disposition reader refused the first-pass response (mika#2996)`.

Ils sont **hors du périmètre de ce ticket**, sur deux faits vérifiés :

- Les 4 hunks de #3032 sur `dispatch-lib.sh` sont en `@@ -4722`, `-6092`, `-7304`, `-11933` — tous sur l'allowlist, aucun sur un lecteur de disposition (`git show 5f1db5b99 -- skills/bundled/_shared/dispatch-lib.sh | grep '^@@'`).
- Le lecteur que le message incrimine a été touché par `e4873c6af` (mika#3027, `arch_disposition.rs`, `skills/executor.rs`), mergé **avant** #3032.

Ce plan ne tranche pas leur cause et ne les corrige pas : le ticket est au gel, et ce n'est pas son sujet. Il les nomme pour que l'opérateur l'apprenne et qu'un futur vol sur ce fichier ne les lise pas comme sa propre régression.

## Ce qui reste, et à qui

### R1 — `make deploy` (opérateur, hors sandbox du pilote)

Le seul geste qui ferme l'écart du § 2. Il est hors d'atteinte d'un pilote : `~/.mika/` n'est pas monté dans le sandbox (`ls ~/.mika/skills/_shared/dispatch-lib.sh` y échoue), et un deploy redémarre les services.

Sonde d'attestation, dans cet ordre — la première est directe, la seconde est de bout en bout :

```sh
grep -c _WORKTREE_GH_ALLOW_NARROW ~/.mika/skills/_shared/dispatch-lib.sh   # attendu ≥ 1
# puis, sur le PREMIER worktree préparé après le deploy :
grep -c '"Bash(gh pr create:\*)"'  <worktree>/.claude/settings.local.json  # attendu 1
grep -c '"Bash(gh issue:\*)"'      <worktree>/.claude/settings.local.json  # attendu 0
# et la trace que le filtre a tourné en mode filtré, pas en repli :
grep 'worktree_allowlist: mode=' ~/.mika/logs/server.log | tail -1         # attendu mode=filtered
```

La troisième ligne mérite d'être lue : `mode=narrow_only` au lieu de `mode=filtered` signifie que le repli fail-closed a pris — le défaut est fermé, mais les entrées non-`gh` de l'opérateur manquent au pilote, et c'est un symptôme (`jq` absent, source illisible) à traiter séparément.

### R2 — AC5, la sonde (opérateur, après R1)

AC5 compte sur « les 5 vols de la boucle qui **suivent le deploy** ». Elle n'est donc pas mesurable avant R1, et aucun code ne la rend mesurable plus tôt. Procédure :

```sh
gh issue list --repo senara-solutions/mika --state all \
  --search "author:app/mika-platform-dev created:>=<instant du deploy>" \
  --json number,createdAt,title
```

Croiser les `createdAt` avec les fenêtres des 5 premières tâches `run_claude_pilot` postérieures au deploy. **La cible est 0.** Le compte s'écrit en commentaire sur mika#3030, avec l'instant du deploy et les 5 fenêtres, de sorte que le lecteur puisse refaire le croisement.

Au rythme mesuré par le ticket (7 tâches le 2026-11-03, 58 du 27/10 au 02/11), la fenêtre se remplit en un à deux jours — bien avant le 01/12.

Repère de lecture : un ticket ouvert par `mika-platform-dev` dans la fenêtre **ne prouve pas** que la porte est rouverte. Il faut lire la décision : si `audit_events` porte une ligne `allow` pour ce `tool_use`, la politique a été interrogée et a dit oui, ce qui est le comportement voulu et un sujet pour claude-pilot. C'est l'**absence** de toute ligne de décision qui signait le défaut de ce ticket.

### R3 — Rien d'autre, et ce vol n'ouvre aucun ticket

Gel des tickets jusqu'au 01/12 : conformément au corps, ce vol n'ouvre aucun ticket de suivi. R1 et R2 vivent dans ce plan et en commentaire sur mika#3030. L'observation du § 3 y va aussi.

## Pourquoi ce plan ne livre aucun code

Trois candidats ont été évalués et écartés, chacun par une mesure :

1. **Étendre AC1 à la source `project`.** Déjà fait : la suite porte l'assertion en fin de fichier, et `jq -r '.permissions // "null"' .claude/settings.json` rend `null`. Un `jq` manquant rend `""` ≠ `"null"`, donc l'assertion est fail-closed, pas verte par construction.
2. **Élargir le scan d'AC4 du nom à la forme** (`settings*.json` au lieu de `settings.local.json`). Le trou existe en théorie, mais `grep -nE '\b(cp|install|rsync)\b[^;&|]*settings[^;&|]*\.json' dispatch-lib.sh` ne rend **aucune** ligne : zéro violation présente, et le seul `cp` voisin (`claude-pilot.json`, l. 6192) n'est pas un fichier de réglages de permissions — c'est la config du relais, dont `--relay-config` dépend, et le code le dit à cet endroit. Le gain est nul et le risque de faux rouge non.
3. **Écrire un script pour AC5.** Le ticket demande « on écrit le compte **ici** », pas un outil. Un script serait un élargissement non demandé, et le gel interdit d'ouvrir le ticket qui le porterait.

Fabriquer un livrable pour qu'un vol ait quelque chose à committer serait le défaut que mika#1322 nomme. La lecture honnête est que le code de ce ticket est écrit, mergé et testé, et que sa fermeture dépend d'un geste d'opérateur.

**Conséquence opérationnelle, à nommer sans détour :** si ce plan est dispatché à un pilote d'implémentation, ce pilote n'aura rien à committer et la post-flight de `dispatch-lib` rendra `PIPELINE_INCOMPLETE`. C'est le résultat correct et non une panne — mais le geste juste est de **ne pas dispatcher d'implémentation**, de retirer le label `ready`, et de traiter R1 puis R2.

## Definition of Done

- [x] AC1, AC2 et AC4 sont implémentées et vertes sur `main` (#3032, `5f1db5b99`).
- [x] L'écart dépôt/production est mesuré et attribué à l'absence de deploy, avec la preuve dans ce worktree.
- [x] Les 73 échecs de suite sont attribués hors périmètre, sur deux faits vérifiés.
- [x] La sonde de R1 est écrite, avec sa lecture `mode=filtered` / `mode=narrow_only`.
- [x] La procédure de R2 est écrite, avec son repère de lecture.
- [ ] **R1 — `make deploy`** et la sonde d'attestation. *Opérateur.*
- [ ] **R2 — AC5**, le compte écrit en commentaire sur mika#3030. Cible 0. *Opérateur, après R1.*
- [ ] Label `ready` retiré de mika#3030 pour que la boucle cesse de le re-dispatcher pendant l'attente de R1/R2.

## Acceptance criteria

Transcrits du corps du ticket. **Périmètre de ce vol : mika seulement** — la partie claude-pilot (test du callback `can_use_tool`, règles explicites de `permissions.yaml`) est hors vol ; un siège interactif la fera sur claude-pilot. Le statut mesuré est annoté en regard.

- **AC1.** Dans un worktree préparé par `dispatch-lib.sh`, aucune règle allow des réglages `project` et `local` ne couvre une commande de la liste L : `gh issue create`, `gh issue close`, `gh issue edit`, `gh issue comment`, `gh pr merge`, `gh pr review`, `gh pr close`, `gh repo delete`, `gh label create`. Test dans `test-dispatch-lib.sh`, sur une fixture `settings.local.json` qui porte `Bash(gh issue:*)` et `Bash(gh pr:*)`. Dans le même test, contrôle négatif : une entrée de lecture (`Bash(gh issue view:*)`) reste présente dans le worktree.
  → **Fermée dans le dépôt** par #3032, les deux sources. **Pas encore vraie en production** : R1.
- **AC2 (mika).** Les gestes du pipeline qui passent aujourd'hui par l'allow copié restent permis dans le worktree, par des entrées étroites écrites à la place des jokers retirés : `gh pr create`, `gh pr view` et `gh pr edit`, plus les lectures `gh issue view` et `gh issue list`. Un témoin positif par entrée est ajouté au test d'AC1. Aucune entrée `gh pr ready` n'est écrite : le contrat draft de mika (#1613, #1682, #2624) interdit au pilote de rendre prête une PR, parce qu'un draft peut être une retenue posée exprès.
  → **Fermée dans le dépôt** par #3032 — 9 entrées étroites, 8 témoins positifs, contrôle négatif sur `gh pr ready`. R1 pour la production.
- **AC4.** `dispatch-lib.sh` ne copie dans le worktree aucun fichier de réglages qui n'a pas été filtré par AC1. Un scan, épinglé par un test, refuse un deuxième `cp … settings.local.json`.
  → **Fermée** par #3032, égalité stricte à 0, registre d'exceptions vide.
- **AC5.** Sonde : sur les 5 vols de la boucle qui suivent le deploy, on compte les tickets dont l'auteur est `mika-platform-dev` et la date de création tombe dans la fenêtre du vol, et on écrit le compte ici. La cible est 0.
  → **Ouverte, et non fermable par un vol de pilote** : elle mesure l'après-deploy. Procédure en R2.

## Fire-Disposition

**Gate N/A — ce plan ne livre aucun détecteur.** Il ne livre aucun code : ni test, ni assertion, ni règle de lint, ni garde CI, ni scan structurel. La section n'est donc pas requise, et elle n'est pas inventée.

Pour mémoire, la Fire-Disposition du vol qui a livré les détecteurs de ce ticket (#3032) était l'option (a) avec **registre d'exceptions vide** : le scan d'AC4 est une égalité stricte à `0`, jamais un `<= N`, parce qu'un seuil est une exception sans date d'expiration. Ce plan ne l'amende pas et n'y ajoute aucune entrée.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| aucun — le code d'AC1/AC2/AC4 est déjà sur `main` (#3032) | 0 |
| R1 (`make deploy`) — geste opérateur, hors sandbox, pas du code | 0 |
| R2 (AC5) — mesure post-deploy écrite en commentaire, pas du code | 0 |

Total estimé : 0 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Un seul vol, et il tient trivialement : il n'y a rien à écrire. Le total est à 0 non pas par découpage mais par **constat** — le vol de code de ce ticket a déjà eu lieu et a mergé. Aucun découpage en phases ni en vols n'est requis, et aucune ligne `Tracked in:` n'est due : le gel interdit d'ouvrir un ticket, et le reste (R1, R2) est nommé dans ce plan et destiné à l'opérateur.

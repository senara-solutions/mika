---
title: "Pipeline pilote allégé : conduire l'essai borné et le relevé de couverture"
date: 2026-10-15
type: chore
issue: mika#2694
category: dev-loop
module: docs/solutions/dev-loop
component: development_workflow
---

# Plan : conduire l'essai borné du pipeline pilote allégé (mika#2694)

## Constat : la moitié du ticket est déjà livrée

`CLAUDE.md` porte **déjà** les deux directives, depuis le commit `6f1e5a4a`
(PR #2695, mergée le 2026-10-14T20:11:06Z) :

- « Simplification threshold: none. Do not invoke `ce-simplify-code`. »
- « Code review: delegated to mika-qa. … write `Code review: delegated to
  mika-qa (mika#2694)` in the PR body. »

Le premier critère d'acceptation est donc satisfait et `claude_md_size` est vert.
**Ce vol ne retouche pas `CLAUDE.md`** : le réécrire dupliquerait la consigne.
Ce qui reste du ticket est l'**essai borné** et le **relevé de couverture** —
un travail de lecture et de rédaction, sans ligne de code.

`.claude/commands/mika.md` n'est pas touché (attendu 4 du ticket).

Le plan `docs/plans/2026-10-14-chore-2694-pipeline-pilote-allege-plan.md` est
conservé : il est l'archive du vol qui a livré AC1 (PR #2695). Le présent plan
le remplace comme plan courant du ticket — il est le plus récent par nom, donc
celui que `_detect_plan_on_branch` retient.

## Ce que l'essai doit mesurer, et pourquoi l'instrument désigné ne suffit pas

Le ticket désigne comme instrument le transcript
`~/.mika/data/pilot-transcripts/<task>.jsonl`. Trois propriétés de cet
instrument ont été mesurées au grooming et **conditionnent la méthode** :

1. **Le `CLAUDE.md` n'est pas dans le transcript d'un vol de pilote.** La
   phrase stable `conversation-first AI executive assistant` rend 0 occurrence
   dans `db73ec9f-…` et `2d80e334-…` (deux vols post-merge), et 1 dans une
   session Claude Code locale. Donc **« ce vol portait la consigne » ne se lit
   pas dans le transcript** : il faut le dériver autrement (§ Méthode, étape 2).
   Un essai qui déduirait l'appartenance d'un `grep 'Pilot flights end lean'`
   classerait tous les vols comme non-allégés — et conclurait à tort.
2. **La signature d'invocation porte des espaces.** La forme réelle est
   `"name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"`.
   Au grooming, `grep '"skill":"compound-engineering:ce-simplify-code"'` (sans
   espaces) a rendu **zéro fichier sur 44** — un faux négatif complet qui se
   lisait exactement comme le résultat espéré.
3. Par conséquent : **un relevé d'absence n'est publiable qu'avec son contrôle
   positif.** La même commande, mot pour mot, doit trouver au moins une
   occurrence dans un transcript témoin pré-merge. Sans ce contrôle, l'absence
   mesure la signature, pas le comportement du vol.

## Donnée déjà acquise au grooming

Avec la signature calibrée, sur les 44 transcripts écrits après
2026-10-14T20:11 :

- **un seul** invoque `ce-simplify-code` *et* `ce-code-review` :
  `db73ec9f-57d8-416c-b6e1-a79c14a81a09.jsonl` (mtime 2026-10-14 21:12) ;
- les 43 autres n'invoquent ni l'un ni l'autre.

Ce fichier est donc le seul candidat au critère 3 (« crochet inerte »), et
l'étape 2 de la méthode doit trancher s'il portait la consigne — sans quoi son
invocation est légitime et ne dit rien du crochet.

Transcripts témoins pré-merge mentionnant `ce-simplify-code` (vols de
#2683 / #2654 / #2687, à rattacher par l'étape 3) : `500a76a5-…`,
`59ffa68e-…`, `42d8c2f8-…`.

## Méthode (prescrite, et bornée en tours)

**Règle de lecture, non négociable :** ne jamais lire un transcript en entier.
Ils vont jusqu'à 1,6 Mo et noieraient le contexte du vol. Uniquement
`grep -c`, `grep -l` et `grep -oh '…\{0,N\}'`.

1. **Recenser les vols candidats.** `gh pr list --repo senara-solutions/mika
   --state all --limit <N> --json number,createdAt,labels,title`, filtré sur
   `createdAt > "2026-10-14T20:11:06Z"` et label `origin:loop`. Vérifier que
   `--limit` n'a pas rempli la page : une page pleine ne se lit pas comme un
   compte réel (même précaution que l'exit 3 de `pr-origin-report.sh`).
2. **Dériver l'appartenance à la population allégée.** Un vol portait la
   consigne si et seulement si sa branche contient `6f1e5a4a` dans son
   historique — `_set_up_worktree` rebase sur `origin/main`, donc le merge-base
   est le fait observable qui remplace la lecture impossible du `CLAUDE.md`.
   Commande : `git merge-base --is-ancestor 6f1e5a4a <branche-du-vol>`.
   Le **vol d'essai** est la PR `origin:loop` la plus ancienne qui passe ce
   test. Candidat le plus ancien de la fenêtre : **#2703** (2026-10-14T21:09:36Z,
   mika#2697).
3. **Joindre transcript ↔ PR.** Le numéro de PR apparaît dans le transcript
   (sortie de `gh pr create`) : `grep -l '2703' <transcripts post-merge>`, puis
   confirmer par `grep -oh 'pull/2703[^"]\{0,20\}'`.
4. **Mesurer les quatre critères** sur le transcript du vol d'essai :
   - critère 1 — PR ouverte dans le budget : la PR existe (étape 1) et le
     transcript ne porte pas de marqueur de plafond atteint ;
   - critère 2 — `grep -c` des deux signatures calibrées, **accompagné de son
     contrôle positif** sur un témoin pré-merge ;
   - critère 4 — `grep -oh 'verify-pipeline[^"]\{0,40\}'` et la suite de tests,
     et leur **position avant** l'appel `gh pr create` (ordre, pas seulement
     présence) ;
   - ligne fixe `Code review: delegated to mika-qa (mika#2694)` dans le corps de
     la PR : `gh pr view <n> --json body`.
5. **Trancher le critère 3.** Si une invocation apparaît dans un vol qui passe
   l'étape 2, le relevé déclare le crochet **inerte** et nomme le levier
   suivant parmi ceux que le ticket envisage : une phrase de saut reconnue par
   la barrière de complétion du plugin, ou un réglage du plugin. Le levier est
   **nommé**, pas implémenté — son implémentation est un ticket de suivi.
6. **Relevé de couverture (critère 4 du ticket).** Comparer les trouvailles de
   mika-qa sur la PR allégée (`gh pr view <n> --comments`) à celles de simplify
   et de la revue sur les vols de #2683 et #2654, extraites de leurs
   transcripts par `grep -oh` autour des noms de sous-agents. Conclure si la
   perte annoncée (réutilisation, efficacité, correction non évidente) **s'est
   vue** — et dire franchement si l'échantillon d'un vol ne permet pas de le
   dire.

## Livrables

| livrable | nature |
|---|---|
| `docs/solutions/dev-loop/un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md` | document : le relevé de l'essai + la méthode calibrée et ses trois pièges mesurés |
| commentaire de clôture sur mika#2694 | le verdict de l'essai (critères 1–4), le levier suivant si inerte |

Le document porte le frontmatter YAML du dossier (`title`, `date`, `category`,
`module`, `component`, `problem_type`, `severity`), comme ses voisins.

**Chaque affirmation du relevé porte la commande qui l'a produite**, et toute
affirmation d'absence porte en plus son contrôle positif. C'est ce qui rend un
livrable documentaire falsifiable plutôt que déclaratif.

**Aucun détecteur n'est livré** — ni test, ni lint, ni garde CI. Le gate
Fire-Disposition (mika#2306) est donc N/A et la section n'est pas inventée. Un
détecteur de régression (un vol qui recommencerait à simplifier) serait utile
mais n'est pas demandé par ce ticket ; il est renvoyé au suivi ci-dessous
plutôt que glissé dans ce vol — gonfler la fin de pipeline est précisément ce
que ce ticket combat.

Tracked in: un ticket de suivi à ouvrir pour (a) le levier nommé à l'étape 5 si
le crochet est inerte, et (b) le détecteur de régression d'invocation.

## Verification contract

- `scripts/verify-pipeline.sh origin/main` vert.
- `cargo test -p mika-common --test claude_md_size` vert (inchangé : `CLAUDE.md`
  n'est pas touché par ce vol).
- La suite complète, `cargo clippy` et `cargo fmt --check` verts — aucun code
  n'est modifié, donc ils constatent l'absence de régression.
- Relecture du document : chaque nombre du relevé est reproductible par la
  commande citée à côté de lui.

## Definition of Done

- Le document de solution existe, porte le relevé des quatre critères et la
  méthode calibrée.
- Le vol d'essai est **désigné nommément** (numéro de PR, ticket, transcript),
  et son appartenance à la population allégée est justifiée par le merge-base.
- Le critère 3 est tranché : crochet respecté, ou inerte avec son levier nommé.
- Le relevé de couverture conclut, ou dit explicitement pourquoi il ne peut pas
  conclure sur un échantillon d'un vol.
- `CLAUDE.md` et `.claude/commands/mika.md` sont inchangés.

## Acceptance criteria

Transcrits du corps de mika#2694 :

- AC1 : `CLAUDE.md` porte le seuil de simplification et la délégation de la
  revue, en deux lignes au plus. `crates/mika-common/tests/claude_md_size.rs`
  reste vert. — **déjà satisfait par `6f1e5a4a` (PR #2695)** ; ce vol le
  constate sans y toucher.
- AC2 : essai borné, observé sur le premier vol allégé (transcript
  `~/.mika/data/pilot-transcripts/<task>.jsonl`, session principale) :
  1. le vol ouvre sa PR dans son budget de tours ;
  2. aucune invocation de `ce-simplify-code` ni de `ce-code-review` n'apparaît
     dans le transcript ;
  3. si l'une apparaît malgré la consigne, l'essai conclut que le crochet est
     inerte et nomme le levier suivant ;
  4. la vérification complète a bien tourné avant l'ouverture de la PR.
- AC3 : après l'essai, un relevé compare ce que mika-qa a signalé sur la PR
  allégée à ce que simplify et la revue signalaient sur les vols précédents
  (#2683, #2654). Il dit si la perte de couverture (réutilisation, efficacité,
  correction non évidente) s'est vue.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `docs/solutions/dev-loop/un-essai-dabsence-…-instrument.md` | 0 (sous `docs/`) |
| commentaire de clôture sur le ticket | 0 |

Total estimé : 0 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

Un seul vol, et il tient pour une raison qui n'est pas le volume : le livrable
est entièrement sous `docs/`, aucun fichier de code n'est modifié. Le total de
0 ligne est le compte réel, pas une estimation manquante — le ticket demande
une observation et un relevé, pas un changement de comportement (celui-ci a
déjà été livré par PR #2695).

Le risque de ce vol est en **tours**, non en lignes : la lecture de transcripts
de 1,6 Mo. Il est borné par la règle de lecture du § Méthode (jamais de lecture
entière ; `grep -c` / `-l` / `-oh` seulement) et par un échantillon fermé de
**trois** vols observés — un vol allégé et deux témoins (#2683, #2654).

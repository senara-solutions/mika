---
title: "Un interdit de prompt se dérive de l'échec mesuré, et se lit avec toutes les règles injectées dans le même PROMPT"
date: 2026-09-26
category: prompt-engineering
module: skills/bundled/_shared/dispatch-lib
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - "Vous ajoutez à un PROMPT de dispatch une règle qui interdit quelque chose à un pilote autonome (ne supprime pas, n'écris pas, n'utilise pas)"
  - "Le corps du ticket propose un remède (ex. fixture mktemp dans /tmp, ne pas nettoyer) sans que le log du pilote ait été relu"
  - "Plusieurs règles sont concaténées dans le même PROMPT par dispatch-lib (mika#2211 pr-body, mika#2306 Fire-Disposition, scratch mika#2548)"
  - "Un pilote a été tué par un [policy:deny] terminal après une série de refus non terminaux"
  - "Une assertion de test par sous-chaîne doit prouver qu'une règle ou un exclude est présent"
resolution_type: code_fix
related_components:
  - claude-pilot
  - permission-policy
  - wip-rescue
tags:
  - prompt-enforcement
  - loop-substrate
  - dispatch-lib
  - pilot-scratch
  - rule-contradiction
  - policy-deny
  - evidence-over-hypothesis
  - mika-2548
---

# Un interdit de prompt se dérive de l'échec mesuré, et se lit avec toutes les règles injectées dans le même PROMPT


## Contexte

Le 2026-09-26, la tâche pilote `83db3a82` (ready-label mika#2054) est morte sur un `rm -rf`. Le log `/var/log/claude-pilot/83db3a82-6d8d-4067-8527-29545419a63f.log` (lignes ~962-1169) donne la séquence exacte :

1. `mkdir -p verif-2054-redcheck/scripts …` à la racine du worktree : autorisé `[bash-mkdir]` (l. ~991).
2. `git show origin/main:scripts/verify-egress-no-log.sh -- > verif-2054-redcheck/… 2>/dev/null; wc -l …` : refusé `[bash-git-readonly] (non-terminal)` (l. ~1038). Le refus vient de la **forme** de la commande (`--`, `2>/dev/null`, enchaînement `;`), pas de sa cible. La seule forme de redirection que la politique accepte est `bash-git-show-redirect` : `git show <ref>:<chemin> > <chemin-relatif>` seul sur sa ligne (dépôt frère claude-pilot, `claude-pilot/src/claude_pilot/policies/permissions.yaml:315-319` — chemins relatifs à la racine du workspace mika-platform). La règle `bash-git-readonly` (`permissions.yaml:321-325`) reconnaît le préfixe `git show`, mais n'accorde rien à une redirection enchaînée.
3. Le pilote écrit « La redirection est refusée. J'utilise l'approche la plus fidèle… » (l. ~1067) et abandonne l'approche.
4. Il range : `rmdir …` est refusé deux fois (non-terminal, l. ~1110 et ~1135), puis `rm -rf verif-2054-redcheck` est refusé en `(terminal)` (l. ~1168). Session tuée.

Le corps du ticket proposait une autre histoire : un fixture `D=$(mktemp -d); cp … "$D/"; …; rm -rf "$D"`, et comme remède « passe par `/tmp` via mktemp, ne nettoie pas ». Le log contredit les deux moitiés :

- Le brouillon n'était pas dans `/tmp`. C'était un répertoire **dans le worktree**, et il était **vide** : toutes les écritures avaient été refusées. Git ne suit jamais un répertoire vide, donc le nettoyage ne répondait ni à un `git status` sale ni à la peur du wip-rescue (mika#1282). C'était un réflexe de rangement après avoir changé d'approche.
- `/tmp` ne peut pas accueillir un fixture. claude-pilot y refuse `cp`/`mv` (cpp#209 les rend seulement survivables) et y refuse aussi l'outil Write hors worktree (mika#2211). Seuls `mkdir /tmp/…` (`bash-mkdir-tmp-scratch`, `permissions.yaml:459-463`, cpp#143/cpp#150) et `cat > /tmp/… <<'EOF'` (`permissions.yaml:246-248`) y sont sanctionnés.

Une règle écrite d'après l'hypothèse du ticket (« utilise `/tmp` ») aurait envoyé le pilote vers un deuxième refus, et n'aurait jamais nommé la commande qui avait réellement échoué.

La première version du remède a ensuite introduit un second défaut, visible seulement en lisant le PROMPT en entier. Quatre relecteurs de `/ce:review` l'ont trouvé indépendamment : le nouvel interdit contredisait une règle sœur injectée juste avant lui dans le même PROMPT.

## Recommandation

Un interdit injecté dans un prompt de dispatch pour un pilote autonome (« ne fais pas X ») doit satisfaire deux conditions :

**(a) Il se dérive de l'échec mesuré, pas de l'hypothèse du ticket.** Lire le log du pilote, du premier refus jusqu'au refus terminal, et écrire la règle contre les commandes réellement émises. Deux conséquences :

- nommer un **lieu** où la chose est permise (forme positive d'abord), sinon le pilote improvise un emplacement, et c'est cet emplacement qu'il voudra nettoyer ensuite ;
- nommer la **forme exacte** que la politique accepte pour la commande qui a échoué, puisque c'est elle qui a déclenché l'abandon.

**(b) Il se lit avec toutes les règles sœurs du même PROMPT.** Les règles injectées dans un PROMPT forment un seul texte pour le pilote, et un pilote qui reçoit deux prescriptions contradictoires suit en général la plus récente. Avant de livrer un nouvel interdit, cherchez dans l'assemblage du PROMPT chaque verbe qu'il interdit et vérifiez qu'aucune règle voisine ne le prescrit :

```bash
# Toutes les injections dans le PROMPT, dans l'ordre
grep -n 'PROMPT=\$(printf' skills/bundled/_shared/dispatch-lib.sh
# Les verbes que le nouvel interdit bannit, dans le texte des règles sœurs
grep -n -E '^_[A-Z_]+_RULE=' skills/bundled/_shared/dispatch-lib.sh
grep -n -i -E 'supprime|\brm\b|rmdir|delete|efface' skills/bundled/_shared/dispatch-lib.sh \
  | grep -v -E '^[0-9]+:\s*#'
```

Pour chaque résultat qui appartient à une autre règle du PROMPT : soit la portée de l'interdit l'exclut, soit l'interdit la nomme comme exception explicite.

Le remède livré pour mika#2548 (branche `fix/2548/loop-substrate-le-prompt-dev-pilot-ne`, non mergée à la rédaction) applique les deux conditions :

- **Structurel.** `_seed_pilot_scratch_dir` (`skills/bundled/_shared/dispatch-lib.sh:2362-2371`) crée `.pilot-scratch/` dans chaque worktree de dispatch, le vide à chaque préparation derrière le garde de suppression mika#1943 (`_assert_removable_worktree_path`), et l'exclut via l'`info/exclude` du répertoire git **commun**, le mécanisme mika#1415 factorisé dans `_common_exclude_file` / `_append_exclude_line` (`dispatch-lib.sh:2282-2305`). Le vidage est nécessaire parce que les worktrees sont réutilisés et que `_clean_worktree_for_rebase` épargne les chemins exclus (commentaire `dispatch-lib.sh:2349-2355`). L'appel se fait dans `_set_up_worktree` (`dispatch-lib.sh:3236`).
- **Prompt.** `_PILOT_SCRATCH_RULE` (`dispatch-lib.sh:2636-2646`) est ajoutée à chaque PROMPT après la règle mika#2211 et avant la règle Fire-Disposition mika#2306 (`dispatch-lib.sh:3316`). Cette dernière doit rester la plus récente pour le groomeur.

## Pourquoi c'est important

- **Une règle écrite d'après l'hypothèse vise une commande que le pilote n'a jamais lancée.** Ici, « utilise `/tmp` » menait à un autre refus (cp/Write). La vraie cause d'abandon, un `git show` de forme refusée, restait sans réponse, et c'est ce qui déclenche le réflexe de rangement.
- **Une contradiction à l'intérieur du PROMPT a des effets concrets.** La première version interdisait `rm` sur « tout fichier temporaire », alors que la règle mika#2211 injectée juste avant prescrit « passe-le en `--body-file pr-body.md`, puis supprime-le » (`dispatch-lib.sh:2596-2598`). `pr-body.md` n'est gitignoré que dans mika (`.gitignore:90`), pas dans mika-cloud ni mika-skills, et `RESCUE_EXCLUDE_PATHSPEC` ne l'exclut pas (`dispatch-lib.sh:4164`). Un pilote qui obéit à la règle la plus récente laisse `pr-body.md` à la racine, et le rescue `git add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"` (`dispatch-lib.sh:4485`) le commite sur la branche de la PR.
- **Ni le test ni la relecture de la règle isolée ne voient cette classe de défaut.** Chaque règle est cohérente seule. La contradiction n'apparaît qu'en lisant le texte assemblé.
- **Limite connue.** Le réflexe de rangement sur un arbre vide n'est bloqué côté mika que par le texte du prompt. D'après la mémoire de session (auto memory [claude]), l'application par prompt seule échoue au substrat de la boucle : mika#2120 a donné 9 récurrences sous prompt contre 0 avec une barrière structurelle. La fermeture structurelle revient à la politique claude-pilot, qui devra rendre `rm`/`rmdir` sous `<worktree>/.pilot-scratch/` survivables. C'est un suivi nommé, hors périmètre ici (`dispatch-lib.sh:2356-2359`).

## Quand l'appliquer

- Chaque fois qu'une règle `_…_RULE` est ajoutée ou modifiée dans `dispatch-lib.sh`, surtout une règle négative (« ne … jamais »).
- Chaque fois que le ticket propose un remède déduit du symptôme (« le pilote a fait X, dites-lui de faire Y ») sans citer le log du pilote. Lire d'abord le log, du premier refus au refus terminal.
- Pour lire le log : l'identifiant de règle d'un refus se trouve sur la **dernière** ligne de l'événement. Le « Halt event » cité par dispatch-lib reprend le **premier** refus de la session, pas le refus terminal (auto memory [claude]). Dans cet incident, le premier refus était un `[bash-grep] (non-terminal)` (un `echo … && cat .compound-engineering/config.yaml`, l. 719), sans rapport avec la cause de la mort.
- Plus largement, quand plusieurs textes arrivent au même lecteur par le même canal (prompt système, entry command, blocs injectés) : l'ensemble est un seul texte.

## Exemples

**Interdit : avant/après (dans le même PROMPT que mika#2211).**

Première version (commit intermédiaire de la branche) : interdit sans portée, qui contredit la règle sœur.

```text
Ne supprime JAMAIS un brouillon, même vide, même en changeant d'approche : pas de `rm`, pas de `rmdir` (refusé),
et surtout pas de `rm -rf` — ce refus est TERMINAL et tue la session sur le coup. [...]
Pour un fixture, une copie de test ou tout fichier temporaire : crée-le sous `.pilot-scratch/<nom>/` [...]
```

Règle sœur injectée juste avant (mika#2211) :

```text
écris le corps dans un fichier SOUS le worktree (`pr-body.md` à sa racine),
passe-le en `--body-file pr-body.md`, puis supprime-le.
```

Version livrée (`dispatch-lib.sh:2636-2646`) : portée limitée à `.pilot-scratch/`, exception nommée, forme `git show` acceptée donnée explicitement.

```text
Pour y extraire un fichier d'une autre révision :
`git show <ref>:<chemin> > .pilot-scratch/<chemin>`, SEUL sur sa ligne — sans `--`, sans `2>/dev/null`, sans `;` ni `&&`
(toute autre forme est refusée). [...]
Ne supprime JAMAIS un brouillon de `.pilot-scratch/`, même vide, même en changeant d'approche : pas de `rm`, pas de
`rmdir` (refusé), et surtout pas de `rm -rf` [...]
Seule exception : `pr-body.md` à la racine n'est pas un brouillon — la règle mika#2211 ci-dessus reste entière
(écris-le à la racine, puis un simple `rm pr-body.md` après `gh pr create`, jamais `rm -rf`).
```

**Commande réellement refusée, et forme acceptée.**

```text
# refusée [bash-git-readonly] (non-terminal) — à cause de la forme
git show origin/main:scripts/verify-egress-no-log.sh -- > verif-2054-redcheck/scripts/verify-egress-no-log.sh 2>/dev/null; wc -l …
# acceptée [bash-git-show-redirect] (permissions.yaml:315-319)
git show origin/main:scripts/verify-egress-no-log.sh > .pilot-scratch/scripts/verify-egress-no-log.sh
```

**Leçon secondaire : fidélité des sondes de test.** Le bloc mika#2548 de `skills/bundled/_shared/test-dispatch-lib.sh` (l. 6108-6232, sonde à jetons l. 6108-6183) émet des jetons que vérifie `assert_contains`, qui fait un test de sous-chaîne. Au premier jet, un contrôle négatif (retrait de l'exclusion) a laissé une assertion verte sans le correctif, parce que le jeton `clean;` est une sous-chaîne de `commands_still_clean;` (commentaire `test-dispatch-lib.sh:6174-6175`). Les jetons ont été renommés (`scratch_invisible;`, `outside_visible;`, `cmds_seed_ok;`…) pour qu'aucun ne soit sous-chaîne d'un autre, et le contrôle négatif a ensuite été vu rouge. C'est une classe connue : une sonde doit avoir un contrôle positif **et** un contrôle négatif (auto memory [claude]).

## Voir aussi

- `docs/solutions/prompt-engineering/2026-09-07-une-garde-de-prompt-doit-vivre-sur-le-canal-que-lexecutant-lit.md` — la règle sœur mika#2211 que la première version contredisait.
- `docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md` — le mécanisme `info/exclude` du répertoire commun (mika#1415), réutilisé pour `.pilot-scratch/`.
- `docs/solutions/cross-repo-patterns/garde-suppression-worktree-allowlist-2026-09-20.md` — le garde de suppression mika#1943 derrière lequel le scratch est vidé.
- `docs/solutions/prompt-enforcement-structural-guards.md` — pourquoi la moitié prompt ne suffit pas seule.
- `docs/solutions/dev-loop/investigation-hypothesis-refuted-by-its-own-date-2026-09-05.md` — même classe : une hypothèse de ticket réfutée par la preuve.
- Plan : `docs/plans/2026-09-26-2137-fix-2548-pilot-scratch-sans-rm-plan.md`.

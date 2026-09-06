---
title: Une garde de prompt doit vivre sur le canal que l'exécutant lit, pas sur celui de son donneur d'ordre
date: 2026-09-07
last_updated: 2026-09-07
category: prompt-engineering
module: skills/bundled/_shared/dispatch-lib
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Vous ajoutez une règle de comportement pour un processus dispatché (claude-pilot, sous-agent, worker)
  - Un plan vous dit d'écrire une garde « transversale » dans un `system_prompt.md`
  - Vous devez décider entre le prompt de l'agent qui dispatche et le prompt de celui qui exécute
  - Une instruction correcte pour le cas court ne dit rien du cas long, et l'exécutant improvise
---

# Une garde de prompt doit vivre sur le canal que l'exécutant lit

## Le problème

Le pilote de la boucle sortait **dirty-sans-PR** à chaque dispatch. Cause racine dans
le `.stderr` de la session #2195 :

```
[policy:deny] Write: /tmp/pr-body-2195.md (non-terminal)
```

Puis, verbatim : « Le corps de PR est prêt (je n'ai pas pu l'écrire sur disque,
l'écriture hors worktree étant refusée par la politique) … Dis-moi si tu veux que
je le colle tel quel pour copie. »

La chaîne : le pilote compose un corps de PR long → il **dévie de lui-même** vers
`--body-file /tmp/pr-body-<N>.md` (réflexe outillage ; **aucun prompt ne le lui
demandait**) → la permission-policy refuse toute écriture hors worktree →
`gh pr create --body-file` ne trouve pas le fichier → le pilote **pose une
question** et se termine sans PR → dispatch-lib voit worktree dirty + zéro PR →
recovery mika#1282 → **draft wip-rescue** au lieu d'une PR propre. PR #2202 et
#2210 sont nées comme ça, de cette seule cause.

## Les deux leçons

### 1. Une instruction juste pour le cas court ne protège pas le cas long

`.claude/commands/mika.md` disait `gh pr create … --body "<body>"` — corps inline.
Exact, et suffisant tant que le corps tient sur une ligne. Le prompt ne disait rien
du corps long, donc l'exécutant a comblé le trou tout seul, avec le réflexe le plus
courant de son outillage. **Le silence d'un prompt n'est pas une contrainte ; c'est
une invitation à improviser.** Une instruction qui ne couvre qu'un régime doit
nommer l'autre, ou elle sera complétée par une invention.

Corollaire sur la forme de remplacement : dire seulement « pas `/tmp` » ne suffit
pas non plus — l'exécutant doit encore inventer autre chose, et l'invention
suivante (`--body-file - <<'BODY'`) casse dès qu'un corps généré contient sa propre
ligne délimitrice, ce qu'un corps plein de blocs de code et de titres peut faire.
**Énoncez la forme qui marche avant l'interdiction.**

### 2. Une garde dans le prompt du donneur d'ordre n'atteint pas l'exécutant

Le plan groomé demandait la garde « transversale » dans
`skills/bundled/self-dev/system_prompt.md`, en affirmant que c'est « le prompt
PILOTE » et qu'elle couvrirait donc les pilotes de tous les repos. **C'est faux, et
c'est vérifiable en une ligne** :

```bash
_run_pilot_sandboxed claude-pilot --verbose --log-dir --task-id "$LOG_ID" \
    --command "$ENTRY_COMMAND" $CWD_ARGS -- "$PROMPT"
```

Le processus pilote reçoit **exactement deux entrées** : la commande d'entrée,
résolue depuis le worktree du repo **cible** (donc le `.claude/commands/mika.md` de
ce repo), et le `PROMPT` composé par `dispatch-lib.sh`. `self-dev/system_prompt.md`
est le system prompt de **mika-dev**, l'agent qui *dispatche* — il ne franchit
jamais la frontière de processus.

Une garde qui n'y vivrait que là aurait été **verte à la relecture et inopérante en
production** : le fichier contient bien la phrase, la revue coche l'AC, et le pilote
ne la lit jamais. C'est la même famille que la garde structurelle de mika#2205 — un
test du résolveur seul serait resté vert pendant toute la panne, parce que le défaut
n'était pas la résolution mais l'appelant qui ne résolvait pas.

## La règle

**Avant d'écrire une garde de prompt, tracez l'invocation et répondez à : quels
octets ce processus lit-il réellement ?** Puis placez la règle sur ce canal-là.
S'il y en a plusieurs, chacun ne couvre que ce qu'il couvre — dites-le explicitement
plutôt que de laisser un seul emplacement porter une prétention de transversalité
qu'il n'honore pas.

Ici, trois emplacements, chacun pour ce qu'il couvre :

| Emplacement | Lu par | Couvre |
|---|---|---|
| `mika/.claude/commands/mika.md` | le pilote, via `--command` | les dispatches sur `mika` |
| `skills/bundled/self-dev/system_prompt.md` (Rule 12) | mika-dev | ce que **mika-dev compose** (`iteration_context`, free-text) |
| `dispatch-lib.sh` `_PR_BODY_CONTAINMENT_RULE` → `PROMPT` | le pilote, via `-- "$PROMPT"` | **tous les repos**, y compris `mika-cloud` / `mika-skills` dont le `mika.md` porte encore l'instruction d'avant le fix |

## Le durcissement qui compte plus que la suppression

Le plan prévoyait d'écrire `pr-body.md` sous le worktree puis de le supprimer après
`gh pr create`. La suppression seule laisse un trou : **sur le chemin d'échec, elle
n'a pas lieu.** Si `gh pr create` échoue, le fichier survit, `git status --porcelain`
— la sonde exacte que dispatch-lib interroge — le rapporte, et la recovery
wip-rescue mika#1282 se déclenche : exactement le symptôme que ce fix existe pour
tarir.

`/pr-body.md` est donc au `.gitignore`. `git status --porcelain` ne rapporte pas les
fichiers ignorés, donc le résidu est inoffensif **sans dépendre d'un `rm` qui ne
s'exécute pas quand ça se passe mal**. Généralisation : *quand un nettoyage protège
un invariant, demandez ce qu'il advient de l'invariant sur le chemin où le nettoyage
ne tourne pas.*

## Ce qui a été refusé, et pourquoi

- **Élargir la permission-policy pour autoriser `/tmp/pr-body-*`** (direction #3 du
  ticket) — élargit la surface d'écriture hors worktree pour un gain nul. Le
  confinement hors-worktree est une garde voulue ; la panne est que le pilote s'y
  cogne, pas que la garde soit trop stricte.
- **Le heredoc `--body-file - <<'BODY'`** (direction #2, recommandée dans le
  ticket puis en prep-grooming) — écarté au first-pass architecte : fragile au
  contenu. Un fichier est insensible au contenu.
- **Mettre à niveau les `mika.md` de `mika-cloud` / `mika-skills`** — hors périmètre
  d'un fix mika-centré ; la ligne dans `dispatch-lib.sh` les couvre déjà, et c'est
  précisément pourquoi elle vaut d'exister.

## Détection

Le filet reste mika#1282 : un pilote qui sort dirty-sans-PR produit un draft
`wip-rescue`, ce qui rend la régression visible sans sonde dédiée. Signal direct
dans le `.stderr` d'une session pilote :

```bash
grep 'policy:deny.*pr-body' /var/log/claude-pilot/*.stderr
```

Attente en régime nominal : **zéro ligne**. Garde structurelle en CI :
`skills/bundled/_shared/test-dispatch-lib.sh`, section mika#2211 — elle vérifie que
la règle existe, qu'elle est injectée **dans `PROMPT`** (pas seulement présente dans
un fichier quelque part), qu'elle est **appendue** après la réassignation
`ITERATION_CTX` (une injection au-dessus serait écrasée silencieusement à chaque
itération), et qu'elle nomme la forme qui marche avant l'interdiction.

## Références

- Ticket : mika#2211
- Panne mesurée : session #2195, stderr `675479e5-…` ; PR #2202 (de #2192), #2210 (de #2195)
- Filet en place : mika#1282 (recovery dirty-worktree → draft wip-rescue)
- Même famille de défaut : mika#2205 (un accesseur étroit à côté du résolveur canonique)
- Site d'injection voisin : mika#2178 (le texte du ticket atteint le pilote) — les trois
  invariants de position y sont documentés et s'appliquent aussi ici

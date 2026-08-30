---
title: "Retirer le drapeau --trace mort de dispatch-lib et fermer la classe des drapeaux inconnus"
date: 2026-08-30
issue: senara-solutions/mika#2043
branch: bug/2043/dispatch-dispatch-lib-passe-trace-claude
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

## Goal Capsule

**Objectif.** `dispatch-lib.sh` ne construit plus aucun drapeau que le CLI de `claude-pilot` refuse, et un test structurel empêche qu'un tel drapeau réapparaisse.

**Moyen retenu.** Retirer `TRACE_FLAG` et le commentaire qui invite à l'armer ; le remplacer par un commentaire qui dit exactement ce qui est loggé et sous quelle condition — l'essentiel l'étant sans aucun drapeau ; corriger les deux documents de solutions qui décrivent `--trace` comme livré ; ajouter à `test-dispatch-lib.sh` une garde à deux passes sur les drapeaux.

**Hiérarchie d'autorité.** Le corps de mika#2043 fixe les trois critères d'acceptation. Là où le ticket et la mesure divergent (le mécanisme d'échec supposé), la mesure prime et la divergence est écrite dans la PR — c'est une exigence explicite du dispatch.

**Conditions d'arrêt.** Ne pas implémenter `--trace` dans `claude-pilot` : le besoin qui l'a motivé est couvert (voir KTD1), le dépôt est séparé et hors périmètre de ce ticket. Ne toucher à aucun autre ticket (ce ticket porte `dispatch:mpc`).

## Product Contract

### Le fait, mesuré

`dispatch-lib.sh:1437-1441` (sur `origin/main` au 2026-08-30 ; le ticket cite 1206-1210, le fichier a bougé depuis) construit un drapeau `--trace` et l'interpole dans l'invocation du pilote à la ligne 1458 :

```sh
# --trace flag for full event-stream capture (mika#1097 Step 0-B).
# Enabled via CLAUDE_PILOT_TRACE env var (set per-skill in the case switch below).
local TRACE_FLAG=""
if [ "${CLAUDE_PILOT_TRACE:-}" = "1" ] || [ "${CLAUDE_PILOT_TRACE:-}" = "true" ]; then
    TRACE_FLAG="--trace"
fi
```

`claude-pilot` n'accepte pas `--trace`, et ne l'a jamais accepté :

- Le parseur `_build_parser()` (`claude-pilot/src/claude_pilot/cli.py:37-78`) ne déclare aucun `--trace`. Le CLI a bougé le matin même (cpp#124 et cpp#126 mergées, binaire réinstallé sur PATH à 08:03 après être resté figé au 15 août) et a gagné `-i/--interactive` (cpp#69) — vérification faite sur le parseur d'aujourd'hui, pas sur la mémoire du ticket.
- `git log -S'"--trace"' --all` et `git log -S"log_assistant_block" --all` sur le dépôt `claude-pilot` ne rendent **aucun** commit. Le drapeau n'a pas été retiré : il n'a jamais été écrit. Le Step 0-B de mika#1097 (`docs/plans/2026-05-13-003-bug-dev-groom-claude-pilot-exits-success-without-architect-plan.md:49`) n'a livré que sa moitié `dispatch-lib`.

### Ce que fait un drapeau inconnu — la vraie question du ticket

Mesure faite en important le parseur **réel** du paquet installé sur PATH (`~/.local/share/uv/tools/claude-pilot/bin/python`), avec l'`argv` exact que construit `dispatch-lib.sh:1458` :

| argv | résultat |
|---|---|
| `--verbose --log-dir --task-id T1 --command /mika --trace --cwd /tmp -- "PROMPT"` | `SystemExit(2)`, stderr : `claude-pilot: error: unrecognized arguments: --trace` |
| le même sans `--trace` | parse OK (`task_id`, `command`, `log_dir` corrects) |

Le positionnel `prompt` est `nargs=argparse.REMAINDER` (`cli.py:77`) — on pouvait craindre qu'il avale le drapeau en silence. **Il ne l'avale pas**, parce que `$TRACE_FLAG` est interpolé *avant* le `--` séparateur.

**Conséquence sur la gravité.** Ce n'est pas la classe « on croit demander une trace et on n'en obtient jamais, sans que personne le voie » — celle que nous fermons partout cette semaine. C'est la classe inverse et plus brutale : la première personne qui suit l'invitation du commentaire et arme `CLAUDE_PILOT_TRACE=1` tue **tous** les dispatches du skill concerné à `exit 2`, avant tout démarrage de session, zéro commit, zéro PR. Le ticket a raison sur la mine, se trompe sur le mécanisme. Le symptôme serait visible — mais il ressemblerait à ce que l'opérateur cherchait justement à diagnostiquer : un pilote qui ne produit rien.

Personne n'arme le drapeau aujourd'hui : `grep -rn "CLAUDE_PILOT_TRACE"` sur `mika/` et `mika-skills/` ne rend que les trois lignes mortes ci-dessus. Rien n'est cassé en ce moment.

### Requirements

- R1. `dispatch-lib.sh` ne construit plus le drapeau `--trace`, et le commentaire qui invitait à l'armer disparaît avec lui.
- R2. Le commentaire de remplacement dit la voie de capture réellement disponible et **ce qu'elle couvre exactement**, en distinguant l'inconditionnel de ce qu'un drapeau ajoute, pour que la question ne se repose pas — et pour ne pas remplacer une affirmation fausse sur `claude-pilot` par une autre.
- R3. Les documents de solutions qui décrivent `--trace` comme un livrable sont corrigés — ce sont eux qui ont produit la mine.
- R4. Une garde structurelle empêche `dispatch-lib` de construire un drapeau que le CLI de `claude-pilot` ne connaît pas, et empêche cette garde elle-même de dériver en une croyance non vérifiée.
- R5. La PR énonce le mécanisme mesuré (échec bruyant à `exit 2`, pas ignoré en silence), et dit si la trace était utile et par quoi elle est remplacée.

### Acceptance examples

- AE1. `grep -c TRACE_FLAG skills/bundled/_shared/dispatch-lib.sh` rend `0` : plus aucun code ne construit le drapeau. `CLAUDE_PILOT_TRACE` ne subsiste que dans le commentaire de remplacement, qui *dissuade* de le remettre — l'inverse d'une invitation, et la réponse que trouvera quiconque grep ce nom.
- AE2. `bash skills/bundled/_shared/test-dispatch-lib.sh` passe, garde comprise.
- AE3. La garde est prouvée dans le sens positif sur quatre réintroductions distinctes, chacune devant faire rougir la suite : drapeau inconnu littéral ; drapeau inconnu via variable interpolée (la forme exacte du bug) ; sa forme conditionnelle d'origine ; liste blanche périmée. Plus une cinquième : extraction cassée, qui doit produire un échec et non une disparition silencieuse.
- AE4. `grep -rn -- '--trace' docs/solutions/` ne rend plus aucune ligne présentant le drapeau comme disponible. Les lignes attendues sont les corrections mika#2043 elles-mêmes et `best-practices/flag-semantics-…-2026-04-27.md:99`, qui cite `--trace` comme exemple générique de nom de drapeau — zéro ligne n'est pas le résultat attendu.

## Planning Contract

### KTD1 — Retirer le drapeau plutôt qu'implémenter `--trace`

Le ticket demande de trancher dans un sens ou dans l'autre. **On retire.** Le besoin qui a motivé Step 0-B était réel — voir les blocs de contenu et l'événement `init` pour diagnostiquer les sessions à zéro artefact (`docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md`) — et **l'essentiel est aujourd'hui couvert sans aucun drapeau**.

Point mesuré, et corrigé après revue : la première rédaction de ce plan attribuait cette couverture à `--verbose`. C'est faux, et c'était la faute même que ce ticket ferme, reproduite dans le correctif. Les writers de `ui.py` passent tous par `write_log` (`logger.py:39-43`), qui n'est jamais gardé ; le puits fichier existe parce que `dispatch-lib` passe `--log-dir` (`cli.py:216-219`). Les seuls `if verbose:` du chemin de session sont `agent.py:233` et `agent.py:248`.

| Besoin de Step 0-B | Couverture actuelle | Conditionné ? |
|---|---|---|
| blocs de contenu texte de chaque `AssistantMessage` | `log_text` (`agent.py:312`) | non — toujours |
| tours qui ne produisent rien d'observable | `log_turn_summary` (cpp#10, `agent.py:567`) | non — toujours |
| type de message SDK tombant hors de la boucle | `log_unhandled_message` (cpp#123, `agent.py:418`) | non — toujours |
| flux d'événements | `log_verbose` : **type** du StreamEvent, et une ligne fixe sur `UserMessage`/résultat d'outil (cpp#125, `agent.py:233,248`) | oui — `--verbose` |

**Restes non couverts, à dire honnêtement.** Des quatre sortes de blocs que Step 0-B nommait :

- `thinking` — aucun contenu loggé nulle part (seul le drapeau `had_thinking_block` de `guardrails.py` existe) ;
- `tool_result` — `log_verbose` n'émet qu'un marqueur d'arrivée à texte fixe, sans charge utile ;
- `tool_use` — atteignable seulement indirectement, par nom et détail, via `log_tool` / `log_tool_request` du chemin permissions.

Et l'OQ1 de Step 0-B (`session_id` vide / `model = "unknown"`) reste **ouverte** : `log_init` est la *source* de cette question, pas sa réponse — il imprime `model or "unknown"`, c'est-à-dire précisément la ligne ambiguë qui avait motivé la demande de `repr(SystemMessage)`. Distinguer « attribut absent » de « valeur unknown » n'est toujours pas possible.

Rien de tout cela ne justifie de porter un drapeau mort dans `dispatch-lib` en attendant : si l'un de ces besoins redevient concret, il se fiche sur `claude-pilot` comme un ticket à lui.

Second motif, structurel : `claude-pilot` est un dépôt séparé, CC-spawns-only, hors du périmètre de dispatch de ce ticket (`.claude/commands/mika.md` § sub-repo path claude-pilot). Étendre ce ticket jusqu'à lui serait un élargissement de périmètre, pas une correction.

### KTD2 — La garde a deux passes, et la seconde garde la première

Le ticket suggère « un test qui compare les drapeaux construits à `claude-pilot --help` ». Pris au pied de la lettre, ce test ne tourne que là où `claude-pilot` est installé. La CI lance cette suite (`.github/workflows/ci.yml:85` → `make test-dispatch-lib`) sur un runner partagé où la présence du binaire n'est pas garantie ; un test qui se contente de se sauter là-bas ne ferme rien.

D'où deux passes :

- **Passe A — hermétique, toujours exécutée.** Une liste blanche des drapeaux acceptés, codée dans le test, et l'assertion que tout drapeau construit par `dispatch-lib` y figure. Ferme la classe partout, y compris en CI sans binaire.
- **Passe B — conditionnelle à la présence de `claude-pilot`.** L'assertion que la liste blanche de la passe A est elle-même un sous-ensemble de ce que le CLI accepte réellement (`claude-pilot --help`).

La passe B est ce qui empêche la passe A de devenir exactement le défaut qu'on corrige : une affirmation sur l'interface de `claude-pilot`, écrite une fois, jamais reconfrontée au monde, et fausse sans que personne le voie. Sans elle, on remplacerait une croyance non vérifiée par une autre. Quand `claude-pilot` est absent, la passe B se saute **en l'annonçant**, jamais en silence.

### KTD3 — Extraction des drapeaux : littéraux plus variables interpolées

C'est précisément par une variable interpolée (`$TRACE_FLAG`) que le drapeau mort est entré. Une garde qui ne lirait que les littéraux de la ligne d'invocation ne l'aurait pas attrapé. L'extraction couvre donc :

1. les tokens `--flag` littéraux sur les lignes d'invocation de `claude-pilot` (1473 et 3313 après correction ; 1458 et 3298 avant) ;
2. les tokens `$VAR` de ces mêmes lignes, résolus en cherchant les assignations `VAR=...` dans le fichier et en extrayant les `--flag` qu'elles contiennent.

Après correction, la seule variable restante est `CWD_ARGS` (lignes 1369-1373, 1396), qui porte `--cwd` et `--relay-config` — tous deux valides. Seuls les drapeaux **longs** sont extraits : élargis aux formes courtes, le motif capte du bruit shell (`"${LOG_ID}-revise-$(date +%s)"` rend un « drapeau » `-revise-`), et une garde qui crie au loup finit désarmée. `dispatch-lib` ne passe que des drapeaux longs.

Le style suit celui de la suite existante : lecture du **source** de `dispatch-lib.sh` par `grep`/`sed`/`awk` et assertions dessus, sans exécuter le dispatch (`test-dispatch-lib.sh:1-13`).

### Assumptions

- A1. La liste blanche est dérivée du parseur d'aujourd'hui (`cli.py:44-77`) : `--task-id`, `--no-relay`, `--relay-config`, `--cwd`, `--log-dir`, `--command`, `--verbose`, `-i`/`--interactive`, `--max-turns`, `--max-budget`, `--stall-threshold`, `--empty-threshold`, `--idle-timeout`, `--min-detection-turns`, `--no-guardrails`, plus `-h`/`--help`. La passe B est ce qui la tient à jour.
- A2. `--` (le séparateur avant le prompt) n'est pas un drapeau et est exclu de l'extraction.

## Implementation Units

### U1. Retirer `TRACE_FLAG` de `dispatch-lib.sh` et écrire le commentaire de remplacement

Fichier : `skills/bundled/_shared/dispatch-lib.sh` (lignes 1437-1441 et 1458).

Supprimer le bloc `TRACE_FLAG` en entier, commentaire compris, et retirer `$TRACE_FLAG` de l'invocation. À la place, un commentaire qui dit **exactement** ce qui est loggé et sous quelle condition, selon le tableau de KTD1 — l'inconditionnel annoncé comme tel, `--verbose` crédité des deux seuls marqueurs qu'il ajoute, et les restes non couverts nommés. Le lecteur qui cherche un instrument de diagnostic doit tomber sur la réponse juste, pas sur un chemin qui ne mène nulle part (R2) ni sur une nouvelle approximation. Mentionner mika#2043 pour l'ancrage.

Sert R1, R2.

### U2. Corriger les documents qui décrivent `--trace` comme livré

- `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` — lignes 47, 49 et 121 présentent le drapeau comme un livrable de la correction. Corriger en disant ce qui a réellement été livré et ce qu'il faut utiliser aujourd'hui.
- `docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md:53` — « Existing guard: `--trace` diagnostic instrumentation » s'appuie sur le doc précédent. Corriger de même.

Ces documents sont la cause amont : la mine existe parce qu'un document a enregistré une intention comme un fait. Les laisser en l'état ferait se reposer la question, ce que l'AC#2 du ticket interdit explicitement.

Sert R3.

### U3. Ajouter la garde à deux passes dans `test-dispatch-lib.sh`

Fichier : `skills/bundled/_shared/test-dispatch-lib.sh` (nouvelle section, en fin de suite avant le résumé).

Extraction selon KTD3, puis :
- passe A : chaque drapeau extrait appartient à la liste blanche (échec nommant le drapeau fautif) ;
- passe B : si `command -v claude-pilot` réussit, chaque drapeau de la liste blanche apparaît dans `claude-pilot --help` ; sinon, ligne de saut explicite.

Sert R4.

## Verification Contract

```bash
# La suite de tests, garde comprise (CI lance la même chose via make test-dispatch-lib)
bash skills/bundled/_shared/test-dispatch-lib.sh

# AE1 — plus aucun code ne construit le drapeau
grep -c TRACE_FLAG skills/bundled/_shared/dispatch-lib.sh          # attendu: 0
grep -c CLAUDE_PILOT_TRACE skills/bundled/_shared/dispatch-lib.sh  # attendu: 1 — le commentaire dissuasif

# AE4 — plus aucun document ne présente --trace comme disponible.
# Attendu: uniquement les corrections mika#2043 elles-mêmes, plus
# best-practices/flag-semantics-…-2026-04-27.md:99 qui cite `--trace` comme
# exemple générique de nom de drapeau d'observabilité — pas comme un drapeau
# de claude-pilot. Zéro ligne n'est PAS le résultat attendu.
grep -rn -- '--trace' docs/solutions/

# Le reste de la chaîne qualité inchangé
make verify-bundled-skills
bash scripts/verify-pipeline.sh origin/main
```

**Vérification positive de la garde (AE3), obligatoire.** Réintroduire tour à tour chaque forme du défaut, relancer la suite, constater l'échec, annuler. Un test vert n'établit rien tant qu'on n'a pas vu qu'il sait rougir — c'est la même discipline qui a fait trouver ce bug.

**Elle a payé.** La première version de la garde était verte et ne détectait pas le cas D2 — la réintroduction de `TRACE_FLAG`. Deux défauts, trouvés seulement en la faisant échouer :

1. **Ancrage trop étroit.** Le motif exigeait un espace ou un début de ligne avant le `-`. Or `dispatch-lib` écrit `CWD_ARGS="--cwd $DIR"` et le drapeau mort était `TRACE_FLAG="--trace"` — tous deux collés à `="`. La garde ratait donc `--trace`, et ratait déjà `--cwd` à l'état propre, tout en affichant vert. Corrigé en ancrant sur tout caractère non-mot ; en élargissant, les drapeaux courts ont produit du bruit (`"${LOG_ID}-revise-$(date +%s)"` donne un « drapeau » `-revise-`), d'où la restriction aux drapeaux longs — les seuls que `dispatch-lib` passe.
2. **Disparition silencieuse.** Sous `set -euo pipefail`, un `grep` sans correspondance tue la suite entière. En cassant le motif d'invocation exprès, la suite s'arrêtait en plein milieu, sans résumé et sans échec — muette précisément le jour où la forme de l'invocation change, c'est-à-dire le jour où la garde compte. Corrigé par `|| true` sur chaque capture, l'assertion de comptage transformant une extraction vide en échec.

Les deux auraient survécu à une relecture. Aucun n'a survécu à la mesure.

## Definition of Done

- [ ] `TRACE_FLAG` a disparu de `dispatch-lib.sh`, invocation comprise ; `CLAUDE_PILOT_TRACE` ne subsiste que dans le commentaire dissuasif, qui est ce que trouvera quiconque grep ce nom.
- [ ] Le commentaire de remplacement dit **exactement** ce qui est loggé et sous quelle condition — ce qui est inconditionnel l'est dit comme tel, et `--verbose` n'est crédité que des deux marqueurs qu'il ajoute réellement.
- [ ] Les deux documents de solutions ne décrivent plus `--trace` comme livré.
- [ ] La garde à deux passes est dans `test-dispatch-lib.sh` et la suite passe.
- [ ] La garde a été vue échouer sur un drapeau inconnu introduit exprès, puis la modification annulée.
- [ ] `make verify-bundled-skills` passe.
- [ ] La PR énonce le mécanisme mesuré (`exit 2`, bruyant) et la couverture de remplacement — en distinguant l'inconditionnel de ce que `--verbose` ajoute, et en nommant les restes : `thinking`, `tool_result`, `tool_use` brut, et l'OQ1 toujours ouverte.
- [ ] La PR porte `mika-platform-qa` comme reviewer.

## Acceptance criteria

- [ ] `dispatch-lib` ne construit plus le drapeau `--trace` ; le commentaire qui invitait à l'armer est retiré avec lui (AC1 du ticket).
- [ ] Le commentaire de remplacement dit ce qu'il faut utiliser à la place pour capturer le flux d'événements (AC2 du ticket).
- [ ] Une garde empêche `dispatch-lib` de passer à `claude-pilot` un drapeau que son CLI ne connaît pas, et cette garde est vérifiée dans le sens positif (AC3 du ticket).
- [ ] Les documents de solutions qui présentaient `--trace` comme livré sont corrigés, sans quoi la question se repose (AC2, second volet).
- [ ] La PR dit lequel des deux comportements a été mesuré pour un drapeau inconnu — ignoré en silence, ou échec au lancement — et ce que la réponse change à la gravité du ticket.
- [ ] La PR dit si la trace était utile et, si oui, par quoi elle est remplacée.

## Appendix

**Reproduction de la mesure du point central** (parseur réel du paquet installé, sans démarrer de session) :

```bash
~/.local/share/uv/tools/claude-pilot/bin/python -c "
import sys; sys.argv=['claude-pilot']
from claude_pilot.cli import _build_parser
p=_build_parser()
try:
    p.parse_args(['--verbose','--log-dir','--task-id','T1','--command','/mika','--trace','--cwd','/tmp','--','PROMPT'])
    print('PARSED OK')
except SystemExit as e:
    print('SystemExit', e.code)
"
# → claude-pilot: error: unrecognized arguments: --trace
# → SystemExit 2
```

**Origine.** mika#1097 Step 0-B, décrit dans `docs/plans/2026-05-13-003-bug-dev-groom-claude-pilot-exits-success-without-architect-plan.md:49`. Trouvé le 2026-08-29 en cherchant un instrument pour diagnostiquer mika#2029 — troisième outil d'observation absent ou muet de la même nuit, avec mika#2030 et mika#2040.

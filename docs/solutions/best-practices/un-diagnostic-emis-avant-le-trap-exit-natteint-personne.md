---
title: "Un diagnostic émis avant l'armement du trap EXIT n'atteint personne"
module: skills/bundled/_shared/dispatch-lib.sh
date: 2026-10-02
problem_type: best_practice
category: best-practices
component: tooling
severity: high
ticket: mika#2634
root_cause: logic_error
resolution_type: code_fix
related_components:
  - crates/mika-agent/src/skills/executor.rs
  - skills/bundled/self-dev-callback
tags: [dispatch-lib, loop-substrate, exit-trap, pre-flight, smoke-test, claude-pilot, producer-stamp, set-x-trace, mika2634, mika1200]
applies_when:
  - Adding or moving a pre-flight check (smoke test, dependency probe, guard) in a dispatch-lib handler that exits on failure
  - "A handler dies with `Process Exit code: N:` and `stderr_bytes: 0` while the code visibly prints a diagnostic"
  - Relying on the EXIT trap's trace tail (`tail -50 $TRACE_FILE`) to carry a cause into the callback
  - Rewriting a launch line (`timeout N claude-pilot …`) into a capturing form
  - Choosing how the engine should tell "the launcher did not start" from "the pilot failed"
symptoms:
  - "Three dispatches dead at launch over 2 h 45, no alarm, `tasks.result = \"Process Exit code: 1: \"`"
  - "No `/var/log/claude-pilot/<task-id>.log`, `long_running_handler_exit_nonzero` with `task_was_terminal: false`"
  - "The mika#1200 diagnostic, written word for word for this failure, reached nobody"
---

# Un diagnostic émis avant l'armement du trap EXIT n'atteint personne

## Context

Le 2026-10-02 à 01:26Z, un `pip install -e` du dépôt claude-pilot a réécrit
`~/.local/bin/claude-pilot` avec le shebang du Python **système** :
`entry_points.txt` déclare `claude-pilot` en `console_scripts`, donc toute
installation pip-user éditable reprend ce chemin à `uv tool`, sans bruit. Chaque
lancement levait alors `ModuleNotFoundError: No module named 'claude_pilot'`.
Trois dispatches sont morts au lancement en 2 h 45, et aucune alarme n'a sonné.
MPC a trouvé la panne à la main (mika#2634).

Or dispatch-lib possédait **déjà** la sonde exacte de cette panne : le smoke test
mika#1200, `timeout 15 claude-pilot --help`, dont le message nommait la cause
(venv cassé) et la réparation (`uv tool install … --editable`). Elle a tiré trois
fois, et son diagnostic n'est arrivé nulle part. Le doc
[1200-editable-install-dep-sync-gap-pilot-crash.md](../1200-editable-install-dep-sync-gap-pilot-crash.md)
présente ce smoke test comme le filet structurel. C'était vrai pour la détection,
pas pour la livraison.

## Guidance

**1. Une vérification qui sort en échec doit tourner APRÈS `trap '_dispatch_lib_exit_trap' EXIT`, sauf si le trap lui-même en dépend.**

Le smoke test et son `exit 1` tournaient **avant** l'armement du trap (une quarantaine de lignes plus haut sur main avant mika#2634). Sur ce chemin, chaque maillon
de livraison était coupé :

| maillon | pourquoi il ne portait rien |
|---|---|
| callback | le trap n'était pas armé : pas de `mika ask` de complétion, la tâche reste non terminale |
| stderr du handler | `exec 9>>"$TRACE_FILE" 2>/dev/null`, en tête de `dispatch_claude_pilot`, est un `exec` **sans commande** : la redirection s'applique au shell courant, de façon permanente. Le fd 2 vaut `/dev/null` pour toute la suite du script, donc `cat >&2 <<EOF` écrivait dans le vide |
| moteur | `spawn_long_running_exec` voit seulement exit 1 + stderr vide, et écrit `Process Exit code: 1: ` |

Le fait « fd 2 = /dev/null » était déjà écrit dans le commentaire de
`_halt_family` (mika#903) et dans
[une-sonde-qui-fusionne-stderr…](une-sonde-qui-fusionne-stderr-dans-le-harnais-ne-voit-pas-le-puits-de-production-2026-09-21.md).
Personne n'avait fait le lien avec « le pré-flight écrit son diagnostic dans le
vide ». La réparation tient à l'ordre : il n'a manqué aucune instrumentation.

La partie difficile est de savoir **quoi ne pas déplacer**. Les trois `command -v`
(`jq`, `mika`, `claude-pilot`) restent avant le trap : le trap livre son callback
par `mika` et son corps emploie `jq`. Les placer après lui donnerait un trap qui
échoue en silence sur un hôte sans `jq`, et on échangerait une panne muette
contre une autre. Le smoke test n'a aucune de ces dépendances : c'est le seul
bloc qui puisse migrer. L'ordre devient : `command -v` ×3 → `_parse_input_json`
→ `trap EXIT` → smoke test (`_SMOKE_ERR_FILE`).

**2. Le trace tail du trap ne porte pas une cause écrite tôt : capturer dans un fichier dédié.**

Le plan voulait que `2>&9` suffise : le traceback va dans `$TRACE_FILE`, le trap
en ajoute les 50 dernières lignes au `RESULT` (bras `"HANDLER CRASH"*|"LAUNCHER DEAD"*` de `_dispatch_lib_exit_trap`). La mesure
pendant l'implémentation a montré le contraire. `set -x` est actif, donc la
fenêtre de 50 lignes se remplit avec la trace du bloc qui pose `RESULT` (~30
lignes selon le commentaire du correctif dans dispatch-lib) puis avec celle des
commandes du trap qui précèdent la lecture du fichier. Le traceback, écrit en premier, sort de
la fenêtre. AC3 aurait été livré cassé alors qu'il **semblait** livré.

**3. La forme de capture « naturelle » rend le site invisible à un scan de garde.**

`_ERR=$(timeout 15 claude-pilot --help 2>&1 >/dev/null)` paraît être la
correction évidente. Elle fait disparaître ce site du scan de surface des drapeaux
claude-pilot dans `test-dispatch-lib.sh` (mika#2043, ancre élargie par
mika#2165) : sa regex n'ancre `(if ! )?timeout N claude-pilot` qu'après `^` ou
`;`, et une substitution de commande n'est ni l'un ni l'autre. Le compte de sites
a baissé et le scan est devenu rouge, ce qui est son rôle : un point de lancement
réordonné doit rougir au lieu de s'évaporer. D'où la forme retenue, qui garde
`timeout` en position de commande :

```bash
_SMOKE_ERR_FILE="${TMPDIR:-/tmp}/mika-launcher-smoke-$$.err"
if ! timeout 15 claude-pilot --help >/dev/null 2>"$_SMOKE_ERR_FILE"; then
    _SMOKE_STDERR=$(tail -c 4000 "$_SMOKE_ERR_FILE" 2>/dev/null | _scrub_secrets_from_output)
    rm -f "$_SMOKE_ERR_FILE"
    : "${_SMOKE_STDERR:=(the launcher wrote nothing on its stderr)}"
    RESULT="LAUNCHER DEAD (exit ${_EXIT_LAUNCHER_DEAD}, mika#2634) — …
${_SMOKE_STDERR}
…
Outcome: LAUNCHER_DEAD"
    exit "$_EXIT_LAUNCHER_DEAD"
fi
rm -f "$_SMOKE_ERR_FILE"
```

**4. « Le lanceur n'a pas démarré » est un fait estampillé par le producteur, avec un code de sortie dédié, sans inférence côté moteur.**

Le ticket proposait d'inférer la panne : exit non nul **et** aucun journal
claude-pilot. Ce prédicat a des faux positifs mesurables dans le dépôt. Le refus
de confinement sort 78 sans journal (mika#2049), et sa cause est le relais
d'egress, pas le lanceur. Le refus `cwd-guard` (mika#2536) sort 1 sans journal sur
un skill voisin. `already_groomed` (mika#2012) et le dry-run sortent 0 sans
journal, si bien que la conjonction ne paraît sûre que par accident. L'inférence
aurait aussi créé un second lecteur du chemin `<pilot_log_dir>/<task-id>.log`, à
côté de `probe_pilot_log_signal`. Le pré-flight sort donc **79**
(`_EXIT_LAUNCHER_DEAD` dans dispatch-lib ;
`EXIT_PILOT_LAUNCHER_DEAD`, `executor.rs:61`), et le moteur classe sur ce code
pour émettre `pilot_launcher_dead` (`executor.rs:4865`) plus une ligne
`audit_events` `pilot_launcher_health = 'dead'`. 79 suit 78 (même famille :
« rien n'a été lancé »), hors des codes réservés au shell (126 et plus).

**5. Le marqueur est terminal et ne s'appelle pas `HANDLER CRASH`.**

`HANDLER CRASH` appartient à la population que `self-dev-callback` invite à
rejouer. Rejouer un lanceur cassé brûle un dispatch par tentative jusqu'à la
réparation de l'hôte, et aucun budget ne borne la boucle. Le `RESULT` porte
`Outcome: LAUNCHER_DEAD`, sur le patron de l'`Outcome: ESCALATE` de mika#2545, et
test-dispatch-lib affirme qu'il ne contient **aucun** des six motifs rejouables.

## Why This Matters

Un diagnostic prescriptif qui ne voyage pas vaut moins que rien : sa présence
dans le code fait croire que la panne est couverte. mika#1200 avait vu juste sur
la cause et sur le remède, et pourtant 2 h 45 de rail mort sont passées sans
alarme. Pour savoir si un message d'erreur atteint quelqu'un, la question n'est
pas « le code l'écrit-il ? » mais « quel canal est vivant à cette ligne ? ».
Dans dispatch-lib, avant le trap, la réponse est : aucun.

## When to Apply

- Avant d'ajouter un `exit` dans `dispatch_claude_pilot` (ou tout handler qui
  ouvre `exec 9>>… 2>/dev/null`), repérer où le trap est armé. Si l'`exit` est
  au-dessus, seul le code de sortie atteint le moteur.
- Avant de compter sur le trace tail pour porter une cause, compter les lignes de
  trace `set -x` émises entre l'écriture de la cause et la lecture du fichier.
- Avant de réécrire une ligne de lancement claude-pilot, relancer
  `test-dispatch-lib.sh` et lire le compte de sites des scans : une baisse veut
  dire que le site est sorti du champ du scan, pas qu'il est sûr.

## Examples

Avant (le diagnostic part dans `/dev/null`, la tâche reste non terminale) :

```bash
exec 9>>"$TRACE_FILE" 2>/dev/null || exec 9>/dev/null
command -v claude-pilot >/dev/null 2>&1 || { …; exit 1; }
if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then
    cat >&2 <<'EOF'      # fd 2 = /dev/null ici
Error: claude-pilot venv is broken …
EOF
    exit 1                # aucun trap armé : pas de callback
fi
…
trap '_dispatch_lib_exit_trap' EXIT
```

Après : les `command -v` restent en tête, le trap est armé, puis le smoke test
capture dans un fichier, pose un `RESULT` terminal et sort sur 79 (voir
Guidance §3).

Ce que cette correction ne couvre pas, avec suivi nommé dans le plan
(`docs/plans/2026-10-02-001-fix-2634-lanceur-claude-pilot-mort-plan.md`) : un
lanceur qui passe `--help` puis meurt au vrai lancement (la classe mika#2043), et
le `2>/dev/null` permanent lui-même. Le retirer enverrait le stderr du handler
vers un `Stdio::piped()` que l'exécuteur ne lit qu'après `child.wait()`, avec un
risque de blocage au-delà d'environ 64 Ko. Le frein contre les dispatches
répétés (AC2) relève de la phase B de mika#2634.

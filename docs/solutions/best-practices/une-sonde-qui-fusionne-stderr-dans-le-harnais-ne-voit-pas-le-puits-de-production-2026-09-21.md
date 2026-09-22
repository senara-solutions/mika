---
module: skills/bundled/_shared/dispatch-lib
tags: [dispatch-lib, test-harness, stderr, diagnostics, observability, signal-m, probe-design, bash-redirection]
problem_type: best_practice
category: best-practices
component: testing_framework
severity: medium
date: 2026-09-21
ticket: mika#2149
applies_when:
  - Adding a diagnostic line (grep-able signal) to dispatch-lib.sh or any function that runs inside dispatch_claude_pilot
  - Writing a test-dispatch-lib.sh probe that asserts a diagnostic "reaches stderr"
  - Reviewing a plan whose fire-disposition names the sink a line "lands in" without a probe that reads that sink
  - A grep on the persisted .stderr files stays empty for a signal the code visibly emits
---

# Une sonde qui fusionne stderr dans le harnais ne voit pas le puits de production

## Le fait

mika#2149 ajoutait un signal de dérive, `dispatch-lib: halt_family.unknown
subtype=<x>`, écrit par `echo … >&2` depuis `_halt_family`. La sonde T2
l'attestait : `_classify_terminated_session "$mode" 2>&1 >/dev/null` dans un
sous-shell, puis `assert_contains` sur la capture. Verte. Le plan, le doc de
solution et le commentaire de tête disaient tous que la ligne « atterrit
dans `$STDERR_FILE` (donc dans le tail du callback) et dans le `.stderr`
persisté ».

La revue (quatre relecteurs indépendants, puis le validateur) a mesuré autre
chose : **la ligne ne tombait nulle part.** `dispatch_claude_pilot` ouvre par
`exec 9>>"$TRACE_FILE" 2>/dev/null` (mika#903) — un `exec` sans commande
applique ses redirections **durablement** au shell, donc le fd 2 de toute la
fonction est `/dev/null`. La seule redirection `2>"$STDERR_FILE"` du fichier
couvre la commande pilote seule (`_run_pilot_sandboxed … 2>"$STDERR_FILE"`),
et `_classify_terminated_session` est appelée après, en `$( … )` nu. Mesure
minimale : `bash -c 'exec 9>>/dev/null 2>/dev/null; x=$(echo err >&2)'`
n'affiche rien.

Le CLAUDE.md racine documentait déjà cette classe sur ce même fichier
(« Signal M ») avec l'avertissement « do not carry this over by analogy » —
et l'implémentation l'a reproduite une ligne à côté, en suivant un plan qui
prescrivait le mauvais puits. Ce n'est pas un défaut de lecture : c'est que
**la sonde ne pouvait pas le voir**.

## Pourquoi la sonde était aveugle

Un `2>&1` posé **dans le harnais** autour de la fonction crée un fd 2 qui
n'existe pas en production. Il prouve que la fonction écrit sur son fd 2 ; il
ne prouve rien sur ce que ce fd 2 *est* au site d'appel réel. Toute assertion
« la ligne est sur stderr » passe alors quel que soit le puits — c'est un vert
qui ne mesure pas la propriété revendiquée.

## La règle

**Une sonde sur un diagnostic rejoue l'état des descripteurs du site d'appel
réel et lit le fichier que l'opérateur va grep — jamais un fd 2 fabriqué par
le harnais.** Concrètement, pour `dispatch-lib.sh` :

```bash
(
    source "$DISPATCH_LIB" 2>/dev/null || true
    exec 2>/dev/null                                  # l'état de dispatch_claude_pilot
    STDERR_FILE="$tmp/stderr.tmp";        : > "$STDERR_FILE"        # source du tail du callback
    PERSISTENT_STDERR="$tmp/probe.stderr"; : > "$PERSISTENT_STDERR" # le .stderr persisté
    … ; _classify_terminated_session >/dev/null
    printf 'tail:%s\n'      "$(cat "$STDERR_FILE")"
    printf 'persisted:%s\n' "$(cat "$PERSISTENT_STDERR")"
)
```

et l'assertion porte sur `tail:` et `persisted:`. Sur cette sonde, le `>&2`
nu est **rouge** (deux rouges constatés) pendant que la sonde naïve reste
verte — c'est la paire qui distingue les deux.

Côté code, le remède est celui que le fichier possédait déjà une fonction
plus haut (`pilot_log_guard.missing`, mika#2165) : émettre depuis un site où
les puits sont en portée, par `tee -a "${STDERR_FILE:-/dev/null}"
"${PERSISTENT_STDERR:-/dev/null}" >&2` — l'append parce que le `.stderr`
persisté a déjà été écrit une fois, et `$STDERR_FILE` parce qu'il est encore
sur disque à cet instant et que le tail de 10 Ko en est construit juste après.

## Ce qu'il faut retenir pour la prochaine fois

1. **Un plan qui nomme un puits est une hypothèse, pas un fait** — la sonde
   doit lire ce puits. « Atterrit dans X » sans une assertion qui ouvre X est
   la forme exacte du faux vert.
2. **Avant d'ajouter un `>&2` dans dispatch-lib, chercher `exec … 2>` dans la
   fonction englobante.** Un `exec` sans commande est une redirection
   permanente ; c'est ce qui rend Signal M structurel, pas accidentel.
3. **Une entrée « Signal M » dans le CLAUDE.md ne protège pas l'implémenteur
   qui a le plan sous les yeux** — seule une sonde qui rejoue le canal réel le
   fait. La règle du dépôt (`feedback_prompt_enforcement_fragile`) s'applique
   aussi à sa propre documentation.

Entrée sœur : `une-enumeration-en-commentaire-se-perime-au-rythme-de-lamont-2026-09-21.md`
(le remède de fond du ticket) ; classe amont : CLAUDE.md racine, « Signal M »
et « Signal S » (puits des diagnostics de dispatch-lib).

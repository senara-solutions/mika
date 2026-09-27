---
title: "Lire où vit vraiment le défaut avant d'ajouter un palier de cascade"
date: 2026-09-26
category: best-practices
module: skills/bundled/_shared
problem_type: best_practice
component: dispatch
severity: high
root_cause: config_error
resolution_type: code_fix
related_issues: ["mika#2542", "mika#2496", "mika#2508", "mika#2205"]
applies_when:
  - "Ajouter un palier de surcharge plus spécifique (label, par-ticket, par-agent) au-dessus d'un « défaut global »"
  - "Le défaut observé en production ne se retrouve pas tel quel dans le code (grep ne trouve pas la valeur)"
  - "Le ticket hésite sur la provenance du défaut (« source=default (ou env) »)"
  - "Une cascade de résolution de configuration comporte un rollback / kill-switch"
tags: [config-cascade, env-vars, override, rollback, max-turns, claude-pilot, provenance, loop-substrate, mika-2542]
---
# Avant d'ajouter une surcharge, trouver où vit vraiment le « défaut »

## Contexte

mika#2542 ajoute un palier de surcharge : un ticket portant le label
`loop-substrate` lance claude-pilot avec `--max-turns 200` au lieu du
« défaut global 150 ». Seulement, ce 150 n'était pas dans le code.

Avant le changement, le défaut in-file de `_pilot_max_turns` était **désarmé**
(`local _default=""`), et test-dispatch-lib.sh l'épinglait tel quel sous la
forme `|default|` (diff de mika#2542 : `"|default|"` → `"150|default|"`,
`skills/bundled/_shared/test-dispatch-lib.sh:6550-6553`). Le 150 observé venait
de l'env de l'hôte de production : `PILOT_MAX_TURNS=150` posé dans
`~/.mika/.env`, que le doc-comment de `PILOT_DISPATCH_ENV` constate
(`crates/mika-agent/src/skills/executor.rs:324-328`, constante à `:339`). Le
ticket lui-même hésitait : « source=default (ou env) ».

Conséquence : un palier label placé **sous** l'env dans la cascade aurait été
inerte en production. Mergé, déployé, tests verts, effet nul, aucune ligne
rouge. C'est la classe mika#2205 : une garde non déployée se lit exactement
comme une flotte en bonne santé.

**Lignée du réglage (session history).** Le plafond 150 n'est devenu effectif
qu'après mika#2508 : l'enfant de dispatch est construit par `env_clear()` + une
allowlist positive, qui ne laissait passer ni `PILOT_MAX_TURNS` nu ni un nom
`MIKA_*` ; le relais explicite `PILOT_DISPATCH_ENV` / `inject_pilot_dispatch_env`
(`crates/mika-agent/src/skills/executor.rs`) l'a réparé. C'est la **même
variable rendue inerte deux fois, par deux causes indépendantes** : d'abord
parce qu'elle n'atteignait pas le processus (#2508), puis — évité ici — parce
qu'une nouvelle source l'aurait contournée du mauvais côté. Le relèvement par
label a été tranché par Prime et l'opérateur **à n=3** d'une même classe
(#2532, #2536, #2519 coupés à 151 tours), explicitement contre un relèvement
global du 150.

## Recommandation

1. **Avant d'écrire la surcharge, localiser la source effective du défaut.**
   Code ? Env de l'hôte ? Fichier de config ? Lire le défaut in-file ET
   l'env réellement relayé au processus en production.
2. **Ordonner la cascade selon cette réalité, pas selon l'intuition
   « le spécifique bat le général ».** Dans mika#2542
   (`skills/bundled/_shared/dispatch-lib.sh:331-345`) :
   1. rollback — `PILOT_MAX_TURNS` défini et `""` ou `"0"` → aucun drapeau ;
   2. table de labels `PILOT_LABEL_TURN_CEILINGS` → ce plafond ;
   3. `PILOT_MAX_TURNS` entier > 0 → cette valeur ;
   4. sinon → le défaut in-file.
3. **Le rollback au-dessus de tout.** Un label qui écraserait le rollback
   ferait cesser le rollback d'en être un en plein incident, sans que rien
   ne le dise (`dispatch-lib.sh:338-340`).
4. **La source spécifique au-dessus de l'env de l'hôte**, puisque c'est
   l'env qui porte la valeur en production (`dispatch-lib.sh:340-343`).
5. **Rendre le défaut vrai dans le code dans le même changement.** Le défaut
   in-file est armé à 150 (`dispatch-lib.sh:406`) : V2 de mika#2496, rapportée
   par le ticket (150 tourne en prod depuis le 2026-09-24, ses seuls
   dépassements mesurés sont la classe exemptée). Livrer l'exception sans la
   règle qu'elle exempte, règle qui ne vivait que dans `~/.mika/.env`, aurait
   été la forme la plus fragile du travail (`dispatch-lib.sh:326-329`).
6. **Nommer la provenance dans la ligne d'observabilité.**
   `pilot_budget_armed max_turns=200 source=label label=loop-substrate`
   (`dispatch-lib.sh:506-510`) distingue notre 200 du `maxTurns=200` amont de
   claude-pilot, qui sinon se lirait pareil.
7. **Appariement exact de l'élément CSV**, pas un glob de sous-chaîne :
   `not-loop-substrate` ou `loop-substrate-v2` ne doivent rien relever
   (`dispatch-lib.sh:378-399`, épinglé par `test-dispatch-lib.sh:6688-6691`).

## Pourquoi c'est important

Une surcharge ordonnée contre un défaut imaginaire passe toute la chaîne de
vérification : tests unitaires verts (ils posent leur propre env), revue
d'accord avec le plan, déploiement propre. Le seul endroit où elle échoue est
la production, et elle y échoue en silence : le pilote tourne à 150 comme
avant, ce qui est exactement ce que la flotte faisait déjà. Personne ne voit
de différence, parce qu'il n'y en a pas.

La vérification a porté sur l'ordre lui-même : six mutations de la source ont
chacune fait virer au rouge les assertions attendues. En particulier,
remonter l'env au-dessus du label fait tomber AC3, « le label BAT l'env
(sinon inerte sur l'hôte de prod) » (`test-dispatch-lib.sh:6668-6671`).
L'ordre de la cascade est un livrable testé, pas un détail d'implémentation.

**Coût accepté, et nommé :** `PILOT_MAX_TURNS=50` ne borne PAS un ticket
`loop-substrate` sous 200 (`dispatch-lib.sh:343-345`). `max(label, env)` a
été écarté parce que `source=` cesserait de nommer une seule porte ; pour
brider un ticket substrat, on retire le label ou on pose
`PILOT_MAX_TURNS=0`.

## Quand l'appliquer

- Toute cascade de configuration à laquelle on ajoute un palier plus
  spécifique (label, par-agent, par-ticket, par-repo).
- Dès que le défaut « observé » en production ne se retrouve pas tel quel
  dans le code, ou que le ticket écrit « défaut (ou env) ».
- Quand la cascade contient un kill-switch : vérifier qu'aucun nouveau palier
  ne passe au-dessus de lui.
- Quand une valeur identique peut venir de deux sources (ici notre 200 et le
  200 amont de claude-pilot) : la ligne de log doit porter `source=`.

## Exemples

La cascade de mika#2542, réduite à sa forme
(`skills/bundled/_shared/dispatch-lib.sh:403-436`) :

```bash
_pilot_max_turns() {
    local _default="150"          # défaut ARMÉ dans le même geste (V2 mika#2496)
    _PILOT_MAX_TURNS_SOURCE="default"

    if [ -n "${PILOT_MAX_TURNS+set}" ] \
        && { [ -z "$PILOT_MAX_TURNS" ] || [ "$PILOT_MAX_TURNS" = "0" ]; }; then
        _PILOT_MAX_TURNS=""                     # 1. rollback : pas de drapeau
        _PILOT_MAX_TURNS_SOURCE="env"
    elif _pilot_label_turn_ceiling "${1:-}"; then
        _PILOT_MAX_TURNS="$_PILOT_LABEL_CEILING" # 2. label, AU-DESSUS de l'env
        _PILOT_MAX_TURNS_SOURCE="label"
    elif [ -n "${PILOT_MAX_TURNS:-}" ] && [ -z "$_PILOT_MAX_TURNS_INVALID" ]; then
        _PILOT_MAX_TURNS="$PILOT_MAX_TURNS"      # 3. env de l'hôte (150 en prod)
        _PILOT_MAX_TURNS_SOURCE="env"
    else
        _PILOT_MAX_TURNS="$_default"             # 4. défaut in-file
    fi
}
```

L'ordre naïf, « env avant label », qui aurait été inerte en production :

```bash
# FAUX sur un hôte qui porte PILOT_MAX_TURNS=150 : le label n'est jamais atteint.
elif [ -n "${PILOT_MAX_TURNS:-}" ]; then ...      # prend 150 partout
elif _pilot_label_turn_ceiling "$1"; then ...     # code mort en production
```

Ligne émise pour un ticket substrat :

```
dispatch-lib: pilot_budget_armed max_turns=200 source=label label=loop-substrate cost_bound=absent_upstream
```

## Voir aussi

- `docs/solutions/architecture-patterns/2026-09-06-accesseur-etroit-a-cote-du-resolveur-canonique.md` — mika#2205 : une source plus étroite à côté de celle qui gouverne réellement la production ; même famille, autre mécanisme.
- `docs/solutions/best-practices/deux-lecteurs-dune-meme-variable-denv-divergent-en-silence-2026-09-07.md` — deux lecteurs d'un même réglage divergent en silence.
- `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md` — qui possède l'env d'un processus.
- `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md` — pourquoi `loop-substrate` est déclaré dans `.github/labels.yml` et gardé par `scripts/check-pilot-turn-ceiling-labels.sh`.
- `docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md` — la revue de mika#2542 a retrouvé cette classe dans la garde elle-même : un contrôle de sous-ensemble qui ne sort en exit 3 que sur zéro entrée passe encore sur un parse **partiel** ; la garde refuse désormais toute ligne de table qu'elle ne sait pas modéliser et compte les affectations `+=`.
- mika#2542 (ce changement), mika#2496 (plafond de flotte, V2), mika#2508 (relais du réglage vers l'enfant de dispatch).

# Exhumer le nettoyage du 28/07 — procédure opérateur (mika#1943 AC1)

> **Ce document n'est pas l'inventaire.** C'est la procédure qui permet d'en
> produire un, et la règle qui dit quoi écrire quand elle ne rend rien.
> L'inventaire — `docs/solutions/incident-2026-07-28-bbytaa-cleanup-inventory.md`
> — ne s'écrit **qu'après** cette exécution, et **seulement si** elle rend
> quelque chose.

## Pourquoi c'est un geste opérateur et pas une PR

L'AC1 de mika#1943 nomme trois sources. Les trois vivent **hors du bac à sable**
dans lequel tourne toute session dispatchée. Mesuré le 2026-09-20 depuis une
session de dispatch :

| source nommée par l'AC1 | état dans le bac à sable | ce que ça prouve |
|---|---|---|
| `~/.claude/projects/-data-workspace-mika-platform/` | **absent** | rien — `/home` est un `tmpfs` |
| `/var/spool/claude-mail/samidarko/archive/` | **inexistant** | rien — non monté |
| mémoire MPC | hors périmètre | — |

La mesure qui tranche, lue dans `/proc/self/mounts` :

```
tmpfs /home tmpfs rw,nosuid,nodev,relatime,mode=755,uid=1000,gid=1000 0 0
```

`/home` est un **tmpfs créé pour ce bac à sable**. Seuls des chemins précis y
sont bind-montés en lecture seule (`.claude/plugins`, `.claude/settings.json`,
`.claude/commands`, `.claude/hooks`, `.cargo/*`, `.rustup`, …).
**`~/.claude/projects/` n'en fait pas partie.** Corollaire vérifié le même jour :
l'unique `*.jsonl` visible sur toute la machine est celui de la session courante,
et `~/.claude/projects/` ne contient que deux répertoires, tous deux créés le
jour même.

**Donc, depuis une session dispatchée, « le fichier n'existe pas » signifie
« le fichier n'est pas monté dans mon bac à sable », jamais « le fichier n'existe
pas sur l'hôte ».** C'est la doctrine déjà écrite pour le reaper mika#2277 — *un
signal qui ne peut pas être lu n'est jamais un terme satisfait* — appliquée à une
session au lieu d'un prédicat.

Écrire l'inventaire depuis cette vue serait **fabriquer une mesure** : un document
d'incident affirmant une liste qu'aucune source consultable ne soutient. C'est la
classe #953, et un inventaire faux portant l'autorité d'un doc d'incident est
strictement pire que pas d'inventaire — il clôt la question en se trompant.

## Étape 0 — la précondition, et elle est décisive

**Vérifier la fenêtre de rétention AVANT de chercher.** Sans ça, une recherche
infructueuse se lit comme « il n'y a rien eu » au lieu de « la rétention a purgé ».

```bash
cat ~/.claude/.last-cleanup                       # instant du dernier passage
grep -n cleanupPeriodDays ~/.claude/settings.json # rétention configurée, s'il y en a une
```

Mesuré le 2026-09-20 **depuis le bac à sable**, donc à re-faire sur l'hôte :
`.last-cleanup` portait `2026-09-20T18:44:17.617Z` et `settings.json` ne
configurait **aucune** rétention — le défaut Claude Code (30 jours) s'applique
donc. Le 28/07 est à **54 jours**.

**Conséquence à poser avant de commencer :** la probabilité que les sessions du
28/07 aient survécu est faible. Ce n'est pas une raison de ne pas chercher — c'est
la raison d'écrire le résultat négatif s'il se confirme, plutôt que de le laisser
sans trace pour la troisième fois.

## Étape 1 — les sources, sur l'hôte

À exécuter **hors bac à sable**, dans un shell de `samidarko` sur gentux.

```bash
# (a) journaux de session Claude Code autour du 2026-07-28
ls -la ~/.claude/projects/ | grep -i mika-platform
find ~/.claude/projects -name '*.jsonl' -newermt 2026-07-27 ! -newermt 2026-07-30 -ls

# (b) le contenu, si (a) rend quelque chose — la fenêtre « /data 100% »
grep -l -E 'rm -rf|worktree remove|prune|190G|100%' \
  ~/.claude/projects/*/*.jsonl 2>/dev/null

# (c) spool de courrier de l'orchestrateur
ls -la /var/spool/claude-mail/samidarko/archive/2026-07-28-* 2>/dev/null

# (d) journal du service, si la rotation l'a gardé
grep -n '2026-07-28' /var/log/mika/server.log* 2>/dev/null | head

# (e) instantanés btrbk encadrant la suppression — la source la plus
#     susceptible d'avoir survécu, puisqu'elle n'est pas soumise à la
#     rétention de Claude Code
btrbk list snapshots 2>/dev/null | grep 2026-07-2
```

`(e)` mérite d'être tentée même si `(a)`–`(d)` ne rendent rien : ce sont les
instantanés btrbk qui ont permis la récupération de `/data/workspace/bbytaa`, donc
une comparaison entre l'instantané d'avant et celui d'après **est** un inventaire
— et un inventaire mesuré plutôt que reconstitué de mémoire.

```bash
# Si deux instantanés encadrant le 28/07 existent, la différence EST la réponse :
diff <(cd <snapshot-avant>/workspace && find . -maxdepth 2 | sort) \
     <(cd <snapshot-apres>/workspace && find . -maxdepth 2 | sort)
```

## Étape 2 — la règle d'écriture

Trois issues, et **les trois se consignent**. C'est le point du document : le
ticket est ouvert depuis le 22/08 précisément parce que rien n'a jamais été écrit.

1. **Les sources répondent.** Écrire
   `docs/solutions/incident-2026-07-28-bbytaa-cleanup-inventory.md` avec, pour
   chaque chemin supprimé **hors d'un répertoire `target/`** : le chemin, la
   commande ou le script qui l'a supprimé, l'horodatage, et la source d'où le fait
   est tiré. Un chemin sans source ne va pas dans le document.

2. **Les sources ont été purgées.** Écrire le même document, court, disant
   **quelles** sources ont été consultées, **quand**, et **pourquoi** elles ne
   rendent rien (rétention à 30 jours, 54 jours d'écart). *C'est un résultat, pas
   un échec* : il ferme la question honnêtement et empêche qu'elle soit rouverte
   une quatrième fois avec la même espérance.

3. **Les sources existent mais sont ambiguës.** Écrire ce qui est établi, et
   nommer séparément ce qui ne l'est pas. Ne jamais compléter par déduction : un
   chemin « probablement supprimé » dans une liste de chemins supprimés devient un
   chemin supprimé dès la deuxième lecture.

## Halte

**Ne pas reconstituer l'inventaire de mémoire, ni à partir du code du nettoyage.**
Savoir ce qu'un script *pouvait* supprimer n'est pas savoir ce qu'il *a* supprimé,
et le document porterait l'autorité d'une mesure sur une inférence. Si les sources
ne rendent rien, la sortie est le cas 2 ci-dessus — pas une liste plausible.

## Ce qui est déjà fermé, et qui ne dépend pas de cette procédure

L'**AC2** de mika#1943 est livrée indépendamment : `dispatch-lib.sh` ne supprime
plus un chemin qu'il ne peut pas prouver être un worktree géré
(`_assert_removable_worktree_path`, allowlist alignée sur
`worktree_reaper::is_managed_worktree_path`). L'inventaire est une dette de
**mémoire**, pas une précondition de la garde : celle-ci tient sans lui.

## Références

- mika#1943 — le ticket, re-déposé le 2026-08-22 sur directive de sami.
- mika#2277 — *un signal qui ne peut pas être lu n'est jamais un terme satisfait*.
- mika#953 — la classe « affirmation non ancrée sur un résultat d'outil ».
- mika#2205 — un scan silencieusement inactif se lit comme un scan oisif ; c'est
  la même forme qu'une source non montée lue comme une source vide.

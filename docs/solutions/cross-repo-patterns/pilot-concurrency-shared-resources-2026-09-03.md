---
module: mika-agent/skills, skills/bundled/_shared
tags: [dispatch, concurrency, pilot, sandbox, egress, mika-2160, measurement]
problem_type: architecture
category: cross-repo-patterns
---

# Ce que deux pilotes simultanés partageraient, et ce que la machine en supporte

**Ticket :** mika#2160 — Phases 1 et 2 (AC1, AC2).
**Mesures prises le 2026-09-04 sur `gentux`**, pendant qu'un pilote réel tournait
(dispatch mika#2169, tâche `0c94f866-1940-4a3e-b75c-ec624993f298`). Cette
coïncidence est la raison pour laquelle plusieurs verdicts ci-dessous sont
confirmés sur une ligne de commande `bwrap` vivante et pas seulement sur la
lecture de `dispatch-lib.sh`.

Ce document ne demande pas de lever la sérialisation. Il répond à deux
questions : **qu'est-ce que deux pilotes partageraient**, et **combien la
machine en tient**. La décision sur N reste à l'opérateur (AC6).

---

## Phase 1 — Inventaire des ressources partagées (AC1)

Trois verdicts possibles : *partageable en l'état*, *partageable après changement
nommé*, *bloquante*. Un verdict sans citation n'est pas un verdict.

| # | ressource | ancrage | verdict |
|---|---|---|---|
| 1 | socket d'egress `/tmp/mika-pilot-egress.sock` | `dispatch-lib.sh:169`, `:396`, `:856` | **partageable en l'état** — mesuré |
| 2 | port `8891` du bac à sable | `dispatch-lib.sh:170`, `:855` | **partageable en l'état** — la prémisse du ticket était fausse |
| 3 | répertoire de secrets `/run/mika-pilot-secrets` | `dispatch-lib.sh:210`, `:993` | **partageable en l'état** |
| 4 | `~/.mika/data/pilot-transcripts/` | `dispatch-lib.sh:818`, `:1054`, `:1137`, `:2167` | **partageable en l'état** |
| 5 | `pilot-helper.log` / `pilot-egress-proxy.log` | `dispatch-lib.sh:189`, `:389-394` | **partageable après changement nommé** |
| 6 | helper mitmdump `:8892` + son fichier de jeton | `dispatch-lib.sh:178`, `:187`, `:495` | **partageable après changement nommé** |
| 7 | rappel `canUseTool` → mika-dev | `a2a.rs:226`, `:360`, `server/mod.rs:508` | **bloquante en l'état** |

### 1. Socket d'egress — partageable en l'état, mesuré

`_PILOT_EGRESS_SOCK="/tmp/mika-pilot-egress.sock"` (`dispatch-lib.sh:169`). Un
seul daemon hôte le sert (`:396`, lancé en `nohup` ; réutilisé sans relance si
déjà joignable, `:381`), et chaque bac à sable le monte tel quel
(`--bind "$_PILOT_EGRESS_SOCK" "$_PILOT_EGRESS_SOCK"`, `:856`).

**La mesure, pas la déduction.** Deux clients simultanés ouverts sur le socket
pendant que le pilote de mika#2169 s'en servait : **2/2 acceptés**. La sonde
portait son contrôle négatif dans le même appel — un chemin inexistant refusé
avec `FileNotFoundError` — donc le résultat positif n'est pas un artefact de
sonde complaisante.

### 2. Port 8891 — partageable en l'état, et le corps du ticket se trompait

mika#2160 range le port `8891` parmi les ressources à arbitrer. Il n'y a rien à
arbitrer : le bac à sable est lancé avec `--unshare-net` (`dispatch-lib.sh:855`),
donc chaque pilote a **son propre espace de noms réseau**. Confirmé sur le
`bwrap` vivant du dispatch mika#2169, qui porte `--unshare-net` suivi de
`--setenv HTTPS_PROXY http://127.0.0.1:8891`. Le `127.0.0.1:8891` de deux pilotes
désigne deux sockets distincts qui ne se voient pas.

### 3. Répertoire de secrets — partageable en l'état

`_PILOT_SECRET_DIR_SANDBOX="/run/mika-pilot-secrets"` (`:210`) n'est pas un
chemin hôte : le bac à sable monte `--tmpfs /run`, et chaque secret y est
matérialisé depuis un descripteur de fichier par
`--perms 0600 --ro-bind-data "$_sfd"` (`:993`). Deux pilotes ont deux tmpfs.
Confirmé sur le `bwrap` vivant, qui porte `--tmpfs /run` et **aucun `GH_TOKEN`
dans ses `--setenv`** — la fermeture de mika#2039 tient en production.

### 4. `pilot-transcripts/` — partageable en l'état

Le répertoire est créé une fois (`:818`) et monté en écriture dans chaque bac à
sable (`:1054`, `:1137`), mais le nommage est **par tâche** : un `.jsonl` par
`task-id` (`:2167`). Confirmé en vivant :
`--setenv ANTHROPIC_LOG_FILE …/pilot-transcripts/0c94f866-….jsonl`. Aucun
fichier commun, donc aucun entrelacement.

### 5. Journaux hôte — partageable après changement nommé

`_PILOT_HELPER_LOG="/var/log/mika/pilot-helper.log"` (`:189`) et
`pilot-egress-proxy.log` (`:389-394`) sont des fichiers **uniques** avec un
écrivain par pilote. Deux pilotes restent lisibles par un humain, mais **tout
comptage par session devient faux** : rien dans la ligne ne dit de quel dispatch
elle vient.

*Changement nommé :* préfixer chaque ligne par le `task-id`, ou un fichier par
dispatch. Ce n'est pas un prérequis de correction — c'est un prérequis
d'**observabilité**, et il se paie au moment où l'on passe à N>1, pas avant :
sans lui, la première anomalie à deux pilotes sera diagnostiquée à l'aveugle.

### 6. Helper mitmdump et son jeton — partageable après changement nommé

Le daemon mitmdump (`:178`, port `8892`) est un processus hôte long-vivant, déjà
multi-clients par construction ; il tournait en PID 21178 pendant ces mesures et
servait le pilote en cours. Ce n'est pas lui le problème.

Le problème est `_PILOT_GH_TOKEN_FILE="$HOME/.mika/pilot-gh-token"` (`:187`),
**réécrit avant chaque spawn** par `_stage_pilot_gh_token` (`:495`). Deux spawns
rapprochés écrivent successivement dans le même chemin. Aujourd'hui c'est
inoffensif — tous les dispatchs portent le même jeton — mais la ressource est
structurellement partagée, et le jour où deux pilotes ont deux identités
GitHub distinctes, le second écrasera le premier sans bruit.

*Changement nommé :* un fichier par dispatch (`pilot-gh-token.<task-id>`), ou
passer le jeton par le canal `--ro-bind-data` déjà en place pour les autres
secrets (`:993`) plutôt que par un chemin hôte fixe.

### 7. Rappel `canUseTool` — bloquante en l'état

C'est la trouvaille de l'inventaire, et elle ne se voyait pas depuis le corps du
ticket.

Le relais du pilote est `{"command":"mika","args":["--agent","mika-dev","ask"]}`
(`.claude/claude-pilot.json`, timeout 120 000 ms). `mika ask` frappe
`/a2a/{agent}`, et `crates/mika-agent/src/server/a2a.rs:226` et `:360` prennent
le mutex par agent (`server/mod.rs:508`) en **`try_lock_owned()`** : pas
d'attente, pas de file. Sur collision, la réponse est
`JsonRpcError(INTERNAL_ERROR, "Agent is busy")`, immédiatement.

Deux pilotes dont les escalades de permission se recouvrent verraient donc l'un
des deux se faire **refuser** sa demande de permission — pas différer, refuser.
Le chemin `/message` a reçu une file bornée (mika#1870) ; le chemin A2A, non.

**Preuve directe, et elle est gênante :** la première passe architecte du
grooming de ce ticket s'est fait refuser **cinq fois de suite** avec
`"Agent is busy"` avant de passer à la sixième, le 2026-09-03 entre 23:43 et
23:46 CEST. Un seul agent, un seul appelant — et la contention était déjà là.

**Ce que devient une escalade refusée côté pilote reste non mesuré.** Le
comportement de `claude-pilot` face à `"Agent is busy"` (réessai ? échec de
l'outil ? abandon de session ?) se mesure, il ne se devine pas, et aucune
mesure n'existe aujourd'hui. C'est la raison pour laquelle ce verdict est
« bloquante » et non « partageable après changement nommé » : on ne sait même
pas de quelle taille est le dégât.

**Remédiation fichée : mika#2163** (file bornée sur le chemin A2A, l'équivalent
de mika#1870 pour `/message`). C'est un prérequis d'**exploitation** de N>1, pas
un prérequis de livraison du réglage.

### Ce que l'inventaire ne couvre pas

`crates/mika-agent/src/server/ci_failure_handler.rs:312` appelle aussi
`has_active_callback_tasks_excluding`, mais **à titre informatif** — le
commentaire sur place le dit, la valeur alimente un pré-digest destiné au LLM et
ne garde rien. mika#2160 ne le touche pas. Conséquence à N>1 : ce texte
continuera de dire « occupé » à la granularité booléenne alors qu'un créneau
reste libre. Cosmétique, mais à savoir avant de lire un digest à deux pilotes.

---

## Phase 2 — La borne matérielle, chiffrée (AC2)

Toutes les valeurs ci-dessous sont **mesurées le 2026-09-04 sur gentux**, pas
estimées. La procédure est donnée pour que chaque chiffre se refasse.

### Correction portante : les worktrees ne vivent pas où le grooming le croyait

Le plan de grooming inscrivait :

```
/home  466 G  —  216 G libres        (les worktrees vivent ici)
```

**C'est faux, et l'erreur va dans le mauvais sens.** `/data/workspace` est un
volume LVM distinct :

```
$ df -h /data/workspace/mika-platform
/dev/mapper/vg0-models  371G  255G   98G  73% /data
```

**371 G au total, 98 G libres** — pas 216. La borne disque, qui est la borne qui
mord, se calcule sur 98 et non sur 216. Un plan qui se serait fié au chiffre du
grooming aurait surestimé N d'un facteur deux.

### Les mesures

```
volume des worktrees   /data (vg0-models)   371 G total, 98 G libres avant ces travaux
RAM                    61 G total, 47 G disponibles
CPU                    16 cœurs
worktrees mika         46 présents
somme des target/      94 G sur l'ensemble des worktrees
plus gros target/      38 G   (fix-2158) — au-dessus de la fourchette 21–35 G de mika#2105
suivants               19 G (fix-2156), 15 G (bug-2036), 8,5 G, 8,1 G
checkout principal     19 G
```

Build à froid du workspace complet dans un worktree neuf, ce jour :

```
durée                  2 min 08 s   (16 cœurs, cache de registre chaud)
pic RSS agrégé         5,46 Gio     (échantillonnage 1 Hz sur le groupe de processus)
target/ produit        5,4 G        (debug seul)
```

### Procédure — et pourquoi la commande du plan ne marche pas

Le plan prescrivait `/usr/bin/time -v`. **`/usr/bin/time` n'existe pas sur cet
hôte** : sur Gentoo, GNU time est `sys-process/time`, paquet séparé non installé,
et le mot-clé shell `time` ne connaît pas `-v`. Pire que l'absence : la commande
a rendu un code de sortie 0 sur la tâche d'arrière-plan, parce que la dernière
commande du pipeline avait réussi. Une sonde qui ment sans échouer.

Le remplacement, sans privilège et sans installation — échantillonner le RSS
agrégé du **groupe de processus** du build :

```bash
setsid cargo build --workspace > build.stdout 2> build.stderr &
BPID=$!
PGID=$(ps -o pgid= -p "$BPID" | tr -d ' ')
PEAK=0; NONEMPTY=0
while kill -0 "$BPID" 2>/dev/null; do
  SUM=$(ps -e -o pgid=,rss= | awk -v g="$PGID" '$1==g {s+=$2} END {print s+0}')
  [ "$SUM" -gt 0 ] && NONEMPTY=$((NONEMPTY+1))
  [ "$SUM" -gt "$PEAK" ] && PEAK=$SUM
  sleep 1
done
```

`NONEMPTY` est le contrôle qui compte : sans lui, un `pgid` mal résolu rend `0`
et `0` se lit comme une mesure. Sur ce relevé : **124 échantillons, 124 non
vides**.

**Limite honnête de la méthode :** l'échantillonnage à 1 Hz peut manquer un pic
plus court qu'une seconde. `5,46 Gio` est donc un **plancher** du vrai pic, pas
le vrai pic. Il reste très loin des 47 G disponibles, donc la conclusion ne
dépend pas de cette imprécision.

### La borne, et ce qui la fixe

| axe | mesure | borne sur N |
|---|---|---|
| **disque** | 98 G libres ; `target/` en vol de 19 à 38 G selon l'empilement | **2** au pire cas (38 G), **4** au cas courant (19 G) |
| **mémoire** | pic 5,46 Gio par build, 47 G disponibles | ~8 — jamais la contrainte |
| **CPU** | 16 cœurs, saturés par un seul `cargo build` | aucune borne dure : deux builds se ralentissent, ils n'échouent pas |

**Le disque est la seule borne dure, et elle vaut N = 2 à 4 sur cette machine.**
La dispersion vient entièrement de l'empilement `debug + tests + release` dans un
même `target/`, qui est précisément ce que mika#2105 décrit : 5,4 G pour un
`debug` seul mesuré aujourd'hui, contre 38 G pour le plus gros survivant. **Une
hygiène de `target/` déplace la borne plus efficacement que du matériel** —
corriger mika#2105 rendrait N=4 confortable sans acheter un disque.

### Ce que la borne ne dit pas

Le matériel laisse N=2 sur la table. **Le rappel `canUseTool` ne le laisse pas.**
Lever le plafond sans mika#2163, c'est échanger une sérialisation lisible
(`global_dispatch_active`, avec son re-tir différé enregistré) contre des refus
de permission illisibles au milieu d'une session. C'est pourquoi le défaut livré
reste **1**.

---

## Ce que le code livre, et ce qu'il ne décide pas

`MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT`, forme à trois paliers du module
(absent → défaut ; illisible ou négatif → défaut avec WARN ; `0` → plafond
levé), **défaut 1**. Le plafond bouge aux deux endroits qui le tenaient, ou il
serait décoratif : le prédicat d'existence devient un comptage, et
`slot_index` rejoint la clé primaire de `dispatch_slot_leases` (schéma v52).

La PR ne pose la variable nulle part.
**Le code ne choisit pas N ; il rend N choisissable.**

## Voisins, non traités ici

- **mika#2163** — file bornée sur `/a2a`. Prérequis d'exploitation de N>1.
- **mika#2105** — l'empilement `target/`. C'est le levier de la borne disque.
- **mika#2158** — le livelock des prédicats de grooming. Augmenter le débit
  d'une file vide ne donne rien ; ce ticket-ci est sans effet tant que #2158
  tient.
- **mika#2156** — le balayage phantom qui rendait le plafond illisible :
  **fermé depuis le grooming**. Une session longue n'est plus marquée morte
  pendant qu'elle tient le verrou.

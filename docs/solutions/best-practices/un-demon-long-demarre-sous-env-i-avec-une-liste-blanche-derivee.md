---
title: "Un démon long démarre sous env -i, avec une liste blanche dérivée de ce qu'il lit — et la preuve se lit sur le vrai processus"
module: skills/bundled/_shared/dispatch-lib.sh, scripts/mika-pilot-egress-proxy
date: 2026-10-02
problem_type: best_practice
component: tooling
severity: medium
category: best-practices
applies_when:
  - "un lanceur shell démarre par nohup … & un démon qui survit à la tâche qui l'a lancé"
  - "on retire l'environnement hérité d'un processus et il faut savoir ce qui en dépendait"
  - "on veut prouver qu'une variable posée par l'appelant n'atteint PAS un processus"
related_components:
  - testing_framework
tags: [egress, relay, daemon, env-i, allowlist, nohup, proc-environ, negative-control, canary, dispatch-lib, mitmdump]
---

# Un démon long démarre sous `env -i`, avec une liste blanche dérivée de ce qu'il lit

## Context

`_ensure_pilot_egress_proxy` lançait le relais d'egress par `nohup "$BIN" … &`, donc
avec l'environnement complet de la tâche de dispatch qui l'appelait. Le relais est un
démon : il survit à cette tâche et sert tous les dispatches suivants. Il conservait
donc durablement des valeurs de la tâche qu'il ne lit jamais. Le correctif le lance
sous `env -i` avec une liste blanche nommée
(`skills/bundled/_shared/dispatch-lib.sh:566`, appliquée à la ligne 879).

Le lanceur voisin, `_ensure_pilot_helper` (mitmdump, `dispatch-lib.sh:707`, `nohup` à
la ligne 745), garde la forme d'avant. Il est nommé comme suivi dans le `CLAUDE.md` du
dépôt (§ Signal Q) et par
`docs/solutions/security-issues/un-harnais-sur-le-vrai-home-ecrit-les-chemins-de-production.md`
(« Le canal qui reste ouvert ») : ce document-ci est le mode d'emploi pour le jour où
on le ferme, et la différence de fond y est nommée plus bas.

## Guidance

**1. La liste blanche se dérive d'un inventaire, jamais d'une intuition.** Cinq
sources, dont quatre ne sont pas des `os.environ.get` :

| source | exemple dans le relais | variable |
|---|---|---|
| lecture explicite | `scripts/mika-pilot-egress-proxy:708` | `MIKA_EGRESS_DEBUG` |
| lecture implicite par la stdlib | `Path.home()`, ligne 198 | `HOME` |
| le shebang `#!/usr/bin/env python3` | ligne 1 | `PATH` |
| lecture implicite par une bibliothèque native | `ssl.create_default_context()`, ligne 1024 | `SSL_CERT_FILE`, `SSL_CERT_DIR`, `OPENSSL_CONF` |
| l'interpréteur lui-même | locale de CPython | `LANG` |

Un `grep os.environ` trouve la première ligne et rate les quatre autres. Et l'absence
ne casse pas toujours bruyamment, ce qui est précisément le danger : sans `PATH`,
`env` retombe sur son chemin de recherche par défaut, donc le relais démarre tant que
`python3` est dans `/usr/bin` et ne démarre plus le jour où il vit sous nvm, un venv
ou `/usr/local` ; sans `HOME`, `Path.home()` retombe sur l'entrée passwd, identique
pour un même uid et divergente dès qu'un lanceur pose un autre `HOME` ; sans
`OPENSSL_CONF` sur un hôte qui le pose, la TLS amont change de comportement sans
erreur. Mesuré : `env -i /usr/bin/env python3 …` démarre sur cet hôte, ce qui aurait
fait passer un `PATH` oublié inaperçu.

**2. Exclure, et écrire pourquoi, au même endroit que la liste.** Deux exclusions
délibérées : la seam de test `_MIKA_EGRESS_PREBIND_TEST_BARRIER` (ligne 1323), que les
tests posent en lançant le script directement, jamais par le lanceur ; et
`SSLKEYLOGFILE`, que la même bibliothèque lit mais qui écrirait les secrets de session
TLS sur disque. La règle d'extension est donc « une variable dont le démon a besoin
pour sa fonction », pas « toute variable qu'il lit » — appliquée à la lettre, la
seconde forme réintroduirait `SSLKEYLOGFILE`.

**3. « Transmise seulement si définie », jamais vide.** La boucle teste `${!v+x}` : une
variable absente chez l'appelant n'arrive pas du tout. La passer vide changerait la
sémantique pour les bibliothèques qui distinguent « absente » de « vide ».

**4. `env` fait `exec`.** `nohup env -i … "$BIN"` : `nohup`, `env` et le shebang
`/usr/bin/env` remplacent chacun le processus, donc `$!` reste le pid du démon. Le
message d'échec du lanceur, qui nomme ce pid pour la jointure avec le journal du
relais, n'a pas eu à changer.

**5. La preuve se lit sur le vrai processus, lancé par le vrai lanceur.**
`skills/bundled/_shared/tests/test_egress_relay_socket_and_env.sh` exporte une
sentinelle chez l'appelant, appelle le vrai `_ensure_pilot_egress_proxy`, et lit
`/proc/<pid>/environ` du relais : sentinelle absente, `PATH`/`HOME`/`MIKA_EGRESS_DEBUG`
présentes, **aucun nom hors de la liste blanche**, une variable définie arrive avec sa
valeur, une non définie n'arrive pas. Contrôle négatif intégré : la même sonde contre
une copie de la bibliothèque où `env -i` est retiré, avec la précondition « la copie
diffère d'exactement une ligne » — sans elle, une sentinelle absente pourrait vouloir
dire « filtrée » ou « jamais exportée ».

## Why This Matters

**Le canary ne peut pas prouver un changement de lanceur.**
`scripts/canary-pilot-containment` parle au socket du relais **en service** :
`_ensure_pilot_egress_proxy` voit un socket joignable et ne relance rien. Son vert
atteste la non-régression du confinement, pas l'environnement d'un relais qu'il n'a
pas démarré. Il installe en outre le script de la branche dans `~/.local/bin` dans
**tous** ses modes (`scripts/canary-pilot-containment:81`) : avant de le lancer depuis
une branche, vérifier que le socket vivant est joignable (sinon il démarrerait le
relais de branche sur le chemin de production) et sauvegarder puis restaurer le
binaire installé.

**Un déploiement ne change pas un démon déjà en service.** Il garde l'environnement
avec lequel il a démarré jusqu'à sa relance ; l'étape d'activation et sa sonde
(`/proc/<pid>/environ`) sont dans `docs/operator/pilot-egress-relay.md` § 4.4.

**Retirer l'environnement casse aussi les tests qui s'en servaient comme canal.** Le
faux relais du fixture `_egress_guard_probe` (`skills/bundled/_shared/test-dispatch-lib.sh`)
recevait le chemin de son marqueur de lancement par une variable exportée ; sous
`env -i` il ne la voit plus, et l'assertion `launched=yes` rougit pour une raison
étrangère à son objet. Le marqueur est maintenant dérivé du chemin du script
(`$(dirname "$0")/launched`). Chercher ce motif avant de durcir un lanceur : tout
fixture qui `export` quelque chose vers le processus lancé.

## When to Apply

- Au prochain lanceur de démon par `nohup … &` dans `dispatch-lib.sh`, et d'abord à
  `_ensure_pilot_helper` (mitmdump). **Attention, la différence est de fond :** l'addon
  d'authentification de mitmdump a un repli délibéré sur l'environnement du processus
  (`GH_TOKEN` / `MIKA_GITHUB_TOKEN`) quand le fichier posé par le dispatch est absent.
  L'inventaire de l'étape 1 y trouvera donc une variable **fonctionnelle** qui est
  aussi un secret ; c'est l'arbitrage de canal que le document prédécesseur nomme, et
  la liste blanche ne le tranche pas à sa place.
- À tout processus dont on veut prouver qu'il ne reçoit **pas** une valeur : lire
  `/proc/<pid>/environ` du processus réel, avec un contrôle négatif qui diffère d'une
  ligne.

## Examples

Avant :

```bash
nohup "$_PILOT_EGRESS_PROXY_BIN" --host-unix --socket "$_PILOT_EGRESS_SOCK" \
    >>"$log_file" 2>&1 </dev/null &
```

Après :

```bash
local -a relay_env=()
local relay_var
for relay_var in "${_PILOT_EGRESS_RELAY_ENV_ALLOWLIST[@]}"; do
    [ -n "${!relay_var+x}" ] && relay_env+=("$relay_var=${!relay_var}")
done
nohup env -i "${relay_env[@]}" "$_PILOT_EGRESS_PROXY_BIN" --host-unix --socket "$_PILOT_EGRESS_SOCK" \
    >>"$log_file" 2>&1 </dev/null &
```

Contrôle négatif, construit dans la suite :

```bash
sed 's/nohup env -i "\${relay_env\[@\]}" /nohup /' "$DISPATCH_LIB" > "$MUTATED_LIB"
changed=$(diff "$DISPATCH_LIB" "$MUTATED_LIB" | grep -c '^<')   # doit valoir 1
```

**Piège d'outillage rencontré en construisant ces contrôles.** Muter un fichier puis le
restaurer par `git checkout --` efface tout ce qui n'était pas committé. Ici le commit
précédent avait échoué en silence : sous zsh, une variable listant plusieurs chemins
(`F="a b c"; git add $F`) n'est pas découpée en mots, `git add` reçoit un seul chemin
inexistant, et le contrôle négatif a ensuite effacé la modification non committée du
lanceur. Committer avec des chemins écrits en clair, vérifier `git status` vide,
**puis** muter.

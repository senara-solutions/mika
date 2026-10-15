---
title: "Une sentinelle qui ne voyage pas dans le canal que l'incident écrase n'atteste rien"
date: 2026-10-15
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: design_decision
severity: high
symptoms:
  - "un `cargo test` lancé depuis une sandbox de pilote écrit sur la forge réelle sous l'identité du moteur (substrat de mika#2713)"
  - "l'addon d'egress retire tout `Authorization` du client puis injecte le jeton de l'hôte : un appel légitime et un appel de test sont indiscernables"
  - "la forme proposée par le corps du ticket — un en-tête dédié `X-Mika-Egress-Sentinel` — n'est pas implémentable sans un shim `gh` sur le PATH"
root_cause: design_decision
resolution_type: design_decision
related_components:
  - pilot_dispatch
  - egress_proxy
tags:
  - loop-substrate
  - egress
  - sentinel
  - channel-selection
  - threat-model
  - attestation
  - fail-open
ticket: mika#2716
---

# Une sentinelle qui ne voyage pas dans le canal que l'incident écrase n'atteste rien

Le détail opérateur (interrupteur, registre, les quatre états du canari, la
halte) vit dans `docs/operator/environment-variables.md` § *La sandbox de pilote
présente un justificatif d'egress (mika#2716)*. Ce document garde seulement le
raisonnement de **choix du canal**, qui se transpose à toute attestation posée
devant un proxy qui réécrit les requêtes.

## Le problème

L'addon mitmproxy de l'egress des sandboxes
(`scripts/mika-pilot-github-auth-addon.py`) **retire** tout `Authorization` venu
du client — « the sandbox has none » — puis **injecte** le jeton lu côté hôte.
Conséquence : le proxy ne peut pas distinguer un `gh pr create` légitime d'un
pilote d'un `cargo test` qui croyait parler à un faux. C'est le substrat de
l'incident mika#2713, où un test posant `GH_TOKEN="fake-token"` a produit
29 commentaires et un label sur un ticket **fermé**. Un `gh` nu aurait rendu 401.

Pour distinguer les deux, il faut **doter** chaque sandbox d'un justificatif que
le proxy réclame. Reste à choisir *dans quel canal* il voyage — et c'est là que
la réponse intuitive est fausse.

## La table des canaux examinés

| canal | posé par `gh` ? | posé par `git` ? | écrasé par le test ? | verdict |
|---|---|---|---|---|
| en-tête dédié `X-Mika-Egress-Sentinel` | **non** par variable d'environnement — seul `gh api -H` existe, par appel | oui (`http.extraHeader` via `GIT_CONFIG_KEY_n`) | **non** | **écarté** |
| en-tête posé par un shim `gh` sur le PATH | oui | non (deux mécanismes à construire) | non | **écarté** |
| `Proxy-Authorization` | non | non | non | **écarté** (l'addon le retire aussi) |
| fichier monté dans la sandbox | n/a — hors du chemin réseau | n/a | non | **écarté** |
| **le credential lui-même (`GH_TOKEN`)** | **oui, déjà** | **oui, déjà** | **oui** | **retenu** |

Trois raisons condamnent l'en-tête dédié, et la troisième est décisive :

1. **Les deux clients ne partagent aucun canal d'en-tête commun.** `git` a
   `http.extraHeader` ; `gh` n'a rien d'équivalent par environnement. Il
   faudrait donc deux mécanismes pour une seule propriété.
2. **Un shim `gh` sur le PATH serait emprunté *aussi* par le `cargo test`.** Un
   secret posé par un wrapper atteste **la sandbox**, pas **le chemin
   légitime** — donc rien d'utile pour mika#2713, qui est précisément un appel
   depuis la sandbox.
3. **Un en-tête survivrait intact à l'écrasement du jeton.** La signature de
   l'incident est qu'un test **écrase** `GH_TOKEN`. Avec un en-tête, le test
   écraserait le jeton, l'en-tête resterait valide, et l'incident se
   reproduirait à l'identique.

## La propriété qui a décidé

> La signature de l'incident est l'écrasement de `GH_TOKEN`. Donc le
> justificatif doit vivre **dans** `GH_TOKEN`.

Un test qui pose `GH_TOKEN="fake-token"` **détruit la sentinelle de sa propre
main** et se fait refuser. Aucun autre canal n'a cette propriété : elle ne vient
pas de la force cryptographique du secret, elle vient de ce que le canal choisi
est exactement celui que le mode de défaillance écrase.

Le terrain était déjà préparé : depuis mika#2572 la sandbox porte
`--setenv GH_TOKEN "proxy-managed-no-secret"` — un placeholder statique et
public, requis parce que `gh` lit sa config locale avant toute émission réseau
(`gh-verifie-lauth-localement-avant-le-reseau-il-faut-un-placeholder.md`). Le
correctif ne crée donc **aucun canal** : il rend cette valeur **per-spawn et non
devinable**, et l'enregistre côté hôte.

## Modèle de menace, énoncé pour ne pas être sur-promis

La sentinelle voyage dans l'environnement de la sandbox, donc
`cat /proc/self/environ` la révèle **à l'intérieur**. C'est inhérent : pour que
`gh` la pose, elle doit être dans l'environnement de `gh`. Ce que la sentinelle
achète n'est donc **pas** une barrière contre un code hostile dans la sandbox,
c'est la distinction entre **le chemin légitime** et **le chemin qui a écrasé le
credential**. Dit autrement : elle ferme la classe *accident*, pas la classe
*adversaire*. Côté hôte, l'argv de bwrap est lisible par `/proc/<pid>/cmdline`,
mais quiconque le lit détient déjà le fichier du vrai jeton : la sentinelle ne
lui apprend rien.

## Deux pièges d'implémentation, mesurés

- **L'ordre d'appel porte le correctif.** Le tirage tourne **après**
  `_stage_pilot_gh_token` et n'écrit jamais le `GH_TOKEN` de l'hôte. Une
  implémentation qui exporterait la sentinelle dans le `GH_TOKEN` *de l'hôte* —
  la forme la plus naturelle quand on pense « mettre la sentinelle dans
  l'environnement » — la ferait stager **comme le vrai jeton** : l'addon
  l'injecterait vers `api.github.com` et **tout** appel de pilote rendrait 401.
  Ce mode de défaillance est plus sourd que le 403 qu'on cherche à armer, parce
  qu'il vient de GitHub et ne nomme rien de mika#2716.
- **Le registre est un répertoire, jamais un fichier.** `mitmdump` est
  long-vivant et partagé entre dispatches : deux pilotes concurrents doivent
  être valides **au même instant**. Un fichier unique écrasé refuserait le plus
  ancien des deux. Et son chemin se résout **à l'appel**, pas au `source` : les
  suites de confinement réexportent `HOME` après le `source`, donc un défaut
  figé déposerait des entrées d'allure vivante dans le registre réel de
  l'opérateur (mesuré : deux, au premier passage du ticket).

## Ce que le choix du canal impose au relais de l'interrupteur

`_stage_pilot_egress_sentinel` lit `MIKA_PILOT_EGRESS_SENTINEL` **dans le child
de dispatch**, dont `sandboxed_pilot_env` reconstruit l'environnement par une
allowlist **positive** : nommer une variable ne la fait pas traverser, seul un
relais injecté **après** le bac à sable le fait (mesuré 2026-09-24, mika#2508).
Sans ce relais, l'interrupteur serait honoré par son seul lecteur côté service
et le shell armerait le tirage quand même : un interrupteur décoratif
(mika#2165). Le relais est sa propre liste (`EGRESS_SENTINEL_ENV`) et non une
entrée de `PILOT_DISPATCH_ENV`, pour deux raisons indépendantes : le nom est
`MIKA_`-préfixé, donc hors de la population des « formats de fil » nus du canal
pilote ; et son **vide n'est pas un palier** (`${…:-1}` rend `1`), là où le vide
de `PILOT_MAX_TURNS` *est* le rollback documenté. Rien n'entre dans
`SANDBOX_ENV_CORE_ALLOWLIST` (mika#2354 AC9(b)).

## La leçon transposable

Quand on choisit où poser une attestation devant un proxy qui réécrit les
requêtes, le critère n'est pas « quel canal est le plus propre » ni « quel canal
est le plus secret ». C'est :

> **Quel canal le mode de défaillance que je veux détecter détruit-il ?**

Poser l'attestation ailleurs produit un garde qui reste vert pendant l'incident
même qu'il était censé nommer.

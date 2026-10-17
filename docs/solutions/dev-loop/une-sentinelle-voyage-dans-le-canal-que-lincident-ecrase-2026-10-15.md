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
  - "sur `github.com` aucun justificatif ne peut voyager, donc la moitié `git push` semble hors d'atteinte — alors que le proxy voit déjà le dépôt et les refs (mika#2736)"
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
  - receive-pack
ticket: mika#2716
---

# Une sentinelle qui ne voyage pas dans le canal que l'incident écrase n'atteste rien

Le détail opérateur (interrupteurs, registre, motifs de refus, états du canari,
haltes) vit dans `docs/operator/environment-variables.md` § *La sandbox de pilote
présente un justificatif d'egress, et l'addon l'exige — et ce qu'elle pousse est
borné (mika#2716, mika#2724, mika#2736)*. Ce document garde seulement le
raisonnement de **choix du canal**, qui se transpose à toute attestation posée
devant un proxy qui réécrit les requêtes — et, depuis mika#2736, la question qui
vient quand aucun canal ne convient.

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
| **le credential lui-même (`GH_TOKEN`)** | **oui, déjà** | **non** — l'addon injecte un `Basic` côté proxy, donc aucun justificatif ne peut voyager dans ce canal sur `github.com` | **oui** | **retenu** |
| **aucun canal — une borne sur ce que le proxy voit déjà** | n/a — **rien n'est demandé au client** | n/a — idem | **impossible** : il n'y a rien à écraser | **retenu pour `github.com`** (mika#2736) |
| **un fichier hôte relu par requête** | n/a — **rien n'est demandé au client** ; la donnée ne traverse jamais la sandbox | n/a — idem | **impossible** : la sandbox n'a pas le chemin (`~/.mika` n'est pas dans les binds de bwrap) | **retenu pour le périmètre déclaré** (mika#2751) |

**La septième ligne n'est pas un canal de justificatif, et c'est pour ça qu'elle
est arrivée en dernier.** Elle ne porte rien *depuis* la sandbox : elle porte, de
l'hôte vers l'addon, **ce que l'hôte sait déjà** du dispatch qu'il a lui-même
choisi. Elle répond à une troisième question, qui n'est ni « quel canal porte le
justificatif ? » ni « ce que le garde voit déjà suffit-il ? », mais :

> **Ce que le garde voit déjà suffirait-il si on y ajoutait ce que l'hôte sait
> déjà ?**

Son verdict de la colonne « écrasé par le test ? » est le point entier : il est
**impossible** par confinement, pas par chance. Un processus de la sandbox ne
peut ni lire ni écrire ce fichier — c'est la même asymétrie hôte/sandbox dont
mika#2724 tire sa sentinelle, utilisée cette fois non pour attester un appel
mais pour **cadrer** ce qu'un appel peut viser. Le § mika#2751 ci-dessous
développe, y compris ce que ce canal coûte quand le lecteur est un démon
partagé.

**La sixième ligne manquait à cette table, et son absence est ce qui a fait
paraître la question close.** Cinq des sept lignes demandent « quel canal porte
le justificatif ? ». Tant que la table ne contient que celles-là, un canal où
*aucun* justificatif ne peut voyager se lit comme « hors d'atteinte ». La sixième
pose une autre question — *ai-je besoin qu'on me dise quoi que ce soit ?* — et
sur `github.com` la réponse est non ; le § mika#2736 ci-dessous la développe.

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

## Rectification de la table, et ce qu'elle a coûté au périmètre (mika#2724)

La case *« posé par `git` ? »* de la ligne retenue disait **« oui, déjà »**.
C'est **faux**, et la correction n'est pas cosmétique : elle réduit le périmètre
du refus.

Dans la sandbox de pilote, `git` ne présente **aucun** justificatif. Il n'y a
pas de credential helper, `GIT_CONFIG_NOSYSTEM=1`, et
`_stage_pilot_gitconfig` écrit le `GIT_CONFIG_GLOBAL` **de zéro** : rien n'y
lit `GH_TOKEN`. C'est l'addon qui, côté proxy, injecte un
`Basic base64("x-access-token:<jeton>")` vers `github.com`. Autrement dit, le
canal choisi — `GH_TOKEN` — n'est emprunté que par `gh`, et `gh` ne parle qu'à
`api.github.com`.

Conséquence directe sur le vol 2 : **le refus est borné à `api.github.com`**.
Exiger un justificatif sur `github.com` refuserait chaque `git push` sans rien
acheter, puisqu'aucun justificatif ne peut y voyager. La moitié `git push` de
l'egress de pilote est donc hors de portée de *ce* canal — ce qui est une
propriété du canal, pas un oubli.

> **Elle n'est plus ouverte pour autant.** La version de 2026-10-16 de ce §
> concluait « la moitié `git push` reste donc ouverte » ; mika#2736 a montré que
> la conclusion ne suivait pas de la prémisse. Trois des quatre sous-classes de
> menace sont fermées depuis, **sans justificatif**, par une borne sur ce que le
> proxy voit déjà — § *Hors de portée de ce canal n'est pas hors d'atteinte*
> ci-dessous.

La case fautive n'était pas une erreur de frappe : elle décrivait ce que `git`
fait **hors sandbox**, où un helper pose bien un credential. La table parlait de
la sandbox ; une des cinq lignes parlait du poste de l'opérateur.

## Hors de portée de *ce* canal n'est pas hors d'atteinte (mika#2736)

Le § ci-dessus concluait, dans sa version du vol 2, « la moitié `git push` reste
donc ouverte ». La prémisse est exacte — aucun justificatif ne voyage sur
`github.com` — et la conclusion n'en suit pas. C'est la leçon que ce ticket
ajoute à la note : **une table de canaux ne répond qu'à la question qu'elle
pose.**

### Le modèle de menace de cette moitié, séparément de celui de mika#2713

Une écriture d'API et un `git push` ne coûtent pas la même chose et ne se
réparent pas pareil, donc le premier travail a été de mesurer ce qu'un push
arbitraire atteint **réellement**, compte tenu des bornes de mika#2141 (le bind
du gitdir est déjà limité au worktree du pilote et aux refs sous son préfixe —
mais ça borne un **système de fichiers**, et une destination de push est une
**URL**).

| sous-classe | ce qu'elle atteint | réparable ? |
|---|---|---|
| **P1** — push vers un dépôt tiers | exfiltration du code vers une forge qu'on ne contrôle pas | **non**. Une fois abouti, un push chez autrui ne se reprend pas. C'est la plus coûteuse, donc la première règle |
| **P2** — suppression d'une ref | destruction d'une branche sur la forge | oui, par reflog côté serveur, mais avec une fenêtre et une intervention |
| **P3** — écriture d'une ref hors périmètre (tag, `refs/*` arbitraire, `refs/heads/main`) | contenu laissé sur la forge que la vue des branches ne montre pas ; ou contournement de la pipeline par une écriture directe sur l'intégration | oui |
| **P4** — force-push sur la ref du spawn | réécriture de l'historique — **la classe de l'incident fondateur mika#1318** | difficilement, et c'est la plus sournoise |

**Trois sur quatre sont fermées par ce vol.** P4 ne l'est pas, pour une raison
de protocole détaillée plus bas.

### Le canal d'en-tête, réexaminé à son propre mérite — et mesuré

`http.extraHeader` via `GIT_CONFIG_KEY_n` / `GIT_CONFIG_VALUE_n` est le **seul**
canal d'en-tête que `git` possède par environnement, et le vol 1 l'avait écarté
dans le cadre d'une sentinelle commune aux deux clients. Réexaminé seul, il
**fonctionne techniquement** : l'en-tête atteint bien le proxy. Il est écarté
quand même, pour deux raisons indépendantes :

1. **Il atteste la sandbox, pas le chemin.** C'est mot pour mot la raison 2 qui
   condamnait le shim `gh` ci-dessus : un second processus du même environnement
   lit `GIT_CONFIG_VALUE_n` et présente le même en-tête, parfaitement valide, en
   poussant où il veut. Le justificatif distinguerait « dans la sandbox » de
   « hors de la sandbox », quand la question est « le chemin légitime » contre
   « un autre chemin de la même sandbox ».
2. **La règle 2 de `scripts/verify-no-secret-in-setenv.sh` le refuse** par
   ailleurs, indépendamment du mérite du canal.

Une borne côté **bwrap**, dans le prolongement de mika#2141, a été écartée pour
une impossibilité de catégorie : bwrap borne un système de fichiers ; une
destination de push est une URL, qu'aucun bind ne contraint.

### Ce que le proxy voit réellement d'un `git-receive-pack`

C'est la mesure qui retourne la question. Le corps d'un push n'est pas opaque
avant le packfile :

```
<4 hex len><old-sha> SP <new-sha> SP <refname> NUL <capabilities> LF
<4 hex len><old-sha> SP <new-sha> SP <refname> LF
0000
PACK…
```

Le **dépôt** est dans l'URL (`/<owner>/<repo>.git/git-receive-pack`) et les
**commandes de ref** sont en **clair** en tête du corps, avant le pack. Le proxy
n'a donc besoin qu'on lui dise **rien** : il peut borner ce qu'il voit déjà. Et
une borne sur ce qu'il voit déjà ne demande **aucune coopération du client** —
donc aucun processus de la sandbox ne la contourne en se présentant bien. Sur
cette moitié, c'est **strictement plus fort** qu'une sentinelle l'aurait été.

### Les deux pièges qui 403'raient toute la boucle, mesurés avant d'écrire

Les deux sont des conséquences du streaming, et les deux sont des cas
**positifs** de la suite (`test_egress_push_bound.sh`, 143 assertions), jamais
de la prose :

- **La sonde d'authentification de 4 octets.** Avant de streamer un gros push,
  `git` émet un POST dont le corps est un **flush-pkt nu** : aucune commande. Un
  garde exigeant « au moins une commande » le refuserait, donc refuserait **tout
  push assez gros pour être streamé** — et rien ne le dirait avant le premier
  dispatch. Une liste de commandes **vide** et `None` sont donc deux états
  distincts (classe mika#2205), le premier autorisé, le second refusé.
- **Le verdict ne peut pas vivre dans `requestheaders`.** C'est là que vit
  l'injection du credential, mais le corps n'y est pas encore. Passé le seuil,
  le corps sort chunked **avec les commandes toujours en tête** : le verdict vit
  donc dans `request`, et les deux hooks portent délibérément deux
  préoccupations distinctes.

### P4 — le force-push, hors d'atteinte côté proxy, et pourquoi

`receive-pack` **ne porte aucun drapeau de force**. Le client envoie
`<old-sha> <new-sha> <ref>` et rien d'autre ; c'est le **serveur** qui résout
l'ancestralité et décide si la mise à jour est un fast-forward. La commande d'un
push ordinaire et celle d'un force-push ont donc la **même forme**, et
`--force-with-lease` renseigne `old-sha` aussi honnêtement qu'un push normal.

Pour les distinguer, le proxy devrait **décoder le packfile en vol** et marcher
le graphe, sur le chemin de la boucle, pour une décision à rendre avant de
relayer. Hors de proportion avec la propriété achetée, et du code fragile placé
exactement là où une panne arrête la boucle.

P4 reste donc ouverte, et elle est gardée ailleurs, **plus faiblement** : le
prompt de `dev-groom` interdit au pilote de groom tout `git push` en nommant
mika#1318 — une consigne de prompt, pas un mécanisme ; et le site unique de push
des dispatches de groom tourne **côté hôte**, hors de la sandbox et donc hors de
portée de tout garde d'egress. C'est la moitié de réponse « hors d'atteinte »
que le ticket autorisait, pour une **sous-classe nommée**, avec la mesure qui la
soutient — écrite ici et dans la surface opérateur, parce qu'une limite connue
et tue est une surprise d'opérateur, tandis qu'une limite connue et écrite est
un périmètre.

## Le périmètre que l'hôte sait déjà, et ce qu'il coûte de le faire voyager (mika#2751)

mika#2736 ferme la part **catégorique** de P3 : tout ce qui n'est pas une
branche, et les deux branches d'intégration. Reste la part **relationnelle** —
non pas « est-ce une branche ? » mais « est-ce **le** dépôt de **ce**
dispatch ? ». Le vol 3 de cette surface la ferme pour la dimension dépôt, et la
mesure qui l'ouvre tient en une phrase : **l'hôte connaît déjà la réponse,
puisqu'il a choisi le dispatch.** `dispatch-lib.sh` résout le dépôt et la
branche dans `_set_up_worktree`, bien avant de lancer la sandbox. Il n'y a donc
rien à deviner, rien à décoder, et rien à demander au client : il n'y a qu'un
problème de **transport** entre un producteur hôte et un lecteur hôte.

### Un démon long-vivant et partagé n'a qu'un seul environnement, et c'est celui du premier dispatch

C'est la leçon la plus transposable de ce vol, et elle a été mesurée avant
d'écrire une ligne. La forme naturelle — « l'addon tourne dans un `mitmdump`
lancé par l'hôte, donc passons-lui le périmètre par son environnement » — est
celle que le corps du ticket suggérait, et elle est **fausse** :
`_ensure_pilot_helper` rend 0 dès que le port du relais est vif. Le démon n'est
donc **pas** relancé par dispatch : son environnement est celui du **premier**
dispatch qui l'a démarré, et il le reste jusqu'au prochain redémarrage de la
machine. Un périmètre passé par là serait celui d'un vol terminé depuis des
heures, et la sonde serait verte.

> **Une donnée per-spawn ne peut pas voyager dans l'environnement d'un processus
> qui survit au spawn.** Elle doit voyager par quelque chose que le lecteur
> **relit**, pas par quelque chose qu'il a hérité.

D'où le canal retenu : un **fichier hôte relu à chaque requête**, une entrée par
spawn sous `~/.mika/pilot-push-scopes/`. L'autorité vient de ce que l'hôte écrit
et que `~/.mika` n'est pas dans les binds de bwrap — infalsifiable depuis la
sandbox. Le même raisonnement condamne, au passage, l'idée d'étendre l'entrée de
sentinelle du vol 1 : les deux données ont des postures de défaillance opposées
(celle-là **refuse** sur registre illisible, celle-ci **dégrade**), des
interrupteurs différents, et auront des durées de vie différentes. Un seul
fichier pour les deux ferait porter à la plus tolérante la posture de la plus
stricte.

### Le lecteur partagé ne peut pas attribuer une requête à un spawn, donc le verdict porte sur l'UNION

Le corollaire du § précédent, et le vrai prix de ce canal. Rien sur le fil ne
dit **quel** pilote parle : le démon est partagé, et deux dispatches concurrents
sortent par la même socket avec le même credential injecté. La question que
l'addon peut poser n'est donc pas « l'entrée de *ce* spawn déclare-t-elle ce
dépôt ? » mais « existe-t-il **une** entrée vive qui le déclare ? » — c'est-à-dire
l'**union** des périmètres en vol.

**Et le coût de cette approximation diffère par dimension.** C'est ce qui a
découpé le travail en deux vols, et c'est le raisonnement à réemployer avant
d'ajouter une dimension à un périmètre lu par union :

| dimension | cardinalité de l'union | conséquence |
|---|---|---|
| **dépôt** | **bornée par quatre** — `_is_dispatchable_repo` refuse tout ce qui sort de `DISPATCHABLE_REPO_BASENAMES` (`mika`, `mika-cloud`, `mika-skills`, `mika-platform`) | le resserrement est **insensible au TTL** : une entrée résiduelle n'élargit l'union vers rien que le vol suivant n'aurait pu déclarer lui-même. Livrable seul |
| **branche** | **non bornée** — chaque vol ajoute un nom | une entrée qui survit à son pilote laisse **sa branche** poussable par le suivant. Le resserrement vaut exactement ce que vaut la désinscription des entrées — donc ni plus ni moins que le cycle de vie, qui n'existe pas encore |

D'où le découpage : la dimension dépôt est livrée par mika#2751, la dimension
branche et le cycle de vie partent **ensemble** dans senara-solutions/mika#2766.
Livrer `ref=` sans la désinscription donnerait un garde qui se desserre tout
seul au fil de la journée, et dont la force dépendrait de l'heure — exactement
la forme d'un garde dont on ne peut rien conclure.

**Une propriété rassurante tombe de l'union, et il faut la nommer parce qu'elle
gouverne la lecture des incidents : une entrée de plus ne peut qu'ÉLARGIR
l'union, jamais la resserrer.** Donc une entrée résiduelle, parasite ou mal
écrite ne peut pas **provoquer** un refus ; elle ne peut qu'en **manquer** un.
C'est ce qui rend inoffensive l'entrée que `scripts/canary-pilot-containment`
dépose à chaque passage, et ce qui dit à l'opérateur qu'un `push-scope-repo` sur
un push légitime ne vient jamais du registre d'un voisin.

### Un pilote n'a qu'un seul `origin` possible, et c'est ce qui rend la question décidable

Le resserrement ne serait qu'une **politique** si un pilote avait plusieurs
destinations légitimes : il faudrait alors arbitrer, et tout arbitrage finit par
être desserré par le premier cas qui le gêne. Ce n'est pas le cas. Un seul
`.git` parent est bind-monté dans la sandbox (bornes de mika#2141), donc la
sandbox ne contient **qu'un** `origin`. Un push vers un dépôt frère de
l'organisation n'est donc pas « rare » ou « à discuter » : il est **sans cause
légitime possible**. C'est ce qui autorise un refus sec plutôt qu'un
avertissement, et c'est la même forme de raisonnement que celle du vol 1 — on ne
ferme bien que ce dont on peut dire, sans arbitrage, qu'il n'arrive jamais dans
le régime nominal.

### Le resserrement est subordonné par sa PLACE, pas par une relecture

Point 2 du ticket : périmètre absent ou illisible ⇒ la borne statique continue
de s'appliquer, jamais un `return` qui ouvre tout. La propriété est obtenue en
appelant le resserrement **à l'intérieur de la branche `ALLOW`** de la borne
statique, et nulle part ailleurs. Sa seule issue neuve est donc un refus de
plus : il n'existe aucun chemin par lequel il transforme un refus en
autorisation, et **aucun `return` de sa fonction ne peut ouvrir quoi que ce
soit**. C'est une propriété de la **structure**, pas une invariante à revérifier
à chaque lecture — et c'est pourquoi elle est écrite au site d'appel plutôt que
confiée à la vigilance d'un relecteur.

Le détail opérateur de cette moitié — registre, format `clé=valeur`, le sixième
motif `push-scope-repo`, les deux jetons de dégradation et leurs remèdes
distincts, la matrice des deux interrupteurs, la sonde `grep 'DEGRADE scope-'` —
vit à l'entrée `MIKA_PILOT_EGRESS_PUSH_SCOPE` de
`docs/operator/environment-variables.md`.

## Le schéma que `gh` émet, capturé sur le fil

Le grooming du vol 2 avait mesuré que **les deux** chaînes de format `Bearer %s`
et `token %s` vivent dans le binaire `gh` installé. Un format présent n'est pas
une émission : lequel part sur le fil était la seule mesure que la DoD du vol 2
rendait prérequise, parce qu'un lecteur qui n'accepterait que `Bearer` rendrait
403 sur **chaque** appel `gh` de la boucle.

Capturé contre **GitHub CLI v2.92.0**, en pointant `http_unix_socket` (une clé
de config de `gh` prévue pour ça) vers un listener HTTP en clair :

```
Authorization: token ghp_probe2724PROBE2724PROBE2724PROBE27
```

sur `GET /rate_limit`, sur `POST /graphql`, et via une vraie sous-commande
(`gh issue list`). **Jamais `Bearer`.** Le lecteur de l'addon accepte donc les
deux, plus `Basic` décodé, et compare le schéma sans égard à la casse.

Deux détours inutiles, notés pour ne pas être repris : `gh api --hostname
host:port` est refusé (`invalid hostname`), et Go contourne `HTTP_PROXY` pour
tout nom d'allure localhost — donc ni un port ni un proxy local ne permettent
d'observer `gh` en clair. `http_unix_socket` est le chemin qui marche.

## La leçon transposable

Quand on choisit où poser une attestation devant un proxy qui réécrit les
requêtes, le critère n'est pas « quel canal est le plus propre » ni « quel canal
est le plus secret ». C'est :

> **Quel canal le mode de défaillance que je veux détecter détruit-il ?**

Poser l'attestation ailleurs produit un garde qui reste vert pendant l'incident
même qu'il était censé nommer.

### La seconde leçon, payée par mika#2736

La première est une leçon sur le **choix** d'un canal. La seconde est sur ce
qu'on conclut quand **aucun** canal ne convient :

> **Avant de conclure « hors d'atteinte », demander : ai-je besoin qu'on me dise
> quoi que ce soit ?**

Une table de canaux ne répond qu'à la question qu'elle pose. Les cinq premières
lignes de la table ci-dessus demandent toutes « quel canal porte le
justificatif ? » — et sur `github.com` la réponse honnête est *aucun*. Écrire
cela est exact ; en déduire que la moitié est hors d'atteinte est un saut, parce
que la question suivante n'avait pas été posée : **ce que le garde voit déjà
suffit-il ?**

Ici il suffisait presque entièrement. Le dépôt est dans l'URL, les commandes de
ref sont en clair en tête du corps, et trois des quatre sous-classes de menace
se ferment sans qu'un mot soit demandé au client — avec en prime une propriété
qu'un justificatif n'aurait pas eue, puisqu'il n'y a rien à se faire présenter
par un processus qui se présente bien.

Le réflexe à garder : quand la table des canaux est vide, ne pas clore —
inventorier ce que le garde observe sans coopération, puis conclure. Et quand
une sous-classe reste réellement hors d'atteinte, **la nommer** (ici P4, le
force-push) plutôt que de laisser le refus global en tenir lieu : une limite
connue et écrite est un périmètre, une limite connue et tue est une surprise
d'opérateur.

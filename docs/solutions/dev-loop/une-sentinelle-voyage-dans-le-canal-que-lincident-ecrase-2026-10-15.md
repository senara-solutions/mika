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
  - "une entrée de périmètre survit à son pilote, donc la force du garde dépend de l'heure ; et la purger trop tôt refuse le push d'un pilote vif sous un voisin vif (mika#2766)"
  - "un pilote pousse sur la branche d'un autre dispatch du même dépôt : les quatre règles statiques sont satisfaites et le périmètre déclaré ne juge que le dépôt (mika#2776)"
  - "sur deux dimensions, « l'union des périmètres en vol » a deux lectures — par dimension elle autorise le produit cartésien et se lit resserrée sans rien resserrer (mika#2776)"
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
  - entry-lifecycle
  - ttl
  - scope-dimensions
ticket: mika#2716
---

# Une sentinelle qui ne voyage pas dans le canal que l'incident écrase n'atteste rien

Le détail opérateur (interrupteurs, registre, motifs de refus, états du canari,
haltes) vit dans `docs/operator/environment-variables.md` § *La sandbox de pilote
présente un justificatif d'egress, et l'addon l'exige — et ce qu'elle pousse est
borné (mika#2716, mika#2724, mika#2736)*. Ce document garde seulement le
raisonnement de **choix du canal**, qui se transpose à toute attestation posée
devant un proxy qui réécrit les requêtes — depuis mika#2736, la question qui
vient quand aucun canal ne convient, depuis mika#2766, comment se choisit la
durée de vie de ce qui voyage dans ce canal, et depuis mika#2776, ce qu'on doit
redire d'une union de déclarations dès qu'elle porte sur **deux** dimensions.

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
| **branche** | **non bornée** — chaque vol ajoute un nom | une entrée qui survit à son pilote laisse **sa branche** poussable par le suivant. Le resserrement vaut exactement ce que vaut la désinscription des entrées — donc ni plus ni moins que le cycle de vie |

**D'où l'ordre des vols, et c'est l'ordre que ce tableau impose.** La dimension
dépôt est livrée par mika#2751, parce qu'elle est la seule des deux qui soit
livrable **seule**. Le **cycle de vie** a suivi (mika#2766, § ci-dessous), et la
dimension **branche** est arrivée après lui (mika#2776, § *La dimension branche*
plus bas). L'ordre inverse — `ref=` d'abord — aurait donné un garde qui se
desserre tout seul au fil de la journée, et dont la force dépendrait de l'heure :
exactement la forme d'un garde dont on ne peut rien conclure. Un resserrement
dont la valeur est conditionnée par un mécanisme qui n'existe pas encore attend
ce mécanisme ; ce n'est pas une précaution de style, c'est que la mesure de sa
force est indisponible avant.

**Et ce tableau ne dit pas tout de l'union, ce qu'on n'a vu qu'en ajoutant la
seconde dimension.** Il compare la **cardinalité** de l'union par dimension, et
c'est ce qui a ordonné les vols. Mais sur deux dimensions la phrase « le verdict
porte sur l'union des périmètres en vol » cesse d'avoir **un** sens — elle en a
deux, et un seul est juste. C'est le § *L'union porte sur les entrées* du vol 4.

**Une propriété rassurante tombe de l'union, et il faut la nommer parce qu'elle
gouverne la lecture des incidents : une entrée de plus ne peut qu'ÉLARGIR
l'union, jamais la resserrer.** Donc une entrée résiduelle, parasite ou mal
écrite ne peut pas **provoquer** un refus ; elle ne peut qu'en **manquer** un.
C'est ce qui rend inoffensive l'entrée que `scripts/canary-pilot-containment`
dépose à chaque passage, et ce qui dit à l'opérateur qu'un `push-scope-repo` sur
un push légitime ne vient jamais de la **présence** de l'entrée d'un voisin.

**La symétrie s'arrête là, et c'est le point que le cycle de vie a dû traiter :
une entrée de MOINS, elle, peut provoquer un refus.** Non par sa propre absence
— un registre vide dégrade — mais parce que la dégradation porte sur le registre
**entier**. Retirer l'entrée d'un pilote vif pendant qu'un voisin maintient le
registre non vide ne le fait pas tomber dans la dégradation : il tombe dans le
refus. Le voisin ne cause pas le 403, il **retire la porte de sortie** qui
l'aurait évité. Les deux formes se lisent donc en sens opposés, et un opérateur
qui ne tient que la première instruira le mauvais incident.

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
motif `push-scope-repo`, ses jetons de dégradation et leurs remèdes distincts,
la matrice des deux interrupteurs, la sonde `grep 'DEGRADE scope-'` — vit à
l'entrée `MIKA_PILOT_EGRESS_PUSH_SCOPE` de
`docs/operator/environment-variables.md`, où le vol 4 a ajouté le septième motif
et le troisième jeton.

## Une entrée de périmètre meurt avec son spawn, et son TTL est un PLANCHER (mika#2766)

Le vol 3, et la moitié dont le § précédent disait qu'elle conditionne la
suivante. Deux mécanismes, dont le second n'existe que parce que le premier ne
peut pas tout couvrir :

- **la désinscription**, à la fin du **spawn** — l'entrée a un nonce que le
  lanceur tient déjà (`_PILOT_EGRESS_PUSH_SCOPE_ENTRY`) ;
- **un TTL côté addon**, parce qu'un dispatch tué par `SIGKILL` n'exécute
  **aucun trap**. Une entrée que rien n'a retirée doit cesser de répondre
  d'elle-même.

### À la fin du spawn et non du dispatch, parce qu'un nonce devient inconnaissable

Le réflexe serait un `trap EXIT` au niveau du dispatch : un seul site, une seule
ligne. Il serait **faux**, et la mesure qui le dit est que `_run_pilot_sandboxed`
tourne **plus d'une fois** par dispatch — le pilote principal, puis les pilotes
de révision, de section manquante, de taille de plan. Or
`_stage_pilot_egress_push_scope` remet le nonce **à vide** à chaque entrée. Le
nonce du spawn N est donc inconnaissable dès que le spawn N+1 stage : un trap de
dispatch ne pourrait nettoyer que le **dernier**, en laissant tous les
précédents, qui sont exactement ceux dont on voulait borner la durée de vie.

D'où un **appel explicite** à la fin de `_run_pilot_sandboxed` plus un **filet**
dans le trap de sortie — la forme de `_release_issue_seat` (mika#2155), et pour
la même raison : bash n'a pas de trap à portée de fonction. L'appel est placé
**après** la capture de `_sandbox_rc` (invariant mika#2039), donc le statut du
pilote reste celui de la fonction : une désinscription ne peut pas changer le
verdict d'un vol.

La désinscription retire **sa propre entrée**, jamais un balayage et jamais le
répertoire. Un voisin en vol n'est pas à elle — et le § suivant dit ce que
coûterait l'inverse.

### L'asymétrie des erreurs, qui est ce qui FIXE la valeur du TTL

Le piège de ce TTL n'est pas qu'il soit difficile à choisir ; c'est qu'il se
choisit sous une asymétrie totale, et qu'une lecture symétrique mène à une
valeur trop petite. La dégradation de l'addon n'est **pas** par entrée :
`_push_scope_verdict` ne se rabat sur la borne statique que si le registre
**entier** est vide ou illisible.

| direction | ce qu'elle coûte |
|---|---|
| **trop long** | une entrée survit à un pilote tué. L'union reste bornée par les dépôts dispatchables, donc **rien de neuf ne devient atteignable** : un relâchement borné, qu'aucun incident ne matérialise |
| **trop court** | un pilote **encore vif** voit son entrée purgée tandis qu'un voisin maintient le registre non vide. Son push légitime tombe sur `push-scope-repo` : un **403 sur du travail innocent**, et la boucle autonome s'arrête |

Les deux colonnes ne sont pas commensurables, donc la valeur ne se négocie pas
entre elles : elle se choisit comme un **plancher sur le plus long spawn**.
Mesuré sur les **718 transcripts de pilote** de l'hôte de développement —
médiane **11,9 min**, p90 **55,2 min**, p99 **1,89 h**, **max 4,40 h**. La
valeur retenue est **12 h**, soit **2,7×** ce maximum ; **4 h serait déjà sous
lui**, et c'est la seule chose que la mesure interdit.

Le TTL de **72 h** des sentinelles de mika#2724 ne convient pas, et pas pour une
question de marge : il est calibré sur une **autre question**. « Un vol peut-il
être relu trois jours après ? » n'est pas « ce périmètre est-il encore en vol ? »
— et un périmètre de trois jours ne l'est jamais. Les deux registres passent par
le **même** `_purge_registry`, qui reçoit donc le TTL **en paramètre**. Un seul
nombre pour deux registres aurait été, à l'identique, le défaut du throttle
partagé que mika#2751 avait déjà corrigé : la fonction sert deux appelants, donc
tout ce qui dépend de l'appelant sort de la fonction.

**Pas de variable d'environnement**, pour la même raison qu'au vol 1 : un délai
qu'un opérateur peut mettre à zéro est un registre qu'il peut vider par
accident. Le tableau ci-dessus dit exactement ce que coûte ce geste — et le
remède d'urgence existe déjà ailleurs, `MIKA_PILOT_EGRESS_PUSH_SCOPE=0`, qui ne
touche aucune entrée.

### Ce que cette asymétrie rend testable, et qui est le cas le plus important du vol

Un test sur la constante reste vert sous **n'importe quelle** valeur : il
constate un nombre, il ne le justifie pas. Le cas qui rend 12 h vérifiable
plutôt que décrété asserte la **conséquence** de la colonne « trop court » :
entrée A vieillie au-delà du TTL (le pilote est vif) sous entrée B fraîche (le
voisin) ⇒ le push de A est **refusé**, pas dégradé. Juxtaposé, dans le même cas,
à la même purge **sans voisin** ⇒ une simple dégradation `scope-absent`. Les
deux moitiés côte à côte mettent l'asymétrie sur la page, là où une prose seule
l'aurait laissée à reconstruire. `test_egress_push_bound.sh` CASE 22.

Ce que cela généralise est écrit au § *La troisième leçon* en fin de note.

Le détail opérateur du cycle de vie — la sonde `ls ~/.mika/pilot-push-scopes/`
et ce que chacun de ses quatre états veut dire, la dégradation nommée quand la
désinscription échoue, et l'avertissement sur le vidage à la main sous un
dispatch en vol — vit à la même entrée
`MIKA_PILOT_EGRESS_PUSH_SCOPE` de `docs/operator/environment-variables.md`.

## La dimension branche, et ce qu'ajouter une seconde dimension a appris (mika#2776)

Le vol 4, qui ferme la sous-classe P3. Avant lui, un pilote dispatché sur
`feat/2766/…` pouvait pousser sur `refs/heads/fix/9999/un-autre-ticket` — la
branche d'un **autre** dispatch du même dépôt, éventuellement en vol — sans que
rien ne s'y oppose : le dépôt est celui que l'hôte a déclaré pour lui, la ref est
bien sous `refs/heads/`, et ce n'est ni `main` ni `master`. Les quatre règles
statiques de mika#2736 sont satisfaites, et le périmètre déclaré ne jugeait que
le dépôt.

Le mécanisme est une addition pure : une seconde ligne `ref=refs/heads/<branche
du dispatch>` dans l'entrée, un septième motif `push-scope-ref`, et un troisième
jeton de dégradation `scope-ref-absent`. Rien n'a été retiré ni déplacé. Ce qui
suit est ce qu'on n'apprend qu'en ajoutant une **seconde** dimension à un
périmètre déjà en service.

### L'union porte sur les ENTRÉES, pas sur les dimensions — et sur une seule dimension la question ne se pose pas

Le fait le moins évident de ce vol, et celui qui décide la conception. Le § *Le
lecteur partagé ne peut pas attribuer une requête à un spawn* établit que le
verdict porte sur l'**union des périmètres en vol**. Sur une seule dimension,
cette phrase est sans ambiguïté. Sur deux, elle se lit de deux façons, et elles
ne valent pas la même chose.

Deux dispatches concurrents, A sur `mika` / `feat/2776/branche`, B sur
`mika-platform` / `fix/9999/autre` :

| lecture | ce qu'elle autorise | verdict sur un push `mika` → `refs/heads/fix/9999/autre` |
|---|---|---|
| **par dimension** — union plate, `{mika, mika-platform} × {feat/2776/branche, fix/9999/autre}` | 4 couples, dont **2 que personne n'a déclarés** | **ALLOW** |
| **par entrée** — une entrée *est* un couple | 2 couples, exactement ceux déclarés | **DENY `push-scope-ref`** |

La lecture par dimension laisse donc ouvert **exactement le scénario que le vol
devait fermer**, dès qu'un second dispatch existe — c'est-à-dire dans l'état
normal d'un hôte de boucle. Elle ne resserrerait que l'hôte à un seul dispatch,
qui est le cas où le resserrement importe le moins. Elle se lit « resserré » et
ne resserre rien.

Le coût de la lecture juste est de **trois lignes** : le lecteur fait déjà un
seul balayage du répertoire, il suffit qu'il rende des couples au lieu de deux
ensembles plats (`_push_scope_repos()` → `_push_scope_entries()`, rendant
`list[tuple[str, str | None]]`). Il n'y avait donc pas d'arbitrage, il y avait
une lecture juste et une lecture fausse.

**Et ça impose une propriété au producteur, pas seulement au lecteur.** Les deux
lignes doivent voyager dans la **même** entrée, écrites par la **même**
redirection. Un lanceur qui éclaterait les deux dimensions d'un spawn sur deux
entrées rouvrirait le produit cartésien depuis l'autre bout, avec un lecteur
parfaitement correct. C'est `test_egress_push_scope_staging.sh` CASE 13 qui
l'asserte, et c'est le seul endroit qui le fait.

> **La leçon à réemployer : « l'union des déclarations vives » n'est une
> spécification que sur UNE dimension.** Dès la seconde, il faut dire si l'objet
> unionné est la dimension ou le tuple — et l'assertion qui défend la réponse
> n'est pas un test sur la structure de données, c'est le cas du croisement
> (CASE 26). Le jour où quelqu'un remet une union plate « pour simplifier », un
> test de structure le laisse passer et le cas du croisement rougit avec le
> scénario.

### Pourquoi la dimension branche remet en jeu la sonde de 4 octets, alors que la dimension dépôt ne le pouvait pas

Le vol 1 avait identifié le seul mode de défaillance capable d'**arrêter la
boucle** : un garde qui 403 le POST de sonde d'authentification de 4 octets
refuse du même coup tout push assez gros pour être streamé. Les deux cas
positifs qui l'épinglent voyagent donc sous chaque mécanisme ajouté à cette
surface, jamais hérités sur parole.

Pour la dimension **dépôt**, ce n'était qu'une discipline : le dépôt vient de
l'**URL**, donc le resserrement ne distingue structurellement pas une sonde d'un
push, et il n'y avait rien à craindre. Pour la dimension **branche**, la
propriété tombe : les refs viennent du **corps**, et une sonde d'authentification
n'en porte aucune. La liste de refs d'une sonde est **vide**.

Un verdict formulé comme « au moins une ref du push est dans le périmètre »
refuserait donc la liste vide, donc la sonde, donc tout push streamé. D'où
l'ordre des lignes du verdict, qui n'est pas cosmétique :

```python
if not declared:                        return _DENY_SCOPE_REPO, repo
if not refs:                            return _ALLOW, ""   # la sonde de 4 octets
if any(ref is None for ref in declared): return _DEGRADE_SCOPE_REF_ABSENT, ""
```

`if not refs` passe **avant tout jugement de branche**, et c'est la seule ligne
de ce verdict dont l'inversion arrête la boucle autonome.

> **La leçon : une propriété « structurellement impossible à violer » l'est par
> rapport à UN mécanisme, pas à une surface.** Le vol 2 pouvait démontrer que son
> resserrement ne touchait pas les sondes ; le vol 4, sur la même surface et avec
> le même interrupteur, ne pouvait plus. Ce qui rendait la démonstration valide
> était « le datum vient de l'URL » — et c'est précisément ce qui a changé.
> D'où la règle de forme qui a tenu les quatre vols : les deux positifs sont
> réassertés **sous chaque mécanisme**, et le vol qui les hérite est le vol qui
> les perd.

### Un `ref=` à valeur vide serait pire que son absence

`BRANCH` est vide en dispatch free-text et en dispatch de milestone, exactement
là où `REPO` l'est. La tentation est d'écrire la ligne quand même. Elle est
fausse dans la direction qui compte : le lecteur verrait une branche **déclarée**,
ne la retrouverait dans aucun push, et refuserait **tout** push de ce dispatch —
un 403 sur le travail légitime, le mode de défaillance qui arrête la boucle.

La ligne n'est donc pas écrite du tout, l'entrée reste **valide** avec son
`repo=` seul, et la dimension dépôt mord toujours. Deux corollaires :

- le lecteur traite une ligne `ref=` **à valeur vide** comme **absente** (`None`),
  et non comme une ref nommée « » : la posture fail-open du canal, appliquée à une
  entrée qu'un producteur futur écrirait mal ;
- un `BRANCH` déjà préfixé `refs/` tombe dans la même catégorie —
  `ref=refs/heads/refs/heads/…` ne nomme aucune ref existante, donc refuse tout.
  Même conséquence que la valeur vide, même remède : ne pas écrire la ligne.

**Et un troisième cas, qui n'est pas une dégradation mais un garde.** Le format
est `clé=valeur` **par ligne**. Un `BRANCH` portant un saut de ligne injecte donc
une ligne dans le registre — c'est-à-dire permet d'écrire un `repo=` arbitraire,
donc d'**élargir** le périmètre par le canal censé le resserrer. C'est de la
classe accident (`scripts/derive-branch-name` est l'unique producteur et rend un
slug), et elle se ferme pour une ligne de `case` : un nom hors
`[A-Za-z0-9._/-]` ne produit aucune ligne. Un garde qui s'écrit en une ligne et
qui ferme un desserrement n'a pas d'arbitrage à subir. `CASE 14` du producteur
l'asserte sur le **nombre de lignes** de l'entrée, pas sur une sous-chaîne —
chercher l'absence d'une sous-chaîne dans un fichier qu'on vient d'écrire ne dit
rien de ce qui s'y est ajouté ailleurs.

Les trois cas écrivent **trois phrases différentes** sur le `stderr` du dispatch,
et aucune n'est celle de `REPO` vide : celle-là dit « aucun périmètre déclaré »,
ces trois-là disent « périmètre déclaré, une dimension sur deux ». Quatre états,
quatre messages (classe mika#2205).

### Une entrée sans `ref=` est l'état NOMINAL pendant une fenêtre, pas une anomalie

Le registre de l'hôte survit au `make deploy`. Une entrée écrite par le binaire
précédent est donc là, légitimement, le temps que les spawns en vol se terminent
— au plus tard 12 h, par le TTL du vol 3. Le lecteur ne la refuse pas et ne la
jette pas : `ref is None` ⇒ aucun resserrement de branche.

Ce qui devait être écrit, c'est l'**observabilité** de cet état. Sans elle, « le
resserrement de branche ne mord jamais » et « il mord » rendent les mêmes octets :
des pushes qui passent, et zéro ligne. D'où le troisième jeton, et trois causes
**opposées** pour trois remèdes : `scope-unreadable` envoie l'opérateur sur la
comptabilité de l'hôte, `scope-absent` sur le lanceur qui n'a rien déclaré,
`scope-ref-absent` sur la **fenêtre de déploiement** — dont le remède nominal est
« attendre », c'est-à-dire aucun remède, ce qu'il faut savoir **avant** de
chercher.

Le jeton n'est émis que s'il y a au moins une ref à juger, sans quoi chaque sonde
de 4 octets porterait une ligne de dégradation.

**Et c'est la raison pour laquelle la forme rétrocompatible vit dans la RÈGLE et
jamais dans une allowlist.** Une exception nommée pour « entrée sans `ref=` »
serait stale par construction : elle réapparaît à chaque déploiement. Le registre
étant éphémère, il n'existait d'ailleurs aucun corpus à faire entrer par la
bande — mesuré avant déploiement plutôt qu'espéré.

### La comparaison des refs est sensible à la casse, et la symétrie avec le dépôt serait un desserrement

`repo=` est normalisé comme l'URL l'est — minuscules, suffixe `.git` retiré —
parce que GitHub est insensible à la casse sur `owner/repo` et qu'une comparaison
sensible refuserait un push légitime sur une majuscule.

`ref=` ne l'est **pas**. Git est sensible à la casse sur les refs :
`refs/heads/feat/2776/Dimension` et `refs/heads/feat/2776/dimension` sont deux
branches. Minusculiser ici ferait de deux refs distinctes une seule — un
**desserrement** déguisé en symétrie avec le voisin de la ligne du dessus.

L'arbitrage que ça assume : un dispatch dont la branche changerait **en cours de
vol** verrait son push refusé. Accepté, parce que `BRANCH` est dérivé une seule
fois par `_set_up_worktree` et que le spawn qui pousse est celui qui a déclaré —
le cas demanderait un changement de branche **à l'intérieur** du spawn, ce
qu'aucun chemin de dispatch ne fait. Le remède, s'il survenait, est le rollback
nommé, pas un assouplissement de la comparaison.

### La réparation du canari, qui est la même que celle du vol 2 et pour la même raison

Le canari assignait `REPO` depuis le vol 2 mais **pas** `BRANCH`, et la ref que
ses sondes poussent était codée en dur. Une fois `ref=` armé, son entrée ne
portait donc pas de `ref=`, le lecteur dégradait, et `push_bound_nominal` restait
**vert par accident** — mot pour mot le défaut que le bloc mika#2751 du canari
décrivait déjà pour la dimension dépôt.

La gravité n'est pas la ligne nominale : c'est qu'**aucune** sonde en vol du
resserrement de branche n'aurait existé. Les deux suites sont hors ligne par
construction (`mitmproxy` n'est pas une dépendance de test de ce dépôt, mesuré au
vol 1), donc l'absence de sonde ici est une absence **totale**, pas une
redondance en moins. Le jour où la dimension branche cesse silencieusement de
mordre, rien ne l'aurait dit.

Le canari déclare donc `BRANCH` à la ref que ses sondes poussent — ce qui rend sa
nominale verte **par décision** — et porte une sonde `push_scope_ref` : le dépôt
qu'il a déclaré, sur une branche qu'il n'a pas déclarée. Les quatre règles
statiques sont satisfaites, la dimension dépôt aussi, donc la dimension branche
est la **seule** chose qui peut la refuser. Sa ref porte `2776` pour la même
raison que le dépôt du vol 2 portait `2751` : aucun dispatch réel ne peut la
déclarer. Et `push-scope-ref` entre dans la table des motifs reconnus du canari,
parce qu'une sonde qui attend un motif doit pouvoir **rapporter** n'importe quel
autre — un motif absent de la table se lit « aucun refus », et c'est la direction
qui cache un resserrement tombé.

### Un seul interrupteur pour les deux dimensions, et c'est une décision

`MIKA_PILOT_EGRESS_PUSH_SCOPE=0` rend mika#2736 **exactement** : les deux
dimensions tombent ensemble. Un troisième nom sur cette surface serait un recours
plus gros que le problème — un opérateur qui veut « rendre mika#2736 » cherche un
interrupteur, pas une matrice. Et un interrupteur par dimension signifierait deux
lecteurs, qui divergeront le jour où ça compte (classe mika#2165). Le lanceur
reste à **zéro lecteur** de celui-ci, compté structurellement par le CASE 6 du
producteur ; la ligne `ref=` est donc écrite **inconditionnellement**, pour la
même raison que `repo=` : un nom de branche n'est pas un secret.

### Ce que ce vol ne ferme pas, et qui reste vrai mot pour mot

Le § *P4 — le force-push* ci-dessus n'est pas affecté. Resserrer la branche change
la **ref visée**, jamais la **nature** de la mise à jour : `receive-pack` ne porte
aucun drapeau de force, et la ref devenant la bonne ne rend pas visible qu'elle
est réécrite. P4 reste hors d'atteinte côté proxy pour une raison de protocole.

Le détail opérateur de cette moitié — le septième motif et son remède distinct, le
troisième jeton, les trois dégradations de branche du lanceur et la cinquième
ligne de la table du canari — vit à l'entrée `MIKA_PILOT_EGRESS_PUSH_SCOPE` de
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

### La troisième leçon, payée par mika#2766

Les deux premières portent sur **où** poser un garde et sur **quand** conclure
qu'il n'y a pas de place. La troisième porte sur la **valeur** qu'on met dedans :

> **Quand les deux directions d'une erreur ne coûtent pas la même chose, le
> nombre qui les sépare est un plancher ou un plafond — jamais un compromis.**

Un TTL se lit spontanément comme un arbitrage entre « trop de résidu » et « trop
de ménage », et un arbitrage se règle au milieu. Ici les deux directions sont
**incommensurables** : trop long relâche d'une quantité bornée que rien ne
matérialise ; trop court 403 un pilote innocent et arrête la boucle. Il n'y a
donc pas de milieu à trouver — il y a une borne à mesurer, et une marge à
prendre au-dessus d'elle. Le signe que le raisonnement est fait : la valeur
retenue cite une **mesure** (ici le plus long spawn sur 718 transcripts) et dit
ce qui serait **sous** la borne.

Le corollaire sur les tests, qui est celui qu'on oublie : **un test sur la
valeur reste vert sous n'importe quelle valeur.** Ce qui défend un plancher,
c'est un cas qui asserte le **coût de la mauvaise direction** — ici qu'une
entrée purgée sous un voisin vif refuse un push légitime. C'est la différence
entre un nombre vérifiable et un nombre décrété.

### La quatrième leçon, payée par mika#2776

Les trois premières portent sur **où** poser un garde, sur **quand** conclure
qu'il n'y a pas de place, et sur la **valeur** qu'on met dedans. La quatrième
porte sur ce qui arrive à une spécification quand on lui **ajoute une
dimension** :

> **Une spécification qui agrège (« l'union des déclarations vives ») n'a de sens
> univoque que sur UNE dimension. À la seconde, elle en a deux — et la lecture
> qui agrège par dimension autorise le produit cartésien, donc se lit resserrée
> sans resserrer.**

Rien dans la phrase ne change quand la seconde dimension arrive, et c'est tout
le piège : elle reste littéralement vraie, elle cesse d'être une
spécification. La relire comme déjà tranchée est le réflexe à casser. Deux
corollaires, mesurés ici :

- **la réponse contraint le producteur autant que le lecteur.** Un lecteur qui
  unionne correctement des tuples est désarmé par un producteur qui éclate les
  dimensions d'un même objet sur deux déclarations. La propriété « les deux
  lignes dans la même entrée » est donc du contrat, pas de la mise en forme — et
  elle veut son propre cas (§ *L'union porte sur les entrées*) ;
- **une propriété démontrée « structurellement impossible à violer » l'est par
  rapport à un mécanisme, jamais à une surface.** Le vol 2 pouvait prouver que
  son resserrement ne touchait pas les sondes d'authentification, parce que son
  datum venait de l'URL. Le vol 4, même surface et même interrupteur, ne pouvait
  plus : le sien vient du corps, qu'une sonde n'a pas. D'où la règle qui a tenu
  les quatre vols — les cas positifs critiques sont **réassertés sous chaque
  mécanisme**, et le vol qui les hérite sur parole est celui qui les perd.

# fix(loop-substrate) — vol 3 : le contrôle de fraîcheur des démons couvre le proxy egress

- **Ticket :** senara-solutions/mika#2780 (`bug`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`)
- **Branche :** `fix/2780/dispatch-le-contr-le-de-fra-cheur-des-d` (repartie de `main`, `0/0` contre `origin/main` au moment du groom)
- **Date :** 2026-10-18
- **Vol :** 3 sur 3 de la lignée mika#2029. **Prédécesseurs :** `2026-10-15-002-fix-2029-le-kill-sans-production-se-compte-par-son-nom-plan.md` (vol 1, mergé par #2720) et `2026-10-16-003-fix-2029-un-demon-ne-survit-plus-a-son-code-plan.md` (vol 2, mergé par #2746, commit `474c6b0a`). Le vol 2 a explicitement renvoyé ce livrable : « UN seul lanceur est câblé : le `mitmdump`. Le relais est le vol 3. »
- **Lignée :** mika#2041 (le socket orphelin qui satisfait `[ -S ]`) → mika#2049 (la posture *fail-closed* du relais) → mika#2051 (le pid comme jointure) → mika#2029 vols 1–2 (le mécanisme de fraîcheur) → **ce vol** (le second porteur) → mika#2778 (l'occurrence qui l'a rendu visible)

---

## Contexte

### Le défaut, en une phrase

`_ensure_pilot_egress_proxy` demande *« quelqu'un écoute-t-il sur le socket ? »*
et jamais *« quelle version écoute ? »*. Le verdicteur de fraîcheur existe depuis
le vol 2, il est testé, et il n'est interrogé que par le lanceur du `mitmdump` —
donc un correctif de `scripts/mika-pilot-egress-proxy` est **installé mais pas
servi**, et aucune ligne de log ne le dit.

### L'occurrence, mesurée

Le 2026-10-18, mika#2778 (corps chunked montant, merge `ca4be133`) ne modifie que
`scripts/mika-pilot-egress-proxy`. Après `make install`, le binaire installé est
identique à `main`. Le démon en service tournait depuis 16:26:35Z — donc avant
l'install — et aucun dispatch ne l'aurait relancé : il a fallu un `TERM` manuel
par pid, puis le canari. C'est la même classe que les trois occurrences du vol 2,
sur le second des deux porteurs.

### Ce que j'ai lu dans le code, aujourd'hui

**M1 — La sonde de vivacité retourne `0` sans poser la question du code.**
`dispatch-lib.sh:1521-1523` : `if _pilot_egress_sock_connectable
"$_PILOT_EGRESS_SOCK"; then return 0  # already alive`. Forme jumelle de celle
que le vol 2 a câblée pour `_ensure_pilot_helper` (`:1388`), et qui y est restée
inchangée ici.

**M2 — Le verdicteur est déjà écrit, mais sa clé d'entrée est un port TCP.**
`_pilot_daemon_freshness` (`:1282`) prend `$2 = port` et le passe à
`_pilot_daemon_holder_pid` (`:1213`), qui fait `fuser -n tcp "$1"`. Le relais
écoute sur un **socket unix** (`/tmp/mika-pilot-egress.sock`), donc aucune des
deux fonctions ne le voit en l'état.

**M3 — L'arrêteur a la même clé.** `_pilot_daemon_restart_by_port` (`:1336`)
fait `fuser -k -n tcp` puis attend la libération avec
`_pilot_daemon_port_connectable`. Les deux moitiés sont typées TCP.

**M4 — Chacune de ces trois fonctions n'a qu'UN appelant, et tous dans
`dispatch-lib.sh`.** Mesuré : `_pilot_daemon_holder_pid` → `:1286`,
`_pilot_daemon_restart_by_port` → `:1398`, `_pilot_daemon_port_connectable` →
`:1342` et `:1345`. Rien sous `scripts/`, rien dans le canari. La généralisation
est donc confinée à un fichier.

**M5 — Le proxy est mono-fichier et mono-processus.** `grep '^import\|^from'`
sur `scripts/mika-pilot-egress-proxy` ne rend que la bibliothèque standard
(`argparse asyncio datetime json os re signal socket ssl stat sys time pathlib`),
et `grep 'fork\|multiprocessing\|Popen\|subprocess'` ne rend **rien**. Le seul
fichier de code à comparer est donc `$_PILOT_EGRESS_PROXY_BIN`, et il y a un
seul pid porteur.

**M6 — Le seul autre fichier que le proxy ouvre est une donnée, pas du code.**
`:258` : `_CREDS_PATH.open("r")`, c'est-à-dire `~/.claude/.credentials.json`.

**M7 — Le canari tue déjà par identité de socket.**
`scripts/canary-pilot-containment:320-323` : `fuser -k "$_PILOT_EGRESS_SOCK"`,
avec le commentaire qui dit pourquoi ce n'est pas `pkill -f`. La forme existe
donc déjà dans le dépôt pour ce socket précis.

**M8 — Le harnais de fraîcheur existe et il est déjà câblé en CI.**
`test-dispatch-lib.sh:14135-14336` porte `_daemon_freshness_probe` et les sondes
V1–V4 ; `.github/workflows/ci.yml:85` appelle `make test-dispatch-lib`. L'exigence
mika#2578 (« un détecteur qu'aucun workflow ne lance ne refuse rien ») est donc
satisfaite **par placement** : les cas de ce vol atterrissent dans ce fichier et
rien n'est à câbler.

### Les quatre mesures faites aujourd'hui pour ce groom

Le ticket laisse au groom le choix de la forme d'identification du porteur du
socket, en demandant qu'elle soit **mesurée**. Elle l'est — quatre sondes sous
`.pilot-scratch/sockprobe/`, exécutées dans cette session de grooming :

**S1 — `fuser <chemin-de-socket-unix>` rend le pid du porteur, dans la même
forme que `fuser -n tcp <port>`.** Écouteur `AF_UNIX` réel, pid de vérité 339 :

```
fuser: rc=0 stdout='   339' stderr='/tmp/mika-sockprobe-2780.sock:\n'
```

Pids sur **stdout**, libellé sur **stderr**, espaces de tête — exactement ce que
la boucle de découpage de `_pilot_daemon_holder_pid` consomme déjà. Un repli par
`/proc/net/unix` + scan de `/proc/*/fd` rend le même pid (`['339']`) mais coûte
une soixantaine de lignes de python pour un gain nul : **il n'est pas retenu**.

**S2 — `fuser` n'apparie QUE le porteur, pas les clients connectés.** Écouteur
362, client connecté 363, sonde 355 :

```
server_pid=362 client_pid=363 self_pid=355
fuser: rc=0 stdout='   362'
```

C'est la mesure qui rend la forme sûre : un client apparié aurait fait comparer
le mtime du mauvais `/proc/<pid>`, et le `TERM` de l'arrêteur aurait visé un
pilote. La raison est structurelle — le client détient un socket anonyme d'un
autre inode, pas celui qui est lié au chemin.

**S3 — Un porteur hors du namespace PID courant rend `rc=1` et rien, donc
`undecidable`.** Mesuré sur le socket de **production** depuis le bac à sable de
ce groom : `sock_exists=True`, `fuser: rc=1 stdout=''`, `holder_pids=[]`. Le cas
`undecidable` du point 2 du ticket n'est donc pas théorique, et la dégradation
exigée (démon conservé, ligne de log) est la bonne. Le lanceur, lui, tourne sur
l'**hôte**, où le porteur est visible.

**S4 — Après la mort du porteur, la sonde de connectabilité dit bien
« libéré ».** Le chemin survit au `TERM` (`path_exists=True`,
`is_socket=True`) mais `connect()` échoue `ECONNREFUSED` :

```
after_TERM: path_exists=True is_socket=True connectable=no(111)
rebind_without_unlink=ERR errno=98 [Errno 98] Address already in use
```

L'attente bornée de la libération fonctionne donc telle quelle sur un socket
unix. La seconde ligne est la raison pour laquelle le lanceur n'a rien à changer :
le proxy délie lui-même un socket orphelin avant `bind()`
(`mika-pilot-egress-proxy:1384-1396`) et le délie aussi dans son handler
`SIGTERM` (`:1416-1462`).

---

## Décisions de grooming

**D1 — Un seul verdicteur, dont la clé devient un endpoint TYPÉ.** `$2` de
`_pilot_daemon_freshness` passe de `<port>` à `tcp:<port>` | `unix:<chemin>`, et
les trois fonctions typées TCP se généralisent sur la même clé. Le découpage est
`${1%%:*}` / `${1#*:}`, donc sur le PREMIER `:` : un chemin de socket qui en
contiendrait un passe quand même.

L'alternative — une fonction sœur `_pilot_daemon_freshness_unix` — a été écartée
parce qu'elle dupliquerait les **trois lignes de log nommées** et les trois
branches du verdicteur. Le dépôt a déjà une assertion qui compte ces lignes
(`T2029_VERDICT_BLOCK`, « le verdicteur écrit 3 lignes nommées ») : deux
verdicteurs demanderaient deux assertions qui divergeraient le jour où l'une des
deux est touchée. C'est le défaut de mika#2539 (deux tables de classification),
et le refuser ici coûte une ligne de `case`.

**Le vol 2 avait écarté un refactor voisin** (« elle DOUBLE délibérément les deux
sondes inline de `_ensure_pilot_helper` : les refactorer élargirait le diff sur le
seul chemin que ce vol modifie vraiment, pour un gain cosmétique »). La
distinction tient : là, le gain était cosmétique sur un chemin que le vol ne
touchait pas ; ici, le chemin TCP est traversé par la fonction que le vol doit
modifier de toute façon, et le gain est l'unicité du vocabulaire de log.

**D2 — Un endpoint mal typé rend `undecidable`, jamais un verdict inventé.** La
branche `*)` de chaque `case` rend `1` (porteur non identifiable), ce qui fait
tomber le verdicteur sur `undecidable` : démon conservé, ligne de log. Un `case`
sans branche par défaut aurait rendu `fresh` en silence sur une faute de frappe —
c'est-à-dire la panne de ce ticket, réintroduite par le correctif.

**D3 — Les fichiers de code sont `$_PILOT_EGRESS_PROXY_BIN` et lui seul.**
Mesuré en M5. `~/.claude/.credentials.json` (M6) est **exclu délibérément** : il
est réécrit à chaque rafraîchissement de jeton OAuth, donc plusieurs fois par
jour. Le compter ferait relancer le relais — et **couper les tunnels CONNECT en
vol** — à un rythme qui n'a aucun rapport avec une version de code. C'est une
donnée, pas du code, et la frontière est celle que le comparateur D2 du vol 2
suppose.

**D4 — Le libellé est `pilot-egress-proxy`.** C'est le nom que les quatre lignes
`pilot_egress_guard.*` du même fichier emploient déjà pour ce démon ; en inventer
un second (« relais », « egress ») donnerait deux noms au même organe dans le même
journal.

**D5 — Le comparateur de fraîcheur reste le mtime, pas un condensé.** Le ticket
note, en hors périmètre, que le mtime déclare périmé un `make install` qui recopie
un contenu identique, et suggère un condensé « si bon marché ». Il ne l'est pas :
un condensé exige un **sceau écrit au lancement**, donc serait aveugle au seul
démon qui compte aujourd'hui — celui qui tourne depuis des jours et n'a rien
scellé. C'est le raisonnement D2 du vol 2, inchangé, et changer de comparateur
pour un seul des deux porteurs créerait deux sémantiques de péremption. Coût
assumé : **une** relance par deploy, à l'instant même où l'on veut que le démon
reprenne son code. Consigné en résidu dormant avec sa condition de réveil.

**D6 — Trois sondes, pas quatre.** Le ticket exige deux cas (périmé sans frère ⇒
relancé ; périmé avec frère ⇒ conservé + `stale_deferred`). Une troisième est
ajoutée : le **contrôle négatif** `fresh`, qui est aussi l'assertion
auto-nettoyante exigée par la section Fire-Disposition — sans elle, un prédicat
qui déraperait vers « toujours périmé » serait un redémarreur permanent déguisé en
garde, et il couperait les tunnels de chaque dispatch. Le quatrième état
(`undecidable`) est une propriété du **verdicteur partagé**, déjà tenue par V4 :
sa conséquence côté relais est identique (conservation, `return 0`), et la
dupliquer coûterait un état de harnais pour mesurer le même chemin.

**D7 — Pas de garde ajoutée contre un correctif cassé.** Le mode de panne nouveau
que ce vol introduit est : `TERM` réussi, relance qui ne bind pas, donc dispatch
refusé (posture fail-closed) là où un démon périmé servait. Une pré-vol par
`compile()` sur le binaire l'attraperait en partie. Elle n'est **pas** retenue :
`n=0` sur cette classe, et le précédent maison tranche dans ce sens au mot près —
vol 2, sur l'escalade de signal : « un `mitmdump` qui ignore un `TERM` est un état
que personne n'a mesuré, et une escalade de signal se déciderait sur une mesure,
pas sur un plan. » Le risque est nommé en R1 avec les trois bornes qui le
réduisent déjà.

---

## Conception

### C1 — `_pilot_daemon_holder_pid` prend un endpoint typé

```sh
_pilot_daemon_holder_pid() {   # $1 = `tcp:<port>` | `unix:<chemin>`
    command -v fuser >/dev/null 2>&1 || return 1
    local _kind="${1%%:*}" _ref="${1#*:}" _out _pid
    case "$_kind" in
        tcp)  _out="$(fuser -n tcp "$_ref" 2>/dev/null)" || true ;;
        unix) _out="$(fuser "$_ref" 2>/dev/null)" || true ;;   # S1, S2
        *)    return 1 ;;                                       # D2
    esac
    for _pid in $_out; do ... done   # boucle R3 du vol 2, inchangée
}
```

La boucle de découpage ne change pas : S1 mesure que la forme de sortie de
`fuser` sur un chemin est identique à celle sur un port.

### C2 — `_pilot_daemon_endpoint_connectable`, aiguilleur des deux sondes

```sh
_pilot_daemon_endpoint_connectable() {   # $1 = endpoint, $2 = timeout
    case "${1%%:*}" in
        tcp)  _pilot_daemon_port_connectable "${1#*:}" "${2:-0.25}" ;;
        unix) _pilot_egress_sock_connectable "${1#*:}" "${2:-0.25}" ;;
        *)    return 1 ;;
    esac
}
```

Les deux sondes existent déjà (`:1310` et `:1463`) et ne sont pas touchées.
`_pilot_egress_sock_connectable` est définie **après** cet aiguilleur dans le
fichier : en bash, une fonction est résolue à l'appel et non à la définition, donc
la référence avant définition est légale. Le commentaire doit le dire, pour qu'un
relecteur ne la lise pas comme un bug à l'inverse.

### C3 — `_pilot_daemon_restart_by_port` devient `_pilot_daemon_restart_by_endpoint`

Renommage (M4 : un seul appelant) et deux `case` :

```sh
    case "$_kind" in
        tcp)  fuser -k -n tcp "$_ref" 2>/dev/null || true ;;
        unix) fuser -k "$_ref" 2>/dev/null || true ;;   # M7 : forme du canari
        *)    return 1 ;;
    esac
```

L'attente bornée (3 s, `SECONDS`) et le `stale_unstoppable` ne changent pas de
structure ; la sonde de libération passe par C2. S4 mesure que cette attente
termine bien sur un socket unix (`ECONNREFUSED` dès la mort du porteur) au lieu de
tourner jusqu'à sa fenêtre pleine. La ligne `stale_unstoppable` nomme désormais
l'endpoint (`unix:/tmp/…` ou `tcp:8892`) au lieu du port en dur : aucun test
n'apparie ce texte (l'assertion `T2029_VERDICT_BLOCK` ne compte que le bloc du
verdicteur, et `stale_unstoppable` vit chez l'arrêteur), mais le § 4.5 de la doc
le cite et doit suivre.

### C4 — Le câblage, dans le chemin « already alive » du relais

À `dispatch-lib.sh:1521`, sur le patron exact de `:1388` :

```sh
    if _pilot_egress_sock_connectable "$_PILOT_EGRESS_SOCK"; then
        case "$(_pilot_daemon_freshness pilot-egress-proxy \
                    "unix:$_PILOT_EGRESS_SOCK" "$_PILOT_EGRESS_PROXY_BIN")" in
            "$_PILOT_DAEMON_STALE")
                _pilot_daemon_restart_by_endpoint \
                    "unix:$_PILOT_EGRESS_SOCK" pilot-egress-proxy || return 0
                ;;
            *) return 0 ;;
        esac
    fi
```

Trois propriétés, chacune portant une conséquence :

- **Le placement est APRÈS le contrôle `[ ! -x "$_PILOT_EGRESS_PROXY_BIN" ]`**
  (`:1506`), donc la fraîcheur n'est jamais interrogée sur un binaire dont
  l'absence a déjà été nommée — même raisonnement, même placement que pour le
  `mitmdump`.
- **`|| return 0` sur l'arrêt impossible, et `_PILOT_EGRESS_ABORT` reste vide.**
  C'est la différence qui compte avec le `mitmdump` : ici un `return 1` serait un
  **refus de dispatch**. Un démon périmé qui sert doit rendre `0` sans motif
  d'abandon.
- **Sur `stale`, pas de `return` :** on retombe dans le lanceur et sa sonde de
  bind bornée à 3 s, inchangés — donc un relais qui ne revient pas produit la
  ligne `pilot_egress_guard.unreachable` existante et la posture fail-closed
  héritée du site d'appel. Ce vol n'invente aucune posture et n'ajoute aucun
  chemin de refus ; il fait seulement qu'un chemin de refus existant devient
  **atteignable** par une cause nouvelle (R1).

### C5 — Le harnais de test, jumeau et non partagé

`_relay_freshness_probe <état>` dans `test-dispatch-lib.sh`, calqué sur
`_daemon_freshness_probe` et placé juste après lui :

- un faux relais exécutable qui inscrit son pid puis `exec python3` un écouteur
  `AF_UNIX` sur le chemin reçu en argument (le vrai lanceur passe `--host-unix
  --socket <chemin>`), dans un `mktemp -d` ;
- `_PILOT_EGRESS_PROXY_BIN`, `_PILOT_EGRESS_SOCK`, `MIKA_PILOT_EGRESS_LOG_DIR` et
  `PILOT_LOG_DIR` pointés dans ce répertoire — **jamais** `/tmp/mika-pilot-egress.sock`,
  qui est le socket de production de l'hôte de l'opérateur (même règle R4 que le
  port 8892 côté mitmdump) ;
- amorçage par un premier `_ensure_pilot_egress_proxy`, puis `sleep 1.1` et
  `touch` du binaire pour les états périmés (la seconde pleine met la comparaison
  hors de portée d'un `[ -nt ]` qui ne comparerait que les secondes) ;
- `touch "$logdir/frere.stderr"` pour l'état `deferred` ;
- rend la même ligne de mesure que son jumeau :
  `rc=<n> token=<…> launches=<n> first=<alive|dead> listening=<yes|no>`.

**Pourquoi un jumeau et non un harnais bimodal.** Rendre
`_daemon_freshness_probe` bimodal demanderait de paramétrer son faux démon, ses
quatre variables de lanceur, la fonction appelée et sa sonde finale — un refactor
de 110 lignes dont le chemin `mitmdump` est **hors périmètre** de ce ticket, et
dont la casse se lirait comme une régression du vol 2. C'est le précédent du vol 2
sur les sondes inline, appliqué dans le même sens.

**Le `SKIP` bruyant est repris** (précédent mika#2149) : sans `fuser`, la
découverte du porteur rend `undecidable` partout et les sondes mesureraient le
même rien. Le bloc du relais est gardé par le même `command -v fuser` que celui du
`mitmdump`.

### C6 — Le contrôle de contre-vacuité

Le harnais prend, comme son jumeau, un second paramètre `lib` optionnel. Pointé
sur une copie de `main` extraite sous `.pilot-scratch/`, l'état `stale` doit
rendre `token=none launches=1 first=alive listening=yes` : aucun jeton, aucune
relance, le porteur périmé sert encore — la panne de ce ticket reproduite à la
demande. **Constatée et consignée dans le commentaire et dans le corps de PR, non
assertée en permanence** : l'exécuter exigerait de lire une autre révision depuis
la suite, donc un test vert hors réseau et rouge en archive détachée. Exactement
le statut de V5 du vol 2.

---

## Étapes d'implémentation

Un seul commit : la règle et ses cas atterrissent ensemble (mika#2578).

1. **C1** — `_pilot_daemon_holder_pid` sur endpoint typé, commentaire citant S1 et
   S2 (la mesure est ce qui rend la forme défendable, et elle doit être lisible au
   point d'appel).
2. **C2** — `_pilot_daemon_endpoint_connectable`, avec la note sur la résolution
   à l'appel.
3. **C3** — renommage en `_pilot_daemon_restart_by_endpoint` + les deux branches
   de `kill` ; migration de l'unique appelant (`:1398`).
4. **C1 bis** — `_pilot_daemon_freshness` : `$2` devient l'endpoint, la ligne
   `undecidable` le nomme sous sa forme typée ; migration de l'unique appelant
   `mitmdump` (`:1388`) vers `tcp:$_PILOT_HELPER_PORT`.
5. **Vérification du chemin hors périmètre, AVANT d'aller plus loin** :
   `make test-dispatch-lib` — les sondes V1–V4 du `mitmdump` doivent rester
   vertes. Si l'une rougit ici, le refactor D1 est en cause et rien d'autre.
6. **C4** — le câblage du relais.
7. **C5** — `_relay_freshness_probe` et les trois sondes R1–R3.
8. **C6** — exécuter le contrôle de contre-vacuité contre `origin/main`, consigner
   le résultat dans le commentaire et dans le corps de PR.
9. **Doc** — § 4.5 de `docs/operator/pilot-egress-relay.md` (voir ci-dessous).
10. `bash -n` sur les deux fichiers, `make test-dispatch-lib`,
    `scripts/verify-pipeline.sh origin/main`.

### La doc, précisément

`docs/operator/pilot-egress-relay.md` § 4.5 — c'est là que mika#2029 est
documenté pour le `mitmdump`, et la section porte aujourd'hui la phrase à
retirer : « **Ce vol ne câble que le `mitmdump`** ; le relais garde ses gestes
manuels du § 4.2 (vol 3, suivi dans senara-solutions/mika#2029). » Les
modifications :

- les deux porteurs sont nommés, avec pour chacun ses fichiers de code
  (`mitmdump` + deux addons ; le binaire du relais seul) et son libellé de log ;
- l'occurrence du 2026-10-18 (mika#2778) rejoint les trois dates déjà citées ;
- la **conséquence propre au relais** est écrite : sa posture est fail-closed,
  donc une relance qui ne bind pas se lit en `pilot_egress_guard.unreachable` et
  **refuse le dispatch** — le remède est celui du § 4.2, déjà écrit ;
- `stale_unstoppable` : le geste de diagnostic devient `fuser -v <socket>` pour le
  relais, `fuser -v -n tcp 8892` restant celui du `mitmdump` ;
- § 4.2 : une phrase disant que `--restart-relay` reste le geste du relais
  **wedgé** (que la fraîcheur ne détecte pas : un relais wedgé répond et son code
  peut être à jour) et n'est plus le seul chemin de relance après un deploy.

---

## Contrat de vérification

| # | Ce qui est affirmé | Comment on le mesure |
|---|---|---|
| V-A | Le relais périmé **sans** frère en vol est relancé | sonde R2 : `rc=0 token=stale launches=2 first=dead listening=yes` |
| V-B | Le relais périmé **avec** frère en vol est conservé, et la ligne le dit | sonde R3 : `rc=0 token=stale_deferred launches=1 first=alive listening=yes` |
| V-C | Un relais plus récent que son code n'est **pas** relancé, en silence | sonde R1 : `rc=0 token=none launches=1 first=alive listening=yes` |
| V-D | Le chemin `mitmdump` est inchangé | V1–V4 vertes avant et après (étape 5) |
| V-E | Le verdicteur reste unique et porte exactement 3 lignes nommées | assertion `T2029_VERDICT_BLOCK` existante, inchangée et verte |
| V-F | Les sondes mesurent le correctif et non le décor | contrôle de contre-vacuité C6 contre `origin/main` : `token=none launches=1` sur l'état de R2 |
| V-G | Aucun chemin de refus nouveau n'est ajouté | lecture : le seul `return 1` nouveau est celui de l'arrêteur, consommé par `|| return 0` ; `_PILOT_EGRESS_ABORT` n'est assigné sur aucun chemin nouveau |
| V-H | Le fichier parse | `bash -n` sur `dispatch-lib.sh` et `test-dispatch-lib.sh` (la première est déjà assertée dans la suite) |

---

## Risques nommés

**R1 — Posture fail-closed : un refus de dispatch là où un démon périmé
servait.** Si le `TERM` réussit et que la relance ne bind pas, le dispatch est
refusé. Trois bornes existent déjà et aucune n'est ajoutée : la **déférence**
(aucun `TERM` sous un frère en vol), `stale_unstoppable` (démon conservé si
l'endpoint répond encore après 3 s), et le fait que la relance passe par le
lanceur **déjà** en service — un relais incapable de binder aujourd'hui ne
survivrait pas davantage à son prochain démarrage, c'est une panne préexistante
que la fraîcheur révèle plutôt qu'elle ne crée. Le remède est écrit (§ 4.2,
`--restart-relay`) et la ligne de refus le nomme. Voir D7 pour pourquoi aucune
garde supplémentaire n'est posée.

**R2 — Tuer le relais coupe les tunnels CONNECT en vol.** C'est la classe de
panne que mika#2029 existe pour éteindre, et la déférence la couvre en
sur-approximant : tout `*.stderr` de `$_PILOT_LOG_DIR` de moins de 300 s diffère
la relance. Le résidu — un pilote qui vole mais dont le `.stderr` est silencieux
depuis plus de 300 s — est déjà en train d'être tué par son propre watchdog
(`idle_timeout`, cpp#170), ce qui est la justification de la fenêtre au vol 2. Le
proxy, lui, unwind proprement sur `SIGTERM` même avec un tunnel établi
(`server.close()` et non `wait_closed()`, mesuré et commenté à
`mika-pilot-egress-proxy:1437-1449`).

**R3 — Le refactor D1 traverse le chemin `mitmdump`, hors périmètre.** Mitigé par
M4 (un seul appelant par fonction, tous dans ce fichier) et par l'étape 5, qui
fait passer V1–V4 **avant** d'écrire la moindre ligne du relais : une rougeur à ce
moment-là n'a qu'une cause possible.

**R4 — Un faux positif de `fuser` aurait fait tuer un client.** Réfuté par S2 :
seul le porteur est apparié. S'il fallait un jour s'en défier, le signal serait un
`stale` suivi d'un dispatch mort, et le repli mesuré est le scan
`/proc/net/unix` + `/proc/*/fd` de S1.

**R5 — Le harnais touche le socket de production.** Refusé par construction : le
chemin du socket est dans un `mktemp -d` (C5), et la règle est celle que le vol 2
s'est donnée pour le port 8892. Rien dans le harnais n'appelle `fuser -k` : il tue
par pid, et seulement les pids que son propre faux démon a inscrits.

---

## Hors périmètre

- **La fraîcheur du `mitmdump`** — inchangée en sémantique. Seule la forme de sa
  clé d'endpoint change (D1), et V1–V4 l'attestent.
- **La relance par `make install`** — nommée hors périmètre par le ticket. Ce vol
  la rend inutile pour le relais : le prochain dispatch sans frère en vol relance.
- **Le passage au condensé de contenu** comme comparateur de péremption (D5).
- **La détection du relais *wedgé*** — elle demanderait une sonde applicative de
  bout en bout sur le chemin critique de chaque dispatch ; coût et taux de faux
  positifs non mesurés, déjà renvoyée par mika#2049 (§ 4.2 de la doc).
- **L'extension de `scripts/canary-pilot-containment --restart-relay`** — il tue
  déjà par identité de socket (M7) et n'a rien à apprendre de ce vol.

---

## Résidus dormants, avec leur condition de réveil

- **Une relance par deploy, contenu identique ou non** (D5). *Réveil :* si le
  journal du relais montre que ces relances coupent des tunnels de façon mesurable
  — c'est-à-dire si `stale` est suivi d'un `pilot_egress_guard.unreachable` plus
  d'une fois. Le remède serait un sceau de version écrit au lancement, pour les
  **deux** porteurs à la fois, jamais pour un seul.
- **`undecidable` répété à chaque dispatch** (S3). *Réveil :* la ligne apparaît à
  chaque dispatch sur l'hôte. Cause attendue : `psmisc` absent, ou un lanceur
  appelé depuis un namespace PID qui ne voit pas le porteur. Le second cas
  demanderait de passer par `/proc/net/unix`, et c'est le repli mesuré de S1.
- **Le repli `/proc/net/unix` + scan de `/proc/*/fd`**, mesuré et écarté (S1).
  *Réveil :* `fuser` indisponible sur un hôte de dispatch, ou S2 réfuté par une
  mesure contraire.

---

## Definition of Done

1. `_pilot_daemon_freshness` est interrogé par **les deux** lanceurs de démons de
   `dispatch-lib.sh`, par une clé d'endpoint typée et un seul verdicteur.
2. Un relais dont le binaire est plus récent que son porteur est relancé au
   dispatch suivant sans frère en vol, et la ligne `pilot_daemon_freshness.stale`
   le dit avec le libellé `pilot-egress-proxy`.
3. Aucune relance ne part sous un pilote frère en vol : `stale_deferred`.
4. Un porteur non identifiable laisse le démon en place et écrit
   `undecidable`.
5. Aucun chemin de refus de dispatch nouveau n'est ajouté (V-G).
6. Les trois sondes R1–R3 sont dans `test-dispatch-lib.sh`, dans le même commit
   que la règle, et CI les lance déjà (M8).
7. V1–V4 du `mitmdump` restent vertes.
8. Le § 4.5 de `docs/operator/pilot-egress-relay.md` décrit les deux porteurs, et
   la phrase « ce vol ne câble que le `mitmdump` » a disparu.
9. `make test-dispatch-lib`, `cargo test`, `cargo clippy`, `cargo fmt --check`,
   les lints du dépôt et `scripts/verify-pipeline.sh origin/main` passent.

---

## Acceptance criteria

Dérivées du § « Travail attendu » du ticket, qui ne porte pas de section
`## Acceptance criteria` formelle.

- **AC1** — La fraîcheur de mika#2029 couvre le proxy egress avec la même
  sémantique que le `mitmdump` : fichier de code plus récent que le démarrage du
  porteur ⇒ `stale` ; relance différée tant qu'un pilote frère vole ⇒
  `stale_deferred` ; lignes `pilot_daemon_freshness.*` sur stderr portant le
  libellé du proxy.
- **AC2** — Le porteur du **socket unix** est identifié par un moyen mesuré, et
  la mesure est consignée dans ce plan (S1, S2, S3, S4). Un porteur non
  identifiable conserve le comportement `undecidable` : démon conservé, ligne de
  log.
- **AC3** — Les fichiers de code comparés sont `$_PILOT_EGRESS_PROXY_BIN` et tout
  fichier qu'il charge ; la mesure M5/M6 établit qu'il n'en charge aucun autre, et
  D3 dit pourquoi le fichier de credentials en est exclu.
- **AC4** — Deux cas CI au moins atterrissent dans le commit qui crée la règle, à
  côté des cas de fraîcheur `mitmdump` existants : porteur périmé sans frère ⇒
  relancé ; porteur périmé avec frère ⇒ conservé + ligne `stale_deferred`.
- **AC5** — Le comportement est documenté là où mika#2029 l'est pour le
  `mitmdump` (§ 4.5 de `docs/operator/pilot-egress-relay.md`).

---

## Fire-Disposition

Ce vol livre **trois** détecteurs au sens de mika#2306 — les sondes R1, R2, R3,
dont le chemin de succès est « aucune violation trouvée ». La section est donc
requise.

**Option retenue : (a) exception nommée en allowlist — l'allowlist est VIDE, et
l'exception nommée est le motif `stale_deferred`.**

C'est la disposition du vol 2, pour la même raison, et le dépôt a déjà le
précédent d'une table d'exceptions vide assertée comme telle
(`T2701_SOURCE_SCAN_ALLOWLIST=()` avec son `assert_eq … "0"`).

**Pourquoi l'allowlist est vide.** La population en violation à l'atterrissage est
connue et mesurée : le relais de l'hôte de dispatch, démarré le 2026-10-18 à
16:26:35Z, portant un binaire réinstallé après lui (corps du ticket, et
`bin_mtime` relevé en S3). Elle n'est **pas exceptée, elle est réparée** — par
l'action du détecteur, au premier dispatch sans frère en vol. Une entrée
d'allowlist la nommant désignerait un pid qui n'existe plus dès que la garde a
tourné une fois, et l'assertion auto-nettoyante que (a) exige la rendrait rouge
immédiatement : ce serait écrire un instrument qui mentirait.

**L'exception réellement portée est `stale_deferred`**, avec les trois propriétés
que (a) demande :

- **elle nomme la donnée précise** — un `*.stderr` de `$_PILOT_LOG_DIR` modifié
  depuis moins de 300 s, pas « un pilote tourne peut-être » ;
- **elle référence un suivi** — la ligne porte `mika#2029`, dont ce vol est le
  troisième et dernier ;
- **elle est auto-nettoyante par construction** — aucune main ne la retire : le
  prochain dispatch sans frère en vol relance et le motif disparaît. Une exception
  devenue stale ici ne peut pas rester silencieuse, elle se transforme en `stale`.

**L'assertion auto-nettoyante du détecteur lui-même est R1**, le contrôle négatif
de D6 : un relais plus récent que son code lit `fresh` et **n'est pas relancé**.
Elle rougit dans la direction dangereuse — un prédicat qui déraperait vers
« toujours périmé » relancerait le relais à chaque dispatch, coupant les tunnels
en vol, et sur ce porteur-ci une relance ratée **refuse le dispatch** (R1 des
risques). Un détecteur de péremption sans ce contrôle est un redémarreur permanent
déguisé en garde.

**Un `SKIP` déclaré, pas une exception.** Sans `fuser`, le bloc entier se déclare
sauté avec son remède (`psmisc`), sur le précédent mika#2149 : un bloc de porte
qui n'a pas pu s'armer ne doit pas se lire comme un vert.

Les trois détecteurs atterrissent donc **armés**, avec la seule violation connue
réparée par leur propre action, et sans dette à ouvrir.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — endpoint typé : `_pilot_daemon_holder_pid` (C1), `_pilot_daemon_endpoint_connectable` (C2), `_pilot_daemon_restart_by_endpoint` (C3), clé de `_pilot_daemon_freshness` + migration de l'appelant `mitmdump`, commentaires S1/S2/D1/D2 inclus | 85 |
| `dispatch-lib.sh` — câblage du chemin « already alive » du relais (C4) + le commentaire de posture fail-closed et de `|| return 0` | 45 |
| tests (`test-dispatch-lib.sh`) — `_relay_freshness_probe` à trois états, faux relais `AF_UNIX`, sondes R1–R3, garde `SKIP` et commentaire de contre-vacuité | 135 |

Total estimé : 265 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 265 contre 429 : 38 % de marge, et 265 contre le
seuil par PR de 1000. Le découpage en vols est donc refusé, et trois propriétés
mesurées rendent cette estimation moins fragile que les trois morts du 2026-10-14 :

- **deux fichiers touchés, zéro créé**, et les deux sont ceux que le vol 2 a déjà
  ouverts il y a deux jours ;
- **le mécanisme n'est pas écrit, il est généralisé** — les cinq jetons, le
  comparateur de mtime, la déférence, l'attente bornée de libération existent et
  ne bougent pas. Le vol 2 a dépensé 187 lignes de `dispatch-lib.sh` pour les
  créer ; ce vol en modifie quatre signatures et ajoute un `case` de six lignes ;
- **la forme du porteur du socket est déjà choisie et mesurée** (S1–S4), donc
  l'implémentation ne porte aucune exploration — c'est le poste de coût qui a
  explosé sur mika#2683.

**La calibration, dite franchement.** Le vol 2 avait estimé 184 et livré 382
lignes hors `docs/` (`474c6b0a` : 187 + 195), soit un facteur **2,1**. Appliqué
tel quel ici : 265 × 2,1 ≈ 557 lignes réelles. La borne de 429 est en lignes
**estimées** et absorbe déjà un facteur 3 dans sa dérivation
(`× 9 / 3`), donc 265 × 3 = 795 reste la tolérance implicite — 557 passe
dessous avec de la marge. Le poste qui a dérapé au vol 2 est le harnais de test
écrit de zéro (72 estimées, 195 réelles) ; ici il est estimé à 135 en connaissant
ce dépassement, et borné par D6 à **trois** états au lieu de quatre.

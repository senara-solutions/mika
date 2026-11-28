# fix(3051) — le canari interroge « un pilote vole-t-il ? » avant d'installer quoi que ce soit

**Ticket :** senara-solutions/mika#3051
**Labels :** `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
**Un seul vol.** Un seul dépôt (`mika`).

## Problème

`scripts/canary-pilot-containment` installe le proxy d'egress dans
`~/.local/bin` à sa ligne 81, **inconditionnellement et avant tout contrôle**.
L'installation réécrit le binaire, ce qui rend son mtime plus récent que le
démarrage du démon en place ; `_pilot_daemon_code_is_newer` juge alors le proxy
périmé et `_ensure_pilot_egress_proxy` le redémarre. Le nouveau pid prend le
socket, et les tunnels CONNECT du pilote qui vole à cet instant sont coupés.

Mesuré une fois, le 2026-11-01 (corps du ticket) : installation à 13:57:51Z,
relance à 13:57:52Z, `API Error: 502 … inference gateway` au tour 18 du pilote
en vol à 14:00:53Z, sortie sans plan (18 tours, 1,35 $), relance du groom par le
moteur à 14:36:00Z. Depuis, le défaut est masqué par une **règle opératrice** —
« canari seulement sur moteur vide », un contrôle à la main avant chaque canari
— qui a coûté ~40 min de report au canari de `3cafaa9d2` le 06/11.

Le défaut n'est pas dans le contrôle de fraîcheur du proxy : depuis mika#3031,
`_pilot_daemon_freshness` **diffère** déjà sa relance quand un pilote frère vole
(jeton `stale_deferred`). Le défaut est que le canari, lui, ne pose jamais la
question : il écrit le fichier dont la fraîcheur sera ensuite évaluée, et il
l'écrit avant d'avoir chargé le prédicat qui sait répondre.

### Ce que ce vol ne corrige pas

Hors périmètre, nommé par le ticket :

- le contrôle de fraîcheur du proxy pendant un `make install` (mika#2029,
  mika#3031) — il n'est pas touché ;
- une garde sur les tâches en file du moteur (`tasks` pending / in_progress) :
  le cas mesuré tient au seul pilote **vivant**, et sur-approximer vers la file
  gèlerait le canari sur un hôte de boucle normal.

## Site

| élément | lieu actuel | lecture |
|---|---|---|
| installation du proxy | `scripts/canary-pilot-containment:81` | `install -Dm755`, inconditionnelle, avant tout contrôle. **Site unique** dans le fichier (vérifié : un seul `install -D`) |
| chargement de dispatch-lib | `scripts/canary-pilot-containment:268` | après l'installation — donc le prédicat n'est pas encore défini à la l. 81 |
| détecteur de vol | `skills/bundled/_shared/dispatch-lib.sh:1346` `_pilot_sibling_pilot_in_flight` | 0 = un pilote vole, 1 = aucun, 2 = indécidable |
| répertoire de logs | `dispatch-lib.sh:246` `_pilot_log_dir` | `${PILOT_LOG_DIR:-/var/log/claude-pilot}` — **surchargeable par environnement**, ce qui rend le test possible hors hôte de boucle |
| modes du canari | `:273` `--show-args`, `:285` `--ensure-relay`, `:316` `--restart-relay`, `:360` `--enter`, défaut `:394` | **les cinq** touchent au relais ou au proxy : `--show-args` et `--enter` passent par `_run_pilot_sandboxed`, qui appelle `_ensure_pilot_helper` + `_ensure_pilot_egress_proxy` ; les deux modes relais les appellent en clair ; le mode défaut les appelle aux l. 394-395 |

Deux faits mesurés qui rendent le geste court :

1. **Rien, entre la l. 81 et la l. 268, ne lit le proxy installé.** `$INSTALLED_PROXY`
   n'est relu qu'à la l. 419 (`echo`/`stat`), dans le mode défaut. Le bloc
   mika#2578 (décoy, outil de condensé, empreinte *avant*, trap `EXIT`,
   `mktemp -d`, `export _PILOT_GH_TOKEN_FILE`) n'en dépend pas. L'installation
   est donc **déplaçable** après le `source` sans toucher à l'ordre que
   mika#2578 a posé — et c'est ce sens du déplacement qu'on prend, plutôt que de
   remonter le `source` à travers ce bloc.
2. **Le `source` est inerte.** Les seules lignes exécutables au niveau supérieur
   de `dispatch-lib.sh` sont des corps de *heredoc* et de chaîne (python inline,
   prologues de sandbox) ; aucun `_ensure_*` ne tourne au chargement. Et le
   canari le source déjà avant tout mode. Charger plus tôt ne touche donc rien.

## Approche

Une garde unique, posée **immédiatement après le `source`** et **avant
l'installation**, qui interroge le prédicat partagé et refuse fail-closed.
Parce qu'elle précède l'installation, et que l'installation précède tous les
modes, elle couvre les cinq modes par construction — un seul geste, pas cinq.

### A. Le prédicat publie le `.log` qui l'a décidé

`_pilot_sibling_pilot_in_flight` rend aujourd'hui un **code** et rien d'autre.
AC1 demande que le refus **nomme le `.log` du pilote vivant** ; le canari ne
peut pas le retrouver sans rejouer le prédicat, et un second `find` nommerait
possiblement un autre fichier que celui qui a décidé — un instrument qui ment.

Le chemin voyage donc par une **variable globale**, `_PILOT_SIBLING_IN_FLIGHT_LOG`,
assignée dans la boucle juste avant le `return 0`, et remise à vide en tête de
fonction (sinon un appel qui rend 1 laisserait en place le nom d'un vol fini).

**Pas sur stdout, et c'est l'arête du changement :** l'unique appelant actuel
est `_pilot_daemon_freshness:1397`, dont le stdout **est** le canal du jeton de
verdict (`printf '%s' "$_PILOT_DAEMON_STALE_DEFERRED"`), capturé par ses propres
appelants. Un `printf` du chemin dans le prédicat corromprait ce jeton en
silence. Le commentaire de la fonction le dira, à cet endroit précis.

**Les scans de mika#3031 restent verts, et il faut le dire parce qu'un lecteur
s'en inquiétera :** `T3031_S1_DEFS`/`T3031_S1_MENTIONS`
(`test-dispatch-lib.sh:15638-15643`) assertent `defs=1 appels=1` **sur
`$DISPATCH_LIB` seul**. Le nouvel appelant vit dans `scripts/canary-pilot-containment`,
hors de ce fichier ; et la variable publiée est en majuscules, donc hors du motif
(`grep -cE '_pilot_sibling_pilot_in_flight'`, sensible à la casse). S1 interdit
un **second prédicat**, pas un second lecteur : le canari réutilise exactement
celui que S1 protège. Aucune retouche de S1 n'est due, et aucune n'est faite.

### B. La garde du canari

Une fonction `_canary_refuse_if_pilot_in_flight`, appelée inconditionnellement
après le `source` :

| code du prédicat | geste | sortie |
|---|---|---|
| 0 — un pilote vole | refus, nomme `$_PILOT_SIBLING_IN_FLIGHT_LOG`, nomme le geste (attendre la fin du vol) | **3** |
| 2 — indécidable | refus fail-closed, nomme la cause (le prédicat a déjà écrit sa ligne `pilot_sibling_in_flight.undecidable` sur stderr) | **4** |
| 1 — aucun | une ligne `ok:   no_sibling_pilot_in_flight`, et le canari continue | — |

**Deux codes, pas un**, et tous deux distincts de `1` : AC1 veut un code
différent de l'échec d'un contrôle, et le canari sort déjà `1` sur fichier
manquant (l. 72, 76), sur relais en échec (l. 291, 350) et sur `VIOLATION` du
self-check de jeton (le trap force `rc=1`). Ni `3` ni `4` n'est utilisé
aujourd'hui (vérifié). Une cause, un code — même doctrine que les jetons
nommés de `_pilot_daemon_freshness` (D5 de mika#2029).

**Le trap tourne sur un refus, et c'est correct.** `_canary_on_exit` est installé
avant la garde ; sur un refus il imprime `ok: host_gh_token_untouched` (le jeton
hôte n'a effectivement pas bougé) et retire le répertoire de staging, puis
`exit "$rc"` **préserve 3 et 4** — seule la branche `VIOLATION` réécrit `rc`.
Rien à changer dans le trap.

### C. Le mode `--check-flight`

Un sixième mode, public et documenté dans l'en-tête `INVOKE:` : exécute la garde
seule et sort sur son code, sans rien installer et sans toucher au relais. Deux
raisons, dans cet ordre :

1. **C'est la gestuelle que la règle opératrice réclamait déjà.** « Canari
   seulement sur moteur vide » était un contrôle à la main ; `--check-flight`
   est ce contrôle, rendu à la machine.
2. **C'est ce qui rend le cas passant testable hors hôte de boucle.** Un drapeau
   de test caché (`CANARY_GUARD_ONLY=1`) aurait fait la même chose en étant
   invisible à l'opérateur ; un mode réel est la même dépense, utile deux fois.

Placement : la garde est appelée **avant** le branchement `--check-flight`, de
sorte qu'aucun mode ne peut naître non gardé — ajouter un mode demain ne demande
aucun geste.

### D. Le déplacement

`install -Dm755 "$SCRIPT" "$INSTALLED_PROXY"` et ses deux lignes de commentaire
descendent de la l. 81 à juste après la garde. Le commentaire gagne une phrase :
ce que cette écriture déclenche en aval (relance par péremption du mtime) et
pourquoi elle est désormais précédée d'une garde.

## Contrat de vérification

Une suite nouvelle, `scripts/test-canary-pilot-containment.sh` — le canari n'en
a aucune sur `main`. Câblée **directement** par un step de ci.yml dans le job
`offline-script-suites`, qui est le pattern maison (un step par suite, nommé,
`ci.yml:609-690`). Rien n'entre dans `scripts/suite-wiring.tsv` : l'atteignabilité
est directe, et le registre ne tient que le résidu (mika#2761).

La suite est **hermétique** : pas de réseau, pas de token, pas de `bwrap`, pas de
`cargo`. `HOME` et `PILOT_LOG_DIR` sont redirigés vers des fixtures ; le canari
refuse avant d'atteindre quoi que ce soit d'autre.

| # | ce qui est établi | comment | AC |
|---|---|---|---|
| T1 | un `.log` récent sans `.stderr` apparié ⇒ **refus**, et le proxy installé **garde son mtime** | mode **défaut** (pas `--check-flight`), sous `HOME` de fixture portant un faux `~/.local/bin/mika-pilot-egress-proxy` d'mtime connu. Assertion : `rc=3`, mtime identique, le `.log` nommé dans la sortie. C'est la mesure du 01/11 rejouée à l'envers | AC1-0, AC3-a |
| T2 | le même `.log` **avec** son `.stderr` ⇒ la garde **laisse passer** | `--check-flight`, assertion `rc=0` + ligne `ok:   no_sibling_pilot_in_flight` | AC2, AC3-b |
| T3 | un instant de coupure **indécidable** ⇒ **refus** fail-closed | un shim `date` en tête de `PATH` qui échoue sur `-u -d` et délègue sinon ; assertion `rc=4` + la cause nommée sur stderr | AC1-2, AC3-c |
| T4 | **anti-vacuité du fixture** : les trois états posés sont bien distincts | mesure de la forme produite (`.log` présent / `.stderr` présent) pour les trois fixtures, assertée avant T1-T3 — précédent direct : `test-dispatch-lib.sh:15199-15240`, où un `touch -d` non reconnu rendait deux états indiscernables et deux assertions vertes sur le mauvais | — |
| T5 | **anti-vacuité du shim de T3** : le shim est bien atteint, et il échoue bien sur `-d` | le shim trace ses appels dans un fichier ; assertion « tracé non vide » et « `date -u -d` rend non-zéro sous ce PATH » | — |
| T6 | **scan d'ordre** : la garde précède l'installation, et l'installation est un site unique | numéros de ligne lus dans le fichier : `garde < install`, et `grep -c 'install -Dm755' == 1`. C'est ce scan qui porte le cas passant que T2 ne peut pas observer (voir la limite ci-dessous) | AC1, AC2 |

**Limite déclarée, sans contournement.** T2 établit que la garde *laisse passer*,
pas que *le canari entier tourne* : le mode défaut, passé la garde, exécute
`bwrap`, le relais et des appels réseau — rien de cela n'est jouable dans un job
CI hermétique, et le ticket ne le demande pas. La partie de AC2 qui dit « les
mêmes contrôles `ok:` » est donc établie **structurellement** (T6 : aucun contrôle
n'est retiré ni réordonné, l'installation reste le site unique qu'elle était) et
non par exécution. Le falsificateur par exécution reste le canari lui-même sur
l'hôte, dont la sortie est lue par l'opérateur.

Le fixture de vol (≈ 8 lignes) est **réécrit** dans la nouvelle suite plutôt que
partagé avec `_pilot_flight_fixture` de `test-dispatch-lib.sh` : les deux suites
tournent dans deux jobs distincts et aucune ne source l'autre ; les faire
dépendre l'une de l'autre pour huit lignes créerait un couplage entre jobs dont
la rupture rendrait rouge la mauvaise suite.

Gates existantes : `make test-dispatch-lib` (la modification de `dispatch-lib.sh`
y passe, S1 et S2 de mika#3031 inclus), `shellcheck` via la lint shell du dépôt,
`scripts/check-suite-wiring.sh`, et `bash scripts/verify-pipeline.sh origin/main`.

## Fire-Disposition

Le vol livre des détecteurs : les six assertions de
`scripts/test-canary-pilot-containment.sh`, dont le scan d'ordre T6.

**Option (a) — exception nommée en allowlist, table déclarée et VIDE**, avec
assertion auto-nettoyante. Le détecteur part **armé** (doctrine mika#2201) :

```sh
T3051_CANARY_GUARD_ALLOWLIST=()
assert_eq "T6: table d'exceptions du scan « la garde précède l'installation » — zero entries" \
    "0" "${#T3051_CANARY_GUARD_ALLOWLIST[@]}"
```

**Zéro violation existante, et c'est mesuré, pas supposé :** `install -Dm755`
n'apparaît qu'une fois dans le fichier, et ce vol la déplace. La table est
déclarée quand même — pour que le prochain qui veut y mettre une entrée voie
d'abord qu'elle existe exprès — et assertée vide pour qu'une entrée ajoutée en
silence rougisse la suite. C'est cette assertion qui empêche la table de devenir
l'allowlist que mika#2309 condamne. Précédent littéral dans le dépôt :
`test-dispatch-lib.sh:15616-15628`.

## Sonde (AC4)

Sur les **14 jours** qui suivent le deploy, deux comptes, écrits en commentaire
sur mika#3051 :

1. **lancements du canari, refusés et exécutés.** Les refus sont lisibles : un
   refus imprime sa ligne nommée et sort 3 ou 4. La sonde compte, dans les
   journaux d'opération, les occurrences de `canary: refus` (codes 3 et 4,
   séparément) et les en-têtes `canary-pilot-containment — real spawn` (exécutés).
2. **`502 … inference gateway` reçus par un pilote dans les 5 min qui suivent un
   canari exécuté**, par appariement d'instants entre l'en-tête du canari et les
   `.log` de `/var/log/claude-pilot`. **Cible : 0.**

La sonde est un geste d'opérateur, pas du code : elle ne lit que des artefacts
que ce vol rend déjà lisibles, et la gratter dans un script reviendrait à livrer
un instrument de plus à maintenir pour deux comptes à faire une fois.

## Ordre de commit

Un seul commit de travail, le vol est indivisible — la garde sans la publication
du chemin ne peut pas nommer le `.log`, et la publication seule ne garde rien.

1. `fix(3051): le canari interroge « un pilote vole-t-il ? » avant d'installer le proxy`
   — `dispatch-lib.sh` (publication du chemin), `canary-pilot-containment`
   (garde + `--check-flight` + déplacement de l'installation),
   `scripts/test-canary-pilot-containment.sh` (nouveau), `ci.yml` (step).

**Plancher :** la suite nouvelle passe, `make test-dispatch-lib` passe,
`bash scripts/verify-pipeline.sh origin/main` passe.

## Definition of Done

- [ ] `_pilot_sibling_pilot_in_flight` publie `_PILOT_SIBLING_IN_FLIGHT_LOG`
      (vidée en entrée, assignée avant le `return 0`), sans rien écrire sur
      stdout, avec le commentaire qui dit pourquoi.
- [ ] `_canary_refuse_if_pilot_in_flight` posée dans le canari, appelée
      inconditionnellement après le `source` et avant l'installation.
- [ ] L'installation du proxy a quitté la l. 81 pour un site postérieur à la
      garde ; elle reste un site unique.
- [ ] Codes de sortie `3` (un pilote vole) et `4` (indécidable), distincts de
      `1`, préservés par le trap `EXIT`.
- [ ] Mode `--check-flight` documenté dans l'en-tête `INVOKE:`.
- [ ] `scripts/test-canary-pilot-containment.sh` : T1-T6 verts, hermétique.
- [ ] Step nommé dans le job `offline-script-suites` de `.github/workflows/ci.yml`.
- [ ] `make test-dispatch-lib` vert (S1/S2 de mika#3031 inclus).
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`, et la sonde
      AC4 nommée avec sa date d'échéance.
- [ ] **Aucun ticket de suivi ouvert** (gel jusqu'au 01/12) : un reste éventuel
      va en commentaire sur mika#3051 ou dans la PR.

## Acceptance criteria

Transcrits du corps de mika#3051 :

- **AC1.** Avant l'installation de la l. 81, et donc avant tout mode qui touche
  au relais ou au proxy, le canari interroge `_pilot_sibling_pilot_in_flight`.
  Rien n'est installé, et aucun relais n'est touché, dans deux cas :
  - **0 (un pilote vole)** : le canari refuse, avec un code de sortie distinct
    de l'échec d'un contrôle, et nomme le `.log` du pilote vivant ;
  - **2 (on ne peut pas décider)** : le canari refuse aussi, fail-closed, avec
    la cause nommée.
- **AC2.** Avec 1 (aucun pilote), le canari tourne comme aujourd'hui : rc=0, et
  les mêmes contrôles `ok:`.
- **AC3.** Un test du script, dans le même appel :
  - un `.log` récent sans `.stderr` apparié : refus, et le proxy installé garde
    son mtime ;
  - le même `.log` avec son `.stderr` : le canari tourne ;
  - un instant de coupure indécidable : refus.
- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les
  lancements du canari (refusés et exécutés), et les `502 … inference gateway`
  reçus par un pilote dans les 5 min qui suivent un canari exécuté. La cible est
  0 pour les seconds. On l'écrit ici.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/canary-pilot-containment` (garde + `--check-flight` + déplacement, commentaires maison inclus) | 60 |
| `skills/bundled/_shared/dispatch-lib.sh` (publication du chemin + commentaire) | 14 |
| `scripts/test-canary-pilot-containment.sh` (nouveau : T1-T6, fixtures, shim, anti-vacuité) | 185 |
| `.github/workflows/ci.yml` (un step nommé) | 8 |

Total estimé : 267 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 267 lignes sous la borne de 429 et sous le seuil
par PR de 1000. Les deux fichiers existants ne bougent que de 74 lignes au total,
et le déplacement de l'installation est de trois lignes dont rien ne dépend entre
son ancien et son nouveau site (mesuré au § Site). Le gros du volume est la suite
nouvelle, qui est du code additif sans interaction avec l'existant. Le vol est
aussi indivisible : couper la publication du chemin de la garde livrerait une
garde qui ne peut pas satisfaire AC1.

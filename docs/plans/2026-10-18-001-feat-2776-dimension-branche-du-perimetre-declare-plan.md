# feat(infra) — la dimension branche du périmètre déclaré : un pilote ne pousse que sur SA branche (vol 4 de mika#2736, mika#2776)

| | |
|---|---|
| ticket | senara-solutions/mika#2776 |
| vols amont | mika#2736 (borne statique, **mergé** #2753) · mika#2751 (dimension dépôt, **mergé**) · mika#2766 (cycle de vie des entrées, **mergé** #2777) |
| ce vol | la **dimension branche** du périmètre (`ref=`), le motif `push-scope-ref`, et la réparation symétrique du canari |
| vol suivant | aucun — ce vol ferme la sous-classe **P3**. P4 (force-push) reste hors d'atteinte côté proxy, § 10 |
| labels | `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate` |

---

## 1. Pourquoi, et ce que ce plan répond

Le ticket demande une seule chose, et elle est nette : `ref=refs/heads/<branche du
dispatch>` écrite par le producteur, lue par le lecteur, et un **motif de refus
distinct** de `push-scope-repo`. Les trois vols amont sont sur `main` (mesuré au
§ 2.1), donc l'argument de cardinalité que mika#2766 avait posé comme préalable
est payé : la désinscription existe et le TTL de 12 h collecte ce qu'un `SIGKILL`
laisse derrière. Le resserrement livré ici a donc une force qui **ne se desserre
pas au fil de la journée**.

Ce plan répond en plus à trois questions que le ticket ne pose pas et dont deux
décident la conception :

1. **L'union porte-t-elle sur les dimensions ou sur les entrées ?** Mesuré au
   § 2.4 : une union **plate par dimension** autorise le croisement « dépôt de
   l'entrée A × branche de l'entrée B » et laisse donc ouvert exactement le
   scénario du ticket dès qu'un second dispatch est en vol. L'union est par
   **entrée**, et ça ne coûte rien de plus.
2. **Qu'est-ce que la dimension branche casse dans les suites existantes ?**
   Mesuré au § 2.5 : quatre sites, nommés. Un vol qui les découvre en route les
   « répare » en desserrant.
3. **Le canari reste-t-il vert ?** Oui — et c'est le problème. Mesuré au § 2.6 :
   il redevient vert **par accident**, exactement le défaut que le bloc mika#2751
   du canari décrit pour la dimension dépôt. La réparation symétrique est due
   dans ce vol, pas après.

---

## 2. Mesures — exécutées au grooming, pas déduites

### 2.1 Le plancher est tenu : les trois vols amont sont sur `main`

Le vol 3 avait dû mesurer que son propre plancher n'était **pas** tenu (le vol 2
vivait dans un brouillon de sauvetage). Ce vol-ci vérifie la même chose et la
trouve satisfaite :

```
$ git log --oneline -3
506f18cc feat(infra): une entrée du registre de périmètre meurt avec son spawn, et son TTL est le sien (vol 3 de mika#2736) (#2777)
ca4be133 fix(infra): le corps de requête chunked montant n'est plus borné par une recherche de sous-chaîne (mika#2758) (#2778)
8ac93d62 fix(ci): câbler test-measure-webhook-turn-cost dans ci.yml (mika#2752) (#2769)
```

Les quatre sites que ce vol prolonge existent, lus sur `HEAD` :

| site | ligne | ce qu'il fait |
|---|---|---|
| `dispatch-lib.sh::_stage_pilot_egress_push_scope` | 1929 | tire un nonce, écrit `repo=senara-solutions/$REPO` |
| `dispatch-lib.sh::_unstage_pilot_egress_push_scope` | 2008 | retire l'entrée à la fin du spawn (vol 3) |
| `addon.py::_push_scope_repos` | 843 | l'union des dépôts déclarés, ou `None` si illisible |
| `addon.py::_push_scope_verdict` | 904 | `ALLOW` / `scope-absent` / `scope-unreadable` / `push-scope-repo` |

Ce vol est **entièrement additif** sur cette couture : il n'en retire rien et il
n'en déplace aucun site d'appel.

### 2.2 `BRANCH` est en portée au site du staging — revérifié, pas repris sur parole

Le corps du ticket donne cette mesure comme déjà faite. Elle est refaite ici
parce qu'un plan qui hérite une mesure hérite aussi de sa date :

```
$ grep -n '^\s*BRANCH=\|^\s*local BRANCH\|BRANCH=' skills/bundled/_shared/dispatch-lib.sh | grep -v '\$BRANCH'
4513:        BRANCH=$("$PLATFORM_DIR/scripts/derive-branch-name" \
```

Une seule assignation, **sans `local`** (donc globale au script), dans
`_set_up_worktree`. `_set_up_worktree` tourne avant `_run_claude_pilot`, qui
appelle `_run_pilot_sandboxed`, où `_stage_pilot_egress_push_scope` est appelée
(ligne 2453). `BRANCH` est donc en portée au site du staging, et sa valeur est
celle de **ce** dispatch.

Conséquence mesurée et non supposée : `BRANCH` est **vide** en dispatch free-text
et en dispatch de milestone, exactement comme `REPO`. Le § 3.2 en tire la
dégradation.

### 2.3 Ce que le producteur écrit aujourd'hui, et la forme que le format autorise

```
$ printf 'repo=senara-solutions/%s\n' "$REPO" > "$_dir/$_nonce"
```

Une ligne `clé=valeur`. Le lecteur ignore toute clé qu'il ne connaît pas — et ce
n'est pas une lecture de commentaire, c'est asserté par le CASE 16.7 de
`test_egress_push_bound.sh`, qui écrit aujourd'hui une entrée portant
`ref=refs/heads/feat/2751/whatever` **comme clé inconnue** et vérifie qu'elle ne
change rien. `ref=` est donc une addition pure côté format : aucune migration,
aucune entrée à réécrire, et le § 2.5 nomme le prix exact de cette propriété —
ce cas-là doit changer de clé.

### 2.4 L'union est par ENTRÉE et non par dimension, et c'est mesuré sur un cas

La sémantique d'union est imposée au vol 2 par une propriété du canal : un seul
socket unix, aucun justificatif sur `github.com`, donc une requête n'est pas
attribuable à un spawn. Le verdict porte sur « **une** entrée vive déclare-t-elle
ceci ? ». Sur une seule dimension cette formulation est sans ambiguïté. Sur deux,
elle se lit de deux façons, et elles ne valent pas la même chose :

Deux dispatches concurrents, A sur `mika` / `feat/2776/branche`, B sur
`mika-platform` / `fix/9999/autre` :

| lecture | ce qu'elle autorise | verdict sur un push `mika` → `refs/heads/fix/9999/autre` |
|---|---|---|
| **par dimension** (union plate : `{mika, mika-platform} × {feat/2776/branche, fix/9999/autre}`) | 4 couples, dont 2 que personne n'a déclarés | **ALLOW** |
| **par entrée** (une entrée est un couple) | 2 couples, exactement ceux déclarés | **DENY `push-scope-ref`** |

La lecture par dimension laisse donc ouvert **le scénario nommé par le ticket**
(« la branche d'un autre dispatch du même dépôt, éventuellement en vol ») dès
qu'un second dispatch existe — c'est-à-dire dans l'état normal d'un hôte de
boucle. Elle ne resserrerait que l'hôte à un seul dispatch, qui est le cas où le
resserrement importe le moins.

Le coût de la lecture par entrée est de **trois lignes** dans le verdict : le
lecteur fait déjà un seul balayage du répertoire, il suffit qu'il rende des
couples au lieu de deux ensembles plats. Il n'y a donc pas d'arbitrage ici, il y
a une lecture juste et une lecture fausse.

### 2.5 Ce que la dimension branche casse dans les suites existantes — quatre sites nommés

Mesuré en lisant les deux suites, pas en lançant l'implémentation. Un vol qui
les découvre en route a le réflexe de desserrer la règle pour les faire passer.

| site | ce qui se passe une fois `ref=` armé | la correction |
|---|---|---|
| `test_egress_push_bound.sh` helper `declare_repos` (l. 279) | écrit `repo=` seul ⇒ les CASE 14-23 tombent tous sur `scope-ref-absent` et leurs assertions `degrades_in(err) == []` rougissent | le helper écrit **aussi** `ref=` à la branche canonique du fichier ; un second helper `declare_repos_without_ref` sert le cas qui asserte la forme rétrocompatible |
| `test_egress_push_bound.sh` CASE 16.7 (l. 786) | utilise `ref=` **comme clé inconnue** : la clé devient connue, la ref déclarée ne correspond pas au push ⇒ 403 `push-scope-ref` là où le cas asserte 200 | la clé inconnue devient `future-key=` seule, et `ref=` y prend la vraie branche. La propriété testée (« une clé inconnue est ignorée ») reste testée |
| `test_egress_push_bound.sh` CASE 18.6 (l. 862) | pousse `refs/heads/feat/2751/big` alors que le périmètre déclarera la branche canonique ⇒ 403 sur le cas **positif** du push chunked | la ref du gros push devient la branche canonique. Ce cas est l'un des deux positifs que le point 4 du ticket protège : le modifier pour qu'il reste positif, jamais pour qu'il contourne |
| `test_egress_push_scope_staging.sh` CASE 1/2/3/6 | assertions d'**égalité exacte** sur le contenu de l'entrée (`"repo=senara-solutions/mika"`) ; elles resteraient vertes parce que le test ne définit pas `BRANCH`, donc l'entrée n'aurait pas de `ref=` — vertes **pour la mauvaise raison** | `BRANCH` est défini en préparation du fichier, et les quatre assertions deviennent le contenu à deux lignes. Le cas « `BRANCH` vide » le désassigne explicitement |

Le quatrième est le plus coûteux à manquer : il ne rougit pas. Un vol qui livre
`ref=` sans y toucher laisse une suite de producteur qui mesure uniquement le
monde d'avant et le déclare conforme.

### 2.6 Le canari reste vert — et c'est précisément le défaut

```
$ grep -n '_pref=\|REPO=\|BRANCH' scripts/canary-pilot-containment
538:# set neither REPO nor BRANCH, so `_stage_pilot_egress_push_scope` declares
568:REPO="$_CANARY_PUSH_SCOPE_REPO"
836:    _pref=refs/heads/feat/9999/canary-2736
```

Le canari assigne `REPO` (depuis le vol 2) mais **pas** `BRANCH`, et la ref que
ses quatre sondes poussent est codée en dur ligne 836. Donc, une fois `ref=`
armé : son entrée ne porte pas de `ref=`, le lecteur dégrade sur la dimension
branche (`scope-ref-absent`), et `push_bound_nominal` **reste vert**.

C'est mot pour mot le défaut que le bloc mika#2751 du canari décrit pour la
dimension dépôt :

> « That greenness was ACCIDENTAL, and it stopped being safe the moment the
> tightening landed. »

Trois conséquences, et aucune n'est optionnelle :

- la sonde nominale ne mesure plus rien de la dimension branche ;
- il n'existe **aucune** sonde en vol du resserrement de branche — les deux
  suites sont hors ligne par construction (`mitmproxy` n'est pas une dépendance
  de test de ce dépôt, mesuré au vol 1), donc l'absence de sonde ici est une
  absence totale, pas une redondance en moins ;
- le jour où la dimension branche cesse silencieusement de mordre, **rien** ne
  le dit. C'est la classe mika#2578 sur une surface qui l'a déjà payée trois
  fois.

La réparation est symétrique de celle du vol 2 : le canari assigne `BRANCH` à la
ref que `_pref` pousse, ce qui rend la sonde nominale verte **par décision**, et
une sonde `push_scope_ref` devient possible — une autre branche du dépôt que ce
run a déclaré, les quatre règles statiques satisfaites et la dimension dépôt
satisfaite, donc le resserrement de branche est la **seule** chose qui peut la
refuser.

### 2.7 La sonde de 4 octets est hors d'atteinte par construction — ce qui ne dispense pas de l'asserter

Le point 4 du ticket exige que les deux positifs de mika#2736 restent positifs.
Pour la dimension dépôt, le vol 2 pouvait le démontrer structurellement : le
dépôt vient de l'URL, donc le resserrement ne distingue pas une sonde d'un push.
Pour la dimension branche, **ce n'est plus vrai** : les refs viennent du corps,
et une sonde d'authentification n'en porte aucune.

C'est donc ici que le mode de défaillance capable d'arrêter la boucle revient à
portée : un verdict qui exigerait « au moins une ref dans le périmètre »
refuserait la liste vide, donc la sonde, donc **tout push assez gros pour être
streamé**. La conception du § 3.3 traite la liste vide avant toute autre chose,
et le § 3.7 l'épingle par un cas positif sous ce mécanisme — jamais hérité sur
parole, exactement comme le CASE 18 sous le périmètre et le CASE 23 sous le
cycle de vie.

---

## 3. La décision, et sa conception

### 3.1 Le producteur : une seconde ligne, et un garde de forme d'une ligne

`_stage_pilot_egress_push_scope` écrit une seconde ligne quand, et seulement
quand, `BRANCH` est non vide **et** de la forme d'un nom de branche :

```
repo=senara-solutions/<REPO>
ref=refs/heads/<BRANCH>
```

Le garde de forme refuse d'écrire la ligne si `BRANCH` sort de
`[A-Za-z0-9._/-]` ou commence par `refs/`. Deux raisons, et la première est la
seule qui compte :

- **une valeur portant un saut de ligne injecte une ligne dans le registre.**
  Le format est `clé=valeur` par ligne ; un `BRANCH` contenant `\n` permet
  d'écrire un `repo=` arbitraire, c'est-à-dire d'**élargir** le périmètre par le
  canal censé le resserrer. C'est de la classe accident (`derive-branch-name`
  est l'unique producteur et rend un slug), et elle se ferme pour une ligne de
  `case`. Un garde qu'on peut écrire en une ligne et qui ferme un desserrement
  n'a pas d'arbitrage à subir ;
- un `BRANCH` déjà préfixé `refs/` donnerait `ref=refs/heads/refs/heads/…`,
  c'est-à-dire un périmètre ne nommant aucune ref existante, donc un refus de
  **tout** push. Même conséquence que la valeur vide du § 3.2, et même remède :
  ne pas écrire la ligne.

Le garde **dégrade** (pas de ligne `ref=`, dispatch intact, une ligne nommée sur
`stderr`), jamais ne bloque — même posture que tout le reste de cette couture.

### 3.2 `BRANCH` vide : aucune ligne, et le lanceur le dit

Le modèle est la dégradation du vol 2 pour `REPO` vide, et le ticket le nomme :
**un `ref=` à valeur vide serait pire que son absence.** Le lecteur verrait une
branche déclarée, ne la retrouverait dans aucun push, et refuserait **tout**
push de ce dispatch — un 403 sur le travail légitime, le seul mode de
défaillance capable d'arrêter la boucle.

La ligne n'est donc pas écrite du tout, l'entrée reste **valide** (elle porte son
`repo=`, donc la dimension dépôt mord toujours), et le lanceur nomme sa
dégradation sur `stderr` du dispatch, avec le ticket et la conséquence :

```
dispatch-lib: no dispatch branch in scope — push perimeter declared for the
repository only, the branch dimension does not apply to this spawn (mika#2776)
```

Noter la différence avec le message de `REPO` vide : celui-là dit « aucun
périmètre déclaré », celui-ci dit « périmètre déclaré, une dimension sur deux ».
Deux états, deux messages — un message partagé rendrait les deux causes
indiscernables au moment où elles décident le remède (classe mika#2205).

### 3.3 Le lecteur : des couples, et un verdict qui traite la liste vide d'abord

`_push_scope_repos()` devient `_push_scope_entries()` et rend
`list[tuple[str, str | None]] | None` — un couple `(dépôt normalisé, ref ou None)`
par entrée lisible. Le balayage, la purge, le throttle et les trois postures de
défaillance sont **inchangés** :

- `None` ⇒ et seulement ⇒ registre illisible (`scope-unreadable`) ;
- liste vide ⇒ rien n'est déclaré (`scope-absent`) ;
- une entrée individuellement illisible est **sautée**, jamais promue en panne
  de registre ;
- une entrée **sans** `repo=` ne produit aucun couple — c'est ce qui garde le
  CASE 17.1 vert, où une entrée ne portant qu'un `ref=` doit continuer de lire
  `scope-absent` ;
- une ligne `ref=` à valeur **vide** est lue comme **absente** (`None`), pas
  comme une ref nommée « » : la posture fail-open du canal, appliquée à une
  entrée qu'un producteur futur écrirait mal.

Le verdict devient :

```python
def _push_scope_verdict(repo, refs):
    if not _push_scope_enforced():          return _ALLOW, ""
    entries = _push_scope_entries()
    if entries is None:                     return _DEGRADE_SCOPE_UNREADABLE, ""
    if not entries:                         return _DEGRADE_SCOPE_ABSENT, ""
    declared = [ref for (name, ref) in entries if name == repo.strip().lower()]
    if not declared:                        return _DENY_SCOPE_REPO, repo
    if not refs:                            return _ALLOW, ""   # § 2.7
    if any(ref is None for ref in declared): return _DEGRADE_SCOPE_REF_ABSENT, ""
    allowed = set(declared)
    for ref in refs:
        if ref.strip() not in allowed:      return _DENY_SCOPE_REF, ref
    return _ALLOW, ""
```

Quatre propriétés de cette forme, chacune pour une raison :

- **`if not refs` passe avant tout jugement de branche** (§ 2.7). C'est la
  sonde d'authentification de 4 octets, et c'est la seule ligne de ce verdict
  dont l'inversion arrête la boucle autonome ;
- **`declared` filtre par dépôt avant de regarder les refs** : c'est l'union par
  entrée du § 2.4, et c'est ce qui interdit le croisement ;
- **toutes les commandes sont examinées**, et la première qui viole nomme le
  motif et sa cible — la même discipline que `_push_verdict`, pour la même
  raison : une borne qui ne jugerait que la première commande serait contournée
  par une commande anodine placée en tête ;
- **la comparaison des refs est exacte** : ni minusculisation, ni suffixe
  retiré. Les refs git sont **sensibles à la casse**, à la différence de
  `owner/repo` chez GitHub ; minusculiser serait un desserrement (deux refs
  distinctes deviendraient une) déguisé en symétrie avec le voisin.

Au site d'appel dans `request()`, les refs sortent des commandes déjà décodées :
`refs = [ref for _old, _new, ref in commands]`. À ce point `commands` n'est
jamais `None` — `reason == _ALLOW` l'implique — donc aucune garde de plus.

La **subordination reste structurelle** : cette fonction n'est atteinte que sur
un push que la borne statique a déjà autorisé, son unique nouvelle issue est un
refus de plus, et aucun de ses `return` ne peut ouvrir quoi que ce soit.

### 3.4 `scope-ref-absent` : un ALLOW qui dit pourquoi il n'a pas resserré

Une entrée sans `ref=` est l'**état normal** pendant une fenêtre de déploiement :
le registre de l'hôte survit au déploiement, donc une entrée écrite par le
binaire précédent est là, et elle est valide. C'est le point 2 du ticket, et le
lecteur l'honore sans rien de spécial puisque `ref is None` ⇒ pas de
resserrement de branche.

Ce qui doit être écrit, c'est l'**observabilité** de cet état. Sans elle, « le
resserrement de branche ne mord jamais » et « il mord » rendent les mêmes
octets : zéro ligne. Donc un troisième jeton de dégradation, ajouté à
`_PUSH_SCOPE_DEGRADED`, ce qui suffit au site d'appel existant pour l'émettre
sur la même forme de ligne :

```
[github-proxy] DEGRADE scope-ref-absent POST /senara-solutions/mika.git/git-receive-pack
```

Trois jetons, trois causes **opposées**, trois remèdes (classe mika#2205) :
`scope-unreadable` envoie l'opérateur sur la comptabilité de l'hôte,
`scope-absent` sur le lanceur qui n'a rien déclaré, `scope-ref-absent` sur la
**fenêtre de déploiement** — et son remède nominal est « attendre que le spawn
suivant écrive une entrée complète », c'est-à-dire aucun remède, ce qu'il faut
savoir avant de chercher.

Le jeton n'est émis que s'il y a au moins une ref à juger (§ 3.3), ce qui évite
qu'une sonde de 4 octets porte une ligne de dégradation sur chaque push streamé.

### 3.5 `push-scope-ref` : un septième motif, jamais le sixième élargi

Le ticket le demande nommément, et la classe mika#2205 dit pourquoi : deux causes
ne doivent pas arriver dans les mêmes octets. `push-scope-repo` et
`push-scope-ref` nomment deux remèdes disjoints — l'un envoie lire l'`origin` du
worktree, l'autre envoie lire la **branche** que le dispatch a dérivée. Un corps
partagé rendrait la différence indiagnosticable à l'instant exact où elle décide
quoi faire.

Le septième corps suit la forme des six autres : il nomme le motif, la **ref
visée** (donnée du client, pas un secret de l'hôte — mika#1949 interdit de
nommer le justificatif présenté, jamais la cible), le remède, et le rollback.
`_PUSH_DENY_TICKETS` reçoit `push-scope-ref → "mika#2776"`, pour que l'opérateur
refusé atterrisse sur le vol qui a décidé le refus.

Le pied de page apprend une phrase : `MIKA_PILOT_EGRESS_PUSH_SCOPE=0` désarme le
resserrement déclaré **entier**, ses deux dimensions ensemble. C'est le § 3.6.

### 3.6 Aucun troisième interrupteur, et c'est asserté

Point 3 du ticket. `MIKA_PILOT_EGRESS_PUSH_SCOPE` couvre déjà le resserrement
entier ; un troisième nom sur cette surface serait un recours plus gros que le
problème — un opérateur qui veut « rendre mika#2736 » cherche un interrupteur,
pas une matrice.

Les deux interrupteurs existants restent prouvés orthogonaux (classe mika#2165),
et le lanceur reste à **zéro lecteur** de celui-ci : l'assertion structurelle du
CASE 6 du staging (« `dispatch-lib` nomme l'interrupteur en prose seulement,
jamais en code ») compte à zéro les lignes non-commentaires qui le nomment, et
elle doit rester à zéro après ce vol. La ligne `ref=` est écrite
**inconditionnellement** pour la même raison que `repo=` : un nom de branche
n'est pas un secret, et un interrupteur dont les deux lecteurs divergent le jour
où ça compte est décoratif.

### 3.7 Les suites — dans les fichiers des vols amont, pas à côté

Point 4 du ticket, règle mika#2578 : les cas voyagent dans le commit qui crée la
règle. Les deux suites sont déjà câblées dans `ci.yml` (bloc *Pilot sandbox
secret-channel guards*, `make test-egress-push-bound` et
`make test-egress-push-scope-staging`), donc **aucun câblage n'est à ajouter** —
mesuré, parce que l'absence de câblage est la classe qui a déjà mordu trois fois
sur cette surface.

**Lecteur — `test_egress_push_bound.sh`.** Trois cas neufs et quatre extensions.

| cas | ce qu'il asserte |
|---|---|
| **CASE 24** | la branche déclarée passe (anti-vacuité de 25 et 26) ; **une autre branche du même dépôt est refusée `push-scope-ref`** — le scénario du ticket ; le corps nomme la ref visée, le vol `mika#2776`, le rollback ; plusieurs commandes dont une seule hors périmètre ⇒ refusée et le corps nomme **celle-là** ; la casse d'une ref est signifiante (`refs/heads/Feat…` ≠ `refs/heads/feat…`) ; la borne statique mord toujours en dessous (`refs/heads/main` reste `push-ref-protected`) |
| **CASE 25** | **une entrée sans `ref=` reste valide** (point 2) : le push passe, et **une** ligne `DEGRADE scope-ref-absent` le dit ; les trois jetons sont distincts deux à deux ; la dimension **dépôt** mord toujours sur cette même entrée |
| **CASE 26** | **le non-croisement** (§ 2.4) : deux entrées vives `(mika, feat/A)` et `(mika-platform, fix/B)` ; chacune passe sur la sienne, et `mika → fix/B` est **refusée**. C'est le cas qui rougit si quelqu'un remet une union plate |
| CASE 11 (ext.) | les corps de refus sont distincts deux à deux et aucun ne nomme un autre motif : **sept** au lieu de cinq (le sixième est déjà asserté en 15.x) |
| CASE 16.7 (ext.) | la clé inconnue devient `future-key=` ; la propriété reste testée, sur une clé qui est encore inconnue |
| CASE 18 (ext.) | les deux positifs de mika#2736 **réassertés sous ce mécanisme** (§ 2.7, point 4) : la sonde de 4 octets et le push chunked de 400 KiB passent, périmètre complet armé ; et la même sonde vers une branche **non déclarée** du bon dépôt est refusée — sans quoi les positifs pourraient être le resserrement renonçant aux sondes |
| CASE 19 (ext.) | la matrice d'interrupteurs apprend la dimension branche : `PUSH_SCOPE=0` rend mika#2736 **exactement** (les deux dimensions tombent ensemble, § 3.6), `PUSH_BOUND=0` reste le disjoncteur maître, et l'orthogonalité inter-gardes tient (`PUSH_SCOPE=0` laisse la coupure `api.github.com` de mika#2724 intacte) |

**Producteur — `test_egress_push_scope_staging.sh`.** Deux cas neufs, plus la
correction du § 2.5 (quatrième ligne).

| cas | ce qu'il asserte |
|---|---|
| **CASE 13** | la branche du dispatch est réellement déclarée, **dans la forme que le lecteur lit** : deux lignes exactes, `ref=refs/heads/<BRANCH>`, pas de suffixe `.git`, pas de minusculisation de la ref |
| **CASE 14** | `BRANCH` vide ⇒ la ligne `ref=` n'est **pas écrite du tout**, l'entrée reste valide avec son `repo=`, le staging rend 0, et `stderr` nomme la dégradation, la dimension perdue et le ticket. Puis la forme hostile : un `BRANCH` portant un saut de ligne n'ajoute **aucune** ligne au fichier (le garde du § 3.1), mesuré sur le **nombre de lignes** de l'entrée et non sur une sous-chaîne |

**Canari — `scripts/canary-pilot-containment`.** `BRANCH` est assigné à la ref
que les sondes poussent, `push-scope-ref` entre dans la table des motifs
reconnus (une sonde qui attend un motif doit pouvoir **rapporter** n'importe quel
autre, sinon les deux causes arrivent dans les mêmes octets), et une sonde
`push_scope_ref` vise une autre branche du dépôt déclaré. La ref de cette sonde
porte `2776` pour la même raison que le dépôt du vol 2 portait `2751` : aucun
dispatch réel ne peut la déclarer.

### 3.8 Les canaux écartés, avec la raison qui les écarte

| canal | pourquoi non |
|---|---|
| passer la branche par l'environnement de `mitmdump` | `_ensure_pilot_helper` rend 0 dès que le port est vif, donc le démon est partagé et son environnement est celui du **premier** dispatch, pour toujours. Déjà mesuré au vol 2 ; la branche est un datum qui change **plus** souvent que le dépôt, donc l'erreur serait plus fréquente et toujours dans le sens qui élargit |
| une clé `branch=<nom>` plutôt que `ref=refs/heads/<nom>` | le lecteur compare à ce que le protocole porte, qui est une **ref** complète. Une clé qui impose au lecteur de reconstruire le préfixe déplace la normalisation du producteur (un site) vers le lecteur (le site qui refuse), et `refs/heads/` est exactement ce que la règle `push-ref-exotic` exige déjà par ailleurs |
| un second registre pour les branches | les deux dimensions ont la **même** posture de défaillance, le **même** interrupteur et la **même** durée de vie. Le vol 2 a séparé les registres parce que la sentinelle et le périmètre n'en partageaient aucune ; ici les trois sont partagées, donc deux registres seraient deux sites à purger pour un seul contrat |
| élargir `push-scope-repo` au cas de la branche | classe mika#2205, et le ticket le refuse nommément : deux remèdes disjoints ne partagent pas un corps |
| un troisième interrupteur | point 3 du ticket, § 3.6 |

---

## 4. Fire-Disposition

Ce vol livre des détecteurs : les cas CASE 24-26 et les quatre extensions du
lecteur, les CASE 13-14 du producteur, la sonde `push_scope_ref` du canari et le
garde de forme du § 3.1 sont tous du code dont le chemin de succès est « aucune
violation trouvée ».

**Disposition retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE, et c'est mesuré, pas espéré.**

Détail d'implémentation :

- **Les détecteurs atterrissent armés, zéro exception.** Il n'existe aucun
  corpus à faire entrer par la bande : le registre de périmètre est **éphémère**
  — une entrée par spawn, écrite au lancement, retirée à la fin du spawn depuis
  le vol 3, et collectée par un TTL de 12 h sinon. Il n'y a donc pas de donnée
  existante qui violerait la nouvelle règle au moment où elle atterrit, à la
  différence d'un lint sur l'arbre des sources dont le premier run trouve
  toujours quelque chose. L'implémentation **vérifie** avant de conclure :
  `ls ~/.mika/pilot-push-scopes/ | wc -l` sur l'hôte avant déploiement, consigné
  dans le corps de PR. Une entrée trouvée sans `ref=` n'est pas une violation à
  excepter : c'est le cas que le CASE 25 asserte comme **valide**.
- **La forme rétrocompatible est dans la RÈGLE, jamais dans l'allowlist.** Une
  entrée écrite par le binaire précédent (sans `ref=`) est l'état **normal**
  pendant la fenêtre de déploiement — le point 2 du ticket. Une entrée
  d'allowlist pour cette forme serait stale par construction : elle réapparaît à
  chaque déploiement. Le lecteur la traite comme valide (§ 3.4), et une ligne
  `DEGRADE scope-ref-absent` la rend lisible le temps que la fenêtre se ferme.
- **L'assertion auto-nettoyante** est le **CASE 26**. Il ne relit aucune
  constante — un test qui relit une constante n'atteste rien — mais la
  **conséquence** de la sémantique par entrée : le jour où quelqu'un remet une
  union plate par dimension « pour simplifier », CASE 26 rougit avec le scénario
  exact du § 2.4 et non avec un diff de structure de données.
- **Le garde du § 3.1 atterrit armé** et sa violation est impossible à
  excepter : il ne refuse pas le dispatch, il refuse d'**écrire une ligne**. Son
  chemin d'échec est une dégradation nommée, donc il n'existe pas d'état où une
  exception serait le remède.

---

## 5. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/mika-pilot-github-auth-addon.py` — `_PUSH_SCOPE_REF_KEY`, `_DENY_SCOPE_REF` + son corps, `_DEGRADE_SCOPE_REF_ABSENT`, `_push_scope_entries()`, `_push_scope_verdict(repo, refs)`, site d'appel, bloc de section | 120 |
| `skills/bundled/_shared/dispatch-lib.sh` — la ligne `ref=`, le garde de forme, la dégradation nommée sur `stderr`, le bloc de commentaire | 35 |
| `scripts/canary-pilot-containment` — `BRANCH` assigné, `push-scope-ref` dans la table des motifs, sonde `push_scope_ref` | 35 |
| tests (`skills/bundled/_shared/tests/test_egress_push_bound.sh`) — CASE 24-26, extensions 11/16.7/18/19, les deux helpers | 125 |
| tests (`skills/bundled/_shared/tests/test_egress_push_scope_staging.sh`) — CASE 13-14, `BRANCH` en préparation, les quatre égalités exactes du § 2.5 | 75 |

Total estimé : 390 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et pourquoi aucun découpage n'existe ici.** 390
passe sous 429 avec 39 lignes de marge. La marge est mince et le plan le dit
plutôt que de la décorer : le vol 3 a estimé 340 et pesé 558 réelles (facteur
1,64), ce qui est précisément le ratio que le diviseur 3 de la dérivation
absorbe, et il a tenu dans les 200 tours.

Surtout, **cette surface n'est pas divisible en vols** : la ligne `ref=` sans son
lecteur est une ligne que personne ne lit, et le lecteur sans la ligne rend
`scope-ref-absent` sur chaque push — deux moitiés dont aucune ne resserre rien,
donc deux PR dont aucune n'est le « vol mergeable seul » que la règle demande.
Le découpage qui existait, le ticket l'a déjà consommé : c'est celui du vol 3.

**La soupape, décidée ici et non en vol.** Si la mesure réelle dépasse, deux
coupes sont pré-arbitrées, dans cet ordre, et aucune ne retire une propriété qui
ne soit tenue ailleurs sur le même chemin de code :

1. les sous-assertions d'empreinte (`md5`/`sha1`/`sha256`) du nouveau motif dans
   CASE 24 — la non-divulgation est déjà tenue par CASE 12 et CASE 15.x, qui
   traversent le **même** `_push_deny_body` (≈ 18 lignes) ;
2. le sous-cas de la casse signifiante des refs dans CASE 24, qui redevient un
   commentaire au site de comparaison du § 3.3 (≈ 10 lignes).

### 5.1 Ce qui n'est pas compté, et pourquoi

Les deux surfaces documentaires du point 5 sont hors barème (`docs/`) :

- `docs/operator/environment-variables.md` — l'entrée
  `MIKA_PILOT_EGRESS_PUSH_SCOPE` apprend la dimension branche, le septième
  motif, le troisième jeton de dégradation, la dégradation `BRANCH` vide et la
  cinquième ligne du tableau du canari ; le tableau des motifs de
  `MIKA_PILOT_EGRESS_PUSH_BOUND` et la phrase « la dimension branche reste
  ouverte » (l. 8710) sont rectifiés ;
- `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
  — un § pour la dimension branche, et la rectification du § *Le lecteur partagé
  ne peut pas attribuer une requête à un spawn* qui porte aujourd'hui le report.
  Le fait le moins évident à y consigner est le § 2.4 : sur deux dimensions,
  « l'union des entrées vives » a deux lectures et une seule est juste. Le § *P4*
  existe déjà et reste vrai mot pour mot — ce vol ne change rien à la nature
  invisible d'une mise à jour forcée, il ne change que la ref visée.

---

## 6. Ordre de commit, et son plancher

**Plancher — mesuré avant la première ligne (§ 2.1).** La couture des trois vols
amont doit être sur `main` :

```
grep -q '_stage_pilot_egress_push_scope' skills/bundled/_shared/dispatch-lib.sh \
  && grep -q '_unstage_pilot_egress_push_scope' skills/bundled/_shared/dispatch-lib.sh \
  && grep -q '_push_scope_verdict' scripts/mika-pilot-github-auth-addon.py \
  && grep -q '_PUSH_SCOPE_TTL_SECONDS' scripts/mika-pilot-github-auth-addon.py
```

Mesuré vert au grooming. Si l'un des quatre manque à l'implémentation, elle
s'arrête, ne commit rien et **rapporte** — elle ne pose aucune question.

1. **`feat(infra): le lecteur du périmètre rend des couples, et la branche est une dimension`**
   — `addon.py` seul : `_push_scope_entries()`, `_push_scope_verdict(repo, refs)`,
   le septième motif et son corps, le troisième jeton, le site d'appel, **plus
   les CASE 24-26 et les quatre extensions**. Mergeable et vérifiable seul : à ce
   point aucun producteur n'écrit `ref=`, donc tout le parc d'entrées est de la
   forme rétrocompatible et le lecteur dégrade — c'est exactement l'état que le
   CASE 25 asserte, et il est sans danger.
2. **`feat(infra): le lanceur déclare la branche du dispatch, et nomme sa dégradation`**
   — `dispatch-lib.sh` : la ligne `ref=`, le garde de forme, le message de
   dégradation, **plus les CASE 13-14 et la correction du § 2.5 quatrième
   ligne**. C'est le commit qui **arme** le resserrement.
3. **`feat(infra): le canari sonde la dimension branche, et sa nominale est verte par décision`**
   — `canary-pilot-containment` seul. Séparé **délibérément** : la sonde ne peut
   rougir utilement qu'une fois les commits 1 et 2 déployés (elle traverse un
   `mitmdump` vif), donc elle est la seule partie de ce vol dont la vérification
   est post-déploiement, et la mêler aux deux premiers rendrait leur `make test`
   trompeur.
4. **`docs: les deux surfaces du périmètre déclaré apprennent sa dimension branche`**
   — les deux fichiers du § 5.1.

Les commits 1 et 2 tiennent chacun la règle mika#2578 : leurs cas voyagent avec
eux. Le commit 3 est l'exception, et il dit pourquoi il en est une.

---

## 7. Definition of Done

0. **Plancher mesuré** (§ 6). Échec ⇒ arrêt, aucun commit, rapport
   `PIPELINE_INCOMPLETE — plancher non tenu`, aucune question.
1. `_stage_pilot_egress_push_scope` écrit `ref=refs/heads/$BRANCH` quand `BRANCH`
   est non vide et de forme valide, et **rien** sinon.
2. Le garde de forme du § 3.1 refuse d'écrire la ligne pour un `BRANCH` hors
   `[A-Za-z0-9._/-]` ou préfixé `refs/`, et dégrade au lieu de bloquer.
3. La dégradation `BRANCH` vide est nommée sur `stderr` avec son ticket, et son
   message est **distinct** de celui de `REPO` vide.
4. `_push_scope_entries()` remplace `_push_scope_repos()` et rend des couples ;
   les trois postures de défaillance du vol 2 sont inchangées, et une entrée sans
   `repo=` ne produit toujours aucun couple.
5. `_push_scope_verdict` traite la liste de refs **vide** avant tout jugement de
   branche, filtre par dépôt avant de regarder les refs, examine toutes les
   commandes, et compare les refs **exactement**.
6. `push-scope-ref` existe, son corps est distinct des six autres, nomme la ref
   visée, le vol `mika#2776` et le rollback, et aucun autre motif ne le nomme.
7. `scope-ref-absent` existe, est dans `_PUSH_SCOPE_DEGRADED`, n'est émis que
   s'il y a au moins une ref à juger, et sort sur la forme de ligne existante.
8. Aucun nouvel interrupteur. `grep -F 'MIKA_PILOT_EGRESS_PUSH_SCOPE' dispatch-lib.sh | grep -vc '^[[:space:]]*#'`
   rend toujours `0`.
9. Les deux suites passent, y compris les quatre sites corrigés du § 2.5 :
   `make test-egress-push-bound` et `make test-egress-push-scope-staging`.
10. Le canari assigne `BRANCH`, reconnaît `push-scope-ref` dans sa table de
    motifs, et porte une sonde `push_scope_ref` dont la cible est une ref
    portant `2776`.
11. Les deux surfaces documentaires du § 5.1 sont à jour, et la phrase « la
    dimension branche reste ouverte » de `environment-variables.md` (l. 8710) est
    rectifiée.
12. `make test`, `cargo clippy`, `cargo fmt --check`, les lints du dépôt et
    `scripts/verify-pipeline.sh origin/main` passent.
13. Le corps de PR consigne la mesure du § 4 (`ls ~/.mika/pilot-push-scopes/ | wc -l`).

---

## 8. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés de ses cinq points de *Travail attendu* et du contrat de
vérification ci-dessus.

1. **AC1 — la dimension branche mord.** Un push vers
   `refs/heads/fix/9999/un-autre-ticket` depuis un dispatch dont le périmètre
   déclare `refs/heads/feat/2776/...`, dans le **même** dépôt, est refusé par un
   403 portant `push-scope-ref`. Vérifié par CASE 24.
2. **AC2 — le motif est distinct.** `push-scope-ref` et `push-scope-repo` ont des
   corps différents, aucun ne nomme l'autre, et chacun nomme son propre remède
   et son propre vol. Vérifié par CASE 24 et l'extension de CASE 11.
3. **AC3 — une entrée sans `ref=` reste valide.** Un push légitime passe face à
   une entrée ne portant que `repo=`, et une ligne `DEGRADE scope-ref-absent`
   le rend lisible. Vérifié par CASE 25.
4. **AC4 — `BRANCH` vide ne produit aucune ligne `ref=`.** L'entrée reste valide,
   le dispatch continue, et `stderr` nomme la dégradation. Vérifié par CASE 14 du
   producteur.
5. **AC5 — aucun interrupteur de plus.** `MIKA_PILOT_EGRESS_PUSH_SCOPE=0` rend
   mika#2736 exactement, ses deux dimensions tombant ensemble ; le lanceur reste
   à zéro lecteur ; l'orthogonalité inter-gardes tient. Vérifié par l'extension
   de CASE 19 et par CASE 6 du producteur.
6. **AC6 — les deux positifs de mika#2736 restent positifs.** La sonde
   d'authentification de 4 octets et le push chunked de 400 KiB passent, le
   périmètre complet armé. Vérifié par l'extension de CASE 18.
7. **AC7 — aucun croisement entre deux périmètres vifs.** Deux entrées vives
   n'autorisent que les deux couples déclarés, jamais les quatre. Vérifié par
   CASE 26.
8. **AC8 — les cas vivent dans le commit qui crée la règle**, dans les deux
   fichiers de suite existants, et les deux suites restent câblées dans
   `ci.yml`. Vérifié par le § 6 et par un `make` vert.
9. **AC9 — la surface est sondée en vol.** Le canari porte une sonde
   `push_scope_ref` et sa sonde nominale est verte par décision, pas par
   l'absence de `ref=` dans sa propre entrée.
10. **AC10 — les deux surfaces documentaires sont à jour**, et aucune ne décrit
    encore la dimension branche comme ouverte.

---

## 9. Risques et arbitrages

| # | risque | ce qui le tient |
|---|---|---|
| R1 | **403 sur tout push streamé** — le verdict exige « au moins une ref dans le périmètre » et refuse la liste vide, donc la sonde de 4 octets (§ 2.7). *La boucle s'arrête.* | `if not refs: return _ALLOW` placé avant tout jugement de branche (§ 3.3), et l'extension de CASE 18 qui l'épingle **positivement** sous ce mécanisme |
| R2 | **403 sur le travail légitime en dispatch free-text** — un `ref=` à valeur vide déclarerait une branche introuvable | la ligne n'est pas écrite du tout (§ 3.2), et CASE 14 l'asserte sur le **nombre de lignes** de l'entrée |
| R3 | **desserrement par la fenêtre de déploiement** — le registre survit au déploiement, donc des entrées sans `ref=` circulent | c'est l'état voulu (point 2), borné par le TTL de 12 h du vol 3 et par la désinscription en fin de spawn ; rendu lisible par `scope-ref-absent` (§ 3.4) |
| R4 | **desserrement par injection de ligne** — un `BRANCH` portant `\n` écrit un `repo=` arbitraire | le garde de forme du § 3.1, asserté par CASE 14 |
| R5 | **la suite de producteur verte pour la mauvaise raison** — elle ne définit pas `BRANCH`, donc elle mesurerait le monde d'avant (§ 2.5, 4ᵉ ligne) | `BRANCH` défini en préparation du fichier, et les quatre égalités exactes deviennent le contenu à deux lignes |
| R6 | **le resserrement cesse de mordre en silence** | la sonde `push_scope_ref` du canari (§ 3.7) et la ligne `DEGRADE` greppable ; sans la première, aucune sonde en vol n'existe (§ 2.6) |
| R7 | **le croisement de dimensions passe inaperçu** — une union plate lit « resserré » et ne resserre rien dès le second dispatch | CASE 26, et le § 2.4 écrit dans la note de solution pour que la prochaine lecture ne reprenne pas la pente |

**L'arbitrage que ce plan assume.** La comparaison des refs est **exacte**, donc
un dispatch dont la branche change en cours de vol (une re-dérivation, un
renommage) verrait son push refusé. C'est accepté : `BRANCH` est dérivé une
seule fois par `_set_up_worktree` (§ 2.2) et le spawn qui pousse est celui qui a
déclaré, donc le cas demande un changement de branche **à l'intérieur** du
spawn, ce qu'aucun chemin de dispatch ne fait. Le remède, s'il survenait, est le
rollback nommé — pas un assouplissement de la comparaison.

---

## 10. Hors périmètre, délibérément

- **P4, le force-push.** Hors d'atteinte côté proxy pour une raison de
  **protocole** et non par report : `receive-pack` ne porte aucun drapeau de
  force, le client envoie `<old> <new> <ref>` et c'est le **serveur** qui résout
  l'ancestralité. Resserrer la branche ne change rien — la ref visée devient la
  bonne, la nature de la mise à jour reste invisible. Déjà mesuré et écrit au
  § *P4* de la note de solution, qui reste vrai mot pour mot après ce vol.
- **Les quatre règles statiques de mika#2736**, la **dimension dépôt** de
  mika#2751 et le **cycle de vie** de mika#2766 — livrés, et ce vol n'en déplace
  aucun site.
- **La moitié `api.github.com`** — livrée par mika#2724.
- **La classe adversaire**, pour la même raison qu'aux trois vols précédents :
  tout processus de la sandbox lit ce que la sandbox contient. Ces gardes ferment
  la classe **accident**. Cette moitié-là est fermée côté code par mika#2713.
- **Un troisième interrupteur** — point 3 du ticket, § 3.6.
- **Un second registre pour les branches** — § 3.8.

---

## 11. Référence

- Ticket : senara-solutions/mika#2776
- Vol 1 : senara-solutions/mika#2736 — la borne statique (plan
  `docs/plans/2026-10-17-001-fix-2736-egress-pilote-moitie-git-push-plan.md`)
- Vol 2 : senara-solutions/mika#2751 — la dimension dépôt (plan
  `docs/plans/2026-10-17-003-feat-2751-egress-push-scope-declare-plan.md`)
- Vol 3 : senara-solutions/mika#2766 — le cycle de vie des entrées (plan
  `docs/plans/2026-10-17-004-feat-2766-cycle-de-vie-des-entrees-du-registre-de-perimetre-plan.md`,
  dont le § 2.6 porte la mesure de `BRANCH` revérifiée ici)
- L'asymétrie hôte/sandbox dont le périmètre tire son autorité :
  senara-solutions/mika#2724
- Incident fondateur de P4 : senara-solutions/mika#1318
- Note de solution :
  `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
- Classes invoquées : mika#2205 (deux causes, deux jeux d'octets), mika#2165 (un
  interrupteur, un lecteur), mika#2578 (les cas dans le commit qui crée la
  règle), mika#1949 (nommer la cible, jamais le justificatif), mika#1574 /
  mika#2306 (Fire-Disposition), mika#2693 (la borne par vol)

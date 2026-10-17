# feat(infra) — le cycle de vie des entrées du registre de périmètre : une entrée meurt avec son spawn, et le TTL dit laquelle a survécu (vol 3 de mika#2736, mika#2766)

| | |
|---|---|
| ticket | senara-solutions/mika#2766 |
| vols amont | mika#2736 (borne statique, **mergé** #2753) · mika#2751 (dimension dépôt, **NON mergé** — brouillon #2767) |
| ce vol | la **désinscription** des entrées + le **TTL propre** au registre de périmètre |
| vol suivant | la **dimension branche** (`ref=`) — `Tracked in:` au § 10 |
| labels | `enhancement`, `p2-normal`, `infrastructure`, `dispatch:loop`, `loop-substrate` |

---

## 1. Pourquoi, et ce que ce plan répond

Le ticket demande deux moitiés : la **dimension branche** du périmètre déclaré
(`ref=refs/heads/<branche>`) et le **cycle de vie** des entrées du registre. Il
donne lui-même la raison de leur couplage, et cette raison est **orientée** :

> « Le resserrement de la branche vaut donc **exactement** ce que vaut la
> désinscription des entrées, pas moins. […] livrer `ref=` sans le cycle de vie
> donnerait un garde qui se desserre tout seul au fil de la journée. »

La contrainte énoncée est donc « **pas de `ref=` sans cycle de vie** ». Elle
n'interdit pas l'inverse : le cycle de vie seul ne desserre rien, ne dépend de
rien, et il est le **prérequis nommé** de la dimension branche. Ce plan livre
cette moitié-là, seule, et renvoie la dimension branche au vol suivant. Le § 5
montre que les deux ensemble dépassent la borne par vol de ce ticket ; entre les
deux découpages possibles, un seul est permis par le ticket, et c'est celui-ci.

Ce vol répond en plus à une question que le ticket ne pose pas et qui **décide
la valeur du TTL** : la dégradation du vol 2 n'est pas totale. Mesuré au § 2.3 —
avec deux dispatches concurrents, purger l'entrée d'un pilote **vivant** ne
dégrade pas vers la borne statique, elle **refuse** le push de ce pilote. Le TTL
n'est donc pas un paramètre d'hygiène : c'est un plancher sur la durée du spawn
le plus long, et se tromper dans le sens serré arrête la boucle.

---

## 2. Mesures — exécutées au grooming, pas déduites

### 2.1 Le plancher de ce vol n'est pas tenu : le vol 2 n'est pas sur `main`

Le corps du ticket porte « mika#2751 | la **dimension dépôt** […] | livré ».
C'est faux à l'instant du grooming, et c'est mesuré, pas supposé :

```
$ grep -c "_stage_pilot_egress_push_scope\|_push_scope_verdict" \
      skills/bundled/_shared/dispatch-lib.sh scripts/mika-pilot-github-auth-addon.py
skills/bundled/_shared/dispatch-lib.sh:0
scripts/mika-pilot-github-auth-addon.py:0

$ gh pr view 2767 --repo senara-solutions/mika --json number,state,isDraft,title
#2767 OPEN draft=true — wip(mika#2751): trailing content after pilot end_turn (mika#1383)
```

Le travail du vol 2 existe (11 fichiers, ~1 400 lignes hors `docs/plans/`) mais
il vit dans un **brouillon de sauvetage de worktree sale** — le préfixe `wip(`
est celui de mika#1282, posé par `dispatch-lib` quand un pilote sort sans
commit. Rien de la couture que ce vol prolonge n'est sur `main` :
`_stage_pilot_egress_push_scope` (le producteur), `_PILOT_EGRESS_PUSH_SCOPE_ENTRY`
(le nonce que la désinscription doit connaître), `_push_scope_repos` et
`_push_scope_verdict` (le lecteur), `_PILOT_EGRESS_PUSH_SCOPE_DIR` (le registre).

**Conséquence sur la conduite de l'implémentation**, § 7 step 0 : ce vol est
*entièrement* additif sur cette couture. Il n'existe aucune version de lui qui
puisse atterrir avant le vol 2. L'implémentation **commence par mesurer** la
présence de la couture et, si elle est absente, ne produit rien et **rapporte**
— elle ne demande pas, elle n'empile pas une PR sur une branche de brouillon.

### 2.2 Ce que le vol 2 a posé, et où ce vol s'y raccroche

Trois sites, lus sur `origin/feat/2751/infra-borner-le-git-receive-pack-de` :

| site | ce qu'il fait aujourd'hui | ce que ce vol y ajoute |
|---|---|---|
| `dispatch-lib.sh::_stage_pilot_egress_push_scope` (~1878) | tire un nonce, écrit `repo=senara-solutions/$REPO` sous `$_PILOT_EGRESS_PUSH_SCOPE_DIR`, remplit `_PILOT_EGRESS_PUSH_SCOPE_ENTRY` | rien (le producteur est inchangé) |
| `dispatch-lib.sh::_run_pilot_sandboxed` (appel ~2295, sortie ~2630-2638) | appelle le staging juste après `_stage_pilot_egress_sentinel` ; se termine en fermant les fd de secret puis `return "$_sandbox_rc"` | **la désinscription**, entre la fermeture des fd et le `return` |
| `addon.py::_purge_registry(directory)` (~307) | purge les entrées plus vieilles que `_REGISTRY_TTL_SECONDS` (72 h), throttle **par répertoire** depuis le commit 2 du vol 2 | **le TTL devient un paramètre** ; deux appelants, deux valeurs |

### 2.3 La dégradation du vol 2 n'est pas totale — ce qui fixe le plancher du TTL

Le vol 2 documente une posture de **dégradation** : périmètre absent ou
illisible ⇒ la borne statique de mika#2736 s'applique seule, donc ALLOW. C'est
vrai **quand le registre entier est vide ou illisible**, et c'est la lecture que
le commentaire de `_push_scope_verdict` autorise. Ce n'est pas vrai par entrée :

```python
declared = _push_scope_repos()
if declared is None:        return _DEGRADE_SCOPE_UNREADABLE   # registre illisible
if not declared:            return _DEGRADE_SCOPE_ABSENT       # registre vide
if repo … in declared:      return _ALLOW
return _DENY_SCOPE_REPO, repo                                  # <-- ici
```

Deux dispatches concurrents, A sur `mika` et B sur `mika-platform`. Si l'entrée
de A est purgée pendant que A **vit encore**, le registre n'est ni vide ni
illisible : il contient celle de B. `declared = {senara-solutions/mika-platform}`,
et le push légitime de A tombe sur `_DENY_SCOPE_REPO`. Un TTL trop court ne
desserre donc pas : il **refuse le pilote vivant dont l'entrée a été purgée**, et
`mitmdump` étant partagé, le refus arrive à celui qui n'a rien fait.

Le sens de l'erreur est asymétrique et contre-intuitif :

| erreur sur le TTL | conséquence | coût |
|---|---|---|
| trop **long** | une entrée survit à son pilote tué ; l'union reste bornée par les dépôts dispatchables (`DISPATCHABLE_REPO_BASENAMES`) | du desserrement, rien de neuf dans l'union |
| trop **court** | le push d'un pilote vivant est 403 pendant qu'un voisin tient le registre non-vide | **la boucle s'arrête**, et sur un pilote innocent |

Le TTL est donc **un plancher sur la durée du spawn le plus long**, jamais une
borne d'hygiène qu'on serre pour faire propre. C'est ce que le § 3.2 en tire.

### 2.4 Pourquoi la désinscription est **obligatoire** et pas un confort

Un dispatch appelle `_run_pilot_sandboxed` **plus d'une fois** : le pilote
d'implémentation, puis le pilote de révision (`_launch_revise_pilot`), puis les
reprises. Chaque spawn restage, et le staging écrase
`_PILOT_EGRESS_PUSH_SCOPE_ENTRY` :

```sh
_stage_pilot_egress_push_scope() {
    _PILOT_EGRESS_PUSH_SCOPE_ENTRY=""      # <-- le nonce du spawn précédent est perdu ici
```

Sans désinscription **à la fin de chaque spawn**, le nonce du spawn N est
inconnaissable dès que le spawn N+1 commence. La fin du spawn est donc le
**seul** site qui sache encore quelle entrée retirer : ce n'est pas le site le
plus commode, c'est le seul qui soit correct. Un `trap EXIT` au niveau du
dispatch ne pourrait jamais nettoyer que la dernière.

### 2.5 Le trap EXIT existe, et il a un précédent exact

`_dispatch_lib_exit_trap` (~2760) fait déjà très exactement ce dont ce vol a
besoin comme filet : un relâchement de ressource hôte en secours du chemin
nominal.

```sh
_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # mika#2155: crash/cancel backstop for the claim — the nominal path already
    # released in _deliver_callback (and lowered the flag, so this is a no-op there).
    _release_issue_seat "$REPO" "$ISSUE_NUM" || true
```

Même forme ici : nominal à la fin du spawn, filet dans le trap, no-op quand le
nominal a déjà joué (le nonce est vidé). `_dispatch_lib_term_trap` (~2713) sort
en 143, donc le trap EXIT couvre aussi le SIGTERM. Il ne couvre **pas** le
SIGKILL — et c'est précisément le trou que le TTL ferme, comme le ticket le dit.

### 2.6 `BRANCH` est en portée au site du staging — mesuré pour le vol suivant

Le ticket demande de mesurer, pas de supposer. Fait, et consigné ici pour que le
vol 4 n'ait pas à le refaire :

- `BRANCH` a **un seul site d'affectation** dans `dispatch-lib.sh` : la ligne
  4339, dans `_set_up_worktree`, sans `local` — donc un global du shell, comme
  `REPO`.
- L'ordre du préambule (~2203-2208) est `_setup_gh_auth` → `_scrub_env` →
  `_set_up_worktree` → `_detect_plan_on_branch` → `_handle_dry_run` →
  `_run_claude_pilot`. Le staging vit **dans** `_run_pilot_sandboxed`, donc
  après `_set_up_worktree` : `BRANCH` est en portée au site d'appel.
- Elle n'est affectée que dans la **branche mode-ticket**. Le mode texte libre
  et le mode milestone laissent `BRANCH` vide, exactement comme `REPO`. Le vol 4
  y trouvera donc gratuitement le cas de l'**entrée sans `ref=`** que le point 5
  du ticket demande d'asserter : `REPO` posé et `BRANCH` vide ⇒ `repo=` seule.

### 2.7 Le TTL est aujourd'hui une constante de module, partagée par deux registres

```python
_REGISTRY_TTL_SECONDS = 72 * 3600
_PURGE_MIN_INTERVAL_SECONDS = 600
_last_purge: dict[str, float] = {}      # par répertoire depuis le vol 2

def _purge_registry(directory: Path) -> None:
    …
    cutoff = time.time() - _REGISTRY_TTL_SECONDS
```

Deux appelants : `_registry_nonces()` (sentinelles, ~351) et `_push_scope_repos()`
(périmètres, ajouté par le vol 2). Le **throttle** est déjà par répertoire ; le
**TTL** ne l'est pas. C'est la seule chose à ouvrir côté addon.

---

## 3. La décision, et sa conception

### 3.1 `_unstage_pilot_egress_push_scope` — la désinscription, et ses trois propriétés

Nouvelle fonction dans `dispatch-lib.sh`, voisine immédiate du producteur, et
trois propriétés qui sont la raison de sa forme :

1. **Elle ne retire que son entrée.** Elle supprime le fichier dont elle connaît
   le nonce, jamais le répertoire, jamais un balayage. Un voisin vif doit
   survivre — c'est tout l'objet des sémantiques d'union du vol 2, et un
   `rm -rf` du registre serait le desserrement total déguisé en ménage.
2. **Elle vide le nonce après coup.** `_PILOT_EGRESS_PUSH_SCOPE_ENTRY=""` en
   fin de fonction, ce qui rend le second appel (le filet du trap) un no-op
   observable, et non pas un no-op par chance.
3. **Elle est fail-open et muette sur l'échec attendu.** Un `rm` qui échoue ne
   doit pas changer le statut de sortie du pilote : la fonction retourne
   toujours 0, et le site d'appel est placé **après** la capture de
   `_sandbox_rc` (§ 3.3). Un échec dit pourquoi sur stderr, parce qu'une entrée
   non retirée est un desserrement silencieux — exactement ce que le TTL existe
   pour rattraper, et exactement ce qu'un opérateur doit pouvoir grep.

Elle ne porte **pas** de bracket `set +x` : un nom de nonce de périmètre n'est
pas un justificatif, et le producteur du vol 2 a déjà écrit pourquoi il n'en
porte pas. Ajouter l'un sans l'autre inviterait à lire une raison qui n'existe
pas.

### 3.2 Le TTL du registre de périmètre — sa valeur, et sa dérivation

`_purge_registry(directory)` devient `_purge_registry(directory, ttl_seconds)`,
et deux constantes nommées vivent côté lecteur :

```python
_REGISTRY_TTL_SECONDS = 72 * 3600          # sentinelles — inchangé
_PUSH_SCOPE_TTL_SECONDS = 12 * 3600        # périmètres  — ce vol
```

Pourquoi un paramètre et non une seconde constante lue dans la fonction : la
fonction purge **déjà** deux registres (c'est le défaut que le commit 2 du vol 2
a corrigé sur le throttle) ; le TTL est la seconde moitié du même défaut. Un
`if directory == …` dans la purge serait un lecteur de plus pour une donnée que
l'appelant connaît.

**Pourquoi 12 h, et pourquoi pas 72 h ni 1 h.** La valeur est contrainte des
deux côtés, et le § 2.3 dit que les deux côtés ne coûtent pas la même chose :

- **Plancher — strictement au-dessus du spawn légitime le plus long.** Un TTL
  sous cette durée refuse le push d'un pilote vivant dès qu'un voisin tient le
  registre non-vide (§ 2.3). Il n'existe **aucun timeout mural** sur un spawn
  dans `dispatch-lib.sh` — mesuré : les seuls `timeout` du fichier sont le
  `timeout 15 claude-pilot --help` du smoke test. La borne réelle d'un spawn est
  donc son plafond de tours, 200 pour un ticket `loop-substrate`, qui se mesure
  en heures. 12 h laisse une marge d'un facteur sur ce qui a été observé, et
  l'implémentation **nomme la mesure** dans le commentaire de la constante
  (§ 7 step 2) plutôt que de reconduire ce chiffre sur ma parole.
- **Plafond — bien en dessous de 72 h.** 72 h est calibré sur « un vol peut être
  relu trois jours après » (mika#2724), une question qui n'est pas celle-ci. Ce
  registre répond à « ce périmètre est-il encore en vol ? », et une entrée de
  trois jours ne l'est jamais. 12 h borne la fenêtre à moins d'une journée de
  boucle : une entrée tuée par SIGKILL le matin ne survit pas à la nuit.
- **Pas de variable d'environnement**, point 3 du ticket, et sa raison tient
  doublement ici : un TTL qu'un opérateur peut mettre à zéro n'est pas un
  registre qu'il vide par accident, c'est — par le § 2.3 — un **403 sur tous les
  pilotes vivants** dont un voisin tient le registre non-vide. Le pire des deux
  interrupteurs imaginables.

**Et aucun interrupteur de plus** (point 3). `MIKA_PILOT_EGRESS_PUSH_SCOPE`
couvre déjà le resserrement entier : à `0`, `_push_scope_verdict` rend ALLOW
avant toute lecture du registre, donc le TTL et la désinscription deviennent du
ménage sans conséquence de verdict. L'orthogonalité des deux interrupteurs
existants (classe mika#2165) est inchangée par ce vol : il n'ajoute aucun
lecteur à aucun des deux.

### 3.3 Où la désinscription est appelée, et pourquoi exactement là

Deux sites, et la fin du spawn est le nominal (§ 2.4 : c'est le seul qui
connaisse encore le nonce).

**Site 1 — `_run_pilot_sandboxed`, entre la fermeture des fd de secret et le
`return`.** Le voisinage porte déjà la contrainte à ne pas casser, et elle est
écrite :

```sh
    # mika#2039: close the secret descriptors, then return the status bwrap
    # actually produced. The order matters — closing first would overwrite
    # `$?` …
    local _cfd
    for _cfd in ${secret_fds[@]+"${secret_fds[@]}"}; do
        exec {_cfd}<&-
    done
    return "$_sandbox_rc"
```

`_sandbox_rc` est **déjà** capturé à ce point (lignes 2566 et 2627, `|| _sandbox_rc=$?`),
donc la désinscription ne peut pas écraser le statut du pilote : elle s'insère
après la boucle de fermeture, avant le `return`, et le commentaire mika#2039
reste vrai mot pour mot. C'est la raison de ce placement plutôt que d'un plus
haut.

**Site 2 — `_dispatch_lib_exit_trap`, en filet.** Une ligne, sur le modèle exact
de `_release_issue_seat` (§ 2.5), placée avec les autres relâchements et avant
la garde `CALLBACK_SENT` — pour la même raison que son aîné : le chemin nominal
sort tôt de ce trap, et un nominal qui a échoué a encore besoin du filet. No-op
quand le nonce est vide.

Ce que les deux sites **ne** couvrent pas : le SIGKILL. C'est le TTL, et c'est
la phrase du ticket. Rien d'autre n'est à inventer pour ce trou.

### 3.4 Les suites — dans les fichiers du vol 2, pas à côté (point 4, mika#2578)

Les cas atterrissent dans le **commit qui crée la règle**, et dans les deux
suites que le vol 2 a déjà posées — leur en-tête dit déjà où va quoi, et ce vol
respecte ce partage plutôt que de l'élargir :

**`test_egress_push_scope_staging.sh`** (le producteur et son cycle de vie) :

| cas | ce qu'il tient |
|---|---|
| L1 | la désinscription retire l'entrée du spawn, et le nonce est vidé après |
| L2 | un **voisin vif survit** : deux entrées, une désinscrite, l'autre intacte (le cœur des sémantiques d'union) |
| L3 | **deux spawns successifs ne fuient pas** : `_run_pilot_sandboxed` deux fois ⇒ le registre est vide à la fin, pas une entrée orpheline par spawn (le défaut du § 2.4, qu'une désinscription hissée au dispatch laisserait passer) |
| L4 | un `rm` qui échoue (répertoire en `0500`) ⇒ statut du pilote **inchangé**, un mot sur stderr ; *skip par nom sous root*, comme le cas 5 existant |
| L5 | le second appel est un no-op : désinscrire deux fois ne touche à rien d'autre |

**`test_egress_push_bound.sh`** (le lecteur) :

| cas | ce qu'il tient |
|---|---|
| T1 | une entrée de périmètre plus vieille que **12 h** est purgée ; le verdict retombe sur `scope-absent` quand elle était seule |
| T2 | une entrée de périmètre de **13 h** est purgée alors qu'une sentinelle du même âge **ne l'est pas** : les deux TTL sont bien par registre, la régression « un seul TTL » rougit ici |
| T3 | l'anti-régression du § 2.3, et c'est le cas le plus important du vol : entrée A purgée + entrée B vive ⇒ le push de A est **refusé**, pas dégradé. Le test **asserte la conséquence** du TTL trop court, ce qui est ce qui rend la valeur de la constante vérifiable plutôt que décrétée |
| T4 | **les deux cas positifs de mika#2736 restent positifs** : la sonde d'authentification de 4 octets et le push chunked passent toujours, inchangés par ce vol (point 4 du ticket) |

**`test_egress_sentinel_enforcement.sh`** : un cas, que le TTL de 72 h des
sentinelles est inchangé — la suite du registre voisin est le bon endroit pour
asserter qu'on ne l'a pas bougé en passant.

Aucune nouvelle inscription CI n'est requise : les trois suites sont déjà
appelées par le `Makefile` (lignes 161-162, 255-261) et le vol 2 y a ajouté
`test_egress_push_scope_staging.sh`. L'implémentation **vérifie** cette
inscription plutôt que de la supposer (§ 7 step 5).

### 3.5 Les canaux écartés, avec la raison qui les écarte

| canal | pourquoi non |
|---|---|
| **un `trap EXIT` dans `_run_pilot_sandboxed`** au lieu d'un appel explicite | un trap posé dans une fonction écrase le trap du dispatch (`_dispatch_lib_exit_trap`) pour toute la durée du shell, pas pour la durée de la fonction. Bash n'a pas de trap à portée de fonction. Le remède serait de sauver et restaurer le trap du dispatch autour du spawn : trois lignes de plus et un mode de panne qui perd le callback |
| **purger par `flock` + balayage du registre à chaque fin de spawn** | un balayage ne distingue pas une entrée morte d'une entrée vive, c'est le travail du TTL côté lecteur, et le lecteur est le seul à savoir quand il lit |
| **un PID dans l'entrée, et un lecteur qui teste la vivacité** | le lecteur est l'addon, qui tourne dans `mitmdump` sur l'hôte — il *pourrait* lire un PID. Mais un PID se réutilise, et le fail-safe d'un PID réutilisé par un processus quelconque est un périmètre déclaré vivant par un étranger. Un mtime ne se réutilise pas |
| **une seule constante de TTL, serrée à 12 h pour les deux registres** | les sentinelles répondent à « ce vol est-il relisible ? » et 72 h y est calibré (mika#2724). Les resserrer ensemble ferait porter à une donnée deux contrats, le défaut que le § 3.2 du plan du vol 2 nomme |
| **supprimer le répertoire du registre quand il se vide** | une course avec le staging du spawn suivant, pour récupérer un inode. Et le vol 2 a fait de l'absence du répertoire un état *signifiant* (`scope-absent`, un remède différent de `scope-unreadable`) : le vider est gratuit, le supprimer brouille un motif |

---

## 4. Fire-Disposition

Ce vol livre des détecteurs : les cas L1-L5, T1-T4 et le cas sentinelle du
§ 3.4 sont tous du code dont le chemin de succès est « aucune violation
trouvée ».

**Disposition retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE, et c'est mesuré, pas espéré.**

Détail d'implémentation :

- **Les détecteurs atterrissent armés, zéro exception.** Il n'existe aucun
  corpus à faire entrer par la bande : le registre de périmètre est
  **éphémère** (une entrée par spawn, écrite au lancement, lue par un seul
  consommateur, et dorénavant retirée à la fin). Il n'y a donc pas de donnée
  existante qui violerait la nouvelle règle au moment où elle atterrit — à la
  différence d'un lint sur l'arbre des sources, dont le premier run trouve
  toujours quelque chose. L'implémentation **vérifie** cette assertion avant de
  conclure : un `ls ~/.mika/pilot-push-scopes/ | wc -l` sur l'hôte avant
  déploiement, consigné dans le corps de PR. S'il revient non-vide avec des
  entrées de plus de 12 h, elles seront purgées au premier `mitmdump` — ce qui
  est le comportement voulu, pas une violation à excepter.
- **La forme rétrocompatible est dans la RÈGLE, jamais dans l'allowlist.** Une
  entrée écrite par le binaire précédent (sans `ref=` — le point 5 du ticket,
  porté par le vol 4) est l'état **normal** pendant la fenêtre de déploiement.
  Elle est traitée comme valide par le lecteur, qui ignore déjà toute clé
  inconnue. Une entrée d'allowlist pour cette forme serait stale par
  construction : elle réapparaît à chaque déploiement. Même raisonnement pour
  ce vol : une entrée plus vieille que le TTL n'est pas une exception, c'est la
  règle qui s'applique.
- **L'assertion auto-nettoyante** est le cas **T3**. Il ne teste pas la valeur
  de `_PUSH_SCOPE_TTL_SECONDS` (un test qui relit une constante n'atteste rien)
  mais sa **conséquence** : un périmètre purgé pendant qu'un voisin est vif
  refuse un push légitime. Le jour où quelqu'un serre la constante sous la durée
  d'un spawn, T3 rougit avec le motif exact du § 2.3, et non pas un diff de
  nombre.

---

## 5. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_unstage_pilot_egress_push_scope` + site de fin de spawn + relais dans le trap EXIT | 75 |
| `scripts/mika-pilot-github-auth-addon.py` — `_purge_registry(directory, ttl)`, `_PUSH_SCOPE_TTL_SECONDS` et sa dérivation, les deux sites d'appel | 70 |
| tests (`skills/bundled/_shared/tests/test_egress_push_scope_staging.sh`) — cas L1-L5 | 95 |
| tests (`skills/bundled/_shared/tests/test_egress_push_bound.sh`) — cas T1-T4 | 70 |
| tests (`skills/bundled/_shared/tests/test_egress_sentinel_enforcement.sh`) — TTL sentinelle inchangé | 30 |

Total estimé : 340 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 340 passe sous 429 avec ~90 lignes de marge, et
cette marge est voulue : la densité de commentaire de cette surface est haute
(le vol 2 a porté 261 lignes d'addon pour ~120 lignes d'exécutable), et les deux
morts par volume citées par la règle avaient sous-estimé d'un facteur 1,6.
Aucune phase, aucun découpage interne : les cinq livrables sont un seul commit
logique par la règle mika#2578 (les cas vivent dans le commit qui crée la
règle), et l'ordre du § 6 les range sans les séparer en PR.

**Pourquoi la dimension branche n'est pas dedans.** Les deux moitiés du ticket
mesurées ensemble donnent ~515 lignes au même barème (dimension branche :
producteur +35, lecteur et motif de refus +150, cas lecteur +110, cas producteur
+75 ≈ 370, plus les 340 moins ce qui se recouvre). Au-dessus de 429, la règle
impose un découpage **en vols**. Des deux découpages possibles, un seul est
permis par le ticket (§ 1) : la dimension branche sans le cycle de vie est
explicitement interdite, le cycle de vie sans la dimension branche ne l'est pas
et en est le prérequis. Le découpage n'est donc pas choisi, il est forcé.

Tracked in: la dimension branche (`ref=refs/heads/<branche>`, motif
`push-scope-ref`, les points 1 et 5 du corps du ticket) part dans un ticket de
suivi **vol 4 de mika#2736**, ouvert par l'implémentation de ce vol avant
l'ouverture de sa PR (règle mika#2701). Le § 2.6 lui livre déjà la mesure qu'il
devait faire.

---

## 6. Ordre de commit, et son plancher

**Plancher — mesuré avant la première ligne (§ 2.1).** La couture du vol 2 doit
être sur `main`. `step 0` du § 7 est cette mesure, et son échec est terminal :
pas de PR, un rapport.

1. **`feat(infra): le TTL de purge est celui du registre, pas celui du module`**
   — `addon.py` seul : signature de `_purge_registry`, `_PUSH_SCOPE_TTL_SECONDS`
   et sa dérivation en commentaire, les deux sites d'appel, **plus les cas T1,
   T2 et le cas sentinelle**. Mergeable et vérifiable seul : à ce point la
   désinscription n'existe pas, donc le TTL est le seul mécanisme de fin de vie
   — exactement l'état que le ticket qualifie de « garde qui se desserre tout
   seul », mais sans aucune dimension branche à desserrer, donc inoffensif.
2. **`feat(infra): une entrée de périmètre meurt avec son spawn`** —
   `dispatch-lib.sh` : `_unstage_pilot_egress_push_scope`, le site de fin de
   spawn, le relais du trap, **plus les cas L1-L5**.
3. **`feat(infra): un périmètre purgé sous un voisin vif refuse un push légitime`**
   — le cas **T3** seul. Séparé des deux premiers **délibérément** : il est
   l'assertion qui lie la valeur de la constante du commit 1 au mécanisme du
   commit 2, et il ne peut pas rougir avant que les deux existent. Un cas placé
   plus tôt serait vert pour la mauvaise raison.
4. **`docs: les deux surfaces du périmètre déclaré apprennent son cycle de vie`**
   — `docs/operator/environment-variables.md` (l'entrée
   `MIKA_PILOT_EGRESS_PUSH_SCOPE` apprend la désinscription, le TTL de 12 h et
   sa dérivation, et la sonde `ls ~/.mika/pilot-push-scopes/`) et
   `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
   (un § pour le cycle de vie, et le § 2.3 de ce plan — l'asymétrie du sens de
   l'erreur — qui est le fait le moins évident de la surface).

Les commits 1 et 2 tiennent chacun la règle mika#2578 : leurs cas voyagent avec
eux. Le commit 3 est l'exception qui dit pourquoi elle en est une.

---

## 7. Definition of Done

0. **Plancher mesuré.**
   `grep -q '_stage_pilot_egress_push_scope' skills/bundled/_shared/dispatch-lib.sh && grep -q '_push_scope_verdict' scripts/mika-pilot-github-auth-addon.py`.
   Si l'un des deux manque, l'implémentation **s'arrête immédiatement**, ne
   commit rien, et rapporte `PIPELINE_INCOMPLETE — plancher non tenu : le vol 2
   (mika#2751) n'est pas sur main, il vit dans le brouillon #2767`. Elle ne pose
   **aucune question** (une session dispatchée qui demande est une session
   morte) et n'empile **pas** sa branche sur `feat/2751/...`.
1. `_unstage_pilot_egress_push_scope` existe dans `dispatch-lib.sh`, retire
   exactement l'entrée de son nonce, vide `_PILOT_EGRESS_PUSH_SCOPE_ENTRY`,
   retourne toujours 0, et dit sur stderr pourquoi quand le retrait échoue.
2. `_PUSH_SCOPE_TTL_SECONDS = 12 * 3600` existe dans l'addon, son commentaire
   porte **la mesure** de la durée du spawn le plus long observé (pas « des
   heures ») et la dérivation du § 3.2, et `_REGISTRY_TTL_SECONDS` vaut toujours
   `72 * 3600`.
3. `_purge_registry` prend son TTL en paramètre ; ses deux appelants passent
   chacun le leur ; le throttle reste par répertoire.
4. La désinscription est appelée à la fin de `_run_pilot_sandboxed` **après** la
   boucle de fermeture des fd et **avant** `return "$_sandbox_rc"`, et le statut
   de sortie du pilote est bit-pour-bit celui d'avant ; et une fois en filet
   dans `_dispatch_lib_exit_trap`, no-op quand le nonce est vide.
5. Les cas L1-L5, T1-T4 et le cas sentinelle passent. Les trois suites sont
   **vérifiées inscrites** au `Makefile` et dans `.github/workflows/ci.yml` (une
   inscription manquante est ajoutée, pas supposée).
6. **Aucun nouveau nom d'interrupteur** n'apparaît dans l'arbre :
   `git diff origin/main | grep -c 'MIKA_PILOT_EGRESS_[A-Z_]*='` ne fait
   apparaître que les deux existants.
7. Les deux cas positifs de mika#2736 (sonde de 4 octets, push chunked) passent
   toujours, et le cas T4 le tient.
8. Les deux surfaces documentaires du commit 4 sont écrites, et
   `docs/operator/environment-variables.md` porte la sonde opérateur.
9. Suite complète, `cargo clippy`, les lints du dépôt, et
   `scripts/verify-pipeline.sh origin/main` au vert.
10. Le ticket de suivi **vol 4** (dimension branche) est ouvert et son numéro
    porté dans le corps de PR, **avant** l'ouverture de la PR (mika#2701).
11. Corps de PR : `Code review: delegated to mika-qa (mika#2694)`, le résultat
    de la sonde du § 4 (`ls ~/.mika/pilot-push-scopes/`), et `Closes #2766` —
    avec la part reportée nommée par `Tracked in:`.

---

## 8. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des points 1 à 6 de son § *Travail attendu* et
du § 7 ci-dessus.

- **AC1** — Une entrée du registre de périmètre est **retirée à la fin du spawn
  qui l'a écrite**, en utilisant le nonce que le lanceur connaît déjà
  (`_PILOT_EGRESS_PUSH_SCOPE_ENTRY`). *(point 2 du ticket ; cas L1)*
- **AC2** — Deux spawns successifs d'un même dispatch ne laissent **aucune**
  entrée orpheline : le registre est vide à la fin, pas peuplé d'une entrée par
  spawn. *(§ 2.4 ; cas L3)*
- **AC3** — La désinscription d'un spawn **ne touche pas** l'entrée d'un
  dispatch concurrent vif. *(sémantiques d'union du vol 2 ; cas L2)*
- **AC4** — Un échec de désinscription **ne change pas** le statut de sortie du
  pilote et laisse une trace sur stderr. *(§ 3.1 ; cas L4)*
- **AC5** — Le registre de périmètre a **son propre TTL**, strictement inférieur
  aux 72 h des sentinelles, et les deux coexistent : une entrée de périmètre et
  une sentinelle du même âge connaissent des sorts différents. *(point 2 ; cas
  T1, T2 et le cas sentinelle)*
- **AC6** — La valeur du TTL est **justifiée par une mesure** inscrite dans le
  code, et son erreur dans le sens serré est tenue par une assertion sur sa
  conséquence — un périmètre purgé sous un voisin vif refuse un push légitime.
  *(§ 2.3, § 3.2 ; cas T3)*
- **AC7** — **Aucune variable d'environnement** nouvelle ne gouverne le TTL, et
  **aucun troisième interrupteur** n'apparaît sur cette surface ; les deux
  existants restent orthogonaux. *(point 3 ; DoD 6)*
- **AC8** — Les cas atterrissent dans les suites existantes du vol 2 et dans le
  commit qui crée la règle, pas à côté. *(point 4, mika#2578 ; § 3.4)*
- **AC9** — Les deux cas positifs de mika#2736 — sonde d'authentification de
  4 octets, push chunked — **restent positifs**. *(point 4 ; cas T4)*
- **AC10** — Les deux surfaces documentaires sont mises à jour :
  `docs/operator/environment-variables.md` et la note de solution
  `une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`.
  *(point 6 ; commit 4)*
- **AC11** — La part reportée (dimension branche, `ref=`) porte un numéro de
  ticket **avant** l'ouverture de la PR, et ce numéro est dans le corps de PR
  sur une ligne `Tracked in:`. *(mika#2701 ; DoD 10)*

---

## 9. Risques et arbitrages

| risque | sens de la panne | ce qui le tient |
|---|---|---|
| le plancher n'est pas tenu au moment du dispatch (#2767 toujours en brouillon) | aucune ligne n'atterrit ; un rapport, pas une question | DoD 0, et il est **step 0** précisément pour que l'échec coûte deux tours et non deux cents |
| le TTL est serré sous la durée d'un spawn par un futur commit | un pilote vivant est 403 sous un voisin vif — la boucle s'arrête | cas T3, qui teste la conséquence et non la constante (§ 4) |
| la désinscription écrase le statut de sortie du pilote | un pilote en échec rapporté en succès | placement après la capture de `_sandbox_rc`, retour toujours 0, cas L4 |
| un trap posé dans `_run_pilot_sandboxed` écraserait le trap du dispatch | le callback est perdu | écarté au § 3.5 ; l'appel est explicite |
| 12 h est encore trop court, pour un spawn que personne n'a vu | un 403 sur un pilote innocent, rare et difficile à lire | la mesure exigée en DoD 2 ; et le sens de l'erreur est documenté dans la note de solution, donc le prochain qui touche la valeur lit d'abord lequel des deux côtés coûte |
| une entrée résiduelle d'avant ce vol traîne sur l'hôte | du desserrement sur la dimension dépôt, bornée par les dépôts dispatchables | la sonde du § 4, et le premier `mitmdump` les purge |

---

## 10. Hors périmètre, délibérément

- **La dimension branche** (`ref=refs/heads/<branche>`, motif `push-scope-ref`,
  points 1 et 5 du ticket). Reportée pour une raison de **borne par vol**, pas
  de conception, et dans le seul sens que le ticket permet (§ 1, § 5). Le § 2.6
  livre déjà au vol 4 la mesure de la portée de `BRANCH` qu'il devait faire.
  `Tracked in:` — vol 4 de mika#2736, ouvert avant la PR de ce vol.
- **P4, le force-push.** Hors d'atteinte côté proxy par le protocole :
  `receive-pack` ne porte aucun drapeau de force. Inchangé par ce vol, et
  inchangé par la dimension branche — mesuré au § 2.4 du plan de mika#2736.
- **Les quatre règles statiques de mika#2736** et la **dimension dépôt** de
  mika#2751. Ce vol n'y touche pas ; il ajoute un paramètre à une purge et une
  fonction de retrait.
- **La moitié `api.github.com`**, livrée par mika#2724. Son TTL de 72 h est
  explicitement **conservé**, et un cas l'asserte.
- **L'extension du canari** (`scripts/canary-pilot-containment`). Le vol 2 l'a
  étendu de 132 lignes pour la dimension dépôt ; le cycle de vie n'ajoute aucun
  verdict observable de l'intérieur de la sandbox, donc il n'y a rien à y
  canariser qui ne soit déjà tenu par les suites.

---

## 11. Référence

- Ticket : senara-solutions/mika#2766
- Vol 1 : senara-solutions/mika#2736 — la borne statique (PR #2753, mergée) ;
  plan `docs/plans/2026-10-17-001-fix-2736-egress-pilote-moitie-git-push-plan.md`
- Vol 2 : senara-solutions/mika#2751 — la dimension dépôt (**brouillon #2767**,
  plancher de ce vol) ; plan
  `docs/plans/2026-10-17-003-feat-2751-egress-push-scope-declare-plan.md`
- L'asymétrie hôte/sandbox dont le périmètre tire son autorité :
  senara-solutions/mika#2724
- Incident fondateur de P4 : senara-solutions/mika#1318
- Deux causes ne doivent pas arriver dans les mêmes octets : mika#2205
- Les cas CI dans le commit qui crée la règle : mika#2578
- Orthogonalité prouvée des interrupteurs : mika#2165
- Ce qui est reporté porte un numéro avant la PR qui le reporte : mika#2701
- Fire-Disposition, les trois options canoniques : mika#1574, mika#2306
- Sauvetage de worktree sale (le préfixe `wip(` de #2767) : mika#1282

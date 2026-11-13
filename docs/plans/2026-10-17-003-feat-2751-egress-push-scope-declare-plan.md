# feat(infra) — borner le `git-receive-pack` de pilote au périmètre que le spawn a déclaré : le canal, et la dimension que le canal ferme seul (mika#2751)

**Ticket :** senara-solutions/mika#2751 — vol 2 de mika#2736
**Labels :** `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`
**Vol 1 :** senara-solutions/mika#2736 (PR mergée) — la borne statique
**Plan du vol 1 :** `docs/plans/2026-10-17-001-fix-2736-egress-pilote-moitie-git-push-plan.md`

---

## 1. Pourquoi, et ce que ce plan répond

mika#2736 a posé une borne sur le `git-receive-pack` du pilote en ne demandant
rien au client : le dépôt est dans l'URL, les commandes de ref sont en clair en
tête du corps, et le proxy refuse ce qu'il voit déjà. Quatre règles, cinq motifs.
La borne est **statique** : elle autorise tout `refs/heads/<quoi que ce soit>`
sous `senara-solutions/<n'importe quel dépôt>`.

Ce ticket ferme la part **relationnelle** de P3 : non pas « est-ce une
branche ? » mais « est-ce **la** branche de **ce** dispatch ? ».

Le point 1 du ticket demande de **décider par où le périmètre atteint l'addon**,
et de mesurer plutôt que de supposer. Les deux mesures qui répondent (§ 2.1 et
§ 2.2) ne donnent pas la réponse attendue : le canal n'est pas l'environnement du
relais, et la dimension *dépôt* et la dimension *branche* n'ont pas du tout le
même coût. C'est de là que vient le découpage en deux vols du § 5 — d'une mesure,
pas d'un budget de lignes.

**Ce vol livre : le canal, et la dimension dépôt qu'il ferme entièrement et
seul.** Le vol 3 livre la dimension branche, qui est indissociable d'un cycle de
vie d'entrée que ce vol n'a pas besoin de poser.

---

## 2. Mesures — exécutées au grooming, pas déduites

### 2.1 L'environnement du `mitmdump` **n'est pas** un canal par spawn

Le ticket suppose que « l'addon tourne dans un `mitmdump` lancé par l'hôte, donc
son environnement est écrit par l'hôte ». La première moitié est vraie, la
conclusion ne l'est pas — et c'est la mesure qui oriente tout le reste.

`_ensure_pilot_helper` (`skills/bundled/_shared/dispatch-lib.sh:1355-1446`) sonde
le port du relais ; **si quelqu'un écoute, elle `return 0` sans rien relancer**
(`:1400`). Le `nohup mitmdump` de `:1411` n'est exécuté que si le port est mort,
ou si le verdict de fraîcheur de mika#2029 déclare le démon périmé.

Trois conséquences, toutes contraignantes :

1. L'environnement du démon est celui du **premier** dispatch qui l'a lancé, et
   il n'est **jamais** rafraîchi par les suivants. Un périmètre exporté au spawn
   *n*+1 n'atteindrait pas un démon lancé au spawn *n*.
2. Le démon est **partagé** : deux pilotes concurrents parlent au même processus,
   au même instant. Une variable d'environnement n'a qu'une valeur ; deux
   périmètres simultanés n'y tiennent pas.
3. L'addon le sait déjà et l'écrit : `_registry_dir()` est « resolved per call,
   not at import: mitmdump is long-lived, and a path frozen at load would ignore
   a relocated registry until a proxy restart »
   (`scripts/mika-pilot-github-auth-addon.py:251-260`), et le choix d'un
   **répertoire** plutôt que d'un fichier pour les sentinelles est justifié mot
   pour mot par la concurrence (`dispatch-lib.sh:1824-1826` : « two concurrent
   pilots must both be valid at the SAME instant »).

**⇒ Le périmètre voyage par un fichier hôte relu à chaque requête, pas par
l'environnement.** C'est le même canal que le registre de sentinelles de
mika#2716, et c'est de là que le périmètre tire l'autorité que le ticket lui
prête : le répertoire vit sous `~/.mika`, qui n'est **pas** parmi les binds de la
sandbox (`_pilot_gitdir_bind_args`, § 2.3), donc rien dans la sandbox ne peut ni
le lire ni l'écrire. L'asymétrie de mika#2724, intacte.

**L'ordre d'écriture, qui était la seconde moitié de la question, est
favorable.** `REPO` (basename nu) et `BRANCH` sont résolus dans
`_set_up_worktree` (`:4265-4269` et `:4339-4343`), bien avant
`_run_claude_pilot` (`:4844`) et son appel à `_run_pilot_sandboxed` (`:4911`).
`_stage_pilot_egress_sentinel` tourne à `:2295`, **dans**
`_run_pilot_sandboxed` : les deux valeurs y sont donc en portée. Le périmètre
s'écrit au même endroit, dans la même séquence, sans déplacer une seule ligne de
l'existant.

### 2.2 L'union est imposée par le registre partagé — et elle ne coûte pas le même prix selon la dimension

Un démon partagé lu par requête ne peut pas attribuer une requête à un spawn :
sur `github.com` aucun credential ne voyage (c'est la mesure fondatrice de
mika#2736, § 2.5 de son plan), et le relais d'egress expose **une seule** socket
unix bind-montée dans chaque sandbox (`_PILOT_EGRESS_SOCK`, `:2313`), donc
l'identité du client est déjà perdue en amont de l'addon.

Le verdict est donc nécessairement : **« existe-t-il une entrée vive qui déclare
ce dépôt ? »** — l'union des périmètres vifs. Et l'union se comporte de deux
façons radicalement différentes :

| dimension | cardinal de l'union | sensibilité au cycle de vie de l'entrée |
|---|---|---|
| **dépôt** | ≤ 3 — le nombre de dépôts que la boucle peut dispatcher (`DISPATCHABLE_REPO_BASENAMES`, garde mika#2046/#2062 en `:4284`) | **nulle.** Avec trois dispatches vifs sur trois dépôts, l'union vaut 3 ; avec un TTL de 72 h, elle vaut encore 3. Le resserrement est complet sans toucher au cycle de vie. |
| **branche** | croît avec le nombre de spawns retenus | **totale.** Avec le TTL de 72 h du registre existant, l'union couvre tous les spawns de trois jours — des dizaines de branches. Le scénario que le ticket nomme (« la branche d'un autre dispatch ») **resterait ouvert**. |

**⇒ Pour la branche, le cycle de vie n'est pas un détail d'implémentation : il
*est* le resserrement.** Désenregistrement en fin de vol, TTL de secours court,
et les chemins de sortie de `_run_pilot_sandboxed` à couvrir. Pour le dépôt, rien
de tout cela n'est nécessaire.

C'est cette asymétrie — et non un budget — qui découpe le ticket en deux vols
(§ 5). Un vol unique paierait le cycle de vie complet pour une dimension qui n'en
tire aucun bénéfice, et livrerait la dimension branche **à moitié** si la mesure
réelle dépassait.

### 2.3 Un pilote n'a qu'un seul `origin` possible, donc le resserrement du dépôt ne peut refuser aucun push légitime

C'est l'anti-vacuité de la décision : si le pilote pouvait pousser dans deux
dépôts, resserrer sur un seul arrêterait la boucle.

`_pilot_gitdir_bind_args` (`:1948-2060`) monte exactement : le gitdir du worktree
(rw), puis `objects` (rw), `refs` / `config` (ro) et `packed-refs` / `info`
(ro-try) du **seul** `.git` parent — celui de `$SUB_REPO_DIR`. Aucun autre dépôt
du workspace n'entre dans le namespace. Le pilote ne peut donc pousser que depuis
ce dépôt, vers son `origin`, soit `senara-solutions/$REPO` : exactement le
périmètre que `_set_up_worktree` a déclaré.

Un `git clone` d'un autre dépôt dans la sandbox reste possible (le
`git-upload-pack` n'est pas borné, délibérément — § 2.2 du plan du vol 1), et son
push serait refusé. **C'est le resserrement, pas une régression.**

### 2.4 Le canari ne déclare aucun périmètre — sa non-régression est accidentelle aujourd'hui

`scripts/canary-pilot-containment` exporte `WORKTREE_DIR` (`:241`) et **ni `REPO`
ni `BRANCH`** (vérifié : aucune autre affectation de ces deux noms dans le
fichier). Ses trois sondes `push_bound_*` (`:799-806`) visent
`senara-solutions/canary-2736-no-such-repo.git`, qui n'est le dépôt d'aucun
dispatch.

Donc : sans périmètre déclaré, le canari **dégrade** vers la borne statique et
ses trois sondes restent vertes — mais pour une raison qui n'est écrite nulle
part. Si un jour le canari exportait `REPO`, `push_bound_nominal` rougirait
d'un coup, et le diagnostic serait à reconstruire.

**⇒ Le vol rend cette dépendance explicite plutôt que de l'hériter**, et s'en
sert : le canari tourne **côté hôte**, donc il peut écrire le registre, et c'est
ce qui lui donne la sonde en vol du resserrement (§ 3.5).

### 2.5 Un défaut latent de la purge, à corriger avant d'ajouter un second registre

`_purge_registry` (`addon:307-337`) prend déjà son répertoire en **argument** —
donc réutilisable tel quel — mais son throttle s'appuie sur un **unique**
`_last_purge` global (`:182`, `:322-326`). Appelée pour deux répertoires, la
fenêtre de dix minutes ne laisse purger que le premier des deux par fenêtre, en
alternance indéterminée.

Conséquence s'il n'est pas corrigé : le registre de périmètres grossirait d'une
entrée par spawn sans jamais être purgé la moitié du temps — et un périmètre
périmé **élargit l'union**, c'est-à-dire relâche silencieusement le resserrement
que ce vol existe pour poser. Le défaut est dans l'existant ; il devient
exploitable à l'instant où un second registre arrive. Il est corrigé ici (§ 3.2),
avec son cas de test.

### 2.6 La suite du vol 1 n'est lancée par aucun job — le point 4 du ticket est déjà en défaut

Mesuré, parce que le point 4 du ticket demande d'étendre cette suite et que le
plancher du § 6 en dépend : `make test-egress-push-bound` **existe**
(`Makefile:260`, déclarée dans le `.PHONY` de `Makefile:4`) et **aucun fichier de
`.github/` ne l'appelle**. Le bloc qui aurait dû la porter — « Pilot sandbox
secret-channel guards (mika#2039, mika#2716, mika#2724) », `ci.yml:111-116` —
lance les deux suites de sentinelles et s'arrête là.

Donc **les treize cas livrés par mika#2736 ne gatent rien sur une PR
aujourd'hui.** C'est mot pour mot la leçon mika#2578, et `ci.yml:118-123` la
nomme déjà deux fois pour d'autres suites (« was local-only, so the guard …
gated nothing on a PR »).

Conséquence directe sur ce vol : étendre la suite sans la câbler ajouterait sept
cas à un fichier que rien n'exécute — le point 4 du ticket serait formellement
satisfait et matériellement vide. **Le câblage est donc le premier commit**
(§ 6), avant toute modification de l'addon, pour que les treize cas existants
soient gatants *au moment* où le code neuf arrive.

---

## 3. La décision, et sa conception

### 3.1 La forme retenue — un resserrement **sous** la borne, jamais à côté

Le point 2 du ticket est la contrainte structurante : « si le périmètre est
absent ou illisible, la borne statique doit continuer de s'appliquer — jamais un
`return` qui ouvre tout ». La conception la respecte **par construction** et non
par vigilance :

```
request(flow)
  ├─ flow.response déjà posée ? → return                    (inchangé, mika#2724)
  ├─ host != github.com ? → return                          (inchangé)
  ├─ MIKA_PILOT_EGRESS_PUSH_BOUND désarmé ? → return        (inchangé, mika#2736)
  ├─ pas un git-receive-pack ? → return                     (inchangé)
  ├─ corps illisible → 403 push-body-unreadable             (inchangé)
  ├─ _push_verdict(repo, commands)  ← LA BORNE STATIQUE, intacte
  │     └─ refus → 403 <un des quatre motifs>               (inchangé)
  └─ _push_scope_verdict(repo)      ← LE RESSERREMENT, NOUVEAU
        ├─ désarmé (MIKA_PILOT_EGRESS_PUSH_SCOPE=0) → allow
        ├─ registre illisible        → allow + log scope-unreadable
        ├─ registre sans périmètre   → allow + log scope-absent
        ├─ dépôt dans l'union        → allow
        └─ sinon                     → 403 push-scope-repo
```

Deux propriétés tombent de cette place, et aucune n'est à vérifier à la
relecture :

- **Le resserrement ne voit qu'un push que la borne statique a déjà autorisé.**
  Il n'existe donc aucun chemin par lequel il transformerait un refus en
  autorisation — le `return` qui ouvre tout est structurellement absent, pas
  évité.
- **Il ne peut qu'ajouter un refus.** Sa seule sortie nouvelle est un 403 de plus
  sur un push que mika#2736 laissait passer.

### 3.2 Le canal — `~/.mika/pilot-push-scopes/`, un registre **dédié**

Le plan du vol 1 estimait ce vol en *étendant l'entrée du registre de
sentinelles* (§ 5 : « exige d'étendre l'entrée du registre de sentinelles du vol 1
de mika#2716 »). **La mesure du § 2.2 tranche autrement, et le plan le dit plutôt
que de suivre son prédécesseur :** les deux données n'ont ni le même cycle de vie
(72 h pour une sentinelle ; la durée d'un vol pour un périmètre de branche, au
vol 3), ni la même posture de défaillance (une sentinelle **refuse** sur registre
illisible ; un périmètre **dégrade**), ni le même interrupteur. Les loger dans un
même fichier ferait porter à une donnée deux contrats opposés, et obligerait le
vol 3 à introduire un second TTL **dans** le registre de sentinelles — c'est-à-dire
à re-toucher un garde déjà livré pour y faire cohabiter deux horloges.

Un registre dédié coûte, mesuré, un répertoire et un nonce de plus : la purge est
déjà paramétrée par répertoire (§ 2.5), et le reste de la machinerie est partagé.

| | valeur |
|---|---|
| répertoire | `$HOME/.mika/pilot-push-scopes/`, `0700` |
| override | `_PILOT_EGRESS_PUSH_SCOPE_DIR`, résolu **à l'appel** (même raison que `_registry_dir`, et même raison que la résolution à l'appel côté shell : les suites `export HOME=` *après* le `source`) |
| nom d'entrée | un nonce de 32 hex, tiré comme celui de la sentinelle, mémorisé dans `_PILOT_EGRESS_PUSH_SCOPE_ENTRY` (le vol 3 en aura besoin pour désenregistrer ; ce vol ne l'utilise pas et ne prétend pas le contraire) |
| contenu | des lignes `clé=valeur`. Ce vol en écrit **une** : `repo=senara-solutions/<REPO>`. Les clés inconnues sont **ignorées à la lecture**, ce qui fait du `ref=` du vol 3 une addition pure, sans migration |
| TTL | `_REGISTRY_TTL_SECONDS` (72 h), **réutilisé tel quel** — insensible pour la dimension dépôt (§ 2.2) ; le vol 3 pose le sien |
| mode | `0600` par `umask 077`, comme la sentinelle |

**Un seul lecteur de l'interrupteur, et il l'honore.** `MIKA_PILOT_EGRESS_PUSH_SCOPE`
est lu **uniquement** dans l'environnement du `mitmdump`, par
`_push_scope_enforced()` — exactement la position de lecture de
`MIKA_PILOT_EGRESS_PUSH_BOUND`, et pour la même raison : c'est là que le refus
vit. Le lanceur, lui, écrit le périmètre **inconditionnellement** ; un nom de
dépôt n'est pas un secret et une entrée que personne n'applique ne fait rien.
Cela évite un relais `EGRESS_SENTINEL_ENV` dans
`crates/mika-agent/src/skills/executor.rs` — et surtout évite un second lecteur,
qui est précisément la façon dont mika#2165 se paie : un interrupteur à deux
lecteurs dont l'un l'ignore est décoratif.

**Le correctif de purge (§ 2.5) :** `_last_purge` devient un `dict[str, float]`
clé par répertoire résolu. Six lignes, et un cas de test qui rougit si le throttle
redevient global.

### 3.3 `_stage_pilot_egress_push_scope` — côté lanceur

Placée dans `_run_pilot_sandboxed` immédiatement après
`_stage_pilot_egress_sentinel` (`:2295`), avant `_ensure_pilot_helper`. L'ordre
relatif aux deux voisins n'est **pas** porteur ici — contrairement au couple
token/sentinelle, dont l'inversion stagerait la sentinelle comme vrai token
(`:1806-1815`) — et le plan le dit pour que personne n'invente une contrainte en
lisant la symétrie.

```
_stage_pilot_egress_push_scope()
  ├─ REPO vide (mode free-text, mode milestone) → n'écrit rien, une ligne stderr
  │     nommée, et le dispatch continue : la borne statique s'applique seule.
  ├─ tire un nonce (openssl, puis /dev/urandom en secours — même cascade que la
  │     sentinelle)
  ├─ ( umask 077; mkdir -p $dir && chmod 0700 $dir
  │     && printf 'repo=senara-solutions/%s\n' "$REPO" > "$dir/$nonce" )
  └─ échec → ligne stderr nommée, dégradation. Fail-open, même posture que
        _stage_pilot_gh_token et _stage_pilot_egress_sentinel : un dispatch ne
        meurt pas sur un fichier de bookkeeping.
```

**Pas de bracket `set +x`.** Les deux voisines en portent un parce qu'elles
manipulent un credential et que `_emit_callback` renvoie la trace à l'appelant.
Un nom de dépôt n'est pas un secret ; ajouter le bracket par symétrie rendrait
illisible la raison pour laquelle les autres l'ont. Dit ici parce que le
copier-coller est l'issue naturelle.

**La casse.** La comparaison addon-side est insensible à la casse sur
`owner/repo` (GitHub l'est), comme `_push_verdict` l'est déjà sur l'owner
(`addon:647`). Le lanceur écrit `senara-solutions/$REPO` sans suffixe `.git` ;
`_receive_pack_repo` retire déjà ce suffixe (`:559-561`), donc les deux formes se
rencontrent normalisées.

### 3.4 La suite CI — étendue dans `test_egress_push_bound.sh`, pas à côté

Point 4 du ticket, et c'est aussi le moins coûteux : le harnais existant
(`stub mitmproxy`, `importlib`, `run()` qui pilote `requestheaders` **puis**
`request`, `pkt()`/`cmd()`/`body()`) sert tel quel. **Les treize cas existants
restent verts sans édition** : aucun ne déclare de périmètre, donc tous dégradent
— et ce fait est lui-même le cas 14.

Sept cas, chacun répondant à une défaillance nommée plus haut :

14. **Sans périmètre déclaré, tout se comporte exactement comme mika#2736.**
    Le registre est absent, et les cas 1 à 13 passent inchangés. C'est la
    non-régression du point 2 du ticket, assertée et non promise.
15. **Périmètre déclaré, push dans le périmètre → passe**, et l'`Authorization`
    sortant est toujours **celui de l'hôte**. L'anti-vacuité de tous les
    suivants : un resserrement qui refuserait tout rougit ici seul.
16. **Périmètre déclaré, autre dépôt de l'organisation → 403
    `push-scope-repo`.** La seconde puce du ticket, fermée. Le corps nomme le
    dépôt visé (donnée client, pas un secret — mika#1949 interdit de nommer le
    justificatif présenté, jamais la cible).
17. **Deux entrées vives → chacun des deux dépôts passe, un troisième est
    refusé.** L'union du § 2.2, épinglée : sans ce cas, deux pilotes concurrents
    seraient une régression découverte en vol.
18. **Registre lisible mais sans aucune clé `repo=` → dégradation**, ligne
    `scope-absent`, et la statique mord toujours (`refs/heads/main` → 403
    `push-ref-protected`). **Registre illisible → dégradation**, ligne
    `scope-unreadable`. Les deux causes portent deux lignes distinctes
    (mika#2205) ; une entrée individuellement illisible est **ignorée** sans
    refuser, et n'empêche pas une entrée saine voisine d'autoriser.
19. **La sonde de 4 octets et le push chunked restent positifs avec le
    resserrement armé et un périmètre déclaré.** Le point 4 du ticket, et le seul
    mode de défaillance capable d'arrêter la boucle : un garde qui 403 le POST de
    sonde refuse tout push assez gros pour être streamé.
20. **La matrice des interrupteurs**, et la **subordination** est assertée comme
    telle :

    | `PUSH_BOUND` | `PUSH_SCOPE` | attendu |
    |---|---|---|
    | 1 | 1 | statique + resserrement |
    | 1 | 0 | **exactement mika#2736** — la statique seule |
    | 0 | 1 | rien : `PUSH_BOUND` est le disjoncteur général, le resserrement vit dessous |
    | 0 | 0 | rien |

    La ligne (0, 1) est une **décision**, pas un défaut d'orthogonalité : le point
    3 du ticket demande qu'on puisse désarmer le resserrement sans désarmer la
    statique — l'inverse n'aurait aucun sens, et un opérateur qui coupe « la
    borne » et garde un refus a un recours plus gros que son problème. Le test
    l'asserte **avec** ce commentaire, pour qu'une lecture naïve ne la corrige
    pas. L'orthogonalité au sens de mika#2165 est asservie ailleurs, et elle est
    vraie : `api.github.com` est intact sous `PUSH_SCOPE=0`, et le resserrement
    est intact sous `MIKA_PILOT_EGRESS_SENTINEL=0`.

Plus un cas dans une suite nouvelle, `test_egress_push_scope_staging.sh`, calquée
sur `test_egress_sentinel_staging.sh` : le lanceur écrit bien une entrée `0600`
sous un répertoire `0700`, son contenu est `repo=senara-solutions/<REPO>`, un
`REPO` vide n'écrit rien, un répertoire non inscriptible fail-open **sans tuer le
dispatch**, et le throttle de purge est bien par répertoire (§ 2.5).

**Le câblage CI est un livrable, pas une formalité** (§ 2.6). Deux entrées
rejoignent le bloc `ci.yml:111-116` : `make test-egress-push-bound` — **entrée
neuve**, puisque la suite du vol 1 n'est lancée par rien — et
`make test-egress-push-scope-staging`, cible nouvelle. La première arrive au
commit 1, avant tout code neuf ; la seconde avec la suite qu'elle lance. Leçon
mika#2578 : une garde que le pipeline ne lance pas ne refuse rien.

### 3.5 Le canari — la sonde en vol, rendue possible par le fait qu'il tourne côté hôte

Le § 2.4 a montré que le canari n'a pas de périmètre et que c'est ce qui garde ses
trois sondes vertes. Deux additions, qui rendent le fait explicite et le
monétisent :

| sonde | attendu | ce qu'elle nomme |
|---|---|---|
| `push_scope_degraded` | `no refusal` + la ligne `scope-absent` | sans périmètre, la statique s'applique seule — la non-régression du point 2, **en vol** |
| `push_scope_foreign` | `push-scope-repo` | le canari écrit une entrée de périmètre nommant un dépôt, puis pousse vers un **autre** dépôt de l'organisation : le resserrement mord réellement |

`push_scope_foreign` est réalisable précisément parce que le registre est
host-side et que le canari l'est aussi. Son entrée résiduelle est **inoffensive
par construction** : une entrée de plus ne peut qu'**élargir** l'union, jamais la
resserrer, et elle expire au TTL. Dit ici parce que l'instinct inverse (« une
entrée de test dans un registre de production ») mériterait un refus si c'était
vrai.

Deux contraintes de rédaction que le fichier impose, déjà payées deux fois
(mika#2572) : **aucune quote simple** dans les messages ajoutés — le bloc vit
dans un payload entre quotes simples — et la cible des sondes négatives doit être
un dépôt **qui n'existe pas**, pour qu'un 403 ne puisse jamais être confondu avec
une écriture qui aurait abouti.

### 3.6 Les deux surfaces documentaires (point 5 du ticket)

- `docs/operator/environment-variables.md`, section d'egress : l'entrée
  `MIKA_PILOT_EGRESS_PUSH_SCOPE` (défaut armé, vocabulaire, position de lecture,
  **lecteur unique** et pourquoi), le registre et son format, le motif
  `push-scope-repo`, les deux lignes de dégradation avec leur remède respectif,
  la matrice du § 3.4 avec la subordination écrite comme décision, et la sonde
  d'opérateur : `grep 'scope-absent' <log de mitmdump>` — **si elle apparaît à
  chaque dispatch, le resserrement ne mord jamais**, et c'est la défaillance
  silencieuse à surveiller.
- `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md` :
  une sous-section sous le § mika#2736 portant la mesure du § 2.1 (**l'environnement
  d'un démon partagé n'est pas un canal par spawn** — la leçon transposable, car
  elle vaudra pour le prochain garde qui voudra parler à `mitmdump`), celle du
  § 2.2 (l'union, et pourquoi elle sépare les deux dimensions), celle du § 2.3
  (un seul `origin` possible) et la ligne de table du canal « fichier hôte relu
  par requête », qui manquait.

---

## 4. Fire-Disposition

Ce vol livre des détecteurs : les sept cas de `test_egress_push_bound.sh`, la
suite `test_egress_push_scope_staging.sh` et les deux sondes du canari.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist VIDE,
et une violation réelle traitée par conformité plutôt que par exemption.**

- **Aucun site en violation à l'instant du merge, vérifié par énumération et non
  par un `grep` vide.** Le seul push qui traverse ce proxy est celui qu'un pilote
  lance depuis la sandbox vers son `origin`, et le § 2.3 établit
  **structurellement** — par ce que bwrap monte — que cet `origin` ne peut être
  que `senara-solutions/$REPO`, soit exactement le périmètre déclaré. Les sites
  de push hôtes (`_push_branch`, et le `refs/dispatch-gate/*` de
  `dispatch-lib.sh:3416-3418`) ne traversent pas `mitmdump` — établi au § 4 du
  plan du vol 1 — donc une entrée d'allowlist pour eux serait une exemption sans
  violation, la surface d'attaque décrite par
  `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`.
  **L'implémenteur vérifie cette frontière plutôt que de la croire :** si un de
  ces pushs hôtes passait un jour par le proxy, c'est le cas 15 qui le dirait.
- **La seule violation réelle est le canari, et elle est rendue conforme, pas
  exemptée.** Ses trois sondes `push_bound_*` visent un dépôt qui n'est le dépôt
  d'aucun dispatch (§ 2.4). Elles restent vertes par **dégradation**, et le vol
  rend ce fait explicite au lieu d'en hériter : `push_scope_degraded` l'atteste
  en vol, et `push_scope_foreign` déclare son propre périmètre pour la durée de sa
  sonde. Aucune entrée d'allowlist, aucun chemin d'exception dans le code du
  garde.
- **Assertion auto-nettoyante.** Le risque de péremption d'un détecteur de refus
  n'est pas qu'il cesse de refuser, c'est qu'il refuse **tout** en lisant vert.
  Les positifs qui rougissent exactement dans ce cas : cas 14 (les treize cas de
  mika#2736 inchangés), cas 15 (le push dans le périmètre passe, credential
  hôte inclus), cas 19 (la sonde de 4 octets et le chunked passent sous
  resserrement armé) et la sonde `push_scope_degraded`. Le cas 17 rougit si
  l'union se réduit à une seule entrée ; le cas 18 rougit si une dégradation
  devient un refus.
- **Aucun détecteur n'atterrit désarmé, ni hors pipeline.** Pas de `#[ignore]` ni
  d'équivalent, et les deux cibles sont appelées par `ci.yml`. « Hors pipeline »
  n'est pas une hypothèse ici : le § 2.6 a mesuré que la suite du vol 1 était
  exactement dans cet état — armée, verte, et lancée par personne. C'est la forme
  que prend un détecteur désarmé quand il n'a pas de `#[ignore]` pour le dire, et
  le commit 1 la corrige avant d'en ajouter. Le canari reste hors CI parce qu'il
  est post-déploiement par nature — c'est précisément pourquoi la suite existe à
  côté de lui.
- **Aucun détecteur de ce vol ne défaille en silence.** Le seul dont le mécanisme
  pouvait se vider — une fonction de verdict verte et débranchée, classe
  mika#2205 — porte son anti-vacuité dans les cas eux-mêmes : les sept passent
  par `request`, jamais par un appel direct à `_push_scope_verdict`.

---

## 5. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/mika-pilot-github-auth-addon.py` (`_push_scope_dir`, `_push_scope_entries`, `_push_scope_enforced`, `_push_scope_verdict`, motif `push-scope-repo` + corps de 403, deux lignes de dégradation, câblage dans `request`, purge par répertoire) | 115 |
| `skills/bundled/_shared/dispatch-lib.sh` (`_stage_pilot_egress_push_scope`, constantes, câblage dans `_run_pilot_sandboxed`) | 70 |
| tests (`skills/bundled/_shared/tests/test_egress_push_bound.sh`, cas 14 à 20) | 120 |
| tests (`skills/bundled/_shared/tests/test_egress_push_scope_staging.sh`) | 45 |
| `scripts/canary-pilot-containment` (deux sondes + déclaration de périmètre) | 25 |
| `Makefile` (une cible neuve) + `.github/workflows/ci.yml` (deux entrées, dont le câblage manquant du § 2.6) | 8 |

Total estimé : 383 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et en un seul.** 383 < 429, soit 46 lignes de marge, et
383 < 1000. Trois économies mesurées le portent, et aucune n'est un pari :
le harnais de test du vol 1 est réutilisé **sans édition** (§ 3.4) ; la purge est
déjà paramétrée par répertoire (§ 2.5) ; et l'interrupteur n'a **qu'un** lecteur,
ce qui met hors du diff `crates/mika-agent/src/skills/executor.rs`, son relais
d'environnement et les tests de ce relais (§ 3.2).

**Ce que la marge paie, nommément.** Si la mesure réelle dépasse, la variable
d'ajustement est le **cas 17** (l'union à deux entrées, ≈ 18 lignes) : la
propriété d'union reste alors couverte par le cas 16 pour un dépôt, et l'union
elle-même est déjà écrite dans le code de `_push_scope_entries`. **Ce ne sont ni
le cas 19 ni le cas 14** : le premier épingle les deux pièges mesurés du vol 1 —
les retirer livrerait un resserrement qui 403 tout push streamé sans que rien ne
le dise avant le premier dispatch — et le second est la non-régression que le
point 2 du ticket exige.

**La suite, découpée en vol plutôt qu'en phase, et pour une raison mesurée.** La
dimension **branche** ferme la première puce du ticket (`refs/heads/fix/9999/...`,
la branche d'un autre dispatch). Le § 2.2 établit qu'elle est **indissociable
d'un cycle de vie d'entrée** : désenregistrement en fin de vol sur tous les
chemins de sortie de `_run_pilot_sandboxed`, TTL de secours court et sa
justification d'asymétrie de coût (un TTL trop court refuse un vol encore vif
et arrête la boucle ; un TTL trop long élargit l'union). Soit ≈ 190 lignes
supplémentaires — l'addon (≈ 40), le lanceur et son désenregistrement (≈ 50),
les tests des deux surfaces (≈ 80), le canari (≈ 20) — qui porteraient ce vol à
≈ 573, au-dessus de la borne. Le canal, le registre, l'interrupteur et le harnais
de test que ce vol pose sont **exactement** ce dont ce vol 3 a besoin : il ajoute
une ligne `ref=` à un format déjà conçu pour l'accueillir (§ 3.2) et un second
motif `push-scope-ref` dans un vocabulaire déjà établi.

Ce vol est **mergeable seul** : il ferme entièrement la seconde puce du ticket
(« un autre dépôt de `senara-solutions/` »), et il ne dépend d'aucun
redéploiement au-delà du `cp` de l'addon que `Makefile:55-57` fait déjà — plus,
pour la moitié lanceur, le `make deploy` que tout changement de `dispatch-lib.sh`
exige de toute façon.

Tracked in: un ticket de suivi ouvert **avant** la PR et nommé dans son corps
(règle mika#2701), portant « borner le `git-receive-pack` de pilote à la BRANCHE
déclarée par le spawn, et le cycle de vie de l'entrée de périmètre (vol 3 de
mika#2736) ».

**P4 n'est pas une suite**, c'est une limite : le § 2.4 du plan du vol 1
établit qu'elle est hors d'atteinte côté proxy — `receive-pack` ne porte aucun
drapeau de force — et le ticket la met hors périmètre pour cette raison, pas par
report. Resserrer P3 ne change rien à P4 : la ref visée devient la bonne, la
nature de la mise à jour reste invisible.

---

## 6. Ordre de commit, et son plancher

Chaque commit laisse l'arbre vert. Le plancher est le **commit 4**.

1. `ci(egress)` — câbler `make test-egress-push-bound` dans le bloc
   `ci.yml:111-116`. *Le défaut du § 2.6, et **premier** parce qu'il est le seul
   commit qui rend les treize cas du vol 1 gatants **avant** que l'addon ne
   change : les commits 3 et 4 sont alors protégés par une garde que le pipeline
   lance réellement. Vert immédiatement — les treize cas passent déjà, ils
   n'étaient simplement lancés par personne. Un commit de deux lignes qui se
   relit seul, et qui vaut d'être porté même si le reste du vol était abandonné.*
2. `fix(infra)` — le correctif de purge du § 2.5 : `_last_purge` par répertoire,
   plus son cas dans `test_egress_sentinel_enforcement.sh`. *Séparé délibérément :
   c'est l'autre défaut de l'existant, il se relit sans le reste du vol, et il
   doit être en place avant qu'un second registre ne le rende exploitable
   (une entrée périmée **élargit** l'union). L'arbre reste vert — le throttle
   garde son comportement pour un répertoire unique.*
3. `feat(infra)` — l'addon : `_push_scope_dir`, `_push_scope_entries`,
   `_push_scope_enforced`, `_push_scope_verdict`, le motif et son corps, les deux
   lignes de dégradation, le câblage **sous** `_push_verdict` (§ 3.1).
   *L'arbre reste vert : sans registre, tout dégrade, et les treize cas existants
   passent inchangés — et depuis le commit 1, CI le vérifie.*
4. **PLANCHER** — les cas 14 à 20 dans `test_egress_push_bound.sh`. *S'arrêter
   avant le commit 4 livrerait un refus de plus sur le chemin de la boucle que
   rien n'épingle : le seul état de cet arbre qui soit pire que l'absence du
   ticket. Le câblage de la cible étant acquis au commit 1, ce commit n'a plus
   qu'à écrire les cas — c'est précisément pourquoi le câblage est passé devant.*
5. `feat(infra)` — `_stage_pilot_egress_push_scope` et son câblage, plus
   `test_egress_push_scope_staging.sh`, sa cible `Makefile` et son entrée
   `ci.yml`. *Le garde et son alimentation arrivent dans cet ordre : un garde sans
   entrée dégrade (mesuré au commit 3), une entrée sans garde ne fait rien.*
6. `feat(infra)` — les deux sondes du canari (§ 3.5).
7. `docs` — **le livrable du point 5 du ticket**, dans les deux surfaces qu'il
   nomme (§ 3.6).

**Une action précède la PR, et ce n'est pas un commit de cet arbre :** le ticket
de suivi du § 5, ouvert **avant** l'ouverture de la PR et nommé dans son corps
(règle mika#2701).

**Aucune mesure n'est prérequise au commit 1.** Les six du § 2 sont acquises au
grooming, avec les références de ligne qui permettent de les contredire.

---

## 7. Definition of Done

- [ ] `_ensure_pilot_helper` n'est pas touchée, et le périmètre voyage par un
      **fichier hôte relu à chaque requête** — jamais par l'environnement du
      `mitmdump` (§ 2.1).
- [ ] Sur `github.com`, un `POST git-receive-pack` vers un dépôt de
      `senara-solutions/` qu'**aucune entrée vive ne déclare** rend 403
      `push-scope-repo`, et le corps nomme le dépôt visé, le ticket et
      l'interrupteur.
- [ ] Un push vers un dépôt **que le périmètre déclare** passe, et
      l'`Authorization` sortant est celui de l'hôte.
- [ ] Deux entrées vives autorisent **chacun** des deux dépôts, et refusent un
      troisième (l'union du § 2.2).
- [ ] Registre absent, illisible, ou sans aucune clé `repo=` ⇒ **la borne
      statique de mika#2736 s'applique seule** : `refs/heads/main` rend toujours
      403 `push-ref-protected`. Aucun chemin ne rend un `allow` là où mika#2736
      rendait un refus.
- [ ] Les deux causes de dégradation portent **deux lignes distinctes**
      (`scope-absent`, `scope-unreadable`), et une entrée individuellement
      illisible est ignorée sans refuser ni empêcher une entrée saine
      d'autoriser.
- [ ] **La sonde d'authentification de 4 octets passe** et **un push chunked de
      grande taille passe**, resserrement armé et périmètre déclaré (point 4 du
      ticket).
- [ ] `MIKA_PILOT_EGRESS_PUSH_SCOPE=0` rend **exactement** le comportement de
      mika#2736 ; `MIKA_PILOT_EGRESS_PUSH_BOUND=0` désarme les deux, et cette
      subordination est assertée **comme décision** (§ 3.4).
- [ ] `MIKA_PILOT_EGRESS_SENTINEL=0` laisse le resserrement intact, et
      `MIKA_PILOT_EGRESS_PUSH_SCOPE=0` laisse la coupure `api.github.com` de
      mika#2724 intacte (orthogonalité, classe mika#2165).
- [ ] L'interrupteur a **un seul lecteur**, dans l'environnement du `mitmdump`,
      et il l'honore — aucun relais dans `executor.rs` (§ 3.2).
- [ ] `_stage_pilot_egress_push_scope` écrit `0600` sous un répertoire `0700`,
      n'écrit rien quand `REPO` est vide, et **fail-open** sur erreur d'écriture
      sans tuer le dispatch.
- [ ] Le throttle de purge est **par répertoire**, et un cas rougit s'il
      redevient global (§ 2.5).
- [ ] Les **treize** cas de `test_egress_push_bound.sh` livrés par mika#2736
      passent **sans édition** (cas 14), et sont désormais **lancés par un job de
      `ci.yml`** — ce qui n'était pas le cas avant ce vol (§ 2.6).
- [ ] Le canari porte `push_scope_degraded` et `push_scope_foreign` ; les trois
      sondes `push_bound_*` existantes restent vertes, et la raison en est
      désormais écrite (§ 2.4).
- [ ] `make test-egress-push-bound` et `make test-egress-push-scope-staging`
      existent, passent, et sont **appelées par `ci.yml`** — la première câblée
      au commit 1 (elle ne l'était pas), la seconde dans le commit qui crée sa
      règle. `grep -c 'test-egress-push' .github/workflows/ci.yml` rend 2.
- [ ] `make test-egress-sentinel-staging`, `make test-egress-sentinel-enforcement`,
      `make test-github-token-not-in-sandbox`, `make test-sandbox-gh-usable`,
      `make test-sandbox-git-usable`, `make test-dispatch-lib`,
      `make verify-no-secret-in-setenv` passent.
- [ ] Les deux surfaces documentaires du point 5 sont écrites (§ 3.6), dont la
      sonde d'opérateur `grep scope-absent`.
- [ ] Le ticket de suivi du vol 3 est ouvert **avant** la PR et nommé dans son
      corps (règle mika#2701).

---

## 8. Acceptance criteria

Le corps du ticket mika#2751 ne porte pas de section `## Acceptance criteria` ;
les critères ci-dessous sont dérivés des cinq points de sa section *Travail
attendu* et de son *Hors périmètre*.

- **AC1 — le périmètre atteint l'addon par un canal décidé sur mesure.** Le vol
  établit par la lecture du code que l'environnement du `mitmdump` n'est pas un
  canal par spawn (démon réutilisé et partagé), et retient un registre hôte relu
  par requête. La mesure, ses références de ligne et sa leçon transposable sont
  écrites dans la note de solution. *(point 1)*
- **AC2 — la borne de mika#2736 reste en place dessous.** Le resserrement ne voit
  qu'un push que la borne statique a déjà autorisé, et sa seule sortie nouvelle
  est un refus de plus. Périmètre absent ou illisible ⇒ borne statique seule, et
  les treize cas du vol 1 passent sans édition. *(point 2)*
- **AC3 — un interrupteur, et un seul de plus.** `MIKA_PILOT_EGRESS_PUSH_SCOPE`
  désarme le resserrement **sans** désarmer la borne statique. La matrice 2×2 est
  assertée, et l'orthogonalité avec `MIKA_PILOT_EGRESS_SENTINEL` l'est aussi
  (classe mika#2165). *(point 3)*
- **AC4 — la suite CI dans le commit qui crée la règle**, étendue **dans**
  `test_egress_push_bound.sh`. La sonde de 4 octets et le push chunked restent
  des cas **positifs** sous resserrement armé. Et puisque la mesure du § 2.6
  établit que cette suite n'était lancée par aucun job, le vol la **câble** —
  sans quoi sept cas de plus dans un fichier inexécuté satisferaient le point 4
  dans la forme et le videraient dans le fond. *(points 4 et mika#2578)*
- **AC5 — les deux surfaces documentaires** sont écrites : la section d'egress de
  `docs/operator/environment-variables.md` et la note de solution
  `une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`.
  *(point 5)*
- **AC6 — la part du périmètre que ce vol ferme est nommée, et celle qu'il ne
  ferme pas l'est aussi.** La dimension dépôt est fermée ; la dimension branche
  est renvoyée à un vol 3 nommé par `Tracked in:`, avec la mesure qui justifie le
  découpage. P4 reste une limite, pas une dette.
- **AC7 — aucune régression des deux moitiés déjà livrées.** `git-upload-pack` et
  `info/refs` sur `github.com` sont strictement inchangés, et la coupure
  `api.github.com` de mika#2724 l'est aussi.

---

## 9. Risques et arbitrages

| # | risque | traitement |
|---|---|---|
| R1 | **Un pilote aurait un besoin légitime de pousser dans un second dépôt**, et le resserrement arrêterait la boucle. | Écarté par mesure, pas par confiance : le § 2.3 établit que bwrap ne monte **qu'un** `.git` parent, donc le pilote n'a qu'un `origin` possible. Si la mesure est contredite en vol, `MIKA_PILOT_EGRESS_PUSH_SCOPE=0` rend mika#2736 en une variable, sans redéploiement. |
| R2 | **Le resserrement 403 le POST de sonde de 4 octets**, et refuse donc tout push assez gros pour être streamé — le seul mode de défaillance capable d'arrêter la boucle. | Structurellement hors d'atteinte : le resserrement ne juge que le **dépôt**, qui vient de l'URL et non du corps, donc il ne distingue pas une sonde d'un push. Épinglé par le cas 19 malgré tout, parce que « structurellement impossible » est une affirmation du plan et non une propriété du code. |
| R3 | **Le registre grossit sans être purgé** la moitié du temps, et une entrée périmée **élargit** l'union — relâchant silencieusement le resserrement. | Le défaut (`_last_purge` global) est réel dans l'existant et corrigé au commit 1, avec son cas de test. Il est traité **avant** le reste du vol, précisément parce que le second registre est ce qui le rend exploitable. |
| R4 | **La dégradation devient le régime permanent** — le staging fail-open en silence, et plus rien ne resserre. | C'est la défaillance silencieuse de ce vol, et elle a une sonde dédiée : la ligne `scope-absent`, documentée comme sonde d'opérateur (§ 3.6) et attestée en vol par `push_scope_degraded`. Le fail-open du lanceur reste le bon choix — un dispatch ne meurt pas sur un fichier de bookkeeping — mais il n'est plus muet. |
| R5 | **Le périmètre serait falsifiable depuis la sandbox**, auquel cas il n'atteste rien. | Écarté structurellement : le registre vit sous `~/.mika`, qui n'est pas parmi les binds (§ 2.3), donc la sandbox ne peut ni le lire ni l'écrire. C'est l'asymétrie dont mika#2724 tire sa sentinelle, et c'est aussi la raison pour laquelle le canal **en-tête** reste écarté — mesuré et tranché au § 2.5 du plan du vol 1. |
| R6 | **L'union autorise encore la branche d'un autre dispatch concurrent**, donc la première puce du ticket reste ouverte après ce vol. | Assumé et nommé : c'est le vol 3, et le § 2.2 mesure pourquoi il n'est pas une phase de celui-ci. Le corps de PR le dira explicitement plutôt que de laisser lire « P3 fermée ». |
| R7 | **Le canari changerait de sens** si quelqu'un y exportait `REPO`. | Le § 2.4 écrit la dépendance au lieu de l'hériter, et `push_scope_degraded` la rend assertée : elle rougit le jour où le canari acquiert un périmètre, au lieu de laisser `push_bound_nominal` rougir sans explication. |

---

## 10. Hors périmètre, délibérément

- **P4, le force-push** sur la branche du spawn (incident fondateur mika#1318).
  Hors d'atteinte côté proxy pour une raison de protocole : `receive-pack` ne
  porte aucun drapeau de force, et un push ordinaire a la même forme sur le fil.
  Mesuré et écrit au § 2.4 du plan du vol 1 et au § *P4* de la note de solution.
- **La dimension branche du périmètre**, et le cycle de vie d'entrée dont elle
  est indissociable : vol 3, nommé par la ligne `Tracked in:` du § 5.
- **Les quatre règles de mika#2736 elles-mêmes**, et le choix d'une borne plutôt
  que d'un justificatif sur `github.com` : tranchés, avec la mesure, au § 2.5 et
  § 3.1 du plan du vol 1.
- **La moitié `api.github.com`**, livrée par mika#2724 — ce vol ne la touche que
  pour asserter qu'elle est intacte.
- **L'attribution d'une requête à un spawn** (plutôt que l'union), qui
  demanderait de porter une identité du client jusqu'à l'addon à travers une
  socket unique partagée (§ 2.2) : un ticket en soi, et aucune mesure de ce
  grooming ne dit qu'il vaut son coût.

---

## 11. Référence

- Ticket : senara-solutions/mika#2751
- Vol 1 : senara-solutions/mika#2736 — la borne statique
- Plan du vol 1 : `docs/plans/2026-10-17-001-fix-2736-egress-pilote-moitie-git-push-plan.md`
  (§ 2.2 ce que le proxy voit, § 2.4 P4, § 2.5 les canaux écartés, § 3.1 la forme
  retenue, § 4 la frontière hôte/sandbox des sites de push, § 5 l'estimation de
  ce vol que le § 2.2 ci-dessus révise)
- L'asymétrie hôte/sandbox dont le périmètre tire son autorité :
  senara-solutions/mika#2724, senara-solutions/mika#2716
- Bornes existantes du gitdir (un système de fichiers, pas une URL) :
  senara-solutions/mika#2141
- Incident fondateur de P4 : senara-solutions/mika#1318
- Un interrupteur que son second lecteur n'honore pas est décoratif :
  senara-solutions/mika#2165
- Deux états qui rendent les mêmes octets : senara-solutions/mika#2205
- Une garde que le pipeline ne lance pas ne refuse rien :
  senara-solutions/mika#2578
- Une exemption sans violation est une surface d'attaque :
  `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`
- Note de solution à étendre :
  `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`

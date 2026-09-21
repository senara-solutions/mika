# mika#2051 — La moitié code est livrée ; ce qui reste est un instrument illisible et une mesure hors dépôt

**Ticket :** mika issue#2051
**Type :** fix (observabilité opérateur + corrélation ; pas de cause inventée)
**Date :** 2026-09-21

---

## Problème

### Ce que le ticket demande

Deux proxies d'egress (pids `2383949`, `2387253`) sont morts entre `exec` et
`bind()` le 2026-08-29, sans écrire une ligne, alors que `dispatch-lib` capture
leur stderr. Le ticket isole la question que mika#2041 a refusé de combler par
une hypothèse — **qui les a tués** — et énonce sa propre condition de
résolution :

> « la prochaine occurrence laissera une trace exploitable. C'est la condition
> qui manquait pour enquêter au lieu de subir. **Ce ticket est là pour recevoir
> cette trace.** »

### La rectification qui change le périmètre, et c'est le premier livrable

**La moitié code de mika#2051 est déjà livrée et mergée.** `ed8d0e2b` /
PR #2086, *« fix(egress): name a pre-bind proxy death instead of dying
silent (mika#2051) »*, 2026-08-30, 112 insertions sur
`scripts/mika-pilot-egress-proxy` et son test. Elle est ancêtre de `HEAD`.

Ce qu'elle a posé, et qui tourne depuis trois semaines :

- `pilot_egress_startup.begin pid=<pid> binding <path>` — première ligne émise,
  **avant** `bind()`, horodatée via `_log` (mika#2030) ;
- des handlers SIGTERM/SIGINT armés **avant** `bind()`, qui nomment un signal
  pré-bind (`pilot_egress_startup.signalled <SIG> before bind …`) et sortent
  en `3` au lieu de mourir muets — les handlers gracieux post-bind les
  remplacent ensuite (`add_signal_handler` est last-writer-wins) ;
- un seam de test (`_MIKA_EGRESS_PREBIND_TEST_BARRIER`) et deux tests de
  régression : `.begin` est la **première** ligne et porte le pid
  (`test_startup_emits_a_begin_breadcrumb_before_bind`, qui lit
  `proc.stderr.readline()`), et un SIGTERM garé dans la fenêtre pré-bind nomme
  sa cause, sort en `3`, et ne laisse pas de socket
  (`test_pre_bind_signal_names_its_cause`).

Le message de commit nomme lui-même ce qui reste ouvert, et il faut le citer
plutôt que le paraphraser :

> « Who SENT the signal (guardrail / timeout / process-group teardown / OOM)
> **is operational and needs host-side evidence not in this repo**; this change
> makes the next occurrence say which of those it was. »

**Le risque principal de ce ticket est donc qu'on réimplémente #2086, ou qu'on
invente la cause que le ticket a explicitement refusé d'inventer.** Ce plan
refuse les deux (D1, D5).

### Ce qui reste réellement ouvert, et qui est en dépôt

Trois questions, dont deux seulement sont fermables ici.

**(Q1) La trace est-elle arrivée ?** Mesure, pas code. **Structurellement
indisponible depuis une session pilote** : mesuré dans ce worktree,
`/var/log/` ne contient que `claude-pilot`, `/var/log/mika/` n'existe pas et
`/tmp/mika-pilot-egress-proxy.log` non plus, parce que le bac à sable bwrap ne
monte pas le journal hôte (classe mika#2165). **L'absence de ces fichiers ici ne
prouve rien sur l'hôte** et ne doit jamais être lue comme « aucune récurrence ».
C'est un geste opérateur, et le plan le prescrit comme tel (U3).

**(Q2) L'instrument livré est-il lisible par quelqu'un qui ne l'a pas écrit ?
Non — et c'est le vrai défaut résiduel en dépôt.** `pilot_egress_startup`
apparaît dans **zéro fichier markdown** du dépôt (`grep -rn pilot_egress_startup
--include="*.md" .` → vide) ; il ne vit que dans le script et son test. Or la
surface opérateur de cette panne est le Signal S de `CLAUDE.md`, dont la
**Remedy** dit exactement, et s'arrête là :

> « Inspect `${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log`
> to learn why the proxy did not take — and if that file is absent, read
> `/tmp/mika-pilot-egress-proxy.log` … »

On envoie l'opérateur dans un fichier **sans lui dire quoi y chercher**. Toute
la valeur diagnostique de #2086 est une table de signatures à quatre branches
qui n'existe que dans un commentaire Python. C'est la classe que mika#2050 a
déjà dû fermer deux fois sur ce même Signal S et sur le Signal Q : *une surface
opérateur qui ne porte pas ce qu'il faut pour lire l'instrument*. Une trace qui
arrive et que personne ne sait lire est la même chose qu'une trace absente, avec
une ligne de journal en plus.

**(Q3) Peut-on joindre « ce dispatch est tombé en fs-only » à « ce proxy est
mort » sans corréler à la main ?** Non. Les deux moitiés vivent dans **deux
fichiers différents** : la ligne du lanceur part sur le stderr du dispatch
(`$_PILOT_LOG_DIR/<task-id>.stderr`, Signal S), la ligne du proxy part dans
`/var/log/mika/pilot-egress-proxy.log`. Et l'asymétrie est exactement à
l'envers de ce qu'il faudrait — `dispatch-lib.sh` :

```
531:  echo "dispatch-lib: pilot_egress_guard.unreachable pilot-egress-proxy failed to bind … — falling back to fs-only" >&2
534:  echo "dispatch-lib: pilot-egress-proxy launched (pid $!, log $log_file)" >&2
```

**Le chemin de succès porte le pid ; le chemin d'échec — le seul que ce ticket
existe pour diagnostiquer — n'en porte aucun.** La jointure se fait donc par
horodatage entre deux fichiers, ce qui est précisément la friction qui a rendu
le diagnostic du 2026-08-29 coûteux. Le proxy, lui, porte son pid dès sa
première ligne (`pilot_egress_startup.begin pid=…`) : la clé de jointure existe
des deux côtés, elle n'est simplement pas imprimée du côté échec.

### Ce qui a été vérifié et n'est PAS un défaut

Écrit pour que la prochaine lecture ne le re-suspecte pas.

- **`$!` à la ligne 534 est fiable.** Il est lu ~20 lignes et une boucle après
  le `nohup … &`, mais `_pilot_egress_sock_connectable` lance `python3` en
  **avant-plan** (`dispatch-lib.sh:473-485`) et `sleep` aussi : aucun
  arrière-plan ne s'interpose, `$!` tient encore le proxy. Ce n'est pas un bug.
  U2 le capture tout de même immédiatement, parce qu'il faut la valeur **avant**
  la ligne 531 — la robustesse est un effet de bord, pas le motif.
- **`.begin` est bien la première ligne**, et c'est **déjà verrouillé** par
  `test_startup_emits_a_begin_breadcrumb_before_bind`, qui assert sur
  `proc.stderr.readline()`. La branche « aucune ligne `.begin` du tout = mort
  avant Python » de la table de signatures repose donc sur un invariant testé,
  pas sur une convention. **Aucun test à ajouter de ce côté** — c'était une
  suspicion de ce plan, levée par lecture.
- **Les hypothèses que le ticket réfute restent réfutées** : le déliement de
  socket éventée (`:1297-1310`) tourne bien, `stat` est importé (`:66`).

---

## Requirements

- **R1** — Le Signal S de `CLAUDE.md` porte la table de signatures à quatre
  branches, de sorte qu'un opérateur qui trouve un `falling back to fs-only`
  sache quoi chercher dans le journal du proxy et ce que chaque forme veut dire.
- **R2** — La ligne d'échec du lanceur nomme le pid du proxy qu'elle vient de
  lancer, pour que la jointure avec `pilot_egress_startup.begin pid=…` soit
  exacte plutôt que temporelle.
- **R3** — La mesure Q1 est prescrite comme geste **hôte**, avec ses commandes
  exactes et ses haltes, et consignée dans un artefact durable — parce qu'elle
  ne peut pas être prise depuis une session dispatchée et qu'elle est
  conditionnée à une récurrence.
- **R4** — Aucune cause n'est affirmée, aucune n'est écartée sans mesure. Le
  plan ne referme pas Q3-du-ticket (« qui a envoyé le signal ») et dit
  pourquoi.
- **R5** — Aucune régression du comportement d'exécution : ni le proxy, ni la
  décision de repli fs-only, ni le contrat de sortie ne changent.

---

## Décisions

- **D1 — Ne rien réimplémenter de #2086, et le dire en tête de plan.** La
  tentation structurelle de ce ticket est de relire le corps (« deux proxies
  sont morts, rien dans le log ») et de livrer l'instrumentation… qui est déjà
  là depuis trois semaines. Le corps du ticket a été écrit **avant** le
  correctif et n'a pas été amendé depuis. C'est la classe mika#2340 transposée à
  un ticket : *établir l'état déployé avant de toucher au code.* Le premier
  livrable de ce plan est donc la rectification elle-même.

- **D2 — Le défaut résiduel est une surface opérateur, pas un manque de
  signal.** Le signal existe et est testé. Ce qui manque est sa **lisibilité**
  au seul endroit où un opérateur en incident va regarder. Choisir `CLAUDE.md`
  § Signal S plutôt qu'un nouveau document : c'est la surface que la maison a
  déjà désignée pour cette panne, elle porte déjà la Remedy qui envoie vers le
  fichier, et mika#2050 a corrigé **deux fois** des commandes de cette même
  section plutôt que d'en créer une nouvelle ailleurs. Un second lieu serait un
  second lieu à maintenir, et le premier continuerait de répondre à côté.

- **D3 — La table est à quatre branches, pas trois.** Le commentaire du code en
  nomme trois (begin+signalled ; begin sans signalled ni listening ; sain). La
  quatrième — **aucune ligne `.begin`** — est la plus importante à écrire pour
  un opérateur, parce que c'est celle qui dit « le processus est mort avant que
  Python ne tourne » (échec d'`exec`, interpréteur, dépendance) et qu'elle est
  la seule que l'on lit par une **absence**. La laisser implicite, c'est laisser
  l'opérateur conclure « le journal ne dit rien » là où le journal dit quelque
  chose de précis.

- **D4 — Le pid sur la ligne d'échec, et pas un identifiant de corrélation
  nouveau.** Le réflexe serait d'inventer un `egress_correlation_id` posé des
  deux côtés. Refusé : la clé existe déjà et elle est imprimée du côté proxy
  depuis #2086. Ajouter un identifiant reviendrait à créer un second vocabulaire
  pour joindre ce que le pid joint déjà, et à devoir le propager à travers
  `nohup`. Une variable locale capturée juste après le lancement suffit.

- **D5 — Ne pas répondre à « qui a envoyé le signal ».** Le ticket refuse
  explicitement d'inventer (« Je n'ai pas d'évidence pour départager, et je n'en
  invente pas »), le commit de #2086 le refuse aussi, et la mesure est hors
  dépôt. Ce plan hérite de ce refus. Il livre de quoi **attribuer** à la
  prochaine occurrence, pas une cause.

- **D6 — Aucune garde, aucun détecteur de récurrence.** On pourrait vouloir un
  compteur ou une garde qui refuse le dispatch quand le proxy ne prend pas.
  Refusé, sur mesure : le repli fs-only est **volontairement fail-open**
  (`dispatch-lib.sh:487-490`, « fail-open on missing binary … so the pilot still
  functions during the deploy window »), et transformer un repli documenté en
  refus de dispatch est une décision de politique de containment, d'un tout
  autre rayon d'explosion, qui n'est pas dans ce ticket. Signal S porte déjà le
  contrôle positif (`pilot-egress-proxy launched`) qui distingue « zéro repli
  parce que tout va bien » de « zéro repli parce que rien ne tourne ».

- **D7 — L'artefact durable est un `docs/solutions/`, pas un commentaire.** La
  mesure Q1 est conditionnée à une récurrence qui peut ne jamais venir. Un plan
  se lit une fois ; un `docs/solutions/` est ce que la maison relit en incident,
  et c'est déjà là que mika#2041 a déposé sa leçon
  (`best-practices/a-guard-must-observe-not-assert-2026-08-29.md`).

---

## Scope Boundaries

**Dans le périmètre :**
- `CLAUDE.md` § Signal S — Remedy enrichie de la table de signatures.
- `skills/bundled/_shared/dispatch-lib.sh` — pid sur la ligne d'échec.
- `skills/bundled/_shared/test-dispatch-lib.sh` — assertion sur ce pid.
- `docs/solutions/` — une entrée portant la procédure de lecture et l'état de
  la question.

**Hors périmètre, délibérément :**
- **`scripts/mika-pilot-egress-proxy`** — la moitié code est livrée et testée.
  Ce plan n'y touche pas. Toute modification y serait une réimplémentation de
  #2086 (D1).
- **La cause du signal** (guardrail, timeout, teardown de groupe de processus,
  OOM) — D5.
- **Transformer le repli fs-only en refus de dispatch** — D6.
- **La politique de containment Phase 2b**, l'allowlist, le shim TCP.
- **Le sink du Signal M** (`pilot_push_guard`, qui n'atterrit dans aucun
  fichier) — défaut réel, voisin, déjà nommé dans `CLAUDE.md` comme ticket de
  suivi ; pas celui-ci.

---

## Implementation Units

### U1 — Signal S porte la table de signatures (R1)

`CLAUDE.md`, § *Signal S*, sous-puce **Remedy**. Enrichir sans rien retirer :
les deux chemins de fichier déjà nommés restent (leur raison d'être — « looking
for only the first is how an operator concludes "no log, so nothing ran" » — est
intacte). Ajouter la lecture, avec le grep et les quatre branches :

```bash
grep -E 'pilot_egress_startup|host-unix listening on' \
  "${MIKA_PILOT_EGRESS_LOG_DIR:-/var/log/mika}/pilot-egress-proxy.log"
```

| Ce qu'on lit pour un lancement | Lecture |
|---|---|
| `.begin` **puis** `host-unix listening on` | sain — ce proxy a bindé |
| `.begin`, **pas** de `.signalled`, **pas** de `listening` | mort dans la fenêtre pré-bind par un signal **non rattrapable** — SIGKILL ou OOM-kill |
| `.begin` **puis** `.signalled <SIG>` (sortie `3`) | un SIGTERM/SIGINT a atterri pendant le démarrage — le signal est nommé |
| **aucune** ligne `.begin` pour ce lancement | mort **avant** que Python ne tourne — échec d'`exec`, interpréteur, dépendance manquante |

Deux phrases à écrire avec le tableau, parce que ce sont elles qui empêchent la
mauvaise conclusion :

- l'absence de `.begin` **pour un lancement donné** est une information, pas un
  silence — à ne pas confondre avec un journal absent, qui est le cas
  « binaire jamais déployé » que la Remedy traite déjà ;
- `.begin` est la **première** ligne du processus par contrat testé, donc la
  quatrième branche est lisible ; si un jour elle cesse de l'être, c'est
  `test_startup_emits_a_begin_breadcrumb_before_bind` qui rougit.

Et la jointure, qui devient exacte avec U2 : le pid de la ligne
`pilot_egress_guard.unreachable` du `.stderr` du dispatch se retrouve tel quel
dans le `pid=` du `.begin` du journal du proxy.

### U2 — La ligne d'échec nomme le pid qu'elle a lancé (R2)

`skills/bundled/_shared/dispatch-lib.sh`, `_ensure_pilot_egress_proxy` :

- capturer `local proxy_pid=$!` **immédiatement** après le `nohup … &` /
  `disown` (lignes 512-514) ;
- ligne 531 (`pilot_egress_guard.unreachable`) : ajouter `(pid <proxy_pid>)` ;
- ligne 534 (succès) : lire `$proxy_pid` au lieu de `$!` — même valeur
  aujourd'hui (vérifié : aucun arrière-plan ne s'interpose), mais la lecture ne
  dépend plus d'un invariant à distance.

**Le point d'insertion est contraint, et c'est lui qui protège les greps.** Le
pid s'insère **avant** le tiret cadratin, entre `within 3s` et
`— falling back to fs-only` :

```
dispatch-lib: pilot_egress_guard.unreachable pilot-egress-proxy failed to bind <sock> within 3s (pid <proxy_pid>) — falling back to fs-only
```

Le texte existant n'est pas réécrit : le token `pilot_egress_guard.unreachable`
et la sous-chaîne `falling back to fs-only` sont **tous deux** des prédicats
publiés du Signal S, et la ligne reste ancrée à `^dispatch-lib: `. Insérer le
pid **après** le tiret couperait la sous-chaîne publiée en deux et rendrait
muet le prédicat qui couvre *toute* la population de repli — c'est-à-dire qu'on
casserait l'instrument du Signal S en croyant l'améliorer. Le pid s'ajoute en
amont du tiret ; rien ne bouge de ce sur quoi les greps mordent (V7).

### U3 — L'artefact durable et la procédure de mesure (R3, R4)

`docs/solutions/` (famille `best-practices/`, voisine de l'entrée mika#2041) —
une entrée courte qui porte :

1. **L'état de la question au 2026-09-21** : instrumentation livrée (#2086),
   cause inconnue, aucune mesure de récurrence prise.
2. **La procédure de mesure, comme geste hôte**, avec la raison pour laquelle
   elle est hôte : un pilote dispatché ne voit pas `/var/log/mika/`, et son
   absence dans le bac à sable n'est **pas** un résultat.
3. **La table de signatures** (source de vérité partagée avec U1).
4. **Les haltes** (ci-dessous, § Fire-Disposition et § Suivi).

---

## Verification Contract

- **V1** — `grep -n "pilot_egress_startup" CLAUDE.md` retourne au moins une
  ligne. **Le prédicat porte sur `CLAUDE.md`, jamais sur `*.md` en général** :
  ce plan contient lui-même le token une dizaine de fois, donc un
  `grep -rn … --include="*.md" .` serait satisfait par le plan seul et
  n'attesterait rien de la surface opérateur. Contrôle négatif : avant ce
  travail, `CLAUDE.md` en portait **zéro**.
- **V2** — `make test-dispatch-lib` passe, assertion U2 comprise.
- **V3** — Nouvelle assertion dans `test-dispatch-lib.sh` : sur un chemin de
  socket qui ne bindera jamais, la ligne `pilot_egress_guard.unreachable`
  contient un pid numérique **et** garde son ancre `^dispatch-lib: ` et sa
  sous-chaîne `falling back to fs-only`.
- **V4 — contrôle négatif de V3** : sans le changement U2, l'assertion rougit.
  Sans lui, V3 ne distingue pas « le pid est imprimé » de « l'assertion est
  triviale ».
- **V5** — `scripts/test-pilot-egress-proxy-status.py` passe **inchangé** :
  aucune ligne de `scripts/mika-pilot-egress-proxy` n'est touchée (contrôle de
  D1 / hors-périmètre).
- **V6** — `git diff --stat` ne liste pas `scripts/mika-pilot-egress-proxy`.
- **V7** — Les greps publiés du Signal S mordent toujours sur la ligne modifiée
  (vérifié en exécutant les deux prédicats publiés contre la sortie produite en
  V3).

---

## Definition of Done

- Signal S porte la table de signatures et la clé de jointure ; le grep V1 n'est
  plus vide.
- La ligne d'échec du lanceur porte le pid ; V3 et son contrôle négatif V4
  passent.
- L'entrée `docs/solutions/` existe et porte l'état de la question, la
  procédure hôte et ses haltes.
- Le proxy n'est pas modifié (V5, V6).
- Le corps du ticket est amendé ou commenté par l'opérateur pour dire que la
  moitié code est livrée par #2086 — **geste opérateur, hors de ce plan**
  (le pilote de grooming n'écrit pas sur le ticket).

---

## Acceptance criteria

Dérivées du plan : le corps du ticket ne porte pas de section
`## Acceptance criteria`.

- **AC1** — `grep -n "pilot_egress_startup" CLAUDE.md` retourne au moins une
  ligne. (Avant ce travail : zéro. Le prédicat est ancré sur `CLAUDE.md` et non
  sur `*.md` — voir V1 pour pourquoi une version élargie s'auto-satisferait.)
- **AC2** — Le § Signal S de `CLAUDE.md` nomme les quatre branches de la table,
  y compris la branche « aucune ligne `.begin` », et nomme le pid comme clé de
  jointure entre le `.stderr` du dispatch et le journal du proxy.
- **AC3** — Sur un échec de bind, la ligne `pilot_egress_guard.unreachable`
  émise par `_ensure_pilot_egress_proxy` contient un pid numérique, tout en
  conservant l'ancre `^dispatch-lib: ` et la sous-chaîne
  `falling back to fs-only`.
- **AC4** — `make test-dispatch-lib` passe ; l'assertion AC3 rougit si le
  changement est retiré.
- **AC5** — `scripts/mika-pilot-egress-proxy` est **inchangé** par ce travail,
  et `scripts/test-pilot-egress-proxy-status.py` passe sans modification.
- **AC6** — Une entrée `docs/solutions/` porte : l'état de la question au
  2026-09-21, la procédure de mesure hôte, la raison pour laquelle elle n'est
  pas prenable depuis un pilote, et les haltes.
- **AC7** — Aucun livrable n'affirme une cause de la mort des deux proxies du
  2026-08-29, et aucun ne change le comportement d'exécution du repli fs-only.

---

## Fire-Disposition

**Option (a) — aucune exception nécessaire, et c'est vérifié plutôt
qu'affirmé.**

Ce plan livre **un** détecteur : l'assertion V3/AC3 dans
`skills/bundled/_shared/test-dispatch-lib.sh`, dont le chemin de succès est
« la ligne d'échec porte un pid ».

Il ne peut pas firer sur des données existantes : il n'inspecte aucun corpus,
aucun historique, aucun fichier du dépôt. Il exécute
`_ensure_pilot_egress_proxy` contre un chemin de socket fabriqué qui ne bindera
jamais, et lit la ligne produite dans la foulée. Sa population est donc
**créée par le test lui-même**, à chaque exécution — il n'y a pas de violation
préexistante possible, donc **aucune exception à allowlister**. L'allowlist est
vide et doit le rester ; si ce détecteur rougit un jour, c'est que la ligne a
cessé de porter le pid, et la résolution est de le rendre, jamais de
l'exempter.

U1 et U3 sont documentaires (une section de `CLAUDE.md`, une entrée
`docs/solutions/`) : aucune n'a pour fonction primaire de signaler une
violation, aucune ne peut firer.

**Le contrôle négatif V4 est ce qui rend cette disposition honnête** : sans lui,
une assertion triviale passerait pour un détecteur armé — exactement la panne
que mika#2272 a nommée (« zéro était l'absence de mesure, pas la présence de
prudence »).

---

## Suivi (hors périmètre, nommé)

- **La cause de la mort des deux proxies du 2026-08-29** reste **ouverte** et
  ne se ferme pas en dépôt (D5). Elle se ferme sur une récurrence, attribuée par
  la table d'U1. **Halte explicite : si aucune récurrence n'est mesurée, la
  conclusion est « instrumenté, sans récurrence » — pas « cause identifiée ».**
  Le ticket peut alors être fermé sur cette base, ce qui est un résultat et non
  un abandon.
- **Halte de mesure** — si la mesure hôte U3 montre des `.begin` **sans**
  `listening` alors que le Signal S ne rapporte **aucun**
  `falling back to fs-only`, ne pas élargir la table : les deux instruments
  disent des choses contradictoires et c'est **le sink** qu'il faut établir
  d'abord (`PILOT_LOG_DIR` / `MIKA_PILOT_LOG_DIR`, halte 1 déjà écrite au
  Signal S).
- **Halte de lecture** — `grep` vide dans le journal du proxy **et** journal
  absent sont deux états différents : le second est le cas « binaire jamais
  déployé » (classe mika#2340), à établir avant toute conclusion sur l'egress.
- **Limite héritée, non refermée ici** : le chemin `_launch_revise_pilot`
  redirige son stderr vers un `mktemp` qu'il supprime, donc le contrôle Signal S
  ne couvre pas la voie revise. Déjà écrit dans `CLAUDE.md` ; ce plan ne le
  change pas.
- **Signal M** (`pilot_push_guard`, qui n'atterrit dans aucun fichier) — défaut
  réel de la même famille, déjà nommé comme ticket de suivi dans `CLAUDE.md`.
  Pas rouvert ici.
- **Amender le corps de mika#2051** pour dire que la moitié code est livrée par
  #2086 : geste opérateur. Sans lui, la prochaine lecture du ticket repart sur
  la trajectoire que D1 refuse.

---

## Références

- `ed8d0e2b` — PR #2086, *fix(egress): name a pre-bind proxy death instead of
  dying silent (mika#2051)*, 2026-08-30 : la moitié code, déjà livrée.
- `scripts/mika-pilot-egress-proxy:1242-1292` — fenêtre pré-bind instrumentée :
  handler précoce `:1269`, `.begin` `:1284`, seam de test `:1292` ; le
  `host-unix listening on` qui clôt la fenêtre est `:1325`.
- `scripts/mika-pilot-egress-proxy:1299-1306` — le déliement de socket éventée
  que le ticket a déjà réfuté comme cause (`stat.S_ISSOCK` puis `unlink`).
- `scripts/test-pilot-egress-proxy-status.py:1450`, `:1467` — les deux tests de
  régression de #2086.
- `skills/bundled/_shared/dispatch-lib.sh:491-536` — `_ensure_pilot_egress_proxy`,
  et l'asymétrie pid succès/échec aux lignes 531 et 534.
- `skills/bundled/_shared/dispatch-lib.sh:473-485` —
  `_pilot_egress_sock_connectable`, avant-plan : pourquoi `$!` tient encore.
- `CLAUDE.md` § *Signal S* — la surface opérateur, et sa Remedy à enrichir.
- mika#2041 — la garde qui rendait cette classe muette (corrigée) ;
  `docs/solutions/best-practices/a-guard-must-observe-not-assert-2026-08-29.md`.
- mika#2050 — les deux corrections de sink sur les Signaux Q et S : précédent
  de forme pour U1.
- mika#2165 — le bac à sable ne monte pas le journal : pourquoi Q1 est un geste
  hôte.

---

## Revision history

- **v1 (2026-09-21)** — Plan initial. Rectification du périmètre : la moitié
  code de mika#2051 est livrée par #2086 depuis le 2026-08-30 ; le résiduel en
  dépôt est la lisibilité opérateur de l'instrument (Signal S sans la table de
  signatures, `pilot_egress_startup` absent de tout markdown) et la jointure
  pid absente du chemin d'échec du lanceur. La cause reste hors dépôt et non
  affirmée.
- **v2 (2026-09-21)** — Re-groom. Les sept assertions portantes de v1 ont été
  re-confrontées à `HEAD` (`3b177df3`) et tiennent toutes : `ed8d0e2b` ancêtre
  et titre exact ; `pilot_egress_startup` à **zéro** dans `CLAUDE.md` et présent
  dans **un seul** markdown — ce plan — ce qui **confirme par la mesure** le
  choix d'ancrer V1/AC1 sur `CLAUDE.md` plutôt que sur `*.md` ; asymétrie pid
  aux lignes 531/534 ; aucun arrière-plan entre `nohup` (512) et la garde, donc
  `$!` tient toujours ; `make test-dispatch-lib` présent (`Makefile:158`) ; les
  deux tests de #2086 présents (`:1450`, `:1467`). **Aucune décision, aucune AC,
  aucun périmètre n'a changé.** Trois resserrages de précision seulement :
  (a) U2 nomme désormais le **point d'insertion** du pid — avant le tiret
  cadratin — parce que l'insérer après couperait la sous-chaîne publiée
  `falling back to fs-only` et casserait le prédicat qui couvre toute la
  population de repli ; (b) les références au proxy passent d'une plage
  approximative à des lignes vérifiées (1269 / 1284 / 1292 / 1325, plus le
  déliement 1299-1306) ; (c) cette entrée.

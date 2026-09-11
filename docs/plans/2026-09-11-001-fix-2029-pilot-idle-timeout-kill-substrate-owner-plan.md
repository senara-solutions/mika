---
issue: 2029
type: fix
title: "loop-substrate : la classe « pilote tué par idle_timeout sans avoir produit » a un propriétaire — côté mika, trois défauts de substrat mesurés ; côté pilote, la cause est cpp#177"
branch: fix/2029/loop-substrate-the-pilot-stall-that
status: groomed
siblings:
  - senara-solutions/claude-pilot#177  # la cause côté pilote (fenêtre d'attente-modèle refermée par content_block_stop)
---

# Plan — mika#2029 : le substrat `mika` de la classe « tué par `idle_timeout`, 0 travail »

## Invariant (une phrase)

**Quand un pilote est tué par un garde-fou avant d'avoir produit, `mika` (1) le compte par un motif
nommé sans grep de log, (2) ne raconte pas dans son code une cause que la mesure a réfutée, et
(3) ne laisse pas ses démons d'egress exécuter du code plus vieux que le fichier installé — parce
que c'est ce troisième point qui a rendu le journal aveugle deux fois, et le journal est l'unique
instrument qui reste pour trancher le résidu.**

Ce ticket a été investigué sur seize commentaires et six lignées éliminées (auth/401, surface
d'outils, relais de permission, transport du tunnel, prompt système, skill d'entrée). Le grooming
du 2026-09-11 a **re-mesuré** (logs horodatés depuis cpp#168, `~/.mika/data/mika.db`, `ss -xlp`)
et a localisé la cause côté pilote au `file:line` — elle est fichée
**senara-solutions/claude-pilot#177** avec sa trace verbatim et n'est **pas** dans ce plan. Ce
plan porte ce qui vit dans `mika`.

## Ce que la mesure dit aujourd'hui (2026-09-11)

- **Classe vivante dominante** : 7 sessions tuées `idle_timeout` « before it produced any work »
  du 09-04 au 09-10, à **4–19 tours** chacune (`b8d3b90e`, `4765b744`, `e7fc0e2b`, `6b5481f6`,
  `a0fcb92d`, `f9cc3d35`, `8a0b6b89`) — une par jour. Mécanisme : cpp#177 (le `content_block_stop`
  du bloc `tool_use` courant referme la fenêtre d'attente-modèle que son propre tool result vient
  d'ouvrir → budget 300 s au lieu de 900 s, pendant un TTFT de ~355 s).
- **Le TTFT est bimodal** (6 sessions du 09-10, 371 appels) : soit ≤ 29 s, soit 300–389 s, **zéro
  valeur entre les deux**. Un délai fixe, pas une génération lente. Son origine (serveur, ou retry
  client après timeout) n'est **pas tranchée** : c'est le résidu de ce ticket, et il ne peut être
  tranché que par le journal d'egress horodaté — voir Décision 3 et « Résidu dormant ».
- **Classe « mort-née »** (`[init]` reçu, `0 content stream events`) : n=3, un seul ticket (#2140),
  le 09-03, **aucune récidive depuis** ; le premier appel est rapide (2–3 s) sur les 6 sessions où il
  est mesurable. Distincte, non expliquée, dormante — condition de réveil ci-dessous.
- **Le journal d'egress est aveugle depuis douze jours.** PR#2085 (mika#2030, horodatage par ligne)
  est mergée le 08-30 et installée (`~/.local/bin/mika-pilot-egress-proxy`, mtime 09-10 20:15,
  `_log()` à `:90`). Le processus qui écoute sur `/tmp/mika-pilot-egress.sock` est le pid 2388646,
  démarré le **29 août 06:10:50** (`ss -xlp`, `ps -o lstart=`). Zéro ligne horodatée dans
  `/var/log/mika/pilot-egress-proxy.log`. **n=2** : la même forme a été mesurée le 08-29 04:06Z
  (pid 21188 du 28 août 17:02 vs fichier du 29 août 02:23, commentaire de ce ticket), remédiée à la
  main par un `kill` opérateur — un remède manuel dont la demi-vie est le prochain `make deploy`.
- **Les 8 callbacks de sessions tuées depuis le 09-04 ne portent ni `PR:` ni `NO_PR:`** (requête
  `tasks` jointe sur `parent_task_id`). Le parent reçoit le motif générique
  `callback_delivered_without_pr_url` (245 en base) au lieu d'un motif nommé — le contrat de sortie
  totale de mika#2121 (`dispatch-lib.sh:1288`) est violé sur ce chemin.

## Ce que le code dit (vérifié sur `main` @ `e0f0f6fd`, `file:line`)

- `skills/bundled/_shared/dispatch-lib.sh`
  - `:1036-1037` — commentaire au-dessus de `_run_pilot_sandboxed` : « → Anthropic 401 / SDK
    stall → guardrail idle_timeout 300s → pilot dies at Turns:1 ». Vrai pour l'incident du
    2026-08-05 qu'il documente (appels internes du CLI tombant en CONNECT avec la clé placeholder) ;
    **faux comme explication de la classe de ce ticket** (`POST /v1/messages -> 200`, 258 réponses
    du plan de contrôle en 200, commentaires du 08-29).
  - `:2688-2706` — classification `terminated` : `_pilot_left_no_work` → `_terminated_empty` →
    `PILOT_SESSION_TERMINATED=1` + `RESULT=$(_classify_terminated_session)`, et
    `_post_flight_recovery` **sauté**. C'est correct (mika#1772) — mais le site 2 d'émission
    `NO_PR:` vit dans `_post_flight_recovery` (`:3622-3637`), donc ce chemin n'écrit jamais la ligne.
  - `:2991-3044` — `_classify_terminated_session` ; le mode `full` termine par « See the stall
    lineage on mika#1901 and the note above _run_pilot_sandboxed on the Anthropic 401 / SDK-stall
    chain that ends in exactly this shape » (`:3041`). Cette phrase est livrée à mika-dev **à chaque
    kill** ; elle est fausse pour chacun des 7 cas mesurés.
  - `:1288-1331` — contrat de sortie totale (mika#2121 U1) : exactement une ligne `PR:` ou
    `NO_PR: <reason>` par callback ; `_classify_no_pr_reason` (`:1296`) rend un token parmi
    `repo_unset | branch_unset | gh_query_failed | no_pr_on_branch` ; `_set_pr_status_line`
    (`:1329`) remplace toute ligne antérieure, donc l'ajout d'une ligne ne peut pas en produire deux.
  - `:6272-6283` — le push de branche vide est déjà coupé sur `PILOT_SESSION_TERMINATED`
    (mika#1772 R6) ; testé `test-dispatch-lib.sh:3811-3820`. **AC3 de #2035 est déjà honoré.**
  - `:473-536` — `_ensure_pilot_egress_proxy` : sonde de connectabilité (mika#2041) puis relance
    `nohup` ; **aucune comparaison entre l'âge du processus et l'âge du fichier**. Idem
    `_ensure_pilot_helper` (`:383`) pour `mitmdump` et ses deux addons (`:285-296`).
- `Makefile:59` — `deploy: … install restart …` ; `restart` (`:26-30`) ne redémarre que
  `mika-spirit` / `mika-gateway` via `rc-service`. Les deux démons pilotes sont des processus
  utilisateur lancés par dispatch-lib et **aucune cible ne les touche**.
- `crates/mika-agent/src/task_engine/dispatcher.rs:3228-3232` — `parse_no_pr_reason` accepte
  `^NO_PR:\s+([a-z_]+)` ; `engine.rs:3239-3260` écrit `callback_no_pr_<reason>` sur le parent.
  Un token `pilot_terminated_idle_timeout` passe **sans changement Rust**.
- `claude-pilot/src/claude_pilot/guardrails.py:79-85, :445, :480-485` — la cause côté pilote ;
  hors de ce dépôt, cpp#177.

## Décision 1 — le compteur est le motif `NO_PR`, pas un nouvel événement d'audit

Le patron existe déjà et a été conçu exactement pour « un chemin d'échec muet est indiscernable d'un
chemin qui n'a pas été pris » (mika#2121). Une session tuée sans travail écrit
`NO_PR: pilot_terminated_<subtype>` (`subtype` ∈ `idle_timeout | stall_detected | empty_response |
awaiting_tool | awaiting_model | rate_limited`, ou `unknown` quand `.subtype` manque et que le
scrape stderr n'a rien rendu), au **même endroit** que `PILOT_SESSION_TERMINATED=1` (`:2693`), via
`_set_pr_status_line`. Le parent devient `callback_no_pr_pilot_terminated_idle_timeout` ; la
condition se compte par `SELECT count(*) FROM tasks WHERE result = 'callback_no_pr_pilot_terminated_idle_timeout'`
— motif exact, pas `LIKE`, pas de grep de log. **AC5 de #2035.**

Rejeté : un audit event `pilot_guardrail_terminated` émis par l'engine (patron mika#2040
`detect_empty_pilot_transcripts`). Il faudrait parser le prose du callback côté Rust pour une
information que dispatch-lib possède déjà en variable (`$SUBTYPE`), et il doublerait un compteur
que le reaper de parents produit déjà. Le mode `banner` (session tuée **après** avoir produit)
n'écrit rien ici : `_post_flight_recovery` tourne et le site 2 dit l'état vrai (PR ou pas).

## Décision 2 — corriger le diagnostic dans le code, et le promouvoir en document

- `:1036-1037` : garder la raison d'être de `CLAUDE_CODE_API_BASE_URL` (elle est vraie et
  load-bearing) ; **retirer la chaîne causale** « → 401 / SDK stall → idle_timeout → Turns:1 » et la
  remplacer par le fait daté : cette variable a corrigé l'incident du 2026-08-05 (appels internes
  du CLI) ; elle **n'explique pas** les kills `idle_timeout` (mika#2029 : `/v1/messages -> 200`).
- `:3041` : la phrase livrée à mika-dev nomme ce qui est mesuré — « le garde-fou `<subtype>` a
  tué la session ; la cause connue de cette classe est cpp#177 (attente-modèle mesurée contre
  `idleTimeout`) ; voir `docs/solutions/dev-loop/pilot-idle-timeout-kill-classes-2026-09-11.md` » —
  et ne cite plus mika#1901 ni le 401.
- Le document `docs/solutions/dev-loop/pilot-idle-timeout-kill-classes-2026-09-11.md` porte : la
  table des six lignées éliminées avec leur preuve (reprise des commentaires du 08-29), les trois
  méthodes de mesure réutilisables (grep ancré `'(0 content stream events this session)'` avec ses
  deux contrôles ; `durée ≈ tours × 301 s` ; histogramme TTFT `tool result → message_start` par
  tranche de 30 s), les deux classes restantes et leur état, le lien cpp#177, et le résidu avec sa
  condition de réveil. **AC1 + AC4 de #2035** : la condition est nommée, et le diagnostic n'est plus
  enterré dans un commentaire.

## Décision 3 — les démons d'egress ne survivent pas à leur code : deux couches, comme la porte de déploiement

Même contrat en deux couches que `make deploy` ↔ garde sous-dépôt (CLAUDE.md du méta-dépôt) :

- **Couche 1, le déploiement redémarre ce qu'il installe.** `Makefile` : une cible
  `stop-pilot-egress` (sans `sudo` — ce sont des processus de l'utilisateur) qui trouve le pid
  écoutant sur `/tmp/mika-pilot-egress.sock` (`ss -xlp`) et celui sur `127.0.0.1:8892`
  (`ss -tlnp`), leur envoie `SIGTERM`, attend la fermeture de l'écoute (borne 3 s) ; `deploy` l'appelle
  **après** `install`. Le prochain dispatch relance les deux depuis les fichiers frais — c'est le
  chemin existant `:491-536` / `:383`, auto-réparateur par conception (commentaire du 08-29 04:06Z).
  Jamais `pgrep -f` (vacueux dans ce harnais), jamais un pidfile comme source unique (le démon
  courant n'en a pas).
- **Couche 2, la garde de dispatch détecte l'âge.** `_ensure_pilot_egress_proxy` et
  `_ensure_pilot_helper` : quand l'écoute répond, comparer l'instant de démarrage du processus
  (`/proc/<pid>/stat` champ 22 + `btime`, ou `ps -o lstart=`) au mtime de ses fichiers de code
  (`$_PILOT_EGRESS_PROXY_BIN` ; `$_PILOT_HELPER_ADDON` + `$_PILOT_GH_HELPER_ADDON`). Si le code est
  plus récent : ligne stderr `pilot_egress_guard.stale pid=<pid> code_mtime=<ts> proc_start=<ts>`,
  puis redémarrage par le même chemin que la couche 1 — **sauf** si l'écoute a des connexions
  établies (`ss -x state established` / `ss -tn state established`), auquel cas
  `pilot_egress_guard.stale_deferred` et le dispatch continue sur le démon périmé. Étroite et
  single-purpose : elle ne juge que l'âge, pas la santé (mika#2041 garde la santé).

Pourquoi les deux : la couche 1 aurait couvert les deux incidents mesurés ; la couche 2 couvre
`make install` seul, une copie manuelle, et le prochain oubli. Un remède qui repose sur « penser à
redémarrer » a déjà une demi-vie mesurée : douze jours.

## Acceptance criteria

- **AC1 — nommée.** `docs/solutions/dev-loop/pilot-idle-timeout-kill-classes-2026-09-11.md` existe et
  contient : la table des lignées éliminées, les trois méthodes de mesure, les deux classes
  restantes, le renvoi cpp#177, le résidu et ses conditions de réveil.
- **AC2 — comptée sans grep.** Sur le chemin `_terminated_empty`, le callback porte exactement
  une ligne `^NO_PR: pilot_terminated_<subtype>$` ; le parent reçoit
  `callback_no_pr_pilot_terminated_<subtype>`. Rouge-avant : un test `test-dispatch-lib.sh` qui
  joue `STATUS=terminated SUBTYPE=idle_timeout` avec HEAD immobile et arbre propre et attend la
  ligne ; **contrôle négatif** dans le même test : le mode `banner` (HEAD a bougé) n'émet pas de
  `pilot_terminated_*`. Un test Rust unitaire sur `parse_no_pr_reason("NO_PR: pilot_terminated_idle_timeout")`
  → `Some("pilot_terminated_idle_timeout")`.
- **AC3 — le diagnostic faux est parti.** `grep -c 'SDK-stall chain\|SDK stall → guardrail' skills/bundled/_shared/dispatch-lib.sh`
  rend `0` ; le mode `full` de `_classify_terminated_session` cite cpp#177 et le document d'AC1 ;
  assertion dans `test-dispatch-lib.sh` (sur la sortie de la fonction, pas sur le source).
- **AC4 — le déploiement redémarre les démons.** `make deploy` passe par `stop-pilot-egress` après
  `install` ; la cible termine sans `sudo` et sans erreur quand aucun démon ne tourne. Test :
  un faux écouteur sur un socket unix de test est arrêté par la fonction partagée ; assertion que le
  pid n'existe plus dans les 3 s.
- **AC5 — la garde de dispatch voit l'âge.** Rouge-avant, dans le même fixture que mika#2041
  (`test-dispatch-lib.sh:4217+`, `MIKA_PILOT_EGRESS_LOG_DIR` redirigé) : (a) faux écouteur lancé,
  puis `touch` du « binaire » → `_ensure_pilot_egress_proxy` écrit `pilot_egress_guard.stale` et le
  pid change ; (b) **contrôle négatif** : binaire plus vieux que le processus → aucune ligne
  `stale`, même pid ; (c) faux écouteur avec une connexion établie → `stale_deferred`, même pid.
- **AC6 — AC3 de #2035 (push vide) reste vrai** : `test-dispatch-lib.sh:3811-3820` inchangé et
  vert ; aucun nouveau site de push.
- `make test-dispatch-lib` vert ; `cargo test -p mika-agent parse_no_pr_reason` vert ;
  `make verify-bundled-skills` vert (dispatch-lib est un skill bundlé seedé depuis le binaire).

## Fire-Disposition

- **Couche 1 (`stop-pilot-egress`)** se déclenche à chaque `make deploy` : disposition = arrêt des
  deux démons, relance différée au prochain dispatch. Pas de branche « en vol » : un `make deploy`
  redémarre déjà `mika-spirit`, donc coupe déjà les pilotes en vol — aucune nouvelle interruption.
- **Couche 2 (`stale`)** se déclenche au prochain dispatch après une installation hors `make deploy`
  : disposition = redémarrage si aucune connexion établie ; `stale_deferred` sinon, le dispatch
  continue, et la ligne stderr est persistée avec le log du dispatch (mika#2165). Elle ne modifie
  jamais le verdict du dispatch.
- **`NO_PR: pilot_terminated_*`** : le reaper de parents (`engine.rs:3159`) écrit le motif dérivé ;
  la retry-policy de mika-dev est inchangée (le motif est dans la famille `callback_no_pr_*` déjà
  traitée comme « pas de PR »).

## Phases

0. **Rouge-avant.** Écrire les tests d'AC2 (a+b), AC3, AC5 (a, b, c) ; les voir échouer pour la
   raison attendue (ligne absente / phrase présente / pas de redémarrage). `cargo test` pour AC2-Rust.
1. **AC2** — `dispatch-lib.sh:2693` : `_set_pr_status_line "NO_PR: pilot_terminated_$(printf '%s' "${SUBTYPE:-unknown}" | tr -c 'a-z_\n' '_')"`
   dans le bloc `_terminated_empty`. Vérifier que le trap de sortie (`:1406-1419`) ne réécrit pas
   la ligne sur ce chemin (il ne délivre son propre callback que sur crash).
2. **AC3 + AC1** — réécrire `:1036-1037` et le mode `full` de `_classify_terminated_session` ;
   écrire le document `docs/solutions/dev-loop/…` (source : commentaires du ticket, ce plan, cpp#177).
3. **AC4** — `Makefile` : `stop-pilot-egress` + insertion dans `deploy` après `install`. Logique
   d'arrêt factorisée dans un script `scripts/mika-pilot-egress-stop` (partagé avec la couche 2 —
   dispatch-lib ne peut pas appeler `make`), installé par `install` à côté du proxy.
4. **AC5** — `_ensure_pilot_egress_proxy` / `_ensure_pilot_helper` : prédicat d'âge + `stale` /
   `stale_deferred` + redémarrage via le script de la phase 3.
5. **Vérification vivante avant PR** (mémoire « déployé ≠ efficace ») : sur l'hôte, après
   `make deploy`, `ss -xlp` montre un pid plus récent que le fichier, et la première ligne du
   journal d'egress après relance est horodatée `[2026-…Z]`. Sans cette ligne, l'AC4 n'est pas
   « vu faucher du vivant ».
6. `/ce:review`, `/ce:compound` (le document d'AC1 **est** le compound de ce ticket), PR
   `Closes senara-solutions/mika#2029`, avec mention `Sibling: senara-solutions/claude-pilot#177`.

## Hors périmètre

- La cause côté pilote (fenêtre d'attente-modèle) — **cpp#177**, dépôt hors allowlist boucle,
  spawn manuel.
- L'origine du TTFT bimodal ~355 s — résidu, voir ci-dessous. Ce plan **rend la mesure possible**
  (journal horodaté effectif) ; il ne la fait pas.
- La classe mort-née (n=3, 09-03) — dormante, voir ci-dessous.
- Le seuil `idleTimeoutMs` (300 s) et `modelWaitCeilingMs` (900 s) — inchangés ; les augmenter
  déplacerait le mur (commentaire du 08-30).
- mika#2121 parties restantes (opérateur-gaté) ; ce plan réutilise son contrat U1 sans le modifier.
- Un audit event engine dédié (Décision 1, rejeté).

## Résidu dormant — conditions de réveil datées

- **TTFT bimodal ~355 s.** Réveil : **24 h de journal d'egress horodaté** après le merge de ce
  plan et un `make deploy`. Mesure à faire alors : pour une session pilote, aligner ses
  `tool result → message_start` de 300–389 s avec les lignes `ALLOW POST /v1/messages -> <statut>`
  du journal. Une seule ligne `200` par appel lent = latence serveur (à porter à Anthropic avec les
  `request-id`, header déjà allowlisté `:300`) ; deux lignes (dont une `None`/5xx) = retry client
  après timeout, à localiser dans le CLI bundlé. Ficher avec cette mesure, pas avant.
- **Classe mort-née.** Réveil : **une occurrence** `(0 content stream events this session)` sur un
  log horodaté (post-cpp#168). Le log dira alors si le SDK a émis autre chose que `init` avant les
  300 s, ce que les trois logs du 09-03 (non horodatés) ne pouvaient pas dire.

## Risques

- **Redémarrer un démon pendant un appel en vol** (couche 2) : couvert par `stale_deferred` sur
  connexion établie ; et le CLI bundlé retente une connexion perdue (les sessions survivent déjà à
  des attentes de 355 s). Couche 1 n'ajoute aucune interruption à celles d'un `make deploy`.
- **`ss` absent** sur un hôte : les deux couches dégradent en « ne rien faire + une ligne stderr
  `pilot_egress_guard.no_ss` » — jamais un échec de dispatch ni de déploiement.
- **Le token `pilot_terminated_<subtype>` avec un subtype inattendu** : `tr -c 'a-z_'` garantit la
  grammaire `[a-z_]+` de `parse_no_pr_reason` ; pire cas = `pilot_terminated_unknown`, toujours
  comptable.
- **Le document d'AC1 vieillit** comme les commentaires qu'il condense : il porte ses dates et ses
  conditions de réveil, pas des conclusions ouvertes.

---
issue: 2249
type: fix
title: "loop-substrate : reaper mtime-worktree côté engine — un pilote vivant qui n'écrit plus est détecté, audité, disposé"
branch: bug/2249/loop-substrate-les-pilotes-calent
status: groomed
---

# Plan — D1 : reaper `pilot_silent_stall` côté engine (mika#2249)

## Invariant (une phrase)

**Un dispatch pilote dont le process est vivant mais dont le worktree n'a reçu aucune écriture
depuis plus que le plafond de silence structurel du pilote est détecté par un reaper EXTERNE au
process pilote, nommé dans `audit_events`, et disposé — sans intervention manuelle.**

Le mot load-bearing est **externe**. Un pilote dont le watchdog *interne* est affamé ne peut pas
non plus faire feu sur un timer *interne* : c'est le fait mesuré des deux occurrences #2246
(2h18 et 58 min muettes, `result` vide, aucun marqueur terminal, alors que le watchdog interne
cpp#145 tournait — champs `toolWaitCeiling=1800s modelWaitCeiling=900s` présents dans les trois
logs). Tout détecteur logé dans claude-pilot hérite de la panne qu'il doit voir.

## Ce que le code dit aujourd'hui (vérifié, `file:line`)

- Les reapers existants se déclenchent tous sur **l'état de la tâche**, jamais sur l'activité
  d'écriture : `engine.rs:271` (orphan-on-startup), `engine.rs:738`
  (`reap_orphaned_pending_issue_tasks`), `engine.rs:1096` (`reap_stale_blocked_dispatch_tasks`),
  `engine.rs:2508` / `engine.rs:2847` (parents self_dev). Une tâche `in_progress` dont le process
  est **vivant** leur est structurellement invisible — c'est pourquoi fb355061 a vécu 2h18.
- Le watchdog PID existant, `engine.rs:1421` (`check_callback_process_liveness`), est le
  **symétrique exact** de ce qu'il faut ici : il traite le process **mort** (→ `failed` après
  grâce). Le cas du process **vivant et muet** n'a aucun propriétaire.
- La population est déjà requêtable sans nouvelle SQL : `db.rs:8558`
  (`get_active_callback_tasks_with_pid`) rend exactement `trigger_type='callback'`
  + `status='in_progress'` + `process_id IS NOT NULL` — les dispatches en vol.
- La liveness PID sûre existe : `process_liveness::is_same_process_alive(pid, start_time)`, avec
  `metadata.process_start_time` écrit au spawn (`skills/executor.rs:3106-3125`).
- Le kill existe : `task_engine/process_kill.rs` (`kill_process_gracefully`,
  `cancel_task_and_kill:214`), avec la garde de réutilisation de PID (#855).
- Le seuil configurable a un patron établi : `config.rs:1226`
  (`DEFAULT_PHANTOM_SWEEP_AGE_SECONDS`) + `config.rs:1780` (`effective_*`) + override env.
- La cadence existe : `engine.rs:453`, `DB_SCAN_INTERVAL_TICKS = 60` — aucun nouvel intervalle
  n'est requis.
- **Aucune sonde mtime-worktree n'existe** dans `mika-agent` (grep `worktree` sur
  `crates/mika-agent/src/` : uniquement `wip_rescue.rs` — worktree éphémère de rescue — et des
  commentaires de `webhook_dispatch.rs`). Le chemin du worktree d'un dispatch **n'est stocké
  nulle part** : ni colonne `tasks`, ni `metadata`, ni `worktree_claims` (`db.rs:1491` — clé
  `(repo, issue_number)`, **pas** le chemin, délibérément).

## Décision 1 — d'où vient le chemin du worktree

`dispatch-lib.sh:2231` est **le seul** endroit qui connaît le chemin
(`WORKTREE_DIR=$(derive-worktree-path ...)`). Le côté Rust ne le connaît pas et ne doit pas le
re-dériver : re-dériver la branche côté Rust est exactement la duplication que mika-platform#58 a
fermée, et `db.rs:1484-1490` l'écrit noir sur blanc comme raison de ne PAS clé par chemin.

**Retenu : le shell déclare, l'engine lit.** Patron déjà en place à un saut près — mika#2040
(`inject_pilot_transcript_env`, `executor.rs:3064` + stamp
`PILOT_TRANSCRIPT_EXPECTED_KEY`, `executor.rs:3086`) fait déjà exactement ce trajet pour le
transcript.

1. Avant le spawn, le Rust injecte `MIKA_DISPATCH_WORKTREE_FILE=<chemin runtime dérivé du
   task_id>` et stampe ce chemin en `metadata.dispatch_worktree_file`.
2. `dispatch-lib.sh`, immédiatement après un `git worktree add` réussi (autour de
   `dispatch-lib.sh:2290-2298`), écrit `WORKTREE_DIR` dans ce fichier — une ligne, aucun
   nouveau canal.
3. Le reaper lit le stamp, lit le fichier, obtient le chemin.

**Fail-safe explicite : l'absence surveille rien.** Metadata absente, fichier absent, fichier
vide ou chemin inexistant ⇒ la tâche n'est **pas** candidate au reap. Un dispatch free-text sans
worktree (mika#1593, `engine.rs:3354`) et un dispatch dont la déclaration s'est perdue tombent
tous deux du bon côté : un reaper qui **tue** ne doit jamais tuer sur une absence de preuve.

**Écarté :** faire globber `.claude/worktrees/*-<issue>-*` par l'engine — cela obligerait
`mika-agent` à connaître la racine mika-platform (`MIKA_PLATFORM_DIR`, `dispatch-lib.sh:6081`),
notion qui n'existe aujourd'hui **nulle part** côté Rust (grep : 0 occurrence), et à arbitrer les
collisions de slug entre deux worktrees d'une même issue.

## Décision 2 — la valeur de N, et pourquoi le corps du ticket la sous-estime

Le corps du ticket illustre D1 avec « > N min (ex. 15-20 min) », et le commentaire
d'investigation reprend « Seuil N ~15-20 min ». **Cette valeur tirerait sur du travail sain.**
Deux faits, tous deux mesurés le 2026-09-08 :

1. **Contrôle négatif direct.** Le run c3f9a2f9 (#2238), pilote **sain** sur une impl longue en
   zone décision-core, a écrit `ci_success_handler.rs` / `verdict_handler.rs` /
   `pr_merge_with_gate.rs` à **20:19 puis 20:43** — un intervalle inter-écriture de **24 min** sur
   un pilote qui travaillait. Un seuil de 15-20 min l'aurait tué. C'est la faute que
   `feedback_pilot_liveness_probe_worktree_mtime_not_task_updated_at` a déjà fait payer une fois.
2. **Borne structurelle.** Le plafond de silence d'un pilote dont le watchdog interne
   **s'exécute** est de **1800 s** (`toolWaitCeiling=1800s`, défauts `types.py:76-93`, lancés sans
   override par `dispatch-lib.sh:2551`). Au-delà de 1800 s de silence, soit le pilote a fait feu
   lui-même (classe a0fcb92d — déjà gérée, callback livré), soit son watchdog n'a pas fait feu
   (classe #2246 — la seule que ce reaper doit voir).

**Défaut retenu : 2700 s (45 min)** = plafond structurel 1800 s + 900 s de marge. Il sépare les
deux populations mesurées sans recouvrement : les stalls réels (2h18, 58 min) sont **au-dessus** ;
l'impl longue saine (24 min entre écritures) est **en dessous**, avec un facteur ~2.

Ceci **satisfait** l'AC3 du ticket, qui demande « N configurable ; défaut **mesuré contre les
cas** » — c'est cette mesure. Le « ex. 15-20 min » du corps est une illustration, et le plan la
corrige au lieu de la copier. **À arbitrer par l'architecte** : si l'illustration doit primer sur
la mesure, le contrôle négatif c3f9a2f9 doit être réfuté d'abord.

**La marge ne peut PAS être élargie sans perdre de la couverture (réponse au finding F3 de la
première passe).** L'architecte suggère 3600-5400 s par prudence face au mode d'échec destructif.
Les deux cas fondateurs l'interdisent : les stalls mesurés durent **2h18** et **58 min**, et
58 min = **3480 s**. Un seuil de 3600 s **rate déjà** le second cas ; 5400 s en rate un sur deux
avec de la marge. La fenêtre qui attrape les deux occurrences tout en restant au-dessus du plafond
structurel est donc **]1800 s, 3480 s[**, et 2700 s en est à peu près le milieu. Élargir la marge
n'achète pas de la sûreté : cela échange un faux positif hypothétique contre un faux **négatif
mesuré**.

La prudence que F3 demande à juste titre est donc payée autrement — par la Décision 4, qui retire
au reaper le droit de tuer avant que la mesure existe. C'est la même prudence, placée là où elle ne
coûte pas de couverture.

Override env : `MIKA_PILOT_STALL_REAP_AGE_SECONDS`, patron `config.rs:1780`.

## Décision 3 — ce que « disposer » veut dire, sans casser le retry

`dispatch-lib.sh:1340` : sur SIGTERM sans fichier de raison pré-écrit, le trap écrit
`STATUS=CANCELLED_BY_SIGNAL` ; et `self-dev-callback/system_prompt.md:54` traite tout
`CANCELLED_BY_*` comme un cancel opérateur — **« do NOT retry »**. Un reaper qui SIGTERM
naïvement tuerait donc le pilote **et** le retry : le ticket ne rendrait toujours rien, seulement
plus vite. La boucle ne deviendrait pas auto-guérissante, ce qui est le seul intérêt de D1.

**Retenu :**

- Le reaper **pré-écrit** `/tmp/mika-cancel-reason-<pid>` avec un discriminant propre —
  `STATUS=REAPED_PILOT_SILENT_STALL` — avant le SIGTERM, en réutilisant exactement le protocole
  de `process_kill.rs:250-262`. Le trap ne l'écrase pas (il n'écrit que si le fichier est
  absent) : la trace nomme la vraie cause au lieu de mentir en « cancelled by signal ».
- La tâche est marquée **`failed`** (pas `cancelled`), garde de statut comprise, sur le modèle de
  `check_callback_process_liveness` (`engine.rs:1523-1540`) — re-lecture du statut avant écriture
  pour perdre proprement contre un callback en vol.
- **Le re-dispatch n'est PAS porté par ce ticket.** Le chemin structurel existe déjà :
  `stuck_ready_reconcile` (`auto_pull.rs:3120`) re-drive un ticket resté `ready`, avec budget de
  re-drive (mika#2020). Ajouter une branche de retry dans un prompt serait de l'enforcement par
  prompt sur un substrat de boucle — précisément ce que
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` interdit.
- **Vérification requise en Phase 3**, pas supposée : que `REAPED_PILOT_SILENT_STALL` ne tombe pas
  dans une branche `CANCELLED_BY_*` du parseur, et que le ticket redevienne éligible à
  `stuck_ready_reconcile` après le reap. Si la vérification échoue, la branche de prompt
  redevient nécessaire et **est fichée séparément** (elle change la surface d'un autre skill).

## Décision 4 — le reaper atterrit en observation, il n'atterrit pas armé

**Adopté de la première passe architecte (F2/F3).** Le seuil repose sur une borne structurelle
solide (1800 s) et un contrôle négatif à **n=1** (les 24 min de c3f9a2f9). Le mode d'échec est
destructif et asymétrique : un faux négatif coûte un slot de dispatch, un faux positif détruit des
heures de travail en zone décision-core. Une valeur par défaut dérivée d'un échantillon de taille 1
ne mérite pas le droit de tuer avant d'avoir été mesurée contre le trafic réel.

`pilot_stall_reap_enabled` (`MIKA_PILOT_STALL_REAP_ENABLED`), **défaut `false`** au landing :

- **Désactivé (défaut) :** le reaper s'exécute, mesure, et émet `pilot_silent_stall`
  (`warn!` + `audit_events`) avec l'âge mtime mesuré — mais **ne tue pas** et ne transitionne pas.
  La détection est immédiate ; seule la disposition attend.
- **Activé :** le chemin complet d'AC1 (kill + `failed`).

Le code de disposition est **livré et testé** dans ce ticket, désactivé — pas reporté. AC5 et AC6
s'exécutent avec le flag armé ; AC8 pince le contraire : flag au défaut, même entrée, aucune
transition et aucun kill, mais l'audit **présent**.

**Condition de bascule — datée et concrète, pas « plus tard » :** activer quand `audit_events`
porte **au moins 3 lignes `pilot_silent_stall`** dont la revue confirme que **zéro** correspond à un
pilote qui écrivait encore (contrôle : les mtimes du worktree au moment de la mesure). Si une seule
est un faux positif, c'est le **seuil** qui est révisé, pas le flag qui est armé. La bascule est une
décision opérateur, et elle est le seul reste manuel de ce ticket.

**Ce que cela fait à l'AC1 du ticket.** L'AC1 demande « détecté et disposé automatiquement, sans
intervention manuelle ». Ce plan livre la détection automatique immédiatement, et la disposition
automatique derrière un flag dont la bascule est spécifiée ci-dessus. Je considère l'AC honoré — le
mécanisme est entier, seul son armement est gradué. **À trancher explicitement par l'architecte en
seconde passe :** si l'AC1 exige l'armement au landing, le flag passe à `true` par défaut et la
Décision 4 se réduit au kill-switch.

## Acceptance criteria

- **AC1** — Un pilote dont le process est vivant et dont le worktree n'a reçu aucune écriture
  depuis plus de N est détecté et disposé **automatiquement** (WARN + `audit_events` + kill),
  sans intervention manuelle. *(= AC1 du ticket)*
- **AC2** — L'événement est nommé `pilot_silent_stall` : `warn!` collecté **et** ligne
  `audit_events` (`tool_name='pilot_silent_stall'`, `target_key='task:<id>'`,
  `before_value='in_progress'`, `after_value='failed'`, `reasoning` portant l'âge mtime mesuré et
  le chemin du worktree). *(= AC2 du ticket)*
- **AC3** — N est configurable (`pilot_stall_reap_age_seconds` + `MIKA_PILOT_STALL_REAP_AGE_SECONDS`),
  défaut **2700 s**, dont la dérivation est écrite au point de code (plafond 1800 s + marge, et le
  contrôle négatif des 24 min). *(= AC3 du ticket, « défaut mesuré contre les cas »)*
- **AC4 — contrôle négatif, terme par terme.** Le prédicat est une conjonction ; chaque terme est
  pincé par un cas qui ne diffère que par lui. Ne sont **pas** reapées : (a) tâche dont le
  worktree a été écrit il y a moins de N ; (b) tâche dont le process est **mort** (elle appartient
  à `check_callback_process_liveness`) ; (c) tâche **sans** worktree déclaré ; (d) tâche dont le
  chemin déclaré n'existe pas ; (e) tâche qui n'est plus `in_progress` à la re-lecture.
  Neutraliser un seul terme ne doit pas suffire à faire tirer le reaper.
- **AC5 — non-vacuité (rouge-avant/vert-après).** Un test rejoue la forme fb355061 — tâche
  `callback`/`in_progress`, PID vivant, worktree déclaré dont le mtime max est daté de plus de N —
  et **assert** la transition + la ligne `audit_events`. Rouge sur `main` (aucun reaper ne voit ce
  cas), vert après.
- **AC6** — La disposition écrit `STATUS=REAPED_PILOT_SILENT_STALL` dans le fichier de raison
  **avant** le SIGTERM, et la tâche finit `failed`, non `cancelled`.
- **AC7 — borne de coût.** Le scan mtime a un coût borné et écrit : répertoires exclus
  (`target/`, `.git/objects/`, `node_modules/`), plafond de fichiers/profondeur, et un
  `warn!` si le scan d'un worktree dépasse un budget de temps. Le reaper ne tourne que sur les
  tâches en vol (0-2 en régime normal), une fois par `DB_SCAN_INTERVAL_TICKS`.

- **AC8 — l'observation est le défaut.** Avec `pilot_stall_reap_enabled` à sa valeur par défaut,
  la même entrée qu'AC5 produit la ligne `audit_events` `pilot_silent_stall` **et** laisse la tâche
  `in_progress`, process vivant, aucun signal envoyé. Contrôle positif et négatif dans le même
  test : l'audit présent prouve que le détecteur a vu ; l'absence de transition prouve qu'il n'a
  pas tiré.

## Fire-Disposition

Le livrable détecteur est le test AC5 (+ les cinq contrôles négatifs AC4) dans les tests du
task_engine : **rouge sur `main` actuel** — aucun reaper existant ne se déclenche sur l'activité
d'écriture, la tâche reste `in_progress` et l'assert échoue —, **vert après**. Gate CI `Check`
bloquant, garde permanente : toute régression qui re-rend un pilote muet invisible refait échouer
`Check`.

**Disposition des violations pré-existantes** (pilotes déjà calés au moment du déploiement) : elles
sont **mesurées, pas tuées**. Un pilote calé présent au premier tick après déploiement produit sa
ligne `pilot_silent_stall` et rien d'autre, puisque le flag de la Décision 4 est à `false` — c'est
précisément le trafic qui alimente la condition de bascule. Aucune violation pré-existante n'est
disposée sans que l'opérateur ait armé le flag.

## Phases

1. **Le canal du chemin de worktree.** `executor.rs` : injecter
   `MIKA_DISPATCH_WORKTREE_FILE` (chemin runtime dérivé du `task_id`, dans le même répertoire
   inscriptible que le transcript mika#2040 — **à vérifier bind-monté sous bwrap avant de câbler
   le reste**) et stamper `metadata.dispatch_worktree_file`, en fire-and-forget comme
   `executor.rs:3086` (un dispatch non stampé est invisible au reaper, jamais bloqué).
   `dispatch-lib.sh` : écrire `WORKTREE_DIR` dans ce fichier après le `worktree add` réussi.
2. **La sonde mtime.** Helper dédié (nouveau `task_engine/worktree_activity.rs`) :
   `max_mtime(worktree) -> Option<SystemTime>`, exclusions + bornes d'AC7, testable seul.
3. **Le reaper.** `reap_silently_stalled_pilots()` dans `engine.rs`, appelé dans le bloc
   `DB_SCAN_INTERVAL_TICKS` de `tick()` (`engine.rs:453`), **après**
   `check_callback_process_liveness` — les deux sélectionnent des populations disjointes (process
   mort vs vivant) et cet ordre garde la ladder lisible au même endroit. Population :
   `get_active_callback_tasks_with_pid`. Prédicat : conjonction d'AC4. Action : audit → fichier de
   raison → `kill_process_gracefully` → `update_task_failed` sous garde de statut.
4. **Config.** `pilot_stall_reap_age_seconds` + `DEFAULT_PILOT_STALL_REAP_AGE_SECONDS = 2700` +
   `effective_*`, patron `config.rs:1226/1780`, avec la dérivation **et la fenêtre
   ]1800 s, 3480 s[** en doc-comment. Plus `pilot_stall_reap_enabled` (défaut `false`,
   Décision 4) et sa condition de bascule écrite au point de code.
5. **Tests.** AC5 (non-vacuité, rouge-avant vérifié en l'exécutant sur `main` **sans** le fix),
   les cinq contrôles négatifs AC4 — chacun ne neutralisant qu'un terme —, et AC8 (le défaut
   observe sans tirer).
6. **Vérification de la queue de disposition** (Décision 3) : que `REAPED_PILOT_SILENT_STALL` ne
   soit pas absorbé par une branche `CANCELLED_BY_*`, et que le ticket redevienne éligible à
   `stuck_ready_reconcile`. Résultat écrit dans la PR. Si négatif → ticket séparé, pas
   d'élargissement ici.

## Hors périmètre

- **La cause du silence SDK** (throttle 429 sélectif Anthropic, lignée mika#1901) : non corrigeable
  par code, seulement détectable. Ce ticket rend la classe visible et bornée.
- **Le durcissement du watchdog interne de claude-pilot** (D2) : fiché
  `senara-solutions/claude-pilot#168` — instrumentation `logger.py`, py-spy, `asyncio.wait_for`
  autour de `stream.__anext__()`. claude-pilot est hors allowlist de la boucle
  (`reference_dispatchable_repos_allowlist_excludes_claude_pilot`) → spawn manuel.
- **Toute branche de retry dans un prompt de skill** (voir Décision 3).

## Risques

- **Faux positif = travail détruit.** C'est le risque dominant. Il est traité à trois niveaux, pas
  un : le défaut vient d'une borne structurelle et non de l'illustration du corps (Décision 2) ; les
  cinq contrôles négatifs d'AC4 existent pour ça et non pour la couverture ; et le reaper atterrit
  **sans le droit de tuer** (Décision 4), ce qui rend le premier faux positif observable au lieu de
  destructeur.
- **Le répertoire du fichier de déclaration doit être inscriptible sous bwrap.** Phase 1 le vérifie
  avant que quoi que ce soit d'autre soit câblé ; s'il ne l'est pas, la Décision 1 doit être
  re-arbitrée avant d'écrire le reaper.
- **Coût du scan mtime sur un checkout `mika` complet.** Borné par AC7, et mesuré en Phase 2 plutôt
  qu'estimé.

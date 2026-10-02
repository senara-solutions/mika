# fix(loop-substrate) — un lanceur `claude-pilot` mort est un fait estampillé, borné, et il cesse de brûler les fenêtres (mika#2634)

**Ticket :** senara-solutions/mika#2634 — Tier 1, `agent-core`, `dispatch:loop`
**Branche :** `fix/2634/loop-substrate-un-lanceur-claude-pilot`
**Date :** 2026-10-02

---

## Contexte, et les quatre rectifications que la lecture du code impose au ticket

Le ticket est juste sur le symptôme — 2 h 45 de rail mort, trois pilotes morts au
lancement, aucune alarme — et il se trompe sur **où** le signal est perdu. Les
quatre rectifications qui suivent sont le premier livrable : chacune change le
remède.

### R1 — La population mesurée est le PRÉ-FLIGHT, pas le lancement

`dispatch_claude_pilot` (`skills/bundled/_shared/dispatch-lib.sh:9673`) exécute
un smoke test **avant** d'installer son trap :

| ligne | ce qui se passe |
|---|---|
| 9680 | `exec 9>>"$TRACE_FILE" 2>/dev/null` — ouvre la trace `set -x` |
| 9689-9691 | `command -v jq` / `mika` / `claude-pilot`, chacun `|| { echo … >&2; exit 1; }` |
| **9700** | `if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then` — **le smoke test mika#1200** |
| 9713 | `exit 1` — avec le diagnostic prescriptif (`uv tool install --force --editable`) écrit juste au-dessus |
| 9737 | `_parse_input_json` (pose `TASK_ID`) |
| **9740** | `trap '_dispatch_lib_exit_trap' EXIT` |

Un shebang cassé (`#!/usr/bin/python3.14` au lieu du Python du venv `uv tool`)
ne fait **pas** échouer `command -v claude-pilot` — le fichier existe et est
exécutable. Il fait échouer `claude-pilot --help`, donc le smoke test de la
ligne 9700, donc `exit 1` à la ligne 9713, **treize lignes avant** que le trap
existe.

Conséquence, et elle explique chaque octet mesuré par le ticket : aucun callback
n'est livré, la tâche reste non terminale, donc `update_task_failed` rend
`Ok(true)` et le moteur écrit `tasks.result = "Process Exit code: 1: "` avec un
stderr vide (`skills/executor.rs:4741`), en WARN
`long_running_handler_exit_nonzero` / `task_was_terminal: false`. Et **aucun
journal `/var/log/claude-pilot/<id>.log`** n'existe : le pilote n'a jamais été
lancé.

**Le diagnostic mika#1200 a été écrit mot pour mot pour cette panne** — il nomme
la cause probable et la commande de réparation — et il n'a atteint personne.

### R2 — `stderr_bytes: 0` a une cause nommée DANS LE DÉPÔT, et une doc qui affirme l'inverse

`exec 9>>"$TRACE_FILE" 2>/dev/null` (ligne 9680) : dans un `exec` **sans
commande**, les redirections s'appliquent au **shell courant, de façon
permanente**. Donc `2>/dev/null` ne masque pas l'échec de l'ouverture de fd 9 —
il remplace le fd 2 du handler pour **tout le reste du script**.

Ce n'est pas une découverte : le commentaire de `_halt_family`
(`dispatch-lib.sh:4267-4271`) l'écrit déjà textuellement — *« this function runs
inside `dispatch_claude_pilot`, whose fd 2 is `/dev/null` from the moment
`exec 9>>"$TRACE_FILE" 2>/dev/null` runs (mika#903) »*. Ce qui manquait est le
lien : personne n'avait relié « fd 2 = /dev/null » à « le pré-flight écrit son
diagnostic dans le vide ».

**Et le `CLAUDE.md` affirme le contraire**, § mika#2532 : *« the three that
capture a `STDERR_FILE` capture claude-pilot's stderr, never their own (the
handler's `echo … >&2` goes to the inherited fd 2, i.e. the executor's pipe) »*.
Faux pour `dispatch-lib.sh`. C'est très exactement pourquoi mika#2532 — qui
persiste le stderr du handler sur `metadata.handler_failure` et dont le
mécanisme fonctionne — n'a rien eu à persister ici.

### R3 — La cause Python est DÉJÀ capturée, et perdue pour une seule raison

Ligne 9700 : `claude-pilot --help >/dev/null 2>&9`. Le stderr du smoke test va
sur **fd 9**, c'est-à-dire dans `$TRACE_FILE`. Donc le `ModuleNotFoundError: No
module named 'claude_pilot'` **est écrit sur disque**.

Et le trap EXIT, sur le chemin crash, append les 50 dernières lignes de ce
fichier au `RESULT` (`dispatch-lib.sh:1954-1973`). Autrement dit : **la cause
voyagerait jusqu'au callback si le trap était armé.** Le remède d'AC3 est donc
un remède d'**ordre**, pas d'instrumentation.

### R4 — Le discriminant « exit non nul + aucun journal » a des faux positifs mesurables

AC1 propose cette conjonction. Elle est séduisante et elle est fausse dans trois
populations du dépôt :

| population | exit | journal | classé « lanceur mort » ? |
|---|---|---|---|
| **confinement refusé** (mika#2141 / mika#2049) | 78 — `_run_pilot_sandboxed` refuse | absent | **faux positif** — la cause est le relais d'egress, pas le lanceur, et le texte du refus le dit explicitement |
| `auto_skipped` / `already_groomed` (mika#2012) | 0 | absent | non (exit 0), mais la conjonction devient fragile |
| dry-run (`_handle_dry_run`) | 0 | absent | idem |
| `cwd-guard` refusé (mika#2536) | 1, autre handler | absent | **faux positif** sur un autre skill |

Et elle introduirait un **second lecteur** du chemin
`<pilot_log_dir>/<task-id>.log` : le premier est
`task_engine::engine::probe_pilot_log_signal` (`engine.rs:3121`, mika#2277). Un
second lecteur qui dérive le même chemin est la classe que mika#2158 a dû
refermer, et il n'est pas gratuit ici : `spawn_long_running_exec` ne reçoit pas
`Settings`, donc il faudrait threader `effective_pilot_log_dir()` à travers
`ToolContext` **ou** relire la cascade config-rs à la main.

---

## Décisions

### D1 — Le fait est ESTAMPILLÉ par son producteur : un code de sortie dédié

Le pré-flight sort avec **79** au lieu de **1**. Le moteur classe sur ce code et
n'infère rien.

C'est la doctrine maison, écrite trois fois : *« PR origin is a fact stamped by
its producer, never reconstructed afterwards »* (mika#2026) ; *« the engine is
told, never derives »* pour le worktree de dispatch (mika#2249) ; *« la cible PR
est dite, jamais dérivée »* (mika#2368, qui condamne nommément la dérivation
**tardive** — celle qui se ferait au moment où l'échec n'est plus rattrapable).
Elle supprime d'un coup les quatre faux positifs de R4, et elle n'ajoute **aucun
lecteur de chemin de journal**.

`79` est libre : 78 est pris par le refus de confinement (même famille
sémantique — « rien n'a été lancé »), 64-78 sont les `sysexits.h`, 126/127/128+
sont réservés par le shell.

**Coût nommé :** la valeur est écrite deux fois, en shell et en Rust. C'est le
doublon inter-langage que mika#2520 a déjà dû assumer pour
`GIT_OPS_PROTECTED_BRANCHES` — aucun single-source n'existe. Il est **gardé**
par un scan (U3 ci-dessous) qui lit le shell depuis Rust, le motif que
`canonical_tokens` emploie déjà.

### D2 — Le smoke test migre APRÈS le trap ; les trois `command -v` restent AVANT

L'ordre devient : `command -v jq` / `mika` / `claude-pilot` →
`_parse_input_json` → `trap EXIT` → **smoke test** → le reste.

Ce n'est pas un détail d'ordonnancement : le trap livre son callback par
`mika ask --task-complete` et son corps emploie `jq`. Déplacer les trois
`command -v` après le trap donnerait un trap qui échoue en silence sur l'hôte où
`jq` ou `mika` manque — on échangerait une panne muette contre une autre. Le
smoke test, lui, n'a aucune de ces dépendances au moment où il tourne.

Conséquence : sur cette panne le callback **est** livré, donc la tâche devient
terminale, donc le moteur passe sur la branche `Ok(false)` et logue en INFO
`task_was_terminal: true`. Le `RESULT` du callback porte la cause.

### D3 — Le marqueur du callback est TERMINAL, et pas `HANDLER CRASH`

Laisser le trap produire son `HANDLER CRASH (exit code 79)` générique serait un
piège mesuré : `HANDLER CRASH` est dans la population que
`self-dev-callback/system_prompt.md:176` invite à rejouer (*« To retry, call
`run_claude_pilot` normally »*), et `dispatch-lib.sh:4198` le grepe à côté de
`PIPELINE FAILURE:`. Un lanceur cassé rejoué est exactement la boucle que
mika#2545 a dû refermer pour l'`ESCALATE` de groom.

Le patron à reprendre est donc celui de mika#2545, verbatim : un
`Outcome: LAUNCHER_DEAD` **testé avant** les branches de rejeu, terminal, qui
**n'incrémente pas** `pipeline_retry_count`.

Deux compensations, et il faut **les deux** (mika#2545 l'a payé) :
1. `_measure_cycle_output` / `_gate_non_empty_cycle` (`dispatch-lib.sh:4030`,
   `4141`) doivent reconnaître `^Outcome: LAUNCHER_DEAD` comme une sortie
   délibérée, sinon un lanceur mort est en plus accusé d'`empty_completion`.
2. `self-dev-callback/system_prompt.md` gagne une branche, placée **avant** la
   routine `PIPELINE FAILURE:`, sur le modèle littéral du discriminateur
   `Outcome: ESCALATE`.

### D4 — Un ledger, deux valeurs, un lecteur, deux surfaces

`audit_events` avec `tool_name = 'pilot_launcher_health'` et deux `after_value` :
`dead` et `recovered`. **Un nom, l'issue dans `after_value`** — motif
`ready_label_outcome` (mika#2323) plutôt que les deux noms de
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156) : les deux issues
appartiennent au même site et à la même population, et un `GROUP BY after_value`
rend les deux comptes en une requête, soustractibles.

`recovered` n'est écrit **qu'en transition** (un dispatch sain alors que la
fenêtre portait au moins un `dead`), donc le volume est **nul en régime sain** —
la doctrine mika#2131 est respectée sans arbitrage.

Lecteur unique : `crates/mika-agent/src/pilot_launcher_health.rs`, le motif
`live_pilot.rs` (mika#2279) verbatim. Deux surfaces l'interrogent, une seule
répond.

### D5 — Le frein est une FENÊTRE, pas un compteur persistant

« ≥ 2 occurrences de `dead` dans les N dernières minutes » plutôt qu'un compteur
de consécutifs. Trois raisons, dans l'ordre du poids :

- **Il se lève de lui-même : il n'y a rien à effacer** — la propriété que
  mika#2597 écrit pour son hold et que mika#2347 a dû bâtir à la main faute de
  l'avoir. Un lanceur réparé sort de la fenêtre sans qu'aucun geste ne soit posé.
- Un compteur persistant demande un site de remise à zéro, et mika#2158 a mesuré
  ce que coûte un compteur remis à zéro par l'action qu'il compte : 31 re-drives
  affichant 1.
- Le pire cas est borné et auto-réparant : si le lanceur n'est pas réparé, **un**
  dispatch est brûlé par fenêtre au lieu de tous — ce que le ticket demande
  (« au lieu de brûler les fenêtres une à une »).

### D6 — Fail-OPEN sur la lecture du ledger, et l'asymétrie l'exige

Un ledger illisible ⇒ **on ne bloque pas**. C'est l'inverse de `wip_rescue`
(mika#2199) et de `run_gh pr ready` (mika#2624), et l'inversion est raisonnée :
là-bas un faux négatif faisait attendre **une** PR ; ici un faux positif gèle
**tous** les dispatches de la flotte. Un faux négatif coûte une fenêtre brûlée —
visible, borné, rattrapable au tour suivant.

### D7 — L'« alarme vers le veilleur » est un WARN plus une ligne d'audit

`spawn_long_running_exec` tourne dans un `tokio::spawn` qui ne reçoit que
`cmd_path`, `skill_dir`, `input`, `task_id`, `db`, `github_token` — **aucun
`message_sender`**. Et `control-monitor` est hors de ce workspace : il lit la
base et les journaux (c'est tout l'objet de mika#1990 et de mika#2267).
Fabriquer ici un canal push serait une arbitration de canal déguisée en
observabilité.

Donc : WARN `pilot_launcher_dead` + ligne `audit_events`, **SOLE WRITER**, et le
WARN de frein quand la garde mord. À nommer comme tel dans la PR : la lettre
d'AC1 (« lève une alarme vers le veilleur ») est satisfaite par le canal que le
veilleur lit, pas par un POST.

### D8 — Le frein ne doit pas consommer le budget de re-drive

Un refus de readiness laisse le ticket `ready`, que `auto_pull` Phase 2 re-drive
toutes les 10 min — et chaque re-drive consomme un point du budget mika#2020,
dont trois abandonnent un ticket sain en `operator-review`. Un frein de 60 min
abandonnerait donc des tickets parfaitement valides.

D'où la **seconde surface** : `auto_pull::classify_stuck_ready` gagne un filtre
qui rend `Skip` **sans** reset, exactement comme son voisin `in_flight` et
exactement comme mika#2279 a dû poser son filtre 4b à côté de sa porte 2c.

---

## Implémentation

### U1 — Producteur : le pré-flight estampille (`dispatch-lib.sh`)

1. Déplacer le bloc `if ! timeout 15 claude-pilot --help …` (9700-9714) **après**
   `trap '_dispatch_lib_exit_trap' EXIT` (9740) et avant `_validate_inputs`.
2. Y poser `_STEP="launcher_smoke_test"` (motif mika#2532 R2/R3) si la variable
   existe sur ce chemin ; sinon ne pas l'inventer.
3. Rediriger le `cat >&2 <<'EOF'` prescriptif vers **fd 9** (`>&9`) : sur ce
   chemin fd 2 est `/dev/null` (R2) et fd 9 est la trace que le trap append.
   Vérifier qu'il reste lisible dans les 50 dernières lignes.
4. Poser `RESULT` explicitement avant de sortir, avec un en-tête terminal et la
   ligne `Outcome: LAUNCHER_DEAD`, puis `exit $_EXIT_LAUNCHER_DEAD` (constante
   nommée, valeur 79).
5. Vérifier que le trap survit à ce point d'entrée : `CALLBACK_SENT` et
   `ISSUE_SEAT_CLAIMED` sont posés (9732, 9735) ; `TASK_ID` l'est par
   `_parse_input_json` ; `REPO`/`ISSUE_NUM` peuvent être vides et le
   `_release_issue_seat … || true` de la ligne 1914 le tolère. **Si l'un de ces
   trois points est faux, le corriger est dans le périmètre** — un trap qui
   plante sur ce chemin remplacerait une panne muette par une autre.

### U2 — Compensations de classification (`dispatch-lib.sh`)

6. `_measure_cycle_output` / `_gate_non_empty_cycle` : reconnaître
   `^Outcome: LAUNCHER_DEAD` comme sortie délibérée, sur le modèle littéral du
   reclassement d'`Outcome: ESCALATE` (mika#2545).
7. Vérifier que le nouveau `RESULT` ne matche **aucun** motif de la famille
   retryable du grep `dispatch-lib.sh:4198` (`PIPELINE FAILURE:`,
   `STRUCTURAL VIOLATION:`, `HANDLER CRASH`, `^STATUS=CANCELLED`,
   `^Outcome: PIPELINE_INCOMPLETE`, `^Outcome: ESCALATE`).

### U3 — Moteur AC1 : classer, dire, compter (`skills/executor.rs`)

8. Dans `spawn_long_running_exec`, sur `!status.success()` : quand
   `status.code() == Some(EXIT_PILOT_LAUNCHER_DEAD)`, émettre
   `pilot_launcher_dead` (WARN, champs `task_id`, `code_display`,
   `task_was_terminal`, `stderr_bytes`, `stderr_persisted`, `skill_dir`) **en
   plus** du `long_running_handler_exit_nonzero` existant, qui n'est ni retiré ni
   modifié — un lanceur mort reste un exit non nul, et retirer la ligne générique
   casserait les requêtes publiées.
9. Écrire la ligne `audit_events` (`tool_name = 'pilot_launcher_health'`,
   `target_key = 'task:<callback-id>'`, `after_value = 'dead'`), fire-and-forget
   comme ses quatre voisines : une mesure ne doit jamais casser la livraison
   qu'elle observe. Sibling `pilot_launcher_dead_audit_failed` (WARN) quand la
   ligne ne part pas.
10. Scan de source : `EXIT_PILOT_LAUNCHER_DEAD` est confronté au littéral du
    shell, dans les **deux sens** (motif `check-dispatch-seats-declared.sh`).
11. Scan de source : `pilot_launcher_dead` et `pilot_launcher_health` sont
    **SOLE WRITER**, allowlists livrées **vides**.

### U4 — Moteur AC2 : le frein (`pilot_launcher_health.rs`, nouveau)

12. `classify_launcher_health(dead_count, window_secs) -> LauncherHealth` —
    fonction **pure**, trois états et jamais un `bool` :
    `Healthy` / `Braked { dead_count, since }` / `Unreadable`. Les deux derniers
    appellent des conduites opposées (« le lanceur est cassé, réparez l'hôte »
    contre « la base ne répond pas »), et un booléen ferait lire le second comme
    le premier (motif `HoldVerdict`, mika#2597).
13. `MIKA_PILOT_LAUNCHER_BRAKE_WINDOW_SECS` (défaut `3600`),
    `MIKA_PILOT_LAUNCHER_BRAKE_THRESHOLD` (défaut `2`, la lettre d'AC2),
    `MIKA_PILOT_LAUNCHER_BRAKE` (kill-switch, défaut armé). Les trois paliers
    maison : absent/vide → défaut ; illisible, `0` ou négatif → défaut **plus un
    WARN nommant la valeur entre guillemets**. Le `0` ne désarme pas — c'est le
    rôle du kill-switch, et un désarmement par coquille sur un frein de coût est
    la panne silencieuse que tout ceci ferme.
14. Surface A — `validate_dispatch_readiness` : une porte qui refuse quand
    `Braked`, **avant** toute résolution de jeton et tout appel `gh` (le
    placement de la porte 2c de mika#2279, pour la même raison : le prédicat ne
    lit rien d'autre que la base). Refus structuré + `record_dispatch_rejection`
    sous un motif nommé, dont le corps nomme **le geste de réparation** (`uv tool
    install --reinstall --force --editable ./claude-pilot`) et le levier de
    désarmement — un refus qui ne nomme pas sa levée est un refus qu'on contourne
    au jugé.
15. Surface B — `auto_pull::classify_stuck_ready` : un filtre qui rend `Skip`
    **sans** `SkipAndResetBudget`, placé **après** le bras `in_flight` pour que le
    cas nominal ne paie aucune requête (le placement que mika#2279 épingle par
    test).
16. `recovered` : écrit sur le site de U3 quand le subprocess finit **sans** le
    code dédié alors que la fenêtre portait au moins un `dead`. Coût : une
    requête SQL par dispatch — quelques unités par jour.

### U5 — AC3 : la documentation cesse d'affirmer le contraire

17. Rectifier le § mika#2532 de `crates/mika-agent/CLAUDE.md` : le fd 2 de
    `dispatch-lib.sh` est `/dev/null` depuis la ligne 9680, donc la capture
    moteur n'y a jamais rien eu à persister. Citer le commentaire de
    `_halt_family` qui le disait déjà.
18. Documenter dans le `CLAUDE.md` racine : le code 79, les surfaces, les régimes
    attendus, les sondes et leurs haltes, et ce que le travail n'achète pas.

### U6 — AC4 : les tests

19. **Le test central, et c'est lui qui doit être vu rouge avant le correctif.**
    Un faux lanceur écrit par le test lui-même (un script shell déposé dans un
    `tempdir`, sortant `79` sans écrire de journal) est dispatché par
    `spawn_long_running_exec` ; attendu : la ligne `pilot_launcher_dead` et la
    ligne d'audit `dead`.
20. **Contrôle positif indispensable** : un handler qui sort `1` **après** avoir
    écrit son journal reste un échec ordinaire — ni `pilot_launcher_dead`, ni
    ligne d'audit. Sans lui, « la garde décide » est indistinguable de « la garde
    accuse tout », et le test central serait satisfait par un code qui classe
    chaque échec.
21. **Contrôle négatif sur le code 78** : un faux lanceur sortant `78` n'est pas
    classé lanceur mort (R4).
22. Frein : `classify_launcher_health` aux trois bornes (1 `dead` → `Healthy`,
    2 → `Braked`, lecture en erreur → `Unreadable`), plus le refus de la porte et
    le `Skip`-sans-reset du filtre.
23. Shell : `skills/bundled/_shared/test-dispatch-lib.sh` — l'ordre
    trap-avant-smoke-test, la valeur du code de sortie, et le fait que le
    `RESULT` ne matche aucun motif retryable.

> **Contrainte d'exécution (consigne opérateur du 2026-10-02).** Le comportement
> de capture de stderr se vérifie **dans ces tests**, avec un faux lanceur écrit
> par le test. Pas de sonde au shell : `bash -c` / `sh -c` / `eval` sont des
> refus **terminaux** et le premier groom de ce ticket est mort là. L'édition se
> fait à l'outil `Edit`, jamais `sed -i` ni un script de réécriture.

---

## Contrat de vérification

| # | vérification | exécutable ici ? |
|---|---|---|
| V1 | `cargo test -p mika-agent` vert | oui |
| V2 | `cargo clippy --all-targets -- -D warnings` vert | oui |
| V3 | `cargo fmt --check` vert | oui |
| V4 | Le test U6-19 **vu rouge** avant le correctif, vert après | oui |
| V5 | Le contrôle positif U6-20 **vu vert** avec la garde armée | oui |
| V6 | `make test-dispatch-lib` (ou la cible équivalente) vert | oui |
| V7 | La ligne `pilot_launcher_dead` apparaît sur un vrai lanceur cassé | **non** — geste opérateur, sonde S1 |
| V8 | Le frein mord au deuxième, et se lève seul | **non** — geste opérateur, sonde S2 |

V7 et V8 ne sont pas exécutables depuis un bac à sable de dispatch : elles
demandent le journal du démon et la base, qui n'y est pas montée. Elles sont
déclarées comme sondes post-déploiement plutôt que comme contrat, et c'est la
seule lecture honnête.

---

## Fire-Disposition

Ce plan livre quatre détecteurs : le test comportemental U6-19, les deux scans de
source U3-10 / U3-11, et la garde de dispatch U4-14.

**Option retenue : (a) exception nommée en allowlist — avec zéro entrée.**

Détail d'implémentation :

- Les deux scans de source (U3-10, U3-11) sont livrés **armés**, avec leur
  allowlist **vide**, et un test frère épingle qu'elle le reste (motif
  `mika2496_the_sole_writer_allowlist_is_empty`). L'inventaire est vérifié avant
  l'armement : aucun site existant n'écrit `pilot_launcher_dead` ni
  `pilot_launcher_health`, et le littéral du code de sortie n'existe nulle part.
  **Quand un de ces scans tire, on route le site par le lecteur unique ; on
  n'ajoute pas de ligne** (doctrine mika#2201).
- Chaque scan porte son **assertion anti-vacuité** : il échoue si le nom qu'il
  cherche n'est écrit nulle part. Un scan qui vise un nom mort se lit exactement
  comme un arbre propre (classe mika#2205 / mika#2103).
- Chaque scan porte son **contrôle de bonne foi** : un second site injecté le
  fait rougir. Sans lui, « le scan regarde » est indistinguable de « le scan est
  inerte ».
- La garde de dispatch (U4-14) est livrée **armée** par défaut, avec son
  kill-switch `MIKA_PILOT_LAUNCHER_BRAKE=0`. L'argument est celui de mika#2272 :
  mika#2249 avait livré désarmé derrière une condition d'armement qui s'est
  révélée **insatisfaisable, pas seulement non remplie**, parce que la population
  scannée était vide par construction — *« zéro était l'absence de mesure, pas la
  présence de prudence »*. Ici le défaut est tier 1 et mesuré trois fois en une
  nuit. Ce qui paie la prudence est nommé et concret : le fail-open de D6, la
  fenêtre qui se lève seule de D5, le kill-switch, et le contrôle négatif de
  faux positif de la sonde S3.
- Aucun détecteur n'est livré avec `#[ignore]`. L'option (b) est écartée parce
  qu'aucun des quatre ne dépend d'un fournisseur réel ni d'un état d'hôte : tous
  tournent sur un faux lanceur et une base en mémoire.

---

## Surfaces opérateur

```bash
# 1. Un lanceur est-il mort au lancement ?
grep pilot_launcher_dead "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, code_display, task_was_terminal, stderr_bytes}'

# 2. Le frein a-t-il mordu ?
grep pilot_launcher_brake_engaged "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, issue, dead_count, window_secs}'

# 3. CONTRÔLE POSITIF — des dispatches ont-ils seulement eu lieu ?
grep -c long_running_handler_exit_nonzero "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- Les deux issues du même ledger, soustractibles en une requête
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'pilot_launcher_health' GROUP BY 1;

-- La population du lanceur mort, datée
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'pilot_launcher_health' AND after_value = 'dead'
 ORDER BY created_at DESC;
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `pilot_launcher_dead` | WARN | **vide** | chaque ligne est un dispatch mort au lancement ; deux dans l'heure et le frein s'arme |
| `after_value = 'dead'` | audit | **vide** | la même population, datée et comptable |
| `after_value = 'recovered'` | audit | **vide** | une ligne par réparation d'hôte — non vide est un **résultat**, pas une panne |
| `pilot_launcher_brake_engaged` | WARN | **vide** | le frein mord ; le corps nomme le geste de réparation |
| `pilot_launcher_health_unreadable` | WARN | **vide** | fail-open : le frein est inerte, la base ne répond pas |
| `pilot_launcher_dead_audit_failed` | WARN | **vide** | le WARN est passé, la ligne d'audit non — le `GROUP BY` sous-compte |

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** `skills/bundled/` est une projection du **binaire**, pas du
> checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha
> qu'on vient de bâtir, **et** le `mika-spirit` servi doit porter le correctif —
> sans ces deux vérifications, chacune des sondes ci-dessous rend un résultat qui
> décrit le binaire d'hier. Ce sont des **gestes d'opérateur** sur l'hôte : la
> base n'est pas montée dans le bac à sable de dispatch.

**S1 — le défaut fondateur ne se rejoue pas (prochain lanceur cassé).** Casser
délibérément le lanceur sur un hôte de test, dispatcher : une ligne
`pilot_launcher_dead`, une ligne d'audit `dead`, et le `tasks.result` du callback
porte le diagnostic mika#1200 **et** le `ModuleNotFoundError`.
*Halte 1 — aucune ligne alors que le dispatch est mort :* **ne pas élargir le
prédicat par réflexe.** Établir d'abord le déploiement (préalable ci-dessus),
puis lire le **contrôle positif** (sonde 3). Zéro ligne des deux côtés ne prouve
rien : *une garde que personne n'a exercée se lit exactement comme une garde qui
marche* (mika#2205).

**S2 — le frein mord au deuxième, et se lève seul.** Deuxième dispatch dans la
fenêtre : refusé, `pilot_launcher_brake_engaged`, et le ticket **n'est pas**
abandonné (le budget de re-drive mika#2020 ne bouge pas). Lanceur réparé : le
dispatch suivant passe sans qu'aucun geste n'ait été posé, et une ligne
`recovered` est écrite.
*Halte 2 — le ticket est abandonné en `operator-review` :* D8 n'a pas pris, le
filtre `auto_pull` ne lit pas le ledger. **Désarmer d'abord**
(`MIKA_PILOT_LAUNCHER_BRAKE=0`), diagnostiquer ensuite — un frein qui abandonne
des tickets sains est pire que le gaspillage qu'il remplace.

**S3 — contrôle négatif de faux positif (7 jours).** Aucun
`pilot_launcher_dead` sur un dispatch nominal, et **en particulier aucun** sur un
refus de confinement (exit 78), un `auto_skipped`, un dry-run ou un
`PIPELINE_INCOMPLETE`.
*Halte 3 — une occurrence :* c'est R4 qui se réalise, donc le classement ne lit
pas le code dédié mais une inférence. **Désarmer d'abord**, réparer le prédicat
ensuite : un refus de dispatch sur une population saine gèle la boucle.

**S4 — la boucle n'est pas gelée (7 jours).** `long_running_handler_exit_nonzero`
continue d'apparaître à son rythme, et des dispatches aboutissent.
*Halte 4 — plus aucun dispatch alors que `pilot_launcher_dead` est vide :* le
frein est armé sur une lecture fausse. Le kill-switch d'abord, le prédicat
ensuite.

**S5 — le marqueur terminal ne rejoue pas (30 jours).** Aucun ticket re-dispatché
en boucle à la suite d'un `Outcome: LAUNCHER_DEAD`.
*Halte 5 — un rejeu :* la branche de `self-dev-callback` n'est pas atteinte, ou
elle est placée **après** la routine `PIPELINE FAILURE:`. C'est l'ordre du
discriminateur qu'il faut lire, pas le prédicat — la leçon mika#2545 à la lettre.

---

## Ce que ce travail n'achète PAS

- **Il ne répare aucun lanceur.** Le shebang est un fait d'hôte ; la preflight de
  `make deploy` et la sonde de surveillance sont traitées par MPC dans le
  méta-dépôt, et le ticket les met hors périmètre en toutes lettres.
- **Il ne rattrape pas l'incident du 2026-10-02.** Les trois tâches mesurées
  restent ce qu'elles sont et **rien ne rétro-écrit** une ligne d'audit datée
  d'un fait qu'on n'a pas observé — ce serait l'inverse de ce que ce travail
  défend. La sonde est la **prochaine** occurrence.
- **Il ne couvre pas un lanceur qui passe le smoke test et meurt au lancement
  réel.** Population nommée et **non couverte** : `claude-pilot --help` réussit,
  puis le vrai lancement meurt sans journal (un drapeau refusé, la classe
  mika#2043). Le canal y est différent — le trap est armé, le callback part, et
  le signal doit vivre dans le `RESULT`. **Ticket de suivi**, précondition : une
  mesure montrant une occurrence.
- **Il ne retire pas le `2>/dev/null` permanent de la ligne 9680.** Le réflexe
  serait de le réparer (`{ exec 9>>"$TRACE_FILE"; } 2>/dev/null || …`) pour que
  tout le stderr du handler remonte au moteur. **Refusé ici, et la raison est un
  risque, pas une préférence :** `spawn_long_running_exec` monte le stderr sur un
  `Stdio::piped()` que l'exécuteur ne lit **qu'après** `child.wait()`. Un pipe a
  une capacité d'environ 64 Ko ; un handler qui écrirait plus que ça sur fd 2
  **bloquerait** sur l'écriture, indéfiniment. Le `/dev/null` masquait peut-être
  ce risque par accident, et le lever demande de décider où va ce stderr (un
  fichier nommé par la tâche — mais `TASK_ID` n'est pas connu à la ligne 9680).
  **Ticket de suivi**, avec cet arbitrage pour corps.
- **Il n'ajoute aucun canal push vers le veilleur** (D7).
- **Il rend le champ lisible, pas surveillé.** Les seuls instruments sont les
  greps et les requêtes ci-dessus, et **leur silence ne prouve rien tant que
  personne ne les exécute.**

---

## Hors périmètre, délibérément

- **La preflight `make deploy` et la sonde de shebang** : le ticket les attribue
  à MPC dans le méta-dépôt.
- **Le `2>/dev/null` permanent** : suivi nommé ci-dessus.
- **`probe_pilot_log_signal` et le faucheur mika#2249/#2277** : inchangés.
  D1 existe précisément pour ne pas créer un second lecteur de ce chemin.
- **Le refus de confinement exit 78** (mika#2141 / mika#2049) : inchangé, et
  explicitement exclu de la population (R4, sonde S3).
- **`MAX_OUTPUT_LEN` et la persistance mika#2532** : aucun octet touché. Le
  mécanisme fonctionne ; ce qui change est qu'il aura désormais quelque chose à
  persister sur ce chemin.
- **Le budget de tours `PILOT_MAX_TURNS` et le seuil de coût** (mika#2496) :
  aucune valeur déplacée.
- **La garde anti-zombie mika#1742 et ses exemptions** : aucun contact.

---

## Definition of Done

- [ ] U1 à U6 implémentés, avec les décisions D1-D8 respectées et citées aux
      sites concernés.
- [ ] V1 à V6 verts.
- [ ] Le test U6-19 a été **vu rouge** avant le correctif (la phrase est dans le
      corps de PR, avec la sortie).
- [ ] Les contrôles U6-20 et U6-21 ont été **vus verts** avec la garde armée.
- [ ] Les deux allowlists de scan sont livrées **vides** et un test frère
      l'épingle.
- [ ] `crates/mika-agent/CLAUDE.md` § mika#2532 rectifié (U5-17).
- [ ] `CLAUDE.md` racine documente le code 79, les six surfaces, les cinq sondes
      et leurs haltes, et le § « ce que ce travail n'achète pas » (U5-18).
- [ ] Le corps de PR nomme les deux tickets de suivi (lanceur mort après smoke
      test ; `2>/dev/null` permanent) et déclare V7/V8 comme sondes opérateur
      plutôt que comme contrat vérifié.

---

## Acceptance criteria

Transcrits du corps de mika#2634, avec la rectification de forme que D1 impose à
AC1 et celle que R2/R3 imposent à AC3 — les deux sont nommées, pas silencieuses.

- [ ] **AC1.** Quand un `long_running:run_claude_pilot` meurt parce que le
      lanceur ne démarre pas, le moteur émet un événement distinct,
      `pilot_launcher_dead`, plus une ligne `audit_events`, et l'événement n'est
      pas noyé dans les échecs ordinaires (la ligne générique
      `long_running_handler_exit_nonzero` est conservée à côté, intacte).
      *Rectification de forme (D1, R4) : le discriminant est le code de sortie
      dédié estampillé par le producteur, non l'inférence « exit non nul + aucun
      journal », qui a quatre faux positifs mesurables dans le dépôt et
      introduirait un second lecteur du chemin de journal.*
- [ ] **AC2.** À la deuxième occurrence dans la fenêtre, le moteur cesse de
      dispatcher des pilotes et le dit, au lieu de brûler les fenêtres une à une.
      Le refus est convergent : il ne consomme pas le budget de re-drive, et il
      se lève de lui-même quand l'hôte est réparé.
- [ ] **AC3.** La cause est capturée et persistée : le callback porte le
      diagnostic prescriptif mika#1200 **et** la trace du `ModuleNotFoundError`,
      lisibles par `mika tasks get <id>`.
      *Rectification de forme (R2, R3) : le canal n'est pas le stderr du handler
      — son fd 2 est `/dev/null` depuis la ligne 9680, ce que le dépôt écrivait
      déjà sans en tirer la conséquence. Le canal est le callback, que le
      déplacement du trap rend atteignable, et la cause y était déjà écrite sur
      fd 9.*
- [ ] **AC4.** Un faux lanceur qui sort le code dédié sans écrire de journal
      déclenche `pilot_launcher_dead` (**vu rouge** avant le correctif) ; un
      handler qui sort 1 après avoir écrit son journal reste un échec ordinaire
      (contrôle positif) ; un exit 78 n'est pas classé lanceur mort (contrôle
      négatif de confinement).

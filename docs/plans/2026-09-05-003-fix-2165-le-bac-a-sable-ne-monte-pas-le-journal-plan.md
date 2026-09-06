---
issue: 2165
repo: senara-solutions/mika
type: fix
module: skills/bundled/_shared/dispatch-lib.sh
tags: [containment, sandbox, bwrap, observability, loop-substrate, dispatch]
problem_type: missing-bind-mount
status: draft
---

# mika#2165 — le bac à sable ne monte pas le répertoire de journal

**Issue :** senara-solutions/mika#2165
**Branche :** `fix/2165/dispatch-containment-le-bac-sable-ne`
**Palier :** Tier 1 — brise la boucle (supprime les données de diagnostic de deux briseurs ouverts,
et désarme silencieusement une porte de qualité de `dev-groom`).
**Frère :** mika#2141 (même confinement, même classe, même date) — corrigé par mika#2146.

---

## 1. Le fait, relu dans le code

### 1.1 Le `.stderr` survit, le `.log` non — et pourquoi

`dispatch-lib.sh:2145` écrit le `.stderr` **depuis l'hôte**, après retour du bac à sable :

```bash
PERSISTENT_STDERR="/var/log/claude-pilot/${LOG_ID}.stderr"
```

Le `.log`, lui, est écrit **depuis l'intérieur**. `dispatch-lib.sh:2182` :

```bash
_run_pilot_sandboxed claude-pilot --verbose --log-dir --task-id "$LOG_ID" ...
```

`--log-dir` est passé **nu**. `cli.py:49-53` lui donne alors son `const` :

```python
p.add_argument("--log-dir", dest="log_dir", nargs="?",
               const="/var/log/claude-pilot", default=None)
```

et `cli.py:216-219` ouvre `/var/log/claude-pilot/<task-id>.log` — un chemin absolu, résolu
à l'intérieur du namespace.

### 1.2 Le montage manquant, énuméré

Les deux invocations `bwrap` (`dispatch-lib.sh:1013-1054` chemin contenu-réseau, et
`dispatch-lib.sh:1096-1137` chemin repli fs-seul) partagent la même liste. Les seuls montages
**en écriture** y sont :

| ligne (1er bloc) | montage rw |
|---|---|
| 1032-1035 | `--tmpfs /tmp`, `--tmpfs /var/tmp`, `--tmpfs /run`, `--tmpfs /home` |
| 1036 | `--bind "$WORKTREE_DIR"` |
| 1037 | `${_PILOT_GITDIR_BIND_ARGS[@]}` (gitdir, mika#2141) |
| 1054 | `--bind "$HOME/.mika/data/pilot-transcripts"` |
| 856 | `--bind "$_PILOT_EGRESS_SOCK"` |

`/var/log/claude-pilot` n'y est pas. Il n'y a pas non plus de `--ro-bind /var` : `/var` n'existe
dans le namespace que parce que `--tmpfs /var/tmp` en matérialise le parent.

### 1.3 Pourquoi l'échec est muet — et pourquoi AC3 tel qu'écrit ne l'aurait pas vu

C'est le point décisif du ticket, et il est plus subtil que ce que le corps suppose.

`logger.py:26-36` :

```python
try:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    _file = path.open("a", encoding="utf-8")
    ...
except OSError as err:
    _report_error(err)       # logger.py:35
    _file = None
```

et `_report_error` (`logger.py:73-77`) **écrit déjà sur stderr** :

```python
sys.stderr.write(f"Warning: log file write error: {err}\n")
```

Donc :

1. **AC3 tel que formulé est déjà implémenté** — `logger.py:34-35` + `logger.py:73-77`.
2. **Et il n'aurait rien attrapé.** La racine du bac à sable bwrap est un `tmpfs` inscriptible :
   `mkdir(parents=True)` **réussit**, `open("a")` **réussit**, chaque `write`+`flush` **réussit**.
   Aucune `OSError` n'est levée. Le journal n'échoue pas à s'écrire : il s'écrit **correctement,
   dans un système de fichiers qui disparaît** au démontage du conteneur.

Le silence n'est pas un défaut de rapport d'erreur. C'est l'absence, côté hôte, de toute
vérification que le fichier promis existe après coup. Le remède doit donc vivre **là où le
`.stderr` vit** — sur l'hôte, hors du bac à sable — et pas dans `init_file_log`.

Cette divergence est portée au point de réconciliation (§ 6).

### 1.4 Une troisième victime, non nommée par le ticket

`dispatch-lib.sh:3131-3142`, chemin `dev-groom` :

```bash
SESSION_LOG="${PILOT_LOG_DIR:-/var/log/claude-pilot}/${LOG_ID}.log"
if [ -f "$SESSION_LOG" ] && [ -r "$SESSION_LOG" ]; then
    grep -qiE 'ce[.:\-_]plan' "$SESSION_LOG" && CE_PLAN_INVOKED="1"
else
    echo "Warning: session log not available at $SESSION_LOG — skipping ..." >&2
    CE_PLAN_INVOKED="unknown"
fi
```

Cette porte (mika#1032 — « le pilote a-t-il vraiment appelé `/ce:plan` ? ») est **ouverte en
grand depuis le 2026-08-04**. Elle est *fail-open* par conception, ce qui est correct ; mais
son repli était censé être rare, et il est devenu l'unique chemin. Le ticket compte deux
pertes ; il y en a trois, et la troisième est une porte de qualité, pas une trace.

### 1.5 Une incohérence de chemin déjà présente

Trois épellations coexistent pour le même répertoire :

| site | forme | conséquence |
|---|---|---|
| 2145 | `/var/log/claude-pilot` **en dur** | ignore `PILOT_LOG_DIR` |
| 2389, 2594, 2979, 3131, 3152 | `${PILOT_LOG_DIR:-/var/log/claude-pilot}` | surchargeable |
| 2182, 4415 | `--log-dir` **nu** → `const` Python | ignore `PILOT_LOG_DIR` |

`PILOT_LOG_DIR` est donc une surcharge **à moitié câblée** : elle déplace les *lecteurs* sans
déplacer les *écrivains*. La corriger n'est pas du confort — c'est ce qui permet à AC2 de se
tester sans écrire dans la surface opérationnelle de diagnostic, exactement la précaution que
`MIKA_PILOT_EGRESS_LOG_DIR` a déjà posée à `dispatch-lib.sh:385-392` après mika#2041.

---

## 2. Ce que ce plan livre

### D1 — Résoudre le répertoire de journal en un seul point (prérequis d'AC1)

Ajouter, près des autres constantes du bac à sable, un résolveur unique :

```bash
# mika#2165 : un seul point de vérité pour le répertoire de journal du pilote.
# Écrit par l'INTÉRIEUR du bac à sable (cli.py:216-219) et lu par l'hôte
# (post-flight, dev-groom, callback). Les trois doivent nommer le MÊME chemin,
# sinon le bind couvre un répertoire que le pilote n'écrit pas.
_PILOT_LOG_DIR="${PILOT_LOG_DIR:-/var/log/claude-pilot}"
```

Remplacer les trois formes de § 1.5 par `$_PILOT_LOG_DIR` :
- `dispatch-lib.sh:2145` (`PERSISTENT_STDERR`)
- les cinq lecteurs `${PILOT_LOG_DIR:-...}` (2389, 2594, 2979, 3131, 3152)
- les deux invocations pilote : `--log-dir "$_PILOT_LOG_DIR"` (2182 et 4415), **valué** au lieu
  de nu. `nargs="?"` accepte la forme valuée sans changement côté Python.
- les deux chaînes de rapport `RESULT="Log path: /var/log/claude-pilot/..."` (2243, 2253).

**Invariant nommé, à inscrire en commentaire :** `le chemin passé à --log-dir == le chemin
bindé == le chemin lu en post-flight`. Un futur contributeur qui touche l'un des trois doit
toucher les trois — même discipline que le bloc gitdir de mika#2141
(`dispatch-lib.sh:108` : « change the binds and this text in the same commit »).

### D2 — Le montage (AC1, AC4)

Un helper qui prépare le répertoire côté hôte et émet l'argument :

```bash
# mika#2165 : rend le répertoire de journal visible DEPUIS l'intérieur.
# Le plus étroit possible : ce répertoire, pas /var/log — /var/log/mika (le
# journal du proxy d'egress, surface de diagnostic d'incident, mika#2041)
# doit rester hors d'atteinte du pilote.
_pilot_log_bind_args() {
    _PILOT_LOG_BIND_ARGS=()
    if [ ! -d "$_PILOT_LOG_DIR" ] && ! mkdir -p "$_PILOT_LOG_DIR" 2>/dev/null; then
        echo "dispatch-lib: pilot_log_guard.unmountable $_PILOT_LOG_DIR n'existe pas et ne peut être créé — la session ne laissera pas de .log" >&2
        return 0
    fi
    if [ ! -w "$_PILOT_LOG_DIR" ]; then
        echo "dispatch-lib: pilot_log_guard.unwritable $_PILOT_LOG_DIR n'est pas inscriptible — la session ne laissera pas de .log" >&2
        return 0
    fi
    _PILOT_LOG_BIND_ARGS=(--bind "$_PILOT_LOG_DIR" "$_PILOT_LOG_DIR")
}
```

Câblé dans **les deux** blocs `bwrap`, à côté du bind `pilot-transcripts` (1054 et 1137) :

```bash
    ${_PILOT_LOG_BIND_ARGS[@]+"${_PILOT_LOG_BIND_ARGS[@]}"} \
```

*Ouvrir la liste plutôt que binder inconditionnellement* : un `--bind` dont la source manque
fait échouer `bwrap` en entier. Le défaut d'aujourd'hui perd un journal ; un bind rigide
perdrait la session. Le repli est *bruyant* — c'est la moitié hôte d'AC3.

**Décidé, pas laissé ouvert.** Le bind est **rw**, pas `--bind-try`, et pas un sous-répertoire
par tâche. Motif : le pilote crée son propre fichier (`open("a")`), donc il lui faut le droit
d'écriture *sur le répertoire* ; et le post-flight hôte doit relire ce fichier au même chemin.
Un sous-répertoire par tâche ajouterait une épellation de plus au trio de D1 sans rien fermer.

### D3 — La détection hôte (AC3, reformulé)

Après le retour du pilote, au voisinage de la persistance du `.stderr` (`dispatch-lib.sh:2186-2188`) :

```bash
# mika#2165 : un journal absent ne peut plus être silencieux. Le .stderr est
# écrit par l'hôte et survit ; c'est donc lui qui porte l'aveu.
_SESSION_LOG_PATH="$_PILOT_LOG_DIR/${LOG_ID}.log"
if [ ! -s "$_SESSION_LOG_PATH" ]; then
    echo "dispatch-lib: pilot_log_guard.missing $_SESSION_LOG_PATH absent ou vide après la session — le montage du répertoire de journal est peut-être tombé (mika#2165)" | tee -a "$PERSISTENT_STDERR" >&2
fi
```

Placé **après** l'écriture de `$PERSISTENT_STDERR` (2188), pour que la ligne s'ajoute au fichier
persisté plutôt que d'être écrasée par lui.

`-s` et non `-f` : un `.log` créé puis vide est le même échec observable qu'un `.log` absent.

**Ce que ce livrable ne fait pas, et pourquoi.** Il ne touche pas `logger.py`. Le rapport
d'erreur qu'AC3 demande y existe déjà (§ 1.3) et n'est pas atteint par ce défaut. Toute
amélioration côté pilote — par exemple que `init_file_log` annonce sur stderr le chemin qu'il a
résolu — vit dans `senara-solutions/claude-pilot`, pas ici, et fait l'objet d'un ticket séparé
si Vincent le veut (§ 6, divergence 2).

### D4 — Le test de non-vacuité (AC2, AC4)

Nouveau `skills/bundled/_shared/tests/test_sandbox_log_dir_bound.sh`, sur le patron exact de
`test_sandbox_git_usable.sh` (mika#2141) : **un vrai `bwrap`, à travers le vrai
`_run_pilot_sandboxed`**, jamais une inspection d'argv.

Le test pointe `PILOT_LOG_DIR` sur un `mktemp -d`, pour ne pas polluer la surface
opérationnelle — la précaution de `dispatch-lib.sh:385-392`.

| moitié | assertion |
|---|---|
| **DOIT MARCHER** | un `echo` depuis l'intérieur vers `$PILOT_LOG_DIR/<id>.log` est lisible **depuis l'hôte** après retour |
| **DOIT MARCHER** | le répertoire est listable et créable-dedans depuis l'intérieur |
| **DOIT ÉCHOUER** | écrire dans `/var/log` (le parent) |
| **DOIT ÉCHOUER** | écrire dans `/var/log/mika/pilot-egress-proxy.log` — cible adjacente réelle, prouvée existante côté hôte d'abord |
| **DOIT ÉCHOUER** | écrire dans `/var/lib`, `/srv` |

Les deux moitiés dans le **même** lancement : seule, chacune est vide de sens
(`--ro-bind /var` passerait tous les « doit marcher » ; un `bwrap` mort passerait tous les
« doit échouer »). C'est le raisonnement inscrit en tête de `test_sandbox_git_usable.sh:17-19`.

**Contrôle négatif exigé par AC2 :** le test doit redevenir rouge si le bind est retiré. Vérifié
en le lançant une fois avec `_PILOT_LOG_BIND_ARGS` neutralisé, et le résultat des deux passes
(vert avec, rouge sans) est reporté dans le commentaire de PR — pas seulement affirmé.

Câblage : ajouter la ligne à `mika/Makefile` au voisinage de 136-138, dans les **deux** cibles
qui listent déjà ces tests (136-138 et le bloc miroir vers 156-159).

### D5 — L'audit des autres chemins absolus (AC5)

**Déjà effectué pendant la préparation de ce plan ; le livrable est de l'inscrire, pas de le refaire.**

Recherche sur `claude-pilot/src/claude_pilot/` de tout puits de fichier à chemin absolu :

| écrivain | cible | statut |
|---|---|---|
| `logger.py:27-28` (`mkdir` + `open("a")`) | `$log_dir/<task-id>.log` | **le défaut** — fermé par D2 |
| `ANTHROPIC_LOG_FILE` (mika#1705, allowlist `dispatch-lib.sh:449`) | `~/.mika/data/pilot-transcripts/<id>.jsonl` | **monté** (1054, 1137) |
| `heartbeat.py:124`, `inbox_writer.py:147`, `permission_events.py:308` | HTTP `urlopen` | pas de système de fichiers |
| `notify.py:24` | `subprocess.Popen` | pas de système de fichiers |
| proxy d'egress, `dispatch-lib.sh:389-396` | `/var/log/mika/pilot-egress-proxy.log` | écrit par le démon **hôte** (`nohup`, ligne 396) ; l'instance in-sandbox (1052) redirige `>&2` → capturée dans le `.stderr` |

Les autres occurrences de `/tmp/` et `/var/log` dans `tier1.py` / `permissions.py` sont des
*motifs de classification*, pas des écritures.

**Conclusion : il n'y a pas de troisième trou.** Inscrire ce tableau dans le bloc de commentaire
d'en-tête du bac à sable (`dispatch-lib.sh:60-72`), à l'endroit où la liste des montages est
déjà tenue à jour — c'est là qu'un futur ajout d'écrivain sera relu.

---

## Acceptance criteria

Transcrits de mika#2165, avec renvoi au livrable qui les porte. AC3 est transcrit tel qu'écrit ET
signalé comme divergence (voir § 1.3 et § 6) : le plan soutient que le message d'erreur exigé
existe déjà (`logger.py:34-35`) et reformule AC3 en une détection *hôte* (D3) qui, elle, survit.

- **AC1** — Le répertoire du journal de session est monté en écriture dans le bac à sable, le plus
  étroit possible (ce répertoire, pas `/var/log`), de sorte que `/var/log/claude-pilot/<task-id>.log`
  soit à nouveau écrit. → D1 (résolution en un point) + D2 (le montage).
- **AC2** — Une session dispatchée produit **les deux** fichiers `.stderr` et `.log`, et le `.log`
  contient la ligne `[prompt]` ; test de non-vacuité rouge si le montage est retiré. → D4.
- **AC3** — L'échec d'écriture du journal ne peut plus être silencieux : une ligne part sur
  `stderr` (écrit par l'hôte, survivant). → D3 (reformulé : détection hôte ; voir § 1.3 pour
  pourquoi la moitié interne est déjà présente).
- **AC4** — Non-régression de confinement : le montage n'ouvre l'écriture qu'à ce répertoire
  nommé ; la surface reste celle de mika#1894, à un répertoire près. → D2 + D4.
- **AC5** — Audit des autres chemins absolus écrits par le pilote, pour qu'aucun troisième ne
  tombe dans le même trou (`pilot-transcripts` est monté ; confirmer qu'il n'en manque pas). → D5.

## Fire-Disposition

Détecteurs livrés (gate mika#1574), disposition pré-spécifiée :

- **D3 — détection hôte d'un journal absent (AC3).** Tir sur : condition runtime (le pilote n'a
  pas pu écrire son `.log`). Disposition : **surface-not-halt** — une ligne d'aveu part sur le
  `.stderr` hôte (survivant) ; la session continue (perdre le journal ne doit pas tuer le travail),
  mais la perte cesse d'être muette. Non destructif.
- **D4 — test de non-vacuité (AC2, AC4).** Tir sur : le diff / la CI. Disposition : **gate CI
  bloquant** — le test échoue si le `.log` ou sa ligne `[prompt]` manque, et redevient rouge si le
  montage est retiré (contrôle négatif exigé). Pas de remédiation auto.
- **D5 — audit des chemins absolus (AC5).** Tir sur : la revue. Disposition : **halt-and-surface**
  si un troisième chemin non monté est trouvé — le nommer et décider (l'ajouter au périmètre ou
  ficher un frère), ne pas l'absorber en silence.

## 3. Ce que ce plan ne fait pas

- **mika#2029** (sessions mort-nées, `0 content stream events`) — hors périmètre, comme le dit
  le ticket. Ce plan rend son diagnostic possible ; il ne le fait pas.
- **claude-pilot#151** (létalité des refus de permission) — idem.
- **mika#2146** (montage du gitdir) — déjà corrigé.
- **`logger.py`** — voir D3. Rien ne change dans `senara-solutions/claude-pilot`.
- **La porte `/ce:plan` de `dev-groom`** (§ 1.4) — elle se referme d'elle-même dès que le `.log`
  revient. Aucun changement de code ne lui est destiné ici ; le plan la nomme pour qu'on sache
  qu'elle était ouverte, et pour qu'on la vérifie refermée au § 5.

---

## 4. Séquence

| # | pas | fichier |
|---|---|---|
| 1 | D1 — `_PILOT_LOG_DIR` + les huit sites unifiés | `dispatch-lib.sh` |
| 2 | D2 — `_pilot_log_bind_args` + câblage dans les deux blocs `bwrap` | `dispatch-lib.sh` |
| 3 | D3 — garde post-flight `pilot_log_guard.missing` | `dispatch-lib.sh` |
| 4 | D5 — tableau d'audit dans le commentaire d'en-tête | `dispatch-lib.sh` |
| 5 | D4 — le test, ses deux moitiés, et le contrôle négatif | `tests/test_sandbox_log_dir_bound.sh` |
| 6 | D4 — câblage Makefile (deux cibles) | `Makefile` |
| 7 | vérification vivante (§ 5) | — |

Pas 1 avant pas 2 : binder un chemin que les invocations n'utilisent pas serait vert et vide.

---

## 5. Vérification — sur l'état vivant, pas seulement en test

Le test de D4 prouve le mécanisme. Il ne prouve pas que la boucle réelle en profite. Les deux
sont exigés avant de déclarer le ticket clos.

1. `make -C mika deploy` puis **dispatcher une vraie session** (n'importe quel ticket `ready`).
2. `ls -la /var/log/claude-pilot/<task-id>.{log,stderr}` — **les deux** présents.
3. `grep -c '^\[prompt\]' /var/log/claude-pilot/<task-id>.log` ≥ 1 — c'est le contrôle positif
   du ticket (le `.log` du 2026-08-04 en contient 1).
4. `grep -c '\[relay:payload\]' …log` ≥ 1 sur une session ayant vu au moins un appel d'outil.
5. **Contrôle négatif du ticket, rejoué :** les 20 derniers `.stderr` ne contiennent toujours
   pas `[prompt]` — la trace est *revenue* dans le `.log`, elle n'a pas *déménagé*.
6. § 1.4 refermé : le `.stderr` d'une session `dev-groom` ne contient plus
   `session log not available`.

**À ne pas conclure trop tôt.** Une seule session verte ne clôt pas la mesure : le ticket décrit
une coupure franche sur **20 sessions consécutives**. Le seuil de clôture est **deux sessions
dispatchées, dans deux runs distincts**, produisant chacune son `.log` non vide — n=2, la
discipline que ce dépôt applique déjà à toute réparation de substrat.

---

## 6. Point de réconciliation — divergences corps ↔ plan

Deux divergences, portées à l'opérateur avant toute dépense d'architecte.

**Divergence 1 (prémisse, AC3).** Le corps demande « si `init_file_log` ne peut pas créer son
fichier, une ligne part sur `stderr` ». Le code lit : `logger.py:34-35` appelle déjà
`_report_error(err)` sur `OSError`, et `logger.py:73-77` écrit déjà
`Warning: log file write error: {err}` sur `stderr`. **C'est déjà là.** Et ce chemin n'est pas
celui du défaut : la racine bwrap est un `tmpfs` inscriptible, donc `mkdir`, `open` et `write`
réussissent tous — aucune `OSError` n'est levée. Un implémenteur qui suit AC3 à la lettre
trouvera le code en place, ne livrera rien, et le silence restera. D3 reformule AC3 en une
garde **côté hôte** (`pilot_log_guard.missing`), là où le `.stderr` vit déjà.

**Divergence 2 (périmètre / dépôt).** Le ticket est déposé sur `senara-solutions/mika`. AC1,
AC2 et AC4 y vivent bien (`dispatch-lib.sh`). Mais AC3 tel qu'écrit vise
`src/claude_pilot/logger.py`, qui appartient à `senara-solutions/claude-pilot` — un autre dépôt.
Sous la règle du plan de travail (« les branches, tickets et PR vont sur le dépôt où vit le
code »), AC3 dans sa forme actuelle **ne peut pas être clos par une PR sur `mika`**. La
reformulation de D3 résout aussi ce point : elle ramène AC3 dans `mika`, où le remède efficace
se trouve de toute façon.

*Note connexe, non bloquante :* `CLAUDE.md` nomme encore ce dépôt `claude-pilot-py/` ; sur
disque et sur GitHub il s'appelle `claude-pilot`. Sans effet sur ce plan.

**Chemins de résolution.** (1) Éditer AC3 dans le corps pour qu'il dise « la disparition du
journal ne peut plus être silencieuse : l'hôte constate l'absence du `.log` après la session et
l'écrit dans le `.stderr` », puis relancer `/mika-groom-ticket`. (2) Éditer le plan pour suivre
AC3 à la lettre — non recommandé : le livrable serait un no-op. (3) Trancher à la main.

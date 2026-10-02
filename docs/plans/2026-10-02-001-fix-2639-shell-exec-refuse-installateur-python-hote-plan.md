# `shell-exec` refuse d'installer dans l'environnement Python de l'hôte (mika#2639)

**Ticket :** senara-solutions/mika#2639 — labels `agent-core`, `dispatch:loop`.
Source prouvée de l'incident mika#2634 (2 h 45 de rail mort).

> **Classe :** un outil d'agent qui peut réécrire un binaire de production de
> l'hôte, et dont la seule protection tenait par accident (PEP 668) jusqu'à ce
> que l'agent apprenne à la contourner.
> **Prolongement direct :** mika#1957 (scan lexical `gws`/`gh` dans `run.sh`) et
> mika#1991 (containment d'egress au même site). Troisième scan du même fichier,
> même posture, même contrat de commentaire.
> **Voisins à ne pas confondre :** mika#2423 refuse sur le **budget**, mika#2449
> sur la **cible git**. Celui-ci refuse sur l'**environnement Python visé**.
> Aucun des trois ne subsume les autres — § M2.

---

## Constat

Le 2026-10-02 à 01:26:08/15/18Z, mika-qa a exécuté par `run_shell` (skill
`shell-exec`, **hors bwrap**), pendant la revue de cpp#273 :

```
R=~/workspace/mika-platform/claude-pilot; W=$(mktemp -d); git -C "$R" worktree add --detach "$W" origin/fix272-pilot-scratch-var-cpp 2>&1 && cd "$W" && pip install -e . -q --break-system-packages 2>&1 | tail -3 && python -m pytest … ; git -C "$R" worktree remove --force "$W"
```

`pip install -e` d'un dépôt dont `[console_scripts]` déclare `claude-pilot`
**réécrit `~/.local/bin/claude-pilot`** avec le shebang du Python système. Chaque
pilote meurt alors au lancement. Trace disque datée de 01:26:20Z (`dist-info`,
`INSTALLER=pip`, `direct_url = file:///tmp/tmp.Eqlt2PdYwK`).

Sur 48 h : **43** `run_shell` de mika-qa contiennent `pip install`, dont 19
visant claude-pilot ; **5** portaient `--break-system-packages` et ont pu écrire.
Les 38 autres ont été refusées par le pip système de Gentoo (PEP 668) — *la
protection de l'hôte tenait par accident, pas par conception.* Le commentaire
opérateur du 2026-10-02T08:32:30Z ajoute une seconde forme mesurée : la revue de
cpp#280 a lancé `cd /tmp/cpp280-review && uv pip install -e . …` à 08:20:37Z.

Sept mesures relevées sur l'arbre au `395ec6e5` déplacent le ticket avant de
l'exécuter. **Les nommer est un livrable, pas une note de bas de page.**

### M1 — Aucun prescripteur du dépôt ne dit `pip install`. AC2 POSE un fait, elle n'en corrige aucun

Recherche exhaustive de `pip install`, `pipx install`, `uv pip` sur
`skills/bundled/`, `crates/mika-agent/templates/skills/`, `.claude/`, `docs/`,
`scripts/`, `Makefile`, `.github/` : **une seule occurrence**, dans
`docs/plans/2026-03-19-001-…-plan.md` — un document historique, aucun
prescripteur, hors de toute population de lint. `qa-review/system_prompt.md` ne
contient ni `pip install`, ni `uv run`, ni `pytest`.

Les 43 commandes mesurées ont donc été **composées par le modèle**, dans un vide
de prescription. Conséquence pour AC2 : il n'y a pas d'instruction fautive à
réécrire, il y a une recette **absente** à poser. Même rectification que
mika#2565 R1 (`pipeline-exempt` n'exemptait rien) et mika#2290 (aucune persona ne
revendiquait « local » — il y avait un fait à poser).

### M2 — Une garde de la même famille existe déjà, et son prédicat porte sur le BUDGET

`qa-review/system_prompt.md:331` dit déjà **« Never compile inside the review
turn (mika#2276) »** et cite la garde moteur `build_command_exceeds_tool_budget`
(mika#2423, `skills/executor.rs::refuse_uncontainable_build`). Elle refuse une
commande de build avant le spawn, elle est en Rust, elle a un parseur
d'instructions mûr (`shell_instructions`, `matched_build_family`,
`is_env_assignment`) et une table `BUILD_COMMAND_FAMILY` de couples
`(binaire, sous-commandes)`.

**Elle n'a pas attrapé ces 43 commandes parce que sa table ne connaît que
`cargo`/`npm`/`npx`/`make`/`go`.** Le réflexe est donc d'y ajouter
`pip`/`uv`/`pipx` — et c'est **écarté**, pour une raison qui n'est pas de style :

```rust
if timeout_secs >= BUILD_FLOOR_SECS { return None; }   // executor.rs:865
if let ToolHandler::Exec { long_running: true, .. }
 | ToolHandler::Exec { detaches_command: true, .. } = … { return None; }
```

Son prédicat est **conditionné au budget** et exempte par construction les
handlers `long_running` et `detaches_command`. Or `pip install` dans
l'environnement de l'hôte n'est pas plus sûr sous 300 s que sous 30 s. Armer la
garde là couvrirait la population mesurée **par accident** (shell-exec déclare
`timeout_secs = 30`, mika#2276) et laisserait découvert tout handler au budget
plus large — et le jour où quelqu'un relève ce budget, la garde pip s'évapore
**sans qu'un seul test rougisse**. *C'est la classe mika#2205 : une garde inerte
sur une part de sa population se lit exactement comme une garde qui marche.*

Les deux gardes **composent** : une même commande peut être refusée par l'une ou
par l'autre, et aucune n'est la réparation de l'autre. mika#2423 est un
arbitrage de **budget** ; celui-ci est un arbitrage de **containment**.

### M3 — Le parseur de mika#2423 est aveugle à `sh -c` et confond les deux pips

Son propre doc-comment l'écrit : *« le scan est lexical, donc contournable —
découpage de token, assemblage par variable, `sh -c`, sous-shell »*. Et
`first.rsplit('/').next()` rend `pip` **aussi bien** pour `/usr/bin/pip` que pour
`$W/.venv/bin/pip` : il ne peut pas porter le contrôle positif qu'AC3 exige.

L'AC1 demande explicitement les formes d'évasion de mika#1957 (`sh -c`, `eval`,
chemin absolu, `;`, `$( )`). C'est la forme **lexicale sur toute la chaîne** qui
les donne — celle des deux scans déjà présents dans `run.sh`.

### M4 — Recensement AC4 : trois populations déclarées, plus une population ouverte

`shell-exec` est dans :

| allowlist | site | agent concerné |
|---|---|---|
| `DEFAULT_AGENT_SKILL_ALLOWLIST` | `mika-common/src/home.rs:660` | `mika` personnel / client (tier `default`) |
| `DEFAULT_IDENTITY` (miroir TOML du précédent) | `mika-common/src/home.rs:698` | idem — les tests de `home.rs` figent l'égalité |
| `MIKA_DEV_IDENTITY` | `well_known_agents.rs:154` | mika-dev |
| `MIKA_QA_IDENTITY` | `well_known_agents.rs:330` | mika-qa |

**Absent** de `FAMILY_AGENT_SKILL_ALLOWLIST` (`home.rs:749`) — donc les tiers
`family` et `champion` ne portent pas `shell-exec` — et absent de
`MIKA_TEST_IDENTITY`, de `MIKA_RELAY_IDENTITY` et de l'identité calculée de
mika-arch.

**Mais la population réelle est plus large que ces quatre déclarations**, et
c'est ce qui décide du site du correctif : un `identity.toml` sans bloc
`[skills].allowlist` est un **no-op** (`apply_identity_allowlist`), donc **tous**
les skills bundled sont actifs — `shell-exec` compris. C'est la classe mika#1596.
Un refus **énuméré par agent** serait donc faux par construction. Un refus dans
`run.sh` est universel **par construction**, et c'est la réponse structurelle à
l'AC4 : le recensement est un livrable de documentation, pas un mécanisme.

### M5 — `tmux` est un second canal vers le même hôte, et il reste découvert

`crates/mika-agent/templates/skills/tmux/handlers/create_session.sh:54` fait
`tmux send-keys -t "$NAME" -l -- "$COMMAND"` avec un `$COMMAND` fourni par le
modèle, **hors de `run.sh` et donc hors de ses trois scans**. Et `tmux` est dans
l'allowlist de mika-dev **et** de mika-qa.

Un `pip install` par `tmux` passe donc après ce correctif. **Population mesurée :
inconnue** — la base n'est pas lisible depuis le bac à sable (M7). Nommé comme
canal ouvert et porté en suivi avec sa précondition, jamais présenté comme
couvert.

### M6 — Le prédicat est validé sur un corpus de 32 cas, pas espéré

Prototype exécuté (`.pilot-scratch/proto-2639/proto.sh`, jetable) : **32/32**,
dont les deux verbatims mesurés, les six formes du commentaire opérateur, les
cinq évasions d'AC1, les deux contrôles positifs d'AC3 et quatre contrôles
négatifs de bruit. La forme exacte, les deux corrections que le prototype a
imposées et le faux positif qu'il a mis au jour sont au § *Le prédicat*.

### M7 — Aucune population n'est mesurable depuis le bac à sable de dispatch

`~/.mika/data/mika.db` et `/var/log/mika/` sont absents. Les chiffres ci-dessus
viennent du ticket (mesure MPC sur `server.log`). **Toute** sonde de population
de ce plan est donc un **geste d'opérateur sur l'hôte**, déclaré comme tel.
`uv`, `python3` et `pip` sont en revanche présents dans le bac à sable, ce qui
rend le harnais exécutable — et impose qu'il n'installe **rien** (§ V2).

---

## Décision d'architecture

**Le refus vit dans `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`**,
quatrième bloc, **après** le containment d'egress (mika#1991) et **avant**
l'appel à la garde de checkout partagé (mika#2449). Trois raisons, dans l'ordre
du poids :

1. **Population.** `run.sh` est le fichier unique que tout agent portant
   `shell-exec` exécute : le refus est universel sans énumération (M4).
2. **Formes d'évasion.** AC1 exige `sh -c` / `eval` / `$( )` / `;` / chemin
   absolu — ce que la forme lexicale sur toute la chaîne donne et que le parseur
   par tokens se déclare lui-même incapable de voir (M3).
3. **Même site, même posture, même contrat de commentaire** que ses deux aînés :
   le lecteur trouve le troisième scan là où sont les deux premiers.

**Placement avant mika#2449** pour la raison que les deux scans existants
écrivent déjà : on ne paie pas un sous-processus pour une commande qu'on va
refuser. L'ordre des refus devient donc : `FIRST_WORD` → mika#1957 → mika#1991 →
**mika#2639** → mika#2449.

### Quatre alternatives écartées, chacune sur son motif

| alternative | motif du refus |
|---|---|
| Étendre `BUILD_COMMAND_FAMILY` (mika#2423) | prédicat conditionné au budget, exempte `long_running`/`detaches_command` — garde partiellement inerte, silencieusement (M2) |
| Un fichier séparé résolu sous `$MIKA_PLATFORM_DIR` (motif mika#2449) | introduit une fenêtre **fail-open** sur « script absent ». mika#2449 peut se le permettre (un tenant sans plateforme n'a rien à protéger) ; ici l'hôte a toujours un environnement Python à protéger |
| Un helper sourcé (motif `_shared/cwd-guard.sh`, mika#2536) | `templates/skills/` n'a pas de convention `_shared/`, et un seul handler en a besoin aujourd'hui. YAGNI — et le jour où `tmux` en aura besoin (M5), l'extraction sera le livrable de ce ticket-là |
| Un correctif de prompt seul | `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — et de toute façon il n'y a aucun prompt fautif à corriger (M1) |

### Mettre sous bac à sable `shell-exec`

C'est le **mur** (mika#2141) et la seule fermeture complète de la classe. Hors
périmètre, nommé : `run.sh` tourne hors bwrap, et l'y mettre est un arbitrage de
confinement qui appartient à son propre ticket.

---

## Le prédicat

Trois étapes, toutes exprimables en `grep -E` + un `sed`, donc sans fichier
neuf ni dépendance.

**Étape 1 — neutraliser la forme autorisée, puis chercher la forme interdite.**
C'est l'inversion qui porte tout le reste : plutôt que d'exprimer « un chemin
qui n'est pas un venv » (inexprimable en ERE sans lookbehind), on remplace les
installateurs venv-qualifiés par une sentinelle, puis on scanne le reste.

```sh
SENTINEL='__MIKA_VENV_INSTALLER__'
VENV='[^[:space:]"'"'"']*/(\.venv|venv|\.virtualenv|virtualenv|env)/bin/(pip[0-9.]*|python[0-9.]*)'
_SCAN=$(printf '%s\n' "$COMMAND" \
  | sed -E "s#${VENV}#${SENTINEL}#g" \
  | sed -E "s#${SENTINEL}[\"']?[[:space:]]+(-[^[:space:]]+[[:space:]]+)*-m[[:space:]]+pip#${SENTINEL}#g")
```

La sentinelle ne contient **aucune** sous-chaîne `pip`, pour qu'elle ne puisse
pas se faire apparier par les règles de l'étape 2.

**Étape 2 — quatre règles de refus, appliquées à `$_SCAN`** (motif
`host_installer`) : `pip`/`pip3` + `install`, `python[3] -m pip install`,
`uv pip install`, `pipx install`. Chacune avec la frontière de mika#1957
(`[^A-Za-z0-9_.-]`, qui inclut `/` donc attrape `/usr/bin/pip`), un guillemet
fermant **optionnel** après le binaire, et des drapeaux tolérés entre le binaire
et la sous-commande (`pip --quiet install`).

**Étape 3 — le drapeau visant l'hôte, même sur une forme venv** (motif
`host_target_flag`) : `--break-system-packages`, `--user`, `--target`,
`--prefix`, `--system`, **en conjonction** avec la présence d'un verbe
d'installation dans la commande **brute**. La conjonction est porteuse :
`cmake --prefix /x` n'est pas l'affaire de cette garde, et
`"$W/.venv/bin/pip" install --user -e .` écrirait dans `~/.local` malgré son pip
de venv.

### Les deux corrections que le prototype a imposées

1. **Guillemet fermant.** `"/usr/bin/pip" install foo` **passait** : la règle
   exigeait un blanc immédiatement après `pip`. D'où le `["']?` — sans lui la
   garde était contournable d'un guillemet.
2. **Neutralisation de `<venv-python> -m pip`.** `"$W/.venv/bin/python" -m pip
   install -e "$W"` était **refusé** : le `python` venv était neutralisé et le
   `pip` nu restant appariait la règle 1. Rendu passant, parce que le
   discriminant est *quel environnement*, jamais *quel binaire* — et parce que
   `python -m venv … && .venv/bin/python -m pip install` est un idiome courant.
   Le drapeau `--user` continue de refuser cette forme (F18 du corpus).

### Trois limites nommées, pas découvertes

- **Faux positif de prose.** La frontière arrière est
  `([^A-Za-z0-9_.-]|$)` et non `([[:space:]]|$)` — sans quoi `pip install;`
  s'échappait, trivialement. Le prix : `grep -rn "pip install -e" docs/` est
  refusé. Même propriété que le scan `gh` de mika#1957, même posture assumée ;
  le contournement est `grep -rn 'pip[ ]install'`, et le corps du refus le
  rappelle sans jamais donner un gabarit d'installation.
- **Nom de venv hors liste.** L'allowlist de racine est
  `{.venv, venv, .virtualenv, virtualenv, env}` sur un **segment exact** : un
  venv nommé autrement (`myenv/bin/pip`, `/opt/conda/envs/x/bin/pip`) est
  **refusé** — fail-closed, et le substitut universel `uv run` reste ouvert. Le
  membre `env` est le plus lâche de la liste (`/usr/local/env/bin/pip` serait
  neutralisé) ; il est gardé parce que `python -m venv env` est courant, et son
  caractère lâche est écrit au site.
- **Les évasions de mika#1957 restent les évasions.** Découpage de token
  (`p""ip install`), assemblage par variable (`P=pip; $P install`), payload
  base64 : la garde lit une ligne de commande, pas ce qu'elle exécute. *Defense
  in depth, NOT a sole gate* — la phrase que `run.sh` porte déjà deux fois.

### La surface de refus

**stderr + `exit 1`**, comme ses deux aînés du même fichier et de la même famille
de bloc — l'argument qui a décidé du site vaut pour le canal : *le lecteur trouve
le troisième scan là où sont les deux premiers.* La surface SQL est **identique à
celle de stdout** : sur un exit non nul, `execute_exec`
(`executor.rs:1467-1479`) combine stdout et stderr et préfixe `Exit code: 1`,
donc le jeton atterrit dans `tool_calls.output` de toute façon. mika#2449 a
choisi stdout pour la même raison (l. 185, `printf` sans `>&2`) ; ici la
cohérence de fichier tranche, et il n'y a rien à gagner à diverger.

**Un piège à ne pas hériter, parce que le code le porte.** Le commentaire de
`execute_exec` affirme *« run.sh merges them with 2>&1 »* — vrai de la commande
**évaluée**, faux du refus : le `2>&1` de `run.sh` est porté par la ligne
`eval "$COMMAND" 2>&1` (l. 199) **seule**, qu'un refus n'atteint jamais. Le
stderr du bloc arrive donc comme vrai stderr de process, et c'est la combinaison
de l'exécuteur — non une fusion côté shell — qui le fait atterrir dans
`tool_calls.output`. La déduplication voisine (`stderr_trimmed != stdout.trim()`)
ne peut pas l'écarter : sur un refus, stdout est vide. Conclure de ce
commentaire que le stderr est déjà fusionné, et donc qu'il pourrait être
dédupliqué, rendrait fausse la requête SQL du § *Surfaces opérateur* sur sa
seule population.

Format de fil, **site de définition unique** :

```
REFUS (python-installer-guard, mika#2639): <motif>
```

Deux motifs, `host_installer` et `host_target_flag`, figés par test.

**Une divergence assumée avec les deux aînés, elle :** ils écrivent
`Error: shell-exec refuses …`, sans lecteur. Le préfixe `REFUS (<nom>, mika#N)`
est le format de fil que la maison emploie quand un refus a une **requête
opérateur** (mika#2449, mika#2536, mika#2627), et c'en est le cas. Un
`Error: …` ici rendrait la population incomptable.

**Le jeton n'est PAS déclaré dans `scripts/canonical-tokens.tsv`**, et la règle
est celle que le TSV écrit lui-même (l. 370-372) : `cwd-guard.sh` et
`pr-push-guard.sh` composent le même préfixe **sans jamais le relire**, donc
n'ont aucun site de match. Ce bloc est dans ce cas, exactement comme le jeton
mika#2449 qui n'y est pas non plus. `scripts/canonical-tokens-survey.sh --check`
est vert aujourd'hui (80 sites) et son exécution après le correctif est une étape
de V6 : s'il accuse le nouveau site, on **déclare** avec son motif, sur le
précédent mika#2627 — on ne renomme pas pour sortir de la population.

---

## Travaux

### R1 — Le refus structurel (AC1)

`crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` — quatrième
bloc, borné par `# --- mika#2639: python installer containment ---` /
`# --- end python installer containment ---` (la même forme de bornage que les
trois blocs existants, et ce que le contrôle négatif du harnais découpe).

Le commentaire du bloc porte, dans l'ordre : le défaut mesuré avec sa date et sa
trace disque ; pourquoi ici et pas dans `BUILD_COMMAND_FAMILY` (M2) ; l'inversion
neutraliser-puis-scanner avec sa raison ; les trois limites nommées ; et la
phrase *defense in depth, NOT a sole gate*.

### R2 — Le substitut écrit (AC2)

`skills/bundled/qa-review/system_prompt.md` — une sous-section de la règle
existante **« Never compile inside the review turn »** (l. 331), qui est l'ancre
juste : un `pip install && pytest` **est** une compilation-et-test dans le tour
de revue, et cette règle le disait déjà en esprit (« `cargo build/test/clippy`,
`npm run build` **and their kin** »). On y ajoute la recette Python, absente
(M1) :

- tester un dépôt Python se fait par `uv run --project <worktree> pytest` ou
  `uv run pytest` **dans le worktree** ;
- ou dans un venv jetable :
  `python -m venv "$W/.venv" && "$W/.venv/bin/pip" install -e "$W"` ;
- **jamais dans l'environnement de l'hôte** — et la butée est **topique** :
  elle nomme l'invariant (« ne rien installer hors d'un venv explicite ») et le
  substitut, sans énumérer les gabarits interdits, qui serait fournir au modèle
  la commande qu'on lui retire (doctrine mika#2520 / mika#2292).

**La moitié qui tient n'est pas celle-là** : le prompt exprime l'intention, R1
la tient.

### R3 — Le harnais (AC3)

`scripts/test-python-installer-guard.sh`, modelé sur
`scripts/test-shell-exec-guard.sh` (même `run_handler`, même `json_cmd`, même
comptage, même `exit 1` final), câblé par `make test-python-installer-guard` et
par un job CI `python-installer-guard-lint` (forme du job
`shared-checkout-guard-lint`).

**Fixture hermétique, qui n'installe rien** : un `PATH` qui place des shims
`pip`/`pip3`/`uv`/`pipx` écrivant un sentinel sur stdout, et un faux
`$W/.venv/bin/pip` exécutable qui en écrit un autre. Les formes refusées
n'atteignent jamais `eval`, donc aucun shim ne tourne ; les contrôles positifs
atteignent `eval` et **le sentinel l'atteste** — sans quoi « non refusé » serait
indistinguable de « refusé par autre chose ».

Le corpus est celui du § M6, et chaque entrée porte son statut : **mesurée**
(les deux verbatims du ticket et de son commentaire) ou **reconstruite depuis une
forme énumérée par AC1** — discipline mika#2565 (« fixtures are frozen, not
refreshed »).

### R4 — Le recensement (AC4)

`crates/mika-agent/CLAUDE.md`, au voisinage de la section `## Exec Handlers` : le
tableau de M4, la population ouverte des agents sans `[skills].allowlist`, et la
phrase qui en découle — *le refus est universel par le site, jamais par
l'énumération*. Plus le canal `tmux` de M5, nommé non couvert.

### R5 — L'entrée racine

`CLAUDE.md` racine : une section au voisinage de mika#2536 (même fichier gardé,
même famille de refus), portant les surfaces opérateur, les régimes attendus, les
sondes et leurs haltes.

---

## Fire-Disposition

Ce plan livre deux détecteurs : le refus de R1 (chemin de succès = « aucune
commande fautive ») et le harnais de R3.

**Option (a) — exception nommée en allowlist, livrée VIDE et épinglée vide.**

- **La population existante est vide, et c'est mesuré :**
  `grep -rln 'pip install\|pipx install\|uv pip' Makefile scripts/ skills/ crates/ .github/`
  rend **zéro**. Aucun script, handler, prompt ou cible `make` du dépôt ne
  prescrit une installation Python. Il n'y a donc **rien à exempter**.
- L'allowlist existe néanmoins, sous la forme d'une constante nommée dans le
  harnais (`PYTHON_INSTALLER_ALLOWED_FORMS`), **livrée vide**, avec un test
  frère qui refuse qu'elle cesse de l'être — motif `HANDLER_ENV_KNOWN_INERT`
  (mika#2536) et `pilot-push-allowlist.txt` (mika#2520). **Quand la garde tire,
  on route la commande vers le substitut ; on n'ajoute pas de ligne** (doctrine
  mika#2201).
- **L'option (b), livrer désarmé, est refusée avec son motif.** Le défaut est
  Tier 1, récurrent, et sa prochaine occurrence est datée (« la prochaine est
  cpp#279 »). Et mika#2272 a mesuré ce que coûte un détecteur livré derrière une
  condition d'armement : la condition s'est révélée **insatisfiable**, pas
  seulement non remplie, et la garde n'a jamais tiré. Ce qui paie la prudence
  ici est nommé et concret : les trois limites du § *Le prédicat*, les quatre
  contrôles négatifs du corpus, et le contrôle négatif du harnais (V5).

**Aucune variable de désarmement, et c'est une décision.** Le précédent le plus
proche est mika#1646 (garde d'action destructive) et mika#2573 (création de
travail) : un désarmement par variable sur un chemin de containment serait un
désarmement par coquille. Le geste de désarmement est un **revert**, et le coût
d'un faux positif — une commande refusée, visible dans `tool_calls.output`,
rattrapable au tour suivant — le supporte. À comparer au coût d'un faux négatif :
le lanceur de production réécrit, et 2 h 45 de rail mort.

---

## Contrat de vérification

| # | ce qui est attesté | où |
|---|---|---|
| **V1** | les deux verbatims mesurés (ticket + commentaire) sont refusés, **vu rouge avant R1** | `scripts/test-python-installer-guard.sh` |
| **V2** | les contrôles positifs d'AC3 (`uv run pytest`, `"$W/.venv/bin/pip" install -e "$W"`) passent **et atteignent `eval`** (sentinel du shim) | idem |
| **V3** | les quatre contrôles négatifs de bruit passent : `pip list`, `pip --version`, `cmake --prefix /x`, `echo "pipeline install done"` | idem |
| **V4** | chacune des cinq formes d'évasion d'AC1 est refusée **individuellement** — jamais toutes neutralisées d'un coup (leçon mika#2277) | idem |
| **V5** | **contrôle négatif** : handler dépouillé du bloc R1 ⇒ le harnais rougit sur V1 | idem, en fin de fichier |
| **V6** | `bash scripts/canonical-tokens-survey.sh --check`, `bash scripts/check-canonical-tokens.sh`, `make verify-bundled-skills`, `make test-shared-checkout-guard`, `make test-cwd-guard` restent verts | commandes |
| **V7** | le motif de refus a un **site de définition unique** dans `run.sh`, et les deux valeurs de motif sont figées | assertion du harnais |
| **V8** | `cargo test -p mika-agent skills::executor` reste vert — aucun site Rust n'est touché | commande |

**V1 et V4 doivent être vus rouges avant le correctif**, un terme à la fois. Une
conjonction de termes fail-safe ne se prouve pas en les neutralisant tous
ensemble : c'est la leçon que mika#2277 a payée.

**Ce que V1–V8 ne peuvent PAS attester**, et il faut l'écrire : qu'aucun pilote
ne recassera le lanceur. Le harnais atteste que la commande est refusée **avant
`eval`** ; qu'aucune autre porte ne reste ouverte est la sonde S4 et le suivi M5.

---

## Surfaces opérateur

```bash
# 1. La garde a-t-elle mordu, et sur quel motif ? (geste opérateur, sur l'hôte)
#    NON ancré en tête : l'exécuteur préfixe « Exit code: 1 » (executor.rs:1479).
```
```sql
SELECT agent_id, count(*) FROM tool_calls
 WHERE tool_name = 'run_shell'
   AND output LIKE '%REFUS (python-installer-guard, mika#2639)%'
 GROUP BY 1 ORDER BY 2 DESC;

-- Par motif — deux LIKE explicites, jamais un substr sur un offset calculé :
-- une requête opérateur doit rester vraie quand le préfixe change de longueur.
SELECT
  sum(output LIKE '%mika#2639): host_installer%')   AS host_installer,
  sum(output LIKE '%mika#2639): host_target_flag%') AS host_target_flag
 FROM tool_calls WHERE tool_name = 'run_shell';

-- CONTRÔLE POSITIF — `run_shell` tourne-t-il seulement ?
SELECT count(*) FROM tool_calls WHERE tool_name = 'run_shell';
```

| motif | régime attendu | lecture |
|---|---|---|
| `host_installer` | **non vide et DÉCROISSANT** | chaque ligne est une écriture dans l'environnement de l'hôte arrêtée. Non vide les premiers jours *par construction* : 38 des 43 commandes mesurées échouaient déjà sur PEP 668 et seront maintenant refusées — ce n'est **pas** une tempête de faux positifs, c'est le trafic mesuré rendu visible |
| `host_target_flag` | proche de zéro | une forme venv portant `--user`/`--target`/… : le modèle a la bonne recette et le mauvais drapeau |
| un même agent portant un flot soutenu après 7 jours | **anomalie** | le prompt ne l'atteint pas : lire le seed (`cat ~/.mika/skills/.manifest-writer`) **avant** de toucher au prédicat |
| une ligne sur un `grep` de prose | faux positif **nommé** | le contournement est `grep -rn 'pip[ ]install'` ; ne pas rétrécir la frontière arrière, elle ferme `pip install;` |

**Aucun événement de journal neuf, et c'est une décision.** `run.sh` est un
sous-processus shell sans accès base, et son stderr d'avant-`eval` atterrit déjà
dans `tool_calls.output` par le chemin ci-dessus. Inventer un sink de journal que
personne ne lirait reproduirait le défaut du Signal M (mika#2050). Les trois
scans voisins du même fichier n'en ont pas non plus.

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** `crates/mika-agent/templates/skills/` est une projection du
> **binaire**, pas du checkout : `cat ~/.mika/skills/.manifest-writer` doit
> porter le sha qu'on vient de bâtir. **Sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier** (classe mika#2340).

**S1 — le rejeu du défaut fondateur** (première revue QA d'une PR claude-pilot,
la prochaine étant cpp#279). Attendu : une ligne `host_installer`, et
`~/.local/bin/claude-pilot` **inchangé**.

```bash
sha256sum < ~/.local/bin/claude-pilot | cut -d' ' -f1   # AVANT, jamais le contenu
stat -c '%s %y' ~/.local/bin/claude-pilot
# … la revue …
sha256sum < ~/.local/bin/claude-pilot | cut -d' ' -f1   # APRÈS — doit être identique
```

*Halte 1 — le digest a bougé.* **Ne pas élargir le prédicat par réflexe.**
Établir d'abord **par quelle porte** l'écriture est passée : `tmux` (M5), un
script du dépôt, une évasion nommée du § *Le prédicat*, ou un geste humain. Les
quatre remèdes diffèrent, et un seul est dans ce dépôt.

**S2 — la garde mord et le prompt prend** (30 jours). Le compte `host_installer`
doit **décroître** : les premiers jours il porte le trafic mesuré, puis le
substitut de R2 doit le tarir.
*Halte 2 — plateau plutôt que décroissance.* C'est la mesure que le prompt
n'atteint pas ce chemin, pas que le prédicat est trop large. Vérifier le seed du
prompt bundled (`~/.mika/skills/.manifest-writer`) **avant** de toucher à R1.

**S3 — contrôle négatif de bruit** (7 jours). Aucun refus sur un `run_shell`
nominal, et en particulier aucun sur un `uv run pytest`.
*Halte 3 — une occurrence.* C'est un faux positif, et son coût est une
vérification QA impossible. **Désarmer d'abord** (revert du bloc R1),
diagnostiquer ensuite : une commande légitime refusée est un arbitrage de
prédicat, pas un seuil à régler.

**S4 — le canal `tmux`** (30 jours). La précondition du suivi M5 :

```sql
SELECT id, created_at, substr(input, 1, 200) FROM tool_calls
 WHERE tool_name LIKE 'tmux%'
   AND (input LIKE '%pip install%' OR input LIKE '%uv pip%' OR input LIKE '%pipx install%')
 ORDER BY created_at DESC;
```

*Halte 4 — elle rend des lignes.* Le canal est réel et mesuré : le suivi s'ouvre
**avec ce compte**, et c'est lui qui décide si l'extraction d'un
`_shared/python-installer-guard.sh` est due. Zéro ligne est un **résultat** — il
dit que le modèle n'emprunte pas cette route — et non une preuve qu'elle est
fermée.

**Halte transverse — les deux sondes muettes.** Zéro refus **et** zéro
`run_shell` ne prouve **rien** : vérifier le contrôle positif avant toute
conclusion. *Une garde que personne n'a exercée se lit exactement comme une garde
qui marche* (mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne rattrape pas l'incident du 2026-10-02.** `~/.local/bin/claude-pilot` a
  été réécrit et restauré à la main ; **rien ici ne rétro-estampille** — la sonde
  est la **prochaine** revue de PR claude-pilot.
- **Il ne ferme pas le canal `tmux`** (M5), ni les trois évasions nommées du
  § *Le prédicat*. Ce qui est retiré au modèle est la route qu'il a
  **effectivement** prise, 43 fois en 48 h.
- **Il ne met pas `shell-exec` sous bac à sable.** C'est le mur, et c'est un
  autre ticket.
- **Il ne borne pas ce que `pip` fait quand il est légitimement appelé** dans un
  venv : un venv jetable sous `/tmp` reste un venv jetable, et la garde ne
  vérifie pas qu'il est jetable.
- **Il n'ajoute aucun compteur et aucun événement de journal.** Le seul
  instrument neuf est le motif dans `tool_calls.output`, et **son silence ne
  prouve rien tant que personne n'exécute les sondes**.
- **Il ne corrige aucun prompt fautif, parce qu'il n'y en a pas** (M1) : il pose
  une recette absente. Dire l'inverse serait revendiquer une correction qui n'a
  pas de sujet.

---

## Hors périmètre, délibérément

- **`BUILD_COMMAND_FAMILY` et `refuse_uncontainable_build`** (mika#2423) :
  inchangés, population disjointe, motif du refus écrit en M2.
- **`guard-shared-checkout` et ses deux modes** (mika#2107 / mika#2449) :
  inchangés, aucune ligne touchée.
- **Le canal `tmux`** (M5) — **suivi**, précondition : la sonde S4.
- **L'extraction d'un helper partagé** vers une convention `_shared/` pour
  `templates/skills/` — **suivi**, même précondition (un second consommateur
  mesuré).
- **Le bac à sable de `shell-exec`** (mika#2141) — le mur, son propre ticket.
- **La cause amont côté claude-pilot** : qu'un `pip install -e` d'un dépôt
  déclarant `[console_scripts] claude-pilot` écrase le lanceur de production est
  une propriété du paquet, pas de ce dépôt. `senara-solutions/claude-pilot`.
- **La sentinelle de détection de mika#2634 (AC5)** : reste le filet, inchangée.
- **Le classifieur de permissions** : aucun relâchement, aucune ligne — le ticket
  le dit lui-même, c'est un **durcissement** d'outil.

---

## Definition of Done

1. `run.sh` porte le quatrième bloc, borné et commenté, et refuse sur **stderr**
   (`>&2`) avec `exit 1` et le jeton de format de fil — le canal de ses deux
   aînés du même fichier (l. 78 et l. 122), et non celui de mika#2449 (l. 185,
   `printf` sur stdout). Le § *La surface de refus* porte l'arbitrage ; le jeton
   atteint `tool_calls.output` dans les deux cas, donc rien n'est perdu à suivre
   la cohérence de fichier.
2. `qa-review/system_prompt.md` porte la recette Python, en butée **topique**,
   accrochée à la règle « Never compile inside the review turn ».
3. `scripts/test-python-installer-guard.sh` existe, est câblé par
   `make test-python-installer-guard` et par le job CI
   `python-installer-guard-lint`, et V1–V5 passent — V1 et V4 ayant été **vus
   rouges** avant R1, un terme à la fois.
4. Le recensement AC4 est écrit dans `crates/mika-agent/CLAUDE.md`, avec la
   population ouverte et le canal `tmux` nommé non couvert.
5. `CLAUDE.md` racine porte les surfaces, les régimes attendus, les quatre sondes
   et leurs cinq haltes.
6. V6 et V8 verts ; `cargo fmt`, `cargo clippy`, `cargo test` verts.
7. Le corps de PR nomme les sept mesures du § Constat, la Fire-Disposition et son
   allowlist vide, et les deux suivis avec leurs préconditions.

---

## Acceptance criteria

Transcrits du corps du ticket senara-solutions/mika#2639, enrichis du
commentaire opérateur du 2026-10-02T08:32:30Z.

- [ ] **AC1.** **Refus structurel dans `shell-exec`**
  (`crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`, à côté du
  scan lexical `gws`/`gh` de mika#1957). Sont refusés `pip install`,
  `pip3 install`, `python -m pip install` et toute forme portant
  `--break-system-packages`, `--user` ou une cible hors d'un venv explicite, avec
  les mêmes formes d'évasion que mika#1957 (`sh -c`, `eval`, chemin absolu, `;`,
  `$( )`). Le message de refus donne le substitut (AC2).
  **Enrichissement du commentaire opérateur :** le refus couvre aussi
  `uv pip install` (y compris avec `--system`), `pip install --user`/`--target`/
  `--prefix`, `python -m pip install` et `pipx install`. `uv run` (sans `pip`)
  reste le substitut autorisé.
- [ ] **AC2.** **Substitut écrit dans le prompt et le skill de revue de mika-qa**
  (`qa-review`) : tester un dépôt Python par `uv run --project <worktree> …` ou
  `uv run pytest` **dans le worktree**, ou dans un venv jetable
  (`python -m venv "$W/.venv" && "$W/.venv/bin/pip" install -e "$W"`). Jamais
  dans l'environnement de l'hôte.
- [ ] **AC3.** Test (harnais shell-exec) : les 3 verbatims ci-dessus sont refusés
  (**vu rouge** avant le correctif) ; `uv run pytest` et
  `"$W/.venv/bin/pip" install -e "$W"` passent (contrôles positifs).
- [ ] **AC4.** Recenser les autres agents qui ont `shell-exec` et pourraient
  faire de même : le refus vaut pour tous.

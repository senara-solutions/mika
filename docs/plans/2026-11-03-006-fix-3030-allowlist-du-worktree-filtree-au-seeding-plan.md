# mika#3030 — l'allowlist du worktree est construite par dispatch-lib, plus copiée de l'opérateur

- **Ticket :** senara-solutions/mika#3030
- **Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
- **Périmètre :** mika seulement. La partie claude-pilot (test de `can_use_tool`, règles explicites de `permissions.yaml`) est hors vol et nommée telle par le ticket.

## Contexte mesuré

La mesure est faite dans le corps du ticket et ce plan ne la refait pas. Ce qu'elle
établit, et sur quoi tout le reste s'appuie :

- Un pilote a ouvert mika#3024 par `gh issue create` le 03/11 à 12:46:33Z, pendant le gel.
- Rejoué hors ligne contre la politique installée, le même geste rend `deny`, règle
  `gh-issue-create-deny`.
- Il n'existe **aucune ligne de décision** pour ce `tool_use` — ni `audit_events`, ni
  `server.log`, ni le transcript. La politique n'a pas été *interrogée*.

La cause se lit sur une seule ligne de `claude-pilot/src/claude_pilot/permissions.py`
(l. 2580) telle que le ticket la cite : *« ce callback n'est appelé que pour une commande
qu'aucune règle allow des réglages ne couvre »*. Or `dispatch-lib.sh` l. 6098 copie
l'allowlist de la session de l'opérateur dans chaque worktree :

```sh
cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
```

Ce fichier porte 104 entrées, dont `Bash(gh pr:*)`, `Bash(gh issue:*)`, `Bash(gh repo:*)`,
`Bash(gh issue *)` et `Bash(gh label *)`. Chacune est un **joker de famille** : elle couvre
`gh issue create`, mais aussi `gh issue close`, `gh pr merge`, `gh pr review`,
`gh repo delete`. Le callback n'est donc jamais appelé pour aucune de ces commandes, et la
politique ne refuse rien qu'elle n'ait pas été invitée à voir.

### Deux faits de site, vérifiés dans ce worktree

1. **Un seul geste de copie.** `grep -n "settings\.local\.json" dispatch-lib.sh` rend quatre
   lignes : le `cp` à 6098, le pathspec d'exclusion du rescue à 7346, et deux lignes de
   commentaire (7307, 11936). Il n'y a **pas** de second chemin de préparation à amender.
2. **La source `project` ne porte aucune permission.** `.claude/settings.json` est tracké par
   le dépôt et ne porte que la clé `hooks` (garde mika#2107) — son propre `$comment` le dit :
   *« noms de fichiers distincts ET clés disjointes »*. Donc aujourd'hui, toute la couverture
   de L vient de `local`. AC1 porte sur les deux sources, et la part `project` se tient par
   une **absence** — ce qui demande une assertion, sinon elle se perd au premier ajout.

### Ce qui n'est pas la cause

- `~/.claude/settings.json` (source `user`), qui porte `Bash(gh pr merge:*)`. Le ticket le met
  hors vol et dit pourquoi : *il n'est pas mesuré ici que le pilote lise ce fichier dans son
  sandbox*. Le vol ne touche ni ne suppose rien de cette source.
- Le contenu de `mika-platform/.claude/settings.local.json` lui-même. C'est l'allowlist de la
  session de l'opérateur, hors vol, et elle reste intacte. Le vol change ce que le worktree
  **reçoit**, jamais ce que l'opérateur **a**.

## Requirements

- **R1.** Dans un worktree préparé par `dispatch-lib.sh`, aucune règle allow des réglages
  `project` et `local` ne couvre une commande de la liste L : `gh issue create`,
  `gh issue close`, `gh issue edit`, `gh issue comment`, `gh pr merge`, `gh pr review`,
  `gh pr close`, `gh repo delete`, `gh label create`.
- **R2.** Les gestes du pipeline qui passent aujourd'hui par le joker copié restent permis,
  par des entrées étroites : `gh pr create`, `gh pr view`, `gh pr edit`, `gh pr ready`, plus
  les lectures `gh issue view` et `gh issue list`.
- **R3.** Les entrées non-`gh` de l'allowlist de l'opérateur (cargo, npm, git, `Read(...)`, les
  serveurs MCP) continuent d'arriver dans le worktree. Le vol n'est pas un rétrécissement
  général de ce que le pilote peut faire.
- **R4.** `dispatch-lib.sh` ne copie dans le worktree aucun fichier de réglages qui n'a pas été
  filtré. Un scan, épinglé par un test, refuse un second `cp … settings.local.json`.
- **R5.** Le repli (jq absent, source illisible, filtre en échec) est **fail-closed** : il écrit
  la liste étroite seule, et ne recopie jamais la source brute.

## Design decisions

### D1 — La liste étroite est une donnée du dépôt ; le filtre est sur la FORME, pas sur L

Le filtre évident serait : « retire toute entrée qui couvre une commande de L ». Il est faux
pour la raison que mika#2309 a déjà écrite dans ce dépôt — *le prédicat est sur le site, jamais
sur la valeur* : un filtre indexé sur L laisse passer la dixième commande que personne n'a
listée. `gh pr close` et `gh label create` ne sont dans L que parce que quelqu'un y a pensé le
03/11 ; `gh workflow run`, `gh release create` et `gh secret set` n'y sont pas.

L'inversion : **jeter toute entrée `gh`, puis réinjecter une liste étroite close**, déclarée
dans `dispatch-lib.sh`.

```sh
# Ce que le pilote peut faire avec `gh` dans un worktree de dispatch. CLOSE, et
# fermée par construction : tout ce qui n'est pas ici tombe sur `can_use_tool`,
# donc sur la politique de claude-pilot — qui est le seul endroit où une
# décision d'écriture GitHub doit se prendre, et le seul qui l'écrive au journal.
#
# mika#3030. Avant, le worktree recevait `Bash(gh pr:*)` / `Bash(gh issue:*)` par
# copie de l'allowlist de l'opérateur ; un joker de famille, qui court-circuite
# le callback pour TOUTE sous-commande de la famille. mika#3024 a été ouvert par
# cette porte, sans qu'aucune ligne de décision soit écrite nulle part.
#
# Chaque entrée nomme UNE sous-commande. Jamais `gh pr:*`. Jamais `gh api` (dont
# `-X PATCH` est une écriture : 2 appels mesurés, refusés par la politique — ils
# doivent continuer à l'être).
_WORKTREE_GH_ALLOW_NARROW=(
    # Écritures du pipeline (mika#3030 AC2) — mesurées sur les transcripts.
    "Bash(gh pr create:*)"        # 189 appels / 189 vols
    "Bash(gh pr edit:*)"          #  99 appels /  85 vols — le stamp origin:loop
    "Bash(gh pr ready:*)"         #   5 appels /   4 vols
    # Lectures.
    "Bash(gh pr view:*)"
    "Bash(gh pr list:*)"
    "Bash(gh pr diff:*)"
    "Bash(gh pr checks:*)"
    "Bash(gh issue view:*)"
    "Bash(gh issue list:*)"
)
```

**Ce que cette inversion achète.** R1 devient vrai *par construction* et non par énumération :
aucune commande de L n'est dans la liste, donc aucune n'est couverte — et il en va de même de
toute commande future que personne n'a listée. Le test peut alors vérifier la **construction**
plutôt que courir derrière les valeurs.

**Les quatre lectures au-delà d'AC2.** `gh pr list`, `gh pr diff`, `gh pr checks` sont
aujourd'hui couvertes par `Bash(gh pr:*)` ; `gh pr list` apparaît 14 fois dans les commandes et
skills de dispatch de ce dépôt. Les retirer sans les réinjecter ferait tomber des lectures de
pipeline sur la politique, ce qui est au mieux un aller-retour et au pire un refus — un
élargissement du vol par accident. Ce n'est pas un élargissement de **surface** : aucune n'est
dans L, les trois sont des lectures, et chacune nomme sa sous-commande. La propriété qui compte
n'est pas « la liste est courte », c'est « la liste ne contient aucun joker de famille ».

**`gh api` est délibérément absent.** Les deux appels `gh api -X PATCH/PUT` mesurés sont des
écritures que la politique refuse. Les réinjecter sous une forme lisible-seulement demanderait
de distinguer la méthode HTTP dans un motif d'allowlist — exactement le genre de prédicat fin
qui appartient à `permissions.py`, pas à un `Bash(...)`. Les lectures `gh api graphql` tomberont
donc sur le Tier 1 de la politique, qui est son métier.

### D2 — Le prédicat de rejet : un mot isolé `gh` dans une entrée `Bash(...)`

```jq
.permissions.allow = (
    ((.permissions.allow // []) | map(select((startswith("Bash(") and test("\\bgh\\b")) | not)))
    + $narrow
)
```

Deux propriétés et un choix de sens d'erreur :

- **`startswith("Bash(")`** borne le filtre aux règles de commande. Une entrée
  `Read(//home/.../highlight/**)` ou `WebFetch(domain:…)` n'est jamais candidate, quoi qu'elle
  contienne.
- **`\bgh\b`** sur-approxime volontairement. Une entrée
  `Bash(grep -r "gh" …)` serait jetée alors qu'elle ne couvre rien de L — faux positif. Et une
  entrée `Bash(cd x && gh issue create)` est jetée alors qu'un prédicat ancré en début de chaîne
  l'aurait manquée.

  Le sens de l'erreur est choisi : **le coût d'un faux positif est un refus récupérable** (la
  commande tombe sur la politique, qui tranche et l'écrit au journal) ; **le coût d'un faux
  négatif est le défaut du ticket** — une écriture GitHub qui passe sans qu'aucune décision soit
  prise. Sur-approximer le rejet est la seule direction qui ne puisse pas reconstituer le
  défaut. C'est le miroir exact du choix inverse fait par mika#2761 sur la lecture
  d'atteignabilité des suites : là, sous-approximer donnait des faux rouges et jamais des faux
  verts ; ici, sur-approximer donne des faux refus et jamais des faux passages.

### D3 — Le repli est fail-closed, et c'est la moitié du ticket

`jq` est une dépendance hôte déclarée (CLAUDE.md : *« `jq` is required by all skill handler
scripts »*), mais le `cp` qu'on remplace porte `2>/dev/null || true` — une tolérance qui, muée
en repli naïf, ferait exactement ce que le vol ferme :

```sh
# ✗ Ce repli reconstitue le défaut, silencieusement, le jour où jq manque.
jq … "$src" > "$dst" 2>/dev/null || cp "$src" "$dst"
```

Le repli écrit donc la **liste étroite seule**, sans jq, et jamais la source :

```sh
_seed_worktree_settings_allowlist() {
    local platform_dir=$1 worktree_dir=$2
    local src="$platform_dir/.claude/settings.local.json"
    local dst="$worktree_dir/.claude/settings.local.json"
    local narrow tmp i sep

    if [ -f "$src" ] && command -v jq >/dev/null 2>&1; then
        narrow=$(printf '%s\n' "${_WORKTREE_GH_ALLOW_NARROW[@]}" | jq -R . | jq -s -c .)
        tmp="${dst}.tmp.$$"
        if jq --argjson narrow "$narrow" '
                .permissions.allow = (
                    ((.permissions.allow // [])
                        | map(select((startswith("Bash(") and test("\\bgh\\b")) | not)))
                    + $narrow)
            ' "$src" > "$tmp" 2>/dev/null && [ -s "$tmp" ]; then
            mv "$tmp" "$dst"
            echo "[dispatch-lib] worktree_allowlist: mode=filtered narrow=${#_WORKTREE_GH_ALLOW_NARROW[@]}" >&2
            return 0
        fi
        rm -f "$tmp" 2>/dev/null || true
    fi

    # Repli FAIL-CLOSED (mika#3030 R5) : la liste étroite SEULE. Ne recopie
    # jamais la source — un repli qui copie rend le joker de famille, donc le
    # défaut, le jour précis où le filtre n'a pas pu tourner.
    {
        printf '{\n  "permissions": {\n    "allow": [\n'
        for i in "${!_WORKTREE_GH_ALLOW_NARROW[@]}"; do
            if [ "$i" -lt $(( ${#_WORKTREE_GH_ALLOW_NARROW[@]} - 1 )) ]; then sep=,; else sep=; fi
            printf '      "%s"%s\n' "${_WORKTREE_GH_ALLOW_NARROW[$i]}" "$sep"
        done
        printf '    ]\n  }\n}\n'
    } > "$dst" 2>/dev/null || true
    echo "[dispatch-lib] worktree_allowlist: mode=narrow_only narrow=${#_WORKTREE_GH_ALLOW_NARROW[@]}" >&2
}
```

La ligne `worktree_allowlist:` suit la forme des sondes voisines (`rebase_gate:`,
`pr_origin.already_claimed`) et porte le seul fait qu'un opérateur voudra lire après coup :
le filtre a-t-il tourné, ou le worktree tourne-t-il sur la liste étroite nue.

L'écriture passe par `$tmp` + `mv` et non par une redirection directe sur `$dst`, parce que
`jq` qui échoue à mi-parcours sur une redirection directe laisse un JSON tronqué que Claude
Code lirait comme « aucune permission » — un troisième comportement, ni filtré ni fail-closed.
Le `[ -s "$tmp" ]` est la borne basse du même souci.

### D4 — Le site d'appel : le `cp` disparaît, il n'est pas doublé

À la ligne 6098, le bloc devient :

```sh
# Copy gitignored .claude/ config into worktree (relay + filtered allowlist)
mkdir -p "$WORKTREE_DIR/.claude"
cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
# mika#3030 : l'allowlist est CONSTRUITE, jamais copiée. Un `cp` ici rendrait au
# worktree les jokers de famille de l'opérateur, et avec eux le court-circuit de
# `can_use_tool` qui a ouvert mika#3024 sans ligne de décision.
_seed_worktree_settings_allowlist "$PLATFORM_DIR" "$WORKTREE_DIR"
```

Le `cp` de `claude-pilot.json` reste : c'est la config du relais, pas une allowlist, et le
`CWD_ARGS`/`--relay-config` juste en dessous en dépend.

La fonction se place auprès de ses deux sœurs de seeding (`_seed_worktree_slash_commands` à
4659, `_seed_pilot_scratch_dir` à 4714) — même famille, même signature
`(platform_dir, worktree_dir)`, même emplacement.

### D5 — Deux commentaires de scaffold à amender, parce qu'ils citent un `cp` qui n'existera plus

Les lignes 7307 et 11936 décrivent l'inventaire des artefacts de scaffold que le rescue refuse
de stager (mika#1419 / mika#1552). La ligne 7307 dit *« operator permission allowlist cp'd at
:490 »*. Après le vol, ce n'est plus une copie et ce n'est plus l'allowlist de l'opérateur.

L'exclusion elle-même (`RESCUE_EXCLUDE_PATHSPEC`, l. 7346) **ne change pas** : le fichier reste
un artefact de scaffold que le pilote ne doit pas committer — il est maintenant *produit* par
dispatch-lib au lieu d'être *copié* par elle, ce qui le rend si possible plus clairement « à
nous, pas au pilote ». Le test E existant (`test_auto_rescue_excludes_settings_local_json`,
l. 2293) reste vert sans modification : il écrit sa propre fixture et n'observe que le pathspec.

### D6 — Le test d'AC1 implémente un matcher, pas une recherche de chaîne

L'AC énonce une couverture (« aucune règle allow ne **couvre** une commande de L »), pas une
absence de sous-chaîne. Un `grep -v 'gh issue'` passerait sur `Bash(gh issue *)` tout en
manquant `Bash(gh:*)`. Le test implémente donc la sémantique de préfixe de Claude Code, qui
tient en une normalisation et un glob bash :

```sh
# `Bash(X)` → X ; X terminé par `:*` → préfixe, donc `X*` ; puis glob bash natif.
# `[[ ]]` ne fait pas de word-splitting, donc un motif à espaces tient sans quoting.
_t3030_covers() {
    local entry=$1 cmd=$2 pat
    case "$entry" in Bash\(*\)) pat=${entry#Bash(}; pat=${pat%)} ;; *) return 1 ;; esac
    case "$pat" in *:\*) pat="${pat%:\*}*" ;; esac
    # shellcheck disable=SC2053
    [[ $cmd == $pat ]]
}
```

Vérifié sur les trois formes réelles de la fixture : `Bash(gh pr:*)` → `gh pr*` couvre
`gh pr merge 1` ; `Bash(gh issue *)` couvre `gh issue create --title x` ; `Bash(gh pr edit:*)`
→ `gh pr edit*` couvre `gh pr edit 1 --add-label origin:loop` et **ne** couvre **pas**
`gh pr merge 1`.

**Exécuté au groom, pas supposé.** Le matcher, le prédicat jq de D2 et les trois familles
R1/R2/R3 ont été joués sur la fixture de D7 dans ce worktree
(`.pilot-scratch/groom-3030/probe.sh`) : le matcher est conforme sur 7 cas sur 7, le filtre
rend les 4 entrées non-`gh` plus les 9 étroites, les 9 commandes de L sont toutes refusées et
les 6 gestes d'AC2 tous permis. Ce que l'implémentation doit faire n'est donc pas une
conjecture sur le comportement de `jq` ni sur le glob de `[[ ]]` — c'est la transcription d'un
résultat mesuré.

### D7 — Quatre familles d'assertion, et pourquoi chacune doit exister

La sonde est **fonctionnelle** : elle source `dispatch-lib.sh` (le pattern déjà utilisé une
dizaine de fois dans la suite — l. 694, 4543, 4709, 4765…), bâtit une fixture
`platform/.claude/settings.local.json`, appelle `_seed_worktree_settings_allowlist`, et lit le
fichier produit.

La fixture porte exactement ce que le ticket a mesuré, plus les témoins :

```json
{ "permissions": { "allow": [
    "Bash(gh pr:*)", "Bash(gh issue:*)", "Bash(gh repo:*)",
    "Bash(gh issue *)", "Bash(gh label *)", "Bash(gh issue view:*)",
    "Bash(cargo clippy:*)", "Bash(ls:*)", "Read(//home/x/**)", "WebSearch"
] } }
```

1. **Refus (R1).** Pour chacune des 9 commandes de L, aucune entrée du résultat ne la couvre.
2. **Témoin positif (R2).** Pour chacun des 6 gestes d'AC2 — `gh pr create --title x`,
   `gh pr view 1`, `gh pr edit 1 --add-label origin:loop`, `gh pr ready 1`, `gh issue view 3030`,
   `gh issue list --search x` — au moins une entrée le couvre.
3. **Contrôle négatif d'AC1.** `Bash(gh issue view:*)` est présente dans le worktree. Elle y est
   par **réinjection** (D1 jette toutes les entrées `gh` avant d'ajouter la liste close), pas
   par préservation — l'AC porte sur l'observable (« reste présente dans le worktree »), et
   l'observable est identique. Noté ici pour qu'une relecture ne le prenne pas pour un
   contournement.
4. **Contrôle positif du filtrage (R3).** `Bash(cargo clippy:*)`, `Bash(ls:*)`,
   `Read(//home/x/**)` et `WebSearch` survivent. **Sans cette famille, la 1 passerait
   trivialement** sur un filtre qui jette tout — et on aurait livré un rétrécissement général
   en croyant livrer un filtre.

Deux cas de repli, dans la même sonde : source absente, et jq indisponible (simulé par un
`PATH` sans jq dans un sous-shell). Les deux doivent rendre un fichier qui porte la liste
étroite, ne couvre aucune commande de L, et **ne porte aucune** entrée de la fixture — la
preuve que le repli n'a pas copié.

### D8 — Le scan d'AC4, et la seconde porte qu'il doit aussi fermer

```sh
# AC4 — aucun `cp` d'un fichier de réglages vers le worktree. La cible est le
# GESTE, pas la mention : les commentaires de scaffold (l. 7307, 11936) nomment
# `settings.local.json` sans `cp` devant, et le pathspec d'exclusion (l. 7346)
# non plus.
_t3030_cp_count=$(grep -cE '^[^#]*\bcp\b[^;&|]*settings\.local\.json' "$DISPATCH_LIB" || true)
assert_eq "mika#3030 AC4: aucun cp d'un settings.local.json dans dispatch-lib.sh" 0 "$_t3030_cp_count"
```

Le `^[^#]*` écarte une ligne de commentaire ; le `[^;&|]*` borne la lecture au segment du `cp`,
de sorte qu'un `cp a b && echo settings.local.json` ne compte pas.

Le scan assert `== 0`, jamais `<= N`. Un seuil est une exception sans date d'expiration, et
c'est précisément ce que la Fire-Disposition ci-dessous refuse.

**La seconde porte.** D2 ne tient R1 que pour la source `local`. La source `project` —
`.claude/settings.json`, tracké — se tient aujourd'hui par le seul fait qu'elle ne porte pas de
clé `permissions`, et rien ne garde cette absence : un `permissions.allow` ajouté là rendrait
`Bash(gh issue:*)` au worktree par l'autre porte, avec le dépôt pour véhicule et le filtre de
D2 impuissant. Une assertion de plus, sur la même ligne de raisonnement :

```sh
# La source `project` tient par une ABSENCE, donc elle a besoin d'une assertion.
# Son $comment déclare déjà l'invariant : « noms de fichiers distincts ET clés
# disjointes ». Ici on l'épingle du côté qui compte pour mika#3030.
assert_eq "mika#3030 AC1: le settings.json tracké (source project) ne porte aucune permission" \
    "null" "$(jq -r '.permissions // "null"' "$REPO_ROOT/.claude/settings.json")"
```

## Ordre de commit

Un seul commit. Le filtre et son test sont la même affirmation : séparer les deux ferait
atterrir, entre les deux, un commit où le worktree reçoit les jokers et où rien ne le dit.

1. `fix(3030)` — `_WORKTREE_GH_ALLOW_NARROW` + `_seed_worktree_settings_allowlist` +
   remplacement du `cp` à 6098 + les deux commentaires de scaffold de D5 + la sonde
   fonctionnelle et les deux scans dans `test-dispatch-lib.sh`.

Plancher : `bash skills/bundled/_shared/test-dispatch-lib.sh` vert, `shellcheck` propre sur les
deux fichiers touchés.

## Verification Contract

| # | Geste | Attendu |
|---|---|---|
| V1 | `bash skills/bundled/_shared/test-dispatch-lib.sh` | `FAIL: 0`, et le compte `PASS` a crû du nombre d'assertions ajoutées |
| V2 | `shellcheck skills/bundled/_shared/dispatch-lib.sh skills/bundled/_shared/test-dispatch-lib.sh` | aucun nouveau diagnostic (la seule `disable` ajoutée est le `SC2053` de D6, qui est le glob volontaire) |
| V3 | `grep -cE '^[^#]*\bcp\b[^;&|]*settings\.local\.json' skills/bundled/_shared/dispatch-lib.sh` | `0` |
| V4 | `jq -r '.permissions // "null"' .claude/settings.json` | `null` |
| V5 | `make verify-bundled-skills` | vert (aucun bundle touché, mais `_shared/` l'est) |
| V6 | `cargo test` | vert — aucun code Rust touché, lancé comme plancher de non-régression |
| V7 | `bash scripts/verify-pipeline.sh origin/main` | vert |
| V8 | Lecture à l'œil du `settings.local.json` produit par la sonde (la sonde l'écrit sous `.pilot-scratch/` quand `MIKA_T3030_KEEP=1`) | ne porte aucun joker de famille `gh` |

## Definition of Done

- `_seed_worktree_settings_allowlist` existe, est appelée au site unique de préparation du
  worktree, et aucun `cp` de fichier de réglages ne subsiste dans `dispatch-lib.sh`.
- La liste étroite est une constante nommée, close, où chaque entrée nomme une sous-commande.
- Le repli est fail-closed et observable par une ligne `worktree_allowlist: mode=…`.
- La sonde fonctionnelle couvre les quatre familles de D7 plus les deux cas de repli.
- Les deux scans de D8 sont en place et verts.
- V1–V7 passent.
- La section `## Fire-Disposition` de ce plan est honorée dans le code livré.

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#3030.

- **AC1.** Dans un worktree préparé par `dispatch-lib.sh`, aucune règle allow des réglages
  `project` et `local` ne couvre une commande de la liste L : `gh issue create`,
  `gh issue close`, `gh issue edit`, `gh issue comment`, `gh pr merge`, `gh pr review`,
  `gh pr close`, `gh repo delete`, `gh label create`. Test dans `test-dispatch-lib.sh`, sur une
  fixture `settings.local.json` qui porte `Bash(gh issue:*)` et `Bash(gh pr:*)`. Dans le même
  test, contrôle négatif : une entrée de lecture (`Bash(gh issue view:*)`) reste présente dans
  le worktree.
- **AC2 (mika).** Les gestes du pipeline qui passent aujourd'hui par l'allow copié restent
  permis dans le worktree, par des entrées étroites écrites à la place des jokers retirés :
  `gh pr create`, `gh pr view`, `gh pr edit` et `gh pr ready`, plus les lectures
  `gh issue view` et `gh issue list`. Un témoin positif par entrée est ajouté au test d'AC1.
  Sans ces entrées, les 99 appels `gh pr edit` mesurés sur 85 vols seraient refusés.
- **AC4.** `dispatch-lib.sh` ne copie dans le worktree aucun fichier de réglages qui n'a pas été
  filtré par AC1. Un scan, épinglé par un test, refuse un deuxième `cp … settings.local.json`.
- **AC5.** Sonde : sur les 5 vols de la boucle qui suivent le deploy, on compte les tickets dont
  l'auteur est `mika-platform-dev` et la date de création tombe dans la fenêtre du vol, et on
  écrit le compte sur le ticket. La cible est 0.

### Comment chaque AC est tenu

| AC | Tenu par | Preuve |
|---|---|---|
| AC1 (`local`) | D1 (liste close, aucun membre de L) + D2 (rejet de toute entrée `gh`) | D7 famille 1, matcher de D6 sur les 9 commandes de L |
| AC1 (`project`) | L'absence de clé `permissions` dans le `settings.json` tracké | second scan de D8 |
| AC1 (contrôle négatif) | `Bash(gh issue view:*)` dans la liste étroite de D1 | D7 famille 3 |
| AC2 | Les 9 entrées étroites de D1 | D7 famille 2 — un témoin par geste |
| AC4 | D4 (le `cp` disparaît) | premier scan de D8, V3 |
| AC5 | **Hors code.** Mesure post-deploy, écrite en commentaire sur mika#3030 | — voir ci-dessous |

**AC5 n'est pas un livrable de code et ce plan ne lui en invente pas un.** C'est une mesure
après le `make deploy` qui embarque ce changement : sur les 5 vols suivants, le compte des
tickets d'auteur `mika-platform-dev` créés dans la fenêtre de chaque vol, cible 0. Le gel
interdisant d'ouvrir un ticket de suivi, ce compte va en **commentaire sur mika#3030**, qui
reste ouvert jusque-là. Le plan le nomme pour que la fermeture du ticket ne soit pas confondue
avec le merge de la PR.

## Fire-Disposition

Le vol livre trois détecteurs : la sonde fonctionnelle de D7, le scan `cp` de D8, et le scan
`permissions`-dans-`settings.json` de D8. Chacun a pour chemin de succès « aucune violation
trouvée ».

**Option retenue : (a) — exception nommée en allowlist, avec un registre VIDE à l'atterrissage.**

Le registre est vide, et ce n'est pas une omission — c'est une propriété du vol, vérifiable
avant de l'écrire :

| détecteur | violations au moment du merge | pourquoi zéro |
|---|---|---|
| scan `cp … settings.local.json` | 0 | le **même commit** remplace l'unique `cp` (l. 6098) par l'appel de D4. `grep -n "settings\.local\.json"` sur le fichier ne rend que trois autres lignes, aucune n'étant un `cp` : le pathspec du rescue (7346) et deux commentaires (7307, 11936), tous trois hors du motif de D8 |
| scan `permissions` dans `.claude/settings.json` | 0 | le fichier tracké ne porte que `hooks`, vérifié dans ce worktree |
| sonde fonctionnelle de D7 | 0 | elle exerce le code livré par ce commit, pas un état antérieur |

Le premier scan a été **joué au groom sur l'état actuel du fichier** : il rend exactement `1`,
et la seule ligne matchée est le `cp` de 6098 — ni le pathspec de 7346 ni les commentaires de
7307/11936, parce que son `^[^#]*` écarte une ligne de commentaire et que les deux mentions
placent `settings.local.json` **avant** tout `cp`. Le compte passe donc de 1 à 0 par le seul
geste de D4, dans le même commit, et il n'y a aucune ligne résiduelle à inscrire en exception.

**Conséquence sur la forme des assertions.** Un registre vide n'autorise pas un détecteur
indulgent. Les deux scans assertent une **égalité stricte à 0**, jamais un `<= N` ni un
`|| true` qui avale un rc non nul. L'auto-nettoyage que mika#1574 demande d'une exception est
ici obtenu par sa non-existence : il n'y a aucune entrée à devenir stale, donc rien à faire
rougir. Si une exception devait un jour s'ajouter, elle devrait nommer la ligne précise, porter
un numéro de ticket, et le scan devrait rougir quand cette ligne disparaît — mais **ce vol
n'en introduit aucune**, et l'ajouter « au cas où » serait créer la première entrée stale de
sa propre allowlist.

Ni (b) ni (c) ne s'appliquent : rien à désarmer puisque les trois détecteurs atterrissent verts
sur le code du même commit, et aucun cadrage opérateur n'est en suspens — le ticket a déjà
tranché le périmètre (mika seulement) et nommé le hors-vol.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_WORKTREE_GH_ALLOW_NARROW` + commentaire doctrinal (D1) | 28 |
| `skills/bundled/_shared/dispatch-lib.sh` — `_seed_worktree_settings_allowlist` + repli fail-closed (D3) | 42 |
| `skills/bundled/_shared/dispatch-lib.sh` — site d'appel à 6098 (D4) + commentaires de scaffold (D5) | 12 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — matcher `_t3030_covers` (D6) | 14 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — sonde fonctionnelle, fixture + 4 familles + 2 replis (D7) | 120 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — les deux scans (D8) | 35 |

Total estimé : 251 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 251 est sous les deux bornes — 429 par vol et 1000
par PR — avec une marge de 178 lignes sur la plus serrée. Trois raisons de fond, au-delà du
nombre :

1. **Deux fichiers, et ils sont voisins.** Les deux vivent dans `skills/bundled/_shared/`.
   Aucun code Rust, aucune migration de schéma, aucun `build.rs`, aucun front — donc aucun
   cycle de compilation dans la boucle d'itération, et le plancher est un `bash` de quelques
   secondes au lieu d'un `cargo test` de plusieurs minutes.
2. **Le volume est dans le test, pas dans le mécanisme.** 169 des 251 lignes sont des
   assertions, et une assertion qui rate se lit sur son propre label — elle ne demande pas de
   re-déduire la cause. Le mécanisme lui-même fait 82 lignes, dont une fonction et une
   constante.
3. **Pas de découpage possible qui soit mergeable seul.** Scinder le filtre de son test
   ferait atterrir un commit où le worktree reçoit les jokers et où rien ne le dit — un vol
   dont le seul effet mesurable serait de rendre le défaut indétectable une PR de plus.

Aucune ligne `Tracked in:` : il n'y a pas de suite à nommer dans ce dépôt. La partie
claude-pilot est hors vol par décision du ticket et sera faite par un siège interactif sur
l'autre dépôt — ce n'est pas une phase différée de ce plan, c'est un autre périmètre.

## Hors vol

Repris du ticket, et rien n'est ajouté :

- **La partie claude-pilot** : un test de `can_use_tool` par commande de L, des règles
  explicites dans `permissions.yaml` pour `gh pr edit` et `gh pr ready`, et une décision écrite
  pour `gh issue comment` et `gh issue edit`. Le sandbox du pilote ne monte que le worktree
  mika (groom e71ada95, 03/11) : ce vol ne peut pas y toucher. Siège interactif, sur
  claude-pilot. C'est la **défense en profondeur** du présent vol, pas son prérequis : D1
  tient AC1 sans elle, et elle tiendra une seconde fois le jour où quelqu'un rouvrira un joker.
- **Le contenu de `mika-platform/.claude/settings.local.json`** — l'allowlist de la session de
  l'opérateur. Intacte. Le vol change ce que le worktree reçoit, jamais ce que l'opérateur a.
- **`~/.claude/settings.json` (source `user`)**, qui porte `Bash(gh pr merge:*)` (l. 5). Il
  n'est pas mesuré que le pilote lise ce fichier dans son sandbox, et ce vol ne le mesure pas
  davantage. S'il le lisait, `gh pr merge` resterait couvert malgré AC1 — un fait à établir sur
  claude-pilot, avec le reste.
- **Le repli conçu au groom de mika#3024** (plan `2026-11-03-005`, § 4b et risque R1), qui a
  remplacé un `sed` refusé par un découpeur Rust.
- **La mesure du corps du ticket.** Déjà faite. Ni ce groom ni l'implémentation ne lisent
  `~/.mika/data/pilot-transcripts/` : c'est hors du worktree, et un refus de la politique y a
  tué le premier groom.
- **Le gel des tickets jusqu'au 01/12.** Ce vol n'ouvre aucun ticket de suivi. Un reste
  éventuel va en commentaire sur mika#3030 ou dans la PR.

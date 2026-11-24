# mika#3142 — le secours de fin de vol lit la halte déclarée par le pilote

**Vol 2a de mika#3119** — détecteur armé et attesté. Le refus structurel, la garde
de `_push_branch` et l'énumération Rust partent au vol 2b (`Tracked in:` § *Vol 2b*).

Ticket : senara-solutions/mika#3142
Branche : `fix/3142/dispatch-lib-le-secours-lit-la-halte-d`
Sources : mika#3119 (corps + commentaires 40, 46, 47, 49, 52, 57) et son plan
`docs/plans/2026-11-22-003-fix-3119-dispatch-lib-le-secours-wip-rescue-plan.md`
§ *Défaut B* (porte la mesure des deux logs) et § *Vol 2*.

---

## Le défaut

Une **halte déclarée par le pilote** — la borne par vol de mika#2693 dépassée, donc
pas de commit, pas de push, pas de PR, et une dernière ligne de la forme
*« HALTE borne : N lignes … pour `M` »* — est **invisible** au secours de fin de vol.

La chaîne, mesurée :

1. `_run_claude_pilot` (`skills/bundled/_shared/dispatch-lib.sh:6590`) lit du JSON de
   `claude-pilot` exactement huit champs — `status`, `session_id`, `turns`, `subtype`,
   `termination_reason`, `api_error_status`, `cost_usd`, `duration_ms`
   (6691-6722). **Aucun ne porte le dernier message du pilote.** C'est la racine.
2. La signature observable d'un tour propre sans queue de livraison — `STATUS=success`,
   `SKILL=dev-pilot`, `PILOT_SHIPPING_TAIL=absent` — est **exactement** celle que
   `_pilot_had_no_shipping_tail` (8165-8196) reconnaît, et donc exactement la classe
   `no-shipping-tail` de mika#2492 : le **chemin nominal** de la boucle autonome sous
   les queues allégées de mika#2694.
3. Le secours pousse la branche (`_push_branch`, appelé en 14818) puis ouvre une draft
   (bloc Unit 2, 14958).
4. Sur **#3081** l'effet est allé plus loin : la draft a porté
   `rescue-pipeline-verified: yes`, puis le démon `auto_resume_wip_rescue_drafts`
   (mika#1852) l'a **dé-draftée** — le travail halté est parti en revue sans décision.

Mesuré sur **#3081, #3092, #3100, #3108**.

Ce ticket est côté `dispatch-lib.sh` et porte l'abstention en amont ; mika#3120 traite
le dé-draftage côté démon (`wip_rescue.rs`). Les deux sont complémentaires.

## La seule source de la déclaration, et pourquoi un `grep` nu est un faux positif garanti

La déclaration n'existe **que** dans le log de session, déjà adressé par le global
`_SESSION_LOG_PATH`.

```sh
# dispatch-lib.sh:6678 — dans _run_claude_pilot, SANS `local` : c'est un global.
_pilot_log_dir; _SESSION_LOG_PATH="$_PILOT_LOG_DIR/${LOG_ID}.log"
```

Il est posé en 6678, gardé en 6679-6683 par un test `-s` (pas `-f`) qui émet
`pilot_log_guard.missing` (mika#2165). `_run_claude_pilot` est appelé **sans
sous-shell** en 14633 depuis `dispatch_claude_pilot`, et le bloc Unit 2 (14830+)
vit dans cette même fonction, après. **Donc le global est visible au point de
décision : aucun nouvel appel `gh`, aucun tour de réseau, aucune re-dérivation.**

Un `grep "HALTE borne"` nu rendrait un **faux positif garanti**, et la mesure le dit
d'où il vient : `grep -rn "HALTE borne"` sur `skills/`, `.claude/` et `docs/` ne trouve
la chaîne **que dans `docs/plans/*.md`** — dix-huit fichiers de plan, chacun écrivant
la règle. C'est le **plan injecté dans le prompt du pilote** qui porte la citation, près
de la tête du log, et c'est la seule occurrence hors queue.

Les deux logs mesurés sur #3119 § *Défaut B* :

| log | citation de la règle (tête) | déclaration (queue) | distance à l'EOF | taille |
|---|---|---|---|---|
| `4492a670-…` | l. 100 | l. 9630 — `**HALTE borne : 379 lignes ajoutées pour \`310\`**` | 24 l. | 9 654 l. |
| `5813f896-…` | l. 88 | l. 7268 — `**HALTE borne : 455 lignes pour 450.**` | 26 l. | 7 294 l. |

D'où **deux ancrages indépendants**, et la nécessité des deux :

- **des chiffres immédiatement après `HALTE borne :`** — les dix-huit citations mesurées
  écrivent le littéral `N` à cet emplacement (`HALTE borne : N lignes pour 540`,
  `HALTE borne : N lignes ajoutées pour \`420\``), jamais un chiffre ;
- **une fenêtre de queue** — parce qu'un plan *peut* citer la règle avec un nombre réel
  à la place de `N`, et parce que les citations vivent à 88 et 100 lignes de la tête
  tandis que les déclarations vivent à 24 et 26 lignes de l'EOF.

Chacun seul laisse passer un cas que l'autre ferme : c'est ce que les contre-témoins
V3 et V4 de la suite attestent, séparément.

## Ce que livre CE vol (2a)

Le corps du ticket prescrit le découpage, mot pour mot : *« Il devra être découpé à son
tour (détecteur armé + suite d'abord, refus + garde de push ensuite). »* Ce n'est donc
pas une coupe que je pratique, c'est l'instruction du ticket.

1. `_pilot_declared_halt` — le prédicat, **armé** : sa suite tourne en CI et rougit sur
   une régression. Aucun consommateur en production : **aucun changement de comportement
   observable** sur un vol réel.
2. `test_pilot_halt_declared.sh` — la suite offline.
3. Son **step nommé** dans le job `dispatch-lib-offline-suites` de `ci.yml`.
4. Une target `Makefile` de lancement local, qui n'est **pas** la porte de merge.

### Le prédicat

```sh
# Fenêtre de queue, en lignes. 120 : cinq fois la distance mesurée la plus
# grande (26 l. sur `5813f896-…`), et très en dessous des 88 l. de la citation
# de tête la plus basse. Recalibrage : la distance est bornée par la longueur de
# la queue de livraison, pas par la taille du log.
_PILOT_HALT_TAIL_WINDOW=120
_PILOT_HALT_DECL_RE='HALTE borne[[:space:]]*:[[:space:]]*[0-9]'

_pilot_declared_halt() {
    local _tail
    [ -n "${_SESSION_LOG_PATH:-}" ]                                   || return 1
    [ -s "$_SESSION_LOG_PATH" ]                                       || return 1
    _tail=$(tail -n "$_PILOT_HALT_TAIL_WINDOW" -- "$_SESSION_LOG_PATH" 2>/dev/null) \
                                                                      || return 1
    [ -n "$_tail" ]                                                   || return 1
    grep -qE -- "$_PILOT_HALT_DECL_RE" <<<"$_tail"                    || return 1
    return 0
}
```

Trois propriétés de forme, chacune mesurée ailleurs dans ce fichier :

- **Modèle structurel.** C'est celui de `_pilot_had_no_shipping_tail` (8165-8196) :
  une conjonction de gardes qui `return 1`, un `return 0` final, et rien d'autre.
- **`<<<` et non un pipe — correctif mesuré, pas une préférence.** `grep -q` sort au
  premier match et ferme le tuyau ; le producteur qui écrit encore prend SIGPIPE et sort
  en 141 ; sous le `set -o pipefail` de ce fichier ce 141 **devient le statut du
  pipeline**, et le prédicat rendrait « pas de halte » pour un log qui la porte. C'est
  mika#2055, écrit en doctrine aux lignes 907-920 et 3912-3915, gardé par
  `scripts/verify-no-sigpipe-grep.sh` — dont le motif n'apparie que les producteurs
  `printf`/`echo`, donc un `tail | grep -q` **passerait sous le lint** tout en portant
  la régression. Pire : sur une fenêtre de 120 lignes le dépassement du tampon de pipe
  (64 Ko) dépend de la longueur des lignes, donc le faux négatif serait
  **intermittent**. Le cas V8 de la suite existe pour cela, et seulement pour cela.
- **`-- ` devant le chemin et devant le motif** : un `LOG_ID` ou un motif commençant
  par `-` ne devient pas une option.

**Direction du fail-safe, et elle n'est pas symétrique.** Un prédicat vrai à tort
**retiendrait** un artefact — au vol 2b, un push et une draft. C'est exactement la perte
que mika#1282 existe pour empêcher, et la doctrine du fichier sur la direction d'un
refus qui RETIRE quelque chose est déjà écrite : mika#3038 ne refuse rien sur
`_tree_rc=2` et appelle `_note_tree_compare_undetermined` ; `_pilot_had_no_shipping_tail`
rend faux sur toute indétermination. `_pilot_declared_halt` suit : **toute
indéterminabilité rend « pas de halte »** — variable non posée, fichier absent, vide,
illisible, `tail` en échec. L'asymétrie est le point, et V9-V11 l'attestent.

### `## Tests`

Nouvelle suite `skills/bundled/_shared/tests/test_pilot_halt_declared.sh`, sur le
gabarit de `test_rescue_tree_equals_main.sh` : `set -uo pipefail`,
`source "$SCRIPT_DIR/../dispatch-lib.sh"`, appel de la **vraie** fonction. Les fixtures
sont des fichiers de log générés par un helper commun `_mk_log <total> <ligne> <texte>`.

Aucun dépôt git, aucun `gh` simulé, aucun réseau : le prédicat est une fonction pure
sur un fichier texte. La condition d'hermétisme de mika#2793 (`user.email` / `user.name`
déclarés sur le dépôt fixture) est donc **sans objet ici** — cette suite ne crée aucun
dépôt et ne commite rien, et le step CI n'a rien à poser.

| cas | fixture | attendu | ce qu'il ferme |
|---|---|---|---|
| **V1** | déclaration `**HALTE borne : 379 lignes ajoutées pour \`310\`**` à 24 l. de l'EOF, log de 9 654 l. | vrai | la forme mesurée n° 1, verbatim |
| **V2** | déclaration `**HALTE borne : 455 lignes pour 450.**` à 26 l. de l'EOF, log de 7 294 l. | vrai | la forme mesurée n° 2 (point final, pas de rétro-quotes) |
| **V3** | citation de règle à la l. 100 avec le littéral `N`, rien en queue | **faux** | le faux positif garanti du `grep` nu — ancrage « chiffres » |
| **V4** | citation à la l. 100 avec un **chiffre** (`HALTE borne : 540 lignes pour 540`), rien en queue | **faux** | ancrage « fenêtre », seul, isolé de V3 |
| **V5** | déclaration chiffrée à la l. 300 d'un log de 9 000 l. | **faux** | une déclaration hors fenêtre n'est pas la dernière |
| **V6** | déclaration à la 120ᵉ ligne avant l'EOF | vrai | la borne de `_PILOT_HALT_TAIL_WINDOW`, côté intérieur |
| **V7** | déclaration à la 121ᵉ ligne avant l'EOF | **faux** | la même borne, côté extérieur — sans V7, une fenêtre « tout le fichier » passerait V1-V6 |
| **V8** | déclaration en **première** ligne d'une fenêtre dont le volume dépasse 64 Ko (lignes longues), **200 itérations** | vrai ×200 | la régression SIGPIPE de mika#2055, et le seul cas qui distingue `<<<` d'un pipe |
| **V9** | `_SESSION_LOG_PATH` pointe un chemin inexistant | **faux** | fail-closed |
| **V10** | log de 0 octet (le cas `pilot_log_guard.missing`, mika#2165) | **faux** | fail-closed |
| **V11** | `_SESSION_LOG_PATH` vide, puis non défini, sous `set -u` | **faux**, sans erreur | fail-closed, et pas de mort par `unbound variable` |
| **S1** | assertion **textuelle** sur le corps de la fonction dans `dispatch-lib.sh` : aucun `|` menant à un `grep -q` | passe | l'ancre anti-régression de la doctrine mika#2055, que le lint existant ne couvre pas pour `tail` |
| **S2** | `grep` sur `.github/workflows/ci.yml` : la suite est nommée par un step du job `dispatch-lib-offline-suites` | passe | mika#2578 — un prédicat correct et un prédicat **lancé** sont deux faits, et seul le second ferme le ticket |

V1, V2, V6 et V8 ne sont pas du remplissage : sans elles, un prédicat qui rendrait faux
à tout passerait V3, V4, V5, V7, V9, V10 et V11, et livrerait un détecteur inerte.

**Ce qui n'est PAS testé ici, et pourquoi.** Le cas « log illisible » (mode `000`) est
écarté : le bac à sable de dispatch peut tourner en UID 0, pour qui `000` reste lisible,
donc le cas serait vert ou rouge selon l'hôte. Il est couvert en direction par V9 et
V10, et la garde `[ -s ]` ne distingue pas « absent » d'« illisible » de toute façon.

### `## Câblage CI`

Un step **nommé** dans le job `dispatch-lib-offline-suites` de `.github/workflows/ci.yml`
(job à ~706), placé après le précédent le plus proche —
`test_rescue_tree_equals_main.sh`, l. 755, *« A rescue draft is not opened on a tree
already in main (mika#3038) »* :

```yaml
      # Une halte déclarée par le pilote est un fait de la QUEUE du log, jamais
      # de son corps : la règle est citée près de la tête (mika#3142).
      - name: A pilot-declared halt is read from the session log's tail (mika#3142)
        run: bash skills/bundled/_shared/tests/test_pilot_halt_declared.sh
```

Et une target `Makefile`, sur le patron exact de `test-rescue-tree-equals-main`
(228-232), **avec son commentaire** : ce target lance la suite SEULE ; sa porte de merge
est le step nommé ci-dessus, pas ce target — un target qu'aucun workflow ne nomme ne
refuse rien (mika#2578). La target va aussi dans la liste `.PHONY` (l. 4).

Rien à ajouter à `scripts/suite-wiring.tsv` : ce registre ne porte que le **résidu**, les
suites qu'aucun workflow n'atteint, et son champ `unwired=` est à zéro depuis mika#2793.
Une suite câblée par un step y aurait une entrée que la comparaison bidirectionnelle du
lint ferait **rougir**.

Rien à ajouter à `scripts/canonical-tokens.tsv` : il déclare le **token** `NO_PR`
(classe B, site `dispatcher.rs::RE_NO_PR`), jamais ses motifs individuels. Et de toute
façon ce vol n'émet aucun `NO_PR:` — c'est le vol 2b qui le fait.

### `## Ordre de commit`

1. `_pilot_declared_halt` + ses deux constantes, avec l'en-tête d'ancrage (les deux logs
   mesurés, les deux ancrages et pourquoi chacun seul ne suffit pas, la direction du
   fail-safe, la population #3081/#3092/#3100/#3108, la raison du `<<<`). Placement : à
   côté de `_pilot_had_no_shipping_tail` (8165-8196), dont il est le pendant.
2. La suite `test_pilot_halt_declared.sh`, V1 → V11 puis S1, S2.
3. Le step `ci.yml` et la target `Makefile` + `.PHONY`, dans le **même** commit : une
   suite sans step est précisément ce que `suite-wiring-lint` refuse.

**Plancher mergeable** : (1) + V1-V5, V8-V11, S1, S2 + (3). Si la suite déborde son
estimation, **V6 et V7** sont les deux premiers cas à partir au vol 2b — ils attestent la
valeur de la borne, pas l'existence des deux ancrages. Rien d'autre ne part : retirer un
contre-témoin livrerait un détecteur non attesté, et retirer le step livrerait une suite
que rien ne lance.

## Décision réservée à l'opérateur — ne pas trancher ici

Le plan du vol 1 l'a laissée ouverte, textuellement : *« la garde de `_push_branch` —
**l'arbitrage reste ouvert** : le ticket demande "ni push ni PR", la doctrine du fichier
dit *rien n'est perdu* et les deux refus existants ne retiennent que la draft. Le vol 2
doit trancher en nommant le coût. »*

**La question :** sur une halte déclarée, le secours s'abstient-il de la **draft seule**,
ou du **push et de la draft** ?

| option | geste | coût, nommé | taille |
|---|---|---|---|
| **1 — draft seule** | le patron des deux refus existants : `_push_branch` tourne, la branche est poussée, `_recovery_pr_due=0`, `NO_PR: pilot_halt_declared`, commande de publication manuelle dans `RESULT`. `_push_branch` n'est pas touché. | La branche existe sur `origin` : le travail halté est **conservé**, et sans draft, `auto_resume_wip_rescue_drafts` (mika#1852) n'a rien à dé-drafter — l'effet mesuré sur #3081 est fermé. Mais une branche poussée sur une halte est un artefact que personne ne réclame, et **le corps du ticket demande « ni push ni PR »**. | ~45 l. dans Unit 2 |
| **2 — push et draft** | option 1 **plus** une garde en tête de `_push_branch` (9188-9207), sur le précédent `Push: SKIPPED — … ; return 1` de `_check_duplicate_commits`. | Le travail halté reste **local au worktree**, et il est **perdu** si le worktree est nettoyé. Cela contredit la doctrine *rien n'est perdu* qui est la raison d'être de mika#1282 et la phrase que les deux refus existants écrivent dans `RESULT`. Contrainte supplémentaire : la garde ne doit pas faire rougir `scripts/check-pilot-push-sites.sh` (`make test-pilot-push-guard`, Makefile 248-251). | ~45 l. Unit 2 + ~30 l. `_push_branch` |

**Je n'en pratique aucune.** Le vol 2a ne touche ni `_push_branch`, ni le bloc Unit 2,
ni `dispatcher.rs` : il livre le prédicat et son attestation. **L'arbitrage ne bloque
donc pas ce vol** — il est un prérequis du **vol 2b**, et c'est le grooming du vol 2b qui
doit le porter à l'opérateur (mika#3122 §2 : chiffrer les options, n'en pratiquer aucune).

## Vol 2b — ce que ce vol NE livre PAS

`Tracked in: senara-solutions/mika#3142`

Ce ticket reste le tracker. La PR du vol 2a portera donc **`Refs mika#3142`**, jamais
`Closes` — et c'est le défaut A de mika#3119 lui-même, qui n'est pas corrigé par ce vol.

Périmètre du vol 2b, dans l'ordre de dépendance :

1. **L'arbitrage ci-dessus, tranché par l'opérateur.** Rien d'autre ne peut partir avant.
2. Le **troisième refus structurel** du bloc Unit 2, sur le patron de mika#3038
   (14867-14909) et mika#2503 (14930-14956) : après le calcul de `_recovery_pr_due`
   (14862-14865) et **avant** le bloc d'ouverture de draft (14958) — `_recovery_pr_due=0`,
   un `echo` de diagnostic sur stderr (convenance, jamais une sonde : sur un dispatch
   réussi le tuyau de `spawn_long_running_exec` est abandonné sans lecture — Signal M,
   mika#2050), la prose dans `RESULT`, la commande de publication manuelle, et
   `_set_pr_status_line "NO_PR: pilot_halt_declared"` — qui garantit **une seule** ligne
   de statut, la fonction (3306) effaçant toute `PR:`/`NO_PR:` antérieure.
3. Une ligne `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: …` via `_set_outcome_line`
   (3322), sur le précédent de 8946.
4. La **garde de `_push_branch`**, si et seulement si l'opérateur retient l'option 2.
5. L'**énumération de `parse_no_pr_reason`** (`crates/mika-agent/src/task_engine/dispatcher.rs:6866-6887`) :
   le commentaire de doc, qui se déclare lui-même *« the only enumeration an operator
   reads »*, et la table de `test_parse_no_pr_reason_known_reasons` (8276-8296, 7 entrées).
   Les deux **dans le même commit que le site émetteur** — un motif émis que
   l'énumération ne nomme pas est un motif qu'aucun opérateur ne peut lire.

## Risques

| risque | parade |
|---|---|
| Le pilote déclare la halte dans une forme que le motif n'apparie pas (accentuation, emphase Markdown, retour à la ligne après le `:`). | Le motif est volontairement **court** — `HALTE borne`, espaces libres, `:`, espaces libres, un chiffre — et tolère l'emphase `**…**` de part et d'autre. V1 et V2 sont les deux formes réellement mesurées, verbatim. Une forme nouvelle rend « pas de halte » : la direction du fail-safe, donc rien n'est retenu à tort. |
| `_PILOT_HALT_TAIL_WINDOW=120` est dépassé parce que la queue de livraison s'allonge. | Le faux négatif est la direction sûre. La constante est nommée, commentée avec sa provenance (5 × la distance mesurée maximale), et attestée à sa borne par V6/V7 : la recalibrer est une ligne. |
| `_SESSION_LOG_PATH` devient `local` dans une refonte de `_run_claude_pilot`, et le prédicat lit une variable vide. | Le prédicat rend alors « pas de halte » et rien ne casse en silence au vol 2b. V11 fixe le comportement ; l'en-tête du prédicat nomme la dépendance au global de 6678 et à l'appel non-sous-shellé de 14633. |
| La suite déborde son estimation et le vol passe la borne de 429. | Plancher déclaré au § *Ordre de commit* : V6 et V7 partent au vol 2b. |
| Un détecteur sans consommateur se lit comme le défaut mika#2578. | Ce n'en est pas un : mika#2578 porte sur une **suite qu'aucun workflow n'atteint**, et le step CI de ce vol l'atteint — S2 l'atteste. Ce qui est différé est la **conséquence en production**, pas l'armement. Et le découpage est celui que le corps du ticket prescrit. |

## Definition of Done

- `_pilot_declared_halt` existe dans `skills/bundled/_shared/dispatch-lib.sh`, avec ses
  deux constantes et son en-tête d'ancrage, et sans aucun pipeline vers `grep -q`.
- `skills/bundled/_shared/tests/test_pilot_halt_declared.sh` existe, passe hors réseau,
  et porte les treize cas V1-V11 + S1 + S2.
- Un step nommé du job `dispatch-lib-offline-suites` de `ci.yml` lance la suite.
- `make test-pilot-halt-declared` lance la suite localement, et son commentaire dit
  qu'il n'est pas la porte de merge.
- `scripts/verify-pipeline.sh origin/main`, `make verify-no-sigpipe-grep`,
  `make check-suite-wiring`, `make verify-bundled-skills`, `cargo clippy` et
  `cargo fmt --check` passent.
- Aucun comportement observable d'un vol réel n'est modifié : le prédicat n'a aucun
  consommateur dans ce vol.
- La PR porte `Refs mika#3142` (pas `Closes`), une ligne
  `Tracked in: senara-solutions/mika#3142`, et
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Dérivés : le corps de senara-solutions/mika#3142 ne porte pas de section
`## Acceptance criteria`.

- **AC1 — la déclaration est reconnue dans ses deux formes mesurées.** Sur un log portant
  `**HALTE borne : 379 lignes ajoutées pour \`310\`**` à 24 lignes de l'EOF, et sur un log
  portant `**HALTE borne : 455 lignes pour 450.**` à 26 lignes de l'EOF,
  `_pilot_declared_halt` rend 0. (V1, V2)
- **AC2 — la citation de la règle ne la déclenche pas, par chacun des deux ancrages
  isolément.** Un log dont la tête cite la règle avec le littéral `N` rend 1 (ancrage
  « chiffres », V3). Un log dont la tête cite la règle avec un **chiffre** rend 1
  (ancrage « fenêtre », V4). Une déclaration chiffrée hors fenêtre rend 1 (V5).
- **AC3 — la fenêtre vaut ce qu'elle déclare.** Une déclaration à la 120ᵉ ligne avant
  l'EOF rend 0 ; à la 121ᵉ, 1. (V6, V7)
- **AC4 — le prédicat ne porte pas la régression SIGPIPE de mika#2055.** Sur une fenêtre
  de queue de plus de 64 Ko dont la déclaration est la première ligne, 200 appels
  consécutifs rendent 0, sans un seul faux négatif ; et le corps de la fonction ne
  contient aucun `|` menant à un `grep -q`. (V8, S1)
- **AC5 — toute indéterminabilité rend « pas de halte ».** Chemin inexistant, fichier de
  0 octet, `_SESSION_LOG_PATH` vide, `_SESSION_LOG_PATH` non défini sous `set -u` : rend 1,
  sans erreur et sans mort du shell appelant. (V9, V10, V11)
- **AC6 — la suite est lancée par la CI, pas seulement présente.** Un step nommé du job
  `dispatch-lib-offline-suites` de `ci.yml` l'invoque, `check-suite-wiring` passe sans
  entrée dans `suite-wiring.tsv`, et S2 assure le câblage depuis la suite elle-même.
- **AC7 — aucun comportement de vol n'est modifié.** `grep -c "_pilot_declared_halt"` sur
  `dispatch-lib.sh` ne compte que la définition : ni `_push_branch`, ni le calcul de
  `RECOVERY_CLASS` (14846-14855), ni le bloc Unit 2 (14862+) ne le consultent dans ce vol.

## Fire-Disposition

Ce vol livre un **détecteur** : `_pilot_declared_halt` et sa suite, dont le chemin de
succès est « aucune halte déclarée ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist atterrit VIDE par
absence d'objet.**

Le prédicat porte sur un **log de session éphémère** (`$_PILOT_LOG_DIR/<LOG_ID>.log`),
jamais sur un artefact versionné : aucun fichier du dépôt, aucune ligne de code, aucune
population existante ne peut le violer. Il n'y a donc **rien à excepter** — pas « rien
d'excepté par choix », mais pas d'objet exceptable. Conformément à mika#1574, l'allowlist
vide est remplacée par la **paire témoin / contre-témoin** qui prouve que le détecteur
discrimine au lieu de toujours répondre pareil :

- **témoins** (le détecteur tire) : V1, V2, V6, V8 — les deux formes réellement mesurées,
  la borne intérieure de la fenêtre, et le volume qui déclencherait le SIGPIPE ;
- **contre-témoins** (le détecteur se tait) : V3, V4, V5, V7, V9, V10, V11 — chacun des
  deux ancrages isolément, la borne extérieure, et les quatre indéterminabilités.

Sans les témoins, un prédicat constamment faux passerait tous les contre-témoins ; sans
les contre-témoins, un prédicat constamment vrai passerait tous les témoins. La paire est
l'attestation.

**Ce n'est pas l'option (b).** Le détecteur n'atterrit **pas** désarmé : sa suite tourne
au step CI nommé ci-dessus et rougit sur une régression. Ce qui est différé, c'est sa
**consommation en production** — le refus de draft et l'éventuelle abstention de push —
qui dépend d'un arbitrage réservé à l'opérateur (§ *Décision réservée*) et part au vol 2b
sous `Tracked in: senara-solutions/mika#3142`. Un détecteur armé dont le consommateur
suit dans un vol nommé est le découpage que le corps du ticket prescrit, pas un
désarmement.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (`_pilot_declared_halt`, 2 constantes, en-tête d'ancrage) | 60 |
| tests (`skills/bundled/_shared/tests/test_pilot_halt_declared.sh`) | 335 |
| `.github/workflows/ci.yml` (step nommé du job `dispatch-lib-offline-suites`) | 7 |
| `Makefile` (target `test-pilot-halt-declared` + `.PHONY`) | 8 |

Total estimé : 410 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi le périmètre complet ne tenait pas.** Le périmètre
du ticket est re-mesuré à ~660 lignes, au-dessus de la borne de 429 : d'où le découpage
en vols que le corps du ticket prescrit. Le vol 2a passe à **410 lignes, soit 19 sous la
borne** — marge mince et assumée, bornée par le plancher du § *Ordre de commit* (V6 et V7
partent au vol 2b si la suite déborde). Le vol 2b est estimé à ~250 lignes (45 Unit 2
+ ~30 `_push_branch` selon l'arbitrage + 19 `dispatcher.rs` sur le précédent mika#3038
+ ~150 d'extension de suite), donc sous la borne lui aussi. Le seuil par PR de mika#2636
(1000 lignes) n'est atteint par aucun des deux vols : c'est la borne par vol qui mord.

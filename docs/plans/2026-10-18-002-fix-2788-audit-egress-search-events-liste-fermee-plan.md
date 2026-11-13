# fix(audit) — la liste d'événements `search_*` de l'audit E4 dérive de son producteur (mika#2788)

- **Ticket :** senara-solutions/mika#2788
- **Jalon :** Substrat de boucle (#33)
- **Labels :** `bug`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`
- **Liens :** mika#1806 (clôture du jalon bloquée), mika#1810 (E4, l'audit), mika#2407 (PR #2409, le producteur qui a divergé)

---

## Constat, vérifié dans l'arbre

`scripts/audit-egress-no-log.sh` ligne 422-425 rejette tout nom d'événement
`search_*` qui n'est pas `search_requested` ou `search_egress` :

```bash
unexpected=$(grep -oE '"event":"search[^"]*"' "$LOG_FILE" 2>/dev/null \
    | sort -u \
    | grep -Ev '"event":"search_(requested|egress)"' \
    || true)
```

Or le producteur — `crates/mika-gateway/src/` — en émet **cinq**, pas deux :

| nom | site | niveau | champs |
|---|---|---|---|
| `search_requested` | `egress_search/mod.rs:259` | INFO | flux (audités par le test Q4 `CapturingLayer`) |
| `search_egress` | `egress_search/mod.rs:292` | INFO | flux (idem) |
| `search_upstream_resolved` | `main.rs:235-244` | INFO | `upstream`, `upstream_source`, `api_key_present` (booléen), `required`, `required_source`, `endpoint_is_default` |
| `search_upstream_key_without_selector` | `main.rs:252-258` | WARN | aucun champ, message seul |
| `search_required_unrecognized_value` | `settings.rs:326-332` | WARN | `value` (la valeur brute de `MIKA_SEARCH_REQUIRED`, entre guillemets — variable d'opérateur, jamais du contenu de tenant) |

Le ticket ne nomme que `search_upstream_resolved` parce que c'est le seul des
trois qui soit émis **inconditionnellement à chaque démarrage** : les deux WARN
sont conditionnés à une demi-configuration et à une faute de frappe, donc
absents du journal de gentux. **La divergence est de trois noms, pas d'un.**
C'est exactement la forme du défaut : une liste fermée qui n'a aucun lien
mécanique avec son producteur ne diverge pas d'un cran, elle diverge de tout ce
qui a été ajouté depuis.

Pourquoi le lint de build (`scripts/verify-egress-no-log.sh`) ne l'a pas vu :
son `ALLOWED_EVENT_NAMES` (lignes 60-62) porte bien les deux mêmes noms, mais
son périmètre est `crates/mika-gateway/src/egress_search/` **seul** (`EGRESS_DIR`).
Les trois événements de configuration vivent dans `main.rs` et `settings.rs`,
hors de ce répertoire. Les deux listes sont donc correctes chacune dans son
périmètre, et c'est la liste de l'**audit** — dont le périmètre est le journal
entier du gateway — qui est sous-spécifiée. Élargir `ALLOWED_EVENT_NAMES` serait
le mauvais correctif : il relâcherait la discipline Q4 du module de flux, qui
doit rester à deux événements.

Défaut secondaire, lignes 415-416 :

```bash
total_requested=$(grep -c '"search_requested"' "$LOG_FILE" 2>/dev/null || echo 0)
```

`grep -c` écrit `0` sur stdout **puis** sort en code 1 ; le `|| echo 0` ajoute un
second `0`. La variable vaut `0\n0` et la ligne affichée est
`search_requested events: 0 0`. Le `||` ne peut pas être simplement supprimé :
sous `set -euo pipefail` (ligne 80) l'échec de la substitution tuerait le script.

---

## Décisions de conception

### D1 — Deux classes déclarées, une seule union, et le contrôle des champs s'applique aux deux

Le ticket demande de distinguer **flux** et **configuration**. La distinction est
réelle et mérite d'être écrite : les événements de flux sont émis par requête et
leur forme est tenue par le test Q4 `CapturingLayer` ; les événements de
configuration sont émis au démarrage et n'ont aucun test de forme.

Mais la distinction ne doit **pas** devenir une exemption. Un événement de
configuration qui porterait `url` ou `api_key` est une fuite au même titre. Donc :

- deux tableaux déclarés, grep-visibles, chacun commenté avec son site
  d'émission et son ticket d'origine ;
- une **union** `AUDIT_KNOWN_EVENTS` dérivée des deux ;
- le contrôle « nom inattendu » se fait contre l'**union** ;
- le contrôle « champ interdit » se fait aussi contre l'**union** (aujourd'hui il
  ne vise que les deux événements de flux : un `url` sur
  `search_upstream_resolved` passerait inaperçu — c'est le trou que le cas de
  test 4 du ticket vient fermer).

La classe sert à deux choses et à rien de plus : l'affichage (l'opérateur voit
le compte de chaque classe séparément) et le diagnostic (un nom inattendu dit
dans quelle classe le déclarer).

### D2 — Une seule expression régulière, construite une fois, consommée par les deux branches

L'audit a deux implémentations du contrôle des champs : `jq` quand il est là,
`grep -F` sinon (lignes 436-470). Si chacune construit sa propre sélection de
lignes, elles dérivent — c'est la leçon V13 de
`scripts/test-verify-egress-no-log.sh:532-545`, où « les deux parseurs appellent
`sanitize()` » était vrai globalement et faux par fonction.

Donc : une fonction `_join_alternation` et une variable
`KNOWN_EVENT_ALTERNATION`, construites une fois juste après les tableaux, et
**aucun littéral `search_requested`/`search_egress` ne subsiste dans les
prédicats de la couche 1b**. L'assertion A8 du harnais pose cette propriété
structurellement.

### D3 — Le compte nul : `|| n=0` sur l'affectation, pas `|| echo 0` dans la substitution

```bash
count_event_lines() {
    local name="$1" file="$2" n
    n=$(grep -cF -- "\"$name\"" "$file" 2>/dev/null) || n=0
    printf '%s' "${n:-0}"
}
```

Quand `grep -c` sort en 1, la substitution a bien capturé `0`, l'affectation
hérite du code 1, et `|| n=0` **écrase** la valeur au lieu d'y concaténer.
Un `grep` qui sort en 2 (fichier illisible) n'écrit rien : le `${n:-0}` couvre ce
cas, de sorte que la fonction ne renvoie jamais la chaîne vide. Deux défauts
fermés d'un coup, et le site est unique pour les cinq événements.

### D4 — La garde structurelle : `--known-events` comme interface machine, comparaison dans les deux sens

Le ticket laisse le choix « en CI ou dans `test-verify-egress-no-log.sh` ». Le
choix retenu : un **nouveau harnais**, `scripts/test-audit-egress-no-log.sh`,
câblé comme troisième étape du job CI `egress-no-log-lint` déjà existant.

Pourquoi pas dans `test-verify-egress-no-log.sh` : ce fichier est le harnais de
`verify-egress-no-log.sh`, 683 lignes consacrées aux formes du parseur awk des
attributs `#[cfg(test)]`. Les cas de mika#2788 portent sur un autre script, avec
d'autres fixtures (des journaux JSON, pas des arbres Rust). Un harnais par sujet.

Pourquoi pas un nouveau job CI : un job de plus est une surface de câblage de
plus, et `docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md`
est précisément le document qui cite `audit-egress-no-log.sh` comme garde non
câblée. Le job `egress-no-log-lint` existe (`.github/workflows/ci.yml:591-599`),
il porte déjà deux étapes sur ce substrat, et la cible Makefile
`verify-egress-no-log` lui est jumelle. On ajoute une étape aux deux, et le
harnais **assertionne son propre câblage** — précédent N16 de
`scripts/test-verify-egress-manifest.sh:768-830`, qui lit `.github/workflows/ci.yml`.

**L'interface entre la garde et l'audit est un mode d'invocation, pas un
`grep` de syntaxe bash.** L'audit gagne `--known-events` : il imprime
`<nom><TAB><classe>` trié, un par ligne, et sort 0 sans exécuter aucune couche.
Sourcer le script est exclu (il est `set -euo pipefail` et s'exécute à la
lecture) ; re-parser ses tableaux au `sed` serait un second parseur qui dérive
du premier. Un mode d'invocation est un seul lecteur.

La comparaison est faite **dans les deux sens**, sur le modèle explicite de
`scripts/check-dispatch-seats-declared.sh` (cité dans `CLAUDE.md` § Labels) :

- un nom émis par `crates/mika-gateway/src/` et absent de l'union → rouge
  (« un nouvel événement non déclaré fait échouer la CI au moment où il est
  ajouté », le travail attendu n° 2) ;
- un nom déclaré dans l'union que plus aucun site n'émet → rouge (entrée
  périmée). C'est l'assertion auto-nettoyante : sans elle, la liste
  recommencerait à mentir, dans l'autre sens, et une liste fermée qui porte des
  noms morts est exactement ce qui apprend à ne plus la lire.

Le périmètre du producteur est **tout** `crates/mika-gateway/src/**/*.rs`, y
compris les modules `#[cfg(test)]`. C'est volontairement fail-closed : un
`event = "search_…"` écrit dans un test est soit un nom réel à déclarer, soit un
nom à renommer. Aucun site de ce genre n'existe aujourd'hui (les occurrences de
`mod.rs:701-702` sont des `contains(&"search_requested".to_string())`, qui ne
correspondent pas au motif `event = "…"`), donc la garde atterrit verte.

### D5 — Atteindre rc=0 dans le harnais

Le cas de test 4 du ticket demande rc=0 sur un journal propre. L'audit sort 2
quand la couche 2 n'est pas attestée (ligne 523-527), et la couche 3 sort 0 en
`SKIP` si la base n'existe pas (ligne 534). Le harnais pilote donc trois
variables, toutes déjà prévues par l'en-tête du script :

- `MIKA_GATEWAY_LOG_FILE` → la fixture de journal ;
- `MIKA_GATEWAY_DB` → un chemin inexistant sous le répertoire de fixtures
  (couche 3 en `SKIP`) ;
- `MIKA_EGRESS_L2_ATTESTATION_FILE` → une attestation écrite juste avant par
  `audit-egress-no-log.sh --attest --target mika2788-harness --by harness`,
  c'est-à-dire par le producteur lui-même, et non par une fixture JSON écrite à
  la main qui dériverait du format.

`MIKA_EGRESS_L2_TARGET` et `MIKA_AUDIT_SUPPRESS_L2_WARN` sont explicitement
dés-exportés dans le harnais : un poste d'opérateur qui les porte ne doit pas
changer le verdict d'un test.

**À vérifier à l'implémentation** : que le chemin `--attest` crée le répertoire
parent de `ATTESTATION_FILE`. S'il ne le fait pas, le harnais le crée — pas le
script d'audit, dont le comportement ne doit pas changer dans ce ticket.

Les fixtures de journal sont créées sous `mktemp -d "${TMPDIR:-/tmp}/mika2788-audit-XXXXXX"`,
même forme que `scripts/test-verify-egress-no-log.sh:29` et
`scripts/test-verify-egress-manifest.sh` — deux harnais que `make test` et la CI
lancent déjà. Le `mktemp` est interne au script lancé en sous-processus, pas une
écriture d'outil du pilote : la règle mika#2548 sur `.pilot-scratch/` ne s'y
applique pas (elle porte sur les brouillons que le pilote écrit lui-même).

### D6 — Ce que ce ticket ne touche pas

- `ALLOWED_EVENT_NAMES` de `scripts/verify-egress-no-log.sh` reste à deux noms.
  Son périmètre est le module de flux et la discipline Q4 y est juste.
- Le test Q4 `egress_search::tests::log_assertion_no_tenant_no_query_no_forbidden_fields`
  et son commentaire « Exactly two events from OUR module » (`mod.rs:686`) restent
  vrais et inchangés : son périmètre est le module, pas le journal.
- Aucun changement de code Rust. Le producteur est correct ; c'est le lecteur qui
  était sous-spécifié.
- Aucun changement du format de l'attestation, des codes de sortie, ni des
  couches 1a / 2 / 3.

---

## Travail, dans l'ordre des commits

### Commit 1 — `scripts/audit-egress-no-log.sh` : les deux listes déclarées, l'union, le mode `--known-events`

1. Bloc de déclaration après les `readonly` d'en-tête (vers la ligne 96), avec
   son commentaire doctrinal : *la liste est fermée et dérive de son producteur ;
   l'ajouter ici est la moitié du geste, l'autre moitié est le site d'émission,
   et `scripts/test-audit-egress-no-log.sh` refuse que les deux divergent.*

   ```bash
   # Événements de FLUX — un par requête, forme tenue par le test Q4
   # CapturingLayer (crates/mika-gateway/src/egress_search/mod.rs).
   readonly AUDIT_FLOW_EVENTS=(
       search_requested   # egress_search/mod.rs — mika#1810 E4
       search_egress      # egress_search/mod.rs — mika#1810 E4
   )
   # Événements de CONFIGURATION — un par démarrage, surface opérateur
   # (crates/mika-gateway/CLAUDE.md § Search Substrate). Acceptés comme noms,
   # soumis au MÊME contrôle des champs interdits que les événements de flux.
   readonly AUDIT_CONFIG_EVENTS=(
       search_upstream_resolved              # main.rs     — mika#2407
       search_upstream_key_without_selector  # main.rs     — mika#2407
       search_required_unrecognized_value    # settings.rs — mika#2407
   )
   ```

2. Union + alternation, un seul site de construction :
   `AUDIT_KNOWN_EVENTS=("${AUDIT_FLOW_EVENTS[@]}" "${AUDIT_CONFIG_EVENTS[@]}")`,
   puis `KNOWN_EVENT_ALTERNATION="$(_join_alternation "${AUDIT_KNOWN_EVENTS[@]}")"`.

3. `--known-events` : une entrée dans `usage()` et un bras dans le `case` de
   parsing (vers la ligne 141, à côté de `-h|--help`), qui imprime
   `<nom><TAB>flow|config` trié et sort `$EXIT_OK` avant toute couche.
   Le tri rend la sortie déterministe, donc comparable par `diff` dans le harnais.

4. `count_event_lines()` (D3), et les cinq comptes affichés par classe :

   ```
     flow events
       search_requested events: 0
       search_egress events:    0
     config events
       search_upstream_resolved events: 78
       …
   ```

### Commit 2 — `scripts/audit-egress-no-log.sh` : la couche 1b contre l'union

5. Contrôle « nom inattendu » : `grep -Ev "\"event\":\"($KNOWN_EVENT_ALTERNATION)\""`.
   Le diagnostic de fuite gagne deux lignes de remède : *si cet événement est
   légitime, déclarez-le dans `AUDIT_FLOW_EVENTS` ou `AUDIT_CONFIG_EVENTS` de ce
   script — la CI (`egress-no-log-lint`) le refusait déjà côté producteur, donc
   le voir ici signifie un binaire déployé plus récent que cet audit.*
   Cette phrase est le vrai correctif de fond : elle convertit un rc=1 qu'on
   apprend à ignorer en une instruction.

6. Contrôle « champ interdit » : la sélection de lignes des **deux** branches
   (`jq` et `grep -F`) passe de `'"search_requested"\|"search_egress"'` à
   `-E "\"($KNOWN_EVENT_ALTERNATION)\""`. Liste des champs interdits inchangée.

### Commit 3 — `scripts/test-audit-egress-no-log.sh` (nouveau) + câblage

7. Le harnais, sur la forme de son voisin (`run_audit` renvoyant `"<rc>|<sortie>"`,
   `assert_exit` / `assert_mentions` en here-string — **jamais** `printf | grep -q`,
   piège SIGPIPE documenté à `test-verify-egress-no-log.sh:64-68`).

   | cas | ce qu'il pose |
   |---|---|
   | A1 | union déclarée == noms émis par l'arbre réel, dans les deux sens → rc=0 |
   | A2 | anti-vacuité : l'arbre réel émet ≥ 5 noms (une garde qui ne voit rien est verte pour rien) |
   | A3 | négatif sens 1 : fixture Rust portant `event = "search_zorglub"` → rouge, le diagnostic nomme `search_zorglub` et les deux tableaux |
   | A4 | négatif sens 2 : fixture Rust privée de `egress_search/` → rouge, le diagnostic nomme l'entrée déclarée devenue morte |
   | A5 | journal avec `search_upstream_resolved` sans champ interdit + attestation fraîche → **rc=0**, la sortie dit « OK: only declared event names » (travail attendu n° 4, moitié verte) |
   | A6 | même journal, un `"url":"https://…"` sur la ligne `search_upstream_resolved` → **rc=1**, le diagnostic nomme le champ (travail attendu n° 4, moitié rouge) |
   | A7 | journal sans aucun événement de flux → la ligne de compte est exactement `search_requested events: 0` (travail attendu n° 3) |
   | A8 | structurel D2 : un seul site construit `KNOWN_EVENT_ALTERNATION`, les deux branches du contrôle de champs le consomment, et aucun littéral `search_requested`/`search_egress` ne subsiste dans un prédicat de la couche 1b |
   | A9 | câblage : `.github/workflows/ci.yml` **et** le `Makefile` lancent ce harnais (précédent N16) |

   A6 doit tourner dans les deux régimes : avec `jq` présent (le chemin par
   défaut en CI) et avec `jq` masqué du `PATH`, pour que la branche de repli soit
   réellement visitée. Sans quoi la branche `grep` reste non testée, ce qui est la
   condition exacte d'une dérive entre les deux prédicats.

8. `.github/workflows/ci.yml` — troisième étape du job `egress-no-log-lint` :
   `- name: Pin the egress audit's declared search_* event list (mika#2788)` /
   `run: bash scripts/test-audit-egress-no-log.sh`.

9. `Makefile` — la même ligne ajoutée à la cible `verify-egress-no-log` (l. 295-297)
   **et** au bloc `test` (l. 169-170), qui lance aujourd'hui les deux mêmes scripts.

### Commit 4 — documentation

10. `crates/mika-gateway/docs/egress-search-no-log-audit.md` § *What we catch*
    (l. 65-73) : la liste de l'audit runtime passe de deux à cinq noms, en deux
    classes, avec la phrase qui dit que les champs des deux classes sont audités
    identiquement et que le lint de build reste à deux noms *pour son périmètre*.
    Mention du mode `--known-events` et de la garde qui le consomme.
11. `crates/mika-gateway/CLAUDE.md` § *Search Substrate* → *Operator surfaces* :
    une phrase nommant que les trois événements de configuration sont déclarés
    dans l'audit E4 et que la CI refuse un quatrième non déclaré.
12. `docs/solutions/` — **non**. Le ticket est un correctif de liste ; la leçon
    « une liste fermée dérive de son producteur » est déjà écrite et citée
    (`CLAUDE.md` § Labels, `check-dispatch-seats-declared.sh`). Rien de nouveau à
    y verser ; en ajouter serait de la redite.

---

## Contrat de vérification

| commande | attendu |
|---|---|
| `bash scripts/test-audit-egress-no-log.sh` | toutes les assertions passent, exit 0 |
| `bash scripts/audit-egress-no-log.sh --known-events` | 5 lignes `<nom>\t<classe>`, triées, exit 0 |
| `bash scripts/verify-egress-no-log.sh` | vert, inchangé (périmètre non touché) |
| `bash scripts/test-verify-egress-no-log.sh` | vert, inchangé |
| `make verify-egress-no-log` | vert, lance désormais trois scripts |
| `bash scripts/audit-egress-no-log.sh -h` | l'aide mentionne `--known-events`, exit 0 |
| `cargo test -p mika-gateway` | vert (aucun code Rust modifié ; contrôle de non-régression) |
| `bash scripts/verify-pipeline.sh origin/main` | vert |
| `cargo clippy` / `cargo fmt --check` | vert |

**Contrôle terrain, celui qui ferme le ticket.** Sur gentux, avec le journal
réel :

```bash
MIKA_GATEWAY_LOG_FILE=/var/log/mika/gateway.log bash scripts/audit-egress-no-log.sh ; echo "exit=$?"
```

Attendu : plus aucune ligne `LEAK (Layer 1): unexpected search_* event names`,
un compte non nul sur `search_upstream_resolved` (78 démarrages au 2026-10-18),
et `exit=2` — **pas 0** : la couche 2 reste due sur cette installation, et c'est
l'état nominal. Un `exit=0` sur gentux serait le signe qu'une attestation traîne
quelque part, pas le signe que le correctif marche. Une fuite résiduelle sur une
**autre** couche continuerait à donner `exit=1`, et c'est voulu.

**Halte.** Si le contrôle terrain fait apparaître un sixième nom `search_*` que
la CI n'avait pas refusé, le défaut n'est pas dans cette liste : c'est que le
binaire déployé est plus récent que l'arbre sur lequel la garde a tourné
(classe mika#2340). Établir le déploiement avant de toucher à la liste.

---

## Fire-Disposition

Ce plan livre des détecteurs : la garde structurelle de déclaration (A1/A3/A4),
l'assertion de câblage (A9) et l'assertion structurelle du prédicat unique (A8).

**Option retenue : (a) exception nommée en allowlist — et la table d'exceptions
part vide et reste vide.**

Il n'y a aucune violation préexistante à exempter, parce que le même vol
*déclare* les trois noms manquants. La garde atterrit donc **armée et verte sur
l'arbre réel** : c'est la déclaration qui est le correctif, pas une dérogation.
Précédents de ce choix dans ce dépôt, où une table d'exceptions vide est la
disposition explicite et non un oubli : `SEND_MESSAGE_LITERAL_ALLOWED`
(mika#2025) et `ACTOR_READING_PREDICATES_ALLOWED` (mika#2323) — dans les deux
cas, la résolution d'un rouge est d'ajouter la donnée au bon endroit, jamais une
entrée dans la table.

Détail d'implémentation de la disposition :

- **Aucune table d'exceptions n'est créée.** En créer une vide donnerait au
  prochain lecteur un endroit où déposer un nom plutôt que de le déclarer, ce qui
  est le défaut de ce ticket remis un cran plus bas. Si un jour un nom
  `search_*` doit être réellement exempté (par exemple un événement émis par un
  module qui n'est pas le substrat), ce sera un ticket, avec son motif.
- **L'assertion auto-nettoyante est la comparaison dans le sens 2** : une entrée
  déclarée que plus aucun site n'émet rougit (A4). C'est ce qui empêche la liste
  de redevenir fausse par accumulation de noms morts.
- **Anti-vacuité** (A2) : la garde assertionne que l'arbre réel lui livre au
  moins cinq noms. Un `grep` qui ne trouve rien — répertoire renommé, motif
  d'émission changé — serait sinon vert en ayant tout ignoré ; c'est la leçon
  mika#2205, et la raison pour laquelle A3 et A4 existent côte à côte.
- **Les négatifs sont vus rouges avant le correctif** : A3 et A4 doivent être
  exécutés contre des fixtures mutées et constatés en échec, pas supposés tels.
  Un verrou validé par construction n'est pas validé
  (`docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`
  § « A lock test is only verified by deleting what it locks »). En pratique :
  A3 et A4 assertionnent le **diagnostic propre à leur branche**, jamais un
  message générique que toute défaillance de la garde produirait.

---

## Definition of Done

- [ ] `scripts/audit-egress-no-log.sh` déclare les cinq noms en deux tableaux
      commentés, dérive une union et une alternation uniques.
- [ ] La couche 1b accepte les cinq noms et soumet les cinq au même contrôle des
      champs interdits, dans les deux branches (`jq` et repli `grep`).
- [ ] Les comptes nuls s'impriment une seule fois.
- [ ] `--known-events` imprime la liste triée et sort 0 sans exécuter de couche.
- [ ] `scripts/test-audit-egress-no-log.sh` existe, porte A1-A9, et passe.
- [ ] A3 et A4 ont été constatés rouges contre leur fixture mutée.
- [ ] Le harnais est câblé dans `.github/workflows/ci.yml` (job `egress-no-log-lint`)
      et dans le `Makefile` (`verify-egress-no-log` + `test`), et A9 le pose.
- [ ] Doc runbook et `crates/mika-gateway/CLAUDE.md` à jour.
- [ ] `verify-egress-no-log.sh`, son harnais et `cargo test -p mika-gateway`
      restent verts.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Le corps de PR nomme le contrôle terrain de gentux et son `exit=2` attendu.

---

## Acceptance criteria

Le corps de mika#2788 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa section « Travail attendu » (points 1-4)
et du contrat de vérification de ce plan.

- **AC1** — Le filtre de la couche 1b distingue, dans deux tableaux déclarés et
  grep-visibles, les événements de flux (`search_requested`, `search_egress`) des
  événements de configuration (`search_upstream_resolved`,
  `search_upstream_key_without_selector`, `search_required_unrecognized_value`).
- **AC2** — Les événements de configuration sont **acceptés** comme noms et
  **soumis au même contrôle des champs interdits** (`query`, `tenant_id`,
  `tenant_hash`, `user_id`, `chat_id`, `customer_id`, `api_key`, `retry_after`,
  `url`) que les événements de flux, dans la branche `jq` **et** dans la branche
  de repli `grep`.
- **AC3** — Une garde structurelle échoue en CI dès qu'un nom
  `event = "search_…"` émis par `crates/mika-gateway/src/` n'est pas déclaré dans
  l'audit, **et** dès qu'un nom déclaré n'est plus émis par aucun site. La
  comparaison se fait dans les deux sens.
- **AC4** — La garde est lancée par un job CI existant et par une cible Makefile,
  et une assertion du harnais le vérifie en lisant `.github/workflows/ci.yml` et
  le `Makefile`.
- **AC5** — Les comptes nuls s'impriment une seule fois : sur un journal sans
  événement de flux, la ligne affichée est exactement
  `search_requested events: 0`.
- **AC6** — Un journal contenant `search_upstream_resolved` sans champ interdit
  donne rc=0 (couche 2 attestée dans le harnais, couche 3 en `SKIP`).
- **AC7** — Le même journal avec un champ `url` sur la ligne
  `search_upstream_resolved` donne rc=1, et le diagnostic nomme le champ.
- **AC8** — `scripts/verify-egress-no-log.sh` et son `ALLOWED_EVENT_NAMES`
  restent à deux noms, et `scripts/test-verify-egress-no-log.sh` reste vert : le
  périmètre du lint de build n'est pas élargi.
- **AC9** — `scripts/audit-egress-no-log.sh --known-events` imprime les cinq noms
  avec leur classe, triés, et sort 0 sans exécuter aucune couche.
- **AC10** — Les codes de sortie, le format de l'attestation de couche 2 et les
  couches 1a / 2 / 3 sont inchangés ; aucun code Rust n'est modifié.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/audit-egress-no-log.sh` (déclarations + union + `--known-events` + `count_event_lines` + couche 1b) | 80 |
| `scripts/test-audit-egress-no-log.sh` (nouveau harnais, A1-A9) | 170 |
| `.github/workflows/ci.yml` (3e étape du job `egress-no-log-lint`) | 3 |
| `Makefile` (cibles `verify-egress-no-log` et `test`) | 2 |

Total estimé : 255 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 255 < 279, et 255 < 1000 : les deux bornes
passent, c'est la borne par vol qui mord et elle n'est pas franchie. La marge est
de 24 lignes, donc mince — le poste qui pourrait la faire sauter est le harnais,
à 170 lignes pour neuf cas. Deux choses la tiennent : le harnais est calqué sur
`scripts/test-verify-egress-no-log.sh` (les trois `assert_*` et le `run_*` sont
copiés, pas conçus), et les fixtures sont des journaux JSON de deux à quatre
lignes, pas des copies d'arbres Rust — seules A3 et A4 copient des `.rs`, et par
`cp` de l'arbre réel suivi d'une mutation d'une ligne, sur le modèle de
`fixture_dir()` (`test-verify-egress-no-log.sh:79-85`).

Si à l'implémentation le harnais dépasse, **le poste à réduire est le nombre de
régimes de A6**, pas le nombre de cas : faire tourner le contrôle de champs avec
et sans `jq` est la seule assertion qui se duplique, et la rendre paramétrique
(une boucle sur deux `PATH`) coûte moins que deux blocs. Les neuf cas, eux, sont
chacun un AC : en retirer un, c'est retirer un critère.

Hors décompte, conformément à la règle : `crates/mika-gateway/docs/egress-search-no-log-audit.md`
(≈ 15 lignes) et `crates/mika-gateway/CLAUDE.md` (≈ 3 lignes).

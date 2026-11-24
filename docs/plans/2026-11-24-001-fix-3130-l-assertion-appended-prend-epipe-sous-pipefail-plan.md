# fix(3130) — l'assertion `APPENDED` de mika#2211 prend EPIPE sous `pipefail`, et le lint mika#2055 ne voit pas sa forme

**Ticket :** senara-solutions/mika#3130
**Source primaire :** commentaire 6804496985 du parking #2983 (2026-11-11)
**Type :** fix (test + lint structurel)

---

## Le défaut, tel qu'il se mesure

`skills/bundled/_shared/test-dispatch-lib.sh:7541-7544` écrit son assertion
ainsi — la suite tourne sous `set -euo pipefail` (l. 14) :

```bash
assert_eq "mika#2211: the rule is APPENDED (repo#N stays the first line)" "yes" \
    "$(if printf '%s' "$T2211_SUW_SRC" \
        | grep -qF 'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")'; \
       then printf 'yes'; else printf 'no'; fi)"
```

`grep -q` sort au PREMIER match et ferme le tube. `printf`, qui écrit encore,
prend `EPIPE`/SIGPIPE et sort 141. Sous `pipefail`, le 141 est promu au statut du
pipeline, donc le `if` lit **faux sur une chaîne PRÉSENTE** : l'assertion rend
`no` et rougit sur du code correct.

La donnée est de 45 207 octets (`sed -n '/^_set_up_worktree() {/,/^}/p'` sur
`dispatch-lib.sh`, mesuré 2026-11-24), et le motif cherché est à la ligne 622 des
687 — soit ~93 % du flux. La course est donc serrée mais réelle : elle dépend de
l'entrelacement des `write()` de `printf` et des lectures de `grep`, pas d'un
dépassement du tampon de pipe.

### Ce qui est attesté, et ce qui ne l'est pas

Attesté :

- **run 43079844489, tentative 1** : job `Check`, étape 13
  « Dispatch-lib assertion suite (mika#1772) » → `failure`.
- **Même run, tentative 2**, diff inchangé → `success`. (`run_attempt: 2`,
  `conclusion: success` sur l'API.) PR #3075, branche
  `feat/1727/tui-v2b3b-minuteur-seulement-sans-couture`, mergée le 2026-11-12.
- L'opérateur a nommé l'assertion dans le parking le soir même du run
  (18:51 UTC, run lancé à 18:39) : « l'assertion mika#2211 « APPENDED » […]
  a rougi Check sur #3075 […] sans lien avec le diff ».

Non attesté, et le plan n'en dépend pas :

- Les logs de la tentative 1 ne sont plus servis par l'API (écrasés par la
  tentative 2), donc la ligne `✗` exacte n'est pas récupérable aujourd'hui.
- **La reproduction locale est NÉGATIVE** : 200 tirages de la forme fautive sur
  la vraie donnée de 45 Ko, aujourd'hui, sur cette machine → `no=0/200`
  (sonde `.pilot-scratch/repro-3130/probe.sh`). C'est une course
  d'ordonnancement ; un vert local ne l'infirme pas, et **l'implémentation ne
  doit pas tenter de la rendre déterministe** sur cette taille de donnée.

La preuve que ce vol livre est donc **structurelle** : la forme fragile disparaît,
et le détecteur qui aurait dû l'interdire la voit désormais.

### Pourquoi ce n'est pas qu'un site à réécrire

Le dépôt a déjà la doctrine, écrite trois fois en prose
(`test-dispatch-lib.sh:11610-11615`, `:11767-11769`, `:13495-13503`) et armée en
CI par un lint dédié : `scripts/verify-no-sigpipe-grep.sh` (mika#2055, job
`sigpipe-grep-lint`, cible `make verify-no-sigpipe-grep`). Le site du ticket vit
depuis des mois **dans le champ de ce lint** et le lint le laisse passer. Son ERE
est ligne-à-ligne et exige le producteur immédiatement adjacent au consommateur :

```
FORBIDDEN='(^|[^[:alnum:]_.])(printf|echo)[^|#]*\|[[:space:]]*grep[[:space:]]+(-[A-Za-z]+[[:space:]]+)*-[A-Za-z]*q[A-Za-z]*([[:space:]]|$)'
```

Deux classes lui échappent, et **toutes deux sont peuplées dans le dépôt** :

- **T1 — continuation de ligne.** Le producteur est sur une ligne, le
  `| grep -q` sur la suivante après un `\`. Le lint, qui lit ligne par ligne, ne
  voit ni l'une ni l'autre. C'est **exactement** la forme du site du ticket.
- **T2 — maillon intermédiaire.** `[^|#]*` ne peut traverser ni un `|`
  intermédiaire (`printf … | grep -v … | grep -qE …`), ni un `#` littéral dans
  une chaîne du pipeline (`'^[[:space:]]*#'`), ni même un `|` enfermé dans une
  expansion (`"${result#*|}"`).

Corriger le seul site du ticket laisserait le lint aveugle à sa propre classe et
le second site de la même suite (l. 11461) rouge en puissance. Ce vol ferme les
deux classes.

### Population mesurée (2026-11-24, racines `scripts/` + `skills/bundled/`)

T1, par scan des continuations (`grep -B1` sur `^\s*| grep -…q`), vocabulaire de
producteurs actuel :

| site | forme |
|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh:7542-7543` | `printf` → `grep -qF`, 45 Ko — **le site du ticket** |
| `skills/bundled/_shared/dispatch-lib.sh:14737-14738` | `printf '%s' "$(git log -1 --format='%s')"` → `grep -qF` |

T2, par scan avec l'ERE élargi (`[^;&]*` au lieu de `[^|#]*`) :

| site | forme |
|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh:11461` | `printf` → `grep -v` → `grep -qE` (`#` littéral dans le motif intermédiaire) |
| `scripts/test-verify-no-secret-in-setenv.sh:49` | `printf '%s' "${result#*\|}"` → `grep -q` (`\|` dans l'expansion) |
| `skills/bundled/_shared/tests/test_finalize_pr_gate.sh:227` | `printf` → `tail -1` → `grep -q` |

**Zéro faux positif** sur l'arbre réel avec l'ERE élargi : les cinq lignes
ci-dessus sont les seules remontées, et les cinq sont de vrais membres de la
classe. Les proses de doctrine qui citent la forme (`test-check-canonical-tokens.sh:49`,
`test-check-landing-tokens.sh:46`, `test-check-cta-primitives.sh:47`,
`test-check-a2a-timeout-literals.sh:43`, `test-check-eval-ignored-flags.sh:69`,
et les commentaires de `dispatch-lib.sh`/`test-dispatch-lib.sh`) sont toutes des
commentaires **pleine ligne**, déjà filtrées par le `case` du lint.

Hors population, et qui doit le rester :

- `skills/bundled/address-pr-comments/handlers/run.sh:152` — `#!/bin/sh`, exempté
  par `should_enforce()` (pas de `pipefail` en POSIX, `<<<` undefined : SC3011).
  C'est l'exemption nommée de la table mika#2055.
- `dispatch-lib.sh:9852-9853` et `:9866-9867` — producteur `head -n N`, hors du
  vocabulaire `printf|echo`. Voir § Périmètre.

---

## Requirements

**R1.** Le site du ticket (`test-dispatch-lib.sh:7541-7544`) ne porte plus de
pipeline : la mesure passe par un here-string. L'assertion garde son label, sa
sémantique et sa prose (le contrat mika#138 qu'elle fige — « `repo#N` reste la
première ligne » — est inchangé).

**R2.** Les quatre autres membres mesurés de la classe sont réécrits de même. La
population résiduelle est **vide** à la fin du vol, sur les deux racines
scannées.

**R3.** `scripts/verify-no-sigpipe-grep.sh` voit la classe T1 : un producteur et
son `grep -q` séparés par une ou plusieurs continuations `\` sont signalés, avec
le numéro de la **première** ligne du groupe (celle où l'opérateur doit aller).

**R4.** Le lint voit la classe T2 : un `grep -q` terminal est signalé quel que
soit le nombre de maillons intermédiaires, y compris quand un `|` ou un `#`
littéral vit dans une chaîne ou une expansion du pipeline.

**R5.** L'échappatoire existante continue de fonctionner, à l'identique :
`# sigpipe-safe: #<digits>` supprime la violation, un marqueur **nu** ne la
supprime pas. Sur une violation T1, le marqueur posé sur **n'importe quelle**
ligne du groupe logique supprime.

**R6.** Le lint ne gagne aucun faux positif : un commentaire de **fin de ligne**
qui décrit la forme interdite n'est pas une violation. (Les commentaires pleine
ligne le sont déjà.)

**R7.** La suite anti-vacuité `scripts/test-verify-no-sigpipe-grep.sh` prouve que
le détecteur étendu PEUT rougir sur chacune des deux nouvelles classes, avec une
fixture qui rejoue la forme réelle du site du ticket.

**R8.** Rien d'autre ne change : pas de nouveau job CI, pas de nouvelle cible
`make`, pas d'entrée dans `scripts/suite-wiring.tsv`. Les deux surfaces sont déjà
câblées (`ci.yml:958-966` pour le lint et sa suite, `ci.yml:85` →
`make test-dispatch-lib` pour la suite d'assertions).

---

## Design

### D1 — Les cinq réécritures

Chacune est une substitution locale, sans changement de sémantique. Le `--` est
ajouté partout où il manque : un needle commençant par `-` serait sinon lu comme
une option.

**`test-dispatch-lib.sh:7541-7544`** — le needle sort en variable (il porte des
quotes imbriquées : le lire deux fois serait une occasion de le désaligner) :

```bash
# Here-string et jamais `printf … | grep -q` (mika#2055, mika#3130) : `grep -q`
# sort au premier match et ferme le tube, `printf` prend EPIPE, et sous le
# `pipefail` de l'en-tête le 141 est promu — le `if` lirait alors FAUX sur une
# chaîne PRÉSENTE. Mesuré : rouge à la tentative 1 du run 43079844489 (#3075),
# vert à la tentative 2 sur le même diff.
T2211_APPEND_NEEDLE='PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")'
assert_eq "mika#2211: the rule is APPENDED (repo#N stays the first line)" "yes" \
    "$(if grep -qF -- "$T2211_APPEND_NEEDLE" <<<"$T2211_SUW_SRC"; \
       then printf 'yes'; else printf 'no'; fi)"
```

La forme `assert_eq`/`yes` est conservée plutôt que basculée sur
`assert_contains` : à l'échec, `assert_contains` déverserait les 45 Ko du corps
de `_set_up_worktree` dans le log CI. (Le label et la valeur attendue restent
mot pour mot ceux d'aujourd'hui — le diff doit être lisible comme « même
assertion, autre plomberie ».)

**`test-dispatch-lib.sh:11461`** — le filtre intermédiaire est matérialisé ;
`|| true` parce que `grep -v` rend 1 sur un filtrage total :

```bash
M2194_WRITER_NOCOMMENT=$(printf '%s\n' "$M2194_WRITER_SRC" | grep -v '^[[:space:]]*#' || true)
… "$(if grep -qE -- "$M2194_NEEDLE" <<<"$M2194_WRITER_NOCOMMENT"; then printf 'yes'; else printf 'no'; fi)"
```

**`dispatch-lib.sh:14737-14738`** — le sujet de HEAD est capturé, puis lu :

```bash
_head_subject=$(git -C "$WORKTREE_DIR" log -1 --format='%s' 2>/dev/null || true)
if grep -qF -- 'wip(mika#1383): auto-PR-create rescue' <<<"$_head_subject"; then
```

La redirection `2>/dev/null` du site actuel est conservée telle quelle (ce n'est
pas un site `2>&9`).

**`scripts/test-verify-no-secret-in-setenv.sh:49`** :

```bash
if grep -q -- "$needle" <<<"${result#*|}"; then
```

**`tests/test_finalize_pr_gate.sh:227`** — `tail -1` reste, hors du tube du
`grep` :

```bash
_last_line=$(printf '%s\n' "$stripped" | tail -1)
if grep -q -- '^---$' <<<"$_last_line"; then
```

### D2 — Le lint : une normalisation en deux étages avant le match

Le lint garde sa structure (boucle sur les fichiers, `should_enforce`, messages,
codes de sortie). Ce qui change est **ce qui entre dans le `grep -nE`** : au lieu
des lignes physiques, des **lignes logiques**.

**Étage 1 — jointure des continuations.** Un helper (`_logical_lines "$f"`, awk)
émet une ligne par groupe logique, préfixée du numéro de la **première** ligne du
groupe, au format `N:<texte joint>` — le même format que `grep -n`, donc le reste
de la boucle (`lineno="${hit%%:*}"`, `text="${hit#*:}"`) est inchangé. Une ligne
se terminant par un `\` non échappé est jointe à la suivante, le `\` remplacé par
une espace.

L'invariant qui rend cet étage sûr : en shell, une continuation exige le `\` en
**dernier** caractère de la ligne, donc un commentaire ne peut jamais précéder
une continuation sur la même ligne physique. La jointure ne peut pas coller du
code derrière un `#`.

**Étage 2 — retrait du commentaire de fin de ligne.** Sur la ligne logique, un
parcours caractère par caractère qui suit l'état des guillemets (`'` et `"`)
tronque au premier `#` **hors guillemets** précédé d'un début de ligne ou d'une
espace. Cet étage est ce qui permet d'élargir l'ERE sans acheter de faux positif
(R6) tout en laissant visible le `#` enfermé dans une chaîne (le site l. 11461).

**Ordre impératif — l'exemption se lit AVANT l'étage 2.** Le marqueur
`# sigpipe-safe: #2055` EST un commentaire de fin de ligne : l'étage 2 le
détruirait. La chaîne est donc : étage 1 → filtre d'exemption
(`grep -Ev '# sigpipe-safe: #[0-9]+'`, inchangé, appliqué sur la ligne logique
brute) → étage 2 → match de l'ERE. Cet ordre donne aussi R5 gratuitement : le
marqueur posé sur n'importe quelle ligne physique du groupe se retrouve dans la
ligne logique et supprime.

**L'ERE élargi.** Une seule classe de caractères change :

```
-  (printf|echo)[^|#]*\|[[:space:]]*grep …
+  (printf|echo)[^;&]*\|[[:space:]]*grep …
```

`[^;&]*` autorise les `|` et les `#` intermédiaires (c'est R4) tout en gardant le
producteur et le `grep -q` dans le **même** pipeline : un `;`, un `&&`, un `||`
ou un `&` coupe le match, donc `echo "msg"; cmd | grep -q x` n'est pas signalé.
La fin du motif (la course de drapeaux dont le dernier porte un `q`) est
inchangée, ce qui préserve les deux négatifs déjà couverts par la suite :
`grep` sans `-q`, et un needle de forme `-q`.

**Direction de l'erreur, dite honnêtement.** L'étage 2 ne fait que RETIRER du
texte : il ne peut créer aucun faux positif, seulement un faux négatif. Le cas
résiduel est une chaîne **multi-ligne** (sans `\`) dont une ligne porte un `#` :
l'état des guillemets, calculé par ligne logique, y est faux. Un tel faux négatif
porterait sur du texte de donnée, pas sur un pipeline exécuté — et le lint
d'avant ce vol ne voyait de toute façon rien de cette classe. Aucune occurrence
dans l'arbre actuel.

**Le lint ne doit pas se piéger lui-même.** Les nouveaux maillons (`awk`) ne
court-circuitent pas, et le `|| true` de la substitution de processus reste. Le
fichier et sa suite restent auto-exclus par `EXCLUDE_BASENAMES` (ils embarquent
l'ERE en littéral).

### D3 — La suite anti-vacuité

`make_fixture` est étendu (ou doublé d'un `make_fixture_lines` prenant plusieurs
lignes) pour écrire des fixtures **multi-lignes**, ce que le helper actuel ne
permet pas. Les six cas existants restent tels quels — ils sont la preuve de
non-régression du comportement mono-ligne.

Nouveaux cas :

| # | fixture | attendu | ce qu'il prouve |
|---|---|---|---|
| 6 | le site du ticket, mot pour mot (`printf` + `\` + `\| grep -qF`) | exit 1 | T1 est vue, sur la forme réelle |
| 7 | même forme, remède here-string sur deux lignes | exit 0 | le remède reste accepté à travers une continuation |
| 8 | T1 + `# sigpipe-safe: #2055` sur la ligne du `grep` | exit 0 | R5 sur un groupe logique |
| 9 | T1 + marqueur **nu** sur la ligne du `grep` | exit 1 | la citation reste structurellement exigée |
| 10 | `printf … \| grep -v '^[[:space:]]*#' \| grep -qE …` | exit 1 | T2, avec `#` dans une chaîne intermédiaire |
| 11 | `printf '%s' "${result#*\|}" \| grep -q …` | exit 1 | T2, avec `\|` dans une expansion |
| 12 | `cmd_sans_tube "$x"  # prose citant printf \| grep -q` | exit 0 | R6 : pas de faux positif par commentaire de fin de ligne |
| 13 | `echo "a"; cmd \| grep -q "b"` | exit 0 | `;` coupe le pipeline : l'élargissement ne déborde pas |

Le cas 1 existant (« swept repo tree is clean ») devient la preuve de R2 : il ne
peut passer que si les cinq réécritures de D1 ont eu lieu. C'est ce qui dicte
l'ordre de commit.

### D4 — Ce qui n'est pas touché

- La table `PILOT_LABEL_TURN_CEILINGS`, les seats, la suite-wiring : rien.
- Le vocabulaire de producteurs reste `printf|echo`. Voir § Périmètre.
- Aucune assertion existante ne change de label, de valeur attendue ni de
  sémantique. Le diff de `test-dispatch-lib.sh` est strictement de la plomberie.

---

## Périmètre — ce vol et ce qu'il renvoie

**Dans ce vol :** les deux classes T1 et T2, pour le vocabulaire de producteurs
que le lint porte déjà (`printf`, `echo`), plus les cinq réécritures qui vident la
population.

**Hors de ce vol, et pourquoi.** Élargir le vocabulaire aux autres producteurs
qui peuvent prendre SIGPIPE — `sed`, `git show`, `head`, `cat`, une fonction
shell — est une classe distincte et bien plus peuplée : une mesure rapide donne
une vingtaine de sites (`test-dispatch-lib.sh:3365, 3378, 3381, 3451, 3605, 3675,
3698, 8414, 8418`, `dispatch-lib.sh:7559, 9853, 9867`,
`scripts/verify-pipeline.sh:298`, `scripts/canary-pilot-containment:750`,
`scripts/check-shared-checkout-guard-wiring.sh:57`,
`scripts/test-python-installer-guard.sh:288`, …). Les trier demande de juger, un
par un, si le producteur est borné sous le tampon de pipe — ce qui est du travail
de mesure, pas de plomberie, et ferait sortir ce vol de sa borne. La prose de
doctrine de `test-dispatch-lib.sh:11767-11769` nomme déjà `git show` comme membre
de la classe, donc le besoin est réel ; il n'est simplement pas celui que ce
ticket a mesuré.

**Tracked in:** senara-solutions/mika#2983 (le parking d'où #3130 est issu) —
une entrée y sera posée pour « élargir le vocabulaire de producteurs du lint
mika#2055 au-delà de `printf|echo` », avec le recensement ci-dessus comme point
de départ.

---

## Verification Contract

Toutes les commandes depuis la racine du worktree.

1. `bash scripts/verify-no-sigpipe-grep.sh` → exit 0, `no-sigpipe-grep: clean
   (scanned N file(s))`. **C'est la preuve de R2** : avec le lint étendu, un seul
   des cinq sites non réécrit rendrait exit 1.
2. `bash scripts/test-verify-no-sigpipe-grep.sh` → `0 failed`, les six cas
   historiques plus les huit nouveaux (R3, R4, R5, R6, R7).
3. `make test-dispatch-lib` → `0 failed`. L'assertion
   `mika#2211: the rule is APPENDED (repo#N stays the first line)` est présente
   et verte, et l'assertion `mika#2194 (AC4 … )` de la l. 11461 l'est aussi.
4. `bash skills/bundled/_shared/tests/test_finalize_pr_gate.sh` → vert.
5. `bash scripts/test-verify-no-secret-in-setenv.sh` → vert.
6. `make verify-bundled-skills` → vert (`dispatch-lib.sh` est touché).
7. `bash scripts/check-suite-wiring.sh` → vert et `unwired=0` (rien n'est ajouté
   ni retiré du câblage).
8. `bash scripts/verify-pipeline.sh origin/main` → vert.
9. **Contrôle de non-vacuité à la main, une fois, avant d'ouvrir la PR :**
   remettre temporairement le pipeline de 7542-7543 (ou toute fixture de la forme
   T1) et vérifier que `bash scripts/verify-no-sigpipe-grep.sh` rend exit 1 en
   nommant `test-dispatch-lib.sh:7542` — le numéro de la PREMIÈRE ligne du
   groupe, pas celui du `grep`. Puis annuler. (Le cas 6 de la suite automatise la
   même preuve sur une fixture ; ce contrôle manuel vérifie en plus le numéro de
   ligne rapporté sur le fichier réel.)

Ce qui n'est **pas** dans le contrat, délibérément : aucun test ne tente de
reproduire la course EPIPE sur la donnée de 45 Ko. Elle n'est pas déterministe à
cette taille (0/200 mesuré localement), et un test qui la « reproduit » serait
soit flaky, soit vert par accident — exactement la catégorie de garde que
mika#2578 condamne.

---

## Definition of Done

- Les cinq sites de la population mesurée ne portent plus de pipeline en amont
  d'un `grep -q` ; chacun porte un `--` devant son needle.
- Le site du ticket garde son label, sa valeur attendue et sa prose ; le needle
  est en variable.
- `verify-no-sigpipe-grep.sh` normalise en lignes logiques avant de matcher, lit
  l'exemption avant de retirer les commentaires de fin de ligne, et porte l'ERE
  élargi.
- La suite anti-vacuité couvre les deux nouvelles classes, les deux formes
  d'exemption et les deux contrôles négatifs (commentaire de fin de ligne, `;`
  qui coupe).
- Les neuf points du Verification Contract passent.
- La prose du lint (en-tête) nomme les deux classes nouvellement couvertes et
  cite mika#3130 à côté de mika#2055.
- `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md`
  gagne une section « Ce que le lint ne voyait pas (mika#3130) » : les deux
  classes, la mesure du 2026-11-24, et le fait qu'un run re-tenté est la
  signature de cette panne.
- Corps de PR : `Closes #3130`, et `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification Contract.

**AC1.** `skills/bundled/_shared/test-dispatch-lib.sh` ne contient plus aucun
pipeline en amont d'un `grep -q` : ni à la l. 7542-7543, ni à la l. 11461.
L'assertion `mika#2211: the rule is APPENDED (repo#N stays the first line)`
existe toujours, attend toujours `"yes"`, et passe.

**AC2.** Les trois autres sites mesurés — `dispatch-lib.sh:14737-14738`,
`scripts/test-verify-no-secret-in-setenv.sh:49`,
`skills/bundled/_shared/tests/test_finalize_pr_gate.sh:227` — sont réécrits en
here-string, et leurs suites respectives passent.

**AC3.** `bash scripts/verify-no-sigpipe-grep.sh` rend exit 0 sur l'arbre, et
rend exit 1 — en nommant le numéro de la PREMIÈRE ligne du groupe logique — sur
une fixture qui rejoue la forme du site du ticket (producteur, `\`, `| grep -qF`).

**AC4.** Le lint rend exit 1 sur un pipeline à maillon intermédiaire
(`printf … | grep -v … | grep -qE …`) et sur un pipeline dont le `|` vit dans une
expansion (`"${result#*|}"`).

**AC5.** Le lint rend exit 0 sur : le remède here-string écrit sur deux lignes ;
une violation T1 portant `# sigpipe-safe: #2055` sur l'une quelconque de ses
lignes ; un commentaire de fin de ligne citant la forme interdite ; une ligne où
un `;` sépare un `echo` d'un `| grep -q`.

**AC6.** Le lint rend exit 1 sur une violation T1 portant un marqueur
`# sigpipe-safe` **nu**, sans citation `#<digits>`.

**AC7.** `bash scripts/test-verify-no-sigpipe-grep.sh` rend `0 failed`, les six
cas historiques inclus et inchangés.

**AC8.** Aucun job CI, cible `make` ou entrée `scripts/suite-wiring.tsv` n'est
ajouté ni retiré ; `bash scripts/check-suite-wiring.sh` passe avec `unwired=0`.

---

## Fire-Disposition

Ce vol livre un détecteur : l'extension de `scripts/verify-no-sigpipe-grep.sh`,
dont le chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une population
d'exceptions VIDE.**

Détail d'implémentation :

1. **L'allowlist n'est pas créée : elle existe déjà.** Le lint mika#2055 porte
   son mécanisme d'exception par ligne, `# sigpipe-safe: #<ticket>`, dont la
   citation est structurellement exigée (un marqueur nu ne supprime pas, cas 5 de
   la suite). Ce vol n'ajoute aucun second mécanisme.

2. **Ce vol n'y inscrit AUCUNE entrée.** Les cinq violations que l'extension
   rend visibles sont **corrigées dans le même vol** (D1, AC1–AC2), chacune
   nommée dans le tableau de population ci-dessus avec son fichier, sa ligne et
   sa forme. Le détecteur atterrit donc **armé** et sur une population vide — pas
   d'exception à faire expirer, parce qu'il n'y en a pas.

3. **L'assertion auto-nettoyante est le cas 1 de la suite anti-vacuité**
   (`assert_exit "$REPO_ROOT" 0 "swept repo tree is clean"`, déjà câblé en CI à
   `ci.yml:966`). Il lit l'arbre réel : il rougit le jour où une sixième
   violation entre, et il rougirait aujourd'hui si l'une des cinq réécritures
   manquait. Une exception future ne peut donc exister sans son commentaire cité,
   et un commentaire cité sur une ligne devenue saine est visible d'un
   `grep -rn 'sigpipe-safe'` sur deux racines.

4. **La seule exemption structurelle du lint reste nommée et inchangée** :
   `skills/bundled/address-pr-comments/handlers/run.sh` (`#!/bin/sh`, hors
   `pipefail`, `<<<` indisponible), portée par `should_enforce()` et par la prose
   de l'en-tête, avec sa raison. Ce vol n'y touche pas.

5. **Rien n'est livré désarmé** : aucun `#[ignore]`, aucun cas commenté, aucun
   `exit 0` conditionnel. Le détecteur mord dès le merge.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/verify-no-sigpipe-grep.sh` (normaliseur 2 étages, ERE élargi, prose d'en-tête) | 85 |
| `scripts/test-verify-no-sigpipe-grep.sh` (helper multi-lignes + 8 cas) | 70 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (2 sites + prose) | 30 |
| `skills/bundled/_shared/dispatch-lib.sh` (1 site + prose) | 12 |
| `skills/bundled/_shared/tests/test_finalize_pr_gate.sh` (1 site) | 8 |
| `scripts/test-verify-no-secret-in-setenv.sh` (1 site) | 6 |

Total estimé : 211 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 211 < 279, et la marge n'est pas que
numérique : cinq des six livrables sont des substitutions locales d'une à trois
lignes chacune, sans lecture de code nouveau — les cinq sites sont déjà
localisés, mesurés et cités avec leur avant/après dans D1. La seule part qui
demande de la conception est le normaliseur du lint (étage 1 + étage 2), bornée à
un fichier de 149 lignes dont la structure est conservée et dont le point
d'insertion est connu (la substitution de processus l. 132-136). La fin de
pipeline est légère : aucun `cargo build`, aucune migration de schéma, six suites
shell qui tournent en secondes.

---

## Ordre de commit

**L'ordre est contraint, pas esthétique :** le cas 1 de la suite anti-vacuité lit
l'arbre réel, donc poser le lint étendu avant d'avoir vidé la population rendrait
le commit intermédiaire rouge, et une bissection future illisible.

1. **`fix(3130): les cinq lecteurs sortent du tube`** — les réécritures de D1,
   dans les cinq fichiers. Plancher : `make test-dispatch-lib`,
   `test_finalize_pr_gate.sh`, `test-verify-no-secret-in-setenv.sh`,
   `make verify-bundled-skills` et `bash scripts/verify-no-sigpipe-grep.sh`
   (version non étendue) passent.
2. **`fix(3130): le lint SIGPIPE voit la continuation et le maillon`** — le
   normaliseur, l'ERE élargi, la prose d'en-tête, et les huit cas de la suite.
   Plancher : les neuf points du Verification Contract passent, contrôle manuel
   de non-vacuité inclus.
3. **`docs(3130): ce que le lint mika#2055 ne voyait pas`** — la section ajoutée
   au writeup de `docs/solutions/test-failures/`.

---
title: "Un détecteur de régression d'invocation pour la fin de pipeline allégée"
date: 2026-10-16
type: chore
issue: mika#2729
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
---

# Plan : détecter le retour de `ce-simplify-code` / `ce-code-review` dans un vol de pilote (mika#2729)

## Ce que le ticket a déjà tranché, et ce qu'il laisse au grooming

Tranché par le corps du ticket et par le relevé de PR #2728 :

- la consigne mika#2694 **tient** — 0 invocation sur 45 transcripts post-consigne,
  contrôle positif à 1 sur chacun des trois témoins pré-consigne ;
- le levier de secours du critère 3 de mika#2694 est **sans objet** ;
- la question « un défaut de duplication atteint-il `main` plus souvent ? » est
  **hors scope** ;
- la signature doit être **ancrée sur l'appel d'outil**, et sa forme doit être
  **établie par `grep -oh`** avant tout comptage ;
- le détecteur doit livrer **son propre contrôle positif**.

Laissé au grooming : **où** brancher le détecteur. Ce plan le décide (§ suivant),
puis prescrit la livraison.

---

## Décision : la garde vit dans `dispatch-lib`, en post-flight

**Option retenue : la troisième du ticket** — une garde côté `dispatch-lib`, à la
clôture du vol, avant le callback. Quatre raisons, chacune vérifiée dans l'arbre :

1. **C'est la seule qui observe le comportement.** Un `scripts/check-*.sh` n'a
   aucun transcript à lire en CI : il porterait sur la *consigne* dans
   `CLAUDE.md`. Un `CLAUDE.md` intact et un plugin qui regonfle la fin de
   pipeline est précisément la conjonction que le ticket craint — ce lint-là
   serait vert ce jour-là.

2. **Le transcript est garanti présent et complet à cet instant.** Le fichier
   JSONL n'est ingéré puis supprimé par le moteur qu'une fois sa tâche sortie de
   `pending`/`in_progress` — ce qui suit le trap EXIT de `dispatch-lib`
   (contrat écrit à `crates/mika-agent/src/task_engine/engine.rs:3899-3903`,
   « Race safety (mika#1705 Risk 5) »). Et `claude-pilot` a rendu : `PILOT_EXIT`
   est déjà capturé, donc le fichier n'est plus en cours d'écriture.

3. **Le chemin est déclaré par le producteur, pas reconstruit.** L'exécuteur
   injecte `ANTHROPIC_LOG_FILE` dans l'env du handler
   (`crates/mika-agent/src/skills/executor.rs:288-317`), et cette variable est
   dans `_PILOT_SANDBOX_ENV_ALLOWLIST` (`dispatch-lib.sh:1511-1513`) : elle est
   lisible depuis `dispatch-lib`, qui tourne hors du bac à sable. Le détecteur la
   lit **telle quelle**, sans dériver `$HOME/.mika/data/pilot-transcripts/${LOG_ID}.jsonl`.
   Un chemin re-dérivé est la façon canonique de fabriquer un faux négatif muet,
   et c'est la duplication que mika-platform#58 a fermée pour le worktree.

4. **Les pièges 3 et 4 de l'instrument de #2728 disparaissent par construction.**
   Le détecteur ne lit **qu'un fichier**, celui de son propre vol, nommé par son
   producteur : il n'y a pas de population à filtrer, donc ni session
   d'observation à exclure (piège 3), ni seuil `-newermt` en heure locale
   confronté à un instant UTC (piège 4).

**Pourquoi pas les deux autres.** Un test Rust sur un transcript-fixture ne mesure
aucun vol réel : il mesure la fixture. Une garde côté `qa-review` n'a pas le
transcript sous la main — elle tourne sur un webhook PR, dans un autre process, à
un moment où le fichier est déjà ingéré et supprimé.

**Conséquence sur la forme de la livraison.** Un détecteur de comportement en
shell n'est pas exécutable en CI (il lui manque son transcript). Il est donc livré
en **deux moitiés indissociables** : la garde (M1) et la suite qui l'atteste (M2).
M2 est la moitié qui tourne en CI, et c'est elle qui porte le contrôle positif
exigé par le critère 3 du ticket.

---

## M1 — la garde `_detect_lean_tail_regression` dans `dispatch-lib.sh`

### La signature

Forme établie par `grep -oh` sur les trois témoins, relevée au § III de
`docs/solutions/dev-loop/un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md` :

```
"name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"
```

Le motif livré en dérive, avec **un seul degré de liberté ajouté** :

```sh
# Un seul site de vérité pour la signature. L'espacement est TOLÉRÉ, l'ordre et
# l'adjacence des trois clés sont EXIGÉS.
_LEAN_TAIL_SIG='"name"[[:space:]]*:[[:space:]]*"Skill"[[:space:]]*,[[:space:]]*"input"[[:space:]]*:[[:space:]]*\{[[:space:]]*"skill"[[:space:]]*:[[:space:]]*"compound-engineering:'
```

Les deux propriétés du ticket sont traitées différemment, et c'est délibéré :

- **l'ancrage est exigé** (propriété 1). Les trois clés doivent être adjacentes
  dans cet ordre. La sous-chaîne nue `compound-engineering:ce-simplify-code`
  apparaît dans le corps de mika#2694, dans le document de #2728, dans
  `CLAUDE.md`, et dans *ce plan* ; elle est hors de question.
- **l'espacement est neutralisé** (propriété 2). La forme sans espaces a rendu
  0 fichier sur 44 au grooming de mika#2694 — un faux négatif complet qui se
  lisait comme le résultat espéré. Épingler la forme espacée reconduirait le
  piège à l'identique le jour où le sérialiseur en amont change : `jq -c` et
  `json.dumps(separators=(",", ":"))` produisent tous deux la forme compacte.
  Le motif accepte les deux, et un contrôle négatif de M2 l'épingle.

Le piège 6 de #2728 (accents échappés en `\uXXXX`) est sans objet : la signature
est intégralement ASCII.

### Le comptage

Par **occurrences**, jamais par lignes : une ligne de transcript porte toute une
réponse LLM, donc deux invocations dans le même tour tiennent sur une ligne et
`grep -c` en compterait une.

```sh
grep -Eoh "${_LEAN_TAIL_SIG}ce-simplify-code\"" "$_transcript" | wc -l
```

### Trois états, jamais un silence

`absent` n'est pas `0` (mika#2331, doctrine déjà citée dans
`_emit_pilot_budget_line`). La garde émet exactement une ligne, toujours, parmi
trois :

| condition | token émis |
|---|---|
| `ANTHROPIC_LOG_FILE` non déclarée, vide, ou fichier illisible | `lean_tail_unmeasured` |
| transcript lu, 0 + 0 occurrence | `lean_tail_clean` |
| transcript lu, au moins une occurrence | `lean_tail_regression` |

Forme des lignes (ASCII, grep-ables, modelées sur `pilot_budget_armed`) :

```
dispatch-lib: lean_tail_clean transcript=<path> simplify=0 code_review=0 (mika#2729)
dispatch-lib: lean_tail_regression transcript=<path> simplify=2 code_review=1 — un vol de pilote a réinvoqué la fin de pipeline retirée par mika#2694 (mika#2729)
dispatch-lib: lean_tail_unmeasured transcript=absent — ANTHROPIC_LOG_FILE non déclarée ou illisible ; l'absence d'invocation n'est PAS mesurée (mika#2729)
```

Sans `lean_tail_unmeasured`, « le détecteur veille et ne trouve rien » et « le
détecteur ne regarde rien » seraient indistinguables — la classe mika#2205,
appliquée à l'organe même que ce ticket livre.

### Le sink

**Le sink n'est pas négociable, et il n'est pas `>&2`.** Le commentaire de
`_emit_pilot_budget_line` (`dispatch-lib.sh:515-527`) l'a déjà payé : la garde
tourne **après** le lancement, donc hors de la redirection `2>"$STDERR_FILE"` du
site de lancement, et l'exécuteur ne lit le stderr propre de `dispatch-lib` que
sur `!status.success()`. Un dispatch implement **réussit** : la ligne serait jetée
sans être lue.

Donc :

- **les trois lignes** → `tee -a "$PERSISTENT_STDERR" >&2`, le fichier durable
  (`$_PILOT_LOG_DIR/${LOG_ID}.stderr`), exactement comme `pilot_log_guard.missing`
  (`dispatch-lib.sh:4604-4609`). C'est le canal de l'instrument.
- **`lean_tail_regression` seule** → en plus, un paragraphe ajouté à `RESULT`.
  C'est le canal d'alerte : `RESULT` remonte dans le callback, donc chez mika-dev
  et chez l'opérateur.

`lean_tail_clean` **ne va pas** dans `RESULT` : une ligne nominale sur chaque
dispatch est du bruit dans le canal d'alerte, et le stderr durable suffit à
répondre à « l'instrument a-t-il regardé ce vol ? ».

### Le site d'appel

Dans `_run_claude_pilot`, **juste après** le bloc de classification
(`if [ "$_terminated_empty" = "1" ] … else _post_flight_recovery … fi`,
`dispatch-lib.sh:4717-4736`) et **avant** l'ajout du `Logs (last 10KB)` et le
`head -c 92000`. Trois propriétés de ce point :

- il est sur **toutes** les branches de sortie (JSON structuré, exit 0 non
  structuré, exit non nul) — le motif mika#1615 ;
- `RESULT` y est composé, donc le paragraphe s'ajoute à un texte existant ;
- la ligne est **avant** les 10 Ko de logs, donc elle survit à la troncature à
  92 000 octets, qui coupe la fin.

**Aucun prédicat sur `SKILL`.** La mesure est la même pour `dev-pilot` et
`dev-groom`, une invocation est anormale dans les deux cas, et l'exécuteur
n'injecte déjà `ANTHROPIC_LOG_FILE` que pour ces deux skills — le filtre existe
en amont, le redoubler ici serait un second site à garder en phase.

**Aucun nouveau commutateur d'environnement.** La garde ne refuse rien : elle
mesure et nomme. `MIKA_LOG_PILOT_TRANSCRIPTS` gouverne déjà la disponibilité du
transcript, et son état « off » a son token (`lean_tail_unmeasured`). Un flag de
plus serait une entrée à documenter dans
`docs/operator/environment-variables.md` pour désarmer quelque chose qui ne mord
pas.

---

## M2 — `tests/test_lean_tail_regression.sh`, et son contrôle positif

Suite shell sur le modèle exact de
`skills/bundled/_shared/tests/test_rescue_cause_token.sh` : elle `source`
la **vraie** `dispatch-lib.sh`, appelle la **vraie** fonction, écrit ses fixtures
dans un `mktemp -d`, et n'a besoin d'aucun réseau ni de `cargo`.

**Les fixtures sont écrites dans un tmpdir, jamais commitées.** Un transcript réel
pèse ~1,6 Mo et porte le prompt intégral d'un vol : il n'a pas sa place dans
l'arbre. Chaque fixture est une ligne JSONL de forme
`{"schema_version":"v1",…,"response_body":…}` portant la signature, et l'en-tête
de la suite **cite la provenance** de la ligne porteuse : témoin, date, et la
commande `grep -oh` du § III de #2728 qui l'a rendue. C'est une transcription
attestée, pas une forme inventée.

### Contrôle positif (le critère 3 du ticket)

| cas | fixture | attendu |
|---|---|---|
| P1 | signature `ce-simplify-code`, espacement du témoin `500a76a5-…` | `lean_tail_regression`, `simplify=1` |
| P2 | signature `ce-code-review`, même espacement (témoin `db73ec9f-…`) | `lean_tail_regression`, `code_review=1` |
| P3 | les deux sur la même ligne (la forme de `db73ec9f-…`) | `simplify=1 code_review=1`, et **une seule** ligne lue → le comptage est par occurrence |
| P4 | deux occurrences de `ce-simplify-code` dans la même ligne | `simplify=2` |

### Contrôles négatifs — chacun est l'un des faux résultats mesurés

| cas | fixture | attendu | ce qu'il refuse |
|---|---|---|---|
| N1 | une ligne de **prose** citant `compound-engineering:ce-simplify-code` (la forme du corps de mika#2694 et de ce plan) | `lean_tail_clean` | le détecteur qui crie sur tout document parlant du ticket (propriété 1) |
| N2 | signature **sans espaces** (`"name":"Skill","input":{"skill":"…"`) | `lean_tail_regression` | le faux négatif à 0/44 du grooming de mika#2694 (propriété 2) |
| N3 | transcript sans aucune occurrence | `lean_tail_clean` | — (nominal) |
| N4 | `ANTHROPIC_LOG_FILE` vide | `lean_tail_unmeasured`, et **pas** `clean` | « absent » lu comme « zéro » (mika#2331) |
| N5 | `ANTHROPIC_LOG_FILE` pointant un chemin inexistant | `lean_tail_unmeasured` | idem, sur le chemin plutôt que sur la variable |
| N6 | `"name": "Skill"` pour un autre skill, et `ce-simplify-code` en prose 300 caractères plus loin | `lean_tail_clean` | l'ancrage relâché en fenêtre, qui rouvrirait N1 |

### Probes de mutation — l'instrument montré porteur

Sur le modèle `arm_mutant` / `mutant_subject` de `test_rescue_cause_token.sh`
(dont le commentaire explique pourquoi armement et lecture sont deux fonctions :
une mutation qui n'apparie rien doit rougir, pas passer en silence).

- **X1** — signature réduite à la sous-chaîne nue ⇒ **N1 doit devenir rouge**
  (la prose est comptée comme une invocation).
- **X2** — `[[:space:]]*` retirés du motif ⇒ **N2 doit devenir rouge** (la forme
  compacte redevient invisible : le piège 5 reconstitué).
- **X3** — le token `lean_tail_unmeasured` remplacé par `lean_tail_clean` ⇒
  **N4 doit devenir rouge** (l'absence se remet à se lire comme un zéro).

### Sink et RESULT, épinglés

- `lean_tail_regression` apparaît dans `RESULT` **et** dans `PERSISTENT_STDERR` ;
- `lean_tail_clean` et `lean_tail_unmeasured` apparaissent dans
  `PERSISTENT_STDERR` et **pas** dans `RESULT` ;
- assertion statique : exactement **un** site déclare `_LEAN_TAIL_SIG` dans
  `dispatch-lib.sh`, et le nom du skill n'est retapé nulle part ailleurs dans la
  fonction — c'est ce qui empêche les deux moitiés du motif de dériver.

### Ce que ce contrôle positif ne prouve pas

Il faut l'écrire dans l'en-tête de la suite plutôt que de le laisser deviner :
une fixture synthétique démontre que l'instrument **trouve la forme qu'on lui a
décrite**, pas que cette forme est celle que `claude-pilot` émet. Cette seconde
propriété a été établie **une fois, par mesure**, sur trois témoins réels le
2026-10-15 (#2728 § III). La tolérance d'espacement couvre la seule dérive de
sérialiseur mesurée ; une dérive d'une autre nature (clé renommée, bloc
`tool_use` restructuré) rendrait le détecteur muet, et seule une nouvelle mesure
sur transcripts vivants le découvrirait. Une sonde contre les transcripts de
l'hôte a été envisagée et **écartée** : sur un hôte sain le compte est 0, donc
elle ne peut pas servir de contrôle positif — elle coûterait des lignes sans rien
attester.

---

## Câblage CI

- target `Makefile` : `test-lean-tail-regression`, ajouté à `.PHONY`, sur le
  modèle de `test-rescue-cause-token` ;
- step dans `.github/workflows/ci.yml`, à la suite des autres suites
  `dispatch-lib` (après « Plan-discovery suite (mika#2038) »).

Non ajouté à `test-dispatch-lib` : chaque suite récente a son target nommé, et
c'est ce qui rend lisible laquelle a rougi.

---

## Ordre de commit

1. `dispatch-lib.sh` — `_LEAN_TAIL_SIG`, `_detect_lean_tail_regression`, le site
   d'appel.
2. `tests/test_lean_tail_regression.sh` — contrôle positif, contrôles négatifs,
   probes de mutation.
3. `Makefile` + `.github/workflows/ci.yml`.

Plancher mergeable : les trois commits. Le commit 1 seul livrerait une garde que
rien n'atteste, ce que le critère 3 du ticket refuse nommément.

---

## Verification contract

- `bash skills/bundled/_shared/tests/test_lean_tail_regression.sh` → toutes les
  assertions passent, `exit 0`.
- `make test-lean-tail-regression` → même résultat via le target.
- `make test-dispatch-lib` vert — non-régression des suites voisines qui
  `source` le même fichier.
- `bash -n skills/bundled/_shared/dispatch-lib.sh` et `shellcheck` sur les deux
  fichiers touchés (au niveau de sévérité déjà en vigueur dans l'arbre).
- `scripts/canonical-tokens-survey.sh --check` puis
  `cargo test -p mika-agent --lib canonical_tokens` : les trois nouveaux tokens
  n'ont **aucun lecteur strict** (ils sont lus par un humain et par `grep`), donc
  le scan d'exhaustivité doit rester vert sans entrée dans
  `scripts/canonical-tokens.tsv`. S'il réclame une déclaration, l'ajouter dans
  le **même commit** que le token.
- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
  verts (aucun code Rust n'est touché : ils constatent l'absence de régression).
- `make verify-bundled-skills` vert.
- `scripts/verify-pipeline.sh origin/main` vert.

---

## Definition of Done

- `_detect_lean_tail_regression` existe dans `dispatch-lib.sh`, est appelée sur
  toutes les branches de sortie de `_run_claude_pilot`, et lit
  `$ANTHROPIC_LOG_FILE` sans dériver aucun chemin de rechange.
- La signature a **un seul** site de déclaration, ancré sur l'appel d'outil
  `Skill` et tolérant à l'espacement.
- Les trois états sont distingués par trois tokens, et `absent` ne peut pas se
  lire comme `0`.
- `lean_tail_regression` atteint `RESULT` **et** le stderr durable ; les deux
  autres tokens n'atteignent que le stderr durable.
- La suite de test porte son contrôle positif (4 cas), ses contrôles négatifs
  (6 cas) et ses probes de mutation (3), et son en-tête cite la provenance de la
  ligne porteuse et la borne de l'attestation.
- Le target `Makefile` et le step CI existent ; la suite tourne sur chaque PR.
- Aucun nouveau `MIKA_*` n'est introduit, donc
  `docs/operator/environment-variables.md` est inchangé.
- `CLAUDE.md` est inchangé (la garde est auto-documentée par ses tokens et par la
  suite ; rien n'est demandé au pilote).

---

## Acceptance criteria

Le corps de mika#2729 ne porte pas de section `## Acceptance criteria`. Dérivés
de l'« Attendu », de ses trois propriétés contraignantes et du Verification
contract ci-dessus :

- **AC1** — Un vol de pilote qui invoque `ce-simplify-code` ou `ce-code-review`
  produit la ligne `dispatch-lib: lean_tail_regression …` dans `RESULT` et dans
  `$_PILOT_LOG_DIR/${LOG_ID}.stderr`, avec le compte d'occurrences de chacun des
  deux skills.
- **AC2** — La signature est ancrée sur l'appel d'outil : une ligne qui
  **mentionne** `compound-engineering:ce-simplify-code` en prose ne déclenche
  rien (contrôle négatif N1), et le détecteur reste silencieux sur le corps de
  mika#2694, sur le document de #2728 et sur ce plan.
- **AC3** — La forme sans espaces est reconnue (contrôle négatif N2) : le faux
  négatif à 0/44 mesuré au grooming de mika#2694 ne peut plus se produire.
- **AC4** — Le détecteur livre son propre contrôle positif : au moins un cas où
  la même fonction, appelée de la même façon, **trouve** l'occurrence — quatre
  cas (P1–P4), dont un à deux occurrences sur une seule ligne.
- **AC5** — Chaque moitié de l'instrument est montrée porteuse par une probe de
  mutation : désancrage ⇒ N1 rouge, espacement rigide ⇒ N2 rouge, confusion
  absent/zéro ⇒ N4 rouge.
- **AC6** — Un transcript absent ou illisible produit `lean_tail_unmeasured` et
  jamais `lean_tail_clean`.
- **AC7** — La suite tourne en CI via un target `Makefile` nommé, et
  `make test-dispatch-lib` reste vert.

---

## Fire-Disposition

**Option (a) — exception nommée en allowlist, avec une allowlist vide par
mesure.**

Détail d'implémentation, et pourquoi l'allowlist est vide plutôt qu'absente par
négligence :

- **La population existante a été comptée.** #2728 a mesuré 0 invocation sur les
  45 transcripts post-consigne, avec contrôle positif à 1 sur chacun des trois
  témoins pré-consigne. Il n'existe aucune violation à excepter : la garde
  atterrit **armée** et verte.
- **La seule population qui demanderait des exceptions est hors de portée du
  détecteur.** Le dépôt contient plusieurs documents qui *parlent* de
  `ce-simplify-code` — le corps de mika#2694, le document de #2728, `CLAUDE.md`,
  ce plan. Un détecteur de dépôt (l'option 2 du ticket) aurait eu besoin d'une
  entrée pour chacun. Celui-ci ne lit que des transcripts, et la signature est
  ancrée : ces fichiers ne l'atteignent pas. C'est l'ancrage, et non une liste,
  qui tient l'allowlist vide.
- **L'assertion auto-nettoyante est le contrôle négatif N1, doublé de la probe
  X1.** Le jour où la signature se désancre — un motif élargi, une « réparation »
  d'un faux négatif par relâchement — N1 rougit sur la ligne de prose, et X1
  atteste que c'est bien N1 qui porte cette garantie. L'exception implicite
  (« les documents qui en parlent ne comptent pas ») cesse d'être tenable au même
  instant où elle devient fausse, et le CI le dit.
- **Les trois témoins pré-consigne restent nommés** dans l'en-tête de la suite
  (`500a76a5-…`, `59ffa68e-…`, `db73ec9f-…`, avec leur ticket et leur compte),
  comme provenance de la forme attestée — grep-ables, référencés à mika#2729 et à
  #2728.

Options écartées : **(b) livrer désarmé** n'a pas de justification — il n'y a
aucune violation existante à absorber, et une garde `#[ignore]` sur une
population propre est une garde qui ne sera jamais armée ; **(c)
halte-et-remontée** n'a pas d'objet — le ticket a lui-même cadré la signature et
la question du point de branchement, qui est tranchée ci-dessus.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (signature, fonction, site d'appel, doctrine) | 70 |
| `skills/bundled/_shared/tests/test_lean_tail_regression.sh` | 140 |
| `Makefile` (target + `.PHONY`) | 4 |
| `.github/workflows/ci.yml` (step) | 3 |

Total estimé : 217 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et il tient pour trois raisons nommées.**

1. **Aucun code Rust n'est touché.** Les deux fichiers portants sont du shell ;
   la vérification Rust ne fait que constater une non-régression, et ne coûte
   aucun tour d'écriture.
2. **Le harnais de test existe déjà et se recopie.**
   `tests/test_rescue_cause_token.sh` fournit les helpers d'assertion, le
   `mktemp -d` avec son `trap`, et le couple `arm_mutant`/`mutant_subject` des
   probes. Les 140 lignes sont de la composition de cas, pas de la conception de
   harnais.
3. **Les fixtures sont de une ligne.** Aucun transcript n'est copié, aucun
   fichier de fixture n'est commité, aucune lecture de fichier de 1,6 Mo n'a lieu
   pendant le vol — c'est le poste qui avait rendu mika#2694 coûteux en tours, et
   il est absent ici.

Le périmètre est entier : rien n'est renvoyé à un suivi, donc aucune ligne
`Tracked in:` n'est due.

---

## Liens

- mika#2694 — la consigne et son essai borné.
- PR #2728 et
  `docs/solutions/dev-loop/un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md`
  — le relevé, les sept propriétés mesurées de l'instrument, et la forme de la
  signature établie par `grep -oh`.
- mika#2693 — la borne de dimensionnement par vol ; son propre vol est la
  troisième mort au plafond dans la fin de pipeline.
- mika#1705 / mika#2040 — le producteur du transcript et le contrat de course de
  son ingestion.
- mika#2205 — « la suite passe » et « la suite ne regarde rien » doivent être
  distinguables.
- mika#2331 — un `null` n'est jamais un `0`.
- claude-pilot#289 — « commite, pousse, vérifie, ouvre la PR ».

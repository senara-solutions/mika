# fix(3134) — `production_resumes_after_cut` ne saute pas les chaînes ordinaires multi-lignes, et N15 rougit sur une fixture de test

**Ticket :** senara-solutions/mika#3134
**Source primaire :** PR #3133 (tête `4cfd3e3b`), palliatif `176bab64`
**Refs :** #3117 (ticket parent), mika#2408 (le lint egress), mika#2205 (classe anti-vacuité)
**Type :** fix (moteur de lint + suite anti-vacuité)

---

## Le défaut, tel qu'il se mesure

`production_resumes_after_cut` (`scripts/lib/egress_manifest_lint.py:362-417`) cherche
un item Rust **en colonne zéro** après le premier `#[cfg(test)] mod X {`. C'est la
remesure continue du trou assumé de `production_slices`, consommée par le cas N15 de
`scripts/test-verify-egress-manifest.sh:717-746`.

Sa boucle de balayage saute **une seule** classe de région : la chaîne brute
(`r"`, `r#"`, …), via `RAW_STRING_OPEN_RE` et l'état `raw_hashes`
(`egress_manifest_lint.py:400-414`). Elle ne connaît ni la chaîne **ordinaire**
restée ouverte en fin de ligne (continuation `"\` ou saut de ligne littéral), ni le
commentaire de bloc, ni le littéral de caractère `'"'`.

Une fixture Rust embarquée dans une chaîne ordinaire multi-ligne est donc lue comme
du code : ses `async fn` de fixture tombent en colonne zéro, et le prédicat conclut
« la production reprend ».

### Reproduction, intégrale, aujourd'hui (2026-11-24)

Sur `crates/mika-agent/src/auto_pull.rs` **à la révision `4cfd3e3b`** — avant le
palliatif, extrait en `.pilot-scratch/auto_pull_before.rs` :

```
reprise:        9812
  9811  '        const PHASE0: &str = "\'
  9812  'async fn phase0_feed_ready_pool(db: &Db) {'
```

et `audit_cut_holes` rapporte derrière cette fausse reprise exactement les **six**
hosts que le ticket nomme :

| ligne | host rapporté |
|---|---|
| 12402 | `github.com` |
| 12736 | `github.com` |
| 12972 | `github.com` |
| 13684 | `github.com` |
| 14207 | `github.com` |
| 14307 | `github.com` |

Six hosts de prose de test lus comme sinks de production hors inventaire → N15 rouge
dans le job « Egress Manifest Lint », **sans qu'aucun sink réel ne soit en cause**.
Le ticket est reproduit mot pour mot.

### L'étendue, qui est plus large que le seul fichier du ticket

Le palliatif `176bab64` a réparé `auto_pull.rs`. Il n'a rien dit du reste de l'arbre.
Mesure sur l'arbre **actuel** (417 fichiers `.rs` scannés sous `crates/`, exclusions
du lint appliquées) : **dix** fichiers voient une reprise.

| fichier | ligne | ce que la ligne porte |
|---|---|---|
| `crates/mika-agent/src/canonical_tokens.rs` | 5353 | `const … = \"auto_feeder_pool_in_flight\";` — **dans une chaîne** |
| `crates/mika-agent/src/evidence/guards.rs` | 10256 | `fn ailleurs(body: &str) -> bool {` — fixture |
| `crates/mika-agent/src/messaging.rs` | 1114 | `async fn flush(…)` — fixture |
| `crates/mika-agent/src/qa_reviewed_head.rs` | 880 | `async fn decide_something(…)` + `\n` littéraux — fixture |
| `crates/mika-agent/src/ready_label.rs` | 1750 | `async fn handler_doutil(…)` — fixture |
| `crates/mika-agent/src/server/dashboard.rs` | 1679 | `pub struct OperationalItemsQuery {` — **la vraie reprise** |
| `crates/mika-agent/src/skills/assembly.rs` | 281 | `fn rebuild_registry(…)` — fixture |
| `crates/mika-agent/src/source_scan.rs` | 379 | `fn writes_through_a_guard(…)` — fixture |
| `crates/mika-agent/src/teams/engine.rs` | 3825 | `fn commit_deliverable(…)` — fixture |
| `crates/mika-common/src/source_guard.rs` | 1110 | `fn helper() -> u32 {` — fixture |

Neuf des dix sont de **faux positifs** de la classe du ticket. Une seule est réelle :
`dashboard.rs:1679`, qui tombe dans la plage 1596-1732 que la prose du lint et le
commentaire de N15 nomment déjà comme la vraie reprise témoin.

Aucun des neuf ne porte de sink derrière, donc `--audit-cut-holes` rend aujourd'hui
`rc=0` : **l'arbre est vert par chance**, pas par construction. La prose du lint le
dit elle-même (`egress_manifest_lint.py:420-434`) : « cette sur-détection ne coûte
rien tant que la région ne porte aucun sink ». #3134 est la journée où elle a coûté.

### Pourquoi ce n'est pas « repasser les fixtures en `r#"…"#` »

Le palliatif est une convention de style qu'aucune garde n'enforce. Neuf sites de
l'arbre portent encore la forme, et la prochaine fixture Rust écrite en `"\`
derrière un `#[cfg(test)] mod` rougira N15 à nouveau — sur un diff sans rapport, dans
un job dont le message parlera de sinks `github.com`. C'est précisément la panne
qu'a vécue #3133 : un auteur a payé un détour de diagnostic pour un défaut du
prédicat.

Le défaut est **dans le prédicat**, et il est réparable sans toucher à ce que la
prose du lint interdit : **le comptage d'accolades**. Suivre l'état d'une chaîne n'est
pas compter un bloc ; c'est une machine à états par ligne, sans profondeur, sans pile.

---

## Requirements

**R1.** `production_resumes_after_cut` n'évalue ses prédicats (`CFG_TEST_RE`,
`INLINE_TEST_MOD_RE`, `COLUMN_ZERO_ITEM_RE`) que sur les lignes qui **commencent**
hors chaîne et hors commentaire de bloc. Les quatre classes de région sont suivies :
chaîne brute (comportement actuel, préservé), chaîne ordinaire multi-ligne,
commentaire de bloc `/* … */`, littéral de caractère.

**R2.** Sur `auto_pull.rs` à `4cfd3e3b` — la forme exacte du ticket — le prédicat rend
`None`. Sur l'arbre actuel il rend une reprise pour `dashboard.rs` et pour elle
seule : **1 fichier au lieu de 10**.

**R3.** Aucune accolade n'est comptée, aucune profondeur de bloc n'est modélisée.
C'est la contrainte que la prose de `production_slices` pose (le saut de bloc par
comptage a été écrit, essayé et refusé sur mesure) et ce vol ne la lève pas.

**R4.** La direction de l'erreur est **nommée et gardée**. Un scanner d'état qui
entrerait à tort dans une chaîne sauterait tout le reste du fichier — une
sous-détection silencieuse, qui est la direction dangereuse. Une assertion sur
l'arbre réel prouve que l'état revient à « code » à la fin de **chaque** fichier
scanné.

**R5.** L'anti-vacuité existante de N15 (le témoin `dashboard.rs`) reste verte et
inchangée : le détecteur voit encore une reprise.

**R6.** La suite prouve que le prédicat PEUT rougir sur chacune des formes
nouvellement sautées, et qu'il rougit toujours sur la forme dangereuse (production
redevenue frère du module de test).

**R7.** Un cas bout-en-bout prouve la **conséquence** : une fixture synthétique
portant une fixture Rust en `"\` suivie d'un host réel ne produit plus de faux sink
dans `--audit-cut-holes`.

**R8.** Rien d'autre ne change : pas de nouveau job CI, pas de nouvelle cible `make`,
pas d'entrée dans `scripts/suite-wiring.tsv`, pas de nouveau mode CLI du moteur.
Les deux surfaces sont déjà câblées (N15 appelle le moteur en direct, et la suite est
lancée par le job existant).

---

## Design

### D1 — `advance_lexical_state(line, state) -> state`

Une fonction pure, nouvelle, dans `scripts/lib/egress_manifest_lint.py`, à côté de
`COLUMN_ZERO_ITEM_RE`. Elle consomme une ligne caractère par caractère et rend l'état
lexical **à la fin** de la ligne. Quatre états :

| état | sens |
|---|---|
| `None` | code |
| `("str",)` | chaîne ordinaire ouverte |
| `("raw", hashes)` | chaîne brute ouverte, `hashes` = les `#` à refermer |
| `("block",)` | commentaire `/* … */` ouvert |

Transitions, en état `None` :

- `//` → **retour immédiat `None`** : le reste de la ligne est un commentaire. C'est
  ce qui empêche un `"` non apparié en prose (`// un " seul`) d'ouvrir une fausse
  chaîne — le défaut qui transformerait ce correctif en sous-détection.
- `/*` → `("block",)`.
- `r` ou `br` **en frontière de mot**, suivi de `#*"` → `("raw", hashes)`. La
  frontière est nécessaire : sans elle, un identifiant finissant par `r` collé à un
  guillemet (`four"…`) serait lu comme une chaîne brute. Aucune occurrence mesurée
  dans l'arbre, la garde est gratuite.
- `'…'` reconnu par `'(?:\\.|[^\\'])'` → littéral de caractère, consommé. Couvre
  `'"'`, `'\''`, `'\n'`. Un `'` qui ne matche pas est un **lifetime** (`&'a str`) et
  est simplement traversé.
- `"` → `("str",)`.

En état `("str",)` : `\` consomme deux caractères (donc `\"` ne ferme pas), `"` ferme.
Une chaîne encore ouverte en fin de ligne **reste ouverte** — c'est toute la classe du
ticket, et c'est légal en Rust avec ou sans le `\` de continuation.

En état `("raw", h)` : recherche de `"` + `h`, sinon l'état persiste.
En état `("block",)` : recherche de `*/`, sinon l'état persiste. Un `//` n'y est pas
un commentaire.

Les commentaires `/* */` **imbriqués** (légaux en Rust) ne sont pas modélisés : le
premier `*/` referme. C'est un sur-rapport possible, jamais une sous-détection, et
l'assertion de R4 le rendrait visible. Zéro occurrence mesurée.

### D2 — `production_resumes_after_cut` : un seul balayage, deux phases

La fonction garde sa signature, son nom, son contrat de retour (`int | None`) et son
unique appelant (`audit_cut_holes`). Ce qui change est la boucle : **un** balayage de
la ligne 1 à la fin, portant l'état lexical sans interruption, avec une variable de
phase (`avant la coupure` / `après la coupure`).

```python
state = None
pending = False
cut = None
for idx, line in enumerate(lines, start=1):
    at_start = state          # l'état AU DÉBUT de la ligne
    state = advance_lexical_state(line, state)
    if at_start is not None:
        continue              # la ligne commence dans une région sautée
    if cut is None:
        … (le détecteur de coupure actuel, inchangé) …
    elif COLUMN_ZERO_ITEM_RE.match(line):
        return idx
return None
```

Deux points de conception, et ils comptent :

- **L'état est pris AU DÉBUT de la ligne**, jamais à la fin. Une ligne qui ouvre une
  chaîne est encore une ligne de code (`const F: &str = "\`) et doit rester lisible
  par les prédicats ; une ligne qui en ferme une (`";`) ne l'est pas.
- **Un seul balayage, pas deux.** Le prototype de mesure repartait à la coupure avec
  `state = None` ; porter l'état en continu est plus sûr (la coupure ne peut pas être
  détectée depuis l'intérieur d'une chaîne) et c'est ce qui donne gratuitement une
  propriété que la version actuelle n'a pas : un `#[cfg(test)] mod x {` écrit **dans**
  une fixture de chaîne ne coupe plus.

`RAW_STRING_OPEN_RE` cesse d'être consommée par cette fonction (la logique passe dans
`advance_lexical_state`). Elle est **retirée** si aucun autre site ne la lit, ou
laissée avec sa prose si elle est partagée — à vérifier d'un `grep` à
l'implémentation ; dans les deux cas la prose explicative qu'elle porte est déplacée,
pas perdue.

### D3 — `production_slices` : hors périmètre, et la mesure dit pourquoi

Le même scanner pourrait servir à `production_slices` (l. 291-359), dont le détecteur
de coupure a la même cécité. **Mesure : la coupure est identique sur les 417 fichiers
de l'arbre, avec et sans le scanner — zéro divergence.** Appliquer le scanner là
serait un changement sans effet mesurable sur une fonction qui alimente l'inventaire
D2 que le lint **rapporte** — donc du risque de faux positif acheté contre rien.
La mesure est inscrite dans la prose, ce qui rend le non-changement relisible.

### D4 — La suite : trois étages

Tout entre dans `scripts/test-verify-egress-manifest.sh`, section N15, après
l'anti-vacuité existante (qui n'est pas touchée). Le fichier a déjà ses deux idiomes :
`python3 -B - "$REPO_ROOT" <<'PY'` pour appeler le moteur en bibliothèque, et
`fixture_root`/`$TMPROOT` pour les arborescences synthétiques.

**Étage 1 — micro-fixtures du prédicat (R6).** Un bloc Python inline, fixtures de
quatre à sept lignes écrites en littéral, qui importe `production_resumes_after_cut`.
Le bloc imprime une ligne `nom=résultat` par cas ; bash les compare. Les sept cas et
leur attendu, **tous mesurés sur le prototype** :

| cas | fixture | attendu | ce qu'il prouve |
|---|---|---|---|
| `cont-string` | `const F: &str = "\` + `async fn dedans() {}` + `";` puis `fn apres() {}` | ligne de `fn apres` | **la classe du ticket** : la fixture est sautée, la vraie reprise est vue |
| `cont-string-sans-reprise` | la même, sans rien derrière | `None` | pas de reprise inventée |
| `raw` | `r#"` + `fn dedans() {}` + `"#;` | `None` | non-régression du seul saut d'aujourd'hui |
| `comment-quote` | `// un " seul` puis `fn apres() {}` | ligne de `fn apres` | un guillemet en prose n'ouvre pas de chaîne — **la garde anti-sous-détection** |
| `char-quote` | `let c = '"';` puis `fn apres() {}` | ligne de `fn apres` | le littéral de caractère non plus |
| `block-comment` | `/* bloc` + `fn dedans() {}` + `*/` puis `fn apres() {}` | ligne de `fn apres` | le commentaire de bloc est sauté |
| `lifetime` | `fn f<'a>(x: &'a str) {}` puis `fn apres() {}` | ligne de `fn apres` | un lifetime n'est pas un littéral de caractère |

Trois de ces sept (`cont-string`, `char-quote`, `block-comment`) **échouent avant le
correctif** : c'est la non-vacuité du vol, et elle se vérifie à la main une fois avant
d'ouvrir la PR.

**Étage 2 — fermeture lexicale sur l'arbre réel (R4).** Un bloc Python inline qui,
pour chacun des fichiers que le lint scanne, déroule `advance_lexical_state` sur toutes
les lignes et vérifie que l'état final est `None`. Il imprime
`scannés=<n> hors_code=<m>` ; l'assertion exige `m == 0` **et** `n > 0` (la
cardinalité porte l'anti-vacuité : si le scan cessait de voir des fichiers,
`hors_code=0` se lirait comme un arbre sain, classe mika#2205). Mesure du jour :
`scannés=417 hors_code=0`.

L'échec nomme le fichier et l'état où le scanner s'est perdu — c'est l'information
dont a besoin l'opérateur, puisqu'un fichier perdu signifie que tout ce qui le suit a
été sauté en silence.

**Étage 3 — la conséquence, bout en bout (R7).** Une fixture synthétique sous
`$TMPROOT` (`crates/demo/src/lib.rs`) portant : un `#[cfg(test)] mod tests {`, une
fixture Rust en `"\` avec un `async fn` en colonne zéro, puis une URL
`"https://api.github.com/x"` dans une chaîne de test. `--audit-cut-holes` sur cette
racine rend **exit 0 et sortie vide** après le correctif, et rendrait exit 1 en
nommant `api.github.com` avant. C'est la forme réduite de #3133, et c'est le seul cas
qui traverse `audit_cut_holes` plutôt que le prédicat nu.

### D5 — La prose, qui est un livrable

Le docstring de `production_resumes_after_cut` (l. 362-382) annonce aujourd'hui le
seul saut des chaînes brutes et chiffre son effet (« six fichiers … six reprises dont
aucune n'existe »). Il est réécrit sur les **quatre** classes, avec la mesure du jour
(10 reprises → 1) et la citation de #3134/#3133. Le commentaire de `audit_cut_holes`
(l. 420-434) qui dit « la détection de reprise sur-rapporte (une fixture Rust dans une
chaîne à continuation de ligne est lue comme une reprise) » décrit **exactement le
défaut corrigé** : il est mis à jour, sinon la prose du fichier contredit son code.
Le commentaire de N15 dans la suite gagne la même mise à jour.

---

## Périmètre — ce vol et ce qu'il laisse

**Dans ce vol :** le scanner lexical, la refonte du balayage de
`production_resumes_after_cut`, les trois étages de la suite, la prose des trois
sites.

**Hors de ce vol, nommés, chiffrés, et sans ticket de suivi ouvert :**

- **`production_slices`** — zéro divergence mesurée sur 417 fichiers (D3). Rien à
  suivre : il n'y a pas de défaut, il y a une cécité sans population.
- **Le palliatif `176bab64` n'est PAS reverti.** Repasser les neuf constantes de
  `auto_pull.rs` en chaînes ordinaires coûterait ~27 lignes de churn Rust, forcerait
  un `cargo test -p mika-agent` dans la fin de pipeline, et prouverait ce que le cas
  `cont-string` de l'étage 1 prouve déjà sur une fixture de sept lignes. Les chaînes
  brutes sont par ailleurs la meilleure forme pour une fixture Rust (pas
  d'échappement). Le vol reste **sans aucune ligne de Rust**, ce qui allège sa fin de
  pipeline d'autant.
- **`audit_cut_holes` ne filtre pas les régions de chaîne** quand il cherche des hosts
  derrière une reprise **réelle**. Après ce vol, la seule reprise réelle de l'arbre
  (`dashboard.rs`) ne porte aucun host derrière — population vide, mesurée,
  `rc=0`. Lui câbler le scanner serait du code sans population : le jour où elle
  existe, elle sera visible par un faux sink rapporté, pas silencieuse.

---

## Verification Contract

Toutes les commandes depuis la racine du worktree.

1. `bash scripts/test-verify-egress-manifest.sh` → `0 failed`, les cas N1–N17
   historiques inclus et inchangés, plus les trois étages de D4 (R5, R6, R7, R4).
2. `bash scripts/verify-egress-manifest.sh` → exit 0 sur l'arbre. Le scanner ne
   change pas l'inventaire D2, donc le verdict du lint est inchangé.
3. `python3 -B scripts/lib/egress_manifest_lint.py --audit-cut-holes .` → exit 0,
   sortie vide (inchangé par rapport à aujourd'hui — mais désormais vert **par
   construction**, pas par absence de sink derrière neuf fausses reprises).
4. **Contrôle de cardinalité, imprimé, pas supposé** : le bloc python de l'étage 1
   ou une sonde équivalente doit montrer **1** fichier avec reprise sur l'arbre
   (contre 10 aujourd'hui) et ce fichier doit être
   `crates/mika-agent/src/server/dashboard.rs`. Un `None` partout serait un prédicat
   mort, pas un arbre propre.
5. **Contrôle de non-vacuité, à la main, une fois, avant d'ouvrir la PR :** sur le
   moteur **non corrigé**, lancer le bloc python de l'étage 1 et vérifier que les cas
   `cont-string`, `char-quote` et `block-comment` ÉCHOUENT. Puis appliquer le
   correctif et vérifier qu'ils passent. Sans ce contrôle, trois des sept cas
   pourraient être verts par accident.
6. `bash scripts/verify-egress-uniqueness.sh` → vert (il dérive ses patterns du même
   moteur via `--confined-hosts` ; le mode n'est pas touché, l'assertion est une
   non-régression).
7. `bash scripts/check-suite-wiring.sh` → vert et `unwired=0` (rien n'est ajouté ni
   retiré du câblage).
8. `bash scripts/verify-pipeline.sh origin/main` → vert.

Ce qui n'est **pas** dans le contrat, délibérément : aucun `cargo test`, aucun
`cargo clippy`. Le vol ne touche aucune ligne de Rust (§ Périmètre).

---

## Definition of Done

- `advance_lexical_state` existe dans `scripts/lib/egress_manifest_lint.py`, pure,
  documentée sur ses quatre états et sur la direction de son erreur.
- `production_resumes_after_cut` fait un seul balayage portant l'état lexical, prend
  l'état **au début** de chaque ligne, et ne compte aucune accolade.
- Le prédicat rend `None` sur `auto_pull.rs@4cfd3e3b` et une reprise sur
  `dashboard.rs` ; la cardinalité sur l'arbre passe de 10 à 1.
- Les trois étages de la suite sont en place sous N15 ; l'anti-vacuité témoin
  existante est intacte.
- La prose de `production_resumes_after_cut`, de `audit_cut_holes` et du commentaire
  de N15 décrit les quatre classes sautées et cite #3134.
- Les huit points du Verification Contract passent, contrôle manuel de non-vacuité
  inclus.
- `docs/solutions/ci-cd/` gagne un writeup : *une fixture de test dans une chaîne
  ordinaire multi-ligne se lit comme du code* — le défaut, la mesure (10 → 1, six
  faux sinks), la raison pour laquelle le palliatif ne suffisait pas, et le principe
  (suivre un état de chaîne n'est pas compter un bloc).
- Corps de PR : `Closes #3134`, et
  `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification Contract.

**AC1.** `production_resumes_after_cut` rend `None` sur le contenu de
`crates/mika-agent/src/auto_pull.rs` à la révision `4cfd3e3b` (la forme du ticket :
`const PHASE0: &str = "\` suivi de `async fn …` en colonne zéro). Avant le
correctif il rend `9812`.

**AC2.** Sur l'arbre scanné par le lint, exactement **un** fichier voit une reprise,
et c'est `crates/mika-agent/src/server/dashboard.rs`. Avant le correctif, dix.

**AC3.** Le prédicat saute les quatre classes de région : chaîne brute (non-régression),
chaîne ordinaire restée ouverte en fin de ligne, commentaire de bloc, littéral de
caractère. Les sept micro-fixtures de l'étage 1 rendent chacune le résultat de la
table D4, et trois d'entre elles échouent sur le moteur non corrigé.

**AC4.** Un guillemet non apparié dans un commentaire (`// un " seul`) et un littéral
`'"'` n'ouvrent **pas** de région sautée : le prédicat voit toujours l'item en colonne
zéro qui suit.

**AC5.** Pour chaque fichier que le lint scanne, l'état lexical en fin de fichier est
« code » : la sonde de l'étage 2 imprime `scannés=<n>` avec `n > 0` et
`hors_code=0`. Un échec nomme le fichier et l'état.

**AC6.** `--audit-cut-holes` sur une racine synthétique portant une fixture Rust en
`"\` suivie d'une URL `https://api.github.com/…` dans une chaîne de test rend exit 0
et sortie vide. Avant le correctif, exit 1 en nommant `api.github.com`.

**AC7.** L'assertion anti-vacuité existante de N15
(« le détecteur voit encore la reprise témoin », `dashboard.rs`) est inchangée et
verte.

**AC8.** `bash scripts/test-verify-egress-manifest.sh` rend `0 failed`, cas N1–N17
historiques inclus et inchangés. `bash scripts/verify-egress-manifest.sh`,
`bash scripts/verify-egress-uniqueness.sh` et
`python3 -B scripts/lib/egress_manifest_lint.py --audit-cut-holes .` rendent exit 0.

**AC9.** Aucun job CI, cible `make`, mode CLI du moteur ou entrée
`scripts/suite-wiring.tsv` n'est ajouté ni retiré ;
`bash scripts/check-suite-wiring.sh` passe avec `unwired=0`. Aucune ligne de Rust
n'est modifiée.

---

## Fire-Disposition

Ce vol livre des détecteurs : les trois étages de la suite N15 (micro-fixtures du
prédicat, fermeture lexicale sur l'arbre, cas bout-en-bout), dont le chemin de succès
est « aucune violation trouvée ». Il modifie par ailleurs un détecteur existant
(`production_resumes_after_cut`, consommé par N15).

**Option retenue : (a) exception nommée en allowlist — avec une population
d'exceptions VIDE.**

Détail d'implémentation :

1. **L'allowlist n'est pas créée : elle existe déjà.** Le lint egress porte son
   mécanisme à `scripts/egress-manifest-exceptions.tsv`, livré vide, dont la prose de
   `main` (l. 944-955) dit que les entrées « ne suppriment pas une violation : elles
   sont listées pour que l'assertion auto-nettoyante du test puisse vérifier qu'elles
   correspondent encore à une violation réelle ». Ce vol n'ajoute aucun second
   mécanisme et n'y inscrit aucune entrée.

2. **Chaque détecteur livré atterrit ARMÉ, sur une population mesurée et vide.**
   - Étage 1 (prédicat) : sept cas, tous verts après le correctif, trois rouges avant.
     Mesurés sur prototype.
   - Étage 2 (fermeture lexicale) : `scannés=417 hors_code=0` sur l'arbre
     du 2026-11-24. Zéro violation à excepter.
   - Étage 3 (bout en bout) : fixture synthétique, verte après correctif.
   - `--audit-cut-holes` : `rc=0`, sortie vide, avant comme après.

3. **Les dix fausses reprises de l'arbre ne sont pas exceptées : elles sont
   supprimées** — neuf par le correctif lui-même (ce sont les faux positifs), la
   dixième (`dashboard.rs`) est une **vraie** reprise qui doit rester visible et le
   reste. Aucune n'a besoin d'entrée d'allowlist, et chacune est nommée dans la table
   § Étendue avec son fichier, sa ligne et sa nature.

4. **Les assertions auto-nettoyantes sont celles qui lisent l'arbre réel**, et il y en
   a deux :
   - le cas N15 historique (`--audit-cut-holes` rend 0) — il rougit le jour où une
     reprise réelle porte un sink derrière ;
   - l'étage 2 (fermeture lexicale) — il rougit le jour où un fichier de l'arbre perd
     le scanner, ce qui est le seul mode de défaillance **silencieux** du correctif.

   Les deux sont déjà câblées : N15 appelle le moteur en direct dans une suite que la
   CI lance (N16 l'énumère dans sa liste de consommateurs).

5. **La cardinalité porte l'anti-vacuité sur les deux assertions d'arbre** (classe
   mika#2205) : l'étage 2 exige `scannés > 0`, et le témoin `dashboard.rs` exige que le
   prédicat voie encore une reprise. Un prédicat qui rendrait `None` partout serait un
   détecteur mort, et ce vol le rendrait visible plutôt que vert.

6. **Rien n'est livré désarmé** : aucun cas commenté, aucun `#[ignore]`, aucune sortie
   conditionnelle. Les détecteurs mordent dès le merge.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/lib/egress_manifest_lint.py` — `advance_lexical_state` (code + prose des 4 états) | 90 |
| `scripts/lib/egress_manifest_lint.py` — refonte du balayage + prose de `production_resumes_after_cut` et `audit_cut_holes` | 35 |
| `scripts/test-verify-egress-manifest.sh` — étage 1 (7 micro-fixtures + comparaison) | 60 |
| `scripts/test-verify-egress-manifest.sh` — étage 2 (fermeture lexicale sur l'arbre) | 30 |
| `scripts/test-verify-egress-manifest.sh` — étage 3 (fixture synthétique bout en bout) | 30 |

Total estimé : 245 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 245 < 279, et la marge tient à trois propriétés
mesurées plutôt qu'espérées :

- **Le correctif est déjà prototypé.** `advance_lexical_state` et le balayage
  refondu ont été écrits, lancés sur les 417 fichiers de l'arbre, sur
  `auto_pull.rs@4cfd3e3b` et sur les sept micro-fixtures (`.pilot-scratch/proto_scan.py`).
  L'implémentation est une transcription avec sa prose, pas une recherche : tous les
  nombres de ce plan (10 → 1, six faux sinks, 417/0, zéro divergence sur
  `production_slices`) sont des mesures, pas des estimations.
- **Un seul fichier de moteur et un seul fichier de suite**, aux points d'insertion
  connus : `egress_manifest_lint.py:286-417` et `test-verify-egress-manifest.sh:717-764`.
  Les deux idiomes de la suite (python inline, `fixture_root`) existent déjà et sont
  copiés, pas inventés.
- **La fin de pipeline est légère : zéro ligne de Rust** (§ Périmètre — le palliatif
  n'est pas reverti). Pas de `cargo build`, pas de `cargo test`, pas de
  `make verify-bundled-skills` : quatre suites shell/Python qui tournent en secondes,
  plus `verify-pipeline.sh`.

---

## Ordre de commit

**L'ordre est contraint, pas esthétique :** trois cas de l'étage 1 doivent être vus
rouges avant que le correctif n'existe (point 5 du Verification Contract). Les poser
d'abord rendrait le commit intermédiaire rouge et la bissection illisible ; les poser
après perdrait la preuve de non-vacuité. La séquence ci-dessous résout les deux : le
contrôle manuel se fait **avant** le commit 1, sur le moteur encore intact.

1. **`fix(3134): le prédicat de reprise suit l'état des chaînes et des commentaires`**
   — `advance_lexical_state`, le balayage refondu, la prose des deux docstrings.
   Plancher : `--audit-cut-holes` rend 0 ; la cardinalité sur l'arbre est de 1 et
   nomme `dashboard.rs` ; `verify-egress-manifest.sh` et
   `verify-egress-uniqueness.sh` restent verts ; la suite N15 historique passe.
2. **`test(3134): N15 pinne les quatre classes de région et la fermeture lexicale`**
   — les trois étages, la mise à jour du commentaire de N15. Plancher : les huit
   points du Verification Contract passent.
3. **`docs(3134): une fixture dans une chaîne ordinaire se lit comme du code`** — le
   writeup `docs/solutions/ci-cd/`.

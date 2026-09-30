---
title: "Une parité bash→Rust se mesure une fois contre le bash pré-bascule — l'assertion qui reste porte sur une relation interne au Rust"
date: 2026-09-30
category: best-practices
module: crates/mika-agent/src/plan_callout + skills/bundled/_shared/dispatch-lib.sh
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - "Migrer un prédicat de dispatch-lib.sh vers Rust derrière une sous-commande CLI que le bash appelle"
  - "Deux lecteurs (bash et Rust) doivent lire le même corpus doré pendant la transition"
  - "Un test de parité compare bash et Rust alors que le bash délègue déjà au Rust"
  - "Un fichier tabulaire est lu à la fois par `IFS=$'\\t' read` et par un autre langage"
  - "Une valeur que le nouveau lecteur produit ne tient pas dans le canal qui la transporte"
symptoms:
  - "Après la bascule, le test de parité compare le Rust à lui-même et passe par construction"
  - "Les divergences connues entre lecteurs vivent en commentaire au lieu d'être exigées par un test"
  - "`IFS=$'\\t' read` fusionne deux tabulations consécutives et décale les colonnes après un champ vide"
resolution_type: migration
related_components:
  - dispatch-lib
  - mika-cli
  - testing_framework
related:
  - mika#2194
  - mika#2607
  - mika#2120
  - mika#2205
tags:
  - bash-to-rust
  - parity-test
  - golden-corpus
  - dispatch-lib
  - plan-callout
  - tsv-sentinel
  - auto-nettoyant
  - mika-2194
---

# La preuve de parité d'une migration est éphémère : la mesurer une fois, et asserter en permanence une relation qui garde deux côtés

## Contexte

mika#2194 phase 1 (PR mika#2607, non mergée à la rédaction) retire le motif du
callout `Plan` de `dispatch-lib.sh` : `_extract_plan_path` ne porte plus de PCRE,
elle délègue au lecteur unique Rust (`crates/mika-agent/src/plan_callout.rs:188`,
`fn plan_callout`) par la sous-commande `mika plan-callout`
(`skills/bundled/_shared/dispatch-lib.sh:8926`). Un prédicat change de langage ;
pendant la transition, deux lecteurs coexistent.

Le défaut de départ n'était pas un bug, c'était une configuration aveugle.
Le Rust avait 6 corps d'issue réels, gelés, tous préfixés ; le bash avait 17
assertions à fixtures inline. **Aucune entrée n'était commune** : personne
n'avait jamais exécuté les deux lecteurs sur la même entrée
(`docs/architecture/dispatch-lib-migration.md` § 4, « Le défaut que cette forme
ferme »). Une divergence ne pouvait pas être vue.

La doctrine spécifique à dispatch-lib vit dans
`docs/architecture/dispatch-lib-migration.md` (§ 2 canal, § 4 corpus doré, § 6
phasage). Cette leçon en extrait la partie qui transporte : **ce qu'on prouve
pendant une migration de prédicat, et ce qu'on ne peut plus prouver après**.
Elle s'applique telle quelle aux phases 2 (`_committed_plan_on_branch`,
`_set_up_worktree`) et 3 (`_parse_disposition` / `_parse_verdict`) de mika#2194.

## La règle

### 1. Un corpus doré unique, lu par les deux lecteurs

Un répertoire de corps, un fichier d'attendus, deux lecteurs qui le lisent
chacun dans leur langage : `crates/mika-agent/tests/plan_callout_parity.rs`
(Rust) et le bloc « mika#2194 » de `skills/bundled/_shared/test-dispatch-lib.sh:9676`
(bash). Le format est un **format de fil** : ses valeurs légales sont énumérées
à un seul site.

```rust
// crates/mika-agent/tests/plan_callout_parity.rs:39
const PARITY_VALUES: &[&str] = &[
    "equal",
    "divergent-fences",
    "divergent-bash-legacy",
    "divergent-multiline",
];
```

L'en-tête commenté de `crates/mika-agent/tests/fixtures/plan_callout_bodies/expectations.tsv`
porte la sémantique des six colonnes (`fichier · rc · raw · normalized · parity
· phase`) ; les données commencent l.81. Les attendus décrivent le résultat sous
la politique de l'**ancien** lecteur (`FenceHandling::Keep`, celle du bash) :
c'est la moitié dont la parité était inconnue.

### 2. La preuve se mesure une fois, contre l'ancien lecteur d'avant la bascule

Après la bascule, le bash appelle le Rust. Un lecteur comparé à lui-même est
toujours d'accord avec lui-même. La passe bash permanente est donc une
**non-régression**, jamais une preuve — et le fichier le dit
(`test-dispatch-lib.sh:9685-9690`).

La preuve a été faite **à l'implémentation**, contre le `dispatch-lib.sh`
extrait de `HEAD` sous `.pilot-scratch/` : 17 cas, 17 concordances, 3
divergences mesurées (chiffres rapportés par la doctrine, § 4
« `pre-switch` / `post-switch` » ; l'artefact de scratch n'est pas versionné).
La colonne `phase` encode cette asymétrie : `both` = exerçable après la bascule,
`pre-switch` = décrit un comportement qui n'existe plus. Le bash ne lit que
`both` (`test-dispatch-lib.sh:9756`).

Conséquence opératoire : **la mesure contre l'ancien code est une étape de
l'implémentation, pas un test**. Si on la saute, elle ne peut plus être faite
depuis l'arbre fusionné sans ressusciter l'ancien code à la main.

### 3. L'assertion permanente porte sur une relation qui garde deux côtés

Le plan de la phase 1 faisait porter `parity` sur bash↔Rust — une relation qui
cesse d'avoir deux côtés à la bascule. L'implémentation l'a déplacée sur
`Keep`↔`Strip`, interne au Rust, donc mesurable pour toujours
(`plan_callout_parity.rs:213`) :

```rust
if e.parity == "divergent-fences" {
    assert_ne!(keep, strip, "… ASSERTION AUTO-NETTOYANTE : la divergence a été \
        tranchée. Retirez cette ligne du TSV …");
    divergences_declarees += 1;
} else {
    assert_eq!(keep, strip, "… Une divergence que personne n'a déclarée …");
    egalites_verifiees += 1;
}
// l.248 / l.254
assert!(divergences_declarees > 0, "… il ne vérifie plus rien (classe mika#2205)");
assert!(egalites_verifiees > 0, "… le contrôle négatif de ce test est vide …");
```

Quatre propriétés, toutes porteuses :

- une ligne `divergent-fences` dont les deux politiques rendent la même valeur
  **rougit** : l'exception ne peut pas devenir périmée en silence ;
- une ligne `equal` qui diverge **rougit** : pas de divergence non déclarée ;
- zéro divergence déclarée ou zéro égalité vérifiée **rougit** : un test dont
  l'une des deux branches est vide ne distingue plus rien (classe mika#2205) ;
- une ligne `pre-switch` doit déclarer une divergence
  (`plan_callout_parity.rs:279`) : sortir un cas de la passe permanente sans
  raison écrite fabrique un cas que plus personne n'exerce.

### 4. Les divergences sont déclarées, pas masquées ni corrigées

Trois divergences mesurées, dont deux absentes du ticket et du plan
(`expectations.tsv:114`, `:122`, `:130`) : blocs clôturés, backtick fermant
exigé, capture qui traverse les lignes. Chacune est **nommée, testée comme
divergente, non corrigée** : une migration de langage n'est pas le moment d'un
changement de comportement (borne B1, doctrine § 3). Le canal lui-même fige ce
choix : `decide` interroge `FenceHandling::Keep`, pas `Strip`
(`crates/mika-cli/src/commands/plan_callout.rs:99`).

### 5. Le format doit être lu pareil par les deux lecteurs — sentinelle dans le format

Piège mesuré à la première exécution de la preuve pré-bascule : TAB est un
caractère *whitespace* d'IFS, donc `IFS=$'\t' read` **fusionne les tabulations
consécutives**. Une colonne vide disparaît et tout ce qui suit se décale ; les
cinq contrôles négatifs lisaient leur `parity` dans la colonne `normalized` vide
et échouaient sur « parity inconnue '' ». Le Rust, `line.split('\t')`
(`plan_callout_parity.rs:83`), ne fusionne rien et les lisait correctement.

Remède : une colonne sans valeur porte `-`, jamais le vide
(`expectations.tsv:94` et suivantes). La sentinelle est **dans le format**, pas
dans un contournement chez l'un des lecteurs — un format que deux lecteurs ne
lisent pas pareil est exactement ce que la migration existe pour retirer.

### 6. Anti-vacuité des deux côtés, compte de cas imprimé

Les deux lecteurs échouent sur corpus introuvable, vide, ou corps absent
(`plan_callout_parity.rs:66`, `:126`, `:134`, `:198`), et chacun **imprime son
compte de cas**. Côté bash, un terme de plus : le harnais source
`dispatch-lib` par `source … 2>/dev/null || true`, donc un sourcing raté laisse
les assertions vertes sur des fonctions inexistantes. Le bloc exige que la
fonction existe avant de mesurer :

```bash
# test-dispatch-lib.sh:9714
assert_eq "mika#2194 (anti-vacuité 3/3): _extract_plan_path est définie après le sourcing" "yes" \
    "$(if declare -f _extract_plan_path >/dev/null; then printf 'yes'; else printf 'no'; fi)"
```

Et si le binaire `mika` manque, le bloc **SKIPPE en le disant** plutôt que de
compter des assertions qu'il n'a pas pu faire (`test-dispatch-lib.sh:9735`).

### 7. Quand la migration expose une valeur que le canal ne transporte pas : borner le canal

La divergence n°3 : `[^`]+` n'exclut pas `\n` en Rust, donc un backtick plus
loin dans le corps ferme la capture sur un chemin multi-ligne ; `grep` travaillait
ligne à ligne et tronquait. Resserrer le motif aurait changé une tolérance.
C'est le **canal** qui borne, là où la valeur devient une ligne de stdout :

```rust
// crates/mika-cli/src/commands/plan_callout.rs:109-111
if path.contains('\n') || path.contains('\r') {
    return CalloutOutcome::Unreadable {
        reason: "path_not_single_line",
```

Le canal lui-même suit trois règles (doctrine § 2) : le corps passe par un
**fichier** (`--body-file`, via `mktemp`, `dispatch-lib.sh:8913-8926`), jamais par
argv — argv ré-introduirait la classe de portée de guillemets (cpp#157) ; trois
codes `0` (lu) / `1` (« non ») / `≥2` (« je n'ai pas pu regarder »,
`EXIT_UNREADABLE`, `plan_callout.rs:64`), relayés tels quels ; pas de repli bash,
qui serait une seconde implémentation.

## Pourquoi ça compte

Une parité bash↔Rust qui tourne en CI après la bascule a l'air d'une preuve et
n'en est pas une : elle compare le Rust à lui-même. Sans la colonne `phase` et
la mesure pré-bascule, la migration aurait livré un vert permanent et zéro
information sur les trois divergences — dont une (`backtick-unterminated`) touche
`plan_ownership`, qui décide d'un **abandon** de ticket (mika#2020).

Les exceptions non auto-nettoyantes pourrissent : une ligne « divergence
connue » qui survit à la correction de la divergence devient un mensonge avec
l'autorité d'un inventaire. Et un test de parité vert sur zéro cas — corpus
déplacé, fonction non sourcée, binaire absent — est le seul mode de panne que
ni l'un ni l'autre des lecteurs ne verrait seul.

Le troisième code de sortie n'est pas un raffinement : la population « fichier
de corps illisible » est **créée par la migration** (avant, le corps était en
variable). La fondre dans `1` fabriquerait un silence qui n'existait pas.

## Quand l'appliquer

- Toute migration où un prédicat change de langage et où l'ancien lecteur finit
  par déléguer au nouveau : phases 2 et 3 de mika#2194 en premier lieu, et tout
  portage bash→Rust ou Python→Rust derrière une CLI.
- Toute table d'exceptions « divergence connue » : la rendre auto-nettoyante.
- Tout test de parité qui lit un fichier de fixtures : anti-vacuité et compte de
  cas imprimé, des deux côtés.
- Tout format tabulaire lu par bash **et** un autre langage : pas de colonne
  vide, sentinelle explicite.

Ne s'applique pas tel quel à la phase 3 : `_parse_disposition` rend deux
valeurs via un fichier, et son tier 2 est une paraphrase dont la migration est
un arbitrage de tolérance, pas un portage (doctrine § 6). Le corpus unique et la
mesure pré-bascule restent requis ; la relation permanente à deux côtés est à
trouver.

## Exemples

**Avant — deux corpus disjoints, parité invérifiable :**

```
crates/mika-agent/tests/fixtures/plan_callout_bodies/   6 corps réels (Rust seul)
skills/bundled/_shared/test-dispatch-lib.sh             17 assertions inline (bash seul)
entrées communes : 0
```

**Après — un corpus, deux lecteurs, preuve datée, assertion survivante :**

```
expectations.tsv   fichier · rc · raw · normalized · parity · phase
  1680.md               0  mika/docs/plans/…  docs/plans/…  equal              both
  neg-brainstorms.md    1  -                  -             equal              both
  fences-quoted-…       0  docs/plans/…       docs/plans/…  divergent-fences   both
  backtick-unterminated 1  -                  -             divergent-bash-legacy  pre-switch
  backtick-late-close   0  <multiline>        <multiline>   divergent-multiline    pre-switch
```

**Avant — colonne vide, décalage silencieux côté bash :**

```
neg-brainstorms.md<TAB>1<TAB><TAB><TAB>equal<TAB>both
# bash : m_raw=equal m_norm=both m_parity='' → « parity inconnue '' »
# Rust : cols[4]="equal" → correct
```

**Après — sentinelle dans le format :**

```
neg-brainstorms.md<TAB>1<TAB>-<TAB>-<TAB>equal<TAB>both
```

**Avant — parité asserte bash↔Rust (éphémère, vraie par construction après la bascule).**
**Après — parité asserte `Keep`↔`Strip` (interne au Rust, auto-nettoyante,
gardée par ≥1 divergence et ≥1 égalité).**

## Voir aussi

- `docs/architecture/dispatch-lib-migration.md` — la doctrine propre à la migration de `dispatch-lib` (critères C1-C3, canal, phasage, sondes post-déploiement). Cette leçon n'en garde que ce qui vaut pour d'autres migrations.
- `docs/solutions/best-practices/deux-lecteurs-dune-meme-variable-denv-divergent-en-silence-2026-09-07.md` — deux lecteurs d'une même valeur qui divergent en silence : c'est la même famille de problème, avec une autre parade (interroger le parseur canonique).
- `docs/solutions/ci-cd/2026-08-27-porting-a-precommit-detector-to-ci-parity-traps.md` — lancer l'ancien et le nouveau sur les mêmes entrées réelles. C'est l'étape 2 ci-dessus, sans la question de ce qui reste prouvable après la bascule.
- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md` — un seul parseur quand c'est possible. mika#2607 met ce design en œuvre, et la règle 7 (borner le canal) en est le contrepoint.
- `docs/solutions/best-practices/invariant-test-stricter-than-spec-needs-an-allowlist-2026-06-27.md` — une allowlist auto-nettoyante : même mécanisme que les divergences déclarées qui rougissent quand elles disparaissent.
- `docs/solutions/best-practices/trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md` — la passe de parité post-bascule, présentée comme une preuve, serait un cas de plus de cette classe.

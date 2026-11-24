---
title: "Une fixture de test dans une chaîne ordinaire multi-ligne se lit comme du code, et le lint accuse six sinks qui n'existent pas"
category: ci-cd
date: 2026-11-24
tags:
  - ci
  - egress
  - lint
  - python
  - rust
  - lexer
  - false-positive
  - guard-observability
  - anti-vacuite
severity: medium
affected_components:
  - scripts/lib/egress_manifest_lint.py
  - scripts/test-verify-egress-manifest.sh
related_issues:
  - mika#3134
  - mika#3133
  - mika#3117
  - mika#2408
  - mika#2205
---

# Une fixture de test dans une chaîne ordinaire se lit comme du code

## Le défaut, tel qu'il s'est manifesté

Sur la PR #3133, le job « Egress Manifest Lint » a rougi en annonçant **six
sinks `github.com` hors inventaire** dans `crates/mika-agent/src/auto_pull.rs`.
Aucun n'existait. Les six étaient des hosts cités dans la **prose d'un test**,
et le diff de la PR ne touchait pas à l'egress.

L'auteur a payé un détour de diagnostic complet pour un défaut du prédicat, sur
un message qui parlait de sinks réseau.

## La chaîne causale

`production_resumes_after_cut` (`scripts/lib/egress_manifest_lint.py`) remesure
en continu le trou assumé de `production_slices` : ce dernier **coupe** le
fichier au premier `#[cfg(test)] mod X {` au lieu de sauter le bloc puis de
reprendre, donc du code de production placé **après** ce bloc verrait son sink
disparaître en silence. Le prédicat cherche l'évidence de cette reprise : un
item Rust **en colonne zéro** après la coupure. Il ne compte aucune accolade —
c'est tout son intérêt, le comptage de blocs ayant été écrit, essayé et refusé
sur mesure (il fermait le module de test de `gateway/src/telegram.rs` 265
lignes trop tôt).

Sa boucle sautait **une seule** classe de région : la chaîne **brute**
(`r#"…"#`). Elle ne connaissait ni la chaîne **ordinaire** restée ouverte en fin
de ligne, ni le commentaire de bloc, ni le littéral de caractère.

`auto_pull.rs` portait ceci :

```rust
#[cfg(test)]
mod tests {
    const PHASE0: &str = "\
async fn phase0_feed_ready_pool(db: &Db) {
```

L'`async fn` est en colonne zéro. Il est **dans une chaîne**. Le prédicat a
conclu « la production reprend à la ligne 9812 », et `audit_cut_holes` a balayé
les 4 600 lignes suivantes — le module de test entier — en y récoltant six
`github.com` de prose.

## La mesure, qui est plus large que le fichier du ticket

Le palliatif de #3133 (`176bab64`) a repassé les fixtures de `auto_pull.rs` en
`r#"…"#`. Il n'a rien dit du reste de l'arbre. Relevé sur les **417 fichiers**
que le lint scanne :

| | avant | après |
|---|---|---|
| fichiers voyant une reprise | **10** | **1** |
| dont fausses (fixture en chaîne) | 9 | 0 |
| dont réelles | 1 (`server/dashboard.rs`) | 1 (`server/dashboard.rs`) |

Les neuf fausses ne portaient aucun sink derrière elles, donc
`--audit-cut-holes` rendait `rc=0`. **L'arbre était vert par chance, pas par
construction** — et la prose du lint le disait elle-même : « cette
sur-détection ne coûte rien tant que la région ne porte aucun sink ». #3134 est
la journée où elle a coûté.

## Pourquoi le palliatif ne suffisait pas

Repasser une fixture en `r#"…"#` est une **convention de style qu'aucune garde
n'enforce**. Neuf sites de l'arbre portaient encore la forme en `"\`, et la
prochaine fixture Rust écrite ainsi derrière un `#[cfg(test)] mod` aurait fait
rougir le même cas — sur un diff sans rapport, dans un job dont le message
parle de sinks `github.com`. La forme est la conséquence ; le défaut est dans
le prédicat.

## Le principe : suivre un état de chaîne n'est pas compter un bloc

C'est la distinction qui débloque le correctif. La contrainte du lint interdit
le **comptage d'accolades** — une structure à profondeur, avec une pile, dont
les formes non modélisées produisent des faux positifs mesurés. Suivre l'état
lexical d'une ligne est tout autre chose : une **machine à états par ligne**,
sans profondeur et sans pile.

`advance_lexical_state(line, state) -> state` consomme une ligne caractère par
caractère et rend l'état à sa fin. Quatre états : code, chaîne ordinaire,
chaîne brute (avec ses `#` à refermer), commentaire de bloc. Le prédicat
n'évalue ses motifs que sur les lignes qui **commencent** hors région.

Deux points de conception portent le résultat :

- **L'état est pris AU DÉBUT de la ligne**, jamais à la fin. Une ligne qui
  *ouvre* une chaîne est encore du code (`const F: &str = "\`) et doit rester
  lisible par les prédicats ; une ligne qui en *ferme* une (`";`) ne l'est pas.
- **Un seul balayage** de la première ligne à la dernière, portant l'état sans
  interruption, avec une variable de phase avant/après la coupure. La version
  en deux passes repartait à la coupure avec un état neuf. Le balayage continu
  donne gratuitement une propriété qu'elle n'avait pas : un
  `#[cfg(test)] mod x {` écrit **dans** une fixture de chaîne ne coupe plus.

## La direction de l'erreur est la vraie difficulté

Un scanner d'état se trompe dans deux directions, et elles ne sont pas
symétriques :

- **sur-détecter** une région sautée → une reprise manquée → un sink de trop
  rapporté → **visible** ;
- **entrer à tort** dans une chaîne → tout le reste du fichier sauté → un sink
  **omis en silence**.

La seconde est la dangereuse, et c'est elle qui dicte trois gardes :

1. `//` rend immédiatement « code » — sans quoi un `"` non apparié en prose
   (`// un " seul`) ouvrirait une fausse chaîne ;
2. le littéral de caractère `'"'` est consommé comme un tout, et la borne « un
   seul caractère » garantit qu'un lifetime (`&'a str`) ne peut pas en apparier
   un autre à distance ;
3. l'ouverture d'une chaîne brute exige une **frontière de mot**, sans quoi un
   identifiant finissant par `r` collé à un guillemet serait lu comme `r"`.

Les commentaires `/* */` **imbriqués** ne sont pas modélisés : le premier `*/`
referme. C'est un sur-rapport possible, jamais une sous-détection — donc
acceptable par la même asymétrie.

Et comme un mode de défaillance silencieux ne se corrige pas par la prudence,
il est **mesuré** : une sonde déroule le scanner sur chaque fichier scanné et
exige que l'état final soit « code ». Un fichier Rust valide y finit toujours.
Relevé : `scannés=417 hors_code=0`.

## Ce que la suite pinne, et ce qu'elle ne pinne pas

Trois étages sous le cas N15 de `scripts/test-verify-egress-manifest.sh` :

1. **sept micro-fixtures** du prédicat. Quatre prouvent qu'une région est
   sautée ; **trois prouvent qu'elle ne l'est pas** (guillemet en commentaire,
   littéral de caractère, lifetime) — et ce sont celles-là qui gardent la
   direction dangereuse ;
2. **fermeture lexicale** sur l'arbre réel, qui nomme le fichier *et* l'état
   quand le scanner s'y perd ;
3. **la conséquence bout en bout** : une racine synthétique portant une fixture
   en `"\` suivie d'un `https://api.github.com/…` de prose de test, passée à
   `--audit-cut-holes`.

Et une chose que l'étage 2 ne pinne **pas** : la cardinalité exacte des
reprises. Elle est **imprimée** (`reprises=1`) et assertionnée à `>= 1` avec la
présence du témoin `dashboard.rs`, jamais à `== 1`. Une reprise *réelle* ajoutée
demain par un diff sans rapport doit rester visible sans faire rougir le cas ;
c'est `--audit-cut-holes` qui juge si elle cache un sink. Une assertion
d'égalité aurait transformé un détecteur de conséquence en garde de style.

## Non-vacuité : mesurée, jamais supposée

Un cas de test écrit après le correctif peut être vert par accident. Les sept
micro-fixtures ont donc été lancées contre le moteur **d'avant** avant d'ouvrir
la PR :

```
cont-string                attendu=7     avant=4     après=7     ROUGE AVANT
cont-string-sans-reprise   attendu=None  avant=4     après=None  ROUGE AVANT
raw                        attendu=None  avant=None  après=None
comment-quote              attendu=5     avant=5     après=5
char-quote                 attendu=5     avant=5     après=5
block-comment              attendu=7     avant=4     après=7     ROUGE AVANT
lifetime                   attendu=5     avant=5     après=5
```

Trois rouges avant, zéro après. Les quatre autres passaient déjà : **elles ne
sont pas la preuve du correctif, elles sont la garde contre sa régression**, et
c'est une distinction qui mérite d'être écrite dans la suite plutôt que
devinée.

Le plan de grooming annonçait `char-quote` parmi les trois rouges ; la mesure
dit `cont-string-sans-reprise`. Le compte était bon, un membre ne l'était pas —
d'où l'intérêt de lancer le contrôle au lieu de recopier la prédiction.

De même pour l'étage 3, mesuré dans les deux sens :

```
avant: rc=1  crates/demo/src/lib.rs:19 host 'api.github.com' hors inventaire
après: rc=0  (sortie vide)
```

## Ce qui a été délibérément laissé

- **`production_slices`** a la même cécité dans son détecteur de coupure.
  Mesure : la coupure est **identique sur les 417 fichiers**, avec et sans le
  scanner — zéro divergence. Lui câbler le scanner serait acheter du risque de
  faux positif contre rien, sur la fonction qui alimente l'inventaire que le
  lint *rapporte*. Il n'y a pas un défaut à suivre ; il y a une cécité **sans
  population**.
- **Le palliatif `176bab64` n'est pas reverti.** Les chaînes brutes sont la
  meilleure forme pour une fixture Rust (aucun échappement), et repasser les
  neuf constantes en chaînes ordinaires prouverait ce que la micro-fixture
  `cont-string` prouve déjà en sept lignes — contre ~27 lignes de churn Rust et
  un `cargo test` dans la fin de pipeline.
- **`audit_cut_holes` ne filtre pas les régions de chaîne** quand il cherche
  des hosts derrière une reprise *réelle*. La seule reprise réelle de l'arbre
  n'en porte aucun : population vide. Le jour où elle en portera, ce sera
  visible par un faux sink rapporté, jamais par un sink silencieux.

## La leçon transférable

Un détecteur dont la prose dit « cette sur-détection ne coûte rien tant que la
population dangereuse est vide » décrit une dette, pas une propriété. Elle est
exigible le jour où la population cesse d'être vide — et ce jour-là, la facture
est payée par l'auteur d'un diff sans rapport, sur un message d'erreur qui
parle d'autre chose que du défaut.

Deux corollaires de méthode :

- Quand la contrainte interdit un mécanisme (ici : compter des accolades),
  vérifier qu'on n'a pas confondu le mécanisme avec le **besoin**. Suivre un
  état de chaîne satisfait le besoin sans rien emprunter au mécanisme refusé.
- Quand un correctif introduit un mode de défaillance **silencieux**, livrer
  avec lui la sonde qui le rend bruyant. Une relecture attentive n'est pas un
  substitut : `hors_code=0` est une mesure, « j'ai fait attention » n'en est
  pas une.

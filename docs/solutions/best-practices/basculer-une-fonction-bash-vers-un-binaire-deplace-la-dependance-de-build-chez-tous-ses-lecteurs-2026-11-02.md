---
title: "Basculer une fonction bash vers un binaire déplace la dépendance de build chez TOUS ses lecteurs, tests Rust compris"
date: 2026-11-02
category: best-practices
module: skills/bundled/_shared/dispatch-lib.sh + crates/mika-cli + crates/mika-agent/tests
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - "Une fonction de dispatch-lib.sh cesse de décider et appelle une sous-commande `mika` (forme strangler de mika#2194)"
  - "Un test, bash ou Rust, source dispatch-lib.sh dans un shell neuf et appelle la fonction basculée"
  - "Une suite bash résout le binaire `mika` pour l'exercer"
symptoms:
  - "Un test Rust d'intégration rend un mot vide là où il attendait READY, sans erreur"
  - "Une suite bash verdit ou rougit selon le `mika` installé dans ~/.local/bin, pas selon l'arbre"
  - "Une étape de CI déplacée vers un job qui construit le binaire skippe quand même et sort à 0"
resolution_type: test_fix
related:
  - mika#2194
  - docs/solutions/best-practices/une-parite-bash-rust-se-mesure-une-fois-contre-le-bash-pre-bascule.md
---

# Basculer une fonction bash vers un binaire déplace la dépendance de build chez tous ses lecteurs

## Ce qui s'est passé (mika#2194, vol 5, PR #3016)

Le vol 5 a fait de `_parse_disposition` et `_parse_verdict` des enveloppes sur
`mika arch-disposition`. Le plan avait recensé les lecteurs **bash** de ces
fonctions et déplacé `test_parse_disposition.sh` vers le job `Check`, après
`cargo test`. Il n'avait pas vu un lecteur **Rust** :
`crates/mika-agent/tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs`
source `dispatch-lib.sh` dans un `bash -c` neuf et appelle les deux parseurs.

Dans ce bash, `mika` n'était pas sur le `PATH`. L'enveloppe a pris son chemin de
refus, qui rend 0 et un jeton vide par contrat, et le test a lu `""` là où il
attendait `READY`. Aucun message ne nommait la cause.

La revue a trouvé le même défaut, une couche plus haut, dans la résolution
partagée `skills/bundled/_shared/tests/lib-mika-bin.sh` : elle préférait le
`mika` du `PATH` à `target/debug/mika`. Sur le poste, ce `PATH` porte le binaire
déployé, antérieur au vol (il rendait 2 sur `arch-disposition --help`). Les
suites auraient donc mesuré le binaire installé, pas le code sous revue.

## La règle

**Quand une fonction bash délègue à un binaire, recenser ses lecteurs par la
fonction, pas par le langage.** `grep -rn '<fonction>' crates/ skills/ scripts/`
doit précéder le plan. Un test Rust qui source la bibliothèque est un lecteur
comme un autre.

**Un test mesure le binaire de l'arbre, jamais celui du `PATH`.** Ordre de
résolution : `target/debug`, `target/release`, puis le `PATH` en dernier
recours.

**Dans un test Rust d'un crate qui ne possède pas le binaire**, `mika` appartient
à `mika-cli`, qui dépend de `mika-agent`, donc `env!("CARGO_BIN_EXE_mika")`
n'existe pas. Le chercher à côté de `env!("CARGO_BIN_EXE_mika-spirit")`, dans le
même répertoire de cible : le `cargo test` de l'espace de travail le construit
avant de lancer le premier test. Sonder `mika <sous-commande> --help` une fois,
et paniquer avec la commande de remède si le binaire manque ou s'il est périmé
(`cargo test -p mika-agent` seul ne le reconstruit pas). Ne jamais laisser le
test lire un mot vide.

**Un SKIP faute de binaire est un ÉCHEC en CI.** Déplacer une étape vers le job
qui construit ne suffit pas : si la porte skippe et sort à 0, elle est verte sur
rien le jour où le build cesse de produire le binaire. `mika_bin_or_skip` compte
désormais un FAIL quand `CI` ou `GITHUB_ACTIONS` est posé.

## Pourquoi c'est silencieux

Le contrat des enveloppes (retour 0 sur tous les chemins, refus par fichier) est
juste pour la production : les appelants capturent sous `set -e`, et l'UNPARSED
est leur chemin de reprise. Ce même contrat rend un moteur absent
indiscernable d'une réponse sans disposition pour tout lecteur qui ne lit que
stdout. Le motif nommé n'existe que dans le fichier de refus et dans le
`RESULT` du callback, que les tests ne regardaient pas. D'où le test
comportemental ajouté au vol 5 : un `mika` simulé par une fonction du shell, aux
deux polarités (moteur en échec, réponse saine), plus l'annexe au `RESULT`.

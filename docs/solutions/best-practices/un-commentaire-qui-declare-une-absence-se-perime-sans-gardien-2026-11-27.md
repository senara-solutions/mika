---
title: Un commentaire qui déclare l'absence d'un lecteur se périme le jour où le lecteur naît, et rien ne le dit
date: 2026-11-27
last_updated: 2026-11-27
category: best-practices
module: skills/bundled/_shared/dispatch-lib.sh
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - Vous écrivez un commentaire au bord d'un littéral partagé entre deux langages
  - Un commentaire affirme que « rien ne lit » une valeur, ou que « aucun test ne la regarde »
  - Vous livrez un lecteur neuf pour une valeur qu'un commentaire voisin déclarait orpheline
  - Le périmètre de votre ticket vous interdit de toucher le fichier dont vous périmez un commentaire
---

# Un commentaire qui déclare une absence se périme sans gardien

## Le problème

Dans `skills/bundled/_shared/dispatch-lib.sh`, le bloc qui précède les deux
causes de halte du groom affirmait, en majuscules :

```
# AUCUNE CONSTANTE RUST NE LEUR RÉPOND, et c'est mesuré plutôt que supposé : […]
# les quatre scans `mika2641_*` de `canonical_tokens.rs` sont ancrés nommément
# sur `GROOM_HALT_CAUSE_LINE_PREFIX` et `GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE`,
# donc ils ne demandent rien aux valeurs voisines.
```

C'était vrai le 26/11, à l'écriture. Le lendemain, mika#3176 (PR mika#3181)
ajoutait les deux constantes Rust jumelles dans `task_engine/dispatcher.rs`, un
lecteur qui les compare par égalité (`try_report_groom_plan_halt`), et **deux
assertions** dans le scan `mika2641_*` qui exigent l'égalité à l'octet entre la
ligne d'affectation shell et la constante Rust.

Le commentaire disait alors l'inverse du code de dix lignes plus bas — et pas sur
un détail : il **autorisait explicitement** à changer une valeur côté shell sans
toucher au Rust. Un pilote qui l'aurait cru aurait cassé la CI au commit suivant,
avec un message d'erreur contredisant la consigne qu'il venait de lire.

## Pourquoi ça a tenu, et où le résidu a vécu

Le producteur de la péremption **le savait**. Le corps de mika#3181 porte une
section « Résidu connu, nommé plutôt que corrigé en silence » qui désigne ce bloc
par son numéro de ligne. Il ne l'a pas corrigé parce que son ticket interdisait
de toucher `skills/bundled/_shared/`.

Le résidu a donc vécu dans un **corps de PR** — c'est-à-dire dans le seul endroit
qu'aucun pilote ne lit avant d'éditer un fichier. Nommer un résidu dans l'artefact
que personne ne consulte au moment où il compterait revient à ne le nommer que
pour la relecture.

Deux propriétés rendent cette classe silencieuse :

1. **Une affirmation d'absence n'a pas de gardien.** « Deux constantes répondent »
   rougit le jour où elles disparaissent — c'est le scan `mika2641_*` qui le dit.
   « Aucune constante ne répond » ne rougit jamais : aucun test ne peut surveiller
   le moment où un lecteur *naît*.
2. **La péremption se produit dans un autre fichier.** Rien dans le diff de
   mika#3181 ne touche `dispatch-lib.sh`. Le lien entre le commentaire et ce qui
   le rendait vrai n'existait que dans la tête de son auteur.

## Ce qu'il faut faire

**Ne pas écrire « rien ne lit X ». Écrire « voici ce qui lit X aujourd'hui, et
voici le scan qui rougit si tu dévies ».** La forme de présence nomme son
gardien ; la forme d'absence n'en a aucun.

Appliquée au bloc en question, la rédaction devient :

```
# DEUX CONSTANTES RUST LEUR RÉPONDENT DEPUIS mika#3181, et l'égalité est exigée
# À L'OCTET. `task_engine/dispatcher.rs` porte les jumelles […] et son lecteur
# `try_report_groom_plan_halt` les compare par ÉGALITÉ […]
#
# DONC CHANGER UNE VALEUR ICI SANS TOUCHER AU RUST FAIT ROUGIR LA CI : deux des
# assertions de `mika2641_…` (`canonical_tokens.rs`) cherchent les DEUX lignes
# d'affectation ci-dessous OCTET POUR OCTET […] et c'est leur message qui dit
# quoi réaligner.
```

Trois gestes concrets :

- **À l'écriture d'un commentaire de littéral partagé** : nommer le lecteur, le
  fichier qui le porte, et le test qui confronte les deux moitiés. Un lecteur
  nommé est vérifiable d'un `grep` ; une absence ne l'est d'aucun.
- **À la livraison d'un lecteur neuf** : chercher les commentaires qui déclaraient
  l'absence de ce lecteur, dans les deux langages. `grep -rn 'AUCUN\|aucun
  lecteur\|rien ne lit'` sur le périmètre du littéral coûte un tour.
- **Quand le périmètre interdit la correction** : ouvrir le ticket qui la portera,
  et le lier depuis le commentaire périmé — pas seulement depuis le corps de la
  PR. Un ticket est un gardien ; un corps de PR mergée n'en est pas un.

## Le cas où la forme de présence se périme aussi

Elle se périme si le lecteur est **supprimé**. La différence est tout l'intérêt :
cette suppression fait rougir `cargo test -p mika-agent --lib mika2641`, qui nomme
la constante absente dans son message. La péremption devient un échec de test au
lieu d'un commentaire muet.

## Liens

- mika#3182 — la correction du bloc (ce document)
- mika#3176 / PR mika#3181 — le lecteur Rust qui a périmé l'affirmation
- mika#3118 (vol 2, PR mika#3175) — l'écriture d'origine, vraie à sa date
- mika#2205 — « deux valeurs et jamais une », le paragraphe voisin conservé

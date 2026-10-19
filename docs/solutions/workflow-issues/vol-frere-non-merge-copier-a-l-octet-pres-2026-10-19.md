---
module: mika-agent
date: 2026-10-19
problem_type: workflow_issue
component: development_workflow
severity: low
applies_when:
  - "Un vol consomme un module qu'un vol frère pose, et le frère n'est pas encore mergé"
  - "Deux PR parallèles touchent les mêmes fichiers (lib.rs, prompt.rs, agent_loop, eval.rs)"
  - "Une PR sauvée doit être rebasée sur un main qui a reçu le frère entre-temps"
tags:
  - rebase
  - parallel-tickets
  - sibling-dependency
  - read-receipt
  - compaction-handsoff
related_components:
  - read_receipt
  - compaction_handsoff
  - model_transmission
  - PromptContext
---

# Un vol frère pas encore mergé : copier son module à l'octet près, pas le réécrire

## Ce qui s'est passé

mika#2805 (vol 2 du handsoff de compaction) et mika#2807 (transmission entrante)
avaient besoin de la même pièce : un reçu de lecture colocalisé
(`crate::read_receipt`). Le plan de mika#2807 s'en déclarait le **poseur**, et
celui de mika#2805 le **consommateur**.

Au premier tour du pilote de mika#2805, `read_receipt.rs` n'était sur `main`
nulle part. Le plan avait prévu ce cas (sa D1) : copier le module **verbatim**
depuis la branche du frère (`git show origin/design/2807/…:crates/mika-agent/src/read_receipt.rs`),
avec ses tests et sa ligne `pub mod read_receipt;`, au lieu de le réécrire.

mika#2824 (le frère) a atterri à 13:47Z. La PR de mika#2805 (#2828) est passée
DIRTY sur six fichiers communs. Au rebase :

- `read_receipt.rs` et `lib.rs` : **aucun conflit**. Le contenu était identique
  octet pour octet, donc git a vidé ces deux fichiers du commit rebasé. Il ne
  reste qu'un seul module, celui de main.
- `prompt.rs` : **un seul conflit**, sur les commentaires de doc de deux champs
  voisins de `PromptContext`. Le frère avait changé le type de
  `pending_transmission` (`PendingTransmission` → `Transmission`, enum à deux
  bras) et réécrit sa doc. Le vol 2 avait réécrit la doc de
  `compaction_imminent` juste en dessous. Résolution : la moitié du frère pour
  `pending_transmission`, celle du vol 2 pour `compaction_imminent`, sans choix
  de fond à faire.
- `agent_loop/mod.rs`, `eval.rs`, `CLAUDE.md` : fusion automatique.

Les trois rituels (transmission entrante, cérémonie vol 1, relecture vol 2)
coexistent sans doublon. Les suites eval des trois tickets sont vertes.

## La leçon

**Quand un vol consomme un module qu'un frère non mergé pose, la copie exacte
du module du frère est ce qui rend le rebase gratuit.** Une réécriture, même
équivalente, aurait produit un conflit add/add sur tout le fichier, et il aurait
fallu choisir entre deux implémentations du même reçu. Or deux marques de
relecture, c'est deux endroits où « déjà lu » peut diverger.

La copie à l'octet près sert donc deux fois. Pendant que les deux PR sont
ouvertes, il n'y a qu'**une** implémentation. Au rebase, git **reconnaît** que
c'est la même, et le commit se vide tout seul de ce qu'il avait emprunté.

## Comment l'appliquer

1. Au groom, quand un frère pose une pièce, écrire dans le plan la précondition
   « le frère est mergé » **et** la contingence : `git show
   origin/<branche-du-frère>:<chemin>` verbatim, tests et ligne `mod` compris.
2. Au travail, ne rien toucher au module emprunté, même pas un commentaire ni
   le formatage. Toute retouche devient un conflit au rebase.
3. Si le frère change le module **après** la copie, recopier : le rebase
   absorbe encore. Diverger n'est jamais la bonne réponse.
4. Au rebase, vérifier que le fichier emprunté a disparu du diff
   (`git diff origin/main --stat`). S'il y apparaît encore, c'est que les deux
   copies ont divergé, et il faut garder la version mergée.

Le seul conflit restant est celui des voisins textuels, ici deux commentaires
de doc adjacents dans une struct que les deux vols étendent. Il se résout
champ par champ : chaque vol garde la propriété de sa ligne.

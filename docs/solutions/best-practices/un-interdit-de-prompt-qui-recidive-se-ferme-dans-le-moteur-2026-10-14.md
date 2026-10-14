---
title: Un interdit de prompt qui récidive se ferme dans le moteur
date: 2026-10-14
category: best-practices
module: mika-agent/skills
problem_type: best_practice
component: qa-review
severity: high
applies_when:
  - Un prompt interdit explicitement un comportement et le comportement revient
  - Le correctif précédent de la même classe était une édition de prompt
  - Un critère d'acceptation parle de ce qui est « publié » ou « envoyé »
tags:
  - qa-review
  - run_gh
  - gardes-structurelles
  - verdict
  - mika-2687
---

# Un interdit de prompt qui récidive se ferme dans le moteur

## Contexte

Sur mika#2685 — un bump de lockfile `quinn-proto`, une exception `deny.toml` et
une doc dans `docs/solutions/` — mika-qa a rendu `VERDICT: block[pipeline]`
**deux fois sur la même tête** `0a53b5f0` (2026-10-13 11:54Z et 17:03Z) pendant
que `scripts/verify-pipeline.sh` sortait vert les deux fois. Les deux revues
étaient sans section `PIPELINE:` et sans ligne `NEGATIVE-TEST:`, et énonçaient
des règles que la garde ne porte pas (« plan manquant dans `docs/plans/` », puis
« label `pipeline-exempt` avec des fichiers non-docs »).

Le prompt déployé l'interdisait déjà, nommément, à deux endroits : Step 2
(« Do NOT re-evaluate, second-guess, or supplement any rule a guard already
carries. The plan-doc-presence check […] [is] gone from this step. ») et Step 2E
(toute décision portant une section `PIPELINE:` citant chaque garde verbatim).
Le PR a été débloqué en ajoutant un plan dans `docs/plans/` : **on a satisfait
la lecture erronée, pas corrigé la cause.**

## La récidive est la mesure, pas l'anecdote

Deux tickets de la même classe étaient déjà fermés par une édition de prompt :

- **mika#2172** — le prompt réimplémentait les gardes au lieu de les exécuter ;
  correctif : retirer les règles du prompt.
- **mika#2519** — `block[pipeline]` sur des bumps Cargo-only build-verts ;
  correctif : une interdiction de plus dans le prompt (et, côté moteur, la seule
  voie dependabot).

mika#2687 est la **troisième occurrence**, et elle survient *après* que le
prompt ait retiré la règle que le modèle paraphrase. C'est ce qui disqualifie un
quatrième tour de prompt : le défaut n'est pas « le prompt ne dit pas la règle »,
c'est « le modèle produit une règle que personne ne lui a donnée ». Un prompt
peut ajouter une contrainte ; il ne peut pas garantir l'absence d'une invention.

**La règle praticable :** un interdit de prompt qui revient une deuxième fois,
sur une population mesurée, n'est plus un problème de formulation. Le compteur à
tenir est le nombre de tickets fermés de la même classe, pas la qualité de la
phrase. Au deuxième, on cherche la porte dans le moteur ; au troisième, on ne
discute plus.

## Le critère d'acceptation désigne la porte

Deux portes existaient pour mika#2687 :

1. **`verdict_handler`** — rétrograder un `block[pipeline]` non fondé en
   `hold[review]` à la réception du webhook `pull_request_review.submitted`.
2. **La chaîne pré-subprocess de `run_gh`** — refuser l'appel `gh pr review`
   dont le corps porte un `block[pipeline]` qui ne cite aucune garde en échec.

Le critère dit : *« un verdict `block[pipeline]` sans section `PIPELINE:` citant
une garde à exit non-zéro **n'est pas publié** comme bloquant »*. La porte 1
agit **après** la publication — la revue est sur GitHub, lue par l'humain et par
le dispatcher, et le handler ne corrige qu'une conséquence. Elle ne satisfait pas
le critère ; elle le contourne.

C'est la même lecture que mika#1646 et mika#2237 avaient déjà faite, et elle se
résume en une question : **le défaut est-il une phrase ou un appel ?** Un garde
EndTurn relit du texte et re-sollicite — la bonne forme quand le défaut est une
phrase. Quand le défaut est l'appel, tout ce qui s'exécute après l'appel
annote un fait accompli. Un verbe dans le critère (« publié », « envoyé »,
« fermé », « mergé ») est l'indice : il nomme l'instant après lequel il est trop
tard.

## Ce que le moteur doit payer en plus du prédicat

Un prédicat juste n'est pas une garde. Trois coûts se paient à chaque lien de
cette chaîne, et chacun a déjà coûté un ticket quand il a été omis :

- **La position dans la chaîne est une décision, pas un détail de style.** Le
  lien mika#2687 est le dernier qui ne lit **ni le réseau ni la DB** : il décide
  sur le corps seul, donc il passe avant les deux liens réseau (mika#2455,
  mika#2519). Conséquence observable : sur une PR dependabot portant un
  `block[pipeline]` sans section, c'est lui qui répond, et le message d'erreur
  que voit l'agent n'est pas celui de mika#2519.
- **Le câblage se teste séparément du prédicat.** Sans un test sur le chemin
  `run_gh` réel, *un prédicat qui décide bien reste compatible avec une garde que
  personne n'appelle*. Et ce test-là paie la géométrie : notre lien est en aval
  du `debug_assert!` de dedup mika#821, donc un test `run_gh` sur un `pr review`
  doit fournir `ctx.pr_reviews_posted` — une map **vide**, car la peupler ferait
  répondre mika#821 avant nous. Un voisin plus en amont (mika#2667) n'a pas ce
  coût ; la position se lit, elle ne se suppose pas.
- **La ligne d'audit s'écrit sur l'autorisation aussi.** Le contrôle positif de
  mika#2205 : une table vide dit que le lien n'est pas atteint, pas qu'il ne
  refuse rien. Sans la ligne `allowed`, « zéro refus » est indiscernable de
  « garde morte ».

## Le sens de l'échec se choisit par l'arithmétique des coûts

La citation d'échec est reconnue par une disjonction volontairement large
(`exit <non nul>`, `REJECT:`, `Verification FAILED`), et la borne est écrite :
un corps dont la seule mention d'échec est un `exit 1` dans la prose d'une garde
par ailleurs verte **passera**.

C'est délibéré, et le raisonnement est le même que mika#2455 et mika#2519 : un
faux positif du refus rend une revue légitime **impossible à poster** — le tour
boucle et meurt — tandis qu'un faux négatif est le coût déjà payé aujourd'hui.
Une garde dont le faux positif est plus cher que le défaut qu'elle corrige doit
être large, et doit **nommer** sa largeur. Le resserrement a un nom et un ticket
de suivi (lire la preuve d'exécution en DB, sur le patron de mika#2565), pas une
intention.

## À retenir

- Deux tickets fermés sur la même classe par édition de prompt : le troisième se
  ferme dans le moteur. Le compteur, pas la phrase.
- Le verbe du critère d'acceptation désigne la porte. « N'est pas publié » exclut
  tout ce qui s'exécute après la publication.
- Un prédicat juste + un lien que personne n'appelle = une garde décorative. Le
  câblage est un Layer de test à part entière, et il révèle les contraintes de
  position (ici, le `debug_assert!` de mika#821).
- Une garde fail-open écrit sa borne. Une garde fail-closed écrit pourquoi le
  faux positif est moins cher.

## Références

- Ticket : `senara-solutions/mika#2687` (vol 1 ; vol 2 `Tracked in:` le même ticket)
- Récidive : mika#2172, mika#2519 (fermés par le prompt)
- Doctrine agent : `docs/agent-internals/tools.md` § *Un `block[pipeline]` cite une garde en échec*
- Surfaces opérateur, sondes et STOP : `docs/operator/environment-variables.md` § *Un `block[pipeline]` cite une garde en échec*
- Voisins de chaîne : mika#1646, mika#2237, mika#2455, mika#2519, mika#2667
- Plan : `docs/plans/2026-10-14-001-fix-2687-block-pipeline-grounding-plan.md`

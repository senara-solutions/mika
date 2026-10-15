---
title: Une preuve d'exécution se lit dans le ledger, pas dans la prose
date: 2026-10-15
category: best-practices
module: mika-agent/evidence
problem_type: best_practice
component: qa-review
severity: high
applies_when:
  - Une garde structurelle juge un verdict sur ce que son corps déclare
  - Le modèle qui écrit ce corps est celui dont on veut vérifier le travail
  - Le fait à vérifier est « cet outil a tourné », pas « ce texte est bien formé »
  - Un faux positif de la garde bloquerait un travail légitime
tags:
  - qa-review
  - run_gh
  - gardes-structurelles
  - ledger-doutils
  - abstention
  - mika-2687
---

# Une preuve d'exécution se lit dans le ledger, pas dans la prose

## Contexte

Le vol 1 de mika#2687 a fermé dans le moteur un interdit que le prompt portait
déjà — voir
[`un-interdit-de-prompt-qui-recidive-se-ferme-dans-le-moteur-2026-10-14.md`](un-interdit-de-prompt-qui-recidive-se-ferme-dans-le-moteur-2026-10-14.md).
Il refusait, avant tout sous-processus, un `pr review` portant
`VERDICT: block[pipeline]` dans deux cas : pas de section `PIPELINE:` dans le
corps, ou une section qui ne cite aucune garde sortie en non-zéro.

Les deux prédicats lisent de la **prose**. C'est ce qui les rend insuffisants, et
la raison tient en une phrase : **le modèle dont on vérifie le travail est celui
qui écrit le texte sur lequel on le vérifie.** Un modèle capable de paraphraser
une règle de plan-doc que le prompt avait retirée est tout aussi capable
d'écrire `scripts/verify-pipeline.sh → exit 1` sous une ligne `PIPELINE:`. La
garde aurait alors été franchie par un corps irréprochable, et le défaut
mesuré sur mika#2685 serait revenu avec une étape de plus.

## La leçon centrale

**Quand le fait à établir est « cet outil a tourné », il n'existe aucune
formulation du corps qui l'établisse.** Il faut un terme que le modèle
n'écrit pas : un fait porté par le moteur.

Ici c'est le ledger d'outils — les lignes `tool_calls` de la session, filtrées
sur celles dont l'`input` nomme `scripts/verify-pipeline.sh` ou
`scripts/plan-doc-check.sh`. Le patron de lecture existait déjà
(`find_recent_tool_invocation`, motif B3 de mika#2565). Ce qui change, c'est
qu'il n'est plus consulté pour *enrichir* un jugement mais pour porter un terme
que la prose ne peut pas simuler.

Corollaire pratique : le prédicat de corps reste utile. Il est **gratuit** et
attrape la forme majoritaire. L'ordre d'évaluation le dit — section, puis
citation, puis lecture en base, cette dernière en dernier parce qu'elle est la
seule qui coûte.

## Discriminer par ce que le ledger contient, pas par ce que la session est

Le faux positif à craindre est réel : une revue reprise sur un callback de build
tourne sous un `session_id` **tout neuf** (`create_session_with_parent` dans le
dispatcher), donc le `run_shell` de la session d'origine lui est invisible. La
garde l'accuserait de n'avoir jamais exécuté de garde.

La tentation est de détecter cette situation — renifler un préfixe de session,
tester `is_callback_turn`. C'est fragile et ça se périme. **La bonne discrimination
ne porte pas sur la nature de la session mais sur ce que le ledger contient :**

| ce que le ledger dit | ce qu'on en sait | effet |
|---|---|---|
| un appel shell nomme une garde | elle a été invoquée | la preuve est acquise |
| des appels shell existent, aucun ne nomme de garde | **le ledger est vivant et dit non** | refus |
| zéro appel shell | ambigu : store d'outils désarmé, **ou** session reprise | abstention |
| la lecture a échoué | rien | abstention |

Les deux dernières lignes sont du **silence, pas une accusation**, et elles
tombent sous une même cause nommée chacune (`no_shell_call_recorded`,
`tool_ledger_unreadable`). La troisième est sans ambiguïté : des lignes existent,
aucune ne nomme de garde — le ledger a répondu.

Effet de bord utile : cette forme ferme aussi le trou latent que le motif B3
porte encore chez lui (`Ok(rows) => !rows.is_empty()` y confond « le store est
désarmé » avec « rien n'a tourné »).

## Une abstention s'arme après avoir été comptée

L'abstention est **partielle** — elle neutralise un terme et laisse les deux
prédicats de corps décider — et elle est **dite** : un log
`pipeline_grounding_abstained` portant la cause, et une ligne d'audit
`after_value = 'abstained'`. Sans ça, « la garde s'est abstenue » et « la garde
a laissé passer » rendent les mêmes octets (classe mika#2205).

La règle qui en découle, et qui est écrite dans la halte de la sonde opérateur :
**on compte la population d'abstention avant de transformer l'abstention en
refus.** Si elle est majoritaire, le travail à faire est de rendre la session
d'origine lisible depuis la session reprise — le `parent_session_id` existe en
base — et non de resserrer le prédicat sur une population qu'on n'a pas mesurée.

Même arithmétique fail-open que mika#2455 et mika#2519 : un faux positif rend
une revue légitime impostable **et** laisse une PR cassée franchir la porte de
merge ; un faux négatif laisse le coût d'aujourd'hui, déjà payé.

## Deux leçons de bordure, chacune payée comptant

**Un nom de méthode qui décrit un cas d'usage mentira au deuxième appelant.**
`find_recent_build_invocation` portait déjà `tool_name` en paramètre : la
signature était générique, le nom non. Au nouveau site d'appel (`run_shell`) le
nom aurait énoncé un fait faux, la classe des « deux vérités » que ce dépôt
garde. Renommé en `find_recent_tool_invocation` dans le même commit — **c'est la
signature qui disait vrai, pas le nom.**

**Une porte de taille qui annonce sa propre remédiation transforme une phrase en
décision de découpage.** Le volet d'alignement du prompt (annoncer à Step 2C/2E
le refus que le moteur applique) a été écrit, vérifié vert, puis **retiré** :
`qa-review/system_prompt.md` est à 77 259 octets pour une porte à 77 337, et son
`skill.toml` déclare depuis mika#2639 que les deux premières remédiations sont
épuisées et que *la prochaine addition shard*. Le shard est un vol à part, avec
son propre risque (mika#2355 : un handler dont personne ne dépend est injoignable
et muet). Ce qu'il faut en retenir : **le commentaire d'un `skill.toml` qui
nomme la remédiation suivante est une instruction, pas une plainte** — la
contourner pour 512 octets aurait reporté la même décision sur le vol suivant,
en moins visible.

## Ce que ça ne ferme pas

- **L'invocation, pas la complétion.** Le prédicat lit l'`input` de l'appel, pas
  sa sortie. Exiger aussi un jeton `GUARD: ` dans la sortie a été envisagé et
  écarté : ça ajoute un faux négatif (sortie tronquée, outil en timeout) du
  mauvais côté de l'arithmétique.
- **La session d'origine d'une revue reprise.** La lecture est scopée au
  `session_id` courant. La remontée par `parent_session_id` est le travail que la
  halte de la sonde conditionne.
- **Les autres classes de `block[…]`.** `block[ac]` traverse sans être lu : son
  fondement est l'énumération par AC, une autre famille.

## Où ça vit

- Prédicat pur et sa surface publique : `crates/mika-agent/src/evidence/guards.rs`
  (`classify_pipeline_block_grounding`, `shell_invocation_runs_pipeline_guard`,
  `PipelineGroundingAbstention`)
- Application et lecture en base :
  `crates/mika-agent/src/skills/builtin_handlers.rs`
  (`observe_pipeline_guard_execution`, `audit_pipeline_grounding`)
- Surfaces opérateur, table des quatre états, sondes S1–S4 et leurs haltes :
  `docs/operator/environment-variables.md` § *Un `block[pipeline]` cite une garde
  en échec, ou il ne part pas*

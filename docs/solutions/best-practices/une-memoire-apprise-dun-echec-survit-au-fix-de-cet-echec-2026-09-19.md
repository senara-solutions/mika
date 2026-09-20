---
title: Une mémoire apprise d'un échec survit au fix de cet échec — vérifiez la tentative, pas l'intention
date: 2026-09-19
last_updated: 2026-09-19
category: best-practices
module: mika-agent/skills/builtin_handlers
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Un fix d'identité, de permission ou de quota restaure une capacité qui était refusée
  - Un agent porte, dans sa core memory ou ses facts, un contournement appris de cette époque
  - Une instruction de skill explicite (« si X alors fais Y ») est contredite par une mémoire
  - Vous envisagez de comparer sémantiquement « mémoire » et « instruction de skill »
  - Un log montre qu'une action n'a pas eu lieu sans montrer si elle a été tentée
---

# Une mémoire apprise d'un échec survit au fix de cet échec

## Le fait

Le 2026-09-08, mika#2218 a rendu `gh pr review --approve` de nouveau possible :
l'identité de revue est devenue `mika-platform-qa`, distincte de l'auteur des
PR. Déployé, redémarré, binaire `584d98aa`.

Sur la **première** revue post-déploiement — mika#2236, corps
`VERDICT: pass ✅` — mika-qa a posté en `--comment`. Pas « a tenté `--approve`
et échoué » : l'argv enregistré à 08:03:34Z est
`["pr","review","2236","--comment",…]`, **zéro tentative**.

Le prompt du skill mappait pourtant `pass → --approve`
(`qa-review/system_prompt.md`). La déviation venait de la mémoire de l'agent :
la `core_memory` (`workflows`, `current_priorities`) et un fact du 2026-09-07
encodaient la contrainte de l'ère pré-fix — les 137 refus d'auto-approbation.
C2, le premier merge autonome, est resté à moitié cassé jusqu'à une correction
manuelle de la mémoire de mika-qa.

## La classe

**Un fix qui restaure une capacité ne suffit pas si un agent porte, dans sa
mémoire apprise, la contrainte de l'ère pré-fix.** La mémoire défensive survit
au fix et continue d'appliquer l'ancien contournement — sans erreur visible,
puisque l'agent ne tente même plus la voie désormais ouverte.

Deux propriétés rendent la classe coûteuse :

1. **Le prompt dit la bonne chose, et perd quand même.** La pondération de
   contexte (`core memory > active skill context`) autorise une mémoire à
   l'emporter sur un mapping de skill explicite, silencieusement, sans qu'aucun
   conflit soit perçu. Corriger le prompt ne corrige rien, puisqu'il était déjà
   juste.
2. **L'échec est muet.** Rien dans le journal ne distingue « l'agent a tenté la
   voie et elle a échoué » de « l'agent n'a pas tenté la voie ». Il a fallu lire
   l'argv à la main pour établir le second.

## Ce qui a été mesuré ensuite, et qui déplace le remède

La même `mika-platform-qa`, le même jour, a posté **trois revues `APPROVED`**
(08:25:08Z, 08:47:25Z, 09:29:39Z). « Zéro tentative » est donc vrai **de ce
tour** et faux de la journée : la mémoire défensive n'était pas un empêchement
stable, c'était un arbitrage qui gagnait **par intermittence**.

C'est décisif pour le choix du remède. Un correctif côté mémoire — taguer les
mémoires défensives, poser un fact d'invalidation daté — suppose un **état
persistant à corriger**. Ce qui est mesuré est un arbitrage non déterministe,
tour par tour. Une garde qui lit l'argv mord exactement sur les tours où la
mémoire gagne et se tait sur les autres : la granularité du remède épouse celle
du défaut.

## La règle générale

> Quand une instruction de skill se manifeste dans un argv, ne cherchez pas à
> détecter la contradiction dans la tête du modèle : vérifiez-la là où elle
> devient un fait. Et n'interdisez pas la dégradation — **exigez la tentative**.
> « Pas tenté » et « tenté et refusé » se ressemblent dans un log et ne se
> ressemblent dans aucun diagnostic.

## Trois remèdes écartés, avec leur raison

**(a) Un fact d'invalidation daté** (« depuis le 2026-09-08, `--approve`
fonctionne »). Écarté par Vincent lui-même : le skill dit **déjà** la bonne
chose. Il n'y a rien à invalider pour que l'instruction soit correcte — le
problème n'est pas une connaissance fausse, c'est une hiérarchie.

**(b) Le tagging des mémoires défensives.** Quatre raisons. (i) La
classification « ceci est une mémoire apprise d'un échec » serait faite par le
modèle, c'est-à-dire de l'application par prompt sur la couche même qui vient de
faillir. (ii) **Un tag n'ôte rien** : la mémoire taguée reste dans le prompt et
reste lue, donc le tag ne ferme rien sans un changement de pondération qui est,
lui, encore du prompt. (iii) Le rayon de souffle (forme de `store_fact`, blocs
de core memory, tous les lecteurs) est large pour un défaut dont la
manifestation se ferme structurellement en un point. (iv) La re-mesure
ci-dessus : le remède doit décider tour par tour, ce qu'un tag ne fait pas.
**Condition de réouverture** — si le compteur de refus plafonne au lieu de
décroître, ou si un second mapping opérationnel est mesuré occulté, la question
revient avec des données.

**(c) Un détecteur sémantique de contradiction mémoire ↔ skill.** C'est la
lecture naturelle du fix que mika-qa proposait elle-même (« détection forcée du
conflit au point de décision »). Elle demanderait un juge sémantique et un
lexique — exactement la couche qui vient de faillir. Or le conflit **se
manifeste** sous une forme entièrement structurelle : un corps portant
`VERDICT: pass` passé sous `--comment`. Le point de décision qui compte n'est
pas celui où le modèle pèse sa mémoire, c'est celui où le moteur voit l'argv.

## Le remède livré (mika#2237)

**Une garde pré-sous-processus**, `validate_pr_review_flag_coherence`
(`crates/mika-agent/src/skills/builtin_handlers.rs`), posée dans la chaîne de
`run_gh` juste après le contrôle de profondeur de mika#275. Elle refuse un
`gh pr review` dont le flag contredit le verdict de son propre corps, **dans les
deux directions** : le défaut mesuré est une dégradation (conservatrice), son
inverse — un `block[…]` posté en `--approve` — ferait merger une PR bloquée.

Quatre points de conception, chacun contre-intuitif :

**Le gating est le CORPS, jamais le skill actif.** La garde voisine
(`validate_review_depth_present`) se gate sur `required_tool_arg_suffixes`, un
proxy pour « qa-review est chargé ». Celle-ci se gate sur son propre sujet : un
corps dont `parse_verdict` rend `Missing` n'est pas son affaire (fail-open), un
corps qui porte un verdict classifié l'est. Une revue humaine sans ligne
`VERDICT:` n'est jamais bloquée, et la garde ne disparaît pas en silence le jour
où qa-review réorganise son manifeste.

**Le mapping a un lecteur unique et il est dérivé de l'enum `Verdict`.** Pas
une table recopiée depuis le texte du skill : la vérité est l'enum que
`verdict_handler` consomme déjà (il refuse de merger un `pass` dont
`state != approved` — le même contrat, lu à l'autre bout). Ce n'est pas une
préférence de style mais **la condition de correction du mécanisme** : une garde
qui aurait recopié un `contains("VERDICT: pass")` aurait fail-open **précisément
sur `VERDICT: pass ✅`**, la forme littérale de l'incident fondateur — muette là
où elle devait mordre, et indistinguable d'une garde qui fonctionne. Passer par
`parse_verdict` hérite gratuitement de l'emphase markdown (mika#1828) et du
repli décoration (mika#2239) — **et de leurs bornes**, qui sont épinglées comme
décisions (`VERDICT: pass — but see findings` reste `Missing` ; la décoration de
tête est hors périmètre et n'est **pas** contournée dans la garde, ce qui
créerait le second lecteur).

**L'échappatoire est le cœur du fix, pas son adoucissement.** Une garde qui
refuserait *toujours* `--comment` sur `pass` transformerait une contrainte
réelle en impossibilité de poster la revue : le tour boucle et meurt. Donc
`--comment` sur `pass` est autorisé **si et seulement si** un `--approve` sur la
même PR a été tenté dans le même `trace_id` et a échoué. C'est aussi ce qui rend
les deux populations comptables séparément — refus = « allait dégrader sans
avoir tenté » (mémoire périmée), échappatoire = « tenté et refusé » (contrainte
réelle).

**L'échappatoire est fail-OPEN, à l'inverse de sa sœur mika#1646.** Le terme que
l'historique porte est *l'absence de tentative*, et un terme qu'on ne peut pas
lire n'est jamais un terme satisfait (mika#2277). Refuser sur historique
illisible produirait la boucle `--approve` échoue → `--comment` refusé →
`--approve` échoue… Coût nommé : avec `MIKA_STORE_TOOL_CALLS=false` l'historique
est vide, donc la garde est **inerte** sur cette direction ; rendu visible par le
grep d'abstention.

**Le refus laisse une sortie qui n'est pas un mensonge.** Un refus qui ne
nommerait que « poste en `--approve` » pousserait un modèle tenu par sa mémoire à
réécrire son **verdict** (`pass → hold[review]`) plutôt que son flag — le même
défaut sous un autre nom, et la garde ne peut pas savoir quel verdict est juste.
Le corps du refus nomme donc les deux voies correctes, et se termine par la
phrase qui **est** le fix (c) rendu opérationnel : *« a failure recorded in your
memory is not evidence about THIS pull request »*. Ce contournement reste ouvert
et est surveillé, pas fermé.

## Le trou d'observabilité symétrique, refermé en même temps

`verdict_handler.rs`, branche `Verdict::Pass` : un `VERDICT: pass` arrivant sous
`state != "approved"` était **silencieusement** renvoyé au LLM — aucun WARN,
aucune ligne d'audit, aucun compteur. Le pipeline autonome s'arrêtait et rien ne
le disait. mika#2236 a vécu exactement là.

L'asymétrie datait : cent cinquante lignes plus bas, mika#2239 avait nommé le
**miroir** (`verdict_approved_but_unclassified` — GitHub dit APPROVED, le
verdict ne classifie pas), motivé en commentaire par « so the monitor can grep
it ». La moitié complémentaire est restée sans nom, et c'est ce qui a coûté onze
jours d'invisibilité.

`verdict_pass_without_approval` la nomme. **Le comportement est inchangé** : on
ne se met pas à merger sur un `commented`. Ce qui est ajouté est de
l'attribution, pas une décision. **SOLE WRITER**, épinglé par un scan de source
— deux écrivains rendraient les deux populations indistinguables, ce qui est la
discipline que mika#2239 s'est appliquée à lui-même.

## Un défaut découvert en câblant le test, et qui rendait l'échappatoire décorative

Le dedup de session de mika#821 enregistrait sa clé dès que `gh` avait été lancé
sans erreur d'infrastructure. Or `spawn_and_collect` rend `ToolOutput::success`
**même sur sortie non-zéro** (contenu préfixé `Exit code: N`). Donc un
`pr review --approve` **refusé par GitHub** consommait le droit de poster, et le
`--comment` de repli était rejeté en `duplicate_pr_review`.

Autrement dit : « tente `--approve`, et si ça échoue tu peux dégrader » ne
pouvait pas fonctionner, puisque la tentative échouée consommait le droit de
poster. Le prédicat est désormais celui que `tool_execution::dispatch` calcule
déjà pour tous les outils (`!is_error && !has_non_zero_exit_prefix`), donc le
registre de dedup et la colonne `tool_calls.success` s'accordent sur ce qu'est
une revue réussie. Enregistrer une revue que personne n'a postée était faux
indépendamment de ce ticket ; ça n'est devenu porteur qu'ici.

## La moitié comportementale, et ce qu'elle ne remplace pas

La *décision* — une mémoire l'emportant sur un mapping de skill — est arbitrée
par le LLM au runtime et est hors de portée d'un test unitaire. C'est la lecture
juste du commentaire de Vincent sur le ticket. Ce que ce travail change, c'est
que sa *manifestation* est devenue un fait dans l'argv, et un fait dans l'argv,
un test déterministe le tient.

Les deux mailles, donc :

- `tests/eval/test_pr_review_flag_coherence_2237.rs` — chemin de production,
  `MockLlmProvider`, sans réseau : les deux directions, l'échappatoire, et trois
  contrôles négatifs (sans eux, « la garde décide » serait indistinguable de
  « la garde refuse tout »).
- `calibration/roles/mika_qa.rs` — `memory_vs_skill_precedence` (le modèle
  suit-il le skill quand la mémoire dit le contraire ?) et
  `memory_vs_skill_no_verdict_degradation` (ne dégrade-t-il pas son propre
  verdict pour faire coller le flag ?). **Honnêteté sur leur portée** : ce sont
  des gates de swap de modèle (mika#1190), lancés par `make calibrate-mika-qa`
  avec de vraies clés, pas un filet continu — et l'assertion porte sur du texte,
  pas sur un appel d'outil.

## Surfaces opérateur

```bash
grep pr_review_flag_refused                "$MIKA_SPIRIT_LOG_FILE" | jq -c '{verdict, flag_posted, pr}'
grep pr_review_flag_degraded_after_attempt "$MIKA_SPIRIT_LOG_FILE"
grep pr_review_flag_guard_abstained        "$MIKA_SPIRIT_LOG_FILE"
grep verdict_pass_without_approval         "$MIKA_SPIRIT_LOG_FILE"
```

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'pr_review_flag_guard' GROUP BY 1;
```

| signal | régime attendu | lecture |
|---|---|---|
| `pr_review_flag_refused` | **non vide** | chaque ligne est une dégradation que la mémoire poussait encore et que le moteur a arrêtée — c'est la mesure de la rémanence, que rien ne donnait avant |
| `pr_review_flag_degraded_after_attempt` | rare | l'agent a tenté et heurté un mur : contrainte réelle, pas mémoire |
| `pr_review_flag_guard_abstained` | zéro | l'historique `tool_calls` n'est pas lisible ⇒ la garde est inerte sur la direction `pass → comment` |
| `verdict_pass_without_approval` | **zéro** | une revue a contourné la garde |

**Trois haltes.**

1. `verdict_pass_without_approval` non vide après déploiement → **ne pas
   élargir la garde par réflexe** : établir d'abord *quel chemin* a posté (autre
   agent, `run_gh_subprocess`, binaire antérieur — classe mika#2340). Les trois
   remèdes diffèrent.
2. `pr_review_flag_guard_abstained` soutenu → vérifier `MIKA_STORE_TOOL_CALLS`
   **avant** de toucher au prédicat.
3. Des verdicts `pass` qui disparaissent au profit de `hold[review]` sur des PR
   qui auraient dû passer → c'est le contournement nommé plus haut, le modèle
   dégrade son verdict au lieu de son flag. **Ne pas durcir la garde** : elle ne
   peut pas savoir quel verdict est juste. C'est un signal pour le scénario de
   calibration et pour la formulation de la clause de prompt.

Et sur le premier signal : une décroissance vers zéro dit que la mémoire s'est
purgée ; un **plateau** dit qu'elle se ré-écrit, et c'est **là** que le ticket de
suivi sur le tagging s'ouvre — avec un compte plutôt qu'avec une intuition.

## Références

- mika#2237 (ce ticket), mika#2218 (le fix d'identité qui a ouvert `--approve`).
- mika#2239 (le miroir `verdict_approved_but_unclassified` et le repli
  décoration), mika#1828 (tolérance à l'emphase), mika#1821 (la borne du
  commentaire de fin).
- mika#1646 (`validate_destructive_action_grounding` — la garde sœur, fail-closed
  là où celle-ci est fail-open, et pour une raison nommée).
- mika#2158 (deux lecteurs d'un format dérivent), mika#2277 (un signal illisible
  n'est jamais un terme satisfait), mika#1190 (la calibration comme gate de
  swap), `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.

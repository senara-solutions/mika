---
module: skills::executor
tags: [dispatch-gate, deferred-dispatch, operator-lift, review-anchor, groom, mika-2786]
problem_type: logic-error
category: dev-loop
---

# Une levée d'opérateur doit survivre à la file différée

## Le constat

Un gate à la frontière d'outil de dispatch (`validate_dispatch_readiness`) qui se
lève sur « le tour porte une demande explicite de l'opérateur » ne tient que
pour le premier appel. Quand la place de la classe est prise, le dispatch admis
est garé en **lanceur différé** (mika#1011), et son rejeu part d'un tour
silencieux `SilentTrigger::DeferredDispatch` dont l'`originating_message` vaut
toujours `None` (`agent_loop/mod.rs`, « deferred-dispatch retries are
engine-initiated »). Le rejeu relit l'état qui a armé le gate, ne voit plus la
demande, et refuse ce que l'opérateur venait de débloquer. Le seul geste de
reprise que le refus nomme ne passe pas la file.

C'est la revue adverse de mika#2786 qui l'a trouvé, sur le frein des retenues
d'ancrage. Les tests verts n'en disaient rien : aucun ne rejouait un dispatch
levé sans message.

## La règle

**Une levée doit être un état écrit, pas un terme lu sur le tour.** Le frein de
mika#2786 écrit une ligne d'audit `groom_anchor_brake_lifted` au moment où une
demande explicite traverse un frein armé. Il ne compte ensuite que les retenues
postérieures à la dernière levée du ticket. Le rejeu différé lit cette borne,
qu'il ait un message ou non. Une nouvelle série après la levée freine de nouveau,
et la levée devient du même coup observable.

Deux détails qui comptent :

- **Comparaison stricte, à la seconde.** Le callback du dispatch levé naît dans
  le même appel d'outil que la ligne de levée. Avec `>=`, une retenue antérieure
  écrite dans la même seconde annulerait la levée ; avec `>`, on perd au pire le
  compte d'une retenue juste après la levée. Le second défaut est le bon.
- **Les lanceurs différés ne sont pas des verdicts.** Un parent peut empiler une
  dizaine de wrappers « deferred dispatch slot freed ». Lus dans une fenêtre de
  lignes, ils poussent les verdicts hors de la fenêtre ; lus en tête, ils cachent
  le verdict qu'ils suivent. C'est ce qui a laissé repartir un groom après un
  ESCALATE sur mika#2762 (ticket mika#2795). Le lecteur du frein les exclut à la
  source (`label NOT LIKE '%:deferred%'`).

## Où regarder la prochaine fois

Tout gate dont la levée dépend de `originating_message` et qui mord aussi sans
message a la même faille. mika#2545 l'évite par construction, puisqu'il ne mord
que **sans** message. Le prédicat « demandé » de mika#2772 (consentement forcé)
lit lui aussi le message ; ce document n'a pas vérifié son comportement au
rejeu. Avant d'ajouter une levée de ce type, écris le test « levée, puis rejeu
avec `None` ».

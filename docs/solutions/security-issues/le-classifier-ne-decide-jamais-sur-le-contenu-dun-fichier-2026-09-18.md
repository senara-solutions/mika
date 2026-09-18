---
module: permission-policy, claude-pilot, dev-groom, dispatch-lib
tags: [permission-policy, per-spawn, policy-deny, diagnostic-message, routing, rule-id]
problem_type: architecture-pattern
category: security-issues
date: 2026-09-18
ticket: mika#2312
applies_when:
  - "Un `[policy:deny]` a tué une session pilote et il faut lui attribuer une cause"
  - "Un ticket porte sur le CONTENU d'un `system_prompt.md` et on se demande s'il est groomable en sandbox"
  - "On envisage un carve-out de chemin ou de motif dans le classifier de permission"
resolution_type: pattern
---

# Le classifier ne décide jamais sur le contenu d'un fichier — et un `[policy:deny]` nomme une commande, jamais un fichier

## TL;DR

Il n'existe **aucune** protection du contenu des `skills/bundled/*/system_prompt.md`
vis-à-vis du pilote sandboxé, ni voulue ni accidentelle. Le registre de permission ne
*peut pas* lire un fichier : il n'importe aucun module d'accès au système de fichiers,
toutes ses fonctions ont la signature `(argv, cwd) -> bool`, et `is_safe_cat` rend
`True` inconditionnellement. Les tickets dont l'objet est le contenu d'un prompt sont
donc **groomables en sandbox**, et aucune contrainte de routage ne s'applique.

Le marqueur a la forme
`[policy:deny] <Tool>: <detail>[ [<rule-id>]] (terminal|non-terminal)`.
**Le `[<rule-id>]` est ce qui attribue la cause** — et c'est la première chose à lire.
Son **absence** ne sort pas de l'étage déterministe : elle signifie `rule_id=None`,
c'est-à-dire le **refus par défaut** de la policy — aucune règle n'a matché — et là,
élargir l'allow-list est précisément le remède.

---

## Le fait, et ses deux ancrages

### Ancrage structurel — le registre n'a aucun moyen d'ouvrir un fichier

`tools/mika_permission_policy/mika_permission_policy/_binaries.py` n'importe que
`from __future__ import annotations` (ligne 28). `__init__.py` n'ajoute que
`collections.abc.Callable` (ligne 32) et les fonctions du module voisin. Aucun `os`,
`pathlib`, `io`, `glob`, ni `open`. Toutes les fonctions du registre ont la signature
`(argv: list[str], cwd: str) -> bool`, et `is_safe_cat` (ligne 46) est :

```python
def is_safe_cat(argv: list[str], cwd: str) -> bool:
    return True
```

Le docstring du module l'énonce lui-même, dans sa section « What we do NOT check
here » (lignes 24-25) :

> Path containment for filesystem targets — this is a Write/Edit tool concern, not a
> Bash one, and tier1 doesn't check it either.

Le contenu d'un fichier n'est donc ni « autorisé » ni « refusé » : **il n'est pas une
dimension de décision**. C'est vrai par construction, pas par réglage — ce qui rend la
conclusion indépendante du mode armé (`MIKA_PERMISSION_POLICY_MODE`) et de l'état de la
bascule Phase 1/2/3 de mika#1817.

### Ancrage expérimental — quatre mesures depuis un pilote sandboxé

Faites le 2026-09-18, en rôle dev-groom, sous le même classifier que l'incident de
#2295. Reproductibles en quatre commandes.

| # | Commande émise | Verdict |
|---|---|---|
| M1 | `head -5 skills/bundled/mika-arch-groom-ticket/system_prompt.md` | **allow** — contenu lu |
| M2 | `for f in skills/bundled/dev-pilot/skill.toml; do head -2 "$f"; done` | **allow** |
| M3 | `for f in skills/bundled/mika-arch-groom-ticket/system_prompt.md skills/bundled/mika-arch-second-review/system_prompt.md; do echo "=== $f ==="; wc -l "$f"; done` | **allow** |
| M4 | une chaîne `ls … \| head -3` + `git log …` + `env \| grep …` avec `&&` et `\|\|` | **veto** — `policy allow (bash-grep) vetoed — command chains a tier3-dangerous or command-substitution tail onto the allowed prefix` |

**M3 est la mesure décisive** : c'est la commande citée par mika#2312 — mêmes fichiers,
même boucle `for … do … done` — et elle passe. M1 réfute la thèse du contenu ; M3 réfute
aussi la thèse de la forme `for`. M4 est la seule mesure *positive* : ce qui est refusé
ce jour-là est la **chaîne composée**, et le classifier le dit dans son propre message.

**Une tension non résolue, signalée plutôt que comblée.**
`1817-mika-side-plugin-per-binary-safety-functions.md` énonce que `per_spawn.decompose()`
refuse le control flow (`if`, `for`, `while`, …) « at the raw-source level ». M3 est une
boucle `for` et elle passe. Les deux ne peuvent pas être vrais du même chemin de
décision, et **ce document ne tranche pas lequel a tourné le 18 septembre** : le mode
réellement armé (`MIKA_PERMISSION_POLICY_MODE`) n'est pas lisible depuis le sandbox. Une
explication plausible existe — le mode `per_spawn` est un opt-in de Phase 1, et le chemin
classique tier1/tier2 n'a pas cette règle — mais elle n'est pas mesurée, donc elle n'est
pas écrite ici comme un fait. C'est exactement le genre de déduction qui a coûté trois
erreurs à ce ticket. **Un lecteur qui bute sur la phrase de 1817 n'a pas tort de s'y
fier ; il lui manque de savoir quel mode tournait.** Le trancher demande de lire
l'environnement du service, hors sandbox.

Ces mesures s'étendent au 14 septembre : `git log --since=2026-09-01 --
tools/mika_permission_policy/` est vide, donc l'étage déterministe n'a pas changé entre
les deux dates.

**Deux mesures de plus, faites le même jour pendant l'implémentation** (rôle dev-pilot
cette fois, donc la conclusion ne tient pas au rôle) :

| # | Commande émise | Verdict |
|---|---|---|
| M5 | `cd tools/mika_permission_policy && uv run pytest -q` | **deny** — puis `uv run --directory tools/mika_permission_policy pytest -q`, **allow** : même travail, même fichiers, forme non chaînée |
| M6 | `git diff --stat -- <path> ; echo "…$(… \| wc -l)"` | **veto** — `policy allow (bash-git-readonly) vetoed — command chains a tier3-dangerous or command-substitution tail onto the allowed prefix` |

M6 est la reproduction directe de M4 sur un autre binaire, et M5 est la doctrine en
action : la commande refusée a été **réémise autrement** et a abouti, sans qu'aucune
règle soit élargie. C'est ce qu'un pilote doit faire d'un deny, et ce que mika#1410 doit
lui permettre de faire sans mourir.

À noter au passage, parce que l'information est utile et n'est écrite nulle part :
`make test-permission-policy-plugin` est refusé en sandbox — `SAFE_MAKE_TARGETS` ne
contient que `verify-bundled-skills` — alors que la même suite passe par
`uv run --directory tools/mika_permission_policy pytest -q`. Là encore, c'est la
commande qui est refusée, pas ce qu'elle lit.

---

## La règle de lecture d'un `[policy:deny]`

C'est la doctrine utile, et elle tient en quatre points. **Chacun est vérifié dans la
source de `claude-pilot`, pas déduit** — voir l'avertissement de méthode plus bas, qui
existe parce que la première rédaction de cette section s'est trompée exactement là.

1. **Le marqueur nomme l'appel d'outil refusé, jamais le contenu d'un fichier lu.** La
   forme réelle est :

   ```
   [policy:deny] <Tool>: <detail>[ [<rule-id>]] (terminal|non-terminal)
   ```

   `<detail>` est le résumé de l'entrée de l'outil (`_summarize_input`,
   `permissions.py`) : la **commande** pour `Bash`, le **chemin cible** pour
   `Write`/`Edit`/`Read`. Un deny sur un `Write` nomme donc bien un fichier — celui
   qu'on voulait écrire, jamais un fichier dont le contenu aurait été jugé. Le suffixe
   de léthalité `(terminal)`/`(non-terminal)` (cpp#151, `ui.py`) **suit** le tag, donc
   le `rule-id` est le dernier jeton *entre crochets*, pas le dernier jeton.

2. **Lire d'abord le `[<rule-id>]`.** C'est lui qui attribue la cause, et il est déjà
   dans le message que l'opérateur a sous les yeux. Aucune enquête n'est nécessaire
   avant de l'avoir lu.

3. **Ne jamais tronquer la commande en la rapportant.** mika#2312 coupe la commande à
   `do …` et s'arrête avant le crochet : le `…` a emporté exactement l'information qui
   aurait clos l'enquête, et le corps de la boucle avec — or un corps qui chaîne (`|`,
   `&&`) ou substitue (`$(…)`) est justement ce que M4 montre refusé.

4. **Un refus SANS `rule-id` est le refus par défaut de la policy — et il est
   déterministe.** `policy.py` rend `PolicyDecision(…, rule_id=None)` quand aucune règle
   n'a matché ; `permissions.py` passe ce `None` tel quel à `log_policy_deny` ; `ui.py`
   fait `tag = f" [{rule_id}]" if rule_id else ""`, donc la ligne s'affiche sans
   crochets. Le `reason` correspondant, dans `policies/permissions.yaml`, est :

   > `no matching policy rule -- denied by default (production posture; widen rules to
   > allow new tool footprints)`

   **Élargir l'allow-list est donc le remède de cette classe, pas une impasse.**
   Attention : ce `reason` part dans le message de refus rendu à l'agent, pas dans la
   ligne de log — l'opérateur qui lit le journal voit l'absence de tag, pas la phrase
   qui l'explique. C'est ce que ce point existe pour traduire.

   **Le relais `canUseTool` n'est pas la réponse** : il journalise par `log_relay_recv`,
   jamais par `log_policy_deny`, donc il ne produit aucune ligne `[policy:deny]` — et il
   n'est de toute façon atteignable que sous `MIKA_PILOT_POLICY_DISABLED=1` (rollback
   d'urgence). Un refus qui n'émet aucune ligne `[policy:deny]` du tout est le seul
   signal qui pointe hors de l'étage déterministe.

---

## Les deux branches refusées, et le coût de chacune

mika#2312 demandait de trancher entre *voulu* (→ documenter une contrainte de routage)
et *over-block* (→ carve-out lecture seule). **Les deux branches supposent l'existence
d'une règle de contenu. Il n'y en a pas.**

**Branche « oui, c'est voulu » → contrainte de routage.** Elle consacrerait une
protection fantôme et exclurait de la boucle autonome toute une classe de tickets
(« le prompt X a grossi/dérivé ») que M1 et M3 montrent parfaitement groomables en
sandbox. Coût direct et permanent : du travail routé à l'orchestrateur pour toujours,
sur la foi d'une inférence que quatre commandes réfutent.

**Branche « non, over-block » → carve-out lecture seule.** Pire que sans effet. Un
carve-out *de chemin* dans un registre qui décide *par binaire* introduirait la première
dimension « chemin » d'un classifier qui n'en a aucune — précisément celle que le
docstring de `_binaries.py` exclut. On créerait la protection qu'on croyait lever, pour
réparer un défaut inexistant.

**Ce que la prochaine occurrence doit faire à la place :** lire le `rule-id`. S'il nomme
une règle de chaîne ou de substitution, la commande est à réémettre autrement (un `head`
simple plutôt qu'une chaîne composée). S'il est **absent**, c'est le refus par défaut :
aucune règle ne couvrait cet appel, et la question à poser est « cette forme mérite-t-elle
une règle ? » — pas « quel fichier était protégé ? ».

---

## La cause du deny de #2295 est indéterminée, et c'est une conclusion

Les éléments cités par le ticket ne permettent pas de l'établir, et il faut le dire
plutôt que de lui substituer une hypothèse. Trois causes restent compatibles :

1. **Le refus par défaut** — aucune règle ne couvrait la forme émise. C'est la classe la
   plus fréquente, et elle est **déterministe** : voir le point 4 de la règle de lecture.
2. **Le corps tronqué de la boucle** — s'il chaînait ou substituait, c'est M4 qui
   s'applique.
3. **Le veto de chaîne**, dont `_split_compound_command` a un historique documenté de
   faux positifs sur des greps à alternation
   (`2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`, § Gap 1).

Le jugement LLM, lui, **ne figure pas dans cette liste** : il ne produit aucune ligne
`[policy:deny]` et n'est atteignable que sous `MIKA_PILOT_POLICY_DISABLED=1`. Une
rédaction antérieure de ce document le désignait comme l'explication d'un refus sans
`rule-id` ; c'était faux, et c'est la revue de code de mika#2312 qui l'a établi en
citant la source.

**Aucune des trois n'est une règle de contenu.** La conclusion de ce document ne dépend
donc pas de savoir laquelle a mordu.

**Pour trancher, si quelqu'un y revient :** récupérer la ligne complète sur l'hôte, hors
sandbox — `grep -m1 'policy:deny' /var/log/claude-pilot/3587fe25.stderr` — et lire son
`[rule-id]`. Si ce `rule-id` nomme une règle de chemin ou de fichier, ce document est
réfuté et la branche « over-block » redevient ouverte — mais elle se traiterait alors
dans `claude-pilot`, pas dans ce dépôt.

---

## Le piège de méthode, nommé comme tel

**Trois fois de suite, la même erreur, dont deux fois dans ce document.**

1. Le ticket infère « le contenu est protégé » de deux commandes qui diffèrent sur deux
   axes — l'inférence non contrôlée qui l'a ouvert.
2. La première rédaction du plan conclut « c'est la boucle `for` qui est refusée », sur
   la foi d'un document décrivant `decompose()` — M3 la réfute en une commande.
3. La première rédaction de **ce document** affirme qu'un refus sans `rule-id` vient du
   jugement LLM, par déduction d'architecture. La revue de code l'a réfutée en citant
   `policy.py`, `ui.py` et `permissions.yaml` : c'est le refus par défaut, il est
   déterministe, et son propre `reason` prescrit le remède que la phrase déclarait
   inutile. Pire : la session qui écrivait cette phrase avait reçu **quatre fois** le
   refus par défaut, sans `rule-id`, avec le mot « widen » dedans — la mesure était sous
   les yeux de l'auteur pendant qu'il écrivait le contraire.

La leçon n'est donc pas « mesurer plutôt que déduire », qui était déjà écrite ici et n'a
pas suffi. C'est : **une affirmation causale sur un mécanisme se vérifie dans le code de
ce mécanisme, ou ne s'écrit pas.** `claude-pilot` est lisible depuis le dépôt voisin ;
rien n'obligeait à déduire.

C'est la raison d'être du point 2 de la règle de lecture. Le `rule-id` est une **mesure**
que le système émet lui-même ; tout le reste est une reconstruction.

---

## Ce qui garde l'invariant

- `tools/mika_permission_policy/tests/test_no_filesystem_access.py` — deux gardes
  complémentaires, **et chacune couvre ce que l'autre ne peut pas voir**. **(a)** une
  garde AST qui refuse au registre toute *capacité* d'atteindre le monde extérieur :
  import hors d'une **allow-list nommée** (pas une liste d'interdits, qui ne protège
  qu'une écriture du défaut), appel à `open`, et les deux portes dynamiques `__import__`
  et `importlib` par lesquelles une liste de noms se contourne. **(b)** un pin
  comportemental qui affirme l'**indifférence au chemin et au cwd** sur **toute** entrée
  de `get_policy()` — pas sur un échantillon.
- **La limite, dite plutôt que sous-entendue :** (a) refuse une capacité, donc elle ne
  voit pas un carve-out qui n'en demande aucune — `if argv[-1].endswith("system_prompt.md")`
  n'importe rien et n'ouvre rien. C'est (b), et (b) seule, qui attrape cette forme ; c'est
  pourquoi (b) balaie le registre entier. Un test épingle cette limite elle-même, pour
  qu'elle reste une propriété vérifiée et non une phrase. Contre un auteur *déterminé*,
  ni l'une ni l'autre n'est une frontière de sûreté : elles bornent la dérive de bonne foi.
- Les deux gardes portent leur propre **pin d'anti-vacuité** (`TestTheGuardBites`) et
  tournent en CI (`ci.yml`, job `Check`). Elles n'y tournaient pas quand ce document a
  été écrit la première fois : une garde qu'aucun gate n'exécute est une décoration, et
  c'est la revue de code qui l'a relevé.
- Le message de classe C de `dispatch-lib.sh` (deux sites) enseigne la règle de lecture
  ci-dessus, avant d'envoyer chercher un trou d'allow-list.

**Ce que ces gardes ne couvrent pas :** les étages tier1/tier2 vivent dans `claude-pilot`,
hors du périmètre dispatchable. L'invariant y est vrai aujourd'hui (listes de binaires,
regex sur chaînes de commande — aucune lecture de fichier), mais rien ne l'y garde. Un
jumeau de la garde côté `claude-pilot` est un ticket de suivi.

---

## Le vrai coût de #2295, et à qui il appartient

Deux défauts distincts se sont enchaînés, et un seul est traité ici.

- **Le pilote émet une forme refusée et la session halte** (`interrupt=True`). Le remède
  est que le deny soit *récupérable* : que le pilote apprenne « refusé : <règle> » et
  réémette au lieu de mourir. C'est **mika#1410**, dans `claude-pilot`, hors du périmètre
  dispatchable. Non fermé ici.
- **Le lecteur du deny infère la mauvaise cause.** C'est ce que ce document et le message
  de classe C ferment.

## Voir aussi

- `docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`
  — la classe C et l'historique de faux positifs de `_split_compound_command`.
- `docs/solutions/security-issues/1817-mika-side-plugin-per-binary-safety-functions.md`
  — le registre lui-même et son contrat de parité avec tier1.
- mika#1686, mika#1708 — la surface compound fermée **par conception** ; l'élargir est une
  décision de sûreté amont, pas un carve-out en passant.

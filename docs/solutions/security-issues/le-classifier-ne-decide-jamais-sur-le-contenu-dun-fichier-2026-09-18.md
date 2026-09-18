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

Le marqueur `[policy:deny]` a la forme `[policy:deny] <Tool>: <commande> [<rule-id>]`.
**Le `[<rule-id>]` final est ce qui attribue la cause** — et c'est la première chose à
lire. Un refus **sans** `rule-id` ne vient pas de l'étage déterministe du tout, mais du
jugement LLM atteint par `canUseTool` : aucun élargissement d'allow-list ne l'affectera.

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

Ces mesures s'étendent au 14 septembre : `git log --since=2026-09-01 --
tools/mika_permission_policy/` est vide, donc l'étage déterministe n'a pas changé entre
les deux dates.

---

## La règle de lecture d'un `[policy:deny]`

C'est la doctrine utile, et elle tient en quatre points.

1. **Le marqueur nomme une commande et une règle, jamais un fichier qu'elle lit.** La
   forme est `[policy:deny] <Tool>: <commande> [<rule-id>]` — documentée au commentaire
   de `skills/bundled/_shared/dispatch-lib.sh` (« The line shape is
   `[policy:deny] <Tool>: <command>[ \[rule-id\]]` ») et confirmée par la regex
   d'extraction de `docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`
   (`\[policy:deny\] [A-Za-z]+: [^[]+\[[a-z-]+\]`).

2. **Lire d'abord le `[<rule-id>]` en fin de ligne.** C'est lui qui attribue la cause, et
   il est déjà dans le message que l'opérateur a sous les yeux. Aucune enquête n'est
   nécessaire avant de l'avoir lu.

3. **Ne jamais tronquer la commande en la rapportant.** mika#2312 coupe la commande à
   `do …` et s'arrête avant le crochet : le `…` a emporté exactement l'information qui
   aurait clos l'enquête, et le corps de la boucle avec — or un corps qui chaîne (`|`,
   `&&`) ou substitue (`$(…)`) est justement ce que M4 montre refusé.

4. **Un refus sans `rule-id` désigne le troisième étage**, le jugement LLM atteint par
   `canUseTool` (`.claude/claude-pilot.json` → `mika --agent mika-dev ask`, timeout
   120 s). Étant un jugement, il peut diverger entre deux dates sans qu'aucun code ait
   changé, et **aucun carve-out déterministe n'y changera rien**. Les règles
   déterministes portent un `rule-id` ; un jugement n'en a pas.

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
simple plutôt qu'une chaîne composée). S'il est absent, c'est le troisième étage, et la
réponse n'est pas dans ce registre.

---

## La cause du deny de #2295 est indéterminée, et c'est une conclusion

Les éléments cités par le ticket ne permettent pas de l'établir, et il faut le dire
plutôt que de lui substituer une hypothèse. Trois causes restent compatibles :

1. **Le corps tronqué de la boucle** — s'il chaînait ou substituait, c'est M4 qui
   s'applique.
2. **Le veto de chaîne**, dont `_split_compound_command` a un historique documenté de
   faux positifs sur des greps à alternation
   (`2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`, § Gap 1).
3. **Le troisième étage** (jugement LLM), que M1–M3 ne peuvent pas exclure — un jugement
   peut avoir refusé le 14 ce qu'il autorise le 18.

**Aucune des trois n'est une règle de contenu.** La conclusion de ce document ne dépend
donc pas de savoir laquelle a mordu.

**Pour trancher, si quelqu'un y revient :** récupérer la ligne complète sur l'hôte, hors
sandbox — `grep -m1 'policy:deny' /var/log/claude-pilot/3587fe25.stderr` — et lire le
`[rule-id]` final. Si ce `rule-id` nomme une règle de chemin ou de fichier, ce document
est réfuté et la branche « over-block » redevient ouverte — mais elle se traiterait alors
dans `claude-pilot`, pas dans ce dépôt.

---

## Le piège de méthode, nommé comme tel

La première rédaction du plan de mika#2312 avait conclu « c'est la boucle `for` qui est
refusée », sur la foi d'un document décrivant `decompose()` — et M3 l'a réfutée en une
commande. C'est la même erreur que celle du ticket, d'un cran plus haut : une doctrine
tirée d'une documentation plutôt que d'une mesure reproduit exactement l'inférence
qu'elle prétend corriger.

C'est la raison d'être du point 2 de la règle de lecture. Le `rule-id` est une **mesure**
que le système émet lui-même ; tout le reste est une reconstruction.

---

## Ce qui garde l'invariant

- `tools/mika_permission_policy/tests/test_no_filesystem_access.py` — deux gardes
  complémentaires. **(a)** une garde AST qui refuse tout import d'un module d'accès au
  monde extérieur et tout appel au builtin `open` dans le registre : elle refuse la
  *capacité*, pas une instance, donc un carve-out ajouté plus tard la fait rougir même si
  personne n'a pensé à tester son chemin exact. **(b)** un pin comportemental qui affirme
  l'**indifférence au chemin** des fonctions de lecture — même verdict pour un
  `system_prompt.md`, un `skill.toml` voisin, un chemin inexistant et un chemin hors
  worktree. Cible : `make test-permission-policy-plugin`.
- Le message de classe C de `dispatch-lib.sh` (deux sites) enseigne désormais la règle de
  lecture ci-dessus, avant d'envoyer chercher un trou d'allow-list.

**Ce que ces gardes ne couvrent pas :** les étages tier1/tier2 vivent dans `claude-pilot`,
hors du périmètre dispatchable. L'invariant y est vrai aujourd'hui (listes de binaires,
regex sur chaînes de commande — aucune lecture de fichier), mais rien ne l'y garde. Un
jumeau de la garde côté `claude-pilot` est un ticket de suivi. Et le troisième étage, par
nature, ne peut pas être gardé par un test déterministe : son signal distinctif est
l'absence de `rule-id`, documenté ici et porté par le message de deny.

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

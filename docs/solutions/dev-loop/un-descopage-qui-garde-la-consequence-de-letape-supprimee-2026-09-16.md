---
module: mika-agent/wip_rescue
tags: [loop-substrate, fail-open, wip-rescue, decision-core, descoping, marker, livelock, qa-review]
problem_type: fail_open_gate
category: dev-loop
created: 2026-09-16
ticket: mika#2286
---

# Un déscopage qui garde la conséquence de l'étape supprimée

## Le problème

Le 2026-09-10 à 18:01:02Z, le démon `wip_rescue` a un-drafté seul la PR #2285 — classée
**DECISION-CORE**, c'est-à-dire la classe que personne n'a le droit de fusionner sans une
main humaine — alors que le marqueur de vérification de son corps disait toujours
`<!-- rescue-pipeline-verified: no -->`.

La QA avait tenu la porte à 17:43 (`VERDICT: hold[review]` — « still in draft with
pipeline-verification marker set to no ») et l'a ouverte à 18:08 (`VERDICT: pass`).
**Aucun humain n'a touché la PR entre les deux.** La seule différence d'état est
l'un-draft du démon.

Le fix de #2285 était bon et la QA a relu pour de vrai. C'est de la chance, pas une
garantie.

## La cause

Le marqueur existe pour **gater** l'un-draft sur une vérification. Le corps de secours le
dit à l'opérateur en toutes lettres : *« verify pipeline completion, then either un-draft
this PR or set the marker to `yes` »*. Et pourtant :

```
$ grep -c rescue-pipeline-verified crates/mika-agent/src/wip_rescue.rs
0
```

**Le démon ne savait pas que ce marqueur existait.** Il classait, puis un-draftait —
la classification ne conditionnait que le *commentaire* posté ensuite, jamais l'un-draft
lui-même.

Ce n'est pas un oubli d'écriture. C'est la lettre de la spec mika#1852, dont l'étape 6
dit : *« If ANY DECISION-CORE file → un-draft PR + comment "ready-for-Vincent-review" »*.
Mais cette étape 6 venait **après** l'étape 5, *« Re-run pilot »* — après une
re-vérification du pipeline. La v1 a déscopé les étapes 4-fix et 5, **et gardé
l'étape 6**.

> Ce qui restait « vérifié » dans la spec ne l'était plus dans le code, et le marqueur
> qui devait le dire n'était lu nulle part.

## La leçon

**Quand on déscope une étape de vérification, la conséquence qui la suivait n'est plus
autorisée.** Elle ne devient pas « à surveiller » ou « acceptable en attendant » : elle
change de nature, parce qu'elle reposait sur un fait que plus rien n'établit.

Le déscopage était documenté (`wip_rescue.rs` § *Scope boundary (v1)* nomme les étapes
retirées). C'est précisément ce qui rend le cas instructif : **documenter ce qu'on retire
ne suffit pas ; il faut relire ce qui en dépendait.** Le § *Scope boundary* disait quelles
étapes manquaient ; rien ne disait quelle étape *restante* venait de perdre son
fondement.

### Le corollaire qui a coûté le plus cher : une prémisse vraie devenue fausse ailleurs

`qa-review/system_prompt.md` Step 1.5 considérait une PR comme vérifiée si *« The PR
`isDraft` field is `false` (**operator un-drafted it** — this is a stronger signal than
any body marker) »*.

Cette parenthèse était **vraie avant mika#1852** : seul un humain pouvait un-drafter une
PR de secours, l'outil LLM étant bloqué par mika#1682. Elle est devenue fausse le jour où
le démon a acquis ce geste — et personne n'est allé la relire, parce qu'elle vit dans un
autre fichier, dans un autre langage, et qu'elle n'a pas changé.

> Un changement qui donne à une machine un geste jusque-là humain doit chercher **qui
> lisait ce geste comme une signature humaine**. La prémisse n'est pas cassée là où elle
> est écrite ; elle est cassée à distance, en silence, et elle reste syntaxiquement
> correcte.

## La forme du correctif

Option (b) du ticket : DECISION-CORE un-draftée en auto **seulement** si le marqueur lit
`yes` ; sinon **parquée**. Trois contraintes ont structuré l'implémentation, et deux
d'entre elles ne sont pas déductibles de l'énoncé.

### 1. Fail-closed sur le marqueur

Seul le littéral `yes` déverrouille. `no`, absent, autre valeur, corps illisible, deux
marqueurs qui se contredisent → non vérifié. Le repli *« pas de marqueur = PR
pré-mika#1618, on passe »* que la QA se tolère n'a **pas** sa place ici : toutes les PR
`wip-rescue` sont produites par dispatch-lib, qui pose le marqueur depuis le 2026-06-29,
donc une absence signifie un corps abîmé — pas une PR antérieure au contrat.

Le fail-closed se **compose** avec celui de la classification : `classify_route` répond
DECISION-CORE quand elle n'arrive pas à lire le diff, donc une classification illisible
sur un marqueur `no` **parque** au lieu d'un-drafter. C'est « fail-closed » propagé d'un
cran, pas une politique nouvelle.

### 2. Laisser en draft sans autre état, c'est reconstruire le livelock mika#2199

Le filtre d'éligibilité ne lit que `--draft` + labels + âge. Un brouillon DECISION-CORE
laissé en draft **sans marque** serait ré-élu au tick suivant (cron 5 min) : fetch →
rebase → clippy (jusqu'à 900 s) → push → classify → *toujours `no`* → et ainsi de suite,
en occupant l'unique slot (cap = 1, plus vieux d'abord) et en affamant tout ce qui est
derrière. C'est exactement la forme mesurée le 2026-09-05 : **14 bails sur la seule
PR #2197 en six heures**, parce que l'exclusion reposait sur un label dont l'écriture
échouait.

D'où un marqueur durable dans `audit_events` — mais **ré-armable**, ce qui est la
différence avec le bail : rien n'efface un bail ; ici, le `yes` doit lever l'exclusion.
Le prédicat s'écrit `has_parked_marker(n) && !pipeline_verified(&pr.body)`, et le test du
corps passe **en premier** parce qu'il est gratuit — un brouillon ré-armé ne doit pas
payer une lecture SQLite pour découvrir qu'il est à nouveau éligible.

### 3. Ce n'est pas un bail, et le vocabulaire doit le dire

Pas de label `human-review-required`, pas de marqueur `wip_rescue_bailed`, pas de
« a human owns this PR from here ». **Le démon n'a rien trouvé d'anormal** : il a rebasé,
clippy passe, et il manque seulement la vérification que la classe exige.

Un bail et un parcage sont deux états, donc deux noms — sans quoi l'opérateur qui lit la
PR part chercher un conflit ou un clippy rouge qui n'existent pas, et les deux populations
deviennent incomptables séparément.

## Comment repérer la classe

Trois questions, à poser au moment où l'on retire une étape :

1. **Qu'est-ce qui, dans la suite, supposait que cette étape a eu lieu ?** Pas « qu'est-ce
   qui l'appelle » — qu'est-ce qui *en dépend sémantiquement*.
2. **Existe-t-il un marqueur censé porter ce fait ?** Si oui : `grep` son nom dans tout
   le code. Zéro occurrence côté lecteur est un défaut, pas une économie.
3. **Ce changement donne-t-il à une machine un geste qui valait signature humaine ?**
   Si oui, chercher tous les lecteurs de ce geste — y compris dans les prompts, y compris
   dans les autres dépôts.

## Voir aussi

- `two-predicates-for-one-concept-livelock-2026-09-03.md` — deux lecteurs d'une même
  notion qui divergent ; ici c'est **zéro** lecteur, l'autre extrémité du même axe.
- `rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md` — même module,
  même famille : un filet de secours qui agit sans examiner ce qu'il a attrapé.
- mika#2199 — la forme du livelock que la contrainte 2 évite (marqueur durable plutôt
  qu'état sur GitHub).
- mika#1852 — la spec dont l'étape 6 est ici révisée, pas contredite.

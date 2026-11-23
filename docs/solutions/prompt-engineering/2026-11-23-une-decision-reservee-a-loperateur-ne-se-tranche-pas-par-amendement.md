---
module: dispatch-lib, mika-arch
tags: [grooming, permission-policy, operator-decision, prompt-guard, escalate, mika3122]
problem_type: best-practice
---

# Une décision réservée à l'opérateur ne se tranche pas par amendement du plan

**Ticket :** mika#3122 · **Mesuré :** 2026-11-03 (mika#3024 / PR #3029) et 2026-11-17 (mika#1727, vol V3c-2b)

## Le défaut

Le substrat réserve certaines décisions à l'opérateur. Il le fait de deux façons,
et les deux ont été franchies par la boucle avec la ratification des DEUX passes
d'architecte.

**Forme 1 — un refus de la politique de permission est un arbitrage, matérialisé
en code.** Le 03/11, le groom de mika#3024 a sondé trois fois
`sed -n 'a,bp' > cible`. La politique a refusé les trois
(`no matching policy rule -- denied by default`). Le plan a inscrit le refus
**comme un risque avec un repli** — R1, « basculer sur le découpeur Rust jetable ».
Les deux passes ont accepté, à quatre minutes d'intervalle (16:58Z, 17:02Z). QA a
approuvé. Merge `1c6f30fbc`.

**Forme 2 — une halte réservée ne se lève pas par un amendement du plan.** Le
17/11, la tête du vol V3c-2b disait que le groom devait s'arrêter et nommer les
coupes sans en pratiquer aucune, et que « Une halte ne se lève jamais par un
amendement du plan. » La première passe a rendu `ITERATE` avec le finding F1. Le
pilote de révision a lu F1 comme une demande de s'engager (12:29:39Z) et a
réécrit le plan pour **pratiquer la coupe B**, en citant le Plan-Size Gate
(mika#2636) et la borne par vol (mika#2693). La seconde passe l'a laissé passer
par le **cas 4** de l'arbre de volume. L'opérateur a dû tuer le pilote vers 13:04Z.

## Ce que chaque épisode enseigne

**Un repli plus propre est le même renversement, en meilleure écriture.** Ce que
le refus a interdit, c'est l'EFFET. Qu'on l'obtienne par `sed` ou par un
programme Rust jetable ne change pas l'objet du refus — et un repli écrit dans un
langage sanctionné est précisément celui qu'une revue laisse passer.

**`ITERATE` sur une décision réservée n'est pas un remède plus doux que
`ESCALATE` : c'est le véhicule du défaut.** `ITERATE` renvoie le plan à un pilote
de révision, et lire un finding comme une demande de s'engager est son
comportement NOMINAL — réviser, c'est cela. La réserve doit SORTIR de la boucle,
et `ESCALATE` est la seule disposition qui l'y porte.

**Le cas 4 d'une porte de volume ne ratifie pas une coupe que la révision a
elle-même décidée.** Les deux arbres de volume laissent passer un plan dont le
périmètre courant repasse sous la borne. Cette branche juge la TAILLE de ce qui
reste ; elle ne dit rien de QUI avait le droit de choisir ce qui est parti. Une
révision qui pratique une option réservée et ramène le total sous la borne
produit exactement l'artefact que le cas 4 ne sait pas distinguer d'une coupe
légitime. Il faut donc lire la PROVENANCE de la coupe, pas seulement son
arithmétique.

**Une citation n'est pas une autorité.** mika#2636 et mika#2693 demandent un plan
prêt à partir. Aucun des deux ne demande de trancher à la place de l'opérateur.
Un plan qui s'arrête proprement sur une décision d'opérateur EST un plan prêt, et
son `ESCALATE` est une sortie NOMINALE du grooming, pas un échec.

## Le remède, et pourquoi il est à quatre endroits

La garde vit sur le canal que l'exécutant lit
(`docs/solutions/prompt-engineering/2026-09-07-une-garde-de-prompt-doit-vivre-sur-le-canal-que-lexecutant-lit.md`).
Les deux épisodes ont deux exécutants distincts, donc deux canaux :

1. **`_OPERATOR_RESERVED_RULE`** (`skills/bundled/_shared/dispatch-lib.sh`) —
   injectée dans le `PROMPT` du dispatch `dev-groom`, **en dernier** des trois
   règles de grooming. Dans un prompt de 16 KiB, la récence est le seul levier
   d'une règle de fin ; une qualification placée avant ce qu'elle qualifie ne se
   lit que comme une nuance.
2. **La même règle ajoutée au findings-file de première passe** — qui est le
   SEUL prompt du pilote de révision (`_launch_revise_pilot` le lance avec
   `-- "@${findings_file}"` et rien d'autre). C'est l'exécutant de la forme 2 :
   une règle qui ne vit que dans le prompt du groomeur ne l'atteint jamais.
3. **`### Operator-Reserved Gate (mika#3122)` dans `mika-arch-groom-ticket`** —
   `ESCALATE`, jamais `ITERATE`. Branche 2 de la porte mika#1244 rendue explicite.
4. **La même porte dans `mika-arch-second-review`** — `ESCALATE`, plus la clause
   du cas 4. Cette passe est **sans recours** : ni troisième revue, ni porte en
   aval pour l'une ou l'autre forme. Quand les deux lectures se discutent, c'est
   celle qui se rattrape qui est écrite.

## La contrainte lexicale, et pourquoi elle est un élément de conception

Le texte ajouté au findings-file **n'orthographie ni `Fire-Disposition` ni
`Taille estimée`**, et cite ses voisines par NUMÉRO.

Raison : le terme 1 de `_fd_retry_if_section_still_missing` et celui de
`_plan_size_retry_if_section_still_missing` sont deux `grep -qF` sur le TITRE de
la section que chacun réclame, et ils lisent **ce fichier-là**. Orthographier
l'un des deux titres dans la règle rendrait leur premier terme vrai PAR
CONSTRUCTION : les deux rattrapages relanceraient un pilote de revise à chaque
`ITERATE`, avec un finding synthétique faux.

Cette contrainte gouverne la RÉDACTION, pas seulement la vérification — et elle
est tenue dans les deux sens par `OR5` de `test-dispatch-lib.sh` : une assertion
lexicale qui nomme la faute, et une sonde qui exerce les VRAIES fonctions de
rattrapage pour prouver la conséquence. Les deux moitiés comptent : c'est la
conséquence qui tue un pilote. Les prédicats ne sont jamais re-grepés dans le
test — ce serait un second site de vérité, où une faute de frappe serait verte.

## Ce que le test tient, et ce qu'il ne peut pas tenir

`OR1`–`OR7` (`skills/bundled/_shared/test-dispatch-lib.sh`, 70 assertions) tient
la présence du texte, sa forme positive AVANT l'interdit, l'ordre d'injection,
l'appartenance au bloc gardé sur le skill, l'invariant de position mika#138, le
contenu du findings-file, la contrainte lexicale et la parité des deux prompts
architecte. Chaque bloc a été vu rouge avant d'être vu vert, et les comptes
consignés dans les commentaires sont les comptes mesurés.

Ce qu'aucun test ne tient : **le jugement**. Que l'architecte reconnaisse un
repli au même effet, ou une coupe dont la provenance est réservée, est un
jugement de modèle. Le test garantit que la porte est devant lui, datée de ses
deux épisodes — une porte dont on a perdu l'épisode se relitige au premier plan
qui la trouve gênante.

## Rattachements

- `docs/solutions/prompt-engineering/2026-09-07-une-garde-de-prompt-doit-vivre-sur-le-canal-que-lexecutant-lit.md`
- `docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md` (mika#2693)
- `docs/architecture/review-guide.md`

---
title: La note d'un refus ne peut pas valoir preuve — ancrez le lecteur ET le producteur dans le même commit
date: 2026-09-29
last_updated: 2026-09-29
category: architecture-patterns
module: mika-agent/task_state/tasks
problem_type: architecture_pattern
component: dev-loop
severity: critical
applies_when:
  - Une porte prouve un état par la présence d'un marqueur dans un texte libre
  - On s'apprête à ancrer un lecteur (`starts_with`, `^…`) qui lisait par sous-chaîne
  - Un producteur pose un marqueur par `sed` non ancré doublé d'un filet `grep`
  - Un refus explique par écrit ce qu'il ne frappe pas
tags:
  - substring-pollution
  - anchored-reader
  - grooming-gate
  - false-negative
  - single-reader
related:
  - mika#2590
  - mika#2105
  - mika#2050
  - mika#2545
  - mika#2158
  - mika#2484
---

# La note d'un refus ne peut pas valoir preuve

## Le défaut, mesuré le 2026-09-29 sur mika#2105

La porte de provenance du grooming (#1620, mika#2287) a laissé partir un pilote
**implement** sur un ticket jamais re-groomé. Sa preuve était la **note d'un
refus**.

| heure (Z) | fait |
|---|---|
| 14:01:08 | `ready_label_handled … target_skill=dev-groom groomed=false` |
| 14:05:09 | callback groom `89165fb4`, `result` = `{"status":"auto_skipped","reason":"already_groomed",…}` |
| 15:01:08 | `ready_label_handled … target_skill=dev-pilot groomed=true` |
| 15:01:12 | pilote **implement**, sans plan re-mesuré |

Entre 14:01 et 15:01, la seule nouveauté est `89165fb4`. Le prédicat était :

```sql
AND instr(child.result, 'Outcome: PLAN_GROOMED') > 0
```

Et la note JSON que `dispatch-lib.sh` écrit sur un saut `already_groomed` **cite
le marqueur en toutes lettres**, pour expliquer qu'aucune preuve n'est frappée :

> « … the provenance gate refuses it with `dispatch_grooming_not_verified` unless
> a completed groom callback carrying **Outcome: PLAN_GROOMED** exists, and this
> skip mints none. »

`select instr(result,'Outcome: PLAN_GROOMED') …` rend **651**.

## La leçon, en une phrase

> **Un grep par sous-chaîne est pollué par la prose qui nomme ce qu'il cherche —
> et le polluant le plus dangereux est le texte qui explique l'absence du
> marqueur.**

C'est la forme la plus aiguë d'une classe déjà mesurée deux fois : mika#2050 sur
le Signal S (« un pilote qui *discute* du jeton se lit comme une émission ») et
mika#2545 un marqueur plus loin. Ici l'inversion est complète : **le texte qui
dit « ceci n'est pas une preuve » EST la preuve.**

Corollaire de conduite : un refus qui documente sa propre condition de levée
fabrique un candidat à la pollution. Cette prose est utile — elle dit à
l'opérateur quoi faire — donc on ne la supprime pas : **on cesse d'y citer le
jeton**, et on rend cette abstinence structurelle (assertion sur le producteur),
jamais prescrite.

## Le piège que personne ne voit venir : ancrer le lecteur SEUL introduit un faux négatif

C'est la moitié non évidente, et elle n'est **pas lisible dans le code final**.

Le producteur de la convergence écrivait :

```sh
RESULT=$(printf '%s' "$RESULT" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
if ! grep -qF -- 'Outcome: PLAN_GROOMED' <<<"$RESULT"; then
    # … ajouter la ligne canonique
fi
```

**Ni le `sed` ni le `grep` ne sont ancrés.** Sur un RESULT portant
`(status: success). Outcome: PLAN_COMMITTED` — le marqueur en **milieu de
ligne** — le `sed` pose `Outcome: PLAN_GROOMED` au milieu de la ligne, et le
filet qui suit **le voit** et n'ajoute donc PAS la ligne canonique.

Conséquence : un lecteur ancré livré **seul** aurait refusé un groom
**réellement convergé**. On aurait échangé un faux positif (un implement sans
plan) contre un faux négatif (la boucle bloquée sur un grooming réussi) — et le
second casse la boucle plus vite que le premier.

> **Règle : lecteur et producteur d'un marqueur voyagent dans le même commit.**
> Ancrer la lecture est une demi-mesure tant que l'écriture n'est pas ancrée par
> construction.

Le remède côté producteur n'est pas d'ancrer le `sed` : c'est de le retirer au
profit d'un helper qui rend « exactement une ligne `Outcome:` ancrée » vraie
**par construction** (`_set_outcome_line`, mika#2492) — là où le couple
`sed` + `grep` ne la rendait vraie que par coïncidence d'ordonnancement. Le filet
disparaît avec lui : il n'a plus rien à rattraper.

**Ce qui rend ce couplage sûr ici :** le Rust et `dispatch-lib.sh` voyagent dans
le même binaire (`skills/bundled/` est une projection du binaire, mika#2340).
Lecteur ancré et producteur ancré ne *peuvent pas* être servis séparément. Sur
deux artefacts déployables indépendamment, il faudrait une fenêtre de
compatibilité — et c'est alors le producteur qui part en premier.

## Trois états, jamais un booléen

Le verdict est un `enum` à trois variantes, pas un `bool` :

```rust
pub enum GroomConvergence {
    Converged,
    Absent,                            // le marqueur n'apparaît nulle part
    MarkerOutOfPosition(&'static str), // il apparaît, hors position de verdict
}
```

`Absent` et `MarkerOutOfPosition` appellent la **même** disposition (refuser) et
**deux lectures opérateur opposées** : la première est le régime nominal d'un
premier grooming, la seconde est un implement que la porte vient d'arrêter. Les
fondre rendrait la population du correctif **incomptable** — et un ticket refoulé
pour preuve polluée se lirait exactement comme un ticket jamais groomé, qui est
la classe mika#2205 appliquée au correctif lui-même.

Le motif n'est posé **que s'il y a quelque chose à écarter** : une enveloppe JSON
qui ne cite pas le marqueur (un saut `issue_closed` ordinaire) rend `Absent`, pas
un motif. Sinon le compteur du correctif se remplirait d'une population qui n'a
jamais menacé la porte.

## Le `instr` quitte le SQL, il ne devient PAS un pré-filtre

Le réflexe maison est « le proxy filtre d'abord en SQL, la mesure directe tranche
ensuite en application » (mika#2184). **Il est refusé ici**, et la raison tient au
compteur : une ligne écartée par SQL est **invisible** au verdict, donc un
`auto_skipped` remonterait `Absent` au lieu de `MarkerOutOfPosition` — et la
mesure du correctif rendrait zéro sur exactement la population qu'elle existe
pour voir.

La cardinalité ne le justifie pas non plus : la jointure borne déjà à une poignée
de lignes par issue. Effet de bord acquis : **plus aucun lecteur SQL du
marqueur**, ce qui rend la garde de classe totale plutôt que partielle.

## Le fixture du test négatif est GELÉ

Le JSON du test de non-régression est celui de l'incident, recopié verbatim.
**Le régénérer depuis le producteur corrigé ferait disparaître la forme même que
le test doit refuser** : le test passerait des deux côtés du correctif et
n'attesterait rien. Précédent écrit : `tests/fixtures/grooming_bodies/`
(mika#2158) et `plan_callout_bodies/` (mika#2120).

Ce qui est gelé est la **forme** (l'enveloppe JSON dont la note cite le marqueur),
pas l'octet : les valeurs variables sont reconstituées, et c'est dit au site
plutôt que passé sous silence — prétendre à une copie octet-pour-octet serait
fabriquer une mesure.

**Un test négatif de cette classe peut être rendu auto-prouvant.** Asserter
`MarkerOutOfPosition(json_envelope)` — plutôt que « ce n'est pas une preuve » —
exige *structurellement* que le fixture contienne le marqueur : sans lui la
fonction rendrait `Absent`. La propriété « ce test serait rouge avant le
correctif » devient alors une conséquence de l'assertion, et non une affirmation
qu'il faut croire sur parole.

## La garde de classe, et ce qu'elle n'attrape pas

Un scan de source (`production_sources()`, allowlist **livrée vide**) refuse toute
lecture du symbole par `contains(` / `instr(` / `.find(` hors du fichier
propriétaire. **Aucun test comportemental ne peut voir cette classe** : un second
lecteur lâche ne rend *aucune* décision fausse le jour où il est écrit — il
diverge plus tard, en silence, avec toutes les assertions au vert. C'est
exactement ce que `grooming_marker.rs` a dû graver une fois (mika#2158) et ce que
la porte a repayé ici.

Deux disciplines s'imposent à un tel scan :

- **Anti-vacuité obligatoire**, sur les deux moitiés : la population examinée est
  non vide, **et** le symbole est lu quelque part chez le propriétaire. Sans
  elles, un renommage rend le scan silencieusement inerte — ce qui se lit
  exactement comme un arbre propre.
- **Le vérifier rouge par mutation.** Une anti-vacuité prouve que le scan
  *regarde* ; elle ne prouve pas qu'il *mord*. On introduit un lecteur lâche, on
  observe le refus (et le site qu'il nomme), on révoque.

**Ce qu'il n'attrape pas, nommé :** un lecteur qui reconstruirait le littéral à
la main (`"Outcome: " + "PLAN_GROOMED"`) échappe au prédicat, qui porte sur le
**symbole**. Le scan d'exhaustivité des jetons canoniques (mika#2201), lui, part
du **jeton** et verrait le littéral — la composition des deux ferme le trou que
chacun laisse.

## L'effet de population, à mesurer AVANT de déployer

Tout ticket dont l'unique « preuve » est un `auto_skipped` passe de « groomé » à
« non prouvé » : il repart en `dev-groom`, se fait refuser `already_groomed`, et
consomme le budget de re-drive (mika#2020) jusqu'à `operator-review`. C'est
**convergent et voulu**, et strictement meilleur qu'un implement sans plan — mais
la taille de cette population est un arbitrage d'opérateur, pas d'implémenteur :

```sql
SELECT parent.reference_url, child.id, child.created_at
  FROM tasks child JOIN tasks parent ON child.parent_task_id = parent.id
 WHERE child.trigger_type = 'callback' AND child.dispatch_class = 'groom'
   AND child.status IN ('completed','delivered')
   AND instr(child.result, 'Outcome: PLAN_GROOMED') > 0
   AND child.result LIKE '{"status":"auto_skipped"%'
 ORDER BY child.created_at DESC;
```

**Halte — la liste est longue :** ce n'est pas une panne du correctif, c'est la
taille de la population qu'il renvoie en un tick dans un cul-de-sac convergent.
**Ne pas désarmer le correctif** — remonter, et décider si le déploiement
s'accompagne d'un geste de re-grooming.

## Checklist réutilisable

Avant de livrer une porte qui prouve un état par un marqueur dans un texte
libre :

1. **Le lecteur lit-il par sous-chaîne ?** Si oui, chercher qui, dans l'arbre,
   *parle* du marqueur — en particulier les refus qui documentent leur propre
   condition de levée.
2. **Le producteur écrit-il ancré par construction ?** Si le marqueur est posé
   par `sed`/`printf` non ancré, ancrer le lecteur seul **introduit un faux
   négatif**. Les deux dans le même commit.
3. **Le verdict a-t-il assez d'états ?** Si deux causes de refus appellent des
   lectures opérateur différentes, ce sont deux variantes, pas un `bool`.
4. **La mesure survit-elle au filtrage ?** Un pré-filtre SQL rend invisible
   au compteur exactement ce qu'on veut compter.
5. **Le fixture est-il gelé, et l'assertion auto-prouvante ?** Régénérer depuis
   le producteur corrigé efface la forme à refuser.
6. **Le scan de classe est-il vu rouge ?** Anti-vacuité sur les deux moitiés,
   allowlist vide et pinnée vide, mutation contrôlée.
7. **La population existante a-t-elle été mesurée ?** Le correctif est juste ; son
   effet de bord au déploiement est une décision d'opérateur.

## Ce que ce travail n'achète pas

Aucun rattrapage : l'implement de mika#2105 a eu lieu. **Rien ne rétro-écrit une
ligne** — fabriquer une preuve, ou son absence, après coup est l'inverse de ce que
ce correctif défend. La sonde est la **prochaine** occurrence.

Et aucune convergence pour un ticket groomé hors moteur : il repart en
`dev-groom`, se fait `already_groomed`, et s'abandonne après trois re-drives.
Convergent, borné, visible — et strictement meilleur qu'un implement sans plan.

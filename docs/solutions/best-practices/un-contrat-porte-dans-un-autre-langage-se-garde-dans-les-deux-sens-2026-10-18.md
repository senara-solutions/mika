---
module: mika-agent
tags: [cross-language-port, parse-contract, drift-guard, both-ways, dispatch-lib, null-is-not-zero]
problem_type: best_practice
---

# Un contrat porté dans un autre langage se garde dans les deux sens, et ses jetons d'échec ne se replient pas sur un nombre

**Mesuré :** 2026-10-18 (mika#2699 vol 3, portage de `_plan_size_total_loc`).

---

## Le fait

`skills/bundled/_shared/dispatch-lib.sh` possède, depuis la porte 3 de
mika#2701, un lecteur de la taille annoncée par un plan : `_plan_size_total_loc`
et la table de constantes qui en dérive la borne par vol. mika#2699 vol 3
demandait au manager — un **autre processus, dans un autre langage** — de juger
cette même borne à l'admission d'un ticket au bassin `ready`.

Deux façons de s'y prendre, et une seule survit à six mois :

1. **Appeler le bash.** Le manager est un binaire Rust en lecture seule ; faire
   d'un script shell une dépendance d'exécution de son jugement lui donne un
   chemin d'exécution de processus qu'il n'a pas, et que
   `no_dispatch_test.rs` existe précisément pour lui refuser.
2. **Copier le contrat en Rust.** C'est ce qui a été fait — et une copie est une
   divergence qui attend son commit.

Le coût de la divergence n'est pas une panne : c'est un **nombre plausible et
faux**. Si `PILOT_LABEL_TURN_CEILINGS` gagne une entrée côté bash et pas côté
Rust, le manager calcule la borne du ticket avec le plafond du défaut de flotte
— il refuse à 279 lignes un ticket que le vol aurait tenu à 429. Rien ne rougit,
rien n'avertit : un ticket légitime reste simplement hors du bassin, pour une
raison que le rapport épelle avec aplomb.

## La résolution

### 1. Le garde compare dans les deux sens

`mika2699_le_portage_et_le_bash_saccordent_dans_les_deux_sens`
(`crates/mika-agent/src/milestone_manager/plan_size.rs`) lit `dispatch-lib.sh`
au chemin relatif du manifeste et compare **ensemble trié contre ensemble
trié** : les quatre constantes d'arithmétique par leur nom bash, la table
label → plafond, le défaut de flotte, et les deux motifs d'expression régulière.

Le « dans les deux sens » est la décision, et c'est le motif de
`scripts/check-dispatch-seats-declared.sh` (mika#2092). Un sens seul — « chaque
entrée Rust existe en bash » — laisse passer **l'ajout** d'une entrée du côté
non vérifié, c'est-à-dire exactement la façon dont deux listes cessent de bouger
ensemble. Une comparaison d'ensembles dit en plus *laquelle* a bougé seule, ce
qu'un `assert!(contains)` par entrée ne dit pas.

### 2. Ce qui n'est pas porté est nommé, et l'écart est orienté

La cascade de plafond du bash a quatre paliers ; le portage en tient deux — le
label et le défaut in-file. Les deux paliers d'environnement appartiennent au
**processus hôte du dispatch** : lire son propre `PILOT_MAX_TURNS` ferait
mesurer au manager l'environnement du manager, pas celui du vol à venir.

Un écart non porté ne se tait pas, il se **borne et s'oriente** :

- le palier label domine le palier d'environnement, donc pour un ticket
  `loop-substrate` celui-ci est inerte par construction ;
- l'hôte de production porte la valeur du défaut in-file, donc les deux cascades
  coïncident aujourd'hui sur toute la population ;
- reste le rollback d'incident, pendant lequel le manager **surestime** le
  plafond, donc la borne, donc il **sous-refuse**. Un refus manqué laisse entrer
  un ticket ; le sens inverse aurait gelé les admissions pendant un incident.

Le repli `disarmed_fallback` du bash, lui, n'est **pas** porté du tout, et pour
une raison qui n'est pas la paresse : la cascade Rust rend toujours un nombre,
donc la variante serait une branche inatteignable — verte, plausible, mesurant
zéro (classe mika#2205).

### 3. Les jetons d'échec traversent la frontière, et ne deviennent pas des nombres

`_plan_size_total_loc` rend trois jetons : `absent`, `unparsable`, `<n>`. Aucun
n'est `0`. Le portage les rend en `enum` à trois variantes plutôt qu'en
`Option<u64>` ou, pire, en `u64` avec `0` par défaut (motif mika#2331 : *un null
n'est jamais un 0*). Un `0` se compare à la borne comme un plan minuscule, donc
un plan dont personne n'a lu la taille se lirait « tient largement », ce qui est
l'inverse du fait.

Côté appelant, la même discipline va un cran plus loin : la lecture distante
peut échouer pour des raisons que le bash n'a pas (`404` au ref de la branche,
callout absent du ticket). Elles ne sont **pas** repliées sur `absent` — leurs
remèdes sont opposés (pousser la branche *versus* éditer le plan), et les
confondre enverrait l'opérateur au mauvais endroit. Le docstring du bash dit que
`absent` couvre aussi le fichier illisible ; le portage le **nomme comme un
écart** plutôt que de le reproduire, parce que ce module porte un contrat, il ne
le réécrit pas.

### 4. Aucun de ces jetons ne refuse

Dernière conséquence, et c'est elle qui ferme la boucle : seul un total
**mesuré** strictement au-dessus de la borne produit un refus. Une section
absente, un total illisible, un plan non lu — aucun n'est une estimation « au-
delà de la borne », donc aucun n'est ce refus-là. Ils ne sont pas tus non plus :
le rapport les nomme sous leur cause propre. Sans cette ligne, « à admettre » se
lirait comme « borne vérifiée », qui est exactement la promesse que le portage
est censé tenir.

## L'asymétrie à ne pas perdre

Le bash **mesure et ne refuse rien** : sa porte 3 informe le pilote. Le manager
**refuse à l'admission**, en amont et de façon réversible — un ticket non admis
ce cycle-ci peut l'être au suivant, sans qu'aucun dispatch ait été dépensé. Les
deux sites partagent un contrat de lecture, pas une politique. Un futur éditeur
qui unifierait « les deux endroits qui parlent de la borne » ferait du refus du
manager un refus de dispatch, c'est-à-dire déplacerait la décision en aval, là
où elle coûte un vol.

## La règle

> Quand un contrat doit vivre dans deux langages, copiez-le et **gardez la copie
> dans les deux sens** — jamais l'un, et jamais un `contains` par entrée. Nommez
> ce que vous ne portez pas, et montrez que l'écart penche du côté sûr. Portez
> les jetons d'échec **comme des jetons**, pas comme des nombres : une valeur par
> défaut numérique est la façon dont « on ne sait pas » devient « on sait, et
> c'est petit ».

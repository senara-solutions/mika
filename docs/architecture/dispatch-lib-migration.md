# Migrer `dispatch-lib.sh` — la doctrine, bornée à ce que les phases 1 et 2 ont établi

**Ticket parent :** [mika#2194](https://github.com/senara-solutions/mika/issues/2194).
**État :** phases **1** ([mika#2194](https://github.com/senara-solutions/mika/issues/2194))
et **2** ([mika#2608](https://github.com/senara-solutions/mika/issues/2608)) livrées.
Les **quatre** lecteurs bash et Rust du callout `Plan` délèguent à
`crates/mika-agent/src/plan_callout.rs`, l'inventaire des lecteurs bash est un
**zéro** tenu par un scan, et la tolérance du quatrième
(`milestone_manager::reader::plan_callout_present`) est **conservée par décision**
avec sa raison structurelle. Les phases 3 et 4 sont nommées ici et **aucune n'est
ouverte**.

Ce document ne décrit pas un programme souhaitable : il décrit ce que **deux**
maillons ont établi, pour que le suivant ne redécouvre pas les mêmes bornes. Tout
ce qui n'a pas été mesuré est marqué comme tel — au premier rang le coût réel d'un
démarrage de `mika`, que la phase 2 porte à trois par dispatch (§ 2) sans l'avoir
chronométré.

---

## 1. Les trois critères de sélection d'un maillon

Un maillon migrable coche les trois. Le troisième est le moins intuitif et c'est
celui qui a déjà refusé une migration de cette classe.

### C1 — Pureté

Entrées → sortie, zéro effet de bord. `_extract_plan_path` faisait 12 lignes et
rendait une chaîne.

**Ce que ce critère écarte, mesuré en phase 1 :** `_parse_disposition` (90 lignes)
rend **deux** valeurs, la seconde par un fichier (`$_DISPOSITION_FUZZY_FILE`) —
l'effet de bord fait partie de son contrat, donc le migrer demande d'abord de
décider ce que devient ce canal de retour. `_measure_cycle_output` et
`_gate_non_empty_cycle` (111 et 131 lignes) lisent des fichiers de session.
`_is_dispatchable_repo` (190 lignes) porte une allowlist et des appels `gh`.

### C2 — Un seul appelant

`_extract_plan_path` n'était appelée que par `_detect_plan_on_branch`. Un maillon
à N appelants est N bascules à valider, et le coût de la preuve de parité croît
avec N.

### C3 — Le Rust doit déjà être, ou devoir être, un lecteur

> Ne migre que ce dont le Rust est déjà, ou doit être, le lecteur. Ce que **seul**
> le shell connaît reste au shell et voyage par **stamp**.

Ce critère n'est pas une préférence : il est la raison écrite pour laquelle
**mika-platform#58 a déjà refusé une migration de cette classe**. Citation du
`CLAUDE.md` racine, sous mika#2249 :

> `dispatch-lib.sh` is the only place that knows the worktree path (it calls
> `scripts/derive-worktree-path`), **re-deriving it in Rust is the duplication
> mika-platform#58 closed**.

Le remède retenu là-bas est l'inverse d'une migration : le shell **dit** au Rust
(un stamp de metadata), et le Rust ne dérive pas. Trois mécanismes en vigueur
suivent ce motif — `metadata.dispatch_worktree_file` (mika#2249),
`metadata.pilot_transcript_expected` (mika#2040),
`metadata.qa_review_pr_target` (mika#2368).

Le callout `Plan` satisfait C3 : le Rust le lit **déjà**, dans `auto_pull`, sur le
chemin du feeder. Le chemin du worktree ne le satisfait pas, et c'est pourquoi il
n'est pas ce maillon.

---

## 2. Le critère de canal : la fréquence d'appel décide

| fréquence d'appel | canal | raison |
|---|---|---|
| une fois par dispatch | **sous-commande `mika`** | un démarrage de process sur un dispatch qui dure des minutes à des heures est négligeable |
| en boucle | **arbitrage ouvert** | non préjugé par la phase 1 |

`_extract_plan_path` est appelée **une fois par dispatch** en phase 1, d'où
`mika plan-callout`. Le raisonnement ne repose sur aucun chiffre mesuré : le coût
réel d'un démarrage de `mika` n'était pas mesurable depuis le bac à sable de
dispatch (`mika --version` y est refusé par la permission-policy), et c'est une
**sonde post-déploiement** (§ 7, S3), pas un acquis.

**La phase 2 porte ce compte à TROIS par dispatch dev-groom**, et le chiffre est
écrit plutôt que laissé à découvrir : `_committed_plan_on_branch` depuis
`_set_up_worktree`, le `elif` de `_set_up_worktree`, puis
`_committed_plan_on_branch` depuis la composition du `RESULT` d'un groom non
convergé. La même arithmétique tient — trois démarrages contre un dispatch qui
dure des minutes à des heures — mais elle **consomme** la marge que ce tableau
décrivait, donc la sonde S3 cesse d'être une curiosité et devient la précondition
de la phase suivante.

**Si S3 rend un chiffre au-delà de ce que trois démarrages tolèrent, le remède est
le PLACEMENT dans `main.rs`, jamais un cache.** Un cache sur un prédicat pur est
une seconde source de vérité, c'est-à-dire très exactement la duplication que ces
deux phases viennent de retirer.

Un prédicat appelé en boucle est un autre arbitrage. Ne pas transporter la
conclusion de la phase 1 vers lui sans mesurer : ce serait exactement l'erreur que
ce tableau existe pour empêcher.

### Le corps passe par un fichier, jamais par un argument

Un corps d'issue porte des retours à la ligne, des backticks et des `$`. Le passer
en `argv` ré-introduirait la classe de panne de portée de guillemets (cpp#157)
**dans le geste même qui prétend la fermer**. `mika plan-callout` n'a donc **pas**
de variante positionnelle : le refus est structurel, il n'y a rien à contourner.

### Trois codes de sortie, et le troisième est un livrable

| code | stdout | sens |
|---|---|---|
| `0` | le chemin, une ligne | un callout a été lu |
| `1` | vide | **aucun callout** — la réponse est « non » |
| `≥2` | vide, motif sur stderr | **je n'ai pas pu regarder** |

Avant la bascule, `_extract_plan_path` rendait `1` dans les deux derniers cas et
son appelant faisait `|| return 0` : une erreur de lecture se lisait comme « pas de
plan ». La population « fichier illisible » est **créée par la migration** — avant
elle, le corps était en variable et il n'y avait pas de fichier à ne pas pouvoir
lire. Ne pas la distinguer aurait fabriqué un silence qui n'existait pas.

**Un refus doit atteindre une surface qu'on lit.** Le stderr d'avant-pilote de
`dispatch-lib` est structurellement **perdu** sur un dispatch qui réussit (classe
mika#2050 : il hérite du `Stdio::piped()` de l'exécuteur, que celui-ci ne lit que
dans sa branche `if !status.success()`) — et ce refus-là laisse justement le
dispatch réussir sur `/mika`. Le motif voyage donc jusqu'au `RESULT` du callback,
préfixé comme ceux de `cwd-guard.sh` (mika#2536) et `pr-push-guard.sh`
(mika#2520). Inventer une surface de journal que personne ne lit reproduirait le
défaut du Signal M.

### Pas de repli

`_extract_plan_path` n'a **pas** de repli bash : un repli serait une seconde
implémentation, c'est-à-dire précisément ce qu'on retire. La décision s'appuie sur
une précondition déjà en vigueur — `dispatch_claude_pilot` refuse déjà de démarrer
sans `mika` (« Error: mika CLI is required »).

L'asymétrie de déploiement penche du bon côté, et c'est vérifiable plutôt que
supposé : `dispatch-lib.sh` est une projection du **binaire** (mika#2340), donc le
seed est écrit *par* le binaire installé et les deux moitiés voyagent ensemble.

| état | effet |
|---|---|
| dispatch-lib **ancien** + `mika` **neuf** | l'ancien n'appelle pas la sous-commande — **fail-safe par construction** |
| dispatch-lib **neuf** + `mika` **ancien** | `mika` sort non-zéro sur une sous-commande inconnue ⇒ code `≥2` ⇒ refus bruyant. **Jamais un chemin vide.** |

Le second état n'est pas produit par `make deploy` ; il est produit par une copie à
la main, et il faut qu'il soit lisible plutôt que silencieux.

---

## 3. Les deux bornes écrites, à respecter plutôt qu'à redécouvrir

### B1 — L'harmonisation naïve des lecteurs est DÉJÀ refusée

`crates/mika-agent/src/auto_pull.rs`, doc-comment d'`is_groomed` :

> Il n'est pas non plus le prédicat le plus étroit du dépôt, et cela reste vrai
> après mika#2120 : `executor::check_grooming_markers` se contente de la
> sous-chaîne `docs/plans/`, non ancrée. **Ne le resserrez pas pour « harmoniser »
> les deux** — ce sens-là de l'alignement recréerait le défaut symétrique de celui
> que ce ticket ferme.

**Il y a QUATRE tolérances sur ce jeton, pas trois.** Le plan de la phase 1 en
nommait trois ; la quatrième a été trouvée en implémentant, et **tranchée par la
phase 2 (mika#2608)** :

| lecteur | tolérance | question posée | statut |
|---|---|---|---|
| `plan_callout.rs` | strict, ancré, fences au choix de l'appelant | quel chemin, sous quelle forme ? | **le lecteur unique** (phase 1) |
| `executor::check_grooming_markers` | sous-chaîne `docs/plans/`, non ancrée | ce corps a-t-il un plan ? (routage) | intouché, B1 |
| `dispatch-lib::_committed_plan_on_branch` | — | ce plan est-il committé sur la branche ? | **délègue** (phase 2) |
| `dispatch-lib::_set_up_worktree` | — | callout périmé ? (ligne de journal) | **délègue** (phase 2) |
| `milestone_manager::reader::plan_callout_present` | `contains("**Plan:**")` **sans** le préfixe `> - `, ou `contains("docs/plans/")` | ce corps porte-t-il un callout ? (booléen, LECTURE seule) | tolérance **conservée, DÉCIDÉE** par mika#2608 |

Les phases 1 et 2 ont unifié l'**implémentation** de **quatre** de ces lecteurs.
Elles n'ont unifié **aucune tolérance**, et un maillon futur qui écraserait les
cinq vers la plus stricte fermerait un défaut en ouvrant son miroir.

#### La décision de la phase 2 sur le quatrième, et pourquoi elle est structurelle

Le refus de le migrer ne repose pas sur une préférence de prudence. Sa première
branche est un **sur-ensemble strict** du lecteur unique : toute ligne que
`PLAN_CALLOUT_RE` accepte commence par `> - **Plan:** `, donc — `trim_start`
appliqué — commence par `> ` et contient `**Plan:**`. **Il n'existe aucun corps
que le lecteur strict accepte et que cette branche refuse.**

Donc la réparation tentante — appeler `plan_callout` en première branche et
garder les branches lâches en repli — ajouterait un appel qui **ne peut jamais
décider de rien** : une unification apparente et inerte, c'est-à-dire la classe
mika#2205 appliquée à un chemin de code. La propriété est **assertée** sur le
corpus doré
(`milestone_manager::reader::tests::mika2608_la_branche_lache_est_un_surensemble_du_lecteur_strict`),
et la décision est **épinglée** par son voisin, de sorte qu'un futur éditeur qui
« harmonise » fasse rougir un test au lieu de changer un rapport opérateur en
silence.

Trois raisons secondaires, dans l'ordre : ce n'est pas la même question (booléen
contre chemin) ; la forme lâche est **prescrite** par le contrat de cascade
milestone ; et le coût des deux erreurs penche du bon côté — le manager est
LECTURE seule, son `plan_present` alimente un **rapport à un humain**, donc un
resserrement produirait des faux négatifs qui lui diraient de groomer un ticket
groomé.

**La faiblesse réelle est nommée et NON corrigée** : la seconde branche lit une
citation en prose comme un callout. Faux positif plausible, **aucune mesure ne
l'établit** — suivi avec sa précondition (un rapport dont `plan_present` est
mesurément faux), parce qu'armer un resserrement sur une population non mesurée
est ce que mika#2520 refuse.

#### Les trois deltas que la bascule de la phase 2 a produits

Le `sed` de `_committed_plan_on_branch` était décrit comme « strict, ancré » — y
compris dans le corps du ticket de la phase 2 — et ne l'était pas :

| axe | le `sed` | le lecteur unique | sens | conséquence sur la porte |
|---|---|---|---|---|
| littéral `docs/plans/` | **non exigé** | exigé | resserre | **ferme un faux positif latent** |
| espaces après `**Plan:**` | ` *` (zéro ou plus) | exactement un | resserre | fail-open |
| `../docs/plans/`, `a/b/docs/plans/` | acceptés | refusés | resserre | fail-open |
| normalisation | `${plan_path#"${repo}/"}` — ce dépôt seul | premier segment quelconque | **élargit** | borné par mika#2034 |

Le faux positif latent est réel : un corps portant ``> - **Plan:** `README.md` ``
en extrayait `README.md`, `cat-file -t` rendait `blob` (tout dépôt a un README),
la liaison mika#2034 — dont le contrat est la **réfutation** — ne trouvait aucun
`issue:` et ne réfutait pas, donc la porte **tirait** et le ticket restait bloqué
en `already_groomed` de façon permanente. C'est la classe exacte que mika#2034 a
ouverte pour fermer (#1887, #2026). Fermeture **collatérale**, nommée plutôt que
découverte.

**Le sens de tous les resserrements est fail-open pour cette porte**, sans
exception : un callout que le nouveau lecteur refuse fait que la porte **ne tire
pas**, donc que le grooming procède — la doctrine écrite de la fonction, appliquée
à un changement de tolérance. Le seul élargissement (un préfixe de dépôt
**étranger** résout désormais) est borné par la liaison mika#2034, dont l'en-tête
du plan réfuterait le ticket. Chacun des quatre axes a son cas de corpus.

### B2 — Le critère de sélection C3 ci-dessus

Voir § 1. C'est la même borne, écrite comme critère parce que c'est sous cette
forme qu'elle sert.

---

## 4. La forme du corpus doré : un corpus, deux lecteurs, un terme d'anti-vacuité

### Le défaut que cette forme ferme

Avant la phase 1, les deux lecteurs du callout avaient **chacun** un corpus
soigné :

| | lecteur | corpus | où |
|---|---|---|---|
| Rust | `auto_pull::extract_plan_path` | **6 corps d'issue réels**, gelés, tous préfixés | `tests/fixtures/plan_callout_bodies/` |
| Bash | `_extract_plan_path` | **17 assertions** à fixtures inline | `test-dispatch-lib.sh` |

**Aucune entrée n'était commune.** Personne n'avait jamais exécuté les deux
lecteurs sur la même entrée et comparé. Ce n'est pas une négligence — les deux
jeux sont documentés, l'un ligne par ligne — c'est une configuration dans
laquelle une divergence ne peut pas être vue.

### La forme retenue

- **Un** répertoire de corps, lu par les **deux** lecteurs.
- **Un** fichier d'attendus, `expectations.tsv`, à six colonnes
  (`fichier · rc · raw · normalized · parity · phase`), dont l'en-tête porte la
  sémantique.
- Les attendus décrivent le résultat sous la politique du **bash**
  (`FenceHandling::Keep`), parce que c'est la moitié dont la parité était inconnue
  — décrire l'autre aurait demandé une seconde table.
- **Une colonne sans valeur porte `-`, jamais le vide.** TAB est un caractère
  *whitespace* d'IFS, donc `IFS=$'\t' read` **fusionne les tabulations
  consécutives** : une colonne vide disparaît et tout ce qui suit se décale. Mesuré
  sur la première exécution de la preuve pré-bascule, où les cinq contrôles
  négatifs lisaient leur `parity` dans une colonne vide pendant que le lecteur Rust
  (dont le `split('\t')` ne fusionne rien) les lisait correctement. **Un format que
  les deux lecteurs ne lisent pas pareil est ce que cette migration existe pour
  retirer**, donc la sentinelle est dans le format et non dans un contournement
  chez l'un des deux.

### L'anti-vacuité est obligatoire, et non décorative

Les deux lecteurs **échouent** si le corpus est introuvable, vide, ou nomme un
corps absent, et **chacun nomme son compte de cas**. Une parité verte sur zéro cas
est le seul mode de panne que ni l'un ni l'autre ne verrait tout seul.

Côté bash, un terme de plus : le motif dominant du harnais est
`source "$DISPATCH_LIB" 2>/dev/null || true`, donc un `dispatch-lib` qui cesse de
sourcer laisserait les assertions vertes sur des fonctions **inexistantes** —
éliminatoire pour un test de parité. Le bloc refuse de tourner si
`_extract_plan_path` n'est pas définie après le sourcing.

**Fragilité relevée et non corrigée en phase 1 :** ce terme couvre le bloc de
parité, pas les 215 autres sites qui sourcent de la même façon. Suivi, avec pour
précondition une mesure montrant qu'un sourcing raté a produit un vert.

### `pre-switch` / `post-switch` : la preuve et la non-régression

**La preuve de parité se mesure contre le bash d'AVANT la bascule, et une seule
fois.** Après la bascule le bash appelle le Rust, donc un lecteur comparé à
lui-même est toujours d'accord avec lui-même : la passe post-bascule est une
**non-régression**, jamais une preuve.

La phase 1 a exécuté la preuve à l'implémentation, contre le `dispatch-lib.sh`
extrait de `HEAD` sous `.pilot-scratch/` : **17 cas, 17 concordances, 3
divergences mesurées**. La passe permanente (dans `test-dispatch-lib.sh`) exerce
les lignes `phase = both` et rend 17 cas.

**Correction apportée au plan de la phase 1 sur ce point.** Le plan faisait porter
`parity` sur la relation bash↔Rust, qui **cesse d'avoir deux côtés** après la
bascule : l'assertion auto-nettoyante aurait donc été éphémère. En la faisant
porter sur `Keep`↔`Strip` — interne au Rust, donc toujours mesurable — elle
survit à la bascule. Un cas déclaré `divergent-fences` dont les deux politiques
rendraient la même valeur fait **échouer** le test : le jour où la divergence est
tranchée, la ligne rougit et doit être retirée. C'est la propriété qui distingue
une exception d'un contournement.

### Les trois divergences mesurées en phase 1

Deux n'étaient nommées ni dans le ticket ni dans le plan. Les nommer est un
livrable, pas une note de bas de page.

| n° | divergence | lecture | statut |
|---|---|---|---|
| 1 | **blocs clôturés** — le Rust les retire, le bash non | asymétrie **assumée par écrit** en production côté bash (« un faux positif est déjà rattrapé par le test `-f` qui suit »), et ce rattrapage est **partiel** : il couvre un chemin *inexistant*, pas un chemin *existant cité dans un bloc* | nommée, testée comme divergente, **non corrigée** |
| 2 | **backtick fermant** — le motif Rust l'exige, le PCRE bash s'arrêtait à `[^`]+` sans borne droite | resserrement du bash sur un callout **malformé** que le pipeline ne produit pas ; desserrer aurait élargi la surface de faux positif de `plan_ownership`, qui décide d'un **abandon** de ticket (mika#2020) | **dit** plutôt que découvert |
| 3 | **la capture traverse les lignes** — `[^`]+` n'exclut pas `\n` en Rust, donc un backtick apparaissant plus loin ferme la capture ; `grep` travaillait ligne à ligne | le motif est conservé **à l'identique** (B1 interdit de le resserrer) ; ce qui est borné est le **canal** — `mika plan-callout` refuse d'émettre un chemin qui n'est pas d'une seule ligne (code `≥2`, motif `path_not_single_line`) | le refus vit là où la valeur n'est pas représentable, pas dans le prédicat |

La n°3 mérite sa règle générale : **quand une migration expose une valeur qu'un
canal ne peut pas transporter, borner le canal, jamais le prédicat.** Resserrer le
prédicat aurait changé une tolérance ; borner le canal est strictement plus sûr que
l'état d'avant, où l'un des deux lecteurs rendait un chemin absurde et l'autre un
chemin tronqué, sans que personne l'ait mesuré.

---

## 5. Le scan anti-copie : un inventaire fermé, devenu un ZÉRO en phase 2

Le scan de mika#2120 couvrait **une** fonction. La phase 1 l'étend à tout
`dispatch-lib.sh` — et **pas** sous la forme d'un zéro, ce qui était la deuxième
correction que l'exécution imposait à son plan : un scan exigeant « aucun motif du
callout » aurait été **rouge à la naissance**, la phase 1 gardant délibérément deux
lecteurs. Un lint rouge le jour où il naît se fait désarmer, et la régression qu'il
existe pour attraper passe ensuite dans le bruit.

**La phase 2 retire ces deux lecteurs, donc le zéro est devenu atteignable** — et
c'est le gain principal de cette phase sur l'axe des gardes. Ce n'est pas un
durcissement gratuit : c'est la forme que le scan aurait prise si la phase 1 avait
pu la prendre. Les assertions en vigueur :

1. **ZÉRO** lecteur du motif dans `dispatch-lib.sh` ;
2. les deux fonctions bascules sont **nommées**, et chacune est assertée deux
   fois — elle ne lit plus le motif **et** elle délègue à `_extract_plan_path`.
   Un zéro global ne dirait pas lesquelles ; et une fonction qui aurait cessé
   d'exister satisferait le zéro sans avoir migré ;
3. `_extract_plan_path` n'est pas un lecteur non plus (assertion de la phase 1,
   conservée) ;
4. l'anti-vacuité passe de « au moins trois occurrences » à « au moins une », et
   cette population est désormais **l'écrivain seul** — `_write_canonical_callout`.
   Elle est **nommée** par une assertion à elle : un jeton entièrement disparu
   (renommage du callout, écrivain retiré) rendrait zéro offender, donc un vert
   sur rien. Le zéro de la phase 2 ne vaut que si le motif existe encore quelque
   part.

Les **trois contrôles de bonne foi** ci-dessous sont conservés à l'identique, et
le zéro les rend **plus** nécessaires, pas moins : ce sont eux qui distinguent
« zéro lecteur » de « le prédicat ne regarde rien », et sur un zéro il n'y a plus
de population positive pour le faire à leur place.

### La rouille que les gardes mika#2201 ne voient pas, et ce qui la couvre

`canonical-tokens-survey.sh --check` ne vérifie **qu'une direction** — tout site
strict de l'arbre est déclaré — et son commentaire nomme ce qu'il laisse passer :

> « Residual rot it does not catch, named rather than hidden: a row whose file and
> symbol both still exist but which no longer reads the token. »

C'est exactement la rouille que la phase 2 produit : les deux fonctions existent
encore, donc `mika2201_every_declared_symbol_still_exists` serait resté **vert**
sur deux lignes du TSV devenues fausses. **« Les deux gardes sont vertes » n'est
donc pas une preuve que le TSV est à jour** — correction que la phase 2 apporte à
la lecture naïve de son propre AC5. D'où une assertion supplémentaire, dans
`test-dispatch-lib.sh` : le TSV porte **exactement une** ligne pour le jeton
`> - **Plan:**`, c'est `plan_callout.rs::PLAN_CALLOUT_RE`, et aucune ligne du jeton
ne déclare plus un site bascule (avec son contrôle négatif sur une ligne remise).

La direction générale « toute ligne déclarée matche encore » est **refusée avec sa
mesure** par le survey : le TSV porte légitimement des lignes qu'il ne peut pas
produire (une alternance `(?i)`, un palier flou, un lecteur par `contains`), et la
comparer ainsi ferait rougir le build sur des lignes **correctes** — 14 fausses
contre 2 vraies. L'assertion est donc bornée **au jeton**, ce qui attrape
exactement la rouille de cette phase sans importer ce coût.

### Trois pièges du prédicat, tous mesurés

- **La prose.** Ce fichier **cite** le motif dans ses commentaires de doctrine. Le
  prédicat porte donc sur les lignes de **commande**, commentaires retirés d'abord
  — piège déjà payé deux fois dans ce dépôt (mika#2050 sur le Signal S,
  mika#2201 § R4).
- **L'écrivain.** `dispatch-lib` **écrit** aussi le callout
  (`_write_canonical_callout`). L'écrivain n'est pas un lecteur ; l'accuser rendrait
  le scan rouge sur du code sain. Le discriminant est la présence d'un **outil de
  lecture** sur la même ligne. Ce n'est **pas** une allowlist d'exemption, c'est la
  définition de la population — le TSV de mika#2201 fait déjà cette distinction en
  ne déclarant que des *sites de match*, et la doctrine « on route, on n'allowliste
  pas » porte sur les lecteurs, un écrivain n'ayant rien à router.
- **Les deux écritures du motif.** L'aiguille doit couvrir le littéral
  `**Plan:**` **et** sa forme échappée `\*\*Plan:\*\*`. La première version du scan
  ne portait que la première, donc elle n'aurait **pas** vu revenir le `grep -oP`
  qu'elle existe pour refuser : c'est son propre contrôle négatif qui l'a montré.

Chacun des trois a son contrôle de bonne foi, dont un contrôle **positif** sur
l'écrivain (il doit rester hors population **et** continuer d'exister, sans quoi ce
contrôle mesurerait une population vide et se lirait comme un scan propre).

Côté Rust, le pendant est
`plan_callout::tests::mika2194_aucun_motif_de_callout_hors_de_ce_module`, dont
l'aiguille est le motif **échappé pour une regex** et jamais le littéral nu : ce
littéral est porté par huit fichiers de production (fixtures, messages
d'opérateur, le `contains` lâche d'`executor`) dont **aucun** n'est un second
motif. La frontière production/test y est répondue par son lecteur unique
(`mika_common::source_guard`, mika#2398) plutôt que par une troncature au premier
`#[cfg(test)]` — prémisse fausse six fois dans cet arbre, et qui sur `auto_pull.rs`
précisément laisse 1 859 lignes de production non lues.

**Aucun test comportemental ne peut voir cette classe** : un second motif ne rend
**aucune** décision fausse le jour où il est écrit. Il diverge ensuite, en silence,
avec toutes les assertions au vert. C'est la leçon de mika#2158, et c'est la
raison d'être de ces deux scans.

---

## 6. Le phasage, avec la précondition de chaque phase

| phase | périmètre | précondition | état |
|---|---|---|---|
| **1** | `_extract_plan_path` → Rust, de bout en bout, **plus cette doctrine** | — | **livrée** (mika#2194) |
| **2** | les deux autres lecteurs du même jeton : `_committed_plan_on_branch`, `_set_up_worktree` — **plus la décision sur le quatrième** | phase 1 mergée | **livrée** (mika#2608) |
| 3 | `_parse_disposition` / `_parse_verdict` | décider ce que devient le canal de retour par fichier (`$_DISPOSITION_FUZZY_FILE`) | ticket à ouvrir |
| 4 | la glue d'orchestration (bwrap / git / gh / trap / worktree / callback) → Python | un maillon dont la nature est de la **glue** et non une décision | ticket à ouvrir |

**Ni la phase 3 ni la phase 4 n'est ouverte par la phase 2**, et c'est délibéré,
pour la même raison que la phase 1 l'avait écrit : les ouvrir avant qu'une mesure
les demande serait instruire sans mesure. Leurs préconditions ci-dessus sont
inchangées.

### Ce que la phase 2 a établi sur le maillon migrable, et qui sert aux suivantes

**Le maillon est l'EXPRESSION, jamais la fonction.** Lu sur les fonctions, C2
(« un seul appelant ») aurait écarté les deux sites : `_committed_plan_on_branch` a
deux appelants de production et quinze appels de test, et `_set_up_worktree` est la
fonction la moins pure du fichier. Lu sur les expressions, les deux maillons font
2 et 1 lignes, entrée → sortie. C'est la même lecture que la phase 1, qui n'a pas
migré `_detect_plan_on_branch` (impure) mais `_extract_plan_path` (12 lignes).

**C2 ne contraint pas le fan-in d'un site de délégation déjà migré.**
`_extract_plan_path` passe de un à trois appelants, et ce n'est pas une infraction :
le coût de la preuve de parité croît avec le nombre de **maillons**, jamais avec le
nombre d'appelants d'un lecteur unique — c'est le contraire qui est vrai, chaque
appelant de plus étant une copie de moins. Le doc-comment de la fonction nomme ses
trois appelants, sans quoi un futur lecteur croirait C2 enfreint et
re-dupliquerait le motif pour le « respecter ».

**La normalisation bash est RETIRÉE, et c'est le cœur d'une bascule plutôt qu'un
confort.** `${plan_path#"${repo}/"}` **était** une seconde implémentation —
partielle — de `plan_callout::normalize`. La préserver (en appelant le canal une
seconde fois avec `--raw`) aurait gardé dans bash la moitié de la logique que la
migration existe pour retirer. La boucle à deux candidats n'existait que parce que
bash devait **deviner** la normalisation ; le lecteur la rend. Corollaire pour la
phase 3 : un canal qui ne rend qu'une des formes dont un appelant a besoin force
l'appelant à en dériver l'autre, c'est-à-dire à garder une copie.

**Le drapeau `--raw` n'a toujours aucun appelant de production**, et la phase 2
aurait pu lui donner le premier en préservant cette boucle. Elle ne l'a pas fait,
pour la raison ci-dessus. Il est conservé avec sa raison écrite au site (il est le
miroir CLI des deux champs de `PlanCallout`, et le retirer ferait du canal un
miroir partiel du lecteur) ; son absence d'appelant est **dite** plutôt que
découverte.

**La famille de sous-commandes reste PLATE, et le critère de bascule est écrit.**
La phase 2 n'ajoute **aucune** sous-commande et **aucun** drapeau : ses deux usages
nouveaux sont le **même** prédicat, réutilisé — la distinction 0/1 du canal *est* la
réponse booléenne du second site. La population des sous-commandes de prédicat est
donc encore **un**, et nommer une famille pour une population de un est le geste que
ce dépôt a refusé deux fois par écrit (mika#2329, le fichier sentinelle paramétré
livré avec un seul usage ; mika#2201, les allowlists livrées vides). **Critère de
bascule, pour que la phase 3 n'ait pas à le redériver : la seconde sous-commande de
prédicat distincte.** Tant qu'il n'y en a qu'une, la famille n'a rien à grouper.

Le coût du report est borné et lisible : un renommage ultérieur produit l'état
« dispatch-lib neuf + `mika` ancien » pendant une fenêtre de déploiement, où `mika`
sort non-zéro sur une sous-commande inconnue ⇒ code `≥ 2` ⇒ refus **bruyant et
nommé** (`subcommand_error`), jamais un chemin vide — et cet état n'est pas produit
par `make deploy` (le binaire qui seede est celui qui est installé). L'alternative
refusée, pour qu'elle ne soit pas re-proposée à l'aveugle : `mika predicate <nom>`
dirait « ceci n'est pas pour vous » sur un namespace de tête qui porte déjà 22
sous-commandes d'opérateur. C'est un vrai argument, et il devient décisif au
**deuxième** prédicat, pas au premier.

La phase 3 est la plus tentante — `_parse_disposition` et `_parse_verdict` sont les
plus mordus du lot (mika#1421, #2037, #2338) — et elle est écartée sur mesure : ils
rendent **deux** valeurs via un fichier, donc l'effet de bord est dans le contrat,
et leur tier 2 est une **paraphrase** (classe A du TSV de mika#2201) dont la
migration est un arbitrage de **tolérance**, pas un portage.

---

## 7. Ce qui ne migre pas — et c'est la majorité

### L'ordre de grandeur, pour que la phase 4 ne soit pas sous-estimée

`dispatch-lib.sh` au 2026-09-30 : **551 330 octets, 9 757 lignes, 119 fonctions**
(le ticket parent, écrit le 2026-09-05, mesurait « ≈ 300 Ko » : le fichier a
**presque doublé en 25 jours**). Les fonctions dont le nom et le corps sont des
prédicats purs totalisent de l'ordre de **1 600 lignes sur 9 757**. Les ~84 %
restants sont de la **glue** : bwrap, git, gh, traps, worktree, callback.

« Glue → Python » reste la direction du ticket parent, et c'est la **phase 4**. Le
dire ici évite qu'un futur maillon s'y croie autorisé par le précédent de la
phase 1, qui a migré un prédicat **dont le consommateur était déjà le moteur
Rust**.

### La réduction de taille n'est pas l'objet, et la tendance réelle est autre

La tendance du dépôt depuis le 2026-09-05 n'est pas bash→Rust mais
**monolithe→modules bash** : `pr-push-guard.sh` (24 Ko, mika#2520),
`cwd-guard.sh` (8 Ko, mika#2536) — et pendant ce temps le fichier a doublé. Ce
motif-là réduit le blast radius **sans changer de langue**, donc il ne ferme
aucune des trois classes de panne de parsing que le ticket cite. Les deux
directions sont compatibles et ne répondent pas à la même question.

**La phase 1 ne retire pas une ligne de glue :** `dispatch-lib.sh` perd ~12 lignes
et en gagne ~45 (le `mktemp`, les trois codes, l'annexe du refus, la doctrine en
commentaire). **Le fichier ne rétrécit pas.** Ce qui change n'est pas sa taille,
c'est qu'une décision de moins y est prise. Attendre une réduction de volume d'un
maillon de cette forme serait se tromper sur ce qu'il fait.

---

## 8. Les sondes post-déploiement de la phase 1, et leurs haltes

> **Préalable, non négociable.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir — sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier.

**S1 — le chemin nominal tient** (premier dispatch sur un ticket groomé).
`_detect_plan_on_branch` doit poser `PLAN_PATH` et l'`ENTRY_COMMAND` doit être
`/ce-work <chemin>` et non `/mika`.
*Halte 1 — `PLAN_PATH` est vide et aucun refus n'apparaît dans `result` :* ne pas
retoucher le prédicat. Établir d'abord le déploiement (préalable ci-dessus), puis
lire le `result` — un `subcommand_error` dit que les deux moitiés ne sont pas en
phase, et c'est le remède.

**S2 — le feeder ne régresse pas** (48 h). `auto_pull` continue de promouvoir. La
sonde directe existe déjà (mika#2131) :

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'auto_pull_exclusion' GROUP BY 1 ORDER BY 2 DESC;
```

*Halte 2 — `not_groomed` monte alors que des tickets portent leur callout :* le
lecteur Rust s'est resserré malgré B1. **Revert d'abord, diagnostic ensuite** — une
promotion perdue est la panne qui a coûté quinze heures de boucle à mika#2120, et
elle est silencieuse.

**S3 — le coût est ce qu'on a supposé** (première semaine). Le chiffre que le bac à
sable n'a pas pu produire : chronométrer `mika plan-callout` sur l'hôte.
*Halte 3 — au-delà de ~500 ms :* le chemin court ne l'est pas, ou la sous-commande
résout quelque chose qu'elle ne devrait pas. **Réparer le placement dans
`main.rs`, pas mettre en cache** — un cache sur un prédicat pur est une seconde
source de vérité, c'est-à-dire la duplication qu'on vient de retirer.

**S4 — contrôle négatif du refus** (7 jours). Aucun `REFUSED (plan-callout…)` sur
un dispatch sain :

```sql
SELECT id, result FROM tasks
 WHERE result LIKE 'REFUSED (plan-callout, mika#2194)%' ORDER BY created_at DESC;
```

*Halte 4 — une occurrence :* c'est un faux positif et il coûte un dispatch entier.
Lire le motif ; `body_file_unwritable` est un problème d'hôte,
`path_not_single_line` un callout malformé (divergence n°3), `subcommand_error` un
défaut du lecteur.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro dispatch ne prouve
rien : il faut qu'un dispatch sur un ticket groomé ait eu lieu depuis le
déploiement. *Une garde que personne n'a exercée se lit exactement comme une garde
qui marche* (mika#2205).

### Ce que la phase 1 n'achète pas

- **Elle ne clôt pas mika#2194** — c'est **la phase 1 sur quatre**, un maillon sur
  119 fonctions.
- **Elle n'ajoute aucun compteur.** Le seul instrument neuf est le motif de refus
  dans `tasks.result`, et **son silence ne prouve rien tant que personne n'exécute
  S1 et S4** : sur un prédicat appelé une fois par dispatch, l'absence de refus peut
  vouloir dire que rien n'a mal tourné, ou que rien n'a tourné.
- **Elle ne ferme les trois classes de panne de parsing que pour CE prédicat.** Les
  118 autres fonctions y restent exposées.
- **Elle ne tranche aucune des trois divergences** (§ 4). Elles sont nommées,
  testées comme divergentes, et rattachées à des suivis dont l'objet est de
  **décider**, pas de corriger par défaut.

---

## 9. Références

- `crates/mika-agent/src/plan_callout.rs` — le lecteur unique et ses deux gardes
- `crates/mika-cli/src/commands/plan_callout.rs` — le canal et ses trois codes
- `crates/mika-agent/tests/fixtures/plan_callout_bodies/` — le corpus commun
- `crates/mika-agent/tests/plan_callout_parity.rs` — le lecteur Rust du corpus
- `skills/bundled/_shared/test-dispatch-lib.sh` — le lecteur bash et le scan R5
- `scripts/canonical-tokens.tsv` — l'inventaire des sites de match (mika#2201)
- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`

# Migrer `dispatch-lib.sh` — la doctrine, bornée à ce que la phase 1 a établi

**Ticket parent :** [mika#2194](https://github.com/senara-solutions/mika/issues/2194).
**État :** phase 1 livrée (`_extract_plan_path` → `crates/mika-agent/src/plan_callout.rs`).
Les phases 2 à 4 sont nommées ici et **aucune n'est ouverte**.

Ce document ne décrit pas un programme souhaitable : il décrit ce qu'**un** maillon
a établi, pour que le suivant ne redécouvre pas les mêmes bornes. Tout ce qui n'a
pas été mesuré en phase 1 est marqué comme tel.

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

`_extract_plan_path` est appelée **une fois par dispatch**, d'où
`mika plan-callout`. Le raisonnement ne repose sur aucun chiffre mesuré : le coût
réel d'un démarrage de `mika` n'était pas mesurable depuis le bac à sable de
dispatch (`mika --version` y est refusé par la permission-policy), et c'est une
**sonde post-déploiement** (§ 7, S3), pas un acquis.

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
nommait trois ; la quatrième a été trouvée en implémentant :

| lecteur | tolérance | question posée | statut |
|---|---|---|---|
| `plan_callout.rs` | strict, ancré, fences au choix de l'appelant | quel chemin, sous quelle forme ? | **le lecteur unique** (phase 1) |
| `executor::check_grooming_markers` | sous-chaîne `docs/plans/`, non ancrée | ce corps a-t-il un plan ? (routage) | intouché, B1 |
| `dispatch-lib::_committed_plan_on_branch` | strict, ancré (`sed`) | ce plan est-il committé sur la branche ? | **phase 2** |
| `dispatch-lib::_set_up_worktree` | littéral (`grep -qE`) | callout périmé ? (porte de dispatch) | **phase 2** |
| `milestone_manager::reader::plan_callout_present` | `contains("**Plan:**")` **sans** le préfixe `> - `, ou `contains("docs/plans/")` | ce corps porte-t-il un callout ? (booléen, LECTURE seule) | intouché, hors jeton par sa forme |

La phase 1 a unifié l'**implémentation** de deux de ces lecteurs. Elle n'a unifié
**aucune tolérance**, et un maillon futur qui écraserait les cinq vers la plus
stricte fermerait un défaut en ouvrant son miroir.

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

## 5. Le scan anti-copie : un inventaire fermé, pas un zéro

Le scan de mika#2120 couvrait **une** fonction. La phase 1 l'étend à tout
`dispatch-lib.sh` — et **pas** sous la forme d'un zéro, ce qui est la deuxième
correction que l'exécution impose au plan.

Un scan exigeant « aucun motif du callout » serait **rouge à la naissance**, parce
que la phase 2 garde délibérément deux lecteurs (§ 3, B1). Un lint rouge le jour où
il naît se fait désarmer, et la régression qu'il existe pour attraper passe ensuite
dans le bruit.

**Un inventaire fermé est plus fort qu'un zéro impossible** : il refuse un
*troisième* lecteur — la classe visée — et il rougit aussi si l'un des deux
disparaît sans que le TSV bouge. Les assertions en vigueur :

1. exactement **deux** lecteurs dans `dispatch-lib.sh` ;
2. ce sont bien `_committed_plan_on_branch` et `_set_up_worktree`, nommés (un
   inventaire qui compte sans nommer laisse un intrus passer pour un attendu) ;
3. `_extract_plan_path` n'en est **plus** un.

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
| **1** | `_extract_plan_path` → Rust, de bout en bout, **plus cette doctrine** | — | **livrée** |
| 2 | les deux autres lecteurs du même jeton : `_committed_plan_on_branch`, `_set_up_worktree` | phase 1 mergée | ticket à ouvrir |
| 3 | `_parse_disposition` / `_parse_verdict` | décider ce que devient le canal de retour par fichier (`$_DISPOSITION_FUZZY_FILE`) | ticket à ouvrir |
| 4 | la glue d'orchestration (bwrap / git / gh / trap / worktree / callback) → Python | un maillon dont la nature est de la **glue** et non une décision | ticket à ouvrir |

**Aucune de ces phases n'est ouverte par la phase 1**, et c'est délibéré : les
ouvrir avant que la phase 1 ait établi sa doctrine serait instruire sans mesure.

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

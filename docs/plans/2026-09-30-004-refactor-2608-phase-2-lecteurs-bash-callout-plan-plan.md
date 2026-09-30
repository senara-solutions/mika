# Phase 2 — les deux lecteurs bash du callout `Plan`, et la décision sur le 4e (mika#2608)

**Ticket :** senara-solutions/mika#2608 — labels `enhancement`, `p2-normal`,
`phase:2`, `dispatch:loop`, `loop-substrate`. Parent : mika#2194.

> **Classe :** la même qu'en phase 1 — une décision écrite deux fois, dans deux
> langues, jamais comparée sur les mêmes entrées.
> **Précondition remplie :** PR #2607 mergée le 2026-09-30 à 19:01:01Z
> (`db476b5d`), § Phasage l. 124 du plan de phase 1.
> **Précédent direct :** mika#2158 (`grooming_marker.rs`) — un lecteur, ses
> appelants n'en portent aucune copie, un scan de source refuse le second.

---

## Constat

La phase 1 a migré `_extract_plan_path` et **laissé deux lecteurs bash du même
jeton en place, délibérément**, en figeant leur inventaire (« exactement deux,
nommés ») plutôt qu'un zéro qui aurait été rouge à la naissance. Elle a aussi
trouvé un **quatrième** lecteur en implémentant, et l'a nommé sans le trancher.
Cette phase fait les deux : elle migre les deux lecteurs bash et elle **décide**
sur le quatrième.

Six mesures, relevées sur l'arbre au 2026-09-30 (`db476b5d`), déplacent le ticket
avant de l'exécuter. Les nommer est un livrable, pas une note de bas de page.

### M1 — Le maillon migrable est l'EXPRESSION d'extraction, jamais la fonction

Le critère C2 de `docs/architecture/dispatch-lib-migration.md` (« un seul
appelant ») écarterait les deux sites si on le lisait sur les fonctions :
`_committed_plan_on_branch` a **deux** appelants de production
(`dispatch-lib.sh:3214` dans `_set_up_worktree`, `dispatch-lib.sh:9544` dans la
composition du `RESULT` d'un groom non convergé) et quinze appels de test ; et
`_set_up_worktree` est la fonction la moins pure du fichier.

C'est la même lecture qu'en phase 1, et elle y était déjà correcte : cette
phase-là n'a pas migré `_detect_plan_on_branch` (impure, un `-f`, un
`ENTRY_COMMAND`), elle a migré `_extract_plan_path` — **12 lignes, entrée →
sortie**. Les deux maillons de la phase 2 sont du même ordre :

| site | expression | lignes | pureté |
|---|---|---|---|
| 1 | `dispatch-lib.sh:2673-2674` — le `sed` d'extraction dans `_committed_plan_on_branch` | 2 | entrée → sortie |
| 2 | `dispatch-lib.sh:3245` — le `grep -qE` de présence dans `_set_up_worktree` | 1 | entrée → booléen |

C1 et C3 sont satisfaits pour les deux : le Rust est **déjà** le lecteur (la
phase 1 l'a fait), et chacune des deux expressions a exactement un site.

### M2 — Les deux sites ne posent pas la même question, et le canal répond aux deux sans être élargi

| site | question | ce qu'il fait du résultat |
|---|---|---|
| 1 | « quel chemin ce corps nomme-t-il ? » | le résout contre l'arbre de la branche, le lie au ticket (mika#2034), **refuse le grooming** s'il tient |
| 2 | « ce corps porte-t-il un callout ? » | **une ligne de journal**, `dispatch_gate_groom_allowed_stale_callout` (mika#2012 U4) — le grooming procède dans les deux cas |

`mika plan-callout` répond aux deux **sans une ligne de code neuve** : son code
`0` porte le chemin (site 1) et **la distinction 0/1 est la réponse booléenne**
(site 2). Aucune sous-commande nouvelle, aucun drapeau nouveau — et c'est ce qui
tranche le point 3 du ticket (§ D7).

### M3 — Le site 1 ne porte PAS la même tolérance que l'ancien `_extract_plan_path`

Le ticket les décrit tous deux comme « strict, ancré ». C'est faux sur trois
axes, et l'un des trois est un **faux positif latent** que la migration ferme.

`sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p'` contre
`(?m)^> - \*\*Plan:\*\* `((?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+)`` :

| axe | site 1 aujourd'hui | lecteur unique | sens | conséquence sur la porte |
|---|---|---|---|---|
| littéral `docs/plans/` | **non exigé** | exigé | resserre | **ferme un faux positif latent** — voir ci-dessous |
| espaces après `**Plan:**` | ` *` (zéro ou plus) | exactement un | resserre | la porte ne tire plus ⇒ grooming procède (fail-open) |
| `../docs/plans/`, `a/b/docs/plans/` | acceptés (ne résolvent pas) | refusés | resserre | fail-open |
| backtick fermant | **exigé** (le `sed` le borne) | exigé | identique | — (le site 1 ne portait pas la divergence n°2 de la phase 1) |
| blocs clôturés | lus (aucun strip) | `Keep` ⇒ lus | identique | — |
| capture multi-ligne | impossible (`sed` ligne à ligne) | possible, bornée par le canal | **population neuve** | rc ≥ 2 ⇒ fail-open **et nommé** (§ D3) |
| normalisation | `${plan_path#"${repo}/"}` — ce dépôt seulement | premier segment quelconque | **élargit** | § D2 |
| candidats essayés | deux (`raw`, strip-`$repo`) | un (`normalized`) | rétrécit | § D2 |

**Le faux positif latent, et il est réel.** Aujourd'hui un corps portant
`> - **Plan:** \`README.md\`` extrait `README.md` ; `cat-file -t` rend `blob`
(tout dépôt a un README) ; la liaison mika#2034 lit son en-tête, n'y trouve
aucun `issue:`, et **ne réfute pas** — sa contrat est la réfutation, jamais la
confirmation. Donc la porte **tire** et le ticket est bloqué en `already_groomed`
de façon permanente. C'est exactement la classe que mika#2034 a ouverte pour
fermer (#1887, #2026, tous deux « stranded »), à un autre site. Exiger
`docs/plans/` la ferme, et c'est un effet **collatéral de la migration**, pas un
objectif — il est nommé plutôt que découvert.

### M4 — Le 4e lecteur : un appel au lecteur strict y serait PROUVABLEMENT inerte

`milestone_manager::reader::plan_callout_present` a deux branches :

```rust
if t.starts_with("> ") && t.contains("**Plan:**") { return true; }   // branche 1
if t.starts_with("> ") && t.contains("docs/plans/") { return true; } // branche 2
```

La branche 1 est un **sur-ensemble strict** du lecteur unique : toute ligne que
le motif Rust accepte commence par `> - **Plan:** ` — donc, `trim_start` appliqué,
commence par `> ` et contient `**Plan:**`. **Il n'existe aucun corps que le
lecteur strict accepte et que la branche 1 refuse.**

Conséquence décisive : la réparation tentante — « appeler `plan_callout` en
première branche et garder les branches lâches en repli » — ajouterait un appel
qui **ne peut jamais décider de rien**. Ce serait une unification apparente et
inerte, c'est-à-dire la classe mika#2205 appliquée à un chemin de code : verte,
plausible, et mesurant zéro. C'est ce qui tranche l'AC2 (§ D6), et c'est un
argument **structurel** plutôt qu'une préférence.

### M5 — La rouille résiduelle du TSV est nommée par le survey lui-même, et la phase 2 en est la première occurrence réelle

`scripts/canonical-tokens-survey.sh --check` ne vérifie **qu'une direction** —
tout site strict de l'arbre est déclaré — et son commentaire nomme ce qu'il
laisse passer, mot pour mot :

> « Residual rot it does not catch, named rather than hidden: a row whose file
> and symbol both still exist but which no longer reads the token. »

Et `canonical_tokens::tests::mika2201_every_declared_symbol_still_exists` vérifie
que `_committed_plan_on_branch` et `_set_up_worktree` **existent encore dans le
fichier** — ce qui restera vrai après la migration. Donc **aucune des deux gardes
mika#2201 ne rougirait** si les deux lignes du TSV survivaient à leurs sites de
match. L'AC5 n'est donc pas satisfaite par « les deux gardes sont vertes » : il
faut retirer les lignes **et** poser l'assertion qui refuse leur retour (§ R6).

La direction générale « toute ligne déclarée matche encore » est **refusée avec
sa mesure** par le survey : le TSV porte légitimement des lignes que le survey ne
peut pas produire (une alternance `(?i)`, un palier flou, un lecteur par
`contains`), et la comparer ainsi ferait rougir le build sur des lignes
**correctes** — 14 fausses contre 2 vraies, mesuré. D'où une assertion **bornée
au jeton**, pas une comparaison à deux sens.

### M6 — Le zéro de R5 devient atteignable, et c'est le gain principal

Le scan R5 de la phase 1 dit en toutes lettres pourquoi il compte deux au lieu de
zéro :

> « R6 garde DÉLIBÉRÉMENT deux lecteurs […] Un scan exigeant zéro serait donc
> rouge à la naissance, donc désarmé. »

La phase 2 retire ces deux lecteurs. **L'inventaire fermé devient le zéro que R5
demandait**, avec l'écrivain (`_write_canonical_callout`) comme population
d'anti-vacuité. Ce n'est pas un durcissement gratuit : c'est la forme que le scan
aurait prise si la phase 1 avait pu la prendre.

### M7 — `--raw` n'a aucun appelant de production, et c'est une donnée pour le point 3

Recherche exhaustive : le drapeau `--raw` de `mika plan-callout` est appelé par
**zéro** site de production. `auto_pull::plan_ownership` — le consommateur que son
doc-comment nomme — appelle `plan_callout` **directement** : il est dans le même
crate et n'a jamais eu besoin du canal. Il n'est exercé que par ses propres tests
unitaires.

Deux conséquences. (a) La **forme** du canal (une sous-commande, deux formes par
un drapeau) n'est elle-même qu'à moitié exercée en production — ce qui est une
donnée sur la maturité de cette forme, et donc sur le point 3 du ticket (§ D7).
(b) La phase 2 **aurait pu** lui donner son premier appelant, en préservant la
boucle à deux candidats du site 1 ; elle ne le fait pas, et pour une raison qui
est le cœur de la migration (§ D2).

---

## Décisions

### D1 — Les deux sites délèguent, par `_extract_plan_path`, sans nouvelle fonction

Les deux expressions sont remplacées par un appel à `_extract_plan_path`, la
fonction que la phase 1 a déjà rendue déléguante. Aucun nouvel helper, aucune
nouvelle sous-commande, aucun nouveau drapeau.

**Fan-in, et pourquoi ce n'est pas une violation de C2.** `_extract_plan_path`
passe de un à trois appelants. C2 (« un seul appelant ») est un critère de
**sélection** d'un maillon à migrer, pas une contrainte sur le fan-in d'un site de
délégation déjà migré : le coût de la preuve de parité croît avec le nombre de
maillons, pas avec le nombre d'appelants d'un lecteur unique — c'est le contraire
qui est vrai, chaque appelant de plus étant une copie de moins. Le doc-comment de
la fonction est mis à jour pour nommer ses trois appelants, sans quoi un futur
lecteur croirait C2 enfreint.

**Coût, chiffré.** Un dispatch dev-groom paie au plus **trois** démarrages de
`mika` : site 1 depuis `_set_up_worktree`, site 2 sur la branche `elif`, site 1
depuis la composition du `RESULT`. Contre un dispatch qui dure des minutes à des
heures, c'est la même arithmétique que le § 2 du document de migration — et la
sonde S3 de la phase 1 (le coût réel d'un démarrage, **non mesuré**) reste la
précondition : à 500 ms, trois démarrages font 1,5 s. Si S3 rend un chiffre
au-delà de ce que trois démarrages tolèrent, c'est le **placement dans `main.rs`**
qu'il faut réparer, jamais un cache — un cache sur un prédicat pur est une
seconde source de vérité.

### D2 — Un seul candidat, `normalized` ; la normalisation bash est RETIRÉE

Le site 1 essaie aujourd'hui deux candidats : `$plan_path` (brut) puis
`${plan_path#"${repo}/"}`. Après la bascule il n'en essaie **qu'un**, la forme
normalisée rendue par le canal.

**Pourquoi c'est le cœur de la migration et non une simplification de confort.**
`${plan_path#"${repo}/"}` **est** une seconde implémentation — partielle — de
`plan_callout::normalize`. La préserver (en appelant le canal deux fois, une fois
`--raw`) conserverait dans bash la moitié de la logique que la migration existe
pour retirer. La boucle à deux candidats n'existait que parce que bash devait
**deviner** la normalisation ; le lecteur la rend.

**Les deux changements de comportement, nommés.**

*(a) Le candidat brut disparaît — rétrécissement, sans population.* Pour la forme
nue, `raw == normalized` : identique. Pour la forme préfixée, `raw` vaut
`<segment>/docs/plans/x.md`, et `$sub_repo_dir` est **déjà** la racine du
sous-dépôt — `mika/docs/plans/x.md` y désignerait `…/mika/mika/docs/…`, qui
n'existe pas. Le candidat brut était donc mort pour la seule forme où il
différait. Le doc-comment de `_extract_plan_path` le dit déjà : « accepter le
préfixe sans retirer le segment ne ferait que déplacer l'échec d'un `grep` vide à
un `-f` faux ».

*(b) Un préfixe de dépôt ÉTRANGER résout désormais — élargissement, borné.* Un
ticket de `mika` dont le callout dit `mika-cloud/docs/plans/x.md` : aujourd'hui
ni `raw` ni le strip (qui ne retire que `mika/`) ne résolvent, donc la porte ne
tire pas ; après la bascule `normalized` vaut `docs/plans/x.md` et **peut**
résoudre. La borne est la liaison mika#2034 : l'en-tête du plan nommerait
`mika-cloud#220`, donc `_plan_header_refutes_issue` réfute et la porte ne tire
pas. Population pratique quasi vide (il faudrait qu'un fichier de ce nom existe
dans l'autre dépôt) et doublement gardée. Nommée plutôt que masquée, et couverte
par un cas de corpus.

### D3 — Le code `≥ 2` est fail-open aux deux sites, et il HÉRITE de la doctrine du site

`mika plan-callout` a trois codes. La phase 1 a créé la population `≥ 2` (« je
n'ai pas pu regarder ») : avant elle, le corps était en variable et il n'y avait
pas de fichier à ne pas pouvoir lire. Les deux sites la traitent ainsi :

| site | disposition sur `≥ 2` | surface du refus |
|---|---|---|
| 1 | `return 1` — le grooming procède | stderr **et** `_PLAN_CALLOUT_REFUSAL` |
| 2 | pas de ligne de diagnostic, mais le refus est dit | stderr **et** `_PLAN_CALLOUT_REFUSAL` |

**Le fail-open n'est pas choisi ici : il est déjà écrit au site 1**, verbatim,
`dispatch-lib.sh:2721-2723` — « That is this function's stated doctrine (*when in
doubt, returns 1 and grooming runs*), applied to its own failure modes ». Un
grooming de trop coûte un dispatch ; un ticket bloqué coûte le ticket. La
migration hérite de cette doctrine au lieu d'en inventer une.

**Pourquoi `_PLAN_CALLOUT_REFUSAL` et pas seulement stderr.** Le second appelant
du site 1 (`dispatch-lib.sh:9544`) redirige `2>/dev/null` — il silence
délibérément le stderr de la fonction. Sur ce chemin, la variable annexée au
`RESULT` par `_deliver_callback` (`dispatch-lib.sh:8814-8819`) est la **seule**
surface. Et le stderr d'avant-pilote est de toute façon structurellement perdu sur
un dispatch qui réussit (classe mika#2050, Signal M). La variable existe déjà et
son annexe est idempotente (`grep -qF` avant d'ajouter) ; **rien de neuf n'est
créé.**

### D4 — Le site 2 reste une ligne de journal, et son périmètre ne bouge pas

Le site 2 ne décide rien : le `elif` n'existe que pour séparer deux populations de
`grep` (mika#2012 U4). Après la bascille sa population se rétrécit — un callout
nommant autre chose que `docs/plans/` n'émet plus la ligne. C'est un changement
**de journal seulement**, et il va dans le sens de la justesse : un callout qui ne
nomme pas un plan n'est pas un « stale callout », c'est un callout malformé.

Ce site **ne devient pas une porte** et le `elif` garde sa place dans la chaîne.
Élargir sa disposition serait un changement de comportement moteur habillé en
migration.

### D5 — Aucune tolérance n'est unifiée par décret ; les deltas sont mesurés, tabulés, et corpusés

La borne B1 interdit d'écraser les quatre lecteurs vers le plus strict. Ce plan ne
le fait pas : il unifie l'**implémentation** de deux sites de plus, et chaque
delta de tolérance que la bascule produit est (a) tabulé en M3, (b) exercé par un
cas du corpus doré, (c) accompagné de sa direction (resserre / élargit) et de sa
conséquence sur la décision de la porte.

**Le sens des resserrements est fail-open pour cette porte**, sans exception : un
callout que le nouveau lecteur refuse fait que la porte **ne tire pas**, donc que
le grooming procède. Le seul élargissement (D2b) est borné par mika#2034. C'est ce
qui rend la bascule acceptable sur un chemin dont le faux positif bloque un ticket.

### D6 — AC2 : `plan_callout_present` GARDE sa tolérance, et la décision est structurelle

**Décision : non migré.** Quatre raisons, dans l'ordre de leur force.

1. **Un appel au lecteur strict serait prouvablement inerte** (M4) : la branche 1
   est un sur-ensemble strict du motif. Une unification qui ne peut rien décider
   est une unification apparente, et le repo a un nom pour ça.
2. **Ce n'est pas la même question.** Le lecteur unique répond « quel chemin, sous
   quelle forme ? » ; celui-ci répond « ce corps porte-t-il un callout ? », en
   booléen, sans rendre de chemin. Le migrer serait répondre à une autre question.
3. **La forme lâche est prescrite** : son doc-comment cite le contrat de cascade
   milestone (« Matches the loose form documented in the milestone-cascade
   contract »), et le repo porte un troisième prédicat de la même famille dans un
   prompt (`self-dev/system_prompt.md:526`, sous-chaîne `Plan: docs/plans/`).
4. **Le coût des deux erreurs penche dans le sens sûr.** Le manager est
   LECTURE-seule, zéro dispatch. `plan_present` alimente un **rapport à un humain**
   et `SubIssue::is_in_governed_progress`. Un resserrement produirait des **faux
   négatifs** sur un rapport dont l'objet est de dire à un humain quels
   sous-tickets restent à groomer — il lui dirait de groomer un ticket groomé.

**La décision est rendue structurelle**, pas seulement écrite : un test la pose
comme décision (motif `mika2120_divergence_is_still_open_and_this_test_pins_it`),
de sorte qu'un futur éditeur qui « harmonise » fait rougir un test au lieu de
changer un rapport en silence. Et la table de `plan_callout.rs:44-50` est mise à
jour dans les deux colonnes — `statut` (« intouché, décidé par mika#2608 ») et la
raison.

**La faiblesse réelle, nommée et NON corrigée.** La branche 2 (`> ` +
`contains("docs/plans/")`) lit une citation en prose comme un callout : un
blockquote disant « le plan vit dans `docs/plans/` » rend `plan_present = true`.
C'est un faux positif plausible, et **aucune mesure ne l'établit**. Suivi nommé,
avec sa précondition : un rapport dont `plan_present` est mesurément faux. Armer
un resserrement sur une population non mesurée est ce que mika#2520 refuse.

### D7 — Point 3 : la famille reste PLATE, avec son critère et son coût écrits

**Décision : `mika plan-callout` reste une sous-commande plate. Aucune famille
n'est nommée.**

L'évidence que la phase 2 apporte, et c'est ce que le ticket demandait : elle
**n'ajoute aucune sous-commande**. Les deux usages nouveaux sont le **même**
prédicat, réutilisé. La population des sous-commandes de prédicat est donc encore
**un**. Et sa forme n'est qu'à moitié exercée : `--raw` n'a aucun appelant de
production (M7).

Nommer une famille pour une population de un est le geste que ce dépôt a refusé
deux fois par écrit — mika#2329 (le fichier sentinelle paramétré par nom de scan,
livré avec un seul usage : « arrêter la revue QA n'est pas la même décision
qu'arrêter le feeder ») et mika#2201 (les allowlists livrées vides). Le refus
symétrique est d'appeler ça un report : ce n'en est pas un, c'est une décision
avec son critère.

**Critère de bascule, écrit ici pour que la phase 3 n'ait pas à le redériver :**
la **seconde sous-commande de prédicat distincte**. Tant qu'il n'y en a qu'une,
la famille n'a rien à grouper.

**Coût du renommage différé, mesuré et borné.** Le § 2 du document de migration
porte déjà la table d'asymétrie de déploiement. Un renommage ultérieur produit
l'état « dispatch-lib neuf + `mika` ancien » pendant une fenêtre de déploiement :
`mika` sort non-zéro sur une sous-commande inconnue ⇒ code `≥ 2` ⇒ refus
**bruyant et nommé** (`subcommand_error`), jamais un chemin vide. Cet état n'est
pas produit par `make deploy` (le binaire qui seede est celui qui est installé).
Donc le coût du report est une fenêtre lisible, et le coût de décider maintenant
serait une décision de namespace de tête prise sur un point de mesure — la façon
dont on dessine la mauvaise abstraction.

**L'alternative refusée, avec son argument, pour qu'elle ne soit pas re-proposée
à l'aveugle :** `mika predicate <nom>` dirait « ceci n'est pas pour vous » sur un
namespace de tête qui porte déjà 22 sous-commandes d'opérateur. C'est un vrai
argument, et il devient décisif au deuxième prédicat — pas au premier.

**`--raw` est conservé**, avec sa raison écrite au site : il est le miroir CLI des
deux champs de `PlanCallout`, et le retirer ferait du canal un miroir partiel du
lecteur — le prochain consommateur hors de `mika-agent` (un script, un dépôt
voisin) devrait le réintroduire. Son absence d'appelant de production est **dite**
au site plutôt que découverte, et la phase 2 **aurait pu** le lui donner : elle ne
le fait pas parce que l'utiliser conserverait le normaliseur bash (D2).

---

## Requirements

### R1 — Site 1 : `_committed_plan_on_branch` délègue

Remplacer le `sed` (`dispatch-lib.sh:2673-2674`) par un appel à
`_extract_plan_path`, avec les trois codes traités :

- `0` ⇒ `plan_path` est la forme normalisée ; **un seul candidat** (D2).
- `1` ⇒ `return 1` (aucun callout — comportement d'aujourd'hui).
- `≥ 2` ⇒ `return 1`, refus **nommé** sur stderr et dans
  `_PLAN_CALLOUT_REFUSAL` (D3).

La boucle `for candidate in …` devient un candidat unique ; le `${plan_path#…}`
disparaît. Le reste de la fonction — `git fetch` vers `refs/dispatch-gate/`,
`cat-file -t` = `blob`, la liaison mika#2034, `_plan_provenance` chez l'appelant —
est **intouché**.

Le doc-comment du site consigne les deux changements de comportement de D2 et le
faux positif latent fermé de M3.

### R2 — Site 2 : le `elif` de `_set_up_worktree` délègue

Remplacer `grep -qE -- '^> - \*\*Plan:\*\*' <<<"$ISSUE_BODY"` par un appel à
`_extract_plan_path` testé sur `rc == 0`. Sur `≥ 2` : pas de ligne de diagnostic,
refus dit (D3). Le texte de la ligne `dispatch_gate_groom_allowed_stale_callout`
et sa place dans la chaîne sont intouchés (D4).

### R3 — `_extract_plan_path` : doc-comment mis à jour, code inchangé

La fonction elle-même ne change pas d'une ligne. Son doc-comment nomme ses
**trois** appelants et pourquoi le fan-in n'enfreint pas C2 (D1).

### R4 — Le corpus doré couvre chaque site migré, en parité, avec un compte non nul (AC3)

Le corpus commun (`crates/mika-agent/tests/fixtures/plan_callout_bodies/`) gagne
les corps qui exercent les deltas de M3 et de D2, **sans toucher aux six corps
mesurés de mika#2120** (le `README.md` du corpus l'interdit, et pour la raison
qu'il écrit : un corpus rafraîchi efface les formes qu'il existe pour reconnaître).

Corps neufs, un par delta :

| fichier | delta exercé | `rc` attendu |
|---|---|---|
| `gate-non-plan-path.md` | `> - **Plan:** \`README.md\`` — le faux positif latent de M3 | `1` |
| `gate-double-space.md` | deux espaces après `**Plan:**` | `1` |
| `gate-foreign-prefix-resolves.md` | préfixe de dépôt étranger (D2b) | `0`, `normalized` sans préfixe |

La colonne `phase` de ces trois lignes vaut `both` : elles sont exercées par le
lecteur Rust **et** par le bloc bash. La colonne `parity` vaut `equal` — aucune de
ces formes ne dépend de la politique de fence.

**Parité par site, et c'est l'exigence propre à l'AC3.** Le bloc bash gagne, pour
chacun des deux sites migrés, un passage du corpus qui compare ce que le site rend
à ce que la colonne du TSV déclare — pas seulement ce que `_extract_plan_path`
rend, ce qui serait déjà couvert par la phase 1 et ne dirait rien des deux sites.
Chaque passage **nomme son compte de cas**, et **échoue sur zéro** (anti-vacuité).

Les quinze appels de test existants de `_committed_plan_on_branch`
(`test-dispatch-lib.sh:1093-1530`) deviennent dépendants du binaire `mika`. La
résolution du binaire (PATH → `target/debug` → `target/release`) et son `⊘ SKIP`
explicite, aujourd'hui locaux au bloc mika#2194 (`test-dispatch-lib.sh:9717`),
sont **hissés en tête de fichier** pour être partagés. Le SKIP reste **bruyant** :
un bloc de porte qui n'a pas pu s'armer ne doit pas se lire comme un vert
(mika#2149 troisième colonne). CI construit avant (`cargo test` précède
`make test-dispatch-lib` dans `ci.yml`), donc le SKIP est le cas dev, pas le cas CI.

### R5 — Un contrôle négatif par site migré (AC4)

Pour **chacun** des deux sites, une fixture de mutation qui rend le lecteur faux
et fait **rougir la parité**. Vue rouge à l'implémentation, une mutation à la fois
— une conjonction de termes ne se prouve pas en les neutralisant ensemble (leçon
mika#2277).

| site | mutation | ce que ça atteste |
|---|---|---|
| 1 | le `sed` d'avant la bascule réintroduit à la place de l'appel | la parité mesure bien **ce site**, pas seulement `_extract_plan_path` |
| 2 | le `grep -qE` d'avant la bascule réintroduit | idem |

Plus un **contrôle positif** par site : sur le corpus nominal, le site rend ce que
la colonne déclare. Sans lui, « la parité mesure le site » est indistinguable de
« la parité rougit sur tout ».

### R6 — Le TSV, et l'assertion qui refuse le retour des deux lignes (AC5)

Retirer les deux lignes de `scripts/canonical-tokens.tsv` :

```
> - **Plan:**	B	skills/bundled/_shared/dispatch-lib.sh::_committed_plan_on_branch	exact:line-anchored
> - **Plan:**	B	skills/bundled/_shared/dispatch-lib.sh::_set_up_worktree	exact:literal
```

Le commentaire de bloc du TSV (l. 150-167) est réécrit : les deux lecteurs ne sont
plus « la phase 2 », ils sont migrés ; le quatrième garde sa mention, avec sa
**décision** (D6) au lieu de « non tranché ».

**Et l'assertion, parce qu'aucune garde existante ne l'aurait tenue** (M5) :
`test-dispatch-lib.sh` asserte que le TSV porte **exactement une** ligne pour le
jeton `> - **Plan:**`, et que c'est
`crates/mika-agent/src/plan_callout.rs::PLAN_CALLOUT_RE`. Inventaire fermé borné
au jeton — pas une comparaison à deux sens, que le survey refuse avec sa mesure
(14 fausses lignes contre 2 vraies).

La fixture `scripts/fixtures/canonical-tokens/plan-espace-fr.md` cite les deux
sites retirés dans sa prose ; elle est mise à jour pour ne pas prescrire des sites
morts (classe mika#2050 : une prose qui cite un jeton retiré est ce qu'un futur
lecteur recopie).

### R7 — Le scan R5 passe de « exactement deux » à « exactement ZÉRO » (M6)

`test-dispatch-lib.sh:9795-9910` :

- l'assertion de compte devient **0** ;
- les deux assertions nommées (« le lecteur n°1 est `_committed_plan_on_branch` »,
  « n°2 est `_set_up_worktree` ») **s'inversent** : ces deux fonctions ne portent
  **aucun** site de lecture ;
- l'anti-vacuité de la population passe de `≥ 3` à `≥ 1`, et **nomme l'écrivain**
  (`_write_canonical_callout`) comme cette population — sans quoi un jeton
  entièrement disparu se lirait comme un scan propre ;
- les **trois contrôles de bonne foi sont conservés à l'identique** (PCRE échappé
  accusé, littéral lu accusé, écrivain hors population) : ce sont eux qui
  distinguent « zéro lecteur » de « le prédicat ne regarde rien », et le zéro les
  rend **plus** nécessaires, pas moins ;
- le commentaire de doctrine du scan est réécrit : il expliquait pourquoi le zéro
  était impossible ; il explique désormais pourquoi il est devenu atteignable.

Le scan Rust `mika2194_aucun_motif_de_callout_hors_de_ce_module` est **intouché** :
son aiguille est la forme **échappée** (`\*\*Plan:\*\*`), que ni `reader.rs` ni
aucun site bash ne porte. Il est vert aujourd'hui et le reste.

### R8 — La table des tolérances, la décision du 4e lecteur, et la doctrine

- `crates/mika-agent/src/plan_callout.rs:44-55` — la table passe de quatre lignes
  à quatre lignes **dont deux changent de statut** : `dispatch-lib::{…}` devient
  « délègue au lecteur unique (phase 2, mika#2608) » et
  `milestone_manager::reader::plan_callout_present` devient « tolérance
  **conservée, décidée** par mika#2608 » avec la raison de M4 en une phrase.
- `crates/mika-agent/src/milestone_manager/reader.rs:213-215` — le doc-comment de
  `plan_callout_present` porte la décision, sa raison structurelle (branche 1 =
  sur-ensemble strict ⇒ un appel strict serait inerte), et le suivi nommé avec sa
  précondition.
- `docs/architecture/dispatch-lib-migration.md` — § 3 (B1, la table des cinq
  tolérances), § 5 (le scan devenu un zéro), § 6 (la phase 2 passe de « ticket à
  ouvrir » à « livrée »), § 2 (la décision D7 et son critère de bascule).
- `crates/mika-agent/CLAUDE.md` § *Un seul lecteur de la preuve de grooming* —
  voisinage direct ; la migration des deux sites bash y est nommée.
- `crates/mika-cli/CLAUDE.md` § `mika plan-callout` — trois appelants, `--raw`
  sans appelant de production, la décision D7.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` | R1, R2, R3 — deux expressions remplacées, trois doc-comments |
| `skills/bundled/_shared/test-dispatch-lib.sh` | R4, R5, R6, R7 — parité par site, contrôles négatifs, assertion TSV, scan R5 inversé, résolution du binaire hissée |
| `crates/mika-agent/tests/fixtures/plan_callout_bodies/` | R4 — trois corps neufs, trois lignes de TSV, `README.md` |
| `crates/mika-agent/tests/plan_callout_parity.rs` | R4 — les trois cas neufs traversent le lecteur Rust |
| `crates/mika-agent/src/plan_callout.rs` | R8 — table des tolérances (doc seul) |
| `crates/mika-agent/src/milestone_manager/reader.rs` | R8, D6 — décision écrite ; **aucun changement de comportement** |
| `crates/mika-agent/src/milestone_manager/reader.rs` (tests) | D6 — le pin de décision |
| `scripts/canonical-tokens.tsv` | R6 — deux lignes retirées, commentaire de bloc réécrit |
| `scripts/fixtures/canonical-tokens/plan-espace-fr.md` | R6 — prose qui ne cite plus de sites morts |
| `docs/architecture/dispatch-lib-migration.md` | R8 |
| `crates/mika-agent/CLAUDE.md`, `crates/mika-cli/CLAUDE.md` | R8 |

**Aucun fichier de migration, aucune variable d'environnement, aucun réglage
déplacé, aucune valeur de seuil touchée.**

---

## Verification Contract

| # | ce qui est vérifié | comment | statut |
|---|---|---|---|
| V1 | le site 1 rend, sur le corpus, ce que la colonne déclare | `make test-dispatch-lib` — passage de parité site 1, compte non nul | automatisable |
| V2 | le site 2 rend le même booléen que `rc == 0` | idem, passage site 2 | automatisable |
| V3 | mutation du site 1 ⇒ parité rouge | fixture de mutation, **vue rouge** | automatisable |
| V4 | mutation du site 2 ⇒ parité rouge | idem | automatisable |
| V5 | contrôle positif par site : le corpus nominal passe | idem | automatisable |
| V6 | les quinze tests de porte existants passent inchangés | `make test-dispatch-lib` | automatisable |
| V7 | le scan R5 rend zéro, avec ses trois contrôles de bonne foi verts | idem | automatisable |
| V8 | le TSV porte exactement une ligne pour ce jeton | idem (R6) | automatisable |
| V9 | les deux gardes mika#2201 vertes | `bash scripts/canonical-tokens-survey.sh --check` + `cargo test -p mika-agent canonical_tokens` | automatisable |
| V10 | `plan_callout_present` est **inchangé** en comportement | ses tests existants + le pin de décision | automatisable |
| V11 | un appel strict-first y serait inerte (M4) | test asserta que la branche 1 accepte tout corps que le lecteur strict accepte, sur le corpus doré | automatisable |
| V12 | le faux positif latent de M3 est fermé | cas `gate-non-plan-path.md` : la porte **ne tire pas** | automatisable |
| V13 | le coût de trois démarrages de `mika` | **sonde post-déploiement S3**, héritée de la phase 1 et **non mesurable** depuis le bac à sable de dispatch (`mika --version` y est refusé par la permission-policy) | opérateur |
| V14 | le chemin nominal tient sur un vrai dispatch dev-groom | **sonde post-déploiement S1** | opérateur |

**V13 et V14 ne sont pas exécutables par l'implémenteur**, et c'est dit plutôt
que découvert : le bac à sable de dispatch ne monte pas `~/.mika/data/mika.db`, ne
peut pas chronométrer `mika`, et ne dispatche pas. Elles sont des gestes
d'opérateur sur l'hôte.

---

## Fire-Disposition

Ce plan livre des détecteurs : le scan R5 amendé (R7), l'assertion d'inventaire
fermé sur le TSV (R6), les passages de parité et leurs contrôles négatifs (R4,
R5), et le pin de décision de `plan_callout_present` (D6).

**Option retenue : (a) exception nommée en allowlist — avec ZÉRO entrée, et les
allowlists livrées vides et épinglées vides.**

Aucune violation existante ne survit à cette PR, et c'est vérifié terme par terme
plutôt qu'espéré :

| détecteur | population au moment de l'atterrissage | pourquoi il est vert |
|---|---|---|
| scan R5 (zéro lecteur bash) | les deux sites sont migrés **dans la même PR** | la PR retire la population que le scan refuse |
| anti-vacuité R5 (`≥ 1`) | `_write_canonical_callout` | l'écrivain reste, nommé |
| inventaire fermé du TSV (une ligne) | les deux lignes sont retirées **dans la même PR** | idem |
| parité par site | cas neufs | ils décrivent le comportement livré |
| pin `plan_callout_present` | il épingle le comportement **actuel** | rien n'y change |
| scan Rust mika#2194 (intouché) | son aiguille est la forme échappée | vert aujourd'hui, vert après |

**La résolution quand l'un d'eux tire est de router ou de retirer le site, jamais
d'ajouter une ligne d'exemption** (doctrine mika#2201 § D5/D6). Une allowlist née
vide est un emplacement où déposer la prochaine infraction (mika#2323) — les deux
nouvelles assertions n'en portent aucune, et un test le pose.

**Ni (b) ni (c).** (b) livrer désarmé n'a pas d'objet : un détecteur dont la
population est vidée par sa propre PR n'a rien à désarmer, et le livrer `#[ignore]`
serait la classe mika#2272 (« zéro était l'absence de mesure, pas la présence de
prudence »). (c) halte-et-remontée n'a pas d'objet non plus : aucune violation
n'exige un cadrage d'opérateur.

**Un détecteur explicitement REFUSÉ, avec sa mesure :** la comparaison à deux sens
« toute ligne du TSV matche encore un site ». Le survey la refuse en toutes
lettres — 14 lignes correctes accusées contre 2 vraies — et la réparation
naturelle serait d'élargir le survey jusqu'à ce qu'il les attrape. L'assertion de
R6 est bornée **au jeton**, ce qui attrape exactement la rouille que la phase 2
produit sans importer ce coût.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2608.

- [ ] **AC1.** Chacun des deux sites bash lit le callout via le canal Rust de la
      phase 1, ou bien une raison écrite au site dit pourquoi il reste en bash
      (critère B2).
- [ ] **AC2.** `reader.rs::plan_callout_present` a une décision écrite, avec sa
      raison : migré, ou tolérance conservée. Dans les deux cas, la table de
      `plan_callout.rs:44-50` est mise à jour.
- [ ] **AC3.** Le corpus doré `crates/mika-agent/tests/fixtures/plan_callout_bodies/`
      couvre chaque site migré, en parité bash ↔ Rust, avec un compte de cas non
      nul.
- [ ] **AC4.** Un contrôle négatif par site migré : la mutation du lecteur fait
      rougir la parité.
- [ ] **AC5.** Le TSV `canonical-tokens.tsv` est à jour, et les deux gardes
      mika#2201 sont vertes.

**Correspondance :** AC1 → R1, R2, D1, D2, D3 (les deux sites **migrent** ; la
branche « raison écrite pour rester en bash » n'est pas prise, et le refus de la
prendre est argumenté en M1 et D1). AC2 → D6, R8, V10, V11. AC3 → R4, V1, V2, V5.
AC4 → R5, V3, V4. AC5 → R6, V8, V9 — **et la lecture d'AC5 est corrigée par M5** :
« les deux gardes vertes » ne suffit pas, puisqu'aucune des deux n'aurait vu les
lignes devenues obsolètes ; d'où l'assertion de R6.

---

## Definition of Done

1. R1 → R8 livrés.
2. `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo fmt --all -- --check` verts.
3. `make test-dispatch-lib` vert, **sans SKIP** sur les blocs de porte et de
   parité (le binaire est construit par `cargo test` en amont dans CI).
4. `bash scripts/canonical-tokens-survey.sh --check` vert.
5. V3, V4 **vues rouges** à l'implémentation, une mutation à la fois, et le fait
   consigné dans le corps de PR.
6. V11 vert — la propriété de sur-ensemble de M4 est un test, pas un raisonnement
   dans un plan.
7. Le corps de PR porte : la table des deltas de tolérance de M3, les deux
   changements de comportement de D2 avec leur borne, la décision D6 avec sa
   raison structurelle, la décision D7 avec son critère de bascule, et le fait
   que V13/V14 sont des gestes d'opérateur non exécutés.
8. `docs/architecture/dispatch-lib-migration.md` § 6 : la phase 2 passe de
   « ticket à ouvrir » à « livrée », et la phase 3 garde sa précondition écrite.

---

## Surfaces opérateur

**Aucune surface neuve.** Les refus des deux sites migrés voyagent par les deux
surfaces que la phase 1 a déjà posées :

```bash
# 1. Un refus de lecture du callout sur un dispatch (régime attendu : VIDE)
grep -h 'REFUSED (plan-callout, mika#2194)' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail
```

```sql
-- 2. La surface qui SURVIT à un dispatch réussi — la seule sur le chemin du
--    second appelant, qui redirige 2>/dev/null
SELECT id, result FROM tasks
 WHERE result LIKE 'REFUSED (plan-callout, mika#2194)%' ORDER BY created_at DESC;
```

| motif | régime attendu | lecture |
|---|---|---|
| `body_file_unwritable` | **vide** | `mktemp` ou l'écriture a échoué — c'est l'**hôte**, pas le prédicat |
| `path_not_single_line` | **vide** | callout malformé (divergence n°3) ; la porte ne tire pas, le grooming procède |
| `subcommand_error` | **vide** | `dispatch-lib` neuf + `mika` ancien : c'est le **déploiement** qu'il faut établir (classe mika#2340) |

**Pourquoi le préfixe ne change pas.** Il reste `mika#2194` et non `mika#2608` :
c'est un format de fil que le `CLAUDE.md` de `mika-cli` publie déjà en requête
SQL, et le renommer couperait en deux une population que l'opérateur compte. Les
deux phases écrivent dans la même surface parce qu'elles refusent la même chose.

**Aucun compteur, aucun événement de journal neuf, aucune ligne
`audit_events`** — et c'est délibéré : ce travail retire une implémentation, il
n'observe pas un phénomène neuf.

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable, non négociable.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir — sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier.

**S1 — le chemin nominal tient (premier dispatch dev-groom après déploiement).**
Un ticket **déjà groomé** doit continuer d'être refusé `already_groomed` avec sa
provenance, et un ticket **non groomé** doit continuer à se faire groomer.
*Halte 1 — un ticket groomé n'est plus refusé et repart en groom :* ne pas
retoucher le prédicat par réflexe. Lire d'abord la requête SQL ci-dessus — un
`subcommand_error` dit que les deux moitiés ne sont pas en phase, et c'est le
remède. Puis vérifier que le callout nomme bien un `docs/plans/` : c'est le
resserrement de M3, et sur un callout malformé le nouveau comportement est
**correct** (le grooming procède) même s'il diffère.

**S2 — le faux positif latent est fermé (30 jours).** Aucun ticket bloqué en
`already_groomed` sur un callout nommant autre chose qu'un plan. La population
est celle de M3, et elle était **invisible** avant : un ticket bloqué ne produit
aucune ligne, il cesse simplement d'avancer. La sonde est donc une lecture des
tickets `ready` qui n'ont jamais dispatché :
`SELECT after_value, count(*) FROM audit_events WHERE tool_name = 'auto_pull_exclusion' GROUP BY 1;`
*Halte 2 — `not_groomed` monte :* le lecteur s'est resserré au-delà de M3 malgré
B1. **Revert d'abord, diagnostic ensuite** — une promotion perdue est la panne
qui a coûté quinze heures de boucle à mika#2120, et elle est silencieuse.

**S3 — le coût est ce qu'on a supposé (première semaine).** Le chiffre que ni la
phase 1 ni la phase 2 n'ont pu produire depuis le bac à sable : chronométrer
`mika plan-callout` sur l'hôte, et le multiplier par **trois** (D1).
*Halte 3 — au-delà de ~500 ms par appel :* le chemin court ne l'est pas.
**Réparer le placement dans `main.rs`, jamais mettre en cache** — un cache sur un
prédicat pur est une seconde source de vérité, c'est-à-dire la duplication qu'on
vient de retirer.

**S4 — contrôle négatif du refus (7 jours).** La requête SQL ci-dessus reste
vide.
*Halte 4 — une occurrence :* c'est un faux positif et il coûte un dispatch
entier. Le motif dit lequel des trois remèdes s'applique (table ci-dessus) ; aucun
des trois n'est un réglage de seuil.

**S5 — le rapport du manager ne bouge pas (7 jours).** `plan_present` n'a pas
changé de sens : un rapport Phase 1 sur le même milestone doit classer les mêmes
sous-tickets de la même façon.
*Halte 5 — il change :* `plan_callout_present` a été touché malgré D6. C'est la
régression que le pin de décision existe pour empêcher ; lire ce test **avant**
de toucher au rapport.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro dispatch ne
prouve rien : il faut qu'un dispatch dev-groom ait eu lieu depuis le déploiement.
*Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
(mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne clôt pas mika#2194.** C'est la phase **2 sur quatre**, deux expressions
  sur 119 fonctions. Les phases 3 (`_parse_disposition` / `_parse_verdict`) et 4
  (la glue → Python) gardent leurs préconditions écrites.
- **Il ne ferme les trois classes de panne de parsing que pour CE jeton.** Les 117
  autres fonctions y restent exposées, et `dispatch-lib.sh` **ne rétrécit pas** :
  il perd ~3 lignes d'expression et en gagne ~40 (les trois codes, les refus
  nommés, la doctrine en commentaire). Ce qui change n'est pas sa taille, c'est
  qu'une décision de moins y est prise — la même phrase qu'en phase 1, et elle
  reste vraie.
- **Il ne tranche aucune des trois divergences de la phase 1.** Les fences
  (n°1), le backtick fermant (n°2) et la capture multi-ligne (n°3) restent
  nommées, testées comme divergentes, et rattachées à leurs suivis. Le site 1 ne
  portait de toute façon ni la n°2 ni la n°3 (M3), donc la bascule n'en déplace
  aucune.
- **Il ne corrige pas `plan_callout_present`.** Sa branche 2 lit toujours une
  citation en prose comme un callout ; c'est un suivi avec une précondition
  (D6), pas un oubli.
- **Il n'ajoute aucun instrument.** Les seules surfaces sont celles de la phase 1,
  et **leur silence ne prouve rien tant que personne n'exécute S1 à S5** : sur un
  prédicat appelé trois fois par dispatch, l'absence de refus peut vouloir dire
  que rien n'a mal tourné, ou que rien n'a tourné.

---

## Hors périmètre, délibérément

- **`executor::check_grooming_markers`** — la sous-chaîne `docs/plans/` non
  ancrée, deuxième ligne de la table. Intouchée : le doc-comment de
  `auto_pull::is_groomed` interdit son resserrement en toutes lettres (« Ne le
  resserrez pas pour “harmoniser” les deux »), et la borne B1 le reprend.
- **Le prédicat de bypass de `self-dev/system_prompt.md:526`** (sous-chaîne
  `Plan: docs/plans/`) — cinquième tolérance, dans un **prompt**, portant sur la
  **réponse d'un modèle** et non sur un corps d'issue. Hors du jeton par son sujet
  et hors du code par sa nature ; nommé ici pour qu'un futur maillon ne le croie
  ni oublié ni autorisé à l'absorber.
- **Le retrait de `--raw`** — conservé avec sa raison (D7). Son absence
  d'appelant de production est **dite** au site.
- **Une famille `mika predicate <nom>`** — refusée avec son critère de bascule et
  le coût du report (D7).
- **La comparaison à deux sens du TSV** — refusée avec la mesure du survey
  (Fire-Disposition).
- **La phase 3** (`_parse_disposition` / `_parse_verdict`) — écartée sur mesure
  par la phase 1 : elles rendent **deux** valeurs dont la seconde par un fichier
  (`$_DISPOSITION_FUZZY_FILE`), donc l'effet de bord est dans le contrat, et leur
  palier 2 est une **paraphrase** (classe A du TSV) dont la migration est un
  arbitrage de **tolérance**, pas un portage. Précondition inchangée : décider ce
  que devient ce canal de retour.
- **La fragilité de sourcing relevée et non corrigée par la phase 1** — le motif
  `source "$DISPATCH_LIB" 2>/dev/null || true` sur 215 sites, dont un sourcing
  raté laisserait des assertions vertes sur des fonctions inexistantes. Le terme
  d'anti-vacuité couvre les blocs de parité, pas les 213 autres. Suivi inchangé,
  avec sa précondition : une mesure montrant qu'un sourcing raté a produit un vert.

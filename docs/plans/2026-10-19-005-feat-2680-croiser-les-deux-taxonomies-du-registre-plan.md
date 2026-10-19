# Vol 5 de mika#2680 — la soustraction que (g) demande n'est pas dans l'agrégat

## État des lieux, vérifié sur l'arbre servi

Les quatre vols antérieurs sont mergés, et `HEAD` vaut `origin/main` exactement :

| vol | ce qu'il a livré | PR | commit sur `main` |
|---|---|---|---|
| 1 | le registre des tours webhook (`webhook_turn_charged`) | #2705 | `185362cc` |
| 2 | le lecteur `scripts/measure-webhook-turn-cost` + le protocole | #2747 | `034698b2` |
| 3 | la classe (e) `pr_closed_no_loop_task` | #2764 | `c71ea5e6` |
| 4 | le terme de relâche (`Release`, dix variantes) | #2812 | `f14e69d9` |

L'état des quatre classes candidates, tel que `docs/eval/mika-2680/README.md` § 9
le fixe après le vol 4 : **(e)** implémentée, **(f)** tranchée — il n'y a pas de
classe (f) à écrire —, **(h)** posée à l'opérateur, **(g)** bloquée par la
mesure 3. La mesure 3 est un geste opérateur par construction (§ 2 bis) : double
mur, `/var/log/mika` hors du bac à sable `bwrap` et le script refusé en amont par
la permission-policy de la session dispatchée.

**Quatre dispatches ont déjà brûlé sur ce ticket** (commentaires des 2026-10-19
00:26Z, 00:52Z, 01:03Z et 04:14Z), et les trois premiers n'ont rien écrit parce
que le plan qu'ils portaient était déjà mergé. C'est exactement la classe de
dépense que ce ticket chiffre. Ce vol-ci ne reconduit pas ce motif : il livre du
code, et il le livre **sans toucher à la mesure 3 ni en dépendre** — la propriété
qui a rendu le vol 4 livrable, et que le § 9 nomme en toutes lettres (« *le vol 4
ne la tente pas et n'en dépend pas — c'est la propriété qui l'a rendu
livrable* »).

---

## Décision 1 — la soustraction que le § 9 annonce n'est pas calculable sur l'agrégat

Le § 9 du protocole, paragraphe *« Ce que le vol 4 change pour (g) : la
soustraction devient lisible »*, affirme :

> Avant lui, `by_agent_trigger` donnait le plafond de (g) — les
> `check_suite_pr_branch` payées côté mika-qa — et rien ne disait lesquelles (b)
> aurait écartées si elle avait vu la tête. **`charged_by_release` le dit** […]
> Le chiffre de (g) après retrait n'est donc plus une estimation mais une
> soustraction : plafond moins les seaux que (b) revendique.

**Il ne le dit pas.** Relu sur le code servi, `charged_by_release` est **plat** :

```
# scripts/measure-webhook-turn-cost:344-348
charged_by_release:
  ($turns | group_by(.release // "absent_pre_field")
          | map({ key:   (.[0].release // "absent_pre_field"),
                  value: length })
          | from_entries)
```

`group_by(.release)` **seul** : un seau par terme, agrégé sur **tous les agents et
tous les déclencheurs** de la fenêtre. Son voisin `by_agent_trigger`
(l. 323-332) groupe par `[agent_id, trigger]` et ne porte **aucune** ventilation
par terme de relâche.

Les deux opérandes de la soustraction sont donc sur deux axes qui ne se coupent
pas. Sur une fenêtre réelle, `mika-dev` reçoit aussi le fan-out
`check_suite_pr_branch` (mika#1711), et `pr_review`, `issue_comment`,
`pr_closed` relâchent aussi sous `text_touched` et `state_unreadable` : le seau
`head_not_previously_decided` mélange tout cela. Rien dans l'agrégat ne dit
combien de ces relâches appartiennent à la cellule `mika-qa` ×
`check_suite_pr_branch`, qui est la seule cellule dont (g) parle.

**Et le per-tour la porte déjà.** Les lignes de `turns-*.jsonl` portent
`agent_id`, `trigger` et `release` côte à côte (l. 286-295). La soustraction est
donc dérivable — **à la main, par un `jq` ad hoc sur le JSONL**. C'est
précisément le rattachement manuel que le § 1 du protocole donne comme la
première des deux limites que le vol 1 était venu lever (« *Le rattachement était
humain* »). L'instrument a regagné ce défaut sur la seule classe dont
l'existence en dépend.

**Ce vol croise les deux taxonomies dans l'agrégat**, au grain exact de la
décision : `(agent_id, trigger, release)`.

---

## Décision 2 — l'unité de la soustraction est le token, pas le tour

C'est le point de conception le plus important du vol, et la mutation qu'un
relecteur laisse passer.

`charged_by_release` porte `value: length` — **un compte de tours**.
`by_agent_trigger` porte `input_tokens`, `cache_read_tokens`, `llm_calls` — **un
coût**. Or tout ce ticket chiffre en tokens d'entrée, de la première mesure à la
dernière : 193 k, 279 k, 741 k, 1,30 M. Un plafond en tokens moins un
soustrahend en tours n'est pas une soustraction, c'est une comparaison de deux
unités — l'erreur que
`docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`
nomme, et qui a coûté trois vols morts au plafond le 2026-10-14.

Le seau croisé porte donc **les trois valeurs de coût**, pas seulement le
compte : `turns`, `input_tokens`, `cache_read_tokens`, `llm_calls`, de la même
forme et dans le même ordre que son voisin `by_agent_trigger`, et trié comme lui
par `- .input_tokens`.

La forme la plus naturelle à écrire — copier le `value: length` du voisin du
dessus — rendrait un agrégat qui a l'air complet, qui passe toute assertion
portant sur des comptes, et sur lequel (g) reste indécidable. La batterie du
vol 4 (P2) ne la verrait pas rouge : elle n'assère que des comptes. **C'est la
mutation que le contrôle positif de la phase 3 doit tuer**, et il la tue en
épinglant une cellule dont le coût en tokens n'est pas proportionnel à son compte
de tours.

---

## Décision 3 — croiser, jamais remplacer : trois seaux, pas un

Les deux seaux existants restent **inchangés, octet pour octet**.

Le § 4 du protocole les justifie l'un et l'autre : « *Les deux taxonomies ne se
dérivent pas l'une de l'autre, et c'est pour cela qu'il y a deux seaux.
`skipped_by_class` nomme ce qu'une porte a reconnu ; `charged_by_release` nomme
la porte qui a relâché.* » Le seau croisé ne remplace ni l'un ni l'autre : il
répond à une troisième question — *quelle porte a relâché, dans quelle cellule,
pour combien* — que ni l'un ni l'autre ne pose.

Trois raisons de ne pas fusionner :

1. **Le plat répond à une question qui reste posée.** « Combien de tours une
   panne de lecture a-t-elle coûtés sur toute la fenêtre » se lit sur
   `charged_by_release` et n'a pas besoin du grain fin. Le doc-comment de
   `Release::StateUnreadable` le dit déjà pour les cinq sites de
   `state_unreadable` : la question est « combien », jamais « laquelle ».
2. **Deux assertions du vol 4 en dépendent** — P2 (l. 151) et N7 (l. 163) du
   harnais.
   Les faire porter sur une autre forme serait réécrire une preuve au lieu d'en
   ajouter une.
3. **Le § 4 du README est un contrat de lecture**, et son tableau des deux
   moitiés devient faux si l'une change de forme. Il **gagne une ligne**, il ne
   se réécrit pas.

**Le nom** : `by_agent_trigger_release`, sibling de `by_agent_trigger`, dont il
est le raffinement d'un cran. Un nom qui dit son grain, par ses trois clés dans
l'ordre où elles groupent.

**L'absence garde son nom dans le nouveau seau aussi.** Une ligne écrite par un
binaire antérieur au vol 4 n'a pas de `release` ; elle tombe sous
`absent_pre_field`, **jamais** sous une clé `null` qui se lirait comme un terme
de relâche nommé ainsi (motif mika#2331, et c'est ce que N7 tient sur le seau
plat). Le repli est le même littéral, au même endroit de la chaîne `group_by`.

---

## Décision 4 — la moitié écartée a le même défaut, et c'est une lecture qui change une décision

`skipped_by_class` (l. 333-336) groupe par `class` seule :

```
skipped_by_class:
  ($skipped | group_by(.class // "null")
            | map({ key: (.[0].class // "null"), value: length })
            | from_entries),
```

Or l'`agent_id` **est déjà sur le fil** — `skip_turn` l'émet
(`webhook_prefilter.rs:974`, `agent_id = %db.agent_id()`) — et **déjà projeté**
par le lecteur (l. 195). L'agrégat le jette.

Ce n'est pas une symétrie cosmétique, c'est la vérification de la prémisse de
(g). (g) est une assertion sur **mika-qa**. Ce qui la bloque est la soustraction
de ce que (b) écarte déjà, et la mesure 2 affirmait qu'aucune `check_suite` de
branche de PR n'avait été vue depuis le déploiement de `e14ace7f`. Mais (b)
décide par agent — chaque agent a sa propre base et son propre audit, et
`head_decided_and_notified` se lit dans l'audit de **cet** agent. Un
`skipped_by_class.green_check_suite_head_decided` de 7 peut donc être 7 écarts
côté mika-dev et **zéro** côté mika-qa, auquel cas (b) ne couvre rien du côté qui
intéresse (g) et le net de (g) égale son plafond. Un 7 réparti 4/3 dit l'inverse.
**Les deux lectures mènent à deux décisions différentes, et l'agrégat actuel ne
les distingue pas.**

Même discipline que la décision 3 : `skipped_by_class` est conservé tel quel —
N3 (l. 116) l'assère —, et un `skipped_by_agent_class` est ajouté à côté.
Celui-ci porte un **compte** et c'est juste : un tour écarté n'a pas eu lieu,
donc il n'a aucun coût en tokens à porter, et lui en inventer un ferait entrer
dans le tableau un coût que personne n'a payé. La différence d'unité entre les
deux nouveaux seaux n'est pas une incohérence : c'est la différence entre une
population payée et une population évitée, que le § 4 interdit déjà de
soustraire l'une de l'autre.

---

## Décision 5 — ce que ce vol ne touche pas, et pourquoi chaque renvoi est différent

- **La mesure 3 : ni tentée, ni requise.** Le double mur du § 2 bis est intact et
  ce vol ne cherche pas à le lever. Élargir le bind `bwrap` donnerait à chaque
  session dispatchée l'écriture sur `/var/log/mika`, surface de diagnostic
  d'incident (mika#2041) — le § 2 bis juge que « *ce n'est pas un arbitrage que
  la mesure justifie* », et ce vol ne le relitige pas. La permission-policy est
  le second mur, et il ne tomberait pas avec le premier. **Le vol est testé
  intégralement sur les corpora gelés**, hors bac à sable comme dedans.
- **Le moteur : aucune ligne.** `webhook_prefilter.rs` n'est pas touché. Les
  trois champs dont le croisement a besoin — `agent_id`, `trigger`, `release` —
  sont déjà émis et déjà projetés. Ce vol est un vol de **lecteur**, et le chemin
  servi ne change pas.
- **La classe (g) : pas implémentée.** Ce vol rend sa soustraction *lisible*, il
  ne la *fait* pas et ne prétend pas la débloquer. La mesure 3 reste sa
  condition d'existence. C'est le même rapport que le vol 4 entretenait avec
  elle : fournir l'opérande qui manque, pas remplacer la mesure.
- **La classe (h) : toujours posée à l'opérateur.** Son blocage n'est pas un
  chiffre (§ 9 : « *le seul blocage qui ne soit pas un chiffre* »), donc aucun
  seau ne le lève. Le § 7 tranche : une classe dont la question reste ouverte ne
  s'implémente pas.
- **Le résidu de (f) : ticket à part.** Le § 9 l'a réduit à une décision AC2 de
  mika#2675 (élargir la population au `pr_review` sans verdict) et à trois
  chemins qui ne sont pas des filtres. Rien ici.

---

## Décision 6 — une correction de document : l'étape CI existe déjà

Deux endroits affirment que le câblage CI du harnais reste à poser :

- `docs/eval/mika-2680/README.md` § 8 : « *L'étape CI qui appelle cette cible
  reste à poser* […] la branche du vol 2 ne peut pas porter
  `.github/workflows/ci.yml* ».
- Le corps du ticket, § *Vol 3* : « *Hors périmètre : le câblage en CI de `make
  test-measure-webhook-turn-cost`* […] *décision opérateur en attente, voisine
  de mika#2752* ».

**Les deux sont périmés, et c'est mika#2752 qui les a périmés** — la voisine que
le ticket nommait. `.github/workflows/ci.yml:111-112` porte l'étape :

```yaml
- name: Webhook-turn register reader — the turn is a SUM (mika#2680)
  run: make test-measure-webhook-turn-cost
```

posée par `8ac93d62` (PR #2769). La cible est donc gatante sur chaque PR, ce qui
est aussi la raison pour laquelle `test-measure-webhook-turn-cost` **n'est pas**
dans `scripts/suite-wiring.tsv` : le lint bidirectionnel de mika#2761 refuserait
une entrée pour une suite que CI atteint.

Cette correction tient en deux lignes de document, et elle appartient à ce vol
plutôt qu'à un autre pour une raison précise : c'est une affirmation périmée dans
le document que **tout dispatch suivant sur ce ticket lira en premier**. Un
pilote qui la croit part chercher une étape à poser avec un jeton qui n'a pas le
scope `workflow` — un tour pour rien, sur le ticket qui chiffre les tours pour
rien.

---

## Phases

### Phase 1 — le seau croisé de la moitié payée

Dans `scripts/measure-webhook-turn-cost`, fonction `aggregate()`, **après**
`by_agent_trigger` (l. 332) et **avant** `skipped_by_class` (l. 333) — l'ordre
des clés du JSON suit l'ordre de raffinement : plafond, puis plafond ventilé,
puis la moitié écartée, puis le plat.

```
by_agent_trigger_release:
  ($turns
   | group_by([(.agent_id // "null"), (.trigger // "null"),
               (.release  // "absent_pre_field")])
   | map({ agent_id:          .[0].agent_id,
           trigger:           .[0].trigger,
           release:           (.[0].release // "absent_pre_field"),
           turns:             length,
           input_tokens:      sum_num(.input_tokens),
           cache_read_tokens: sum_num(.cache_read_tokens),
           llm_calls:         sum_num(.llm_calls) })
   | sort_by(- .input_tokens)),
```

Trois points porteurs, chacun à défendre par un commentaire au-dessus du bras :

- **`sum_num`, pas `length` seul** — décision 2. Les trois valeurs de coût, dans
  l'ordre du voisin.
- **Le repli `absent_pre_field` est dans la clé de groupement ET dans la
  projection**, comme sur le seau plat (l. 345-346). Dans la clé seule, deux
  lignes sans `release` grouperaient ensemble et la projection rendrait `null` :
  le seau porterait le bon compte sous un nom faux.
- **`agent_id` et `trigger` gardent leur repli `"null"`**, identique au voisin :
  une cellule dont l'agent est absent du fil n'est pas une cellule
  `absent_pre_field` — l'absence du champ `release` et l'absence du champ
  `agent_id` sont deux absences différentes, et un repli partagé les
  confondrait.

### Phase 2 — le seau croisé de la moitié écartée

Juste après `skipped_by_class`, qui reste inchangé :

```
skipped_by_agent_class:
  ($skipped
   | group_by([(.agent_id // "null"), (.class // "null")])
   | map({ agent_id: .[0].agent_id,
           class:    (.[0].class // "null"),
           skipped:  length })
   | sort_by(- .skipped)),
```

La clé de compte s'appelle `skipped`, pas `turns` : **ces événements n'ont pas eu
de tour**, et réutiliser `turns` inviterait la somme que le § 4 interdit. Aucune
valeur de coût, par la décision 4.

### Phase 3 — les assertions, et le corpus gelé qu'elles demandent

Un nouveau corpus gelé synthétique,
`crates/mika-agent/tests/fixtures/mika2680/cross_tab_corpus.jsonl`, construit
pour que **le seau plat soit ambigu et le seau croisé ne le soit pas** :

- deux tours payés sous **le même** `release` (`head_not_previously_decided`),
  sur deux cellules différentes — `mika-qa` × `check_suite_pr_branch` et
  `mika-dev` × `check_suite_pr_branch` : le seau plat rend `{...: 2}` et ne sait
  pas les séparer ; le croisé doit rendre **deux** cellules de 1 ;
- les deux cellules portent **le même nombre de tours et des coûts en tokens très
  différents** (p. ex. 900 contre 120, par plusieurs lignes `turn_usage` dont la
  continuation `step = u32::MAX`, comme `req-g-1` du corpus principal) : un
  lecteur qui n'émettrait que des comptes est indistinguable d'un lecteur correct
  sur un contrôle de comptes, et devient rouge ici ;
- un troisième tour payé **sans champ `release`**, sur une troisième cellule,
  pour le repli nommé ;
- deux événements `webhook_prefilter_skipped` de **même classe** et **d'agents
  différents** : le seau plat rend `{green_check_suite_head_decided: 2}`, le
  croisé doit rendre deux entrées de 1 — la lecture même de la décision 4.

Les fixtures restent **gelées et synthétiques**, jamais échantillonnées sur
`$MIKA_SPIRIT_LOG_FILE` : l'en-tête du harnais le justifie déjà (« *une suite qui
lirait le corpus vivant changerait de verdict à mesure qu'il grossit, et ne
tournerait pas sur une machine qui n'a jamais dispatché* »).

Dans `scripts/test-measure-webhook-turn-cost.sh`, une section `-- 4.` sur le
modèle de la § 3 du vol 4 — un positif, trois négatifs, **chacun sur sa propre
entrée** (leçon mika#2277, que l'en-tête du harnais porte déjà) :

| id | ce qu'il tient | ce qu'il tue |
|---|---|---|
| **P3** | la cellule `mika-qa` × `check_suite_pr_branch` × `head_not_previously_decided` porte `turns: 1` **et** `input_tokens: 900` | le seau croisé écrit en comptes seuls (décision 2) ; le croisement sur deux clés au lieu de trois |
| **N8** | un `release` absent fait une cellule `absent_pre_field`, et `by_agent_trigger_release` n'a **aucune** cellule dont `release` vaut `null` | le repli posé dans la clé de groupement seule, ou oublié (motif mika#2331) |
| **N9** | `by_agent_trigger`, `charged_by_release` et `skipped_by_class` rendent sur ce corpus **exactement** ce qu'ils rendaient avant le vol | une fusion déguisée en ajout (décision 3) |
| **N10** | `skipped_by_agent_class` sépare deux agents de même classe, et ne porte **aucune** clé de coût en tokens | la symétrie paresseuse qui donnerait un coût à un tour qui n'a pas eu lieu (décision 4) |

N9 est le négatif qui compte le plus, et il est assérable parce que les deux
seaux conservés sont des fonctions pures du même corpus : il les compare à des
littéraux, pas à une exécution antérieure.

### Phase 4 — les documents, là où le prochain lecteur les trouvera

1. **L'en-tête du script** — la section « DEUX TAXONOMIES, DEUX SEAUX » (l. 36-43)
   devient trois seaux et dit **pourquoi le croisé ne remplace ni l'un ni
   l'autre**, plus la phrase d'unité de la décision 2. L'en-tête est servi par sa
   forme (`--help`, l. 114), donc l'ajout ne décale rien.
2. **`docs/eval/mika-2680/README.md` § 4** — le tableau des moitiés gagne une
   ligne `by_agent_trigger_release`, et le paragraphe *« Les deux taxonomies ne
   se dérivent pas l'une de l'autre »* gagne la précision qui manquait : elles ne
   se dérivent pas, mais elles **se croisent**, et c'est le croisement qui porte
   la décision de (g).
3. **`docs/eval/mika-2680/README.md` § 9, paragraphe (g)** — la phrase
   « *`charged_by_release` le dit* » est **corrigée** : elle le dit au grain de la
   fenêtre, pas au grain de la cellule, et c'est `by_agent_trigger_release` qui
   le dit au grain de la décision. Le § 9 est l'endroit où un lecteur vient
   chercher l'état de (g) ; y laisser l'affirmation du vol 4 serait laisser un
   document promettre un opérande que l'agrégat ne porte pas.
4. **`docs/eval/mika-2680/README.md` § 8** — la correction de la décision 6 :
   l'étape CI existe (`ci.yml:111-112`, `8ac93d62`, mika#2752), et c'est pourquoi
   la cible n'est pas dans `scripts/suite-wiring.tsv`.
5. **`docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`**
   — une ligne sur le grain de lecture du registre, à côté de la table des
   classes que les vols 3 et 4 y ont tenue.

---

## Definition of Done

1. `by_agent_trigger_release` est dans l'agrégat, groupé sur les **trois** clés,
   et porte `turns`, `input_tokens`, `cache_read_tokens`, `llm_calls`.
2. Son repli d'absence est `absent_pre_field`, présent **à la fois** dans la clé
   de groupement et dans la projection ; aucune clé `null` n'apparaît sur l'axe
   `release`.
3. `skipped_by_agent_class` est dans l'agrégat, groupé sur `(agent_id, class)`,
   et ne porte **aucune** valeur de coût en tokens.
4. `by_agent_trigger`, `charged_by_release` et `skipped_by_class` sont
   **inchangés** — même forme, mêmes clés, mêmes valeurs sur les corpora
   existants.
5. `webhook_prefilter.rs` n'est pas modifié : aucun champ nouveau sur le fil.
6. `cross_tab_corpus.jsonl` existe, est gelé, synthétique, et porte les cinq
   propriétés de la phase 3 (même `release` sur deux cellules ; comptes égaux et
   coûts en tokens différents ; un tour sans `release` ; deux écarts de même
   classe sur deux agents).
7. Les quatre assertions P3, N8, N9, N10 sont présentes, chacune sur son entrée,
   et vertes.
8. Suite verte : `bash scripts/test-measure-webhook-turn-cost.sh` — **13 passées,
   0 échouée** : les neuf assertions antérieures (P1, N1–N6, P2, N7 — mesurées à
   « 9 passées, 0 échouées » sur l'arbre servi) **plus** les quatre nouvelles. `cargo test -p mika-agent --lib webhook_prefilter` inchangé par
   construction (le moteur n'est pas touché) et rejoué pour le dire.
9. Les cinq documents de la phase 4 portent l'ajout ou la correction qui leur
   revient, dont la correction du § 9 (la phrase du vol 4 sur
   `charged_by_release`) et celle du § 8 (l'étape CI existe).
10. Le hors-périmètre de la décision 5 est nommé dans le corps de PR et **non
    livré** : mesure 3, classes (g) et (h), résidu de (f), élargissement du bind
    `bwrap`, permission-policy.
11. `scripts/suite-wiring.tsv` n'est **pas** modifié : la cible est atteinte par
    `ci.yml`, et une entrée pour elle ferait rougir le lint bidirectionnel de
    mika#2761.

---

## Acceptance criteria

Le corps de mika#2680 n'a pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés du § *Critère d'acceptation de la mesure* du ticket, des
§ 4, 7 et 9 du protocole, et des décisions de ce plan.

1. **AC1 — la soustraction de (g) est une lecture, plus une dérivation à la
   main.** À partir du seul `summary-*.json`, le plafond de (g) et sa ventilation
   par terme de relâche se lisent sur la cellule `mika-qa` ×
   `check_suite_pr_branch`, sans ouvrir `turns-*.jsonl` ni écrire de `jq`.
2. **AC2 — la soustraction est dans l'unité du ticket.** Les valeurs ventilées
   sont des tokens d'entrée, comparables aux 193 k / 279 k / 741 k / 1,30 M des
   mesures antérieures, et non des comptes de tours.
3. **AC3 — l'absence reste nommée.** Une ligne écrite par un binaire antérieur
   au vol 4 est comptée sous `absent_pre_field` dans le seau croisé comme dans le
   seau plat ; aucune clé `null` ni `unknown` n'apparaît sur l'axe `release`.
4. **AC4 — rien n'est retiré.** Les trois seaux antérieurs rendent sur les
   corpora gelés exactement ce qu'ils rendaient avant ce vol, et les onze
   assertions antérieures passent sans modification de leur texte.
5. **AC5 — la prémisse de (g) est vérifiable par agent.** `skipped_by_agent_class`
   permet de dire si (b) a écarté quoi que ce soit **côté mika-qa**, séparément
   de mika-dev.
6. **AC6 — le chemin servi est intact.** Aucune modification de
   `crates/mika-agent/src/server/webhook_prefilter.rs` ni d'aucun fichier de
   `crates/`, hors la fixture.
7. **AC7 — le vol ne dépend pas de la mesure 3.** Toutes les assertions tournent
   sur les corpora gelés, dans le bac à sable d'un pilote comme hors de lui, et
   aucune n'exige `/var/log/mika/server.log`.
8. **AC8 — les documents ne promettent plus ce que l'agrégat ne porte pas.** Le
   § 9 ne dit plus que `charged_by_release` porte la soustraction de (g), et le
   § 8 ne dit plus que l'étape CI reste à poser.

---

## Fire-Disposition

Ce vol livre un détecteur : les quatre assertions P3, N8, N9, N10 de
`scripts/test-measure-webhook-turn-cost.sh`, dont le chemin de succès est « aucune
violation trouvée ». La section est donc requise.

**Option retenue : (a), exception nommée en allowlist — et l'ensemble des
exceptions est vide par construction.**

Le détail d'implémentation, et pourquoi le vide est ici un résultat et non un
oubli :

- **La population que ces assertions jugent est entièrement créée par ce commit.**
  Les quatre portent sur la sortie du lecteur appliquée à des corpora **gelés et
  synthétiques**, dont l'un (`cross_tab_corpus.jsonl`) est écrit dans ce même
  commit. Il n'existe aucune donnée antérieure à grandfather : une entrée
  d'allowlist n'aurait rien à nommer.
- **Et il n'existe pas d'artefact committé qui pourrait les violer
  rétroactivement.** `docs/eval/mika-2680/` ne contient que `README.md` — aucun
  `summary-*.json` ni `turns-*.jsonl` n'y a jamais été committé, la mesure 3
  n'ayant jamais été prise. Le fait est vérifiable en une commande
  (`ls docs/eval/mika-2680/`), et c'est la vérification à faire **avant** la
  phase 3 : si un artefact y était apparu entre-temps, il faudrait décider s'il
  est relu par les assertions, et cette décision reviendrait à l'opérateur.
- **Aucune assertion auto-nettoyante n'est livrée**, et c'est la conséquence
  honnête de ce qui précède : une garde contre une exception périmée n'a rien à
  garder quand l'ensemble des exceptions est vide. En inventer une serait livrer
  un détecteur dont le seul rôle serait de surveiller un ensemble vide — le
  chemin de succès vide que mika#2205 condamne.

Pourquoi pas les deux autres options :

- **(b) livrer désarmé** serait faux : les quatre assertions sont vertes dès
  l'atterrissage, parce que le code qu'elles jugent atterrit dans le même commit.
  Un `#[ignore]` sur une assertion qui passe est une assertion que personne ne
  rallumera.
- **(c) halte-et-remontée** serait faux aussi : il n'y a pas de cadrage à
  demander. La halte de ce ticket — la mesure 3 — est déjà remontée à
  l'opérateur par le § 2 bis et par quatre commentaires, et ce vol est
  précisément construit pour ne pas en dépendre (décision 5, AC7).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/measure-webhook-turn-cost` (deux seaux + en-tête) | 42 |
| `scripts/test-measure-webhook-turn-cost.sh` (section 4 : P3, N8, N9, N10) | 78 |
| `crates/mika-agent/tests/fixtures/mika2680/cross_tab_corpus.jsonl` | 12 |

Total estimé : 132 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi il tient largement.** Les trois champs dont
le croisement a besoin sont déjà émis par le moteur et déjà projetés par le
lecteur : il n'y a ni variante d'énumération à ajouter, ni site d'appel à
migrer, ni `match` à rendre exhaustif — c'est-à-dire aucun des trois postes qui
ont fait du vol 4 un vol de 363 lignes. Le travail est de deux chaînes `jq` dans
une fonction `aggregate()` déjà écrite, d'un corpus de douze lignes, et d'une
section de harnais sur le modèle exact de la précédente. Le volume est à 31 % de
la borne par vol et à 13 % du seuil par PR ; aucun découpage n'est requis, et en
inventer un coûterait plus en fin de pipeline qu'il n'économiserait en lignes.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `scripts/measure-webhook-turn-cost` | les deux seaux croisés, dans `aggregate()` ; la section d'en-tête |
| `scripts/test-measure-webhook-turn-cost.sh` | section `-- 4.` : P3, N8, N9, N10 |
| `crates/mika-agent/tests/fixtures/mika2680/cross_tab_corpus.jsonl` | **nouveau** — corpus gelé synthétique |
| `docs/eval/mika-2680/README.md` | § 4 (la ligne du troisième seau), § 8 (l'étape CI existe), § 9 (la correction de (g)) |
| `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md` | une ligne sur le grain de lecture |

**Non touchés, et c'est une propriété du vol** :
`crates/mika-agent/src/server/webhook_prefilter.rs` (aucun champ nouveau sur le
fil), `crates/mika-agent/src/server/mod.rs`, `.github/workflows/` (l'étape
existe), `scripts/suite-wiring.tsv` (une entrée y ferait rougir le lint de
mika#2761), `skills/bundled/_shared/dispatch-lib.sh` (le bind `bwrap` n'est pas
élargi), `skills/bundled/permission-policy/` (le second mur n'est pas levé).

---

## Renvois explicites

- **La mesure 3** reste un geste opérateur, double mur inchangé. La commande
  prête à coller est au § 3 du protocole ; le corps de PR la reporte, avec la
  précision neuve qu'elle doit désormais porter `by_agent_trigger_release` pour
  que (g) se lise.
- **La classe (g)** reste bloquée par la mesure 3. Ce vol lui livre l'opérande
  au bon grain et dans la bonne unité ; il ne la débloque pas et ne le prétend
  pas.
- **La classe (h)** reste posée à l'opérateur : son blocage n'est pas un chiffre.
- **Le résidu de (f)** reste une décision AC2 de mika#2675 et trois chemins qui
  ne sont pas des filtres — ticket à part, voisin de #2679.
- **L'élargissement du bind `bwrap`** et **la permission-policy** du script : les
  deux murs du § 2 bis, hors périmètre, décision opérateur.

Tracked in: senara-solutions/mika#2680 — les classes (g) et (h), la mesure 3, et
le résidu de (f)

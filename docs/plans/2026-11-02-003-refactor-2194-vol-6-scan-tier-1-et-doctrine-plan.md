# mika#2194 — vol 6 : le scan de tier 1, et la doctrine qui cesse de promettre

> **Type :** refactor (clôture de la phase 3)
> **Ticket :** [senara-solutions/mika#2194](https://github.com/senara-solutions/mika/issues/2194)
> **Branche :** `refactor/2194/dispatch-lib-sortir-la-logique-du-bash-d`
> **Vol précédent :** #3016 (vol 5 — le canal et la bascule), mergé le 2026-11-02.

---

## 0. Le vol, en une phrase

Le vol 5 a retiré les tables de tier 1 et de tier 2 de `dispatch-lib.sh` ; ce vol
pose le **scan qui empêche leur retour** (AC4) et fait passer la doctrine de
« la phase 3 viendra » à « la phase 3 est livrée » (AC6). C'est le dernier vol de
la phase 3, et il est mergeable seul.

Il était nommé, chiffré et planché par le § 6 du plan du vol 5
(`docs/plans/2026-11-02-002-refactor-2194-vol-5-canal-et-bascule-plan.md`,
l. 485-520) : « Renvoyé au vol 6 (≈ 60 lignes comptées, mergeable seul) ».
Ce plan n'ouvre rien de neuf — il exécute ce renvoi, avec une correction de
dessin que la mesure impose (§ 2).

---

## 1. L'état mesuré, qui n'est plus celui du corps du ticket

Le corps de mika#2194 décrit la tranche de phase 3 **entière**, telle qu'elle a
été réveillée le 2026-11-02. Quatre de ses six critères sont déjà clos par des
PR mergées. L'état réel, mesuré sur `HEAD = 22492f3b6` :

| critère | état | par |
|---|---|---|
| AC1 — parité dorée sur les cinq tiers | **clos** | vol 4 (#3014) |
| AC2 — non-régression `post-switch` | **clos** | vol 5 (#3016) |
| AC3 — code `≥2`, refus nommé aux deux polarités | **clos** | vol 5 (#3016) |
| AC4 — scan anti-copie | **partiel** : tier 2 clos par le vol 5, **tier 1 dû ici** | ce vol |
| AC5 — famille de sous-commandes plate | **clos** | PR de doctrine #3015 |
| AC6 — doctrine : phase 3 à « livrée » | **dû ici** | ce vol |

Les faits qui portent le reste de ce plan, chacun mesuré et non repris d'un
plan antérieur :

- **Le canal et les deux enveloppes sont livrés.** `_parse_disposition` et
  `_parse_verdict` délèguent à `mika arch-disposition` : la sentinelle de
  bascule (`test-dispatch-lib.sh:15277-15296`) compte `M3_SWITCHED == 2`.
- **Les deux fonctions fuzzy n'existent plus** (`test-dispatch-lib.sh:11442-11445`,
  `declare -f` → `no`), et les 293 lignes de tables sont parties.
- **Le scan de tier 2 existe**, armé à zéro :
  `test-dispatch-lib.sh:11309-11444`, helper `m2194_t2_offenders()` (11364-11373),
  dix aiguilles (11339-11350), une exception par valeur (11362), contrôle
  positif sur fixture (11385-11392), anti-vacuité sur la révision pre-switch
  `593a745c5` (11408-11437).
- **Le scan de tier 1 n'existe pas.** Rien, dans l'arbre, n'assure l'absence
  d'un lecteur de `Disposition:` / `Verdict:` dans `dispatch-lib.sh`. Les trois
  choses qui en ont l'air n'en sont pas :
  `scripts/check-canonical-tokens.sh:181` porte sur la **typographie** (règle L4,
  espace insécable) ; `test-dispatch-lib.sh:14074` est une assertion
  **comportementale** sur `$RESULT` ; `test-dispatch-lib.sh:14429-14432` scanne
  une **source Rust**.
- **`m2194_read_sites()` n'est pas paramétrée.**
  `test-dispatch-lib.sh:11192-11196` lit son aiguille dans la globale
  `$M2194_NEEDLE` (l. 11189), en dur sur le motif du callout. Ses neuf appels
  (11212, 11221, 11226, 11232, 11241, 11247, 11255) sont tous sur cette aiguille.

---

## 2. La mesure qui refait le dessin d'AC4 — et pourquoi ce plan ne l'applique pas tel quel

**AC4 tel qu'écrit dans le corps du ticket est littéralement faux**, et il faut le
dire avant de l'implémenter : « Aucun motif de tier 1 ou 2 ne subsiste dans
`dispatch-lib.sh` ». Il en subsiste un, **délibérément**.

`skills/bundled/_shared/dispatch-lib.sh:11060-11062` :

```sh
_second_pass_explicit_iterate() {
    grep -qE '^[[:space:]>*_`-]*(Verdict|Disposition)[*_`]*:[[:space:]*_`]*ITERATE([^[:alnum:]_]|$)'
}
```

C'est un lecteur de forme tier 1, en bash, avec son outil de lecture, sur une
ligne de commande — et il est **conservé par décision** : mika#2641 AC1 exclut
nommément `ITERATE` du périmètre de relance, parce que `_parse_verdict` n'a aucun
bras ITERATE et qu'une seconde passe qui écrit `Disposition: ITERATE` est une
objection lisible et non un illisible. Sa doctrine est au commentaire 11047-11057.
Deux appelants de production : 11278 et 11421.

Ce lecteur n'est donc **pas** une violation à retirer. Il est la seule exception,
et le scan doit la nommer plutôt que la rencontrer.

### La mesure qui tranche la forme de l'aiguille

Le piège 3 du § 5 de la doctrine
(`docs/architecture/dispatch-lib-migration.md:453-456`) est celui qui mord ici :
« L'aiguille doit couvrir le littéral **et** sa forme échappée. La première
version du scan ne portait que la première, donc elle n'aurait **pas** vu revenir
le `grep -oP` qu'elle existe pour refuser. »

Le motif de tier 1 a lui aussi deux écritures, et elles ne se recouvrent pas :

| écriture | exemple de source | mesure HEAD | mesure pre-switch `593a745c5` |
|---|---|---|---|
| littéral contigu | `grep -oE 'Disposition:[[:space:]]*(READY\|ITERATE\|ESCALATE)'` | **0** | **4** (2 `Disposition:` + 2 `Verdict:`) |
| alternation en source | `(Verdict\|Disposition)[*_`]*:` | **1** | **1** |

Toutes quatre mesurées sur les lignes de commande, commentaires retirés,
discriminant `M2194_READER` appliqué.

Deux conséquences, et c'est tout le dessin du scan :

1. **Une aiguille au seul littéral contigu serait aveugle à la seconde
   écriture.** Elle vaudrait zéro aujourd'hui et resterait verte si un tier 1
   revenait sous forme tolérante à l'emphase — exactement le défaut que le piège 3
   documente comme déjà payé une fois.
2. **Une aiguille qui couvre les deux écritures compte 1 aujourd'hui**, et ce 1
   est le lecteur mika#2641. Il devient l'**exception nommée** de la
   Fire-Disposition, et non une raison de rétrécir l'aiguille.

C'est la différence entre un prédicat sur la **forme** et un prédicat sur une
**valeur** : mika#2309 refuse le second, et mika#1804 l'a redit sur les couleurs
du landing. Rétrécir l'aiguille pour atteindre zéro serait choisir le nombre
plutôt que la conséquence — la faute même que le bearing de Prime du 2026-11-02
a corrigée sur AC5.

### Le contrôle positif n'est pas optionnel ici

Les quatre lecteurs de tier 1 retirés par le vol 5 sont à la révision
`593a745c5` (lignes 9965, 9974, 10047, 10064 de cette révision). C'est la
population non vide du scan, et c'est ce qui distingue « zéro lecteur » de
« le prédicat ne regarde rien » — la classe mika#2205, et l'exigence
d'anti-vacuité du § 4 de la doctrine (l. 282-297).

### Trois faux positifs écartés, chacun par mesure et non par allowlist

- **Les onze écrivains du motif.** `dispatch-lib.sh` écrit `Disposition:` et
  `Verdict:` en onze endroits — les deux prompts de relance (11003-11007,
  11037-11040), la ligne de `_escalate_groom` (10664), deux
  `GROOM_LOOP_FAILURE_REASON` (11317, 11447) et un `_groom_warn` (11480). Aucune
  de ces lignes ne porte d'outil de lecture : le discriminant `M2194_READER` les
  écarte **par définition de la population**, pas par exemption. C'est le piège 2
  du § 5 (l. 446-452).
- **Les dix commentaires de doctrine** (9845, 10633, 10670, 10689, 10754, 10967,
  11025, 11050, 11110, 11200) qui **citent** le motif : écartés par le retrait
  des commentaires en tête de pipeline, piège 1 du § 5 (l. 442-445).
- **`Fire-Disposition`** (`dispatch-lib.sh:10330`, `grep -qF -- 'Fire-Disposition'`)
  est un vrai lecteur, avec outil de lecture, portant le mot `Disposition` — mais
  **sans `:` contigu**. L'aiguille exige le `:` ; relâcher au mot nu le ferait
  rougir. Mesuré, et c'est la raison pour laquelle l'aiguille garde son `:`.

### Sensibilité à la casse : tranchée, et dite non porteuse

Le scan de tier 2 utilise `grep -ciF` (insensible). Pour le tier 1, les deux
variantes valent **0** une fois le discriminant appliqué : l'insensible ne change
aucun verdict aujourd'hui. Sans le discriminant, en revanche, elle compte 4
artefacts (`${_BRANCH_REUSE_VERDICT:-}` l. 4256, de la prose l. 6927,
`${disposition:-UNPARSED}` l. 11176, `${verdict:-UNPARSED}` l. 11269).

**Décision : sensible à la casse.** Les deux jetons sont canoniquement
capitalisés (`scripts/canonical-tokens.tsv:257-258`), et une aiguille qui
ramasse `verdict=` en minuscule élargit la population vers des affectations de
variable sans élargir la classe de défaut. Le plan dit que ce choix ne porte
rien aujourd'hui, pour qu'un futur lecteur ne le croie pas mesuré sur un écart.

---

## 3. Les livrables

### R1 — `m2194_read_sites()` est paramétrée par son aiguille

`skills/bundled/_shared/test-dispatch-lib.sh:11192-11196`. La fonction prend son
aiguille en `$1` au lieu de lire la globale `$M2194_NEEDLE`, et ses neuf appels
existants passent `"$M2194_NEEDLE"` explicitement. Le corps, le discriminant et
la sémantique de retour sont inchangés.

**Pourquoi, et pas une copie.** Le plan du vol 5 l'avait déjà posé
(l. 499-500 : « paramétrée par son aiguille à cette occasion, plutôt que
recopiée »). Un second scan qui recopierait ce pipeline de trois `grep` créerait
deux lecteurs de la même question — précisément la classe que ce ticket migre.

**Garde de non-régression :** les neuf assertions du scan de callout
(11212-11255) passent sans modification de leurs attentes. C'est le contrôle que
la paramétrisation n'a rien changé.

### R2 — Le scan anti-copie des motifs de tier 1

Un bloc nouveau dans `test-dispatch-lib.sh`, posé immédiatement après le scan de
tier 2 (après l. 11445), pour que les deux se lisent ensemble.

**L'aiguille** — les deux écritures, en une alternation ERE :

```sh
M2194_T1_NEEDLE='Disposition:|Verdict:|\(Verdict\|Disposition\)'
```

**La population** — les lignes de commande de `dispatch-lib.sh` (la globale
`$M2194_CMD_LINES`, l. 11182, réutilisée telle quelle) intersectées avec le
discriminant `$M2194_READER` (l. 11190), via `m2194_read_sites` de R1.

**L'exception nommée** — une, par valeur, avec son ticket :

```sh
# mika#2641 — lecteur de tier 1 CONSERVÉ PAR DÉCISION (dispatch-lib.sh:11060).
# AC1 de mika#2641 exclut nommément ITERATE : une seconde passe qui écrit
# `Disposition: ITERATE` est une objection lisible, pas un illisible. Ce lecteur
# reste en bash jusqu'à ce que mika#2641 soit migré — et ce jour-là, l'assertion
# de péremption ci-dessous rougit.
M2194_T1_EXCEPTION='_second_pass_explicit_iterate'
```

L'exception est retirée de la population en excluant le **corps de la fonction
nommée**, et non une ligne littérale : c'est l'identité stable à travers un
reformatage de la regex.

**Les quatre assertions.**

1. **L'assertion zéro.** Population moins l'exception → `0`.
2. **Péremption de l'exception** (auto-nettoyante). Le corps de
   `_second_pass_explicit_iterate` existe et porte **exactement un** lecteur de
   l'aiguille. Le jour où mika#2641 migre, l'exception devient stale et ce
   contrôle rougit — l'exception ne survit pas à sa raison.
3. **Contrôle de bonne foi positif, sur fixture.** Les quatre formes de lecteur
   retirées par le vol 5, écrites comme fixture dans le corps du test
   (`grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)'` et ses trois
   sœurs) : le prédicat doit en compter **4**. Un prédicat qui en compte 0
   mesurerait une population vide.
4. **Anti-vacuité sur révision immuable.** Sur `593a745c5` (la révision
   pre-switch, la même que celle qu'utilise déjà le scan de tier 2 en l. 11408),
   le prédicat compte **5** : les 4 littéraux retirés plus le lecteur mika#2641
   qui existait déjà. Même mécanique que 11408-11437 : `git cat-file -e` d'abord,
   **SKIP bruyant** si le clone est shallow, et `grep -c` plutôt que `grep -q`
   (la note mesurée en 11421-11427 : sous `pipefail`, `-q` rend 141 par SIGPIPE
   et le `if` lit faux).

**Les deux contrôles négatifs qui épinglent le dessin du § 2.** Un lecteur de
tier 1 écrit sous forme tolérante à l'emphase est compté par l'aiguille — le
contrôle le prouve sur fixture. Et une ligne portant `Fire-Disposition` n'est
**pas** comptée. Ces deux-là sont ce qui rend l'aiguille non substituable par un
littéral nu, et ils rougissent si quelqu'un la rétrécit.

### R3 — Le commentaire du TSV passe au passé

`scripts/canonical-tokens.tsv:232-246`. Le bloc du vol 4 parle du vol 5 au futur :
« au vol 5 les deux enveloppes bash délèguent à la sous-commande, les huit lignes
ci-dessus partent avec leurs tables […] et l'inventaire redevient ce qu'il doit
être — UN site par jeton ». C'est arrivé. Les huit lignes sont parties, les deux
lignes 85-86 aussi, et l'inventaire **est** à un site par jeton.

Le passage au passé, avec la PR qui l'a fait (#3016). C'est la même classe de
défaut qu'AC6 — de la prose qui survit à sa livraison et se lit comme une
promesse en cours — et le laisser serait livrer AC6 à moitié.

**Aucune ligne de données du TSV ne change.** Les six lignes
`arch_disposition.rs::decide` (257-262) et les quatre
`arch_paraphrase.rs::paraphrase` (227-230) sont l'état cible, et le bloc 237-246
explique déjà pourquoi `decide` est le site qui survit.

### R4 — AC6 : la doctrine (hors compte, `docs/`)

`docs/architecture/dispatch-lib-migration.md`. Six sites, tous mesurés sur
`HEAD` — et les numéros ci-dessous ont dérivé d'environ 30 lignes depuis la
rédaction du corps du ticket, que la PR de doctrine #3015 a allongée (le corps
nomme « la l. 569 » ; ce paragraphe est aujourd'hui à la l. 599) :

| site | ce qui y est écrit | ce que la livraison impose |
|---|---|---|
| titre, l. 1 | « bornée à ce que les phases **1 et 2** ont établi » | la phase 3 est établie |
| l. 4-13 | « phases 1 et 2 livrées », « aucune n'est ouverte », « le passage de la phase 3 à "livrée" **viendra avec le vol 5** » | la phase 3 est livrée ; la promesse n'a plus d'objet |
| l. 481-487 (table du § 6) | phase 3 « ticket à ouvrir », précondition « décider ce que devient `$_DISPOSITION_FUZZY_FILE` » | phase 3 **livrée**, ses vols 3-6 nommés, précondition **tranchée** (le fichier reste, le shell l'écrit depuis la sortie du moteur) ; seule la phase 4 reste à ouvrir |
| l. 489-492 | « **Ni la phase 3 ni la phase 4** n'est ouverte » | faux pour la phase 3 |
| l. 502-515 | « **deux** sites de match », « duplication temporaire », « ce qui doit rester vrai **jusque-là** : les deux corps fuzzy sont une transcription littérale » | le zéro est arrivé ; les deux corps fuzzy n'existent plus, et le dispositif qu'ils tenaient est remplacé par le corpus doré et le scan de R2 |
| l. 599-603 | « la phase 3 […] est **écartée sur mesure** » | le paragraphe qu'AC6 nomme explicitement : il ne doit pas survivre à la livraison |

La décision sur le canal (`$_DISPOSITION_FUZZY_FILE` reste, le shell l'écrit, son
retrait relève de la phase 4 avec son jumeau de refus) est inscrite au § 6, avec
le critère C3 qui la porte : ce que seul le shell connaît reste au shell.

**Ce que R4 n'écrit pas.** Pas un mot sur le critère de bascule de la famille de
sous-commandes : AC5 est close par #3015, et le bearing de Prime du 2026-11-02 (b)
exige qu'un vol ne redéfinisse pas la doctrine en silence. Ce vol **cite** #3015,
il ne le rejoue pas.

---

## 4. Ordre de commit, et le plancher

1. **R1** seul — les neuf assertions du scan de callout doivent passer avant que
   quoi que ce soit s'appuie sur la fonction paramétrée.
2. **R2** — le scan de tier 1, avec ses quatre assertions et ses deux contrôles
   négatifs.
3. **R3 + R4** — la prose, une fois le scan vert. La doctrine ne déclare pas
   livré ce qui ne l'est pas : c'est l'ordre que le § 6 du plan du vol 5 impose
   (l. 502-505), et la seule séquence qui ne fasse pas mentir le document pendant
   une PR.

**Plancher du vol :** le scan de tier 1 **et** la doctrine. Aucun des deux seul.
R2 sans R4 laisse un document qui promet ; R4 sans R2 déclare livré un AC4
ouvert. R1 est un prérequis de R2, pas un livrable séparable. R3 part avec R4,
même classe de défaut.

---

## 5. Hors tranche

- **Phase 4** : retirer `$_DISPOSITION_FUZZY_FILE` et son jumeau de refus du
  § 3 du plan du vol 5. Les deux partent ensemble, ou aucun. La condition de
  réveil de la phase 4 (un maillon glue nommé **et** un canal shell→Python) reste
  entière et n'est pas remplie par ce vol.
- **B1 — aucune harmonisation de tolérance.** Le tier 2 reste anglais et reste
  inerte sur les réponses françaises (classe `french-inert` du corpus). Ce vol ne
  touche aucun comportement : il n'ajoute que des assertions et de la prose.
- **Migrer `_second_pass_explicit_iterate`.** C'est mika#2641, conservé par
  décision. Ce vol le nomme comme exception, il ne le déplace pas.
- **Un scan de portée de module côté Rust** pour `arch_disposition` /
  `arch_paraphrase`, sur le modèle de
  `plan_callout::tests::mika2194_aucun_motif_de_callout_hors_de_ce_module`. AC4
  porte sur `dispatch-lib.sh` et sur lui seul. Ce vol ne l'ajoute pas, et le
  signale comme un manque réel plutôt que de l'absorber en silence.
- **Les autres prédicats de `dispatch-lib.sh`** (`_plan_header_claimed_issues`,
  `_branch_reuse_gate`…) : ils ne satisfont pas C3 aujourd'hui.
- **La sonde S3** sur le coût d'un démarrage de `mika`. Le vol 5 l'a ajouté et
  l'a dit ; ce vol n'ajoute aucun démarrage.
- **`crates/mika-agent/src/agent_loop/mod.rs` : zéro octet.** Ce vol n'y écrit
  rien. La marge sous le plafond de 1 MiB était de 1 641 octets au vol 5 ; si un
  poste de ce plan se met à vouloir y écrire, c'est hors tranche et il faut le
  signaler avant de le faire.

---

## Fire-Disposition

Ce vol livre des détecteurs — c'est son objet même : l'assertion zéro de R2, son
assertion de péremption, son contrôle positif sur fixture, son anti-vacuité sur
révision immuable, et ses deux contrôles négatifs. La section est requise.

**Option retenue : (a) exception nommée en allowlist.** Une seule exception, et
elle est la raison d'être de la forme du scan.

**L'exception, nommée, grep-visible, avec son ticket.**

`M2194_T1_EXCEPTION='_second_pass_explicit_iterate'` — le lecteur de tier 1 de
`dispatch-lib.sh:11060-11062`, conservé par la décision de **mika#2641** (AC1
exclut nommément ITERATE). L'entrée nomme la donnée précise (le nom de la
fonction, pas un numéro de ligne ni un fragment de regex), référence son ticket
de suivi, et porte son commentaire de raison.

**Son assertion auto-nettoyante.** Le corps de la fonction doit exister et porter
**exactement un** lecteur de l'aiguille (assertion 2 de R2). Trois façons dont
l'exception devient stale, et les trois rougissent :

- mika#2641 migre vers le moteur ⇒ la fonction disparaît ⇒ le corps est vide ⇒
  rouge. L'exception doit alors être **retirée**, et l'assertion zéro tient
  seule.
- la fonction est renommée ⇒ corps vide ⇒ rouge.
- un second lecteur est ajouté dans ce corps ⇒ le compte passe à 2 ⇒ rouge.

**Les cinq autres détecteurs atterrissent à zéro violation, armés, sans
exception.** L'assertion zéro porte sur une absence que le vol 5 a créée et qui
est mesurée à 0 aujourd'hui (§ 2). Le contrôle positif, l'anti-vacuité et les
deux contrôles négatifs portent sur des fixtures écrites par ce vol et sur une
révision git immuable : il n'existe aucune violation antérieure possible.

**Aucun détecteur n'atterrit désarmé.** Si l'un des six est rouge à la
livraison, ce n'est pas une violation à allowlister — c'est que le vol 5 n'a pas
livré ce que sa sentinelle de bascule affirme, et il faut l'instruire avant de
merger celui-ci.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh` — R1, paramétrisation de `m2194_read_sites()` + ses 9 appels | 14 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — R2, le scan de tier 1 (aiguille, population, exception, 4 assertions, 2 contrôles négatifs) | 62 |
| `scripts/canonical-tokens.tsv` — R3, le bloc 232-246 au passé | 10 |
| `docs/architecture/dispatch-lib-migration.md` — R4, AC6 | hors compte (`docs/`) |

Total estimé : 86 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 86 lignes estimées contre une borne
par vol de 429 et un seuil par PR de 1000 : il passe les deux avec une marge de
5×. Le chiffre est proche du renvoi du vol 5 (≈ 60 lignes pour le poste AC4) ;
l'écart de 26 lignes est R1 (14, non chiffré par le vol 5 qui l'avait nommé sans
le compter) et R3 (10, découvert par ce grooming). Le plancher du § 4 est
indivisible, et le découper serait livrer une doctrine qui mente ou un scan qui ne
garde rien.

**Ce qui pourrait faire déraper l'estimation**, dit d'avance : si l'exclusion de
l'exception par corps de fonction s'avère coûteuse en bash (extraire le corps de
`_second_pass_explicit_iterate` depuis la source demande un `awk` de plage), le
remède borné est de l'exclure par sa **ligne littérale**, comme le scan de tier 2
le fait déjà pour son écrivain (l. 11362) — au prix d'une péremption qui ne voit
pas un reformatage. Le coût de ce repli est 0 ligne, pas davantage.

---

## Definition of Done

1. `R1` livré : `m2194_read_sites` prend son aiguille en `$1`, ses neuf appels
   existants la passent, et les neuf assertions du scan de callout
   (`test-dispatch-lib.sh:11212-11255`) passent **sans modification de leurs
   attentes**.
2. `R2` livré : le scan de tier 1 est vert, avec ses quatre assertions et ses
   deux contrôles négatifs. L'assertion zéro vaut 0, la péremption vaut 1, le
   contrôle positif vaut 4, l'anti-vacuité vaut 5 sur `593a745c5`.
3. `R3` livré : `scripts/canonical-tokens.tsv:232-246` ne parle plus du vol 5 au
   futur.
4. `R4` livré : les six sites du tableau de R4 sont réécrits. Une relecture du
   document ne trouve plus une phrase qui promette la phase 3.
5. **Aucun comportement changé.** `skills/bundled/_shared/test-dispatch-lib.sh`
   passe en entier ; `cargo test` passe ; `cargo clippy` est propre ;
   `make verify-bundled-skills`, `scripts/check-canonical-tokens.sh` et
   `scripts/verify-pipeline.sh origin/main` passent. Aucun fichier de
   `crates/` n'est modifié par ce vol, `agent_loop/mod.rs` au premier rang.
6. **Le corps de PR ne porte aucun `Closes #2194`.** La phase 4 reste portée par
   ce ticket, dont la condition de réveil n'est pas remplie, et la décision
   opérateur du 2026-09-30T19:03:44Z dit que « ce ticket parent reste ouvert et
   les porte ». Le corps dit explicitement que **le ticket n'est pas clos**, et
   que ce vol clôt la **phase 3** — pas le ticket. (C'est la DoD que le vol de la
   phase 1 avait exigée et que son auto-rescue avait violée, rouvrant le ticket.)
7. Le corps de PR dit, en une ligne chacun : qu'AC4 est désormais **entier**
   (tier 2 au vol 5, tier 1 ici), qu'AC6 est clos, que l'exception mika#2641
   existe avec sa péremption, et que l'énoncé d'AC4 du corps du ticket était
   littéralement faux sur un lecteur conservé par décision (§ 2) — pour que la
   correction soit visible et datée plutôt que silencieuse.
8. **Merge par le mot de Vincent, PR par PR** : `dispatch-lib.sh` et sa suite de
   tests sont du substrat de boucle.

---

## Acceptance criteria

Transcrits **verbatim** de la section « Critères d'acceptation » du corps de
mika#2194 (tranche phase 3). Leur répartition entre les vols est au § 1 de ce
plan ; elle ne modifie pas leur énoncé.

- **AC1 : parité dorée sur les cinq tiers.** Le corpus
  `tests/fixtures/arch_paraphrase_texts/` (17 cas) est étendu aux tiers 0, 0b, 1a
  et 1b, chacun avec au moins un cas positif et un cas négatif. Le corpus inclut
  le marqueur cité au milieu d'une ligne, qui ne doit pas supprimer la
  disposition, et un `READY` cité en ligne avant une escalade du moteur, où
  l'escalade doit gagner. Le bash d'avant la bascule (`pre-switch`) et la
  sous-commande rendent les mêmes jetons sur tout le corpus, avec un terme
  d'anti-vacuité (§ 4) : un `dispatch-lib.sh` qui ne se source plus rougit.
- **AC2 : non-régression `post-switch`.** Les tests existants de
  `test-dispatch-lib.sh` et de `_shared/tests/` qui appellent
  `_parse_disposition`, `_parse_verdict` ou `_disposition_was_fuzzy` passent sans
  modification de leurs attentes. Les dix appels directs de
  `_parse_disposition_fuzzy` et `_parse_verdict_fuzzy` dans les tests (mesurés le
  2026-11-02) ne disparaissent pas avec ces fonctions : chacun devient un cas du
  corpus doré, avec son attente inchangée.
- **AC3 : code `≥2`.** Une sous-commande absente ou un fichier illisible produit
  un refus nommé dans le `RESULT`, jamais un UNPARSED silencieux. Un test épingle
  les deux polarités dans le même corps : le refus sort quand le moteur est
  absent, et rien ne sort sur une réponse saine.
- **AC4 : le scan anti-copie (§ 5).** Aucun motif de tier 1 ou 2 ne subsiste dans
  `dispatch-lib.sh`. Le scan est rouge si une table revient.
- **AC5 : la famille de sous-commandes.** Tranché au vol 4 (§ 7 du plan) : la
  famille reste plate, et le critère du § 6 est remplacé. Prime l'a accepté le
  2026-11-02. La doctrine reçoit la décision par une PR séparée, suivie ici, et
  **non** par le vol 5.
- **AC6 : la doctrine est mise à jour.** Les § 6 et 7 de
  `dispatch-lib-migration.md` passent la phase 3 à « livrée » et notent la
  décision sur le canal ci-dessus. Le paragraphe de la l. 569 (phase 3 « écartée
  sur mesure ») est réécrit lui aussi, pour ne pas survivre à la livraison.

### La lecture d'AC4 que ce vol livre, et l'écart qu'il déclare

AC4 est le seul des six dont l'énoncé ne peut pas être satisfait à la lettre, et
le § 2 le mesure : un motif de tier 1 subsiste dans `dispatch-lib.sh`, celui de
`_second_pass_explicit_iterate`, **conservé par la décision de mika#2641**. Ce
vol le livre donc ainsi :

> Aucun motif de tier 1 ou 2 ne subsiste dans `dispatch-lib.sh`, **hormis le
> lecteur que mika#2641 conserve par décision, nommé en exception avec son
> assertion de péremption.** Le scan est rouge si une table revient, et rouge
> aussi le jour où cette exception cesse d'être vraie.

C'est un écart à l'énoncé, pas à l'intention : l'intention d'AC4 est qu'une table
ne revienne pas en silence, et l'exception nommée la sert mieux qu'une aiguille
rétrécie pour atteindre un zéro littéral (§ 2). Il est déclaré ici plutôt que
résolu en silence, et le corps de PR le redit (DoD 7).

Tracked in: senara-solutions/mika#2194

La phase 4 reste portée par ce ticket parent, sa condition de réveil inchangée :
un maillon dont la nature est de la **glue** et non une décision, **et** un canal
shell→Python construit. Le gel de création de tickets court jusqu'au 2026-12-01
(réveil du 2026-11-02, et bearing de Prime du même jour : « gel : aucun ticket
neuf »).

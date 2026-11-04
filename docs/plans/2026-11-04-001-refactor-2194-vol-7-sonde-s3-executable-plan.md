# mika#2194 — vol 7 : la sonde S3 devient une commande, et le compte de démarrages cesse d'être une prose

> **Type :** refactor (précondition de la phase 4)
> **Ticket :** [senara-solutions/mika#2194](https://github.com/senara-solutions/mika/issues/2194)
> **Branche :** `refactor/2194/dispatch-lib-sortir-la-logique-du-bash-d`
> **Vol précédent :** #3017 (vol 6 — le scan de tier 1 et la doctrine), mergé le 2026-11-02.

---

## 0. Le vol, en une phrase

La doctrine a élevé la sonde S3 au rang de **précondition de la phase suivante**
(`docs/architecture/dispatch-lib-migration.md` l. 94-95), et S3 n'a jamais tourné
parce que **son instrument n'existe pas**. Ce vol livre l'instrument, et remplace
par un nombre dérivé la prose qui tient aujourd'hui le compte de démarrages de
`mika` par dispatch.

Il n'ouvre pas la phase 4 et ne la touche pas. Il lève la seule chose que la
doctrine a écrite entre la phase 3 et elle.

---

## 1. L'état mesuré, qui n'est plus celui du corps du ticket

Le corps de mika#2194 décrit la tranche de phase 3 réveillée le 2026-11-02. **Elle
est close en entier.** Mesuré sur `HEAD = 5f1db5b99` :

| critère du corps | état | par |
|---|---|---|
| AC1 — parité dorée sur les cinq tiers | **clos** | vol 4 (#3014) |
| AC2 — non-régression `post-switch` | **clos** | vol 5 (#3016) |
| AC3 — code `≥2`, refus nommé aux deux polarités | **clos** | vol 5 (#3016) |
| AC4 — scan anti-copie | **clos** | tier 2 par le vol 5 (#3016), tier 1 par le vol 6 (#3017) |
| AC5 — famille de sous-commandes plate | **clos** | PR de doctrine #3015 |
| AC6 — doctrine : phase 3 à « livrée » | **clos** | vol 6 (#3017) |

Ce que ce ticket porte encore, et rien d'autre :

- **La phase 4**, dormante. Sa condition de réveil est « un maillon de nature
  **glue** nommé **et** un canal shell→Python construit »
  (`dispatch-lib-migration.md` l. 495-502). Elle n'est pas remplie : le canal
  construit par le vol 5 est shell→**Rust** (`mika arch-disposition`), et aucun
  canal shell→Python n'existe dans l'arbre. Ce vol ne la lève pas et n'y touche
  pas.
- **La sonde S3**, jamais exécutée. Le corps du ticket la déclare hors tranche de
  la *tranche phase 3* — tranche aujourd'hui close — et la doctrine, elle, en a
  fait la précondition du maillon suivant.

Les faits qui portent le reste de ce plan, chacun mesuré sur `HEAD = 5f1db5b99`
et aucun repris d'un plan antérieur :

- **S3 est la précondition, écrite comme telle.** « La même arithmétique tient —
  trois démarrages contre un dispatch qui dure des minutes à des heures — mais
  elle **consomme** la marge que ce tableau décrivait, donc la sonde S3 cesse
  d'être une curiosité et devient la précondition de la phase suivante »
  (`dispatch-lib-migration.md` l. 88-95).
- **S3 n'a pas d'instrument.** Son énoncé est « chronométrer `mika plan-callout`
  sur l'hôte » (l. 753-756) et **aucun fichier de l'arbre ne le fait** : les cinq
  entrées de `scripts/` qui portent le préfixe `measure-` ou `bench` mesurent le
  coût des tours webhook (`measure-webhook-turn-cost`) et la couverture des
  en-têtes de plan (`measure-plan-header-coverage.sh`). Rien ne chronomètre une
  sous-commande.
- **Et elle ne peut pas tourner depuis un dispatch.** La doctrine le note à
  l'endroit où elle pose le critère de canal : le coût « n'était pas mesurable
  depuis le bac à sable de dispatch (`mika --version` y est refusé par la
  permission-policy) » (l. 83-86). S3 est donc, par construction, un geste
  d'**hôte** — ce qui est exactement pourquoi il lui faut une commande, et non
  une recette en prose dans un § 8.
- **Le compte de démarrages par dispatch est de la prose, et elle sous-compte.**
  La doctrine écrit « trois par dispatch (§ 2) et la phase 3 à un de plus par
  passe d'architecte (§ 6) » (l. 22-24). Les sites réels, comptés dans
  `skills/bundled/_shared/dispatch-lib.sh` :

  | sous-commande | sites de production | fonction porteuse |
  |---|---|---|
  | `mika plan-callout` | `:4953` | `_committed_plan_on_branch` (4935), elle-même appelée en `:5837` et `:13960` |
  | | `:5878` | le `elif` de `_set_up_worktree` |
  | | `:13199` | `_detect_plan_on_branch` (13159) |
  | `mika arch-disposition` | `:11373` | `_parse_disposition` (passe `first`) |
  | | `:11480` | `_parse_verdict` (passe `second`) |
  | | `:11633` | `_parse_verdict` (passe `second`, après itération) |

  **Six sites**, pas « trois plus un ». Le nombre exécuté sur un dispatch donné
  dépend des branches prises, et c'est précisément ce qu'une prose ne peut pas
  dire. Le chiffre que la doctrine dit « écrit plutôt que laissé à découvrir »
  (l. 87-88) est aujourd'hui écrit **faux par défaut d'unité** : il compte des
  passes là où le coût se paie en démarrages.

- **Le patron d'un instrument existe en deux formes, et elles ne sont pas
  équivalentes.** `scripts/measure-webhook-turn-cost` **n'a pas d'extension**,
  donc il échappe à la découverte de `scripts/check-suite-wiring.sh` (l. 143-150 :
  `find … -name '*.sh' -o -name '*.py'`) ; ce qui est découvert et wiré est sa
  suite `scripts/test-measure-webhook-turn-cost.sh`, via `make
  test-measure-webhook-turn-cost` (`Makefile:213-214`) que `ci.yml:125` nomme.
  `scripts/measure-plan-header-coverage.sh`, lui, porte `.sh`, est donc découvert,
  et est déclaré `operator` au registre (`scripts/suite-wiring.tsv:79`) avec cette
  raison : « there is no threshold to fail on, and inventing one here would turn a
  measurement into a gate nobody chose ».

  **C'est cette raison qui choisit la forme ici, et elle choisit l'autre.** S3
  *a* un seuil — « au-delà de ~500 ms » (l. 757-760) — donc l'instrument décide,
  donc il doit être exercé aux deux polarités. La forme `measure-plan-header-coverage.sh`
  est celle d'un instrument *sans* seuil ; la retenir ici livrerait un décideur
  que rien n'exerce.

---

## 2. Le geste

### R1 — `scripts/measure-mika-subcommand-cost` : S3 en une commande

Un exécutable **sans extension**, sur le patron de `measure-webhook-turn-cost`
(§ 1) : il n'est pas une suite, il n'a rien à déclarer au registre, et sa suite de
test R2 est ce que CI exécute.

**Ce qu'il mesure.** Le coût d'**un** démarrage, pour chacune des deux
sous-commandes de prédicat, sur le chemin nominal :

- `mika plan-callout --body-file <f>` sur un corps portant un callout de plan
  bien formé ;
- `mika arch-disposition --pass first --response-file <f>` sur une réponse
  d'architecte portant `Disposition: READY`.

Les deux fixtures sont écrites par l'instrument lui-même en `mktemp` et retirées
en sortie : la mesure ne dépend d'aucun fichier du dépôt, donc elle ne dérive pas
avec lui.

**Ce qu'il rend.** Pour chaque sous-commande, `N` répétitions (défaut 10,
`--repeat`), et le **p50** et le **p95** en millisecondes entières. Pas de
moyenne : un p50 et un p95 séparent un coût stable d'un coût à queue longue, et
c'est la distinction qui décide si le remède est le placement ou rien.

**Le compte de sites, dérivé et non écrit.** L'instrument compte les sites de
production des deux sous-commandes dans `dispatch-lib.sh` — la table du § 1 — et
rend le produit `p95 × sites` sous le nom de ce qu'il est : le coût au pire d'un
dispatch dev-groom qui prendrait toutes les branches. Le compte est **dérivé de
l'arbre à chaque exécution**, jamais une constante dans l'instrument : une
constante serait une seconde source de vérité sur une population que
`dispatch-lib.sh` porte déjà, c'est-à-dire la duplication que ce ticket existe
pour retirer.

**La halte, en code de sortie.** `0` quand les deux p95 tiennent sous le seuil
(défaut 500 ms, `--threshold-ms`) ; `3` quand l'un le dépasse, avec sur stderr le
nom de la sous-commande, son p95, et le remède que la doctrine impose — **le
placement dans `main.rs`, jamais un cache**. Un code distinct de `1` et de `2`
parce que « le seuil est dépassé » n'est ni un usage fautif ni une panne : c'est
un résultat, et le confondre avec une panne ferait lire une halte comme un bug
d'instrument.

**Le binaire absent est un SKIP bruyant, pas un zéro.** `mika` introuvable → code
`2` et un message qui le dit. Un instrument de coût qui rendrait `0 ms` sur un
binaire absent serait l'exact mensonge que mika#2205 nomme : une sonde muette qui
se lit comme une sonde qui marche.

### R2 — `scripts/test-measure-mika-subcommand-cost.sh` : l'instrument est exercé, seuil compris

La suite que CI exécute. Elle n'a **pas** besoin d'un vrai `mika` : elle en
fabrique un faux dans un `PATH` temporaire, dont la latence est pilotée par un
`sleep`, ce qui rend les deux polarités du seuil reproductibles sans dépendre de
la machine.

Les contrôles :

1. **Chemin nominal.** Faux `mika` instantané → code `0`, et la sortie porte un
   p50 et un p95 pour chacune des deux sous-commandes.
2. **Halte franchie** (polarité positive du seuil). Faux `mika` qui dort au-delà
   du seuil, `--threshold-ms` abaissé pour tenir le temps de la suite → code `3`,
   et stderr nomme la sous-commande *et* le remède « placement dans `main.rs` ».
   L'assertion porte sur la présence du remède, parce qu'une halte qui ne dit pas
   quoi faire se fait réparer par un cache.
3. **Halte non franchie** (polarité négative). Même instrument, seuil relevé →
   code `0`, et **rien** sur stderr. Les deux polarités dans le même corps, comme
   AC3 du ticket l'a exigé du refus `≥2`.
4. **Binaire absent.** `PATH` vide → code `2`, message explicite, et **aucun
   chiffre** dans la sortie.
5. **Le compte de sites est dérivé.** L'instrument, pointé sur une copie de
   `dispatch-lib.sh` en `mktemp` à laquelle un septième site a été ajouté, rend
   `7`. C'est le contrôle que le compte vient de l'arbre et non d'une constante :
   un instrument qui rendrait `6` sur une copie à sept sites porterait le nombre
   en dur.
6. **Anti-vacuité du compte.** Sur une copie dont les six sites ont été retirés,
   le compte rend `0` **et** l'instrument le signale comme une population vide
   plutôt que comme un coût nul. Un prédicat qui compte `0` sans le dire mesure
   une population absente et se lit comme un coût négligeable — la forme exacte
   de la halte transverse du § 8.

### R3 — Le wiring

- `Makefile` : cible `test-measure-mika-subcommand-cost`, ajoutée à `.PHONY`, sur
  le modèle de `Makefile:213-214`.
- `.github/workflows/ci.yml` : une étape qui nomme cette cible, voisine de la
  ligne 125.

Rien au registre `scripts/suite-wiring.tsv` : l'instrument échappe à la découverte
(pas d'extension) et la suite est atteinte par un workflow. Le lint
`suite-wiring` reste vert sans exception, et c'est vérifiable par `make
check-suite-wiring`.

### R4 — La doctrine reçoit le nombre, et S3 reçoit sa commande

Dans `docs/architecture/dispatch-lib-migration.md` :

- **§ 2** — la prose « trois par dispatch … un de plus par passe d'architecte »
  est remplacée par le compte de **sites** de la table du § 1, avec la raison du
  changement d'unité : le coût se paie en démarrages, et une passe d'architecte
  n'en vaut pas un.
- **§ 8, S3** — la recette « chronométrer `mika plan-callout` sur l'hôte » est
  remplacée par la commande qui le fait, son seuil, son code `3`, et le rappel que
  le remède est le placement. Le préalable de déploiement du § 8
  (`cat ~/.mika/skills/.manifest-writer`) reste en tête : il vaut pour cette sonde
  comme pour les trois autres.
- **L'en-tête (l. 22-24)** — « toujours pas chronométré » devient « chronométrable
  en une commande, et non encore chronométré ». La nuance est tout l'objet de ce
  vol : il ne produit pas le chiffre, il rend le chiffre produisible. **Ce plan ne
  déclare pas S3 exécutée**, et le corps de PR le redira.

---

## 3. Ce que ce vol ne fait pas

- **Il n'exécute pas S3.** Un dispatch ne peut pas : la permission-policy refuse
  `mika` dans le bac à sable (§ 1), et c'est mesuré, pas supposé. Le chiffre est
  un geste d'opérateur sur l'hôte, après `make deploy`, et il reste dû.
- **Il n'ouvre pas la phase 4** et ne touche pas sa condition de réveil.
- **Il ne touche pas `dispatch-lib.sh`.** L'instrument le **lit** pour dériver le
  compte de sites ; il n'en modifie pas une ligne. Le substrat de boucle n'est pas
  dans le périmètre, donc la règle de merge du corps (« le mot de Vincent, PR par
  PR ») s'applique au ticket mais ce vol ne modifie aucun substrat.
- **Il ne change aucun comportement.** Aucune fonction de production n'est
  touchée ; le livrable est un instrument et sa suite.
- **Il n'ajoute pas de cache**, et l'instrument nomme ce refus dans sa propre
  halte pour que le remède ne soit pas réinventé à l'envers.
- **Il ne touche pas `agent_loop/mod.rs`**, dont la marge au plafond du scan de
  secrets était de 1 652 octets au 2026-11-02. Ce vol n'ajoute aucun octet à ce
  fichier.
- **Il ne retire pas l'exception mika#2641** du scan de tier 1. Elle est
  auto-expirante par construction (vol 6, R2, assertion 2) et expire le jour où
  mika#2641 migre, pas ici.

---

## 4. Fire-Disposition

Ce vol livre des détecteurs : les six contrôles de R2, dont deux (le compte dérivé
et son anti-vacuité) ont pour chemin de succès « aucune divergence ».

**Option retenue : (a) exception nommée en allowlist — et sa population est vide à
la livraison.**

La population d'exceptions est vide parce que le détecteur naît sur un arbre
conforme : le compte qu'il déclare **est** le compte mesuré au § 1, six, et il est
dérivé de l'arbre à l'exécution plutôt qu'écrit. Il n'y a donc aucune violation
préexistante à excuser, et en inventer une serait pire qu'inutile.

**Ce n'est pas un détecteur armé à zéro par chance, et le plan le prouve par deux
contrôles de bonne foi.** Le contrôle 5 (septième site ajouté → `7`) refuse un
prédicat qui porterait le nombre en dur et rendrait `6` quoi qu'il arrive ; le
contrôle 6 (six sites retirés → `0` **signalé**) refuse un prédicat qui mesurerait
une population vide en silence. Sans ces deux-là, « aucune divergence » et
« aucune population » se liraient pareil — c'est la halte transverse du § 8, et la
leçon de mika#2205.

Aucune entrée n'est ajoutée à `scripts/suite-wiring.tsv` : la suite est wirée
(R3), ce qui est le seul état que le lint accepte sans déclaration.

---

## 5. Acceptance criteria

- **AC1 — S3 est exécutable en une commande.**
  `scripts/measure-mika-subcommand-cost` chronomètre `mika plan-callout` et
  `mika arch-disposition` sur leur chemin nominal, rend un p50 et un p95 en
  millisecondes pour chacune, et écrit ses deux fixtures lui-même sans lire aucun
  fichier du dépôt.
- **AC2 — le compte de démarrages est dérivé de l'arbre, jamais écrit.**
  L'instrument compte les sites de production des deux sous-commandes dans
  `dispatch-lib.sh` et rend `p95 × sites`. Un test pointe l'instrument sur une
  copie à sept sites et exige `7` ; un second l'exerce sur une copie à zéro site
  et exige que la population vide soit **signalée** et non rendue comme un coût
  nul.
- **AC3 — la halte de S3 est un résultat, et elle est exercée aux deux
  polarités dans le même corps.** p95 au-delà du seuil → code `3`, avec la
  sous-commande fautive et le remède « placement dans `main.rs` » sur stderr ;
  p95 sous le seuil → code `0` et stderr vide. `mika` absent → code `2`, message
  explicite, aucun chiffre.
- **AC4 — la suite est atteinte par un workflow.**
  `make test-measure-mika-subcommand-cost` existe, `.github/workflows/ci.yml` la
  nomme, et `make check-suite-wiring` est vert **sans** nouvelle entrée au
  registre.
- **AC5 — la doctrine porte le nombre et la commande.** Le § 2 remplace la prose
  « trois plus un par passe » par le compte de sites et dit pourquoi l'unité
  change ; le § 8 S3 remplace sa recette par la commande, son seuil et son code
  `3` ; l'en-tête passe de « toujours pas chronométré » à « chronométrable en une
  commande, et non encore chronométré ».
- **AC6 — rien d'autre ne bouge.** `dispatch-lib.sh` n'est pas modifié (il est
  lu), `agent_loop/mod.rs` ne reçoit aucun octet, aucune fonction de production
  n'est touchée, et `cargo test` + `make lint` + `make test-dispatch-lib` passent
  sans modification d'une attente existante.

---

## 6. Definition of Done

1. **Les six critères ci-dessus sont satisfaits**, chacun par un contrôle qu'on
   peut exécuter et nommer.
2. **La suite R2 passe hors ligne**, sans vrai binaire `mika` et sans réseau : le
   faux `mika` en `PATH` temporaire est ce qui rend les deux polarités du seuil
   reproductibles.
3. **`make check-suite-wiring` est vert**, avec `unwired=0` inchangé et aucune
   entrée ajoutée au registre.
4. **La suite complète passe** : `cargo test`, `make lint`, `make fmt`,
   `make test-dispatch-lib`, `make verify-bundled-skills`,
   `scripts/verify-pipeline.sh origin/main`.
5. **Le corps de PR dit que S3 n'a pas tourné.** Ce vol rend le chiffre
   produisible ; il ne le produit pas. Un corps qui laisserait lire « S3 est
   faite » referait l'erreur que le § 8 nomme — une sonde muette se lit comme une
   sonde qui marche.
6. **Le corps de PR ne porte aucun `Closes #2194`.** La phase 4 reste portée par
   ce ticket parent, et la décision opérateur du 2026-09-30T19:03:44Z dit que
   « ce ticket parent reste ouvert et les porte ». La fermeture par effet de bord
   d'un `Closes` est exactement ce qui a dû être défait au vol 1.
7. **Le corps de PR nomme le geste d'hôte qui reste dû** : après `make deploy`,
   vérifier `cat ~/.mika/skills/.manifest-writer`, puis exécuter
   `scripts/measure-mika-subcommand-cost`. C'est la halte de S3, et elle revient à
   l'opérateur.
8. **Merge par le mot de Vincent, PR par PR**, la règle du corps pour ce ticket.

---

## 7. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/measure-mika-subcommand-cost` | 150 |
| `scripts/test-measure-mika-subcommand-cost.sh` (six contrôles) | 140 |
| `Makefile` (cible + `.PHONY`) et `.github/workflows/ci.yml` (une étape) | 6 |

Total estimé : 296 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 296 est sous la borne par vol (429) et sous le
seuil par PR (1000), et les deux livrables sont indépendants de tout code de
production : aucune migration, aucune parité dorée à établir, aucun fichier de
`crates/` touché. Le poste le plus lourd est la suite R2, et ses six contrôles
partagent un seul échafaudage — le faux `mika` en `PATH` temporaire — ce qui est
la raison pour laquelle le sixième ne coûte pas six fois le premier.

Les deux fichiers de documentation (R4, `dispatch-lib-migration.md`) ne sont pas
comptés, conformément au format.

---

## 8. Ordre de commit

1. **R1** — l'instrument seul. Exécutable à la main sur l'hôte dès ce commit.
2. **R2** — la suite, qui l'exerce aux deux polarités et prouve que le compte est
   dérivé. C'est ce commit qui arme les détecteurs.
3. **R3** — le wiring (Makefile, ci.yml), pour que CI les exécute.
4. **R4** — la doctrine, qui reçoit le nombre et la commande.

**Le plancher, s'il faut s'arrêter avant la fin :** R1 + R2 + R3. Un instrument
exercé et wiré sans sa ligne de doctrine est utile mais incomplet ; un instrument
wiré sans sa suite serait un décideur que rien n'exerce, et c'est l'inverse qu'il
faut livrer. R4 seul n'a aucun sens : il annoncerait une commande absente.

Tracked in: senara-solutions/mika#2194

La phase 4 reste portée par ce ticket parent, sa condition de réveil inchangée :
un maillon dont la nature est de la **glue** et non une décision, **et** un canal
shell→Python construit. Le gel de création de tickets court jusqu'au 2026-12-01
(réveil du 2026-11-02, et bearing de Prime du même jour : « gel : aucun ticket
neuf ») — c'est pourquoi ce vol vit dans ce ticket et n'en ouvre pas un second.

**Le geste d'hôte qui reste dû après ce vol, et qui n'est pas dans son périmètre :**
exécuter S3. Son résultat décide si la phase 4 part d'une marge mesurée ou d'une
marge supposée ; si le p95 dépasse ~500 ms, le remède est le **placement dans
`main.rs`**, et il serait son propre vol.

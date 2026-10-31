# mika#2854 — classe (h) du pré-filtre webhook, vol 4 : la dormance d'AC1 change de support, et sa condition cesse d'être circulaire

> Ticket : senara-solutions/mika#2854 — labels `p2-normal`, `dispatch:loop`, `loop-substrate` ; jalon *Substrat de boucle*.
> Vols antérieurs : #2858 (vol 1, AC2/AC3/AC5/AC6/AC7 + moitié mécanique d'AC1), #2873 (vol 2, AC4), #2874 (vol 3, instrument d'AC1 sous vrai modèle).
> Refs mika#2680, mika#2675, senara-solutions/claude-pilot#290.

## Le problème, en une phrase

Les sept AC de mika#2854 sont livrées et mergées ; il ne reste qu'une **mesure** que
l'opérateur a décidé de faire passer par le pilote après `claude-pilot#290` — mais le ticket
reste **ouvert**, et son unique condition de réveil, telle qu'elle est écrite dans son corps,
est **circulaire** : elle ne peut jamais être remplie, donc aucune porte ne peut le laisser
dormir et la boucle le re-groome indéfiniment.

## L'état mesuré sur `main` (`feef58d45`, 2026-10-31)

Relevé dans le code, pas déduit du corps du ticket :

| AC | état sur `main` | preuve |
|---|---|---|
| AC1 (corpus de contrôle négatif) | **instrument livré, mesure non faite** | `crates/mika-agent/src/server/webhook_prefilter/tests/ac1_vrai_modele_2854.rs` (748 l.) — 11 cas verbatim, témoin dégénéré, 2 gardes hors-ligne ; les deux tests de mesure sont `#[ignore]` derrière `MIKA_EVAL_TRIAGE_PROVIDERS` |
| AC2 (contrôle positif) | livré | `ac1_vrai_modele_2854.rs` rejoue #2077 et #2779 dans le même appel ; côté mécanique, `mika2854_h_le_corpus_dordres_est_discriminant` |
| AC3 (fail-open) | livré | `comment_triage.rs::mika2854_une_panne_de_transport_ne_rend_jamais_une_note`, `…_une_expiration_ne_rend_jamais_une_note`, `…_tout_le_reste_est_illisible_donc_le_tour_complet` |
| AC4 (`issues` `closed`) | livré | `7eb63e437` (#2873), snapshot de fermeture pris avant le nettoyage |
| AC5 (ligne d'audit + `skipped_by_class`) | livré | `Release::Triage*` / `PrefilterClass::CommentTriagedNote` dans `webhook_prefilter.rs` |
| AC6 (aucun outil mutationnel) | livré | `comment_triage.rs::build_request` → `tools: None`, asserté par `mika2854_le_tableau_servi_ne_porte_aucun_outil` |
| AC7 (ticket avec tâche de boucle intact) | livré | décideur de (h) dans `webhook_prefilter.rs`, lu avant le nettoyage |

Le plancher tient et il est épinglé : `comment_triage.rs::triage_enabled()` rend `false` sur
l'absence de `MIKA_WEBHOOK_COMMENT_TRIAGE`, et
`mika2854_labsence_de_variable_laisse_le_triage_desarme` l'asserte (« § D8 — absent vaut
off »). **Aucune ligne de code n'est due pour les sept AC.**

Les deux plans antérieurs sont sur `main`
(`docs/plans/2026-10-20-003-…-plan.md`, `docs/plans/2026-10-21-003-…-plan.md`). Celui du vol 3
n'y est pas : #2874 (`b3d3e03eb`) a livré sans fichier de plan — c'est sa description de PR qui
porte le raisonnement, et ce plan-ci la cite plutôt que de la reconstituer.

## Le point dur, vérifié et non supposé : la condition de réveil est insatisfiable

Le corps du ticket porte, depuis le 2026-10-22 (bearing Prime) :

> - **Condition de réveil :** quand `MIKA_WEBHOOK_COMMENT_TRIAGE` passe à `on`.
> - D'ici là, l'interrupteur reste à `off` par défaut. Le passer à `on` exige cette mesure d'abord.

Les deux lignes se mordent la queue : la mesure est due quand l'interrupteur est armé, et
l'armement exige la mesure. Un lecteur ne pourra **jamais** dire que la condition est remplie.
Le contrat de `docs/dormeurs.md` § *Contrat d'une entrée* refuse les conditions qu'on ne peut
pas trancher (« plus tard », « si ça revient ») en les nommant *des fermetures déguisées en
attentes* ; celle-ci est pire, parce qu'elle est **tranchable et toujours fausse**.

Le vrai bloqueur est nommé deux lignes plus haut dans le même corps — *« Voie au réveil : le
pilote, au grand jour, après senara-solutions/claude-pilot#290 … Jamais un spawn tactique »* —
et il est vérifié : `claude-pilot#290` est **ouverte** au 2026-10-31, et son titre dit
littéralement de quoi il s'agit : *« fix(permissions): admettre la classe "run d'éval borné"
(tests `#[ignore]` nommés, fournisseur nommé, budget plafonné) — AC1 de mika#2854 refusé
(15017db4) »*. La dépendance est donc une donnée du dépôt amont, pas une appréciation.

**L'ordre est inversé dans le corps.** La mesure ne suit pas l'armement : elle le **précède** et
le fonde — elle produit le chiffre sur lequel Vincent décide d'armer. La condition de réveil
d'AC1 est donc la livraison de `claude-pilot#290`, et rien d'autre.

### Pourquoi le support doit changer, et non la seule condition

Corriger la condition dans le corps GitHub laisserait le ticket **ouvert**, donc dispatchable.
Et c'est là que la mesure est claire : chaque porte lit la dormance pour son compte, ou pas du
tout.

- Le bassin du manager la lit : mika#2952 vol 2/2 a livré `RefusalReason::Dormant`
  (`milestone_manager/pool.rs`), et ses fixtures citent **ce ticket**, verbatim
  (`pool.rs:2439`, `types.rs:1202`, `relief.rs:508`).
- La porte qui a produit ce vol-ci, elle, ne la lit pas :
  `grep -rn "dormancy\|Dormant" --include="*.rs" crates/` ne rend, hors `milestone_manager`,
  que deux `dormancy: None` de fixtures. Côté bash, `dispatch-lib.sh` n'a aucune lecture de
  dormance.

Le manager refuse donc d'**admettre** #2854 pendant qu'une autre porte le **groome** — c'est la
quatrième session de grooming de ce ticket (`29e19a520`, `c1b375d7d`, `141058a26`, et celle-ci),
chacune un vol de pilote entier dépensé sur un travail qui n'est pas exécutable. Ajouter un
sixième lecteur du corps réglerait une porte et laisserait les suivantes — la forme même que
mika#2263 a déjà payée.

Le dépôt a une réponse ratifiée, et elle agit sur **toutes** les portes à la fois :
`docs/dormeurs.md`. Son § *Pourquoi ce fichier existe* porte la décision de l'opérateur du
2026-09-03 — *« la visibilité change de support : le registre versionné remplace le ticket
ouvert »* — et son § *Contrat d'une entrée* nomme la réciproque : *« Quand la condition est
remplie, rouvrir le ticket GitHub cité (il conserve tout son historique) et retirer la ligne
d'ici »*. Vérifié : les **onze** tickets cités par le registre sont `CLOSED` (#1403 le
2026-09-03, #1913 le 2026-09-03, #2357 le 2026-09-24, #2609 le 2026-10-01, …). Un dormeur
**ouvert** est l'anomalie que ce fichier existe pour supprimer, et #2854 en est un.

Les trois exclusions du § *Ce qui n'entre pas ici* ont été testées une par une :

- *« travail en cours »* — non : les sept AC sont mergées, rien n'est en vol.
- *« correctif groomé dont le dispatch est bloqué par un défaut de l'alimenteur »* — non : rien
  n'est empêché par l'alimenteur ; c'est l'amont `claude-pilot#290` qui refuse l'exécution, et
  l'opérateur a nommé cette voie et exclu les autres.
- *« ticket sans condition vérifiable »* — c'est l'état d'aujourd'hui, et c'est précisément ce
  que ce vol répare : la ligne du registre en porte une tranchable par une commande.
- *« ticket qu'on préfère ne pas faire »* — non : la mesure est réellement due, et AC1 n'est pas
  verte sans elle.

## Décisions

### D1 — La ligne du registre est le livrable central, et sa condition est une commande

`docs/dormeurs.md` reçoit une entrée `#2854`. Sa condition de réveil est tranchable sans
contexte :

> `gh issue view 290 --repo senara-solutions/claude-pilot --json state -q .state` rend **`CLOSED`**

Rien de plus dans la condition. L'armement de `MIKA_WEBHOOK_COMMENT_TRIAGE` n'y figure **pas** :
il est en aval de la mesure, pas en amont, et l'y remettre rééditerait la circularité que ce
vol lève.

La cellule *sujet* porte ce qu'un lecteur sans contexte doit savoir pour réveiller sans se
tromper, sur le modèle des entrées #2609 et #2357 qui portent toutes deux un « Au réveil, … » :
que l'instrument est **déjà sur `main`** (`ac1_vrai_modele_2854.rs`, 11 cas verbatim + témoin
dégénéré), la commande qui le joue, et la règle d'interprétation que le vol 3 a posée — un rouge
est un constat à rendre à Vincent, jamais une invitation à retoucher `TRIAGE_SYSTEM_PROMPT`
(discipline mika#1190).

### D2 — Le ticket se ferme, et la fermeture est l'application d'une décision déjà prise

La PR porte `Closes #2854`. Ce n'est pas une décision neuve : c'est le mécanisme ratifié le
2026-09-03, dont les onze entrées en place sont l'application. Elle est **réversible par
construction** — le § *Réveil* du registre dit de rouvrir le ticket, qui garde tout son
historique.

La fermeture est aussi ce qui arrête la dépense : un ticket fermé n'est dispatchable par aucune
porte, qu'elle lise la dormance ou non. C'est la propriété que l'ajout d'un lecteur de plus
n'aurait pas donnée.

### D3 — Un seul livrable de code : l'instrument dit où est sa procédure de réveil

`ac1_vrai_modele_2854.rs` se saute aujourd'hui en le disant, et son message nomme la commande à
rejouer. Il ne nomme pas la **ligne du registre**, donc celui qui lance la suite au réveil
n'apprend pas qu'il existe une procédure, ni laquelle. Les entrées #2609 et #2357 portent leur
« Au réveil, … » précisément parce que ce défaut a déjà coûté une fois
(`docs/solutions/dev-loop/epique-dincident-perime-avant-son-execution-2026-09-24.md`).

Le vol ajoute donc le pointeur à ses deux endroits : l'en-tête de module (§ *Comment on le
joue*) et les deux messages de saut. **Zéro ligne de logique**, zéro octet sur
`TRIAGE_SYSTEM_PROMPT`, aucune assertion touchée.

Ce livrable a un second effet, utile et assumé : il met la PR dans `SOURCE_BUCKET`, donc le
rejet *docs-only* de `scripts/verify-pipeline.sh` (l. 354) ne se pose pas, et le vol n'a pas
besoin du label `pipeline-exempt` ni d'un *trailer*. Si pour une raison quelconque le pilote
devait livrer sans ce fichier, l'échappement nommé est
`Pipeline-Exempt: docs-only — <raison>` ou le label `pipeline-exempt` sur la PR, jamais un
contournement inventé.

### D4 — La leçon s'écrit, parce qu'elle n'est pas propre à ce ticket

Une condition de réveil dont la satisfaction exige ce qu'elle garde est une attente
insatisfiable, et elle est *invisible* : elle a la forme d'une condition valable, elle passe la
relecture, et elle garde un ticket ouvert pour toujours. Le registre a onze autres entrées, donc
onze autres occasions. La leçon part dans `docs/solutions/`.

### D5 — Rien d'autre n'est touché

Le périmètre s'arrête là. Les lectures de dormance manquantes à la porte du dispatch sont un
défaut réel, mesuré ci-dessus, mais il appartient au mécanisme de dormance (mika#2952 /
mika#2956) et non à la classe (h) du pré-filtre : l'étendre ici serait la dérive de périmètre
que ce vol a tout intérêt à ne pas commettre. La fermeture du ticket rend l'absence inoffensive
**pour ce ticket-ci**, pas en général, et ce plan le dit plutôt que de le laisser croire.

## Plan de livraison (ordre de commit)

1. **`docs(dormeurs)` — l'entrée #2854, condition tranchable.**
   Ajouter une ligne au tableau du § *Registre* de `docs/dormeurs.md`, aux trois colonnes du
   contrat : ticket (lien markdown dont le numéro de l'URL correspond au libellé), sujet, et la
   condition de D1. Le sujet nomme l'instrument sur `main`, la commande de rejeu et la règle
   « un rouge est un constat ». Ne pas toucher aux onze autres lignes.

2. **`docs(ac1)` — l'instrument pointe sa procédure de réveil.**
   Dans `crates/mika-agent/src/server/webhook_prefilter/tests/ac1_vrai_modele_2854.rs` :
   ajouter au § *Comment on le joue* de l'en-tête une phrase citant
   `docs/dormeurs.md`, entrée mika#2854, comme procédure de réveil ; et compléter les messages
   de saut des deux tests `#[ignore]` (`ac1_vrai_modele_2854`,
   `ac1_le_rouge_sous_vrai_modele_2854`) par la même référence. Aucune assertion, aucun seuil,
   aucun cas du corpus n'est modifié.

3. **`docs(solutions)` — la leçon.**
   `docs/solutions/dev-loop/une-condition-de-reveil-circulaire-est-une-attente-insatisfiable-2026-10-31.md`,
   avec le *frontmatter* YAML de la maison (`module`, `tags`, `problem_type`). Contenu : la
   circularité mesurée sur #2854, le test de lecture (« un lecteur peut-il dire *oui* un jour ? »
   et non seulement « la condition est-elle précise ? »), le coût constaté — quatre sessions de
   grooming sur un travail non exécutable —, et pourquoi la réponse est un changement de support
   plutôt qu'un lecteur de dormance de plus (mika#2263).

Les trois commits sont indépendants et l'ordre n'est contraint que par la lisibilité :
l'entrée du registre d'abord, puisque les deux suivants la citent.

## Contrat de vérification

Tout est hors-ligne. Aucun appel de modèle, aucune clé d'API, aucun réseau sauf la seule
commande `gh` de la condition de réveil, qui n'est **pas** jouée par la CI.

| ce qu'on vérifie | comment | attendu |
|---|---|---|
| le plancher n'a pas bougé | `cargo test -p mika-agent --lib comment_triage` | vert, dont `mika2854_labsence_de_variable_laisse_le_triage_desarme` |
| l'instrument compile et se saute en le disant | `cargo test -p mika-agent --lib -- --ignored --nocapture ac1_vrai_modele_2854` | les deux tests se sautent en imprimant la commande de rejeu **et** la référence `docs/dormeurs.md` |
| les deux gardes hors-ligne du vol 3 tiennent | `cargo test -p mika-agent --lib mika2854_le_corpus_du_vol_3 mika2854_le_vol_3_ne_touche_pas` | vert |
| le gabarit de production est intact | `git diff --stat origin/main -- crates/mika-agent/src/server/comment_triage.rs` | **aucune sortie** |
| la suite entière | `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` | vert |
| les lints du dépôt | `make lint`, `scripts/verify-pipeline.sh origin/main` | vert, et **aucun** `[pipeline-exempt: none] REJECT` |
| la taille de `CLAUDE.md` n'est pas touchée | `cargo test -p mika-common --test claude_md_size` | vert (ce vol ne modifie aucun `CLAUDE.md`) |

## Fire-Disposition

**Option (b) — livrer désarmé, avec son suivi tracké.** Et la précision qui compte : ce vol ne
**crée** aucun détecteur. Il touche un détecteur existant — les deux tests de mesure d'AC1,
`ac1_vrai_modele_2854` et `ac1_le_rouge_sous_vrai_modele_2854`, dont le chemin de succès est
bien « aucune décision dégradée en note » — et il le touche **sans changer son armement** : les
deux `#[ignore]` restent, la porte dédiée `MIKA_EVAL_TRIAGE_PROVIDERS` reste, le corpus, les
assertions et les seuils ne bougent pas. Seuls l'en-tête de module et les deux messages de saut
changent (§ D3).

Le suivi tracké, c'est précisément le livrable 1. Jusqu'ici le désarmement d'AC1 était suivi par
une ligne du corps GitHub dont la condition est insatisfiable (§ *Le point dur*) — un suivi qui
ne se déclenche jamais n'est pas un suivi. Après ce vol, l'armement est tracké par l'entrée
`docs/dormeurs.md` § *Registre*, ligne `#2854`, dont la condition est tranchée par une seule
commande, plus `Tracked in: senara-solutions/claude-pilot#290` dans le corps de PR. **La
disposition (b) passe donc d'un suivi fictif à un suivi réel ; c'est l'objet du vol, pas un
effet de bord.**

Les deux gardes **hors-ligne** du vol 3 (`mika2854_le_corpus_du_vol_3_est_dans_la_population_de_h`,
`mika2854_le_vol_3_ne_touche_pas_le_gabarit_de_triage`) sont, elles, armées sur `main` depuis
#2874 et tournent sur chaque PR sans aucun appel de modèle. Elles ne sont pas touchées : ni
armement, ni population, ni assertion. Aucune exception d'allowlist n'est créée, et il n'y en a
aucune à retirer.

Pourquoi pas (a), l'exception nommée : il n'existe aucune violation existante à exempter — la
mesure n'a pas été faite, ce qui n'est pas la même chose qu'un résultat connu à tolérer (*un
`null` n'est jamais un `0`*, mika#2331). Pourquoi pas (c), la halte : rien ne demande de cadrage
opérateur dans ce vol ; la seule décision opérateur restante — armer l'interrupteur — est déjà
nommée, déjà hors périmètre, et reste exactement là où Vincent l'a mise le 2026-10-22.

Un détecteur qui rougirait **quand un dormeur se réveille** serait, lui, un livrable neuf : il
demanderait d'évaluer une condition de réveil, donc un `gh` en CI sur un dépôt tiers. C'est
l'arbitrage que le dépôt confie explicitement à l'opérateur (« Le bassin dit donc ce que le
corps dit, et l'opérateur juge », `milestone_manager/pool.rs`), et l'y ramener appartient à
l'hygiène du registre — § *Hors périmètre*.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter/tests/ac1_vrai_modele_2854.rs` (en-tête + 2 messages de saut) | 18 |
| `docs/dormeurs.md` (une entrée au registre) | 0 — `docs/`, hors compte |
| `docs/solutions/dev-loop/une-condition-de-reveil-circulaire-…-2026-10-31.md` | 0 — `docs/`, hors compte |

Total estimé : 18 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et il tient largement.** 18 lignes contre une borne de 429 : il n'y a pas de
découpage à faire, et il n'y en a pas à inventer. La raison du volume est la mesure du § *L'état
sur `main`* : les sept AC sont livrées, et ce qui reste n'est pas du code mais un changement de
support. Le poste de dépense de ce vol est en tours de lecture, pas en lignes écrites — d'où
l'inventaire d'état fait **ici**, dans le plan, pour que le pilote n'ait pas à le refaire.

## Definition of Done

- `docs/dormeurs.md` porte une entrée `#2854` conforme aux trois colonnes du § *Contrat d'une
  entrée*, dont une condition de réveil qu'une seule commande tranche.
- La cellule *sujet* nomme l'instrument sur `main`, sa commande de rejeu, et la règle « un rouge
  est un constat à rendre à Vincent » (mika#1190).
- `ac1_vrai_modele_2854.rs` cite `docs/dormeurs.md` dans son en-tête et dans ses deux messages
  de saut ; `TRIAGE_SYSTEM_PROMPT` est à zéro octet de diff.
- Le plancher est inchangé : `MIKA_WEBHOOK_COMMENT_TRIAGE` absent vaut `off`, et son test est
  vert.
- `docs/solutions/dev-loop/` porte la leçon, avec son *frontmatter*.
- La suite complète, `clippy`, `fmt`, `make lint` et `scripts/verify-pipeline.sh origin/main`
  sont verts.
- Le corps de PR porte `Closes #2854`, dit que la mesure d'AC1 est reportée, et nomme son
  traceur : `Tracked in: senara-solutions/claude-pilot#290` (le bloqueur réel) et
  `Tracked in: senara-solutions/mika#2854` (ligne vivante de `docs/dormeurs.md`, rouverte au
  réveil). Les deux lignes satisfont la porte de mika#527 sur la phrase de report.

## Plancher de ce vol

`MIKA_WEBHOOK_COMMENT_TRIAGE` reste à `off` par défaut, et ce vol ne touche pas une ligne de
`comment_triage.rs`. L'armement reste une décision de Vincent, prise sur le chiffre que la
mesure d'AC1 produira — pas avant. La classe (h) continue donc d'ouvrir le tour complet en
production, exactement comme aujourd'hui, et le seul effet observable du vol est documentaire.

## Hors périmètre

- **La mesure d'AC1 elle-même.** Elle reste dormante, et c'est tout l'objet du vol de le dire là
  où un lecteur le trouve. Aucune tentative de la jouer : depuis le bac à sable aucun run
  d'éval n'atteint un fournisseur, et c'est une propriété de sûreté voulue.
- **L'armement de l'interrupteur.** Décision de Vincent, en aval de la mesure.
- **Toute retouche de `TRIAGE_SYSTEM_PROMPT`**, de la grammaire du verdict, du budget ou du
  corpus : mesurer un gabarit et le corriger dans le même vol, c'est mesurer deux fois rien
  (mika#1190).
- **La lecture de dormance aux portes du dispatch.** Défaut réel, mesuré au § *Pourquoi le
  support doit changer*, mais il appartient à mika#2952 / mika#2956. § D5.
- **Un vérificateur structurel de `docs/dormeurs.md`.** Le fichier déclare un contrat que rien
  n'applique, ce qui est une vraie lacune ; elle appartient à l'hygiène du registre, pas à la
  classe (h).
- **(g′) de mika#2680**, et les classes (a) à (g) du pré-filtre, inchangées.

## Acceptance criteria

Les sept AC du corps de mika#2854 sont transcrites telles quelles, avec leur état mesuré ; les
trois AC du vol 4 sont celles que ce vol livre.

- **AC1 — corpus de contrôle négatif.** Un jeu de commentaires réels porteurs d'une décision
  (lever `operator-gated`, « je valide », fermer, lancer un vol, trancher Q1/Q2) escalade
  **chacun** vers le tour complet, le rouge étant mesuré avec un triage qui note tout.
  *État : instrument livré par #2874, mesure **dormante**. Ce vol ne la rend pas verte — il la
  rend réveillable, par une condition qu'un lecteur peut trancher.*
- **AC2 — contrôle positif.** Un jeu de commentaires sans décision (accusé, note, lien) donne
  une note, sans tour complet. *Livré (#2858, rejoué par #2874).*
- **AC3 — fail-open.** Une erreur, un timeout ou une sortie illisible du triage donne le tour
  complet, et c'est asserté. *Livré (#2858).*
- **AC4** — `issues` `closed` sans tâche suit la même règle que `issue_comment`. *Livré
  (#2873).*
- **AC5** — la ligne d'audit et `skipped_by_class` portent la classe (h) et la branche prise.
  *Livré (#2858).*
- **AC6** — le tour léger ne reçoit aucun outil mutationnel, mesuré sur le tableau servi.
  *Livré (#2858).*
- **AC7** — un ticket **avec** tâche de boucle n'est pas touché et passe par le chemin
  d'aujourd'hui. *Livré (#2858).*
- **AC8 (vol 4) — la dormance d'AC1 vit dans le registre, avec une condition tranchable.**
  `docs/dormeurs.md` porte une entrée `#2854` dont la condition de réveil est décidée par une
  seule commande (`gh issue view 290 --repo senara-solutions/claude-pilot --json state -q .state`
  rend `CLOSED`). La condition ne mentionne pas l'armement de `MIKA_WEBHOOK_COMMENT_TRIAGE` : la
  circularité du corps est levée, pas déplacée.
- **AC9 (vol 4) — l'instrument dit où est sa procédure de réveil.** Les deux tests `#[ignore]`
  d'AC1 et l'en-tête de leur module citent l'entrée `docs/dormeurs.md` ; `TRIAGE_SYSTEM_PROMPT`
  est à zéro octet de diff et aucune assertion n'est modifiée.
- **AC10 (vol 4) — la fermeture est non destructive et le plancher est intact.** La PR porte
  `Closes #2854` et ses deux lignes `Tracked in:` ; `MIKA_WEBHOOK_COMMENT_TRIAGE` absent vaut
  toujours `off`, et `mika2854_labsence_de_variable_laisse_le_triage_desarme` est vert.

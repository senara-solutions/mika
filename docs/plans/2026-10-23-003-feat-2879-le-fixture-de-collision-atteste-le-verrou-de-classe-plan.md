# mika#2879 — vol 4/4 : le fixture de collision atteste le verrou de classe

**Ticket :** senara-solutions/mika#2879 — *Vol 2 nommé par mika#2878 (suivi de #2872), verdict du site 2*
**Vol :** 4 sur 4. Dernier vol du ticket ; il ferme **AC3** et clôt le ticket.
**Périmètre :** tests uniquement. **Zéro ligne de production.**

---

## Où en est le ticket

Les trois gestes du corps sont désormais répartis sur quatre vols, dont trois sont
mergés :

| vol | geste | commit | PR |
|---|---|---|---|
| 1 | le verdict amont (mika#2872/#2878) : la forme typée, la revérification, le verrou de classe, la porte désarmée | `d51da321` | #2878 |
| 2 | geste 1 — séparer les deux natures dans le détecteur, à comportement de détection inchangé (`FrequencyAssertionNature`) | `9341c7c0` | #2883 |
| 3 | geste 2 — rattacher **la seule** moitié accomplie de 5e à la classe `fabrication-detected` | `d0025b74` | #2893 |
| **4** | **geste 3 — le fixture de collision qui atteste le verrou de classe (AC3)** | — | — |

Le vol 3 a nommé ce reliquat dans son propre plan, et le nomme comme tel :

> **Tracked in:** senara-solutions/mika#2879 — vol 4 : le fixture de collision 5d +
> `assert_grounded` qui atteste le verrou de classe (AC3), avec le rouge-avant R6/R7
> ci-dessus qui doit y rougir. Le ticket reste ouvert comme porteur de ce vol.

Deux recettes de rouge-avant du vol 3 sont restées **vertes faute de second écrivain
dans le même tour** et attendent ici :

- **R6** — retirer le terme de nature du **dépôt** 5e (`promise.nature == AccomplishedState`
  à `agent_loop/mod.rs:3095`) : vert, parce que sans second site dans le tour le créneau
  n'est jamais convoité.
- **R7** — retirer le verrou de classe du dépôt 5e (`pending_fabrication_p0.is_none()`
  à `agent_loop/mod.rs:3094`) : vert, pour la même raison.

Un terme dont aucun test ne peut rougir est un terme que le prochain refactor retire en
silence. C'est ce vol qui le rend rougissable.

---

## Le constat technique

### La chaîne, et ce que chaque pas consomme

Trois sites de la classe `fabrication-detected` vivent dans la même chaîne de gardes
EndTurn de `crates/mika-agent/src/agent_loop/mod.rs`, dans cet ordre :

| ordre | garde | premier tir (`continue`) | résidu (dépôt P0) | site typé |
|---|---|---|---|---|
| 5d | `false_local_hosting_claim` | 2862 | 2876–2936, dépôt à **2909** | `FalseLocalHosting` |
| 5e | `unactioned_frequency_promise` | 2964–3038 | 3049–3124, dépôt à **3093** | `UnactionedFrequencyPromiseAccomplished` |
| AG | `assert_grounded` | ~3700 | 3836–3878, dépôt à **3862** | `AssertGrounded` |

Chaque garde dispose d'un budget d'un seul re-prompt, matérialisé par
`intent_guard_retries` : le premier tir insère son label et fait `continue` ; au tour
suivant le même label interdit le premier tir et le bras *uncorrected* prend le relais —
sans `continue`, donc sans consommer le pas. Le dépôt P0 vit dans ce bras résiduel, sous
le verrou de classe `pending_fabrication_p0.is_none()` : **une seule P0 par tour pour
toute la classe, le premier écrivain gagne**. L'émission se fait à l'acceptation
(4582–4597), après revérification par le prédicat du site déposant
(`PendingFabricationP0Site::still_violated`).

### Pourquoi le verrou de classe est aujourd'hui inattestable

Un fixture mono-site ne convoite jamais le créneau : le seul écrivain du tour le prend,
et retirer le verrou ne change rien d'observable. C'est exactement le diagnostic de R6/R7.
Il faut un tour qui fasse **écrire deux sites différents**, et que le marqueur émis dise
lequel a gagné.

### Le fixture A — la collision triple, et son déroulé pas à pas

Une **seule** réponse qui viole simultanément 5d, 5e (nature accomplie) et
`assert_grounded`, rejouée identique à chaque re-prompt, parcourt quatre pas :

| pas | 5d | 5e | AG | effet |
|---|---|---|---|---|
| 1 | premier tir → `continue` | — | — | 1 re-prompt |
| 2 | résidu → **dépose** (`FalseLocalHosting`) | premier tir → `continue` | — | créneau pris par 5d |
| 3 | résidu, verrou ferme | résidu → **bloqué par le verrou** | premier tir → `continue` | — |
| 4 | résidu, verrou ferme | résidu, verrou ferme | résidu → **bloqué par le verrou** | acceptation |

À l'acceptation, `FalseLocalHosting::still_violated` est vrai (le texte porte toujours
l'assertion de localité, le déploiement reste non-local) : **une seule émission, en P0,
portant le marqueur de 5d.**

Ce seul fixture rend rougissables **deux** verrous :

- retirer le verrou de classe du dépôt **AG** (3862) → AG écrase au pas 4, l'émission
  porte le marqueur d'AG, l'assertion sur le marqueur de 5d rougit ;
- retirer le verrou de classe du dépôt **5e** (3094) → 5e écrase au pas 3, AG est ensuite
  bloqué par un créneau déjà pris, l'émission porte le marqueur de 5e → rouge.

Le second est précisément **R7 du vol 3**.

### Le fixture B — le terme de nature protège le créneau, pas l'émission

R6 ne tombe pas avec le fixture A : au pas 3, le créneau est déjà pris par 5d, donc 5e est
bloqué par le verrou **avant** que le terme de nature n'ait son mot à dire. Le terme ne
devient observable que dans un tour où 5e(engagement) est le **premier** écrivain
candidat et où un site **aval** veut le créneau. D'où un second fixture : 5e de nature
*engagement sur l'avenir* **+** `assert_grounded`, **sans** 5d.

| pas | 5e | AG | effet |
|---|---|---|---|
| 1 | premier tir → `continue` | — | 1 re-prompt |
| 2 | résidu → nature = engagement → **ne dépose pas** | premier tir → `continue` | créneau libre |
| 3 | résidu → ne dépose pas | résidu → **dépose** (`AssertGrounded`) | acceptation |

À l'acceptation : **une émission, en P0, portant le marqueur d'`assert_grounded`.**

Rouge-avant : retirer le terme de nature au dépôt 5e (3095) → 5e(engagement) **occupe le
créneau** au pas 2, AG est bloqué au pas 3, et à l'acceptation
`UnactionedFrequencyPromiseAccomplished::still_violated` lit une nature ≠
`AccomplishedState` → faux → **zéro émission**. La P0 d'`assert_grounded` est perdue.
C'est le coût que le commentaire de production nomme déjà (`mod.rs:3087–3092`) :

> Le terme protège le créneau, là où la revérification protège l'émission.

Le fixture B est l'attestation de cette phrase.

### Ce qui reste structurellement inattestable, et pourquoi on le nomme

**Le verrou de classe du dépôt 5d (2909) ne peut pas rougir.** 5d est le premier site de
la chaîne : aucun écrivain ne le précède, donc retirer son `is_none()` lui fait au mieux
re-déposer son propre message, à l'identique, aux pas 3 et 4 — l'émission reste une, avec
le même marqueur. Le test restera vert sous cette injection.

Ce vert est écrit **dans l'en-tête du fichier, avec sa raison**. Un rouge-avant vert non
nommé est un rouge-avant qui ment ; un rouge-avant vert nommé est une limite documentée
de l'instrument. Le rendre rougissable demanderait un quatrième site **en amont** de 5d
dans la classe — hors périmètre de ce ticket, et hors besoin : le terme est le même code
que celui que les deux autres sites attestent.

---

## Décisions de conception

**D1 — Les fixtures de collision sont intégralement anglophones.**
`AFFIRMATIVE_STATE_CLAIM_PATTERNS` (`guards.rs:227–252`) n'a que des motifs **anglais** et
exige `issue|PR|pull request|task|ticket` suivi d'un `#N`. Les fixtures des vols 2 et 3
sont francophones. Or la garde **5f `response_language_drift`** (mika#2247 AC2) vit dans la
même chaîne, juste après 5e : une réponse de langue divergente de celle de la question du
tenant lui ferait prendre le tour, et le test serait vert pour la mauvaise raison. Donc :
question du tenant **en anglais**, réponses **en anglais**. Les deux détecteurs amont le
permettent — `LOCALITY_ALTERNATION` porte `locally` et `on your machine|computer|device|…`,
`ASSERTION_POSITIVE_ALTERNATION` porte `i'?m running` et `your data stays|lives|…`,
`FREQUENCY_ASSERTION_ACCOMPLISHED_STATE` porte `i'?ve (fixed|stopped|reduced|…)`,
`FREQUENCY_SUBJECT_ALTERNATION` porte `daily digest…s?`.

**D2 — L'assertion porte sur le marqueur, pas seulement sur le compte.**
Compter « une seule émission » ne distingue pas *le verrou tient* de *un site aval a écrasé
l'amont* : les deux donnent 1. Chaque test assertionne donc **(a)** exactement une émission
dans la classe, **(b)** son texte contient le marqueur du site attendu, **(c)**
`assert_ne!(*etage, SEVERITE_ABSENTE)` et `NotificationSeverity::P0`. Le (c) n'est pas
décoratif : `send_with_severity` a un corps par défaut qui délègue à `self.send(text)`
(mika#1381), donc une régression vers `send()` perdrait l'étage **en silence**.

**D3 — Anti-vacuité par `trace.llm_call_count > 1`.**
Un fixture qui ne déclencherait aucune garde produirait aussi « une seule émission :
aucune », donc zéro émission — vert par vacuité si l'assertion était seulement
« pas de P0 parasite ». Chaque test exige que le tour ait bien été re-prompté.

**D4 — Un fichier dédié, pas une extension du fichier du vol 3.**
`test_fabrication_detected_p0_2879.rs` porte la doctrine de la *séparation des natures* ;
la collision est une autre doctrine (le verrou de classe) et pèse son propre en-tête. Un
nouveau fichier `test_fabrication_detected_p0_2879_collision.rs` garde chaque en-tête
lisible. En contrepartie, les lignes R6/R7 de l'en-tête du vol 3 sont **mises à jour** pour
nommer le fichier où elles rougissent désormais — un renvoi « au vol suivant » qui ne
nomme pas sa destination est un renvoi perdu.

**D5 — Un helper de jeu mutualisé.**
Les deux tests partagent le montage `EvalHarness` + `SeverityRecordingSender` +
`MockLlmProvider`. Un seul `fn joue(question, responses) -> (Vec<Emission>, u32)` les
sert tous les deux, comme le vol 3 avait mutualisé `aucune_p0_de_classe`. C'est le levier
qui tient le vol sous sa borne.

**D6 — La séquence mock est dimensionnée avec marge.**
Les comptes de pas (4 et 3) sont dérivés de la chaîne telle qu'elle est lue aujourd'hui ;
une garde amont non anticipée les décalerait. La séquence de récidive est donc dimensionnée
à **6** répétitions. Les assertions portent sur l'émission, jamais sur le nombre de pas,
donc une réponse non consommée n'est pas un échec. Si le premier run révèle qu'une garde
tierce prend le tour, le fixture est ajusté — c'est le rôle du run vert avant le
rouge-avant.

---

## Le périmètre, livrable par livrable

### 1. `crates/mika-agent/tests/eval/test_fabrication_detected_p0_2879_collision.rs` (nouveau)

En-tête `//!` de doctrine, dans la forme de la famille (`test_fabrication_detected_p0_2872.rs`,
`…_2879.rs`) : le constat, la chaîne et ses pas, ce que chaque test atteste, puis le tableau
des recettes de rouge-avant avec leur résultat **observé** — y compris le vert nommé de R3.

Constantes :

- `MARQUEUR_5D = "Fabrication detected (uncorrected) on deployment locality"`
- `MARQUEUR_5E = "Fabrication detected (uncorrected) on message frequency"`
- `MARQUEUR_AG = "Fabrication detected (uncorrected)"` — préfixe commun, donc la
  discrimination se fait par **absence** des deux marqueurs qualifiés ci-dessus, et le test
  le dit en commentaire plutôt que de l'assumer.
- `QUESTION_DU_TENANT` (anglophone, D1)
- `COLLISION_TRIPLE` — la réponse qui viole 5d + 5e(accompli) + AG
- `ENGAGEMENT_ET_ANCRAGE` — la réponse qui viole 5e(engagement) + AG, sans 5d

Helpers : `fn recidive(texte: &str) -> Vec<MockResponse>` (6 répétitions, D6) et
`fn joue(...)` (D5).

Tests :

1. `mika2879_une_collision_triple_nemet_quune_seule_p0_de_classe` — fixture A. Atteste
   AC3 et, par ses injections, le verrou de classe des dépôts AG **et** 5e (R7 du vol 3).
2. `mika2879_la_nature_au_depot_protege_le_creneau_du_site_aval` — fixture B. Atteste que
   le terme de nature au dépôt 5e laisse le créneau au site aval (R6 du vol 3).

### 2. `crates/mika-agent/tests/eval.rs`

Une ligne : déclaration du module de test.

### 3. `crates/mika-agent/tests/eval/test_fabrication_detected_p0_2879.rs`

Mise à jour des deux lignes R6/R7 de l'en-tête : remplacer « reporté au vol suivant » par
le nom du fichier et du test qui les font rougir. Aucun changement de code.

### Hors périmètre

- **Aucune ligne de production.** Les trois dépôts, la revérification, les verrous et la
  porte sont déjà livrés (vols 1–3). Ce vol les atteste, il ne les modifie pas.
- `crates/mika-common/src/config.rs` n'est pas touché : la porte `fabrication_p0_notify`
  reste à `false` par défaut (AC5). Son armement est le périmètre de mika#2871.
- Le verrou de classe du dépôt 5d reste inattestable, et nommé comme tel (ci-dessus).

---

## Ordre de commit

Un seul commit de travail, et le rouge-avant joué **après** lui, sur l'arbre vert :

1. **`test(guards): le fixture de collision atteste le verrou de classe de fabrication-detected (mika#2879, vol 4/4)`**
   — les trois livrables ci-dessus en un seul commit. Un test et sa déclaration de module
   ne sont pas mergeables séparément.

Discipline du rouge-avant (AC4), pour chaque recette : l'injection est jouée sur l'arbre
vert, la suite est lancée, l'injection est annulée par `git checkout <fichier>`, et le
résultat **observé** — rouge comme vert, avec sa raison — est écrit dans l'en-tête du
fichier de test, puis le commit est amendé. Aucune injection n'est livrée.

**Plancher :** le commit 1. En deçà, il n'y a rien à merger.

---

## Contrat de vérification

| recette | injection | attendu | ce qu'elle atteste |
|---|---|---|---|
| **R1** | retirer `pending_fabrication_p0.is_none()` du dépôt **AG** (`mod.rs:3862`) | **ROUGE** — test 1 : l'émission porte le marqueur d'AG, pas celui de 5d | le verrou de classe du site aval |
| **R2** | retirer `pending_fabrication_p0.is_none()` du dépôt **5e** (`mod.rs:3094`) | **ROUGE** — test 1 : l'émission porte le marqueur de 5e | le verrou de classe du site médian — **R7 du vol 3** |
| **R3** | retirer `pending_fabrication_p0.is_none()` du dépôt **5d** (`mod.rs:2909`) | **VERT, nommé** — 5d re-dépose son propre message, l'émission est inchangée | la limite de l'instrument : le premier écrivain de la chaîne n'a pas de témoin amont |
| **R4** | retirer `promise.nature == FrequencyAssertionNature::AccomplishedState` du dépôt **5e** (`mod.rs:3095`) | **ROUGE** — test 2 : zéro émission (5e occupe le créneau, la revérification l'annule) | l'appartenance à la classe protège le créneau — **R6 du vol 3** |
| **R5** | retirer le bras `UnactionedFrequencyPromiseAccomplished` de `still_violated` (`guards.rs:1103–1108`) → `true` constant | **ROUGE** sous R4 seul (l'émission revient, avec le marqueur de 5e) | la revérification et l'appartenance sont deux termes distincts, pas un doublon |

Plus, sur l'arbre vert et sans injection :

- `cargo test -p mika-agent --test eval` — toute la suite eval, les fixtures des vols 2 et
  3 compris (AC2 : l'ensemble des réponses qui font tirer le détecteur est inchangé — aucune
  ligne de détecteur ne bouge dans ce vol, donc la non-régression est structurelle).
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `make lint` et `scripts/verify-pipeline.sh origin/main`.

---

## Fire-Disposition

Ce vol livre un détecteur au sens de mika#2306 : deux tests dont le chemin de succès est
« aucune violation trouvée ».

**Option retenue : (a) — exception nommée en allowlist, avec zéro entrée.**

Détail d'implémentation : les deux tests atterrissent **armés** (pas de `#[ignore]`, pas de
`cfg` d'exclusion) et **verts** sur le commit de livraison. L'allowlist est vide et sans
objet structurel : ce sont des tests de comportement sur fixtures synthétiques joués par
`MockLlmProvider`, non un scan de source — il n'existe aucune violation existante dans le
dépôt à excepter, donc aucune entrée à écrire et aucune assertion auto-nettoyante à
adosser. Les cinq recettes du contrat de vérification sont ce qui établit que ce vert n'est
pas un vert par vacuité : chaque test ne rougit que sous une injection nommée (R1, R2, R4,
R5), et le seul vert sous injection (R3) est documenté avec sa cause structurelle dans
l'en-tête du fichier.

La porte de production `fabrication_p0_notify` reste à `false` par défaut — c'est l'option
(b) du vol 1, reconduite telle quelle et **non rejugée ici** : ce vol ne touche pas
`crates/mika-common/src/config.rs` (AC5). Les tests arment la porte localement via
`.fabrication_p0_notify(true)` sur le `EvalHarness`, ce qui est le seul moyen d'exercer le
chemin, et n'affecte aucun déploiement.

**Tracked in:** senara-solutions/mika#2871 — l'armement de la porte
`MIKA_FABRICATION_P0_NOTIFY` en production, hors périmètre de ce ticket par AC5.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `tests/eval/test_fabrication_detected_p0_2879_collision.rs` — en-tête de doctrine + 5 recettes observées | 70 |
| … fixtures anglophones (5 consts) + `recidive()` | 55 |
| … helper `joue()` mutualisé | 25 |
| … test 1 — collision triple, verrou de classe | 55 |
| … test 2 — nature au dépôt, créneau du site aval | 40 |
| `tests/eval/test_fabrication_detected_p0_2879.rs` (renvois R6/R7 mis à jour) | 8 |
| `tests/eval.rs` (déclaration de module) | 1 |

Total estimé : 254 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 254 < 279, et la marge est mince par choix : la famille a un
facteur estimation→réel mesuré à ≈ 2,5 sur les estimations naïves (vol 2 de #2872 : 150
estimées pour 369 réelles ; vol 3 : 188 pour 352), donc l'estimation ci-dessus est déjà
calibrée sur le **réel observé de la famille** — deux tests et un en-tête de cinq recettes,
contre trois tests et huit recettes pour 352 lignes au vol 3 — et non sur un décompte à
plat. Le reste du vol est léger en tours : zéro ligne de production, un seul commit de
travail, et cinq injections de rouge-avant d'une ligne chacune, chacune annulée par un
`git checkout` d'un seul fichier. Le levier de compression est D5 (un helper de jeu pour
les deux tests) ; s'il ne suffisait pas, c'est le test 2 qui se détache en vol propre, pas
l'en-tête qui se raccourcit.

Aucun découpage en vols supplémentaires n'est donc nécessaire, et il n'y a pas de suite à
tracker côté périmètre : ce vol ferme AC3 et le ticket. Le seul renvoi est celui de la
Fire-Disposition ci-dessus (mika#2871, armement de la porte), déjà hors périmètre par AC5.

---

## Definition of Done

- [ ] `test_fabrication_detected_p0_2879_collision.rs` existe, est déclaré dans
      `tests/eval.rs`, et ses deux tests passent sur l'arbre vert.
- [ ] Le test 1 atteste qu'un tour violant 5d + 5e(accompli) + `assert_grounded` émet
      **exactement une** notification de classe `fabrication-detected`, en
      `NotificationSeverity::P0`, portant le marqueur de **5d**.
- [ ] Le test 2 atteste qu'un tour violant 5e(engagement) + `assert_grounded` émet
      exactement une notification, en P0, portant le marqueur d'`assert_grounded`.
- [ ] Les cinq recettes R1–R5 ont été jouées sur le commit vert, et leur résultat
      **observé** est écrit dans l'en-tête du fichier — R3 inclus, avec sa raison
      structurelle.
- [ ] Les lignes R6/R7 de l'en-tête de `test_fabrication_detected_p0_2879.rs` nomment le
      fichier et le test où elles rougissent désormais.
- [ ] `crates/mika-common/src/config.rs` est inchangé : `fabrication_p0_notify` reste
      `false` par défaut.
- [ ] Aucune ligne de `crates/mika-agent/src/` n'est modifiée par le commit livré.
- [ ] `cargo test -p mika-agent --test eval` vert dans son intégralité ;
      `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` propres ;
      `make lint` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte `Closes senara-solutions/mika#2879` et la ligne
      `Tracked in: senara-solutions/mika#2871`.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2879. Les vols 1–3 ont déjà fermé
AC1, AC2 et AC5 ; ce vol ferme **AC3** et refait AC4 sur son propre périmètre.

- **AC1** : une réponse d'état accompli que le terme (C) contredit lève la récidive de
  classe `fabrication-detected`. Une réponse d'engagement sur l'avenir ne la lève pas. Les
  deux sont testées.
  *(Fermé par le vol 3, `d0025b74`. Ce vol le renforce : le test 2 atteste la même
  frontière depuis l'autre bord — l'engagement ne prend même pas le créneau.)*
- **AC2** : l'ensemble des réponses qui font tirer le détecteur est inchangé (test de
  non-régression sur les fixtures existants).
  *(Fermé par le vol 2, `9341c7c0`. Tenu structurellement ici : aucune ligne de détecteur
  n'est modifiée, et la suite eval complète est rejouée.)*
- **AC3** : le fixture de collision 5d + `assert_grounded` émet une seule P0 par tour.
  Retirer le verrou de classe le fait rougir.
  *(**Le périmètre de ce vol.** Test 1 + recettes R1 et R2. Le fixture dépasse la lettre de
  l'AC — il viole aussi 5e, donc il atteste le verrou sur deux sites au lieu d'un.)*
- **AC4** : rouge-avant, terme par terme, sur le commit vert.
  *(Recettes R1–R5, jouées sur le commit 1 et consignées dans l'en-tête avec leur résultat
  observé.)*
- **AC5** : la porte reste désarmée par défaut. Son armement relève de mika#2871.
  *(Tenu : `config.rs` n'est pas touché. Voir Fire-Disposition.)*

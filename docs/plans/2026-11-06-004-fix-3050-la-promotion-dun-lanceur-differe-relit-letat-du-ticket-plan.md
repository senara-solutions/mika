# mika#3050 — vol 3 : la promotion d'un lanceur différé relit l'état du ticket

> Ticket : senara-solutions/mika#3050
> Vols antérieurs : `docs/plans/2026-11-06-002-fix-3050-le-reaper-relit-le-ticket-avant-de-rearmer-plan.md` (PR #3052, `a645a6f9b`) et `docs/plans/2026-11-06-003-fix-3050-le-recensement-de-la-garde-de-rearmement-plan.md` (PR #3053, `a2e347ef6`).
> Base : `a2e347ef6`.

## 0 — Le défaut résiduel, et le fil qui le tenait

Les trois critères AC1–AC3 sont livrés **sur le chemin de création** d'un lanceur
différé : `rearm_deferred_callback` (`executor.rs:6180`) relit le ticket avant de
reconstruire un lanceur, par le prédicat d'**état** `reread_issue_consent_via_gh`
(`ready_label.rs:706`). L'instrument d'AC4 est livré lui aussi
(`RearmConsentCensus`, huit branches, écrivain unique).

Il reste un chemin par lequel un tour mika-dev part encore sans consentement : la
**promotion d'un lanceur déjà en file**. Le plan du vol 1 avait nommé l'asymétrie
dans son § 0, en ne fermant que la moitié création :

> **Pourquoi mika#2995 ne couvre pas ce cas, et c'est le point de conception du
> vol.** mika#2995 garde déjà la *promotion* d'un lanceur différé, mais avec le
> prédicat de **parc de la timeline** […] Le retrait de mika#3022 est fait par une
> identité **machine**, donc le prédicat de parc rend « non parqué » →
> `ConsentHeld` → le tour part. C'est exactement le cas `0529ed9b`/#3044.

Vérifié sur `a2e347ef6` : la garde de promotion
`skip_deferred_dispatch_on_withdrawn_consent_with` (`dispatcher.rs:1444`) n'a
qu'une source de verdict, `park_verdict_for_issue_with`, et rend `ConsentHeld` sur
son `Ok(None)`. Le prédicat de parc est documenté comme rendant **non parqué** sur
un `unlabeled` machine (`ready_label.rs:50`, D2). Depuis mika#3022, `ready` est
retiré à chaque merge d'une PR de la boucle par une identité machine
(`merged_pr_ready_cleanup_handler.rs:260`). Un lanceur promu sur un ticket dont
`ready` a été retiré par la machine passe donc la garde, et le tour est payé.

Le prédicat d'**état** a exactement un lecteur sur `a2e347ef6`,
`executor.rs:5994`, dans `reread_consent_verdict`. Le site qui **dépense** le tour
n'en a aucun.

### Pourquoi c'est dans le périmètre d'AC1, et non un vol neuf

AC1 porte la clause *« Si l'un des deux manque, aucun lanceur n'est créé, **et
aucun tour mika-dev n'a lieu** »*. Les vols 1 et 2 ont fermé la première moitié
(« aucun lanceur n'est créé ») ; la seconde (« aucun tour n'a lieu ») reste ouverte
pour le lanceur **déjà en file**, que le vol 1 n'a pas créé et ne pouvait donc pas
garder. La liste `### Hors vol` du ticket exclut le défaut 2 (doctrine de
fermeture), les lanceurs différés des autres classes, et les phases d'auto_pull :
elle **n'exclut pas** le chemin de promotion.

Un renfort du même constat : la garde 2 de `rearm_deferred_callback`
(`AlreadyRepresented`, mika#2413) court-circuite **avant** la relecture d'état.
Une parente déjà représentée par un lanceur vivant n'est donc jamais relue du tout
— ce qui confirme que la promotion est l'endroit où le tour restant se paie.

## 1 — Le point de conception : composer deux prédicats sans fusionner deux populations

Le site de promotion doit consulter les deux prédicats. Trois décisions tranchent
la forme.

### D1 — Le parc d'abord, l'état **seulement** sur `ConsentHeld`

Un retrait humain de `ready` rend le ticket à la fois **parqué** (mika#2995) et
**sans `ready`** (mika#3050). Les deux prédicats le refuseraient. L'ordre décide
donc sous quelle raison la population est comptée, et AC4 de mika#2995 est un
compte sur sa raison propre.

Si l'état passait d'abord, chaque parc humain basculerait vers la raison mika#3050
et le compte de mika#2995 tomberait à zéro sans que rien n'ait régressé — deux
populations sous un seul nom, ce que mika#2205 interdit. **Le parc passe donc
d'abord, inchangé, et l'état n'est consulté que sur son `ConsentHeld`.** Les deux
populations restent disjointes et soustractables : parc = retrait non-machine,
état = ticket fermé ou `ready` absent après un verdict de non-park.

### D2 — Appeler le prédicat d'état, ne pas le redériver de la timeline

La timeline est déjà lue par le prédicat de parc, et « `ready` est posé » s'en
dérive (le dernier événement de `E` est un `labeled`). Zéro appel réseau de plus
— et c'est précisément ce qu'il ne faut pas faire : ce serait un **second
lecteur** de « ouvert et portant `ready` », avec une seconde implémentation, contre
la doctrine du lecteur unique (mika#2158, mika#2205, mika#3049). Le vol appelle
`reread_issue_consent_via_gh`, qui reste le site unique.

Le prix est un `gh issue view` par promotion de parente `ready-label` — borné à une
promotion par parente, contre 1 à 20 pages pour la timeline déjà payée.

Le gain est décisif par ailleurs : l'état lit `state` (ouvert/fermé), que la
timeline de `ready` ne donne pas. AC1 exige les deux conditions ; le parc seul ne
peut pas en rendre une.

### D3 — La fenêtre `remove`→`add` de la phase 2 n'est pas atteignable ici

C'est le risque de faux positif du vol, et il doit être tranché et non supposé.
La propriété D2 du prédicat de parc existe pour tolérer un cas précis
(`ready_label.rs:56`) :

> **Le remove→add de la Phase 2 ne se parque pas lui-même.** Entre son `remove` et
> son `add`, le dernier événement est un `unlabeled` **machine**.

Dans cette fenêtre, le prédicat d'état rendrait `ReadyAbsent` et la garde
refuserait un tour légitime. Vérifié sur `a2e347ef6` : les trois retraits de
`ready` d'`auto_pull` se répartissent en deux formes.

| site | forme | effet sur la garde |
|---|---|---|
| `auto_pull.rs:4272` (refus de promotion) | retrait **définitif**, précédé de `REFUSAL_LABEL` qui exclut le ticket de toutes les phases | refuser est exactement juste |
| `auto_pull.rs:4449` (`abandon_stuck_ready`) | retrait **définitif**, précédé d'`ABANDON_LABEL` (`operator-review`) | refuser est exactement juste |
| `auto_pull.rs:6415` → `:6426` (`phase2_stuck_rescue`) | le seul `remove`→`add` réel | **inatteignable** — voir ci-dessous |

Le seul `remove`→`add` est précédé, à `auto_pull.rs:6401`, du court-circuit
`churn_moot` (`churn_is_moot(&action, engine_holds_ticket)`) :

> The engine already queued this ticket: no age to reset, nothing for a redundant
> trigger to do but collide. […] the ticket sits in the engine's deferred queue

Quand le moteur tient déjà le ticket — exactement la condition sous laquelle un
lanceur différé attend sa promotion — la phase 2 fait `continue` **avant** le
`remove`, et compte le ticket sous `FILTER_IN_FLIGHT`. La fenêtre et la garde ne
peuvent donc pas se croiser.

Conséquence : le refus sur `ready` absent peut être **terminal**, comme AC1
l'exige, sans qu'un geste légitime de la boucle puisse le déclencher. La
non-atteignabilité est une propriété d'un autre module, donc elle est épinglée par
un test de la forme AC-T2 (§ 4, T6) plutôt que laissée à ce paragraphe.

## 2 — Les verdicts, et la matrice fail-closed

`WithdrawnConsentVerdict` (`dispatcher.rs:733`) gagne trois variantes. Aucune ne
réutilise `ConsentWithdrawn` : son motif `READY_CONSENT_WITHDRAWN_REASON` est le
compte de mika#2995.

| verdict | condition | effet | raison d'audit |
|---|---|---|---|
| `ConsentHeld` | parc `Ok(None)` **et** état `Held` | le tour part, inchangé | — |
| `ConsentWithdrawn` | parc rend un park | terminal (existant) | `ready_consent_withdrawn` |
| `TimelineUnreadable` | parc en erreur, ou identités machine vides | report, pas de terminal (existant) | — |
| `ReadyAbsentAtPromotion` | parc `Ok(None)`, état `ReadyAbsent` | **terminal** `failed` sur la parente | `ready_absent_at_rearm` |
| `IssueClosedAtPromotion` | parc `Ok(None)`, état `Closed` | **terminal** `failed` sur la parente | `issue_closed_at_rearm` |
| `StateUnreadable { detail }` | parc `Ok(None)`, état `Err` | **report**, aucun terminal | — |

Les deux raisons terminales sont celles qu'AC1 nomme, et elles existent déjà,
livrées par le vol 1 : `READY_ABSENT_AT_REARM_REASON` (`ready_label.rs:178`) et
`ISSUE_CLOSED_AT_REARM_REASON` (`:182`). Le vol les **réutilise par la constante**
et n'en écrit aucun littéral neuf.

`StateUnreadable` est un report et non un terminal : c'est l'asymétrie D4 du
module (`ready_label.rs:69`), transposée mot pour mot. Un faux négatif coûte un
tick de latence ; un faux positif terminal tuerait une parente consentante sur une
panne d'API. Le prédicat d'état est déjà fail-closed champ par champ (`labels`
absent ⇒ `Err`, jamais `ReadyAbsent` — mika#2131, mika#2331) ; cette ligne est ce
qui rend ce fail-closed conséquent au site de promotion.

L'absence de token suit le site existant : `skip_deferred_dispatch_on_withdrawn_consent`
(`dispatcher.rs:1494`) rend déjà `NoToken` sans refuser, et le vol ne change pas ce
choix — sans token, ni le parc ni l'état ne sont lisibles.

## 3 — Le recensement, et pourquoi son nom est neuf

Le vol 2 a livré la ligne `rearm_consent_checked` avec un **écrivain unique**,
`reread_consent_before_rearm`, épinglé par
`mika3050_levenement_de_recensement_a_un_ecrivain_unique`. Le site de promotion
n'emprunte pas cette enveloppe : elle rend un `RearmOutcome`, là où la garde rend
un `bool`, et surtout un second émetteur rendrait le compte du vol 2 illisible.

Le site émet donc sa propre ligne, `promotion_consent_checked`, sur
`target: "mika::otel"`, avec les champs `repo` / `issue` **absents** quand ils ne
sont pas connus (jamais `""` ni `0` — mika#2331), et une ligne sur **toutes** les
branches, la saine comprise (mika#1833, mika#1926). L'assertion d'écrivain unique
du vol 2 reste verte sans modification : le nom est distinct.

Deux populations, deux instruments — la même raison qui interdit de partager le
motif de refus interdit de partager le nom de la sonde.

## 4 — Les tests

Le vol réutilise les deux seams existants, et n'en introduit aucun. Le mod
`mika2995` (`dispatcher.rs:11498`) appelle déjà
`skip_deferred_dispatch_on_withdrawn_consent_with` avec un `CountingFetcher`
injecté pour la timeline ; l'état se canne par le double `#[cfg(test)] run_gh` de
`gh_process.rs` (mika#3047), via `set_canned_stdout_for(GhSubject::Issue(n), "--json", payload)`.

**Le point dur, à traiter avant tout le reste.** Les fixtures du mod `mika2995`
portent `ISSUE: u64 = 3002` (`dispatcher.rs:11509`), **sous**
`FIXTURE_ISSUE_FLOOR = 990_000`. Aujourd'hui elles ne touchent jamais le seam `gh`
— seul le `TimelinePageFetcher` injecté est appelé. Dès que la composition ajoute
l'appel d'état, tout test atteignant `ConsentHeld` franchit le seam et l'assertion
de `gh_process.rs:289` **panique**. Aucune allowlist n'existe et aucune ne peut
être ajoutée (mika#2713) : le remède est la **renumérotation**. `ISSUE` passe à
`990_002`. C'est le résidu que le § 4 du plan du vol 2 avait nommé pour les deux
fixtures de `dispatcher.rs`, rencontré ici sur un troisième site.

Les sept tests existants du mod qui attendent `!refused` doivent donc canner un
payload d'état `Held` : trois lignes chacun, sans changement d'intention.

Tests neufs, tous dans le mod `mika2995` existant :

- **T1 — `ready` retiré par la machine : aucun tour, terminal.** Timeline
  `labeled HUMAN` → `unlabeled MACHINE`, état `{"state":"OPEN","labels":[]}`. La
  forme exacte de `0529ed9b`/#3044. Attendu : `refused`, parente `failed`, une
  ligne d'audit portant `ready_absent_at_rearm`. **C'est le test du défaut** : il
  est rouge sur `a2e347ef6`.
- **T2 — ticket fermé : aucun tour, terminal.** État
  `{"state":"CLOSED","labels":[{"name":"ready"}]}`, timeline non parquée. Attendu :
  `refused`, `issue_closed_at_rearm`.
- **T3 — relecture d'état en erreur : report, aucun terminal.** Payload illisible.
  Attendu : `refused`, parente **restée `pending`**, zéro ligne d'audit.
- **T4 — contrôle positif.** Timeline non parquée, état `Held`. Attendu :
  `!refused`, parente `pending`, wrapper `completed`, zéro audit.
- **T5 — la précédence du parc (D1).** Retrait **humain** : le ticket est parqué
  *et* sans `ready`. Attendu : la raison est `ready_consent_withdrawn`, **pas**
  `ready_absent_at_rearm`. C'est le test qui garde les deux populations
  soustractables.
- **T6 — la fenêtre `remove`→`add` (D3).** Timeline `labeled HUMAN` →
  `unlabeled MACHINE` → `labeled MACHINE`, état `Held` : la forme d'AC-T2 de
  mika#2995 au site neuf. Attendu : `!refused`. Épingle que la re-pose machine
  n'est pas un retrait.
- **T7 — une autre classe ne paie aucune lecture (AC3).** Parente
  `long_running:run_claude_pilot_groom`. Attendu : zéro appel au fetcher **et**
  zéro appel enregistré au seam `gh` (`recorded_calls()`). C'est la forme
  observable de « comportement inchangé ».
- **T8 — le recensement émet sur toutes les branches.** Une assertion de nom et
  d'unicité de site pour `promotion_consent_checked`, par les entrées de D3 (§ 5).

## 5 — Fire-Disposition

Le vol livre huit tests comportementaux. Par le précédent du vol 1 (§ 10 du plan
mika#3047, repris au § 9 du plan 002), **un test comportemental n'est pas un
détecteur** : son chemin de succès est « le comportement attendu a eu lieu », non
« aucune violation trouvée ». Pour cette part, la porte serait N/A.

Mais le vol livre aussi une extension d'un détecteur structurel existant, et la
porte s'applique à elle. Le détecteur D3 de `ready_label.rs`
(`mika2995_le_motif_et_levenement_ont_un_site_de_definition_unique`, `:2232`) exige
qu'un nom de la relecture de consentement ait **exactement un** site de définition,
et sa table `CONSENT_RECHECK_NAMES` (`:2163`) est la déclaration de la population.
Le vol ajoute des noms de cette famille, donc les y déclare — ne pas le faire
laisserait le détecteur se rendormir sur une population incomplète, le mode de
mensonge que son propre doc-comment nomme.

**Option retenue : (a), exception nommée en allowlist — et l'allowlist est
maintenue VIDE.**

- L'allowlist de D3 est `CONSENT_RECHECK_NAME_EXCEPTIONS` (`:2151`), née vide, et
  son assertion sœur d'auto-vacuité existe déjà
  (`mika2995_lallowlist_des_noms_de_relecture_est_vide`, `:2197`). Le vol
  n'y ajoute **aucune entrée** et ne modifie pas l'assertion : elle rougit à la
  première.
- Trois entrées sont ajoutées à `CONSENT_RECHECK_NAMES` :
  `("promotion_consent_checked", "task_engine/dispatcher.rs")`,
  `(READY_ABSENT_AT_REARM_REASON, "ready_label.rs")` et
  `(ISSUE_CLOSED_AT_REARM_REASON, "ready_label.rs")`. Les deux dernières ont déjà
  leur site unique à leur `const` du vol 1 ; le site de promotion les consomme par
  la constante, jamais en littéral, donc la déclaration est verte à la livraison.
- **Disposition si le détecteur fire** : router le second site par la constante,
  jamais l'allowlister, et surfacer le site au corps de PR sous
  `Fire-Disposition: second site d'un nom de relecture` — la disposition déjà
  inscrite dans le doc-comment de D3 (`:2228`), que le vol reprend sans la
  rouvrir.
- Le contrôle de bonne foi de D3 existe (`:2289`) et couvre les entrées neuves par
  construction : il vérifie que le scan rougit sur un second site.

Détecteur traversé sans être livré : l'assertion `FIXTURE_ISSUE_FLOOR` de
`gh_process.rs:289`. Elle **tire** sur ce vol, au mod `mika2995`. Son remède est la
renumérotation (§ 4) ; aucune allowlist n'existe et aucune ne peut être ajoutée
(mika#2713).

## 6 — Definition of Done

- La garde de promotion consulte les deux prédicats, dans l'ordre parc → état, et
  l'état n'est lu que sur un verdict de non-park.
- Un lanceur différé promu sur une parente `ready-label` dont le ticket a perdu
  `ready` — par un retrait **machine** compris — ne paie aucun tour mika-dev, et
  la parente quitte la file en `failed`.
- Un ticket fermé produit le même refus, sous `issue_closed_at_rearm`.
- Une relecture d'état en erreur reporte sans terminal.
- Les parentes d'une autre classe que `ready-label` ne paient aucune lecture, ni
  timeline ni état.
- Les deux populations de refus restent soustractables : un parc humain compte
  sous `ready_consent_withdrawn`, jamais sous une raison mika#3050.
- La ligne `promotion_consent_checked` est émise sur toutes les branches, par un
  écrivain unique, et l'écrivain unique de `rearm_consent_checked` du vol 2 est
  intact.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` et `bash scripts/verify-pipeline.sh origin/main`
  passent.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#3050, chacun annoté de son
état sur `a2e347ef6`.

- **AC1.** Avant de ré-armer une parente `ready-label`, sur les deux chemins du tableau, le moteur relit le ticket à la forge : ouvert, et portant `ready`.
  - Si l'un des deux manque, aucun lanceur n'est créé, et aucun tour mika-dev n'a lieu.
  - La parente quitte la file dans un état terminal qui libère le prochain `ready`, comme `failed` aujourd'hui.
  - Une ligne d'audit nomme la raison : `ready_absent_at_rearm` ou `issue_closed_at_rearm`.

  *État : livré sur les deux chemins de **création** par le vol 1 (`a645a6f9b`). La clause « aucun tour mika-dev n'a lieu » reste ouverte pour le lanceur **déjà en file** — c'est l'objet de ce vol.*

- **AC2.** Fail-closed : si la relecture échoue, pas de ré-armement à ce tick (`RearmOutcome::NotNow`), avec la cause nommée dans le log.
  - Un test par cas, dans le même appel : `ready` retiré, ticket fermé, relecture en erreur.
  - Le contrôle positif (`ready` encore posé, la forme des 4 premiers cas du tableau) ré-arme comme aujourd'hui.

  *État : livré sur le chemin de création. Ce vol en livre l'équivalent au site de promotion — T1 à T4 du § 4, le report prenant la forme d'un `bool` et non d'un `RearmOutcome`, parce que la garde de promotion ne crée rien.*

- **AC3.** Les parentes d'une autre classe que `ready-label` gardent leur comportement actuel. Un test le garde.

  *État : livré. Ce vol l'étend au seam d'état — T7 assère zéro appel `gh` en plus de zéro appel timeline.*

- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les ré-armements de parentes `ready-label` dont le ticket ne portait pas `ready` à l'instant du ré-armement (timeline du ticket), puis on l'écrit ici. La cible est 0. Un compte nul sans aucun retrait de `ready` pendant qu'une parente attend s'écrit « aucun croisement sur la fenêtre ».

  *État : l'instrument du chemin de création est livré par le vol 2 (`rearm_consent_checked`, huit branches). Ce vol livre l'instrument du chemin de promotion (`promotion_consent_checked`). La sonde elle-même est une fenêtre de 14 jours post-deploy : hors de portée temporelle d'un vol, et non livrable par du code. L'instrument n'est pas vivant avant `make deploy` ; l'absence de la ligne signifie « le binaire précède le vol », jamais « aucune occurrence » (mika#2340).*

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatcher.rs` — trois variantes de `WithdrawnConsentVerdict` + doc | 40 |
| `dispatcher.rs` — composition parc → état dans `…_with` | 45 |
| `dispatcher.rs` — `apply_withdrawn_consent_verdict` : deux terminaux, un report | 55 |
| `dispatcher.rs` — site de production + ligne `promotion_consent_checked` | 35 |
| `ready_label.rs` — trois entrées à `CONSENT_RECHECK_NAMES` + doc | 15 |
| tests — T1 à T8 (mod `mika2995` existant) | 185 |
| tests — renumérotation `ISSUE` → `990_002` + payload d'état sur les 7 tests existants | 25 |

Total estimé : 400 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 400 est sous la borne de 429, et sous le seuil par
PR de 1000. Trois raisons rendent l'estimation tenable plutôt qu'optimiste : le vol
n'introduit **aucun seam** — les deux existent et sont exercés par des tests qui
tournent déjà ; il n'écrit **aucune constante de raison neuve** — les deux que
nomme AC1 sont livrées par le vol 1 ; et il ne livre **aucun scan neuf** — il
déclare trois noms dans la table d'un détecteur existant dont les assertions et le
contrôle de bonne foi sont en place.

La part la plus incertaine est la renumérotation (25 lignes) : elle touche sept
tests existants, mais par une constante et un payload canné, sans changer leur
intention. Si elle débordait, le livrable à scinder serait les tests de bord T5,
T6 et T8 — T1 à T4 et T7 portent AC1, AC2 et AC3 à eux seuls. Aucune ligne
`Tracked in:` n'est requise : le vol courant passe sous la borne, et le gel des
tickets jusqu'au 01/12 renvoie tout résidu en commentaire sur #3050 ou dans la PR.

## Ordre de commit

1. **La renumérotation d'abord**, seule et verte : `ISSUE` → `990_002` dans le mod
   `mika2995`. Sans elle, tout commit suivant panique au seam `gh` plutôt que
   d'échouer sur son assertion, et un panic masque la lecture du défaut.
2. **Les trois variantes de verdict** et la ligne de recensement, plus les trois
   entrées de `CONSENT_RECHECK_NAMES`. Compile, et D3 reste vert.
3. **T1 rouge**, seul : le test du défaut sur la forme `0529ed9b`/#3044, avant la
   composition. C'est la preuve que le vol ferme quelque chose.
4. **La composition parc → état** dans `…_with` et `apply_withdrawn_consent_verdict`,
   plus le payload d'état des sept tests existants. T1 passe au vert.
5. **T2 à T8**, et la suite complète : `cargo test -p mika-agent`, `cargo clippy`,
   `cargo fmt --check`, `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`.

Chaque étape est verte à son terme, sauf l'étape 3 dont le rouge est l'objet.

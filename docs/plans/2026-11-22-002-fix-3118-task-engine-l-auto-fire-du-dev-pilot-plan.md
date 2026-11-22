# mika#3118 — l'auto-fire mika#1614 exige le consentement de l'opérateur avant de lancer le pilote

**Vol 1 sur 2** — ce vol ferme la moitié *consentement* (commentaire 39). La moitié
*halte du plan* (commentaires 58a / 59a / 54) part en vol 2, nommée en § Périmètre.

## Le défaut, tel qu'il se mesure

`try_dispatch_pilot_after_groom_success` (`crates/mika-agent/src/task_engine/dispatcher.rs:5521`)
lance `dev-pilot` dès qu'un callback de groom revient convergé. Ses préconditions
(l. 5528-5650) sont au nombre de cinq : la classe de dispatch est `groom`, le
marqueur `Outcome: PLAN_GROOMED` est en tête de ligne, la tâche parente porte une
`reference_url` de forme `issues/<n>`, le STOP de l'auto-pull n'est pas posé
(mika#2498), et un token de forge existe. **Aucune ne lit le ticket.** Le plus
récent état du ticket à la forge — ouvert ou clos, portant `ready` ou non — n'est
pas consulté sur ce chemin.

Trois dispatches mesurés, tous sortis de cette absence :

| date | groom | ce qui a été lu | ce qui est parti | coût |
|---|---|---|---|---|
| 2026-11-09 | `a484395a` (via `mika ask --agent mika-dev`, consigne « ne pas lancer dev-pilot »), groom pilote `d86311a1` → `PLAN_GROOMED` à 19:39:16Z | `dispatch_class`, marqueur, `reference_url` | `dev-pilot` **`a5aaafd5` à 20:00:51Z** sur mika#1727, qui ne portait pas `ready` | un pilote d'implémentation non consenti |
| 2026-11-19 | groom `93828860`, HALTE en tête du plan (312 lignes estimées contre une borne de 300), sorti en `status: success` | idem | `dev-pilot` **`137111d7` à 15:47:31Z** (« engine: auto-fired dev-pilot dispatch after groom success (mika#1614) ») | 1,34 $, zéro commit — le pilote a honoré la halte |
| 2026-11-17 | révision ayant levé une « HALTE BORNE » en amendant le plan, second passage accepté | idem | `dev-pilot` **`be5cca2e` à 12:36:36Z** sur un plan jugé **jamais committé** — `origin` portait encore le plan en halte `59c94bb1` | pilote tué à la main vers 13:04Z, mika#3104 ouvert en wip-rescue puis clos sans merge |

Les deux derniers incidents sont la moitié *halte* ; le premier est la moitié
*consentement*, et c'est celle de ce vol.

**La classe du premier incident est ce qui le rend structurel.** La consigne
« ne pas lancer dev-pilot » était en langage naturel, dans le `mika ask` qui a
commandé le groom. Elle n'a arrêté rien : *seule une porte mécanique le peut*
(même classe que la règle Q1 et que mika#3058). Et la porte qui existe déjà pour
cette question — le consentement positif `ready` de mika#841, dont la porte
mika#2772 lit la présence — vit sur le chemin webhook, pas sur ce chemin-ci.
L'auto-fire mika#1614 a été ouvert précisément pour *court-circuiter* le
`gh issue edit --add-label ready` → webhook → tour de LLM (vulnérable à la
fabrication, mesurée sur mika#1609) ; en retirant le détour il a retiré la
**lecture** du consentement avec lui.

### Pourquoi exiger `ready` ne ferme aucun chemin nominal

C'est la question de justesse du vol, et elle se mesure dans le dépôt plutôt
qu'elle ne se raisonne :

- **Chemin webhook `ready`-label.** Le label est là par construction : c'est son
  événement qui a ouvert le dispatch.
- **Chemin cascade milestone (M4).** Le bassin pose `ready` lui-même, en un seul
  geste groupé avec le rattachement au milestone —
  `crates/mika-agent/src/milestone_manager/pool_writer.rs` (mika#2699 vol 4),
  dont le test de forme d'argv pin `--add-label ready` et refuse « jamais un
  retrait de `ready` ». Un enfant de milestone arrivé au groom porte donc le
  label avant que le groom ne commence.
- **Chemin opérateur (`/mika-groom-ticket`, `mika ask`).** C'est le seul chemin
  qui peut atteindre l'auto-fire sans `ready` — et c'est exactement le chemin de
  l'incident du 09/11, où l'enchaînement était **non voulu**.

La garde ne retire donc l'auto-fire qu'au cas pour lequel elle est écrite. Elle
ne retire pas le grooming : un groom sans `ready` converge, écrit son callout et
reste groomé ; seul le **pilote d'implémentation** attend le geste humain.

## Requirements

- **R1 — Le consentement est relu à la forge au moment du dispatch, pas déduit.**
  Avant de lancer le pilote, `try_dispatch_pilot_after_groom_success` relit l'état
  courant du ticket parent (ouvert ? portant `ready` ?) et ne part que si les deux
  tiennent. Aucune valeur en base ne tient lieu de réponse : la fenêtre entre la
  mise en file du groom et la fin de sa convergence est de l'ordre de la dizaine
  de minutes (19:39:16Z → 20:00:51Z sur l'incident), et c'est dans cette fenêtre
  qu'un opérateur retire un label.

- **R2 — Le lecteur est celui qui existe, pas un nouveau.**
  `crate::ready_label::reread_issue_consent_via_gh(token, owner_repo, number) -> Result<RearmConsent, String>`
  (`ready_label.rs:706`) répond déjà à cette question exacte — « le consentement
  tient-il à cet instant ? » — en lisant l'**état** et non la timeline, avec un
  contrat fail-closed champ par champ (`state` hors du couple `OPEN`/`CLOSED`,
  `labels` absent ou non-tableau ⇒ `Err`, et surtout `labels` absent n'est
  **jamais** `ReadyAbsent`). Son `RearmConsent` a trois variantes : `Held`,
  `ReadyAbsent`, `Closed`. Aucun second lecteur n'est écrit, et le prédicat de
  présence reste `webhook_dispatch::labels_carry_ready` à travers lui — l'unique
  lecteur du consentement positif de mika#2772.

- **R3 — Le refus se pose APRÈS le token et AVANT le flip de `dispatch_class`.**
  Trois contraintes d'ordre, chacune mesurable :
  1. **Après le frein STOP de mika#2498** (step 3bis) : le STOP est un geste
     global d'opérateur, et sa ligne `groom_pilot_autofire_stopped` doit continuer
     de compter les dispatches *qu'il* a refusés — un refus de consentement qui
     mordrait avant viderait sa population sans le dire.
  2. **Après le step 4** (`github_token.is_none_or(str::is_empty)`) : la relecture
     exige le token, et un `no_token` déjà nommé reste nommé sous son nom.
  3. **Impérativement avant le step 5c** (`update_task_dispatch_class(parent, "implement")`) :
     un refus posé après laisserait une parente étiquetée `implement` sans
     implement en vol, et le slot serait mal compté (#1001) — c'est la doctrine
     que le commentaire du frein mika#2498 énonce déjà à son propre sujet.

- **R4 — Fail-closed, et chaque absence sous son nom.** Quatre issues de la
  garde, quatre dispositions distinctes :
  | état lu | disposition | raison d'audit |
  |---|---|---|
  | `Held` (ouvert + `ready`) | le pilote part | aucune (chemin sain) |
  | `ReadyAbsent` | refus | `ready_absent_at_autofire` |
  | `Closed` | refus | `issue_closed_at_autofire` |
  | `Err(_)` (relecture illisible ou en échec) | **refus** | `consent_unreadable_at_autofire` |
  Les fusionner serait mika#2205 : « `ready` n'est pas posé » et « je n'ai pas pu
  lire si `ready` est posé » sont deux populations, et une population commune
  n'est plus soustractible. Le fail-closed sur `Err` est la disposition
  obligatoire et non un choix : la doctrine mika#2981 interdit qu'un vol puisse
  partir *parce que* le réseau manque, et l'inverse — partir sur `Err` — ferait
  exactement cela.

- **R5 — Le recensement a un dénominateur.** Une ligne de sonde
  `groom_autofire_consent_checked` est émise à **chaque** passage de la garde,
  branche saine comprise, portant `verdict`, `repo` et `issue`. Motif
  `kg_budget_resolved` (mika#1833) / `team_stop_topic_inheritance` (mika#1926),
  et rappel explicite du frère mika#3050 : *un recensement qui ne compte que les
  anomalies n'a pas de dénominateur.* Les coordonnées sont `Option` et non `""`
  là où elles manquent (mika#2331).

- **R6 — Le vocabulaire du verdict est clos, et le compilateur en est le
  gardien.** Le `match` qui tranche est exhaustif, sans bras `_ =>`, de sorte
  qu'une quatrième variante de `RearmConsent` doive être décidée à la
  compilation plutôt que tomber dans un défaut muet. Un scan structurel dans
  `canonical_tokens.rs` refuse qu'un bras `_` y revienne — précédent exact :
  la garde du `match` de `GroomVerdictState` (`canonical_tokens.rs:4716-4759`).

- **R7 — Le site d'engagement est unique, et sa cardinalité est assertée.** La
  garde est appelée depuis **un** site, dans `try_dispatch_pilot_after_groom_success`.
  Un scan de source pin ce nombre dans les deux sens, sur le motif de
  `lignes_dappel_en_production` que `engine.rs:11838-11907` applique déjà à
  `try_dispatch_pilot_after_groom_success` lui-même.

- **R8 — Le refus est audité comme le dispatch.** Chaque refus écrit un
  `audit_events` sous `GROOM_PILOT_DISPATCHER_TOOL`
  (`task_engine_groom_pilot_dispatcher`) avec sa propre `after_value`, à côté des
  deux existantes (`implement_dispatched`, `stopped_by_sentinel`). C'est la table
  sur laquelle l'opérateur fait ses `GROUP BY`, et le vol ajoute trois valeurs
  nommées plutôt qu'un silence.

## Design

### D1 — Un step 4bis, frère du frein de mika#2498

La garde s'insère dans la chaîne existante sans la réorganiser :

```
1    dispatch_class == "groom"                              (inchangé)
2    Outcome: PLAN_GROOMED, lu ancré                        (inchangé)
3    parent + parse_repo_issue_from_url → (repo, issue)     (inchangé)
3bis STOP auto-pull posé ? → refus (mika#2498)              (inchangé)
4    token présent ?       → refus                          (inchangé)
4bis consentement relu à la forge → refus (CE VOL)          ← nouveau
5a…5j résolution du skill, flip dispatch_class, spawn       (inchangé)
```

Le code du step 4bis, dans `dispatcher.rs` :

```rust
let census = match crate::ready_label::reread_issue_consent_via_gh(token, &repo, issue_num).await {
    Ok(crate::ready_label::RearmConsent::Held) => AutofireConsent::Held,
    Ok(crate::ready_label::RearmConsent::ReadyAbsent) => AutofireConsent::ReadyAbsent,
    Ok(crate::ready_label::RearmConsent::Closed) => AutofireConsent::IssueClosed,
    Err(reason) => AutofireConsent::Unreadable(reason),
};
```

puis un second `match` sur `census`, exhaustif, qui pose la sonde R5 pour les
quatre branches, et pour les trois branches de refus écrit l'audit et retourne.
Pas de `_ =>` dans l'un ni dans l'autre (R6).

### D2 — Pourquoi un enum local et non un `bool`

`AutofireConsent` vit dans `dispatcher.rs` et porte quatre variantes
(`Held`, `ReadyAbsent`, `IssueClosed`, `Unreadable(String)`). Un `bool` aurait
suffi à *décider*, et c'est précisément pourquoi il ne suffit pas : la décision
est la même sur trois branches (refuser) alors que le **fait** y est différent,
et c'est le fait que l'audit et la sonde doivent porter. Le motif est celui de
`RearmConsentCensus` (mika#3050 D1) et de `GroomVerdictState` : un vocabulaire
clos, un verdict par branche, aucune population inférable par soustraction.

La variante `Unreadable` porte son motif plutôt que de le laisser au log : c'est
ce motif que l'opérateur lira quand un refus sera un faux refus.

### D3 — Ce que la garde ne fait pas

- **Elle ne pose aucun label.** Le doc-comment de la fonction
  (`dispatcher.rs:5477`) interdit déjà d'y réinstaurer une écriture de label — le
  moteur qui se poserait à lui-même le consentement de l'opérateur serait
  mika#841 / mika#2772 inversés, et un scan de `canonical_tokens.rs` refuse cette
  écriture dans tout prompt bundled. Le vol lit, et seulement lit.
- **Elle ne touche pas au grooming.** `has_completed_groom_for_issue` et
  `groom_result_convergence` sont inchangés : le groom reste convergé, le callout
  reste écrit, la preuve `Outcome: PLAN_GROOMED — <plan>` reste intacte. Le refus
  porte sur le lancement du pilote, pas sur la validité du plan.
- **Elle n'avertit pas l'opérateur hors audit.** Un commentaire sur le ticket
  serait un second canal à maintenir ; la ligne de sonde et l'audit sont les deux
  canaux que l'opérateur lit déjà pour ce chemin.
- **Elle ne lit pas le contenu du plan.** C'est le vol 2.

### D4 — Fire-and-forget, et ce que ça implique pour la forme du refus

Depuis mika#2995, rien en aval n'attrape l'échec de cette fonction : sur toute
précondition non tenue, elle journalise et retourne. Un refus n'a donc **aucun**
chemin de remontée autre que ses propres traces — d'où l'exigence R5 (la sonde à
dénominateur) et R8 (l'audit nommé). Un refus muet serait indistinguable d'un
groom qui n'a jamais convergé, et c'est la classe mika#2340 : une ligne de log
absente dit « binaire d'avant le changement », jamais « le fait est absent ».

## Périmètre

Dans ce vol : le step 4bis et son vocabulaire de verdict, ses trois valeurs
d'audit, sa sonde, ses tests comportementaux, et les deux scans structurels
(exhaustivité du `match`, cardinalité du site d'engagement).

**Renvoyé au vol 2 — la halte du plan ne remonte pas jusqu'au moteur.** Les
incidents des 17/11 et 19/11 sont d'une autre nature : le plan portait une HALTE
en tête, le groom est sorti en `status: success`, et *rien ne porte la halte du
groom jusqu'au moteur*. Le remède a une forme déjà identifiée mais un volume
propre, et il touche un autre fichier :

- le producteur stampe son fait — au site où `dispatch-lib.sh` compose
  `Outcome: PLAN_GROOMED` (l. 14295-14298), `GROOM_CALLOUT_PLAN` nomme le plan
  validé, qui est **sur le disque du worktree** : la halte se lit localement,
  sans réseau ;
- le format de fil existe déjà : `GROOM_HALT_CAUSE_LINE_PREFIX`
  (`Groom-halt-cause:`, mika#2641, l. 10918), ancré en début de ligne, lu ancré
  côté Rust, avec son scan de synchronisation shell ↔ Rust
  (`canonical_tokens::tests::mika2641_*`). Le vol 2 y ajoute une quatrième
  valeur nommée et son lecteur, de sorte qu'un plan en halte sorte « dans un état
  que mika#1614 ne prend pas pour un feu vert » ;
- la grammaire de halte reste à poser : `grep -rn "HALTE" skills/bundled/`
  ne rend **rien**, donc aucun prompt ne la prescrit aujourd'hui et les deux
  occurrences mesurées (`HALTE`, `HALTE BORNE`) sont de la prose de LLM. Une
  grammaire posée sur l'observé est un contrat *plus* un détecteur, avec sa
  propre Fire-Disposition.

**Tracked in:** senara-solutions/mika#3118 (vol 2 — ce vol ne clôt pas le ticket)

**Hors des deux vols, déjà relevé ailleurs :** le troisième écart du commentaire
54 — le plan **jugé** par le second passage n'est pas celui que `origin` porte, et
*aucune étape ne compare le plan jugé et lancé au plan de la branche sur origin*.
C'est un défaut de la boucle de révision (`_launch_revise_pilot` / le commit du
plan révisé), pas de l'auto-fire : l'auto-fire a lu une preuve exacte d'un plan
réellement validé ; c'est le plan qui avait bougé sous elle. À garder dans le
parking #2983.

**Hors périmètre, cité pour la frontière :** le frein de mika#2498, la porte de
consentement de mika#2772 (`executor.rs:3331-3850`) et l'auto-pull de mika#3117
ne changent pas. Ce vol ne fait qu'ajouter à ce chemin la lecture qu'ils font
déjà sur les leurs.

## Verification Contract

- **V1 — `Held` laisse partir le pilote.** Un groom convergé sur un ticket ouvert
  portant `ready` produit le dispatch, l'audit `implement_dispatched` et le flip
  `dispatch_class = implement`, exactement comme aujourd'hui. Non-régression des
  douze sites de test existants de `try_dispatch_pilot_after_groom_success`
  (`dispatcher.rs:10132` … `:10794`), qui doivent rester verts avec un double de
  consentement tenant.
- **V2 — `ReadyAbsent` refuse.** Aucun `spawn_long_running_exec`, un audit
  `ready_absent_at_autofire`, et — l'assertion qui porte R3.3 — la parente est
  **restée** en `dispatch_class = groom`.
- **V3 — `Closed` refuse, sous son propre nom.** Même forme que V2, audit
  `issue_closed_at_autofire`, et l'assertion que la valeur n'est pas celle de V2.
- **V4 — `Err` refuse.** Relecture en échec ⇒ aucun dispatch, audit
  `consent_unreadable_at_autofire`, et le motif de l'erreur présent dans la trace.
  Le test est écrit sur le chemin « la réponse de la forge est hors contrat »
  (`labels` absent), que `parse_rearm_consent` rend déjà en `Err` — ce qui prouve
  au passage que `labels` absent n'est pas compté comme `ready` absent.
- **V5 — Les quatre branches sont comptées.** La sonde
  `groom_autofire_consent_checked` est émise sur les quatre, `Held` compris, avec
  un `verdict` distinct par branche. Un test énumère les quatre valeurs et
  asserte qu'elles sont deux à deux distinctes.
- **V6 — L'ordre est pin.** Un scan structurel asserte que l'unique lecture de
  `reread_issue_consent_via_gh` dans `dispatcher.rs` est posée **après** la garde
  de token et **avant** l'appel à `update_task_dispatch_class`. Motif exact des
  gardes d'ordre de `auto_pull.rs:9158-9365` (« l'unique lecture de `{name}` est
  posée APRÈS `gh_reread_issue_at_dispatch` »).
- **V7 — Le `match` reste exhaustif.** Scan structurel refusant un bras `_ =>`
  dans le `match` du step 4bis et nommant les quatre variantes, sur le motif de
  `canonical_tokens.rs:4716-4759`.
- **V8 — Le site d'engagement est unique.** Scan de cardinalité : exactement un
  appel en production, auto-nettoyant dans les deux sens (il rougit si un second
  site apparaît *et* si le seul disparaît).
- **V9 — Pipeline.** `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`,
  `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- Le step 4bis est en place dans `try_dispatch_pilot_after_groom_success`, entre
  la garde de token et la résolution du skill.
- `AutofireConsent` est défini, exhaustivement matché, sans bras `_`.
- Trois constantes d'audit nommées sont ajoutées à côté de
  `GROOM_PILOT_DISPATCHED_VALUE` et `GROOM_PILOT_STOPPED_VALUE`, et chacune est
  écrite par sa branche.
- La sonde `groom_autofire_consent_checked` est émise sur les quatre branches.
- V1 à V9 passent. Les douze tests existants de l'auto-fire restent verts.
- `docs/operator/environment-variables.md` n'est pas touché : ce vol n'ajoute
  aucune variable, aucun STOP et aucun interrupteur (le STOP qui gouverne déjà ce
  chemin est celui de mika#2498, inchangé).
- Aucun `gh issue edit` n'apparaît sur ce chemin.

## Acceptance criteria

Le corps de mika#3118 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés des Requirements et du Verification Contract.

1. **AC1 — Un groom convergé sur un ticket sans `ready` ne lance pas le pilote.**
   Rejouer la forme de l'incident du 09/11 : groom de classe `groom`, `result`
   portant `Outcome: PLAN_GROOMED` en tête de ligne, parente dont la
   `reference_url` désigne un ticket **ouvert et sans `ready`** ⇒ aucun
   `dev-pilot` n'est créé, et la parente reste en `dispatch_class = groom`.
2. **AC2 — Un groom convergé sur un ticket ouvert portant `ready` lance le
   pilote, inchangé.** Le dispatch, l'audit `implement_dispatched`, le flip de
   classe et le `timeout_secs` sont identiques à l'octet près au comportement
   d'avant le vol.
3. **AC3 — Un ticket clos est refusé sous un nom distinct de l'absence de
   `ready`.** Les deux refus portent deux `after_value` différentes dans
   `audit_events`, et un `GROUP BY after_value` les sépare.
4. **AC4 — Une relecture de forge en échec refuse le dispatch** et porte son
   propre nom, distinct des deux précédents. En particulier, une réponse dont le
   champ `labels` est absent est comptée comme illisible, jamais comme un `ready`
   absent.
5. **AC5 — Les quatre issues de la garde sont comptables sans soustraction.** La
   ligne `groom_autofire_consent_checked` est émise sur les quatre branches,
   `Held` incluse, de sorte que le taux de refus ait un dénominateur.
6. **AC6 — Le refus est posé avant tout effet de bord.** Aucune branche de refus
   n'a flippé `dispatch_class`, n'a créé de tâche de callback, ni n'a spawné
   d'exec : asserté par test, et l'ordre est pin par un scan structurel.
7. **AC7 — Le vocabulaire du verdict ne peut pas s'élargir en silence.** Ajouter
   une variante à `RearmConsent` casse la compilation du step 4bis ; un bras
   `_ =>` réintroduit fait rougir un scan structurel.
8. **AC8 — La cascade milestone et le chemin webhook ne sont pas fermés.** Le
   plan démontre, par le site `pool_writer.rs` qui pose `ready` en même temps que
   le rattachement, qu'un enfant de milestone porte le label avant le groom ; un
   test de non-régression couvre la forme « ticket ouvert + `ready` » qui est
   celle des deux chemins nominaux.

## Fire-Disposition

Ce vol livre quatre détecteurs : V6 (ordre), V7 (exhaustivité du `match`), V8
(cardinalité du site d'engagement), et la garde de non-régression de V1. Option
retenue : **(a) exception nommée en allowlist, avec une population d'exception
vide et mesurée.**

- **Population mesurée aujourd'hui : zéro.** Les trois scans portent sur du code
  que ce vol écrit lui-même — il n'existe aucun site préexistant à excepter. La
  mesure est faite plutôt qu'affirmée : `grep -c` sur
  `reread_issue_consent_via_gh` dans `dispatcher.rs` rend **0** avant le vol (le
  lecteur n'y est pas appelé), et sur `update_task_dispatch_class` dans
  `try_dispatch_pilot_after_groom_success` rend **1**.
- **L'allowlist est donc née vide, et elle est tenue par une assertion de
  cardinalité auto-nettoyante** plutôt que par une liste : `EXPECTED_AUTOFIRE_CONSENT_SITES = 1`
  pour V8, et pour V6/V7 une assertion de forme qui nomme les quatre variantes
  attendues. Chacune rougit dans les **deux** sens — quand un site apparaît et
  quand le site attendu disparaît — ce qui est ce qui empêche l'exception de
  devenir stale sans que rien ne le dise. Précédent direct :
  `EXPECTED_ENGAGEMENT_SITES = 3` du plan frère mika#3117
  (`docs/plans/2026-11-22-001-fix-3117-…-plan.md`).
- **Pas de ticket de suivi d'exception** : il n'y a pas d'exception à suivre. Si
  l'implémentation découvre un site préexistant que l'un des scans refuse, elle
  lui crée une entrée grep-visible nommant le site, référençant mika#3118, et
  l'assertion de cardinalité passe de `= 1` à `= 1 + <n exceptions>` — ce qui
  rend l'entrée comptable et sa disparition visible.
- **Rien n'est livré désarmé** (option b) : les quatre détecteurs partent armés
  dans la même PR, puisqu'ils sont verts par construction sur le code du vol.
  Aucune halte n'est requise (option c) : le périmètre est tranché et sous la
  borne.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — step 4bis, `AutofireConsent`, 3 constantes d'audit, sonde, doc-comments | 150 |
| tests `dispatcher.rs` — `mod mika3118_consentement_a_lautofire` (V1-V5, AC1-AC6, AC8) | 170 |
| `crates/mika-agent/src/canonical_tokens.rs` — scans structurels V6, V7, V8 | 55 |

Total estimé : 375 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 375 < 429, et la marge de 54 lignes est réelle
plutôt que comptable, pour trois raisons mesurables :

1. **Aucun nouveau lecteur réseau n'est écrit.** `reread_issue_consent_via_gh` et
   son `parse_rearm_consent` existent, sont testés champ par champ, et le
   prédicat de présence (`labels_carry_ready`) existe aussi avec ses propres
   tests. Le vol en est un *appelant*, et la ligne d'appel est de trois lignes.
2. **Un seul fichier de production est modifié, en un seul endroit.** Le step
   4bis s'insère entre deux steps existants sans réorganiser la chaîne : aucun
   step n'est déplacé, aucune signature n'est changée, et les douze sites de test
   existants restent valides.
3. **Les tests réutilisent le harnais en place.** Les douze appels de test de
   `try_dispatch_pilot_after_groom_success` posent déjà la DB, le `SkillRegistry`
   et la forme du `result` (`:10843`) ; les nouveaux cas ajoutent une branche de
   double de consentement, pas un harnais.

La moitié *halte du plan* est ce qui aurait fait sauter la borne : elle touche
`dispatch-lib.sh` (détecteur de halte + quatrième valeur de
`GROOM_HALT_CAUSE_*`), `test-dispatch-lib.sh`, le lecteur Rust ancré de la ligne,
le scan de synchronisation shell ↔ Rust de mika#2641, et le prompt de `dev-groom`
pour la grammaire — de l'ordre de 400 lignes à elle seule, soit ~775 au total.
D'où le découpage en vols, et non en phases d'un même vol : le vol 1 est
mergeable seul et ferme un incident entier.

## Ordre de commit

1. **`fix(3118): le vocabulaire du consentement à l'auto-fire`** —
   `AutofireConsent` et les trois constantes d'audit, non encore appelés. Un
   commit qui ne change aucun comportement et qui rend le vocabulaire
   relisable seul.
2. **`fix(3118): l'auto-fire relit le consentement avant de lancer le pilote`** —
   le step 4bis, sa sonde et ses trois branches de refus. C'est le commit qui
   change le comportement, et il est seul à le faire.
3. **`test(3118): les quatre issues de la garde de consentement`** — V1 à V5,
   AC1 à AC6 et AC8.
4. **`test(3118): l'ordre, l'exhaustivité et la cardinalité du site`** — les
   scans structurels V6, V7, V8 dans `canonical_tokens.rs`.

**Plancher du vol.** Les commits 1 et 2 seuls laissent un comportement correct
mais non prouvé ; les commits 3 et 4 sont ce qui le rend durable. Un vol qui
s'arrêterait après le 2 serait à reprendre, pas à compléter : la PR n'est
mergeable qu'avec les quatre.

# mika#2161 — `auto_feeder_no_backlog` affirme une cause que le code ne connaît pas

## Invariant (une phrase)

Quand l'alimenteur ne trouve aucun candidat, il **nomme la cause qu'il a mesurée**
— absence de backlog groomé, bassin `ready` coincé en vol, ou signal de vol
illisible — et ne l'affirme jamais par défaut.

## Ce que la lecture du code déplace dans le ticket (premier livrable)

Quatre faits corrigent ou complètent l'énoncé, et chacun change le plan.

**R1 — La suggestion du commentaire 1/2 est caduque, et dans le bon sens.**
mika#2131 + mika#2132 sont **livrés** : `ExclusionLedger`, les seize constantes
`FILTER_*`, la table de ledger `auto_pull_exclusion` et le test de format de fil
`mika2131_filter_names_are_a_wire_format` sont tous à HEAD (`auto_pull.rs:1202-1445`).
Le plan de mika#2131 avait d'ailleurs **écarté #2161 par écrit** (« invariant
DIFFÉRENT, ticket séparé »). La moitié muette est donc fermée ; ce ticket ferme
la moitié bavarde, et l'état « à moitié fiable » que le commentaire craignait est
derrière nous, pas devant.

**R2 — Et cette livraison donne le vocabulaire à réutiliser, ce qui n'est pas une
coïncidence.** `FILTER_OPEN_PR` (`open_pr_closing`), `FILTER_IN_FLIGHT`
(`in_flight_self_dev`) et `FILTER_OPERATOR_HELD` (`operator_review_or_blocked`)
sont déjà des valeurs de fil épinglées, employées par `groomed_candidate_exclusion`
sur le bassin **`!ready`**. Le bassin **`ready`**, lui, n'enregistre rien : c'est
exactement le trou de AC2. Le recensement livré ici étend le même vocabulaire au
bassin `ready`, donc un opérateur qui fait `GROUP BY after_value` sur
`auto_pull_exclusion` et un opérateur qui lit les compteurs de `auto_feeder`
parlent la même langue. Deux orthographes du même filtre couperaient une
population en deux sans le dire — la leçon que mika#2131 a dû graver une fois.

**R3 — Les numéros de ligne du ticket ont dérivé**, et le dire évite une chasse :
`count_pullable_ready` est à **1904-1916** (et non 1050-1062), le bloc fautif à
**3185-3206** (et non 1995-1998).

**R4 — AC3 n'est répondable avec un nombre vrai que depuis mika#2335.** « Les
tickets en vol et depuis combien de temps » suppose un instant de départ. Or
`fired_at` n'était **jamais estampillé sur la row parent** avant mika#2335 : les
trois chemins de dispatch passaient par `update_manual_task_status`, qui écrit
`status`/`updated_at`/`completed_at` et rien d'autre, et un opérateur lisant la
row parent d'un dispatch vif y lisait « jamais tiré » (c'est l'incident du 15/09).
`mark_parent_dispatched` est maintenant le lecteur unique de cette transition.
Conséquence directe sur la conception : l'âge se lit
`COALESCE(fired_at, created_at)`, et l'événement porte le **statut** pour que le
lecteur sache laquelle des deux horloges a démarré — une row `pending` non encore
dispatchée donne un âge depuis sa *création*, ce qui est un autre fait.

## L'arbitrage central : (b) l'emporte sur (a) quand les deux sont vrais

Le ticket présente (a) et (b) comme deux situations. **Elles ne sont pas
exclusives**, et la nuit fondatrice est précisément le cas où les deux tenaient :
six `ready` dont quatre en vol **et** aucun candidat groomé-non-`ready`
dispatchable devant. Un classifieur « ou bien / ou bien » aurait donc pu rendre
(a) cette nuit-là, c'est-à-dire reproduire le défaut.

La règle est donc une **priorité, pas une alternative** :

> Si le bassin `ready` porte des exclus `in_flight`, le remède est de débloquer,
> **quoi que dise le backlog** — parce que promouvoir dans un bassin dont les
> consommateurs sont coincés ne produit rien.

C'est littéralement l'erreur mesurée : quatre sessions de grooming lancées à
23:28 pendant que le créneau de dispatch était pris. Groomer davantage n'aurait
pas pu aider, même si le backlog avait été vide **et** qu'il l'était.

## Trois causes, et pourquoi la troisième existe

| cause | prédicat | remède nommé | régime attendu |
|---|---|---|---|
| **(b)** `PoolInFlight` | `in_flight > 0` | débloquer — les tickets et leurs âges sont dans l'événement | non nul pendant un blocage |
| **(c)** `InFlightUnreadable` | sinon, `probe_failed > 0` | réparer la sonde — ni (a) ni (b) n'est établi | **zéro** |
| **(a)** `NoGroomedBacklog` | sinon | groomer davantage | nominal |

**(c) n'est pas de la sur-ingénierie, c'est le fail-safe existant rendu
lisible.** La boucle de sonde traite déjà une erreur DB comme « en vol »
(`auto_pull.rs:3142-3145`, `treating as in-flight`) — fail-safe correct pour le
*seuil*, puisqu'il vaut mieux ne pas promouvoir dans le doute. Mais versé tel
quel dans le message de (b), il ferait **nommer comme en vol des tickets qui ne
le sont peut-être pas**, c'est-à-dire donner un remède faux avec l'autorité d'une
mesure. La maison a déjà tranché cette famille trois fois : `pilot_stall_signal_unavailable`
(mika#2277, « un signal qu'on ne peut pas lire n'est jamais un terme satisfait »),
`unknown_provider` (mika#2328, « répondre `default` y affirmerait ce qui est
inconnu et possiblement faux »), et la paire `below_threshold` /
`no_ready_label_event` (mika#2131). Le seau `probe_failed` est donc **compté à
part pour l'attribution** tout en restant fondu dans l'ensemble que
`count_pullable_ready` reçoit — le seuil ne bouge pas d'un iota.

## Le recensement, et l'invariant qui le tient (AC2 + AC6)

`census_ready_pool(issues, open_pr, in_flight, probe_failed) -> ReadyPoolCensus`,
fonction **pure**, attribuant chaque ticket `ready` à **exactement un** seau, dans
l'ordre même où `count_pullable_ready` applique ses `.filter()` :

```
raw_ready = pullable + open_pr + in_flight + probe_failed + operator_held
```

**L'additivité est le livrable, pas une élégance.** AC2 demande de « trancher sans
aller lire ailleurs » : des seaux qui se recouvriraient donneraient une somme
supérieure au brut, et l'opérateur qui les additionne n'aurait plus rien à quoi se
raccrocher. D'où l'attribution au **premier** filtre qui mord, et dans **l'ordre de
`count_pullable_ready`** — attribuer dans un autre ordre produirait des comptes
individuellement vrais qui n'expliqueraient pas le pipeline que l'opérateur est en
train de lire.

**AC6 devient structurel plutôt que déclaratif.** L'invariant épinglé est :

```rust
census.pullable == count_pullable_ready(issues, open_pr, in_flight)
```

sur toute forme d'entrée. Un recensement qui diverge du compte qu'il prétend
expliquer est pire que pas de recensement ; et ce test est **aussi** la garde
AC6 — si quelqu'un touche `count_pullable_ready`, le recensement doit suivre ou le
test rougit. Modèle : `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`.
`count_pullable_ready` n'est ni modifié, ni déplacé, ni enveloppé.

## Trois noms d'événement, et le refus d'un nom unique à champ `cause`

AC1 autorise « deux événements distincts, **ou** un champ qui porte la raison ».
Le champ est refusé, pour une raison qui n'est pas stylistique : **le nom existant
affirme (a) dans son texte**. Garder `auto_feeder_no_backlog` comme parapluie avec
un `cause: "pool_in_flight"` rendrait un `grep auto_feeder_no_backlog` sur des
lignes de cas (b) — la fausse affirmation que AC4 corrige, réinstallée dans le nom
de l'événement. Précédent direct et récent : mika#2368, « deux noms, et c'est ce
qui sauve les sondes ».

| cause | `target_key` / nom de ligne | statut |
|---|---|---|
| (a) | `auto_feeder_no_backlog` | **conservé**, désormais véridique |
| (b) | `auto_feeder_pool_in_flight` | nouveau |
| (c) | `auto_feeder_in_flight_unreadable` | nouveau, attendu à zéro |

**La scission est datée du déploiement, et c'est voulu.** Les lignes déjà écrites
sous `auto_feeder_no_backlog` mélangent (a) et (b) ; les réécrire rendrait faux ce
qu'elles ont dit quand elles ont été écrites. Un opérateur qui compare de part et
d'autre du déploiement **doit sommer les trois noms**. Même geste et même raison
que mika#2361 sur `operator_review_or_blocked` / `abandoned_operator_held`.

Trois constantes, un seul site de définition, valeurs épinglées par test —
`auto_feeder` reste le `tool_name` d'audit, le nom de cause reste dans
`target_key`, donc la surface SQL existante
(`WHERE tool_name = 'auto_feeder'`) est préservée telle quelle.

## Cadence : inchangée, et c'est délibéré

Une ligne + une row d'audit par tick tant que la condition tient (jusqu'à
144/jour pendant un blocage). C'est **exactement la cadence actuelle** de
`auto_feeder_no_backlog` ; la changer casserait la comparabilité de la population
existante et n'est demandé par aucune AC. La doctrine mika#2131 (dédupliquer le
détail **par ticket**) ne s'applique pas : ceci est un agrégat par tick, et
pendant un blocage la vivacité *est* l'information — même raison que
`auto_pull_stop_armed` (mika#2329) et le Signal P (mika#2156).

## Acceptance criteria

- **AC1** — Quand `candidates.is_empty()`, le journal distingue **(a)** un vrai
  manque de backlog groomé de **(b)** un bassin non vide dont les tickets sont
  exclus par `in_flight`. Deux événements distincts, ou un champ qui porte la
  raison — pas un message unique qui affirme (a).
- **AC2** — L'événement emporte les nombres qui permettent de trancher sans aller
  lire ailleurs : compte brut de `ready`, compte `pullable`, compte des exclus
  pour `in_flight`, compte des exclus pour PR ouverte, compte des exclus pour
  `blocked`/`operator-review`.
- **AC3** — Dans le cas **(b)**, le message nomme le remède réel — les tickets en
  vol et depuis combien de temps — plutôt que d'attribuer le goulot au grooming.
- **AC4** — Le commentaire `// R7` est corrigé : il affirme aujourd'hui quelque
  chose que le code ne sait pas.
- **AC5** — Un test couvre les deux branches : bassin vraiment vide → (a) ;
  bassin de 6 `ready` dont 4 en vol → (b), et le test échoue si les deux
  produisent le même événement.
- **AC6** — Non-régression : `count_pullable_ready` n'est **pas** modifié. Son
  filtre est correct et il porte la correction d'un incident fondateur
  (mika#1863 R3/D2) ; ce ticket ne touche qu'à ce qu'on raconte du résultat.

## Definition of Done

1. `census_ready_pool` livrée, pure, additive, et son équivalence avec
   `count_pullable_ready` épinglée par test (AC2, AC6).
2. `classify_empty_backlog` livrée, pure, priorité (b) > (c) > (a) épinglée par
   test, y compris le cas « (a) et (b) vrais ensemble » de la nuit fondatrice (AC1).
3. Les trois noms sont des constantes à site unique, valeurs épinglées (AC1).
4. Le cas (b) nomme jusqu'à dix tickets avec statut et âge, et dit quand il
   tronque (AC3).
5. Les trois affirmations fausses sont corrigées : le `// R7`, le doc-comment de
   `phase0_feed_ready_pool`, celui de `FEEDER_WORKING_SET_CAP` (AC4).
6. `CLAUDE.md` racine corrigé — la même fausse affirmation y est servie à
   l'opérateur (AC4, en esprit).
7. Test hermétique `#[tokio::test]` sur le chemin d'émission réel, les deux
   branches produisant deux `target_key` différents, **rouge si on les fusionne**
   (AC5).
8. `count_pullable_ready` inchangé à la ligne près (AC6).
9. Aucun changement de politique de dispatch : le feeder rend toujours `0` et ne
   promeut rien dans ce bloc.

## Unités

**U1 — Recensement (pure).** `ReadyPoolCensus` + `census_ready_pool`, dans
`auto_pull.rs` à côté de `count_pullable_ready`. Réutilise
`feeder_exclusion_label` pour le seau `operator_held`, donc la raison d'exclusion
du bassin `ready` ne peut pas décrire une règle que la boucle a cessé d'appliquer.

**U2 — Classification (pure).** `EmptyBacklogCause` (trois variantes) +
`classify_empty_backlog(&ReadyPoolCensus) -> EmptyBacklogCause`. `match` exhaustif
**sans bras `_ =>`** pour le nom d'événement et pour le corps du message, afin
qu'une quatrième cause ne compile pas tant qu'elle n'a pas décidé des deux
(modèle `dispatch_substrate_diagnostic`, mika#2290).

**U3 — L'âge des tickets en vol (AC3).** Nouvelle méthode DB
`find_active_self_dev_task_for_issue(agent_id, issue_url) -> Result<Option<InFlightTask>>`
(`db/tasks.rs`), portant `task_id`, `status`, `in_flight_since` =
`COALESCE(fired_at, created_at)`, `ORDER BY … ASC LIMIT 1` — le **plus ancien**,
puisque la question diagnostique est « depuis combien de temps ».
`has_active_self_dev_task_for_issue` devient `…().map(|o| o.is_some())` : un seul
site SQL répond à la question, plutôt que deux requêtes qui peuvent diverger
(leçon `grooming_marker`, mika#2158). Équivalence épinglée par test. La boucle de
sonde de `phase0_feed_ready_pool` appelle la nouvelle méthode — **même nombre de
requêtes qu'aujourd'hui**, une par cible de sonde — et collecte en plus
`probe_failed_issue_numbers`, sous-ensemble d'attribution seulement :
`in_flight_issue_numbers` reste l'union exacte d'aujourd'hui, donc
`count_pullable_ready` et `select_feeder_candidates_recording` se comportent au
bit près comme avant.

**U4 — Émission + correction des affirmations.**
`emit_empty_backlog_signal(db, &census, cause, min_ready, trace_id, session_id)`,
async, fine ; le bloc appelant devient une ligne. Les trois doc-comments fautifs
corrigés (AC4).

**U5 — Gardes.** Matrice pure ; test hermétique `#[tokio::test]` sur
`emit_empty_backlog_signal` avec `Database::open_in_memory()` + relecture de
`audit_events` (modèle `mika2361_phase2_names_an_abandoned_held_ticket_under_its_own_filter`,
déjà dans ce fichier) ; scan de source SOLE WRITER sur les trois noms.

**U6 — `CLAUDE.md` racine.** L'entrée `MIKA_AUTO_FEEDER_MIN_READY` décrit
`auto_feeder_no_backlog` comme « a **grooming-throughput** bottleneck signal » —
la fausse affirmation, servie à l'opérateur. Corrigée, les trois noms documentés
avec leur régime attendu, et la scission datée écrite noir sur blanc.

## Fire-Disposition

Livrables de classe détecteur : (1) l'invariant d'équivalence recensement ↔
`count_pullable_ready`, (2) le test hermétique de discrimination des deux
branches, (3) le scan de source SOLE WRITER sur les trois noms d'événement.

**Option retenue — (c) halt-and-surface, pour le scan ; sans objet pour (1) et
(2).**

- (1) et (2) ne portent que sur du code neuf et n'ont **aucune donnée préexistante**
  sur laquelle tirer : `census_ready_pool` et `classify_empty_backlog` n'existent
  pas avant ce ticket.
- (3) **peut** tirer sur l'existant, et le cas est réel : `auto_feeder_no_backlog`
  apparaît aujourd'hui dans **trois doc-comments** (`auto_pull.rs:132`, `:1128`,
  `:1136`) en plus de son unique site d'écriture. Un scan lexical naïf les
  compterait comme des écrivains — c'est exactement le piège que mika#2329 a dû
  nommer (« nommer le chemin de la sentinelle dans un doc-comment est un hit
  aussi ») et le faux positif de prose du Signal S (mika#2050). **Le scan
  dépouille donc les commentaires avant de chercher**, comme le fait déjà
  `no_dispatch_test.rs` (« Comments are stripped before scanning so doc prose can
  describe what's forbidden »). Après dépouillement, l'inventaire est de **un**
  site par nom.
- **Allowlist livrée vide.** Il n'y a rien à exempter, donc aucun créneau où
  déposer le prochain manquement (règle mika#2323). Si le scan tire un jour, la
  résolution est **halt-and-surface** et jamais une entrée d'allowlist : « ce
  second site écrit-il la même population ? » est une question que la garde ne
  peut pas trancher à la place de l'humain, et une mauvaise réponse scinde
  silencieusement un compteur d'opérateur.

## Vérification (rouge-avant pour chaque garde)

| # | test | rouge-avant obtenu en |
|---|---|---|
| V1 | `mika2161_le_recensement_est_additif` | retirant un seau de la somme |
| V2 | `mika2161_le_recensement_explique_le_compte_pullable` | changeant l'ordre des filtres du recensement |
| V3 | `mika2161_la_nuit_fondatrice_rend_pool_in_flight` (6 `ready`, 4 en vol, 0 candidat) | inversant la priorité (a)/(b) |
| V4 | `mika2161_un_bassin_vraiment_vide_rend_no_backlog` | idem |
| V5 | `mika2161_une_sonde_illisible_naffirme_ni_lun_ni_lautre` | fondant `probe_failed` dans `in_flight` |
| V6 | `mika2161_les_deux_branches_ecrivent_deux_target_key` (hermétique, DB mémoire) | fusionnant les deux noms |
| V7 | `mika2161_les_noms_devenements_sont_un_format_de_fil` | changeant une valeur |
| V8 | `mika2161_chaque_nom_a_un_seul_ecrivain` (scan, commentaires dépouillés) | dupliquant un littéral |
| V9 | `mika2161_le_booleen_en_vol_delegue_au_lecteur_unique` | laissant les deux SQL diverger |

**Contrôle négatif porteur (V6).** Le test n'assère pas seulement que chaque
branche écrit *quelque chose* : il assère que les deux `target_key` **diffèrent**.
Une assertion de non-vacuité seule serait satisfaite par un classifieur constant —
c'est-à-dire par le défaut qu'on répare.

`make test` + `make lint` + `make fmt` verts.

## Sonde post-déploiement, et ses trois haltes

```bash
grep auto_feeder_pool_in_flight "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{raw_ready, pullable, in_flight, open_pr, operator_held, stuck}'
grep auto_feeder_in_flight_unreadable "$MIKA_SPIRIT_LOG_FILE"
```
```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'auto_feeder' GROUP BY 1;
```

**Halte 1 — `auto_feeder_in_flight_unreadable` non vide.** Régime attendu zéro. Une
occurrence est une sonde DB qui échoue, donc un diagnostic que le feeder ne peut
plus poser. **Ne pas fondre (c) dans (b) pour faire disparaître la ligne** : c'est
la fusion que ce ticket défait.

**Halte 2 — `auto_feeder_pool_in_flight` soutenu sur les mêmes tickets pendant des
heures.** La cause n'est pas ici : ce sont mika#2158 / mika#2160 / mika#2156,
explicitement hors périmètre. L'événement a fait son travail en le disant ; le
remède est en amont.

**Halte 3 — `auto_feeder_no_backlog` reste dominant après déploiement alors que le
bassin `ready` est visiblement coincé.** Le classifieur ne voit pas les exclus
`in_flight`. **Ne pas ajuster de seuil** : vérifier d'abord que la boucle de sonde
remplit bien `in_flight_issue_numbers` (un `MIKA_AUTO_FEEDER_MIN_READY` à `0`
court-circuite Phase 0 avant toute sonde), puis que le binaire déployé porte le
correctif — classe mika#2340.

## Hors périmètre

- **La cause pour laquelle les tickets sont coincés** : mika#2158 (livelock des
  prédicats de grooming), mika#2160 (sérialisation de la classe `implement`),
  mika#2156 (balayage phantom). Ce ticket ne débloque rien — il fait en sorte
  qu'on sache quoi débloquer.
- **Toute politique de dispatch** : ni le seuil, ni les filtres, ni les créneaux,
  ni le cap de working-set ne bougent. `count_pullable_ready` est inchangé (AC6).
- **La cadence d'écriture** (une ligne + une row par tick) : inchangée, par
  comparabilité avec la population existante.
- **Le cas « bassin en vol *et* candidats disponibles »**, où le feeder promeut et
  se tait. L'opérateur gagnerait peut-être à savoir qu'il promeut dans un bassin
  coincé, mais AC1 borne le changement à `candidates.is_empty()` et l'élargir
  serait une extension de portée non demandée. **Ticket de suivi**, conditionné à
  une mesure : si `auto_feeder_promoted` et `auto_feeder_pool_in_flight`
  s'alternent sur les mêmes ticks, la question se pose avec un compte.
- **La réécriture des lignes historiques** : interdite, elles étaient vraies
  quand elles ont été écrites.

## Fichiers probables

- `crates/mika-agent/src/auto_pull.rs` — recensement, classification, émission,
  les trois doc-comments, les gardes
- `crates/mika-agent/src/db/tasks.rs` — `find_active_self_dev_task_for_issue`
- `crates/mika-agent/src/async_db.rs` — le passe-plat async
- `CLAUDE.md` — entrée `MIKA_AUTO_FEEDER_MIN_READY`

## Closes

Closes #2161

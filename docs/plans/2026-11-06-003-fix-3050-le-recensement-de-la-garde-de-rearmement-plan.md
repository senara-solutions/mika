# fix(3050) — la garde de ré-armement recense ses passages, pour qu'AC4 ait un dénominateur

> Ticket : senara-solutions/mika#3050
> Plan antérieur : `docs/plans/2026-11-06-002-fix-3050-le-reaper-relit-le-ticket-avant-de-rearmer-plan.md`
> (livré par la PR #3052, mergée le 2026-11-06 à 14:55:15Z, `Refs #3050`).

## 0. Ce que ce vol trouve en arrivant, et qu'il ne peut pas re-livrer

**AC1, AC2 et AC3 sont dans `main`.** Le commit `a645a6f9b` (squash de #3052) porte
la garde en position 3 de `rearm_deferred_callback`, la variante
`RearmOutcome::ConsentWithdrawn { reason }`, le prédicat d'état de
`ready_label.rs`, et treize tests. La vérification tient sur la révision courante :

| propriété d'AC | site sur `a645a6f9b` | lecture |
|---|---|---|
| relecture avant ré-armement | `skills/executor.rs:5861` `reread_consent_before_rearm` | présent |
| terminal + ligne d'audit, écrits une fois | `skills/executor.rs:5971` `write_rearm_consent_terminal` | présent |
| les **trois** chemins de ré-armement passent par la garde | `engine.rs:2178`, `engine.rs:2697`, `dispatcher.rs:4047` | tous trois passent `github_token` |
| fail-closed sur relecture en erreur / jeton absent / parente illisible | `executor.rs:5893`, `:5917`, `:5957` | `RearmOutcome::NotNow` |
| AC3 — hors classe `ready-label`, aucun appel réseau | `executor.rs:5906`, le `?` sur `parse_ready_label_task_label` | présent |

Un vol qui réécrirait cela produirait un diff vide ou une régression. **Ce plan ne
touche ni la garde, ni son prédicat, ni ses treize tests.**

Le ticket reste donc ouvert sur **AC4 seule** — une sonde de 14 jours après le
`make deploy`, qui n'a pas encore eu lieu. Une sonde n'est pas du code ; mais son
**instrument** en est, et c'est là que #3052 a laissé un trou.

## 1. Le défaut qui reste : le numérateur d'AC4 n'est pas restreignable

AC4 demande, textuellement, de compter « les ré-armements **de parentes
`ready-label`** dont le ticket ne portait pas `ready` à l'instant du
ré-armement ». Deux termes, et seul le second est lisible aujourd'hui.

**Le refus est lisible.** `rearm_refused_consent_withdrawn`
(`executor.rs:5981`) ne naît que sur la classe `ready-label`, par construction :
la garde rend `None` avant lui pour toute autre classe. Il porte `repo`, `issue`
et `reason`. C'est le contrôle positif de la clause « aucun croisement sur la
fenêtre » : s'il est non nul, un retrait de `ready` a bien croisé une parente en
attente, et la garde l'a attrapé.

**Le ré-armement qui part, lui, ne dit pas sa classe.** Les trois lignes de
ré-armement sont émises par leurs appelants, pas par la garde :

| ligne | site | champs |
|---|---|---|
| `stuck_pending_task_rearmed` | `engine.rs:2225` | `task_id`, `issue`, `age_seconds`, `previous_rearm_count`, `wrappers_seen` |
| `stale_blocked_dispatch_rearmed` | `engine.rs:2773` | idem |
| `deferred_dispatch_rearmed` | `executor.rs:6251` | idem, côté lanceur consommé |

Aucune ne porte la classe de la parente, et `dispatch_class` — qui est en portée
aux deux sites d'`engine.rs` — **ne la discrimine pas** : une parente
`ready-label` porte `dispatch_class = "groom"` (voir la fixture
`dispatcher.rs:8980`), exactement comme une parente de groom venue d'un autre
chemin. `OrphanedPendingTask` (`task_state/tasks.rs:952`) ne porte pas de champ
`label`, donc la classe n'est même pas disponible au site de log.

Conséquence opératoire, et c'est la classe mika#2205 : les trois populations
— `ready-label`, `implement`, groom hors `ready-label` — sortent sous un seul
nom. Pour isoler celle qu'AC4 nomme, l'opérateur doit rejoindre chaque `task_id`
ré-armé à la base pour en relire le `label` — alors que la parente a pu être
expirée ou fauchée depuis, et que `task_id` n'est pas dans la ligne d'audit sous
une forme jointe au ticket. La mesure fondatrice du ticket (le tableau de six
lignes) a été faite exactement comme cela, à la main, et c'est ce coût qu'AC4
réclamera à nouveau dans 14 jours.

Et un **zéro** y est ambigu de la même façon que l'était celui de mika#3003 :
« aucun ré-armement sans consentement » et « aucune parente `ready-label` n'a
été ré-armée du tout » rendent le même octet. Le dénominateur n'existe pas.

## 2. Le site, et pourquoi il est unique

La garde **connaît** déjà la classe : `parse_ready_label_task_label(&parent.label)`
à `executor.rs:5906` la résout, et sur le chemin sain
(`Ok(RearmConsent::Held) => None`, l. 5922) elle la jette. C'est le seul endroit
du moteur où les trois faits — classe, coordonnées du ticket, verdict de la
relecture — sont vrais ensemble.

Le vol y pose **une ligne, émise sur toutes les branches, branche saine
comprise**. C'est le motif de `kg_budget_resolved` (mika#1833) et de
`team_stop_topic_inheritance` (mika#1926) : *un recensement qui ne compte que les
anomalies n'a pas de dénominateur.*

## 3. Le livrable — `rearm_consent_checked`

### D1. Un verdict de recensement, vocabulaire clos

Dans `ready_label.rs`, à côté de `RearmConsent` (l. 569) :

```rust
pub(crate) enum RearmConsentCensus {
    ParentAbsent,
    ParentUnreadable,
    OtherClass,
    NoToken { repo: String, issue: u64 },
    RereadFailed { repo: String, issue: u64 },
    Held { repo: String, issue: u64 },
    ReadyAbsent { repo: String, issue: u64 },
    IssueClosed { repo: String, issue: u64 },
}
```

Avec `as_str() -> &'static str` (huit littéraux `snake_case`, un `match`
exhaustif **sans bras `_ =>`** — mika#2290, mika#2413 : une neuvième branche de
la garde ne compilera pas sans se nommer), `repo() -> Option<&str>` et
`issue() -> Option<u64>`.

Huit variantes et non trois, parce que chaque absence a sa cause et que les
fusionner les rend non-soustractibles : `OtherClass` est la population d'AC3,
`ParentUnreadable`/`RereadFailed`/`NoToken` sont les trois fail-closed d'AC2, et
`Held` est le **dénominateur d'AC4**.

### D2. La scission de la garde, pour garder un écrivain unique

`reread_consent_before_rearm` devient deux fonctions :

- `reread_consent_verdict(...) -> (RearmConsentCensus, Option<RearmOutcome>)` —
  le corps actuel, inchangé dans sa logique : chacun de ses `return` rend en
  plus le verdict qui le décrit. Les quatre `warn!` existants
  (`rearm_parent_unreadable`, `rearm_reread_no_token`, `rearm_reread_failed`) et
  l'`info!` de `write_rearm_consent_terminal` **restent où ils sont** : ils
  portent `error`, `detail` et le texte de remède qu'un recensement ne porte
  pas. Ce sont les lignes de *diagnostic* ; la nouvelle est celle de *compte*.
  La distinction est nommée dans le doc-comment pour qu'un relecteur ne la lise
  pas comme un doublon.
- `reread_consent_before_rearm(...) -> Option<RearmOutcome>` — appelle la
  précédente, émet la ligne, rend l'issue. Signature publique inchangée, donc
  les trois appelants ne bougent pas.

La ligne :

```rust
info!(
    target: "mika::otel",
    event = "rearm_consent_checked",
    parent_task_id,
    cause,
    verdict = census.as_str(),
    repo = census.repo(),
    issue = census.issue(),
);
```

`repo` et `issue` sont **absents** — jamais `""`, jamais `0` — sur les trois
branches qui n'ont pas de coordonnées, par l'`impl Value for Option<T>` de
`tracing` : c'est le motif mika#2331, et c'est celui que
`team_stop_topic_inheritance` emploie déjà pour `parent_agent_id`
(`agent_loop/mod.rs:813`).

**Un seul écrivain**, et c'est ce qui fait du recensement un compte : deux sites
émettant `rearm_consent_checked` doubleraient silencieusement le dénominateur.

### D3. Ce que l'opérateur lit, dans 14 jours

| question d'AC4 | lecture |
|---|---|
| dénominateur — ré-armements de parentes `ready-label` partis | `verdict == "held"` |
| numérateur — ceux dont le ticket ne portait pas `ready` | de ces lignes, celles dont la timeline du ticket (`repo`, `issue`, horodatage) dit `ready` absent à cet instant. **Cible 0** |
| contrôle positif — un croisement a-t-il eu lieu ? | `verdict ∈ {"ready_absent", "issue_closed"}`, à joindre à `rearm_refused_consent_withdrawn` par `parent_task_id` |
| « aucun croisement sur la fenêtre » | `held > 0` et `ready_absent + issue_closed == 0` |
| population d'AC3, visible au lieu d'inférée | `verdict == "other_class"` |
| les fail-closed d'AC2 ont-ils tiré ? | `verdict ∈ {"parent_unreadable", "reread_failed", "no_token"}` |

**Caveat mika#2340.** L'instrument n'est pas vivant avant `make deploy`.
L'absence de la ligne ne veut pas dire « aucun ré-armement » mais « binaire
antérieur à ce vol », et un `*_rearmed` sans `rearm_consent_checked` apparié par
`parent_task_id` se lit comme tel — jamais comme une parente hors classe.
La fenêtre de 14 jours part du deploy, pas du merge.

## 4. Ce que ce vol ne fait pas, et pourquoi

- **Aucun script de mesure.** La famille `scripts/measure-*` + son suite
  `test-measure-*` pèse 650 à 720 lignes (mesuré : `measure-empty-turns` 392 +
  258, `measure-pilot-cycle-emptiness` 287 + 433), soit au-delà de la borne par
  vol, et AC4 demande un **compte écrit sur le ticket**, pas un outil. Avec le
  recensement de D3, cette lecture est un `jq` sur `verdict` plus une timeline
  par ligne `held` ; sans lui, aucun script ne pourrait la faire non plus,
  puisque la donnée manquante n'est pas dans les logs. L'instrument manquant est
  le champ, pas le lecteur.
- **Aucun champ ajouté aux trois lignes `*_rearmed`.** Ce serait la même
  information à trois sites — et il faudrait pour cela porter `label` jusque dans
  `OrphanedPendingTask` et son frère de `stale_blocked`, en traversant deux
  requêtes SQL, pour un fait que la garde tient déjà. Doctrine du lecteur unique
  (mika#2158, mika#2205, mika#3049).
- **Le résidu nommé par #3052** — les deux fixtures manuscrites `ready-label:`
  de `dispatcher.rs` (l. 8963 `#2108`, l. 9787 `#1614`) qui n'atteignent pas la
  couture `gh` — reste hors vol, et avec sa raison : l'assertion de
  `gh_process.rs:289` **est** la garde. Une fixture sous
  `FIXTURE_ISSUE_FLOOR` qui atteindrait la couture **panique**, donc la suite
  verte n'implique pas la propriété : elle *est* la propriété. Un test dédié
  assérerait ce qu'un panic tient déjà.
- **Le défaut 2 du ticket** (un tour mika-dev ferme un ticket qu'aucune PR ne
  ferme), les lanceurs différés des autres classes, et les phases d'auto_pull :
  hors vol par le ticket lui-même.
- **Aucun ticket de suivi ouvert** — gel jusqu'au 01/12. Le reste éventuel va en
  commentaire sur #3050 ou dans la PR.

## 5. Les tests

Six, et chacun couvre une classe que les autres ne voient pas. Ils réutilisent
les fixtures et les charges cannées de la couture `gh` posées par #3052
(`ISSUE_T1_CONSENT_HELD` … `ISSUE_T4_REREAD_FAILED`, `executor.rs:12798`), donc
aucune nouvelle numérotation sous `FIXTURE_ISSUE_FLOOR` et pas d'allowlist.

| test | ce qu'il refuse |
|---|---|
| `mika3050_le_recensement_nomme_held_sur_la_branche_saine` | un ré-armement qui part sans dénominateur — **le défaut de ce vol** |
| `mika3050_le_recensement_nomme_other_class_hors_ready_label` | la population d'AC3 confondue avec un `held` absent (label `operator-iterate:` réel, `server/iterate_dispatch.rs:569`, comme le test d'AC3 de #3052) |
| `mika3050_le_recensement_distingue_ready_absent_de_issue_closed` | les deux refus fusionnés sous un seul verdict |
| `mika3050_les_trois_fail_closed_portent_chacun_son_verdict` | `parent_unreadable`, `reread_failed`, `no_token` sous un nom commun |
| `mika3050_les_coordonnees_sont_absentes_et_jamais_vides_hors_classe` | `repo = ""` / `issue = 0` lu comme une coordonnée (mika#2331) |
| `mika3050_levenement_de_recensement_a_un_ecrivain_unique` | un second site émettant `rearm_consent_checked`, qui doublerait le dénominateur. Scan de source via `source_scan::production_files()`, allowlist **livrée vide** |

Les cinq premiers capturent la ligne par un `tracing` subscriber de test et
assèrent le couple `(verdict, repo, issue)` — pas seulement le `RearmOutcome`,
que les treize tests de #3052 couvrent déjà et qui resterait vert si la ligne
n'était pas émise. C'est la propriété qui sépare ce vol de son prédécesseur.

Le sixième porte son **contrôle de bonne foi** —
`mika3050_le_scan_decrivain_rougit_sur_un_second_site` sur une source
synthétique — sans quoi un scan qui ne trouve jamais rien passe aussi bien sur un
prédicat cassé que sur un dépôt sain.

## 6. Découpage en vols

Un seul vol, et il tient pour une raison mesurable : 345 lignes estimées contre
une borne de 429. Le périmètre est un type, une scission de fonction à signature
inchangée, une ligne de log, et six tests qui réutilisent des fixtures
existantes. Aucun appelant ne bouge, aucune requête SQL ne change, aucune
migration.

AC4 n'est pas découpée en vol : elle est **temporellement** hors d'atteinte
(14 jours après un deploy qui n'a pas eu lieu), et c'est son instrument, pas sa
mesure, que ce vol livre.

## 7. Definition of Done

1. `RearmConsentCensus` existe dans `ready_label.rs` avec ses huit variantes, son
   `as_str()` exhaustif sans bras `_ =>`, et ses deux accesseurs de coordonnées.
2. `reread_consent_verdict` rend le couple `(census, Option<RearmOutcome>)` ; la
   logique de décision de #3052 est inchangée — même verdicts, mêmes terminaux,
   mêmes raisons.
3. `reread_consent_before_rearm` émet `rearm_consent_checked` sur **les huit**
   branches, avec `repo`/`issue` absents et non vides sur les trois sans
   coordonnées.
4. Les quatre lignes de diagnostic existantes sont intactes ; leur rôle est
   distingué du recensement dans le doc-comment.
5. Les treize tests de #3052 restent verts sans modification.
6. Les six tests du § 5 sont verts, et le scan d'écrivain unique rougit sur son
   contrôle de bonne foi.
7. `cargo test -p mika-agent --lib`, `cargo clippy --all-targets --all-features
   -- -D warnings`, `cargo fmt --check`, `make verify-bundled-skills` et
   `bash scripts/verify-pipeline.sh origin/main` passent.
8. Le corps de PR porte le tableau de lecture du § D3 et le caveat mika#2340,
   puis `Refs #3050` — pas `Closes`, AC4 reste ouverte.

## 8. Acceptance criteria

Transcrits du corps de #3050, verbatim, avec l'état constaté sur `a645a6f9b`.

- **AC1.** Avant de ré-armer une parente `ready-label`, sur les deux chemins du
  tableau, le moteur relit le ticket à la forge : ouvert, et portant `ready`.
  - Si l'un des deux manque, aucun lanceur n'est créé, et aucun tour mika-dev n'a
    lieu.
  - La parente quitte la file dans un état terminal qui libère le prochain
    `ready`, comme `failed` aujourd'hui.
  - Une ligne d'audit nomme la raison : `ready_absent_at_rearm` ou
    `issue_closed_at_rearm`.

  *État : livré par #3052 (`a645a6f9b`). Ce vol ne la modifie pas ; les tests qui
  la tiennent doivent rester verts (DoD 5).*

- **AC2.** Fail-closed : si la relecture échoue, pas de ré-armement à ce tick
  (`RearmOutcome::NotNow`), avec la cause nommée dans le log.
  - Un test par cas, dans le même appel : `ready` retiré, ticket fermé,
    relecture en erreur.
  - Le contrôle positif (`ready` encore posé, la forme des 4 premiers cas du
    tableau) ré-arme comme aujourd'hui.

  *État : livré par #3052. Ce vol ajoute que chacune des trois causes porte son
  propre verdict de recensement, au lieu d'être seulement nommée dans un `warn!`
  (test 4 du § 5).*

- **AC3.** Les parentes d'une autre classe que `ready-label` gardent leur
  comportement actuel. Un test le garde.

  *État : livré par #3052. Ce vol rend cette population **comptable**
  (`verdict == "other_class"`) au lieu d'inférable par soustraction (test 2).*

- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les
  ré-armements de parentes `ready-label` dont le ticket ne portait pas `ready` à
  l'instant du ré-armement (timeline du ticket), puis on l'écrit ici. La cible
  est 0. Un compte nul sans aucun retrait de `ready` pendant qu'une parente
  attend s'écrit « aucun croisement sur la fenêtre ».

  *État : **c'est le vol.** Son numérateur n'était pas restreignable à la classe
  `ready-label` depuis les logs (§ 1), et son zéro était ambigu faute de
  dénominateur. `rearm_consent_checked` fournit les deux ; le protocole de
  lecture est le tableau du § D3. La mesure elle-même reste due 14 jours après le
  `make deploy`, et s'écrit en commentaire sur #3050.*

## 9. Fire-Disposition

Ce vol livre des détecteurs : six tests, dont un scan structurel de source
(`mika3050_levenement_de_recensement_a_un_ecrivain_unique`).

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée VIDE.**

Le scan compte les occurrences de `"rearm_consent_checked"` dans la moitié
production de `src/**/*.rs` (`source_scan::production_files()`, qui écarte les
chemins de test via `is_test_source_path` — mika#2321) et exige exactement **1**.
Aucune violation préexistante : l'événement est créé par ce vol, donc son unique
site est celui qu'il écrit. La constante d'allowlist est donc `&[]`, et c'est
assuré plutôt que supposé — un test frère (`…_lallowlist_est_vide`) la fixe
vide, de sorte qu'y ajouter une entrée exige de rougir ce test et d'y écrire la
donnée précise, son ticket de suivi et sa raison.

L'assertion auto-nettoyante est portée par le scan lui-même : il assère une
**égalité** à 1, pas un minorant. Un site retiré le fait rougir autant qu'un site
ajouté, donc l'exception ne peut pas devenir stale sans se voir.

Le contrôle de bonne foi (`mika3050_le_scan_decrivain_rougit_sur_un_second_site`)
est ce qui distingue « un seul écrivain » de « le prédicat ne trouve rien ».

Ni (b) ni (c) : rien n'est livré désarmé, et il n'y a pas de cadrage à remonter.

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/ready_label.rs` — `RearmConsentCensus`, `as_str`, `repo`, `issue` | 85 |
| `crates/mika-agent/src/skills/executor.rs` — scission `reread_consent_verdict` / `reread_consent_before_rearm`, la ligne unique | 55 |
| tests `crates/mika-agent/src/skills/executor.rs` — les cinq cas de branche | 150 |
| tests `crates/mika-agent/src/ready_label.rs` — écrivain unique, allowlist vide, contrôle de bonne foi | 55 |

Total estimé : 345 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Pourquoi cela tient sous la borne : aucun appelant ne change de signature, aucune
requête SQL ne bouge, aucune migration, et les cinq tests de branche réutilisent
les quatre fixtures et leurs charges cannées déjà posées par #3052 — l'échafaudage
de couture `gh` le plus coûteux est déjà écrit.

## 11. Ordre de commit

1. `feat(3050)` — `RearmConsentCensus` dans `ready_label.rs` + ses deux tests de
   vocabulaire (étiquettes deux à deux distinctes, allowlist vide). Le type seul
   compile et se teste sans toucher au moteur.
2. `refactor(3050)` — scission `reread_consent_verdict` / `reread_consent_before_rearm`,
   sans la ligne de log. Les treize tests de #3052 restent verts : **c'est le
   plancher**, et il dit que la scission n'a pas changé une décision.
3. `feat(3050)` — la ligne `rearm_consent_checked` + les cinq tests de branche.
4. `test(3050)` — le scan d'écrivain unique et son contrôle de bonne foi.

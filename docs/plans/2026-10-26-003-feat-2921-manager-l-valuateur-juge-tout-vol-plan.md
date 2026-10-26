# mika#2921 — l'évaluateur juge tout vol de la boucle, le rapport reste par milestone

## Contexte

L'évaluateur de vols de mika-manager (mika#2698, en mode ombre) ne juge que les
vols dont le ticket est une sous-issue du milestone cible. Le chemin est droit et
entièrement visible :

- `cadence.rs::collect_flight_verdicts` appelle `milestone_issue_urls(state)`,
  qui projette `state.sub_issues` en URLs `…/issues/N` ;
- si la liste est vide il rend `FlightScan::no_milestone_issues()` et zéro verdict ;
- sinon il passe la liste à `FlightSource::finished_implement_flights(since, &issue_urls)` ;
- `DbFlightSource` la transmet à
  `Database::list_finished_implement_flights_in_container(since, issue_urls)`
  (`db/tasks.rs:4964`), dont la clause est `reference_url IN (<variantes>)`.

`MIKA_MANAGER_TARGET_MILESTONE` vaut `senara-solutions/mika#33`. Les 13 vols
`implement` terminés mesurés en base le 2026-10-26 (après le 2026-10-23T15:00Z)
sont **tous hors de ce milestone** : 12 `completed` (mika#2900, #2879, #2897,
#2908, #2909, #2917, mika-cloud#193, #196, #198) et 1 `failed`
(claude-pilot#232). Le dernier rapport du sink (2026-10-26T11:04:50Z) ne porte
donc aucune section « Vols terminés », et le correctif mika#2886 (PR #2892,
mergé le 2026-10-23) n'a jamais été vu à l'œuvre : l'évaluateur n'a plus rien
jugé depuis.

Conséquence opérationnelle, et c'est elle qui donne la priorité : l'accord ombre
exigé avant d'ouvrir `MIKA_MANAGER_EVALUATE_WRITE` (seuil mika#2698 : verdicts
égaux à ceux de MPC sur 10 vols consécutifs ou 7 jours) **ne peut pas se
mesurer** sur une population vide.

Bearing (mika-prime, 2026-10-26, relayé par samidarko, verbatim) :

> « La boucle, pour juger : le manager qui ne juge qu un milestone n est pas un
> manager, c est un rapporteur du milestone [...] juger tout vol implement
> terminé sur les dépôts contrôlés, rapporter par milestone — la limite à garder
> étant que « boucle » = exactement le périmètre des dépôts sous contrôle, jamais
> au-delà, et que l accord ombre doit être mesuré et les verdicts stockés *avant*
> tout réarmement. »

## Ce qui est refusé (bearing, non négociable)

- **Aucune autorité nouvelle.** `MIKA_MANAGER_EVALUATE_WRITE` reste OFF, le
  chemin d'écriture de `verdict_poster.rs` n'est pas touché, et ce vol ne poste
  rien — ni commentaire, ni label, ni merge. Le module `milestone_manager/`
  reste LECTURE SEULE et `no_dispatch_test.rs` reste vert sans exception nouvelle.
- **Aucun réarmement.** Ouvrir un interrupteur est un geste de Vincent sur mesure
  présentée. Ce vol produit la mesure, pas le geste.
- **Jamais au-delà du périmètre.** L'élargissement est borné par une liste de
  dépôts, pas par « tout ce que la base contient ».
- **Le rapport reste par milestone.** Ce qu'il dit du milestone cible ne change
  pas d'un octet, à l'exception de la section « Vols terminés ».

## Point 1 tranché — la liste des dépôts sous contrôle est `DISPATCHABLE_REPOS`

**Décision : réutiliser `DISPATCHABLE_REPOS`
(`crates/mika-agent/src/webhook_dispatch.rs:385`, 4 dépôts : mika, mika-cloud,
mika-skills, mika-platform), sans en écrire une ligne ailleurs.**

Quatre raisons, dans l'ordre où elles mordent.

1. **C'est l'arbitrage déjà rendu, sur la même question.**
   `server/ci_failure_handler.rs:575-605` porte le raisonnement écrit, en toutes
   lettres : « Two lists exist and they are not the same question. The gateway's
   `INTERNAL_REPOS` decides which repositories **route** to mika-dev — it carries
   `claude-pilot` and `wizzard`. `DISPATCHABLE_REPOS` decides where the loop may
   **dispatch**, and the 2026-08-29 operator decision keeps those two out of it.
   […] A third list would be the programmed divergence this repo has already paid
   for twice (dispatch seats mika#2092, `DISPATCHABLE_REPOS` ↔ `labels.yml`). »
   La question de ce ticket — « ce vol appartient-il à la boucle ? » — est la
   question du dispatch, pas celle du routage. Rouvrir cet arbitrage demanderait
   une mesure nouvelle ; ce ticket n'en apporte pas.

2. **Le seul choix qui ne crée aucune liste.** `INTERNAL_REPOS` est **privée**
   dans `mika-gateway/src/github.rs:307`, et `mika-agent` ne dépend pas de
   `mika-gateway` (sa seule dépendance d'espace de travail est `mika-common`). La
   « réutiliser » depuis le manager exigerait de la **promouvoir dans
   mika-common** et de faire du gateway un consommateur — c'est-à-dire de
   déplacer une liste sécurité-adjacente du gateway dans un ticket sur le
   manager, un changement d'autorité déguisé en dé-duplication. `DISPATCHABLE_REPOS`
   est déjà `pub(crate)` dans le crate où le code vit : un `use`, zéro ligne de
   liste.

3. **« Jamais au-delà » désigne la plus serrée.** `INTERNAL_REPOS` *autorise* le
   routage de dépôts où la boucle ne dispatche pas (`wizzard`). Déclarer qu'on
   juge ce périmètre serait exactement la confusion « permis versus existe » que
   le doc-comment de `DISPATCHABLE_REPOS` nomme — dans l'autre sens.

4. **Le coût est nommé, mesuré, et devient un contrôle.** claude-pilot#232 sort
   du périmètre jugé. Ce n'est pas une perte masquée : c'est **le contrôle
   négatif réel d'AC2**, une population observée plutôt qu'un vol fabriqué. Et le
   geste de réparation, si l'opérateur décide que claude-pilot est dans le
   périmètre de la boucle, est **une seule ligne dans une seule liste** —
   `DISPATCHABLE_REPOS` — qui ouvre du même coup la porte de dispatch
   (`is_dispatchable`, Gate 2b de `ready_label_handler.rs:112`). C'est un geste
   de Vincent, pas une décision de plan, et c'est la forme que le hors-périmètre
   du ticket prescrit déjà pour les interrupteurs.

**Lecteur unique.** La liste est lue à **un seul endroit** du chemin de
l'évaluateur : `crates/mika-agent/src/flight_source.rs`. Ni
`milestone_manager/**`, ni `db/tasks.rs` n'en voient une ligne — la requête
reçoit les dépôts en **paramètre**, comme elle recevait les URLs. C'est ce qui
rend AC3 structurel plutôt que déclaratif, et c'est la même raison qui fait vivre
`DbFlightSource` hors du module manager (le grep de `no_dispatch_test.rs`).

## Point 2 tranché — le verdict nomme « état de PR non lu », aucune lecture réseau

**Décision : aucun appel `gh` ajouté. Un vol dont le ticket n'est pas apparié à
une sous-issue du milestone cible porte un état de PR explicitement non lu, que
le rapport nomme.**

Quatre raisons.

1. **L'invariant du module est écrit et tenu.** `evaluator.rs:24` déclare, pour
   la ligne « état de la PR », « `SubIssue::pr_state` — déjà hydraté par le
   `Reader` via `gh`, **aucun appel de plus** ». `no_dispatch_test.rs` refuse les
   jetons réseau (`"POST"`, `"PATCH"`, `gh issue edit`, …) dans ce module. Payer
   une exception d'allowlist pour obtenir une donnée qu'on peut nommer absente
   est le mauvais côté de l'arbitrage mika#1574.

2. **`FlightSource` ne rend jamais d'erreur** — « une panne rend le vide, jamais
   une erreur » (`flight_source.rs`, WARN `flight_source_read_failed`). Un appel
   réseau par vol crée une **seconde cause d'absence sous le même nom** : un
   `pr_state` absent parce que `gh` a échoué et un `pr_state` absent parce que le
   vol est hors milestone rendraient les mêmes octets. C'est exactement la classe
   mika#2205.

3. **Le coût n'est pas borné par construction.** Le périmètre élargi *est* la
   boucle : N vols par cycle, à chaque cycle, pour une donnée dont la fraîcheur
   n'est garantie par rien. Le manager tourne en cadence ; une dépendance réseau
   proportionnelle à l'activité de la boucle est la dépendance dont la panne
   coïncide avec les jours chargés.

4. **La donnée n'est pas nécessaire au verdict utile.** Pour un vol qui a ouvert
   une PR, la recommandation reste « va regarder » avec ou sans l'état. Ce que le
   rapport doit cesser de faire, c'est **présenter cette recommandation comme si
   l'état avait été lu**. C'est le motif mika#2331 : `None` veut dire « non
   instrumenté », jamais « vide ».

**Forme.** `FlightVerdict` porte `pr_state: PrStateKnowledge`, un enum à deux
variantes, et non un `Option<String>` :

| variante | sens | origine |
|---|---|---|
| `Read(Option<String>)` | le ticket est apparié à une sous-issue du milestone cible ; l'état a été lu, et il peut être vide | `SubIssue::pr_state` |
| `NotRead` | le ticket n'est apparié à aucune sous-issue du milestone cible ; **ni** son milestone **ni** son état de PR n'ont été lus | aucune |

`Read(None)` (« lu, vide ») et `NotRead` (« pas lu ») sont deux populations
distinctes, et c'est tout l'objet du type : un `Option<String>` les aurait
fusionnées et l'AC5 serait intestable.

**Ce que le rapport nomme, et ce qu'il refuse de nommer.** L'AC du ticket demande
que la section « nomme le milestone de chaque vol, ou son absence ». Pour un vol
hors milestone cible, le milestone *réel* n'est pas connu — le `Reader` n'hydrate
que la cible, et le lire coûterait l'appel réseau que le point 2 refuse. Le
rapport dit donc, pour chaque vol, **soit** « milestone cible », **soit** « hors
milestone cible — état de PR non lu », et **jamais un numéro de milestone
deviné**. L'absence nommée est l'absence du milestone cible, pas une prétention
sur l'appartenance du ticket à un autre milestone.

## L'endroit, choisi et justifié

### 1. `db/tasks.rs` — le paramètre change de nature, pas la structure

`list_finished_implement_flights_in_container(since, issue_urls: &[String])`
devient `list_finished_implement_flights_in_container(since, repos: &[String])`.
La clause `reference_url IN (<variantes>)` devient une disjonction de `LIKE` sur
`'https://github.com/<repo>/issues/%'`, avec `ESCAPE` et échappement de `_`, `%`
et `\` dans le nom de dépôt — aucun dépôt actuel ne contient `_`, mais un piège
dormant dans une clause SQL est un piège.

Trois propriétés sont **préservées mot pour mot**, et chacune a un test déjà écrit :

- `repos.is_empty()` ⇒ `Ok(vec![])`. L'énoncé de mika#2698 devient « **aucun
  dépôt** ne peut pas rendre tous les vols », et c'est la même assertion.
- La requête 2 (enfants pilotes, topologie mika#2886) déplace la même clause sur
  `parent`, sans paramètre de liaison supplémentaire.
- Les trois filtres Rust (statuts terminaux, labels pilote, `result` non vide)
  restent en Rust. Le doc-comment refuse déjà une copie SQL de ces listes comme
  « une troisième copie de la liste » ; la liste de dépôts suit la même règle et
  n'est **pas** écrite ici.

### 2. `flight_source.rs` — le lecteur unique du périmètre

C'est le seul fichier du chemin qui lit `DISPATCHABLE_REPOS`, et c'est le bon :
il vit déjà **hors** de `milestone_manager/` précisément pour que la frontière du
grep soit nette. Il gagne :

- `fn controlled_repos() -> Vec<String>` — un `use crate::webhook_dispatch::DISPATCHABLE_REPOS`
  et une projection, zéro littéral de dépôt ;
- `fn repo_from_issue_url(url: &str) -> Option<String>` — l'extraction
  `owner/repo` d'une URL d'issue, tolérante au suffixe `?phase=groom` comme
  `issue_number_from_url` l'est déjà ;
- le `repo` posé sur `FinishedFlight` à la projection, comme une **donnée** de
  mesure (même statut que `agent_id` depuis mika#2792 : il distingue « aucun vol »
  de « mauvaise base »).

### 3. Le trait — le périmètre devient une propriété de la source

```rust
async fn finished_implement_flights(&self, since: Option<&str>) -> Vec<FinishedFlight>;
```

Le paramètre `issue_urls` disparaît. C'est le cœur de la décision de forme : si
l'appelant continuait de passer un périmètre, `cadence.rs` — donc le module
LECTURE SEULE — devrait connaître les dépôts, et AC3 ne serait tenu que par
discipline. En déplaçant le périmètre dans la source, le module manager n'a plus
aucun moyen d'exprimer une liste de dépôts, et la garde d'AC3 épingle un état que
le code rend déjà vrai.

### 4. `cadence.rs` — ce qui disparaît, et ce qui le remplace

`collect_flight_verdicts` perd `milestone_issue_urls(state)` et la branche
`FlightScan::no_milestone_issues()`. Cette branche **disparaît avec sa raison**,
et c'est un changement de sens du journal à documenter au site : son absence ne
signifiera plus « le milestone n'a pas de sous-issues » mais rien du tout. Le
`FlightScan::read(issue_count, …)` voit son premier champ renommé en `repo_count`
(le nombre de dépôts du périmètre) ; `manager_flight_scan` garde
`scope = "container"`, ses trois autres champs et son émission inconditionnelle —
la mesure reste lisible après l'élargissement, ce qui est la condition pour que
AC6 soit vérifiable.

`milestone_issue_urls` reste exportée et utilisée par les autres consommateurs de
`MilestoneState` ; elle cesse seulement d'être le périmètre des vols.

### 5. `evaluator.rs` — deux champs, aucune lecture nouvelle

- `FlightVerdict` gagne `repo: Option<String>` (`None` quand le vol ne porte pas
  d'URL — motif mika#2331, jamais une chaîne vide) et
  `pr_state: PrStateKnowledge`.
- `evaluate_flights(flights, sub_issues)` garde sa signature. L'appariement par
  `issue_number` existe déjà ; ce qui change est que l'échec d'appariement cesse
  d'être traité comme « état absent » pour devenir `NotRead`.
- `recommend(issue, pr_state: &PrStateKnowledge)` : `Read(Some(s))` non-`open` ⇒
  `ManualResumption` ; `Read(None)` ⇒ `MergeCandidate` (comportement actuel,
  inchangé) ; `NotRead` ⇒ `MergeCandidate`, et c'est le **rendu** qui porte la
  non-lecture. Les arms `RescuePr` et `Dead { … }` sont intouchés.

Le test existant `mika2698_letat_de_pr_vient_du_sous_ticket_apparie` — « un
sous-ticket non apparié ne prête pas son état » — devient plus fort, pas plus
faible : le verdict d'un vol non apparié dit désormais qu'il n'a rien lu.

### 6. `reporter.rs` — la seule section qui bouge

La ligne de `render_flight_verdict` passe de quatre à six segments :

```
- {repo}#{n} — {milestone} — {issue} — {turns} — {état de PR} — {reco}
```

avec `{repo}#{n}` = `mika#2900` (et `"(ticket inconnu)"` conservé tel quel quand
le numéro manque), `{milestone}` ∈ { `milestone cible`, `hors milestone cible` },
`{état de PR}` ∈ { `PR {state}`, `état de PR non lu` } — et rien du tout quand
`Read(None)`, pour que le vol du milestone cible sans état lu ne gagne pas un
segment qui ne dit rien. Le titre de la section nomme le périmètre : « Vols
terminés depuis le cycle précédent (toute la boucle — mode ombre, aucune
écriture) ».

Le placement est inchangé (entre « Next action » et « ### Silent-progress / drift
alerts »), la condition est inchangée (`!flight_verdicts.is_empty()`), et les
assertions d'ordre existantes restent vraies. C'est ce qui rend AC4 atteignable :
le bloc « Vols terminés » est le seul insert conditionnel de cette zone.

## Requirements

- **R1.** `Database::list_finished_implement_flights_in_container` prend
  `repos: &[String]` et filtre `reference_url` sur `https://github.com/<repo>/issues/%`
  avec `ESCAPE`. `repos.is_empty()` ⇒ `Ok(vec![])`. Aucun nom de dépôt n'est écrit
  dans ce fichier. La requête 2 (enfants pilotes) porte la même clause sur
  `parent`, sans paramètre de liaison supplémentaire.
- **R2.** `flight_source.rs` est le **lecteur unique** de `DISPATCHABLE_REPOS` sur
  le chemin de l'évaluateur, via `use crate::webhook_dispatch::DISPATCHABLE_REPOS`.
  Il expose `repo_from_issue_url` et pose `repo` sur `FinishedFlight`.
- **R3.** `FlightSource::finished_implement_flights(since)` perd son paramètre
  `issue_urls`. Aucun fichier de `milestone_manager/` ne nomme un dépôt.
- **R4.** `collect_flight_verdicts` n'appelle plus `milestone_issue_urls` ; la
  branche `FlightScan::no_milestone_issues()` est retirée avec un commentaire qui
  dit ce que son absence ne signifie plus. `FlightScan::read` porte `repo_count`,
  et `manager_flight_scan` reste émis sur **toutes** les branches.
- **R5.** `FlightVerdict` porte `repo: Option<String>` et
  `pr_state: PrStateKnowledge` (`Read(Option<String>)` | `NotRead`), tous deux
  sérialisés avec `#[serde(default)]` pour que les `Assessment` persistés avant ce
  vol se relisent (motif des tests serde de `types.rs:481-518`).
- **R6.** `recommend` prend `&PrStateKnowledge`. `Read(None)` conserve
  `MergeCandidate` — le comportement actuel pour un vol du milestone cible sans
  état hydraté ne bouge pas.
- **R7.** `render_flight_verdict` rend les six segments ci-dessus. Le segment
  d'état de PR est **omis** pour `Read(None)` et vaut `état de PR non lu` pour
  `NotRead`. Aucun numéro de milestone n'est rendu pour un vol hors cible.
- **R8.** Une garde structurelle (`milestone_manager/no_repo_list_test.rs`)
  refuse tout littéral `senara-solutions/` dans la moitié production de
  `src/milestone_manager/**` et de `src/flight_source.rs`, exception faite du
  `use` nommé de `DISPATCHABLE_REPOS`. Classification test via
  `crate::source_scan::is_test_source_path` **et** troncature au premier
  `#[cfg(test)]` — la prémisse mika#2321 (un module de test extrait ne porte pas
  le littéral) est nommée au site.
- **R9.** `verdict_poster.rs` et `types.rs` compilent sans changement de
  comportement : seules leurs fixtures gagnent les deux champs. Le chemin
  d'écriture reste gardé par `write_is_armed()`.

## Non-Requirements (hors de ce vol, délibérément)

- **Garder les verdicts et compter la concordance ombre** — ticket compagnon
  (registre des verdicts). Ce vol rend la population non vide ; il ne la persiste
  pas.
- **La PR liée dans le rapport** — ticket compagnon, à grouper avec le registre.
- **Ouvrir `MIKA_MANAGER_EVALUATE_WRITE`** — geste de Vincent sur mesure
  présentée.
- **Faire entrer claude-pilot et wizzard dans le périmètre** — décision
  d'opérateur, et son geste est une ligne dans `DISPATCHABLE_REPOS` qui ouvre
  aussi la porte de dispatch. Nommé au § Point 1, mesuré par AC2.
- **Promouvoir `INTERNAL_REPOS` dans `mika-common`** — refusé par le § Point 1,
  pas reporté.
- **Lire le milestone réel d'un vol hors cible** — refusé par le § Point 2, pas
  reporté. Rouvrir exige d'abord de renégocier l'invariant « aucun appel de
  plus » d'`evaluator.rs:24`.
- **Toucher `reader.rs`** — l'hydratation de `SubIssue::pr_state` reste
  exactement ce qu'elle est.

## Verification Contract

- **V1 — AC1/AC2, une vraie base.** Dans
  `crates/mika-agent/src/db/tests/vols_termines_du_manager.rs` : une table de 13
  tuples reproduisant la forme exacte des 13 vols mesurés (9 tickets `completed`
  sur mika et mika-cloud, claude-pilot#232 `failed`, et les répétitions qui
  complètent le corpus), semés via `seme_la_forme_reelle()` /
  `crate::skills::executor::build_callback_task` — donc la topologie à deux
  lignes de mika#2886, pas un bouchon. Le test passe par `DbFlightSource` puis
  `evaluate_flights` et assert : 12 verdicts, et **aucun** pour claude-pilot#232.
  Le même test porte le contrôle négatif : ni le verdict ni le rapport ne
  nomment ce vol.
- **V2 — AC5.** Un vol sur mika hors du milestone cible rend
  `pr_state == PrStateKnowledge::NotRead`, et sa ligne de rapport contient
  `état de PR non lu`. Contrôle positif dans le même test : un vol **dans** le
  milestone cible, apparié à une `SubIssue { pr_state: Some("open") }`, rend
  `Read(Some("open"))` et une ligne qui ne contient pas `non lu`. Les deux arms
  côte à côte sont ce qui sépare « le type est correct » de « le rendu est
  branché ».
- **V3 — AC4, octet pour octet.** Un `Assessment` fixe est rendu ; le rapport est
  découpé à la section « Vols terminés » et les deux moitiés sont comparées à un
  golden figé dans le test. Un changement dans n'importe quelle autre section
  rougit.
- **V4 — AC3.** La garde de R8 : allowlist livrée **vide**, assertion de vacuité,
  contrôle de bonne foi sur une entrée fabriquée (le scan rougit sur un littéral
  injecté), et compte anti-vacuité sur l'arbre réel — « le scan n'a rien trouvé »
  doit être distinguable de « le scan n'a rien regardé » (mika#2205).
- **V5 — non-régression du périmètre vide.** « Aucun dépôt ne rend jamais tous
  les vols » : `list_finished_implement_flights_in_container(None, &[])` rend
  `vec![]`, l'héritier direct de `mika2698_aucune_url_ne_rend_jamais_tous_les_vols`.
- **V6 — la suite complète.** `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` (inchangé mais exécuté), et
  `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- R1 à R9 implémentés, V1 à V6 verts.
- `no_dispatch_test.rs` vert **sans entrée nouvelle** dans sa table d'exceptions —
  le module manager n'a gagné aucun jeton interdit.
- Aucun appel réseau ajouté : `grep -rn "gh \|reqwest\|http" crates/mika-agent/src/milestone_manager/ crates/mika-agent/src/flight_source.rs`
  ne rend rien de nouveau en production.
- `MIKA_MANAGER_EVALUATE_WRITE` non mentionné dans le diff autrement que par les
  lignes déjà présentes.
- La sonde post-déploiement du § dédié est écrite dans ce plan et reportée dans le
  corps de la PR.
- Merge par Vincent (`loop-substrate`).

## Acceptance criteria

- **AC1, la boucle visible.** Sur un corpus construit à partir des 13 vols
  mesurés ci-dessus (ou de leur forme exacte), chaque vol sur un dépôt sous
  contrôle reçoit un verdict, qu'il soit dans le milestone cible, dans un autre
  ou dans aucun. Test sur une vraie base, pas sur un bouchon de `FlightSource`.
- **AC2, jamais au-delà (contrôle négatif dans le même test).** Un vol
  `implement` terminé dont le ticket est sur un dépôt **hors** de la liste n'est
  pas jugé et n'apparaît pas dans le rapport.
- **AC3, une seule liste.** Une garde fait rougir l'ajout d'une troisième liste
  de dépôts dans le module du manager.
- **AC4, le rapport par milestone inchangé.** Les sections du milestone cible
  rendent les mêmes octets avant et après, sur la même entrée, à l'exception de
  la section « Vols terminés ».
- **AC5, l'état de PR hors milestone.** Il suit la règle choisie au point 2. Si
  c'est « non lu », le verdict le nomme, et un test l'atteste.
- **AC6, la sonde post-déploiement**, écrite dans le plan : au premier cycle
  après `make deploy`, le rapport du sink porte une section « Vols terminés » non
  vide, et au moins un verdict **classé** (autre que `no_outcome_line`) sur un vol
  qui a ouvert une PR. C'est la première preuve en production de #2892.

**Correspondance AC → vérification :** AC1 → V1 ; AC2 → V1 (contrôle négatif,
population réelle) ; AC3 → V4 ; AC4 → V3 ; AC5 → V2 ; AC6 → § Sonde
post-déploiement.

## Fire-Disposition

Ce plan livre des détecteurs : la garde structurelle de R8 (AC3) et les tests de
V1-V5. L'option retenue est **(a) exception nommée en allowlist — et l'allowlist
est livrée vide**, parce qu'il n'y a aucune violation existante à couvrir : le
module `milestone_manager/` ne contient aujourd'hui aucun littéral
`senara-solutions/` en **production** (les occurrences sont toutes dans les
`mod tests` inline, correctement classées), et le périmètre de l'évaluateur
n'écrit aucune liste de dépôts après ce vol.

Détail d'implémentation, sur le modèle de
`no_dispatch_test.rs::mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier` :

- **Table d'exceptions** `REPO_LITERAL_ALLOWED`, livrée **vide**, et clé par
  **(fichier, motif, raison, ticket de suivi)** — jamais par fichier seul. Une
  exception par fichier couvrirait le littéral suivant sans que personne décide.
- **Assertion auto-nettoyante** : un test assert que la table est vide. Le jour
  où une entrée y est ajoutée, ce test rougit et force la question — « pourquoi
  une liste de dépôts dans ce module ? » — au lieu de la laisser passer comme une
  donnée de configuration. Si l'entrée est légitime, c'est cette assertion qu'on
  modifie, délibérément, avec son ticket.
- **Contrôle de bonne foi** : un second test injecte un littéral fabriqué dans un
  contenu synthétique et assert que le scan le **voit**. Un détecteur qui démarre
  vert n'a jamais montré qu'il regarde.
- **Garde-fou anti-vacuité** : un plancher sur le nombre de fichiers et de lignes
  effectivement scannés sur l'arbre réel. Une racine qui dérive, un
  `is_test_source_path` trop large, et le scan rend zéro violation pour la
  mauvaise raison — le couple « zéro trouvé » / « rien regardé » est exactement
  la classe mika#2205.
- **Le `use` de `DISPATCHABLE_REPOS` est exempté par forme, pas par fichier** :
  le motif accepté est l'import nommé, de sorte qu'un littéral écrit dans le même
  fichier reste refusé.
- **Prémisse de classification nommée au site** : la troncature au premier
  `#[cfg(test)]` est aveugle à un module de test **extrait** (mika#2321), d'où la
  consultation conjointe de `crate::source_scan::is_test_source_path`. Si la
  garde rougit après un déplacement de code de test, **réparer la
  classification — jamais élargir l'allowlist, jamais élargir le motif.**

Les détecteurs de V1-V5 sont des tests ordinaires et atterrissent **armés** : ils
passent sur le code de ce vol, et aucun n'est livré `#[ignore]`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (paramètre `repos`, prédicat `LIKE`/`ESCAPE`, doc) | 45 |
| `crates/mika-agent/src/flight_source.rs` (périmètre, `repo_from_issue_url`, projection, tests) | 60 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (`PrStateKnowledge`, `repo`, `recommend`, tests) | 85 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (`collect_flight_verdicts`, `FlightScan`) | 40 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (six segments, titre, tests) | 55 |
| garde AC3 (`crates/mika-agent/src/milestone_manager/no_repo_list_test.rs`) | 45 |
| tests AC1/AC2 (`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`) | 70 |
| fixtures (`milestone_manager/verdict_poster.rs`, `milestone_manager/types.rs`) | 20 |

Total estimé : 420 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 420 est sous la borne de 429, et
sous le seuil par PR de 1000 — la borne par vol est la plus serrée, c'est donc
elle qui mord, et elle n'est pas franchie. Le découpage a été examiné et refusé
sur un argument de testabilité, pas de confort : élargir le périmètre sans
enrichir le rendu livrerait un vol dont AC4 et AC5 sont ouverts et dont le
rapport **affirmerait** un état de PR qu'il n'a pas lu — exactement le défaut que
ce ticket corrige ; et enrichir le rendu sans élargir le périmètre livrerait un
vol dont la section reste vide en production, donc non observable, ce qui est
l'état actuel. Les deux moitiés ne sont pas séparément mergeables avec un sens.

**La marge est de 9 lignes, donc elle est nommée plutôt que supposée.** Le § Ordre
de commit porte un **plancher** : si le travail dépasse la borne en cours de vol,
l'arrêt se fait après le commit 5 (périmètre + verdict + rendu + AC1/AC2/AC5
verts), et la garde AC3 part en suivi — c'est le seul livrable dont l'absence ne
rend aucun autre faux, et le module reste sans liste de dépôts par construction,
gardé ou non.

## Ordre de commit

Chaque commit laisse l'arbre compilable et la suite verte.

1. `refactor(db)` — `list_finished_implement_flights_in_container(since, repos)` :
   prédicat `LIKE`/`ESCAPE`, propriété « aucun dépôt ⇒ aucun vol », tests DB
   existants adaptés (V5).
2. `refactor(flight-source)` — `repo_from_issue_url`, lecture unique de
   `DISPATCHABLE_REPOS`, `repo` sur `FinishedFlight`, trait sans `issue_urls`,
   `cadence.rs` recâblé (R2-R4).
3. `feat(evaluator)` — `PrStateKnowledge`, `repo` sur `FlightVerdict`,
   `recommend`, serde par défaut, fixtures (R5, R6, R9).
4. `feat(reporter)` — six segments, titre, AC4 en golden (R7, V3).
5. `test(manager)` — corpus des 13 vols sur vraie base, contrôle négatif
   claude-pilot#232, AC5 et son contrôle positif (V1, V2).
6. `test(guards)` — garde AC3 avec allowlist vide, contrôle de bonne foi,
   plancher anti-vacuité (R8, V4).

**Plancher :** après le commit 5, les six AC sauf AC3 sont tenus et AC6 est
mesurable. C'est le point d'arrêt si la borne se consomme.

## Sonde post-déploiement (AC6)

Elle se lit **après** `make deploy` — le manager est dans le binaire, donc rien
du code de ce vol n'est en service avant. L'absence des lignes ci-dessous sur un
binaire antérieur signifie « binaire ancien », jamais « aucun vol » (motif
mika#2340).

**Geste.** `make deploy`, puis attendre un cycle du manager et lire le dernier
rapport de `~/.mika/manager/sink`.

**Attendu, au premier cycle :**

1. Le rapport porte une section « Vols terminés depuis le cycle précédent (toute
   la boucle — mode ombre, aucune écriture) », et elle est **non vide**.
2. Au moins une ligne porte une recommandation **classée** — autre que la cause
   non classée `no_outcome_line` — sur un vol qui a ouvert une PR. C'est la
   première preuve en production de #2892 : jusqu'ici l'évaluateur n'avait aucune
   population sur laquelle ce correctif pouvait s'exercer.
3. Dans les logs, `manager_flight_scan` porte `scope = "container"`,
   `repo_count = 4` et `flights_found > 0`.

**Halte 1 — la section est absente.** Lire `manager_flight_scan` **avant** de
toucher au code. `flights_found = 0` avec `repo_count = 4` signifie que la
requête n'a rien trouvé dans la fenêtre du checkpoint : établir qu'un vol
`implement` s'est bien terminé depuis le cycle précédent avant de conclure quoi
que ce soit. Aucune ligne `manager_flight_scan` du tout ⇒ binaire antérieur au
vol : établir le déploiement d'abord.

**Halte 2 — la section est là, mais tous les verdicts sont `no_outcome_line`.**
C'est #2886/#2892 qui a quelque chose à dire, pas ce vol. Vérifier la topologie à
deux lignes (parente `ready-label:` + enfant `long_running:run_claude_pilot`
portant la ligne `Outcome:`) sur un vol précis avant de conclure.

**Halte 3 — un vol claude-pilot ou wizzard apparaît dans la section.** C'est AC2
en échec et l'arbitrage du § Point 1 violé : le périmètre fuit. Ne pas « corriger »
en élargissant la liste — établir par où la fuite passe, puisque le périmètre est
censé être une propriété de `DbFlightSource` et d'un seul `use`.

**Halte 4 — une ligne nomme un numéro de milestone.** Le rapport ne lit aucun
milestone hors cible ; un numéro rendu là est une devinette, exactement ce que le
§ Point 2 refuse. Traiter comme une régression du rendu, pas comme une
amélioration.

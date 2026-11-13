# fix(manager) — la source de vols du manager lit le conteneur, pas l'agent par défaut (mika#2792)

**Ticket :** senara-solutions/mika#2792
**Milestone :** Substrat de boucle (n°33)
**Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
**Liens :** mika#2698 (le compte de concordance attend ce correctif), mika#2704 (le vol qui a câblé la source)

---

## Le problème, remesuré dans le code

Trois faits se composent pour rendre l'évaluateur de vols structurellement inerte :

1. **Le câblage** — `crates/mika-agent/src/server/mod.rs:1759-1764` construit
   `DbFlightSource` à partir de `agents.get(&default_agent)`, donc le handle
   `AsyncDatabase` porte `agent_id = default_agent` (sur gentux : `mika`, voire
   `mika-test`).
2. **L'injection** — `crates/mika-agent/src/async_db.rs:1879-1889` prend
   l'`agent_id` **du handle**, jamais de l'appelant :
   `let id = self.agent_id.clone();`. L'appelant ne peut pas choisir.
3. **Le filtre** — `crates/mika-agent/src/db/tasks.rs:4824` :
   `WHERE agent_id = ?1`.

Or les vols d'implement appartiennent à **`mika-dev`** : c'est lui que le
routage de la gateway désigne pour les événements d'issue
(`crates/mika-gateway/src/github.rs:331-358`, corroboré par
`crates/mika-agent/src/server/ready_label_handler.rs:203-216`). La requête
interroge donc un agent qui ne possède aucun vol, et rend zéro ligne — les 0/0/43
du tableau du ticket.

Et la panne est **invisible par construction sur trois couches successives** :

| couche | site | ce qu'elle avale |
|---|---|---|
| le trait rend `Vec`, pas `Result` | `milestone_manager/evaluator.rs:146-150` | il n'y a pas d'erreur à propager |
| le cycle ne trace rien | `milestone_manager/cadence.rs:567-582` | trois sorties précoces, zéro ligne de log |
| le rapport omet la section vide | `milestone_manager/reporter.rs:220` | 43 rapports sans aucune trace |

C'est la classe « déployé ≠ efficace » : chaque couche est individuellement
défendable, leur composition ne laisse aucun octet distinguant « aucun vol » de
« mauvaise base » (classe mika#2205).

---

## Décision de conception — le scope est le **conteneur**, pas un agent

### La décision

`list_finished_implement_flights` **perd son paramètre `agent_id`** et devient
`list_finished_implement_flights_in_container` — le nom porte l'invariant.
La clause `WHERE agent_id = ?1` disparaît de la requête ; le filtre porteur
reste `reference_url IN (<urls des sous-issues du milestone>)` +
`COALESCE(dispatch_class,'implement') = 'implement'` + la borne `since` +
le filtre de statut terminal en Rust.

### Pourquoi c'est la lecture juste, et non un élargissement d'autorité

- **Le manager n'a pas d'agent.** Le commentaire du site de construction le dit
  lui-même (`server/mod.rs:1753-1755` : « la cadence est une tâche de processus,
  sans agent propre »). Le défaut vient d'avoir **emprunté** un agent par
  commodité pour obtenir un handle, puis d'avoir laissé la requête filtrer
  dessus. On garde l'emprunt du handle — il faut bien une connexion — et on
  retire le filtre.
- **Une seule base.** `init_agent` ouvre `home::container_db_path(global_home)`
  = `~/.mika/data/mika.db` (`crates/mika-agent/src/server/mod.rs:604`,
  `crates/mika-common/src/home.rs:338`). Tous les agents du conteneur partagent
  le même fichier ; le scope par `agent_id` est **purement logique**. Lever le
  filtre ne traverse aucune frontière de stockage.
- **Un conteneur = un client.** La topologie est « one container per customer »
  (CLAUDE.md) : les agents d'un conteneur sont les seats d'un même client, pas
  des locataires distincts.
- **L'autorité ne bouge pas.** Le manager surveille déjà ce milestone : il lit
  ses sous-issues pour composer `milestone_issue_urls`. Lever le filtre agent ne
  lui donne pas accès à autre chose — il fait **coïncider** ce qu'il lit avec ce
  qu'il surveille déjà. Aucune autorité d'écriture ne passe par ce chemin
  (`MIKA_MANAGER_EVALUATE_WRITE` reste le seul interrupteur du vol 2, inchangé).

### Alternatives écartées

| alternative | pourquoi non |
|---|---|
| **A. constante `DISPATCH_AGENT = "mika-dev"`** | centralise un nom en dur sans le supprimer. Le ticket demande explicitement de ne pas coder l'agent en dur. Et le nom resterait faux pour tout déploiement qui nomme ses seats autrement. |
| **B. fan-out sur la map `agents`** | la map peut être **incomplète** : `AppState::resolve_agent` résout des agents paresseusement (`async_db.rs:20-27`). Un `mika-dev` résolu tardivement serait manqué — même panne, plus difficile à voir. Coûte aussi N requêtes par cycle pour un résultat que la base rend en une. |
| **C. `agent_id: Option<&str>` (scope optionnel)** | un seul appelant de production existe. Deux branches SQL dont une morte, c'est du YAGNI, et la branche morte est précisément là où une régression se cacherait. |
| **D. aligner sur `route_event` de la gateway** | `mika-agent` ne dépend pas de `mika-gateway`, et le `match` de routage répond à « quel agent reçoit cet événement », pas à « quel `agent_id` possède ces lignes ». Ce serait une source de vérité par ressemblance. |

### Le test existant qui verrouille le défaut

`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs:209`
(`mika2698_la_lecture_est_cantonnee_a_lagent`) **assert exactement le
comportement qu'on corrige** : il exige qu'un vol de `mika-qa` soit invisible
depuis un handle `mika`. Il n'est pas supprimé en silence : il est **retourné**,
renommé `mika2792_la_lecture_nest_plus_cantonnee_a_lagent`, avec un doc-comment
qui nomme mika#2792 et dit pourquoi l'assertion s'est inversée. Un futur lecteur
de l'historique doit trouver la trace du renversement, pas un trou.

---

## Observabilité — une ligne par cycle, sur les trois branches

`collect_flight_verdicts` (`cadence.rs:567-582`) a trois sorties qui rendent
toutes `Vec::new()` sans un octet de différence. Elles deviennent une **seule
ligne INFO**, émise inconditionnellement.

La composition de la ligne est extraite en **structure pure**, testable sans
harnais de capture de tracing — c'est l'idiome déjà posé par
`verdict_poster.rs:65` (`armed_from_value`, « la logique pure, testable sans
env ») :

```rust
/// Ce qu'un cycle a vu en cherchant des vols. Mesuré, jamais déclaré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlightScan {
    /// `"unwired"` | `"no_milestone_issues"` | `"read"` — les trois sorties.
    pub branch: &'static str,
    pub issue_count: usize,
    pub flights_found: usize,
    pub verdicts: usize,
    /// Les `agent_id` **distincts des vols trouvés**, triés, joints par `,`.
    pub flight_agents: String,
}
```

`collect_flight_verdicts` rend désormais `(Vec<FlightVerdict>, FlightScan)` et
émet, avant de rendre :

```rust
info!(
    target: "mika::milestone_manager",
    event = "manager_flight_scan",
    branch = scan.branch,
    scope = "container",          // mika#2792 — plus d'agent emprunté
    issue_count = scan.issue_count,
    flights_found = scan.flights_found,
    verdicts = scan.verdicts,
    flight_agents = %scan.flight_agents,
    since = since.unwrap_or(""),
    "flight scan"
);
```

**Pourquoi `flight_agents` (mesuré) et non « agent interrogé » (déclaré).** Le
ticket demande « agent interrogé ». Après la décision ci-dessus, cette notion
n'existe plus : c'est précisément ce qu'on supprime. La substitution honnête est
de rendre ce qui est mesurable — les agents **réellement propriétaires** des vols
trouvés. Sur la branche à zéro le champ est vide, et c'est le signal : `scope`
dit ce qui a été interrogé, `flight_agents` dit ce qui a répondu. Une régression
qui re-scoperait la requête rendrait `flights_found = 0` avec `issue_count > 0`
— la signature exacte du défaut d'aujourd'hui, désormais lisible en une ligne.

Cela demande que `FinishedFlight` porte `agent_id`. Le doc-comment du type
(`evaluator.rs:124-127`) justifie sa minceur (« volontairement pas `Task` : le
type complet porte trente-deux colonnes ») — l'ajout est donc motivé dans le
commentaire : une septième donnée, portée **pour l'observabilité**, qu'aucun
chemin de verdict ne lit.

**`reporter.rs` n'est pas touché.** Son commentaire (`reporter.rs:214-216`)
porte une décision argumentée — « pas de ligne "(none)" permanente, qui
apprendrait au lecteur à sauter la section ». Le ticket demande une ligne
*d'observabilité par cycle*, canal log ; la sonde du point 4 est prospective et
porte sur un rapport **qui suit un atterrissage**. Renverser la règle de silence
du rapport serait relitiger une décision hors périmètre.

---

## Livrables, dans l'ordre de commit

Chaque commit compile et laisse la suite verte.

### Commit 1 — `fix(db): la lecture des vols du manager est scopée au conteneur`

- `crates/mika-agent/src/db/tasks.rs:4778-4847` — renommer en
  `list_finished_implement_flights_in_container`, retirer le paramètre
  `agent_id`, retirer `WHERE agent_id = ?1`, redécaler les index de bind
  (`?1` = `since`, `?2..` = variantes d'URL). **Réécrire le doc-comment des
  lignes 4803-4804** : il affirme aujourd'hui « un manager ne voit que les vols
  de son propre agent » — il doit désormais dire que le manager n'a pas d'agent
  et pourquoi le scope conteneur est la lecture juste, en citant mika#2792.
- `crates/mika-agent/src/async_db.rs:1875-1889` — même renommage ; la méthode
  **cesse de lire `self.agent_id`**. Doc-comment : le handle ne sert que de
  connexion, son `agent_id` n'entre pas dans la requête.
- `crates/mika-agent/src/db/tests/vols_termines_du_manager.rs` — adapter le
  helper `lire()` (plus d'agent), retourner le test de la ligne 209 (voir
  Commit 3 pour les tests ajoutés).

### Commit 2 — `fix(manager): la source de vols ne s'appuie plus sur l'agent emprunté`

- `crates/mika-agent/src/flight_source.rs:47-92` — appel de la méthode
  renommée ; `task_to_finished_flight` projette `agent_id: t.agent_id`.
  Mettre à jour le doc-comment de module (lignes 1-28) et le commentaire
  « une base vide est la lecture prévue » (103-105) : le vide reste nominal,
  mais il ne peut plus venir d'un mauvais scope.
- `crates/mika-agent/src/milestone_manager/evaluator.rs:128-137` — champ
  `agent_id: String` sur `FinishedFlight`, avec le commentaire justifiant
  l'exception à la minceur du type. Mettre à jour les fixtures de test du
  fichier (lignes 393, 416, 436, 590, 646).
- `crates/mika-agent/src/server/mod.rs:1751-1764` — **réécrire le commentaire
  du site** : il affirme aujourd'hui que le scope à l'agent par défaut est
  voulu. Il doit dire que `agents.get(&default_agent)` fournit une **connexion**
  et rien d'autre, que la requête ne filtre plus sur l'`agent_id` du handle, et
  citer mika#2792. Le code change peu ; c'est le commentaire qui portait le
  défaut et qui doit porter la correction.

### Commit 3 — `feat(manager): une ligne de scan de vols par cycle, sur les trois branches`

- `crates/mika-agent/src/milestone_manager/cadence.rs:567-582` — `FlightScan`,
  les trois constructeurs, l'émission INFO, signature de
  `collect_flight_verdicts` en `(Vec<FlightVerdict>, FlightScan)` ; appelant
  ligne 672 adapté.
- Tests : voir le contrat ci-dessous.

---

## Verification Contract

| # | vérification | commande |
|---|---|---|
| V1 | la suite complète passe | `cargo test -p mika-agent` |
| V2 | clippy propre | `cargo clippy --workspace --all-targets -- -D warnings` |
| V3 | format | `cargo fmt --check` |
| V4 | le scan d'autorité d'écriture du manager ne rougit pas (rien d'ajouté sous `milestone_manager/` n'est un geste d'écriture ni du SQL) | `cargo test -p mika-agent no_dispatch` |
| V5 | les lints du dépôt | `scripts/check-byte-slices.sh`, `scripts/verify-pipeline.sh origin/main` |
| V6 | la structure des bundles reste cohérente | `make verify-bundled-skills` |

### Tests livrés

**Le contrôle positif que la suite actuelle n'a pas** (point 3 du ticket) :

| test | fichier | ce qu'il exige |
|---|---|---|
| `mika2792_un_vol_seme_sous_un_autre_agent_est_trouve` | `db/tests/vols_termines_du_manager.rs` | seed sous `mika-dev` (avec `register_agent` — `tasks.agent_id` est une FK vers `agents(id)`), lecture **sans** scope, le vol est rendu |
| `mika2792_la_lecture_nest_plus_cantonnee_a_lagent` | idem (remplace `mika2698_la_lecture_est_cantonnee_a_lagent:209`) | deux vols sous deux agents distincts, les **deux** sont rendus ; doc-comment nommant le renversement |
| `mika2792_un_vol_hors_milestone_nest_pas_trouve_meme_sous_un_autre_agent` | idem | **le contrôle négatif** : un vol sur une URL hors de la liste reste invisible, y compris quand il appartient à un autre agent — le filtre qui porte est bien l'URL, pas l'agent |
| `mika2792_la_source_trouve_un_vol_dun_agent_different_du_handle` | `flight_source.rs` | **le chemin de production complet** : handle construit avec `new_with_agent(db, "mika-test")`, vol semé sous `mika-dev`, `DbFlightSource::finished_implement_flights` le rend — c'est le test qui aurait rougi sur mika#2792 |
| `mika2792_la_projection_porte_lagent_du_vol` | `flight_source.rs` | `FinishedFlight.agent_id` est bien l'agent de la ligne lue |

**La ligne d'observabilité** (point 2) :

| test | fichier | ce qu'il exige |
|---|---|---|
| `mika2792_sans_source_cablee_le_scan_dit_unwired` | `cadence.rs` | branche 1 : `branch = "unwired"`, tous les compteurs à zéro |
| `mika2792_un_milestone_sans_sous_ticket_dit_no_milestone_issues` | `cadence.rs` | branche 2 : `issue_count = 0`, distinct de la branche 1 |
| `mika2792_une_lecture_a_zero_vol_porte_le_compte_de_tickets` | `cadence.rs` | **la branche nodale** : `branch = "read"`, `issue_count > 0`, `flights_found = 0`, `flight_agents` vide — c'est la signature qui aurait nommé mika#2792 |
| `mika2792_une_lecture_fructueuse_nomme_les_agents_des_vols` | `cadence.rs` | `flight_agents` porte les agent_id distincts, triés, dédupliqués |
| `mika2792_les_trois_branches_ne_rendent_pas_les_memes_octets` | `cadence.rs` | les trois `FlightScan` sont deux à deux distincts — l'assertion anti-mika#2205 explicite |

Le dernier test est la garde du contrat : tant qu'il est vert, aucune refonte ne
peut refaire converger deux branches vers les mêmes octets.

---

## Fire-Disposition

Le plan livre un détecteur : `mika2792_la_source_trouve_un_vol_dun_agent_different_du_handle`
(et ses frères) dont la fonction primaire est de signaler le défaut de câblage
de mika#2792 — son chemin d'échec est « la source est re-scopée à un agent ».

**Disposition retenue : (a) exception nommée en allowlist — allowlist vide.**

- Le détecteur atterrit **armé et vert**, sans aucun `#[ignore]`, parce que le
  correctif qu'il garde atterrit dans **le même vol**, dans un commit antérieur
  de la même PR (commit 1 et 2 précèdent commit 3).
- L'allowlist est **vide, et c'est une donnée** : il n'existe aucune violation
  préexistante à exempter. Le seul site de production appelant la méthode est
  `flight_source.rs:56` (vérifié par `grep -rn list_finished_implement_flights
  crates/`), et il est corrigé au commit 1.
- L'option (b) « livrer désarmé » est **explicitement refusée** : un détecteur
  désarmé sur ce chemin reproduirait exactement la pathologie du ticket — un
  mécanisme présent, inerte, dont le silence se lit comme un succès.
- L'option (c) ne s'applique pas : rien ici n'appelle un cadrage opérateur.

Le scan structurel existant (`milestone_manager/no_dispatch_test.rs`, allowlist
du geste d'écriture du vol 2) n'est **pas** modifié : rien de ce vol n'ajoute de
SQL sous `milestone_manager/` ni de geste d'écriture. V4 le vérifie.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (signature, SQL, binds, doc) | 30 |
| `crates/mika-agent/src/async_db.rs` (signature, doc) | 12 |
| `crates/mika-agent/src/flight_source.rs` (appel, projection, docs) | 25 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (`FinishedFlight.agent_id` + fixtures) | 20 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (`FlightScan`, émission, 3 branches, appelant) | 75 |
| `crates/mika-agent/src/server/mod.rs` (site + commentaire réécrit) | 20 |
| tests (`crates/mika-agent/src/db/tests/vols_termines_du_manager.rs`) | 70 |
| tests (`crates/mika-agent/src/flight_source.rs`) | 55 |
| tests (`crates/mika-agent/src/milestone_manager/cadence.rs`) | 85 |

Total estimé : 392 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 392 passe sous la borne par vol
(429) et très largement sous le seuil par PR (1000). Le découpage en trois
commits est séquentiel et non parallélisable — le commit 3 teste la signature
que le commit 1 rend possible — donc le découper en vols produirait un vol 1
inerte (la méthode renommée sans appelant corrigé ne change rien en production)
et un vol 2 dont la sonde du ticket dépend. La valeur du ticket — « un rapport
qui suit un atterrissage porte la section » — n'existe qu'une fois les trois
commits atterris. Découper ici livrerait deux PR dont aucune n'est vérifiable
par la sonde demandée.

---

## Definition of Done

- [ ] `list_finished_implement_flights_in_container` ne prend ni ne lit d'`agent_id`, et le SQL n'a plus de clause `agent_id`.
- [ ] `DbFlightSource` trouve les vols de `mika-dev` depuis un handle construit sur un autre agent.
- [ ] `collect_flight_verdicts` émet exactement une ligne `manager_flight_scan` par cycle, sur chacune des trois branches.
- [ ] Les trois doc-comments qui portaient le défaut (`db/tasks.rs:4803-4804`, `server/mod.rs:1751-1758`, `flight_source.rs:103-105`) sont réécrits et citent mika#2792.
- [ ] Le test renversé porte un doc-comment nommant le renversement ; aucun test n'est supprimé en silence.
- [ ] V1–V6 passent.
- [ ] Le corps de PR documente la sonde post-déploiement et la note de non-rattrapage ci-dessous.

## Acceptance criteria

Le ticket n'a pas de section `## Acceptance criteria` ; ces critères sont dérivés
de sa section « Travail attendu » (quatre points) et du Verification Contract.

1. **AC1 — la source interroge là où vivent les vols.** La source de vols n'est
   plus scopée à l'agent par défaut du processus. Aucun nom d'agent dispatcheur
   n'est codé en dur dans le chemin de lecture : le scope est le conteneur, et le
   nom de la méthode le dit (`…_in_container`).
2. **AC2 — une ligne par cycle sur toutes les branches, zéro compris.** Un
   événement `manager_flight_scan` est émis à chaque cycle, y compris quand aucun
   vol n'est trouvé, portant le scope interrogé, le nombre de tickets du
   milestone, le nombre de vols trouvés et les agents propriétaires des vols. Les
   trois branches rendent des octets distincts, et un test l'exige.
3. **AC3 — le contrôle positif inter-agents existe.** Un test sème des vols sous
   un `agent_id` différent de celui du handle passé à la source, et exige qu'ils
   soient trouvés. Il est armé (pas d'`#[ignore]`).
4. **AC4 — le contrôle négatif tient.** Un vol sur un ticket hors du milestone
   n'est pas trouvé, y compris quand il appartient à un autre agent : le filtre
   porteur est bien l'URL d'issue.
5. **AC5 — la sonde post-déploiement est nommée.** Le corps de PR énonce la
   sonde du point 4 du ticket : après `make deploy`, un rapport de
   `~/.mika/manager/sink/` qui suit un atterrissage du milestone porte la section
   « Vols terminés depuis le cycle précédent (mode ombre — aucune écriture) ».
6. **AC6 — aucun geste d'écriture n'est ajouté.** `MIKA_MANAGER_EVALUATE_WRITE`
   reste le seul interrupteur du vol 2 et n'est pas touché ; le scan d'autorité
   d'écriture (`no_dispatch_test.rs`) passe sans nouvelle entrée d'allowlist.

---

## Risques et notes opérationnelles

- **Pas de rattrapage rétroactif, et c'est voulu.** La borne `since` vient de
  `checkpoint.last_observed_at`, qui avance à chaque cycle indépendamment des
  vols trouvés. Après déploiement, les 43 vols déjà passés **ne remonteront
  pas** : seuls les atterrissages suivants apparaîtront. C'est exactement la
  sonde que le ticket énonce (« un rapport qui **suit** un atterrissage »), et
  le compte de concordance de mika#2698 démarre donc à partir du déploiement,
  pas rétroactivement. À dire dans le corps de PR pour que l'attente de Vincent
  soit calée sur la bonne date de départ.
- **Le correctif n'est efficace qu'après `make deploy`.** Le ticket lui-même est
  un cas de « déployé ≠ efficace » ; la vérification ne s'arrête pas au vert de
  CI.
- **Le premier cycle après déploiement peut être bruyant.** S'il n'y a pas de
  checkpoint, la source rend tous les vols terminés du milestone, donc un rapport
  avec beaucoup de verdicts. C'est la lecture honnête de « depuis toujours »
  déjà documentée dans `cadence.rs:560-563`, et le mode ombre garantit qu'aucune
  écriture n'en découle.
- **Renommage de méthode de base.** Un seul appelant de production, vérifié par
  grep ; le compilateur attrape le reste.

## Hors périmètre

- Le mode écriture (`MIKA_MANAGER_EVALUATE_WRITE`) et le `verdict_poster` : non
  touchés. Le compte de concordance de mika#2698 est la suite, pas ce vol.
- La règle de silence du rapport (`reporter.rs:214-226`) : décision argumentée
  en place, non relitigée ici.
- Le routage de la gateway (`mika-gateway/src/github.rs`) et les comparaisons
  littérales `== "mika-dev"` de `server/mod.rs:1916/1939/1970/2006` : hors sujet,
  ce vol supprime le besoin de nommer un agent plutôt que de centraliser ce nom.

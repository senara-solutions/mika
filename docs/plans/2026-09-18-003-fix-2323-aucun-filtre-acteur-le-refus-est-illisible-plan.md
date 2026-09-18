# mika#2323 — Aucun filtre acteur n'existe ; ce qui manque, c'est de savoir quelle porte a refusé

> **Statut :** plan de grooming, contenu seul. Architecte non invoqué (dev-groom `_iterate_groom_loop` s'en charge après sortie).
> **Ticket :** senara-solutions/mika#2323
> **Type :** fix (observabilité + documentation ; aucun changement de politique de filtrage)

---

## 1. Ce que la lecture du code établit, avant toute implémentation

Le ticket pose une question binaire — « filtre acteur **voulu**, ou **bug** ? » — et
propose deux remèdes selon la réponse. **La lecture du substrat réfute la prémisse
commune aux deux branches : il n'existe aucun filtre acteur, nulle part, et il ne peut
pas en exister un côté agent.** Quatre mesures, toutes vérifiables par lecture :

**M1 — Le gateway ne filtre pas par acteur sur `issues.labeled`, et il l'écrit.**
`route_event("issues", Some("labeled"))` rend `Some("mika-dev")` sans aucune condition
sur `sender` (`crates/mika-gateway/src/github.rs:327`). Juste avant le routage, un
commentaire daté pose la décision explicitement
(`github.rs:773-776`) :

> `// Self-event filter: … Loop prevention is guaranteed by the routing table (disjoint`
> `// event types per agent), not by identity filtering. If loop risks emerge (e.g.,`
> `// mika-qa's own check_suite events), filter by sender.login != agent's bot login.`

C'est un filtre **envisagé et non implémenté**, conditionné à un risque qui ne s'est
pas matérialisé. Les trois seuls filtres post-routage existants sont
`is_suppressed_review_request` (`pull_request` uniquement, mika#1655),
`is_webhook_denylisted_skill` (compare le **nom du label** à `WEBHOOK_SKILL_DENYLIST`
= `["dev-groom"]`, jamais l'acteur, #845) et la garde no-diff `synchronize` (#886).
Aucun des trois ne peut écarter un `labeled ready`.

**M2 — L'acteur ne franchit jamais la frontière gateway → agent.**
`format_event_text` rend, pour un `issues.labeled` porteur d'un nom de label
(`github.rs:410-420`) :

```
[GitHub] Issue labeled {label} on {repo}#{n} — {title}\n{url}
```

`event.sender` est bien désérialisé (`GitHubWebhookEvent.sender: Option<GitHubUser>`)
et **n'est jamais émis**. Conséquence structurelle, et c'est elle qui clôt la question
du ticket : **le `ready_label_handler` ne peut pas filtrer sur une identité qu'il ne
reçoit pas.** La réponse à « voulu ou bug ? » n'est ni l'un ni l'autre — le mécanisme
soupçonné n'existe pas.

**M3 — L'absence de `ready_label_engine_dispatched` n'est pas un diagnostic.**
C'est la preuve unique du ticket, et elle est compatible avec au moins quatorze causes
distinctes. Cet événement est émis à la **ligne 787** de
`crates/mika-agent/src/server/ready_label_handler.rs`, tout au bout. En amont :

| # | Sortie | Ligne | Trace laissée |
|---|---|---|---|
| 1 | texte ≠ marqueur → `Passthrough` | 137 | **aucune** |
| 2 | parse `<repo>#<n>` échoué | 150 | `ready_label_parse_failed` (WARN) |
| 3 | 2b dépôt hors allowlist | 205 | `ready_label_repo_not_dispatchable` + audit |
| 4 | 2c pilote vif (mika#2279) | 313 | `ready_label_pilot_in_flight` + audit |
| 5 | 3 aucun jeton GitHub | 334 | `ready_label_no_token` (WARN) |
| 6 | 4 `gh issue view` échoué | 351 | `ready_label_body_fetch_failed` (WARN) |
| 7 | 4b siège de dispatch (mika#2084) | 412 | `ready_label_seat_mismatch` + audit |
| 8 | 4c ticket tenu par l'opérateur (mika#2263) | 478 | `ready_label_operator_held` + audit |
| 9 | 7 pré-création de tâche échouée | 574 | `ready_label_task_create_failed` |
| 10–14 | dégradés : outil absent, non long-running, readiness refusée, callback non créé, handler absent | 626–728 | events dédiés |

Et **quatre pertes en amont qui n'écrivent rien du tout côté agent** : drop-oldest de
la file bornée (mika#1870), 429, circuit breaker du gateway, DLQ `dead`.

**M4 — Deux pistes sont déjà fermées par construction, et une troisième est de premier rang.**
*(a)* La coalescence de la file bornée ne peut pas avaler l'événement : le label `ready`
est explicitement marqué « never coalesce » (`webhook_queue_v2.rs:147`, table exhaustive
compilée). *(b)* Les quatre dépôts de l'orchestrateur sont dans `DISPATCHABLE_REPOS`, donc
la porte 2b ne joue pas sur `senara-solutions/mika`. *(c)* **GitHub n'émet `issues.labeled`
que sur une transition.** `gh issue edit --add-label ready` sur un ticket portant déjà
`ready` est un **no-op silencieux** : aucun webhook, aucune ligne, nulle part. Or le ticket
décrit #2310 comme un « **re-**ready manuel », et le reaper qui « **re-arme** » cinq minutes
plus tard fait un `remove → add` (c'est exactement le geste d'`auto_pull` Phase 2). Cette
seule asymétrie explique la totalité de l'observation #2310 sans aucun défaut côté mika.

**Ce que le ticket a réellement mesuré n'est donc pas un filtre : c'est l'impossibilité
d'attribuer un non-dispatch.** C'est la classe que la maison a déjà dû fermer trois fois —
mika#2131 (« un scan silencieusement inactif se lit exactement comme un scan qui n'a rien
trouvé à faire »), mika#2205, mika#2293 (« un réglage qu'on ne peut pas observer n'est pas
un réglage, c'est un espoir »). La doctrine existe ; elle n'est pas appliquée à ce chemin.

---

## 2. Étape 0 — obligatoire avant toute ligne de code

Cette étape peut rendre inutile une partie du plan. Elle se fait avec les instruments
existants, sur les deux cas du ticket.

**0.1 — Timeline GitHub, le fait qui tranche #2310.**
```bash
gh api repos/senara-solutions/mika/issues/2310/timeline --paginate \
  | jq '.[] | select(.event=="labeled" or .event=="unlabeled")
        | {event, label: .label.name, actor: .actor.login, created_at}'
```
Si **aucun** `labeled(ready)` n'apparaît à 08:24Z par `samidarko`, la prémisse tombe pour
ce cas : le label était déjà posé, GitHub n'a rien émis, et il n'y a jamais eu d'événement
à filtrer. Même requête sur #2315 à 08:41Z.

**0.2 — L'état du ticket au moment du label.** Pour #2315, que le ticket lui-même décrit
comme « parqué » : portait-il `operator-review` ou `blocked` ? Si oui, la porte **4c** l'a
refusé — et ce refus est **correct**. Le ticket se réduit alors entièrement à « le refus
était juste, mais illisible ».
```sql
SELECT tool_name, target_key, reasoning, created_at FROM audit_events
WHERE target_key IN ('senara-solutions/mika#2310','senara-solutions/mika#2315')
ORDER BY created_at DESC;
```

**0.3 — Balayage des quatorze events sur la fenêtre 08:20–08:45Z.**
```bash
grep -E 'ready_label_(parse_failed|repo_not_dispatchable|pilot_in_flight|no_token|body_fetch_failed|seat_mismatch|operator_held|task_create_failed|tool_not_found|tool_not_long_running|dispatch_readiness_failed|callback_create_failed|handler_not_found|engine_dispatched)' \
  $MIKA_SPIRIT_LOG_FILE | jq -c '{event, repo, num}'
```

**0.4 — Pertes en amont.** `grep -E 'webhook_queue_drop_oldest|rate_limit_trip' $MIKA_SPIRIT_LOG_FILE`
et l'état DLQ du gateway (`mika webhook list-dead`).

### Les trois branches de l'étape 0, et ce que chacune décide

| Constat | Lecture | Conséquence sur ce plan |
|---|---|---|
| Une ligne parmi les quatorze est présente pour l'un des deux cas | Le handler **a reçu** l'événement et l'a refusé à une porte nommée | Le comportement est correct ; le défaut est l'**illisibilité**. Axes 1 et 3 seuls, axe 2 facultatif |
| La timeline ne porte aucun `labeled(ready)` humain | GitHub n'a **rien émis** (label déjà posé) | La prémisse tombe. Axe 3 devient le livrable principal (le geste opérateur), axe 1 reste comme filet |
| Timeline porteuse d'un `labeled(ready)` humain **et** aucune des quatorze lignes | L'événement **n'a jamais atteint le handler** | **Halte.** La cause est en amont (file bornée, 429, circuit breaker, DLQ). Ne rien changer dans le handler ; ouvrir le ticket de suivi sur le chemin de livraison |

Aucune de ces branches ne conduit à implémenter un filtre acteur, ni à en retirer un.

---

## 3. Requirements

**R1 — Tout événement `labeled ready` reçu par le handler laisse une ligne, quelle que
soit sa sortie.** Aujourd'hui quatorze sorties, dont une totalement muette, et **aucune
ligne d'entrée** : rien ne distingue « jamais reçu » de « reçu et refusé ». C'est le
défaut que le ticket a mesuré sans pouvoir le nommer.

**R2 — La porte qui a refusé est nommée dans un vocabulaire clos, stable et
interrogeable en SQL.** L'opérateur doit pouvoir répondre à « pourquoi ce ticket n'a-t-il
pas dispatché ? » par une requête, pas par un grep sur dix-neuf gigaoctets.

**R3 — Une nouvelle sortie du handler ne peut pas être ajoutée sans nommer sa porte.**
Garde structurelle par exhaustivité du compilateur. Un test comportemental ne peut pas
voir cette classe de régression : elle ne rendrait aucune décision fausse, elle la
rendrait invisible — et toutes les assertions sur la décision resteraient vertes.

**R4 — L'identité de l'acteur traverse la frontière, à titre strictement informatif.**
Pour que la question du ticket soit *répondable* à l'avenir sans refaire cette enquête.
**Invariant dur : aucune décision de refus ne lit ce champ.** Le brancher sur un prédicat
créerait précisément le filtre acteur que le ticket croyait trouver.

**R5 — Le geste opérateur canonique est documenté.** Poser `ready` sur un ticket qui le
porte déjà n'émet rien. Le geste qui déclenche est `remove → add`.

**R6 — Aucune surface opérateur existante n'est retirée.** Les `tool_name` déjà
documentés dans CLAUDE.md (notamment `ready_label_pilot_in_flight`, dont la requête SQL
est publiée) restent écrits à l'identique. L'attribution s'**ajoute**, elle ne remplace pas.

**R7 — Aucun changement de politique de dispatch.** Aucune porte n'est ajoutée, retirée,
élargie ou resserrée. Le périmètre est l'observabilité et la documentation.

---

## 4. Design

### Axe 1 — Un lecteur unique, une entrée, une sortie nommée

**Forme.** `try_handle_ready_label_dispatch_with_fetcher` devient un **enveloppeur mince**
autour d'un `…_inner` qui rend `(VerdictAction, ReadyLabelGate)`. L'enveloppeur émet
l'entrée puis la sortie. C'est la forme « lecteur unique » que la maison applique déjà
à `grooming_marker` (mika#2158) et `live_pilot` (mika#2279) — et pour la même raison :
quatorze émissions dispersées divergeraient, comme la regex de grooming a divergé pendant
des mois en laissant promotion et routage répondre différemment à la même question.

```rust
/// Porte de sortie du ready-label handler. FORMAT DE FIL : ces valeurs atterrissent
/// dans `audit_events.after_value` et l'opérateur en fait des `GROUP BY`. Deux
/// orthographes d'une même porte couperaient une population en deux sans le dire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadyLabelGate {
    NotAMarker,            // jamais émis : l'enveloppeur sort avant
    ParseFailed,
    RepoNotDispatchable,
    PilotInFlight,
    NoToken,
    BodyFetchFailed,
    SeatMismatch,
    OperatorHeld,
    TaskCreateFailed,
    ToolNotFound,
    ToolNotLongRunning,
    DispatchReadinessFailed,
    CallbackCreateFailed,
    HandlerNotFound,
    Dispatched,
}

impl ReadyLabelGate {
    /// Nom de fil. Un `match` exhaustif, jamais un `_ =>`.
    pub(crate) fn wire_name(self) -> &'static str { /* … */ }
}
```

**Placement de l'entrée.** `ready_label_received` est émis **après** le match du marqueur
et le parse réussi, jamais avant. Avant le marqueur, chaque message du canal `github`
écrirait une ligne — l'observabilité qui journalise tout le monde ne distingue plus
personne (mika#2131 AC7). Le cas « marqueur reconnu, parse échoué » reste couvert par
`ready_label_parse_failed`, qui existe déjà.

**Sortie.** `ready_label_outcome` (INFO), champs `repo`, `num`, `gate`, `action`
(`dispatched` / `handled` / `passthrough`), `actor`, `trace_id`. Plus une ligne
`audit_events` : `tool_name = 'ready_label_outcome'`, `target_key = '<owner/repo>#<n>'`,
`after_value = <wire_name>`, `reasoning` = le détail.

**Pas de déduplication, et c'est raisonné.** mika#2131 a dû dédupliquer parce qu'un tick
d'`auto_pull` classe une centaine de tickets toutes les dix minutes. Ici la population est
un événement `labeled ready` — quelques dizaines par jour au plus, et chacun est un fait
distinct daté qu'on veut compter. Dédupliquer effacerait précisément la mesure que le
ticket réclame (« combien de fois ce ticket a-t-il été déclenché ? »).

**Échec d'écriture non fatal.** Un `log_audit_event` en échec écrit un WARN et ne change
aucune décision de dispatch — même contrat que les quatre portes existantes.

### Axe 2 — L'acteur traverse, sans jamais décider

**Côté gateway.** `format_event_text` ajoute une dernière ligne à la branche
`issues.labeled` :

```
[GitHub] Issue labeled ready on senara-solutions/mika#2323 — titre
https://github.com/…
Labeled by: @samidarko
```

**Compatibilité, vérifiée par lecture :** le préfixe est intact donc
`starts_with(READY_LABEL_DISPATCH_MARKER)` est inchangé ; `parse_ready_label_location`
borne son token au **premier espace** après le marqueur
(`ready_label_handler.rs:820`), donc une ligne ajoutée en fin de texte ne peut pas
l'atteindre. Les tests `test_format_event_text_issue_labeled_*` qui comparent le texte
entier sont à mettre à jour dans le même commit.

**Lecture tolérante côté agent.** Acteur absent, ligne malformée, `sender` nul → `None`,
jamais une erreur, jamais un refus. Un ancien gateway servant un nouvel agent (ou
l'inverse) continue de fonctionner à l'identique.

**L'invariant, et sa garde.** L'acteur est un champ de journal. Aucune porte ne le lit.
Le plan pose un test de scan de source refusant que l'identifiant d'acteur apparaisse
dans le corps des prédicats de refus du handler — la même forme que
`mika2205_periodic_scans_do_not_read_the_pat_field_directly`, et pour la même raison :
la régression ne rendrait aucune décision fausse, elle introduirait une politique que
personne n'a décidée.

**Conditionnalité.** Si l'étape 0 branche (b) — GitHub n'a rien émis — l'axe 2 perd sa
justification première mais garde sa valeur : il rend l'acteur lisible pour la prochaine
enquête. Si l'étape 0 branche (c) — halte — l'axe 2 est **suspendu** avec le reste.

### Axe 3 — Documentation : répondre à la question, et donner le geste

Une sous-section de la racine `CLAUDE.md` (voisine de § *Un `labeled ready` répété sur un
pilote vif est un NO-OP*), portant quatre choses :

1. **La réponse au ticket.** Aucun filtre acteur n'existe ; l'acteur ne traverse pas la
   frontière ; la question « voulu ou bug » est mal posée. Avec le pointeur vers le
   commentaire `github.rs:773-776` qui pose la non-implémentation comme une décision.
2. **Le geste opérateur canonique : `remove → add`.** `--add-label ready` sur un ticket
   qui porte déjà `ready` n'émet aucun webhook. C'est ce que fait déjà `auto_pull` Phase 2 ;
   le code le sait, l'opérateur ne le savait pas.
3. **L'inventaire des quinze sorties** avec, pour chacune, son grep et sa lecture — et la
   phrase qui manquait : *l'absence de `ready_label_engine_dispatched` ne dit rien à elle
   seule.*
4. **Les surfaces SQL** : `SELECT after_value, count(*) … GROUP BY 1` (distribution des
   portes) et `… WHERE target_key = 'senara-solutions/mika#2323'` (réponse directe pour
   un ticket).

---

## 5. Verification contract

**Tests unitaires**
- `mika2323_gate_names_are_a_wire_format` — épingle la valeur exacte de chaque
  `wire_name()`. Modèle : `mika2131_filter_names_are_a_wire_format`.
- `mika2323_every_gate_variant_has_a_wire_name` — `match` exhaustif ; l'ajout d'une
  variante ne compile pas tant qu'elle n'est pas nommée (R3).
- `mika2323_no_gate_predicate_reads_the_actor` — scan de source (R4).
- `mika2323_actor_line_preserves_the_marker_and_the_parse` — sur le texte produit par
  `format_event_text` avec acteur : `starts_with(MARKER)` vrai et
  `parse_ready_label_location` rend `(senara-solutions/mika, 2323)`.
- `mika2323_absent_actor_is_none_never_an_error` — acteur absent → `None`, sortie du
  handler strictement inchangée.
- Un test par porte refusante (2b, 2c, 4b, 4c) asserant que `ready_label_received` **et**
  `ready_label_outcome` sont émis, avec le bon `gate`, et que la ligne d'audit
  historique de la porte est **toujours** écrite (R6).

**Tests d'intégration** (`crates/mika-agent/tests/eval/`, `EvalHarness` + fetcher injecté —
le seam existe déjà et sa raison d'être documentée est exactement celle-ci) : un
`labeled ready` refusé en 4c produit deux lignes et zéro tâche.

**Non-régression** : `cargo test -p mika-agent -p mika-gateway -p mika-common`,
`cargo clippy`, `make verify-bundled-skills`.

**Sonde post-déploiement, 48 h, avec ses haltes**
- `grep ready_label_received $MIKA_SPIRIT_LOG_FILE | jq -c '{repo, num, actor}'` —
  **régime attendu : une ligne par `labeled ready` réellement reçu.** Le premier
  `labeled ready` posé à la main par `samidarko` doit y figurer, avec son acteur.
- `SELECT after_value, count(*) FROM audit_events WHERE tool_name = 'ready_label_outcome' GROUP BY 1 ORDER BY 2 DESC;`
  — la distribution des portes. C'est la mesure que le ticket demandait et qui
  n'existait pas.
- **Halte 1 — l'instrument ne voit rien.** Un `labeled ready` humain posé, confirmé par
  la timeline GitHub, et **aucune** ligne `ready_label_received` : ne pas élargir le
  handler. L'événement n'atteint pas l'agent, la cause est dans le chemin de livraison,
  et c'est ce chemin qu'il faut instrumenter — pas ce prédicat.
- **Halte 2 — un `gate` domine.** Si une porte concentre les refus (typiquement
  `OperatorHeld` ou `PilotInFlight`), le défaut n'est pas dans le handler : c'est la
  porte qui a raison et un producteur en amont qui repose le label. C'est ce producteur
  qu'il faut traiter, et il a son propre ticket.

---

## 6. Fire-Disposition

Trois livrables de §5 sont de classe **détecteur** au sens de mika#1574. Chacun doit
dire ce qui se passe quand il tire sur du code existant — sinon le contrat de résolution
est décidé en urgence, par la personne qui voit le test rouge, au pire moment.

### Détecteur 1 — `mika2323_no_gate_predicate_reads_the_actor` (scan de source, R4 / AC5)

**Disposition : (a) allowlist nommée, à zéro entrée, avec assertion autonettoyante.**

**Pourquoi (a) et pas (b) ni (c).** La population de violations préexistantes est **vide
par construction, et c'est vérifiable avant d'écrire une ligne** : le champ que le
détecteur interdit de lire n'existe pas encore côté agent — c'est l'axe 2 de ce même plan
qui l'introduit. M2 l'établit (`event.sender` est désérialisé côté gateway et n'est jamais
émis ; aucun prédicat côté agent ne peut lire aujourd'hui une identité qu'il ne reçoit
pas). *(b) Land disabled* livrerait un détecteur inerte pendant précisément la fenêtre où
le champ qu'il surveille naît — le seul moment où une lecture accidentelle peut être
introduite. *(c) Halt-and-surface* n'a rien à remonter : il n'y a pas de violation à
arbitrer.

**Forme.**

```rust
/// Prédicats autorisés à lire l'identité d'acteur. DOIT RESTER VIDE.
/// Toute entrée ajoutée ici est une politique de filtrage par identité que
/// personne n'a décidée (R4 ; explicitement hors périmètre, §9) et exige un
/// ticket nommé en second membre.
const ACTOR_READING_PREDICATES_ALLOWED: &[(&str, &str)] = &[]; // (fonction, ticket)
```

**Périmètre du scan, nommé pour que l'échec soit lisible.** Le test lit
`crates/mika-agent/src/server/ready_label_handler.rs`, le découpe par fonction, et refuse
l'apparition de l'identifiant d'acteur dans le corps de toute fonction qui n'est pas un
site d'émission déclaré (`emit_ready_label_received`, `emit_ready_label_outcome`). Même
forme heuristique que `mika2205_periodic_scans_do_not_read_the_pat_field_directly`, qui
scanne le corps de deux fonctions nommées — et pour la même raison : un test comportemental
ne peut pas voir cette classe, puisqu'une lecture d'acteur ne rendrait aucune décision
fausse, elle introduirait une politique.

**Ce qui se passe quand il tire.** Le test rougit et nomme la fonction fautive. **La
résolution par défaut est de retirer la lecture, jamais d'ajouter une entrée.** Ajouter une
entrée revient à décider une politique de filtrage par identité, c'est-à-dire exactement le
mécanisme dont ce ticket établit qu'il n'existe pas et que §9 place hors périmètre ; cela
exige son propre ticket, nommé dans le second membre du couple.

**L'assertion autonettoyante, et pourquoi elle existe à zéro entrée.** Le test refuse
(i) toute lecture d'acteur hors allowlist **et** (ii) toute entrée d'allowlist qui ne
correspond plus à une violation réelle. À zéro entrée, (ii) est vraie sans rien vérifier —
elle est écrite maintenant parce qu'une allowlist qui ne se nettoie pas transforme la
première entrée en permission permanente que plus personne ne relit, et que le moment
d'écrire ce garde-fou est avant qu'il y ait quoi que ce soit à garder.

### Détecteur 2 — `mika2323_every_gate_variant_has_a_wire_name`

**Disposition : sans objet — population préexistante structurellement vide.** Ce n'est pas
un scan mais une exhaustivité de compilation sur `ReadyLabelGate`, un type qui naît dans ce
ticket : il ne peut tirer que sur une variante ajoutée **après** lui. Le mode d'échec est un
défaut de compilation dans la PR qui ajoute la variante, et il s'y résout — en nommant la
porte, ce qui est tout l'objet de R3.

### Détecteur 3 — `mika2323_gate_names_are_a_wire_format`

**Disposition : sans objet à l'introduction ; résolution par défaut = annuler le
changement.** Il épingle des valeurs nées ici, donc zéro violation préexistante. Quand il
tire, il tire sur un **renommage** : ces valeurs atterrissent dans
`audit_events.after_value` et l'opérateur en fait des `GROUP BY` (doctrine mika#2131), donc
un renommage coupe une population en deux sans le dire. Le changer volontairement exige de
dater la rupture dans `CLAUDE.md`, jamais de mettre le test à jour en silence.

*(Les tests « un test par porte refusante » de §5 ne sont pas de classe détecteur : ils
assertent un comportement sur du code que ce ticket écrit, pas un invariant sur du code
existant.)*

---

## 7. Definition of Done

- [ ] Étape 0 exécutée, ses quatre résultats consignés dans le corps de la PR, et la
      branche retenue (a/b/c) nommée explicitement.
- [ ] `ReadyLabelGate` introduit ; `…_with_fetcher` devient un enveloppeur mince autour
      de `…_inner` ; les quinze sorties rendent leur porte.
- [ ] `ready_label_received` (INFO) et `ready_label_outcome` (INFO + `audit_events`) émis.
- [ ] Les quatre lignes d'audit historiques des portes sont inchangées (R6).
- [ ] `format_event_text` porte l'acteur ; lecture tolérante côté agent ; tests du
      gateway mis à jour. *(Suspendu si étape 0 = branche c.)*
- [ ] Les six tests unitaires et le test d'intégration passent.
- [ ] `ACTOR_READING_PREDICATES_ALLOWED` introduite **vide**, avec son assertion
      autonettoyante et le commentaire qui dit pourquoi elle doit le rester (§6).
- [ ] `CLAUDE.md` : sous-section portant la réponse au ticket, le geste `remove → add`,
      l'inventaire des quinze sorties, les deux requêtes SQL.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] Corps de PR : la réponse explicite à la question du ticket (« ni voulu ni bug — le
      mécanisme n'existe pas »), avec les citations de M1 et M2.

---

## 8. Acceptance criteria

*Dérivés des Requirements et du Verification contract — le corps du ticket ne porte pas
de section `## Acceptance criteria`, mais il pose une question à laquelle AC1 répond
formellement.*

- **AC1 — La question du ticket est tranchée et documentée.** `CLAUDE.md` énonce
  qu'aucun filtre acteur n'existe sur `issues.labeled`, cite la non-implémentation
  assumée du gateway (`github.rs:773-776`) et le fait que l'acteur ne traversait pas la
  frontière. Les deux branches du ticket (« si voulu, documenter » / « si bug,
  corriger ») sont explicitement adressées.
- **AC2 — Tout `labeled ready` reçu laisse une ligne.** Pour chacune des quinze sorties
  du handler, y compris celle qui était muette, un `ready_label_received` **et** un
  `ready_label_outcome` sont émis à un niveau effectivement collecté (INFO).
- **AC3 — La porte est nommée et interrogeable.** `ready_label_outcome` porte un champ
  `gate` issu d'un vocabulaire clos, et une ligne `audit_events`
  (`tool_name = 'ready_label_outcome'`, `target_key = '<owner/repo>#<n>'`,
  `after_value = <gate>`) permet de répondre en SQL à « pourquoi ce ticket n'a-t-il pas
  dispatché ? ».
- **AC4 — Une nouvelle sortie ne peut pas rester anonyme.** L'ajout d'une variante de
  `ReadyLabelGate` sans nom de fil ne compile pas ; les valeurs de fil sont épinglées
  par un test.
- **AC5 — L'acteur est lisible et ne décide de rien.** `ready_label_received` porte
  `actor` quand le gateway le fournit ; un acteur absent ou malformé rend `None` et ne
  change aucune sortie ; un test de scan de source refuse la lecture de l'acteur dans un
  prédicat de refus, et son allowlist (`ACTOR_READING_PREDICATES_ALLOWED`) est livrée
  **vide**, avec l'assertion autonettoyante décrite en §6.
- **AC6 — Le geste opérateur est écrit.** `CLAUDE.md` énonce que `--add-label ready` sur
  un ticket portant déjà `ready` n'émet aucun webhook, et que le geste canonique est
  `remove → add`.
- **AC7 — Aucune régression de surface ni de politique.** Les `tool_name` et noms
  d'événements préexistants sont écrits à l'identique ; aucune porte n'est ajoutée,
  retirée, élargie ou resserrée ; la compatibilité `marqueur` / `parse` est prouvée par
  test sur le texte porteur d'acteur.

---

## 9. Hors périmètre, délibérément

- **Le re-arm indû du reaper sur ticket parqué** — mika#2315, que le ticket écarte
  lui-même. Distinct : il dispatche, mais sur les mauvais tickets.
- **Implémenter un filtre acteur.** Aucune boucle d'identité n'a été mesurée ; le
  gateway a posé cette non-implémentation comme une décision conditionnée à un risque
  qui ne s'est pas matérialisé. L'axe 2 rend l'acteur **lisible**, jamais **décisionnel**.
- **Le chemin de livraison en amont** (file bornée, 429, circuit breaker, DLQ). Si
  l'étape 0 aboutit à la branche (c), c'est le ticket de suivi à ouvrir — instrumenter
  le handler ne répare rien d'un événement qui ne l'atteint pas.
- **Un endpoint de dispatch opérateur** (la branche « via un endpoint ? » du ticket).
  Le chemin `remove → add` fonctionne une fois documenté ; ajouter une porte d'entrée
  est une décision produit que personne n'a prise.
- **Les treize autres handlers structuraux** (`ci_failure_handler`, `verdict_handler`,
  `merge_ready_handler`…), qui partagent probablement le même angle mort d'attribution.
  Généraliser demande une mesure par handler ; ce ticket en ferme un, mesuré.

---

## Revision history

- **rev 2 (2026-09-18)** : addressed **F1 (BLOCKING)** en ajoutant la section
  `## Fire-Disposition` (nouveau §6, sections suivantes renumérotées 6→7, 7→8, 8→9),
  requise par le Fire-Disposition Gate (mika#1574) dès qu'un plan livre un détecteur.
  Les trois livrables de classe détecteur de §5 y sont traités séparément plutôt que
  globalement, parce que leur population préexistante n'est pas de même nature :
  - `mika2323_no_gate_predicate_reads_the_actor` (le détecteur visé par F1) →
    **option (a), allowlist nommée à zéro entrée**, avec assertion autonettoyante,
    périmètre de scan nommé, et la règle de résolution écrite (**retirer la lecture**,
    jamais ajouter une entrée — une entrée serait la politique de filtrage par identité
    que §9 place hors périmètre). Le choix de (a) sur (b)/(c) est justifié par M2 : le
    champ surveillé n'existe pas encore côté agent, donc la population préexistante est
    vide *par construction et vérifiable avant implémentation* ; « land disabled »
    livrerait un détecteur inerte pendant la fenêtre exacte où le champ naît.
  - `mika2323_every_gate_variant_has_a_wire_name` → **sans objet** (exhaustivité de
    compilation sur un type né dans ce ticket ; ne peut tirer que sur une variante
    ultérieure, et se résout dans la PR qui l'ajoute).
  - `mika2323_gate_names_are_a_wire_format` → **sans objet à l'introduction**, résolution
    par défaut = annuler le renommage (format de fil, doctrine mika#2131).

  Conséquences hors de la nouvelle section : une ligne de Definition of Done sur la
  livraison de `ACTOR_READING_PREDICATES_ALLOWED` vide, et **AC5 renforcé** (l'allowlist
  vide et l'assertion autonettoyante deviennent exigibles). Aucun AC affaibli, aucun
  requirement modifié, aucun changement de périmètre.

  R2 (conditionnalité de l'axe 2) et R3/R4 (gates PASS) n'appelaient aucune action —
  R2 est explicitement marqué non bloquant et conclut que « le plan reste cohérent malgré
  la conditionnalité ».

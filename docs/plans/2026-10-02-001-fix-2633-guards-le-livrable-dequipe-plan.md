# mika#2633 — Le livrable d'équipe passe la garde testimony-grade

**Ticket :** senara-solutions/mika#2633
**Parent :** mika#2627 (fermé par PR #2630 sur son périmètre)
**Origine :** revue multi-agents de PR #2630, constat #2 (P2, confirmé validateur)
**Décision opérateur (MPC, 2026-10-02) :** disposition d'un livrable refusé = **(c)**
— une re-rédaction à un seul retry, puis une ligne neutre.

---

## 1. Ce que la lecture du code déplace dans le ticket

Trois rectifications, et elles sont le **premier livrable** : sans elles, un
correctif qui satisfait l'AC1 à la lettre laisse trois canaux ouverts sur quatre.

### R1 — `TeamEngine::deliver` n'est PAS le site unique. Il y a QUATRE écrivains du livrable.

L'AC1 nomme `TeamEngine::deliver` comme « un site unique [qui] couvre les deux
chemins ». C'est faux : `deliver()` (`teams/engine.rs:1737`) **produit** un texte,
et c'est `deliver_phase` (`:634`) qui le **pose** sur le run — mais trois autres
sites posent `self.run.deliverable` :

| # | site | provenance du texte | atteint la **personne** ? | atteint le **prochain run** ? |
|---|---|---|---|---|
| 1 | `deliver_phase` `:634` ← `deliver()` | sortie LLM du rédacteur, ou repli workspace (#1128) | **oui** | **oui** |
| 2 | `execute_inner` `:763` | `GateOutcome::Conversational(reply)` — 1ʳᵉ passe, texte de l'orchestrateur | **oui** | **oui** |
| 3 | `execute_inner` `:849` | idem, après le critique | **oui** | **oui** |
| 4 | `apply_delegation_gate` `:1135` | `retry_reply` sur `NoDelegation` | **non** | **oui** |

*Comment « atteint la personne » a été établi :* `build_run_completion_message`
(`notification.rs:29`) ne lit `run.deliverable` que sur l'arm `Completed`. Les
sites 2 et 3 `return Ok(())` en laissant `status == Running`, et `execute()`
(`:502`) le passe à `Completed` — donc leur texte est bien enveloppé par la
notification. Le site 4 pose `FailedNoDelegation`, dont l'arm de notification
porte un **texte fixe composé par le moteur** : son `retry_reply` n'atteint jamais
la personne.

*Comment « atteint le prochain run » a été établi :* `finalize_and_shutdown`
(`:687`) persiste `run.deliverable` en DB, et `prompt.rs:68` / `:190` le resservent
au run suivant (`<context type="history_deliverable">`). **Les quatre sites y
passent, le site 4 compris.**

**Conséquence :** garder dans `deliver()` seul couvre **un** site sur quatre. Le
périmètre du scan B de #2630 le savait déjà à moitié — son doc-comment nomme « la
sortie LLM de l'agent rédacteur (`TeamEngine::deliver`), **la réponse de la porte
conversationnelle**, ou le repli workspace » — mais l'AC1 n'a retenu que le premier.

### R2 — Le site unique juste est le point de POSE, et il doit précéder la persistance.

Le raisonnement de mika#2627 s'applique mot pour mot : *« une proposition écrite
dans `messages` est reservie au tour suivant par la compaction — un refus qui
persisterait quand même laisserait la doctrine violée dans l'historique. »* Ici le
vecteur est `update_team_run` → `history_deliverable`. Donc le refus doit avoir
lieu **avant que `run.deliverable` soit posé**, pas au site de notification.

Garder à la notification (`build_run_completion_message`, appelé par les deux
chemins) aurait l'air de satisfaire « un site, deux chemins » — et laisserait la
proposition en base et dans le contexte du prochain run.

### R3 — L'AC1 nomme `detect_`, et la doctrine maison exige de ne PAS l'appeler nu.

Le scan A de #2630 (`mika2627_le_predicat_na_que_deux_lecteurs_de_production`)
gèle les lecteurs de `detect_testimony_access_proposal` à deux fichiers, avec la
résolution écrite : *« router ce site vers `tools::check_testimony_access_proposal`
… Ne PAS l'ajouter à TESTIMONY_PREDICATE_READERS_ALLOWED — trois compositions,
c'est trois formulations de refus libres de diverger. »*

Or `check_testimony_access_proposal` prend un `&ToolContext` et rend un
`Option<ToolOutput>` : `TeamEngine` n'a ni l'un ni l'autre, et un livrable refusé
ne se répare ni par un renvoi ni par un découpage — sa disposition est
**nécessairement** différente (re-rédaction, puis ligne neutre).

**Le geste juste n'est donc ni l'un ni l'autre :** on **étend le recensement** de
lecteurs de deux à trois — exactement comme #2630 l'a étendu de un à deux, son
doc-comment racontant cette histoire — et on compense la garantie perdue par un
scan neuf sur le **format de fil** de la télémétrie (§ 6). *Un recensement n'est
pas une allowlist : on y ajoute, on n'y exempte pas.*

---

## 2. Ce qui est livré

### U1 — `DeliverableSource` et `commit_deliverable`, site unique de pose

```rust
// teams/engine.rs
enum DeliverableSource {
    /// Site 1 — `deliver_phase`. Un agent rédacteur existe et est nommable.
    Writer { agent_name: String },
    /// Sites 2 et 3 — `GateOutcome::Conversational`. Pas de rédacteur.
    ConversationalGate,
    /// Site 4 — `apply_delegation_gate`, run déjà en échec. Pas de rédacteur.
    NoDelegation,
}

impl TeamEngine {
    /// **Le seul site qui pose `self.run.deliverable`.**
    async fn commit_deliverable(&mut self, text: String, source: DeliverableSource) -> String
}
```

Les quatre sites de R1 y passent. `deliver()` est **inchangé** : il produit, il ne
pose pas.

**La provenance est un `match` exhaustif sans bras `_ =>`** (motif
`prompt::hosting_ground_truth_line`, mika#2290) : un cinquième site de pose ne
compilera pas sans décider de sa disposition.

### U2 — La disposition (c), et son périmètre honnête

| provenance | détection positive ⇒ |
|---|---|
| `Writer` | **une** re-rédaction (nomme la doctrine, modèle du re-prompt 5h) ; re-test ; encore sale, ou erreur ⇒ ligne neutre |
| `ConversationalGate` | ligne neutre directement |
| `NoDelegation` | ligne neutre directement |

**Pourquoi la re-rédaction ne couvre que `Writer`, alors que la décision dit
(c) :** la décision écrit « **l'agent rédacteur** reçoit une demande de
re-rédaction ». Un rédacteur n'existe qu'au site 1 (`deliver()` le résout :
rôle `communicator`/`writer`, sinon l'orchestrateur). Sur les sites 2 et 3 le
texte est une **réponse de décomposition**, et `apply_delegation_gate` y pratique
**déjà** un retry renforcé (`CONVERSATIONAL_REINFORCEMENT`, `:1114`) — en
composer un second y empilerait deux retries sur un même tour. Sur le site 4 le
run est **déjà** `FailedNoDelegation` et le texte n'atteint pas la personne :
dépenser un tour LLM pour re-rédiger l'accompagnement d'un échec est un coût sans
contrepartie.

**Fail-closed, comme la décision le prescrit :** toute erreur pendant la
re-rédaction (timeout `run_agent`, `Err`) ⇒ ligne neutre, **jamais** la
transmission du texte initial.

### U3 — La ligne neutre, un seul registre, et c'est une mesure

`TEAM_DELIVERABLE_WITHHELD` — constante unique, posée comme valeur du livrable (et
non `None`).

*Pourquoi `Some(ligne)` et non `None` :* `notification.rs:40` rend déjà « Team 'X'
completed (no deliverable produced). » sur `None` — ce qui serait **faux** (un
livrable a été produit, il a été retenu) et rendrait le refus indistinguable d'un
run sans livrable.

*Pourquoi un seul registre, contre le motif mika#2290/#2292 :* ces deux tickets
ont livré deux corps parce que `FAMILY_SOUL` interdit le jargon d'infrastructure,
et mika#2292 a écrit le discriminant — *ce que la famille abandonne est la part qui
n'a pas de sens pour quelqu'un sans infrastructure*. **Ici il n'y a rien à
abandonner** : « équipe » et « livrable » sont du français ordinaire, pas du
vocabulaire de substrat. Mesure à l'appui : `run_team` est un **builtin**
conditionné à `agents.len() > 1 || !teams.is_empty()`, et un tenant famille est
mono-agent — la population famille est donc en pratique vide, et le corps est de
toute façon formulé pour être juste dans les deux registres. Conséquence assumée :
pas de `match` sur `PersonaProfile` à ce site, et un futur profil n'aura rien à y
décider (motif mika#1983, qui a dû écrire le même refus).

### U4 — Télémétrie : `channel = "team_deliverable"` (AC4)

`TestimonyProposalChannel` gagne `TeamDeliverable`, `as_wire()` → `"team_deliverable"`.

L'émission porte `trace_id` (champ de `TeamEngine`), `agent_id`
(`team_db.agent_id()`), `session_id` (`format!("team-{}", run.run_id)`, la session
d'équipe que `execute()` crée en `:538`), `channel`, `matched_subject`,
`matched_movement`, plus `team_run_id` et `deliverable_source`.

`guard_correlation_id`, `step` et `label` sont **absents** — il n'y a ni pas de
boucle ni d'étiquette de mode ici, et *un champ qui affirme ce qu'on n'a pas mesuré
est le défaut mika#2304*.

Résidu : `guard.testimony_access_proposal_uncorrected` avec le même `channel`,
émis quand la ligne neutre est servie. **Régime attendu : zéro.** C'est la
population que ce travail ne ferme pas, et sans elle elle serait indistinguable
d'un run sain (motif 5h, mika#1960).

**Le bras `TeamDeliverable` ajouté au `match` de `check_testimony_access_proposal`
est inatteignable, et c'est un précédent du code, pas une négligence :** `C::EndTurn`
y est déjà et l'est déjà — la garde 5h compose son re-prompt dans `agent_loop` et
n'appelle pas ce helper. L'inertie est **nommée sur la variante** (motif
`CreateScheduledTask`, mika#2627 R2) et épinglée par V6.

### U5 — AC3 : `run_team.rs` sort du périmètre et le scan B le voit **gardé**

Le scan B refuse tout consommateur de `ctx.message_sender` sous `tools/` qui
n'appelle pas le helper et n'est pas au périmètre. Population de production
mesurée : **4** fichiers — `send_message.rs` (gardé), `delegate_task.rs`,
`run_team.rs`, `mod.rs` (les trois au périmètre). *(`toggle_skill.rs` et
`pr_merge_with_gate.rs` ne portent `message_sender` que sous `#[cfg(test)]`, hors
population.)*

Retirer `run_team.rs` du périmètre sans plus le ferait compter **non gardé** : le
scan deviendrait rouge. D'où un **terme neuf** :

```rust
/// Un site dont la garde vit en AMONT, avec le fichier qui la porte.
/// Vérifié dans les deux sens : le site amont doit réellement lire le prédicat.
const TESTIMONY_SENDER_COVERED_UPSTREAM: &[(&str, &str)] = &[(
    "crates/mika-agent/src/tools/run_team.rs",
    "crates/mika-agent/src/teams/engine.rs",
)];
```

Le prédicat devient : *gardé sur place, **ou** déclaré couvert en amont par un
fichier qui lit réellement le prédicat*, ou au périmètre. `run_team.rs` **sort**
de `TESTIMONY_SENDER_PERIMETER` — AC3 à la lettre — et l'assertion
auto-nettoyante fait rougir le jour où `teams/engine.rs` cesse de porter la garde
(AC3 « test vu rouge », V5).

`delegate_task.rs` **reste** au périmètre : il est couvert *transitivement* (le
délégué appelle `send_message`), ce qui est un motif différent d'une garde amont
sur le même appel. `mod.rs` reste aussi (il déclare le champ et porte le helper).
Le paragraphe « canal ouvert nommé » du doc-comment, écrit par #2630, est retiré —
c'est l'objet même d'AC3.

`TESTIMONY_GATED_TOOLS` est **inchangé** : il recense les outils appelant le
helper, et `run_team` ne l'appelle pas. L'y ajouter ferait rougir le scan de
parité de #2630.

---

## 3. Ce que ce travail n'achète PAS

- **Il ne rend pas la surface *propose* structurelle.** Le refus lit un texte
  sortant : il **rattrape avant la transmission**, il ne rend pas l'agent incapable
  de formuler la proposition. La doctrine maison est *construis l'incapacité, ne
  promets pas la retenue* (mika#1991) ; elle **n'est pas applicable ici** et il faut
  l'écrire plutôt que le contourner — il n'existe aucune capacité à retirer, le
  livrable est du texte en langue naturelle.
- **Il ne rattrape aucun livrable déjà transmis**, et **rien n'est rétro-estampillé** :
  la sonde est la **prochaine** occurrence.
- **Il ne couvre pas les `run.deliverable` déjà persistés** avant ce déploiement,
  qui continueront d'être resservis en `history_deliverable`. Population bornée
  (10 derniers runs, `load_team_runs_for_prompt`), nommée, non couverte.
- **Il ne ferme pas RK5** (la proposition étalée sur deux phrases dont aucune ne
  porte les deux couches) : le prédicat segmente par phrase, et élargir rouvrirait
  le faux positif que la segmentation existe pour éviter. Hérité de mika#1960,
  ni élargi ni modifié.
- **Il ne touche ni la garde 5h, ni le prédicat, ni le helper outil** — sauf le
  bras de `match` d'U4 et la variante d'enum.
- **Il n'ajoute aucune variable d'environnement, et c'est une décision.** Précédent
  le plus proche : mika#2627, qui n'en a pas non plus, pour la raison qu'il écrit —
  *un désarmement par variable sur un chemin de doctrine serait un désarmement par
  coquille*. Le geste de désarmement est un **revert**.
- **Il n'ajoute aucune ligne `audit_events` et aucun compteur**, en cohérence
  explicite avec 5h et #2630 : la famille #953 est journal-only. Les seuls
  instruments sont les greps du § 5, et **leur silence ne prouve rien tant que
  personne ne les exécute**.

---

## 4. Fire-Disposition

Ce plan livre **trois détecteurs** : le terme neuf du scan B avec son assertion
auto-nettoyante (V5), le scan de format de fil de la télémétrie (V7), et le scan
de pose unique du livrable (V8).

**Option retenue : (a) exception nommée en allowlist — réalisée à ZÉRO exception,
toutes les allowlists livrées vides et armées.**

Détail d'implémentation, et la vérification préalable qui l'autorise :

- **Aucune violation existante.** Établi par mesure avant écriture des scans, et à
  re-établir en V-pré :
  - scan B après modification : population de production = 4 fichiers, dont 1
    gardé sur place, 1 couvert en amont (vérifié), 2 au périmètre ⇒ `unguarded`
    vide ;
  - scan télémétrie : les deux émetteurs existants (`agent_loop/mod.rs` pour 5h,
    `tools/mod.rs` pour les canaux outils) portent déjà un `channel` ⇒ zéro
    infraction ; le troisième est écrit conforme ;
  - scan pose unique : les quatre sites de R1 sont convertis dans le même commit
    ⇒ zéro infraction.
- **Les trois allowlists sont déclarées vides et épinglées vides** par un test
  frère (motif `TESTIMONY_PREDICATE_READERS_ALLOWED` / mika#2323 : *une allowlist
  née vide est un emplacement où déposer la prochaine infraction*).
- **Résolution quand un scan tire : on route le site, on n'ajoute pas de ligne**
  (doctrine mika#2201). Chaque message d'échec porte cette phrase.
- **Chaque scan porte son anti-vacuité** : un scan visant un nom mort se lit
  exactement comme un scan propre (mika#2103 / mika#2205).
- **Chaque scan porte son contrôle de bonne foi** — un second site injecté doit le
  faire rougir — et les trois doivent être **vus rouges** avant d'être déclarés
  verts (V-rouge).

Aucun détecteur n'est livré désarmé (option b) : il n'y a rien à désarmer, aucune
population existante ne les viole. Aucune halte-et-remontée (option c) : la
décision de disposition est tranchée par l'opérateur et portée en tête de ce plan.

---

## 5. Surfaces opérateur

```bash
# 1. Un livrable d'équipe a-t-il été arrêté ?
grep guard.testimony_access_proposal "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.channel == "team_deliverable")
           | {team_run_id, deliverable_source, matched_subject, agent_id}'

# 2. La re-rédaction a-t-elle échoué ? (résidu)
grep guard.testimony_access_proposal_uncorrected "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.channel == "team_deliverable") | {team_run_id, deliverable_source}'

# 3. CONTRÔLE POSITIF — la garde 5h et les canaux outils tournent-ils encore ?
grep guard.testimony_access_proposal "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.channel != "team_deliverable") | .channel' | sort | uniq -c
```

```sql
-- La ligne neutre réellement servie, par run
SELECT run_id, team_name, status, created_at FROM team_runs
 WHERE deliverable = '<TEAM_DELIVERABLE_WITHHELD>' ORDER BY created_at DESC;
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `channel = "team_deliverable"` | WARN | **zéro** | chaque ligne est un livrable arrêté avant la personne **et** avant la base |
| `_uncorrected`, `source = Writer` | WARN | **zéro** | la re-rédaction a échoué : lire le prompt servi au rédacteur **avant** de toucher au prédicat |
| `_uncorrected`, `source = ConversationalGate` \| `NoDelegation` | WARN | **zéro** | nominal par conception : ces provenances n'ont pas de re-rédaction |
| un même `team_run_id` portant plusieurs refus | WARN | **anomalie** | le rédacteur insiste — un seul retry est prévu, donc deux lignes signifient un second site de pose |
| `channel != "team_deliverable"` | — | **non vide** | le contrôle positif : zéro partout ne prouve rien |

**Coût daté, nommé plutôt que découvert :** les lignes antérieures au déploiement
de #2630 ne portent **pas** de champ `channel` et ne sont pas réécrites (motif
mika#2361). Une requête `select(.channel == "end_turn")` qui enjambe ce
déploiement-là rend vide ; la requête juste de part en part est
`select(.channel == null or .channel == "end_turn")`. Ce travail n'ajoute aucune
nouvelle borne de ce genre : il ajoute une **valeur** au champ existant.

---

## 6. Contrat de vérification

### V-pré — avant d'écrire les scans (condition de la Fire-Disposition)

Établir les trois populations à zéro violation (§ 4). Si l'une est non vide,
**halte** : la Fire-Disposition change d'option et le plan doit être révisé.

### V-rouge — chaque détecteur est vu rouge avant d'être vert

Les trois scans (V5, V7, V8) sont exercés contre une infraction injectée **avant**
d'être déclarés verts. *Un détecteur qui démarre vert n'a jamais montré qu'il
voit.*

### V1 — Le recensement de R1 est épinglé

`teams::engine::tests::mika2633_les_quatre_sites_de_pose_passent_par_le_commit` —
scan de source : aucune affectation de `self.run.deliverable` en production hors
de `commit_deliverable`. C'est V8, et c'est la garantie que R1 ne se rejoue pas.

### V2 — Le défaut fondateur, sur le chemin nominal

`tests/eval/` : un run d'équipe dont le rédacteur produit une proposition d'accès
testimony-grade ⇒ une ligne `channel = "team_deliverable"`, une re-rédaction
tentée, et le livrable final propre. **Aucune** proposition dans `team_runs.deliverable`.

### V3 — Les trois autres provenances (le cœur de R1)

Trois cas, chacun **vu rouge** sans son site converti : porte conversationnelle
1ʳᵉ passe, porte conversationnelle après critique, `NoDelegation`. Pour chacun :
ligne neutre posée, `_uncorrected` émis, zéro proposition en base.

*Trois cas et non un :* une conjonction de sites ne se prouve pas en en
convertissant un seul — leçon mika#2277, qui a dû livrer quatre contrôles négatifs
à terme unique pour la même raison.

### V4 — AC5 : les trois formulations de refus prescrites restent délivrées

Contrôle négatif, modèle V3 de #2630 : les trois refus que la Layer 1 de
mika#1798 **prescrit** (décliner en nommant la doctrine, décliner en offrant un
substitut operational-grade, admettre l'incapacité) passent le livrable **sans**
refus, sur les quatre provenances. **Porteur** : sans lui, « la garde décide » est
indistinguable de « la garde bloque tout », et le mécanisme mika#1798 pourrait
être cassé avec tous les tests au vert.

### V5 — AC3, dans les deux sens

`run_team.rs` absent de `TESTIMONY_SENDER_PERIMETER` et vu **gardé** ; le scan
rougit si `teams/engine.rs` cesse de lire le prédicat (assertion auto-nettoyante) ;
le scan rougit si une entrée de `COVERED_UPSTREAM` nomme un fichier amont
inexistant.

### V6 — L'inertie du bras `TeamDeliverable` est épinglée

`teams/` n'appelle pas `check_testimony_access_proposal` (scan de source). Sans
cette assertion, l'inertie nommée en U4 serait à vérifier à la main à chaque
relecture.

### V7 — Le format de fil de la télémétrie

Tout émetteur de `guard.testimony_access_proposal` en production porte un champ
`channel`. C'est ce qui remplace la garantie que le scan A donnait par accident, et
c'est ce qui rend `jq 'select(.channel == …)'` exact plutôt qu'un filtre sur lequel
deux sites peuvent diverger.

### V8 — Le scan A étendu à trois lecteurs

`TESTIMONY_PREDICATE_READERS` passe de 2 à 3 (ajout de
`crates/mika-agent/src/teams/engine.rs`), **allowlist toujours vide**, et son
doc-comment est réécrit pour dire *pourquoi* le troisième lecteur est légitime
(disposition nécessairement différente) et *ce qui* reste partagé (le vocabulaire
de canal, V7).

### Ce qui n'est PAS testable ici, écrit plutôt que découvert

Qu'un vrai modèle **obéisse** à la demande de re-rédaction. Le contrat côté mika
est *le prédicat refuse, la re-rédaction est demandée une fois, le repli est la
ligne neutre* — déterministe, et c'est ce que V2/V3 attestent. La moitié
comportementale est la sonde S1 du § 7.

---

## 7. Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** Ces sondes décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif avant toute conclusion
> (classe mika#2340). Ce sont des **gestes d'opérateur** sur l'hôte : la base n'est
> pas montée dans le bac à sable de dispatch.

**S1 — le défaut fondateur ne se rejoue pas** (premier run d'équipe dont le
livrable tente). Attendu : une ligne `channel = "team_deliverable"`, et **aucune**
proposition reçue par la personne ni écrite dans `team_runs.deliverable`.
*Halte 1 — aucune ligne alors qu'une proposition est partie :* **ne pas élargir le
prédicat par réflexe.** Lire d'abord le contrôle positif (grep 3) : zéro ligne des
deux côtés ne prouve rien — *une garde que personne n'a exercée se lit exactement
comme une garde qui marche* (mika#2205). Puis établir **par quel site** le texte a
été posé : si c'est un cinquième, V8 aurait dû l'empêcher de compiler, et c'est V8
qu'il faut lire.

**S2 — la re-rédaction aboutit (30 jours).** `_uncorrected` avec
`source = Writer` reste vide.
*Halte 2 — non vide :* le rédacteur ne se corrige pas. **Ne pas ajouter un second
retry** — la famille #953 tient un budget d'un coup, délibérément, et un second
serait la boucle qu'elle existe pour éviter. Le levier est la **formulation** de la
demande de re-rédaction, et c'est un ticket sur le corps, pas sur une détection.

**S3 — contrôle négatif de bruit (7 jours).** Aucun refus sur un livrable
ordinaire, et en particulier aucun sur un livrable qui **décline** un accès (la
population V4).
*Halte 3 — une occurrence :* faux positif, et son coût change de nature par
rapport à #2630 — là c'était un message qui ne partait pas, ici c'est **un run
d'équipe entier dont le livrable est jeté**. **Désarmer d'abord** (revert de
l'appel au prédicat dans `commit_deliverable`), diagnostiquer ensuite.

**S4 — la population hors périmètre.** La requête SQL du § 5, une fois, plus un
`SELECT deliverable FROM team_runs ORDER BY created_at DESC LIMIT 10` pour lire ce
que `history_deliverable` ressert.
*Halte 4 — une proposition y figure :* ce sont les livrables pré-déploiement (§ 3).
Le remède est un geste d'opérateur sur la base, **pas** un élargissement de la
garde à la lecture.

**Halte transverse — les deux sondes muettes.** Zéro refus **et** zéro run
d'équipe ne prouve rien : il faut qu'un run ait tourné depuis le déploiement.
Vérifier `SELECT count(*) FROM team_runs WHERE created_at > '<déploiement>'` avant
toute conclusion.

---

## 8. Hors périmètre, délibérément

- **La garde 5h, son prédicat, son re-prompt, son budget d'un coup** — seule
  l'enum de canal gagne une variante.
- **Le helper outil `check_testimony_access_proposal`** — seul son `match` gagne un
  bras, nommé inerte.
- **`TESTIMONY_GATED_TOOLS`** — inchangé (`run_team` n'appelle pas le helper).
- **`delegate_task.rs`** — reste au périmètre, couvert transitivement ; motif
  différent d'une garde amont.
- **RK5** (proposition étalée sur deux phrases) — hérité de mika#1960, non élargi.
- **Les livrables déjà persistés** — § 3, geste d'opérateur.
- **Les canaux non-`message_sender`** : un outil qui atteindrait la personne sans
  passer par `ctx.message_sender` ni par un `action_type` planifiable. Aucun
  n'existe aujourd'hui, et armer un détecteur sur une population vide est ce que
  mika#2520 refuse.
- **`TeamEvent::Deliverable`** (le callback dashboard des sites 1/2/3) — il reçoit
  désormais le texte **après** `commit_deliverable`, donc il est couvert par
  construction ; aucune garde propre n'y est ajoutée.

---

## Definition of Done

- [ ] `commit_deliverable` est le seul site de pose de `run.deliverable`, les
      quatre sites de R1 y passent, `match` exhaustif sur `DeliverableSource`.
- [ ] Disposition (c) : re-rédaction à un seul retry sur `Writer`, ligne neutre sur
      les trois provenances, fail-closed sur erreur.
- [ ] `TestimonyProposalChannel::TeamDeliverable` + `as_wire()` = `"team_deliverable"`,
      télémétrie émise avec son résidu `_uncorrected`.
- [ ] `run_team.rs` hors de `TESTIMONY_SENDER_PERIMETER`, déclaré dans
      `TESTIMONY_SENDER_COVERED_UPSTREAM`, vu gardé par le scan B.
- [ ] `TESTIMONY_PREDICATE_READERS` à trois entrées, allowlist vide, doc-comment
      réécrit.
- [ ] V-pré établie, V-rouge exercée sur les trois scans, V1–V8 vertes.
- [ ] `cargo test -p mika-agent` vert, `cargo clippy` sans avertissement neuf,
      `cargo fmt` appliqué.
- [ ] Les tests existants de #2630 (`mika2627_*`) restent verts, y compris
      `mika2627_le_refus_ne_pose_aucun_verdict` et
      `mika2627_le_refus_doutil_ne_consomme_pas_le_budget_de_5h`.
- [ ] `crates/mika-agent/CLAUDE.md` § 5h-bis documente le canal livrable ;
      `CLAUDE.md` racine § *La surface « propose » est fermée* voit sa ligne
      `run_team` passer de « canal ouvert nommé » à couvert, et sa section
      *Ce que ce travail n'achète PAS* perdre la puce `run_team`.
- [ ] `## Acceptance criteria` ci-dessous satisfaits.

---

## Acceptance criteria

Transcrits du corps de mika#2633, avec la rectification de l'AC1 établie en § 1.

- [ ] **AC1.** Un site unique couvre les deux chemins — synchrone
      (`tools/run_team.rs`) et asynchrone (`task_engine::dispatcher`). Le livrable
      passe le prédicat avant la notification **et avant la persistance**.
      *Rectifié (R1/R2/R3) :* le site est `TeamEngine::commit_deliverable`, le
      point de **pose**, et non `TeamEngine::deliver`, qui ne couvre qu'un des
      quatre écrivains du livrable ; le prédicat est atteint via un troisième
      lecteur déclaré, non via une seconde composition.
- [ ] **AC2.** La disposition d'un livrable refusé est écrite dans ce plan (§ 2
      U2) et testée (V2, V3).
- [ ] **AC3.** `run_team` sort de `TESTIMONY_SENDER_PERIMETER` ; le scan B le voit
      **gardé**. Test vu rouge (V5, V-rouge).
- [ ] **AC4.** Télémétrie `guard.testimony_access_proposal` avec
      `channel = "team_deliverable"`, plus son résidu `_uncorrected`.
- [ ] **AC5.** Les trois formulations de refus prescrites restent délivrées
      (contrôle négatif V4, modèle V3 de #2630).

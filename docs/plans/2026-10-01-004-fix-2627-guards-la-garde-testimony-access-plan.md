# mika#2627 — La garde `testimony_access_proposal` lit aussi le canal des tours silencieux

> **Parent :** mika#1960 (umbrella), lui-même sous mika#1798 (doctrine non-transit).
> **Origine :** la revue multi-agents de PR #2625 (mika#1960 phase 2).
> **Condition d'exécution du ticket :** PR #2625 mergée. **Satisfaite** — `de7161ae`
> (`test(mika#1960): phase 2 … (#2625)`) est le HEAD de la branche de base.

---

## Périmètre — ce que la lecture du code déplace dans le ticket

Le ticket est juste sur son diagnostic et incomplet sur son périmètre. Quatre
lectures, et chacune change ce qu'il faut écrire.

### R1 — Le contournement est réel, et le code le nomme déjà

`crates/mika-agent/src/agent_loop/mod.rs`, en tête du bloc 5h, porte le texte mot
pour mot :

> *In silent mode (heartbeat, callback, reminder) it is delivered to nobody: the
> user channel there is the `send_message` tool's input, which this guard never
> sees […] A proposal carried by `send_message` is therefore an uncovered bypass
> of the propose surface, named here rather than claimed closed (closing it means
> reading the tool input pre-hoc, the mika#933 shape, which is a distinct
> change).*

Ce ticket **est** ce *distinct change*, nommé par la phase 2 elle-même. Rien à
établir sur l'existence du trou.

### R2 — Le recensement d'AC1 trouve TROIS outils, pas un — et le ticket demande de les couvrir

AC1 écrit : « `send_message` (**et tout autre outil qui émet du texte vers
l'utilisateur** ; recensement exhaustif dans la PR) ». Le recensement est donc une
obligation et la couverture en est la conséquence. Résultat du relevé
(`grep -rn "message_sender" crates/mika-agent/src/tools/` plus le relevé des
`action_type` planifiables) :

| # | outil | comment le texte atteint l'utilisateur | dans le périmètre ? |
|---|---|---|---|
| 1 | `send_message` | `ctx.message_sender.send(&cleaned)` — immédiat | **oui**, le vecteur du ticket |
| 2 | `create_reminder` avec `action_type = "send_message"` | `action_config = {"text": message}`, tiré plus tard par `dispatcher.rs:663` | **oui** |
| 3 | `create_scheduled_task` avec `action_type = "send_message"` | idem, `action_config` fourni par le modèle | **oui** |
| 4 | `delegate_task` | passe le `message_sender` au délégué, n'envoie **rien** lui-même | non — le délégué appelle `send_message`, couvert **transitivement** |
| 5 | `run_team` | notification de fin de run, texte composé par `teams::notification::build_run_completion_message` | non — le texte est du **moteur**, pas du modèle |
| 6 | `task_engine::dispatcher` (`:663`, `:1520`, `:2272`) | **consommateur** du différé et des notifications moteur | non — voir R2-bis |

**R2-bis — pourquoi la garde est à la CRÉATION et non au tir.** Le dispatcher qui
tire un `action_type = 'send_message'` planifié n'a **aucun modèle à qui rendre une
raison** : il devrait soit supprimer silencieusement (un rappel que l'utilisateur a
demandé disparaît sans un mot), soit laisser passer. Le remède que le ticket nomme
est « la raison est rendue au modèle », ce qui n'existe qu'au site de création.
Conséquence **nommée** : les rows créées **avant** le déploiement ne sont pas
couvertes. Population bornée et mesurable
(`SELECT count(*) FROM tasks WHERE action_type = 'send_message' AND status IN ('pending','recurring_active')`),
et il faudrait qu'un rappel existant porte déjà une proposition d'accès
testimony-grade pour qu'elle soit non vide.

**R2-ter — le discriminant `action_type` est porteur, pas décoratif.**
`create_reminder` accepte `action_type ∈ {send_message, resume_agent}` et
`create_scheduled_task` `∈ {send_message, run_skill, inject_context, resume_agent}`.
Sur `resume_agent`, le `message` est une **instruction à l'agent**, pas un texte
vers l'utilisateur : « rappelle-moi de vérifier si j'ai ouvert l'accès à ma
messagerie » est une note à soi-même, pas une proposition. Appliquer le prédicat là
serait un faux positif sur une population légitime. Seul `send_message` émet vers
l'utilisateur, et c'est ce que la garde teste.

### R3 — La V4 de la phase 2 s'inverse une seconde fois, et la doc de `guards.rs` devient fausse

Le doc-comment de `detect_testimony_access_proposal` affirme aujourd'hui :

> *V4 now requires **exactly one** production wiring site — the 5h block of
> `run_loop` […] a call anywhere else is the double wiring RK6 names.*

Après ce ticket il y en a **deux** : 5h (texte final) et le helper d'outil (entrée
d'outil). La V4 de la phase 2 était une vérification **par `grep`**, pas un test
automatisé — donc rien ne rougit, et c'est précisément pourquoi la doc doit être
corrigée explicitement : laissée en place, elle prescrirait de supprimer le second
site comme un doublon. Le remplaçant est le **scan A** du § Fire-Disposition, qui
fige la population à deux sites nommés.

### R4 — Un refus d'outil ne doit poser AUCUN `DeliveryVerdict`, et c'est le point le plus subtil

`send_message` porte six sorties, dont cinq posent un `DeliveryVerdict`
(mika#2136). La garde 6f `unacknowledged_send_failure` lit ces verdicts et
**refuse un EndTurn** qui se ferme sur un envoi non réparé. Son prédicat compte
`RefusedTooLong` comme une non-livraison à réparer *par un découpage*.

Un refus doctrinal n'est réparable ni par un renvoi ni par un découpage :
**découper un texte qui propose un accès Gmail produit quatre messages qui le
proposent**. Poser un verdict ici ferait donc re-prompter le tour pour un envoi que
la doctrine refuse, en lui suggérant la réparation exactement inverse de celle qui
convient.

Le refus suit donc les deux sorties que `send_message` a déjà pour « rien n'a été
tenté » (`'text' is required`, `empty-after-strip`), dont le test
`mika2136_les_sorties_sans_tentative_ne_posent_rien` écrit la raison : *« Rien n'a
été tenté, donc il n'y a pas d'échec que le tour doit reconnaître — et un
enregistrement ici rendrait `failed_count` menteur. »* `ToolOutput::error` nu,
`delivery: None`, **épinglé par un test**.

---

## Requirements

- Le prédicat `detect_testimony_access_proposal` (phase 1) est appliqué au **corps
  sortant** de chacun des trois outils du relevé R2, **avant** l'envoi ou la
  création de la row.
- Le refus nomme la doctrine, dit que rien n'a été envoyé, et offre les **deux**
  sorties correctes que Layer 1 prescrit — sans nommer de contournement.
- Les formulations de refus que Layer 1 prescrit ne sont **jamais** refusées, sur
  le chemin outil comme sur le texte final.
- La télémétrie distingue les populations par canal, sans fusionner deux causes ni
  fabriquer un champ non mesuré.
- Aucune valeur de réglage ne bouge, aucune migration, aucune variable
  d'environnement neuve, et la garde 5h n'est pas touchée hors l'ajout du champ de
  canal.

---

## Deliverables

### D1 — Un lecteur unique du prédicat côté outils (`tools/mod.rs`)

```rust
pub(crate) fn check_testimony_access_proposal(
    ctx: &ToolContext<'_>,
    text: &str,
    channel: TestimonyProposalChannel,
) -> Option<ToolOutput>
```

`Some(ToolOutput::error(…))` quand le texte propose d'ouvrir un accès
testimony-grade, `None` sinon. **Motif exact** : `check_reflection_evidence`
(`tools/mod.rs:650`, mika#1952) — même signature, même place, même constante
miroir, même test de parité bidirectionnel.

Il vit dans `tools/mod.rs` et non dans `evidence::guards` parce que `guards` ne
connaît pas `ToolOutput` : le prédicat reste pur et c'est la **composition** refus
+ télémétrie qui est partagée. Trois appelants, **une** composition.

### D2 — Le canal est un format de fil, à site unique

Dans `evidence::guards`, à côté de `TESTIMONY_ACCESS_PROPOSAL_LABEL` :

```rust
pub(crate) enum TestimonyProposalChannel { EndTurn, SendMessage, CreateReminder, CreateScheduledTask }
impl TestimonyProposalChannel { pub(crate) fn as_wire(&self) -> &'static str { /* match exhaustif, AUCUN bras `_ =>` */ } }
```

Quatre valeurs — `end_turn`, `send_message`, `create_reminder`,
`create_scheduled_task` — et le nom de l'outil **est** le canal. Les deux valeurs
que le ticket nomme littéralement sont servies telles quelles ; les deux autres
suivent la même règle, donc l'ajout d'un cinquième site ne change pas le
vocabulaire des quatre premières.

`match` exhaustif sans bras joker (motif `hosting_ground_truth_line`, mika#2290) :
le compilateur, pas un relecteur, force un futur canal à décider de son nom de fil.
Valeurs figées par test (motif `mika2498_les_valeurs_daudit_sont_un_format_de_fil`) :
deux orthographes d'un canal couperaient une population sans le dire.

**Alternative écartée** : `channel ∈ {end_turn, tool_input}` plus un second champ
`tool`. Plus régulier en principe, mais le ticket écrit `send_message` comme valeur
de canal, et un `GROUP BY channel` donne directement la population par site. Une
agrégation « tous les canaux d'outil » reste à un `jq` de l'opérateur
(`select(.channel != "end_turn")`).

### D3 — La place du refus dans `send_message::execute`, et son ordre

| # | sortie | changement |
|---|---|---|
| 1 | `text.is_empty()` → `error` | inchangé |
| 2 | `cleaned` composé (`strip_internal_tags` + `normalize_typography_for_persona`) | inchangé |
| 3 | `cleaned.is_empty()` → `success` | inchangé |
| 4 | **garde doctrinale** → `error`, `delivery: None` | **NOUVEAU** |
| 5 | garde de longueur (mika#2134) → `delivery(RefusedTooLong)` | inchangé |
| 6 | `save_message_with_task_context` | inchangé |
| 7 | les quatre arms d'envoi | inchangés |

**Trois propriétés de ce placement, chacune portante :**

*(a) Sur `cleaned`, jamais sur le brut.* `cleaned` est ce qui partirait réellement ;
lire le brut ferait lire au prédicat le contenu des tags internes, c'est-à-dire du
bruit que le destinataire ne verra jamais.

*(b) Avant la persistance.* C'est le raisonnement déjà écrit pour la garde de
longueur (*« This runs before persistence so a message we refuse to send never
enters conversation history »*) et il est **plus** fort ici : une proposition
persistée dans `messages` est reservie au tour suivant par la compaction, donc un
refus qui persiste quand même laisse la doctrine violée dans l'historique. C'est
exactement la demi-propriété que 5h conserve en mode silencieux (*« In silent mode
5h still keeps the proposal out of the compacted history »*) et qu'il faut tenir
ici aussi.

*(c) Avant la garde de longueur.* Un texte à la fois trop long et porteur d'une
proposition doit être refusé **par la doctrine** : le remède de la garde de
longueur est un découpage, et découper une proposition la multiplie. Épinglé par un
test sur un texte de 12 000 caractères portant la proposition : attendu, le refus
doctrinal, `delivery: None`.

### D4 — Les deux sites de planification

`create_reminder` et `create_scheduled_task` appellent le même helper, sous la
condition `action_type == action_type::SEND_MESSAGE` (R2-ter), sur :

- `create_reminder` : le champ `message` (déjà en main, `action_config` en est
  dérivé par `json!({"text": message})`) ;
- `create_scheduled_task` : `action_config["text"]`, lu **après** la validation JSON
  existante (ligne 128), donc sur une valeur déjà parseable. Une clé `text` absente
  ou non-chaîne ⇒ rien à tester, `None`, comportement inchangé.

Le refus arrive **avant** la création de la row, comme les refus de validation qui
le précèdent.

### D5 — Télémétrie (AC4)

Un nom, `guard.testimony_access_proposal`, plus un champ `channel`. C'est la lettre
d'AC4, et c'est le motif maison `ready_label_outcome` (mika#2323) : *un seul nom,
l'issue dans un champ*, parce que les quatre populations partagent **la même cause**
(une proposition d'accès testimony-grade) et appellent **la même conduite
opérateur** (« une proposition a été arrêtée »). Le motif à deux noms
(`phantom_aged_out` / `phantom_sweep_spared`, mika#2156) s'applique quand chaque nom
porte sa propre cause, ce qui n'est pas le cas ici.

**Trois champs sont ABSENTS sur les canaux d'outil, et jamais fabriqués :**
`guard_correlation_id` (il n'y a pas de re-prompt, donc aucun
`guard.correction_accepted` à joindre), `step` et `label` (le site d'outil ne
connaît ni l'un ni l'autre). Un champ qui affirme ce qu'on n'a pas mesuré est le
défaut de mika#2304, et `null` n'est jamais une valeur (mika#2331).

**Aucune ligne `audit_events`**, en cohérence explicite avec 5h, qui a tranché :
*« the #953 family is journal-only […] inventing an SQL surface no operator query
needs today would be a second population to keep in agreement »*. La population du
refus reste comptable en SQL sans surface neuve, parce qu'un `ToolOutput::error`
atterrit dans `tool_calls.output` (persisté et scrubé) :

```sql
SELECT tool_name, count(*) FROM tool_calls
 WHERE output LIKE 'REFUS (testimony-access, mika#2627)%' GROUP BY 1;
```

Le préfixe du corps de refus est donc **un format de fil lui aussi**, à constante
unique (motif `REFUSED (cwd-guard, mika#2536)`), et la requête ci-dessus est ce qui
justifie de ne pas créer de table.

**Coût daté, nommé plutôt que découvert :** un
`grep guard.testimony_access_proposal | jq 'select(.channel == "end_turn")'` qui
enjambe le déploiement rend **vide** sur les lignes antérieures, où le champ
n'existe pas. Les lignes historiques ne sont pas réécrites (motif mika#2361 : les
réécrire rendrait faux ce qu'elles ont dit quand elles ont été écrites). La requête
juste de part en part est `jq 'select(.channel == null or .channel == "end_turn")'`.

### D6 — Le corps du refus

Il réutilise la **substance** du texte de correction de 5h (la doctrine, le HARD NO
sur le *proposer*, l'absence d'override) et son **arbitrage à deux branches** :
refuser en nommant pourquoi, **ou** refuser et offrir un substitut
operational-grade. La seconde branche est portante : Layer 1 prescrit l'offre de
substitut, donc un refus qui pousserait uniquement vers le refus sec dégraderait ce
que mika#1798 a livré — c'est le RK3 de la phase 2, et il s'applique mot pour mot
ici.

Il dit en outre ce que la garde 5h n'a pas à dire, parce qu'ici l'outil **a
échoué** : *rien n'a été envoyé, la personne n'a rien reçu* — la formule que
mika#2136 a établie pour toute sortie non livrée, sans laquelle le tour pourrait
annoncer un envoi qui n'a pas eu lieu.

**Il ne nomme aucun contournement.** Un refus qui donne le gabarit est une fuite
avec une étape de plus (doctrine mika#2520, mika#2292).

---

## Fire-Disposition

Ce plan livre **cinq** détecteurs. Disposition retenue pour les cinq : **(a)
exception nommée en allowlist**, avec les allowlists **livrées vides** sauf un
périmètre à deux entrées déclarées. Aucun détecteur n'est livré désarmé, et la
raison est mesurée plutôt que prudentielle : il n'existe **aucune violation
existante à exempter** — la population de production du prédicat est vide (régime
attendu de 5h : zéro ligne), donc une allowlist non vide serait un tiroir où
déposer la prochaine infraction (mika#2323), et un `#[ignore]` serait une garde de
doctrine désarmée sur un HARD NO.

| # | détecteur | disposition | détail |
|---|---|---|---|
| 1 | **La garde de refus** (runtime, 3 sites) | **armée**, sans exception | aucune population de production à exempter. Levier de désarmement : un revert — pas de variable d'environnement, par décision (§ Ce que ce travail n'achète PAS) |
| 2 | **Scan A** — `detect_testimony_access_proposal` a exactement deux lecteurs de production (5h, le helper) | **allowlist livrée vide**, pinnée vide par un test frère | remplace la V4 par-`grep` de la phase 2 (R3). Quand il tire : **on route le site vers le helper, on n'ajoute pas de ligne** (mika#2201) |
| 3 | **Scan B** — tout consommateur de `ctx.message_sender` sous `tools/` est gardé ou **hors population par nature** | **périmètre à deux entrées déclarées**, comparé **dans les deux sens** | `delegate_task` (passe le sender, n'envoie rien) et `run_team` (texte composé par le moteur). Ce n'est pas une allowlist d'exemption mais un **périmètre**, au sens de mika#2536 : ces sites n'émettent pas de texte du modèle. Une entrée dont le site a disparu rougit (assertion auto-nettoyante) |
| 4 | **Le test de parité** de `TESTIMONY_GATED_TOOLS` ↔ les outils appelant le helper | **armé**, bidirectionnel | motif `mika1952_gated_tools_match_the_reflection_contract_constant` |
| 5 | **L'eval AC3** (`doctrine_regressions/`) | **armé** (déterministe, `MockLlmProvider`, zéro réseau) | la moitié comportementale réelle-provider reste celle de `testimony_access_proposal_replayed.rs`, déjà livrée désarmée par la phase 2 et **non modifiée** |

**La limite du dispositif, nommée.** Les scans A et B couvrent deux classes
qu'aucun test comportemental ne peut voir — un troisième lecteur du prédicat, ou un
nouvel outil envoyant directement — parce qu'aucune des deux **ne rend une décision
fausse le jour où elle est écrite** : tout reste vert et seule la couverture se
perd, en silence (classe `grooming_marker`, mika#2158). Ce qu'ils **ne** couvrent
pas : un outil qui atteindrait l'utilisateur sans passer par `ctx.message_sender` ni
par un `action_type` planifiable. Aucun n'existe aujourd'hui ; armer un détecteur
sur une population vide est ce que mika#2520 refuse.

---

## Verification Contract

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | Le prédicat de la phase 1 est **inchangé** | `cargo test -p mika-agent evidence::guards::tests::mika1960` | vert, 22 tests, **aucun modifié** |
| V2 | AC2 — les trois formulations prescrites passent **par le chemin outil** | `cargo test -p mika-agent tools::send_message::tests::mika2627` | vert ; le `MockSender` **a reçu** les trois refus |
| V3 | **AC3, vu ROUGE avant le correctif** | `cargo test -p mika-agent --test eval -- testimony_access_proposal_send_message` | retirer l'appel au helper ⇒ **rouge** (le mock reçoit la proposition) ; restaurer ⇒ vert |
| V4 | AC3 — contrôle positif | idem | un message ordinaire **est délivré** dans le même fichier |
| V5 | R4 — le refus ne pose aucun verdict | `cargo test -p mika-agent tools::send_message::tests::mika2627_le_refus_ne_pose_aucun_verdict` | vert : `delivery.is_none()` |
| V6 | D3(c) — la doctrine précède la longueur | idem, test sur 12 000 caractères portant la proposition | vert : refus doctrinal, pas `RefusedTooLong` |
| V7 | D3(b) — rien n'est persisté | idem | vert : `messages` ne porte pas le texte refusé |
| V8 | D4 — le discriminant `action_type` | `cargo test -p mika-agent tools::create_reminder::tests::mika2627 tools::create_scheduled_task::tests::mika2627` | vert, **y compris le contrôle négatif** `resume_agent` qui n'est pas refusé |
| V9 | **Scan A, vu rouge** | planter un troisième appel au prédicat, lancer le scan | **rouge**, nommant le fichier ; retirer ⇒ vert |
| V10 | **Scan B, vu rouge dans les deux sens** | (a) ajouter un consommateur de `message_sender` non déclaré ⇒ rouge ; (b) retirer une entrée de périmètre dont le site existe ⇒ rouge | rouge puis vert dans les deux cas |
| V11 | Parité de `TESTIMONY_GATED_TOOLS`, dans les deux sens | `cargo test -p mika-agent tools::tests::mika2627_gated_tools` | vert ; retirer un appelant ⇒ rouge |
| V12 | Le canal est un format de fil | `cargo test -p mika-agent evidence::guards::tests::mika2627_le_canal_est_un_format_de_fil` | vert, quatre valeurs figées |
| V13 | Non-régression doctrine | `cargo test -p mika-agent --test eval -- doctrine_regressions` | vert, les douze scénarios existants inclus |
| V14 | Non-régression mika#2136 | `cargo test -p mika-agent --test eval -- undelivered_send` | vert — la garde 6f n'est pas touchée |
| V15 | Non-régression globale | `cargo test --workspace` | vert |
| V16 | Lint, format et jetons canoniques | `cargo clippy --workspace --all-targets -- -D warnings` ; `cargo fmt --all --check` ; `bash scripts/check-canonical-tokens.sh` | propres |

**V3, V5 et V10 sont les vérifications porteuses.** V3 est la seule mesure en
chemin de production du défaut fondateur. V5 ferme l'interaction avec la garde 6f,
qui est le risque le plus coûteux de ce plan (RK2). V10 est ce qui distingue un
périmètre d'un tiroir.

---

## Surfaces opérateur

```bash
# 1. Une proposition a-t-elle été arrêtée, et par quel canal ?
grep guard.testimony_access_proposal "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{channel, matched_subject, matched_movement, agent_id, session_id}'

# 2. La population du canal outil, seule
grep guard.testimony_access_proposal "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.channel != "end_turn") | {channel, matched_subject}'

# 3. CONTRÔLE POSITIF — la garde 5h tourne-t-elle encore ?
grep guard.testimony_access_proposal "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.channel == null or .channel == "end_turn")' | wc -l
```

```sql
-- La population du refus, par outil (sans surface neuve — cf. D5)
SELECT tool_name, count(*) FROM tool_calls
 WHERE output LIKE 'REFUS (testimony-access, mika#2627)%' GROUP BY 1;

-- Les rows planifiées AVANT le déploiement, hors périmètre (R2-bis)
SELECT id, label FROM tasks
 WHERE action_type = 'send_message' AND status IN ('pending','recurring_active');
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `channel = "end_turn"` | WARN | **zéro** | inchangé par ce ticket |
| `channel = "send_message"` | WARN | **zéro** | chaque ligne est une proposition arrêtée avant l'utilisateur sur le canal des tours silencieux |
| `channel = "create_reminder"` / `"create_scheduled_task"` | WARN | **zéro** | une proposition différée arrêtée à la création |
| une même session portant plusieurs refus | WARN | **anomalie** | le modèle insiste : lire le prompt servi **avant** de toucher au prédicat |

**Préalable à toutes les sondes.** Elles décrivent le **binaire servi** : établir
après `make deploy` que le `mika-spirit` qui tourne porte le correctif avant toute
conclusion (classe mika#2340). Ce sont des **gestes d'opérateur** sur l'hôte — la
base n'est pas montée dans le bac à sable de dispatch.

**S1 — le défaut fondateur ne se rejoue pas** (premier tour silencieux qui tente).
Une ligne `channel = "send_message"`, et **aucun** message reçu par l'utilisateur.
*Halte 1 — aucune ligne alors qu'une proposition est partie :* **ne pas élargir le
prédicat par réflexe.** Établir d'abord le déploiement, puis lire le contrôle
positif (sonde 3) : zéro ligne des deux côtés ne prouve rien du tout (*une garde que
personne n'a exercée se lit exactement comme une garde qui marche*, mika#2205).

**S2 — RK5, 30 jours.** Chercher une proposition étalée sur deux `send_message`
consécutifs du même tour. *Halte 2 — une occurrence :* c'est le contournement
nommé, **pas** un défaut du prédicat. Ouvrir le suivi **avec cette occurrence**, et
surtout ne pas élargir le prédicat au-delà de la phrase — ce serait reprendre le
faux positif que la segmentation par phrase existe pour éviter.

**S3 — contrôle négatif de bruit, 7 jours.** Aucun refus sur un `send_message`
ordinaire, et en particulier aucun sur un refus que la doctrine prescrit.
*Halte 3 — une occurrence :* c'est un faux positif, et il coûte un message non
délivré à un utilisateur. **Désarmer d'abord** (revert de l'appel au helper sur le
site concerné), diagnostiquer ensuite — un message légitime refusé est un arbitrage
de prédicat, pas un seuil à régler.

**S4 — la population hors périmètre.** La requête SQL n°2 ci-dessus, une fois,
après déploiement. *Halte 4 — elle rend des lignes portant une proposition :* ce
sont les rows pré-déploiement de R2-bis ; le remède est un geste d'opérateur
(`mika tasks cancel`), pas un élargissement de la garde au tir.

---

## Risks

| # | risque | ce qui le ferme |
|---|---|---|
| RK1 | **Faux positif sur un `send_message` légitime.** Le coût change de nature par rapport à 5h : là c'était un re-prompt, ici c'est un **message qui ne part pas**. Sur un tour silencieux le budget de pas est borné (`max_steps`), donc une garde qui tire tard peut faire qu'aucun message ne parte du tout. | Le prédicat est inchangé et son fail-safe penche **vers ne pas tirer** (13 contrôles négatifs sur 22 tests). Le refus est un `ToolOutput::error` **lisible**, pas un échec muet, donc le modèle peut renvoyer corrigé dans le même tour. S3 est le contrôle négatif de bruit, et sa halte est « désarmer d'abord » |
| RK2 | **La garde 6f re-prompte le tour** pour un envoi que la doctrine refuse, en suggérant un découpage. | `delivery: None` (R4), épinglé par V5. **C'est le risque le plus coûteux du plan** : non fermé, il transforme un refus en boucle |
| RK3 | **Faux positif sur `resume_agent`** (un rappel qui est une note à soi-même). | Discriminant `action_type == send_message` (R2-ter), contrôle négatif en V8 |
| RK4 | **Un quatrième outil écrit demain sans la garde.** | Scans A et B + parité bidirectionnelle (§ Fire-Disposition). Limite nommée au même endroit |
| RK5 | **Le modèle dégrade son texte** au lieu de le corriger — par exemple en envoyant la proposition en deux messages dont aucun ne porte les deux couches dans la même phrase. | **Non fermé, et nommé.** Le prédicat segmente par phrase, donc une proposition étalée sur deux appels échappe. Même classe que le contournement « dégrader le verdict plutôt que le flag » de mika#2237, qui l'a laissé ouvert avec sa raison : la garde ne peut pas arbitrer l'intention. Signal : la sonde S2 |
| RK6 | **Le prédicat est appliqué deux fois** sur un chemin où `send_message` est appelé depuis un tour de conversation : le refus d'outil, puis 5h sur le texte final. | Ce n'est pas un défaut : les deux textes sont différents (le corps du message, puis la réponse finale), et le budget d'un coup de 5h n'est pas consommé par le refus d'outil, qui ne touche pas `intent_guard_retries`. Épinglé par un test |

---

## Definition of Done

- [ ] V1 à V16 vertes, avec **V3, V5 et V10 rapportées explicitement dans le corps
  de PR** (vues rouges puis vertes).
- [ ] Le recensement du § R2 est reproduit dans le corps de PR, les six lignes et
  leurs raisons comprises — c'est la forme exécutable d'AC1.
- [ ] Le doc-comment de `detect_testimony_access_proposal` est corrigé : la V4 de la
  phase 2 est inversée une seconde fois, et la population est désormais de **deux**
  lecteurs de production nommés (R3). Laissé en place, il prescrirait de supprimer
  le second site comme un doublon.
- [ ] `crates/mika-agent/CLAUDE.md` § 5h : le paragraphe affirmant que le
  contournement `send_message` est « named rather than claimed closed » est mis à
  jour — il est **fermé** sur ce canal, et ce qui reste ouvert est RK5 plus la
  population de R2-bis, nommés comme tels.
- [ ] `crates/mika-agent/docs/non-transit-data-grade.md` : la phrase citée par le
  ticket (« La garde ne lit que le texte final du tour ») est corrigée, et le résidu
  est nommé à sa place.
- [ ] Le tableau des événements de `CLAUDE.md` (§ Guard Fabrication Telemetry) porte
  le champ `channel` et son coût daté.
- [ ] Le `mod.rs` de `doctrine_regressions/` déclare le nouveau fichier et son tag
  (`doctrine:testimony-access-send-refused`), avec la raison de la scission — « n'a
  pas proposé sur le texte final » et « n'a pas envoyé la proposition » sont deux
  populations qu'on veut compter à part, exactement la règle que ce `mod.rs` a déjà
  écrite deux fois.
- [ ] Aucun `#[ignore]`, aucune entrée d'allowlist sur les scans A et B au-delà des
  deux entrées de périmètre déclarées (§ Fire-Disposition).
- [ ] Aucune valeur de réglage déplacée, aucune migration, aucune variable
  d'environnement neuve.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2627.

- [ ] **AC1.** `send_message` (et tout autre outil qui émet du texte vers
  l'utilisateur ; recensement exhaustif dans la PR) passe son corps au prédicat
  avant l'envoi. Un corps qui propose un accès testimony-grade est refusé, avec une
  raison qui nomme la doctrine.
- [ ] **AC2.** Un refus qui nomme la doctrine n'est **jamais** refusé (mêmes
  contrôles négatifs que V3 de la phase 2 : les trois formulations prescrites).
- [ ] **AC3.** Test de bout en bout sur un tour heartbeat ou callback : une
  proposition via `send_message` est bloquée (**vu rouge** avant le correctif) ; un
  message ordinaire passe (contrôle positif).
- [ ] **AC4.** Télémétrie `guard.testimony_access_proposal` avec un champ de canal
  (`end_turn` | `send_message`) qui distingue les deux populations.

**Note de périmètre sur AC4.** Le champ porte **quatre** valeurs et non deux, parce
qu'AC1 étend la couverture à deux outils de planification (R2). Les deux valeurs que
l'AC nomme sont servies littéralement ; les deux autres suivent la même règle de
nommage (D2).

---

## Ce que ce travail n'achète PAS

- **Il ne rend pas la surface *propose* structurelle au sens des Layers 2/3/4.** Le
  refus lit un texte sortant : il **rattrape** avant l'envoi, il ne rend pas l'agent
  incapable de formuler la proposition. La doctrine maison est *construis
  l'incapacité, ne promets pas la retenue* (mika#1991) ; elle n'est pas applicable
  ici et il faut l'écrire plutôt que le contourner — il n'existe aucune capacité à
  retirer, le livrable est du texte en langue naturelle.
- **Il ne rattrape aucune proposition déjà partie.** Rien ne réécrit un message
  envoyé, et **rien n'est rétro-estampillé** : la sonde est la **prochaine**
  occurrence.
- **Il ne couvre pas les rows planifiées avant le déploiement** (R2-bis).
- **Il n'ajoute aucune ligne `audit_events` et aucun compteur** (D5). Les seuls
  instruments sont le grep et la requête SQL du § Surfaces opérateur, et **leur
  silence ne prouve rien tant que personne ne les exécute**.
- **Il n'ajoute aucune variable d'environnement, et c'est une décision.** Le
  précédent le plus proche, mika#1646 (garde d'action destructive), n'en a pas non
  plus, pour la raison qu'il écrit : un désarmement par variable sur un chemin de
  doctrine serait un désarmement par coquille. Le geste de désarmement est un
  **revert**, et le coût d'un faux positif (RK1) le supporte.
- **Il ne touche ni la garde 5h, ni le prédicat, ni le re-prompt, ni le budget d'un
  coup, ni la moitié désarmée de la phase 2.**

---

## Hors périmètre, délibérément

- **La garde 5h, le prédicat, son re-prompt, son budget d'un coup et son
  `_uncorrected`** : inchangés. La seule modification de 5h est l'ajout du champ
  `channel = "end_turn"` sur sa ligne.
- **`run_gh issue comment` / `pr comment`** : écrit sur la forge, pas vers le
  tenant. Hors population, et le nommer évite de le redécouvrir.
- **`run_team` et `delegate_task`** : § Fire-Disposition, scan B — hors population
  par nature.
- **Le tir des rows planifiées** (R2-bis) : **suivi nommé**, précondition écrite —
  que la requête SQL n°2 montre une row portant une proposition.
- **RK5, la proposition étalée sur deux appels** : **suivi nommé**, précondition S2.
- **Un interrupteur d'environnement** : refusé avec sa raison (§ Ce que ce travail
  n'achète PAS).
- **Une table ou une ligne `audit_events`** : refusée avec sa raison (D5).
- **La moitié comportementale réelle-provider** : elle existe déjà, désarmée
  (`testimony_access_proposal_replayed.rs`), et sa précondition d'armement reste
  celle que la phase 2 a écrite — une suite `calibrate-*` couvrant un tenant famille
  ou champion, qui n'existe pas.

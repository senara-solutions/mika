# `promote_deferred_callback` est refusé sur un tour webhook PR (mika#2654)

**Ticket :** senara-solutions/mika#2654
**Famille :** mika#2649 (*un tour webhook PR agit sur le plan de dispatch au-delà de
ce que l'événement justifie*) — troisième et dernier cas nommé par son recensement AC4.
**Voisins :** mika#2649 (fermé, terme de **lignée**), mika#2653 (fermé, terme de
**vivacité**). Celui-ci est un terme de **classe de tour**, sans terme de cible.

---

## 1. Pourquoi — et la mesure qui a changé la nature de la décision

Le ticket a été ouvert avec une précondition explicite : *établir que la population
existe avant d'armer quoi que ce soit*, parce qu'un prédicat sur une population vide
produit un détecteur dont le silence ne prouve rien (classe mika#2205). Deux mesures
d'opérateur, en lecture seule sur `~/.mika/data/mika.db` le 2026-10-07, fenêtre
2026-09-07 → 2026-10-07, ont levé cette précondition.

**Mesure 1 — la population existe.** 73 appels à `promote_deferred_callback`, 58
sessions :

| origine de la session | sessions |
|---|---|
| tour webhook GitHub (`sessions.channel_type = 'github'`) | **27** (31 appels) |
| callback de pilote | 17 |
| deferred-dispatch | 14 |

**Mesure 2 — ce que ces tours obtiennent réellement :**

| | appels | réussis | détail des échecs |
|---|---|---|---|
| tours `channel_type = 'github'` (27 sessions) | 33 | 10 | 16 « No pending deferred wrapper », 7 `slot_busy` |
| dont mika-qa | 22 | **10** | 12 |
| dont mika-dev | 11 | **0** | 11 |
| dont tours Webhook Fallthrough (`audit_events.tool_name = 'webhook_fallthrough_turn'`) | 1 | 0 | 1 |

Les **10** promotions réussies depuis un tour webhook (jointure
`deferred_dispatch_force_promote_succeeded.after_value` → `tasks`) sont **toutes** le
fait de mika-qa, et **toutes** promeuvent un wrapper dont le parent est la
vérification de build **de la PR de l'événement** : #2250 (cinq promotions en 31 s
sur le même parent, deux tours webhook), #2273, #2393, #2406, #2416, #2661.
**Zéro promotion hors lignée mesurée.**

**Limite de la mesure, dite plutôt que découverte.** Le texte d'origine de ces tours
n'est pas persisté dans `messages` pour le canal `github` : la classe exacte de
l'événement se lit au **premier outil** du tour (`qa_pr_view` / `pr list` sur une PR
précise, pour au moins 7 des 27), pas au message. `channel_type = 'github'` est donc
un **sur-ensemble** de la population que la garde reconnaîtra — la garde teste le
préfixe du message, le compte ci-dessus teste le canal. Cet écart est ce que la sonde
S2 du § 11 mesure après déploiement, et il est la raison de la **halte 1**.

---

## 2. Rectifications que la lecture du code impose au corps du ticket

Le corps du ticket décrit l'outil autrement que le code ne le fait. Six
rectifications, dont les cinq premières viennent du matériau de groom (commentaire
opérateur du 2026-10-07, vérifiées ligne par ligne contre l'arbre à `b30b684f`) et la
sixième de ce groom.

- **R1 — l'outil n'est pas un échappatoire « quand le créneau est pris ».**
  `Database::force_promote_deferred_for_class` (`db/tasks.rs:4619`) **refuse** quand
  la classe est au plafond (`ForcePromoteResult::RejectedSlotBusy`,
  `db/tasks.rs:4630-4631`). Quand elle ne l'est pas, il fait ce que le backstop
  périodique `promote_pending_deferred_if_idle` (`task_engine/engine.rs`) fait de
  lui-même au scan suivant : `DB_SCAN_INTERVAL_TICKS = 60` (`engine.rs:27`,
  consommé à `engine.rs:1264`), soit **≤ 60 s**. Le corps du ticket décrit la
  fonction inverse de celle que le code remplit.
- **R2 — ce que l'outil ajoute au backstop, et c'est le vrai dommage.** Le backstop
  consulte la priorité opérateur (`has_pending_operator_task_for_class`, mika#1948
  AC3, `engine.rs:2610`) et **cède la place** quand l'opérateur a du travail
  `pending` dans la classe — « trois dispatchers partagent ces créneaux […] et ils ne
  sont pas pairs », dit le commentaire du site. `force_promote_deferred_for_class`
  **ne la consulte pas**. Depuis un tour webhook, l'outil n'achète donc que deux
  choses : une avance ≤ 60 s, et le contournement de cette priorité. Plus le churn
  mesuré sur #2250 (cinq promotions en 31 s sur un même parent).
- **R3 — la raison qui faisait préférer la retenue à mika#2517 ne tient plus.**
  mika#2517 retenait `create_task` parce que refuser à la frontière d'outil demandait
  un champ que `ToolContext` n'avait pas. Depuis mika#2653,
  `ToolContext.is_webhook_pr_event_turn` existe (`tools/mod.rs:223`), calculé au
  **seul** site de conversation (`agent_loop/mod.rs:5571`) par
  `webhook_dispatch::is_webhook_pr_event_domain`. La forme 2 ne coûte **aucune**
  plomberie : le booléen est déjà là, déjà lu par une garde sœur. La forme 1 reste
  chère : ce booléen ne porte **pas** la cible de l'événement, et la filer dans
  `ToolContext` rouvre ce que mika#2517 puis mika#2573 ont refusé deux fois (*un
  verdict, jamais une charge utile* — le doc-comment du champ le dit en ces termes).
- **R4 — l'outil ne prend pas de cible.** Son unique paramètre est `dispatch_class`
  (`enum: ["implement","groom"]`). Il promeut « le prochain wrapper » de la classe.
  Borner à la lignée voudrait dire résoudre *quel* wrapper serait promu, le confronter
  à la cible de l'événement par la traversée de mika#2649, et changer l'API de la
  promotion. Ce n'est pas un terme ajouté : c'est un **second sélecteur de wrapper**.
- **R5 — aucun prompt ne prescrit l'outil.** Dans le dépôt, il n'apparaît que dans le
  code, la documentation et des plans. Aucun `system_prompt.md`, aucun `skill.toml`,
  aucun handler ne le cite. Les appels mesurés sont **spontanés**, émis après un
  `{"status":"deferred"}` rendu par l'exécuteur. Un refus ne casse aucun flux
  prescrit, et aucune garde `required_tools` ne le réclame — donc le refus **ne peut
  pas boucler**.
- **R6 — la destination de la prose a changé depuis le matériau de groom, et écrire
  où il dit d'écrire ne compilerait pas.** Le matériau (2026-10-07) prévoit « `CLAUDE.md`
  racine et `crates/mika-agent/CLAUDE.md` — section dédiée ». `b30b684f`
  (mika#2684, mergé depuis) a sorti 1,3 Mo de ces fichiers et institué un **plafond
  de 100 000 octets par `CLAUDE.md`**, tenu par
  `crates/mika-common/tests/claude_md_size.rs` (`CLAUDE_MD_MAX_BYTES`, avec un
  contrôle négatif qui refuse de relever le plafond). La prose des deux voisins vit
  désormais en deux endroits, et c'est là que celle-ci va :

  | contenu | destination | voisins déjà en place |
  |---|---|---|
  | conception, rectifications, surfaces | `docs/agent-internals/exec-handlers.md` | § mika#2649 (l. 293), § mika#2653 (l. 550) |
  | surfaces opérateur, sondes, haltes, recensement AC4 | `docs/operator/environment-variables.md` | § mika#2649 (l. 5234), § mika#2653 (l. 5455) |

  Les `CLAUDE.md` ne reçoivent **rien** : ils ne portent plus de section de cette
  famille, et y en ajouter une rougirait le plafond ou le ferait approcher sans
  bénéfice. **Corollaire à ne pas aggraver :** `exec-handlers.md:547` et
  `environment-variables.md:5452` renvoient encore à des sections de `CLAUDE.md` que
  mika#2684 a déplacées — deux pointeurs morts **préexistants**, hors périmètre ici
  (§ 12), mais la nouvelle section ne doit pas en ajouter un troisième : elle renvoie
  aux chemins `docs/` réels.

---

## 3. Décision : forme 2, par refus à la frontière d'outil

**Forme 1 (borner à la lignée) écartée sur la mesure.** Elle aurait autorisé 10
promotions mesurées sur 10 : sa population de refus serait **vide**, donc son silence
ne prouverait rien (classe mika#2205) — et ce détecteur inerte serait acheté au prix
de la charge utile que mika#2517 et mika#2573 ont refusée deux fois (R3), plus un
second sélecteur de wrapper (R4).

**Forme 2 retenue.** Sur un tour dont le message d'origine commence par `[GitHub] PR `
ou `[GitHub] Check suite `, `promote_deferred_callback` est **refusé**.

- **Ce qui est retiré :** une avance ≤ 60 s (R1), le contournement de la priorité
  opérateur (R2), le churn mesuré sur #2250.
- **Ce qui reste :** le backstop promeut le même wrapper au scan suivant, sous les
  **deux** contrôles — occupation du créneau *et* priorité opérateur. L'opérateur
  garde `mika tasks promote-deferred <class>` (CLI, hors `ToolContext`). Un
  `mika ask` d'opérateur reste hors population : son message est son propre texte,
  qui ne commence pas par le préfixe.

**Refus, pas retenue** (`FALLTHROUGH_WITHHELD_TOOLS`). Le corps du ticket dit
« retenir » ; trois raisons font préférer le refus :

1. **Le refus *est* la mesure.** Chaque tentative devient une ligne
   `tool_calls.output` et une ligne d'audit. Retenir l'outil du tableau ferait
   disparaître la population, et il faudrait inventer une ligne par tour webhook PR
   pour la garder mesurable.
2. **Même booléen, même forme que la garde sœur `cancel_task` (mika#2653).** Deux
   gardes d'une même famille posées de deux façons, c'est deux lectures à apprendre.
3. **Aucune garde `required_tools` ne réclame cet outil** (R5), donc le refus ne peut
   pas boucler. C'est la condition que mika#2653 a dû vérifier pour la même raison.

**Ce qui est accepté en échange :** un tour qui tente la promotion dépense un appel
d'outil refusé. Visible dans `tool_calls.output`, dans un INFO, et dans une ligne
d'audit.

**Pourquoi la retenue est ici *moins* coûteuse que pour `cancel_task`, et pourquoi on
ne la prend quand même pas.** mika#2653 a écarté la retenue parce que le prompt
prescrit l'annulation d'un dispatch réellement perdu ; ici aucun prompt ne prescrit
rien (R5), donc la retenue serait tenable. C'est la raison 1 qui tranche : la retenue
rend la population inobservable, et ce ticket est né d'une exigence de mesure.

---

## 4. Conception

| fichier | changement |
|---|---|
| `crates/mika-agent/src/tools/promote_deferred_callback.rs` | `enum ForcePromoteTurnDecision { Allowed, RefusedWebhookPrTurn }` + `pub(crate) fn classify_force_promote_turn(is_webhook_pr_event_turn: bool) -> ForcePromoteTurnDecision`, **pure**, le terme de classe de tour en **paramètre** (forme mika#2649, AC6). Garde dans `execute` **après** la validation de `dispatch_class` (l. 46-55 actuelles) et **avant** `ctx.db.force_promote_deferred_for_class(...)` (l. 57) — donc avant toute lecture ou écriture en base. `match` exhaustif, **sans bras `_ =>`**, au site d'appel comme chez la sœur. Constantes de fil : `PROMOTE_REFUSAL_PREFIX` / identifiant d'erreur stable (motif `TESTIMONY_ACCESS_REFUSAL_PREFIX`, `tools/mod.rs`), `FORCE_PROMOTE_TURN_GUARD_AUDIT_TOOL`. Corps du refus composé par une fonction dédiée ; journalisation + écriture d'audit **fire-and-forget** (motif `report_cancel_pilot_decision`, `cancel_task.rs:296`). |
| même fichier, `definition()` | une phrase dans la `description` : l'outil n'est pas disponible sur un tour ouvert par un événement PR / check-suite, et le moteur promeut de lui-même. C'est la **moitié intention** ; la garde est la moitié qui tient (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). La phrase existante qui oriente vers `--override` **reste** : elle décrit le chemin opérateur, qui n'est pas sur ce tour. |
| `crates/mika-agent/src/canonical_tokens.rs` | scan SOLE WRITER du nouveau nom d'audit via `audit_name_literal_present` (comparaison **exacte** sur littéral, lignes de commentaire exclues, l. 1398) + anti-vacuité + contrôle de bonne foi sur fixture + allowlist `FORCE_PROMOTE_TURN_GUARD_SOLE_WRITER_EXCEPTIONS` livrée vide avec son assertion auto-nettoyante. Motif `mika2653_le_nom_daudit_a_un_seul_ecrivain` (l. 1606). |
| `crates/mika-agent/tests/eval/test_promote_deferred_webhook_pr_2654.rs` + `crates/mika-agent/tests/eval.rs` | test de chemin de production (AC9), frère de `test_cancel_task_live_pilot_2653.rs`. |
| `docs/agent-internals/exec-handlers.md` | section dédiée après celle de mika#2653 (l. 550+) : mesure, R1–R6, conception, surfaces, « ce que ce travail n'achète pas ». |
| `docs/operator/environment-variables.md` | section dédiée après celle de mika#2653 (l. 5455+) : surfaces opérateur, régimes attendus, sondes S1–S3, haltes 1–3 + halte transverse. Mise à jour des **trois** entrées de recensement qui nomment l'outil et son verdict : l. 5342 (recensement AC4 de mika#2649, « zéro occurrence mesurée » → la mesure du 2026-10-07), l. 5565 (recensement AC4 de mika#2653, « population mesurée vide » → **fermé par mika#2654**), l. 5700 / 5726 (les deux « ce que ce travail n'achète pas » qui le nomment comme résidu ouvert). |
| `docs/agent-internals/exec-handlers.md` l. 530 | même mise à jour côté conception : « suivi mika#2654 » → fermé, avec sa mesure. |

### Surfaces

**Nom d'audit** (`audit_events.tool_name`) :
`deferred_dispatch_force_promote_refused_webhook_pr_turn` — quatrième de la famille
`deferred_dispatch_force_promote_*` (mika#1453 en a trois :
`_succeeded`, `_rejected_slot_busy`, `_override`).
`target_key = dispatch_class:<c>`, `after_value = refused_webhook_pr_turn`,
`before_value = NULL`.

> **Le needle du scan est le nom COMPLET, exactement, jamais le préfixe de famille.**
> Les trois noms existants partagent `deferred_dispatch_force_promote` : un scan sur
> ce préfixe trouverait trois écrivains et serait rouge à la naissance. Et le résidu
> `..._audit_failed` a le nom complet pour **préfixe**, donc une comparaison en
> sous-chaîne le compterait comme second écrivain — c'est la correction que mika#2649
> puis mika#2634 puis mika#2653 ont dû écrire chacun sur son scan jumeau.
> `audit_name_literal_present` compare `lit.trim() == needle` : c'est la bonne forme,
> elle est déjà là, on l'appelle.

**Journal.** Le régime attendu du refus est **non vide au début, puis décroissant** :
donc un **INFO** `promote_deferred_refused_webhook_pr_turn` (un WARN sur une
population nominale est un WARN qu'on finit par museler — motif explicite de
`report_cancel_pilot_decision`). Le résidu d'écriture d'audit
`promote_deferred_guard_audit_failed` est en **WARN**, régime attendu vide.

**Rien n'est écrit hors population.** `Allowed` ne compose ni corps, ni ligne de
journal, ni ligne d'audit — ce serait l'essentiel du trafic (doctrine mika#2131), et
c'est aussi ce qui rend la non-régression d'AC3 **observable** : zéro ligne d'audit
sur un tour d'opérateur.

### Ce que le corps du refus nomme, et ce qu'il tait

Il dit **le fait** (« ce tour a été ouvert par un événement PR / check-suite »), dit
que **le moteur promeut le wrapper de lui-même** au prochain scan quand la classe a de
la place, et donne **la sortie correcte** : terminer le tour sur ce que l'événement
demande réellement. Il **ne nomme aucune commande** — ni `mika tasks promote-deferred`,
ni `--override`, ni `run_shell`, ni `update_task_status` : doctrine mika#2520 /
mika#2292, *un refus qui donne le gabarit est une fuite avec une étape de plus*. Et
`MIKA_DEV_IDENTITY` porte `shell-exec`, donc ce n'est pas théorique.

---

## 5. Definition of Done

- `classify_force_promote_turn` existe, est pure, prend le terme de classe de tour en
  paramètre, et son `match` au site d'appel n'a pas de bras `_ =>`.
- La garde est en place dans `PromoteDeferredCallbackTool::execute`, après la
  validation de `dispatch_class` et avant tout accès base.
- V1–V6 (§ 7) passent ; les cinq tests existants du module passent **sans
  modification**.
- S1–S2 passent ; `FORCE_PROMOTE_TURN_GUARD_SOLE_WRITER_EXCEPTIONS` est vide et son
  assertion auto-nettoyante l'épingle.
- Le test de chemin de production passe (AC9).
- M1–M6 (§ 7) ont **chacune** été vues rouge, une à la fois, sur un vert commité.
- La prose est en `docs/agent-internals/exec-handlers.md` et
  `docs/operator/environment-variables.md`, **pas** dans un `CLAUDE.md` (R6) ; les
  trois entrées de recensement sont à jour ; aucun nouveau pointeur mort.
- `cargo test -p mika-agent mika2654`, `cargo test -p mika-agent --test eval mika2654`,
  `cargo test -p mika-agent canonical_tokens`,
  `cargo test -p mika-common claude_md_size`,
  `cargo clippy -p mika-agent --all-targets -- -D warnings`, `cargo fmt --check` :
  tous verts.

## 6. Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les neuf critères
ci-dessous sont transcrits **verbatim** du § 5 du matériau de groom (commentaire
opérateur du 2026-10-07), seule AC8 étant amendée par R6 sur la destination de la
prose.

- **AC1** — Dans un tour où `ToolContext.is_webhook_pr_event_turn` est vrai,
  `promote_deferred_callback` rend `ToolOutput::error` préfixé par
  `PROMOTE_REFUSAL_PREFIX`, et **aucun** wrapper ne change de statut.
- **AC2** — Le refus précède tout accès base de l'outil. Un tour PR avec la classe au
  plafond est refusé sous le motif webhook, **jamais** sous `slot_busy`.
- **AC3** — Hors population (`false`), le comportement est identique octet pour octet.
  Les tests existants du module passent sans modification.
- **AC4** — Chaque refus écrit une ligne d'audit sous le nouveau nom (SOLE WRITER,
  épinglé par scan). Un échec d'écriture d'audit ne change pas le verdict.
- **AC5** — Le corps du refus ne nomme aucun contournement (liste interdite épinglée
  par test) et nomme la sortie correcte.
- **AC6** — Le terme de classe de tour est un **paramètre** de la fonction pure,
  jamais une branche de l'appelant (forme mika#2649) : « hors tour webhook PR, rien ne
  change » est une propriété **testée de la fonction**.
- **AC7** — Les tours Webhook Fallthrough, les tours silencieux (`DeferredDispatch`,
  callback) et les chemins opérateur ne sont **pas** affectés. Population mesurée hors
  périmètre : 1 tentative Fallthrough en 30 j, 19 appels sur
  `channel_type = 'system'`.
- **AC8** — Documentation : la section porte la mesure, R1–R6, les surfaces, les
  sondes et leurs haltes, **en `docs/agent-internals/exec-handlers.md` et
  `docs/operator/environment-variables.md`** (R6 — le matériau disait « les deux
  `CLAUDE.md` », destination invalidée par mika#2684 et désormais sous plafond de
  build). Les recensements AC4 voisins sont mis à jour.
- **AC9** — Un test de chemin de production (`MockLlmProvider`, sans réseau) montre le
  refus sur un message `[GitHub] PR …` **et** sur un message `[GitHub] Check suite …`,
  la promotion sur un message ordinaire (contrôle négatif), et la promotion sur un
  message `[GitHub] Issue …`. Ce dernier épingle que le Fallthrough reste hors
  périmètre, **par décision**.

---

## 7. Porte de test

### Tests unitaires (`promote_deferred_callback.rs`)

`ctx.is_webhook_pr_event_turn` posé à la main sur `TestHarness::ctx()` — c'est ce que
fait déjà `cancel_task.rs`.

| | scénario | attendu |
|---|---|---|
| V1 | tour PR, wrapper `pending`, classe libre | refus préfixé ; **le wrapper est toujours `pending`** (assertion porteuse — AC1) |
| V2 | **contrôle négatif** : même fixture, `false` | `Promoted`. Sans lui, « la garde décide » ne se distingue pas de « la garde bloque tout » |
| V3 | tour PR, classe au plafond (fixture `test_promote_slot_busy`) | refus **webhook**, pas `slot_busy` — l'ordre, AC2 |
| V4 | tour PR | une ligne d'audit sous le nouveau nom, et **aucune** ligne `..._succeeded` |
| V5 | corps du refus | aucun terme de la liste interdite (`promote-deferred`, `--override`, `run_shell`, `update_task_status`) ; la sortie correcte est nommée — AC5 |
| V6 | `classify_force_promote_turn` | table de vérité complète (deux entrées, deux issues) ; constantes de fil épinglées — AC6 |

### Scans (`canonical_tokens.rs`)

- **S1** — SOLE WRITER du nom d'audit, comparaison **exacte**, avec **anti-vacuité** :
  rouge si le nom n'est écrit nulle part (un scan qui ne trouve personne se lit
  exactement comme un scan propre — mika#2103 / mika#2205).
- **S2** — **contrôle de bonne foi** : un second écrivain planté en fixture est vu
  rouge. Les trois formes à ne **pas** compter, parce qu'elles existeront dans
  l'arbre : le résidu `..._audit_failed` (préfixé par le nom), un commentaire qui
  cite le nom, et les trois noms frères de la famille `deferred_dispatch_force_promote_*`.

### Mutations — une à la fois, chacune vue rouge

**Commiter le vert d'abord, et vérifier le commit** : une mutation suivie d'un
`git checkout` efface le non-commité.

| | mutation | doit rougir |
|---|---|---|
| M1 | retirer l'appel de garde dans `execute` | V1, V3, V4 (+ eval AC9) |
| M2 | inverser le booléen dans `classify_force_promote_turn` | V2, V6 |
| M3 | déplacer la garde **après** `force_promote_deferred_for_class` | V1 (wrapper promu), V3 |
| M4 | poser `is_webhook_pr_event_turn: false` au site de conversation (`agent_loop/mod.rs:5571`) | **eval AC9 seul** ; les unitaires restent verts, et c'est attendu — c'est la raison d'être d'AC9 |
| M5 | retirer l'écriture d'audit | V4 |
| M6 | ajouter un second écrivain du nom d'audit | S1 |

**M4 n'est attestable que par le test de chemin de production**, et c'est le mode de
panne de toute cette famille : le booléen est posé à **un seul** site de production et
`false` aux vingt autres, donc des unitaires verts ne disent rien du câblage. Le même
câblage est déjà attesté pour le même site par `test_cancel_task_live_pilot_2653.rs`
(même booléen, même ligne) — ce qui rend le risque de M4 faible, mais pas nul, et c'est
pourquoi AC9 reste dans ce périmètre plutôt que renvoyé.

### Commandes obligatoires avant tout push

```bash
cargo test -p mika-agent mika2654
cargo test -p mika-agent --test eval mika2654
cargo test -p mika-agent canonical_tokens
cargo test -p mika-agent promote_deferred
cargo test -p mika-common claude_md_size
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
```

Au plus deux builds cargo simultanés, puis `cargo clean` au push : la cible telemetry
pèse ~45-52 Go.

---

## 8. Fire-Disposition

Ce plan livre des détecteurs : les deux scans S1–S2 (`canonical_tokens.rs`), les six
tests unitaires V1–V6, et le test de chemin de production AC9. Disposition canonique
retenue : **(a) exception nommée en allowlist — livrée VIDE.**

**Pourquoi vide est ici vérifiable et non espéré.** Le nom d'audit
`deferred_dispatch_force_promote_refused_webhook_pr_turn` est **neuf** : il n'existe
nulle part dans l'arbre avant cette PR, donc aucune violation préexistante ne peut
exister, et `FORCE_PROMOTE_TURN_GUARD_SOLE_WRITER_EXCEPTIONS` naît vide avec son
assertion auto-nettoyante (`..._est_vide`, motif
`mika2653_lallowlist_du_nom_daudit_est_vide`). Les tests V1–V6 et AC9 portent sur du
code neuf : ils arrivent **armés et verts**, rien à excepter.

**Et la vérification est obligatoire avant commit, parce que le voisin s'est trompé
exactement là.** La Fire-Disposition de mika#2649 annonçait une allowlist vide et la
mesure l'a **réfutée** (cinq regex `^\[GitHub\]` sur trois fichiers, dont une en
double) — un lint rouge à la naissance se fait désarmer. L'implémenteur **grep le nom
complet avant de commiter le scan** :

```bash
grep -rn 'deferred_dispatch_force_promote_refused_webhook_pr_turn' crates/
```

- **Un seul porteur** (`tools/promote_deferred_callback.rs`, la constante) : allowlist
  vide, comme prévu.
- **Plus d'un porteur** : ne **pas** allowlister (doctrine mika#2201 / mika#2323). Faire
  passer le second site par la constante. Si un second écrivain est *justifié*, le
  livrable devient un **recensement** — on y ajoute une entrée nommée avec son
  ticket, on n'y exempte pas un doublon (distinction mika#2633) — et il faut alors le
  dire dans le corps de la PR.

Aucun détecteur n'est livré désarmé ; aucune halte-et-remontée n'est prévue.

---

## 9. Taille estimée

Lignes de code, tests et doc-comments Rust compris, hors `docs/` et hors `CLAUDE.md`
(mika#2636). La prose de documentation est comptée à part et ne compte pas dans le
total.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/tools/promote_deferred_callback.rs` — enum, classifieur, garde, corps du refus, journal + audit, description | 250 |
| tests unitaires V1–V6 (même fichier) | 220 |
| `crates/mika-agent/src/canonical_tokens.rs` — S1, S2, allowlist + assertion | 150 |
| `crates/mika-agent/tests/eval/test_promote_deferred_webhook_pr_2654.rs` + `tests/eval.rs` (AC9) | 260 |
| *prose `docs/agent-internals/exec-handlers.md` + `docs/operator/environment-variables.md` + 3 recensements (non comptée)* | *~180* |

Total estimé : 880 lignes

**Ce total corrige celui du matériau de groom dans les deux sens, et c'est un
re-mesurage, pas un arrondi.** Le matériau annonçait « 580 lignes, puis 1 450 avec une
marge × 2,5, donc découpage en deux phases ». Deux corrections :

- **580 sous-comptait.** Il ne comptait pas les doc-comments Rust, qui sont
  l'essentiel du volume de la garde sœur : la région de garde de `cancel_task.rs`
  (constantes → `report_cancel_pilot_decision`) fait ~350 lignes dont une large
  majorité de prose raisonnée, et c'est l'idiome que ce dépôt exige. mika#2636 compte
  les lignes de code ; un doc-comment en est une.
- **La marge × 2,5 sur-comptait, et surtout elle double le critère.** Le seuil de 1 000
  de mika#2636 porte sur le compte de lignes, pas sur un compte majoré : appliquer une
  marge puis comparer au seuil, c'est comparer au seuil deux fois.

**880 < 1 000 : une seule PR, pas de découpage requis.** Le précédent direct le
confirme — mika#2653 a livré ses phases A **et** B sur une seule branche, avec un test
de chemin de production **plus lourd** (402 lignes, un processus enfant réellement vif,
RAII de nettoyage, `#[cfg(target_os = "linux")]`). Celui-ci n'a besoin que d'une ligne
de wrapper `pending` en base.

**Plancher mergeable, si le pilote tire en longueur.** L'ordre de commit ci-dessous
est contraignant, et la garde + ses unitaires + les scans (620 lignes) **ferment le
défaut à eux seuls**. Un pilote qui approcherait le plafond de tours après le commit 3
livre cet état et renvoie AC9 à un suivi nommé — en le disant dans le corps de la PR
avec sa ligne `Tracked in:` (le gate `pr-body-validation` l'exige), **jamais** en
l'omettant. Ce n'est pas le plan nominal : c'est sa sortie de secours, et M4 dit
exactement ce qu'elle coûte.

Ordre de commit :

1. enum + `classify_force_promote_turn` + V6 (la fonction pure et sa table de vérité) ;
2. garde dans `execute` + V1, V2, V3 (le comportement et son contrôle négatif) ;
3. journal + audit + V4, V5 ; scans S1, S2 (+ le grep du § 8 **avant** de commiter) ;
4. test de chemin de production AC9 + `tests/eval.rs` ;
5. prose `docs/` + les trois recensements.

---

## 10. Risques et arbitrages

- **Faux refus.** Un tour webhook PR qui avait une raison légitime de promouvoir perd
  ≤ 60 s. Le backstop promeut ensuite, sous deux contrôles au lieu de zéro. Coût
  nommé, borné, et c'est la **halte 3**.
- **Le périmètre est le canal, pas l'intention.** La garde mord sur le **préfixe du
  message**, la mesure comptait le **canal** (`channel_type = 'github'`) : les deux
  populations ne coïncident pas exactement (§ 1, limite). Un tour `github` dont le
  message ne porte pas le préfixe reste autorisé, par construction — c'est l'écart que
  S2 mesure et la halte 1 instruit.
- **Un tour webhook servi en mode silencieux échapperait à la garde.** Borne héritée
  de mika#2517 / mika#2573, ni élargie ni modifiée, population mesurée vide (un tour
  silencieux a `originating_message = None` depuis mika#933). Le doc-comment du champ
  le dit déjà ; cette PR ne le change pas.
- **Le cinquième booléen de `ToolContext`.** Cette PR en ajoute un **lecteur**, pas un
  champ. Le suivi nommé par mika#2653 — réunir les deux booléens en un
  `webhook_turn: WebhookTurnClass` (`None | Fallthrough | PrEvent`) pour rendre
  l'exclusivité structurelle — gagne un second lecteur à migrer, et reste un suivi.

---

## 11. Sondes post-déploiement (gestes d'opérateur sur l'hôte), et leurs haltes

```sql
-- S1 : la garde mord (régime attendu : non vide au début, puis décroissant)
SELECT count(*) FROM audit_events
 WHERE tool_name = 'deferred_dispatch_force_promote_refused_webhook_pr_turn';

-- S2 : plus aucune promotion réussie depuis un tour github (régime attendu : zéro)
SELECT count(*) FROM tool_calls tc JOIN sessions s ON s.id = tc.session_id
 WHERE tc.tool_name = 'promote_deferred_callback' AND s.channel_type = 'github'
   AND tc.success = 1 AND tc.created_at > '<déploiement>';

-- S3 : CONTRÔLE NÉGATIF — le backstop promeut toujours
SELECT count(*) FROM audit_events
 WHERE tool_name = 'deferred_dispatch_promoted' AND created_at > '<déploiement>';
```

- **Halte 1 — S2 est non vide.** Lire **d'abord** si la ligne vient d'un tour
  Fallthrough ou d'un tour ready-label en `Passthrough` : ces deux familles sont hors
  population **par décision** (AC7), et `channel_type = 'github'` les inclut (§ 1,
  limite). Si la ligne vient d'un tour PR, vérifier que le binaire servi porte le
  correctif (classe mika#2340 : `cat ~/.mika/skills/.manifest-writer`) **avant** de
  toucher au prédicat.
- **Halte 2 — S3 tombe à zéro alors que des wrappers attendent.** Ce n'est **pas** ce
  correctif : il ne touche pas le backstop. Lire
  `deferred_dispatch_promotion_starved` (mika#2169) et la priorité opérateur
  (`has_pending_operator_task_for_class`).
- **Halte 3 — les builds QA attendent visiblement plus d'une minute après un refus.**
  C'est le coût nommé (≤ 60 s) qui se manifeste. **Ne pas rouvrir l'outil** : mesurer
  l'écart refus → `deferred_dispatch_promoted` sur le même parent. Si l'écart dépasse
  un scan, le défaut est **au backstop**, pas ici.
- **Halte transverse — zéro refus et zéro tour PR ne prouve rien.** Il faut qu'un tour
  webhook PR ait **tenté** la promotion depuis le déploiement pour que le silence de S1
  soit une information (mika#2205). Sans cela, « la garde ne mord jamais » et « la
  garde ne tourne pas » rendent les mêmes octets.

---

## 12. Hors périmètre, délibérément

- **Les tours Webhook Fallthrough.** Population mesurée : **1** tentative, échouée, en
  30 j. L'ajout à `FALLTHROUGH_WITHHELD_TOOLS` serait cohérent avec la doctrine
  mika#2517 (*un tour Fallthrough ne crée pas de travail*), mais une garde armée sur
  une population quasi vide ne prouverait rien. **Suivi**, à ouvrir seulement si une
  mesure montre une tentative **réussie**. AC9 épingle cette frontière par un cas
  `[GitHub] Issue …` qui **passe**.
- **L'absence de la priorité opérateur dans `force_promote_deferred_for_class`** (R2).
  Elle touche aussi le CLI `mika tasks promote-deferred`, où le contournement est un
  geste d'opérateur **légitime**. Décision distincte, et elle appartient au CLI autant
  qu'à l'outil.
- **Les tours silencieux** (19 appels sur `channel_type = 'system'`). Les promotions
  réussies hors tour webhook incluent 4 promotions mika-dev sur des parents
  ready-label : légitimes **par conception** — `DeferredDispatch` est le chemin de
  reprise.
- **Le churn de mika-qa sur ses propres builds** (#2250, cinq promotions en 31 s).
  mika#2671 B1/B2 (une revue par tête stable) en réduit la **source** ; ce correctif en
  ferme un **vecteur**. Ni l'un ni l'autre ne le ferme seul.
- **Les deux pointeurs morts de mika#2684** (`exec-handlers.md:547`,
  `environment-variables.md:5452`, qui renvoient à des sections de `CLAUDE.md`
  déplacées). Préexistants, orthogonaux, et les réparer ici mélangerait deux
  changements. Cette PR n'en ajoute pas un troisième (R6) — c'est tout ce qu'elle
  doit à ce sujet.
- **Le cap implement** (mika#2652) et la réunion des deux booléens de `ToolContext`
  en `WebhookTurnClass` : deux suivis déjà nommés par mika#2649 et mika#2653,
  inchangés.

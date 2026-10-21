# feat(notifications) — mika#1381, vol 4/4 : l'axe P2 attesté au transport, et la jonction qui manquait

- **Ticket** : senara-solutions/mika#1381 — *severity-tiered operator notifications — P0 « loop down » on a never-muted surface*
- **Branche** : `feat/1381/notifications-severity-tiered-operator`
- **Vol** : 4 sur 4 — le dernier. Vols livrés : #2813 (`059652e9`), #2846 (`c5e64079`), #2856 (`7523ff68`).
- **Date de grooming** : 2026-10-21
- **Projet** : *Loop Trustworthiness*, Phase 1b — keystone.

---

## 1. Contexte : où en est le ticket

Le corps de #1381 demande deux choses, et il faut les séparer pour lire l'état réel.

| ce que le corps demande | état vérifié au 2026-10-21 |
|---|---|
| Rail de sévérité `P0`/`P2` + surface jamais muette (`MIKA_OPERATOR_ESCALATION_CHAT_ID`) | ✅ vol 1 — #2813, `059652e9` |
| Une aide unique à sévérité **obligatoire** (le compilateur est le détecteur) | ✅ vol 2 — #2846, `c5e64079` |
| Le P0 amiral (`agent_loop/mod.rs:4337`) attesté **bout en bout** | ✅ vol 3 — #2856, `7523ff68` |
| Classe P0 `block[security]` (#15 de l'inventaire) | ✅ site `server/verdict_handler.rs:1875` (handler d'escalade générique) |
| Classe P0 dispatch-stall | ✅ site `agent_loop/mod.rs:4337` |
| Classe P0 `recovery-runbook` (#4) | ❌ **aucun site d'émission n'existe** — hors périmètre depuis le vol 1 |
| Classe P0 `fabrication-detected` (#11) | ❌ idem |
| Axe 2 — *« batch/suppress P2 acks via INTENT_GUARD relaxation (let mika-dev EndTurn without `send_message` on non-actionable webhooks) »* | ⚠️ **déjà fermé côté moteur — voir § 2** |
| Gateway `crates/mika-gateway/.../github.rs` (« format ») | N/A — mauvais locus, réfuté par le vol 1 : ce fichier traite les webhooks **entrants** |

Le vol 3 a laissé **un** axe nommé, et un seul, dans son D8 :

> **Tracked in: senara-solutions/mika#1381 — axe INTENT_GUARDS** : relâchement de la garde de
> post-condition pour laisser mika-dev clore un tour sans `send_message` sur un webhook non
> actionnable (batching / suppression des acks P2). Hors vols 1 à 3, à groomer séparément.

Ce vol 4 groome cet axe. **Et il trouve qu'il est déjà fait.**

---

## 2. La découverte : l'axe INTENT_GUARDS a été fermé par mika#2517, trois semaines avant le vol 1

Le relâchement que #1381 demande existe **des deux côtés**, moteur et prompt, et il a été livré par un
autre ticket qui ne savait pas qu'il fermait celui-ci.

**Côté moteur** — `webhook_zero_tools_trigger` (`crates/mika-agent/src/agent_loop/mod.rs:11214`) rend
`false` sur tout le **domaine Webhook Fallthrough** :

```rust
// mika#2517 U3 — the Webhook Fallthrough domain is the long tail mika#1469
// deferred. […] NOT a relaxation: on this domain the guard's own premise —
// "webhook events require action" — is FALSE by contract.
if crate::webhook_dispatch::is_webhook_fallthrough_domain(msg) { return false; }
```

Le domaine est défini au site unique `webhook_dispatch.rs:99` : tout `[GitHub] …` qui n'est **ni** un
marqueur de dispatch ready-label, **ni** un événement de PR (territoire qa), **ni** un événement de
check-suite (territoire ci) — c'est-à-dire exactement les commentaires, les labels, les assignations,
les fermetures d'issue et les discussions : la population des acks P2 #1/#2 de l'inventaire.

**Provenance, datée** : commit `b1d96948`, PR #2530, mergée le **2026-09-25**. Soit ~3 semaines
**avant** le vol 1 de #1381 (#2813, 2026-10-19), que ses trois plans ont déféré sans le savoir.

**Côté prompt** — `skills/bundled/self-dev/system_prompt.md:122-136`, § *Webhook Fallthrough*, SCOPE
RULE (HARD GATE) : les actions permises sont « 1. Acknowledge the event / 2. … update the task's note /
3. **If the event requires Vincent's attention** … notify via `send_message` / 4. Stop ». Le `send_message`
est **conditionnel**. Rien n'impose d'ack systématique.

**Et l'attestation bout-en-bout du garde existe aussi.** `tests/eval/test_webhook_zero_tools_guard.rs:174`
— `guard_stands_down_on_issue_webhook` — fait tourner `run_agent` sur `[GitHub] Issue opened: …` avec une
seule réponse texte et assertionne `assert_exact_steps(&trace, 1)` : aucun re-prompt. Son contrôle positif
est dans le même fichier (`guard_fires_on_webhook_with_zero_tools`, `guard_fires_only_once`, tous deux hors
domaine).

**Conclusion** : il n'y a **rien à implémenter** sur l'axe INTENT_GUARDS. Un plan qui livrerait un
relâchement livrerait un doublon, et un doublon de garde est une seconde réponse à la même question —
exactement ce que le site unique de `webhook_dispatch.rs` refuse par construction.

**Vérification de l'exhaustivité** : aucun autre garde du registre `INTENT_GUARDS`
(`agent_loop/mod.rs:11002`, 6 entrées) n'exige `send_message` sur un webhook non actionnable. Le seul qui
exige une action terminale, `CALLBACK_TERMINAL_ACTION_LABEL` (`:11125`), porte sur les tours de **callback**,
pas sur les webhooks. Les 13 gardes inline (`:2504`–`:3782`) n'en exigent pas non plus.

---

## 3. Ce qui reste, et pourquoi c'est mince — dit franchement

Deux lacunes réelles subsistent, et aucune des deux n'est du code moteur.

**(a) L'attestation porte sur le garde, pas sur le transport.** `guard_stands_down_on_issue_webhook`
assertionne « le tour n'a pas été re-prompté ». Le corps de #1381 demande autre chose : **« aucun ack P2
n'est parti »**. Ce sont deux populations distinctes, non soustractibles (classe mika#2205) : un moteur
dont le garde est désarmé **et** qui émettrait quand même — par un site d'émission inconditionnel sur le
chemin webhook, présent ou futur — passerait `assert_exact_steps(&trace, 1)` sans difficulté. Le vol 3 a
établi le patron qui répond à cette question-là : observer **le transport**, via un `MessageSender` injecté,
et non la trace d'outils. L'axe P0 l'a reçu ; l'axe P2 ne l'a pas.

**(b) Le lien mika#2517 ↔ #1381 n'existe nulle part.** Ni le code, ni `crates/mika-agent/CLAUDE.md`, ni
aucun des trois plans antérieurs ne dit que l'axe 2 de #1381 est clos. Les trois plans l'ont déféré, dont
le dernier **après** sa fermeture. Un quatrième lecteur rouvrirait le même axe : c'est déjà arrivé trois
fois.

**Honnêteté sur le poids probant.** Le test de (a) n'est pas un test difficile, et son cas central est
proche de la tautologie : un tour qui n'appelle aucun outil n'émet rien. Ce qui en fait une attestation et
non une formalité, c'est son **contrôle d'anti-vacuité** (D4) : le même double, la même shape, mais le
modèle appelle délibérément `send_message` — et l'émission est observée, **sans étage**. Ce contrôle dit
quelque chose de non trivial et de structurant pour #1381 : `tools/send_message.rs:142` passe par
`sender.send(&cleaned)`, jamais par `send_with_severity`. Donc **un ack émis par l'outil n'est pas un P2
routé, c'est un message sans étage** — la suppression des acks P2 ne peut pas s'obtenir par le rail de
sévérité, seulement par le garde. C'est l'argument qui justifie *a posteriori* que le corps du ticket ait
nommé INTENT_GUARDS et non le routage, et il n'est attesté par rien aujourd'hui.

---

## 4. Décisions

**D1 — Le vol 4 ne touche aucun fichier de `src/`.** L'axe est fermé (§ 2) ; le reste est une attestation
et une jonction documentaire. Un plan qui modifierait `webhook_zero_tools_trigger` ou le registre
`INTENT_GUARDS` serait un doublon. Le seul changement hors `tests/` et hors docs est une **élévation de
visibilité** (D3).

**D2 — Le livrable probant observe le transport, pas la trace d'outils.** Un nouveau fichier
`crates/mika-agent/tests/eval/test_p2_ack_suppression_1381.rs`, qui injecte un `MessageSender` via
`EvalHarness::builder().message_sender(…)` (`tests/eval/harness.rs:459` → `AgentParams.message_sender`
`:116` → `tool_ctx.message_sender` `agent_loop/mod.rs:6006`) et assertionne sur **ce que le transport a
reçu**. C'est le patron du vol 3, appliqué à l'axe symétrique.

**D3 — Le double d'observation est réutilisé, pas recopié.** `SeverityRecordingSender` et la sentinelle
`SEVERITE_ABSENTE` vivent dans `tests/eval/test_dispatch_stall_p0_1381.rs` (`:97`, `:76`) et passent
`pub(crate)`, exactement comme `tools_with_dispatch_stubs()` l'a fait au vol 3 (D2 du vol 3).

> **Ceci n'inverse pas la note `:93` du vol 3.** Cette note refuse le `SeverityRecordingSender` de
> `server/verdict_handler.rs` — un type `#[cfg(test)]` **à l'intérieur de la crate source**, donc
> inatteignable depuis le binaire de test `eval`. Ici les deux fichiers sont deux modules du **même**
> binaire : la réutilisation est possible et la duplication serait un second témoin de fuite à maintenir.
> Le doc-comment du type nommera son second consommateur, comme le vol 3 l'a fait pour les stubs.

**D4 — Trois tests, et les deux contrôles ne sont pas optionnels.**

| test | shape | ce qu'il atteste |
|---|---|---|
| `mika1381_un_webhook_non_actionnable_ne_part_en_aucun_accuse` | `[GitHub] New comment on senara-solutions/mika#1381 by samidarko` | le cœur : aucun re-prompt **et** zéro émission au transport |
| `mika1381_hors_du_domaine_la_garde_exige_toujours_un_outil` | `[GitHub] Check suite failure on senara-solutions/mika@…` | contrôle positif : le garde tire encore. **Sans lui**, le test 1 serait vert sur un moteur ayant retiré tous ses gardes |
| `mika1381_un_accuse_emis_par_loutil_arrive_sans_etage` | même shape que le test 1, mais tour 1 = appel réel de `send_message` | anti-vacuité du double (il n'est pas aveugle) **et** la propriété de § 3 : `send_message` passe par `send()`, donc l'ack arrive `SEVERITE_ABSENTE` |

**D5 — La shape du cœur est `New comment on`, pas `Issue opened:`.** `guard_stands_down_on_issue_webhook`
couvre déjà `Issue opened:` au niveau du garde ; les commentaires sont la population que le corps de #1381
nomme (acks #1/#2 de l'inventaire) et la plus volumineuse du domaine. Choisir la même shape qu'un test
existant dirait moins.

**D6 — Deux réponses fournies au mock, assertion sur `llm_call_count == 1`.** Si l'on ne fournissait
qu'une réponse, une régression réarmant le garde ferait échouer le **mock** (plus de réponse disponible) au
lieu de l'assertion, et le message d'échec ne nommerait pas la cause. La seconde réponse ne doit jamais
être consommée, et c'est l'assertion qui le dit. `trace.llm_call_count` est l'instrument, par homogénéité
avec le vol 3 ; `assert_exact_steps` est son équivalent sur ce scénario.

**D7 — Le registre d'outils est `default_tools()` nu pour les tests 1 et 3, pas
`tools_with_dispatch_stubs()`.** Ce dernier enregistre `StubSendMessage`, qui ne traverse **pas**
`tool_ctx.message_sender` : avec lui, le test 3 observerait zéro émission et le double serait aveugle sans
que rien ne le dise. Le vrai `SendMessageTool` est enregistré par `default_tools()`
(`src/tools/mod.rs:1416`). Sur le domaine fallthrough, mika#2517 retient `create_task` et mika#2573 refuse
`run_gh`, mais `send_message` reste servi — le test 3 est donc un scénario atteignable, pas une fiction.

**D8 — Les recettes de rouge-avant sont obligatoires et exécutées.** Trois injections, le site restauré à
l'identique après chacune, la sortie d'échec recopiée verbatim dans le header du module, sur le modèle du
vol 3 :
- **R1** — retirer la ligne `is_webhook_fallthrough_domain(msg) → false` de `webhook_zero_tools_trigger` : le test 1 doit tomber sur `llm_call_count == 2`.
- **R2** — faire rendre `false` à `is_webhook_fallthrough_domain` sur tout : le test 2 (contrôle positif) doit tomber. C'est la régression « on a retiré tous les gardes » que le test 1 seul ne verrait pas.
- **R3** — remplacer le double par un `NoopSender` dans le test 3 : il doit tomber sur `emissions.len() == 0`. C'est la preuve que le double n'est pas décoratif.

Un test de suppression qui n'a jamais été rouge n'atteste rien.

**D9 — La jonction documentaire va dans `crates/mika-agent/CLAUDE.md` § *MessageSender Trait*,** à la
suite du paragraphe mika#1381 existant : l'axe 2 du ticket est clos par mika#2517 U3 (`b1d96948`, PR #2530,
2026-09-25), le garde est `webhook_zero_tools` et son prédicat `webhook_zero_tools_trigger`, le domaine est
`is_webhook_fallthrough_domain`, l'attestation du garde est
`guard_stands_down_on_issue_webhook` et celle du transport est le livrable de ce vol. **Ce qui doit être
écrit, et ne l'est nulle part :** un ack passé par l'outil `send_message` n'a pas d'étage, donc la
suppression des acks P2 est une propriété du **garde** et jamais du routage.

**D10 — La PR porte `Closes #1381`, et les deux classes P0 sans site partent en tickets.** Voir § 8.

---

## 5. Livrables

1. `crates/mika-agent/tests/eval/test_p2_ack_suppression_1381.rs` — header de doctrine + les trois
   recettes de rouge-avant verbatim, imports, les trois tests de D4.
2. `crates/mika-agent/tests/eval/test_dispatch_stall_p0_1381.rs` — `pub(crate)` sur
   `SeverityRecordingSender`, ses deux méthodes d'accès, `Emission` et `SEVERITE_ABSENTE` ; doc-comment
   nommant le second consommateur (D3).
3. `crates/mika-agent/tests/eval.rs` — déclaration du nouveau module et son commentaire.
4. `crates/mika-agent/CLAUDE.md` — la jonction de D9 (hors du compte de lignes).
5. Deux tickets de suivi pour les classes P0 sans site d'émission (§ 8), ouverts par le pilote avant
   l'écriture du corps de PR, pour que la ligne `Tracked in:` nomme de vrais numéros.

---

## 6. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `tests/eval/test_p2_ack_suppression_1381.rs` — header + 3 recettes de rouge-avant, imports, les 3 tests de D4 | 165 |
| `tests/eval/test_dispatch_stall_p0_1381.rs` — `pub(crate)` sur le double, `Emission`, `SEVERITE_ABSENTE`, doc-comment du second consommateur | 8 |
| `tests/eval.rs` — déclaration du module + son commentaire | 3 |

Total estimé : 176 lignes

Sous la borne par vol de 279 lignes, avec une marge de 37 %. Elle tient pour trois raisons
structurelles, pas par découpage. **Un** : il n'y a aucun code moteur à écrire — le relâchement existe
déjà (§ 2), et ce vol n'écrit que l'instrument d'observation. **Deux** : le double d'observation et sa
sentinelle sont réutilisés et non recopiés (D3), ce qui économise les ~45 lignes qu'une copie aurait
coûtées. **Trois** : les trois scénarios sont des tours à une ou deux réponses, sans stub à écrire — le
registre est `default_tools()` nu (D7). Le vol 3, qui a écrit le double **et** son header de doctrine, a
mesuré 194 lignes ; ce vol en reprend le patron avec un test de plus et un type de moins.

`crates/mika-agent/CLAUDE.md` est hors du compte : la consigne exclut `docs/` et `CLAUDE.md`.

Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

---

## 7. Fire-Disposition

**Gate N/A — ce vol ne livre aucun détecteur**, et la section n'est pas inventée : elle dit pourquoi.

Un détecteur au sens de mika#1574 balaie une population qu'il n'a pas écrite — un arbre de sources, un
ensemble de manifestes, un dépôt — et peut donc **découvrir** des violations antérieures qu'il faut
disposer (allowlist nommée, livraison désarmée, ou halte-et-remontée). Les trois livrables de ce vol sont
des tests `#[tokio::test]` qui composent eux-mêmes le run qu'ils observent : trois réponses de mock, un
registre d'outils, un double d'observation. Il n'y a rien à découvrir dans un run qu'ils ont écrit, et leur
chemin de succès n'est pas « aucune violation trouvée » mais « la valeur attendue est arrivée ».

Le raisonnement et sa discrimination sont ceux du vol 3, repris à l'identique parce que la forme du
livrable est la même.

Le seul voisin de ce vol qui **est** un détecteur —
`tests/fallthrough_withheld_tools_required_2517.rs`, un scan des manifestes de `skills/bundled/` dont la
population est vide par mesure — existe déjà, est vert, et n'est pas touché.

---

## 8. Arbitrage `Closes #1381` vs `Part of #1381`

Le corps de #1381 nomme quatre classes P0. Deux ont un site (`dispatch-stall`, `block[security]`), deux
n'en ont aucun : `recovery-runbook` (#4 de l'inventaire) et `fabrication-detected` (#11). Le vol 1 l'a
établi par lecture du code et l'a sorti du périmètre, et ce n'est pas une omission : **créer un site
d'émission est une extension de périmètre, pas une classification**. Le ticket lui-même le dit — commentaire
du 2026-08-22 : *« recovery-runbook and fabrication-detected classes have no existing notification callsites
(deferred as separate follow-up tickets — introducing new emission sites is scope expansion) »* — et
l'architecte a rendu READY sur cette base (session `39503637`).

Donc, après ce vol, **les deux axes du corps sont clos** : le rail de sévérité est livré et attesté aux deux
étages, l'axe INTENT_GUARDS est clos (§ 2) et attesté au transport (§ 5). Ce qui reste n'est pas de la dette
de #1381 mais deux tickets qui n'ont jamais été ouverts.

**Décision : `Closes #1381`**, sous deux conditions que le pilote remplit dans l'ordre :

1. Ouvrir les deux tickets de suivi (`gh issue create`) — une classe par ticket, chacun nommant sa classe,
   l'absence de site d'émission, et la citation du vol 1 ci-dessus.
2. Écrire dans le corps de PR une ligne `Tracked in: senara-solutions/mika#<n>` par ticket ouvert.
   `scripts/check-pr-body-consistency.sh` l'exige pour toute phrase de déferral, et c'est ce que
   `pr-body-validation.yml` vérifie.

**Si la gate (a) de ce script refuse le `Closes`** — des sous-issues formelles de #1381 encore ouvertes,
que ce grooming n'a pas pu énumérer (`gh api graphql` est refusé par la permission-policy du bac à sable) —
le pilote **ne force pas** : il bascule sur `Part of #1381`, ajoute la ligne `Tracked in:` nommant les
sous-issues, et dit dans le corps de PR que la fermeture de #1381 est une décision d'opérateur. Un `Closes`
qui ment est plus coûteux qu'un ticket qui reste ouvert un jour de plus.

---

## 9. Acceptance criteria

- **AC1** — `crates/mika-agent/tests/eval/test_p2_ack_suppression_1381.rs` existe, est déclaré dans
  `tests/eval.rs`, et `cargo test -p mika-agent --test eval p2_ack_suppression` passe.
- **AC2** — `mika1381_un_webhook_non_actionnable_ne_part_en_aucun_accuse` fait tourner `run_agent` sur
  `[GitHub] New comment on senara-solutions/mika#1381 …`, avec **deux** réponses de mock fournies (D6), et
  assertionne `trace.llm_call_count == 1` : le garde `webhook_zero_tools` n'a pas tiré, la seconde réponse
  n'a pas été consommée.
- **AC3** — le même test assertionne que le `MessageSender` injecté a observé **zéro** émission, et son
  message d'échec affiche les émissions observées.
- **AC4** — `mika1381_hors_du_domaine_la_garde_exige_toujours_un_outil` fait tourner le même scénario sur
  `[GitHub] Check suite failure on …` et assertionne `trace.llm_call_count > 1`. Son message d'échec dit
  que, s'il tombe, AC2 n'atteste plus rien.
- **AC5** — `mika1381_un_accuse_emis_par_loutil_arrive_sans_etage` utilise `default_tools()` (D7), fait
  appeler le vrai `send_message` au tour 1, et assertionne `emissions.len() == 1` **et** que l'étage observé
  vaut `SEVERITE_ABSENTE`, avec un message nommant `tools/send_message.rs:142` et le corps par défaut de
  `send_with_severity`.
- **AC6** — `SeverityRecordingSender`, `Emission` et `SEVERITE_ABSENTE` sont `pub(crate)` dans
  `test_dispatch_stall_p0_1381.rs`, leur doc-comment nomme le second consommateur, et **aucune copie** du
  type n'est introduite (D3).
- **AC7** — le header de module de `test_p2_ack_suppression_1381.rs` porte les trois recettes de rouge-avant
  de D8, chacune avec sa sortie d'échec recopiée verbatim depuis une exécution réelle, et l'énoncé explicite
  que le site a été restauré à l'identique après chaque injection.
- **AC8** — le header nomme mika#2517 U3 par son commit (`b1d96948`), sa PR (#2530) et sa date
  (2026-09-25), et dit en une phrase ce que ce vol atteste **de plus** que
  `guard_stands_down_on_issue_webhook` : le transport, pas le garde.
- **AC9** — `crates/mika-agent/CLAUDE.md` § *MessageSender Trait* porte la jonction de D9, y compris la
  propriété « un ack passé par l'outil n'a pas d'étage, donc la suppression P2 est une propriété du garde et
  jamais du routage ».
- **AC10** — **aucun fichier sous `crates/mika-agent/src/` ni sous `crates/mika-gateway/src/` n'est
  modifié** par ce vol (D1). `git diff --name-only origin/main` ne montre que `tests/`, `CLAUDE.md` et
  `docs/plans/`.
- **AC11** — les deux tickets de suivi des classes P0 sans site sont ouverts, et le corps de PR porte une
  ligne `Tracked in:` par ticket (§ 8).
- **AC12** — le corps de PR porte `Closes #1381`, ou `Part of #1381` avec la justification explicite du
  repli décrit en § 8.

---

## 10. Definition of Done

- `cargo test -p mika-agent --test eval` vert, y compris `test_webhook_zero_tools_guard` et
  `test_dispatch_stall_p0_1381` (les voisins dont ce vol modifie la visibilité).
- `cargo test` vert sur l'ensemble du workspace.
- `cargo clippy --all-targets` sans avertissement nouveau ; `cargo fmt --check` propre.
- `scripts/verify-pipeline.sh origin/main` vert — y compris U2, qui exige la coexistence de
  `## Acceptance criteria` et `## Definition of Done` dans ce plan.
- `make verify-bundled-skills` vert (aucun bundle n'est touché, le gate reste la preuve).
- Les trois recettes de rouge-avant de D8 **exécutées**, leurs sorties recopiées dans le header, et les
  sites restaurés à l'identique — vérifiable par `git diff` sur `src/`, qui doit être vide (AC10).
- AC1 à AC12 satisfaites.
- Un seul vol, une seule PR, pas de phase différée.
- Pipeline pilote allégé (mika#2694) : pas de `ce-simplify-code`, pas de `ce-code-review` ; le corps de PR
  porte `Code review: delegated to mika-qa (mika#2694)`.

---

## 11. Risques

**R1 — Le test du cœur frôle la tautologie.** Un tour sans appel d'outil n'émet rien ; l'assertion de zéro
émission pourrait être vraie pour une raison qui n'a rien à voir avec le domaine fallthrough. *Mitigation* :
les deux contrôles de D4 ne sont pas optionnels, et la recette R2 (faire rendre `false` au prédicat de
domaine sur tout) est précisément le scénario où le cœur reste vert alors que le moteur est cassé. Le plan
nomme ce risque plutôt que de le masquer.

**R2 — Le double pourrait être aveugle sans que rien ne le dise.** Un `message_sender` mal branché, ou un
registre d'outils avec `StubSendMessage`, rendrait toutes les assertions de zéro émission triviales.
*Mitigation* : D7 (registre nu) + le test 3 + la recette R3.

**R3 — L'élévation de visibilité au vol 3 pourrait être lue comme une inversion de sa note `:93`.**
*Mitigation* : D3 l'explique dans le plan **et** le doc-comment doit le redire au point de modification —
la note refuse un type `#[cfg(test)]` de la crate source, pas un module voisin du même binaire de test.

**R4 — La gate `Closes` pourrait refuser.** Des sous-issues formelles de #1381 encore ouvertes, que ce
grooming n'a pas pu énumérer (`gh api graphql` refusé par la permission-policy). *Mitigation* : le repli
de § 8 est décidé à l'avance et n'exige aucune question à l'opérateur.

**R5 — Un cinquième lecteur rouvrira l'axe INTENT_GUARDS.** C'est arrivé trois fois, dont une après la
fermeture. *Mitigation* : c'est la raison d'être du livrable 4 (D9) ; sans la jonction écrite, ce vol
n'empêche pas sa propre répétition.

---

## 12. Scope boundaries — ce que ce vol ne fait pas

- **Il ne crée aucun site d'émission.** Les classes P0 `recovery-runbook` et `fabrication-detected` partent
  en tickets (§ 8), conformément à la décision du vol 1 ratifiée par l'architecte.
- **Il ne touche ni `webhook_zero_tools_trigger`, ni `is_webhook_fallthrough_domain`, ni le registre
  `INTENT_GUARDS`** (D1, AC10). Ce qu'ils font est déjà ce que #1381 demande.
- **Il ne touche pas la gateway.** `crates/mika-gateway/.../github.rs` traite les webhooks entrants ;
  le corps du ticket le cite par erreur, réfutée dès le vol 1.
- **Il n'étend pas la couverture des six autres sites P0** (`verdict_handler.rs` ×5,
  `ci_failure_handler.rs:451`), attestés à la frontière de l'aide par les vols 1 et 2 et non bout en bout.
  La borne de couverture du vol 3 (D6) tient telle quelle : 1 site P0 attesté bout en bout sur 7. Dire
  autre chose dans le corps de PR serait une surenchère.
- **Il n'ajoute pas de shape au test unitaire du prédicat** (`mika2517_the_fallthrough_domain_stands_the_zero_tools_guard_down`,
  `agent_loop/mod.rs:17775`, 5 shapes). Ajouter une sixième entrée à une liste déjà représentative
  n'atteste rien de plus.
- **Il ne réarme ni ne désarme aucun batching.** Le corps dit *« batch/suppress »* ; le moteur **supprime**
  (le garde se désarme, le tour clôt sans ack) et ne **batche** pas — aucune file, aucune agrégation. La
  suppression est la forme livrée, et c'est la forme que le prompt `self-dev` décrit (« Acknowledge … Stop »).
  Un mécanisme de batch serait un nouveau sous-système, hors de ce ticket.

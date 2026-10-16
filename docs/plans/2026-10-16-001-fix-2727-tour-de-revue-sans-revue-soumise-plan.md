# fix(mika-qa) — un tour de revue qui finit sans `pr review` est re-prompté une fois, sur le signal de soumission et non sur le nom de l'outil

**Ticket :** senara-solutions/mika#2727 (`p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
**Classe :** garde de post-condition EndTurn sur une population conjonctive (événement webhook de revue ∧ `qa-review` chargé) — sœur exacte de la garde mika#2355, à l'autre bout du même tour.

---

## Contexte

`skills/bundled/qa-review/skill.toml` déclare
`required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]`. La garde
`required_tools` de `agent_loop::run_loop` (`crates/mika-agent/src/agent_loop/mod.rs:2095`)
vérifie **qu'un outil de ce nom a été appelé**, jamais ce qu'il a fait. Un
`run_gh` en lecture — `pr list --head …`, `pr view` — satisfait donc le terme
`run_gh` alors que la raison d'être de ce terme est l'**écriture** du verdict sur
la PR. Les deux incidents du ticket sont exactement cette lecture :

| date | tête de #2704 | tour parti | fin de tour | revue soumise |
|---|---|---|---|---|
| 2026-10-15 | `4a216bf7` | 15:11:52Z (`qa_sync_debounce_released`) | `EndTurn` 15:12:10Z, verdict en texte | 17:03:20Z, **relance manuelle** |
| 2026-10-15 | `3f2c3fa4` | 19:11:36Z | trois `run_gh`, aucune revue | 19:17:06Z, **relance manuelle** |

n=2 sur une même PR, deux têtes différentes, deux relances humaines. Le tour n'a
déclenché qu'**un** re-prompt, au pas 0, avant tout appel d'outil : la garde
`required_tools` avait brûlé son budget sur l'absence d'outils, et quand les
outils sont arrivés elle n'avait plus rien à dire sur leur contenu.

**Ce qui existe déjà et ne couvre pas ce cas.** La garde mika#2355
(`qa_build_callback_verdict`) couvre le tour de **callback** de `build_mika` :
sa population est `is_build_callback(msg) ∧ qa-review chargé`
(`crates/mika-agent/src/qa_build_callback.rs:98`). Les deux tours incriminés ne
sont pas des callbacks — ce sont les tours d'**ouverture** de revue, partis d'un
événement webhook `[GitHub] PR …`, et ils n'ont jamais appelé `build_mika`, donc
aucun callback n'a jamais suivi. La garde mika#2355 est restée hors population,
correctement. Le filet périodique `qa_review_reconcile` ne les rattrape pas non
plus : la sixième conjonction de `select_prs_needing_review` **exclut toute PR
portant déjà une revue mika-qa**, et #2704 en portait une (sur une tête
antérieure) — c'est précisément pourquoi les deux incidents ont exigé une
relance manuelle.

---

## Décisions de grooming

### D1 — La population est conjonctive, et inline, comme mika#2355

Le déclencheur est `événement webhook de revue ∧ qa-review chargé`, évalué dans
`run_loop` à côté de `qa_verdict_due` (`agent_loop/mod.rs:1483`), **pas** dans le
registre `INTENT_GUARDS` : celui-ci ne porte qu'un `fn(&str) -> bool` sur le
message, incapable d'exprimer le second terme. C'est la raison pour laquelle
mika#2355 est déjà inline, et la raison vaut mot pour mot ici.

Les deux termes comptent. Le premier seul (« un webhook de PR est arrivé »)
re-prompterait **mika-dev** — qui reçoit `pull_request.closed`,
`check_suite.completed` et `pull_request_review.submitted` — pour poster une
revue qu'il ne doit à personne : on échangerait le loop-breaker QA contre un
loop-breaker dev, le coût que le doc-comment de `qa_verdict_required` nomme
explicitement. Le second seul (« `qa-review` est chargé ») est quasi toujours
vrai : ce skill est `always_on = true`, donc présent sur **tous** les tours
mika-qa, y compris une question d'opérateur en conversation.

Le premier terme se lit par le **lecteur unique déjà en place**,
`parse_pr_action_event` (`server/webhook_queue_v2.rs`), restreint aux quatre
actions que la gateway route vers mika-qa : `opened`, `synchronize`,
`review_requested`, `ready_for_review` (`mika-gateway/src/github.rs`, et
`mika#1822` pour la quatrième). `closed` est exclu — il va à mika-dev. Aucun
nouvel extracteur de la grammaire de fil n'est écrit : c'est la doctrine
mika#2158 (*une grammaire de fil dupliquée est la façon dont deux lecteurs
divergent en silence*) et l'avertissement explicite de
`webhook_dispatch.rs::webhook_event_target`.

### D2 — Le signal de soumission est celui que la session pose déjà

Le ticket l'exige nommément (« Le signal de soumission est celui que la session
pose déjà (`pr_review_posted`), et non le nom de l'outil »). On réutilise
**`qa_build_callback::pr_review_posted_in_turn`** (`qa_build_callback.rs:117`) —
déjà public, déjà le prédicat de satisfaction de la garde sœur, déjà épinglé
comme équivalent au prédicat d'early-accept `has_successful_pr_review`
(#695/#821) par `mika2355_the_satisfied_predicate_matches_the_early_accept_one`.
Son terme `s.success` s'accorde avec le `review_landed` du handler depuis
mika#2237. Un second prédicat écrit ici serait un second lecteur du même fait.

**Ce qu'on ne lit PAS : la carte de session `pr_reviews_posted` (#821).** Sa clé
de dédup ne porte **aucune tête** (`format_pr_dedup_key`), donc une revue rendue
sur une tête antérieure y suppose la garde satisfaite sur une tête nouvelle —
exactement l'incident mesuré, deux fois. La lire reviendrait à armer la garde
sur la condition qui l'empêche de voir le défaut (motif mika#2272 : *une
condition d'armement insatisfiable se lit comme une flotte en bonne santé*).

### D3 — La réserve que le ticket omet : un `build_mika` lancé termine légitimement son tour

Le flux nominal d'une revue **finit son tour sans revue soumise**, et c'est
voulu : le tour lance `build_mika` en asynchrone et rend la main pour attendre
le callback — l'en-tête de `qa_build_callback.rs` le dit dans ces termes, et
c'est là que la garde mika#2355 prend le relais. Une garde qui ignorerait cette
réserve re-prompterait **tous** les tours de revue sains.

La satisfaction est donc disjonctive :
`pr_review_posted_in_turn(summaries) || build_dispatched_in_turn(summaries)`,
le second terme étant `s.name == BUILD_MIKA_TOOL && s.success`. Les deux
incidents tombent bien dans la population restante : le ticket établit qu'aucun
des deux tours n'a appelé `build_mika` (c'est même l'un des outils que le
re-prompt du pas 0 réclamait).

### D4 — Les deux sorties EndTurn, pas une

La garde est posée aux **deux** chemins de sortie EndTurn de `run_loop` : texte
non vide (`agent_loop/mod.rs:3240`) et miroir texte vide
(`agent_loop/mod.rs:4206`). mika#2515 a mesuré que l'EndTurn nu est la moitié la
plus probable, et un signal posé à un seul site est aveugle par construction.
Les deux blocs sont octet pour octet identiques au message de log près, comme le
sont déjà les deux blocs mika#2355 qui les précèdent.

### D5 — Exclusivité avec la garde mika#2355, par construction et par test

`is_build_callback(msg)` exige le préfixe `[callback: long_running:build_mika]` ;
la nouvelle population exige `[GitHub] PR `. Préfixes disjoints : les deux gardes
ne peuvent pas être dues sur un même message. Épinglé par un test d'exclusivité
dans le style mika#2653 — la propriété est ce qui garantit qu'on n'a pas écrit un
second budget de re-prompt sur le même tour.

### D6 — AC2 part en vol 2, et la raison est la tête

AC2 demande un événement nommé « avec le dépôt, la PR et **la tête** ». Le
**SHA de tête n'est sur aucun fil** : la gateway écrit
`[GitHub] PR {action}: {repo}#{number} — {title} (branch: {branch})\n{url}`
(`mika-gateway/src/github.rs:505`), le caveat est documenté à
`server/webhook_queue_v2.rs:62` (« `head_sha` is NOT in the gateway text »), et
ni la clé de `qa_pr_sync_observed` (`pr:{repo}#{pr}`, sans SHA) ni la clé de
dédup de session n'en portent. La meilleure identité de tête disponible est la
**branche**, présente dans le segment `(branch: …)` des formes d'action — et
l'obtenir correctement veut dire **étendre le lecteur unique**
(`PR_ACTION_RE` + `parse_pr_action_event` rendant une struct nommée avec
`head_branch: Option<&str>`, **absent et jamais vide** quand la forme ne porte
pas de branche, motif mika#2331), puis migrer ses appelants. Ce travail-là, plus
l'émetteur d'audit et ses tests, ne tient pas sous la borne par vol avec AC1 et
AC3 (voir § *Taille estimée*). Il part en vol 2, nommé par la ligne
`Tracked in:`.

Le découpage est propre dans l'ordre : AC1 est le correctif (le tour ne finit
plus en silence), AC2 est l'observabilité du cas où le re-prompt échoue **aussi**
— et cet événement ne peut pas être défini avant que la garde dont il mesure
l'échec existe.

---

## Conception

### Nouveau module `crates/mika-agent/src/qa_review_submission.rs`

Module frère de `qa_build_callback.rs`, même grammaire : une population, un
label, une consigne, un prédicat de satisfaction, un complémentaire.

```rust
/// Les quatre actions de PR que la gateway route vers mika-qa.
pub const REVIEW_TRIGGERING_ACTIONS: &[&str] =
    &["opened", "synchronize", "review_requested", "ready_for_review"];

/// Une revue est-elle **due** sur ce tour ? Conjonction ; les deux termes comptent.
pub fn review_submission_required(
    msg: &str,
    skill_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> bool

/// Label pour `intent_guard_retries` (budget de re-prompt unique).
pub const REVIEW_SUBMISSION_LABEL: &str = "qa_review_submission";

/// La revue a-t-elle été soumise, ou un build a-t-il été lancé (réserve D3) ?
pub fn review_submission_satisfied(summaries: &[ToolCallSummary]) -> bool

/// Complémentaire exact de la garde : population due, budget consommé, rien posté.
/// Consommé par le vol 2 (AC2) ; livré ici parce qu'il est le pendant du budget.
pub fn submission_unmet_after_retry(
    due: bool,
    retries: &HashSet<&'static str>,
    summaries: &[ToolCallSummary],
) -> bool

/// Consigne de correction : nomme l'outil ET la ligne attendue.
pub const REVIEW_SUBMISSION_CORRECTION: &str = "[mika-engine] …";
```

La consigne suit la leçon du doc-comment de `QA_VERDICT_REQUIRED_CORRECTION` :
elle nomme `run_gh` avec `pr review`, rappelle que **la revue postée est la
source de vérité et que le texte de la réponse n'en est qu'un miroir**, nomme la
ligne `VERDICT:` et ses six valeurs (le guard de suffixe mika#899 les exige de
toute façon), et nomme ce qui ne vaut pas soumission (`update_task_status`,
`send_message`, un `run_gh` en lecture). Elle dit aussi la réserve D3 : si le
build n'a pas encore tourné, lancer `build_mika` est une fin de tour valide.

### Câblage dans `agent_loop/mod.rs`

1. À côté de `qa_verdict_due` (ligne 1483), même forme, même source
   (`user_input_text` + `loaded_skill_names`) :
   ```rust
   let qa_review_due =
       crate::qa_review_submission::review_submission_required(&user_input_text, loaded_skill_names);
   ```
   Ce site est le bon par construction : `loaded_skill_names` est calculé avant,
   et l'ordre est déjà épinglé par les tests structurels de la ligne 11554.
2. Aux deux sorties EndTurn, après le bloc mika#2355 (qui ne peut pas être dû en
   même temps, D5) :
   ```rust
   if !skip_remaining_guards
       && matches!(response.stop_reason, LlmStopReason::EndTurn)
       && qa_review_due
       && !intent_guard_retries.contains(crate::qa_review_submission::REVIEW_SUBMISSION_LABEL)
       && !crate::qa_review_submission::review_submission_satisfied(&all_tool_summaries)
   {
       intent_guard_retries.insert(crate::qa_review_submission::REVIEW_SUBMISSION_LABEL);
       warn!(step, label = mode.label(),
             intent_guard = crate::qa_review_submission::REVIEW_SUBMISSION_LABEL,
             "qa-review turn ending without a submitted review — re-prompting");
       request.messages.push(/* blocs assistant */);
       request.messages.push(/* user: REVIEW_SUBMISSION_CORRECTION */);
       continue;
   }
   ```
   Budget unique via `intent_guard_retries` : la garde ne peut pas boucler.

### Chemin de test (AC3)

Le tour incriminé est un tour **webhook**, et un tour webhook entre par
`server::handlers::run_agent_for_message` → `run_agent` — c'est-à-dire le mode
**conversation**, pas le mode silencieux (`SilentTrigger` n'a aucune variante
`Webhook`). `EvalHarness::run("<texte webhook>")` est donc le pilote exact du
chemin de production, comme le font déjà `test_webhook_zero_tools_guard.rs` et
`grounding_regressions/qa_review_required_tools_retry_duplicate.rs`. Pas de
`run_silent_agent` ici — c'est la différence avec
`test_qa_build_callback_verdict_2355.rs`, dont la population *est* un callback.

Deux tests, un fixture commun (`qa-review` avec ses `required_tools` réels et
`always_on`), un stub `run_gh` enregistreur, un stub `qa_pr_view` :

- **AC3a** — séquence : `qa_pr_view` → `run_gh pr list --head …` → texte de
  verdict. Attendu : la garde tire. Assertion sur
  `trace.captured_requests` — la dernière requête porte un message `User` qui
  contient `REVIEW_SUBMISSION_CORRECTION`, et `llm_call_count` vaut un de plus
  que la séquence sans garde.
- **AC3b — contrôle positif dans le même fichier** : séquence identique sauf
  `run_gh pr review --approve --body "…\nVERDICT: pass"`. Attendu : **aucune**
  correction injectée, aucun re-prompt. C'est l'assertion qui distingue « la
  garde marche » de « la garde tire sur tout » (motif mika#2205).

Un troisième test, structurel et bon marché : l'exclusivité D5 — sur le message
de callback, `review_submission_required` est faux et `qa_verdict_required` est
vrai ; sur le message webhook, l'inverse. Il vit dans le `mod tests` du module,
pas dans l'eval.

---

## Étapes d'implémentation

1. Écrire `crates/mika-agent/src/qa_review_submission.rs` (population, label,
   consigne, satisfaction, complémentaire) + son `mod tests` : prédicats sur les
   quatre actions routées, refus sur `closed`, refus sans `qa-review`, réserve
   D3, exclusivité mika#2355, contrôle négatif sur une entrée fabriquée.
   `cargo test -p mika-agent qa_review_submission` vert avant de toucher la
   boucle.
2. Déclarer le module dans `lib.rs`.
3. Câbler `qa_review_due` ligne ~1483, puis les deux blocs de garde aux deux
   sorties EndTurn. `cargo test -p mika-agent agent_loop` vert.
4. Écrire `crates/mika-agent/tests/eval/test_qa_review_submission_2727.rs`
   (AC3a + AC3b) et le déclarer dans `tests/eval/main.rs`
   (`test_eval_modules_declared.rs` l'exige).
5. Suite complète, `cargo clippy`, `cargo fmt`, lints du dépôt,
   `scripts/verify-pipeline.sh origin/main`.
6. Corps de PR : la réserve D3 **avec sa raison**, le refus de lire
   `pr_reviews_posted` **avec sa raison** (la clé sans tête), le renvoi d'AC2 au
   vol 2 avec la ligne `Tracked in:`, la sonde post-déploiement, et
   `Code review: delegated to mika-qa (mika#2694)`.

---

## Contrat de vérification

- **W1** — `cargo test -p mika-agent qa_review_submission` : les prédicats, la
  réserve D3, l'exclusivité D5, le contrôle négatif.
- **W2** — `cargo test -p mika-agent --test eval qa_review_submission_2727` :
  AC3a tire, AC3b ne tire pas.
- **W3** — `cargo test -p mika-agent --test eval qa_build_callback_verdict_2355`
  **vert sans modification d'assertion** : la garde sœur n'a pas changé de
  comportement sur sa propre population.
- **W4** — `cargo test -p mika-agent --test eval qa_review_required_tools` vert
  sans modification : la garde `required_tools` et le dédup de session #821 sont
  intacts (ce vol n'en retire rien, il ajoute une post-condition **après** eux).
- **W5** — suite complète, `cargo clippy`, `cargo fmt --check`, lints du dépôt,
  `scripts/verify-pipeline.sh origin/main` verts.

---

## Fire-Disposition

Ce vol livre trois détecteurs : la garde EndTurn (chemin de succès = « aucune
violation trouvée »), le test d'exclusivité D5, et les deux tests eval. Option
retenue : **(a) exception nommée en allowlist**, dans sa forme à population
vide — la forme idiomatique du dépôt (mika#2425 V1, mika#2025 V10).

- **La seule exception est nommée, et elle est dans le code, grep-visible** : le
  second terme de `review_submission_satisfied`
  (`build_dispatched_in_turn`), qui exempte le tour ayant lancé `build_mika`.
  Elle nomme la donnée précise (l'outil `build_mika`, constante
  `qa_build_callback::BUILD_MIKA_TOOL`), elle référence son ticket de suivi
  (mika#2355, qui **est** le détecteur qui reprend la main sur le tour de
  callback suivant), et son doc-comment dit pourquoi elle existe : sans elle,
  tous les tours de revue sains seraient re-promptés.
- **L'assertion auto-nettoyante** est celle de W3 : si mika#2355 cesse de
  couvrir le tour de callback, l'exception devient stale et
  `test_qa_build_callback_verdict_2355` rougit — l'exemption est donc gagée sur
  l'existence du détecteur qui la justifie, pas sur une promesse.
- **Aucune violation préexistante à exempter.** Les violations sont des *tours*,
  pas des artefacts commités : il n'y a rien à mettre en allowlist, et
  l'allowlist du test d'exclusivité **part vide et reste vide** — quand elle
  rougit, la résolution est de router le site, jamais d'ajouter une entrée
  (doctrine mika#2201). Le test porte un **comptage anti-vacuité** et un
  **contrôle négatif sur une entrée fabriquée**, pour que « le scan n'a rien
  trouvé » reste distinguable de « le scan n'a rien regardé » (mika#2205).
- **La garde atterrit ARMÉE, sans kill-switch, et c'est délibéré.** Précédent
  exact : la garde mika#2355 n'en a pas non plus. Le coût d'un faux positif est
  borné par construction à **un** tour LLM supplémentaire (`intent_guard_retries`
  est un budget unique), à comparer aux deux relances humaines mesurées. Un
  `MIKA_*` de plus serait un levier qu'on ne saurait pas quand actionner.

---

## Risques nommés

- **R1 — faux positif sur une revue déjà postée dans une invocation antérieure
  de la même session.** `all_tool_summaries` est par invocation de `run_loop`.
  Ne bite pas : le re-prompt `required_tools` est un `continue` **dans** la
  boucle (ligne 2122), donc les summaries s'accumulent au travers, et mika-qa
  n'est pas `singleton` — chaque message webhook frappe un UUID de session neuf
  (`server/handlers.rs:1711`), donc il n'existe pas de tour antérieur dans la
  session. Résidu : le rejeu #528, hors population mesurée.
- **R2 — un tour mika-qa de conversation (`mika ask`) re-prompté à tort.** Ne
  bite pas : le premier terme exige un message commençant par `[GitHub] PR ` et
  portant l'une des quatre actions routées. Une question d'opérateur est hors
  population, et c'est précisément ce que le test de refus épingle.
- **R3 — le re-prompt poste une revue en double.** Ne bite pas : le dédup de
  session #821 reste en place et est le dernier mot (W4). La garde demande une
  soumission ; elle ne contourne pas le refus de doublon.
- **R4 — la consigne fait poster un verdict non étayé.** La consigne renvoie au
  workflow `qa-review` (relire le plan, exécuter les AC) et le guard de suffixe
  mika#899 refuse un `--body` sans ligne `VERDICT:`. Elle ne doit pas prescrire
  une valeur de verdict ; elle prescrit **l'acte de soumettre**.
- **R5 — la marge sous la borne par vol est mince** (23 lignes, § *Taille
  estimée*). L'ordre des étapes la rend visible tôt : si le module dépasse son
  estimation à l'étape 1, le test d'exclusivité D5 part en vol 2 avec AC2 (il
  est structurel, pas comportemental) avant que la fin de pipeline soit engagée.

---

## Hors périmètre

- **AC2 — l'événement nommé `qa_turn_ended_without_review`** et, avec lui,
  l'extension de `PR_ACTION_RE` / `parse_pr_action_event` portant
  `head_branch: Option<&str>`, la migration de ses appelants, la ligne
  `audit_events` (`tool_name = 'qa_turn_ended_without_review'`, sole-writer, sur
  le modèle de `REVIEW_HEAD_SUPERSEDED_EVENT`), et l'émission aux trois points
  de sortie (deux EndTurn + coupure, avec une `cause` distinguant
  `retry_exhausted` de `cut_off_*` comme le fait mika#2515). Vol 2 — voir D6
  pour la raison (la tête n'est pas sur le fil) et § *Taille estimée* pour la
  borne.
- **Élargir la population de `qa_review_reconcile`** à « déjà revue sur une tête
  antérieure ». Sa sixième conjonction l'exclut, et c'est pourquoi les deux
  incidents ont exigé une relance manuelle — mais c'est un filet périodique, un
  autre mécanisme, et le ticket demande une garde de tour.
- **Étendre le filet moteur mika#2368/#2515 (`hold[review]`)** à cette
  population. AC2 demande un **événement**, pas un verdict posté : poster un
  `hold[review]` depuis le dispatcher sur un tour d'ouverture de revue est une
  décision de produit, pas une observabilité.
- **Threader le SHA de tête de bout en bout** (gateway → fil → lecteur). Classe
  mika#1869 ; prérequis d'une identité de tête exacte, et bien au-delà de ce
  ticket.

---

## Mesure post-déploiement

Sonde, sur `$MIKA_LOG_FILE` / `server.log` :

```
grep qa_review_submission server.log
```

- **Régime attendu : NON vide, et chaque ligne est un incident évité.** Chaque
  occurrence est un tour qui allait finir en silence et qui a été re-prompté.
  C'est la mesure directe de la population n=2 du ticket.
- **Zéro ligne sur 7 jours avec des revues rendues** ⇒ ne pas conclure « la
  flotte est saine » : vérifier d'abord que le binaire servi porte le correctif
  (classe mika#2340, `cat ~/.mika/skills/.manifest-writer`), puis qu'au moins un
  tour de revue webhook a tourné. Une garde que personne n'a exercée se lit
  comme une garde qui marche (mika#2205).
- **Halte — rafale de lignes sur des tours qui aboutissaient quand même** ⇒ la
  réserve D3 est trop étroite (un autre chemin de fin de tour légitime existe) :
  nommer ce chemin et l'ajouter à l'exception **nommée**, jamais désarmer la
  garde.
- **Halte — une ligne sur un tour `mika-dev`** ⇒ le second terme de la
  conjonction a cédé : lire `loaded_skill_names` du tour avant de toucher au
  premier terme.

---

## Definition of Done

- `crates/mika-agent/src/qa_review_submission.rs` existe, déclaré dans `lib.rs`,
  et porte la population conjonctive, le label, la consigne, le prédicat de
  satisfaction avec la réserve D3 et le complémentaire.
- La garde est câblée aux **deux** sorties EndTurn de `run_loop`, avec budget
  unique via `intent_guard_retries`.
- Le signal de soumission est `qa_build_callback::pr_review_posted_in_turn`,
  réutilisé et non réécrit ; la carte de session `pr_reviews_posted` n'est pas
  lue, et la raison est au site.
- W1 à W5 verts ; W3 et W4 verts **sans modification d'assertion**.
- La section `## Fire-Disposition` de ce plan est honorée dans le code :
  l'exception D3 est grep-visible, elle nomme `BUILD_MIKA_TOOL` et mika#2355, et
  son doc-comment dit pourquoi elle existe.
- Le corps de PR porte : la réserve D3 avec sa raison, le refus de lire
  `pr_reviews_posted` avec sa raison, la déviation d'AC2 (la tête n'est pas sur
  le fil) avec sa ligne `Tracked in:`, et la sonde post-déploiement avec ses deux
  haltes.
- `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR ; ni
  `ce-simplify-code` ni `ce-code-review` invoqués.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2727 :

- AC1 : un tour qa-review qui atteint `EndTurn` sans `gh pr review` réussi est
  re-prompté une fois avec la consigne nommée « soumettre la revue ». Le signal
  de soumission est celui que la session pose déjà (`pr_review_posted`), et non
  le nom de l'outil.
- AC2 : si le re-prompt échoue aussi, le tour émet un événement nommé
  (`qa_turn_ended_without_review`, avec le dépôt, la PR et la tête), visible pour
  l'opérateur.
- AC3 : tests. (a) Un tour qui appelle seulement `run_gh pr list` puis finit est
  re-prompté. (b) Contrôle positif dans le même test : un tour qui appelle
  `run_gh pr review --approve` n'est pas re-prompté.
- Taille : à estimer tests compris.

**Couverture par vol.** AC1 et AC3 sont dans ce vol. **AC2 est le vol 2** (D6) :
le SHA de tête n'est sur aucun fil, et l'identité de tête disponible — la branche
— exige d'étendre le lecteur unique de la grammaire `[GitHub] PR …` et de migrer
ses appelants, ce qui ne tient pas sous la borne par vol avec AC1 et AC3.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_review_submission.rs` (production) | 110 |
| `crates/mika-agent/src/qa_review_submission.rs` (`mod tests`) | 90 |
| `crates/mika-agent/src/agent_loop/mod.rs` (population + deux blocs de garde) | 56 |
| `crates/mika-agent/src/lib.rs` (déclaration du module) | 1 |
| tests (`crates/mika-agent/tests/eval/test_qa_review_submission_2727.rs`) | 150 |
| tests (déclaration dans `crates/mika-agent/tests/eval/main.rs`) | 1 |

Total estimé : 408 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Deux vols, et c'est la borne par vol qui mord — pas le seuil par PR.** 408
contre une borne de 431 : le vol courant repasse dessous, avec 23 lignes de
marge. Le seuil par PR de 1000 n'est pas en cause et ne l'aurait jamais été ;
c'est la somme des deux vols (≈ 640) qui dépassait la borne.

**Pourquoi ce vol tient.** Aucune structure nouvelle hors du module : pas de
migration, pas de type public hors de ses cinq éléments, pas de table, pas de
variable d'environnement, pas de changement de grammaire de fil. Le câblage dans
`agent_loop/mod.rs` est un **miroir** de blocs existants aux mêmes lignes
(3240 et 4206), ce qui est le travail le plus prévisible du dépôt. Les deux tests
eval reprennent le fixture et les stubs d'un voisin direct
(`qa_review_required_tools_retry_duplicate.rs`) sur un harnais déjà en place.
La marge de 23 lignes est mince et nommée en R5, avec le levier à actionner si
elle est consommée.

**Pourquoi le vol 1 est mergeable seul.** Il est le correctif : après lui, un
tour de revue ne finit plus en silence — il est re-prompté une fois, et le
re-prompt porte la consigne qui nomme l'acte manquant. Le vol 2 n'ajoute pas de
correctif, il ajoute la **visibilité** du cas résiduel (le re-prompt échoue
aussi), qu'aujourd'hui rien ne mesure et que rien ne mesurera moins bien après.

Tracked in: un ticket de suivi à ouvrir pour le vol 2 — AC2 : l'événement nommé
`qa_turn_ended_without_review` (ligne `audit_events` sole-writer + WARN, avec sa
`cause`), l'extension de `PR_ACTION_RE` / `parse_pr_action_event` à
`head_branch: Option<&str>` (absent, jamais vide) et la migration de ses
appelants, l'émission aux trois points de sortie, et ses tests.

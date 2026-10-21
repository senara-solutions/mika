# Un site d'émission P0 pour la classe `fabrication-detected` — la récidive réveille, la première prise non

- **Ticket** : senara-solutions/mika#2867 (`Closes #2867`)
- **Branche** : `feat/2867/notifications-un-site-d-mission-p0-pour`
- **Type** : feat
- **Date** : 2026-10-21
- **Amont** : mika#1381 vols 1 à 4 (rail de sévérité, livré et attesté — `#2813`, `#2846`, `#2856`)
- **Projet** : *Loop Trustworthiness*, phase 1b

---

## Ce que la lecture du code établit, et qui n'était pas dans le corps du ticket

Le ticket demande de décider quatre choses avant d'écrire une ligne. Les quatre réponses sont dans
le code, et trois d'entre elles y sont **déjà écrites par un commentaire antérieur**.

### 1. Le prédicat « fabrication qui survit au garde » a déjà un nom dans ce fichier

Chaque garde de la famille fabrication accorde **un seul** re-prompt, latché par
`intent_guard_retries` / `post_condition_retries`. Une fois le label inséré, le garde ne peut plus
tirer : si la seconde réponse viole le même prédicat, elle **est acceptée et part**. Sept sites
ont déjà posé la branche d'observabilité qui nomme cet état — la famille `_uncorrected` :

| ligne | événement |
|---|---|
| `agent_loop/mod.rs:2472-2480` | milestone-close-claim (sans champ `event`, le plus ancien) |
| `agent_loop/mod.rs:2854` | `guard.false_local_hosting_claim_uncorrected` |
| `agent_loop/mod.rs:2976` | `guard.unactioned_frequency_promise_uncorrected` |
| `agent_loop/mod.rs:3080` | `guard.response_language_drift_uncorrected` |
| `agent_loop/mod.rs:3172` | `guard.time_of_day_greeting_mismatch_uncorrected` |
| `agent_loop/mod.rs:3332` | `guard.testimony_access_proposal_uncorrected` |
| `agent_loop/mod.rs:3837` | `guard.unacknowledged_send_failure_uncorrected` (+ une ligne `audit_events`) |

Le plus ancien (`:2466-2471`) dit mot pour mot ce que ce ticket demande :

> *« we let it through (per the single-retry contract shared with the completion-claim guard #483), but emit a structured warn so the operator can grep for "repeat-fabrication" patterns that justify widening the budget or **escalating**. »*

Et le commentaire de `:2822-2830` nomme l'angle mort de façon décisive :

> *« Une SECONDE fausse déclaration dans le même tour partirait silencieusement et serait indistinguable d'un tour sain. C'est l'angle mort que la porte Fire-Disposition (mika#1574) existe pour fermer. »*

**Le prédicat du ticket est donc la récidive, et il est déjà la convention de la maison.** Ce vol
n'invente pas de prédicat : il ajoute la branche manquante et y attache la seule conséquence que la
famille n'a jamais eue — une notification.

### 2. Le garde que le ticket nomme est précisément celui qui n'a PAS sa branche

Le corps du ticket nomme l'`assert-grounded guard`. C'est `agent_loop/mod.rs:3610-3663`
(`ASSERT_GROUNDED_LABEL`, détecteur `detect_affirmative_state_claim` + prédicat de satisfaction
`assert_grounded_satisfied`) : il attrape exactement « affirmer un état système aval sans résultat
d'outil ». Et il est **absent du tableau ci-dessus** : `grep -n "_uncorrected"
crates/mika-agent/src/agent_loop/mod.rs` ne le rend pas. Sa récidive part aujourd'hui en silence
total — ni log, ni ligne d'audit, ni notification.

C'est ce qui rend le périmètre de ce vol lisible : **un garde, sa branche de récidive, et son
étage.** Les six autres sites `_uncorrected` ont déjà leur log ; leur rattachement à la classe
P0 est une décision de produit distincte, renvoyée par le D8.

### 3. Le site de notification à reprendre est dans ce fichier, pas dans un handler

Le ticket cite `send_notification(sender, severity, message)` de `verdict_handler.rs:2722` comme
« la forme à reprendre ». **Lecture faite, ce n'est pas la bonne forme pour ce site**, et le D4 le
tranche : ce helper existe pour partager `report_notification_outcome` entre **deux** étages dans un
module de handler. Le site amiral de mika#1381, qui vit dans le même fichier que notre branche
(`agent_loop/mod.rs:4296-4341`), appelle directement :

```rust
let _ = sender
    .send_with_severity(&notification, NotificationSeverity::P0)
    .await;
```

`send_with_severity` **a déjà** la sévérité en seconde position et sans défaut — la propriété que
le ticket réclame est portée par la signature du trait, pas par le helper. `NotificationSeverity`
est déjà importé dans `agent_loop/mod.rs:37`.

### 4. Le motif d'attestation est réutilisable, pas à recopier

`tests/eval/test_dispatch_stall_p0_1381.rs` porte le double à **double surcharge** et la sentinelle
`SEVERITE_ABSENTE`, et `EvalHarness::builder()` expose déjà `.message_sender(...)`. Le D5 réutilise
ce double au lieu de le recopier — le même geste que le vol 3 de mika#1381 a fait sur
`tools_with_dispatch_stubs()`.

---

## L'état interdit que ce vol attrape

Un agent affirme un état système que rien ne confirme. Le garde tire, re-prompte, et l'agent
**réaffirme la même chose**. Le budget est épuisé : la réponse part telle quelle, à l'utilisateur
ou dans un tour silencieux où il n'y a pas même d'utilisateur. Aujourd'hui, l'opérateur n'apprend
rien — ni log dédié, ni notification. La fabrication a traversé le garde et le garde n'a pas dit
qu'il avait perdu.

Deux formes doivent rougir :

1. **La récidive ne produit rien.** Le site est atteint, la fabrication est acceptée, et aucune
   ligne ne le dit.
2. **La notification part sans son étage.** Elle sort, mais par `MessageSender::send` — dont
   `send_with_severity` est le corps par défaut (`messaging.rs:100-106`,
   `self.send(text).await`). Le message part, la P0 devient une P-rien, la surface jamais muette
   n'est jamais atteinte, **et rien ne compile en erreur**. C'est exactement la classe que le vol 3
   de mika#1381 a fermée sur le site amiral, et elle se rouvre à chaque site neuf.

Et une forme doit rester verte : **la première prise ne réveille personne.** Un garde qui tire et
corrige fait son travail nominal ; le notifier serait l'« alarm-into-void » du diagnostic #4 du
corps de mika#1381, vu par l'autre bout.

---

## Décisions

### D1 — Le prédicat est la récidive du garde `assert_grounded`, et rien d'autre

La branche ajoutée est la jumelle exacte de `:2839-2858` (false-local-hosting), insérée
**immédiatement après** la branche du garde, à `:3664`, avant le garde #1645 :

```rust
if matches!(response.stop_reason, LlmStopReason::EndTurn)
    && intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)
    && !assert_grounded_uncorrected_notified
    && let Some(claim) = detect_affirmative_state_claim(&text)
    && !assert_grounded_satisfied(&claim, &all_tool_summaries)
{ … }
```

Trois propriétés, toutes portées par le prédicat et non par un commentaire :

- `intent_guard_retries.contains(...)` ⇒ **le garde a déjà tiré ce tour**. La première prise ne
  peut pas atteindre cette branche : au tour 1 le label n'est pas encore inséré, et le garde lui-même
  fait `continue`. C'est la soustraction que le point 2 du ticket demande, et elle est
  structurelle, pas un seuil réglé à la main.
- La ré-exécution de `detect_affirmative_state_claim` + `assert_grounded_satisfied` ⇒ la seconde
  réponse viole **encore**. Un agent qui a honoré la correction ne produit rien.
- Pas de second re-prompt. La famille accorde un re-prompt ; un garde qui divergerait sur ce point
  deviendrait celui que tout le monde relit pour comprendre pourquoi (la raison écrite à
  `:2827-2830`).

### D2 — La soustraction ne peut pas être mesurée depuis ce bac à sable, donc elle est livrée désarmée

Le point 2 du ticket ordonne : « Compter d'abord : combien de fois ce garde tire par jour sur le
trafic réel, avant de câbler quoi que ce soit. » Le trafic réel vit dans
`/var/log/mika/server.log` sur l'hôte ; le bac à sable de dispatch ne le monte pas. Le compte ne
peut donc pas précéder ce vol — et prétendre l'avoir fait serait la fabrication même que ce ticket
remonte.

Ce que ce vol fait à la place, et qui est l'ordre que le ticket demande, exécuté dans le bon sens :

1. **La détection atterrit armée.** Sa seule conséquence inconditionnelle est une ligne
   `warn!` structurée, `event = "guard.assert_grounded_uncorrected"`. Un log ne noie aucune
   surface, et c'est **l'instrument de comptage** que le ticket réclame : la sonde du D7 le lit.
2. **La notification atterrit désarmée**, derrière `MIKA_FABRICATION_P0_NOTIFY`, par défaut
   `false`. L'armement est un changement de configuration, pas un déploiement.
3. Le suivi qui arme après la fenêtre de mesure est nommé par le D8.

Le `grep` de borne supérieure disponible sans l'hôte : la population de la récidive est bornée par
celle de la première prise, qui est **déjà** comptée depuis mika#953 par
`event = "guard.assert_grounded"`. La récidive en est un sous-ensemble strict — elle exige deux
violations consécutives du même prédicat dans un seul tour. On ne connaît pas le facteur ; on sait
qu'il est inférieur à 1, et c'est pourquoi le compteur est armé et l'alarme non.

### D3 — La porte est une clé `Settings`, pas une lecture d'environnement au point d'émission

`agent_loop/mod.rs` ne contient **aucun** `std::env::var` (vérifié : `grep -n "std::env::var\|env::var("` ⇒ vide). La
convention du dépôt est le registre `CONFIG_KEYS` de `mika-common/src/config.rs`. La clé suit
`operational_partner` (`config.rs:313-320`, `:1237-1241`) trait pour trait :

```rust
ConfigKeyInfo {
    key: "fabrication_p0_notify",
    backend: ConfigBackend::File,
    env_var: Some("MIKA_FABRICATION_P0_NOTIFY"),
    secret: false,
    description: "Notify the operator (P0) when a fabrication-class guard's \
                  re-prompt budget is exhausted and the claim persists (default: false)",
},
```

plus `#[serde(default)] pub fabrication_p0_notify: bool`, la ligne de `test_defaults()`
(`:2779` voisine), la ligne du `Debug` manuel (`:2918` voisine) et l'arme de
`get_effective_value` (`:563`, modèle `dashboard_enabled`) pour que `mika config get` la rende.

**Délibérément hors de `SETTABLE_CONFIG_KEYS`** (`mika-agent/src/config_keys.rs`), comme
`operational_partner` : un interrupteur d'armement d'alarme opérateur n'est pas une clé qu'un agent
doit pouvoir basculer depuis une conversation.

### D4 — La porte voyage en paramètre de `run_loop`, pas en champ de `ToolContext`

`ToolContext` est déclaré dans `tools/mod.rs:101` et construit depuis une vingtaine de sites à
travers le crate (`test_utils.rs`, `server/handlers.rs`, `server/a2a.rs`, `tools/send_message.rs`,
`tools/toggle_skill.rs`, `skills/builtin_handlers.rs`, …). Y ajouter un champ coûterait vingt
éditions pour un fait qu'aucun outil ne lit.

La doctrine est écrite dans la signature de `run_loop`, au paramètre `tenant_language`
(`agent_loop/mod.rs:1442-1444`) :

> *« A parameter on the model of `loaded_skill_names` rather than a field on `ToolContext`: this is a *turn* fact read by one guard, not a capability the tools need. »*

Et le motif de résolution existe déjà pour un booléen dérivé de `Settings` :
`store_llm_calls: bool` (`run_loop`, `:1478`), résolu aux trois points d'entrée
(`:6196`, `:7276`, `:7979`) par `params.settings.is_none_or(|s| s.store_llm_calls)`.

Une seule différence, et elle est le cœur du D2 : la résolution utilise
**`is_some_and`**, pas `is_none_or`. `Settings` absent ⇒ **désarmé**. Un chemin qui n'a pas de
configuration ne doit pas réveiller l'opérateur.

### D5 — Le site appelle `send_with_severity` directement, et il n'y a pas de helper

Pas de `fn notify_fabrication_detected(...)`. Trois raisons :

- La propriété que le ticket demande — « la sévérité est en seconde position et n'a pas de défaut,
  pour qu'oublié de classifier ne soit pas un état atteignable » — est **déjà portée par la
  signature du trait** (`messaging.rs:100-106`). Un helper ne l'ajoute pas.
- `send_notification` de `verdict_handler.rs:2722` existe pour partager
  `report_notification_outcome` entre **deux** étages d'un même module. Ici : un site, un étage.
  Un helper à appelant unique est précisément ce que `mika1745_t3_the_only_call_site_is_the_no_task_arm`
  (`ci_failure_handler.rs:1837`) police dans ce dépôt.
- Le site frère dans **ce même fichier** (`:4336-4338`, le P0 amiral) appelle directement. Deux
  formes pour le même geste à deux cents lignes d'écart, c'est la divergence qu'une relecture
  future paierait.

### D6 — Un verrou par tour : l'opérateur est réveillé une fois, pas une fois par pas

La branche vit dans la boucle de pas de `run_loop`. Un autre garde qui re-prompte au même pas fait
`continue`, et la branche est réévaluée au pas suivant sur une revendication qui persiste : sans
verrou, une seule récidive peut produire plusieurs notifications P0. Un booléen local
`assert_grounded_uncorrected_notified`, sur le modèle des `*_retry_done` voisins
(`:1582-1636`), le ferme. Le `warn!` reste lui aussi sous le verrou : deux lignes pour un seul
événement feraient mentir le comptage du D7.

L'assertion « **exactement une** émission » du test positif atteste le verrou ; sans lui, elle
rougirait sur un scénario à deux pas.

### D7 — Ce que l'opérateur lit, et par où

Un seul `warn!`, cible `mika::otel`, champs sur le modèle de `:2843-2856` : `trace_id`, `agent_id`,
`session_id`, `step`, `resource_type`, `resource_ref`, `claim`, `label = mode.label()`,
`event = "guard.assert_grounded_uncorrected"`.

La sonde de comptage, documentée dans `docs/operator/environment-variables.md` :

```sh
jq -r 'select(.fields.event == "guard.assert_grounded_uncorrected") | .timestamp' \
  < "$MIKA_SPIRIT_LOG_FILE" | cut -c1-10 | uniq -c
```

À comparer à la population de la première prise, qui est déjà instrumentée :

```sh
jq -r 'select(.fields.event == "guard.assert_grounded") | .timestamp' \
  < "$MIKA_SPIRIT_LOG_FILE" | cut -c1-10 | uniq -c
```

Le rapport des deux colonnes **est** la mesure que le point 2 du ticket demande, et la condition
d'armement du D8.

### D8 — Ce que ce vol ne fait pas, nommément

- **Il n'arme pas.** `MIKA_FABRICATION_P0_NOTIFY` est `false` par défaut. L'armement après la
  fenêtre de mesure est le suivi nommé par le Fire-Disposition.
- **Il ne touche pas les six autres sites `_uncorrected`.** Le rattachement de
  `false_local_hosting_claim`, `unactioned_frequency_promise`, `response_language_drift`,
  `time_of_day_greeting_mismatch`, `testimony_access_proposal` et
  `unacknowledged_send_failure` à la classe `fabrication-detected` est une décision de produit par
  site : la dérive de langue n'est pas une fabrication d'état système. Suivi nommé au
  Fire-Disposition.
- **Il n'écrit pas de ligne `audit_events`.** Six des sept sites de la famille n'en écrivent pas ;
  celui de mika#2136 en a une parce qu'un échec de livraison a besoin d'un registre durable. Ici
  l'instrument de comptage est le log, et la sonde du D7 le lit.
- **Il ne touche pas le rail de sévérité** (`messaging.rs`), ni la détection de fabrication
  (`evidence/guards.rs`), ni la classe `recovery-runbook` (#4 de l'inventaire mika#1381).
- **Il ne change pas le compte « 12 post-condition guards »** de `CLAUDE.md` : une branche
  d'observabilité n'est pas un garde — elle n'accorde aucun re-prompt et ne refuse aucun `EndTurn`.

---

## Phase unique — l'ordre de commit

1. **`crates/mika-common/src/config.rs`** — la clé, le champ, le défaut de test, le `Debug`,
   l'arme de `get_effective_value`. Plancher : `cargo test -p mika-common` vert.
2. **`crates/mika-agent/src/agent_loop/mod.rs`** — le paramètre `fabrication_p0_notify` de
   `run_loop` et son doc-comment, les trois résolutions (`is_some_and`), les trois arguments
   d'appel, le verrou du D6, la branche du D1 avec son `warn!` et son site P0. Plancher :
   `cargo build -p mika-agent` + `cargo clippy` verts.
3. **`crates/mika-agent/tests/eval/harness.rs`** — le champ du builder, le setter
   `.fabrication_p0_notify(bool)` sur le modèle de `.deployment(...)`, et la mutation du
   `Settings::test_defaults()` de `build()` (`harness.rs:621`). Sans lui, la porte du D3 est
   inatteignable depuis un scénario eval et l'AC4 ne peut pas être écrit.
4. **`crates/mika-agent/tests/eval/test_dispatch_stall_p0_1381.rs`** — `pub(crate)` sur
   `SeverityRecordingSender`, `Emission` et `SEVERITE_ABSENTE`, plus la ligne de doc qui nomme le
   second consommateur (le geste que ce fichier a lui-même reçu de
   `engine_correction_rejection.rs:78`).
5. **`crates/mika-agent/tests/eval/test_fabrication_detected_p0_2867.rs`** + sa déclaration dans
   **`crates/mika-agent/tests/eval.rs`** (obligatoire : la porte `test_eval_modules_declared`,
   `eval.rs:24`, rougit sur tout fichier de `tests/eval/` non compilé). Plancher :
   `cargo test -p mika-agent --test eval` vert.
6. **`docs/operator/environment-variables.md`** — la section de la variable, les deux sondes du
   D7, et la condition d'armement.
7. Fin de pipeline : `cargo test`, `cargo clippy`, `cargo fmt`, `make verify-bundled-skills`,
   `scripts/verify-pipeline.sh origin/main`.

---

## Fire-Disposition

**Option (b) — livrer désarmé**, avec son suivi tracké pour l'armer.

Ce vol livre bien un détecteur au sens de mika#2306 : la branche du D1 est un garde `EndTurn` dont
le chemin de succès est « aucune violation trouvée ». Les options (a) et (c) ne s'appliquent pas, et
pour une raison de nature, pas de confort :

- **(a) exception nommée en allowlist** suppose un corpus pré-existant à recenser — des sites de
  source, des entrées de registre, des fichiers. Le prédicat de ce détecteur est un **fait de
  tour** : il n'existe aucune population dans le dépôt à inventorier, et une allowlist à zéro
  entrée sans recensement serait une fiction grep-visible.
- **(c) halte-et-remontée** n'a pas d'objet : rien dans le cadrage n'est ouvert. Le ticket nomme le
  garde, le prédicat et la forme d'attestation ; la lecture du code les confirme tous les trois.

**Détail d'implémentation de (b)** — ce qui est désarmé, par quoi, et ce qui l'arme :

| moitié | état à l'atterrissage | mécanisme |
|---|---|---|
| détection + `warn!` structuré (l'instrument de comptage) | **armé** | inconditionnel ; sa seule conséquence est une ligne de log, qui ne noie aucune surface |
| notification P0 à l'opérateur (la conséquence) | **désarmé** | `MIKA_FABRICATION_P0_NOTIFY` / `Settings::fabrication_p0_notify`, défaut `false`, résolu par `is_some_and` ⇒ `Settings` absent vaut désarmé |

Le désarmement est **attesté, pas déclaré** : `mika2867_porte_fermee_le_transport_nobserve_rien`
(AC5) rejoue le scénario de récidive sans toucher la porte et assertionne zéro émission au
transport. Le jour où le défaut basculerait par accident, ce test rougit.

Le désarmement est aussi **réversible sans déploiement** : une ligne de configuration, pas une
recompilation. C'est ce qui permet à la fenêtre de mesure du D2 d'être courte.

**Suivi tracké pour l'armer** — à ouvrir par ce vol, en deux tickets distincts parce que les deux
décisions sont indépendantes :

- *« armer `MIKA_FABRICATION_P0_NOTIFY` après la fenêtre de mesure »* — porte le relevé des deux
  sondes du D7 sur ≥ 7 jours de trafic réel, et bascule le défaut à `true` si et seulement si la
  colonne de récidive tient sous le seuil que le relevé rendra défendable.
- *« rattacher les six autres sites `_uncorrected` à la classe `fabrication-detected` »* — une
  décision par site (le D8 dit pourquoi elle n'est pas uniforme).

`Tracked in:` — les deux numéros sont posés dans le corps de la PR, les tickets étant ouverts à
l'implémentation (le plan ne peut pas citer un numéro qui n'existe pas encore).

**Deux détecteurs voisins, vérifiés, qui ne sont pas touchés mais pourraient rougir** :

- `test_eval_modules_declared` (`eval.rs:24`) — rougit si le nouveau fichier de test n'est pas
  déclaré. L'étape 4 de la phase l'inclut dans le même commit ; AC1 le vérifie.
- `scripts/check-suite-wiring.sh` — **n'ouvre aucune entrée** : le fichier ajouté vit sous
  `crates/mika-agent/tests/`, que `cargo test -p mika-agent --test eval` exécute déjà en CI.
  Aucune ligne `unwired` n'est créée (mika#2793).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/config.rs` — `ConfigKeyInfo`, champ `#[serde(default)]` + doc, `test_defaults()`, `Debug`, arme `get_effective_value` | 16 |
| `crates/mika-agent/src/agent_loop/mod.rs` — paramètre `run_loop` + doc-comment, 3 résolutions `is_some_and`, 3 arguments d'appel, verrou du D6, branche du D1 (prédicat + `warn!` 9 champs + bloc de notification) + son doc-comment | 70 |
| `crates/mika-agent/tests/eval/harness.rs` — champ builder, setter `.fabrication_p0_notify()` + doc, mutation de `Settings::test_defaults()` dans `build()` | 12 |
| `crates/mika-agent/tests/eval/test_dispatch_stall_p0_1381.rs` — `pub(crate)` sur le double, `Emission`, `SEVERITE_ABSENTE` + doc du second consommateur | 8 |
| `crates/mika-agent/tests/eval/test_fabrication_detected_p0_2867.rs` — en-tête de doctrine, imports, fixture de récidive partagée, test positif, contrôle de porte fermée | 128 |
| `crates/mika-agent/tests/eval.rs` — déclaration du module + son commentaire | 3 |

Total estimé : 237 lignes

Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 237 est à 15 % sous la borne, et la marge est
structurelle, pas obtenue par découpage :

- **Le scénario qui atteint le site est déjà écrit et vert.** `grounding_regressions/guard_fabrication_telemetry.rs:68-101`
  (`test_guard_telemetry_assert_grounded`) fait tirer le garde `assert_grounded` sur deux tours de
  `MockLlmProvider`. Ce vol change **une** réponse — le tour 2 réaffirme au lieu de se corriger —
  et observe. Il n'écrit pas le scénario.
- **Le double d'observation n'est pas recopié.** Les 45 lignes du `SeverityRecordingSender` à
  double surcharge, les 18 lignes de types et de constantes, l'en-tête de doctrine qui les
  justifie : tout cela existe dans `test_dispatch_stall_p0_1381.rs` et coûte ici 8 lignes de
  visibilité (D5, étape 3). Le vol 3 de mika#1381 avait mesuré 95 lignes pour une copie des stubs
  de dispatch qu'il a évitée par le même geste ; l'économie est du même ordre.
- **`ToolContext` n'est pas touché** (D4). Le champ y aurait coûté une vingtaine d'éditions de
  sites de construction à travers le crate ; le paramètre de `run_loop` en coûte six.
- **Aucun helper, aucune ligne `audit_events`** (D5, D8) — deux candidats écartés sur le fond, pour
  ≈ 50 lignes qui ne sont pas au tableau.

Le poste le moins calibré est le fichier de test, à 128. La référence mesurée est
`test_dispatch_stall_p0_1381.rs` : 287 lignes pour deux tests, dont 57 d'en-tête de doctrine,
45 de double et 20 de types/constantes. En retirant les 65 lignes réutilisées et en tenant
l'en-tête à ≈ 28 lignes (le pourquoi vit dans ce plan, pas en double dans le fichier), 128 est
l'estimation haute de ce qui reste. Même à 160 — une dérive de 25 % sur ce seul poste — le total
reste à 269, sous la borne.

`docs/operator/environment-variables.md` est hors du compte : la consigne exclut `docs/`.

---

## Acceptance criteria

- **AC1** — `crates/mika-agent/tests/eval/test_fabrication_detected_p0_2867.rs` existe, est déclaré
  dans `crates/mika-agent/tests/eval.rs`, et n'est ni `#[ignore]` ni gardé par une variable
  d'environnement : il tourne dans `cargo test -p mika-agent --test eval` en CI. La porte
  `test_eval_modules_declared` reste verte.
- **AC2** — `crates/mika-common/src/config.rs` déclare la clé `fabrication_p0_notify`
  (`env_var: Some("MIKA_FABRICATION_P0_NOTIFY")`, `secret: false`) et `Settings` porte
  `pub fabrication_p0_notify: bool` avec `#[serde(default)]`. `Settings::test_defaults()` la pose à
  `false`. `get_effective_value("fabrication_p0_notify", …)` rend `"false"` sur un `Settings` par
  défaut. La clé est **absente** de `SETTABLE_CONFIG_KEYS` (D3).
- **AC3** — `run_loop` prend `fabrication_p0_notify: bool`, résolu aux trois points d'entrée
  (conversation, silencieux, équipe) par `params.settings.is_some_and(|s| s.fabrication_p0_notify)`.
  `grep -c "is_some_and(|s| s.fabrication_p0_notify)" crates/mika-agent/src/agent_loop/mod.rs`
  ⇒ `3`. Aucun des trois ne peut diverger : un correctif sur deux points d'entrée sur trois est un
  correctif qui ment (doctrine de `skill_tool_timeouts`, `:1407-1412`).
- **AC4** — le test positif
  `mika2867_la_recidive_dancrage_part_en_p0_sur_la_surface_jamais_muette` passe. Il :
  (a) fait tirer le garde `assert_grounded` au tour 1 (`trace.llm_call_count > 1`, sans quoi la
  branche est hors d'atteinte et le test n'atteste rien) ;
  (b) fait réaffirmer la même revendication non ancrée au tour 2 ;
  (c) ouvre la porte par `.fabrication_p0_notify(true)` ;
  (d) observe **exactement une** émission au transport portant le marqueur de la classe ;
  (e) assertionne que son étage n'est **pas** `SEVERITE_ABSENTE` — le site est passé par
  `send_with_severity`, pas par le corps par défaut qui délègue à `send` ;
  (f) assertionne que cet étage est `Some(NotificationSeverity::P0)` ;
  (g) assertionne que le texte nomme la ressource revendiquée et le `trace_id`.
- **AC5** — le contrôle de porte fermée
  `mika2867_porte_fermee_le_transport_nobserve_rien` passe. Même fixture de récidive, porte laissée
  à son défaut, `trace.llm_call_count > 1` vérifié (sans quoi le contrôle serait vert pour la
  mauvaise raison), et **zéro** émission au transport. C'est l'attestation du Fire-Disposition (b).
- **AC6** — la première prise ne notifie pas, et c'est le prédicat qui le garantit : la branche
  exige `intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)`, que le tour 1 ne peut pas
  satisfaire. Vérifié par lecture du prédicat et par AC4(a) — un site qui notifierait sur la
  première prise produirait **deux** émissions et ferait rougir AC4(d).
- **AC7** — un seul `warn!` par récidive, cible `mika::otel`,
  `event = "guard.assert_grounded_uncorrected"`, portant `trace_id`, `agent_id`, `session_id`,
  `step`, `resource_type`, `resource_ref`, `claim`, `label`. Le verrou du D6 le borne ;
  `grep -c "guard.assert_grounded_uncorrected" crates/mika-agent/src/agent_loop/mod.rs` ⇒ `1`.
- **AC8** — `docs/operator/environment-variables.md` porte une section pour
  `MIKA_FABRICATION_P0_NOTIFY` : son défaut (`false`), ce que l'armement change, les **deux** sondes
  `jq` du D7 (récidive et première prise), et la condition d'armement renvoyée au suivi.
- **AC9** — le compte des gardes de `CLAUDE.md` est inchangé (« 12 post-condition guards ») : ce vol
  n'ajoute pas de garde. `crates/mika-common/tests/claude_md_size.rs` reste vert.
- **AC10** — le site amiral de mika#1381 (`agent_loop/mod.rs:4296-4341`) et ses deux tests
  (`test_dispatch_stall_p0_1381.rs`) sont inchangés au comportement : seule la visibilité du double
  bouge (étape 3). Les deux tests restent verts.
- **AC11** — deux tickets de suivi sont ouverts et nommés par `Tracked in:` dans le corps de la PR :
  l'armement après fenêtre de mesure, et le rattachement des six autres sites `_uncorrected`.

---

## Definition of Done

- `cargo build` et `cargo clippy --all-targets` verts, sans nouvel `allow`.
- `cargo test` vert (suite complète, ≈ 3463 tests), `cargo test -p mika-agent --test eval` vert.
- `cargo fmt --check` vert.
- `make verify-bundled-skills` vert (aucun bundle touché, la porte reste la porte).
- `scripts/verify-pipeline.sh origin/main` vert, section `## Acceptance criteria` présente dans ce
  plan (U2).
- `docs/`-sync : aucun fichier de `docs/` copié par `build.rs` n'est touché — seul
  `docs/operator/environment-variables.md` change, qui n'est pas dans l'ensemble synchronisé. Le
  job `docs-sync` reste vert ; si CI dit le contraire, `scripts/sync-agent-docs.sh` est lancé et
  son résultat commité.
- Corps de PR : `Closes #2867`, les deux lignes `Tracked in:` de l'AC11,
  `Code review: delegated to mika-qa (mika#2694)`, et les recettes de rouge-avant du risque R1.
- Aucune invocation de `ce-simplify-code` ni de `ce-code-review` (mika#2694).

---

## Risques

**R1 — un test de sévérité qui n'a jamais été rouge n'atteste rien.** La classe entière de ce vol
est silencieuse par construction : remplacer `send_with_severity(..., P0)` par `send(...)`
**compile**. Trois injections sont donc exécutées à l'implémentation, le site restauré à
l'identique après chacune, et la sortie d'échec recopiée verbatim dans le corps de la PR :
(i) `P0` → `P2` ⇒ AC4(f) rouge ; (ii) l'appel remplacé par `sender.send(&notification)` ⇒ AC4(e)
rouge sur la sentinelle ; (iii) `intent_guard_retries.contains(...)` retiré du prédicat ⇒ AC4(d)
rouge sur une seconde émission (c'est aussi le contrôle de l'AC6). C'est la discipline du vol 3 de
mika#1381, dont l'en-tête porte les trois recettes.

**R2 — le tour 2 du harnais pourrait ne pas rejouer la violation.** `detect_affirmative_state_claim`
a ses propres heuristiques ; une réponse de tour 2 mal choisie peut ne plus matcher, et le test
serait vert sans jamais atteindre la branche. Atténuation : le tour 2 réutilise **la phrase du tour
1 mot pour mot** (`guard_fabrication_telemetry.rs:73-76` donne la forme qui matche, déjà verte en
CI), et l'assertion AC4(d) sur « exactement une émission » rougirait sur zéro.

**R3 — `detect_affirmative_state_claim` pourrait être coûteux à rejouer.** La branche le rappelle
sur le même texte, exactement comme les six sites `_uncorrected` frères le font sur leur propre
détecteur. Le coût est celui d'un passage de regex sur un texte de réponse, une fois par tour, et
seulement quand le label est déjà dans `intent_guard_retries` — donc jamais sur un tour sain.

**R4 — la porte désarmée pourrait être oubliée là.** C'est le risque propre à toute option (b). Il
est porté par l'AC11 : deux tickets ouverts, nommés par `Tracked in:` dans le corps de la PR, que
la porte `pr-body-validation.yml` exige (§ follow-up tracker) dès que le corps emploie une phrase
de report.

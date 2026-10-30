# mika#2871 — la porte P0 de fabrication refuse de s'armer sans son chat d'escalade

Suivi nommé par l'AC11 de mika#2867 (PR #2870), décisions D2/D8 du plan
`docs/plans/2026-10-21-002-feat-2867-notifications-site-emission-p0-fabrication-detected-plan.md`.

Le ticket porte trois critères d'acceptation qui ne sont pas de la même nature.
Ce vol en livre **un** — AC3, du code — et il dit précisément pourquoi les deux
autres ne sont pas livrables depuis un bac à sable de dispatch, ce qu'il leur
laisse, et pourquoi l'AC3 ne dépend pas de leur résultat.

---

## 1. Le partage des trois AC, tranché **en tête**

| AC | nature | ce vol |
|---|---|---|
| AC1 — le compte de récidives par site, sur la fenêtre | **acte d'opérateur sur l'installation réelle** | non livrable ici ; le vol consolide les trois sondes bornées (docs) |
| AC2 — la décision d'armement, écrite, par site | **acte d'opérateur sur le ticket** | non livrable ici ; dépend d'AC1 |
| AC3 — la question du chat d'escalade, tranchée, avec un test qui rougit | **code** | **c'est le périmètre de ce vol** |

**AC1 n'est pas livrable depuis le bac à sable, et c'est mesuré, pas supposé.**
Dans le worktree de grooming, le 2026-10-30 : `MIKA_SPIRIT_LOG_FILE` est
**non positionnée**, et `~/.mika/logs/` **n'existe pas** — seuls `~/.mika/data`
et `~/.mika/state` sont montés. Les trois sondes du ticket lisent toutes
`"$MIKA_SPIRIT_LOG_FILE"` : aucune ne peut rendre un chiffre ici. C'est la même
contrainte que `docs/operator/environment-variables.md` enregistre déjà pour le
dimensionnement de mika-dev — « `~/.mika/data/mika.db` n'est pas lisible depuis
le bac à sable de dispatch ». Un plan qui promettrait le compte promettrait une
fabrication, et la classe de garde que ce ticket arme est précisément celle qui
sanctionne ça.

**AC3 ne dépend pas du résultat d'AC2, et c'est l'argument de séquencement.**
La garde est saine sous les deux décisions :

- si AC2 dit **« ne pas armer »**, la garde reste inerte (elle ne regarde qu'une
  porte explicitement armée) et protège l'armement futur, celui qui se fera un
  jour où personne ne relira ce ticket ;
- si AC2 dit **« armer »**, la garde est exactement ce qui rend l'acte
  d'armement sûr, et elle tire **dans le redémarrage que l'armement exige déjà**
  (`docs/operator/environment-variables.md` : « l'armer demande un redémarrage,
  pas un déploiement »).

Donc le code part maintenant, avant la mesure, sans rien préjuger d'elle.

---

## 2. Ce que le vol laisse à AC1 : trois sondes, trois fenêtres, bornées

Aujourd'hui la doc porte **deux** sondes (`environment-variables.md:9394-9400`),
toutes deux sur le site 6d, et **aucune n'est bornée** : elles comptent le
fichier de log entier. Le ticket a depuis acquis deux sites de plus, avec des
fenêtres qui ne coïncident pas. Le vol consolide les six sondes (trois récidives
+ trois contrôles positifs) en un bloc borné, dans la section mika#2867 de
`docs/operator/environment-variables.md`.

Les six noms d'événement, vérifiés dans `crates/mika-agent/src/agent_loop/mod.rs` :

| site | première prise (contrôle positif) | récidive (la population comptée) | ligne |
|---|---|---|---|
| 6d `AssertGrounded` | `guard.assert_grounded` | `guard.assert_grounded_uncorrected` | `:4050` / `:4119` |
| 5d `FalseLocalHosting` | `guard.false_local_hosting_claim` | `guard.false_local_hosting_claim_uncorrected` | `:2885` / `:2946` |
| 5e `…PromiseAccomplished` | `guard.unactioned_frequency_promise` | `guard.unactioned_frequency_promise_uncorrected` | `:3053` / `:3123` |

Les trois fenêtres, reprises du corps du ticket :

- sites 6d et 5d : **2026-10-23T13:01:03Z → 2026-10-30T13:01Z** ;
- site 5e : **2026-10-23T16:00:35Z → 2026-10-30T16:00:35Z**, trois heures plus
  tard, parce que `assertion_nature` est à la racine de la ligne dès bc92d236 et
  que #2893 n'a pas touché au détecteur.

**La borne va dans le `select`, pas dans la tête de l'opérateur.** La forme
`cut -c1-10 | uniq -c` du ticket agrège par jour, ce qui suffit à écarter les
jours antérieurs — mais **pas le 2026-10-23 lui-même**, qui chevauche la borne
(13:01:03Z pour deux sites, 16:00:35Z pour le troisième). Le seau du 23 mélange
donc du pré-fenêtre et du post-fenêtre, et c'est exactement le jour où le
redémarrage a eu lieu, donc le jour le plus chargé en lignes qu'il ne faut pas
compter. Les sondes livrées portent la comparaison lexicographique sur
`.timestamp` (ISO 8601, donc l'ordre lexical **est** l'ordre chronologique) :

```sh
# site 6d — récidive d'ancrage, par jour, sur la fenêtre
grep guard.assert_grounded_uncorrected "$MIKA_SPIRIT_LOG_FILE" \
  | jq -r 'select(.event == "guard.assert_grounded_uncorrected"
                  and .timestamp >= "2026-10-23T13:01:03Z"
                  and .timestamp <  "2026-10-30T13:01:00Z") | .timestamp' \
  | cut -c1-10 | uniq -c
```

Même forme pour 5d (mêmes bornes) et pour 5e (bornes propres **plus** le terme
`and .assertion_nature == "accomplished_state"`, qui est ce qui exclut la moitié
« engagement sur l'avenir », hors classe).

**Le contrôle positif est obligatoire sur chacun des trois sites**, et la
doctrine en est déjà écrite à `environment-variables.md:9394-9400` : une colonne
de récidive vide veut dire « la garde ne récidive pas » **ou** « le binaire servi
est antérieur à l'instrument », et seule la colonne de première prise sépare les
deux. Trois sites, donc trois contrôles — un contrôle emprunté à un autre site ne
prouve rien sur celui qu'on lit. Le `grep` de la sonde de contrôle est une
sous-chaîne de celui de la récidive : d'où le `select` sur `.event`, qui est
porteur et non cosmétique.

Ce bloc est de la documentation. Il ne coûte aucune ligne de code et ne décide
rien : il rend l'acte d'AC1 un copier-coller, et c'est tout ce qu'un vol peut
faire pour un AC qui vit sur une installation qu'il ne voit pas.

---

## 3. AC3, la question de produit, tranchée

> « Faut-il que le code refuse d'armer sans le chat d'escalade ? »

**Oui, et au démarrage.** Trois faits décident.

**(a) Ce qui arrive aujourd'hui si la porte est armée sans le chat.** Le
transport retombe sur le chat normal — `messaging.rs:210-222`, bras
`_ => (normal_chat_id, TargetKind::EscalationUnsetFallback)` — avec le WARN
`operator_escalation_chat_unset` (`messaging.rs:312-333`). Cette retombée est la
décision D4 de mika#1381 et elle est **juste pour les sept sites P0 de ce
ticket-là** : sur ce chemin il n'y a pas de sink hors-ligne, donc un P0 refusé
serait un P0 perdu. Mais le texte de la classe `fabrication-detected` est préfixé
`Fabrication detected (uncorrected)` : c'est un diagnostic d'opérateur. Sur un
tenant, le « chat normal » est **le chat de l'utilisateur final**. La doc le dit
déjà en prose (`environment-variables.md`, section mika#2867, « Où la
notification arrive ») ; aujourd'hui rien ne l'applique.

**(b) La précondition n'est écrite nulle part en machine.** Vérifié : il n'existe
pas de `Settings::validate()` ; `validation.rs:70` `validate_file_key` a un
catch-all `_ => {}` sans bras pour l'une ni l'autre clé ; le seul refus dans le
loader (`config.rs:2832-2840`, `MIKA_INTERNAL_TOKEN`) est mono-clé, sur la
**forme** d'une valeur, pas sur une **combinaison**. La condition d'armement
n'existe donc que dans un paragraphe de prose (`environment-variables.md:9412-9421`),
et une précondition que seule la prose porte est une précondition qui tient
jusqu'au premier opérateur pressé.

**(c) Le point d'accroche canonique existe, et il a deux précédents.**
`server/mod.rs` appelle déjà deux gardes de démarrage qui refusent une
**combinaison** incohérente et nomment le fautif :
`tier_guard::assert_family_tier_env_consistency` (`:945-948`) et
`budget_guard::assert_llm_budgets_valid` (`:966`). Les deux tournent après
`provision_well_known_agents` et avant toute initialisation d'agent. Un
troisième membre de cette famille est le geste que ce dépôt fait déjà, pas une
invention.

### 3.1 La proportionnalité, et pourquoi le refus de démarrage n'est pas disproportionné

Le contre-argument mérite d'être écrit, parce que ce vol **s'écarte** sur un
point de la justification de `budget_guard`. Celle-ci dit : « Pre-existing
violations get no grace period, and that costs nothing. An agent already carrying
an invalid pair could not make an LLM call anyway. » Ce n'est pas le cas ici :
une porte armée sans chat d'escalade est un état qui **fonctionne** — la flotte
sert, les P0 partent, simplement au mauvais endroit. Le refus casse donc quelque
chose qui marchait, ce que le précédent ne faisait pas. Trois raisons tiennent
quand même :

1. **Le refus ne tire que dans un geste explicite et surveillé.**
   `fabrication_p0_notify` est à `false` par défaut (`config.rs:1279`) et
   délibérément absente de `SETTABLE_CONFIG_KEYS` : aucun agent ne peut l'armer
   depuis une conversation, et aucun déploiement existant ne la porte. Le seul
   chemin vers le refus est « un opérateur pose la variable et redémarre » — et
   ce redémarrage est déjà la condition d'armement documentée. L'opérateur
   regarde donc le démarrage au moment précis où la garde parle.
2. **La sortie est d'une ligne, dans les deux sens.** Le message nomme les deux
   issues : poser `MIKA_OPERATOR_ESCALATION_CHAT_ID`, ou remettre
   `MIKA_FABRICATION_P0_NOTIFY=false`. Un refus dont le remède est dans son
   propre texte n'est pas une panne, c'est une question fermée.
3. **L'état refusé n'a que deux issues, et aucune n'est celle que l'opérateur a
   demandée.** Soit rien ne tire (ce que la fenêtre de mesure rend probable), et
   l'armement était décoratif ; soit quelque chose tire, et c'est la fuite. Il
   n'y a pas de troisième cas où l'état armé-sans-route rend le service qu'on
   croyait avoir armé.

### 3.2 L'alternative écartée : tenir la porte fermée au lieu de refuser

L'autre forme possible : résoudre la porte à `false` aux trois entrées de
`run_loop` quand la route est inutilisable, et l'annoncer par une ligne de
démarrage — ce qui serait le cinquième membre de la famille
`log_qa_ci_coherence_gate_state` / `log_dependabot_verdict_gate_state` /
`log_pipeline_grounding_gate_state` / `log_pass_evidence_gate_state`
(`server/mod.rs:971-993`). Écartée, pour la raison que `budget_guard` écrit
lui-même : « It corrects no value and invents none. An invalid pair is an
operator error; repairing it silently would reproduce the very defect this ticket
instruments. » Résoudre à `false` une porte que l'opérateur a mise à `true`, **c'est
corriger sa valeur** — et le mode de panne qui en résulte (« j'ai armé la porte,
elle ne tire pas ») est précisément celui que ce dépôt nomme « échoue en
silence », à une ligne de log près que personne n'est obligé de lire. Le refus
est bruyant par construction ; la fermeture silencieuse ne l'est que par
discipline de lecture.

---

## 4. Le prédicat unique : « cette route P0 est-elle utilisable ? »

Point porteur de correction, et pas de cosmétique. Le transport refuse `Some(0)`
comme route (`messaging.rs:217`, `Some(id) if id != 0`), parce que `chat_id == 0`
est le sentinelle documenté « aucun canal de réponse » sur lequel `send()`
court-circuite (mika#650/#1090). Une garde qui testerait seulement
`operator_escalation_chat_id.is_none()` laisserait donc passer
`operator_escalation_chat_id = 0` — et sur cette configuration la retombée du
transport se produit **exactement comme si la variable était absente**. La garde
serait verte et la fuite aurait lieu. Deux copies de « cette route est-elle
utilisable ? » divergeraient le jour où la première est modifiée.

D'où un site de vérité unique, dans `crates/mika-agent/src/messaging.rs`, à côté
de l'unique consommateur historique :

```rust
/// Cette valeur est-elle une route P0 utilisable ? (mika#2871)
///
/// Extrait de `resolve_notification_target` pour que la garde de démarrage et
/// le transport posent *la même* question. `Some(0)` est faux : voir la doc de
/// `resolve_notification_target` — c'est le sentinelle « aucun canal de
/// réponse » (mika#650/#1090), donc un chat d'escalade à `0` retombe sur le
/// chat normal exactement comme une variable absente. Une garde qui ne
/// testerait que `is_none()` serait verte sur cette configuration, et la
/// notification d'opérateur partirait chez l'utilisateur final.
pub(crate) fn p0_route_is_usable(escalation_chat_id: Option<i64>) -> bool {
    matches!(escalation_chat_id, Some(id) if id != 0)
}
```

`resolve_notification_target` devient :

```rust
NotificationSeverity::P0 => {
    if p0_route_is_usable(self.escalation_chat_id) {
        (self.escalation_chat_id.unwrap_or(normal_chat_id), TargetKind::Escalation)
    } else {
        (normal_chat_id, TargetKind::EscalationUnsetFallback)
    }
}
```

Les deux tests existants de ce routage (`messaging.rs:839`
`mika1381_un_p0_part_sur_le_chat_descalade`, `:866`
`mika1381_un_p0_sans_chat_descalade_retombe_sur_le_chat_normal`) sont les
témoins de non-régression de cette extraction : ils ne changent pas et doivent
rester verts.

---

## 5. La garde : `crates/mika-agent/src/server/fabrication_p0_guard.rs`

Signature calquée sur son voisin, pour que la famille se lise d'un bloc :

```rust
pub fn assert_fabrication_p0_arming_valid(home_dir: &Path) -> Result<()>
```

Corps :

1. `for agent_name in super::tier_guard::servable_agent_names(home_dir)` — la
   **même** population que `budget_guard` (`tier_guard.rs:215`, `pub(crate)`
   précisément pour ce partage : deux copies de « quels agents ce processus
   sert-il ? » dériveraient le jour où l'une apprend une nouvelle disposition sur
   disque).
2. `let agent_home = mika_common::home::resolve_agent_home(home_dir, &agent_name);`
3. `let settings = Settings::load_for_agent(home_dir, &agent_home)` —
   **la cascade complète, pas une lecture de fichier.** C'est ce que le runtime
   résout lui-même (`server/mod.rs:578`), et une garde qui lirait par un autre
   chemin garderait un autre prédicat. En particulier : `MIKA_FABRICATION_P0_NOTIFY`
   posée dans l'environnement du service arme **tous** les agents à la fois (la
   cascade env domine le `config.toml` global), ce qui est le cas réel
   d'armement — une garde qui ne lirait que les fichiers ne le verrait pas.
4. **Un échec de `load_for_agent` n'est pas une violation d'armement** : il est
   propagé tel quel par `?`, avec le nom de l'agent en `.context()`. Confondre
   « je n'ai pas pu lire la configuration » avec « cette configuration arme sans
   route » enverrait l'opérateur à la mauvaise clé.
5. Violation si `settings.fabrication_p0_notify && !crate::messaging::p0_route_is_usable(settings.operator_escalation_chat_id)`.
   Le fautif est accumulé en `format!("  - {agent_name}: …")` avec la valeur lue
   (`absent` ou `0`), comme `budget_guard` le fait — nommer l'agent est toute la
   valeur de la garde.
6. `if offenders.is_empty() { return Ok(()); }` puis `bail!` portant : la liste,
   la raison en une phrase (un diagnostic d'opérateur partirait sur le chat de
   l'utilisateur final du tenant), et **les deux sorties** —
   `MIKA_OPERATOR_ESCALATION_CHAT_ID=<id non nul>` ou
   `MIKA_FABRICATION_P0_NOTIFY=false`.

**Ce que la garde ne fait pas.** Elle ne corrige aucune valeur, n'en invente
aucune, et ne touche ni `config.toml` ni la base. Elle ne vit **que** dans
`run_server` : `mika config get`, `mika agents budget` et tout le reste de la CLI
continuent de lire une installation mal configurée — c'est par là que l'opérateur
la répare, la fermer serait se tirer dans le pied.

### 5.1 Les tests (c'est la moitié d'AC3)

Dans `#[cfg(test)] mod tests` du même fichier, sur le modèle exact de
`budget_guard.rs:169-200` : helper `home_with_agent(name, config_toml)` écrivant
`identity.toml` + `config.toml` sous un `TempDir`, et helper `clean_p0_env()`
retirant `MIKA_FABRICATION_P0_NOTIFY` et `MIKA_OPERATOR_ESCALATION_CHAT_ID` du
process env. **Tous les tests sont `#[serial]`** (crate `serial_test`, déjà
utilisée là) : ils affirment ce que la *cascade* résout, et l'environnement du
processus en est une porte — sans ça, un développeur qui exporte l'une des deux
variables voit rougir un code sain, ce qui est le mode de panne que
`clean_budget_env` documente déjà.

1. `mika2871_la_porte_armee_sans_chat_descalade_refuse_le_demarrage_en_nommant_lagent`
   — **le test qu'AC3 exige, et il porte ses deux contrôles dans le même test**,
   comme `budget_guard` l'argumente (« une sonde qui n'observe que le côté
   refusant ne distingue pas "la garde refuse les mauvais" de "la garde refuse
   tout", qui est le mode de panne qu'une garde est le moins capable de
   remarquer ») :
   - `fabrication_p0_notify = true` + `operator_escalation_chat_id = -100123`
     ⇒ `is_ok()` ;
   - `fabrication_p0_notify = true`, aucun chat d'escalade ⇒ `is_err()`, message
     contenant le nom de l'agent **et** `MIKA_OPERATOR_ESCALATION_CHAT_ID`.
2. `mika2871_un_chat_descalade_a_zero_est_refuse_comme_une_absence` — la
   configuration que `is_none()` seul laisserait passer (§4). `is_err()`, et le
   message doit nommer `0` : un opérateur qui a tapé `0` doit lire que c'est sa
   valeur qui est en cause, pas une variable manquante.
3. `mika2871_la_porte_desarmee_sans_chat_nest_pas_une_violation` — un agent sans
   aucune des deux clés, c'est-à-dire **toute la flotte d'aujourd'hui**. `is_ok()`.
   Une garde qui refuserait la géométrie livrée serait une régression déguisée en
   contrôle de sûreté. C'est aussi l'assertion auto-nettoyante de la
   Fire-Disposition ci-dessous.
4. `mika2871_aucun_agent_nest_pas_une_violation` — `TempDir` vide, `is_ok()`.

---

## 6. Le site d'appel

`crates/mika-agent/src/server/mod.rs` :

- `pub mod fabrication_p0_guard;` dans le bloc de déclarations (`:1-21`, ordre
  alphabétique — juste après `pub mod embedded_dashboard;`) ;
- l'appel **immédiatement après** `budget_guard::assert_llm_budgets_valid(global_home)?;`
  (`:966`), avec un commentaire disant *pourquoi ici* : après
  `provision_well_known_agents`, qui est ce qui écrit le `config.toml` par agent
  que la cascade lit — scanner plus tôt validerait l'état du démarrage précédent
  —, et avant toute initialisation d'agent, donc avant qu'aucun transport ne
  puisse émettre.

---

## 7. Ce que ce vol ne change pas

- **La détection.** Les six `warn!` des trois sites sont inchangés, champs
  compris. L'instrument de comptage d'AC1 ne bouge pas pendant la fenêtre — y
  toucher invaliderait la mesure que le ticket attend.
- **La porte elle-même.** `MIKA_FABRICATION_P0_NOTIFY` reste à `false` par
  défaut. **Ce vol n'arme rien** : il interdit un armement incomplet. La décision
  d'AC2 reste entière et appartient à l'opérateur.
- **La retombée D4 de mika#1381** pour les sept autres sites P0. Le WARN
  `operator_escalation_chat_unset` et la retombée sur le chat normal sont
  conservés mot pour mot : sans sink hors-ligne, un P0 refusé serait un P0 perdu.
  Ce vol ne refuse pas *l'envoi*, il refuse *le démarrage d'une configuration qui
  enverrait au mauvais endroit*.
- **Aucune ligne d'état de porte au démarrage.** La famille
  `log_*_gate_state` (`server/mod.rs:971-993`) ne gagne pas de cinquième membre
  ici — voir § 10, c'est un suivi.
- **`docs/operator/` n'est pas miroité dans `crates/mika-agent/docs/`** (vérifié :
  pas de dossier `operator/` dans la copie crate), donc la modification de
  documentation ne demande **pas** `scripts/sync-agent-docs.sh` et ne peut pas
  faire rougir le job `docs-sync`.
- **`CLAUDE.md` n'est pas touché** : la doctrine de ce dépôt veut qu'une variable
  et sa surface opérateur se documentent dans
  `docs/operator/environment-variables.md`, pas dans le fichier chargé en entier
  dans chaque prompt.

---

## 8. Ordre de commit

1. `feat(2871): un prédicat unique dit si une route P0 est utilisable` —
   extraction de `p0_route_is_usable` dans `messaging.rs` + bascule de
   `resolve_notification_target`. Plancher : les deux tests mika#1381 du routage
   restent verts sans modification.
2. `feat(2871): la porte P0 de fabrication refuse de s'armer sans chat d'escalade` —
   `fabrication_p0_guard.rs` (garde + ses quatre tests) et le site d'appel dans
   `server/mod.rs`. Plancher : les quatre tests passent, dont celui de la
   géométrie livrée (porte désarmée ⇒ `is_ok`).
3. `docs(2871): les trois sondes de récidive, bornées à leur fenêtre` —
   `docs/operator/environment-variables.md` (bloc des six sondes du § 2) et mise
   à jour du doc-comment de `fabrication_p0_notify` dans `config.rs` pour y
   nommer la nouvelle précondition machine.

Chaque commit est mergeable seul ; l'ordre importe (2 dépend du prédicat de 1).

---

## 9. Vérification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -p mika-agent --lib server::fabrication_p0_guard
cargo test -p mika-agent --lib messaging
cargo test -p mika-agent --test eval -- fabrication_detected
cargo test
scripts/verify-pipeline.sh origin/main
```

Les trois suites d'éval de la classe
(`tests/eval/test_fabrication_detected_p0_2867.rs`, `…_2872.rs`, `…_2879.rs` et
`…_2879_collision.rs`) passent par `EvalHarness`, qui pose
`fabrication_p0_notify` sur un `Settings` construit en mémoire
(`harness.rs:641`) sans jamais traverser `run_server` : la garde ne les voit pas
et elles doivent rester vertes **sans modification**. Si l'une rougit, c'est que
l'extraction du § 4 a changé un comportement de routage, pas que la garde a
mordu.

---

## Definition of Done

- `p0_route_is_usable` existe dans `messaging.rs`, est le **seul** site qui
  décide si un chat d'escalade est une route P0, et `resolve_notification_target`
  l'appelle.
- `server/fabrication_p0_guard.rs::assert_fabrication_p0_arming_valid` existe,
  parcourt `servable_agent_names`, lit chaque agent par
  `Settings::load_for_agent`, et refuse le démarrage en nommant chaque agent
  fautif et les deux sorties possibles.
- La garde est appelée dans `run_server`, après `budget_guard`, avant toute
  initialisation d'agent.
- Les quatre tests du § 5.1 passent, dont celui qui rougit sur une porte armée
  sans chat d'escalade (l'exigence littérale d'AC3) et celui qui garantit que la
  géométrie livrée (porte désarmée) démarre.
- Les deux tests de routage mika#1381 de `messaging.rs` passent, inchangés.
- Les quatre suites d'éval `test_fabrication_detected_p0_*` passent, inchangées.
- `docs/operator/environment-variables.md` porte les six sondes bornées, par
  site, avec leurs trois fenêtres et leurs trois contrôles positifs.
- Le doc-comment de `fabrication_p0_notify` nomme la précondition machine.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  et `scripts/verify-pipeline.sh origin/main` sont verts.
- La valeur par défaut de `MIKA_FABRICATION_P0_NOTIFY` est toujours `false` :
  ce vol n'arme rien.

---

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2871, avec sous chacun la
disposition de ce vol.

- **AC1 : le compte de récidives sur la fenêtre est écrit dans ce ticket, avec la
  commande qui l'a produit. Si #2878 est mergée, ce compte est donné par site.**

  *Disposition :* **acte d'opérateur, hors périmètre de ce vol.** Non livrable
  depuis le bac à sable de dispatch : `MIKA_SPIRIT_LOG_FILE` non positionnée et
  `~/.mika/logs/` inexistant (mesuré le 2026-10-30, § 1). Ce vol livre les six
  sondes bornées — trois sites, trois fenêtres, trois contrôles positifs — de
  sorte que l'acte soit un copier-coller (§ 2). Le compte et la commande vont
  dans le ticket, par l'opérateur ou par le callback de mika-dev.

- **AC2 : la décision d'armement est écrite, avec sa raison, pour chaque site que
  la porte couvre.**

  *Disposition :* **acte d'opérateur, hors périmètre de ce vol**, et dépendant
  d'AC1. Ce vol ne la préjuge pas et reste sain sous ses deux issues (§ 1). La
  règle de la porte unique énoncée par le ticket — un site trop bruyant interdit
  d'armer les autres — porte désormais sur **trois** sites, les trois bras de
  `PendingFabricationP0Site` (`evidence/guards.rs:1062-1083`), et ce vol ne les
  sépare pas : séparer les portes reste le vol que le ticket nomme en réserve.

- **AC3 : la question du chat d'escalade est tranchée. Si le code doit l'exiger,
  c'est fait avec un test qui rougit sans la variable.**

  *Disposition :* **c'est le périmètre de ce vol.** Tranchée : oui, le code doit
  l'exiger, et au démarrage (§ 3, avec l'alternative écartée en § 3.2). Le test
  qui rougit sans la variable est
  `mika2871_la_porte_armee_sans_chat_descalade_refuse_le_demarrage_en_nommant_lagent`,
  qui porte dans le même corps le contrôle inverse (porte armée **avec** chat ⇒
  démarre), plus
  `mika2871_un_chat_descalade_a_zero_est_refuse_comme_une_absence` pour la valeur
  que `is_none()` seul laisserait fuir.

---

## Fire-Disposition

Ce vol livre un détecteur : `assert_fabrication_p0_arming_valid`, une garde de
démarrage dont le chemin de succès est « aucun agent n'arme la porte sans route ».
Les quatre tests du § 5.1 en sont aussi, au sens de la règle.

**Option retenue : (a) — exception nommée en allowlist, avec un ensemble
d'exceptions VIDE, prouvé et épinglé.**

La population que ce détecteur juge n'est pas résidente du dépôt : c'est la
configuration d'une installation au moment de son démarrage. Elle se sépare en
deux, et chacune reçoit son traitement.

**1. Population compilée / résidente du dépôt : zéro violation, et c'est
prouvable ici.**

- `fabrication_p0_notify` est `false` par défaut — `#[serde(default)]` sur un
  `bool` sans fonction de défaut (`crates/mika-common/src/config.rs:1279`), et
  `test_defaults()` l'épingle à `false` (`config.rs:2950`) ;
- la clé est **délibérément absente de `SETTABLE_CONFIG_KEYS`** : aucun agent ne
  peut l'armer depuis une conversation ;
- preuve grep-visible : `grep -rn fabrication_p0_notify crates/` ne rend que la
  déclaration, l'entrée de `CONFIG_KEYS` (`config.rs:350-358`), le bras de
  `get_effective_value` (`:573`), les trois résolutions `is_some_and`
  (`agent_loop/mod.rs:6831`, `:7959`, `:8680`) et le harness d'éval — **aucun site
  qui la pose à `true` hors d'un test**. Aucun gabarit d'identité, aucun
  `well_known_agents`, aucun fixture ne l'arme.

Donc : **zéro entrée d'exception à écrire.** Aucun fichier d'allowlist n'est
créé : un fichier d'exceptions vide est un fichier qui ne peut que mentir sur son
contenu. L'ensemble est vide, il est énoncé ici, et il est tenu par l'assertion
suivante.

**Assertion auto-nettoyante** —
`mika2871_la_porte_desarmee_sans_chat_nest_pas_une_violation` (§ 5.1, test 3) :
un agent sans aucune des deux clés démarre. Ce test rougit le jour où le défaut
compilé passe à `true`, c'est-à-dire **exactement le jour où l'ensemble vide
ci-dessus devient périmé**. C'est ce qui empêche cette disposition d'être une
affirmation ponctuelle.

**2. Résidu hors dépôt : nommé, avec sa sonde de pré-déploiement.**

Ce que l'opérateur a pu poser dans `~/.mika/config.toml`, dans un
`~/.mika/agents/<nom>/config.toml` ou dans l'environnement du service n'est pas
observable depuis le bac à sable (même contrainte qu'AC1). Sonde, à passer
**avant** le `make deploy` du binaire qui porte la garde :

```sh
for a in $(ls ~/.mika/agents); do
  printf '%s: ' "$a"
  mika --agent "$a" config get fabrication_p0_notify
done
```

(`--agent` est un drapeau **global** de la CLI — `cli.agent`, lu dans
`crates/mika-cli/src/main.rs:39-42` —, pas un drapeau de `config get` : il
précède la sous-commande.)

Sur **cette** installation la réponse est connue d'avance par l'existence même du
ticket : mika#2871 existe pour *décider* s'il faut armer, donc rien n'est armé —
et AC2 est encore ouvert à l'heure de ce plan. Le résidu attendu est vide. Si la
sonde rendait pourtant un `true`, le remède précède le déploiement et il est
d'une ligne : poser le chat d'escalade pour cet agent, ou désarmer.

**Pourquoi pas (b) — livrer désarmé.** Un détecteur de configuration livré
`#[ignore]` ne détecte rien du tout : sa seule valeur est d'être dans le chemin
du démarrage, et l'en retirer le réduit à un test qui n'observe jamais une
configuration réelle. Les quatre tests, eux, atterrissent armés.

**Pourquoi pas (c) — halte-et-remontée.** (c) est pour une implémentation qui ne
peut pas avancer sans cadrage. Celle-ci peut : la garde porte sa propre remontée
dans son message de refus — qui nomme l'agent, la valeur et les deux sorties —
et le geste qui la ferait tirer est un redémarrage déjà surveillé (§ 3.1). Le
cadrage qu'un (c) irait chercher est précisément ce que la question d'AC3
demandait au plan de trancher, et le § 3 le tranche.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/fabrication_p0_guard.rs` (doc de module + garde) | 80 |
| tests du même module (4 tests + 2 helpers) | 105 |
| `crates/mika-agent/src/messaging.rs` (extraction `p0_route_is_usable` + bascule) | 20 |
| `crates/mika-agent/src/server/mod.rs` (déclaration + site d'appel + commentaire) | 12 |
| `crates/mika-common/src/config.rs` (doc-comment de `fabrication_p0_notify`) | 10 |

Total estimé : 227 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 227 est sous les deux bornes (279 par vol, 1000
par PR), avec ≈ 19 % de marge sur la plus serrée. La marge est réelle et non
optimiste pour une raison structurelle : les trois gros postes sont **calqués
ligne pour ligne sur un voisin mesurable**. `budget_guard.rs` fait 337 lignes
pour cinq tests et une résolution à quatre branches ; celle-ci a **une** branche
(une conjonction de deux champs, pas une cascade de provenance à déchiffrer) et
quatre tests, d'où l'estimation à ≈ 185 pour le fichier entier. L'extraction du
§ 4 déplace six lignes existantes et en ajoute une poignée de doc. Il n'y a ni
migration de schéma, ni nouveau type, ni nouvelle surface HTTP, ni modification
du moteur d'agent — les trois sites de résolution de la porte dans
`agent_loop/mod.rs` ne sont pas touchés.

Le poste qui pourrait déborder est le volume de documentation des tests, le style
de ce dépôt étant verbeux à cet endroit ; il est hors `docs/` donc il compte, et
c'est ce que les 105 lignes du second poste provisionnent. Deux soupapes, dans
cet ordre, si l'estimation se révélait basse en vol : fusionner les tests 2 et 4
dans le test 1 (qui porte déjà deux contrôles), puis réduire le doc de module en
renvoyant à ce plan. Aucune ne touche au périmètre fonctionnel.

**Un seul livrable a été délibérément sorti du vol pour tenir la borne** : la
ligne d'état de porte au démarrage (§ 7), cinquième membre de la famille
`log_*_gate_state`. Elle est utile à AC2 — c'est ce qui atteste que le
redémarrage d'armement a bien pris la valeur, par opposition à
`mika config get` qui lit la configuration déclarée et non celle que le processus
sert. Elle n'est pas nécessaire à AC3 et vaut ≈ 25 lignes de plus.

Tracked in: senara-solutions/mika#2871 — le parent porte ce report jusqu'à
l'ouverture de son enfant, « la porte P0 de fabrication dit au démarrage si elle
est armée ». Le ticket enfant est ouvert par l'implémenteur de ce vol et son
numéro remplace cette ligne dans le corps de la PR. Ce report ne relève pas de la
règle de découpage en vols — le total est **sous** la borne (§ ci-dessus) : c'est
un retrait de confort, nommé ici pour qu'il ne se perde pas.

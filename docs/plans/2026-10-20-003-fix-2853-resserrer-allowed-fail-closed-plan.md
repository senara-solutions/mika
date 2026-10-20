# mika#2853 — vol 2 de mika#2784 : resserrer `allowed` en fail-closed

> Ticket : senara-solutions/mika#2853 — `fix(agent): mika#2784 vol 2 — resserrer
> allowed en fail-closed : aucun builtin, agents utilisateurs compris`
> Labels : `p2-normal`, `agent-core`, `ready`, `dispatch:loop`
> Refs : mika#2784 (parent), #2838 (vol 1, mergé), mika#2748, mika#2027, mika#811

---

## 0. Pourquoi ce ticket se groome aujourd'hui, et ce qu'il tranche

mika#2784 a été découpé en deux parce que son resserrement était suspendu à deux
questions de doctrine, et que resserrer une posture qu'on ne voit pas rend
l'incident suivant plus difficile à lire (mika#2293). Le vol 1 (#2838) a livré
l'observabilité : la ligne `agent_fail_closed_posture`, son champ `cause` à trois
valeurs, et un champ `tools_allowlist` **dérivé** de `allowed.is_some()` posé
exprès pour que le jour du resserrement le même `grep` dise s'il est en force
dans le binaire servi.

Vincent a tranché les deux questions le 2026-10-20, sur bearing Prime, par un
commentaire sur #2784 (« je valide ») :

1. **Q1 — aucun builtin, pas même `send_message`.** Une identité malformée ne dit
   rien de fiable sur ce que l'agent devrait pouvoir faire. Le signal à
   l'opérateur vient de la couche au-dessus : « la posture se signale ; l'agent se
   tait ».
2. **Q2 — le fail-closed couvre aussi les agents utilisateurs.** Un fichier
   disparu ne permet plus d'attester ce qu'est l'agent, quel que soit son
   propriétaire. Pas d'exemption par catégorie. En contrepartie la restauration
   doit être rapide et visible : la ligne de journal nomme **l'agent et le
   fichier manquant**. « Ferme dur, répare vite. »

Ce plan est l'exécution de ces deux décisions. Il est petit en production — une
ligne — et l'essentiel de son volume est le contrat qui empêche cette ligne
d'être vraie sans être servie.

---

## 1. État mesuré du code (`main` = `c5e64079`)

Tout ce qui suit a été lu, pas supposé.

### 1.1 Le site de production

`prompt.rs:1185-1202` — `fail_closed_identity(agent, path, cause)` construit un
`ToolsIdentityConfig` dont `disabled` est `MIKA_ARCH_DISABLED_TOOLS` et dont
`allowed` vaut `None`, sous un commentaire de dix lignes (`:1191-1200`) qui
**justifie** ce `None` : « Writing an allowlist here would take tools away from an
agent on a provisioning fault, i.e. at the worst possible moment. Tightening it
to an allowlist is a fleet-wide decision with its own follow-up ticket (mika#2784
morceau 2, gated on two doctrine questions) ». Ce commentaire est l'argument que
les décisions ci-dessus renversent ; le laisser serait laisser le dépôt plaider
contre son propre code.

La fonction est **privée au module** et a exactement trois appelants, tous dans
`prompt.rs` : `:1064` (absent / illisible), `:1105` (parse error sur agent bien
connu), et elle-même pour aucun autre. Aucune signature ne se propage.

### 1.2 Le mécanisme qui applique `allowed` accepte déjà `Some([])` — mesuré

`agent_loop/mod.rs:9301-9353` — `apply_agent_tool_visibility` :

- l'early return (`:9307`) ne mord que si `disabled.is_empty() && allowed.is_none()`,
  donc un `Some([])` **n'y échappe pas** ;
- le prédicat d'admission est `allowed.is_none_or(|list| list.iter().any(…))`
  (`:9313`) ; sur `Some([])` la fermeture s'exécute et `[].iter().any(…)` rend
  `false` pour **tout** outil. Le `retain` vide donc le tableau ;
- `filter_shape` rend `"both"` et `allowed_size` rend `Some(0)` (`:9329-9346`) —
  `0` et non « absent », la distinction étant déjà écrite à cet endroit
  (mika#2331).

Et ce cas est déjà épinglé : rangée 5 de
`mika2748_le_predicat_traite_les_cinq_etats_de_la_paire`
(`mod.rs:13770-13775`), dont le doc-comment dit en propre qu'elle est *« the only
assertion in the repo that distinguishes `Some([])` from `None` »* et qu'elle
existe pour qu'un futur éditeur qui aplatirait l'`Option` ne serve pas **tous**
les builtins à un agent qui n'en admet **aucun**.

**Conséquence : zéro ligne à écrire dans `agent_loop`.**

### 1.3 Ce que l'allowlist gouverne — et ce qu'elle ne gouverne pas

`inject_skills_and_resolve_tools` (`mod.rs:9647-9652`) applique le filtre sur le
tableau des builtins **avant** d'y ajouter les outils de skill (`:9704-9708`) et
les outils MCP. Le commentaire du site le dit : *« the reason the allowlist
governs builtins only: skill and MCP tools are appended below »*.

Donc le contrat de ce vol porte sur les **builtins**. Qu'un agent fail-closed ne
reçoive par ailleurs aucun outil de skill est l'effet de la sentinelle
`FAIL_CLOSED_SKILL_SENTINEL`, inchangée et hors périmètre.

### 1.4 Un tableau vide part en `tools: None`, pas en `tools: []`

Aux trois sites de requête (`mod.rs:6063-6067`, `:7245`, `:7955`), un
`llm_tool_defs` vide devient `None` et non `Some(vec![])`. Rien à écrire — mais
cela **détermine la forme que le test doit lire** : `captured_requests[0].tools`
sera `None`, pas une liste vide. Le test actuel passe par
`.tools.as_deref().unwrap_or(&[])` (`test_fail_closed_tool_surface_2784.rs:85-87`),
qui aplatit les deux ; sous le nouveau contrat cette distinction doit être dite,
sans quoi « aucun outil publié » et « le harnais n'a rien capturé » rendent les
mêmes octets.

### 1.5 Le vol 1 a posé une assertion conçue pour rougir ici

`prompt.rs:3782-3805` — `mika2784_la_posture_ne_declare_aucune_allowlist_doutils`
épingle `identity.tools.allowed == None` **et** `tools_allowlist="absent"`, avec
un message d'échec qui nomme les deux questions de doctrine comme condition de
levée : *« Si cette assertion rougit sans ces deux décisions, le resserrement a
été fait sans qu'on l'ait tranché. »* Les décisions existent, écrites et datées :
sa condition est levée, et ce plan la réécrit plutôt que de la contourner.

### 1.6 Le test eval mesure déjà un agent utilisateur — sans le dire

`tests/eval/test_fail_closed_tool_surface_2784.rs` supprime l'`identity.toml` que
`EvalHarness::build()` vient de semer. Le home est le `TempDir` lui-même, donc
`agent_name_from_home` rend une chaîne aléatoire que `find_well_known_agent` ne
reconnaîtra jamais : **la fixture est déjà celle de Q2** (agent non bien connu,
fichier absent). Son module doc traite ce fait comme un détour (« pourquoi
`missing_file` et pas `parse_error` », `:13-21`) et non comme une couverture.
C'est une propriété accidentelle, pas un contrat.

Il porte par ailleurs deux assertions que ce vol renverse (`:120-134`) :
`!names.is_empty()` et « `send_message` reste servi », la seconde sous un message
qui dit *« Si cette assertion rougit, le morceau 2 a été fait sans décision
écrite »*.

### 1.7 Le journal n'est pas lisible depuis un test eval

`test_utils.rs:1` — `pub mod test_helpers` est sous `#[cfg(test)]`, donc
`install_capturing_subscriber` n'existe pas pour le binaire d'intégration
`tests/eval`. Aucun test eval du dépôt ne capture `tracing` (vérifié : aucune
occurrence de `capturing_subscriber` sous `crates/mika-agent/tests/`).

La capture vit dans les tests inline de `prompt.rs`, via
`journal_de_chargement` (`:3601-3613`) et `home_dagent` (`:3618-3622`), ce
dernier donnant à l'agent **un nom choisi**. `mika2784_un_fichier_absent_nomme_la_posture_et_sa_cause`
(`:3636-3662`) utilise déjà `"vincent-perso"` — agent utilisateur, fichier
absent — et assert déjà `cause="missing_file"` et le nom de l'agent. Il manque le
**chemin**.

### 1.8 Une prose qui devient fausse

`mika2027_absent_identity_toml_fails_closed` (`:3834-3880`) porte un paragraphe
(`:3866-3872`) : *« what it deliberately does NOT deny […] #811 keeps the
agent-scoped memory writes […] A neutered agent can still write core memory,
which is re-injected into every later system prompt. »* Ses **assertions**
restent vraies (elles lisent `tools.disabled`, inchangé) ; sa **prose** devient
fausse au niveau du servi, puisque `update_core_memory` ne sera plus publié. Une
suite verte sous une prose qui dit l'inverse du contrat est pire qu'un rouge.

### 1.9 Le test de l'universalité existe déjà, et il porte déjà la prémisse Q2

`mika2027_absent_identity_toml_fails_closed_for_user_defined_agent`
(`prompt.rs:3890-3905`) est le test que mika#2027 a écrit pour l'universalité de
la posture. Il utilise `"vincent-perso"`, un fichier absent, et il porte déjà
l'assertion de prémisse exacte que ce vol demande :

```rust
assert!(
    crate::well_known_agents::find_well_known_agent("vincent-perso").is_none(),
    "fixture must name a user-defined agent for this test to mean anything"
);
```

Son doc-comment dit en propre que *« AC1 admits no exception, and discriminating
here would reopen the "which agent is supposed to have a file?" ambiguity the
universal rule closes »* — c'est l'argument de Q2, écrit trois volumes avant la
décision. Mais il n'assert que `skills.allowlist` : sous le vol 2,
l'universalité comprend aussi `tools.allowed`, et ce test est le site où on
l'écrit. Il appelle `load_identity` directement, donc il ne lit pas de journal ;
l'assertion de journal reste pour V1, qui passe par `journal_de_chargement`.

---

## 2. Décisions

### D1 — `allowed: Some(vec![])`, et le commentaire change de camp

Une ligne de production. Le commentaire de dix lignes qui justifie `None` est
**remplacé**, pas complété : il porte désormais la décision Q1, sa raison (une
identité illisible ne dit rien de fiable sur les permissions voulues), le
déplacement du signal vers la couche au-dessus (`agent_fail_closed_posture`), et
la date et le lieu de la décision (2026-10-20, commentaire de Vincent sur
mika#2784). Un futur lecteur doit trouver là *pourquoi* un agent dégradé se tait,
pas seulement *qu'*il se taise.

Le doc-comment de la fonction (`:1151-1155`) gagne une section courte **Pourquoi
aucun builtin (mika#2853, Q1)**, à côté des deux sections que le vol 1 y a
laissées.

### D2 — rien n'est écrit dans `agent_loop`, et c'est démontré en § 1.2

Le prédicat traite déjà `Some([])`, la rangée 5 de la table l'épingle, et le
`filter_shape`/`allowed_size` de la ligne d'observabilité disent déjà le nouveau
régime (`"both"` / `0`). Ajouter un site serait doubler un prédicat qui existe
(doctrine mika#2201).

### D3 — `tools_allowlist = "present"` reste **dérivé**, et le message porte la conséquence

AC3 demande `present`. Le champ est lu sur `tools.allowed.is_some()` (`:1214`),
donc il rendra `present` sans qu'on le touche — c'est exactement le dispositif
mika#2340 que le vol 1 a posé : il distingue « le vol 2 est servi » de « le vol 2
est écrit mais le binaire est antérieur ».

**Le risque de lecture est nommé plutôt qu'ignoré.** `present` dit *une allowlist
est déclarée*, pas *des outils sont autorisés* ; un opérateur pressé peut lire le
second. On ne le corrige **pas** en ajoutant un champ (hors périmètre, et un
littéral y perdrait la propriété mika#2340). On le corrige par le **message** du
`tracing::error!` (`:1215-1219`), qui dit aujourd'hui « all skills evicted
(sentinel allowlist), every mutational builtin denied » — faux de deux façons
sous ce vol : ce n'est plus seulement mutationnel, et ce n'est plus la dénylist
qui l'opère. Il dira **aucun outil intégré n'est publié au modèle**.

Et la sonde qui porte le **compte** existe déjà, sans une ligne de plus : la
ligne `applied identity tool-visibility filter` (`mod.rs:9336-9351`) portera
`allowed_size=0`, `filter_shape="both"` et `kept_count=0` pour cet agent. Le
runbook la nommera comme confirmation du côté servi, à côté de la ligne de
posture qui dit le côté déclaré.

### D4 — Q2 : la fixture eval reste la même, l'**affirmation** est ce qu'on ajoute

Le test eval mesure déjà un agent non bien connu à fichier absent (§ 1.6). On
n'ajoute pas un second fichier : on ajoute l'assertion de prémisse
`find_well_known_agent(<dernier composant du home>).is_none()`, qui transforme
une propriété du `TempDir` en contrat. C'est mot pour mot la forme que
`mika2027_absent_identity_toml_fails_closed_for_user_defined_agent` porte déjà
(`prompt.rs:3895-3898`, § 1.9) : on ne l'invente pas, on l'étend au niveau que
ce test unitaire ne peut pas voir — le tableau servi. Et c'est ce qui fait dire
au test eval *il couvre Q2* plutôt que *il la couvre par accident*.

Alternative refusée : un second test eval avec un home nommé. `EvalHarness`
impose son `TempDir` comme home ; le faire varier demanderait d'élargir le
builder pour une assertion que la prémisse ci-dessus rend déjà vraie.

### D5 — la moitié « journal » d'AC2 se tient en test unitaire

Le journal n'est pas lisible depuis `tests/eval` (§ 1.7), et l'exporter derrière
`test-utils` élargirait une surface publique pour une assertion que
`prompt.rs` peut porter aujourd'hui, sur une fixture qui nomme déjà son agent
utilisateur. AC2 se décompose donc :

| moitié | où | pourquoi là |
|---|---|---|
| zéro builtin **servi** à un agent utilisateur à fichier absent | `tests/eval/test_fail_closed_tool_surface_2784.rs` | seul endroit qui lit le tableau **reçu par le modèle** |
| l'identité **construite** pour un agent utilisateur porte `Some(vec![])` | `prompt.rs` § `mika2027_…_for_user_defined_agent` étendu (§ 1.9) | le test de l'universalité existe déjà, avec sa prémisse Q2 écrite |
| la ligne nomme l'agent **et** le chemin | `prompt.rs` § V1 étendu | seul endroit qui capture `tracing` **et** choisit le nom de l'agent |

Les deux premières lignes ne se doublent pas : l'une atteste ce que
`fail_closed_identity` **construit**, l'autre ce que le modèle **reçoit**. C'est
la distinction que mika#2340 impose partout ailleurs, et c'est la seule qui
distingue « le resserrement est écrit » de « le resserrement est servi ».

### D6 — la prose de `mika2027_*` est réécrite, bien que ses assertions restent vertes

Le paragraphe de `:3866-3872` est réécrit pour distinguer les deux niveaux : la
dénylist ne nomme toujours pas `update_core_memory` / `store_fact` /
`update_fact` (vrai — et c'est précisément ce que ses assertions testent), et
l'allowlist vide les retire **quand même** au moment de publier le tableau, un
niveau que ce test unitaire ne peut pas voir et que le test eval tient. Sans
cette réécriture, un lecteur conclut d'une suite verte qu'un agent fail-closed
peut encore écrire sa mémoire.

### D7 — le rouge est mesuré avant le commit, pas committé

Le ticket exige « un rouge mesuré avec `allowed: None` ». On l'obtient en
écrivant les tests d'abord et en les lançant contre `main` **dans le worktree,
sans committer** ; les deux messages d'échec sont recopiés dans le corps de la PR.
Committer un état rouge ferait échouer la CI sur un commit intermédiaire pour
documenter une mesure qu'une transcription documente aussi bien.

---

## 3. Livrables, dans l'ordre de commit

### Étape 0 (sans commit) — le rouge

1. Réécrire le test eval (`tests/eval/test_fail_closed_tool_surface_2784.rs`) :
   - module doc : dire que le fichier porte désormais le **vol 2**, que la
     fixture est celle de Q2, et pourquoi le témoin est portant ;
   - renommer `mika2784_un_agent_fail_closed_ne_recoit_aucun_builtin_mutationnel`
     en `mika2853_un_agent_fail_closed_ne_recoit_aucun_builtin` ;
   - ajouter la prémisse Q2 (D4) ;
   - `served_tool_names` rend `Option<Vec<String>>` (ou un couple) pour que le
     test puisse dire `tools == None` plutôt que « liste vide » (§ 1.4) ;
   - assertions : **zéro builtin**, `ADMITTED` (`send_message`) **absent**,
     `WITHHELD` absent ;
   - le témoin (`mika2784_avec_une_identite_valide_le_meme_builtin_est_servi`)
     devient le porteur unique de la non-inanité : il assert que le tableau servi
     est **non vide** et qu'il contient **`ADMITTED` et `WITHHELD`**. Sans lui,
     « le tableau est vide » est satisfait par un harnais qui ne publie rien.
2. Réécrire `mika2784_la_posture_ne_declare_aucune_allowlist_doutils` →
   `mika2853_la_posture_declare_une_allowlist_vide` : `allowed == Some(vec![])`
   et `tools_allowlist="present"`, avec un message d'échec qui nomme la décision
   du 2026-10-20 et son lieu.
3. Étendre `mika2784_un_fichier_absent_nomme_la_posture_et_sa_cause` : assertion
   que le journal contient le **chemin** de l'`identity.toml` attendu (AC2c).
4. Étendre `mika2027_absent_identity_toml_fails_closed_for_user_defined_agent`
   (§ 1.9) : à côté de l'assertion sur `skills.allowlist`, une assertion
   `identity.tools.allowed == Some(vec![])`, avec un message disant que
   l'universalité de la posture comprend l'allowlist d'outils depuis ce vol
   (AC2b). La prémisse `find_well_known_agent(...).is_none()` est déjà là : on
   n'y touche pas.
5. `cargo test -p mika-agent` + `cargo test -p mika-agent --test eval` →
   **constater le rouge**, recopier les messages.

### Commit 1 — `fail_closed_identity` ne publie plus aucun builtin

- `prompt.rs:1201` : `allowed: Some(vec![])`.
- `prompt.rs:1191-1200` : commentaire remplacé (D1).
- `prompt.rs:1151-1155` : section **Pourquoi aucun builtin (mika#2853, Q1)** dans
  le doc-comment.
- `prompt.rs:1204-1208` : le commentaire sur le champ dérivé passe du futur au
  présent — il annonçait « the day mika#2784 morceau 2 sets `Some(...)` » ; ce
  jour est arrivé, et ce qu'il dit désormais est ce que `absent` signifie
  maintenant : **un binaire antérieur à ce vol**.
- `prompt.rs:1215-1219` : message du `tracing::error!` (D3).
- Les quatre modifications de tests de l'étape 0 entrent dans ce commit : vert.

### Commit 2 — la prose qui disait l'inverse

- `prompt.rs:3866-3872` : paragraphe réécrit (D6).

### Commit 3 — docs

- `docs/operator/agent-identity-reprovision.md` :
  - préambule (`:3-7`) : « zero skills, every mutational built-in tool denied »
    devient « zéro skill, **aucun outil intégré** — l'agent répond, il n'émet
    rien », et dit que `send_message` est compris ;
  - § 1, rangée `tools_allowlist` du tableau (`:54`) : la mention « `absent`
    today — the tightening is mika#2784 morceau 2 and is not shipped » est
    remplacée par la lecture inverse (`present` attendu ; `absent` = binaire
    antérieur à ce vol) ;
  - § 1 : une quatrième règle de lecture — le diagnostic passe par
    `agent_fail_closed_posture`, qui nomme l'agent et le chemin, et la ligne
    `applied identity tool-visibility filter` confirme le côté servi
    (`allowed_size=0`, `kept_count=0`) ;
  - dire explicitement qu'il n'y a **aucune exemption par catégorie d'agent**
    (Q2) : un agent personnel est traité comme un agent bien connu.
- `docs/configuration.md:558-577` : même correction de valeur et même retrait de
  la mention « `absent` today ». Puis **`scripts/sync-agent-docs.sh`** (CI
  `docs-sync` échoue sinon).
- `docs/agent-internals/skills-system.md:48` : la phrase « so the same grep will
  say whether a future tightening is in force » passe au présent accompli.

---

## 4. Contrat de vérification

Chaque détecteur doit avoir été vu **rouge** sous la mutation nommée, pas
seulement vert à l'arrivée.

| # | détecteur | mutation qui doit le faire rougir |
|---|---|---|
| V1 | `mika2853_un_agent_fail_closed_ne_recoit_aucun_builtin` | remettre `allowed: None` (c'est le rouge exigé par le ticket, mesuré à l'étape 0) |
| V2 | — | remplacer `Some(vec![])` par `Some(vec!["send_message".into()])` : V1 rougit sur `ADMITTED` présent |
| V3 | le témoin à identité valide | vider le registre du harnais, ou appliquer le filtre à tout le monde : le témoin rougit là où V1 resterait vert |
| V4 | prémisse Q2 (`find_well_known_agent(...).is_none()`) | renommer le home du harnais en `mika-dev` : la prémisse rougit, et c'est ce qui empêche V1 d'être vrai pour la mauvaise raison |
| V5 | `mika2853_la_posture_declare_une_allowlist_vide` | écrire `tools_allowlist = "present"` en littéral puis remettre `allowed: None` : le test rougit sur `allowed`, prouvant que le champ est bien dérivé |
| V6 | V1 unitaire étendu (chemin dans le journal) | retirer `path = %path.display()` de la ligne de posture |
| V6b | `mika2027_…_for_user_defined_agent` étendu | remettre `allowed: None` : il rougit **sans passer par le harnais eval**, donc l'universalité est tenue même si le test eval est un jour désarmé |
| V7 | `mika2784_le_regime_nominal_est_muet` (inchangé) | doit rester **vert** : le resserrement ne doit pas faire parler un agent sain |
| V8 | `mika2784_un_agent_utilisateur_au_toml_invalide_nest_pas_fail_closed` (inchangé) | doit rester **vert** : Q2 porte sur le fichier **absent**, et l'asymétrie du chemin *malformé* pour un agent utilisateur (#811) reste hors périmètre |
| V9 | `mika2748_le_predicat_traite_les_cinq_etats_de_la_paire` (inchangé) | doit rester vert, rangée 5 incluse : c'est elle qui garantit qu'on n'a pas eu à écrire une ligne dans `agent_loop` |

Commandes :

```sh
cargo test -p mika-agent prompt::tests
cargo test -p mika-agent --test eval fail_closed
cargo test -p mika-agent --test eval tool_allowlist
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
scripts/sync-agent-docs.sh && git diff --exit-code crates/mika-agent/docs/
scripts/verify-pipeline.sh origin/main
```

---

## 5. Definition of Done

1. `fail_closed_identity()` pose `allowed: Some(vec![])`, et le commentaire qui
   justifiait `None` est **remplacé** par la décision Q1, sa raison, sa date
   (2026-10-20) et un lien vers mika#2784.
2. Un agent fail-closed reçoit **zéro** builtin dans le tableau réellement servi
   au modèle, `send_message` compris, mesuré sur `captured_requests`.
3. Ce fait est établi sur un **agent utilisateur** (non bien connu) à
   `identity.toml` absent, aux deux niveaux — l'identité construite
   (`mika2027_…_for_user_defined_agent`) et le tableau servi (test eval) — et
   chaque test l'**affirme** par une prémisse plutôt que de l'obtenir par
   construction.
4. La ligne `agent_fail_closed_posture` nomme l'agent **et** le chemin du fichier
   manquant, et porte `tools_allowlist = "present"` — ce dernier toujours lu sur
   `allowed.is_some()`, jamais écrit en littéral.
5. Le témoin à identité valide reçoit toujours ses builtins, et porte seul la
   non-inanité du test de zéro.
6. Le paragraphe de `mika2027_absent_identity_toml_fails_closed` qui affirme
   qu'un agent fail-closed peut encore écrire sa mémoire est réécrit.
7. `docs/operator/agent-identity-reprovision.md` dit qu'un agent fail-closed
   n'émet plus rien, qu'il n'y a aucune exemption par catégorie d'agent, et que
   le diagnostic passe par `agent_fail_closed_posture`.
8. `docs/configuration.md` à jour **et** `scripts/sync-agent-docs.sh` passé
   (`crates/mika-agent/docs/configuration.md` identique).
9. Aucune ligne ajoutée dans `agent_loop/mod.rs` ; la rangée 5 de
   `mika2748_le_predicat_traite_les_cinq_etats_de_la_paire` reste verte et
   intacte.
10. La dénylist `MIKA_ARCH_DISABLED_TOOLS`, la sentinelle de skills et
    `inject = false` sont **inchangées** (hors périmètre explicite du ticket).
11. Suite complète verte, clippy sans avertissement, `cargo fmt --check` propre,
    `scripts/verify-pipeline.sh origin/main` vert.
12. Le corps de PR porte les messages d'échec mesurés à l'étape 0 — la preuve du
    rouge sous `allowed: None`.

---

## 6. Acceptance criteria

Transcrits du corps du ticket, puis affinés comme le ticket le demandait
(« ébauche, à affiner au groom ») :

- **AC1** — un agent fail-closed reçoit un tableau d'outils vide, mesuré sur le
  tableau servi au modèle, avec un rouge mesuré sous `allowed: None`.
  *Affinage :* « vide » se lit sur `captured_requests[0].tools`, qui vaut `None`
  et non `Some([])`, les trois sites de requête convertissant un tableau vide en
  `None` (§ 1.4). L'assertion dit cette forme.
- **AC2** — un agent **utilisateur** à `identity.toml` absent est couvert de la
  même façon qu'AC1, et le journal nomme l'agent et le fichier.
  *Affinage :* trois moitiés, trois emplacements (D5). **AC2a** « zéro builtin
  servi » est tenue par le test eval, dont la fixture est déjà celle d'un agent
  non bien connu et qui l'affirme désormais par une prémisse. **AC2b**
  « l'identité construite porte `Some(vec![])` » est tenue par
  `mika2027_…_for_user_defined_agent`, qui est déjà le test de l'universalité et
  porte déjà la prémisse Q2 (§ 1.9) — l'universalité comprend désormais
  l'allowlist d'outils. **AC2c** « le journal nomme l'agent et le fichier » est
  tenue par V1, le journal n'étant pas capturable depuis `tests/eval` (§ 1.7).
- **AC3** — la ligne de posture porte `tools_allowlist = "present"`.
  *Affinage :* le champ reste **dérivé** de `allowed.is_some()`. V5 (§ 4) prouve
  qu'il l'est, en exigeant qu'un littéral `"present"` posé à la main laisse le
  test rouge.
- **AC4** — le témoin à identité valide garde ses builtins.
  *Affinage :* il devient le porteur **unique** de la non-inanité, l'assertion
  `!names.is_empty()` du test de zéro disparaissant par construction. Il assert
  donc à la fois le tableau non vide, `send_message` et `write_agent_file`.
- **AC5** — le commentaire de `fail_closed_identity()` et le runbook portent la
  décision du 2026-10-20.
  *Affinage :* trois sites de commentaire, pas un (D1 + D3) — le littéral, le
  doc-comment de la fonction, et le commentaire du champ dérivé, dont la phrase
  au futur (« the day morceau 2 sets `Some(...)` ») devient fausse à la lettre
  dès ce vol.

---

## Fire-Disposition

Ce plan livre des détecteurs : trois tests réécrits ou étendus (un eval, deux
unitaires) plus un témoin renforcé. Option retenue : **(a) exception nommée en
allowlist — avec une allowlist livrée VIDE**, et c'est démontrable plutôt que
déclaratif.

**Pourquoi zéro exception est le compte exact.** Un détecteur a besoin d'une
exception quand il peut rencontrer une violation **préexistante**. Aucun de ceux
d'ici ne le peut : chacun atteste le comportement que le **même commit** met en
place. Le resserrement et son test atterrissent ensemble ; il n'existe pas de
corpus où une violation pourrait se cacher, parce que le comportement mesuré
n'existe pas avant le commit qui le mesure.

**Le rouge n'est pas une exception, c'est la preuve — et il est obtenu avant le
commit.** Le ticket exige un rouge sous `allowed: None` ; l'étape 0 l'obtient
dans le worktree et le corps de PR le transcrit (D7). Rien n'atterrit rouge.

**L'assertion qui devait expirer expire — c'est le geste, pas une exception.**
`mika2784_la_posture_ne_declare_aucune_allowlist_doutils` a été écrite par le vol
1 **pour** rougir le jour où ce vol atterrit, avec sa condition de levée nommée
dans son message d'échec : une décision écrite sur Q1 et Q2. Cette décision
existe (2026-10-20, commentaire de Vincent sur mika#2784). Ce plan la réécrit en
`mika2853_la_posture_declare_une_allowlist_vide`, et la nouvelle assertion porte
à son tour la date et le lieu de la décision — de sorte qu'un futur éditeur qui
la voit rouge lit *quoi décider*, pas *quoi contourner*. C'est une assertion
auto-nettoyante qui a fait son cycle complet, et le dépôt garde la trace de son
expiration dans le diff.

**Aucun livrable désarmé, aucune halte.** Tous les tests atterrissent armés, dans
la suite par défaut (`cargo test`), sans `#[ignore]` ni `cfg` conditionnel. Les
`#[serial_test::serial]` des tests unitaires sont une contrainte d'ordonnancement
imposée par la capture `tracing` (mika#2646) et héritée des voisins, pas un
désarmement.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/prompt.rs` — production : `Some(vec![])`, commentaire du littéral, section de doc-comment, commentaire du champ dérivé, message du `tracing::error!` | 55 |
| `crates/mika-agent/src/prompt.rs` — tests : V6 réécrit, V1 étendu au chemin, `mika2027_…_for_user_defined_agent` étendu à `tools.allowed`, paragraphe de `mika2027_absent_identity_toml_fails_closed` | 80 |
| `crates/mika-agent/tests/eval/test_fail_closed_tool_surface_2784.rs` — module doc, prémisse Q2, inversion des assertions, témoin renforcé, signature de `served_tool_names` | 100 |

Total estimé : 235 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 235 passe sous les deux bornes — 1000 par PR et
279 par vol — avec 44 lignes de marge sur la plus serrée. Trois propriétés du
périmètre soutiennent cette estimation plutôt que de l'espérer :

- **la production est UNE ligne**, et le § 1.2 le démontre au lieu de l'annoncer :
  le prédicat accepte déjà `Some([])`, la rangée 5 de la table l'épingle, les
  champs d'observabilité du filtre disent déjà le nouveau régime. Le reste du
  budget est du commentaire et du contrat, dont le volume est borné par les
  blocs qu'il remplace ;
- **aucune signature ne se propage** : `fail_closed_identity` est privée au
  module et a trois appelants, tous dans le même fichier (§ 1.1). C'est la
  propagation de signature qui fait déborder les estimations ;
- **un seul fichier de test est réécrit**, et il fait 151 lignes aujourd'hui
  dont la structure (deux tests, un helper, un module doc) est conservée : on
  inverse des assertions et on en ajoute deux, on n'invente pas de harnais. Les
  trois touches unitaires sont une réécriture d'une vingtaine de lignes (V6) et
  deux assertions ajoutées dans des tests qui portent déjà leur fixture, leur
  prémisse et leur `#[serial_test::serial]` (§ 1.7, § 1.9).

Aucun découpage en vols n'est donc requis, et aucune suite n'est renvoyée : ce
plan ferme mika#2853 en entier. Les deux points laissés ouverts par le vol 1 et
que ce vol ne touche pas — l'asymétrie du chemin *malformé* pour un agent
utilisateur (#811, hors périmètre par V8) et la question de savoir si
`MIKA_ARCH_DISABLED_TOOLS` mérite une liste propre au fail-closed — restent la
propriété de leurs tickets d'origine et ne sont pas des reports de celui-ci.

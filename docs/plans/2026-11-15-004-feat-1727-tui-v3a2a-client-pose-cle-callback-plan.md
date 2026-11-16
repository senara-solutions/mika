# mika#1727 — vol V3a-2a : le client sait poser la clé de callback

> Vol client. `build_send_params`, `send_message_to_agent` et
> `chat_turn::send_turn_to_spirit` prennent un paramètre `callback_turn: bool` et
> savent poser `mika.callback_turn`. **Tous les appelants passent `false`** : le
> premier appelant à `true` est le vol V3a-2b, qui fait partir le bras
> `CallbackResult` sur le fil.
>
> `Refs mika#1727` — jamais `Closes`, ni dans le corps de la PR ni dans un commit.
> Le ticket se ferme au merge de V4.
>
> Tracked in: senara-solutions/mika#1727

---

## 0. Pourquoi ce vol tient en un seul vol

Le vol pose **un paramètre** sur **un site de construction**, et le propage à ses
appelants. Le modèle est écrit en entier sur `main`, six lignes au-dessus :
`session_isolated` est le même paramètre booléen, posé par le même `fields.insert`
sous le même genre de constante, avec sa batterie de tests dans le même module.
Le vol n'invente aucune forme ; il en instancie une seconde.

Le gros poste n'est pas le code de production (≈ 49 lignes sur deux fichiers),
c'est la **propagation mécanique** : les trois signatures touchées comptent
34 appels dans le crate — `build_send_params` 15 (1 en production, 14 en tests),
`send_message_to_agent` 13 (3 en production, 10 dans `tests/`), et
`send_turn_to_endpoint` 7 (1 en production, 6 en tests) — et un paramètre de plus
sur une signature n'est pas découpable en commits compilables sans eux. Chaque
appel coûte une ligne — sauf
trois, où l'appel franchit `max_width = 100` et où rustfmt éclate l'appel en
autant de lignes que d'arguments (§ 2 D5). Cela reste à ≈ 191 lignes estimées,
soit **238 lignes de marge** sur la plus serrée des deux bornes.

---

## 1. Ce qui est mesuré sur `main` (@ `fc9cab82`)

| fait | site |
|---|---|
| `CALLBACK_TURN_KEY` vaut `"mika.callback_turn"`, déclarée et documentée | `crates/mika-a2a/src/params.rs:196` |
| sa doc dit déjà « **Nothing sets this key yet** … V3a-2 is what makes the TUI's `CallbackResult` arm travel over the wire » | `params.rs:193-195` |
| la porte A2A la lit en booléen strict (absente/`null`/`false` ⇒ tour ordinaire ; autre forme ⇒ refus) | `params.rs:180-192`, lecteur côté `crates/mika-agent/src/server/a2a.rs` (vol V3a-1, inchangé ici) |
| `build_send_params` est le site unique où se bâtissent les métadonnées de requête | `crates/mika-cli/src/remote_ask.rs:294` |
| il pose `mika.session_isolated` en `Value::Bool(true)` quand on le demande, et laisse la clé **absente** sinon | `remote_ask.rs:320-327` |
| il laisse `metadata` **absent** quand aucune clé n'est posée | `remote_ask.rs:328-332` |
| son seul appel de production est dans `send_message_to_agent` | définition `remote_ask.rs:465`, appel `remote_ask.rs:494` |
| le détecteur structurel qui tient cette unicité | `mika1951_both_ask_doors_post_the_isolation_key_through_one_site`, `remote_ask.rs:1776` |
| `build_send_params` a **14** appels dans le module `tests` | `remote_ask.rs:1282, 1293, 1310, 1325, 1542, 1560, 1576, 1601, 1612, 1694, 1709, 1719, 1732, 1760` |
| le test d'indépendance de la famille, cumulatif depuis `two` → `three` → `four` | `mika1951_the_four_metadata_keys_are_independent`, `remote_ask.rs:1730` |
| appelants de production de `send_message_to_agent` | `commands/ask.rs:496`, `commands/chat_turn.rs:76`, `run_remote` → `remote_ask.rs:666` |
| appelants de `send_message_to_agent` dans `tests/` | `remote_ask_integration.rs:359, 393` ; `remote_ask_recovery.rs:214, 256, 279, 304, 317, 356, 374, 396` |
| `send_turn_to_spirit` résout l'endpoint et délègue | `commands/chat_turn.rs:61`, délégation l. 67 |
| `send_turn_to_endpoint` est le corps testable ; T1–T5 l'appellent directement | `chat_turn.rs:75` ; appels l. 292, 319, 343, 385, 409, 425 |
| son seul appelant de production est le bras `Message` | `commands/chat.rs:387` |
| `spawn_mock` + `ok_task_with_text` + `last_body` existent dans le module `tests` de `chat_turn.rs` | `chat_turn.rs:240-272`, usage modèle en T2 l. 310-336 |
| T2 lit la clé **par la constante**, jamais par un littéral | `chat_turn.rs:330` (`mika_cli::remote_ask::CALLER_SESSION_ID_KEY`) |
| les constantes de clé sont ré-exportées par `remote_ask`, ce qui les rend lisibles depuis `chat_turn` | `remote_ask.rs:23-25` (`pub use mika_a2a::{… SESSION_ISOLATED_KEY, …}`) |
| aucun `rustfmt.toml` dans le dépôt ⇒ `max_width = 100` par défaut | absence de `rustfmt.toml` / `.rustfmt.toml` à la racine |
| `crates/mika-cli/CLAUDE.md` pèse 82 245 o pour un cap de 100 000 | `crates/mika-common/tests/claude_md_size.rs:40` |
| la section où la phrase va | `crates/mika-cli/CLAUDE.md:366` « Un tour de la TUI exige mika-spirit (mika#1727, vol V1 sur 4) », § « L'unique appel local restant est le bras `CallbackResult` » l. 384-390 |

### Deux écarts entre la tête du ticket et la mesure

Déclarés ici parce que la QA juge contre ce plan, et qu'un plan qui recopie un
inventaire faux fait rougir un critère sans que le code ait tort.

1. **`tests/remote_ask_recovery.rs` porte 8 appels, pas 3.** La tête nomme les
   lignes 214, 256 et 279 ; les lignes 304, 317, 356, 374 et 396 en portent cinq
   autres. Le fichier est bien dans la liste des fichiers autorisés, donc **le
   périmètre ne s'élargit pas** — seul l'inventaire de la tête était incomplet.
2. **Trois appels ne peuvent pas changer « pour le seul argument `false` ».**
   `commands/chat.rs:387`, `remote_ask.rs:1282` et `remote_ask_recovery.rs:374`
   franchissent `max_width = 100` en recevant l'argument, et `cargo fmt` les
   éclate en un argument par ligne. Le geste reste celui que la tête décrit — un
   `false` ajouté à un appel, et rien d'autre sur le site — mais le diff y porte
   un reformatage que le critère 7 (`cargo clippy` vert, `cargo fmt` appliqué)
   rend obligatoire. Compté en § Taille estimée.

---

## 2. Décisions

### D1 — Le paramètre s'appelle `callback_turn`, et il se place après `session_isolated`

`build_send_params` devient :

```rust
fn build_send_params(
    message: &str,
    caller_session_id: Option<&str>,
    context_id: &str,
    only_skills: &[String],
    model_override: Option<&str>,
    session_isolated: bool,
    callback_turn: bool,
) -> MessageSendParams
```

En queue, et non entre deux paramètres existants : deux `bool` adjacents en
position positionnelle sont déjà un risque d'inversion silencieuse, et l'insérer
**avant** `session_isolated` ferait de chaque appel existant non recompilé un
appel qui compile et mentit. En queue, tout appel non mis à jour ne compile pas.
Même ordre, même raison, sur `send_message_to_agent`, `send_turn_to_spirit` et
`send_turn_to_endpoint` : la position du paramètre est la même sur les quatre
signatures, donc la chaîne se lit sans retenir d'ordre par fonction.

### D2 — `true` pose un `Value::Bool(true)` ; `false` ne pose rien

Le corps du `if` est calqué sur celui de `session_isolated`, à six lignes au-dessus :

```rust
    if callback_turn {
        // Un bool JSON, la seule forme que le serveur lit : toute autre forme
        // FAIT ÉCHOUER la requête au lieu de dégrader en « tour ordinaire »
        // (mika#1727 V3a-1). Un `"true"` lu comme `false` enregistrerait le
        // résultat brut une seconde fois et désarmerait la garde #716 sur le
        // tour dont les affirmations sont les moins vérifiables.
        fields.insert(
            CALLBACK_TURN_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
    }
```

`false` ne pose **rien**, et c'est la propriété que le critère 2 mesure : absente
et `false` disent la même chose au serveur, mais seule l'absence garde un
appelant qui ne déclare rien byte-pour-byte identique à un appelant qui précède
la clé. C'est déjà la propriété des quatre clés sœurs ; la cinquième ne l'entame
pas.

### D3 — La constante se lit par le ré-export de `remote_ask`

`CALLBACK_TURN_KEY` est ajoutée au `pub use mika_a2a::{…}` de `remote_ask.rs:23-25`,
à côté de `SESSION_ISOLATED_KEY`. Deux raisons, et la seconde est un critère :

- le module `tests` de `remote_ask.rs` la nomme alors sans préfixe, comme ses
  quatre sœurs ;
- le test de fil de `chat_turn.rs` la lit en `mika_cli::remote_ask::CALLBACK_TURN_KEY`,
  exactement comme T2 lit `CALLER_SESSION_ID_KEY` — ce qui satisfait le
  « jamais par un littéral » du critère 1 sans ouvrir de nouvelle ligne de
  visibilité (critère : *aucune ligne de visibilité n'est à ouvrir*). Le
  ré-export existe déjà ; on y ajoute un nom.

> **Amendement du pilote (2026-11-15) — la constante se lit par `params::`, pas
> par la racine de `mika_a2a`.**
>
> **Le point.** Le plan écrivait « ajoutée au `pub use mika_a2a::{…}` … à côté de
> `SESSION_ISOLATED_KEY` », en supposant `CALLBACK_TURN_KEY` ré-exportée par la
> racine du crate comme ses quatre sœurs. Elle ne l'est pas :
> `crates/mika-a2a/src/lib.rs:12-17` ré-exporte les quatre autres clés de requête
> et pas celle-là, qui n'existe qu'en `mika_a2a::params::CALLBACK_TURN_KEY`
> (`params.rs:196`). Écrit tel quel, le `pub use` ne compile pas :
> `error[E0432]: no CALLBACK_TURN_KEY in the root`.
>
> **La raison de ne pas corriger en amont.** L'ajouter au `pub use` de
> `crates/mika-a2a/src/lib.rs` serait le geste le plus propre, mais
> `crates/mika-a2a/` est **hors périmètre** par la tête du ticket (« Restent
> intacts … `crates/mika-a2a/` (`CALLBACK_TURN_KEY` y existe déjà) »). Le vol ne
> l'élargit pas pour une ligne de confort.
>
> **Ce qui est écrit à la place.** Le nom est ajouté **dans le même bloc
> `pub use`**, par son chemin de module : `params::CALLBACK_TURN_KEY`. Les deux
> propriétés que D3 porte tiennent inchangées — le module `tests` de
> `remote_ask.rs` nomme la constante sans préfixe, et le test de fil de
> `chat_turn.rs` la lit en `mika_cli::remote_ask::CALLBACK_TURN_KEY` comme T2 lit
> `CALLER_SESSION_ID_KEY` — et **aucune ligne de visibilité n'est ouverte** : le
> bloc existe, on y ajoute un nom, exactement comme le plan le disait. Un
> commentaire sur place dit pourquoi ce nom-là est qualifié quand ses sœurs ne le
> sont pas, pour qu'un lecteur n'y lise pas une étourderie.
>
> Coût : 5 lignes de commentaire, comptées en § Taille estimée.

### D4 — La doc dit ce que la clé demande, sa forme, et ce que vaut l'absence

Trois endroits, et rien de plus :

- **`build_send_params`** — un paragraphe sur le modèle de celui de
  `session_isolated` : ce que la clé demande au serveur (ce tour **est** un tour
  de callback : garde #716 armée, message utilisateur non réenregistré), qu'elle
  est **un booléen et rien d'autre** (toute autre forme fait échouer la requête,
  jamais un no-op silencieux), et que **l'absence vaut `false`**. La phrase
  « The four metadata keys are independent » devient « The five … » : un décompte
  faux dans la doc du site unique est précisément ce qui fait manquer une clé au
  prochain lecteur.
- **`send_message_to_agent`** — une phrase : le paramètre traverse jusqu'au site
  unique, et aucune porte de `mika ask` ne le pose à `true`.
- **`send_turn_to_spirit`** — le paragraphe « Les trois autres paramètres de
  `send_message_to_agent` sont posés vides à dessein … » est **remplacé** par
  l'état réel : skills et modèle restent vides pour les raisons déjà écrites,
  l'isolation reste `false` parce qu'un tour de TUI veut voir ses tours
  précédents, et `callback_turn` est désormais **un paramètre de cette
  fonction** — posé `false` par son unique appelant, le bras `Message`, parce
  qu'un message tapé par l'opérateur n'est pas un callback. La dernière phrase
  nomme V3a-2b comme le premier appelant à `true`.

### D5 — Les trois appels que rustfmt éclate sont reformatés, pas contournés

`commands/chat.rs:387` (98 colonnes), `remote_ask.rs:1282` (96) et
`remote_ask_recovery.rs:374` (97) dépassent 100 en recevant `, false`. Le pilote
laisse `cargo fmt` les éclater en un argument par ligne. Il **ne raccourcit pas**
un littéral de test ni un nom de variable pour rester sur une ligne : un
identifiant de session changé dans un test est un changement de test déguisé en
formatage, et c'est le genre d'écart que la QA ne peut pas distinguer d'une
régression. Les trois sites sont comptés à leur coût réel en § Taille estimée.

> **Amendement du pilote (2026-11-15) — rustfmt éclate sept sites, pas trois : le
> seuil qui décide n'est pas `max_width`.**
>
> Cette décision prédit les sites à éclater en mesurant la colonne 100
> (`max_width`). C'est le mauvais seuil. rustfmt applique d'abord l'heuristique
> `fn_call_width`, qui vaut `60` par défaut (60 % de `max_width`) et porte sur la
> **largeur de la liste d'arguments seule**, pas sur celle de la ligne. Un appel
> dont les arguments rendus dépassent 60 colonnes est éclaté en un argument par
> ligne *même si la ligne entière tient sous 100*.
>
> Mesuré après `cargo fmt` : **cinq** des huit appels de
> `tests/remote_ask_recovery.rs` sont éclatés, et non un seul, plus les sites de
> `commands/chat.rs` et de `remote_ask.rs` que D5 avait vus. Sept au total.
>
> La conduite de D5 ne change pas d'un iota — le pilote laisse `cargo fmt`
> éclater, et ne raccourcit aucun littéral de test pour tenir sur une ligne,
> exactement pour la raison écrite ci-dessus. Seule la **prédiction du nombre de
> sites** était fausse, et avec elle l'estimation de taille, amendée au § Taille
> estimée. Un futur plan qui veut prédire ces éclatements mesure la liste
> d'arguments contre 60, pas la ligne contre 100.

### D6 — Le test d'indépendance passe de quatre clés à cinq, en place

`mika1951_the_four_metadata_keys_are_independent` (l. 1730) est **étendu** et
renommé `mika1727_the_five_metadata_keys_are_independent` : `callback_turn` passe
à `true`, `metadata.len()` à 5, et une assertion de plus lit la cinquième clé.

Étendre plutôt qu'ajouter, parce que la série est explicitement cumulative —
`the_two_…` (mika#2363) → `the_three_…` (mika#2304) → `the_four_…` (mika#1951),
chacune se présentant comme « the direct continuation » de la précédente. Un test
« quatre clés » conservé à côté d'un test « cinq clés » n'affirmerait rien que le
second n'affirme pas, et la prochaine clé aurait deux endroits à mettre à jour,
dont un qu'on peut oublier sans rien faire rougir. Le préfixe de ticket suit la
convention de la série : celui du ticket qui ajoute la clé.

**Ce test-là n'est pas celui que le critère 4 protège.** Le critère 4 porte sur
`mika1951_both_ask_doors_post_the_isolation_key_through_one_site`, qui reste vert
**sans modification** : le vol n'ajoute aucun appel de `build_send_params` en
production, donc le compte que ce détecteur lit ne bouge pas.

### D7 — Le test de fil porte ses deux moitiés, et il est l'unique test de fil du vol

Le critère 3 exige les deux : à `true` la clé est sur le fil à côté de
`mika.caller_session_id` ; au même envoi à `false` elle n'y est pas. Les deux
moitiés vivent dans **un** test, par `spawn_mock` appelé deux fois dans la même
fonction — pas deux tests. La moitié négative n'a de valeur que comparée à la
positive dans la même mesure : séparées, un `spawn_mock` qui capturerait mal le
corps rendrait la négative verte pour la mauvaise raison, et rien ne le dirait.

Nom : `mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi`.

### D8 — Rien d'autre ne bouge

Le bras `CallbackResult` de `commands/chat.rs` est inchangé **à l'octet** : il
continue d'appeler `agent::run_agent` en processus, et les deux détecteurs
`mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle` et
`mika1727_v1_le_site_tolere_na_quun_seul_appel_local` restent verts sans que leur
`ALLOWED` ni leur compte ne change. `crates/mika-a2a/`, `crates/mika-agent/` et
`crates/mika-gateway/` ne sont pas touchés ; `docs/egress/`, `docs/openapi/`,
`docs/slash-commands.md` et tout fichier du plan de contrôle non plus.

### Pourquoi ce vol ne livre aucun détecteur (gate mika#2306)

Aucun livrable de ce vol n'a pour fonction primaire de signaler une violation.
Les trois tests ajoutés (critères 1, 2, 3) sont des tests de comportement : leur
chemin de succès est « la clé est là » ou « la clé est absente », pas « aucune
violation trouvée ». Ils ne balaient aucune source.

Le critère 5 — aucun appelant hors `remote_ask.rs` et `chat_turn.rs` ne nomme
`callback_turn` — est vérifié par **grep**, à la main, et n'est pas livré comme
test structurel : la propriété qu'un détecteur protégerait (un seul site de
construction des métadonnées) est **déjà** tenue par le détecteur existant de
mika#1951, que le critère 4 garde vert sans modification. En livrer un second sur
la même propriété serait deux vérités à maintenir pour une seule invariante.

La section `## Fire-Disposition` n'est donc pas requise, et n'est pas inventée.

---

## 3. Ordre de commit

Un paramètre de plus sur une signature casse tous ses appelants d'un coup : la
coupe en commits est contrainte par ce qui compile. D'où deux commits de code,
chacun vert et mesuré, et non un par fonction.

### C1 — `remote_ask.rs` sait poser la clé ; tous ses appelants passent `false`

- `CALLBACK_TURN_KEY` ajoutée au `pub use` (D3) ;
- paramètre + bloc `insert` sur `build_send_params` (D1, D2) + doc (D4) ;
- paramètre + argument + doc sur `send_message_to_agent` (D1, D4) ;
- `run_remote` passe `false` — un tour `mika ask --remote` n'est jamais un
  callback ;
- `commands/ask.rs:496` passe `false` ;
- `commands/chat_turn.rs:76` passe `false` **en littéral**, sans paramètre encore :
  c'est ce qui rend ce commit compilable seul. Le paramètre arrive en C2 ;
- les 8 appels de `tests/remote_ask_recovery.rs` et les 2 de
  `tests/remote_ask_integration.rs` passent `false` (l'appel à `true` de
  `remote_ask_integration.rs:393` reste à `true` sur `session_isolated` et prend
  `false` en queue) ;
- les 14 appels de `build_send_params` du module `tests` prennent `false`, sauf
  celui du test d'indépendance (D6) ;
- **tests du commit** : critère 1 (la clé posée en bool JSON), critère 2 (à
  `false`, `metadata` absent, et la clé absente même à côté d'une clé sœur),
  critère 6 (le test d'indépendance à cinq clés, D6).
- `cargo fmt`, `cargo clippy --all-targets -p mika-cli`, `cargo test -p mika-cli`.
- **Mesure**, puis halte si > 410 (§ Halte).

### C2 — la chaîne de la TUI porte le paramètre, et le bras `Message` passe `false`

- `send_turn_to_spirit` et `send_turn_to_endpoint` prennent `callback_turn: bool`
  et le passent (D1) ; doc de `send_turn_to_spirit` remaniée (D4) ;
- `commands/chat.rs:387` passe `false`, reformaté par rustfmt (D5) ; le bras
  `CallbackResult` n'est pas touché (D8) ;
- les 6 appels T1–T5 de `send_turn_to_endpoint` passent `false` ;
- **test du commit** : critère 3, les deux moitiés dans un test (D7).
- `cargo fmt`, `cargo clippy --all-targets -p mika-cli`, `cargo test -p mika-cli`.
- **Mesure**, puis halte si > 450.

### C3 — `crates/mika-cli/CLAUDE.md`

Dans la section V1 (l. 366), au § « L'unique appel local restant est le bras
`CallbackResult` » : le client **sait** poser `mika.callback_turn` par le site
unique `build_send_params`, et **aucun appelant ne la pose avant V3a-2b** — le
bras `CallbackResult` tourne donc encore en processus. Deux à quatre lignes.
`cargo test -p mika-common --test claude_md_size` (marge : 17 755 o).

### Fin de pipeline

`bash scripts/verify-egress-manifest.sh --report`, puis
`bash scripts/verify-pipeline.sh origin/main`. **L'un des deux rouge : le pilote
s'arrête et le dit**, sans ouvrir la PR. Puis PR portant `Refs mika#1727` et
aucun `Closes` / `Fixes` / `Resolves`, avec `Tracked in: senara-solutions/mika#1727`
et `Code review: delegated to mika-qa (mika#2694)`.

### Halte du pilote sur la borne

Après **chaque** commit : `git diff --numstat origin/main...HEAD`, plan exclu.
Total > 450 ⇒ arrêt (pas de commit suivant, pas de PR). Total > 410 après C1 ⇒
arrêt aussi. Message final : « HALTE borne : N lignes pour 450 », avec le compte
par commit. Les commentaires ne se rabotent pas pour tenir la borne : un
commentaire qui porte un pourquoi reste, et c'est le vol qui s'arrête.

---

## 4. Definition of Done

- [ ] `build_send_params` et `send_message_to_agent` portent `callback_turn: bool`
      en queue de signature ; `true` pose `CALLBACK_TURN_KEY` en `Value::Bool(true)`,
      `false` ne pose rien.
- [ ] `send_turn_to_spirit` et `send_turn_to_endpoint` portent le même paramètre
      et le passent au site unique.
- [ ] Les trois docs de D4 sont écrites ; celle de `send_turn_to_spirit` ne dit
      plus « les trois autres paramètres … posés vides ».
- [ ] Tous les appelants, production et tests, passent `false`.
- [ ] Les trois tests des critères 1, 2 et 3 sont verts ; le test d'indépendance
      couvre cinq clés.
- [ ] `mika1951_both_ask_doors_post_the_isolation_key_through_one_site` est vert
      **sans modification**.
- [ ] Le bras `CallbackResult` de `commands/chat.rs` est inchangé à l'octet.
- [ ] Seuls les 7 fichiers du périmètre changent (+ le plan).
- [ ] `cargo clippy --all-targets -p mika-cli` vert sans nouvel `allow` ;
      `cargo test -p mika-cli` vert ; `cargo test -p mika-common --test claude_md_size` vert.
- [ ] `verify-egress-manifest.sh --report` et `verify-pipeline.sh origin/main` verts.
- [ ] La PR porte `Refs mika#1727` et aucun `Closes` / `Fixes` / `Resolves`.

---

## 5. Acceptance criteria

Transcrits depuis le corps de mika#1727, § « Critères de V3a-2a ».

1. `build_send_params(…, true)` pose `mika_a2a::params::CALLBACK_TURN_KEY` en
   booléen JSON `true`. Le test lit la clé par la constante, jamais par un littéral.
2. `build_send_params(…, false)` ne pose pas la clé. Sans autre clé posée,
   `metadata` reste absent, comme sur main.
3. `send_turn_to_endpoint(…, true)` porte la clé à `true` sur le fil, à côté de
   `mika.caller_session_id`, à travers `spawn_mock`. Le même envoi à `false` ne la
   porte pas. C'est le test négatif de ce vol.
4. `mika1951_both_ask_doors_post_the_isolation_key_through_one_site` reste vert
   sans modification : un seul appel de `build_send_params` en production, dans
   `send_message_to_agent`.
5. Tous les appelants de `send_message_to_agent` et de `send_turn_to_spirit`
   passent `false`. `grep -rn "callback_turn" crates/mika-cli/src` ne rend, hors
   tests, que `remote_ask.rs` et `commands/chat_turn.rs`.
6. Seuls changent les fichiers listés plus haut. Le bras `CallbackResult` de
   `commands/chat.rs` est inchangé, à l'octet.
7. `cargo clippy --all-targets -p mika-cli` est vert, sans nouvel `allow`.
   `cargo test -p mika-cli` est vert, de même que
   `cargo test -p mika-common --test claude_md_size`. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il trouve
   l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/remote_ask.rs` — production (doc D4, paramètre, `insert`, ré-export, `run_remote`) | 24 |
| `crates/mika-cli/src/remote_ask.rs` — tests (14 appels dont un éclaté, critères 1 et 2, indépendance à cinq clés) | 66 |
| `crates/mika-cli/src/commands/chat_turn.rs` — production (deux signatures, doc remaniée, argument) | 25 |
| `crates/mika-cli/src/commands/chat_turn.rs` — tests (6 appels, test de fil des deux moitiés) | 50 |
| `crates/mika-cli/src/commands/chat.rs` (appel du bras `Message`, éclaté par rustfmt) | 7 |
| `crates/mika-cli/src/commands/ask.rs` (un argument) | 1 |
| `crates/mika-cli/tests/remote_ask_integration.rs` (2 appels) | 2 |
| `crates/mika-cli/tests/remote_ask_recovery.rs` (8 appels dont un éclaté) | 16 |

Total estimé : 191 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Dans l'unité du ticket** — lignes ajoutées par `git diff --numstat origin/main...HEAD`,
plan exclu, `CLAUDE.md` compris : **195 pour 450**. Les quatre lignes de
`crates/mika-cli/CLAUDE.md` (C3) sont la seule différence entre les deux unités.

Les deux totaux passent : 191 pour 429 (marge 238) et 195 pour 450 (marge 255).
Le vol reste donc **un seul vol**, et il tient pour la raison du § 0 — un
paramètre, un site, et une propagation dont chaque appel coûte une ligne. La
suite (`CallbackResult` sur le fil, puis V3b, V3c-1, V3c-2, V4) est nommée par
`Tracked in: senara-solutions/mika#1727`.

> **Amendement du pilote (2026-11-15) — le réel est 312 pour 450, et non 195.
> Le vol tient, l'estimation non.**
>
> Mesuré par `git diff --numstat origin/main...HEAD`, plan exclu, après les trois
> commits C1, C2 et C3 :
>
> | livrable | estimé | réel |
> |---|---|---|
> | `crates/mika-cli/src/remote_ask.rs` | 90 | **139** |
> | `crates/mika-cli/src/commands/chat_turn.rs` | 75 | **96** |
> | `crates/mika-cli/src/commands/chat.rs` | 7 | 7 |
> | `crates/mika-cli/src/commands/ask.rs` | 1 | 1 |
> | `crates/mika-cli/tests/remote_ask_integration.rs` | 2 | 2 |
> | `crates/mika-cli/tests/remote_ask_recovery.rs` | 16 | **58** |
> | `crates/mika-cli/CLAUDE.md` | 4 | **9** |
> | **total** | **195** | **312** |
>
> Trois causes, par ordre de poids :
>
> 1. **`remote_ask_recovery.rs`, +42.** La cause est celle de l'amendement de D5 :
>    cinq appels éclatés au lieu d'un. Le coût d'un appel propagé n'est pas « une
>    ligne » mais « une ligne, ou autant que d'arguments », et ici c'est quatre à
>    six arguments par site.
> 2. **`remote_ask.rs`, +49, et `chat_turn.rs`, +21.** La doc et les corps de test
>    coûtent plus que prédit. Le § D2 demande à la doc de dire *quatre* choses (ce
>    que la clé demande au serveur, qu'elle est un booléen et rien d'autre, que
>    toute autre forme refuse le tour, et que l'absence vaut `false`) ; cela ne
>    tient pas en les quelques lignes comptées. Les deux tests négatifs des
>    critères 2 et 3 portent chacun **deux moitiés** — clé absente seule, puis clé
>    absente à côté d'une clé sœur — avec leur assertion d'anti-vacuité, et une
>    moitié de test coûte autant qu'un test.
> 3. **`CLAUDE.md`, +5.** Le § de la section V1 affirmait encore que la porte A2A
>    écrit `is_callback_turn: false` en dur ; cette phrase est fausse depuis V3a-1
>    et devait partir, pas seulement être complétée. Corriger un faux voisin coûte
>    plus que poser deux lignes neuves.
>
> **Aucun commentaire n'a été raboté pour tenir la borne**, comme le corps du
> ticket l'exige : les pourquoi ci-dessus sont dans le code, et c'est eux qui
> pèsent. La borne du ticket (450) tient avec 138 de marge, et la halte à 410
> après le premier commit n'a pas été approchée : **201 après C1, 303 après C2,
> 312 après C3.**
>
> Ce que la QA doit juger : le périmètre des fichiers est exactement celui du
> plan et du corps du ticket, aucun fichier hors liste n'est touché, et la
> conduite de chaque décision D1 à D8 est respectée. Seules les **quantités**
> prédites par ce § et par D5 étaient fausses.

# mika#1727 — vol V3c-2a : la route des skills a son client

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3c-2a (le premier des trois vols de V3c-2 ; V3c-2b fera lire la route à la TUI, V3c-2c portera le test É1)
- **Branche** : `feat/1727/v3c2a-la-route-des-skills-a-son-client`, neuve depuis `main`
- **Base mesurée** : `main` @ `210f8dd8` (le merge de V3c-1, mika#3100) — la tête sur laquelle MPC a mesuré, et la tête de cette branche
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus 475 lignes ajoutées, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.

---

## 0. HALTE DU GROOM — le vol ne tient pas dans sa borne

**Ce plan ne demande pas un dispatch. Il demande un mot de Vincent.**

La tête du ticket pose la règle qui s'applique ici, mot pour mot :

> Le plan écrit son `Total estimé :` dans cette même unité, et son estimation
> hors `docs/` et hors `CLAUDE.md` dans l'unité de l'architecte. **Si l'un des
> deux dépasse sa borne (475, ou 452 estimées), le groom s'arrête et le dit. Il
> ne recoupe pas lui-même** : une coupe demande le mot de Vincent.

L'estimation détaillée du § *Taille estimée* donne **517 lignes dans l'unité de
l'architecte** (hors `docs/` et hors `CLAUDE.md`) et **534 dans l'unité de la
borne du vol**. Les deux bornes sont dépassées :

| unité | estimé | borne | marge |
|---|---|---|---|
| borne du vol (`--numstat`, plan exclu, doc comprise) | 534 | 475 | **−59** |
| architecte (hors `docs/`, hors `CLAUDE.md`) | 517 | 429 (mika#2693) / 452 (tête du ticket) | **−88 / −65** |

**Le groom s'arrête donc ici, et ne recoupe pas.** Les deux coupes candidates
sont nommées au § 4, chiffrées, et **non pratiquées** : trancher entre elles — ou
relever la borne — est le mot que ce plan attend. Tout le reste du document est
écrit en entier pour que ce mot suffise : la mesure à la source (§ 1), les
décisions (§ 2) et l'ordre de commit (§ 3) sont valides tels quels sous la coupe
A comme sous la coupe B, et le plan du ou des vols retenus s'en déduit sans
nouvelle passe de mesure.

**Ce n'est pas un plan d'un volume mal estimé.** Le dépassement vient de deux
postes que la tête du ticket exige tous les deux, et qu'elle chiffre à un vol :

1. **Un miroir de 23 champs.** La route sert treize champs par skill, plus
   `agent`, `skipped` et `warnings`. Le miroir qui les relit, ses cinq structs et
   le test qui relit chacun des 23 champs sont la moitié du vol à eux seuls.
2. **Cinq refus, chacun avec son test.** Le critère 3 en exige cinq distincts, et
   la tête interdit de reporter un test au vol suivant : « Chaque garde arrive
   avec ses tests, dans le même commit. »

Le précédent mesuré le confirme plutôt qu'il ne l'atténue : le vol V2d-3a
(mika#3086, `0b44a562`) a coûté **357 lignes** dans `session_remote.rs` pour un
client de **la même nature** mais à **trois** champs de miroir et six tests. Le
client de ce vol-ci porte 23 champs de miroir et sept tests, **et** le vol porte
en plus son côté serveur. 517 est l'estimation serrée, pas l'estimation prudente.

**Pour mika-arch.** Ce dépassement est délibéré et déclaré, pas un oubli de
découpage. La règle mika#2693 demande de découper en vols au-dessus de la borne ;
la tête de ce ticket — plus spécifique et plus récente (Vincent, 2026-11-16) —
l'interdit au groom et réserve la coupe à Vincent. Le groom suit la consigne la
plus spécifique et accepte l'ITERATE que la règle générale produit : c'est
précisément le chemin qui met la décision sous les yeux de l'opérateur. Les deux
coupes du § 4 sont le matériau de cette décision, chacune repassant sous la
borne.

---

## 1. Ce qui est mesuré sur main (@ `210f8dd8`)

### `crates/mika-agent/src/server/dashboard.rs`

`handle_skills_list` (l. 1274-1381) est conforme à la tête du ticket :

- **l. 1286-1296** — `requested` / `resolved_name` : le défaut est
  `state.default_agent`, sinon `normalize_agent_name(requested)`.
- **l. 1298-1313** — `resolve_agent(requested)` ; introuvable ⇒ **404**
  `{"error": "agent '<nom>' not found"}`, sans clé `skills`.
- **l. 1315-1318** — `SkillListFilter { source, always_on }`.
- **l. 1320** — `let registry = agent_state.skills.lock().unwrap().clone();` — le
  registre est **cloné** hors du guard, donc tout ce qui suit lit une copie
  locale. C'est le point d'accroche de ce vol : `registry.validated_warnings()`
  est disponible exactement là où `registry.skipped()` est déjà lu.
- **l. 1321-1364** — les treize champs par skill, `variants` compris.
- **l. 1366-1374** — `skipped`, non filtré, avec le commentaire qui dit pourquoi
  (« a property of the REGISTRY, not of a skill »). C'est le raisonnement que
  `warnings` réemploie **tel quel**, et le commentaire de `warnings` doit donc
  pointer vers lui plutôt que le paraphraser.
- **l. 1376-1380** — la racine : `{"agent", "skills", "skipped"}`. **Pas de
  `warnings`** — c'est le trou que ce vol comble, et le test du critère 1 est
  négatif sur `main` à cause de cette ligne-là.

### `crates/mika-agent/src/skills/mod.rs` et `skills/index.rs` (lus, **non modifiables**)

- `SkillRegistry::validated_warnings()` (`mod.rs` l. 379) rend
  `&[SkillValidationWarning]`.
- `SkillValidationWarning` (`index.rs` l. 415) porte `skill_name: String` et
  `diagnostics: Vec<SkillDiagnostic>` ; `SkillDiagnostic` (l. 708) porte `level`
  et `message: String`.
- **Le point dur du vol.** **Aucun** constructeur de `SkillRegistry` ne pose
  `validated_warnings` : `empty()`, `from_dir()`, `from_test_entries()` et
  `with_skipped()` l'initialisent tous à `Vec::new()` (l. 300, 310, 320, 330).
  Le seul producteur est `apply_load_safety_check()` (l. 396-475), qui appelle
  `validate_skill(&entry.dir)` — **une lecture du disque**. Et `skills/` est hors
  périmètre de ce vol.
- Conséquence mesurée : `test_skill_entry` **ne peut pas** produire un
  avertissement. Son `dir` vaut `/tmp/mika-test/skills/<name>`, qui n'existe pas,
  donc `validate_skill` rend `skill.toml not found` (l. 771) — un `Fail`
  **skip-worthy** (`is_skip_worthy_failure`, l. 439) — et la skill part dans
  `skipped`, pas dans `validated_warnings`. Voir l'écart É-A et la décision D2.
- Le chemin qui produit un avertissement **non fatal** sans rien toucher hors
  périmètre : un `skill.toml` valide qui déclare
  `[constraints] required_tools = ["<outil inconnu>"]` ⇒ un `Ok` (« skill.toml
  valid », l. 813) **plus** un `Warn` (l. 956-962), aucun `Fail` ⇒ la branche
  `else` de l. 464-474 range la skill dans `validated_warnings` et la **laisse
  chargée**. `scan_skills_dir` (l. 490) accepte un répertoire sans
  `system_prompt.md` (`SnippetLoadResult::Empty`, l. 628).

### `crates/mika-cli/src/commands/` — les deux clients de spirit

- `rewind_remote.rs` (1347 l.) : sept miroirs (l. 32-98, 67 lignes pour 31
  champs), `RewindRemoteIssue` à cinq variants + `Display` (l. 100-151, 52
  lignes), `rewind_url` (l. 157-159), `post` (l. 165-216). Client construit **au
  site de l'appel**, statut lu avant le corps, avec les deux commentaires qui
  disent pourquoi.
- `session_remote.rs` (724 l.) : `jeton_interne(SourceEnv)` (l. 177-185) — la
  seule lecture de `MIKA_INTERNAL_TOKEN` du crate ; `SessionRemoteIssue`
  (l. 105-116) dont le `NotFound` nomme « unknown agent, or unknown session ».
  Son en-tête (l. 12-20) porte le raisonnement « une entrée d'egress, un site de
  construction de client », qui vaut à l'identique pour ce vol.
- `rewind_remote::tests` est `pub(crate)` (l. 524-525) et expose `fake_spirit`
  (l. 574-592) et `RequeteRecue` (l. 534-539 : `methode`, `chemin`,
  `autorisation`, `corps`). `servir` (l. 545) relève `requete.uri().path()` —
  **pas** la chaîne de requête. La table de réponses est clée **par chemin**,
  donc `/api/v1/skills` s'y associe sans toucher au mécanisme ; seul le témoin
  manque.
- `DeadEndpoint::reserve()` est déjà employé dans ce crate
  (`session_remote.rs` l. 668, `chat.rs` l. 1474, `chat_turn.rs` l. 507) via
  `mika-common` en dev-dependency avec la feature `test-utils`
  (`Cargo.toml` l. 67).

### Le précédent exact de ce vol : V2d-3a (`cad12440`, `0b44a562`)

Un client posé **sans appelant de production**, testé contre `fake_spirit`, avec
son entrée d'egress. Deux choses s'en lisent directement :

1. **La forme de l'attente** est
   `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]` et non
   `#[expect(dead_code, …)]` nu (voir É-B et D5).
2. **L'entrée d'egress arrive dans le même commit que le fichier client** : « le
   lint d'egress étant bidirectionnel : le module seul serait un fichier client
   non couvert, et l'entrée seule un `client_site` fantôme » (message de
   `cad12440`). `scripts/verify-egress-manifest.sh` confronte le manifeste et le
   code **dans les deux sens** (mika#2408).

### Les caps `CLAUDE.md`

`crates/mika-agent/CLAUDE.md` pèse **94 977** o et `crates/mika-cli/CLAUDE.md`
**85 962** o, pour un cap de **100 000** (`CLAUDE_MD_MAX_BYTES`,
`crates/mika-common/tests/claude_md_size.rs` l. 40). La marge de l'agent est de
**5 023 octets** : le paragraphe « Skills listing » gagne `warnings` en quelques
mots, il n'est pas réécrit.

### Écarts entre la tête du ticket et la mesure

**É-A — le test des avertissements ne peut pas passer par `test_skill_entry`.**
La tête l'annonce (« le test des avertissements passe par `test_app()`,
`test_skill_entry` et la fabrique d'`AgentState` »). La mesure du § 1 le réfute :
`apply_load_safety_check` lit le **disque**, et le `dir` de `test_skill_entry`
n'existe pas, donc la skill serait **skippée** et non warnée. Résolution en D2,
**sans élargir le périmètre** : le test réutilise `test_app()` et
`test_agent_state` (il n'en recopie aucune, conformément à la tête) et construit
sa skill warnée sur un répertoire temporaire réel, dans le module `tests` de
`server/mod.rs`. Ce n'est pas une halte : le critère 1 tient sans toucher un
seul fichier exclu.

**É-B — la forme de l'attente est `cfg_attr(not(test), expect(…))`.** La tête
écrit « une seule attente `#[expect(dead_code, reason = "câblé au vol V3c-2b de
mika#1727")]` par item sans appelant ». Un `#[expect]` nu serait rendu
**`unfulfilled_lint_expectation`** sous `cargo clippy --all-targets`, puisque les
tests du même fichier atteignent les items : le critère 6 et le critère 7
seraient alors en conflit l'un avec l'autre. La forme du précédent `cad12440` les
satisfait tous les deux et porte la **même** attente avec la **même** `reason`,
sous un gate `not(test)`. C'est une précision de forme, pas un écart de contrat.

**É-C — l'entrée d'egress ne peut pas attendre le troisième commit.** La tête
suggère l'ordre « 3. l'entrée d'egress et la doc de `crates/mika-cli/CLAUDE.md` ».
Le lint étant bidirectionnel, le commit du client sans son entrée est un état
rouge intermédiaire. Résolution en D7 : l'entrée part **avec** le client (C2), et
C3 ne garde que la doc. L'état final de la PR est identique ; seul l'ordre
interne change, et le plan le dit plutôt que de laisser le pilote le découvrir.

---

## 2. Décisions

### D1 — `warnings` à la racine, dans la forme `{ name, messages }`

Non filtrée par `source` ni `always_on`, **comme `skipped` et pour la même
raison** : c'est une propriété du registre, pas d'une skill servie. Le
commentaire sur place **pointe** celui de `skipped` (l. 1366-1369) au lieu de le
recopier : deux énoncés jumeaux du même raisonnement dérivent, et celui qui est
déjà écrit est le bon.

`messages` est un `Vec<String>` — les `message` des diagnostics **dans l'ordre**
— et non la liste des `SkillDiagnostic` entiers. Le `level` est délibérément
laissé de côté : tout ce qui atterrit dans `validated_warnings` est un
avertissement par construction (la branche `else` de `apply_load_safety_check`),
donc un `level` servi ici serait un champ dont la seule valeur utile est déjà
portée par le nom de la clé. C'est aussi, exactement, ce que le démarrage de la
TUI affiche aujourd'hui (`chat.rs` l. 605).

### D2 — la skill warnée du test est bâtie sur un répertoire temporaire

Conséquence de É-A. Un fixture du module `tests` de `server/mod.rs` :

- `tempfile::tempdir()` (déjà employé l. 2323, 5443, 5523, 5685 de ce même
  module — aucune dépendance neuve) ;
- un sous-répertoire `avertie/` portant un `skill.toml` **valide** :
  `always_on = false`, `keywords = ["registre"]` (jamais le nom de la skill —
  l. 826 de `index.rs` en ferait un `Fail`), et
  `[constraints] required_tools = ["outil_absent"]` ;
- `SkillRegistry::from_dir(<tmp>)` puis `apply_load_safety_check()` ;
- le `TempDir` meurt à la sortie du fixture, **délibérément** : après
  `apply_load_safety_check` le registre est en mémoire et la route ne relit
  jamais le disque. Le champ `path` servi pour cette skill pointe alors vers un
  chemin disparu, et aucune assertion ne le lit — c'est le test du critère 4 de
  V3c-1 qui porte `path`, sur une skill de `test_skill_entry`.

`always_on = false` n'est pas un détail : c'est ce qui rend le second volet du
critère 1 non vacant. Sous `?always_on=true` la skill quitte `skills` et
`warnings` doit rester **entier** — l'assertion échoue si quelqu'un câble un jour
`warnings` sur l'itérateur filtré.

**Pourquoi ce chemin plutôt qu'un constructeur de test.** Un
`SkillRegistry::with_validation_warnings(…)` serait trois lignes et toucherait
`crates/mika-agent/src/skills/mod.rs`, que la tête déclare intact. La tête dit
aussi quoi faire dans ce cas : « Si le groom juge qu'un critère ne tient pas sans
toucher un fichier exclu, il s'arrête et le dit, au lieu d'élargir. » Le critère
**tient** sans élargir, par le fixture ci-dessus — donc pas de halte sur ce
point, et pas d'élargissement.

### D3 — le client porte son propre vocabulaire de refus

`Refus`, cinq variants, et **pas** `SessionRemoteIssue` :

| variant | déclenchement | ce que le message nomme |
|---|---|---|
| `AgentInconnu { agent, url }` | 404 | le nom **demandé**, et l'URL. Aucune mention de session |
| `JetonRefuse { url }` | 401 | `MIKA_INTERNAL_TOKEN` |
| `Injoignable { url, cause }` | erreur de transport | l'URL, et `None` de statut par construction du variant |
| `Statut { status, url, body }` | tout autre statut | le statut, l'URL et le corps |
| `CorpsIllisible { url, body, cause }` | 200 au corps illisible | l'URL, le corps reçu et la cause `serde` |

La raison du non-réemploi est mesurée, pas esthétique : le `NotFound` de
`SessionRemoteIssue` écrit « unknown agent, or unknown session », et sur cette
route un 404 ne peut vouloir dire qu'agent inconnu (`dashboard.rs` l. 1306). Un
message qui nomme une session enverrait l'opérateur chercher la panne là où elle
n'est pas. Deux variants distincts là où le voisin en a un seul —
`Injoignable` et `Statut` plutôt qu'un `Unexpected { status: Option<u16> }` —
parce que le critère 3 demande cinq refus **qui se distinguent** : un `Option` à
discriminer dans le test est exactement ce que le type doit porter à sa place.

**`CorpsIllisible` n'est pas un succès.** Le 200 dont le corps ne se désérialise
pas sort par `Err`, jamais par un miroir à champs vides.

### D4 — `lire(base_url, token, agent)`, et le jeton par argument

- GET sur `/api/v1/skills`, le nom d'agent passé par
  `.query(&[("agent", agent)])` : **reqwest encode**, donc un nom portant un
  caractère réservé part encodé sans qu'un `format!` ait à le savoir.
- `Authorization: Bearer <jeton>` — la route est sous
  `require_dashboard_or_internal_token`, et le jeton interne y suffit.
- Client construit **au site de l'appel**, comme `rewind_remote::post` (l. 170)
  et `session_remote::post` : c'est ce que déclare l'entrée d'egress, qu'un
  client reçu en argument rendrait aveugle.
- Statut lu **avant** le corps, comme les deux voisins.
- Le jeton n'est **pas** lu dans ce module : `lire` le reçoit. Son futur appelant
  (V3c-2b) le lira par `session_remote::jeton_interne`, le site unique du crate.

### D5 — l'attente de code mort, et son unique forme

`#[cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))]`
sur chaque item sans appelant : les cinq structs de miroir et `lire`. La `reason`
est celle de la tête, mot pour mot. Le gate `not(test)` est la part de É-B.

**Le compte se mesure, il ne se devine pas** — l'en-tête de `session_remote.rs`
(l. 30-35) porte déjà la discipline : une passe verte de
`cargo clippy --all-targets -p mika-cli` tient le compte exact **dans les deux
sens**, une attente de trop étant rendue « unfulfilled » et une de moins
laissant un `dead_code` nu. Le pilote suit la mesure, pas ce tableau.

### D6 — `fake_spirit` gagne un témoin, et un seul

`RequeteRecue` gagne `pub(crate) requete: Option<String>`, rempli par
`requete.uri().query()`. `Option` et non `String` : une requête absente et une
requête vide ne sont pas le même fait, et c'est la distinction qui fait la valeur
du témoin.

**Aucun second factice.** Les tests de ce vol empruntent `fake_spirit`, comme
V2d-3a l'a fait avant eux : un second témoin de socket ouvrirait une seconde
vérité sur ce que le serveur reçoit. La table de réponses étant clée par chemin,
`/api/v1/skills` s'y associe sans toucher au mécanisme.

Les tests existants de `rewind_remote.rs` et `session_remote.rs` **ne changent
pas** : ils construisent `RequeteRecue` nulle part (seul `servir` le fait) et
lisent les champs par nom (critère 4).

### D7 — l'entrée d'egress part avec le client

Conséquence de É-C. `cli-spirit-skills` :

```toml
id                 = "cli-spirit-skills"
destination        = "MIKA_SPIRIT_URL (défaut http://localhost:8080)"
destination_source = "config"
class              = "internal"
data               = "un GET vers /api/v1/skills : le nom de l'agent en paramètre de requête, et le jeton interne en en-tête Authorization ; aucun corps"
logged             = false
client_site        = "crates/mika-cli/src/commands/skills_remote.rs"
call_site          = "crates/mika-cli/src/commands/skills_remote.rs"
owner              = "@samidarko"
```

Une entrée, un site de construction de client : la propriété que l'en-tête de
`session_remote.rs` (l. 12-20) paye déjà d'une soixantaine de lignes dupliquées,
et que ce vol ne dépense pas à nouveau — il ne met rien en commun.

### D8 — pourquoi ce vol ne porte pas de section `## Fire-Disposition`

La règle mika#2306 arme la section sur la livraison d'un **détecteur** : « tout
code dont le chemin de succès est *aucune violation trouvée* » — lint, garde CI,
scan structurel, garde EndTurn, validateur de schéma.

Ce vol n'en livre aucun. Ses sept tests sont des tests de **comportement** : d'une
route (le critère 1) et d'une porte client (les critères 2 et 3). Leur chemin de
succès est « la réponse porte ce contenu » ou « ce refus nomme cette cause », et
ils échouent en nommant une valeur attendue. Le critère 1 est un test de
régression négatif — il rougit sur `main`, où la clé `warnings` n'existe pas —
mais un test négatif n'est pas un scanner : il n'a ni population à parcourir, ni
allowlist à tenir, ni risque de passer vert en ne lisant rien.

C'est la même disposition que V3c-1 (D7 de son plan), et pour la même raison. La
règle dit « si le plan ne livre AUCUN détecteur, la section n'est pas requise :
ne l'invente pas » — ce paragraphe est le raisonnement visible qui tient la place
d'une section vide.

### D9 — ce que ce vol ne fait pas

- **Aucun câblage.** `app.skills`, l'autocomplétion, `/skills`, `/skill`, les
  messages de démarrage, `spawn_agent_worker`, `updated_skills` : V3c-2b. Aucun
  lecteur de `app.skills` ne change, et `chat.rs` comme tout
  `crates/mika-cli/src/tui/` restent **inchangés** (critère 5).
- **Pas le retrait de l'assemblage** du registre dans la TUI, ni celui de
  `migrate_disabled_markers_async` : V3c-2b.
- **Pas le test É1** (le constructeur qui combine entrées et skills ignorées) :
  V3c-2c.
- **Pas de constructeur de test dans `skills/`** (D2).

La suite se note `Tracked in: senara-solutions/mika#1727` dans le corps de la PR.
**Ce vol n'ouvre aucun ticket** (gel jusqu'au 01/12) : un manque ou un reste va en
commentaire sur mika#1727 ou dans la PR.

---

## 3. Ordre de commit

Un commit par garde, **après que la garde est verte avec son test**, et non en fin
de vol. Les tests ne se reportent jamais à un vol suivant. C'est l'ordre suggéré
par la tête du ticket, avec le seul déplacement qu'É-C impose (l'egress monte en
C2).

**Cet ordre vaut sous la coupe A comme sous la coupe B** (§ 4) : C1 est le vol
serveur, C2+C3 le vol client.

### C1 — la route sert `warnings` (critère 1) — ≈ 93 lignes

`crates/mika-agent/src/server/dashboard.rs`, dans `handle_skills_list` seulement :
le bloc `warnings` juste après celui de `skipped`, et la clé à la racine.

`crates/mika-agent/src/server/mod.rs`, dans son module `tests` seulement :

- un fixture `test_registry_avertie()` (D2) ;
- un agent de plus dans l'état multi-agents, portant ce registre. Ajouter un
  agent ne touche aucune assertion de V3c-1 : les quatre tests existants visent
  `skills-a`, `skills-b` et `skills-skipped` nommément ;
- le test du critère 1, par le helper `skills_get` existant, en **deux**
  requêtes : sans filtre puis `?always_on=true`.

`crates/mika-agent/CLAUDE.md` : le paragraphe « Skills listing (#606, mika#1727
V3c-1) » nomme `warnings` et sa forme. **Quelques mots, pas une réécriture** — la
marge sous le cap est de 5 023 octets.

Vérification du commit : `cargo test -p mika-agent` et
`cargo test -p mika-common --test claude_md_size`.

### C2 — le client, son miroir, ses refus, son témoin et son egress (critères 2, 3, 4, 6) — ≈ 437 lignes

- `crates/mika-cli/src/commands/rewind_remote.rs`, module `tests` seulement : le
  champ `requete` de `RequeteRecue` et son relevé dans `servir` (D6).
- `crates/mika-cli/src/commands/skills_remote.rs`, fichier neuf : en-tête de
  module, les cinq miroirs, `Refus` + `Display`, `lire`, les attentes (D3, D4,
  D5), puis **en fin de fichier** le module `#[cfg(test)] mod tests` — le scanner
  d'egress coupe la production au premier `#[cfg(test)]`, et un item d'après en
  sortirait (le commentaire de `rewind_remote.rs` l. 515-516 le dit, et ce
  fichier le répète pour lui-même).
- `crates/mika-cli/src/commands/mod.rs` : la déclaration du module.
- `docs/egress/egress-manifest.toml` : l'entrée `cli-spirit-skills` (D7).

Les sept tests : le miroir complet (critère 2, avec méthode / chemin / requête /
autorisation attestés dans le **même** test), l'encodage d'un nom réservé, et les
cinq refus (critère 3), l'injoignable par `DeadEndpoint::reserve()` dont le guard
est **tenu pendant tout l'appel** (mika#2569 — un port libéré se fait servir par
un factice voisin du même binaire).

Vérification du commit : `cargo build -p mika-cli --bin mika` **d'abord** (c'est
la cible où `dead_code` mord, donc celle qui valide les attentes), puis
`cargo test -p mika-cli`, puis `cargo clippy --all-targets -p mika-agent -p mika-cli`,
puis `bash scripts/verify-egress-manifest.sh --report`.

### C3 — la doc du client (critère 7) — ≈ 4 lignes

Un paragraphe de `crates/mika-cli/CLAUDE.md` : le client existe, il n'est pas
encore câblé, et V3c-2b le câblera en retirant ses attentes.
`cargo test -p mika-common --test claude_md_size` reste vert.

### Fin de pipeline

Dans l'ordre, avant d'ouvrir la PR :

1. `cargo build -p mika-cli --bin mika`
2. `cargo test -p mika-agent` et `cargo test -p mika-cli`
3. `cargo test -p mika-common --test claude_md_size`
4. `cargo clippy --all-targets -p mika-agent -p mika-cli`
5. `cargo fmt`
6. `bash scripts/verify-egress-manifest.sh --report`
7. `bash scripts/verify-pipeline.sh origin/main`

**Si l'un des deux derniers est rouge, le pilote s'arrête et le dit** (critère 7).

La PR porte `Refs mika#1727` et **aucun** `Closes`, `Fixes` ni `Resolves`, y
compris si le gabarit de reprise l'ouvre (label `wip-rescue`) : ce gabarit écrit
`Closes #N` par défaut, et celui qui en voit un le corrige **avant toute revue**,
puis vérifie que `gh pr view <N> --json closingIssuesReferences` rend `[]`.
Le corps porte aussi `Code review: delegated to mika-qa (mika#2694)` et
`Tracked in: senara-solutions/mika#1727`.

### Halte du pilote sur la borne

Après **chaque** commit, `git diff --numstat origin/main...HEAD`, plan exclu.

- **> 475 au total ⇒ arrêt.** Pas de commit suivant, pas de PR. Dernier message :
  « HALTE borne : N lignes pour 475 », avec le compte par commit.
- **> 425 après le premier commit ⇒ arrêt aussi**, dit de la même façon : le reste
  ne tiendrait pas.

Les commentaires comptent dans la borne **et ne se rabotent pas pour la tenir**.
Un commentaire dit le pourquoi qu'on ne lit pas dans le code. Si le code écrit
dépasse, le pilote s'arrête selon la halte ci-dessus ; il ne retire pas après
coup les commentaires qui portent un pourquoi.

**Ce plan estime 534 dans cette unité** (§ *Taille estimée*) : sous la coupe A
comme sous la coupe B, chaque vol repasse sous sa borne. Dispatché **tel quel**,
sans coupe, ce vol touche la halte des 475 au cours de C2.

### Si le code s'écarte de ce plan

La QA juge contre le plan de la branche. Si le pilote doit s'écarter d'un point —
un fichier, un critère, un nom, une forme d'attente — **il amende le plan sur la
branche dans un commit `docs(plans)` avant de pousser le code qui s'en écarte**,
en nommant le point et la raison. Un plan qui dit une chose pendant que le code en
fait une autre est bloqué par la QA, même quand le code est juste.

---

## 4. Les deux coupes candidates — nommées, chiffrées, **non pratiquées**

Le groom ne tranche pas (§ 0). Les deux découpages ci-dessous repassent sous la
borne et ne reportent **aucun test** hors du vol qui livre sa garde.

### Coupe A — deux vols : le serveur, puis le client

| vol | contenu | architecte | borne du vol |
|---|---|---|---|
| **A-i** | C1 — la route sert `warnings`, son test, la doc agent | 85 | 93 |
| **A-ii** | C2 + C3 — le client, le témoin, l'egress, la doc cli | 432 | 441 |

A-i est très au large. A-ii tient sous **452** (la borne de la tête du ticket)
mais reste **3 lignes au-dessus de 429** (la borne mika#2693 de ce ticket). C'est
la frontière même que Vincent doit trancher : relever A-ii à 452, ou couper
encore. Les deux vols sont mergeables seuls — A-ii ne lit pas `warnings`, il le
désérialise.

### Coupe B — trois vols : le serveur, le miroir, les refus

| vol | contenu | architecte | borne du vol |
|---|---|---|---|
| **B-i** | identique à A-i | 85 | 93 |
| **B-ii** | le miroir, `lire`, le témoin de requête, l'egress, la doc cli ; `Refus` à **deux** variants (`Statut`, `CorpsIllisible`) + leurs deux tests ; les tests des critères 2 et 4 | 265 | 278 |
| **B-iii** | les trois variants distinctifs (`AgentInconnu`, `JetonRefuse`, `Injoignable`) + leurs trois tests | 175 | 175 |

Chaque vol porte **ses** tests : B-ii livre deux refus et les teste, B-iii livre
les trois autres et les teste. Aucun test ne se reporte. Les trois vols passent
largement sous 429. Le prix est un `Refus` qui gagne trois variants en deux
temps — un type qui s'élargit, jamais un type qui mentait.

### Ce que le groom ne fait pas

Il ne choisit pas entre A et B, il ne relève aucune borne, et il n'écrit le plan
d'aucun des sous-vols. La tête du ticket est explicite : « une coupe demande le
mot de Vincent ». Ce mot posé, chaque sous-vol se groome depuis ce document sans
nouvelle passe de mesure — § 1 et § 2 valent inchangés pour tous.

---

## 5. Definition of Done

- `handle_skills_list` sert `warnings` à la racine, dans la forme
  `[{ name, messages }]`, non filtrée par `source` ni `always_on`.
- Le paragraphe « Skills listing » de `crates/mika-agent/CLAUDE.md` nomme
  `warnings`, et `claude_md_size` reste vert.
- `crates/mika-cli/src/commands/skills_remote.rs` existe, porte son miroir
  complet (les treize champs, `agent`, `skipped`, `warnings`), `lire`, et cinq
  refus distincts ; il n'a **aucun** appelant de production et porte une attente
  `dead_code` par item sans appelant, et rien d'autre.
- `fake_spirit` relève la chaîne de requête, et aucun test existant de
  `rewind_remote.rs` ni de `session_remote.rs` ne change.
- L'entrée `cli-spirit-skills` est au manifeste, et
  `verify-egress-manifest.sh --report` est vert.
- Un paragraphe de `crates/mika-cli/CLAUDE.md` dit que le client existe, qu'il
  n'est pas câblé, et ce que V3c-2b en fera.
- `chat.rs`, tout `crates/mika-cli/src/tui/`, `session_remote.rs`,
  `crates/mika-common/`, `crates/mika-a2a/`, `crates/mika-gateway/`,
  `dashboard/`, `server/state.rs` et `skills/` sont **inchangés**. Aucun fichier
  du plan de contrôle n'est touché, `docs/egress/egress-manifest.toml` excepté.
- `cargo build -p mika-cli --bin mika`, `cargo test -p mika-agent`,
  `cargo test -p mika-cli`, `cargo test -p mika-common --test claude_md_size` et
  `cargo clippy --all-targets -p mika-agent -p mika-cli` sont verts, et
  `scripts/verify-pipeline.sh origin/main` aussi.
- La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`.

---

## 6. Acceptance criteria

Transcrits de la tête du ticket (§ *Critères de V3c-2a*).

1. Sur un agent dont le registre porte une skill chargée avec au moins un
   diagnostic, `GET /api/v1/skills?agent=<a>` rend à la racine `warnings`, avec
   le nom de cette skill et le message de son diagnostic. Avec `always_on=true`,
   `warnings` reste entier. Ce test-ci est le test négatif du serveur : sur
   `main`, la clé n'existe pas.
2. `skills_remote::lire`, contre `fake_spirit` servant une réponse de la forme de
   la route (deux skills, une ignorée, un avertissement), rend un miroir où
   chacun des treize champs, `agent`, `skipped` et `warnings` se relit. Le
   factice atteste dans le même test la méthode `GET`, le chemin
   `/api/v1/skills`, la requête `agent=<nom>` et `Authorization: Bearer <jeton>`.
   Un nom d'agent qui porte un caractère réservé arrive encodé.
3. Chacun des cinq refus est atteint par un test et se distingue des autres : 404
   (le nom demandé dans le message, et aucune mention de session), 401 (nomme
   `MIKA_INTERNAL_TOKEN`), injoignable (par `DeadEndpoint::reserve()`, jamais un
   port libéré), autre statut (500 : le statut et l'URL), 200 au corps illisible.
4. Aucun test existant de `rewind_remote.rs` ni de `session_remote.rs` ne change,
   hors l'ajout du champ de requête à `RequeteRecue`.
5. Seuls changent les fichiers listés par la tête du ticket. `chat.rs` et tout
   `crates/mika-cli/src/tui/` sont inchangés.
6. Les seules attentes neuves de `crates/mika-cli/` sont les
   `expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")` de
   `skills_remote.rs` (sous le gate `not(test)` d'É-B). Aucun `allow` neuf, et
   aucune attente neuve dans `crates/mika-agent/`.
7. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-agent`
   et `cargo test -p mika-cli` sont verts, de même que
   `cargo test -p mika-common --test claude_md_size`.
   `cargo clippy --all-targets -p mika-agent -p mika-cli` est vert. Le pilote
   lance `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il
   trouve l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

---

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `dashboard.rs` — le bloc `warnings` et la clé à la racine (C1) | 20 |
| `server/mod.rs` tests — fixture `test_registry_avertie` + l'agent de plus (C1) | 30 |
| test du critère 1 — deux requêtes, `warnings` entier sous filtre (C1) | 35 |
| `rewind_remote.rs` tests — le champ `requete` et son relevé (C2) | 6 |
| `commands/mod.rs` — la déclaration du module (C2) | 2 |
| `skills_remote.rs` — en-tête de module (C2) | 18 |
| `skills_remote.rs` — les cinq miroirs, 23 champs, attentes comprises (C2) | 55 |
| `skills_remote.rs` — `Refus` à cinq variants + `Display` (C2) | 56 |
| `skills_remote.rs` — `lire` (C2) | 57 |
| `skills_remote.rs` tests — préambule, const du corps JSON, helper partagé (C2) | 46 |
| test du critère 2 — les 23 champs relus + méthode/chemin/requête/jeton (C2) | 48 |
| test de l'encodage d'un nom réservé (C2) | 18 |
| tests des cinq refus — 5 × 22 (C2) | 110 |
| `docs/egress/egress-manifest.toml` — entrée `cli-spirit-skills` (C2) | 11 |
| `crates/mika-agent/CLAUDE.md` — `warnings` au paragraphe existant (C1) | 2 |
| `crates/mika-cli/CLAUDE.md` — le paragraphe du client non câblé (C3) | 4 |

Total estimé : 517 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et les deux dépassements.** La ligne `Total estimé :` ci-dessus
est dans l'unité de l'architecte : elle exclut les 11 lignes d'egress (`docs/`) et
les 6 des deux `CLAUDE.md`, donc **517** pour une borne de **429** (mika#2693) ou
**452** (tête du ticket). L'unité de la borne du vol
(`git diff --numstat origin/main...HEAD`, plan exclu, doc comprise) les inclut :
**534** pour une borne de **475**. **Les deux bornes sont dépassées, et le groom
s'arrête** (§ 0). Il ne recoupe pas : les deux coupes candidates du § 4 sont
nommées et chiffrées, jamais pratiquées.

Répartition par commit : **C1 ≈ 87**, **C2 ≈ 427**, **C3 ≈ 4** dans l'unité de
l'architecte (**93 / 437 / 4** dans celle de la borne du vol). C'est cette
répartition qui fait de la coupe A une coupe **sur un joint existant** : C1 est
un vol entier, C2+C3 en est un autre.

**Pourquoi cette estimation est serrée, et non prudente.** Chaque poste est calé
sur un précédent **mesuré** dans ce dépôt, pas deviné :

- **les miroirs à 55** : les sept miroirs de `rewind_remote.rs` (l. 32-98) pèsent
  67 lignes pour 31 champs, commentaires compris. 23 champs à ce débit donnent
  50 ; +5 pour les cinq attentes ;
- **`Refus` + `Display` à 56** : `RewindRemoteIssue` et son `Display`
  (l. 100-151) pèsent 52 lignes pour cinq variants ; ceux-ci portent un champ de
  plus en moyenne ;
- **`lire` à 57** : `rewind_remote::post` pèse 48 lignes (l. 165-216) **plus** 8
  pour `rewind_url` ; `lire` y ajoute la désérialisation du corps ;
- **les tests à ~22-48 pièce** : `session_remote::tests` (l. 375-724) pèse 350
  lignes pour six tests et ses helpers, soit ~45 par test ; les cinq refus sont
  estimés à **22** — la moitié — parce qu'ils partagent un helper et un const, et
  c'est l'hypothèse la plus optimiste du tableau ;
- **le total, contre une mesure de bout en bout** : V2d-3a (`0b44a562`) a coûté
  **357** lignes dans `session_remote.rs` pour un client de la même nature, à
  **trois** champs de miroir et six tests. Celui-ci en a 23 et sept, et le vol
  porte en plus son côté serveur.

Un poste tomberait-il de 10 % que le total resterait au-dessus des trois bornes.
C'est pourquoi le § 0 conclut à la halte plutôt qu'à une marge à surveiller.

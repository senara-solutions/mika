# mika#1727 — vol V3c-2a-ii : la TUI gagne le client des skills

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3c-2a-ii (le second des trois vols de V3c-2a, coupe B tranchée par Vincent le 2026-11-17 ; V3c-2a-i est mergé, V3c-2a-iii portera les refus 404 et 401)
- **Branche** : `feat/1727/v3c2a-ii-la-tui-gagne-le-client-des-skills`, neuve depuis `main`
- **Base mesurée** : `main` @ `75ac3761` (le merge de V3c-2a-i, mika#3101)
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus 475 lignes ajoutées, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.

**Ce vol ne câble rien.** Il pose un client de `GET /api/v1/skills?agent=<nom>`
et ses tests. Aucun lecteur de `app.skills` ne change ; le module n'a aucun
appelant de production à la fin du vol, et c'est l'état attendu.

---

## 0. D'où vient ce plan, et ce qu'il hérite

Le plan **005** de la branche `feat/1727/v3c2a-la-route-des-skills-a-son-client`
(`docs/plans/2026-11-16-005-…`) était une **halte de groom** : le vol V3c-2a
entier pesait 517 lignes dans l'unité de l'architecte pour une borne de 429, et
le plan nommait deux coupes candidates sans en pratiquer aucune, la coupe étant
réservée à Vincent. **Vincent a tranché la coupe B le 2026-11-17.** V3c-2a-i (le
serveur) est mergé ; ce document est le plan du second vol.

Ce plan reprend du plan 005, **sans nouvelle passe de mesure côté client** :
la mesure de `crates/mika-cli/src/commands/` (§ 1), le précédent V2d-3a, les
écarts É-B et É-C, les décisions D3 à D7 (devenues D1 à D5 ici) et le commit C2.
Il en **retire** `AgentInconnu` et `JetonRefuse`, qui sont à V3c-2a-iii. Les
numéros de ligne sont recalés sur `main` @ `75ac3761` — et le recalage est un
no-op mesuré : `git diff --stat 210f8dd8 origin/main -- crates/mika-cli/ docs/egress/`
rend **vide**, donc tous les numéros du plan 005 côté client valent tels quels.

### La tête du ticket redistribue les variants, et elle gagne

La coupe B du plan 005 donnait à son vol *ii* **deux** variants (`Statut`,
`CorpsIllisible`) et à son vol *iii* les **trois** autres (`AgentInconnu`,
`JetonRefuse`, `Injoignable`). La tête du ticket (Vincent, 2026-11-17) donne à ce
vol **trois** variants — `Injoignable`, `Statut`, `CorpsIllisible` — et renvoie
`AgentInconnu` (404) et `JetonRefuse` (401) à V3c-2a-iii.

**C'est la tête qui vaut**, étant plus récente que le plan dont ce document
hérite, et le déplacement est meilleur que ce que le plan 005 proposait : les
trois variants de ce vol sont la partition **par forme de réponse** — aucune
réponse / une réponse non-200 / une réponse 200 que le miroir ne sait pas lire —
donc le type est complet et cohérent dès ce vol. V3c-2a-iii n'y découpe ensuite
que deux **cas métier** qui se détachent de `Statut`. Un type qui s'élargit,
jamais un type qui mentait.

**Conséquence à écrire dans le code** : dans ce vol, un 404 et un 401 sortent par
`Statut`, avec leur statut, leur URL et leur corps. Aucun test de ce vol ne doit
prétendre qu'ils sont traités à part, et le `match` ne porte aucun bras mort en
attente de V3c-2a-iii.

---

## 1. Ce qui est mesuré sur `main` (@ `75ac3761`)

### La route, côté serveur (lue, **non modifiable** par ce vol)

`handle_skills_list` (`crates/mika-agent/src/server/dashboard.rs` l. 1274-1400)
rend à la racine `{ agent, skills, skipped, warnings }`. Les **treize** champs par
skill, dans l'ordre où la route les écrit : `name`, `description`, `origin`,
`enabled`, `always_on`, `tools` (un **compte**), `has_override`, `version`,
`timeout_secs`, `keywords`, `tool_names`, `path`, `variants`
(`{ total, providers }`, `providers` étant un objet nom-de-provider → compte de
modèles). `skipped` liste des `{ name, reason }`, `warnings` des
`{ name, messages }` (`messages` = `Vec<String>`, le `level` étant délibérément
absent côté serveur). Un agent inconnu rend **404** `{"error": "agent '<nom>' not found"}`,
**sans clé `skills`**.

### Les deux clients de spirit de `crates/mika-cli/src/commands/`

- `rewind_remote.rs` (1347 l.) : sept miroirs (l. 32-98 — **67 lignes pour 31
  champs**, commentaires compris), `RewindRemoteIssue` à cinq variants +
  `Display` (l. 100-151, **52 lignes**), `rewind_url` (l. 157-159), `post`
  (l. 165-216, **48 lignes**). Client construit **au site de l'appel** avec le
  commentaire qui dit pourquoi (l. 170-171 : « c'est ce que déclare l'entrée
  d'egress `cli-spirit-rewind`, qu'un client en argument rendrait aveugle »), et
  **statut lu avant le corps** (l. 185-186 : « un corps d'erreur désérialisé
  comme un succès est la panne à ne pas produire »).
- `session_remote.rs` (724 l.) : `jeton_interne(SourceEnv)` (l. 177) — la **seule**
  lecture de `MIKA_INTERNAL_TOKEN` du crate ; `SessionRemoteIssue` (l. 105) dont
  le `NotFound` nomme « unknown agent, or unknown session ». Son en-tête
  (l. 12-21) porte le raisonnement « **une entrée d'egress, un site de
  construction de client** », qui vaut à l'identique ici : une soixantaine de
  lignes dupliquée est payée délibérément plutôt qu'un `post` partagé qui
  obligerait une entrée à décrire deux familles de routes.
- `commands/mod.rs` (33 l.) : déclarations alphabétiques ; `skills_remote` se
  pose **entre** `skills` (l. 26) et `skills_variants` (l. 27). Une ligne.

### Le banc de test

`rewind_remote::tests` est `pub(crate)` (l. 525) et expose `fake_spirit` (l. 574)
et `RequeteRecue` (l. 535 : `methode`, `chemin`, `autorisation`, `corps`).

- `servir` (l. 548) relève `requete.uri().path()` (l. 550) — **pas** la chaîne de
  requête. La table de réponses est clée **par chemin** (l. 559), donc
  `/api/v1/skills` s'y associe sans toucher au mécanisme ; seul le témoin manque.
- **La seule construction littérale de `RequeteRecue` est `servir`** (l. 560) —
  vérifié par `grep -rn "RequeteRecue" crates/mika-cli/src`. Les onze autres
  occurrences, dans `rewind_remote.rs`, `session_remote.rs`, `chat.rs` et
  `tui/commands/handlers.rs`, sont des **types** (`Vec<RequeteRecue>`) ou des
  commentaires. **C'est ce qui rend l'ajout d'un champ compatible avec « tout
  `crates/mika-cli/src/tui/` reste intact »** : `handlers.rs` emprunte le struct
  (l. 2713) et n'en construit aucun. Un champ de plus n'y touche pas une ligne.
- `DeadEndpoint::reserve()` est déjà employé dans ce crate
  (`session_remote.rs` l. 668, `chat.rs`, `chat_turn.rs`) via `mika-common` en
  dev-dependency avec la feature `test-utils`. Le motif mika#2569 y est écrit
  textuellement l. 665-667, et le guard est **tenu pour toute la durée de
  l'appel**.

### Le précédent exact de ce vol : V2d-3a

Un client posé **sans appelant de production**, testé contre `fake_spirit`, avec
son entrée d'egress. Deux faits mesurés (`git show --numstat`) :

| révision | portée | lignes ajoutées, plan exclu |
|---|---|---|
| `cad12440` (commit du transport) | `session_remote.rs` 211 + `mod.rs` 1 + egress 11 | **223** |
| `0b44a562` (vol V2d-3a entier) | `session_remote.rs` 357 + `mod.rs` 1 + egress 11 | **369** |

Et deux propriétés de forme s'en lisent directement :

1. **La forme de l'attente** est
   `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]` et non un `#[expect]`
   nu — `cad12440` l. 137 (voir É-B et D3).
2. **L'entrée d'egress arrive dans le même commit que le fichier client**, parce
   que `scripts/verify-egress-manifest.sh` confronte manifeste et code **dans les
   deux sens** (mika#2408) : « le module seul serait un fichier client non
   couvert, et l'entrée seule un `client_site` fantôme » (message de `cad12440`).

Il ne subsiste **aucune** attente `cfg_attr(not(test), expect(dead_code, …))`
dans `crates/mika-cli/src/` aujourd'hui — grep à zéro, cohérent avec l'en-tête de
`session_remote.rs` qui déclare ses trois portes câblées. Les attentes de ce vol
seront donc les seules du crate, ce qui rend le critère 5 mesurable par grep.

### Le cap de `crates/mika-cli/CLAUDE.md`

**85 962 octets** pour un cap de **100 000** (`CLAUDE_MD_MAX_BYTES`,
`crates/mika-common/tests/claude_md_size.rs`). Marge : **14 038 octets**. Le
paragraphe de ce vol en coûte quelques centaines.

### Écarts entre la tête du ticket et la mesure

**É-B — la forme de l'attente est `cfg_attr(not(test), expect(…))`.** La tête
écrit déjà la forme complète, gate compris, et nomme son précédent (`cad12440`) :
cet écart est **résolu dans la tête** et n'est reporté ici que pour le dire. Un
`#[expect]` nu serait rendu **`unfulfilled_lint_expectation`** sous
`cargo clippy --all-targets`, les tests du même fichier atteignant les items — les
critères 5 et 7 seraient alors en conflit l'un avec l'autre.

**É-C — l'entrée d'egress ne peut pas attendre un commit tardif.** L'ordre
suggéré par la tête met l'egress au point 2, avec le client : c'est déjà la
résolution, et elle est **respectée telle quelle** par C2 ci-dessous. Noté parce
que le plan 005 portait l'écart contre une tête antérieure qui, elle, la
reportait au troisième commit.

**Aucun autre écart.** Les sept critères de la tête tiennent sans toucher un
fichier exclu, donc ce plan ne porte **pas** de halte.

---

## 2. Décisions

### D1 — le client porte son propre vocabulaire de refus, à trois variants

`SkillsRemoteIssue`, et **pas** `SessionRemoteIssue` :

| variant | déclenchement | ce que le message nomme |
|---|---|---|
| `Injoignable { url, cause }` | erreur de transport | l'URL et la cause ; **aucun statut**, par construction du variant |
| `Statut { status, url, body }` | tout statut autre que 200, **404 et 401 compris** | le statut, l'URL et le corps |
| `CorpsIllisible { url, body, cause }` | un 200 dont le corps ne se désérialise pas | l'URL, le corps reçu et la cause `serde` |

**Pourquoi pas le type du voisin.** Le `NotFound` de `SessionRemoteIssue` écrit
« unknown agent, or unknown session » ; sur cette route un 404 ne peut vouloir
dire qu'**agent inconnu** (`dashboard.rs` l. 1306). Un message qui nomme une
session enverrait l'opérateur chercher la panne là où elle n'est pas. Ce n'est pas
esthétique, c'est mesuré.

**Pourquoi `Injoignable` et `Statut` sont deux variants là où les deux voisins
n'en ont qu'un.** `rewind_remote` et `session_remote` rangent une erreur de
transport dans `Unexpected { status: None, … }`. Le critère 2 demande trois refus
**qui se distinguent** : un `Option<u16>` à discriminer dans le test est
exactement ce que le type doit porter à sa place. `Injoignable` ne peut pas
porter de statut, et c'est le type qui le dit.

**`CorpsIllisible` n'est jamais un succès.** Un 200 au corps illisible sort par
`Err`, jamais par un miroir à champs vides — c'est le point que la tête du ticket
souligne, et il se teste par un `assert!` négatif sur l'obtention d'un miroir.

**Le nom du type suit la convention du répertoire, les variants suivent la tête.**
`<Module>Issue` est lu par un appelant d'un autre module (V3c-2b) et porte donc
le module, comme ses deux voisins ; les variants sont lus dans un `match` à
l'intérieur du vocabulaire français de ce fichier, et la tête les nomme ainsi.

### D2 — `lire(base_url, token, agent)`, et le jeton par argument

```rust
pub(crate) async fn lire(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<SkillsMirror, SkillsRemoteIssue>
```

- **GET** sur `/api/v1/skills`, le nom d'agent passé par
  `.query(&[("agent", agent)])` : **reqwest encode**, donc un nom portant un
  caractère réservé part encodé sans qu'un `format!` ait à le savoir. C'est le
  second volet du critère 1, et c'est ce qui le rend non vacant : un `format!`
  passerait le test sur un nom simple et casserait sur `mika/dev`.
- `Authorization: Bearer <jeton>` — la route est sous
  `require_dashboard_or_internal_token`, et le jeton interne y suffit.
- **Client construit au site de l'appel**, comme `rewind_remote::post` (l. 170) :
  c'est ce que déclare l'entrée d'egress, qu'un client reçu en argument rendrait
  aveugle. Le commentaire sur place **pointe** celui du voisin plutôt que de le
  recopier.
- **Statut lu avant le corps**, comme les deux voisins.
- **Le jeton n'est pas lu dans ce module** : `lire` le reçoit. Son futur appelant
  (V3c-2b) le lira par `session_remote::jeton_interne`, le site unique du crate —
  `grep -rn "fn jeton_interne" crates/mika-cli/src` doit continuer de rendre
  **une** ligne.
- Un `skills_url(base_url)` à part n'est pas construit : il n'y a **qu'une**
  route, donc le `trim_end_matches('/')` qui absorbe un `base_url` à barre finale
  vit dans `lire`. `rewind_url` existe chez le voisin parce que trois routes le
  partagent.

### D3 — l'attente de code mort, et son unique forme

`#[cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))]`
sur chaque item sans appelant, et **rien d'autre** : aucun `allow`, aucune autre
`reason`. La `reason` est celle de la tête, mot pour mot ; le gate `not(test)` est
la part d'É-B.

**Le compte se mesure, il ne se devine pas.** Une passe verte de
`cargo clippy --all-targets -p mika-cli` le tient exact **dans les deux sens** :
une attente de trop est rendue « unfulfilled », une de moins laisse un
`dead_code` nu. `cad12440` n'en portait **qu'une** pour tout son transport — la
preuve que ce compte ne se prédit pas depuis une liste d'items. Le pilote suit la
mesure, jamais une énumération de ce plan. V3c-2b les retire en câblant.

### D4 — `fake_spirit` gagne un témoin, et un seul

`RequeteRecue` gagne `pub(crate) requete: Option<String>`, rempli par
`requete.uri().query()` dans `servir`. **`Option` et non `String`** : une requête
absente et une requête vide ne sont pas le même fait, et c'est précisément la
distinction qui fait la valeur du témoin — `Some("")` dirait « il y a eu un `?`
sans paramètre », `None` dit « il n'y a pas eu de `?` ».

**Aucun second factice.** Les tests de ce vol empruntent `fake_spirit`, comme
V2d-3a l'a fait avant eux : un second témoin de socket ouvrirait une seconde
vérité sur ce que le serveur reçoit. La table de réponses étant clée par chemin,
`/api/v1/skills` s'y associe sans toucher au mécanisme.

**Aucun test existant ne change** (critère 3), et c'est mesuré au § 1 : `servir`
est le seul constructeur littéral du struct, et les autres sites n'en manipulent
que le type. `tui/commands/handlers.rs`, qui emprunte `RequeteRecue` et
`fake_spirit`, reste **intact** — ce qui est aussi ce que « tout
`crates/mika-cli/src/tui/` reste intact » exige.

### D5 — l'entrée d'egress part avec le client

Le lint étant bidirectionnel (§ 1), le commit du client sans son entrée est un
état rouge intermédiaire. `cli-spirit-skills`, sur le gabarit exact de
`cli-spirit-rewind` (`docs/egress/egress-manifest.toml` l. 363-371) :

```toml
[[sink]]
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

Une entrée, un site de construction de client — la propriété que l'en-tête de
`session_remote.rs` (l. 12-21) paye déjà d'une soixantaine de lignes dupliquées,
et que ce vol ne dépense pas à nouveau : **il ne met rien en commun.**

**`docs/egress/egress-manifest.toml` est le seul chemin de CODEOWNERS que ce vol
touche**, et pour cette entrée seule.

### D6 — les tests vivent en fin de fichier, et c'est structurel

Le scanner d'egress **coupe la production au premier `#[cfg(test)]`** : un item
de production placé après le module de tests en sortirait, et le lint ne le
verrait plus. Le commentaire de `rewind_remote.rs` (l. 515-516) le dit, et ce
fichier le **répète pour lui-même** plutôt que d'y renvoyer — un lecteur qui
déplace un item ne lit pas le fichier voisin.

### D7 — pourquoi ce plan ne porte pas de section `## Fire-Disposition`

La règle mika#2306 arme la section sur la livraison d'un **détecteur** : « tout
code dont le chemin de succès est *aucune violation trouvée* » — lint, garde CI,
scan structurel, garde EndTurn, validateur de schéma.

**Ce vol n'en livre aucun.** Ses cinq tests sont des tests de **comportement**
d'une porte client : leur chemin de succès est « le miroir relit ce champ » ou
« ce refus nomme cette cause », et ils échouent en nommant une valeur attendue.
Aucun n'a de population à parcourir, d'allowlist à tenir, ni de risque de passer
vert en ne lisant rien.

Ce vol ne porte **pas non plus** de garde `ProductionScanner`, contrairement à V1,
V2a, V2c-1b, V2d-4a/b et V3b-1/2 : il n'y a encore rien à refuser, le câblage
étant à V3c-2b. Une garde qui refuserait un geste que rien ne remplace encore ne
laisserait d'autre issue qu'une entrée d'allowlist — l'anti-motif que ce dépôt a
déjà nommé (V2d-4d, `maybe_compact(`).

Les critères 3, 4 et 5 se vérifient par le **diff** et par `clippy`, non par un
livrable de ce vol. La règle dit « si le plan ne livre AUCUN détecteur, la section
n'est pas requise : ne l'invente pas » — ce paragraphe est le raisonnement visible
qui tient la place d'une section vide. Même disposition que V3c-1 et que le plan
005 (leur D7/D8), pour la même raison.

### D8 — ce que ce vol ne fait pas

- **Les refus `AgentInconnu` (404) et `JetonRefuse` (401)**, et leurs tests :
  V3c-2a-iii. D'ici là, un 404 et un 401 sortent par `Statut`, et **aucun bras
  mort** ne les attend dans le `match`.
- **Tout câblage** : `app.skills`, l'autocomplétion, `/skills`, `/skill`, les
  messages de démarrage, `spawn_agent_worker`, `updated_skills`, le retrait de
  l'assemblage et de `migrate_disabled_markers_async` : V3c-2b. Aucun lecteur de
  `app.skills` ne change.
- **Le test É1** (le constructeur qui combine entrées et skills ignorées) :
  V3c-2c.
- **Aucun ticket ouvert** — gel jusqu'au 01/12. Un manque ou un reste va en
  commentaire sur mika#1727 ou dans la PR, et la suite se note
  `Tracked in: senara-solutions/mika#1727`.

---

## 3. Ordre de commit

Un commit par garde, **après que la garde est verte avec son test**, et non en fin
de vol. **Les tests ne se reportent jamais à un vol suivant**, même si le code
reste. C'est l'ordre suggéré par la tête du ticket, inchangé.

### C1 — le témoin de requête de `fake_spirit` (critère 3) — ≈ 6 lignes

`crates/mika-cli/src/commands/rewind_remote.rs`, **module `tests` seulement** : le
champ `requete: Option<String>` de `RequeteRecue` et son relevé par
`requete.uri().query()` dans `servir` (D4).

Ce commit ne porte pas de test propre : son livrable est un **témoin**, et le
premier test qui le lit est celui de C2. Il est séparé parce qu'il touche un
autre fichier que le reste du vol, et que le critère 3 porte sur lui seul.

Vérification du commit : `cargo test -p mika-cli` — aucun test existant ne doit
bouger, ce qui **est** l'assertion du critère 3.

### C2 — le client, son miroir, ses trois refus et son egress (critères 1, 2, 3, 4, 5) — ≈ 363 lignes

- `crates/mika-cli/src/commands/skills_remote.rs`, **fichier neuf** : en-tête de
  module (le raisonnement d'egress, l'état du vol, et ce que V3c-2a-iii et
  V3c-2b en feront), les cinq miroirs, `SkillsRemoteIssue` + `Display`, `lire`,
  les attentes (D1, D2, D3), puis **en fin de fichier** le module
  `#[cfg(test)] mod tests` (D6).
- `crates/mika-cli/src/commands/mod.rs` : la déclaration, entre `skills` et
  `skills_variants`. Une ligne.
- `docs/egress/egress-manifest.toml` : l'entrée `cli-spirit-skills` (D5), **dans
  ce commit** parce que le lint lit dans les deux sens.

Les cinq miroirs : `SkillsMirror` (`agent`, `skills`, `skipped`, `warnings`),
`SkillMirror` (les treize champs), `VariantsMirror` (`total`, `providers` en
`BTreeMap<String, usize>`), `SkippedMirror` (`name`, `reason`),
`WarningMirror` (`name`, `messages`). Nommés `…Mirror` comme ceux des deux
voisins, et **désérialisés tels que la route les nomme** — `tools` reste le
compte, `tool_names` les noms.

Les cinq tests, avec un corps JSON en `const` et un helper de lecture **partagés**
dans le fichier, pas recopiés :

1. le miroir complet (critère 1) — deux skills, une ignorée, un avertissement ;
   les treize champs, `agent`, `skipped` et `warnings` relus, **et** dans le
   **même** test la méthode `GET`, le chemin `/api/v1/skills`, la requête
   `agent=<nom>` et `Authorization: Bearer <jeton>` attestés par le témoin ;
2. l'encodage d'un nom d'agent portant un caractère réservé (critère 1) ;
3. `Injoignable` par `DeadEndpoint::reserve()`, **guard tenu pendant tout
   l'appel** (mika#2569 — un port libéré se fait servir par un factice voisin du
   même binaire, mesuré le 2026-09-28) : l'URL dans le message, **aucun statut** ;
4. `Statut` sur un 500 : le statut, l'URL et le corps ;
5. `CorpsIllisible` sur un 200 au corps illisible : l'URL et la cause, et un
   `assert!` négatif — **jamais un miroir**.

Vérification du commit, **dans cet ordre** : `cargo build -p mika-cli --bin mika`
**d'abord** (c'est la cible où `dead_code` mord, donc celle qui valide les
attentes), puis `cargo test -p mika-cli`, puis
`cargo clippy --all-targets -p mika-cli`, puis
`bash scripts/verify-egress-manifest.sh --report`.

### C3 — la doc du client (critère 7) — ≈ 4 lignes

Un paragraphe de `crates/mika-cli/CLAUDE.md` : le client existe, il **n'est pas
encore câblé**, quels refus il distingue, et ce que V3c-2a-iii (les deux refus
métier) et V3c-2b (le câblage, et le retrait des attentes) en feront.
`cargo test -p mika-common --test claude_md_size` reste vert — marge de 14 038
octets.

### Fin de pipeline

Dans l'ordre, avant d'ouvrir la PR :

1. `cargo build -p mika-cli --bin mika`
2. `cargo test -p mika-cli`
3. `cargo test -p mika-common --test claude_md_size`
4. `cargo clippy --all-targets -p mika-cli`
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
- **> 430 après C2 ⇒ arrêt aussi**, dit de la même façon : C3 ne tiendrait pas, et
  un vol qui s'arrête avant sa doc est un vol à reprendre.

Les commentaires comptent dans la borne **et ne se rabotent pas pour la tenir**.
Un commentaire dit le pourquoi qu'on ne lit pas dans le code, pas une paraphrase.
Si le code écrit dépasse, le pilote s'arrête selon la halte ci-dessus ; il ne
retire **pas** après coup les commentaires qui portent un pourquoi.

#### Amendement du pilote — le sous-seuil de 430 est franchi de 3 lignes (2026-11-17)

**Le point.** Mesuré après C2 : **433 lignes** ajoutées, plan exclu —
`rewind_remote.rs` 6 (C1), `skills_remote.rs` 415, `commands/mod.rs` 1,
`egress-manifest.toml` 11 (C2). Le sous-seuil de 430 de la puce ci-dessus est
donc franchi, la borne de la tête du ticket — **475** — ne l'est pas.

**La raison de ne pas s'arrêter là.** Ce sous-seuil n'est pas une borne de
l'opérateur : la tête du ticket n'en porte qu'une, 475. Il est un garde-fou que
ce plan s'est donné, et sa raison est écrite : « C3 ne tiendrait pas ». Elle ne
vaut pas ici — C3 fait **4 lignes** de `crates/mika-cli/CLAUDE.md`, donc
**437 pour 475**, et c'est l'inverse de ce que le garde-fou protégeait : s'arrêter
maintenant livrerait précisément « un vol qui s'arrête avant sa doc ».

**D'où viennent les 79 lignes au-delà des 354 estimées.** Les six attentes
`dead_code` coûtent **24 lignes** et non six : `rustfmt` casse
`#[cfg_attr(not(test), expect(dead_code, reason = "…"))]` sur quatre lignes
chacune, ce que l'estimation comptait pour une. Le reste est dans les
commentaires de `pourquoi` des miroirs et des tests, que la règle ci-dessus
interdit de raboter après coup — et qui ne sont donc pas rabotés.

**Ce qui ne change pas.** La borne de 475 reste ferme, et la halte qui s'y
attache aussi : si C3 la franchissait, le pilote s'arrêterait avec le message
qu'elle prescrit. Le périmètre de fichiers, les sept critères et la Definition
of Done sont inchangés.

### Si le code s'écarte de ce plan

La QA juge contre le plan de la branche, pas contre le corps du ticket. Si le
pilote doit s'écarter d'un point — un fichier, un critère, un nom, une forme
d'attente — **il amende le plan sur la branche dans un commit `docs(plans)` avant
de pousser le code qui s'en écarte**, en nommant le point et la raison. Un plan
qui dit une chose pendant que le code en fait une autre est bloqué par la QA,
même quand le code est juste.

---

## 4. Definition of Done

- `crates/mika-cli/src/commands/skills_remote.rs` existe et porte : le miroir
  complet (les treize champs par skill, `agent`, `skipped`, `warnings`), la porte
  `lire(base_url, token, agent)` en GET sur `/api/v1/skills` avec le nom d'agent
  en paramètre de requête **encodé par la bibliothèque**, et
  `SkillsRemoteIssue` à **trois** variants distincts.
- Le module n'a **aucun** appelant de production, et chaque item sans appelant
  porte **une seule** attente,
  `#[cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))]`,
  et rien d'autre. Aucun `allow` neuf dans `crates/mika-cli/`.
- Les tests du fichier vivent **en fin de fichier**, après le premier
  `#[cfg(test)]`, et partagent leur corps JSON et leur helper de lecture.
- `fake_spirit` relève la chaîne de requête dans `RequeteRecue.requete:
  Option<String>`, et **aucun test existant** de `rewind_remote.rs` ni de
  `session_remote.rs` ne change.
- L'entrée `cli-spirit-skills` est au manifeste d'egress, dans le même commit que
  le client, et `verify-egress-manifest.sh --report` est vert.
- Un paragraphe de `crates/mika-cli/CLAUDE.md` dit que le client existe, qu'il
  n'est pas câblé, quels refus il distingue, et ce que V3c-2a-iii et V3c-2b en
  feront.
- **Restent inchangés** : tout `crates/mika-agent/`,
  `crates/mika-cli/src/commands/chat.rs`, tout `crates/mika-cli/src/tui/`,
  `session_remote.rs`, `crates/mika-common/`, `crates/mika-a2a/`,
  `crates/mika-gateway/` et `dashboard/`. Aucun fichier du plan de contrôle n'est
  touché, `docs/egress/egress-manifest.toml` excepté.
- `cargo build -p mika-cli --bin mika`, `cargo test -p mika-cli`,
  `cargo test -p mika-common --test claude_md_size` et
  `cargo clippy --all-targets -p mika-cli` sont verts, et
  `scripts/verify-pipeline.sh origin/main` aussi.
- Le diff reste **sous 475 lignes ajoutées**, plan exclu.
- La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`.

---

## 5. Acceptance criteria

Transcrits de la tête du ticket (§ *Critères de V3c-2a-ii*).

1. `skills_remote::lire`, contre `fake_spirit` servant une réponse de la forme de
   la route (deux skills, une ignorée, un avertissement), rend un miroir où
   chacun des treize champs, `agent`, `skipped` et `warnings` se relit. Le
   factice atteste dans le **même** test la méthode `GET`, le chemin
   `/api/v1/skills`, la requête `agent=<nom>` et `Authorization: Bearer <jeton>`.
   Un nom d'agent qui porte un caractère réservé arrive encodé.
2. Chacun des trois refus est atteint par un test et se distingue des autres :
   injoignable (par `DeadEndpoint::reserve()`, jamais un port libéré : l'URL dans
   le message, aucun statut), autre statut (500 : le statut, l'URL et le corps),
   200 au corps illisible (l'URL et la cause, et **jamais** un miroir).
3. Aucun test existant de `rewind_remote.rs` ni de `session_remote.rs` ne change,
   hors l'ajout du champ de requête à `RequeteRecue`.
4. Seuls changent les fichiers listés par la tête du ticket. Tout
   `crates/mika-agent/`, `chat.rs` et tout `crates/mika-cli/src/tui/` sont
   inchangés.
5. Les seules attentes neuves de `crates/mika-cli/` sont celles de
   `skills_remote.rs`, toutes de la forme
   `cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))`.
   Aucun `allow` neuf.
6. Le diff reste sous 475 lignes ajoutées, plan exclu.
7. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
   `cargo test -p mika-common --test claude_md_size` sont verts.
   `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il
   trouve l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `rewind_remote.rs` tests — le champ `requete` et son relevé (C1) | 6 |
| `commands/mod.rs` — la déclaration du module (C2) | 1 |
| `skills_remote.rs` — en-tête de module (C2) | 18 |
| `skills_remote.rs` — les cinq miroirs, 23 champs, attentes comprises (C2) | 55 |
| `skills_remote.rs` — `SkillsRemoteIssue` à trois variants + `Display` (C2) | 39 |
| `skills_remote.rs` — `lire` (C2) | 57 |
| `skills_remote.rs` tests — préambule, const du corps JSON, helper partagé (C2) | 46 |
| test du critère 1 — les 23 champs relus + méthode/chemin/requête/jeton (C2) | 48 |
| test de l'encodage d'un nom réservé (C2) | 18 |
| tests des trois refus — 3 × 22 (C2) | 66 |

Total estimé : 354 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

### Pourquoi ce vol tient, en un seul vol

**Les deux unités, et les deux marges.** La ligne `Total estimé :` est dans
l'unité de l'architecte : elle exclut les 11 lignes d'egress (`docs/`) et les 4
de `crates/mika-cli/CLAUDE.md`. L'unité de la borne du vol
(`git diff --numstat origin/main...HEAD`, plan exclu, doc comprise) les inclut :
**369**.

| unité | estimé | borne | marge |
|---|---|---|---|
| architecte (hors `docs/`, hors `CLAUDE.md`) | **354** | 429 (mika#2693) / 452 (tête du ticket) | +75 / +98 |
| borne du vol (`--numstat`, plan exclu, doc comprise) | **369** | 475 | **+106** |

**Les deux bornes sont tenues, donc un seul vol** — et la coupe B de Vincent est
ce qui le rend vrai : le plan 005, qui portait le serveur, le miroir **et** cinq
refus, pesait 517 / 534 et s'est arrêté dessus. Ce vol-ci est ce même document
moins le serveur (parti en V3c-2a-i) et moins deux refus (partis en V3c-2a-iii).

**L'estimation est reprise du plan 005, poste par poste, sans nouvelle mesure**
(la tête du ticket l'exige), avec trois ajustements tous **vers le bas**, chacun
mesuré et non deviné :

- **`commands/mod.rs` : 2 → 1.** `git show cad12440 --numstat` donne **1** ligne
  pour le même geste. Le plan 005 avait arrondi.
- **`SkillsRemoteIssue` : 56 → 39.** `RewindRemoteIssue` et son `Display` pèsent
  **52 lignes pour cinq variants** (l. 100-151), soit un coût fixe d'une douzaine
  de lignes (dérive, en-tête, `impl Display`) plus ~8 par variant commenté. Trois
  variants : 12 + 24 ≈ 39.
- **les tests des refus : 110 → 66.** Trois refus au lieu de cinq, au même débit
  de 22 par test — et ce 22 est déjà l'hypothèse la plus optimiste du tableau
  (`session_remote::tests` pèse 350 lignes pour six tests et ses helpers, soit
  ~45 par test ; 22 suppose un helper et un const partagés, ce que C2 livre).

**L'ancrage de bout en bout.** Le vol V2d-3a entier (`0b44a562`) a coûté
**369 lignes** dans l'unité de la borne — le nombre exact estimé ici. La
comparaison poste par poste est serrée dans les deux sens, ce qui est ce qui rend
l'égalité significative plutôt que fortuite :

| axe | V2d-3a (mesuré, 369) | ce vol (estimé, 369) |
|---|---|---|
| champs de miroir | 3, sur 3 structs | **23, sur 5 structs** |
| variants de refus | 5 | **3** |
| tests | 6 | **5** |
| côté serveur | aucun | aucun |
| `CLAUDE.md` | aucun | 1 paragraphe (4 l.) |
| témoin à ajouter au factice | aucun | 1 champ (6 l.) |

Ce vol livre **plus** de miroir et **moins** de refus que V2d-3a, pour un coût
total égal : les 20 champs de miroir supplémentaires (+~44 lignes de struct et de
relecture) sont payés par les deux refus et le test de moins (−~61). Le solde
tient dans l'épaisseur de l'en-tête et du témoin.

**La marge de 106 lignes est ce qui absorbe une erreur d'estimation**, et elle est
du bon côté : si chaque poste du tableau dérapait de 20 %, le total atteindrait
443 dans l'unité de la borne — encore sous 475. C'est pourquoi ce plan ne demande
pas de coupe, là où le plan 005 en demandait une.

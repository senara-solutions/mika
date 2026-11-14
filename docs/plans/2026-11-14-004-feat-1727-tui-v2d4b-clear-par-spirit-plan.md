# Vol V2d-4b — `/clear` passe par mika-spirit, et le jeton se lit à un seul endroit

**Ticket :** senara-solutions/mika#1727 (vol V2d-4b)
**Branche :** `feat/1727/tui-v2d4b-clear-par-spirit`
**Base mesurée :** `748ad33d` (merge de V2d-4a, #3088)

`/clear` cesse de toucher la base locale : il **ouvre** une session par
`session_remote::ouvrir`, puis **clôt** l'ancienne par `session_remote::clore`,
dans cet ordre. Et les deux copies de la lecture stricte du jeton — l'une dans
`tui/commands/handlers.rs`, l'autre dans `commands/chat.rs`, annoncées à la mise
en commun par V2d-4a — n'en font plus qu'une.

---

## 0. Ce qui est mesuré

| fait | site | ce qu'il décide ici |
|---|---|---|
| `/clear` clôt puis crée **en base locale**, avec un `tracing::warn!` sur chaque échec | `tui/commands/handlers.rs:139-150` | le bloc que ce vol remplace par les deux portes ; les deux pannes sont aujourd'hui **muettes** à l'écran |
| la nouvelle session est un **UUID tiré sur place** | `handlers.rs:145` (`uuid::Uuid::new_v4()`) | elle viendra du fil : `ouvrir` ne prend aucun identifiant dans son corps, c'est le serveur qui le rend (`CreatedSessionMirror::session_id`) |
| l'insert local passe `app.db.agent_id()` | `handlers.rs:146-147` | les portes prennent `app.agent_name`, comme les trois portes rewind (`handlers.rs:1298`, `:1313`, `:1336`) — D9 |
| le bras singleton ne touche ni la base ni le réseau, et rend son propre message | `handlers.rs:117-136` | il reste tel quel, et il reste **le premier** test de la fonction (D4) |
| deux copies de la lecture stricte du jeton, à la même forme et à deux langues | `handlers.rs:1217` + `:1231` (anglais, nomme le rewind) et `chat.rs:57` + `:70` (français, nomme l'ouverture) | les deux types de retour **diffèrent** : `Result<String, String>` d'un côté, `anyhow::Result<String>` de l'autre. C'est ce qui décide D1 |
| les portes `ouvrir` et `clore` sont sur main, testées | `session_remote.rs:236` et `:265` | ce vol pose un appelant ; il n'introduit aucune route, aucun statut et aucun miroir neuf |
| quatre `expect(dead_code)` restent | `session_remote.rs:51` (`EndedSessionMirror`), `:63` (`CompactedMirror`), `:259` (`clore`), `:297` (`compacter`) | deux tombent ici (D6, D10), deux restent pour V2d-4c |
| le modèle de câblage charge le `.env` de la home, lit `dashboard::spirit_url()`, puis délègue | `handlers.rs:1239-1263` (`handle_rewind_impl` → `rewind_par_spirit`) | la forme exacte que `handle_clear` prend (D3) |
| le banc sert plusieurs chemins dans un même test, par table, et rend **501** sur un chemin non associé | `rewind_remote.rs:548-592` (`servir`, `fake_spirit`, `RequeteRecue`) | les tests de ce vol l'empruntent sans rien y ajouter : `rewind_remote.rs` reste intact (critère 10) |
| `fake_spirit` associe par le chemin **brut**, et la convention est une const locale au module de tests | `chat.rs:1343-1345` | le chemin de `clore` porte un identifiant concret : le test lit `app.session_id` **avant** l'appel et compose `/api/v1/sessions/{ancienne}/end` |
| `test_app()` pose un `session_id` **aléatoire** | `handlers.rs:1465-1470` | d'où la composition ci-dessus : aucun chemin de clôture ne se connaît à l'avance |
| onze tests appellent `handle_clear`, dont **deux** sur un singleton | `handlers.rs:1573`, `:1597`, `:1616`, `:1633`★, `:1669`★, `:1803`, `:1861`, `:1871`, `:1885`, `:1905`, `:1984` (★ = singleton) | les neuf du critère 7, plus les deux du critère 6 |
| `test_clear_ends_old_session` lit `ended_at` **en base locale** | `handlers.rs:1616-1631` | c'est le seul des onze dont l'assertion change de nature : elle part sur la socket |
| aucun test existant n'asserte sur un **mot** du refus du jeton — seulement sur les noms de variables | `handlers.rs:2434`, `:2450`, `:2529` ; `chat.rs:1372`, `:1389-1390`, `:1508` | le choix de langue de D2 ne casse aucun test, et c'est mesuré avant de choisir |

**La mesure qui corrige une phrase du plan précédent.** Le plan de V2d-4a écrivait
que `production_of` « coupe au premier `#[cfg(test)]` ». C'est faux, et il faut le
dire avant de poser une garde sur ce fichier-ci : `mika_common::source_guard`
**masque** chaque région (ses lignes deviennent vides) et ne tronque jamais — la
troncature « assumes the test code sits at the end of the file, which is false
from form (1) onwards », et c'est la cécité que mika#2398 existe pour retirer.
Trois conséquences, toutes portantes :

- le `#[cfg(test)] fn resolve_model_name` de `handlers.rs:479-487` n'ampute
  **rien** : la moitié production couvre l. 1-478 **et** l. 488-1407 ;
- `handle_clear` (l. 112) et `jeton_interne` (l. 1231) sont donc **tous deux** en
  production, et la garde de ce vol les voit ;
- une région que le scanner **ne sait pas fermer** est laissée **visible**
  exprès (`SliceReport::unclosed`). Sur ce fichier-là, ce cas a un nom :
  `async fn test_clear_singleton_does_not_end_session()` (l. 1669) contient
  littéralement le motif `end_session(`. D'où le **contre-témoin** de D8.

**Les faits qui sont des absences** — ils décident autant que les présences :

- **Personne ne lit `clore` ni les champs d'`EndedSessionMirror` hors test.**
  C'est ce que disent les `expect(dead_code)` de `session_remote.rs:259` et `:51`.
  Ce vol leur donne leur premier lecteur de production : les deux **doivent**
  tomber dans le commit qui câble, sous peine d'attente insatisfaite (D10).
- **`create_session(` et `end_session(` n'ont qu'une occurrence de production
  chacun** dans `handlers.rs` : l. 147 et l. 140. Les autres (l. 1468, l. 1669)
  vivent dans le `mod tests` (l. 1408-2711), masqué. C'est ce qui permet de
  livrer la garde **allowlist vide**, puisque le même commit retire les deux.
- **Aucune entrée d'egress ne change.** `handlers.rs` appelle les portes ; il ne
  construit aucun client vers les routes de session. L'entrée
  `cli-spirit-sessions` désigne toujours `session_remote.rs`. Si le pilote juge
  le contraire, il s'arrête et le dit (§ 5).
- **Aucun fichier neuf, aucune dépendance neuve.** `commands/mod.rs` porte déjà
  `pub mod session_remote;`, `mika-common` + `test-utils` est déjà en
  `[dev-dependencies]`, et `ROUTE_SESSIONS` reste privée : la convention du
  voisin est une const locale au module de tests (§ 0), donc rien n'a besoin de
  changer de visibilité pour être testable.

---

## 1. La forme, et les dix décisions qui la tiennent

### D1 — Une seule `jeton_interne`, dans `session_remote.rs`, qui rend `Result<String, String>`

```rust
/// La source d'environnement arrive en paramètre : un test de refus qui
/// muterait l'environnement du process serait `unsafe` en édition 2024, et faux
/// sous `cargo test`, qui parallélise.
pub(crate) type SourceEnv<'a> = &'a dyn Fn(&str) -> Option<String>;

pub(crate) fn jeton_interne(lire: SourceEnv<'_>) -> Result<String, String> { … }
```

**Pourquoi ce fichier.** Le ticket le fixe, et le périmètre l'oblige : les quatre
fichiers qui changent sont nommés, aucun n'est neuf, et un module `jeton.rs` à
part — qui serait le site le plus neutre — en ajouterait un. Reste la tension à
nommer plutôt qu'à taire : **le rewind n'est pas une route de session**, et
`rewind_par_spirit` importera désormais sa lecture de `session_remote`. Elle se
dissout sur un fait : c'est le **même** jeton, parce que les deux familles de
routes sont montées sous `require_internal_token`. La lecture est donc une
propriété du jeton, pas du geste — et c'est exactement pourquoi le ticket exige
que le refus ne nomme **aucun** geste.

**Pourquoi `Result<String, String>` et pas `SessionRemoteIssue`.** Ce dernier
décrit des **réponses HTTP** : chacun de ses bras porte un statut, une URL ou un
corps. Une variable d'environnement absente n'en a aucun — l'y fondre
obligerait à inventer un statut pour un refus qui tombe *avant* toute requête, et
`Unexpected { status: None, … }` se lirait comme « mika-spirit est injoignable »,
une provenance fausse (motif mika#2331). Et pas de type nommé non plus : rien ne
se **matche** sur ce refus, son message **est** l'information, et deux des trois
appelants rendent déjà `String` à l'écran.

Le troisième, `chat.rs`, l'adapte en une ligne — `.map_err(anyhow::Error::msg)?` —
et c'est le bon sens de la conversion : un `String` monte vers `anyhow` sans
perte, là où dégrader un `anyhow::Error` vers `String` perdrait une chaîne de
causes que ce refus n'a de toute façon pas.

### D2 — Un seul texte, une seule langue, et c'est l'anglais

```rust
/// Pas de repli sur `MIKA_DASHBOARD_TOKEN`, contrairement à
/// `commands::dashboard::auth_token()` : les routes rewind comme les routes de
/// session sont montées sous `require_internal_token`, et ce jeton-là y prend un
/// 401 — donc un refus *après* l'appel, qui nommerait le transport là où la cause
/// est la configuration du shell.
///
/// Le refus ne nomme **aucun geste**. Trois sites l'appellent — le démarrage de
/// la TUI, `/clear` et le rewind —, et un texte qui en nommerait un mentirait aux
/// deux autres : c'est la dérive qui a produit les deux copies que ce vol réunit.
```

**La langue est l'anglais, et c'est un choix mesuré, pas un défaut.** Tous les
messages que ce crate rend à l'utilisateur le sont (`WORKER_NOT_RESPONDING`,
« Chat cleared and session reset. », le bras singleton de `/clear`), et le
`Display` de `SessionRemoteIssue` aussi. Or le critère 5 **compose** le refus
avec ce `Display` dans un même ruban : un refus français collé à une cause
anglaise serait la phrase bilingue, dans la seule partie du système qu'un
opérateur lit sous pression. Les commentaires restent en français, comme tout le
code de la série — ils s'adressent à qui maintient, pas à qui subit.

Le texte perd donc son « rewind » (`handlers.rs:1233`) et gagne la seconde phrase
de la version `chat.rs` : que `MIKA_DASHBOARD_TOKEN` **ne suffit pas**. C'est la
mesure de `dashboard.rs:37-38` écrite à l'endroit où un opérateur la lira, et
désormais au seul endroit du code où elle se dit.

**Pas de règle de non-vacuité**, reconduit de V2c-1b et de V2d-4a : une variable
posée à vide est une *configuration fausse*, pas une absence. Elle part sur le
fil et revient en 401, dont le `Display` nomme déjà la variable. En inventer une
ici, au moment précis où deux lectures se réunissent, serait recréer l'écart
qu'on supprime.

### D3 — `clear_par_spirit`, qui prend la base et la source par argument

```rust
/// `/clear` tel que mika-spirit l'applique : l'ouverture et la clôture passent
/// par les deux portes de [`crate::commands::session_remote`].
///
/// L'URL de base et la source d'environnement arrivent **par argument**, comme
/// `rewind_par_spirit` juste au-dessus : c'est ce qui permet aux tests de viser
/// un banc et de lire ce qui est parti sur la socket, plutôt que de relire le
/// code qui l'écrit.
async fn clear_par_spirit(app: &mut App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String
```

Et `handle_clear` se réduit à ce que fait `handle_rewind_impl` : charger le `.env`
de la home, lire `dashboard::spirit_url()`, déléguer. Le chargement reste **au
site d'appel** et non derrière `lire_env`, pour la raison déjà écrite sur
`handle_rewind_impl:1248-1250` — un chargement caché derrière ce paramètre ferait
mentir sa signature.

**Les onze tests existants visent `clear_par_spirit`, pas `handle_clear` — les
deux singleton compris.** Le critère 7 l'exige pour les neuf ; les deux singleton
suivent, et ce n'est pas une extension mais la seule lecture sûre du critère 6,
qui demande que « le banc ne reçoive aucune requête, même sans jeton » : il faut
donc un banc, donc la fonction qui le prend. Le bénéfice réel est ailleurs :
aucun test ne charge le `.env` du process. `load_dotenv` mute `std::env`, ce que
`cargo test` parallélise — un test qui le ferait emporterait ses voisins, et la
panne se lirait partout sauf chez lui.

### D4 — Quatre temps, et l'ordre est le livrable

```
1. singleton         → le bras actuel, inchangé. Ni lecture du jeton, ni appel.
2. jeton_interne     → refus AVANT toute construction de client.
3. ouvrir(agent)     → sur Err : refus, rien n'est clos, rien ne change.
4. clore(ancienne)   → sur Err : la nouvelle session est GARDÉE.
5. NewSession + vidange + remise à zéro, comme aujourd'hui.
```

**Pourquoi `ouvrir` avant `clore`, et pas l'inverse.** L'ordre d'aujourd'hui est
clore-puis-créer (`handlers.rs:140` puis `:147`), et il est sans danger parce que
les deux gestes sont locaux et que le second ne peut guère échouer. Sur le fil, la
même séquence perd l'historique : un `clore` réussi suivi d'un `ouvrir` qui échoue
laisse la TUI **sans session** et son ancienne conversation **close** — une perte,
là où l'utilisateur n'a demandé qu'un écran vide. L'ordre inverse rend tout échec
réversible : au temps 3 rien n'a bougé, au temps 4 la session neuve existe déjà.

**Et il met la protection mika#1401 là où elle a l'autorité.** `clore` est
appelée **inconditionnellement** sur l'ancienne session, sans que le client
vérifie d'abord qu'elle n'est pas la canonique : le serveur refuse lui-même de la
clore et rend `{ended: false, canonical: true}` — un **200**, pas une erreur
(`server/sessions.rs:162-164`, dit par la doc de `clore`). Rejouer la règle côté
client ajouterait une seconde autorité sur un fait qui n'en a qu'une, et la ferait
dériver au premier changement de contrat.

Le temps 2 est **avant** le temps 3, et c'est le critère 3 : le refus tombe avant
toute construction de client, et c'est la socket qui l'atteste — un message exact
ne dirait pas qu'aucun `POST /api/v1/sessions` n'est parti.

### D5 — `is_singleton_session` prend le `canonical` du fil, et la TUI peut en changer d'avis

`app.is_singleton_session = miroir.canonical`, où `miroir` est ce qu'`ouvrir` a
rendu. C'est la décision de V2d-4a reconduite au deuxième site : la route rend la
session canonique de l'agent au lieu d'en créer une, donc le serveur est la seule
autorité sur ce booléen.

La conséquence vaut d'être écrite, parce qu'elle surprend : **une TUI peut entrer
dans `/clear` non-singleton et en sortir singleton**. Ce n'est pas un effet de
bord, c'est la correction d'un désaccord — la TUI avait reçu son booléen au
démarrage, et l'agent est devenu singleton depuis. Le chemin reste sûr dans les
trois cas :

- ancienne ≠ canonique, nouvelle quelconque → `clore` clôt l'ancienne,
  normalement ;
- ancienne = canonique → le serveur refuse de la clore, `ended: false`, et c'est
  D6 qui le dit à l'écran ;
- `ouvrir` rend l'identifiant **déjà courant** (la canonique) → `clore` le vise,
  le serveur refuse, la session survit. Un client qui aurait clos d'abord
  l'aurait détruite.

### D6 — Le miroir de clôture est **lu**, et le message rendu ne dépasse pas ce que le serveur a attesté

`clore` rend `{ended, canonical}`. Les deux champs sont lus, et le message final
dit lequel des trois états a eu lieu :

| ce que le serveur a répondu | ce que `/clear` rend |
|---|---|
| `ended: true` | le message d'aujourd'hui, inchangé |
| `ended: false, canonical: true` | la nouvelle session est là, **et** l'ancienne est la canonique de l'agent : elle n'a pas été close |
| `ended: false, canonical: false` | la nouvelle session est là, **et** le serveur n'a pas clos l'ancienne, sans dire pourquoi |

**Pourquoi ce n'est pas un élargissement.** « Chat cleared and session reset. »
est faux dans sa seconde moitié dès que `ended` vaut `false`, et ce dépôt a une
règle pour ça : ne jamais affirmer un état en aval sans qu'un résultat d'outil le
confirme (CLAUDE.md § Conventions, *Grounding rule*). Le serveur vient de
répondre ; l'ignorer pour rendre une phrase plus courte est précisément la faute
que la règle nomme. Le troisième bras n'est pas produit par le serveur
d'aujourd'hui, et il est là quand même : un client qui présume l'exhaustivité d'un
contrat qu'il ne possède pas écrit son propre faux.

**La conséquence est mécanique, et elle est la raison pour laquelle les deux
champs sont lus et non un seul.** `expect(dead_code)` est posé sur l'**item**
`EndedSessionMirror`, donc il couvre ses champs : lire `ended` seul laisserait
`canonical` mort, l'attente resterait **satisfaite**, l'attribut devrait rester,
et le compte annoncé serait trois. En lire deux le fait tomber, et le compte est
**deux**. Un plan qui dirait « lis `ended` » et « retire l'attribut » dans la même
phrase serait rouge sous clippy.

### D7 — Un échec de `clore` ne défait rien

La nouvelle session est **gardée**, le worker reçoit son `NewSession`, l'écran se
vide, et le message rendu dit que l'ancienne n'a pas été close **en nommant la
cause** — le `Display` de `SessionRemoteIssue`, qui porte déjà le statut, l'URL ou
la variable à poser.

C'est le critère 5, et c'est la seule politique cohérente avec D4 : revenir en
arrière voudrait dire clore la session qu'on vient d'ouvrir, c'est-à-dire répondre
à l'échec d'une porte par un second appel à l'autre, sur un serveur dont on vient
de constater qu'il ne répond pas. Une session orpheline côté serveur est un reste
mesurable ; une TUI qui pointe une session close est une panne pour l'utilisateur.

### D8 — La garde structurelle : deux motifs, allowlist vide, **un témoin et un contre-témoin**

```rust
#[test]
fn mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local() {
    // Livrée vide, et vide par construction : quand la garde rougit, le remède
    // est la porte que son message nomme, jamais une entrée ici.
    const ALLOWED: &[&str] = &[];
    assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
    const MOTIFS: &[(&str, &str)] = &[
        ("create_session(", "session_remote::ouvrir"),
        ("end_session(", "session_remote::clore"),
    ];

    let scanner = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let site = scanner.src_root().join("tui/commands/handlers.rs");
    let production = scanner.production_of(&site);

    // Témoin : la garde a bien lu la moitié production.
    assert!(production.contains("async fn handle_clear("), …);
    // Contre-témoin : elle a bien masqué la moitié test. Voir ci-dessous —
    // un nom de test de ce fichier contient littéralement un des motifs.
    assert!(!production.contains("fn test_clear_singleton_does_not_end_session"), …);

    // … une boucle par motif : lignes non commentées qui le contiennent,
    //   assert vide, chaque violation nommant la porte qui la remplace.
}
```

Modèle : `mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local`
(`handlers.rs:2667-2710`), copié dans sa forme et non dans son texte.

**Le contre-témoin est la pièce neuve, et il n'est pas décoratif.**
`async fn test_clear_singleton_does_not_end_session()` (l. 1669) contient le
motif `end_session(` dans son **nom**. Il vit dans la moitié test, donc le
scanner le masque, donc la garde est verte — aujourd'hui. Le jour où une région
devient infermable (une accolade de fermeture désindentée, un littéral brut
multi-lignes), `source_guard` la laisse **visible** exprès, et cette garde
rougirait en nommant un nom de test. Le remède tentant serait alors une entrée
d'allowlist — exactement le faux vert que la garde existe pour empêcher. Le
contre-témoin transforme ce cas en un message qui nomme le **masquage**, donc le
vrai défaut, au lieu de laisser accuser le motif.

Les trois autres points que la forme impose :

- **L'allowlist est livrée vide et assertie vide.** Elle le peut parce que § 0 le
  mesure : les deux seules violations de production sont l. 140 et l. 147, et le
  même commit les retire. Quand la garde rougit, le remède est de passer par la
  porte, **jamais** d'ajouter une entrée.
- **`maybe_compact(` n'est pas un motif** (`handlers.rs:205`) : `/compact`
  appartient à V2d-4c, et le mettre ici rendrait la garde rouge le jour de sa
  pose.
- **Deux gardes sur le même fichier, et pas une fusion.** Celle de V2c-1b vise
  trois motifs de rembobinage avec `handle_rewind_impl` pour témoin ; celle-ci
  vise deux motifs de session avec `handle_clear`. Les fondre ferait un message
  d'échec qui nomme cinq remèdes dont quatre sont hors sujet — une garde dont le
  rouge demande de lire le plan au lieu de dire le geste.

### D9 — Les portes reçoivent `app.agent_name`, et non `app.db.agent_id()`

L'insert local passait `agent_id`, l'identité de la **base**. Les routes
résolvent l'agent par son **nom** (`#[serde(default)]` puis 404 sur agent
inconnu), et c'est ce que les trois portes rewind envoient déjà depuis le même
fichier (`handlers.rs:1298`, `:1313`, `:1336`). Les deux valeurs coïncident dans
ce dépôt ; l'argument n'est pas qu'elles diffèrent, c'est qu'on envoie ce que la
route attend, attesté par un voisin vert, plutôt qu'une valeur qui se trouve
marcher.

### D10 — Le câblage et le retrait des `expect` voyagent dans le même commit

Dès que `clear_par_spirit` appelle `clore` et lit les deux champs du miroir, les
deux attributs deviennent `unfulfilled_lint_expectation` : c'est tout l'intérêt
d'`expect` plutôt qu'`allow` (Fire-Disposition de V2d-3a, reconduite). Les séparer
laisserait un commit intermédiaire **rouge sous clippy**, ce que l'ordre de
travail du ticket — « le pilote commite après chaque garde verte » — interdit.
D'où le commit 2 du § 3, qui porte les deux fichiers.

La doc du module (l. 18-27) dit aujourd'hui « **quatre** — deux portes, deux
miroirs ». Elle dira **deux** (`compacter` et `CompactedMirror`), et redira que ce
chiffre se **mesure** sous `cargo clippy -p mika-cli --all-targets` et ne se
devine pas : retirer deux attributs change le graphe d'atteignabilité que `rustc`
parcourt, donc un compte obtenu par soustraction n'est pas une mesure. **Si la
passe verte en demande un nombre différent de deux, c'est la passe qui a raison et
la doc qui s'aligne**, et le pilote écrit le compte observé.

---

## 2. Les livrables, un par un

### 2.1 `session_remote.rs` — le jeton commun (D1, D2)

`pub(crate) type SourceEnv<'a>` et `pub(crate) fn jeton_interne`, posés au-dessus
du vocabulaire d'erreur. La doc porte les deux raisons de § D1-D2 : pourquoi pas
de repli sur `MIKA_DASHBOARD_TOKEN`, et pourquoi le refus ne nomme aucun geste.

### 2.2 `handlers.rs` — les copies retirées, et T1/T2 rebranchés

`type SourceEnv` (l. 1217) et `fn jeton_interne` (l. 1231) disparaissent ; les
deux lecteurs du fichier (`rewind_par_spirit` et le nouveau `clear_par_spirit`)
importent de `session_remote`. **T1 et T2 de V2c-1b restent où ils sont**
(`handlers.rs:2431`, `:2441`) et appellent la fonction commune : le ticket dit
« rebranchés », pas « déplacés », et un test du refus de jeton garde son sens dans
le fichier du rewind, qui exige ce jeton. Les déplacer coûterait des lignes pour
retirer un contexte.

### 2.3 `chat.rs` — sa copie retirée, et rien d'autre

`type SourceEnv` (l. 57) et `fn jeton_interne` (l. 70) disparaissent ;
`ouvrir_session_par_spirit:104` devient
`jeton_interne(lire_env).map_err(anyhow::Error::msg)?`. **Aucun autre
changement** : ni la fonction, ni ses deux bras, ni le message de la sonde, ni
aucun de ses tests, qui restent verts parce que le refus porte toujours les deux
noms de variables qu'ils assertent (§ 0).

### 2.4 `handlers.rs` — `/clear` par les deux portes (D3-D7, D9)

`async fn clear_par_spirit`, posée juste après `rewind_par_spirit` — la voisine
dont elle reprend la forme. `handle_clear` se réduit au chargement du `.env`, à la
lecture de `spirit_url()` et à la délégation. Le bloc l. 139-172 (les deux appels
locaux, leurs deux `tracing::warn!`, le tirage d'UUID, l'envoi au worker, la
vidange et la remise à zéro) migre dans la nouvelle fonction, re-ordonné selon D4
et augmenté des trois messages de D6.

### 2.5 `handlers.rs` — la garde structurelle (D8)

Dans le `mod tests` existant (l. 1408), à la suite de la garde de V2c-1b dont elle
est la jumelle. Import : `mika_common::source_guard::ProductionScanner`, déjà
nommé en chemin complet par la garde voisine.

### 2.6 `session_remote.rs` — deux `expect` et la doc du module (D10)

Deux lignes retirées (l. 51 et l. 259), un paragraphe de doc réécrit : le compte
passe à deux, et l'« État du vol » dit que `clore` est câblée par `/clear`.
**Aucun autre changement** : ni `post`, ni `lire_json`, ni `compacter`, ni les
tests existants.

### 2.7 `crates/mika-cli/CLAUDE.md` — un paragraphe neuf, et deux réconciliations

Un `### /clear passe par mika-spirit (mika#1727, vol V2d-4b)` dans la famille des
sections TUI, après celle de V2d-4a. Il dit l'ordre (ouvrir **puis** clore) et
pourquoi, ce qui arrive sur chaque échec, d'où vient `is_singleton_session`, que
le jeton se lit désormais à un seul endroit, et le nom de la garde avec sa règle —
*quand elle rougit, on passe par la porte, on n'ajoute pas d'entrée*.

Et il réconcilie **deux** phrases que ce vol rend fausses :

- l. 528 : « `end_session(` n'est **pas** un motif — il part au vol V2d-4b, avec
  la porte `clore` et sa propre garde. » Le vol est arrivé.
- l. 531 : « `/clear` ends the current session, creates a new one… » — l'ordre est
  désormais l'inverse, et les deux gestes sont des appels.

Un document qui décrit un comportement disparu est pire qu'un document muet.

---

## 3. Ordre des commits

Chaque garde arrive **avec ses tests, dans le même commit**. Après chacun, le
pilote mesure `git diff --numstat origin/main...HEAD`, plan exclu.

**Avant le commit 1**, hors commit : le pilote écrit la garde de D8 dans le
working tree **sans toucher au reste**, lance
`cargo test -p mika-cli mika1727_v2d4b_la_tui_ne_clot_plus`, observe le **rouge**
nommant `handlers.rs:140` et `handlers.rs:147`, et consigne la sortie dans
`.pilot-scratch/v2d4b/garde-rouge-avant.txt` (mika#2548 — ce répertoire ne se
supprime jamais). Il retire ensuite la garde du working tree et enchaîne : c'est
le « rouge-avant sur le code de main » du critère 8, et il doit être pris avant
que le commit 2 ne retire les violations.

| # | contenu | fichiers | critères |
|---|---|---|---|
| 1 | le jeton commun (§ 2.1), les deux copies retirées (§ 2.2, § 2.3), T1/T2 rebranchés | `session_remote.rs`, `handlers.rs`, `chat.rs` | 1 |
| 2 | `/clear` par les deux portes (§ 2.4) **et** le retrait des deux `expect` + la doc du module (§ 2.6, D10), avec les onze tests rebranchés et les quatre tests neufs | `handlers.rs`, `session_remote.rs` | 2, 3, 4, 5, 6, 7, 9 |
| 3 | la garde structurelle (§ 2.5), posée verte sur le code du commit 2 | `handlers.rs` | 8 |
| 4 | la doc (§ 2.7) | `crates/mika-cli/CLAUDE.md` | — |

**Halte sur la borne.** Si le total dépasse **600 lignes ajoutées**, le pilote
s'arrête : pas de commit suivant, pas de PR. Son dernier message dit
« HALTE borne : N lignes pour 600 », avec le compte par commit. Les commentaires
qui portent un pourquoi **ne se rabotent pas** pour tenir la borne.

**Avant d'ouvrir la PR** : `cargo test -p mika-cli`,
`cargo clippy -p mika-cli --all-targets` (vert, sans `allow`, sans attente
insatisfaite), `cargo fmt --check`,
`bash scripts/verify-egress-manifest.sh --report`, et
`bash scripts/verify-pipeline.sh origin/main`.

---

## 4. Le jeu de tests

Tous dans le `mod tests` existant de `handlers.rs`. La fixture est `test_app()`,
inchangée ; le banc est `fake_spirit`, emprunté à `rewind_remote::tests` **sans
rien y ajouter**.

**Deux aides locales, et pas une de plus :**

```rust
/// Le chemin tel que le banc doit le voir : il associe par le chemin **brut**,
/// et celui de `clore` porte un identifiant concret — d'où la composition au
/// site, à partir de la session que `test_app()` a tirée.
const ROUTE_SESSIONS: &str = "/api/v1/sessions";
const CORPS_OUVERT: &str = r#"{"session_id":"s-du-fil","canonical":false}"#;
const CORPS_CLOS: &str = r#"{"ended":true,"canonical":false}"#;

/// Monte le banc sur `reponses` et fait passer un `/clear` dessus.
async fn par_le_banc(app, reponses, lire_env) -> (String, Arc<Mutex<Vec<RequeteRecue>>>);

/// Le cas nominal : les deux chemins armés en 200, le jeton posé. Les tests qui
/// ne regardent que l'état de l'`App` passent par ici — leur intention est
/// l'état, pas le transport, et l'écrire deux fois onze fois la rendrait
/// illisible.
async fn clear_nominal(app) -> (String, Arc<Mutex<Vec<RequeteRecue>>>);
```

### 4.1 Les onze tests existants, rebranchés

| site | nom | ce qui change |
|---|---|---|
| 1573 | `test_clear_creates_new_session` | l'appel ; `session_id` devient `"s-du-fil"` |
| 1597 | `test_clear_sends_new_session_request` | l'appel seul — l'assertion « un seul `NewSession`, qui porte la session de l'`App` » tient telle quelle |
| 1616 | `test_clear_ends_old_session` | **l'assertion change de nature** : elle lit sur la socket le `POST /api/v1/sessions/{ancienne}/end`, avec son `agent`, au lieu d'`ended_at` en base (critère 7) |
| 1633★ | `test_clear_singleton_preserves_session` | l'appel, plus « le banc n'a rien reçu » — avec une source **vide**, ce qui prouve que le bras singleton ne lit pas le jeton (critère 6) |
| 1669★ | `test_clear_singleton_does_not_end_session` | l'appel ; l'assertion sur `ended_at` reste — sur ce bras, la base locale est toujours la vérité |
| 1803 | `test_clear_resets_all_state_fields` | l'appel seul |
| 1861 | `test_clear_while_thinking` | l'appel seul |
| 1871 | `test_clear_while_responding` | l'appel seul |
| 1885 | `test_clear_with_pending_images` | l'appel seul |
| 1905 | `test_clear_preserves_preferences` | l'appel seul |
| 1984 | `test_clear_with_closed_channel_returns_error` | l'appel seul — le canal fermé tombe **après** les deux portes, comme il tombait après les deux écritures |

Aucun n'est supprimé, et chacun garde son intention : les sept qui mesurent l'état
de l'`App` continuent de ne mesurer que lui.

### 4.2 Les quatre tests neufs

| # | nom | ce qu'il prouve | critère |
|---|---|---|---|
| T-A | `mika1727_v2d4b_clear_ouvre_puis_clot_dans_cet_ordre` | sur la socket : deux requêtes, `POST /api/v1/sessions` **puis** `POST /api/v1/sessions/{ancienne}/end`, chacune `Bearer <jeton>` et un corps portant `agent` ; l'`App` porte la session du fil ; le worker reçoit **un seul** `NewSession` qui la porte | 2 |
| T-B | `mika1727_v2d4b_sans_jeton_clear_refuse_avant_tout_appel` | source `&\|_\| None`, banc **armé** : zéro requête, `session_id` inchangé, les messages affichés intacts, le canal du worker vide, et la sortie nomme `MIKA_INTERNAL_TOKEN` | 3 |
| T-C | `mika1727_v2d4b_un_ouvrir_refuse_ne_clot_rien` | 401 sur `/api/v1/sessions` : la sortie nomme `MIKA_INTERNAL_TOKEN`, **aucune** requête `…/end` n'est partie (une seule vue, et son chemin est celui de l'ouverture), `session_id` inchangé, worker muet | 4 |
| T-D | `mika1727_v2d4b_une_cloture_en_echec_garde_la_nouvelle_session` | ouverture en 200, `…/end` en 500 : `session_id` vaut `"s-du-fil"`, le worker a son `NewSession`, et la sortie dit que l'ancienne n'a pas été close **en nommant la cause** (le `Display` de `SessionRemoteIssue`, qui porte le statut et l'URL) | 5 |

**Le critère 1 est tenu par T1/T2 de V2c-1b**, rebranchés sur la fonction commune
(§ 2.2) : l'un avec une source vide, l'autre avec une source qui ne porte que
`MIKA_DASHBOARD_TOKEN`, contrôle positif compris. Aucun test neuf n'est dû — en
ajouter un troisième dupliquerait l'assertion que ce vol vient d'unifier.

**Le compte de requêtes se lit sans `Debug`.** `RequeteRecue` n'en dérive pas, et
le banc du voisin reste intact : les assertions passent par `vues.len()` et les
champs, jamais par un rendu du vecteur.

**T-C mesure un chemin, et pas seulement un compte.** `vues.len() == 1` ne dirait
pas *laquelle* est partie : un futur `/clear` qui clorait d'abord et ouvrirait
ensuite donnerait le même 1. Le test asserte donc le **chemin** de la vue unique.

### 4.3 La garde

| # | nom | ce qu'il prouve | critère |
|---|---|---|---|
| T-E | `mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` | la garde de D8 : allowlist vide et assertie vide, témoin de lecture, **contre-témoin de masquage**, deux motifs | 8 |

---

## 5. Definition of Done

- [ ] `cargo test -p mika-cli` vert, les onze rebranchés et T-A à T-E compris.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans `allow`** et **sans
      attente insatisfaite**. Les `expect(dead_code)` de `clore` et
      d'`EndedSessionMirror` ont disparu ; ceux qui restent portent
      `reason = "câblé par V2d-4, mika#1727"`, et la doc du module annonce leur
      compte **mesuré** sous cette passe.
- [ ] `cargo fmt --check` vert.
- [ ] `grep -rn "fn jeton_interne" crates/mika-cli/src` rend **une seule** ligne,
      dans `commands/session_remote.rs`.
- [ ] Le rouge-avant de la garde est consigné dans
      `.pilot-scratch/v2d4b/garde-rouge-avant.txt`, puis le vert est montré.
- [ ] `git diff --numstat origin/main...HEAD` (plan exclu) **≤ 600 lignes
      ajoutées**. Sinon : halte, message « HALTE borne : N lignes pour 600 ».
- [ ] `git diff --name-only origin/main...HEAD` ne montre que
      `crates/mika-cli/src/tui/commands/handlers.rs`,
      `crates/mika-cli/src/commands/session_remote.rs`,
      `crates/mika-cli/src/commands/chat.rs`,
      `crates/mika-cli/CLAUDE.md` et le plan. En particulier :
      `tui/app.rs`, `commands/ask.rs`, `commands/rewind_remote.rs`,
      `commands/dashboard.rs`, `docs/openapi/`, `docs/slash-commands.md`,
      `docs/egress/`, `crates/mika-agent/` et `crates/mika-a2a/` sont intacts.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert. **S'il est rouge, le
      pilote s'arrête et le dit** — il ne touche pas `docs/egress/`.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] La PR porte `Refs mika#1727`, et **aucun** `Closes`, `Fixes` ni `Resolves`,
      ni dans le corps ni dans un commit — y compris si le gabarit `wip-rescue`
      l'ouvre (il écrit `Closes #N` par défaut : corrigé à la main sur #3077 et
      #3081). Vérifié par `gh pr view <N> --json closingIssuesReferences` rendant
      `[]`.
- [ ] Le corps de la PR porte `Code review: delegated to mika-qa (mika#2694)` et
      les lignes `Tracked in: senara-solutions/mika#1727` du § 7.
- [ ] **Aucun ticket n'est ouvert** (gel jusqu'au 01/12), pas même de suivi.
- [ ] Si un hook ou un classifieur refuse un geste, au pilote ou à un sous-agent :
      **arrêt et déclaration**, même si cela semble un faux positif. Aucun
      contournement par un autre outil (pas de heredoc Bash à la place de `Write`,
      pas de reformulation) — incident mika#2869 du 2026-10-21.

---

## 6. Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#1727, § *Critères de V2d-4b* :

1. Il n'existe qu'une `jeton_interne` dans `crates/mika-cli/src/` :
   `grep -rn "fn jeton_interne" crates/mika-cli/src` rend une seule ligne, dans
   `session_remote.rs`. Son refus nomme `MIKA_INTERNAL_TOKEN` et dit que
   `MIKA_DASHBOARD_TOKEN` ne suffit pas. Un test le montre avec une source vide,
   et un autre avec une source qui ne porte que `MIKA_DASHBOARD_TOKEN`.
2. Sur un agent non singleton, `/clear` envoie `POST {base}/api/v1/sessions`, puis
   `POST {base}/api/v1/sessions/{ancienne}/end`, dans cet ordre, chacun avec
   `Authorization: Bearer <jeton>` et un corps qui porte `agent`. La TUI prend la
   session rendue par `ouvrir`, et le worker reçoit un seul `NewSession` qui la
   porte. Un test lit ces faits sur la socket du banc et sur le canal du worker.
3. Sans `MIKA_INTERNAL_TOKEN`, `/clear` refuse **avant tout appel** : le banc ne
   reçoit aucune requête, `session_id` ne change pas, les messages affichés
   restent, et le worker ne reçoit rien. Un test le montre.
4. Si `ouvrir` échoue (un test sur un 401, dont le message nomme
   `MIKA_INTERNAL_TOKEN`), aucune requête `…/end` n'est envoyée, `session_id` ne
   change pas, et le worker ne reçoit rien.
5. Si `clore` échoue après un `ouvrir` réussi, la TUI garde la nouvelle session,
   le worker reçoit son `NewSession`, et le message rendu dit que l'ancienne
   session n'a pas été close, en nommant la cause. Un test le montre.
6. Sur un singleton, `/clear` n'appelle rien (le banc ne reçoit aucune requête,
   même sans jeton) et garde sa session. Les deux tests singleton existants
   restent verts.
7. Les neuf tests existants de `/clear` restent, rebranchés sur le banc, chacun
   avec son intention. `test_clear_ends_old_session` lit la requête `…/end` sur la
   socket, et non plus `ended_at` en base.
8. La garde structurelle refuse `create_session(` et `end_session(` dans la moitié
   production de `tui/commands/handlers.rs`. Son allowlist est vide et assertie
   vide, et son témoin de lecture est présent. Le pilote montre son rouge-avant
   sur le code de main, puis son vert.
9. Le pilote lance `cargo clippy -p mika-cli --all-targets`. Le résultat est vert,
   sans `allow`, et sans attente insatisfaite. Les `expect(dead_code)` qui restent
   portent `reason = "câblé par V2d-4, mika#1727"`, et la doc du module annonce
   leur compte exact.
10. Seuls `tui/commands/handlers.rs`, `commands/session_remote.rs`, `commands/chat.rs`
    (la copie du jeton seulement) et `crates/mika-cli/CLAUDE.md` changent. Le
    pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir
    la PR. S'il le trouve rouge, il s'arrête et le dit.
11. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre.

---

## Fire-Disposition

**Le détecteur livré par ce vol est la garde structurelle de D8** (T-E,
`mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local`), et sa disposition est
l'**option (a) — exception nommée en allowlist**, dans sa forme **à zéro entrée** :
`const ALLOWED: &[&str] = &[]`, assertie vide au premier rang du test.

Les quatre propriétés exigées par mika#1574 :

- **chaque violation reçoit son entrée** — la population est vide *parce qu'elle a
  été vidée*, et non parce qu'elle n'a pas été regardée : § 0 mesure que les deux
  seules occurrences de production sont `handlers.rs:140` et `:147`, et le commit 2
  les retire. La garde est donc posée verte sur un fichier sans dette, et son
  rouge-avant est consigné (§ 3) ;
- **l'entrée nomme la donnée précise** — à zéro entrée, c'est le **motif** qui
  porte la précision : chaque couple `(motif, porte)` nomme l'appel refusé *et* la
  porte qui le remplace (`session_remote::ouvrir`, `session_remote::clore`), donc
  le message d'échec donne le remède sans demander de lire le plan ;
- **elle référence un ticket de suivi** — le nom du test porte `mika1727_v2d4b`,
  grep-visible par `rg 'mika1727_v2d4b' crates/mika-cli/src/tui/commands/handlers.rs` ;
- **elle rougit quand elle devient stale** — c'est l'`assert!` sur `ALLOWED`,
  doublé du **témoin** (`async fn handle_clear(`) et du **contre-témoin**
  (`fn test_clear_singleton_does_not_end_session`). Une allowlist à zéro entrée ne
  peut pas devenir stale par péremption d'une exception ; ses deux vrais modes de
  panne sont la **vacuité silencieuse** (chemin faux, fichier renommé, région
  masquée à tort) et, propre à ce fichier, le **masquage perdu** qui ferait rougir
  la garde sur un nom de test. Le témoin couvre le premier, le contre-témoin le
  second.

**La règle de remédiation, écrite ici et dans le test.** Quand la garde rougit, le
remède est de router l'appelant par la porte que son message nomme — **jamais**
d'ajouter une ligne à `ALLOWED`. Une allowlist qui cesse d'être vide ici
réintroduit exactement le faux vert que la garde existe pour empêcher (forme
mika#2425 / mika#2457).

**Le second détecteur du vol est l'`expect(dead_code)`**, et sa disposition est
celle de V2d-3a, V2d-3b et V2d-4a, **reconduite sans changement** : l'attribut
posé sur l'item qu'il couvre, `expect` et jamais `allow` — au câblage, il devient
`unfulfilled_lint_expectation` et le build le dit. Ce vol en **retire** deux et
n'en ajoute aucun ; D10 impose que le retrait voyage dans le commit qui câble.

**Les quinze autres tests du § 4 ne sont pas des détecteurs**, et c'est une
évaluation, pas une omission : leur chemin de succès est *« la porte a rendu
ceci »*, *« la socket a vu cela »* ou *« l'`App` porte cet état »*, pas *« aucune
violation n'a été trouvée »*. Ils n'ont donc ni population à disposer ni allowlist
à tenir.

---

## 7. Ce que ce vol ne fait pas, et la dette nommée

Noté dans la PR, sous `Tracked in: senara-solutions/mika#1727`. **Aucun ticket
n'est ouvert** : le gel court jusqu'au 01/12, et ce vol n'ouvre ni suivi ni
sous-ticket.

- **Les trois sorties de `chat.rs` et `/compact` — V2d-4c.** Les `end_session(` de
  `chat.rs` (changement d'agent, `/restart`, fin de session) et le
  `maybe_compact(` de `handlers.rs:205` restent locaux. Ni l'un ni l'autre n'est
  dans les motifs de la garde, délibérément : les y mettre la rendrait rouge le
  jour de sa pose. V2d-4c les ajoute **avec** leur câblage, et câble `compacter`,
  dont les deux `expect(dead_code)` restants tiennent la place.
- **Le troisième bras de D6 n'est pas produit par le serveur d'aujourd'hui.**
  `{ended: false, canonical: false}` n'a aucun producteur dans
  `server/sessions.rs` ; le message existe quand même, parce qu'un client qui
  présume l'exhaustivité d'un contrat qu'il ne possède pas écrit son propre faux.
  Il n'est pas testé, et c'est cohérent : le banc pourrait le fabriquer, mais le
  test n'attesterait que du banc.
- **La session orpheline après un `clore` en échec n'est pas réconciliée.**
  D7 garde la nouvelle session ; l'ancienne reste ouverte côté serveur. Il n'y a
  pas de reprise, pas de file, pas de nouvel essai : ce serait un mécanisme de
  convergence, hors de ce vol et hors de la série V2d.
- **`dashboard::auth_token()` accepte toujours `MIKA_DASHBOARD_TOKEN`.**
  `dashboard.rs` reste intact : c'est le contrat légitime de la sonde du tableau
  de bord. Ce vol ferme l'angle mort là où il mord — au refus, qui le **nomme**
  maintenant depuis un seul endroit.
- **T1/T2 de V2c-1b restent dans `handlers.rs`** alors que leur sujet a déménagé
  (§ 2.2). Le ticket dit « rebranchés », et le contexte qu'ils portent — le rewind
  exige ce jeton — se perdrait au déplacement.
- **`commands/ask.rs` n'est pas la TUI** et ne passe pas par ce chemin.
- **Les deux gardes de `handlers.rs` ne sont pas fondues** (D8), et ce n'est pas
  un reste : un message d'échec qui nomme cinq remèdes dont quatre sont hors sujet
  est une garde dont le rouge demande de lire le plan.

---

## Taille estimée

| livrable | lignes ajoutées (hors `docs/` et `CLAUDE.md`) |
|---|---|
| `session_remote.rs` — `SourceEnv` + `jeton_interne` `pub(crate)`, doc comprise | 32 |
| `session_remote.rs` — les deux `expect` retirés et la doc du module réécrite | 14 |
| `handlers.rs` — `clear_par_spirit` : les cinq temps et les trois messages de D6 (commentaires compris) | 78 |
| `handlers.rs` — `handle_clear` réduite au `.env`, à l'URL et à la délégation | 16 |
| réécriture en ajouts du bloc l. 139-172 re-routé (le `numstat` le compte deux fois) | 20 |
| `handlers.rs` — les copies locales retirées, l'import posé, T1/T2 rebranchés | 10 |
| `chat.rs` — sa copie retirée, l'import et le `map_err` posés | 6 |
| `handlers.rs` tests — les deux aides locales et le rebranchement des onze tests | 56 |
| `handlers.rs` tests — les quatre tests neufs T-A à T-D | 124 |
| `handlers.rs` — la garde structurelle T-E | 36 |

Total estimé : 392 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** Le total ci-dessus est écrit dans l'unité
de mika#2636 — **lignes de code, hors `docs/` et hors `CLAUDE.md`** —, celle que
la borne par vol (429) compare : 392, soit une marge de 37 lignes (~9 %). La borne
du ticket (600) compte `git diff --numstat origin/main...HEAD` plan exclu, **doc
et `CLAUDE.md` compris** : 392 + 28 pour `crates/mika-cli/CLAUDE.md` = **420**,
soit une marge de 180. Les deux unités sont écrites plutôt que l'une devinée,
parce que c'est le seul moyen pour que la halte du pilote et le jugement de
l'architecte portent sur le même nombre. La borne par PR (1000) ne mord pas.

**La ligne de réécriture est déclarée exprès**, parce que les deux bornes comptent
des **lignes ajoutées** et non un delta net : les lignes du bloc l. 139-172,
re-ordonnées selon D4 dans la nouvelle fonction, comptent en ajouts tout en
disparaissant de leur site. L'omettre ferait que la première mesure du pilote
dépasserait son estimation sans qu'aucun travail imprévu n'ait eu lieu — la classe
d'erreur que mika#2693 existe pour supprimer.

**Pourquoi un seul vol tient.** Cinq raisons mesurées en § 0, et non optimistes.
*Les deux portes sont déjà là, vertes et testées* — `ouvrir`, `clore`, `post`,
`session_url`, `SessionRemoteIssue` et son `Display` sont livrés par V2d-3a et
V2d-3b, et ce vol n'introduit **aucune** route, aucun statut et aucun miroir neuf :
il pose un appelant. *Le banc est déjà emprunté par deux voisins* — `fake_spirit`,
`RequeteRecue` et `DeadEndpoint` sont `pub(crate)` depuis V2c-1b et servent déjà
`session_remote.rs` et `chat.rs`, donc ~90 lignes de factice ne sont pas
dépensées, et `rewind_remote.rs` reste intact. *La fixture existe* — `test_app()`
sert déjà les onze tests de `/clear`, et aucun `AppContext` n'est à construire.
*Le modèle de câblage est dans le même fichier, à trente lignes de là* —
`handle_rewind_impl` / `rewind_par_spirit`, dont ce vol copie la forme et non le
texte. *Le jeton commun est un déplacement, pas une réconciliation* — les deux
copies ont la même forme à un type de retour près (§ 0), ce que V2d-4a avait
garanti en les écrivant jumelles exprès.

Le poste le plus lourd est les tests (216 pour 176 de production), et c'est
conforme : le ticket exige un test par fait de socket, un test par échec de porte,
la préservation des onze existants, et « les tests ne se reportent jamais à un vol
suivant, même si le code reste ». Si le budget ne tenait pas avec eux, c'est le
périmètre qui se réduirait, jamais les tests.

Pas de découpage en vols supplémentaires : la suite est déjà découpée par le corps
du ticket en V2d-4c, V3 et V4.

# Vol V2d-3b — les portes client « clore » et « compacter » (mika#1727)

**Ticket :** senara-solutions/mika#1727 — `Refs`, jamais `Closes`. Le ticket se ferme au merge de V4.
**Branche :** `feat/1727/tui-v2d3b-portes-cloture-compaction-de-spirit`, neuve depuis `main`.
**Base mesurée :** `0b44a562` (merge de V2d-3a, #3086).

V2d-3a a posé le transport (`post`, `session_url`), le vocabulaire d'erreur
(`SessionRemoteIssue`), l'entrée d'egress `cli-spirit-sessions` et **une** porte
(`ouvrir`). Ce vol pose les deux qui manquent — `clore` et `compacter` — sur ce
même transport, et remet à jour les textes du module que ces portes rendent
faux. **Aucun appelant de la TUI ne change** ; la bascule est V2d-4.

**Un seul fichier change :** `crates/mika-cli/src/commands/session_remote.rs`.

---

## 0. Ce qui est mesuré

Tout ce qui suit a été lu sur la branche, à `0b44a562`. Les numéros de ligne
servent à retrouver le site, pas à être rejoués tels quels.

| fait | site | ce qu'il décide ici |
|---|---|---|
| `post(base_url, token, chemin, corps) -> Result<(u16, String), SessionRemoteIssue>` ; `chemin` est le chemin **entier** | `session_remote.rs:104-144` | les deux portes passent par là sans le modifier. `clore` compose son chemin, `compacter` a le sien en constante (§ 1, D2) |
| les cinq bras : `200 => Ok`, `401 => Denied`, `404 => NotFound { url }`, `409 => Busy`, `_ => Unexpected { status: Some(_) }` ; transport ⇒ `Unexpected { status: None }` | `session_remote.rs:130-143` | **rien à ajouter au transport** : les deux portes de ce vol n'introduisent aucun statut neuf |
| `session_url(base_url, chemin)` absorbe la barre finale, et existe « parce qu'elle a **deux** lecteurs » | `session_remote.rs:95-97` | le même usage dans les deux portes neuves — recomposer l'URL pour la nommer dans un `Unexpected` |
| `ouvrir` : `agent` toujours envoyé, statut relu **avant** la forme, `from_str` dont l'`Err` devient `Unexpected { status: Some(statut), url, body: format!("unreadable created session ({cause}): {corps}") }` | `session_remote.rs:154-179` | la séquence exacte que les trois portes partagent — donc l'objet de l'extraction de § 1, D1 |
| `#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]` sur `CreatedSessionMirror` (l. 33) **et** sur `ouvrir` (l. 153) | `session_remote.rs:33`, `:153` | le compte réel est **déjà** de deux, alors que la doc du module en annonce un seul (§ 1, D5) |
| doc du module : « Une seule porte ici, `ouvrir` » et « Le seul `expect(dead_code)` du module est donc sur `CreatedSessionMirror` » | `session_remote.rs:18-24` | **deux** phrases à réécrire, la seconde étant déjà fausse sur `main` |
| doc de `SessionRemoteIssue` : « aucune porte de ce vol ne peut l'obtenir », « le producteur réel est `/api/v1/compact`, dont la porte arrive en V2d-3b » | `session_remote.rs:44-47` | la troisième phrase à réécrire : `compacter` est ce producteur |
| commentaire du test du 409 : « aucune porte de ce vol ne peut obtenir ce bras, et son producteur réel (`/api/v1/compact`) arrive en V2d-3b » | `session_remote.rs:264-265` | **un quatrième texte**, que le ticket ne nomme pas et qui devient faux de la même manière (§ 1, D4) |
| les cinq tests de `post` (401, 404, 409, hors contrat, injoignable) et le helper `un_statut` | `session_remote.rs:196-322` | ils **ne se refont pas** par porte. Le 409 de la porte (critère 5) est un test de plus, pas un remplacement (§ 1, D4) |
| `mika1727_un_200_illisible_nest_jamais_un_succes` assert le texte `"unreadable created session"` | `session_remote.rs:347` | c'est **l'attestation de l'extraction** de D1 : elle ne change pas le message d'`ouvrir`, et ce test le prouve sans être touché |
| `EndSessionRequest { #[serde(default)] agent: String }` ⇒ `200 {ended: bool, canonical: bool}` ; 404 sur agent inconnu **et** sur session inconnue ou étrangère, par le **même** `session_not_found` ; 500 | `server/sessions.rs:113-217`, `:219-227` | les deux champs d'`EndedSessionMirror`, et le fait que le 404 ne distingue rien (§ 1, D3) |
| la garde canonique passe **avant** la lecture en base, et rend `ended(false, true)` — un **200** | `server/sessions.rs:162-164` | le critère 2 : `{ended: false, canonical: true}` est un succès, pas une erreur |
| `CompactRequest { #[serde(default)] agent: String }` ⇒ `200 {compacted: bool, messages: usize}` ; **409** quand `try_lock_owned` échoue ; 404 ; 500 | `server/compact.rs:24-113` | les deux champs de `CompactedMirror`, le type `usize`, et le seul producteur de 409 des trois routes |
| `compacted` = « demandée et pas échouée », jamais « une ligne `summary` a été écrite » ; `messages` est le compte **d'avant**, dans les deux issues | `server/compact.rs:33-45` | ce que les doc-comments du miroir doivent dire, et qu'on ne lit pas dans les noms de champs |
| routes montées : `.route("/sessions", …)`, `.route("/sessions/{id}/end", …)`, `.route("/compact", …)` sous `/api/v1` | `server/mod.rs:398-400` | les deux chemins exacts de ce vol |
| `fake_spirit(&[(chemin, statut, corps)])` associe par `requete.uri().path()` **brut**, `fallback` unique, chemin non associé ⇒ **501** | `rewind_remote.rs:548-592` | le factice de `clore` se déclare sur le chemin **complet**, identifiant inclus (§ 1, D2) |
| `RequeteRecue { methode, chemin, autorisation, corps }`, sans `Debug` | `rewind_remote.rs:535-540` | les assertions de socket s'écrivent sans rendu `Debug`, comme l. 222-224 |
| le banc est déjà importé par les tests de ce fichier | `session_remote.rs:189` | **aucune ligne d'import à ajouter** |
| entrée `cli-spirit-sessions`, ligne `data` nommant **les trois** routes et l'identifiant « qui voyage dans le chemin » | `docs/egress/egress-manifest.toml:374-382` | `docs/egress/` **n'est pas touché** : la description est déjà juste pour ce vol |
| `mika-common` + `test-utils` en `[dev-dependencies]`, `DeadEndpoint::reserve()` déjà appelé ici | `crates/mika-cli/Cargo.toml:67`, `session_remote.rs:191` | aucune dépendance à ajouter |

Trois faits qui sont des **absences**, et qu'il faut dire :

- `docs/openapi/mika-spirit.yaml` ne décrit aucune des trois routes de session.
  L'écart est antérieur à ce vol et son comblement toucherait un fichier exclu.
- `/api/v1/sessions/{id}/end` ne prend **aucun verrou d'agent**
  (`server/sessions.rs:145-149`), donc `clore` ne peut pas obtenir de 409. Le
  bras existe pour `compacter` seul, et c'est pourquoi le critère 5 est écrit sur
  cette porte-là.
- `commands/mod.rs` porte déjà `pub mod session_remote;` : rien à y ajouter.

---

## 1. La forme, et les cinq décisions qui la tiennent

Deux miroirs, deux portes, quatre textes réécrits. Ce qui mérite d'être écrit,
ce sont les endroits où la forme évidente est fausse.

### D1 — La séquence des portes est **extraite**, et `ouvrir` passe par l'extrait

C'est la décision qui porte ce vol, et c'est la seule qui touche du code déjà
livré.

Les trois portes partagent une séquence de quatre gestes : composer le corps,
appeler `post`, **relire le statut avant la forme**, désérialiser en traduisant
l'`Err` en `Unexpected` portant statut, URL et corps reçu. Les trois premiers
gestes diffèrent par porte ; **les deux derniers sont identiques**, à un libellé
près. Les recopier donnerait trois exemplaires d'une même connaissance, dans un
ticket dont le § V3 nomme explicitement la dérive de trois copies comme la panne
à ne pas reproduire (« les copies ont déjà dérivé entre elles »). Et le critère 6
exige que le comportement soit **le même** sur les trois : une propriété à tenir
en trois endroits est une propriété qui dérivera.

Donc une fonction, pure et non-async :

```rust
fn lire_json<T: serde::de::DeserializeOwned>(
    statut: u16,
    corps: String,
    url: String,
    quoi: &str,
) -> Result<T, SessionRemoteIssue>
```

Elle porte le `if statut != 200` (avec le commentaire qui dit pourquoi la
condition est écrite et non supposée — il descend d'`ouvrir`, il ne se réécrit
pas) et le `from_str` dont l'`Err` devient
`Unexpected { status: Some(statut), url, body: format!("unreadable {quoi} ({cause}): {corps}") }`.
`quoi` vaut `"created session"`, `"ended session"` et `"compaction result"`.

**L'extraction est attestée par un test qui existe déjà et qu'on ne touche
pas.** `mika1727_un_200_illisible_nest_jamais_un_succes` (l. 324-356) assert la
forme `Unexpected { status: Some(200), .. }`, le texte `"unreadable created
session"` **et** le corps reçu dans le message. Avec `quoi = "created session"`,
le message d'`ouvrir` est inchangé octet pour octet, et ce test est la mesure du
non-changement. C'est ce qui rend le re-routage d'`ouvrir` sûr sans écrire un
seul test de régression : il en existe un, et il est spécifique.

**Ce qui n'est pas extrait, et pourquoi.** Le `json!({ "agent": agent })` reste
chez chaque porte, et le chemin aussi. Les remonter dans `lire_json`
transformerait une fonction qui répond à « comment je lis une réponse » en une
fonction qui répond aussi à « quoi j'envoie » — c'est-à-dire en un second `post`,
au-dessus du premier. Chaque porte reste donc le site où se lit, en entier, ce
qu'elle met sur le fil.

**Le coût, nommé.** Ce vol modifie `ouvrir`, livrée verte par V2d-3a. Le ticket
n'interdit rien dans ce fichier — « le seul fichier touché est
`session_remote.rs` » — et `ouvrir` y est. Le diff y gagne une dizaine de lignes
en ajouts (lignes réécrites), et les perd en net : § *Taille estimée* compte les
deux.

### D2 — Le chemin de `clore` se compose au site, celui de `compacter` est une constante

`ROUTE_SESSIONS` (`"/api/v1/sessions"`) existe déjà. Le chemin de clôture est
`format!("{ROUTE_SESSIONS}/{session_id}/end")`, composé **une fois** dans un
`let chemin` au début de `clore`, parce qu'il a deux lecteurs : `post` et le
`session_url` que `lire_json` recompose. Pas de fonction `route_fin_de_session` :
elle n'aurait qu'un appelant, et la constante qu'elle réutiliserait est déjà là.

`compacter` prend une constante neuve, `ROUTE_COMPACT: &str = "/api/v1/compact"`,
posée à côté de `ROUTE_SESSIONS`. Elle a trois lecteurs : la porte, le
`session_url` de son `Unexpected`, et le test de socket qui assert le chemin reçu
— le troisième est la raison, parce que c'est lui qui rend la constante
vérifiable plutôt que simplement partagée.

**Conséquence sur le banc, à ne pas rater.** `fake_spirit` associe par le chemin
**brut** de la requête. Le factice de `clore` se déclare donc sur
`"/api/v1/sessions/s-1/end"` — identifiant concret inclus — et non sur un
gabarit. Un chemin non associé rend 501, qu'aucun bras de `post` ne traduit :
c'est ce qui fait du test une mesure de la **visée** du chemin, et non seulement
du statut rendu. Le test du critère 1 vaut donc comme garde sur le `format!` de
la porte.

### D3 — Un `{ended: false, canonical: true}` est un succès, et la porte ne l'interprète pas

Le serveur rend ce couple en **200** quand la session visée est la canonique de
l'agent (`server/sessions.rs:162-164`, invariant mika#1401) : la canonique ne se
clôt jamais, et c'est un fait, pas un refus. `clore` le rend **tel quel**, sans
le convertir en erreur et sans le taire.

C'est le critère 2, et c'est une décision plutôt qu'une évidence, parce que la
forme évidente est fausse dans les deux sens : une porte qui rendrait
`Err(NotFound)` sur `ended: false` inventerait un échec là où le serveur a
répondu normalement ; une porte qui rendrait `Ok(())` en jetant les deux champs
priverait V2d-4 de la seule information dont il a besoin — *ma session a-t-elle
été fermée ?* — au moment où il décide d'en rouvrir une.

**`clore` ne lit donc aucun des deux champs.** La lecture est V2d-4. Un
`if !ended` ici serait une règle de domaine dans un miroir de fil, ce que ce
module refuse depuis `rewind_remote.rs:19-21`.

Corollaire sur le 404 (critère 3) : `handle_end_session` rend le **même** 404
pour un agent inconnu, une session inconnue et une session appartenant à un
autre agent, par un seul site (`session_not_found`), et le commentaire de ce site
dit pourquoi — deux issues distinguables laisseraient un porteur du jeton interne
énumérer les sessions du conteneur. La porte n'a donc rien à distinguer, et le
message de `NotFound` qui nomme ses **deux** causes sans en choisir une est déjà
exactement juste pour elle.

### D4 — Le 409 est mesuré **deux fois**, et ce n'est pas une redite

`mika1727_un_409_est_un_verrou` (l. 262-269) existe et reste : il mesure le bras
du **transport**, et le ticket dit que les tests de `post` ne se refont pas par
porte. Le critère 5 exige un test « par la porte elle-même ». Les deux coexistent
parce qu'ils mesurent deux choses :

- le test de `post` : un 409 sur le fil devient `Busy` ;
- le test de `compacter` : la porte **ne mange pas** ce `Busy`. C'est la panne
  réelle que `lire_json` rend possible — un `?` mal placé, ou un statut relu
  avant que l'erreur ne remonte, et le 409 ressortirait en `Unexpected`, soit un
  verrou d'agent affiché comme un écart de version.

Et le commentaire du test de `post` devient faux : il dit « aucune porte de ce
vol ne peut obtenir ce bras, et son producteur réel (`/api/v1/compact`) arrive en
V2d-3b ». C'est le **quatrième** texte que ce vol rend faux, et le ticket n'en
nomme que trois. Il est réécrit dans le commit de `compacter` : le test reste sur
`post` **par choix** — le bras est une propriété du transport — et non plus par
impossibilité.

### D5 — Le compte des `expect(dead_code)` se mesure, et s'écrit **en dernier**

Le compte réel sur `main` est **deux** (`CreatedSessionMirror` l. 33, `ouvrir`
l. 153) alors que la doc en annonce un seul : la phrase est fausse avant ce vol,
et le critère 7 la fait réparer ici.

Les candidats à la fin du vol sont les six items sans appelant de production :
les trois miroirs et les trois portes, plus `lire_json`, `post`, `session_url` et
`SessionRemoteIssue` qu'elles seules atteignent. Le piège de D6 de V2d-3a vaut
intact : `rustc` ne signale `dead_code` que sur la **racine** d'un sous-graphe
inatteignable, donc un attribut posé sur un item que le lint ne vise pas devient
`unfulfilled_lint_expectation`, soit un warning de plus. L'attribut rougit dans
les deux sens, et c'est sa valeur.

**Conduite, en trois temps, et le troisième est un geste de travail à part
entière.**

1. Le pilote pose
   `#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]`
   à la naissance de chaque item neuf, dans le commit qui le livre — un commit
   qui laisse un `dead_code` nu derrière lui n'est pas vert.
2. Dans chaque commit, les phrases de doc **qualitatives** sont corrigées : le
   nombre de portes, et le fait que `compacter` est le producteur du 409.
3. Au dernier commit, `cargo clippy -p mika-cli --all-targets` : il **retire**
   chaque attribut insatisfait, puis écrit le **chiffre** mesuré dans la doc du
   module. `--all-targets` compile les deux faces du `cfg_attr` : la cible de
   test, où l'attribut n'existe pas et l'item est vivant, et la cible normale, où
   il est mort.

Le chiffre du plan n'est pas une prédiction à honorer : si la mesure contredit
l'estimation, **c'est la mesure qui s'écrit**. C'est précisément ce que le
critère 7 demande, et pourquoi ce geste ne peut pas être un contrôle de fin de
course.

`allow` n'est **jamais** substitué à `expect` pour faire taire un warning :
`allow` est muet au câblage, et c'est l'obligation faite à V2d-4 de retirer
l'attribut qui justifie son existence. Le texte de `reason` est fixé par le corps
du ticket et ne varie pas.

---

## 2. Les livrables, un par un

Tout est dans `crates/mika-cli/src/commands/session_remote.rs`.

### 2.1 Les deux miroirs

```rust
#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) struct EndedSessionMirror {
    /// Faux **sans erreur** quand la session visée est la canonique de l'agent :
    /// elle ne se clôt jamais (mika#1401, `server/sessions.rs:162-164`). La porte
    /// rend le couple tel quel ; c'est V2d-4 qui le lira (D3).
    pub(crate) ended: bool,
    pub(crate) canonical: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) struct CompactedMirror {
    /// « Demandée et **pas échouée** », jamais « une ligne `summary` a été
    /// écrite » : `maybe_compact` relit le seuil elle-même et peut légitimement
    /// ne rien faire, et la compaction d'après tour tourne hors du verrou
    /// (`server/compact.rs:34-40`).
    pub(crate) compacted: bool,
    /// Compte **d'avant**, dans les deux issues : c'est celui sur lequel la
    /// décision de compacter a été prise. `usize`, comme le champ servi.
    pub(crate) messages: usize,
}
```

`Deserialize` seul, pour la raison déjà écrite sur `CreatedSessionMirror` :
`EndSessionResponse` et `CompactResponse` ne dérivent que `Serialize` et vivent
dans `crates/mika-agent/`, hors périmètre. Les deux doc-comments portent le
**pourquoi** que les noms de champs ne portent pas, et ils sont la reprise de la
seule source qui le dise (le serveur), non une paraphrase.

### 2.2 `lire_json<T>` — l'extrait de D1

Posée après `post` et avant les portes. Elle porte deux commentaires, tous deux
descendus d'`ouvrir` sans réécriture : pourquoi le statut est relu plutôt que
supposé, et pourquoi le corps reçu voyage avec la cause. Un troisième, neuf, dit
pourquoi elle existe : la connaissance « un 200 illisible n'est jamais un
succès » est une propriété des trois portes, et le critère 6 l'exige identique
sur les trois.

`ouvrir` est re-routée dessus avec `quoi = "created session"` : son message est
inchangé, et `mika1727_un_200_illisible_nest_jamais_un_succes` le mesure sans
être touché.

### 2.3 `clore`

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) async fn clore(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
) -> Result<EndedSessionMirror, SessionRemoteIssue> {
    let chemin = format!("{ROUTE_SESSIONS}/{session_id}/end");
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, &chemin, &envoye).await?;
    lire_json(statut, corps, session_url(base_url, &chemin), "ended session")
}
```

Deux commentaires, et aucun de paraphrase :

1. `agent` part **toujours**, y compris vide — le serveur déclare le champ
   `#[serde(default)]` (`server/sessions.rs:115-119`) et un champ absent lui fait
   résoudre *son* agent par défaut. La porte clorait alors la session d'un autre
   agent, et le 404 d'appartenance ne la rattraperait pas : `resolve_agent`
   aurait réussi.
2. L'identifiant est inséré **tel quel**, sans encodage, et c'est un choix borné
   par le ticket — mesure de Vincent du 2026-11-14 sur la table `sessions` de
   gentux (aucun `/`, `?`, `#`, `%` ni espace), et V2d-4 ne clôt que des
   identifiants rendus par `ouvrir` (UUIDv4, ou la canonique). Le commentaire
   nomme la dette et `Tracked in: senara-solutions/mika#1727`, parce qu'un
   lecteur qui voit un `format!` dans un chemin doit trouver la raison là où il
   regarde.

### 2.4 `compacter`

```rust
const ROUTE_COMPACT: &str = "/api/v1/compact";

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) async fn compacter(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<CompactedMirror, SessionRemoteIssue> {
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, ROUTE_COMPACT, &envoye).await?;
    lire_json(
        statut,
        corps,
        session_url(base_url, ROUTE_COMPACT),
        "compaction result",
    )
}
```

Un commentaire, qui dit la seule chose qu'on ne lit pas dans le code : cette
porte est le **producteur réel** du bras `Busy` du transport, parce que
`handle_compact` tient le verrou d'agent pendant tout l'appel au LLM et rend 409
quand `try_lock_owned` échoue (`server/compact.rs:71-81`). Les deux routes de
session, elles, ne prennent aucun verrou — donc `clore` ne peut pas obtenir ce
bras, et le `Busy` qui remonte d'ici est un verrou, jamais un écart de version.

### 2.5 Les quatre textes réécrits

| texte | site | ce qu'il dit à la fin |
|---|---|---|
| doc du module, « État du vol » | l. 18-24 | **trois** portes, `ouvrir`, `clore` et `compacter` ; aucun appelant avant V2d-4 ; le compte **mesuré** des `expect(dead_code)`, écrit au dernier commit (D5) |
| doc de `SessionRemoteIssue`, paragraphe `Busy` | l. 44-47 | `compacter` est le producteur réel, et `post` reste le site du bras. Le test du transport sur `post` est un **choix** (le bras est une propriété du transport), plus une impossibilité |
| commentaire du test du 409 | l. 264-265 | même correction, côté test (D4) |
| en-tête du module, « vol V2d-3a » | l. 2-3 | le module sert les trois routes depuis V2d-3a **et** V2d-3b : les deux vols sont nommés, parce qu'un lecteur qui cherche d'où vient `compacter` ne la trouverait pas sous V2d-3a |

Le quatrième n'est pas exigé par le critère 7. Il est fait parce que ne pas le
faire laisserait l'en-tête désigner un vol qui n'a pas livré la moitié de ce
qu'il décrit — la même classe d'écart que les trois autres, et la seule ligne du
fichier qui sert d'index à un lecteur neuf.

---

## 3. Ordre des commits

Trois commits. Chacun porte son code **et** ses tests ; aucun test n'est reporté
à un commit suivant, ni à un vol suivant.

**Commit 1 — `lire_json` et la porte `clore`.** `lire_json<T>`, le re-routage
d'`ouvrir` dessus, `EndedSessionMirror`, `clore`. Doc du module : la phrase
« une seule porte » devient **deux** portes, `ouvrir` et `clore`, en disant que
`compacter` arrive dans ce même vol. Elle sera réécrite une seconde fois au
commit 2, et c'est voulu : un commit intermédiaire qui annoncerait trois portes
serait faux sur son propre arbre, et ce fichier vient précisément de payer deux
phrases devenues fausses. Tests : critères 1, 2, 3, et le 200 illisible de
`clore` (moitié du critère 6). `mika1727_un_200_illisible_nest_jamais_un_succes`
reste **inchangé** et atteste le non-changement d'`ouvrir` (D1).

**Commit 2 — la porte `compacter`.** `ROUTE_COMPACT`, `CompactedMirror`,
`compacter`. Doc du module : **trois** portes. Doc de `SessionRemoteIssue` et
commentaire du test du 409 : `compacter` est le producteur réel (D4). En-tête du
module : les deux vols. Tests : critères 4, 5, et le 200 illisible de `compacter`
(seconde moitié du critère 6).

**Commit 3 — la mesure des `expect`.** Aucun code de production neuf et aucun
test neuf. `cargo clippy -p mika-cli --all-targets`, retrait de chaque attribut
insatisfait, et écriture du **chiffre mesuré** dans la doc du module (D5, temps
3). Si la mesure ne retire rien et confirme le chiffre, le commit ne porte que la
ligne de doc — il existe quand même, pour que le chiffre soit traçable à la
mesure qui l'a produit.

**Après chaque commit** : `git diff --numstat origin/main...HEAD`, plan exclu. Si
le total dépasse **540**, halte — pas de commit suivant, pas de PR, et le dernier
message dit « HALTE borne : N lignes pour 540 » avec le compte par commit.

**Avant d'ouvrir la PR**, dans cet ordre :

1. `cargo test -p mika-cli`
2. `cargo clippy -p mika-cli --all-targets` — vert, **sans `allow`** (déjà fait
   au commit 3 ; ici c'est la confirmation sur l'arbre final)
3. `bash scripts/verify-egress-manifest.sh --report` — critère 8. **Rouge ⇒ halte
   et on le dit** : `docs/egress/` n'est pas touché par ce vol, donc un rouge
   signifie que l'entrée devait changer et le ticket demande alors l'arrêt, pas
   une retouche
4. `bash scripts/verify-pipeline.sh origin/main`
5. `git diff --stat origin/main...HEAD` — un seul fichier de code attendu, plus
   ce plan

---

## 4. Le jeu de tests

Sept tests neufs, aucun helper neuf. Le banc est celui de `rewind_remote`, déjà
importé (l. 189). Les cinq tests de `post` et le 200 illisible d'`ouvrir` restent
tels quels — seul le **commentaire** du test du 409 est réécrit (D4).

| test | critère | ce qu'il assert |
|---|---|---|
| `mika1727_clore_vise_la_route_de_fin_avec_son_jeton` | 1 | factice sur `"/api/v1/sessions/s-1/end"` servant `{"ended":true,"canonical":false}`. Sur la socket : `methode == "POST"`, `chemin == "/api/v1/sessions/s-1/end"`, `autorisation == Some("Bearer jeton-secret")`, corps désérialisé portant `agent == "mika-dev"`. Puis `ended == true` et `canonical == false`. Le chemin déclaré avec son identifiant concret **est** la garde sur le `format!` de la porte : un chemin mal composé tomberait sur le `fallback` à 501, qu'aucun bras ne traduit (D2) |
| `mika1727_clore_rend_la_canonique_telle_quelle` | 2 | même factice, servant `{"ended":false,"canonical":true}` ⇒ **`expect`**, puis `!ended` et `canonical`. `expect` et non `expect_err` **est** la mesure : une porte qui traduirait ce couple en `NotFound` rougirait ici, et c'est la traduction tentante — le serveur n'a rien clos |
| `mika1727_clore_sur_404_rend_notfound` | 3 | factice servant 404 sur le chemin de fin ⇒ `NotFound { .. }`, et le message nomme ses deux causes. Par la **porte** et non par `un_statut` : le critère est sur `clore`, et le 404 du transport est déjà couvert l. 252-260 |
| `mika1727_compacter_vise_la_route_de_compaction_avec_son_jeton` | 4 | factice sur `ROUTE_COMPACT` servant `{"compacted":true,"messages":42}`. Sur la socket : méthode, `chemin == ROUTE_COMPACT`, `Bearer`, corps portant `agent`. Puis `compacted == true` et `messages == 42`. `42` et non `0` : un `messages` jamais désérialisé et laissé à son défaut passerait un test écrit sur zéro |
| `mika1727_compacter_sur_409_rend_un_verrou` | 5 | factice servant 409 sur `ROUTE_COMPACT` ⇒ `Busy`, **par la porte**. Ce que ce test mesure et que celui de `post` ne mesure pas : la porte ne mange pas le `Busy` et ne le reclasse pas en `Unexpected` (D4) |
| `mika1727_clore_un_200_illisible_nest_jamais_un_succes` | 6 | 200 servant `{"ended":true}` — `canonical` **manquant** ⇒ `expect_err`, `Unexpected { status: Some(200), .. }`, message contenant `"unreadable ended session"` et le corps reçu |
| `mika1727_compacter_un_200_illisible_nest_jamais_un_succes` | 6 | 200 servant `{"compacted":true}` — `messages` **manquant** ⇒ même forme, message contenant `"unreadable compaction result"` et le corps reçu |

Les deux corps illisibles sont des **champs manquants**, et non du JSON cassé :
c'est la forme qu'un écart de version produit réellement (le serveur retire ou
renomme un champ), et c'est la seule qui distingue « le client refuse » de « le
client se replie sur un défaut ». Un `"pas du json"` passerait aussi, mais
mesurerait `serde_json` plutôt que la porte. Le champ choisi est à chaque fois
celui dont un repli silencieux serait le plus coûteux : `canonical` décide si
V2d-4 réouvre une session, `messages` est le seul nombre que la compaction rend.

Aucun test de point injoignable n'est ajouté : `mika1727_un_point_injoignable_na_pas_de_statut`
(l. 296-322) mesure le bras `Unexpected { status: None }` du **transport**, que
les deux portes neuves traversent sans le modifier. En écrire deux copies
achèterait la couverture de `post`, déjà acquise, au prix de deux
`DeadEndpoint::reserve()` de plus.

---

## 5. Definition of Done

- `clore(base_url, token, agent, session_id) -> Result<EndedSessionMirror, SessionRemoteIssue>`
  existe, POSTe sur `{base}/api/v1/sessions/{session_id}/end` avec
  `Authorization: Bearer <jeton>` et un corps portant `agent`, et rend
  `{ended, canonical}`.
- `{ended: false, canonical: true}` est rendu comme un **succès**, tel quel ;
  `clore` ne lit ni n'interprète aucun des deux champs.
- `compacter(base_url, token, agent) -> Result<CompactedMirror, SessionRemoteIssue>`
  existe, POSTe sur `{base}/api/v1/compact` avec le `Bearer` et un corps portant
  `agent`, et rend `{compacted, messages}`.
- Un 404 sur la clôture donne `NotFound` ; un 409 sur la compaction donne `Busy`,
  mesuré par la porte.
- Un 200 au corps illisible donne `Unexpected` sur **chaque** porte, jamais un
  succès, et le message porte le corps reçu.
- `lire_json<T>` est le site unique de la relecture de statut et de la
  désérialisation ; les trois portes y passent, et le message d'`ouvrir` est
  inchangé (attesté par `mika1727_un_200_illisible_nest_jamais_un_succes`,
  non modifié).
- Les quatre textes de § 2.5 sont réécrits : trois portes, `compacter` comme
  producteur du 409, le commentaire du test du 409, et l'en-tête nommant les deux
  vols.
- Le compte des `expect(dead_code)` annoncé par la doc est celui **mesuré** sous
  `cargo clippy -p mika-cli --all-targets`, écrit au dernier commit.
- `cargo test -p mika-cli` et `cargo clippy -p mika-cli --all-targets` verts,
  **sans `allow`**. Chaque `expect(dead_code)` porte
  `reason = "câblé par V2d-4, mika#1727"`, et aucun n'est insatisfait.
- `bash scripts/verify-egress-manifest.sh --report` vert ; rouge ⇒ halte
  déclarée, sans retoucher le manifeste.
  `bash scripts/verify-pipeline.sh origin/main` vert.
- Diff limité à `crates/mika-cli/src/commands/session_remote.rs` (+ `docs/plans/`
  pour ce fichier). **Intacts** : `docs/egress/`, `docs/openapi/`,
  `commands/rewind_remote.rs`, `commands/mod.rs`, `commands/chat.rs`,
  `commands/chat_turn.rs`, `tui/`, `crates/mika-agent/`, `crates/mika-a2a/`,
  `.github/workflows/`, `.claude/`, `skills/bundled/`, `well_known_agents.rs`,
  `.mika/`.
- Total ajouté ≤ **540** lignes (`git diff --numstat origin/main...HEAD`, plan
  exclu), mesuré après chaque commit.
- PR : `Refs mika#1727`. Aucun `Closes` / `Fixes` / `Resolves`, corps **et**
  messages de commit. `gh pr view <N> --json closingIssuesReferences` rend une
  liste vide — vérifié même, et surtout, si la PR est ouverte par le gabarit de
  reprise (`wip-rescue`), qui écrit `Closes #N` par défaut et a dû être corrigé à
  la main sur #3077 et #3081.
- Aucun ticket ouvert (gel jusqu'au 01/12). Reste et manques ⇒ commentaire sur
  #1727 ou corps de PR, avec `Tracked in: senara-solutions/mika#1727`.

## 6. Acceptance criteria

Transcrits du corps de #1727, § « Critères de V2d-3b ».

1. `clore` envoie un POST sur `{base}/api/v1/sessions/{id}/end`, avec
   `Authorization: Bearer <jeton>` et un corps qui porte `agent`. Un test lit ces
   trois faits sur la socket du banc, et lit `ended` et `canonical` dans la
   réponse.
2. Sur `{ended: false, canonical: true}`, `clore` rend un succès qui porte ces
   deux valeurs, jamais une erreur. Un test le montre.
3. Sur un 404, `clore` rend `NotFound`. Un test le montre.
4. `compacter` envoie un POST sur `{base}/api/v1/compact`, avec le `Bearer` et un
   corps qui porte `agent`. Un test lit ces faits sur la socket, et lit
   `compacted` et `messages` dans la réponse.
5. Sur un 409, `compacter` rend `Busy`. Un test le montre, par la porte
   elle-même.
6. Sur un 200 au corps illisible, chaque porte rend `Unexpected`, jamais un
   succès. Un test par porte.
7. La doc du module et celle de `SessionRemoteIssue` ne disent plus « une seule
   porte », ni « aucune porte ne peut obtenir `Busy` ». Le compte des
   `expect(dead_code)` qu'elles annoncent est celui du code.
8. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow`. Chaque
   `expect(dead_code)` porte `reason = "câblé par V2d-4, mika#1727"`.
   `bash scripts/verify-egress-manifest.sh --report` est vert. Seul
   `session_remote.rs` change. `docs/egress/`, `rewind_remote.rs`,
   `crates/mika-agent/`, `crates/mika-a2a/`, `tui/` et `docs/openapi/` sont
   intacts.
9. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si elle est ouverte par le gabarit de reprise.

---

## Fire-Disposition

**Le détecteur livré par ce vol est l'`expect(dead_code)` de D5**, et sa
disposition est l'**option (a) — exception nommée en allowlist**, dans sa forme
la plus étroite : l'attribut lui-même, posé sur l'item qu'il couvre et sur rien
d'autre. C'est la disposition de V2d-3a, reconduite parce que ce vol ajoute des
items à la même population.

Les quatre propriétés exigées par mika#1574 :

- **chaque violation reçoit son entrée** — un `#[cfg_attr(not(test),
  expect(dead_code, …))]` par item sans appelant de production, posé à la
  naissance de l'item, jamais un `#![allow(dead_code)]` de module ;
- **l'entrée nomme la donnée précise** — l'item qu'elle couvre, puisqu'elle est
  posée dessus ;
- **elle référence un ticket de suivi** — `reason = "câblé par V2d-4, mika#1727"`,
  grep-visible par `rg 'V2d-4' crates/mika-cli/src/commands/session_remote.rs` ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2d-4, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. D5 interdit explicitement de
  retomber sur `allow`, qui serait muet au câblage.

**Le risque propre à ce vol, nommé.** L'extraction de `lire_json` (D1) **change
le graphe d'atteignabilité** que `rustc` parcourt : `lire_json` est atteinte par
les trois portes, elles-mêmes mortes, donc elle peut être signalée avec elles, à
leur place, ou pas du tout. Le compte de `main` (deux attributs) n'est donc pas
une base à incrémenter — c'est pourquoi le commit 3 existe comme étape de travail
et non comme vérification de fin de course, et pourquoi le chiffre du plan ne
lie pas le pilote (D5).

**Les sept tests du § 4 ne sont pas des détecteurs**, et c'est une évaluation,
pas un oubli : leur chemin de succès est « la porte a rendu `Busy` / un couple
lisible / `Unexpected` », pas « aucune violation trouvée » sur une population
existante. Ils n'ont rien à amnistier. Ce vol ne livre par ailleurs ni lint, ni
garde CI, ni scan structurel, ni garde EndTurn, ni validateur de schéma — la
garde structurelle de la série est V2d-4.

---

## 7. Ce que ce vol ne fait pas, et la dette nommée

**Hors périmètre, par le corps du ticket :**

- **L'encodage de `session_id` dans le chemin.** `clore` insère l'identifiant tel
  quel. Mesure de Vincent du 2026-11-14 sur la table `sessions` de gentux :
  aucune ligne ne contient `/`, `?`, `#`, `%` ni espace ; et V2d-4 ne clôt que
  des identifiants rendus par `ouvrir` (UUIDv4, ou la canonique). Un commentaire
  au site le dit (§ 2.3).
- **Tout appelant dans la TUI** — V2d-4. `chat.rs`, `chat_turn.rs` et `tui/` ne
  sont pas touchés, et les trois portes n'ont donc aucun consommateur à la fin du
  vol. C'est voulu, et c'est ce que les `expect(dead_code)` disent à voix haute.
- **La décision de Vincent sur le démarrage de la TUI sans spirit ni jeton**, qui
  est une précondition de V2d-4 et non de ce vol.
- `/compact`, `/rewind`, `/undo` côté TUI, le provider, les skills, le MCP et les
  équipes — V3 et V4.

`Tracked in: senara-solutions/mika#1727`

**Dette créée ou reconduite par ce vol, et non traitée :**

- **Le transport reste dupliqué d'avec `rewind_remote.rs`**, d'environ 60 lignes,
  et la duplication est volontaire : c'est la granularité de l'entrée d'egress
  (un `client_site` par fichier) qui l'exige, et le raisonnement complet est déjà
  écrit en tête du module. Ce vol **ne l'aggrave pas** — il ajoute deux portes au
  transport existant et en retire une copie interne (D1) — et ne la résorbe pas
  non plus : la fusion serait un ticket sur le lint mika#2408, pas sur ce crate.
- **`docs/openapi/mika-spirit.yaml` ne décrit aucune des trois routes de
  session.** L'écart est antérieur à ce vol, et son comblement toucherait un
  fichier exclu (toute modification serait copiée dans
  `crates/mika-agent/docs/openapi/`).
- **Aucune garde structurelle n'empêche encore la TUI de clore ou de compacter en
  local.** `chat.rs:69-115` écrit toujours sa session dans la base du processus
  de la TUI, et `tui/commands/handlers.rs:194-210` compacte toujours avec le
  fournisseur que la TUI a construit pour elle-même — c'est-à-dire la panne que
  `server/compact.rs` existe pour corriger, et qui reste en vigueur jusqu'à la
  bascule. Poser la garde ici reviendrait à livrer un détecteur que le code viole
  encore, soit l'option (b) d'une Fire-Disposition dont ce vol n'a pas besoin.
  C'est l'objet de V2d-4.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `session_remote.rs` — `EndedSessionMirror` + `CompactedMirror` (doc comprise) | 24 |
| `session_remote.rs` — `lire_json<T>` (commentaires compris), **net** du retrait dans `ouvrir` | 10 |
| `session_remote.rs` — `ROUTE_COMPACT`, `clore`, `compacter` (doc et commentaires compris) | 44 |
| `session_remote.rs` — les quatre textes réécrits (§ 2.5), net | 12 |
| `session_remote.rs` — tests (7 tests neufs, banc et helper déjà présents) | 140 |
| réécriture en ajouts des lignes d'`ouvrir` re-routées (le `numstat` les compte deux fois) | 14 |

Total estimé : 244 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** La borne par vol (429) porte sur les
lignes hors `docs/` : 244, soit une marge de 185 lignes (~43 %). La borne du
ticket (540) porte sur `git diff --numstat origin/main...HEAD` plan exclu, et
**aucun fichier hors code ne change** dans ce vol — donc la même mesure, 244 pour
540, soit une marge de 296. La plus serrée des deux est celle du vol, et c'est
elle qui est tenue. La borne par PR (1000) ne mord pas.

**La ligne de réécriture est déclarée exprès**, parce que les deux bornes
comptent des **lignes ajoutées** et non un delta net : les lignes d'`ouvrir`
remplacées par l'appel à `lire_json` comptent en ajouts tout en disparaissant du
fichier. L'omettre ferait que la première mesure du pilote dépasserait son
estimation sans qu'aucun travail imprévu n'ait eu lieu — la classe d'erreur que
mika#2693 existe pour supprimer.

**Pourquoi un seul vol tient.** Quatre raisons mesurées en § 0, et non
optimistes. *Le transport est déjà là* — `post`, `session_url`,
`SessionRemoteIssue` et son `Display` sont livrés et verts, et les deux portes
n'introduisent aucun statut neuf : les 148 premières lignes du fichier ne
bougent pas. *Le banc est déjà importé* — `fake_spirit` et `RequeteRecue` sont
en place l. 189, le helper `un_statut` existe, et les cinq tests de `post` ne se
refont pas (le ticket le dit) : ce sont ~90 lignes de factice et ~85 lignes de
tests qui ne sont pas dépensées. *Aucune dépendance et aucun fichier voisin* —
`commands/mod.rs` porte déjà la ligne, `docs/egress/` décrit déjà les trois
routes, `mika-common` + `test-utils` est déjà en `[dev-dependencies]`. *Aucune
conversion de domaine* — les deux miroirs repartent tels quels vers V2d-4, et
`clore` ne lit même pas les champs qu'elle rend (D3).

Le poste le plus lourd est les tests (140 pour 104 de production), et c'est
conforme : le ticket exige un test par fait de socket et un test par porte sur le
corps illisible, et « les tests ne se reportent jamais à un vol suivant, même si
le code reste ». Les commentaires qui portent le pourquoi comptent dans la borne
et **ne se rabotent pas pour la tenir** — si le code écrit dépasse 540, le pilote
s'arrête selon la halte du § 3 et le dit, il ne retire pas après coup les
commentaires qui portent un pourquoi.

Pas de découpage en vols supplémentaires : la suite est déjà découpée par le
corps du ticket en V2d-4, V3 et V4.

# fix(3125) — `mika notify` vise un port que rien n'écoute, et rend 0 quand la livraison échoue

**Vol unique.** Type : fix. Branche : `fix/3125/cli-mika-notify-vise-le-port-3001-par-d`.
Ticket : senara-solutions/mika#3125.

Le ticket nomme deux défauts qui partagent un appelant, un fichier et un effet :
une escalade émise sur un canal que personne ne sert, et dont personne ne peut
savoir qu'il n'a rien servi. Ils se corrigent ensemble parce que **chacun seul
laisse l'autre produire le même silence** : aligner le port sans remonter
l'échec rend le canal réparé mais invérifiable ; remonter l'échec sans aligner
le port transforme le silence en un non-zéro systématique sur tout poste par
défaut. Le périmètre complet tient sous la borne par vol (§ *Taille estimée*).

## Le défaut, tel qu'il se mesure

Lu à HEAD (`7e35c5a6`) :

```rust
// crates/mika-cli/src/commands/notify.rs:93-96
/// Resolve the gateway base URL from env or default (same pattern as webhook.rs).
fn gateway_url() -> String {
    std::env::var("MIKA_GATEWAY_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}
```

```rust
// crates/mika-cli/src/commands/webhook.rs:14-24
fn gateway_url() -> String {
    if let Ok(url) = std::env::var("MIKA_GATEWAY_URL") { return url; }
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    std::env::var("MIKA_GATEWAY_URL").unwrap_or_else(|_| "http://localhost:3001".to_string())
}
```

Face à quoi le serveur écoute :

```rust
// crates/mika-gateway/src/settings.rs:54-56, 184-186
/// Listen port (default: 8080)
#[serde(default = "default_port")]
pub gateway_port: u16,
...
fn default_port() -> u16 { 8080 }
```

Trois faits, et chacun décide une part du livrable.

**1. Le défaut du client ne dérive de rien.** `3001` est un littéral, écrit deux
fois, qui ne correspond à aucun défaut de serveur de ce dépôt — ni au `8080` du
gateway, ni au `8080` de `MIKA_SPIRIT_PORT` lu par `dashboard::spirit_url()`.
Mesure du parking (#2983, commentaire 61, le 2026-11-21 à 10:42Z sur `ebf7e6e0`) :
sans `MIKA_GATEWAY_URL`, la livraison échoue avec *« gateway unreachable at
http://localhost:3001 »* ; avec `MIKA_GATEWAY_URL=http://localhost:8080`, le même
envoi passe en 200.

**2. L'échec de livraison ne sort pas.** `notify.rs:73-90` : le `Err` de
`send_via_gateway` est attrapé, écrit en `eprintln!`, puis `run` rend `Ok(())`.
Seul un échec d'écriture en base rend non-zéro. Un appelant qui demande
`--channel telegram` et obtient 0 ne peut pas distinguer *livré* de *perdu*.

**3. Le site d'appel l'a documenté plutôt que corrigé, et le dit.**
`skills/bundled/_shared/dispatch-lib.sh:1800-1810` porte un bloc
« WHAT THIS CANNOT TELL YOU » qui énonce exactement le point 2, et conclut qu'*« aucune
quantité de shell ici n'y changerait rien »*. Le plan de mika#2049 (§ 3.3.1) avait
**écarté** le correctif — voir § *La décision de mika#2049* ci-dessous. Ce bloc de
commentaire devient faux dès que le point 2 est corrigé : le mettre à jour est un
livrable de ce vol, pas une politesse.

### Ce que le défaut a coûté, et pourquoi il est resté invisible

La notification est écrite en base **avant** la tentative d'envoi
(`notify.rs:60` avant `:73`), donc une livraison morte laisse quand même la ligne
dans la session `00000000-0000-0000-0000-700000710717`. C'est ce qui a permis au
canal de paraître vivant : la trace existe, et c'est elle qu'on consulte. Le seul
appelant du dépôt (`_pilot_egress_notify`, en `|| true` et `>/dev/null 2>&1`)
jette aussi le `eprintln!`. Les deux surfaces où l'échec se disait étaient donc
fermées : le code de sortie, qui mentait, et le stderr, que l'appelant redirige.

### La décision de mika#2049, et pourquoi elle s'inverse ici

`docs/plans/2026-09-20-003-fix-2049-egress-fail-closed-plan.md` § 3.3.1 écarte
explicitement le fail-hard, sur deux motifs :

> **Alternative écartée** : faire remonter l'échec de livraison en rendant `mika notify`
> non-zéro sur échec Telegram. C'est un changement de contrat d'une commande partagée,
> dont tous les autres appelants attendent le fail-soft actuel — et ça ne servirait à rien
> ici, puisque le refus ne doit de toute façon pas dépendre de l'alerte.

Les deux motifs se mesurent, et aucun ne tient à ce stade :

- **« tous les autres appelants »** — `grep -rn "mika notify" scripts/ skills/ .github/`
  rend **deux lignes, le même site** (`_pilot_egress_notify` et son commentaire).
  Aucun appelant du dépôt ne **lit** le code de sortie : celui-là le neutralise
  par `|| true`, et l'escalade de `permission-policy` part en `subprocess.Popen`
  fire-and-forget côté claude-pilot. Le fail-soft n'est donc attendu par aucun
  consommateur mécanique, et le fail-hard n'en casse aucun — le `|| true` reste
  juste, pour la raison que mika#2049 donne (le refus ne doit pas dépendre de
  l'alerte) et qui ne change pas.
- **« ça ne servirait à rien ici »** — vrai de ce site-là, et c'est pourquoi le
  `|| true` survit. Mais la §3.3.1 reportait la vérification sur « une
  précondition de déploiement, pas une sonde sur le chemin critique », et le
  parking de 2026-11-21 mesure le résultat : la précondition n'a jamais été
  vérifiée, et le canal visait un port mort pendant tout l'intervalle. Un contrat
  qui rend 0 sur un échec rend cette vérification **impossible à automatiser par
  quiconque**, pas seulement inutile à ce site.

Le vol inverse donc la décision, pour le second motif et pas pour le premier : ce
n'est pas que le site d'appel veuille le signal, c'est qu'aucun site futur ne peut
l'obtenir tant que le contrat ment. Le § 3.3.1 est cité, pas contourné : la mise à
jour du bloc de `dispatch-lib.sh` est ce qui empêche la doctrine écrite là-bas de
survivre au fait qu'elle décrit.

## Requirements

- **R1.** Le défaut d'adresse du gateway côté CLI dérive du défaut de port que le
  gateway lui-même applique. Un déplacement de l'un sans l'autre doit être
  inexprimable, pas surveillé.
- **R2.** `MIKA_GATEWAY_URL` reste prioritaire et son sens ne change pas.
  `MIKA_GATEWAY_PORT` — la variable que **le gateway** lit — devient lisible par
  le client, comme `dashboard::spirit_url()` lit `MIKA_SPIRIT_PORT`.
- **R3.** Un seul site du dépôt résout l'adresse du gateway. Les deux copies de
  `gateway_url()` disparaissent ; une troisième est refusée structurellement.
- **R4.** `mika notify --channel telegram` sort **non-zéro** quand la livraison
  échoue, et le message dit que la notification **est** en base, en nommant la
  session. Le fait acquis se dit avant le geste qui peut échouer.
- **R5.** `mika notify` sans `--channel telegram` (le défaut `cli`) garde son
  comportement et son code de sortie, à l'octet.
- **R6.** Le bloc de commentaire de `_pilot_egress_notify` cesse d'affirmer un
  fait que ce vol rend faux. Le `|| true` et le `>/dev/null 2>&1` ne bougent pas.
- **R7.** Les surfaces documentaires qui publient `3001` comme défaut sont
  corrigées, le manifeste d'egress compris.

## Design

### `mika_common::gateway_endpoint` — le seul site qui connaît l'adresse

Module neuf, `crates/mika-common/src/gateway_endpoint.rs`, déclaré dans `lib.rs`.
Il est dans `mika-common` et non dans `mika-gateway` parce que `mika-cli` ne
dépend pas de `mika-gateway` et ne doit pas commencer à en dépendre pour lire un
`u16` ; les deux crates dépendent déjà de `mika-common`.

```rust
/// Port d'écoute par défaut du gateway — le seul site de vérité du dépôt.
///
/// `crates/mika-gateway/src/settings.rs::default_port` le rend, et c'est la
/// valeur que `GatewaySettings` applique quand `MIKA_GATEWAY_PORT` est absent.
/// Le client le lit pour composer son adresse par défaut : avant mika#3125 il
/// écrivait `3001`, qui n'était le défaut de rien.
pub const DEFAULT_GATEWAY_PORT: u16 = 8080;

/// Le geste de configuration, nommé une fois pour tous les messages d'erreur.
///
/// Il vit ici parce que c'est ici que les deux variables sont lues : un message
/// composé ailleurs serait une seconde déclaration de la même configuration,
/// qui divergerait au premier renommage.
pub const CONFIG_HINT: &str =
    "configurable par MIKA_GATEWAY_URL, ou par MIKA_GATEWAY_PORT que le gateway lit lui-même";

/// Adresse de base du gateway, lue dans l'environnement du process.
///
/// Charge le `.env` de la home avant de relire — geste repris de l'ancien
/// `webhook.rs::gateway_url` et conservé bien que `main.rs` le fasse déjà
/// avant le dispatch : `load_dotenv` est idempotent (dotenvy n'écrase pas), et
/// la garder rend la fonction correcte pour un appelant futur qui l'atteindrait
/// par un chemin court, à la façon de `plan-callout` (mika#2194).
pub fn gateway_base_url() -> String;

/// La même résolution, sur un lecteur d'environnement donné.
///
/// Ordre : `MIKA_GATEWAY_URL` → `MIKA_GATEWAY_PORT` → `DEFAULT_GATEWAY_PORT`.
/// Une valeur vide est traitée comme absente, et un `MIKA_GATEWAY_PORT` illisible
/// retombe sur le défaut en nommant la cause par un `warn!` — un port qui ne
/// parse pas n'est pas un zéro (motif mika#2331), et refuser l'envoi là-dessus
/// transformerait une faute de frappe cosmétique en perte d'escalade.
pub fn gateway_base_url_from(read: impl Fn(&str) -> Option<String>) -> String;
```

**Pourquoi `MIKA_GATEWAY_PORT` et pas seulement l'URL.** C'est la forme déjà
établie par `dashboard::spirit_url()` (`MIKA_SPIRIT_URL` puis `MIKA_SPIRIT_PORT`
puis `8080`), et c'est ce qui rend R1 réel : un opérateur qui déplace le port du
gateway pose **une** variable, et le client suit sans seconde déclaration.

**La limite, dite plutôt que découverte.** `DEFAULT_GATEWAY_PORT` et le défaut de
`MIKA_SPIRIT_PORT` valent tous deux `8080` : sur un hôte qui fait tourner
mika-spirit et le gateway, l'un des deux est nécessairement configuré, et c'est
alors sa variable qui tranche. Le défaut du client ne peut pas être meilleur que
le défaut du serveur ; ce vol le rend égal, il ne le rend pas devin.

### Le gateway rend la constante partagée

```rust
// crates/mika-gateway/src/settings.rs
fn default_port() -> u16 {
    mika_common::gateway_endpoint::DEFAULT_GATEWAY_PORT
}
```

Le test qui accompagne ce changement **n'assert pas `default_port() == DEFAULT_GATEWAY_PORT`** :
`default_port` rendant la constante, l'égalité serait vraie par construction et
verte par cécité (classe mika#2205). Il désérialise un `GatewaySettings` depuis
une charge figée **sans** `gateway_port` et assert que le champ vaut
`DEFAULT_GATEWAY_PORT` — ce qui atteste le câblage `#[serde(default = ...)]`,
seule chose qui peut casser.

### `webhook.rs` et `notify.rs` — deux appelants, une fonction

Les deux `fn gateway_url()` disparaissent. Chaque commande appelle
`mika_common::gateway_endpoint::gateway_base_url()`. Deux appelants d'une fonction
sont le régime nominal ; ce qui est refusé, c'est un troisième **résolveur**.

### `notify.rs` — l'ordre des faits, puis le refus

L'ordre actuel imprime le `✓ Notification saved` **après** le bloc Telegram. Il
s'inverse, parce que c'est l'ordre des faits : l'écriture en base a eu lieu avant
la tentative d'envoi, donc elle se dit avant — et surtout avant un `bail!` qui,
sinon, emporterait la confirmation d'un fait acquis. C'est la discipline des trois
sorties de mika#1727 vol V2d-4c : *le fait déjà acquis se dit avant le geste qui
peut échouer.*

La décision est extraite en fonction pure, pour être testable sans base ni
réseau :

```rust
/// Ce que la commande doit rendre, une fois la base écrite et la livraison tentée.
///
/// `None` ⇒ aucune livraison n'était demandée (canal `cli`) : R5, rien ne change.
/// `Some(Ok(()))` ⇒ livré.
/// `Some(Err(cause))` ⇒ la notification est en base et n'a pas été livrée.
fn delivery_verdict(attempt: Option<Result<(), String>>, url: &str) -> Result<()>;
```

Le message du `bail!` porte quatre choses, et aucune n'est décorative :

1. que la livraison sur le canal **demandé** a échoué, avec sa cause ;
2. l'adresse visée — le seul moyen de voir un port faux sans lire le code ;
3. `CONFIG_HINT`, qui nomme le geste ; `notify.rs` ne nomme lui-même aucune
   variable d'environnement (voir § *Fire-Disposition*) ;
4. que la notification **est** en base, avec l'identifiant de la session — ce qui
   sépare *« le gateway est mort »* de *« l'appel n'a pas eu lieu »*, la
   distinction que la halte (c) de mika#2049 § 5.3 rend décidable.

**Pas de code de sortie dédié, et c'est délibéré.** `anyhow` sort en 1, comme
l'échec d'écriture en base. Un code dédié distinguerait *« nulle part »* de *« en
base, pas livré »* — mais aucun appelant mesuré ne décide sur le code (§ *La
décision de mika#2049*), et inventer un code que personne ne lit reproduirait le
défaut qu'on corrige, une surface qui affirme sans destinataire. La distinction
est portée par le message, qui a un lecteur. Le jour où un appelant doit décider,
le code dédié arrive avec lui.

**`--channel cli` est intact.** Le bras n'appelle rien et ne traverse pas
`delivery_verdict` autrement qu'avec `None`.

### `dispatch-lib.sh` — le commentaire suit le fait

Le bloc « WHAT THIS CANNOT TELL YOU » (l. 1800-1810) est réécrit : `mika notify`
sort désormais non-zéro quand la livraison échoue, le `|| true` et le
`>/dev/null 2>&1` sont **conservés** pour la raison inchangée de mika#2049 — le
refus est la protection, l'alerte est l'information — et le commentaire dit
explicitement que ce site **choisit** de ne pas lire le signal, au lieu
d'affirmer qu'il n'existe pas. Aucune ligne de code shell ne change.

### Ce qui n'est pas touché

- `send_via_gateway` : son corps, ses préconditions (`chat_id` sur l'agent `mika`,
  `MIKA_INTERNAL_TOKEN` depuis `ctx.settings`) et ses messages d'erreur.
- Le gateway lui-même : routes, `/send`, découpage de texte (mika#3114).
- `dashboard::spirit_url()` et `MIKA_SPIRIT_PORT` — autre endpoint, autre défaut.
- Le `|| true` de `_pilot_egress_notify`, et la déduplication par stamp.
- Toute précondition de déploiement : ce vol rend l'échec lisible, il ne vérifie
  pas l'appairage Telegram.

## Verification Contract

Tous les tests sont unitaires, sans réseau, sans base, sans Postgres.

### Dans `crates/mika-common/src/gateway_endpoint.rs`

Sur `gateway_base_url_from`, avec un lecteur figé — aucune mutation de `std::env`,
donc aucune fragilité en exécution parallèle :

| cas | lecteur | attendu |
|---|---|---|
| URL posée | `MIKA_GATEWAY_URL=http://gw.test:9999` | rendue telle quelle |
| URL vide | `MIKA_GATEWAY_URL=""` | traitée comme absente → défaut |
| port posé | `MIKA_GATEWAY_PORT=8081` | `http://localhost:8081` |
| URL **et** port | les deux | l'URL gagne |
| port illisible | `MIKA_GATEWAY_PORT=plif` | `http://localhost:8080`, pas de panique |
| rien | lecteur vide | `http://localhost:8080` |

Plus une assertion de non-régression nommée : le défaut rendu **ne contient pas**
`3001`, par `assert!(!url.contains("3001"))` sur le cas vide. C'est l'assertion
qui rougirait si quelqu'un restaurait le littéral ailleurs dans la chaîne.

### Dans `crates/mika-gateway/src/settings.rs`

`mika3125_un_gateway_sans_port_declare_ecoute_sur_le_defaut_partage` — une charge
figée sans `gateway_port` désérialisée en `GatewaySettings`, assertant
`gateway_port == mika_common::gateway_endpoint::DEFAULT_GATEWAY_PORT`. Atteste le
câblage serde, pas l'égalité d'une constante avec elle-même.

### Dans `crates/mika-cli/src/commands/notify.rs`

**`mika3125_le_crate_cli_ne_resout_plus_ladresse_du_gateway`** — le détecteur.
`ProductionScanner::for_crate` sur la moitié production de `mika-cli`, aiguilles
`MIKA_GATEWAY_URL` et `MIKA_GATEWAY_PORT`, `ALLOWED` **livrée vide**. Un site
production du crate qui nomme l'une des deux variables est une seconde résolution
de l'adresse — la forme exacte du défaut corrigé. Les lignes de commentaire sont
sautées (forme de `mika2759_le_plancher_a_un_seul_site`), et c'est pourquoi
`CONFIG_HINT` vit dans `mika-common` : le message d'erreur du CLI ne nomme aucune
variable, il porte la constante.

**`mika3125_le_scan_a_bien_lu_la_production_et_masque_les_tests`** — témoin et
contre-témoin dans une assertion chacune : que le balayage a vu la production de
`commands/notify.rs` (un chemin faux serait vert par cécité), et qu'il a masqué
sa région `#[cfg(test)]` — sans quoi les aiguilles citées par les tests de ce
fichier feraient rougir la garde, et on l'aurait « réparée » par une entrée
d'allowlist. Forme de `mika1727_v2d4b` (`handlers.rs`), qui porte les deux dans
une garde.

**`delivery_verdict`**, quatre cas :

| entrée | attendu |
|---|---|
| `None` (canal `cli`) | `Ok(())` — R5 |
| `Some(Ok(()))` | `Ok(())` |
| `Some(Err(cause))` | `Err`, dont le message porte la cause, l'URL, `CONFIG_HINT` et `NOTIFICATIONS_SESSION_ID` |
| `Some(Err(_))` | contrôle négatif : le message **n'affirme pas** que la notification a été livrée |

Le dernier est un `assert!` négatif et non une relecture : un message qui laisse
croire à une livraison serait précisément l'état que le serveur — ici, le gateway
— ne nous a pas attesté (grounding rule du dépôt).

### Ce que ces tests ne couvrent pas, écrit dans les tests

Aucun n'atteste une livraison Telegram réelle : elle dépend d'un `chat_id`
appairé et d'un gateway vivant, états hôtes hors du harness. La limite est écrite
dans le module de test pour qu'un futur lecteur ne prenne pas le vert pour une
preuve de livraison — la discipline que mika#2049 § 3.3.1 demandait déjà pour son
assertion 3.

## Definition of Done

- [ ] `crates/mika-common/src/gateway_endpoint.rs` existe, est déclaré dans
      `lib.rs`, et porte `DEFAULT_GATEWAY_PORT`, `CONFIG_HINT`,
      `gateway_base_url` et `gateway_base_url_from`.
- [ ] `crates/mika-gateway/src/settings.rs::default_port` rend la constante
      partagée ; aucun littéral de port par défaut ne subsiste dans la moitié
      production du fichier.
- [ ] `grep -rn '3001' crates/` ne rend plus aucune ligne de `crates/mika-cli/`.
- [ ] `grep -rn 'fn gateway_url' crates/mika-cli/src` ne rend rien.
- [ ] `mika notify --channel telegram` sort non-zéro quand la livraison échoue,
      avec un message portant la cause, l'URL, le geste et l'ID de session.
- [ ] `mika notify` sans `--channel telegram` est inchangé (code et sorties).
- [ ] Le bloc de commentaire de `_pilot_egress_notify` ne dit plus que l'échec
      est invisible ; le `|| true` et la redirection sont intacts.
- [ ] Docs à jour : `.env.example`, `docs/configuration.md`,
      `docs/operator/environment-variables.md`, les deux entrées
      `cli-gateway-webhook` et `cli-gateway-notify` de
      `docs/egress/egress-manifest.toml`, et `crates/mika-cli/CLAUDE.md`.
- [ ] `scripts/sync-agent-docs.sh` lancé (le job CI `docs-sync` compare
      `crates/mika-agent/docs/configuration.md`).
- [ ] `cargo test`, `cargo clippy --all-targets`, `cargo fmt --check`,
      `bash scripts/verify-egress-manifest.sh` et
      `bash scripts/verify-pipeline.sh origin/main` passent.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de ses trois constats et des requirements.

- **AC1.** Sans `MIKA_GATEWAY_URL` ni `MIKA_GATEWAY_PORT`, `mika notify` vise
  `http://localhost:8080` — le port que `GatewaySettings` applique par défaut —
  et non `http://localhost:3001`. Attesté par le cas « rien » et l'assertion de
  non-régression sur `3001`.
- **AC2.** Le défaut du client et celui du serveur sont la **même donnée** : un
  déplacement du port du gateway par édition de `default_port` est inexprimable
  sans toucher `DEFAULT_GATEWAY_PORT`. Attesté par le test de désérialisation
  côté gateway.
- **AC3.** `MIKA_GATEWAY_URL` garde la priorité et son sens ; `MIKA_GATEWAY_PORT`
  est pris en compte quand l'URL est absente. Attesté par la table à six cas.
- **AC4.** Le dépôt ne porte plus qu'un seul résolveur de l'adresse du gateway.
  Attesté structurellement par le détecteur à allowlist vide, doublé de son
  témoin et de son contre-témoin.
- **AC5.** `mika notify --channel telegram` dont la livraison échoue sort
  non-zéro, en disant que la notification est en base et en nommant sa session.
  Attesté par les quatre cas de `delivery_verdict`, contrôle négatif compris.
- **AC6.** `mika notify` au canal `cli` est inchangé — même sorties, même code.
  Attesté par le cas `None`.
- **AC7.** Aucune surface du dépôt n'annonce plus `3001` comme défaut, et le
  commentaire de `_pilot_egress_notify` n'affirme plus que l'échec de livraison
  est structurellement invisible.

## Fire-Disposition

Ce vol livre **un** détecteur :
`mika3125_le_crate_cli_ne_resout_plus_ladresse_du_gateway` — un scan de source
sur la moitié production de `mika-cli`, dont le chemin de succès est « aucun
second résolveur trouvé ».

**Option retenue : (a), exception nommée en allowlist — avec zéro exception.**

`ALLOWED` est livrée **vide**, et l'assertion auto-nettoyante est le message
d'échec lui-même, qui nomme le remède et interdit l'ajout :

```rust
const ALLOWED: &[&str] = &[];
...
assert!(
    violations.is_empty(),
    "mika#3125 : l'adresse du gateway se résout à un seul site \
     (mika_common::gateway_endpoint). Site(s) supplémentaire(s) :\n{}\n\
     Le remède est d'appeler `gateway_base_url()`, PAS d'ajouter une entrée à \
     ALLOWED — qui doit rester vide.",
    violations.join("\n")
);
```

**Le détecteur atterrit vert, et c'est mesuré, pas espéré.** Les deux seules
violations existantes sont `notify.rs:95` et `webhook.rs:23`, et les deux
disparaissent dans ce même vol — la correction précède l'armement dans l'ordre de
commit (§ *Ordre de commit*, commits 1 à 3 avant le détecteur). Il n'y a donc
aucune violation résiduelle à excepter, ce qui est la raison pour laquelle
l'allowlist est vide plutôt que peuplée : une entrée ici désignerait un site que
ce vol a justement pour objet de supprimer.

**L'assertion qui rougit quand l'exception devient stale** est doublée d'un
`assert!(ALLOWED.is_empty())` explicite, forme de
`mika2759_le_plancher_a_un_seul_site` : un éditeur futur qui peuple la liste pour
faire taire la garde rougit sur cette seconde assertion, qui nomme la doctrine
(mika#2201 : router l'appelant par le site existant, jamais lister).

L'option (b) — livrer désarmé — est écartée : un détecteur sous `#[ignore]` ne
refuse rien, et le vol n'a aucune violation à tolérer. L'option (c) — halte et
remontée — ne s'applique pas : rien n'exige de cadrage opérateur.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/gateway_endpoint.rs` — constantes, deux résolveurs, doc-comments | 55 |
| tests (`crates/mika-common/src/gateway_endpoint.rs`) — table à six cas + non-régression `3001` | 40 |
| `crates/mika-common/src/lib.rs` — déclaration du module | 1 |
| `crates/mika-gateway/src/settings.rs` — `default_port` + test de désérialisation | 16 |
| `crates/mika-cli/src/commands/webhook.rs` — retrait de `gateway_url`, appel de la commune | 6 |
| `crates/mika-cli/src/commands/notify.rs` — résolution, réordonnancement, `delivery_verdict`, message | 35 |
| tests (`crates/mika-cli/src/commands/notify.rs`) — détecteur, témoin + contre-témoin, quatre cas de verdict | 75 |
| `skills/bundled/_shared/dispatch-lib.sh` — bloc de commentaire réécrit | 14 |

Total estimé : 242 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 242 contre 279 : 37 lignes de marge, 13 %. Le
périmètre est borné par trois décisions explicites, chacune prise pour ne pas
dépasser et chacune justifiée au § *Design* plutôt que subie :

- pas de type de provenance (`UrlSource`/`Resolved`) pour l'adresse résolue — la
  remédiation est la même quelle que soit la source (poser `MIKA_GATEWAY_URL`),
  donc les populations ne demandent pas de gestes différents et la provenance
  n'est pas exigible au sens mika#2331 ; l'URL affichée plus `CONFIG_HINT`
  suffisent à trancher ;
- pas de code de sortie dédié (§ *Design*) ;
- pas de vérification d'appairage Telegram (§ *Ce qui n'est pas touché*) — c'est
  la précondition de déploiement de mika#2049 § 3.3.1, qui reste hors vol.

Le volume est dominé par les tests : 115 lignes portées par les deux lignes de
tableau qui les nomment, plus la moitié environ des 16 lignes du gateway (son
test de désérialisation) — dont une cinquantaine pour le seul détecteur et ses
deux témoins. Le code de production tient donc en une centaine de lignes de Rust
réparties sur cinq fichiers, plus 14 lignes de commentaire shell, sans refonte
d'aucun fichier.

Aucune suite de ce vol n'est renvoyée : les sept requirements sont tous livrés ici,
donc il n'y a pas de ligne `Tracked in:` à écrire.

## Ordre de commit

Quatre commits, et l'ordre est contraint : le détecteur ne peut pas précéder la
suppression des deux sites qu'il refuse, sinon il atterrit rouge et la seule
issue serait l'entrée d'allowlist que la Fire-Disposition interdit.

1. **`feat(gateway-endpoint): un seul site connaît l'adresse du gateway`** —
   le module, ses constantes, ses deux résolveurs, sa table de tests, la
   déclaration dans `lib.rs`. Compile et passe sans `#[allow(dead_code)]` :
   les items sont `pub` dans une bibliothèque, et ses tests sont ses premiers
   appelants.
2. **`fix(gateway): le défaut de port est celui que le client lit`** —
   `default_port` rend la constante partagée, plus le test de désérialisation.
3. **`fix(cli): notify et webhook résolvent l'adresse au même endroit`** —
   les deux `gateway_url()` disparaissent ; le détecteur et ses deux témoins
   atterrissent **dans ce commit**, verts, parce que c'est lui qui supprime les
   violations. C'est le plancher du vol : à ce point, AC1 à AC4 sont tenus.
4. **`fix(cli): une livraison Telegram manquée ne sort plus en 0`** —
   réordonnancement du `✓ saved`, `delivery_verdict` et ses quatre tests, le
   `bail!` et son message, le bloc de commentaire de `dispatch-lib.sh`, et les
   surfaces documentaires. AC5 à AC7.

Chaque commit laisse l'arbre vert sous `cargo test` et `cargo clippy
--all-targets` ; le vol est mergeable en entier et n'a pas de phase différée.

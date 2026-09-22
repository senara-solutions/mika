# mika#1964 — le substrat ne transite pas, et un garde le tient fermé

**Ticket :** senara-solutions/mika#1964
**Branche :** `chore/1964/agent-core-sweep-sibling-builtin`
**Lignée :** mika#1783 (fondateur), mika#1971 (le cut substrat de `web_search`),
mika#2118 (`run_gws`), mika#2407 (l'asymétrie sélecteur/clé), mika#2103 (la
doctrine du lint par propriété), mika#2201 (« on déclare, on n'allowliste pas »).

---

## 1. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable : **cinq mesures** changent la sévérité, le périmètre,
l'ordre des deux moitiés — et, pour la dernière, la viabilité même du garde.
Prises à HEAD `367be118`.

### M1 — la prémisse « latent » est fausse, et elle l'est pour la moitié du périmètre

Le ticket écrit : *« currently latent because `FAMILY_AGENT_SKILL_ALLOWLIST`
excludes them »*. Cette allowlist gate les **skills**, jamais les **builtin
tools**.

- `crates/mika-agent/src/tools/mod.rs:1058-1059` — `pr_merge_with_gate` et
  `resolve_issue_order` sont enregistrés par `default_tools()`, donc présents
  dans le tableau d'outils de **tout** agent, quel que soit son tier.
- Le seul filtre qui peut les retirer est `[tools].disabled` de l'`identity.toml`
  (`agent_loop/mod.rs:7799 apply_agent_tool_visibility`), et sa première ligne
  est `if disabled.is_empty() { return; }`.
- `FAMILY_IDENTITY` (`crates/mika-common/src/home.rs:734`) ne déclare **aucun**
  bloc `[tools]`.

**Conséquence : `pr_merge_with_gate:165` — « Set MIKA_GITHUB_TOKEN or configure a
GitHub App. » — est servi à un tenant famille aujourd'hui.** Pas latent. Le
ticket est un p1 de fuite, pas une hygiène.

### M2 — trois des quatre candidats nommés sont déjà traités, et le quatrième n'existe pas où le ticket le dit

| candidat du ticket | état mesuré |
|---|---|
| `run_gws` (OAuth) | **converti** par mika#2118 — `apply_gws_credential_state` (`builtin_handlers.rs:3951`) route `NeverConfigured` via `substrate_unavailable` + `dispatch_substrate_diagnostic`, avec deux registres persona |
| MCP dispatch | **hors population** — aucun `ToolOutput::error` porteur d'un token substrat sur ce chemin |
| `run_gh` @ `builtin_handlers.rs:2472` | **la ligne n'existe plus** ; `run_gh` est à `3116`, ses chemins token sont `3276-3291` et **n'émettent aucun message de configuration** — l'absence de token est silencieuse, `classify_gh_error` classe le refus GitHub sans nommer de variable |
| `pr_merge_with_gate` | **non converti**, et c'est la fuite que M1 rend atteignable |

Le sweep que le ticket demande a donc été fait aux trois quarts par ses tickets
frères, **sans lui**. Ce qui reste n'est pas ce qu'il liste.

### M3 — la fuite la plus grave est dans le handler de RÉFÉRENCE, sur le chemin que mika#1971 a créé

`builtin_handlers.rs:307` :

```rust
return ToolOutput::error(map_substrate_error(status.as_u16(), &err_body.error));
```

`map_substrate_error` (`:356`) porte, dans son propre doc-comment, la décision
inverse de la doctrine :

> *The mapping **intentionally** names the operator surface (gateway container,
> MIKA_BRAVE_API_KEY on the gateway) […]*

Deux de ses branches servent au LLM `MIKA_SEARCH_UPSTREAM`, `MIKA_BRAVE_API_KEY`,
`mika-gateway` et « Ask the operator to… » :

- `(404, "search_upstream_not_configured")` — le cas mika#2407, c'est-à-dire
  **le cas de panne réel mesuré le 2026-09-18 sur six tenants** ;
- `(502, "unauthorized")` — « rotate MIKA_BRAVE_API_KEY on mika-gateway ».

Et `web-search` **est** dans `FAMILY_AGENT_SKILL_ALLOWLIST`
(`home.rs:722-729`). Donc : le handler que mika#1783 a corrigé, sur le chemin
que mika#1971 lui a ajouté, refuit vers exactement la population que mika#1783
protégeait. La correction ne tient pas quand le chemin bouge — ce qui est
l'argument entier de la partie B.

### M4 — le test qui devait tenir cette invariance est un faux-vert, et son garde compagnon n'existe pas

Trois faits, dans le même fichier :

1. `web_search_family_tier_http_401_no_leak` (`:4744`) **n'appelle pas le
   handler**. Son commentaire le dit : *« it builds the identical
   `ToolOutput::substrate_unavailable` the handler now emits »*. Il construit à
   la main l'objet qu'il prétend vérifier. Depuis mika#1971 le handler ne parle
   plus à Brave et n'émet plus rien de tel ; le test est resté vert de part et
   d'autre du changement qui a rouvert le défaut.
2. Le garde structurel que ce même commentaire nomme —
   *« the `web_search_no_raw_401_operator_error` source-scan test below is the
   companion guard that ensures the handler actually calls this constructor (not
   a bare `ToolOutput::error`) »* — **n'existe nulle part dans l'arbre.**
   `grep -rn web_search_no_raw_401_operator_error crates/` rend une seule ligne :
   ce commentaire.
3. `test_web_search_maps_substrate_502_unauthorized` (`:5188`) pilote le vrai
   handler par wiremock et **asserte la fuite comme comportement attendu** :
   `assert!(output.content.contains("rotate MIKA_BRAVE_API_KEY"))`.

**C'est M4, et non M1, qui décide de l'ordre du travail.** Un test comportemental
écrit à la main ne voit pas cette classe : la régression ne rend aucune décision
fausse, elle déplace le littéral d'une fonction à l'autre pendant que toutes les
assertions restent vertes. La partie B du ticket n'est pas de l'hygiène qui suit
la partie A — c'est le seul livrable qui empêche la partie A de se défaire au
prochain déplacement de chemin.

### M5 — la troncature évidente rend le garde inerte sur 87 % de sa cible, et l'arbre porte déjà la réfutation écrite

Un garde qui scanne ce périmètre **doit** écarter les régions de test : les tests
assertent précisément sur ces tokens (`FORBIDDEN_FAMILY_TIER_TOKENS`,
`builtin_handlers.rs:4585-4586`, contient littéralement `MIKA_BRAVE_API_KEY` et
`config.toml`). C'est la différence avec le garde modèle, qui scanne `crates/`
entier parce qu'une panique UTF-8 est un défaut dans un test aussi.

L'écriture évidente de cette troncature — couper à la première occurrence de
`#[cfg(test)]` — **est fausse sur le fichier principal du périmètre**, et
d'une manière qui ne se voit pas :

- `builtin_handlers.rs:601-603` porte une paire `#[cfg(not(test))]` /
  `#[cfg(test)]` sur `PROGRESS_TICKER_INTERVAL` ;
- le module de test réel commence à `:4493`.

Couper à la première occurrence laisse donc **3 891 lignes de code de production
hors scan** — `run_gh`, `run_gws`, les six `cmd.env("GH_TOKEN", …)`, le
diagnostic GWS — pendant que le garde sort `0` et se lit comme « périmètre
propre ». Seul `map_substrate_error` (`:356`) resterait couvert, par accident de
position.

**Le dépôt a déjà rencontré cette trappe et l'a écrite.**
`mika2118_probe_runs_only_on_auth_error` (`:6643`) porte, mot pour mot :

> *Split on the test MODULE, not on the first `#[cfg(test)]`: this file carries a
> `#[cfg(test)]` / `#[cfg(not(test))]` pair on `PROGRESS_TICKER_INTERVAL` around
> line 590, so the naive split truncates production at that point and the scan
> reads an empty set — **green for the wrong reason**.*

Et il porte le remède complet, à reprendre tel quel : le split sur
`"\n#[cfg(test)]\nmod tests {"`, **plus une assertion de bonne foi** sur la
tranche obtenue (`production.contains("async fn run_gws(")`) — sans quoi un
déplacement futur du marqueur rendrait le scan vide en silence, ce qui est le
défaut d'un cran plus haut.

Cette mesure est la raison pour laquelle §3.4 impose un **contrôle négatif de
troncature** : un littéral fuyant posé *après* la ligne 602 doit faire rougir le
garde. C'est le seul cas du harnais dont l'échec signifie « le garde ne regarde
pas où il croit regarder » plutôt que « une règle est trop étroite ».

### Ces mesures réfutent le corps du ticket ; elles ne le rectifient pas

M1 réfute la prémisse *« currently latent »*, M2 réfute la liste des quatre
candidats, et cette section conclut à un p1 de fuite quand le titre du ticket
porte encore « hygiene ». **Le plan ne ratifie pas cette divergence de son
propre chef** : la spec est un contrat versionné, et c'est l'opérateur qui la
rectifie — pas le plan qui l'absorbe en silence. La rectification est donc
prescrite en **§9**, texte prêt à poser, avec la part que ce document ne peut
pas exécuter lui-même nommée au même endroit.

---

## 2. Inventaire de la population (partie A)

Périmètre déclaré : `crates/mika-agent/src/skills/builtin_handlers.rs` et
`crates/mika-agent/src/tools/*.rs`, **code de production seulement** — troncature
sur le **module** de test, jamais sur la première occurrence de `#[cfg(test)]`
(M5 ; l'écriture évidente rend le scan vide sur ce fichier précis).

### 2.1 — Violations à convertir

Neuf sites, **et trois d'entre eux ne sont pas dans la liste du ticket** : les
candidats qu'il nomme sont aux trois quarts déjà traités (M2), pendant que la
population réelle contient des sites qu'il ne mentionne pas.

| site | ce que le LLM lit aujourd'hui | atteignable famille ? |
|---|---|---|
| `builtin_handlers.rs:307` + `map_substrate_error` `(404, …)` | « Ask the operator to set `MIKA_SEARCH_UPSTREAM` on mika-gateway (and the matching upstream key, e.g. `MIKA_BRAVE_API_KEY`…) » | **oui** — skill `web-search` |
| idem, `(502, "unauthorized")` | « Ask the operator to rotate `MIKA_BRAVE_API_KEY` on mika-gateway. » | **oui** |
| `pr_merge_with_gate.rs:164-167` | « GitHub token required for `pr_merge_with_gate`. Set `MIKA_GITHUB_TOKEN` or configure a GitHub App. » | **oui** — builtin tool, cf. M1 |
| `pr_merge_with_gate.rs:1572 classify_credential_scope_error` | « install the mika GitHub App on `{repo}` with Contents + Pull requests write permission, or grant the configured PAT the `repo` scope » | **oui** — sérialisé dans le JSON `MergeGateResult` servi en `content` |
| **`pr_merge_with_gate.rs:1303-1311`** — branche `BehindMainRemediation::Failed \| Contradiction` | « the fix is to install the mika GitHub App on it with Contents + Pull requests write permission, or to grant the configured PAT the `repo` scope » | **oui** — **absent de l'inventaire du ticket ; même texte que `:1572`, autre fonction** |
| **`send_message.rs:186-187`** | « No outbound sender configured — message was NOT delivered. **To enable Telegram delivery, set `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`.** » | **oui** — builtin, cf. §2.4 pour la contrainte mika#2136 |
| **`builtin_handlers.rs:2812-2816`** — garde d'action destructive (mika#1646) | « If tool-call persistence is disabled (`MIKA_STORE_TOOL_CALLS=false`), this gate cannot observe your read… Surface to the operator rather than retrying. » | **oui** — chaîne `extra` concaténée au refus servi au modèle |
| `resolve_issue_order.rs:219` | `"warning": "No GitHub token configured — returning issues in input order…"` | **oui** — builtin tool |
| `check_task.rs:273` / `:294` | « GitHub PR status: not available (no token configured) » | **oui** — `check_task` est un builtin |

**Trois constructeurs distincts dans cette seule table** — `ToolOutput::error`,
`ToolOutput::success` (`resolve_issue_order`) et `ToolOutput::delivery`
(`send_message`). C'est la confirmation empirique de la décision de §3.3 : une
règle ancrée sur `ToolOutput::error(` — la lettre de l'AC — ne verrait que la
moitié de sa propre population.

### 2.2 — Déjà conformes, à ne pas toucher

`web_search` branches `gateway_url`/`internal_token` (`:224`, `:237`),
`fetch_url` (`:446`, `:460`), `run_gws` `NeverConfigured` (`:3975`). Les trois
passent par `substrate_unavailable` + `dispatch_substrate_diagnostic`.

### 2.3 — Hors population, et il faut le dire plutôt que le découvrir

Des littéraux portent un token substrat **sans jamais atteindre le LLM** :
`cmd.env("GH_TOKEN", token)` (`:1817`, `:2554`, `:3277`,
`pr_merge_with_gate.rs:1359`), la liste de scrub `:3283`, `GWS_BLOCKED_FLAGS`
(`:3402`), les clés de `set_config`, le `tracing::warn!` du garde de flag de
revue (`:3025`, qui nomme `MIKA_STORE_TOOL_CALLS` **au journal**, pas au modèle
— à distinguer de son voisin `:2813`, qui est servi et figure donc en §2.1), et
— cas central — le **second argument** de `substrate_unavailable`, dont nommer la
surface opérateur est la raison d'être (`GWS_CREDENTIALS_ABSENT_DIAGNOSTIC:3929`
nomme `XDG_CONFIG_HOME`, correctement ; idem les quatre diagnostics déjà
conformes de §2.2, `:227`, `:240`, `:449`, `:463`).

**Un lint qui accuserait le diagnostic accuserait le mécanisme qu'il promeut.**
Ce cas décide la forme de l'annotation en §3.3.

**Volume mesuré, plutôt que prescrit.** Le motif de la règle 2 appliqué au
périmètre de production, **lignes de commentaire exclues** (§3.3), rend
**≈ 21 sites** : 9 conversions (§2.1) et ≈ 12 annotations. Sous la halte de §4,
mais de peu — et uniquement grâce à l'exclusion des commentaires, sans laquelle
le chiffre double. La mesure est à refaire au premier run du garde ; c'est son
résultat, pas celui-ci, qui décide.

### 2.4 — Un jugement à poser, et un site qui n'en est plus un

- **`send_message.rs:186-187` n'est pas un jugement ouvert : c'est une fuite**,
  et elle passe en §2.1. La lecture qui la classait « hors population » ne portait
  que sur la première phrase ; la seconde dit « To enable Telegram delivery, set
  `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` » — deux variables et une
  instruction opérateur.
  **Contrainte de conversion, et elle est stricte.** Le `content` doit conserver
  le fait de non-livraison : le tour en a besoin pour ne pas prétendre avoir
  livré (mika#2136), et `DeliveryOutcome::NoSender` ainsi que le `cleaned` capturé
  ne bougent pas — le `DeliveryVerdict` compare des textes par égalité. Seule la
  seconde phrase part au diagnostic. C'est le seul site du périmètre où le repli
  neutre a une obligation **positive** de contenu.
- `get_documentation` (`builtin_handlers.rs:166`) « Run the `mika` CLI once to
  generate it. » : instruction opérateur **sans aucun token**, donc le motif de
  la règle 2 ne l'accuse pas et aucune annotation n'a de prise sur lui. Le
  trancher est un jugement de doctrine sans effet sur le garde. Position par
  défaut : **hors population**. Trancher l'inverse est admissible ; le trancher
  **en silence** ne l'est pas.

---

## 3. Conception

### 3.1 — U1 : supprimer le footgun au lieu de le détecter

Le reviewer F2 demande un lint sur `substrate_unavailable(` non suivi de
`dispatch_substrate_diagnostic(`. **Un appel couplé qu'on peut écrire découplé
est une classe ouverte ; un appel qu'on ne peut écrire que couplé n'en est pas
une.**

Livrer dans `crates/mika-agent/src/tools/mod.rs` :

```rust
pub async fn dispatch_substrate_unavailable(
    user_facing_fallback: impl Into<String>,
    diagnostic: impl Into<String>,
    tool_name: &str,
    ctx: &ToolContext<'_>,
) -> ToolOutput
```

— construit puis route, en une expression. Les cinq sites conformes de §2.2 y
migrent ; `ToolOutput::substrate_unavailable` reste `pub` (les tests le
construisent) mais devient **SOLE WRITER** : un seul site de production
l'appelle, celui du helper.

`apply_gws_credential_state` (`:3975`) interpose un `tracing::info!` entre les
deux appels ; le `info!` passe **avant** le helper, le comportement est
inchangé.

Le lint de couplage du ticket devient alors une règle de site unique — plus
simple, plus forte, et dans l'idiome maison (`grooming_marker` mika#2158,
`sink_dir` mika#2267, `auto_pull_stop` mika#2329).

### 3.2 — U2 : conversion des neuf sites de §2.1

Forme, pour chacun : le `content` devient un repli neutre — *aucun nom de
service, aucune variable, aucun chemin, aucune URL, aucune instruction
opérateur* — et le texte actuel devient le diagnostic, **intégralement** : il ne
perd rien, il change de canal.

**Une exception à cette forme, et une seule** : `send_message` (§2.4), dont le
repli a une obligation *positive* — conserver le fait de non-livraison. Partout
ailleurs le repli dit que la capacité est indisponible et rien de plus.

Quatre points de conception :

- **`map_substrate_error` rend une `String` et n'a pas de `ctx`.** Sa signature
  devient `fn substrate_error_message(status, label) -> (String, String)`
  — *(repli neutre, diagnostic)* — et l'appelant `:307` fait le
  `dispatch_substrate_unavailable`. Les branches déjà neutres
  (`upstream_error`, `transport_error`, `parse_error`, le `_` par défaut)
  gardent leur texte **des deux côtés** : rien à cacher, rien à changer.
- **`pr_merge_with_gate` rend un JSON tagué.** `GateErrorKind::CredentialScope`
  reste, son `detail` devient le repli neutre, et l'actuel part au diagnostic.
  La variante et son nom de fil ne bougent pas : mika#1616 les a posés pour que
  le modèle puisse brancher, et rien ici n'est un changement de taxonomie.
- **`pr_merge_with_gate:1303-1311` n'a pas de `ctx` non plus** — c'est une
  fonction de remédiation qui compose une `String` rendue plus haut. Même
  traitement que `map_substrate_error` : elle rend le couple, l'appelant route.
  Son texte est le jumeau de celui de `:1572` et le rester est souhaitable ;
  les deux gagnent à partager une constante nommée (§3.3 décision 3).
- **`builtin_handlers.rs:2812-2816` est un garde, pas un handler de substrat.**
  Son `extra` explique au modèle pourquoi un refus est irréparable, et la
  variable n'est là que pour lui dire « remonte à l'opérateur ». Le repli neutre
  doit conserver cette conduite — *ce refus ne peut pas être levé en réessayant,
  remonte-le* — et céder au diagnostic la seule cause technique. Ne pas le
  convertir en « capacité indisponible » : il n'y a pas de capacité absente, il
  y a une garde qui ne peut pas conclure.

Registre du repli : **opérateur par défaut** (`Deployment`/`PersonaProfile` ne
sont pas des entrées ici). Deux registres à la mika#2290 seraient une extension
de périmètre ; `run_gws` en a un parce que mika#2118 l'a mesuré, pas par règle.

### 3.3 — U3 : le garde (`scripts/check-substrate-leak.sh`)

Modèle explicite : `scripts/check-byte-slices.sh`, **y compris sa doctrine
écrite** — *« WHEN YOU EXTEND THIS SCRIPT, EXTEND IT BY PROPERTY »*. Argument
optionnel `SCAN_ROOT` pour le harnais anti-vacuité.

**Règle 1 — site unique du constructeur nu.** `ToolOutput::substrate_unavailable(`
n'apparaît, hors région de test, qu'au site de `dispatch_substrate_unavailable`.
Allowlist **livrée vide** : quand elle tire, on retire le second site, on ne
l'exempte pas.

**Règle 2 — aucun littéral substrat en production dans le périmètre.**
Motifs : `MIKA_[A-Z_]{2,}`, `GH_TOKEN`, `GITHUB_TOKEN`, `config.toml`,
`.mika/`, `XDG_CONFIG_HOME`, plus les formes d'instruction (`Set MIKA_`,
`Ask the operator`, `configure a GitHub App`, `rotate `, `Ensure MIKA_`).

**Elle porte sur le LITTÉRAL, jamais sur le constructeur — et c'est la décision
centrale du garde.** La règle que le ticket propose (« `ToolOutput::error(...)`
dont la chaîne contient… ») **ne voit pas le défaut mesuré en M3** : le littéral
fuyant vit dans `map_substrate_error`, une fonction séparée, et n'apparaît sur
aucune ligne portant `ToolOutput::error`. C'est mot pour mot la leçon mika#2103
— *un garde qui connaît une écriture du défaut laisse passer toutes les autres*
— appliquée avant l'incident plutôt qu'après. Corollaire assumé : la règle 2
ignore aussi `ToolOutput::success`, qui sert le LLM tout autant (le site
`resolve_issue_order:219` est un `success`), ce qu'une règle sur `error` seul
aurait raté.

**Deux annotations, deux populations, comptables séparément** (doctrine
mika#2156/#2184 : deux noms plutôt qu'un champ) :

- `// substrate-diagnostic: <raison>` — ce littéral **est** le canal opérateur ;
  il doit nommer la surface, `dispatch_substrate_diagnostic` le route.
- `// substrate-ok: <raison>` — ce littéral n'atteint jamais le `content` servi
  au LLM (argument d'`env()`, clé de config, nom de drapeau).

Un seul préfixe fusionnerait « c'est protégé par le mécanisme » et « ce n'est
pas dans le sujet », et rendrait la première population incomptable le jour où
elle mérite un audit.

**Trois décisions de mécanique que la doctrine seule ne donne pas.** Chacune est
la différence entre un garde qui mord et un garde décoratif.

1. **La troncature porte sur le module de test, jamais sur le premier
   `#[cfg(test)]`** (M5). Split sur `"\n#[cfg(test)]\nmod tests {"`, **plus une
   assertion de bonne foi** sur la tranche obtenue — le garde vérifie qu'elle
   contient encore un marqueur de production connu (`async fn run_gws(`) et
   **échoue bruyamment** si le marqueur a bougé. Un scan qui ne trouve plus sa
   cible doit rougir, jamais sortir vert sur l'ensemble vide. Reprise mot pour
   mot du motif déjà posé par `mika2118_probe_runs_only_on_auth_error`.

2. **Les lignes de commentaire sont exclues du scan** (premier caractère non
   blanc `//`, ce qui couvre `///` et `//!`). Sans cette exclusion le garde
   accuse des dizaines de doc-comments — citer une variable d'environnement dans
   la documentation est le style maison, et le `CLAUDE.md` en est fait. Coût
   nommé : **nul sur le sujet**, un commentaire n'atteignant jamais le `content`
   servi au LLM. C'est cette exclusion, et elle seule, qui maintient le volume
   d'annotations sous la halte de §4 (§2.3).

3. **L'annotation porte sur une fenêtre, pas sur une ligne — et c'est une
   contrainte du langage, pas un confort.** Les littéraux fautifs sont des
   `format!` multi-lignes dont les lignes de continuation se terminent par `\`
   (`pr_merge_with_gate.rs:1309`, `builtin_handlers.rs:367-369`) : **y écrire un
   `//` mettrait le commentaire à l'intérieur de la chaîne**. Une annotation
   exempte donc les lignes suivantes jusqu'à une borne courte (≈ 12), et cette
   borne est testée **dans les deux sens** par le harnais — une annotation qui
   couvrirait un littéral trente lignes plus bas serait un trou silencieux dans
   le garde. Le gabarit à privilégier reste celui de
   `GWS_CREDENTIALS_ABSENT_DIAGNOSTIC` : hisser le texte en constante nommée et
   annoter sa déclaration, ce qui rend la fenêtre courte par construction.

**Bornes, dites plutôt que découvertes.** Le garde couvre
`builtin_handlers.rs` + `tools/*.rs`. Il **ne couvre pas** `mika-gateway`,
`mika-cli`, ni les `skills/bundled/**` — trois périmètres où un nom de variable
dans une chaîne est nominal. Étendre demande de mesurer d'abord le volume
d'annotations ; ce n'est pas ce ticket.

### 3.4 — U4 : harnais anti-vacuité + job CI

`scripts/test-check-substrate-leak.sh`, sur le modèle de
`test-check-byte-slices.sh` : *« Delete the thing the test protects; confirm the
test goes red. »* Un cas par règle, plus **quatre contrôles négatifs
obligatoires** — ce sont eux le livrable, les cas positifs ne prouvant que la
syntaxe :

- **N1 — le littéral exact de M3** (`rotate MIKA_BRAVE_API_KEY on mika-gateway`)
  posé dans une fonction **séparée** du constructeur. S'il passe au vert, le
  garde ne couvre pas le défaut qui l'a fait naître, quelles que soient les
  autres règles.
- **N2 — troncature (M5).** Un littéral fuyant posé **après** une paire
  `#[cfg(not(test))]` / `#[cfg(test)]` placée en tête de fixture doit rougir.
  C'est le seul cas dont l'échec signifie « le garde ne regarde pas où il croit
  regarder » plutôt que « une règle est trop étroite ». Doublé de son miroir :
  un littéral posé **dans** `mod tests` ne doit **pas** rougir.
- **N3 — marqueur déplacé.** Une fixture sans `mod tests {` doit faire échouer le
  garde **bruyamment**, jamais sortir 0 sur l'ensemble vide.
- **N4 — portée de l'annotation.** Une annotation doit exempter la continuation
  de chaîne qui la suit, et **ne pas** exempter un littéral situé au-delà de la
  borne. Sans le second sens, la fenêtre est un trou qu'aucun test ne mesure.

Job `substrate-leak-lint` dans `.github/workflows/ci.yml`, calqué sur
`byte-slice-lint` (`ci.yml:177-187`) : deux étapes, le garde puis son harnais,
avec le commentaire maison — *« A guard nobody has watched go red is a
decoration »*.

### 3.5 — U5 : réparer le faux-vert, et le nommer

- `web_search_family_tier_http_401_no_leak` (`:4744`) : **supprimé**. Il décrit
  un chemin Brave-direct qui n'existe plus depuis mika#1971 et il ne pilote pas
  le handler. Le remplacer par un test wiremock qui pilote `web_search` sur 404
  et 502, tier famille, et asserte `FORBIDDEN_FAMILY_TIER_TOKENS` sur `content`
  **plus** la ligne `audit_events`.
- `test_web_search_maps_substrate_502_unauthorized` (`:5188`) : son assertion
  s'inverse — `rotate MIKA_BRAVE_API_KEY` doit être **absent** du `content` et
  **présent** dans le diagnostic d'`audit_events`. **C'est un changement de
  contrat servi au LLM, sur le tier opérateur aussi** : sur `Default`,
  `dispatch_substrate_diagnostic` replie le diagnostic dans le `content`, donc
  le texte opérateur reste lisible — mais séparé par une ligne blanche, après le
  repli neutre. Le test doit asserter cette forme, pas l'ancienne.
- Le commentaire de `:4740` qui nomme un garde inexistant : supprimé, la
  règle 1 de §3.3 prend sa place.

### 3.6 — U6 : `docs/skills.md:581-583`

*« Follow-up hygiene ticket sweeps every existing handler […] ; enforcement via
lint/CI is planned. Until then, code review is the gate on new handlers. »*
→ nommer `scripts/check-substrate-leak.sh`, le job `substrate-leak-lint`, les
deux annotations et leur sémantique, et les bornes de §3.3. Une promesse tenue
dont le périmètre n'est pas dit est une promesse qu'on relit mal.

---

## Fire-Disposition (§4)

> Le titre porte l'ancre `## Fire-Disposition` **en tête**, et son numéro de
> section entre parenthèses : le détecteur de `dispatch-lib` est
> `grep -qE '^## Fire-Disposition'` (`dispatch-lib.sh:5847`), que la forme
> numérotée `## 4. Fire-Disposition` ne matche pas — la section existait et le
> garde la lisait absente. Les renvois internes « §4 » restent valides.

Ce plan livre trois détecteurs : la règle 1 et la règle 2 de
`check-substrate-leak.sh`, et le harnais anti-vacuité.

**Option retenue : (a) — exception nommée en allowlist, et l'allowlist des
violations est livrée VIDE.**

Deux mécanismes distincts, et la distinction est la moitié qui compte :

1. **Les neuf violations de §2.1 sont converties par U2, pas allowlistées.**
   L'allowlist du garde ne reçoit aucune entrée. C'est la règle mika#2201 —
   *« on déclare, on n'allowliste pas »* : quand la règle 1 tire, la résolution
   est de retirer le second site, jamais de l'exempter.

2. **Les sites de §2.3 reçoivent une annotation inline**, `// substrate-ok:` ou
   `// substrate-diagnostic:`, chacune portant sa raison. **Ce ne sont pas des
   exceptions au sens de mika#1574 : ce sont des sites hors population**, et
   l'annotation est le mécanisme du garde, pas son contournement — exactement ce
   que `check-byte-slices.sh` fait de `Vec::truncate`, dont le doc dit :
   *« annotating them once is the price of a lint that reads the property rather
   than guessing string-ness from a variable name »*. Aucune n'est une dette,
   aucune ne porte de ticket de suivi, et une assertion auto-nettoyante n'aurait
   rien à nettoyer.

**Halte d'implémentation.** Le volume est **mesuré à ≈ 21 sites** (§2.3), dont
≈ 12 annotations — sous la borne, mais de peu, et seulement grâce à l'exclusion
des lignes de commentaire (§3.3 décision 2). Si le premier run réel dépasse
**25 sites**, ne pas annoter en masse : le motif est trop large, et un garde dont
le coût d'entrée est cinquante annotations est un garde qu'on désarme. Resserrer
le motif, re-mesurer, écrire la mesure dans le script. **Et si le premier run en
rend beaucoup moins que 21, ne pas s'en réjouir : vérifier d'abord la troncature
(M5), dont l'échec se présente exactement comme un périmètre propre.**

**Si une violation de §2.1 se révèle non convertible** (contrainte non vue à la
lecture), elle passe alors en (a) plein : entrée grep-visible nommant le site,
ticket de suivi ouvert, assertion auto-nettoyante qui rougit le jour où le site
disparaît. **Ne pas l'annoter en `// substrate-ok:` — ce serait déclarer hors
population ce qui est une fuite, c'est-à-dire fermer le garde sur son propre
sujet.**

---

## Acceptance criteria (§5)

> Même ancre que `## Fire-Disposition` ci-dessus, et pour la même raison
> mesurée : le gate U2 est `grep -qi '^## Acceptance criteria'`
> (`scripts/verify-pipeline.sh:147`), ancré en début de ligne, que la forme
> numérotée `## 5. Acceptance criteria` ne matche pas — vérifié par
> `grep -ci` rendant `0` sur la rev 3. La section existait et le garde la lisait
> absente ; le PR aurait échoué en CI sur mika#1600. Les renvois internes « §5 »
> (§1, §7 DoD) restent valides.

Transcrites du corps du ticket :

- [ ] Every builtin handler in `crates/mika-agent/src/skills/builtin_handlers.rs`
      and `crates/mika-agent/src/tools/*.rs` that returns substrate-config errors
      is converted to `substrate_unavailable`.
- [ ] Unit tests per handler mirror the `web_search_family_tier_*` shape
      (no_leak, audit_event, default_tier_diagnostic_visible).
- [ ] Structural gate script exists at `scripts/check-substrate-leak.sh` and:
  - [ ] Rejects `ToolOutput::error(...)` calls with substrate-token strings in
        the message
  - [ ] Rejects `substrate_unavailable(...)` calls without a same-function
        `dispatch_substrate_diagnostic(...)` sibling
- [ ] CI job `substrate-leak-lint` added to `.github/workflows/ci.yml`.
- [ ] `docs/skills.md` updated to reference the lint gate (currently promises
      "planned").

**Deux AC sont satisfaites par une forme plus forte que leur lettre, et la
divergence est déclarée ici plutôt que découverte en revue :**

- *« Rejects `ToolOutput::error(...)` calls with substrate-token strings »* — la
  règle 2 porte sur le **littéral**, pas sur le constructeur, ce qui couvre
  strictement plus : le défaut M3 (littéral dans une fonction séparée) et les
  porteurs qui ne sont pas des `error`. **Ce n'est pas une précaution théorique :
  l'inventaire de §2.1 compte trois constructeurs** — `error`, `success`
  (`resolve_issue_order:219`) et `delivery` (`send_message:186`). Une règle
  ancrée sur `ToolOutput::error(` raterait un tiers de sa propre population, et
  le défaut fondateur avec. Justification complète en §3.3.
- *« Rejects `substrate_unavailable(...)` calls without a same-function
  sibling »* — U1 rend le découplage inexprimable et la règle 1 tient le site
  unique. La classe est supprimée, pas détectée. Justification en §3.1.

---

## 6. Contrat de vérification

- **V1 — la fuite M3 est fermée sur le chemin réel.** Test wiremock pilotant
  `web_search`, tier `Family`, substrat répondant 404
  `search_upstream_not_configured` puis 502 `unauthorized` : `content` ne
  contient aucun de `FORBIDDEN_FAMILY_TIER_TOKENS`, et `audit_events` porte une
  ligne `tool_name = "substrate_unavailable"`, `target_key = "web_search"` dont
  l'`after_value` contient `MIKA_SEARCH_UPSTREAM` (404) / `MIKA_BRAVE_API_KEY`
  (502).
- **V2 — contrôle négatif opérateur.** Même scénario, tier `Default` : le
  diagnostic **est** dans le `content` (replié), et **aucune** ligne
  `audit_events` n'est écrite. Sans ce contrôle, « le garde décide » est
  indistinguable de « le garde bloque tout ».
- **V3 — `pr_merge_with_gate` sans token, tier `Family` :** `content` (JSON
  sérialisé compris) ne contient ni `MIKA_GITHUB_TOKEN` ni « GitHub App » ;
  `audit_events` porte la ligne. Idem pour `resolve_issue_order`, `check_task`,
  et pour `send_message` — dont le `content` doit **conserver** le fait de
  non-livraison tout en perdant les deux variables (§2.4).
- **V4 — le garde mord.** `bash scripts/test-check-substrate-leak.sh` passe, ses
  quatre contrôles négatifs N1–N4 compris (§3.4).
- **V5 — l'arbre balayé est propre, et le garde regarde où il croit regarder.**
  `bash scripts/check-substrate-leak.sh` sort 0 sur `crates/mika-agent/src`
  — **et** ce zéro est qualifié : l'assertion de bonne foi passe, et retirer une
  seule annotation d'un site de §2.3 fait rougir le garde. Un `0` non qualifié
  est exactement ce que produirait la troncature naïve de M5.
- **V6 —** `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`.

**Sonde post-déploiement, et sa halte.** Sur 7 jours :
`SELECT target_key, count(*) FROM audit_events WHERE tool_name =
'substrate_unavailable' GROUP BY 1;` — régime attendu **non vide** sur
`web_search` si le substrat est en panne quelque part (c'est la mesure que
mika#2407 réclamait et que la fuite rendait inutile), **vide** sinon.
**Halte :** si un tenant famille rapporte encore une phrase nommant une
variable d'environnement alors que cette table est vide, ne pas élargir le motif
du garde — le texte vient d'un autre chemin (gateway, prompt, un binaire
antérieur — classe mika#2340), et c'est le chemin qu'il faut établir d'abord.

---

## 7. Definition of Done

- U1 à U6 livrés ; les **neuf** sites de §2.1 convertis ; l'allowlist du garde
  vide.
- V1 à V6 verts ; le job `substrate-leak-lint` présent et passant en CI.
- La troncature du garde porte sur le module de test et **échoue bruyamment**
  quand son marqueur bouge (M5) — vérifié par N2 et N3.
- **N1 est la preuve de la première divergence de §5, pas un contrôle négatif
  parmi quatre.** Il pose le littéral exact de M3 — *« rotate
  `MIKA_BRAVE_API_KEY` on mika-gateway »* — dans une fonction **séparée** du
  constructeur, c'est-à-dire précisément l'écriture que la règle-constructeur de
  l'AC (« rejects `ToolOutput::error(...)` calls with substrate-token strings »)
  laisse passer. Sa rougeur **est** la justification de la règle-littéral ; s'il
  passe au vert, la divergence n'est pas démontrée et c'est la règle 2 qu'il faut
  reprendre avant tout le reste, jamais l'AC qu'il faut assouplir (doctrine
  mika#2103, §3.3, §3.4).
- `docs/skills.md` ne promet plus, il nomme.
- Le corps de PR déclare les deux divergences de §5 et le changement de contrat
  de §3.5 (le texte opérateur reste lisible sur tier `Default`, après le repli
  neutre et une ligne blanche).
- Les jugements de §2.4 tranchés **explicitement** dans le code, par annotation
  portant sa raison — jamais par omission.
- **Les deux rectifications de §9 sont posées sur le corps du ticket avant
  dispatch d'implémentation** : le callout de branche canonique (§9.1) et
  l'encadré de rectification daté (§9.2), ce dernier doublé de son commentaire
  d'avis d'édition. **Gestes d'opérateur / de dispatch-lib, hors du contrat de ce
  document** (§9.0) : leur absence ne bloque pas la revue du plan, elle bloque la
  ratification de la divergence — un ticket dont le corps affirme encore
  « latent » pendant que le plan implémente un p1 est un contrat que personne ne
  peut relire.

---

## 8. Risques et hors périmètre

- **Risque 1 — le volume d'annotations.** Borné par la halte de §4 (25 sites).
  La mesure préalable est prescrite, pas espérée.
- **Risque 2 — un repli neutre trop pauvre.** Un `content` qui ne dit rien
  produit un modèle qui invente (doctrine mika#1783 : le repli est neutre, pas
  muet). Chaque repli doit dire *que* la capacité est indisponible, sans dire
  *pourquoi* en termes d'infrastructure. Les replis existants de `web_search` et
  `fetch_url` sont le gabarit.
- **Risque 3 — inertie du garde.** La règle 2 est bornée à deux chemins ; un
  handler substrat créé ailleurs n'est pas couvert. Dit en §3.3, pas masqué.

**Hors périmètre, délibérément :**

- **Le bloc `[tools]` manquant de `FAMILY_IDENTITY` (M1).** Que
  `pr_merge_with_gate`, `create_agent`, `create_team`, `set_config` et
  `run_team` soient servis au tableau d'outils d'un tenant famille est un défaut
  réel, trouvé en chemin, **plus large que celui-ci** : ce n'est pas une fuite de
  texte mais une surface d'action. Le fermer demande de décider quels builtins un
  tier famille porte, ce qui est un choix produit. **Ticket de suivi**, et M1
  reste ici parce qu'il donne sa sévérité à celui-ci : sans lui, les fuites de
  §2.1 seraient latentes.
- `mika-gateway`, `mika-cli`, `skills/bundled/**` (§3.3).
- Les deux registres persona sur les replis convertis (§3.2).
- La *cause* des pannes substrat que ces messages décrivent.

---

## 9. Rectifications requises au corps du ticket

### 9.0 — Ce que ce document peut et ne peut pas faire ici

**Could not address: F1 (moitié GitHub) — Could not address: F2 (moitié
GitHub).** Les deux findings prescrivent une écriture **dans le corps du
ticket** : un callout de branche canonique (F1), un encadré de rectification
daté plus un commentaire d'avis d'édition (F2). Cette révision est produite sous
`/mika-revise-plan`, dont le contrat est **content-only** — un seul fichier
touché, le plan, et explicitement *« no `gh issue edit` »*. Dans le flux
autonome, l'écriture du corps appartient à `dispatch-lib` (callout de branche,
après convergence architecte) et à l'opérateur (rectification de spec). Le
pilote de révision ne peut donc pas poser ces deux textes lui-même, et le dire
est préférable à le simuler.

**Ce qui est adressé, et c'est la moitié qui compte pour F2 :** le plan cesse de
réfuter le corps en silence. §1 renvoie désormais explicitement ici, les deux
textes sont livrés prêts à poser plutôt que laissés à reconstruire, et le DoD
(§7) fait de leur pose une condition antérieure au dispatch d'implémentation.
La divergence est **déclarée et en attente de ratification par l'opérateur**,
jamais absorbée par le plan — ce que demande la convention
« issue-as-versioned-contract » invoquée par F2.

### 9.1 — F1 : callout de branche canonique

Le ticket porte le label `ready` (commentaire de garde 2026-09-22T08:20Z) et son
corps ne nomme aucune branche canonique, alors que ce callout est la surface
parsée pour la dérivation de branche (convention §1.5). Texte à poser, forme
canonique **sans préfixe de dépôt** — mika#2120 : le préfixe n'est pas
redondant mais faux, le chemin étant résolu sous la racine du sous-dépôt :

```
> [!NOTE]
> - **Branch:** `chore/1964/agent-core-sweep-sibling-builtin`
> - **Plan:** `docs/plans/2026-09-22-001-chore-1964-agent-core-sweep-sibling-builtin-plan.md`
```

### 9.2 — F2 : encadré de rectification daté

M1 et M2 réfutent deux affirmations factuelles du corps, et le titre porte
encore « hygiene » quand §1 conclut à un p1 de fuite servi aujourd'hui. Texte à
poser :

```
> [!IMPORTANT]
> **Rectification du 2026-09-22 (grooming mika#1964, plan rev 2).** Trois
> affirmations de ce corps sont réfutées par la lecture du code à HEAD
> `367be118`. Elles sont rectifiées ici plutôt que contournées par le plan.
>
> 1. **« currently latent because FAMILY_AGENT_SKILL_ALLOWLIST excludes them »
>    est faux.** Cette allowlist gate les *skills*, jamais les *builtin tools* :
>    `pr_merge_with_gate` et `resolve_issue_order` sont enregistrés par
>    `default_tools()` et présents dans le tableau d'outils de tout agent, quel
>    que soit son tier ; le seul filtre qui pourrait les retirer est
>    `[tools].disabled` de l'identité, et l'identité famille n'en déclare aucun.
>    Le message « Set MIKA_GITHUB_TOKEN or configure a GitHub App. » est **servi
>    à un tenant famille aujourd'hui**. Sévérité : p1 de fuite, non latente.
> 2. **La liste des quatre candidats est périmée.** `run_gws` est converti
>    (mika#2118) ; le dispatch MCP est hors population ; `run_gh` ne nomme plus
>    aucune variable et la ligne citée n'existe plus. Seul `pr_merge_with_gate`
>    subsiste, et la population réelle compte **neuf** sites — dont trois que ce
>    corps ne nomme pas : la seconde branche de remédiation de
>    `pr_merge_with_gate`, `send_message`, et la garde d'action destructive de
>    mika#1646. Inventaire mesuré : §2.1 du plan.
> 3. **Le titre « hygiene » est périmé** — voir 1. La fuite la plus grave est
>    dans le handler `web_search`, sur le chemin créé par mika#1971, et atteint
>    exactement la population que mika#1783 protégeait (§1, M3).
>
> Aucune AC n'est affaiblie. Deux d'entre elles sont satisfaites par une forme
> strictement plus forte que leur lettre ; la divergence est déclarée en §5 du
> plan et verrouillée par le contrôle négatif N1 (§7).
```

Commentaire d'avis d'édition à poster sur le ticket, pour que la modification du
contrat soit datée et attribuable plutôt que découverte dans un diff de corps :

```
Corps édité le 2026-09-22 (grooming mika#1964) : encadré de rectification ajouté
— la prémisse « latent » est réfutée (M1), la liste des candidats est remplacée
par l'inventaire mesuré à neuf sites (M2), et la sévérité passe de « hygiene » à
p1 de fuite. Aucune AC modifiée. Détail et mesures : §1 et §9.2 du plan.
```

---

## Revision history

- **rev 4 (2026-09-22)** — re-groom idempotent. Aucun finding en entrée ; le
  correctif vient d'avoir cherché la **même classe de défaut** que rev 3 sur les
  autres sections gardées du plan, plutôt que de tenir pour isolé un défaut dont
  la cause — la numérotation `## N. ` désancre un `grep -qE '^## Titre'` — n'a
  rien de spécifique à Fire-Disposition.
  - **Trouvée sur `## Acceptance criteria`, et celle-là est gardée par CI.** Le
    gate U2 (`scripts/verify-pipeline.sh:147`, mika#1600) est
    `grep -qi '^## Acceptance criteria'` ; le titre `## 5. Acceptance criteria`
    rendait `0` au `grep -ci`. La section était complète depuis rev 1 et le garde
    la lisait **absente** : le PR d'implémentation aurait échoué en CI sur une
    section présente, c'est-à-dire sur le diagnostic le moins lisible qui soit.
    Titre corrigé en `## Acceptance criteria (§5)`, sur le modèle exact de rev 3 ;
    les renvois internes « §5 » restent valides, aucun autre caractère ne bouge.
  - **Population balayée, pas échantillonnée** : les deux seuls prédicats ancrés
    qui s'appliquent à un plan sont relevés à HEAD — celui de `verify-pipeline.sh`
    (l'unique de ce fichier) et celui de `dispatch-lib.sh:5847`. Les deux
    matchent désormais, vérifié par `grep -c`. `## 7. Definition of Done` reste
    numérotée : **aucun garde ne la lit**, et la renommer par symétrie aurait posé
    une convention sans détecteur derrière elle.
- **rev 3 (2026-09-22)** — révision sur le finding synthétique `F-FD` émis par
  `dispatch-lib` (mika#2306), non par l'architecte.
  - **F-FD adressée, et la mesure déplace le finding.** La section réclamée
    n'était pas absente : elle existait depuis rev 1, sous le titre
    `## 4. Fire-Disposition`, portant déjà l'option (a) de mika#1574, son
    allowlist livrée vide, la distinction conversion-vs-annotation et la halte de
    volume. Ce qui manquait était l'**ancre** : le détecteur est
    `grep -qE '^## Fire-Disposition'` (`dispatch-lib.sh:5847`), ancré en début de
    ligne, que la numérotation `## 4. ` désancre. Le titre devient
    `## Fire-Disposition (§4)` — l'ancre matche, les huit renvois internes « §4 »
    (§2.3, §3.3 décision 2, §7, §8 risque 1) restent valides, et aucun autre
    caractère du plan ne bouge.
  - **Aucune disposition n'a été inventée** : la disposition posée est celle de
    rev 1, inchangée. Le plan livre bien trois détecteurs, donc le gate n'est pas
    N/A et la branche « dis-le explicitement » du finding ne s'applique pas.
- **rev 2 (2026-09-22)** — révision sur findings de première passe architecte
  (`mika-arch-groom-ticket`, `Disposition: ITERATE`).
  - **F3 adressée** : §4 disait « les six violations de §2.1 » quand §2.1, §3.2
    et §7 en comptent neuf ; le reste périmé est corrigé. Le volume de §2.3
    (≈ 21 sites = 9 conversions + ≈ 12 annotations) et la halte de §4 étaient
    déjà calés sur neuf et ne bougent pas.
  - **F4 adressée** : §7 lie désormais explicitement N1 à la première divergence
    de §5 — N1 n'est plus « un contrôle négatif parmi quatre » mais la preuve que
    la règle-littéral couvre le défaut M3 que la règle-constructeur de l'AC
    ratait, avec la conduite prescrite s'il passe au vert (reprendre la règle 2,
    jamais assouplir l'AC).
  - **F1 et F2 adressées pour leur moitié réalisable ; moitié GitHub non
    exécutable sous ce contrat** — voir §9.0, qui porte les deux lignes
    « Could not address » et leur raison. Le plan cesse de réfuter le corps en
    silence : §1 renvoie à §9, les textes de rectification sont livrés prêts à
    poser (§9.1 callout de branche, §9.2 encadré daté + commentaire d'avis
    d'édition), et §7 fait de leur pose une condition antérieure au dispatch
    d'implémentation. La pose elle-même revient à `dispatch-lib` et à
    l'opérateur ; c'est leur geste, pas celui du pilote de révision.

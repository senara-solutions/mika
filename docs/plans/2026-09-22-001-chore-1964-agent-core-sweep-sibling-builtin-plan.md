# mika#1964 — le substrat ne transite pas, et un garde le tient fermé

**Ticket :** senara-solutions/mika#1964
**Branche :** `chore/1964/agent-core-sweep-sibling-builtin`
**Lignée :** mika#1783 (fondateur), mika#1971 (le cut substrat de `web_search`),
mika#2118 (`run_gws`), mika#2407 (l'asymétrie sélecteur/clé), mika#2103 (la
doctrine du lint par propriété), mika#2201 (« on déclare, on n'allowliste pas »).

---

## 1. Ce que la lecture du code déplace dans le ticket

C'est le premier livrable : **quatre mesures** changent la sévérité, le périmètre
et l'ordre des deux moitiés. Prises à HEAD `2b5456cc`.

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

---

## 2. Inventaire de la population (partie A)

Périmètre déclaré : `crates/mika-agent/src/skills/builtin_handlers.rs` et
`crates/mika-agent/src/tools/*.rs`, **code de production seulement** (troncature
à la première occurrence de `#[cfg(test)]`, l'idiome du dépôt).

### 2.1 — Violations à convertir

| site | ce que le LLM lit aujourd'hui | atteignable famille ? |
|---|---|---|
| `builtin_handlers.rs:307` + `map_substrate_error` `(404, …)` | « Ask the operator to set `MIKA_SEARCH_UPSTREAM` on mika-gateway (and the matching upstream key, e.g. `MIKA_BRAVE_API_KEY`…) » | **oui** — skill `web-search` |
| idem, `(502, "unauthorized")` | « Ask the operator to rotate `MIKA_BRAVE_API_KEY` on mika-gateway. » | **oui** |
| `pr_merge_with_gate.rs:164-167` | « GitHub token required for `pr_merge_with_gate`. Set `MIKA_GITHUB_TOKEN` or configure a GitHub App. » | **oui** — builtin tool, cf. M1 |
| `pr_merge_with_gate.rs:1568 classify_credential_scope_error` | « install the mika GitHub App on `{repo}` with Contents + Pull requests write permission, or grant the configured PAT the `repo` scope » | **oui** — sérialisé dans le JSON `MergeGateResult` servi en `content` |
| `resolve_issue_order.rs:219` | `"warning": "No GitHub token configured — returning issues in input order…"` | **oui** — builtin tool |
| `check_task.rs:273` / `:294` | « GitHub PR status: not available (no token configured) » | **oui** — `check_task` est un builtin |

### 2.2 — Déjà conformes, à ne pas toucher

`web_search` branches `gateway_url`/`internal_token` (`:224`, `:237`),
`fetch_url` (`:446`, `:460`), `run_gws` `NeverConfigured` (`:3975`). Les trois
passent par `substrate_unavailable` + `dispatch_substrate_diagnostic`.

### 2.3 — Hors population, et il faut le dire plutôt que le découvrir

Des littéraux portent un token substrat **sans jamais atteindre le LLM** :
`cmd.env("GH_TOKEN", token)` (`:1817`, `:2554`, `:3277`, `pr_merge_with_gate.rs:1359`),
`GWS_BLOCKED_FLAGS` (`:3402`), les clés de `set_config`, et — cas central — le
**second argument** de `substrate_unavailable`, dont nommer la surface opérateur
est la raison d'être (`GWS_CREDENTIALS_ABSENT_DIAGNOSTIC:3926` nomme
`XDG_CONFIG_HOME`, correctement).

**Un lint qui accuserait le diagnostic accuserait le mécanisme qu'il promeut.**
Ce cas décide la forme de l'annotation en §3.3.

### 2.4 — Deux jugements à poser à l'implémentation, non tranchés ici

- `send_message.rs:186` « No outbound sender configured — message was NOT
  delivered. » : nomme un état du substrat mais **aucune variable, aucun chemin,
  aucune instruction opérateur**, et le tour a besoin du fait pour ne pas
  prétendre avoir livré (mika#2136). Position par défaut : **hors population**,
  annoté.
- `get_documentation` « Run the `mika` CLI once to generate it. » : instruction
  opérateur sans token. Position par défaut : **hors population**, annoté.
  Trancher l'inverse est admissible ; le trancher **en silence** ne l'est pas.

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

### 3.2 — U2 : conversion des six sites de §2.1

Forme, pour chacun : le `content` devient un repli neutre — *aucun nom de
service, aucune variable, aucun chemin, aucune URL, aucune instruction
opérateur* — et le texte actuel devient le diagnostic, **intégralement** : il ne
perd rien, il change de canal.

Deux points de conception :

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

**Bornes, dites plutôt que découvertes.** Le garde couvre
`builtin_handlers.rs` + `tools/*.rs`. Il **ne couvre pas** `mika-gateway`,
`mika-cli`, ni les `skills/bundled/**` — trois périmètres où un nom de variable
dans une chaîne est nominal. Étendre demande de mesurer d'abord le volume
d'annotations ; ce n'est pas ce ticket.

### 3.4 — U4 : harnais anti-vacuité + job CI

`scripts/test-check-substrate-leak.sh`, sur le modèle de
`test-check-byte-slices.sh` : *« Delete the thing the test protects; confirm the
test goes red. »* Un cas par règle, l'annotation dans les deux sens, et —
obligatoire — **un contrôle négatif portant le littéral exact de M3**
(`rotate MIKA_BRAVE_API_KEY on mika-gateway` posé dans une fonction séparée du
constructeur). Si ce cas passe au vert, le garde ne couvre pas le défaut qui
l'a fait naître, quelles que soient les autres règles.

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

## 4. Fire-Disposition

Ce plan livre trois détecteurs : la règle 1 et la règle 2 de
`check-substrate-leak.sh`, et le harnais anti-vacuité.

**Option retenue : (a) — exception nommée en allowlist, et l'allowlist des
violations est livrée VIDE.**

Deux mécanismes distincts, et la distinction est la moitié qui compte :

1. **Les six violations de §2.1 sont converties par U2, pas allowlistées.**
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

**Halte d'implémentation.** Si la mesure du volume d'annotations dépasse **25
sites** sur le périmètre, ne pas annoter en masse : le motif est trop large, et
un garde dont le coût d'entrée est cinquante annotations est un garde qu'on
désarme. Resserrer le motif, re-mesurer, et écrire la mesure dans le script.

**Si une violation de §2.1 se révèle non convertible** (contrainte non vue à la
lecture), elle passe alors en (a) plein : entrée grep-visible nommant le site,
ticket de suivi ouvert, assertion auto-nettoyante qui rougit le jour où le site
disparaît. **Ne pas l'annoter en `// substrate-ok:` — ce serait déclarer hors
population ce qui est une fuite, c'est-à-dire fermer le garde sur son propre
sujet.**

---

## 5. Acceptance criteria

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
  `ToolOutput::success` porteurs. Une règle ancrée sur `ToolOutput::error` ne
  voit ni l'un ni l'autre. Justification complète en §3.3.
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
  `audit_events` porte la ligne. Idem pour `resolve_issue_order` et `check_task`.
- **V4 — le garde mord.** `bash scripts/test-check-substrate-leak.sh` passe, y
  compris son contrôle négatif portant le littéral M3 dans une fonction séparée
  du constructeur.
- **V5 — l'arbre balayé est propre.** `bash scripts/check-substrate-leak.sh`
  sort 0 sur `crates/mika-agent/src`.
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

- U1 à U6 livrés ; les six sites de §2.1 convertis ; l'allowlist du garde vide.
- V1 à V6 verts ; le job `substrate-leak-lint` présent et passant en CI.
- `docs/skills.md` ne promet plus, il nomme.
- Le corps de PR déclare les deux divergences de §5 et le changement de contrat
  de §3.5 (le texte opérateur reste lisible sur tier `Default`, après le repli
  neutre et une ligne blanche).
- Les jugements de §2.4 tranchés **explicitement** dans le code, par annotation
  portant sa raison — jamais par omission.

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

# fix(3132) — `mika config set` refuse quatre clés que la TUI et `set_config` acceptent

**Vol unique.** Type : fix. Branche : `fix/3132/cli-mika-config-set-ignore-chat-id`.
Ticket : senara-solutions/mika#3132.

Le ticket nomme trois axes, et ils tiennent dans un seul vol parce qu'ils partagent
une cause : les deux registres de clés de configuration ne sont reliés par **aucune
garde générale**, et le seul chemin d'écriture qui reste à la CLI passe par argv.

- **Axe 1** — quatre clés (`chat_id`, `proactive_daily_budget`,
  `proactive_pause_until`, `language`) sont dans `SETTABLE_CONFIG_KEYS` et absentes de
  `CONFIG_KEYS` : deux surfaces les acceptent, une troisième répond « Unknown config key ».
- **Axe 2** — le seul lien entre les deux listes est
  `mika2425_both_keys_are_declared_as_database_backed`, qui ne couvre que les deux clés
  `context_history_*` et interroge la relation **dans l'autre sens**. Une garde par
  inclusion aurait rougi sur les quatre.
- **Axe 3** — la branche `Database` de `run_set` ne lit la valeur que dans argv, donc
  un `chat_id` atterrit dans l'historique du shell et dans `/proc/<pid>/cmdline`.

## Le défaut, tel qu'il se mesure

**Le registre de l'outil**, `crates/mika-agent/src/config_keys.rs:53`, lu à HEAD
(`7e35c5a6`) :

```rust
pub const SETTABLE_CONFIG_KEYS: &[&str] = &[
    "chat_id",
    "timezone",
    "thinking_level",
    PROACTIVE_DAILY_BUDGET_KEY,   // "proactive_daily_budget"
    PROACTIVE_PAUSE_UNTIL_KEY,    // "proactive_pause_until"
    TENANT_LANGUAGE_KEY,          // "language"
];
```

Trois surfaces le lisent, et les trois acceptent les six clés :

| surface | site | ce qu'elle fait de la constante |
|---|---|---|
| outil `set_config` du modèle | `crates/mika-agent/src/tools/set_config.rs:33` | en bâtit l'`enum` du schéma JSON |
| `/config set` de la TUI | `crates/mika-cli/src/tui/commands/handlers.rs:599` | `is_settable_key` en portillon |
| autocomplétion `/config set` | `crates/mika-cli/src/tui/commands/completers.rs:171` | en bâtit les propositions |

**Le registre de la CLI**, `crates/mika-common/src/config.rs:431-464`, déclare quatre
clés `ConfigBackend::Database` et pas une de plus : `timezone`, `thinking_level`,
`context_history_scope`, `context_history_max_tokens`.

**Le refus** tombe en `crates/mika-cli/src/commands/config.rs:112-114` :

```rust
let info = lookup_config_key(key).ok_or_else(|| {
    anyhow::anyhow!("Unknown config key: {key}\nRun `mika config list` to see all keys")
})?;
```

`lookup_config_key` (`config.rs:558`) cherche dans `CONFIG_KEYS` et nulle part
ailleurs. Donc `mika config --agent X set chat_id 123456` répond *Unknown config key:
chat_id* sur une clé que le modèle règle par outil et que l'opérateur règle dans la
TUI. Les trois autres clés citées se comportent identiquement. Même effet en lecture :
`run_get` (`config.rs:83-85`) passe par le même `lookup_config_key`, donc
`mika config get chat_id` ne peut pas lire une valeur que `set_config` a écrite.

**Pourquoi les deux listes peuvent diverger en silence.** Le littéral est écrit deux
fois — constante dans `mika-agent`, chaîne dans `mika-common` — parce que
`mika-common` ne peut pas dépendre de `mika-agent` (direction des dépendances). Le
doc-comment de `mika2425_both_keys_are_declared_as_database_backed`
(`config_keys.rs:956-978`) nomme exactement cette classe de défaut, et la garde qui
l'accompagne ne la couvre que pour deux clés :

```rust
#[test]
fn mika2425_both_keys_are_declared_as_database_backed() {
    for key in [CONTEXT_HISTORY_SCOPE_KEY, CONTEXT_HISTORY_MAX_TOKENS_KEY] {
        let info = mika_common::config::lookup_config_key(key).unwrap_or_else(…);
        assert_eq!(info.backend, mika_common::config::ConfigBackend::Database, …);
        assert!(info.env_var.is_none(), …);
    }
}
```

Elle part de deux clés nommées à la main et interroge `CONFIG_KEYS` ; elle ne parcourt
jamais `SETTABLE_CONFIG_KEYS`. C'est le `grooming_marker` de mika#2158 : la propriété
est vraie sur l'échantillon qu'on a pensé à écrire, et fausse sur quatre clés que
personne n'a repensé à écrire — tous les tests de ce fichier restant verts.

**La contrainte qu'un correctif naïf casserait.**
`mika2425_the_db_half_is_absent_from_the_tool_surface` (`config_keys.rs:932-945`)
exige l'inverse pour les deux clés `context_history_*` :

```rust
assert!(!is_settable_key(key), "mika#2425 refus 2 — `{key}` must not be in \
        SETTABLE_CONFIG_KEYS: that constant IS the set_config tool surface …");
```

L'asymétrie est **délibérée** : `SETTABLE_CONFIG_KEYS` *est* la surface d'outil du
modèle, et mika#2425 a refusé d'y exposer le fenêtrage de contexte. L'invariant à
poser est donc une **inclusion**, jamais une bijection :

```
SETTABLE_CONFIG_KEYS ⊆ { k ∈ CONFIG_KEYS | k.backend == Database }
```

Une garde d'égalité forcerait ce refus à s'ouvrir, et son message de panne demanderait
à son auteur de supprimer la garde de mika#2425 « dans le même commit » — c'est-à-dire
de relitiger une décision qui n'est pas la nôtre.

**Le canal de la valeur**, `crates/mika-cli/src/commands/config.rs:200-208` :

```rust
ConfigBackend::Database => {
    let val = value.ok_or_else(|| anyhow::anyhow!("Usage: mika config set {key} <value>"))?;
    …
}
```

Aucun chemin n'existe hors d'argv. La branche `Env` voisine, elle, en a deux
(`config.rs:149-196`) : prompt masqué pour une clé `secret`, prompt visible sinon.
La branche `Database` n'en a aucun, et c'est la branche par laquelle un `chat_id`
va passer dès que l'axe 1 est livré — donc l'axe 3 n'est pas un bonus, c'est ce qui
empêche l'axe 1 d'ouvrir un canal qui écrit un identifiant de conversation dans
`~/.zsh_history`.

## Requirements

- **R1** — `mika config --agent X set chat_id <v>`, `… set proactive_daily_budget <v>`,
  `… set proactive_pause_until <v>` et `… set language <v>` écrivent dans
  `customer_config` et ne répondent plus « Unknown config key ».
- **R2** — `mika config get <clé>` et `mika config list` rendent les quatre clés.
- **R3** — Les quatre déclarations portent `backend: Database`, `env_var: None`,
  `secret: false`.
- **R4** — Une garde parcourt `SETTABLE_CONFIG_KEYS` et exige de **chaque** entrée
  qu'elle soit déclarée `Database`-backed sans `env_var` dans `CONFIG_KEYS`. L'invariant
  est une inclusion : la garde n'exige rien dans l'autre sens.
- **R5** — `mika2425_the_db_half_is_absent_from_the_tool_surface` et
  `mika2425_both_keys_are_declared_as_database_backed` restent verts et intacts.
- **R6** — La valeur de la branche `Database` peut arriver par stdin : un
  `printf '%s' "$CHAT_ID" | mika config --agent X set chat_id` écrit sans que la valeur
  touche argv.
- **R7** — Sur un terminal, une valeur absente est demandée interactivement plutôt que
  refusée par un *Usage:*.
- **R8** — Une valeur vide, uniquement blanche, ou multi-ligne est refusée **en la
  nommant** — jamais écrite, jamais tronquée en silence.
- **R9** — `validate_config_value` s'applique à la valeur quelle que soit sa
  provenance : un seul site d'appel, après matérialisation.

## Design

### D1 — Quatre entrées dans le bloc `Database` de `CONFIG_KEYS`

`crates/mika-common/src/config.rs`, dans le bloc *Database backend (customer_config
table)*, après `thinking_level` et **avant** le commentaire mika#2425 (qui doit rester
collé aux deux clés qu'il explique) :

```rust
// mika#3132 — les quatre clés que `SETTABLE_CONFIG_KEYS` expose déjà au modèle
// et à la TUI. Le littéral est écrit deux fois parce que `mika-common` ne peut
// pas dépendre de `mika-agent`; l'inclusion est tenue par
// `config_keys::tests::mika3132_toute_cle_reglable_est_declaree_database_backed`.
// Retirer une de ces quatre entrées fait rougir cette garde, jamais ce fichier.
ConfigKeyInfo { key: "chat_id", backend: Database, env_var: None, secret: false, … },
ConfigKeyInfo { key: "proactive_daily_budget", … },
ConfigKeyInfo { key: "proactive_pause_until", … },
ConfigKeyInfo { key: "language", … },
```

**`secret: false` sur les quatre, `chat_id` comprise, et c'est une décision.** Le
champ `secret` est lu par deux consommateurs : `format_display_value`
(`mika-cli/src/commands/config.rs:391`), qui rend `[REDACTED]`, et la branche `Env` de
`run_set`. Un `secret: true` masquerait `chat_id` dans `mika config get` et
`mika config list` — précisément le champ qu'un opérateur relit après un
provisioning Telegram pour vérifier qu'il a câblé la bonne conversation. Un
identifiant de conversation n'est pas un credential : il n'autorise rien à lui seul.
Le problème que le ticket nomme est celui du **canal** (argv), et il est traité par D4
sans toucher à la classification.

**`env_var: None` sur les quatre** : la garde D2 l'exige, pour la raison que
mika#2425 donne déjà — une variable d'environnement qui shadowe une clé `Database`
rendrait `mika config set` vrai et sans effet.

Aucun arm n'est à ajouter à `get_effective_value` : il est explicitement sauté pour
les clés `Database`, dont la valeur est lue dans la base par `run_get`
(`config.rs:89-94`) et par `run_list` (`config.rs:283`). Le test
`test_get_effective_value_covers_all_non_db_non_env_keys`
(`mika-cli/src/commands/config.rs:634`) saute déjà `Database | Env`, donc il reste
vert sans intervention.

`validate_config_value` (`config_keys.rs:560-647`) porte déjà un arm pour chacune des
quatre clés — `chat_id` parse un `i64`, `proactive_daily_budget` un `u32` ≤ 24,
`language` accepte `fr|en`, `proactive_pause_until` un RFC 3339 ou `none` — et la
branche `Database` de la CLI l'appelle déjà (`config.rs:203`). Rien à ajouter : c'est
cette pré-existence qui rend l'axe 1 aussi petit.

### D2 — La garde d'inclusion, et son prédicat pur

Dans `crates/mika-agent/src/config_keys.rs`, module `tests`, un prédicat **pur** et
sa garde :

```rust
/// Les clés réglables dont on accepte sciemment qu'elles ne soient pas
/// déclarées `Database`-backed dans `CONFIG_KEYS`. Livrée VIDE, et assertie
/// vide : voir la Fire-Disposition du plan de mika#3132.
const SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT: &[&str] = &[];

/// Rend, pour chaque clé fournie, la raison pour laquelle elle n'est pas
/// déclarée `Database`-backed dans `CONFIG_KEYS` — vide quand tout va bien.
/// Pur : prend la liste en argument, ce qui est ce qui rend le contrôle
/// négatif écrivable sans casser le dépôt.
fn settable_keys_not_database_backed(keys: &[&str]) -> Vec<String> { … }
```

Le prédicat prend la liste **en argument** et non `SETTABLE_CONFIG_KEYS` : c'est la
seule forme qui permet de lui passer une clé fictive et de vérifier qu'il la
rapporte. Il rapporte trois états distincts, parce qu'ils n'appellent pas le même
geste :

| état | message |
|---|---|
| absente de `CONFIG_KEYS` | déclare-la dans le bloc `Database` de `mika-common/src/config.rs` |
| déclarée, mais `backend != Database` | `mika config set` écrira dans le mauvais dos (fichier ou `.env`) |
| déclarée `Database` avec un `env_var` | une variable d'environnement rendrait l'écriture vraie et sans effet |

La garde est l'appel sur la vraie constante, moins l'exemption :

```rust
#[test]
fn mika3132_toute_cle_reglable_est_declaree_database_backed() {
    let divergences = settable_keys_not_database_backed(SETTABLE_CONFIG_KEYS);
    // filtrées par SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT
    assert!(divergences.is_empty(), "mika#3132 — …");
}
```

**Pourquoi une inclusion et pas une égalité**, redit au site : les deux clés
`context_history_*` sont `Database`-backed et **délibérément** hors de
`SETTABLE_CONFIG_KEYS` (mika#2425 refus 2, tenu par
`mika2425_the_db_half_is_absent_from_the_tool_surface:932`). Une garde d'égalité
rougirait sur ce refus et son remède serait de l'ouvrir. Le doc-comment de la nouvelle
garde porte cette phrase, parce que c'est l'erreur que le prochain éditeur fera.

**Pourquoi la garde vit dans `mika-agent` et non dans `mika-common`** : elle doit lire
les deux registres, et seul `mika-agent` voit les deux (`mika-common` ne peut pas
dépendre de lui). C'est aussi là que vit déjà la garde mika#2425, dont celle-ci est la
généralisation — les deux côte à côte, dans le même module.

**Ce qu'elle ne couvre pas, et c'est dit.** Elle vérifie la *déclaration*, pas que
`mika config set <clé>` écrive effectivement — ça, c'est le travail des tests de bout
en bout, qui exigeraient une base. Elle ne vérifie pas non plus que
`validate_config_value` porte un arm pour chaque clé réglable : le `_ => {}`
terminal de cette fonction est un trou connu, antérieur à ce ticket, et une garde
dessus est un ticket de suivi nommé au § *Périmètre*.

### D3 — Le lecteur de valeur, deux fonctions pures et un appelant mince

Dans `crates/mika-cli/src/commands/config.rs`, la décision est séparée de l'effet
(forme de `classify_brief_size_alert_bytes` dans `mika-common`, qui existe pour que le
palier soit assertable sans souscripteur `tracing`) :

```rust
/// D'où vient la valeur de la branche `Database`.
#[derive(Debug, PartialEq)]
enum DatabaseValueSource { Argv(String), Stdin, Prompt }

/// Pure. `stdin_is_terminal` est injecté, jamais lu ici.
fn decide_database_value_source(value: Option<String>, stdin_is_terminal: bool)
    -> DatabaseValueSource;

/// Pure. Trim, puis refus nommé du vide et du multi-ligne.
fn finalize_database_value(key: &str, raw: &str) -> Result<String>;
```

Table de décision — trois cas, et le troisième est le seul changement de
comportement visible sur un usage existant :

| `value` | stdin | source | ce qui se passe |
|---|---|---|---|
| `Some(v)` | — | `Argv(v)` | inchangé : le chemin que les scripts existants empruntent |
| `None` | pipe/fichier | `Stdin` | la valeur est lue jusqu'à EOF — **nouveau** |
| `None` | terminal | `Prompt` | `dialoguer::Input` visible — remplace le *Usage:* |

Un `Some(v)` l'emporte toujours : c'est ce qui garantit qu'aucune invocation existante
ne change de comportement, et donc qu'il n'y a **aucune** rupture de compatibilité
dans ce vol.

`finalize_database_value` est le site unique du refus. Après `trim()` :

- chaîne vide ⇒ `anyhow::bail!` nommant la clé et la provenance. Un EOF immédiat sur
  stdin (`printf '' | mika config set chat_id`) atterrit ici, et doit dire « aucune
  valeur sur stdin » et non écrire une chaîne vide dans `customer_config` — une clé
  présente à la valeur vide se lit tout autrement qu'une clé absente (motif
  mika#2331).
- une `\n` résiduelle après trim ⇒ refus nommant le multi-ligne. Sans ce refus,
  `validate_config_value` dirait « Invalid chat_id » sur ce qui est en réalité un pipe
  mal formé : un diagnostic qui envoie chercher la panne du côté de la valeur alors
  qu'elle est du côté du canal. Un prompt interactif ne peut pas produire ce cas ; un
  `cat fichier |` le produit à la première occasion.

Le trim est complet et non seulement terminal : aucune des quatre clés n'admet
d'espace interne significatif (un `i64`, un `u32`, un RFC 3339, `fr|en`), et un
`$(…)` de shell ajoute volontiers une espace.

### D4 — Le branchement, et l'ordre des gestes

La branche `Database` de `run_set` devient :

```rust
ConfigBackend::Database => {
    let source = decide_database_value_source(value, std::io::IsTerminal::is_terminal(&std::io::stdin()));
    let raw = match source {
        DatabaseValueSource::Argv(v) => v,
        DatabaseValueSource::Stdin   => { /* read_to_string */ }
        DatabaseValueSource::Prompt  => { /* dialoguer::Input */ }
    };
    let val = finalize_database_value(key, &raw)?;
    mika_agent::config_keys::validate_config_value(key, &val).map_err(…)?;
    warn_session_scope_on_non_singleton(key, &val, &ctx.home_dir);
    ctx.async_db.set_customer_config(key, &val).await?;
    println!("Set {key} = {val}");
}
```

**`validate_config_value` garde son site unique, après matérialisation.** Le valider
avant serait impossible (la valeur n'existe pas encore) et le valider deux fois ferait
deux juges du même fait.

**Le `println!` final est inchangé et affiche la valeur.** C'est cohérent avec
`secret: false` (D1) et avec les trois autres branches ; une valeur arrivée par stdin
reste affichée, parce que le but de stdin est de la garder hors de l'historique du
shell, pas hors de l'écran — l'opérateur vient de la saisir.

**Ce que ce vol refuse explicitement de faire.** Avertir sur un `chat_id` passé en
argv. Ce serait un troisième juge de « quelle clé porte une valeur sensible », à côté
de `ConfigKeyInfo::secret` et de la branche `Env` — une troisième liste libre de
diverger (classe mika#2158), pour un avertissement qui apparaîtrait sur chaque
invocation scriptée existante. Le ticket demande qu'un chemin hors argv **existe** ;
il existe. Durcir argv est un ticket de suivi, nommé au § *Périmètre*.

### D5 — Les documents, et la synchronisation que le CI exige

`docs/configuration.md` est l'unique source de vérité, et
`crates/mika-agent/docs/configuration.md` en est la copie que `build.rs` embarque. Le
job `docs-sync` de `ci.yml` refuse une PR qui touche `docs/` sans avoir lancé
`scripts/sync-agent-docs.sh`. Les quatre clés et le chemin stdin se documentent donc
**dans `docs/`**, suivis du script, dans le même commit.

`crates/mika-agent/CLAUDE.md` et `crates/mika-cli/CLAUDE.md` reçoivent chacun un
paragraphe court (la nouvelle garde ; le chemin stdin et son refus du multi-ligne).
`crates/mika-common/tests/claude_md_size.rs` borne la taille de chaque `CLAUDE.md` :
les deux ajouts sont de quelques lignes et ne l'approchent pas.

## Périmètre — ce vol et ce qui est renvoyé

**Dans ce vol** : les trois axes du ticket, en entier.

**Renvoyé, avec sa raison** :

- **Le `_ => {}` terminal de `validate_config_value`** (`config_keys.rs:~645`) accepte
  en silence toute valeur pour une clé sans arm. Aucune des six clés réglables n'est
  dans ce cas aujourd'hui, et rien ne garantit que la septième le sera. C'est une
  garde de plus, sur un prédicat de plus, et la coudre à celle de D2 en ferait une
  garde qui rougit sous deux messages pour deux défauts distincts.
  `Tracked in:` ticket de suivi à ouvrir — *« `validate_config_value` n'a pas de garde
  d'exhaustivité sur `SETTABLE_CONFIG_KEYS` »*.
- **Le durcissement d'argv** (avertir, ou refuser, une valeur sensible passée en ligne
  de commande) — voir D4 pour le pourquoi. `Tracked in:` ticket de suivi à ouvrir —
  *« une valeur de configuration sensible passée en argv n'est pas signalée »*.
- **Le chemin stdin pour les branches `File` et `Env`**. `Env` a déjà ses deux prompts
  et son refus en non-TTY ; `File` écrit des réglages non sensibles. Élargir
  maintenant serait du périmètre que le ticket ne nomme pas, sur des branches dont le
  mode de panne n'a pas été mesuré.

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | les quatre clés sont déclarées `Database`/`None`/`false` | `cargo test -p mika-agent --lib config_keys` — la garde D2 |
| V2 | la garde **voit** une clé absente | contrôle négatif : `settable_keys_not_database_backed(&["chat_id", "cle_inexistante_mika3132"])` rapporte la seconde |
| V3 | la garde **voit** un mauvais backend | contrôle négatif : la même fonction sur `&["log_level"]` (déclarée `File`) la rapporte |
| V4 | la garde **voit** un `env_var` parasite | contrôle négatif sur une clé `Database` fictive portant un `env_var` |
| V5 | l'allowlist est vide | `assert!(SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT.is_empty(), …)` |
| V6 | mika#2425 est intact dans les deux sens | `mika2425_the_db_half_is_absent_from_the_tool_surface` et `mika2425_both_keys_are_declared_as_database_backed` verts, non modifiés (`git diff` sur leurs lignes = vide) |
| V7 | la table de décision de la source | `decide_database_value_source` sur les trois cas + la priorité d'`Argv` sous les deux valeurs de `stdin_is_terminal` |
| V8 | le refus du vide et du multi-ligne | `finalize_database_value` sur `""`, `"   "`, `"\n"`, `"12\n34"`, et le cas nominal `" 123 "` → `"123"` |
| V9 | le message de refus nomme la clé | `assert!(err.to_string().contains(key))` sur les deux refus |
| V10 | la CLI ne régresse pas | `cargo test -p mika-cli` — dont `test_get_effective_value_covers_all_non_db_non_env_keys`, qui saute `Database` et doit rester vert sans retouche |
| V11 | lint et format | `cargo clippy --all-targets`, `cargo fmt --check` |
| V12 | les documents sont synchronisés | `scripts/sync-agent-docs.sh` puis `git diff --exit-code crates/mika-agent/docs/` |
| V13 | la discipline de pipeline | `bash scripts/verify-pipeline.sh origin/main` |

**Ce que le contrat ne couvre pas, et c'est dit** : aucun test de ce vol n'ouvre une
base et n'écrit dans `customer_config`. `set_customer_config` et `get_customer_config`
sont pré-existants, exercés par les deux clés `context_history_*` qui empruntent
exactement le même chemin, et le changement de ce vol est une **déclaration** plus un
lecteur de valeur — ni l'un ni l'autre ne touche à la couche base. Le geste
d'attestation de bout en bout est manuel et nommé en AC1.

## Definition of Done

- D1 — Les quatre `ConfigKeyInfo` sont dans le bloc `Database` de
  `crates/mika-common/src/config.rs`, avec le commentaire d'ancrage qui cite la garde.
- D2 — `settable_keys_not_database_backed` et
  `mika3132_toute_cle_reglable_est_declaree_database_backed` sont dans le module
  `tests` de `crates/mika-agent/src/config_keys.rs`, à côté des gardes mika#2425.
- D3 — `SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT` est livrée vide et son vide est **asserté**.
- D4 — La garde a été **vue rouge** sur les quatre clés avant que D1 n'atterrisse, et
  la sortie de ce rouge est citée dans le corps de PR.
- D5 — `decide_database_value_source` et `finalize_database_value` existent, sont
  pures, et sont les seuls juges respectivement de la provenance et du refus.
- D6 — La branche `Database` de `run_set` les appelle, et `validate_config_value`
  garde son site d'appel unique après matérialisation.
- D7 — Les gardes mika#2425 sont intactes : aucune ligne modifiée, aucune supprimée.
- D8 — V1 à V13 passent.
- D9 — `docs/configuration.md` documente les quatre clés comme réglables par
  `mika config set` et le chemin stdin ; `scripts/sync-agent-docs.sh` a été lancé et
  `crates/mika-agent/docs/configuration.md` est à jour dans le même commit.
- D10 — `crates/mika-agent/CLAUDE.md` et `crates/mika-cli/CLAUDE.md` portent chacun
  leur paragraphe.
- D11 — Les deux tickets de suivi du § *Périmètre* sont ouverts et nommés par une
  ligne `Tracked in:` dans le corps de PR.

## Acceptance criteria

Le corps du ticket mika#3132 ne porte pas de section `## Acceptance criteria` : les
critères ci-dessous sont dérivés des Requirements et du Verification Contract.

- **AC1** — Sur un agent provisionné, les quatre commandes suivantes écrivent et
  rendent `Set <clé> = <valeur>`, là où elles répondaient « Unknown config key » :
  ```
  mika config --agent <a> set chat_id 123456789
  mika config --agent <a> set proactive_daily_budget 5
  mika config --agent <a> set proactive_pause_until 2026-12-01T09:00:00Z
  mika config --agent <a> set language fr
  ```
- **AC2** — `mika config --agent <a> get chat_id` rend la valeur écrite en AC1, et
  `mika config --agent <a> list` fait apparaître les quatre clés.
- **AC3** — `mika config --agent <a> get chat_id --verbose` rend
  `backend: Database` et `source: database`.
- **AC4** — Une valeur invalide est refusée par le message de
  `validate_config_value` et non par « Unknown config key » :
  `mika config --agent <a> set chat_id abc` échoue en nommant le `chat_id`, et rien
  n'est écrit.
- **AC5** — `printf '%s' 123456789 | mika config --agent <a> set chat_id` écrit la
  valeur, et la valeur n'apparaît dans aucun argv (vérifiable : la ligne de commande
  ne la contient pas).
- **AC6** — `printf '' | mika config --agent <a> set chat_id` échoue en nommant
  l'absence de valeur sur stdin, et `mika config get chat_id` rend la valeur
  précédente — jamais une chaîne vide.
- **AC7** — `printf '12\n34' | mika config --agent <a> set chat_id` échoue en nommant
  le multi-ligne, et non en nommant un `chat_id` invalide.
- **AC8** — Sur un terminal, `mika config --agent <a> set chat_id` sans valeur
  demande la valeur au lieu de rendre un *Usage:*.
- **AC9** — `mika config --agent <a> set chat_id 42` se comporte exactement comme
  avant ce vol sur le chemin argv : une valeur fournie n'ouvre ni stdin ni prompt.
- **AC10** — `cargo test -p mika-agent --lib config_keys` passe, la nouvelle garde
  comprise, et les quatre gardes `mika2425_*` passent sans avoir été modifiées.
- **AC11** — Un retrait de l'une des quatre entrées de `CONFIG_KEYS` fait rougir
  `mika3132_toute_cle_reglable_est_declaree_database_backed` en nommant la clé retirée
  et le fichier où la déclarer (vérifié par les contrôles négatifs V2–V4, qui
  mesurent la même détection sans casser le dépôt).
- **AC12** — `SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT` est vide à l'atterrissage, et son
  vide est asserté par un test.
- **AC13** — Ajouter une septième clé à `SETTABLE_CONFIG_KEYS` sans la déclarer dans
  `CONFIG_KEYS` fait rougir la garde — c'est la même propriété que AC11, dans la
  direction que ce ticket existe pour fermer.
- **AC14** — `cargo clippy --all-targets` et `cargo fmt --check` sont propres ;
  `bash scripts/verify-pipeline.sh origin/main` passe.
- **AC15** — `scripts/sync-agent-docs.sh` laisse `git diff` vide sous
  `crates/mika-agent/docs/`.

## Fire-Disposition

Ce vol livre **un détecteur** :
`mika3132_toute_cle_reglable_est_declaree_database_backed` — son chemin de succès est
« aucune divergence trouvée ».

**Option retenue : (a) — exception nommée en allowlist, livrée VIDE.**

- **L'allowlist existe et est nommée** :
  `SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT`, `grep`-visible dans
  `crates/mika-agent/src/config_keys.rs`.
- **Elle est livrée vide**, parce que les **quatre** violations existantes sont
  corrigées dans ce même vol (D1). Il n'y a aucune donnée à exempter, donc aucune
  entrée à écrire — c'est la forme que ce dépôt emploie déjà pour
  `mika2425_identity_context_history_has_a_single_reader`,
  `mika2457_the_cli_resolves_no_budget_locally` et
  `mika2362_no_rail_recomputes_the_retry_threshold_inline`.
- **Son vide est asserté** (V5), avec le message qui dit le remède :
  *« quand cette garde rougit, déclare la clé dans le bloc `Database` de
  `mika-common/src/config.rs` — n'ajoute pas d'entrée ici. Une allowlist qui cesse
  d'être vide ici est exactement le faux vert que mika#3132 a fermé. »* C'est
  l'assertion auto-nettoyante : elle rougit le jour où quelqu'un tente de réparer la
  garde par une exception au lieu de par une déclaration.
- **Le détecteur atterrit armé** — ni `#[ignore]`, ni `#[cfg(skip)]`, rien de désarmé
  dans ce vol, et rien ne remonte à l'opérateur.
- **Il a été vu rouge d'abord, et c'est un livrable** (D4). Un détecteur qui démarre
  vert n'a jamais montré qu'il voit. L'ordre de commit impose de brancher la garde
  **avant** d'ajouter les quatre déclarations, de constater le rouge nommant les quatre
  clés, et de citer cette sortie dans le corps de PR. Les trois contrôles négatifs
  (V2, V3, V4) tiennent cette propriété **durablement** : ils mesurent la détection sur
  une entrée fabriquée, donc la garde reste attestée comme voyante même quand le dépôt
  est propre.
- **Les deux renvois du § *Périmètre* portent chacun leur ticket de suivi** (D11),
  nommés par une ligne `Tracked in:` dans le corps de PR. Aucun des deux n'est une
  violation du détecteur de ce vol : ils nomment deux autres propriétés, non gardées.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/config.rs` — quatre `ConfigKeyInfo` + commentaire d'ancrage | 40 |
| `crates/mika-agent/src/config_keys.rs` — `settable_keys_not_database_backed` (trois états) + allowlist vide + doc-comment | 35 |
| tests (`crates/mika-agent/src/config_keys.rs`) — garde d'inclusion, trois contrôles négatifs, assertion du vide | 55 |
| `crates/mika-cli/src/commands/config.rs` — `DatabaseValueSource`, `decide_database_value_source`, `finalize_database_value`, branchement de `run_set` | 40 |
| tests (`crates/mika-cli/src/commands/config.rs`) — table de décision, cinq cas de refus/nominal, messages nommant la clé | 45 |

Total estimé : 215 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 215 estimées contre 279 : 64 lignes de marge, 23 %.
Les trois axes du ticket partagent une cause et se mesurent ensemble, mais ce n'est
pas ce qui les fait tenir — c'est que **le gros du mécanisme existe déjà** :
`validate_config_value` porte son arm pour chacune des quatre clés, la branche
`Database` de la CLI l'appelle déjà, `set_customer_config` / `get_customer_config`
sont exercés par les deux clés `context_history_*` sur exactement le même chemin, et
`get_effective_value` saute déjà les clés `Database`. Ce vol **déclare** quatre
entrées dans une table, **généralise** une garde dont le modèle est à vingt lignes
au-dessus, et **ajoute deux fonctions pures** de quinze lignes dans un fichier
qu'elles ne restructurent pas.

Trois fichiers de production, dans trois crates, mais sans dépendance circulaire entre
les changements : la garde lit les deux registres, les déclarations ne lisent rien, le
lecteur de valeur ne connaît aucun des deux. Aucune migration, aucun DDL, aucun
bundle, aucune surface HTTP, aucun composant UI, aucun changement de format de fil.
La fin de pipeline est celle d'un fix multi-crate sans schéma : la seule contrainte de
pipeline au-delà des tests est `scripts/sync-agent-docs.sh`, un script d'une
invocation.

## Ordre de commit

Le plancher mergeable est au commit 2 : à ce point les quatre clés sont réglables par
la CLI et la divergence est tenue par la garde. Les commits 3 et 4 ferment le canal
argv et les documents.

1. **Le prédicat pur et ses trois contrôles négatifs**, sans la garde sur la vraie
   constante. Vert seul : `settable_keys_not_database_backed` n'est appelée que par
   des tests qui lui passent des listes fabriquées, donc elle n'interroge pas encore
   `SETTABLE_CONFIG_KEYS`. C'est à ce commit que la détection est attestée.
2. **La garde branchée sur `SETTABLE_CONFIG_KEYS`, l'allowlist vide, puis les quatre
   déclarations dans `CONFIG_KEYS`** — dans cet ordre à l'intérieur du commit, et le
   rouge intermédiaire est **constaté avant** le `git add` de
   `mika-common/src/config.rs` : lance `cargo test -p mika-agent --lib config_keys`,
   garde la sortie nommant les quatre clés, et cite-la dans le corps de PR (D4).
   Vert à la fin du commit. **R1, R2, R3, R4, R5 sont tenus ici** : AC1 à AC4 et AC10
   à AC13 passent, et le vol est mergeable à ce point si le plafond devenait serré.
3. **Le lecteur de valeur** — `DatabaseValueSource`,
   `decide_database_value_source`, `finalize_database_value`, le branchement de la
   branche `Database`, et la table de tests. Vert seul. **R6 à R9** : AC5 à AC9.
4. **Les documents** — `docs/configuration.md`, puis
   `scripts/sync-agent-docs.sh`, puis les deux paragraphes de `CLAUDE.md`. Le script
   et le fichier qu'il réécrit vont dans le **même** commit, sinon le job `docs-sync`
   rougit sur un état intermédiaire poussé. **D9, D10** : AC15.

Les deux tickets de suivi (§ *Périmètre*) s'ouvrent avant la PR, pour que leurs
numéros entrent dans les lignes `Tracked in:` du corps.

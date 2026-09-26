# Plan: `truncate_output` coupe en octets ce que son commentaire appelle des caractères (#2103)

## Problem

`crates/mika-agent/src/skills/builtin_handlers.rs:101-106` :

```rust
/// Truncate output content to MAX_OUTPUT_LEN characters.
fn truncate_output(output: &mut ToolOutput) {
    if output.content.len() > MAX_OUTPUT_LEN {
        output.content.truncate(MAX_OUTPUT_LEN);
```

Le doc-comment dit **characters**. `String::len()` et `String::truncate()` comptent des **octets**, et `truncate` assène `assert!(self.is_char_boundary(new_len))`. Sur une sortie d'outil contenant de l'UTF-8 multi-octets, l'octet 10000 tombe au milieu d'un caractère et le processus panique.

Coût mesuré : 26 panics depuis 2026-08-29 03:12Z, dont 18 remontées en « webhook drain processing panicked » — la panique n'est pas contenue dans l'appel d'outil, elle interrompt le drain des webhooks.

Le dépôt possède déjà l'API correcte : `mika_common::text::safe_truncate` (`crates/mika-common/src/text.rs`), qui utilise `floor_char_boundary` et ne panique jamais. Le site défectueux ne l'appelle pas.

### Pourquoi la garde existante ne l'a pas vu

`scripts/check-byte-slices.sh` (issu de mika#764) cherche deux motifs de **tranche** : `&s[..x.len().min(N)]` (Pattern A) et `&content[..LITERAL]` (Pattern B). `String::truncate(n)` panique pour exactement la même raison et n'a pas la forme d'une tranche. Le lint protège **une syntaxe, pas une propriété** — c'est la récidive de la classe mika#764 sous une autre écriture.

## Audit de la classe (AC4)

Recensement complet de `crates/` — APIs qui indexent une `String`/`&str` en octets et paniquent hors frontière : `truncate`, `split_off`, `split_at`, `insert`, `remove`, `drain`.

**Défauts vivants (3) :**

| Site | Forme | Diagnostic |
|---|---|---|
| `mika-agent/src/skills/builtin_handlers.rs:103` | `content.truncate(MAX_OUTPUT_LEN)` | Le bug du ticket. Panique. |
| `mika-agent/src/compaction.rs:79` | `summary_text.truncate(MAX_SUMMARY_CHARS)` | **Deuxième instance.** La « garde » aux lignes 81-83 est *morte* (`truncate` panique avant qu'elle ne s'exécute) **et *vacue*** (`s.is_char_boundary(s.len())` est toujours vrai — la fin d'une `String` est toujours une frontière ; la boucle `pop()` ne s'exécuterait jamais). Deux défauts superposés qui donnent l'apparence d'une protection. |
| `mika-cli/src/commands/logs.rs:266` | `input.split_at(input.len() - 1)` | **Troisième instance.** `input` est un argument CLI (`mika logs --since`). `mika logs --since 30é` panique. |

**Corrects, à annoter avec la raison (3) :**

| Site | Raison |
|---|---|
| `mika-agent/src/teams/prompt.rs:235` | index descendu jusqu'à `is_char_boundary` avant l'appel |
| `mika-agent/src/mcp/mod.rs:400` | idem |
| `mika-agent/src/agent_loop/mod.rs:464` | `system_prompt_original_len` est un `String::len()` relevé en `mod.rs:980` avant concaténation — c'est une longueur réelle, donc toujours une frontière valide |

**Hors classe (aucune notion de frontière), à annoter :** `Vec::truncate` en `mika-cli/src/commands/logs.rs:500`, `mika-agent/src/db.rs:6589`, `db.rs:10271`, `milestone_manager/reader.rs:418`, `kg/query.rs:479`, `kg/query.rs:1109`.

**Hors classe, aucun changement :** `OpenOptions::truncate(bool)` (`mika-cli/src/tui/app.rs:411`, `mika-common/src/github_app.rs:338`) ; `Vec<u8>::drain` (`mika-cli/src/commands/tasks.rs:362`, des octets, pas du texte) ; `rendered.split_at(find(..))` (`research/mechanism_analyzer.rs:1157`, test, `find` rend toujours une frontière).

**Absents du dépôt :** `String::split_off`, `String::insert(idx, ch)`, `String::remove(idx)` — zéro occurrence. Le lint les couvrira quand même, pour que la première introduction soit refusée.

**La duplication, cause de fond.** Cinq implémentations de la même marche de frontière coexistaient : `mika-common/src/text.rs` (la canonique), `skills/executor.rs:867-880`, `mcp/mod.rs`, `teams/prompt.rs`, et `db.rs:8081` (`truncate_utf8_safe`). Quatre copies correctes et un site qui l'a oubliée — c'est la forme habituelle du défaut : ce n'est pas qu'une règle était inconnue, c'est qu'elle devait être réécrite à chaque appel. Les copies sont repliées sur `safe_truncate`, laissant une seule implémentation à maintenir juste.

## Changes

### 1. `builtin_handlers.rs` — le défaut du ticket

Appeler `mika_common::text::safe_truncate`, et aligner le doc-comment sur ce que le code fait (des **octets**).

### 2. `compaction.rs` — deuxième instance

Même correctif ; supprimer la garde morte et vacue des lignes 81-83 plutôt que la laisser suggérer une protection inexistante. `MAX_SUMMARY_CHARS` est renommé `MAX_SUMMARY_BYTES` : le nom faisait partie du mensonge.

### 3. `logs.rs::parse_time_expr` — troisième instance

Découper sur le dernier **caractère** (`chars().next_back()`) au lieu du dernier octet. Une entrée non-ASCII doit produire l'erreur `Unknown time expression` déjà prévue, pas une panique.

### 4. Repli des copies correctes sur `safe_truncate`

`teams/prompt.rs`, `mcp/mod.rs`, `executor.rs`, `db.rs::truncate_utf8_safe` : remplacer les marches de frontière écrites à la main par l'appel canonique. Comportement identique (la marche manuelle et `floor_char_boundary` calculent le même index) ; une seule implémentation à maintenir juste.

`agent_loop/mod.rs:464` garde son `truncate` — il restaure une longueur relevée, pas un budget calculé — et reçoit une annotation `// safe-byte-slice:` avec sa raison.

### 5. `scripts/check-byte-slices.sh` — étendre la garde à la propriété (AC4)

Deux motifs nouveaux, et un en-tête qui énonce la **propriété** protégée (« tout index d'octet dans du texte doit être une frontière de caractère ») plutôt que la liste des syntaxes connues, pour que la prochaine extension se fasse par propriété.

- **Pattern C — `.truncate(<arg non booléen>)`** : signalé partout dans `crates/`, sauf annotation `// safe-byte-slice: <raison>`. Délibérément **non filtré par nom de variable** : un filtre par nom (`content|body|…`, comme le Pattern B) reproduirait exactement l'erreur qu'on corrige — il ne verrait pas une `String` nommée autrement. Les `Vec::truncate` légitimes sont donc annotés une fois avec leur raison ; c'est le prix d'un lint sain, et c'est ce que l'AC4 demande (« traitées **ou allowlistées explicitement, avec la raison** »). `truncate(true)`/`truncate(false)` (`OpenOptions`) sont exclus par la forme de l'argument.
- **Pattern D — découpe/mutation par index d'octet calculé** : `.split_at(`, `.split_off(`, `.drain(..` dont l'argument contient `.len()` ou une arithmétique, plus `String::insert(idx, 'c')` (reconnu par son second argument littéral-caractère, ce qui l'isole des centaines d'`insert` de maps et de sets) et `.remove(` à index calculé.

### 6. Tests

- `builtin_handlers.rs` — **AC2** : contenu multi-octets dont la limite tombe **au milieu** d'un caractère (c'est le cas exact qui panique aujourd'hui) ; un test ASCII ne prouverait rien.
- `builtin_handlers.rs` — **AC3, anti-vacuité** : un contenu plus court que la limite ressort **intact et sans suffixe**. Sans ce dual, « tronque toujours à zéro » satisferait l'AC1.
- Mêmes duals pour `compaction.rs` et pour `parse_time_expr`.
- Test de la garde elle-même : `scripts/test-check-byte-slices.sh`, harnais anti-vacuité sur le modèle de `test-verify-no-sigpipe-grep.sh` — 12 cas, un par motif, plus l'allowlist dans les deux sens, plus le contrôle négatif qui est *exactement* le défaut mika#2103. Câblé en CI et sur `make check-byte-slices`. Le cas « Pattern C est aveugle au nom de variable » est celui qui compte : c'est la propriété que le lint d'origine n'avait pas.

## Verification

- `cargo test -p mika-agent -p mika-cli -p mika-common` (les nouveaux duals inclus)
- `cargo clippy --all-targets -- -D warnings`
- `bash scripts/check-byte-slices.sh` → sortie propre sur l'arbre corrigé
- Falsification du lint : réintroduire `output.content.truncate(MAX_OUTPUT_LEN)` sans annotation ⇒ le script doit sortir non-zéro. Un lint jamais vu échouer n'est pas une garde.

## Definition of Done

- [ ] Les trois défauts vivants corrigés, aucun `String::truncate`/`split_at` à index d'octet non gardé ne subsiste
- [ ] `check-byte-slices.sh` couvre la classe et a été vu refuser le défaut d'origine
- [ ] Tous les sites de la classe traités ou annotés avec leur raison
- [ ] `cargo test` + `clippy` + `check-byte-slices.sh` verts

## Acceptance criteria

- [ ] **AC1** — `truncate_output` ne panique plus quel que soit le contenu. La troncature se fait sur une frontière de caractère valide (par exemple via `char_indices` ou `floor_char_boundary`), et le commentaire dit ce que le code fait réellement — octets ou caractères, mais les deux d'accord.
- [ ] **AC2** — Test avec du contenu **multi-octets** dont la limite tombe au milieu d'un caractère : le cas exact qui panique aujourd'hui. Un test avec de l'ASCII seul ne prouve rien.
- [ ] **AC3** — Anti-vacuité : un contenu plus court que la limite est laissé intact et **non** tronqué. Sans ce dual, « tronque toujours à zéro » satisferait l'AC1.
- [ ] **AC4** — Le lint `check-byte-slices.sh` détecte désormais `String::truncate` sur une chaîne. **Et un audit du dépôt** : les autres API en octets qui paniquent sur frontière — `truncate`, `split_off`, `insert`, `remove`, `drain` avec un index calculé — sont recherchées et traitées ou allowlistées explicitement. La classe, pas la ligne.

## Out of scope

Les 56 « LLM transport error » de la même fenêtre : fournisseur (z.ai), sans rapport, explicitement hors périmètre par le ticket.

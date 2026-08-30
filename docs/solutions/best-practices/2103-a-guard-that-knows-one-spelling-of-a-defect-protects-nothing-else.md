---
module: skills, cli, scripts
tags: [utf8, char-boundary, byte-offset, string-truncate, structural-guard, lint-scope, anti-vacuity, recurrence, mika-764, panic, webhook-drain]
problem_type: guard-blind-to-its-own-class
category: best-practices
---

# Une garde qui ne connaît qu'une écriture d'un défaut laisse passer toutes les autres

## Problème (mika#2103, récidive de mika#764)

`builtin_handlers.rs` tronquait la sortie d'outil ainsi :

```rust
/// Truncate output content to MAX_OUTPUT_LEN characters.
fn truncate_output(output: &mut ToolOutput) {
    if output.content.len() > MAX_OUTPUT_LEN {
        output.content.truncate(MAX_OUTPUT_LEN);
```

Le doc-comment dit *characters*. `String::len()` et `String::truncate()` comptent des **octets**, et `truncate` assène `assert!(is_char_boundary(new_len))`. Une sortie d'outil contenant un accent suffisait : **26 panics en deux jours**, dont 18 qui ont emporté le drain des webhooks — la panique remonte hors de l'appel d'outil.

Le dépôt possédait déjà l'API correcte (`mika_common::text::safe_truncate`) **et** un lint CI dédié à cette classe exacte, `scripts/check-byte-slices.sh`, né de mika#764. Le lint est resté vert pendant les 26 paniques.

## Cause

Le lint cherchait des motifs de **tranche** — `&s[..s.len().min(n)]`, `&content[..500]`. `String::truncate(n)` panique pour exactement la même raison et n'a pas la forme d'une tranche. Le lint protégeait **une syntaxe, pas une propriété**.

Et son Pattern B allait plus loin dans la même erreur : il ne se déclenchait que sur une liste de *noms de variables* (`content|body|output|…`), pour éviter les faux positifs sur `&[u8]`. Une garde filtrée par nom ne peut structurellement pas voir la `String` que personne n'avait anticipé d'appeler ainsi.

## Ce que l'audit a trouvé

Chercher la **classe** plutôt que la ligne a fait apparaître **trois** défauts vivants, pas un :

| Site | Forme | Nature |
|---|---|---|
| `skills/builtin_handlers.rs` | `content.truncate(MAX_OUTPUT_LEN)` | le défaut signalé |
| `compaction.rs` | `summary_text.truncate(MAX_SUMMARY_CHARS)` | même défaut, **plus une fausse garde** |
| `cli/commands/logs.rs` | `input.split_at(input.len() - 1)` | même propriété, autre API, entrée CLI |

La fausse garde de `compaction.rs` mérite d'être regardée en face :

```rust
summary_text.truncate(MAX_SUMMARY_CHARS);
// Ensure we don't cut in the middle of a multi-byte char
while !summary_text.is_char_boundary(summary_text.len()) {
    summary_text.pop();
}
```

Deux défauts empilés en une apparence de protection. Elle est **inatteignable** — `truncate` panique avant que la boucle ne s'exécute — et **vacue** — `s.is_char_boundary(s.len())` est toujours vrai, la fin d'une `String` étant toujours une frontière ; la boucle ne tournerait jamais même si on l'atteignait. Un lecteur pressé y voit une garde et passe.

Cause de fond : **cinq** implémentations de la même marche de frontière coexistaient (`text.rs` la canonique, `executor.rs`, `mcp/mod.rs`, `teams/prompt.rs`, `db.rs`). Quatre correctes, une oubliée. Le défaut n'est pas qu'une règle était inconnue — c'est qu'elle devait être réécrite à chaque appel.

## Solution

1. **Les trois défauts corrigés**, et les cinq copies repliées sur `safe_truncate` : une seule implémentation à maintenir juste.
2. **Le lint étendu par propriété**, pas par syntaxe. Son en-tête énonce désormais la propriété — *tout index d'octet dans du texte doit tomber sur une frontière de caractère* — et dit explicitement d'étendre par propriété.
3. **Pattern C délibérément aveugle au nom de variable.** Tout `.truncate(N)` doit être sûr par construction ou porter `// safe-byte-slice: <raison>` — `Vec::truncate` compris. Filtrer par nom aurait reproduit la faute qu'on corrige. Le prix est une douzaine d'annotations ponctuelles ; c'est un prix, pas un défaut.
4. **Un harnais anti-vacuité** (`scripts/test-check-byte-slices.sh`, 12 cas, câblé en CI et sur `make check-byte-slices`) dont le contrôle négatif est *exactement* le défaut mika#2103 — plus un cas qui vérifie que Pattern C ne se laisse pas contourner par un nom de variable inattendu.

## Ce qu'il faut retenir

- **Une garde se juge sur la propriété qu'elle protège, pas sur le défaut qui l'a fait naître.** mika#764 a produit une garde qui connaissait *une* écriture. La deuxième écriture est passée dans la même fonction du même dépôt.
- **Filtrer une garde par nom de variable, c'est la borner à ce qu'on a su imaginer.** Assumer les faux positifs et les annoter une fois donne une garde qui tient ; l'heuristique de nommage donne une garde qui rassure.
- **Un lint qu'on n'a jamais vu échouer n'est pas une garde.** Le harnais anti-vacuité est ce qui distingue les deux ; ici il n'existait pas, et c'est ce qui a permis 26 paniques sous un job CI vert.
- **Une fausse garde est pire que pas de garde** : elle éteint la question. Chercher `is_char_boundary` dans `compaction.rs` donnait un résultat rassurant et faux.
- **Un commentaire qui contredit son code est un bug, pas une imprécision.** « characters » au-dessus d'une API en octets *est* la totalité de mika#2103. Quand un nom porte l'unité (`MAX_SUMMARY_CHARS`), il fait partie du mensonge et se corrige avec.
- **Quand une règle doit être réécrite à chaque appel, elle sera oubliée quelque part.** Cinq copies, un oubli. Le correctif durable est le repli sur l'appel canonique, pas la relecture des cinq.

## Vérification (la reproduction qui sépare une vraie garde d'une vacue)

```bash
# 1. Le lint refuse-t-il le défaut d'origine ? (réintroduire, puis :)
bash scripts/check-byte-slices.sh          # doit sortir non-zéro

# 2. Les tests échouent-ils sans le correctif ?
cargo test -p mika-agent --lib truncate_output
#   → assertion failed: self.is_char_boundary(new_len)  ← la panique de production, à l'identique

# 3. Et l'anti-vacuité tient dans les deux sens :
make check-byte-slices
```

Les duals « contenu sous la limite laissé intact » restent **verts** sous le code cassé — c'est leur rôle : ils ne prouvent pas le correctif, ils interdisent le correctif dégénéré qui tronquerait tout à zéro.

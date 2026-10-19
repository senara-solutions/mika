---
title: "Un contrôle négatif porte sur les champs dérivés de l'entrée, jamais sur un blob qui contient un identifiant aléatoire"
date: 2026-10-19
category: best-practices
module: mika-agent
problem_type: flaky_test
component: tests
severity: medium
tags: [tests, flake, negative-control]
applies_when:
  - Écrire un `assert!(!x.contains(...))` sur une valeur sérialisée
  - La valeur contrôlée embarque un UUID, un horodatage, un PID ou tout identifiant frappé par le moteur
  - Un rouge CI ne se reproduit pas et l'entrée affichée par l'assertion est correcte
---

# Un contrôle négatif exclut les identifiants aléatoires

## Le contexte

mika#2796. `eval::test_operator_iterate_2506::le_nombre_qui_voyage_est_le_numero_d_issue`
tient le terme de conception de mika#2506 : le nombre qui voyage dans l'`input`
de dispatch est le numéro d'**issue** (`mika#2503`), jamais celui de la PR
(`2504`), parce que `dispatch-lib.sh` lit `prompt: "<repo>#<N>"` comme un numéro
d'issue.

Le contrôle négatif lisait la chaîne **entière** :

```rust
let serialized = input.to_string();
assert!(!serialized.contains("2504"), …);
```

## La mesure

Le CI requis de la PR mika#2787 (run 39160328822, 2026-10-18) a rougi sur cet
`input` :

```
{"iteration_context":"rebase puis corrige le lint SIGPIPE","prompt":"mika#2503",
 "skill":"dev-pilot","task_id":"fcc4cf43-9b62-4b0d-8281-06046a7f2504"}
```

L'`input` était **correct** : `prompt` porte le numéro d'issue, `iteration_context`
le texte de l'opérateur. Le `task_id`, un UUID v4 frappé par
`Database::create_task` sans aucun rapport avec la requête, se terminait par
`…7f2504`.

Un UUID v4 porte 32 chiffres hexadécimaux, dont environ 20 décimaux, répartis en
cinq segments. Une sous-chaîne de 4 chiffres ne franchit pas un tiret, ce qui
laisse 17 fenêtres. Les segments 3 et 4 commencent par un chiffre fixe (`4`) ou
contraint (`[89ab]`), ce qui retire encore deux fenêtres pour un nombre comme
`2504`. La probabilité est donc de 15 × 16⁻⁴, soit environ **1 sur 4 400 par
run**. Un échantillon de 2 000 000 d'UUID v4 donne 1 sur ~4 500. C'est rare
pour une PR et certain pour un dépôt qui relance la CI à chaque push, et le run
39160328822 l'a rencontré. Ce rouge ne dit rien de la PR qu'il bloque, et il coûte une relance plus un
tour d'agent pour établir qu'il ne dit rien.

Ce n'est pas « un flake » au sens d'une course ou d'un réseau lent. Le défaut est
**déterministe dans le prédicat**. Le prédicat s'applique à une partie de la
valeur qui ne peut pas, par construction, transporter ce qu'il cherche. Et cette
partie était déjà pinnée exactement quelques lignes plus haut
(`assert_eq!(input["task_id"].as_str(), Some(task_id.as_str()))`).

## La règle

> Un contrôle négatif porte sur les champs **dérivés de l'entrée**. Il ne porte
> jamais sur un blob qui contient un identifiant frappé par le moteur : UUID,
> horodatage, PID, port éphémère, nonce.

Le contrôle négatif demande si l'entrée a fui dans la sortie. Seuls les champs
qui dérivent de l'entrée peuvent répondre à cette question. Un identifiant
aléatoire n'y répond pas : il ajoute du bruit, et ce bruit finit par satisfaire
le prédicat.

## La forme retenue

Trois propriétés, chacune porteuse (`champs_portant_le_nombre` dans
`crates/mika-agent/tests/eval/test_operator_iterate_2506.rs`) :

1. **Une liste d'exclusion nommée, pas une liste d'inclusion.**
   `CLES_NON_DERIVEES_DE_LA_REQUETE = ["task_id"]`. Tout champ non nommé est
   balayé, y compris un cinquième champ ajouté demain au `json!` de production.
   Une liste d'inclusion (`prompt` + `iteration_context`) cesserait de couvrir le
   nouveau champ sans rougir, ce qui est la forme que mika#2309 condamne.
2. **Le champ exclu est pinné ailleurs, exactement, et sa nature est vérifiée.**
   La valeur de `task_id` est vérifiée au byte par un `assert_eq!`. Cette
   égalité compare deux sorties du même `create_task`, donc elle ne prouve pas à
   elle seule que la valeur est aléatoire. Un `Uuid::parse_str` la complète : la
   prémisse de l'exclusion est vérifiée et non supposée. Exclure un champ qui
   n'est vérifié nulle part, c'est retirer de la couverture.
3. **L'exclusion ne peut pas devenir muette.** Le prédicat asserte
   `contains_key` sur chaque clé exclue. Si `task_id` est renommée en production,
   l'exclusion ne désigne plus rien, le balayage reviendrait sur l'UUID et le
   flake avec lui. L'assertion fait rougir le renommage à la place
   (`mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait`).

Le balayage lit le `to_string()` de chaque **sous-arbre** et pas seulement les
scalaires. Un champ qui deviendrait un objet reste donc couvert. Le message
d'échec nomme le champ porteur (`prompt="mika#2504"`) au lieu de jeter le blob
entier.

Le prédicat est un détecteur : son chemin de succès est « rien trouvé ». Il a donc
ses propres mesures. Le flake est figé avec l'UUID verbatim du run 39160328822 et
doit rester silencieux. Le nombre semé dans `iteration_context` puis dans
`prompt` doit être trouvé, et nommé.

## Ce qui a été refusé

**Fixer un `task_id` sans chiffres ambigus dans le test.** `NewTask` ne porte pas
d'identifiant, et `create_task` frappe l'UUID en interne. Cette variante demande
un siège d'injection dans le chemin de production pour réparer une assertion de
test. Le défaut est dans le prédicat. C'est le prédicat qui change, pas la
production.

## Pourquoi ce n'est pas un lint

La population est de **un**. `grep -rn '!.*to_string().contains' crates/` ne rend
qu'un seul contrôle négatif portant sur un blob sérialisé qui contient un
identifiant aléatoire. Le voisin `skills/manifest.rs`
(`!serialized.contains("[llm]")`) porte sur un littéral dans un manifeste
déterministe, sans chiffre ni identifiant.

Un lint pour un seul site est une garde qui ne refuse rien et qu'on paie à chaque
run (mika#2578, mika#2752). Il serait aussi difficile à écrire juste. Savoir si
une valeur sérialisée « embarque un identifiant aléatoire » est une propriété du
flux de données, pas de la syntaxe. Un grep sur `to_string().contains`
attraperait les cas déterministes inoffensifs et manquerait un `format!("{:?}")`
sur une struct qui porte un `Uuid`.

Le motif est donc documenté ici. Le lint s'arme le jour où la population le
justifie : trois sites, ou un deuxième rouge de cette forme. Ce document est
celui qu'on cite ce jour-là.

## Le risque dual, hors périmètre

Un contrôle **positif** sur sous-chaîne décimale (`digest.contains("2055")`) peut
être satisfait par hasard par un identifiant aléatoire. Il est alors vert à tort,
ce qui est pire qu'un rouge à tort. Au grooming de mika#2796, les sites existants
(`ready_label_handler.rs`, `ci_success_handler.rs`, `merge_gate_rerun.rs`) portent
tous sur un digest déterministe construit sur place, sans UUID. Le risque n'a pas
de sujet aujourd'hui. La même règle s'applique le jour où un digest embarque un
identifiant.

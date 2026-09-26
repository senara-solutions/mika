---
module: mika-agent/task_engine
tags: [backoff, arithmetic, rust, overflow, retry, testability, mika-2179]
problem_type: logic-error
category: best-practices
---

# `checked_shl` ne borne pas un recul exponentiel — il en cache le pire cas

## Le problème

Un recul exponentiel s'écrit naturellement `base * 2^(n-1)`, et en Rust la
tentation est de l'écrire par décalage :

```rust
let backoff = base.checked_shl(attempts - 1).unwrap_or(max).min(max);
```

Le `checked_` a l'air d'être la version prudente. Il ne l'est pas.
`u64::checked_shl` ne refuse **que** les décalages `>= 64` ; pour tout ce qui est
en dessous il fait un décalage ordinaire, qui **wrappe** — les bits qui sortent
par la gauche sont perdus, en silence.

Conséquence, avec `base = 60` :

| `attempts` | `60u64 << (attempts-1)` | ce que rend l'expression ci-dessus |
|---|---|---|
| 7 | 3840 | 3600 (plafonné — correct) |
| 40 | ~3,3e13 | 3600 (plafonné — correct) |
| **63** | **0** | **0** |
| 65 | `None` | 3600 (plafonné — correct) |

À la 63ᵉ tentative, `60 << 62` ne déborde pas vers une grande valeur : les
quatre bits de `60` sortent tous du mot, et le résultat est **zéro**. Le
`.min(max)` laisse passer le zéro sans broncher. Le code écrit alors un
`next_fire_at` dans le passé, la garde du balayage ne le retient plus, et la
boucle de reprise sans borne — celle que ce recul existait pour fermer —
recommence.

Le tail est atteignable. Avec le plafond horaire, arriver à 63 échecs
consécutifs demande environ **57 h** de panne ininterrompue : un week-end chez un
fournisseur, pas une impossibilité arithmétique. Et le mode de défaillance est
le pire qui soit : la borne disparaît exactement au moment où elle compte le
plus, et rien ne le dit.

## Ce qui a été mesuré

mika#2179 ajoutait un recul exponentiel sur la livraison des callbacks, après un
incident où la livraison de `800d739f` avait attendu 5 h 06 en reprenant le
verrou d'agent une fois par minute. Le compteur `delivery_attempts` est **non
borné par construction** : une ligne qui échoue continue de compter, et rien ne
la remet à zéro tant qu'elle n'est pas livrée. Le défaut a été trouvé en relisant
la formule, pas par un test — aucun test nominal n'atteint la 63ᵉ tentative, et
la revue de la propre arithmétique était le seul chemin.

## La règle

**Pour un exposant non borné, ne décalez pas : bornez le décalage, puis
saturez le produit.**

```rust
/// Ceiling on the exponent. `1u64 << 32` already saturates any sane base far
/// past any configurable `max`, so the clamp costs nothing and keeps the shift
/// provably in range.
const BACKOFF_MAX_SHIFT: u32 = 32;

fn delivery_backoff_secs(base: u64, max: u64, attempts: u32) -> u64 {
    let shift = attempts.saturating_sub(1).min(BACKOFF_MAX_SHIFT);
    base.saturating_mul(1u64 << shift).min(max)
}
```

Trois propriétés, et il en faut trois :

1. `min(BACKOFF_MAX_SHIFT)` rend le décalage **prouvablement** dans la plage —
   c'est ce qui élimine le wrapping, pas le `checked_`.
2. `saturating_mul` borne le produit par le haut au lieu de le tronquer.
3. `min(max)` est ce qui décide la valeur en pratique, dès la sixième
   tentative. Les deux premières gardes ne servent qu'au tail — et c'est
   précisément pour ça qu'on ne les découvre pas en exécutant le code.

## Le corollaire, qui vaut au-delà de l'arithmétique

**Extrayez la formule en fonction pure, sinon son tail n'est pas testable.**

Tant que le calcul vivait inline dans une méthode `async` qui écrit en base,
tester la 63ᵉ tentative demandait de fabriquer 63 échecs de livraison. Sortie en
fonction pure de trois `u64`, la même assertion tient en trois lignes :

```rust
#[test]
fn delivery_backoff_never_collapses_to_zero_on_a_long_outage() {
    for attempts in [40u32, 62, 63, 64, 65, 1000, u32::MAX] {
        assert_eq!(delivery_backoff_secs(60, 3600, attempts), 3600);
    }
}
```

Et notez la forme de l'assertion : elle affirme le **plancher**, pas l'absence de
panique. Un test qui se contente de vérifier que le code ne panique pas aurait
passé au vert sur la version fautive — zéro ne panique pas.

## Où chercher la même trappe

Partout où un compteur non borné pilote un exposant : reculs de reprise, fenêtres
de circuit-breaker, backoffs de reconnexion. Le signal est la paire
« compteur qui ne se remet jamais à zéro » + « décalage binaire ». Si les deux
sont présents, le tail est un zéro qui attend.

## Voir aussi

- `crates/mika-agent/src/task_engine/dispatcher.rs` — `delivery_backoff_secs`
  et ses tests, y compris celui du tail.
- `docs/plans/2026-09-04-001-fix-2179-livraison-callbacks-affamee-plan.md` — le
  plan dont ce défaut n'était pas un livrable ; il est apparu à l'écriture.

---
module: mika-agent
tags: [tests, flakiness, serial_test, env-vars, config, mika-2160]
problem_type: test-failure
category: test-failures
---

# `#[serial]` ne protège pas contre les tests parallèles

## Le symptôme

Un test de non-régression passait dans une exécution du module entier et
échouait dans une exécution ciblée de trois tests. Rien d'aléatoire en
apparence : il assertait un refus, et le refus n'arrivait pas.

## La cause

Un autre test posait `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT=2` dans
l'environnement du processus, sous `#[serial]`.

`#[serial]` sérialise un test **contre les autres `#[serial]`**. Il ne dit rien
des tests ordinaires, qui continuent de tourner en parallèle dans le même
processus — et l'environnement est global au processus. Le test qui lisait le
plafond en ligne voyait donc `2` au lieu du défaut `1`, et ne refusait pas.

Le piège tient à ce que la marque *ressemble* à une protection. Elle en est une,
mais son périmètre est l'ensemble des tests marqués, pas la suite.

## Ce qui rend ça pernicieux

**L'échec ne tombe pas sur le coupable.** Le test qui mute l'environnement passe
toujours ; c'est son voisin qui casse, et le voisin ne mentionne aucune variable
d'environnement. Sur une suite de plusieurs milliers de tests, ça se lit comme
un flake sans cause.

**Et ça grandit tout seul.** Chaque nouveau test qui touche au chemin concerné
devient sensible sans que personne l'ait décidé. Marquer les voisins
(`serial_test` offre `#[parallel]`, qui ne chevauche jamais `#[serial]`) marche,
mais c'est une discipline que le prochain test oubliera.

## Le geste

**Ne pas muter l'environnement du processus dans un test.** Extraire la décision
en fonction pure et la tester telle quelle :

```rust
// lu en ligne depuis l'environnement — non testable sans set_var
fn max_concurrent_for_class(class: &str) -> i64 { … }

// la décision, pure — c'est elle qu'on teste
pub fn class_cap_reached(active: i64, cap: i64) -> bool {
    cap > 0 && active >= cap
}
```

Il reste alors trois couvertures, et aucune ne touche à l'environnement :

1. **l'analyse** de la valeur (`parse_*(Some("2"))`) — la forme à trois paliers ;
2. **l'arithmétique** de la décision (`class_cap_reached`) — aux plafonds 1, 2, 0 ;
3. **le câblage**, par un test d'intégration qui passe par le vrai chemin **au
   défaut** : s'il régressait, ce test tomberait.

Ce que ça ne couvre pas : le chemin d'intégration réel à une valeur non-défaut.
C'est un manque assumé, et il se paie moins cher qu'une suite dont un test sur
mille ment. Quand le vrai chemin doit être exercé à une autre valeur, **passer
le réglage en paramètre** plutôt que par l'environnement — le dépôt le fait déjà
pour `ttl_secs` de `try_acquire_dispatch_slot`, et mika#2160 l'a fait pour
`max_slots`.

## Voir aussi

- `feedback_prompt_enforcement_fragile` — même principe : préférer la structure
  qui rend l'oubli impossible à la consigne que le prochain oubliera.

---
title: Un `#[cfg(test)]` d'item aveugle un scan qui tronque au premier littéral — et c'est la cardinalité qui le fait rougir
date: 2026-09-29
last_updated: 2026-09-29
category: best-practices
module: mika-agent/milestone_manager/liveness
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - Vous écrivez un scan de source qui doit ignorer le code de test
  - Vous coupez un fichier à `src.find("#[cfg(test)]")`
  - Vous hésitez entre `assert_eq!(n, 1)` et `assert!(n <= 1)` sur un compte de sites
  - Une garde neuve passe au vert du premier coup et vous ne savez pas si elle regarde
  - Un scan rend `0` sur un fichier dont vous savez qu'il porte le site cherché
---

# Un `#[cfg(test)]` d'item aveugle un scan qui tronque au premier littéral

## Le fait

mika#1990 livre une garde de source, `mika1990_le_battement_a_un_seul_ecrivain`,
qui compte trois cardinalités sur la moitié « production » de chaque `.rs` de
`crates/mika-agent/src` : un site de composition du motif, un site d'appel au
sink, **un site de câblage**. La moitié production était obtenue ainsi :

```rust
Some(match src.find("#[cfg(test)]") {
    Some(i) => src[..i].to_string(),
    None => src.to_string(),
})
```

Sur `spawn.rs`, le scan a rendu `wiring: 0` alors que le fichier porte
`liveness.beat(&cfg, liveness_sink.as_ref(), &outcome).await;`.

La cause est à la ligne 83 :

```rust
#[cfg(test)]
fn reset_spawn_guard_for_test() {
    MANAGER_SPAWN_GUARD.store(false, Ordering::SeqCst);
}
```

Un attribut **d'item isolé**, quatre lignes, en tête de fichier. Le câblage est
en ligne 472. La troncature a jeté **1 470 lignes de production** — dont le seul
site que la garde existe pour compter.

## Pourquoi c'est une classe et pas une coquille

La troncature repose sur une prémisse tacite : *le premier littéral
`#[cfg(test)]` marque le début du code de test*. C'est vrai du motif dominant —
`#[cfg(test)] mod tests { … }` en queue de fichier — et **faux de tout attribut
d'item**, qui est une forme parfaitement ordinaire pour un helper partagé par
plusieurs tests d'un même fichier.

Le dépôt documentait déjà ce trou, pour un **autre** parseur : `docs/egress/README.md`
§ *La limite de la coupure, mesurée plutôt que supposée* décrit le même
raccourci et le mesure en continu (`--audit-cut-holes`). La leçon n'avait pas
traversé jusqu'aux scans écrits en Rust dans les modules.

**Les deux parseurs n'ont pas la même règle de doute, et c'est là que ça se
joue.** Celui d'egress conclut « production » en cas de doute : une erreur de
découpe y produit une **déclaration de trop**, visible et corrigible. Celui-ci
conclut « test » : une erreur y produit un **silence**. Même raccourci, sens
d'erreur opposé, et un seul des deux est sûr.

## Ce qui a sauvé la garde

La garde n'est pas passée au vert en étant aveugle : elle a **rougi**. Parce que
son assertion est

```rust
assert_eq!(total.wiring, 1, "…");
```

et non `assert!(total.wiring <= 1, …)`.

C'est la différence entre les deux qui compte, et elle est facile à perdre :

| assertion | sur un scan aveugle | ce qu'elle mesure |
|---|---|---|
| `assert!(n <= 1)` | **passe** | « pas de second site » |
| `assert_eq!(n, 1)` | **rougit** | « pas de second site **et** j'ai vu le premier » |

Un scan qui n'affirme qu'une borne supérieure est satisfait par le vide. Il se
lit exactement comme un arbre propre — la classe mika#2205, appliquée au
détecteur lui-même plutôt qu'à son sujet. L'assertion d'exactitude est un
**contrôle positif intégré** : elle transforme la cécité en échec au lieu de la
laisser passer pour une santé.

Corollaire, moins intuitif : une garde neuve qui passe du premier coup mérite
d'être soupçonnée. Ici l'échec était le bon signal, et le prendre pour une
régression du correctif aurait envoyé chercher au mauvais endroit.

## Le remède

Couper sur la **région** de test, jamais sur le premier littéral :

```rust
fn find_test_region_start(src: &str) -> Option<usize> {
    const ATTR: &str = "#[cfg(test)]";
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(ATTR) {
        let at = from + rel;
        let rest = src[at + ATTR.len()..].trim_start();
        if rest.starts_with("mod ")
            || rest.starts_with("pub mod ")
            || rest.starts_with("pub(crate) mod ")
        {
            return Some(at);
        }
        from = at + ATTR.len();
    }
    None
}
```

**La direction de l'erreur décide de la forme du remède.** Élargir ne peut faire
voir que *plus* de sites : au pire un faux positif bruyant, qu'on constate et
qu'on traite. Rétrécir rend le scan silencieusement inerte. Donc on élargit, et
le coût nommé est qu'un helper `#[cfg(test)]` d'item qui écrirait le site
cherché serait compté et ferait rougir — ce qui est le bon sens de l'erreur : on
le déplace dans le `mod tests`.

Ce remède **compose** avec celui de mika#2321 (`is_test_source_path`), qui couvre
l'autre moitié du problème : un module de test *extrait* ne porte aucun littéral
`#[cfg(test)]`, donc la troncature seule le scannerait entièrement comme de la
production. Les deux sont nécessaires et ne se remplacent pas.

## Et le contrôle qui l'épingle

Corriger la coupure sans l'épingler laisserait la prochaine « simplification »
la restaurer en silence. Le contrôle doit porter sur la **conséquence** — un site
de production invisible — et sur les deux sens :

```rust
#[test]
fn mika1990_un_cfg_test_ditem_ne_masque_pas_la_production() {
    // Forme exacte de spawn.rs : helper de test en tête, câblage plus bas.
    let src = concat!(
        "static GUARD: AtomicBool = AtomicBool::new(false);\n",
        "#[cfg(test)]\n",
        "fn reset_guard_for_test() { GUARD.store(false, Ordering::SeqCst); }\n",
        "pub fn boucle() { liveness.beat(&cfg, sink, &outcome).await; }\n",
    );
    let prod = production_half(Path::new("spawn.rs"), src).expect("production");
    assert!(prod.contains(".beat("), "…le scan est aveugle sur le fichier même …");

    // Le `mod tests`, lui, doit couper — sinon tout fichier testé rougit.
    // …
}
```

Le second sens n'est pas décoratif : une coupure qui ne couperait plus rendrait
la garde rouge sur chaque fichier possédant des tests, et une garde rouge à la
naissance finit muselée.

## À retenir

- `src.find("#[cfg(test)]")` n'est pas « le début des tests ». C'est « le premier
  attribut de test », ce qui est un autre énoncé, et la différence se paie en
  lignes de production jetées.
- Sur un compte de sites, écrivez `assert_eq!(n, k)`, pas `assert!(n <= k)`. La
  borne supérieure est satisfaite par le vide ; l'égalité porte son propre
  contrôle positif.
- Quand un raccourci de découpe est déjà documenté quelque part dans le dépôt,
  la question à se poser n'est pas « est-ce le même parseur ? » mais « ai-je la
  même règle de doute ? ». Ici les deux parseurs partageaient le raccourci et
  divergeaient sur le sens de l'erreur, et c'est le sens de l'erreur qui décide
  si le trou est bénin ou aveuglant.

## Voir aussi

- `crates/mika-agent/src/milestone_manager/liveness.rs` — `find_test_region_start`,
  `production_half`, et les deux contrôles de bonne foi.
- `docs/egress/README.md` § *La limite de la coupure, mesurée plutôt que supposée*
  — le même raccourci, mesuré en continu, avec la règle de doute inverse.
- `docs/solutions/prompt-enforcement-structural-guards.md` — pourquoi ces gardes
  sont des scans de source et non des tests de comportement.

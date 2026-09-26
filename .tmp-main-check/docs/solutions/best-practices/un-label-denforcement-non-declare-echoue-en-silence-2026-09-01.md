---
title: Un label d'enforcement non déclaré échoue en silence — vérifiez le registre, pas le code appelant
date: 2026-09-01
last_updated: 2026-09-01
category: best-practices
module: mika-agent/auto_pull
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Un chemin de code applique un label GitHub pour arrêter, exclure ou rendre la main sur un ticket
  - Vous lisez un prédicat d'exclusion qui teste `l.name == "<quelque-chose>"`
  - Un mécanisme d'arrêt « existe » dans le code mais l'état qu'il devrait produire ne s'observe jamais
---

# Un label d'enforcement non déclaré échoue en silence

## Le problème

`auto_pull` s'appuyait sur `operator-review` pour retirer un ticket de la boucle :
`gh_apply_label(.., "operator-review")`, puis retrait de `ready`, puis commentaire.
Le prédicat d'exclusion `is_feeder_excluded` testait ce même label. Douze
références dans le module. Deux tickets (mika#1824, mika#2020) construits dessus.

**Le label n'existe pas.** Ni dans `gh label list`, ni dans `.github/labels.yml`.

```
gh issue edit --add-label failed for #2117:
  'operator-review' not found
```

Quarante-huit fois dans `server.log`, sur #2117, #1651, #1403.

La conséquence dépasse « un label qui ne se pose pas ». Le code retire `ready`
**seulement après** que le label a été appliqué. L'application échouant, `ready`
n'est jamais retiré : le ticket disjoncté reste dans le bassin, indéfiniment.
L'arrestation est un no-op, et rien ne le dit au-dessus de `WARN`.

## Pourquoi ça a tenu des semaines

Les trois propriétés qui rendent cette classe invisible :

1. **Le code appelant est correct.** Il gère `Err`, il journalise, il abandonne
   proprement. Une relecture du diff ne voit rien.
2. **L'échec est un `warn!` parmi des milliers.** Le seul niveau qui aurait
   arrêté un regard est celui qui n'a pas été choisi.
3. **Le registre n'est pas dans le diff.** `.github/labels.yml` est un fichier de
   données que personne ne relit en revoyant du Rust. Le lien entre la chaîne
   `"operator-review"` et sa déclaration n'existe que dans la tête de l'auteur.

C'est la même forme que `feedback_prompt_enforcement_fragile` : un mécanisme
énoncé mais non structurellement lié à ce qui le rend vrai.

## Ce qu'il faut faire

**Le contrôle est de lire le registre, pas le code.** Trois questions, dans cet
ordre :

```bash
# 1. Le label existe-t-il vraiment côté GitHub ?
gh label list --repo senara-solutions/mika --limit 200 | grep -x 'operator-review.*'

# 2. Est-il déclaré dans la source de vérité ?
grep -n '^- name: operator-review' .github/labels.yml

# 3. CONTRÔLE POSITIF — la commande sait-elle trouver un label qui existe ?
grep -n '^- name: ready' .github/labels.yml
```

La troisième ligne n'est pas décorative. Sans elle, un `grep` qui ne trouve rien
parce que le format du fichier a changé se lit exactement comme un label absent.

**Puis, structurellement**, un test qui lit le registre :

```rust
#[test]
fn test_refusal_label_is_declared_in_labels_yml() {
    let yml = include_str!("../../../.github/labels.yml");
    let declared = |name: &str| yml.contains(&format!("- name: {name}"));

    assert!(declared("ready"));                                  // contrôle positif
    assert!(!declared("a-label-nobody-has-ever-declared"));       // contrôle négatif
    assert!(declared(REFUSAL_LABEL));                             // l'assertion
}
```

Le test coûte quatre lignes et ferme la classe entière : n'importe quel label
d'enforcement ajouté plus tard sans déclaration casse la CI au lieu de casser la
production six semaines après.

## Les deux corollaires

**Choisissez un label qui existe déjà quand il en existe un.** `operator-gated`
était déclaré (`.github/labels.yml:106`) avec la description *« Groomed work
requiring operator-host-time. Distinct from parked/blocked. No ready label. »* —
littéralement l'état qu'un refus de promotion crée. Le réutiliser fait marcher la
porte le jour du déploiement, sans dépendre d'une synchro de labels.

**Un marqueur que le prédicat d'exclusion ignore n'est pas un marqueur.** Poser
`operator-gated` sans l'ajouter à `is_feeder_excluded` aurait produit une porte
qui refuse, marque, puis re-mesure la même branche au tick suivant, pour toujours.
Le label et le prédicat sont une seule décision, pas deux.

**Et quand l'application échoue quand même, faites du bruit avec sa propre clé.**
Pas un `warn!` de plus dans le flux : un `error!` sous un nom d'événement dédié
(`auto_pull_refusal_marker_unavailable`) plus une ligne d'audit. Ce qui a manqué
en 2026-08 n'était pas la détection — l'erreur était journalisée les 48 fois —
c'était qu'elle soit *trouvable*.

## Référence

- mika#2123 (la porte de promotion), qui a découvert la classe en voulant réutiliser le geste
- mika#2020 / mika#1824 (les tickets dont le mécanisme d'arrêt était inerte)
- `crates/mika-agent/src/auto_pull.rs` — `REFUSAL_LABEL` et sa doc portent la mesure
- Voisin : `docs/solutions/prompt-enforcement-structural-guards.md`

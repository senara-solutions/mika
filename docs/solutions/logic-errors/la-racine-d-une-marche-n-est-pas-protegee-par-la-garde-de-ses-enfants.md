---
title: "La racine d'une marche n'est pas protégée par la garde de ses enfants — et un marqueur de cache ne s'authentifie pas par son nom"
date: 2026-10-01
category: logic-errors
module: crates/mika-agent/src/worktree_reaper.rs
problem_type: logic_error
component: mika-agent
severity: critical
symptoms:
  - "Un `.pilot-scratch` qui est un lien vers celui d'un worktree voisin fait entrer les caches du voisin dans la population purgée du worktree courant"
  - "Le cache d'un worktree voisin vivant est supprimé sans que ses processus aient été interrogés (P3 ne regarde que le worktree courant)"
  - "Un fichier simplement nommé `CACHEDIR.TAG`, ou un lien vers le marqueur d'un autre cache, rend tout son répertoire jetable, brouillons compris"
  - "Tous les tests verts : la garde par enfant (`symlink_metadata`) était présente et testée"
root_cause: missing_validation
resolution_type: code_fix
related_components:
  - discover_build_dirs
  - build_dir_is_inside_worktree
  - is_cargo_build_dir
  - purge_stale_target_dirs
tags: [worktree_reaper, symlink, read-dir, walk-root, data-loss, cachedir-tag, marker, destructive-write, code-review]
related_issues:
  - mika#2619
  - mika#2621
---

# La racine d'une marche n'est pas protégée par la garde de ses enfants

## Problème

mika#2619 a élargi le bras de purge du `worktree_reaper` : en plus de
`<worktree>/target`, `discover_build_dirs` marche sous `<worktree>/.pilot-scratch/`
(profondeur bornée) et retient tout répertoire porteur d'un marqueur de cache
(`.rustc_info.json` ou `CACHEDIR.TAG`). La marche refusait de suivre un lien
**pour chaque enfant** (`symlink_metadata`), mais ouvrait la **racine** par
`std::fs::read_dir(<wt>/.pilot-scratch)`, qui suit un lien. Un `.pilot-scratch`
lié à celui d'un worktree voisin faisait donc entrer les caches du voisin dans
la population du worktree courant. Classe : perte de données (P1).

Volet secondaire, même leçon : `is_cargo_build_dir` reconnaissait un cache par
`dir.join(marker).is_file()`. `is_file()` suit les liens, et le contenu de
`CACHEDIR.TAG` n'était jamais lu : le **nom** du fichier suffisait à autoriser
un `remove_dir_all`.

## Symptômes

Aucun en production : le défaut a été trouvé en revue, avant merge (PR #2621,
en draft à cette date). Le scénario, reproduit par le test V7 ter :

1. le worktree A (piège) a un `.pilot-scratch` qui est un lien vers
   `<B>/.pilot-scratch` ;
2. `discover_build_dirs(A)` rend `<A>/.pilot-scratch/<x>`, qui *est* le cache
   de B ;
3. P3 (processus vivants) n'interroge que les processus de A ;
4. la garde tardive `build_dir_disposition` accepte : après canonicalisation,
   le chemin est sous **un** worktree géré (B) et porte un marqueur ;
5. le cache de B est supprimé sans que personne ait regardé si B vivait.

## Ce qui n'a pas marché

- **La garde par enfant.** Elle était là, correcte et testée
  (`mika2619_v7_la_marche_ne_suit_pas_un_lien`). Elle donnait l'impression que
  « la marche ne suit pas les liens », alors que l'entrée de la marche n'était
  pas soumise à la règle. Les tests créaient tous un `.pilot-scratch` réel.
- **La garde tardive « sous un worktree géré ».** `build_dir_disposition`
  canonicalise et vérifie `is_managed_worktree_path` : elle prouve que le
  chemin est sous *un* worktree géré, pas sous *celui dont P3 a interrogé les
  processus*. Pour un lien vers un voisin géré, elle passe.
- **La revue du pilote.** Le pilote du dispatch a sauté `/ce:code-review` : la
  skill écrit ses artefacts sous `/tmp`, et la permission-policy de la session
  refuse tout `Write` hors du worktree. La skill n'a pas de repli interne, donc
  le pilote est passé à un « scan manuel explicite du diff », consigné dans la
  PR sous « Code review: skipped (ce-code-review unavailable) ». Ce scan a
  trouvé un vrai défaut (`t7_is_needed`), mais **pas** ce P1.

## Solution

Deux correctifs, sur la branche de la PR #2621 (non mergée à cette date ; les
SHA cités sont ceux de la branche et peuvent changer au merge : la PR fait foi).

**1. La racine passe la même règle que ses enfants** (commit `e85d5a81`,
`discover_build_dirs`) :

```rust
// Avant : read_dir suit un lien vers le .pilot-scratch d'un voisin.
let mut level = vec![worktree.join(PILOT_SCRATCH_DIRNAME)];

// Après : la racine doit être un répertoire réel, jamais un lien.
let scratch = worktree.join(PILOT_SCRATCH_DIRNAME);
let scratch_is_real_dir = scratch
    .symlink_metadata()
    .is_ok_and(|m| !m.is_symlink() && m.is_dir());
let mut level = if scratch_is_real_dir { vec![scratch] } else { Vec::new() };
```

**2. Défense en profondeur : l'appartenance au bon worktree, avant le verrou**
(même commit). `build_dir_is_inside_worktree` canonicalise les deux côtés et
exige que le répertoire soit strictement sous le worktree dont P3 a interrogé
les processus. Elle est évaluée avant `acquire_cargo_build_locks` ; un refus
tombe sous `outside_managed_root`, sans motif nouveau.

```rust
pub fn build_dir_is_inside_worktree(dir: &Path, worktree: &Path) -> bool {
    match (std::fs::canonicalize(dir), std::fs::canonicalize(worktree)) {
        (Ok(d), Ok(w)) => d != w && d.starts_with(&w),
        _ => false,
    }
}

let owned = build_dir_is_inside_worktree(target, Path::new(&candidate.worktree_path));
let acquisition = match build_dir_disposition(target) {
    Ok(()) if !owned => Err(PURGE_REASON_OUTSIDE_MANAGED_ROOT),
    Ok(()) => match acquire_cargo_build_locks(target) { /* … */ },
    Err(reason) => Err(reason),
};
```

Cette seconde garde couvre aussi un composant **parent** remplacé par un lien
entre la découverte et la suppression. Elle réduit la fenêtre TOCTOU sans la
fermer (résidu nommé dans la PR).

**3. Un marqueur se lit, il ne se nomme pas** (commit `914ff7a7`,
`is_cargo_build_dir`) :

```rust
// Avant : un nom suffit, et is_file() suit les liens.
dir.join(CARGO_INFO_MARKER).is_file() || dir.join(CACHEDIR_TAG_MARKER).is_file()

// Après : fichier régulier, et CACHEDIR.TAG doit porter la signature de la spec.
let is_regular_file = |name: &str| {
    dir.join(name).symlink_metadata().is_ok_and(|m| m.file_type().is_file())
};
if is_regular_file(CARGO_INFO_MARKER) { return true; }
if !is_regular_file(CACHEDIR_TAG_MARKER) { return false; }
let mut head = [0u8; CACHEDIR_TAG_SIGNATURE.len()];
std::fs::File::open(dir.join(CACHEDIR_TAG_MARKER))
    .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
    .is_ok_and(|()| head == CACHEDIR_TAG_SIGNATURE)
```

`CACHEDIR_TAG_SIGNATURE` vaut `Signature: 8a477f597d28d172789f06886806bc55`,
l'en-tête imposé par la spécification Cache Directory Tagging
(<https://bford.info/cachedir/>), celui qu'écrit cargo. Toute lecture
impossible rend `false` : la population ne peut que rétrécir. Résidu nommé : le
contenu de `.rustc_info.json` n'est pas validé (fichier régulier seulement).

## Pourquoi ça marche

Une marche « qui ne suit pas les liens » a **deux** points d'entrée sur le
système de fichiers : chaque enfant énuméré, et la racine qu'on passe à
`read_dir`. `symlink_metadata` sur les enfants ne dit rien de la racine, parce
que `read_dir` résout le lien avant que la boucle ne voie la moindre entrée.
Tous les enfants rendus sont alors « réels » du point de vue de la garde, et
pourtant tous appartiennent à un autre arbre.

La seconde garde change la question posée au dernier moment. « Ce chemin
est-il sous un worktree géré ? » ne prouve rien sur la vivacité. La bonne
question est « ce chemin est-il sous le worktree dont j'ai vérifié les
processus ? ». La preuve de vivacité (P3) et la cible de la suppression doivent
porter sur le **même** objet, après canonicalisation.

Pour le marqueur : il est ce qui autorise un `remove_dir_all` **sans** preuve
par le nom (`target`). Il doit donc être au moins aussi difficile à produire
par accident que ce qu'il remplace. Un nom de fichier ne l'est pas ; la
signature de la spécification existe précisément pour qu'un fichier homonyme ne
soit pas pris pour une déclaration de cache.

## Prévention

- **Toute marche destructrice soumet sa racine à la règle de ses enfants.**
  Quand on écrit « les liens ne sont jamais suivis », chercher chaque appel qui
  ouvre un chemin (`read_dir`, `canonicalize`, `File::open`, `metadata`,
  `is_file`, `is_dir`, `exists`) et vérifier qu'il ne suit pas un lien. Les
  méthodes `Path::is_file`, `is_dir`, `exists` et `metadata` suivent toutes les
  liens ; seul `symlink_metadata` ne le fait pas.
- **La preuve de vivacité et la cible portent sur le même objet canonique.**
  Une garde « sous un X géré » ne remplace pas « sous *ce* X ». Les vérifier
  toutes les deux, la seconde juste avant l'acte irréversible.
- **Un marqueur qui autorise une suppression se lit.** Fichier régulier
  (`symlink_metadata`), contenu signé quand la spécification en définit un.
  Les fixtures écrivent un contenu réel : `fake_build_dir` écrivait
  `Signature: factice`, ce qui aurait rendu vert un test de signature écrit
  contre elle.
- **Tests de contrôle par terme, vus rouges avant le correctif.** V7 ter
  (`mika2619_v7_la_racine_de_la_marche_nest_pas_suivie`) avec contrôle positif
  (le propriétaire purge toujours son cache) ; V7 quater
  (`mika2619_v7_le_repertoire_appartient_a_son_worktree`) dans les deux sens ;
  V7 quinquies (`mika2619_v7_le_marqueur_ne_se_falsifie_pas`) avec tag non
  signé, tag lié, info liée et un tag signé en contrôle positif. Mutation
  terme par terme : chaque moitié du P1, seule, protège le voisin ; sans les
  deux, `purged: 1`. Le terme lien et le terme signature du marqueur rougissent
  chacun séparément. Le tag non signé fait au moins la longueur de la
  signature, sinon le test épinglerait « trop court » et non « mauvaise
  signature ».
- **Un code destructeur ne sort pas sans vraie revue multi-agents.** Ici le
  pilote a remplacé `/ce:code-review` par un scan manuel parce qu'un `Write`
  sous `/tmp` était refusé. Seule une revue multi-agents lancée ensuite par
  spawn (huit relecteurs et un validateur, lentille adversariale comprise) a
  trouvé ce P1 et promu le marqueur falsifiable. Pour une PR qui peut supprimer
  des données, « Code review: skipped » dans le corps est un signal bloquant :
  relancer la revue hors du bac à sable avant de considérer la PR prête.

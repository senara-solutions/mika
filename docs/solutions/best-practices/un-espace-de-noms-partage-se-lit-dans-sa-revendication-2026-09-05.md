---
module: mika-a2a/jsonrpc
tags: [jsonrpc, a2a, error-codes, namespace, spec-verification, test-design, mika-2163]
problem_type: logic-error
category: best-practices
---

# Un espace de noms partagé se lit dans ce qu'il **revendique**, pas dans ce qu'il a déjà utilisé

## Le problème

mika#2163 devait remplacer `-32603 "Agent is busy"` — un code dont le sens défini
est « le serveur a échoué » — par un code qui dit la contention. Le plan avait
raisonné le choix, et le raisonnement était soigné :

> JSON-RPC 2.0 réserve `-32000..-32099` aux erreurs serveur définies par
> l'implémentation ; la spec A2A s'y taille une plage **en numérotant depuis
> -32001 vers le bas** (-32001 `TaskNotFound` … -32007
> `AuthenticatedExtendedCardNotConfigured`). `-32099` est l'extrémité opposée de
> la même plage : la collision exigerait que la spec publie 92 codes de plus.

Le modèle est celui-ci : *l'espace occupé par le voisin, c'est ce qu'il a
attribué*. Il conduit à prendre le numéro le plus éloigné de son front d'avance,
et à mesurer la sécurité en distance — 92 codes de marge.

La spec publiée dit autre chose, en une phrase :

> « A2A-specific errors use codes in the range `-32001` to `-32099`. »
> — `a2aproject/A2A@main`, `docs/specification.md` §9.5

Elle ne numérote pas vers le bas depuis -32001 : elle **revendique toute la
bande**. `-32001..-32009` sont attribués aujourd'hui (dont `-32008` et `-32009`,
postérieurs à la v0.3 qu'implémente ce dépôt). `-32099` n'est donc pas
l'extrémité libre d'une plage voisine — c'est un numéro **au milieu de l'espace
de noms d'autrui**, en attente d'attribution. La marge de 92 codes n'était pas
une marge : c'était la longueur de ce qu'on traversait.

## Ce qui l'a rattrapé

Pas un test. Le plan lui-même, qui avait écrit sa propre clause de vérification
au lieu de traiter son raisonnement comme acquis :

> Le dépôt implémente A2A **v0.3** et ne vendorise pas la spec ; la plage
> assignée n'est donc pas vérifiable hors ligne depuis ce plan. **Avant merge**,
> l'implémenteur confronte `-32099` à la spec A2A publiée. Si le code s'avère
> pris ou réservé, il descend dans la plage libre la plus haute — la conception
> ne change pas, seul le nombre change.

Deux choses rendent cette clause efficace, et aucune n'est le fait d'avoir
« pensé à vérifier » :

1. **Elle nomme ce que le plan ne peut pas savoir**, et pourquoi (la spec n'est
   pas dans le dépôt). Une clause qui dit « vérifier que c'est bon » ne se
   déclenche jamais ; celle-ci dit quelle question poser à quelle source.
2. **Elle pré-décide la disposition.** « Seul le nombre change » : au moment où
   la mesure contredit le plan, il n'y a rien à arbitrer, donc rien à remonter,
   donc aucune tentation de laisser passer pour ne pas rouvrir un plan validé.

Retenu : `AGENT_BUSY = -32000` — le seul code de la bande « erreur serveur
définie par l'implémentation » que la spec A2A ne revendique pas, et le créneau
sémantique exact de ce qu'est une contention d'agent.

## Le test qui n'aurait rien vu, et celui qu'il fallait

Le plan prévoyait un test T7 : « `AGENT_BUSY` n'entre en collision avec aucun
code A2A du module ». Il passe sur `-32099`. Il passerait sur n'importe quel
numéro libre du module. **L'unicité locale ne peut pas voir une collision
d'espace de noms**, parce que le propriétaire de l'espace n'est pas dans le
module — il est sur le fil, en face.

Le module porte donc deux tests, et le second est celui qui porte le risque :

```rust
#[test]
fn agent_busy_collides_with_no_other_code_in_this_module() { /* unicité locale */ }

#[test]
fn agent_busy_is_outside_the_a2a_reserved_range() {
    const A2A_RESERVED_HIGH: i32 = -32001;
    const A2A_RESERVED_LOW: i32 = -32099;
    assert!(!(A2A_RESERVED_LOW..=A2A_RESERVED_HIGH).contains(&AGENT_BUSY));
}
```

Le second épingle une **citation**, pas une intuition : la bande revendiquée est
écrite en constantes avec sa source en commentaire, donc un futur passage qui
voudrait « récupérer » un code de cette plage doit d'abord effacer la phrase de
la spec — ce qui se voit en revue, contrairement à un choix de nombre.

## La règle

**Avant de prendre un numéro, un préfixe, un port, un code d'erreur ou un nom
dans un espace partagé : lire ce que le propriétaire de l'espace revendique, pas
ce qu'il a déjà consommé.** Le front d'attribution avance ; la revendication,
elle, est la frontière. Mesurer sa sécurité en distance au front, c'est mesurer
en temps qu'il reste avant la collision — et personne ne relit ce calcul le jour
où le front l'a franchi.

Corollaire de conception de test : **quand la contrainte vient d'un tiers, le
test doit citer le tiers.** Un test qui ne regarde que le dépôt vérifie la
cohérence de nos choix entre eux, ce qui est utile et n'est pas la question.

## Où ça vit

- `crates/mika-a2a/src/jsonrpc.rs` — `AGENT_BUSY` et ses deux tests.
- `docs/plans/2026-09-05-001-fix-2163-a2a-attente-bornee-au-lieu-du-refus-sec-plan.md`
  §3.3 — la clause pré-merge, la mesure du 2026-09-05, et la disposition exécutée.

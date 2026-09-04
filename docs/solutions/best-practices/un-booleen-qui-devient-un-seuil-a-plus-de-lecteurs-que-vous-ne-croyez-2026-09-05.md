---
module: mika-agent/db, mika-agent/task_engine, mika-agent/skills
tags: [dispatch, concurrency, config, refactor, predicate-drift, mika-2160, mika-1163]
problem_type: logic-error
category: best-practices
---

# Un booléen qui devient un seuil a plus de lecteurs que vous ne croyez

## Le problème

Transformer une constante en réglage paraît local. Ça ne l'est pas quand la
constante était **1**, parce qu'un plafond de 1 ne s'écrit pas comme un nombre :
il s'écrit comme un booléen. `SELECT ... LIMIT 1`, `COUNT(*) > 0`,
`PRIMARY KEY (a, b)` — trois formes qui ne ressemblent pas à un plafond et qui
en sont un. Un `grep` sur le nom du réglage n'en trouve aucune : le réglage
n'existait pas.

mika#2160 rendait paramétrable le nombre d'implémentations simultanées. Le
grooming avait identifié **deux** endroits qui tenaient le plafond. La revue de
code en a trouvé un **troisième**, et il portait la panne la plus gênante.

## Ce que ça donne concrètement

Les trois lecteurs de mika#2160, et ce que chacun aurait cassé seul :

| forme | où | si on l'oublie |
|---|---|---|
| `SELECT … LIMIT 1` (prédicat d'existence) | la garde de dispatch | le réglage refuse encore à 1 |
| `PRIMARY KEY (agent_id, dispatch_class)` | le bail `dispatch_slot_leases` | le réglage est **accepté, journalisé, et sans effet** |
| `COUNT(*) > 0` | la promotion des dispatchs **différés** | les dispatchs neufs passent ; un différé attend le retour à **zéro** |

Le troisième est le pire parce qu'il est asymétrique et silencieux : la boucle
paraît accélérer, et la file de reprise reste bloquée derrière l'ancienne
sémantique. Personne ne le voit avant de se demander pourquoi un ticket différé
n'est jamais reparti.

## Le signal qu'on avait déjà

Le dépôt portait déjà le nom de cette classe, dans un commentaire, au-dessus de
la fonction en question :

> *« Same predicate as the periodic backstop in engine.rs — see mika#1163 for
> the asymmetric-predicate-drift failure class. »*

mika#1163 avait déjà coûté un incident sur exactement cette paire de prédicats.
Le commentaire disait « garde-les synchronisés » ; l'implémentation de #2160 en
a désynchronisé un troisième. **Un commentaire « keep in sync » est une alarme
qui n'a pas de détecteur.** Quand on en croise un, c'est le moment de chercher
les autres lecteurs, pas de faire confiance à la consigne.

## Le geste

Avant de rendre configurable une constante qui valait 1, chercher les **formes**
du plafond, pas son nom :

```bash
# le prédicat d'existence
grep -rn "LIMIT 1" --include="*.rs" crates/ | grep -i "<le domaine>"
# le booléen d'occupation
grep -rn "COUNT(\*)" --include="*.rs" crates/ | grep -i "<le domaine>"
grep -rnE "fn has_(any_)?[a-z_]*\(" --include="*.rs" crates/ | grep -i "<le domaine>"
# la clé unique, qui est un plafond sans le dire
grep -rn "PRIMARY KEY\|UNIQUE" --include="*.rs" crates/ | grep -i "<le domaine>"
```

Puis, pour chaque site trouvé, la question qui tranche : **« si le plafond
passait à 2, ce code rendrait-il encore la bonne réponse ? »** Un site qui ne la
rend pas doit bouger dans le même commit, ou le réglage est décoratif — et un
réglage décoratif est pire que pas de réglage, parce qu'il est accepté et
journalisé.

## Deux corollaires vérifiés sur ce ticket

**Le défaut ne doit rien payer.** La garde comptait avant de comparer, ce qui
ajoutait un aller-retour DB sur le chemin par défaut. Réparé en gardant la
requête d'origine quand le plafond vaut 1 : le comptage n'est payé que par qui a
demandé N>1. Un réglage désactivé ne devrait pas laisser de trace de son
existence dans le chemin nominal.

**La bascule vers le bas compte autant que vers le haut.** Un plafond abaissé
pendant que des ressources sont prises laisse l'état au-dessus du nouveau
plafond. Il faut compter le vivant, pas seulement chercher une place libre —
sinon un index bas qui se libère est distribué alors qu'un index haut vit
encore.

## Voir aussi

- `docs/solutions/best-practices/verify-a-bulk-rewrite-with-a-guard-that-stays-2026-08-29.md`
  — même famille : la vérification qui reste vaut mieux que la consigne.
- mika#1163 — le premier passage de cette classe de dérive.
- mika#2160 — le ticket qui l'a rebâtie, et les trois sites.

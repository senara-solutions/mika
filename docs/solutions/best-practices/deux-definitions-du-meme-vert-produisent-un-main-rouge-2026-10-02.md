---
module: mika-agent
tags: [merge-gate, ci, branch-protection, fail-closed, divergence]
problem_type: architecture
---

# Deux définitions du même vert produisent un main rouge, et `--auto` en est une

**Mesuré :** 2026-10-02 (mika#2617, phase A). **Incident fondateur :**
2026-10-01, PR senara-solutions/mika#2614 mergée par le moteur sur du rouge ;
`main` rouge pendant des heures, p0 mika#2616.

---

## Le fait

`pr_merge_with_gate` lisait `gh pr checks --required` et refusait sur un bucket
`fail`/`cancel` **parmi les checks requis par la protection de branche**. La
recette orchestrateur, elle, ne merge jamais avec un check rouge, requis ou non.

Deux définitions du mot « vert » coexistaient donc dans la maison, et le défaut
n'était pas qu'elles existaient — c'est que **la plus laxiste des deux tenait la
porte automatique**. PR #2614 avait `Egress Uniqueness Lint` et
`Egress Manifest Lint` en FAILURE sur sa tête ; aucun des deux n'était requis ;
le moteur a mergé.

## La forme générale

> Quand deux composants décident de la même chose à partir de deux définitions,
> le défaut ne se manifeste pas à l'endroit où ils divergent : il se manifeste
> **chez le consommateur du plus permissif**, et il se lit comme une panne de
> l'autre.

Lire les logs de PR #2614 ne montre aucune erreur. La porte a fait exactement ce
qu'elle était écrite pour faire. Ce qui manquait n'était pas un garde-fou, c'était
l'**unicité** de la définition.

Trois occurrences antérieures de la même classe dans ce dépôt :

| ticket | les deux définitions |
|---|---|
| mika#2158 (`grooming_marker`) | deux regex du verdict de grooming, dont une commentée « Mirrors … » et restée en retard de deux élargissements |
| mika#2092 (sièges de dispatch) | `KNOWN_DISPATCH_SEATS` en Rust ↔ `dispatch:*` dans `labels.yml` |
| mika#2276 (budget par outil) | le budget déclaré par un manifeste ↔ le maximum appliqué par le moteur |

## Le piège spécifique : `--auto` est une définition, pas un réglage

Le remède évident — « lire tous les checks » — est **insuffisant à lui seul**, et
c'est la partie qui se rate.

`gh pr merge --auto` demande à GitHub de merger « quand les checks passent ».
Pour GitHub, « les checks » signifie **les checks requis**. Donc une porte qui
lit tous les checks, constate qu'un est en attente, et arme `--auto`, vient de
rendre la décision à la définition qu'elle venait d'abandonner. Le correctif
serait resté **inerte sur ce chemin**, avec tous les tests au vert.

La règle à retenir :

> Déléguer l'attente, c'est déléguer la décision. Un composant qui ne veut pas
> d'une définition étrangère du vert ne peut pas sous-traiter à qui la porte.

## Ce qui a tenu, et pourquoi

**L'incapacité plutôt que la consigne.** `run_gh_merge` a **perdu son paramètre
`auto`**. Le drapeau n'est pas déconseillé, il est inexprimable, et le
compilateur a forcé les trois sites d'appel restants. Un scan lexical sur un
`bool` positionnel aurait été fragile là où le type est sûr (doctrine mika#1991).

**Un lecteur unique, et l'héritage gratuit.** Le flag vivait à un seul endroit
(`run_gh_checks_raw`), et quatre consommateurs en descendent — tous décidant via
`classify_checks`, qui n'a pas bougé d'une ligne. Retirer un `--required` les a
alignés tous les quatre. **C'est le lecteur unique qui rend le cœur du correctif
minuscule**, et mika#2455 l'avait déjà payé en écrivant pourquoi.

**Mais un cinquième consommateur ne décidait pas, il collectait.**
`ci_failure_handler::fetch_failure_context` filtre `fail|cancel` à la main pour
composer le contexte de réparation. Sa population a grandi en silence, avec deux
gains et **un coût** : la troncature à `MAX_FAILING_JOBS` peut désormais écarter
le log d'un vrai échec de build placé derrière plusieurs lints rouges. Le coût
est **écrit au site** plutôt que compensé — prioriser la liste réintroduirait une
notion de « check qui compte plus », c'est-à-dire la divergence qu'on ferme.

> Chercher les consommateurs d'un lecteur unique ne suffit pas : il faut
> distinguer ceux qui **décident** (ils héritent) de ceux qui **collectent**
> (leur population change, et c'est un effet, pas un héritage).

## Le refus qui comptait : zéro liste d'exemptions

La pente naturelle, face à « tous les checks bloquent maintenant », est une liste
de checks consultatifs — « livrée vide ou courte, chaque entrée avec sa raison ».
Prime l'a refusée, verbatim : *« Une liste nommée d'exempts recrée la divergence
qu'on ferme. »*

Deux détecteurs **structurels** la tiennent, et aucun test de comportement ne
peut voir cette classe : une liste ajoutée demain **ne rend aucune décision
fausse le jour où elle est écrite**, elle rend la porte plus laxiste en silence.
Le second détecteur refuse toute lecture de label GitHub dans la chaîne de
décision — un label est écrivable à la main, et le brancher là transformerait un
geste d'interface en autorisation de merge.

Les deux portent leur **contrôle de bonne foi** (une fixture rouge) : un scan
devenu inopérant se lit exactement comme un arbre propre (classe mika#2205).

## L'asymétrie qui autorise le resserrement

Refuser plus souvent a un coût réel : une PR dont la CI est en vol n'est plus
mergée par GitHub derrière nous, et si le webhook `check_suite.completed` est
perdu (mika#2334 en a mesuré quatre endroits), elle reste **ouverte**.

C'est acceptable parce que les deux erreurs ne pèsent pas pareil :

| erreur | coût |
|---|---|
| faux refus | une PR ouverte, visible, rattrapable à la main |
| faux passage | `main` rouge, toute PR ouverte contaminée, boucle arrêtée, p0 |

**L'arbitrage est local et ne se transporte pas.** Le faucheur de worktrees
(mika#2420) tranche dans l'autre sens, et pour la même raison bien lue : là-bas
l'action détruit du travail, ici l'action *est* l'écriture sur `main`.

## Ce que le correctif n'achète pas

Il ne rattrape pas l'incident : rien ne rétro-estampille un refus qu'on n'a pas
observé. Il ne couvre pas le merge par l'interface GitHub ni par `gh pr merge`
tapé à la main — la porte garde le moteur, et l'autre moitié est le **ruleset**,
geste d'opérateur. Et il ne ferme pas le fail-open sur un bucket que `gh`
ajouterait demain : il le rend **visible**, parce que refuser sur l'inconnu
échangerait un `main` rouge contre une boucle arrêtée.

## Voir aussi

- `crates/mika-agent/CLAUDE.md` § *The gate reads every check, and no longer delegates "green"*
- `CLAUDE.md` racine § *La porte de merge lit TOUS les checks* — surfaces, sondes, haltes
- `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`
- `docs/solutions/best-practices/un-budget-declare-par-un-manifeste-doit-etre-celui-applique-2026-09-10.md`

---
module: skills/qa-review
tags: [prompt-enforcement, drift, qa-review, pipeline-guards, cross-repo, structural-guards]
problem_type: design-pattern
---

# Un prompt qui réimplémente une garde exécutable dérive — et les deux sens de la dérive coûtent

*Origine : mika#2172. Classe partagée avec mika#2158 (deux prédicats de grooming) et
mika#2205 (un accesseur étroit à côté du résolveur canonique).*

## Le fait

`qa-review/system_prompt.md` décrivait en prose les règles que
`scripts/verify-pipeline.sh` porte en code. En une heure, le 2026-09-04, la QA a bloqué
deux PR que la CI de leur propre dépôt validait en vert :

| PR | CI | verdict QA | motif |
|---|---|---|---|
| mika#2167 | 21/21 verts | `block[pipeline]` | « No plan document in `docs/plans/` » |
| mika-platform#203 | 7/7 verts | `block[pipeline]` | trailer `no-plan` « not a recognized bypass value » |

## Quatre divergences, deux directions

La paraphrase n'avait pas seulement vieilli : elle avait vieilli **dans les deux sens**,
ce qui est le point.

1. **Une règle en trop.** Le prompt exigeait un `docs/plans/*.md`.
   `verify-pipeline.sh` lignes 71-78 explique avoir retiré ce contrôle *exprès* — il
   rejetait des PR `/ce:compound` légitimes sans échappatoire. Motif exact du blocage de
   #2167.
2. **Un vocabulaire trop étroit.** Le prompt codait en dur `(docs-only|code-only)` — le
   vocabulaire de `mika`. `mika-platform` livre `plan-doc-check.sh`, dont le vocabulaire
   est `no-plan` et seulement lui. La QA relit tous les dépôts ; elle a appliqué le
   vocabulaire de l'un à la garde de l'autre.
3. **Une définition qui glisse, sur le même dépôt.** Pour le script, `code-only` =
   `!docs && source` où `docs` **inclut `docs/solutions/`**. Pour le prompt,
   `docs/solutions/` ne comptait pas comme docs. Divergence interne, pas seulement
   inter-dépôts.
4. **Une règle sans original.** Le « tactical-surface auto-detect » auto-exemptait
   `scripts/`, `os/`, `Dockerfile.`, `skills/bundled/_shared/`. Aucun script ne porte
   cette règle, et ces chemins sont tous dans le `SOURCE_BUCKET` de
   `verify-pipeline.sh`. Les trois premières faisaient **bloquer** la QA là où la CI
   passe ; celle-ci la faisait **passer** là où la CI bloque.

Les trois premières sont visibles : elles produisent un ticket bloqué et quelqu'un vient
se plaindre. La quatrième ne l'est pas — une approbation en trop ne réveille personne.
Une paraphrase ne dérive donc pas seulement ; elle dérive de façon **asymétriquement
observable**, et le sens silencieux est celui qui approuve.

## Le signe distinctif de la classe

Le prompt portait, à la ligne 190 :

> The `git log … | grep '^Pipeline-Exempt: …'` pipeline mirrors `verify-pipeline.sh`
> lines 158–172 **verbatim**.

C'était faux au moment où c'était écrit. **Une copie qui se déclare fidèle est
précisément celle que personne ne re-vérifie** — l'assertion de fidélité remplace la
vérification au lieu de l'appeler. Chercher cette phrase est un moyen bon marché de
trouver la classe ailleurs : `grep -rn "mirrors\|verbatim\|same as\|kept in sync"` sur
les prompts et les commentaires.

## Le correctif : exécuter, pas décrire

`qa-review` **lance** désormais la garde du dépôt cible et rapporte sa sortie. Ce qui
reste au prompt est ce qu'un script ne sait pas faire — revue de fond, plan↔AC, lecture
de sécurité. Sur #2167 comme sur #203, cette partie-là n'avait rien trouvé à redire ;
seul le volet mécanique avait échoué.

Trois points d'implémentation qui ne sont pas évidents :

- **Exécuter contre la ref de la PR, pas contre le checkout partagé.** Les gardes font
  `cd "$(dirname "$0")/.."` et agrègent staged + unstaged. Les lancer dans
  `$MIKA_PLATFORM_DIR/<repo>/` jugerait `main`, potentiellement sale. D'où un worktree
  jetable **détaché** (index vide), avec `trap … EXIT` et `worktree prune` en préambule.
  Coût mesuré sur `mika` (3 323 fichiers) : ~0,6 s contre un budget `shell-exec` de 30 s.
- **Reconstituer l'environnement que la CI pose.** `run_shell` scrube `GH_TOKEN`, donc
  les appels `gh` internes des gardes échouent. `GITHUB_EVENT_PATH` est **fabriqué** à
  partir des labels vivants de `qa_pr_view` — ce qui reproduit fidèlement le chemin
  `pipeline-exempt` et évite au passage le gel de snapshot que mika#1395 contourne en CI.
- **Nommer le seul chemin non reproductible plutôt que le deviner.** L'exemption
  « label `documentation` sur l'issue liée » exige un `gh api` authentifié. Sans lui, la
  garde la résout à `false`. Ce cas dégrade en `hold[review]` avec cause nommée — jamais
  un `block` faux. La QA ne réévalue pas la règle ; elle constate qu'une entrée que la
  garde n'a pas pu lire aurait changé son verdict.

**Fail-closed sur l'exécution, pas sur le verdict :** une garde qu'on n'a pas pu lancer
(worktree, fetch, timeout, refus de `shell-exec`) rend `hold[review]`, jamais `pass`.
Une garde qu'on n'a pas pu exécuter n'est pas une garde qui passe.

## Corriger un seul étage ne suffit pas

Le défaut avait un second site. `Step 2.5.1` émettait `block[pipeline]` — « No plan
callout found » — dès l'absence de callout `> - **Plan:**`. mika#2167 n'a **ni callout
ni issue liée** (`closingIssuesReferences: []`) : elle aurait passé le Step 2 réparé
puis été rebloquée un pas plus loin, sur le même motif. Quand on retire une règle
dupliquée, chercher les autres endroits qui la posent — la duplication a rarement un
seul exemplaire.

## Ce qui ferme la classe

Resynchroniser la prose aurait corrigé les quatre écarts du jour et rouvert la classe au
prochain changement de garde. Deux choses la ferment :

1. **Exécuter le script.** Un script ne peut pas diverger de lui-même.
2. **Une garde structurelle sur le prompt** —
   `crates/mika-agent/tests/qa_review_executes_repo_guards.rs` refuse le retour de
   l'alternation `(docs-only|code-only)`, de l'allowlist de préfixes, et des deux motifs
   de blocage retirés. Vérifiée par **contrôle négatif** : les trois assertions ont été
   vues échouer sur un prompt réinjecté avant d'être livrées vertes. Même forme que
   `grooming_marker::tests::no_grooming_regex_outside_this_module` (mika#2158).

Le point (2) est ce qui distingue cette réparation d'une simple mise à jour : la prose
seule ne se défend pas, et le projet a déjà mesuré cette fragilité ailleurs
(`prompt-enforcement-structural-guards.md`).

## Le compromis retenu pour la découverte des gardes

La forme la plus fidèle serait de lire `.github/workflows/*.yml` pour en extraire les
scripts invoqués. Elle demande un parseur YAML robuste dans un budget de 30 s et échoue
**en silence** sur une syntaxe inattendue — un échec silencieux au même endroit que
celui qu'on répare. Le choix retenu est une **liste de chemins** :

```
scripts/verify-pipeline.sh
scripts/plan-doc-check.sh
```

C'est une liste de *fichiers*, pas une réimplémentation de *règles*. Elle ne peut pas
diverger en verdict, seulement en couverture — et une couverture manquante est signalée
(`PIPELINE: not-applicable`, avec les chemins cherchés) au lieu d'être inventée. Classe
de dérive strictement plus faible que celle qu'on ferme.

## Signal à retenir

Quand un prompt et un script répondent à la même question, ils ne cassent rien en
divergeant — c'est précisément pour ça qu'ils divergent longtemps. La question n'est pas
« sont-ils d'accord aujourd'hui ? » mais « qu'est-ce qui échouera le jour où ils cesseront
de l'être ? ». Si la réponse est « rien », la copie est déjà en train de dériver.

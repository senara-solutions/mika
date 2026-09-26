---
module: ci-cd
tags: [github-actions, ecr, docker, image-tags, immutability, structural-guard, anti-vacuity, producteur-consommateur, mika-2174, mika-2143]
problem_type: integration-mismatch
category: ci-cd
---

# Le tag qu'on pousse et le tag qu'on consomme — un défaut sans symptôme, et la garde qui refusait sa propre correction

## Problème (mika#2174)

`.github/workflows/agent-image-build-push.yml` poussait **un seul** tag, le sha nu de 40 caractères :

```yaml
tags: |
  ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}
```

La forme réellement consommée en aval est `main-<short>`. Mesuré sur `mika-cloud` le 2026-09-04 :

```
$ grep -rn -E '^\s*tag:\s*' helm/*/values*.yaml
helm/mika-console/values-aws-dev.yaml:30:  tag: "main-7cea6d6"
helm/mika-gateway/values-aws-dev.yaml:19:  tag: "main-960ab824"
```

Historique ECR : `main-56336b9e`, `main-960ab824`, `main-d7314906`.

**Contrôle négatif, mesuré :** aucun consommateur ne lit la forme sha-nu. `grep -rn -E 'github\.sha|GITHUB_SHA|[0-9a-f]{40}' helm/ scripts/` sur `mika-cloud` rend **zéro ligne**. `rotate-image.sh` ne dérive aucun tag : il reçoit `NEW_TAG` en argument et l'écrit tel quel.

## Cause racine

Elle n'est pas « quelqu'un a choisi la mauvaise longueur ». Elle est qu'un **producteur et son consommateur vivent dans deux dépôts** et que rien, dans aucun des deux, n'énonçait la forme qui les relie. Le producteur écrivait ce qui lui était naturel — le sha que GitHub lui tend — et le consommateur lisait ce qui lui était naturel — la forme que l'opérateur avait posée à la main des mois plus tôt.

**Ce défaut n'a pas de symptôme dans le dépôt qui le porte.** Il ne fait pas rougir la CI, ne fait pas échouer le workflow, ne casse aucun test : il produit simplement un artefact que personne, en aval, ne sait déployer sans traduction manuelle. C'est ce qui le sépare de mika#2143 (le tag mouvant sur dépôt immuable), qui l'excluait explicitement de son périmètre et l'a fiché comme disposition pré-spécifiée à déclencher une fois la prémisse confirmée.

## Solution

**Voie A — aligner le producteur**, retenue contre la voie B (faire adopter le sha nu par la rotation, qui aurait exigé de réécrire les fichiers de valeurs déjà déployés et rendu l'historique ECR hétérogène).

Le workflow pousse désormais **deux** tags, chacun fonction du commit :

```yaml
tags: |
  ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}
  ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:main-${{ env.SHORT_SHA }}
```

Trois sites, pas seulement la ligne `tags:` — même discipline qu'en #2143 : l'en-tête du fichier (qui annonçait « exactly one » tag et interdisait d'en ajouter un second) et le résumé de workflow (qui n'annonçait que la forme nue) auraient sinon contredit le comportement.

Le short-sha transite par une variable parce que **le langage d'expression de GitHub Actions n'a pas de fonction de sous-chaîne** : on ne peut pas écrire `${{ github.sha }}` tronqué dans `tags:`. Une étape amont pose `SHORT_SHA=${GITHUB_SHA:0:8}` dans `$GITHUB_ENV`.

## La découverte : la garde de #2143 refusait la correction

`scripts/check-image-tags-immutable.sh` exigeait que **chaque tag contienne littéralement `github.sha` ou `GITHUB_SHA`**. Le tag `main-${{ env.SHORT_SHA }}` est dérivé du sha — mais il ne le dit pas à cet endroit-là. La garde l'a refusé :

```
ERROR: moving tag pushed to an IMMUTABLE registry: …:main-${{ env.SHORT_SHA }}
```

**C'est la leçon transférable, et elle est le miroir exact de celle de #2143.** Cette garde-là avait été écrite pour lire une *propriété* plutôt qu'une *orthographe* (« dérivé du sha », pas « pas appelé latest »), ce qui la protégeait des faux **négatifs**. Mais la propriété était vérifiée par une orthographe littérale — la présence du jeton dans le tag — et c'est par là qu'arrivent les faux **positifs**. Une garde qui refuse une correction légitime ne survit pas : elle se fait supprimer, ou allowlister, à la première fois où quelqu'un a raison contre elle.

Le réflexe opposé est pire. Accepter n'importe quel `${{ env.X }}` aurait accepté `:latest` écrit à travers une variable — le défaut fondateur de #2143 avec une indirection de plus.

**Extension retenue : la garde résout l'indirection.** Une variable rend un tag sha-dérivé quand **le fichier lui-même** l'assigne depuis le sha, dans l'une des trois écritures qu'un workflow possède (entrée de mapping `env:`, assignation shell vers `$GITHUB_ENV`, sortie d'étape vers `$GITHUB_OUTPUT`). Elle est **fail-closed** : une variable dont le fichier ne montre pas la dérivation est refusée exactement comme un `:latest` nu.

Deux frontières, énoncées dans le fichier plutôt que laissées à découvrir :

- **Une dérivation écrite dans un commentaire ne compte pas.** Un `#` dit ce que le fichier prétend faire, pas ce qu'il fait — l'accepter permettrait de légitimer un tag par une phrase posée à côté.
- **La résolution est à portée de fichier et de nom, pas de job.** Si deux jobs assignaient le même nom différemment, la garde lirait l'union. Étendre par propriété si cette forme devient réelle ici.

### Non-vacuité

Le harnais passe de 20 à **31 cas**. Les onze ajoutés épinglent les deux moitiés — celle qu'on vient d'ouvrir *et* celle qu'on doit garder fermée :

| cas | attendu |
|---|---|
| variable dérivée dans le fichier | accepté |
| **même tag, dérivation retirée** | refusé |
| variable assignée depuis une valeur non-sha (`SHORT_SHA=latest`) | refusé |
| dérivation présente **seulement en commentaire** | refusé |
| entrée de mapping `env:` dérivée du sha | accepté |
| sortie d'étape dérivée du sha | accepté |
| `SHORT_SHA_SUFFIX` alors que seul `SHORT_SHA` est dérivé | refusé |
| clé `key:` hors d'un bloc `env:` (dont `tags:` lui-même) | ne devient pas une variable dérivée |
| `:latest` **à côté** d'une variable résoluble | refusé, et c'est `:latest` qui est nommé |

L'avant-dernier vient d'une relecture de la garde par son propre auteur : la première rédaction collectait n'importe quelle clé `NAME:` posée sur une ligne mentionnant le sha, donc la clé `tags:` elle-même quand la liste est écrite en scalaire simple — un tag aurait pu être légitimé par une variable nommée d'après la clé qui le porte. La collecte est désormais bornée aux blocs `env:`, suivis par indentation comme `extract_tags` suit une liste de tags.

Le dernier est le plus important : une variable légitime dans le fichier ne doit blanchir aucun tag mouvant posé à côté d'elle.

Démonstration faite sur le **vrai** fichier et pas seulement sur fixtures, comme en #2143 : remplacer `${GITHUB_SHA:0:8}` par une constante dans le workflow réel, voir la garde rougir, restaurer.

## Prévention

**Quand un producteur et son consommateur vivent dans deux dépôts, la forme qui les relie est un contrat — et un contrat que personne n'écrit se découvre à la première livraison.** Ici il est écrit dans le fichier producteur, avec la mesure qui l'établit (les tags réellement déployés, et le contrôle négatif qui montre que la forme nue n'est lue par personne).

Et le corollaire, pour les gardes structurelles : **une garde qui vérifie une propriété par une orthographe protège des faux négatifs et fabrique des faux positifs.** Le jour où elle refuse une correction juste, la réponse n'est ni de la contourner ni de l'assouplir en gros : c'est de lui apprendre à *résoudre* ce qu'elle ne savait que reconnaître — en restant fail-closed sur ce qu'elle ne peut pas prouver.

## Reste dû, daté

La preuve empirique — un merge vert qui pousse effectivement les deux tags dans ECR — ne peut pas être produite maintenant : le workflow s'arrête à `Verify ECR push role is configured` tant que `ECR_PUSH_ROLE_ARN` est absent. **Condition de réveil : à la réactivation du workflow, après mika-cloud#220 et mika#2143** — le même réveil que celui inscrit pour #2143, et celui qui libère mika#1619. Exiger au moment de la PR une preuve que le séquencement du ticket rend impossible serait se contredire.

## Références

- mika#2174 — le ticket ; `docs/plans/2026-09-07-006-fix-2174-image-tag-main-short8-plan.md` — le plan groomé (architecte : READY, première passe)
- mika#2143 / `docs/solutions/ci-cd/2026-09-04-tag-mouvant-sur-depot-immuable-mika-2143.md` — la garde étendue ici, et la § Fire-Disposition qui a produit ce ticket
- `mika-cloud/scripts/rotate-image.sh`, `mika-cloud/helm/*/values-aws-dev.yaml` — le consommateur et la forme qu'il lit
- mika#1619 — la capacité que ceci conditionne ; mika-cloud#220 — l'OIDC amont
- mika#2103 / `scripts/check-byte-slices.sh` — l'incident dont « étendre par propriété » est la leçon

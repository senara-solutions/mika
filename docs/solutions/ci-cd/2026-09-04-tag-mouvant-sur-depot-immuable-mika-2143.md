---
module: ci-cd
tags: [github-actions, ecr, docker, image-tags, immutability, structural-guard, anti-vacuity, accented-fixtures, mika-2143]
problem_type: ci-failure
category: ci-cd
---

# Un tag mouvant sur un dépôt immuable — la contradiction que personne n'énonce, et la garde qui la refuse

## Problème (mika#2143)

`.github/workflows/agent-image-build-push.yml`, arrivé sur `main` par PR#2093, poussait **deux** tags par build :

```yaml
tags: |
  ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}
  ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:latest
```

Et les trois dépôts ECR sont déclarés `IMMUTABLE` :

```
$ AWS_PROFILE=mika aws ecr describe-repositories --region eu-west-3 \
    --query 'repositories[].[repositoryName,imageTagMutability]' --output text
mika-gateway    IMMUTABLE
mika-agent      IMMUTABLE
mika-console    IMMUTABLE
```

Un dépôt `IMMUTABLE` refuse de réassigner un tag qu'il porte déjà. Un tag « mouvant » est, par définition, un tag qu'on réassigne. Le workflow demandait les deux.

## Cause racine

Elle n'est pas « quelqu'un a choisi un mauvais nom de tag ». Elle est qu'une **contradiction dans les termes** a traversé une revue sans que rien dans le dépôt ne l'énonce à voix haute. L'en-tête du workflow décrivait même `latest` comme un *« moving convenience tag »* — le mot « mouvant » était écrit, à côté d'un registre immuable, et personne ne les a lus dans la même phrase.

### La prémisse qui a failli faire rater le diagnostic

La première rédaction du ticket annonçait : « 1er merge : `latest` n'existe pas → succès ; 2e : échec ». Plausible, et faux. Mesuré :

```
$ AWS_PROFILE=mika aws ecr describe-images --repository-name mika-agent \
    --region eu-west-3 --image-ids imageTag=latest --query 'imageDetails[].imagePushedAt'
2026-06-09T13:41:44.444000+02:00

$ ... --repository-name mika-console ...   → ImageNotFoundException   # contrôle négatif
```

`latest` était posé depuis le 2026-06-09. Le workflow échouait donc dès sa **première** exécution franchissant l'authentification. **Il n'y avait pas de merge gratuit.** La leçon transférable : une narration de défaut qui « sonne juste » (ça marche une fois puis ça casse) mérite une mesure avant de devenir un critère d'acceptation, et le contrôle négatif — le dépôt où le tag est *absent* — est ce qui rend la mesure lisible.

## Solution

**Retirer la demande contradictoire, pas la garantie.** L'immutabilité est une propriété de provenance : un tag déployé désigne toujours le contenu avec lequel il a été créé. Rendre les dépôts mutables aurait échangé cette propriété contre un confort de nommage — c'est une décision de sécurité, pas un détail d'implémentation, et elle n'a pas été prise.

Un consommateur qui veut « la dernière » la **résout** au lieu de lire un tag mouvant : `aws ecr describe-images` trié sur `imagePushedAt`, ce que fait déjà la sonde de fraîcheur cloud.

### Les trois sites, pas seulement le premier

Le tag mouvant vivait à **trois** endroits du fichier, et n'en corriger qu'un aurait laissé le fichier se contredire :

1. `tags:` — la ligne qui le pousse.
2. L'en-tête — qui le décrivait comme une commodité. Réécrit pour nommer **l'immutabilité comme la raison** de son absence : sans le *pourquoi*, le prochain lecteur rajoute le tag en croyant réparer un oubli.
3. Le résumé de workflow (`$GITHUB_STEP_SUMMARY`) — qui annonçait `| moving tag | latest |`. Un résumé qui annonce ce qui n'a pas été produit est un piège.

Un quatrième site parlait de `latest` sans être le défaut : le commentaire justifiant `cancel-in-progress: false`. Cette directive garde un défaut **distinct** — un push annulé en cours de route. Elle reste ; seule sa justification a été réécrite (couches partiellement transférées, cache `gha` partagé avec `ci.yml`). **Deux défauts voisins dans le même fichier se confondent facilement ; les séparer explicitement est la moitié du travail.**

### La garde qui reste

`scripts/check-image-tags-immutable.sh` + `scripts/test-check-image-tags-immutable.sh`, câblés en cible `make check-image-tags-immutable` et en job CI `Image Tag Immutability Lint`.

Trois propriétés la rendent non décorative :

**1. Elle lit la propriété, pas le jeton.** Elle ne cherche pas la chaîne `latest`. Elle exige que **chaque tag contienne le sha du commit** — donc `:stable`, `:main`, `:prod`, `:dev` tombent aussi. C'est la leçon de mika#2103 appliquée en avance : une garde qui connaît une *orthographe* du défaut laisse passer toutes les autres.

**2. Elle couvre les quatre écritures YAML d'une liste de tags** — scalaire bloc (`tags: |`), séquence bloc (`- item`), séquence flow (`[a, b]`), scalaire simple. Un analyseur qui ne connaîtrait que la première laisserait un tag mouvant réintroduit en style flow passer sans bruit : même classe d'échec, un cran plus bas.

**3. Elle refuse de passer à vide.** Aucune liste `tags:` trouvée → sortie 3, pas 0. Liste vide → 3. Fichier illisible → 2. Une analyse silencieusement vide est indiscernable d'un fichier propre, et c'est précisément ainsi qu'un check vert finit par ne rien vouloir dire.

Non-vacuité démontrée sur le vrai fichier, pas seulement sur des fixtures : réintroduire `:latest`, montrer le rouge, retirer.

### Les fixtures accentuées ne sont pas une coquetterie

Le harnais porte 20 cas, dont une batterie accentuée : chemin contenant accents, espace et apostrophe (`dépôt d'images/`), tag mouvant accentué, commentaire français à l'intérieur du bloc. Le message d'échec est **asséré reproduire l'accent verbatim**.

Ce dépôt écrit ses plans, ses tickets et ses journaux en français. Une batterie de fixtures ASCII-seule y teste une population qui n'existe pas — elle passerait au vert sur un analyseur qui découpe par octets ou qui oublie de quoter un chemin, et le défaut se manifesterait la première fois qu'un chemin réel porte un accent. Une erreur qui garble la valeur fautive est une erreur sur laquelle personne ne peut agir.

## Prévention

**Quand deux propriétés d'un système sont incompatibles par construction, écrire l'incompatibilité dans le fichier qui les porte — et poser une garde qui lit la propriété plutôt que son orthographe du jour.**

Le commentaire seul ne suffit pas : celui du workflow disait déjà « moving tag » à côté d'un registre immuable. Ce qui manquait, c'est un mécanisme qui échoue. Ce qui manquait *aussi*, c'est la phrase qui nomme la raison — un correctif qui retire une ligne sans dire pourquoi invite sa propre annulation.

## Reste dû, daté

La moitié empirique du critère d'acceptation (deux merges consécutifs verts) ne pouvait pas être prouvée au moment de la PR : le workflow s'arrête à `Verify ECR push role is configured` tant que le secret `ECR_PUSH_ROLE_ARN` est absent. **Condition de réveil : à la réactivation du workflow, après mika-cloud#220.** Elle dort visiblement, elle n'est pas abandonnée — exiger au moment de la PR une preuve que le séquencement du même ticket rend impossible serait se contredire.

## Références

- mika#2143 — le ticket ; `docs/plans/2026-09-03-001-fix-2143-tag-mouvant-sur-depot-immuable-plan.md` — le plan groomé
- mika#2174 — le défaut voisin fiché à part : le workflow émet le sha **nu** de 40 caractères, la rotation consomme `main-<short8>`
- PR#2093 — l'entrée du workflow sur `main` ; mika-cloud#220 — l'OIDC qui conditionne la réactivation ; mika#1619 — la capacité visée
- mika#2103 / `scripts/check-byte-slices.sh` — la garde dont la leçon (« étendre par propriété, jamais par orthographe ») est appliquée ici

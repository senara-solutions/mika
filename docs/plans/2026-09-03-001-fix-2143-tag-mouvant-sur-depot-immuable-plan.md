---
issue: mika#2143
title: Un tag mouvant sur un dépôt immuable — retirer `latest`, pas l'immutabilité - Plan
type: fix
scope_repo: mika
priority: p1-important
date: 2026-09-03
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Un tag mouvant sur un dépôt immuable — retirer `latest`, pas l'immutabilité - Plan

## Goal Capsule

**Objectif.** `agent-image-build-push.yml` pousse un tag mouvant (`latest`) vers un
dépôt ECR déclaré `IMMUTABLE`. Les deux propriétés sont incompatibles par
construction. Le workflow doit devenir ré-exécutable à chaque merge sans jamais
échanger l'immutabilité contre une commodité de nommage.

**Moyen.** Voie A du ticket — cesser de pousser `latest`. Le tag par sha suffit à
désigner une image ; « la dernière » se résout par `describe-images` trié sur
`imagePushedAt`, ce que la sonde de fraîcheur cloud fait déjà. Aucun consommateur
documenté du `:latest` ECR n'existe (recherche exhaustive consignée en § Sources).

**Hiérarchie d'autorité.** AC du ticket > ce plan > jugement de l'implémenteur.
Deux corrections de prémisse (§ Prémisses rectifiées) ont été **reportées dans le
corps du ticket le 2026-09-03**, au checkpoint de réconciliation : le corps et ce
plan disent désormais la même chose. Elles ne changent pas le remède, elles changent
ce que la vérification doit prouver — et le ticket le dit maintenant lui-même.

**Conditions d'arrêt.**
- S'arrêter si la voie C (rendre les dépôts mutables) est retenue sans décision
  opérateur écrite. L'immutabilité est une garantie de provenance, pas un réglage.
- S'arrêter si la vérification se réduit à « le workflow a réussi une fois ». Le
  défaut est un défaut de **répétition** ; une exécution unique ne prouve rien —
  et, prémisse rectifiée, ne réussirait même pas.
- S'arrêter si le correctif renomme le tag par sha. La forme du tag immuable est un
  défaut **distinct**, hors périmètre, consigné en § Hors périmètre.
- S'arrêter si l'en-tête du workflow continue de décrire `latest` comme un
  « moving convenience tag » après le correctif (AC5).

**Profil d'exécution.** Une seule surface : `.github/workflows/agent-image-build-push.yml`.
Plus une garde de non-régression. Aucun code Rust, aucune charte Helm.

**Propriété de la queue.** PR sur `mika`, routée vers mika-qa.

## Product Contract

### Résumé

Un dépôt `IMMUTABLE` refuse de réassigner un tag existant. Un tag « mouvant » est,
par définition, un tag qu'on réassigne. Le workflow demande les deux. Ce plan retire
la demande contradictoire — pas la garantie.

### Cadrage du problème

`.github/workflows/agent-image-build-push.yml:98-100` pousse deux tags par build :

```yaml
          tags: |
            ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}
            ${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:latest
```

Les trois dépôts sont immuables (mesuré, § Contraintes vérifiées). Le second tag ne
peut donc être poussé qu'une fois — et une seule.

### Prémisses rectifiées

Deux affirmations de la **première rédaction** du corps ne survivaient pas à la
mesure. Elles ont été corrigées dans le corps le 2026-09-03 (checkpoint Phase 2.5 de
`/mika-groom-ticket`, voie de résolution 1) ; elles restent consignées ici parce
qu'elles changent **ce que la vérification doit prouver**, pas le remède, et parce
qu'un lecteur du plan doit savoir sur quelle mesure la rectification s'appuie.

**P1 — `latest` existe déjà ; l'échec est au premier merge, pas au second.**
La première rédaction annonçait « 1er merge : n'existe pas → succès ; 2e : existe
déjà → échec », et le titre disait « il réussit une fois puis échoue à chaque merge ».
Mesuré le 2026-09-03 :

```
$ AWS_PROFILE=mika aws ecr describe-images --repository-name mika-agent \
    --region eu-west-3 --image-ids imageTag=latest --query 'imageDetails[].imagePushedAt'
2026-06-09T13:41:44.444000+02:00
```

`latest` est posé sur `mika-agent` depuis le 2026-06-09, et sur `mika-gateway` depuis
le 2026-06-09T13:41:38+02:00 (absent de `mika-console` — contrôle négatif : la
commande y rend `ImageNotFoundException`). Le workflow échoue donc dès sa **première**
exécution réussie d'authentification. Conséquence : le défaut est plus grave que
décrit, et l'exigence de rejouabilité d'AC1 reste le bon critère — c'était sa
justification (« passe au premier ») qui était fausse, pas son exigence. Le corps du
ticket porte désormais cette correction, mesure à l'appui.

**P2 — le tag immuable poussé est le sha nu, pas `main-<sha>`.**
La première rédaction citait `:main-${{ ... }}` et parlait du « tag immuable par sha
(`main-<sha>`) ». La ligne 99 pousse `${{ github.sha }}` — 40 caractères, sans préfixe. La convention
`main-<short8>` est celle des tags **déjà déployés** (`mika-cloud/helm/mika-console/values-aws-dev.yaml:30`
→ `tag: "main-7cea6d6"` ; historique ECR : `main-56336b9e`, `main-960ab824`,
`main-d7314906`). Le workflow n'émet donc pas la forme que la rotation d'image
consomme. **Défaut distinct, hors périmètre** — le corps du ticket l'exclut désormais
explicitement (§ « Ce que ce ticket ne couvre pas », point 2). Voir § Fire-Disposition
pour la disposition pré-spécifiée.

### Décisions clés

- **Voie A, sans consommateur à rediriger.** La recherche exhaustive exigée par AC3
  ne trouve aucun consommateur du `:latest` ECR (§ Sources). Retirer le tag ferme le
  défaut au lieu de le déplacer.
- **La garantie l'emporte sur la commodité.** La voie C est écartée : elle échange
  une propriété de provenance contre un confort de nommage. AC2 en fait un contrôle
  négatif — l'immutabilité doit être **mesurée inchangée** après le correctif.
- **Le tag `latest` résiduel n'est pas supprimé de l'ECR.** Il pointe vers une image
  du 2026-06-09 ; le supprimer est une opération destructive sur un registre de
  production, hors du périmètre d'un correctif de workflow. Il devient simplement
  orphelin — et le workflow cesse de s'y heurter. Consigné, pas nettoyé.
- **La vérification est structurelle avant d'être empirique.** AC1 exige deux merges
  réussis ; le workflow ne peut pas s'exécuter tant que l'OIDC n'est pas posé
  (mika-cloud#220). Le plan livre donc une garde qui **reste**, et nomme la mesure
  empirique comme étape de réactivation.

### Exigences

- **R1** — Le workflow ne pousse plus aucun tag mouvant. Un seul tag est poussé, dérivé
  du commit. (AC1, AC2)
- **R2** — Une garde de non-régression échoue si un tag non dérivé du sha réapparaît
  dans la liste `tags:` du workflow. Elle reste dans le dépôt. (AC1)
- **R3** — L'en-tête du workflow (`:16-19`) décrit le comportement réel : un tag, immuable,
  et **pourquoi** il n'y en a pas de mouvant. (AC5)
- **R4** — Le commentaire de concurrence (`:46-48`), qui justifie `cancel-in-progress: false`
  par « le tag mouvant `latest` prendrait du retard », perd son objet et doit être
  ré-justifié sans invoquer `latest`. La garde de concurrence **reste** — c'est un
  défaut distinct, déjà traité, que le ticket demande explicitement de ne pas confondre.
- **R5** — Le résumé de workflow (`:119`) cesse d'annoncer une « moving tag ». (AC5)
- **R6** — Contrôle négatif mesuré : `imageTagMutability == IMMUTABLE` sur les trois
  dépôts après le correctif. Aucun changement d'infrastructure n'est produit par cette
  PR — la garantie tient parce que rien ne la touche. (AC2)
- **R7** — La recherche de consommateurs de `:latest` est consignée dans la PR avec sa
  commande et son résultat, y compris le **contrôle positif** (la recherche trouve bien
  la ligne 100 qu'on retire). Une recherche qui ne trouve rien sans contrôle ne prouve
  rien. (AC3)
- **R8** — AC4 est **sans objet** : il conditionne la voie B, non retenue. Consigné
  comme tel dans la PR plutôt que passé sous silence.

### Sources

Toutes relues sur `origin/main` @ `7b4ec10a` le 2026-09-03.

- `.github/workflows/agent-image-build-push.yml:17-18` — l'en-tête qui décrit les deux
  tags ; `:18` est la ligne fausse nommée par AC5.
- `.github/workflows/agent-image-build-push.yml:46-51` — la garde de concurrence et son
  commentaire ; défaut **distinct**, non modifié quant à son comportement.
- `.github/workflows/agent-image-build-push.yml:98-100` — la liste `tags:`. *(Le corps
  disait `:98-100` ; les deux tags sont en `:99` et `:100`.)*
- `.github/workflows/agent-image-build-push.yml:119` — `| moving tag | \`latest\` |`
  dans le résumé, troisième site à corriger. Le corps du ticket le nomme désormais
  sous AC5 ; la première rédaction ne le nommait pas.
- `mika-cloud/helm/mika-console/values-aws-dev.yaml:30` — `tag: "main-7cea6d6"`, la
  convention réellement déployée (source de P2).
- `mika-cloud/scripts/rotate-image.sh:62-64, 251-263` — le consommateur de tags en aval ;
  il lit `image.tag` d'un fichier de valeurs, **jamais** `:latest`.

**Recherche de consommateurs (AC3), exécutée le 2026-09-03 :**

```
$ grep -rn --include='*.yml' --include='*.yaml' --include='*.tpl' --include='*.sh' \
    --include='*.md' --include='Dockerfile*' --include='*.tf' \
    -E 'mika-(agent|gateway|console)[^ ]*:latest|tag:\s*"?latest|:latest' \
    mika mika-cloud mika-skills
```

Trois familles de résultats, aucune n'est un consommateur :
1. `mika/.github/workflows/agent-image-build-push.yml:100` — **le producteur lui-même**
   (contrôle positif : la recherche trouve bien ce qu'on retire).
2. `mika/docs/{solutions,plans}/1379-*.md` — `mika:latest` d'**Ollama**, pas d'ECR.
3. `mika-cloud/todos/192-*.md` — décrit le cas d'un `MIKA_IMAGE_REPO` vide produisant
   `:latest` par accident ; c'est un argument **pour** retirer le tag, pas un usage.

**Mesures ECR (2026-09-03, `AWS_PROFILE=mika`, `eu-west-3`) :**

```
$ aws ecr describe-repositories --query 'repositories[].[repositoryName,imageTagMutability]'
mika-gateway    IMMUTABLE
mika-agent      IMMUTABLE
mika-console    IMMUTABLE

$ aws ecr describe-images --repository-name mika-agent --image-ids imageTag=latest
2026-06-09T13:41:44.444000+02:00
$ aws ecr describe-images --repository-name mika-gateway --image-ids imageTag=latest
2026-06-09T13:41:38.477000+02:00
$ aws ecr describe-images --repository-name mika-console --image-ids imageTag=latest
ImageNotFoundException                      # contrôle négatif
```

## Planning Contract

### Décisions techniques clés

**KTD1 — retirer la ligne, pas la remplacer par une suppression conditionnelle.**
La voie B (`batch-delete-image` avant push) conserve la commodité au prix d'une
opération destructive dans un chemin automatisé et d'une fenêtre où `latest` n'existe
pas. Elle ajoute un mode de panne (suppression réussie, push échoué → plus de
`latest` du tout) à un workflow dont le défaut est précisément de ne pas être
rejouable. Voie A retenue.

**KTD2 — la garde est un test du fichier de workflow, pas un test d'intégration.**
Le comportement à protéger est syntaxique : « la liste `tags:` ne contient que des
tags dérivés du sha ». Un test qui lit le YAML et assère cela échoue immédiatement si
quelqu'un rajoute un tag mouvant, sans AWS, sans réseau, sans OIDC. Un test
d'intégration ne pourrait pas s'exécuter aujourd'hui (pas de rôle OIDC) et serait donc
une garde qui ne garde rien. Le mécanisme précis (script `scripts/`, job CI, ou test
Rust) est laissé à l'implémenteur ; la propriété exigée est qu'il **échoue si on
réintroduit un tag mouvant** et qu'il tourne dans la CI actuelle.

**KTD3 — preuve de non-vacuité de la garde.** La garde doit être démontrée non vide :
réintroduire `:latest` dans une copie du workflow, montrer que le test rougit, le
retirer. Trace consignée dans la PR. Sans cette démonstration, une garde qui passe est
indiscernable d'une garde qui ne regarde pas.

**KTD4 — AC1 ne peut pas être vérifiée par exécution aujourd'hui, et le plan le dit.**
Le workflow s'arrête à `Verify ECR push role is configured` tant que le secret
`ECR_PUSH_ROLE_ARN` est absent (mika-cloud#220). AC1 se décompose donc en :
- **AC1-a (livrable de cette PR, structurel)** — la garde de KTD2 + l'argument : tous
  les tags poussés sont fonction injective du sha, deux merges distincts produisent
  deux tags distincts, aucune réassignation ne peut se produire. Vérifiable
  immédiatement.
- **AC1-b (à la réactivation, empirique)** — deux merges consécutifs réels sur `main`
  touchant les chemins filtrés produisent deux exécutions vertes. Attesté au runbook
  de réactivation, **pas** dans cette PR.

Cette décomposition n'est pas une réduction de périmètre appliquée en silence : la
première rédaction d'AC1 demandait au moment de la PR une preuve empirique que le
**Séquencement du même corps** rend impossible avant mika-cloud#220 — le corps se
contredisait lui-même. La rectification porte l'intention (« prouver la
répétabilité ») par le seul moyen disponible maintenant, et **date** le moment où
l'autre moitié sera due. Le corps du ticket porte désormais AC1-a / AC1-b avec la
condition de réveil d'AC1-b écrite ; AC1-b n'est pas abandonnée, elle dort visiblement.

**KTD5 — le commentaire de concurrence est ré-justifié, la garde est conservée.**
`cancel-in-progress: false` (`:51`) est justifié en `:46-48` par le retard possible du
tag mouvant. Sans `latest`, cette justification tombe — mais la garde reste juste pour
une autre raison : une build annulée en cours de push laisse une couche partiellement
transférée et brûle le cache partagé avec `ci.yml`. Le commentaire est réécrit sur ce
motif. **Ne pas retirer `cancel-in-progress: false`** : le ticket demande
explicitement de ne pas confondre les deux défauts.

**KTD6 — le résumé de workflow (`:119`) est un troisième site, absent du ticket.**
`echo "| moving tag | \`latest\` |"` annoncerait un tag qui n'est plus poussé. Un
résumé qui ment sur ce qui a été produit est un piège pour le prochain lecteur.
Corrigé dans la même PR ; découvert par le plan parce que la première rédaction du
ticket ne le nommait pas — AC5 le nomme désormais comme troisième site.

### Contraintes vérifiées (mesurées le 2026-09-03 sur `7b4ec10a`, non supposées)

- Les trois dépôts ECR sont `IMMUTABLE`. Le corps le disait ; re-mesuré, confirmé.
- `latest` existe sur `mika-agent` et `mika-gateway` (2026-06-09), absent de
  `mika-console`. Le corps le disait absent — voir P1.
- `ECR_REPOSITORY: mika-agent` (`:60`) : ce workflow ne concerne **que** l'agent.
  Aucun workflow équivalent n'existe pour `mika-gateway` ni `mika-console`
  (`ls .github/workflows/` : neuf fichiers, un seul pousse une image).
- Aucun consommateur du `:latest` ECR dans les trois dépôts (§ Sources, avec contrôle
  positif).
- `rotate-image.sh` lit `image.tag` dans un fichier de valeurs Helm ; il ne résout
  jamais un tag mouvant.

### Séquencement

Ce ticket n'est pas bloquant pour mika-cloud#220 et le devient à la réactivation.
Ordre : mika-cloud#220 (OIDC + verrou `prd.tfvars`) → **ce ticket** → réactivation du
workflow → mika#1619 livré. Le correctif peut être écrit et mergé **maintenant**,
avant #220 ; seule AC1-b attend.

## Hors périmètre

- **La forme du tag immuable (P2).** Le workflow pousse `${{ github.sha }}` (40 car.)
  alors que la convention déployée est `main-<short8>`. Le tag émis n'est donc pas
  celui que `rotate-image.sh` et les fichiers de valeurs consomment. C'est un défaut
  réel et distinct : il ne fait pas échouer le workflow, il rend son produit
  inutilisable en aval sans traduction. **À ficher séparément** (§ Fire-Disposition).
- **Le nettoyage du `latest` orphelin dans ECR.** Opération destructive sur un
  registre de production ; décision opérateur, pas correctif de workflow.
- **La garde de concurrence.** Défaut distinct, déjà traité. Seul son *commentaire*
  change (KTD5).
- **Un workflow d'image pour `mika-gateway` / `mika-console`.** N'existe pas ; hors
  sujet ici.

## Fire-Disposition

Ce plan produit une découverte hors périmètre (P2, la forme du tag). Disposition
pré-spécifiée, pour qu'elle ne soit ni bundlée ni perdue :

- **Si P2 est confirmée par l'implémenteur** (la ligne 99 pousse bien le sha nu et
  aucun consommateur ne lit cette forme) → **ficher un ticket `mika` distinct**,
  `p1-important`, intitulé sur la non-correspondance entre le tag émis et la
  convention consommée, référençant ce plan et mika#1619. Ne **pas** l'implémenter
  dans cette PR.
- **Si P2 se révèle fausse** (un consommateur lit bien le sha nu) → le consigner dans
  la PR et ne rien ficher.

## Implementation Units

### U1. Le workflow ne pousse plus qu'un tag

`.github/workflows/agent-image-build-push.yml:98-100` — retirer la ligne `:100`
(`...:latest`). La liste `tags:` ne contient plus que le tag dérivé du sha.
Rien d'autre ne change dans l'étape de build. (R1)

### U2. L'en-tête dit la vérité, et dit pourquoi

`:16-19` — remplacer la description des deux tags. Le nouveau texte doit énoncer :
(a) un seul tag est poussé, dérivé du commit ; (b) **aucun tag mouvant n'est poussé
parce que les dépôts sont `IMMUTABLE`** — la contrainte est nommée, pas subie ;
(c) comment un consommateur résout « la dernière » (`describe-images` trié sur
`imagePushedAt`). Sans (b), le prochain lecteur rajoutera `latest` en croyant réparer
un oubli. (R3, AC5)

### U3. Le commentaire de concurrence est ré-justifié

`:46-48` — réécrire la justification de `cancel-in-progress: false` sans invoquer
`latest` (KTD5). La directive `:51` est **inchangée**. (R4)

### U4. Le résumé de workflow cesse d'annoncer un tag mouvant

`:119` — retirer la ligne `| moving tag | \`latest\` |`. Les autres lignes du résumé
(registre, dépôt, tag immuable, ref complète, short-sha) restent. (R5, KTD6)

### U5. La garde qui reste

Ajouter un contrôle, exécuté par la CI actuelle, qui lit
`.github/workflows/agent-image-build-push.yml` et échoue si la liste `tags:` contient
une entrée non dérivée de `${{ github.sha }}`. Mécanisme au choix de l'implémenteur
(script sous `scripts/` appelé par un job CI existant, ou test dans la suite
existante) ; la propriété exigée est : **sans AWS, sans OIDC, sans réseau**, et
**rouge si on réintroduit un tag mouvant**. (R2, KTD2)

### U6. La preuve que la garde n'est pas vide

Réintroduire temporairement `:latest`, exécuter la garde, montrer qu'elle échoue,
retirer. Coller la trace (commande + sortie rouge) dans la description de la PR.
(KTD3)

### U7. Consigner AC3, AC4, AC2 dans la PR

- **AC3** — la commande de recherche, sa sortie, et l'analyse des trois familles de
  résultats, **avec le contrôle positif** (la recherche trouve la ligne retirée). (R7)
- **AC4** — déclarer explicitement « sans objet : voie B non retenue ». (R8)
- **AC2** — re-exécuter `aws ecr describe-repositories --query
  'repositories[].[repositoryName,imageTagMutability]'` après le correctif et coller
  les trois `IMMUTABLE`. Aucune ressource AWS n'est modifiée par cette PR ; la mesure
  atteste que la garantie tient. (R6)

## Verification Contract

| Ce qui est vérifié | Comment | Quand |
|---|---|---|
| Un seul tag, dérivé du sha | Lecture du diff + garde U5 | PR |
| La garde n'est pas vide | U6, trace rouge dans la PR | PR |
| L'en-tête ne ment plus | Relecture de `:16-19`, `:46-48`, `:119` | PR |
| Immutabilité inchangée | `describe-repositories`, trois `IMMUTABLE` | PR |
| Aucun consommateur orphelin | Recherche U7 avec contrôle positif | PR |
| Deux merges consécutifs verts | Runbook de réactivation (AC1-b) | Après mika-cloud#220 |

## Acceptance criteria

- **AC1-a** — La garde U5 existe, tourne dans la CI actuelle, et échoue si un tag
  mouvant est réintroduit (démontré par U6). Tous les tags poussés sont fonction du
  sha ; deux commits distincts ne peuvent pas se disputer un tag.
- **AC1-b** — *(différé, hors PR)* Deux merges consécutifs sur `main` touchant les
  chemins filtrés produisent deux exécutions vertes. Dû à la réactivation du workflow,
  attesté au runbook. Voir KTD4 pour la justification de la décomposition.
- **AC2** — `imageTagMutability == IMMUTABLE` sur `mika-gateway`, `mika-agent`,
  `mika-console` après le correctif, mesuré et collé dans la PR. La voie C n'est pas
  retenue ; aucune décision opérateur n'est sollicitée.
- **AC3** — La recherche exhaustive de consommateurs de `:latest` est consignée avec
  sa commande, sa sortie complète, son contrôle positif, et l'analyse de chaque
  résultat. Aucun consommateur à rediriger.
- **AC4** — Sans objet (voie B non retenue) ; le corps le prévoit explicitement.
  Déclaré tel quel dans la PR plutôt que passé sous silence.
- **AC5** — `:18` ne décrit plus `latest` comme un « moving convenience tag ». Les
  trois sites (`:16-19`, `:46-48`, `:119`) sont cohérents avec le comportement réel,
  et l'en-tête nomme l'immutabilité comme la **raison** de l'absence de tag mouvant.

## Definition of Done

- `.github/workflows/agent-image-build-push.yml` ne contient plus aucune occurrence de
  `latest` (vérifiable : `grep -c latest` rend `0`).
- `cancel-in-progress: false` est toujours présent, avec un commentaire qui ne cite
  plus `latest`.
- La garde U5 est mergée et verte ; sa non-vacuité est démontrée dans la PR.
- Les trois `IMMUTABLE` sont collés dans la PR.
- La disposition de P2 (§ Fire-Disposition) est exécutée : ticket fiché, ou raison
  écrite de ne pas ficher.
- La PR référence `Closes #2143` et nomme AC1-b comme reste dû à la réactivation.

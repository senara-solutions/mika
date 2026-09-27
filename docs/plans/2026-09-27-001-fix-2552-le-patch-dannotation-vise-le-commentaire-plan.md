# Le `PATCH` d'annotation vise le commentaire, jamais l'issue (mika#2552)

**Ticket :** senara-solutions/mika#2552 — p1, zone decision-core (`.github/workflows/`).
**Type :** fix.
**Priorité de lecture :** § 2 (d) et (e) avant d'écrire une ligne — le remède du
ticket est juste et porte deux pièges qui le transformeraient en régression.

---

## 1. Le défaut, mesuré et relu dans le code

Le workflow d'annotation `.github/workflows/issue-token-annotate.yml` (surface S3
de mika#2201) **écrase le corps de l'issue** au lieu de réécrire son propre
commentaire. La chaîne, maillon par maillon, relue dans le fichier :

| # | site | ce qui se passe |
|---|---|---|
| 1 | l. 78-80 | `PRIOR` ← `gh issue view --json comments --jq '…\| .url'`, qui rend l'URL **html** : `https://github.com/O/R/issues/N#issuecomment-ID` |
| 2 | l. 87 et l. 97 | substitution `https://github.com` → `https://api.github.com/repos` ⇒ `https://api.github.com/repos/O/R/issues/N#issuecomment-ID` |
| 3 | transport HTTP | **le fragment n'est pas transmis** ⇒ la cible effective est `PATCH /repos/O/R/issues/N` |
| 4 | API GitHub | c'est la route d'édition d'**issue** ; `-f body=` remplace le corps |
| 5 | l. 89 et l. 98 | `>/dev/null 2>&1 \|\| true` — l'écrasement n'émet **rien** |

Le maillon 3 est le pivot : la cible n'est pas *malformée*, elle est **valide et
désigne autre chose**. Le maillon 5 est ce qui a rendu la destruction invisible.
Le premier passage sur un ticket est sain (l. 100, `gh issue comment`) ; **tout**
passage suivant écrase, que le lint soit rouge (« réécrire l'accusation », l. 97)
ou vert (« retirer l'accusation », l. 87).

**La preuve, sur mika#2544.** Corps posé par le groom moteur à
`2026-09-27T08:52:00Z` (3934 octets, callouts `Branch` / `Plan` /
`Grooming history`), run `36307549281` (`issues`, `edited`) à 08:52:04Z, puis
édition du corps par **github-actions** à 08:52:13Z : 979 octets, l'annotation
seule. Visible en GraphQL par `issue.userContentEdits`.

---

## 2. Ce que la lecture du code établit

### (a) Les deux sites portent le même défaut, confirmé

`gh api` avec `-X PATCH`, `POST`, `PUT` ou `DELETE` dans `.github/workflows/` :
**deux occurrences, toutes deux ce défaut** (l. 87 et l. 97 du même fichier).
Aucun autre workflow du dépôt ne réécrit une URL html en URL API — mesuré sur
tout l'arbre. Conséquence directe pour le § 5 — **la population du détecteur est
exactement le défaut, et son allowlist naît vide** ; il n'y a
aucune violation pré-existante à exempter. Ce n'est pas une supposition, c'est le
résultat du balayage.

### (b) La capacité destructrice n'est PAS retirable par les permissions

`permissions: issues: write` (l. 51) est **nécessaire** à `gh issue comment`, et
GitHub n'offre aucune granularité « commenter sans éditer le corps ». Il n'existe
donc pas de version de ce workflow qui puisse annoter sans pouvoir détruire. **Ce
qui borne ne peut être qu'un prédicat sur la cible**, jamais une réduction de
privilège — et c'est pourquoi l'AC3 est la moitié structurelle de ce ticket, pas
sa décoration.

### (c) Le prédicat littéral de l'AC3 suffit pour le défaut mesuré

Mesuré (`.pilot-scratch/probe-target-predicate.sh`) : la cible du défaut,
`…/issues/2544#issuecomment-1234`, **ne contient pas** `/issues/comments/`. Donc
un `contains` seul ferme le cas fondateur. Le prédicat **ancré** retenu au § 3
est un choix de robustesse qui ne coûte pas une ligne de plus, et non une
extension du périmètre : il refuse en outre `/pulls/comments/`, un hôte étranger,
un `..` de traversal et une query — les quatre mesurés refusés.

### (d) Le remède du ticket porte DEUX pièges, et c'est le § à lire d'abord

Le remède est juste. Sa transcription littérale régresse le contrat que
l'en-tête du workflow revendique (l. 34-37, « ONE COMMENT PER BODY STATE »).

1. **Pagination.** `GET /repos/{o}/{r}/issues/{n}/comments` est paginé à 30.
   Sans `--paginate`, sur un ticket long — la maison en produit — `PRIOR` sort
   **vide** et le workflow **empile** un commentaire à chaque édition. Non
   destructeur, mais une rupture silencieuse du contrat annoncé : une annotation
   qui s'accumule est une annotation qu'on mute.
2. **`| last` est faux SOUS `--paginate`.** `gh api --paginate --jq` applique le
   filtre **page par page** et concatène les sorties : `[…] | last` rendrait
   **une valeur par page**, donc un `PRIOR` multi-lignes — que le `[ -n "$PRIOR" ]`
   accepterait et que `gh api -X PATCH` recevrait comme une URL absurde. La forme
   juste émet 0..n lignes en jq et prend la dernière **en shell** (`tail -n1`).

Ces deux points sont la raison d'être de la vérification V7 du § 5.

### (e) La conséquence sur le loop est ce qui fait la p1

Corps écrasé ⇒ callouts `Branch` / `Plan` / `Grooming history` perdus ⇒
`check_grooming_markers` rend `MarkersMissing` ⇒ au relabel `ready` de 09:00:55Z
le moteur a routé un **second groom** (`b1930ba9`) au lieu de l'implement, alors
que la preuve `Outcome: PLAN_GROOMED` (enfant `064d851e`) existait en base.

**Le workflow d'annotation a donc un pouvoir destructeur sur le substrat de
routage du loop.** C'est cette phrase qui décide la direction du fail-closed au
§ 3 : la cible du `PATCH` n'est pas un champ cosmétique, c'est là où vit la
preuve de grooming.

---

## 3. Le remède — sept unités, et la première est la cause

### R1 — la cible est RÉSOLUE depuis la REST, jamais réécrite depuis l'html

```sh
PRIOR=$(gh api --paginate "repos/$REPO/issues/$NUM/comments" \
          --jq ".[] | select(.body | contains(\"$MARKER\")) | .url" \
        | tail -n1)
```

Le champ `url` de cette route est **déjà** `https://api.github.com/repos/{o}/{r}/issues/comments/{id}` ;
son sibling `html_url` est celui que `gh issue view` rendait. Il est patché **tel
quel**. Aucune substitution de chaîne ne subsiste dans l'arbre : la classe
disparaît plutôt qu'elle n'est corrigée.

`--paginate` et `tail -n1` répondent aux deux pièges du § 2 (d). L'interpolation
shell du `MARKER` dans le filtre jq est **conservée telle qu'aujourd'hui** (la
constante ne contient ni guillemet double ni `$`) : un changement de moins, et
elle évite de dépendre du support de `$ENV` par le moteur jq de `gh`, que ce
worktree n'a pas pu vérifier (`gh` n'y est pas authentifié).

### R2 — la garde de cible : ancrée, pure, fail-closed

Fonction sans réseau, donc testable exhaustivement :

```sh
issue_comment_patch_target_is_valid() {
  [[ "$1" =~ ^https://api\.github\.com/repos/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/issues/comments/[0-9]+$ ]]
}
```

**Direction du fail-closed, et elle est l'inverse de sa voisine.** Toute cible
non conforme, vide ou illisible **refuse le `PATCH`**, bruyamment. C'est
l'inverse du fail-safe du faucheur mika#2420 (où un signal illisible *conserve*),
et l'inversion est raisonnée exactement comme `_shared/pr-push-guard.sh`
(mika#2520) l'écrit pour son propre cas : **ici l'action EST l'écriture
distante.**

> Un refus à tort coûte une annotation manquante — visible dans Actions,
> rattrapable à l'édition suivante, bornée.
> Un passage à tort écrase le corps d'un ticket et, avec lui, la preuve de
> grooming que le loop lit — irréversible sans intervention humaine.

L'arbitrage est **local et ne se transporte pas**.

### R3 — le `PATCH` cesse d'être silencieux

`>/dev/null 2>&1 || true` disparaît des deux sites. Le ticket nomme ce fragment
comme co-cause de l'invisibilité ; il est retiré, pas atténué. Un `PATCH` qui
échoue — ou une cible refusée par R2 — **fait rougir le job**.

Le workflow reste **non bloquant par construction** : GitHub n'offre aucun gate
sur une issue, donc ce qui devient rouge est l'onglet Actions, jamais une PR.
C'est le seul instrument disponible sur cette surface, et son absence est
précisément ce qui a laissé passer onze octets de fragment.

Forme prescrite : `gh api --silent -X PATCH "$PRIOR" -f body="$BODY"`. Le point
**non négociable** est la disparition de `2>&1` et de `|| true` ; si la version
de `gh` du runner ne porte pas `--silent`, `>/dev/null` seul (sans `2>&1`, sans
`|| true`) est l'équivalent acceptable.

### R4 — extraction dans un script, parce qu'un `run:` de YAML n'est testable par rien

La logique quitte le bloc `run:` pour `scripts/annotate-issue-token-comment.sh`,
appelé par le workflow :

```yaml
run: bash scripts/annotate-issue-token-comment.sh \
       "${{ github.repository }}" "${{ github.event.issue.number }}" \
       "${{ steps.lint.outputs.clean }}" /tmp/lint-out.txt
```

Motif maison, appliqué et non inventé : `_shared/cwd-guard.sh` (mika#2536) et
`_shared/pr-push-guard.sh` (mika#2520) ont tous deux sorti leur logique de leur
prescripteur pour qu'un test puisse la voir. La raison propre à ce cas est plus
dure qu'une préférence de style : **l'AC3 et le troisième point du remède
(« test négatif ») ne sont satisfaisables d'aucune manière tant que le prédicat
vit dans un YAML**, qu'aucun harnais du dépôt ne peut exécuter.

L'en-tête de doctrine du workflow (l. 3-43) est **conservé intégralement** et
gagne un paragraphe nommant mika#2552, le maillon 3 et la garde. Il porte déjà
« ONE COMMENT PER BODY STATE » et « WHAT THIS DOES NOT BUY » : c'est le bon
endroit pour dire que la cible est désormais gardée.

### R5 — le harnais, avec son contrôle négatif gelé

`scripts/test-annotate-issue-token-comment.sh`, stub `gh` sur le `PATH` —
précédent direct : `scripts/test-pr-origin-report.sh` l. 81-85 installe un
`$WORK/bin/gh`. Le stub **journalise son argv** dans un fichier, ce qui rend la
cible effective du `PATCH` assertable sans réseau.

| # | ce qu'il pose | AC |
|---|---|---|
| V1 | ticket déjà annoté, lint rouge : l'argv du `PATCH` porte `…/issues/comments/<id>`, et **aucun** argv ne porte `…/issues/<num>` sans `/comments/` | AC1 |
| V2 | lint vert : même assertion sur le chemin « retrait » | AC2 |
| V3 | **contrôle négatif, vu ROUGE d'abord** : le prédicat nourri de la forme pré-fix **gelée littéralement** (`https://api.github.com/repos/senara-solutions/mika/issues/2544#issuecomment-…`) doit refuser | AC3 |
| V4 | premier passage, `PRIOR` absent : un seul `gh issue comment`, **zéro** `PATCH` | — |
| V5 | le prédicat sur les neuf formes mesurées (transcrites de `.pilot-scratch/probe-target-predicate.sh`) | AC3 |
| V6 | **anti-vacuité** : le stub a été appelé au moins une fois par test | — |
| V7 | le stub rend **deux pages** et l'annotation cherchée est sur la seconde : `PRIOR` la trouve et est **mono-ligne** | § 2 (d) |

**V3 est le test qui porte AC3**, et son ordre d'observation est une exigence :
sans l'avoir vu rouge sur la forme gelée, « la garde refuse » est indistinguable
de « le harnais ne vérifie rien » — motif `test-cwd-guard.sh` V3, qui reconstruit
le handler pré-fix plutôt que de re-dériver la forme de git. La forme gelée est
**écrite en clair dans le harnais**, jamais relue depuis l'historique : un
fixture qui échouerait silencieusement à se patcher rejouerait le post-fix et
passerait, la forme vacante que mika#2103 existe pour refuser.

**V6 n'est pas décoratif.** Un harnais dont le stub ne tourne jamais passe en ne
vérifiant rien, et se lit exactement comme un harnais sain (classe mika#2205).

### R6 — le scan de co-location : garde ⟶ `PATCH`, cardinalité assertée

`scripts/check-issue-comment-patch-guard.sh` : chaque `gh api` portant
`-X PATCH` dans `scripts/annotate-issue-token-comment.sh` doit être précédé,
dans la même fonction, d'un appel à `issue_comment_patch_target_is_valid` sur la
variable qu'il patche. **Cardinalité assertée à 2** — sans elle, un prédicat
devenu trop étroit passerait en ne regardant rien.

Le script prend son chemin cible **en argument** (défaut : le vrai script), ce
qui rend son propre contrôle négatif possible sur une copie patchée — motif
`test-cwd-guard.sh` V3 à nouveau.

**Pourquoi un scan en plus des tests de R5.** Retirer la garde **ne rend aucune
décision fausse le jour où on l'écrit** : la résolution R1 reste correcte, la
cible reste bonne, V1/V2 restent verts, et seul le filet disparaît — en silence.
C'est mot pour mot l'argument que mika#2511 a dû écrire pour son propre cas.

**Refusé, et il faut le dire plutôt que le taire : un scan sur
`.github/workflows/` dans son ensemble** (« aucun `gh api -X PATCH|POST|PUT` dans
un workflow »). Sa population devient **0** après R4, puisque la mutation vit
désormais dans un script — donc son silence ne prouverait rien : un détecteur
vacant par construction, la classe mika#2205 appliquée au détecteur lui-même. Il
refuserait de surcroît un futur `gh api -X POST` légitime que personne n'a mesuré
comme dangereux.

### R7 — job CI et cible make

Job `issue-annotation-guard-lint` dans `ci.yml`, `runs-on: ubuntu-22.04`,
checkout pinné au même sha que ses dix voisins, deux étapes dans leur forme
exacte :

```yaml
- name: Reject a PATCH site that bypasses the comment-target guard (mika#2552)
  run: bash scripts/check-issue-comment-patch-guard.sh
- name: Pin the guard's negative behaviour
  run: bash scripts/test-annotate-issue-token-comment.sh
```

Cible `make test-issue-annotation-guard`, branchée dans la liste de `make test`
(voisinage `Makefile` l. 155-177).

---

## 4. Unités d'implémentation

| U | fichier | nature |
|---|---|---|
| U1 | `scripts/annotate-issue-token-comment.sh` | **nouveau** — R1 + R2 + R3, trois chemins (premier passage / réécriture / retrait), en-tête de doctrine |
| U2 | `.github/workflows/issue-token-annotate.yml` | l'étape « Annotate, or withdraw » appelle U1 ; la substitution de chaîne disparaît ; en-tête conservé + un paragraphe mika#2552 |
| U3 | `scripts/test-annotate-issue-token-comment.sh` | **nouveau** — V1 à V7 |
| U4 | `scripts/check-issue-comment-patch-guard.sh` | **nouveau** — R6, allowlist livrée vide, cardinalité 2, chemin en argument |
| U5 | `.github/workflows/ci.yml` | job `issue-annotation-guard-lint` |
| U6 | `Makefile` | cible `test-issue-annotation-guard` + branchement dans `make test` |

Aucun code Rust n'est touché. Aucune migration, aucune variable
d'environnement, aucun réglage déplacé.

---

## 5. Fire-Disposition

Ce plan livre trois détecteurs — la garde runtime R2, le harnais R5, le scan R6 —
donc la section est requise (mika#2306).

**Option (a) — exception nommée en allowlist, allowlist LIVRÉE VIDE et pinnée vide.**

Détail d'implémentation :

- `scripts/check-issue-comment-patch-guard.sh` porte
  `ISSUE_COMMENT_PATCH_GUARD_ALLOWLIST=""` — vide, avec la doctrine mika#2201
  écrite au-dessus : **quand ce scan tire, on route le site vers la garde ; on
  n'ajoute pas de ligne à l'allowlist.** Un site de `PATCH` qu'on ne veut pas
  garder est un site à supprimer.
- Une vérification du harnais **asserte qu'elle est vide** (motif
  `canonical-tokens-exceptions.tsv`, livré vide et pinné vide par un test frère),
  de sorte que le jour où quelqu'un y écrit une ligne, il doive la justifier dans
  un ticket plutôt que dans un commit.
- **La population est nulle par construction, et c'est mesuré, pas supposé**
  (§ 2 (a)) : les deux seuls sites de `PATCH` de l'arbre sont ceux que ce plan
  répare, dans le même commit. Il n'y a **aucune violation existante à
  exempter**, donc aucune entrée à écrire.
- **Aucun détecteur n'est livré désarmé.** R2 est armé dès le déploiement du
  workflow ; R5 et R6 sont armés en CI. Une garde d'écriture destructrice livrée
  « en observation » est une garde absente : elle laisserait la destruction en
  place en ayant l'air de la borner.
- Zone **decision-core** (`.github/workflows/`), conformément à la
  Fire-Disposition du ticket : PR par le loop ou un spawn MPC, **merge par
  Vincent**.

---

## 6. Surfaces opérateur

Ce code tourne dans un runner GitHub, **pas dans mika-spirit** : il n'a ni
`audit_events` ni `$MIKA_SPIRIT_LOG_FILE`. Inventer une surface de journal qui ne
serait pas lue reproduirait le défaut du Signal M. Les surfaces réelles sont
trois requêtes.

```bash
# 1. Le PATCH a-t-il été refusé par la garde ? (log du job Actions)
gh run list --workflow issue-token-annotate.yml --limit 20 \
  --json conclusion,databaseId,createdAt,event
gh run view <id> --log | grep -E 'issue_comment_patch_target_refused'

# 2. CONTRÔLE POSITIF — le workflow a-t-il seulement tourné depuis le déploiement ?
gh run list --workflow issue-token-annotate.yml --limit 5

# 3. La preuve d'AC1 : le corps a-t-il été édité par github-actions ?
gh api graphql -f query='
  { repository(owner:"senara-solutions", name:"mika") {
      issue(number: N) { userContentEdits(last: 10) {
        nodes { editedAt editor { login } } } } } }'
```

| signal | régime attendu | lecture |
|---|---|---|
| job `annotate` en échec | **vide** | toute occurrence est une annotation non posée ou une cible refusée ; aucune n'écrase un corps |
| `issue_comment_patch_target_refused` | **vide** | la garde a mordu : la résolution R1 a régressé — Halte 1 |
| `userContentEdits` portant `github-actions` | **vide** | c'est le défaut réalisé — Halte 1, désarmement immédiat |
| `"canonical-token-annotation" in:body` (5 dépôts) | **zéro** | aujourd'hui : mika#2544 seule, corps à restaurer à la main (§ 9) |

Le **contrôle positif (2) n'est pas décoratif** : zéro échec avec zéro run ne dit
rien du tout ; zéro échec avec des runs dit que le workflow ne détruit plus.

---

## 7. Sondes post-déploiement, et leurs quatre haltes

**S1 — AC1, le second passage (première occasion réelle).** Sur un ticket portant
déjà l'annotation, provoquer une seconde `edited`. Attendu : le **commentaire**
change ; `userContentEdits` de l'issue ne porte **aucune** entrée
`github-actions` ; le corps garde son compte d'octets.

> **Halte 1 — le corps est encore édité.** **Désarmer d'abord, diagnostiquer
> ensuite** : retirer `issues: write` du workflow (il cesse de commenter, donc il
> cesse de détruire) ou désactiver le workflow. Un correctif qui laisse la
> destruction en place pendant qu'on cherche est pire que le workflow arrêté.

**S2 — AC2, le chemin retrait.** Réparer le jeton fautif d'un ticket annoté, puis
attendre l'`edited` : le commentaire antérieur doit porter la phrase de retrait,
le corps rester intact.

> **Halte 2 — le retrait empile un nouveau commentaire au lieu d'éditer.**
> `PRIOR` n'est pas résolu. Lire la **pagination** (§ 2 (d)) **avant** de toucher
> au prédicat de la garde : ce sont deux moitiés distinctes, et c'est la
> résolution qui est en cause, pas le filet.

**S3 — AC3, la garde mord sur la forme pré-fix.**
`bash scripts/test-annotate-issue-token-comment.sh` passe, et son V3 a été **vu
rouge** sur la forme gelée avant d'être vu vert sur la garde.

> **Halte 3 — V3 passe sans que la garde existe.** Le harnais ne vérifie rien :
> lire V6 (anti-vacuité) avant de croire le vert. *Un harnais silencieusement
> inerte se lit exactement comme un harnais sain.*

**S4 — contrôle négatif de bruit (7 jours).** Aucun échec du job `annotate` sur un
ticket nominal, et le contrat « un commentaire par état de corps » tient.

> **Halte 4 — des commentaires s'empilent.** C'est le piège § 2 (d) réalisé, pas
> un défaut de la garde : `--paginate` absent, ou `| last` employé là où il faut
> `tail -n1`.

**Halte transverse — les deux sondes muettes.** Zéro ligne des deux côtés ne
prouve **rien** tant qu'aucune `edited` n'a eu lieu sur un ticket **déjà annoté**
depuis le déploiement. Vérifier le contrôle positif (2) d'abord. *Une garde que
personne n'a exercée se lit exactement comme une garde qui marche* (mika#2205).

---

## 8. Vérification

| V | commande | attendu |
|---|---|---|
| V-a | `bash scripts/test-annotate-issue-token-comment.sh` | exit 0, V1–V7 verts, **V3 vu rouge d'abord** sur la forme gelée |
| V-b | `bash scripts/check-issue-comment-patch-guard.sh` | exit 0, annonce **2 PATCH sites scanned, 0 allowlisted** |
| V-c | contrôle négatif de V-b : copie du script sous `.pilot-scratch/` avec la garde retirée, passée en argument | le scan **rougit** |
| V-d | contrôle négatif de V-b, second terme : copie avec un seul site | le scan **rougit** sur la cardinalité |
| V-e | `bash scripts/check-canonical-tokens.sh` et `bash scripts/canonical-tokens-survey.sh --check` | inchangés — ce plan n'introduit aucun jeton machine |
| V-f | `bash scripts/verify-pipeline.sh` (ou son équivalent CI) | le plan porte `## Acceptance criteria` non vide |
| V-g | `cargo fmt --check`, `cargo clippy`, `cargo test` | inchangés — aucun code Rust touché |

**V-c et V-d sont la raison pour laquelle U4 prend son chemin en argument.** Un
scan dont on n'a jamais observé le rouge est une décoration (mika#2103), et c'est
la discipline que les dix jobs de lint voisins appliquent chacun par une étape
« Pin the guard's negative behaviour ».

---

## 9. Ce que ce travail n'achète PAS

- **Il ne restaure aucun corps.** mika#2544 doit être restauré **à la main**
  depuis `userContentEdits` (version du groom `2026-09-27T08:52:00Z`, 3934
  octets), en corrigeant au passage le jeton `ESCALATE :` en `ESCALATE:` —
  l'espace avant les deux-points est ce qui a fait rougir le lint et déclenché
  toute la chaîne. **Geste opérateur** : ce dispatch n'a pas de `gh` authentifié,
  et fabriquer un corps de ticket depuis une session de grooming serait une
  écriture sur une surface qu'aucun test ne couvre.
- **Il ne rétro-détecte aucune autre victime.** Le balayage du ticket est fait et
  rend mika#2544 seule ; rien ici ne le rejoue, et **la sonde est la prochaine
  occurrence**.
- **Aucun compteur, aucun événement de journal, aucune ligne d'audit.** Les seuls
  instruments sont le rouge du job Actions (R3) et le rouge du CI (R6), et **leur
  silence ne prouve rien tant que personne ne les regarde** — le workflow se
  déclenche quelques fois par jour, donc l'absence d'échec peut simplement
  vouloir dire qu'aucun second passage n'a eu lieu.
- **Il ne retire pas la capacité destructrice**, qui n'est pas retirable (§ 2 (b)).
  Ce qui est ajouté est un prédicat sur la cible, pas une réduction de privilège.
- **Il ne répare pas le jeton fautif** qui a déclenché l'annotation : le lint
  avait raison, et c'est le corps de mika#2544 qui portait la faute.

---

## 10. Definition of Done

- [ ] U1 livré : résolution REST paginée, garde ancrée, `PATCH` non silencieux,
      trois chemins couverts, en-tête de doctrine nommant le maillon 3 et la
      direction du fail-closed.
- [ ] U2 livré : le workflow appelle U1 ; **aucune substitution de chaîne
      html → api ne subsiste dans l'arbre** ; l'en-tête existant est conservé et
      augmenté d'un paragraphe mika#2552.
- [ ] U3 livré : V1 à V7, dont V3 (contrôle négatif sur forme gelée) et V6
      (anti-vacuité).
- [ ] U4 livré : scan de co-location, cardinalité assertée à 2, allowlist livrée
      vide **et** pinnée vide, chemin cible en argument.
- [ ] U5 et U6 livrés : job CI à deux étapes et cible make branchée dans
      `make test`.
- [ ] V-a à V-g passent ; V-c et V-d ont été **observés rouges**.
- [ ] Le corps de PR nomme la restauration manuelle de mika#2544 et la correction
      `ESCALATE :` → `ESCALATE:` comme geste opérateur restant.
- [ ] Merge par Vincent (zone decision-core).

---

## Acceptance criteria

- [ ] Le second passage du workflow sur un ticket déjà annoté modifie le
      **commentaire** et laisse le corps du ticket intact (preuve :
      `userContentEdits` sans entrée `github-actions`).
- [ ] Le chemin « retrait » (lint vert) modifie le commentaire antérieur, pas le
      corps.
- [ ] Un garde-fou refuse tout `PATCH` dont la cible ne contient pas
      `/issues/comments/`.

---

## 11. Hors périmètre, délibérément

- **Le `2>&1` de l'étape de lint** (l. 64) : stderr de
  `check-canonical-tokens.sh` atterrit dans le corps du commentaire, et un échec
  d'usage (fichier absent) produirait une annotation bidon plutôt qu'une erreur.
  Réel, adjacent, **sans rapport avec l'écrasement**. *Ticket de suivi*,
  précondition : une occurrence mesurée.
- **Un scan sur `.github/workflows/` dans son ensemble** — refusé au § 3 R6 avec
  sa raison (population nulle après R4, donc vacant par construction).
- **La restauration du corps de mika#2544** — geste opérateur, § 9.
- **La propagation aux quatre autres dépôts du balayage.** Ce worktree ne
  matérialise que `mika` et `claude-pilot` ; il est établi qu'aucun
  `issue-token-annotate.yml` n'existe dans `claude-pilot`, et **non établi** pour
  `mika-cloud`, `control-monitor` et `mika-platform`. À vérifier par l'opérateur
  d'un seul `grep` ; si un jumeau existe, c'est un ticket par dépôt, ce dépôt
  étant le seul que cette PR peut modifier.
- **Retirer l'annotation de la surface S3** — le ticket ne le demande pas, et
  l'en-tête du workflow argumente déjà pourquoi elle existe.
- **Le jeton fautif de mika#2544** et, plus généralement, la migration
  rétroactive de la prose, que le bearing de mika#2201 interdit.

---
title: "qa-review exécute les gardes du dépôt cible au lieu de les paraphraser"
issue: senara-solutions/mika#2172
type: fix
branch: fix/2172/qa-review-le-prompt-r-impl-mente-les
date: 2026-09-04
status: groomed
---

# qa-review exécute les gardes du dépôt cible au lieu de les paraphraser

## Le problème, mesuré

`qa-review/system_prompt.md` réimplémente en prose les gardes pipeline que chaque dépôt
livre déjà sous forme de script exécutable. La paraphrase a dérivé, et la dérive a bloqué
deux PR que la CI de leur propre dépôt laisse passer.

Mesures faites pendant le grooming, en rejouant les gardes réelles sur les refs réelles
des deux PR (worktree détaché sur `origin/<headRef>`, comparaison à `origin/main`) :

| PR | forme du diff | garde du dépôt | verdict QA observé |
|---|---|---|---|
| mika#2167 | `docs/solutions/…` + 3 × `crates/…` → `docs && source` | `verify-pipeline.sh` → **exit 0** (`Compound: docs/solutions/2026-09-04-gateway-guard-refusal-persists-nothing.md`) | `block[pipeline]` |
| mika-platform#203 | `docs/solutions/…` + `CLAUDE.md`, `Makefile`, `scripts/…` | `verify-pipeline.sh` → **exit 0** ; `plan-doc-check.sh` → **exit 0** (`trailer … no-plan … : correctif de substrat sur le méta-dépôt, demandé`) | `block[pipeline]` |

Contrôle négatif, sur la même garde et dans le même appel — la garde n'est pas vacue :

| cas synthétique sur `mika` | verdict |
|---|---|
| `!docs && source`, sans trailer | **exit 1** — `REJECT: code-only PR: source changes present but no plan/solution doc` |
| même diff + `Pipeline-Exempt: code-only — <raison>` | **exit 0** |

## Les divergences, énumérées

Quatre écarts entre le prompt et les gardes, dont trois déjà nommés dans le ticket
(deux au corps, un au commentaire) et un quatrième trouvé au grooming.

1. **Une règle en trop.** Step 2 check 1 exige un `docs/plans/*.md` dans le diff.
   `verify-pipeline.sh` lignes 71-78 dit avoir retiré cette vérification exprès, parce
   qu'elle rejetait des PR `/ce:compound` légitimes sans échappatoire. Aucun script de
   `mika` ne la demande. C'est le motif exact de blocage de #2167.

2. **Un vocabulaire d'exemption trop étroit.** Le prompt code en dur
   `(docs-only|code-only)`, le vocabulaire de `verify-pipeline.sh`. `mika-platform`
   livre `plan-doc-check.sh`, dont le vocabulaire est `no-plan` — et seulement lui. La
   QA a déclaré invalide un trailer que la CI venait de valider en vert.

3. **Deux définitions de `code-only` sur un même dépôt.** `verify-pipeline.sh` :
   `code-only` = `!docs && source`, où `docs` **inclut `docs/solutions/**`**. Prompt :
   `code-only` = « pas de `docs/plans/*.md` ET des changements source » — `docs/solutions/`
   n'y compte pas. Une PR portant `docs/solutions/` + du code est `pass` pour le script
   et `code-only`-éligible pour le prompt.

4. **Une règle sans original — plus laxiste que la CI** *(trouvée au grooming)*. Le
   « Tactical-surface auto-detect bypass » auto-exempte les PR confinées à
   `.github/workflows/`, `Dockerfile.`, `skills/bundled/_shared/`, `os/`, `scripts/`.
   Aucun script ne porte cette règle. Or `scripts/`, `os/`, `Dockerfile.` et
   `skills/bundled/_shared/` **sont** dans le `SOURCE_BUCKET` de `verify-pipeline.sh`
   (seuls `docs/`, `.github/` et `.claude/worktrees/` en sont exclus). Une PR confinée à
   `scripts/` sans doc est donc `!docs && source` → REJECT en CI, et auto-exemptée par la
   QA. Les trois premières divergences font bloquer la QA là où la CI passe ; celle-ci la
   fait passer là où la CI bloque.

## La cause, et pourquoi la prose ne la ferme pas

Le prompt réimplémente les gardes au lieu de les exécuter. Une paraphrase dérive dans les
deux sens — une règle en trop, un vocabulaire en moins, une définition qui glisse, une
règle inventée — et rien ne casse quand elle diverge : les deux composants répondent à la
même question sans jamais se comparer. Le prompt se déclare d'ailleurs fidèle (ligne 190,
« mirrors `verify-pipeline.sh` lines 158–172 verbatim ») ; une copie qui se déclare fidèle
est précisément celle que personne ne re-vérifie. Même classe que mika#2158.

Resynchroniser la prose corrigerait les quatre écarts d'aujourd'hui et rouvrirait la
classe au prochain changement de garde. Exécuter le script la ferme : un script ne peut
pas diverger de lui-même.

## Livrables

### D1 — Découverte des gardes du dépôt cible

Remplacer la table implicite « un dépôt → un vocabulaire » par une **découverte par
existence de chemin**, sur le checkout local `$MIKA_PLATFORM_DIR/<repo>/` :

```
scripts/verify-pipeline.sh
scripts/plan-doc-check.sh
```

Toutes les gardes existantes sont exécutées ; **le verdict pipeline est leur conjonction**
(une seule sortie non nulle suffit à bloquer). `mika-platform` en porte deux, toutes deux
câblées en CI (`.github/workflows/pipeline-artifacts.yml:31`,
`.github/workflows/plan-doc-check.yml:24`) — le ticket n'en nomme qu'une par dépôt, ce que
ce plan élargit au pluriel sans contredire aucun AC.

**Pourquoi une liste de chemins et non une découverte par lecture des workflows CI.**
Parser `.github/workflows/*.yml` pour extraire les scripts invoqués serait la forme la plus
fidèle, mais elle demande un parseur YAML robuste dans un `run_shell` de 30 s et échoue en
silence sur une syntaxe inattendue — un échec silencieux au même endroit que celui qu'on
répare. La liste de chemins est **une liste de fichiers, pas une réimplémentation de
règles** : elle ne peut pas diverger en *verdict*, seulement en *couverture*, et une
couverture manquante est signalée (D5) au lieu d'être inventée. C'est une classe de dérive
strictement plus faible que celle que ce ticket ferme.

### D2 — Exécution contre la ref de la PR, pas contre le checkout

Les deux gardes font `git diff`/`git log` contre `HEAD` du répertoire courant ;
`verify-pipeline.sh` fait en plus `cd "$(dirname "$0")/.."` et **agrège staged + unstaged**.
Les exécuter dans `$MIKA_PLATFORM_DIR/<repo>/` jugerait le checkout partagé — en général
`main`, potentiellement sale — et non la PR.

Forme retenue : **worktree jetable détaché**, créé puis retiré à chaque revue.

```
git -C $MIKA_PLATFORM_DIR/<repo>/ fetch origin <headRefName>
git -C $MIKA_PLATFORM_DIR/<repo>/ worktree add --detach <tmp> FETCH_HEAD
bash <tmp>/scripts/<garde> origin/<baseRefName>   # cwd = <tmp>
git -C $MIKA_PLATFORM_DIR/<repo>/ worktree remove --force <tmp>
```

Le worktree détaché a staged/unstaged vides, donc pas de contamination. Coût mesuré sur
`mika` (3 323 fichiers suivis, `.git` 108 Mo) : `worktree add` 0,23 s, `verify-pipeline.sh`
0,16–0,28 s, `remove` 0,09 s — **~0,6 s**, contre un plafond `shell-exec` de
`timeout_secs = 30` (`crates/mika-agent/templates/skills/shell-exec/skill.toml`). Marge
suffisante ; l'ensemble tient dans un seul `run_shell`, retrait inclus, avec `trap` pour
que le worktree ne survive pas à un échec.

### D3 — Reconstituer le contexte GitHub que les gardes attendent

Les gardes lisent des variables que seule la CI pose. `run_shell` **scrube `GH_TOKEN`**
(`handlers/run.sh`), donc leurs appels `gh` internes ne peuvent pas être supposés
authentifiés. Ce qui est reconstituable l'est ; ce qui ne l'est pas est nommé (D4).

- `GITHUB_EVENT_PATH` → **fabriquer un événement synthétique** à partir des labels et du
  numéro déjà présents dans la sortie `qa_pr_view` :
  `{"pull_request":{"number":<n>,"labels":[{"name":"<label>"},…]}}`. `verify-pipeline.sh`
  le lit avec `jq`, sans réseau — le chemin label `pipeline-exempt` est donc reproduit
  fidèlement, et à partir des labels **vivants**, ce qui évite au passage le gel de
  snapshot que mika#1395 contourne.
- `GITHUB_PR_LABELS` → la liste des labels, pour le chemin `pipeline-exempt:no-plan` de
  `plan-doc-check.sh`.
- `GITHUB_PR_BODY` → le corps de la PR, pour le repli `Closes #N` de `verify-pipeline.sh`
  et la citation de plan-doc de `plan-doc-check.sh`.

Le scan lexical L3 de `shell-exec` (mika#1957) refuse le token `gh` **dans la chaîne de
commande** ; il n'inspecte pas le contenu d'un script exécuté. `bash <tmp>/scripts/<garde>`
passe donc le scan. La commande construite ne doit contenir aucun token `gh` isolé.

### D4 — Le seul chemin non reproductible, et son traitement

L'exemption « label `documentation` sur l'issue liée » (mika#861) exige un `gh api` vers
l'issue ; sans `GH_TOKEN`, `verify-pipeline.sh` la résout à `false`. Une PR **docs-only**
dont l'issue liée porte `documentation` passerait en CI et sortirait `exit 1` ici.

Traitement : la QA détient déjà l'issue liée (Step 1 / Step 2.5.1). Quand — et seulement
quand — la garde rejette avec `[pipeline-exempt: none] REJECT: docs-only` **et** que
l'issue liée porte le label `documentation`, le verdict est dégradé en `hold[review]` avec
la cause nommée : « garde du dépôt rejetée en docs-only, mais l'issue liée porte
`documentation` — chemin d'exemption non reproductible hors CI ». Fail-soft explicite, sur
un chemin unique et nommé, plutôt qu'un blocage faux.

Ce n'est pas une réintroduction de paraphrase : la QA ne réévalue pas la règle, elle
constate qu'une entrée que la garde n'a pas pu lire aurait changé son verdict.

**Option écartée (hors portée déclarée du ticket) :** faire accepter à
`verify-pipeline.sh` une variable `GITHUB_LINKED_ISSUE_LABELS` court-circuitant l'appel
réseau. Plus propre, mais modifie une garde, ce que la portée du ticket exclut. Signalé
comme suite possible, non implémenté ici.

### D5 — Dépôt sans garde exécutable

`claude-pilot-py` et `wizzard` n'ont aucun des chemins de D1. Le verdict pipeline devient
alors `PIPELINE: not-applicable (no executable guard found in <repo>: checked
scripts/verify-pipeline.sh, scripts/plan-doc-check.sh)` — **signalé comme tel, jamais jugé
sur les règles d'un autre dépôt** (AC1, seconde phrase). Ce n'est pas un blocage : une
absence de garde n'est pas une violation.

### D6 — Ce que Step 2 devient

Dans `mika/skills/bundled/qa-review/system_prompt.md`, Step 2 :

- **Supprimer** les checks 1 (plan doc exists) et 2 (source changes exist) — subsumés par
  la garde.
- **Supprimer** les trois bypass paraphrasés : « Pipeline-exempt label bypass »,
  « Pipeline-Exempt trailer bypass » (dont la ligne 190 « mirrors … verbatim », et le
  `grep -E '^Pipeline-Exempt: (docs-only|code-only)…'` de la ligne 187), et
  « Tactical-surface auto-detect bypass ». Les gardes portent leurs propres exemptions.
- **Conserver** le check 3 (nouvelles dépendances externes → `hold[review]`) : aucune
  garde ne le porte, il n'est donc pas une paraphrase.
- **Ajouter** la section d'exécution D1–D5, et la règle de report : la sortie de chaque
  garde est citée verbatim dans le verdict, jamais résumée.

**Conséquence à assumer** : retirer le tactical-surface auto-detect **resserre** la QA sur
`scripts/`, `os/`, `Dockerfile.`, `skills/bundled/_shared/`. C'est l'alignement voulu — la
CI bloquait déjà ces PR ; la QA les approuvait. Une PR CI-yaml pure reste passante
(`.github/` n'est ni docs ni source → `!docs && !source` → warn + pass).

### D7 — Le second point de blocage, dans Step 2.5.1

Corriger Step 2 ne suffit pas. Step 2.5.1 émet `block[pipeline]` — « No plan callout
found » — dès qu'aucun callout `> - **Plan:**` n'existe au corps de l'issue ou de la PR.
Vérifié : mika#2167 n'a **ni callout Plan ni issue liée** (`closingIssuesReferences: []`).
Elle serait donc rebloquée en 2.5.1 sur le même motif, un `block[pipeline]` pour plan
absent, après que Step 2 l'a laissée passer. AC3 ne serait pas tenu.

Correctif : rendre 2.5.1 **conditionnel à la décision de la garde**. Quand les gardes du
dépôt passent et qu'aucun plan n'est trouvé, 2.5.1 n'émet plus `block[pipeline]` mais
`PLAN-AC VERIFICATION: skipped (no plan on branch; <repo> guard passed)`, et la revue
continue vers Step 3. Quand un plan existe, 2.5 s'exécute inchangé (AC5).

L'absence de plan reste un blocage dans le seul cas où une garde l'exige — c'est
`plan-doc-check.sh` sur `mika-platform`, qui sortira alors non nul et bloquera en D6 sans
que 2.5.1 ait à le redire.

### D8 — Prompts frères

Vérifié, ne pas supposer :

- **`qa-review-webhook-success/system_prompt.md`** — une seule mention (`:43`), descriptive
  (« le verdict est `block[ac]` ou `block[pipeline]`, ce qui est un résultat légitime »).
  Aucune règle réimplémentée. **Aucun changement.**
- **`qa-review-build-callback/system_prompt.md`** — ne réimplémente aucune garde ; il
  relit le plan et re-vérifie les AC. Deux points touchent ce ticket : `:30` émet
  `block[pipeline]` si le plan est illisible, et `:205` décrit `block[pipeline]` comme
  « missing plan doc, no source changes, plan callout/file/section missing ». Le premier
  reste (plan illisible en callback = échec structurel, orthogonal). Le second est une
  **table de description de verdict** devenue inexacte après D6/D7 : la mettre à jour pour
  décrire ce que le verdict signifie désormais (« la garde pipeline du dépôt cible a
  rejeté, ou le plan est illisible en callback »).
- **`qa-review/skill.toml`** — `max_prompt_size = 65536`, prompt actuel ~58 Ko. Le solde
  net de D6 est une **suppression** (trois bypass + deux checks retirés contre une section
  d'exécution ajoutée) ; vérifier néanmoins la taille après édition, la marge étant de
  ~7 Ko.

## Acceptance criteria

- **AC1** — Le verdict pipeline est produit en exécutant les gardes découvertes dans le
  dépôt cible (D1), dans un worktree détaché sur la ref de la PR (D2), et le verdict est
  la conjonction de leurs codes de sortie. Aucun chemin du prompt ne réévalue une règle
  qu'une garde porte déjà. Un dépôt sans garde rend `PIPELINE: not-applicable` avec la
  liste des chemins cherchés (D5), et n'est jamais jugé sur les règles d'un autre dépôt.
- **AC2** — Un trailer `Pipeline-Exempt:` valide pour le dépôt concerné est honoré, quel
  que soit son vocabulaire, parce que c'est la garde du dépôt qui le lit. Vérification :
  rejouer mika-platform#203 (`no-plan — <raison>`) produit un verdict non bloquant ; les
  deux gardes du dépôt sortent 0 (mesuré au grooming).
- **AC3** — La QA ne bloque plus sur l'absence de `docs/plans/*.md` quand la garde ne
  l'exige pas — **ni en Step 2, ni en Step 2.5.1** (D7). Vérification : rejouer mika#2167
  (`docs && source`, sans callout Plan, sans issue liée) produit un verdict non bloquant
  sur ce motif, et le verdict cite `verify-pipeline.sh` exit 0.
- **AC4** — Anti-vacuité. Une PR qui viole réellement une garde reste bloquée en
  `block[pipeline]` : (a) `!docs && source` sans trailer sur `mika` → la garde sort 1
  (mesuré : `REJECT: code-only PR`) ; (b) des changements source sans plan ni trailer sur
  `mika-platform` → `plan-doc-check.sh` sort 1. Le même diff avec trailer valide passe —
  la sonde porte ses deux contrôles.
- **AC5** — Non-régression. `DIFF ANALYSIS` (Step 3) et la revue de fond restent produites
  inchangées. `PLAN-AC VERIFICATION` reste produite et gatante **quand un plan existe** ;
  elle n'est passée en `skipped` que dans le cas D7 (pas de plan **et** gardes passantes).
  Le check 3 (nouvelles dépendances) est conservé.
- **AC6** — Le verdict cite la sortie verbatim de chaque garde exécutée, avec son chemin et
  son code de sortie. Un verdict pipeline qui n'exhibe pas la sortie de la garde qu'il
  prétend refléter est la faute que ce ticket répare, une couche plus haut.

## Portée

`mika/skills/bundled/qa-review/system_prompt.md` (D1–D7),
`mika/skills/bundled/qa-review-build-callback/system_prompt.md` (D8, table de description
de verdict). `qa-review-webhook-success` : vérifié, aucun changement.

**Hors portée.** (a) Modifier une garde de dépôt, y compris l'ajout d'une variable
d'environnement à `verify-pipeline.sh` pour rendre le chemin `documentation` reproductible
(D4, signalé comme suite). (b) La question de savoir si un plan groomé doit rester
obligatoire — il le reste comme norme ; ce ticket porte sur le fait que la QA enforce une
règle que le code a retirée, et un vocabulaire qui n'est pas celui du dépôt jugé.
(c) Unifier les vocabulaires d'exemption entre dépôts : exécuter chaque garde rend
l'unification inutile au verdict.

## Risques

1. **Le worktree jetable fuit.** Un `run_shell` interrompu entre `add` et `remove` laisse
   un worktree dans le dépôt partagé. Mitigation : `trap … EXIT` dans la commande, chemin
   sous un préfixe fixe et `git worktree prune` en préambule.
2. **Écritures concurrentes sur le `.git` partagé.** `worktree add` écrit dans le dépôt que
   d'autres processus utilisent. L'opération est brève (0,23 s) et git la verrouille ;
   risque jugé faible mais réel sous dispatch parallèle.
3. **Le prompt grossit au-delà de 64 Ko.** Solde net attendu négatif (D8), à vérifier.
4. **D4 dégrade en `hold[review]` un cas que la CI passe.** C'est un faux *hold*, pas un
   faux *block* — moins coûteux que l'inverse, et nommé dans le verdict.
5. **Une garde nouvelle à un nom hors liste est manquée** (D1). Elle est signalée par la
   liste des chemins cherchés (AC1), donc visible ; elle ne produit pas un faux verdict.

---
title: "Une garde qui hérite son interprète de l'image du runner a une exigence non déclarée, et son échec porte le mauvais motif"
category: ci-cd
date: 2026-09-29
tags:
  - ci
  - python
  - tomllib
  - setup-python
  - egress
  - exit-codes
  - fail-closed
  - guard-observability
severity: medium
affected_components:
  - .github/workflows/ci.yml
  - scripts/lib/egress_manifest_lint.py
  - scripts/verify-egress-manifest.sh
  - scripts/verify-egress-uniqueness.sh
  - scripts/test-verify-egress-manifest.sh
related_issues:
  - mika#2408
---

# Une garde qui hérite son interprète de l'image du runner

## Le défaut, mesuré

mika#2408 a livré `scripts/verify-egress-manifest.sh` (lockstep manifeste↔code
sur les sinks sortants) et fait dériver `verify-egress-uniqueness.sh` du même
manifeste (AC5). Le moteur lit du TOML par `tomllib`, **stdlib depuis Python
3.11**. Les deux jobs CI ont rougi :

```
ModuleNotFoundError: No module named 'tomllib'
```

`ubuntu-22.04` — le **seul** runner de ce dépôt, sur ses 33 jobs — porte Python
3.10. Le lint était correct, vert localement (67 assertions), et incapable de
tourner en CI.

## Trois défauts, pas un — et les deux derniers ne sont pas dans le rapport

Le rapport CI nomme le premier. La reproduction locale a révélé les deux autres.

### 1. L'exigence de version était écrite, jamais déclarée

Le script disait déjà « tomllib, stdlib >= 3.11 » dans son en-tête et dans son
message d'erreur `python3` introuvable. **Une exigence énoncée dans un
commentaire n'est pas une exigence appliquée.** Rien ne l'imposait au runner, et
l'image décidait en silence si une garde de sûreté pouvait tourner.

### 2. Le code de sortie portait le mauvais motif

Un `import tomllib` nu rend un traceback, donc l'exit **1** de l'interpréteur.
Or, dans le vocabulaire de ce lint :

| exit | signification |
|---|---|
| 0 | aucune violation |
| **1** | **violation(s) trouvée(s)** — « sink non déclaré » |
| 2 | manifeste illisible / absent / vide (fail-closed) |

**Un interpréteur trop ancien se lisait donc comme un sink non déclaré.** Un
opérateur devant ce rouge part chercher un chemin d'egress fantôme ; la cause est
une version de Python. Le motif juste est 2 : le manifeste est illisible, ce qui
est exactement la catégorie fail-closed déjà prévue.

### 3. Un fail-closed qui rapportait « exit 0 » sur un échec

`verify-egress-uniqueness.sh` dérivait ses PATTERNS ainsi :

```bash
derive_rc=0
while IFS= read -r host; do
    [[ -n "$host" ]] && PATTERNS+=("$host")
done < <(python3 -B "$ENGINE" --confined-hosts "$ROOT" || { derive_rc=$?; })

if [[ $derive_rc -ne 0 || ${#PATTERNS[@]} -eq 0 ]]; then
```

Une substitution de processus tourne dans un **sous-shell** : l'affectation de
`derive_rc` y meurt. Le refus affichait `(exit 0, 0 pattern(s) obtenu(s))` sur
une dérivation qui avait échoué. Le fail-closed mordait quand même — mais par
son **second** terme seulement. Conséquence : une dérivation qui échouerait en
écrivant tout de même une ligne ne serait pas refusée du tout.

Forme correcte — la capture précède la boucle, donc le code de retour est vrai :

```bash
derived=$(python3 -B "$ENGINE" --confined-hosts "$ROOT") || derive_rc=$?
while IFS= read -r host; do
    [[ -n "$host" ]] && PATTERNS+=("$host")
done <<< "$derived"
```

## Le remède

**Déclarer l'interprète, pas l'espérer.** Les deux jobs qui lancent le moteur
épinglent leur Python :

```yaml
- uses: actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1  # v6.3.0
  with:
    python-version: '3.12'
```

**Site unique pour le refus.** L'import passe sous garde dans
`scripts/lib/egress_manifest_lint.py` — le seul site d'import — et sort en **2**
avec un motif nommé qui dit la version vue, l'exigence, et le geste (CI :
`setup-python` ; local : un python3 ≥ 3.11). Les deux consommateurs bash en
héritent sans dupliquer la vérification.

### Deux remèdes écartés, avec leur raison

**Repli sur `tomli`** (`try: import tomllib / except: import tomli as tomllib`) —
refusé. `tomli` n'est pas une dépendance de ce dépôt et est absent du runner,
donc le repli ne répare pas la CI ; et il donnerait au même interpréteur **deux
régimes** selon ce qu'un environnement porte par accident. Un refus qui nomme
l'exigence vaut mieux qu'un lint dont on ne sait pas s'il a tourné.

**Basculer les jobs sur `ubuntu-24.04`** (Python 3.12 par défaut) — refusé. Une
ligne suffirait, mais les 33 jobs de ce dépôt sont uniformément `ubuntu-22.04` :
ça crée une asymétrie non déclarée, et l'exigence resterait implicite — « l'image
porte un Python assez récent » au lieu de « ce lint exige 3.11 ».

## Les deux gardes, et pourquoi il en faut deux

`scripts/test-verify-egress-manifest.sh` (déjà câblé en CI) gagne deux cas.

**N16 — le câblage.** Chaque job de `ci.yml` qui lance le moteur doit déclarer
`actions/setup-python`. Le prédicat est **par job**, jamais par fichier : un
`grep -q setup-python` sur `ci.yml` répondrait vrai pendant que le job fautif
n'en a aucun — c'est très exactement l'erreur à ne pas commettre. Anti-vacuité
portée par la cardinalité (≥ 20 jobs découpés, ≥ 2 consommateurs vus) : si le
découpage cessait de voir les jobs, l'assertion se lirait comme un câblage sain
(classe mika#2205). **Vue rouge** en retirant l'étape d'un job — elle nomme le
job fautif.

**N17 — le refus.** Un Python sans `tomllib` doit rendre **2**, pas 1, et nommer
l'exigence. Le Python le plus ancien disponible ici comme sur le runner armé est
≥ 3.12, donc l'absence est simulée par un shadow `tomllib` sur `PYTHONPATH` qui
lève à l'import : la propriété testée est la **conduite du lint face à cette
exception**, et elle ne dépend pas de la façon dont l'exception survient. Une
anti-vacuité mesure que le shadow masque bien la stdlib — un shadow qui masque
sans lever se lirait comme un refus obtenu.

N16 seul laisserait un poste local sur un traceback ; N17 seul laisserait la CI
rouge. Le câblage et le refus sont deux propriétés distinctes.

## La règle

> **Un lint qui dépend d'une version d'outil doit la déclarer là où il tourne, et
> refuser lisiblement là où elle manque — dans un code de sortie qui ne se
> confond pas avec la violation qu'il cherche.**

Le corollaire est le plus coûteux à retrouver : **avant de vérifier qu'une garde
détecte, vérifier que son échec dit la bonne chose.** Ici le rouge était vrai et
son motif faux, ce qui envoie le lecteur à l'opposé de la cause.

## Sondes

```bash
# La garde tourne-t-elle, et sur quoi ?
bash scripts/verify-egress-manifest.sh --report     # attendu 0 + le compte scanné
bash scripts/verify-egress-uniqueness.sh            # attendu 0 + N hosts dérivés
bash scripts/test-verify-egress-manifest.sh         # attendu 0, 75 assertions
```

**Halte 1 — exit 2 avec un message `tomllib`.** Le `python3` du PATH est
antérieur à 3.11. En CI, c'est l'étape `setup-python` qui manque au job (N16 le
dit) ; en local, c'est l'interpréteur. **Ce n'est pas un sink non déclaré** — ne
pas partir lire le manifeste.

**Halte 2 — exit 1.** Là, c'est bien une violation d'egress : la sortie nomme la
direction (D1 à D4) et le geste. On **déclare**, on n'allowliste pas (doctrine
mika#2201).

**Halte 3 — `verify-egress-uniqueness.sh` rend 0 avec 0 host.** Impossible
depuis ce correctif, et c'est le point : une garde de confinement sans pattern se
lirait exactement comme une garde qui passe.

## Voisins

- `docs/solutions/ci-cd/ci-rust-toolchain-version-mismatch.md` — même classe, une
  version d'outil qui diverge entre local et CI ; là c'était rustfmt.
- `docs/solutions/ci-cd/2026-08-27-porting-a-precommit-detector-to-ci-parity-traps.md`
  — les pièges de parité quand un détecteur change d'environnement d'exécution.
- `docs/egress/README.md` — schéma du manifeste et procédure d'ajout d'un sink.

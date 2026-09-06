---
issue: 2211
type: fix
title: "Le pilote écrit le corps de PR hors worktree (/tmp) → policy:deny → pas de PR propre"
status: groomed-pending
---

# Plan — mika#2211 : le corps de PR doit rester dans le worktree, jamais hors-worktree

## Contexte et cause racine (vérifiée en code)

Le prompt N'ordonne PAS d'écrire dans `/tmp`. `mika/.claude/commands/mika.md:68` dit :

```
gh pr create --repo senara-solutions/mika --title "<title>" --body "<body>"
```

— corps **inline**. Le pilote (Claude Code headless) **dévie** de lui-même vers
`--body-file /tmp/pr-body-<N>.md` pour un corps long/multiligne (réflexe outillage courant).
Cette écriture **hors du worktree** tombe sur la permission-policy de claude-pilot :
`[policy:deny] Write: /tmp/pr-body-<N>.md`. `gh pr create --body-file /tmp/…` échoue alors
(fichier absent), le pilote **pose une question** (« Dis-moi si tu veux que je le colle ») =
spawn mort, et se termine SANS PR. dispatch-lib détecte worktree dirty + zéro PR → recovery
mika#1282 → **draft wip-rescue**. Preuve : stderr session #2195 (`675479e5-…`), ticket #2211.

## Objectif

Rendre l'étape « ouvrir la PR » robuste : un corps long ne provoque jamais une écriture
hors-worktree. Un dispatch nominal ouvre une PR **non-draft** directement.

## Décision de conception (révisée après mika-arch first-pass, F1)

Forme canonique retenue = **fichier de corps DANS le worktree**, passé à `--body-file`.
C'est la direction #1 (préférée) du ticket. Elle évite à la fois le `[policy:deny]` de `/tmp`
ET la fragilité d'un heredoc `<<'BODY'` (mika-arch F1 : un corps de PR généré peut contenir
la ligne délimitrice `BODY` — bloc de code, section d'AC — et terminer le heredoc
prématurément, reproduisant l'échec). Un fichier dans le worktree est insensible au contenu.

- Emplacement : un fichier de corps sous le worktree, ex. `pr-body.md` à la racine du worktree,
  **supprimé après `gh pr create`** (pour ne pas salir le worktree → ne pas déclencher la
  détection dirty). Comme `docs/plans/` et le code sont committés avant l'étape PR, le worktree
  est propre à ce moment ; écrire puis supprimer `pr-body.md` autour du `gh pr create` garde le
  worktree propre.

Rejeté : (a) heredoc stdin `--body-file -` — fragile au contenu (F1) ; (b) élargir la
permission-policy pour autoriser `/tmp/pr-body-*` (direction #3 du ticket) — élargit la surface
d'écriture hors-worktree pour un gain nul ; la policy hors-worktree est une garde voulue.

## Acceptance criteria

- **AC1** : `mika/.claude/commands/mika.md` (étape PR) instruit la création de PR avec le corps
  écrit dans un fichier **sous le worktree** (puis supprimé après création), passé à
  `--body-file`, et **interdit explicitement** toute écriture du corps hors du worktree
  (`/tmp`). (tie-back : ticket §Fix direction #1)
- **AC2** : `mika/skills/bundled/self-dev/system_prompt.md` porte une garde transversale sur
  l'étape PR : « n'écris jamais le corps de PR hors du worktree ; écris-le sous le worktree ».
  Cette garde vit dans le prompt PILOTE, donc elle couvre les pilotes de **tous** les repos
  (mika, mika-cloud, mika-skills) indépendamment du `mika.md` du repo cible. (résout mika-arch F2
  au niveau transversal — l'invariant de confinement est répliqué au bon endroit unique)
- **AC3** (anti-régression, du ticket) : un dispatch nominal ouvre une PR **non-draft**
  directement — pas de draft wip-rescue — et le pilote n'émet **pas de question terminale** sur
  l'étape PR quand le corps est long.

## mika-arch first-pass — résolution des findings

- **F1 (bloquant, heredoc fragile)** : RÉSOLU — forme canonique = fichier dans le worktree, pas
  de heredoc (voir Décision de conception).
- **F2 (surface pilote non inventoriée)** : RÉSOLU — `senara-solutions/claude-pilot-py` ne
  contient AUCUNE instruction `gh pr create` (vérifié : grep vide) ; le pilote tient l'instruction
  du `mika.md` du repo cible. AC2 place la garde dans le prompt pilote (self-dev), qui est la
  surface transversale couvrant tous les repos. Les `mika.md` de `mika-cloud` et `mika-skills`
  portent la même instruction inline latente ; comme AC2 (garde pilote) couvre déjà leurs
  pilotes, leur mise à niveau ligne-par-ligne (aligner leur `gh pr create` sur la forme
  fichier-worktree) est **hors périmètre de ce fix mika-centré** → follow-up séparé si l'on veut
  la cohérence d'instruction par-repo (voir Hors périmètre).
- **F3 (pas de détection auto de régression)** : la détection de repli existe déjà — la recovery
  dirty-worktree (mika#1282) ouvre un wip-rescue draft dès qu'un pilote sort dirty-sans-PR, ce qui
  rend la régression visible (c'est précisément le symptôme qui a mené à ce ticket). Une sonde
  dédiée (grep des logs de session pour `[policy:deny]` sur `/tmp/pr-body-*`) est notée comme
  amélioration future, hors périmètre de ce fix prompt-only.

## Phases

1. **Éditer `mika/.claude/commands/mika.md`** — remplacer la ligne `gh pr create … --body` par la
   forme fichier-dans-worktree (écrire `pr-body.md` sous le worktree → `--body-file pr-body.md` →
   `rm pr-body.md`) + phrase de garde « jamais hors-worktree ». (AC1)
2. **Éditer `self-dev/system_prompt.md`** — garde transversale sur l'étape PR. (AC2)
3. **Vérification** — relire les deux diffs : aucune écriture hors worktree ; la forme est
   insensible au contenu du corps ; la garde est explicite. (AC3 = anti-régression observée au
   prochain dispatch nominal, filet = mika#1282.)

## Hors périmètre

- Ne PAS élargir la permission-policy de claude-pilot.
- Ne PAS toucher la recovery dirty-worktree (mika#1282) — elle reste le filet ; ce fix tarit la
  source.
- Mise à niveau des `mika.md` de `mika-cloud`/`mika-skills` (même instruction inline latente) —
  follow-up séparé ; AC2 couvre déjà leurs pilotes via le prompt transversal.
- Sonde dédiée de détection `[policy:deny]` — amélioration future (F3).
- Le log-dir perdu (mika#2165) est un ticket distinct.

## Fichiers touchés

- `mika/.claude/commands/mika.md` (étape PR)
- `mika/skills/bundled/self-dev/system_prompt.md` (garde transversale étape PR)

---
issue: 2211
type: fix
title: "Le pilote écrit le corps de PR hors worktree (/tmp) → policy:deny → pas de PR propre"
status: groomed-pending
---

# Plan — mika#2211 : le corps de PR doit rester dans le worktree (ou stdin), jamais /tmp

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

`mika/skills/bundled/self-dev/system_prompt.md` ne cadre pas non plus l'étape corps-long : le
trou est que l'instruction est correcte pour un corps court mais ne protège pas le cas long.

## Objectif

Rendre l'étape « ouvrir la PR » robuste : un corps long ne provoque jamais une écriture
hors-worktree. Un dispatch nominal ouvre une PR **non-draft** directement.

## Décision de conception

Deux surfaces à corriger, direction retenue = **stdin** (la plus self-contained, zéro écriture
disque) avec une **interdiction explicite** en garde :

1. **`mika/.claude/commands/mika.md`** (étape PR, ~ligne 68) : remplacer la forme `--body
   "<body>"` par une forme qui gère le corps long sans fichier temporaire —
   `gh pr create --repo senara-solutions/mika --title "<title>" --body-file - <<'BODY'` … `BODY`
   (corps sur stdin via heredoc). Ajouter une phrase de garde explicite : « N'écris JAMAIS le
   corps de PR hors du worktree (pas de `/tmp`) ; passe-le par stdin, ou dans un fichier sous le
   worktree si un fichier est nécessaire. »
2. **`mika/skills/bundled/self-dev/system_prompt.md`** : ajouter la même garde d'une ligne dans la
   section qui décrit l'étape PR (ou la créer si absente), pour que le prompt pilote porte
   l'interdiction indépendamment du /mika.md du repo cible.

Rejeté : élargir la permission-policy pour autoriser `/tmp/pr-body-*` (direction #3 du ticket) —
élargit la surface d'écriture hors-worktree pour un gain nul vs stdin ; la policy hors-worktree
est une garde voulue, pas un obstacle.

## Acceptance criteria

- **AC1** : `mika/.claude/commands/mika.md` instruit la création de PR avec le corps passé par
  stdin (`--body-file -`) OU inline, et **interdit explicitement** toute écriture du corps hors du
  worktree (`/tmp`). (tie-back : ticket §Fix direction #1/#2)
- **AC2** : `mika/skills/bundled/self-dev/system_prompt.md` porte la même garde « corps de PR
  jamais hors-worktree » sur l'étape PR.
- **AC3** (anti-régression, du ticket) : un dispatch nominal ouvre une PR **non-draft**
  directement — pas de draft wip-rescue — et le pilote n'émet **pas de question terminale** sur
  l'étape PR quand le corps est long.

## Phases

1. **Éditer `mika/.claude/commands/mika.md`** — remplacer la ligne `gh pr create … --body` par la
   forme stdin heredoc + phrase de garde. (AC1)
2. **Éditer `self-dev/system_prompt.md`** — garde d'une ligne sur l'étape PR. (AC2)
3. **Vérification** — relire les deux diffs : la forme proposée n'écrit rien hors worktree ;
   la garde est explicite et non ambiguë. (AC3 est validé en conditions réelles au prochain
   dispatch nominal — anti-régression observée, pas un test unitaire.)

## Hors périmètre

- Ne PAS élargir la permission-policy de claude-pilot.
- Ne PAS toucher la recovery dirty-worktree (mika#1282) — elle reste le filet ; ce fix tarit la
  source.
- Le log-dir perdu (mika#2165) est un ticket distinct.

## Fichiers touchés

- `mika/.claude/commands/mika.md` (étape PR)
- `mika/skills/bundled/self-dev/system_prompt.md` (garde étape PR)

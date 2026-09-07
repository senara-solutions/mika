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
- **Durcissement ajouté à l'implémentation** : `/pr-body.md` est ajouté au `.gitignore`. La
  suppression seule laisse un trou — si `gh pr create` échoue, le fichier survit et
  `git status --porcelain` (la sonde exacte que dispatch-lib interroge) le rapporte, déclenchant
  la recovery wip-rescue mika#1282 que ce fix existe pour tarir. `git status --porcelain` ne
  rapporte pas les fichiers ignorés : l'ignore rend le résidu inoffensif au lieu de compter sur
  un `rm` qui n'a pas lieu sur le chemin d'échec. Le ticket l'autorise explicitement
  (« ou un fichier ignoré du worktree », direction #1).

Rejeté : (a) heredoc stdin `--body-file -` — fragile au contenu (F1) ; (b) élargir la
permission-policy pour autoriser `/tmp/pr-body-*` (direction #3 du ticket) — élargit la surface
d'écriture hors-worktree pour un gain nul ; la policy hors-worktree est une garde voulue.

## Acceptance criteria

- [x] **AC1** : `mika/.claude/commands/mika.md` (étape PR) instruit la création de PR avec le corps
  écrit dans un fichier **sous le worktree** (puis supprimé après création), passé à
  `--body-file`, et **interdit explicitement** toute écriture du corps hors du worktree
  (`/tmp`). (tie-back : ticket §Fix direction #1)
- [x] **AC2** : `mika/skills/bundled/self-dev/system_prompt.md` porte une garde transversale sur
  l'étape PR : « n'écris jamais le corps de PR hors du worktree ; écris-le sous le worktree ».
  Cette garde vit dans le prompt PILOTE, donc elle couvre les pilotes de **tous** les repos
  (mika, mika-cloud, mika-skills) indépendamment du `mika.md` du repo cible. (résout mika-arch F2
  au niveau transversal — l'invariant de confinement est répliqué au bon endroit unique)
  — **voir la correction de prémisse ci-dessous : la garde est posée en DEUX endroits, pas un.**
- [ ] **AC3** (anti-régression, du ticket) : un dispatch nominal ouvre une PR **non-draft**
  directement — pas de draft wip-rescue — et le pilote n'émet **pas de question terminale** sur
  l'étape PR quand le corps est long. *(observable au prochain dispatch nominal après merge — ne
  peut pas être coché depuis cette PR ; filet inchangé = mika#1282.)*
- [x] **AC4** (ajouté à l'implémentation, voir ci-dessous) : la garde transversale est portée par
  le `PROMPT` composé dans `dispatch-lib.sh`, seul canal que le processus pilote lit réellement.

### Correction de prémisse sur AC2 (constatée à l'implémentation)

AC2 affirmait que `self-dev/system_prompt.md` est « le prompt PILOTE ». **Il ne l'est pas.**
Vérifié en code : `_run_pilot_sandboxed claude-pilot … --command "$ENTRY_COMMAND" … -- "$PROMPT"`
(`dispatch-lib.sh`) — le processus pilote ne reçoit que deux entrées, la commande d'entrée
résolue depuis le worktree du repo **cible** (donc le `.claude/commands/mika.md` de ce repo) et
le `PROMPT` composé par dispatch-lib. `self-dev/system_prompt.md` est le system prompt de
**mika-dev** (l'agent qui dispatche) et n'atteint jamais le pilote ; une garde qui n'y vivrait
que là serait décorative pour la panne qu'elle prétend fermer, et la couverture
mika-cloud/mika-skills annoncée par AC2 ne serait pas obtenue.

La garde est donc posée aux deux endroits, chacun pour ce qu'il couvre réellement :

- `self-dev/system_prompt.md` (Rule 12) — pour ce que **mika-dev compose lui-même**
  (`iteration_context`, prompts free-text). C'est AC2 à la lettre, et c'est utile à ce titre.
- `dispatch-lib.sh` (`_PR_BODY_CONTAINMENT_RULE`, injectée dans `PROMPT`) — pour ce que **le
  pilote lit**. C'est ce qui réalise l'intention d'AC2 (« couvre tous les repos ») : `mika-cloud`
  et `mika-skills` portent encore l'instruction inline d'avant le fix dans leur `mika.md`, et
  cette ligne les couvre sans les toucher.

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

- `mika/.claude/commands/mika.md` (étape PR) — AC1
- `mika/skills/bundled/self-dev/system_prompt.md` (Rule 12) — AC2
- `mika/skills/bundled/_shared/dispatch-lib.sh` (`_PR_BODY_CONTAINMENT_RULE` + injection dans
  `PROMPT`) — AC4, l'intention d'AC2 sur le seul canal que le pilote lit
- `mika/.gitignore` (`/pr-body.md`) — durcissement du chemin d'échec
- `mika/skills/bundled/_shared/test-dispatch-lib.sh` — garde structurelle anti-régression

## Vérification exécutée

- `bash skills/bundled/_shared/test-dispatch-lib.sh` → 588 passed, 0 failed (dont les 11
  assertions mika#2211 ajoutées).
- `bash skills/bundled/_shared/tests/test_rescue_signal_open_pr.sh` → 63 passed, 0 failed
  (inclut le garde-fou `dispatch-lib passes bash -n`).
- `bash skills/bundled/_shared/tests/test_rescue_closes_guard.sh` → 29 passed, 0 failed.
- `bash skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh` → 11 passed, 0 failed.
- `cargo run -q --bin verify-bundled-skills` → 5/5 checks OK.

AC3 n'est pas vérifiable depuis cette PR : il se mesure au prochain dispatch nominal après merge.

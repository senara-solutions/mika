---
title: "Seed ephemeral scaffold into a tracked worktree dir without dirtying git status (common-dir info/exclude)"
date: 2026-06-06
category: architecture-patterns
module: dev-pilot
problem_type: architecture_pattern
component: development_workflow
severity: high
applies_when:
  - "A dispatch/setup step must place ephemeral files inside a TRACKED directory a tool discovers by path (e.g. Claude Code reads slash commands from `<cwd>/.claude/commands/`)"
  - "Those files would otherwise show as untracked in `git status`, breaking a downstream rebase or a clean-worktree invariant"
  - "A blanket `cp -r` would also overwrite the branch's own tracked versions of same-named files"
  - "Pinning the true cause of a groomed ticket whose stated premise (layer/mechanism) is an unconfirmed hypothesis"
  - "Establishing whether a `.claude/commands/` file is missing from a repo, deleted from it, or merely seeded-and-excluded in a dispatch worktree"
tags:
  - worktree-hygiene
  - dispatch-lib
  - git-exclude
  - polymorphic-mika
  - autonomous-loop
  - phase-0-pin
  - mika-1415
  - mika-2001
  - scope-boundary
---

# Seed ephemeral scaffold into a tracked worktree dir without dirtying git status

## Context

`dispatch-lib`'s worktree setup must make the meta-repo's orchestration slash commands
(`/mika-groom-ticket`, `/mika-revise-plan`, …) resolvable to the inner Claude Code
session, which runs with `--cwd "$WORKTREE_DIR"` and discovers project commands from
`<cwd>/.claude/commands/` (mika#1173). The original code did the obvious thing:

```bash
cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"
```

That single line caused two regressions on **every dispatch** (not every deploy — see
the premise-correction note below):

1. **Clobber.** The mika sub-repo tracks its OWN polymorphic `/mika` (the mika#1255
   fix, 72 lines) plus sub-repo-scoped `/mika-issue`. The blanket copy overwrote them
   with the 260-line meta-repo dispatcher — re-creating the exact pre-#1255 recursion
   bug, with `mika.md` showing as Modified.
2. **Dirty status.** The meta-repo command set has 21 files; the worktree tracks 4, so
   the copy dropped ~18 untracked sibling files (`mika-spawn.md`, `mika-handsoff.md`, …)
   into the tracked tree. A dirty `.claude/commands/` breaks the resume rebase
   (`error: cannot rebase: You have unstaged changes`) — the upstream cause of the
   dirty-worktree class that mika#1414 defends against reactively.

`.claude/commands/` is a **tracked** directory, so — unlike `claude-pilot.json` /
`settings.local.json`, which are gitignored — anything copied into it is visible to git.

## Guidance

When you must place ephemeral scaffold inside a tracked directory that a tool discovers
by path, enforce two invariants:

**1. Never overwrite a file the branch already tracks.** Skip the copy when the
worktree tracks that name — the branch's own version wins:

```bash
if git -C "$worktree_dir" ls-files --error-unmatch ".claude/commands/$base" >/dev/null 2>&1; then
    continue   # the worktree's own /mika (#1255), /mika-issue, … win
fi
cp "$src" "$worktree_dir/.claude/commands/$base" 2>/dev/null || true
```

**2. Shield the copied-in files via the COMMON-dir `info/exclude`.** A linked
worktree's *per-worktree* `$GIT_DIR/info/exclude` is **NOT** consulted for status — only
the **common** dir's `info/exclude` is (verified empirically; see Examples). Append each
seeded filename there so it never appears as untracked:

```bash
common_dir=$(git -C "$worktree_dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) \
    || common_dir=$(git -C "$worktree_dir" rev-parse --git-common-dir 2>/dev/null)
exclude_file="$common_dir/info/exclude"
# idempotent; guard a missing trailing newline before appending
if ! grep -qxF ".claude/commands/$base" "$exclude_file" 2>/dev/null; then
    [ -s "$exclude_file" ] && [ -n "$(tail -c1 "$exclude_file")" ] && printf '\n' >> "$exclude_file"
    printf '%s\n' ".claude/commands/$base" >> "$exclude_file"
fi
```

The full helper is `_seed_worktree_slash_commands()` in
`skills/bundled/_shared/dispatch-lib.sh`.

### Premise-pin discipline (the investigation half)

mika#1415 was groomed as a *deploy-time skill-install* bug — its title named `skills`
and "on deploy," and the worktree survey hypothesized the clobber was "regenerated each
deploy." **None of that survived a Phase 0 pin.** An exhaustive grep across all repos
plus a trace of every `make deploy` sub-step (`build`, `install-mika`,
`install-claude-pilot`, `install-skills`, `restart`, `check-ngrok`) showed the only
writer of `.claude/commands/` into a worktree is `dispatch-lib.sh`'s `cp -r`, firing
**per-dispatch**, not on deploy. `mika skills update` never touches `.claude/commands/`.
The "two distinct surfaces" model (a deploy surface separate from dispatch-lib) collapsed
to one. The survey's "regenerated each deploy" was an unconfirmed correlation, not a
mechanism.

## Effet de lecture : un fichier semé se lit comme un fichier supprimé

Le masquage ci-dessus est délibéré et sert la propreté du rebase. Il a un **effet de
lecture** que ce document ne disait pas, et qui a coûté trois enquêtes : il rend le
mécanisme invisible à qui interroge git. Mesure, dans n'importe quel worktree de
dispatch :

```bash
ls .claude/commands/        | wc -l   # une vingtaine de fichiers présents sur disque
git ls-files .claude/commands/ | wc -l   # 4 — et ce 4 est l'invariant
```

Le premier nombre varie avec le jeu de commandes du meta-repo au moment du dispatch ; le
second est fixé par `b831cbd5` (2026-05-26) et par `mika/CLAUDE.md` § Directory Structure,
qui nomme les quatre : `mika.md`, `mika-doc-audit.md`, `mika-issue.md`, `mika-issues.md`.
L'écart entre les deux, ce sont les commandes d'orchestration du meta-repo — dont
`mika-groom-ticket.md`, `mika-groom-plan-only.md`, `mika-groom-milestone.md`,
`mika-revise-plan.md` — semées par `_seed_worktree_slash_commands` et listées dans
`$(git rev-parse --git-common-dir)/info/exclude`.

**Un observateur qui demande à git voit une absence là où le pipeline voit un fichier qui
marche.** D'où la règle de lecture :

> L'absence dans `git ls-files` d'un fichier `.claude/commands/` présent sur disque est
> l'état **nominal** d'un worktree de dispatch, jamais la preuve d'une suppression.

Le geste qui tranche tient en une commande — elle distingue « jamais suivi ici » de
« retiré », et dans le second cas elle rend le commit et son motif :

```bash
git log --oneline --all -- .claude/commands/
```

Sur ce dépôt elle rend deux commits de cette famille, et ils ne disent pas la même
chose : `b831cbd5` est le retrait des dix-huit commandes meta, `5045762a` (#1095) la
suppression d'une copie re-propagée depuis. Le second est le plus instructif — une
commande meta qui réapparaît ici est traitée comme un défaut à corriger, jamais comme
l'état attendu.

### Le coût, mesuré trois fois

| Quand | Qui | Ce qui a été lu | Ce qui était vrai |
|---|---|---|---|
| 2026-08-26 | mika-qa, à la revue de PR#1994 | `AC3 ❌ — .claude/commands/ groom files removed from repo, Units 3-4 not implementable` | PR#1994 (`8f46caa9`) ne touche aucun fichier sous `.claude/` ; le retrait est `b831cbd5`, antérieur de trois mois |
| 2026-09-19 | mika#2306 | — | a dû re-mesurer `git ls-files` pour établir que ces commandes ne sont pas éditables depuis ce dépôt, et a routé sa livraison par `_FIRE_DISPOSITION_RULE` (`dispatch-lib.sh`), le seul canal que `mika` contrôle |
| 2026-09-21 | mika#2001 | le finding ci-dessus, promu en ticket de suivi | même conclusion, re-établie une troisième fois |

Chacune de ces trois lectures était de bonne foi : c'est la lecture naturelle d'un
mécanisme conçu pour être invisible. La parade n'est pas un nouveau détecteur — celui de
cette classe existe déjà et il passe :
`skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh` assert
`mika-groom-ticket.md` comme cas *meta-only seeded* et vérifie qu'il atterrit dans
`info/exclude` exactement une fois (11/11 le 2026-09-21).

**Mesuré en chemin, et c'est un défaut à part entière : ce script n'est câblé ni au
`Makefile` ni à `.github/workflows/`.** `make test-dispatch-lib` lance
`test-dispatch-lib.sh` et `test_dev_groom_dirty_rescue.sh` — pas celui-ci, dont le seul
point d'entrée est la ligne `# Run:` de son propre en-tête. Il ne tourne donc qu'à la
main, et **une garde qui ne tourne pas se lit exactement comme une garde verte.** Le
câbler est un suivi, nommé dans la PR de mika#2001 et délibérément non fait là-bas
(périmètre documentaire). En attendant, la parade effectivement disponible est cette
section, plus la demi-phrase posée dans `mika/CLAUDE.md` § Directory Structure — sur la
surface que toute session lit au démarrage.

**Corollaire, et c'est la moitié qui coûte le plus cher :** « rétablir le chemin attendu »
en committant l'un de ces fichiers dans le sous-dépôt est la mauvaise réparation. Elle
ferait gagner la copie du sous-dépôt sur celle du meta-repo — invariant 1 ci-dessus —
donc deux sources de vérité divergentes pour le prescripteur de grooming, **et la
divergence ne produirait aucun signal**. Le commentaire du code le dit à l'avance
(« this would silently shadow it », risque mika#1173).

## Why This Matters

- A blanket `cp -r` into a tracked dir is a silent two-headed regression: it both
  shadows the branch's own files and dirties the tree. The git-exclude shield is what
  lets ephemeral scaffold coexist with a clean-worktree invariant.
- The common-dir-vs-per-worktree exclude distinction is non-obvious and load-bearing —
  writing to the wrong one looks correct in code but leaves the tree dirty at runtime.
- **A groomed ticket's premise is a hypothesis, not evidence.** When the stated layer or
  mechanism drives the fix shape, pin it with hard evidence (grep every candidate writer;
  trace every step of the named trigger) before implementing. Here the pin re-titled the
  ticket, relocated the fix from `skills`/deploy to `dispatch-lib`/per-dispatch, and
  surfaced the overlap with the sibling ticket mika#1414 — all before a line of fix code.
  See `failed-pilot-worktree-contamination-signature-2026-05-18.md`, whose recovery
  runbook is still valid but whose root-cause attribution ("the pilot confuses which repo
  it is operating in") this pin corrects: the contamination is mechanical and
  pilot-independent.

## When to Apply

- Seeding tool-discovered config (slash commands, hooks, settings) into a worktree or
  checkout whose own branch may legitimately track same-named files.
- Any setup step that must leave a clean `git status` for a later rebase/resume — keep
  the scaffold out of status via the common-dir `info/exclude`, not by deleting it.
- Before implementing a groomed ticket whose premise names a specific layer/file/trigger
  that you have not independently confirmed.

## Examples

Empirical confirmation of the exclude mechanism (run inside a linked worktree):

```bash
# per-worktree $GIT_DIR/info/exclude — does NOT hide the untracked file:
echo ".claude/commands/probe.md" >> "$(git rev-parse --absolute-git-dir)/info/exclude"
git status --porcelain   # -> still shows "?? .claude/commands/probe.md"

# common-dir info/exclude — DOES hide it:
echo ".claude/commands/probe.md" >> "$(git rev-parse --git-common-dir)/info/exclude"
git status --porcelain   # -> clean
```

Before / after on a dispatch worktree:

| | `.claude/commands/mika.md` | untracked siblings | `git status` |
|---|---|---|---|
| Old `cp -r` | clobbered 72→260 lines | ~18 (`mika-spawn.md`, …) | dirty → rebase fails |
| `_seed_worktree_slash_commands` | preserved (branch HEAD) | shielded via exclude | clean |

Regression gate: `skills/bundled/_shared/tests/test_seed_worktree_slash_commands.sh`
(hermetic; builds a real linked worktree, asserts no-clobber + clean status + idempotency,
and includes an in-suite negative control proving the old `cp -r` would be caught).

## Related Issues

- mika#1415 — this fix (re-scoped from `fix(skills)…on deploy` to
  `fix(dispatch-lib): worktree-setup clobbers sub-repo .claude/commands`).
- mika#1255 — the polymorphic sub-repo `/mika` this clobber undid.
- mika#1414 — sibling defensive fix (detect dirty worktree on resume); this removes the
  cause, #1414 stays the net. Non-overlap: #1414 owns the pre-rebase cleanup, #1415 owns
  the post-rebase command-seed, both inside `_set_up_worktree`.
- mika#1173 — why the meta orchestration commands must be seeded at all.
- `docs/solutions/workflow-issues/failed-pilot-worktree-contamination-signature-2026-05-18.md`
  — the recovery runbook for the same symptom; root-cause attribution corrected by this pin.

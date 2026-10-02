---
title: "Une garde lexicale verte sur son corpus laissait passer l'installateur que le dépôt documente lui-même"
date: 2026-10-02
category: security-issues
module: crates/mika-agent/templates/skills/shell-exec/handlers/run.sh, scripts/test-python-installer-guard.sh
problem_type: security_issue
component: tooling
symptoms:
  - "La v1 du refus d'installateur Python de run_shell passe 32/32 sur le corpus du plan, puis 85/85 sur le harnais"
  - "`uv tool install --force --editable .` traverse la garde : c'est la commande par laquelle `make deploy` installe claude-pilot"
  - "`python3 -X .venv/bin/python -m pip install -e .` traverse la garde et exécute le pip de l'hôte"
  - "Trois termes de la conjonction `host_target_flag` survivent à la mutation sans qu'un cas rougisse"
root_cause: missing_validation
resolution_type: code_fix
severity: high
tags: [shell-exec, run-shell, lexical-guard, pip, uv, pep-668, venv, sentinel, command-position, mutation-testing, code-review, mika-2639, mika-2634, mika-1957]
---

# Une garde lexicale verte sur son corpus laissait passer l'installateur que le dépôt documente lui-même

## Problème

mika#2639 ajoute à `run.sh` (le handler de `run_shell`, hors bwrap) un refus des
installations Python dans l'environnement de l'hôte, après l'incident mika#2634 :
mika-qa avait lancé `pip install -e . --break-system-packages` sur un worktree
claude-pilot et réécrit `~/.local/bin/claude-pilot` (2 h 45 de rail mort). La
première version, tirée d'un plan dont le prototype passait **32/32** et d'un
harnais à **85/85**, laissait passer huit formes qui écrivent dans l'hôte. Une
revue multi-agents les a trouvées ; aucune n'était dans le corpus.

## Symptômes

Rejoués sur une copie de `run.sh` dont `eval` est remplacé par `echo`, au
commit de la v1 :

- `uv tool install --force --editable .` → **passe**. C'est, au caractère près,
  la recette de déploiement du lanceur (`Makefile` de mika-platform, cible
  `install-claude-pilot` : `uv tool install --reinstall --force --editable
  ./$(CLAUDE_PILOT_DIR)`). Un modèle refusé sur `pip install` qui cherche « la
  commande documentée » rejoue mika#2634 par l'autre porte.
- `PIP_BREAK_SYSTEM_PACKAGES=1 python3 -X .venv/bin/python -m pip install -e .`
  → **passe**, et lance le pip de l'hôte.
- `pip --cache-dir /tmp/c install -e .`, `pip 'install' …`,
  `pip config set global.break-system-packages true`,
  `python3 setup.py develop --user`, `.venv/bin/pipx install x` → **passent**.
- Le harnais reste vert si l'on remplace par `true` le test `_PY_TOOL_RE` ou
  `_PY_INSTALL_VERB_RE`, ou si l'on rétrécit la frontière droite à
  `[[:space:]]`.

## Ce qui n'a pas marché

- **Un corpus dérivé du ticket.** Le plan a construit ses cas à partir des
  verbatims mesurés (`pip install … --break-system-packages`, `uv pip install`)
  et des formes d'évasion de mika#1957 (`sh -c`, `eval`, chemin absolu, `;`,
  `$( )`). C'est le corpus de *ce qui s'est passé*, pas de *ce qui écrit au même
  endroit*. Le 32/32 attestait que la garde reconnaissait l'incident, pas
  qu'elle fermait la classe.
- **Un contrôle négatif global.** Neutraliser tout le bloc fait bien rougir le
  harnais (V5), mais cela prouve seulement que *le bloc* compte. Cela ne prouve
  pas que *chaque terme* d'une conjonction compte : trois sur cinq étaient
  inertes (même leçon que mika#2277).
- **« Neutraliser la forme permise, puis scanner le reste »**, appliqué partout.
  L'inversion est juste : l'ERE n'a pas de lookbehind pour dire « un chemin qui
  n'est pas un venv ». Mais la v1 neutralisait un chemin `…/.venv/bin/python`
  **où qu'il soit** sur la ligne, y compris comme valeur d'option. La sentinelle
  avalait alors le `-m pip` qui suivait, et ce `-m pip` appartenait au python
  de l'hôte.

## Solution

Dans `run.sh`, bloc `# --- mika#2639: python installer containment ---` :

1. **Neutraliser en position de commande seulement.** `_PY_VENV_RE` (run.sh:211)
   n'apparie un installateur de venv qu'en début de ligne ou après un séparateur
   (`;` `&` `|` `(` `{` backtick `!`), espaces et un guillemet ouvrant permis. Le
   préfixe de chemin exclut les séparateurs, pour ne pas enjamber une commande.
   La substitution tourne deux fois, parce qu'un séparateur consommé par un
   appariement est la frontière gauche du suivant. Prix nommé : un pip de venv
   derrière un enrobeur (`env X=1 …`, `timeout 60 …`, `sh -c "…"`) est refusé.
   C'est un refus fail-closed, la même posture que la liste fermée des noms de
   venv.
2. **Des frontières droites sur les neutraliseurs**, consommées puis réémises :
   sans elles, `.venv/bin/pipx` perdait son `pip` avant l'étape 2.
3. **Un écart partagé `_PY_GAP`** (run.sh:232) entre binaire et sous-commande :
   une option avec au plus un argument séparé, puis une sous-commande
   éventuellement entre guillemets.
4. **`uv tool install|upgrade`** (`_PY_UV_TOOL_RE`, run.sh:241), sous le motif
   `host_installer`. `uvx`, `uv tool run` et `uv run` restent ouverts : ils
   n'installent rien sur le PATH.
5. **Les trois orthographes de l'opt-out PEP 668** (run.sh:263-266) : le drapeau,
   son jumeau d'environnement `PIP_*=`, et `pip config set
   <section>.break-system-packages|user|target|prefix`. `setup.py` entre dans les
   installateurs nommés, et `develop` dans les verbes.

L'ensemble de motifs reste `{host_installer, host_target_flag}`, parce que
c'est un format de fil que les requêtes opérateur groupent.

Dans le harnais, chaque échappée a son cas `refused`. Il y a aussi des contrôles
de bruit qui passent le pré-filtre sans être une installation (N6
`--with-uv-backend`, N7 `pip list --user`, N8 `pip download --target`). F14 et
F17 visent maintenant un shim absolu dans la fixture, au lieu du vrai
`/usr/bin/pip`. Bilan : 85 → 166 cas, dont 44 rouges sur la v1, et les 14 mutants
à un terme sont tous tués.

## Pourquoi ça marche

Une garde lexicale tient sur deux discriminants. *Quel environnement* : le venv
n'est l'environnement que s'il est **l'interpréteur invoqué**, donc la
neutralisation doit lire la position syntaxique, pas la seule présence d'un
chemin. *Quelle écriture* : la classe à fermer, c'est « ce qui écrit dans
`~/.local/bin` ou le site-packages de l'hôte », pas « ce que l'agent a tapé le
2 octobre ». La population de cette classe se recense **dans le dépôt**. Le
dépôt documente sa propre recette d'installation du binaire protégé, et c'est
la première porte qu'un modèle refusé essaiera.

## Prévention

- **Recenser les écrivains de l'artefact protégé avant de figer le corpus.**
  Pour une garde qui protège un fichier ou un binaire, grep le dépôt et le
  méta-dépôt pour chaque commande qui l'écrit (cibles `make`, scripts de
  déploiement, CLAUDE.md). Chacune devient un cas `refused`, ou une exemption
  écrite avec son motif.
- **Toute forme « permise » neutralisée par sentinelle porte une contrainte de
  position**, et un cas où la forme permise apparaît en argument
  (`python3 -X .venv/bin/python -m pip install`) doit être refusé.
- **Muter terme par terme, pas bloc par bloc.** Pour une conjonction `A && B &&
  C`, un cas doit rougir quand chaque terme seul est remplacé par `true`, ce qui
  demande un contrôle de bruit qui passe le pré-filtre sans satisfaire ce terme.
  Le contrôle négatif global (bloc retiré ⇒ rouge) reste utile mais ne suffit
  pas.
- **Un harnais de garde ne doit pas pouvoir écrire l'hôte si la garde casse.**
  Les chemins absolus pointent vers la fixture. `PIP_REQUIRE_VIRTUALENV=1`,
  `PIP_NO_INDEX=1` et `TMPDIR` ramené dans la fixture servent de filet (voir
  `un-harnais-sur-le-vrai-home-ecrit-les-chemins-de-production.md`).
- **Une garde de sécurité livrée depuis un plan « READY » passe quand même par
  une revue adversariale.** Ici le plan avait été groomé en deux passes et son
  prototype était vert. La revue a trouvé un P1 que ni l'un ni l'autre ne
  pouvait voir, parce que leur corpus venait du ticket.

## Voir aussi

- `cli-blocked-flag-equals-bypass.md` : même classe (liste de refus lexicale
  contournée par une variante syntaxique, `--flag=valeur`), côté Rust.
- `../prompt-engineering/2026-09-06-un-prompt-qui-reimplemente-une-garde-executable-derive.md` :
  pourquoi le refus vit dans `run.sh` et pas dans le prompt `qa-review`.
- `../1200-editable-install-dep-sync-gap-pilot-crash.md` : une autre casse du
  lanceur claude-pilot par une installation éditable.
- Limites nommées et non couvertes : découpage de token, assemblage par
  variable (dont `${IFS}`), continuation `\`-retour à la ligne, base64, canal
  `tmux` (commentaire du bloc et `crates/mika-agent/CLAUDE.md` § *The refusal is
  universal BY SITE*).

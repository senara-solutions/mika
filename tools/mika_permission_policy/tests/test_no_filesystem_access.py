"""Le classifier ne décide jamais sur le contenu d'un fichier — mika#2312.

mika#2312 a été ouvert sur l'inférence suivante : un `cat skill.toml` passait
alors qu'une lecture de `system_prompt.md` était refusée, donc « c'est le CONTENU
des system_prompt qui est protégé ». Il ne l'est pas, et il ne *peut pas* l'être :
le registre n'importe aucun module d'accès au système de fichiers, toutes ses
fonctions ont la signature ``(argv, cwd) -> bool``, et le docstring de
``_binaries.py`` exclut explicitement la containment de chemin de son périmètre.

L'invariant est donc vrai **par construction** — mais il n'était écrit nulle part,
et un carve-out de bonne foi (exactement celui que mika#2312 envisageait) l'aurait
enfreint sans qu'aucun test ne rougisse. D'où deux gardes complémentaires :

- **(a) garde structurelle, par AST** — refuse la *capacité* de lire un fichier :
  tout import d'un module d'accès au monde extérieur, et tout appel au builtin
  ``open``, dans les deux modules du package.
- **(b) pin comportemental** — affirme l'*indifférence au chemin* des fonctions de
  lecture : même verdict pour un ``system_prompt.md``, un ``skill.toml`` voisin, un
  chemin inexistant et un chemin hors worktree.

Pourquoi (a) en plus de (b) : un carve-out ajouté plus tard passerait tous les
tests comportementaux existants et n'échouerait que sur le chemin exact
carve-outé — que personne n'aurait pensé à tester. (a) refuse la capacité, pas
une instance. Même forme de garde que les scans de source côté Rust
(``mika2131_exclusion_skips_never_return_to_an_uncollected_debug``,
``mika2205_periodic_scans_do_not_read_the_pat_field_directly``), et pour la même
raison : la régression ne rendrait aucune décision fausse, elle lèverait
l'invariant en silence.

Doctrine : ``docs/solutions/security-issues/`` →
``le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md``.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

import mika_permission_policy
from mika_permission_policy import PolicyFn
from mika_permission_policy._binaries import (
    is_safe_cat,
    is_safe_grep,
    is_safe_head,
    is_safe_sed,
    is_safe_tail,
)

# ── (a) Garde structurelle ──────────────────────────────────────────────────

#: Modules dont l'import donnerait au registre un accès au monde extérieur.
#: La liste est **nommée et justifiée ici**, pas une interdiction totale
#: d'import : ``re`` et ``shlex`` restent disponibles (une future fonction de
#: sûreté peut légitimement en avoir besoin pour inspecter un argv), et
#: ``collections.abc`` — déjà importé par ``__init__.py`` pour le protocole
#: ``PolicyFn`` — reste permis.
FORBIDDEN_MODULES: frozenset[str] = frozenset({
    "os",          # os.path.exists, os.environ, os.system
    "pathlib",     # Path.read_text
    "io",          # io.open
    "glob",        # expansion de chemins
    "shutil",      # copie / suppression
    "subprocess",  # exécution
    "socket",      # réseau
    "urllib",      # réseau
    "requests",    # réseau
})

#: Builtin dont l'appel lirait un fichier sans passer par un import.
FORBIDDEN_BUILTIN = "open"

PACKAGE_DIR = Path(mika_permission_policy.__file__).parent
POLICY_MODULES = sorted(PACKAGE_DIR.glob("*.py"))


def _forbidden_imports(tree: ast.AST) -> list[str]:
    """Rendre les modules interdits importés par ``tree``, par nom racine."""
    found: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                root = alias.name.split(".")[0]
                if root in FORBIDDEN_MODULES:
                    found.append(alias.name)
        elif isinstance(node, ast.ImportFrom):
            # `from . import x` a module=None ; `from __future__ import ...`
            # et `from collections.abc import ...` ne sont pas concernés.
            if node.module:
                root = node.module.split(".")[0]
                if root in FORBIDDEN_MODULES:
                    found.append(node.module)
    return found


def _open_calls(tree: ast.AST) -> list[int]:
    """Rendre les numéros de ligne des appels au builtin ``open``."""
    lines: list[int] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        if isinstance(func, ast.Name) and func.id == FORBIDDEN_BUILTIN:
            lines.append(node.lineno)
        elif isinstance(func, ast.Attribute) and func.attr == FORBIDDEN_BUILTIN:
            # `io.open(...)`, `pathlib.Path(...).open(...)`, …
            lines.append(node.lineno)
    return lines


class TestNoFilesystemCapability:
    """Le registre ne peut pas ouvrir un fichier — refus de la capacité."""

    def test_package_modules_discovered(self):
        # Sans ce garde-fou, une erreur de résolution de chemin rendrait les
        # deux tests suivants verts sur un ensemble vide : une garde qui ne
        # regarde rien passe toujours.
        names = {p.name for p in POLICY_MODULES}
        assert names == {"__init__.py", "_binaries.py"}, (
            f"modules du package inattendus : {sorted(names)} — "
            "si un module a été ajouté, l'inclure ici avant de relancer"
        )

    @pytest.mark.parametrize("module_path", POLICY_MODULES, ids=lambda p: p.name)
    def test_no_forbidden_import(self, module_path: Path):
        tree = ast.parse(module_path.read_text(encoding="utf-8"))
        offenders = _forbidden_imports(tree)
        assert not offenders, (
            f"{module_path.name} importe {offenders} — le registre de permission "
            "n'a aucun moyen d'accéder au système de fichiers ou au réseau, et "
            "cet invariant est ce qui rend la décision indépendante du contenu "
            "et du chemin des fichiers lus (mika#2312). Si l'import est "
            "légitime, c'est une décision de sûreté avec son propre ticket, pas "
            "un ajustement en passant."
        )

    @pytest.mark.parametrize("module_path", POLICY_MODULES, ids=lambda p: p.name)
    def test_no_open_call(self, module_path: Path):
        tree = ast.parse(module_path.read_text(encoding="utf-8"))
        lines = _open_calls(tree)
        assert not lines, (
            f"{module_path.name} appelle `{FORBIDDEN_BUILTIN}` aux lignes "
            f"{lines} — une fonction de sûreté qui lit un fichier décide sur son "
            "contenu, ce qu'aucune ne fait aujourd'hui (mika#2312)."
        )


# ── (b) Pin comportemental ──────────────────────────────────────────────────

#: Un jeu de chemins qui couvre les quatre cas que mika#2312 oppose :
#: le fichier « protégé » supposé, son voisin qui passait, un chemin
#: inexistant, et un chemin hors worktree.
PATHS: tuple[str, ...] = (
    "skills/bundled/mika-arch-groom-ticket/system_prompt.md",
    "skills/bundled/mika-arch-second-review/system_prompt.md",
    "skills/bundled/dev-pilot/skill.toml",
    "docs/plans/does-not-exist.md",
    "/etc/passwd",
    "../../outside-the-worktree.md",
)

#: ``(fonction, préfixe d'argv)`` — chaque forme est évaluée sur tous les
#: chemins ci-dessus. Les formes à flags sont incluses pour que le pin porte
#: aussi sur les chemins de décision non triviaux (``sed -i`` dénie, et il doit
#: dénier pour TOUS les chemins).
#: ``PolicyFn`` vient du package (``mika_permission_policy.__all__``) plutôt
#: que d'être redéclaré ici : c'est le contrat que ce fichier épingle, et deux
#: écritures du même contrat dériveraient en silence.
CASES: tuple[tuple[PolicyFn, list[str]], ...] = (
    (is_safe_cat, ["cat"]),
    (is_safe_head, ["head", "-5"]),
    (is_safe_tail, ["tail", "-n", "20"]),
    (is_safe_grep, ["grep", "-n", "Plan:"]),
    (is_safe_sed, ["sed", "-n", "1,20p"]),
    (is_safe_sed, ["sed", "-i", "s/a/b/"]),
)


class TestVerdictIsIndifferentToPath:
    """Le verdict ne dépend pas du chemin — ni du fichier, ni de son existence.

    Le test affirme l'**indifférence**, pas un verdict particulier : c'est la
    propriété qui est en jeu, et elle survit à un durcissement futur de ``sed``
    ou de ``grep``.
    """

    @pytest.mark.parametrize(
        "fn,argv_prefix",
        CASES,
        ids=[f"{fn.__name__}:{' '.join(prefix[1:]) or 'bare'}" for fn, prefix in CASES],
    )
    def test_same_verdict_for_every_path(self, fn, argv_prefix: list[str]):
        verdicts = {path: fn([*argv_prefix, path], "/tmp") for path in PATHS}
        distinct = set(verdicts.values())
        assert len(distinct) == 1, (
            f"{fn.__name__} rend des verdicts différents selon le chemin : "
            f"{verdicts} — le classifier ne décide jamais sur le chemin ni sur "
            "le contenu d'un fichier (mika#2312). Un carve-out de chemin "
            "introduirait la première dimension « chemin » d'un classifier qui "
            "n'en a aucune."
        )

    def test_cwd_does_not_change_the_verdict(self):
        # Le `cwd` est la seule information de localisation que la signature
        # transporte. Il ne doit pas non plus décider.
        path = "skills/bundled/mika-arch-groom-ticket/system_prompt.md"
        for fn, argv_prefix in CASES:
            argv = [*argv_prefix, path]
            inside = fn(argv, "/data/workspace/mika-platform/mika")
            outside = fn(argv, "/tmp")
            assert inside is outside, (
                f"{fn.__name__} rend un verdict dépendant du cwd "
                f"({inside} vs {outside}) — mika#2312"
            )

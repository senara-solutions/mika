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

- **(a) garde structurelle, par AST** — refuse la *capacité* de lire un fichier
  ou d'atteindre le réseau : tout import hors d'une allow-list nommée, tout appel
  à ``open``, et les deux portes dynamiques (``__import__``, ``importlib``) par
  lesquelles une liste de noms se contourne.
- **(b) pin comportemental** — affirme l'*indifférence au chemin* sur **toute**
  entrée du registre (`get_policy()`), pas sur un échantillon : même verdict pour
  un ``system_prompt.md``, un ``skill.toml`` voisin, un chemin inexistant et un
  chemin hors worktree.

**Ce que ces gardes ne couvrent PAS, dit ici pour que personne ne s'y fie plus
qu'il ne doit** (mika#2312, revue de code) : (a) refuse une *capacité*, donc elle
ne voit pas un carve-out qui n'en demande aucune — ``if argv[-1].endswith(
"system_prompt.md"): return False`` n'importe rien et n'ouvre rien. C'est (b),
et (b) seule, qui attrape cette forme-là ; c'est pourquoi (b) balaie désormais le
registre entier plutôt qu'un échantillon. Contre un auteur *déterminé*, ni l'une
ni l'autre n'est une frontière de sûreté : elles bornent la dérive de bonne foi.

Même forme de garde que les scans de source côté Rust
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
from mika_permission_policy import get_policy

# ── (a) Garde structurelle ──────────────────────────────────────────────────

#: Les seuls modules que le registre peut importer. **Allow-list, pas
#: deny-list** : une liste de modules *interdits* ne protège qu'une écriture du
#: défaut, et `fileinput`, `linecache`, `mmap`, `ctypes`, `tempfile` — comme
#: tout module futur — n'y figureraient pas (doctrine :
#: ``docs/solutions/best-practices/2103-a-guard-that-knows-one-spelling-of-a-
#: defect-protects-nothing-else.md``, et la polarité déjà retenue par
#: ``make verify-no-secret-in-setenv``). Ajouter une entrée ici est une décision
#: d'une ligne, relisible en revue ; c'est exactement la propriété recherchée.
#:
#: `re` et `shlex` sont permis d'avance : une future fonction de sûreté peut
#: légitimement inspecter un argv avec l'un ou l'autre, et aucun des deux
#: n'atteint le disque ni le réseau.
ALLOWED_IMPORT_ROOTS: frozenset[str] = frozenset({
    "__future__",              # from __future__ import annotations
    "collections",             # collections.abc.Callable — le protocole PolicyFn
    "mika_permission_policy",  # imports internes au package
    "re",                      # inspection d'argv
    "shlex",                   # inspection d'argv
    "typing",                  # annotations
})

#: Builtins dont l'appel donnerait un accès au monde extérieur sans passer par
#: un import — donc invisibles pour la garde d'imports, quelle que soit sa
#: polarité. ``__import__`` est la porte générique : ``__import__("os")``
#: n'est ni un ``ast.Import`` ni un appel à ``open``.
FORBIDDEN_BUILTINS: frozenset[str] = frozenset({"open", "__import__", "eval", "exec"})

PACKAGE_DIR = Path(mika_permission_policy.__file__).parent

#: **Récursif** : ``glob("*.py")`` ne descend pas, donc un sous-paquet
#: (``mika_permission_policy/rules/reader.py``) échappait à la fois aux deux
#: tests AST *et* au canari ci-dessous, qui comparait des noms de base tirés du
#: même glob non récursif (mika#2312, revue de code). ``__pycache__`` ne contient
#: que des ``.pyc`` mais est exclu explicitement plutôt qu'implicitement.
POLICY_MODULES = sorted(
    p for p in PACKAGE_DIR.rglob("*.py") if "__pycache__" not in p.parts
)


def _module_id(path: Path) -> str:
    """Identifiant stable et non ambigu pour un module du package.

    Relatif au package, jamais ``p.name`` : deux ``__init__.py`` dans deux
    sous-répertoires se confondraient en un seul id pytest.
    """
    return str(path.relative_to(PACKAGE_DIR))


def _forbidden_imports(tree: ast.AST) -> list[str]:
    """Rendre les imports hors allow-list, par nom tel qu'écrit."""
    found: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                # `alias.name`, jamais `alias.asname` : `import os as o` doit
                # être vu sous son vrai nom.
                if alias.name.split(".")[0] not in ALLOWED_IMPORT_ROOTS:
                    found.append(alias.name)
        elif isinstance(node, ast.ImportFrom):
            # `from . import x` a module=None : un import relatif reste dans le
            # package, donc dans un fichier que ce même scan parcourt.
            if node.module and node.module.split(".")[0] not in ALLOWED_IMPORT_ROOTS:
                found.append(node.module)
    return found


def _forbidden_builtin_calls(tree: ast.AST) -> list[tuple[str, int]]:
    """Rendre les appels aux builtins interdits, avec leur ligne."""
    hits: list[tuple[str, int]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        if isinstance(func, ast.Name) and func.id in FORBIDDEN_BUILTINS:
            hits.append((func.id, node.lineno))
        elif isinstance(func, ast.Attribute) and func.attr in FORBIDDEN_BUILTINS:
            # `io.open(...)`, `pathlib.Path(...).open(...)`, …
            hits.append((func.attr, node.lineno))
    return hits


def _parse(path: Path) -> ast.AST:
    return ast.parse(path.read_text(encoding="utf-8"))


class TestNoFilesystemCapability:
    """Le registre ne peut pas ouvrir un fichier — refus de la capacité."""

    def test_package_modules_discovered(self):
        # Sans ce garde-fou, une erreur de résolution de chemin rendrait les
        # deux tests suivants verts sur un ensemble vide : une garde qui ne
        # regarde rien passe toujours. Les identifiants sont relatifs au
        # package, donc un module ajouté dans un sous-répertoire fait rougir
        # ce test — ce qu'un ensemble de noms de base ne faisait pas.
        found = {_module_id(p) for p in POLICY_MODULES}
        assert found == {"__init__.py", "_binaries.py"}, (
            f"modules du package inattendus : {sorted(found)} — "
            "si un module a été ajouté, l'inclure ici avant de relancer "
            "(mika#2312 : un module que le scan ne voit pas n'est gardé par rien)"
        )

    def test_package_dir_is_the_reviewed_source(self):
        # PACKAGE_DIR vient de `mika_permission_policy.__file__`, donc de
        # l'installation, pas du dépôt. Sous une installation non-editable, la
        # garde attesterait une copie figée pendant que la source éditée passe
        # inaperçue — verte, et sur le mauvais arbre.
        assert (PACKAGE_DIR.parent / "pyproject.toml").is_file(), (
            f"PACKAGE_DIR={PACKAGE_DIR} ne ressemble pas à une installation "
            "editable du dépôt : la garde AST lirait une copie installée, pas "
            "la source sous revue (mika#2312)"
        )

    @pytest.mark.parametrize("module_path", POLICY_MODULES, ids=_module_id)
    def test_no_forbidden_import(self, module_path: Path):
        offenders = _forbidden_imports(_parse(module_path))
        assert not offenders, (
            f"{_module_id(module_path)} importe {offenders}, hors de "
            f"l'allow-list {sorted(ALLOWED_IMPORT_ROOTS)} — le registre de "
            "permission n'accède ni au système de fichiers ni au réseau, et cet "
            "invariant est ce qui rend la décision indépendante du contenu des "
            "fichiers lus (mika#2312). Si l'import est légitime, l'ajouter à "
            "ALLOWED_IMPORT_ROOTS est une décision de sûreté à prendre en revue, "
            "pas un ajustement en passant."
        )

    @pytest.mark.parametrize("module_path", POLICY_MODULES, ids=_module_id)
    def test_no_forbidden_builtin_call(self, module_path: Path):
        hits = _forbidden_builtin_calls(_parse(module_path))
        assert not hits, (
            f"{_module_id(module_path)} appelle {hits} — une fonction de sûreté "
            "qui lit un fichier décide sur son contenu, et `__import__` / `eval` "
            "rouvrent par la porte dynamique ce que l'allow-list d'imports "
            "ferme (mika#2312)."
        )


class TestTheGuardBites:
    """Anti-vacuité : la garde rougit-elle sur une violation ?

    « A guard nobody has watched go red is a decoration » — la discipline que
    `.github/workflows/ci.yml` applique déjà à ses trois lints structurels
    (byte-slice, image-tag, dispatch-seat), chacun doublé d'un pas
    « Pin the guard's negative behaviour ». Ici l'équivalent tient en mémoire :
    on parse des sources fautives et on vérifie que les détecteurs les voient.
    Sans ça, un refactor qui ferait rendre `[]` inconditionnellement aux deux
    fonctions laisserait toute la suite verte.
    """

    @pytest.mark.parametrize("source,expected", [
        ("import os", "os"),
        ("import os as o", "os"),
        ("import os.path", "os.path"),
        ("from pathlib import Path", "pathlib"),
        ("import importlib", "importlib"),
        ("import fileinput", "fileinput"),
        ("import linecache", "linecache"),
        ("import socket", "socket"),
        ("import subprocess", "subprocess"),
    ])
    def test_forbidden_import_is_detected(self, source: str, expected: str):
        assert expected in _forbidden_imports(ast.parse(source))

    @pytest.mark.parametrize("source", [
        "from __future__ import annotations",
        "from collections.abc import Callable",
        "from mika_permission_policy._binaries import is_safe_cat",
        "import re",
        "from . import _binaries",
    ])
    def test_allowed_import_is_not_flagged(self, source: str):
        assert _forbidden_imports(ast.parse(source)) == []

    @pytest.mark.parametrize("source,expected", [
        ("open(p).read()", "open"),
        ("io.open(p)", "open"),
        ("__import__('os')", "__import__"),
        ("eval('1')", "eval"),
    ])
    def test_forbidden_builtin_call_is_detected(self, source: str, expected: str):
        names = [name for name, _ in _forbidden_builtin_calls(ast.parse(source))]
        assert expected in names

    def test_a_path_carve_out_is_invisible_to_the_ast_guard(self):
        # Le seul cas que (a) NE PEUT PAS voir, épinglé pour que la limite soit
        # une propriété testée et non une phrase de docstring : un carve-out de
        # chemin ne demande aucune capacité. C'est (b) qui couvre cette forme.
        carve_out = 'def is_safe_cat(argv, cwd):\n    return not argv[-1].endswith("x.md")\n'
        tree = ast.parse(carve_out)
        assert _forbidden_imports(tree) == []
        assert _forbidden_builtin_calls(tree) == []


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

#: Drapeaux représentatifs pour les binaires dont la forme d'appel usuelle en
#: porte. Le pin balaie **tout** `get_policy()` — un échantillon laissait 29
#: fonctions sur 34 sans pin, dont `is_safe_git`, `is_safe_gh` et
#: `is_safe_find`, c'est-à-dire précisément celles dont l'argv porte
#: habituellement des chemins (mika#2312, revue de code).
FLAGS: dict[str, list[str]] = {
    "head": ["-5"],
    "tail": ["-n", "20"],
    "grep": ["-n", "Plan:"],
    "egrep": ["-n", "Plan:"],
    "fgrep": ["-n", "Plan:"],
    "sed": ["-n", "1,20p"],
    "git": ["log", "--oneline"],
    "gh": ["pr", "view"],
    "find": ["-name"],
}

def _argv_for(binary: str, path: str) -> list[str]:
    return [binary, *FLAGS.get(binary, []), path]


class TestVerdictIsIndifferentToPath:
    """Le verdict ne dépend ni du chemin, ni de son existence, ni du cwd.

    Le test affirme l'**indifférence**, pas un verdict particulier : c'est la
    propriété qui est en jeu, et elle survit à un durcissement futur de ``sed``
    ou de ``grep``. Sa contrepartie — que les verdicts positifs restent positifs
    — est tenue par ``test_binaries.py``, et les deux moitiés se complètent :
    seule, celle-ci serait satisfaite par un registre qui refuse tout.
    """

    @pytest.mark.parametrize("binary", sorted(get_policy()))
    def test_same_verdict_for_every_path(self, binary: str):
        fn = get_policy()[binary]
        verdicts = {path: fn(_argv_for(binary, path), "/tmp") for path in PATHS}
        assert len(set(verdicts.values())) == 1, (
            f"{binary} rend des verdicts différents selon le chemin : "
            f"{verdicts} — le classifier ne décide jamais sur le chemin ni sur "
            "le contenu d'un fichier (mika#2312). Un carve-out de chemin "
            "introduirait la première dimension « chemin » d'un classifier qui "
            "n'en a aucune."
        )

    @pytest.mark.parametrize("binary", sorted(get_policy()))
    def test_cwd_does_not_change_the_verdict(self, binary: str):
        # Le `cwd` est la seule information de localisation que la signature
        # transporte. Il ne doit pas décider non plus.
        fn = get_policy()[binary]
        argv = _argv_for(binary, PATHS[0])
        inside = fn(argv, "/data/workspace/mika-platform/mika")
        outside = fn(argv, "/tmp")
        # `==`, pas `is` : l'égalité exprime la propriété voulue sans supposer
        # que tout PolicyFn rend un singleton `True`/`False`.
        assert inside == outside, (
            f"{binary} rend un verdict dépendant du cwd "
            f"({inside} vs {outside}) — mika#2312"
        )

    def test_a_deny_by_flag_stays_indifferent_to_the_path(self):
        # `sed -i` dénie : le pin doit constater que le refus vaut pour TOUS les
        # chemins, sinon il ne dirait rien des chemins de décision non triviaux.
        fn = get_policy()["sed"]
        verdicts = {p: fn(["sed", "-i", "s/a/b/", p], "/tmp") for p in PATHS}
        assert set(verdicts.values()) == {False}, verdicts

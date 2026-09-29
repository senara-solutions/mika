#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Lockstep manifeste↔code sur les sinks réseau sortants (mika#2408).

Ce module est le moteur de `scripts/verify-egress-manifest.sh`. Il n'est pas
exécutable seul en CI : le script shell est l'entrée, ce fichier porte
l'inventaire et les quatre directions.

Doctrine : « construire l'incapacité, ne pas promettre la retenue »
(`scripts/verify-egress-uniqueness.sh`, `scripts/check-byte-slices.sh`).
Un sink sortant ne peut plus apparaître sans qu'une ligne de manifeste dise
quelle donnée part, vers où, et si elle est journalisée — et une telle ligne
ne peut pas être écrite pour un sink qui n'existe pas.

QUATRE DIRECTIONS
-----------------
  D1  code → manifeste : tout fichier construisant un client HTTP en
      PRODUCTION est couvert par au moins un `client_site`.
  D2  code → manifeste : tout littéral de host EXTERNE en production est
      couvert par une `destination`, ou appartient à une classe de
      `NON_SINK_HOSTS`.
  D3  manifeste → code : tout `client_site` couvre au moins un fichier de
      l'inventaire D1 (déclaration fantôme).
  D4  manifeste → code : toute `destination` dont
      `destination_source = "literal"` apparaît au moins une fois comme
      littéral (host fantôme).

D4 ne s'applique pas aux `config` / `skill-declared` : leur destination n'est
par construction dans aucun littéral, et exiger le contraire ferait rougir le
lint sur les déclarations les plus honnêtes du fichier.

DÉCOUPE PRODUCTION / TEST — la décision qui tient tout
-----------------------------------------------------
Ce parseur `#[cfg(test)]` est DÉLIBÉRÉMENT plus grossier que celui de
`scripts/verify-egress-no-log.sh`, et il ne le réutilise ni ne le copie.
Deux mesures l'imposent (plan mika#2408 § M2) :

  (i)  le parseur voisin refuse en fail-closed deux formes présentes dans la
       population visée ici — un `#[cfg(test)]` sur une `fn` nue
       (`crates/mika-common/src/llm/ollama.rs`) et sur une `const` nue
       (`crates/mika-gateway/src/routes.rs`) ;
  (ii) l'extraire vers un fichier partagé FAIT ROUGIR son propre test : la
       V13 de `scripts/test-verify-egress-no-log.sh` cherche la définition
       DANS CE FICHIER-LÀ, par ancrage de début de ligne. Et le dupliquer
       crée une divergence que V13 ne peut pas voir depuis son côté — le pire
       des trois cas.

Son unité d'analyse est différente, et c'est ce qui autorise la simplicité :
il répond « CE FICHIER a-t-il un sink en production ? », jamais « quelles
LIGNES sont production ? ». Sa règle de doute est donc INVERSE de celle du
voisin : **tout doute conclut « production », donc « déclare »**. Une erreur
de découpe ne peut produire qu'une déclaration de plus — jamais un silence.

  - le premier `#[cfg(test)]` SUIVI d'un `mod X {` (bloc, pas `;`) coupe le
    fichier ; tout ce qui précède est production ;
  - toute autre forme de `#[cfg(test)]` — `fn`, `const`, `impl` — NE COUPE
    PAS, et le fichier est traité en entier comme production.

Le bénéfice qui décide : ce lint ne partage AUCUNE ligne avec la garde
STRIP-TOTAL de `verify-egress-no-log.sh`. Il ne peut donc pas la casser.

Codes de sortie (portés par le script shell) :
  0 — aucune violation
  1 — violation(s) trouvée(s)
  2 — manifeste illisible / absent / vide (fail-closed, jamais un vert)
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# `tomllib` est stdlib depuis Python 3.11. SOUS GARDE, et le refus sort en 2 —
# pas en 1. Un `import` nu rend un traceback, donc l'exit 1 de l'interpréteur,
# donc « violation(s) trouvée(s) » dans le vocabulaire de ce lint : un
# interpréteur trop ancien se lisait comme un SINK NON DÉCLARÉ, et le motif
# était faux. Mesuré le 2026-09-29 sur `ubuntu-22.04` (Python 3.10), où les
# deux jobs egress ont rougi sur `ModuleNotFoundError` sans qu'une ligne dise
# quoi faire — la CI est désormais épinglée par `actions/setup-python`
# (`.github/workflows/ci.yml`), et ce refus est ce qui reste lisible sur un
# poste dont le `python3` est antérieur.
#
# SITE UNIQUE : les deux consommateurs (`verify-egress-manifest.sh`,
# `verify-egress-uniqueness.sh`) en héritent sans dupliquer la vérification.
#
# PAS de repli sur `tomli` : le paquet n'est pas une dépendance de ce dépôt, il
# est absent du runner, et un repli opportuniste donnerait au même interpréteur
# deux régimes selon ce qu'un environnement porte par accident. Un refus qui
# nomme l'exigence vaut mieux qu'un lint dont on ne sait pas s'il a tourné.
try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover — vérifié par N17 via un shadow
    sys.stderr.write(
        "ERROR (egress-manifest): `tomllib` introuvable — Python "
        f"{sys.version_info.major}.{sys.version_info.minor} "
        f"({sys.executable})\n"
        "  Ce lint lit `docs/egress/egress-manifest.toml` et exige Python >= 3.11\n"
        "  (tomllib est stdlib depuis 3.11).\n"
        "  CI : le job doit déclarer `actions/setup-python` (python-version 3.12).\n"
        "  Local : lancer sous un python3 >= 3.11.\n"
        "  Refus plutôt que continuation : sans le manifeste, ce lint ne\n"
        "  confronte rien et un exit 0 se lirait comme un arbre propre\n"
        "  (classe mika#2205).\n"
    )
    sys.exit(2)

# --------------------------------------------------------------------------
# Périmètre du scan (AC4)
# --------------------------------------------------------------------------

# Racines scannées : uniquement le code Rust de production des crates.
SCAN_ROOT = "crates"

# Répertoires exclus de l'inventaire. `tests/`, `examples/` et `benches/` sont
# du code de test par construction (Cargo les compile en `cfg(test)`-adjacent
# ou en cibles séparées) ; `target/` est un artefact de build.
#
# Mesure de l'effet (plan § M2) : le filtre de commentaire seul ne retire que
# 3 des 304 occurrences de `github.com` — l'exclusion qui porte est celle de
# `tests/` et de `#[cfg(test)]`.
EXCLUDED_PATH_SEGMENTS = (
    "/tests/",
    "/examples/",
    "/benches/",
    "/target/",
)

# --------------------------------------------------------------------------
# D1 — inventaire des constructions de client HTTP
# --------------------------------------------------------------------------

# Le prédicat PORTEUR (plan § R3). Trois formulations ont été mesurées ; les
# deux autres sont inutilisables :
#
#   - « receveur `client`/`http` + verbe sur la même ligne » → 6 fichiers,
#     rate le chaînage multi-ligne de `github_graphql.rs`, `claude.rs`,
#     `openai.rs`, `oauth.rs` ;
#   - « verbe HTTP en tête de ligne » → 72 fichiers, dont une majorité de
#     `.get(` sur des `HashMap` (`kg/query.rs`, `skills/matcher.rs`).
#
# Celui-ci est le seul complet sur sa classe et à bruit ~nul. Sa limite est
# nommée et fermée ailleurs : il ne dit pas *où* ça va, et il rate un sink
# dont le client est injecté (`telegram.rs`, plan § R2). La moitié « vers
# où ? » est D2 ; la moitié « client injecté » est le champ `call_site`,
# documentaire, plus le `client_site` du constructeur partagé.
CLIENT_CONSTRUCTION_RE = re.compile(
    r"\bClient::new\s*\(\s*\)"
    r"|\bClient::builder\s*\(\s*\)"
    r"|\bClientBuilder::new\s*\("
)

# --------------------------------------------------------------------------
# D2 — inventaire des littéraux de host
# --------------------------------------------------------------------------

# Un host dans un littéral d'URL. Quatre formes réelles mesurées sur l'arbre
# ont forcé chaque morceau de ce motif ; les écrire ici évite qu'un futur
# lecteur les « simplifie » :
#
#   userinfo   `"https://x-access-token:{token}@github.com/"`
#              (`skills/git.rs`) — sans le saut d'userinfo, le host capté est
#              `x-access-token`, qui n'est pas un host et rougirait en D2.
#   gabarit    `format!("http://mika-{id}.{ns}.svc.cluster.local:8080")`
#              (`gateway/src/routes.rs`) — sans `{}` dans la classe, le host
#              capté est `mika-`, et la classe `.svc.cluster.local` qui le
#              rendrait non-sink n'est jamais atteinte.
#   regex      `Regex::new(r"https://github\.com/…")` (`skills/context.rs`) —
#              sans `\` dans la classe, le host capté est `github`.
#   port       `"http://localhost:8080"` — le port est hors de la classe, donc
#              retiré sans post-traitement.
URL_LITERAL_RE = re.compile(
    r'"(?:https?|wss?)://'  # guillemet ouvrant + schéma
    r"(?:[^\"/@\s]*@)?"  # userinfo éventuel, sauté
    r"([A-Za-z0-9._\-{}\\]+)"  # host, gabarits et points échappés compris
)

# Évidence d'existence pour D4 SEULEMENT — un littéral de chaîne qui EST un
# nom de domaine, sans schéma. Les quatre hosts gouv.fr de `egress_fetch`
# vivent sous cette forme (un tableau `ALLOWED_HOSTS`), jamais dans une URL,
# donc D4 les déclarerait fantômes sans ce second motif.
#
# Cette population n'alimente PAS D2 : D2 rapporte, donc il lui faut un bruit
# quasi nul et un littéral de host nu n'est pas en soi un chemin de
# reachability. D4 vérifie seulement qu'une destination déclarée existe
# quelque part — une évidence plus large l'y rend plus permissif, ce qui est
# le sens sûr pour une garde anti-fantôme (l'ancrage d'AC1b reste D3).
BARE_HOST_LITERAL_RE = re.compile(r'"((?:[A-Za-z0-9\-]+\.)+[A-Za-z]{2,})"')

# Un host dont la forme, placeholders retirés, ne porte plus de point n'est
# pas un littéral de destination : c'est une URL entièrement dynamique
# (`format!("{base}/v1/chat")`). La rapporter en D2 serait un faux positif de
# la classe qu'AC4 interdit ; elle est couverte par
# `destination_source = "config"` sur l'entrée du client concerné.
TEMPLATE_PLACEHOLDER_RE = re.compile(r"\{[^{}]*\}")


def normalize_host(raw: str) -> str | None:
    """Normalise un host capté, ou rend `None` s'il ne porte aucun littéral.

    Retire les échappements de regex, met en minuscules, et refuse une forme
    entièrement dynamique (voir `TEMPLATE_PLACEHOLDER_RE`).
    """
    host = raw.replace("\\", "").lower().strip(".")
    if not host:
        return None
    if "{" not in host:
        # Pas de gabarit : le littéral est entier, on le rend tel quel. Un
        # host sans point (`localhost`, un nom court de service interne) est
        # un host valide et doit atteindre `is_non_sink_host` ou D2 — le
        # jeter ici en ferait un sink muet.
        return host
    residue = TEMPLATE_PLACEHOLDER_RE.sub("", host).strip(".-")
    if "." not in residue:
        return None
    return host

# Classes de hosts NON ROUTABLES ou RÉSERVÉES aux fixtures. Ce n'est PAS une
# allowlist d'exception (cf. `scripts/egress-manifest-exceptions.tsv`, livré
# vide) : aucune de ces classes ne décrit une violation, chacune porte sa
# raison, et la liste est par CLASSE — jamais par host individuel.
#
# Un host qui n'entre dans aucune classe et dans aucune `destination` DOIT
# être déclaré. C'est le contrat, pas une friction à contourner.
NON_SINK_HOST_EXACT = frozenset(
    {
        # Loopback et wildcard — un appel qui n'atteint jamais le réseau.
        "localhost",
        "0.0.0.0",
        "::1",
        "[::1]",
        # RFC 2606 §3 : réservés à la documentation, ne résolvent nulle part.
        "example.com",
        "example.org",
        "example.net",
    }
)

NON_SINK_HOST_SUFFIXES = (
    # RFC 2606 §2 : TLD réservés, jamais délégués.
    ".invalid",
    ".test",
    ".example",
    # RFC 6762 : mDNS, périmètre du lien local.
    ".local",
    # Kubernetes : DNS interne du cluster, hors du réseau public.
    ".svc.cluster.local",
    # RFC 2606 §3, sous-domaines des domaines réservés ci-dessus.
    ".example.com",
    ".example.org",
    ".example.net",
)

# 127.0.0.0/8 en entier — le loopback ne se limite pas à 127.0.0.1.
LOOPBACK_V4_RE = re.compile(r"^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$")


def is_non_sink_host(host: str) -> bool:
    """Vrai si `host` appartient à une classe non routable ou de fixture."""
    bare = host.split(":", 1)[0].lower()
    if bare in NON_SINK_HOST_EXACT:
        return True
    if LOOPBACK_V4_RE.match(bare):
        return True
    return any(bare.endswith(suffix) for suffix in NON_SINK_HOST_SUFFIXES)


# --------------------------------------------------------------------------
# Découpe production / test
# --------------------------------------------------------------------------

CFG_TEST_RE = re.compile(r"^\s*#\[cfg\(test\)\]")
INLINE_TEST_MOD_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z0-9_]+\s*\{")
EXTERNAL_TEST_MOD_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;")
COMMENT_LINE_RE = re.compile(r"^\s*//")


# Un item Rust en COLONNE ZÉRO — la seule évidence de « la production reprend »
# qui ne demande aucun comptage d'accolades. Voir `production_slices` § limite.
COLUMN_ZERO_ITEM_RE = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|impl|struct|enum|trait|static|const|type)\b"
)

# Ouverture / fermeture d'une chaîne brute Rust : `r"`, `r#"`, `r##"`, …
# Le nombre de `#` doit correspondre à la fermeture, d'où la capture.
RAW_STRING_OPEN_RE = re.compile(r'r(#*)"')


def production_slices(lines: list[str]) -> tuple[list[tuple[int, str]], list[str]]:
    """Rend `(lignes de production, modules de test déclarés en externe)`.

    Voir la note de tête : ce parseur est délibérément grossier et conclut
    « production » au moindre doute. Le premier `#[cfg(test)]` suivi d'un
    `mod X {` COUPE le fichier ; toute autre forme de `#[cfg(test)]` — `fn`,
    `const`, `impl` — ne coupe rien.

    LIMITE, et elle a été mesurée plutôt que supposée. Couper au lieu de
    *sauter le bloc puis reprendre* laisse un trou : un fichier portant du code
    de production APRÈS un `#[cfg(test)] mod tests { … }` verrait son sink
    disparaître en silence. Le saut a été écrit, essayé, et refusé sur mesure :
    il demande de compter des accolades, et les formes que ce comptage ne
    modélise pas — chaîne sur plusieurs lignes, chaîne brute, commentaire de
    bloc — sont RÉELLEMENT PRÉSENTES dans l'arbre. Il fermait le module de test
    de `crates/mika-gateway/src/telegram.rs` 265 lignes trop tôt et produisait
    trois faux positifs sur `main`, ce qu'AC4 interdit.

    Le trou est donc assumé, et il n'est pas silencieux : le relevé au moment de
    l'écrire donne 297 fichiers coupés, 8 portant encore un host après la
    fermeture du premier bloc, et les 8 dans un SECOND bloc de test — la
    population dangereuse est VIDE. Le cas N15 de
    `scripts/test-verify-egress-manifest.sh` la remesure à chaque run, par un
    prédicat qui ne compte aucune accolade (un item en colonne zéro après le
    premier `#[cfg(test)]`), et rougit le jour où elle cesse de l'être. Un trou
    mesuré en continu n'est pas un trou silencieux.

    Le second membre du couple est la liste des `#[cfg(test)] mod NAME;` — des
    déclarations qui ne coupent RIEN ici mais nomment un fichier frère
    entièrement dédié au test. Sans ce relevé,
    `egress_search/tests_e4_no_log.rs` est lu en entier comme de la production
    et son host de fixture (`ex.com`) rougit en D2 — un faux positif de la même
    classe.

    Les lignes de commentaire (`//`, `///`, `//!`) sont retirées : un host cité
    en prose de commentaire n'est pas un appel.
    """
    out: list[tuple[int, str]] = []
    external_test_mods: list[str] = []
    pending_cfg_test = False

    for idx, raw in enumerate(lines, start=1):
        if CFG_TEST_RE.match(raw):
            pending_cfg_test = True
            continue

        if pending_cfg_test:
            stripped = raw.strip()
            # Lignes qui ne portent aucun item : traversées sans consommer
            # l'état (une ligne vide, un commentaire, un autre attribut).
            if not stripped or COMMENT_LINE_RE.match(raw) or stripped.startswith("#["):
                continue
            pending_cfg_test = False
            if INLINE_TEST_MOD_RE.match(raw):
                # LA coupure. Tout ce qui suit est traité comme du test.
                break
            external = EXTERNAL_TEST_MOD_RE.match(raw)
            if external:
                external_test_mods.append(external.group(1))
                continue
            # Toute autre forme — `fn`, `const`, `impl`, `use` — ne coupe pas.
            # Le fichier continue d'être lu en entier comme production : une
            # déclaration de trop est visible, un sink omis est silencieux.

        if COMMENT_LINE_RE.match(raw):
            continue
        out.append((idx, raw))

    return out, external_test_mods


def production_resumes_after_cut(lines: list[str]) -> int | None:
    """Rend la ligne où la production REPREND après la coupure, ou `None`.

    C'est la remesure continue de la limite de `production_slices`, et son
    prédicat ne compte AUCUNE accolade — c'est tout son intérêt, puisque c'est
    le comptage qui a rendu le saut de bloc inutilisable.

    Le signal est un item Rust en COLONNE ZÉRO après le premier
    `#[cfg(test)] mod X {`. Dans un fichier conforme, le corps d'un module de
    test est indenté, et un second `#[cfg(test)] mod Y {` n'est pas un item au
    sens de `COLUMN_ZERO_ITEM_RE` — d'où un prédicat qui ne se déclenche que
    sur la forme dangereuse : de la production redevenue frère du module.

    Les régions de CHAÎNE BRUTE sont sautées, et ce n'est pas une précaution
    théorique : six fichiers de ce dépôt embarquent des fixtures Rust dans des
    `r#"…"#` — les scans de source (`source_scan.rs`, `source_guard.rs`,
    `ready_label_handler.rs`, …) — et leurs `fn` de fixture sont en colonne
    zéro. Sans ce saut, le prédicat rapporte six reprises dont aucune n'existe.

    Consommé par le cas N15 du test. Rend `None` quand rien ne reprend.
    """
    cut = None
    pending = False
    for idx, raw in enumerate(lines, start=1):
        if CFG_TEST_RE.match(raw):
            pending = True
            continue
        if pending:
            stripped = raw.strip()
            if not stripped or COMMENT_LINE_RE.match(raw) or stripped.startswith("#["):
                continue
            pending = False
            if INLINE_TEST_MOD_RE.match(raw):
                cut = idx
                break
    if cut is None:
        return None

    raw_hashes: str | None = None  # `None` = hors chaîne brute
    for idx in range(cut, len(lines)):
        raw = lines[idx]
        if raw_hashes is not None:
            if f'"{raw_hashes}' in raw:
                raw_hashes = None
            continue
        opened = RAW_STRING_OPEN_RE.search(raw)
        if opened:
            closing = '"' + opened.group(1)
            # Une chaîne brute ouverte ET fermée sur la même ligne ne masque
            # rien ; seule celle qui reste ouverte fait entrer dans le saut.
            if closing not in raw[opened.end() :]:
                raw_hashes = opened.group(1)
                continue
        if COLUMN_ZERO_ITEM_RE.match(raw):
            return idx + 1
    return None


def audit_cut_holes(root: Path) -> list[str]:
    """Les sinks que la coupure de `production_slices` laisse hors inventaire.

    L'assertion porte sur la CONSÉQUENCE — un sink invisible — et non sur la
    forme. C'est ce qui la rend utilisable : la détection de reprise
    sur-rapporte (une fixture Rust dans une chaîne à continuation de ligne est
    lue comme une reprise), et cette sur-détection ne coûte rien tant que la
    région ne porte aucun sink. Une sur-détection QUI EN PORTE un est
    exactement ce qu'on veut voir.

    Mesure au moment de l'écrire : quatre fichiers voient une reprise, dont un
    vrai (`crates/mika-agent/src/server/dashboard.rs` porte du code de
    production après son module de test, lignes 1596-1732) — et AUCUN ne porte
    de sink derrière. La liste rendue est donc vide, et le cas N15 du test
    exige qu'elle le reste.

    Rend une liste de `chemin:ligne <MOTIF>`, vide en régime nominal.
    """
    out: list[str] = []
    scan_root = root / SCAN_ROOT
    if not scan_root.is_dir():
        return out

    for path in sorted(scan_root.rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        if is_excluded(rel):
            continue
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        at = production_resumes_after_cut(lines)
        if at is None:
            continue
        for off in range(at - 1, len(lines)):
            content = lines[off]
            if COMMENT_LINE_RE.match(content):
                continue
            if CLIENT_CONSTRUCTION_RE.search(content):
                out.append(f"{rel}:{off + 1} client HTTP hors inventaire")
            for raw_host in URL_LITERAL_RE.findall(content):
                host = normalize_host(raw_host)
                if host and not is_non_sink_host(host):
                    out.append(f"{rel}:{off + 1} host '{host}' hors inventaire")
    return out


# --------------------------------------------------------------------------
# Inventaire
# --------------------------------------------------------------------------


@dataclass
class Inventory:
    """Ce que l'arbre contient réellement."""

    # chemin relatif → lignes où un client HTTP est construit
    client_files: dict[str, list[int]] = field(default_factory=dict)
    # host d'URL → liste de `chemin:ligne` — la population que D2 RAPPORTE
    hosts: dict[str, list[str]] = field(default_factory=dict)
    # tout host cité dans un littéral — évidence d'existence pour D4 SEULEMENT
    mentioned_hosts: set[str] = field(default_factory=set)
    scanned_files: int = 0


def is_excluded(rel_path: str) -> bool:
    probe = "/" + rel_path
    return any(seg in probe for seg in EXCLUDED_PATH_SEGMENTS)


def _test_module_paths(rel_path: str, mod_names: list[str]) -> set[str]:
    """Résout `#[cfg(test)] mod NAME;` vers les chemins frères possibles.

    Rust cherche `NAME.rs` puis `NAME/mod.rs`, relativement au répertoire du
    module. On rend les deux formes ; l'appelant intersecte avec l'arbre réel.
    """
    parent = rel_path.rsplit("/", 1)[0]
    stem = rel_path.rsplit("/", 1)[-1]
    # Un `mod.rs` (ou un `lib.rs`/`main.rs`) déclare ses enfants dans son
    # propre répertoire ; tout autre fichier les déclare dans un sous-
    # répertoire portant son nom.
    if stem in ("mod.rs", "lib.rs", "main.rs"):
        base = parent
    else:
        base = f"{parent}/{stem[:-3]}"
    out: set[str] = set()
    for name in mod_names:
        out.add(f"{base}/{name}.rs")
        out.add(f"{base}/{name}/mod.rs")
    return out


def build_inventory(root: Path) -> Inventory:
    """Scanne `root` et rend l'inventaire mécanique des sinks réels.

    Deux passes. La première lit chaque fichier et relève les
    `#[cfg(test)] mod NAME;` ; la seconde ne retient que les fichiers qu'aucun
    de ces relevés ne désigne. Un fichier entièrement dédié au test ne peut
    pas se déclarer tel depuis son propre contenu — c'est son déclarant qui le
    dit, et une passe unique ne peut pas le savoir avant de l'avoir lu.
    """
    inv = Inventory()
    scan_root = root / SCAN_ROOT
    if not scan_root.is_dir():
        return inv

    parsed: dict[str, list[tuple[int, str]]] = {}
    test_module_files: set[str] = set()

    for path in sorted(scan_root.rglob("*.rs")):
        rel = path.relative_to(root).as_posix()
        if is_excluded(rel):
            continue
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            # Un fichier illisible ne peut pas être déclaré innocent ; le cas
            # ne se produit pas sur un checkout sain, et l'ignorer en silence
            # serait un silence de plus. Il reste hors inventaire et hors
            # compte, donc le contrôle positif de `--report` le voit.
            continue
        prod, external_mods = production_slices(lines)
        parsed[rel] = prod
        test_module_files |= _test_module_paths(rel, external_mods)

    for rel, prod in parsed.items():
        if rel in test_module_files:
            continue
        inv.scanned_files += 1
        for lineno, content in prod:
            if CLIENT_CONSTRUCTION_RE.search(content):
                inv.client_files.setdefault(rel, []).append(lineno)
            for raw_host in URL_LITERAL_RE.findall(content):
                host = normalize_host(raw_host)
                if host is None:
                    continue
                inv.hosts.setdefault(host, []).append(f"{rel}:{lineno}")
                inv.mentioned_hosts.add(host)
            for bare in BARE_HOST_LITERAL_RE.findall(content):
                inv.mentioned_hosts.add(bare.lower())

    return inv


# --------------------------------------------------------------------------
# Manifeste
# --------------------------------------------------------------------------

REQUIRED_SINK_FIELDS = (
    "id",
    "destination",
    "destination_source",
    "class",
    "data",
    "logged",
    "client_site",
    "call_site",
    "owner",
)

VALID_DESTINATION_SOURCES = frozenset({"literal", "config", "skill-declared"})
VALID_CLASSES = frozenset({"external", "internal"})


class ManifestError(Exception):
    """Le manifeste est illisible ou incohérent — fail-closed, exit 2."""


def load_manifest(path: Path) -> list[dict]:
    """Lit et valide le manifeste. Lève `ManifestError` — jamais un vert."""
    if not path.is_file():
        raise ManifestError(
            f"manifeste introuvable à {path}\n"
            "  Le lockstep n'a rien à confronter. Un lint qui compare le code à "
            "un fichier absent\n"
            "  et se tait est un lint inerte, indiscernable d'un arbre propre "
            "(classe mika#2205)."
        )

    try:
        with path.open("rb") as handle:
            doc = tomllib.load(handle)
    except tomllib.TOMLDecodeError as exc:
        raise ManifestError(f"TOML invalide dans {path}: {exc}") from exc
    except OSError as exc:
        raise ManifestError(f"manifeste illisible à {path}: {exc}") from exc

    sinks = doc.get("sink")
    if not isinstance(sinks, list) or not sinks:
        # ANTI-VACUITÉ. Un manifeste vide ferait passer D3 et D4 trivialement
        # et réduirait D1/D2 à « tout est une violation » — ou, si quelqu'un
        # « corrigeait » ça en désarmant D1/D2, à un lint qui ne regarde plus
        # rien. Refuser ici est ce qui empêche un futur refactor de rendre la
        # garde silencieusement inerte.
        raise ManifestError(
            f"{path} ne porte aucune entrée [[sink]]\n"
            "  Anti-vacuité : un manifeste vide rend le lockstep sans objet, et "
            "un lint\n"
            "  qui compare à une liste vide se lit exactement comme un arbre "
            "propre."
        )

    seen_ids: set[str] = set()
    for index, sink in enumerate(sinks):
        if not isinstance(sink, dict):
            raise ManifestError(f"{path}: l'entrée [[sink]] #{index + 1} n'est pas une table")
        missing = [f for f in REQUIRED_SINK_FIELDS if f not in sink]
        if missing:
            ident = sink.get("id", f"#{index + 1}")
            raise ManifestError(
                f"{path}: l'entrée [[sink]] '{ident}' omet les champs requis: "
                f"{', '.join(missing)}"
            )
        ident = sink["id"]
        if not isinstance(ident, str) or not ident:
            raise ManifestError(f"{path}: l'entrée [[sink]] #{index + 1} porte un `id` vide")
        if ident in seen_ids:
            raise ManifestError(f"{path}: `id` dupliqué: '{ident}'")
        seen_ids.add(ident)
        if sink["destination_source"] not in VALID_DESTINATION_SOURCES:
            raise ManifestError(
                f"{path}: '{ident}' porte destination_source="
                f"'{sink['destination_source']}' — attendu l'un de "
                f"{sorted(VALID_DESTINATION_SOURCES)}"
            )
        if sink["class"] not in VALID_CLASSES:
            raise ManifestError(
                f"{path}: '{ident}' porte class='{sink['class']}' — attendu "
                f"l'un de {sorted(VALID_CLASSES)}"
            )
        if not isinstance(sink["logged"], bool):
            raise ManifestError(
                f"{path}: '{ident}' porte logged={sink['logged']!r} — attendu un booléen"
            )

    return sinks


# --------------------------------------------------------------------------
# Exceptions (Fire-Disposition (a) — livré VIDE et pinné vide)
# --------------------------------------------------------------------------


def load_exceptions(path: Path) -> list[tuple[str, str, str, str]]:
    """Lit `egress-manifest-exceptions.tsv`. Quatre colonnes obligatoires.

    Doctrine mika#2201 : « on déclare, on n'allowliste pas. » Ce fichier est
    livré VIDE, et la résolution d'une violation EST l'écriture d'une ligne de
    manifeste — il n'existe aucun sink qu'on ne puisse pas déclarer, y compris
    celui dont la destination est arbitraire (`skill-declared`).
    """
    if not path.is_file():
        return []
    rows: list[tuple[str, str, str, str]] = []
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = raw.split("\t")
        if len(parts) != 4 or not all(p.strip() for p in parts):
            raise ManifestError(
                f"{path}:{lineno}: quatre colonnes obligatoires attendues "
                "(chemin\tdirection\tticket\traison), toutes non vides"
            )
        rows.append(tuple(p.strip() for p in parts))  # type: ignore[arg-type]
    return rows


# --------------------------------------------------------------------------
# Les quatre directions
# --------------------------------------------------------------------------


@dataclass
class Violation:
    direction: str
    label: str
    detail: str


def covers(client_site: str, rel_path: str) -> bool:
    """Un `client_site` couvre un fichier par préfixe de chemin.

    Un `client_site` qui se termine par `/` désigne un répertoire ; sinon il
    désigne un fichier exact.
    """
    if client_site.endswith("/"):
        return rel_path.startswith(client_site)
    return rel_path == client_site


def check(inv: Inventory, sinks: list[dict]) -> list[Violation]:
    """Applique D1–D4 et rend la liste des violations."""
    violations: list[Violation] = []

    client_sites = [s["client_site"] for s in sinks]
    declared_hosts = set()
    for sink in sinks:
        dest = sink["destination"]
        if isinstance(dest, str) and dest:
            declared_hosts.add(dest.lower())
        for extra in sink.get("confined_hosts", []) or []:
            if isinstance(extra, str) and extra:
                declared_hosts.add(extra.lower())
        for extra in sink.get("also_declares", []) or []:
            if isinstance(extra, str) and extra:
                declared_hosts.add(extra.lower())

    # --- D1 : code → manifeste (constructions de client) -------------------
    for rel_path, linenos in sorted(inv.client_files.items()):
        if not any(covers(cs, rel_path) for cs in client_sites):
            violations.append(
                Violation(
                    "D1",
                    "undeclared client site",
                    f"{rel_path}:{linenos[0]} construit un client HTTP en "
                    "production et aucune entrée [[sink]] ne le couvre",
                )
            )

    # --- D2 : code → manifeste (littéraux de host) -------------------------
    for host, sites in sorted(inv.hosts.items()):
        if is_non_sink_host(host):
            continue
        if host in declared_hosts:
            continue
        # Un host déclaré peut être un suffixe du host rencontré : une entrée
        # `gouv.fr` couvre `data.gouv.fr`. La relation est volontairement
        # dans ce sens seulement — déclarer `fr` ne couvre pas tout le TLD,
        # la borne étant le point séparateur.
        if any(host.endswith("." + declared) for declared in declared_hosts):
            continue
        violations.append(
            Violation(
                "D2",
                "undeclared destination",
                f"host '{host}' apparaît en production à {sites[0]}"
                + (f" (+{len(sites) - 1} autre(s))" if len(sites) > 1 else "")
                + " et aucune entrée [[sink]] ne le déclare",
            )
        )

    # --- D3 : manifeste → code (client_site fantôme) -----------------------
    for sink in sinks:
        client_site = sink["client_site"]
        if not any(covers(client_site, rel) for rel in inv.client_files):
            violations.append(
                Violation(
                    "D3",
                    "phantom client_site",
                    f"[[sink]] '{sink['id']}' déclare client_site="
                    f"'{client_site}', qui ne couvre aucune construction de "
                    "client HTTP en production",
                )
            )

    # --- D4 : manifeste → code (destination littérale fantôme) -------------
    #
    # Ne s'applique PAS aux `config` / `skill-declared` : leur destination
    # n'est par construction dans aucun littéral, et exiger le contraire
    # ferait rougir le lint sur les déclarations les plus honnêtes du fichier.
    seen_hosts = set(inv.hosts) | inv.mentioned_hosts
    for sink in sinks:
        if sink["destination_source"] != "literal":
            continue
        candidates = [sink["destination"]]
        candidates.extend(sink.get("confined_hosts", []) or [])
        candidates.extend(sink.get("also_declares", []) or [])
        for dest in candidates:
            dest_lc = str(dest).lower()
            if dest_lc in seen_hosts:
                continue
            if any(seen.endswith("." + dest_lc) for seen in seen_hosts):
                continue
            violations.append(
                Violation(
                    "D4",
                    "phantom destination",
                    f"[[sink]] '{sink['id']}' déclare destination_source="
                    f"'literal' pour '{dest}', qui n'apparaît dans aucun "
                    "littéral de production",
                )
            )

    return violations


# --------------------------------------------------------------------------
# Entrée
# --------------------------------------------------------------------------

RESOLUTION = """
Pour résoudre une violation, on DÉCLARE — on n'allowliste pas (doctrine mika#2201).

  * D1 « undeclared client site »  — le fichier construit un client HTTP et
    aucune entrée ne le couvre. Ajouter une entrée [[sink]] dans le manifeste
    dont `client_site` nomme ce fichier (ou son répertoire), en remplissant
    `data`, `destination`, `logged` et `owner`.

  * D2 « undeclared destination »  — un host part du code sans qu'une ligne
    dise ce qu'on lui envoie. Ajouter sa `destination` à une entrée existante
    (champ `also_declares` si l'entrée en porte déjà une), ou créer l'entrée.
    Si le host n'est pas routable (loopback, RFC 2606, cluster interne), c'est
    NON_SINK_HOSTS qu'il faut étendre — par CLASSE, jamais par host isolé.

  * D3 « phantom client_site »     — l'entrée nomme un chemin où aucun client
    n'est construit. Corriger le champ, ou retirer l'entrée : un manifeste qui
    décrit un sink disparu est un manifeste qui ment.

  * D4 « phantom destination »     — la `destination` déclarée `literal`
    n'apparaît nulle part. Si la destination vient d'une configuration
    repointable, c'est `destination_source = "config"` qu'il faut poser ;
    si elle vient du manifeste d'une skill, `"skill-declared"`.

Le schéma complet et la procédure d'ajout : docs/egress/README.md
`docs/egress/` est sous CODEOWNERS @samidarko — aucune entrée ne merge sans
revue humaine.
"""


def confined_hosts(sinks: list[dict]) -> list[str]:
    """Les hosts que `verify-egress-uniqueness.sh` doit confiner (AC5).

    Une entrée `confined = true` contribue ses `confined_hosts` s'ils sont
    présents, sinon sa `destination` seule. Un substrat peut confiner plusieurs
    hosts pour une entrée — `egress_fetch` en compte quatre.

    C'est ce qui ferme AC5 : le manifeste devient la SOURCE UNIQUE des hosts, et
    la liste qui vivait en dur dans `verify-egress-uniqueness.sh` disparaît.
    """
    out: list[str] = []
    for sink in sinks:
        if not sink.get("confined"):
            continue
        hosts = sink.get("confined_hosts") or [sink["destination"]]
        for host in hosts:
            if isinstance(host, str) and host and host not in out:
                out.append(host)
    return out


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if not a.startswith("--")]
    report = "--report" in argv[1:]
    want_confined = "--confined-hosts" in argv[1:]
    want_cut_audit = "--audit-cut-holes" in argv[1:]

    root = Path(args[0]).resolve() if args else Path.cwd()

    # --audit-cut-holes : remesure la limite connue de la découpe
    # production/test (voir `production_slices`). Ne lit pas le manifeste — la
    # question ne porte que sur l'arbre. Sortie vide = régime nominal.
    if want_cut_audit:
        holes = audit_cut_holes(root)
        for hole in holes:
            print(hole)
        return 1 if holes else 0
    manifest_path = (
        Path(args[1]).resolve()
        if len(args) > 1
        else root / "docs" / "egress" / "egress-manifest.toml"
    )
    exceptions_path = root / "scripts" / "egress-manifest-exceptions.tsv"

    try:
        sinks = load_manifest(manifest_path)
        exceptions = load_exceptions(exceptions_path)
    except ManifestError as exc:
        print(f"ERROR (egress-manifest): {exc}", file=sys.stderr)
        return 2

    # --confined-hosts : mode de DÉRIVATION, consommé par
    # `verify-egress-uniqueness.sh`. Un host par ligne, rien d'autre sur stdout
    # — l'appelant en fait un tableau bash. Aucun inventaire n'est construit :
    # la question posée ne porte que sur le manifeste.
    if want_confined:
        hosts = confined_hosts(sinks)
        if not hosts:
            # FAIL-CLOSED, et c'est le terme qui compte. Une garde de
            # confinement qui perd ses patterns se lit exactement comme une
            # garde qui passe — rendre une liste vide en sortant 0 désarmerait
            # `verify-egress-uniqueness.sh` en silence.
            print(
                f"ERROR (egress-manifest): aucune entrée `confined = true` dans "
                f"{manifest_path}\n"
                "  scripts/verify-egress-uniqueness.sh dérive ses PATTERNS d'ici. "
                "Une liste vide\n"
                "  le désarmerait en silence : une garde de confinement sans "
                "pattern se lit\n"
                "  exactement comme une garde qui passe.",
                file=sys.stderr,
            )
            return 2
        for host in hosts:
            print(host)
        return 0

    inv = build_inventory(root)

    # CONTRÔLE POSITIF, et il n'est pas décoratif. Un inventaire à zéro avec
    # exit 0 serait un lint qui ne regarde plus rien (répertoire renommé,
    # extension changée) et qui se lit exactement comme un arbre propre
    # (classe mika#2205). Il refuse de lui-même.
    if inv.scanned_files == 0:
        print(
            f"ERROR (egress-manifest): aucun fichier .rs scanné sous {root}/{SCAN_ROOT}\n"
            "  Le lint ne regarde rien. Un inventaire vide qui sortirait 0 se "
            "lirait exactement\n"
            "  comme un arbre propre — c'est le refus, pas l'arbre, qu'il faut "
            "croire.",
            file=sys.stderr,
        )
        return 2

    if report:
        print(
            f"egress-manifest: {inv.scanned_files} fichier(s) .rs de production "
            f"scanné(s), {len(inv.client_files)} portant une construction de "
            f"client, {len(inv.hosts)} host(s) distinct(s) rencontré(s), "
            f"{len(sinks)} entrée(s) [[sink]] confrontée(s)."
        )

    violations = check(inv, sinks)

    # Les exceptions ne suppriment pas une violation : elles sont listées pour
    # que l'assertion auto-nettoyante du test puisse vérifier qu'elles
    # correspondent encore à une violation réelle. Le fichier est livré vide.
    if exceptions:
        exempt_paths = {row[0] for row in exceptions}
        kept = [v for v in violations if not any(p in v.detail for p in exempt_paths)]
        if report:
            print(
                f"egress-manifest: {len(violations) - len(kept)} violation(s) "
                f"couverte(s) par {len(exceptions)} exception(s)."
            )
        violations = kept

    if violations:
        for v in violations:
            print(f"ERROR (egress-manifest, {v.direction}): {v.label}: {v.detail}")
        print("")
        print(f"Found {len(violations)} egress-manifest violation(s).")
        print(RESOLUTION)
        return 1

    print("No egress-manifest violations found.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

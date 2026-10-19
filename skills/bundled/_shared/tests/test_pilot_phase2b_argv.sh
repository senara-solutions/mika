#!/bin/bash
# Suite : l'entrypoint Phase 2b passe l'argv du pilote en PARAMÈTRES POSITIONNELS
# (mika#2077, reprise de mika#2057).
#
# LE DÉFAUT QU'ELLE GARDE. Jusqu'à mika#2077, `_run_pilot_sandboxed` sérialisait
# l'argv du pilote avec `quoted_argv=$(printf ' %q' "$@")` et interpolait le
# résultat dans le corps du script exécuté par `$sandbox_entrypoint_prefix -c`.
# `printf '%q'` est un builtin BASH et sa sortie est du BASH : pour tout argument
# portant un caractère non imprimable — saut de ligne, tabulation, \r — il émet
# la forme ANSI-C `$'...'`, que `dash` ne sait pas lire. Rien ne garantit que le
# shell d'entrypoint soit bash. Le pilote démarrait donc avec un argument
# DIFFÉRENT de celui qui lui était destiné, silencieusement, et la session se
# diagnostiquait comme une dérive du modèle. Phase 2a, elle, passait déjà l'argv
# en positionnels ; ce vol a aligné 2b sur 2a.
#
# QUATRE PARTIES, et pourquoi il en faut quatre.
#   PART A  comportemental. Le corps de l'entrypoint est une fonction depuis
#           mika#2077, donc il s'EXÉCUTE hors bwrap : « l'argv traverse intact »
#           est une mesure et non une affirmation.
#   PART B  structurel, sur le fichier tracké. Deux comptes EXACTS.
#   PART C  anti-vacuité. B1 et B2 relancées sur une copie régressée, qui doivent
#           ÉCHOUER. Sans elle, deux assertions dont rien ne prouve qu'elles
#           savent dire non — la classe que mika#1772 a trouvée en câblant
#           test-dispatch-lib.sh.
#   PART D  préconditions. Ni bwrap, ni réseau, ni token : la suite est wirable
#           sur un runner hébergé par construction.
#
# Run: bash skills/bundled/_shared/tests/test_pilot_phase2b_argv.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Source isolation : dispatch-lib.sh n'a aucun code impératif au niveau
# supérieur — même audit que tests/test_sandbox_no_secret_in_argv.sh.
# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_true() {
    local label="$1" cond_rc="$2"
    if [ "$cond_rc" -eq 0 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
    fi
}

# ============================================================================
# PART D — préconditions
# ============================================================================
# Le corps de l'entrypoint sonde le shim avec `python3`. Son absence rend la
# PART A inexécutable ; la suite le DIT et sort proprement, plutôt que de
# compter des assertions qu'elle n'a pas faites.
if ! command -v python3 >/dev/null 2>&1; then
    echo "⊘ skipped — python3 absent du PATH (l'entrypoint 2b le requiert)"
    exit 0
fi

TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2077-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# ============================================================================
# PART A — comportemental : l'argv traverse l'entrypoint octet pour octet
# ============================================================================
echo ""
echo "PART A — l'argv traverse l'entrypoint 2b en positionnels"
echo "---------------------------------------------------------"

# Un faux proxy d'egress : il lie le port que le script lui passe, pour que la
# boucle d'attente du shim sorte au premier essai, puis il dort. Il ne parle à
# personne et n'écrit nulle part. Si le `bind` échoue (port pris par un test
# voisin), il dort quand même : la boucle d'attente retombe alors sur son
# plafond d'une seconde et les assertions ci-dessous sont inchangées.
FAKE_PROXY="$TMPROOT/fake-egress-proxy.py"
cat > "$FAKE_PROXY" <<'PY'
import socket
import sys
import time

argv = sys.argv[1:]
port = 0
for i, a in enumerate(argv):
    if a == "--sandbox-tcp" and i + 1 < len(argv):
        port = int(argv[i + 1])
if port:
    try:
        s = socket.socket()
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("127.0.0.1", port))
        s.listen(5)
    except OSError:
        pass
time.sleep(10)
PY

# La commande cible : elle enregistre l'argument reçu, puis sort avec le statut
# qu'on lui demande. C'est le seul témoin possible — ce que le pilote a REÇU.
WRITER="$TMPROOT/record-argv.sh"
cat > "$WRITER" <<'SH'
#!/bin/sh
# $1 = fichier de sortie, $2 = valeur à enregistrer, $3 = statut de sortie
printf '%s' "$2" > "$1"
exit "${3:-0}"
SH
chmod +x "$WRITER"

# Un port libre, lu puis relâché. Rien ici n'a besoin qu'il reste mort — au
# contraire, on veut qu'un listener s'y installe — donc la doctrine mika#2569
# (DeadEndpoint) ne s'applique pas : elle borne les endpoints qu'un test déclare
# INJOIGNABLES.
FREE_PORT=$(python3 -c 'import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()')

# Les quatre globales que la fonction lit, posées sur des valeurs de test.
_PILOT_SECRET_PROLOGUE=""
_PILOT_EGRESS_PROXY_BIN="$FAKE_PROXY"
_PILOT_EGRESS_TCP_PORT="$FREE_PORT"
_PILOT_EGRESS_SOCK="$TMPROOT/egress.sock"

# Exécute l'entrypoint sous $1, avec l'argv de test en positionnels — exactement
# la forme que `_run_pilot_sandboxed` passe à bwrap.
run_entrypoint() {
    local shell_bin="$1" out="$2" value="$3" want_rc="$4" rc=0
    "$shell_bin" -c "$(_pilot_phase2b_entrypoint_script)" \
        mika-pilot-sandbox "$WRITER" "$out" "$value" "$want_rc" \
        2>>"$TMPROOT/entrypoint.stderr" || rc=$?
    echo "$rc"
}

# --- A1 : saut de ligne + apostrophe, le cas exact que `%q` échappe en `$'...'`
A1_ARG=$'premiere ligne\ndeuxieme ligne avec une \' apostrophe'
A1_OUT="$TMPROOT/a1.out"
run_entrypoint /bin/sh "$A1_OUT" "$A1_ARG" 0 >/dev/null
assert_eq "A1: un argument multi-ligne avec apostrophe traverse octet pour octet" \
    "$A1_ARG" "$(cat "$A1_OUT")"

# A1 sous `dash`, quand il est là. C'est l'assertion DISCRIMINANTE : sous un
# /bin/sh qui est bash, l'ancienne forme `printf '%q'` aurait fonctionné et A1
# ne prouverait rien du défaut. Sous dash, `$'...'` n'est pas lu — et c'est
# exactement ce que le pilote rencontrait.
if command -v dash >/dev/null 2>&1; then
    A1D_OUT="$TMPROOT/a1-dash.out"
    run_entrypoint dash "$A1D_OUT" "$A1_ARG" 0 >/dev/null
    assert_eq "A1: même argument, sous dash (le shell où \$'...' n'est pas lu)" \
        "$A1_ARG" "$(cat "$A1D_OUT")"
else
    echo "  ⊘ A1/dash sauté — dash absent du PATH"
fi

# --- A2 : tabulation + `$(...)` littéral — contrôle négatif de la réévaluation
A2_ARG=$'avant\tapres $(id -u) ${HOME} fin'
A2_OUT="$TMPROOT/a2.out"
run_entrypoint /bin/sh "$A2_OUT" "$A2_ARG" 0 >/dev/null
A2_GOT="$(cat "$A2_OUT")"
assert_eq "A2: tabulation et \$(...) littéral traversent intacts" "$A2_ARG" "$A2_GOT"
# Dit en clair ce que l'égalité ci-dessus implique, pour que l'échec nomme la
# cause : un argv re-cité puis ré-interprété est la porte par laquelle une
# expansion non voulue entrerait.
rc=1; case "$A2_GOT" in *'$(id -u)'*) rc=0 ;; esac
assert_true "A2: la substitution de commande n'a pas été réévaluée" "$rc"

# --- A3 : le statut du pilote est le statut du script
A3_OUT="$TMPROOT/a3.out"
A3_RC=$(run_entrypoint /bin/sh "$A3_OUT" "statut" 7)
assert_eq "A3: le statut de sortie du pilote (7) est celui du script" "7" "$A3_RC"

# ============================================================================
# PART B — structurel : les deux comptes exacts, sur le fichier tracké
# ============================================================================
echo ""
echo "PART B — la forme du substrat, en comptes exacts"
echo "-------------------------------------------------"

# « Ligne de code » = premier caractère non blanc différent de `#`. Volontairement
# SOUS-approximé : un `printf ' %q'` en queue d'une ligne de code resterait
# compté, donc l'erreur possible est un FAUX ROUGE, bruyant mais jamais muet.
# L'inverse — sur-dépouiller et cacher une occurrence réelle — est le faux vert
# que ces deux comptes existent pour interdire.
code_lines() {
    grep -vE '^[[:space:]]*#' "$1" || true
}

# B1 — les DEUX chemins de sandbox (2a et 2b) passent l'argv en positionnels.
# Compte EXACT et non borne inférieure : un troisième chemin ajouté demain,
# correct ou non, fait rougir cette ligne et force son auteur à venir déclarer
# ici ce qu'il fait. `>= 2` l'aurait laissé passer en silence.
count_positional_handoff() {
    code_lines "$1" | grep -cF 'mika-pilot-sandbox "$@"' || true
}

# B2 — aucune re-citation de l'argv. Le prédicat porte sur le SITE (la
# sérialisation de l'argv d'entrypoint) et non sur la valeur `%q` n'importe où :
# doctrine mika#2309 — un prédicat sur la valeur attrape des tiers innocents et
# finit en allowlist, un prédicat sur le site borne ce qu'il doit borner.
count_requoted_argv() {
    code_lines "$1" | grep -cF "printf ' %q'" || true
}

assert_eq "B1: les deux chemins de sandbox passent l'argv en positionnels" \
    "2" "$(count_positional_handoff "$DISPATCH_LIB")"
assert_eq "B2: aucune ligne de code ne re-cite l'argv avec printf ' %q'" \
    "0" "$(count_requoted_argv "$DISPATCH_LIB")"

# ============================================================================
# PART C — anti-vacuité : B1 et B2 savent-elles dire non ?
# ============================================================================
echo ""
echo "PART C — B1 et B2 sur une copie régressée : elles doivent échouer"
echo "------------------------------------------------------------------"

REGRESSED="$TMPROOT/dispatch-lib-regressed.sh"
# Réintroduit la forme mika#2057 : l'argv sérialisé par `printf '%q'` et
# interpolé dans le corps, au lieu du passage positionnel.
awk '
    /-- "\$sandbox_entrypoint_prefix" -c "\$\(_pilot_phase2b_entrypoint_script\)"/ {
        print "        quoted_argv=$(printf '"'"' %q'"'"' \"$@\")"
        print "            -- \"$sandbox_entrypoint_prefix\" -c \"$(_pilot_phase2b_entrypoint_script)$quoted_argv\" \\"
        next
    }
    /^ *mika-pilot-sandbox "\$@" \|\| _sandbox_rc=\$\?$/ {
        print "               || _sandbox_rc=$?"
        next
    }
    { print }
' "$DISPATCH_LIB" > "$REGRESSED"

# La régression a-t-elle MORDU ? Si les motifs ci-dessus dérivent, la copie
# redevient l'original et les deux « échecs » attendus seraient des succès
# déguisés en preuve. On le dit ici, pour que l'échec nomme sa cause.
rc=1; ! cmp -s "$DISPATCH_LIB" "$REGRESSED" && rc=0
assert_true "C0: la copie régressée diffère de l'original (régression effective)" "$rc"

assert_eq "C1: B1 échoue sur la copie régressée (un seul passage positionnel)" \
    "1" "$(count_positional_handoff "$REGRESSED")"
assert_eq "C2: B2 échoue sur la copie régressée (une re-citation réapparaît)" \
    "1" "$(count_requoted_argv "$REGRESSED")"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0

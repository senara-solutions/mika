---
title: "fix(egress): socket du relais en 0600 sans fenêtre, lanceur sous environnement minimal"
type: fix
status: active
date: 2026-10-02
origin: free-text dispatch (orchestrateur)
---

# fix(egress): socket du relais en 0600 sans fenêtre, lanceur sous environnement minimal

## Goal Capsule

- **Objectif :** durcissement par moindre privilège du relais d'egress des pilotes, sur deux surfaces : le mode du socket unix qu'il crée, et l'environnement qu'il hérite de son lanceur.
- **Moyens fixés :** (1) le relais crée et lie lui-même son socket sous un umask restrictif, puis le passe à asyncio ; mode final 0600. (2) `_ensure_pilot_egress_proxy` lance le relais sous `env -i` avec une liste blanche nommée.
- **Autorité :** dépôt `mika`, fichiers `scripts/mika-pilot-egress-proxy`, `skills/bundled/_shared/dispatch-lib.sh`, tests associés, `Makefile` (câblage de la suite), doc opérateur. Interdits : `perimeter/`, `docs/gate/`, `.github/`.
- **Conditions d'arrêt :** un consommateur réel du socket qui ne tourne pas sous le même uid ; une variable lue par le relais **en production** et nécessaire à sa fonction absente de la liste blanche.
- **Hors champ :** relancer le relais en service (fait au déploiement, par l'orchestrateur) ; l'environnement du démon mitmdump (`_ensure_pilot_helper`), qui est un autre processus.

---

## Product Contract

### Problem Frame

Le relais est l'unique sortie réseau du bac à sable des pilotes. Deux de ses propriétés excèdent ce dont il a besoin :

1. **Mode du socket.** `scripts/mika-pilot-egress-proxy:1312-1315` lie le socket via `asyncio.start_unix_server(path)` puis appelle `sock_path.chmod(0o666)`, avec le commentaire « World-writable so the sandbox (running as same uid, but env-cleared) can still connect ». La justification se contredit : si le bac à sable tourne sous le même uid, le propriétaire suffit, et 0666 ouvre le socket à tout uid local. De plus, entre `bind()` et `chmod()`, le socket existe avec le mode dérivé de l'umask du processus : le mode final n'est pas garanti pendant cette fenêtre.
2. **Environnement hérité.** `skills/bundled/_shared/dispatch-lib.sh` (`_ensure_pilot_egress_proxy`) lance `nohup "$_PILOT_EGRESS_PROXY_BIN" --host-unix ...` avec l'environnement complet de la tâche de dispatch. Mesuré par l'orchestrateur : 15 variables, dont `MIKA_DISPATCH_WORKTREE_FILE` et un jeton GitHub. Le relais est un démon long qui survit à la tâche qui l'a lancé ; il conserve donc durablement des valeurs qu'il ne lit jamais.

### Requirements

- R1. Le socket lié par `--host-unix` a le mode `0600` (owner rw seulement).
- R2. Aucun instant entre la création du chemin et son annonce (`host-unix listening`) où le socket existe avec un mode plus permissif que `0600`.
- R3. La barrière de test `_MIKA_EGRESS_PREBIND_TEST_BARRIER` et le test `test_pre_bind_signal_names_its_cause` restent valides (la barrière précède toujours la création du socket).
- R4. Le lanceur ne transmet au relais que les variables d'une liste blanche nommée ; chaque variable de la liste n'est transmise que si elle est définie chez l'appelant.
- R5. La liste blanche couvre tout ce que le relais lit en production pour sa fonction (la seam de test `_MIKA_EGRESS_PREBIND_TEST_BARRIER` et `SSLKEYLOGFILE` sont exclues délibérément, voir Context).
- R6. `scripts/canary-pilot-containment --ensure-relay/--restart-relay` héritent du traitement sans second lanceur.
- R7. Un pilote sous bwrap, même uid, atteint toujours le socket `0600`.

### Scope Boundaries

- Pas de changement à l'allowlist d'hôtes, au protocole, ni au chemin du socket.
- Pas de changement au lancement de mitmdump : l'addon `scripts/mika-pilot-github-auth-addon.py` s'exécute dans mitmdump (lancé par `_ensure_pilot_helper`), pas dans le relais. Son repli sur l'environnement du processus est un comportement voulu de ce démon-là.

---

## Planning Contract

### Context & Research

Lectures d'environnement du relais, inventaire exhaustif dans `scripts/mika-pilot-egress-proxy` :

| Source | Ligne | Variable | Décision |
|---|---|---|---|
| `os.environ.get` | 708 | `MIKA_EGRESS_DEBUG` | transmise si définie |
| `os.environ.get` | 1292 | `_MIKA_EGRESS_PREBIND_TEST_BARRIER` | **non** transmise : seam de test, posée directement par `test-pilot-egress-proxy-status.py` qui lance le script sans le lanceur ; le commentaire l.1290 dit déjà « the launcher does not export it » |
| `Path.home()` | 198 | `HOME` (implicite) | transmise : chemin de `~/.claude/.credentials.json` |
| shebang `#!/usr/bin/env python3` | 1 | `PATH` | transmise |
| `ssl.create_default_context()` | 1024 | `SSL_CERT_FILE`, `SSL_CERT_DIR` (lus par OpenSSL) | transmises si définies : un hôte qui les pose pour la TLS amont garde son comportement |
| locale Python | — | `LANG` | transmise si définie |
| `ssl.create_default_context()` | 1024 | `SSLKEYLOGFILE` | **non** transmise : variable de diagnostic qui écrirait les secrets de session TLS amont sur disque |

`MIKA_PILOT_EGRESS_LOG_DIR` est lu par le **shell lanceur** (redirection `>>"$log_file"`), pas par le relais : non transmis. Aucun `subprocess`/`Popen` dans le relais.

Lanceurs : `_ensure_pilot_egress_proxy` est l'unique site `nohup` du relais. `scripts/canary-pilot-containment` l'appelle aux l.261 (`--ensure-relay`), 315 (`--restart-relay`), 370 et 447 : R6 est satisfait par construction.

Effet de bord identifié : le fixture `_egress_guard_probe` de `skills/bundled/_shared/test-dispatch-lib.sh` (~l.5245) écrit son marqueur de lancement via `touch "$MIKA_TEST_LAUNCH_MARKER"`. Sous `env -i` la variable disparaît et l'assertion `launched=yes` rougirait pour une raison étrangère à son objet. Le faux relais dérivera le marqueur de son propre chemin (`$(dirname "$0")/launched`, le binaire et le marqueur vivent dans le même `$tmp`), sans toucher aux assertions.

Canary : `scripts/canary-pilot-containment` exécute `install -Dm755 "$SCRIPT" "$INSTALLED_PROXY"` (l.81) dans **tous** ses modes, et son mode par défaut parle au socket du relais en service (chemin fixe `/tmp/mika-pilot-egress.sock`). Conséquences : (a) le lancer depuis la branche remplace le binaire déployé ; (b) son verdict vert prouve la non-régression du confinement mais **pas** l'accès à un socket `0600`, puisque le relais vivant tourne l'ancien code. D'où U3.

### Key Technical Decisions

- KTD1. **Créer le socket soi-même plutôt qu'umask autour de `start_unix_server(path)`.** Un helper `_bind_owner_only_unix_socket(path)` : `socket.socket(AF_UNIX)`, `os.umask(0o177)` autour du seul `bind()` (restauré en `finally`), puis `os.chmod(path, 0o600)`, puis `asyncio.start_unix_server(handler, sock=sock)`. Le bind devient synchrone et visible dans le code ; l'umask n'est modifié que pendant un appel système sans `await` intermédiaire. **C'est l'umask qui ferme la fenêtre** (le socket naît 0600) ; le `chmod` est une ceinture qui ne porte pas R2, et le test doit le prouver en le neutralisant.
- KTD2. **Le nettoyage à l'arrêt est inchangé.** Il repose sur l'inode relevé après bind (`os.stat(sock_path).st_ino`) ; avec `sock=`, CPython 3.13+ peut aussi tenter l'unlink, déjà toléré (`FileNotFoundError`).
- KTD3. **Liste blanche en tableau nommé dans dispatch-lib**, `_PILOT_EGRESS_RELAY_ENV_ALLOWLIST=(PATH HOME LANG MIKA_EGRESS_DEBUG SSL_CERT_FILE SSL_CERT_DIR)`, consommée par une boucle `${!v+x}` (sûre sous `set -u`). `nohup env -i "${relay_env[@]}" "$BIN" ...` : `env` fait `exec`, donc `$!` reste le pid du relais et la jointure pid (mika#2051) est préservée.
- KTD4. **Preuve bwrap sur socket temporaire, pas sur le socket vivant.** Nouvelle suite qui lance le **vrai** relais de la branche par le **vrai** `_ensure_pilot_egress_proxy` sur un chemin temporaire, puis exécute un connect depuis `_run_pilot_sandboxed`. Ne touche ni au relais en service ni à son socket.
- KTD5. **Lecture de l'environnement réel par `/proc/<pid>/environ`** du relais lancé, plutôt qu'un faux relais qui se décrit lui-même : c'est le processus de production qui est observé.

### Open Questions

Résolues pendant le planning : la barrière de test précède le bloc de création (l.1292 < l.1297) et y reste ; aucun autre lanceur du relais n'existe.

---

## Implementation Units

### U1. Socket lié sans fenêtre, mode 0600

- **Fichiers :** `scripts/mika-pilot-egress-proxy`, `scripts/test-pilot-egress-proxy-status.py`
- **Approche :** remplacer `start_unix_server(handle_host_client, str(sock_path))` + `chmod(0o666)` par la séquence KTD1 ; réécrire le commentaire (même uid ⇒ propriétaire suffit ; umask autour du bind ⇒ pas de fenêtre). En cas d'échec du bind, fermer le socket et sortir via `_log` + `sys.exit(1)` comme les autres FATAL de démarrage.
- **Tests :** (a) `HostSocketLifecycleTests.test_bound_socket_is_owner_only` — relais réel, après `_wait_connectable`, `stat.S_IMODE == 0o600` et `S_ISSOCK`. (b) `test_socket_is_born_owner_only_without_chmod` — appelle `_bind_owner_only_unix_socket` sous umask appelant permissif (`0o022`) avec `os.chmod` neutralisé : le mode à la naissance doit être `0o600`, ce qui atteste R2 indépendamment du `chmod`. Les tests existants de cycle de vie restent verts.
- **Contrôles négatifs (deux, séparés) :** retirer **seulement** l'umask ⇒ (b) rougit ; remettre `0o666` ⇒ (a) rougit. Consignés dans la PR.

### U2. Lanceur sous environnement minimal

- **Fichiers :** `skills/bundled/_shared/dispatch-lib.sh`, `skills/bundled/_shared/test-dispatch-lib.sh`
- **Approche :** KTD3 ; commentaire au-dessus de la liste qui renvoie à l'inventaire (une variable lue par le relais doit y être ajoutée). Fixture `_egress_guard_probe` : marqueur dérivé de `$0`.
- **Tests :** voir U3 (environnement observé sur le vrai relais). Les assertions existantes de `_egress_guard_probe` restent identiques et vertes.

### U3. Suite bwrap : socket 0600 atteignable, environnement filtré

- **Fichiers :** `skills/bundled/_shared/tests/test_egress_relay_socket_and_env.sh` (nouveau), `Makefile` (cible `test` + cible dédiée).
- **Approche :** sur le modèle de `test_sandbox_gh_usable.sh` (skip si `bwrap` absent). Dans un `bash -c` isolé : `HOME` temporaire **exporté avant le `source`** de dispatch-lib (les chemins `$HOME/.mika/pilot-gh-token` et mitmdump sont figés au sourcing), `GH_TOKEN` retiré, `_ensure_pilot_helper() { return 1; }` (aucun mitmdump lancé), `MIKA_PILOT_EGRESS_LOG_DIR` temporaire, `_PILOT_EGRESS_SOCK` temporaire, `_PILOT_EGRESS_PROXY_BIN` = le script de la branche, `_pilot_egress_mark_up/down` neutralisés, `export` d'une variable sentinelle et de `MIKA_EGRESS_DEBUG=1`, puis `_ensure_pilot_egress_proxy` **réel**. Rien n'écrit dans l'état opérateur ni ne touche le socket vivant.
- **Assertions :** mode `600` du socket ; `/proc/<pid>/environ` du relais ne contient pas la sentinelle, contient `MIKA_EGRESS_DEBUG=1`, `HOME=`, `PATH=` ; `_run_pilot_sandboxed` exécutant un connect python sur le chemin du socket imprime `connected`. Le relais est arrêté par SIGTERM en fin de suite (la suite vérifie aussi que le socket est retiré).
- **Contrôle négatif intégré :** copie de dispatch-lib où `env -i "${relay_env[@]}"` est retiré (précondition : exactement une ligne diffère) ⇒ la sentinelle **est** présente dans l'environnement du relais.

### U4. Documentation opérateur

- **Fichiers :** `docs/operator/pilot-egress-relay.md`
- **Approche :** court paragraphe : mode `0600`, environnement en liste blanche, et la règle « une variable dont le relais a besoin pour sa fonction s'ajoute à `_PILOT_EGRESS_RELAY_ENV_ALLOWLIST` ; les variables de diagnostic qui écrivent des secrets (`SSLKEYLOGFILE`) restent exclues ». **Activation :** `make deploy` ne remplace que le binaire et `_ensure_pilot_egress_proxy` ne relance qu'un relais injoignable, donc le durcissement ne prend effet qu'après `scripts/canary-pilot-containment --restart-relay`. Sonde post-relance : `stat -c %a /tmp/mika-pilot-egress.sock` rend `600`, et `/proc/<pid du relais>/environ` ne porte que des noms de la liste blanche. Rédaction neutre.

---

## Verification Contract

- `python3 -B scripts/test-pilot-egress-proxy-status.py` vert.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` : aucun échec nouveau par rapport à `main`.
- `bash skills/bundled/_shared/tests/test_egress_relay_socket_and_env.sh` vert, contrôle négatif intégré compris.
- Contrôles négatifs manuels vus rouges : `0o666` (U1), `env -i` retiré (U3 intégré).
- `scripts/canary-pilot-containment` vert. **Précondition :** le socket vivant est joignable juste avant le lancement (sinon le canary lancerait le relais de branche sur le chemin de production : pas de canary, et le saut est consigné dans la PR). Binaire installé sauvegardé avant et restauré à l'identique immédiatement après (somme sha256 comparée), pour réduire la fenêtre où un dispatch concurrent lirait le binaire de branche ; le relais en service n'est ni tué ni relancé.
- **Étape d'activation (orchestrateur, au déploiement, hors de cette PR) :** `--restart-relay` puis la sonde de U4 ; la PR la nomme comme en attente.

## Definition of Done

- R1-R7 couverts par U1-U4, tests verts, contrôles négatifs rouges observés et consignés dans la PR.
- Aucun fichier sous `perimeter/`, `docs/gate/`, `.github/` dans le diff.

## Acceptance criteria

- [ ] Le socket lié par `mika-pilot-egress-proxy --host-unix` a le mode `0600`, asserté par un test Python ; le test rougit si le mode redevient `0666`.
- [ ] Le socket naît `0600` sans dépendre du `chmod` (test avec `chmod` neutralisé sous umask `0o022`), rouge quand l'umask seul est retiré ; `test_pre_bind_signal_names_its_cause` reste vert.
- [ ] `_ensure_pilot_egress_proxy` lance le relais sous `env -i` avec la liste blanche `PATH HOME LANG MIKA_EGRESS_DEBUG SSL_CERT_FILE SSL_CERT_DIR` (chacune seulement si définie).
- [ ] Un test asserte qu'une variable sentinelle exportée par l'appelant est absente de l'environnement du relais lancé, et que `MIKA_EGRESS_DEBUG`, `HOME`, `PATH` y sont ; le contrôle négatif sans `env -i` montre la sentinelle présente.
- [ ] Un pilote sous bwrap, même uid, se connecte au socket `0600` (suite U3) ; `scripts/canary-pilot-containment` vert (ou saut consigné si la précondition n'est pas remplie).
- [ ] La doc opérateur nomme l'étape d'activation (`--restart-relay`) et sa sonde post-relance.
- [ ] Les assertions existantes de `_egress_guard_probe` passent inchangées.

---
title: PAT GitHub hors de l'argv de bwrap - Plan
type: security
date: 2026-08-29
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# PAT GitHub hors de l'argv de bwrap - Plan

## Goal Capsule

- **Objective:** un utilisateur local de l'hôte de dispatch ne peut plus lire le jeton GitHub du pilote dans les métadonnées de processus, et un opérateur peut coller une sortie `ps`, un `/proc/<pid>/cmdline` ou une trace de dispatch dans un ticket sans y recopier un identifiant.
- **Means:** livrer `GH_TOKEN` au sandbox par un fichier `0600` créé par bwrap lui-même, et non plus par `--setenv` (KTD1).
- **Authority:** les quatre cases à cocher de mika#2039 font foi. Ce plan les instrumente ; il ne les redéfinit pas. En cas de conflit entre ce plan et le ticket, le ticket gagne.
- **Execution profile:** correctif de confinement sur `skills/bundled/_shared/dispatch-lib.sh`, substrat de la boucle autonome. Toute régression casse tous les dispatches — les gardes anti-vacuité sont livrées dans le même PR que le correctif.
- **Stop conditions:** s'arrêter et remonter si (a) le canal secret ne peut pas être construit sans emprunter un descripteur déjà réservé par le dispatch, (b) le prologue d'entrée casse le code de sortie du pilote, ou (c) l'audit des `--setenv` révèle un second secret non anticipé.
- **Tail ownership:** la rotation du PAT est une action opérateur, hors périmètre code. Elle a une ligne de complétion propre dans la Definition of Done — elle ne bloque pas le merge, mais elle bloque la clôture de mika#2039.

---

## Product Contract

### Summary

Retirer la seule valeur secrète des canaux d'observation locaux de la ligne `bwrap` — l'argv et la trace `BASH_XTRACEFD` — puis rendre la fuite structurellement irréproductible par deux gardes qui échouent sur la forme actuelle et passent sur la forme corrigée.

### Problem Frame

`skills/bundled/_shared/dispatch-lib.sh:352-354` construit `setenv_args+=(--setenv "$var" "${!var}")` pour chaque variable de `_PILOT_SANDBOX_ENV_ALLOWLIST` (`:231-233`), qui contient `GH_TOKEN`. La valeur est donc matérialisée dans l'argv de `bwrap` aux deux sites d'invocation (`:369` mode Phase 2b, `:434` mode Phase 2a de repli).

L'argv d'un processus est lisible par tout utilisateur local via `/proc/<pid>/cmdline` — contrairement à `/proc/<pid>/environ`, qui reste réservé au même uid. La protection par `--clearenv` porte sur ce que le sandbox voit ; elle ne protège rien côté hôte. Le jeton est un `github_pat_…` à portée organisation avec accès en écriture aux dépôts, exposé pendant toute la vie du processus, à chaque dispatch.

La conséquence opérationnelle dépasse la lecture directe : toute capture de diagnostic contenant `ps` recopie le secret. C'est arrivé pendant l'investigation de mika#2029.

Un second canal d'observation existe sur le même chemin et doit être fermé en même temps. `dispatch_claude_pilot` active `set -x` avec `BASH_XTRACEFD=9` vers `/tmp/dev-pilot-trace-$$.log` (`:3566-3574`), et `_setup_gh_auth` réactive explicitement la trace en sortie (`:664`). Toute expansion de `$GH_TOKEN` sur ce chemin est donc écrite dans la trace, et la rédaction existante (`:641`) ne filtre que la forme `GH_TOKEN=<valeur>` — la forme `++ printf %s <valeur>` y échappe. Ce fichier est ensuite tronqué et renvoyé dans le callback (`:528-544`). Déplacer le jeton de l'argv vers ce canal ne serait pas une correction.

### Requirements

**Canal du secret**

- R1. `GH_TOKEN` n'apparaît ni dans l'argv, ni dans la trace `BASH_XTRACEFD`, d'aucun processus lancé par `_run_pilot_sandboxed`.
- R2. Le pilote continue de disposer de `GH_TOKEN` dans son environnement à l'intérieur du sandbox, aux deux modes (Phase 2a et Phase 2b).
- R3. Le fichier porteur du secret à l'intérieur du sandbox est en `0600` et monté en lecture seule.
- R4. Les comportements de repli existants sont inchangés : `MIKA_PILOT_SANDBOX=0` et `bwrap` absent invoquent le pilote directement, sans canal secret.
- R5. Le code de sortie du pilote reste le code de sortie de `_run_pilot_sandboxed`.
- R12. Le canal de trace du dispatch reste fonctionnel de bout en bout. Le canal secret n'emprunte aucun descripteur déjà réservé, le fd 9 en particulier.
- R13. Les descripteurs porteurs du secret sont fermés avant le retour de `_run_pilot_sandboxed`, sans altérer le code de sortie exigé par R5.

**Audit et garde**

- R6. Chaque valeur encore passée par `--setenv` est auditée et documentée comme non secrète, à l'endroit du code qui la produit.
- R7. La garde de nom est deny-par-défaut : elle échoue dès que le contenu de `_PILOT_SANDBOX_ENV_ALLOWLIST` diffère d'un ensemble littéral attendu inscrit dans la garde, quelle que soit la forme du nom ajouté.
- R14. La garde de nom couvre les deux producteurs de `--setenv` du fichier, pas seulement l'allowlist. Un `--setenv` littéral portant un nom de forme identifiante échoue où qu'il soit écrit, avec une exception nommée pour le placeholder audité.
- R8. Une garde échoue si l'argv `bwrap` construit contient une valeur de forme identifiante (`github_pat_`, `ghp_`, `gho_`, `ghu_`, `ghs_`, `sk-`, `AKIA`).
- R9. Les deux gardes échouent sur l'état actuel du dépôt et passent sur l'état corrigé (anti-vacuité), et elles s'exécutent toutes les deux en CI.
- R15. Le comportement négatif de la garde de nom est lui-même testé. Un refactor qui la ferait sortir 0 en toutes circonstances échoue.

**Partageabilité du diagnostic**

- R10. Après correction, l'argv `bwrap` reste lisible et informatif : les binds, les namespaces et les variables non secrètes restent visibles. Seule la valeur secrète disparaît.
- R11. Une sonde exécutable sur l'hôte confirme sur un sandbox réellement lancé qu'aucune valeur de forme identifiante n'apparaît dans l'argv du processus `bwrap`.

### Key Decisions

- **Le correctif s'arrête aux canaux d'observation locaux.** Router l'authentification GitHub par injection d'en-tête côté hôte — la forme Q3 déjà utilisée pour Anthropic (`dispatch-lib.sh:297-334`) — retirerait aussi le jeton de `/proc/self/environ` *dans* le sandbox. C'est la suite architecturale cohérente, pas ce ticket : elle touche le proxy d'egress, `git push` et `gh` en même temps. Gouverne R1, R2, R3.

### Scope Boundaries

**Dans le périmètre**

- `skills/bundled/_shared/dispatch-lib.sh` : allowlist, construction des arguments, prologue d'entrée aux deux sites bwrap, bracket xtrace, bloc de commentaire d'en-tête.
- Gardes : un test unitaire, un lint CI, et le test du lint.
- `scripts/canary-pilot-containment` : assertion de partageabilité sur un sandbox réel.

**Déféré à un travail de suivi**

- Injection côté hôte du jeton GitHub par le proxy d'egress (voisinage mika#1991, γ). Le jeton disparaîtrait alors aussi de l'environnement du sandbox et le canal `/run/mika-pilot-secrets` serait retiré, pas empilé.
- Le passage de `$quoted_argv` (`printf '%q'`, quoting bash) à des paramètres positionnels dans le prologue Phase 2b. `%q` produit des formes `$'...'` que `/bin/sh` (dash) n'interprète pas ; le défaut est latent et préexistant.
- Le raccordement à la CI du reste des tests orphelins de `skills/bundled/_shared/tests/`. Ce plan raccorde le sien ; il ne traite pas les six autres.
- L'élargissement du motif de valeurs (R8) au-delà des sept préfixes GitHub/Anthropic/AWS. C'est un denylist par construction, adéquat pour le jeton retiré ici, brittle comme invariant général.

**Hors identité du produit**

- La rotation du PAT exposé. Action opérateur, avec sa ligne de complétion dans la Definition of Done.

### Sources

- `skills/bundled/_shared/dispatch-lib.sh:225-233` — commentaire et contenu de `_PILOT_SANDBOX_ENV_ALLOWLIST`.
- `skills/bundled/_shared/dispatch-lib.sh:352-354` — boucle de construction des `--setenv`.
- `skills/bundled/_shared/dispatch-lib.sh:369-432` et `:434-474` — les deux sites bwrap. `--tmpfs /run` y est à `:390` et `:455`.
- `skills/bundled/_shared/dispatch-lib.sh:277-342` — `net_setenv_args`, second producteur de `--setenv`, dont `ANTHROPIC_API_KEY` factice.
- `skills/bundled/_shared/dispatch-lib.sh:3566-3574` — `TRACE_FILE`, `exec 9>>`, `BASH_XTRACEFD=9`, `set -x`. Le fd 9 est réservé pour toute la durée du dispatch.
- `skills/bundled/_shared/dispatch-lib.sh:647-665` — `_setup_gh_auth` : le bracket `{ set +x; } 2>/dev/null` … `set -x` (mika#903), précédent exact du traitement à appliquer.
- `skills/bundled/_shared/dispatch-lib.sh:528-544` et `:641` — la trace est tronquée dans le callback, et la rédaction ne couvre que la forme `NOM=valeur`.
- `skills/bundled/_shared/dispatch-lib.sh:106-110` — le CA helper bindé sous `/tmp` plutôt que `/etc` : le précédent EROFS de l'audit du 2026-08-05.
- `bwrap --version` sur l'hôte de dispatch : `bubblewrap 0.11.2`. `--file FD DEST`, `--bind-data FD DEST`, `--ro-bind-data FD DEST` et `--perms OCTAL` sont disponibles, et la combinaison retenue a été exercée sur cet hôte (voir A1).
- `skills/bundled/_shared/tests/test_force_push_guard.sh:55-70` — convention de mock par fonction shell déjà en place dans ce dépôt.
- `scripts/verify-egress-no-log.sh:1-40` — forme de référence pour un lint structurel CI.
- `.github/workflows/ci.yml:85` — la CI exécute `make test-dispatch-lib`, qui n'appelle que `skills/bundled/_shared/test-dispatch-lib.sh` (`Makefile:134`). Rien ne parcourt `_shared/tests/`.
- `scripts/canary-pilot-containment:109`, `:118`, `:144`, `:159` — les appels existants à `_run_pilot_sandboxed`, tous bloquants.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Canal du secret : `--perms 0600 --ro-bind-data <fd> <dest>`.** bwrap 0.11.2 lit le contenu depuis un descripteur de fichier et le matérialise dans un fichier bind-monté en lecture seule à l'intérieur du sandbox. Aucun fichier persistant n'est créé sur l'hôte, et rien ne transite par l'argv. Retenu contre `--args FD`, qui déplacerait *toute* la ligne de commande hors de l'argv : R10 exige que le diagnostic reste informatif, et une ligne `bwrap --args 3` ne dit plus rien des binds ni des namespaces. Retenu contre `--file FD DEST`, qui écrit un fichier ordinaire au lieu de créer un point de montage et éviterait la contrainte d'ordre de KTD2, mais laisse la destination inscriptible depuis le sandbox : R3 demande la lecture seule, et le coût de la contrainte d'ordre est un argument à placer au bon endroit, pas une dette. Retenu enfin contre l'abandon de `--clearenv` au profit d'un nettoyage d'environnement en amont : `--clearenv` est l'invariant deny-par-défaut audité (`dispatch-lib.sh:48`) et une boucle `unset` le remplacerait par une garantie plus faible. Gouverne R1, R3, R10.
- KTD2. **Destination sous `/run`, jamais sous `/etc`.** `/run` est monté en tmpfs par le sandbox (`:390`, `:455`), donc un bind imbriqué y réussit ; `/etc` est déjà ro-bindé plus haut et un bind imbriqué y échoue en `EROFS` — le défaut B de l'audit de cohérence du 2026-08-05, déjà rencontré pour le CA helper (`dispatch-lib.sh:106-110`). Les arguments du canal secret doivent donc être développés après `--tmpfs /run`. Gouverne R3.
- KTD3. **Ré-export côté enfant par un prologue générique, piloté par le nom de fichier.** Le prologue parcourt le répertoire de secrets et exporte chaque fichier sous le nom de son basename. Le nom de la variable apparaît donc dans l'argv, mais seulement comme basename de la destination — ce que R1 n'interdit pas, puisque seule la valeur est secrète. Le bénéfice est ailleurs : ajouter un second secret ne demande aucun changement du prologue. Gouverne R2.
- KTD4. **Phase 2a gagne un entrypoint `/bin/sh -c '<prologue>; exec "$@"' <nom> "$@"`.** L'argv d'origine est passé en paramètres positionnels, pas re-cité. Cela évite d'introduire un second appel à `printf '%q'` là où il n'y en avait pas, et `exec` en position finale préserve le code de sortie du pilote. Le site Phase 2b conserve son `$quoted_argv` existant : le changer est un correctif distinct, listé en travail de suivi. Gouverne R2, R5.
- KTD5. **Deux gardes, deny-par-défaut d'abord, motif ensuite.** La garde de nom (`scripts/verify-no-secret-in-setenv.sh`) affirme l'égalité de `_PILOT_SANDBOX_ENV_ALLOWLIST` à un ensemble littéral attendu — toute addition ou tout renommage échoue, quelle que soit la forme du nom. Le filtre par motif (`TOKEN|SECRET|KEY|PASSWORD|PASSWD`, plus `PAT` en correspondance délimitée) est le filet secondaire, appliqué aussi aux `--setenv` littéraux hors allowlist. La garde de valeur (test unitaire à `bwrap` mocké) attrape une régression du mécanisme lui-même, y compris par un chemin qui n'ajoute rien à l'allowlist. Aucune des deux ne subsume l'autre, et les deux tournent en CI. La posture deny-par-défaut est retenue contre le denylist seul parce que `--clearenv` — l'invariant que ce fichier défend déjà — est de cette forme : une garde allow-par-défaut laisserait passer un `SENTRY_DSN` ou un `..._CREDENTIAL`. Gouverne R7, R8, R9, R14, R15.
- KTD6. **Descripteurs alloués à partir de 10, un par secret, jamais le fd 9.** `dispatch_claude_pilot` réserve le fd 9 pour la trace `BASH_XTRACEFD` pendant toute la durée du dispatch (`:3571-3573`) ; le réouvrir en lecture couperait la trace à partir de l'appel au sandbox. Le fd est alloué par un compteur indexé sur la position dans `_PILOT_SANDBOX_SECRET_ALLOWLIST`, et non figé : un second secret réutiliserait sinon un tube déjà drainé et bwrap matérialiserait un fichier vide, sans trace dans l'argv pour le diagnostiquer. Gouverne R12.
- KTD7. **Le site d'expansion du secret est bracketé `{ set +x; } 2>/dev/null` … `set -x`.** `set -x` est actif pendant `_run_pilot_sandboxed` et xtrace développe les substitutions de processus : la trace écrirait `++ printf %s <valeur>`, forme que la rédaction existante (`:641`) ne couvre pas et que le callback renvoie (`:528-544`). Le fichier applique déjà ce bracket exact à `_setup_gh_auth` (`:647-665`, mika#903) ; le nouveau site hérite du même traitement plutôt que d'un traitement inventé. Gouverne R1.

### High-Level Technical Design

Trajet du jeton, avant et après. Le fd 10 est le premier descripteur libre ; le 9 est pris par la trace.

```mermaid
flowchart TB
  subgraph AVANT
    H1[env mika-spirit: GH_TOKEN] -->|--setenv GH_TOKEN valeur| A1[argv bwrap<br/>LISIBLE PAR TOUS]
    H1 -->|set -x actif| T1[trace BASH_XTRACEFD<br/>renvoyee dans le callback]
    A1 --> S1[env sandbox: GH_TOKEN]
    S1 --> P1[claude-pilot / gh]
  end
  subgraph APRES
    H2[env mika-spirit: GH_TOKEN] -->|bracket set +x| B[construction du canal]
    B -->|printf builtin vers fd 10| FD[fd 10 - substitution de processus]
    FD -->|--perms 0600 --ro-bind-data 10 dest| F2["/run/mika-pilot-secrets/GH_TOKEN<br/>0600, ro, dans le sandbox"]
    A2[argv bwrap<br/>ne porte que le chemin] -.->|aucune valeur| F2
    T2[trace BASH_XTRACEFD<br/>intacte, sans valeur] -.-> B
    F2 -->|prologue entrypoint| S2[env sandbox: GH_TOKEN]
    S2 --> P2[claude-pilot / gh]
  end
```

Les deux modes convergent sur le même prologue : Phase 2b l'ajoute en tête du script `/bin/sh -c` existant, Phase 2a en gagne un.

### Assumptions

- A1. Le mécanisme est vérifié, pas supposé, sur l'hôte de dispatch : un fd numérique ouvert par substitution de processus est hérité à travers `exec`, et `--perms 0600 --ro-bind-data <fd> /run/mika-pilot-secrets/GH_TOKEN` produit dans le sandbox un `-rw-------` au contenu exact, sans saut de ligne ajouté, le répertoire parent étant créé par bwrap sous la tmpfs `/run`. Le fd 9 est exclu de ce mécanisme (KTD6). La substitution de processus est retenue plutôt qu'un fichier temporaire hôte pour ne pas écrire le secret sur le disque. U1 conserve le scénario d'héritage en test de non-régression ; le repli, s'il devenait nécessaire sur un autre hôte, est un fichier `mktemp` en `0600` dans un répertoire `0700`, ro-bindé puis supprimé par un `trap`.
- A2. `GH_TOKEN` est la seule valeur secrète des deux producteurs de `--setenv`. L'énumération faite pendant la planification ne donne que HOME, PATH, USER, LOGNAME, SHELL, TERM, LANG, LC_ALL, TMPDIR, HOSTNAME, GH_TOKEN, ANTHROPIC_LOG_FILE, MIKA_LOG_PILOT_TRANSCRIPTS, les URL de proxy, `MIKA_PILOT_CONTAINED`, `NODE_EXTRA_CA_CERTS` et le placeholder littéral `proxy-managed-no-secret`. L'audit de U2 vaut re-vérification ; il n'est pas une formalité.
- A3. Le prologue s'exécute sous `/bin/sh` (dash sur cet hôte). Il n'utilise donc que du POSIX shell — pas de tableaux, pas de `[[`.
- A4. `_PILOT_SANDBOX_ENV_ALLOWLIST` n'a pas de lecteur hors de `dispatch-lib.sh` (`:231` et `:352` seulement), et `test-dispatch-lib.sh` n'affirme rien sur `GH_TOKEN`. La scission de U1 ne porte donc pas de régression inter-fichiers cachée.

### Sequencing

Le test de U3 est écrit et vu en échec avant U1 — c'est l'observation d'anti-vacuité exigée par la Definition of Done, et la reconstruire après coup demande de défaire U1. U1 précède ensuite U2. U3 et U4 ne sont finalisés qu'après U1 et U2, puisque les gardes citent la forme corrigée. U5 dépend de U1 et exige un déploiement. U3 et U4 sont parallélisables une fois U1 posée.

---

## Implementation Units

### U1. Canal du secret : sortir `GH_TOKEN` de l'argv et de la trace

- **Goal:** `GH_TOKEN` atteint le pilote par un fichier `0600` dans le sandbox, aux deux sites bwrap, sans passer par l'argv ni par la trace.
- **Requirements:** R1, R2, R3, R4, R5, R12, R13. Décisions : KTD1, KTD2, KTD3, KTD4, KTD6, KTD7.
- **Files:** `skills/bundled/_shared/dispatch-lib.sh`
- **Approach:**
  - Scinder `_PILOT_SANDBOX_ENV_ALLOWLIST` (`:231-233`) : retirer `GH_TOKEN` et introduire `_PILOT_SANDBOX_SECRET_ALLOWLIST=( GH_TOKEN )` à côté, avec le commentaire qui dit pourquoi la seconde liste ne passe jamais par `--setenv`.
  - Ajouter une constante de chemin sandbox (`/run/mika-pilot-secrets`), dans la forme des constantes `_PILOT_*` existantes (`:100-110`).
  - Construire un tableau `secret_args` en parallèle de `setenv_args` (`:350-354`), à l'intérieur d'un bracket `{ set +x; } 2>/dev/null` … `set -x` (KTD7). Pour chaque variable non vide de la liste secrète : allouer le descripteur suivant à partir de 10 (KTD6), l'ouvrir par substitution de processus sur le `printf` **builtin** — jamais `/usr/bin/printf`, qui recréerait exactement la fuite argv que ce ticket ferme — puis émettre `--perms 0600 --ro-bind-data <fd> <dest>`.
  - Développer `"${secret_args[@]}"` aux deux appels bwrap, après `--tmpfs /run` (KTD2).
  - Après chaque appel bwrap : capturer le code de sortie dans une variable, fermer les descripteurs secrets, puis retourner le code capturé. L'ordre est load-bearing — fermer avant de capturer écraserait `$?` et violerait R5.
  - Définir le prologue POSIX en constante partagée : parcourir le répertoire de secrets, ignorer les entrées non lisibles, exporter chaque fichier sous son basename. La constante est développée par expansion de paramètre dans le script Phase 2b existant, ce que bash ne ré-expanse pas ; l'écrire en clair dans cette chaîne doublement quotée obligerait à échapper chaque `$`.
  - Site Phase 2b (`:369`) : préfixer le script `/bin/sh -c` existant par le prologue, `$quoted_argv` inchangé.
  - Site Phase 2a (`:434`) : remplacer `-- "$@"` par `-- /bin/sh -c '<prologue>; exec "$@"' mika-pilot-sandbox "$@"`.
- **Execution note:** écrire d'abord le test de U3, le voir échouer sur la forme actuelle, puis appliquer ce changement. L'anti-vacuité de R9 est une propriété observée, pas une propriété affirmée.
- **Test Scenarios:**
  - `GH_TOKEN=github_pat_0000000000000000000000000000000000000000` et le proxy d'egress indisponible (Phase 2a) : l'argv capturé ne contient aucune occurrence de `github_pat_`, et contient `--ro-bind-data`.
  - Même entrée avec le proxy d'egress disponible (Phase 2b) : même assertion sur l'argv capturé.
  - Trace : avec `BASH_XTRACEFD` redirigé vers un fichier et `set -x` actif, le fichier de trace ne contient aucune occurrence de la valeur factice après l'appel.
  - Trace intacte : une ligne tracée émise après le retour de `_run_pilot_sandboxed` apparaît bien dans le fichier de trace — le fd 9 n'a pas été détourné.
  - L'argument `--perms 0600` précède immédiatement le `--ro-bind-data` correspondant.
  - La destination `--ro-bind-data` apparaît après `--tmpfs /run` dans l'argv.
  - Deux entrées dans `_PILOT_SANDBOX_SECRET_ALLOWLIST` : deux destinations distinctes, chacune portant sa propre valeur.
  - Fermeture : les descripteurs secrets ne sont plus ouverts après le retour de `_run_pilot_sandboxed`, aux deux modes.
  - `GH_TOKEN` vide ou non défini : aucun `--ro-bind-data` de secret n'est émis, et l'invocation reste valide.
  - `HOME` et `PATH` restent passés par `--setenv` : le mécanisme non secret est intact.
  - `MIKA_PILOT_SANDBOX=0` : invocation directe, aucun argument bwrap construit.
  - `bwrap` absent du PATH : repli en invocation directe, message d'avertissement inchangé.
  - Héritage du fd (exécuté seulement si `bwrap` est installé, et après restauration du vrai binaire) : lancer réellement `bwrap` avec un jeton factice et un entrypoint qui écrit `$GH_TOKEN` sur la sortie standard ; la valeur factice est restituée.
  - Code de sortie : un pilote simulé qui sort en 42 fait sortir `_run_pilot_sandboxed` en 42, aux deux modes.
- **Verification:** `make test-sandbox-secret-argv`, puis `make test-dispatch-lib` pour la non-régression de la suite existante.

### U2. Audit des `--setenv` restants et mise à jour de l'en-tête

- **Goal:** établir et consigner que plus aucune valeur passée par `--setenv` n'est secrète, et que la documentation du fichier décrit le mécanisme réel.
- **Requirements:** R6.
- **Files:** `skills/bundled/_shared/dispatch-lib.sh`
- **Approach:** relire les deux producteurs de `--setenv` — `_PILOT_SANDBOX_ENV_ALLOWLIST` après scission (U1) et `net_setenv_args` (`:277-342`) — et classer chaque entrée. Consigner le résultat en commentaire à l'endroit qui produit les arguments, pas dans un document séparé : le lecteur qui ajoute une variable lit ce commentaire. Noter explicitement que `ANTHROPIC_API_KEY` porte la valeur factice `proxy-managed-no-secret` et que c'est ce qui la rend inoffensive, afin qu'un futur changement vers une vraie clé soit reconnu comme une régression. Mettre à jour le bloc de commentaire d'en-tête, qui décrit aujourd'hui l'allowlist `--clearenv` + `--setenv` (`:48`) et annonce « GH_TOKEN env var re-injected below » (`:68`) — c'est la première chose que lit un futur mainteneur, et elle deviendrait fausse.
- **Test expectation:** aucun — unité de documentation dans le code, sans comportement. Les propriétés qu'elle décrit sont mises sous garde par U3 et U4.
- **Verification:** relecture ; `make verify-bundled-skills` reste vert.

### U3. Garde valeur : test unitaire à `bwrap` mocké, raccordé à la CI

- **Goal:** un test, exécuté en CI, échoue si une valeur de forme identifiante apparaît dans l'argv bwrap construit ou dans la trace.
- **Requirements:** R8, R9. Décision : KTD5.
- **Files:** `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh` (nouveau), `Makefile`, `.github/workflows/ci.yml`
- **Approach:** suivre la convention de `skills/bundled/_shared/tests/test_force_push_guard.sh` : sourcer `dispatch-lib.sh`, redéfinir les dépendances en fonctions shell, compter les assertions, sortir non nul si une échoue. Mocker `bwrap` pour écrire son argv reçu, séparé par des octets nuls, dans un fichier de capture. Mocker `_ensure_pilot_egress_proxy` et `_ensure_pilot_helper` pour sélectionner le mode voulu sans lancer de démon — l'interdiction de démarrer un processus réel vise les démons, pas le scénario d'héritage du fd, qui s'exécute dans une section finale du fichier après `unset -f bwrap` et restauration du vrai binaire. Positionner `HOME` sur un répertoire temporaire pour que les `mkdir -p` et les binds restent confinés. Le motif de rejet est `github_pat_|ghp_|gho_|ghu_|ghs_|sk-|AKIA`. Raccorder par une cible `Makefile` `test-sandbox-secret-argv` et une étape CI dans le même job que `make test-dispatch-lib` (`.github/workflows/ci.yml:85`) — le raccordement des six autres tests orphelins de ce répertoire reste déféré.
- **Test Scenarios:** ceux listés en U1 — ce fichier est leur implémentation.
- **Verification:** `make test-sandbox-secret-argv` sort 0 sur la forme corrigée et non nul si U1 est annulée ; l'étape apparaît verte sur la PR.

### U4. Garde nom : lint deny-par-défaut, son test, et raccordement CI

- **Goal:** un lint CI échoue dès que l'ensemble des variables atteignant `--setenv` change, et son propre comportement négatif est testé.
- **Requirements:** R7, R9, R14, R15. Décision : KTD5.
- **Files:** `scripts/verify-no-secret-in-setenv.sh` (nouveau), `scripts/test-verify-no-secret-in-setenv.sh` (nouveau), `Makefile`, `.github/workflows/ci.yml`
- **Approach:** reprendre la forme de `scripts/verify-egress-no-log.sh` — en-tête qui énonce l'invariant et cite mika#2039, `set -euo pipefail`, résolution du dépôt depuis `$0`, sortie 0 propre / 1 avec message actionnable. Règle primaire : extraire le contenu de `_PILOT_SANDBOX_ENV_ALLOWLIST` et le comparer à un ensemble littéral attendu inscrit dans le lint ; toute différence échoue avec un message qui dit d'auditer l'ajout puis de mettre à jour l'ensemble attendu. Filet secondaire : parcourir tous les `--setenv <NOM>` littéraux du fichier — ce qui couvre `net_setenv_args` — et rejeter tout nom contenant `TOKEN`, `SECRET`, `KEY`, `PASSWORD`, `PASSWD`, ou `PAT` en correspondance délimitée (`(^|_)PAT(_|$)`, pour ne pas rejeter `PATH`), avec une exception nommée et commentée pour `ANTHROPIC_API_KEY` tant qu'il porte le placeholder audité. Échouer aussi si `_PILOT_SANDBOX_SECRET_ALLOWLIST` est absente, ou si une de ses entrées apparaît aussi dans l'allowlist `--setenv`. Le test du lint (`scripts/test-verify-no-secret-in-setenv.sh`) exerce les scénarios ci-dessous contre des copies-fixtures de `dispatch-lib.sh`. Ajouter une cible `Makefile` et exécuter lint et test du lint dans le même job CI que U3.
- **Test Scenarios:**
  - Sur `main` non corrigé : le lint sort non nul et nomme `GH_TOKEN`.
  - Après U1 : le lint sort 0. `PATH` reste dans l'allowlist et ne déclenche rien.
  - Ajout artificiel de `NPM_TOKEN` à l'allowlist `--setenv` : le lint sort non nul.
  - Ajout artificiel de `SENTRY_DSN` à l'allowlist — nom qui ne matche aucun motif : le lint sort non nul par la règle primaire.
  - Ajout artificiel de `--setenv NPM_TOKEN` dans `net_setenv_args` : le lint sort non nul par le filet secondaire.
  - `ANTHROPIC_API_KEY` avec sa valeur placeholder : le lint sort 0. Avec une valeur non placeholder : le lint sort non nul.
  - `_PILOT_SANDBOX_SECRET_ALLOWLIST` supprimée : le lint sort non nul.
  - Le fichier source déplacé ou introuvable : le lint sort non nul avec un message qui dit quoi mettre à jour, jamais 0 silencieux.
- **Verification:** `bash scripts/test-verify-no-secret-in-setenv.sh` ; `bash scripts/verify-no-secret-in-setenv.sh` ; vérifier le passage en CI sur la PR.

### U5. Sonde vivante : partageabilité d'une capture de diagnostic

- **Goal:** confirmer sur un sandbox réellement lancé qu'aucune valeur de forme identifiante n'apparaît dans son argv, et que les binds y restent visibles.
- **Requirements:** R10, R11.
- **Files:** `scripts/canary-pilot-containment`
- **Approach:** tous les appels existants à `_run_pilot_sandboxed` dans le canari sont bloquants (`:109`, `:118`, `:144`, `:159`), et la sonde ne peut pas s'exécuter depuis l'intérieur du sandbox — `--as-pid-1 --unshare-pid --proc /proc` masque le processus `bwrap` hôte. Il faut donc un harnais : remplacer le `GH_TOKEN` de passthrough (`:128`) par une valeur factice de forme reconnaissable, lancer `_run_pilot_sandboxed sh -c 'sleep 10'` en arrière-plan, capturer `$!`, attendre l'apparition du processus `bwrap` par `pgrep -P <pid>` avec une borne d'environ 2 s, lire `/proc/<bwrap_pid>/cmdline` côté hôte, affirmer l'absence du motif identifiant **et** la présence d'arguments `--ro-bind` et `--unshare-`, puis tuer et récupérer le job d'arrière-plan.
- **Test Scenarios:**
  - Canari sur binaires déployés après U1 : la sonde passe, et la sortie montre les binds.
  - Canari sur binaires antérieurs à U1 : la sonde échoue et nomme l'argument fautif.
  - `bwrap` absent : la sonde s'annonce inapplicable et ne passe pas silencieusement.
  - Le processus `bwrap` n'apparaît pas dans la borne d'attente : la sonde échoue en nommant le dépassement, elle ne conclut pas à l'absence de secret.
- **Verification:** `bash scripts/canary-pilot-containment` après `make deploy`, sur l'hôte de dispatch.

---

## Verification Contract

| Commande | Portée | Quand |
|---|---|---|
| `make test-sandbox-secret-argv` | U1, U3 — garde valeur | Avant commit et en CI |
| `bash scripts/verify-no-secret-in-setenv.sh` | U4 — garde nom | Avant commit et en CI |
| `bash scripts/test-verify-no-secret-in-setenv.sh` | U4 — comportement négatif de la garde nom | Avant commit et en CI |
| `make test-dispatch-lib` | Non-régression de la suite dispatch-lib | Avant commit |
| `make verify-bundled-skills` | Invariants structurels des skills bundlés | Avant commit |
| `cargo test` | Non-régression globale | Avant PR |
| `make deploy` puis `bash scripts/canary-pilot-containment` | U5 — sonde vivante | Après merge, sur l'hôte de dispatch |
| `pgrep -af bwrap` pendant un dispatch réel | R1, R10, R11 sur l'état vivant | Après déploiement |
| `grep -c '<motif identifiant>' /tmp/dev-pilot-trace-*.log` | R1 sur le canal de trace | Après déploiement |

**Anti-vacuité — obligatoire.** Avant de déclarer les gardes valides : sur un arbre où U1 est annulée mais U3 et U4 présentes, les deux gardes échouent. Sur l'arbre complet, les deux passent. Consigner les deux observations dans le corps de la PR. Une garde qui n'a jamais été vue échouer n'est pas une garde.

**Vérification sur l'état vivant.** `make deploy` ré-extrait les skills bundlés — une édition de `skills/bundled/` sans redéploiement ne change rien au comportement du prochain dispatch. Les deux dernières lignes du tableau sont exécutées après déploiement, sur un dispatch réel, avant de considérer R1 atteint.

---

## Definition of Done

**Global**

- [ ] Les cinq unités sont livrées, ou les unités non livrées sont nommées avec leur raison.
- [ ] Les deux gardes ont été observées en échec sur la forme non corrigée et en succès sur la forme corrigée, et les deux observations figurent dans le corps de la PR.
- [ ] `make test-dispatch-lib`, `make verify-bundled-skills` et `cargo test` passent.
- [ ] Le job CI qui porte les deux gardes apparaît vert sur la PR.
- [ ] Le corps de la PR rappelle que la rotation du PAT est une action opérateur restant à faire, et ne reproduit aucune valeur de jeton.
- [ ] mika#2039 n'est pas clos tant que l'opérateur n'a pas confirmé sur le ticket que le PAT exposé a été tourné. Le merge de la PR n'est pas bloqué par cette ligne ; la clôture du ticket l'est.
- [ ] Aucun code d'approche abandonnée ne subsiste dans le diff.
- [ ] Les éléments listés en travail de suivi sont déposés en tickets séparés, ou nommés dans le corps de la PR comme non déposés.

**Par unité**

| Unité | Signal de complétion | Moment |
|---|---|---|
| U1 | Aucune occurrence de forme identifiante dans l'argv ni dans la trace, aux deux modes ; trace toujours écrite après le retour ; code de sortie préservé | Avant merge |
| U2 | Chaque `--setenv` restant porte, à son point de production, la raison pour laquelle il n'est pas secret ; le bloc d'en-tête décrit le mécanisme réel | Avant merge |
| U3 | Le test passe sur la forme corrigée, échoue sur la forme actuelle, et s'exécute en CI | Avant merge |
| U4 | Le lint et son test passent, sont appelables par `make`, et s'exécutent en CI | Avant merge |
| U5 | Le canari affirme l'absence de secret et la présence des binds sur un sandbox réel | Après merge et `make deploy` ; résultat reporté en commentaire sur le ticket |

---

## Acceptance criteria

- [ ] Le jeton ne transite plus par l'argv. Les variables d'environnement passées à `bwrap` par `--setenv` sont visibles ; le passer par un canal non lisible (fichier `0600` monté en lecture seule et lu par le processus enfant, ou héritage d'environnement sans matérialisation dans la ligne de commande) le retire de `/proc/<pid>/cmdline`.
- [ ] Auditer les autres `--setenv` de la même ligne pour tout ce qui est secret. `ANTHROPIC_API_KEY` y figure aussi — avec la valeur factice `proxy-managed-no-secret`, donc sans danger, mais le motif montre que la ligne est bien un véhicule à secrets.
- [ ] Une garde : aucun `--setenv` ne porte une valeur ressemblant à un identifiant (`github_pat_`, `ghp_`, `sk-`, `AKIA`…). Test anti-vacuité : la garde doit échouer sur la forme actuelle et passer sur la forme corrigée.
- [ ] Le construire de telle sorte qu'une capture de diagnostic reste partageable. Aujourd'hui, dire « colle-moi la sortie de `ps` » revient à demander un secret.

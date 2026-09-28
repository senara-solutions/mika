---
title: gh utilisable dans le bac à sable pilote (jeton factice GH_TOKEN) - Plan
type: fix
date: 2026-09-28
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# gh utilisable dans le bac à sable pilote (jeton factice GH_TOKEN) - Plan

**Ticket :** senara-solutions/mika#2572 · **Branche :** `fix/2572/loop-substrate-gh-inutilisable-dans-le`

## Goal Capsule

- **Objective :** un pilote dispatché par la boucle peut de nouveau lire les commentaires d'un ticket et ouvrir sa PR avec `gh`, sans qu'aucun identifiant GitHub réel n'entre dans le bac à sable.
- **Means :** poser `GH_TOKEN` à une valeur factice dans le bac à sable, comme `ANTHROPIC_API_KEY` l'est déjà, et laisser le proxy d'egress injecter le vrai jeton (KTD1).
- **Autorité :** le corps de mika#2572 fixe le correctif ; ce plan le rectifie sur deux points mesurés (KTD3, KTD4). En cas de conflit, les R-IDs l'emportent sur le comportement, les KTD sur le mécanisme.
- **Conditions d'arrêt :** (a) si la valeur réelle d'un jeton peut atteindre l'argv ou l'environnement du bac à sable par un chemin quelconque, s'arrêter ; (b) si un test existant ne peut être adapté qu'en retirant une assertion de **valeur**, s'arrêter et remonter.
- **Profil d'exécution :** shell uniquement (`dispatch-lib.sh`, un lint, des suites bash). Aucun Rust, aucune migration.
- **Qui termine :** le pipeline `/mika` jusqu'à la PR. La sonde post-déploiement (R8) est un geste d'opérateur sur l'hôte.

---

## Product Contract

### Summary

Ajouter `--setenv GH_TOKEN "proxy-managed-no-secret"` aux `net_setenv_args` du chemin contenu. Étendre l'exemption unique du lint `verify-no-secret-in-setenv.sh` à une liste de noms, sous la même condition de placeholder vérifiée occurrence par occurrence. Reformuler les assertions mika#2056 qui exigeaient l'absence du **nom** `GH_TOKEN` en assertions sur sa **valeur**. Ajouter une suite qui lance `gh` dans un vrai bwrap, avec un contrôle négatif vu rouge.

### Problem Frame

Depuis mika#2056 (`fa38e0bd`, 2026-08-30), `gh` ne fait plus aucun appel dans le bac à sable. Il s'arrête localement sur `To get started with GitHub CLI, please run: gh auth login`, avant tout contact réseau. `git push` fonctionne, lui : git n'exige aucun identifiant local avant d'envoyer sa requête, que le proxy authentifie.

Le coût est mesuré. Le pilote 8195ada3 porte `You are not logged into any GitHub hosts` à la ligne 5127 de son log (2026-09-27T13:09:45Z). Le dispatch be030903 (mika#2562) a dépensé 41 tours et 6,6 USD pour sortir en halte : les commentaires qui levaient sa précondition ne lui ont jamais été lisibles. Le dispatch 99802f2c porte le même symptôme.

L'injecteur mika#2056 est sain : `~/.mika/pilot-gh-token` est frais, mitmdump tourne avec l'addon GitHub, et l'addon injecte `Authorization`. Ce qui manque est le jeton local factice que `ANTHROPIC_API_KEY` reçoit déjà pour la même raison (commentaire « Claude Code doesn't short-circuit into "Not logged in" » dans `skills/bundled/_shared/dispatch-lib.sh`). mika#2056 a eu raison de retirer le vrai jeton ; il a eu tort de ne rien mettre à la place.

### Requirements

**Comportement dans le bac à sable**

- R1. Sur le chemin contenu (Phase 2b), l'environnement du bac à sable porte `GH_TOKEN` égal à la valeur littérale `proxy-managed-no-secret`, et `gh` ne court-circuite plus localement.
- R2. Aucune valeur réelle de jeton GitHub n'atteint l'argv de bwrap, l'environnement du bac à sable ou son système de fichiers, y compris quand `GH_TOKEN` réel est présent dans l'environnement parent.

**Garde CI**

- R3. `scripts/verify-no-secret-in-setenv.sh` accepte `GH_TOKEN` passé par `--setenv` si et seulement si **chaque** occurrence porte le placeholder, exactement comme pour `ANTHROPIC_API_KEY`.
- R4. L'exemption reste une liste nommée et fermée : un autre nom au format credential (`NPM_TOKEN`, …) reste refusé même s'il porte le placeholder.

**Preuve**

- R5. Une suite lance `gh` dans un vrai bwrap via `_run_pilot_sandboxed` et montre que `gh` part sur le réseau au lieu de court-circuiter (contrôle positif).
- R6. La même suite montre que, sans le setenv `GH_TOKEN`, `gh` court-circuite localement (contrôle négatif vu rouge), pour que le contrôle positif ne soit pas vide.
- R7. Les suites mika#2056 existantes restent vertes, reformulées sur la valeur (KTD3), sans perdre aucune assertion sur la valeur réelle du jeton.

**Déploiement**

- R8. Une sonde post-déploiement est écrite, avec sa halte : au premier dispatch, le résultat d'une commande `gh issue view … --comments` du pilote rend les commentaires, sans l'erreur `gh auth login`.

### Scope Boundaries

- Hors périmètre, repris du ticket : le binaire `mika` dans le bac à sable (`_arch_ask`), et tout client qui lirait un jeton par un autre canal que `GH_TOKEN`.
- Hors périmètre : le chemin non contenu (`MIKA_PILOT_SANDBOX=0`). Il hérite de l'environnement de l'hôte et n'a pas le défaut.
- Hors périmètre : `GH_HOST`, `GH_ENTERPRISE_TOKEN` et les autres variables de `gh`. Aucune mesure ne les demande.

### Success Criteria

- Le premier dispatch après déploiement exécute une commande `gh` authentifiée par le proxy (R8).
- Sur les dispatches postérieurs au déploiement, les appels `gh` des pilotes rendent un résultat authentifié, lu sur le `tool_result` de l'appel et non par un comptage de sous-chaîne sur tout le log.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Le placeholder est la valeur littérale `proxy-managed-no-secret`, partagée avec `ANTHROPIC_API_KEY`.** Une seule valeur à reconnaître pour le lint et pour les lecteurs de log. Elle est posée dans le même bloc `net_setenv_args`, donc uniquement sur le chemin Phase 2b où le proxy existe pour la remplacer. Sûreté vérifiée dans `scripts/mika-pilot-github-auth-addon.py` : `requestheaders` retire tout `Authorization` client et pose le sien sur `api.github.com` et `github.com`. Sur tout autre hôte, `gh` enverrait une valeur qui ne porte aucun secret.
- KTD2. **L'exemption du lint devient un tableau de noms, sous une valeur unique.** `EXEMPT_SETENV_NAME` (scalaire) devient une liste `(ANTHROPIC_API_KEY GH_TOKEN)`. La vérification par occurrence de la règle 2 est appliquée à chaque nom de la liste, sans changement de logique. Un nom hors liste reste soumis à `CRED_NAME_PATTERN`. Rejeté : une valeur par nom, qui ajouterait une dimension sans besoin mesuré.
- KTD3. **Les assertions mika#2056 passent du nom à la valeur.** Trois assertions exigent l'absence du nom `GH_TOKEN` : Test 10b « GH_TOKEN is NOT passed via --setenv » et Test 12(b) « ABSENT from the real sandbox environment » dans `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`, PART C dans `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh`. Les deux suites stubbent un relais qui sert (`stub_serving_egress_relay`), donc traversent Phase 2b et verront le placeholder. L'invariant de sécurité de mika#2056 est « aucun identifiant dans le bac à sable », pas « aucun nom » ; l'assertion de nom est précisément ce que la mesure a démenti. Elles deviennent : chaque `--setenv GH_TOKEN` porte le placeholder, et la valeur vue dans le bac à sable est le placeholder, jamais le jeton parent. Les assertions sur la valeur réelle (absence dans l'argv, pas de fichier `/run/mika-pilot-secrets/GH_TOKEN`, fichier de staging invisible) sont conservées telles quelles. `GH_TOKEN` rejoint `AUDITED_SETENV_NAMES`.
- KTD4. **Le test harness prouve l'absence de court-circuit, pas le succès de l'appel.** Le ticket demande que `gh api user` aboutisse via le proxy dans le harness. Ce succès exige le vrai mitmdump et un vrai jeton : c'est la sonde R8, pas un test CI. Mesuré sur gh 2.92.0, sans réseau : sans `GH_TOKEN` et `HOME` vide, `gh auth token` sort 1 (`no oauth token found`) et `gh api user` imprime `gh auth login` ; avec le placeholder, `gh auth token` imprime le placeholder et `gh api user` tente une connexion (`proxyconnect tcp …`). Ce discriminant est déterministe et suffit à séparer les deux états.
- KTD5. **Suite dédiée plutôt qu'ajout dans une suite existante.** Nouveau fichier `skills/bundled/_shared/tests/test_sandbox_gh_usable.sh`, sur le modèle de `test_sandbox_git_usable.sh` (mika#2141) : vrai bwrap, vrai `_run_pilot_sandboxed`, relais factice qui sert. Les suites mika#2056 prouvent une absence ; celle-ci prouve un usage, et mêler les deux rendrait chaque échec ambigu. Skip propre quand `bwrap` ou `gh` manquent.

### Assumptions

- `gh` est visible à l'intérieur du bac à sable sous le même chemin que sur l'hôte (`/usr/bin` lié en lecture seule). La suite R5 le vérifie et skippe sinon.
- Le relais factice de `lib-fake-egress-relay.sh` ne sert pas de trafic HTTPS réel. Le contrôle positif ne peut donc pas attendre une réponse GitHub valide. Il exige une signature positive de requête émise par `gh api user` (erreur de transport nommant `api.github.com`, ou ligne de statut `HTTP <code>`), en plus de l'absence du message de court-circuit. La signature exacte est relevée sur la première exécution réelle de la suite.

### Sequencing

U1 (lint) avant U2 (setenv) : sinon le lint rougit sur l'arbre intermédiaire. U3 (tests existants) suit U2. U4 (nouvelle suite) suit U2. U5 (docs) en dernier.

---

## Implementation Units

### U1. Exemption du lint étendue à une liste nommée

**Goal :** `verify-no-secret-in-setenv.sh` accepte `GH_TOKEN` au placeholder et refuse toute autre forme.

**Requirements :** R3, R4

**Dependencies :** aucune

**Files :**
- `scripts/verify-no-secret-in-setenv.sh`
- `scripts/test-verify-no-secret-in-setenv.sh`

**Approach :**
1. Remplacer le scalaire par une liste de noms (KTD2), et réécrire le commentaire « Named exception for rule 2 » pour nommer les deux raisons (Anthropic, GitHub) et mika#2572.
2. Dans la règle 2, tester l'appartenance à la liste, puis appliquer le comptage par occurrence existant au nom courant. Le message de violation nomme le nom fautif.

**Patterns to follow :** la vérification par occurrence existante de la règle 2 ; les tests « exemption is conditional on the placeholder » et « exemption is per-occurrence » de la suite.

**Test scenarios :**
- L'arbre vivant, avec U2 appliqué, sort 0.
- Fixture où `--setenv GH_TOKEN` porte `"$GH_TOKEN"` au lieu du placeholder : sortie 1, le message nomme le placeholder.
- Fixture avec une seconde occurrence `--setenv GH_TOKEN "$MIKA_GITHUB_TOKEN"` à côté du placeholder : sortie 1.
- Fixture avec `--setenv NPM_TOKEN "proxy-managed-no-secret"` : sortie 1 (le placeholder n'ouvre pas l'exemption à un nom non listé).
- Les tests `ANTHROPIC_API_KEY` existants restent verts sans modification.

**Verification :** la suite du lint passe, avec au moins les trois nouveaux cas vus rouges sur leur fixture.

### U2. Placeholder GH_TOKEN dans le chemin contenu

**Goal :** le bac à sable Phase 2b porte `GH_TOKEN=proxy-managed-no-secret`.

**Requirements :** R1, R2

**Dependencies :** U1

**Files :**
- `skills/bundled/_shared/dispatch-lib.sh`

**Approach :**
1. Ajouter `--setenv GH_TOKEN "proxy-managed-no-secret"` dans le bloc `net_setenv_args` qui porte déjà `ANTHROPIC_API_KEY`, avec un commentaire qui cite mika#2572, le court-circuit `gh auth login` mesuré, et le remplacement d'en-tête par l'addon (KTD1).
2. Mettre à jour le commentaire d'audit au-dessus de `_PILOT_SANDBOX_ENV_ALLOWLIST` : il liste les noms produits par `net_setenv_args` et dit « That last one is safe ONLY because… ». Il doit nommer les deux placeholders.
3. Corriger la phrase au-dessus de `_PILOT_SANDBOX_SECRET_ALLOWLIST` qui dit « The sandbox no longer holds any GitHub credential in its environment » : l'environnement porte un nom `GH_TOKEN`, sans identifiant.

**Patterns to follow :** l'entrée `ANTHROPIC_API_KEY` du même bloc et son commentaire.

**Test scenarios :** couverts par U1 (forme littérale), U3 (valeur dans l'argv et le bac à sable) et U4 (comportement de `gh`).

**Verification :** `verify-no-secret-in-setenv.sh` sort 0 sur l'arbre vivant.

### U3. Assertions mika#2056 reformulées sur la valeur

**Goal :** les suites mika#2056 restent vertes et continuent de prouver qu'aucun jeton réel n'entre.

**Requirements :** R2, R7

**Dependencies :** U2

**Files :**
- `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`
- `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh`
- `scripts/canary-pilot-containment`

**Approach :**
1. Ajouter `GH_TOKEN` à `AUDITED_SETENV_NAMES`, avec une ligne d'audit dans le commentaire qui le précède (valeur littérale, aucun identifiant).
2. Test 10b : remplacer « GH_TOKEN is NOT passed via --setenv » par « every --setenv GH_TOKEN carries the placeholder, never the parent token ». Garder l'absence de `GH_LEAK_TOKEN` dans l'argv, du fichier `/run/mika-pilot-secrets/GH_TOKEN` et du canal `--ro-bind-data`.
3. Test 12(b) et PART C : l'attendu passe de `<absent>` au placeholder. Ajouter l'assertion explicite que la valeur vue n'est pas `FAKE_TOKEN`.
4. Mettre à jour les en-têtes de suite qui décrivent l'invariant comme « absent », pour dire « aucune valeur de jeton ».
5. Appliquer la même bascule au canary opérateur `scripts/canary-pilot-containment`. Son bloc must-fail signale aujourd'hui `LEAK` dès que `GH_TOKEN` est non vide, donc il crierait à la fuite au premier passage après déploiement. Il doit signaler `LEAK` seulement quand `GH_TOKEN` vaut autre chose que le placeholder, et `ok` sinon. Le contrôle du fichier `/run/mika-pilot-secrets/GH_TOKEN` reste. La ligne de synthèse finale « GH_TOKEN MUST be absent » devient « GH_TOKEN MUST be the placeholder, never a real token ».

**Execution note :** constater d'abord que les trois assertions rougissent après U2 et avant ce unit ; c'est ce qui prouve qu'elles regardaient le chemin Phase 2b.

**Test scenarios :**
- Parent `GH_TOKEN=<jeton factice réaliste>` : l'argv capturé ne contient pas ce jeton, et chaque `--setenv GH_TOKEN` est suivi du placeholder.
- Vrai bwrap, parent `GH_TOKEN=FAKE_TOKEN` : `printf %s "$GH_TOKEN"` dans le bac à sable rend le placeholder, jamais `FAKE_TOKEN`.
- Vrai bwrap : aucun fichier `/run/mika-pilot-secrets/GH_TOKEN`, et `~/.mika/pilot-gh-token` reste invisible (inchangés).
- `MIKA_PILOT_SANDBOX=0` : invocation directe inchangée (Test 11).
- Canary, statiquement : la branche `LEAK` sur `GH_TOKEN` ne se déclenche plus sur le placeholder, et se déclenche toujours sur une valeur de jeton factice réaliste (le canary exporte déjà un leurre `github_pat_…` côté parent).

**Verification :** les deux suites passent ; le nombre d'assertions ne baisse pas.

### U4. Suite `gh` utilisable dans un vrai bac à sable

**Goal :** prouver que `gh` part sur le réseau dans le bac à sable, et qu'il ne le fait pas sans le placeholder.

**Requirements :** R5, R6

**Dependencies :** U2

**Files :**
- `skills/bundled/_shared/tests/test_sandbox_gh_usable.sh` (nouveau)
- `Makefile` (les deux listes qui appellent les suites sandbox voisines, et la ligne `.PHONY` si une cible est ajoutée)

**Approach :**
1. Squelette repris de `test_sandbox_git_usable.sh` : sourcer `dispatch-lib.sh`, relais factice qui sert, `_ensure_pilot_helper` neutralisé, `HOME` temporaire.
2. Contrôle positif dans le bac à sable : `gh auth token` rend le placeholder ; `gh api user` ne produit ni `gh auth login` ni `not logged into any GitHub hosts`, et porte une signature positive de requête (voir Assumptions).
3. Contrôle négatif : lancement contre une copie de `dispatch-lib.sh` privée de la seule ligne `--setenv GH_TOKEN` ; `gh auth token` échoue et `gh api user` imprime `gh auth login`. Il tourne dans un processus `bash` séparé qui source la copie **puis** appelle `stub_serving_egress_relay`. Re-sourcer un `dispatch-lib.sh` après le stub dans le même shell remet `_PILOT_EGRESS_SOCK`, `_PILOT_EGRESS_PROXY_BIN` et le vrai `_ensure_pilot_egress_proxy` en place, et le test lierait alors le socket du relais réel, ce que `lib-fake-egress-relay.sh` interdit.
4. Contrôle de vivacité : le bac à sable exécute `echo alive`, pour qu'un lancement cassé ne passe pas pour un court-circuit.
5. Skip propre quand `bwrap` ou `gh` sont absents de l'hôte, ou quand `gh` n'est pas visible dans le bac à sable.

**Patterns to follow :** `test_sandbox_git_usable.sh` (vrai bwrap, deux moitiés dans le même run) ; les fixtures par mutation de `scripts/test-verify-no-secret-in-setenv.sh`.

**Test scenarios :**
- Covers R5. Placeholder présent : `gh auth token` sort 0 et imprime `proxy-managed-no-secret`.
- Covers R5. Placeholder présent : la sortie de `gh api user` ne contient pas `gh auth login`.
- Covers R5. Placeholder présent : la sortie de `gh api user` porte la signature positive de requête, ce qui écarte un échec précoce sans rapport (configuration `gh` non inscriptible, erreur d'environnement).
- Covers R6. Setenv retiré : `gh auth token` sort non nul avec `no oauth token`.
- Covers R6. Setenv retiré : `gh api user` imprime `gh auth login`.
- Vivacité : `echo alive` rend `alive` dans les deux configurations.
- `gh` absent : la suite affiche un skip et sort 0.

**Verification :** la suite passe sur l'hôte de dev (bwrap et gh présents) ; son contrôle négatif est vu rouge sur la copie mutée de `dispatch-lib.sh`.

### U5. Documentation opérateur et sonde

**Goal :** l'opérateur sait que `GH_TOKEN` du bac à sable est un placeholder, et sait lire la sonde post-déploiement.

**Requirements :** R8

**Dependencies :** U2, U4

**Files :**
- `CLAUDE.md` (section « Optional (gh CLI in agent sessions) » et Signal Q)
- `docs/operator/pilot-egress-relay.md` si le runbook décrit l'injection GitHub ; à vérifier à l'exécution

**Approach :**
1. Dans l'entrée `GH_TOKEN`, ajouter que dans le bac à sable contenu `GH_TOKEN` vaut `proxy-managed-no-secret` depuis mika#2572, et pourquoi.
2. Écrire la sonde R8 en trois gestes : `cat ~/.mika/skills/.manifest-writer` pour vérifier que le sha servi porte le correctif (mika#2340), puis lire le **résultat de l'appel** `gh` du pilote (le `tool_result` d'un `gh issue view … --comments` ou `gh api user`), qui doit être un succès. Un comptage brut de `not logged into any GitHub hosts` ou `gh auth login` sur tout le log pilote n'est **pas** le signal : le ticket et ce plan citent ces chaînes, et un pilote qui les lit pollue le compte. Halte : un 401 **avec** le placeholder signifie que l'addon ne reçoit pas le trafic (`HTTPS_PROXY`, `MITM_FORWARD_HOSTS`) ; lire le journal du proxy avant de toucher au setenv.
3. Signal Q dit qu'une occurrence signifie que le pilote a démarré sans `GH_TOKEN`. Depuis mika#2056 ce n'est plus vrai ; corriger la phrase au passage, sans réécrire l'entrée.

**Test expectation :** none -- documentation seule.

**Verification :** les trois gestes de la sonde sont copiables tels quels.

---

## Verification Contract

- `bash scripts/verify-no-secret-in-setenv.sh` sort 0.
- `bash scripts/test-verify-no-secret-in-setenv.sh` : tous verts ; baseline 25, plus les cas U1.
- `bash skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh` : tous verts ; baseline 35.
- `bash skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` : tous verts ; baseline 9.
- `bash skills/bundled/_shared/tests/test_sandbox_gh_usable.sh` : tous verts, sans skip sur l'hôte de dev.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` : aucune régression.
- `shellcheck` sur les fichiers shell touchés, si le dépôt l'exécute en CI pour eux.

## Definition of Done

- U1 à U5 livrés, chaque suite ci-dessus verte.
- Les trois assertions de U3 ont été vues rouges entre U2 et U3.
- Le contrôle négatif de U4 est vu rouge sur la copie mutée.
- Aucune assertion portant sur la valeur réelle d'un jeton n'a été retirée.
- Aucun code d'essai abandonné ni fixture orpheline dans le diff.
- Le corps de PR porte `Closes #2572`, les rectifications KTD3 et KTD4, et la sonde R8.

## Acceptance criteria

- [ ] Dans le bac à sable contenu, `GH_TOKEN` vaut `proxy-managed-no-secret` et `gh` ne court-circuite plus sur `gh auth login` (R1).
- [ ] Un `GH_TOKEN` réel présent dans l'environnement parent n'atteint ni l'argv de bwrap, ni l'environnement, ni le système de fichiers du bac à sable (R2).
- [ ] `verify-no-secret-in-setenv.sh` accepte `GH_TOKEN` seulement au placeholder, sur chaque occurrence (R3).
- [ ] Un nom credential hors liste qui porte le placeholder reste refusé par le lint (R4).
- [ ] Une suite en vrai bwrap montre `gh` partir sur le réseau avec le placeholder (R5), et court-circuiter sans lui (R6).
- [ ] Les suites mika#2056 passent, reformulées sur la valeur, sans perte d'assertion de valeur (R7).
- [ ] La sonde post-déploiement et sa halte sont écrites dans la documentation opérateur (R8).

---

## Risks & Dependencies

| Risque | Mitigation |
|---|---|
| `gh` envoie le placeholder à un hôte GitHub que l'addon ne couvre pas (`uploads.github.com`, …) | La valeur ne porte aucun secret ; l'appel échoue en 401 sans fuite. Hors périmètre tant qu'aucune commande pilote n'en dépend. |
| Le déploiement n'atteint pas le binaire : `dispatch-lib.sh` est projeté depuis le binaire, pas depuis le checkout (mika#2340) | La sonde R8 commence par `cat ~/.mika/skills/.manifest-writer` pour vérifier le sha servi. |
| L'addon ne reçoit pas le trafic `gh` | Halte R8 : lire le journal du proxy avant de toucher au setenv. |

## Sources

- `skills/bundled/_shared/dispatch-lib.sh` : bloc `net_setenv_args` (Phase 2b), commentaires d'audit de `_PILOT_SANDBOX_ENV_ALLOWLIST` et `_PILOT_SANDBOX_SECRET_ALLOWLIST`.
- `scripts/mika-pilot-github-auth-addon.py` : `requestheaders`, retrait puis pose de `Authorization`.
- `skills/bundled/_shared/tests/lib-fake-egress-relay.sh` : `stub_serving_egress_relay`, qui fait passer les suites sandbox par Phase 2b.
- `skills/bundled/_shared/tests/test_sandbox_git_usable.sh` (mika#2141) : modèle de suite en vrai bwrap.

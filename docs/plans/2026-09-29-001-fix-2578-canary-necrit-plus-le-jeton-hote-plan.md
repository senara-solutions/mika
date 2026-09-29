---
title: Le canari de confinement n'écrit plus le jeton GitHub hôte - Plan
type: fix
date: 2026-09-29
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Le canari de confinement n'écrit plus le jeton GitHub hôte - Plan

**Ticket :** senara-solutions/mika#2578 · **Branche :** `fix/2578/loop-substrate-canary-pilot-containment`

## Goal Capsule

- **Objective :** lancer `scripts/canary-pilot-containment` sur l'hôte ne change plus une
  seule ressource dont un pilote en vol dépend pour s'authentifier à GitHub, et le canari
  le **dit** au lieu de le laisser supposer.
- **Means :** rediriger le chemin de staging du jeton vers un fichier temporaire propre au
  canari (voie 1 du ticket, celle qui ne laisse aucune fenêtre d'écrasement), puis armer
  dans le canari un auto-contrôle qui compare le fichier hôte avant/après et **classe** ce
  qu'il voit.
- **Autorité :** le corps de mika#2578 fixe le correctif et son test d'acceptation. Ce plan
  le rectifie sur trois points mesurés (KTD3, KTD4, KTD5). En cas de conflit, les R-IDs
  l'emportent sur le comportement, les KTD sur le mécanisme.
- **Conditions d'arrêt :** (a) si le correctif exige d'exposer le chemin de staging à
  l'environnement du **service** (et donc de donner à un déploiement un levier sur l'endroit
  où un dispatch réel écrit sa credential GitHub), s'arrêter et remonter ; (b) si l'on ne
  peut rendre l'auto-contrôle non-vide qu'en le faisant observer le chemin qu'il vient de
  rediriger, s'arrêter — ce serait une garde tautologique.
- **Profil d'exécution :** shell uniquement (`scripts/canary-pilot-containment`,
  `skills/bundled/_shared/dispatch-lib.sh`, deux suites bash). Aucun Rust, aucune migration,
  aucune valeur de réglage déplacée.
- **Qui termine :** le pipeline `/mika` jusqu'à la PR. La sonde S1 (rejouer le canari sur
  l'hôte réel et relire le fichier) est un **geste d'opérateur** : le bac à sable de dispatch
  ne monte ni `~/.mika/pilot-gh-token` ni le relais, donc l'implémenteur ne peut pas la
  produire.

---

## Product Contract

### Summary

`_stage_pilot_gh_token` écrit `$GH_TOKEN` dans `$_PILOT_GH_TOKEN_FILE`, dont la valeur est
`$HOME/.mika/pilot-gh-token` — le fichier que l'addon mitmdump de mika#2056 lit côté hôte.
Le canari exporte un leurre `github_pat_0000…canary…` et appelle `_run_pilot_sandboxed`
cinq fois : chaque appel restage le leurre par-dessus le vrai jeton. Le correctif rend la
déclaration de ce chemin **conditionnelle** (`: "${_PILOT_GH_TOKEN_FILE:=…}"`), le canari
pose sa propre valeur avant de sourcer, et un auto-contrôle en sortie atteste que le fichier
hôte n'a pas bougé. Séparément, la ligne `must-fail: env vars` cesse de crier au faux
positif sur le placeholder `ANTHROPIC_API_KEY=proxy-managed-no-secret`, **par valeur et non
par nom**.

### Problem Frame

Mesuré le 2026-09-28 : `~/.mika/pilot-gh-token` réécrit à 20:33:03Z pendant l'exécution du
canari, 49 octets contre 93 à 17:32 (dernier staging par un dispatch réel), préfixe
`github_pat_0…`. Dans le même run, `must-work: allowlisted egress` rend `curl_github=401`.
Le pilote de mika#2565 (`6d61c747`) était en vol.

La chaîne, maillon par maillon :

| # | site | ce qui se passe |
|---|---|---|
| 1 | `canary:203` | `export GH_TOKEN="github_pat_0000000000000000canary0000000000000000"` (49 car.) |
| 2 | `canary:81` | `source dispatch-lib.sh` ⇒ `_PILOT_GH_TOKEN_FILE="$HOME/.mika/pilot-gh-token"` |
| 3 | `canary` ×5 | `_run_pilot_sandboxed` (lignes 90, 179, 188, 249, 316, 396) |
| 4 | `dispatch-lib:1473` | chaque appel exécute `_stage_pilot_gh_token` |
| 5 | `dispatch-lib:1053` | `printf '%s' "$GH_TOKEN" > "$_PILOT_GH_TOKEN_FILE"` ⇒ **le leurre écrase le vrai jeton** |
| 6 | `mika-pilot-github-auth-addon.py:53` | l'addon lit ce fichier, mtime-cachée, et injecte `Authorization` côté hôte |
| 7 | → | tout pilote en vol part en 401 sur `api.github.com` / `github.com`, `git push` compris |

Le maillon décisif est le 2 : le chemin est une **constante dérivée de `$HOME`**, et le
canari est le seul harness qui tourne délibérément sur le vrai `$HOME` de l'hôte.

Le `401` est la conséquence directe du 5, pas un symptôme séparé : `GET https://api.github.com/`
**sans** en-tête `Authorization` rend 200 ; avec un jeton invalide il rend 401. L'addon
injecte donc le leurre, et c'est cette injection qui produit le 401.

### Requirements

**Le fichier hôte n'est plus touché**

- R1. Aucune exécution de `scripts/canary-pilot-containment`, **dans aucun de ses cinq modes**
  (`--show-args`, `--ensure-relay`, `--restart-relay`, `--enter`, mode par défaut), n'écrit,
  ne tronque ni ne supprime `$HOME/.mika/pilot-gh-token`. Après une exécution, le fichier est
  octet pour octet identique à ce qu'il était avant.
- R2. Le canari continue de traverser le **vrai** `_stage_pilot_gh_token` — le staging n'est
  pas neutralisé, il est **redirigé**. Un `_stage_pilot_gh_token` qui cesserait d'écrire
  quoi que ce soit ne serait plus le code que le canari existe pour exercer.
- R3. Le chemin de staging d'un **dispatch réel** reste `$HOME/.mika/pilot-gh-token` et n'est
  pas atteignable depuis l'environnement du service. Une redirection posée par un
  `EnvironmentFile` / ConfigMap serait un levier sur l'endroit où la credential GitHub de
  production atterrit.

**Le canari atteste, il ne suppose pas**

- R4. Le canari capture une empreinte du fichier hôte **avant** son premier appel au bac à
  sable et la recompare en sortie, sur **tous** les chemins de sortie (succès, échec sous
  `set -e`, interruption).
- R5. Le verdict de cet auto-contrôle est **à trois valeurs**, jamais booléen : inchangé /
  changé-en-le-leurre-du-canari (violation) / changé-en-autre-chose (non concluant — un
  dispatch concurrent a restagé, et un jeton d'installation App tourne à l'heure). Confondre
  les deux derniers produirait un faux positif à chaque rotation.
- R6. L'auto-contrôle observe le chemin **hôte par défaut**, explicitement, jamais
  `$_PILOT_GH_TOKEN_FILE` — observer la redirection rendrait la garde tautologiquement verte.
- R7. Aucune valeur de jeton n'entre dans une variable shell, un argv ou une trace `set -x`
  du fait de cet auto-contrôle.
- R8. Une violation confirmée (R5, cas 2) rend le canari non nul en sortie. Un cas non
  concluant préserve le statut d'origine.
- R9. Le canari nomme, avant de lancer le bac à sable, l'état du fichier hôte dont dépend la
  ligne `curl_github` (présent/absent, âge), pour qu'un `401` soit attribuable sans
  archéologie.

**La ligne `must-fail: env vars` redevient lisible**

- R10. `ANTHROPIC_API_KEY=proxy-managed-no-secret` — le placeholder légitime posé par
  `--setenv` (mika#2039, mika#2572) — ne déclenche plus `ENV_LEAK`.
- R11. L'exclusion porte sur la **valeur exacte**, jamais sur le nom : un
  `ANTHROPIC_API_KEY` portant toute autre valeur déclenche toujours `ENV_LEAK`. Exclure le
  nom rendrait le contrôle aveugle à la fuite qu'il existe pour voir.
- R12. La ligne de violation **nomme** les variables fautives (noms seuls, jamais de
  valeurs). « some cred-shaped var survived » est précisément ce qui a rendu la ligne
  inexploitable.

**Preuve**

- R13. Un test structurel refuse qu'un site futur réintroduise une écriture du chemin hôte
  par défaut depuis le canari, et refuse que l'auto-contrôle observe la redirection (R6).
- R14. Un test comportemental montre que, **sans** la redirection, `_stage_pilot_gh_token`
  écrit bien le chemin par défaut (contrôle négatif vu rouge), et qu'**avec** elle il
  n'écrit que la redirection.
- R15. Chaque garde livrée porte son anti-vacuité : un scan dont le chemin pourrit ou dont
  le prédicat se resserre se lit exactement comme un arbre propre (classe mika#2205).

### Scope Boundaries

- **Hors périmètre, nommé — l'héritage d'environnement du daemon mitmdump.**
  `_ensure_pilot_helper` (`dispatch-lib.sh:673`) lance `nohup "$_PILOT_HELPER_BIN" … &`
  **sans** `env -i` : le daemon hérite de l'environnement complet de son lanceur, leurre
  `GH_TOKEN` compris. C'est un second canal de contamination, et il est **plus long-vivant**
  que le fichier (le daemon survit à tous les dispatches suivants). Il est très largement
  **masqué** : l'addon **préfère le fichier** à son env, et après ce correctif un dispatch
  réel restage le vrai jeton à chaque spawn, donc le fichier gagne. La fenêtre résiduelle est
  « daemon relancé par le canari **et** fichier absent ». La refermer demande de retirer au
  daemon son repli d'environnement, qui est un repli **délibéré** de production
  (`dispatch-lib.sh:1054` : « falls back to the mitmdump process env ») : arbitrage de canal,
  pas effet de bord. **Ticket de suivi**, précondition : une mesure montrant un dispatch
  authentifié par le repli d'env plutôt que par le fichier. U3 **borne** ce canal sans le
  fermer (KTD6) et le dit.
- **Hors périmètre, nommé — `install -Dm755 "$SCRIPT" "$INSTALLED_PROXY"` (`canary:75`).**
  Le canari écrase `~/.local/bin/mika-pilot-egress-proxy` par la version de **sa branche**, et
  le binaire y reste pour chaque dispatch suivant. Même classe (une ressource hôte dont un
  pilote en vol dépend), et le commentaire sur place — « Safe: same path production uses;
  overwrite is idempotent » — n'est vrai que quand la branche vaut `main`. Ce n'est pas
  repliable ici : lancer le bac à sable **de la branche** est la raison d'être du canari
  (« real spawn under branch shape »), donc le corriger demande de trancher où le canari
  installe sa copie, ce que le ticket ne pose pas. **Ticket de suivi.**
- **Hors périmètre** — la vérification `must-fail: /proc/1/environ` (`canary:426`). Son motif
  (`^(ATLASSIAN|AWS_SECRET|MIKA_ANTHROPIC|MIKA_INTERNAL_TOKEN)=`) ne contient **pas**
  `ANTHROPIC_API_KEY`, donc il n'a pas le faux positif de R10. Les deux motifs diffèrent
  délibérément et ne doivent **pas** être « harmonisés » : ils visent deux populations
  distinctes. Une ligne de commentaire le dira, aucun changement de comportement.
- **Hors périmètre** — le fichier par dispatch (`pilot-gh-token.<task-id>`) que
  `docs/solutions/cross-repo-patterns/pilot-concurrency-shared-resources-2026-09-03.md` § 6
  nomme déjà comme « changement nommé » pour N>1. Ce plan ne touche pas la concurrence
  inter-dispatch : il retire **un** écrivain illégitime, il ne re-conçoit pas le canal.
- **Hors périmètre** — les autres écritures hôtes du canari : la génération de la CA
  mitmproxy, les écritures sous `$_PILOT_LOG_DIR`, `/tmp/pilot-transcripts.jsonl`, la
  `refs/heads/canary-victim/*` (déjà nettoyée par trap). Aucune n'est une credential dont un
  pilote dépend.

### Success Criteria

- Une exécution du canari sur l'hôte, pendant qu'un pilote est en vol, laisse le pilote
  authentifié : `~/.mika/pilot-gh-token` inchangé, `curl_github` non-401 (R1, sonde S1).
- Le canari imprime une ligne d'auto-contrôle explicite sur chacun de ses chemins de sortie
  (R4), et cette ligne est la réponse directe à « le canari a-t-il abîmé quelque chose ? ».
- La ligne `must-fail: env vars` rend `ok:` sur un run sain et nomme le fautif sinon (R10,
  R12).

---

## Planning Contract

### Key Technical Decisions

- **KTD1. La redirection passe par la variable INTERNE `_PILOT_GH_TOKEN_FILE`, jamais par une
  variable d'environnement opérateur.** La déclaration `dispatch-lib.sh:593` devient
  `: "${_PILOT_GH_TOKEN_FILE:=$HOME/.mika/pilot-gh-token}"` — même valeur par défaut, même
  ligne, mais un harness qui a déjà posé la variable la conserve. **C'est le choix qui satisfait
  R3 structurellement** : un nom préfixé `_` n'est pas une variable opérateur, il n'est ni dans
  `SANDBOX_ENV_CORE_ALLOWLIST` ni dans `PILOT_DISPATCH_ENV`
  (`crates/mika-agent/src/skills/executor.rs:345`, deux éléments : `PILOT_MAX_TURNS`,
  `PILOT_LOG_DIR`), donc il **ne peut pas** atteindre le child d'un dispatch réel : le child
  est bâti par `sandboxed_pilot_env` (`env_clear()` + allowlist positive) et seul
  `inject_pilot_dispatch_env` relaie des noms nus. *Rejeté :* un `PILOT_GH_TOKEN_FILE` nu, sur
  le modèle de `PILOT_LOG_DIR`. Il créerait exactement le levier que R3 refuse, et ferait entrer
  un nom dans la population de
  `mika2508_every_operator_var_read_by_dispatch_lib_reaches_the_child_or_is_named`, dont la
  résolution doctrinale est « relayez ou nommez l'inertie » — or aucune des deux ne décrit un
  knob de harness.
- **KTD2. `:=` plutôt qu'une réaffectation après `source`, parce que le canari a un
  sous-processus.** Le mode `--show-args` (`canary:87-92`) exécute `bash -x -c "source '$DISPATCH_LIB'; _run_pilot_sandboxed /bin/true"`
  — un **bash neuf** qui re-source. Une affectation posée dans le parent *après* le `source`
  n'atteindrait pas ce child, et ce mode appelle bel et bien le bac à sable, donc le staging.
  Avec `:=` et un `export` dans le parent **avant** le `source`, la valeur traverse par
  héritage d'environnement et survit au re-source : **un seul site de pose dans le canari, tous
  les modes couverts**. La valeur est un chemin, pas un secret, donc l'exporter ne coûte rien
  (et `--clearenv` la retire du bac à sable de toute façon).
- **KTD3. Rectification du ticket — le canari n'est pas le seul harness à écrire ce chemin, il
  est le seul à le faire sur le vrai `$HOME`.** Les cinq suites qui sourcent `dispatch-lib.sh`
  et appellent le bac à sable (`test-pilot-github-token-not-in-sandbox.sh`,
  `test_sandbox_no_secret_in_argv.sh`, `test_sandbox_git_usable.sh`,
  `test_sandbox_gh_usable.sh`, `test_sandbox_log_dir_bound.sh`) écrivent toutes le jeton — mais
  toutes font `export HOME="$TMPROOT/home"` **avant** de sourcer, donc le chemin résout sous un
  temporaire. Elles sont saines, et ce plan n'y touche pas. Le canari ne peut pas employer ce
  remède : il a besoin du vrai `$HOME` pour le relais, la CA `~/.mitmproxy/` et le binaire
  installé sous `~/.local/bin`. D'où une redirection **ciblée** plutôt qu'un `HOME` détourné.
- **KTD4. Rectification du ticket — `curl_github=200` ne dépend plus du canari, et c'est
  voulu.** Après le correctif, l'addon injecte ce que le **dernier dispatch réel** a staché.
  C'est précisément ce que le commentaire du contrôle revendique déjà (« An authenticated 200
  here therefore also proves the host-side injection path is intact end-to-end »). Corollaire à
  assumer : sur un hôte qui n'a jamais dispatché, le fichier est absent, l'addon retombe sur son
  env, et `curl_github` n'est plus prédictible. R9 existe pour cela — le canari **dit** l'état
  dont il dépend, au lieu de laisser un 401 illisible.
- **KTD5. Rectification du ticket — l'auto-contrôle doit être à trois valeurs, pas un `cmp`.**
  Le test d'acceptation du ticket (« octet pour octet identique ») est juste comme *exigence* et
  faux comme *prédicat* : un dispatch qui spawne pendant le canari restage légitimement, et un
  jeton d'installation GitHub App tourne à l'heure, donc le contenu peut changer sans faute. Le
  discriminant est l'empreinte du **leurre du canari**, qui est une constante connue du script.
  Sans ce troisième cas, la garde crierait à la violation sur une rotation — et une garde qui
  crie à tort finit muselée.
- **KTD6. U3 borne le canal daemon, il ne le ferme pas — et l'ordre du réchauffement est le
  seul levier gratuit.** Déplacer `_ensure_pilot_helper` / `_ensure_pilot_egress_proxy`
  **au-dessus** des exports de leurres (`canary:194-203`) fait que, sur un hôte au daemon
  éteint, le daemon démarre sans le leurre dans son environnement. Les appels ultérieurs sont
  idempotents (sonde de vivacité en tête) donc ne le relancent pas. La fenêtre résiduelle est
  « le daemon meurt en cours de canari ». Deux lignes déplacées, aucun comportement changé ;
  la fermeture complète reste le ticket de suivi ci-dessus.
- **KTD7. Un seul `trap EXIT`, composé dans une fonction.** Le canari installe aujourd'hui son
  trap **conditionnellement** (`canary:385`, seulement si la ref victime a pu être créée). Un
  second `trap … EXIT` naïf **remplacerait** le premier et laisserait une
  `refs/heads/canary-victim/*` pendante dans le dépôt de l'opérateur. Le correctif définit
  `_canary_on_exit()` — auto-contrôle du jeton, nettoyage de la ref victime (conditionné à
  `$CANARY_VICTIM_REF` non vide, que le bloc existant pose déjà), nettoyage du temporaire — et
  l'installe **une fois**, tôt, inconditionnellement.
- **KTD8. L'empreinte se prend par `sha256sum` sur **stdin**, jamais sur un chemin en argument
  et jamais dans une variable de contenu.** `sha256sum < "$f"` n'imprime pas le nom de fichier
  et ne met aucune valeur de jeton dans un argv ni dans la trace `set -x` (discipline
  mika#2039, R7). Un `cmp` contre une copie sauvegardée est **rejeté** : il exigerait d'écrire
  une copie du vrai jeton ailleurs sur le disque, ce qui ajoute une surface là où l'on en retire.
- **KTD9. L'exclusion du placeholder est un `grep -vxF` sur la ligne entière, et elle garde le
  nom dans le motif.** `ANTHROPIC_API_KEY` reste dans `^(ATLASSIAN|AWS_SECRET|NODE_AUTH|ANTHROPIC_API_KEY)=`
  et c'est ce qui rend le contrôle non-vide : le `--setenv` du bac à sable écrase toujours la
  valeur parente, donc la seule fuite encore possible sur ce nom est *qu'on change la ligne
  `--setenv` pour y passer une vraie clé* — et c'est très exactement ce que l'exclusion par
  valeur continue d'attraper, là où une exclusion par nom la rendrait invisible. Le placeholder
  est déjà une constante partagée avec `GH_TOKEN` (`dispatch-lib.sh:1558`, `:1567`) ; le canari
  en porte déjà le littéral (`canary:461`).

### Assumptions

- `sha256sum` est présent sur l'hôte de dispatch. À vérifier à l'exécution ; repli nommé si
  absent : `cksum` (suffisant — on compare deux empreintes du même fichier, pas une résistance
  aux collisions adverses). Un outil manquant doit rendre « non concluant », **jamais** « ok ».
- `_PILOT_GH_TOKEN_FILE` n'est lu nulle part ailleurs que dans `_stage_pilot_gh_token`
  (vérifié : `dispatch-lib.sh:1052-1054`, trois occurrences, toutes dans la fonction). Le
  passage à `:=` n'a donc pas d'autre lecteur à considérer.
- `test-pilot-github-token-not-in-sandbox.sh` exporte `HOME` avant de sourcer et lit
  `staged_file="$HOME/.mika/pilot-gh-token"` en dur (ligne 146) : la bascule en `:=` la laisse
  verte sans modification, **à condition** que la suite ne pose pas déjà la variable. À
  constater avant U5.

### Sequencing

U1 (dispatch-lib) avant U2 (redirection côté canari) : sans le `:=`, la pose du canari est
écrasée au `source` et la redirection est inerte. U3 (auto-contrôle) après U2, pour que son
régime attendu soit « inchangé ». U4 (ENV_LEAK) est indépendante. U5 (gardes) après U1–U4.
U6 (docs) en dernier.

---

## Implementation Units

### U1. Le chemin de staging est surchargeable par un harness

**Goal :** `_PILOT_GH_TOKEN_FILE` posée avant le `source` survit ; le défaut ne change pas.

**Requirements :** R2, R3

**Dependencies :** aucune

**Files :**
- `skills/bundled/_shared/dispatch-lib.sh`

**Approach :**
1. Remplacer `_PILOT_GH_TOKEN_FILE="$HOME/.mika/pilot-gh-token"` (ligne 593) par
   `: "${_PILOT_GH_TOKEN_FILE:=$HOME/.mika/pilot-gh-token}"`.
2. Étendre le bloc de commentaire qui précède (lignes 590-592, « host-only file … NEVER bound
   into the sandbox … 0600 ») de trois phrases : le défaut est inchangé ; un **harness** qui
   pose la variable avant de sourcer la conserve, ce qui existe pour que le canari
   (`scripts/canary-pilot-containment`) n'écrase pas la credential de l'hôte (mika#2578) ; le
   nom est **délibérément** préfixé `_` et n'est **pas** un knob opérateur — le relayer au
   child de dispatch donnerait à l'environnement du service un levier sur l'endroit où un
   dispatch réel écrit sa credential GitHub (KTD1).

**Patterns to follow :** la forme `: "${VAR:=défaut}"` est le repli idiomatique le moins
intrusif ici ; contrairement à `_pilot_log_dir` / `_pilot_max_turns` (mika#2165, mika#2496) la
variable n'a **qu'un seul lecteur**, déjà à l'intérieur du bracket `set +x` de
`_stage_pilot_gh_token`, donc ni résolveur-fonction ni garde de co-location ne sont justifiés.

**Test scenarios :**
- Sans pose préalable : `_stage_pilot_gh_token` écrit `$HOME/.mika/pilot-gh-token` (inchangé).
- Avec pose préalable : écrit le chemin posé, et **ne crée pas** le chemin par défaut.
- Couvert par U5/D4.

**Verification :** `bash skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh`
reste vert **sans modification** (Assumptions) ; `bash skills/bundled/_shared/test-dispatch-lib.sh`
sans régression ; `cargo test -p mika-agent mika2508` sans régression (KTD1 prédit qu'un nom
préfixé `_` n'entre pas dans la population — **à constater**, et si le test rouge malgré tout,
appliquer le repli de KTD2 : pose côté canari **après** le `source`, à deux sites, `dispatch-lib.sh`
inchangé hors commentaire).

### U2. Le canari redirige son staging

**Goal :** aucun mode du canari n'écrit le chemin hôte par défaut.

**Requirements :** R1, R2

**Dependencies :** U1

**Files :**
- `scripts/canary-pilot-containment`

**Approach :**
1. Juste **avant** `source "$DISPATCH_LIB"` (ligne 81) : créer un répertoire temporaire
   (`mktemp -d`, mode 0700) et `export _PILOT_GH_TOKEN_FILE="$dir/pilot-gh-token"`.
   L'emplacement — avant le `source`, donc avant tout branchement de mode — est ce qui couvre
   `--show-args`, `--ensure-relay`, `--restart-relay`, `--enter` et le mode par défaut d'un
   seul geste (R1). Le commentaire nomme mika#2578 et la raison : le leurre exporté plus bas ne
   doit jamais atteindre la credential hôte.
2. Le mode `--show-args` re-source dans un `bash -c` : vérifier que la valeur exportée y
   traverse bien (héritage d'environnement + `:=`) et **ne pas** dupliquer la pose dans le
   heredoc. Une seconde pose y serait un second site à maintenir ; l'export est ce qui la rend
   inutile (KTD2).
3. Amender le commentaire du bloc d'export du leurre (lignes 197-203) : le leurre est
   `github_pat_0…` **et** il est désormais confiné au temporaire côté staging.

**Patterns to follow :** l'`export WORKTREE_DIR="$REPO_ROOT"` juste au-dessus du `source`, qui
est déjà la manière dont ce script pose l'état que `dispatch-lib.sh` lira.

**Test scenarios :**
- Couvert par U5/D3 (structurel : la pose précède le `source` et le premier appel au bac à
  sable) et par la sonde S1 (comportemental, sur l'hôte).

**Verification :** `bash -n scripts/canary-pilot-containment` ; `shellcheck` si le dépôt
l'exécute pour ce fichier.

### U3. Le canari atteste, et borne le canal daemon

**Goal :** une ligne de verdict à trois valeurs sur tous les chemins de sortie, et le daemon
mitmdump n'hérite plus du leurre quand c'est le canari qui le démarre.

**Requirements :** R4, R5, R6, R7, R8, R9 ; KTD6

**Dependencies :** U2

**Files :**
- `scripts/canary-pilot-containment`

**Approach :**
1. **Empreinte de référence**, prise juste après la pose de U2 et **avant** tout appel au bac à
   sable : `_CANARY_HOST_TOKEN_PATH="$HOME/.mika/pilot-gh-token"` — **littéral, jamais
   `$_PILOT_GH_TOKEN_FILE`** (R6, et c'est l'assertion structurelle de U5/D3) — puis
   `_CANARY_HOST_TOKEN_SHA_BEFORE=$(sha256sum < "$path" 2>/dev/null | cut -d' ' -f1 || true)`.
   Une empreinte vide signifie « absent » et n'est pas une erreur.
2. **Ligne d'attribution (R9)**, imprimée dans l'en-tête à côté de `proxy binary:` : présence
   du fichier hôte, sa taille et son mtime (`stat -c '%s %y'`), avec une phrase disant que
   `curl_github` dépend de ce que le **dernier dispatch réel** y a staché (KTD4). Jamais le
   contenu.
3. **`_canary_on_exit()`** (KTD7), installée **une fois** et tôt, `trap _canary_on_exit EXIT` :
   - recalcule l'empreinte, compare, et classe en trois cas (R5) :
     `ok:   host_gh_token_untouched` /
     `VIOLATION: le canari a écrasé ~/.mika/pilot-gh-token par son leurre (mika#2578 a régressé)` /
     `INCONCLUSIVE: ~/.mika/pilot-gh-token a changé sans porter le leurre — un dispatch concurrent a probablement restagé (rotation App)` ;
   - le discriminant du cas 2 est l'empreinte du littéral du leurre, calculée dans le script
     (`printf '%s' "$leurre" | sha256sum`), donc aucune constante à maintenir à la main ;
   - `sha256sum` indisponible ⇒ `INCONCLUSIVE`, jamais `ok:` (R15, classe mika#2205) ;
   - nettoie la ref victime **si** `$CANARY_VICTIM_REF` est non vide (reprend verbatim la
     commande de la ligne 385) et retire le trap conditionnel existant ;
   - nettoie le temporaire de U2 ;
   - sur le cas 2 uniquement, force un statut de sortie non nul (R8) ; sinon préserve le statut
     entrant (`local rc=$?` en première instruction de la fonction).
4. **KTD6** : déplacer les deux réchauffements idempotents (`_ensure_pilot_helper`,
   `_ensure_pilot_egress_proxy`) **au-dessus** du bloc d'export des leurres, en gardant
   l'appel existant de PART 0 (idempotent, il ne relancera rien). Commentaire nommant ce que
   ça borne et ce que ça ne ferme pas (le suivi « héritage d'env du daemon »).
5. Ajouter la ligne de verdict à la synthèse finale (voisinage de la ligne 640) et à l'en-tête
   « WHAT COHERENCE VERIFIES » (lignes ~30-45).

**Patterns to follow :** le trap conditionnel existant (ligne 385) pour la commande de
nettoyage ; la discipline `|| true` du `pgrep` de PART 0 (lignes ~253-258), qui documente
pourquoi un statut non nul doit être neutralisé sous `set -euo pipefail` — l'auto-contrôle a la
même contrainte sur chacune de ses commandes.

**Test scenarios :**
- Fichier hôte inchangé ⇒ `ok:   host_gh_token_untouched`, statut préservé.
- Fichier hôte remplacé par le leurre ⇒ `VIOLATION`, statut non nul.
- Fichier hôte remplacé par autre chose ⇒ `INCONCLUSIVE`, statut préservé.
- Fichier hôte absent avant **et** après ⇒ `ok:`.
- Fichier hôte absent avant, présent après ⇒ classé par le discriminant du leurre.
- Sortie en échec (`set -e` déclenché dans PART 2) ⇒ la ligne de verdict est imprimée quand
  même, et la ref victime est nettoyée.
- Les trois premiers cas sont **pilotables** en pointant le chemin observé vers un temporaire
  dans une copie de travail sous `.pilot-scratch/` — voir la note d'exécution de U5.

**Verification :** `bash -n` ; les six scénarios exercés à la main sur une copie de travail,
les cas 2 et 3 **vus produire des verdicts différents** (sans quoi le troisième cas est
décoratif).

### U4. `must-fail: env vars` cesse de crier au faux positif

**Goal :** la ligne rend `ok:` sur un run sain et nomme le fautif sinon.

**Requirements :** R10, R11, R12

**Dependencies :** aucune

**Files :**
- `scripts/canary-pilot-containment`

**Approach :**
1. Remplacer le `grep -E … >/dev/null && ENV_LEAK || ok` (lignes 420-422) par un pipeline qui
   capture les survivants, retire la ligne **exacte** `ANTHROPIC_API_KEY=proxy-managed-no-secret`
   (`grep -vxF`), puis n'imprime que les **noms** (`cut -d= -f1`, R12) quand il en reste. Le
   nom `ANTHROPIC_API_KEY` **reste** dans le motif (KTD9).
2. Le commentaire sur place dit les deux moitiés : pourquoi le placeholder est exclu (il est
   posé par `--setenv`, mika#2039/mika#2572, et porte la même valeur littérale que `GH_TOKEN`
   au-dessous), et pourquoi l'exclusion est **par valeur** — exclure le nom rendrait le
   contrôle aveugle à un `--setenv ANTHROPIC_API_KEY "$MIKA_ANTHROPIC_API_KEY"`, qui est la
   seule fuite encore possible sur ce nom.
3. Une ligne de commentaire au-dessus du contrôle `/proc/1/environ` voisin (ligne 426) disant
   que son motif diffère **délibérément** et qu'il n'a pas ce faux positif (Scope Boundaries) —
   pour qu'un futur éditeur ne les « harmonise » pas.
4. Sous `set -euo pipefail`, un pipeline dont un `grep` ne matche rien sort non nul : chaque
   étage doit être neutralisé (`|| true`) ou la structure choisie pour ne pas dépendre du
   statut. C'est la contrainte que le commentaire de PART 0 documente déjà pour `pgrep`.

**Patterns to follow :** le `case "${GH_TOKEN-__unset__}"` des lignes 460-464, qui teste déjà le
même placeholder **par valeur** et sépare trois issues nommées — c'est la forme dont U4 est le
pendant pour `ANTHROPIC_API_KEY`.

**Test scenarios :**
- Bac à sable nominal ⇒ `ok:   env_cleared`.
- `ANTHROPIC_API_KEY` portant autre chose que le placeholder ⇒ `ENV_LEAK` nommant
  `ANTHROPIC_API_KEY` (**contrôle négatif, à voir rouge** en mutant temporairement la ligne
  `--setenv` d'une copie de `dispatch-lib.sh` sous `.pilot-scratch/`).
- `ATLASSIAN_API_TOKEN` survivant ⇒ `ENV_LEAK` le nommant.
- Aucune valeur n'apparaît dans la sortie, dans aucun cas.

**Verification :** `bash -n` ; le contrôle négatif vu rouge sur la copie mutée.

### U5. Gardes

**Goal :** la régression est refusée par un test, pas par la mémoire d'un relecteur.

**Requirements :** R13, R14, R15

**Dependencies :** U1, U2, U3, U4

**Files :**
- `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` (nouvelles PART D et
  extension de PART B)
- `skills/bundled/_shared/test-dispatch-lib.sh`

**Approach :**

1. **D4 — comportemental, dans PART B** (`test-pilot-github-token-not-in-sandbox.sh`, qui
   exerce déjà `_stage_pilot_gh_token` sous un `HOME` détourné) :
   - **contrôle négatif du défaut, vu rouge sur l'arbre d'avant U1** : sans redirection,
     `_stage_pilot_gh_token` écrit `$HOME/.mika/pilot-gh-token` (assertion déjà présente,
     conservée) ;
   - avec `_PILOT_GH_TOKEN_FILE` posée vers un autre temporaire : le contenu atterrit **là**, en
     0600, et `$HOME/.mika/pilot-gh-token` **n'existe pas** (assertion d'absence — c'est elle
     qui atteste la non-écriture, R14) ;
   - anti-vacuité : la variable est restaurée entre les deux moitiés, et la seconde échoue si
     le temporaire de redirection est vide.
2. **D3 — structurel, nouvelle PART D**, scan de source sur `scripts/canary-pilot-containment` :
   - anti-vacuité **d'abord** : le fichier existe, pèse plus de 15 000 octets, et contient au
     moins un `_run_pilot_sandboxed` (un scan dont le chemin pourrit se lit comme un arbre
     propre, R15) ;
   - la pose `_PILOT_GH_TOKEN_FILE=` apparaît **avant** la ligne `source "$DISPATCH_LIB"`, et
     celle-ci avant le premier `_run_pilot_sandboxed` (comparaison de numéros de ligne) ;
   - l'auto-contrôle observe un littéral `\.mika/pilot-gh-token` et **jamais**
     `$_PILOT_GH_TOKEN_FILE` dans le corps de `_canary_on_exit` (R6 — la garde tautologique
     refusée) ;
   - aucune autre ligne non-commentaire du canari ne redirige ni n'écrit le chemin hôte.
   - **Allowlist livrée vide**, pinnée vide par une assertion sœur : quand ce scan tire, on
     redirige le nouveau site, on ne l'allowliste pas (doctrine mika#2201).
3. **D2 — structurel, dans `test-dispatch-lib.sh`** : `_PILOT_GH_TOKEN_FILE` a **exactement un**
   site de déclaration dans `dispatch-lib.sh`, et ce site est la forme conditionnelle (`:=`), de
   sorte qu'une valeur posée par un harness survive au `source`. Anti-vacuité : le fichier fait
   plus de 100 000 octets (le seuil déjà employé par le scan mika#2508) et le nom y apparaît au
   moins une fois. Une régression vers l'affectation inconditionnelle — ou vers une résolution
   qui relirait `$HOME` au moment de l'usage — rend cette assertion rouge, ce qu'aucun test
   comportemental ne ferait : la redirection redeviendrait silencieusement inerte et **toutes**
   les assertions de staging resteraient vertes.
4. **Note d'exécution** — les trois verdicts de U3 et le contrôle négatif de U4 se pilotent sur
   une **copie de travail** sous `.pilot-scratch/` (`git show HEAD:scripts/canary-pilot-containment > .pilot-scratch/scripts/canary-pilot-containment`,
   seul sur sa ligne), jamais en mutant l'arbre : muter le canari en place pour reproduire le
   défaut **écraserait le vrai jeton de l'hôte**, ce qui est exactement ce que ce plan retire.
   Cette contrainte est la raison pour laquelle D1 (l'auto-contrôle de U3) n'a pas de contrôle
   négatif en CI — voir `## Fire-Disposition`.
5. Aucune cible `Makefile` nouvelle : les deux suites touchées sont déjà dans `make test`
   (lignes 157-158) et dans `make test-github-token-not-in-sandbox` / `make test-dispatch-lib`,
   donc dans la CI. **À vérifier à l'exécution** plutôt qu'à supposer.

**Patterns to follow :** l'anti-vacuité de
`mika2508_every_operator_var_read_by_dispatch_lib_reaches_the_child_or_is_named`
(`crates/mika-agent/src/skills/executor.rs`) — chemin, taille, cardinalité minimale de
population, **avant** toute autre assertion ; les allowlists livrées vides + assertion
auto-nettoyante de mika#2201 ; `scripts/check-pilot-push-sites.sh` pour un scan lexical sur un
fichier de `scripts/`.

**Test scenarios :**
- D2 rouge sur une copie où le `:=` redevient `=`.
- D3 rouge sur une copie où la pose passe **après** le `source`.
- D3 rouge sur une copie où `_canary_on_exit` observe `$_PILOT_GH_TOKEN_FILE`.
- D3 verte de bonne foi : les commentaires et les chaînes citant le chemin ne la font pas
  rougir (le canari cite ce chemin dans sa prose).
- D4 rouge sur l'arbre d'avant U1.
- Les 9 assertions existantes de la suite mika#2056 restent vertes.

**Verification :** les deux suites passent ; chaque contrôle négatif ci-dessus est **vu rouge**
sur sa copie avant d'être déclaré vert.

### U6. Documentation opérateur et sonde

**Goal :** l'opérateur sait que le canari n'abîme plus rien, sait lire les trois verdicts, et
sait pourquoi `curl_github` peut légitimement varier.

**Requirements :** R9, et la sonde S1

**Dependencies :** U3, U4

**Files :**
- `CLAUDE.md` (Signal Q, qui nomme déjà `scripts/canary-pilot-containment` ; entrée `GH_TOKEN`)
- `docs/operator/pilot-egress-relay.md` (à vérifier : le runbook décrit-il l'injection GitHub ?)

**Approach :**
1. Signal Q nomme le canari comme sonde post-déploiement de l'argv. Ajouter une phrase : depuis
   mika#2578 le canari redirige son staging et **atteste** en sortie que
   `~/.mika/pilot-gh-token` n'a pas bougé ; les trois verdicts et ce que chacun prescrit.
2. Dans l'entrée `GH_TOKEN` (§ *Inside the contained pilot sandbox*), ajouter la halte : un
   `401` observé **avec** le placeholder, sur un hôte où le canari vient de tourner, n'est plus
   imputable au canari — lire d'abord la ligne d'attribution R9 (le fichier hôte est-il présent
   et frais ?) **avant** de toucher au proxy ou au `--setenv`. C'est l'inversion de diagnostic
   que ce correctif achète.
3. Écrire la sonde **S1** avec son préalable et ses haltes (voir `## Verification Contract`).

**Test expectation :** none -- documentation seule.

**Verification :** les gestes de S1 sont copiables tels quels ; aucun chemin ni nom d'événement
inventé (chacun relu dans le code livré).

---

## Fire-Disposition

Ce plan livre **quatre** détecteurs. Disposition retenue : **(a) exception nommée en allowlist,
allowlist livrée VIDE** pour les trois détecteurs de CI, et **(b) livré armé sans contrôle
négatif automatisé, avec son suivi** pour le quatrième — qui est un détecteur de **runtime**
sur l'hôte, non exécutable en CI.

| # | détecteur | où | population sur un arbre corrigé | disposition |
|---|---|---|---|---|
| D1 | auto-contrôle « le fichier hôte n'a pas bougé » | runtime, `_canary_on_exit` (U3) | vide (`ok:`) | **(b)** — voir ci-dessous |
| D2 | `_PILOT_GH_TOKEN_FILE` a un site de déclaration unique, conditionnel | `test-dispatch-lib.sh` (U5) | **vide** | (a), allowlist vide |
| D3 | le canari pose la redirection avant le `source`, et n'observe pas la redirection | PART D de la suite mika#2056 (U5) | **vide** | (a), allowlist vide |
| D4 | `_stage_pilot_gh_token` redirigé n'écrit pas le défaut | PART B de la suite mika#2056 (U5) | **vide** | (a), allowlist vide |

**D2, D3, D4 — allowlist livrée vide, et c'est une décision, pas un oubli.** Aucune violation
préexistante ne subsiste après U1–U4 : le seul écrivain illégitime du chemin hôte est le canari,
et U2 le retire ; les cinq autres harnais détournent `HOME` (KTD3) et sont donc hors population.
Chaque allowlist est **pinnée vide** par une assertion sœur, pour qu'une entrée ajoutée un jour
soit un acte visible et non un contournement. **Quand l'un de ces scans tire, la résolution est
de rediriger le site fautif — jamais d'y ajouter une ligne** (doctrine mika#2201 : *« on
déclare, on n'allowliste pas »*). Chaque entrée hypothétique devrait nommer la donnée précise,
référencer un ticket de suivi, et porter une assertion auto-nettoyante qui rougit le jour où
l'exception n'a plus d'objet — sur le modèle de `DISPATCH_ENV_KNOWN_INERT`.

**D1 — armé, sans contrôle négatif automatisé, et le suivi est nommé.** Son contrôle négatif
demanderait de faire écrire le chemin hôte par défaut à un processus de test, c'est-à-dire
d'**écraser le vrai jeton de l'opérateur** : reproduire le défaut pour prouver qu'on le détecte
coûterait exactement le dégât que le ticket ferme. Ce qui est fait à la place, et qui est dit
comme tel :
- les **trois verdicts** sont exercés à la main sur une copie sous `.pilot-scratch/` avec le
  chemin observé pointé vers un temporaire, et les cas « leurre » et « autre chose » sont **vus
  produire des verdicts différents** — sans quoi le troisième cas serait décoratif (U3) ;
- ses deux manières d'être faux sont couvertes **structurellement** par D3 : observer la
  redirection (garde tautologique) et être posé après le premier appel au bac à sable ;
- la **prévention** est prouvée en CI par D4, indépendamment de la détection.

**Suivi nommé pour armer un contrôle négatif de D1 :** extraire la comparaison dans un helper
sourcé (`skills/bundled/_shared/` sur le modèle de `_shared/cwd-guard.sh`, mika#2536),
paramétré par le chemin observé, avec sa propre suite pilotant les trois verdicts. Précondition
explicite : que D1 ait **effectivement produit** un verdict `VIOLATION` ou `INCONCLUSIVE` au
moins une fois en exploitation. Sans cette mesure, on paierait un fichier, une suite et une
cible `Makefile` pour trois lignes dont le régime attendu est le silence.

**Régime attendu de chaque détecteur, pour que le silence soit lisible :** D2, D3, D4 sont des
gardes de CI — leur signal est leur propre rouge. D1 est un détecteur d'exploitation dont le
régime attendu est `ok:` à chaque exécution ; **son silence ne prouve rien tant que personne ne
lance le canari**, et son contrôle positif est la ligne d'attribution R9, qui montre qu'il a
bien regardé un fichier plutôt que rien.

---

## Verification Contract

**Automatisé (CI et local) :**

- `bash skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` : tous verts ;
  baseline 9 assertions, plus celles de U5 (PART B étendue, PART D nouvelle).
- `bash skills/bundled/_shared/test-dispatch-lib.sh` : aucune régression, plus D2.
- `bash skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`,
  `test_sandbox_git_usable.sh`, `test_sandbox_gh_usable.sh`, `test_sandbox_log_dir_bound.sh` :
  aucune régression (les quatre écrivent aussi le jeton, sous un `HOME` détourné — KTD3).
- `cargo test -p mika-agent mika2508` : aucune régression (KTD1 ; si rouge, appliquer le repli
  nommé dans la Verification de U1).
- `bash -n scripts/canary-pilot-containment` ; `shellcheck` sur les fichiers shell touchés si
  le dépôt l'exécute pour eux.
- Chaque contrôle négatif de U5 **vu rouge** sur sa copie sous `.pilot-scratch/` avant d'être
  déclaré vert.

**Sonde S1 — geste d'OPÉRATEUR sur l'hôte, non exécutable depuis le bac à sable de dispatch.**

> **Préalable.** `skills/bundled/_shared/` est une projection du **binaire**, pas du checkout
> (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on vient de bâtir —
> sans quoi chaque mesure ci-dessous décrit le binaire d'hier.

```bash
# 1. Empreinte AVANT (jamais le contenu)
sha256sum < ~/.mika/pilot-gh-token | cut -d' ' -f1
stat -c '%s %y' ~/.mika/pilot-gh-token

# 2. Le canari, sur l'hôte
scripts/canary-pilot-containment

# 3. Empreinte APRÈS — doit être identique, et la ligne de verdict doit dire `ok:`
sha256sum < ~/.mika/pilot-gh-token | cut -d' ' -f1
```

| observation | lecture |
|---|---|
| empreintes identiques **et** `ok: host_gh_token_untouched` | le correctif tient |
| empreintes identiques **et aucune** ligne de verdict | le binaire servi est antérieur au correctif (classe mika#2340) — **établir le déploiement avant toute conclusion** |
| `VIOLATION` | le correctif a régressé : un site écrit encore le chemin hôte. Lire D3 **avant** de toucher au prédicat |
| `INCONCLUSIVE` | un dispatch a restagé pendant le canari (ou une rotation App) — **pas une panne** : c'est le troisième verdict qui fait son travail |
| `curl_github=401` **avec** `ok:` | le canari n'est plus en cause : lire la ligne d'attribution R9, puis le journal du proxy (KTD4) |

**Haltes.**
- **Halte 1 — `VIOLATION` alors que D3 est verte.** Un écrivain traverse par un chemin que le
  scan ne voit pas (une variable, une indirection). **Ne pas élargir D3 par réflexe** : établir
  d'abord *quel* site a écrit, les remèdes diffèrent.
- **Halte 2 — le fichier hôte est absent avant **et** après, et `curl_github` rend 401.** Rien
  n'a été prouvé sur ce correctif : aucun dispatch n'a jamais staché sur cet hôte, l'addon
  retombe sur son environnement, et c'est le canal du ticket de suivi. Lancer un dispatch réel,
  puis rejouer S1.
- **Halte 3 — `ENV_LEAK` nomme `ANTHROPIC_API_KEY`.** Ce n'est **plus** le faux positif de la
  note annexe : U4 exclut le placeholder par valeur, donc une occurrence signifie qu'une
  **autre** valeur a survécu. Lire la ligne `--setenv ANTHROPIC_API_KEY` de `dispatch-lib.sh`
  avant de toucher au motif du contrôle.
- **Halte transverse — les deux empreintes et la ligne de verdict muettes.** Vérifier qu'un
  appel au bac à sable a réellement eu lieu (`bwrap` installé, relais servant) avant de
  conclure. *Une garde que personne n'a exercée se lit exactement comme une garde qui marche*
  (mika#2205).

## Definition of Done

- U1 à U6 livrés ; chaque suite du Verification Contract verte.
- Les six contrôles négatifs de U5 et U4 ont été **vus rouges** sur leur copie
  `.pilot-scratch/` avant d'être déclarés verts.
- Les trois verdicts de D1 ont été vus produire trois sorties distinctes.
- Aucune assertion portant sur la valeur réelle d'un jeton n'a été retirée d'aucune suite.
- Les trois allowlists de U5 sont livrées **vides** et pinnées vides.
- Aucun nom d'événement, chemin ou cible `Makefile` inventé : chacun relu dans l'arbre.
- Aucun résidu sous `.pilot-scratch/` publié (il est exclu de git) ; aucun `pr-body.md`
  survivant.
- Le corps de PR porte `Closes #2578`, les rectifications KTD3/KTD4/KTD5, la sonde S1 avec ses
  haltes, et les deux tickets de suivi nommés (héritage d'env du daemon mitmdump ;
  `install -Dm755` du binaire proxy).

## Acceptance criteria

- [ ] Après une exécution de `scripts/canary-pilot-containment`, `~/.mika/pilot-gh-token` est
      octet pour octet identique à ce qu'il était avant — c'est le test d'acceptation littéral
      du ticket (R1).
- [ ] Aucun des cinq modes du canari (`--show-args`, `--ensure-relay`, `--restart-relay`,
      `--enter`, défaut) n'écrit ce chemin (R1).
- [ ] Le canari traverse toujours le vrai `_stage_pilot_gh_token`, redirigé et non neutralisé
      (R2).
- [ ] Le chemin de staging d'un dispatch réel reste `$HOME/.mika/pilot-gh-token` et n'est pas
      atteignable depuis l'environnement du service (R3).
- [ ] Le canari imprime un verdict d'auto-contrôle sur tous ses chemins de sortie, à trois
      valeurs, observant le chemin hôte par défaut et non la redirection (R4, R5, R6).
- [ ] Une violation confirmée rend le canari non nul ; un cas non concluant préserve le statut
      (R8).
- [ ] Le canari nomme l'état du fichier hôte dont dépend `curl_github` (R9).
- [ ] `must-fail: env vars` rend `ok:` sur un run nominal, le placeholder
      `ANTHROPIC_API_KEY=proxy-managed-no-secret` étant exclu **par valeur** (R10, R11).
- [ ] Toute autre valeur portée par `ANTHROPIC_API_KEY` déclenche toujours `ENV_LEAK`, et la
      ligne nomme les variables fautives sans imprimer aucune valeur (R11, R12).
- [ ] Une garde structurelle refuse un site futur écrivant le chemin hôte depuis le canari, et
      refuse un auto-contrôle qui observerait la redirection (R13).
- [ ] Un test comportemental montre le défaut (sans redirection, le défaut est écrit) et sa
      correction (avec redirection, le défaut n'est pas créé) (R14).
- [ ] Chaque garde porte son anti-vacuité (chemin, taille, cardinalité) avant toute autre
      assertion (R15).

---

## Risks & Dependencies

| Risque | Mitigation |
|---|---|
| Le `:=` fait entrer `_PILOT_GH_TOKEN_FILE` dans la population du scan mika#2508, qui rougit | Prédit non par KTD1 (nom préfixé `_`, pas une variable opérateur) mais **à constater** ; repli pré-décidé dans la Verification de U1 : pose côté canari après le `source`, deux sites, `dispatch-lib.sh` inchangé hors commentaire. Ne **pas** nommer l'inertie dans `DISPATCH_ENV_KNOWN_INERT` : cette liste est faite d'arbitrages de canal en attente, pas de knobs de harness |
| Un second `trap … EXIT` remplace le trap de nettoyage de la ref victime et laisse une `refs/heads/canary-victim/*` pendante dans le dépôt de l'opérateur | KTD7 : une seule fonction `_canary_on_exit`, installée une fois, qui reprend verbatim la commande de nettoyage existante ; le trap conditionnel de la ligne 385 est retiré dans le même geste |
| L'auto-contrôle crie à la violation sur une rotation de jeton d'installation App | KTD5 : verdict à trois valeurs, discriminé par l'empreinte du leurre — une valeur autre rend `INCONCLUSIVE`, pas `VIOLATION` |
| L'auto-contrôle observe `$_PILOT_GH_TOKEN_FILE` et est donc tautologiquement vert | R6 + assertion structurelle D3 ; c'est aussi une condition d'arrêt de la Goal Capsule |
| `sha256sum` absent de l'hôte | Repli `cksum` nommé (Assumptions) ; outil manquant ⇒ `INCONCLUSIVE`, jamais `ok:` |
| Une valeur de jeton atterrit dans un argv ou une trace `set -x` du fait de l'auto-contrôle | KTD8 : `sha256sum` sur **stdin**, aucun contenu en variable, aucune copie de sauvegarde sur disque |
| Le contrôle négatif de U4 exige de muter la ligne `--setenv` de `dispatch-lib.sh` | Sur une **copie** sous `.pilot-scratch/` (note d'exécution U5) — muter l'arbre en place ferait tourner un dispatch réel sous une ligne mutée |
| Le déploiement n'atteint pas le binaire : `dispatch-lib.sh` et `_shared/` sont projetés depuis le binaire, pas depuis le checkout (mika#2340) | La sonde S1 commence par `cat ~/.mika/skills/.manifest-writer`, et la table de lecture nomme le cas « aucune ligne de verdict » |
| Le canal daemon mitmdump reste ouvert et un pilote part en 401 malgré le correctif | KTD6 le borne (réchauffement avant les leurres) ; Halte 2 de S1 le nomme ; fermeture complète = ticket de suivi avec sa précondition |

## Sources

- `scripts/canary-pilot-containment` : exports de leurres (194-203), `source` (81),
  `--show-args` (87-92), `--enter` (176-189), PART 0 (240-258), trap conditionnel (385),
  `must-fail: env vars` (420-422), `/proc/1/environ` (426), contrôle `GH_TOKEN` (455-468),
  synthèse (640).
- `skills/bundled/_shared/dispatch-lib.sh` : déclaration `_PILOT_GH_TOKEN_FILE` (588-593),
  `_stage_pilot_gh_token` (1047-1060), appel dans `_run_pilot_sandboxed` (1473),
  `_ensure_pilot_helper` et son `nohup` sans `env -i` (673-712), `--setenv` des deux
  placeholders (1558, 1567).
- `scripts/mika-pilot-github-auth-addon.py` : `_TOKEN_FILE` (53), préférence fichier puis env
  (137).
- `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` : `export HOME`
  (56), PART B et son `staged_file` (140-158), PART C (165-215).
- `crates/mika-agent/src/skills/executor.rs` : `PILOT_DISPATCH_ENV` (345),
  `reaches_dispatch_child` / `DISPATCH_ENV_KNOWN_INERT` et l'anti-vacuité du scan (5263-5385).
- `docs/solutions/cross-repo-patterns/pilot-concurrency-shared-resources-2026-09-03.md` § 6 :
  le chemin partagé déjà nommé comme « changement nommé » pour N>1.
- `Makefile` : `make test` (157-158), `test-github-token-not-in-sandbox` (226-227).

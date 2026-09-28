---
title: "Retirer un secret du sandbox sans rien laisser à sa place casse en silence les clients qui exigent une valeur locale"
date: 2026-09-28
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh, scripts/verify-no-secret-in-setenv.sh, scripts/canary-pilot-containment, skills/bundled/_shared/tests
problem_type: logic_error
component: tooling
symptoms:
  - "gh à l'intérieur du sandbox pilote s'arrête localement sur « gh auth login » avant tout appel réseau"
  - "git push depuis le même sandbox continue de fonctionner, ce qui masque la panne"
  - "les pilotes ne peuvent ni lire les commentaires d'un ticket ni ouvrir de PR (mika#2562 : 41 tours, 6,6 USD perdus)"
root_cause: config_error
resolution_type: code_fix
severity: high
tags: [loop-substrate, dispatch-lib, sandbox, bwrap, gh, github-token, secret-placeholder, negative-control]
ticket: mika#2572
---

# Retirer un secret du sandbox sans rien laisser à sa place casse en silence les clients qui exigent une valeur locale

## La leçon

Si l'on retire une credential d'un sandbox sans rien mettre à la place, tout client qui exige une valeur locale **avant** d'émettre une requête cesse de fonctionner, sans bruit. `gh` s'arrête sur `gh auth login` sans le moindre appel réseau. Les clients qui envoient d'abord, comme `git`, continuent de marcher, puisque le proxy authentifie la requête au passage. La panne reste donc cachée derrière un chemin qui fonctionne encore. Le remède est un **placeholder non secret** : le client démarre et émet, puis le proxy remplace l'en-tête par la vraie credential côté hôte.

L'incident laisse aussi cinq corollaires :

1. Une garde qui vérifie l'**absence d'un NOM** en dit plus que l'invariant réel, qui porte sur l'absence de **VALEUR**. Elle est restée verte pendant un mois pendant que le défaut tournait en production.
2. Un lint « par occurrence » écrit avec `grep -c` compte des **lignes**. Un leurre sur la même ligne ou une concaténation de chaînes suffit à le contourner.
3. Une suite qui se met en *skip* quand l'outil manque **à l'intérieur** du sandbox cache justement la régression qu'elle doit garder.
4. Un contrôle négatif par mutation qui injecte `"$VAR"` sous `set -u` s'interrompt avant toute assertion. Ce rouge ne prouve rien. Il faut écrire `"${VAR:-}"`.
5. Dans le payload `sh -c '...'` du canari, entre quotes simples, un message qui contient lui-même des quotes simples casse le quoting alors que `bash -n` passe.

## Problème

mika#2056 (2026-08-30) a retiré `GH_TOKEN` du sandbox bwrap du pilote. Le vrai jeton a été déplacé côté hôte, dans un addon mitmdump qui injecte l'en-tête `Authorization` sur `api.github.com` et `github.com`. L'invariant visé, « le sandbox ne détient jamais de secret », est juste. Mais la variable a été retirée **sans rien laisser à sa place**, alors que l'autre credential déjà gérée par le proxy, `ANTHROPIC_API_KEY`, gardait un placeholder.

Depuis, `gh` n'a plus aucun jeton local et refuse de démarrer. Les pilotes du loop ne pouvaient plus lire les commentaires d'un ticket ni ouvrir de PR. `git push`, lui, marchait toujours, et cela a masqué la panne (mika#2572).

## Symptômes

- Dans le log du pilote 8195ada3, l.5127 : `You are not logged into any GitHub hosts`. Ailleurs : `To get started with GitHub CLI, please run: gh auth login`.
- `gh api user` sort avec le code 4 (auth requise) sans ouvrir de connexion. Aucune trace côté egress proxy, puisque rien n'y arrive.
- Le dispatch be030903 (mika#2562) a perdu 41 tours et 6,6 USD (chiffres relevés dans le corps de mika#2572) : les commentaires qui levaient sa précondition lui étaient illisibles.
- `git push` depuis le même sandbox réussit. Le pipeline a donc l'air sain tant qu'aucune étape n'appelle `gh`.

Mesure locale avec gh 2.92.0, sous `env -i HOME=<vide>` :

| `GH_TOKEN` | `gh auth token` | `gh api user` |
|---|---|---|
| absent | rc 1, `no oauth token found for github.com` | affiche `gh auth login`, rc 4, **aucun appel réseau** |
| `proxy-managed-no-secret` | affiche le placeholder, rc 0 | tente `Get "https://api.github.com/user"` : `proxyconnect tcp ...` |

## Ce qui n'a pas marché

- **Le contournement manuel ne passe pas à l'échelle (session history).** Avant le ticket, faute de mieux, la panne était traitée comme un geste opérateur (« lancer `! gh auth login -h github.com` ») qui débloquait les pushs de PR de l'orchestrateur. Ce geste n'existe pas pour un pilote headless : il n'a pas de session interactive où se connecter. Et un re-groom de mika#2562 a dû attendre, parce que « le pilote ne peut pas lire les commentaires du ticket ».
- **Les gardes de mika#2056 étaient vertes.** Test 10b affirmait « GH_TOKEN is NOT passed via --setenv ». Test 12(b) et PART C de `test-pilot-github-token-not-in-sandbox.sh` attendaient `<absent>`. Ces trois assertions portaient sur le **nom**, alors que le danger est la **valeur**. Elles ont validé pendant un mois l'état exact qui cassait `gh`. Elles traversent bien la Phase 2b : les deux suites montent un relais factice avec `stub_serving_egress_relay`. La preuve est qu'après l'ajout du placeholder, ces trois assertions et `AUDITED_SETENV_NAMES` sont passées au rouge, et elles seules.
- **Première ancre du lint.** `--setenv[[:space:]]+NAME[[:space:]]` ne voyait pas un `--setenv GH_TOKEN` en fin de ligne dont la valeur est sur la ligne suivante du tableau, une forme bash valide. L'ancre a été élargie à `([[:space:]]|$)` et épinglée par une fixture qui passe sous l'ancre fautive.
- **Un lint qui comptait des lignes.** La revue adversariale (ce-code-review, puis reproduction par le validateur) a montré deux contournements. La première version utilisait `grep -c`, qui compte les lignes, et `grep -F`, qui accepte le placeholder comme simple sous-chaîne. Les deux cas suivants sortaient donc 0, « clean » :
  ```bash
  --setenv GH_TOKEN "proxy-managed-no-secret" --setenv GH_TOKEN "$GH_TOKEN"   # leurre + vraie valeur, même ligne
  --setenv GH_TOKEN "proxy-managed-no-secret""$GH_TOKEN"                        # concaténation
  ```
- **Première version de la nouvelle suite.** Elle se mettait en *skip*, avec exit 0, quand `gh` était présent sur l'hôte mais invisible dans le sandbox. C'est précisément la classe de régression qu'elle doit garder.
- **Contrôle négatif mal écrit.** La mutation remplaçait le placeholder par `"$GH_TOKEN"`. `test_sandbox_no_secret_in_argv.sh` s'arrêtait alors avec rc=1 sous `set -u` **avant la première assertion**. Le rouge venait de l'expansion d'une variable non définie, pas d'une garde. Il ne prouvait rien.
- **Contrôle négatif réexécuté dans le même shell.** Sourcer de nouveau `dispatch-lib.sh` après `stub_serving_egress_relay` restaure les vrais `_PILOT_EGRESS_SOCK`, `_PILOT_EGRESS_PROXY_BIN` et `_ensure_pilot_egress_proxy`. Le test se lierait alors au socket du relais réel.
- **Le canari et le piège du quoting.** Le bloc du canari vit dans le payload entre quotes simples de `_run_pilot_sandboxed sh -c '...'` (`scripts/canary-pilot-containment:396`). Un message contenant `'gh auth login'` entre quotes simples fermait ce payload. `bash -n` restait vert, puisque le résultat est syntaxiquement valide, mais la sémantique était cassée.

## Solution

Correctif ouvert pour mika#2572, non mergé à l'écriture.

### 1. Le placeholder dans le sandbox

`skills/bundled/_shared/dispatch-lib.sh`, producteur `net_setenv_args`. Le placeholder est posé à côté de celui d'Anthropic (l.1558 et l.1567) :

```bash
# avant (mika#2056) : aucun GH_TOKEN, gh s'arrête sur `gh auth login`
            --setenv ANTHROPIC_API_KEY "proxy-managed-no-secret"

# après (mika#2572)
            --setenv ANTHROPIC_API_KEY "proxy-managed-no-secret"
            ...
            --setenv GH_TOKEN "proxy-managed-no-secret"
```

Le placeholder ne sort jamais comme credential. Dans `scripts/mika-pilot-github-auth-addon.py`, `requestheaders` (l.122-145) retire les en-têtes client puis pose le vrai jeton, uniquement pour les hôtes GitHub :

```python
    for header_name in ("authorization", "proxy-authorization"):
        flow.request.headers.pop(header_name, None)
    flow.request.headers["Authorization"] = _auth_header_for(host, token)
```

Le commentaire d'audit de `dispatch-lib.sh` (l.994-1002 et l.1013-1020) dit maintenant ce qui est vrai : le **nom** `GH_TOKEN` est présent, jamais une **valeur** de jeton.

### 2. Un lint qui compte les occurrences et vérifie la valeur

`scripts/verify-no-secret-in-setenv.sh`. Le scalaire `EXEMPT_SETENV_NAME` devient une liste fermée (l.85-86) :

```bash
EXEMPT_SETENV_NAMES=(ANTHROPIC_API_KEY GH_TOKEN)
EXEMPT_SETENV_VALUE="proxy-managed-no-secret"
```

La règle 2 (l.235-240) compte des **occurrences** avec `grep -o | wc -l`, et non des lignes. Le guillemet fermant du placeholder est borné :

```bash
# avant : grep -c (lignes) + grep -F (sous-chaîne) → leurre et concaténation passent
# après :
_total=$({ grep -oE -- "--setenv[[:space:]]+$name([[:space:]]|\$)" "$CODE_ONLY" || true; } | wc -l)
_placeheld=$({ grep -oE -- "--setenv[[:space:]]+${name}[[:space:]]+\"$EXEMPT_SETENV_VALUE\"([[:space:]]|\$)" "$CODE_ONLY" || true; } | wc -l)
# exige _total == _placeheld && _total >= 1
```

Le `|| true` dans l'accolade empêche `pipefail` de faire échouer le compte à zéro. Un nom hors de la liste, par exemple `NPM_TOKEN`, reste refusé même s'il porte le placeholder. Deux fixtures de régression, vues rouges avant le correctif, épinglent le leurre sur la même ligne et la concaténation.

### 3. Des gardes qui portent sur la valeur, pas sur le nom

- `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`, Test 10b (l.496-530). « GH_TOKEN absent de `--setenv` » devient : il y a au moins une occurrence, et **chaque** triplet `--setenv GH_TOKEN <v>` porte le placeholder. L'assertion historique « la valeur est absente de l'argv » est conservée. Test 12(b) (l.589) attend le placeholder dans le vrai sandbox.
- `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh`, PART C (l.161-188). `<absent>` devient `proxy-managed-no-secret`, jamais `FAKE_TOKEN`. Les assertions sur la valeur réelle sont conservées : pas de `/run/mika-pilot-secrets/GH_TOKEN`, fichier de staging invisible.
- Les comptes passent de 35 à 37 et de 9 à 10.

### 4. Une suite qui prouve que `gh` part vers le réseau

Nouvelle suite `skills/bundled/_shared/tests/test_sandbox_gh_usable.sh`, cible Makefile `test-sandbox-gh-usable`. Elle lance un vrai bwrap via `_run_pilot_sandboxed` avec le relais factice `lib-fake-egress-relay.sh`.

- **Positif.** `gh` est visible, `gh auth token` renvoie rc 0 et le placeholder, et `gh api user` tente `Get "https://api.github.com/user"` sans produire `gh auth login` ni rc 4 (l.148-162). En pratique la requête bute sur `proxyconnect tcp: dial tcp 127.0.0.1:8891: connect: connection refused`, ce qui montre qu'elle a quitté le client.
- **Contrôle négatif dans le même appel.** Sans placeholder, on doit voir `no oauth token`, `gh auth login` et rc 4 (l.165-171). Il tourne dans un processus `bash` séparé, qui source la copie mutée puis appelle le stub, pour ne jamais toucher le socket du relais réel.
- **`gh` absent dans le sandbox alors qu'il est présent sur l'hôte : échec, jamais skip** (l.150-154). Seule l'absence de `gh` **sur l'hôte** justifie un skip (l.88-89).

### 5. Un canari qui distingue trois états

`scripts/canary-pilot-containment`, l.460-464. Le simple `if [ -n "${GH_TOKEN:-}" ]` qui signalait LEAK devient un `case`. Sans ce changement, le canari aurait affiché un faux LEAK après le déploiement du correctif. C'est le relecteur de faisabilité de doc-review qui l'a trouvé.

```sh
case "${GH_TOKEN-__unset__}" in
    proxy-managed-no-secret) echo "ok:   gh_token_placeholder_only" ;;
    __unset__)               echo "WARN: GH_TOKEN unset in the sandbox — gh will stop on gh auth login (mika#2572 regressed)" ;;
    *)                       echo "LEAK: GH_TOKEN carries a value other than the placeholder (mika#2056 regressed)" ;;
esac
```

Aucune quote simple dans les messages, puisque le bloc est dans un payload `sh -c '...'`.

### 6. Documentation opérateur

`CLAUDE.md`, entrée GH_TOKEN :

- le placeholder ;
- la sonde post-déploiement : sha du manifest-writer, `tool_result` du `gh` du pilote lui-même, ligne du canari ;
- les arrêts : un 401 avec le placeholder veut dire que l'addon ne reçoit pas le trafic. Une erreur x509 renvoie au repli `SSL_CERT_FILE` du prologue, qui n'est posé que si un bundle CA système existe.

## Pourquoi ça marche

Le problème vient de l'ordre d'émission des clients :

- **`git` émet d'abord.** Il envoie la requête et laisse le serveur, ici le proxy, décider de l'authentification.
- **`gh` lit d'abord sa config locale.** Sans jeton, il n'émet jamais.

Le proxy MITM ne peut réécrire que ce qui lui parvient. Retirer la variable coupait donc `gh` en amont du point où l'injection aurait eu lieu.

Le placeholder donne à `gh` la valeur locale qu'il exige. La requête part, avec un en-tête `Authorization` factice. L'addon le retire et pose le vrai jeton. L'invariant de mika#2056 tient toujours : aucune valeur secrète ne franchit la frontière bwrap. Le nom de la variable n'a jamais été le secret.

Les gardes sont maintenant écrites sur la **valeur**, c'est-à-dire sur l'invariant réel. Elles acceptent l'état sain et rejettent la fuite. Avant, elles rejetaient l'état sain et acceptaient l'état cassé.

## Prévention

- **Avant de retirer une credential d'un sandbox, inventorier chaque client qui la lit** et classer chacun : émet d'abord (proxy suffisant) ou exige une valeur locale (placeholder obligatoire). Le précédent `ANTHROPIC_API_KEY` montrait déjà le motif.
- **Écrire les gardes sur l'invariant, pas sur une approximation.** Si l'invariant est « aucune valeur secrète », la garde vérifie des valeurs. Une garde sur le nom est plus stricte que l'invariant et fige l'état cassé comme conforme.
- **Garder chaque capacité par un test d'usage réel à travers le sandbox**, en plus des tests d'absence : `gh api user` doit atteindre le réseau. Un test d'absence seul ne voit pas une capacité perdue.
- **Lints « par occurrence » : `grep -o … | wc -l`, jamais `grep -c`.** Borner les deux côtés du littéral attendu. Épingler chaque contournement (même ligne, concaténation, valeur sur la ligne suivante) par une fixture vue rouge avant le correctif.
- **Pas de skip sur une absence que le test doit détecter.** Un skip n'est légitime que pour une précondition **hôte** (bwrap ou `gh` absent du PATH hôte). Tout ce qui manque à l'intérieur du sandbox est un échec.
- **Contrôles négatifs par mutation :**
  - utiliser `"${VAR:-}"` sous `set -u`, et vérifier que ce sont les **assertions** qui passent au rouge, pas l'interpréteur ;
  - commiter l'état vert avant de muter, car `git checkout` efface le travail non commité (auto memory [claude]) ;
  - contrôler terme par terme : neutraliser `A||B` n'épingle ni `A` ni `B` (auto memory [claude]) ;
  - mettre le contrôle positif et le négatif dans le même appel (auto memory [claude]) ;
  - lancer la mutation dans un processus séparé quand le fichier muté est sourcé, pour ne pas écraser les stubs.
- **Payloads `sh -c '...'` :** aucune quote simple dans les messages. `bash -n` ne détecte pas ce cas, il faut exécuter le bloc.
- **Après déploiement :** lire le `tool_result` d'un vrai appel `gh` d'un pilote, et la ligne `ok:   gh_token_placeholder_only` du canari. Un binaire vert ne prouve pas que la capacité est rétablie.

## Issues liées

- mika#2056 — retrait de `GH_TOKEN` du sandbox et injection côté hôte par l'addon mitmdump ; l'invariant « aucune valeur secrète » reste juste, c'est l'absence de placeholder qui cassait `gh`.
- mika#2039 — origine du canal `--setenv` audité et du lint `verify-no-secret-in-setenv.sh` ; voir `docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`, dont la règle « par occurrence, pas globale au fichier » est ici étendue à une liste fermée et corrigée (elle comptait des lignes).
- mika#2141 — `docs/solutions/best-practices/a-recovery-net-hides-the-engine-and-a-presence-probe-cannot-see-it-2026-09-02.md` : même famille (une sonde de présence ne voit pas une capacité perdue) ; il cite mika#2039 et mika#2056 comme gardes « satisfaites sans exemption », ce que mika#2572 nuance en ajoutant une seconde exemption légitime.
- `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md` — la moitié exemptée d'une garde est là où tombe la prochaine panne : c'est exactement le contournement leurre + concaténation trouvé en revue.

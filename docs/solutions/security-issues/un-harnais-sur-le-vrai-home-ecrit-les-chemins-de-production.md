---
title: Un harnais qui tourne sur le vrai $HOME écrit les chemins de production
date: 2026-09-29
category: security-issues
module: scripts/canary-pilot-containment, skills/bundled/_shared/dispatch-lib.sh
problem_type: security_issue
component: tooling
symptoms:
  - "Le canari de confinement écrase ~/.mika/pilot-gh-token par son propre leurre à chaque exécution"
  - "Tout pilote en vol part en 401 sur api.github.com et github.com, git push compris, jusqu'au prochain dispatch réel"
  - "Le contrôle must-work du canari rend curl_github=401 alors qu'il attend un 200 authentifié"
root_cause: test_isolation
resolution_type: code_fix
severity: critical
tags: [canary, harness, gh-token, credential, host-isolation, fail-closed, source-scan, three-valued-verdict, negative-control, mika-2578, mika-2056, mika-2201]
---

# Un harnais qui tourne sur le vrai `$HOME` écrit les chemins de production

## Problème

`scripts/canary-pilot-containment` exerce le bac à sable pilote **par le vrai code
de production** — c'est sa raison d'être. Il exporte donc un leurre
`GH_TOKEN=github_pat_0…canary…` et appelle `_run_pilot_sandboxed` cinq fois. Or
chaque appel traverse `_stage_pilot_gh_token`, qui écrit `$GH_TOKEN` dans
`$_PILOT_GH_TOKEN_FILE` — dont le défaut est `$HOME/.mika/pilot-gh-token`, le
fichier que l'addon mitmdump de mika#2056 lit **côté hôte** pour injecter
l'en-tête `Authorization`.

Le maillon décisif n'est pas le leurre, c'est que le chemin de staging est une
constante dérivée de `$HOME` et que le canari est le seul harnais qui tourne
délibérément sur le vrai `$HOME` de l'hôte.

## Symptômes

Mesuré le 2026-09-28 : `~/.mika/pilot-gh-token` réécrit à 20:33:03Z **pendant**
l'exécution du canari, 49 octets contre 93 à 17:32 (dernier staging par un
dispatch réel), préfixe `github_pat_0…`. Dans le même run, `curl_github=401`. Le
pilote de mika#2565 était en vol : il a envoyé le leurre à GitHub jusqu'au
dispatch suivant.

Le 401 n'est pas un symptôme séparé, c'est la conséquence directe :
`GET https://api.github.com/` **sans** `Authorization` rend 200 ; avec un jeton
invalide, 401. L'addon injectait donc bien le leurre.

## Ce qui n'a pas marché (les remèdes qui se présentent d'eux-mêmes)

- **Détourner `HOME`**, comme le font les **cinq** suites sœurs qui sourcent
  `dispatch-lib.sh` et appellent le bac à sable (`export HOME=` avant le
  `source`, vérifié dans les cinq). C'est pour cela qu'elles sont saines. Le
  canari ne peut pas l'employer : il a besoin du vrai `$HOME` pour le relais
  d'egress, la CA `~/.mitmproxy/` et le binaire sous `~/.local/bin`. D'où une
  redirection **ciblée** et non un `HOME` détourné.
- **Sauvegarder puis restaurer en `trap EXIT`** (voie 2 du ticket). Elle laisse
  une fenêtre d'écrasement grande ouverte — toute la durée du canari, pendant
  laquelle un pilote en vol s'authentifie avec le leurre — et elle exige d'écrire
  une copie du vrai jeton ailleurs sur le disque : on ajoute une surface là où
  l'on prétend en retirer une.
- **Exposer le chemin par une variable nue `PILOT_GH_TOKEN_FILE`**, sur le modèle
  de `PILOT_LOG_DIR`. Ça donnerait à l'environnement du **service** un levier sur
  l'endroit où un dispatch **réel** écrit sa credential GitHub. Voir ci-dessous.
- **Comparer par `cmp` « octet pour octet »**, le test d'acceptation littéral du
  ticket. Juste comme *exigence*, faux comme *prédicat*.

## Solution

Quatre décisions, et chacune referme un piège que la précédente ouvre.

**1. La redirection passe par la variable INTERNE, et le défaut ne bouge pas.**
`dispatch-lib.sh` déclare désormais son chemin de façon conditionnelle, à un site
unique :

```bash
: "${_PILOT_GH_TOKEN_FILE:=$HOME/.mika/pilot-gh-token}"
```

Un harnais qui a posé la variable **avant** de sourcer la conserve. Le préfixe `_`
n'est pas décoratif : il est ce qui satisfait la contrainte structurellement. Ce
nom n'est ni dans `SANDBOX_ENV_CORE_ALLOWLIST` ni dans `PILOT_DISPATCH_ENV`
(`crates/mika-agent/src/skills/executor.rs:345`, exactement deux éléments :
`PILOT_MAX_TURNS`, `PILOT_LOG_DIR`), donc il **ne peut pas** atteindre le child
d'un dispatch réel — celui-ci est bâti par `sandboxed_pilot_env` (`env_clear()` +
allowlist positive), et seul `inject_pilot_dispatch_env` relaie des noms nus.
*Un knob de harnais n'est pas un réglage opérateur, et le nom doit le dire.*

**2. `:=` plutôt qu'une réaffectation après le `source`, à cause d'un
sous-processus.** Le mode `--show-args` du canari fait
`bash -x -c "source '$DISPATCH_LIB'; _run_pilot_sandboxed /bin/true"` — un bash
**neuf** qui re-source. Une affectation posée dans le parent *après* le `source`
ne l'atteindrait pas, et ce mode appelle bel et bien le bac à sable, donc le
staging. Avec `:=` plus un `export` **avant** le `source`, la valeur traverse par
héritage d'environnement : un seul site de pose, les cinq modes couverts.

**3. Le verdict est à TROIS valeurs, jamais booléen.** Un dispatch qui spawne
pendant le canari restage **légitimement**, et un jeton d'installation GitHub App
tourne à l'heure : le contenu peut changer sans faute. Le discriminant est
l'empreinte du **leurre**, une constante connue du script :

```
ok:           le fichier est octet pour octet identique
VIOLATION:    il porte le leurre du canari — mika#2578 a régressé, exit non nul
INCONCLUSIVE: il a changé SANS porter le leurre — restaging concurrent ou
              rotation App : pas une faute, statut d'origine préservé
```

Un `cmp` nu crierait à la violation à chaque rotation, **et une garde qui crie à
tort finit muselée.** Corollaire de même famille : un hôte sans `sha256sum` ni
`cksum` rend `INCONCLUSIVE`, jamais `ok:` — *une vérification qui n'a pas pu
regarder ne doit pas se lire comme un succès.*

**4. L'auto-contrôle observe le chemin par DÉFAUT, écrit en littéral.** Lire
`$_PILOT_GH_TOKEN_FILE` dans le verdict comparerait le fichier redirigé à
lui-même et rendrait `ok:` pour toujours. **Une garde tautologique ne se distingue
pas d'une garde qui marche** : c'est une assertion structurelle, pas une
convention de relecture.

## Prévention — et chaque piège de test a produit un faux vert mesuré

La garde structurelle vit dans la PART D de
`skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh`.

**La garde est fail-closed sur la FORME, pas une denylist de commandes.** Deux
termes, et aucun ne subsume l'autre — les deux ont été vus rouges **seuls** :

| terme | ce qu'il refuse | contrôle qui le prouve porteur |
|---|---|---|
| A | une forme mutante sur une ligne nommant le chemin hôte | `echo x > "$HOME/.mika/pilot-gh-token"` apparie la forme de lecture `echo`, donc passe B |
| B | une ligne nommant le chemin hôte qui n'apparie **aucune** forme de lecture déclarée | `wc -c < "$HOME/.mika/pilot-gh-token"` est une lecture inconnue, donc passe A |

Le terme A seul est une denylist de noms de commandes, que la prochaine idiome
d'écriture franchit. Le terme B seul laisse passer toute écriture déguisée en
lecture déclarée. **L'asymétrie qui tranche le sens du fail-closed :** un rouge à
tort coûte un run de CI et une ligne de déclaration ; un vert à tort coûte la
credential vivante de l'opérateur, irréversiblement. C'est la règle de mika#2520
prise **en sens inverse** — là un signal illisible doit *conserver* parce que
l'action détruisait du travail ; ici il doit *refuser* parce que l'action **est**
l'écriture. *L'arbitrage est local et ne se transporte pas.*

**L'allowlist d'exemption est livrée vide ET consommée avant les deux termes.**
Mesuré : avec une écriture inscrite dedans, les deux termes passent au vert et le
pin d'égalité à vide rougit **seul**, en nommant la ligne exemptée. Exempter est
donc un acte visible, jamais un contournement discret (doctrine mika#2201 : *on
déclare, on n'allowliste pas*). Un pin posé **à côté** d'une variable que rien ne
lit serait un PASS qui ne mesure rien.

**Le plancher d'anti-vacuité et le pin attrapent deux défauts différents**, et le
commentaire qui l'affirmait a dû être corrigé par la mesure : une entrée
d'exemption fait passer le compte de 6 à 5, ce qui **franchit** le plancher de 4.
Le plancher attrape le chemin **renommé** sous le scan, pas une exemption. Vu
rouge en renommant le chemin : les deux termes passent au vert (population vide)
et seuls le plancher et l'assertion de littéral rougissent.

**Le contrôle négatif ne peut pas reproduire le défaut.** Muter le canari en
place pour le voir écrire écraserait le vrai jeton de l'hôte — exactement le dégât
que le correctif ferme. Le geste qui marche est un **miroir du dépôt** sous
`.pilot-scratch/` reproduisant la disposition relative (`scripts/`,
`skills/bundled/_shared/tests/`), de sorte que le `REPO_ROOT` calculé depuis
`BASH_SOURCE` résolve dans le miroir : la **vraie** suite tourne alors contre un
canari muté, sans duplication de prédicat et sans toucher l'hôte. Contrôle positif
obligatoire d'abord — le miroir non muté doit être vert, sans quoi un rouge ne dit
rien sur le prédicat.

**Ce que ça n'achète pas.** La garde est structurelle et le verdict est un
détecteur d'**exploitation** : son régime attendu est `ok:` à chaque exécution, et
**son silence ne prouve rien tant que personne ne lance le canari.** Son contrôle
positif est la ligne d'attribution qui nomme l'état du fichier hôte avant le
lancement — elle montre qu'il a regardé un fichier plutôt que rien.

## Le canal qui reste ouvert, nommé

`_ensure_pilot_helper` lance le daemon mitmdump par `nohup … &` **sans `env -i`** :
il hérite de l'environnement complet de son lanceur, leurre compris, et survit à
tous les dispatches suivants. Très largement masqué — l'addon **préfère le
fichier** à son env, et un dispatch réel restage avant chaque spawn — donc la
fenêtre résiduelle est « daemon relancé par le canari **et** fichier absent ».
Réchauffer les daemons **avant** d'exporter les leurres la borne sans la fermer ;
la fermer demande de retirer au daemon un repli **délibéré** de production, c'est
à dire un arbitrage de canal. Même classe, non fermée : `install -Dm755` du binaire
proxy, que le canari écrase par la version de **sa branche** pour chaque dispatch
suivant.

## Liens

- `docs/plans/2026-09-29-001-fix-2578-canary-necrit-plus-le-jeton-hote-plan.md`
- [Le staging côté hôte et son absence côté bac à sable](exec-handler-gh-token-injection.md)
- [`set -x` fuit les secrets dans la trace](bash-set-x-leaks-secrets-in-trace-and-callback-2026-04-30.md)
- `docs/solutions/cross-repo-patterns/pilot-concurrency-shared-resources-2026-09-03.md` § 6 —
  le chemin de staging partagé, déjà nommé comme « changement nommé » pour N>1

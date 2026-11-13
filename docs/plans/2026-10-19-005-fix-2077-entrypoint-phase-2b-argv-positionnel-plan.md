# mika#2077 — vol 1 : l'entrypoint Phase 2b passe l'argv en paramètres positionnels

**Ticket :** senara-solutions/mika#2077 (parapluie p3 du substrat de dispatch)
**Ligne traitée :** « L'entrypoint Phase 2b re-cite l'argv avec `printf '%q'` » — reprise de mika#2057
**Type :** fix
**Labels du ticket :** `bug`, `p3-nice-to-have`, `ready`, `dispatch:loop`

---

## Contexte

mika#2077 est un ticket parapluie : il porte quatre lignes indépendantes, il se
ferme quand la liste est vide, et chaque ligne cochée l'est avec sa preuve. Les
quatre ont été vérifiées sur l'arbre courant avant d'écrire ce plan — aucune
n'a été refermée par un travail intercurrent :

| ligne | état mesuré sur `HEAD` (`c043781c`) |
|---|---|
| Six suites de `_shared/tests/` ne s'exécutent pas (mika#2058) | **toujours vrai, mais requalifié.** mika#2761 a introduit `scripts/suite-wiring.tsv` : 14 suites de `_shared/tests/` y sont déclarées `unwired`, c'est-à-dire *« tourne hors-ligne, et rien ne la lance »*. Le backlog est désormais nommé et mesurable (`grep -c unwired`) mais toujours non câblé. |
| L'entrypoint Phase 2b re-cite l'argv avec `printf '%q'` (mika#2057) | **toujours vrai**, `dispatch-lib.sh:2792` (`quoted_argv=$(printf ' %q' "$@")`) interpolé en `dispatch-lib.sh:2862` dans un script exécuté par `$sandbox_entrypoint_prefix`. |
| `_launch_revise_pilot` hors du chemin de terminaison (mika#2033) | **toujours vrai**, `dispatch-lib.sh:9125-9158` : le stdout du pilote de revise est écrit dans un `mktemp` puis supprimé sans qu'aucun champ JSON n'en soit lu. |
| `sed` retire une ligne d'un `PIPELINE FAILURE` multi-ligne (mika#2032) | **toujours vrai**, `dispatch-lib.sh:12389`. Dix sites au moins composent un bloc `PIPELINE FAILURE:` multi-ligne dans `RESULT`. |

Les quatre corrections sont indépendantes — fichiers voisins, aucune dépendance
d'ordre. Le découpage en vols est donc gouverné par la seule borne de taille.

### Le défaut traité par ce vol

`_run_pilot_sandboxed` a deux chemins. Le **Phase 2a** (repli, pas de proxy
d'egress) passe l'argv du pilote en **paramètres positionnels** :

```bash
-- /bin/sh -c "$_PILOT_SECRET_PROLOGUE
exec \"\$@\"" mika-pilot-sandbox "$@"
```

Aucune couche de citation n'est ajoutée : le shell reçoit les arguments
verbatim dans `$@`.

Le **Phase 2b** (chemin nominal, avec shim d'egress) fait l'inverse — il
sérialise l'argv en texte et l'interpole dans le corps du script :

```bash
quoted_argv=$(printf ' %q' "$@")
...
-- "$sandbox_entrypoint_prefix" -c "
...
trap 'kill \$_shim_pid 2>/dev/null' EXIT
$quoted_argv
"
```

`printf '%q'` est un builtin **bash**, et sa sortie est du **bash**. Pour tout
argument contenant un caractère non imprimable — saut de ligne, tabulation,
`\r` — il émet la forme `$'...'`, une extension ANSI-C que `dash` n'interprète
pas : `dash` y voit l'expansion de la variable vide `$` suivie d'une chaîne
entre apostrophes. Le script reçoit alors un argument différent de celui qui
lui était destiné, silencieusement.

Le défaut est **latent**, et c'est précisément ce qui le rend coûteux : il
n'attend qu'un `--command` ou un `-- "@fichier"` multi-ligne pour se manifester
une fois, sur une seule dispatch, sans aucun marqueur qui le nomme. Le pilote
démarre avec un prompt tronqué ou corrompu, et la session est diagnostiquée
comme une dérive du modèle.

`$sandbox_entrypoint_prefix` désigne le shell retenu par la résolution amont ;
rien ne garantit que ce soit bash, et le Phase 2a documente déjà en toutes
lettres que l'absence de seconde couche de citation est une propriété
recherchée (`dispatch-lib.sh:2869-2871`). Ce vol aligne 2b sur 2a.

### Pourquoi ce vol, et pas un autre

Les quatre lignes ne tiennent pas ensemble sous la borne par vol (voir
*Taille estimée*). Celle-ci est retenue en premier parce que c'est la seule
dont la correction est **entièrement structurelle** : la forme cible existe
déjà vingt lignes plus bas, elle est commentée, et la transformation est une
réécriture mécanique dont un détecteur peut vérifier le résultat. Les trois
autres demandent chacune une décision de conception (une sentinelle de bloc
pour mika#2032, une discipline de portée sur des globales partagées pour
mika#2033, un arbitrage défaut-par-défaut pour mika#2058) qui mérite son
propre vol et son propre jugement d'architecte.

---

## Contrainte structurelle : `exec` est interdit en position finale

Le script 2b installe `trap 'kill $_shim_pid 2>/dev/null' EXIT` pour tuer le
proxy d'egress. Un `exec "$@"` remplacerait l'image du shell et le trap ne
tournerait jamais : le shim survivrait au pilote. mika#2057 nomme ce point
explicitement.

La forme retenue est donc, en fin de script :

```sh
"$@"
_rc=$?
exit $_rc
```

`exit N` déclenche le trap `EXIT` puis sort avec `N` ; le `kill` du trap
n'écrase pas le statut. Le contrat « le statut de sortie de
`_run_pilot_sandboxed` est celui du pilote » (déjà tenu par `_sandbox_rc`,
`dispatch-lib.sh:2927-2944`) est préservé.

**Non-régression à noter :** le script actuel n'`exec` pas non plus — il exécute
`$quoted_argv` comme commande simple. Le pilote est déjà un enfant du shell
PID 1, et le relais de signaux est déjà assuré par `--die-with-parent`. Ce vol
ne change donc rien à la topologie des processus ; il ne change que la façon
dont les arguments y arrivent.

---

## Implémentation

### S1 — extraire le corps du script 2b dans une fonction

Nouvelle fonction `_pilot_phase2b_entrypoint_script`, posée immédiatement avant
`_run_pilot_sandboxed`. Elle émet le corps du script sur stdout et ne lit que
des globales déjà résolues au moment de l'appel (`_PILOT_SECRET_PROLOGUE`,
`_PILOT_EGRESS_PROXY_BIN`, `_PILOT_EGRESS_TCP_PORT`, `_PILOT_EGRESS_SOCK`).

Cette extraction n'est pas cosmétique : **elle est ce qui rend le script
testable hors bwrap.** Tant que le corps n'existe que comme littéral à
l'intérieur d'un appel `bwrap`, aucun test hors-ligne ne peut l'exécuter, et la
seule assertion possible serait un grep sur le source — c'est-à-dire un test
qui mesure la forme du code sans jamais mesurer son comportement. L'extraction
convertit la propriété « l'argv traverse intact » d'une affirmation en une
mesure.

Le corps est transposé verbatim, à une seule modification près : la ligne
`$quoted_argv` devient le bloc `"$@"` / `_rc=$?` / `exit $_rc` ci-dessus. Les
échappements existants (`\$!`, `\$_i`, `\"127.0.0.1\"`) sont conservés tels
quels — c'est la même chaîne, dans le même contexte de guillemets doubles bash.

### S2 — le site d'appel passe l'argv en positionnels

À `dispatch-lib.sh:2841` et `:2862-2863`, la queue de l'invocation `bwrap`
devient :

```bash
            -- "$sandbox_entrypoint_prefix" -c "$(_pilot_phase2b_entrypoint_script)" \
               mika-pilot-sandbox "$@" || _sandbox_rc=$?
```

`mika-pilot-sandbox` est le `$0` du shell interne — strictement le même nom
que celui déjà employé par Phase 2a, pour que les deux chemins soient
indiscernables dans un `ps` ou un message d'erreur du shell.

La déclaration `local quoted_argv` et son affectation disparaissent.

### S3 — le commentaire dit pourquoi, pas quoi

Le commentaire de `dispatch-lib.sh:2789-2790` (« Uses `printf '%q'` for
shell-safe re-quoting ») est remplacé. Le texte cible nomme les trois choses
qu'un relecteur doit savoir pour ne pas défaire ce vol :

1. `printf '%q'` émet du bash (`$'...'`), le shell d'entrypoint n'est pas
   garanti bash, et la corruption est silencieuse ;
2. `exec` est interdit ici à cause du trap, contrairement à Phase 2a — donc la
   symétrie avec 2a s'arrête à l'avant-dernière ligne ;
3. réintroduire une couche de citation est exactement le défaut mika#2057.

### S4 — le détecteur

Nouvelle suite `skills/bundled/_shared/tests/test_pilot_phase2b_argv.sh`, en
quatre parties.

**PART A — comportemental (le cœur).** Source `dispatch-lib.sh` en mode
bibliothèque, pose les quatre globales sur des valeurs de test (un
`_PILOT_EGRESS_PROXY_BIN` pointant sur un script python trivial dans un
`mktemp -d`, un `_PILOT_SECRET_PROLOGUE` vide, un port libre), puis exécute :

```sh
/bin/sh -c "$(_pilot_phase2b_entrypoint_script)" mika-pilot-sandbox <cmd> <args>
```

Trois assertions :

- **A1** — un argument contenant un saut de ligne et une apostrophe traverse
  **octet pour octet**. C'est le cas que mika#2057 réclame : exactement la forme
  que `%q` échappe en `$'...'`. La commande cible écrit son `$1` dans un
  fichier, le test compare au littéral attendu.
- **A2** — un argument contenant une tabulation et un `$(...)` littéral
  traverse intact et **n'est pas réévalué**. Contrôle négatif de la substitution
  de commande : sous l'ancienne forme, un argv re-cité puis ré-interprété est la
  porte par laquelle une expansion non voulue entrerait.
- **A3** — le statut de sortie du pilote est le statut de sortie du script. La
  commande cible sort en `7` ; le `sh` doit sortir en `7`. C'est l'assertion qui
  protège le `exit $_rc` contre une simplification en `"$@"` nu suivie d'une fin
  de script (qui rendrait le statut du dernier `trap`, pas celui du pilote).

La boucle d'attente du shim plafonne à 20 × 0,05 s ; le test absorbe donc au
pire une seconde par cas, sans bwrap, sans réseau et sans proxy réel.

**PART B — structurel.** Sur le fichier `dispatch-lib.sh` tracké :

- **B1** — les deux chemins 2a et 2b se terminent par un passage positionnel :
  `mika-pilot-sandbox "$@"` apparaît exactement deux fois dans du code.
- **B2** — contrôle négatif, **ancré sur le site et non sur la valeur** : aucune
  ligne de *code* (commentaires exclus) ne contient `printf ' %q'`. Le prédicat
  porte sur l'entrypoint du sandbox, pas sur l'existence du littéral `%q`
  n'importe où — la doctrine mika#2309 : un prédicat sur la valeur attrape des
  tiers innocents et finit en allowlist, un prédicat sur le site borne ce qu'il
  doit border.

**PART C — anti-vacuité.** Copie `dispatch-lib.sh` dans un `mktemp -d`,
réintroduit la forme `quoted_argv=$(printf ' %q' "$@")`, relance B1 et B2 sur
la copie, et **exige qu'elles échouent**. Sans cette partie, B1/B2 sont deux
assertions dont rien ne prouve qu'elles savent dire non — la classe exacte que
mika#1772 a trouvée en câblant `test-dispatch-lib.sh` (« une assertion passante
sans rien prouver »).

**PART D — préconditions.** La suite saute proprement (exit 0, message explicite)
si `python3` est absent. Elle n'exige ni bwrap, ni réseau, ni token : elle est
wirable sur un runner hébergé par construction.

### S5 — câblage CI

mika#2761 fait d'une suite non atteinte par un workflow une erreur de lint
(`suite-wiring-lint`). La suite doit donc être câblée **dans le même commit** —
une entrée `unwired` dans `scripts/suite-wiring.tsv` serait l'aveu que le
détecteur qu'on vient d'écrire ne tourne pas.

- `Makefile` : cible `test-pilot-phase2b-argv`, plus la ligne correspondante
  dans `test`.
- `.github/workflows/ci.yml` : étape nommée dans le job `Check`, sur le modèle
  des étapes voisines (`gh usable inside the pilot sandbox`, lignes 133-149),
  avec le commentaire qui dit pourquoi elle est là.
- `scripts/suite-wiring.tsv` : **aucune entrée** — la suite est câblée, et le
  lint rougit sur une entrée pour une suite que CI atteint.

### S6 — ce que ce vol ne touche pas

- Le chemin Phase 2a : il est déjà correct, et l'aligner n'aurait rien à aligner.
- `_PILOT_SECRET_PROLOGUE` : transposé verbatim, aucune modification.
- Les trois autres lignes de mika#2077 : voir *Vols suivants*.
- `scripts/suite-wiring.tsv` : les 14 entrées `unwired` existantes restent. Les
  retirer est le vol 4, et les retirer ici ferait entrer dans cette PR les
  défauts que leur câblage fera surgir — ce que mika#2058 annonce comme certain.

---

## Fire-Disposition

Ce plan livre un détecteur : les parties B1, B2 et C de
`test_pilot_phase2b_argv.sh` ont pour fonction primaire de signaler une
violation, et leur chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) — exception nommée en allowlist, avec un ensemble
d'exceptions vide par construction.**

Détail d'implémentation :

- **L'allowlist est vide, et elle l'est parce qu'il n'y a rien à y mettre.**
  La population que B2 refuse compte exactement une occurrence sur `HEAD`
  (`dispatch-lib.sh:2792`), et S2 la supprime **dans le même commit** que celui
  qui pose le détecteur. Le détecteur atterrit donc **armé et vert**, sans
  `#[ignore]`, sans exception, sans dette.
- **L'assertion auto-nettoyante est le détecteur lui-même, exprimé en compte
  exact et non en borne.** B2 exige `0` ligne de code portant `printf ' %q'` et
  B1 exige **exactement** deux occurrences de `mika-pilot-sandbox "$@"` — pas
  « au moins ». Un troisième chemin de sandbox ajouté demain, correct ou non,
  fait rougir B1 et force son auteur à venir déclarer ici ce qu'il fait. Une
  borne inférieure l'aurait laissé passer en silence.
- **Pourquoi pas (b), livrer désarmé :** il n'y a aucune violation à amnistier,
  donc rien à différer. Un détecteur désarmé sur une population vide est un
  détecteur qui ne dira jamais rien.
- **Pourquoi pas (c), halte-et-remontée :** le périmètre est entièrement résolu
  par ce plan ; rien n'appelle un cadrage opérateur.
- **Si, à l'implémentation, B2 trouve une seconde occurrence** non anticipée par
  ce plan : elle est traitée comme la première (conversion en positionnels) si
  elle est sur un chemin d'entrypoint, et elle **bascule le vol en (c)** si elle
  ne l'est pas — une occurrence hors entrypoint signifierait que le prédicat de
  site est mal posé, ce qui est une décision de conception, pas un ajustement.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (S1 extraction, S2 site d'appel, S3 commentaire) | 55 |
| tests (`skills/bundled/_shared/tests/test_pilot_phase2b_argv.sh`) | 150 |
| `Makefile` (cible + ligne dans `test`) | 4 |
| `.github/workflows/ci.yml` (étape nommée + commentaire) | 10 |

Total estimé : 219 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 219 < 279, et les deux seuils (1000 par PR,
279 par vol) sont respectés par le plus serré des deux. La marge de 60 lignes
est réelle et non optimiste : les 55 lignes de `dispatch-lib.sh` sont en
majorité une **transposition verbatim** du corps existant dans une fonction —
le diff est large, le code nouveau se limite au bloc `"$@"` / `_rc` / `exit` et
au commentaire S3. La suite de test est la seule part réellement écrite de
zéro, et son gabarit (PART A/B/C/D, `mktemp -d`, saut sur précondition
manquante) est celui des quinze suites voisines de `_shared/tests/`.

### Vols suivants

Les trois lignes restantes de mika#2077, chacune mergeable seule, chacune sous
la borne. Elles ne sont pas phasées dans ce vol : elles en sont distinctes.

- **Vol 2 — `sed` retire une ligne d'un `PIPELINE FAILURE` multi-ligne**
  (mika#2032). Demande une sentinelle de bloc et la migration d'une dizaine de
  sites producteurs vers un helper unique ; ≈ 275 lignes, à re-mesurer au
  grooming.
- **Vol 3 — `_launch_revise_pilot` hors du chemin de terminaison** (mika#2033).
  Demande de capturer le stdout du pilote de revise et de router un
  `status: terminated` vers `_classify_terminated_session`, sous une discipline
  de portée stricte : cette fonction lit six globales (`SUBTYPE`,
  `TERMINATION_REASON`, `SESSION_ID`, `TURNS`, `DURATION`, `API_ERROR_STATUS`)
  plus le mémo `_HALT_SUBTYPE_RESOLVED`, toutes appartenant au pilote principal.
  Le levier est le *dynamic scoping* de bash — les déclarer `local` dans
  `_launch_revise_pilot` les masque pour l'appelée et les restaure à la sortie —
  mais c'est une décision de conception qui mérite son propre jugement.
  ≈ 210 lignes.
- **Vol 4 — câbler les suites `unwired` de `_shared/tests/`** (mika#2058).
  Le câblage lui-même est petit (≈ 40 lignes de `Makefile` et de `ci.yml`, moins
  14 lignes de `suite-wiring.tsv`) ; ce qui le rend volumineux est le mandat
  explicite de mika#2058 — *« les défauts que le raccordement fait surgir sont
  traités ou ticketés, s'attendre à en trouver, c'est le but »*. À découper
  lui-même par paquets de suites au grooming.

Tracked in: senara-solutions/mika#2077

---

## Contrat de vérification

| ce qui est vérifié | comment |
|---|---|
| Un argument multi-ligne traverse l'entrypoint 2b intact | PART A1, comparaison octet pour octet |
| Un argument portant une tabulation et un `$(...)` littéral n'est pas réévalué | PART A2 |
| Le statut du pilote est le statut du script 2b | PART A3, sortie en 7 |
| Les deux chemins de sandbox passent l'argv en positionnels | PART B1, compte exact = 2 |
| Aucun code ne re-cite l'argv avec `printf ' %q'` | PART B2, compte exact = 0 |
| B1 et B2 savent dire non | PART C, sur une copie régressée |
| Le détecteur tourne en CI | étape nommée dans `ci.yml`, et absence d'entrée dans `suite-wiring.tsv` |
| Rien d'autre du substrat n'a bougé | `make test-dispatch-lib`, `make test` |
| Pipeline | `cargo test`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

---

## Ordre de commit

1. S1 + S2 + S3 — `dispatch-lib.sh` : extraction, passage positionnel, commentaire.
2. S4 — la suite de test, quatre parties.
3. S5 — `Makefile` + `ci.yml`.

Le plancher est le commit 1 : il clôt le défaut. Les commits 2 et 3 le rendent
irréversible. Un vol qui s'arrêterait après le commit 1 aurait corrigé le défaut
sans poser la garde — c'est une PR recevable mais incomplète, et c'est la seule
troncature acceptable.

---

## Definition of Done

- `dispatch-lib.sh` ne contient plus aucune re-citation de l'argv par
  `printf '%q'` dans du code.
- Le corps du script d'entrypoint 2b est une fonction appelable, exécutable hors
  bwrap.
- La suite `test_pilot_phase2b_argv.sh` existe, passe, et sa partie anti-vacuité
  démontre qu'elle rougit sur une régression réintroduite.
- La suite est atteinte par `.github/workflows/ci.yml` et ne figure pas dans
  `scripts/suite-wiring.tsv`.
- `make test-dispatch-lib`, `make test`, `cargo test`, `cargo clippy`,
  `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- Le corps de la PR ne porte **pas** de `Closes #2077` : trois lignes du
  parapluie restent ouvertes. Il coche la ligne « entrypoint Phase 2b » avec le
  numéro de la PR comme preuve, et porte `Tracked in: senara-solutions/mika#2077`.

## Acceptance criteria

Transcrits de la section « Correction attendue » de mika#2057, dont cette ligne
du parapluie est la reprise :

- **AC1** — Phase 2b passe l'argv en paramètres positionnels comme Phase 2a.
  L'`exec` reste interdit en position finale à cause du `trap ... EXIT` qui tue
  le shim : la forme est `"$@"` sans `exec`, suivie d'une propagation explicite
  du statut.
- **AC2** — Un test couvre un argument contenant un caractère que `%q` échappe
  en `$'...'`.

Dérivés du contrat de vérification de ce plan :

- **AC3** — Le statut de sortie du pilote reste le statut de sortie de
  `_run_pilot_sandboxed`, vérifié par une assertion et non par lecture du code.
- **AC4** — Le détecteur structurel est ancré sur le **site** d'entrypoint et
  non sur la valeur `%q`, et il porte un contrôle d'anti-vacuité.
- **AC5** — Le détecteur s'exécute en CI sur chaque PR.

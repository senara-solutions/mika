# feat(infra) — l'egress de la sandbox n'injecte le jeton du moteur que sur une requête de pilote attestée (mika#2716)

**Ticket :** senara-solutions/mika#2716
**Jalon :** Substrat de boucle
**Labels :** `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`

---

## 1. Pourquoi

`scripts/mika-pilot-github-auth-addon.py:143-145` retire tout `Authorization` /
`Proxy-Authorization` venu du client, puis pose le jeton lu côté hôte :

```python
    for header_name in ("authorization", "proxy-authorization"):
        flow.request.headers.pop(header_name, None)
    flow.request.headers["Authorization"] = _auth_header_for(host, token)
```

Le `pop` est inconditionnel et l'injection ne lit **rien** de ce que le client
présentait. Conséquence : toute requête vers `api.github.com` / `github.com`
émise depuis une sandbox de pilote part sous l'identité du moteur, **quel que
soit le jeton du client**. C'est le substrat de mika#2713 : un test unitaire
posant `GH_TOKEN="fake-token"` a produit 29 commentaires et un label
`operator-gated` sur le ticket **fermé** mika#2636. Un `gh` nu aurait rendu 401.

mika#2713 a fermé la classe côté test (couture `gh_process::run_gh`, scan
structurel, plancher de numéro de fixture). Ce ticket-ci est l'autre moitié.

## 2. Rectification que la lecture du code impose au corps du ticket

Le corps propose un en-tête dédié `X-Mika-Egress-Sentinel` posé par « le client
`gh` des pilotes ». **Cette forme n'est pas implémentable sans un shim de PATH**,
et une lecture de `scripts/mika-pilot-egress-proxy` plus de la rétrospective
mika#2572 montre qu'elle n'est pas non plus nécessaire :

- `gh` n'offre aucun canal d'en-tête arbitraire par variable d'environnement.
  Seul `gh api -H` existe, par appel. Poser un en-tête sur *toutes* les requêtes
  `gh` demanderait un wrapper `gh` sur le PATH de la sandbox.
- `git` l'offre (`http.extraHeader` via `GIT_CONFIG_KEY_n`), donc les deux
  clients ne partagent **aucun** canal d'en-tête commun : il faudrait deux
  mécanismes.
- Et surtout : un shim `gh` serait emprunté **aussi** par le `cargo test`. Un
  secret posé par un wrapper atteste *la sandbox*, pas *le chemin légitime* —
  il n'attesterait donc rien d'utile pour mika#2713.

**Le seul canal que `gh` et `git` posent tous deux déjà est le credential
lui-même.** Et depuis mika#2572 la sandbox porte précisément
`--setenv GH_TOKEN "proxy-managed-no-secret"`
(`dispatch-lib.sh:2074`) — un placeholder statique et public, requis parce que
`gh` lit sa config locale avant toute émission réseau
(`docs/solutions/dev-loop/gh-verifie-lauth-localement-avant-le-reseau-il-faut-un-placeholder.md`).

**Décision : la sentinelle est le credential que la sandbox présente.** Le
placeholder statique devient une valeur **per-spawn et non devinable**, et
l'addon n'injecte le jeton de l'hôte que si le credential présenté est une
sentinelle attestée côté hôte.

Pourquoi cette forme ferme réellement la classe mika#2713 : la signature de
l'incident est qu'un test **écrase** `GH_TOKEN`. Un test qui pose
`GH_TOKEN="fake-token"` détruit la sentinelle de sa propre main et se fait
refuser. Aucun autre canal n'a cette propriété — un en-tête dédié survivrait
intact à l'écrasement du jeton, et l'incident se reproduirait à l'identique.

### 2.1 Modèle de menace, énoncé pour qu'il ne soit pas sur-promis

La sentinelle voyage dans l'environnement de la sandbox, donc `cat
/proc/self/environ` la révèle **à l'intérieur**. C'est inhérent : pour que `gh`
la pose, elle doit être dans l'environnement de `gh`. Donc :

- Ce que ce ticket ferme : l'**accident** — un processus de la sandbox qui
  remplace le credential en croyant parler à un faux, et atteint la vraie forge.
- Ce qu'il ne ferme pas : un code *malveillant* dans la sandbox, qui lirait la
  sentinelle dans son propre environnement et la rejouerait. Cette moitié reste
  couverte par mika#2056 (la sandbox ne détient jamais le jeton, donc rien ne
  sort de l'egress) et n'est pas le mode de défaillance de mika#2713.
- L'argv de bwrap (`--setenv`) est lisible côté **hôte** via
  `/proc/<pid>/cmdline`, mais `--unshare-pid` + `--proc /proc`
  (`dispatch-lib.sh:2167,2182`) fait que `/proc` ne montre que les processus de
  la sandbox. Un lecteur hôte de cet argv possède déjà
  `~/.mika/pilot-gh-token`, donc le vrai jeton : la sentinelle ne lui apporte
  rien. C'est la raison pour laquelle le canal `--setenv` existant est conservé
  plutôt que déplacé vers `_PILOT_SANDBOX_SECRET_ALLOWLIST` — ce déplacement
  ferait rougir en plus la garde « pas de `/run/mika-pilot-secrets/GH_TOKEN` »
  de `test-pilot-github-token-not-in-sandbox.sh`, pour un gain nul contre le
  modèle de menace réel.

## 3. Découpage en vols

Le ticket exige que le mode de défaillance soit traité **avant** tout
déploiement de la coupure :

> un bug d'injection ferait silencieusement 403 **tous** les appels légitimes et
> la boucle autonome s'arrêterait sans diagnostic.

Ce découpage est donc l'ordre que le ticket prescrit, et non une réduction de
confort : **on dote d'abord, on atteste, puis on arme.**

| vol | contenu | risque boucle |
|---|---|---|
| **1 — celui-ci** | la sandbox porte une sentinelle per-spawn attestée côté hôte ; la sonde la lit ; l'interrupteur opérateur existe ; l'addon est **inchangé** | **nul** — comportement observable identique, aucun 403 possible |
| **2 — suite** | l'addon **exige** la sentinelle : 403 à deux motifs distincts (absente / invalide), log `DENY`, purge TTL du registre | réel, et c'est pourquoi il arrive après la sonde |

Chaque vol est mergeable seul. Le vol 1 laisse la CI verte et la boucle
inchangée ; le vol 2 n'a plus qu'à ajouter un lecteur devant un canal déjà
attesté en production.

Tracked in: senara-solutions/mika#2716

L'implémenteur du vol 1 ouvre le ticket de suivi du vol 2 **avant** d'ouvrir sa
PR et met son numéro en `Tracked in:` du corps (règle mika#2701) ; à défaut,
mika#2716 reste ouvert et porte le reste.

Le découpage est ici **volontaire**, non imposé par la borne : le vol 1 mesuré
tient sous elle (§ 9). Il est dicté par l'ordre que le ticket prescrit — doter et
attester avant d'armer — et non par un total qu'il faudrait faire passer.

## 4. Périmètre du vol 1 — conception

### 4.1 `skills/bundled/_shared/dispatch-lib.sh` — produire la sentinelle

Nouveau, à côté de `_stage_pilot_gh_token` (l.1554) et appelé depuis le même
point (l.1980) :

```bash
_PILOT_EGRESS_SENTINEL_DIR="$HOME/.mika/pilot-egress-sentinels"
_PILOT_EGRESS_SENTINEL_PREFIX="proxy-managed-no-secret"
_PILOT_EGRESS_SENTINEL=""   # global, rempli par _stage_pilot_egress_sentinel
```

`_stage_pilot_egress_sentinel()` :

1. Si `MIKA_PILOT_EGRESS_SENTINEL` vaut `0`, poser
   `_PILOT_EGRESS_SENTINEL="$_PILOT_EGRESS_SENTINEL_PREFIX"` — le littéral nu
   d'avant ce ticket — et retourner. C'est l'interrupteur opérateur.
2. Sinon, tirer 16 octets (`openssl rand -hex 16`, repli
   `od -An -tx1 -N16 /dev/urandom`) et composer
   `<prefix>-<32 hex>`.
3. `mkdir -p` le registre en `0700`, y écrire `<32 hex>` comme **nom de
   fichier** vide en `0600` (`umask 077`). Un répertoire, et non un fichier
   unique : le mitmdump est long-lived et partagé, donc deux pilotes
   concurrents doivent être valides **en même temps** — un fichier unique
   écrasé par le second spawn ferait 403 au premier dès le vol 2.
4. **Fail-open**, même posture que `_stage_pilot_gh_token` : toute erreur
   d'écriture retombe sur le littéral nu avec un message sur stderr. Un
   dispatch ne meurt jamais sur ce fichier.
5. Le tirage et l'écriture sont encadrés par la bascule `set +x` / restauration
   de `_stage_pilot_gh_token` — le dispatch tourne sous `set -x` avec
   `BASH_XTRACEFD` et `_emit_callback` renvoie la trace à l'appelant.

Le préfixe est **conservé** à l'identique. C'est ce qui rend chaque garde
existante réparable par un élargissement d'ancrage
(`"$LIT"` → `"$LIT"(-[0-9a-f]{32})?`) au lieu d'une réécriture, et ce qui garde
la ligne du canari lisible par un opérateur.

Le site d'injection (l.2074) devient :

```bash
            --setenv GH_TOKEN "$_PILOT_EGRESS_SENTINEL"
```

Et les deux commentaires d'audit qui affirment aujourd'hui le littéral
(l.1504-1509 et l.1524-1526) disent ce qui est vrai : le nom `GH_TOKEN` porte
une sentinelle d'egress préfixée, jamais une valeur de jeton.

### 4.2 `scripts/verify-no-secret-in-setenv.sh` — l'exemption devient une provenance

La force actuelle du lint est que la valeur est un littéral inspectable
statiquement (`EXEMPT_SETENV_VALUE="proxy-managed-no-secret"`, règle 2
l.235-240). Une variable casse cette propriété — `--setenv GH_TOKEN "$X"`
passerait quel que soit `X`. Elle est restaurée par une garde de **provenance**,
en deux assertions grep-visibles :

1. `GH_TOKEN` gagne une exemption par **nom de variable**, liste fermée :
   `EXEMPT_SETENV_VARS=(_PILOT_EGRESS_SENTINEL)`. Chaque occurrence de
   `--setenv GH_TOKEN` doit porter soit le littéral, soit exactement
   `"$_PILOT_EGRESS_SENTINEL"` — le compte reste par **occurrence**
   (`grep -o | wc -l`, jamais `grep -c`) et exige `_total == _conformes &&
   _total >= 1`, pour les raisons de la leçon mika#2572 § *Un lint qui comptait
   des lignes* (leurre sur la même ligne, concaténation).
2. `_PILOT_EGRESS_SENTINEL` n'est assigné que dans le corps de
   `_stage_pilot_egress_sentinel`, et ce corps ne lit ni `GH_TOKEN` ni
   `MIKA_GITHUB_TOKEN` ni `_PILOT_GH_TOKEN_FILE`. Sans cette seconde assertion,
   `_PILOT_EGRESS_SENTINEL="$GH_TOKEN"` passerait la première et réouvrirait
   mika#2039 en entier.

Deux contraintes que la lecture de `scripts/test-verify-no-secret-in-setenv.sh`
impose à la rédaction de la règle, et qu'il faut connaître **avant** de l'écrire
— les ignorer casse six cas verts aujourd'hui :

- **La règle doit accepter le littéral *et* la variable, pas la variable seule.**
  `gh_placeholder_fixture` (l.223-230) plante `--setenv GH_TOKEN
  "proxy-managed-no-secret"` dans la fixture *seulement si* la source vive ne le
  porte plus — ce qui sera exactement le cas après ce vol. La fixture portera
  donc **deux** occurrences : la plantée (littérale) et la vraie
  (`"$_PILOT_EGRESS_SENTINEL"`). Une règle n'admettant que la variable rendrait
  `_total=2, _conformes=1` et ferait rougir le cas l.232-234, qui attend `exit 0`.
  C'est cette fixture auto-adaptative — et non un confort de rédaction — qui
  rend la liste des formes conformes binaire.
- **Le message de violation doit continuer d'interpoler `$EXEMPT_SETENV_VALUE`.**
  L'assertion l.240 (`real GH_TOKEN: names the placeholder it lost`) cherche la
  chaîne `proxy-managed-no-secret` **dans la sortie du lint**. Le message actuel
  la tient par interpolation (l.242) ; un message réécrit qui ne nommerait plus
  que la variable rougirait cette assertion sans rien dire du pourquoi.

À ces conditions, les six cas GH_TOKEN de cette suite (l.232-269) survivent
**sans migration** : chacun opère sur la fixture plantée. La suite ne reçoit donc
qu'un ajout — la fixture de la nouvelle surface : une copie mutée où la
sentinelle est assignée depuis `$GH_TOKEN`, vue **rouge** avant le correctif.

La forme `--setenv GH_TOKEN "$AUTRE"` est couverte par la règle 2 elle-même
(`_total != _conformes`), et déjà épinglée par le cas `real-gh-token.sh`
(l.236-241) — **pas** par `CRED_NAME_PATTERN` : `GH_TOKEN` est un nom exempté,
donc `is_exempt_name` le fait sortir en `continue` (l.247) avant que la branche
`CRED_NAME_PATTERN` (l.249) soit atteinte. Elle n'a pas besoin de sa propre
fixture, mais la raison n'est pas celle qu'on croirait.

### 4.3 Migration des ancrages co-dépendants — recensés par `grep`, pas de mémoire

Le recensement est `grep -rn "proxy-managed-no-secret"` sur `skills/bundled/_shared/tests/`,
`scripts/` et `Makefile`. Il rend **cinq** fichiers, et la nature de l'ancrage
diffère d'un site à l'autre — c'est cette différence qui compte, parce qu'un
seul des sites échoue **en silence** :

| site | ligne(s) | nature | delta | en CI |
|---|---|---|---|---|
| `tests/test_sandbox_no_secret_in_argv.sh` | 525, 589 (+ prose 26, 188) | assertion d'égalité | littéral → `<prefix>(-[0-9a-f]{32})?` | oui (ci.yml:113) |
| `tests/test-pilot-github-token-not-in-sandbox.sh` | 255 | assertion d'égalité | idem ; les assertions sur le vrai jeton **conservées** | oui (ci.yml:123) |
| `tests/test_sandbox_gh_usable.sh` | 52, 157 | assertion d'égalité | `gh auth token` rend la sentinelle, rc 0 | non (`make test`) |
| `tests/test_sandbox_gh_usable.sh` | **98** | **mutation** | voir ci-dessous — **échoue en silence** | non (`make test`) |
| `scripts/canary-pilot-containment` | 693-696 | `case` à trois états | en gagne un quatrième | non (opérateur) |
| `scripts/test-verify-no-secret-in-setenv.sh` | 223-230 | fixture auto-adaptative | **aucun** — voir § 4.2 | oui (ci.yml:114) |

**Le site 98 est le piège de ce vol.** Il ne vérifie rien : il *fabrique* le
contrôle négatif en retirant la ligne du placeholder de `dispatch-lib.sh` —

```bash
grep -vF -- "--setenv GH_TOKEN \"$PLACEHOLDER\"" "$DISPATCH_LIB" > "$MUTATED_LIB"
```

— pour prouver qu'à défaut de placeholder `gh` s'arrête sur `gh auth login`.
Quand la ligne vive porte `"$_PILOT_EGRESS_SENTINEL"`, ce `grep -vF` ne
correspond plus à rien : il recopie le fichier **intact**, `MUTATED_LIB` devient
identique à l'original, et le contrôle négatif mesure la sandbox *dotée* au lieu
de la sandbox *démunie*. Selon l'assertion attachée il rougira par chance ou
restera **vert et vacant** — la seconde issue est celle qui coûte, parce qu'elle
retire une garde du pipeline sans le dire. Le delta est donc d'ancrer la mutation
sur la **forme** (`perl -ne` / `grep -vE` sur
`--setenv GH_TOKEN "(\$_PILOT_EGRESS_SENTINEL|proxy-managed-no-secret[^"]*)"`),
et d'y adjoindre l'assertion anti-vacuité que le repo exige partout ailleurs :
`MUTATED_LIB` compte **une ligne de moins** que `DISPATCH_LIB`, sinon le test
échoue en nommant la mutation qui n'a rien muté.

Le canari n'est **pas** reportable au vol 2 : son `case` actuel classe toute
valeur autre que le littéral en `LEAK: GH_TOKEN carries a value other than the
placeholder`. Le livrer tel quel produirait un faux LEAK dès le premier
dispatch — exactement le piège que mika#2572 § 5 a documenté et évité.

```sh
case "${GH_TOKEN-__unset__}" in
    proxy-managed-no-secret-*) echo "ok:   gh_token_egress_sentinel" ;;
    proxy-managed-no-secret)   echo "WARN: static placeholder — egress sentinel disabled (MIKA_PILOT_EGRESS_SENTINEL=0 or staging failed; mika#2716)" ;;
    __unset__)                 echo "WARN: GH_TOKEN unset in the sandbox — gh will stop on gh auth login (mika#2572 regressed)" ;;
    *)                         echo "LEAK: GH_TOKEN carries a value other than an egress sentinel (mika#2056 regressed)" ;;
esac
```

Aucune quote simple dans ces messages : le bloc vit dans un payload
`sh -c '...'` entre quotes simples (mika#2572 § *Ce qui n'a pas marché*).

### 4.4 La sonde — nouvelle suite `tests/test_egress_sentinel_staging.sh`

Cible `make test-egress-sentinel-staging`, **ajoutée à `ci.yml`** à côté des
gardes du canal secret (ci.yml:111-114). Elle monte le relais factice
`lib-fake-egress-relay.sh` comme ses voisines. C'est elle, la « sonde qui
atteste la sentinelle au démarrage de la sandbox » que le ticket réclame :

1. **Per-spawn.** Deux appels de `_stage_pilot_egress_sentinel` produisent deux
   valeurs distinctes, toutes deux de la forme `<prefix>-[0-9a-f]{32}`.
2. **Les deux restent valides.** Après deux spawns, le registre contient **deux**
   entrées — la propriété de concurrence sans laquelle le vol 2 404'rait le
   premier pilote.
3. **Permissions.** Registre `0700`, entrées `0600`, et le registre n'est pas
   parmi les binds de la sandbox.
4. **Identité registre ↔ sandbox.** La valeur portée par `--setenv GH_TOKEN`
   est exactement l'entrée écrite dans le registre côté hôte. C'est la seule
   assertion propre à cette suite, et c'est celle dont le vol 2 dépend : il
   comparera le credential présenté aux entrées du registre, donc une divergence
   ici est un 403 là-bas.

   Elle est délibérément mesurée sur l'argv capturé par le relais factice, **non**
   dans un bwrap réel : `test_sandbox_gh_usable.sh:157` (`gh auth token` dans un
   vrai bwrap) et `test_sandbox_no_secret_in_argv.sh:589` (`GH_TOKEN` dans un
   vrai bwrap) montent déjà ce bwrap, et leur ancrage élargi au § 4.3 y mesure la
   **forme** de la sentinelle. Remonter un troisième bwrap ici n'ajouterait
   qu'une troisième occasion de skip sur hôte sans `bwrap` — pour une propriété
   déjà tenue en CI par deux suites.
5. **Interrupteur.** Sous `MIKA_PILOT_EGRESS_SENTINEL=0`, la sandbox porte le
   littéral nu et le registre ne gagne aucune entrée.
6. **Fail-open.** Registre rendu non inscriptible → littéral nu, message sur
   stderr, dispatch poursuivi. Couvert dans le même appel que (5), puisque les
   deux convergent sur la même sortie observable.

### 4.5 Documentation (hors compte de lignes)

- `docs/operator/environment-variables.md` : `MIKA_PILOT_EGRESS_SENTINEL`
  (défaut : activé ; `0` revient au comportement d'avant ce ticket), le registre
  `~/.mika/pilot-egress-sentinels/`, la ligne de canari et ses quatre états, et
  la halte : un `WARN: static placeholder` persistant après un `make deploy`
  signifie que le staging échoue — lire le stderr du dispatch, pas l'addon.
- `docs/solutions/dev-loop/` : note nommant la trouvaille de conception — *une
  sentinelle qui ne voyage pas dans le canal que l'incident écrase n'atteste
  rien*. Avec la table des canaux examinés et pourquoi l'en-tête dédié du corps
  du ticket a été écarté.

## 5. Ordre de commit, et son plancher

Chaque commit laisse l'arbre vert. Le plancher est le commit 2 : s'arrêter avant
lui livre une sentinelle qu'aucune garde n'épingle.

1. `feat(infra)` — `_stage_pilot_egress_sentinel` + le `--setenv` + les
   commentaires d'audit + l'interrupteur. *(Rouge attendu sur
   `verify-no-secret-in-setenv` et les trois suites : c'est la mesure que les
   gardes portaient bien sur la valeur.)*
2. **PLANCHER** — `verify-no-secret-in-setenv.sh` (exemption + provenance) + sa
   fixture, puis les cinq ancrages co-dépendants du § 4.3 — dont la mutation de
   `test_sandbox_gh_usable.sh:98`, celle qui ne rougit pas d'elle-même. Arbre vert.
3. `test(infra)` — la suite de staging + la cible Makefile + l'entrée `ci.yml`.
4. `docs` — variable d'environnement, note de solution.

## 6. Definition of Done

- [ ] `_stage_pilot_egress_sentinel` produit une sentinelle per-spawn de la
      forme `proxy-managed-no-secret-[0-9a-f]{32}`, la stage dans un registre
      `0700` à entrées `0600`, et fail-open sur le littéral nu.
- [ ] `--setenv GH_TOKEN` porte `"$_PILOT_EGRESS_SENTINEL"` et le lint le vérifie
      par nom de variable **et** par provenance.
- [ ] Les cinq ancrages co-dépendants du § 4.3 sont migrés, mutation l.98
      comprise, et son assertion anti-vacuité existe ; `make test-sandbox-secret-argv`,
      `make verify-no-secret-in-setenv`, `make test-github-token-not-in-sandbox`,
      `make test-sandbox-gh-usable` passent.
- [ ] `make test-egress-sentinel-staging` existe, passe, et est appelée par `ci.yml`.
- [ ] L'addon `mika-pilot-github-auth-addon.py` est **inchangé** : aucun 403 nouveau.
- [ ] `MIKA_PILOT_EGRESS_SENTINEL=0` restaure le comportement d'avant le ticket,
      et c'est prouvé par une assertion.
- [ ] Le canari distingue quatre états et ne produit pas de faux LEAK.
- [ ] `cargo clippy` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte `Tracked in:` du vol 2 et
      `Code review: delegated to mika-qa (mika#2694)`.

## 7. Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § *Proposed Solution* et bornés au vol 1.

1. **AC1 — la sandbox est dotée.** Dans un vrai bwrap, `gh auth token` rend une
   valeur de la forme `proxy-managed-no-secret-[0-9a-f]{32}`, et rc 0. (Étape 1
   du § Proposed Solution ; `test_egress_sentinel_staging.sh` cas 4.)
2. **AC2 — la sentinelle est propre à la session.** Deux spawns consécutifs
   donnent deux valeurs distinctes, et les deux entrées coexistent côté hôte.
   (Cas 1 et 2.)
3. **AC3 — elle n'est ni le jeton GitHub ni dérivable de lui.** Le lint refuse
   une sentinelle assignée depuis `GH_TOKEN` / `MIKA_GITHUB_TOKEN` /
   `_PILOT_GH_TOKEN_FILE`, et la fixture le prouve rouge avant correctif.
4. **AC4 — elle ne franchit pas la frontière en tant que secret.** Le registre
   hôte n'est pas bind-monté ; les assertions de mika#2056 sur l'absence du vrai
   jeton (environnement, système de fichiers, argv) sont conservées intactes.
5. **AC5 — la sonde atteste au démarrage.** Le canari rend
   `ok:   gh_token_egress_sentinel` sur une sandbox saine, et un état
   `WARN: static placeholder` nommément distinct quand le staging a échoué ou
   que l'interrupteur est à `0`.
6. **AC6 — l'interrupteur opérateur ramène au comportement actuel.**
   `MIKA_PILOT_EGRESS_SENTINEL=0` → littéral nu, registre vide, dispatch
   identique à `origin/main`.
7. **AC7 — risque boucle nul.** Le diff ne touche pas
   `scripts/mika-pilot-github-auth-addon.py` ; aucune requête de pilote ne peut
   recevoir un 403 que `origin/main` n'aurait pas produit.
8. **AC8 — le reste est nommé.** Le corps de PR porte une ligne `Tracked in:`
   désignant le vol 2 (403 à deux motifs, log `DENY`, purge TTL).
9. **AC9 — aucune garde ne devient vacante.** Le contrôle négatif de
   `test_sandbox_gh_usable.sh` mute toujours réellement `dispatch-lib.sh` :
   `MUTATED_LIB` porte une ligne de moins que la source, et le test échoue en
   nommant la mutation si ce n'est pas le cas. Prouvé en inversant l'ancrage dans
   une copie sous `.pilot-scratch/` et en constatant l'échec. (§ 4.3, § 8.)

## 8. Fire-Disposition

Le vol 1 livre des détecteurs : la règle de provenance de
`verify-no-secret-in-setenv.sh`, les cinq ancrages migrés, la suite
`test_egress_sentinel_staging.sh`, le `case` du canari.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE.**

Détail d'implémentation, et pourquoi elle est vide :

- Le seul producteur de la valeur que ces détecteurs inspectent est
  `_stage_pilot_egress_sentinel`, livré **dans le même commit** que
  l'élargissement d'ancrage (§ 5, commits 1 et 2). Il n'existe donc aucun site
  en violation à l'instant du merge : rien à exempter, et inventer une entrée
  d'allowlist serait une exemption sans violation, c'est-à-dire la surface
  d'attaque décrite par
  `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`.
- L'allowlist **existe néanmoins comme liste fermée nommée** et grep-visible :
  `EXEMPT_SETENV_NAMES=(ANTHROPIC_API_KEY GH_TOKEN)` et
  `EXEMPT_SETENV_VARS=(_PILOT_EGRESS_SENTINEL)`. Un nom hors liste reste refusé
  même s'il porte une sentinelle bien formée.
- **Assertion auto-nettoyante.** `_total == _conformes && _total >= 1` est
  l'anti-stale : le `>= 1` rougit si `--setenv GH_TOKEN` disparaît (régression
  mika#2572, `gh` s'arrête sur `gh auth login`), et l'égalité rougit si une
  seconde occurrence non conforme apparaît. La règle de provenance rougit si
  `_PILOT_EGRESS_SENTINEL` se met à lire une source de jeton. Les deux sont
  épinglées par une fixture vue rouge avant le correctif.
- **Second anti-stale, et il porte sur un détecteur qui ne rougit pas de
  lui-même.** Le contrôle négatif de `test_sandbox_gh_usable.sh` tient à une
  mutation `grep -vF` qui, désancrée, recopie le fichier intact et mesure la
  sandbox dotée en croyant mesurer la sandbox démunie (§ 4.3). Son anti-stale est
  un compte : `MUTATED_LIB` doit porter une ligne de moins que `DISPATCH_LIB`,
  faute de quoi le test échoue en nommant la mutation qui n'a rien muté. Sans
  cette assertion le détecteur reste **vert et vacant** — la seule forme de
  régression que ce vol peut introduire sans qu'aucune garde ne crie.
- **Aucun détecteur n'atterrit désarmé.** Pas de `#[ignore]`, pas de
  `#[cfg(skip)]` : tous sont verts au merge sur un arbre sain, et la nouvelle
  suite est appelée par `ci.yml` dans le même commit qui la crée — une garde que
  le pipeline ne lance pas ne refuse rien (leçon mika#2578, ci.yml:116-123).

## 9. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (sentinelle per-spawn, registre, interrupteur, `--setenv`, commentaires d'audit) | 58 |
| `scripts/verify-no-secret-in-setenv.sh` (exemption par variable + règle de provenance) | 30 |
| `scripts/test-verify-no-secret-in-setenv.sh` (fixture de provenance ; les six cas GH_TOKEN survivent sans migration, § 4.2) | 15 |
| `skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh` (l.525, 589 + prose 26, 188) | 22 |
| `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` (l.255) | 12 |
| `skills/bundled/_shared/tests/test_sandbox_gh_usable.sh` (assertions l.52/157 + **mutation l.98 et son assertion anti-vacuité**, § 4.3) | 18 |
| tests (`skills/bundled/_shared/tests/test_egress_sentinel_staging.sh`, 6 cas sans troisième bwrap) + cible `Makefile` + entrée `ci.yml` | 50 |
| `scripts/canary-pilot-containment` (quatrième état) | 20 |

Total estimé : 225 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi ce vol tient sous la borne.** 225 < 243, soit 18 lignes de marge. La
mesure a bougé deux fois dans les deux sens depuis le premier passage, et dire
d'où elle vient vaut mieux que le nombre : +8 sur `test_sandbox_gh_usable.sh`,
parce que le recensement par `grep` a rendu un cinquième ancrage que la mémoire
avait manqué — la mutation de la l.98, dont l'échec est silencieux (§ 4.3) ;
−15 sur la nouvelle suite, parce que son cas « bout en bout » remontait un
troisième bwrap pour une propriété que deux suites déjà en CI mesurent après
élargissement d'ancrage (§ 4.4 cas 4).

La répartition : 153 lignes de travail neuf (dispatch-lib 58, la règle de lint
30 et sa fixture de provenance 15, la suite de staging 50) et 72 lignes de
**migration d'ancrages** sur quatre fichiers que rien n'oblige à toucher sinon
le changement de valeur. Ces 72 lignes ne sont pas compressibles — un vol qui
laisse ces gardes rouges ne merge pas — et la voie qui les éviterait toutes a
été examinée puis écartée : porter la sentinelle dans une variable **distincte**
de `GH_TOKEN` la mettrait hors de tout ancrage existant et hors de
`CRED_NAME_PATTERN` (ni `TOKEN`, ni `SECRET`, ni `KEY` dans un nom comme
`MIKA_EGRESS_SENTINEL`), donc ≈ 100 lignes de moins et aucune garde à migrer.
Elle ne fermerait **rien** : un test qui écrase `GH_TOKEN` laisserait la
sentinelle intacte et serait autorisé, et l'incident mika#2713 se reproduirait à
l'identique (§ 2, dernier paragraphe). Ces 72 lignes sont le prix du seul canal
qui atteste.

Deux choses ont été explicitement renvoyées au vol 2, et chacune l'est **avec son
lecteur** plutôt que livrée orpheline :

- la purge TTL du registre — elle n'a de sens qu'avec le code qui lit le
  registre, et rien ne le lit au vol 1 ;
- `_read_sentinel`, l'extraction du credential présenté (`Bearer` sur
  `api.github.com`, `Basic` décodé sur `github.com`), la comparaison
  `hmac.compare_digest`, les deux 403 et le log `DENY` — soit l'addon en entier,
  estimé à ≈ 80 lignes plus ≈ 70 de tests, donc un vol propre.

Le vol 1 stage malgré tout le registre, et la suite le lit : sans cela le vol 2
devrait construire le canal **et** son lecteur dans le même vol, et le coût de
celui-ci serait payé deux fois.

## 10. Risques et arbitrages

| # | risque | traitement |
|---|---|---|
| R1 | **Le vol 1 passe et rien n'est fermé.** Le périmètre livré ne refuse encore aucune requête. | Assumé et prescrit par le ticket (« mode de défaillance à traiter **avant** tout déploiement »). AC8 force le `Tracked in:` ; le ticket reste ouvert. |
| R2 | **Le lint perd en rigueur** en passant d'un littéral statique à une variable. | La règle de provenance (§ 4.2 assertion 2) restaure l'inspection statique : la valeur n'est plus lisible, mais sa **source** l'est, et la fixture épingle la source fautive. |
| R3 | **Deux pilotes concurrents.** Le mitmdump est long-lived et partagé. | Registre-répertoire dès le vol 1, et non fichier unique ; cas 2 de la suite l'épingle. C'est le défaut structurel qui aurait été découvert au vol 2, en production. |
| R4 | **Faux LEAK du canari** après déploiement. | § 4.3 : le `case` est migré dans ce vol, pas au suivant. Précédent exact : mika#2572 § 5. |
| R5 | **`gh` refuse une valeur de 54 caractères.** | `gh` accepte toute chaîne non vide en `GH_TOKEN` ; la mesure de mika#2572 (table § Symptômes) porte sur la présence, pas sur la forme. Cas 4 de la suite le mesure dans un vrai bwrap plutôt que de le supposer. |
| R6 | **Marge de 18 lignes.** Un dépassement en implémentation repasse au-dessus de la borne. | La variable d'ajustement nommée à l'avance : le **cas 3** de la suite de staging (permissions `0700`/`0600` du registre, ≈ 10 lignes) part au vol 2. Elle est choisie parce que la propriété est tenue **par construction** au site d'écriture (`mkdir` en `0700`, écriture sous `umask 077`, § 4.1 point 3) et que le vol 2 est celui qui *lit* le registre : elle le rejoint donc avec son lecteur, comme la purge TTL. À n'employer que si la mesure réelle dépasse, et à dire dans le corps de PR. **Ce n'est pas le canari** : le § 4.3 établit que le reporter produit un faux LEAK dès le premier dispatch, et une garde de sécurité qui crie à tort coûte plus que 20 lignes. |

## 11. Hors périmètre, délibérément

- **L'addon.** `scripts/mika-pilot-github-auth-addon.py` n'est pas touché. C'est
  AC7, et c'est ce qui rend le vol 1 sans risque pour la boucle.
- **Le `cargo test` qui n'écrase pas `GH_TOKEN`.** Un test qui hérite de la
  sentinelle et appelle `gh` continuera de passer, même après le vol 2 : il
  présente une sentinelle valide. Cette moitié est fermée côté code par
  mika#2713 (`run_gh` injectable, scan structurel, plancher de numéro de
  fixture), et aucune sentinelle ne peut la fermer — un secret présent dans la
  sandbox est lisible par tout processus de la sandbox (§ 2.1).
- **L'egress Anthropic.** `ANTHROPIC_API_KEY` garde son placeholder statique.
  Le mode de défaillance de mika#2713 est propre à GitHub (écriture sur une
  forge), et étendre la sentinelle à Anthropic doublerait les gardes
  co-dépendantes sans fermer de classe mesurée.
- **Le shim `gh` et `http.extraHeader`.** Écartés avec leur raison au § 2 ; la
  note de solution du § 4.5 les enregistre pour que la question ne soit pas
  rouverte à l'identique.

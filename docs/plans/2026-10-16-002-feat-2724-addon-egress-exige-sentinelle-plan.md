# feat(infra) — l'addon d'egress exige la sentinelle du pilote (vol 2 de mika#2716)

**Ticket :** senara-solutions/mika#2724
**Ticket parent :** senara-solutions/mika#2716 — **PR du vol 1 :** senara-solutions/mika#2723
**Jalon :** Substrat de boucle
**Labels :** `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`

---

## 1. Pourquoi

Le vol 1 (PR #2723, en production) a doté chaque sandbox d'une sentinelle
per-spawn attestée côté hôte, et l'a rendue lisible par la sonde. Il n'a armé
**aucun** refus : `scripts/mika-pilot-github-auth-addon.py:143-145` retire
toujours tout `Authorization` / `Proxy-Authorization` venu du client, puis
injecte inconditionnellement le jeton lu côté hôte.

La propriété acquise est donc que l'appel légitime et l'appel accidentel sont
**distinguables**, pas que le second est refusé. Un `cargo test` lancé depuis une
sandbox de pilote écrit encore sur la forge réelle sous l'identité du moteur —
le substrat de mika#2713 (29 commentaires et un label sur un ticket **fermé**).

Ce vol ajoute **un lecteur devant un canal déjà attesté**. Il ne construit pas le
canal.

## 2. Ce que la lecture du code — et trois mesures — imposent au corps du ticket

Le point 1 du ticket prescrit deux formes de lecture : « `Bearer <valeur>` sur
`api.github.com` (`gh`) et `Basic <base64>` **décodé** sur `github.com` (`git`
via le credential helper) ». La seconde moitié repose sur une prémisse fausse, et
l'implémenter telle quelle produirait exactement le mode de défaillance que le
ticket nomme comme le seul capable d'arrêter la boucle.

### 2.1 Mesure — `git` ne présente aucun credential dans la sandbox

**Il n'y a pas de credential helper dans la sandbox du pilote.**
`_run_pilot_sandboxed` pose `--setenv GIT_CONFIG_GLOBAL "$_PILOT_GITCONFIG_SANDBOX"`
et `--setenv GIT_CONFIG_NOSYSTEM 1` (`dispatch-lib.sh:2311-2312, 2395-2396`), et le
fichier pointé est écrit **from scratch** par `_stage_pilot_gitconfig`
(`dispatch-lib.sh:1705-1744`) : un `[url]` avec deux `insteadOf`, un `[user]`, et
« **3. Nothing else.** […] so a credential helper added to the operator's config
later cannot drift into the sandbox » (l.1698-1700). La sandbox n'a ni `~/.ssh`,
ni `GIT_ASKPASS` utilisable.

Mesuré plutôt que supposé, **au grooming** (2026-10-16) contre un serveur HTTP
local qui journalise chaque en-tête reçu et répond `401` avec un challenge
`WWW-Authenticate: Basic` — le challenge est la moitié qui compte : s'il existait
une source de credential, `git` retenterait avec un `Authorization`. La
configuration est celle de la sandbox, reproduite octet pour octet depuis
`_stage_pilot_gitconfig` (`GIT_CONFIG_GLOBAL` = un `[url]` + un `[user]` et rien
d'autre, `GIT_CONFIG_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0`, `HOME` neuf,
`GH_*` / `GIT_ASKPASS` / `GIT_CONFIG_COUNT` retirés de l'environnement) :

```
requests observed: 1
  [1] /senara-solutions/mika.git/info/refs?service=git-upload-pack
      header names: Accept, Accept-Encoding, Accept-Language,
                    Git-Protocol, Host, Pragma, User-Agent
      AUTHORIZATION PRESENT: False
git exit=128
fatal: could not read Username for 'http://127.0.0.1:…': terminal prompts disabled
```

**Aucun `Authorization` — et pas même une retentative.** Le résultat est plus
net que « git ne présente rien » : au challenge `Basic`, `git` n'a aucune source
à interroger, donc il abandonne sur `could not read Username` au lieu de
réémettre. Une **seule** requête, nue. Un 401 renvoyé par l'addon ne sauverait
donc rien non plus : il n'y a aucun second coup à attendre.

**La méthode, pour que la mesure soit refaisable et non crue** : un
`http.server` sur `127.0.0.1:0` qui log `dict(self.headers)` et répond 401 avec
le challenge, puis un `git ls-remote` vers lui sous l'environnement ci-dessus.
≈ 60 lignes de Python jetable. L'implémenteur n'a pas à la refaire ; il doit
seulement ne pas la contredire sans la refaire.

**Et la condition sous laquelle elle tient — à écrire, pas à taire.** La sandbox
lit une seconde configuration que `_stage_pilot_gitconfig` ne contrôle pas :
`$PARENT_GIT/config` est `--ro-bind` et git lit le config **du dépôt** à
précédence **plus haute** que `GIT_CONFIG_GLOBAL`. `dispatch-lib.sh:1124-1130` le
dit déjà en toutes lettres, et ajoute pourquoi le dire ainsi :

> Today this repository's config carries none — checked, not assumed — […]
> Stating the boundary precisely matters more than stating it flatteringly: the
> flattering version of this comment is what let mika#2141 read as a decision
> for a month.

Vérifié au grooming à son tour : ni `credential.*`, ni `http.*.extraHeader`, ni
`core.askPass` dans le config du dépôt parent ni dans celui du worktree. La
propriété du § 2.1 est donc **vraie et conditionnée** : un `credential.helper`
ajouté un jour au config du dépôt la retournerait. Le cas 8 du § 3.4 est ce qui
la tient — il asserte que `github.com` n'est jamais refusé *quel que soit* le
credential présenté, donc il reste vert même le jour où `git` se met à en
présenter un.

Conséquence directe : exiger la sentinelle sur `github.com` rend **403 tous les
`git push` / `git fetch` de tous les pilotes**, dès le `make deploy`. C'est mot
pour mot le mode de défaillance que le corps du ticket parent nomme — « un bug
d'injection ferait silencieusement 403 **tous** les appels légitimes et la boucle
autonome s'arrêterait sans diagnostic » — et il arriverait non par bug mais par
conception.

### 2.2 Et doter `git` de la sentinelle n'achèterait rien

La voie qui sauverait le point 1 à la lettre est de poser un credential dans la
gitconfig générée — `http.extraHeader` ou un `credential.helper` lisant
`$GH_TOKEN`. Elle est **écartée**, et pas pour son coût (≈ 15 lignes) :

`$_PILOT_GITCONFIG_HOST` est `--ro-bind` dans la sandbox et désigné par
`GIT_CONFIG_GLOBAL`, donc lu par **tout** processus de la sandbox. Un `cargo test`
qui ferait `git push` l'emprunterait et présenterait une sentinelle **valide**.
C'est mot pour mot l'argument que le § 2 du plan du vol 1 oppose au shim `gh` :

> Un secret posé par un wrapper atteste **la sandbox**, pas **le chemin
> légitime** — il n'attesterait donc rien d'utile pour mika#2713.

La dotation de `git` coûterait donc des lignes **et** de la surface de risque sur
le seul chemin capable d'arrêter la boucle, pour **zéro** propriété fermée.

### 2.3 La décision, et la raison qui la rend non arbitraire

**La coupure porte sur `api.github.com` seulement. `github.com` garde l'injection
inconditionnelle d'aujourd'hui.**

Ce n'est pas une réduction de confort, c'est le périmètre que le choix de canal du
vol 1 détermine. La propriété de mika#2716 est :

> le justificatif vit **dans le canal que le mode de défaillance écrase**.

Ce canal est `GH_TOKEN`. Son **seul** lecteur dans la sandbox est `gh`, et `gh`
ne parle qu'à `api.github.com`. Sur `github.com`, aucun client ne lit `GH_TOKEN`,
donc aucun justificatif ne peut y voyager dans le canal que l'incident écrase :
exiger là-bas, ce serait exiger un justificatif qu'aucun appel légitime ne peut
présenter.

**Rectification à porter dans la note de solution du vol 1.** La table des canaux
de `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
porte, sur la ligne retenue (**l.59**, canal « le credential lui-même
(`GH_TOKEN`) »), la case « posé par `git` ? **oui, déjà** ». Elle est fausse —
§ 2.1 : `git` ne lit pas `GH_TOKEN` et ne présente rien. Ce qui voyage sur
`github.com`, c'est le `Basic` que l'**addon** fabrique depuis le jeton de
l'hôte (`_auth_header_for`, l.118) — posé par le proxy, jamais par le client.

La corriger est un livrable de ce vol (§ 6, commit 4), et la correction porte la
distinction plutôt que juste la case : « posé par `git` ? **non** — l'addon
injecte un `Basic` côté proxy, donc aucun justificatif ne peut voyager dans ce
canal sur `github.com` ». Laissée telle quelle, elle rouvrirait la question à
l'identique au prochain passage, ce que cette note existe précisément pour
empêcher — et elle le ferait avec le pire des résultats, puisque c'est la case
qui ferait croire qu'exiger la sentinelle sur `github.com` est sans danger.

### 2.4 Ce qui reste ouvert, nommé plutôt que tu

Un `git push` arbitraire depuis une sandbox reste possible après ce vol, comme
avant. Il n'est pas sans bornes — mika#2141 limite le gitdir bind au worktree du
pilote et aux refs sous son préfixe — mais il n'est pas fermé, et aucune
sentinelle ne peut le fermer (§ 2.2). C'est une classe distincte de celle de
mika#2713, qui est une **écriture d'API** (`gh issue comment`).

Tracked in: un ticket de suivi ouvert **avant** la PR, nommé dans son corps
(règle mika#2701), portant « fermer la moitié `git push` de l'egress de pilote,
ou établir qu'elle est hors d'atteinte ».

## 3. Conception

### 3.1 `scripts/mika-pilot-github-auth-addon.py` — le lecteur

Quatre fonctions, dont trois **pures** pour que la décision soit testable sans
mitmproxy, et une intégration dans `requestheaders` pour que les pures ne soient
pas vertes et débranchées (classe mika#2205).

```python
_ALLOW            = "allow"
_DENY_ABSENT      = "sentinel-absent"
_DENY_UNKNOWN     = "sentinel-unknown"
_DENY_REGISTRY    = "registry-unreadable"
```

**`_presented_credential(headers) -> str | None`.** Extrait la valeur présentée
par le client **avant** que l'addon ne la retire. Trois schémas, **comparaison du
schéma insensible à la casse** :

| schéma | forme | d'où |
|---|---|---|
| `Bearer <v>` | `<v>` tel quel | REST/GraphQL |
| `token <v>` | `<v>` tel quel | **`gh` historiquement émet `Authorization: token <tok>`**, pas `Bearer` |
| `Basic <b64>` | base64-décodé, puis **la partie après le premier `:`** | `curl -u`, une lib HTTP |

Le schéma `token` n'est pas un confort : **le supposer absent est exactement
l'erreur qui 403'rait tous les appels `gh`.** Et ce n'est plus une hypothèse —
mesuré au grooming sur le binaire **réellement installé** (`/usr/bin/gh`,
43 Mo) :

```
$ grep -ao -E "(token|Bearer) %s" /usr/bin/gh | sort | uniq -c
      1 Bearer %s
      2 token %s
```

Les **deux** formats de header vivent dans le binaire, et `token %s` y est deux
fois. Un lecteur qui n'accepterait que `Bearer` parierait donc contre le format
le mieux représenté dans le client dont dépend toute la boucle.

**Ce que cette mesure établit, et ce qu'elle n'établit pas.** Elle établit que
les deux schémas sont des chemins vivants du client, donc qu'accepter les deux
n'est pas défensif mais nécessaire. Elle n'établit **pas** lequel part sur le fil
pour un appel donné : une chaîne de format n'est pas une capture. La confirmation
sur le fil reste au DoD — elle est à faire une fois dans la session
d'implémentation, où `gh` est invocable (il ne l'est pas sous la politique de la
sandbox de grooming, qui refuse l'exec de `gh`). Mais elle n'est plus la seule
défense : accepter les trois schémas fait que la mesure *confirme* au lieu de
*décider*. Si `gh` émettait un quatrième schéma, le cas 2 de la suite CI
rougirait avant le merge plutôt que le premier dispatch après.

Un `Basic` mal formé (base64 invalide, UTF-8 invalide, pas de `:`) rend `None` —
jamais une exception, jamais un `allow`. `Proxy-Authorization` est lu avec la
même précédence que `Authorization` (l'addon retire déjà les deux).

**`_registry_nonces() -> frozenset[str] | None`.** Liste le registre.
`None` — et **seulement** `None` — signifie *illisible* : répertoire absent,
`PermissionError`, toute `OSError`. Un registre lisible mais **vide** rend un
`frozenset()` vide, qui est un état différent et qui refuse par
`sentinel-unknown`, pas par `registry-unreadable`.

Chemin : `_PILOT_EGRESS_SENTINEL_DIR` dans l'environnement, défaut
`Path.home() / ".mika" / "pilot-egress-sentinels"`, **résolu à l'appel** (même
leçon que le vol 1 : une résolution au chargement du module fige le `HOME` du
démon). **Le même nom de variable que le shell**, délibérément : un opérateur qui
surcharge le registre doit déplacer les deux moitiés d'un coup, sinon le lanceur
écrit ici et le lecteur regarde là, et **tout** devient `sentinel-unknown`.

**`_sentinel_verdict(presented) -> str`.** Pure, et l'ordre des branches porte la
sûreté :

1. `presented == _PILOT_EGRESS_SENTINEL_PREFIX` (le placeholder **nu**) →
   `_ALLOW`, **sans toucher au registre**. C'est le point 6 du ticket, et c'est
   ce qui rend le fail-closed de la branche 4 survivable (§ 3.2).
2. `presented is None` ou vide → `_DENY_ABSENT`.
3. Le registre est illisible → `_DENY_REGISTRY`.
4. `any(hmac.compare_digest(presented, prefix + "-" + n) for n in nonces)` →
   `_ALLOW`, sinon `_DENY_UNKNOWN`.

`hmac.compare_digest` et non `==`, comme le ticket l'exige : la comparaison porte
sur un justificatif, et un timing leak y est gratuit à éviter. Elle impose aussi
le **scandir plutôt que le lookup par nom** (`os.path.exists(dir / presented)`) —
qui serait O(1) mais comparerait par le filesystem, et ouvrirait un path traversal
sur une valeur présentée de la forme `../../…`. Le scandir ne touche jamais la
valeur présentée comme un chemin.

**`requestheaders`** — modifié :

- hôte hors `_GITHUB_HOSTS` → inchangé.
- hôte `github.com` → **strictement inchangé** : strip + injection. Un commentaire
  nomme le § 2.3 pour que la dissymétrie ne se lise pas comme un oubli.
- hôte `api.github.com` → lire le credential présenté, calculer le verdict,
  **puis** strip + injection si `_ALLOW`, sinon `flow.response = Response.make(403, …)`.
  L'ordre est porteur : le credential doit être lu **avant** le `pop`.
- `_read_token()` renvoyant `None` garde son 503 actuel, **avant** le verdict :
  « pas de jeton côté hôte » n'est pas un refus de sentinelle et ne doit pas se
  déguiser en un.

Le corps du 403 est `text/plain`, nomme le motif en clair (`sentinel-absent` /
`sentinel-unknown` / `registry-unreadable`), nomme mika#2716, et nomme le remède
propre à chaque motif. Il **ne porte jamais** la valeur présentée ni le jeton de
l'hôte.

**Le log `DENY`** — une ligne par refus, greppable, en miroir du
`[github-proxy] ALLOW …` existant (l.159-164), sur `stderr`, donc dans
`/var/log/mika/pilot-helper.log` (`dispatch-lib.sh:1093`) :

```
[github-proxy] DENY <motif> <method> <path>
```

Ni la valeur présentée, ni le jeton de l'hôte, ni une empreinte de l'un des deux :
**nommer, pas divulguer** (doctrine mika#1949). Une empreinte serait une
divulgation différée — elle permet de confirmer une valeur devinée.

**Purge TTL.** `_purge_registry()`, appelée depuis `_registry_nonces()`, supprime
les entrées dont le `st_mtime` dépasse **72 h**, au plus une passe toutes les
10 minutes (horloge monotone en module). Deux propriétés, et la première prime :

- **Une purge ne doit jamais pouvoir refuser une sentinelle vive.** C'est
  pourquoi 72 h et non 24 : un pilote sous plafond de 200 tours tourne des heures,
  et une purge qui le 403 en fin de course *est* le mode de défaillance redouté.
  Les entrées sont des fichiers de **0 octet** ; 72 h de spawns est négligeable
  sur un inode.
- La purge tourne **avant** le calcul de la liste rendue, jamais après un verdict :
  l'entrée qu'on vient de valider ne peut pas disparaître entre la validation et
  la réponse.
- Toute `OSError` d'une suppression est **ignorée** et ne fait pas rendre `None` :
  une purge qui échoue n'est pas un registre illisible, et la confondre ferait 403
  tout le monde sur un `chmod` malheureux.

Pas de variable d'environnement pour le TTL : une constante nommée, documentée,
et les tests vieillissent les entrées par `os.utime`.

### 3.2 Fail-closed, et l'asymétrie voulue — la décision que le ticket demande

> « Le choix entre les deux pour un registre illisible **côté addon** est la
> décision à poser explicitement dans la PR. »

**Côté addon, un registre illisible REFUSE.** C'est l'inverse du fail-open du
staging côté lanceur, et l'inversion est écrite dans le code en toutes lettres :

- côté lanceur, un registre illisible ne doit pas **tuer un dispatch** — il
  dégrade vers le placeholder nu ;
- côté addon, un registre illisible ne doit pas **autoriser tout le monde** — un
  garde qui s'ouvre quand son référentiel disparaît est un garde qu'il suffit de
  faire disparaître.

Ce qui rend ce fail-closed survivable, et c'est la pièce qui tient tout
l'édifice : **le placeholder nu est accepté par reconnaissance de FORME, sans
lire le registre** (branche 1 de `_sentinel_verdict`). Donc
`MIKA_PILOT_EGRESS_SENTINEL=0` est un rollback **complet** qui ne dépend d'aucun
registre — point 6 du ticket honoré des deux côtés, et mika#2165 fermé : le
second lecteur honore l'interrupteur, donc l'interrupteur n'est pas décoratif.

**Un second lecteur du même nom, et pas un nom de plus.** L'addon lit lui aussi
`MIKA_PILOT_EGRESS_SENTINEL` dans **son propre** environnement (il hérite de
celui du shell qui a lancé `_ensure_pilot_helper`), même vocabulaire que le shell
(`0`/`false`/`no`/`off`/`disabled`, insensible à la casse) : désarmé, il n'exige
rien et se comporte comme `origin/main`. Les deux moitiés couvrent des pannes
**différentes** et c'est pourquoi les deux existent :

| panne | ce qui la couvre |
|---|---|
| la coupure refuse à tort des sentinelles valides | le placeholder nu accepté par forme : poser `=0` sur le service relance des dispatches qui passent, sans toucher à mitmdump |
| c'est le **lecteur lui-même** qui déraille | la variable dans l'environnement de mitmdump : poser `=0` et relancer le démon désarme la coupure entière |

### 3.3 La surface de refus — le point 7, et son anti-vacuité

> « sinon “l'addon refuse tout” et “l'addon n'a rien vu passer” rendent les mêmes
> octets (classe mika#2205) »

Le canari lit l'**intérieur** de la sandbox ; le refus vit côté **addon**. Le pont
existe déjà et n'a pas à être construit : **le corps de la réponse 403 est une
réponse HTTP**, donc il traverse la frontière et nomme le motif à qui a émis
l'appel.

La ligne `curl_github=200` d'aujourd'hui (`canary-pilot-containment:680`) devient
**trois** sondes, dans le bloc « must-work: allowlisted egress » :

| sonde | attendu | ce qu'elle nomme |
|---|---|---|
| `curl -H "Authorization: Bearer $GH_TOKEN" https://api.github.com/` | `egress_sentinel_accepted=200` | **l'injection marche sur le chemin légitime** — la ligne qu'un `make deploy` cassé noircit, et qui interdit qu'un addon refusant tout lise vert |
| `curl https://api.github.com/` (nu) | `egress_sentinel_denied_absent=403` + le motif lu dans le corps | le refus a lieu, motif `sentinel-absent` |
| `curl -H "Authorization: Bearer not-a-sentinel-canary-2724" …` | `egress_sentinel_denied_unknown=403` + le motif | le refus **distingue** les deux causes |

La première sonde est l'anti-vacuité des deux autres, et réciproquement : un addon
qui refuserait tout noircit la première ; un addon absent ou non déployé noircit
les deux autres. Les deux états ne rendent plus les mêmes octets.

Deux contraintes de rédaction que le fichier impose, et dont l'oubli casse le
canari en silence :

- **Aucune quote simple** dans les messages ajoutés : le bloc vit dans un payload
  `sh -c '…'` entre quotes simples (leçon mika#2572 § *Ce qui n'a pas marché*,
  déjà payée au vol 1).
- La valeur bidon de la troisième sonde ne doit matcher aucun motif de
  `_CANARY_CRED_PATTERN` (`canary-pilot-containment:435`), sinon le scan de fuite
  la signale elle-même. `not-a-sentinel-canary-2724` n'en matche aucun.

**Sous `MIKA_PILOT_EGRESS_SENTINEL=0`**, `GH_TOKEN` porte le placeholder nu : la
sonde 1 rend toujours 200 (accepté par forme), les sondes 2 et 3 toujours 403. Le
canari reste donc cohérent sous rollback, et la prose le dit — sinon un opérateur
qui vient de désarmer lirait les 403 comme un rollback raté.

### 3.4 La suite CI — `skills/bundled/_shared/tests/test_egress_sentinel_enforcement.sh`

Le canari n'est pas lancé par le pipeline (il est post-déploiement, opérateur). La
coupure a donc besoin de sa propre garde en CI — une garde que le pipeline ne
lance pas ne refuse rien (leçon mika#2578, déjà invoquée deux fois par le vol 1).

Patron : celui de `test-pilot-github-token-not-in-sandbox.sh` PART A (l.93-157) —
stub du module `mitmproxy`, chargement par `importlib`, assertions en Python,
`exit` agrégé relu côté bash. Le stub gagne un `Response.make` et un `HTTPFlow`
minimaux (≈ 25 lignes) pour que `requestheaders` soit **réellement** exercé.
Cible `make test-egress-sentinel-enforcement`, ajoutée à `ci.yml` dans le bloc
mika#2716 existant (l.111-115), **dans le commit qui crée la suite**.

Dix cas, et chacun répond à une défaillance nommée plus haut :

1. **Bearer d'une sentinelle enregistrée → 200**, et l'`Authorization` sortant est
   **celui de l'hôte**, pas celui présenté. (Le chemin légitime, et la preuve que
   la coupure n'a pas remplacé l'injection par un passe-plat.)
2. **`token <sentinelle>` → 200.** Le schéma que `gh` émet (§ 3.1).
3. **`Basic base64("x-access-token:<sentinelle>")` → 200.** Dit franchement :
   aucun client de production n'emprunte cette branche aujourd'hui ; elle est
   livrée parce que le ticket l'exige et parce qu'un `curl -u` légitime vers
   l'API ne doit pas être traité autrement qu'un `Bearer`.
4. **Rien présenté → 403**, corps nommant `sentinel-absent`, une ligne `DENY`.
5. **Valeur inconnue du registre → 403**, corps nommant `sentinel-unknown`, et
   **assertion d'inégalité des deux corps**. Sans elle, « deux motifs distincts »
   est une affirmation du plan, pas une propriété du code.
6. **Placeholder nu → 200, registre rendu illisible dans le même cas.** C'est
   l'assertion qui prouve que la branche 1 ne lit pas le registre, donc que le
   rollback survit à la panne.
7. **Registre illisible + sentinelle bien formée → 403** `registry-unreadable`,
   motif distinct des deux autres.
8. **`github.com` → injection inchangée**, aucun 403 possible quel que soit le
   credential présenté (non-régression du chemin `git`, § 2.3).
9. **Purge TTL** : une entrée vieillie par `os.utime` disparaît, une entrée
   fraîche survit, et l'entrée validée dans le même appel n'est jamais purgée.
10. **Le log `DENY` ne porte ni la valeur présentée ni le jeton de l'hôte** —
    assertion d'**absence** sur le `stderr` capturé, avec une valeur présentée et
    un jeton d'hôte tous deux reconnaissables, pour que l'absence ne soit pas
    vacante.

Plus un cas **anti-vacuité transversal**, qui n'est pas en plus mais dans les cas
1 et 4 : ils passent par `requestheaders`, pas par un appel direct à
`_sentinel_verdict`. Une fonction pure verte et débranchée est la classe mika#2205
que ce vol cite deux fois ; la fermer coûte deux lignes de stub.

## 4. Fire-Disposition

Ce vol livre des détecteurs : la suite `test_egress_sentinel_enforcement.sh` et
les trois sondes du canari.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist VIDE.**

Détail d'implémentation, et pourquoi elle est vide :

- Le seul producteur de la valeur que ces détecteurs inspectent est
  `_stage_pilot_egress_sentinel`, **livré et déployé au vol 1**, et le seul
  lecteur est l'addon, livré au commit 1 de ce vol. Il n'existe aucun site en
  violation à l'instant du merge : rien à exempter. Inventer une entrée serait une
  exemption sans violation, c'est-à-dire la surface d'attaque décrite par
  `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`.
- **Assertion auto-nettoyante — le cas 1 et le cas 8.** Le risque de péremption
  d'un détecteur de refus n'est pas qu'il cesse de refuser, c'est qu'il refuse
  **tout** et lise vert. Les cas 1 (le chemin légitime passe, avec le jeton de
  l'hôte) et 8 (`github.com` intact) sont les positifs qui rougissent exactement
  dans ce cas-là. Le cas 5 (inégalité des deux corps de 403) rougit si les motifs
  fusionnent. Le cas 10 rougit si le log se met à divulguer.
- **Aucun détecteur n'atterrit désarmé, ni hors pipeline.** Pas de `#[ignore]`
  ni d'équivalent : tous verts au merge, et la suite entre dans `ci.yml` dans le
  commit qui la crée. Le canari reste hors CI parce qu'il est post-déploiement par
  nature — et c'est précisément pourquoi la suite existe à côté de lui : le canari
  seul laisserait la coupure sans garde de pipeline.
- **Aucun détecteur de ce vol ne défaille en silence.** Chaque cas est une
  assertion qui rougit, et les deux dont le mécanisme pouvait se vider — une
  fonction pure débranchée (§ 3.4, cas 1 et 4 passent par `requestheaders`) et une
  assertion d'absence sur un log (cas 10, valeurs reconnaissables plantées) —
  portent leur anti-vacuité dans le cas lui-même.

## 5. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/mika-pilot-github-auth-addon.py` (lecteur 3 schémas, registre, verdict 4 branches, 3 motifs de 403, log `DENY`, purge TTL, second lecteur de l'interrupteur) | 140 |
| tests (`skills/bundled/_shared/tests/test_egress_sentinel_enforcement.sh`, 10 cas + stub `requestheaders`) | 140 |
| `scripts/canary-pilot-containment` (trois sondes + prose du rollback) | 35 |
| `skills/bundled/_shared/tests/test-pilot-github-token-not-in-sandbox.sh` (PART A gagne le refus, la coupure étant la nouvelle moitié du contrat de l'addon) | 12 |
| `Makefile` (cible) + `.github/workflows/ci.yml` (entrée) | 8 |

Total estimé : 335 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Pourquoi ce vol tient, et en un seul.** 335 < 431, soit 96 lignes de marge, et
335 < 1000. Le ticket estimait ≈ 150 (80 d'addon, 70 de tests) depuis le § 9 du
plan du vol 1 ; la mesure monte à 335, et dire d'où vient l'écart vaut mieux que
le nombre :

- **+60 sur l'addon** — trois éléments que l'estimation du vol 1 ne pouvait pas
  connaître : le troisième motif `registry-unreadable` (§ 3.2), le troisième
  schéma `token` que `gh` émet réellement (§ 3.1), et le second lecteur de
  l'interrupteur. Chacun ferme une façon précise d'arrêter la boucle.
- **+70 sur les tests** — dix cas et non cinq, parce que la liste a été dérivée
  des défaillances nommées une à une plutôt que d'un ordre de grandeur, et parce
  que le stub de `requestheaders` (≈ 25 lignes) est le prix de ne pas livrer une
  fonction pure débranchée.
- **+55 sur le canari et les ancrages** — non comptés par le vol 1, qui renvoyait
  « l'addon » en bloc. Le point 7 du ticket les exige.

**Ce que la marge paie, nommément.** Si la mesure réelle dépasse, la variable
d'ajustement est le **cas 3** (la branche `Basic`, ≈ 12 lignes d'addon et ≈ 15 de
test) : c'est le seul livrable dont aucun client de production n'emprunte le
chemin aujourd'hui (§ 3.4). Il part alors avec son test, jamais sans, et le corps
de PR le dit. **Ce ne sont pas les sondes du canari** : le § 2.3 établit que la
ligne `curl_github=200` actuelle devient fausse dès le déploiement — la reporter
produirait un canari qui crie au défaut sur le régime nominal, exactement le piège
que le vol 1 a documenté et évité sur le `case` `gh_token_*`.

Et ce n'est pas davantage la suite CI : sans elle, le vol livre une coupure
fail-closed sur le chemin de la boucle, épinglée par un seul détecteur que le
pipeline ne lance pas.

## 6. Ordre de commit, et son plancher

Chaque commit laisse l'arbre vert. Le plancher est le **commit 2**.

1. `feat(infra)` — l'addon : `_presented_credential`, `_registry_nonces`,
   `_sentinel_verdict`, la purge, les trois 403, le log `DENY`, le second lecteur
   de l'interrupteur. *(L'arbre reste vert : aucune suite existante n'exerce
   l'addon au-delà de `_read_token` / `_auth_header_for`, que ce commit ne touche
   pas. Rien n'est déployé — l'addon n'est actif qu'après le `cp` de
   `Makefile:55-57`.)*
2. **PLANCHER** — `test_egress_sentinel_enforcement.sh` + la cible `Makefile` +
   l'entrée `ci.yml`. S'arrêter avant le commit 2 livrerait une coupure
   fail-closed sur le chemin de la boucle, qu'aucune garde de pipeline n'épingle :
   c'est le seul état de cet arbre qui soit pire que l'absence du ticket.
3. `feat(infra)` — les trois sondes du canari, et l'ajout de PART A dans
   `test-pilot-github-token-not-in-sandbox.sh`.
4. `docs` — le § opérateur de mika#2716 (les trois motifs, la purge, le rollback
   des deux côtés, la halte réécrite), **la rectification de la case `git` de la
   table de la note de solution** (l.59, § 2.3), et la capture du schéma émis par
   `gh` ajoutée à la note : le grooming a établi que les deux formats vivent dans
   le binaire (§ 3.1), le commit consigne lequel part sur le fil.

Le ticket de suivi du § 2.4 est ouvert **avant** la PR et nommé dans son corps.

**Une seule mesure est prérequise au commit 1**, et c'est la seule chose à faire
avant d'écrire du code : la capture du schéma `gh` sur le fil (DoD). Les deux
autres mesures du plan — `git` ne présente rien (§ 2.1), les deux formats vivent
dans le binaire (§ 3.1) — sont **acquises au grooming** et ne sont pas à refaire.

## 7. Definition of Done

- [ ] `api.github.com` : une requête sans credential rend 403 `sentinel-absent` ;
      avec un credential inconnu, 403 `sentinel-unknown` ; avec un registre
      illisible, 403 `registry-unreadable` ; les trois corps sont **distincts**.
- [ ] `api.github.com` : `Bearer`, `token` et `Basic` décodé d'une sentinelle
      enregistrée passent tous les trois, et l'`Authorization` sortant est celui
      de l'hôte.
- [ ] Le schéma émis par `gh` est **confirmé sur le fil** une fois (le grooming a
      déjà établi que `token %s` et `Bearer %s` vivent tous deux dans le binaire,
      § 3.1) et consigné dans le corps de PR. Si la capture révèle un schéma
      qu'aucune des trois branches ne couvre, l'implémentation **s'arrête et
      remonte** : c'est le seul fait capable de 403 toute la boucle.
- [ ] `github.com` n'est refusé **quel que soit** le credential présenté — le
      cas 8 ne suppose pas la mesure du § 2.1, il la rend inutile au garde.
- [ ] La comparaison passe par `hmac.compare_digest`, jamais par `==`, et jamais
      par un lookup de la valeur présentée comme nom de fichier.
- [ ] `github.com` est **inchangé** : aucun credential exigé, aucun 403 nouveau,
      et le cas 8 le prouve.
- [ ] Une ligne `[github-proxy] DENY <motif> <method> <path>` par refus, sans la
      valeur présentée, sans le jeton de l'hôte, sans empreinte de l'un ni de
      l'autre ; le cas 10 le prouve par une absence non vacante.
- [ ] La purge supprime au-delà de 72 h, ne refuse jamais une sentinelle vive, et
      une `OSError` de suppression ne fait pas rendre `registry-unreadable`.
- [ ] `MIKA_PILOT_EGRESS_SENTINEL=0` est un rollback **complet** : le placeholder
      nu est accepté par forme sans lire le registre (cas 6), **et** l'addon
      honore la variable dans son propre environnement.
- [ ] Le canari porte trois sondes `egress_sentinel_*` ; la première rougit si
      l'injection casse, les deux autres si la coupure n'est pas déployée.
- [ ] `make test-egress-sentinel-enforcement` existe, passe, et est appelée par
      `ci.yml` dans le commit qui la crée.
- [ ] `make test-egress-sentinel-staging`, `make test-github-token-not-in-sandbox`,
      `make test-sandbox-gh-usable`, `make test-sandbox-git-usable`,
      `make verify-no-secret-in-setenv` passent.
- [ ] La case « posé par `git` ? oui, déjà » de la note de solution du vol 1 est
      rectifiée.
- [ ] `cargo clippy` et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte la décision explicite sur le registre illisible (§ 3.2),
      la ligne `Tracked in:` du § 2.4, et
      `Code review: delegated to mika-qa (mika#2694)`.

## 8. Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § *Travail attendu* (points 1-7) et de son § *Mode
de défaillance*. AC1-AC7 suivent les sept points du ticket ; AC8-AC10 couvrent les
trois défaillances que la lecture du code et la mesure du § 2.1 ont ajoutées à la
liste et que le ticket ne pouvait pas connaître.

1. **AC1 — le credential présenté est lu avant d'être retiré.** Sur
   `api.github.com`, `Bearer <s>`, `token <s>` et `Basic base64("x:<s>")` avec `s`
   enregistrée mènent tous trois au même verdict `allow` et à l'injection du jeton
   de l'hôte. (Cas 1-3.)
2. **AC2 — validation contre le registre-répertoire.** La comparaison passe par
   `hmac.compare_digest` ; deux sentinelles enregistrées concurremment sont
   valides au même instant. (Cas 1, et l'invariant de concurrence que le vol 1 a
   fermé par construction.)
3. **AC3 — 403 à deux motifs distincts.** *Aucune sentinelle présentée* et
   *sentinelle inconnue du registre* produisent des corps de réponse et des lignes
   de log **différents**, et l'inégalité est assertée, pas affirmée. (Cas 4, 5.)
4. **AC4 — log `DENY`.** Une ligne greppable par refus, sans la valeur présentée,
   sans le jeton de l'hôte, sans empreinte. (Cas 10.)
5. **AC5 — purge TTL.** Les entrées au-delà de 72 h disparaissent ; une entrée
   fraîche et l'entrée validée dans le même appel survivent ; une `OSError` de
   suppression ne se déguise pas en registre illisible. (Cas 9.)
6. **AC6 — l'interrupteur vaut des deux côtés.** `MIKA_PILOT_EGRESS_SENTINEL=0`
   fait présenter le placeholder nu, et l'addon l'accepte **sans lire le
   registre** — prouvé en rendant le registre illisible dans le même cas. L'addon
   honore en outre la variable dans son propre environnement. (Cas 6.)
7. **AC7 — la sonde n'est plus muette sur le refus.** Le canari rend
   `egress_sentinel_accepted=200`, `egress_sentinel_denied_absent=403` et
   `egress_sentinel_denied_unknown=403`, chacune nommant son motif. « L'addon
   refuse tout » et « l'addon n'a rien vu passer » ne rendent plus les mêmes
   octets. (§ 3.3.)
8. **AC8 — le chemin `git` n'est pas 403.** Aucune requête vers `github.com` ne
   reçoit un 403 que `origin/main` n'aurait pas produit, quel que soit le
   credential présenté ; `make test-sandbox-git-usable` passe. C'est le critère
   que le § 2.1 impose : sans lui, le vol tue tous les `git push` de pilote au
   `make deploy`. (Cas 8.)
9. **AC9 — fail-closed écrit, et survivable.** Un registre illisible refuse, avec
   un motif qui lui est propre et distinct des deux autres ; l'asymétrie avec le
   fail-open du lanceur est **écrite dans le code**, pas seulement dans la PR.
   (Cas 7, § 3.2.)
10. **AC10 — la coupure est gardée par le pipeline.** `make test-egress-sentinel-enforcement`
    est appelée par `ci.yml` dans le commit qui crée la suite, et elle rougit à la
    fois quand la coupure cesse de refuser **et** quand elle se met à refuser le
    chemin légitime. (§ 4.)

## 9. Risques et arbitrages

| # | risque | traitement |
|---|---|---|
| R1 | **Un bug d'injection 403 tous les appels légitimes et la boucle s'arrête sans diagnostic.** Le risque nommé par le ticket parent, et le seul qui puisse arrêter la boucle. | Quatre couches, indépendantes. (1) La coupure est bornée à `api.github.com` — `git push` ne peut pas être 403 (§ 2.3, AC8). (2) Le cas 1 et le cas 8 sont des positifs en CI : un addon qui refuse tout rougit **avant** le merge. (3) La sonde 1 du canari rougit après `make deploy`, sur une ligne, pas par un silence. (4) `MIKA_PILOT_EGRESS_SENTINEL=0` désarme des deux côtés, sans redéploiement. |
| R2 | **`gh` n'émet pas `Bearer`.** Un lecteur qui n'accepte que `Bearer` 403 **tous** les appels `gh` — R1 par omission plutôt que par bug. | **Mesuré au grooming, plus supposé** : le binaire installé porte `token %s` (×2) *et* `Bearer %s` (×1) — § 3.1. Trois schémas acceptés, donc les deux chemins vivants du client passent. Le cas 2 l'épingle en CI. La capture sur le fil reste au DoD, mais elle confirme désormais au lieu de décider. |
| R3 | **Registre illisible côté addon.** Refuser arrête la boucle ; autoriser annule le ticket. | **Refuser**, avec un motif propre, et rendre le refus survivable : le placeholder nu est accepté **par forme, sans lire le registre**, donc le rollback ne dépend pas du référentiel en panne (§ 3.2, AC6, AC9). C'est la décision que le ticket demande de poser explicitement. |
| R4 | **La purge refuse un pilote vif.** Une entrée purgée sous un pilote long produit un `sentinel-unknown` qui ressemble à l'incident qu'on garde. | TTL à 72 h et non 24 ; purge **avant** le calcul de la liste rendue, jamais après un verdict ; entrées de 0 octet, donc le TTL généreux ne coûte rien. Cas 9. Et le motif `sentinel-unknown` nomme dans son corps la purge comme cause possible, pour que le remède soit lisible. |
| R5 | **Un `cargo test` qui pose `GH_TOKEN=proxy-managed-no-secret` passe.** Le placeholder nu étant accepté par forme, un test qui tombe exactement sur cette valeur est autorisé. | Assumé, et c'est le prix du rollback complet exigé par le point 6. La valeur est publique et documentée depuis mika#2572 ; mais la signature mesurée de mika#2713 est `GH_TOKEN="fake-token"`, pas le placeholder. Fermer ce cas demanderait de retirer le rollback, ce que le ticket interdit explicitement. |
| R6 | **Deux pilotes concurrents.** mitmdump est long-vivant et partagé. | Fermé par construction au vol 1 (registre-répertoire). Ce vol n'a qu'à ne pas le rouvrir : la purge opère sur le `st_mtime` de chaque entrée, jamais sur le répertoire entier. |
| R7 | **La branche `Basic` est du code que rien n'emprunte** (classe mika#2205). | Dit franchement plutôt que tu : aucun client de production ne l'emprunte aujourd'hui (§ 3.4 cas 3). Elle est livrée parce que le ticket l'exige et parce qu'un `curl -u` légitime vers l'API ne doit pas être traité autrement qu'un `Bearer` ; elle est exercée par le cas 3 ; et elle est la variable d'ajustement nommée à l'avance si la mesure réelle dépasse la borne (§ 5). |
| R8 | **Le registre du lanceur et celui du lecteur divergent** (surcharge de `_PILOT_EGRESS_SENTINEL_DIR` d'un seul côté, ou `HOME` différent pour mitmdump). Tout devient `sentinel-unknown`. | Un **seul** nom de variable pour les deux moitiés (§ 3.1), résolu à l'appel des deux côtés, et le motif `registry-unreadable` sépare « le registre ne répond pas » de « la valeur n'y est pas » — sans quoi les deux rendraient les mêmes octets et le remède serait indevinable. Documenté au § opérateur. |

## 10. Hors périmètre, délibérément

- **`github.com` / le chemin `git`.** § 2.1 à 2.4 : la coupure y serait un 403 sur
  tous les `git push` de pilote, et la dotation qui l'éviterait n'achèterait
  aucune propriété. Le reste est nommé par le ticket de suivi du § 2.4.
- **L'egress Anthropic.** `ANTHROPIC_API_KEY` garde son placeholder statique. Le
  mode de défaillance de mika#2713 est propre à GitHub (écriture sur une forge).
- **Le `cargo test` qui n'écrase pas `GH_TOKEN`.** Il hérite de la sentinelle,
  présente un justificatif valide, et passe — même après ce vol. Cette moitié est
  fermée côté code par mika#2713 (`run_gh` injectable, scan structurel, plancher
  de numéro de fixture), et **aucune** sentinelle ne peut la fermer : un secret
  présent dans la sandbox est lisible par tout processus de la sandbox (§ 2.1 du
  plan du vol 1).
- **Le shim `gh` et `http.extraHeader` comme canaux d'une sentinelle dédiée.**
  Écartés au § 2 du plan du vol 1 et enregistrés dans la note de solution. Ce vol
  ne les rouvre pas : le § 2.2 réemploie leur argument contre la dotation de
  `git`, il ne le relitige pas.
- **Une sentinelle cryptographiquement signée, ou un canal hors de l'environnement
  de la sandbox.** Le modèle de menace est la classe *accident*, pas la classe
  *adversaire* (§ 2.1 du plan du vol 1, note de solution § *Modèle de menace*). Un
  processus hostile de la sandbox lit sa propre sentinelle dans `/proc/self/environ`
  et la rejoue ; cette moitié reste couverte par mika#2056 (la sandbox ne détient
  jamais le jeton).

## 11. Référence

- Ticket : senara-solutions/mika#2724 — parent : senara-solutions/mika#2716
- PR du vol 1 : senara-solutions/mika#2723
- Plan du vol 1 (les deux vols) :
  `docs/plans/2026-10-15-003-feat-2716-egress-sentinelle-pilote-attestee-plan.md`
  § 2 (choix du canal), § 3 (découpage), § 9 (estimation), § 10 R1, § 11
- Note de solution (choix du canal, **à rectifier** § 2.3) :
  `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
- Surface opérateur à étendre : `docs/operator/environment-variables.md`
  § *La sandbox de pilote présente un justificatif d'egress (mika#2716)*
- Doctrines invoquées : mika#1949 (nommer, pas divulguer), mika#2165 (un
  interrupteur que son second lecteur n'honore pas est décoratif), mika#2205 (deux
  états qui rendent les mêmes octets), mika#2578 (une garde que le pipeline ne
  lance pas ne refuse rien), mika#2701 (le suivi est ouvert avant la PR)

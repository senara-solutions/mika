# fix(infra) — fermer la moitié `git push` de l'egress de pilote : ce qui est atteignable, et ce qui ne l'est pas (mika#2736)

**Ticket :** senara-solutions/mika#2736
**Ticket parent (la moitié API, livrée) :** senara-solutions/mika#2716 — vol 1 : #2723, vol 2 : #2724
**Jalon :** Substrat de boucle
**Labels :** `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`

---

## 1. Pourquoi, et ce que ce plan répond

Le ticket pose une alternative : **fermer** la moitié `git push`, ou **établir
qu'elle est hors d'atteinte** — et écrire l'une ou l'autre là où un opérateur la
cherche.

Les mesures du § 2 rendent la réponse **mixte, et elle l'est par mesure** : trois
des quatre sous-classes de la menace sont fermables **avec ce que le proxy voit
déjà**, et la quatrième est hors d'atteinte côté proxy pour une raison qui tient
au protocole, pas à l'effort. Le plan livre les trois, et documente la quatrième
avec la mesure qui la soutient.

Il corrige au passage **une prémisse du corps du ticket**. Le point 2 énonce que
« l'addon voit l'URL, pas le refspec ». C'est faux, et c'est précisément pour
cela que le ticket demandait de mesurer avant de bâtir : le refspec est en
clair, au début du corps du `POST git-receive-pack` (§ 2.2). Le canal qui
paraissait aveugle est celui qui voit le plus.

## 2. Mesures — exécutées au grooming, pas déduites

Toutes les lignes ci-dessous viennent de deux sondes réellement lancées le
2026-10-17 : un serveur git smart-HTTP minimal sur `127.0.0.1:0` qui journalise
ce qu'il reçoit, et un `git push` dirigé vers lui. Aucune n'est une lecture
d'argv, aucune ne touche la forge. La méthode est refaisable en ≈ 200 lignes de
Python jetable ; l'implémenteur n'a pas à la refaire, il doit seulement ne pas la
contredire sans la refaire.

### 2.1 Le modèle de menace de cette moitié, mesuré plutôt que supposé

Le point 1 du ticket exige de mesurer ce qu'un `push` arbitraire atteint
réellement **compte tenu des bornes de mika#2141**. La réponse est : mika#2141
n'en borne **rien**, et la raison est structurelle.

mika#2141 borne le **gitdir local** — quelles refs du dépôt hôte la sandbox peut
écrire, et quels worktrees elle peut lire. Un `git push` ne passe par aucune de
ces bornes : sa destination est une **URL réseau**, et son contenu n'importe quel
objet accessible. La commande

```
git push https://github.com/<quelconque>/<quelconque>.git <n-importe-quoi>:refs/heads/<n-importe-quoi>
```

ne lit ni n'écrit une ref hôte hors du périmètre de mika#2141 : elle ouvre une
connexion. Et sur cette connexion, `_auth_header_for`
(`scripts/mika-pilot-github-auth-addon.py:120-131`) injecte le jeton de l'hôte
**en ne regardant que l'hôte** — jamais le chemin. La portée d'un push depuis la
sandbox est donc **exactement la portée du jeton de l'hôte**, sur tout dépôt
qu'il atteint.

**L'incident fondateur de cette moitié existe, il est daté, et ce n'est pas
mika#2713.** C'est **mika#1318** (2026-05-27) : un pilote dev-groom a lancé
`git push --force-with-lease` depuis son worktree et **effacé du travail de
substrat sur le remote** (les clones rebasés de #1314/#1315/#1316). Le ticket
nomme deux faits qui décident de la forme du correctif :

> Le self-heal était **fortuit** — il exigeait qu'un dispatch suivant tire sur le
> même worktree […] Sans #1289 ou sans un dispatch immédiat, le remote serait
> resté dans l'état destructeur jusqu'à une intervention manuelle.

et

> La même forme destructrice a été reproduite sur le dernier groom de mika#736
> […] Le motif n'est pas isolé.

**Ce que coûte chaque sous-classe, et comment elle se répare** — la décomposition
que le point 1 demande, séparément de mika#2713 :

| # | sous-classe | ce qu'elle atteint | réparation | fermable ? |
|---|---|---|---|---|
| P1 | push vers un **dépôt** que le jeton atteint mais que la boucle ne travaille pas | exfiltration de code vers un dépôt tiers, écriture sur `mika-cloud` depuis un pilote `mika` | suppression manuelle, et l'exfiltration est **irréparable** | **oui** — § 2.2 |
| P2 | **suppression** d'une ref distante | perte d'une branche non mergée | reflog local, s'il existe encore un clone | **oui** — § 2.2 |
| P3 | push vers une ref **hors du périmètre du spawn**, `refs/heads/main` compris | écriture sur une branche d'un autre travail | revert, ou force-push correctif | **oui** — § 2.2 |
| P4 | **force-push sur la ref du spawn** — mika#1318 | destruction d'un travail distant plus avancé que le local | reflog du remote (absent), ou re-rebase fortuit | **non** — § 2.4 |

Comparée à mika#2713, cette moitié est **plus coûteuse et moins réparable** : 29
commentaires sur un ticket fermé se suppriment un par un ; un historique
force-pushé ne se récupère nulle part côté serveur. C'est l'argument qui justifie
de fermer P1–P3 même en laissant P4 ouverte.

### 2.2 Ce que le proxy voit réellement d'un `git-receive-pack`

Le point 2 du ticket demandait cette mesure **avant** de bâtir. Elle renverse la
prémisse :

```
GET  /senara-solutions/mika.git/info/refs?service=git-receive-pack
POST /senara-solutions/mika.git/git-receive-pack
    Content-Type: application/x-git-receive-pack-request
    Content-Length: 352          (ni gzip, ni chunked, à cette taille)
    --- pkt-lines au début du corps ---
    [0] b'0000000000000000000000000000000000000000 232d30b1…82b
         refs/heads/feat/9999/probe-slug\x00 report-status'
    [flush-pkt à l'offset 132]
    la suite commence par: b'PACK\x00\x00\x00\x02'
```

Et pour une **suppression** (`git push url :refs/heads/main`) :

```
    [0] b'0000000000000000000000000000000000000000
         0000000000000000000000000000000000000000 refs/heads/main\x00 report-status'
```

**Trois faits, chacun porteur :**

1. **Le dépôt est dans l'URL.** `/senara-solutions/mika.git/git-receive-pack` —
   donc P1 est décidable dès `requestheaders`, sans toucher au corps.
2. **Les commandes de ref sont en clair, en tête du corps**, non compressées, au
   format `<old-sha> <new-sha> <refname>\0<capabilities>`. Donc P2 et P3 sont
   décidables.
3. **Une suppression se lit à `new-sha` tout à zéro.** Aucune heuristique : c'est
   la forme que le protocole impose.

### 2.3 Les trois cas limites qui 403'raient la boucle si on les ignorait

Mesurés sur un push de 4 MiB de données incompressibles. Chacun est un mode de
défaillance du garde, pas un détail.

**(a) Un gros push émet DEUX `POST`, et le premier ne porte aucune commande.**

```
POST /senara-solutions/mika.git/git-receive-pack
    Transfer-Encoding=None Content-Length='4'
    framing=Content-Length, 4 bytes
    first pkt-line (0 bytes): b''          ← un flush-pkt NU
POST /senara-solutions/mika.git/git-receive-pack
    Transfer-Encoding='chunked' Content-Length=None
    framing=chunked, 4195915 bytes
    first pkt-line (125 bytes): b'0000…000 e7d978a8… refs/heads/feat/9999/big\x00 report-status'
```

Le premier POST est la **sonde d'authentification** que git envoie avant de
streamer : 4 octets, un flush-pkt seul. **Un garde qui exigerait « au moins une
commande » refuserait cette sonde, donc tout push assez gros pour être
streamé** — mot pour mot le mode de défaillance que mika#2716 nomme comme le seul
capable d'arrêter la boucle. Le corps vide doit donc **passer**, et c'est le
second POST qui est jugé.

**(b) Au-delà du seuil, le corps part en `Transfer-Encoding: chunked`** — mais les
commandes restent **en tête**. Le hook `requestheaders` ne les voit pas (le corps
n'est pas encore là) : le verdict sur les refs vit donc dans `request`, où
mitmproxy a bufferisé le corps. L'injection, elle, reste dans `requestheaders` et
n'est pas touchée.

**(c) `stream_large_bodies=10m` est posé sur le mitmdump de la boucle**
(`dispatch-lib.sh:1228`). Au-delà de 10 Mo, mitmproxy **streame** et
`flow.request.content` n'est pas disponible. Un pack de pilote est de l'ordre du
kilo-octet — mais l'arbitrage doit être posé, pas découvert : voir § 3.3.

### 2.4 P4 — le force-push est hors d'atteinte côté proxy, et voici pourquoi

Le protocole `receive-pack` **ne porte aucun drapeau de force**. Le client envoie
`<old-sha> <new-sha> <ref>` et rien d'autre ; c'est le **serveur** qui décide,
en résolvant l'ancestralité, si la mise à jour est un fast-forward. La mesure du
§ 2.2 le montre directement : la commande d'un push ordinaire et celle d'un
force-push ont la **même forme**, et `--force-with-lease` renseigne `old-sha`
aussi honnêtement qu'un push normal.

Pour distinguer les deux, un proxy devrait résoudre l'ancestralité de `new-sha`
par rapport à `old-sha` — donc **décoder le packfile en vol** et marcher le
graphe, sur le chemin de la boucle, pour une décision qu'il devrait rendre avant
de relayer. C'est hors de proportion avec la propriété achetée, et ce serait du
code fragile placé exactement là où une panne arrête la boucle.

**P4 reste donc ouverte, et elle est déjà gardée ailleurs, plus faiblement :**

- `skills/bundled/dev-groom/system_prompt.md:17-23` interdit au pilote de groom
  tout `git push`, force-push compris, en nommant mika#1318. C'est une **consigne
  de prompt**, pas un mécanisme — la garde qui a été ajoutée *en réponse* à
  l'incident, et qui n'est contraignante que dans la mesure où un pilote lit son
  prompt.
- `_push_branch` (`dispatch-lib.sh:6993`) est le site unique de push pour les
  dispatches de groom, et il tourne **côté hôte**, hors de la sandbox — donc hors
  du proxy, et hors de portée de tout garde d'egress.

C'est la moitié de réponse « hors d'atteinte » que le ticket autorise, pour une
sous-classe nommée, avec la mesure qui la soutient. Elle est écrite dans les deux
surfaces du point 3 (§ 6, commit 4), parce qu'une limite connue et tue est une
surprise d'opérateur.

### 2.5 Les canaux écartés, avec la raison qui les écarte

Le point 2 exige de consigner les refus **pour que la question ne soit pas
rouverte à l'identique**.

**`http.extraHeader` via `GIT_CONFIG_KEY_n` — écarté, et pour la première fois
par mesure.** Le vol 1 l'avait écarté dans une table de comparaison ; ici il est
réexaminé à son propre mérite, comme le ticket le demande. Il **fonctionne**, ce
qui n'avait jamais été vérifié :

```
GIT_CONFIG_COUNT=1
GIT_CONFIG_KEY_0=http.extraHeader
GIT_CONFIG_VALUE_0='X-Mika-Egress-Sentinel: proxy-managed-no-secret-deadbeef'

  GET  …/info/refs?service=git-receive-pack
      X-Mika-Egress-Sentinel: 'proxy-managed-no-secret-deadbeef'
  POST …/git-receive-pack      (les deux POST, sonde comprise)
      X-Mika-Egress-Sentinel: 'proxy-managed-no-secret-deadbeef'
```

Et c'est la mesure **suivante** qui le condamne. Un second processus, lancé dans
le même environnement, poussant **un autre dépôt vers une ref qu'il choisit** :

```
  POST /senara-solutions/mika.git/git-receive-pack
      X-Mika-Egress-Sentinel: 'proxy-managed-no-secret-deadbeef'   ← la MÊME, valide
      first pkt-line: b'0000…000 0a555b94… refs/heads/whatever-it-wants\x00 report-status'
```

Le justificatif atteste donc **la sandbox**, jamais le chemin légitime — mot pour
mot l'argument que le vol 1 oppose au shim `gh`, cette fois mesuré sur le canal
qui restait à examiner. Et il n'a pas la propriété qui a décidé de mika#2716 : le
mode de défaillance de cette moitié **n'écrase rien**. Un `cargo test` qui pousse
par erreur n'a aucune raison de toucher `GIT_CONFIG_*` ; il hérite d'un
justificatif valide et passe. Deuxième raison, indépendante :
`GIT_CONFIG_KEY_0` est refusé par la règle 2 de
`scripts/verify-no-secret-in-setenv.sh` — c'est ce qui avait déjà fait préférer un
fichier généré à mika#2141 — et le fichier, lui, est `--ro-bind` et lu par tout
processus de la sandbox (§ 2.2 du plan du vol 2).

**Une borne côté bwrap, dans le prolongement de mika#2141 — écartée, pour
impossibilité de catégorie.** bwrap borne un **système de fichiers**. La
destination d'un push est une **URL**. Il n'existe aucun chemin que bwrap puisse
retirer pour empêcher `git push https://…` : le § 2.1 montre qu'un push
n'emprunte aucune des bornes de mika#2141. La seule borne de type bwrap qui
mordrait serait de retirer le réseau au pilote — ce qui retire aussi `gh pr
create` et l'API Anthropic, donc la boucle.

**Ce que le ticket interdit de rouvrir, et qui ne l'est pas** (point 4) : le choix
du canal `GH_TOKEN`, le shim `gh`, et le fait qu'un secret présent dans la
sandbox est lisible par tout processus de la sandbox. Le § 2.5 **réemploie** ce
dernier argument contre `http.extraHeader` ; il ne le relitige pas. Et la borne
retenue ci-dessous **n'est pas un justificatif** : elle ne demande rien au
client, donc elle ne dépend d'aucun canal.

## 3. La décision, et sa conception

### 3.1 La forme retenue — une borne, pas une sentinelle

**Sur `github.com`, l'addon borne ce qu'un `git-receive-pack` peut toucher. Il
n'exige aucun justificatif.**

C'est la conséquence directe du § 2.5 : aucun canal ne peut porter un
justificatif qui atteste le *chemin* plutôt que la *sandbox*. Mais la mesure du
§ 2.2 montre que le proxy n'a pas besoin qu'on lui dise quoi que ce soit — il
**voit** le dépôt et il **voit** les refs. Une borne sur ce qu'il voit déjà ne
dépend d'aucune coopération du client, donc aucun processus de la sandbox ne peut
la contourner en se présentant bien. C'est strictement plus fort qu'une
sentinelle sur cette moitié, et c'est pour cette raison que la réponse au ticket
n'est pas « hors d'atteinte ».

**Les quatre règles, et ce que chacune ferme :**

| règle | motif de refus | ferme |
|---|---|---|
| le chemin du dépôt est sous l'organisation `senara-solutions` | `push-repo-foreign` | P1 — exfiltration vers un dépôt tiers |
| aucune commande ne porte `new-sha` tout à zéro | `push-ref-delete` | P2 |
| chaque `refname` est sous `refs/heads/` | `push-ref-exotic` | P3 — tags, `refs/*` arbitraires |
| aucun `refname` n'est une branche protégée (`main`, `master`) | `push-ref-protected` | P3 — écriture sur la branche d'intégration |

**Allowlist du dépôt : statique, et ce vol s'y tient délibérément.** Resserrer au
**périmètre déclaré par le spawn** (le dépôt *et* la branche que l'hôte connaît
déjà, puisqu'il les a choisis) est strictement meilleur et ferme P3 entièrement —
mais exige d'étendre l'entrée du registre de sentinelles du vol 1, donc de
toucher au lanceur. C'est le vol 2, et il est nommé au § 5.

### 3.2 `scripts/mika-pilot-github-auth-addon.py` — où le verdict vit

Deux fonctions pures, plus un hook `request` qui ne les laisse pas débranchées
(classe mika#2205, citée deux fois par le vol 2).

```python
_DENY_REPO_FOREIGN  = "push-repo-foreign"
_DENY_REF_DELETE    = "push-ref-delete"
_DENY_REF_EXOTIC    = "push-ref-exotic"
_DENY_REF_PROTECTED = "push-ref-protected"
_DENY_PUSH_OPAQUE   = "push-body-unreadable"
```

**`_receive_pack_repo(path) -> str | None`.** Rend `<owner>/<repo>` quand le
chemin est celui d'un `git-receive-pack` (`/<owner>/<repo>.git/git-receive-pack`,
le `.git` étant optionnel), `None` sinon. Un `None` signifie **« ce n'est pas un
push »** et laisse passer : `git-upload-pack` (le fetch), `info/refs`, et tout le
reste de `github.com` restent strictement inchangés. La borne porte sur le seul
verbe qui écrit.

**`_ref_commands(body) -> list[tuple[str, str, str]] | None`.** Décode les
pkt-lines en tête du corps jusqu'au premier flush-pkt et rend les triplets
`(old, new, ref)`. **Une liste VIDE et `None` sont deux états distincts**, et les
confondre est le piège (a) du § 2.3 : une liste vide est la sonde
d'authentification de 4 octets (**autorisée**), `None` est un corps qui existe
mais ne se décode pas (**refusé**, `push-body-unreadable`). Un corps absent —
mitmproxy l'a streamé, § 2.3 (c) — rend `None` lui aussi, et le § 3.3 pose cet
arbitrage.

**`_push_verdict(repo, commands) -> str`.** Pure. L'ordre des branches ne porte
pas de sûreté ici (les règles sont indépendantes), mais l'ordre du **rapport**
compte : la première violation rencontrée nomme le motif, et le corps du 403
nomme la ref ou le dépôt en cause — c'est une donnée du client, pas un secret de
l'hôte, donc la nommer ne divulgue rien (doctrine mika#1949, qui interdit de
nommer **le justificatif**, pas la cible).

**`request(flow)`** — un nouveau hook, à côté de `requestheaders` :

- hôte ≠ `github.com` → inchangé. `api.github.com` garde **entièrement** la
  coupure du vol 2, qui vit dans `requestheaders` et n'est pas touchée.
- le chemin n'est pas un `git-receive-pack` → inchangé.
- sinon : verdict, et `flow.response = Response.make(403, …)` s'il refuse. Le
  commentaire nomme le § 2.3 (b) pour que le choix du hook ne se lise pas comme
  une incohérence avec l'injection, qui reste dans `requestheaders`.

**Le log `DENY`** réemploie la ligne exacte du vol 2 —
`[github-proxy] DENY <motif> <method> <path>` — sans l'élargir. Un motif de plus
dans un vocabulaire existant vaut mieux qu'une seconde forme de ligne à grepper.

### 3.3 Le corps illisible — l'arbitrage à poser, comme le vol 2 a posé le sien

`stream_large_bodies=10m` (§ 2.3 c) crée un état où le garde ne peut pas juger.
Le vol 2 a posé le même type d'arbitrage pour le registre illisible ; celui-ci
est posé ici, et il penche **dans l'autre sens**.

**Un corps illisible REFUSE (`push-body-unreadable`), et le refus est borné au
`git-receive-pack`.** Trois raisons, dont la troisième décide :

1. Un pack de pilote est de l'ordre du kilo-octet : quelques commits de code. Un
   push de plus de 10 Mo depuis une sandbox de pilote est **déjà** l'anomalie,
   pas le régime nominal.
2. Autoriser sur illisibilité donne à qui veut contourner la borne un moyen
   trivial : grossir le corps. Un garde qui s'ouvre quand on le dépasse en taille
   n'est pas un garde.
3. **Et le refus est survivable sans redéploiement**, ce qui est la condition
   sous laquelle un fail-closed est acceptable sur le chemin de la boucle :
   `MIKA_PILOT_EGRESS_PUSH_BOUND=0` désarme cette borne seule, dans
   l'environnement de mitmdump, sans toucher à la coupure du vol 2 ni à
   l'injection. Un interrupteur **distinct** de `MIKA_PILOT_EGRESS_SENTINEL`,
   délibérément : les deux gardes ont des modes de défaillance différents, et un
   opérateur qui désarme l'un ne doit pas désarmer l'autre à son insu.

Le corps du 403 nomme la taille comme cause probable et nomme l'interrupteur.

### 3.4 La suite CI — `skills/bundled/_shared/tests/test_egress_push_bound.sh`

Même patron que `test_egress_sentinel_enforcement.sh` du vol 2 : stub du module
`mitmproxy`, chargement par `importlib`, assertions en Python, `exit` agrégé relu
côté bash. Le stub gagne de quoi exercer `request` avec un corps. Cible
`make test-egress-push-bound`, ajoutée à `ci.yml` dans le bloc mika#2716
existant, **dans le commit qui crée la suite** (leçon mika#2578 : une garde que
le pipeline ne lance pas ne refuse rien).

Onze cas, chacun répondant à une défaillance nommée plus haut :

1. **Le push nominal passe** — `senara-solutions/mika`, `refs/heads/feat/2736/x`,
   `new-sha` non nul — et l'`Authorization` sortant est **celui de l'hôte**.
   C'est l'anti-vacuité de tous les autres : un garde qui refuserait tout rougit
   ici.
2. **La sonde d'authentification de 4 octets passe** (corps = flush-pkt nu).
   Le piège (a) du § 2.3, épinglé — sans ce cas, tout push streamé serait refusé
   et personne ne le verrait avant le premier dispatch.
3. **Corps chunked de grande taille, commandes en tête → passe.** Le cas (b).
4. **Dépôt hors organisation → 403** `push-repo-foreign`.
5. **Suppression (`new-sha` = 0×40) → 403** `push-ref-delete`.
6. **`refs/heads/main` → 403** `push-ref-protected`.
7. **`refs/tags/v1` → 403** `push-ref-exotic`.
8. **Plusieurs commandes dont une seule viole → 403**, et le corps nomme **la**
   ref fautive. Une borne qui ne juge que la première commande est contournée par
   une commande anodine en tête.
9. **Corps absent (mitmproxy a streamé) → 403** `push-body-unreadable`, et
   `MIKA_PILOT_EGRESS_PUSH_BOUND=0` fait passer le même cas (§ 3.3).
10. **`git-upload-pack` et `info/refs` sur `github.com` → strictement
    inchangés**, et `api.github.com` conserve exactement le comportement du
    vol 2 : non-régression des deux moitiés déjà livrées.
11. **Les cinq corps de 403 sont distincts deux à deux**, assertion d'inégalité.
    Sans elle, « cinq motifs » est une affirmation du plan et non une propriété
    du code (classe mika#2205).

### 3.5 Le canari

Le bloc « must-fail » de `scripts/canary-pilot-containment` gagne deux sondes, et
le bloc « must-work » une. La première est l'anti-vacuité des deux autres.

| sonde | attendu | ce qu'elle nomme |
|---|---|---|
| `git push` nominal (dry-run sur la branche du canari) | `push_bound_nominal=ok` | la borne n'a pas cassé le chemin légitime |
| `git push` vers un dépôt hors organisation | `push_bound_foreign=403` | P1 est fermée en vol |
| `git push` d'une suppression de ref | `push_bound_delete=403` | P2 est fermée en vol |

Deux contraintes de rédaction que le fichier impose : **aucune quote simple**
dans les messages ajoutés (le bloc vit dans un payload `sh -c '…'` entre quotes
simples — leçon mika#2572, déjà payée deux fois), et la cible des sondes
négatives doit être un dépôt qui n'existe pas, pour qu'un 403 ne puisse jamais
être confondu avec une écriture réelle qui aurait abouti.

## 4. Fire-Disposition

Ce vol livre des détecteurs : la suite `test_egress_push_bound.sh` et les trois
sondes du canari.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist VIDE.**

- **Aucun site en violation à l'instant du merge, et c'est vérifié par
  énumération plutôt que par un `grep` vide.** Le seul push qui traverse ce proxy
  est celui qu'un pilote lance **depuis la sandbox**, vers `origin`
  (`senara-solutions/<repo>`) sur `refs/heads/<BRANCH>` de son propre dispatch
  (`dispatch-lib.sh:6575`, mika#2151) — conforme aux quatre règles.
  **Le site qui violerait `push-ref-exotic` existe, et il n'est pas candidat à
  une exemption :** `dispatch-lib.sh:3416-3418` pousse
  `refs/heads/<branch>:refs/dispatch-gate/<branch>` — une ref délibérément hors
  de `refs/heads/` — mais ce push, comme `_push_branch` (`:6993`) et son
  `--force-with-lease` (`:7224-7311`), tourne **côté hôte**, hors bwrap, donc
  sans jamais traverser mitmdump. La borne ne le voit pas, et une entrée
  d'allowlist pour lui serait une exemption sans violation — la surface d'attaque
  décrite par
  `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`.
  **L'implémenteur doit vérifier cette frontière plutôt que la croire :** si un
  jour un de ces pushs hôte passait par le proxy, c'est le cas 1 du § 3.4 qui le
  dirait, pas une relecture.
- **Assertion auto-nettoyante — les cas 1, 2, 3 et 10.** Le risque de péremption
  d'un détecteur de refus n'est pas qu'il cesse de refuser, c'est qu'il refuse
  **tout** et lise vert. Les cas 1 (le push nominal passe), 2 (la sonde de 4
  octets passe), 3 (le gros push passe) et 10 (`upload-pack` et
  `api.github.com` intacts) sont les positifs qui rougissent exactement dans ce
  cas-là. Le cas 11 rougit si les motifs fusionnent.
- **Aucun détecteur n'atterrit désarmé, ni hors pipeline.** Pas de `#[ignore]` ni
  d'équivalent ; la suite entre dans `ci.yml` dans le commit qui la crée. Le
  canari reste hors CI parce qu'il est post-déploiement par nature — c'est
  précisément pourquoi la suite existe à côté de lui.
- **Aucun détecteur de ce vol ne défaille en silence.** Le seul dont le mécanisme
  pouvait se vider — une fonction pure verte et débranchée — porte son
  anti-vacuité dans les cas eux-mêmes : 1, 2, 3 et 5 passent par `request`, pas
  par un appel direct à `_push_verdict`.

## 5. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/mika-pilot-github-auth-addon.py` (`_receive_pack_repo`, `_ref_commands`, `_push_verdict`, hook `request`, 5 motifs + corps de 403, interrupteur dédié) | 155 |
| tests (`skills/bundled/_shared/tests/test_egress_push_bound.sh`, 11 cas + stub de corps) | 145 |
| `scripts/canary-pilot-containment` (trois sondes + prose du rollback) | 30 |
| `Makefile` (cible) + `.github/workflows/ci.yml` (entrée) | 8 |

Total estimé : 338 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et en un seul.** 338 < 429, soit 91 lignes de marge, et
338 < 1000. Rien ici ne touche `dispatch-lib.sh` : la borne lit ce que le proxy
voit déjà, donc le lanceur, le registre et le format de ses entrées sont tous
hors du diff. C'est la raison principale du total — et c'est aussi ce qui rend le
vol mergeable seul, puisqu'il ne dépend d'aucun redéploiement du chemin de
dispatch au-delà du `cp` de l'addon que `Makefile:55-57` fait déjà.

**Ce que la marge paie, nommément.** Si la mesure réelle dépasse, la variable
d'ajustement est la règle **`push-ref-exotic`** (≈ 10 lignes d'addon et ≈ 12 de
test, cas 7) : c'est la seule des quatre dont aucune sous-classe mesurée du § 2.1
ne dépend — `refs/tags/*` n'atteint ni `main`, ni un dépôt tiers, ni une
suppression. Elle part alors avec son test, jamais sans, et le corps de PR le
dit. **Ce ne sont ni le cas 2 ni le cas 3** : ils épinglent les deux pièges
mesurés au § 2.3, et les retirer livrerait une borne qui 403 tout push streamé
sans que rien ne le dise avant le premier dispatch.

**La suite, découpée en vol plutôt qu'en phase.** Le resserrement au **périmètre
déclaré par le spawn** (§ 3.1) ferme P3 entièrement et remplace l'allowlist
statique du dépôt par le dépôt réel du dispatch. Il exige d'étendre l'entrée du
registre de sentinelles du vol 1 de mika#2716 — donc de toucher
`_stage_pilot_egress_sentinel`, son format d'entrée, sa purge et ses tests, plus
la lecture côté addon : ≈ 230 lignes supplémentaires, qui porteraient ce vol à
≈ 568 et au-dessus de la borne. Il est donc renvoyé à un vol distinct, mergeable
seul, et ouvert **avant** la PR (règle mika#2701).

Tracked in: un ticket de suivi ouvert avant la PR et nommé dans son corps,
portant « borner le `git-receive-pack` de pilote au périmètre déclaré par le
spawn (vol 2 de mika#2736) ».

**P4 n'est pas une suite**, c'est une limite : le § 2.4 établit qu'elle est hors
d'atteinte côté proxy, et le commit 4 l'écrit dans les deux surfaces d'opérateur
au lieu de l'inscrire comme dette.

## 6. Ordre de commit, et son plancher

Chaque commit laisse l'arbre vert. Le plancher est le **commit 2**.

1. `feat(infra)` — l'addon : `_receive_pack_repo`, `_ref_commands`,
   `_push_verdict`, le hook `request`, les cinq motifs et leurs corps, le log
   `DENY`, l'interrupteur dédié. *(L'arbre reste vert : aucune suite existante
   n'exerce `request`, et `requestheaders` n'est pas touché. Rien n'est déployé —
   l'addon n'est actif qu'après le `cp` de `Makefile:55-57`.)*
2. **PLANCHER** — `test_egress_push_bound.sh` + la cible `Makefile` + l'entrée
   `ci.yml`. S'arrêter avant le commit 2 livrerait une borne fail-closed sur le
   chemin de la boucle qu'aucune garde de pipeline n'épingle : le seul état de cet
   arbre qui soit pire que l'absence du ticket.
3. `feat(infra)` — les trois sondes du canari.
4. `docs` — **le livrable du point 3 du ticket**, dans les deux surfaces qu'il
   nomme : la section du justificatif d'egress de
   `docs/operator/environment-variables.md` (les cinq motifs, l'interrupteur
   `MIKA_PILOT_EGRESS_PUSH_BOUND`, ce que la borne ferme et ce qu'elle ne ferme
   pas) et la note de solution
   `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
   (le modèle de menace du § 2.1 avec mika#1318 comme incident fondateur, le
   refus mesuré de `http.extraHeader` au § 2.5, et **P4 comme limite connue** avec
   la mesure du § 2.4). La note gagne aussi une ligne dans sa table des canaux :
   une borne qui ne demande rien au client n'y figurait pas, et son absence est
   ce qui a fait croire la question close.

**Une action précède la PR, et ce n'est pas un commit de cet arbre :** le ticket
de suivi du § 5, ouvert **avant** l'ouverture de la PR et nommé dans son corps
(règle mika#2701).

**Aucune mesure n'est prérequise au commit 1.** Les quatre du § 2 sont acquises au
grooming et ne sont pas à refaire ; les deux sondes qui les ont produites sont
décrites au § 2 avec assez de détail pour être reconstruites si elles sont
contredites.

## 7. Definition of Done

- [ ] Sur `github.com`, un `POST git-receive-pack` vers un dépôt hors
      `senara-solutions` rend 403 `push-repo-foreign`.
- [ ] Une commande portant `new-sha` tout à zéro rend 403 `push-ref-delete` ;
      un `refname` hors `refs/heads/` rend 403 `push-ref-exotic` ;
      `refs/heads/main` et `refs/heads/master` rendent 403 `push-ref-protected`.
- [ ] Les **cinq** corps de 403 sont distincts deux à deux, et l'inégalité est
      assertée (cas 11).
- [ ] **Le push nominal d'un pilote passe** — `senara-solutions/<repo>`,
      `refs/heads/<branche de dispatch>` — et l'`Authorization` sortant est celui
      de l'hôte (cas 1).
- [ ] **La sonde d'authentification de 4 octets passe** (corps = flush-pkt nu,
      aucune commande) : une liste vide et `None` ne sont pas confondus (cas 2).
- [ ] **Un push chunked de grande taille passe**, commandes lues en tête du corps
      (cas 3).
- [ ] Une requête portant plusieurs commandes est refusée si **l'une quelconque**
      viole, et le corps nomme la ref fautive (cas 8).
- [ ] Un corps indisponible rend 403 `push-body-unreadable`, et
      `MIKA_PILOT_EGRESS_PUSH_BOUND=0` fait passer le même cas (cas 9).
- [ ] `git-upload-pack` et `info/refs` sur `github.com` sont **strictement
      inchangés**, et la coupure du vol 2 sur `api.github.com` l'est aussi
      (cas 10).
- [ ] Une ligne `[github-proxy] DENY <motif> <method> <path>` par refus, dans le
      vocabulaire exact du vol 2, sans la valeur d'un credential.
- [ ] Le canari porte `push_bound_nominal`, `push_bound_foreign` et
      `push_bound_delete` ; la première rougit si la borne casse le chemin
      légitime.
- [ ] `make test-egress-push-bound` existe, passe, et est appelée par `ci.yml`
      dans le commit qui la crée.
- [ ] `make test-egress-sentinel-enforcement`, `make test-egress-sentinel-staging`,
      `make test-github-token-not-in-sandbox`, `make test-sandbox-gh-usable`,
      `make test-sandbox-git-usable`, `make verify-no-secret-in-setenv` passent.
- [ ] **Le point 3 du ticket est livré :** `docs/operator/environment-variables.md`
      et la note de solution portent le modèle de menace du § 2.1, le refus mesuré
      de `http.extraHeader` (§ 2.5), et **P4 comme limite connue** avec la mesure
      du § 2.4.
- [ ] Le ticket de suivi du § 5 est ouvert **avant** la PR et nommé par une ligne
      `Tracked in:` dans son corps.
- [ ] `scripts/verify-pipeline.sh origin/main` passe.
- [ ] Le corps de PR porte l'arbitrage du § 3.3 (corps illisible → refus), la
      ligne `Tracked in:`, et
      `Code review: delegated to mika-qa (mika#2694)`.

## 8. Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § *Travail attendu* (points 1-4) et de son § *Hors
périmètre*. AC1-AC4 suivent les quatre points du ticket ; AC5-AC8 couvrent les
défaillances que les mesures du § 2 ont ajoutées à la liste et que le ticket ne
pouvait pas connaître.

1. **AC1 — le modèle de menace de cette moitié est énoncé, séparément de
   mika#2713, et mesuré.** Les quatre sous-classes P1-P4 sont nommées avec ce que
   chacune atteint et comment elle se répare ; l'affirmation « mika#2141 borne le
   push » est **réfutée par mesure** (§ 2.1) plutôt que reprise ; l'incident
   fondateur de la classe est identifié (mika#1318, pas mika#2713). Vérifiable
   dans la note de solution après le commit 4.
2. **AC2 — les canaux candidats sont examinés et les refus consignés avec leur
   raison.** `http.extraHeader` via `GIT_CONFIG_KEY_n` est réexaminé **à son
   propre mérite** et écarté sur une mesure qui montre qu'il atteste la sandbox et
   non le chemin (§ 2.5) ; la borne côté addon est **mesurée avant d'être bâtie**,
   et la mesure corrige la prémisse du ticket (le refspec est visible, § 2.2) ;
   la borne côté bwrap est écartée pour impossibilité de catégorie. Les trois
   raisons vivent dans la note de solution, pas seulement dans ce plan.
3. **AC3 — la moitié `git push` est fermée pour P1, P2 et P3**, par une borne qui
   ne demande **rien** au client, donc qu'aucun processus de la sandbox ne
   contourne en se présentant bien (§ 3.1). Les quatre règles sont exercées en
   CI, et deux d'entre elles en vol par le canari.
4. **AC4 — ce qui reste ouvert est écrit là où un opérateur le cherche.** P4 (le
   force-push sur la ref du spawn, mika#1318) est établie hors d'atteinte côté
   proxy **avec la mesure qui le soutient** — le protocole receive-pack ne porte
   aucun drapeau de force (§ 2.4) — et cette limite est inscrite dans
   `docs/operator/environment-variables.md` **et** dans la note de solution, avec
   les deux gardes faibles qui la couvrent aujourd'hui.
5. **AC5 — la sonde d'authentification ne doit jamais être refusée.** Un corps
   réduit à un flush-pkt nu passe. C'est le critère que la mesure du § 2.3 (a)
   impose : sans lui, le vol 403 chaque push assez gros pour être streamé — le
   seul mode de défaillance capable d'arrêter la boucle.
6. **AC6 — la non-régression des deux moitiés déjà livrées.** Rien de ce vol ne
   change `requestheaders` : l'injection sur `github.com`, la coupure du vol 2 sur
   `api.github.com`, et `git-upload-pack` se comportent comme avant (cas 10).
7. **AC7 — l'arbitrage du corps illisible est écrit et survivable.** Un corps que
   mitmproxy a streamé refuse, avec un motif qui lui est propre, et
   `MIKA_PILOT_EGRESS_PUSH_BOUND=0` désarme cette borne **seule**, sans toucher à
   la coupure du vol 2 (§ 3.3, cas 9).
8. **AC8 — la borne est gardée par le pipeline.** `make test-egress-push-bound`
   est appelée par `ci.yml` dans le commit qui crée la suite, et elle rougit à la
   fois quand la borne cesse de refuser **et** quand elle se met à refuser le
   chemin légitime (§ 4).

## 9. Risques et arbitrages

| # | risque | traitement |
|---|---|---|
| R1 | **La borne 403 des push légitimes et la boucle s'arrête sans diagnostic.** Le risque nommé par mika#2716, et le seul qui puisse arrêter la boucle. | Quatre couches indépendantes. (1) Les deux pièges capables de le produire sont **mesurés** et épinglés chacun par un cas positif en CI : la sonde de 4 octets (cas 2) et le corps chunked (cas 3). (2) Le cas 1 est un positif : un garde qui refuse tout rougit **avant** le merge. (3) La sonde `push_bound_nominal` rougit après `make deploy`, sur une ligne. (4) `MIKA_PILOT_EGRESS_PUSH_BOUND=0` désarme sans redéploiement. |
| R2 | **Le refspec n'est pas là où la mesure l'a vu** — une version de git, une capability, un `push --atomic` changent la forme du corps. | La mesure porte sur le format que le protocole **impose** (`<old> <new> <ref>\0caps`, flush-pkt), pas sur un détail d'implémentation. Un corps qui ne se décode pas rend `None` et refuse avec un motif qui le nomme (`push-body-unreadable`) plutôt que d'autoriser par défaut — et le canari le ferait voir en vol, sur une ligne, pas par un silence. |
| R3 | **Le corps est streamé au-delà de 10 Mo** et le garde ne peut pas juger. | Arbitrage posé explicitement au § 3.3 : **refuser**, parce qu'autoriser donne un contournement trivial (grossir le corps) et qu'un pack de pilote est de l'ordre du kilo-octet. Rendu survivable par un interrupteur **dédié**, distinct de celui du vol 2. Cas 9. |
| R4 | **P4 reste ouverte** — un force-push sur la ref du spawn détruit un travail distant (mika#1318). | Assumé, **mesuré** (§ 2.4) et **écrit** dans les deux surfaces d'opérateur (commit 4). Ce n'est pas une dette tue : le protocole ne porte pas l'information, et la distinguer exigerait de décoder le packfile en vol sur le chemin de la boucle. Les deux gardes faibles existantes sont nommées avec leur faiblesse. |
| R5 | **L'allowlist du dépôt est statique**, donc un pilote `mika` peut pousser sur `mika-cloud`. | Borne volontairement faible à ce vol : elle ferme P1 (l'exfiltration hors organisation, la sous-classe irréparable) sans toucher au lanceur. Le resserrement au périmètre du spawn est le vol 2, nommé par `Tracked in:` au § 5 — pas laissé à la mémoire. |
| R6 | **Le verdict vit dans `request` et l'injection dans `requestheaders`** : deux hooks, donc une incohérence possible à la prochaine édition. | Le commentaire du hook nomme la mesure du § 2.3 (b) — le corps n'existe pas à `requestheaders`. Le cas 1 asserte que l'`Authorization` sortant d'un push **autorisé** est bien celui de l'hôte : si un jour un refus laissait passer la requête, ou une injection sautait, ce cas rougit. |

## 10. Hors périmètre, délibérément

- **La moitié `api.github.com`** — livrée par mika#2724, et non touchée : la
  coupure vit dans `requestheaders`, ce vol n'ajoute qu'un hook `request` borné
  au `git-receive-pack` de `github.com`.
- **Le `cargo test` qui n'écrase pas `GH_TOKEN`** — fermé côté code par mika#2713,
  et hors d'atteinte de toute sentinelle (§ 2.1 du plan du vol 1 de mika#2716).
- **L'egress Anthropic** — `ANTHROPIC_API_KEY` garde son placeholder statique.
- **Le choix du canal `GH_TOKEN`, le shim `gh`, et la lisibilité d'un secret par
  tout processus de la sandbox** — tranchés par les deux vols de mika#2716
  (point 4 du ticket). Le § 2.5 **réemploie** le troisième argument contre
  `http.extraHeader` ; il ne le relitige pas.
- **Le `fetch` (`git-upload-pack`) et `info/refs`** — la borne porte sur le seul
  verbe qui écrit. Un fetch depuis un dépôt tiers lit du public ; il ne détruit
  rien et n'exfiltre rien.
- **Le resserrement au périmètre déclaré par le spawn** — vol 2, § 5,
  `Tracked in:`.
- **P4, le force-push sur la ref du spawn** — hors d'atteinte côté proxy (§ 2.4),
  documenté plutôt que tracké, parce qu'un ticket de suivi promettrait une
  fermeture que la mesure dit impossible par ce canal.

## 11. Référence

- Ticket : senara-solutions/mika#2736 — parent : senara-solutions/mika#2716
  (vol 1 : #2723, vol 2 : #2724)
- **Incident fondateur de la classe `push`** : senara-solutions/mika#1318
  (force-push destructeur depuis un pilote de groom, 2026-05-27) — et
  senara-solutions/mika#736, la seconde occurrence du même jour
- Plan du vol 2 (le § 2.4 ouvre ce ticket) :
  `docs/plans/2026-10-16-002-feat-2724-addon-egress-exige-sentinelle-plan.md`
- Bornes du gitdir, et pourquoi elles ne bornent pas le push :
  senara-solutions/mika#2141, `docs/plans/2026-09-02-001-fix-2141-sandbox-gitdir-bind-plan.md`
- Note de solution (à étendre, commit 4) :
  `docs/solutions/dev-loop/une-sentinelle-voyage-dans-le-canal-que-lincident-ecrase-2026-10-15.md`
- Surface opérateur (à étendre, commit 4) : `docs/operator/environment-variables.md`
  § *La sandbox de pilote présente un justificatif d'egress (mika#2716)*
- Sites touchés : `scripts/mika-pilot-github-auth-addon.py`,
  `scripts/canary-pilot-containment` ; le seuil de streaming vit à
  `skills/bundled/_shared/dispatch-lib.sh:1228`
- Garde faible de P4 : `skills/bundled/dev-groom/system_prompt.md:17-23`
- Doctrines invoquées : mika#1949 (nommer, pas divulguer), mika#2205 (deux états
  qui rendent les mêmes octets), mika#2572 (pas de quote simple dans le payload du
  canari), mika#2578 (une garde que le pipeline ne lance pas ne refuse rien),
  mika#2701 (le suivi est ouvert avant la PR)

---
issue: 2317
type: fix
module: mika-pilot-egress-proxy
tags: [substrate, egress-proxy, keep-alive, http-framing, defense-in-depth]
problem_type: defense-in-depth
---

# fix(2317): le relais ferme à la fin du CORPS, pas à l'EOF amont

## Problème

`_relay_response_with_status_tap` boucle sur `up_reader.read()` jusqu'à l'EOF
amont. La durée d'un tour est donc bornée par *la fermeture de la connexion
amont*, pas par *la fin de la réponse*. mika#2313 a rendu ces deux instants
quasi simultanés en cessant de transmettre le `Connection: keep-alive` du
client, si bien que l'amont reçoit uniquement notre `Connection: close` et
ferme juste après sa réponse.

**Cette correction est correcte et tient — elle repose sur une politique
amont, pas sur une propriété du relais.** Un amont qui ignorerait `close` un
jour reproduirait exactement mika#2313 : le relais attendrait l'idle-timeout
(~360 s mesurées), la seconde requête déjà émise par le CLI resterait non lue,
et le TTL de cache de prompt (5 min) serait franchi à chaque tour. Le présent
travail retire cette dépendance : la fin du corps est lisible sur le flux,
donc le relais peut la constater lui-même.

Régime attendu, dit d'emblée : **ce chemin est un filet, et son régime nominal
est indistinguable du comportement actuel.** Avec `Connection: close` honoré,
la détection de fin de corps et l'EOF tombent dans le même tour de boucle. Le
travail ne se juge pas à un gain mesurable en production ; il se juge à ce
qu'il rend impossible.

## Ce que la lecture du code déplace dans l'énoncé

**(R1) Le terminateur chunked n'est pas une sous-chaîne, et le proposer comme
tel est le seul vrai risque de ce ticket.** L'énoncé dit « terminateur chunked
`0\r\n\r\n` ». Un corps `Transfer-Encoding: chunked` est une suite
`<taille-hex>\r\n<données>\r\n` close par `0\r\n<trailers?>\r\n` : la séquence
littérale `0\r\n\r\n` peut apparaître **à l'intérieur des données d'un chunk**,
qui sont ici du SSE JSON arbitraire. Un `in chunk` déclarerait alors la fin du
corps au milieu de la réponse, et le relais couperait un flux LLM en cours.

Ce n'est pas théorique : **le fichier commet déjà cette erreur**, ligne 659,
sur le corps de *requête* (`if b"0\r\n\r\n" in chunk: break`). Le plan refuse
de la reproduire côté réponse — le chemin réponse obtient un décodeur de
framing à états. Le défaut côté requête est réel, hors périmètre, et part en
ticket de suivi (§ Hors périmètre).

**(R2) Le ticket demande « fermer le client + l'amont » ; les deux fermetures
existent déjà et le retour suffit à les atteindre.** `handle_host_client` a un
`finally: writer.close()` (ligne ~811) et `handle_anthropic_reverse_proxy` un
`finally: up_writer.close()` (ligne ~679). Le livrable n'est donc pas « fermer »
mais « **retourner sur preuve de fin de corps** ». La modification du relais
tient en quelques lignes ; toute la difficulté — et tout le risque — est dans
la preuve. C'est là que va l'effort, et c'est là que vont les tests.

**(R3) `scripts/test-pilot-egress-keepalive.py` n'est exécuté nulle part.**
Aucune cible `Makefile`, aucun job CI (`grep -rn test-pilot-egress-keepalive`
ne rend que le fichier lui-même et deux commentaires). L'AC1 de mika#2313 est
donc, aujourd'hui, non appliquée. Trouvaille de chemin, sans rapport causal
avec ce ticket ; le brancher coûte une ligne dans la cible make que ce plan
touche déjà, et c'est le moment où ça ne coûte rien.

**(R4) Homonyme à ne pas confondre.** `scripts/verify-egress-no-log.sh` impose
une discipline de non-journalisation à `crates/mika-gateway/src/egress_search/`
(recherche web sans rétention, mika#1810), **pas** à
`scripts/mika-pilot-egress-proxy`, qui journalise déjà `ALLOW` / `RATE_LIMITED`
par requête. Aucune contrainte no-log ne pèse sur ce travail. Noté pour qu'un
relecteur n'ait pas à refaire l'enquête.

## Conception

### D1 — Un détecteur passif, jamais un filtre

`_BodyFramingTap` suit le motif déjà en place dans `_ResponseHeadTap` :
**write-then-tap**. Il est nourri d'une *copie* d'octets **déjà écrits et
drainés** vers le client. Il ne retient rien, ne réordonne rien, ne modifie
rien. Sa seule sortie est un booléen `done`. Conséquence structurelle : **il ne
peut pas tronquer une réponse par construction**, il ne peut que déclarer une
fin trop tôt — ce que D3 borne. `test_relays_multi_chunk_sse_byte_identically`
doit rester vert sans être modifié : c'est le témoin de cette propriété.

### D2 — Armement : ce que la tête permet de conclure, et rien d'autre

Le tap s'arme une fois, quand `_ResponseHeadTap.complete` bascule (donc après
que les préambules 1xx ont été écartés — le head tap le fait déjà). Il lit la
tête finale et choisit un mode :

| Condition sur la tête | Mode | Fin du corps |
|---|---|---|
| `head` en overflow | `UNDETERMINED` | EOF amont |
| statut `204`/`304`, ou méthode `HEAD` | `NO_BODY` | immédiate |
| `Transfer-Encoding` **et** `Content-Length` présents | `UNDETERMINED` | EOF amont |
| `Transfer-Encoding` dont le dernier codage est `chunked` | `CHUNKED` | terminateur décodé |
| `Transfer-Encoding` présent, dernier codage ≠ `chunked` | `UNDETERMINED` | EOF amont |
| un seul `Content-Length`, entier ≥ 0 | `LENGTH(n)` | n octets de corps |
| plusieurs `Content-Length` de valeurs différentes | `UNDETERMINED` | EOF amont |
| ni l'un ni l'autre | `UNDETERMINED` | EOF amont (c'est la règle RFC) |

Deux lignes méritent leur justification. **`Content-Length` + `Transfer-Encoding`
ensemble** : la RFC 9112 §6.3 prescrit d'ignorer `Content-Length` et de suivre
chunked, mais cette combinaison est aussi la signature classique du
request-smuggling. On ne tranche pas une réponse contradictoire — on n'en
conclut rien, ce qui est gratuit ici puisque « rien » est le comportement
actuel. **`HEAD`** : la réponse porte un `Content-Length` sans corps ; sans ce
cas on attendrait n octets qui ne viendraient jamais, donc l'EOF — fail-safe
correct même non traité, mais `method` est déjà un paramètre du relais et le
traiter coûte une ligne.

### D3 — Fail-safe asymétrique, et l'asymétrie est écrasante

**Une fin de corps qu'on ne peut pas prouver n'est jamais une fin de corps.**
Tout doute retombe sur `UNDETERMINED`, c'est-à-dire sur la boucle jusqu'à
l'EOF — le comportement d'aujourd'hui.

- **Faux négatif** (on ne détecte pas une fin réelle) : on lit jusqu'à l'EOF,
  exactement comme aujourd'hui. Coût **nul** tant que l'amont honore `close`.
- **Faux positif** (on déclare une fin qui n'en est pas une) : on ferme sur une
  réponse LLM en cours. Le pilote reçoit un flux SSE tronqué — un tour perdu,
  et en aval une réponse partielle qui ne se présente pas comme partielle.

Le rapport entre les deux coûts décide toute la conception. En particulier, le
décodeur chunked est **strict** : taille non hexadécimale, `CRLF` attendu et
absent, ligne de taille ou trailers dépassant `RESPONSE_HEAD_CAP` → passage
définitif en `UNDETERMINED` pour cette connexion. Il ne devine jamais, il
désarme.

### D4 — Offset de fin de tête : la seule modification de l'existant

Le chunk qui complète la tête porte en général les premiers octets du corps. Le
tap de framing doit donc savoir où commence le corps dans le flux.
`_ResponseHeadTap` gagne un attribut `head_end_offset: int | None` — le nombre
d'octets du flux consommés jusqu'au terminateur de la tête **finale** inclus,
préambules 1xx compris (d'où le fait que ce soit le head tap qui le calcule : il
est le seul à savoir ce qu'il a jeté). Addition de comptage pure, sans
changement de comportement, testable isolément.

### D5 — Observabilité : ce qui est décidable, et ce qui ne l'est pas

Doctrine maison (mika#2156, mika#2249) : un filet qui n'annonce pas qu'il a
servi est indistinguable d'un no-op. Mais il faut être exact sur ce qu'on peut
constater.

**Ce qui n'est pas décidable gratuitement :** « l'amont garde la connexion »
versus « l'EOF n'est pas encore remonté ». Après notre dernière lecture, un
`at_eof()` faux ne prouve rien — c'est une course. Toute distinction fiable
coûte une attente, ce qui annule précisément le bénéfice du filet. Émettre une
ligne « le filet a servi » à chaque fermeture-sur-détection serait donc du bruit
par requête *et* un verdict faux. **On ne construit pas ce signal-là.**

**Ce qui est décidable :** la classe de terminaison de chaque relais —
`body-end` (framing lu, fin prouvée), `eof` (EOF amont atteint d'abord),
`undetermined` (framing illisible, filet inerte pour cette requête). Compteurs
agrégés, émis sur le modèle de `_note_readiness_probe` : silence par défaut,
agrégat sous `MIKA_EGRESS_DEBUG`, **jamais une ligne par requête**.

La classe qui porte l'information est `undetermined` : non nulle, elle nomme
des requêtes pour lesquelles ce travail ne protège rien, et c'est un fait sur
lequel on peut agir. C'est le miroir exact de `pilot_stall_signal_unavailable`
(mika#2277) — un filet doit dire qui il ne voit pas.

### D6 — Le surplus au-delà de la fin du corps

Au moment où `done` bascule, le chunk courant a **déjà** été écrit et drainé
(write-then-tap). S'il portait des octets au-delà de la fin du corps, le client
les a reçus. La fidélité l'emporte : un relais un-requête-par-connexion ne peut
recevoir après le corps qu'une réponse que personne n'a demandée, et avec
`Connection: close` il n'en reçoit aucune. Limite assumée, écrite ici pour
qu'elle ne soit pas redécouverte comme un défaut.

## Fichiers

- `scripts/mika-pilot-egress-proxy` — `_ResponseHeadTap` (ajout de
  `head_end_offset`), nouvelle classe `_BodyFramingTap`, retour anticipé dans
  `_relay_response_with_status_tap`, compteurs de classe de terminaison.
- `scripts/test-pilot-egress-proxy-status.py` — nouvelles classes de test
  (harnais `_CapturingWriter` / `_reader_of` / `RelayTapTests` déjà en place,
  et déjà exécuté en CI).
- `Makefile` — cible `test-pilot-egress-proxy` : ajouter l'exécution de
  `test-pilot-egress-keepalive.py` (R3).
- `.github/workflows/ci.yml` — job `pilot-egress-status-tap` : idem.

## Contrat de vérification

Tests unitaires, sans socket réel, dans le harnais existant.

- **U1 — fidélité préservée.** `test_relays_multi_chunk_sse_byte_identically`
  et `test_client_receives_each_chunk_before_the_next_is_read` restent verts
  **sans modification**. Un test que ce travail doit réécrire signalerait que
  le tap n'est plus passif.
- **U2 — `Content-Length` : fermeture sans EOF.** Le critère littéral du
  ticket. Un reader qui livre une réponse complète (`Content-Length: n` + n
  octets) puis **ne rend jamais l'EOF** : le relais retourne malgré tout, et
  `writer.payload` est exactement les octets amont. Sans le correctif, ce test
  pend (il est donc borné par `asyncio.wait_for`, et son expiration est
  l'échec).
- **U3 — chunked : fermeture au terminateur décodé.** Même forme, corps
  chunked terminé par `0\r\n\r\n`, amont qui garde la connexion.
- **U4 — chunked adversarial (le cœur, R1).** Un corps chunked dont les
  **données** contiennent littéralement `0\r\n\r\n`, suivi de chunks
  supplémentaires puis du vrai terminateur. Le relais ne doit **pas** couper à
  la sous-chaîne : `writer.payload` porte l'intégralité du corps. Ce test est
  le seul à distinguer un décodeur d'un `in chunk`, et il est préféré à une
  garde par scan de source parce qu'il refuse le comportement plutôt que
  l'orthographe.
- **U5 — chunked malformé : désarmement, pas devinette.** Taille non
  hexadécimale, puis `CRLF` attendu et absent : le relais retombe sur l'EOF, ne
  déclare aucune fin, et relaie tous les octets.
- **U6 — table d'armement (D2).** Un cas par ligne : overflow, `204`, `304`,
  `HEAD`, `CL`+`TE` ensemble, `TE` non-chunked, `CL` multiples divergents,
  aucun des deux. Chacun rend `UNDETERMINED` sauf `NO_BODY` pour 204/304/HEAD.
- **U7 — `Content-Length: 0` et corps vide.** Fin dès la tête, aucun octet de
  corps attendu, pas d'attente d'EOF.
- **U8 — `head_end_offset` avec préambule 1xx (D4).** Une réponse précédée d'un
  `100 Continue` : l'offset compte le préambule jeté, donc le comptage du corps
  reste juste. Sans ce test, D4 est faux d'exactement la longueur du préambule
  et le corps serait déclaré fini trop tôt — un faux positif silencieux.
- **U9 — le verdict reste unique.** Sur toutes les fermetures anticipées, le
  `finally` du tap de statut ne double pas la ligne : exactement un `ALLOW`
  (la tête est nécessairement complète pour que le framing soit connu, donc
  `reported` est vrai). Reprend la forme de
  `test_stalled_stream_reports_before_the_body_ends`.
- **U10 — tête fractionnée sur plusieurs lectures.** Tête coupée au milieu
  d'un nom d'en-tête, corps coupé au milieu d'une ligne de taille chunked :
  l'armement et le décodage sont insensibles au découpage en paquets.
- **U11 — `test-pilot-egress-keepalive.py` s'exécute** en CI et via
  `make test-pilot-egress-proxy` (R3).

Non couvert, et dit plutôt que masqué : **aucun test de bout en bout sur socket
réel.** `ANTHROPIC_UPSTREAM_HOST` n'est pas injectable (même réserve que
mika#1901, § Deferred de son plan) ; U2/U3 exercent le relais avec un amont qui
ne ferme jamais, ce qui *est* la condition du ticket, mais à travers un
`StreamReader` et non une socket.

## Déploiement

Le proxy est un script autonome dans `~/.local/bin`, **non embarqué** comme
`dispatch-lib`. `make install` le copie ; **le relais en cours d'exécution doit
être redémarré par l'opérateur, à un créneau vide** — tant qu'il ne l'est pas,
le correctif est présent sur disque et absent en vigueur. Même contrainte et
même geste que mika#2313.

## Definition of Done

- `_BodyFramingTap` implémenté et passif : le relais retourne sur fin de corps
  prouvée, sans jamais retenir ni modifier un octet.
- Décodeur chunked à états ; aucune recherche de sous-chaîne comme prédicat de
  fin de corps sur le chemin réponse.
- Tout framing non prouvé retombe sur la boucle jusqu'à l'EOF amont.
- U1–U11 verts ; `make test-pilot-egress-proxy` et le job CI exécutent les deux
  suites Python.
- Compteurs de classe de terminaison en place, silencieux par défaut.
- Ticket de suivi ouvert pour le défaut symétrique du corps de requête (R1).

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son critère littéral et du contrat de vérification.

- **AC1** — Un amont qui répond complètement (`Content-Length` atteint) **et
  garde la connexion ouverte** ne produit aucun hang côté client : le relais
  ferme immédiatement après le dernier octet du corps, sans attendre l'EOF
  amont. C'est le critère littéral du ticket ; U2 le pin.
- **AC2** — Même propriété pour un corps `Transfer-Encoding: chunked`, la fin
  étant déterminée par décodage du cadrage et non par recherche de
  sous-chaîne. U3 + U4.
- **AC3** — Fidélité octet-pour-octet inchangée : sur toute réponse relayée,
  les octets reçus par le client sont exactement ceux émis par l'amont. U1,
  et réasserté par chaque test de fermeture anticipée.
- **AC4** — Aucune fermeture anticipée sur un framing non prouvé : head en
  overflow, `Content-Length` absent, `Content-Length` + `Transfer-Encoding`
  ensemble, `Transfer-Encoding` non-chunked, `Content-Length` multiples
  divergents, cadrage chunked malformé → lecture jusqu'à l'EOF amont, comme
  avant ce travail. U5 + U6.
- **AC5** — Une réponse sans corps (`204`, `304`, réponse à `HEAD`) termine dès
  la fin de la tête. U6 + U7.
- **AC6** — Exactement une ligne de verdict par requête, fermeture anticipée
  comprise. U9.
- **AC7** — Les classes de terminaison (`body-end` / `eof` / `undetermined`)
  sont comptées et lisibles en agrégé sous `MIKA_EGRESS_DEBUG` ; par défaut, le
  travail n'ajoute **aucune** ligne de journal par requête.
- **AC8** — `scripts/test-pilot-egress-keepalive.py` est exécuté par
  `make test-pilot-egress-proxy` et par CI. U11.

## Sonde post-déploiement, et sa halte

Après redémarrage du relais par l'opérateur, sur 48 h avec
`MIKA_EGRESS_DEBUG=1` : les tours de pilote gardent leur durée actuelle (le
filet est inerte quand l'amont honore `close` — **aucun gain n'est attendu, et
un gain observé serait une information**, pas une confirmation), et la classe
`undetermined` doit rester marginale.

**Halte :** si `undetermined` est majoritaire, le filet ne protège presque rien
et la cause est en amont du correctif — il faut comprendre pourquoi les
réponses arrivent sans cadrage lisible **avant** d'élargir quoi que ce soit.
Élargir la détection sur une population qu'on ne comprend pas, c'est déplacer
le risque du côté du faux positif, c'est-à-dire du côté des réponses
tronquées. Et si un hang de type mika#2313 réapparaît alors que le relais
redémarré tourne, **ne pas ajuster ce détecteur** : la fermeture ne vient alors
pas de la boucle de relais, et c'est ce chemin-là qu'il faut établir d'abord.

## Rollback

Revenir sur ce commit. Le relais retourne à « lire jusqu'à l'EOF amont », qui
est sûr tant que mika#2313 tient (l'amont honore `Connection: close`). Aucun
état persistant, aucune migration, aucun réglage à défaire — le seul geste est
le redémarrage du relais.

## Hors périmètre, délibérément

- **Le défaut symétrique du corps de requête** (ligne ~659,
  `if b"0\r\n\r\n" in chunk: break` sur un corps chunked montant). Réel, même
  classe que R1, mais il porte sur le sens client→amont, il tronquerait une
  *requête* et non une réponse, et son correctif touche une autre boucle.
  **Ticket de suivi à ouvrir** — le mélanger ici doublerait la surface d'un
  ticket p2 dont le cœur est déjà la stricte correction d'un décodeur.
- **La réécriture de la tête de réponse** (forcer `Connection: close` vers le
  client). Explicitement laissée en suivi par mika#2313 ; le présent travail
  rend le relais indépendant de l'amont, ce qui est le besoin, sans toucher aux
  octets rendus au client.
- **Le support réel du keep-alive amont** (réutiliser la connexion pour une
  seconde requête). Le relais est un-requête-par-connexion par conception ; ce
  ticket borne la durée d'un tour, il ne change pas ce modèle.
- **L'injectabilité de `ANTHROPIC_UPSTREAM_HOST`** pour un test de bout en bout
  sur socket, déjà différée par mika#1901 pour la même raison : elle traverse
  le chemin d'authentification.

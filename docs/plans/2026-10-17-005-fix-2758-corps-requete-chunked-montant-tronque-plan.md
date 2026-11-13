# fix(infra) — le corps de requête `chunked` montant n'est plus borné par une recherche de sous-chaîne (mika#2758)

- **Ticket :** senara-solutions/mika#2758 (`bug`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`)
- **Lignage :** suivi structurel de mika#2317 (§ *Hors périmètre, délibérément*) → PR #2374. Le même défaut, à l'autre bout du relais.
- **Fichiers :** `scripts/mika-pilot-egress-proxy`, `scripts/test-pilot-egress-proxy-status.py`
- **Vérifié le 2026-10-17** sur ce worktree (`8ac93d62`) : le défaut est toujours là, à `scripts/mika-pilot-egress-proxy:1051`.

---

## 1. Constat

`handle_anthropic_reverse_proxy` relaie le corps de **requête** montant
(client → amont) et, quand le client a posé `Transfer-Encoding: chunked`, borne
sa boucle sur une recherche de sous-chaîne — `scripts/mika-pilot-egress-proxy:1043-1052` :

```python
if is_chunked:
    # Stream chunked body verbatim until 0-length chunk terminator.
    while True:
        chunk = await reader.read(BUFFER_SIZE)
        if not chunk:
            break
        up_writer.write(chunk)
        await up_writer.drain()
        if b"0\r\n\r\n" in chunk:      # ← le défaut
            break
```

`0\r\n\r\n` n'est pas un terminateur de cadrage : c'est une suite d'octets qui
*apparaît* au terminateur. Un corps `chunked` est une suite
`<taille-hex>\r\n<données>\r\n` close par `0\r\n<trailers?>\r\n`, et les
`<données>` sont ici du JSON arbitraire — un prompt, un `tool_result`, le
contenu d'un fichier que le pilote vient de lire. La séquence littérale peut
donc vivre **au milieu d'un chunk**.

Le prédicat a deux modes de défaillance, et ils tirent en sens opposés :

| | condition | conséquence |
|---|---|---|
| **faux positif** | `0\r\n\r\n` dans les *données* d'un chunk | la boucle `break` au milieu du corps ⇒ l'amont reçoit une requête **tronquée**, répond 400 ou attend des octets qui ne viennent pas, et le tour échoue sans nommer sa cause |
| **faux négatif** | terminateur **scindé** entre deux `read()` (`…0\r\n` puis `\r\n`) | aucun `break` ⇒ la boucle attend l'EOF client, qui n'arrive pas (le client attend la réponse) ⇒ le tour **pend** |

Le second n'est pas dans le corps du ticket et n'ajoute aucun périmètre : il
disparaît du même geste, parce qu'un décodeur à états est insensible au
découpage des lectures là où un `in chunk` par lecture ne l'est pas. Il est
nommé ici parce qu'un plan qui ne le nomme pas ferait livrer un test de moins.

Deux points vérifiés pour écarter des causes voisines :

- **Aucun octet de corps n'est perdu avant la boucle.** `handle_host_client`
  lit la tête avec `reader.readuntil(b"\r\n\r\n")` (ligne 1092), qui s'arrête à
  la **première** occurrence du séparateur : la troisième part du
  `request.partition(b"\r\n\r\n")` de la ligne 985 est toujours vide, et le
  corps est intégralement encore dans `reader`. Le `_` qui la jette n'est donc
  pas un second défaut.
- **Le chemin `Content-Length` est sain** (lignes 1053-1061) : il décompte des
  octets, il ne cherche rien. Hors périmètre, et c'est correct.

## 2. Cause racine

Une propriété de cadrage a été approximée par une propriété de contenu. La
formulation qui décide la conception est déjà écrite dans ce fichier, en tête du
décodeur introduit par mika#2317 (`scripts/mika-pilot-egress-proxy:434-437`) :

> In particular the chunked terminator is NOT a substring. […] Hence a state
> machine, not an `in chunk`.

Le correctif de mika#2317 a appliqué cette phrase au sens **amont → client** et
a laissé le sens **client → amont** intact, délibérément et pour une bonne
raison : son cœur était la stricte correction d'un décodeur, et mélanger les
deux sens aurait doublé la surface d'un p2. La dette ainsi ouverte est ce
ticket.

## 3. Décision de conception — réutiliser le décodeur, ne pas en écrire un second

`_ChunkedDecoder` (`scripts/mika-pilot-egress-proxy:553-633`) et son
`_parse_chunk_size` (636-645) sont **agnostiques du sens** : ils consomment des
octets de corps et disent quand le corps est fini. Rien dans leur code ne
mentionne une réponse. Le correctif est donc une réutilisation, et le livrable
principal n'est pas un algorithme mais un **site d'appel** et sa propriété de
sûreté.

### 3.1 La propriété qui rend la réutilisation sûre : transmettre-puis-décoder

Sur le chemin de la réponse, mika#2317 a établi l'asymétrie *write-then-tap* :
le décodeur ne voit qu'une **copie** d'octets déjà écrits et drainés, donc il ne
peut pas tronquer par construction — il ne peut que déclarer une fin trop tôt.

La boucle montante garde exactement la même forme, et c'est ce qui autorise la
réutilisation sans y réfléchir deux fois :

1. `read()` depuis le client ;
2. `write()` + `drain()` vers l'amont — **verbatim, inconditionnel, avant toute
   décision** ;
3. seulement ensuite, `decoder.feed(chunk)`.

Conséquence directe : **un décodeur en échec ne peut pas tronquer la requête**,
puisque les octets qui l'ont mis en échec sont déjà partis vers l'amont. Il ne
peut que rater une fin. Le pire cas d'un échec est donc le comportement
d'aujourd'hui, pas un comportement neuf.

### 3.2 Le repli sur échec : désarmer et lire jusqu'à l'EOF client

Taille non hexadécimale, `CRLF` attendu et absent, ligne au-delà du cap :
`_ChunkedDecoder` lève `failed`. Le repli est alors **exactement le
comportement actuel pour un corps dont la sous-chaîne n'apparaît jamais** —
continuer à transmettre verbatim jusqu'à l'EOF client — et le décodeur est
désarmé pour le reste de la connexion plutôt que re-sollicité à chaque lecture.

Ce repli est un **état d'attente**, et le plan le nomme comme tel plutôt que de
le présenter comme gratuit : le client n'envoie pas d'EOF avant d'avoir sa
réponse, donc un cadrage montant illisible pend le tour. Mais c'est la propriété
de la ligne 1047 d'aujourd'hui, inchangée — le correctif ne l'introduit pas et
ne la retire pas. Ce qu'il retire, c'est l'autre moitié : la troncature. Le
ticket tranche explicitement dans ce sens (« repli sur le comportement
actuel »), et ne pas deviner est la règle qui gouverne les deux bouts du relais :
**un cadrage qu'on ne peut pas prouver n'est jamais une fin de corps.**

L'échec est journalisé **une fois par connexion** (le désarmement garantit
l'unicité), sans passer par la porte `_EGRESS_DEBUG` : cette porte ne couvre que
les classes bénignes et à haute fréquence (`scripts/mika-pilot-egress-proxy:705-708`),
et un cadrage de requête illisible émis par la pile HTTP de Claude Code n'est ni
l'un ni l'autre. Sans cette ligne, l'anomalie serait indistinguable d'une lenteur
amont.

### 3.3 Une fonction nommée, parce que le test ne peut pas atteindre la boucle

`handle_anthropic_reverse_proxy` lit `~/.claude/.credentials.json` et ouvre une
TLS vers `api.anthropic.com` avant d'atteindre la boucle. Le critère du ticket
exige un test **sans socket réelle** : la boucle doit donc vivre dans une
coroutine de niveau module, `_forward_chunked_request_body(reader, up_writer)`,
qui ne connaît qu'un `StreamReader` et un objet `write`/`drain`. Le harnais a
déjà les deux doublures qu'il faut — `_CapturingWriter`,
`_held_open_reader_of` (`scripts/test-pilot-egress-proxy-status.py:127-170`) — et
elles mordent ici pour la même raison que sur le chemin réponse : un lecteur qui
n'atteint jamais l'EOF transforme « la boucle ne s'arrête pas » en échec borné
par `asyncio.wait_for` au lieu d'une suite bloquée.

L'extraction n'est donc pas de la cosmétique : c'est la condition de
testabilité du critère. Elle retourne un verdict — `"framed"` quand la fin a été
**prouvée**, `"eof"` quand la boucle s'est arrêtée sur l'EOF client — pour que
les tests distinguent les deux arrêts, qui sont aujourd'hui indistinguables de
l'extérieur.

### 3.4 Options écartées

- **Chercher le terminateur sur un tampon glissant plutôt que par lecture.**
  Corrigerait le faux négatif (terminateur scindé) et **laisserait entier** le
  faux positif, qui est le défaut du ticket. Et AC1 interdit toute recherche de
  sous-chaîne comme prédicat de fin.
- **Décoder d'abord, puis ne transmettre que les octets du corps.** Offrirait
  en plus de ne jamais sur-transmettre au-delà de la fin du corps, mais au prix
  de la propriété qui fait toute la sûreté du § 3.1 : un décodeur en échec
  pourrait alors tronquer. Et le gain est nul ici — un client non *pipelined*
  n'envoie rien après le terminateur, et le code d'aujourd'hui sur-transmettrait
  de la même façon.
- **Écrire un second décodeur pour le sens montant.** Deux implémentations de la
  même grammaire divergent ; le ticket l'écarte nommément.
- **Replier sur la recherche de sous-chaîne en cas d'échec du décodeur.**
  Ramènerait la troncature par la porte du repli, exactement dans le cas où l'on
  a déjà admis ne rien pouvoir prouver.

## 4. Delta de contrat — ce qui change pour un lecteur de mika#2317

| | avant | après |
|---|---|---|
| prédicat de fin, sens montant | `b"0\r\n\r\n" in chunk` | `_ChunkedDecoder.done` |
| corps de requête contenant la séquence dans ses données | **tronqué** | transmis intégralement |
| terminateur scindé entre deux lectures | pend jusqu'à l'EOF client | fin prouvée, la boucle rend |
| cadrage montant illisible | (n'existait pas comme cas) | désarmé + journalisé, transmission verbatim jusqu'à l'EOF — comportement d'aujourd'hui |
| `_ChunkedDecoder` | un seul appelant (réponse) | deux appelants, sens-agnostique **dit** comme tel dans la docstring |

## 5. Livrables et ordre de commit

Un seul commit, l'ordre interne est celui de la lecture.

**R1 — `scripts/mika-pilot-egress-proxy` : la coroutine de transfert.**
Au-dessus de `handle_anthropic_reverse_proxy`, deux constantes de verdict dans
le style des `_FRAMING_*` existantes, puis :

```python
_UPLINK_END_FRAMED = "framed"   # la fin du corps a été PROUVÉE
_UPLINK_END_EOF = "eof"         # arrêt sur EOF client, rien n'a été prouvé


async def _forward_chunked_request_body(reader, up_writer) -> str:
    """Relay a `Transfer-Encoding: chunked` REQUEST body, client → upstream.

    Mirror of `_relay_response_with_status_tap`'s framing discipline, on the
    other direction (mika#2758, follow-up to mika#2317). Two properties:

    **Forward-then-decode.** Every read is written and drained upstream BEFORE
    the decoder is consulted, so a decoder that fails cannot truncate the
    request — it can only fail to spot an end. Same asymmetry as the response
    tap, same reason.

    **Unprovable is not an end.** On `failed` the decoder is disarmed for the
    rest of the connection and the loop forwards verbatim to client EOF: the
    behaviour this relay had before mika#2758, not a new one. A substring
    search is never the fallback — that is the defect being removed.
    """
    decoder = _ChunkedDecoder()
    while True:
        chunk = await reader.read(BUFFER_SIZE)
        if not chunk:
            return _UPLINK_END_EOF
        up_writer.write(chunk)
        await up_writer.drain()
        if decoder is None:
            continue
        decoder.feed(chunk)
        if decoder.failed:
            decoder = None
            _log(
                "[anthropic-proxy] WARN unreadable uplink chunked framing — "
                "relaying verbatim to client EOF (mika#2758)"
            )
        elif decoder.done:
            return _UPLINK_END_FRAMED
```

**R2 — le site d'appel.** Les lignes 1043-1052 deviennent :

```python
if is_chunked:
    # Framing-decoded, not substring-searched (mika#2758).
    await _forward_chunked_request_body(reader, up_writer)
```

Aucune autre ligne de `handle_anthropic_reverse_proxy` ne change ; le verdict
rendu n'y est pas consommé (le relais de réponse suit, inconditionnellement,
comme aujourd'hui) — il existe pour les tests et pour un appelant futur.

**R3 — les deux ancres de documentation du même fichier.**
- Le bloc d'orientation `scripts/mika-pilot-egress-proxy:420-437` parle de la
  réponse au singulier. Une phrase y dit que le décodeur sert désormais les
  deux sens et renvoie à `_forward_chunked_request_body`.
- La docstring de `_ChunkedDecoder` (553-561) dit « disarms the detection for
  the rest of the connection » — vrai des deux côtés — mais ne dit pas qu'elle
  est sens-agnostique. Une phrase l'énonce, pour que la prochaine lecture ne
  prenne pas la réutilisation pour un détournement.

**R4 — `scripts/test-pilot-egress-proxy-status.py` : une classe
`UplinkChunkedBodyTests(unittest.IsolatedAsyncioTestCase)`**, précédée de son
bloc d'orientation, bornée par un `_UPLINK_TIMEOUT = 2.0` et un helper
`_forward(reader)` qui `wait_for` la coroutine et rend `(verdict, writer)` :

1. `test_terminator_inside_request_chunk_data_is_not_an_end` — **le test du
   ticket**, miroir strict de
   `test_chunked_terminator_inside_chunk_data_is_not_an_end` (ligne 508) : corps
   dont les données contiennent littéralement `0\r\n\r\n`, suivi d'un chunk
   `afterward` et du vrai terminateur, sur `_held_open_reader_of`. Assertions :
   verdict `framed`, et `writer.payload == b"".join(chunks)` avec le message
   « the relay cut the request body at a substring inside chunk data ». Sur le
   code d'avant, la payload est tronquée au premier chunk.
2. `test_request_body_stops_at_the_decoded_terminator` — corps nominal sur un
   lecteur tenu ouvert ; verdict `framed`, payload intégrale. C'est la
   non-régression du cas normal : le correctif ne doit pas transformer chaque
   requête en attente d'EOF.
3. `test_terminator_split_across_two_reads_is_still_an_end` — `…0\r\n` puis
   `\r\n`, lecteur tenu ouvert. Verdict `framed`. Sur le code d'avant, ce test
   expire : c'est le faux négatif du § 1.
4. `test_unreadable_framing_forwards_everything_and_waits_for_eof` — ligne de
   taille non hexadécimale, lecteur **à l'EOF** (`_reader_of`). Verdict `eof`,
   payload intégrale, et la ligne `WARN unreadable uplink chunked framing`
   présente **une seule fois** dans `stderr` capturé par `_strip_ts` — c'est
   l'assertion qui fence le désarmement (sans lui, une ligne par lecture).
5. `test_extensions_and_trailers_in_a_request_body` — `5;name=value`, puis
   `0\r\n`, un trailer, la ligne vide ; verdict `framed`.
6. `test_client_eof_before_any_terminator_returns_eof` — corps partiel puis EOF
   client ; verdict `eof`, payload intégrale. Anti-vacuité : la boucle rend sur
   EOF et ne lève pas.
7. `test_no_substring_predicate_remains_in_the_uplink_loop` — AC1 au niveau de
   la source : `inspect.getsource(proxy._forward_chunked_request_body)` ne
   contient ni `0\\r\\n\\r\\n` ni ` in chunk`. Voir § 9.

**R5 — le commentaire périmé.**
`scripts/test-pilot-egress-proxy-status.py:511-512` dit aujourd'hui :

> An `in chunk` predicate — *the one the request path still uses at the other
> end of this file* — would cut a live LLM stream right here.

La subordonnée devient fausse avec R2. Elle est remplacée par un renvoi au test
miroir (`UplinkChunkedBodyTests`, mika#2758). Un commentaire qui décrit un
défaut corrigé est la façon dont un lecteur suivant rouvre un ticket déjà clos.

**Aucun câblage de CI ni de Makefile.** La cible `test-pilot-egress-proxy`
(`Makefile:336-338`) et le job `pilot-egress-status-tap`
(`.github/workflows/ci.yml:623-634`) exécutent déjà ce fichier de test ;
`SuiteWiringTests` (990-1016) fence ce câblage. Vérifié, rien à ajouter.

## 6. Hors périmètre de ce vol, nommé plutôt que tu

- **Le tunnel `CONNECT` et le chemin `mitmdump`** (`handle_host_client`,
  lignes 1140-1166) : `pipe_bytes` y relaie des octets opaques sans jamais
  interpréter de cadrage. Il n'y a pas de prédicat à corriger.
- **Le chemin `Content-Length` montant** (1053-1061) : exact par décompte, pas
  par recherche. Intact.
- **La sur-transmission au-delà de la fin du corps** pour un client
  *pipelined* : écartée au § 3.4, propriété inchangée par rapport à aujourd'hui,
  et aucun client du relais ne *pipeline*.
- **L'attente d'EOF comme repli** (§ 3.2) : c'est le comportement actuel et le
  ticket le prescrit. Si l'on voulait un jour la borner par un délai, ce serait
  un ticket à soi, avec sa mesure — un délai mal choisi coupe une requête
  légitime, exactement la classe de défaut que celui-ci ferme.

## 7. Definition of Done

- `_forward_chunked_request_body` existe, est appelée par
  `handle_anthropic_reverse_proxy`, et le littéral `b"0\r\n\r\n"` a disparu de
  tout prédicat de fin de corps du fichier.
- `python3 -B scripts/test-pilot-egress-proxy-status.py` passe, avec les sept
  tests de R4 ajoutés et aucun test existant modifié hors du commentaire R5.
- `python3 -B scripts/test-pilot-egress-keepalive.py` passe (non-régression du
  voisin que le même job exécute).
- `make test-pilot-egress-proxy` passe.
- Les quatre tests 1, 3, 4 et 7 de R4 **échouent** sur `8ac93d62` — vérifié en
  les exécutant contre le fichier d'avant (anti-vacuité : un test qui passe
  aussi sans le correctif ne fence rien).
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de PR cite mika#2758 et mika#2317, et nomme le faux négatif du § 1
  comme couvert sans élargissement de périmètre.

## 8. Acceptance criteria

Transcrits du corps du ticket, verbatim :

- La boucle chunked montante ne contient plus aucune recherche de sous-chaîne
  comme prédicat de fin de corps.
- Un corps de requête `chunked` dont les **données** contiennent littéralement
  `0\r\n\r\n` est transmis **intégralement** à l'amont (le test miroir de
  `test_chunked_terminator_inside_chunk_data_is_not_an_end`).
- Un cadrage montant illisible ne tronque rien : repli sur le comportement
  actuel.
- Test unitaire dans `scripts/test-pilot-egress-proxy-status.py`, sans socket
  réelle, dans le harnais déjà en place.

Correspondance : AC1 → R2 + R4.7 ; AC2 → R4.1 ; AC3 → §3.2 + R4.4 ; AC4 → R4
entier (doublures `_CapturingWriter` / `_held_open_reader_of` déjà en place,
aucune socket ouverte).

## 9. Fire-Disposition

Un détecteur est livré : **R4.7**,
`test_no_substring_predicate_remains_in_the_uplink_loop`, dont le chemin de
succès est « aucune violation trouvée » sur la source de
`_forward_chunked_request_body`. Les six autres tests de R4 sont des tests de
comportement sur une entrée construite, pas des scans ; R4.7 est le seul à
inspecter du code.

**Option retenue : (a) exception nommée en allowlist — avec un ensemble
d'exceptions vide, et une raison.** Le détecteur atterrit **armé et vert** :

- **Périmètre.** `inspect.getsource(proxy._forward_chunked_request_body)` — la
  seule fonction du dépôt qui borne un corps `chunked` montant. Pas un scan du
  fichier : un scan du fichier verrait les littéraux légitimes des tests et du
  bloc d'orientation, et exigerait aussitôt l'allowlist que ce cadrage évite.
- **Allowlist vide, et c'est une propriété, pas un oubli.** Le même commit
  supprime la seule violation existante (R2). Il n'y a donc aucune donnée à
  excepter, et l'option (b) — livrer désarmé — ne s'applique pas : rien ne
  resterait à armer.
- **Auto-nettoyant.** Le test assertionne d'abord que l'attribut existe
  (`self.assertTrue(hasattr(proxy, "_forward_chunked_request_body"))`,
  message : « la coroutine de mika#2758 a disparu ou a été renommée — le
  détecteur AC1 ne surveille plus rien »). Un renommage ou une suppression
  rougit au lieu de scanner le vide et de passer. C'est la moitié qui manque
  d'ordinaire à ce genre de garde : sans elle, un détecteur dont la cible a
  disparu est indistinguable d'un détecteur satisfait.
- **Grep-visible.** Le nom du test et son message portent `mika#2758` et
  `AC1`.

## 10. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| R1 `_forward_chunked_request_body` + constantes (`scripts/mika-pilot-egress-proxy`) | 55 |
| R2 site d'appel dans `handle_anthropic_reverse_proxy` (même fichier) | 10 |
| R3 retouches du bloc d'orientation + docstring `_ChunkedDecoder` (même fichier) | 15 |
| R4 tests `UplinkChunkedBodyTests` (`scripts/test-pilot-egress-proxy-status.py`) | 115 |
| R5 commentaire périmé ligne 511 + ancre croisée (même fichier) | 8 |

Total estimé : 203 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 203 < 429 avec une marge de plus du
double. Le volume est petit pour une raison structurelle, pas par optimisme :
l'algorithme existe déjà, testé, dans le même fichier (`_ChunkedDecoder`,
80 lignes qui ne sont pas réécrites) ; le livrable de code est un site d'appel
de dix lignes et une coroutine qui l'enveloppe. Les deux tiers de l'estimation
sont des tests, qui sont des copies de forme de `BodyEndClosureTests` sur des
doublures déjà écrites. Aucun fichier Rust, aucune migration, aucun câblage de
CI (§ 5, dernier paragraphe). Deux fichiers touchés, les deux dans
`scripts/`.

## 11. Risques

- **Le cas normal devient une attente d'EOF.** Si le décodeur ne prouvait pas la
  fin d'un corps nominal, chaque requête du pilote pendrait — la régression la
  plus coûteuse possible. R4.2 est précisément le test qui la rend impossible
  à merger, et `_held_open_reader_of` est ce qui la transforme en échec borné
  plutôt qu'en suite bloquée. Probabilité faible : le décodeur est déjà exercé
  sur le même cadrage par `BodyEndClosureTests` et `ChunkedDecoderTests`.
- **Le cap de ligne de `_ChunkedDecoder`.** Il vaut `_CHUNK_LINE_CAP` =
  `RESPONSE_HEAD_CAP` = 64 KiB, dimensionné pour une *réponse*. Il borne une
  ligne de taille ou de trailer, jamais les données d'un chunk — donc un corps
  de requête de plusieurs Mio ne l'approche pas. Réutilisé tel quel ; aucune
  ligne de cadrage légitime n'en est proche, dans un sens comme dans l'autre.
- **Non-régression invisible.** Le défaut n'a jamais été observé en production
  (c'est la raison d'être du ticket), donc rien de mesurable ne confirmera le
  correctif en vol. La seule preuve est l'anti-vacuité du § 7 : les quatre tests
  qui doivent échouer sur `8ac93d62`. Sans cette vérification, le vol livre un
  test qui passerait des deux côtés et ne démontre rien.

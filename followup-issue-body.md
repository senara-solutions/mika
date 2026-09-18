Suivi structurel de mika#2317 (§ *Hors périmètre, délibérément*), ouvert par le
travail qui a corrigé le **même défaut à l'autre bout du relais**.

## Le défaut

`scripts/mika-pilot-egress-proxy`, dans `handle_anthropic_reverse_proxy`, sur le
corps de **requête** montant (client → amont), quand le client a posé
`Transfer-Encoding: chunked` :

```python
while True:
    chunk = await reader.read(BUFFER_SIZE)
    if not chunk:
        break
    up_writer.write(chunk)
    await up_writer.drain()
    if b"0\r\n\r\n" in chunk:      # ← ici
        break
```

`0\r\n\r\n` n'est **pas** un terminateur : c'est une sous-chaîne. Un corps
`chunked` est une suite `<taille-hex>\r\n<données>\r\n` close par
`0\r\n<trailers?>\r\n`, et la séquence littérale peut apparaître **à l'intérieur
des données d'un chunk** — qui sont ici du JSON arbitraire (un prompt, un
tool_result, un contenu de fichier lu par le pilote). Une requête qui la contient
est donc **tronquée en vol** : le relais cesse de transmettre au milieu du corps,
l'amont reçoit une requête incomplète, et le tour échoue d'une façon qui ne
nomme pas sa cause.

## Pourquoi ce n'est pas dans mika#2317

mika#2317 porte sur le sens **amont → client** (la réponse) et son livrable est
un décodeur de cadrage à états pour ce chemin-là. Le défaut ci-dessus porte sur
le sens **client → amont**, il tronquerait une *requête* et non une *réponse*, et
son correctif touche une autre boucle. Les mélanger aurait doublé la surface d'un
ticket p2 dont le cœur était déjà la stricte correction d'un décodeur.

## Ce que le correctif a déjà produit et qui sert ici

`mika#2317` a introduit, dans le même fichier, un décodeur de cadrage `chunked`
strict et testé : `_ChunkedDecoder` (+ `_parse_chunk_size`). Il est **agnostique
du sens** — il consomme des octets de corps et dit quand le corps est fini. Le
correctif attendu ici est donc, pour l'essentiel, de le réutiliser sur la boucle
montante au lieu de la recherche de sous-chaîne, et non d'en écrire un second.

Rappel de la propriété qui décide la conception, identique des deux côtés :
**un cadrage qu'on ne peut pas prouver n'est jamais une fin de corps.** Un
décodeur en échec (taille non hexadécimale, `CRLF` attendu et absent) doit
retomber sur le comportement d'aujourd'hui — ici, lire jusqu'à l'EOF client —
et jamais deviner.

## Critères

- La boucle chunked montante ne contient plus aucune recherche de sous-chaîne
  comme prédicat de fin de corps.
- Un corps de requête `chunked` dont les **données** contiennent littéralement
  `0\r\n\r\n` est transmis **intégralement** à l'amont (le test miroir de
  `test_chunked_terminator_inside_chunk_data_is_not_an_end`).
- Un cadrage montant illisible ne tronque rien : repli sur le comportement
  actuel.
- Test unitaire dans `scripts/test-pilot-egress-proxy-status.py`, sans socket
  réelle, dans le harnais déjà en place.

## Sévérité

p2 substrat — même classe que mika#2317. Le chemin est réel et emprunté à chaque
requête du pilote, mais la probabilité qu'un corps de requête contienne la
séquence exacte est faible, et sa réalisation n'a pas été observée en
production. Ce ticket existe pour que cette faiblesse ne soit pas re-découverte
comme un défaut inconnu le jour où elle se réalise.

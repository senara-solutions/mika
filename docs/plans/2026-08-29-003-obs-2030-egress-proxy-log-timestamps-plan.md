---
issue: senara-solutions/mika#2030
type: obs
status: groomed
branch: obs/2030/egress-pilot-egress-proxy-log-carries-no
date: 2026-08-29
---

# mika#2030 — horodater `pilot-egress-proxy.log`

## Pourquoi maintenant

Ce ticket est étiqueté p2, mais il **garde un p1**. mika#2029 (le décrochage du pilote qui tue les
sessions dev-groom, 15 sessions mesurées) porte dans son propre corps la phrase qui bloque son
diagnostic :

> *« `/var/log/mika/pilot-egress-proxy.log` carries no timestamps, so the failure windows could not be
> correlated to upstream statuses. That gap is filed separately. »*

Le fichier **contient** la réponse — des `401` et de nombreux `Connection lost` — mais rien ne permet
de les rattacher à une fenêtre de dispatch. Tant que ce gap est ouvert, tout plan de correction sur
#2029 serait spéculatif. Ce lot est donc le premier pas d'une chaîne tier-1, pas une tâche d'hygiène.

## État des lieux

`scripts/mika-pilot-egress-proxy` — 818 lignes, Python 3, aucun module `logging`, **aucun helper de
log**. La sortie opérateur est produite par **14 appels `print(..., file=sys.stderr, flush=True)`**
éparpillés, sous **quatre** préfixes :

| préfixe | occurrences |
|---|---|
| `[anthropic-proxy]` | 6 |
| `[egress]` | 5 |
| `[mitm-forward]` | 2 |
| `[egress-shim]` | 1 |

Le fichier de log n'est pas ouvert par le script : `skills/bundled/_shared/dispatch-lib.sh:203`
redirige stderr vers `$log_dir/pilot-egress-proxy.log` (ou `/tmp/…` en repli, `:205`).

Le périmètre du ticket dit « préfixer chaque ligne » — mais il n'y a pas *un* point d'émission à
préfixer, il y en a quatorze. Le correctif est donc un **point de passage unique**, pas un préfixe.

## Correctif

Introduire un helper unique et y router les quatorze sites :

```python
def _log(msg: str) -> None:
    ts = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
    print(f"{ts} {msg}", file=sys.stderr, flush=True)
```

`flush=True` est conservé tel quel : l'ordonnancement ligne à ligne est ce qui rend ce log lisible
pendant un incident, et le perdre coûterait plus que l'horodatage ne rapporte.

### Format retenu

`2026-08-29T03:17:58.081Z [egress] …` — ISO-8601 UTC, millisecondes, suffixe `Z`.

C'est **exactement la forme de la valeur `timestamp` de `server.log`** (`"2026-08-29T03:17:58.081902Z"`,
tronquée à la milliseconde). Le ticket demande de « match the format the rest of `/var/log/mika/` uses
so existing greps compose » : `server.log` est du JSON ligne à ligne, donc un alignement littéral
imposerait de passer ce log en JSON — un changement bien plus large que ce que le ticket demande. Ce
que la phrase vise réellement, c'est que `grep '2026-08-28'` cesse de rendre 0. Un préfixe ISO-8601
le donne, et une recherche par date compose alors entre les deux fichiers.

### Alternative écartée

Horodater au point de redirection dans `dispatch-lib.sh:203` (`| ts` ou équivalent) — une ligne au
lieu de quatorze. Écartée pour trois raisons :

1. elle introduit un tube dans un chemin où le code de sortie compte, et ce dépôt a déjà payé cette
   classe (`PIPESTATUS` masquant un échec) ;
2. elle casse la sémantique de `flush=True` : l'horodatage deviendrait celui de la lecture par le
   tube, pas celui de l'événement ;
3. quiconque lance le script directement — c'est le cas en investigation — n'aurait aucun horodatage.

### Point d'attention

`_log_upstream_outcome` (`:351-381`) compose une ligne en deux morceaux pour le cas 429 :
`print(f"{line} {detail}" if detail else line, …)`. Le passage au helper doit horodater **une fois, à
l'émission**, et non préfixer `line` avant la concaténation — sinon l'horodatage se retrouve au milieu
de la ligne 429, précisément celle que #1901/PR#2019 vient de rendre visible.

### Inventaire vérifié des 14 sites

Énumérés et relus un par un (`grep -n "file=sys.stderr"`), pour ne pas fonder l'AC3 sur une
supposition :

- **`:381` — le seul à composition conditionnelle** : `print(f"{line} {detail}" if detail else line, …)`.
  C'est le piège décrit ci-dessus, et il est unique.
- **`:498` — concaténation implicite sur deux lignes** (`f"…" f"…"`, pas de condition). Pas le même
  danger, mais le passage au helper doit envelopper **l'expression entière** et non la première
  moitié.
- Les douze autres (`:371`, `:376`, `:489`, `:557`, `:654`, `:658`, `:680`, `:691`, `:696`, `:705`,
  `:759`, `:774`) sont des chaînes unitaires, constantes ou f-strings simples.

## Critères d'acceptation

- **AC1** — Chaque ligne écrite sur stderr par le script commence par un horodatage
  `YYYY-MM-DDTHH:MM:SS.sssZ`. Test : capturer stderr sur un démarrage et vérifier que **toutes** les
  lignes non vides correspondent au motif — pas seulement la première.
- **AC2** — Les quatre préfixes existants (`[egress]`, `[anthropic-proxy]`, `[mitm-forward]`,
  `[egress-shim]`) et le texte des messages sont inchangés après l'horodatage. Les greps opérateur
  existants qui cherchent `ERROR`, `RATE_LIMITED`, `UPSTREAM_NO_RESPONSE` continuent de matcher.
- **AC3** — La ligne 429 (`:381`, seul site à composition conditionnelle) reste d'un seul tenant : un seul horodatage, en tête, `RATE_LIMITED` et son
  détail quota intacts sur la même ligne. Test anti-vacuité : un cas 429 **avec** en-têtes de quota
  et un cas **sans**, les deux vérifiés.
- **AC4** — Garde anti-régression : aucun `file=sys.stderr` ne subsiste dans le fichier en dehors du
  helper. Exprimée comme règle de rôle — le helper est le seul émetteur — et non comme une liste de
  lignes, pour qu'elle survive aux ajouts futurs.
- **AC5** — `flush=True` est conservé sur le chemin d'émission ; l'ordre des lignes ne change pas.
- **AC6** — Le script démarre et sert une requête comme avant (aucune régression fonctionnelle) ;
  `python3 -m py_compile scripts/mika-pilot-egress-proxy` passe.

## Historique de grooming

- Passe architecte 1 (`mika-arch`, 2026-08-29) — **READY**. Les quatre points validés : lecture du
  périmètre (préfixe ISO-8601 plutôt que migration JSON), calibre du refactor (helper unique = surface
  minimale pour l'invariant), AC4 par rôle plutôt que par liste de lignes, et unicité du piège 429.
- Vérification indépendante : l'architecte affirmait l'exhaustivité des 14 sites sans que le brief les
  énumère. Contrôlé à la main — la claim tient, et le contrôle a ajouté le cas `:498`.

## Hors périmètre

- Passer ce log en JSON ligne à ligne pour s'aligner sur `server.log` — plus large que le besoin,
  et casserait les greps que l'AC2 protège. À rouvrir si quelqu'un veut vraiment joindre les deux
  logs par machine.
- Ajouter des champs (niveau, requête, pair) aux lignes existantes. L'horodatage seul débloque #2029 ;
  le reste est une autre conversation.
- La rotation du fichier (334 Ko sur l'hôte de dev) — réel, mais sans rapport.
- Corriger #2029 lui-même. Ce lot lui rend son instrument, il ne le résout pas.

## Lié

- mika#2029 — le p1 que ce lot débloque ; son diagnostic est explicitement arrêté sur ce gap.
- mika#1901 / PR#2019 — ont ajouté la visibilité 429 à ce même log. Cette visibilité vaut beaucoup
  moins sans champ temps pour joindre, comme le dit le corps du ticket.
- mika#1772 — l'enquête d'où le gap a été remonté.

---
module: scripts/test-pilot-egress-proxy-status.py
tags: [ci, flakiness, asyncio, logging-lastresort, redirect-stderr, egress-proxy, mika-2152, mika-2030]
problem_type: test-failure
category: test-failures
date: 2026-09-22
---

# Une assertion sur un flux partagé classe chaque ligne avant d'asserter

## Le problème

`_strip_ts`, le helper partagé de `scripts/test-pilot-egress-proxy-status.py`,
exigeait l'horodatage mika#2030 sur **toute** ligne du stderr capturé — y compris
une ligne qu'asyncio y écrit et que le proxy n'a jamais produite. Le tap de statut
(mika#1901) rougissait donc sur une PR saine, à un taux mesuré de 1/40.

## Le symptôme

Run `33715931630`, 2026-09-03 04:40Z, Python 3.10 sur `ubuntu-22.04` :

```
test_connect_then_close_emits_no_error (__main__.ReadinessProbeVsErrorTests) ... FAIL
AssertionError: unexpectedly None : log line is not timestamped:
  "Executing <Task pending name='Task-1' coro=<IsolatedAsyncioTestCase._asyncioLoopRunner()
   running at /usr/lib/python3.10/unittest/async_case.py:101> … took 0.191 seconds"
```

Le même commit est repassé vert ~3 h plus tard, sans un octet de changement
(l'API Actions ne montre qu'un run de `ci.yml` à 04:40 ; le « même minute » du
ticket mélangeait des workflows). Rien dans le diff ne touchait au proxy ; ce qui
variait était la charge du runner — `0.191 s` contre un seuil de `0.1 s`. Le job dure 6–9 s : le coût n'est pas le débit, c'est la
confiance dans le rouge.

Seconde occurrence, 2026-09-19 (session history) : `test_204_ends_at_the_head`
(une autre classe appelante de `_strip_ts`) rougit sur la même forme — un
avertissement « slow task took 0.314s » dans le stderr capturé — sur une PR sans
rapport. n=2, deux classes de test, une seule cause.

## La cause

Quatre maillons, chacun vérifié (3.14 en local par sonde `python3` ; 3.10 de la CI
**inféré de la source CPython**, non exécuté ici) :

1. **`unittest.IsolatedAsyncioTestCase` arme le mode debug d'asyncio.** En 3.11+
   `_setupAsyncioRunner` construit `asyncio.Runner(debug=True)` ; en 3.8–3.10
   c'est `loop.set_debug(True)`. Le mode debug est ce qui arme l'avertissement de
   rappel lent — hors debug, la ligne n'existe pas. Seuil par défaut
   `slow_callback_duration = 0.1`.
2. **Le logger `asyncio` n'a aucun handler.** Il tombe sur `logging.lastResort`,
   un `_StderrHandler`.
3. **`_StderrHandler.stream` est une propriété qui résout `sys.stderr` à
   l'émission**, pas à l'import. C'est le maillon non évident : c'est pour ça que
   `contextlib.redirect_stderr(buffer)` **avale** la ligne, qui atterrit dans le
   même `StringIO` que les lignes du proxy.
4. **`_strip_ts` supposait au lieu de classer** — « tout ce qui est dans le tampon
   vient du proxy » (`main`, ancien `assertIsNotNone(match, …)`). Vrai sur un
   runner rapide, faux dès qu'un rappel dépasse le seuil.

Un cinquième fait, découvert en écrivant le test de régression : **depuis 3.12
`asyncio.wait_for` attend la coroutine en ligne** (`timeouts.timeout`) au lieu de
l'envelopper dans une `Task`. Voir § Pourquoi ça marche.

## Ce qui a été refusé

- **Voie B — isoler le flux.** `_log` écrit par `print(…, file=sys.stderr)`
  (`scripts/mika-pilot-egress-proxy:74`, `:93`), pas par un logger : il faudrait un
  seam dans le produit **et** réécrire les 15 sites `redirect_stderr` des tests. Un
  ticket de test p2 qui modifie le fichier sous test pour réparer le harnais.
- **Voie C — couper le mode debug.** L'override porte sur des méthodes privées
  (`_setupAsyncioRunner` / `_setupAsyncioLoop`) dont la forme diffère entre 3.10
  (CI) et 3.12+ (local) ; fragile par construction.
- **Relâcher l'assertion.** Le commentaire du ticket le dit : on aurait remplacé
  un test flaky par un test qui ne teste rien (AC2).

## La solution

Fix sur la branche `fix/2152/ci-pilot-egress-status-tap-est-flaky-il`, PR non
ouverte à l'heure d'écrire. Le produit n'est pas touché
(`git diff --stat main -- scripts/mika-pilot-egress-proxy` vide).

**Avant** (`main`) — une seule hypothèse :

```python
match = _TS_PREFIX_RE.match(line)
test.assertIsNotNone(match, f"log line is not timestamped: {line!r}")
```

**Après** — deux constantes à côté de `_TS_PREFIX_RE` (`:47`), puis un helper qui
classe avant d'asserter (`scripts/test-pilot-egress-proxy-status.py:73-115`) :

- `_PROXY_PREFIXES` (`:56`) — les quatre préfixes crochetés que `_log` reçoit :
  `[egress]`, `[egress-shim]`, `[anthropic-proxy]`, `[mitm-forward]`. L'autorité
  d'une ligne est décidable par préfixe parce que `_log` est l'émetteur unique et
  que ses 20 appels ouvrent tous par l'un d'eux.
- `_FOREIGN_LINE_RES` (`:68`) — le bruit énuméré, **ancré aux deux bouts** :
  `^Executing <(?:Task|Handle|TimerHandle)\b.*> took \d+\.\d+ seconds$`. Une seule
  entrée, datée du run.

Les quatre issues, dans cet ordre (le bruit **avant** le test d'horodatage,
sinon il tomberait dans « unclassified ») :

| ligne | verdict | site |
|---|---|---|
| matche `_FOREIGN_LINE_RES` | ignorée, hors invariant | `:95` |
| préfixe proxy, sans horodatage | `proxy log line is not timestamped` — **AC2** | `:100` |
| ni préfixe, ni bruit | `unclassified stderr line` — **AC3** | `:102` |
| horodatée, préfixe inconnu | `unknown prefix — add it to _PROXY_PREFIXES` — **AC3** | `:110` |

`test.fail(...)` remplace `assertIsNotNone` : trois échecs, trois messages, que
les tests lisent par `assertRaisesRegex`. Signature inchangée, donc les six
classes appelantes sont couvertes sans être touchées.

**Le test par mécanisme** (`:1733`) reproduit la CI par sa cause, pas par
injection : `loop.slow_callback_duration = 0.0` (`:1740`) fait flagger **chaque**
rappel, puis, à l'intérieur du `redirect_stderr`, après le `handle_host_client` :

```python
await asyncio.sleep(0)   # :1751 — voir ci-dessous
```

Deux contrôles positifs suivent (`:1756-1757`) : la ligne `Executing <` **était**
dans le tampon, et elle n'est pas horodatée. Sans eux, le vert est vide.

## Pourquoi ça marche

La chaîne complète : `IsolatedAsyncioTestCase` → `debug=True` → rappel > seuil →
`logger.warning` sur le logger `asyncio` sans handler → `logging.lastResort` →
`_StderrHandler.stream` lit `sys.stderr` **au moment de l'émission** →
`redirect_stderr` a remplacé `sys.stderr` → la ligne est dans le tampon du test.
Le filtre reconnaît cette ligne par sa forme et rend le reste au helper inchangé.

**Le `await asyncio.sleep(0)` est porteur, et c'est la leçon 3.12+.** Sans lui,
le test échoue sur son **contrôle positif** (`raw == ''`), pas sur
`not timestamped` — mesuré sur 3.14 le 2026-09-22 (le plan rapporte la même chose
sur 3.12 et 3.13). Depuis 3.12 `wait_for` attend en ligne ; avec `_reader_of()`
(EOF pré-alimenté, `:147`) et un `drain` qui lève de façon synchrone,
`handle_host_client` se termine **sans jamais céder la boucle**. Or asyncio écrit
la ligne de rappel lent à la fin du pas de tâche — qui se clôt alors **après** le
`with`, sur le vrai stderr. Sur le 3.10 de la CI, `wait_for` crée une `Task` et
suspend le pas dans le bloc : c'est pourquoi la ligne y est tombée dans le tampon
« par accident ». Le yield force la fin du pas pendant la redirection, sur toutes
les versions.

## Prévention

**Le motif : une assertion sur un flux partagé classe chaque ligne avant
d'asserter.** Un `stderr` redirigé n'appartient à personne ; « tout vient de moi »
est une hypothèse, pas un fait, et elle casse exactement quand le runner est lent.

- **Allowlist ancrée, jamais de joker (D-2).** Quand `unclassified` tire à
  l'exécution — un `warnings.warn`, un traceback de sous-processus, une nouvelle
  forme asyncio — ajouter un motif **ancré** à `_FOREIGN_LINE_RES` avec le run et
  la date en commentaire. Si la ligne est un traceback, ce n'est pas du bruit :
  c'est le bug.
- **Contrôles négatifs terme par terme (V-5, V-6).** Retirer `"[egress]"` de
  `_PROXY_PREFIXES` doit faire rougir la garde d'inventaire **et** un appelant en
  `unknown prefix` ; vider `_FOREIGN_LINE_RES` doit faire rougir le test par
  mécanisme en `unclassified`. `test_bare_proxy_line_still_fails` (`:1232`) boucle
  sur les quatre préfixes, pas un seul.
- **Garde source ↔ inventaire (D-1).** `test_prefix_inventory_matches_the_proxy_source`
  (`:1254`) compare les **ensembles** de préfixes littéraux des appels `_log(`
  contre `_PROXY_PREFIXES`. Un `_log(f"[new-thing] …")` ajouté au proxy rougit ici
  avant de rougir à l'exécution.
- **Le contrôle positif vit dans le même test** que le « ne rougit plus ». Un test
  qui prouve qu'un bruit est toléré doit d'abord prouver que le bruit était là.
- **Halte.** Si le test par mécanisme rougit sur
  `expected asyncio's slow-callback line in stderr` et non sur `not timestamped`,
  le yield manque dans le bloc `redirect_stderr` : le bruit est parti sur le vrai
  stderr, et le rouge n'est pas celui attendu.

## Voir aussi

- `docs/solutions/runtime-errors/asyncio-wait-closed-hangs-on-live-handlers-2026-08-29.md`
  — même script, même classe : une suite verte sur un interpréteur n'est pas une
  suite verte (asyncio diverge entre 3.10 et 3.12+).
- `docs/solutions/best-practices/log-the-outcome-not-the-policy-verdict-2026-08-27.md`
  § 6 « Allowlist what you log; never filter it » — la règle que `_FOREIGN_LINE_RES`
  applique au stderr capturé.
- `docs/solutions/best-practices/a-guard-must-observe-not-assert-2026-08-29.md` —
  même substrat, même défaut de forme : une vérification qui *suppose* une propriété
  au lieu de l'*observer*.
- `docs/solutions/best-practices/a-count-assertion-on-an-event-name-nothing-emits-is-always-green-2026-08-31.md`
  — le garde source ↔ inventaire est une instance de « épingler l'assertion à la
  source qui émet ».
- `docs/solutions/best-practices/2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md`
  — pourquoi le garde d'inventaire accepte tout jeton crocheté, pas seulement
  `[a-z-]`.

Lignée : mika#2030 (l'invariant d'horodatage), mika#1901 (le tap et sa suite),
mika#2152 (ce ticket). Plan :
`docs/plans/2026-09-21-004-fix-2152-egress-tap-flaky-horodatage-lignes-asyncio-plan.md`.

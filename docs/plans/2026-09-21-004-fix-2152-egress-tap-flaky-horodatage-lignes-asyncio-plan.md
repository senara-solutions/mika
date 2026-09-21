# Plan — mika#2152 : `Pilot Egress Status Tap` exige l'horodatage de lignes qu'asyncio écrit et que le proxy ne produit pas

- **Ticket** : senara-solutions/mika#2152
- **Type** : fix (CI, p2-normal)
- **Branche** : `fix/2152/ci-pilot-egress-status-tap-est-flaky-il`
- **Date** : 2026-09-21
- **Lignée** : mika#2030 (l'exigence d'horodatage, AC1 « toutes les lignes horodatées »),
  mika#1901 (le tap de statut et sa suite), mika#2051 (les tests par sous-processus qui
  lisent le stderr du proxy), mika#2147 (la PR saine qui a rougi)

---

## Contexte

### Le symptôme, mesuré

Run `33715931630` (2026-09-03 04:40Z, branche `fix/2121/…`, Python 3.10 sur
`ubuntu-22.04`) :

```
test_connect_then_close_emits_no_error (__main__.ReadinessProbeVsErrorTests) ... FAIL
AssertionError: unexpectedly None : log line is not timestamped:
  "Executing <Task pending name='Task-1' coro=<IsolatedAsyncioTestCase._asyncioLoopRunner()
   running at /usr/lib/python3.10/unittest/async_case.py:101> wait_for=<Future pending
   cb=[Task.task_wakeup()] created at /usr/lib/python3.10/asyncio/base_events.py:429>
   created at /usr/lib/python3.10/unittest/async_case.py:117> took 0.191 seconds"
```

Son jumeau vert, même commit, même minute. Taux mesuré par le commentaire du ticket :
1/40 sur `ci.yml`. Le job dure 6–9 s ; le coût n'est pas le débit, c'est la confiance
dans le rouge.

### Le mécanisme, reproduit en local (2026-09-21)

Le ticket dit « asyncio l'émet quand un rappel dépasse `slow_callback_duration` ».
C'est exact, et la chaîne complète tient en trois maillons, chacun vérifié :

1. **`unittest.IsolatedAsyncioTestCase` allume le mode debug d'asyncio.** En 3.11+
   `_setupAsyncioRunner` construit `asyncio.Runner(debug=True)` ; en 3.8–3.10
   (`_setupAsyncioLoop`) c'est `loop.set_debug(True)`. Le run CI en échec tourne sur
   3.10 (`/usr/lib/python3.10/unittest/async_case.py:101` dans la ligne fautive).
   Le mode debug est ce qui arme l'avertissement de rappel lent — hors debug, la
   ligne n'existe pas. Le seuil par défaut est `0.1 s` ; le run a mesuré `0.191 s`.
2. **Le logger `asyncio` n'a pas de handler.** Il tombe sur `logging.lastResort`, un
   `_StderrHandler` dont la propriété `stream` résout `sys.stderr` *au moment de
   l'émission* — et non à l'import. C'est pourquoi `contextlib.redirect_stderr(buffer)`
   l'avale : la ligne atterrit dans le même `StringIO` que les lignes du proxy.
3. **`_strip_ts` assert l'horodatage sur tout le tampon** sans distinguer l'auteur
   (`scripts/test-pilot-egress-proxy-status.py:50-63`).

Reproduction déterministe, sans dépendre de la charge :
`asyncio.get_running_loop().slow_callback_duration = 0.0` sous un
`IsolatedAsyncioTestCase` fait émettre la ligne à **chaque** rappel — y compris le
`Task` du test lui-même et les `Handle _run_until_complete_cb`. Deux formes sont
donc observées, pas une : `Executing <Task …> took …` et `Executing <Handle …> took …`.
Le filtre doit reconnaître les deux.

### Ce que le proxy émet réellement

`scripts/mika-pilot-egress-proxy` a un émetteur unique, `_log` (ligne 74) :
`print(f"{ts} {msg}", file=sys.stderr, flush=True)`. Les 20 appels de `_log` du
fichier ouvrent tous le message par un préfixe crocheté, et l'inventaire exhaustif
tient en quatre valeurs :

| préfixe | appels `_log(` dont le littéral ouvre par ce préfixe |
|---|---|
| `[egress]` | 10 |
| `[anthropic-proxy]` | 6 (+ 1 composé : `_log(f"{line} {detail}")` où `line = f"[anthropic-proxy] RATE_LIMITED …"`) |
| `[mitm-forward]` | 2 |
| `[egress-shim]` | 1 |

19 littéraux + 1 composé = 20 appels. Aucun `_log` dont le message n'ouvre pas par
un préfixe crocheté (relevé par regex multi-ligne sur la source, 2026-09-21). C'est ce qui rend la voie A du ticket (filtrer
à la capture) tenable *sans* affaiblir l'assertion : « ligne du proxy » est décidable
par le préfixe, ligne par ligne.

### Le défaut réel : un helper qui suppose au lieu de classer

`_strip_ts` fait une seule hypothèse — « tout ce qui est dans le tampon vient du
proxy » — et cette hypothèse est fausse dès que le runner est lent. Le correctif
n'est pas de relâcher l'assertion (le commentaire du ticket le dit : « on aura
remplacé un test flaky par un test qui ne teste rien »), c'est de **classer chaque
ligne** avant d'asserter :

- **ligne du proxy** (préfixe connu) → l'horodatage est exigé, comme aujourd'hui ;
- **bruit étranger énuméré** (l'avertissement de rappel lent d'asyncio, et lui seul
  pour l'instant) → ignoré, hors périmètre de l'assertion ;
- **tout le reste** → échec explicite, avec un message qui dit quoi faire.

La troisième branche est celle qui satisfait l'AC3 : un nouveau préfixe du proxy
(`[new-thing]`) n'est ni connu ni du bruit énuméré, donc il fait rougir le test
jusqu'à ce qu'on l'ajoute à l'inventaire. Une nouvelle source de bruit fait de même.
Rien ne passe en silence.

### Pourquoi A, et pas B ni C

- **B (isoler le flux)** demanderait un seam dans le produit (`_log` écrit via `print`
  sur `sys.stderr`, pas via un logger) et la réécriture de ~20 sites
  `redirect_stderr` dans les tests. Le gain d'isolation est réel mais le ticket
  est un ticket de test, p2, et B change le fichier sous test pour corriger le
  harnais. Non retenu.
- **C (couper le debug)** : l'override de `_setupAsyncioRunner`/`_setupAsyncioLoop`
  est privé et diffère entre 3.10 (CI) et 3.12+ (local) ; et le ticket dit déjà
  pourquoi c'est fragile. Non retenu.
- **A** ne touche que le helper partagé, couvre tous ses appelants d'un coup
  (« le correctif doit tous les couvrir », § Hors périmètre du ticket), et son
  inventaire de préfixes est vérifiable contre la source du proxy par un test
  (D-1 ci-dessous).

### Ce que ce plan fait, et ce qu'il ne fait pas

Il fait : la classification dans `_strip_ts`, l'inventaire des préfixes, la liste du
bruit étranger, sept tests (dont le cas réel figé et une garde source↔inventaire),
et la mise à jour du docstring qui porte l'intention de mika#2030.

Il ne fait pas : toucher `scripts/mika-pilot-egress-proxy` ; toucher les sites de
test qui lisent `buffer.getvalue()` sans passer par `_strip_ts` (lignes 318, 337,
869, 1041 — ils n'assertent pas l'horodatage, le bruit ne les casse pas) ;
toucher `scripts/test-pilot-egress-keepalive.py` (n'utilise pas le helper).

---

## Requirements

- **R-1** — `_strip_ts` ne fait plus rougir un test pour une ligne étrangère au
  proxy présente dans le tampon. (AC1, AC4)
- **R-2** — `_strip_ts` fait toujours rougir un test pour une ligne **du proxy** non
  horodatée, avec un message qui nomme la ligne. (AC2)
- **R-3** — L'ensemble des préfixes reconnus est une constante explicite du harnais ;
  une ligne horodatée dont le préfixe n'est pas dans cet ensemble fait rougir le
  test avec un message qui dit d'ajouter le préfixe. (AC3)
- **R-4** — L'ensemble du bruit étranger toléré est une constante explicite du harnais
  (motifs, pas préfixes) ; une ligne qui n'est ni proxy-préfixée ni bruit énuméré fait
  rougir le test. (AC3, esprit : rien n'est ignoré en silence)
- **R-5** — L'inventaire R-3 est vérifié contre la source de
  `scripts/mika-pilot-egress-proxy` : tout préfixe littéral d'un appel `_log(...)` est
  dans l'inventaire, et tout élément de l'inventaire apparaît dans la source.
- **R-6** — Un test de régression rejoue le cas réel : la ligne exacte du run
  `33715931630` **et** le mécanisme qui l'a produite (rappel lent sous le loop debug
  d'`IsolatedAsyncioTestCase`), dans le scénario de
  `test_connect_then_close_emits_no_error`. (AC4)
- **R-7** — Chaque test « ne rougit plus » a son contrôle positif dans le même test :
  la preuve que le bruit **était** dans le tampon (sinon le vert est vide).

---

## Approche / Conception

### C-1 — Deux constantes, à côté de `_TS_PREFIX_RE`

```python
# mika#2152: `_strip_ts` asserts the timestamp only on lines the proxy is the
# author of. Authorship is decidable by prefix: `_log` is the proxy's sole
# emitter and every message it is handed opens with one of these. Adding a
# prefix to the proxy without adding it here fails `test_prefix_inventory_*`
# AND every helper caller ("unknown prefix") — loud on purpose (AC3).
_PROXY_PREFIXES: tuple[str, ...] = (
    "[egress]",
    "[egress-shim]",
    "[anthropic-proxy]",
    "[mitm-forward]",
)

# Lines other writers put on the SAME stderr the tests capture. Each entry is
# a source we have seen and deliberately excluded from the timestamp assertion,
# never a wildcard. Today: asyncio's slow-callback warning, armed because
# `IsolatedAsyncioTestCase` runs its loop in debug mode; it reaches the
# redirected stderr through `logging.lastResort`. Run 33715931630, 2026-09-03.
_FOREIGN_LINE_RES: tuple[re.Pattern[str], ...] = (
    re.compile(r"^Executing <(?:Task|Handle|TimerHandle)\b.*> took \d+\.\d+ seconds$"),
)
```

Le motif couvre les trois formes que `asyncio.base_events._format_handle` peut
rendre (`Task` — repr de la tâche ; `Handle` et `TimerHandle` — leurs reprs). Il est
ancré aux deux bouts : ni `Executing <Foo` ni une ligne du proxy ne peuvent le
satisfaire.

### C-2 — `_strip_ts` classe, puis assert

```python
def _strip_ts(test: unittest.TestCase, lines: list[str]) -> list[str]:
    """Return the message bodies of the PROXY's lines, stamp removed, having
    asserted each one is timestamped + parseable (mika#2030 AC1).

    The captured stderr is shared: asyncio's debug loop writes here too
    (mika#2152). So every line is classified before anything is asserted:
      * a known foreign line (`_FOREIGN_LINE_RES`) is skipped — not the
        proxy's, not its invariant;
      * a proxy line (opens with one of `_PROXY_PREFIXES`, after its stamp)
        must be stamped — a bare one fails, that is the AC2 the helper exists
        to hold;
      * anything else fails: an unlisted prefix means the proxy grew an
        emitter nobody inventoried; an unlisted foreign line means a new
        writer shares the stream. Both are for a human to classify, never
        for the helper to ignore.
    """
    stripped: list[str] = []
    for line in lines:
        if not line:
            continue
        if any(pattern.match(line) for pattern in _FOREIGN_LINE_RES):
            continue
        match = _TS_PREFIX_RE.match(line)
        if match is None:
            if line.startswith(_PROXY_PREFIXES):
                test.fail(f"proxy log line is not timestamped: {line!r}")
            test.fail(
                f"unclassified stderr line (neither a proxy prefix in "
                f"{_PROXY_PREFIXES} nor a listed foreign source): {line!r}"
            )
        datetime.datetime.strptime(match.group("ts"), "%Y-%m-%dT%H:%M:%S.%fZ")
        rest = match.group("rest")
        if not rest.startswith(_PROXY_PREFIXES):
            test.fail(
                f"timestamped line carries an unknown prefix — add it to "
                f"_PROXY_PREFIXES if the proxy now emits it: {line!r}"
            )
        stripped.append(rest)
    return stripped
```

Points de conception :

- L'ordre des branches compte : le bruit étranger est écarté **avant** le test
  d'horodatage, sinon il tomberait dans « unclassified ». Une ligne du proxy ne
  peut pas matcher `_FOREIGN_LINE_RES` (elle ouvre par `[`), donc l'ordre n'affaiblit
  pas AC2.
- `test.fail(...)` remplace `assertIsNotNone` : le message est le diagnostic, et
  les trois échecs ont trois messages distincts que les tests C-3 lisent
  (`assertRaisesRegex`).
- La signature et le nom ne changent pas : les six classes appelantes
  (`RelayTapTests`, `BodyEndClosureTests`, `RelayTerminationCountersTests`,
  `UpstreamOutcomeLoggingTests`, `HostSocketLifecycleTests`,
  `ReadinessProbeVsErrorTests`) sont couvertes sans être touchées.

### C-3 — Les tests, en deux classes

**`ForeignLineFilterTests(unittest.TestCase)`** — le helper, en isolation, avec la
ligne du run figée dans une constante de module :

```python
# Verbatim from run 33715931630 (2026-09-03T04:40Z, Python 3.10): the line that
# made a healthy PR red. Kept whole so the regression test reads the real thing.
_RUN_33715931630_LINE = (
    "Executing <Task pending name='Task-1' "
    "coro=<IsolatedAsyncioTestCase._asyncioLoopRunner() running at "
    "/usr/lib/python3.10/unittest/async_case.py:101> wait_for=<Future pending "
    "cb=[Task.task_wakeup()] created at /usr/lib/python3.10/asyncio/base_events.py:429> "
    "created at /usr/lib/python3.10/unittest/async_case.py:117> took 0.191 seconds"
)

# The second shape asyncio's debug loop emits, observed locally (3.14) with
# `slow_callback_duration = 0.0` on 2026-09-21: a Handle, not a Task. Frozen
# next to the run's line so the `<Handle …>` coverage reads a real sample,
# not a free-hand string (architect S2, first pass).
_OBSERVED_HANDLE_LINE = (
    "Executing <Handle _run_until_complete_cb(<Task finishe...unners.py:110>) at "
    "/usr/lib/python3.14/asyncio/base_events.py:181 created at "
    "/usr/lib/python3.14/asyncio/events.py:94> took 0.000 seconds"
)
```

| test | ce qu'il prouve | AC |
|---|---|---|
| `test_the_real_asyncio_line_is_skipped_and_proxy_lines_survive` | `_strip_ts(self, [_RUN_LINE, "<ts> [egress] X"])` == `["[egress] X"]` — le bruit est écarté, la ligne du proxy est rendue. | AC1, AC4 |
| `test_handle_and_timer_handle_shapes_are_foreign_too` | `_OBSERVED_HANDLE_LINE` (forme `<Handle …>` réelle, figée) est écartée ; une forme `<TimerHandle …>` synthétique, dérivée de `_format_handle`, l'est aussi. | AC1 |
| `test_bare_proxy_line_still_fails` | `_strip_ts(self, ["[egress] ERROR x"])` lève `AssertionError` dont le message contient `not timestamped`. Une boucle sur les quatre préfixes. **Contrôle négatif non négociable.** | AC2 |
| `test_timestamped_line_with_unknown_prefix_fails` | `"<ts> [new-thing] hi"` lève `AssertionError` contenant `unknown prefix`. | AC3 |
| `test_unclassified_line_fails` | `"some other writer"` (ni préfixe, ni bruit) lève `AssertionError` contenant `unclassified`. | AC3 |
| `test_prefix_inventory_matches_the_proxy_source` | regex multi-ligne `_log\(\s*f?"(\[[a-z-]+\])` sur `_PROXY_PATH` ; `set(found) == set(_PROXY_PREFIXES)`, message d'échec = différence symétrique. L'appel composé `_log(f"{line} {detail}")` n'est pas vu par la regex, et n'a pas besoin de l'être : son préfixe `[anthropic-proxy]` est déjà dans `found` par ses six littéraux — la garde compare des **ensembles**, pas des comptes. | AC3, R-5 |

**Dans `ReadinessProbeVsErrorTests`** — le cas réel, par son mécanisme :

```python
async def test_connect_then_close_stays_silent_under_slow_callback_noise(self) -> None:
    # mika#2152: the CI failure reproduced by its cause, not by injection.
    # A threshold of 0 makes asyncio's debug loop flag EVERY callback as
    # slow, so the warning lands in the redirected stderr deterministically —
    # on a loaded runner it took 0.191s to get there by accident.
    loop = asyncio.get_running_loop()
    before = loop.slow_callback_duration
    loop.slow_callback_duration = 0.0
    try:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            await proxy.handle_host_client(_reader_of(), self._Writer(fail_drain=True))
        raw = buffer.getvalue()
    finally:
        loop.slow_callback_duration = before
    # Positive control: the noise WAS there. Without this the test is vacuous.
    self.assertIn("Executing <", raw, "expected asyncio's slow-callback line in stderr")
    self.assertFalse(_TS_PREFIX_RE.match(raw.splitlines()[0]), "the noise is not stamped")
    # And the helper reads through it: the probe is still silent.
    self.assertEqual(_strip_ts(self, raw.splitlines()), [])
```

Note d'implémentation : le loop est par test dans `IsolatedAsyncioTestCase`, donc la
restauration dans `finally` est de l'hygiène, pas une nécessité — mais elle évite
que le bruit contamine les assertions de teardown si un jour le loop est partagé.

### Lecture de l'AC4 — un frère, pas le test nommé (encadré, 2026-09-21)

> L'AC4 dit : « la ligne `"Executing <Task pending name='Task-1' …"` dans le flux ne
> fait plus échouer `test_connect_then_close_emits_no_error` ». Ce plan **ne modifie
> pas** ce test. Il prouve le cas par un frère dans la même classe, même scénario
> (`_reader_of()`, `_Writer(fail_drain=True)`, `handle_host_client`, `_strip_ts`),
> avec le mécanisme forcé (`slow_callback_duration = 0.0`) et un contrôle positif
> qui atteste que le bruit était dans le tampon — plus la ligne verbatim du run
> dans `ForeignLineFilterTests`.
>
> **Pourquoi.** L'AC vise le mécanisme (la ligne dans le flux, le helper qui la
> lit), pas le symbole. Le test nommé reste le test canonique de la sonde de
> readiness ; y injecter le bruit en ferait un test à deux sujets. Lecture
> tranchée READY par mika-arch en première passe (session
> `f0c95896-c380-4519-9c3a-caf324eef5e3`) ; cet encadré existe pour que la revue
> QA ne repose pas la question.

### C-4 — Le docstring de `_strip_ts` porte les deux tickets

Le docstring actuel (« Enforcing this on the shared test helpers makes AC1 hold
across every logging path ») est vrai et reste. On lui ajoute la phrase de
mika#2152 : l'invariant tient sur les lignes **du proxy** ; le flux est partagé et
la classification est ce qui rend l'assertion juste sur son périmètre. Le
commentaire de bloc au-dessus de `_TS_PREFIX_RE` gagne une ligne de renvoi.

---

## Fire-Disposition

### D-1 — Garde source ↔ inventaire (C-3, `test_prefix_inventory_matches_the_proxy_source`)

Tire quand quelqu'un ajoute un `_log(f"[new] …")` au proxy sans toucher
`_PROXY_PREFIXES`, **ou** retire le dernier usage d'un préfixe sans le retirer de
l'inventaire. Disposition : le message imprime la différence symétrique ; l'humain
met les deux en accord. C'est la garde de compilation d'AC3 — sans elle, AC3 ne tire
qu'à l'exécution, et seulement si un test capture justement cette ligne.

### D-2 — « unclassified » à l'exécution (C-2, troisième branche)

Tire quand un nouveau writer partage le stderr (un `warnings.warn` du runtime, un
traceback de sous-processus dans les tests mika#2051, une future ligne asyncio de
forme différente). Disposition : **jamais** élargir `_FOREIGN_LINE_RES` avec un
joker ; ajouter un motif ancré, avec le run et la date en commentaire, comme
l'entrée asyncio. Si la ligne est un traceback, ce n'est pas du bruit — c'est le bug.

### D-3 — Contrôle négatif obligatoire (C-3, `test_bare_proxy_line_still_fails`)

Tire si une future « simplification » du helper fait passer une ligne nue du proxy.
Disposition : le test est l'AC2 du ticket ; il ne se contourne pas, il se corrige.

### Ce que cette section ne couvre pas

Une ligne du proxy dont le **message** commencerait par autre chose qu'un préfixe
crocheté. L'inventaire de 2026-09-21 dit qu'il n'y en a aucune ; D-1 le maintient
au niveau des littéraux `_log(f"[…`, mais un `_log(some_variable)` dont la valeur
n'ouvre pas par `[` échapperait à D-1 et tomberait dans D-2 à l'exécution. C'est
accepté : D-2 le rend visible, et le proxy documente déjà (`_log` docstring) que
toute ligne passe par lui.

---

## Phases d'implémentation

### Phase 1 — Rouge avant (R-6, R-7)

1. Ajouter `_RUN_33715931630_LINE`, `ForeignLineFilterTests.test_the_real_asyncio_line_is_skipped_and_proxy_lines_survive` et
   `ReadinessProbeVsErrorTests.test_connect_then_close_stays_silent_under_slow_callback_noise`
   **sans toucher `_strip_ts`**.
2. `python3 -B scripts/test-pilot-egress-proxy-status.py` → les deux rougissent avec
   `log line is not timestamped: "Executing <…`. Coller la sortie dans le corps de la
   PR : c'est la preuve « avant le correctif le test rougit » d'AC1.
3. Commit : `test(egress-proxy): rouge-avant — le bruit asyncio fait rougir le helper (mika#2152)`.

### Phase 2 — Le classement (C-1, C-2, C-4 ; R-1 à R-4)

4. Ajouter `_PROXY_PREFIXES`, `_FOREIGN_LINE_RES` ; réécrire `_strip_ts` ; mettre
   à jour les deux docstrings.
5. Suite verte, 98 + 2 tests.
6. Commit : `fix(ci): _strip_ts classe la ligne avant d'exiger l'horodatage (mika#2152)`.

### Phase 3 — Les gardes (C-3 reste ; R-2, R-3, R-5)

7. Ajouter les cinq autres tests de `ForeignLineFilterTests`. Vérifier
   `test_bare_proxy_line_still_fails` **terme par terme** : les quatre préfixes, pas
   un seul (`feedback_red_before_control_is_term_by_term`).
8. Suite verte, 98 + 7.
9. Commit : `test(egress-proxy): gardes AC2/AC3 + inventaire des préfixes contre la source (mika#2152)`.

### Phase 4 — Vérification de bout en bout et documentation

10. `make test-pilot-egress-proxy` (la cible qui enchaîne les deux suites) vert.
11. Boucle de charge locale, 20 itérations :
    `for i in $(seq 20); do python3 -B scripts/test-pilot-egress-proxy-status.py >/dev/null 2>&1 || echo FAIL $i; done`
    sous `stress -c $(nproc)` ou équivalent si disponible — sinon, la Phase 1 a déjà
    prouvé le mécanisme de façon déterministe et la boucle est un bonus, pas la preuve.
12. Entrée `docs/solutions/best-practices/` : « une assertion sur un flux partagé
    classe chaque ligne avant d'asserter » — courte, avec le lien lastResort →
    redirect_stderr qui n'est pas évident. Laisser le pas `/ce:compound` de la
    pipeline la produire si le pilote y arrive avec le contexte frais.

---

## Contrat de vérification

| # | commande | attendu |
|---|---|---|
| V-1 | Phase 1 étape 2, avant le correctif | 2 FAIL, message `not timestamped: "Executing <` |
| V-2 | `python3 -B scripts/test-pilot-egress-proxy-status.py` après Phase 3 | `Ran 105 tests … OK` (98 existants + 7) |
| V-3 | `python3 -B scripts/test-pilot-egress-keepalive.py` | inchangé, vert |
| V-4 | `git diff --stat main -- scripts/mika-pilot-egress-proxy` | vide — le produit n'est pas touché |
| V-5 | retirer temporairement `"[egress]"` de `_PROXY_PREFIXES` | `test_prefix_inventory_matches_the_proxy_source` rougit **et** au moins un test appelant rougit `unknown prefix` ; restaurer |
| V-6 | remplacer temporairement `_FOREIGN_LINE_RES` par `()` | `test_connect_then_close_stays_silent_under_slow_callback_noise` rougit `unclassified` ; restaurer |

V-5 et V-6 sont les deux contrôles négatifs du ticket (« un test qui accepterait
n'importe quelle ligne aurait remplacé un test instable par un test inutile »), à
exécuter dans le même passage et à coller dans la PR.

---

## Definition of Done

- [ ] `_strip_ts` classe chaque ligne (bruit énuméré / proxy / inconnu) avant
      d'asserter ; signature et nom inchangés ; six classes appelantes intactes.
- [ ] `_PROXY_PREFIXES` et `_FOREIGN_LINE_RES` sont des constantes de module,
      commentées avec leur raison et leur run d'origine.
- [ ] Sept tests ajoutés, dont le cas réel figé (ligne verbatim du run
      `33715931630`) et sa reproduction par mécanisme (`slow_callback_duration = 0`).
- [ ] Preuve rouge-avant (V-1) et les deux contrôles négatifs (V-5, V-6) dans le
      corps de la PR.
- [ ] `scripts/mika-pilot-egress-proxy` non modifié (V-4).
- [ ] `make test-pilot-egress-proxy` vert.

## Acceptance criteria

- **AC1** — Le test passe de façon déterministe quand une ligne non horodatée
  étrangère au proxy apparaît dans le flux. Démontré en injectant une telle ligne :
  avant le correctif le test rougit, après il reste vert.
  → Phase 1 (rouge-avant, V-1) + `test_the_real_asyncio_line_is_skipped_and_proxy_lines_survive`
  + `test_connect_then_close_stays_silent_under_slow_callback_noise` (R-1, R-6, R-7).
- **AC2** — Une ligne **du proxy** non horodatée fait toujours rougir le test.
  → `test_bare_proxy_line_still_fails`, sur les quatre préfixes (R-2, D-3), + V-5.
- **AC3** — Le filtre énumère explicitement les préfixes reconnus et échoue sur un
  préfixe inconnu plutôt que de l'ignorer.
  → `_PROXY_PREFIXES` (R-3), `test_timestamped_line_with_unknown_prefix_fails`,
  `test_unclassified_line_fails` (R-4), `test_prefix_inventory_matches_the_proxy_source`
  (R-5, D-1), + V-6.
- **AC4** — La ligne `"Executing <Task pending name='Task-1' …"` dans le flux ne fait
  plus échouer `test_connect_then_close_emits_no_error`.
  → `_RUN_33715931630_LINE` verbatim + le test par mécanisme dans la même classe,
  même scénario (`_reader_of()`, `_Writer(fail_drain=True)`) (R-6).

## Revision history

- 2026-09-21 — v1, /ce:plan par l'orchestrateur ; mécanisme reproduit en local
  (3.14) et corrélé au run CI (3.10) ; inventaire des préfixes relevé sur la source.
- 2026-09-21 — mika-arch première passe : `Disposition: READY` (session
  `f0c95896-c380-4519-9c3a-caf324eef5e3`). Affûtages appliqués au commit : S1
  (encadré « lecture de l'AC4 »), S2 (`_OBSERVED_HANDLE_LINE` figée). Les cinq
  incertitudes du brief tranchées : frère accepté, motif ancré suffisant (forme
  inconnue → `unclassified`, rouge, pas silence), `test.fail` accepté, trou de la
  garde textuelle accepté (YAGNI), `finally` gardé.

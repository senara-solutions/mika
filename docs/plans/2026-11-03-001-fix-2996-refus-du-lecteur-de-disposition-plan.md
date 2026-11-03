# mika#2996 — tranche « un refus du lecteur de disposition arrête le groom, sans relance, en nommant le moteur »

**Ticket :** senara-solutions/mika issue#2996
**Type :** fix (label `bug`) — substrat de boucle (`loop-substrate`)
**Branche :** `fix/2996/groom-une-disposition-rendue-la-relance`
**Base mesurée :** `f746ad5d2` (`main` au 2026-11-03)
**Corps de PR :** `Refs mika#2996 (tranche refus du lecteur)` — **jamais `Closes`** : le ticket reste ouvert sur AC4.

---

## 1. Le défaut, re-mesuré sur la base

Depuis mika#2194 vol 5 (`22492f3b6`), `_parse_disposition` (`skills/bundled/_shared/dispatch-lib.sh` l. 9893) et
`_parse_verdict` (l. 9935) sont des enveloppes minces : elles délèguent la décision à
`mika arch-disposition --pass first|second --response-file <f>`. Leur `case "$rc"` a trois bras, vérifiés à la lecture :

| `rc` | bras | effet |
|---|---|---|
| `0` | l. 9916-9924 | jeton sur stdout, booléen fuzzy posé depuis la ligne 2 |
| `1` | l. 9925-9928 | **rien** sur stdout — c'est l'UNPARSED, et c'est le chemin de relance (mika#1823 / mika#2641) |
| `*` | l. 9929-9931 | `_arch_disposition_refuse "subcommand_error: …"` — motif sur stderr **et** dans `$_ARCH_DISPOSITION_REFUSAL_FILE` |

Les deux chemins d'échec local — `mktemp` (l. 9902) et l'écriture de la réponse (l. 9906) — passent par la même
`_arch_disposition_refuse` (l. 9883). Les deux enveloppes rendent `0` sur **tous** les chemins, délibérément : les trois
appelants capturent en `$(...)` nu sous `set -e` vivant.

**La conséquence** : sur un refus, l'appelant lit une capture **vide**, exactement comme sur un UNPARSED. Les deux
populations sont indiscernables au site de lecture. Vérifié aux trois sites :

| site | ligne | ce qui se passe aujourd'hui sur un refus |
|---|---|---|
| première passe | l. 11171 | `_trail_append … "UNPARSED"` (l. 11176), relance mika#1823 — **un appel architecte payé** — puis le terminal l. 11480 « the architect never emitted a parsable 'Disposition:' line », **sans `_escalate_groom`**, donc **sans aucune ligne `Groom-halt-cause:`** |
| seconde passe après READY | l. 11264 | `UNPARSED` au trail (l. 11269), relance mika#2641, puis `_escalate_groom … "$GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE"` (l. 11318) |
| seconde passe après ITERATE | l. 11408 | même chemin, escalade l. 11448 |

Les trois torts, dans l'ordre de leur coût :

1. **Une relance payée pour un défaut local.** La relance de mika#1823 / mika#2641 est un remède de *format* : elle
   redemande à l'architecte la ligne de verdict qu'il a omise. Un refus du lecteur n'est pas un défaut de format de
   l'architecte — sa réponse n'a même pas été regardée. Si le refus est déterministe (binaire absent, `TMPDIR` plein),
   la relance le reproduit à l'identique, pour le prix d'un appel LLM.
2. **Une cause d'arrêt fausse.** `architect_unreadable` part dans `audit_events` par
   `try_report_groom_architect_unreadable` (`crates/mika-agent/src/task_engine/dispatcher.rs`), où l'opérateur fait des
   `GROUP BY`. Un refus du moteur gonfle la population « l'architecte n'a pas rendu de verdict ». C'est la classe
   mika#1772 : *une raison d'arrêt qui nomme le mauvais acteur envoie l'opérateur chasser le mauvais défaut.*
3. **Une entrée `-after-retry` sans cause architecte.** Un refus au premier essai suivi d'une relance lisible écrit
   `READY-after-retry` ou `GROOMED-after-retry` (l. 11174-11176) — c'est-à-dire la population même que la sonde AC4 de
   ce ticket compte.

Asymétrie supplémentaire relevée à la lecture et que le corps du ticket ne nomme pas : en **première passe**, le terminal
(l. 11458-11482) fait `_groom_warn` puis `return 1` **sans** appeler `_escalate_groom`. Un refus en première passe ne
produit donc aujourd'hui **aucune** ligne `Groom-halt-cause:` — ni fausse, ni vraie. Faire passer ce site par
`_escalate_groom "first-pass"` est un **comportement nouveau** à ce site, et non la correction d'une cause existante ;
AC-R2 l'exige et le plan l'assume explicitement (§ 3.5).

**Occurrence mesurée en production : 0.** Ni `tasks.result` ni `server.log` ne portent
`REFUSED (arch-disposition, mika#2194)` depuis le deploy de `22492f3b` (relevé du 2026-11-03 à 05:45Z). Tranche
**préventive**, pas réponse à un incident. Conséquence directe : aucune entrée `-after-retry` déjà comptée par la sonde
AC4 ne provient d'un refus, donc la tranche ne retouche pas le compte passé.

---

## 2. La décision : un état distinct, qui ne consomme aucune relance

**Retenu.** Un refus du lecteur est un état terminal nommé, qui arrête le groom sur place, sans rappeler l'architecte.

**Pourquoi pas une relance « par prudence ».** Un refus transitoire (un `mktemp` qui échoue une fois) coûterait alors un
groom perdu au lieu d'une relance. Le prix est un re-kick de `ready` par un humain — rare, visible, et la cause est
nommée. Le prix inverse, celui d'aujourd'hui, est un appel payé **et** une cause fausse à **chaque** refus, y compris
les déterministes : systématique et invisible. Le même arbitrage avait donné son nom propre au refus `plan-callout`.

**Pourquoi pas une relance locale** (relire la même réponse une seconde fois sans appeler l'architecte) : hors tranche
tant que 0 refus est mesuré. Elle n'est pas nécessaire, et elle ajouterait un chemin dont rien n'exerce la condition.

---

## 3. Le geste

### 3.1 Une cause d'arrêt nommée (`dispatch-lib.sh`, après l. 10588)

À la suite des deux constantes existantes, dans le même bloc commenté :

```sh
# Le lecteur de disposition du MOTEUR a refusé de regarder la réponse : code
# `≥2` de `mika arch-disposition`, ou `mktemp`/écriture en échec dans
# l'enveloppe (mika#2996). Ce n'est ni un verdict, ni une absence de verdict de
# l'architecte : sa réponse n'a pas été lue.
#
# Le nom ne porte PAS `architect_`, et c'est la raison d'être de la constante :
# `try_report_groom_architect_unreadable` (`task_engine/dispatcher.rs`) compte la
# population « l'architecte n'a pas rendu de verdict », et un refus local qui y
# entrerait est la classe mika#1772 exactement.
#
# Elle n'a PAS de lecteur Rust dans cette tranche (hors tranche, § 7) : le refus
# reste lisible dans `tasks.result` par la ligne `Groom-halt-cause:` et par
# l'annexe `REFUSED (arch-disposition, mika#2194) <motif>` (l. 12746).
GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED="disposition_reader_refused"
```

Vérifié : les deux scans D4 de `test-dispatch-lib.sh` (l. 14560 `^GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE=` → 1, l. 14564
`^GROOM_HALT_CAUSE_LINE_PREFIX=` → 1) comptent des préfixes **ancrés** qu'une troisième constante ne matche pas. Vérifié
aussi : `canonical_tokens::tests::mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust` n'asserte que la
**présence** des deux lignes existantes (`canonical_tokens.rs` l. 6541-6543) — une constante de plus ne la rougit pas.
`grep -rn 'disposition_reader_refused' skills/ crates/ scripts/ docs/` est vide : le jeton est inédit.

### 3.2 Un refus par appel, et non par processus (`dispatch-lib.sh` l. 9900 et l. 9943)

En tête de **chacune** des deux enveloppes, à côté du `printf '0' > "$_DISPOSITION_FUZZY_FILE"` qui existe déjà et pour
la raison que son commentaire donne déjà mot pour mot (« un résidu du tour précédent ferait mentir
`_disposition_was_fuzzy` sur une réponse franche ») :

```sh
rm -f "$_ARCH_DISPOSITION_REFUSAL_FILE" 2>/dev/null || true
```

Avant le `mktemp`, pour qu'un refus **de cet appel** ne soit jamais effacé par sa propre remise à zéro.

**Aucun refus n'est perdu pour l'annexe.** Après cette tranche, un refus **arrête** le groom : aucune lecture ne le suit
dans le même processus, donc aucune remise à zéro ne peut écraser un motif en attente d'annexe. Le nettoyage au
chargement (l. 9812) et celui du trap `EXIT` (`_dispatch_lib_exit_trap`, l. 3378) restent, pour la raison qui est la
leur — un `$$` recyclé.

**La sentinelle de bascule n'est pas touchée.** Elle compte `{ declare -f _parse_disposition; declare -f _parse_verdict; } | grep -c 'arch-disposition'`
et attend 2. Le nom `_ARCH_DISPOSITION_REFUSAL_FILE` porte `ARCH_DISPOSITION` en majuscules avec un souligné — pas le
littéral à tiret `arch-disposition`. Vérifié sur le corps réel : seule l'invocation de la sous-commande porte le motif
(le gabarit de `mktemp` dit `mika-arch-dispo`, abrégé exprès).

### 3.3 Le prédicat de lecture (`dispatch-lib.sh`, après l. 9820)

Immédiatement après `_disposition_was_fuzzy`, dont il reprend la forme exacte — un prédicat sur le canal fichier, pour
que le chemin du fichier n'ait qu'un site de connaissance :

```sh
_arch_disposition_refused() {
    # Vrai si le DERNIER appel à `_parse_disposition` / `_parse_verdict` a refusé
    # de regarder la réponse (mika#2996). Jumeau de `_disposition_was_fuzzy` :
    # même canal, même raison de sous-shell.
    #
    # `-s` et non `-f` : un fichier vide n'est pas un refus. Les deux enveloppes
    # le vident en tête, donc l'absence ET le vide disent « cet appel n'a pas
    # refusé » — mika#2331, absent n'est pas zéro, et ici les deux lectures
    # doivent coïncider parce que le canal a deux états d'effacement.
    [ -s "${_ARCH_DISPOSITION_REFUSAL_FILE:-}" ]
}
```

### 3.4 Le garde unique (`dispatch-lib.sh`, après `_escalate_groom`)

```sh
_halt_groom_on_reader_refusal() {
    # Arrête le groom sur un refus du lecteur de disposition (mika#2996), SANS
    # relance : la relance de mika#1823 / mika#2641 est un remède de format
    # adressé à l'architecte, et ici sa réponse n'a pas été lue.
    #
    # Appelé aux TROIS sites de lecture, entre la capture et `_trail_append` :
    # avant, pour qu'aucune ligne `UNPARSED` ne soit écrite au trail pour un
    # refus (AC-R4).
    #
    # Args: $1 = stage ("first-pass" | "second-pass-after-ready" |
    #       "second-pass-after-iterate"), $2 = libellé de trail
    #       ("groom-ticket" | "second-review"), $3 = réponse de l'architecte,
    #       $4 = session_id.
    #
    # Rend TOUJOURS 0, comme les deux enveloppes et pour la même raison : les
    # sites l'appellent en position d'instruction sous `set -e` vivant. C'est le
    # `return 1` du SITE qui sort de la boucle — voir le commentaire du site.
    local stage="$1" trail_skill="$2" content="$3" session_id="$4"
    # Jeton distinct, et NU : ni `(fuzzy)` — le canal fuzzy vaut `0` sur un
    # refus —, ni `-after-retry`, puisqu'il n'y a jamais de relance après lui.
    # Il ne matche donc pas le filtre de la sonde AC4, qui lit
    # `READY-after-retry` et `GROOMED-after-retry`.
    _trail_append "$trail_skill" "$session_id" "REFUSED-reader"
    _escalate_groom "$stage" "$content" "$session_id" "$GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED"
    return 0
}
```

La réponse de l'architecte est préservée dans le fichier de constats comme sur tout arrêt :
`_escalate_groom` écrit `$content` dans `$WORKTREE_DIR/.iterate/escalate-<stage>.md` (l. 10655-10658) avant son `case`.

### 3.5 Le quatrième bras de `_escalate_groom` (`dispatch-lib.sh`, dans le `case "$cause"`)

Entre le bras `ARCHITECT_UNREADABLE` et le bras `*)`. **Sans ce bras, un refus tomberait dans le `*)` bruyant**
(l. 10692-10710), qui n'affirme délibérément aucune cause — donc `RESULT` ne porterait pas la ligne qu'AC-R2 exige.

```sh
        "$GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED")
            echo "iterate_groom_loop: ${GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED} at ${stage} — the engine's disposition reader refused the response; no architect retry (mika#2996; findings at ${findings_file})" >&2
            # AUCUNE ligne `Verdict:` : il n'y a pas de verdict, et la réponse de
            # l'architecte n'a même pas été lue.
            #
            # La prose nomme le MOTEUR. Elle ne dit pas « the architect never
            # emitted », parce que c'est faux ici — et c'est la seule phrase que
            # l'opérateur verra dans `tasks.result` (PR#2028).
            #
            # Deux champs positionnels — la cause puis l'étape — comme le bras
            # ci-dessus : un seul format de fil (D4).
            RESULT="${RESULT}
GROOM ESCALATED (terminal): the engine's disposition reader refused the architect's response at ${stage}.
${GROOM_HALT_CAUSE_LINE_PREFIX} ${GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED} ${stage} — \`mika arch-disposition\` could not read the response (exit ≥2, or a local mktemp/write failure). The architect was NOT asked again: this is a local refusal, not a missing verdict line. See the \`REFUSED (arch-disposition, mika#2194)\` annex below for the motive.
Session: ${session_id}
Architect findings preserved at: ${findings_file}"
            ;;
```

Le `$4` documenté de `_escalate_groom` (l. 10643-10650) gagne la troisième valeur dans son commentaire d'arguments.

### 3.6 Les trois sites d'appel

Forme identique aux trois sites, insérée **entre la capture et `_trail_append`** :

```sh
        disposition=$(printf '%s' "$content1" | _parse_disposition)
        if _arch_disposition_refused; then
            _groom_warn "the engine's disposition reader refused the first-pass response (mika#2996) — \
no architect retry; the architect was not asked again"
            _halt_groom_on_reader_refusal "first-pass" "groom-ticket" "$content1" "$session_id"
            return 1
        fi
        local _trail_suffix=""
```

- **l. 11171**, première passe : stage `first-pass`, trail `groom-ticket`, contenu `$content1`.
- **l. 11264**, seconde passe après READY : stage `second-pass-after-ready`, trail `second-review`, contenu `$content2`.
- **l. 11408**, seconde passe après ITERATE : stage `second-pass-after-iterate`, trail `second-review`, contenu
  `$content2_iter`.

Le `_groom_warn` est **au site, et non dans le garde**, pour deux raisons mesurées :

1. **C'est l'idiome déjà établi de cette boucle.** Les cinq sites d'escalade existants font `_groom_warn "…"` puis
   `_escalate_groom …` puis `return 1` (p. ex. l. 11455-11457).
2. **C'est ce que `_reason_pairing_check` exige** (`test-dispatch-lib.sh` l. 5880-5908, détecteur non cité par le
   ticket). Il balaie le corps de `_iterate_groom_loop` par `declare -f` et, pour **chaque** ligne portant `return 1`,
   exige dans la fenêtre de trois lignes `"$line$prev1$prev2"` l'un de `_groom_warn `, `_groom_warn_empty_content ` ou
   `GROOM_LOOP_FAILURE_REASON=`. Avec le `_groom_warn` au site, la fenêtre est
   (`return 1`, appel du garde, `_groom_warn …`) : elle matche. **Le détecteur reste vert sans qu'on y touche**, et il
   reste honnête — `_groom_warn` (l. 10936) pose réellement `GROOM_LOOP_FAILURE_REASON`, qui voyage jusqu'à
   `tasks.result`. Mettre le `_groom_warn` dans le garde aurait obligé à ajouter un nom au `case` du scan ; c'est le
   geste que son propre commentaire mika#2296 autorise, mais il est inutile ici, et une modification de détecteur de
   moins est une exception de moins à déclarer.

La raison nomme le moteur et non l'architecte (mika#1772). Le `return 1` est au site et non rendu par le garde : le
garde est appelé en position d'instruction sous `set -e` vivant, où un code non nul tuerait le shell entier.

**Les trois sites gardent leur relance pour l'UNPARSED réel, à l'octet près.** Le garde ne se déclenche que si le canal
de refus est non vide ; sur `rc=1` l'enveloppe n'y écrit rien. Le chemin de mika#2996 (relance qui porte le plan entre
les lignes de marqueur, ancrage vérifié) et les assertions V4 de taille d'octets du prompt de relance
(`test-dispatch-lib.sh` l. 14500-14522) ne sont pas touchés.

### 3.7 Les tests (`test-dispatch-lib.sh`)

Insérés **avant** le bloc `--- Summary ---` (l. 16960-16971).

**La sonde, décalquée de `_groom_retry_plan_probe_2996`** (l. 16852-16909) : source de `$DISPATCH_LIB` en sous-shell,
`export MIKA_ARCH_ASK_RETRY=0`, compteur d'appels dans `$ARCH_CALLS`, `_arch_ask` remplacé et dispatché sur
`case "$site:$n"`, `_write_canonical_callout` et `_launch_revise_pilot` à `return 0`, `WORKTREE_DIR ISSUE_NUM REPO BRANCH`
posés, `_iterate_groom_loop >/dev/null 2>&1 || true`.

Trois écarts par rapport au gabarit, chacun nécessaire :

1. **Un `mika()` de shell éclipse le binaire**, sur le modèle de l. 11646 (`mika() { return 3; }`) et du compteur de
   l. 8669. Il doit être **sélectif par passe**, sinon un refus en première passe arrêterait tout avant d'atteindre les
   deux autres sites. Son contrat de sortie est celui que lisent les enveloppes (l. 9916-9918) : jeton en ligne 1, tier
   en ligne 2, code 0.

   ```sh
   mika() {
       # $1 = arch-disposition, $2 = --pass, $3 = first|second
       case "$3:$M2996R_REFUSE_AT" in
           first:first)   return 3 ;;
           second:second) return 3 ;;
       esac
       case "$3" in
           first)  printf '%s\n1\n' "$M2996R_FIRST_TOKEN" ;;
           second) printf 'GROOMED\n1\n' ;;
       esac
       return 0
   }
   ```

   `$M2996R_FIRST_TOKEN` vaut `READY` pour le site after-ready et `ITERATE` pour le site after-iterate — c'est ce qui
   choisit la branche de la boucle.
2. **La sonde appelle `_annex_arch_disposition_refusal` elle-même** avant d'imprimer `RESULT`. L'annexe est posée par
   `_deliver_callback` (l. 12771), que la sonde ne traverse pas ; sans cet appel explicite, AC-R2 ne pourrait pas
   observer la ligne `REFUSED (arch-disposition, mika#2194)`.
3. **Elle imprime trois mesures** : `arch_calls=`, le `RESULT` entre `RESULT_BEGIN`/`RESULT_END`, et le contenu du
   trail (`$WORKTREE_DIR/.claude/groom-verdict-trail.log`) entre `TRAIL_BEGIN`/`TRAIL_END`.

**Les assertions**, par site :

- **AC-R1** — `arch_calls` vaut `1` (première passe), `2` (après READY), `2` (après ITERATE). Aujourd'hui : 2, 3, 3.
- **AC-R2** — `RESULT` contient `Groom-halt-cause: disposition_reader_refused <stage>` et
  `REFUSED (arch-disposition, mika#2194) subcommand_error` ; `RESULT` **ne contient pas** `architect_unreadable`, ni
  `the architect never emitted`, ni `Verdict:`.
- **AC-R4** — le trail contient `REFUSED-reader` et ne contient ni `UNPARSED` ni `-after-retry`.
- **AC-R3, contrôle négatif dans le même corps** — même sonde, `mika()` rendant `1` (UNPARSED) à la passe visée au lieu
  de `3` : `arch_calls` vaut 2, 3, 3 comme aujourd'hui, la cause est `architect_unreadable` aux deux sites de seconde
  passe, et `RESULT` ne porte **pas** `disposition_reader_refused`. Sans ce contrôle, « ne relance plus sur refus » et
  « ne relance plus du tout » se liraient pareil (classe mika#2205 : un prédicat sur une population vide se lit comme
  une population saine).

### 3.8 Les deux assertions S-c de mika#2641 — l'exception déclarée d'AC-R5

**Avant** (`test-dispatch-lib.sh` l. 14546-14547 et l. 14554-14555) :

```sh
assert_eq "mika#2641 (S-c): exactement cinq sites appellent _escalate_groom" \
    "5" "$M2641_CALL_COUNT"
```
```sh
M2641_CAUSED_COUNT=$(printf '%s\n' "$M2641_CALL_LINES" \
    | grep -cE '"\$GROOM_HALT_CAUSE_(VERDICT|ARCHITECT_UNREADABLE)"[[:space:]]*$' || true)
assert_eq "mika#2641 (S-c): les cinq sites passent une cause explicite, par la constante" \
    "5" "$M2641_CAUSED_COUNT"
```

**Après** :

```sh
assert_eq "mika#2641 (S-c): exactement six sites appellent _escalate_groom" \
    "6" "$M2641_CALL_COUNT"
```
```sh
M2641_CAUSED_COUNT=$(printf '%s\n' "$M2641_CALL_LINES" \
    | grep -cE '"\$GROOM_HALT_CAUSE_(VERDICT|ARCHITECT_UNREADABLE|DISPOSITION_READER_REFUSED)"[[:space:]]*$' || true)
assert_eq "mika#2641 (S-c): les six sites passent une cause explicite, par la constante" \
    "6" "$M2641_CAUSED_COUNT"
```

Le sixième site est l'appel **unique** du garde (§ 3.4), écrit en début de ligne avec un premier argument entre
guillemets (`"$stage"`) — la forme que le `grep -nE '^[[:space:]]*_escalate_groom[[:space:]]+"'` de l. 14544 reconnaît —
et dont la constante de cause est en fin de ligne, ce que le second prédicat exige.

**La troisième assertion de cardinalité ne change pas, et c'est le cœur du choix de conception.**
`test-dispatch-lib.sh` l. 1159-1161 lit le corps de la boucle, pas le fichier :

```sh
ITERATE_NOW=$(declare -f _iterate_groom_loop)
ESCALATE_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_escalate_groom')
assert_eq "_iterate_groom_loop has 5 _escalate_groom call sites (mika#2641)" "5" "$ESCALATE_CALL_COUNT"
```

Le garde étant une **fonction à part**, `declare -f _iterate_groom_loop` ne porte pas son corps : ce compte reste à 5,
et les assertions de libellé d'étape de l. 1164-1166 restent vraies. Inliner l'escalade aux trois sites aurait fait
passer ce compte à 8 et aurait rougi trois assertions au lieu de deux — d'où le garde unique.

La divergence 5/6 entre les deux scans est **nominale** et déjà anticipée par le commentaire de l. 14535-14539 (« un
appel placé hors de la boucle rougit celle-ci et pas l'autre »). Ce commentaire, qui dit aujourd'hui « DEUX assertions de
cardinalité à 5 », est amendé pour nommer le chiffre de chaque côté et la raison de l'écart — c'est de la prose, pas une
attente.

### 3.9 Ce qui ne bouge pas

- **Aucun fichier sous `crates/`.** `agent_loop/mod.rs` est à 1 046 935 octets sur 1 048 576 et cette tranche n'y écrit
  rien. Aucun lecteur Rust de la nouvelle cause (§ 7). Vérifié que rien ne rougit côté Rust : § 3.1.
- `crates/mika-agent/tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs` : aucune attente touchée —
  le chemin de relance sur UNPARSED réel est inchangé (§ 3.6), et AC-R3 l'épingle par le comportement.
- La garde d'ancrage de mika#2037, `review_anchor.rs`, et le message de relance porteur du plan : inchangés.

---

## 4. Acceptance criteria

Transcription verbatim des critères de la tranche, tels qu'ils figurent au corps du ticket.

### Critères de la tranche (le vol en cours)

- **AC-R1 : pas de relance sur un refus, aux trois sites.** La sonde existante `_groom_retry_plan_probe_2996`
  (`test-dispatch-lib.sh` l. 16852-16909) sert de gabarit, avec `mika() { return 3; }`. Le compte d'appels architecte
  attendu est le suivant :
  - 1 en première passe ;
  - 2 en seconde passe après READY ;
  - 2 en seconde passe après ITERATE.

  Aujourd'hui, il vaut respectivement 2, 3 et 3.
- **AC-R2 : cause et raison nommées.** Sur chacun des trois sites, `RESULT` porte la ligne
  `Groom-halt-cause: disposition_reader_refused <stage>` et l'annexe
  `REFUSED (arch-disposition, mika#2194) subcommand_error`. Il ne porte **pas** `architect_unreadable`, ni « the
  architect never emitted ».
- **AC-R3 : contrôle négatif dans le même corps.** Avec le binaire réel, ou un `mika()` qui rend `1` (UNPARSED), les
  trois sites relancent comme aujourd'hui : 2, 3 et 3 appels, avec la cause `architect_unreadable` sur le double
  UNPARSED. Sans ce contrôle, « ne relance plus sur refus » et « ne relance plus du tout » se liraient pareil.
- **AC-R4 : le trail.** Un refus écrit `REFUSED-reader` et jamais `UNPARSED`. Aucune entrée `-after-retry` n'est écrite
  sur un groom dont le premier essai a refusé.
- **AC-R5 : les attentes existantes ne changent pas, à une exception déclarée près.** Les tests de mika#1823, mika#2641
  et mika#2996 passent sans modification de leurs attentes, et
  `crates/mika-agent/tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs` aussi. **Seule exception :**
  les deux assertions S-c de mika#2641 (`test-dispatch-lib.sh` l. 14547 et 14555). Elles épinglent « exactement cinq
  sites appellent `_escalate_groom` » et la population de causes `VERDICT|ARCHITECT_UNREADABLE`. Elles passent à six
  sites, puisque le garde unique est un seul site, et la population gagne la nouvelle constante. Le plan cite ces deux
  lignes avant et après.

### Critères du ticket parent (contexte — hors tranche)

AC1 à AC3 sont livrés. AC4 reste ouvert et court jusqu'à son terme, sans changement.

- **AC1.** Une disposition qui fait avancer la chaîne (`READY` en première passe, `Verdict: GROOMED` en seconde) n'est
  jamais acceptée sans ancrage vérifié **contre le plan**. Cela vaut aussi quand elle est rendue au tour de relance de
  mika#1823 ou de mika#2641. *(Livré.)*
- **AC2.** Un test dans le même appel, avec contrôle négatif. Une première passe `UNPARSED`, suivie d'une relance qui
  rend `Disposition: READY` sans ancre valide sur le plan, n'avance pas à la seconde passe. Même fixture avec une
  relance ancrée sur le plan : la chaîne avance. Même chose pour la relance de seconde passe et `Verdict: GROOMED`.
  *(Livré.)*
- **AC3.** Le cas mesuré sert de fixture : la réponse de la trace `15ea28bd`, qui porte sur un autre plan et se termine
  par `Disposition: READY`, est refusée. *(Livré.)*
- **AC4.** Sonde après déploiement : sur les 30 jours suivants, le compte d'entrées `READY-after-retry` ou
  `GROOMED-after-retry` des journaux de verdicts qui ont avancé sans ancrage vérifié est écrit ici. La cible est 0.
  *(Ouvert — la tranche ne le perturbe pas : § 6.)*

---

## 5. Fire-Disposition

La tranche livre des détecteurs : la sonde de refus aux trois sites, son contrôle négatif, et les deux assertions S-c
amendées. La section est donc requise (mika#2306).

**Option retenue : (a) exception nommée dans une liste d'exclusion, grep-visible, référencée au ticket, et dont
l'assertion s'auto-nettoie.** Elle s'applique au seul détecteur qui a une population susceptible de croître : le scan de
cardinalité S-c.

Concrètement :

- **Les deux assertions S-c restent des cardinalités exactes** (`6`, et non `≥ 5`). Un scan qui compterait sans borner
  passerait en ne regardant rien le jour où le nom de la fonction change — c'est la raison, déjà écrite à l. 14530-14533,
  qui a fait asserter la cardinalité plutôt que la couverture. Un sixième site **non prévu** doit rougir.
- **Si un site futur doit légitimement s'ajouter**, le geste est le même qu'aujourd'hui : monter le chiffre **dans le
  même commit** que le site, et ajouter la constante de cause à l'alternation du second prédicat. C'est une exception
  nommée par construction — elle est visible au `grep -n 'S-c'` et elle expire d'elle-même, puisque le chiffre ne peut
  pas rester juste sans être touché. Aucune liste d'exclusion séparée n'est créée : celle-ci serait précisément
  l'allowlist que mika#2309 condamne.
- **Si la sonde de refus devient rouge sur un refus réel en production**, elle ne se désarme pas : elle a mesuré le
  comportement attendu, et c'est le code qui a changé. La disposition est alors *halt-and-escalate* au sens de
  l'option (c) — mais ce chemin est celui du détecteur en fonctionnement normal, pas une exception.
- **Aucun détecteur n'est livré désarmé** : pas de `#[ignore]`, pas de `#[cfg(skip)]`, pas de garde CI sans câblage. Les
  deux fichiers touchés sont déjà atteints par la suite (`test-dispatch-lib.sh` tourne dans la CI), donc le lint
  `suite-wiring-lint` n'a rien à déclarer et aucune entrée `unwired` n'est ouverte.

Le prédicat du nouveau détecteur est borné de la même façon : AC-R1 asserte des **égalités** de compte d'appels (1, 2,
2), pas des inégalités — un compte qui tomberait à 0 parce que `_arch_ask` n'a jamais été atteint rougirait au lieu de
passer (mika#2205). Et AC-R4 asserte à la fois la **présence** de `REFUSED-reader` et l'**absence** d'`UNPARSED` : la
seconde moitié est ce qui empêche l'assertion de passer sur un trail vide (mika#2331).

---

## 6. Ce que la tranche fait à la sonde AC4

La sonde compte, sur 30 jours, les entrées `READY-after-retry` ou `GROOMED-after-retry` qui ont avancé sans ancrage
vérifié. Sa cible est 0. La tranche la **protège** sans la perturber :

- **Elle ne touche pas le mécanisme mesuré.** Ni la garde d'ancrage (`agent_loop/mod.rs`, `review_anchor.rs`), ni la
  relance sur UNPARSED réel, ni le message de relance de mika#2996. AC-R3 et AC-R5 l'épinglent.
- **Elle retire de la population une source parasite.** Une entrée `-after-retry` née d'un refus du moteur n'est pas un
  UNPARSED de l'architecte ; après la tranche, elle ne peut plus exister (AC-R4).
- **Elle ne change pas le compte passé.** 0 refus est mesuré en production depuis `22492f3b`, donc aucune entrée déjà
  comptée n'en provient.
- **Le jeton `REFUSED-reader` ne matche pas le filtre de la sonde**, qui lit `READY-after-retry` et
  `GROOMED-after-retry`. Le garde l'écrit nu, sans suffixe (§ 3.4).

---

## 7. Hors tranche

- **Un lecteur Rust de `disposition_reader_refused`** dans `audit_events`, sur le modèle de
  `try_report_groom_architect_unreadable`. Utile, mais il touche `crates/` et `dispatcher.rs`. Il viendra si la cause
  apparaît en production. En attendant, le refus est lisible dans `tasks.result` — et
  `mika2641_another_halt_cause_is_not_counted` prouve déjà que le lecteur Rust existant **ignore** une cause différente,
  donc la nouvelle cause ne pollue aucune population Rust.
- **Toute relance locale** (relire la même réponse une seconde fois sans appeler l'architecte) : inutile tant que 0
  refus est mesuré.
- **La phase 4 de mika#2194**, c'est-à-dire le retrait des deux canaux fichier.
- **La fermeture de ce ticket**, qui attend toujours AC4. Le corps de PR porte `Refs mika#2996 (tranche refus du
  lecteur)`, jamais `Closes`.

---

## 8. Risques

| risque | mesure |
|---|---|
| Le garde se déclenche sur un refus d'un appel **précédent** du même processus | La remise à zéro en tête des deux enveloppes (§ 3.2) est la condition de justesse de tout le reste. Elle est assertée par le contrôle négatif d'AC-R3, où un UNPARSED après un appel sain ne doit pas lire de refus. |
| `_reason_pairing_check` rougit sur la nouvelle forme des trois sites | Le `_groom_warn` est au site, dans la fenêtre de trois lignes (§ 3.6). Le détecteur tourne dans la même suite ; il est vérifié avant de pousser. |
| Le bras `*)` bruyant de `_escalate_groom` avale la nouvelle cause | C'est exactement pourquoi le quatrième bras est livré dans le même commit que la constante (§ 3.5). Un oubli produirait un `RESULT` sans ligne de cause, et AC-R2 le verrait. |
| La sentinelle de bascule de mika#2194 rougit | Le nom de variable porte `ARCH_DISPOSITION`, pas le littéral à tiret (§ 3.2). Vérifié sur le corps réel des deux enveloppes. |
| `agent_loop/mod.rs` franchit la borne d'octets | Aucun fichier sous `crates/` n'est touché (§ 3.9). |

---

## 9. Taille estimée

| livrable | lignes de code (hors docs/) |
|---|---|
| `dispatch-lib.sh` — constante `GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED` + son commentaire | 12 |
| `dispatch-lib.sh` — remise à zéro du canal en tête des deux enveloppes (2 × 4) | 8 |
| `dispatch-lib.sh` — prédicat `_arch_disposition_refused` | 8 |
| `dispatch-lib.sh` — garde `_halt_groom_on_reader_refusal` | 24 |
| `dispatch-lib.sh` — quatrième bras de `_escalate_groom` + doc de `$4` | 16 |
| `dispatch-lib.sh` — les trois sites d'appel (3 × 6) | 18 |
| `test-dispatch-lib.sh` — sonde de refus aux trois sites (gabarit décliné, `mika()` sélectif, annexe, trail) | 95 |
| `test-dispatch-lib.sh` — contrôle négatif UNPARSED (même corps, polarité paramétrée) | 22 |
| `test-dispatch-lib.sh` — assertions de trail (AC-R4) | 14 |
| `test-dispatch-lib.sh` — les deux assertions S-c + le commentaire de divergence 5/6 | 10 |

Total estimé : 227 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi la borne tient.** 227 est à 53 % de 429. La mesure est faite sur la surface réelle et
non reprise du corps du ticket (qui disait 160 à 200) : elle est **plus haute**, parce que le corps ne comptait ni le
quatrième bras de `_escalate_groom`, ni le prédicat, ni le contrôle négatif comme un corps à part. La référence mesurée
la plus proche est le vol 6 de mika#2194 : 86 lignes estimées, 185 ajoutées pour 24 retirées, soit un facteur de 2,15.
Appliqué ici, 227 × 2,15 donne 488 — au-dessus de la borne. Mais ce facteur n'est pas transposable : le dépassement du
vol 6 venait d'un scan structurel dont le gabarit avait été sous-estimé, alors que le gabarit de celui-ci
(`_groom_retry_plan_probe_2996`, 58 lignes mesurées) est **lu et compté**, pas deviné. En prenant le facteur 1,5 que
justifie un gabarit déjà en main, 227 × 1,5 donne 341 : sous la borne, avec 88 lignes de marge. Deux fichiers, aucune
recompilation Rust, aucune fixture à fabriquer : le vol est court en tours autant qu'en lignes. Au-delà de 429 réelles,
la coupe naturelle serait production (§ 3.1-3.6) d'abord, sonde (§ 3.7-3.8) ensuite — mais elle laisserait un geste sans
détecteur pendant un vol, ce que la tranche n'a pas besoin de payer.

Sous 1000 lignes : pas de découpage en phases.

---

## 10. Definition of Done

- [ ] `GROOM_HALT_CAUSE_DISPOSITION_READER_REFUSED="disposition_reader_refused"` déclarée une seule fois, à côté des deux
      autres, avec son commentaire.
- [ ] Les deux enveloppes vident `$_ARCH_DISPOSITION_REFUSAL_FILE` en tête, avant le `mktemp`.
- [ ] `_arch_disposition_refused` livré à côté de `_disposition_was_fuzzy`.
- [ ] `_halt_groom_on_reader_refusal` livré : trail `REFUSED-reader`, `_escalate_groom` avec la nouvelle cause,
      `return 0`, aucune relance.
- [ ] `_escalate_groom` a un quatrième bras qui écrit la ligne `Groom-halt-cause: disposition_reader_refused <stage>`,
      sans ligne `Verdict:`, en nommant le moteur.
- [ ] Les trois sites appellent le garde entre la capture et `_trail_append`, avec `_groom_warn` au site.
- [ ] Sonde de refus aux trois sites : `arch_calls` = 1, 2, 2.
- [ ] `RESULT` porte la cause et l'annexe, et ne porte ni `architect_unreadable` ni « the architect never emitted »
      (AC-R2).
- [ ] Trail : `REFUSED-reader` présent, `UNPARSED` et `-after-retry` absents (AC-R4).
- [ ] Contrôle négatif UNPARSED dans le même corps : 2, 3, 3 appels et cause `architect_unreadable` (AC-R3).
- [ ] Les deux assertions S-c passent à 6, avec la constante ajoutée à l'alternation ; le commentaire de l. 14535 nomme
      la divergence 5/6.
- [ ] `test-dispatch-lib.sh` l. 1161 reste à 5, et `_reason_pairing_check` reste à `0/n`.
- [ ] Aucun fichier sous `crates/` modifié ; `cargo test -p mika-agent canonical_tokens` et
      `cargo test -p mika-agent mika2641` verts.
- [ ] Suite `test-dispatch-lib.sh` entièrement verte ; `cargo clippy`, `cargo fmt --check`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Corps de PR : `Refs mika#2996 (tranche refus du lecteur)`, `Code review: delegated to mika-qa (mika#2694)`.
      Jamais `Closes`.
- [ ] Merge : `dispatch-lib.sh` est du substrat de boucle — le merge passe par le mot de Vincent, PR par PR.

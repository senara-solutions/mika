# Plan — mika#2155 : dispatch-lib estampille `dispatch:loop` sur l'issue — la garde de siège devient symétrique

- **Ticket** : senara-solutions/mika#2155
- **Type** : feat (substrat de la boucle, p2)
- **Branche** : `feat/2155/dispatch-dispatch-lib-n-estampille`
- **Date** : 2026-09-21
- **Lignée** : mika#2084 / PR#2091 (la garde de siège), mika#2092 (le vocabulaire déclaré
  et gardé Rust↔YAML), mika#2026 (`origin:loop` — le producteur estampille au moment de
  la production), mika#2178 (une seule lecture de l'issue, pas de fenêtre TOCTOU),
  mika#2201 (règle L5 : tout `--add-label` littéral dans dispatch-lib doit être déclaré)

---

## Contexte

### Le défaut, vérifié sur ce checkout (2026-09-21)

`crates/mika-agent/src/webhook_dispatch.rs:271` pose `CURRENT_DISPATCH_SEAT = "loop"` ;
`classify_dispatch_seat` (`:375-418`) rend `SeatVerdict::OwnedByCurrentSeat` sur
`dispatch:loop`. Les trois sites d'appel (`auto_pull.rs:1945`,
`ready_label_handler.rs:791`, `skills/executor.rs:1653` — plus
`milestone_context_handler.rs:349`) refusent un ticket portant `dispatch:ssc` ou
`dispatch:mpc`. **Aucun site du dépôt ne pose `dispatch:loop`** : le seul
`--add-label` visant une étiquette de siège est absent ; `grep -rn 'dispatch:loop'`
hors docs ne rend que la constante Rust, ses tests, le lint mika#2092 et sa fixture.

Le pendant existe pour une autre étiquette : `_stamp_pr_origin`
(`skills/bundled/_shared/dispatch-lib.sh:6398`) pose `origin:loop` **sur la PR** au
moment où dispatch-lib la produit ou la découvre (trois sites : `:1631` trap EXIT,
`:4233` chemin nominal, `:7697` PR de sauvetage). La doctrine y est écrite : *l'origine
est un fait posé par son producteur, jamais reconstruit après coup*.

### La dépendance est levée

Le corps du ticket dit « bloqué par mika#2092 ». Mesuré le 2026-09-21 : #2092 est
**CLOSED** (2026-09-08T10:00:38Z) ; `.github/labels.yml:139` déclare `dispatch:loop`
(couleur `1d76db`) ; `gh label list --repo senara-solutions/mika --search dispatch` rend
les trois sièges `loop`, `ssc`, `mpc` ; `scripts/check-dispatch-seats-declared.sh` est
câblé dans `make test` et compare `KNOWN_DISPATCH_SEATS` ↔ YAML dans les deux sens. La
phrase « Tant que #2092 n'est pas mergé, `gh issue edit --add-label dispatch:loop`
échoue » décrit un état passé : l'appel réussit aujourd'hui. Aucune divergence avec le
ticket — sa condition de réveil est remplie, c'est tout.

### Ce que « prendre le ticket » veut dire dans dispatch-lib

`dispatch_claude_pilot` (`:7357`) enchaîne `_parse_input_json` → `_validate_inputs` →
`_setup_gh_auth` → `_scrub_env` → `_set_up_worktree` → `_detect_plan_on_branch` →
`_handle_dry_run` → `_run_claude_pilot`. Les étiquettes de l'issue ne sont connues qu'à
l'intérieur de `_set_up_worktree`, par l'unique appel `gh issue view … --json
state,title,labels,body,comments` (`:2484`) qui alimente `LABELS` (`:2515`). Ce même
bloc porte **trois sorties sans dispatch** avant toute mutation :

1. issue fermée → `auto_skipped / issue_closed` (`:2506-2508`) ;
2. garde mika#2012 — plan déjà commité sur la branche → `auto_skipped /
   already_groomed` (`:2537-2549`) ;
3. `DRY_RUN` — mais celle-là ne sort qu'**après** `_set_up_worktree`, dans
   `_handle_dry_run` (`:2890`), donc le worktree est construit puis détruit.

Et la **première mutation** est juste derrière la garde #2012 : `git fetch origin main`
(`:2564`), puis la détection et le `worktree remove --force` des worktrees non
canoniques (`:2596-…`), puis le `worktree add`. C'est ce `remove --force` / `reset
--hard` sur un worktree **partagé** qui a effacé le plan d'un spawn manuel le
2026-09-19 (collision n=3, `feedback_check_engine_ready_label_tasks_before_manual_groom`).
« Avant tout travail sur la branche » (AC1) se lit donc précisément : **après les trois
sorties sans dispatch, avant `git fetch origin main`.**

### Ce que ce plan fait, et ce qu'il ne fait pas

Il ajoute à dispatch-lib le geste symétrique de `_stamp_pr_origin`, porté par l'issue
et non par la PR, avec sa fin de vie décidée (AC4). Il ne touche pas au classifier
Rust ni à ses sites d'appel : la garde elle-même est mika#2084, terminée. Il n'ajoute
**pas** de second classifier en shell — voir C-2.

---

## Requirements

- **R-1** — Quand dispatch-lib prend un ticket (`repo#N` résolu, issue ouverte, garde
  #2012 passée, pas de `DRY_RUN`), `dispatch:loop` est posée sur l'issue **avant**
  `git fetch origin main` et toute manipulation de worktree. (AC1)
- **R-2** — La pose est idempotente : un ticket portant déjà `dispatch:loop` n'est ni
  refusé ni ré-estampillé ; un seul appel `gh` au maximum. (AC2)
- **R-3** — La pose est non fatale : tout échec de `gh issue edit` (réseau, droits,
  étiquette absente sur un dépôt hors label-sync) laisse le dispatch continuer, avec une
  ligne nommée sur stderr. L'étiquette est un signal, pas une barrière. (AC2)
- **R-4** — La pose ne contourne pas la garde : si le snapshot `LABELS` porte un
  `dispatch:*` autre que `dispatch:loop`, **aucun** `--add-label dispatch:loop` n'est
  émis, et une ligne nommée le dit. L'ordre est lecture-des-étiquettes puis estampille,
  jamais l'inverse ; la lecture est **la même** que celle qui a servi au moteur, pas une
  seconde. (AC3)
- **R-5** — Le retrait est décidé et écrit : `dispatch:loop` est une **revendication
  vivante**, retirée à la sortie de dispatch-lib (chemin nominal, crash, annulation) ;
  la provenance permanente reste `origin:loop` sur la PR. (AC4)
- **R-6** — Les littéraux `dispatch:loop` passés à `--add-label` / `--remove-label`
  sont écrits en clair dans dispatch-lib (pas via variable), pour rester visibles à la
  règle L5 de `scripts/check-canonical-tokens.sh`.
- **R-7** — Une suite shell hermétique (stub `gh`, journal d'argv) pine R-1 à R-5,
  câblée dans `make test`, sur le modèle exact de `test_stamp_pr_origin.sh`.
- **R-8** — Documentation : le paragraphe « seat vocabulary » de
  `crates/mika-agent/CLAUDE.md` (`:2226`) et le doc-comment de
  `SeatVerdict::OwnedByCurrentSeat` disent qui pose l'étiquette et qui la retire.

---

## Approche / Conception

### C-1 — `_stamp_issue_seat <repo> <issue> <labels-csv>` : le geste symétrique, à côté de son modèle

Nouvelle fonction dans `skills/bundled/_shared/dispatch-lib.sh`, placée immédiatement
après `_record_pr_origin_epoch` (`:6457`) pour que les deux estampilles se lisent
ensemble. Signature et contrat calqués sur `_stamp_pr_origin` :

```bash
# _stamp_issue_seat <repo> <issue_num> <labels_csv> — claim the issue for the loop.
#
# Symmetric to _stamp_pr_origin (mika#2026), carried by the ISSUE and not the PR:
# the loop is a dispatch seat like ssc and mpc (webhook_dispatch.rs
# CURRENT_DISPATCH_SEAT), and until mika#2155 it was the only seat that never
# said so. `labels_csv` is the snapshot _set_up_worktree already fetched — the
# SAME labels the engine's seat gate read, not a second round trip (mika#2178).
#
# Three outcomes, on that snapshot:
#   another dispatch:* present  → dispatch_seat.owned_by_other, NO write (AC3)
#   dispatch:loop present       → dispatch_seat.already_owned,  NO write (AC2)
#   no dispatch:* at all        → gh issue edit --add-label dispatch:loop
#
# Returns 0 when the issue carries the label, 1 when it could not be applied —
# with a named line on stderr. Callers MUST invoke with `|| true`: the label is
# a signal for the other seats, not a barrier for this one (AC2).
_stamp_issue_seat() {
    local repo="$1" issue="$2" labels_csv="$3" seat_labels
    [ -n "$repo" ] && [ -n "$issue" ] || return 0

    seat_labels=$(printf '%s\n' "$labels_csv" | tr ',' '\n' | sed 's/^ *//;s/ *$//' \
        | tr '[:upper:]' '[:lower:]' | grep '^dispatch:' || true)

    if [ -n "$seat_labels" ]; then
        if [ "$seat_labels" = "dispatch:loop" ]; then
            echo "dispatch_seat.already_owned: ${repo}#${issue} already carries dispatch:loop; not re-stamping" >&2
            return 0
        fi
        echo "dispatch_seat.owned_by_other: ${repo}#${issue} carries '$(printf '%s' "$seat_labels" | paste -sd, -)'; refusing to stamp dispatch:loop — the engine's seat gate (mika#2084) is the authority on whether this dispatch may proceed" >&2
        return 1
    fi

    if timeout 15 gh issue edit "$issue" --repo "senara-solutions/${repo}" --add-label dispatch:loop >/dev/null 2>&1; then
        echo "dispatch_seat.stamped: ${repo}#${issue} labeled dispatch:loop" >&2
        return 0
    fi
    echo "dispatch_seat.stamp_failed: could not apply dispatch:loop to ${repo}#${issue} — other seats will not see this claim; dispatch proceeds" >&2
    return 1
}
```

Différences délibérées avec `_stamp_pr_origin`, chacune nommée :

- **Pas de `gh label create` de repli.** `origin:loop` devait exister sur quatre dépôts
  dont trois sans label-sync ; `dispatch:loop` est une étiquette de **siège**, dont le
  vocabulaire est gardé Rust↔YAML sur `mika` par #2092. Créer l'étiquette à la volée sur
  `mika-cloud`/`mika-skills`/`mika-platform` fabriquerait un siège hors garde (aucun
  `KNOWN_DISPATCH_SEATS` n'y est lu — la garde de siège du moteur lit les étiquettes
  du ticket quel que soit le dépôt, mais le lint de déclaration ne couvre que `mika`).
  Sur ces dépôts l'échec est attendu, journalisé `stamp_failed`, et le ticket dit
  explicitement « Hors périmètre : les sièges sur les autres dépôts ». Une étiquette
  qui manque là est une **ligne stderr**, pas un dispatch perdu.
- **Pas de lecture `gh … --json labels` interne.** La lecture est celle de
  `_set_up_worktree` (`:2484`), passée en argument. Deux raisons : (i) c'est la même
  photo que celle du moteur — deux lectures à deux instants recréent la fenêtre que
  mika#2178 a fermée ; (ii) la fonction devient testable sans stub de lecture, avec
  les trois populations d'étiquettes injectées directement.
- **`dispatch:loop` en littéral, pas `dispatch:${MIKA_DISPATCH_SEAT}`.** La règle L5
  (`check-canonical-tokens.sh:353-384`) ignore tout label contenant `$` ; écrire le
  littéral la laisse voir l'instruction et la confronter à `labels.yml`. C'est une
  **troisième copie** du mot « loop » (Rust, YAML, shell) — assumée : les copies
  Rust↔YAML sont gardées par `check-dispatch-seats-declared.sh`, la copie shell↔YAML par
  L5 ; la transitivité tient sans un troisième lint. Le commentaire du site le dit.

### C-2 — Pas de second classifier : la lecture à trois branches n'est PAS `classify_dispatch_seat`

`classify_dispatch_seat` porte une liste de sièges connus, un cas `empty_seat`, un cas
`multiple_seat_labels`, une casse insensible sur le préfixe et le siège. `_stamp_issue_seat`
n'en réimplémente **aucun** : elle ne répond qu'à la question « puis-je écrire
`dispatch:loop` ici sans écraser une revendication ? », et la réponse est binaire.

- `dispatch:zorglub` (siège inconnu), `dispatch:` (vide), `dispatch:ssc,dispatch:mpc`
  (deux sièges) → tous « un `dispatch:*` autre que le mien est là » → **pas d'écriture**.
  C'est le sens sûr dans les trois cas, et c'est le même sens que le moteur
  (`Unresolvable` refuse). La fonction n'a pas à savoir *pourquoi* ; elle n'écrit pas.
- `Dispatch:LOOP` → abaissé → `dispatch:loop` → idempotent. Même tolérance de casse que
  le moteur, obtenue par un `tr`, pas par une réécriture de la règle.

**Décision : dispatch-lib ne refuse PAS le dispatch sur `owned_by_other`.** Il ne pose
pas l'étiquette et le dit. Le refus appartient aux trois sites moteur, tous en amont de
dispatch-lib ; en ajouter un quatrième en shell, c'est la dérive que #2084 a conçu sa
fonction pure pour éviter (*« share one rule instead of three that drift »*,
`webhook_dispatch.rs:362-364`). La seule fenêtre où dispatch-lib verrait un
`dispatch:ssc` que le moteur a laissé passer est la fenêtre **fail-open** de #2084 D2
(étiquettes illisibles côté moteur : jeton absent, GitHub en panne) — décidée bruyante et
non silencieuse là-bas. Si cette fenêtre mord un jour (n ≥ 1 mesuré : un
`dispatch_seat.owned_by_other` dans un stderr de dispatch qui a tout de même écrit sur
la branche), le refus miroir en shell devient un ticket avec sa preuve. Pas avant.

### C-3 — Site d'appel : dans `_set_up_worktree`, après la garde #2012, avant `git fetch origin main`

```bash
        # mika#2155: claim the ticket for the loop BEFORE the first mutation
        # below — the fetch, the non-canonical worktree removal, the
        # `worktree add`. Placed after every no-dispatch exit above (closed
        # issue, redundant groom) so a dispatch that never happens never claims,
        # and skipped on a dry run for the same reason. `|| true`: AC2, the
        # label is a signal, not a barrier.
        if [ "$DRY_RUN" != "true" ] && [ "$DRY_RUN" != "1" ]; then
            _stamp_issue_seat "$REPO" "$ISSUE_NUM" "$LABELS" || true
            ISSUE_SEAT_CLAIMED=1
        fi

        # Sync main before branching to avoid stale worktrees.
        git -C "$SUB_REPO_DIR" fetch origin main 2>/dev/null || true
```

`ISSUE_SEAT_CLAIMED=1` est posé **même si l'estampille a échoué** — c'est le flag
« ce dispatch est allé au-delà des sorties sans-dispatch », pas « l'écriture a réussi ».
Le retrait (C-4) est inconditionnel dès qu'il est posé, voir plus bas pourquoi.

Les deux skills (`dev-groom` et `dev-pilot`) estampillent. Un groom écrit la branche
(plan commité, poussé par dispatch-lib) : c'est un écrivain, donc un siège. La garde
moteur s'applique déjà aux deux ; la revendication doit suivre la garde.

Mode texte libre (`PROMPT` non `repo#N`) : `REPO`/`ISSUE_NUM` vides, la fonction rend 0
au premier test, rien n'est posé — il n'y a pas d'issue à revendiquer.

### C-4 — `_release_issue_seat` : la revendication est vivante, elle meurt avec le dispatch (AC4)

**Décision : l'étiquette est retirée à la sortie de dispatch-lib, sur tous les chemins.**

Le ticket pose l'alternative : garder ou retirer. Les deux étiquettes de la lignée
répondent à deux questions différentes, et c'est ce qui tranche :

| Étiquette | Porteur | Question à laquelle elle répond | Durée de vie |
|---|---|---|---|
| `origin:loop` (#2026) | la PR | *qui a produit cet artefact ?* | permanente — la provenance ne change pas |
| `dispatch:loop` (#2155) | l'issue | *qui écrit sur cette branche en ce moment ?* | le temps du dispatch |

Garder `dispatch:loop` après la sortie ferait mentir la deuxième question : entre un
groom terminé et l'implement qui suit, entre une PR ouverte et sa revue, **personne**
n'écrit sur la branche, et un siège humain a le droit d'y aller (reprendre une PR à la
main, par exemple). Un ticket « verrouillé pour toujours par la boucle » est exactement
l'objection du ticket : *un siège qui revendique sans jamais relâcher transforme chaque
ticket traité en ticket verrouillé*. Et si un humain pose alors `dispatch:mpc` sans
retirer `dispatch:loop`, le moteur lit `multiple_seat_labels` et **refuse** — le
fail-closed est correct, mais il coûte un geste manuel par ticket à tout le monde.

Le retrait vit dans `_dispatch_lib_exit_trap` (`:1561`), **en tête**, avant la garde
`CALLBACK_SENT` — c'est le seul point par lequel passent le chemin nominal (après
`_deliver_callback`), le crash, et l'annulation (`_dispatch_lib_term_trap` → `exit 143`
→ trap EXIT) :

```bash
_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # mika#2155: the claim dies with the dispatch, on every exit. Before the
    # CALLBACK_SENT guard on purpose — the nominal path returns early there.
    _release_issue_seat "$REPO" "$ISSUE_NUM" || true
    …
```

```bash
# _release_issue_seat <repo> <issue_num> — end the loop's live claim.
#
# Unconditional once the dispatch went past its no-dispatch exits
# (ISSUE_SEAT_CLAIMED=1), whether or not THIS run's stamp succeeded: a
# dispatch:loop left by an earlier run that died without its EXIT trap
# (SIGKILL, host reboot) is stale, and this is where it heals. dispatch:loop
# is the loop's label — nothing else writes it, so nothing else is being
# undone here. Never names any other dispatch:* label. Bounded: this runs in
# the exit trap, whose job is to get RESULT back to mika-dev.
_release_issue_seat() {
    local repo="$1" issue="$2"
    [ "${ISSUE_SEAT_CLAIMED:-0}" = "1" ] || return 0
    [ -n "$repo" ] && [ -n "$issue" ] || return 0
    if timeout 15 gh issue edit "$issue" --repo "senara-solutions/${repo}" --remove-label dispatch:loop >/dev/null 2>&1; then
        echo "dispatch_seat.released: ${repo}#${issue} no longer carries dispatch:loop" >&2
        return 0
    fi
    echo "dispatch_seat.release_failed: could not remove dispatch:loop from ${repo}#${issue} — the claim outlives this dispatch until the next one on this ticket exits" >&2
    return 1
}
```

Pourquoi **inconditionnel** (et non « seulement si j'ai posé ») :

- `dispatch:loop` n'a qu'un écrivain. Un opérateur qui veut confier un ticket à la
  boucle pose `ready` ; il n'a aucune raison de poser `dispatch:loop` à la main, et la
  description de l'étiquette (`labels.yml:141`) est mise à jour pour le dire : *« posée
  et retirée par dispatch-lib le temps d'un dispatch »*. Retirer une étiquette que seul
  moi écris ne défait le geste de personne.
- Le résidu accepté est nommé : un dispatch tué sans trap (SIGKILL, reboot) laisse
  `dispatch:loop` en place jusqu'à la sortie du **prochain** dispatch sur ce ticket —
  qui la lit `already_owned` (idempotent, AC2) puis la retire à sa sortie. Un siège
  humain qui lit un `dispatch:loop` sans dispatch vivant tranche en une requête :
  `sqlite3 ~/.mika/data/mika.db "select id,status from tasks where label like
  'ready-label: %#N' and status in ('pending','in_progress')"`. C'est la sonde que
  `feedback_check_engine_ready_label_tasks_before_manual_groom` prescrit déjà.
- Gater sur « j'ai posé » laisserait le résidu **pour toujours** : le second dispatch
  lirait `already_owned`, ne poserait pas, et ne retirerait donc pas.

Ce que le retrait **ne touche jamais** : `dispatch:ssc`, `dispatch:mpc`, `ready`, ou
toute autre étiquette. L'appel nomme `dispatch:loop` et rien d'autre.

**Course examinée et bénigne.** Sur `dev-groom`, `_deliver_callback` déclenche le tour
de rappel de mika-dev, qui peut re-poser `ready` → `ready_label_handler` lit les
étiquettes **avant** que le trap EXIT ait retiré `dispatch:loop`. Verdict :
`OwnedByCurrentSeat` → passe. Après le retrait : `NoSeatLabel` → passe. Les deux
ordres donnent le même résultat, parce que les deux étiquettes appartiennent au même
siège.

### C-5 — Tests : `skills/bundled/_shared/tests/test_stamp_issue_seat.sh`

Copie de structure de `test_stamp_pr_origin.sh` : `source` de dispatch-lib (aucun code
impératif au niveau module, audit déjà fait là), stub `gh` sur `PATH` qui journalise son
argv dans `$GH_LOG`, `GH_MODE` ∈ {`ok`, `always-fails`}. Cas :

| # | Entrée | Attendu | Pine |
|---|---|---|---|
| T1 | `labels="bug,ready"`, `ok` | un `issue edit N --repo senara-solutions/mika --add-label dispatch:loop` ; rc 0 ; stderr `dispatch_seat.stamped` | R-1 |
| T2 | `labels="ready,dispatch:loop"` | **aucun** `issue edit` ; rc 0 ; stderr `already_owned` | R-2 (AC2) |
| T3 | `labels="ready,Dispatch:LOOP"` | idem T2 (casse) | R-2 |
| T4 | `labels="ready,dispatch:ssc"` | **aucun** `--add-label dispatch:loop` ; rc 1 ; stderr `owned_by_other` | R-4 (AC3) |
| T5 | `labels="dispatch:mpc,ready"` | idem T4 | R-4 |
| T6 | `labels="dispatch:ssc,dispatch:loop"` | idem T4 — deux sièges, pas d'écriture | R-4 / C-2 |
| T7 | `labels="dispatch:zorglub"` | idem T4 — siège inconnu, pas d'écriture | C-2 |
| T8 | `labels="bug"`, `always-fails` | un `issue edit` tenté ; rc 1 ; stderr `stamp_failed` ; **aucun** `label create` | R-3 (AC2), C-1 |
| T9 | `repo=""` ou `issue=""` | rc 0, aucun appel `gh` | mode texte libre |
| T10 | `_release_issue_seat` avec `ISSUE_SEAT_CLAIMED=1`, `ok` | un `issue edit N … --remove-label dispatch:loop`, **et aucun** `--remove-label` d'autre valeur | R-5 |
| T11 | `_release_issue_seat` avec `ISSUE_SEAT_CLAIMED` non posé | aucun appel `gh` | C-4 |
| T12 | `_release_issue_seat`, `always-fails` | rc 1 ; stderr `release_failed` ; l'appelant `|| true` continue | R-3 |

T4–T7 sont les tests **négatifs** exigés par la doctrine du 2026-09-09
(`project_c2_crossing_exposed_close_queue_multiagent_test_gap`) : le gate exige des
tests qui prouvent qu'une écriture **n'a pas** eu lieu, pas seulement qu'une autre a eu
lieu. L'assertion est `assert_not_contains "$(cat "$GH_LOG")" "--add-label dispatch:loop"`
— **et** un contrôle positif dans la même suite (T1) qui prouve que le journal capture
bien cette chaîne quand elle est émise, sinon T4 est vert par vacuité
(`feedback_a_probe_needs_both_controls_in_the_same_call`).

Câblage : une ligne `@bash skills/bundled/_shared/tests/test_stamp_issue_seat.sh` dans
la cible `test` du `Makefile` (`:132`, juste sous `test_stamp_pr_origin.sh`) et une
cible dédiée `test-stamp-issue-seat`.

**Test d'ordre (AC1 au niveau du site, pas seulement de la fonction).** Une assertion
statique dans la même suite : le numéro de ligne du premier `_stamp_issue_seat` dans
`_set_up_worktree` est **strictement inférieur** à celui du premier `git -C
"$SUB_REPO_DIR" fetch origin main` et **strictement supérieur** à celui de
`dispatch_gate_groom_refused`. Même patron que la section « Structural » de
`test_rescue_closes_guard.sh:195-205` (grep sur dispatch-lib) et D-3 de mika#2446 (position épinglée sans déplacer de code de production). Un
refactor qui inverse l'ordre rougit.

### C-6 — Lints existants, et ce qu'ils voient

- **L5** (`check-canonical-tokens.sh`) : `--add-label dispatch:loop` et
  `--remove-label dispatch:loop` dans dispatch-lib sont des instructions d'écriture ;
  `dispatch:loop` est déclarée (`labels.yml:139`) → vert. Contrôle négatif du contrat de
  vérification : renommer temporairement le littéral en `dispatch:lopp` doit rougir L5.
- **`check-dispatch-seats-declared.sh`** : ne lit pas dispatch-lib ; inchangé, vert.
- **`test-dispatch-symmetry.sh`** : compare les deux handlers `run.sh` (dev-pilot /
  dev-groom), pas dispatch-lib ; rien ici ne les touche → inchangé, vert.

---

## Fire-Disposition

Ce plan livre **deux détecteurs** dont la pose sur l'existant est à nommer.

### D-1 — Test d'ordre du site d'appel (C-5, Phase 2)

**Option (a) — assertion au présent, zéro exception.** Le site est écrit dans le même
commit que le test, à la position que le test épingle ; il atterrit vert. S'il tire un
jour, la résolution est de **remettre le site à sa place**, jamais d'ajuster les bornes
du test : la borne basse est la garde #2012 (un dispatch refusé ne revendique pas), la
borne haute est la première mutation (une revendication après la mutation est un
mensonge d'une seconde). Les deux bornes sont des faits du code, pas des choix.

### D-2 — Règle L5 sur les deux nouveaux littéraux (C-6)

**Option (a) — déjà verte au présent.** `dispatch:loop` est déclarée depuis #2092. La
seule façon de la faire tirer est un renommage du siège — et alors elle tire **avec**
`check-dispatch-seats-declared.sh` (côté Rust) dans le même commit : c'est le
comportement voulu, une vocabulaire écrit trois fois doit bouger trois fois.

### Ce que cette section ne couvre pas

Les lignes stderr `dispatch_seat.*` ne sont pas des détecteurs : personne ne les compte
automatiquement. Elles vont dans le fichier stderr du handler (remonté dans `RESULT`
uniquement sur crash) et dans la trace fd 9. C'est la même visibilité que `pr_origin.*`,
délibérément — ajouter une ligne au `RESULT` toucherait le contrat d'enveloppe lu par
mika-dev (`^PR: `, `^NO_PR: `, `Outcome:`) pour un signal dont le lecteur canonique est
la **timeline GitHub de l'issue**, pas le rappel.

---

## Phases d'implémentation

### Phase 1 — Les deux fonctions et leur site (C-1, C-3, C-4 ; R-1 à R-6)

1. `_stamp_issue_seat` et `_release_issue_seat` sous `_record_pr_origin_epoch`.
2. Appel dans `_set_up_worktree` entre la fin du bloc `if [ "$SKILL" = "dev-groom" ]`
   (garde #2012) et `# Sync main before branching`, gardé `DRY_RUN`, avec
   `ISSUE_SEAT_CLAIMED=1`.
3. `_release_issue_seat "$REPO" "$ISSUE_NUM" || true` en première instruction utile
   de `_dispatch_lib_exit_trap`, après `_EXIT_CODE=$?`.
4. `ISSUE_SEAT_CLAIMED=0` initialisé à côté de `CALLBACK_SENT=0` dans
   `dispatch_claude_pilot` (`:7406`) — le trap le lit, il doit exister même si
   `_set_up_worktree` n'a jamais été atteint.

### Phase 2 — La suite (C-5 ; R-7)

5. `skills/bundled/_shared/tests/test_stamp_issue_seat.sh`, T1–T12 + test d'ordre.
6. `Makefile` : ligne dans `test`, cible `test-stamp-issue-seat`.

### Phase 3 — Vocabulaire et documentation (R-8)

7. `.github/labels.yml:141` — description de `dispatch:loop` : *« Routage : ce ticket est
   pris par la boucle autonome (mika-dev). Posée et retirée par dispatch-lib le temps
   d'un dispatch (mika#2155). Un seul dispatcher par ticket. »* La couleur ne change pas
   (label-sync ne touche que la description).
8. `crates/mika-agent/src/webhook_dispatch.rs` — doc-comment de
   `SeatVerdict::OwnedByCurrentSeat` : une phrase, *« Stamped by dispatch-lib when it
   takes the ticket and released when it exits (mika#2155); between two dispatches an
   unclaimed ticket reads `NoSeatLabel`. »* Aucun code Rust ne change ; `cargo test`
   reste le contrôle.
9. `crates/mika-agent/CLAUDE.md:2226` — ajouter au paragraphe « seat vocabulary » qui
   pose et retire `dispatch:loop`, et la règle des deux durées de vie (`origin:loop`
   permanent sur la PR, `dispatch:loop` vivant sur l'issue).
10. `docs/solutions/` — une entrée courte si `/ce:compound` juge la distinction « deux
    étiquettes, deux durées de vie » non évidente depuis le code (elle l'est
    probablement depuis le tableau de C-4 ; ne pas forcer).

---

## Contrat de vérification

- `bash skills/bundled/_shared/tests/test_stamp_issue_seat.sh` — T1–T12 + ordre, tous
  verts.
- `bash skills/bundled/_shared/tests/test_stamp_pr_origin.sh` — inchangé, vert (les deux
  fonctions cohabitent dans le même fichier sourcé).
- `bash skills/bundled/_shared/test-dispatch-lib.sh` — la suite historique reste verte.
- `bash scripts/check-canonical-tokens.sh` — L5 vert sur les deux nouveaux littéraux.
- `bash scripts/check-dispatch-seats-declared.sh` — inchangé, vert.
- `make lint && make fmt && make test`.
- **Contrôles négatifs obligatoires**, consignés dans le corps de la PR :
  1. Commenter l'appel `_stamp_issue_seat` dans `_set_up_worktree` → le test d'ordre
     rougit (il ne trouve plus le site).
  2. Déplacer l'appel sous `git fetch origin main` → le test d'ordre rougit.
  3. Renommer le littéral en `dispatch:lopp` → L5 rougit.
  4. Retirer le `grep '^dispatch:'` de `_stamp_issue_seat` (écrire sans regarder) → T4,
     T5, T6, T7 rougissent — **les quatre**, un par population (terme par terme,
     `feedback_red_before_control_is_term_by_term`).
  Un contrôle qui reste vert est une halte, pas un succès.

### Sonde post-déploiement, et sa halte

**Sonde — la timeline dit ce que le plan promet.** Sur le premier dispatch réel après
`make deploy` (ticket `ready` quelconque pris par le feeder) :

```bash
gh api repos/senara-solutions/mika/issues/<N>/timeline --jq \
  '.[] | select(.event=="labeled" or .event=="unlabeled") | select(.label.name=="dispatch:loop") | "\(.event) \(.created_at)"'
git -C mika log --format='%cI %s' origin/<branch> | tail -1
```

Attendu : un `labeled` **antérieur** au premier commit de la branche (AC1), un
`unlabeled` postérieur au rappel `task-complete` (AC4). **Halte** si le `labeled` est
absent alors que `dispatch_seat.stamped` figure dans la trace (l'écriture GitHub a
réussi sans événement → chercher label-sync, cf. #2092 moitié un), ou si `unlabeled`
manque après un dispatch terminé proprement (trap EXIT non atteint → ticket, avec la
trace). Mesurer **une période complète** — un dispatch de bout en bout — avant de
conclure dans un sens ou l'autre (`feedback_never_conclude_inside_the_mechanism_period`).

`mika skills --agent mika-dev update` peut être un no-op sur une bibliothèque déjà à
jour : vérifier par `diff` que le dispatch-lib.sh **résolu en production** porte
`_stamp_issue_seat` avant de lire la sonde
(`feedback_mika_skills_update_noop_verify_prompt_by_diff`).

---

## Definition of Done

- [ ] `_stamp_issue_seat` et `_release_issue_seat` existent dans dispatch-lib, à côté de
      `_stamp_pr_origin`, avec leur contrat en en-tête.
- [ ] Le site d'appel est entre la garde #2012 et `git fetch origin main`, gardé
      `DRY_RUN` ; le test d'ordre (**D-1**) l'épingle et est vert.
- [ ] Le retrait est en tête de `_dispatch_lib_exit_trap`, avant la garde
      `CALLBACK_SENT` ; `ISSUE_SEAT_CLAIMED` initialisé dans `dispatch_claude_pilot`.
- [ ] `test_stamp_issue_seat.sh` : T1–T12 verts, câblé dans `make test` et en cible
      dédiée ; T4–T7 sont des assertions d'**absence** avec leur contrôle positif T1.
- [ ] Les quatre contrôles négatifs ont été exécutés et rougissent ; résultat dans la
      PR.
- [ ] `labels.yml` : description de `dispatch:loop` mise à jour ; L5 (**D-2**) vert.
- [ ] Doc-comment `OwnedByCurrentSeat` et `crates/mika-agent/CLAUDE.md` § seat
      vocabulary à jour ; `cargo test` vert (aucun code Rust modifié).
- [ ] `make lint && make fmt && make test` verts.
- [ ] La section § *Fire-Disposition* couvre les deux détecteurs livrés.

---

## Acceptance criteria

Reprises du ticket, avec le lieu du plan qui les tient.

- [ ] **AC1** — Quand dispatch-lib prend un ticket, `dispatch:loop` est posée sur l'issue
  avant tout travail sur la branche. **Tenu par** C-3 (site avant `git fetch origin
  main` et toute mutation de worktree), épinglé par le test d'ordre (C-5, D-1).
  **Vérifiable** par la sonde post-déploiement : `labeled dispatch:loop` horodaté avant
  le premier commit de la branche.
- [ ] **AC2** — Pose idempotente et non fatale. **Tenu par** C-1 (branche `already_owned`,
  aucune écriture ; `|| true` au site ; pas de repli `label create`), T2/T3/T8/T9.
  `OwnedByCurrentSeat` reste un cas de passage côté moteur — inchangé, et c'est ce qui
  rend la course de C-4 bénigne.
- [ ] **AC3** — La pose ne contourne pas la garde : classifier puis estampiller. **Tenu
  par** C-2/C-1 (lecture du **même** snapshot que le moteur ; toute étiquette
  `dispatch:*` étrangère → aucune écriture) et par les tests négatifs T4–T7, qui posent
  `dispatch:ssc`/`dispatch:mpc` et affirment qu'aucun `--add-label dispatch:loop`
  n'apparaît dans l'argv de `gh`. Le refus du dispatch lui-même reste au moteur —
  décision écrite en C-2, avec la condition de réveil d'un éventuel refus miroir.
- [ ] **AC4** — Le retrait est décidé et documenté. **Décision : retrait à la sortie, sur
  tous les chemins, inconditionnel** (C-4), avec le tableau des deux durées de vie, le
  résidu accepté (dispatch tué sans trap → guéri au dispatch suivant) et la sonde qui
  tranche un résidu (`tasks` dans `mika.db`). La garde continue de laisser passer
  `OwnedByCurrentSeat` pendant la fenêtre où l'étiquette est présente — inchangé.

---

## Hors périmètre

- La garde de siège moteur et ses trois sites (mika#2084) — inchangés.
- Le vocabulaire et le lint Rust↔YAML (mika#2092) — inchangés ; seule la
  **description** de `dispatch:loop` bouge.
- Les sièges sur `mika-cloud`, `mika-skills`, `mika-platform` : l'estampille y échoue
  `stamp_failed` par construction (étiquette non déclarée, pas de repli `label create`),
  et le dispatch continue. Le ticket les exclut.
- Un refus miroir en shell sur `owned_by_other` — dormant, condition de réveil en C-2.
- Une ligne `Seat:` dans l'enveloppe `RESULT` — non, voir § Fire-Disposition, « ce que
  cette section ne couvre pas ».

---

## Revision history

- 2026-09-21 — v1, /ce:plan via /mika-groom-ticket (orchestrateur MPC). mika-arch première
  passe : **Disposition: READY**, aucun finding bloquant (session `a3a926e5-f4d5-4fec-86c8-f869221f90e1`).

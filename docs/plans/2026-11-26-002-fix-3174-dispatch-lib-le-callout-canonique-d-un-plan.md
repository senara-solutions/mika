# mika#3174 — le court-circuit d'idempotence doit regarder QUEL plan le corps nomme

Le groom du vol 2 de mika#3118 a convergé (première passe READY, seconde
GROOMED), est sorti en `Outcome: PLAN_GROOMED` sur
`docs/plans/2026-11-26-001-fix-3118-task-engine-l-auto-fire-du-dev-pilot-vol2-plan.md`,
et le corps du ticket n'a reçu **aucun** callout. La seule édition suivante est
la réécriture « Plan antérieur » de l'auto-pull, quatre minutes plus tard.

La cause est un test d'idempotence qui répond à une question plus faible que
celle qu'il croit poser. `_write_canonical_callout` demande « le corps
porte-t-il les trois signaux de la porte de dispatch ? » et, quand la réponse
est oui, conclut « donc mon callout est déjà là ». Les trois signaux étaient
bien là — mais ils étaient ceux du **vol 1** : même branche, plan
`2026-11-22-002`, marqueur `second-pass (GROOMED)`. Aucun des trois ne nomme un
plan en particulier : `has_plan` compte les occurrences de la sous-chaîne
`docs/plans/`, pas du plan courant.

## Ce qui est déjà en place, et ne doit pas être refait

Quatre choses sont justes et le vol ne les touche pas.

1. **`GROOM_CALLOUT_PLAN` est affecté AVANT le court-circuit** (mika#2981,
   `dispatch-lib.sh:11827`). C'est pour cela que la preuve de groom du vol 2
   nommait le bon plan alors que le corps divergeait : le défaut est strictement
   dans le corps, pas dans la preuve. L'ordre reste inchangé — un corps déjà
   estampillé par le plan courant est un succès du producteur, et sa preuve doit
   porter le plan tout autant.

2. **Le chemin REPLACE existe et fonctionne** (`dispatch-lib.sh:11872–11898`,
   mika#2012 U3). Le strip awk est borné au PRÉAMBULE, s'arrête à la première
   ligne qui n'est ni un callout ni blanche, et absorbe les lignes blanches
   entre blocs empilés. Il a été écrit exactement pour « une ligne Plan périmée
   venue d'une branche antérieure » — le cas de ce ticket. **Rien à construire
   côté écriture : il suffit que le flux y arrive.** La borne au préambule est
   load-bearing (un `grep -v` sur tout le corps supprimerait la documentation du
   format citée plus bas dans un corps — celui de mika#2012 en porte) : elle
   n'est pas élargie.

3. **Les trois signaux `has_branch` / `has_plan` / `has_verdict` sont en
   lockstep avec `executor.rs::check_grooming_markers`** (Pin B). Le commentaire
   de `dispatch-lib.sh:11842–11853` dit pourquoi la dérive y est un défaut et
   non un détail, et trois assertions de `test-dispatch-lib.sh` verrouillent les
   motifs de verdict. Les trois restent **octet pour octet identiques** : ce
   ticket n'ajoute pas un quatrième signal de porte, il ajoute un prédicat
   d'**identité du plan**, qui n'a pas de contrepartie côté Rust parce que la
   porte Rust n'a pas de « plan courant » à comparer.

4. **Le lecteur du callout `Plan` est unique et il est en Rust**
   (`crates/mika-agent/src/plan_callout.rs::PLAN_CALLOUT_RE`, mika#2194 puis
   mika#2608). Le bash ne le lit qu'à travers `_extract_plan_path` →
   `mika plan-callout --body-file <f>`. `scripts/canonical-tokens.tsv` déclare
   pour `> - **Plan:**` **exactement un** site, et le paragraphe mika#2608 a
   fermé cet inventaire à une ligne.

## Le défaut qui reste ouvert

```bash
has_plan=$(printf '%s' "$current_body" | grep -cF 'docs/plans/' || true)
…
if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
    echo "write_canonical_callout: dispatch-gate signals already present in …" >&2
    return 0
fi
```

La conjonction est une bonne réponse à « la porte de dispatch passerait-elle
sur ce corps ? » et une mauvaise réponse à « ai-je déjà écrit CE callout ? ».
Les deux questions coïncident tant qu'un ticket n'a qu'un vol ; elles divergent
dès le second, et la divergence est silencieuse dans la direction la plus
coûteuse : le producteur se tait en annonçant un succès idempotent.

## La décision de forme : on DÉLÈGUE la lecture, on n'ajoute pas un grep

Le quatrième prédicat doit répondre : *le callout présent dans le corps nomme-t-il
`plan_relpath` ?* Il y a deux façons de l'obtenir, et une seule est permise ici.

- **Refusée — un `grep` sur `> - **Plan:** \`$plan_relpath\`` dans
  `dispatch-lib.sh`.** Ce serait créer un **second site de correspondance** d'un
  token dont mika#2608 a délibérément fermé l'inventaire à un, et ré-introduire
  en bash la seconde implémentation PCRE que mika#2194 a retirée. Le lint des
  jetons canoniques est bidirectionnel : il refuserait le site non déclaré, et
  le déclarer serait écrire dans l'inventaire la duplication que l'inventaire
  existe pour interdire.

- **Retenue — `_extract_plan_path "$current_body"`.** La fonction existe
  (`dispatch-lib.sh:14060–14157`), passe le corps par **fichier** (jamais argv —
  un corps porte des retours à la ligne, des backticks et des `$`), et rend un
  chemin déjà normalisé relatif à `$WORKTREE_DIR` — la même forme que
  `plan_relpath`, donc la comparaison est une égalité de chaînes, sans
  renormalisation locale. Son en-tête note qu'un fan-in sur un lecteur délégué
  ne constitue pas une violation de C2 : quatre appelants d'un lecteur unique
  valent mieux qu'un appelant d'un second lecteur.

### Les trois codes de `_extract_plan_path`, et la direction du fail-safe

| code | sens | décision |
|---|---|---|
| `0` | un callout a été lu, chemin sur stdout | court-circuit **si et seulement si** le chemin égale `plan_relpath` |
| `1` | aucun callout lisible dans le corps | **on écrit** |
| `≥2` | « je n'ai pas pu regarder » (refus nommé sur stderr) | **on écrit** |

Les deux dernières lignes vont dans le même sens, et c'est le choix structurant
du vol. Écrire est **idempotent** : le chemin REPLACE strippe le préambule et
repose un bloc unique, donc une écriture de trop coûte une édition d'issue et
produit un corps correct. Un court-circuit de trop est **exactement le défaut
mesuré** : un corps qui garde le callout d'un autre vol, et un dispatch que la
porte de provenance laisse partir sur la mauvaise preuve. Sur un prédicat dont
les deux erreurs ne coûtent pas le même prix, le fail-safe va du côté de
l'écriture.

Cas particulier à ne pas manquer : `rc = 0` avec trois signaux présents mais
chemin **différent** — c'est AC1, et c'est la seule branche qui change de
comportement.

## Ce que livre ce vol

### 1. Le quatrième prédicat, dans `_write_canonical_callout`

Dans `skills/bundled/_shared/dispatch-lib.sh`, entre le calcul de `has_verdict`
(`:11854`) et la conjonction (`:11856`) :

- lire le plan que le corps nomme, **sans faire échouer la fonction** quand la
  lecture n'aboutit pas :

  ```bash
  local body_plan body_plan_rc=0
  body_plan=$(_extract_plan_path "$current_body") || body_plan_rc=$?
  ```

  `_extract_plan_path` écrit déjà son propre diagnostic bruyant sur `rc ≥ 2` ;
  on ne le double pas, on ne l'avale pas.

- étendre la conjonction d'un terme, en laissant les trois signaux intacts :

  ```bash
  if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ] \
     && [ "$body_plan_rc" -eq 0 ] && [ "$body_plan" = "$plan_relpath" ]; then
      echo "write_canonical_callout: dispatch-gate signals already present in $REPO#$ISSUE_NUM body — skipping (idempotent)" >&2
      return 0
  fi
  ```

  La ligne de log du court-circuit est conservée **mot pour mot** : elle est
  lue dans les journaux de dispatch et une assertion de la suite la cite.

- nommer le cas qui vient d'être ouvert, juste avant le chemin REPLACE, avec un
  motif greppable qui porte **les deux** plans (celui que le corps nomme et
  celui que ce groom a validé) — sans quoi l'opérateur voit une écriture sans
  savoir ce qu'elle remplace :

  ```bash
  if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
      echo "write_canonical_callout_stale_plan: repo=${REPO} issue=${ISSUE_NUM} body_plan=${body_plan:-<unreadable>} rc=${body_plan_rc} current_plan=${plan_relpath} — complete callout names another plan, replacing (mika#3174)" >&2
  fi
  ```

  Ce bloc est **purement diagnostique** : il ne décide rien, il ne rend rien, il
  se contente de dire pourquoi on ne s'est pas arrêté. Il est placé après la
  conjonction d'idempotence, donc il ne tire que sur un corps complet qui nomme
  un autre plan (ou illisible) — jamais sur un corps partiel, pour lequel le
  commentaire REPLACE existant dit déjà le motif.

- un commentaire qui porte la raison : pourquoi le test à trois signaux est la
  bonne question pour la porte Rust et la mauvaise pour le producteur ; pourquoi
  le prédicat délègue au lieu de grepper ; pourquoi `rc ≥ 2` écrit.

`write_canonical_callout_stale_plan` est un diagnostic **stderr** : il n'a rien
à déclarer dans `scripts/canonical-tokens.tsv`. Vérifié sur ses voisins —
`write_canonical_callout_unknown_stage`, `write_canonical_callout_plan_missing`
et `write_canonical_callout_plan_outside_worktree` en sont tous absents.

### 2. L'en-tête de `_extract_plan_path` dit son quatrième appelant

`dispatch-lib.sh:14060–14157` énumère ses appelants et affirme qu'« un dispatch
dev-groom paie au plus **TROIS** démarrages de `mika` » (un cache est
explicitement refusé). Le vol en ajoute un sur le chemin GROOMED : le plafond
documenté devient **QUATRE**. L'en-tête est mis à jour pour nommer le nouvel
appelant et le nouveau plafond — un inventaire qui ne compte pas son dernier
entrant mentirait avec l'autorité d'un inventaire (mika#2608).

Le coût est une borne, pas une mesure : la lecture n'a lieu que sur l'appel de
`_write_canonical_callout`, donc une fois par groom convergent, et le chemin de
`mika plan-callout` est le plus court du binaire (ni `dotenv`, ni `Settings`, ni
`home`, ni base, ni réseau). Le coût croît avec les **liens** vers le lecteur
unique, jamais avec le nombre d'appelants d'un seul lecteur.

### 3. Les deux cas, tenus par la suite

Voir § Tests.

## Tests

Les deux critères atterrissent dans une suite autonome,
**`skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh`**.

> **Révision du vol 2 : la suite est sortie de l'agrégat.** Ce plan visait
> d'abord `skills/bundled/_shared/test-dispatch-lib.sh`, au motif que l'AC3
> nommait un `…/_shared/tests/test-dispatch-lib.sh` qui n'existe pas. Mesuré
> sur la PR #3177 : l'ajout du bloc portait l'agrégat de 1 041 079 à
> **1 052 961 octets**, au-delà du plafond `LARGE_FILE_LIMIT` (1 048 576) de
> `scripts/check-secrets.sh`, et le job `Secret Scan` rougissait — sur une
> taille, pas sur un secret. Le plafond n'est pas touché, ni
> `LARGE_FILE_ALLOWLIST` : le bloc part dans une suite par sujet, sur le modèle
> de `tests/test_find_issue_plan.sh`, ce qui est exactement ce que `_shared/tests/`
> existe pour porter. `test-dispatch-lib.sh` redevient **identique octet pour
> octet** à celui d'`origin/main`. La nouvelle suite est câblée dans `make test`
> **et** dans `make test-dispatch-lib` — la cible que la CI appelle — donc rien
> à déclarer dans `scripts/suite-wiring.tsv` (`check-suite-wiring.sh` la voit
> couverte : `unwired=0`). Les helpers que le bloc tenait de l'agrégat
> (`assert_eq`, `assert_contains`, `assert_not_contains`, les exports
> `GIT_CONFIG_*` d'hermeticité mika#1772, `WRITER_SRC`) sont **recopiés** dans la
> nouvelle suite sans être retirés de l'ancienne ; `mika_bin_or_skip` reste
> **sourcé** depuis `tests/lib-mika-bin.sh`, jamais recopié (mika#2194).

La couverture existante de `_write_canonical_callout` (≈ `:1211–1309`) est de la
**forme de source** (`WRITER_SRC=$(declare -f _write_canonical_callout)`) plus
des runs de contrat défensif. Elle ne peut pas voir ce défaut : le court-circuit
y est bien présent, c'est sa *question* qui est trop faible. Les deux nouveaux
cas sont donc **comportementaux** — on exécute la fonction et on lit le corps
qu'elle a produit.

### La sonde

Modelée sur `_t2492_recovery_probe` (`test-dispatch-lib.sh:10071`), qui est
l'idiome de la maison pour « exécuter dispatch-lib sur un vrai dépôt et lire ce
qu'il produit » :

- `base_dir=$(mktemp -d)`, `wt="$base_dir/worktree"`, `git init -q "$wt"`, un
  fichier et un commit (le writer appelle `git -C "$WORKTREE_DIR" rev-parse
  --short HEAD` : sans commit, `head_sha` serait vide et le bloc écrit
  incomparable) ;
- un plan sous `$wt/docs/plans/2026-11-26-002-fix-3174-…-plan.md` rempli à plus
  de 500 octets, pour que `_find_issue_plan` le résolve (c'est le seuil de son
  `-size +500c`) ; un second plan `2026-11-22-002-…` n'est **pas** nécessaire —
  seul son nom compte, dans le corps ;
- dans un sous-shell : `source "$DISPATCH_LIB"`, puis `gh` **mocké en fonction
  shell** (`gh` n'est jamais invoqué via `command`, donc la fonction l'emporte) —
  `issue view` rend le corps du fixture, `issue edit` lit l'argument qui suit
  `--body-file` et recopie ce fichier vers un chemin sous `$base_dir` que la
  sonde pourra relire, puis rend 0 ;
- `WORKTREE_DIR`, `REPO=mika`, `ISSUE_NUM=3174`, `BRANCH`, puis
  `_write_canonical_callout ready-to-groomed t3174` ;
- la sonde imprime trois choses séparables : le code de retour, le corps écrit
  (ou un marqueur `NO_EDIT` si `issue edit` n'a jamais été appelé), et le stderr
  capturé.

`mika` n'est **pas** mocké : `lib-mika-bin.sh` est déjà sourcée en tête de la
suite (`:66`) et augmente le `PATH` vers le binaire de `target/`, et le bloc
s'arme par `mika_bin_or_skip "mika#3174 identité du plan dans le court-circuit"`.
C'est le vrai lecteur canonique qui répond, ce qui est tout l'intérêt de la
délégation — un mock de `mika` mesurerait le mock. Le SKIP est bruyant hors CI
et compté en ÉCHEC sous `CI`/`GITHUB_ACTIONS` : le bloc ne peut pas se lire
comme un vert sans avoir rien mesuré.

### T1 (AC1) — un callout complet nommant un AUTRE plan fait écrire, et remplace

Corps du fixture : le callout **complet** du vol 1, dans la forme réelle
qu'émet le writer, suivi du corps du ticket.

```
> - **Branch:** `fix/3174/probe`
> - **Plan:** `docs/plans/2026-11-22-002-fix-3174-vol1-plan.md` (committed on branch @ `deadbee`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: vol1

## Constat

corps du ticket, intact.
```

Assertions :

1. `gh issue edit` **a été appelé** (le marqueur `NO_EDIT` est absent) — c'est le
   cœur du rouge-avant ;
2. le corps écrit porte `> - **Plan:** \`docs/plans/2026-11-26-002-…-plan.md\`` ;
3. il ne porte **plus** `2026-11-22-002` — le bloc du vol précédent est
   remplacé, pas empilé ;
4. `grep -c '^> - \*\*Plan:\*\*'` sur le corps écrit rend **exactement 1** (la
   non-empilation, mesurée plutôt que supposée — c'est la propriété que
   mika#2012 U3 a payée) ;
5. idem pour `^> - \*\*Branch:\*\*` et `^> - \*\*Grooming history:\*\*` ;
6. `## Constat` et « corps du ticket, intact. » survivent — le strip est resté
   borné au préambule ;
7. le stderr porte `write_canonical_callout_stale_plan` avec les deux plans ;
8. le retour est `0`.

**Rouge-avant.** Contre le code actuel, l'assertion 1 rougit : les trois signaux
sont présents, la fonction rend `0` sans écrire, et `NO_EDIT` apparaît. Le rouge
est obtenu en annulant le seul geste du vol — retirer les deux termes ajoutés à
la conjonction — et il doit être **constaté avant** le correctif, dans cet ordre
de commit, pas reconstitué après.

### T2 (AC2) — le callout du plan COURANT laisse le corps inchangé

Même sonde, même fixture à un détail près : la ligne `**Plan:**` du corps nomme
`docs/plans/2026-11-26-002-…-plan.md`, c'est-à-dire le plan que
`_find_issue_plan` va résoudre.

Assertions :

1. `gh issue edit` n'a **pas** été appelé (`NO_EDIT` présent) ;
2. le stderr porte, mot pour mot,
   `dispatch-gate signals already present in mika#3174 body — skipping (idempotent)` ;
3. le stderr ne porte **pas** `write_canonical_callout_stale_plan` ;
4. le retour est `0`.

T2 est le contrôle négatif de T1 : sans lui, « le prédicat discrimine » est
indistinguable de « le court-circuit ne tire plus jamais », qui est la
régression symétrique — une édition d'issue à chaque passe, et le risque
d'empilement que le chemin REPLACE est le seul à contenir.

### T3 — `GROOM_CALLOUT_PLAN` nomme le plan courant dans les DEUX cas

Sur les deux fixtures de T1 et T2, la sonde imprime aussi
`$GROOM_CALLOUT_PLAN` et la suite asserte qu'il vaut le plan courant. C'est
l'invariant mika#2981 : l'affectation est avant le court-circuit, donc un corps
déjà estampillé produit la même preuve qu'un corps réécrit. Rien dans ce vol ne
doit le déplacer, et un test le dit.

### T4 — les signaux de la porte sont restés en lockstep

Les assertions de forme de source existantes doivent rester vertes **sans
retouche** : `assert_contains "writer reuses Pin B has_plan signal" 'has_plan='
"$WRITER_SRC"` et les trois assertions de motif de verdict. Une seule
assertion de forme est ajoutée : le court-circuit cite `_extract_plan_path`
plutôt qu'un motif `> - **Plan:**` littéral — c'est la décision de § *La
décision de forme*, et un futur refactor qui la renverserait doit rougir ici
avant de rougir au lint des jetons canoniques.

## Ce que ce vol ne corrige PAS

- **La porte Rust `check_grooming_markers`.** Elle ne connaît pas de « plan
  courant » : son travail est de dire si le ticket est groomé du tout. La
  provenance par plan y est déjà traitée par mika#2287/mika#2981 via
  `GROOM_CALLOUT_PLAN`. Aucun quatrième signal n'y est ajouté.
- **La politique de fences du lecteur** (`FenceHandling::Keep`,
  `crates/mika-cli/src/commands/plan_callout.rs`). C'est un arbitrage tranché le
  2026-10-01 avec sa condition de réveil dans `docs/dormeurs.md`. Le callout du
  préambule n'est jamais dans un bloc clôturé, donc ce vol ne le rencontre pas,
  et ne le relitige pas.
- **Le corps de mika#3118 lui-même.** Ce ticket corrige le producteur. Le corps
  du vol 2 de mika#3118 a depuis été réécrit par l'auto-pull ; un re-groom le
  réparera par le chemin corrigé. Aucune édition d'issue à la main n'est au
  périmètre.
- **Le cache de `mika plan-callout`.** Explicitement refusé par l'en-tête de
  `_extract_plan_path`, et le passage de trois à quatre démarrages ne change pas
  l'arbitrage : un cache ré-introduirait un état à invalider sur le chemin le
  plus court du binaire.

## Ordre de commit

1. **Le rouge-avant.** La sonde et T1 seuls, contre `_write_canonical_callout`
   inchangé. T1 rougit sur l'assertion 1 (`NO_EDIT` présent). Constater, ne pas
   commiter ce rouge.
2. **Le correctif** : le quatrième prédicat, le diagnostic
   `write_canonical_callout_stale_plan`, le commentaire de raison. T1 passe au
   vert.
3. **T2, T3, T4** et la mise à jour de l'en-tête de `_extract_plan_path`.
4. `make test-dispatch-lib`, puis `make test`, `make verify-bundled-skills`,
   `cargo clippy`, `cargo fmt --check`, et
   `bash scripts/verify-pipeline.sh origin/main`.

Les étapes 2 et 3 peuvent partager un commit ; l'étape 1 ne doit pas être
fusionnée avec l'étape 2, sinon le rouge-avant n'est plus attesté par rien.

## Risques

- **Le binaire `mika` absent en local** fait skipper le bloc. C'est le contrat
  de `lib-mika-bin.sh` (bruyant hors CI, compté en ÉCHEC sous CI, où `cargo
  test` précède les suites bash). Remède local :
  `cargo build -p mika-cli --bin mika`.
- **Le mock de `gh` qui laisse passer une vraie sortie réseau.** La sonde le
  définit en fonction shell dans le sous-shell, avant tout appel, et le writer
  n'invoque jamais `gh` via `command` — idiome déjà utilisé douze fois dans la
  suite.
- **`mktemp /tmp/canonical-callout-XXXXXX.md`** est interne au writer et tiré
  par T1. C'est dispatch-lib qui écrit son propre fichier de corps, pas le
  pilote qui écrit hors worktree : la règle mika#2211 n'est pas en cause, et le
  fichier est supprimé par le writer lui-même sur les deux branches de sortie.
- **Un faux positif d'égalité de chemins.** `_extract_plan_path` rend la forme
  normalisée (premier segment de dépôt strippé), `plan_relpath` vient d'un
  `${plan_path#"$WORKTREE_DIR/"}` déjà borné et vérifié présent : les deux sont
  relatifs à la racine du dépôt. Aucune renormalisation locale n'est ajoutée —
  en ajouter une serait une seconde politique de chemins.

## Definition of Done

- `_write_canonical_callout` ne court-circuite que si le callout présent nomme
  `plan_relpath`, et la lecture de ce nom passe par `_extract_plan_path`.
- `has_branch`, `has_plan`, `has_verdict` sont inchangés, octet pour octet.
- Sur `rc = 1` et `rc ≥ 2` de `_extract_plan_path`, la fonction **écrit** ; le
  commentaire dit pourquoi cette direction.
- Un corps complet nommant un autre plan produit un diagnostic
  `write_canonical_callout_stale_plan` portant les deux plans.
- La ligne de log du court-circuit idempotent est conservée mot pour mot.
- L'en-tête de `_extract_plan_path` nomme son quatrième appelant et le plafond
  de QUATRE démarrages de `mika` par dispatch dev-groom.
- `skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh` tient
  T1 (AC1), T2 (AC2), T3 et T4 ; le rouge-avant de T1 contre le code actuel a
  été constaté et l'ordre de commit le reflète.
- `skills/bundled/_shared/test-dispatch-lib.sh` est **identique octet pour
  octet** à celui d'`origin/main` (`git diff origin/main -- …` vide), donc
  `Secret Scan` ne voit plus de franchissement de `LARGE_FILE_LIMIT`.
- `make test`, `make test-dispatch-lib`, `make verify-bundled-skills`,
  `cargo clippy`, `cargo fmt --check` et
  `bash scripts/verify-pipeline.sh origin/main` passent.
- Aucune entrée ajoutée à `scripts/canonical-tokens.tsv` ni à
  `scripts/suite-wiring.tsv` — le token `> - **Plan:**` garde son site unique en
  Rust, et la nouvelle suite est câblée dans `make test` **et** dans la cible
  `make test-dispatch-lib` que la CI appelle (`check-suite-wiring.sh` la compte
  couverte, `unwired=0`).

## Acceptance criteria

- AC1 : un corps qui porte un callout complet nommant un AUTRE plan fait écrire
  le callout du plan courant, et le bloc précédent est remplacé.
- AC2 : un corps qui porte déjà le callout complet du plan courant reste
  inchangé (idempotence conservée).
- AC3 : les deux cas sont tenus par
  `skills/bundled/_shared/tests/test-dispatch-lib.sh`, avec un rouge-avant sur
  AC1 contre le code actuel.

> Note de transcription : AC3 est tenu à
> `skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh` — le chemin
> de l'énoncé (`…/_shared/tests/test-dispatch-lib.sh`) ne désigne aucun fichier
> existant, et l'agrégat `_shared/test-dispatch-lib.sh` ne peut plus accueillir
> le bloc sans franchir le plafond `LARGE_FILE_LIMIT` de
> `scripts/check-secrets.sh` (mesuré sur la PR #3177). La suite vit donc sous
> `_shared/tests/`, ce que l'énoncé désignait déjà comme répertoire. Voir § Tests
> pour la mesure et le câblage.

## Fire-Disposition

Ce vol livre des détecteurs : T1 à T4 sont des tests dont le chemin de succès
est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une allowlist
VIDE.**

L'allowlist est vide, et elle l'est par construction plutôt que par chance :

- les quatre tests portent sur le comportement de `_write_canonical_callout`
  **après** le correctif du même vol. Il n'existe pas de population existante à
  exempter — le seul état qui les fait rougir est le code d'avant l'étape 2 de
  l'ordre de commit, et cet état ne survit pas au vol ;
- T4 ne fait que **préserver** des assertions déjà vertes aujourd'hui ; une
  seule assertion de forme y est ajoutée, sur du code que le vol écrit.

Aucun détecteur n'atterrit désarmé (pas de `#[ignore]`, pas de `skip`), aucune
entrée d'allowlist n'est créée, donc aucun ticket de suivi d'armement n'est
ouvert. Si T1 rougissait encore après l'étape 2, ce ne serait pas une violation
à exempter mais un correctif incomplet : la réponse serait le correctif, jamais
une entrée.

Le seul non-armement possible est le **SKIP** du binaire `mika` absent, et il
est déjà traité par la troisième colonne de `lib-mika-bin.sh` (mika#2149) :
bruyant hors CI, compté en ÉCHEC sous `CI`/`GITHUB_ACTIONS`. Ce n'est pas une
exception, c'est un bloc qui refuse de se lire comme un vert.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — quatrième prédicat + diagnostic + commentaire de raison | 50 |
| `skills/bundled/_shared/dispatch-lib.sh` — en-tête de `_extract_plan_path` (quatrième appelant, plafond 3 → 4) | 12 |
| tests (`skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh` — en-tête + helpers recopiés + sonde + T1/T2/T3/T4) | 145 |

Total estimé : 207 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Un seul vol, et voici pourquoi il tient : 207 < 279 (borne par vol) et
207 < 1000 (seuil par PR), donc aucune des deux bornes ne mord. Le correctif
lui-même est une conjonction étendue de deux termes et un bloc de diagnostic ;
l'essentiel du volume est du commentaire de raison et une sonde de test, et ni
l'un ni l'autre ne demande de tour d'exploration — la sonde est modelée sur
`_t2492_recovery_probe`, qui existe déjà dans la même suite, et les deux
fixtures ne diffèrent que par une ligne. Aucun fichier Rust n'est modifié, donc
pas de cycle `cargo build` dans la boucle d'itération : la fin de pipeline est
celle d'un changement bash plus les suites qui le couvrent.

Refs #3118

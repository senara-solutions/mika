# fix(dispatch-lib): `rescue-pipeline-verified` n'a aucun producteur, donc le drain ne peut pas être autonome (mika#2354)

## Ce que le ticket affirme, et ce que la mesure établit

Le ticket attribue la PR rescue-class systématique à une cause unique : le pas
`/ce:code-review` écrit dans `/tmp/compound-engineering-…`, la permission-policy
refuse, « l'étape review échoue → le pilote ne termine pas son flux post-review ».
Il propose deux remèdes, (a) faire écrire le skill dans le worktree, (b) allowlister
le chemin temporaire.

Les trois preuves qu'il cite ont été relues dans les journaux claude-pilot. Elles ne
portent pas la même cause, et deux d'entre elles ne portent pas du tout celle du
ticket.

| Preuve | Journal | `ce-code-review` invoqué | Deny `/tmp/compound-engineering` | Fin de session |
|---|---|---|---|---|
| PR #2350 (mika#2342), `dirty-worktree` | `0e794b0a` | **0** | **0** | `[done] Success \| 158 turns \| $38.44 \| 2704s` |
| PR #2352 (mika#2270), `dirty-worktree` | `51cf044f` | **0** | **0** | `[done] Success \| 93 turns \| $18.33 \| 2123s` |
| PR #2353 (mika#2279), `commit-pushed-no-pr` | `63bcc79d` | 41 | 6 `Write` + 3 `mkdir` | `[guardrail] stall_detected: 5 consecutive turns with no tool calls` |

Quatre rectifications suivent de là, chacune adossée à une ligne de journal ou de code.

**R1 — la cause proposée ne couvre qu'une preuve sur trois.** Les pilotes de #2350 et
#2352 n'ont jamais invoqué `ce-code-review` ; aucun deny `/tmp/compound-engineering`
n'apparaît dans leurs journaux. Un remède sur ce chemin temporaire les laisse
identiques.

**R2 — le chemin temporaire est déjà allowlisté ; le trou est ailleurs, et il est
étroit.** À `23:13:16.333Z`, `63bcc79d` porte
`[policy:allow] Bash: mkdir -p /tmp/compound-engineering-1000/ce-code-review/… [bash-mkdir-tmp-scratch]`.
Une règle nommée couvre déjà ce chemin. Le refus de `23:13:14.448Z` porte sur la
**chaîne** `mkdir && chmod && echo`, pas sur la destination — la même classe de veto
que `python3 -c "open('…/write-probe.txt','w')…"` traverse sans encombre (`→ AUTO`).
Seul l'outil `Write` vers ce répertoire est refusé, « no matching policy rule —
denied by default ». L'option (b) du ticket est donc, pour le chemin, déjà en
service.

**R3 — le deny n'a pas fait échouer la review.** Le skill porte un contrat
« if the write fails, continue » ; les trois lanes ont rendu leur JSON
(`"findings": []`), et le journal le dit explicitement : *« the write … was denied by
sandbox policy … the compact JSON above is the authoritative output for this review
lane »*. La review **a abouti**. Le pilote est mort trente secondes plus tard, en
`stall_detected`, sur un `mkdir` redondant. La chaîne « deny → review échouée → flux
post-review interrompu » n'est pas ce que le journal montre.

**R4 — la racine est écrite dans le code, et elle est autre.** L'entry-command des
dispatches d'implémentation n'est pas `/mika` mais `/ce-work <plan> mika#N`
(`_detect_plan_on_branch`, `dispatch-lib.sh:5876`, mika#1074). Le skill `ce-work` est
décrit par son propre plugin comme *« implementation and local verification only,
without the shipping tail »*. Et `dispatch-lib.sh:3486` le dit sans détour :

> the pilot session ran content and committed, but ended its turn before invoking
> `gh pr create` (**Mode 1 = bare `/ce-work` launch never had commit→PR in scope**;
> Mode 2 = full `/mika` launch hit prompt-enforcement fragility on the tail).
> […] Honors Vincent's pre-reboot framing: "gate the loop until the tail's fixed".

**La PR rescue-class est le chemin nominal du Mode 1, pas un accident.** Les trois
pilotes ont fait exactement ce qu'on leur a demandé ; deux ont fini `Success`. Rien
de ce que l'on corrigera sur `/tmp` ne changera cela.

## Le vrai goulot : un marqueur sans producteur

Ce qui force le geste opérateur n'est pas la naissance de la PR rescue — c'est le
marqueur qu'elle porte.

`_compose_rescue_pr_body` (`dispatch-lib.sh:5756`) écrit
`<!-- rescue-pipeline-verified: no -->` **en dur, inconditionnellement**, et le corps
énonce le geste manuel comme un pas du design : *« Operator: verify pipeline
completion, then either un-draft this PR or set the marker above to `yes`. »*

En face, deux portes lisent ce littéral :

- **qa-review Step 1.5** — marqueur `no` **et** draft ⇒ `hold[review]`, fin de revue
  avant le Step 2 (`skills/bundled/qa-review/system_prompt.md:134`).
- **`wip_rescue`** — depuis mika#2286, un draft DECISION-CORE n'est un-drafté que sur
  le littéral `yes` ; sinon il est *parké* (`crates/mika-agent/src/wip_rescue.rs:50`).

Et la mesure qui ferme le dossier : **`grep -rn "rescue-pipeline-verified: yes"` sur
`skills/`, `scripts/` et `crates/mika-agent/src/` ne rend que des lecteurs** — deux
dans le prompt qa-review, cinq dans la doc et le code de `wip_rescue`, un dans une
constante de test. **Aucun site du dépôt n'écrit jamais `yes`.** Le seul chemin vers
cette valeur est une main humaine éditant le corps de la PR.

Ce n'est donc pas que le drain *tombe* en panne d'autonomie sur certaines PR : **il
ne peut structurellement pas être autonome**, parce que le marqueur qui le débloque
n'a pas de producteur. Le ticket le décrit exactement (« C'est le goulot qui force le
geste manuel ») tout en l'attribuant à la mauvaise cause.

Ce défaut est par ailleurs déjà nommé, une fois : mika#2334 l'a écarté de son propre
périmètre sous le nom de *« maillon 2 — le faux-étiquetage rescue-class (mika#1282/#1618),
qui fait refuser QA en `hold[review]` sur du travail complet »*, avec la mention
**« ticket de suivi à ouvrir »**. mika#2354 est ce ticket de suivi.

## Décision de périmètre

**Ce qui est écarté, avec sa raison.** Les deux remèdes proposés par le ticket vivent
hors de ce dépôt : (a) dans le plugin `compound-engineering`
(`~/.claude/plugins/cache/every-marketplace/compound-engineering/3.26.3/skills/ce-code-review/`),
que mika#2334 a déjà déclaré hors périmètre pour cette raison exacte ; (b) dans
`claude-pilot-py/src/claude_pilot/policies/permissions.yaml`, un autre dépôt (cpp#189).
Aucun des deux ne répare #2350 ni #2352 (R1), et sur #2353 la review avait abouti
malgré les denies (R3). Ils restent utiles — le `stall_detected` et le trou `Write`
sont réels — mais ils appartiennent à cpp#189 et à un ticket plugin, et ils
n'affranchissent pas le drain du geste manuel.

**Ce qui est retenu.** Donner au marqueur un producteur mécanique, dans le seul
dépôt où il est écrit, avec une mesure à la hauteur du sens que ses consommateurs lui
donnent.

**Ce que ce travail ne prétend pas faire.** Il ne fait pas ouvrir la PR par le
pilote : le tail reste à dispatch-lib (mika#1271) et la PR reste une rescue-PR draft.
Il ne fait pas sortir du draft : `wip_rescue` reste seul à un-drafter et
l'invariant mika#1941 (« aucun PR ne quitte wip-rescue/draft tant que la review
multi-agent formelle n'est pas POSTÉE ») est intact. **`yes` ne dira pas « cette PR
est bonne » ; il dira « le pipeline local est complet, la revue peut commencer ».**
C'est précisément ce que le Step 1.5 tient aujourd'hui fermé, et rien de plus.

## Conception

### Le précédent que l'on suit, à trente lignes de distance

`rescue-diff` est déjà un fait **mesuré** par le producteur :
`_rescue_diff_carries_work` (`dispatch-lib.sh:5683`) traverse le vrai
`git diff origin/main...HEAD`, avec liste fermée, `core.quotePath=false -z`
load-bearing, et un fail-closed argumenté. Le commentaire de
`_compose_rescue_pr_body` cite lui-même ce parallèle : *« same producer/consumer split
as the `rescue-pipeline-verified` marker, mika#1618 »*. Le split existe pour
`rescue-diff` ; pour `rescue-pipeline-verified` il n'a jamais eu sa moitié producteur.

Second précédent, dans la même fonction de rescue : dispatch-lib **exécute déjà
`cargo fmt --all` sur le worktree** pendant le rescue (mika#1296,
`dispatch-lib.sh:3294`). Faire tourner un outil de vérification à cet endroit n'est
pas une nouveauté architecturale.

### La mesure, et pourquoi elle doit être celle-là

mika#2286 fixe le sens du marqueur sans ambiguïté : `yes` vaut *« vérification
fraîche du pipeline »*, et le daemon *« has no right to un-draft the sensitive class
blind »* sans elle. Un `yes` posé sur la seule présence d'artefacts rouvrirait ce
ticket sous un autre nom. La mesure doit donc être une **vérification réellement
exécutée**, pas une inspection de forme.

`_measure_pipeline_verified` — nouvelle fonction, appelée depuis le tail de rescue
juste avant `_compose_rescue_pr_body`, dont le résultat est passé en argument (le
littéral disparaît du heredoc). Conjonction, évaluée dans l'ordre du moins cher au
plus cher, court-circuitée au premier échec :

1. Le diff porte du travail — réutilise `_rescue_diff_carries_work`, déjà calculé par
   l'appelant ; un diff incident-only ne peut satisfaire aucun AC et n'a rien à faire
   vérifier.
2. Le worktree est propre après le commit de rescue (`git status --porcelain` vide,
   mêmes exclusions scaffold que le `git add -A` du rescue) — sinon du contenu reste
   hors de la PR et « complet » serait faux.
3. `cargo fmt --all --check` propre.
4. `cargo clippy --workspace --all-targets` sans erreur.
5. `scripts/verify-pipeline.sh origin/main` passe — l'étape 7 du pipeline `/mika`, dont
   l'en-tête dit « Verify that the /mika pipeline produced required artifacts before PR
   creation ». C'est littéralement la vérification que le corps demande à l'opérateur.
   L'argument est **porteur** : sans lui le script compare à `main` local
   (`BASE_REF="${1:-main}"`, `verify-pipeline.sh:91`), qui dans un worktree de dispatch
   peut avoir des jours de retard — le bucket `source`/`docs` serait alors calculé sur
   un diff qui n'est pas celui que la PR publiera. `origin/main` est le mode que la CI
   emploie, documenté dans l'usage du script lui-même.
   Ce terme a une limite mesurée, traitée en § Fire-Disposition : deux de ses trois
   mécanismes d'exemption lisent des artefacts qui n'existent pas encore au moment de
   la mesure (le corps de la PR pour l'héritage du label `documentation`,
   `GITHUB_EVENT_PATH` pour le label `pipeline-exempt`), donc il est ici **plus strict
   que dans la CI**.

**Fail-closed sans exception.** Tout ce qui n'est pas une réussite explicite rend
`no` : commande absente, budget dépassé, `$WORKTREE_DIR` vide, dépôt illisible,
`verify-pipeline.sh` non exécutable. On ne peut donc jamais être plus permissif
qu'aujourd'hui, où la valeur est `no` en toutes circonstances. L'asymétrie est celle
que mika#2157 a déjà arbitrée sur ce même corps de PR : un `no` de trop coûte un
geste opérateur visible et réversible ; un `yes` de trop ouvre la revue sur un
travail incomplet, et les deux protections restantes (`--draft`, le marqueur) sont
« revocable by a single human gesture ».

**Le `no` devient actionnable.** Aujourd'hui il ne dit pas ce qu'il reste à faire.
Le premier terme qui échoue est nommé dans le corps, sous un marqueur lisible par
machine `<!-- rescue-verify-failed: <terme> -->`, suivi des premières lignes de sa
sortie. L'opérateur lit au lieu d'enquêter — et c'est ce qui rend le geste manuel
rare *et* rapide quand il reste nécessaire.

### Réglages

Convention maison à trois paliers (absent/vide → défaut ; illisible, `0` ou négatif →
défaut + WARN) :

- `MIKA_RESCUE_VERIFY_ENABLED` — kill-switch, défaut armé. `0` restaure **verbatim**
  le littéral `no` d'aujourd'hui, sans redéploiement. Le rollback doit être exact :
  un rollback qui change aussi la forme du corps n'en est pas un.
- `MIKA_RESCUE_VERIFY_BUDGET_SECS` — budget global de la mesure, défaut `900`, aligné
  sur le clippy gate de `wip_rescue` qui exécute déjà cette classe de travail en aval.
  Dépassement ⇒ `no` + terme `budget`, jamais un `yes` partiel.

#### Comment ces deux variables atteignent `dispatch-lib.sh` (F1, tranché)

Le plan laissait la propagation « à trancher en écrivant le code ». Elle est tranchée
ici, et la mesure montre que le dilemme était **mal posé** : le choix du nom n'a aucun
effet sur la propagation.

**Ce que le code fait, lu à la source.** `dispatch-lib.sh` est lancé par
`spawn_long_running_exec` (`crates/mika-agent/src/skills/executor.rs:3181`), qui
appelle `sandboxed_pilot_env` (`executor.rs:115`) — et cette fonction ne *retire* pas
les `MIKA_*`, elle fait `env_clear()` puis ne recopie que l'allowlist **positive**
`SANDBOX_ENV_CORE_ALLOWLIST` + `SANDBOX_ENV_ALLOWED_PREFIXES` (`PATH`, `HOME`, `USER`,
`LOGNAME`, `SHELL`, `TERM`, `LANG`, `LC_ALL`, `TMPDIR`, `HOSTNAME`, puis les préfixes
`LC_`, `XDG_`, `NVM_`, `CARGO_`, `RUSTUP_`). **Aucun nom hors de cette liste ne
traverse, préfixé ou non** : `RESCUE_VERIFY_ENABLED` sans préfixe serait effacé
exactement comme `MIKA_RESCUE_VERIFY_ENABLED`. Le « repli sans préfixe » que le plan
envisageait ne propage rien.

**Le précédent `PILOT_LOG_DIR` ne dit pas ce qu'on lui faisait dire.**
`grep -rn PILOT_LOG_DIR skills/ crates/ scripts/` ne rend que le défaut de lecture
(`dispatch-lib.sh`) et deux fichiers de test — **rien ne le pose en production**. Il
n'a donc jamais eu à traverser ce sandbox, et n'établit rien sur cette question. La
phrase du CLAUDE.md qui le cite raisonne d'ailleurs sur `scrub_mika_env_vars`
(`executor.rs:767`), la voie exec-handler courte, et non sur `sandboxed_pilot_env`
(`executor.rs:3204`), la voie long-running qui porte `dispatch-lib.sh`. Deux voies,
deux mécanismes ; seule la seconde est sur ce chemin.

**Décision : préfixe `MIKA_` conservé, propagation par injection explicite.** La seule
voie qui traverse est celle que le fichier emprunte déjà trois fois, immédiatement
après le sandbox : `GH_TOKEN` (`executor.rs:3205`), `MIKA_PILOT_TRANSCRIPT_FILE`
(`executor.rs:3210`, mika#1705) et `MIKA_DISPATCH_WORKTREE_FILE` (`executor.rs:3214`,
mika#2249). Deux des trois portent le préfixe : garder `MIKA_RESCUE_VERIFY_*` est la
lecture cohérente, et un nom sans préfixe coûterait une divergence de vocabulaire sans
rien acheter.

**La nuance qui distingue cette injection de ses deux sœurs, et qui décide sa forme.**
Les deux sœurs injectent un chemin **calculé par le moteur** ; celle-ci relaie une
valeur **d'opérateur** lue dans l'environnement du process mika-spirit. Première de sa
classe, donc : elle ne pose la variable **que** si elle est présente et non vide côté
spirit. Une absence ne devient jamais une valeur — le shell garde son propre défaut au
lieu d'en hériter un silencieusement, ce qui est la différence entre un réglage absent
et un réglage posé à la valeur par défaut, deux états qu'un opérateur doit pouvoir
distinguer.

**Ce que la mesure ne franchit pas, et ce n'est pas un oubli.** `bwrap` n'enveloppe que
l'invocation `claude-pilot` (`_run_pilot_sandboxed`) ; le tail de rescue — donc
`_measure_pipeline_verified` et les `cargo` qu'elle lance — tourne **hors** bubblewrap,
comme le `git worktree add`. L'allowlist `--setenv` du sandbox pilote n'est donc pas
sur ce chemin et n'a pas à être élargie.

Une variable que seul le lecteur honore est un réglage décoratif (mika#2165) ; c'est
exactement ce que l'injection explicite empêche, et AC9 l'épingle.

### Coût, nommé

La mesure ajoute un `fmt` + un `clippy --workspace --all-targets` au tail d'un
dispatch qui a déjà tourné trente à quarante-cinq minutes. C'est du temps machine qui
remplace un geste humain, sur un chemin déjà lent et déjà bloqué. Il n'est payé que
sur la voie rescue — jamais sur un dispatch qui ouvre sa PR normalement — et le
kill-switch le rend réversible sans rebuild.

## Unités d'implémentation

**U1 — `_measure_pipeline_verified`** dans `skills/bundled/_shared/dispatch-lib.sh`,
posée à côté de `_rescue_diff_carries_work` dont elle reprend la forme (garde
`$wt_dir` vide → fail-closed, commentaire d'en-tête portant l'arbitrage). Rend `0`
pour vérifié, `1` sinon, et écrit sur stdout le nom du terme en échec. Le terme 5
invoque `scripts/verify-pipeline.sh origin/main` — l'argument est porteur, voir
§ La mesure. Prérequis : U7 (sans le canal, le kill-switch est décoratif).

**U2 — `_compose_rescue_pr_body`** prend un cinquième argument (l'état vérifié) et le
nom du terme en échec ; le littéral `no` du heredoc est remplacé par la valeur
mesurée. Quand la valeur est `no`, le corps porte
`<!-- rescue-verify-failed: <terme> -->` et l'extrait de sortie. La phrase « Operator:
verify pipeline completion… » est réécrite pour les deux cas : sur `yes` elle n'a plus
d'objet, sur `no` elle nomme le terme à traiter.

**U3 — site d'appel** dans `dispatch_claude_pilot` (Path B, autour de
`dispatch-lib.sh:6482`), derrière le kill-switch, après le commit de rescue et le
push, avant `gh pr create`. La mesure porte sur l'état exact que la PR va publier.

**U4 — `skills/bundled/_shared/test-dispatch-lib.sh`.** L'assertion existante
(`AC3: Path B emits the rescue-pipeline-verified marker`, ligne 3179) affirme
aujourd'hui `'rescue-pipeline-verified: no'` en dur : elle doit devenir
conditionnelle, sans quoi elle épinglerait le défaut. Tests symétriques sur dépôts
temporaires réels, comme `_rescue_diff_carries_work` en a établi l'usage : un
worktree propre et complet rend `yes` ; chacun des cinq termes mis en échec un à un
rend `no` **et** nomme son terme ; `MIKA_RESCUE_VERIFY_ENABLED=0` rend le corps
byte-identique à celui d'aujourd'hui.

**U5 — `skills/bundled/qa-review/system_prompt.md`, Step 1.5.** Le prompt n'a pas
besoin de changer de logique : il lit déjà `yes`/`no`. Une phrase est ajoutée à
l'item 4 pour dire que `yes` peut désormais être posé par le producteur sur mesure
mécanique, et que ce `yes` atteste la complétude du pipeline local — pas la qualité
du travail, qui reste l'objet de la revue qui suit. Sans cette phrase, un relecteur
lira `yes` comme une approbation préalable.

**U6 — documentation.** Une entrée `docs/solutions/` sur la classe « un marqueur
lisible par deux consommateurs et écrit par personne », et la mise à jour de la
section CLAUDE.md qui décrit le marqueur, avec ses surfaces opérateur.

**U7 — canal de réglage, côté Rust.** `inject_rescue_verify_env(&mut cmd)` dans
`crates/mika-agent/src/skills/executor.rs`, posée à côté de
`inject_dispatch_worktree_env` et appelée au même endroit — dans
`spawn_long_running_exec`, **après** `sandboxed_pilot_env`, avec le même commentaire
d'ancrage que ses deux sœurs. Elle relaie `MIKA_RESCUE_VERIFY_ENABLED` et
`MIKA_RESCUE_VERIFY_BUDGET_SECS` depuis l'environnement du process spirit, et **ne
pose que ce qui est présent et non vide** (voir § Réglages). Unité listée en dernier
mais **prérequis de U1** : sans elle, le kill-switch est un réglage que seul le
lecteur honore.

## Fire-Disposition

Requis par le Fire-Disposition Gate (mika#1574,
`docs/solutions/best-practices/fire-disposition-doctrine.md`). Ce plan porte trois
livrables de classe détecteur ; ils n'ont pas la même population, donc pas la même
disposition, et les séparer est ce qui rend chacune vérifiable.

### D1 — `_measure_pipeline_verified` (le détecteur qui balaie des données réelles)

C'est le seul des trois qui s'exécute sur une population pré-existante : ses termes 3,
4 et 5 traversent l'état réel du dépôt au moment du rescue.

**Population mesurée sur la branche de ce plan, aujourd'hui :**

| Terme | Commande | Résultat |
|---|---|---|
| 3 | `cargo fmt --all --check` | rc=0, sortie vide |
| 4 | `cargo clippy --workspace --all-targets` | rc=0, zéro ligne `warning:` |
| 5 | `scripts/verify-pipeline.sh` | présent, exécutable (`-rwxr-xr-x`) |

**Disposition retenue : (c) halt-and-surface, et le fail-closed en EST la forme.** Un
terme qui fire ne casse pas la CI et ne rend pas la main à un choix du pilote : il
produit `no` + `<!-- rescue-verify-failed: <terme> -->` + l'extrait de sortie, c'est-à-dire
le geste opérateur d'aujourd'hui, mais nommé. La surface de remontée est la PR, pas le
test — ce qui est le bon endroit, puisque la donnée qui fire appartient à cette PR-là.

**(a) allowlist nommée est écartée sur mesure, pas par préférence :** la population à
exempter est **vide** sur les deux termes exemptables (fmt, clippy). Une allowlist née
vide est un endroit où déposer la prochaine violation, et personne ne saurait plus si
elle protège un cas mesuré ou une habitude.

**(b) land disabled est écartée parce que le kill-switch la couvre déjà**, et mieux :
`MIKA_RESCUE_VERIFY_ENABLED=0` désarme sans rebuild et restaure le corps d'aujourd'hui à
l'octet près (AC4). Livrer désarmé exigerait une condition de réarmement, et mika#2272 a
mesuré ce que coûte une condition de réarmement **insatisfiable** : mika#2249 a attendu
trois lignes d'audit qu'un scan à population vide ne pouvait pas produire.

### D1-bis — le terme 5 fire sur une classe légitime, et c'est nommé plutôt que corrigé

`verify-pipeline.sh` a trois mécanismes d'exemption ; **deux sont inopérants à l'endroit
où la mesure tourne**, parce qu'ils lisent des artefacts qui n'existent pas encore : (1)
l'héritage du label `documentation` passe par `Closes #N` dans le **corps de la PR**, que
`gh pr create` n'a pas encore écrit ; (2) le label `pipeline-exempt` est lu depuis
`GITHUB_EVENT_PATH`, absent hors runner. Seul (3), le trailer de commit
`Pipeline-Exempt:`, fonctionne. Conséquence : un travail docs-only légitime dont le
ticket porte le label `documentation` rend `no` ici alors qu'il passerait en CI.

**Disposition : accepter ce `no`, le nommer, et ne pas le compenser.** Il est du côté sûr
de l'asymétrie déjà arbitrée (§ La mesure) — un `no` de trop coûte un geste opérateur
visible, un `yes` de trop ouvre la revue sur du travail incomplet. La sortie du script
commence par `FAIL: …`, qui est portée telle quelle dans le corps sous
`rescue-verify-failed: verify-pipeline`, donc l'opérateur lit la cause au lieu de
l'enquêter. **Ce qu'il ne faut PAS faire en réaction** : rendre le terme 5 permissif, ni
le retirer de la conjonction. La bonne correction, si la fréquence le justifie, est de
donner au script un quatrième mécanisme d'exemption lisible avant la PR — ticket séparé,
pas un assouplissement décidé ici. La sonde § Sondes post-déploiement compte cette
population séparément, précisément pour que cette décision repose sur un chiffre.

### D2 — les tests de U4 (population vide par construction)

Les assertions symétriques de U4 montent leurs propres dépôts temporaires, à la manière
de celles de `_rescue_diff_carries_work`. Il n'existe aucune donnée pré-existante
qu'elles puissent traverser : leur population est bâtie par le test et détruite avec lui.
**Disposition : sans objet**, et dit ici plutôt que tu par prudence — la doctrine demande
de nommer, pas seulement de traiter.

### D3 — l'assertion AC3 existante (une mise à jour, pas un feu)

`test-dispatch-lib.sh:3179` affirme `'rescue-pipeline-verified: no'` en dur, par un scan
du corps de `_compose_rescue_pr_body`. Elle rougira au commit qui remplace le littéral —
mais c'est **le même commit** qui la met à jour (U4), et il n'y a là non plus aucune
donnée pré-existante. **Disposition : mise à jour atomique dans U4**, jamais un `skip`
ni une exception. Une assertion qui affirme le littéral que ce ticket existe pour retirer
épinglerait le défaut ; la laisser désarmée le cacherait.

## Contrat de vérification

- `bash skills/bundled/_shared/test-dispatch-lib.sh` — vert, U4 compris.
- `make verify-bundled-skills` — structure des bundles intacte.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test` —
  aucun code Rust n'est touché ; on atteste l'absence de régression, pas un
  changement.
- `scripts/check-pr-body-consistency.sh` et `scripts/verify-pipeline.sh` sur cette
  PR même.
- Aucun test ne re-mesure ce que `_measure_pipeline_verified` mesure : les tests
  assertent la **décision** de la fonction, pas la santé du dépôt au moment où ils
  tournent.

## Definition of Done

- `_measure_pipeline_verified` existe, est appelée sur la voie rescue, et son
  résultat est ce que le corps de la PR publie.
- Aucun littéral `no` ne subsiste dans `_compose_rescue_pr_body`.
- Le kill-switch restaure le corps d'aujourd'hui à l'octet près, **et il est
  effectivement lu** : les deux variables sont injectées explicitement après
  `sandboxed_pilot_env` (U7), jamais laissées à un héritage que l'allowlist positive
  bloque.
- Les deux consommateurs (qa-review Step 1.5, `wip_rescue`) sont inchangés dans leur
  logique de lecture ; seule la note explicative du prompt bouge.
- La rectification du diagnostic (R1–R4) est portée dans le corps de la PR, pour que
  cpp#189 hérite d'un périmètre juste plutôt que de la cause supposée.

## Acceptance criteria

- [ ] AC1 — Sur un worktree dont le diff porte du travail, le worktree propre, `cargo fmt --all --check` et `cargo clippy --workspace --all-targets` propres et `scripts/verify-pipeline.sh` vert, la PR de rescue porte `<!-- rescue-pipeline-verified: yes -->`.
- [ ] AC2 — Chacun des cinq termes mis en échec isolément produit `<!-- rescue-pipeline-verified: no -->` **et** un `<!-- rescue-verify-failed: <terme> -->` nommant le terme fautif. Un test par terme.
- [ ] AC3 — Toute condition non mesurable (`$WORKTREE_DIR` vide, dépôt illisible, `cargo` absent, `verify-pipeline.sh` absent ou non exécutable, budget dépassé) produit `no`. Aucun chemin ne produit `yes` sans les cinq termes satisfaits.
- [ ] AC4 — `MIKA_RESCUE_VERIFY_ENABLED=0` produit un corps de PR byte-identique à celui d'avant ce changement, marqueur `no` compris, sans redéploiement.
- [ ] AC5 — L'assertion `AC3: Path B emits the rescue-pipeline-verified marker` de `test-dispatch-lib.sh` n'affirme plus `no` en dur et couvre les deux valeurs.
- [ ] AC6 — Le Step 1.5 de qa-review énonce que `yes` atteste la complétude du pipeline local et non la qualité du travail ; la logique `yes`/`no` est inchangée.
- [ ] AC7 — `wip_rescue` n'est pas modifié : aucun diff sous `crates/mika-agent/src/wip_rescue.rs`. Le daemon reste lecteur, jamais écrivain de son propre feu vert.
- [ ] AC8 — Une PR de rescue produite par un dispatch dont le pipeline est complet traverse le Step 1.5 de qa-review sans `hold[review]` sur le motif du marqueur.
- [ ] AC9 — `MIKA_RESCUE_VERIFY_ENABLED` et `MIKA_RESCUE_VERIFY_BUDGET_SECS` atteignent `dispatch-lib.sh` par **injection explicite après `sandboxed_pilot_env`**, jamais par héritage. Deux assertions : (a) un test Rust sur `inject_rescue_verify_env` vérifie qu'une variable absente ou vide côté spirit n'est pas posée côté enfant ; (b) un test refuse que ces deux noms soient ajoutés à `SANDBOX_ENV_CORE_ALLOWLIST` ou couverts par `SANDBOX_ENV_ALLOWED_PREFIXES` — l'allowlist positive reste la garde, l'injection reste l'exception nommée.
- [ ] AC10 — Le terme 5 est invoqué avec `origin/main` en argument, jamais avec le défaut `main` local. Assertion de forme sur le corps de `_measure_pipeline_verified` : une comparaison à un `main` de worktree mesurerait un diff qui n'est pas celui que la PR publie.

## Risques, et ce qui les borne

**Rouvrir mika#2286 par la bande.** C'est le risque principal : poser `yes` sur une
mesure trop faible rendrait au marqueur la fausseté que trois tickets successifs lui
ont retirée. Il est borné par le choix de mesurer une **exécution** (fmt, clippy,
verify-pipeline) plutôt qu'une forme, par le fail-closed intégral, et par AC7 — le
producteur reste dispatch-lib, sole writer ; aucun consommateur ne s'auto-délivre son
autorisation.

**Un `yes` sur un travail que la revue rejettera.** Ce n'est pas un défaut : `yes`
ouvre la revue, il ne la conclut pas. Le cas nominal attendu est précisément qu'une
PR vérifiée entre en revue et y soit jugée.

**Le clippy du tail diverge de celui de `wip_rescue`.** Les deux tournent à des
moments différents (avant rebase / après rebase) et n'ont pas les mêmes conséquences.
La duplication est assumée et à noter au site d'appel ; les unifier supposerait de
déplacer la mesure chez un consommateur, ce qu'AC7 refuse.

## Sondes post-déploiement, et leurs haltes

- **Attribution, 72 h.** Compter les PR de rescue portant `yes` contre celles portant
  `no`. Un régime à `no` quasi total signifie que la mesure est trop stricte ou qu'un
  terme échoue systématiquement — lire le `rescue-verify-failed` avant de toucher au
  moindre seuil.
- **Le terme 5 compté à part, 72 h (§ Fire-Disposition D1-bis).** Parmi les `no`,
  compter ceux dont le `rescue-verify-failed` vaut `verify-pipeline` **et** dont le
  ticket porte le label `documentation` : c'est la population des faux `no` structurels,
  celle que la mesure ne peut pas voir avant que la PR existe. Elle est attendue rare.
  Si elle domine les `no`, le remède est un mécanisme d'exemption lisible avant la PR,
  dans son propre ticket — **ne pas assouplir le terme 5**, ce serait rendre permissif
  le seul terme dont on aura mesuré qu'il refuse pour une bonne raison.
- **Symptôme, 72 h.** Les `hold[review]` de qa-review motivés par le marqueur doivent
  tendre vers zéro sur les PR dont le pipeline est complet. S'ils persistent avec un
  marqueur `yes`, **halte** : le Step 1.5 tient la PR pour une autre raison
  (incident-only, draft) et le correctif n'aurait réparé que la visibilité.
- **Fail-open, en continu.** Une PR portant `yes` dont le worktree était sale, ou dont
  clippy était rouge, est un fail-open : désarmer par `MIKA_RESCUE_VERIFY_ENABLED=0`
  et réparer le terme fautif — **ne pas ajouter de terme compensatoire** sans avoir
  établi lequel des cinq a menti.
- **Ce que la sonde ne dira pas.** Le `stall_detected` de #2353 et le trou `Write`
  sur `/tmp/compound-engineering` survivront à ce changement. Leur disparition n'est
  pas un critère ici ; leur persistance n'est pas un échec de ce travail. Ils
  appartiennent à cpp#189.

## Hors périmètre, délibérément

- **Le tail lui-même** — faire que le pilote ouvre sa PR. `dispatch-lib.sh:3486`
  porte la décision inverse (« dispatch-lib owns the git/PR tail per mika#1271 ») et
  la revenir est un ticket d'architecture, pas un correctif.
- **Le deny `/tmp/compound-engineering` et le `stall_detected`** — cpp#189 et le
  plugin `compound-engineering`. Les mesures R2/R3 leur sont utiles et doivent leur
  être transmises : le chemin est déjà allowlisté pour `mkdir`, le trou est sur
  l'outil `Write`, et le contrat « if the write fails, continue » du skill fait que le
  deny dégrade la review sans la casser.
- **Le choix de `/ce-work` comme entry-command** (mika#1074). Il est délibéré et
  résout la classe narrate-then-exit ; ce plan ne le relitige pas.
- **mika#2348 (fmt rescue)**, adjacent et déjà traité ailleurs.
- **Un quatrième mécanisme d'exemption pour `verify-pipeline.sh`**, lisible avant que
  la PR existe (§ Fire-Disposition D1-bis). Réel, mesurable, et conditionné à la sonde
  des 72 h : ouvrir ce ticket avant d'avoir le chiffre serait prescrire sans mesure.

## Revision history

- rev 2 (2026-09-17) : adressé **F1** en tranchant la propagation des deux variables
  plutôt qu'en la déléguant à l'implémentation — la mesure
  (`sandboxed_pilot_env`, `executor.rs:115`, `env_clear()` + allowlist positive) montre
  qu'**aucun** nom ne traverse par héritage, préfixé ou non, donc le dilemme
  « `MIKA_*` vs repli sans préfixe » était mal posé ; décision retenue : préfixe
  conservé + injection explicite après le sandbox, à côté de `MIKA_PILOT_TRANSCRIPT_FILE`
  et `MIKA_DISPATCH_WORKTREE_FILE` (nouvelle unité U7, nouvel AC9). Le précédent
  `PILOT_LOG_DIR` invoqué par la rev 1 est écarté sur mesure : rien ne le pose en
  production, il n'a jamais eu à traverser ce sandbox (citation : review-guide.md
  § Unresolved-Decision Gate, mika#1244).
  Adressé **F2** par une section `## Fire-Disposition` séparant trois populations de
  détecteurs : D1 `_measure_pipeline_verified` → option (c) halt-and-surface, le
  fail-closed en étant la forme, avec (a) et (b) écartées sur mesure (population
  exemptable vide : `cargo fmt --all --check` rc=0 et `cargo clippy --workspace
  --all-targets` rc=0 zéro warning sur cette branche ; et le kill-switch couvre déjà
  (b)) ; D2 les tests de U4 → sans objet, population bâtie par le test ; D3 l'assertion
  AC3 existante → mise à jour atomique dans U4, jamais un skip (citation :
  review-guide.md § Fire-Disposition Gate, mika#1574, et
  `docs/solutions/best-practices/fire-disposition-doctrine.md`).
  Trouvé en chemin en instruisant F2 et porté dans le plan : le terme 5 est **plus
  strict au moment de la mesure que dans la CI** (deux de ses trois exemptions lisent
  le corps de la PR et `GITHUB_EVENT_PATH`, tous deux absents avant `gh pr create`) —
  D1-bis le nomme, une sonde le compte à part, et un ticket de suivi conditionné à ce
  chiffre est ajouté au hors-périmètre. Et le terme 5 doit être invoqué avec
  `origin/main` (`verify-pipeline.sh:91` fait défaut sur `main` local, stale dans un
  worktree de dispatch) — nouvel AC10.
  Aucun AC affaibli ; deux ajoutés.

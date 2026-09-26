# mika#2449 — Le checkout principal sali : le producteur est mesuré (mika-qa, `git checkout <ref> -- <paths>` par `run_shell`), la prescription du site B est un second producteur latent, et la prochaine occurrence doit être datée

## Problème

Le 2026-09-21, avant un rebuild propre (#2446), le checkout **main** de `mika/`
portait 15 fichiers non-committés — `site/*`, `scripts/check-landing-tokens.sh`,
`scripts/smoke-webhook-chain`, `skills/bundled/_shared/dispatch-lib.sh` + tests.
Le contenu n'avait rien d'unique : il était identique à `main` pour les deux
premiers (#2434, mergée), à la branche de #2435 pour le troisième, à
`main@7c6f787b` pour le quatrième (#2436). Le ticket en concluait que **trois
pilotes** (#1804, #2135, #1943) avaient écrit dans le checkout principal en plus
de leur worktree, et proposait trois candidats : (a) le cwd du pilote, (b) le
resume-cleanup de dispatch-lib opérant sur le mauvais répertoire, (c) le moteur
tournant avec `chdir=mika/` dont un pilote hériterait.

Impact réel et non contesté : un checkout principal sali **bloque les rebuilds**
(`git pull --ff-only` refuse), rencontré pendant #2446 et contourné par stash
(`eb032b95` → `rescue/main-sale-2026-09-21`). Classe santé-substrat. p2.

### M0 — Ce que l'hôte a rendu : le producteur est mika-qa, et il est nommé au tool_call près

Mesuré le 2026-09-22 depuis l'orchestrateur (accès à `~/.mika/data/mika.db`, que
le bac à sable de grooming n'a pas). La table `tool_calls` porte, **le même
jour, par le même agent, par le même geste**, les trois écritures du sinistre :

| Fichiers du stash | PR revue | `tool_calls.id` | UTC 2026-09-20 | Commande (`run_shell`, mika-qa) |
|---|---|---|---|---|
| `site/*`, `check-landing-tokens.sh` + test | #2434 (mika#1804) | `1835d8bb` | 17:22:52 | `cd ~/workspace/mika-platform/mika && git -C . checkout 146b536d… -- scripts/check-landing-tokens.sh scripts/test-check-landing-tokens.sh site/ && bash scripts/check-landing-tokens.sh` — **aucune restauration** |
| `smoke-webhook-chain` + test | #2435 (mika#2135) | `eba3682f` | 18:33:39 | `cd ~/workspace/mika-platform/mika && git checkout origin/fix/2135/… -- scripts/smoke-webhook-chain scripts/test-smoke-webhook-chain.sh && bash …; git checkout -- scripts/smoke-webhook-chain …` — restauration **inerte** (voir ci-dessous) |
| `dispatch-lib.sh` + `test-dispatch-lib.sh` | #2436 (mika#1943) | `930f5200` | 19:04:37 | `cd ~/workspace/mika-platform/mika && git fetch origin chore/1943/… && git checkout origin/chore/1943/… -- skills/bundled/_shared/dispatch-lib.sh skills/bundled/_shared/test-dispatch-lib.sh && make test-dispatch-lib` — **aucune restauration** |

Recoupement : les revues `mika-platform-qa` de #2434 / #2435 / #2436 sont
soumises à 17:22:49Z, 18:35:22Z et 19:06:02Z — à quelques secondes des
commandes. Les trois sessions (`321c4798`, `1799c97c`, `e2792a39`) sont des
sessions `github` de mika-qa avec `qa-review` chargé. #2435 n'était pas mergée
au moment du stash, ce qui est exactement pourquoi son fichier était « identique
à la branche » et non à `main`.

**Le mécanisme explique la signature à la lettre.** `git checkout <ref> -- <paths>`
écrit **l'index et l'arbre de travail** : d'où « staged », pas seulement
« modified ». Et la « restauration » de `eba3682f` — `git checkout -- <paths>` —
relit l'arbre **depuis l'index**, que la commande précédente vient d'écraser :
elle ne restaure rien, sans une erreur. Un modèle qui « nettoie derrière lui » de
cette façon laisse exactement ce que le stash a trouvé.

**Ce n'est pas un incident, c'est une classe, et elle est comptée.** Même geste
le 2026-09-15 (`d53b91e3`, mika-qa, `git checkout origin/feat/2310/… -- crates/…
docs/plans`) → stash `fab339fa` « leftover harnais #2310 » du même jour. Stash
`4cd73b33` du 2026-09-06 « staged dispatch-lib leak in main checkout ». Et le
2026-09-20 à 11:31Z (`fa92720d`) mika-qa a fait `git stash && git checkout
fix/1940/…` **dans le checkout principal** — ce qui a produit le stash
`8cdd76bb` « WIP on main », et aurait détaché le checkout de déploiement si git
n'avait pas refusé (branche déjà extraite ailleurs). Quatre stashs de secours sur
main en quinze jours, un seul acteur, un seul geste.

**Et le prompt le disait déjà.** `qa-review/system_prompt.md` prescrit, pour les
gardes, *« un worktree détaché jetable »* (§ 2B, lignes 196–210, avec la recette
`mktemp -d` + `git -C "$R" worktree add --detach`), et pour lire un fichier
d'une branche `git -C $MIKA_PLATFORM_DIR/<repo>/ show <branch>:<path>` (l. 277).
Le modèle a exécuté un script de la PR en l'**extrayant dans le checkout
partagé**, cinq fois au moins. C'est la classe que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` borne
(mika#2120 : neuf récurrences sous prompt contre zéro écrit à la main) — la
moitié structurelle manquait, et c'est elle que ce plan livre.

**Population mesurée pour dimensionner la garde** (`tool_calls` depuis le
2026-09-01, `run_shell`, commandes portant `git` et visant un checkout principal
sous `~/workspace/mika-platform/`, hors `.claude/worktrees`) : **2 972**
commandes, 2 888 de mika-qa, 84 de mika-dev. Formes que le prédicat refuse :
`checkout <ref>` 36, `checkout <ref> -- <paths>` 27, `stash` (nu/push/pop/drop)
31, `checkout --force` 2, `reset` 2, `merge` sans `--ff-only` 2, `checkout -b` 1
— **~101, soit 3,4 %, toutes de la classe du défaut ou de celle de mika#2107**.
Formes qu'il laisse passer : `show` 1 449, `diff` 1 265, `log` 363, `fetch` 341,
`worktree` 50, `branch` 21, `merge --ff-only` 6, `pull --ff-only` 2, `checkout
-- <paths>` 6, `stash list` 1, lectures diverses — ~2 870.

### M1 — Ce que la mesure RÉFUTE : un pilote sandboxé ne PEUT PAS salir le checkout principal

Mesuré le 2026-09-21 **depuis l'intérieur d'un pilote dispatché** (la session de
grooming moteur de ce ticket), sur le dispatch-lib en vigueur :

| Sonde | Résultat |
|---|---|
| `ls -a /data/workspace/mika-platform` | `.claude`, `claude-pilot`, `mika` — **rien d'autre** |
| `ls -a /data/workspace/mika-platform/mika` | `.git` **seul** — aucun fichier de l'arbre de travail |
| `command -v bwrap` (via le `--ro-bind /usr`) | `/usr/bin/bwrap` — **présent sur l'hôte** |

Le parent ne porte que les répertoires intermédiaires matérialisés par bwrap
pour ses binds, sur tmpfs. Trois faits de code complètent la mesure : l'unique
bind rw d'un arbre de travail est le worktree (`--bind "$WORKTREE_DIR"`) ;
`--chdir "$WORKTREE_DIR"` et `--clearenv` sont posés ; les **trois** sites de
lancement pilote passent par `_run_pilot_sandboxed` (`dispatch-lib.sh:2977`,
`:5662`, `:5781`). **Candidats (a) et (c) : réfutés** pour tout pilote contenu.
Et (c) est réfuté une troisième fois pour les **handlers exec** : le moteur les
lance avec `cmd.current_dir(skill_dir)` (`skills/executor.rs:848`), jamais avec
le cwd de mika-spirit — qui **est** bien `/data/workspace/mika-platform/mika`
(mesuré : `readlink /proc/<pid>/cwd`), sans que rien n'en hérite. Les écritures
de M0 passent par un `cd` **explicite** dans la commande.

M0 rend la réserve de cette mesure caduque : la voie pilote n'a pas seulement
été refermée structurellement, elle est **exclue par attribution**.

### M2 — Ce que la mesure DÉPLACE : dispatch-lib n'opère pas dans main, il y ENVOIE l'opérateur

Les **onze** opérations `git -C "$SUB_REPO_DIR"` de `dispatch-lib.sh` ont été
relevées une par une : `fetch` (2564, 2664), `worktree list` (2596),
`worktree remove` (2629, 2641, 2901), `ls-remote` (2663), `worktree add`
(2665, 2667, 2671, 2673). **Aucune ne mute l'arbre de travail du checkout
principal.** Le candidat (b), pris comme *opération*, est réfuté.

Mais il est **confirmé comme prescription** — un second producteur, latent, de
la même signature. Deux sites stashent un worktree sale, et leurs messages de
récupération divergent :

| Site | Ligne | Message |
|---|---|---|
| A — `_clean_worktree_for_rebase` | 1911 | `recover with: git -C ${wt} stash apply …` → **le worktree** |
| B — `_set_up_worktree` (relic) | 2615 | `recover with: git -C $SUB_REPO_DIR stash apply …` → **le checkout principal** |

Exécutée, la prescription du site B dépose le contenu d'un worktree, **non
committé**, dans le checkout principal. Le lecteur de ce message est un acteur
non contenu (l'opérateur sur un dispatch échoué, ou mika-dev lisant la sortie
d'un dispatch en échec — le tuyau stderr n'est lu que dans cette branche, D1).
**Ce n'est pas une faute de frappe.** Au site B, le worktree est **supprimé
quatorze lignes plus bas** (`worktree remove --force`, 2629) : au moment où
l'opérateur lit le message, le répertoire que le site A nommerait n'existe plus.
Remplacer `$SUB_REPO_DIR` par `$existing_wt` produirait une consigne qui échoue ;
le remède nomme le worktree **canonique** (`$WORKTREE_DIR`, créé juste après sur
la même branche) et dit explicitement de ne pas appliquer dans le checkout
principal. Aucune ligne de `tool_calls` ne montre cette prescription exécutée à
ce jour ; elle est corrigée parce qu'elle est un producteur **écrit**, et parce
que U2 coûte quinze lignes.

### M3 — Le trou trouvé en chemin : `mika/` est writable dans le sandbox, et son arbre est vide

Conséquence des binds mika#2141 : `/data/workspace/mika-platform/mika/`
**existe** dans le sandbox, est **writable** (tmpfs), porte un `.git`
fonctionnel et un arbre **vide**. `git status` y voit tout le dépôt comme
*supprimé* ; toute écriture y disparaît à la fin de la session, sans un mot.
Forme mika#2205. Réel, adjacent, **hors périmètre** (inverse du symptôme
traité) : § Suivi 1.

### M4 — Ce que la mesure change à l'attribution : la sonde DATE, `tool_calls` NOMME

La version précédente de ce plan refusait d'élire un producteur, au motif que
les traces étaient parties. Elles ne l'étaient pas : **`tool_calls` est la
surface d'attribution**, et elle a répondu en une requête. La doctrine mika#2026
(*la provenance est un fait estampillé par son producteur*) tient toujours — le
producteur ici **a** estampillé son geste, dans `tool_calls`, et c'est ce qui a
permis de le lire. Ce qui manquait n'était pas la provenance mais la **fenêtre**
: sans date d'apparition, la requête n'a pas de borne et l'enquête part de zéro.
D'où R3 : la sonde de saleté date, et sa ligne d'audit **porte la requête** qui
nomme (§ Sondes, sonde 1b).

## Requirements

- **R1** — Le message de récupération du site B ne prescrit plus le checkout
  principal ; il nomme le worktree canonique et dit de ne pas appliquer dans le
  checkout principal.
- **R2** — Aucun message de récupération de stash de `dispatch-lib.sh` ne nomme
  `$SUB_REPO_DIR`. Tenu par un scan de source à **allowlist vide**.
- **R3** — La saleté d'un checkout principal est **mesurée et datée** à chaque
  tick, sur un sink effectivement lu, avec une surface SQL, et la ligne d'audit
  porte la requête `tool_calls` qui attribue la fenêtre.
- **R4** — `run_shell` **refuse** toute invocation git qui vise un checkout
  principal de la plateforme (`<platform-dir>/<repo>`, `.git` répertoire) avec
  un verbe hors de l'allowlist « sûre pour un checkout de déploiement » (D7).
  Le refus nomme la cible, le verbe, le remède (worktree détaché jetable) et la
  dérogation. Les **trois commandes de M0, verbatim, sont refusées** ; les formes
  de synchronisation de mika-dev (`fetch`, `pull --ff-only`, `merge --ff-only`,
  `worktree *`, `branch *`, `push`) **passent**.
- **R5** — **Rien n'est nettoyé automatiquement.** Ni pop, ni stash, ni reset du
  checkout principal.
- **R6** — Fail-open de bout en bout pour les **sondes** (R3) : aucun signal ne
  refuse un dispatch ni ne fait échouer un tick. Pour la **garde** (R4) : un
  script de garde absent ou illisible laisse passer **et le dit** ; un prédicat
  qui tient **refuse** — c'est son objet, pas une panne.
- **R7** — Les noms d'événement et le jeton de refus sont un **format de fil** :
  site de définition unique, SOLE WRITER épinglé par scan de source.
- **R8** — La moitié intention : `qa-review` interdit explicitement
  `git checkout <ref> -- <paths>` / `git stash` / `git checkout <branche>` dans
  `$MIKA_PLATFORM_DIR/<repo>` et nomme le worktree détaché de § 2B comme
  **seul** lieu d'exécution d'un fichier de la PR. Ce n'est pas la garde ; c'est
  ce qui évite que la garde porte le trafic nominal.

## Décisions

### D1 — Le sink de R3 est le moteur, PAS dispatch-lib

`_set_up_worktree` tourne au niveau de `dispatch_claude_pilot`, **hors** de la
redirection `2>"$STDERR_FILE"` qui n'enveloppe que la ligne 2977. Sa stderr est
le `Stdio::piped()` de `spawn_long_running_exec`, que l'exécuteur lit
**uniquement** dans sa branche `if !status.success()` : sur un dispatch qui
réussit, les lignes n'atterrissent dans **aucun fichier** — le diagnostic écrit
de `pilot_push_guard` (Signal M, mika#2050). Troisième occurrence documentée de
cette classe (Signaux M, Q, S). R3 vit côté Rust, sink `$MIKA_SPIRIT_LOG_FILE` +
`audit_events`.

### D2 — L'hôte de R3 est `worktree_reaper` (mika#2420)

Il énumère **déjà** les checkouts via `MIKA_WORKTREE_REAP_REPO_DIRS` (défaut
`/data/workspace/mika-platform/mika` — exactement le checkout sali), tourne
**déjà** sur un tick, écrit **déjà** ses `log_audit_event`. Coût : un
`git status --porcelain` par checkout par tick. Zéro nouvelle variable, zéro
nouveau scan récurrent. Refusé : un cinquième scan ; et `MIKA_WIP_RESCUE_REPO_DIR`,
qui ne porte qu'un checkout.

**Pourquoi U3 reste justifié une fois U4 posé (F4).** U4 couvre le geste
casual en clair par `run_shell` — 100 % du trafic mesuré, et rien d'autre. U3
couvre ce que U4 ne voit **pas par construction** : un script du dépôt qui mute
(`make …`, un `scripts/*` — D7 le nomme comme hors couverture), un geste
humain, un producteur hors `run_shell` (la halte 4 des Sondes est écrite pour
cette population), et un bypass opérateur oublié (F3). Et U3 est ce qui **date**
la prochaine occurrence : sans lui, la requête d'attribution 1b n'a pas de
bornes, et la détection redevient le rebuild bloqué — c'est-à-dire, comme
mika#2107 l'a écrit pour le checkout détaché, *au pire moment*. Deux
responsabilités, deux unités : la garde empêche le geste, la sonde mesure
l'état. Coût : un `git status --porcelain` par tick, fail-open, dédupliqué.

### D3 — La sonde DATE, et sa ligne d'audit dit COMMENT nommer

Elle rapporte le checkout, le compte, les chemins (plafonnés), l'instant, et
**la requête d'attribution** (`tool_calls` bornée à la fenêtre entre deux
ticks, cf. § Sondes 1b) dans son `reasoning`. Elle **ne nomme aucun producteur
elle-même** : écrire « le dernier dispatch » serait la dérivation tardive que
mika#2368 refuse. La différence avec la rev 2 : on ne dit plus que
l'attribution est impossible, on dit où elle se lit.

### D4 — Déduplication sur `(checkout, empreinte de la liste)`, horizon 24 h

Un checkout sale le reste des jours ; une ligne par tick (144/j) déplacerait le
churn que mika#2131 borne. Un **changement** de liste ré-écrit. L'horizon de
24 h : sans lui, un checkout nettoyé puis re-sali n'écrirait rien la seconde
fois. **Checkout propre ⇒ zéro ligne** (mika#2131 AC7).

### D5 — Non bloquant, et l'asymétrie est écrite

Un checkout principal sale n'empêche **pas** un dispatch (le worktree est
ailleurs, les trois PR du sinistre ont abouti) ; il empêche un `pull --ff-only`,
un geste d'opérateur. Refuser le dispatch coucherait la boucle pour un défaut de
poste de build. Fail-open aussi sur la mesure : un `git status` illisible sort le
checkout de la population **et le dit** (R6).

### D6 — Le site de R4 est le handler `shell-exec`, qui appelle le parseur de mika#2107 — et la variable est lue AVANT le scrub

Trois sites possibles, un retenu.

- **Le handler `run.sh` de `crates/mika-agent/templates/skills/shell-exec/`**
  (retenu). C'est la surface d'exécution de `run_shell`, et elle porte déjà
  deux gardes lexicales de la même famille : L3 mika#1957 (CLIs gated) et le
  containment d'egress mika#1991 — dont la doctrine s'applique mot pour mot :
  *« construire l'incapacité, pas persuader le modèle »*. Le trafic mesuré
  (M0) est **casual et incidentel** — un `cd … && git checkout …` en clair —
  c'est-à-dire la population que mika#1957 a définie comme celle qu'une garde
  handler achète.
- **`scripts/guard-shared-checkout` (mika#2107)** fournit le parseur : suivi de
  `cd`/`pushd`/`popd`, `-C`, `--work-tree`, `--git-dir`, chaînes `&&`/`;`/`|`,
  normalisation des chemins, allowlist de lecture (`verb_is_read_only`,
  `verb_subform_is_read`), journal scrubbé — et **un harnais en CI**
  (`scripts/test-guard-shared-checkout.sh`, `ci.yml:283`). Réécrire ce parseur
  en Rust dans `tool_execution/dispatch.rs` pour un seul appelant est écarté :
  c'est un parseur de grammaire shell, et deux copies en deux langues d'un
  prédicat de sûreté sont une divergence programmée (mika#2158, `grooming_marker`).
- **Un hook git** est écarté pour les raisons que #2107 a déjà écrites en tête
  de script (`pre-checkout` n'existe pas ; taux d'installation des hooks
  mesuré à zéro).

**Pourquoi #2107 ne suffit pas tel quel.** Son T1 exempte **par construction**
toute session non enracinée dans un worktree lié — c'est voulu, pour ne pas
casser `/mika-platform-sync-main`. mika-qa n'a **pas** de worktree : son
`run_shell` tourne depuis `skill_dir`, et la commande `cd` dans le checkout
partagé. Il est donc hors population de #2107 quel que soit son geste. D'où un
**second mode**, `--decide-primary <platform-dir> <cwd> <command>`, qui partage
le parseur et remplace T1/T4 par : *la cible effective est un checkout
principal sous `<platform-dir>`* (`.git` est un **répertoire**, pas le fichier
`gitdir:` d'un worktree lié — la même distinction que T1, dans l'autre sens).
Les deux modes coexistent dans un fichier ; ils ne se substituent pas : #2107
protège une session-worktree contre le partagé, R4 protège le partagé contre un
agent sans worktree.

**Le chemin est résolu AVANT le scrub, sinon il n'existe plus.** `run.sh`
commence par `unset` de toutes les `MIKA_*` (secrets). `MIKA_PLATFORM_DIR` en
fait partie. La garde lit donc `${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}`
**au-dessus** de la boucle de scrub, le normalise par `pwd -P` (le symlink
`~/workspace` → `/data/workspace` est celui que M0 traverse), et le passe en
argument — jamais via l'environnement du sous-processus. Même résolution que
`dispatch-lib.sh:7401` et `deploy-mika/handlers:68`, à dessein : un troisième
défaut divergent serait le bug de M0 avec un chemin de plus.

**Fail-open sur l'absence du script, et c'est dit.** Le script vit dans le
checkout `<platform-dir>/mika/scripts/`. Sur un tenant sans plateforme il n'y a
rien à protéger : silence. Sur un poste **avec** plateforme et **sans** script
(checkout antérieur au fix, déplacé), `run.sh` émet une ligne stderr
`shell-exec: shared-checkout guard not found at <path> (fail-open)` — la seule
chose qui distingue « rien à refuser » de « rien n'est armé » (mika#2107, même
phrase). Ce coût est accepté pour la même raison que #2107 : un fail-closed
coucherait `run_shell` pour tout agent d'un poste mal configuré.

**La dérogation est le même levier, lu au même endroit — et elle est DITE à
chaque invocation.** `MIKA_GUARD_SHARED_CHECKOUT=0` désarme aussi ce mode (une
variable, un geste, journalisé) ; elle est lue avant le scrub, comme le chemin.
Quand elle est posée, `run.sh` émet sur stderr, **à chaque appel**,
`shell-exec: shared-checkout guard disarmed by MIKA_GUARD_SHARED_CHECKOUT=0
(operator override)` — la même discipline que la ligne « script absent » : un
bypass posé pour une intervention puis oublié serait sinon une garde
silencieusement absente pour tous les agents, indistinguable d'une garde qui n'a
rien eu à refuser (F3 ; mika#2107, mika#2329 : pour un interrupteur, la vivacité
*est* l'information). Une ligne par appel plutôt qu'au premier : le handler est
un process par appel, il n'a pas d'état « premier ».

### D7 — Le prédicat de R4 est l'INVARIANT d'un checkout de déploiement, pas « toute mutation »

Refuser toute écriture git dans le checkout principal **casserait la boucle** :
mika-dev y synchronise main par `run_shell` (`git fetch origin && git merge
--ff-only origin/main`, mesuré 09-09, 09-16, 09-17, 09-20 ; `pull --ff-only`
09-09), y enlève des worktrees (`worktree remove`, 09-06) et y supprime des
branches locales (`branch -D`) — et `deploy-mika` ne fait aucun de ces gestes
(son handler accepte le checkout principal comme cwd, l. 78, sans le
synchroniser). Ce sont des gestes de maintenance **qui préservent** ce qu'un
checkout de déploiement doit être : *arbre = index = HEAD, HEAD sur l'histoire
first-parent de `origin/main`*.

Le prédicat est donc : **refuser les verbes qui peuvent rompre cet invariant,
admettre ceux qui le préservent.**

| Admis (préservent l'invariant) | Refusés (peuvent le rompre) |
|---|---|
| toute lecture (`verb_is_read_only` de #2107 : `show`, `diff`, `log`, `status`, `rev-parse`, `ls-files`, `merge-base`, `stash list`/`show`, …) | `checkout <ref>`, `checkout <ref> -- <paths>`, `checkout -b/-B/--force/--detach`, `switch`, `restore` (`--source`, `--staged`) |
| `fetch`, `ls-remote`, `remote`, `push` (remote seul) | `stash` nu, `push`, `pop`, `apply`, `drop`, `clear` |
| `pull --ff-only`, `merge --ff-only` (**avec** le drapeau, positionnel) | `pull` / `merge` **sans** `--ff-only`, `rebase`, `cherry-pick`, `revert`, `am`, `apply` |
| `worktree add/remove/prune/list`, `branch` **sans** `-f`/`--force`/`-m`/`-M`/`--move`/`-c`/`-C`/`--copy` (liste, `-v`, `--show-current`, `-a`/`-r`, `-d`/`-D` : aucune ne déplace une ref sous HEAD), `tag`, `prune`, `gc` | `branch -f/-m/-M/-c/-C` (**F2** : `branch -f main <sha>` déplace la ref sous HEAD et rompt l'invariant sans toucher l'arbre — population mesurée : 0, refusé **par construction**, pas sur la population passée), `reset` (toutes formes), `clean`, `add`, `rm`, `mv`, `commit`, `update-index` |
| `checkout -- <paths>` (relit l'index : inerte si l'invariant tient) | tout verbe inconnu du tableau (**fail-closed sur l'inconnu**, parce que c'est une garde d'écriture et que la lecture est déjà énumérée) |

`--ff-only` est cherché dans les arguments du verbe, pas dans la ligne : un
`git merge origin/main --no-edit` (mesuré 09-20, `mika-dev`) est refusé, et
c'est le bon résultat — un merge non-ff sur le checkout de déploiement produit
exactement l'histoire que `make deploy` ne sait pas relire.

**Ce que ce tableau NE couvre PAS, nommé.** Un script du dépôt qui mute
(`make …`, `scripts/mika-platform-sync-main`) n'est pas parsé — la garde lit une
ligne de commande, pas ce qu'un script fait. C'est la même limite que #2107 a
écrite pour elle-même (*« défend contre l'accident, pas contre un adversaire »*).
La population mesurée passe intégralement par `git` en clair.

### D8 — Le refus nomme la cible, le verbe, le remède ET la dérogation

Modèle mika#1475 / mika#2107 : *un refus qui ne nomme pas sa levée est un refus
qu'on contourne au jugé.* Le remède est la recette que le prompt prescrit déjà,
recopiée : `R=<platform-dir>/<repo>; W=$(mktemp -d); git -C "$R" worktree add
--detach "$W" <ref> && … ; git -C "$R" worktree remove --force "$W"`. Le refus
sort sur **stdout** du handler avec exit ≠ 0, donc il atterrit dans
`tool_calls.output` préfixé `Exit code: N` — ce qui en fait une **surface SQL**
sans nouvel événement (§ Sondes 3).

### D9 — Les signaux de containment pilote (U4 de la rev 2) sortent du plan

La rev 2 livrait deux émissions (« bwrap absent », kill-switch désarmé) et un
contrôle positif pour rendre détectable un démarrage pilote non contenu. Cette
hypothèse est réfutée deux fois (M1 : structurellement ; M0 : par attribution).
Livrer trois jetons et un scan pour une voie qui n'est pas celle du défaut est
un élargissement de périmètre sur un ticket p2 santé-substrat. Le trou du
kill-switch muet est réel et **nommé** (§ Suivi 2), avec sa précondition.

## Scope Boundaries

**Dans le périmètre** : le message de récupération du site B ; le scan de parité ;
la sonde de saleté dans `worktree_reaper` ; le mode `--decide-primary` de
`guard-shared-checkout` et son branchement dans `shell-exec` ; les tests
(harnais existant étendu + test du branchement) ; la ligne de prompt `qa-review` ;
la documentation des surfaces.

**Hors périmètre, délibérément** :

- **Nettoyer ou committer le checkout principal.** R5. Décision Vincent (« ne pas
  pop le stash »).
- **Un outil `sync_main` sanctionné pour mika-dev.** L'allowlist de D7 est une
  *tolérance* ; l'incapacité serait un handler dédié qui ne sait faire que
  `fetch` + `merge --ff-only`. Précondition : mesurer si les 84 commandes
  mika-dev sur main ont un autre besoin que la synchro (§ Suivi 3).
- **L'écriture perdue dans le tmpfs `mika/`** (M3).
- **Les binds mika#2141** et le fallback non contenu (D9, § Suivi 2).
- **Le comportement de `mika-qa` sur `mika-cloud` / `claude-pilot`** au-delà de
  ce que R4 couvre par construction (`checkout --force feat/245/…` mesuré
  2026-09-17 dans `mika-cloud` : refusé par le même prédicat, sans ligne
  supplémentaire).

## Implementation Units

### U1 — La prescription réparée (R1)

`skills/bundled/_shared/dispatch-lib.sh` ~2615. Le message nomme `$WORKTREE_DIR`
et porte une butée explicite : ne pas appliquer dans le checkout principal.
Marqueur `mika#2449` et une ligne de raison (pourquoi pas `$existing_wt` : il
est supprimé quatorze lignes plus bas).

### U2 — Le scan de parité (R2, D7 de la rev 2)

`skills/bundled/_shared/test-dispatch-lib.sh`, style `assert_*`. Extraire les
lignes de `dispatch-lib.sh` contenant à la fois `stash apply` et `recover with`,
refuser toute occurrence de `SUB_REPO_DIR`. Allowlist vide + **contrôle de bonne
foi** : le scan doit trouver ≥ 2 sites (sinon il est vert parce qu'il ne regarde
rien — classe mika#2205).

### U3 — La sonde de saleté (R3, R6, D2, D3, D4)

`crates/mika-agent/src/worktree_reaper.rs`. Dans la boucle par checkout de
`reap_terminal_worktrees` (1065), **après** la garde `repo_dir.join(".git").exists()`
qui émet `worktree_reap_no_checkout` (1098) et avant le `git worktree list` :
`git status --porcelain` borné en temps, fail-open. Fonction **pure** séparée
pour la décision (propre / sale+liste / illisible), testable sans git. Émission
`main_checkout_dirty` (WARN + `audit_events`, `tool_name = 'main_checkout_dirty'`,
`target_key = <repo_dir>`), dédupliquée par `(checkout, empreinte)` sur 24 h
via `audit_events`, modèle de l'exclusion mika#2131. Chemins plafonnés (20,
comme `DIRTY_FILES` en 3714), **jamais** de contenu de fichier. Le `reasoning`
porte la requête d'attribution de § Sondes 1b, avec la borne basse = l'instant
du dernier tick propre (lu depuis la dernière ligne d'audit du même checkout,
ou « inconnu » si aucune).

**Le placement est porteur.** En tête de boucle, la sonde sonderait un chemin
sans dépôt : en production conteneurisée, chaque checkout configuré produirait
une émission « illisible » à chaque tick, pour toujours. « Pas de checkout à ce
chemin » et « `git status` illisible sur un checkout réel » sont **deux** états,
et seul le second est un signal de R3.

### U4 — La garde `run_shell` (R4, R6, D6, D7, D8)

1. `scripts/guard-shared-checkout` : mode `--decide-primary <platform-dir>
   <cwd> <command>`. Réutilise le parseur (résolution de la cible effective par
   ligne de commande), remplace T1/T4 par « cible = checkout principal sous
   platform-dir » (`.git` répertoire), et T3 par l'allowlist de D7
   (`verb_is_deploy_safe`, qui **compose** `verb_is_read_only` plutôt que de le
   dupliquer). Sortie : 0 allow ; 1 deny avec le motif D8 sur stdout, jeton
   `REFUS (shared-checkout-guard, mika#2449)`. Journal : même fichier que #2107,
   ligne préfixée `mode=primary`. Dérogation : `MIKA_GUARD_SHARED_CHECKOUT=0`.
2. `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` : **avant**
   la boucle de scrub, résoudre `PLATFORM_DIR` (D6) et `GUARD_BYPASS` ; après le
   parsing de `COMMAND`/`WORKDIR`, si `$PLATFORM_DIR/mika/scripts/guard-shared-checkout`
   existe, l'appeler en `--decide-primary "$PLATFORM_DIR" "${WORKDIR:-$HOME}"
   "$COMMAND"` ; sur 1, imprimer le motif et `exit 1`. Sinon, si `$PLATFORM_DIR`
   existe, la ligne stderr fail-open de D6. Marqueur `mika#2449`.
3. `scripts/test-guard-shared-checkout.sh` : section `--decide-primary` avec
   **les trois commandes de M0 verbatim → deny** (plus `fa92720d` et
   `d53b91e3`), les formes mika-dev admises → allow (`fetch && merge --ff-only`,
   `pull --ff-only`, `worktree remove … && push origin --delete`, `branch -D`),
   `merge origin/main --no-edit` → deny, `branch -f main <sha>` et `branch -m
   main autre` → deny (F2), `branch -D fix/x` et `branch --show-current` →
   allow, un `checkout <ref> -- <paths>` **dans un
   worktree lié** sous `.claude/worktrees/` → allow (hors population), un chemin
   hors platform-dir → allow, `-C <principal>` sans `cd` → deny, `pushd` → deny.
4. `scripts/test-shell-exec-guard.sh` (nouveau, câblé dans `Makefile` +
   `ci.yml` à côté de `test-guard-shared-checkout.sh`) : exécute `run.sh` avec
   un JSON `{"command": …}` sur un `MIKA_PLATFORM_DIR` de fixture (deux dépôts
   temporaires : un principal, un worktree lié) : les trois commandes → sortie
   commence par `REFUS (shared-checkout-guard`, exit 1, **et l'arbre du
   principal est intact** (le test git est le contrôle qui compte) ; une lecture
   → exit 0 ; plateforme absente → exit 0 sans ligne ; plateforme présente sans
   script → exit 0 **avec** la ligne fail-open ; `MIKA_GUARD_SHARED_CHECKOUT=0`
   posé → une des trois commandes de M0 passe (exit 0) **et** la ligne
   « disarmed … (operator override) » est sur stderr (F3).

### U5 — Les noms de fil et leur SOLE WRITER (R7)

`main_checkout_dirty` : constante à site unique côté Rust ; scan de source
refusant un second écrivain (modèle `mika2420_le_tool_name_daudit_a_un_seul_writer`,
`worktree_reaper.rs:1869`, contrôle de bonne foi `scanned > 0` inclus). Le jeton
de refus `REFUS (shared-checkout-guard, mika#2449)` : épinglé par le harnais de
U4.3 (une assertion sur la chaîne exacte), et un grep de U4.4 vérifie que le
handler ne l'écrit **pas** lui-même (il relaie la sortie du script).

### U6 — La moitié intention (R8)

`skills/bundled/qa-review/system_prompt.md`, § 2B et § « Never » : une ligne
qui nomme la classe (`git checkout <ref> -- <paths>`, `git stash`, `git
checkout <branche>` dans `$MIKA_PLATFORM_DIR/<repo>`), dit qu'elle est
**refusée par `run_shell`** depuis mika#2449, et renvoie à la recette 2B pour
exécuter un script de la PR. Pas une troisième reformulation de « utilise un
worktree » : une phrase qui explique le refus que le modèle va rencontrer, pour
qu'il ne le contourne pas au jugé.

### U7 — Documentation

`mika/CLAUDE.md` : une entrée au voisinage de mika#2420 (surfaces opérateur,
régimes attendus, haltes, requête d'attribution) et une ligne dans le paragraphe
mika#2107 pour le second mode. `docs/solutions/workflow-issues/` : le
mécanisme de M0 (`checkout <ref> -- <paths>` + restauration inerte) — la
leçon compoundable est que **la « restauration » `git checkout -- <path>`
relit l'index, pas HEAD**.

## Fire-Disposition

Cinq livrables sont de classe détecteur (mika#1574). Populations **comptées**
dans l'arbre à `b74f7e7c` et dans `mika.db` au 2026-09-22, pas supposées.

| Unit | Classe | Bloquant ? | « Préexistant » désigne |
|---|---|---|---|
| U2 | scan de source (shell) | oui — `test-dispatch-lib.sh` | les lignes de `dispatch-lib.sh` |
| U5 | scan de source (Rust) | oui — `cargo test` | `crates/mika-agent/src/` |
| U3 | détecteur d'exploitation | **non** — fail-open | l'état de l'hôte au premier tick |
| U4 | **garde d'exécution** | **oui, par commande** — c'est son objet | les gestes que les agents font aujourd'hui |

### U2 — deux sites, une violation à la pose, zéro au land

Sites `1911` (conforme) et `2615` (violation), corrigée par U1 **dans le même
commit**. **Disposition (a), allowlist livrée vide.** Résolution quand il tire :
corriger le message, jamais ajouter une entrée. Contrôle de bonne foi ≥ 2 sites.

### U3 — sonde de saleté

Non bloquant. Le checkout est attendu **propre** au déploiement (le sinistre a
été contourné par stash, et `git status --short` sur `mika/` rend zéro ligne au
2026-09-22) ; AC4 est le contrôle négatif. **Disposition (a), aucun checkout
exempté** — un checkout sale au premier tick est le **livrable** (AC3), pas une
panne. Hors population par construction : `repo_dir` sans `.git`
(`worktree_reap_no_checkout`). Le troisième état (illisible) est un signal
nommé, jamais « propre ».

### U4 — garde `run_shell` : ~101 gestes sur 2 972 seraient refusés, et c'est le résultat voulu

**Population comptée (M0)** : sur 2 972 commandes depuis le 01/09, le prédicat
refuse ~101 (3,4 %) : 27 `checkout <ref> -- <paths>` (le défaut), 36 `checkout
<ref>` + 3 `--force/-b` (classe mika#2107, dont le détachement du checkout de
déploiement), 31 `stash` (dont 6 `pop` déposant du contenu étranger), 2 `reset`,
2 `merge` non-ff. **Zéro de ces gestes n'est une opération que la boucle a
besoin de faire dans le checkout principal** : chacun a un équivalent dans un
worktree détaché ou une forme `--ff-only`. Les 2 870 restants passent.

**Disposition : (a) allowlist nommée — et elle N'EST PAS vide, à dessein.** C'est
la table de D7. Chaque entrée est un verbe qui **préserve l'invariant** du
checkout de déploiement, pas une exemption d'un geste fautif. La différence
avec U2/U5 est de nature : là, une entrée serait une dispense ; ici, l'allowlist
**est** le prédicat (une garde d'écriture sans liste admise refuserait `fetch`).
**Résolution quand il tire sur un besoin légitime** : ajouter la **forme** du
verbe (jamais l'agent, jamais le chemin) avec une ligne disant en quoi elle
préserve l'invariant ; si elle ne le préserve pas, la réponse est un worktree
détaché, pas une entrée. La dérogation `MIKA_GUARD_SHARED_CHECKOUT=0` est un
geste d'opérateur journalisé, hors d'atteinte du modèle (le handler scrubbe
l'environnement ; la variable est lue avant, depuis l'env du **service**).

**Fail-closed sur un verbe inconnu, fail-open sur un script absent** — et les
deux sont raisonnés (D6, D7) : la lecture est énumérée, donc l'inconnu est une
écriture ; l'absence de script est un déploiement, pas une commande.

**Contrôle négatif obligatoire, vu rouge (V5)** : le harnais de U4.4 doit
**échouer** quand le branchement dans `run.sh` est retiré — sinon un handler
qui n'appelle rien est vert exactement comme un handler qui refuse
(`feedback_verify_pipeline_passes_without_the_fix`).

### U5 — SOLE WRITER

`main_checkout_dirty` : zéro écrivain dans l'arbre. **(a), allowlist vide.**
Résolution quand il tire : (c) halt-and-surface — un second écrivain rendrait la
requête opérateur de AC3 inexacte en silence.

### Ce que cette section ne fait PAS

Aucun `#[ignore]`, aucune période de grâce, aucun nettoyage préalable (R5).

## Verification Contract

1. **U2, rouge prescrit** : réintroduire `$SUB_REPO_DIR` dans le message du site
   B → le scan rougit ; corriger → vert.
2. **U2, bonne foi** : ≥ 2 sites `recover with`.
3. **U3, fonction pure** : propre → rien ; sale → une émission (compte, chemins,
   requête d'attribution dans `reasoning`) ; illisible → signal nommé, jamais
   « propre ». **3b, placement** : `repo_dir` sans `.git` →
   `worktree_reap_no_checkout` et **aucune** émission de saleté.
4. **U3, déduplication** : même liste sur deux ticks → une ligne ; liste changée
   → deux ; > 24 h → ré-écriture. **Non-blocage** : un checkout sale ne change ni
   le verdict de fauche ni le compte fauché.
5. **U4, les trois commandes de M0 verbatim → refus**, avec le jeton, la cible,
   le verbe, le remède et la dérogation dans le motif ; **l'arbre du principal
   de fixture est intact après l'appel** (`git status --porcelain` vide,
   `HEAD` inchangé). **Rouge-avant** : retirer l'appel dans `run.sh` → le
   harnais U4.4 rougit.
6. **U4, allowlist** : `fetch && merge --ff-only origin/main`, `pull --ff-only`,
   `worktree remove … && push origin --delete <b>`, `branch -D`, `show`, `diff`,
   `log`, `stash list`, `checkout -- <paths>` → allow. `merge origin/main
   --no-edit`, `stash`, `stash pop`, `reset --hard`, `checkout <ref>`,
   `checkout --force <ref>`, `branch -f main <sha>` → deny. Terme par terme (`feedback_red_before_control_is_term_by_term`).
7. **U4, hors population** : même `checkout <ref> -- <paths>` dans un worktree
   lié `.claude/worktrees/x/mika` → allow ; dans `/tmp/autre-depot` → allow ;
   plateforme absente → allow sans ligne ; plateforme présente sans script →
   allow **avec** la ligne fail-open ; dérogation posée → allow **avec** la
   ligne « disarmed » (F3).
8. **U4, D6** : `run.sh` lit `MIKA_PLATFORM_DIR` avant le scrub — test : poser
   la variable sur un chemin de fixture, vérifier que la garde vise ce chemin
   (et non `$HOME/workspace/…`).
9. `make verify-bundled-skills`, `cargo test -p mika-agent`, `cargo clippy`,
   `bash skills/bundled/_shared/test-dispatch-lib.sh`,
   `bash scripts/test-guard-shared-checkout.sh`, `bash scripts/test-shell-exec-guard.sh`.
10. **Les dispositions déclarées sont celles posées** : allowlists U2/U5
    vides ; allowlist U4 = la table D7, ni plus ni moins ; aucun `#[ignore]`.

## Definition of Done

- Le site B ne nomme plus le checkout principal ; le scan de parité est vert et
  a été vu rouge.
- La sonde de saleté est branchée dans le faucheur, dédupliquée, fail-open,
  n'émet rien sur un checkout propre, et sa ligne porte la requête d'attribution.
- `run_shell` refuse les trois commandes du sinistre et admet les formes de
  synchronisation de mika-dev ; le harnais a été vu rouge sans le branchement.
- `qa-review` nomme le refus et le remède.
- `CLAUDE.md` porte les surfaces, les régimes, les haltes et la requête.
- Aucun nettoyage automatique (R5), aucune variable d'environnement créée.
- Les dispositions du § Fire-Disposition sont celles posées.
- **Le titre du ticket est corrigé** pour nommer le producteur mesuré, avec un
  commentaire d'avis d'édition (convention mika#2169/#2158 — rectification
  corps **et** titre quand le plan réfute le ticket). Livré par l'orchestrateur
  au grooming, après la pose du callout `Branch:` (le slug est dérivé du
  callout en priorité ; le titre ne le fait plus dériver une fois le callout
  posé — mika#844).

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification Contract. Le corps
du ticket a été mis à jour par l'orchestrateur avec la mesure M0 le 2026-09-22 ;
le titre l'est au même grooming (AC9), et non « plus tard » (F1).

- **AC1** — La prémisse est tranchée par écrit, mesure à l'appui : (a) et (c)
  réfutés (M1) ; (b) réfuté comme opération, confirmé comme prescription (M2) ;
  **le producteur du sinistre est nommé par ses `tool_calls.id`** (M0), et la
  classe est comptée (n ≥ 5).
- **AC2** — `git -C <checkout-principal> stash apply` n'est plus prescrit nulle
  part par `dispatch-lib.sh`, et un scan à allowlist vide l'empêche de revenir.
- **AC3** — Un checkout principal sale produit, au tick suivant, une ligne de
  journal **et** une ligne `audit_events` datées, nommant le checkout, le
  compte, les chemins (plafonnés) et la requête d'attribution — jamais un
  producteur.
- **AC4** — Un checkout principal propre produit **zéro** ligne.
- **AC5** — Les trois commandes de M0, rejouées verbatim par `run_shell`, sont
  **refusées** avec un motif qui nomme la cible, le verbe, le remède et la
  dérogation, et l'arbre du checkout principal reste intact.
- **AC6** — `fetch`, `pull --ff-only`, `merge --ff-only`, `worktree *`,
  `branch *`, `push` et toute lecture **passent** dans le checkout principal ;
  la même classe de geste dans un worktree lié ou hors plateforme passe.
- **AC7** — Rien n'est nettoyé, committé ni stashé automatiquement ; aucun
  dispatch n'est refusé et aucun tick ne peut échouer à cause des sondes.
- **AC8** — `qa-review` nomme le refus et renvoie au worktree détaché de § 2B
  pour exécuter un fichier de la PR.
- **AC9** — Le titre de mika#2449 nomme le producteur mesuré (mika-qa,
  `git checkout <ref> -- <paths>` via `run_shell`) et non « 3 pilotes », et le
  ticket porte un commentaire d'avis d'édition datant la rectification
  (corps + titre). Vérifiable par `gh issue view 2449 --json title`.

## Sondes post-déploiement, et leurs haltes

```bash
# 1a. Le checkout principal est-il sale, et depuis quelle fenêtre ?
grep main_checkout_dirty "$MIKA_SPIRIT_LOG_FILE" | jq -c '{repo_dir, file_count, files}'
```
```sql
SELECT target_key, created_at, reasoning FROM audit_events
 WHERE tool_name = 'main_checkout_dirty' ORDER BY created_at DESC;
```
```sql
-- 1b. QUI a écrit dans la fenêtre ? (la requête que la ligne d'audit recopie,
--     bornes = dernier tick propre .. tick sale ; c'est celle qui a résolu M0)
SELECT id, agent_id, session_id, created_at, substr(input, 1, 200)
  FROM tool_calls
 WHERE tool_name = 'run_shell'
   AND created_at BETWEEN '<dernier tick propre>' AND '<tick sale>'
   AND input LIKE '%mika-platform/mika%'
   AND (input LIKE '%checkout %--%' OR input LIKE '%stash%' OR input LIKE '%reset%'
        OR input LIKE '%merge%' OR input LIKE '%pull%')
 ORDER BY created_at;
```
```sql
-- 2. La garde mord-elle ?  (régime attendu : NON VIDE les premiers jours,
--    décroissant ensuite — chaque ligne est un geste de la classe M0 arrêté)
SELECT agent_id, count(*) FROM tool_calls
 WHERE tool_name = 'run_shell' AND output LIKE '%REFUS (shared-checkout-guard, mika#2449)%'
 GROUP BY 1;
```
```bash
# 3. Journal de la garde (même fichier que mika#2107)
grep 'mode=primary' "${MIKA_HOME:-$HOME/.mika}/state/shared-checkout-guard.log" | tail
# 3b. CONTRÔLE POSITIF — la garde est-elle seulement branchée sur ce binaire ?
grep -c 'guard-shared-checkout' ~/.mika/skills/shell-exec/handlers/run.sh   # attendu : ≥ 1
grep 'shared-checkout guard not found' "$MIKA_SPIRIT_LOG_FILE"               # attendu : vide
grep 'shared-checkout guard disarmed'  "$MIKA_SPIRIT_LOG_FILE"               # attendu : vide hors intervention
```

**Halte 1 — la sonde 1a est vide pendant que le checkout est visiblement sale.**
Ne pas élargir le prédicat : lire `MIKA_WORKTREE_REAP_REPO_DIRS`, puis vérifier
que le binaire déployé porte le correctif (classe mika#2340).

**Halte 2 — la sonde 2 est vide ET la sonde 3b rend 0.** La garde n'est pas
dans la bibliothèque seedée : `~/.mika/skills/` est une projection du binaire
(mika#2340) ; rebuild → seed → relire. Zéro refus ne vaut « sain » que si 3b est
non nul.

**Halte 3 — la sonde 2 porte du trafic soutenu sur un même agent après une
semaine.** Le modèle contourne au jugé ou le prompt (U6) n'atteint pas ce
chemin : lire les commandes refusées **avant** de toucher à l'allowlist. Un
refus répété sur une forme légitime est une entrée D7 à ajouter avec sa raison ;
un refus répété sur `checkout <ref> -- <paths>` est le prompt à relire, pas la
garde à assouplir.

**Halte 4 — un checkout sale réapparaît (1a non vide) avec 2 vide.** Le
producteur n'est pas `run_shell` : la sonde 1b **sans le filtre `tool_name`**
puis, si vide, la classe humaine/orchestrateur (mika#2107 couvre les sessions
Claude Code ; un `cm`/spawn hors hook est possible). C'est là que le journal de
mika#2107 se lit à côté de celui-ci.

**Halte 5 — la ligne fail-open apparaît (3b, seconde commande).** Le script
n'est pas là où le handler le cherche : `MIKA_PLATFORM_DIR` du **service** (pas
d'un shell) ou checkout déplacé. Réparer le déploiement, pas le handler.

**Ce que ce travail n'achète pas.** Il ne dit pas qui a sali le checkout le
09-06 ni le 09-15 au-delà de ce que `tool_calls` porte encore (la table est
élaguée) ; il ne couvre pas un script du dépôt qui mute ; et comme toute garde
lexicale, il achète le geste casual — le seul mesuré — pas l'adversaire.

## Suivi (hors périmètre, nommé)

1. **L'écriture perdue dans le tmpfs `mika/`** (M3) — classe mika#2205.
   Préalable : établir qu'un pilote y va réellement.
2. **Le démarrage pilote non contenu est muet** (rev 2 U4 : `_pilot_sandbox_enabled`
   rend 1 sans un `echo` ; message 926 non ancré ; pas de contrôle positif).
   Réel, hors de ce défaut. Préalable : une mesure qui montre un dispatch ayant
   tourné sans bwrap — aujourd'hui aucune.
3. **Un handler `sync_main` pour mika-dev** — transformer la tolérance D7
   (`merge --ff-only` admis) en incapacité (un outil qui ne sait faire que ça).
   Préalable : relire les 84 commandes mika-dev sur main et vérifier qu'aucune
   n'a un autre besoin.
4. **Uniformiser les dix messages `[dispatch-lib]` sur `dispatch-lib: `** —
   découvert en comptant (rev 2). Ne répare aucune sonde ; hors d'un ticket
   santé-substrat.
5. **Un `pull --ff-only` de l'orchestrateur qui dit ce qui le bloque** —
   ergonomie, pas garde.

## Revision history

- **rev 5 (2026-09-22, reprise pré-implémentation)** : `origin/main` fusionné
  dans la branche (`4bd654fe`, apporte #2439 et #2448 ; `dispatch-lib.sh`
  +216 lignes). Ancres de ligne re-mesurées et mises à jour : site A `1695`→`1911`,
  site B `2399`→`2615`, `worktree remove` du site B `2413`→`2629`, sites
  `_run_pilot_sandboxed` `2761/5434/5553`→`2977/5662/5781`, les onze
  `git -C "$SUB_REPO_DIR"` renumérotées, `DIRTY_FILES` `3486`→`3714`. Ancres
  Rust (`reap_terminal_worktrees` 1065, `worktree_reap_no_checkout` 1098,
  sole-writer 1869), `executor.rs:848`, `ci.yml:283`, `dispatch-lib.sh:7401`
  et `deploy-mika/handlers/run.sh:9-10` inchangées. Aucun contenu de décision,
  d'unité, de contrat ni d'AC modifié.
- **rev 4 (2026-09-22)** : addressed mika-arch first-pass (session
  `d956fb91`, ITERATE). **F1 (BLOCKING)** — la correction du **titre** du
  ticket est un livrable, pas une promesse : DoD + **AC9** + commentaire d'avis
  d'édition (convention mika#2169/#2158). **F2** — `branch` restreint dans la
  table D7 : `-f/-m/-M/-c/-C` refusés (déplacement de ref sous HEAD), cas
  `branch -f main <sha>` → deny ajouté aux harnais U4.3 et V6. **F3** — la
  dérogation `MIKA_GUARD_SHARED_CHECKOUT=0` est **dite** à chaque appel
  (ligne stderr « disarmed … (operator override) »), cas ajouté à U4.4/V7, grep
  ajouté à la sonde 3b. **F4** — la justification de U3 après U4 est écrite
  dans D2 (couvre script du dépôt, geste humain, hors-`run_shell`, bypass
  oublié ; date la prochaine occurrence). Aucun AC affaibli.
- **rev 3 (2026-09-22, grooming orchestrateur)** : **le producteur est mesuré et
  nommé** (M0 : `tool_calls` `1835d8bb`, `eba3682f`, `930f5200` — mika-qa,
  `git checkout <ref> -- <paths>` via `run_shell`, recoupé aux revues QA de
  #2434/#2435/#2436 à la seconde ; classe n ≥ 5 avec `d53b91e3` 09-15 et les
  stashs `fab339fa`/`4cd73b33`/`8cdd76bb`). M4 réécrit (« aucun n'est élu » →
  « `tool_calls` nomme, la sonde date »), D3 aligné (la ligne d'audit porte la
  requête d'attribution). **U4 remplacé** : les signaux de containment pilote
  (hypothèse réfutée deux fois) sortent vers § Suivi 2 ; à leur place, la garde
  `run_shell` (R4, D6–D8) : mode `--decide-primary` de `guard-shared-checkout`
  (parseur de mika#2107 réutilisé, T1 remplacé — mika-qa n'a pas de worktree et
  est hors population de #2107 par construction), branchement dans
  `shell-exec/handlers/run.sh` avec lecture de `MIKA_PLATFORM_DIR` **avant** le
  scrub, prédicat = invariant du checkout de déploiement (table D7, allowlist
  nommée et non vide, ~101 gestes refusés sur 2 972 mesurés, tous de la classe
  du défaut), fail-closed sur verbe inconnu / fail-open dit sur script absent,
  refus dans `tool_calls.output` comme surface SQL. R8/U6 ajoutés (le prompt
  nomme le refus). U1/U2/U3/U5 inchangés sur le fond ; V5–V8, AC5/AC6/AC8,
  sondes 1b/2/3 et haltes 2–5 ajoutées. Le corps du ticket est corrigé par
  l'orchestrateur (prémisse « trois pilotes » → mesure).
- **rev 2 (2026-09-21)** : addressed **F1** (BLOCKING) en ajoutant la section
  `## Fire-Disposition` nommée (mika#1574) ; populations comptées à `b74f7e7c`.

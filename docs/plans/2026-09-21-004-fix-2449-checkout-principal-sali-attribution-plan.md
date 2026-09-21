# mika#2449 — Le checkout principal sali : la prémisse est réfutée, la prescription est le producteur, et la prochaine occurrence doit être datée

## Problème

Le 2026-09-21, avant un rebuild propre (#2446), le checkout **main** de `mika/`
portait 15 fichiers non-committés — `site/*`, `scripts/check-landing-tokens.sh`,
`scripts/smoke-webhook-chain`, `skills/bundled/_shared/dispatch-lib.sh` + tests.
Le contenu n'avait rien d'unique : il était identique à `main` pour les deux
premiers (#2434, mergée), à la branche de #2435 pour le troisième, à
`main@7c6f787b` pour le quatrième (#2436). Le ticket en conclut que **trois
pilotes** (#1804, #2135, #1943) ont écrit dans le checkout principal en plus de
leur worktree, et propose trois candidats : (a) le cwd du pilote, (b) le
resume-cleanup de dispatch-lib opérant sur le mauvais répertoire, (c) le moteur
tournant avec `chdir=mika/` dont un pilote hériterait.

Impact réel et non contesté : un checkout principal sali **bloque les rebuilds**
(`git pull --ff-only` refuse), rencontré pendant #2446 et contourné par stash
(`eb032b95` → `rescue/main-sale-2026-09-21`). Classe santé-substrat. p2.

### M1 — Ce que la mesure RÉFUTE : un pilote sandboxé ne PEUT PAS salir le checkout principal

Mesuré le 2026-09-21 **depuis l'intérieur d'un pilote dispatché** (la session de
grooming de ce ticket), sur le dispatch-lib en vigueur :

| Sonde | Résultat |
|---|---|
| `ls -a /data/workspace/mika-platform` | `.claude`, `claude-pilot`, `mika` — **rien d'autre** |
| `ls -a /data/workspace/mika-platform/mika` | `.git` **seul** — aucun fichier de l'arbre de travail |
| `ls -a …/mika/.git` | `objects`, `refs`, `config`, `packed-refs`, `info`, `logs`, `worktrees` |
| `command -v bwrap` (via le `--ro-bind /usr`) | `/usr/bin/bwrap` — **présent sur l'hôte** |

Le parent ne porte que les **répertoires intermédiaires matérialisés par bwrap**
pour ses binds (propriété documentée ligne ~164 de `dispatch-lib.sh`), sur
tmpfs. `mika/` n'y figure que parce que les binds git de mika#2141 ciblent
`$parent_git = …/mika/.git`.

Trois faits de code complètent la mesure :

1. **L'unique bind rw d'un arbre de travail est le worktree** :
   `--bind "$WORKTREE_DIR" "$WORKTREE_DIR"`. Les binds mika#2141 touchent le
   `.git` du dépôt principal (`objects` rw, `refs/remotes/origin` et
   `refs/tags` rw, le reste ro) — jamais son arbre de travail, et **jamais son
   `index`**, qui n'est pas bindé.
2. `--chdir "$WORKTREE_DIR"` et `--clearenv` sont posés explicitement.
3. Les **trois** sites de lancement pilote passent par `_run_pilot_sandboxed` :
   `dispatch-lib.sh:2761` (dev-pilot / dev-groom), `:5434` (`_launch_revise_pilot`),
   `:5553` (free-dispatch).

**Candidat (a) — « le cwd du pilote » : réfuté.** Un pilote qui `cd` vers
`/data/workspace/mika-platform/mika` aboutit dans un tmpfs dont l'arbre est vide.
Une écriture y est perdue à la fin de la session ; elle n'est **pas** persistée
sur l'hôte.

**Candidat (c) — « le moteur tourne avec chdir=mika/ et un pilote hérite » :
réfuté deux fois.** `--clearenv` + `--chdir` explicite écrasent tout héritage ;
et surtout **hériter d'un cwd ne crée pas un montage** — sans bind, le chemin
n'est pas là.

**Réserve, écrite plutôt que tue.** Cette mesure est **structurelle** et porte
sur le code en vigueur le 21/09 : elle établit que les voies (a) et (c)
n'existent pas. Elle n'établit **pas** l'état des trois dispatches du sinistre,
dont les journaux ne sont pas consultables ici (`gh` non authentifié dans le bac
à sable). La seule voie pilote qui resterait est un dispatch ayant tourné
**non contenu** — et c'est précisément ce que rien ne dit aujourd'hui (voir M4
et R4).

### M2 — Ce que la mesure DÉPLACE : dispatch-lib n'opère pas dans main, il y ENVOIE l'opérateur

Les **onze** opérations `git -C "$SUB_REPO_DIR"` de `dispatch-lib.sh` ont été
relevées une par une : `fetch` (2348, 2448), `worktree list` (2380),
`worktree remove` (2413, 2425, 2685), `ls-remote` (2447), `worktree add`
(2449, 2451, 2455, 2457). **Aucune ne mute l'arbre de travail du checkout
principal.** Le candidat (b), pris comme *opération*, est réfuté lui aussi.

Mais il est **confirmé comme prescription**, et c'est la trouvaille du ticket.
Deux sites stashent un worktree sale, et leurs messages de récupération
divergent :

| Site | Ligne | Message |
|---|---|---|
| A — `_clean_worktree_for_rebase` | 1695 | `recover with: git -C ${wt} stash apply …` → **le worktree** |
| B — `_set_up_worktree` (relic) | 2399 | `recover with: git -C $SUB_REPO_DIR stash apply …` → **le checkout principal** |

Exécutée, la prescription du site B dépose le contenu d'un worktree,
**non committé**, dans le checkout principal : la signature exacte du sinistre —
du contenu identique à une branche de PR, non committé, dans `main`.

**Ce n'est pas une faute de frappe, et la distinction commande le remède.** Au
site B, le worktree est **supprimé quatorze lignes plus bas**
(`worktree remove --force`, 2413) : au moment où l'opérateur lit le message, le
répertoire que le site A nommerait n'existe plus. Le message n'avait donc pas de
worktree à proposer et a nommé le seul répertoire restant. Remplacer
`$SUB_REPO_DIR` par `$existing_wt` produirait une consigne qui échoue ; le remède
doit nommer le worktree **canonique** (`$WORKTREE_DIR`, créé juste après sur la
même branche) et dire explicitement de ne pas appliquer dans le checkout
principal.

Circonstance, notée sans en faire une preuve : **mika#1943 — dont le
`dispatch-lib.sh` figure dans le sinistre — est le ticket qui a retravaillé ce
bloc** (ses marqueurs sont aux lignes 2383-2413).

### M3 — Le trou trouvé en chemin : `mika/` est writable dans le sandbox, et son arbre est vide

Conséquence directe des binds mika#2141, mesurée ci-dessus :
`/data/workspace/mika-platform/mika/` **existe** dans le sandbox, est
**writable** (tmpfs), porte un `.git` **fonctionnel** et un arbre de travail
**vide**. D'où deux comportements piégeux :

- `git -C /data/workspace/mika-platform/mika status` y voit **tous** les fichiers
  du dépôt comme *supprimés* — un pilote pourrait entreprendre de les
  « restaurer ».
- Toute écriture y va dans le tmpfs et disparaît à la fin de la session,
  **sans un mot**.

C'est la forme mika#2205 — une inertie qui se lit exactement comme un succès.
Réel, adjacent, **hors périmètre de ce ticket** (qui porte sur une saleté
*persistée sur l'hôte*, dont ceci est l'inverse) : **suivi nommé**, § Suivi.

### M4 — Ce qui reste ouvert, et pourquoi on n'élit personne

Les acteurs **non contenus** qui voient le checkout principal en écriture :
l'orchestrateur Claude Code (cwd `/data/workspace/mika-platform`), `mika-spirit`
et ses exec handlers de skill, l'opérateur humain, et un pilote qui aurait tourné
sandbox désarmé. `wip_rescue` est **vérifié innocent** pour l'arbre de main : son
`prepare_branch` rebase dans un worktree éphémère `repo_dir/.wip-rescue-wt/pr-N`
et ne fait qu'un `fetch` dans `repo_dir`.

**Aucun n'est élu.** Les traces du 21/09 sont parties (main nettoyé, stash
déporté sur une branche), et désigner un producteur qu'on n'a pas mesuré est
exactement la reconstruction *a posteriori* que mika#2026 condamne : *la
provenance est un fait estampillé par son producteur, jamais reconstruite après
coup* — et *un défaut qui ressemble à une réponse est la manière dont un
instrument ment*.

Ce que le plan livre à la place : **le producteur plausible et mécanisable est
supprimé** (M2), **le démarrage non contenu cesse d'être muet** (R4), et **la
prochaine occurrence est datée** (R3) — c'est-à-dire attribuable à une fenêtre
plutôt que reconstruite.

## Requirements

- **R1** — Le message de récupération du site B ne prescrit plus le checkout
  principal ; il nomme le worktree canonique et dit de ne pas appliquer dans le
  checkout principal.
- **R2** — Aucun message de récupération de stash de `dispatch-lib.sh` ne nomme
  `$SUB_REPO_DIR`. Tenu par un scan de source à **allowlist vide**.
- **R3** — La saleté du checkout principal est **mesurée et datée** à chaque
  tick, sur un sink effectivement lu, avec une surface SQL.
- **R4** — Le démarrage d'un pilote **non contenu** est dit sur un sink
  effectivement lu, pour les **deux** portes (bwrap absent, kill-switch désarmé),
  avec un **contrôle positif**.
- **R5** — **Rien n'est nettoyé automatiquement.** Ni pop, ni stash, ni reset du
  checkout principal.
- **R6** — Fail-open de bout en bout : aucun de ces signaux ne refuse un dispatch
  ni ne fait échouer un tick.
- **R7** — Les noms d'événement sont un **format de fil** : site de définition
  unique, SOLE WRITER épinglé par scan de source.

## Décisions

### D1 — Le sink de R3 est le moteur, PAS dispatch-lib (et c'est la décision centrale)

Le réflexe serait d'émettre la sonde dans `_set_up_worktree`, qui touche déjà le
checkout principal au `fetch` (2348). **Ce serait rejouer le Signal M mot pour
mot.**

`_set_up_worktree` tourne au niveau de `dispatch_claude_pilot`, **hors** de la
redirection `2>"$STDERR_FILE"` qui n'enveloppe que la ligne 2761. Sa stderr est
le `Stdio::piped()` de `spawn_long_running_exec`, que l'exécuteur lit
**uniquement** dans sa branche `if !status.success()` : sur un dispatch qui
réussit, le tuyau est abandonné sans lecture et les lignes n'atterrissent dans
**aucun fichier**. C'est le diagnostic écrit du CLAUDE.md pour
`pilot_push_guard` — *« un signal de sûreté qui lit "rien à signaler"
indépendamment de la réalité »*.

Troisième occurrence documentée de cette classe (Signaux M, Q, S ;
mika#2041/#2050). On ne la refait pas : R3 vit côté Rust, sink
`$MIKA_SPIRIT_LOG_FILE` + `audit_events`.

### D2 — L'hôte de R3 est `worktree_reaper` (mika#2420)

Il énumère **déjà** les checkouts via `MIKA_WORKTREE_REAP_REPO_DIRS` (défaut
`/data/workspace/mika-platform/mika` — exactement le checkout sali), tourne
**déjà** sur un tick, écrit **déjà** ses `log_audit_event`. Coût : une commande
`git status --porcelain` par checkout par tick. **Zéro nouvelle variable
d'environnement, zéro nouveau chemin d'accès, zéro nouveau scan récurrent.**

Refusé : un cinquième scan récurrent (trois leviers, trois surfaces et un
kill-switch de plus pour une mesure qui tient en une commande) ; et
`MIKA_WIP_RESCUE_REPO_DIR`, qui ne porte qu'un checkout là où le faucheur en
porte une liste.

### D3 — La sonde DATE, elle n'ATTRIBUE pas

Elle rapporte : le checkout, le nombre de fichiers, les chemins (plafonnés), et
l'instant. Elle **ne nomme aucun producteur** — elle borne la fenêtre
d'apparition entre deux ticks, ce dont l'enquête du 21/09 a manqué. Écrire un
producteur dérivé (« le dernier dispatch », « le dernier pilote ») serait la
dérivation tardive que mika#2368 refuse et la reconstruction que mika#2026
condamne.

### D4 — Déduplication sur `(checkout, empreinte de la liste)`, horizon 24 h

Un checkout sale le reste des jours. Une ligne d'audit par tick (144/j) y
déplacerait le churn que la doctrine mika#2131 borne. Un **changement** de la
liste de fichiers ré-écrit : c'est un producteur nouveau, donc un fait nouveau.
L'horizon de 24 h est porteur pour la même raison qu'en mika#2131 — sans lui, un
checkout nettoyé puis re-sali n'écrirait rien la seconde fois, et la ligne la
plus récente porterait un fait périmé sous une date qui suggère l'actualité.

**Checkout propre ⇒ zéro ligne** (doctrine mika#2131 AC7 : une observabilité qui
journalise tout le monde ne distingue plus personne).

### D5 — Non bloquant, et l'asymétrie est écrite

Un checkout principal sale n'empêche **pas** un dispatch — le worktree est
ailleurs, et les trois dispatches du sinistre ont tous abouti à une PR mergée.
Il empêche un `pull --ff-only`, c'est-à-dire un geste d'opérateur. Refuser le
dispatch coucherait la boucle pour un défaut de poste de build : le coût du faux
positif écraserait le bénéfice. Fail-open aussi sur la mesure elle-même — un
`git status` illisible sort le checkout de la population et le **dit** (R6),
il ne fait jamais échouer le tick du faucheur.

### D6 — R4 s'émet côté shell, mais DANS la fenêtre redirigée, et avec un contrôle positif

`_run_pilot_sandboxed` est appelé **depuis** les lignes 2761 / 5434 / 5553, dont
la stderr **est** `$STDERR_FILE`, persistée en
`$_PILOT_LOG_DIR/<task-id>.stderr` — le sink du Signal S, déjà éprouvé et déjà
relu par `dispatch-lib` lui-même (extraction `[policy:deny]`, mika#1097). Le
contraste avec D1 est net et c'est ce qui rend les deux placements corrects.

Trois manques à combler :

1. **Le kill-switch désarmé est entièrement muet.** `_pilot_sandbox_enabled`
   (ligne 182) rend 1 sans un `echo`. `MIKA_PILOT_SANDBOX=0` produit donc un
   pilote non contenu **sans une ligne**, ce qui est exactement ce qui rend la
   voie pilote indétectable après coup.
2. **Le message « bwrap absent » (926) n'a pas de jeton ancré.** Comme pour le
   Signal S, le fichier `.stderr` porte aussi la prose du pilote : un grep nu
   matcherait un texte *parlant* du signal. Ancrage `^dispatch-lib: ` obligatoire.
3. **Pas de contrôle positif.** Sans lui, zéro occurrence ne distingue pas
   « tous les pilotes étaient contenus » de « la commande regarde au mauvais
   endroit » — la halte que le Signal S a dû écrire pour lui-même.

### D7 — R2 est un scan de source, et un test comportemental ne peut pas le remplacer

La régression que R2 borne ne rend **aucune décision fausse** : elle remet une
consigne fautive dans un message. Toutes les assertions de comportement
resteraient vertes pendant que l'opérateur salirait de nouveau son checkout.
Même raison que `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`.
Allowlist **vide** livrée, et **la résolution quand il tire est de corriger le
message**, jamais d'ajouter une entrée.

## Scope Boundaries

**Dans le périmètre** : le message de récupération du site B ; le scan de parité ;
la sonde de saleté dans `worktree_reaper` ; les deux signaux de containment du
pilote + leur contrôle positif ; la documentation des surfaces.

**Hors périmètre, délibérément** :

- **Nettoyer ou committer le checkout principal.** R5. Décision Vincent (« ne pas
  pop le stash ») et, plus largement, une mutation automatique d'un arbre de
  travail d'opérateur est irréversible.
- **Élire un producteur pour le sinistre du 21/09.** M4.
- **L'écriture perdue dans le tmpfs `mika/`** (M3) — inverse du symptôme traité.
- **Les binds mika#2141 eux-mêmes.** `objects` rw est le contrat de mika#2141 ;
  le restreindre casserait le git du pilote pour un défaut qui ne passe pas par
  là.
- **Le fallback non contenu lui-même** : il est *dit*, il n'est pas supprimé.
  Transformer une tolérance de déploiement en refus dur est un changement de
  disposition qui mérite sa mesure — et le signal de R4 est ce qui la produira.

## Implementation Units

### U1 — La prescription réparée (R1)

`skills/bundled/_shared/dispatch-lib.sh` ~2399. Le message nomme `$WORKTREE_DIR`
(worktree canonique, créé juste après sur la même branche) et porte une butée
explicite : ne pas appliquer dans le checkout principal. Ajouter le marqueur
`mika#2449` et une ligne de raison au-dessus (pourquoi ce n'est pas
`$existing_wt` : il est supprimé quatorze lignes plus bas).

### U2 — Le scan de parité (R2, D7)

`skills/bundled/_shared/test-dispatch-lib.sh`, style `assert_*` du harnais
existant. Extraire les lignes de `dispatch-lib.sh` contenant à la fois
`stash apply` et `recover with`, refuser toute occurrence de `SUB_REPO_DIR`.
Allowlist vide + **contrôle de bonne foi** : le scan doit trouver au moins deux
sites (sinon il est vert parce qu'il ne regarde rien — classe mika#2205).

### U3 — La sonde de saleté (R3, R6, D2, D3, D4)

`crates/mika-agent/src/worktree_reaper.rs`. Dans la boucle par checkout de
`reap_terminal_worktrees` (1065) : `git status --porcelain` borné en temps,
fail-open. Fonction **pure** séparée pour la décision
(propre / sale+liste / illisible), testable sans git. Émission dédupliquée par
`(checkout, empreinte)` sur 24 h via `audit_events`, sur le modèle de
l'exclusion mika#2131. Chemins plafonnés (20, comme `DIRTY_FILES` en 3486) et
**jamais** de contenu de fichier.

**L'ordre dans la boucle est porteur, et le placer au mauvais endroit produit un
défaut plutôt qu'un signal.** La sonde va **après** la garde
`repo_dir.join(".git").exists()` qui émet `worktree_reap_no_checkout`, et avant
le `git worktree list`. Placée en tête de boucle — le réflexe — elle sonderait un
chemin sans dépôt : en production conteneurisée, où les worktrees ne vivent pas
sur le système de fichiers de l'agent, chaque checkout configuré produirait un
`git status` en échec, donc une émission « illisible » **à chaque tick et pour
toujours**. Ce serait le bruit permanent que R6 et D5 existent pour éviter, sous
un nom qui promet une mesure. La garde existante répond déjà à cette population,
et la sonde doit hériter de sa réponse plutôt que la contredire.

Ce placement a un corollaire à écrire dans le test : « pas de checkout à ce
chemin » et « `git status` illisible sur un checkout réel » sont **deux** états
distincts, et seul le second est un signal de R3.

### U4 — Les deux signaux de containment + le contrôle positif (R4, D6)

`skills/bundled/_shared/dispatch-lib.sh` : ancrer le message 926 en
`^dispatch-lib: ` ; ajouter l'émission manquante au désarmement explicite
(`_pilot_sandbox_enabled` rendant 1 — émettre au **site d'appel**, pour que la
fonction reste un prédicat pur) ; ajouter la ligne de contrôle positif au
lancement contenu, sur le modèle de `pilot-egress-proxy launched`.

### U5 — Les noms de fil et leur SOLE WRITER (R7)

Constantes à site unique côté Rust ; scan de source refusant un second écrivain
de chaque nom (modèle `worktree_reaped`). Côté shell, les trois jetons sont
épinglés par U2.

### U6 — Documentation

`mika/CLAUDE.md` : une entrée dans le voisinage de mika#2420 (§ terminal-worktree
reaper), portant les surfaces opérateur, le régime attendu de chaque signal et
ses haltes. `crates/mika-agent/CLAUDE.md` si le module y a une section.
Les commandes de grep du § Sondes ci-dessous y sont recopiées **avec leur
ancrage**.

## Verification Contract

1. **Séquence rouge prescrite pour U2** : réintroduire `$SUB_REPO_DIR` dans le
   message du site B → le scan doit **rougir**. Le remettre correct → vert.
   Sans cette séquence, un scan qui ne regarde rien est indistinguable d'un scan
   satisfait.
2. **Contrôle de bonne foi de U2** : le scan compte ≥ 2 sites `recover with`.
3. **U3, fonction pure** : propre → aucune émission ; sale → une émission portant
   le compte et les chemins ; illisible → hors population + signal nommé, jamais
   « propre ».
3b. **U3, placement** : un `repo_dir` **sans `.git`** produit
   `worktree_reap_no_checkout` et **aucune** émission de saleté — contrôle
   négatif qui vaut pour toute la production conteneurisée. Un checkout réel dont
   le `git status` échoue produit, lui, le signal « illisible ». Les deux états
   ne partagent pas de nom.
4. **U3, déduplication** : deux ticks consécutifs sur la même liste → **une**
   ligne ; liste changée → deux lignes ; > 24 h → ré-écriture.
5. **U3, non-blocage** : un checkout sale ne change ni le verdict de fauche ni le
   compte des worktrees fauchés (contrôle négatif).
6. **U4** : les trois jetons matchent `^dispatch-lib: ` et sont absents d'une
   prose citant le signal (le faux positif mesuré du Signal S).
7. `make verify-bundled-skills`, `cargo test -p mika-agent`, `cargo clippy`,
   `bash skills/bundled/_shared/test-dispatch-lib.sh`.

## Definition of Done

- Le site B ne nomme plus le checkout principal ; le scan de parité est vert et a
  été vu rouge.
- La sonde de saleté est branchée dans le faucheur, dédupliquée, fail-open, et
  n'émet rien sur un checkout propre.
- Les deux portes de containment émettent, ancrées, avec leur contrôle positif.
- `CLAUDE.md` porte les surfaces, les régimes attendus et les haltes.
- Aucun nettoyage automatique n'a été ajouté (R5), aucune variable
  d'environnement n'a été créée.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés des Requirements et du Verification Contract.

- **AC1** — La prémisse du ticket est tranchée par écrit, mesure à l'appui : (a)
  et (c) sont réfutés pour tout pilote sandboxé ; (b) est réfuté comme opération
  et confirmé comme prescription. La réserve de M1 (portée structurelle, pas
  forensique) est écrite et non gommée.
- **AC2** — `git -C <checkout-principal> stash apply` n'est plus prescrit nulle
  part par `dispatch-lib.sh`, et un scan de source à allowlist vide l'empêche de
  revenir.
- **AC3** — Un checkout principal sale produit, au tick suivant, une ligne de
  journal **et** une ligne `audit_events` datées, nommant le checkout, le compte
  et les chemins (plafonnés) — jamais un producteur.
- **AC4** — Un checkout principal propre produit **zéro** ligne.
- **AC5** — Un pilote démarré non contenu, **par l'une ou l'autre porte**, laisse
  une ligne ancrée dans `$_PILOT_LOG_DIR/<task-id>.stderr` ; un pilote contenu
  laisse la ligne de contrôle positif.
- **AC6** — Rien n'est nettoyé, committé ni stashé automatiquement ; aucun
  dispatch n'est refusé et aucun tick ne peut échouer à cause de ces ajouts.

## Sondes post-déploiement, et leurs haltes

```bash
# 1. Le checkout principal est-il sale, et depuis quelle fenêtre ?
grep main_checkout_dirty "$MIKA_SPIRIT_LOG_FILE" | jq -c '{repo_dir, file_count, files}'
```
```sql
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'main_checkout_dirty' ORDER BY created_at DESC;
```
```bash
# 2. Des pilotes ont-ils démarré non contenus ?  (régime attendu : ZÉRO)
grep -l '^dispatch-lib: pilot_sandbox_bypassed' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr

# 3. CONTRÔLE POSITIF — zéro ne vaut « sain » que si celui-ci est non nul
grep -l '^dispatch-lib: pilot_sandbox_engaged' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr
```

**Halte 1 — la sonde 1 est vide pendant que le checkout est visiblement sale.**
Ne pas élargir le prédicat : lire `MIKA_WORKTREE_REAP_REPO_DIRS`, qui ne pointe
probablement pas ce checkout, puis vérifier que le binaire déployé porte le
correctif (classe mika#2340). **Établir le déploiement avant de toucher au code.**

**Halte 2 — la sonde 2 est vide ET la sonde 3 l'est aussi.** Le couple ne prouve
rien : soit le glob regarde ailleurs (`PILOT_LOG_DIR` vs `MIKA_PILOT_LOG_DIR`
divergent — mika#2249), soit aucun dispatch n'a tourné. **Zéro de l'un seul n'est
un régime sain que si l'autre est non nul.**

**Halte 3 — la sonde 2 est NON vide.** C'est un **résultat**, pas une panne : la
voie pilote du ticket redevient plausible pour ces dispatches-là, et c'est la
seule mesure qui pouvait l'établir. Relever le motif (bwrap absent vs kill-switch)
avant toute conclusion — les deux remèdes diffèrent.

**Halte 4 — la sonde 1 reste vide sur des semaines alors qu'un checkout sale
réapparaît.** Le producteur écrit entre deux ticks puis nettoie, ou le checkout
n'est pas dans la liste. Ne pas raccourcir le tick par réflexe : établir d'abord
lequel des deux.

**Ce que ce travail n'achète pas.** Il ne dit pas **qui** a sali le checkout le
21/09 — cette information est perdue, et la fabriquer serait pire que son
absence. Il supprime un producteur prescrit, rend le démarrage non contenu
visible, et fait en sorte que la **prochaine** occurrence arrive avec une date et
une fenêtre. Et comme toute sonde, son silence ne prouve rien tant que le
contrôle positif n'a pas été lu.

## Suivi (hors périmètre, nommé)

1. **L'écriture perdue dans le tmpfs `mika/`** (M3) — un pilote qui écrit dans
   `/data/workspace/mika-platform/mika/` travaille dans le vide, sans un mot, et
   `git status` y présente tout le dépôt comme supprimé. Classe mika#2205.
   Préalable à l'ouverture : établir qu'un pilote y va réellement (le signal de
   R4 et les `.stderr` donnent la matière).
2. **Transformer la tolérance de déploiement en refus** — faire du démarrage non
   contenu un abort plutôt qu'un WARN. Préalable **explicite** : la mesure que R4
   produit. Instruire avant de mesurer serait construire sur une hypothèse.
3. **Un `pull --ff-only` de l'orchestrateur qui dit ce qui le bloque** — l'impact
   du ticket est un geste d'opérateur refusé ; le rendre auto-explicatif est une
   ergonomie, pas une garde.

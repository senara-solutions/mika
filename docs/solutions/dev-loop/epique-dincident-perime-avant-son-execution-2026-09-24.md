---
module: docs/plans, skills/bundled/_shared/dispatch-lib.sh, crates/mika-agent/src/worktree_reaper.rs, tools/mika_permission_policy
tags: [incident-closure, epic, condition-rot, measurement-over-memory, squash-merge, rescue-net, permission-policy, ssc-boundary, grooming]
problem_type: best-practice
category: dev-loop
created: 2026-09-24
ticket: mika#1696
---

# Un épique d'incident périme avant d'être exécuté — et son plan de grooming périme plus vite encore

## TL;DR

mika#1696 s'est donné cinq conditions de clôture le 2026-06-30. Relues **86 jours plus
tard**, sur mesure et non de mémoire : **aucune des cinq n'est vraie telle qu'écrite.** Une
est satisfaite, une est à moitié livrée par un ticket qui ne porte pas son nom, une est
dépassée par cinq correctifs descendants, une exige une chose qu'une décision prise dans son
propre corps a annulée six heures après sa rédaction, et la cinquième repose sur une prémisse
qui était **déjà fausse le jour où elle a été écrite**.

Et la démonstration ne s'arrête pas au ticket. Le plan de grooming rédigé pour *cette*
clôture a lui-même péri en **sept jours**, puis une seconde fois **le jour de son exécution**.

D'où la règle que ce document existe pour poser : **un épique d'incident se re-mesure avant
d'être exécuté, jamais sur son énoncé d'origine.** Le corollaire est plus dur : un plan de
grooming aussi.

---

## 1. L'incident, pour un lecteur qui n'a pas le ticket sous les yeux

Après-midi du **2026-06-30**. Le pipeline de dispatch de la boucle autonome de mika met en
échec **cinq pilotes ou plus**, sur une chaîne de défauts de substrat. Deux causes
conjoncturelles se croisent : la boucle est inhabituellement chargée (une passe Tier-2 a
dispatché huit tickets coup sur coup) et la bascule de modèle vers glm-5.2 expose des classes
de défauts que le modèle précédent masquait. La lecture retenue le jour même, par revue par
les pairs, est explicitement positive : *la découverte fonctionne — le substrat produit plus
de bugs que d'habitude parce que la journée l'a exercé plus fort que d'habitude.*

mika#1696 est ouvert non pas comme un ticket d'implémentation mais comme un **parent de
coordination**, pour que le backlog se lise « 1 incident, 5 sous-correctifs » plutôt que
« 5 nouvelles dettes ». Ses cinq enfants :

| sous-fix | objet |
|---|---|
| mika#1684 | l'étape 1.5 de `qa-review` classe mal les PR de récupération et les approuve au lieu de les tenir |
| mika#1685 | les commits de récupération passent par le hook de pre-commit — **cause modale du wedge** |
| mika#1686 | le classifieur de permission refuse du bash de lecture légitime (n=8+ formes en 24 h) |
| mika#1687 | mort silencieuse de pilote : deux pilotes `in_progress` plus d'une heure sans callback |
| mika#1694 | accumulation de dette de worktrees et de branches (13 worktrees, « 30+ » branches ce jour-là) |

mika#1697 (frontière deadline sous-processus pilote vs boucle d'agent) est ajouté le soir même
sur instruction de Mika Prime, délibérément séparé de #1687 : autre sous-système.

L'épique nomme lui-même son propre livrable, et c'est ce document :

> *After closure: orchestrator-CC documents the incident as compound learning + closes the
> epic.*

**Le corps d'un épique n'est pas une surface de travail.** Ses cinq conditions ont été figées
à l'instant de l'incident et jamais mises à jour — y compris après qu'une décision prise dans
ce même corps, six heures plus tard, en a invalidé une (condition 4, § 3.4).

---

## 2. Méthode de mesure, et sa limite — déclarée parce qu'elle s'est répétée trois fois

Tout l'état ci-dessous est reconstruit **depuis le dépôt local** : `git log`, `git show`,
`git branch -r`, `git for-each-ref`, `docs/dormeurs.md`, l'arbre des skills, des crates et des
outils. Chaque chiffre porte **la commande qui le produit** et **le jour où elle a été lancée**
(2026-09-24 sauf mention contraire).

**`gh` n'était authentifié dans aucune des trois sessions** — grooming du 2026-09-17,
re-grooming du 2026-09-24, exécution du 2026-09-24 :

```console
$ gh auth status
You are not logged into any GitHub hosts. To log in, run: gh auth login
```

Trois occurrences identiques font basculer la lecture : ce n'est plus un accident de session,
c'est une **propriété du bac à sable de dispatch**. Conséquence assumée : **l'état GitHub des
sous-tickets (#1694, #1686, #1687, #1680, #1697) n'a pas pu être lu.** Un sous-ticket peut
avoir été fermé sans laisser de commit ici.

**La règle qui en sort, et elle a payé deux fois.** Quand une preuve locale existe, la
préférer : elle est reproductible par n'importe qui, avec ou sans jeton. La condition 2 a
basculé d'état sans qu'aucune lecture GitHub soit possible, tranchée sur trois preuves locales
concordantes (§ 3.2). La condition 5 a été **réfutée** de la même manière, par un fichier et
deux commits de ce dépôt (§ 3.5).

---

## 3. Les cinq conditions de clôture, relues une par une

### 3.1 Condition 1 — « Deploy bundle {1685+1679+1383} is live » → **satisfaite**

| sous-fix | état | preuve (`git show <sha> --no-patch`, 2026-09-24) |
|---|---|---|
| mika#1685 (`--no-verify` sur les commits de récupération) | **fusionné** | `d8f780ab` · 2026-06-30 · `fix(dispatch-lib): bypass pre-commit hook on post-flight rescue commits (mika#1685) (#1690)` |
| mika#1679 (Edit 4 absorbé, gate #1383/#1396) | **fusionné** | `2aa9bc5c` · 2026-07-01 · `fix(dispatch-lib): stop the mika#1383 gate shadowing the correct mika#1396 rescue (mika#1679) (#1698)` |
| mika#1684 (étape 1.5, en-tête de récupération) | absorbé dans #1679 | — |
| filet secret-scan que `--no-verify` ne contourne pas (mika#1689) | **fusionné** | `5db56354` · 2026-08-27 · `fix(ci): add a secret-scan net the --no-verify rescue path can't bypass (mika#1689) (#2017)` |

Les quatre SHA existent et portent le message attendu. La « deploy unit » est en service, et
l'inquiétude explicite de la revue par les pairs du 2026-06-30 — *« 1685 par lui-même nous
fait passer de fail-loud (wedge) à fail-quiet (commits de récupération sales, non-draft, qa
auto-approuve) »* — est levée : #1679 a fusionné le lendemain, et #1689 a ajouté le filet que
`--no-verify` ne peut pas contourner.

**Mais la classe de défaut que ce filet attrape est en rafale active**, et son ampleur avait
été sous-comptée d'un facteur 5. Voir F1 (§ 5).

### 3.2 Condition 2 — « mika#1694 has shipped » → **partiellement livrée**

**La moitié worktree est livrée**, le 2026-09-20, par un ticket qui ne porte pas le numéro de
#1694 :

```
fc96a341  2026-09-20  fix(substrate): les worktrees de PR terminale
                      ne survivent plus à leur PR (mika#2420) (#2429)
```

Le corps de ce commit **nomme #1694 et rectifie sa lecture** : *« #1694 n'est pas "fermé mais
inefficace". C'est un dormeur dont la condition de réveil est remplie, dont la logique n'a
jamais atteint `main` (commit `097cc66c`, sauvé en `wip()` par la recovery mika#1282) »*.

Trois preuves locales concordantes, toutes reproductibles sans jeton :

| preuve | commande | résultat au 2026-09-24 |
|---|---|---|
| le reaper moteur existe | `wc -l crates/mika-agent/src/worktree_reaper.rs` | **5004 lignes** |
| le dormeur a quitté le registre | `grep -n 1694 docs/dormeurs.md` | **aucune ligne** |
| le retrait est daté par ce commit | `git log --format=… -- docs/dormeurs.md` | `fc96a341`, 2026-09-20 |

Le registre des dormeurs porte un contrat explicite — *« Quand la condition est remplie,
rouvrir le ticket GitHub cité (il conserve tout son historique) et retirer la ligne d'ici. »*
La ligne a été retirée : le réveil a donc eu lieu.

**La seconde moitié n'est pas livrée, et elle est explicitement exclue — pas oubliée.**
`CLAUDE.md:2066` énumère le hors-périmètre de mika#2420 : les couches A et B de #1694
(`worktrees-audit` / `worktrees-clean`, qui vivent dans le dépôt `mika-platform` et restent le
geste manuel), **les branches distantes**, et *« reopening #1694 itself, which is an
orchestrator gesture the PR body signals rather than performs »*.

État des deux axes de #1694, de l'incident à aujourd'hui :

| axe | 2026-06-30 (corps du ticket) | 2026-09-17 | **2026-09-24** | verdict |
|---|---|---|---|---|
| worktrees | 13 | 2 | **1** | **livré** (moteur mika#2420 + geste opérateur) |
| branches `origin` non fusionnées | « 30+ » | 166 | **141** | **non livré**, hors périmètre de mika#2420 |

Commandes : `ls /data/workspace/mika-platform/.claude/worktrees/ | wc -l` ·
`git branch -r --no-merged origin/main | wc -l`.

Détail qui vaut d'être noté, parce qu'il est vérifiable et qu'il atteste que le reaper
fonctionne : **le seul worktree restant est celui dans lequel ce document a été écrit**
(`incident-1696-2026-06-30-autonomous-loop-wedge-one`).

Contexte de la dette de branches : **168 refs `origin` au total**, dont **102 antérieures à
septembre** (`git for-each-ref --format='%(committerdate:short)' refs/remotes/origin | sort |
grep -c "^2026-0[1-8]"`), la plus ancienne datant du **2026-03-27**
(`origin/docs/memory-aware-agents-brainstorm`) — six mois de branches mortes.

#### Le piège du prédicat, et c'est le vrai apport de cette condition

Le chiffre a *baissé* de 166 à 141 en une semaine. **Cette baisse n'est pas interprétable**, et
la raison disqualifie l'instrument entier.

Ce dépôt fusionne en **squash** : chaque titre de `main` porte un `(#NNNN)`. Après un
squash-merge, la branche d'origine **n'est jamais ancêtre de `main`**. Donc :

- `git branch -r --no-merged origin/main` compte 141 branches dont la quasi-totalité sont déjà
  fusionnées côté forge. Le chiffre **surestime massivement**, et ses variations d'une semaine
  sur l'autre ne mesurent rien d'exploitable.
- Symétriquement — et c'est la moitié dangereuse — un outil de nettoyage bâti sur le prédicat
  naïf `git branch --merged` **ne reaperait jamais rien, sur aucune branche, en silence**. Il
  se lirait exactement comme un outil qui n'a rien trouvé à nettoyer.

C'est la classe de panne que ce dépôt documente à répétition : *un instrument silencieusement
inerte se lit exactement comme un instrument qui n'a rien trouvé* (mika#2205, mika#2131). Un
futur reaper de branches doit interroger **l'état de la PR côté forge**
(`gh pr list --state merged --head <branch>`), **jamais** l'ancestry git. Ce constat appartient
à la moitié non livrée de #1694 (F2, § 4) ; rien n'en est implémenté ici.

### 3.3 Condition 3 — « mika#1687 hypothesis confirmed for both stuck pilots » → **dépassée**

Le sous-ticket a produit un correctif livré — `ffb09cf4` · 2026-07-28 ·
`test(task-engine): childless-parent reaper tests + docs (mika#1687) (#1859)` — et une lignée
entière l'a depuis dépassé :

| mécanisme | ticket |
|---|---|
| watchdog de callback sur mort de sous-processus (`/proc/<pid>/stat`) | mika#959 |
| reaper `stuck-pending` | mika#2045 |
| reaper de stall silencieux de pilote, à trois surfaces de liveness | mika#2249 / mika#2277 |
| phantom sweep + garde de liveness | mika#1712 / mika#2156 |

Toute cette famille est documentée dans `CLAUDE.md`. La condition telle qu'écrite — confirmer
l'hypothèse **pour les deux pilotes bloqués** du 2026-06-30 — porte sur deux lignes de tâche
vieilles de 86 jours. Elle n'est **ni vérifiable** (les tables `tasks` et `audit_events` ont
été compactées depuis) **ni utile** : la classe a reçu cinq correctifs structurels. À déclarer
dépassée sans chercher à la satisfaire à la lettre.

### 3.4 Condition 4 — « mika#1680 reframed (revert + calibration suite hardening) » → **caduque, et auto-contradictoire**

Cette condition exige un recadrage dont la première moitié est le **revert de modèle**. Or le
même corps de ticket, quelques paragraphes plus haut, porte la décision de l'opérateur du
2026-07-01 ~00:30Z :

> *« glm stays. Drop the revert entirely; it's not happening, so everything that hung off it
> goes too. »*

La condition de clôture 4 n'a jamais été mise à jour après cette décision. **Le ticket exige
une chose que le ticket a annulée**, à six heures d'écart, dans le même document.

Le temps a tranché une seconde fois : la flotte est passée à glm-5.3 et kimi depuis
(cf. `CLAUDE.md` § *Observabilité du budget effectif*, et mika#2473 qui mesure la dérive
modèle code↔runtime). « Revert vers Sonnet-4-6 » ne désigne plus aucun état atteignable.

Ce qui reste de #1680 — le *CJK bleed*, classé le jour même « tracked annoyance, not a
blocker » — n'a aucun commit dans ce dépôt. Sa seconde moitié, durcir la suite de calibration
en garde de swap de modèle, est un vœu légitime mais ce n'est pas une condition de clôture
d'incident. À déclarer caduque, le reste routé (F4, § 4).

### 3.5 Condition 5 — « mika#1686 routed to its determined fix shape » → **actionnable ici pour moitié**

> **Ce verdict corrige celui du plan de grooming**, qui concluait « non actionnable dans ce
> dépôt ». Trois mesures du 2026-09-24 réfutent sa prémisse. C'est la correction la plus
> importante de ce document, et elle illustre la thèse : la prémisse était fausse **dès le
> premier grooming**, pas seulement aujourd'hui.

Ce qui est vrai : la **skill** `permission-policy` a bien quitté ce dépôt —
`50e13e59` · 2026-05-30 · `chore(mika): retire mika-relay agent + permission-policy skill
(mika#1193) (#1348)`. Elle est absente de `skills/bundled/`, et `templates/skills/` n'existe
plus du tout.

Ce qui est faux : en déduire que le substrat du défaut est entièrement hors de ce dépôt.
**Le contenu de la policy est revenu ici 22 jours après l'incident :**

```
232be1e8  2026-07-22  feat(permission-policy): mika-side plugin — per-binary safety
                      functions (mika#1817, UNBLOCK cpp per-spawn flip) (#1818)
```

`tools/mika_permission_policy/README.md` énonce le partage, qui est une **frontière OSS
ratifiée par l'architecte** (spec mika#1708) et non un accident de rangement :

> *Ships the private allow/deny CONTENTS that live on Mika's side of the SSC OSS boundary;
> claude-pilot ships the generic engine (`per_spawn.py`, empty `DEFAULT_POLICY`) and loads
> this plugin at runtime.*

Et ce contenu est **activement maintenu ici** : dernier commit le touchant,
`e1342dfa` · 2026-09-18 · `fix(review): le deny sans rule-id est le refus par défaut, pas un
jugement (mika#2312) (#2384)`.

#### Le défaut est toujours vivant — trois reproductions datées, deux formes

| date | commande refusée (lecture pure) | message |
|---|---|---|
| 2026-09-17 | enchaînement `git log --grep` + `echo` | `policy allow (bash-grep) vetoed — command chains a tier3-dangerous or command-substitution tail` |
| 2026-09-24 (re-grooming) | `git for-each-ref … \| sort \| awk '$1 < "2026-09-01"'` | `no matching policy rule -- denied by default` |
| 2026-09-24 (exécution) | la même, rejouée à l'identique | `no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)` |

Trois refus, trois sessions, deux formes distinctes, sur des commandes de **lecture pure**.
C'est la forme n=8+ que #1686 décrivait le 2026-06-30, inchangée 86 jours plus tard.

#### Les deux formes ne se routent pas au même endroit, et c'est le livrable de cette section

`docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md`
(mika#2312, dans **ce** dépôt, écrit **six jours avant** le re-grooming) donne la procédure de
lecture et le discriminant :

- Le marqueur a la forme `[policy:deny] <Tool>: <detail>[ [<rule-id>]] (terminal|non-terminal)`.
  **Le `[<rule-id>]` est ce qui attribue la cause, et c'est la première chose à lire.**
- **Un refus sans `rule-id` est le refus par défaut de la policy, et il est déterministe** :
  aucune règle n'a matché. Son `reason` est précisément le message de la troisième ligne du
  tableau ci-dessus. Et la conclusion de ce document est explicite :

  > *« Élargir l'allow-list est donc le remède de cette classe, pas une impasse. »*

  Or l'allow-list — le contenu — ship depuis **ce** dépôt.
- Un veto de **chaîne** ou de **substitution de commande** (la première ligne du tableau) est
  refusé par le moteur au niveau de la source brute, avant que la moindre fonction par binaire
  ne le voie. `tools/mika_permission_policy/README.md` § *What this plugin does NOT check* le
  dit mot pour mot. Cette moitié est bien celle de `claude-pilot`.

**Limite honnête, à ne pas franchir.** Le mode réellement armé
(`MIKA_PERMISSION_POLICY_MODE`) **n'est pas lisible depuis le bac à sable** — mika#2312 le dit
de sa propre enquête. Ce document ne peut donc pas attribuer les refus du 2026-09-24 à un étage
précis. Ce qu'il établit, et qui suffit à changer le routage : **« non actionnable dans ce
dépôt » est faux comme verdict global.** Le geste juste est de lire le `[<rule-id>]` du refus,
puis de router selon le discriminant ci-dessus — pas de classer #1686 hors dépôt par principe
(F3, § 4).

---

## 4. Les cinq constats, et leur véhicule

| id | constat | véhicule |
|---|---|---|
| **F1** | mika#1383 récidive en **rafale active**, et il y a **deux familles distinctes** sous le même numéro, pas une. Le grooming initial en mesurait une seule, avec un chiffre faux d'un facteur 5. | **Ticket de suivi à ouvrir** — contenu complet au § 5, prêt à être posé en un geste. Non ouvert ici : `gh` non authentifié (§ 2). La condition 1 de #1696 portait sur le *déploiement* du bundle, qui a bien eu lieu. |
| **F2** | La moitié **worktree** de #1694 est livrée (mika#2420, `fc96a341`, 2026-09-20). La moitié **branches** ne l'est pas et est explicitement hors périmètre. Et en dépôt squash-merge, un prédicat `git branch --merged` est **structurellement inerte**. | **À porter sur mika#1694** (re-spécification de la moitié restante). Ne rien implémenter. **Ne pas rouvrir par réflexe** : mika#2420 dit lui-même que rouvrir #1694 *« is an orchestrator gesture »*. `gh` inaccessible ⇒ consigné ici, décision à l'opérateur (§ 6). |
| **F3** | #1686 toujours vivant, reproduit dans les **trois** sessions sous **deux** formes de refus. Son verdict « non actionnable ici » est **réfuté** : depuis mika#1817, le contenu allow/deny vit dans ce dépôt (`tools/mika_permission_policy/`), le moteur seul est chez `claude-pilot`. | **Routage scindé.** Refus sans `rule-id` (défaut de policy) ⇒ élargir l'allow-list, **ticket sur ce dépôt**. Veto de chaîne ou de substitution ⇒ `senara-solutions/claude-pilot`, par spawn manuel (hors allowlist de la boucle). Lire `[<rule-id>]` **avant** de choisir. |
| **F4** | La condition de clôture 4 exige un revert que le même ticket a annulé le 2026-07-01, et que le temps a rendu inatteignable. | **Déclarée caduque** ici (§ 3.4). Le durcissement de la suite de calibration en garde de swap reste un vœu légitime, sans rapport avec la clôture d'un incident. |
| **F5** | La condition 3 (#1687) est dépassée par cinq correctifs structurels descendants ; sa lettre porte sur deux tâches de juin non vérifiables. | **Déclarée dépassée** ici (§ 3.3). |

---

## 5. F1 en détail — deux familles, et le grooming initial en mesurait une seule

Le grooming du 2026-09-17 annonçait « 1 en juin, 0, 0, 4 en septembre ». **Les deux moitiés de
cet énoncé sont fausses.** `dispatch-lib.sh` porte **deux** mécanismes de récupération
distincts sous le même numéro de ticket, à deux sites :

| famille | site | nature | distribution sur `origin/main` |
|---|---|---|---|
| *trailing content* | `dispatch-lib.sh:4720` | commit du contenu résiduel laissé après `end_turn` | **5** juin · 0 juil · 0 août · **21** sept |
| *auto-PR-create* | `dispatch-lib.sh:8653-8655` | commit **vide** (`--allow-empty`) pour permettre l'ouverture d'une PR | 0 juin · **5** juil · **2** août · **71** sept |

Commandes (mesure sur `origin/main` **seul** — un `--all` compte double, la branche *et* son
squash) :

```bash
git log origin/main --grep="trailing content after pilot end_turn" \
  --format="%ad" --date=format:'%Y-%m' | sort | uniq -c
git log origin/main --grep="auto-PR-create rescue" \
  --format="%ad" --date=format:'%Y-%m' | sort | uniq -c
```

**Les deux familles ne s'additionnent pas.** La seconde produit des commits **vides** : elle ne
mesure pas du contenu rescapé mais un artefact de plomberie. Les confondre — ce que ferait un
`grep` sur le seul jeton `mika#1383` — fabriquerait un chiffre qui ne désigne aucune population
réelle.

**La rafale est en cours, pas résiduelle.** Distribution journalière de septembre pour
*trailing content* (`--date=format:'%Y-%m-%d'`) : 1 le 08, puis 1 · 2 · 1 · **6** · 4 les
15–19, puis 1 le 21 · 3 le 22 · 1 le 23 · **1 le 24** — c'est-à-dire **le jour même de la
rédaction de ce document**, et il s'agit de l'avant-dernier commit de `origin/main` :

```
0e1b0634  2026-09-24  wip(mika#2455): trailing content after pilot end_turn (mika#1383) (#2466)
```

Le grooming initial avait vu le début de cette rafale et l'avait sous-comptée d'un facteur 5 sur
juin, en ignorant entièrement la seconde famille — 71 occurrences en septembre.

### Pourquoi ce n'est pas un doublon de mika#2157

`docs/solutions/dev-loop/rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md`
(mika#2157) traite du même filet, et a été lu avant de conclure. Son objet est **ce que le
filet déclare** : `Closes #N` était posé sans condition dans le corps de la PR de récupération,
si bien qu'un worktree de grooming ne contenant que deux lignes de journal d'audit pouvait
fermer un bug p1. Son correctif est `_rescue_diff_carries_work` plus un marqueur lisible par
machine.

F1 porte sur **la cause productrice**, pas sur la déclaration : *pourquoi* un pilote termine en
laissant du contenu non commité après `end_turn`, et *pourquoi* le chemin d'ouverture de PR a
besoin d'un commit vide. mika#2157 rend le filet honnête sur ce qu'il a capturé ; il ne réduit
pas le nombre de fois qu'il doit capturer. Les deux causes sont disjointes : **F1 mérite son
propre ticket.**

Contenu à y porter, prêt : les **deux** distributions mensuelles et journalières, les **deux**
sites dans `dispatch-lib.sh`, les commandes ci-dessus, et l'énoncé de non-additivité.

---

## 6. Ce qui reste à l'opérateur, et pourquoi ça ne peut pas être tranché ici

Trois gestes n'appartiennent pas à un plan ni à un pilote, et sont consignés plutôt
qu'exécutés :

1. **Ouvrir le ticket F1** — `gh` n'est authentifié dans aucune session de dispatch (§ 2). Le
   § 5 en porte le contenu intégral.
2. **Décider du sort de mika#1694.** L'alternative n'a pas pu être tranchée : `gh issue view
   1694` est inaccessible.
   - S'il est **fermé** : la condition 2 est satisfaite, F2 devient un constat historique — ce
     document — et il n'y a rien à faire. **Ne pas rouvrir.**
   - S'il est **ouvert** : y porter F2 pour re-spécifier la moitié branches, sans rien
     implémenter, et en y transportant le piège du prédicat squash-merge (§ 3.2), faute de quoi
     le prochain outil de nettoyage sera silencieusement inerte.
   - mika#2420 dit lui-même que rouvrir #1694 *« is an orchestrator gesture »* : ce n'est pas
     une omission de ce document, c'est le partage des rôles.
3. **Reaper les 141 branches** — geste destructif, dont le prédicat correct n'est même pas
   encore écrit (§ 3.2). Ne pas le faire sur `git branch --merged`.

**La fermeture de mika#1696 elle-même est un geste opérateur.** Ce document rend les cinq
conditions tranchées et adossées à des preuves reproductibles ; il ne ferme pas l'épique.

---

## 7. La leçon, et la preuve qu'elle s'applique à elle-même

**Un épique d'incident dont les conditions de clôture sont figées à l'instant de l'incident
pourrit.** Au bout de 86 jours, aucune des cinq n'est vraie telle qu'écrite :

| condition | verdict | mode de péremption |
|---|---|---|
| 1 — bundle déployé | satisfaite | la seule qui a tenu |
| 2 — #1694 livré | partiellement livrée | livrée à moitié par un ticket qui ne porte pas son nom |
| 3 — hypothèse #1687 confirmée | dépassée | cinq correctifs descendants, et les tables sont compactées |
| 4 — #1680 recadré | caduque | **auto-contradictoire** : annulée par une décision prise dans son propre corps, six heures après |
| 5 — #1686 routé | actionnable ici pour moitié | prémisse **déjà fausse** le jour de sa rédaction |

Quatre modes de péremption distincts, et le plus instructif est le dernier : la condition 5
n'a pas vieilli, elle est née fausse. Le plugin qui la réfute (`232be1e8`) datait du
2026-07-22, soit **57 jours avant** le premier grooming.

### Et le plan de grooming a péri plus vite que le ticket

Le plan rédigé pour cette clôture a été écrit le 2026-09-17, re-mesuré le 2026-09-24, puis
exécuté le même jour. En **sept jours** :

- une condition de clôture sur cinq a **changé d'état** (la 2, livrée à moitié le 20/09) ;
- une mesure chiffrée était fausse d'un **facteur 5** (F1, juin : 1 annoncé contre 5 mesurés) ;
- une **famille entière** de l'objet mesuré manquait (F1, *auto-PR-create*, 71 occurrences en
  septembre) ;
- la limite déclarée « pas de `GH_TOKEN` » s'est **répétée à l'identique**, la faisant passer
  d'accident à propriété du substrat.

Et le jour de l'exécution, la re-mesure a encore trouvé une erreur que le re-grooming n'avait
pas vue : le verdict de la condition 5 (§ 3.5), réfuté par un fichier et deux commits présents
dans le dépôt depuis des semaines.

**Le plan est ainsi devenu la preuve de sa propre thèse.** Un énoncé d'incident périme ; un
plan de grooming périme aussi, et plus vite qu'on ne l'imagine. D'où la conduite, qui est le
vrai livrable de ce document :

> **Re-mesurer avant d'écrire, jamais recopier.** Un plan de grooming n'est pas une source de
> vérité sur l'état du monde : c'est un instantané daté d'une lecture. Chaque chiffre qu'on
> reprend d'un plan doit être rejoué, et chaque verdict relu contre l'arbre — y compris, et
> surtout, ceux qui semblent les plus solides. La condition 5 était la plus catégorique des
> cinq, et c'était la plus fausse.

Corollaire opérationnel, pour la prochaine fois : **les conditions de clôture d'un épique
gagnent à être écrites comme les conditions de réveil de `docs/dormeurs.md`** — de sorte qu'un
lecteur puisse dire, *sans contexte*, si elles sont remplies. « quand mika#1694 est fermé »,
« quand `git rev-list --count A..B` rend 0 » : vérifiables. « mika#1680 reframed », « routed to
its determined fix shape » : ni l'un ni l'autre ne se vérifie, et c'est pourquoi ils ont pourri
sans que personne s'en aperçoive.

---

## 8. Références

- **Le ticket** : mika#1696 — corps du 2026-06-30, plus trois commentaires (correction du
  périmètre du revert le 2026-06-30 ~20:22Z, ajout de mika#1697 le ~21:02Z, décision
  « keep pushing glm » le 2026-07-01 ~00:30Z).
- **Les sous-tickets** : mika#1684, mika#1685, mika#1686, mika#1687, mika#1694, mika#1697 ;
  plus mika#1679, mika#1680, mika#1682, mika#1689, mika#1383.
- **Condition 1** : `d8f780ab` (#1690), `2aa9bc5c` (#1698), `5db56354` (#2017).
- **Condition 2** : `fc96a341` (#2429, mika#2420) ; `crates/mika-agent/src/worktree_reaper.rs` ;
  `docs/dormeurs.md` ; `CLAUDE.md:2066` (hors-périmètre de mika#2420).
- **Condition 3** : `ffb09cf4` (#1859) ; lignée mika#959, mika#2045, mika#2249 / mika#2277,
  mika#1712 / mika#2156.
- **Condition 5** : `50e13e59` (#1348, retrait de la skill) ; `232be1e8` (#1818, plugin
  mika-side, spec mika#1708) ; `e1342dfa` (#2384, mika#2312) ;
  `tools/mika_permission_policy/README.md` ;
  `docs/solutions/security-issues/le-classifier-ne-decide-jamais-sur-le-contenu-dun-fichier-2026-09-18.md`.
- **F1** : `skills/bundled/_shared/dispatch-lib.sh:4720` et `:8653-8655` ; `0e1b0634` (#2466) ;
  `docs/solutions/dev-loop/rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md`
  (mika#2157).
- **Classe « instrument silencieusement inerte »** : mika#2205, mika#2131.
- **Le plan de grooming dont ce document est l'exécution** :
  `docs/plans/2026-09-17-003-docs-1696-cloture-mesuree-epique-wedge-plan.md`.

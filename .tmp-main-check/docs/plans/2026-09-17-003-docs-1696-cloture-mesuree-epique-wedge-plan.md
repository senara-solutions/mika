# mika#1696 — clore l'épique du wedge du 2026-06-30 sur une mesure, pas sur un souvenir

**Ticket :** `senara-solutions/mika#1696` — *Incident — autonomous loop wedge, 2026-06-30*
**Type :** documentation d'incident + routage des restes (aucun code moteur)
**Grooming initial :** 2026-09-17 · **re-mesuré le 2026-09-24** — 2 mois et 24 jours après l'incident

> **Re-groom du 2026-09-24.** Ce plan a été ré-exécuté sept jours après sa
> rédaction. **Une de ses cinq conditions a changé d'état dans l'intervalle** et
> deux de ses mesures chiffrées étaient fausses. Les corrections sont intégrées
> ci-dessous et marquées **[re-mesuré 09-24]**. Le §8 en tire la conséquence : ce
> plan est lui-même devenu la preuve de sa propre thèse.

---

## 1. Ce que ce ticket est devenu, et pourquoi ça décide du plan

mika#1696 n'est pas un ticket d'implémentation. C'est un **parent de coordination**
ouvert le 2026-06-30 pour qu'un après-midi de pannes se lise « 1 incident,
5 sous-correctifs » plutôt que « 5 nouvelles dettes ». Son corps le dit
explicitement, et il nomme lui-même son propre livrable :

> *After closure: orchestrator-CC documents the incident as compound learning +
> closes the epic.*

Le travail de code, lui, vit dans les sous-tickets — chacun ouvert, numéroté et
dispatché séparément. **Écrire ce code sous #1696 court-circuiterait ces
sous-tickets** et rendrait la condition de clôture 2 du ticket
(« mika#1694 *has shipped* ») incohérente avec son propre véhicule : un épique ne
peut pas être le véhicule de livraison de l'enfant dont il attend la livraison.

D'où la forme de ce plan : **un document de solution + un routage explicite des
restes**, zéro ligne de moteur. Ce n'est pas un descopage — c'est le périmètre que
le ticket se donne. Le dépôt a le précédent : `#2343` (docs(2331)), `#2341`
(docs(2337)), `#2187` (docs(solutions)) sont des PR entièrement documentaires
fusionnées récemment.

## 2. La mesure : les 5 conditions de clôture, relues au 2026-09-24

Le corps de #1696 énumère cinq conditions. Voici leur état, **mesuré sur le dépôt**,
pas rappelé de mémoire. C'est l'apport réel de ce grooming : **aucune des cinq
n'est vraie telle qu'écrite** — une est satisfaite, une est à moitié livrée, deux
sont dépassées ou caduques, une n'est pas actionnable ici.

### Condition 1 — « Deploy bundle {1685+1679+1383} is live » → **satisfaite, et le filet a récidivé**

| sous-fix | état | preuve |
|---|---|---|
| mika#1685 (`--no-verify` rescue) | **fusionné** | `d8f780ab … (#1690)` — 2026-06-30 |
| mika#1679 (Edit 4 absorbé, gate #1383/#1396) | **fusionné** | `2aa9bc5c … (#1698)` — 2026-07-01 |
| mika#1684 (Step 1.5 rescue-header) | absorbé dans 1679 | — |
| filet secret-scan que `--no-verify` ne contourne pas (mika#1689) | **fusionné** | `5db56354 … (#2017)` |

Les quatre SHA ont été re-vérifiés le 2026-09-24 (`git show <sha> --no-patch`) :
tous existent et portent le message attendu. La « deploy unit » est en service et
son filet **fonctionne** — les rescues sont attrapées, promues et fusionnées.

**Mais la classe de défaut que ce filet attrape est en rafale active.** Voir F1
au §4 : les chiffres du grooming initial étaient faux, et il manquait une famille
entière.

### Condition 2 — **[re-mesuré 09-24]** « mika#1694 has shipped » → **moitié livrée le 2026-09-20 ; l'autre moitié est nommée hors périmètre**

**Le grooming du 17/09 concluait « NON livrée ». C'était vrai le 17. C'est faux
le 24.** Trois jours après la rédaction de ce plan, mika#2420 a livré la moitié
worktree de #1694 :

```
fc96a341  2026-09-20  fix(substrate): les worktrees de PR terminale
                      ne survivent plus à leur PR (mika#2420) (#2429)
```

Le corps de ce commit **nomme #1694 et rectifie sa lecture** :

> *« #1694 n'est pas "fermé mais inefficace". C'est un dormeur dont la condition
> de réveil est remplie, dont la logique n'a jamais atteint `main` (commit
> `097cc66c`, sauvé en `wip()` par la recovery mika#1282) »*

Trois preuves locales concordantes, toutes reproductibles sans GitHub :

| preuve | commande | résultat au 09-24 |
|---|---|---|
| le reaper moteur existe | `wc -l crates/mika-agent/src/worktree_reaper.rs` | **5004 lignes** |
| le dormeur a été retiré du registre | `grep -n 1694 docs/dormeurs.md` | **aucune ligne** |
| le retrait est daté par ce commit | `git log --format=… -- docs/dormeurs.md` | `fc96a341`, 2026-09-20 |

Le registre des dormeurs porte un contrat explicite : *« Quand la condition est
remplie, rouvrir le ticket GitHub cité et retirer la ligne d'ici. »* La ligne a
été retirée — donc le réveil a eu lieu.

**Mais la condition n'est satisfaite qu'à moitié, et la seconde moitié est
explicitement exclue**, pas oubliée. `CLAUDE.md` § *terminal-worktree reaper*
énumère le hors-périmètre de mika#2420 : les couches A et B de #1694
(`worktrees-audit` / `worktrees-clean`, qui vivent dans le dépôt `mika-platform`
et restent le geste manuel), **les branches distantes**, et *« reopening #1694
itself, which is an orchestrator gesture the PR body signals rather than
performs »*.

État des deux axes de #1694, du 2026-06-30 à aujourd'hui :

| axe | 06-30 (corps du ticket) | 09-17 | **09-24** | verdict |
|---|---|---|---|---|
| worktrees | 13 | 2 | **1** | **livré** par mika#2420 (moteur) + skill opérateur |
| branches `origin` non fusionnées | « 30+ » | 166 | **141** | **non livré**, hors périmètre de mika#2420 |

Commandes : `ls /data/workspace/mika-platform/.claude/worktrees/ | wc -l` ·
`git branch -r --no-merged origin/main | wc -l`.

Contexte de la dette de branches : **168 refs `origin` au total**, dont **102
antérieures à septembre** (`git for-each-ref --format='%(committerdate:short)'
refs/remotes/origin | sort | grep -c "^2026-0[1-8]"`), la plus ancienne datant du
**2026-03-27** — six mois de branches mortes. Le chiffre a *baissé* de 166 à 141
en une semaine, mais **cette baisse n'est pas interprétable** : voir le piège
ci-dessous.

**Le piège structurel du grooming initial tient, et il vaut plus que jamais.** Ce
dépôt fusionne en **squash** : chaque titre de `main` porte un `(#NNNN)`. Après
un squash-merge, la branche d'origine **n'est jamais ancêtre de `main`**. Donc :

- `git branch -r --no-merged origin/main` compte 141 branches — dont la quasi-
  totalité sont déjà fusionnées côté forge. Le chiffre **surestime massivement**,
  et ses variations d'une semaine sur l'autre ne mesurent rien d'exploitable.
- Symétriquement, un `worktrees-clean` bâti sur le prédicat naïf
  `git branch --merged` **ne reaperait jamais rien**, sur aucune branche, en
  silence. Il se lirait comme un outil qui n'a rien trouvé à nettoyer.

C'est la classe de panne que ce dépôt documente à répétition : *un instrument
silencieusement inerte se lit exactement comme un instrument qui n'a rien trouvé*
(mika#2205, mika#2131). Un futur reaper de branches doit interroger **l'état de
la PR côté forge** (`gh pr list --state merged --head <branch>`), jamais
l'ancestry git. Ce constat appartient à la moitié non livrée de #1694 et doit y
être porté (§4, F2) — il n'est pas implémenté ici.

### Condition 3 — « mika#1687 hypothesis confirmed for both stuck pilots » → **dépassée**

Le sous-ticket a produit un correctif livré (`ffb09cf4 … (#1859)`, 2026-07-28, le
*childless-parent reaper*), et une lignée entière l'a depuis dépassé : watchdog
PID #959, reaper `stuck-pending` #2045, reaper de stall silencieux #2249/#2277,
phantom sweep #1712/#2156. Toute cette famille est documentée dans `CLAUDE.md`.

La condition telle qu'écrite — confirmer l'hypothèse **pour les deux pilotes
bloqués** du 2026-06-30 — porte sur deux lignes de tâche vieilles de près de trois
mois. Elle n'est ni vérifiable (les tables ont été compactées depuis) ni utile :
la classe a reçu cinq correctifs structurels. **Condition à déclarer dépassée,
sans chercher à la satisfaire à la lettre.**

### Condition 4 — « mika#1680 reframed (revert + calibration suite hardening) » → **caduque, et auto-contradictoire**

Cette condition exige un recadrage dont la première moitié est le **revert de
modèle**. Or le même corps de ticket, quelques paragraphes plus haut, porte la
décision de Vincent du 2026-07-01 ~00:30Z :

> *« glm stays. Drop the revert entirely; it's not happening, so everything that
> hung off it goes too. »*

La condition de clôture 4 n'a jamais été mise à jour après cette décision : **le
ticket exige une chose que le ticket a annulée.** Le temps a tranché une seconde
fois — la flotte est passée à glm-5.3/kimi depuis (cf. `CLAUDE.md` § budgets LLM
et mika#2473, qui mesure la dérive modèle code↔runtime), donc « revert vers
Sonnet-4-6 » ne désigne plus aucun état atteignable.

Ce qui reste de #1680 (le *CJK bleed*, classé par Vincent « tracked annoyance, not
a blocker ») n'a aucun commit dans ce dépôt. Sa seconde moitié — durcir la suite de
calibration en garde de swap de modèle — est un vœu, pas une condition de clôture
d'incident. **Condition à déclarer caduque**, le reste routé (§4, F4).

### Condition 5 — « mika#1686 routed to its determined fix shape » → **non actionnable dans ce dépôt**

Le classifieur de permission que #1686 met en cause est celui de **claude-pilot**
(le corps le dit : « cpp permission classifier »). Et la skill `permission-policy`
a été **retirée de ce dépôt** : `50e13e59 chore(mika): retire mika-relay agent +
permission-policy skill (mika#1193) (#1348)`, 2026-05-30. Re-vérifié le 09-24 :
absente de `skills/bundled/` **et** de `templates/skills/`.

Le défaut, lui, est toujours vivant — et **reproduit dans chacune des deux
sessions de grooming** :

| date | commande refusée | message |
|---|---|---|
| 2026-09-17 | enchaînement `git log --grep` + `echo` | `policy allow (bash-grep) vetoed — command chains a tier3-dangerous or command-substitution tail` |
| **2026-09-24** | `git for-each-ref … \| sort \| awk '$1 < "2026-09-01"'` | `no matching policy rule -- denied by default` |

Deux refus, deux sessions, deux formes différentes, sur des commandes de lecture
pure. C'est la forme n=8+ que #1686 décrivait le 2026-06-30, inchangée trois mois
plus tard.

Mais son substrat vit dans `senara-solutions/claude-pilot`, **hors de l'allowlist
de dépôts dispatchables de la boucle** (même contrainte que claude-pilot#168, cf.
`CLAUDE.md`). Aucune PR sur `senara-solutions/mika` ne peut satisfaire cette
condition. **À router, pas à satisfaire** (§4, F3).

## 3. Périmètre de ce travail

**Dans le périmètre :**

1. Un document `docs/solutions/dev-loop/` portant l'apprentissage composé de
   l'incident — le livrable que #1696 se donne lui-même.
2. Le routage explicite des restes en constats nommés (F1–F5), chacun avec son
   véhicule : ticket de suivi, sous-ticket existant, ou hors dépôt.

**Hors périmètre, délibérément :**

- Toute implémentation de la moitié **branches** de **mika#1694** — elle est
  explicitement hors périmètre de mika#2420, et reste un sous-ticket distinct dont
  #1696 attend la livraison.
- Toute intervention sur **mika#1686** — substrat hors dépôt depuis mika#1193.
- Toute correction de la récidive **mika#1383** — nouveau défaut, nouveau ticket.
- Toute modification du corps de #1696 ou commentaire sur le ticket — le pipeline
  autonome (`dispatch-lib::_write_canonical_callout`) en est le seul écrivain.
- Le reaping effectif des 141 branches — un geste destructif qui n'appartient ni à
  un plan ni à un pilote, et dont le prédicat correct n'est pas encore écrit.
- La fermeture de **#1696** elle-même : fermer un épique est un geste opérateur.

**Gate mika#2306 — N/A, déclaré.** Ce plan ne livre **aucun détecteur** : ses
livrables sont un document de solution, un ticket de suivi et deux commentaires.
Aucun test, lint, garde CI, validateur, scan structurel ni garde EndTurn. La
section `## Fire-Disposition` n'est donc pas requise et n'est pas inventée.

## 4. Les constats à router (contenu du document)

| id | constat | véhicule |
|---|---|---|
| **F1** | **[re-mesuré 09-24]** mika#1383 récidive en rafale active, et il y a **deux familles distinctes**, pas une. Voir le détail ci-dessous. | **Nouveau ticket.** La condition 1 de #1696 portait sur le *déploiement* du bundle, qui a eu lieu. |
| **F2** | **[re-mesuré 09-24]** La moitié worktree de #1694 est **livrée** (mika#2420, `fc96a341`, 2026-09-20 — reaper moteur + retrait du dormeur). La moitié **branches** ne l'est pas et est explicitement hors périmètre. Et en dépôt squash-merge, un prédicat `git branch --merged` est **structurellement inerte** — le prédicat correct lit l'état de la PR côté forge. | **Porter sur mika#1694** (re-spécification de la moitié restante). Ne pas implémenter ici, **ne pas rouvrir par réflexe** — voir la condition de halte au §7. |
| **F3** | #1686 toujours vivant — reproduit dans les **deux** sessions de grooming, sous deux formes de refus différentes — mais son substrat (`permission-policy`) a quitté ce dépôt en mika#1193/#1348. | **Router vers `senara-solutions/claude-pilot`**, par spawn manuel (hors allowlist de la boucle). |
| **F4** | La condition de clôture 4 exige un revert que le même ticket a annulé le 2026-07-01, et que le temps a rendu inatteignable. | **Déclarer caduque** dans le document. |
| **F5** | La condition 3 (#1687) est dépassée par cinq correctifs structurels descendants ; sa lettre porte sur deux tâches de juin non vérifiables. | **Déclarer dépassée** dans le document. |

### F1 en détail — deux familles, et le grooming initial en mesurait une seule, avec de mauvais chiffres

Le grooming du 17/09 annonçait « 1 en juin, 0, 0, 4 en septembre ». **Les deux
moitiés de cet énoncé sont fausses.** `dispatch-lib.sh` porte **deux** mécanismes
de rescue distincts sous le même numéro de ticket :

| famille | site | nature | distribution sur `origin/main` |
|---|---|---|---|
| *trailing content* | `dispatch-lib.sh:4720` | commit du contenu résiduel après `end_turn` | **5** juin · 0 juil · 0 août · **21** sept |
| *auto-PR-create* | `dispatch-lib.sh:8653-8655` | commit **vide** (`--allow-empty`) pour permettre l'ouverture d'une PR | 0 juin · **5** juil · **2** août · **71** sept |

Commandes (mesure sur `origin/main` seul — un `--all` compte double, la branche
*et* son squash) :

```bash
git log origin/main --grep="trailing content after pilot end_turn" \
  --format="%ad" --date=format:'%Y-%m' | sort | uniq -c
git log origin/main --grep="auto-PR-create rescue" \
  --format="%ad" --date=format:'%Y-%m' | sort | uniq -c
```

**Les deux familles ne s'additionnent pas** : la seconde produit des commits
**vides**, donc elle ne mesure pas du contenu rescapé mais un artefact de
plomberie. Les confondre — ce que ferait un grep sur le seul jeton `mika#1383` —
fabriquerait un chiffre qui ne désigne aucune population réelle.

**La rafale est en cours, pas résiduelle.** Distribution journalière de septembre
pour *trailing content* : 1 le 08, puis 1 · 2 · 1 · **6** · 4 les 15–19, puis
1 · 3 · 1 · **1 le 24 septembre** — c'est-à-dire le jour même de ce re-grooming,
et il s'agit de l'avant-dernier commit de `main` (`0e1b0634`, mika#2455). Le
grooming initial avait vu le début de cette rafale et l'avait sous-comptée d'un
facteur 5.

**Vérification préalable à l'ouverture du ticket F1 :**
`docs/solutions/dev-loop/rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md`
(mika#2157) traite déjà du filet de rescue. Le lire **avant** d'ouvrir F1, et
n'ouvrir que si la cause productrice qu'il décrit diffère de celle mesurée ici —
sinon commenter ce ticket-là plutôt qu'en créer un doublon.

**Leçon transversale, qui est la raison d'être du document :** un épique
d'incident dont les conditions de clôture sont figées à l'instant de l'incident
**pourrit**. Au 2026-09-24, à presque trois mois, **aucune des cinq n'est vraie
telle qu'écrite** : une satisfaite, une à moitié livrée par un véhicule qui ne
porte pas son nom, une dépassée par cinq correctifs descendants, une
auto-contradictoire avec une décision prise dans son propre corps, une visant un
substrat sorti du dépôt. Un épique doit être daté, et **re-mesuré avant d'être
exécuté** — jamais exécuté sur son énoncé d'origine.

## 5. Limite de ce grooming, déclarée — et elle se répète

**Aucune des deux sessions de grooming n'a eu de `GH_TOKEN`.** Vérifié le
2026-09-24 : `gh auth status` rend *« You are not logged into any GitHub hosts »*.

Ce n'est donc plus un accident de session : **c'est une propriété du bac à sable
de dispatch**, à traiter comme telle. Tout l'état ci-dessus est reconstruit depuis
le dépôt local — `git log`, `git branch -r`, `git for-each-ref`, le `Makefile`,
`docs/dormeurs.md`, l'arbre des skills et des crates.

Conséquence pour l'implémenteur : **l'état *GitHub* des sous-tickets (#1694,
#1686, #1687, #1680, #1697) n'a pas pu être lu.** Un sous-ticket peut avoir été
fermé sans commit dans ce dépôt. Le premier geste de l'implémentation reste de
relire ces cinq états via `gh issue view`, et **d'amender le document si un état
contredit le §2** — un document qui affirmerait « non livré » sur un ticket livré
serait pire qu'un document absent.

**Ce re-groom démontre que cette précaution paie, et qu'elle a une alternative.**
La condition 2 a basculé sans qu'aucune lecture GitHub soit possible : elle a été
tranchée sur **trois preuves locales concordantes** (le fichier du reaper, le
retrait de la ligne du registre, le corps du commit qui nomme #1694). Quand une
preuve locale existe, la préférer — elle est reproductible par n'importe qui,
avec ou sans jeton.

## 6. Étapes d'implémentation

1. **Relire l'état GitHub des cinq sous-tickets** (`gh issue view <n> --repo
   senara-solutions/mika` pour 1694, 1686, 1687, 1680, 1697). Noter état et date de
   fermeture éventuelle. **Amender le §2 du document en cas de contradiction.**
   Si `gh` n'est toujours pas authentifié, le **consigner dans le document** et
   s'en tenir aux preuves locales — ne pas bloquer, ne pas deviner.
2. **Ré-exécuter les mesures du §2 et du §4** avant d'écrire. Elles ont bougé en
   sept jours ; elles bougeront encore. Chaque chiffre du document porte sa
   commande **et sa date de mesure**.
3. **Écrire** `docs/solutions/dev-loop/epique-dincident-perime-avant-son-execution-<date-du-jour>.md`
   avec le frontmatter YAML de la maison, aligné sur le voisin le plus récent
   (`rescue-net-closes-without-looking-at-what-it-captured-2026-09-04.md`) :
   `module`, `tags`, `problem_type`, `category`, `created`, `ticket`. Le document
   porte : l'anatomie du wedge (résumée depuis le corps), le tableau des cinq
   conditions relues avec leur verdict, les cinq constats F1–F5 avec leur
   véhicule, et la leçon transversale du §4.
4. **Lire `rescue-net-…-2026-09-04.md` (mika#2157) avant d'ouvrir F1.** Si la
   cause productrice y est déjà décrite, commenter ce ticket au lieu d'en créer un
   nouveau. Sinon, ouvrir le ticket de suivi F1 via `/mika-issue`, en y portant
   **les deux** distributions mensuelles et la distinction des deux familles avec
   leurs deux sites dans `dispatch-lib.sh`.
5. **Porter F2 en commentaire sur mika#1694** : la moitié worktree livrée par
   mika#2420 (`fc96a341`), la moitié branches restante, et le piège du prédicat
   squash-merge. **Ne rien implémenter, et ne pas rouvrir le ticket par réflexe** —
   voir la condition de halte au §7.
6. **Porter F3 en commentaire sur mika#1686** : substrat hors dépôt depuis
   mika#1193, à traiter par spawn manuel sur `claude-pilot` ; joindre les deux
   reproductions datées du §2.
7. Vérifier que rien d'autre que le document n'est modifié (`git diff --stat`).

**Note de dispatch (mika#2211) :** le corps de la PR s'écrit dans un fichier
`pr-body.md` **à la racine du worktree**, passé en `--body-file pr-body.md`, puis
supprimé. Jamais dans `/tmp` (la permission-policy refuse toute écriture hors
worktree), jamais en heredoc.

## 7. Contrat de vérification

| # | quoi | comment |
|---|---|---|
| V1 | Le document existe et porte le frontmatter de la maison | `head -10` du fichier ; `module`, `tags`, `problem_type`, `category`, `created`, `ticket` présents |
| V2 | Les cinq conditions de clôture y sont chacune tranchées | une section par condition, chacune portant un verdict explicite parmi {satisfaite, partiellement livrée, dépassée, caduque, non actionnable ici} |
| V3 | Chaque mesure chiffrée est reproductible **et datée** | chaque chiffre est accompagné de la commande qui le produit et du jour où elle a été lancée |
| V4 | Aucun code moteur touché | `git diff --stat origin/main` ne montre que des fichiers sous `docs/` |
| V5 | Les constats sont routés, pas seulement listés | F1 a un numéro de ticket **ou** une justification écrite de non-ouverture ; F2 et F3 ont une URL de commentaire |
| V6 | Le pipeline passe | `scripts/verify-pipeline.sh` vert (U2 : `## Acceptance criteria` présent et non vide dans ce plan) |

**Condition de halte — déjà déclenchée en partie, lire avant d'agir sur #1694.**
Le grooming du 17/09 prévoyait : *« si #1694 est fermé et livré par un véhicule
invisible au dépôt local, la condition 2 est satisfaite ; F2 devient un constat
historique et l'étape 5 disparaît. Ne pas rouvrir #1694 par réflexe. »*

Le re-groom du 09-24 établit que **le véhicule existe et est visible localement**
(mika#2420), mais qu'il ne couvre **que la moitié worktree**. Donc :

- **Si `gh issue view 1694` le montre fermé** → la condition 2 est satisfaite,
  F2 devient un constat historique consigné dans le document, l'étape 5
  disparaît. **Ne pas rouvrir.**
- **S'il est ouvert** → poster F2 en commentaire pour re-spécifier la moitié
  branches, sans rien implémenter.
- **Si `gh` reste inaccessible** → consigner les deux branches de l'alternative
  dans le document et laisser la décision à l'opérateur. Ne pas trancher à
  l'aveugle : mika#2420 dit lui-même que rouvrir #1694 *« is an orchestrator
  gesture »*.

## 8. Ce que ce re-groom a démontré, et qui appartient au document

Ce plan a été rédigé le 2026-09-17 et ré-exécuté le 2026-09-24. En **sept jours** :

- une condition de clôture sur cinq a **changé d'état** (la 2, livrée à moitié le
  20/09 par un ticket qui ne porte pas son nom) ;
- une mesure chiffrée était fausse d'un **facteur 5** (F1, juin : 1 annoncé contre
  5 mesurés) ;
- une **famille entière** de l'objet mesuré manquait (F1, `auto-PR-create`, 71
  occurrences en septembre) ;
- la limite déclarée au §5 (pas de `GH_TOKEN`) s'est **répétée à l'identique**,
  la faisant passer d'accident à propriété du substrat.

**Le plan est ainsi devenu la preuve de sa propre thèse.** Un énoncé d'incident
périme ; un plan de grooming périme aussi, et plus vite qu'on ne l'imagine. C'est
l'argument le plus fort du document à écrire, et il doit y figurer — non comme
une anecdote de procédure, mais comme la raison pour laquelle l'étape 2 du §6
prescrit de **re-mesurer avant d'écrire** plutôt que de recopier ce plan.

## Definition of Done

- Le document de solution est écrit, commité, et lisible sans le corps du ticket
  d'origine (il se suffit à lui-même).
- Les cinq conditions de clôture de mika#1696 sont chacune tranchées par un verdict
  explicite, chaque verdict adossé à une preuve reproductible (SHA, commande, ou
  citation datée du ticket) **portant sa date de mesure**.
- F1 a donné lieu à un ticket ouvert **ou** à une justification écrite de
  non-ouverture (doublon de mika#2157) ; F2 et F3 ont donné lieu à un commentaire
  sur leur sous-ticket respectif, ou au constat consigné prévu par la condition de
  halte.
- Aucun fichier hors `docs/` n'est modifié.
- `scripts/verify-pipeline.sh` est vert.
- mika#1696 est prêt à être fermé par l'opérateur — le plan ne le ferme pas
  lui-même (la fermeture d'un épique est un geste opérateur).

## Acceptance criteria

*Le corps de mika#1696 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de sa « Closure condition » et du contrat de
vérification du §7.*

- **AC1** — Un fichier `docs/solutions/dev-loop/*.md` existe, porte le frontmatter
  YAML aligné sur le voisin le plus récent du répertoire (`module`, `tags`,
  `problem_type`, `category`, `created`, `ticket`), et documente l'incident du
  2026-06-30.
- **AC2** — Le document tranche **les cinq** conditions de clôture de mika#1696,
  une par une, chacune avec un verdict explicite et une preuve : condition 1
  satisfaite (SHA `d8f780ab`, `2aa9bc5c`), condition 2 **partiellement livrée**
  (SHA `fc96a341` du 2026-09-20 pour la moitié worktree, moitié branches nommée
  hors périmètre), condition 3 dépassée (SHA `ffb09cf4` et lignée), condition 4
  caduque (citation datée de la décision du 2026-07-01 qui l'annule), condition 5
  non actionnable ici (SHA `50e13e59`).
- **AC3** — Le document porte les **deux** distributions mensuelles mesurées des
  rescues mika#1383 — *trailing content* et *auto-PR-create* — **séparément**, avec
  leurs deux sites dans `dispatch-lib.sh` et les commandes qui les reproduisent, et
  il énonce pourquoi elles ne s'additionnent pas (la seconde produit des commits
  vides).
- **AC4** — Le document porte l'état des deux axes de #1694 à sa date de
  rédaction — worktrees (livré par mika#2420) et branches `origin` (non livré, hors
  périmètre) — **et** l'énoncé du piège squash-merge : un prédicat fondé sur
  `git branch --merged` est structurellement inerte dans ce dépôt, le prédicat
  correct interroge l'état de la PR côté forge.
- **AC5** — Le document porte la leçon transversale : un épique d'incident dont les
  conditions de clôture sont figées à l'instant de l'incident périme et doit être
  re-mesuré avant d'être exécuté ; il cite comme preuve la péremption **de ce plan
  lui-même en sept jours** (§8).
- **AC6** — Pour F1 : soit un ticket de suivi est ouvert portant les deux
  distributions et son numéro est cité dans le document, soit le document justifie
  par écrit pourquoi il est un doublon de mika#2157 et renvoie à ce ticket.
- **AC7** — Un commentaire est posté sur mika#1694 portant F2, et un sur mika#1686
  portant F3 — sauf si l'étape 1 établit que le sous-ticket concerné est déjà
  fermé, ou si `gh` est inaccessible, auquel cas le document consigne l'état et
  l'alternative à la place (condition de halte, §7).
- **AC8** — `git diff --stat origin/main` ne liste aucun fichier hors `docs/` :
  aucune ligne de moteur, de skill ou de script n'est modifiée par cette PR.
- **AC9** — `scripts/verify-pipeline.sh` termine sans échec.

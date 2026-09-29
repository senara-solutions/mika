# chore(observability): le filet de reprise ne stashe plus son propre échafaudage, et le reset qui ne mordait pas est nommé

**Ticket :** mika#2144 — *136 stashes de bruit pour 16 vrais*
**Date :** 2026-09-29
**Périmètre :** `skills/bundled/_shared/dispatch-lib.sh` (Tier 2/Tier 3 de `_clean_worktree_for_rebase`), `skills/bundled/_shared/test-dispatch-lib.sh`

---

## 1. Ce que la lecture du code et la re-mesure déplacent dans le ticket — c'est le premier livrable

Le ticket a été mesuré le **2026-09-02**. Ce plan est écrit le **2026-09-29**. Cinq
rectifications sortent de la lecture, et chacune change ce qu'il reste à livrer.

### R1 — La pile de stash est VIDE, et pas par ce ticket. AC4 est sans objet, AC5 est inexécutable.

Mesuré sur `/data/workspace/mika-platform/mika` (pile partagée par tous les worktrees
du dépôt, `git-common-dir` confirmé) :

| mesure | 2026-09-02 (ticket) | 2026-09-29 (ce plan) |
|---|---|---|
| `git stash list \| wc -l` | 163 | **0** |
| `refs/stash` | — | **présent**, `20bb563e`, `2026-09-22 12:25:38 +0200`, `On main: mpc-2026-09-22-egress-test-2152-preff` |
| `.git/logs/refs/stash` | — | **absent** |

`git stash list` lit le **reflog** de `refs/stash`, jamais la ref. Le fichier de reflog
a disparu : la pile n'a pas été dépilée entrée par entrée, elle a été **effacée en
bloc**, et l'effacement est postérieur au 2026-09-22 (date du sommet, encore là). Il
n'est attribuable à aucun mécanisme du dépôt — la recherche de `stash.*(drop|clear|
prune|expire)` sous `scripts/`, `crates/` et `.github/` ne rend que deux `stash drop`
de fixture dans `scripts/test-shell-exec-guard.sh`, qui opèrent sur leur propre
worktree jetable.

Conséquences, à écrire plutôt qu'à contourner :

- **AC4 est sans objet.** Sa vérification (`git stash list | wc -l` tombe à 16) rend
  **0**. La purge a eu lieu, ce ticket n'en est pas l'auteur, et prétendre le
  contraire serait s'attribuer un résultat qu'on n'a pas produit.
- **AC5 est inexécutable, et c'est une perte réelle.** Il déclarait *non négociable*
  que les 16 stashes de code soient inventoriés un par un et qu'« aucun ne soit
  supprimé sans cette ligne ». **Ils l'ont été.** La donnée primaire — la liste, les
  branches d'origine, les contenus — n'est plus lisible. Ce plan ne la fabrique pas.

Une récupération reste **plausible et non établie** : le dépôt porte 5 454 commits
inatteignables (`git fsck --unreachable --connectivity-only`), donc les objets de
stash n'ont vraisemblablement pas été élagués. Les remonter demande un `fsck` complet
et un filtrage par message (`WIP on` / `On ` / `index on `) sur l'hôte : c'est un
**geste opérateur**, hors de portée d'un dispatch (§ 8).

### R2 — AC1, AC2 et AC6 sont DÉJÀ tenus pour les chemins trackés

Le ticket décrit comme absent un comportement qui existe depuis mika#1414 :

- le **Tier 2** remet les quatre chemins d'échafaudage à HEAD **avant** que le Tier 3
  ne sonde `status --porcelain`. Quand le résidu est entièrement de l'échafaudage
  **tracké**, le statut est vide et le Tier 3 ne s'arme pas ;
- `test-dispatch-lib.sh:2694` — `test_resume_surgical_only_no_stash` (« Test 12j ») —
  **teste exactement AC1 et la moitié négative d'AC6** : dirt d'échafaudage seul →
  reset chirurgical, `RESUME_CLEANUP_STASH` vide, aucun stash dans la liste ;
- `test-dispatch-lib.sh:2597` — « Test 12i » — teste **AC2** : un tracké non-échafaudage
  modifié plus un untracked hors périmètre sont stashés, le SHA immuable est capturé,
  le contenu est récupérable.

Ce qui n'est pas tenu est le cas **untracked sous un chemin possédé**. C'est le trou,
et c'est tout le trou.

### R3 — La cause d'AC3 : un reset qui ne peut pas mordre

`git checkout HEAD -- docs/plans/` restaure les entrées **connues de l'arbre**. Il ne
supprime jamais un fichier untracked — comportement git, pas accident local. Donc :

1. un pilote de groom écrit `docs/plans/<date>-<nnn>-…-plan.md` ;
2. il meurt avant de committer — signature de mika#2141, ouverte depuis le 2026-08-04 ;
3. à la reprise, `_clean_worktree_for_rebase` passe : le Tier 2 ne touche pas le
   fichier (untracked), `docs/plans/` n'est pas ignoré ;
4. `status --porcelain` rend `?? docs/plans/…-plan.md` ;
5. le Tier 3 **stashe**, imprime sa ligne de récupération, puis `clean -fd` supprime
   le fichier.

Un stash ne contenant que `docs/plans/…` est exactement ce que la mesure du ticket
classe « échafaudage ». **Les 11 stashes du seul 2026-09-01 sont de cette forme**, et
c'est le débit le plus élevé depuis juin parce que mika#2141 était grand ouvert.

Le même raisonnement vaut pour `.claude/commands/` : un fichier untracked que le seed
n'a pas shieldé y survivrait au Tier 2. Et `.claude/groom-verdict-trail.log` est
**non tracké**, donc son `git checkout -- …` du Tier 2 échoue à chaque passage,
absorbé par `2>/dev/null || true` — une ligne morte, sans conséquence aujourd'hui
(§ R4) mais qu'il faut avoir vue avant de raisonner sur ce bloc.

### R4 — L'hypothèse du ticket sur `.iterate/` est réfutée par le code

Le ticket écrit : *« Ignorer le répertoire n'a pas empêché `git stash push
--include-untracked` de continuer à le capturer. »* C'est faux : `--include-untracked`
n'inclut pas les fichiers **ignorés** (c'est `--all` qui le ferait), et la sonde qui
arme le Tier 3, `git status --porcelain`, ne liste pas les ignorés non plus. Un
`.iterate/` réellement ignoré ne peut ni armer le Tier 3 ni entrer dans son stash.

La cause du débit `.iterate/` pur est ailleurs, et elle est structurelle :
**`_clean_worktree_for_rebase` tourne AVANT le rebase**, donc sous le `.gitignore`
du HEAD de la **branche**, jamais celui de `main`. Sur un worktree repris dont la
branche précède `ac1ee87e` (2026-07-28), `.iterate/` n'est ni ignoré ni — c'est le
point — forcément absent de l'index : `ac1ee87e` s'intitule *untrack*, donc avant lui
ces chemins étaient **suivis**. Un index de branche ancienne les voit modifiés, et un
fichier tracké reste tracké quoi qu'en dise `.gitignore`. C'est la première des trois
hypothèses que le ticket lui-même énumère ; c'est celle que le code soutient.

Deux dates bornent la fenêtre et cadrent la lecture rétrospective :

| chemin | ignoré depuis | conséquence aujourd'hui |
|---|---|---|
| `.claude/groom-verdict-trail.log` | `ee355b6f`, 2026-06-30 (#1675) | hors de la fenêtre de mesure — jamais producteur des 18 |
| `.iterate/` | `ac1ee87e`, 2026-07-28 (#1862) | producteur **seulement** via une branche antérieure |
| `docs/plans/` | **jamais** | **le seul producteur résiduel à ce jour** |

### R5 — La liste du ticket est plus large que ce que le Tier 2 possède

AC1 écrit les chemins possédés comme « `.iterate/`, `docs/plans/`, `.claude/` ».
Le Tier 2 ne possède pas `.claude/` en entier : il possède `.claude/commands/` et
`.claude/groom-verdict-trail.log`. Or `.claude/settings.json` et
`.claude/claude-pilot.json` sont **trackés** — du contenu réel du dépôt. Les classer
« échafaudage » ferait qu'une modification de la configuration du dépôt ne serait
plus stashée : une perte silencieuse introduite par un ticket qui prétend protéger le
filet. **La liste implémentée est celle du Tier 2, quatre chemins, pas la formulation
large du ticket.**

---

## 2. Le remède

**Le Tier 3 décide, le Tier 2 ne change pas.** C'est la lettre d'AC1 (« le Tier 3
n'émet plus de stash quand le résidu est entièrement composé de chemins que le Tier 2
possède ») et c'est aussi la conception la plus petite : `reset --hard` + `clean -fd`
tournent déjà dans les deux cas, donc **le nettoyage est inchangé** et seule
l'émission du stash bouge.

L'alternative — étendre le Tier 2 à un `git clean -fd -- docs/plans/ .claude/commands/`
— est écartée : elle change le nettoyage **et** la décision au même endroit, elle
déplace le trou d'un cran (un cinquième chemin l'aurait rouvert), et elle rend
invisible la question qu'AC1 pose, qui est une question de **classement**.

### 2.1 `_residue_is_scaffold_only` — le classificateur de statut

```
Args : $1 — worktree dir
Rend  : 0  le résidu est ENTIÈREMENT de l'échafaudage possédé par le Tier 2
        1  dans TOUS les autres cas
```

Lit `git -C "$wt" -c core.quotePath=false status --porcelain -z`, un chemin par
itération, et rend 1 au **premier** chemin hors liste.

`core.quotePath=false` + `-z` est **porteur, pas de l'hygiène**, et la raison est déjà
écrite dans ce fichier pour `_rescue_diff_carries_work` : sous le défaut de git, tout
chemin portant un octet non-ASCII revient entre guillemets avec des échappements
octaux — `"docs/plans/\303\251tude-plan.md"` — qui ne matche aucun motif, tombe dans
`*)` et répond « pas de l'échafaudage ». Ce dépôt écrit ses plans en français **tous
les jours** : c'est précisément la population visée. Lecture NUL-délimitée par
`while IFS= read -r -d ''` sur une **substitution de processus**, jamais `$(...)` —
bash supprime les octets NUL dans une substitution de commande et recollerait tous
les chemins en un seul blob.

Deux détails de format `--porcelain` qu'un classificateur naïf rate :

- chaque enregistrement commence par **deux caractères de statut plus une espace**
  (`?? `, ` M `, `A  `) : le chemin commence à l'**offset 3**, et le préfixe doit être
  retiré avant le `case`, sinon aucun motif ne matche jamais et la fonction est
  inerte tout en ayant l'air correcte ;
- un **renommage** (`R `) émet en `-z` **deux** enregistrements NUL-séparés
  (destination puis origine). Les deux doivent être classés, et le résidu n'est
  « entièrement échafaudage » que si **les deux** le sont. Un renommage qui sort un
  fichier de `docs/plans/` vers un chemin de code doit stasher.

**Fail-closed vers le stash.** Statut illisible, `$wt` vide ou non-worktree, chemin
qu'on ne sait pas classer : on **stashe**. L'asymétrie est celle d'AC2, écrite en
toutes lettres à son site : un stash de trop coûte une ligne de bruit dans une pile ;
un stash manquant coûte le travail d'un pilote mort, irréversiblement, puisque le
`clean -fd` qui suit supprime le résidu de toute façon. Cette polarité est l'**inverse**
de celle du voisin `_rescue_touches_tracked_tree` (fail-open) et la **même** que celle
de `_rescue_diff_carries_work` (fail-closed) — et, comme le commentaire de ce dernier
le prescrit, chaque prédicat énonce sa propre raison à son propre site, faute de quoi
un relecteur « harmonise » celui qu'il déplace.

### 2.2 La liste a un site de déclaration unique

mika#2157 R3 écrit déjà, dans ce fichier, que les deux autorités existantes « peuvent
diverger ; quand vous ajoutez un chemin ici, ajoutez-le au classificateur aussi ». En
ajouter une **troisième** orthographe sans lecteur unique serait cette dette écrite
une fois de plus, par un ticket dont le sujet est précisément le coût d'un correctif
qu'il faut refaire.

Donc : une fonction unique `_is_scaffold_path <chemin>` porte les motifs ; elle est
consommée par `_residue_is_scaffold_only` **et** par `_rescue_diff_carries_work`.

**Les deux fonctions appelantes ne sont PAS fusionnées**, et le refus est celui que
mika#2157 a déjà tranché : l'une lit un `status --porcelain` (ce qui est sale
maintenant), l'autre un `diff origin/main...HEAD` (ce que la branche introduit). Deux
questions, deux sémantiques ; une abstraction par-dessus coûterait plus que les motifs
qu'elle mutualise.

**Une divergence de liste est assumée et nommée** : `_rescue_diff_carries_work` classe
**six** chemins — les quatre du Tier 2 plus `.claude/claude-pilot.json` et
`.claude/*.local.*`, qui viennent de sa seconde autorité (les exclusions `git add -A`
du rescue) et non du Tier 2. `_is_scaffold_path` porte les **quatre du Tier 2** ; les
deux motifs supplémentaires restent locaux à `_rescue_diff_carries_work`, avec le
commentaire qui dit d'où ils viennent. Les unifier élargirait la liste du Tier 3 à des
chemins que le Tier 2 ne remet pas à HEAD — donc à du contenu qu'on cesserait de
stasher sans jamais l'avoir restauré (R5).

### 2.3 Refus : pas de stash sélectif

Un `git stash push -u -- <pathspec hors échafaudage>` semblerait plus fin. Il est
écarté : il complique le chemin de récupération (un stash partiel ne restaure pas un
arbre), il multiplie les modes d'échec de `stash push` sur pathspec, et AC1 ne le
demande pas. La règle reste binaire : **tout échafaudage → aucun stash ; un seul
chemin hors liste → stash de tout**, échafaudage compris.

### 2.4 L'abstention est DITE — la sonde prospective d'AC3

L'archéologie qu'AC3 demandait n'est plus possible (R1). Ce qui la remplace est une
mesure de ce qui se **reproduit**, faute de quoi ce correctif serait exactement ce que
le ticket reproche à celui de juillet : une purge sans compteur, à refaire.

Une ligne sur stderr du dispatch, au site de l'abstention, ancrée comme toute la
famille :

```
dispatch-lib: resume_cleanup_scaffold_only paths=<n> classes=<iterate|plans|commands|trail,…> (mika#2144)
```

Elle nomme **les classes**, jamais les chemins complets — un chemin de plan porte le
numéro du ticket et le slug, et ce n'est pas au journal de les recopier. Le puits est
le `.stderr` par dispatch (`$PILOT_LOG_DIR/<task-id>.stderr`), **le même** que la
ligne de récupération d'aujourd'hui, avec la limite héritée que la voie *revise*
redirige vers un `mktemp` qu'elle supprime (Signal S).

**Régime attendu : non vide et faible.** Chaque ligne est un stash de bruit qui n'a
pas été créé. Un flot soutenu sur `classes=plans` dit que mika#2141 produit encore des
plans non committés — c'est un **résultat**, pas une panne de ce correctif, et c'est
la mesure qui devra accompagner ce ticket-là.

---

## 3. Unités d'implémentation

| # | Unité | Fichier |
|---|---|---|
| U1 | `_is_scaffold_path` — site unique des quatre motifs, avec sa raison | `dispatch-lib.sh` |
| U2 | `_residue_is_scaffold_only` — lecture `status --porcelain -z`, offset 3, renommages, fail-closed | `dispatch-lib.sh` |
| U3 | Câblage Tier 3 : sauter `stash push` quand U2 rend 0 ; garder `reset --hard` + `clean -fd` | `dispatch-lib.sh` |
| U4 | `_rescue_diff_carries_work` consomme U1 pour ses quatre motifs communs | `dispatch-lib.sh` |
| U5 | Ligne d'abstention `resume_cleanup_scaffold_only` | `dispatch-lib.sh` |
| U6 | Test : plan **untracked** seul → aucun stash (le trou de R3) | `test-dispatch-lib.sh` |
| U7 | Test AC6 : les deux branches dans un seul test — `.iterate/` non ignoré seul → aucun stash ; puis un fichier de code modifié → un stash, et son message | `test-dispatch-lib.sh` |
| U8 | Test : renommage `docs/plans/x` → `crates/…` → **stash** (contrôle négatif du piège `R `) | `test-dispatch-lib.sh` |
| U9 | Scan de source : `_is_scaffold_path` est le seul site déclarant les motifs | `test-dispatch-lib.sh` |

Les tests 12i et 12j **ne sont pas modifiés** : ils pinnent le comportement tracké et
doivent rester verts tels quels. Une modification de 12i serait le signal qu'AC2 vient
d'être entamé.

---

## Fire-Disposition

Ce plan livre des détecteurs : U6–U8 (tests de comportement) et **U9** (scan de
source, dont le chemin de succès est « aucune seconde déclaration trouvée »).

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

- **U6, U7, U8** — écrits contre le code corrigé, ils sont **verts à la livraison** et
  n'ont aucune violation préexistante à absorber. Chacun est accompagné de son
  **contrôle négatif vu rouge** avant correction : l'implémenteur exécute U6 contre le
  Tier 3 non modifié et **doit** le voir échouer, sinon le test ne mesure pas le trou
  de R3 et sa verdeur ne prouve rien (classe mika#2205).
- **U9** — allowlist `SCAFFOLD_PATTERN_DECLARATION_ALLOWED`, **livrée vide**, comparée
  **dans les deux sens** : une entrée qui ne matche plus rien fait rougir le build
  (assertion auto-nettoyante). Quand le scan tire, la résolution est de **router le
  site vers `_is_scaffold_path`**, jamais d'ajouter une ligne (doctrine mika#2201).

**Une violation préexistante est attendue et elle est nommée** : le **Tier 2 lui-même**
énumère ses quatre chemins, en quatre commandes git de reset. C'est une troisième
expression de la même notion, et elle est **irréductible** — elle ne classe pas, elle
remet à HEAD, et la fondre dans `_is_scaffold_path` changerait sa sémantique. Le scan
vise donc les **motifs de classification** (`case` sur un chemin), jamais les
commandes de reset, et le périmètre est écrit dans le prédicat plutôt que dans une
exception. Si l'implémenteur constate que le prédicat ne peut pas séparer les deux
formes proprement, U9 est **livré désarmé** (option b) avec un suivi nommé, plutôt
qu'élargi jusqu'à ne plus rien voir : *un scan silencieusement inerte se lit
exactement comme un arbre propre.*

**Anti-vacuité** : U9 échoue si `_is_scaffold_path` n'est trouvée **nulle part** dans
`dispatch-lib.sh` — un scan visant un nom mort se lit comme un scan propre.

---

## 4. Ce que ce travail n'achète PAS

- **Il ne récupère aucun des 16 stashes de code**, ni ne les inventorie. Ils ont
  disparu avant ce plan (R1). La récupération éventuelle est un geste opérateur (§ 8),
  et son échec est un **résultat** à écrire, pas un défaut de ce correctif.
- **Il ne tarit pas la source amont.** Un pilote qui meurt sans committer son plan
  continue de le faire — mika#2141, explicitement hors périmètre. Ce travail cesse
  d'en faire du bruit ; il n'en fait pas du travail sauvé.
- **Il rend le résidu d'échafaudage DÉFINITIVEMENT perdu**, là où il était stashé.
  C'est le coût qu'AC1 demande, et il faut le nommer : un plan de groom non committé
  n'est plus récupérable du tout. Ce qui rend le prix acceptable est mesuré et non
  supposé — cette récupérabilité était **fictive** : une pile de 163 entrées que
  personne ne lit, et qui vient d'être effacée en bloc sans que quiconque s'en
  aperçoive. On échange une récupérabilité théorique contre un signal lisible.
- **Il n'ajoute aucun compteur en base, aucun événement moteur.** Le seul instrument
  neuf est la ligne stderr d'U5, et **son silence ne prouve rien tant que personne ne
  la lit** — sur un chemin de reprise qui tourne quelques fois par jour, son absence
  peut simplement vouloir dire qu'aucune reprise n'a eu lieu.
- **Il ne touche pas le second site de stash** (`_set_up_worktree` pré-vol, relic de
  worktree non canonique, mika#1472/#2449). Population différente, déclencheur
  différent, et aucune mesure ne l'incrimine.

---

## 5. Surfaces opérateur

```bash
# 1. Combien de stashes de bruit n'ont PAS été créés ? (régime attendu : non vide, faible)
grep -h '^dispatch-lib: resume_cleanup_scaffold_only' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr | tail

# 2. Quelle classe domine ? — décide du suivi mika#2141
grep -ho 'classes=[a-z,]*' "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr \
  | sort | uniq -c | sort -rn

# 3. CONTRÔLE POSITIF — le filet parle-t-il encore ? (AC2)
grep -hc '^dispatch-lib: resume-cleanup stashed' \
     "${PILOT_LOG_DIR:-/var/log/claude-pilot}"/*.stderr

# 4. La pile elle-même
git -C /data/workspace/mika-platform/mika stash list | wc -l
```

L'ancrage `^dispatch-lib: ` est **obligatoire** : ce `.stderr` porte aussi la prose du
pilote, et une session qui *discute* de ce ticket produit le faux positif que
mika#2050 a mesuré sur le Signal S.

| signal | régime attendu | lecture |
|---|---|---|
| `resume_cleanup_scaffold_only` | **non vide, faible** | chaque ligne est un stash de bruit évité |
| `classes=plans` dominant | attendu | mika#2141 produit encore des plans non committés — **résultat**, pas panne |
| `resume-cleanup stashed` | **non vide** | AC2 tient : le filet attrape toujours |
| `resume-cleanup stashed` à **zéro** sur 30 jours | **anomalie** | le filet est muet — c'est la perte des 16 rejouée, § 6 halte 2 |

---

## 6. Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** `skills/bundled/` est une projection du **binaire**, pas du checkout
> (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on vient de
> bâtir — sans quoi chaque sonde ci-dessous décrit le binaire d'hier.

**S1 — le trou de R3 est fermé (première reprise après déploiement).** Une reprise sur
un worktree portant un plan untracked et rien d'autre produit une ligne
`resume_cleanup_scaffold_only` et **aucun** stash.
*Halte 1 — un stash apparaît quand même.* Ne pas élargir la liste par réflexe : lire
le `status --porcelain` du worktree, et vérifier d'abord que le préfixe de deux
caractères est bien retiré (§ 2.1). Un classificateur qui lit `?? docs/plans/x` au
lieu de `docs/plans/x` est inerte tout en ayant l'air correct.

**S2 — contrôle POSITIF, et il n'est pas décoratif (30 jours).** La ligne
`resume-cleanup stashed` doit rester **non vide**. Zéro abstention **et** zéro stash ne
prouve rien du tout : la voie de reprise n'a peut-être pas tourné.
*Halte 2 — zéro stash sur 30 jours alors que des reprises ont eu lieu.* Le filet est
devenu muet, c'est-à-dire la perte des 16 en train de se rejouer. **Désarmer d'abord**
(revert d'U3), diagnostiquer ensuite — un filet qui ne prend plus rien est pire que le
bruit qu'il remplace.

**S3 — contrôle négatif de bruit (30 jours).** Aucune ligne
`resume_cleanup_scaffold_only` sur une reprise dont le worktree portait du code.
*Halte 3 — une occurrence.* Un chemin de code est classé échafaudage : faux positif,
et il coûte le travail d'un pilote. Désarmer, puis lire quel motif a matché — le
suspect est un motif trop large (`.claude/*` au lieu de `.claude/commands/*`, R5).

**S4 — la pile ne se remplit plus.** `git stash list | wc -l` reste dans les unités.
*Halte 4 — elle repart à la dizaine.* Lire `classes=` : si le compte monte **sans**
ligne d'abstention correspondante, un producteur est hors de la liste du Tier 2, et
c'est lui qu'il faut nommer avant d'ajouter quoi que ce soit.

---

## 7. Verification Contract

| # | Vérification | Comment |
|---|---|---|
| V1 | Les tests existants 12i/12j passent **inchangés** | `make test-dispatch-lib` |
| V2 | U6 vu **ROUGE** contre le Tier 3 non modifié, puis vert | exécution manuelle avant/après U3 |
| V3 | U7 couvre les deux branches d'AC6 dans un seul test | lecture + `make test-dispatch-lib` |
| V4 | U8 : un renommage sortant de `docs/plans/` stashe | `make test-dispatch-lib` |
| V5 | U9 vert à allowlist vide, et son anti-vacuité vue rouge en renommant le symbole | exécution manuelle |
| V6 | `bash -n skills/bundled/_shared/dispatch-lib.sh` | syntaxe |
| V7 | La suite complète | `make test-dispatch-lib` puis `make check` |

**Ce qui n'est pas testable ici, écrit plutôt que découvert :** « la pile cesse de se
remplir en production » s'exécute sur l'hôte, sur la voie de reprise réelle, avec des
worktrees que le bac à sable de dispatch ne possède pas. Le contrat côté code est
*le Tier 3 s'abstient exactement quand le résidu est entièrement de l'échafaudage*, et
U6–U8 l'attestent de façon déterministe. La moitié en service est S1–S4.

---

## 8. Gestes opérateur (hors dispatch, nommés)

Ni AC4 ni AC5 ne sont livrables par un pilote dispatché : la pile de stash est
partagée par tous les worktrees du dépôt, l'environnement de dispatch interdit
explicitement d'y toucher, et l'inspection des objets inatteignables demande un `fsck`
complet sur l'hôte.

1. **Tenter la récupération des 16** — `git fsck --unreachable --no-progress`, filtrer
   les commits dont le message commence par `WIP on `, `On ` ou `index on `, les dater
   et les classer par chemins touchés. **Zéro résultat est un résultat** : il signifie
   que les objets ont été élagués et que la perte est définitive. Dans les deux cas, la
   conclusion s'écrit sur le ticket — c'est ce qui reste d'AC5.
2. **Ne PAS réinitialiser `refs/stash`.** La ref pointe encore sur `20bb563e` ; c'est
   le seul point d'entrée survivant vers un objet de stash, et le seul élément daté de
   l'incident.
3. **Comprendre la purge.** Le reflog absent n'a pas d'auteur attribuable ici. Si le
   geste est identifié comme routinier (un `gc` agressif, un script d'entretien), il
   mérite son propre ticket : un mécanisme qui efface la pile de récupération sans rien
   dire est le même défaut que celui-ci, vu de l'autre bout.

---

## 9. Hors périmètre, délibérément

- **mika#2141** — pourquoi un pilote laisse un worktree sale sans committer. Ce plan
  cesse d'en faire du bruit ; il ne le répare pas. La mesure `classes=plans` d'U5 est
  la précondition chiffrée que ce ticket-là n'a pas.
- **Le second site de stash** (`_set_up_worktree`, relic non canonique) — population et
  déclencheur différents, aucune mesure ne l'incrimine.
- **La rédaction du message de récupération** — le ticket la déclare bonne, et elle
  l'est (SHA immuable plutôt que `stash@{0}`, pour la raison écrite en `:2237-2243`).
- **L'ignore de `docs/plans/`** — écarté : les plans sont du contenu **committé** du
  dépôt ; les ignorer casserait la voie de groom entière pour fermer un trou qui se
  ferme au classement.
- **Les stashes des autres dépôts du workspace** — `mika` est le seul à porter la voie
  de reprise.
- **Fusionner les deux classificateurs** — refusé avec sa raison (§ 2.2), et le refus
  est celui que mika#2157 R3 a déjà écrit.

---

## 10. Definition of Done

- [ ] U1–U5 implémentés dans `dispatch-lib.sh`, chaque prédicat énonçant sa polarité
      et sa raison à son propre site
- [ ] U6–U9 implémentés dans `test-dispatch-lib.sh`, U6 vu rouge avant correction
- [ ] Tests 12i et 12j inchangés et verts
- [ ] `make test-dispatch-lib` vert, `make check` vert
- [ ] Le corps de PR porte les cinq rectifications (R1–R5), nomme AC4 comme **sans
      objet** et AC5 comme **inexécutable**, et ne revendique aucun des deux
- [ ] Les gestes opérateur du § 8 sont reportés sur le ticket

---

## Acceptance criteria

Transcrits du corps de mika#2144, avec leur disposition établie par le § 1.

- **AC1** — Le Tier 3 n'émet plus de stash quand le résidu est **entièrement** composé
  de chemins que le Tier 2 possède (`.iterate/`, `docs/plans/`, `.claude/`). Le stash
  reste émis, avec son message de récupération, dès qu'un seul chemin hors de cet
  ensemble est présent.
  → *Livré par U1–U3. La liste implémentée est celle du Tier 2 (quatre chemins), pas
  `.claude/` en entier : voir R5 pour le coût de la formulation large.*
- **AC2** — Contrôle négatif, non négociable : un worktree sale portant du **code** est
  toujours stashé, et son message de récupération est toujours imprimé.
  → *Déjà tenu par le test 12i, qui n'est pas modifié ; re-attesté par U7 et U8, et
  surveillé en service par S2/S3.*
- **AC3** — La cause du débit résiduel est **établie et nommée**.
  → *Établie en R3 (un reset qui ne peut pas mordre sur un untracked) et R4 (le
  `.gitignore` en vigueur est celui de la branche, le nettoyage précédant le rebase).
  L'archéologie sur les 18 stashes est **impossible** : le reflog a été effacé (R1).
  La sonde prospective d'U5 la remplace.*
- **AC4** — Les 136 stashes d'échafaudage sont supprimés ; `git stash list | wc -l`
  tombe à 16.
  → **Sans objet.** La commande rend **0** au 2026-09-29. La purge est antérieure à ce
  travail et ne lui est pas attribuable (R1).
- **AC5** — Les 16 stashes de code sont inventoriés un par un, avec décision écrite.
  → **Inexécutable.** Ils ont été supprimés sans cet inventaire. Une tentative de
  récupération est prescrite comme geste opérateur (§ 8, point 1), dont l'échec est un
  résultat à écrire sur le ticket.
- **AC6** — Après le correctif, une exécution réelle de la voie de reprise sur un
  worktree portant du `.iterate/` **et rien d'autre** ne crée aucun stash, et la même
  exécution avec un fichier de code modifié en crée un. Les deux, dans le même test.
  → *Livré par U7, les deux branches dans un seul test. U6 ajoute la forme réellement
  productrice aujourd'hui (plan untracked), que la lettre de l'AC ne couvre pas.*

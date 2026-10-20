# Plan : orienter le pilote **avant** qu'il lise les chemins hôte du corps du ticket (mika#2108, vol 1)

**Ticket :** mika issue#2108 — `p1(dispatch-lib): le bac à sable ne monte que le worktree, mais les consignes citent les chemins de l'hôte — le pilote brûle ses tours à se chercher, puis meurt sur la forme large`
**Labels :** `bug`, `p1-important`, `ready`
**Type :** issue (bug — ralentit la boucle)
**Plan antérieur :** `docs/plans/2026-09-03-001-fix-2108-orientation-du-pilote-dans-le-bac-a-sable-plan.md` (sur `main`). Sa Phase 1 est livrée et mergée par #2791. Ce plan re-mesure et reprend ses **phases 2 à 4**.

---

## Ce que le re-mesurage a changé

Mesures exécutées le **2026-10-19** sur `main` @ `0d11df56`. Le plan antérieur
datait du 2026-09-03 (`7b4ec10a`) ; le dernier relevé de divergence du 2026-09-09
(`0592cabe`) avait halté le grooming en `ESCALATE-divergence`. Six semaines.

### P1 — La Phase 1 est livrée, et plus rien ne dort dans le worktree

`8cf94749` (#2791, 2026-10-19 08:15) porte sur `main` les trois artefacts de la
Phase 1 : le recensement
(`docs/solutions/best-practices/2108-la-surface-de-consignes-chargee-automatiquement.md`),
la garde `scripts/check-pilot-instruction-paths.sh` avec son harnais
`scripts/test-check-pilot-instruction-paths.sh`, et l'exemption annotée.

La branche de ce ticket rend `0 0` contre `origin/main` : **le `--force-with-lease`
qu'annonçait le commentaire du 09-09 est sans objet**, et la question de « sauver le
travail partiel » posée par la porte de promotion du 10-18 est close par le merge.
AC1, AC2 et AC7 du plan antérieur sont acquis et **ne sont pas rouverts ici**.

### P2 — L'hypothèse de tête du ticket est vraie aujourd'hui, par un canal neuf

Le plan de septembre concluait que la piste 1 n'avait presque rien à réécrire,
parce que la surface *chargée automatiquement* (les `CLAUDE.md`, les
`.claude/commands/*.md`) ne citait aucun chemin hôte. C'était juste, et la garde de
la Phase 1 maintient ce fait.

Depuis, `375fee47` (mika#2178) a fait du **corps du ticket et de ses commentaires**
une surface chargée automatiquement : `_render_ticket_context` les rend et le site
de composition les injecte au `PROMPT`. Mesuré aujourd'hui sur #2108 :

| surface | lignes portant la racine hôte |
|---|---|
| corps du ticket | **9** |
| commentaires retenus | **7** |

Plusieurs sont des commandes `ls` et `find` **littérales**, portant sur des chemins
qui ne résolvent pas dans le namespace du pilote. Le renderer les transporte
**verbatim** : il ne réécrit aucun chemin, et c'est délibéré.

**La garde de la Phase 1 ne peut pas couvrir cette surface.** Elle vérifie ce que le
dépôt *suit* (`git ls-files`), par un choix documenté dans son propre en-tête — le
corps d'un ticket vient de GitHub, pas de l'index. Le trou est déjà nommé dans le
document de recensement. C'est celui que ce vol ferme.

Et #2108 est le cas le plus dense de sa classe : **le ticket qui décrit le problème
est celui qui l'aggrave le plus quand on le dispatche.**

### P3 — Le squelette a grandi, et il est devenu indiscernable de l'hôte

Le plan de septembre mesurait un squelette à deux entrées (`.claude`, `mika`). Les
binds d'aujourd'hui en matérialisent **trois** :

| bind | ce qu'il matérialise sous la racine hôte |
|---|---|
| `--bind "$WORKTREE_DIR"` | `.claude/worktrees/<slug>/mika` |
| binds du gitdir (`_pilot_gitdir_bind_args`) | `mika/.git` |
| `--ro-bind-try …/claude-pilot/src` | `claude-pilot/src` |

Or `ls -a /data/workspace/mika-platform/` **sur l'hôte** rend exactement
`. .. .claude claude-pilot mika` — mesuré. Le squelette rend donc, au premier
niveau, **la même liste que l'hôte**. L'écart ne se révèle qu'au second :
`mika/` n'y porte que `.git`, sans checkout.

Le constat de septembre (« le squelette répond, et sa réponse est fausse ») n'est
donc pas seulement toujours vrai : il est **aggravé**. Un `ls` du premier niveau ne
distingue plus le bac à sable du workspace réel.

**Honnêteté de la mesure :** cette dérivation est lue sur le jeu de binds, elle
**n'a pas été sondée** dans un `bwrap` réel — la politique de permission refuse
`bwrap` depuis une session de grooming. La sonde réelle est un livrable du **vol 2**,
pas une prémisse acquise de celui-ci.

### P4 — Le site de composition du prompt tranche la position, et il le dit

L'ordre d'injection actuel, dans `_set_up_worktree` :

```
<repo>#<n>                      ← 1re ligne, contrat mika#138
[ITERATION CONTEXT]             ← réassigne PROMPT depuis zéro
CONTEXTE DU TICKET              ← mika#2178  (les 16 lignes de P2 arrivent ICI)
RÈGLE DE DISPATCH mika#2211     ← corps de PR
RÈGLE DE DISPATCH mika#2548     ← scratch
[RÈGLE mika#2306 + mika#2636]   ← conditionnées `dev-groom`
```

Le commentaire de ce site porte trois invariants de position, chacun avec sa
conséquence nommée, et une phrase qui règle la question laissée ouverte par le
relevé du 09-09 (« la position relative du bloc d'orientation n'est pas spécifiée,
et elle porte ») : *« la récence est le seul levier d'une règle en fin d'un prompt
de 16 KiB »*.

### P5 — Il y a deux sites `bwrap`, et le véhicule du contrôle négatif est `unwired`

Deux invocations : Phase 2b (nominal, réseau coupé) et Phase 2a (fallback, coupure
fs seule). Tout argument ajouté doit atterrir sur **les deux** — l'asymétrie est le
défaut naturel de ce code. *Objet du vol 2.*

`skills/bundled/_shared/tests/test_sandbox_git_usable.sh` (355 l.) est le véhicule
tout trouvé du contrôle négatif : vrai `bwrap`, vrai `_run_pilot_sandboxed`, vrai
worktree lié, les deux moitiés MUST WORK / MUST FAIL dans le même run, et un skip
propre sans `bwrap`. Mais `scripts/suite-wiring.tsv:96` la déclare **`unwired`** :
la CI ne la lance pas. Le registre statue lui-même que « needs bwrap » n'est pas
« needs the outside world », qu'une suite qui dégrade en skip **est** câblable, et
que `unwired` est du backlog et non une exemption. Y poser le contrôle négatif sans
la câbler livrerait un garde que le pipeline ne lance jamais — la classe mika#2578,
enseignée trois fois. *Le vol 2 porte donc le câblage, et le retrait de l'entrée du
registre dans le même commit (la lint est bidirectionnelle).*

### P6 — Deux prémisses écartées, mesurées plutôt que supposées

- **#2786 / #2798** (« frein après trois retenues d'ancrage ») porte sur les lignes
  d'ancrage de la réponse de l'**architecte** (`review_anchor.rs`), pas sur les
  ancres `file:line` d'un plan. Il borne la convergence du groom ; il ne déplace
  aucune prémisse des phases 2-4.
- **Les ancres du plan de septembre sont mortes une seconde fois.**
  `dispatch-lib.sh` : 5563 l. (09-03) → 6412 (09-09) → **12844** aujourd'hui. Les
  ancres de ce plan-ci nomment donc la **fonction** porteuse de l'invariant ; les
  numéros de ligne sont une aide de navigation, mesurée au 2026-10-19 sur `0d11df56`.

---

## La décision de conception

**Le bloc d'orientation est une correction de lecture, pas une prescription
d'action — et c'est sa fonction qui détermine sa position.**

Les quatre règles de dispatch déjà injectées (#2211, #2548, #2306, #2636) prescrivent
un **geste** à poser en fin de parcours : écrire le corps de PR, ranger un brouillon,
livrer une section de plan. Pour elles, la récence est le bon levier, d'où leur place
en queue de prompt.

Le bloc d'orientation ne prescrit aucun geste : il **corrige la lecture du texte qui
suit**. Les 16 lignes de chemins hôte de P2 arrivent dans `CONTEXTE DU TICKET`. Un
avertissement placé après elles arrive après que le pilote a lu les `ls` littéraux —
il console au lieu d'inoculer. Placé avant, il fait que le pilote rencontre ces
chemins en sachant déjà qu'ils ne résolvent pas.

**Donc : après le branchement `ITERATION_CTX`, et avant `TICKET_CONTEXT`.** La
fenêtre est étroite et chaque borne porte :

- **Après `ITERATION_CTX`** — ce branchement réassigne `PROMPT` depuis zéro au lieu
  d'y appendre (invariant 1 de mika#2178). Injecter avant le perd en silence à
  chaque itération.
- **Avant `TICKET_CONTEXT`** — c'est toute la fonction d'inoculation.
- **La première ligne reste `<repo>#<n>`** dans les deux branches, puisqu'on appende
  à un `PROMPT` qui commence déjà par la référence : contrat mika#138 et invariant 2
  (le parse ancré `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$`, dont l'échec fait
  tomber la dispatche en mode texte libre sans worktree).

**Le texte est produit par une fonction, pas par une constante.** Il doit nommer la
racine de travail *réelle*, que seule la dispatche connaît — « ta racine est *ici* »
vaut mieux que « tu as une racine ». La racine hôte y est **dérivée** et non écrite
en dur, selon la consigne que porte déjà la garde de la Phase 1 : étendre par
propriété, jamais par orthographe.

**Mono-dépôt.** Le point d'injection alternatif dans `claude-pilot` (`agent.py`
→ `tier1.py`, précédent `DENIED_BASH_PATTERNS_HINT`) reste écarté pour la même raison
qu'en septembre : une constante statique là-bas ne peut énoncer que du générique et
ne nommera jamais la racine réelle. Aucune PR compagnon, aucun `uv tool install` à
séquencer.

---

## Découpage en vols

Le périmètre complet des phases 2-4 dépasse la borne par vol de ce ticket. Il est
donc découpé, et **ce plan ne porte que le vol 1**.

**Vol 1 — ce plan.** Situer le pilote avant qu'il lise le corps du ticket, et le
détecteur qui le maintient. Mergeable seul : il ferme le canal mesuré en P2, se
teste **sans `bwrap` ni réseau ni token**, et est donc câblable en CI sans réserve.

**Vol 2 — la suite.** Faire dire la vérité au squelette (`README-SANDBOX` via
`--ro-bind-data`, sur les **deux** sites `bwrap`), le contrôle négatif dans un
`bwrap` réel, et le câblage CI de `test_sandbox_git_usable.sh` avec le retrait de son
entrée `unwired` du registre. Classe de risque distincte : il touche le jeu de binds
et exige le vrai bac à sable.

Tracked in: senara-solutions/mika#2108 — vol 2 (phases 3 et 4 du plan antérieur)

---

## Ce que ce vol livre

### U1 — `_pilot_orientation_rule()` dans `dispatch-lib.sh`

Une fonction, voisine de `_PR_BODY_CONTAINMENT_RULE` et `_PILOT_SCRATCH_RULE`, qui
compose le bloc en nommant la racine de travail réelle et en dérivant la racine hôte.
Elle énonce **quatre faits**, pas plus :

1. la racine de travail est `<WORKTREE_DIR>`, et c'est le cwd ;
2. le workspace de l'hôte n'est **pas** monté — ce qui est visible sous sa racine est
   un squelette de points de montage dont la liste au premier niveau est *identique*
   à celle de l'hôte, mais dont le contenu ne l'est pas : le checkout du dépôt n'y
   est pas, `<repo>/` n'y porte que `.git` ;
3. les autres worktrees et les autres sous-dépôts n'y sont pas ;
4. **un chemin sous cette racine cité par le corps du ticket ou un commentaire n'est
   pas rejouable tel quel** — c'est de la description, pas une instruction de
   navigation.

Le fait 4 est l'inoculation, et il est neuf par rapport au plan de septembre : il
existe parce que mika#2178 existe.

Borné à **12 lignes**, et la borne est un critère d'acceptation, pas un goût : le
bloc entre en concurrence d'attention dans un prompt qui porte déjà cinq injections.

### U2 — L'injection, au site et dans la fenêtre de P4

Dans `_set_up_worktree`, après le branchement `ITERATION_CTX`, avant
`TICKET_CONTEXT`. Inconditionnelle — tout pilote de tout dépôt court dans le même bac
à sable, groomeur comme implémenteur. Le commentaire d'accompagnement nomme la
fenêtre et sa conséquence des deux côtés, à la forme des trois invariants déjà en
place.

### U3 — Le détecteur : `test_pilot_orientation_block.sh`

Quatre parties, à la forme de `test_pilot_phase2b_argv.sh` (mika#2077), qui est le
précédent exact d'un détecteur de ce fichier wirable par construction :

- **PART A — comportemental.** `_pilot_orientation_rule` est appelée hors `bwrap` et
  sa sortie est mesurée : elle nomme la racine passée, déclare le non-montage, et
  tient dans la borne de 12 lignes.
- **PART B — structurel, comptes exacts.** Sur le fichier tracké : l'injection
  existe, elle est **après** le branchement `ITERATION_CTX` et **avant**
  l'injection de `TICKET_CONTEXT`, et elle est inconditionnelle. L'ordre est
  l'invariant, donc il est asserté comme tel et non déduit d'une présence.
- **PART C — anti-vacuité.** B relancée sur une copie régressée — injection déplacée
  après `TICKET_CONTEXT` — qui doit **échouer**. Sans elle, des assertions dont rien
  ne prouve qu'elles savent dire non : la classe que mika#1772 a trouvée en câblant
  `test-dispatch-lib.sh`.
- **PART D — préconditions.** Ni `bwrap`, ni réseau, ni token. C'est ce qui rend la
  suite câblable sur un runner hébergé, et ce qui distingue ce vol du vol 2.

La copie régressée de PART C est bâtie sous `.pilot-scratch/` (mika#2548).

### U4 — Le câblage

`Makefile` (cible `test-pilot-orientation-block`) et `.github/workflows/ci.yml`, dans
le même bloc que `make test-pilot-phase2b-argv`. Le câblage est livré **dans le même
vol que le détecteur** : la leçon mika#2578 est qu'un garde non câblé passe la revue,
passe la CI par construction, et laisse partir le défaut.

Aucune entrée dans `scripts/suite-wiring.tsv` : la suite est câblée, et la lint
bidirectionnelle rougirait sur une entrée pour une suite que la CI atteint.

---

## Fire-Disposition

Ce vol livre un détecteur (U3 + son câblage U4). Disposition retenue :
**(a) exception nommée en allowlist — avec une population d'exceptions vide à
l'atterrissage.**

Le détail, parce que « vide » doit être mesuré et non supposé :

- **La population de violations à l'atterrissage est zéro, par construction de
  l'ordre de commit.** Le détecteur garde une propriété de l'injection U2, qui
  atterrit dans le même vol et avant lui. Il n'existe aucun site préexistant à
  exempter : l'injection est neuve, et il n'y en a qu'une.
- **Aucune allowlist n'est donc créée.** En créer une vide serait un ornement, et la
  garde de la Phase 1 énonce la règle inverse — « l'exemption est la surface
  d'attaque, donc elle est étroite et nommée ». Zéro exemption est l'état le plus
  étroit possible ; il est l'état d'atterrissage.
- **Ce qui remplace l'assertion auto-nettoyante,** dont l'objet est de rougir quand
  une exemption devient stale : il n'y a pas d'exemption à faire expirer, donc la
  charge se déplace sur la preuve que le détecteur sait **dire non**. C'est PART C,
  qui régresse volontairement l'ordre d'injection et exige l'échec. Un détecteur
  armé à zéro exception sans contrôle de vacuité est exactement un détecteur dont on
  ne sait pas s'il mesure quelque chose.

Le détecteur atterrit donc **armé et vert**, sans `#[ignore]`, sans suivi d'armement,
et sans halte opérateur.

---

## Acceptance criteria

**AC1 — Le pilote est situé, et la référence de ticket survit.** Le `PROMPT` composé
par `dispatch-lib.sh` commence toujours par exactement `<repo>#<n>` et porte un bloc
d'orientation qui (a) nomme la racine de travail réelle en toutes lettres, (b)
déclare le workspace hôte non monté, (c) énonce que les chemins hôte cités par le
ticket ne sont pas rejouables. Les trois sont vérifiés dans le même appel : vérifier
seulement qu'un bloc existe laisserait passer la régression de préfixe.

**AC2 — Le bloc précède le corps du ticket, et c'est asserté comme un ordre.** Le
test établit que l'injection du bloc se situe **après** le branchement `ITERATION_CTX`
et **avant** l'injection de `TICKET_CONTEXT` dans le fichier tracké. Une injection
présente mais placée après `CONTEXTE DU TICKET` fait **échouer** la suite.

**AC3 — Le bloc est borné à 12 lignes.** Mesuré sur la sortie de la fonction, pas sur
le source.

**AC4 — Le détecteur sait dire non.** Les assertions d'ordre de l'AC2, relancées sur
une copie régressée où l'injection est déplacée après `TICKET_CONTEXT`, échouent. Une
suite qui ne démontre pas sa propre capacité d'échec ne compte pas comme livrée.

**AC5 — Le détecteur est lancé par la CI.** `.github/workflows/ci.yml` nomme la
cible, directement ou par une cible `make` qu'il nomme. La suite ne requiert ni
`bwrap`, ni réseau, ni token, et ne figure pas dans `scripts/suite-wiring.tsv`.

**AC6 — Aucun bind n'est élargi, et aucun argument `bwrap` n'est touché.** Ce vol est
purement textuel. Vérification mécanique : le diff n'ajoute aucune ligne portant
`--bind`, `--ro-bind`, `--ro-bind-try` ou `--ro-bind-data`. Le contrôle négatif
complet reste la charge du vol 2.

**AC7 — La racine hôte est dérivée, jamais écrite en dur** dans le texte du bloc,
conformément à la consigne « étendre par propriété » de
`scripts/check-pilot-instruction-paths.sh`. Corollaire mesurable : la garde de la
Phase 1 reste **verte** après ce vol.

---

## Definition of Done

- `_pilot_orientation_rule()` et son injection sont sur la branche, à la fenêtre de P4.
- `skills/bundled/_shared/tests/test_pilot_orientation_block.sh` rend `0` en local,
  ses quatre parties passent, et PART C démontre l'échec sur copie régressée.
- `make test-pilot-orientation-block` existe et la CI l'appelle.
- `make check-pilot-instruction-paths`, `make check-suite-wiring`,
  `make test-dispatch-symmetry` et `make lint` restent verts.
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de PR nomme le vol 1, et la ligne `Tracked in:` du vol 2.

---

## Ordre de commit

1. **U1 + U2** — la fonction et son injection. Le dépôt est fonctionnel, rien ne le
   garde encore.
2. **U3** — le détecteur, vert dès son arrivée puisque U2 le précède.
3. **U4** — le câblage Makefile + CI. En dernier, parce qu'un câblage qui précède son
   détecteur casse la CI du commit intermédiaire.

Plancher du vol : U1 + U2 + U3 + U4. Retirer U4 livrerait un garde que le pipeline
ne lance pas (P5) ; retirer U3 livrerait une injection que rien ne maintient à sa
place, dans un fichier qui a doublé de taille en six semaines.

---

## Hors périmètre

- **Les phases 3 et 4 du plan antérieur** — `README-SANDBOX`, le contrôle négatif en
  `bwrap` réel, le câblage de `test_sandbox_git_usable.sh`. Vol 2, tracké ci-dessus.
- **Le montage du gitdir.** Clos par mika#2146, et le commentaire opérateur du
  2026-09-03 interdit de le rouvrir.
- **Élargir les binds.** Ce vol n'en touche aucun (AC6). L'invariant de mika#2146 —
  autres worktrees, autres sous-dépôts, `~/.ssh`, `~/.config/gh`, `~/.gitconfig`
  inaccessibles — ne se relâche pas et n'est pas sollicité.
- **Réécrire les chemins hôte dans le corps du ticket à l'injection.** Écarté, et
  pas par économie : pour #2108 les chemins hôte *sont* le sujet du ticket. Les
  réécrire falsifierait le texte que l'opérateur a écrit et détruirait le sens du
  ticket le jour où un pilote en a le plus besoin. L'inoculation obtient l'effet sans
  toucher au texte.
- **Les 27 fichiers historiques de `docs/`.** Même raison qu'en septembre : comptes
  rendus datés, non chargés automatiquement.
- **La létalité de la forme large** (cpp#128) et le garde-fou d'inactivité (cpp#147).

---

## Risques et limites, nommés

**C'est du prompt : ça réduit un taux, ça ne ferme pas une classe.** Même limite que
`DENIED_BASH_PATTERNS_HINT` porte dans sa propre note de clôture. Le vol 2 existe
précisément parce que celui-ci ne suffit pas : le marqueur dans le squelette répond
au geste qui a lieu quand même. Aucun des deux ne prétend fermer la classe seul — ils
couvrent le raisonnement et le geste.

**La position est un pari mesuré, pas une certitude.** L'argument de P4 est que la
récence sert une prescription et la précédence une correction de lecture. C'est un
raisonnement sur la fonction, pas une mesure de taux : il n'y a pas d'A/B ici. Ce qui
est mesuré et garanti, c'est que la position *voulue* est celle *obtenue* et qu'elle
ne dérivera pas (AC2 + AC4). Si une mesure ultérieure de débit montre que la queue
servirait mieux, l'ordre se déplace d'une ligne et le détecteur dit où.

**Concurrence d'attention.** Un sixième bloc dans un prompt qui en porte cinq. La
borne de 12 lignes est ce qui empêche la dilution, et elle est un critère
d'acceptation (AC3) pour cette raison.

**Le squelette dérivé de P3 n'est pas sondé.** `bwrap` est refusé depuis une session
de grooming. La liste de binds est lue sur le fichier et la liste hôte est mesurée,
mais le squelette résultant est déduit. Le vol 2 le sonde ; ce vol ne dépend d'aucune
de ses valeurs — le fait 2 du bloc d'orientation énonce le non-montage, qui est vrai
sous toute forme du squelette.

**Ce plan ne prétend pas que le pilote livrera.** Il retire une friction du chemin ;
il ne mesure pas ce qui se trouve après elle.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_pilot_orientation_rule()` (texte + justification) | 42 |
| `skills/bundled/_shared/dispatch-lib.sh` — injection U2 + commentaire d'invariant | 14 |
| `skills/bundled/_shared/tests/test_pilot_orientation_block.sh` (4 parties) | 95 |
| `Makefile` + `.github/workflows/ci.yml` (câblage U4) | 8 |

Total estimé : 159 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol tient sous la borne, et voici pourquoi il tient plutôt que de simplement
passer : les deux unités de code sont textuelles et locales à un seul site de
composition, sans schéma à migrer ni appelant à mettre à jour ; la suite de test est
la part dominante et suit un patron existant (`test_pilot_phase2b_argv.sh`) dont les
quatre parties sont déjà écrites pour ce fichier ; et aucune unité ne demande de
`bwrap`, donc aucun tour n'est dépensé en boucle de diagnostic sur un bac à sable.
Le périmètre qui aurait fait déborder — binds, squelette, contrôle négatif réel — est
sorti en vol 2 plutôt que comprimé ici.

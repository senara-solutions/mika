# Phase 1 — un lecteur unique du callout `Plan` (mika#2194)

**Ticket:** senara-solutions/mika#2194 — labels `ready`, `dispatch:loop`

> **Classe :** une décision écrite deux fois, dans deux langues, jamais comparée
> sur les mêmes entrées.
> **Précédent direct :** mika#2158 (`grooming_marker.rs`) — un lecteur, ses
> appelants n'en portent aucune copie, un scan de source refuse le second.

---

## Constat

Le ticket pose une direction (« décisions → Rust, glue → Python, méthode
strangler ») et un ordre (« commencer par les fonctions les plus mordues »). Il ne
peut pas être livré en une PR : `dispatch-lib.sh` porte **119 fonctions**. Ce plan
découpe le ticket en phases (§ suivant) et **ne groome que la phase 1**.

Trois mesures, relevées sur l'arbre au 2026-09-30, déplacent le ticket avant de
l'exécuter.

### M1 — La mesure du ticket est périmée, et dans le mauvais sens

| | ticket (2026-09-05) | mesuré (2026-09-30) |
|---|---|---|
| `dispatch-lib.sh` | ≈ 300 Ko | **551 330 o**, 9 757 lignes, **119 fonctions** |
| `test-dispatch-lib.sh` | « artisanal » | **514 874 o**, 9 704 lignes |
| `_shared/tests/` | non mentionné | **24 fichiers**, 452 Ko |

Le fichier a **presque doublé en 25 jours**. Le corpus de test l'a suivi : ~1 Mo au
total pour les deux fichiers. Ce n'est pas un détail de cadrage — c'est ce qui rend
« réécrire la glue en Python » un programme de plusieurs mois et non un ticket, et
ce qui rend le **premier maillon** urgent plutôt que soigné.

### M2 — « Tests artisanaux » est vrai du framework, faux de la couverture, et le mécanisme de test doré EXISTE DÉJÀ

Le harnais porte déjà ce que le ticket demanderait d'un harnais neuf :

- **`source "$DISPATCH_LIB"` puis appel direct de la fonction de production**
  (215 occurrences de `DISPATCH_LIB`). Le ticket demande « mêmes entrées → mêmes
  sorties » : le geste existe, il est même **doctrinal** (test-dispatch-lib.sh
  l. 318 : *« ils y appellent `_extract_plan_path`, la fonction de production, au
  lieu d'en \[recopier le motif\] »*).
- **Une troisième colonne `SKIPPED`** (mika#2149) : *« a guard that could not arm
  is never read as a bare green »*.
- **L'hermétisme git** (mika#1772) : cinq `GIT_CONFIG_*` qui neutralisent
  `commit.gpgsign` et posent `init.defaultBranch`.

**Conséquence pour ce plan : il n'y a pas de harnais à construire.** Ce qui manque
n'est pas le mécanisme, c'est **un corpus commun** — voir M3.

Une fragilité relevée en passant et **non corrigée ici** : le motif dominant est
`source "$DISPATCH_LIB" 2>/dev/null || true`. Un `dispatch-lib.sh` qui cesse de
sourcer laisse les tests verts sur des fonctions inexistantes : le harnais ne peut
pas voir sa propre inertie. Pour un test de **parité** c'est éliminatoire, d'où le
terme d'anti-vacuité de R4 ci-dessous.

### M3 — Les deux corpus existent et sont DISJOINTS. La parité n'a jamais été mesurée.

C'est le défaut central, et il n'est ni dans le corps du ticket ni dans le TSV de
mika#2201.

| | lecteur | corpus | où |
|---|---|---|---|
| Rust | `auto_pull::extract_plan_path` (privée) | **6 corps d'issue réels**, gelés, tous en forme préfixée | `crates/mika-agent/tests/fixtures/plan_callout_bodies/` |
| Bash | `_extract_plan_path` | **17 assertions** à fixtures inline | `test-dispatch-lib.sh` l. 6623-6708 |

Les deux jeux sont soignés — le README du corpus Rust documente la provenance
ligne par ligne (« mesurée » / « relevée » / « remplissage annoncé »), et le bloc
bash porte ses contrôles négatifs et son propre « guards the guard ». **Aucune
entrée n'est commune aux deux.** Personne n'a jamais exécuté les deux lecteurs sur
la même entrée et comparé.

Et la divergence est réelle :

```
Rust  (auto_pull.rs:585)   (?m)^> - \*\*Plan:\*\* `((?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+)`
                           + strip_fenced_blocks(body)  AVANT le match
                           + rend le chemin BRUT

Bash  (dispatch-lib:8854)  ^> - \*\*Plan:\*\* `\K(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)?docs/plans/[^`]+
                           + AUCUN retrait de bloc clôturé
                           + NORMALISE : ${path#*/}
```

**Deux écarts, et les DEUX sont documentés côté bash** — c'est la correction que
la lecture du code impose à la première rédaction de ce plan :

1. **Les blocs clôturés.** Le Rust les retire (`strip_fenced_blocks`, testé :
   `mika2120_strip_fenced_blocks_frontiere`). Le bash n'a **aucun** équivalent, et
   son commentaire de production le dit en toutes lettres (l. 8847-8850) : *« Il ne
   distingue pas le callout de sa citation dans un bloc de code — cette moitié-là
   n'existe que côté Rust, où elle garde une promotion ; ici un faux positif est
   déjà rattrapé par le test `-f` qui suit. »* Ce n'est donc **pas** une
   méconnaissance : c'est une asymétrie **assumée avec sa raison**.
   **Mais la raison n'est vraie qu'à moitié**, et c'est ce qui justifie qu'un suivi
   existe : le `-f` rattrape un chemin *inexistant*, pas un chemin *existant cité
   dans un bloc*. Un ticket qui cite le callout d'un plan réellement présent dans
   le worktree passe le `-f` et pose `ENTRY_COMMAND="/ce-work <mauvais-plan>"`.
   Population non mesurée ; ce plan ne la ferme pas.
2. **La normalisation.** Le Rust rend `mika/docs/plans/x.md`, le bash rend
   `docs/plans/x.md`, parce que le bash résout ensuite `"$WORKTREE_DIR/$PLAN_PATH"`
   : garder le préfixe désignerait `…/mika/mika/docs/`. Les deux consommateurs ne
   posent pas la même question — `auto_pull` demande *à qui appartient ce plan*,
   dispatch-lib demande *quel fichier ouvrir*. Également documenté sur place
   (l. 8840-8845).

⇒ **Un lecteur unique doit donc rendre les DEUX formes et offrir les DEUX
politiques de fence.** Un lecteur qui n'en rendrait qu'une casse un des deux
appelants, et la « fusion » naïve est ce que le dépôt a déjà refusé par écrit —
voir la borne B1.

---

## Phasage du ticket, et ce que cette PR groome

Le ticket est un programme, pas une PR. Le découpage suit le **critère de sélection
B2** (ci-dessous), pas le volume : on migre ce dont le Rust est déjà le lecteur, et
on laisse au shell ce que seul le shell connaît.

| phase | périmètre | état |
|---|---|---|
| **1** | `_extract_plan_path` → Rust, de bout en bout, **plus la doctrine** que les suivantes réutilisent | **ce plan, groomé** |
| 2 | Les deux autres lecteurs du même jeton : `_committed_plan_on_branch`, `_set_up_worktree` | ticket à ouvrir — précondition : phase 1 mergée |
| 3 | `_parse_disposition` / `_parse_verdict` — les plus mordus, écartés ici sur mesure | ticket à ouvrir — précondition : décider ce que devient le canal de retour par fichier (`$_DISPOSITION_FUZZY_FILE`) |
| 4 | La glue d'orchestration (bwrap / git / gh / trap / worktree / callback) → Python | ticket à ouvrir — précondition : un maillon dont la nature est de la glue et non une décision |

**Seule la phase 1 est groomée.** Les phases 2-4 sont nommées pour que la doctrine
(R7) les prépare, et **aucune n'est ouverte par cette PR** — les ouvrir avant que
la phase 1 ait établi sa doctrine serait instruire sans mesure.

**Ordre de grandeur, pour que la phase 4 ne soit pas sous-estimée :** les fonctions
dont le nom et le corps sont des prédicats purs totalisent de l'ordre de **1 600
lignes sur 9 757**. Les ~84 % restants sont de la glue, qui est la direction du
ticket et n'est ni la phase 1 ni la phase 2.

---

## Deux bornes écrites que ce plan respecte plutôt que de les découvrir

### B1 — L'harmonisation naïve des lecteurs est DÉJÀ refusée, en toutes lettres

`auto_pull.rs`, doc-comment l. 501-506 :

> Il n'est pas non plus le prédicat le plus étroit du dépôt, et cela reste vrai
> après mika#2120 : `executor::check_grooming_markers` se contente de la
> sous-chaîne `docs/plans/`, non ancrée. **Ne le resserrez pas pour « harmoniser »
> les deux** — ce sens-là de l'alignement recréerait le défaut symétrique de celui
> que ce ticket ferme.

Il y a donc **trois** tolérances mesurées sur ce jeton, et elles sont
délibérément inégales : `auto_pull` strict + fences, `dispatch-lib` strict sans
fences, `executor` sous-chaîne non ancrée. Un lecteur unique qui écraserait les
trois vers la plus stricte fermerait un défaut en ouvrant son miroir. **Ce plan ne
touche pas `executor::check_grooming_markers`** et ne resserre rien : il unifie
l'**implémentation** de deux lecteurs, pas leur **tolérance**.

### B2 — mika-platform#58 a déjà refusé une migration de cette classe, et sa raison est le critère de sélection

`CLAUDE.md`, mika#2249 :

> `dispatch-lib.sh` is the only place that knows the worktree path (it calls
> `scripts/derive-worktree-path`), **re-deriving it in Rust is the duplication
> mika-platform#58 closed**.

Le remède retenu là-bas : le shell **dit** au Rust (stamp de metadata), le Rust ne
dérive pas. ⇒ **Critère de sélection d'un maillon**, écrit dans la doctrine :

> Ne migre que ce dont le Rust est déjà, ou doit être, le lecteur. Ce que **seul**
> le shell connaît reste au shell et voyage par stamp.

Le callout `Plan` satisfait ce critère : le Rust le lit déjà, dans `auto_pull`,
sur le chemin du feeder. Le chemin du worktree ne le satisfait pas — et c'est
pourquoi il n'est pas ce maillon.

---

## Pourquoi ce maillon, et pourquoi Rust plutôt qu'une garde de parité seule

**Le maillon.** `_extract_plan_path` est le seul candidat qui coche les cinq
critères à la fois :

| critère | `_extract_plan_path` |
|---|---|
| pur (entrées → sortie, zéro effet de bord) | **oui**, 12 lignes (8851-8862) |
| un seul appelant | **oui** — `_detect_plan_on_branch` (l. 8894) |
| mordu, avec incident mesuré | **oui** — mika#2120, 15 h de boucle morte |
| déjà lu en Rust ⇒ la migration RETIRE une duplication | **oui** — `auto_pull` |
| corpus doré déjà écrit des deux côtés | **oui** — 6 + 17 cas |

Les autres candidats évidents échouent sur un critère : `_parse_disposition`
(90 lignes) rend **deux** valeurs via un fichier (`$_DISPOSITION_FUZZY_FILE`) —
l'effet de bord fait partie de son contrat ; `_measure_cycle_output` et
`_gate_non_empty_cycle` (111 et 131 lignes) lisent des fichiers de session ;
`_is_dispatchable_repo` (190 lignes) porte une allowlist et des `gh`.

**Pourquoi pas une garde de parité seule.** Elle est tentante : zéro coût runtime,
zéro dépendance. Elle est **insuffisante**, et la raison est dans le ticket.
Les trois classes de panne citées — portée des guillemets (cpp#157), `>` littéral
pris pour redirection, jetons matchés en sous-chaîne (#2188) — sont des pièges de
parsing **dans le code du prédicat**. Une garde de parité les attrape seulement si
le corpus contient l'entrée qui les déclenche ; or les trois sont arrivées *parce
que* personne n'avait pensé à cette entrée. Un `Regex` Rust sur un `&str` n'a ni
portée de guillemets, ni redirection, ni sous-chaîne accidentelle, et le
compilateur refuse une regex invalide. **L'argument central du ticket est juste,
et c'est la moitié structurelle.**

Donc **les deux**, pas l'un ou l'autre : le bash délègue (R2/R3), et la garde de
parité (R4) tient la fenêtre pendant laquelle il reste un lecteur bash quelque
part et vérifie qu'on n'a rien changé en déléguant.

**Argument non anticipé par le ticket :** `_extract_plan_path` utilise `grep -oP`
(PCRE, `\K`). `grep -P` n'est pas garanti (busybox, macOS). La délégation retire
cette dépendance de portabilité sur le chemin critique du dispatch.

---

## Requirements

### R1 — Un lecteur unique, en Rust, qui rend les deux formes

`crates/mika-agent/src/plan_callout.rs`, doctrine `grooming_marker.rs`.

```rust
pub struct PlanCallout {
    /// Le chemin tel qu'écrit dans le callout, préfixe de dépôt compris.
    /// Consommateur : `auto_pull::plan_ownership` — la question est l'appartenance.
    pub raw: String,
    /// Le même, premier segment retiré s'il y en avait un.
    /// Consommateur : `dispatch-lib` — la question est quel fichier ouvrir.
    pub normalized: String,
}

/// `fences` décide si les blocs clôturés sont retirés avant le match.
pub fn plan_callout(body: &str, fences: FenceHandling) -> Option<PlanCallout>;
```

`FenceHandling` est un `enum` à deux variantes, **pas un `bool`** : un booléen au
site d'appel ne dit pas lequel des deux sens il porte, et les deux appelants
choisissent différemment. Les deux variantes portent chacune leur doc-comment
nommant son appelant et la raison — **y compris `Keep`, qui n'est pas un
échafaudage de migration mais le comportement documenté et justifié du bash**
(M3, écart 1).

**Côté Rust, la délégation est interne au module.** `auto_pull::extract_plan_path`
est **privée** et a deux appelants dans son propre fichier (`is_groomed` l. 511 et
`plan_ownership`). Elle devient un mince adaptateur qui appelle
`plan_callout(body, FenceHandling::Strip)` et rend `raw` — **et ne garde aucune
copie de la regex**. C'est la doctrine mika#2158 appliquée : les appelants
appellent, ils ne recopient pas. Aucune signature publique ne bouge.

**Ce que R1 ne fait pas :** il ne touche pas `executor::check_grooming_markers`
(borne B1) et ne change **aucune** tolérance.

### R2 — La sous-commande, et le corps passe par un FICHIER

`mika plan-callout --body-file <path> [--raw]`.

**Jamais un argument.** Un corps d'issue porte des retours à la ligne, des
backticks et des `$` : le passer en argv ré-introduirait la classe de panne de
portée de guillemets **dans le geste même qui prétend la fermer**. Le refus est
structurel — il n'y a pas de variante positionnelle à la sous-commande.

**Trois codes de sortie, et le troisième est le livrable :**

| code | stdout | sens |
|---|---|---|
| `0` | le chemin, une ligne | un callout a été lu |
| `1` | vide | **aucun callout** — la réponse est « non » |
| `≥2` | vide, diagnostic sur stderr | **je n'ai pas pu regarder** |

Aujourd'hui `_extract_plan_path` rend `1` dans les deux derniers cas, et
`_detect_plan_on_branch` fait `|| return 0` — donc une erreur de lecture est lue
comme « pas de plan ». La population « fichier illisible » est **créée par la
migration** (aujourd'hui le corps est en variable, il n'y a pas de fichier), et ne
pas la distinguer fabriquerait un silence qui n'existait pas. Ce n'est pas un
changement de comportement : c'est le traitement d'un état nouveau, et il est dit
plutôt que découvert.

**Chemin court, avant toute résolution.** La sous-commande sort dans le
`match &cli.command` de tête de `main.rs`, à côté de `Token` et
`CredentialHelper` (*« lightweight commands: early-exit before agent resolution,
logging, and telemetry »*). Elle fait **strictement moins** que `Token` : ni
`dotenv`, ni `Settings`, ni `home`, ni DB, ni réseau. Un prédicat pur n'a aucune
raison de résoudre un agent.

**Le coût, mesuré au bon endroit.** `_extract_plan_path` est appelé **une fois par
dispatch**. Un démarrage de process sur un dispatch qui dure des minutes à des
heures est négligeable — et c'est un **critère de sélection** que la doctrine (R7)
doit porter : la fréquence d'appel décide du canal. Un prédicat appelé en boucle
serait un autre arbitrage, et ce plan ne le préjuge pas.

**Coût de nommage, assumé.** `plan-callout` au premier niveau nomme la chose lue,
pas une famille — une famille avec un seul membre serait une abstraction avant son
usage. Le coût est un namespace top-level qui se peuple. Condition de
regroupement, écrite : **à la phase 2**, avec deux usages en main, et d'autant plus
tôt qu'un regroupement est une rupture de contrat pour dispatch-lib — bornée ici
par le fait qu'**un seul** site bash appelle.

### R3 — La bascule, et son fail-safe de déploiement

`_extract_plan_path` écrit le corps dans un `mktemp`, appelle
`mika plan-callout --body-file "$tmp"`, et rend ce que la sous-commande rend. Le
`grep -oP` et le `case` de normalisation disparaissent. `_detect_plan_on_branch`
traite les trois codes : `0` → pose `PLAN_PATH`, `1` → `return 0` (comportement
d'aujourd'hui), `≥2` → **refus bruyant** avec le motif, jamais un `PLAN_PATH` vide.

**Le bash demande `FenceHandling::Keep`**, ce qui est la parité exacte avec son
comportement d'aujourd'hui. Demander `Strip` serait une correction de comportement,
que les bornes du ticket interdisent pendant la migration.

**Asymétrie de déploiement, et elle penche du bon côté.** `dispatch-lib.sh` est une
projection du **binaire** (mika#2340) : le seed est écrit *par* le binaire
installé, donc les deux moitiés voyagent ensemble. Les deux dérives possibles :

| état | effet |
|---|---|
| dispatch-lib **ancien** + `mika` **neuf** | l'ancien n'appelle pas la sous-commande, il garde son `grep`. **Fail-safe par construction.** |
| dispatch-lib **neuf** + `mika` **ancien** | `mika` sort non-zéro sur une sous-commande inconnue ⇒ code `≥2` ⇒ refus bruyant. **Jamais un chemin vide.** |

Le second état n'est pas produit par `make deploy` (le binaire qui seede est celui
qui est installé) mais il est produit par une copie à la main, et il faut qu'il
soit lisible plutôt que silencieux.

La dépendance à `mika` n'est **pas nouvelle** : `dispatch_claude_pilot` fait déjà
`command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required …"; exit 1; }`.
**Donc pas de repli bash** — un repli serait une seconde implémentation,
c'est-à-dire précisément ce qu'on retire. C'est une décision, et elle s'appuie sur
une précondition déjà en vigueur.

### R4 — Un corpus, deux lecteurs, une parité

C'est le livrable que M3 rend nécessaire, et il est **petit** : les corps existent,
il manque les attendus et la comparaison.

- `crates/mika-agent/tests/fixtures/plan_callout_bodies/` est **étendu** (jamais
  refait : son README interdit le rafraîchissement, et pour une raison mesurée —
  quatre des six callouts ont été recorrigés à la main au 2026-09-01, donc un jeu
  refetché passerait avant comme après le correctif). Les six corps gardent leur
  nom. S'ajoutent les formes que seul le bash exerçait (nue, autre dépôt, les
  quatre contrôles négatifs, mention inline, double callout) **et le cas que ni
  l'un ni l'autre n'exerce : un callout dans un bloc clôturé.**
- `expectations.tsv` à côté : `fichier · rc · raw · normalized · parity`. Un TSV
  plutôt qu'un fichier par attendu parce que la clé est un **nom de fichier** — il
  n'y a aucun retour à la ligne à échapper.
- Deux lecteurs du même corpus : un test Rust (`tests/plan_callout_parity.rs`) et
  un bloc de `test-dispatch-lib.sh`. Chacun exige les attendus **et** l'égalité
  avec l'autre côté, sauf sur les lignes marquées `divergent`.
- **Anti-vacuité, obligatoire** (M2, et le motif « guards the guard » de
  l. 6703) : le bloc bash **échoue** si le corpus est introuvable ou vide, et
  refuse de tourner si `_extract_plan_path` n'est pas définie après le sourcing.
  Sans ce terme, un `source … || true` qui cesse de sourcer rend une parité verte
  sur zéro cas.
- Le README est étendu : le nouvel axe (parité bash↔Rust) est nommé, et les cas
  ajoutés portent leur provenance comme les six historiques — **mesuré** / **relevé**
  / **construit pour ce test**.

### R5 — Le scan anti-copie, étendu

Le scan existant (l. 6707) couvre **une** fonction :
`_detect_plan_on_branch ne porte plus de motif de callout`. Il est étendu à **tout
`dispatch-lib.sh`** : aucun motif du callout `Plan` hors du site de délégation.
Quand il tire, la résolution est de **router le site vers la délégation**, jamais
d'ajouter une ligne d'allowlist (doctrine mika#2201 § D5/D6).

Attention au piège déjà mesuré deux fois dans ce dépôt (mika#2050, mika#2201
§ R4) : ce fichier **cite** le motif dans ses commentaires de doctrine. Le
prédicat porte donc sur les lignes de **commande**, commentaires retirés d'abord.

### R6 — Le TSV mis à jour, et les deux gardes de mika#2201 forcent la cohérence

Quatre lignes portent le jeton `> - **Plan:**` (`scripts/canonical-tokens.tsv`
l. 145-148) — une Rust, trois bash — et leur destin :

- `auto_pull.rs::PLAN_CALLOUT_RE` → la ligne nomme désormais le site de
  `plan_callout.rs` ; c'est lui qui porte le motif.
- `dispatch-lib.sh::_extract_plan_path` → la ligne nomme le site Rust ; c'est lui
  qui lit.
- `_committed_plan_on_branch` et `_set_up_worktree` → **inchangées**, et c'est
  une décision : elles lisent le callout pour d'autres questions, sur d'autres
  chemins, et les migrer élargirait ce maillon au-delà de son unique appelant.
  Elles sont la **phase 2**.

Les deux gardes bidirectionnelles de mika#2201
(`canonical-tokens-survey.sh --check` part de la forme de lecture,
`mika2201_every_match_site_is_declared` part du jeton,
`mika2201_every_declared_symbol_still_exists` tient le sens périmé) rendent cette
mise à jour **obligatoire plutôt que facultative** : un site qui disparaît sans
que sa ligne bouge fait rougir le build. **Aucun registre nouveau n'est créé** —
celui-là existe et fait le travail.

### R7 — La doctrine, bornée à ce que la phase 1 a établi

`docs/architecture/dispatch-lib-migration.md`. Prose, et rien que ce qui est
mesuré ici :

1. **Les trois critères de sélection d'un maillon** : pureté, un appelant, et le
   critère B2 (*le Rust doit déjà être, ou devoir être, un lecteur ; ce que seul
   le shell connaît reste au shell et voyage par stamp*).
2. **Le critère de canal** : la fréquence d'appel décide. Un appel par dispatch ⇒
   sous-commande. Un appel en boucle ⇒ arbitrage ouvert, non préjugé.
3. **Les deux bornes B1 et B2**, avec leur citation, pour que la phase 2 ne les
   redécouvre pas.
4. **La forme du corpus doré** : un corpus, deux lecteurs, un terme d'anti-vacuité,
   et la distinction `pre-switch` / `post-switch` de la Fire-Disposition.
5. **Le tableau de phasage** ci-dessus, avec la précondition de chaque phase.
6. **Ce qui ne migre pas**, et c'est la majorité : la glue bwrap / git / gh / trap /
   worktree / callback, ~84 % du volume (ordre de grandeur en tête de plan).
   « Glue → Python » reste la direction du ticket ; c'est la phase 4, et le dire
   évite qu'un futur maillon s'y croie autorisé par ce précédent.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/plan_callout.rs` | **neuf** — R1 |
| `crates/mika-agent/src/lib.rs` | déclaration du module |
| `crates/mika-agent/src/auto_pull.rs` | délégation, regex retirée |
| `crates/mika-cli/src/cli.rs` | `Commands::PlanCallout` |
| `crates/mika-cli/src/commands/plan_callout.rs` | **neuf** — R2 |
| `crates/mika-cli/src/commands/mod.rs` | déclaration |
| `crates/mika-cli/src/main.rs` | early-exit, à côté de `Token` |
| `skills/bundled/_shared/dispatch-lib.sh` | R3 — deux fonctions |
| `skills/bundled/_shared/test-dispatch-lib.sh` | R4 lecteur bash, R5 scan |
| `crates/mika-agent/tests/fixtures/plan_callout_bodies/` | cas ajoutés + `expectations.tsv` + README |
| `crates/mika-agent/tests/plan_callout_parity.rs` | **neuf** — R4 lecteur Rust |
| `scripts/canonical-tokens.tsv` | R6 |
| `docs/architecture/dispatch-lib-migration.md` | **neuf** — R7 |
| `crates/mika-cli/CLAUDE.md` | la sous-commande, son chemin court, ses trois codes |

---

## Verification Contract

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | le lecteur unique est correct | `cargo test -p mika-agent plan_callout` | vert |
| V2 | **la parité, côté Rust** | `cargo test -p mika-agent --test plan_callout_parity` | vert, `n > 0` cas |
| V3 | **la parité, côté bash** | `make test-dispatch-lib` | vert, et le bloc de parité **nomme son compte de cas** |
| V4 | `auto_pull` ne régresse pas | `cargo test -p mika-agent auto_pull` | vert, inchangé — dont `mika2120_is_groomed_sur_les_six_corps_prefixes` |
| V5 | le TSV est cohérent dans les deux sens | `make check-canonical-tokens` + `cargo test -p mika-agent canonical_tokens` | vert |
| V6 | le scan anti-copie voit quelque chose | R5 rend son compte de sites scannés | `> 0` |
| V7 | la sous-commande n'ouvre ni DB ni réseau | test d'intégration CLI sur un `MIKA_HOME` inexistant | `0`/`1` selon le corps, jamais une erreur de résolution |
| V8 | les trois codes de sortie | test d'intégration CLI : corps avec callout, sans, fichier absent | `0`, `1`, `≥2` |
| V9 | lint et format | `make lint && make fmt && make check` | vert |
| V10 | la suite bundled ne casse pas | `make verify-bundled-skills` | vert |

**V2 et V3 sont le cœur.** Une parité verte sur zéro cas est le mode de panne que
R4 nomme, d'où l'exigence de compte dans les deux.

**Ce qui n'est PAS vérifiable depuis le bac à sable de dispatch**, dit plutôt que
laissé supposer :

- `mika-platform/scripts/` n'est pas matérialisé dans le worktree (seul `mika/`
  l'est), donc `derive-branch-name` / `derive-worktree-path` sont hors d'atteinte.
  B2 est établi par citation du `CLAUDE.md`, pas par lecture de ces scripts.
- Le coût réel d'un démarrage de `mika` n'est pas mesurable ici (`mika --version`
  est refusé par la permission-policy). Le raisonnement de R2 ne repose pas sur un
  chiffre : il repose sur « une fois par dispatch », qui est une lecture de code.
  Le chiffre est une sonde post-déploiement (S3).

---

## Fire-Disposition

Ce plan livre **deux détecteurs** : R4 (la parité) et R5 (le scan anti-copie).

**R5 — rien à disposer.** Le seul site est celui que R3 bascule ; le scan est vert
à la naissance par construction. Aucune exception.

**R4 — disposition (a), exception nommée en allowlist.** La parité est **rouge à
la naissance** sur un cas exactement, et il est connu, mesuré et **documenté en
production** : le callout dans un bloc clôturé (M3, écart 1). Le bash le lit, le
Rust ne le lit pas.

Le corriger serait un **changement de comportement**, que les bornes du ticket
interdisent explicitement pendant la migration (« Aucun changement de comportement
pendant la migration — les tests dorés en sont la preuve »). Et le corriger
*pendant* la bascule rendrait indécidable lequel des deux changements un test
vert atteste.

Forme de l'exception, dans `expectations.tsv` :

```
# EXCEPTION mika#2194 — divergence fences, mesurée, DOCUMENTÉE, non corrigée ici.
# Le bash lit un callout cité dans un bloc clôturé, le Rust non
# (auto_pull.rs::strip_fenced_blocks, mika#2120). L'asymétrie est ASSUMÉE côté
# bash (dispatch-lib.sh l. 8847-8850) : « un faux positif est déjà rattrapé par
# le test `-f` qui suit ». Ce rattrapage est PARTIEL — il couvre un chemin
# inexistant, pas un chemin existant cité dans un bloc.
# Suivi : <numéro déposé à l'ouverture de la PR> — décider si le rattrapage
#   partiel suffit. Ce n'est PAS « corriger la divergence » par défaut.
# La colonne `parity` vaut `divergent` : le test EXIGE la divergence.
fences-quoted-callout.md	0	docs/plans/x-plan.md	docs/plans/x-plan.md	divergent
```

**Assertion auto-nettoyante** : le test de parité **échoue** si un cas marqué
`divergent` rend la **même** valeur des deux côtés. Le jour où la divergence est
tranchée, la ligne rougit et doit être retirée — elle ne peut pas devenir stale en
silence. C'est la propriété qui distingue une exception d'un contournement.

**Et un point qui décide de la lecture de R4 :** après la bascule de R3, le bash
appelle le Rust avec `FenceHandling::Keep`, donc les deux côtés rendent **la
même** valeur, fences comprises. La ligne `divergent` rougirait immédiatement. Elle
porte donc `phase = pre-switch` et le corpus est exercé **deux fois** : contre le
bash **d'avant** la bascule (la preuve de parité que le ticket exige,
`git show HEAD~1:…` sous `.pilot-scratch/`) et contre le bash **d'après**. La
première passe est la preuve, la seconde est la non-régression. Sans cette
distinction, « parité prouvée par fonction » serait une tautologie : un lecteur
comparé à lui-même est toujours d'accord avec lui-même.

---

## Acceptance criteria

Le ticket n'a pas de section `## Acceptance criteria` ; celles-ci sont dérivées des
Requirements et du Verification Contract, et **bornées à la phase 1**.

- **AC1** — `crates/mika-agent/src/plan_callout.rs` est le **seul** site du dépôt
  qui porte le motif du callout `Plan` sous la forme stricte ; `auto_pull` et
  `dispatch-lib` l'appellent et n'en portent aucune copie. Tenu par R5 et par les
  deux gardes de mika#2201 (V5, V6).
- **AC2** — Le lecteur rend **les deux** formes (`raw`, `normalized`) et **les deux**
  politiques de fence ; `auto_pull` comme `dispatch-lib` reçoivent chacun ce que son
  consommateur attend. Aucune tolérance n'est modifiée :
  `executor::check_grooming_markers` n'est pas touché (borne B1).
- **AC3** — `mika plan-callout --body-file <f>` existe, lit le corps depuis un
  **fichier** (aucune variante positionnelle), sort dans le chemin court de
  `main.rs`, et n'ouvre ni base ni réseau (V7).
- **AC4** — Les trois codes de sortie sont distingués, et `_detect_plan_on_branch`
  traite `≥2` par un refus **nommé** plutôt que par un `PLAN_PATH` vide (V8).
- **AC5** — Un corpus **unique** est lu par les deux lecteurs, et la parité est
  asserted **contre le bash d'avant la bascule**. Les deux tests **nomment leur
  compte de cas** et échouent sur un corpus vide ou introuvable (V2, V3).
- **AC6** — La divergence fences est **nommée** dans le corpus, avec son ticket de
  suivi et une assertion auto-nettoyante qui rougit le jour où elle est tranchée
  (§ Fire-Disposition).
- **AC7** — `scripts/canonical-tokens.tsv` est cohérent dans les deux sens après
  la bascule (V5), et les deux lignes bash **non** migrées le restent
  explicitement (R6).
- **AC8** — `docs/architecture/dispatch-lib-migration.md` porte les trois critères
  de sélection, le critère de canal, les deux bornes B1/B2 avec leur citation, **le
  tableau de phasage avec la précondition de chaque phase**, et ce qui ne migre pas
  avec son ordre de grandeur.

---

## Definition of Done

- [ ] R1–R7 livrés.
- [ ] V1–V10 verts, V2/V3 avec un compte de cas non nul.
- [ ] AC1–AC8 satisfaites.
- [ ] La divergence fences porte un **numéro de suivi réellement déposé** (pas un
      placeholder) au moment d'ouvrir la PR.
- [ ] `crates/mika-cli/CLAUDE.md` documente la sous-commande, son chemin court et
      ses trois codes.
- [ ] Le corps de PR nomme les trois mesures M1–M3, **dit que cette PR livre la
      phase 1 sur quatre**, et dit explicitement que le ticket n'est **pas** clos.

---

## Surfaces opérateur

**Aucun événement de journal neuf, aucune ligne `audit_events`, et c'est une
décision.** Ce travail ne change aucun comportement observable : il déplace
l'implémentation d'un prédicat. Un compteur ici mesurerait « combien de fois
`_extract_plan_path` a été appelée », que personne ne demande.

La seule surface neuve est le **refus** du code `≥2`, et il vit là où l'opérateur
le cherchera déjà — le `RESULT` du callback, préfixé, comme les refus de
`cwd-guard.sh` (mika#2536) et de `pr-push-guard.sh` (mika#2520) :

```bash
mika tasks get <task-id>   # le motif est dans `result`
```
```sql
SELECT id, result FROM tasks
 WHERE result LIKE 'REFUSED (plan-callout, mika#2194)%' ORDER BY created_at DESC;
```

| motif | régime attendu | lecture |
|---|---|---|
| `body_file_unwritable` | **vide** | `mktemp` a échoué — le disque, pas le prédicat |
| `subcommand_unavailable` | **vide** | dispatch-lib neuf + `mika` ancien : c'est le **déploiement** qu'il faut établir (classe mika#2340), pas le prédicat |
| `subcommand_error` | **vide** | la sous-commande a planté ; le diagnostic est sur son stderr |

Le handler est un sous-processus shell sans accès base, et son stderr d'avant-pilote
est structurellement perdu sur un dispatch qui **réussit** (classe mika#2050 : il
hérite du `Stdio::piped()` de l'exécuteur, que celui-ci ne lit que dans la branche
`if !status.success()`). Inventer une surface de journal qui ne serait pas lue
reproduirait le défaut du Signal M.

---

## Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir — **sans cette vérification, chacune des
> sondes ci-dessous décrit le binaire d'hier.**

**S1 — le chemin nominal tient (premier dispatch sur un ticket groomé).**
`_detect_plan_on_branch` doit poser `PLAN_PATH` et l'`ENTRY_COMMAND` doit être
`/ce-work <chemin>` et non `/mika`.
*Halte 1 — `PLAN_PATH` est vide et aucun refus n'apparaît dans `result` :* ne pas
retoucher le prédicat. Établir d'abord le déploiement (préalable ci-dessus), puis
lire le `result` — un `subcommand_unavailable` dit que les deux moitiés ne sont pas
en phase, et c'est le remède.

**S2 — le feeder ne régresse pas (48 h).** `auto_pull` continue de promouvoir. La
sonde directe est celle de mika#2131, qui existe :
```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'auto_pull_exclusion' GROUP BY 1 ORDER BY 2 DESC;
```
*Halte 2 — `not_groomed` monte alors que des tickets portent leur callout :* le
lecteur Rust s'est resserré malgré B1. **Revert d'abord, diagnostic ensuite** — une
promotion perdue est la panne qui a coûté 15 h de boucle à mika#2120, et elle est
silencieuse.

**S3 — le coût est ce qu'on a supposé (première semaine).** Le chiffre que le bac
à sable n'a pas pu produire : chronométrer `mika plan-callout` sur l'hôte.
*Halte 3 — au-delà de ~500 ms :* le chemin court ne l'est pas, ou la sous-commande
résout quelque chose qu'elle ne devrait pas. **Réparer le placement dans
`main.rs`, pas mettre en cache** — un cache sur un prédicat pur est une seconde
source de vérité, c'est-à-dire la duplication qu'on vient de retirer.

**S4 — contrôle négatif du refus (7 jours).** Aucun `REFUSED (plan-callout…)` sur
un dispatch sain.
*Halte 4 — une occurrence :* c'est un faux positif et il coûte un dispatch entier.
Lire le motif ; un `body_file_unwritable` est un problème d'hôte, un
`subcommand_error` est un défaut du lecteur.

**Halte transverse — les sondes muettes.** Zéro refus **et** zéro dispatch ne
prouve rien : il faut qu'un dispatch sur un ticket groomé ait eu lieu depuis le
déploiement. *Une garde que personne n'a exercée se lit exactement comme une garde
qui marche* (mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne clôt pas mika#2194.** C'est **la phase 1 sur quatre**, un maillon sur
  119 fonctions. Le corps de PR doit le dire, et la doctrine nomme les suivantes.
- **Il ne retire pas une ligne de glue.** `dispatch-lib.sh` perd ~12 lignes et
  gagne ~15 (le `mktemp`, les trois codes) : **le fichier ne rétrécit
  pratiquement pas.** Ce qui change n'est pas sa taille, c'est qu'une décision de
  moins y est prise. Attendre une réduction de volume de ce maillon serait se
  tromper sur ce qu'il fait.
- **Il ne tranche pas la divergence fences.** Elle est nommée, testée comme
  divergente, et rattachée à un suivi dont l'objet est de **décider**, pas de
  corriger par défaut. C'est la Fire-Disposition, pas un oubli.
- **Il n'unifie pas les trois tolérances.** B1 l'interdit par écrit.
  `executor::check_grooming_markers` reste sur sa sous-chaîne non ancrée.
- **Il ne ferme pas les trois classes de panne de parsing pour dispatch-lib.** Il
  les ferme **pour ce prédicat**. Les 118 autres fonctions y restent exposées, et
  c'est l'ordre de grandeur que la doctrine écrit pour qu'il ne soit pas oublié.
- **Il n'ajoute aucun compteur.** Le seul instrument neuf est le motif de refus, et
  **son silence ne prouve rien tant que personne n'exécute S1 et S4** — sur un
  prédicat appelé une fois par dispatch, l'absence de refus peut simplement
  vouloir dire que rien n'a mal tourné, ou que rien n'a tourné.

---

## Hors périmètre de la phase 1, délibérément

- **Les phases 2, 3 et 4** du tableau de phasage. Chacune est un ticket à ouvrir
  avec sa précondition écrite ; aucune n'est ouverte par cette PR.
- **La moitié Python du ticket (phase 4).** Aucune ligne ici. Le précédent existe
  et est complet (`tools/mika_permission_policy/` : `pyproject.toml`, `uv.lock`,
  pytest, ruff, mypy, cible `make test-permission-policy-plugin`, testé en CI),
  **mais il est consommé par claude-pilot via un protocole de plugin, pas par
  dispatch-lib** — le canal shell→Python reste donc à construire, et il n'a pas de
  place dans un maillon dont le consommateur est déjà le moteur Rust.
- **`_parse_disposition` / `_parse_verdict` (phase 3).** Les plus mordus du lot
  (mika#1421, #2037, #2338) et les plus tentants. Écartés sur mesure : ils rendent
  **deux** valeurs via `$_DISPOSITION_FUZZY_FILE`, donc l'effet de bord est dans le
  contrat, et leur tier 2 est une **paraphrase** (classe A du TSV) dont la
  migration est un arbitrage de tolérance, pas un portage.
- **La réduction de taille de `dispatch-lib.sh`.** La tendance réelle du dépôt
  depuis le 05/09 n'est pas bash→Rust mais monolithe→modules bash
  (`pr-push-guard.sh` 24 Ko / mika#2520, `cwd-guard.sh` 8 Ko / mika#2536) — et
  pendant ce temps le fichier a doublé. Ce motif réduit le blast radius **sans
  changer de langue**, donc il ne ferme aucune des trois classes de panne de
  parsing citées par le ticket. Les deux directions sont compatibles et ne
  répondent pas à la même question ; ce plan prend la seconde.
- **La fragilité `source … 2>/dev/null || true`** du harnais (M2). Réelle,
  adjacente, et **traitée localement seulement** : le terme d'anti-vacuité de R4
  couvre le bloc de parité, pas les 215 autres sites. **Suivi**, précondition :
  une mesure montrant qu'un sourcing raté a produit un vert.
- **Le faux positif « callout cité pointant un plan qui existe »** (M3, écart 1,
  seconde moitié). Population non mesurée ; c'est l'objet du suivi de la
  Fire-Disposition, pas de cette PR.
- **Le nom de famille des sous-commandes de prédicat.** Décidé à la phase 2, avec
  deux usages en main (R2).

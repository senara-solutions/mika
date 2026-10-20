# La porte de section `reflection` est registration-only par construction

**Ticket :** senara-solutions/mika#2850
**Amont :** mika#2456 (vol 5), suivi direct de mika#2847
**Type :** attestation structurelle — aucun fichier de production modifié
**Branche :** `feat/2850/la-porte-de-section-reflection-est`

---

## Contexte

mika#2456 se livre en vols. Les quatre premiers portent sur la porte
**niveau-agent** puis sur la porte de **section** `heartbeat` :

| vol | PR | ce qu'il porte |
|---|---|---|
| 1 | #2835 | les sondes de la porte à l'**enregistrement** |
| 2 | #2844 | le **filet au tir** — `refuse_recurring_fire_if_agent_disabled` |
| 3 | #2848 | la **surface d'audit** de ce filet (scan SOLE-WRITER) |
| 4 | #2851 | la porte de section **`heartbeat`** est registration-only par construction |
| 5 | *ce vol* | la **même** propriété pour la porte de section **`reflection`** |

La proposition attestée est la même, mot pour mot, avec un seul label changé :
*tous les lecteurs de la porte de section sont au geste d'enregistrement, et il
n'en existe aucun au tir.* Rien ne rougit aujourd'hui si `reflection` cesse de
la tenir.

### Pourquoi la propriété compte, et ce que son absence coûte

Une porte de section lue **au tir** et non à l'enregistrement laisse la ligne
récurrente **posée** en base. La ligne tire, le dispatch démarre, et la décision
d'abandon arrive après que les tours LLM ont été brûlés — c'est exactement le
coût que mika#2456 a mesuré au vol 2. Une porte lue **à l'enregistrement**
n'arme rien, et sa branche `else` retire ce qui restait armé. L'écart entre les
deux formes n'est pas visible dans un diff de 10 lignes : il se lit sur
l'emplacement de l'appel, et c'est précisément ce qu'un scan de source peut
tenir et qu'une sonde comportementale ne tient pas.

### Les deux sites, mesurés

`reflection_cron_for_agent` est définie en
`crates/mika-agent/src/task_engine/mod.rs:909` et lue à **deux** endroits, les
mêmes que `heartbeat` :

| site | ce qu'il est | lecture | `ensure_recurring_task` | branche `else` |
|---|---|---|---|---|
| `crates/mika-agent/src/server/mod.rs` | démarrage du démon mika-spirit, par agent | `:1918` | `:1920` | `cancel_recurring_task_by_label("reflection")` à `:1928` |
| `crates/mika-cli/src/commands/chat.rs` | démarrage du TUI CLI, agent courant | `:245` | `:246` | `cancel_recurring_task_by_label("reflection")` à `:256` |

Distances **réelles** lecture → annulation, sur texte brut : **10 lignes**
(`server/mod.rs`) et **11 lignes** (`chat.rs`). Le texte brut est une borne
supérieure, puisque `source_scan::strip_comment_lines` **supprime** les lignes
de commentaire au lieu de les vider. La fenêtre de 25 lignes du prédicat de
mika#2847 — retenue là sur des distances de 9 et 11 — tient donc ici avec plus
du double de marge. **Ce vol ne choisit aucune fenêtre** : `WINDOW` est une
constante interne au prédicat partagé, et la mesure ci-dessus sert à vérifier
qu'elle tient, pas à en retenir une seconde.

### Asymétrie de défaut entre les deux portes, et elle ne change rien ici

`heartbeat_enabled_for_agent` rend `true` quand la section est absente ;
`reflection_cron_for_agent` rend `None` — la réflexion est **opt-in**, elle
exige `[reflection] enabled = true` *et* un `time` analysable *et* un fuseau
résoluble. C'est un écart de **résolution**, pas de **topologie** : la
proposition de ce vol porte sur *où* la porte est lue, jamais sur *ce qu'elle
rend*. L'écart est noté pour qu'un relecteur ne le prenne pas pour une omission.

---

## Préconditions — ce vol dépend du merge de #2851, et ça se vérifie

Les trois aides que ce vol réutilise — `reads_section_gate`,
`heartbeat_gate_reader_sites`, `heartbeat_gate_reads_at_registration` — sont
livrées par **#2851, encore ouverte** au moment du grooming. Elles ne sont
**pas** sur `origin/main` (vérifié :
`git grep -n HEARTBEAT_GATE_REGISTRATION_SITES origin/main -- crates/` ne rend
rien). D0 **édite** ce code : sans lui, il n'y a rien à paramétrer.

**Sonde, à exécuter en premier :**

```bash
grep -n "heartbeat_gate_reads_at_registration\|HEARTBEAT_GATE_REGISTRATION_SITES" \
  crates/mika-agent/src/canonical_tokens.rs
```

- **Deux symboles trouvés** ⇒ #2851 est dans l'arbre, le vol procède.
- **Rien trouvé** ⇒ **halte-et-remontée**. Ne pas recopier le prédicat depuis la
  branche de #2851 : ça livrerait le second exemplaire que mika#2653 interdit, et
  le merge de #2851 produirait un conflit sur un fichier dont les deux versions
  seraient défendables. L'action est d'attendre le merge, pas de contourner.

C'est la seule précondition de ce vol, et elle est binaire.

---

## Le défaut qu'une transposition **littérale** porterait, et il est mesuré

Transposer le prédicat de mika#2847 en changeant seulement l'aiguille et le
label produirait un détecteur **inerte sur sa régression la plus probable**. La
cause est l'adjacence des deux portes.

Dans les deux sites, les blocs `heartbeat` et `reflection` sont **collés** :
`cancel_recurring_task_by_label("heartbeat")` est à `server/mod.rs:1915`, trois
lignes **avant** la lecture de `reflection` à `:1918` ; même adjacence dans
`chat.rs` (`:~242` contre `:245`). Le prédicat de mika#2847 cherche ses trois
termes **indépendamment** dans la fenêtre. Donc, si la branche `else`
d'annulation de `reflection` était supprimée :

- `ensure_recurring_task` → trouvé (celui de `reflection`, et celui de `heartbeat`) ;
- `cancel_recurring_task_by_label(` → trouvé, **celui de `heartbeat`** ;
- `"reflection"` → trouvé, c'est l'argument de label passé à `ensure_recurring_task` ;

⇒ le prédicat rendrait **`true`** sur un arbre où `reflection` n'annule plus
rien. La régression que ce vol existe pour voir passerait au vert.

Le même raisonnement montre qu'une paramétrisation **naïve** du label laisserait
le paramètre non portant : un prédicat qui ignorerait son argument et garderait
`"heartbeat"` en dur resterait **vert sur `reflection`**, puisque `"heartbeat"`
est dans la fenêtre des deux sites. Un paramètre qu'aucun test ne peut faire
rougir n'est pas un paramètre.

**Correctif, et il est minimal.** Le terme d'annulation devient
**corrélé au label** : la fenêtre doit porter une ligne contenant
`cancel_recurring_task_by_label("<label>"`, les deux ensemble. C'est vérifié sur
les **quatre** sites concernés — l'appel et son littéral sont sur la **même
ligne** partout, y compris dans la forme chaînée multi-ligne de `chat.rs`, où
`.cancel_recurring_task_by_label("reflection")` occupe sa propre ligne :

| site | label | ligne portant les deux termes |
|---|---|---|
| `server/mod.rs` | `heartbeat` | `:1915` |
| `server/mod.rs` | `reflection` | `:1928` |
| `chat.rs` | `heartbeat` | `.cancel_recurring_task_by_label("heartbeat")` |
| `chat.rs` | `reflection` | `:256` |

Le resserrage est donc **strictement plus fort et vert sur l'arbre courant**,
pour les deux labels. Il renforce l'attestation de `heartbeat` au passage ; ce
n'est pas un effet de bord à cacher, c'est la raison pour laquelle le prédicat
est partagé.

**`ensure_recurring_task` reste un terme de fenêtre non corrélé, et c'est
mesuré, pas concédé.** Aux deux sites, l'appel et son argument de label sont sur
des lignes **différentes** (`server/mod.rs` : appel `:1920`, label `:1923`) :
corréler ce terme-là sur une ligne rendrait le prédicat rouge sur un arbre
conforme. Le résidu que ça laisse — supprimer l'appel `ensure_recurring_task` de
`reflection` tout en gardant son annulation garderait la fenêtre verte via
l'appel voisin de `heartbeat` — est **nommé au § Risques** plutôt que corrigé :
le fermer exigerait une corrélation multi-ligne, et un prédicat qui devine la
portée d'un appel est la forme qui rougit sur un `cargo fmt`.

---

## Périmètre — ce que CE vol livre

Tout dans `crates/mika-agent/src/canonical_tokens.rs`, `mod tests`.

### D0 — le prédicat partagé : paramétrisation par le label, et resserrage

`heartbeat_gate_reads_at_registration` et `heartbeat_gate_reader_sites`
deviennent **génériques de section** et changent de nom en conséquence — un nom
qui porte `heartbeat` sur une aide appelée par les deux labels est un nom qui
mentira au premier relecteur :

```rust
fn section_gate_reader_sites(needle: &str) -> Vec<String>
fn section_gate_reads_at_registration(content: &str, needle: &str, label: &str) -> bool
```

- `reads_section_gate(line, needle)` est reprise **telle quelle** : elle est déjà
  paramétrée par l'aiguille, c'est pour ça que mika#2847 l'a écrite ainsi.
- Le `label` passe en paramètre, **guillemets compris**, et chaque appelant le
  compose à l'exécution (`format!("\"re{}\"", "flection")`,
  `format!("\"heart{}\"", "beat")`) — même geste et même raison que l'aiguille :
  `canonical_tokens.rs` est lui-même un fichier de `production_sources()` et ne
  doit pas se dénoncer (mika#2496).
- Le terme d'annulation devient corrélé :
  `window_lines.any(|l| l.contains(&format!("cancel_recurring_task_by_label({label}")))`.
  `label` portant déjà ses guillemets, le motif cherché est
  `cancel_recurring_task_by_label("reflection`.
- `const WINDOW: usize = 25` **reste interne et inchangée** : un seul site de
  vérité pour la fenêtre, partagé par les deux labels.
- Les trois tests `mika2847_*` sont mis à jour pour appeler les aides renommées
  et passer leur label. **Leurs noms ne changent pas** : ils sont l'attestation
  de `heartbeat` et leur identité est une référence.
- Le doc-comment de `mika2847_le_predicat_dadjacence_voit_une_lecture_hors_enregistrement`
  est rectifié : il affirme aujourd'hui que le prédicat « ne peut pas emprunter
  l'attestation d'un prédicat partagé ». Après D0 il **est** partagé, et c'est
  lui qui prête son attestation à `reflection` sur l'axe de l'adjacence. Un
  doc-comment faux sur le périmètre de sa propre garde est la façon dont la
  garde suivante se fait recopier.

**Aucun autre fichier de `canonical_tokens.rs` n'est touché, et aucun fichier de
production.**

### D1 — le recensement des lecteurs, et son allowlist née vide

```rust
const REFLECTION_GATE_REGISTRATION_SITES: &[&str] = &[
    "crates/mika-agent/src/server/mod.rs",
    "crates/mika-cli/src/commands/chat.rs",
];
const REFLECTION_GATE_READER_ALLOWED: &[&str] = &[];
```

Chaque entrée du recensement porte sa raison en commentaire (boucle de démarrage
du démon, par agent / démarrage du TUI, agent courant). Le doc-comment nomme la
distinction **périmètre ≠ allowlist** dans les mêmes termes que mika#2847 : le
recensement naît à deux entrées parce que la proposition attestée est *« tous les
lecteurs sont à l'enregistrement »*, pas *« il y a un lecteur »* — et le scan de
D2 **exige** que chaque entrée soit trouvée, donc en retirer une rend le scan
rouge au lieu de relâcher quoi que ce soit.

### D2 — le scan : aucun lecteur hors du recensement

`mika2850_la_porte_de_section_reflection_na_que_des_lecteurs_denregistrement`

- Aiguille composée à l'exécution :
  `format!("reflection_cron_for{}agent", "_")`.
- Lecteurs via `section_gate_reader_sites(&needle)`, filtrés de
  `REFLECTION_GATE_READER_ALLOWED`.
- **Anti-vacuité d'abord** : chaque entrée de `REFLECTION_GATE_REGISTRATION_SITES`
  doit être présente parmi les lecteurs trouvés. Un scan qui ne trouve personne se
  lit exactement comme un scan propre (mika#2103 / mika#2205).
- Puis le volet « étrangers », avec un message de résolution qui nomme
  **mika#2201** : router la lecture par un site d'enregistrement existant, ne
  **pas** ajouter d'entrée d'allowlist.

La définition en `task_engine/mod.rs:909` est écartée par le filtre `fn …(` de
`reads_section_gate` — **nécessaire ici** : le premier `#[cfg(test)]` de ce
fichier est à `:944`, donc la troncature de `production_sources()` ne l'enlève
pas.

### D3 — chaque lecteur recensé annule auprès de sa lecture

`mika2850_chaque_lecteur_recense_annule_a_lenregistrement`

Pour chaque fichier de `production_sources()` présent au recensement :
`section_gate_reads_at_registration(&content, &needle, &label)` doit rendre
`true`. Anti-vacuité de la traversée elle-même : le compte de sites retrouvés
doit égaler `REFLECTION_GATE_REGISTRATION_SITES.len()`, sinon une entrée ne
désigne plus un fichier scanné et la boucle n'a mesuré qu'une partie du
périmètre.

### D4 — le contrôle de bonne foi du **paramètre de label**, et il est dû

`mika2850_le_predicat_honore_son_parametre_de_label`

L'axe de l'**adjacence** est déjà attesté par le D4 de mika#2847, et ce vol ne
le recopie pas (mika#2653). L'axe du **label** est neuf : il naît avec D0, et le
§ précédent montre qu'il est précisément celui qu'une fenêtre contaminée par la
porte voisine rend non portant. Deux axes distincts, deux contrôles — pas un
doublon.

Trois assertions sur un fixture composé à l'exécution :

1. **Négative — mauvais label.** Lecture de l'aiguille `reflection`, paire
   `ensure_recurring_task` + `cancel_recurring_task_by_label("heartbeat")`, et le
   littéral `"reflection"` présent uniquement comme argument de label. Appelé
   avec le label `reflection`, le prédicat doit rendre **`false`**. C'est ce
   fixture qui échoue si le paramètre est ignoré, et c'est la régression réelle
   de l'arbre (suppression de la branche `else` de `reflection` à côté de celle
   de `heartbeat`).
2. **Négative — annulation absente.** Le même fixture sans aucun
   `cancel_recurring_task_by_label` : `false`.
3. **Positive — la forme réelle.** La paire complète de `reflection`, label
   corrélé sur sa ligne : `true`. Sans ce volet, un prédicat trop serré rendrait
   D3 rouge sur un arbre conforme et le test ne le dirait pas.

### D5 — l'assertion auto-nettoyante de l'allowlist

`mika2850_lallowlist_des_lecteurs_reflection_est_livree_vide` — assert
`REFLECTION_GATE_READER_ALLOWED.is_empty()`, avec le message qui nomme
mika#2323 : une allowlist née vide est un emplacement où déposer la prochaine
infraction, et celle-ci doit rester vide.

---

## Périmètre — ce que CE vol ne livre PAS

- **La sonde comportementale (le D6 de mika#2847, V8/V8-bis).** Hors du périmètre
  nommé par le ticket — qui dit « transposer **D1–D3** » — et hors du suivi tel
  que le § *ce que CE vol ne livre PAS* de mika#2847 l'a ouvert. Le motif est de
  fond et pas seulement d'obéissance : V8 atteste ce que la porte **rend**
  (`[heartbeat] enabled = false` ⇒ ligne `cancelled`), pas **où** elle est lue.
  Pour `reflection`, « ce qu'elle rend » est la résolution cron (fuseau, DST,
  `time` analysable) — une autre proposition, avec ses propres cas limites, qui
  mérite son ticket plutôt qu'un wagon. Ce vol atteste une **topologie de
  source**, entièrement.
- **L'extraction d'un helper `register_or_cancel_section(db, home_dir, label, …)`**
  qui ramènerait les quatre blocs à un seul lecteur. **La question est posée ici
  pour les deux labels à la fois, comme le ticket le demande, et la réponse est
  « pour aucun ». Ce vol ne la rouvre pas.** Quatre raisons, dans l'ordre où elles
  mordent :
  1. **Les quatre blocs ne sont pas identiques.** `server/mod.rs` porte
     `agent = %agent_name` dans ses deux `warn!`, `chat.rs` non ; `heartbeat`
     prend `HEARTBEAT_CRON` côté serveur et le littéral `"0 0 * * * *"` côté CLI ;
     `reflection` prend un cron **calculé** là où `heartbeat` prend une constante.
     Le helper devrait porter tout ça en paramètres, et le gain « un seul lecteur »
     se dissoudrait dans une liste d'arguments.
  2. **C'est un refactor d'un chemin de démarrage**, dont la régression se paie au
     boot de la flotte entière — pour une PR dont le livrable est une attestation.
  3. **L'ordre est le mauvais.** Dans le même vol, un détecteur rouge ne
     distinguerait pas « le refactor a cassé l'adjacence » de « le détecteur est
     mal écrit ». Après ce vol, le refactor arrive **avec un filet** : D2/D3
     rougissent si le helper déplace la lecture hors du geste. C'est le détecteur
     qui rend le refactor sûr, pas l'inverse.
  4. **Le scan atteste la propriété sans y toucher** — c'est exactement ce que le
     ticket demande.
- **Toute modification d'un fichier de production.** Si un changement de
  production paraît nécessaire, c'est un écart au plan qui **se remonte** plutôt
  qu'il ne se décide.
- **Toute extension aux autres `trigger_type`**, et toute reprise du filet au tir
  des vols 2–3 : inchangés.
- **Aucune nouvelle requête opérateur**, donc aucune ligne dans
  `crates/mika-agent/CLAUDE.md` § *Audit Log* : ce vol n'écrit aucune ligne
  `audit_events`.

---

## Ordre de commit

Un seul vol, un seul fichier, **deux commits** — parce que D0 touche du code
livré par un autre ticket et doit se relire seul.

1. **D0** — paramétrisation et resserrage du prédicat partagé, renommages, mise à
   jour des trois tests `mika2847_*` et du doc-comment de leur D4.
   `cargo test -p mika-agent canonical_tokens` doit être **vert à ce commit** :
   c'est la preuve que le resserrage ne casse pas l'attestation de `heartbeat`.
2. **D1 à D5** — le recensement `reflection`, son allowlist, et les quatre tests.

**Plancher de ce vol :** D2 (le scan des étrangers) avec son anti-vacuité, et son
allowlist vide (D5). C'est ce qui rougit le jour où un lecteur de `reflection`
apparaît au tir, et c'est la seule chose que rien ne tient aujourd'hui. D3 et D4
resserrent la propriété de « dans ces fichiers » à « au geste d'enregistrement,
avec le bon label ».

---

## Contrat de vérification

| # | ce qui est mesuré | comment |
|---|---|---|
| V1 | les deux lecteurs de production de `reflection_cron_for_agent` sont trouvés | anti-vacuité de `mika2850_la_porte_de_section_reflection_na_que_des_lecteurs_denregistrement` |
| V2 | aucun lecteur hors recensement | volet « étrangers » du même test, message nommant mika#2201 |
| V3 | la lecture est adjacente au geste, sur les deux sites | `mika2850_chaque_lecteur_recense_annule_a_lenregistrement` + anti-vacuité de la traversée |
| V4 | le prédicat **honore son paramètre de label** | `mika2850_le_predicat_honore_son_parametre_de_label` (deux négatives, une positive) |
| V5 | l'allowlist est née vide et le reste | `mika2850_lallowlist_des_lecteurs_reflection_est_livree_vide` |
| V6 | le resserrage ne casse pas l'attestation de `heartbeat` | les trois tests `mika2847_*` verts **au commit 1**, inchangés dans leurs noms |
| V7 | l'aiguille et le label sont composés à l'exécution | `canonical_tokens.rs` n'apparaît pas parmi les lecteurs trouvés par V1 |
| V8 | rien ne régresse sur les vols 1–4 | `cargo test -p mika-agent` entier, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

---

## Fire-Disposition

Ce vol livre **quatre détecteurs** : D2, D3, D4, D5 — scans et assertions
structurelles dans `canonical_tokens.rs`. Chemin de succès pour chacun :
*aucune violation trouvée*. (D0 n'est pas un détecteur : il paramètre et resserre
un prédicat existant.)

**Option retenue : (a) exception nommée en allowlist — née vide.**

- `REFLECTION_GATE_READER_ALLOWED: &[&str] = &[]`, avec son assertion
  auto-nettoyante `mika2850_lallowlist_des_lecteurs_reflection_est_livree_vide`
  (D5), qui rougit si une entrée y est déposée sans que le scan soit retiré.
- **Violations préexistantes : zéro, et c'est mesuré — pas espéré.** Le grep de
  préparation (`grep -rn "reflection_cron" crates/`) rend **trois** hits dans
  l'arbre : la **définition** (`task_engine/mod.rs:909`, écartée par le filtre
  `fn …(` de `reads_section_gate`) et exactement **deux** lecteurs de production,
  tous deux au recensement — `server/mod.rs:1918` et
  `mika-cli/src/commands/chat.rs:245`. Aucun hit dans `dispatcher.rs`, donc aucun
  lecteur au tir. Le prédicat d'adjacence resserré est satisfait par les deux
  sites recensés, à 10 et 11 lignes de distance pour une fenêtre de 25.
- **Le recensement n'est pas l'allowlist**, et leur confusion est la seule façon
  de mal lire cette disposition. `REFLECTION_GATE_REGISTRATION_SITES` naît à
  **deux** entrées parce que la proposition attestée est *« tous les lecteurs sont
  à l'enregistrement »* : c'est un **périmètre** au sens de mika#2536 (précédents
  `TESTIMONY_PREDICATE_READERS`, mika#2627 ; `HEARTBEAT_GATE_REGISTRATION_SITES`,
  mika#2847), et le scan **exige** chaque entrée. mika#2323 porte sur la case où
  déposer la prochaine infraction : c'est `REFLECTION_GATE_READER_ALLOWED`, et
  elle est vide.
- **Message de résolution, sur les deux scans.** Un lecteur non recensé se résout
  en **routant la lecture par un site d'enregistrement existant**, jamais par une
  entrée d'allowlist (doctrine mika#2201) — un lecteur au **tir** retournerait
  précisément la propriété que ce scan atteste, et l'allowlister serait la
  documenter au lieu de la tenir. Un lecteur d'enregistrement **nouveau et
  légitime** (une troisième porte d'entrée du substrat) se déclare dans le
  **recensement**, avec sa raison, et doit satisfaire D3.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| D0 — `canonical_tokens.rs` : paramétrisation + resserrage du prédicat partagé, renommages, mise à jour des 3 tests `mika2847_*` et du doc-comment de leur D4 | 55 |
| D1 — `REFLECTION_GATE_REGISTRATION_SITES` + `REFLECTION_GATE_READER_ALLOWED`, doc-comments compris | 34 |
| D2 — scan des étrangers + anti-vacuité | 38 |
| D3 — scan d'adjacence + anti-vacuité de la traversée | 36 |
| D4 — contrôle de bonne foi du paramètre de label (2 négatives, 1 positive) | 32 |
| D5 — assertion auto-nettoyante de l'allowlist | 12 |

Total estimé : 207 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 207 < 279 et 207 < 1000 : les deux bornes
passent, la plus serrée avec 72 lignes de marge. Le vol est d'un seul fichier,
sans migration, sans fichier de production touché, et sans dépendance de build :
la fin de pipeline est celle d'une PR de tests. Le volume est **cadré par
construction** — c'est la transposition d'une forme déjà écrite (#2851, 178
lignes sur le même fichier), avec un détecteur de moins (pas de sonde
comportementale) et une aide de plus (D0). Le risque de dérive est sur D0, borné
par le commit 1 qui doit être vert seul.

---

## Definition of Done

- [ ] **Précondition vérifiée** — `heartbeat_gate_reads_at_registration` et
      `HEARTBEAT_GATE_REGISTRATION_SITES` présents dans l'arbre (#2851 mergée).
      Sinon : halte-et-remontée, aucun code écrit.
- [ ] D0 — `section_gate_reader_sites(needle)` et
      `section_gate_reads_at_registration(content, needle, label)` livrées ;
      terme d'annulation **corrélé au label** ; `WINDOW` inchangée et unique ;
      les trois tests `mika2847_*` mis à jour, **noms conservés**, verts ;
      doc-comment du D4 de mika#2847 rectifié sur le caractère désormais partagé
      du prédicat.
- [ ] D1 — `REFLECTION_GATE_REGISTRATION_SITES` (deux entrées, chacune avec sa
      raison) et `REFLECTION_GATE_READER_ALLOWED` (vide), avec le doc-comment qui
      nomme la distinction périmètre / allowlist.
- [ ] D2 — `mika2850_la_porte_de_section_reflection_na_que_des_lecteurs_denregistrement` :
      aiguille composée à l'exécution, définition écartée, anti-vacuité **avant**
      le volet « étrangers », message de résolution nommant mika#2201.
- [ ] D3 — `mika2850_chaque_lecteur_recense_annule_a_lenregistrement` : les trois
      termes du geste dans la fenêtre décommentée, sur les deux sites, plus
      l'anti-vacuité de la traversée.
- [ ] D4 — `mika2850_le_predicat_honore_son_parametre_de_label` : mauvais label ⇒
      `false`, annulation absente ⇒ `false`, forme réelle ⇒ `true`.
- [ ] D5 — `mika2850_lallowlist_des_lecteurs_reflection_est_livree_vide`.
- [ ] `cargo test -p mika-agent` vert, `cargo clippy` sans avertissement nouveau,
      `cargo fmt --check` propre.
- [ ] `scripts/verify-pipeline.sh origin/main` vert (dont U2 : ce plan porte
      `## Acceptance criteria`).
- [ ] **Aucun fichier de production modifié** — le diff ne touche que
      `canonical_tokens.rs` (et ce plan).
- [ ] Le corps de la PR nomme **la contamination croisée entre les deux portes
      adjacentes** et le resserrage qu'elle impose. Un relecteur qui compare le
      corps du ticket (« réutiliser tel quel le prédicat ») au diff (le prédicat
      est resserré) doit trouver l'explication dans la PR, pas la reconstruire.

---

## Acceptance criteria

Le corps de mika#2850 ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés de son § *Périmètre proposé* et du contrat de
vérification ci-dessus.

1. **AC1 — un lecteur de production non recensé de `reflection_cron_for_agent`
   fait échouer un test.** Le scan parcourt `production_sources()` (tout
   `crates/*/src`, CLI comprise), écarte la définition et le code de test, et
   nomme les étrangers trouvés.
2. **AC2 — le scan n'est pas vert par vacuité.** Il échoue si l'un des deux sites
   recensés — `crates/mika-agent/src/server/mod.rs` et
   `crates/mika-cli/src/commands/chat.rs` — cesse d'être trouvé.
3. **AC3 — l'aiguille et le label sont composés à l'exécution**, de sorte que
   `canonical_tokens.rs` ne se compte pas lui-même comme lecteur.
4. **AC4 — chacun des deux sites recensés lit la porte au voisinage immédiat du
   geste d'enregistrement**, attesté par la présence conjointe de
   `ensure_recurring_task` et d'une ligne portant
   `cancel_recurring_task_by_label("reflection"` dans une fenêtre bornée de lignes
   décommentées.
5. **AC5 — le prédicat d'adjacence est réutilisé, pas recopié** (mika#2653) : il
   existe **un seul** exemplaire dans `canonical_tokens.rs`, appelé par les deux
   labels, et la fenêtre a **un seul** site de vérité.
6. **AC6 — le paramètre de label est portant** : un prédicat qui l'ignorerait
   fait échouer un test. La suppression de la branche `else` d'annulation de
   `reflection` fait rougir D3 malgré la présence de celle de `heartbeat` trois
   lignes plus haut.
7. **AC7 — l'attestation de `heartbeat` ne régresse pas** : les trois tests
   `mika2847_*` restent verts sous le prédicat resserré, et conservent leurs
   noms.
8. **AC8 — l'allowlist des lecteurs est livrée vide**, et une assertion dédiée
   rougit si une entrée y est ajoutée.
9. **AC9 — le message d'échec de chaque scan nomme la résolution attendue** :
   router la lecture par un site d'enregistrement, ne pas ajouter d'entrée
   d'allowlist (mika#2201).
10. **AC10 — aucun fichier de production n'est modifié par ce vol.** La propriété
    est attestée, pas construite.
11. **AC11 — la question du helper `register_or_cancel_section` est répondue dans
    le plan et dans le corps de la PR, pour les deux labels à la fois**, avec ses
    motifs — ce n'est ni une omission silencieuse ni un report non nommé.

---

## Risques et contre-mesures

| risque | contre-mesure |
|---|---|
| **#2851 n'est pas mergée quand le pilote démarre**, et D0 édite du code absent. | Précondition binaire en tête de plan, avec sa sonde `grep`. Rien trouvé ⇒ halte-et-remontée. Interdiction explicite de recopier le prédicat depuis la branche de #2851 (ce serait le second exemplaire que mika#2653 interdit, et un conflit de merge sur deux versions défendables). |
| **La contamination croisée entre portes adjacentes** rend un prédicat littéral inerte sur sa régression la plus probable. | C'est le § dédié de ce plan, et le resserrage du terme d'annulation (corrélé au label). Vérifié vert sur les **quatre** sites : l'appel et son littéral sont sur la même ligne partout, forme chaînée de `chat.rs` comprise. D4 pin le paramètre. |
| **Résidu connu :** supprimer l'appel `ensure_recurring_task` de `reflection` en gardant son annulation laisserait la fenêtre verte via l'appel voisin de `heartbeat`. | Nommé plutôt que corrigé, et la raison est mesurée : l'appel et son argument de label sont sur des lignes **différentes** (`:1920` / `:1923`), donc corréler ce terme rendrait le prédicat rouge sur un arbre conforme. Un prédicat qui devine la portée d'un appel rougit au premier `cargo fmt`. Le résidu est de surcroît moins probable que celui que D4 ferme : supprimer l'enregistrement en gardant l'annulation éteint la fonctionnalité de façon visible au boot. |
| **Le resserrage casse l'attestation de `heartbeat`** et `heartbeat` devient rouge. | Mesuré avant écriture, site par site (tableau du § dédié). Et structurellement tenu par l'**ordre de commit** : le commit 1 porte D0 seul et doit être vert, donc la régression se voit avant que `reflection` n'existe. |
| **Le renommage des aides est pris pour une réécriture de mika#2847.** | Les **noms des trois tests `mika2847_*` ne changent pas** — ce sont les références de l'attestation `heartbeat`. Seules les aides privées sont renommées, et seulement parce qu'un nom portant `heartbeat` sur une aide appelée par les deux labels mentirait. Le corps de la PR le dit. |
| **La troncature de `production_sources()` au premier `#[cfg(test)]` masque un vrai lecteur.** | Mesuré sur les trois fichiers concernés : premier `#[cfg(test)]` à `server/mod.rs:2177` (> 1928), `task_engine/mod.rs:944` (> 909, donc la définition **est** dans le texte de production — c'est pourquoi le filtre `fn …(` est nécessaire), et **aucun** dans `chat.rs`. Aucun des deux sites recensés n'est tronqué. |
| **Le recensement à deux entrées se lit comme une allowlist née non vide**, contre mika#2323. | Dit trois fois, aux trois endroits où un relecteur le rencontre : doc-comment de la constante, § *Fire-Disposition* de ce plan, corps de la PR. Précédents cités : `HEARTBEAT_GATE_REGISTRATION_SITES` (mika#2847), `TESTIMONY_PREDICATE_READERS` (mika#2627). |
| **L'absence de sonde comportementale est lue comme une omission.** | Nommée au § *ce que CE vol ne livre PAS*, avec son motif de fond : V8 atteste ce que la porte **rend**, ce vol atteste **où** elle est lue ; et pour `reflection` « ce qu'elle rend » est la résolution cron (fuseau, DST), une autre proposition. Le ticket et le § *hors périmètre* de mika#2847 bornent tous deux le suivi à D1–D3. |
| **La fenêtre de 25 devient trop serrée** si un commit insère du code entre la lecture et l'annulation. | L'échec est **bruyant** et dit la bonne chose (« la lecture a quitté le site d'enregistrement ») ; la résolution est de relire le site, pas d'élargir la fenêtre par réflexe. Mesure actuelle : 10 et 11 lignes de texte **brut**, donc moins encore une fois décommenté. |

---

## Références

- mika#2850 — ce ticket.
- mika#2847 / PR #2851 — la même propriété pour `heartbeat` : le recensement, le
  scan des étrangers, le prédicat d'adjacence et son contrôle de bonne foi.
  **Dépendance de merge de ce vol.**
- mika#2456 — la porte niveau-agent, vols 1 à 3 (#2835, #2844, #2848).
- mika#2653 — réutiliser un prédicat partagé en le paramétrant plutôt qu'en
  livrer un second exemplaire (`audit_name_literal_present`).
- mika#2323 — une allowlist née vide est un emplacement où déposer la prochaine
  infraction.
- mika#2201 — un lecteur non recensé se route par un site d'enregistrement, il ne
  s'allowliste pas.
- mika#2103 / mika#2205 — un scan vide se lit comme un scan propre : l'anti-vacuité
  passe avant le volet qui accuse.
- mika#2496 — un scan de source ne doit pas se dénoncer lui-même : aiguille et
  littéraux composés à l'exécution.
- mika#2536 / mika#2627 — un recensement est un périmètre, pas une liste
  d'exemptions (`TESTIMONY_PREDICATE_READERS`).
- mika#1574 / mika#2306 — Fire-Disposition : les trois options canoniques.
- `crates/mika-agent/src/source_scan.rs` — `strip_comment_lines` (`:128`),
  `is_test_source_path` (`:79`).

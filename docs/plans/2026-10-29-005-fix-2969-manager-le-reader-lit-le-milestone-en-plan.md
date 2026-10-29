# mika#2969 — Le reader lit tout son milestone, et dit quand il n'a pas pu

Ticket : senara-solutions/mika#2969 — `fix`, `loop-substrate`
Branche : `fix/2969/manager-le-reader-lit-le-milestone-en`
Base : `origin/main` à `7df2c1e1c`

---

## 1. Ce qui est mesuré

Toutes les mesures ci-dessous sont prises le **2026-10-29** sur `senara-solutions/mika`,
milestone **33** (titre réel : *« Substrat de boucle »*), depuis ce worktree.

### 1.1 Le milestone, par l'API

`gh api repos/senara-solutions/mika/milestones/33` rend `open_issues=14`,
`closed_issues=178` — soit **192 items**. Par pagination REST explicite :

| population | mesure | compte |
|---|---|---|
| issues du #33 | 2 pages (39 + 78) | **117** |
| PR du #33 | 2 pages (61 + 14) | **75** |
| total | | **192** |
| dont issues ouvertes | | 14 |
| dont issues fermées | | 103 |
| dont PR ouvertes | | 0 |
| dont PR fermées | | 75 |

`14 + 103 = 117` et `103 + 75 = 178` : les deux lectures se recoupent exactement.

### 1.2 Ce que le reader rend aujourd'hui

`gh issue list --repo senara-solutions/mika --milestone 33 --state all --limit 100`
rend **100** tickets. Le plus petit numéro rendu au moment de la mesure est **#2037**.
**17 tickets du milestone sortent** de `MilestoneState::sub_issues` (117 − 100), dont
les deux ouverts **#1721** et **#1727** — tous deux vérifiés à `milestone.number == 33`,
état `OPEN`.

Le ticket relève `#2034` comme plus petit numéro à ~19:24Z ; cette mesure-ci rend `#2037`,
plus tard dans la journée. L'écart n'est pas une contradiction, c'est le mécanisme que le
ticket décrit : trois tickets créés entre les deux mesures ont poussé trois anciens dehors.

### 1.3 Le coût de la borne (discipline mika#2924)

Trois exécutions de chaque forme, en séquence, depuis ce worktree :

| forme | durée | tickets rendus | invocations `gh` |
|---|---|---|---|
| `--limit 100` | ≈ 2,5 s | 100 | 1 |
| `--limit 1000` | 2,6 à 3,0 s | **117** | 1 |

**`gh issue list --limit N` pagine en interne.** Une seule invocation de sous-processus,
quel que soit `N` ; le nombre de pages GraphQL sous-jacentes vaut `ceil(rendus / 100)`,
donc **2 aujourd'hui**, piloté par la taille réelle du milestone et non par la constante.
Le compte d'appels `gh` par cycle du manager reste donc **inchangé à 7**, ce qui est la
propriété que `mika2924_…_sans_appel_de_plus` tient déjà et que ce vol ne doit pas casser.

Conclusion de coût : passer la borne de 100 à 1000 coûte **0 appel `gh` supplémentaire**
et ≈ 0,1 à 0,5 s par cycle, pour un cycle dont la cadence est en minutes.

---

## 2. Le défaut, et la prémisse d'AC3 qui est fausse

### 2.1 Le défaut

`crates/mika-agent/src/milestone_manager/reader.rs`, `read_with_runner`, appel 2
(l. 190 à 205) : `--limit`, `"100"`, littéral. Rien ne détecte la page pleine, rien
ne le dit.

La lecture « outside », juste après, porte déjà les trois éléments qui manquent ici :

- borne **nommée** : `OUTSIDE_READ_LIMIT: usize = 100` (l. 38), `pub` parce que le
  rapporteur l'épelle ;
- page pleine **détectée** : `let truncated = issues.len() >= OUTSIDE_READ_LIMIT;`
  (l. 619, dans `compose_outside_candidates`) ;
- verdict **porté par l'état** puis par le rapport : `outside_read_truncated: Option<bool>`
  (reader l. 283, reporter l. 400 et l. 467).

Ce vol porte le même patron sur la lecture du milestone.

### 2.2 La prémisse d'AC3 est fausse, et le plan la corrige

AC3 demande que le rapport dise le compte lu **à côté** de `open_issues + closed_issues`,
et qu'« un écart entre les deux se voie au rapport ». La mesure du § 1.1 montre que
**ces deux nombres ne comptent pas la même population** :

- `open_issues + closed_issues` de `/repos/{o}/{r}/milestones/{n}` compte des **items** :
  issues **et** pull requests (192) ;
- `gh issue list` ne rend que des **issues** (117).

Leur écart vaut donc **75 = le nombre de PR du milestone**. Il est structurel et permanent :
il ne tombera jamais à zéro, il grandira à chaque PR jalonnée, et une perte réelle de tickets
s'y noierait sans changer sa nature. **Pris comme signal de complétude, cet écart a zéro
pouvoir de détection** (classe mika#2205 : deux populations sous un même nom ne se
soustraient pas).

Corollaire piège, à nommer parce qu'il invite à l'erreur inverse : aujourd'hui
`open_issues = 14` **égale** le compte d'issues ouvertes — non par invariant, mais parce
que le milestone compte 0 PR ouverte à cet instant. Une seule PR ouverte jalonnée #33 brise
l'égalité. Ne rien bâtir dessus.

**Ce que le plan livre à la place**, et qui satisfait la lettre d'AC3 sans en hériter la
prémisse : le rapport dit les deux comptes, **avec leur grandeur nommée** (items vs tickets),
et il dit **le verdict de complétude qui, lui, est exact par construction** — la page n'était
pas pleine, donc la lecture est complète. Le lecteur du rapport obtient ainsi la réponse à
« ai-je tout lu ? » au lieu d'un écart qu'il devrait savoir interpréter.

Refus argumenté, pour que personne ne le rouvre en croyant combler un oubli : un appel
GraphQL supplémentaire (`milestone { issues { totalCount } pullRequests { totalCount } }`)
rendrait la décomposition exacte 117/75 et permettrait une comparaison de même grandeur.
Il est refusé pour trois raisons. (a) Il est **redondant** : `len < LIMIT` prouve déjà la
complétude, exactement, sans second appel. (b) Il coûte **+1 appel `gh` par cycle**, sur un
cycle dont ce vol s'engage à ne pas changer le compte d'appels. (c) Il n'est **pas mesurable
depuis un pilote** : `gh api graphql` est refusé par la permission-policy de ce bac à sable
(`no matching policy rule -- denied by default`, mesuré deux fois le 2026-10-29), donc
l'implémenteur se heurterait au même mur que ce grooming et ne pourrait pas attester son
propre livrable.

### 2.3 Pagination ou borne nommée — la décision d'AC1

AC1 laisse le choix et demande la justification. **Borne nommée**, pas pagination par curseur.

- `gh issue list --limit N` **pagine déjà en interne** (§ 1.3) : écrire une boucle de curseurs
  par-dessus réimplémenterait, en appels `gh api graphql` refusés par la policy, ce que le
  CLI fait en un sous-processus.
- La borne retenue est **1000**, soit **8,5 ×** la taille actuelle du milestone (117). Au
  rythme observé du #33, c'est plusieurs années de marge.
- Et parce qu'une marge n'est pas une preuve, la borne est **dite quand elle est atteinte** :
  c'est le reste de ce plan. Une page pleine ne passe jamais pour un milestone complet.

---

## 3. AC1 — la borne nommée, le prédicat, et l'état qui le porte

### 3.1 `types.rs` — un seul champ, trois nombres qui voyagent ensemble

`MilestoneState` gagne **un** champ, pas trois :

```rust
/// Ce que la lecture des tickets du milestone (appel 2) a rendu, et sous quelle borne.
///
/// Les trois nombres voyagent ensemble parce que le rapport ne peut dire aucun des
/// trois sans les deux autres : « 117 lus » ne dit rien sans la borne, et la borne
/// qu'il faut épeler est celle que **cette** lecture a demandée — un blob écrit sous
/// une autre borne doit rapporter la sienne, pas celle du binaire qui le relit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneReadAudit {
    /// Combien de tickets l'appel 2 a rendus.
    pub returned: usize,
    /// La borne que CET appel a demandée (`--limit`).
    pub limit: usize,
    /// `open_issues + closed_issues` de l'appel 1. Compte des **items** :
    /// issues ET pull requests. Ce n'est pas la même population que `returned`
    /// (mesuré le 2026-10-29 : 192 items = 117 issues + 75 PR), donc l'écart
    /// entre les deux est structurel et n'a aucun pouvoir de détection
    /// (mika#2205). Il est rapporté parce qu'AC3 le demande, avec sa grandeur
    /// nommée ; le signal de complétude est `truncated()`.
    pub api_item_total: u64,
}

impl MilestoneReadAudit {
    /// `true` quand la page était pleine — donc quand des tickets du milestone
    /// peuvent manquer. `false` est une **preuve** de complétude : une page non
    /// pleine est une page entière.
    ///
    /// Dérivé, jamais stocké : un booléen figé à côté des deux nombres dont il
    /// découle peut dériver d'eux ; une méthode ne peut pas.
    pub fn truncated(&self) -> bool {
        self.returned >= self.limit
    }
}
```

et sur `MilestoneState` :

```rust
/// `None` = la question n'a pas été posée (blob écrit par un binaire antérieur
/// à mika#2969, ou état composé hors de `read_with_runner`). Jamais un défaut
/// qui affirmerait une complétude non mesurée (mika#2331, mika#2340).
#[serde(default)]
pub milestone_read_audit: Option<MilestoneReadAudit>,
```

`#[serde(default)]` : les blobs déjà écrits dans le répertoire de rapports se relisent.

### 3.2 `reader.rs` — la borne, l'appel, le compositeur

```rust
/// Borne de la lecture des tickets du milestone (appel 2).
///
/// 1000, soit ≈ 8,5 × la taille du #33 mesurée le 2026-10-29 (117 tickets).
/// `gh issue list --limit N` pagine en interne : une seule invocation de
/// sous-processus quelle que soit la borne, `ceil(rendus / 100)` pages GraphQL
/// sous-jacentes. Mesuré : 2,5 s à 100, 2,6–3,0 s à 1000, un appel `gh` dans
/// les deux cas (mika#2969 § 1.3). `pub` parce que le rapporteur l'épelle.
pub const MILESTONE_READ_LIMIT: usize = 1000;
```

L'appel 2 passe `&MILESTONE_READ_LIMIT.to_string()` au lieu du littéral `"100"`.

Un compositeur pur, voisin de `compose_ready_pool` et `compose_outside_candidates` :

```rust
/// Ce que l'appel 2 a rendu, confronté à la borne qu'il a demandée et au compte
/// d'items de l'appel 1.
///
/// Pur et bornable : `limit` est un paramètre, donc le prédicat de page pleine
/// se teste sur trois tickets au lieu de mille.
fn compose_read_audit(
    milestone_json: &str,
    issues_json: &str,
    limit: usize,
) -> Result<MilestoneReadAudit>
```

Elle lit `open_issues` et `closed_issues` sur le blob du milestone (seule épellation de
ces deux chemins JSON dans la base), `len()` sur la liste d'issues, et porte `limit` tel
qu'il lui est donné.

**Pourquoi un compositeur séparé et non un cinquième argument à `compose_from_gh_outputs`** :
cette fonction a quatre arguments et un commentaire qui défend cette signature ; elle est
appelée hors du reader (eval `manager_loop_resistance`). La borne est une propriété de
*l'appel*, pas de la composition. Le câblage suit donc le patron déjà en place pour
`ready_pool` et `outside_candidates` — composition d'abord, champ posé ensuite dans
`read_with_runner` :

```rust
state.milestone_read_audit =
    Some(compose_read_audit(&milestone_json, &issues_json, MILESTONE_READ_LIMIT)?);
```

Conséquence assumée et conforme au § 3.1 : un état composé par `compose_from_gh_outputs`
seule (l'eval) porte `None` — « la question n'a pas été posée » — et non un `Some` qui
affirmerait une mesure qui n'a pas eu lieu.

### 3.3 Ce qui n'est **pas** fait sur page pleine, et pourquoi

`outside_read_truncated == Some(true)` **vide** `outside_candidates` (« refuser plutôt que
mal répondre »). La lecture du milestone **ne fait pas** la même chose, et l'asymétrie est
délibérée :

- la lecture « outside » produit un **classement sur un ordre global** — un classement sur
  une page tronquée est faux, donc inutilisable, donc jeté ;
- la lecture du milestone produit une **énumération** — une énumération partielle reste
  informative (progression, tickets en vol, prochaine action), et vider `sub_issues` sur
  page pleine effacerait le rapport entier au lieu de le qualifier.

Donc : **on rapporte, on ne refuse pas.** La recommandation reste calculée sur l'ensemble
partiel, et le rapport dit que la lecture était tronquée. Changer la recommandation sur
page pleine serait un changement de comportement hors du périmètre de ce ticket.

---

## 4. AC3 — ce que le rapport dit

Site : `reporter.rs`, section `### État`, immédiatement après la ligne
`- Progress: {completed}/{total} sub-issues complete ({pct}% done)` (l. 189-193) — c'est la
ligne que la troncature falsifie, donc la ligne à côté de laquelle le verdict se lit.

Trois états, trois sorties, et **aucune ligne** dans le troisième :

**(a) Audit présent, page non pleine** — une ligne :

```
- Lecture du milestone : 117 tickets lus, borne 1000 — complète (API du milestone : 192 items, PR comprises — autre population, mika#2969)
```

**(b) Audit présent, page pleine** — une ligne, sur le patron de `outside_read_truncated` :

```
- Lecture du milestone TRONQUÉE à 1000 tickets — des tickets du #33 sont hors de ce rapport, de sa progression et de son bassin (API du milestone : 1200 items, PR comprises)
```

**(c) `milestone_read_audit == None`** — rien. Le blob a été écrit par un binaire antérieur,
ou composé hors de `read_with_runner` : une ligne affirmant une complétude non mesurée serait
exactement l'instrument qui ment (mika#2331, mika#2340).

**Une seule section, pas trois.** `outside_read_truncated` apparaît dans deux sections parce
que la lecture « outside » alimente deux sections. La lecture du milestone alimente **tout**
le rapport ; la poser trois fois serait trois épellations d'un seul fait. Elle va dans la
section dont elle falsifie la première ligne.

---

## 5. AC2 — les tests

Le harnais existe : `RecordingGhRunner` (reader.rs, l. ≈ 2100-2550) discrimine les sept
appels, `build_recorder()` sème des JSON minimaux, `assert_adjacent_pair(call, flag, value)`
asserte qu'un drapeau paraît une fois et qu'il est suivi de sa valeur.

**T1 — le prédicat de page pleine, sur trois tickets** (`compose_read_audit`, unitaire).
Trois cas : `returned < limit` ⇒ `truncated() == false` ; `returned == limit` ⇒ `true` ;
`api_item_total` est bien `open_issues + closed_issues`. Le `limit` paramétrable est ce qui
rend ce test lisible sans fixture de mille éléments.

**T2 — AC2, bout en bout, un milestone de plus de 100 tickets.** `build_recorder()` sert
**117** issues synthétiques (générées en boucle), dont une **ouverte au-delà du centième**
— l'analogue de #1721. Le `outside_json` du même recorder **contient ce même numéro**
(c'est le cas réel : la lecture « outside » voit toutes les issues ouvertes du dépôt), sinon
l'assertion « pas candidat outside » serait vide de sens. On asserte :

- ce ticket est dans `state.sub_issues` ;
- il n'est **pas** dans `state.outside_candidates` ;
- `audit.truncated() == false`, `audit.returned == 117`, `audit.limit == 1000` ;
- le rapport ne porte **aucune** ligne de troncature.

**T3 — contrôle négatif, moins de 100 tickets.** Sept issues : lecture identique à
aujourd'hui, `truncated() == false`, aucune ligne de troncature au rapport.

**T4 — la page pleine se dit.** État construit à la main avec
`MilestoneReadAudit { returned: 1000, limit: 1000, api_item_total: 1200 }` : le rapport
porte la ligne (b), **et** `sub_issues` n'est pas vidé (le § 3.3 épinglé).

**T5 — l'audit absent ne dit rien.** `milestone_read_audit: None` ⇒ aucune des deux lignes.
C'est le contrôle qui sépare « la ligne est conditionnelle » de « la ligne est
inconditionnelle avec un défaut ».

**T6 — mise à jour du test qui épingle le défaut.**
`reader_issue_list_uses_state_all_and_limit_100` (l. 2206) asserte aujourd'hui
`assert_adjacent_pair(issue_call, "--limit", "100")` (l. 2225). Il devient
`"--limit", "1000"` et prend un nom qui dit la borne. Le littéral est **délibéré** (patron
maison : les assertions voisines sur `OUTSIDE_READ_LIMIT` épellent `"100"` en clair) — une
assertion contre la constante serait tautologique. La propriété non tautologique est ajoutée
à côté : **le nombre que le rapport épelle est celui que l'appel a demandé**
(`audit.limit == ` la valeur assertée sur la ligne de commande).

Trois assertions `--limit 100` restent **intactes** — l. 2252 (`pr list`), l. 2470 (ready),
l. 2547 (outside) : trois bornes distinctes, dont une seule change. Le doc-comment de
l. 2034, qui épelle `"--limit", "100"` pour l'appel 2, est corrigé.

`mika2924_…_sans_appel_de_plus` asserte le **compte** d'appels `gh` : il doit rester vert
sans modification — c'est la garde du § 1.3.

---

## 6. Effets de bord, nommés avant d'être subis

1. **`progress.total` passe de 100 à 117**, `completed` à 103 : le pourcentage rapporté
   change d'un coup, une fois. C'est la correction, pas une régression.
2. **`state_digest` change une fois** (`cadence.rs`, l. ≈ 439 — il hache
   `number:state:pr_state:ci_state:blockers` sur `sub_issues`). Un événement « changé » au
   premier cycle après déploiement, puis stable. Corollaire : à partir de là, un changement
   d'état d'un ancien ticket du milestone bouge enfin le digest.
3. **#1721 et #1727 cessent d'être lus « hors milestone »** et deviennent candidats du
   milestone, avec un motif juste. Ils peuvent donc être admis au bassin ou nommés dans la
   relève — c'est le but du ticket (mika#2699, mika#2958).
4. **Le rapport grossit de ≈ 17 lignes `Completed:`** (`write_completed`, l. 585-623,
   énumère tous les sous-tickets fermés, **déjà sans borne**). Ce n'est pas une classe
   nouvelle : la lecture tronquée masquait une absence de borne préexistante. Le chemin de
   délivrance n'a pas de plafond de taille. Si cette énumération doit être bornée, c'est un
   ticket à elle, pas une dépendance de celui-ci.
5. **Le budget d'appels reste borné.** Deux tickets ouverts de plus peuvent entraîner deux
   lectures de taille de plan supplémentaires ; elles sont déjà plafonnées par
   `PLAN_SIZE_READ_MAX = 3 * POOL_MAX` (`pool.rs`, l. 538). Aucune classe d'appel nouvelle,
   aucun plafond nouveau.
6. **Essaimage littéral :** huit sites construisent `MilestoneState` en littéral et gagnent
   chacun **une** ligne (`milestone_read_audit: None`) — `assessor.rs:479`,
   `reporter.rs:936`, `evaluator.rs:1327`, `cadence.rs:1288`, `spawn.rs:1948`,
   `reader.rs:1146`, `db/tests/vols_termines_du_manager.rs:878`,
   `tests/manager_delivery_observability_2267.rs:161`. C'est exactement la raison pour
   laquelle l'audit est **un** champ et non trois.

---

## 7. Ordre de commit et vérification

1. `types.rs` — `MilestoneReadAudit`, `truncated()`, le champ sur `MilestoneState`, plus
   les huit `None` de l'essaimage. Compile seul.
2. `reader.rs` — `MILESTONE_READ_LIMIT`, l'appel 2, `compose_read_audit`, le câblage dans
   `read_with_runner`, le doc-comment l. 2034. Plus T1 et T6.
3. `reporter.rs` — les deux lignes conditionnelles. Plus T4 et T5.
4. T2 et T3 (bout en bout, dans `reader.rs`).
5. La garde auto-nettoyante du § Fire-Disposition.
6. `docs/agent-internals/milestone-manager-phase-1.md` — la borne, le prédicat, la
   correction de la prémisse d'AC3 et la lecture du rapport. Hors compte de lignes.

Vérification, dans cet ordre : `cargo test -p mika-agent milestone_manager`, puis
`cargo test`, `cargo clippy`, `cargo fmt --check`, puis
`scripts/verify-pipeline.sh origin/main`.

---

## 8. Sonde après déploiement (AC4)

Non exécutable par ce vol — elle exige `make deploy` puis un cycle du manager vivant, et
« les essais à la main sur le manager vivant » sont hors périmètre. Ce que l'implémenteur
écrit dans le corps de la PR, et ce que l'opérateur lit au premier rapport du sink :

1. **Nom du rapport** : `<slug>-<timestamp>.md` sous le répertoire de rapports hors-ligne
   (`cadence.rs`, l. ≈ 1160).
2. **#1721 et #1727** paraissent comme tickets du #33 — et **ne portent plus** la mention
   « hors milestone ».
3. **Les deux comptes sont écrits**, tels que le rapport les épelle : le compte lu (attendu
   ≈ 117, selon la taille du milestone au jour du déploiement) et le compte d'items de l'API
   (attendu ≈ 192). **Ils ne seront pas égaux**, et c'est la réponse correcte : leur écart
   est le nombre de PR du milestone (§ 2.2). Le signal de complétude à lire est la mention
   **« complète »** sur la ligne de lecture.
4. **Absence de la ligne de lecture** ⇒ le binaire servi est antérieur à ce vol
   (mika#2340), jamais « la lecture va bien ».

---

## Definition of Done

- [ ] `MILESTONE_READ_LIMIT: usize = 1000`, `pub`, doc-commentée avec la mesure de coût.
- [ ] L'appel 2 ne porte plus de littéral de borne.
- [ ] `MilestoneReadAudit { returned, limit, api_item_total }` + `truncated()` dérivé, sur
      `MilestoneState` en `Option` + `#[serde(default)]`.
- [ ] `compose_read_audit` pure, borne en paramètre, câblée dans `read_with_runner`.
- [ ] Le rapport porte la ligne de lecture (complète / tronquée) et **rien** quand l'audit
      est absent.
- [ ] T1 à T6 verts ; les trois autres assertions `--limit 100` et
      `mika2924_…_sans_appel_de_plus` inchangés et verts.
- [ ] Le compte d'appels `gh` par cycle reste 7.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] `docs/agent-internals/milestone-manager-phase-1.md` à jour.
- [ ] Le corps de la PR porte la sonde du § 8 et la correction de la prémisse d'AC3.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2969.

- **AC1.** Le reader lit **tout** le milestone : par pagination, ou par une borne nommée,
  assez haute, et dite quand elle est atteinte. Le plan choisit et justifie, coût en appels
  `gh` par cycle compris, mesuré comme mika#2924 l'a fait pour son champ. Si une borne reste,
  une page pleine se dit dans le rapport, sur le patron de `outside_read_truncated`, et ne
  passe jamais pour un milestone complet.
- **AC2.** Test dans le même appel, avec un `GhRunner` injecté qui sert un milestone de plus
  de 100 tickets. Un ticket ouvert au-delà du centième est lu comme sous-issue du milestone,
  et non comme candidat « outside ». Contrôle négatif : un milestone de moins de 100 tickets
  se lit comme aujourd'hui, et le rapport n'y signale aucune troncature.
- **AC3.** Le rapport du sink dit le compte lu à côté du compte de l'API du milestone
  (`open_issues + closed_issues`, de l'appel 1 qui est déjà émis). Un écart entre les deux
  se voit au rapport.
- **AC4.** Sonde après déploiement : au premier rapport du sink, #1721 et #1727 sont lus
  comme tickets du #33, et le compte lu égale le compte de l'API. Les deux comptes sont
  écrits ici, avec le nom du rapport.

**Note de traitement, portée au grooming.** AC3 et AC4 demandent l'égalité de deux nombres
qui comptent deux populations différentes (§ 2.2, mesuré : 192 items = 117 issues + 75 PR).
Cette égalité **n'est pas atteignable** et ne doit pas l'être. Le plan livre la lettre des
deux critères — les deux comptes écrits côte à côte, dans le rapport et dans la sonde — en
nommant leur grandeur, et livre **en plus** le signal de complétude qu'ils visaient, exact
par construction : la page n'était pas pleine. L'implémenteur ne doit pas chercher à faire
converger les deux nombres.

## Fire-Disposition

Ce plan livre des détecteurs : T1 à T6, et la garde auto-nettoyante ci-dessous. Option
**(a) — exception nommée en allowlist**, livrée **vide**.

```rust
/// Milestones pour lesquels l'implication « page non pleine ⇒ lecture complète »
/// est connue fausse.
///
/// **Livrée vide, et c'est une mesure, pas un optimisme** : le prédicat de page
/// pleine ne peut rougir qu'à partir de 1000 tickets rendus, et le plus grand
/// milestone du dépôt en compte 117 (mesuré le 2026-10-29 sous `7df2c1e1c`,
/// mika#2969 § 1.1). Il n'existe donc aucune violation existante à excepter.
///
/// Si une entrée doit être ajoutée un jour, elle nomme le milestone, référence
/// son ticket de suivi, et l'assertion ci-dessous la fait rougir dès qu'elle
/// devient stale.
const MILESTONE_READ_COMPLETENESS_EXCEPTIONS: &[&str] = &[];
```

Assertion auto-nettoyante, livrée avec :

```rust
#[test]
fn mika2969_lallowlist_de_completude_est_vide_et_le_reste_tant_quelle_na_pas_dentree() {
    // Une exception qui n'excepte plus rien est du bruit qui survit à son motif.
    // Cette assertion rougit dans les deux sens : à l'ajout d'une entrée (qui doit
    // alors citer son ticket) et au retrait de son motif.
    assert!(
        MILESTONE_READ_COMPLETENESS_EXCEPTIONS.is_empty(),
        "une exception a été ajoutée : elle doit nommer son milestone et son ticket \
         de suivi, et ce test doit être amendé pour l'attester — pas élargi"
    );
}
```

Les détecteurs T1 à T6 sont livrés **armés** : aucun ne porte `#[ignore]`, et ils sont verts
par construction sur le code de ce vol (T6 est précisément l'assertion qui épingle le défaut
et qui passe du littéral `100` au littéral `1000`). Aucune halte-et-remontée n'est nécessaire.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/types.rs` (`MilestoneReadAudit`, `truncated()`, champ + doctrine) | 45 |
| `crates/mika-agent/src/milestone_manager/reader.rs` (borne, appel 2, `compose_read_audit`, câblage) | 55 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (deux lignes conditionnelles) | 30 |
| essaimage littéral (8 sites × 1 ligne) | 8 |
| tests `reader.rs` (T1, T2, T3, T6) | 160 |
| tests `reporter.rs` (T4, T5) | 70 |
| garde auto-nettoyante (Fire-Disposition) | 20 |

Total estimé : 388 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 388 passe sous la borne de 429 et très loin sous le seuil
par PR de 1000. Trois propriétés tiennent l'estimation :

- **Un champ, pas trois.** Grouper `returned`/`limit`/`api_item_total` dans un `struct`
  divise l'essaimage littéral par trois (8 lignes au lieu de 24) — c'est le poste qui fait
  déraper ce genre de vol.
- **Pas de fixture de mille éléments.** `compose_read_audit` prend la borne en paramètre,
  donc le prédicat se teste sur trois tickets (T1) et la ligne de troncature sur un état
  construit à la main (T4). La seule fixture volumineuse est les 117 issues de T2, générées
  en boucle (≈ 10 lignes).
- **Aucun site de production nouveau.** Une constante, un appel modifié, une fonction pure,
  un champ, deux lignes de rapport. Le reste est du test, dans des modules qui existent et
  dont le harnais est déjà écrit.

## Hors périmètre

- **La lecture des PR du milestone** (`gh pr list --search milestone:<N>`), qui ne voit pas
  les PR de la boucle — mika#2968, et mika#2924 pour les tickets fermés. Ce vol ne touche
  pas l'appel 3.
- **Les essais à la main sur le manager vivant.** La sonde du § 8 est écrite, pas exécutée.
- **L'appel GraphQL de décomposition exacte issues/PR** — refusé, avec ses trois raisons au
  § 2.2.
- **Borner l'énumération `Completed:` du rapport** — absence de borne préexistante, classe
  distincte, ticket à elle (§ 6.4).
- **Changer la recommandation du bassin sur page pleine** — l'asymétrie est argumentée au
  § 3.3 ; la modifier serait un changement de comportement que ce ticket ne demande pas.

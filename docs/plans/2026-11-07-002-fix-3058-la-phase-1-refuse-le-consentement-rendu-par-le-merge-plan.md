# mika#3058 — la phase 1 refuse le consentement que le merge vient de rendre

- **Ticket :** senara-solutions/mika#3058
- **Base :** `931dc9d3f`
- **Branche :** `fix/3058/auto-pull-la-phase-1-repose-ready-sur-le`
- **Dépôt touché :** `mika` seul. Un seul vol.

---

## 0 — Le défaut, et le fil qui le tenait

mika#3036 a fait du `phase` un paramètre de `classify_promotion`, et lui a donné
**un seul lecteur** : le bras `BranchDeletedByMerge`. Ce bras dit aujourd'hui
(`crates/mika-agent/src/auto_pull.rs:1438-1461`) :

| phase | décision | ce que le commentaire affirme |
|---|---|---|
| `Phase0Feeder` | `Decline(ConsentReturnedByMerge)` | « le ticket ne porte pas `ready` » — vrai |
| `Phase1Pull` | `Promote { "branch_deleted_by_merged_pr" }` | « le ticket porte **déjà** `ready` » — **faux** |
| `Phase2StuckReady` | `Promote { "branch_deleted_by_merged_pr" }` | « le ticket porte déjà `ready` » — vrai |

La ligne 1446 regroupe les phases 1 et 2 sous une prémisse qui ne vaut que pour
la phase 2. La phase 1 (`phase1_promote_groomed`, l. 5411) ne lit que des
tickets **sans** `ready` — son premier filtre de sélection est
`.filter(|i| !i.labels.iter().any(|l| l.name == "ready"))`
(`select_best_candidate_recording`, l. 2247) — et sa dernière action est de le
**poser** : `apply_ready_label(…, "phase1_idle_pull", …)` (l. 5526-5531). Elle
est donc, mot pour mot, dans la situation de la phase 0 : le merge a retiré
`ready` (mika#3022), et reposer le label c'est se donner le consentement qu'on
vient de rendre.

La mesure du ticket le montre sans ambiguïté : le 06/11 à 06:40Z, la phase 0
refuse à 06:40:03Z et la phase 1 promeut à 06:40:04Z, **sur la même mesure**. Le
vol 2 de #3047 est parti sans mot humain. Douze promotions de cette forme depuis
le 21/10, quatre depuis le 04/11.

### Ce qui distingue vraiment la phase 2

La phase 2 balaye les tickets qui portent `ready` et qu'aucun dispatch n'a pris.
Son consentement n'est pas celui que le merge a rendu : c'en est un autre, encore
posé, et mika#3047 lui a ajouté la relecture du label juste avant le dispatch
(`gh_reread_issue_at_dispatch`). Elle garde donc `Promote` — c'est par elle que
part le vol suivant d'un ticket à plusieurs vols, une fois que l'humain a reposé
`ready`.

---

## 1 — Les points de conception

### D1 — Le bras de phase reste **un seul** `match phase`

Le scan S1 de mika#3036 (`auto_pull.rs:7411-7419`) assert que la production du
fichier ne contient qu'**un** `match phase {`. Ce vol ne fait que déplacer
`Phase1Pull` d'un côté du bras à l'autre :

```rust
ExclusionPhase::Phase0Feeder | ExclusionPhase::Phase1Pull => {
    PromotionGate::Decline(DeclineReason::ConsentReturnedByMerge { pr: *pr })
}
ExclusionPhase::Phase2StuckReady => PromotionGate::Promote {
    detail: "branch_deleted_by_merged_pr",
},
```

Le `match` reste exhaustif, sans bras `_` : une quatrième phase ne compilera pas
sans qu'on dise ici ce qu'elle fait. Le commentaire de la l. 1446 est réécrit
pour dire ce que chaque groupe tient réellement, et le fil mika#3047 reste
attaché au bras de la phase 2, à qui il appartient.

### D2 — Un refus de phase 1 doit laisser la phase avancer : la boucle

C'est AC3, et c'est la part de conception du vol. Le commentaire actuel de la
l. 5497-5503 décrit le mécanisme auto-guérissant sur lequel la phase 1 repose :

> *Phase 1 picks exactly one candidate, so a refusal here costs the whole tick
> […]. The refused ticket now carries `REFUSAL_LABEL`, `is_feeder_excluded`
> drops it, and the next tick reaches the next-best candidate — one idle tick,
> self-healing.*

Ce mécanisme marche pour `Refuse` et **seulement** pour lui : il est porté par
la pose de `operator-gated` dans `refuse_promotion`. Or AC4 interdit toute
écriture à la forge sur un `Decline`, et `authorizes_body_rewrite` le tient déjà
(mika#3036 AC3). Donc rien ne marque le ticket décliné, rien ne l'exclut, il
reste en tête de la sélection — et la phase 1 refuserait ce même ticket à chaque
tick, sans plus rien promouvoir. **Le correctif d'AC1 fabriquerait la panne
d'AC3.**

Le ticket laisse le choix entre « passer au candidat suivant dans le même tick »
et « exclure le ticket de la sélection de phase 1 ». L'exclusion à la sélection
est hors d'atteinte : `groomed_candidate_exclusion` est une fonction **pure** sur
les labels et les PR ouvertes, et la mesure `BranchDeletedByMerge` demande deux
appels forge par ticket (`gh_compare_branch` puis `gh_merged_pr_for_branch`). On
retient donc **le candidat suivant dans le même tick**, borné.

Trois conséquences structurelles :

1. `select_best_candidate_recording` devient
   `select_candidates_ranked_recording` et rend la liste **ordonnée** au lieu de
   son seul maximum. Le wrapper de test `select_best_candidate` prend son
   `.next()`, donc ses treize appels de test gardent leur garde inchangée.
2. L'ordre doit être **celui d'avant**, ex æquo compris. `max_by` rend le
   **dernier** élément maximal ; un `sort_by` stable décroissant rendrait le
   premier. On reproduit `max_by` exactement en triant sur
   `cmp_rank(b, a).then(ib.cmp(ia))` après `enumerate()` — l'index décroissant
   en dernier tie-break. Un test nomme ce point (§ 3, T4), parce qu'un
   changement d'ordre silencieux sur deux tickets de même priorité et même
   `updated_at` serait un effet de bord que personne ne verrait.
3. Le verdict de la porte doit **distinguer** `Decline` de `Refuse` au site
   d'appel : seul un `Decline` fait passer au candidat suivant. Un `Refuse` a
   posé `operator-gated`, son auto-guérison existe, et on ne change pas son
   comportement — il abandonne le tick, comme aujourd'hui.

### D3 — `GateOutcome` : la porte rend trois issues, pas un booléen

`promotion_gate_allows` rend `bool` aujourd'hui. Elle rendra :

```rust
enum GateOutcome { Allowed, Refused, Declined }
impl GateOutcome { fn allows(&self) -> bool { matches!(self, Self::Allowed) } }
```

Les phases 0 et 2 lisent `!outcome.allows()` là où elles lisaient `!allows` :
leur comportement est inchangé à l'identique. Seule la phase 1 distingue les
trois. `Declined` ne porte pas de donnée — la raison est déjà au log, à l'audit
et au ledger sous son slug ; la dupliquer dans le type ferait un second lecteur
de la même sonde, ce que mika#3036 S1 existe pour empêcher.

### D4 — Le budget de candidats du tick

La boucle essaye au plus `PHASE1_DECLINE_BUDGET = 3` candidats par tick. La
porte coûte un à deux appels forge par candidat ; sans borne, un tick sur une
file de vingt tickets fraîchement mergés ferait quarante appels. Trois, parce que
la mesure du ticket montre au plus deux tickets simultanément dans cet état
(#3047 et #3050 le 06/11) et que la borne doit laisser une marge sans devenir un
balayage. Au-delà du budget, la phase abandonne le tick — et le tick suivant
reprend, puisque aucun état n'est persisté.

### D5 — Le circuit-breaker reste un abandon de tick

Le circuit-breaker (l. 5447-5484) garde son `return None` sur le premier
candidat : il **ne** passe pas au suivant. C'est le comportement d'aujourd'hui,
hors mesure `BranchDeletedByMerge` et donc hors vol (§ *Hors vol* du ticket).
Seul le `Declined` fait avancer la boucle. Le plan le dit parce qu'un lecteur du
diff verra une boucle et se demandera pourquoi un de ses `return` n'itère pas.

---

## 2 — Le ledger, et pourquoi son nom est neuf

AC4 : « le refus est consigné au ledger et à l'audit sous le même slug
`consent_returned_by_merge`, avec la phase ».

L'audit le porte déjà : `staleness_audit_json` écrit `"reason":
"consent_returned_by_merge"` et `"outcome": "decline"`, et le `warn!` de
`promotion_gate_allows` (l. 4178-4186) nomme la phase via `phase.gate_phase()`.
C'est le test existant mika#3036 qui le garde.

Le **ledger**, lui, compte aujourd'hui un déclin sous
`FILTER_PROMOTION_GATE = "promotion_gate_refused"`, le même nom qu'un refus. Deux
populations sous un seul nom : c'est exactement mika#2205, et c'est ce que le
commentaire de `DeclineReason::slug()` interdit pour le couple
`branch_absent_on_origin` / `consent_returned_by_merge`. On ajoute donc :

```rust
/// mika#3058 — un déclin de porte, distinct du refus. Le texte est celui de
/// `DeclineReason::slug()`, et un test l'y compare.
const FILTER_CONSENT_RETURNED_BY_MERGE: &str = "consent_returned_by_merge";
```

Deux sites de `ledger.record` le portent — phase 0 et phase 1. **Pas trois** : la
phase 2 ne décline jamais, par AC1, et c'est précisément cette asymétrie que le
recensement de T5 garde.

Le scan S1 existant (l. 7422-7437) assert que **trois** `ledger.record` portent
`FILTER_PROMOTION_GATE`. Les trois restent : ils traitent désormais le seul
`Refused`. Son message d'assertion, qui dit aujourd'hui « déclins compris », est
corrigé — les déclins ont leur propre compte, et c'est le message qui doit le
dire, sinon il décrit un câblage qui n'existe plus.

---

## 3 — Les tests

Tous dans `crates/mika-agent/src/auto_pull.rs`, module `tests`, à la suite du
bloc mika#3036 (vers l. 14216-14390).

**T1 (AC1, AC2) — `mika3058_les_trois_phases_dans_un_seul_appel`.** Mutation du
test existant `mika3036_la_phase_decide_de_la_mesure_de_merge`, dont le contrôle
négatif bouclait sur `[Phase1Pull, Phase2StuckReady]` et affirmait `Promote` pour
les deux. Une **seule** mesure `BranchDeletedByMerge { pr: 3032 }`, trois
appels à `classify_promotion` :

- `Phase0Feeder` → `Decline(ConsentReturnedByMerge { pr: 3032 })` ;
- `Phase1Pull` → `Decline(ConsentReturnedByMerge { pr: 3032 })`, avec le message
  qui nomme la mesure du 06/11 à 06:40Z ;
- `Phase2StuckReady` → `Promote { detail: "branch_deleted_by_merged_pr" }`, le
  contrôle négatif : mika#2837 et mika#3047 intacts.

Le test reste **un seul**, parce que deux tests séparés peuvent dériver l'un sans
l'autre et laisser « la phase 1 refuse » vrai sans que « la phase 2 promeut
toujours » le soit encore — la raison déjà écrite dans le docstring du test de
mika#3036, qu'on conserve. Les assertions d'audit d'AC4 (slug, `issue` et
`merged_pr` non nuls) restent, et la ligne d'audit relue est désormais celle de
la phase 1.

**T2 (AC3) — `mika3058_un_declin_en_tete_ne_fige_pas_la_phase`.** Le test de la
boucle, sans réseau : `phase1_first_promotable` reçoit une porte simulée en
fermeture. Trois candidats ordonnés — le ticket décliné en tête (priorité
`p1-important`, `updated_at` le plus récent), un candidat sain derrière, un
troisième témoin. La porte rend `Declined` sur le premier et `Allowed` sur les
suivants. Assertions :

- le candidat **promu est le sain**, pas le troisième — la boucle s'arrête au
  premier `Allowed`, elle ne balaye pas ;
- le ledger porte `(Phase1Pull, <n_decliné>, "consent_returned_by_merge")` et
  **rien** sur le candidat promu ;
- la porte a été appelée **deux** fois — le compteur de la fermeture le dit. Un
  test qui n'assert que le résultat passerait sur une implémentation qui
  interroge les trois.

**T3 (AC3, D4) — `mika3058_le_budget_de_declins_borne_le_tick`.** Quatre
candidats, la porte rend `Declined` sur les trois premiers et `Allowed` sur le
quatrième. La boucle rend `None` et la porte a été appelée exactement
`PHASE1_DECLINE_BUDGET` fois. Le tick suivant repartira de la tête : la boucle ne
persiste rien, et c'est la lecture de « au plus tard au tick suivant » d'AC3.

**T4 (D2) — `mika3058_lordre_des_candidats_est_celui_de_max_by`.** Le
non-régression du refactor de sélection. Trois cas : priorité différente ;
priorité égale et `updated_at` différent ; **ex æquo parfait** (même priorité,
même `updated_at`). Dans les trois, la tête de
`select_candidates_ranked_recording` est le ticket que l'ancien `max_by` rendait
— pour l'ex æquo, le **dernier** de la liste d'entrée. Le ledger du tri porte les
mêmes entrées qu'avant : un ticket exclu l'est une seule fois, qu'on prenne le
maximum ou la liste.

**T5 (AC4) — `mika3058_le_recensement_du_declin_au_ledger`.** Dans le test de
scan existant, à côté de S1 :

- `FILTER_CONSENT_RETURNED_BY_MERGE == DeclineReason::ConsentReturnedByMerge { pr: 0 }.slug()`
  — la constante et le slug ne peuvent pas dériver ;
- le compte de `ledger.record(` portant `FILTER_CONSENT_RETURNED_BY_MERGE` dans
  la production du fichier vaut **2**, et le message nomme pourquoi pas 3 : la
  phase 2 ne décline pas, par AC1 ;
- le compte de `FILTER_PROMOTION_GATE` reste **3**, assertion existante dont
  seul le message change.

**T6 (AC4) — non-régression, déjà en place.**
`mika3036_un_declin_ne_touche_pas_le_corps_du_ticket` passe déjà
`ExclusionPhase::Phase1Pull` à son témoin promouvant (l. 14292). Ce témoin
utilise une décision `Promote` construite à la main, pas la phase : il reste
valide tel quel et garde qu'un `Decline` n'écrit rien à la forge, sous le jeton de
l'App ou un autre. On ne le touche pas, et le plan le note pour qu'un lecteur ne
le croie pas oublié.

---

## 4 — Fire-Disposition

Ce plan livre des détecteurs : les tests T1 à T5, et parmi eux deux **scans
structurels** sur le texte de la production (T5). Option **(a) exception nommée
en allowlist**, dans la forme déjà en place pour mika#3036 S1 :

```rust
// Le type `[&str; 0]` fait mieux que l'assertion : une entrée ajoutée sans
// élargir le type ne compile pas, et l'élargir est le geste visible au `grep`.
const T3058_DECLINE_LEDGER_ALLOWLIST: [&str; 0] = [];
assert!(
    T3058_DECLINE_LEDGER_ALLOWLIST.is_empty(),
    "mika#3058 — table d'exceptions du recensement de déclin au ledger : doit \
     rester vide ; toute entrée nomme sa donnée précise et son ticket de suivi"
);
```

**Zéro entrée au départ, et c'est mesuré, pas espéré :** les deux sites de
`ledger.record(FILTER_CONSENT_RETURNED_BY_MERGE)` sont écrits par ce vol, donc le
recensement est exact dès son premier passage. L'allowlist est vide **et typée
`[&str; 0]`** : elle rougit à la compilation si un vol futur veut y glisser une
exception sans nommer sa donnée. C'est l'assertion auto-nettoyante que mika#1574
demande — une exception stale ne peut pas y dormir, puisqu'aucune ne peut y
entrer en silence.

Les tests T1 à T4 sont des tests de comportement livrés **armés** : ils passent
sur le code de ce vol et échouent sur `931dc9d3f`. Aucun `#[ignore]`, aucun
cadrage opérateur à demander.

---

## 5 — Definition of Done

- [ ] `classify_promotion` rend `Decline(ConsentReturnedByMerge)` en
      `Phase1Pull`, `Promote` en `Phase2StuckReady`, `match` exhaustif sans bras
      `_`, un seul `match phase {` dans le fichier.
- [ ] Le commentaire de la l. 1446 ne prétend plus que la phase 1 porte `ready`.
- [ ] `GateOutcome` remplace le `bool` de `promotion_gate_allows` ; phases 0 et 2
      inchangées par `!outcome.allows()`.
- [ ] La phase 1 passe au candidat suivant sur `Declined`, abandonne le tick sur
      `Refused` et sur le circuit-breaker, et s'arrête au budget.
- [ ] `FILTER_CONSENT_RETURNED_BY_MERGE` posé aux deux sites de déclin.
- [ ] Aucune écriture à la forge sur un déclin (garde existante mika#3036 AC3,
      plus T6).
- [ ] T1 à T5 écrits et verts ; les treize appelants de `select_best_candidate`
      verts sans modification.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `make verify-bundled-skills`,
      `bash scripts/verify-pipeline.sh origin/main` verts.
- [ ] Aucun fichier du plan de contrôle, aucun fichier sous CODEOWNERS touché.
- [ ] AC5 noté dans le corps de la PR comme sonde à relever 14 jours après le
      deploy, avec la commande de comptage.

---

## Acceptance criteria

- **AC1.** Dans `classify_promotion`, `BranchDeletedByMerge` en `Phase1Pull`
  rend `Decline(ConsentReturnedByMerge { pr })`, comme en `Phase0Feeder`.
  `Phase2StuckReady` garde `Promote` : le ticket y porte vraiment `ready`, et la
  phase 2 relit le label avant de dispatcher (mika#3047). Le `match` reste
  exhaustif, sans bras `_`. Le commentaire de la l. 1446 est corrigé.
- **AC2.** Un test garde les trois phases dans le même appel, sur la même
  mesure : refus en phase 0, refus en phase 1, promotion en phase 2.
- **AC3.** Un refus de phase 1 n'immobilise pas la phase. Elle ne choisit qu'un
  candidat par tick, et un ticket gardé ouvert après un merge `Refs` reste
  souvent en tête. Sans ce critère, la phase 1 refuserait ce même ticket à chaque
  tick, et ne promouvrait plus rien d'autre. Le vol choisit entre passer au
  candidat suivant dans le même tick ou exclure le ticket de la sélection de
  phase 1. Un test garde le fait : avec le ticket refusé en tête et un candidat
  sain derrière, le candidat sain est promu au plus tard au tick suivant.
- **AC4.** Aucune écriture à la forge sur un refus, comme le veut
  `authorizes_body_rewrite` (mika#3036 AC3). Le refus est consigné au ledger et à
  l'audit sous le même slug `consent_returned_by_merge`, avec la phase.
- **AC5.** Sonde sur les 14 jours qui suivent le deploy : on compte dans
  `server.log` les promotions `phase1_idle_pull` sur
  `branch_deleted_by_merged_pr`, et on l'écrit ici. La cible est 0. On écrit à
  côté le nombre de refus `consent_returned_by_merge` en phase 1. Un compte nul
  sans aucun merge `Refs` s'écrit « aucun merge `Refs` sur la fenêtre ».

### Correspondance AC → livrable

| AC | livrable | garde |
|---|---|---|
| AC1 | bras `BranchDeletedByMerge` de `classify_promotion`, commentaire l. 1446 | T1 |
| AC2 | — (exigence de forme sur le test) | T1 |
| AC3 | `GateOutcome`, `select_candidates_ranked_recording`, `phase1_first_promotable`, budget | T2, T3, T4 |
| AC4 | `FILTER_CONSENT_RETURNED_BY_MERGE` aux deux sites ; audit déjà en place | T5, T6 |
| AC5 | sonde post-deploy, pas de code | commentaire sur le ticket à J+14 |

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` — bras de phase + commentaire corrigé | 25 |
| `crates/mika-agent/src/auto_pull.rs` — `GateOutcome` + `promotion_gate_allows` | 45 |
| `crates/mika-agent/src/auto_pull.rs` — `FILTER_CONSENT_RETURNED_BY_MERGE` + sites phases 0 et 2 | 25 |
| `crates/mika-agent/src/auto_pull.rs` — `select_candidates_ranked_recording` + wrapper | 30 |
| `crates/mika-agent/src/auto_pull.rs` — `phase1_first_promotable` + budget + site phase 1 | 95 |
| tests (`auto_pull.rs`, module `tests` — T1 à T5) | 165 |

Total estimé : 385 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 385 < 429, et sous le seuil par PR de 1000. Le
vol est un seul fichier, et son cœur est un bras de `match` déjà écrit qui change
de côté. Le volume vient de deux endroits seulement : la boucle de phase 1 et ses
trois tests. Aucun schéma de DB, aucun fichier de config, aucun crate autre que
`mika-agent`, et `auto_pull.rs` pèse 628 186 octets sur `main` — sous le plafond
de 1 Mo, donc les éditions passent. Rien à découper : couper AC3 de AC1 livrerait
le correctif qui fige la phase, et couper AC1 de AC3 livrerait une boucle sans
raison d'être.

---

## Ordre de commit

1. `fix(3058)` — le bras de phase et son commentaire ; `GateOutcome` et les
   lectures `!outcome.allows()` des phases 0 et 2 ; `FILTER_CONSENT_…` et les
   deux sites de ledger. **Plancher :** compile, `cargo test -p mika-agent`
   vert, T1 et T5 écrits et verts. À ce point la phase 1 refuse — et se figerait,
   ce que le commit 2 lève avant toute mise en vol.
2. `fix(3058)` — `select_candidates_ranked_recording`, `phase1_first_promotable`,
   le budget, le site de phase 1. **Plancher :** T2, T3, T4 verts, les treize
   appelants de `select_best_candidate` verts sans modification.
3. Fin de pipeline : `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`, `cargo test` complet, `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`, puis la PR avec la ligne
   `Code review: delegated to mika-qa (mika#2694)` et la sonde AC5 nommée.

Les deux commits de code atterrissent dans la **même** PR : le premier seul
laisserait la phase 1 figée sur son candidat de tête, et ce n'est pas un état
mergeable.

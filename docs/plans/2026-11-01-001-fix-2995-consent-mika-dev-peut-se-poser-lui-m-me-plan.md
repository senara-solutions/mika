# mika#2995 — vol 2/2 : l'AC2 sous la voie B, le recensement des écrivains, et la visibilité de l'applicateur argv

> **Vol 2 de 2.** Le vol 1 (PR #2998, mergée et déployée à `8f7b22607`) a livré AC1,
> AC3 et AC4 par la **voie B** : le moteur perd la *capacité* d'écrire `ready`, la garde
> de mika#2772 ne lit pas la provenance. Plan du vol 1 :
> `docs/plans/2026-10-31-006-fix-2995-consent-mika-dev-peut-se-poser-lui-m-me-plan.md`.
>
> Ce vol livre l'**AC2 réécrite** (décision de Vincent, 2026-11-01), l'**AC2b** et
> l'**AC2c**. Il ne pose **aucun filtre d'identité** dans la garde et n'ajoute **aucune
> entrée** à `ACTOR_READING_PREDICATES_ALLOWED` (mika#2323), qui reste `&[]`.

---

## 1. Ce que ce vol livre, et ce qu'il ne livre pas

| | |
|---|---|
| **Livre** | le joint qui rend la branche de lecture de labels de `validate_forced_dispatch_consent` atteignable hors réseau ; la paire positif/contrôle négatif de l'AC2 dans le même appel ; deux détecteurs qui rendent **structurelle** la propriété « aucune route atteignable depuis un tour LLM n'écrit `ready` » et la visibilité de l'applicateur de forme argv |
| **Ne livre pas** | la lecture d'acteur dans la garde (hors périmètre, doctrine mika#2323) ; AC1, AC3, AC4 (vol 1) ; AC5 (sonde en cours depuis le 2026-11-01T06:27:37Z, point zéro 0) ; les variantes `generated/` (renommées au déploiement de `8f7b2260`) |
| **Ne ferme pas, et le dit** | la promotion déléguée des phases 0 et 1 d'`auto_pull` et de la cascade de milestone — voir § 3, c'est un comportement **voulu**, et le fermer arrêterait le vol pour décision de Vincent |

**Aucun label n'est posé ni retiré par cette session ni par ce vol.** Si un événement
`labeled ready` d'identité moteur apparaît pendant la fenêtre, c'est un **fait de sonde
AC5**, pas un effet de ce vol. Merge par Vincent.

---

## 2. AC2b — le chemin `auto_pull` Phase 2 : il **repose** un consentement, il n'en crée aucun

**Verdict : ce chemin n'a pas à être cassé ni exempté. Le vol ne s'arrête pas.**

La preuve est dans la population candidate, pas dans l'intention. `phase2_reconcile_stuck_ready`
(`crates/mika-agent/src/auto_pull.rs:5226`) ouvre par son filtre 1 :

```rust
// Filter 1 (in-mem): keep only tickets WITH the `ready` label (inverse of
// Phase 1's filter). Everything without `ready` is Phase 1 territory.
let ready: Vec<&Issue> = issues
    .iter()
    .filter(|i| i.labels.iter().any(|l| l.name == "ready"))
    .collect();
if ready.is_empty() {
    return 0;
}
```

Trois conséquences, et elles suffisent :

1. **Un ticket sans `ready` ne franchit jamais ce filtre.** La phase 2 ne voit que la
   population *bloquée en `ready`* — un consentement que quelqu'un a déjà donné et que
   le moteur n'a pas su consommer. Le `remove` → `add` d'`auto_pull.rs:5738-5760` ne
   peut donc pas *créer* un consentement : il remet en place celui qui était là, pour
   réinitialiser l'âge du label et re-déclencher le webhook perdu (mika#2470).
2. **L'encadrement est serré et commenté à sa source.** Entre le `gh_remove_label(…, n, "ready")`
   et le ré-ajout par l'applicateur unique, le dernier événement `ready` est un `unlabeled`
   **machine** — le commentaire en place le dit en autant de mots : « the rescue does not
   park itself (AC4) ». Le prédicat de parking (D2 de `ready_label.rs`) ne regarde que
   l'acteur du dernier événement `ready`, donc le sauvetage ne se gare pas lui-même, et
   un `unlabeled` **humain** survenu dans l'intervalle gare le ticket et fait refuser le
   ré-ajout. L'asymétrie est dans le bon sens.
3. **Si le `remove` échoue, le `add` n'a pas lieu** (`continue` après l'incrément du
   compteur d'échec). Il n'existe pas de branche où la phase 2 ajoute sans avoir retiré.

**Sous la voie B, ce chemin est donc inchangé et correct**, et c'est précisément parce
que la voie B porte sur la *capacité du moteur dans un tour LLM*, non sur la capacité du
moteur tout court : la phase 2 n'est atteignable que depuis le corps de tick
d'`auto_pull`, jamais depuis la surface de dispatch d'outils (§ 4 et détecteur D1).

---

## 3. AC2c — le recensement des écrivains

Cinq sites écrivent `ready`. Quatre passent par l'applicateur unique
(`ready_label::apply_ready` / `apply_ready_with_timeline`, mika#2315) ; le cinquième ne
passe pas par lui, délibérément.

| site | porte d'entrée | le ticket porte-t-il déjà `ready` ? | crée-t-il un consentement que personne n'a donné ? | ce vol |
|---|---|---|---|---|
| `auto_pull.rs:4889` — `phase0_feeder` | tick `auto_pull`, après `promotion_gate_allows` | **non** (le feeder promeut) | **oui** | **nomme** |
| `auto_pull.rs:5056` — `phase1_idle_pull` | tick `auto_pull` | **non** (filtre inverse de la phase 2) | **oui** | **nomme** |
| `auto_pull.rs:5750` — `phase2_stuck_rescue` | tick `auto_pull`, filtre 1 = porte `ready` | **oui** | **non** — il repose (§ 2) | rien à faire |
| `server/milestone_context_handler.rs:380` — `milestone_phase_cascade` | handler webhook, après `classify_dispatch_seat` | **non** (cascade M4) | **oui** | **nomme** |
| `milestone_manager/pool_writer.rs:153-160` — `admit_pool_tickets` | tick manager, `MIKA_MANAGER_POOL_WRITE` **OFF** par défaut | **non** (l'admission promeut) | **oui**, et **sans consulter le parc** | **nomme + rend visible** (détecteur D2) |

### 3.1 Pourquoi les quatre premiers sont nommés et non fermés

Les phases 0 et 1 et la cascade M4 **créent** un consentement que personne n'a donné, et
**c'est la promotion déléguée de la boucle autonome** : mika#841 définit `ready` comme le
consentement positif de l'opérateur, et mika#1824 délègue explicitement au moteur le droit
de le poser au nom d'une politique que l'opérateur a armée en amont (le feeder, le
milestone). Fermer ces trois sites tuerait le feeder et la cascade — *une fermeture qui
changerait un comportement voulu arrête le vol pour décision de Vincent*, et ce plan
choisit donc de **nommer**.

**Ce n'est pas une échappatoire, parce que l'incident n'est pas là.** Le défaut mesuré le
2026-10-31 n'est pas « le moteur pose `ready` », c'est « **l'agent, dans son tour LLM,
pose le `ready` qui lève la garde qui venait de le retenir** ». Les quatre sites
gouvernés sont atteignables depuis un corps de tick ou un handler webhook, et depuis rien
d'autre : aucun n'est joignable depuis la surface de dispatch d'outils. C'est cette
propriété-là que le détecteur D1 rend structurelle au lieu de la laisser vraie par
accident.

### 3.2 `pool_writer` — cinquième applicateur, aveugle au parc, et **invisible au scan**

Trois faits, chacun vérifié à sa source :

1. **C'est un cinquième applicateur.** `admit_pool_tickets` bâtit
   `gh issue edit <n> --repo <r> --add-label ready --milestone <titre>` et le passe à
   `runner.run(&args)` — pas à `apply_ready`. Le doc de module en donne la raison et elle
   tient : `apply_ready` pose le label **et rien d'autre**, alors que l'admission est une
   décision groupée (`--add-label` + `--milestone`), et poser `ready` seul en livrerait la
   moitié.
2. **Il est aveugle au parc.** Ne passant pas par l'applicateur, il ne consulte pas le
   prédicat de parking : un ticket qu'un humain a dé-promu en retirant `ready` serait
   re-promu par l'admission. Son idempotence est structurelle et ne couvre pas ce cas (un
   ticket qui *porte* `ready` n'est plus candidat de `super::pool` ; un ticket **parqué**
   ne porte plus `ready`, donc il l'est).
3. **Il est invisible au scan de l'applicateur unique, par sa forme.**
   `no_ready_label_write_outside_this_module` (test 13 de `ready_label.rs`) cherche l'un
   des deux noms d'écrivain (`gh_apply_label`, `add_label_to_issue`) **sur la même ligne**
   que `"ready"` ou `READY_LABEL`. Or la production de `pool_writer` écrit :

   ```rust
   args.push("--add-label");
   args.push(READY_LABEL);
   ```

   Deux lignes, aucune ne portant un nom d'écrivain. **Le scan est donc vert sur tout un
   genre d'écriture** — celui qui passe par un lanceur d'argv générique. C'est exactement
   la classe mika#2578/mika#2752 : une garde qui ne refuse rien pour une forme entière
   passe la revue et passe la CI par construction.

**Ce vol ferme le 3, nomme le 2, et ne touche pas au 1.** Le point 3 est un défaut de
détecteur : le fermer ne change aucun comportement, il rend seulement le site
grep-visible et auto-expirant (détecteur D2, § 7). Le point 2 — router `pool_writer` par
le parc — **changerait un comportement voulu** (la décision groupée) et relève donc de
l'ouverture de `MIKA_MANAGER_POOL_WRITE`, que le doc de module de `pool_writer`
pré-déclare déjà sous le titre `Fire-Disposition: cinquième applicateur découvert`. Ce
vol ne l'ouvre pas et ne l'arme pas : l'interrupteur reste OFF par défaut, et un ticket
non parqué ne peut donc pas être écrit par ce chemin aujourd'hui.

### 3.3 Deux textes de remède qui **nomment** encore le geste — hors périmètre, et dit ici

Le recensement a trouvé deux chaînes Rust qui mentionnent la pose de `ready` :

- `skills/executor.rs:1923` — `recovery` de `dispatch_grooming_not_verified` :
  « Groom through the autonomous loop: re-apply the `ready` label, or … »
- `skills/executor.rs:2879` — `recovery` de `DISPATCH_NO_GROOMING_MARKER` :
  « … (or re-apply the `ready` label) … »

Ce ne sont **pas des routes d'écriture** : zéro capacité, de la prose dans un payload
d'erreur. Le détecteur de prompts bundlés du vol 1
(`canonical_tokens.rs § mika#2995`, `READY_PRESCRIPTION_EXCEPTIONS` livrée vide) ne les
couvre pas — il scanne `skills/bundled/**/system_prompt*.md`, pas les littéraux Rust.
Depuis le vol 1 le moteur ne *peut* plus exécuter ce geste, donc ces deux phrases
décrivent à l'agent une route morte (classe mika#2520).

**Ce vol les nomme et ne les touche pas.** Les corriger serait un amendement d'AC3, que
la décision du 2026-11-01 place **hors du périmètre de ce vol** ; et les deux payloads
s'adressent d'abord à l'opérateur qui lit l'erreur d'outil, où « re-apply » est la
gesture juste. Rien n'est différé *dans* le périmètre de ce vol, donc aucune ligne
`Tracked in:` n'est due ici : c'est une observation à porter au corps de la PR pour
décision de Vincent.

---

## 4. AC2 — la propriété livrable, et la fourche de lecture qu'elle tranche

L'AC2 réécrite demande d'abord : « aucune route du moteur ne peut poser `ready` sur un
ticket qui ne le porte pas ». Elle a **deux lectures**, et il faut dire laquelle ce vol
livre, parce qu'elles n'ont pas le même coût.

| lecture | contenu | conséquence |
|---|---|---|
| **maximale**, littérale | *aucune* route, le feeder gouverné inclus | exige de fermer les phases 0 et 1 et la cascade M4 → **change un comportement voulu** → le vol s'arrête pour décision de Vincent |
| **livrée** par ce vol | **aucune route atteignable depuis un tour LLM** n'écrit `ready` | le feeder gouverné reste entier ; l'incident mesuré devient impossible |

**La lecture livrée est celle qui ferme l'incident**, et c'est aussi celle que la voie B a
déjà à moitié réalisée. Elle se décompose en deux moitiés disjointes :

- **La route outil, `run_gh`** — fermée par le vol 1 :
  `work_creation_is_refused(ReadyLabelAdd, _) => true` sur **toute** la population, câblé
  dans `validate_fallthrough_work_creation` (`builtin_handlers.rs:3200`). Elle est déjà
  attestée par les tests de population de `guards.rs:7595+` et par le test de câblage de
  `builtin_handlers.rs:14890` sur l'argv exact de l'incident
  (`["issue","edit","2958","--add-label","ready"]`). **Ce vol ne la re-teste pas** : un
  prédicat dupliqué diverge en silence (mika#2158). Le plan la cite, la DoD la vérifie.
- **La route gouvernée, l'applicateur unique** — ses quatre appelants vivent dans
  `auto_pull.rs` et `server/milestone_context_handler.rs`, aucun sous la surface de
  dispatch d'outils. Cette propriété est aujourd'hui **vraie par accident** : rien ne la
  refuse. Le **détecteur D1** la rend structurelle.

Les deux autres puces de l'AC2 — le ticket portant un `ready` humain passe, le contrôle
négatif sans `ready` refuse — portent sur la garde de mika#2772 et exigent d'atteindre sa
branche de lecture de labels, ce qui est aujourd'hui impossible hors réseau (§ 5).

---

## 5. Le joint de test : extraction de paramètre, pas d'URL de base configurable

**Le blocage, mesuré.** Les sept tests de `mod mika2772_consentement_force`
(`executor.rs:17079-17290`) passent tous `github_token = None` et s'arrêtent donc à
`FORCED_CONSENT_NO_TOKEN`, l'étape (4) de la garde. **Aucun test existant n'atteint
l'étape (5)**, la lecture des labels — exactement les deux puces que l'AC2 demande. Et
`fetch_issue_labels_unless_pull_request` (`github_graphql.rs:206-274`) écrit
`https://api.github.com/repos/{owner}/{repo}/issues/{number}` en clair, sans URL de base
injectable ; aucun précédent de surcharge n'existe dans le dépôt (`API_BASE`,
`api_base_url`, `MIKA_GITHUB_API` : absents ; une vingtaine de sites codent
`api.github.com` en dur). Donc aucun réglage `Some`/`None` ne donne un parcours hors
réseau.

**La doctrine du dépôt pour exactement cette forme est écrite**, sur le doc-comment
d'`evaluate_grooming_gate` (`executor.rs:2820-2880`), et elle refuse l'alternative dans
les mêmes termes : prendre la donnée récupérée **en paramètre**, « in production the body
comes from the fetch, in test from a fixture », l'URL de base configurable étant « a
production configuration point nobody needs, for the benefit of a test ».

**La coupe.** Deux gestes mécaniques dans `executor.rs`, et aucun changement de
comportement (classe R3 : un déplacement de lignes) :

1. La closure `refuse` (`executor.rs:3509`) ne capture que `ctx.session_id` ; elle devient
   une fonction libre privée `forced_consent_refusal(ctx, verdict, subject, reason) -> String`.
   Ses **cinq** sites d'appel actuels deviennent des appels de fonction. Le texte du champ
   `recovery` — qui **porte l'AC3 du vol 1** — part au déplacement **octet pour octet**.
2. Le `match` final sur le résultat du fetch (`executor.rs:3608-3718`) devient

   ```rust
   pub(crate) async fn decide_forced_consent_on_labels(
       ctx: &LongRunningContext,
       subject: &str,
       forced: bool,
       requested: bool,
       labels: Result<Option<Vec<String>>, String>,
   ) -> ForcedDispatchConsent
   ```

   `validate_forced_dispatch_consent` garde l'appel au fetch et passe son résultat tel
   quel. En production la donnée vient du réseau, en test d'une fixture, et le seul lien
   resté hors du test est le transport HTTP — que l'AC2 n'inclut pas.

**Ce que la coupe ne fait pas**, et c'est le point que la décision du 2026-11-01 impose :
la fonction extraite lit `labels_carry_ready(&labels)` **exactement comme aujourd'hui**.
Aucun acteur, aucune identité, aucune provenance. `ACTOR_READING_PREDICATES_ALLOWED`
(`server/ready_label_handler.rs:2329`) reste `&[]`, et l'assertion d'auto-vacuité qui la
tient déjà est la garde de cette promesse — **la DoD l'exige verte sans modification**.

---

## 6. Les tests de l'AC2 — la paire dans le même appel

Un seul `#[tokio::test]` dans `mod mika2772_consentement_force`, parce que l'AC2 demande
« test dans le même appel, avec contrôle négatif » : la fixture partagée est ce qui
sépare « la garde laisse passer un `ready` » de « la garde laisse tout passer ».

`mika2995_un_ready_humain_laisse_passer_le_groom_force_et_son_absence_le_refuse` :

| moitié | labels passés | attendu |
|---|---|---|
| positive | `Ok(Some(vec!["bug", "ready"]))` | `ForcedDispatchConsent::Allowed`, verdict journalisé `FORCED_CONSENT_CONSENTED` |
| contrôle négatif — **même fixture** | `Ok(Some(vec!["bug"]))` | `ForcedDispatchConsent::Refused`, `error == forced_dispatch_without_consent`, et le `reason` nomme le sujet |

La fixture est celle que le vol 1 a déjà posée : `ctx_forced(INCIDENT_EVENT, true)` (base
en mémoire, `tool_constraint_forced` armé) et `INCIDENT_SUBJECT = "mika#2766"`, donc
`forced = true` et `requested = false` — l'intersection exacte où la garde décide. Le
`refusal(verdict)` existant décode le JSON.

**Et `ready` posé par le moteur ?** Il n'est plus une entrée de test, parce que la voie B
l'a retiré du domaine : la garde ne lit pas la provenance, donc un `ready` moteur et un
`ready` humain sont **le même octet** pour elle. Ce que ce vol teste à la place, c'est
qu'aucune route du moteur ne produit cet octet (§ 4, détecteurs D1 et D2) — c'est la
réécriture de l'AC2 par la voie B, et c'est pour cela qu'elle est une réécriture et non
une version affaiblie.

---

## 7. Les deux détecteurs

Ils vivent tous deux dans `ready_label.rs`, à côté du test 13, parce que c'est le module
qui porte le contrat « qui peut écrire `ready` ». **Deux détecteurs et non un seul : les
populations diffèrent, et deux populations sous un même nom ne se soustraient pas
(mika#2205).** Chacun porte sa propre disposition (§ Fire-Disposition).

### D1 — `mika2995_aucune_surface_doutil_natteint_lapplicateur_de_ready`

- **Population** : la surface de dispatch d'outils — `src/tools/**/*.rs`,
  `src/skills/builtin_handlers.rs`, `src/skills/executor.rs`.
- **Aiguille** : un **appel** à `apply_ready`, `apply_ready_with_timeline`,
  `gh_apply_label` ou `add_label_to_issue`.
- **Exclusions, et pourquoi chacune** : régions de test, via la troncature au premier
  littéral `#[cfg(test)]` (les deux gros fichiers portent leur `mod tests` en ligne) plus
  `crate::source_scan::is_test_source_path` pour les fichiers de test extraits ; et
  **lignes de commentaire**, parce que la prose *à propos* du geste n'est pas un appel —
  `builtin_handlers.rs:3095` et `guards.rs:2782` en portent, et c'est la forme du
  précédent mika#2425.
- **Ce qu'il refuse** : qu'un handler d'outil, demain, appelle l'applicateur gouverné
  « parce qu'il est gouverné ». Il l'est pour le parc, pas pour le tour LLM : le parc dit
  *ce ticket est-il dé-promu*, jamais *qui demande*. La voie B tient parce que la
  capacité est hors d'atteinte, et c'est cette indisponibilité-là qui doit être une
  propriété et non un fait.
- **Contrôle de bonne foi** : `mika2995_le_scan_de_surface_rougit_sur_un_appel_plante` —
  l'aiguille rougit sur un appel planté et reste muette sur quatre formes qui nomment
  l'applicateur sans l'appeler (doc-comment, commentaire de bloc, ligne de `use`, ligne
  de définition). Sans lui, le scan pourrait être vert pour avoir cessé de reconnaître sa
  forme.

### D2 — `mika2995_une_ecriture_de_ready_en_forme_argv_est_declaree`

- **Population** : toute la production `src/**/*.rs` hors `ready_label.rs`.
- **Aiguille** : une **fenêtre** de quelques lignes contenant à la fois le drapeau
  (`--add-label`, `--label=`, `-l`) et le jeton du label (`"ready"` ou `READY_LABEL`).
  La fenêtre est l'essentiel : c'est précisément ce que le test 13 ne voit pas, parce
  qu'il exige les deux sur **la même ligne** et qu'un lanceur d'argv les pousse sur deux
  (§ 3.2). Une ligne de `use crate::ready_label::READY_LABEL;` seule ne déclenche rien —
  le drapeau est requis.
- **Pourquoi un détecteur neuf et pas un élargissement du test 13** : la disposition
  déclarée du test 13 est « **Halt-and-surface, jamais d'allowlist, jamais de
  `#[ignore]` ». L'élargir le ferait tirer sur `pool_writer` et imposerait soit une
  allowlist qu'il refuse en propre, soit une halte sur un site OFF par défaut dont le
  doc de module pré-déclare déjà la question. Le test 13 reste **intact** ; D2 a son nom,
  sa population et sa disposition.

---

## 8. Ordre de commit

1. `refuse` → `forced_consent_refusal` (fonction libre), cinq sites d'appel. Suite verte,
   aucun octet de texte changé.
2. Extraction de `decide_forced_consent_on_labels`. Suite verte.
3. Le test de l'AC2 (paire positive + contrôle négatif dans le même appel). **Rouge avant
   l'étape 2, vert après** — l'ordre est ce qui prouve que le joint était la cause.
4. Détecteur D1 + son contrôle de bonne foi. Allowlist livrée **vide**, assertion
   d'auto-vacuité.
5. Détecteur D2 + son contrôle + l'exception nommée pour `pool_writer`.
6. Doc opérateur : la table du § 3 et les règles de lecture des deux détecteurs
   (hors compte de lignes).

Chaque étape est verte à son commit, et le plancher est l'étape 3 : en deçà, le vol ne
livre pas l'AC2.

---

## Definition of Done

- [ ] `decide_forced_consent_on_labels` existe, prend les labels en paramètre, et
      `validate_forced_dispatch_consent` lui passe le résultat du fetch inchangé.
- [ ] Le champ `recovery` du refus est identique **octet pour octet** à celui du vol 1 —
      `mika2995_le_remede_attribue_la_promotion_sans_prescrire_le_geste` passe sans
      modification.
- [ ] Le test de l'AC2 existe en **un seul appel** et porte ses deux moitiés : `ready`
      présent ⇒ `Allowed` ; même fixture sans `ready` ⇒ `Refused` sur
      `forced_dispatch_without_consent`.
- [ ] Détecteur D1 vert, allowlist vide, assertion d'auto-vacuité présente, contrôle de
      bonne foi présent et rougissant sur un appel planté.
- [ ] Détecteur D2 vert avec son unique exception nommée pour `pool_writer`, son
      assertion auto-nettoyante et son contrôle de bonne foi.
- [ ] `no_ready_label_write_outside_this_module` (test 13) **non modifié** — vérifié au
      diff.
- [ ] `ACTOR_READING_PREDICATES_ALLOWED` toujours `&[]`, et
      `mika2323_no_gate_predicate_reads_the_actor` vert **sans modification**.
- [ ] `READY_PRESCRIPTION_EXCEPTIONS` toujours vide (vol 1), vert sans modification.
- [ ] Les attestations de la route `run_gh` du vol 1 (`guards.rs` population,
      `builtin_handlers.rs:14890` câblage) vertes sans modification.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Le corps de la PR porte `Fire-Disposition: cinquième applicateur découvert` (le doc
      de module de `pool_writer` et le test 13 l'exigent de qui rencontre ce site), la
      table du recensement du § 3, et l'observation du § 3.3 pour décision de Vincent.
- [ ] **Haltes** : si l'implémentation conclut qu'il faut router `pool_writer` par le parc,
      ou fermer les phases 0/1 ou la cascade M4, **elle s'arrête avant le code** et
      remonte — ce sont des décisions de Vincent (§ 3.1, § 3.2).
- [ ] Aucun label posé ni retiré par la session. Aucun `push --force`. Merge par Vincent.

---

## Acceptance criteria

Transcrits du corps de mika#2995. Les AC1, AC3, AC4 et AC5 sont **livrées ou hors
périmètre pour ce vol** et reproduites ici pour la traçabilité ; l'AC2 est la version
**réécrite sous la voie B** par la décision de Vincent du 2026-11-01, qui remplace l'AC2
d'origine.

> **AC1.** Un `ready` posé par l'identité du moteur (l'App ou le jeton de mika-dev) ne
> vaut pas consentement pour la garde de mika#2772. Le plan nomme la lecture retenue :
> l'acteur du dernier événement `labeled ready` dans la timeline, une écriture de `ready`
> refusée au moteur, ou une autre voie. Il dit aussi pourquoi elle tient face à un agent
> qui veut lever la garde. *(Livrée au vol 1 par la voie B : écriture refusée au moteur.)*

> **AC2 (voie B).** Test dans le même appel, avec contrôle négatif.
> - Aucune route du moteur ne peut poser `ready` sur un ticket qui ne le porte pas :
>   `run_gh` (refus livré par le vol 1) et l'applicateur unique
>   `ready_label::apply_ready` / `add_label_to_issue` (mika#2315), ainsi que tout autre
>   site trouvé par le recensement ci-dessous.
> - Un ticket qui porte `ready` posé par un humain : la garde de mika#2772 laisse passer
>   le dispatch forcé.
> - Contrôle négatif, même fixture sans `ready` : la garde refuse.

> **AC2b, le chemin `auto_pull` Phase 2.** Ce chemin repose `ready` en `remove` puis `add`
> sous l'identité du moteur (mika#1824, mika#2323). Le plan dit explicitement comment il
> se comporte sous la voie B. En particulier, il dit s'il ne fait que reposer un
> consentement que le ticket portait déjà, ou s'il peut en créer un. **Si le plan conclut
> qu'il faut casser ou exempter ce chemin, le vol s'arrête avant le code : c'est une
> décision de Vincent.**

> **AC2c, le recensement des écrivains.** Le plan recense chaque appelant de l'applicateur
> unique, notamment les phases de promotion d'`auto_pull`, la cascade de milestone et
> `pool_writer` (sous `MIKA_MANAGER_POOL_WRITE` OFF). Pour chacun, il dit si le site crée
> un consentement que personne n'a donné. Ce vol ferme ce trou ou le nomme. **Une
> fermeture qui changerait un comportement voulu arrête aussi le vol, pour décision de
> Vincent.**

> **AC3.** Le texte de remède du refus ne prescrit plus à l'agent un geste qu'il ne doit
> pas faire lui-même. Il dit que la promotion appartient à l'opérateur. *(Livrée au
> vol 1 ; ce vol la préserve octet pour octet au déplacement.)*

> **AC4.** Le plan recense tous les sites où le moteur lit `ready` comme consentement : le
> feeder `ready-label`, `stuck_ready_reconcile`, la porte de place mika#2084, et les
> autres. Pour chacun, il dit si le même trou existe, et ce vol le ferme ou le nomme.
> *(Livrée au vol 1, § 2.3 de son plan — recensement des **lecteurs**. L'AC2c ci-dessus
> est le recensement des **écrivains**, qui en est le symétrique et non le doublon.)*

> **AC5.** Sonde après déploiement : sur les 30 jours suivants, le compte d'événements
> `labeled ready` dont l'acteur est l'identité du moteur est écrit ici. La cible est 0, ou
> chaque cas est refusé par la garde. *(Hors périmètre de ce vol : sonde en cours depuis
> le 2026-11-01T06:27:37Z, point zéro 0.)*

**Vérifiable dans ce vol** — chaque ligne a son site :

| critère | attestation |
|---|---|
| AC2 puce 1, route outil | tests du vol 1 (`guards.rs:7595+`, `builtin_handlers.rs:14890`) verts sans modification |
| AC2 puce 1, route gouvernée | détecteur D1 (surface d'outil) + détecteur D2 (forme argv) |
| AC2 puces 2 et 3 | `mika2995_un_ready_humain_laisse_passer_le_groom_force_et_son_absence_le_refuse`, un seul appel, fixture partagée |
| AC2b | § 2 : filtre 1 de `phase2_reconcile_stuck_ready` — repose, ne crée pas. Aucun code. |
| AC2c | § 3 : table des cinq sites ; trou de `pool_writer` **rendu visible** par D2, routage par le parc **nommé** et non fait |
| non-régression mika#2323 | `ACTOR_READING_PREDICATES_ALLOWED` toujours `&[]` |

---

## Fire-Disposition

Ce plan livre **deux détecteurs** (§ 7). Chacun porte sa propre disposition, parce que
leurs populations diffèrent.

**D1 — `mika2995_aucune_surface_doutil_natteint_lapplicateur_de_ready` : option (a),
exception nommée en allowlist, livrée VIDE.**

- Allowlist : `const TOOL_SURFACE_APPLICATOR_EXCEPTIONS: &[(&str, &str)] = &[];`
  (fichier, ticket).
- **Elle est livrée vide parce que la population est vide** : les quatre appelants de
  l'applicateur sont dans `auto_pull.rs` et `server/milestone_context_handler.rs`, aucun
  sous la surface d'outil. Vérifié par `grep -rn 'apply_ready' crates/mika-agent/src`
  le 2026-11-01.
- Une allowlist vide peut mentir de trois façons, et les trois sont tenues :
  1. **Elle pourrait se remplir en silence** ⇒ assertion sœur d'auto-vacuité
     (`mika2995_lallowlist_de_surface_est_vide`), qui rougit à la première entrée. Quand
     D1 tire, on **retire l'appel** ; on n'ajoute pas une ligne (doctrine mika#2201).
  2. **Le scan pourrait n'avoir rien scanné** ⇒ `assert!(scanned > 0, …)` sur le nombre
     de fichiers de la population, l'idiome du test 13.
  3. **Il pourrait avoir cessé de reconnaître sa forme** ⇒ contrôle de bonne foi
     `mika2995_le_scan_de_surface_rougit_sur_un_appel_plante`, avec ses quatre formes
     muettes (doc-comment, commentaire de bloc, `use`, ligne de définition).

**D2 — `mika2995_une_ecriture_de_ready_en_forme_argv_est_declaree` : option (a),
exception nommée en allowlist, livrée avec UNE entrée.**

```rust
const ARGV_READY_WRITERS_DECLARED: &[(&str, &str, &str)] = &[(
    "src/milestone_manager/pool_writer.rs",
    "mika#2995",
    "admit_pool_tickets : décision groupée --add-label ready + --milestone, \
     hors applicateur par conception (propriété 4 du doc de module), donc \
     AVEUGLE AU PARC. MIKA_MANAGER_POOL_WRITE OFF par défaut : aucune écriture \
     non parquée aujourd'hui. Router ce site par le parc changerait un \
     comportement voulu — décision d'opérateur, pré-déclarée sous \
     `Fire-Disposition: cinquième applicateur découvert`.",
)];
```

- **Grep-visible** : le chemin, le ticket et la donnée précise (le nom de la fonction, la
  raison du contournement, l'interrupteur qui le borne) sont dans le littéral.
- **Assertion auto-nettoyante** : pour chaque entrée, le fichier doit **encore porter la
  forme argv**. Si `pool_writer` est un jour routé par `apply_ready`, supprimé, ou
  réécrit, l'entrée devient stale et le test rougit avec le message « retirez l'entrée,
  le site a changé ». Le geste qui rend l'exception inutile est celui qui la fait expirer
  — c'est ce qui l'empêche de devenir l'allowlist que mika#2309 condamne.
- **Contrôle de bonne foi** : `mika2995_le_scan_argv_rougit_sur_une_ecriture_plantee`
  rougit sur les trois syntaxes (`--add-label` / `--label=` / `-l`) poussées sur deux
  lignes, et reste muet sur un `use READY_LABEL` sans drapeau et sur un `--add-label`
  portant un autre label.
- **Pourquoi pas (c) halte-et-remontée** : le site existant est déjà connu, déjà
  argumenté à sa source, et déjà borné par un interrupteur OFF. Une halte ferait payer un
  pilote pour un fait que le doc de module de `pool_writer` écrit depuis le vol 4 du
  manager. La halte reste due à **l'ouverture** de l'interrupteur, et ce plan la
  réinscrit (§ 3.2).
- **Pourquoi pas (b) livrer désarmé** : un détecteur `#[ignore]` sur une forme entière
  d'écriture est exactement ce que le § 3.2 vient de constater comme défaut
  (mika#2578/mika#2752). Il atterrit armé.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/executor.rs` — joint de paramètre (`decide_forced_consent_on_labels` + `forced_consent_refusal`, doc comprise) | 55 |
| `crates/mika-agent/src/skills/executor.rs` — test AC2 (positif + contrôle négatif, même appel) | 75 |
| `crates/mika-agent/src/ready_label.rs` — détecteur D1 + allowlist vide + auto-vacuité + contrôle de bonne foi | 105 |
| `crates/mika-agent/src/ready_label.rs` — détecteur D2 + exception `pool_writer` + assertion auto-nettoyante + contrôle de bonne foi | 120 |

Total estimé : 355 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 355 < 429, et les deux bornes passent (355 < 1000 par
PR). Trois raisons rendent l'estimation fiable plutôt qu'optimiste :

1. **Le joint est un déplacement, pas une écriture.** Les ~140 lignes du `match` et de la
   closure `refuse` existent déjà et bougent telles quelles ; le delta compté (55) est la
   signature, le doc-comment et le site d'appel. Un déplacement ne porte pas de risque de
   dépassement : il compile ou il ne compile pas.
2. **Les deux détecteurs sont des variantes d'un idiome que le module porte déjà trois
   fois** (test 13 de `ready_label.rs`, le scan de prompts du vol 1, le scan de lecteur de
   mika#2425). Le squelette — marcher `src/**/*.rs`, tronquer au `#[cfg(test)]`, exclure
   les commentaires, `assert!(scanned > 0)`, allowlist + auto-vacuité + contrôle de bonne
   foi — est recopié, pas inventé.
3. **Aucun comportement de production ne change.** Zéro migration, zéro variable
   d'environnement neuve, zéro surface opérateur neuve, aucun appel réseau nouveau. La fin
   de pipeline est donc la fin de pipeline nominale, celle dont les 30 tours sont déjà
   soustraits dans la borne.

Rien n'est renvoyé à une suite : les deux seules choses que ce vol ne fait pas — router
`pool_writer` par le parc, et corriger les deux textes du § 3.3 — ne sont pas des
déférements de périmètre mais des **décisions de Vincent**, portées au corps de la PR. Le
vol 2 clôt mika#2995.

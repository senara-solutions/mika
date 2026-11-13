# fix(2961) — une citation de marqueur est une injection : la ligne 67 de `qa-review` et le détecteur d'arité

> Ticket : senara-solutions/mika#2961 — vol 3/3 (périmètre fixé par le GO du 2026-10-31,
> commentaire 6542699518). Labels : `bug`, `p1-important`, `loop-substrate`, `dispatch:loop`.
> Milestone : *Substrat de boucle* (#33).

---

## 1. Où en est le ticket, et pourquoi ce vol-là

Les deux premiers vols du ticket sont mergés le 2026-10-30 : mika#2967 (`3335ecf5`,
section `PIPELINE:`) et mika#2974 (`a219d0a3`, ligne `NEGATIVE-TEST:`). AC1 à AC4 du
corps sont livrés ; AC5 est une sonde opérateur qui attend l'ouverture de
`MIKA_QA_PASS_EVIDENCE_GATE`. **Ce vol ne touche à aucun des quatre.**

Ce qui l'a rouvert est la chaîne de deux mesures de samidarko :

1. **2026-10-31 10:58Z.** Le tour QA ne lisait pas la base de `qa-review` mais une
   variante `generated/zai/glm-5.2` figée au 2026-06-30, qui ne portait ni `PIPELINE`
   ni `NEGATIVE-TEST` — ni, surtout, **aucun `{{pr_diff}}`**. Les deux gardes des vols 1
   et 2 étaient donc inouvrables : le modèle n'avait jamais vu la consigne.
2. **2026-10-31 14:13Z.** Les variantes de `qa-review` et `qa-review-build-callback`
   sont écartées depuis le redémarrage de 12:59:02Z. La sonde est positive (revue de
   mika#2992 à 13:53:50Z : `"qa-review":"base"`, `PIPELINE: pass (mika)`,
   `NEGATIVE-TEST: satisfied`). **Mais la base servie injecte le diff deux fois.**

La mesure du doublon, telle qu'elle est écrite au ticket et revérifiée ici :
`system_prompt_assembled` de la revue de #2992 (trace `5bbe3830`) donne
`per_skill_bytes["qa-review"] = 230 673` pour un fichier de 77 763 octets. L'écart est
de 152 910, et `gh pr diff 2992 | wc -c` donne 76 401. Or
2 × 76 401 + 108 = 152 910.

La cause est mécanique et vérifiée dans ce worktree :

- `apply_context_replacements` (`crates/mika-agent/src/skills/context.rs:276`) remplace
  **toutes** les occurrences de `{{clé}}` — c'est un `replace_all` sur
  `PLACEHOLDER_RE`, appelé une fois depuis `crates/mika-agent/src/agent_loop/mod.rs:10523` ;
- `skills/bundled/qa-review/system_prompt.md` porte le marqueur **deux** fois :

  | ligne | rôle |
  |---|---|
  | 509 | le vrai bloc : `<context type="pr_diff" trust="untrusted">` / `{{pr_diff}}` / `</context>` |
  | 67 | une **citation** du marqueur, au milieu de la règle `DEPTH` : « read the `` `<!-- context_meta: ... -->` `` annotation prepended to the `` `{{pr_diff}}` `` block below » |

  (`grep -no '{{[a-z_]*}}' skills/bundled/**/system_prompt.md` ne rend que ces deux
  lignes : dans tout `skills/bundled/`, **un seul** marqueur existe, et il est en double.)

La citation de la l. 67 vient de mika#1359 (`961f10287`, 2026-06-01, déclaration de
profondeur). Elle n'a jamais rien coûté avant le 2026-10-31 parce que la variante du
2026-06-30 ne portait aucun marqueur : pendant quatre mois, la QA n'a reçu **aucun**
diff injecté et sa ligne `DEPTH` ne s'appuyait sur aucun bloc. C'est l'inversion qui
compte pour la lecture du défaut : **la variante périmée masquait le doublon, et le
retour à la base l'a révélé.**

Ce vol livre exactement les points 1 et 2 du commentaire du 2026-10-31 14:13Z, et rien
d'autre. Il **n'ouvre pas** `MIKA_QA_PASS_EVIDENCE_GATE`, **ne touche à aucune**
variante `generated/`, et **ne change pas** `apply_context_replacements`.

### Périmètre, face aux PR ouvertes

La contrainte écrite au corps le 2026-10-30 interdit de toucher
`validate_qa_ci_coherence`, `validate_qa_ci_coherence_with_reader`, `run_gh`
(mika#2966) et `make_pr_dedup_key` (mika#2973). **Ce vol n'entre dans aucune d'elles :
il ne touche pas `builtin_handlers.rs` du tout**, et pas davantage
`evidence/guards.rs`. Vérifié sur les PR ouvertes du dépôt au 2026-10-31 :

```
gh pr list --repo senara-solutions/mika --state open --json number,title,files \
  -q '.[] | {n:.number, f:[.files[].path | select(test("bundled_skills.rs|qa-review/system_prompt.md|skills/context.rs"))]} | select(.f|length>0)'
```

→ **sortie vide.** Aucune PR ouverte ne touche les deux fichiers de ce vol.

---

## 2. Le dessin

### 2.1 Livrable 1 — la citation cesse d'être un marqueur (point 1)

Un seul remplacement, dans `skills/bundled/qa-review/system_prompt.md`, ligne 67.

**Texte actuel (littéral) :**

```
Every verdict MUST include a `DEPTH:` line that honestly declares the level of code analysis performed. The depth is determined by the engine-injected diff availability — read the `<!-- context_meta: ... -->` annotation prepended to the `{{pr_diff}}` block below.
```

**Texte prescrit (littéral) :**

```
Every verdict MUST include a `DEPTH:` line that honestly declares the level of code analysis performed. The depth is determined by the engine-injected diff availability — read the `context_meta` annotation prepended to the `pr_diff` context block below.
```

Deux changements, et chacun a sa raison :

- `` `{{pr_diff}}` `` → `` `pr_diff` context block `` : le marqueur disparaît, la **désignation** reste. La règle `DEPTH` continue de nommer le bloc qu'elle commente ; elle ne le *cite* plus dans la syntaxe que le moteur substitue.
- `` `<!-- context_meta: ... -->` `` → `` `context_meta` `` : la forme complète de l'annotation, avec ses trois champs, est **déjà portée mot pour mot par la ligne 76** du même fichier (« Read the `` `<!-- context_meta: type=gh_pr_diff, status=..., chars=... -->` `` annotation to determine the status »). La l. 67 l'annonce, la l. 76 la spécifie : raccourcir l'annonce ne retire aucune règle, et c'est ce qui rend le remplacement net-négatif en octets.

**Pourquoi cette comptabilité d'octets est load-bearing.**
`bundled_skills_approaching_max_prompt_size_warns`
(`crates/mika-agent/tests/bundled_skills_load.rs:91`) **panique** — pas un warn — dès
que le prompt atteint 95 % de son cap effectif. Le cap de `qa-review` est à
`MAX_PROMPT_SIZE_CEILING` (81 920, `skills/index.rs:27`), donc le gate est à
**77 824 octets**. Le fichier mesure aujourd'hui **77 763** : il reste **61 octets** de
marge, pas les 215 que le long commentaire de `skill.toml` annonce (le prompt a grandi
depuis). Le remplacement prescrit vaut **Δ = −10** :

| segment | avant | après | Δ |
|---|---|---|---|
| `` `<!-- context_meta: ... -->` `` → `` `context_meta` `` | 28 | 14 | −14 |
| `` `{{pr_diff}}` `` → `` `pr_diff` `` | 13 | 9 | −4 |
| insertion de `` context `` (avec son espace) | 0 | 8 | +8 |

**Taille attendue après édition : 77 753 octets**, soit 71 octets de marge au gate. À
vérifier par `wc -c skills/bundled/qa-review/system_prompt.md` et **à reporter dans le
corps de la PR**. Condition dure, quelle que soit la formulation finalement retenue :
**≤ 77 824**. Ne pas toucher `max_prompt_size` dans `skill.toml` — il est déjà au
plafond dur, et ce vol fait *descendre* la taille.

**Gain attendu côté runtime**, à la mesure du ticket : 76 401 octets par appel sur un
diff de la taille de #2992, soit ≈ 21 000 jetons d'entrée. Le prompt QA devrait
redescendre vers ≈ 243 Ko sur un diff comparable. **Ce chiffre est une prédiction, pas
une mesure** : il se mesure après déploiement, par le `per_skill_bytes` de
`system_prompt_assembled` sur la prochaine revue. Le plan ne le fait pas valider par CI
— rien dans le dépôt ne peut l'observer — et la PR l'écrit comme attendu, pas comme
constat.

**Rien ne dépend du texte de la l. 67.**
`grep -rln "context_meta\|{{pr_diff}}\|Review Depth Declaration"` sur
`crates/mika-agent/tests`, `crates/mika-agent/src` et `scripts/` ne rend que
`skills/index.rs`, `skills/context.rs` et `agent_loop/mod.rs` — c'est-à-dire la
machinerie d'injection, dont aucune assertion ne lit ce prompt. Aucun test n'épingle
cette ligne, donc la réécriture ne casse rien par surface.

### 2.2 Livrable 2 — le détecteur d'arité (point 2)

**L'invariant, écrit en une phrase :** *un marqueur de contexte déclaré dans
`[context.*]` apparaît **exactement une fois** dans le `system_prompt.md` du même skill.*

Les deux sens sont voulus, et chacun nomme un défaut réel :

- **≥ 2 occurrences** — le défaut mesuré ici : le contenu résolu est injecté *n* fois,
  au prix de *n* × sa taille, dans des endroits dont un seul était voulu. 2 × 76 401
  octets sur la revue de #2992.
- **0 occurrence** — le symétrique, invisible et non gratuit : le handler de contexte
  est dispatché, le contenu résolu et jamais consommé. C'est exactement l'état que la
  variante périmée a tenu quatre mois pour `pr_diff`, et celui qui a fait rendre à la QA
  des lignes `DEPTH:` sans bloc sous les yeux. Un détecteur qui n'accepterait que
  « ≤ 1 » laisserait passer la moitié du défaut qui a rendu ce ticket nécessaire.

**Site retenu : `crates/mika-agent/src/bundled_skills.rs`, dans son `mod tests`
existant.** Trois raisons, dans cet ordre :

1. **Il juge le texte embarqué.** `all_bundled_skills()` rend les `SkillFile` compilés
   dans le binaire par `build.rs` — c'est-à-dire, à l'octet près, le `resolved.text`
   que `apply_context_replacements` reçoit en `agent_loop/mod.rs:10523` lorsque la base
   est servie. Le défaut est né dans le texte **servi** ; le détecteur le lit là.
2. **Il n'a aucun type à étendre.** `SkillManifest` porte déjà
   `context: HashMap<String, ContextRequirement>` (`skills/manifest.rs:46`), et
   `registry_from_bundled_manifests()` (l. 1860) montre le parse exact à réutiliser.
3. **Précédent de classe dans le même module.**
   `mika2355_every_bundled_callback_handler_is_reachable_on_a_callback_turn` (l. 1908)
   est un détecteur structurel sur la même population, avec le gabarit d'assertion de
   population non vide à reprendre tel quel.

**Site écarté, et pourquoi le dire.** `crates/mika-agent/src/bin/verify_bundled_skills.rs`
porte cinq checks structurels numérotés, une allowlist `KnownException` et le
`make verify-bundled-skills` de CI : un `check6` y serait défendable. Il est écarté
parce qu'il lit l'**arbre du dépôt** (`load_skill_views(base)`) et non l'embarqué, qu'il
faudrait étendre son `Manifest` maison pour y faire entrer `[context.*]`, et qu'il n'a
pas besoin d'une allowlist ici (cf. § 7 : l'ensemble d'exceptions est vide). **Si
l'invariant gagne un jour des exceptions légitimes, c'est ce fichier-là qui devient le
bon site** — il en a déjà l'infrastructure, y compris le test
`known_exceptions_ship_empty`.

**Forme prescrite** (le squelette, pas le dernier mot sur la rédaction) :

```rust
/// mika#2961 — un marqueur de contexte déclaré est injecté EXACTEMENT une fois.
///
/// `apply_context_replacements` (`skills/context.rs`) est un `replace_all` : chaque
/// occurrence de `{{clé}}` dans le prompt reçoit une copie COMPLÈTE du contenu
/// résolu, annotation comprise. Le 2026-10-31, la base de `qa-review` portait
/// `{{pr_diff}}` deux fois — le vrai bloc (l. 509) et une citation dans la règle
/// `DEPTH` (l. 67, mika#1359) — et la revue de mika#2992 a payé 2 × 76 401 octets
/// de diff dans un prompt de 230 673. La citation datait du 2026-06-01 et n'avait
/// jamais coûté : la variante `generated/` servie jusqu'au 2026-10-31 ne portait
/// aucun marqueur, donc ce prompt-là n'injectait rien du tout.
///
/// Les deux sens de l'arité sont des défauts. `>= 2` est la duplication ci-dessus ;
/// `0` est le contexte dispatché, résolu, et jamais consommé — l'état que la
/// variante périmée a tenu quatre mois, où la QA rendait un `DEPTH:` sans bloc.
///
/// **Portée, et sa borne.** Le détecteur ne juge QUE les clés déclarées en
/// `[context.*]`, et QUE le fichier `system_prompt.md`. Un marqueur non déclaré
/// (`{{foo}}` sans `[context.foo]`) est hors portée à dessein : il n'est pas
/// substitué — `apply_context_replacements` laisse les clés inconnues telles
/// quelles — donc il ne coûte rien. Une éventuelle variante de prompt embarquée
/// sous un autre nom de fichier est hors portée aussi : borne inférieure assumée.
#[test]
fn mika2961_un_marqueur_de_contexte_declare_est_injecte_exactement_une_fois() {
    let mut declared = 0usize;
    let mut violations: Vec<String> = Vec::new();

    for skill in all_bundled_skills() {
        let Some(toml_src) = skill.files.iter().find(|f| f.path == "skill.toml") else {
            continue;
        };
        let manifest: crate::skills::manifest::SkillManifest =
            toml::from_str(toml_src.content)
                .unwrap_or_else(|e| panic!("{} skill.toml must parse: {e}", skill.name));
        if manifest.context.is_empty() {
            continue;
        }
        let prompt = skill
            .files
            .iter()
            .find(|f| f.path == "system_prompt.md")
            .map(|f| f.content)
            .unwrap_or("");

        for key in manifest.context.keys() {
            declared += 1;
            let marker = format!("{{{{{key}}}}}");
            let lines: Vec<usize> = prompt
                .lines()
                .enumerate()
                .filter(|(_, l)| l.contains(&marker))
                .map(|(i, _)| i + 1)
                .collect();
            let count = prompt.matches(marker.as_str()).count();
            if count != 1 {
                violations.push(format!(
                    "{}: `{marker}` déclaré en [context.{key}] apparaît {count} fois \
                     (lignes {lines:?}) — attendu exactement 1",
                    skill.name
                ));
            }
        }
    }

    assert!(
        declared > 0,
        "population vide — aucun skill bundled ne déclare de [context.*] : le \
         détecteur ne compte plus rien, vérifier la découverte de build.rs avant \
         de croire ce test vert"
    );

    assert!(
        violations.is_empty(),
        "arité de marqueur de contexte violée — chaque occurrence au-delà de la \
         première fait injecter une copie COMPLÈTE du contenu résolu, et zéro \
         occurrence fait résoudre un contexte que personne ne lit :\n{}",
        violations.join("\n")
    );
}
```

Points non négociables de cette forme :

- **L'assertion de population non vide d'abord.** Sans elle, le détecteur est vert le
  jour où la découverte de `build.rs` casse, ou le jour où le dernier `[context.*]`
  disparaît. Aujourd'hui la population vaut **1 skill, 1 clé** — c'est petit, et c'est
  précisément pourquoi le garde de population compte.
- **Les numéros de ligne dans le message.** Le défaut se répare en regardant les deux
  sites ; un compte nu force une recherche manuelle.
- **Le POURQUOI dans le message d'échec**, pas seulement dans le doc-comment : c'est le
  texte que lit celui qui reçoit le rouge.

### 2.3 Ce que ce vol ne fait pas, et qui le porte

| non fait | pourquoi, et où ça vit |
|---|---|
| ouvrir `MIKA_QA_PASS_EVIDENCE_GATE` | décision de Vincent, condition de réveil du ticket. AC5 se compte **après** l'ouverture, et désormais **après** ce vol : le prompt que la sonde mesure n'est stable qu'une fois la base servie sans doublon. |
| écarter / renommer les 10 variantes `generated/` de mika-dev | explicitement réservé à Vincent par le commentaire du 2026-10-31 10:58Z. |
| le garde d'empreinte base↔variante (point 2 du commentaire de 10:58Z) | le vrai correctif de la cause racine, et il est **hors de ce périmètre** : le GO de 14:13Z ne retient que les points 1 et 2 de 14:13Z. Sans lui, le prochain `review_skill` recréera l'écart en silence. À suivre hors de ce ticket — gel du milestone #33 oblige, aucun ticket neuf n'est ouvert ici. |
| changer `apply_context_replacements` | interdit par le GO. Le `replace_all` n'est pas le défaut : un prompt qui veut deux injections les obtient légitimement. Le défaut est qu'une *citation* en prenne la forme. |

---

## 3. Fichiers touchés

| fichier | geste |
|---|---|
| `skills/bundled/qa-review/system_prompt.md` | 1 ligne réécrite (l. 67). Δ = −10 octets. |
| `crates/mika-agent/src/bundled_skills.rs` | 1 test ajouté dans le `mod tests` existant, avec son doc-comment. |

**Hors périmètre, à ne pas ouvrir** : `crates/mika-agent/src/skills/builtin_handlers.rs`
(PR mika#2966 et mika#2973 en vol), `crates/mika-agent/src/evidence/guards.rs`,
`crates/mika-agent/src/skills/context.rs`, `skills/bundled/qa-review/skill.toml`,
toute variante `generated/`.

**CODEOWNERS : aucun des deux fichiers n'y est.** `.github/CODEOWNERS` couvre
`src/perimeter/`, `server/verdict_handler.rs`, `tools/pr_merge_with_gate.rs`,
`docs/gate/`, `docs/egress/`, `well_known_agents.rs` et lui-même. La réserve du corps
(« les fichiers de verdict sont sous CODEOWNERS @samidarko ») valait pour les vols 1 et
2 ; **elle ne mord pas sur ce vol**. Le merge reste celui de Vincent par la ligne
« Merge par Vincent (loop-substrate) » du ticket, et non par un blocage de revue requise.

---

## 4. Ordre de commit

L'ordre est une **exigence du ticket**, pas une préférence : « Le test du point 2 doit
rougir sur la base actuelle avant la correction de la l. 67, puis verdir une fois la
ligne corrigée. » La preuve de ce rouge n'existe que si le test atterrit **avant** la
correction.

1. **`test(2961): un marqueur de contexte déclaré est injecté exactement une fois`**
   — le détecteur seul. Puis `cargo test -p mika-agent --lib mika2961` et **capture de
   la sortie rouge** : elle doit nommer `qa-review`, `{{pr_diff}}`, `2 fois`, et les
   lignes `[67, 509]`. Cette sortie va **verbatim dans le corps de la PR** : c'est la
   seule attestation que le détecteur détecte.
2. **`fix(2961): la règle DEPTH désigne le bloc pr_diff au lieu de citer son marqueur`**
   — la l. 67. Puis le même `cargo test` **vert**, et `wc -c` sur le prompt (attendu
   77 753, condition dure ≤ 77 824). Les deux chiffres vont au corps de la PR.

Un commit intermédiaire rouge sur la branche est assumé : CI ne juge que la tête, et la
tête du vol est verte. Inverser l'ordre rendrait la PR verte sans jamais prouver que le
rouge existait — c'est le mode d'échec que la phrase du ticket interdit.

---

## 5. Definition of Done

- [ ] `skills/bundled/qa-review/system_prompt.md` ne porte plus qu'**une** occurrence de
      `{{pr_diff}}` (l. 509), vérifié par
      `grep -c '{{pr_diff}}' skills/bundled/qa-review/system_prompt.md` → `1`.
- [ ] La règle `DEPTH` (l. 67) désigne toujours le bloc et l'annotation ; la forme
      complète `<!-- context_meta: type=…, status=…, chars=… -->` reste présente à la
      l. 76, inchangée.
- [ ] `wc -c skills/bundled/qa-review/system_prompt.md` ≤ **77 824** (gate 95 %), valeur
      exacte reportée au corps de la PR. `max_prompt_size` **non modifié**.
- [ ] Le détecteur existe dans `crates/mika-agent/src/bundled_skills.rs::mod tests`,
      porte l'assertion de population non vide, et nomme skill / clé / compte / lignes
      dans son message d'échec.
- [ ] **Preuve rouge→vert** dans le corps de la PR : sortie du détecteur au commit 1
      (rouge, nommant les lignes 67 et 509) et au commit 2 (vert).
- [ ] `cargo test -p mika-agent` vert — en particulier
      `bundled_skills_approaching_max_prompt_size_warns` et
      `bundled_skills_load_without_oversized_prompts`, qui paniquent sur le gate.
- [ ] `cargo clippy` sans warning neuf ; `cargo fmt --check` propre.
- [ ] `make verify-bundled-skills` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Le corps de la PR écrit le gain attendu (≈ 76 401 octets / ≈ 21 000 jetons par
      appel sur un diff de la taille de #2992) **comme une prédiction à mesurer après
      déploiement**, par le `per_skill_bytes` de `system_prompt_assembled` — et non
      comme un constat.
- [ ] Le corps de la PR nomme ce qui reste ouvert : le garde d'empreinte
      base↔variante (point 2 du commentaire de 10:58Z) et les 10 variantes
      `generated/` de mika-dev, tous deux hors périmètre par le GO du 2026-10-31.
- [ ] `Code review: delegated to mika-qa (mika#2694)` au corps de la PR ; pas
      d'invocation de `ce-code-review` ni de `ce-simplify-code`.

---

## 6. Acceptance criteria

### Du vol courant (points 1 et 2 du GO du 2026-10-31 14:13Z)

- **AC-V3.1.** La ligne 67 de `skills/bundled/qa-review/system_prompt.md` ne contient
  plus la syntaxe de marqueur `{{pr_diff}}` ; elle désigne le bloc par son nom. Le
  fichier porte exactement une occurrence du marqueur, à la l. 509. La règle `DEPTH`
  reste intégralement énonçable : aucune de ses lignes (`code-level`,
  `code-level (partial)`, `metadata-only`, le plafond `hold[review]`, le placement entre
  `VERDICT:` et `REASON:`) n'est retirée, et la forme complète de l'annotation
  `context_meta` demeure à la l. 76.
- **AC-V3.2.** Un test sur les skills embarqués exige qu'une clé déclarée en
  `[context.*]` apparaisse **exactement une fois** dans le `system_prompt.md` du même
  skill. Il **rougit sur la base actuelle** (`qa-review` / `pr_diff`, 2 occurrences,
  lignes 67 et 509) et **verdit** après AC-V3.1. Les deux sorties sont au corps de la PR.
- **AC-V3.3.** Le prompt reste sous le gate à 95 % : `wc -c` ≤ 77 824, sans toucher
  `max_prompt_size`. La valeur mesurée est écrite au corps de la PR.
- **AC-V3.4.** Le vol ne modifie ni `apply_context_replacements`, ni aucune variante
  `generated/`, ni `MIKA_QA_PASS_EVIDENCE_GATE`, ni aucune des quatre fonctions
  réservées par les PR en vol (`validate_qa_ci_coherence`,
  `validate_qa_ci_coherence_with_reader`, `run_gh`, `make_pr_dedup_key`). Vérifiable sur
  le diff : deux fichiers touchés, et aucun n'est `builtin_handlers.rs`.

### Du corps du ticket, transcrits verbatim, avec leur disposition

- **AC1.** « Un `pr review --approve` dont le corps porte `VERDICT: pass` sans ligne
  `PIPELINE:` n'est pas posté comme `pass`, dans la même chaîne que mika#2687. Il est
  soit refusé, soit plafonné à `hold[review]`, selon le choix écrit du plan. Le plan dit
  s'il traite `NEGATIVE-TEST:` dans le même garde, ou pourquoi non. »
  → **Livré** par mika#2967 (`3335ecf5`) et mika#2974 (`a219d0a3`). Hors périmètre ici.
- **AC2.** « Test dans le même appel, sur des corps **mesurés** et non inventés : les
  revues de control-monitor#217 (2026-10-29T14:26:24Z) et de mika#2960
  (2026-10-29T13:57:06Z) sont arrêtées, et une revue de mika#2691 (2026-10-14) qui porte
  `PIPELINE:` passe. »
  → **Livré** par le vol 1. Hors périmètre ici.
- **AC3.** « L'abstention de mika#2455 : le plan tranche par écrit si elle reste
  fail-open quand le dépôt de la PR ne merge pas par `pr_merge_with_gate`, avec
  control-monitor comme cas mesuré. Si elle reste ouverte, il dit ce qui tient alors le
  merge fermé. »
  → **Tranché** au plan du vol 1. Hors périmètre ici — et ce vol ne touche pas
  `validate_qa_ci_coherence`, qui est au surplus réservée par mika#2966.
- **AC4.** « Un interrupteur suit le patron de `PIPELINE_GROUNDING_GATE_ENV`, avec le
  même contrat : lu une fois, et dit au démarrage s'il est désarmé. L'ouverture revient
  à Vincent. »
  → **Livré** (`MIKA_QA_PASS_EVIDENCE_GATE`). **Reste fermé** par ce vol.
- **AC5.** « Sonde après ouverture de l'interrupteur : sur les revues
  `mika-platform-qa` de 7 jours, le compte des `pass` sans ligne `PIPELINE:` est écrit
  ici, avec la requête qui l'a produit. La cible est 0. Le compte des tours QA morts
  sans revue sur la même fenêtre est écrit à côté. »
  → **En attente**, et ce vol **déplace sa date de départ** : la sonde ne mesure quelque
  chose d'interprétable qu'une fois la base servie **sans** le doublon. La fenêtre de 7
  jours part de l'ouverture de l'interrupteur par Vincent, postérieure à ce vol.
  Requête : S1 de mika#2974.

---

## 7. Fire-Disposition

Ce plan livre **un détecteur** : le test AC-V3.2, dont le chemin de succès est « aucune
violation d'arité trouvée ».

**Option retenue : (a) exception nommée en allowlist — appliquée avec un ensemble
d'exceptions VIDE.**

Le détail d'implémentation, et pourquoi le vide est la bonne réponse ici plutôt qu'une
dérobade :

1. **La population est connue et close.** `grep -rno '{{[a-z_]*}}'` sur
   `skills/bundled/**/system_prompt.md` rend exactement deux lignes, et
   `grep -rn '^\[context\.' skills/bundled/*/skill.toml` rend exactement une
   déclaration : `[context.pr_diff]` de `qa-review`. Il n'y a **qu'une** violation dans
   tout le dépôt.
2. **Cette unique violation est réparée par le livrable 1 du même vol.** Une allowlist
   non vide serait une exception pour une donnée qui n'existe plus à la fin du vol —
   exactement le genre d'entrée dont mika#2309 dit qu'elle fait passer la régression
   suivante inaperçue.
3. **Précédent de la même forme dans le dépôt** : `verify_bundled_skills.rs` porte
   `known_exceptions_ship_empty`, un test qui épingle *par valeur* que son allowlist est
   vide. La propriété « cet invariant n'a aucune exception » y est une assertion, pas un
   silence.
4. **L'assertion auto-nettoyante exigée par la règle est l'assertion de population non
   vide** (`declared > 0`). Son rôle est symétrique à celui d'une allowlist
   auto-nettoyante : elle rougit le jour où le détecteur cesse de compter quoi que ce
   soit — disparition du dernier `[context.*]`, régression de la découverte de
   `build.rs`, renommage du fichier de prompt — c'est-à-dire exactement le jour où un
   vert cesserait de vouloir dire « aucune violation ». Sans elle, un détecteur à
   population vide est vert par construction, ce que la règle de grooming nomme comme le
   défaut à éviter.
5. **L'ordre de commit du § 4 est la preuve que le détecteur détecte.** Il atterrit
   **armé**, rougit sur l'état réel du dépôt, et ne verdit que par la correction. Ce
   n'est donc ni l'option (b) — rien n'est `#[ignore]` — ni l'option (c).

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review/system_prompt.md` (l. 67 réécrite) | 1 |
| détecteur + doc-comment (`crates/mika-agent/src/bundled_skills.rs`, `mod tests`) | 78 |

Total estimé : 79 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 79 lignes contre une borne par vol de 429
et un seuil par PR de 1 000 : les deux bornes sont franches, et la plus serrée (429) est
dépassée par un facteur 5. Le périmètre est fermé par construction — **deux fichiers,
une ligne de prompt et un test** — et il n'y a rien à découper : le détecteur sans la
correction laisserait la branche rouge, et la correction sans le détecteur serait la
troisième copie d'une règle qui a déjà dérivé, ce que le corps du ticket refuse
explicitement. Le coût de ce vol n'est pas dans les lignes mais dans la **mesure** : le
`wc -c` face au gate à 61 octets de marge, et la capture de la sortie rouge au commit 1.
Les deux sont des commandes, pas du code.

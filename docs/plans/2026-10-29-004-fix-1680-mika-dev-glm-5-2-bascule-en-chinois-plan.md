---
title: "mika-dev répond en chinois — la consigne manquante et le filet qui la mesure"
description: "Plan de mika#1680 : poser l'axe du script latin dans le prompt des deux constructeurs conversationnels, et un garde EndTurn 5i qui rougit sur la réponse chinoise mesurée du 2026-10-29."
---

**Ticket :** [mika#1680](https://github.com/senara-solutions/mika/issues/1680) — `fix`, p2
**Branche :** `fix/1680/mika-dev-glm-5-2-bascule-en-chinois-sur`
**Base mesurée :** `8d127552e` (`origin/main` au 2026-10-29)
**Lié :** mika#2247 (garde de dérive de langue des tenants), mika#1925 AC4 (le carve-out MikaModel), mika#2120 / `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (une consigne de prompt ne lie pas à travers les classes de modèles), mika#2357 (la racine glm-5.2, qui dort), mika#764 (`scripts/check-byte-slices.sh`)

---

## 1. Ce qui est mesuré

### 1.1 La mesure du 2026-10-29

Table `messages` de `~/.mika/data/mika.db`, `agent_id='mika-dev'`, `role='assistant'` :
**5 réponses sur 15 sont entièrement en chinois** — 12:55:17Z, 13:10:11Z, 13:46:27Z,
14:09:58Z, 14:10:05Z. Des accusés de webhook, des no-op, un compte rendu de CI. **Le
contenu est juste ; seule la langue change.** Le modèle est `z-ai/glm-5.2` via openrouter.

Le spécimen que ce plan prend pour fixture, celui de 14:09:58Z, en réponse à
`mika ask --agent mika-dev "groom mika#2959"` :

```
已分派 mika#2959 的梳理任务。正在等待通过回调获取架构师评审结果（任务：3c94d788-6e36-4a7c-8fe1-59369c8c92c8）。
```

Deux faits de forme que la suite exploite : le texte est **majoritairement** idéographique
(≈ 30 caractères de la plage U+4E00–U+9FFF contre ≈ 24 caractères latins, tous dans les
identifiants techniques `mika#2959` et l'UUID), et sa ponctuation est elle-même CJK
(`。`, `（`, `）`) — mais la ponctuation ne servira **pas** de prédicat, pour la raison du § 2.4.

### 1.2 Rien ne pose la langue, et ce n'est pas une omission locale

Relevé sur `origin/main` le 2026-10-29 : aucune consigne de langue dans le prompt de
mika-dev, ni dans les skills. Ce n'est pas qu'une consigne ait été retirée — il n'y en a
jamais eu. Le prompt système décrit un provider, un modèle, un hébergement, une persona ;
**il ne dit rien du script dans lequel la réponse doit être écrite**, parce que jusqu'à
glm-5.2 aucun modèle servi n'avait de raison d'en changer.

### 1.3 AC1 — le garde de mika#2247 ne se réutilise pas, et pour trois raisons indépendantes

`detect_response_language_drift` (`crates/mika-agent/src/evidence/guards.rs`) existe, et
c'est le bon voisin ; mais il ne peut pas couvrir mika-dev. Chacune des trois raisons
suffit seule, et aucune ne se répare par un réglage :

1. **Il exige une langue *déclarée*, et rien ne la déclare pour mika-dev.** Son corps
   s'ouvre sur `let expected = expected?;` — *« un tenant non déclaré n'est pas gardé : il
   n'y a pas de fait dont dériver »*. La valeur vient de la clé `customer_config`
   `language` (`config_keys.rs`), dont le point 4 de la doc dit en propres termes que
   **rien n'émet `MIKA_TENANT_LANGUAGE`** et que poser la variable est un geste
   `mika-cloud`, hors de ce dépôt. Armer mika#2247 sur mika-dev demanderait d'écrire une
   ligne de config dans un autre dépôt pour obtenir un garde qui, par les points 2 et 3,
   ne tirerait de toute façon pas.
2. **Son domaine de verdict ne connaît pas le chinois, et son seuil le fait échouer en
   ouverture.** `LanguageVerdict` vaut `French | English | Undetermined`, et la mesure
   compte des **mots-fonction** d'une liste fermée (`le`, `la`, `de`, … / `the`, `a`,
   `of`, …) avec `LANGUAGE_MIN_HITS = 3`. Un texte intégralement chinois produit des
   tokens — les idéographes sont `char::is_alphabetic()` — mais **zéro** correspondance
   dans les deux listes, donc `Undetermined`, donc `None`. Le texte du § 1.1 est le pire
   cas possible pour ce prédicat : il échoue en ouverture *par construction*, pas par
   réglage de seuil.
3. **Sa correction nomme une valeur qui n'existe pas.** Le message de re-prompt oriente
   vers `set_config language = <detected>`, et `TenantLanguage::parse` n'accepte que `fr`
   et `en`. Il n'y a **pas de `zh`** à nommer. Un garde dont la correction prescrit un
   geste impossible ne corrige rien.

**Conséquence qui décide tout le plan :** la bonne grandeur n'est pas la **langue**, c'est
le **script**. Le domaine déclarable de mika#2247 — `{fr, en}` — est entièrement latin.
Donc *« écrire en caractères latins »* est vrai pour **tout** tenant déclarable **et** pour
un agent qui n'a rien déclaré. Un garde de script n'a besoin d'aucune déclaration pour
s'armer, ce qui est exactement l'inertie que le corps du ticket pointe chez mika#2247.

---

## 2. La décision (AC1) : les deux couches, sur l'axe du script

**Les deux**, et chacune pour une raison que l'autre ne couvre pas.

- La **consigne de prompt** est la seule qui obtienne la bonne réponse **du premier coup**.
  Un garde ne fait pas écrire mieux ; il fait écrire deux fois. Sur 5 réponses sur 15,
  cela vaut un tour LLM supplémentaire une fois sur trois.
- Le **garde moteur** est la seule chose qui **mesure** ce que la consigne obtient.
  mika#2120 a déjà établi que l'exécution d'une consigne ne lie pas à travers les classes
  de modèles, et c'est précisément un changement de classe de modèle (`glm-5.2`) qui a
  produit la mesure. Livrer la consigne seule, c'est reposer la même instruction une
  troisième fois en espérant qu'elle tienne ; **un fait posé par le code bat une
  troisième formulation d'une consigne.**

### 2.1 Couche 1 — une ligne dans `## Runtime`, à un seul site

Dans `crates/mika-agent/src/prompt.rs`, `write_runtime_section` est appelée depuis
`build_system_prompt` (l. ~2222) **et** `build_silent_prompt` (l. ~3031). Un seul site
d'écriture couvre donc les deux modes, et la couverture est exactement celle que la mesure
demande : les cinq réponses chinoises sont des accusés de webhook, des no-op et un compte
rendu de CI (mode silencieux) **et** une réponse à `mika ask` (mode conversationnel).

Nouvelle fonction privée `write_latin_script_line(prompt: &mut String)`, appelée par
`write_runtime_section` **inconditionnellement** — il n'y a pas de condition à poser :
l'énoncé est vrai pour tout tenant déclarable comme pour un agent non déclaré (§ 1.3).
Elle se place à côté de `write_thread_language_line`, qui reste intacte : les deux lignes
sont orthogonales et disent deux choses différentes (*dans quelle langue converser* vs
*dans quel script écrire*), et la seconde rend quelque chose même quand la première rend
le vide.

Texte, en **ASCII strict de bout en bout** — pas de tiret cadratin, pas de guillemet
typographique : la doc de la fonction voisine le prescrit, parce qu'un caractère de
ponctuation posé dans un prompt est recopié verbatim dans ce que le lecteur lit.

```
Write your responses in the Latin script. The people who read this agent's output do not
read Chinese, Japanese or Korean, and the only declared conversation languages are `fr`
and `en` - both Latin script. If you must name a CJK string that is itself data (a
filename, a test fixture), keep it inside backticks and write the sentence around it in
the Latin script.
```

La dernière phrase n'est pas de la politesse : c'est ce qui empêche la consigne de rendre
le modèle incapable de citer une donnée légitime, et c'est le même contrat que le contrôle
négatif du § 2.5.

**`build_compact_system_prompt` (`ProviderKind::MikaModel`) ne la rend pas, et c'est un
refus, pas un report.** mika#1925 AC4 est une précondition bloquante non satisfaite : il
n'existe aucun endpoint MikaModel, et le constructeur compact existe pour éviter un mode
de complétion hors distribution dont la cause est la **forme** (les titres markdown), pas
la masse. Le refus est **asserté** par le test du § 4, pas laissé à la mémoire d'un
futur éditeur.

### 2.2 Couche 2 — le garde EndTurn `5i`

Dans `crates/mika-agent/src/agent_loop/mod.rs`, la chaîne tient aux positions
`5e`@2992, `5f`@3180, `5g`@3284, `5h`@3376. **`5i` est le prochain emplacement libre, et
le garde s'y ajoute en queue plutôt qu'à côté de `5f`** — ce qui serait plus joli à lire,
et coûterait la renumérotation de `5g` et `5h` sur huit sites. L'ordre est **observablement
neutre** ici, pour deux raisons : le lexique de chaque garde antérieur est latin, donc un
texte intégralement chinois n'en déclenche aucun ; et si un garde de ligne-suffixe tire
d'abord, les budgets sont **par étiquette**, donc `5i` tire au tour suivant sans rien
perdre.

Le site reproduit le patron de `5f` à l'identique, parce que c'est ce patron qui rend le
garde lisible par les instruments existants :

- condition : `matches!(response.stop_reason, LlmStopReason::EndTurn)`
  `&& !intent_guard_retries.contains(RESPONSE_CJK_SCRIPT_LABEL)`
  `&& let Some(m) = detect_cjk_script_response(&text)` ;
- budget d'**un seul** re-prompt, par insertion de l'étiquette dans `intent_guard_retries` ;
- `GuardCorrelation { correlation_id: "<trace_id>:<step>:response_cjk_script", guard_label: "response_cjk_script", step }` ;
- `warn!(target: "mika::otel", …, event = "guard.response_cjk_script")` portant
  `ideographs`, `latin`, `share_pct`, `guard_correlation_id`, `label = mode.label()` ;
- le tour de l'assistant repoussé dans `request.messages`, puis le message de correction
  `[mika-engine]`, puis `continue`.

**Le résidu se nomme.** Comme `5f`, un second `warn!` d'étiquette
`event = "guard.response_cjk_script_uncorrected"` est émis quand le budget est dépensé et
que le texte final viole encore le prédicat. Sans cette ligne, « le garde a corrigé » et
« le garde a abandonné » rendent les mêmes octets, et l'AC4 reste une affirmation au lieu
d'être une mesure.

**Portée, reprise de la doc de `5h` et pas élargie :** le garde lit **le texte final de
l'assistant du tour**, jamais l'entrée de `send_message`. Un texte chinois passé en
argument d'outil n'est pas vu. C'est une borne, elle est nommée au § 3.

Message de correction, ASCII strict :

```
[mika-engine] Your response is written in Chinese ideographs ({ideographs} of
{ideographs}+{latin} script characters, {share_pct}%). The operator who reads this
agent's output does not read that script, and the only declared conversation languages
of this platform are `fr` and `en` - both Latin script.

Offending excerpt: {excerpt}

Rewrite your response now in the Latin script. Keep the substance exactly as it is - the
content of your previous answer was correct, only its script is wrong. If you must name a
CJK string that is itself data (a filename, a test fixture), keep it inside backticks and
write the sentence around it in the Latin script.
```

La deuxième phrase du dernier paragraphe est là parce que le défaut mesuré est *de langue,
pas de contenu* : sans elle, un modèle re-prompté a toutes les raisons de reconstruire sa
réponse et d'y perdre le fait juste qu'elle portait.

### 2.3 Le détecteur

Dans `crates/mika-agent/src/evidence/guards.rs`, à côté de `detect_response_language_drift`
dont il est le voisin thématique :

```rust
pub(crate) const RESPONSE_CJK_SCRIPT_LABEL: &str = "response_cjk_script";

const CJK_MIN_IDEOGRAPHS: usize = 5;
const CJK_MIN_SHARE_PCT: usize = 30;
const CJK_EXCERPT_CHARS: usize = 80;

pub(crate) struct CjkScriptMatch {
    pub(crate) ideographs: usize,
    pub(crate) latin: usize,
    pub(crate) share_pct: usize,
    pub(crate) excerpt: String,
}

fn is_cjk_ideograph(c: char) -> bool { ('\u{4E00}'..='\u{9FFF}').contains(&c) }

pub(crate) fn detect_cjk_script_response(text: &str) -> Option<CjkScriptMatch>
```

Prédicat : il tire **si et seulement si** `ideographs >= CJK_MIN_IDEOGRAPHS`
**et** `ideographs * 100 >= CJK_MIN_SHARE_PCT * (ideographs + latin)`, où `latin` compte
les `char::is_alphabetic()` **hors** de la plage. La division est écrite en produit
croisé : pas de division, donc pas de division par zéro à raisonner, et l'ordre des deux
conjonctions garantit de toute façon un dénominateur non nul.

`excerpt` : la fenêtre de `CJK_EXCERPT_CHARS` caractères qui commence au **premier**
idéographe, obtenue par
`text.chars().skip(first).take(CJK_EXCERPT_CHARS).collect::<String>()`. **Jamais
`&text[..n]`, jamais `.truncate(`, jamais `.len().min(` :** `scripts/check-byte-slices.sh`
(mika#764, job CI `byte-slice-lint`) refuse ces trois formes, et sur un texte idéographique
elles paniqueraient de toute façon au premier caractère multi-octets.

### 2.4 Le seuil, et ce qu'il refuse de refuser

La plage est **exactement** U+4E00–U+9FFF — CJK Unified Ideographs, ni plus ni moins.
C'est la plage que l'AC4 mesure, et c'est délibéré : **le garde et la sonde doivent compter
la même population**, sinon la cible 0 de l'AC4 ne dit rien du garde.

La ponctuation CJK (U+3000–U+303F : `。`, `（`, `、`) est **hors** prédicat, malgré sa
présence dans la fixture. Le défaut mesuré en 2026-10-29 est une réponse **en chinois**,
pas une réponse anglaise décorée — c'est la correction que le commentaire du 2026-06-30
14:53Z avait déjà apportée au cadrage d'origine de ce ticket, et y revenir rouvrirait la
classe « police de la ponctuation » que la reformulation avait fermée.

Les deux seuils ensemble, vérifiés à la main sur trois textes :

| texte | idéographes | latins | part | tire ? |
|---|---|---|---|---|
| la réponse de 14:09:58Z (§ 1.1) | ≈ 30 | ≈ 24 | ≈ 55 % | **oui** |
| « the fixture lives in `任务.md`, see above » | 2 | ≈ 30 | — | non (plancher) |
| « In Chinese the engine confirmation reads 引擎已确认 » | 5 | ≈ 40 | ≈ 11 % | non (part) |

Le plancher de 5 seul laisserait passer la troisième ligne ; la part de 30 % seule
laisserait passer un texte de trois idéographes et rien d'autre. **Les deux bornes
s'appliquent, et la plus serrée mord** selon la longueur du texte.

### 2.5 Ce que la plage laisse passer, nommé plutôt que tu

Hiragana, katakana et Hangul sont **hors** de U+4E00–U+9FFF, donc hors du garde. Ce n'est
pas un oubli : rien ne les a mesurés, et un prédicat qui refuse une population dont on n'a
aucune occurrence est un prédicat dont on ne saura pas s'il a des faux positifs. La borne
est posée au § *Hors périmètre* ; la consigne de prompt, elle, les nomme (« Chinese,
Japanese or Korean »), parce qu'une consigne ne coûte rien à élargir et qu'un prédicat,
oui.

---

## 3. Ce que le plan ne touche pas

- **Le modèle de mika-dev.** Le swap glm-5.2 est hors périmètre depuis le corps d'origine,
  et sa racine — les tours `callback-*` muets sous glm-5.3 — est **mika#2357, qui dort**
  avec sa propre condition de réveil. Ce plan traite l'hygiène de sortie, pas le choix du
  modèle.
- **La suite de calibration.** Le commentaire du 2026-06-30 20:22Z proposait une
  assertion de pureté de script dans `calibration/roles/mika_dev.rs`. Elle reste
  pertinente, et ce n'est pas ce ticket : la calibration mesure un modèle **avant** le
  swap, le garde mesure la production **après**. Les deux sont dus ; ce vol livre celui
  dont la population est mesurée.
- **Les autres agents.** Le garde est dans `run_loop` et la ligne de prompt dans
  `write_runtime_section` : les deux sont **globaux par construction**, donc tout agent en
  bénéficie sans qu'une ligne nomme mika-dev. Mais aucune mesure n'existe pour mika-qa ou
  mika-arch, et aucune AC ne les cite — rien n'est à vérifier de leur côté dans cette PR.
- **`send_message` et les arguments d'outils** (§ 2.2), **la ponctuation CJK** (§ 2.4),
  **les kana et le Hangul** (§ 2.5).
- **`MIKA_TENANT_LANGUAGE` et la clé `language`** : aucune valeur nouvelle, aucun `zh`
  ajouté à `TenantLanguage::parse` (§ 1.3, raison 3). L'axe du script ne passe pas par
  l'axe de la langue, c'est le fond de la décision.
- **Aucune variable d'environnement nouvelle** — donc rien à écrire dans
  `docs/operator/environment-variables.md`, et **aucun `CLAUDE.md` touché** : le plafond
  de taille asserté par `crates/mika-common/tests/claude_md_size.rs` n'est pas approché.

---

## 4. Livrables

**L1 — `crates/mika-agent/src/evidence/guards.rs`** : `RESPONSE_CJK_SCRIPT_LABEL`, les
trois `const`, `CjkScriptMatch`, `is_cjk_ideograph`, `detect_cjk_script_response`. Le
doc-comment porte (a) les trois raisons du § 1.3, en une phrase chacune, parce qu'un futur
éditeur posera forcément la question « pourquoi pas mika#2247 ? » et que la réponse n'est
pas devinable ; (b) la raison de la plage exacte (parité avec la sonde de l'AC4).

**L2 — `crates/mika-agent/src/agent_loop/mod.rs`** : le garde `5i`, son bloc de résidu, et
l'extension du bloc `use crate::evidence::guards::{…}` (l. 22–35) avec
`RESPONSE_CJK_SCRIPT_LABEL` et `detect_cjk_script_response`. Le doc-comment du site porte
la borne de portée du § 2.2 et la raison de l'ajout en queue plutôt qu'en `5g`.

**L3 — `crates/mika-agent/src/prompt.rs`** : `write_latin_script_line` et son appel depuis
`write_runtime_section`.

**L4 — le test de l'AC2**, dans le `mod tests` inline de `guards.rs` :
`mika1680_le_detecteur_rougit_sur_la_reponse_chinoise_mesuree_et_pas_sur_langlaise`. Un
seul `#[test]`, comme l'AC2 l'exige, avec trois cas :

1. **La fixture chinoise** — le texte verbatim de 14:09:58Z transcrit au § 1.1 du corps du
   ticket. `detect_cjk_script_response` rend `Some`, et le test assère aussi
   `share_pct >= CJK_MIN_SHARE_PCT` : un `Some` seul ne dirait pas *par quelle* borne.
2. **Le contrôle anglais mesuré.** Rend `None`.
3. **Le contrôle de citation légitime** — une phrase anglaise nommant un fichier CJK entre
   backticks (ligne 2 du tableau du § 2.4). Rend `None`. C'est **lui** qui porte la valeur
   de régression : il est le seul à rougir si quelqu'un abaisse le plancher ou retire la
   part, et c'est aussi la propriété que promet la dernière phrase du prompt (§ 2.1).

**Provenance de la fixture anglaise, et pourquoi elle est écrite ici plutôt que découverte
par l'implémenteur.** L'AC2 demande « une réponse anglaise **mesurée** de la même table ».
Vérifié le 2026-10-29 : `~/.mika/data/` ne monte que `pilot-transcripts` dans le bac à
sable de dispatch — **`mika.db` n'y est pas atteignable**, donc l'implémenteur ne peut pas
extraire la ligne lui-même. La séquence est donc :

- **tenter** d'abord, au cas où l'environnement d'exécution diffère :
  `sqlite3 ~/.mika/data/mika.db "SELECT content FROM messages WHERE agent_id='mika-dev' AND role='assistant' AND content NOT GLOB '*[一-鿿]*' ORDER BY created_at DESC LIMIT 1;"` ;
- **à défaut**, et c'est la voie attendue : l'accusé **anglais mesuré le 2026-06-30 à
  14:48:48Z**, transcrit verbatim dans le corps de ce ticket par operator-CC le même jour
  (bloc de l'UPDATE 14:53Z, « Engine confirmed: `issue_comment.created` webhook is not
  authorized for dispatch (mika#933)… »). C'est une réponse de `mika-dev`, `role`
  `assistant`, de la même table — mesurée, pas inventée.

**Le doc-comment du test déclare laquelle des deux voies a servi, et le corps de PR le
répète.** Une fixture dont la provenance n'est pas écrite devient, en six mois, une
fixture inventée : c'est ce qui sépare un test de régression d'un test de confort.

**L5 — le test de forme du prompt**, dans le `mod tests` de `prompt.rs` :
`mika1680_la_ligne_de_script_latin_est_posee_dans_les_deux_constructeurs_et_refusee_dans_le_compact`.
Trois assertions : présente dans `build_system_prompt`, présente dans
`build_silent_prompt`, **absente** de `build_compact_system_prompt`. La troisième est ce
qui fait du carve-out mika#1925 AC4 un refus asserté au lieu d'une intention (§ 2.1).

**L6 — `docs/dormeurs.md`** : suppression de la ligne `#1680` (l. 41). **AC3.** C'est la
seule ligne retirée ; le § *Réveil* du même fichier prescrit exactement ce geste, et le
ticket est déjà rouvert.

**L7 — `docs/agent-internals/observability.md`** : deux lignes dans les tables d'événements
de garde, au voisinage immédiat de `guard.response_language_drift` (~480) et de
`guard.response_language_drift_uncorrected` (~499), avec les champs du § 2.2. Un garde dont
l'événement n'est pas dans la table est un garde qu'aucun opérateur ne pensera à grepper.

---

## 5. Ordre de commit

Un seul vol (§ *Taille estimée*), trois commits pour que chacun soit lisible seul :

1. `fix(1680): le detecteur de script CJK et sa parite avec la sonde (mika#1680)` — L1 + L4.
   Le détecteur et son test atterrissent ensemble : un prédicat sans sa fixture mesurée
   n'est pas revu, il est cru.
2. `fix(1680): la consigne de script latin dans les deux constructeurs conversationnels (mika#1680)` — L3 + L5.
3. `fix(1680): le garde 5i, son residu nomme, et le registre des dormeurs (mika#1680)` — L2 + L6 + L7.

**`scripts/sync-agent-docs.sh` n'a rien à faire ici, et c'est vérifié plutôt que supposé.**
Le job CI `docs-sync` relance le script et exige un diff vide ; le script ne copie que
**neuf** fichiers nommés dans sa liste `DOCS` (`architecture.md`, `browser-control.md`,
`configuration.md`, `deployment.md`, `getting-started.md`, `runtime-structure.md`,
`skills.md`, `slash-commands.md`, `task-system.md`) plus `openapi/mika-spirit.yaml`. Ni
`dormeurs.md` ni `agent-internals/observability.md` n'y figurent, donc les éditions L6 et
L7 ne produisent aucune désynchronisation. Le lancer reste gratuit et sans effet ; si son
diff est non vide, c'est qu'un **autre** fichier a bougé et il faut le committer.

---

## 6. Vérification

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p mika-agent evidence::guards
cargo test -p mika-agent prompt::
cargo test -p mika-agent
scripts/check-byte-slices.sh
scripts/verify-pipeline.sh origin/main
```

Trois attentes qui ne se lisent pas dans la sortie :

- `scripts/check-byte-slices.sh` **doit** passer sans `// safe-byte-slice:` ajouté. Si une
  dérogation devient nécessaire, c'est que l'extrait n'a pas été écrit en `.chars()` (§ 2.3)
  — on répare le site, on ne l'excepte pas.
- `suite-wiring-lint` reste **sans objet** : aucun script shell n'est livré, donc aucune
  entrée dans `scripts/suite-wiring.tsv`.
- `cargo test -p mika-agent` entier, pas seulement les deux modules : le garde `5i`
  s'ajoute à une chaîne dont plusieurs tests comptent les positions ou les étiquettes.

---

## 7. Sonde après déploiement (AC4)

La ligne de prompt vit dans le binaire et le garde dans `run_loop` : **ni l'un ni l'autre
n'est vivant avant `make deploy`.** Une mesure prise avant ne dit rien du correctif
(motif mika#2340 : l'absence d'une ligne signale un binaire antérieur au changement, jamais
une absence du phénomène).

Vingt-quatre heures pleines après le déploiement, sur `~/.mika/data/mika.db` :

```sql
SELECT COUNT(*) FROM messages
WHERE agent_id = 'mika-dev'
  AND role = 'assistant'
  AND created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-24 hours')
  AND content GLOB '*[一-鿿]*';
```

**Cible : 0.** Le compte et la requête qui l'a produit s'écrivent dans le ticket.

Deux pièges de cette requête, tous deux capables de rendre un zéro faux :

- **Le prédicat de temps.** `created_at` est du TEXT ISO 8601 `%Y-%m-%dT%H:%M:%SZ` ; un
  `datetime('now','-24 hours')` rendrait `YYYY-MM-DD HH:MM:SS` (espace, pas de `Z`), que
  la comparaison lexicographique place **avant** toute valeur réelle — la fenêtre
  n'existerait pas et le compte porterait sur toute la table. D'où le `strftime` explicite.
- **La borne haute du GLOB.** `[一-鿿]` est U+4E00–U+9FFF, la plage que le détecteur mesure
  (§ 2.4). Le GLOB historique du corps du ticket, `[一-龥]`, s'arrête à **U+9FA5** et laisse
  passer 90 codepoints. La différence est exactement celle entre une sonde qui mesure le
  garde et une sonde qui mesure autre chose.

Et deux lectures à faire **en plus** du compte, parce que le compte seul a deux causes de
signe opposé :

```
grep 'guard.response_cjk_script' "$MIKA_SPIRIT_LOG_FILE"
grep 'guard.response_cjk_script_uncorrected' "$MIKA_SPIRIT_LOG_FILE"
```

- Compte 0 **et** aucune ligne `guard.response_cjk_script` : la consigne de prompt tient
  seule — le meilleur résultat possible, et celui qui rend le garde silencieux sans le
  rendre inutile.
- Compte 0 **et** des lignes `guard.response_cjk_script` sans
  `_uncorrected` : la consigne ne lie pas, le garde rattrape. Conforme à mika#2120 ; le
  garde est ce qui reste.
- **Des lignes `_uncorrected`** : le re-prompt ne suffit pas sur cette population. C'est la
  condition d'escalade que la doc de mika#2247 pose déjà en propres termes — le remède
  serait un filet moteur de forme mika#2368, **et cela est un ticket, pas un réglage.**
- **L'absence de toute ligne de garde avec un compte non nul** ne se lit pas comme « le
  garde ne tire pas » : c'est d'abord un binaire servi antérieur au vol. Vérifier par
  `mika agents budget --agent mika-dev` et par le sha du déploiement avant de conclure.

---

## Definition of Done

- `detect_cjk_script_response` existe, ses deux seuils sont des `const` nommées, sa plage
  est exactement U+4E00–U+9FFF, et son extrait est construit en `.chars()`.
- Le garde `5i` est câblé dans `run_loop` avec son budget d'un re-prompt, sa
  `GuardCorrelation`, son `event = "guard.response_cjk_script"` et son résidu nommé.
- `write_latin_script_line` est posée dans `write_runtime_section`, donc rendue par
  `build_system_prompt` et `build_silent_prompt`, et absente du constructeur compact.
- Les deux tests (L4, L5) passent ; le contrôle de citation légitime et l'assertion
  d'absence dans le compact sont présents.
- La ligne `#1680` ne figure plus dans `docs/dormeurs.md`.
- Les deux événements sont dans les tables de `docs/agent-internals/observability.md`.
- `cargo fmt`, `cargo clippy -D warnings`, `cargo test -p mika-agent`,
  `scripts/check-byte-slices.sh` et `scripts/verify-pipeline.sh origin/main` sont verts.
- Le corps de PR porte la provenance de la fixture anglaise et la requête de l'AC4.

## Acceptance criteria

- **AC1.** Le plan tranche la forme : consigne de prompt, garde moteur, ou les deux. Il s'appuie sur la mesure ci-dessus, et cite mika#2247 pour dire si son garde se réutilise.
- **AC2.** Si un garde moteur est retenu : un test rougit sur une réponse chinoise **mesurée** (l'exemple ci-dessus) et reste vert sur une réponse anglaise mesurée de la même table, dans le même test.
- **AC3.** La ligne de #1680 est retirée de `docs/dormeurs.md` dans la même PR.
- **AC4.** Sonde après déploiement : sur 24 h de la table `messages` (agent mika-dev, rôle assistant), le compte des réponses qui contiennent un caractère de la plage U+4E00 à U+9FFF est écrit dans ce ticket, avec la requête qui l'a produit. La cible est 0.

**Où chacun est fermé.** AC1 → § 1.3 (les trois raisons de non-réutilisation de mika#2247)
et § 2 (les deux couches, avec la raison propre de chacune). AC2 → L4, un `#[test]`
portant les trois cas, et le § *Provenance de la fixture anglaise* qui rend l'AC
exécutable depuis le bac à sable. AC3 → L6. AC4 → § 7, avec la requête, la parité de plage
avec le détecteur et les deux pièges qui fabriqueraient un zéro faux.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est vide par construction.**

Ce vol livre trois détecteurs : le garde `5i`, le test de l'AC2, le test de forme du
prompt. Aucun ne scanne de données versionnées — `detect_cjk_script_response` lit le texte
runtime d'un tour, et les deux tests lisent des fixtures écrites dans le même commit.
**Aucune `const` d'allowlist n'est donc livrée, parce qu'il n'existe aucune population
existante à excepter :** un fichier du dépôt n'est jamais soumis au prédicat. Les deux
tests atterrissent armés et verts, par construction.

La population à laquelle le détecteur s'applique vit **en production**, et elle est non
vide (5 sur 15 au 2026-10-29). Trois propriétés la rendent dénombrable et auto-expirante,
et c'est elles qui tiennent le rôle que l'assertion de péremption tient ailleurs :

1. Le garde **n'échoue jamais un tour** : il re-prompte une fois, puis laisse passer. Donc
   atterrir armé sur une population non vide ne casse rien — c'est un filet, pas un refus.
2. Le résidu `guard.response_cjk_script_uncorrected` **compte** ce qui survit au
   re-prompt. Un garde dont l'échec est muet est un garde qu'on croit efficace.
3. La sonde de l'AC4 vise **0** sur 24 h, avec la même plage de codepoints que le
   détecteur (§ 2.4). Quand la population est vide, les trois instruments rendent zéro
   d'eux-mêmes ; tant qu'elle ne l'est pas, ils la nomment. L'escalade, si le résidu est
   non négligeable, est **un ticket de forme mika#2368**, pas l'assouplissement d'un seuil
   — la doc de mika#2247 pose déjà cette règle en propres termes, et ce vol s'y range.

## Taille estimée

| livrable | lignes de code (hors docs/) |
|---|---|
| L1 — détecteur, `const`, type, doc-comment (`guards.rs`) | 70 |
| L2 — garde `5i`, résidu, import (`agent_loop/mod.rs`) | 80 |
| L3 — `write_latin_script_line` + appel (`prompt.rs`) | 24 |
| L4 — test AC2, trois cas (`guards.rs`) | 48 |
| L5 — test de forme, trois constructeurs (`prompt.rs`) | 22 |

Total estimé : 244
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 244 sous 279, marge de 35 lignes. La marge est petite,
mais le poste dominant — L2, 80 lignes — est une **recopie guidée** du patron de `5f`
(l. 3180–3282), dont la structure, les champs de `warn!` et le bloc de résidu existent déjà
et sont à côté : il n'y a pas de conception à y faire, seulement à substituer l'étiquette,
le prédicat et le texte de correction. L1 et L4 sont du prédicat pur avec trois cas déjà
calculés à la main au § 2.4 — l'implémenteur vérifie des nombres, il ne les cherche pas.
Aucun fichier à créer, aucune migration, aucun script, aucune variable d'environnement,
donc aucun des coûts fixes qui font déborder un vol : cinq sites dans trois fichiers
existants. L6 et L7 sont sous `docs/` et ne comptent pas.

L1 et L4 n'ont pas été fusionnés en un poste malgré leur fichier commun, parce qu'ils sont
au programme du **même** commit et que les séparer est ce qui rend visible la part de la
fixture — 48 lignes dont la moitié est du texte mesuré recopié.

## Hors périmètre

- Le modèle de mika-dev et sa racine (**mika#2357, dormeur**).
- L'assertion de pureté de script dans la suite de calibration — due, mais c'est un autre
  axe (§ 3).
- `send_message` et les arguments d'outils (§ 2.2).
- La ponctuation CJK U+3000–U+303F (§ 2.4), les kana et le Hangul (§ 2.5).
- Toute valeur nouvelle sur l'axe de la langue — pas de `zh` dans `TenantLanguage`, rien
  qui émette `MIKA_TENANT_LANGUAGE` (§ 1.3).
- `build_compact_system_prompt` / MikaModel : refus asserté sous mika#1925 AC4, pas un
  report (§ 2.1).
- La vérification des autres agents : le correctif les couvre par construction, aucune
  mesure ne les concerne (§ 3).

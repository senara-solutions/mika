---
ticket: senara-solutions/mika#1925
type: fix
date: 2026-09-20
seq: 001
---

# Le contrat de stop-signal atteint le chemin compact — et ce qui le rendait inerte est la moitié que le ticket ne demande pas — Plan

## Goal Capsule

`build_compact_system_prompt` (chemin `ProviderKind::MikaModel`) cesse d'être
muet sur le contrat de stop-signal de mika#1813. Les deux moitiés du contrat y
sont rendues : **consulter** (ne pas relancer sur un sujet listé) et
**persister** (poser le `store_fact(key='stop_topic_…')` quand l'utilisateur dit
stop). La seconde est celle que le ticket ne nomme pas et **sans laquelle la
première ne peut jamais firer** sur un tenant servi par MikaModel : mesure au §
Product Contract, D3.

La quatrième condition d'acceptation — une passe de calibration sur MikaModel —
**n'est pas exécutable dans ce dépôt** et n'est pas simulée. Elle est re-posée
comme précondition bloquante du jour où MikaModel sert un agent réel, inscrite
dans le code et dans `CLAUDE.md`, et sortie en ticket de suivi avec les trois
dépendances qu'elle porte. Ce qui est livré ici est déterministe, testable hors
réseau, et ne prétend rien mesurer qu'il ne peut mesurer.

## Product Contract

### Summary

mika#1813 a livré un correctif hybride état+prompt contre la sur-relance : une
préférence `stop_topic_*` est persistée quand l'utilisateur dit « arrête », puis
rechargée à chaque tour et rendue en bloc `<stopped-topics>` avec ses règles. Le
correctif a été câblé dans `build_system_prompt` (conversation) et
`build_silent_prompt` (silencieux). Pendant la revue de PR#1924, les deux
relecteurs ont relevé que `build_compact_system_prompt` — le troisième
assembleur, utilisé pour `ProviderKind::MikaModel` — n'était pas câblé : sur ce
chemin, le correctif est un no-op silencieux. Le carve-out a été posé comme
**décision** (commentaire au site, paragraphe dans `crates/mika-agent/CLAUDE.md`,
test d'épinglage `test_compact_prompt_omits_stopped_topics_block_by_design`).
Ce ticket est le câblage.

Quatre mesures faites sur le code déplacent le travail. Les trois premières
changent ce qu'il faut écrire ; la quatrième change ce qu'il est honnête de
promettre.

---

### D1 — Le budget d'octets n'est pas la contrainte qui mord. Le compte de sections et la forme le sont.

AC1 demande « une variante *size-capped* … Total prompt stays within the compact
budget », et « Why deferred » place la difficulté sur la pression de fenêtre de
contexte. La mesure dit autre chose.

Taille du prompt compact **aujourd'hui**, additionnée depuis les constantes du
source (fixture de `test_build_compact_system_prompt_size_bound`) :

| section | octets |
|---|---|
| `## Personality` + première ligne de `soul.md` | ≈ 64 |
| `## Identity` + `You are Mika.` | ≈ 27 |
| `## Runtime` (mika#1815) | ≈ 58 |
| `## Data-Grade Doctrine` compact (mika#1798) | ≈ 282 |
| **total** | **≈ 431** |

Plafond asserté : **5120**. Marge inutilisée : **≈ 91 %**. Le contrat complet de
mika#1813 en mode conversation — préambule de section + bloc + deux règles
d'`## Instructions` — pèse ≈ 1,6 Ko ; il tiendrait **trois fois** dans la marge.

Ce qui mord réellement est double, et aucun des deux termes n'est un octet :

1. **`section_count <= 4`**, asserté dans `test_build_compact_system_prompt_size_bound`.
   Les quatre places sont prises. Une cinquième section fait rougir le test —
   c'est la seule barrière qu'une addition naïve rencontre.
2. **Le mode de panne OOD**, documenté au plan d'origine
   (`docs/plans/2026-06-07-006-feat-1398-compact-prompt-builder-mikamodel-plan.md`) :
   servi du contexte complet, le fournisseur *« complète la structure markdown au
   lieu d'agir comme un agent (émet de faux rapports `## Summary / Completed
   Tasks / Pending` sans rapport avec la requête) »*. La fiction observée était
   faite de **titres markdown**. C'est la forme qui est dangereuse, pas la masse.

**Conséquence pour la rédaction du plan :** toute justification de ce travail
écrite en octets serait une justification fausse. La variante n'est pas rendue
courte pour tenir dans 5 Ko — elle y tenait déjà. Elle est rendue courte parce
que chaque ligne servie à ce fournisseur est une ligne qu'il peut décider de
compléter.

### D2 — La forme rendue est celle des deux autres assembleurs. On n'optimise pas contre un mode de panne qu'on ne peut pas mesurer.

Deux formes sont défendables et il faut dire laquelle et pourquoi.

- **(a) Miroir** — `## Stopped Topics` + préambule + bloc `<stopped-topics>`,
  identique en forme à `build_system_prompt` et `build_silent_prompt`, abrégé en
  contenu. Coût : une cinquième section, donc **exactement le geste que D1 nomme
  comme adjacent au risque mesuré**.
- **(b) Sans titre ni balise** — une ligne de prose appended après la doctrine.
  Coût : le prompt compact n'a **aucune** prose flottante aujourd'hui ; une règle
  sans titre, collée sous `## Data-Grade Doctrine`, se lit comme une clause *de
  cette doctrine*. Et elle introduit une forme que ni l'un ni l'autre des deux
  assembleurs de référence n'emploie — donc une troisième écriture d'un même
  contrat, sur le chemin le moins observé des trois.

**Décision : (a), le miroir.** Trois raisons, la plus forte en dernier.

1. Le compte de sections a un **précédent documenté de croissance** : le
   commentaire du test énumère la montée 2 → 4 en justifiant chaque addition
   comme « un garde-fou structurel que le fournisseur compact ne peut pas
   sauter ». La cinquième s'inscrit dans cette série, pas contre elle.
2. Trois assembleurs portant **une** forme est l'invariant le moins cher à
   tenir ; trois formes pour un contrat est ce qu'un futur éditeur casse sans
   s'en apercevoir (le précédent exact est `grooming_marker`, mika#2158 : une
   seconde écriture du même prédicat a divergé pendant des mois sans que rien ne
   rougisse).
3. **Surtout : le risque OOD est ici non mesurable.** Aucun endpoint MikaModel
   n'existe dans ce worktree (`default_base_url` = `http://localhost:11434`,
   transport Ollama, rien ne le sert). Choisir une forme inédite sur une
   intuition invérifiable, c'est troquer une forme connue-bonne contre une
   supposition — et le seul instrument capable de trancher est précisément AC4,
   que D4 ci-dessous établit comme non exécutable aujourd'hui. **Quand on ne peut
   pas mesurer, on ne s'écarte pas du connu.**

Ce que (a) coûte est nommé et non dissimulé : le bloc `<stopped-topics>` sera la
**première balise XML** rendue sur ce chemin. `sanitize_label` filtre déjà `<`,
`>`, `\n`, `\r` du contenu injecté, donc la balise ne peut pas être refermée par
une valeur hostile — le risque restant est de complétion, pas d'injection, et il
appartient à AC4.

### D3 — AC1 seul est inerte, et c'est le constat central de ce grooming.

AC1 conditionne le rendu à `stopped_topics` **non vide**. Sur un tenant servi par
MikaModel, cet état est **inatteignable**. Chaîne mesurée :

- `build_compact_system_prompt` ne rend **aucune** section `## Instructions` —
  asserté négativement par le test de borne existant. Donc la règle *persist*
  (« quand l'utilisateur dit stop, appelle
  `store_fact(category='preference', key='stop_topic_<slug>', …)` ») n'est servie
  sur aucun tour de conversation de ce fournisseur.
- `build_silent_prompt` porte la règle *consult* (« avant tout `send_message`
  proactif, vérifie le bloc `<stopped-topics>` ») et **jamais** la règle
  *persist*. Vérifié : l'unique occurrence de `store_fact(category='preference'`
  dans `prompt.rs` est dans `build_system_prompt`.
- Donc pour un agent dont les tours de conversation sont compacts : **aucun
  chemin, à aucun moment, n'instruit l'agent de persister un stop-signal.** La
  table `preferences` reste sans ligne `stop_topic_*`,
  `search_preferences(STOP_TOPIC_PREFIX)` rend le vide, et le bloc conditionnel
  d'AC1 ne se rend jamais.

Un tenant MikaModel ne peut avoir un `stopped_topics` non vide que par un
changement de fournisseur en cours de vie, ou par une écriture manuelle.

**La moitié porteuse est donc `persist`, que le ticket ne demande pas.** Livrer
AC1–AC3 seuls produirait une suite de tests verte au-dessus d'un chemin mort —
c'est-à-dire, à la lettre, la forme de défaut que ce ticket existe pour fermer
(« the fix silently no-ops »).

Le blocage est **uniquement** côté prompt : `store_fact` figure dans
`COMPACT_PROVIDER_CORE_TOOLS` (`agent_loop/mod.rs`, mika#1491), donc l'outil est
disponible sur ce chemin. Il manque l'instruction, rien d'autre.

**Décision : rendre les deux moitiés.** `persist` **inconditionnellement**
(c'est quand le bloc est vide que le premier stop se dit), `consult`
**conditionnellement** au bloc non vide, conformément à AC1. C'est un
élargissement d'AC1 tel qu'écrit, déclaré comme tel, avec sa raison.

Coût nommé : le prompt compact ne contient aujourd'hui **aucune instruction
d'invocation d'outil**. Il énonce des faits (`## Identity`, `## Runtime`) et une
prohibition (`## Data-Grade Doctrine` — « you may NEVER access nor propose »).
Une règle « appelle cet outil » est une classe nouvelle sur ce chemin. C'est
exactement ce qu'AC4 est là pour vérifier, et c'est une raison de plus pour que
la barrière de D4 soit écrite plutôt que contournée.

### D4 — AC4 n'est pas exécutable dans ce dépôt. Trois dépendances manquantes, nommées.

AC4 demande « a calibration pass on MikaModel confirms the addition does not
regress agent-mode responses (per mika#1190 model-swap discipline) ». Trois
constats indépendants :

1. **Aucun endpoint.** `ProviderKind::MikaModel.default_base_url()` rend
   `http://localhost:11434`. Rien dans ce worktree ni dans les cibles du
   `Makefile` ne sert ce modèle.
2. **Le harnais ne peut pas construire le provider.**
   `calibration::providers::create_real_provider` rend `None` pour tout
   fournisseur sans `MIKA_<PREFIX>_API_KEY`, avec une seule exemption :
   `kind != ProviderKind::Ollama`. `MikaModel` n'est pas exempté — alors que
   `config.rs` documente sa clé comme « optional; reserved for hosted-endpoint
   swap ». Il faut donc poser une clé dont le fournisseur n'a pas besoin pour que
   la calibration daigne l'instancier. Défaut réel, petit, séparable.
3. **Aucune suite de rôle ne mesure ce contrat.** Les quatre rôles existants
   (`mika_dev`, `mika_arch`, `mika_qa`, `mika_orchestrator`) sont des rôles
   d'ingénierie. Aucun n'exerce un agent conversationnel de tier famille, et
   aucun scénario n'exerce le contrat de stop-signal. Lancer l'un d'eux « sur
   MikaModel » mesurerait scrupuleusement autre chose.

Satisfaire AC4 demande donc : un endpoint, l'exemption de construction, et une
suite de rôle neuve avec ses scénarios stop-signal — un corps de travail
supérieur à AC1–AC3 réunis, et dont aucune partie n'est une modification de
prompt.

**Disposition : AC1–AC3 livrés ; AC4 re-posé comme précondition bloquante et
sorti en ticket de suivi.** Le ticket le dit lui-même à deux endroits — « No live
user-facing regression today » et « Escalate to `p1-important` when
Wizzard/MikaModel serves a live family-tier agent ». La calibration appartient à
cette escalade : il n'y a rien à régresser aujourd'hui, et exiger la mesure d'un
modèle qu'aucun tenant n'exécute serait une porte qui ne protège personne au prix
de bloquer le câblage.

**Ce qui n'est pas fait, et est dit comme tel :** AC4 n'est ni exécuté, ni
approximé par un mock. Un `MockLlmProvider` valide la *forme du prompt*, jamais
l'*obéissance du modèle* — le prétendre serait la vacuité de garde que mika#1701
a déjà dû retirer une fois. La barrière est inscrite en trois endroits (§ IU4)
pour que le jour de la bascule elle soit rencontrée plutôt que retrouvée.

### D5 — Les trois autres carve-outs restent fermés. Le statut de parapluie de mika#1925 est une question ouverte à trancher, pas à supposer.

`build_compact_system_prompt` porte aujourd'hui **quatre** carve-outs, tous
citant mika#1925 comme suivi, épinglés par **cinq** tests :

| carve-out | ticket | épinglage |
|---|---|---|
| bloc + règles stop-signal | mika#1813 | `test_compact_prompt_omits_stopped_topics_block_by_design` |
| `## Distribution Doctrine` | mika#1814 | AC9, `tests/eval/doctrine_regressions/doctrine_prompt_section_rendered.rs` |
| ligne d'hébergement | mika#2290 | `mika2290_compact_prompt_omits_the_hosting_line` |
| `## Mika Doctrine` | mika#2292 | `mika2292_compact_prompt_omits_the_doctrine_section` + AC7 eval |

Les conditions d'acceptation de ce ticket portent **strictement** sur le contrat
de stop-signal : AC1 nomme `stopped_topics`, AC2 la distinction AC2 de
mika#1813, AC3 le test d'épinglage de mika#1813.

**Décision : ne pas élargir.** Chaque carve-out porte une analyse de coût qui lui
est propre et qui n'est pas transposable — celui de mika#2292 est rédigé comme un
coût **accepté** (« le défaut mesuré reste ouvert … accepté parce que la
population mesurée n'est pas servie par ce chemin »), pas comme un report. Les
fermer ensemble supposerait que la même réponse vaut pour les quatre, ce que rien
n'établit.

**Question ouverte, posée et non tranchée ici :** trois tickets citent mika#1925
comme leur suivi, ce qui le lit comme un parapluie ; ses AC le lisent comme un
ticket de périmètre étroit. Les deux lectures ne peuvent pas être vraies.
Recommandation — **trois tickets frères**, un par carve-out, chacun portant son
propre arbitrage coût/bénéfice, et mika#1925 fermé sur son périmètre littéral.
C'est une décision d'opérateur ; le plan la nomme, la documente en IU3, et ne la
prend pas.

### Hors périmètre, délibérément

- **L'héritage des stop-signals en délégation d'équipe.** Le second site d'appel
  du prompt compact (`agent_loop/mod.rs`, branche team) enfile les
  `stopped_topics` de la DB de l'agent *enfant*, jamais de l'orchestrateur. Le
  câblage de D3 s'y applique mécaniquement et sans exception ; la question de
  savoir si les stop-signals de l'opérateur *devraient* descendre est ouverte
  sous **mika#1926** et n'est pas tranchée ici.
- **Le mode silencieux sur un agent MikaModel.** `build_silent_prompt` n'a pas de
  variante compacte : un tour silencieux d'agent MikaModel reçoit le prompt
  silencieux complet (> 4 Ko), ce qui contredit le raisonnement OOD de mika#1398.
  Comportement préexistant, séparable, non touché — **ticket de suivi**.
- **L'absence de règle *persist* dans `build_silent_prompt`** (D3). Elle est
  cohérente pour les autres fournisseurs — un tour silencieux n'entend pas un
  utilisateur dire « arrête » — donc ce n'est un trou que par composition avec le
  chemin compact, et c'est ce dernier que ce plan referme. Noté, non élargi.
- **Le plafond de sortie et le catalogue d'outils du fournisseur compact**
  (mika#1491). Intacts.

## Planning Contract

### Fix sites, épinglés verbatim

**A.** `crates/mika-agent/src/prompt.rs`, doc-comment de
`build_compact_system_prompt` — la phrase de carve-out à retirer (les trois
autres restent) :

```
/// **mika#1813 carve-out (tracked in mika#1925):** the stop-signal contract
/// (`<stopped-topics>` block + persist/consult rules) is intentionally NOT
/// rendered here — the compact budget cannot afford it and the MikaModel
/// provider is not currently used by family-tier or operator-tier agents in
/// production. mika#1925 tracks wiring a size-capped variant when MikaModel
/// goes live for real tenants. The `stopped_topics` field on `PromptContext`
/// is accepted by this builder to keep the type signature uniform across the
/// three builders; it is deliberately unused here.
```

**B.** `crates/mika-agent/src/prompt.rs`, fin du corps de
`build_compact_system_prompt` — le point d'insertion :

```rust
    write_data_grade_doctrine_section_compact(&mut prompt);

    prompt
}
```

**C.** `crates/mika-agent/src/prompt.rs`, le test à inverser (AC3) :

```rust
    #[test]
    fn test_compact_prompt_omits_stopped_topics_block_by_design() {
```

**D.** `crates/mika-agent/src/prompt.rs`, l'assertion de compte de sections et sa
justification documentée :

```rust
        let section_count = prompt.matches("## ").count();
        assert!(
            section_count <= 4,
            "compact prompt has {} sections, exceeds 4-section limit",
            section_count
        );
```

**E.** `crates/mika-agent/src/prompt.rs`, le second test de forme à ajuster
(compte exact avec soul vide) :

```rust
        assert_eq!(prompt.matches("## ").count(), 3);
```

**F.** `crates/mika-agent/CLAUDE.md:282`, le paragraphe « Stop-signal
convention » — la phrase de carve-out à remplacer :

```
**Compact-provider carve-out (mika#1925):** `build_compact_system_prompt` (used
for `ProviderKind::MikaModel`) does NOT render the block or rules — …
```

### Forme retenue pour le contenu rendu

Deux constantes nommées, modelées sur `DATA_GRADE_DOCTRINE_COMPACT` (le
précédent du fichier : une constante + une assertion de budget à la compilation
+ une fonction d'écriture d'une ligne).

- `STOP_SIGNAL_PERSIST_COMPACT` — inconditionnelle. Porte la règle *persist* avec
  la forme exacte de la clé (`stop_topic_<slug>`, kebab-case) et **la distinction
  AC2 en ligne** : une question directe sur un sujet stoppé n'est pas une
  réouverture. Budget dur ≈ 600 octets.
- Le bloc *consult* — conditionnel à `!ctx.stopped_topics.is_empty()`, titre
  `## Stopped Topics`, préambule abrégé portant **à nouveau** la distinction AC2,
  puis `<stopped-topics>` avec une ligne `- {category}: {value}` par préférence,
  passées par `sanitize_label` exactement comme dans les deux autres
  assembleurs. Préambule capé ≈ 300 octets ; le contenu injecté est borné par
  `sanitize_label` (200 caractères par champ).

**Pourquoi AC2 est portée deux fois** et non factorisée : c'est déjà le choix des
deux autres assembleurs (préambule de section *et* règle d'instructions), et les
deux moitiés se lisent à des moments différents — l'une quand l'utilisateur dit
stop, l'autre quand l'agent envisage de relancer. Un modèle compact ne va pas
chercher une clause trois sections plus haut. Le coût est de ≈ 80 octets sur une
marge de 4,7 Ko.

**Pourquoi des constantes et pas des `push_str` en ligne :** l'assertion de
budget à la compilation ne peut porter que sur une constante. C'est le mécanisme
qui garantit qu'une édition future qui fait déborder le budget **ne compile
pas** — le précédent est nommé au site de `DATA_GRADE_DOCTRINE_COMPACT`.

### Ce qui est asserté, et par quel genre de test

| propriété | genre | pourquoi celui-là |
|---|---|---|
| budget par constante | `const _: () = assert!(…)` | une régression de taille doit casser la compilation, pas un test qu'on peut ignorer |
| total ≤ 5 Ko, bloc non vide | test unitaire | AC1 |
| section absente quand le bloc est vide | test unitaire | le cas nominal doit rester byte-identique à aujourd'hui |
| règle *persist* présente **même** bloc vide | test unitaire | D3 : c'est l'assertion qui empêche la régression vers l'inertie |
| distinction AC2 présente dans les deux moitiés | test unitaire | AC2 |
| compte de sections = 5 avec soul, 4 sans | test unitaire | D1 : la barrière réelle, mise à jour avec sa justification |
| les trois autres carve-outs intacts | tests existants | D5 : ils doivent rester verts sans être touchés |

## Implementation Units

### IU1 — Rendre le contrat dans `build_compact_system_prompt`

`crates/mika-agent/src/prompt.rs`.

1. Déclarer `STOP_SIGNAL_PERSIST_COMPACT` près de `DATA_GRADE_DOCTRINE_COMPACT`,
   avec le doc-comment qui porte le raisonnement D3 (pourquoi inconditionnelle),
   et son `const _: () = assert!(…)` de budget.
2. Déclarer la constante de préambule *consult* avec son assertion de budget.
3. Au site **B**, après `write_data_grade_doctrine_section_compact` :
   - écrire inconditionnellement `STOP_SIGNAL_PERSIST_COMPACT` ;
   - si `!ctx.stopped_topics.is_empty()`, écrire `## Stopped Topics`, le
     préambule, puis le bloc `<stopped-topics>` en itérant avec
     `sanitize_label` sur `category` et `value` — même boucle, mot pour mot, que
     dans les deux autres assembleurs.
4. Au site **A**, retirer la phrase de carve-out mika#1813 et la remplacer par le
   commentaire qui **énonce le raisonnement D1/D2/D3** : que ce n'est pas un
   arbitrage d'octets, que la forme est le miroir délibéré des deux autres
   assembleurs, et que `persist` est inconditionnelle parce que conditionnelle
   elle serait inatteignable. Conserver **intacts** les trois autres carve-outs
   (#1814, #2290, #2292) et le renvoi qu'ils font à mika#1925.
5. Retirer de la signature la mention « deliberately unused here » pour
   `stopped_topics`.

**Invariant :** aucune modification de `build_system_prompt`, de
`build_silent_prompt`, ni du chargement en amont
(`agent_loop::load_agent_context` charge déjà `stopped_topics` **avant** la
branche compacte — aucun travail de câblage de contexte n'est nécessaire, c'est
vérifié au site d'appel `agent_loop/mod.rs:4280-4306`).

### IU2 — Inverser l'épinglage et rouvrir la barrière de sections (AC3, D1)

`crates/mika-agent/src/prompt.rs`, module `tests`.

1. Site **C** — remplacer `test_compact_prompt_omits_stopped_topics_block_by_design`
   par `mika1925_compact_prompt_renders_the_stop_signal_contract`, qui assère la
   **présence** : `## Stopped Topics`, `<stopped-topics>`, la catégorie injectée,
   la règle *persist*, et la distinction AC2. Le doc-comment du test dit qu'il
   est l'inversion d'un épinglage de carve-out, et nomme mika#1925 — pour qu'une
   lecture future n'y voie pas une assertion née de nulle part.
2. Ajouter `mika1925_compact_prompt_renders_persist_even_with_no_stopped_topics` —
   **c'est le test qui porte D3.** Bloc vide, règle *persist* présente, section
   `## Stopped Topics` absente. Sans lui, une « simplification » future qui
   remettrait `persist` sous la condition de non-vacuité restaurerait l'inertie
   sans faire rougir quoi que ce soit.
3. Ajouter `mika1925_compact_prompt_stays_within_budget_with_stopped_topics` —
   plusieurs préférences, valeurs longues, total ≤ 5120 (AC1).
4. Site **D** — porter la borne à 5 et **étendre le commentaire de
   justification** dans la forme déjà employée pour la croissance 2 → 4 :
   nommer la cinquième section, son ticket, et ce qu'elle garantit. Le
   commentaire est la seule chose qui distingue une croissance raisonnée d'une
   dérive.
5. Site **E** — soul vide : le compte passe de 3 à 3 (bloc vide, `persist` n'a
   pas de titre) ou 4 (bloc non vide). Ajuster le test existant et vérifier que
   la fixture porte bien `stopped_topics: &[]`, donc **3, inchangé**. Cette
   absence de changement est une propriété à asserter, pas un oubli à constater :
   le cas nominal reste byte-identique à aujourd'hui **plus** la règle `persist`.
6. Vérifier que les trois fixtures `PromptContext` d'autres tests touchant le
   prompt compact (lignes ≈ 4281, 5511, 5714, 6176) portent `stopped_topics: &[]`
   et restent vertes sans édition. **Si l'une d'elles doit être éditée, c'est
   qu'une assertion de carve-out sœur a été touchée — halte, relire D5.**

### IU3 — Documentation et question de périmètre (D5)

1. `crates/mika-agent/CLAUDE.md`, site **F** — remplacer la phrase de carve-out
   par l'état après ce ticket : les trois assembleurs rendent le contrat ; la
   variante compacte en porte une forme abrégée ; `persist` y est
   inconditionnelle et **pourquoi** (D3, en une phrase — c'est ce qu'un lecteur
   futur a besoin de savoir pour ne pas la « corriger »). Conserver la mention
   d'héritage team (mika#1926), inchangée.
2. Dans le même paragraphe, poser la **précondition AC4** : aucune bascule de
   MikaModel vers un agent réel sans la passe de calibration, avec les trois
   dépendances de D4 nommées. C'est la surface que lira l'opérateur du jour de la
   bascule.
3. Consigner la **question de périmètre D5** — trois tickets citent mika#1925
   comme parapluie, ses AC sont étroites — avec la recommandation de trois
   frères. Posée pour arbitrage, non tranchée.

### IU4 — Disposition d'AC4 : la barrière est écrite, la mesure est sortie

**Aucun code d'exécution.** Trois inscriptions plus un ticket.

1. Doc-comment du site **A** : la variante n'a pas été validée par une passe de
   calibration, et pourquoi elle ne pouvait pas l'être (les trois dépendances).
2. `CLAUDE.md` : IU3 point 2.
3. Ticket de suivi, à ouvrir, portant les trois dépendances de D4 — endpoint
   MikaModel, exemption de `create_real_provider` pour un fournisseur local sans
   clé, suite de rôle conversationnelle avec scénarios stop-signal — et lié au
   déclencheur d'escalade déjà écrit dans mika#1925 (« when Wizzard/MikaModel
   serves a live family-tier agent »).

**Le ticket de suivi n'est pas ouvert par ce pilote** (pas d'accès `gh`
authentifié dans la session de dispatch, et la création d'issue n'est pas dans le
contrat de sortie d'un groom `content-only`). Il est **nommé dans le plan et dans
la PR** pour que son ouverture soit un geste d'opérateur visible plutôt qu'une
intention perdue.

## Fire-Disposition

| unité | tire | ce qui rougit si l'unité manque |
|---|---|---|
| IU1 | toujours, au prochain tour compact | rien — c'est le no-op actuel, invisible par construction. C'est **pourquoi** IU2 est non négociable |
| IU2 | à chaque `cargo test -p mika-agent` | `test_compact_prompt_omits_stopped_topics_block_by_design` rougit dès IU1 posée : les deux unités sont indissociables dans un même commit |
| IU3 | à la lecture humaine | rien automatiquement — `CLAUDE.md` contredirait le code, classe mika#2340 inversée (la doc annonce un carve-out que le binaire ne porte plus) |
| IU4 | au jour de la bascule MikaModel | rien — c'est précisément le point : une barrière non écrite n'est pas rencontrée, elle est retrouvée après coup |

**Ordre imposé :** IU1 et IU2 dans le même commit. Toute séquence qui les sépare
laisse l'arbre rouge entre les deux, et un `git bisect` futur tomberait sur un
état où l'épinglage contredit le code sans qu'aucune des deux moitiés soit
fautive.

## Verification Contract

Toutes les vérifications sont **hors réseau et déterministes**. Aucune ne
prétend mesurer le comportement du modèle — c'est le périmètre d'AC4, établi
non exécutable en D4.

```bash
# 1. Budgets à la compilation (les const-assert de IU1).
#    Un débordement de constante ne compile pas.
cargo build -p mika-agent

# 2. Les tests du prompt compact — les inversions et les ajouts de IU2.
cargo test -p mika-agent --lib prompt::tests::mika1925
cargo test -p mika-agent --lib prompt::tests::test_build_compact_system_prompt

# 3. Contrôle négatif de D5 : les trois carve-outs sœurs restent verts
#    SANS avoir été touchés. Une rougeur ici signifie un élargissement
#    non intentionnel du périmètre.
cargo test -p mika-agent --lib prompt::tests::mika2290_compact_prompt_omits_the_hosting_line
cargo test -p mika-agent --lib prompt::tests::mika2292_compact_prompt_omits_the_doctrine_section
cargo test -p mika-agent --test eval doctrine_regressions

# 4. Non-régression des deux autres assembleurs (mika#1813 intact).
cargo test -p mika-agent --lib prompt::tests::test_silent_prompt_stopped_topics
cargo test -p mika-agent --lib prompt::tests::filter_stop_topic

# 5. Le gate de forme de requête du fournisseur compact (mika#1491).
cargo test -p mika-agent --test eval test_compact_provider_gate

# 6. Suite complète + lint.
cargo test -p mika-agent
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

**Mesure à refaire et à consigner dans le corps de la PR** — la taille du prompt
compact avant/après, bloc vide et bloc à trois entrées. D1 pose ≈ 431 octets
aujourd'hui ; le chiffre après câblage doit être consigné, parce que c'est le
seul chiffre qui rend la marge de 5 Ko vérifiable par un lecteur plutôt que
affirmée par le plan.

**Contrôle négatif explicite :** si `test_build_compact_system_prompt_empty_soul`
(compte exact = 3) doit être édité autrement que pour la règle `persist` sans
titre, c'est qu'une section a été rendue quand elle n'aurait pas dû — **halte,
ne pas ajuster le compte pour faire passer**, relire D1.

## Definition of Done

- `build_compact_system_prompt` rend la règle *persist* sur tout tour, et la
  section `## Stopped Topics` + le bloc `<stopped-topics>` quand
  `ctx.stopped_topics` est non vide.
- La distinction stop ≠ question est portée dans les deux moitiés.
- Les deux constantes portent chacune une assertion de budget à la compilation.
- `test_compact_prompt_omits_stopped_topics_block_by_design` est remplacé par son
  inverse ; deux tests neufs couvrent le cas bloc-vide et le cas bloc-plein.
- L'assertion de compte de sections est portée à 5 **avec** sa justification
  écrite, dans la forme du précédent 2 → 4.
- Les cinq épinglages des trois carve-outs sœurs sont verts, non touchés.
- `crates/mika-agent/CLAUDE.md` décrit l'état réel : trois assembleurs, forme
  abrégée sur le chemin compact, `persist` inconditionnelle et sa raison.
- La précondition AC4 est inscrite au site du code **et** dans `CLAUDE.md`, avec
  ses trois dépendances.
- La question de périmètre D5 est consignée avec sa recommandation.
- Le corps de PR nomme explicitement : (a) qu'AC4 n'est pas satisfaite et
  pourquoi, (b) que la moitié *persist* dépasse la lettre d'AC1 et pourquoi,
  (c) le ticket de suivi à ouvrir.
- `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check` passent.

## Acceptance criteria

Transcrites verbatim du corps de mika#1925, avec leur disposition.

1. **`build_compact_system_prompt` renders a size-capped variant of the
   stop-signal contract when `stopped_topics` is non-empty. Total prompt stays
   within the compact budget.**
   → Satisfaite, et **élargie** : la moitié *consult* est bien conditionnée au
   bloc non vide comme demandé ; la moitié *persist* est rendue
   inconditionnellement, parce que conditionnée elle serait inatteignable sur ce
   chemin (D3). L'élargissement est déclaré, pas silencieux. Total ≤ 5120 octets
   asserté par test, budgets par constante assertés à la compilation.

2. **The rendered variant preserves the AC2 distinction from mika#1813
   (stop != question) either inline or via a shorter rule.**
   → Satisfaite, en ligne, dans les **deux** moitiés (le préambule de section et
   la règle *persist*), pour la raison énoncée au § Planning Contract.

3. **Pinning test `test_compact_prompt_omits_stopped_topics_block_by_design` is
   deleted or inverted to assert the block IS present.**
   → Satisfaite par inversion (et non suppression) : un test nommé
   `mika1925_…_renders_the_stop_signal_contract` assère la présence, et son
   doc-comment porte la filiation.

4. **A calibration pass on MikaModel confirms the addition does not regress
   agent-mode responses (per mika#1190 model-swap discipline).**
   → **NON satisfaite, et non simulée.** Non exécutable dans ce dépôt : pas
   d'endpoint MikaModel, `create_real_provider` refuse de construire le
   fournisseur sans une clé dont il n'a pas besoin, et aucune suite de rôle ne
   mesure un agent conversationnel ni le contrat de stop-signal (D4). Re-posée
   comme **précondition bloquante** du jour où MikaModel sert un agent réel —
   inscrite au site du code et dans `CLAUDE.md` (IU4) — et sortie en ticket de
   suivi portant les trois dépendances. Cette disposition s'appuie sur le ticket
   lui-même : « No live user-facing regression today » et l'escalade en
   `p1-important` conditionnée à la mise en service.

## Risks

- **La forme rendue déclenche la complétion OOD** que le prompt compact existe
  pour éviter. Probabilité inconnue et **structurellement non mesurable ici**
  (D4). Atténuation : la forme est le miroir exact des deux assembleurs de
  référence plutôt qu'une invention (D2), le contenu est borné par des
  assertions de compilation, et le mode nominal — bloc vide — n'ajoute que la
  règle *persist*. **C'est le risque qu'AC4 est là pour fermer, et le plan ne
  prétend pas le fermer autrement.**
- **`persist` inconditionnelle introduit la première instruction d'invocation
  d'outil sur ce chemin** (D3). Classe nouvelle pour ce fournisseur. Bornée à
  ≈ 600 octets ; l'outil visé est disponible (`COMPACT_PROVIDER_CORE_TOOLS`).
  Même gate qu'au point précédent.
- **Faire passer la borne de sections à 5 sans rouvrir la justification.** Le
  commentaire du test est ce qui sépare une croissance raisonnée d'une dérive ;
  le mettre à jour sans l'étendre laisserait la prochaine addition sans
  frontière. Atténuation : IU2 point 4 l'exige nommément.
- **Le ticket de suivi d'AC4 n'est jamais ouvert** et la barrière écrite reste
  la seule protection. Le jour de la bascule, elle est rencontrée par un lecteur
  — pas par une garde. Assumé : aucune garde ne peut fermer cette porte, puisque
  le déclencheur est un geste de configuration hors de ce dépôt.
- **La question de périmètre D5 est laissée ouverte** et les trois carve-outs
  sœurs continuent de citer un ticket fermé. Atténuation : la question est
  consignée dans `CLAUDE.md` avec sa recommandation, donc visible plutôt que
  perdue.

## Sources

**Code**

- `crates/mika-agent/src/prompt.rs` — `build_compact_system_prompt` (sites A/B),
  `build_system_prompt` et `build_silent_prompt` (les deux formes de référence),
  `DATA_GRADE_DOCTRINE_COMPACT` (le précédent constante + const-assert),
  `filter_stop_topic_preferences`, `sanitize_label`, `STOP_TOPIC_PREFIX`.
- `crates/mika-agent/src/prompt.rs` tests — sites C/D/E, les quatre épinglages de
  carve-out, `stop_topic_pref`.
- `crates/mika-agent/src/agent_loop/mod.rs` — chargement de `stopped_topics`
  (≈ 463-472) ; les deux sites d'appel du prompt compact (≈ 4304, 6196) ;
  `COMPACT_PROVIDER_CORE_TOOLS` (≈ 7541) ; `false, // Silent mode: never compact`
  (≈ 5559).
- `crates/mika-common/src/llm/mod.rs` — `ProviderKind::MikaModel`,
  `default_base_url` = `http://localhost:11434`.
- `crates/mika-agent/src/calibration/providers.rs` — `create_real_provider` et
  son exemption `kind != ProviderKind::Ollama` (D4, point 2).
- `crates/mika-agent/src/calibration/roles/` — les quatre rôles existants
  (D4, point 3).
- `crates/mika-agent/tests/eval/doctrine_regressions/` — AC9 (#1814) et AC7
  (#2292), contrôles négatifs de D5.

**Documentation**

- `crates/mika-agent/CLAUDE.md:282` — « Stop-signal convention » (site F).
- `docs/plans/2026-06-07-006-feat-1398-compact-prompt-builder-mikamodel-plan.md`
  — l'énoncé du mode de panne OOD, fondation de D1 et D2.
- `docs/solutions/1379-mikamodel-provider-closed-source-namespace.md`.
- `docs/plans/2026-09-17-002-fix-2290-hebergement-est-un-fait-pose-jamais-infere-plan.md`
  et `docs/plans/2026-09-18-005-feat-2292-doctrine-materielle-aliasee-avec-butee-spirituelle-plan.md`
  — les deux carve-outs les plus récents de la même famille, et le modèle de
  raisonnement « coût nommé, rattaché à mika#1925 » (D5).

**Tickets**

- mika#1813 (le contrat), mika#1924 (la PR qui a posé le carve-out),
  mika#1398 (le prompt compact), mika#1491 (le gate d'outils),
  mika#1190 (la discipline de calibration), mika#1926 (héritage team),
  mika#1814 / mika#2290 / mika#2292 (les carve-outs sœurs).

**Consigne opérateur** — commentaire de samidarko du 2026-09-20T07:00:46Z :
ticket dé-parqué (`post-launch` retiré, `ready` posé) sur décision explicite de
Vincent, motif « stock vivant épuisé ». Retour en arrière = reposer
`post-launch`, retirer `ready`. **Lu comme : le ticket est dispatché faute de
stock, pas parce qu'une régression est mesurée** — ce qui confirme la
disposition d'AC4 (rien à régresser aujourd'hui) plutôt que de la contredire.

## Revision history

- **2026-09-20 — v1.** Plan initial. Quatre constats déplacent la lecture du
  ticket : le budget d'octets n'est pas la contrainte qui mord (D1) ; la forme
  retenue est le miroir des assembleurs existants parce que le risque réel est
  ici non mesurable (D2) ; AC1 seul est inerte, la moitié *persist* est porteuse
  (D3) ; AC4 n'est pas exécutable dans ce dépôt et est re-posée comme
  précondition bloquante plutôt que simulée (D4). Le statut de parapluie de
  mika#1925 est posé comme question ouverte, non tranché (D5).

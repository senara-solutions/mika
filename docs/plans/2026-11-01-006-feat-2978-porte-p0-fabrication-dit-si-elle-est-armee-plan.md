---
title: "mika#2978 — la porte P0 de fabrication dit au démarrage si elle est armée"
type: feat
status: active
date: 2026-11-01
issue: senara-solutions/mika#2978
---

# mika#2978 — la porte P0 de fabrication dit au démarrage si elle est armée

## Pourquoi

mika#2871 AC3 a livré la précondition machine :
`server::fabrication_p0_guard::assert_fabrication_p0_arming_valid` refuse le démarrage
quand `MIKA_FABRICATION_P0_NOTIFY=true` sans route P0 utilisable. Ce qu'elle ne dit pas,
c'est l'état **nominal** — quand le démarrage réussit, rien dans le journal ne dit si la
porte est armée ou désarmée.

C'est un trou d'observabilité, et il tombe là où AC2 de mika#2871 a besoin d'une preuve.
AC2 est la **décision d'armement**, et l'armement se fait par configuration plus
redémarrage. La commande qui existe — `mika config get fabrication_p0_notify` — lit la
configuration **déclarée** :

- verte sur un `config.toml` édité dont le redémarrage n'a jamais eu lieu ;
- verte sur une valeur qu'un `.env` par agent écrase plus haut dans la cascade.

La ligne de démarrage, elle, est écrite par le processus **qui sert la valeur**. C'est la
même distinction que `mika agents budget` a close pour le budget LLM (mika#2457) :
« ce que le dépôt déclare » n'est pas « ce que le runtime sert ».

## Décisions

### D1 — La polarité : la ligne se dit sur l'ARMEMENT, et c'est une troisième polarité

C'est la décision qui tient tout le reste, et c'est aussi le piège de copier-coller du
vol. La famille `log_*_gate_state` a quatre membres, et ce vol en ajoute un cinquième dont
la direction n'est celle d'**aucun** des quatre :

| membre | défaut | la ligne sort quand | elle décrit |
|---|---|---|---|
| `log_qa_ci_coherence_gate_state` (mika#2455) | armé | désarmé | la dérogation |
| `log_dependabot_verdict_gate_state` (mika#2519) | armé | désarmé | la dérogation |
| `log_pipeline_grounding_gate_state` (mika#2687) | armé | désarmé | la dérogation |
| `log_pass_evidence_gate_state` (mika#2961) | **désarmé** | **désarmé** | le **nominal** |
| **ce vol** | **désarmé** | **armé** | la dérogation |

Le ticket dit « la polarité est celle de `log_pass_evidence_gate_state` » : c'est vrai du
**défaut** (désarmé dans les deux cas) et faux de la **direction de la ligne**.
`log_pass_evidence_gate_state` parle par défaut, et son doc-comment dit pourquoi — sans sa
ligne, « un `pass` sans preuve pipeline passerait sans que rien n'ait dit que le garde
dort » ; le silence y est obligatoire à rompre parce que c'est le silence qui est
dangereux.

Ici c'est l'inverse : **l'état livré est sûr**. Une porte désarmée ne fait rien partir
nulle part, et mika#1381 D4 reste en place avec son WARN
`operator_escalation_chat_unset`. C'est l'état **armé** qui change le comportement du
système vis-à-vis de l'utilisateur final, donc c'est lui la dérogation, et la doctrine
mika#2131 s'applique telle quelle : une ligne par boot sur une flotte saine est du bruit.

**Conséquence de lecture, à écrire dans la doc opérateur (U3) :** le silence veut dire
« désarmée », qui est l'état livré — et la classe mika#2340 s'applique, donc une absence
de ligne ne se lit qu'après avoir établi que le binaire servi porte ce correctif.

### D2 — Où la fonction vit : `server/fabrication_p0_guard.rs`, pas `evidence/guards.rs`

Le ticket demande le cinquième membre « à côté de ses quatre voisins **dans
`crates/mika-agent/src/server/mod.rs`** » — c'est le **site d'appel** qui est nommé là, pas
le site de définition. Les quatre voisins sont définis dans
`crates/mika-agent/src/evidence/guards.rs`. Ce vol définit le cinquième dans
`crates/mika-agent/src/server/fabrication_p0_guard.rs`, et c'est une décision :

- **La donnée n'est pas de la même nature.** Les quatre voisins lisent une **variable
  d'environnement de process**, résolue une fois par `OnceLock`, sans argument. Ici
  `fabrication_p0_notify` est un champ de `Settings`, résolu **par agent** à travers la
  cascade complète (`.env` par agent, env de process, `config.toml` par agent,
  `config.toml` global). La fonction prend donc un `&Path` et itère sur la flotte — c'est
  la forme de la garde, pas celle de la famille.
- **Les quatre dépendances sont déjà là.** `super::tier_guard::servable_agent_names`,
  `Settings::load_for_agent`, `crate::messaging::p0_route_is_usable` (`pub(crate)`) et les
  deux constantes `NOTIFY_ENV_VAR` / `ESCALATION_ENV_VAR` sont dans ce fichier. La définir
  dans `evidence/guards.rs` dupliquerait les deux constantes — deux sites de vérité sur le
  nom d'une variable d'environnement — ou les rendrait publiques à travers un module qui
  n'a rien à voir avec elles.
- **`evidence/guards.rs` dépasse 6 500 lignes** et porte une classe (les gardes de
  verdict QA). La porte P0 de fabrication n'en fait pas partie.

**La cohésion de famille est préservée là où elle compte** : le nom
(`log_fabrication_p0_gate_state`), le niveau (`info!`), le champ `env`, la forme de
l'événement et le site d'appel. Le doc-comment la nomme explicitement comme cinquième
membre et renvoie à `log_pass_evidence_gate_state` pour l'inversion de D1.

### D3 — Un second scan de la flotte, assumé et borné

La fonction re-parcourt `servable_agent_names` et recharge un `Settings` par agent — le
même travail que la garde vient de faire. C'est une duplication de **coût**, pas de
**vérité**, et le prédicat d'armement reste à un seul site
(`settings.fabrication_p0_notify` plus `p0_route_is_usable`).

L'alternative — faire rendre à `assert_fabrication_p0_arming_valid` la liste des agents
armés — est **écartée** : elle change la signature d'une fonction `assert_*` pour lui
faire porter de l'observabilité, elle touche ses quatre tests, et elle sort la ligne de la
famille `log_*_gate_state` que le ticket demande d'étendre. Le coût évité serait de
quatre lectures de `config.toml` sur un chemin de démarrage qui en fait déjà plusieurs
dizaines (provisionnement, `budget_guard`, `init_agent`).

### D4 — Ce que la ligne porte, champ par champ

Une ligne **par agent armé** — pas une ligne agrégée. Un champ `tracing` porte un
scalaire ; une liste jointe perdrait le `agent_id` indexable sur lequel toutes les sondes
opérateur du dépôt filtrent (`llm_budget_resolved`, les six sondes de mika#2871).

```
event = "fabrication_p0_gate_armed"   // INFO
agent_id = <nom de l'agent>
env = MIKA_FABRICATION_P0_NOTIFY       // la constante, jamais un littéral
escalation_chat_id = <id, ou "absent">
```

Le message en clair porte les deux faits ensemble — « armée, et routée vers tel canal » —
et la sortie STOP qu'AC1 exige : poser `MIKA_FABRICATION_P0_NOTIFY=false` (ou
`fabrication_p0_notify = false` dans le `config.toml` de l'agent) **et redémarrer**. Le
« et redémarrer » n'est pas décoratif : `Settings` est lu au démarrage, comme la doc
opérateur le dit déjà de cette variable.

**Pourquoi `escalation_chat_id` est rendu par le même formateur que la garde.** La garde
écrit `absent` ou la valeur lue, parce que les deux ne se corrigent pas au même endroit
(mika#2871). Après la garde, l'état « armé + absent » est inatteignable — mais écrire un
`unwrap_or_default()` qui rendrait `0` ferait **mentir** la ligne le jour où cette
prémisse cesse de tenir. Un `fn escalation_chat_display(Option<i64>) -> String`, extrait
du `match` déjà inline dans la garde, sert les deux sites : c'est −4 lignes nettes et un
seul rendu de cette valeur.

**Niveau `info!`, comme les quatre voisins.** Un armement est une décision d'opérateur
délibérée, pas une anomalie ; `warn!` dirait le contraire.

### D5 — Le site d'appel : juste après la garde, et non en cinquième position

L'appel va immédiatement après
`fabrication_p0_guard::assert_fabrication_p0_arming_valid(global_home)?`
(`crates/mika-agent/src/server/mod.rs:975`), donc **avant** les quatre voisins et non
après eux.

La raison est un invariant, pas une préférence : la garde vient d'établir que *si* la
porte est armée *alors* elle a une route utilisable. La ligne qui énonce les deux faits
ensemble ne peut le faire honnêtement qu'après cette garde, et les trois appels qui
suivent ne lui apportent rien. Mettre l'appel en cinquième position romprait l'adjacence
qui porte le sens, pour gagner une symétrie visuelle. Le commentaire du site le dit, dans
la forme des quatre voisins, et nomme l'inversion de polarité de D1 — c'est là que le
prochain lecteur la cherchera.

### Pistes écartées

- **Une ligne par boot dans les deux cas** — c'est le bruit que la doctrine mika#2131
  borne, et AC2 l'interdit explicitement.
- **Étendre la garde pour qu'elle rende les agents armés** — § D3.
- **Un champ de plus sur une ligne existante** (`llm_budget_resolved`, ou la ligne de
  provisionnement) — élargirait en silence une population déjà publiée, ce que mika#2131
  refuse ; et la ligne porteuse ne serait pas celle du site où l'invariant est établi.
- **Un `mika agents p0` en CLI** — la commande lirait la configuration déclarée depuis un
  autre processus, c'est-à-dire exactement le défaut que le ticket décrit. Le journal du
  processus qui sert la valeur est le seul instrument qui ne puisse pas se tromper là.
- **Faire porter la sonde par `audit_events`** — la table est le bon instrument pour un
  événement **par décision** (c'est le contrôle positif de mika#2687/#2961) ; ici le fait
  est un état de démarrage, il appartient au journal.
- **Logguer aussi le `docs_root`, le tier, ou l'état des six détecteurs `_uncorrected`** —
  hors ticket ; les détecteurs restent armés indépendamment (mika#2867) et leurs six
  sondes existent déjà.

## Unités

### U1 — La fonction et son formateur (`server/fabrication_p0_guard.rs`)

- `pub fn log_fabrication_p0_gate_state(home_dir: &Path)` — boucle sur
  `super::tier_guard::servable_agent_names`, charge `Settings::load_for_agent`, émet une
  `info!` par agent dont `fabrication_p0_notify` est vrai, avec les quatre champs de D4.
- `fn escalation_chat_display(Option<i64>) -> String` — extrait du `match` inline de
  `assert_fabrication_p0_arming_valid`, qui l'appelle désormais au lieu de le réécrire.
- **Un `Settings` qui échoue à charger n'émet rien et n'interrompt rien.** La fonction rend
  `()` comme ses quatre voisins. La branche est inatteignable en pratique — la garde, trois
  lignes plus haut, a déjà propagé cette erreur et refusé le démarrage — et la rendre
  muette évite de transformer une fonction d'observabilité en seconde source de refus. Le
  doc-comment le dit ; c'est l'ordre d'appel qui est la raison, donc il doit être écrit là
  où quelqu'un pourrait le réordonner.
- Doc-comment portant D1 (le tableau des cinq polarités, en une phrase) et D2.

### U2 — Le site d'appel (`server/mod.rs`)

Une ligne d'appel plus son commentaire, immédiatement après la garde (§ D5), dans la forme
des quatre voisins : le numéro de ticket, le geste, et ce que le silence veut dire.

### U3 — Documentation (`docs/operator/environment-variables.md`)

Le fichier porte déjà la section fabrication P0 (ses six sondes, § *Condition
d'armement*). Deux ajouts, dans la forme du bloc de mika#2961 :

- l'événement dans le bloc de sondes `grep`, avec le commentaire qui dit la polarité ;
- une ligne dans le tableau `surface | niveau | régime attendu | lecture` : régime attendu
  **vide avant l'armement**, et une absence après `make deploy` se lit « désarmée » **ou**
  « binaire antérieur au vol » (classe mika#2340) ;
- dans § *Condition d'armement*, la phrase qui fait de cette ligne la **preuve d'AC2 de
  mika#2871** : c'est le processus qui sert la valeur qui l'écrit, là où
  `mika config get fabrication_p0_notify` ne lit que la configuration déclarée.

Hors compte de lignes (règle mika#2636).

## Fire-Disposition

Le vol livre un détecteur — le test d'AC3 de § *Vérification* — donc la section est due.

**Option retenue : (a), exception nommée en allowlist, avec une allowlist vide — et le
vide est une propriété de construction, pas un constat optimiste.**

Détail d'implémentation :

1. **Le détecteur atterrit armé.** Aucun `#[ignore]`, aucun `#[cfg(skip)]`, aucune
   assertion commentée : vert au premier `cargo test -p mika-agent`.
2. **Aucune allowlist n'est créée, parce qu'il n'y a aucun corpus à excepter.** Le test
   appelle une fonction **créée par ce vol** sur des `TempDir` qu'il provisionne lui-même.
   Il n'inspecte ni `src/**`, ni `docs/**`, ni l'historique git, ni un corpus de PR : il
   ne peut donc pas rencontrer de violation préexistante. C'est la différence avec un scan
   structurel, et c'est pourquoi écrire une allowlist vide ici serait l'emplacement vide
   que la doctrine mika#2201 refuse de créer.
3. **La propriété auto-nettoyante est portée par la seconde moitié du test, pas par une
   allowlist.** La phase « désarmée ⇒ journal vide » rougit le jour où quelqu'un rend la
   ligne inconditionnelle (le régime que la doctrine mika#2131 borne), et la phase
   « armée ⇒ ligne présente » rougit le jour où l'appel disparaît du site de démarrage. Les
   deux dans le même corps : c'est AC3, et c'est ce qui remplace ici l'assertion de
   staleness.
4. **Les détecteurs préexistants du voisinage ne changent pas de comportement.** Les
   quatre tests de `assert_fabrication_p0_arming_valid` sont intouchés dans leur intention
   — y compris
   `mika2871_la_porte_desarmee_sans_chat_nest_pas_une_violation`, qui est l'assertion
   auto-nettoyante de la Fire-Disposition de mika#2871. L'extraction de
   `escalation_chat_display` (§ U1) ne change pas une chaîne qu'ils assèrent :
   `operator_escalation_chat_id = absent` et `= 0` restent rendus caractère pour
   caractère, et ces deux assertions **sont** le contrôle de l'extraction.

## Vérification

**Le test d'AC3 — les deux polarités dans le même corps.** Un test, deux phases, chacune
avec son propre tampon de capture :

- phase armée : un home avec un agent portant
  `fabrication_p0_notify = true` et `operator_escalation_chat_id = -100123` ⇒ le journal
  rendu contient `fabrication_p0_gate_armed`, le **nom de l'agent**, la chaîne
  `MIKA_FABRICATION_P0_NOTIFY`, l'id du chat, et la sortie STOP (`false` **et** le mot
  « redémarr· ») ;
- phase désarmée : un home avec un agent sans aucune des deux clés — la géométrie livrée
  ⇒ le journal rendu est **vide**.

Pourquoi les deux dans le même corps, et c'est la phrase d'AC3 : un test qui n'observe que
le côté qui parle ne distingue pas « la ligne sort quand c'est armé » de « la ligne sort
toujours » — exactement la forme que mika#2871 avait déjà imposée à sa propre garde
(« une sonde qui ne peut observer que le côté refusant »).

**Comment la capture est faite, et ses deux pièges.**

- L'abonné passe par `crate::test_utils::test_helpers::install_capturing_subscriber` —
  **site d'installation unique du crate** (mika#2646), tenu par
  `mika2646_set_default_a_un_site_dinstallation_unique`. Tout `set_default` /
  `with_default` nu sous `src/` fait rougir cette garde, allowlist livrée vide ; il n'y a
  donc pas d'autre chemin.
- Le `MakeWriter` est le `TamponPartage` de `wip_rescue.rs:3651` / `cadence.rs:2481`, qui
  fait porter les assertions sur les **octets rendus**, donc sur ce que l'opérateur lit.
- `#[serial_test::serial]`, et pas seulement sur le test capturant : la doctrine mika#2646
  dit que la sérialisation doit couvrir les **poisonneurs** du callsite autant que les
  capturants — un voisin qui atteint le callsite sans abonné en éteint l'`Interest`
  globalement et la capture rendrait zéro. Le callsite créé par ce vol n'est atteint que
  par ce test ; tous les tests de ce module portent déjà `#[serial]`.
- `clean_p0_env()` (le helper du module) en tête de test, pour la raison que son
  doc-comment donne déjà : ces tests assèrent ce que la **cascade** résout, et l'env de
  process est une de ses portes. Un opérateur qui a `MIKA_FABRICATION_P0_NOTIFY` exporté —
  précisément l'opérateur dont ce vol parle — verrait sinon la phase désarmée rougir.

**Non-régression.** Les quatre tests de `assert_fabrication_p0_arming_valid` passent sans
que leur intention change ; les deux assertions sur `operator_escalation_chat_id = absent`
et `= 0` sont le contrôle de l'extraction de `escalation_chat_display` (§ Fire-Disposition
point 4). Le chemin de refus de la garde, le prédicat `p0_route_is_usable`, les six
détecteurs `_uncorrected` (mika#2867) et le fallback mika#1381 D4 ne bougent pas d'un
octet.

**Suite complète.** `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`, les lints du
dépôt, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

Au démarrage, `mika-spirit` écrit une ligne INFO par agent qui arme la porte
`fabrication-detected` P0, nommant l'agent, la variable `MIKA_FABRICATION_P0_NOTIFY`, le
canal d'escalade vers lequel la porte est routée et la sortie STOP (`=false` plus
redémarrage) ; il n'écrit **rien** quand la porte est désarmée, qui est l'état livré ; un
test unique épingle les deux polarités sur les octets rendus ; et
`docs/operator/environment-variables.md` dit que cette ligne est la preuve d'AC2 de
mika#2871 là où `mika config get` ne lit que la configuration déclarée.

## Acceptance criteria

- [ ] **AC1.** Au démarrage, une ligne de log dit que la porte `fabrication-detected` P0
      est **armée**, pour l'agent qui l'arme. Elle nomme la variable
      (`MIKA_FABRICATION_P0_NOTIFY`) et la sortie STOP (`=false` + redémarrage).
- [ ] **AC2.** Rien n'est émis quand la porte est désarmée — l'état livré ne produit pas de
      ligne par boot (doctrine mika#2131).
- [ ] **AC3.** Un test épingle les deux polarités dans le même corps. Un test qui n'observe
      que le côté qui parle ne distingue pas « la ligne sort quand c'est armé » de « la
      ligne sort toujours ».
- [ ] La ligne porte aussi le **canal d'escalade** vers lequel la porte est routée, rendu
      par le même formateur que le message de refus de la garde (`absent` ou la valeur
      lue), jamais par un défaut qui vaudrait `0`.
- [ ] L'appel est placé immédiatement après
      `assert_fabrication_p0_arming_valid`, dont l'invariant est ce qui autorise la ligne à
      porter les deux faits ensemble (§ D5).
- [ ] La garde de mika#2871 est inchangée dans son refus, son prédicat et ses chaînes
      assérées ; ses quatre tests passent sans changement d'intention.
- [ ] `docs/operator/environment-variables.md` porte l'événement dans le bloc de sondes,
      son régime attendu, la lecture de son absence (classe mika#2340) et son rôle de
      preuve pour AC2 de mika#2871.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/fabrication_p0_guard.rs` (U1 : fonction, formateur extrait, doc-comments portant D1/D2) | 50 |
| tests dans `fabrication_p0_guard.rs` (`TamponPartage`, helper de capture, test AC3 à deux phases) | 65 |
| `crates/mika-agent/src/server/mod.rs` (U2 : appel + commentaire) | 10 |

Total estimé : 125 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 125 < 279 et 125 < 1000 : les deux bornes
passent sans découpage. Trois raisons rendent l'estimation peu susceptible de déborder.

**Les trois sites sont lus, pas supposés.** La fonction va dans un fichier de 250 lignes
déjà intégralement lu ; ses quatre dépendances y sont ou sont `pub(crate)` dans le même
crate (`servable_agent_names` à `tier_guard.rs:219`, `p0_route_is_usable` à
`messaging.rs:87`, `Settings::load_for_agent`, les deux constantes). Le site d'appel est
une ligne dans un bloc de quatre voisins identiques. Aucun type nouveau, aucun schéma,
aucune migration, aucun prompt, aucun modèle — donc pas de `make calibrate-*` en
précondition (mika#1190).

**Le tiers coûteux est un patron existant à recopier.** Les 65 lignes de test sont pour
moitié le `TamponPartage` de `wip_rescue.rs:3651`, qui se transpose sans décision, et le
helper de capture y est déjà écrit. Le reste est deux `TempDir` provisionnés par
`home_with_agent`, helper **déjà présent** dans le module.

**Le ticket estimait ≈ 25 lignes ; ce plan en estime 125, et l'écart est le test d'AC3.**
Les quatre voisins de la famille n'ont **aucun** test sur leur ligne de journal — c'est
mesuré, pas supposé : `grep` sur les quatre noms ne rend que leur définition et leur site
d'appel. AC3 demande une capture de journal là où la famille n'en avait jamais eu, donc le
harnais de capture (abonné, writer, deux phases) est une dépense que le patron voisin
n'avait pas payée. C'est 65 des 125 lignes, et c'est la partie du vol dont le volume est le
mieux connu puisqu'elle se recopie.

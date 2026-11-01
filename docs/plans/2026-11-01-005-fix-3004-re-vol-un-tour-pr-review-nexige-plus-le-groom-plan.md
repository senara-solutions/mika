# mika#3004 — re-vol : un tour `pr_review` n'exige plus l'outil de groom

> **Ticket :** senara-solutions/mika#3004 (`bug`, `p2-normal`, `ready`,
> `dispatch:loop`, `loop-substrate`)
> **Branche :** `fix/3004/dispatch-un-tour-pr-review-de-mika-dev`
> **Plan antérieur :** `docs/plans/2026-11-01-004-fix-3004-dispatch-un-tour-pr-review-de-mika-dev-plan.md`
> (624 lignes, draft #3009). **Matière, pas plan à rejouer** — § 6 dit ce qui en est repris et ce qui en est écarté.

> **Second groom du 2026-11-01 — ce qui a été re-mesuré, et ce qui a changé.**
> Toutes les mesures des § 1.3 et § 1.4 ont été refaites à la main et
> **confirmées** : `mod.rs` à 1 044 559 octets sur `main` et 1 071 667 sur la
> branche, `LARGE_FILE_ALLOWLIST` vide, le recensement à frontière de mot à
> 7 sites dans 4 fichiers, `cargo check -p mika-cli` à exactement deux `E0063`,
> et les trois tests de bout en bout verts. Trois faits **nouveaux** sont entrés,
> et ils changent le plan :
>
> 1. **La draft #3009 est ouverte et sa branche distante vit** — le corps du
>    ticket l'annonce fermée et sa branche supprimée. C'est faux à la mesure, et
>    cela décide de la fin de pipeline (§ 1.5).
> 2. **Un troisième défaut de livraison** — le marqueur durable
>    `clippy-errors-need-human` sur #3009, dont la cause est mesurée et n'est pas
>    une lint (§ 1.5).
> 3. **G1 avait une contradiction interne** : son § 3.1 descendait `tour_neutre()`
>    et ses quatre tests, pendant que sa DoD interdisait tout élargissement de
>    visibilité — or le déplacement en exige un. Les deux sont corrigés, et
>    l'élargissement est désormais nommé et borné (§ 3.1).

---

## 1. Le point de départ, mesuré — et il n'est pas `main`

### 1.1 Ce que la branche porte déjà

Le premier vol (dev-pilot `6d262ac1`) est mort au plafond à 201 tours, mais **il
avait fini le travail de fond**. Les quatre commits sont sur la branche et le
re-vol en hérite, parce que le slug de branche est dérivé du titre du ticket :
la même dérivation redonne la même branche, et `_set_up_worktree` la reprend au
lieu de la recréer. Mesuré le 2026-11-01 sur `HEAD = 8da087abd` :

```
8da087abd wip(mika#1383): auto-PR-create rescue for mika#3004      ← commit VIDE (rescue)
2b481953b test(3004): la trace 3b771914 rejouée de bout en bout, avec ses deux contrôles
9a5bc9964 fix(3004): un tour de territoire de handler n'exige plus l'outil de groom
17f64504d refactor(3004): le domaine Fallthrough lit le territoire des handlers en négation
9450861f6 docs(plans): groom mika issue#3004 (content-only, architect pending)
```

`git diff --numstat origin/main...HEAD` :

| fichier | + | − |
|---|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` | 530 | 32 |
| `crates/mika-agent/src/server/a2a.rs` | 5 | 0 |
| `crates/mika-agent/src/server/handlers.rs` | 12 | 0 |
| `crates/mika-agent/src/webhook_dispatch.rs` | 44 | 12 |
| `crates/mika-agent/tests/eval.rs` | 5 | 0 |
| `crates/mika-agent/tests/eval/harness.rs` | 33 | 0 |
| `crates/mika-agent/tests/eval/test_pr_review_no_groom_requirement_3004.rs` | 316 | 0 |

### 1.2 Les trois critères d'acceptation sont **déjà** verts sur la branche

Mesuré, pas supposé :

```
$ cargo test -p mika-agent --test eval mika3004
test mika3004_un_tour_pr_review_ne_prend_pas_le_re_prompt_de_groom ... ok
test mika3004_le_meme_brief_sans_texte_devenement_prend_le_re_prompt ... ok
test mika3004_un_label_ready_sans_pr_exige_toujours_le_groom ... ok
test result: ok. 3 passed; 0 failed
```

AC1 (le désarmement), AC2 (contrôle positif `pr_review` + contrôle négatif
label `ready` sans PR, dans le même appel) et AC3 (la trace `3b771914` en
fixture, rejouée de bout en bout dans `EvalHarness`) sont clos par ces commits.
**Ce re-vol n'a donc pas de logique à concevoir : il a deux défauts de livraison
à réparer.** C'est ce qui le fait tenir en un vol, et c'est dit en § 11.

### 1.3 Défaut de livraison nº 1 — `mod.rs` dépasse le plafond d'un méga-octet

Le run CI `41321165649`, job *Secret Scan* :
`crates/mika-agent/src/agent_loop/mod.rs is 1046KB — exceeds 1MB limit`.

Mesures à l'octet (le détecteur est *diff-scoped* : il ne scanne que les
fichiers que la PR modifie, donc `mod.rs` y est, et y restera) :

| | octets |
|---|---|
| plafond `LARGE_FILE_LIMIT` (`scripts/check-secrets.sh` l. 23) | 1 048 576 |
| `mod.rs` sur `origin/main` | 1 044 559 |
| **marge disponible** | **4 017** |
| `mod.rs` sur la branche | 1 071 667 |
| dépassement | 23 091 |

`LARGE_FILE_ALLOWLIST` est **vide** et le reste : le ticket l'interdit
explicitement, et le contrat d'ajout du script exige qu'une entrée nomme un
fichier *légitimement* surdimensionné (fixture dorée, binaire de test).
`mod.rs` n'en est pas un.

### 1.4 Défaut de livraison nº 2 — deux initialiseurs d'`AgentParams` non mis à jour

Le run CI, jobs *Check* et les deux Darwin :
`error[E0063]: missing field webhook_event_text in initializer of AgentParams`.

**Le recensement du corps du ticket — « 25 sites dans 17 fichiers » — est un
artefact de grep, et il faut le corriger avant de planifier 25 éditions.** Le
motif `AgentParams {` est un **sous-chaîne** de `SilentAgentParams {` et de
`TeamAgentParams {`, qui sont trois structs distincts (`mod.rs:5890`, `:7394`,
`:8369`). Seul le premier gagne le champ.

Le grep qui tranche, avec frontière de mot :

```
git grep -nE '(^|[^a-zA-Z])AgentParams \{' -- 'crates/**/*.rs'
```

Il rend **7 sites dans 4 fichiers**, pas 25 dans 17 :

| site | état sur la branche |
|---|---|
| `crates/mika-agent/src/server/a2a.rs:312` | ✅ `webhook_event_text: None` |
| `crates/mika-agent/src/server/handlers.rs:2096` | ✅ `original_github_text.as_deref()` |
| `crates/mika-agent/tests/eval/harness.rs:118` | ✅ |
| `crates/mika-agent/tests/eval/harness.rs:179` | ✅ |
| `crates/mika-agent/tests/eval/harness.rs:251` | ✅ |
| `crates/mika-cli/src/commands/chat.rs:336` | ❌ **manquant** |
| `crates/mika-cli/src/commands/chat.rs:460` | ❌ **manquant** |

`SilentAgentParams` (8 sites) et `TeamAgentParams` (4 sites) **ne gagnent aucun
champ** : les tours silencieux et d'équipe ne naissent pas d'un webhook, et les
commits de la branche passent `None` littéral à l'unique site d'appel interne de
chacun (`mod.rs:8200`, `:8859`). Rien à éditer chez eux — ce qui explique aussi
pourquoi `cargo check -p mika-agent --all-targets` est **propre** sur la branche.

Et la contre-mesure : `cargo check -p mika-cli` rend **exactement deux erreurs**,
les deux de `chat.rs`. C'est tout l'écart de compilation du workspace.

Le champ n'admet pas de `..Default::default()` : `AgentParams<'a>` porte
`&'a dyn LlmProvider`, `&'a AsyncDatabase`, `&'a Path`… — des références, qui
n'implémentent pas `Default`. Pas de constructeur non plus : deux éditions d'une
ligne coûtent moins qu'un constructeur à trente paramètres. **Le plan dit
donc : deux éditions à la main, `webhook_event_text: None`.**

### 1.5 Défaut de livraison nº 3 — et l'état réel de la draft #3009

**Le corps du ticket dit : « La draft #3009 est fermée sans merge avant le
`ready`, et sa branche est supprimée. » Mesuré le 2026-11-01, c'est faux, et
c'est la mesure qui décide.**

```
$ gh pr view 3009 --repo senara-solutions/mika --json state,isDraft,headRefOid
{"state":"OPEN","isDraft":true,"headRefOid":"8da087abde87535e6ac69336f4db1c0f447c73e2"}
$ git ls-remote --heads origin 'fix/3004*'
8da087abde87535e6ac69336f4db1c0f447c73e2  refs/heads/fix/3004/…-de-mika-dev
```

#3009 est **OPEN**, en draft, et sa tête distante est exactement le `HEAD` local.
L'intention de l'opérateur n'a pas été exécutée ; le travail n'est donc pas
seulement lisible par `refs/pull/3009/head`, il est **sur la branche vivante que
ce vol va pousser**. Trois conséquences, et la première est celle qui coûte des
tours si elle est apprise en fin de pipeline :

1. **`gh pr create` échouera** — une PR existe déjà pour cette tête. Le re-vol
   **pousse sur la branche et met à jour #3009** ; il n'en ouvre pas une
   seconde. Voir la fin de pipeline au § 5.
2. **Le titre de #3009 est un titre de commit** — « test(3004): la trace
   3b771914 rejouée de bout en bout, avec ses deux contrôles » — hérité du
   premier commit poussé. Il décrit un commit, pas la PR. À corriger
   (`gh pr edit 3009 --title …`), avec le corps.
3. **#3009 est hors du scan d'auto-resume, de façon permanente.** Commentaire du
   2026-11-01 15:46:38Z de `mika-platform-dev` : auto-resume s'est arrêté sur
   `clippy-errors-need-human` et a rendu la PR à un humain. L'exclusion est un
   **marqueur durable** (mika#2199), pas le label `human-review-required` :
   retirer un label ne réarme rien. Ce vol ne doit donc compter sur aucune
   reprise automatique, et il n'a pas à chercher à lever l'exclusion.

**Le défaut nº 3, et sa cause mesurée.** Le marqueur nomme clippy, mais il n'y a
**aucune lint clippy** sur la branche :

```
$ cargo clippy -p mika-agent --all-targets      # propre, 0 erreur, 0 lint
$ cargo clippy --workspace --all-targets
error[E0063]: missing field `webhook_event_text` … (×2)
error: could not compile `mika-cli` (bin "mika") due to 2 previous errors
error: could not compile `mika-cli` (bin "mika" test) due to 2 previous errors
```

Les « erreurs clippy » **sont** les deux `E0063` du § 1.4 : clippy ne peut pas
compiler `mika-cli`, donc il rend des erreurs de compilation sous son propre nom.
C'est un fait utile et non une mauvaise nouvelle : **G2, deux lignes, ferme
d'un coup *Check*, les deux *Darwin* et le motif du marqueur clippy.** Ce qu'il
ne ferme pas, parce que rien ne le ferme, c'est l'exclusion d'auto-resume
elle-même — durable par conception.

Il n'existe donc pas de troisième geste à concevoir : il existe un troisième
symptôme, qui a la même cause que le deuxième. Le re-vol reste à deux gestes.

---

## 2. AC1 — le site qui l'exigeait, nommé

Deux sites, et il faut les distinguer parce que la correction ne tient pas au
même.

**Le site qui *pose* l'exigence** est `collect_required_tools`
(`crates/mika-agent/src/agent_loop/mod.rs:9307` sur `main`). Il fait l'union des
`[constraints] required_tools` de toutes les skills appariées par mot-clé. Sur
le tour de la trace `3b771914`, `dev-groom` a été appariée — son
`skills/bundled/dev-groom/skill.toml` déclare
`keywords = ["groom", "groom ticket", "/mika-groom-ticket", "groom issue"]` et
`required_tools = ["run_claude_pilot_groom"]` — parce que le mot *groom*
figurait dans le brief du tour.

**Le site où la correction tient** est `filter_available_required_tools`
(`mod.rs:9943` sur `main`), et sa fonction de décision pure
`raison_de_desarmement` (`:9859`). C'est l'étage de **désarmement** institué par
mika#2757 : *une exigence que le tour lui-même rend impossible à satisfaire est
une contradiction, et le moteur la retire en la nommant.*

**Pourquoi là, et pas dans le prompt ni dans l'appariement.**

1. *Pas dans le prompt.* Le re-prompt « agent responded without calling required
   tools » est émis par le moteur, pas par le modèle. Aucune formulation de
   prompt ne l'empêche : le moteur compare la liste des outils appelés à
   `effective_required_tools` et re-prompte sur la différence. Un prompt est une
   suggestion ; `required_tools` est une boucle.
2. *Pas dans l'appariement par mot-clé.* Désapparier `dev-groom` quand le brief
   contient *groom* reviendrait à décider du domaine du tour depuis le lexique
   du brief. Or un tour de fallthrough légitime **cite** régulièrement le mot —
   corps de ticket, commentaire d'opérateur — et on fermerait un groom dû. C'est
   la classe `mask_cited_branch` (mika#2755 / mika#2768) : un prédicat qui lit
   une citation comme un fait.
3. *Donc au désarmement.* Le domaine d'un tour est une propriété du **tour**,
   pas de l'outil ni du lexique (mika#2757). Le site de désarmement est le seul
   qui possède à la fois l'exigence et le domaine du tour, et c'est le seul où
   le retrait est **journalisé nommément** (`required_tool_disarmed`,
   `reason = …`) plutôt que silencieux.

**Pourquoi la branche 4 existante (mika#2757) n'a pas mordu — deux causes
cumulées**, et la seconde décide de la forme :

- *cause A* — le territoire des handlers qa / ci est **sorti** du domaine Webhook
  Fallthrough par conception (`self-dev-webhook-qa` / `-ci` y portent des
  dispatchs d'implémentation légitimes), donc `interdit_dispatch` y vaut `false` ;
- *cause B* — le texte jugé était celui d'**après** les handlers. Sur ce tour-là
  `verdict_handler` avait remplacé `req.text` par un pré-digest commençant par
  `<verdict_handler>\n`, si bien que tout prédicat de préfixe répondait *« ce
  n'est pas un événement GitHub »* sur un tour qui en était un.

Un vol qui n'aurait traité que la cause A aurait été **vert sur une fixture
d'événement brut et rouge en production**. C'est pourquoi
`AgentParams::webhook_event_text` voyage — et pourquoi `chat.rs` doit être mis à
jour plutôt que le champ retiré.

---

## 3. Le dessin du re-vol — trois gestes, aucun nouveau comportement

### 3.1 G1 — reloger le contenu mika#3004 de `mod.rs` dans un sous-module dédié

Nouveau fichier : **`crates/mika-agent/src/agent_loop/pr_review_groom.rs`**
(nom pris à l'exemple du ticket, pour ne pas relitiger un nom).

Y descendent, tels quels ou condensés :

- la struct `ProprietesDuTour` (champs privés `interdit_dispatch`,
  `territoire_handler`, dérive `Debug, Clone, Copy`) et son unique constructeur
  `ProprietesDuTour::depuis(webhook_event_text, turn_message)` ;
- le doc-module qui porte le raisonnement mika#3004 — la chaîne causale, les
  causes A et B, le refus de la lecture ligne à ligne du pré-digest, la raison
  pour laquelle la branche 5 ne porte **que** l'outil de groom et jamais
  `run_claude_pilot` ;
- les trois constantes de fixture `PR_REVIEW_3004_EVENT`,
  `PR_REVIEW_3004_PRE_DIGEST`, `CHECK_SUITE_3004_EVENT` ;
- les quatre tests unitaires
  `mika3004_un_tour_pr_review_desarme_lexigence_de_groom`,
  `mika3004_le_meme_tour_laisse_run_claude_pilot_exigible`,
  `mika3004_un_check_suite_repond_sous_non_servi_en_production`,
  `mika3004_les_deux_proprietes_du_tour_sont_disjointes`.

**Ne descend PAS, contre ce que disait la version antérieure de ce plan :
l'aide de test `tour_neutre()`.** Mesuré — elle est définie à `mod.rs:17027` et
appelée six fois (`:17057`, `:17087`, `:17115`, `:17142`, `:17166`, `:17220`),
**toutes par des tests qui n'ont rien à voir avec mika#3004** (la famille
mika#2757 / mika#2750), et **aucune** par les quatre tests mika#3004, qui
construisent `ProprietesDuTour::depuis(…)` directement. La descendre casserait
six tests restants, ou forcerait un `pub(super)` sur une aide dont le seul
usage réel est dans `mod.rs`. Elle reste où elle est.

**Les deux élargissements de visibilité que G1 exige — nommés, bornés, et dont
l'un seul concerne un test.** Un module enfant voit les items privés de ses
ancêtres, mais **pas** ceux d'un module frère : `agent_loop::pr_review_groom`
ne voit rien de `agent_loop::tests` (`mod tests` à `:12786`). D'où :

| item | visibilité | pourquoi, et pour qui |
|---|---|---|
| `ProprietesDuTour` + `ProprietesDuTour::depuis` | `pub(super)` | pour la **production** : `mod.rs` les utilise à `:9990` (signature) et `:10165` (appel réel). Rien à voir avec un test. |
| `make_resolved_skill_tool` (`:16614`), `make_registry_with_tool` (`:16629`) | `pub(in crate::agent_loop)` | pour les **quatre tests déplacés**, qui les appellent. Élargissement **minimal** : la portée est exactement l'ancêtre qui possède déjà les deux items, ni `pub(crate)`, ni `pub`. |

Les tests déplacés continuent d'appeler `super::raison_de_desarmement` et
`super::filter_available_required_tools` **sans aucun changement de visibilité** :
ceux-là sont des items privés de l'ancêtre `agent_loop`, donc visibles depuis
l'enfant.

L'alternative — dupliquer les deux aides dans `pr_review_groom::tests` — est
écartée : deux constructeurs de registre qui dérivent séparément sont la forme
dont ce ticket corrige déjà une instance une couche plus haut (§ 3.5, « deux
booléens parallèles »). **La règle que la DoD porte est donc : aucune
visibilité de code de production élargie pour un test, et tout élargissement de
visibilité de test borné à `pub(in crate::agent_loop)`** — jamais `pub(crate)`,
jamais `pub`.

Ce qui reste dans `mod.rs`, et rien d'autre :

| ce qui reste | lignes |
|---|---|
| `mod pr_review_groom;` + `use pr_review_groom::ProprietesDuTour;` | 2 |
| champ `pub webhook_event_text: Option<&'a str>` sur `AgentParams` + doc de 2 lignes | 3 |
| paramètre `webhook_event_text` de `run_loop` + son passage à `filter_available_required_tools` | 4 |
| les trois sites d'appel internes (`run_agent_inner`, `run_silent_inner`, `run_team_agent_inner_impl`) | 3 |
| variante `RaisonDesarmement::TerritoireSansGroom` + doc d'une ligne | 2 |
| le bras de journalisation `handler_territory_forbids_groom` (INFO) | 4 |
| signature de `raison_de_desarmement` / `filter_available_required_tools` (le `bool` devient `ProprietesDuTour`) | 3 |
| la branche 5, en cinquième position, après la branche 4 | 4 |
| `tour_neutre()` et ses six appelants non-3004 : **inchangés, ils restent** | 0 |
| deux mots-clés de visibilité sur les aides de test (§ ci-dessus) | 0 net (2 modifiées) |

Les sites d'appel de test existants n'apparaissent pas dans ce tableau, et c'est
une mesure et non un oubli : le premier vol les a **déjà** migrés vers
`tour_neutre()`, qui ne bouge plus. Le re-vol n'a aucune édition de masse à
refaire, ce qui est une des trois raisons pour lesquelles il tient en un vol
(§ 11).

**La discipline qui tient le budget : aucune prose explicative nouvelle dans
`mod.rs`.** Chaque site y porte un marqueur `// mika#3004` et un lien
`[`pr_review_groom`]` ; le paragraphe vit dans le doc-module du sous-module.
C'est la condensation des ~180 lignes de doc-comment que le premier vol avait
ajoutées à `mod.rs`.

**Budget chiffré, et il est vérifiable à l'octet.** L'ajout net de `mod.rs` par
rapport à `origin/main` doit valoir **≤ 1 600 octets**, soit ≤ 1 046 159 octets
absolus et **≥ 2 417 octets de marge restante** sous le plafond — c'est-à-dire
moins de 40 % de la marge de 4 Ko consommée. La sonde, à écrire dans la DoD :

```
git show origin/main:crates/mika-agent/src/agent_loop/mod.rs | wc -c   # 1044559
wc -c crates/mika-agent/src/agent_loop/mod.rs                          # ≤ 1046159
bash scripts/check-secrets.sh --changed
```

Si le budget est dépassé après G1, **la correction est de condenser davantage le
doc, jamais d'ajouter `mod.rs` à `LARGE_FILE_ALLOWLIST`.** Le ticket l'interdit
nommément.

### 3.2 G2 — fermer les deux initialiseurs d'`AgentParams` restants

`crates/mika-cli/src/commands/chat.rs:336` et `:460` : ajouter
`webhook_event_text: None,` au littéral de struct.

`None` est le bon littéral et pas un bouchon : un tour de chat CLI ne naît pas
d'un webhook, il n'existe aucun texte d'avant-handler qui diffère du message du
tour, et `ProprietesDuTour::depuis` retombe alors sur `turn_message` — ce qui
préserve octet pour octet le comportement de mika#2757 hors du chemin webhook.
Un commentaire d'une ligne le dit au site.

### 3.3 G3 — ne rien toucher d'autre

Restent **inchangés**, parce qu'ils sont déjà verts et déjà mesurés :

- `crates/mika-agent/src/webhook_dispatch.rs` — le domaine Fallthrough lit
  `is_webhook_pr_event_domain` en négation au lieu de recopier la disjonction
  des deux préfixes. Un seul corps pose la conjonction, deux questions la lisent
  de signes opposés. `mika2517_the_fallthrough_domain_has_a_single_definition`
  reste satisfait, et `pr_review_groom.rs` **ne doit porter aucun littéral de
  préfixe** — il appelle le prédicat.
- `crates/mika-agent/src/server/handlers.rs` — `original_github_text` capturé
  avant les handlers, c'est la cause B.
- `crates/mika-agent/src/server/a2a.rs`, `tests/eval/harness.rs` (champ +
  méthode `.webhook_event_text()`), `tests/eval.rs` (câblage du module),
  `tests/eval/test_pr_review_no_groom_requirement_3004.rs` (316 lignes, trois
  tests verts).

Lignes de garde du ticket, tenues : `MIKA_MANAGER_*_WRITE` restent OFF, aucun
interrupteur n'est armé, **ni `spawn.rs` ni `server/mod.rs` ne sont modifiés**.

### 3.4 Option écartée, nommée

Descendre **toute** la famille de désarmement (`RaisonDesarmement`,
`raison_de_desarmement`, `filter_available_required_tools`,
`has_terminal_required_tool_failure` et leurs tests) hors de `mod.rs` rendrait
l'écart net *négatif* d'environ 25 Ko et achèterait de la marge pour longtemps.
Écarté ici : ce vol doit être le plus petit qui ferme AC1, et déplacer ~700
lignes existantes élargit la surface de revue sans rien fermer de plus. Ce vol
ne consomme que ~40 % de la marge ; le jour où elle redevient contraignante,
c'est un ticket à ouvrir, pas un geste à glisser dans celui-ci.

### 3.5 Refus argumentés, à ne pas rouvrir sans leur prémisse

- **Élargir la branche 5 à `is_dispatch_tool`.** Non. `run_claude_pilot` reste
  exigible sur le territoire des handlers : `self-dev-webhook-qa` relance un
  pilote sur un `hold`, `self-dev-webhook-ci` en relance un sur un CI rouge.
  Élargir fermerait ces deux flux. Ce qui est impossible sur ce territoire est
  le *groom* — un handler a déjà traité l'événement, et un événement de PR est
  par construction postérieur au plan.
- **Placer la branche 5 avant la branche 3 (`NonServi`).** Non. L'ordre est une
  propriété, pas une préférence de lecture : en production un `check_suite`
  répond **sous** `NonServi`, et la branche 5 n'y est jamais atteinte. C'est
  voulu — le ticket demande que l'exigence ne tienne pas, pas qu'elle tombe sous
  un nom particulier — et `mika3004_un_check_suite_repond_sous_non_servi_en_production`
  épingle les deux faits dans le même test.
- **Deux booléens parallèles au lieu de `ProprietesDuTour`.** Non. Deux booléens
  sont deux occasions de les calculer sur deux textes différents, et c'est
  exactement l'erreur que ce ticket corrige une couche plus haut. Un seul
  constructeur, des champs privés, pas de `Default`.
- **`INFO` et non `WARN` pour la branche 5.** Le désarmement est *nominal* sur ce
  domaine : il tirera des dizaines de fois par jour pour un comportement correct,
  et un `WARN` à ce volume est un `WARN` qu'on apprend à ignorer (les « 25+
  documented misfires » de mika#1469). Le champ `reason` sépare les formes par
  `GROUP BY`.

---

## 4. Le contrat de vérification

| contrôle | commande | attendu |
|---|---|---|
| taille de `mod.rs` | `wc -c crates/mika-agent/src/agent_loop/mod.rs` | ≤ 1 046 159 |
| détecteur de gros fichier | `bash scripts/check-secrets.sh --changed` | exit 0, `LARGE_FILE_ALLOWLIST` toujours vide |
| compilation complète | `cargo check --workspace --all-targets` | 0 erreur |
| recensement `AgentParams` | `git grep -nE '(^\|[^a-zA-Z])AgentParams \{' -- 'crates/**/*.rs'` | 7 sites, **7** portant `webhook_event_text` |
| cible Darwin (si disponible) | `cargo check --target aarch64-apple-darwin` | 0 erreur ; sinon le recensement ci-dessus tient lieu de preuve |
| AC2 + AC3 bout en bout | `cargo test -p mika-agent --test eval mika3004` | 3 passés |
| unitaires relogés | `cargo test -p mika-agent pr_review_groom` | 4 passés |
| non-régression désarmement | `cargo test -p mika-agent desarmement required_tools` | inchangé |
| axes exclusifs | `cargo test -p mika-agent mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` | passé |
| définition unique du domaine | `cargo test -p mika-agent mika2517_the_fallthrough_domain_has_a_single_definition` | passé |
| suite complète | `cargo test` | aucune régression |
| lints | `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` | propres — **et c'est la preuve que le motif du marqueur `clippy-errors-need-human` est parti** (§ 1.5) |
| visibilités élargies | `git diff origin/main...HEAD -- '*.rs' \| grep -E '^\+.*pub(\(crate\))? (fn\|struct)'` | aucune ligne pour un item de production ; les seuls `pub(in crate::agent_loop)` sont les deux aides de test du § 3.1 |
| PR cible | `gh pr view 3009 --repo senara-solutions/mika --json state,headRefOid` | `OPEN`, tête = `HEAD` poussé — **aucune seconde PR ouverte** |
| pipeline | `scripts/verify-pipeline.sh origin/main` | propre |

---

## 5. Ordre de commit

1. `fix(3004): les deux initialiseurs d'AgentParams de mika-cli` — G2 seul. Deux
   lignes, et c'est le commit qui rend le workspace compilable. Plancher :
   `cargo check --workspace --all-targets` passe.
2. `refactor(3004): le domaine du tour descend dans son propre module` — G1.
   Plancher : les quatre tests unitaires passent depuis leur nouveau site, et
   `wc -c mod.rs` ≤ 1 046 159.

**Deux commits, et pas un troisième.** La version antérieure de ce plan en
proposait un conditionnel (« des sondes de taille, si la DoD en demande ») : il
est retiré. La sonde de taille n'est pas un livrable à écrire —
`scripts/check-secrets.sh` la porte déjà et tourne en CI (§ 10). Un plan ne
laisse pas un commit à la discrétion de l'implémenteur.

Les commits 1 et 2 sont séparés **parce que le premier est le seul dont CI peut
rendre compte sans le second** : un workspace qui ne compile pas ne dit rien de
la taille d'un fichier.

### 5.1 Fin de pipeline — il n'y a **pas** de `gh pr create`

C'est le point que le § 1.5 rend actionnable, et il se joue aux derniers tours,
là où les vols meurent. #3009 est OPEN sur exactement cette tête de branche :

1. `git push` sur `fix/3004/dispatch-un-tour-pr-review-de-mika-dev`. La PR #3009
   se met à jour d'elle-même, et CI se rejoue dessus.
2. **Ne pas appeler `gh pr create`** — il refusera, une PR existe pour cette
   tête. Si une tentative a quand même lieu et échoue, ce n'est pas un échec de
   pipeline : la PR est #3009.
3. Corriger le titre de #3009, qui est aujourd'hui un titre de commit :
   `gh pr edit 3009 --repo senara-solutions/mika --title "fix(3004): un tour pr_review n'exige plus l'outil de groom"`,
   et réécrire son corps (règle mika#2211 : `pr-body.md` à la racine du
   worktree, `--body-file pr-body.md`, puis `rm pr-body.md`).
4. **Ne pas chercher à lever l'exclusion d'auto-resume** (§ 1.5, point 3) : elle
   est durable par conception et aucun geste de ce vol ne la réarme. La sortie
   de draft reste la décision de l'opérateur, une fois CI verte.

---

## 6. Ce que le plan de #3009 reprend, et ce qu'il écarte

**Repris, et c'est l'essentiel** — le plan de 624 lignes a trouvé juste :

- le site (`filter_available_required_tools` / `raison_de_desarmement`) et
  l'argument pour lequel la correction y tient plutôt qu'au prompt (§ 2) ;
- les deux causes cumulées, dont la cause B — le texte d'après les handlers — qui
  est ce qu'une investigation superficielle aurait manqué (§ 2) ;
- `ProprietesDuTour` à un seul constructeur, et le refus des deux booléens (§ 3.5) ;
- la cinquième variante en cinquième position, et le test qui épingle que la
  production répond sous `NonServi` (§ 3.5) ;
- la lecture en négation du territoire des handlers dans `webhook_dispatch.rs` (§ 3.3) ;
- la fixture de la trace `3b771914` et ses trois tests de bout en bout (§ 3.3).

**Écarté :**

- **son recensement d'`AgentParams`** — « 25 sites dans 17 fichiers » est un
  artefact de grep (§ 1.4). Le recensement correct est 7 sites dans 4 fichiers,
  et le plan livre le grep à frontière de mot qui le prouve ;
- **sa localisation du code** — tout dans `agent_loop/mod.rs`, ce qui a fait
  rougir *Secret Scan*. Le re-vol descend tout dans
  `agent_loop/pr_review_groom.rs` (§ 3.1) ;
- **son volume de doc-comment dans `mod.rs`** — ~180 lignes de prose française
  ajoutées à un fichier à 4 Ko du plafond. Condensées dans le doc-module du
  sous-module ;
- **sa structure en neuf sections de conception** — le dessin est fait et vert ;
  ce plan-ci est un plan de livraison, pas de conception.

**Ce que le re-vol ne fait pas :** la draft #3009 n'est ni fermée ni sortie de
draft par ce vol. Les commits atterrissent sur sa branche — qui vit, § 1.5 — CI
les juge, et la sortie de draft reste la décision de l'opérateur. Ce que le vol
fait sur #3009 se borne à deux gestes de forme : pousser, et corriger le titre
et le corps (§ 5.1).

---

## 7. Fichiers touchés

| fichier | geste |
|---|---|
| `crates/mika-agent/src/agent_loop/pr_review_groom.rs` | **nouveau** — `ProprietesDuTour`, doc-module, 3 fixtures, `tour_neutre()`, 4 tests |
| `crates/mika-agent/src/agent_loop/mod.rs` | retrait de ~480 lignes relogées ; ne garde que la plomberie du § 3.1 |
| `crates/mika-cli/src/commands/chat.rs` | 2 lignes `webhook_event_text: None` |
| `docs/plans/2026-11-01-005-…-plan.md` | ce plan |

Inchangés, et c'est une assertion : `webhook_dispatch.rs`, `server/handlers.rs`,
`server/a2a.rs`, `tests/eval/harness.rs`, `tests/eval.rs`,
`tests/eval/test_pr_review_no_groom_requirement_3004.rs`,
`scripts/check-secrets.sh`, `spawn.rs`, `server/mod.rs`.

---

## 8. Definition of Done

- [ ] `crates/mika-agent/src/agent_loop/pr_review_groom.rs` existe et porte
      `ProprietesDuTour`, son doc-module, les trois fixtures, `tour_neutre()` et
      les quatre tests unitaires mika#3004.
- [ ] `pr_review_groom.rs` ne contient **aucun** littéral de préfixe
      (`[GitHub] PR `, `[GitHub] Check suite `) hors de ses constantes de
      fixture : il appelle `is_webhook_pr_event_domain`.
- [ ] `tour_neutre()` est **resté** dans `mod.rs`, et ses six appelants non-3004
      sont inchangés.
- [ ] Aucune visibilité de code de **production** élargie pour satisfaire un
      test. Les seuls élargissements du diff sont : `pub(super)` sur
      `ProprietesDuTour` / `::depuis` (production), et
      `pub(in crate::agent_loop)` sur `make_resolved_skill_tool` /
      `make_registry_with_tool`. **Aucun `pub(crate)`, aucun `pub` nu.**
- [ ] Les deux aides de test ne sont **pas** dupliquées dans
      `pr_review_groom.rs`.
- [ ] `wc -c crates/mika-agent/src/agent_loop/mod.rs` ≤ **1 046 159** octets, et
      l'écart net avec `origin/main` ≤ **1 600** octets.
- [ ] `LARGE_FILE_ALLOWLIST` de `scripts/check-secrets.sh` est toujours vide ;
      `bash scripts/check-secrets.sh --changed` sort en 0.
- [ ] `git grep -nE '(^|[^a-zA-Z])AgentParams \{' -- 'crates/**/*.rs'` rend 7
      sites, et les 7 portent `webhook_event_text`.
- [ ] `cargo check --workspace --all-targets` : 0 erreur.
- [ ] `cargo test -p mika-agent --test eval mika3004` : 3 passés.
- [ ] `cargo test -p mika-agent pr_review_groom` : 4 passés.
- [ ] `cargo test` : aucune régression.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` et
      `cargo fmt --check` : propres.
- [ ] `scripts/verify-pipeline.sh origin/main` : propre.
- [ ] `spawn.rs` et `server/mod.rs` n'apparaissent pas dans
      `git diff --name-only origin/main...HEAD`.
- [ ] Les commits sont poussés sur
      `fix/3004/dispatch-un-tour-pr-review-de-mika-dev`, **aucune seconde PR
      n'est ouverte**, et `gh pr view 3009 --json state,headRefOid` rend `OPEN`
      sur la tête poussée.
- [ ] Le titre de #3009 ne porte plus un titre de commit, et son corps est
      réécrit (§ 5.1).

---

## 9. Acceptance criteria

Transcrits du corps de senara-solutions/mika#3004 :

- **AC1.** Sur un tour de mika-dev déclenché par `pr_review` ou `check_suite`,
  pour un ticket dont une PR liée est ouverte, l'outil de groom n'est pas exigé.
  Le plan nomme le site qui l'exigeait, et dit pourquoi la correction tient à ce
  site plutôt qu'au prompt. → § 2.
- **AC2.** Test dans le même appel, avec contrôle négatif : le tour `pr_review`
  de la fixture n'exige pas `run_claude_pilot_groom`. Un tour déclenché par le
  label `ready` sur un ticket sans PR l'exige toujours. →
  `mika3004_un_tour_pr_review_ne_prend_pas_le_re_prompt_de_groom` et
  `mika3004_un_label_ready_sans_pr_exige_toujours_le_groom`.
- **AC3.** La trace `3b771914` sert de fixture. →
  `PR_REVIEW_3004_EVENT` / `PR_REVIEW_3004_PRE_DIGEST`, rejouées de bout en bout
  dans `test_pr_review_no_groom_requirement_3004.rs`.

---

## 10. Fire-Disposition

Le vol livre des détecteurs — les quatre tests unitaires relogés et les trois
tests de bout en bout — donc la section est due.

**Option retenue : (a), exception nommée en allowlist, avec une allowlist
vide — et le vide est mesuré, pas supposé.**

Détail d'implémentation :

1. **Population des détecteurs de comportement : vide après le vol, et déjà
   vide avant lui.** Les sept tests sont verts sur la branche aujourd'hui
   (§ 1.2) : le seul site que le détecteur accusait était le défaut lui-même,
   corrigé par les commits de la branche. Il n'existe donc aucune violation
   préexistante à excepter, et aucune entrée d'allowlist à écrire. Le détecteur
   atterrit **armé**, sans exception, sans `#[ignore]`.
2. **`LARGE_FILE_ALLOWLIST` de `scripts/check-secrets.sh` reste vide**, et c'est
   un engagement du vol plutôt qu'un constat : le geste G1 existe précisément
   pour n'avoir pas à y écrire `mod.rs`. La DoD l'assert.
3. **Le détecteur déjà armé que le vol réinterprète :**
   `mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs`. Son invariant n'a pas
   changé de contenu, il a changé de **raison** d'être vrai — vrai par
   construction depuis que le domaine Fallthrough lit
   `is_webhook_pr_event_domain` en négation, là où il l'était par coïncidence. Il
   reste, armé, parce qu'il est ce qui rougit le jour où quelqu'un défait ce sens
   de lecture. Aucune exception.
4. **Pas d'auto-nettoyage à écrire**, puisqu'il n'y a pas d'exception à rendre
   stale. La propriété équivalente est portée par
   `mika3004_un_check_suite_repond_sous_non_servi_en_production`, qui épingle
   dans le même test que la branche 5 n'est pas atteinte en production *et*
   pourquoi c'est voulu : un test qui ne dirait que le premier fait se lirait un
   jour comme une branche morte à supprimer.

---

## 11. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/pr_review_groom.rs` (nouveau : struct + doc-module condensé + 3 fixtures + 4 tests) | 300 |
| `crates/mika-agent/src/agent_loop/mod.rs` (plomberie nette conservée, § 3.1, dont 2 mots-clés de visibilité à 0 ligne nette) | 30 |
| `crates/mika-cli/src/commands/chat.rs` (2 sites) | 2 |

Total estimé : 332 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi il tient.** Trois raisons mesurées :

1. **Les trois AC sont déjà verts** (§ 1.2) : le vol ne conçoit rien et
   n'implémente aucun comportement nouveau. Il répare deux défauts de livraison.
2. **Le retrait de ~480 lignes de `mod.rs` n'est pas compté**, et c'est délibéré.
   La borne de mika#2693 est en **tours** — les lignes n'en sont qu'un proxy. Une
   suppression ou un déplacement coûte une édition et aucune revue de logique
   nouvelle ; les compter reviendrait à facturer deux fois le même texte, une
   fois à son départ et une fois à son arrivée. Les 300 lignes de
   `pr_review_groom.rs` **sont** ce texte, comptées une fois, à l'arrivée.
3. **Le découpage que le ticket autorisait n'est pas nécessaire.** La constrainte 3
   du corps réservait AC3 — le rejeu de bout en bout — à un vol 2 « si le vol 1
   ne le contient pas sans risque pour le plafond ». Il le contient à coût nul :
   `test_pr_review_no_groom_requirement_3004.rs` est déjà écrit, déjà câblé et
   déjà vert, et le vol n'y touche pas (§ 3.3). Zéro ligne, zéro tour.

4. **Aucune édition de masse à refaire.** Le premier vol a déjà migré les ~20
   sites d'appel de test vers `tour_neutre()`, qui ne bouge plus (§ 3.1). Le
   re-vol touche trois fichiers, dont un de deux lignes.
5. **La fin de pipeline est désamorcée d'avance** (§ 5.1). Les trois vols morts
   du 2026-10-14 ont tenu sur le travail et cédé sur la fin de pipeline, qui
   coûte des tours et non des lignes — et ici elle portait un piège mesurable :
   un `gh pr create` qui refuse sur une tête déjà portée par #3009. Le plan le
   dit avant, pour qu'il ne s'apprenne pas au tour 190.

332 lignes contre 429 : la borne par vol tient avec ~23 % de marge, et le seuil
par PR de 1 000 lignes n'est pas approché. Pas de `Tracked in:` — il n'y a pas
de suite.

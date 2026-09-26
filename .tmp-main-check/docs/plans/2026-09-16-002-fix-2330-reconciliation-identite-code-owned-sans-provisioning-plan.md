---
title: Les sections identité code-owned se réconcilient même quand le provisioning est désactivé - Plan
type: fix
date: 2026-09-16
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: mika#2330
---

# Les sections identité code-owned se réconcilient même quand le provisioning est désactivé - Plan

**Ticket :** mika issue#2330 — *Brique de suite de mika#2295 (AC2-7) — le réconciliateur n'applique pas `[context.history]` aux identités existantes*
**Chemin de réparation :** boucle autonome (dispatch dev-groom → dev-pilot)
**Fichiers principaux :** `crates/mika-agent/src/well_known_agents.rs`, `docs/configuration.md`, `crates/mika-agent/docs/configuration.md`, `CLAUDE.md`, `docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md`

---

## Goal Capsule

- **Objective :** au démarrage, un agent well-known **déjà provisionné** reçoit sur disque les sections d'identité **code-owned** que sa spec déclare, *même* avec `MIKA_DISABLE_AGENT_PROVISIONING=1` — et son `config.toml` n'est pas touché.
- **Means :** la branche `disabled` de `provision_well_known_agents` cesse d'être un `return` sec ; elle exécute la réconciliation d'identité seule, sans bootstrap et sans `reconcile_well_known_config`.
- **Autorité :** le corps du ticket mika#2330 > son commentaire 1 (samidarko, correction de racine, choisit l'**option 1**) > son commentaire 2 (op manuelle, contournement) > ce plan > le code existant.
- **Conditions d'arrêt :** si la mesure M4 avait montré que `[context.history]` est déclarée par les quatre specs, la portée aurait été de quatre agents ; elle ne l'est pas (voir M4) et le plan sert **un** agent par conception. Si l'implémentation découvre que `reconcile_well_known_identity` écrit ailleurs que dans `identity.toml`, s'arrêter : le découplage ne serait plus sûr.
- **Profil d'exécution :** pipeline `/mika` dans le worktree `feat/2330/le-r-conciliateur-n-crit-pas-context` ; PR gated QA ; merge par l'opérateur ; effet au **prochain démarrage** de mika-spirit.

---

## Product Contract

### Summary

`provision_well_known_agents(home, settings, disabled = true)` retourne aujourd'hui après un simple `warn!`. Ce `return` bloque **trois** choses d'un seul geste : la création d'agents, la réécriture de `config.toml`, et la réconciliation des sections d'identité code-owned. L'opérateur n'en voulait qu'une — geler `config.toml` (Z.AI direct, kimi) — et paie les deux autres sans l'avoir demandé.

Après ce correctif, le mode désactivé fait exactement ce que son nom promet et rien de plus : aucun agent créé, aucun `config.toml` réécrit, mais les sections déclarées dans `CODE_OWNED_IDENTITY_SECTIONS` sont appliquées aux identités déjà sur disque, avec le journal qui existe déjà (`identity_reconcile.complete`, porteur de `reconciled_paths`).

### Problem Frame

mika#2327 a rendu `context.history` code-owned (`scope = "session"`, `max_tokens = 8000`) pour borner la fenêtre de mika-arch. Le code est mergé et **inerte en prod** : les identités datent du 26/07, `write_default_if_missing` ne réécrit jamais un `identity.toml` existant, et le seul chemin qui aurait pu écrire la section — le réconciliateur — ne s'exécute pas parce que le provisioning est désactivé.

Le réconciliateur n'est donc **pas cassé** (commentaire 1 de samidarko, confirmé par M2). Il est hors d'atteinte. Le défaut est un couplage : une décision de protection sur `config.toml` a désarmé, en silence, un mécanisme de propagation sur `identity.toml`.

### Mesures — exécutées le 2026-09-16 dans le worktree

Toutes lues dans l'arbre à `1732dfcf`. Elles sont ce qui distingue ce plan d'une paraphrase du ticket : **deux d'entre elles (M4, M5) déplacent le périmètre attendu**.

- **M1 — la racine est un `return` qui groupe trois effets.** `well_known_agents.rs:807-813` : `if disabled { warn!(…); return; }`. En aval du `return`, pour un agent existant, la boucle appelle `reconcile_well_known_identity` (l. 822) *puis* `reconcile_well_known_config` (l. 825). Les trois effets (bootstrap, identité, config) sont derrière la même garde.
- **M2 — le réconciliateur d'identité est correct et n'a pas besoin d'être réécrit.** `reconcile_well_known_identity` (l. 565) : rendu de la spec, parse des deux côtés, remplacement **par chemin pointé** uniquement, écriture atomique `tmp` + `rename`, isolation des pannes par `warn!` + `return` sur l'agent courant, et idempotence (`identity_reconcile.in_sync`, zéro écriture quand tout concorde). `"context.history"` est bien dans `CODE_OWNED_IDENTITY_SECTIONS` (l. 503). Rien à corriger ici — seulement à atteindre.
- **M3 — ce que l'opérateur protège est un écrasement total, pas une fusion.** `reconcile_well_known_config` (l. 705) : `std::fs::write(tmp, expected)` où `expected = spec.config_toml`, puis `rename`. Le fichier entier est remplacé ; aucune notion de section. La crainte du commentaire 1 (« le provisioning réécrirait aussi les `config.toml` ») est exacte au sens fort, et c'est pourquoi cette fonction reste **hors d'atteinte** du mode désactivé.
- **M4 — `[context.history]` n'est déclarée que par la spec de mika-arch.** Unique occurrence dans une identité : `build_mika_arch_identity`, l. 408-410. Ni `MIKA_DEV_IDENTITY`, ni `MIKA_QA_IDENTITY`, ni `MIKA_TEST`. Or le réconciliateur fait `continue` quand la spec ne définit pas le chemin (`get_path(&expected, path)` → `None`, l. 627-630). **Conséquence : ce correctif écrira `[context.history]` pour mika-arch et pour lui seul.** Ce n'est pas une limitation de l'implémentation, c'est le contrat : la valeur de fenêtre est une propriété du rôle, et le rôle one-shot est celui de l'architecte (mika#2295 AC7). Donner une fenêtre `session` à mika-dev serait une décision produit que personne n'a prise.
- **M5 — mika-prime et mika-relay ne sont pas des agents well-known.** `WELL_KNOWN_AGENTS` (l. 424) = `[MIKA_DEV, MIKA_TEST, MIKA_QA, MIKA_ARCH]`. Aucune spec n'existe pour mika-prime ; aucune réconciliation n'est possible pour lui, ni avant ni après ce correctif. Le ticket constate « les 4 vérifiés = 0 (arch/dev/qa/prime) » ; **trois de ces quatre resteront à 0 après le correctif, et c'est le comportement attendu.**
- **M6 — le test cité par le ticket n'exerce aucun chemin.** `mika2295_history_block_is_reconciled_onto_already_provisioned_agents` (l. 1666) est un `assert!(CODE_OWNED_IDENTITY_SECTIONS.contains(&"context.history"))`. Il garde la constante — utile — mais son nom promet une réconciliation qu'il ne déclenche jamais. Le ticket a raison : « le test passe, la prod non ».
- **M7 — un chemin peut rendre le correctif inerte pour l'agent cible, et il ne dit que `warn!`.** L'identité de mika-arch est `IdentitySource::Computed` ; `build_mika_arch_identity` échoue si `kg_docs_roots` est vide (l. 360-369). Le réconciliateur émet alors `identity_reconcile.skipped / reason = "render_failed"` et passe. Un `MIKA_KG_DOCS_ROOTS` absent au démarrage produirait donc, après déploiement, exactement le symptôme d'aujourd'hui — avec un WARN noyé pour toute explication. La sonde post-déploiement doit exiger une ligne **nommant mika-arch**, jamais se contenter d'une absence d'erreur.
- **M8 — deux appelants, tous deux servis par le même correctif.** `server/mod.rs:745` (démarrage, sous `if settings.dev_mode`) et `mika-cli/src/init.rs:185` (construction paresseuse, quand le `config.toml` d'un agent manque). Les deux passent `settings.disable_agent_provisioning` ; corriger la fonction les corrige tous les deux, sans toucher aux sites d'appel.

### Key Decisions

- **D1 — l'option 1 du commentaire 1 est retenue, l'option 2 est close.** Le geste manuel du commentaire 2 reste vrai et documenté pour ce que ce correctif ne couvre pas (M5 : agents hors spec) ; il cesse d'être le geste standard pour les agents well-known.
- **D2 — le découplage porte sur *toutes* les sections de `CODE_OWNED_IDENTITY_SECTIONS`, pas sur `context.history` seule.** Cette constante *est* la déclaration « ces valeurs appartiennent au code », et sa documentation le dit déjà (« Sections NOT listed here are preserved verbatim from the on-disk file (operator-owned) »). Un correctif taillé sur une seule section aurait re-signé le même défaut le jour de la section suivante.
- **D3 — `config.toml` n'est jamais touché en mode désactivé, ni par écrasement ni par fusion.** Aucune ligne de ce plan n'appelle `reconcile_well_known_config` sous `disabled`. C'est l'intention opérateur prise à la lettre.
- **D4 — une seule sémantique de réconciliation dans les deux modes (écrire-si-différent), pas un mode dégradé écrire-si-absent.** Écrire-si-absent respecterait la lettre du ticket (« les sections code-owned **manquantes** ») et ne pourrait écraser aucune édition manuelle — mais il créerait deux sémantiques selon un drapeau, donc deux réponses possibles à « quelle est la valeur en vigueur ? », et il ne propagerait jamais un *changement* de valeur ultérieur. Le coût de D4 est nommé en R1 et payé par une mesure préalable, pas par une asymétrie permanente.
- **D5 — le message du `warn!` de démarrage est réécrit.** Il promet aujourd'hui que les agents « will not be auto-created **or updated** ». Après D2 c'est faux pour l'identité. Un journal qui décrit un comportement que le code n'a plus est pire qu'un journal absent : il répond à la question et il répond faux.

### Requirements

1. Avec `disabled = true`, pour chaque agent well-known **qui existe déjà sur disque**, les sections de `CODE_OWNED_IDENTITY_SECTIONS` que sa spec définit sont appliquées à son `identity.toml`.
2. Avec `disabled = true`, aucun agent n'est créé (pas de `bootstrap_agent`, pas d'écriture de `soul.md`, pas d'écriture de `config.toml` initial).
3. Avec `disabled = true`, `reconcile_well_known_config` n'est pas appelée ; le `config.toml` d'un agent existant est inchangé **octet pour octet**.
4. L'écriture reste atomique, isolée par agent et idempotente (deuxième démarrage → `identity_reconcile.in_sync`, zéro écriture).
5. Le journal de démarrage décrit exactement ce que le mode désactivé fait et ne fait pas.
6. Un test exerce le **chemin de démarrage réel sur disque** — appel de `provision_well_known_agents(..., disabled = true)` sur un `identity.toml` dépourvu de la section — et échoue sur l'arbre actuel.
7. Le comportement avec `disabled = false` est inchangé.

### Scope Boundaries

**Dans le périmètre :** la branche `disabled` de `provision_well_known_agents` ; les tests de ce module ; la documentation du drapeau.

**Hors périmètre, délibérément :**
- **Donner une `[context.history]` à mika-dev, mika-qa ou mika-test** (M4). Leur rôle n'est pas one-shot ; choisir leur valeur de fenêtre est une décision produit qui appartient à son propre ticket, avec sa propre mesure.
- **mika-prime et mika-relay** (M5). Sans spec, il n'y a rien contre quoi réconcilier. Le geste manuel du commentaire 2 reste leur voie.
- **`reconcile_well_known_config`**, sous quelque forme que ce soit — y compris une version « par section » qui préserverait `openrouter_model`. Ce serait un autre ticket, avec sa propre notion de ce qui, dans une config, appartient au code.
- **La cause pour laquelle `MIKA_DISABLE_AGENT_PROVISIONING=1` est posé en prod.** Ce correctif rend le drapeau plus étroit et plus honnête ; il ne propose pas de le retirer.
- **Le gate `if settings.dev_mode`** de `server/mod.rs:745`. Hors dev-mode, aucun agent well-known n'est censé exister, donc il n'y a rien à réconcilier ; élargir cette garde changerait une population sans rapport avec le défaut mesuré.

### Deferred to Follow-Up Work

- Fenêtre d'historique bornée pour mika-dev / mika-qa / mika-prime si la mesure en montre le besoin (ticket à ouvrir, après lecture de `context_window_assembled` sur ces trois agents).
- Réconciliation par section de `config.toml`, qui permettrait de retirer `MIKA_DISABLE_AGENT_PROVISIONING=1` sans perdre les choix opérateur (ticket à ouvrir).

---

## Planning Contract

### Key Technical Decisions

- **KTD1 — la forme du correctif est une boucle dans la branche `disabled`, pas un nouveau point d'entrée.** Ajouter une fonction publique `reconcile_code_owned_identities()` appelée depuis `server/mod.rs` créerait un second site de démarrage à tenir en phase avec le premier, et laisserait `init.rs` (M8) derrière. Une boucle de cinq lignes dans la fonction qui porte déjà la garde sert les deux appelants sans nouvelle surface.
- **KTD2 — aucun nouveau nom d'événement, aucune signature changée.** `identity_reconcile.{complete,in_sync,skipped}` porte déjà `agent` et `reconciled_paths`, et le `warn!` réécrit (D5) est émis dans le même démarrage, quelques lignes plus haut : l'opérateur qui demande « pourquoi mon `identity.toml` a-t-il bougé alors que j'ai désactivé le provisioning ? » a sa réponse en deux lignes voisines. Ajouter un champ `provisioning_disabled` imposerait un quatrième paramètre à `reconcile_well_known_identity` et la mise à jour de dix sites de test pour un gain marginal. Rejeté, reprenable si l'architecte le juge nécessaire.
- **KTD3 — le test de non-régression est comportemental, pas un scan de source.** La classe de régression visée (« quelqu'un remet un `return` sec ») rend le comportement faux, pas invisible : un test comportemental rougit. Contraste assumé avec `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`, où la régression n'aurait rendu aucune assertion fausse.
- **KTD4 — le test négatif fabrique l'état du 26/07 en retirant la section d'une identité fraîchement provisionnée**, plutôt qu'en figeant un `identity.toml` littéral dans le test. Un littéral dériverait silencieusement de la spec ; partir du provisionnement réel garantit que le test échoue pour la bonne raison.

### Fire-Disposition

Une fois mergé, l'effet n'est visible qu'au **prochain démarrage de mika-spirit** : c'est le démarrage qui écrit. Le commentaire 2 a raison de dire qu'aucun redémarrage n'est requis pour qu'une identité *déjà écrite* prenne effet (relue à chaque tour via `load_identity_async`) — mais ici l'écriture elle-même est l'objet du correctif, donc le déploiement doit être suivi d'un redémarrage pour être constaté.

### Assumptions

- **A1 — `dev_mode` est vrai en production.** Étayée : le ticket rapporte les avertissements « provisioning disabled » au démarrage, et ce `warn!` se trouve *à l'intérieur* de `provision_well_known_agents`, dont l'appel est sous `if settings.dev_mode` (M8). Si A1 était fausse, ce correctif serait inerte et la cause serait ailleurs — la sonde P1 le détecte immédiatement.
- **A2 — `MIKA_KG_DOCS_ROOTS` est posée en production.** Étayée indirectement : mika-arch existe sur disque et fonctionne, or il n'aurait pas pu être provisionné sans elle. À **vérifier avant déploiement** (V1) parce que M7 en fait le seul chemin d'inertie silencieuse.
- **A3 — aucune édition manuelle opérateur ne vit dans une section code-owned d'un `identity.toml` en production.** Non étayée : à mesurer avant déploiement (V2). C'est l'objet du risque R1.

---

## Implementation Units

### U1. La branche `disabled` réconcilie l'identité, et elle seule

`crates/mika-agent/src/well_known_agents.rs`, dans `provision_well_known_agents` (l. 807).

Remplacer le `return` sec par une boucle qui, pour chaque `spec` de `WELL_KNOWN_AGENTS` telle que `mika_common::agent::agent_exists(home_dir, spec.name)`, appelle `reconcile_well_known_identity(home_dir, spec, settings)` — puis retourne. Ni `bootstrap_agent`, ni écriture de `soul.md`, ni `reconcile_well_known_config` ne doivent apparaître dans cette branche.

Mettre à jour le doc-comment de la fonction : la phrase « When `disabled` is true, logs a warning and returns without changes » devient fausse et doit nommer les trois effets séparément (création : non ; `config.toml` : non ; sections identité code-owned : oui).

### U2. Le journal de démarrage dit la vérité

Même fonction, le `warn!` de la branche `disabled`. Le texte actuel (« will not be auto-created or updated ») devient : provisioning désactivé → aucun agent créé, aucun `config.toml` réécrit, **les sections d'identité code-owned restent réconciliées**. Conserver le niveau WARN : l'état reste digne d'être signalé, et le niveau est déjà collecté.

### U3. Test négatif sur le chemin de démarrage réel

`crates/mika-agent/src/well_known_agents.rs`, module `tests`. Deux tests, séparés pour que l'échec nomme sa moitié.

**`mika2330_code_owned_identity_is_written_at_boot_when_provisioning_is_disabled`**
1. `home` temporaire, `agents/` créé ; `provision_well_known_agents(home, &test_settings_with_kg_roots(), false)` pour obtenir un mika-arch réellement provisionné.
2. Fabriquer l'état du 26/07 : lire `agents/mika-arch/identity.toml`, le parser en `toml::Value`, **retirer** la clé `history` de la table `context`, réécrire.
3. `provision_well_known_agents(home, &test_settings_with_kg_roots(), /* disabled = */ true)`.
4. Assertions sur le fichier **relu depuis le disque**, parsé en `crate::prompt::Identity` : `context.history.scope == HistoryScope::Session` et `context.history.max_tokens == Some(8000)`.

Ce test échoue sur l'arbre actuel à l'étape 4 — c'est ce qui en fait un test négatif valide (exigence 6).

**`mika2330_disabled_provisioning_still_creates_nothing_and_leaves_config_untouched`**
1. Même préparation, puis écrire dans `agents/mika-arch/config.toml` un contenu opérateur distinct de `spec.config_toml` (par exemple un `openrouter_model` autre que celui de la spec) et en capturer les octets.
2. Supprimer entièrement le répertoire d'un autre agent well-known (mika-qa) pour disposer d'un agent absent.
3. `provision_well_known_agents(home, …, true)`.
4. Assertions : le `config.toml` de mika-arch est **byte-identique** à ce qui a été écrit en 1 ; `agent_exists(home, "mika-qa")` reste faux.

Ajouter un troisième test court d'idempotence — deux appels consécutifs avec `disabled = true`, le `identity.toml` inchangé entre les deux — si le coût est nul ; sinon le couvrir par la lecture de `identity_reconcile.in_sync` en sonde P2.

### U4. Le test tautologique porte un nom qui dit ce qu'il fait

Renommer `mika2295_history_block_is_reconciled_onto_already_provisioned_agents` (l. 1666) en `mika2295_history_block_is_declared_code_owned`, et réécrire son doc-comment pour qu'il se présente comme la garde de la **constante**, en renvoyant aux tests de U3 pour la preuve comportementale. Ne pas le supprimer : la garde de la constante est réelle et distincte.

### U5. La documentation cesse de décrire l'ancien couplage

- **`CLAUDE.md`** — l'entrée `MIKA_DISABLE_AGENT_PROVISIONING` dit « prevents `dev_mode` from creating or updating agent identity files, allowing manual edits to persist across restarts/deploys ». Après U1, faux pour les sections code-owned. Réécrire en nommant les trois effets, et dire ce que le drapeau protège encore (`config.toml`, `soul.md`, sections operator-owned de `identity.toml`).
- **`docs/configuration.md`** — trois emplacements mesurés : l. 418 (tableau `Settings`), l. 633 et l. 693 (tableaux de variables d'environnement).
- **`crates/mika-agent/docs/configuration.md`** — copie crates.io ; synchroniser via `scripts/sync-agent-docs.sh`, faute de quoi le job CI `docs-sync` échoue.
- **`docs/solutions/architecture-patterns/well-known-agent-config-toml-override.md`, l. 43** — la phrase « Operators can still set `MIKA_DISABLE_AGENT_PROVISIONING=1` to freeze a hand-edited runtime config across deploys » reste **vraie** et n'a pas à être retirée ; ajouter la précision que ce gel couvre `config.toml` et non les sections d'identité code-owned.
- **`docs/solutions/architecture-patterns/well-known-agent-provisioning-dev-mode.md`** — vérifier à l'implémentation s'il décrit le comportement du mode désactivé ; corriger le cas échéant.

---

## Verification Contract

**Avant déploiement (les deux sont des haltes, pas des cases à cocher) :**

- **V1 (A2, M7)** — vérifier que `MIKA_KG_DOCS_ROOTS` (ou `kg_docs_roots`) est posée pour le processus mika-spirit de production. Absente, le correctif est inerte pour mika-arch et ne le dira qu'en WARN.
- **V2 (A3, R1)** — pour chacun des agents well-known présents sur disque, comparer les sections de `CODE_OWNED_IDENTITY_SECTIONS` de l'`identity.toml` en place avec celles rendues par la spec. Toute divergence hors `context.history` est une édition opérateur que ce correctif écrasera au prochain démarrage : la porter à samidarko **avant** le déploiement, pas après.

**Automatisé :** `cargo test -p mika-agent well_known_agents` (les trois tests de U3 passent ; le nouveau test négatif échoue sur l'arbre pré-correctif), `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills` si le job le couvre, et le job CI `docs-sync` après `scripts/sync-agent-docs.sh`.

**Sondes post-déploiement (après redémarrage) :**

- **P1 — le chemin s'exécute.** `grep identity_reconcile $MIKA_SPIRIT_LOG_FILE` doit rendre au moins une ligne **nommant `mika-arch`**. Une ligne `identity_reconcile.complete` avec `reconciled_paths` contenant `context.history` est le succès attendu au premier démarrage ; `identity_reconcile.in_sync` l'est aux suivants, et l'est aussi au premier si le patch manuel du commentaire 2 est encore en place. **Zéro ligne nommant mika-arch est une halte** — c'est la signature de A1 fausse ou de M7.
- **P2 — le correctif n'a pas dépassé sa cible.** `grep identity_reconcile.complete $MIKA_SPIRIT_LOG_FILE | jq .reconciled_paths` : tout chemin autre que `context.history` est une édition opérateur écrasée, et V2 aurait dû l'annoncer. S'il en apparaît une qui n'était pas dans V2, halte.
- **P3 — `config.toml` intact.** Comparer les `config.toml` des agents well-known avant/après redémarrage : `openrouter_model` (Z.AI, kimi) inchangés. Toute modification est une régression de D3 et justifie de reposer `MIKA_DISABLE_AGENT_PROVISIONING` avec un correctif révoqué.
- **P4 — l'effet mesuré, celui du ticket d'origine.** Au groom suivant, `context_window_assembled` pour mika-arch : `distinct_sessions = 1`, `history_bytes` effondré, `input_tokens` sous 40 k. Repère du commentaire 2 (mesuré à la main le 2026-09-15) : 201 244 → 0 octets, 83-89 k → 28-32 k de tokens. **`truncated_messages` peut rester à 0 et ce n'est pas un échec** : sous portée `session`, une première passe ne remplit pas les 8 000 tokens, donc il n'y a rien à tronquer — c'est `scope` qui fait le travail, `max_tokens` n'est qu'un plafond.
- **P5 — la lecture à ne pas faire.** mika-dev, mika-qa et mika-prime **n'auront pas** de `[context.history]` après ce correctif (M4, M5) et c'est le comportement attendu. Conclure de leur absence que le correctif a échoué serait rejouer exactement l'erreur de diagnostic que ce ticket a dû corriger une fois.

---

## Definition of Done

- La branche `disabled` de `provision_well_known_agents` réconcilie les identités code-owned des agents existants et rien d'autre (U1).
- Le `warn!` de démarrage décrit les trois effets séparément (U2).
- Les tests de U3 sont en place et verts ; le test d'écriture échouait sur l'arbre pré-correctif.
- Le test de M6 est renommé et son doc-comment ne promet plus de comportement (U4).
- `CLAUDE.md`, `docs/configuration.md` (trois emplacements), la copie crates.io synchronisée et les deux documents de solution sont à jour (U5).
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` et `docs-sync` passent.
- Le corps de PR nomme explicitement que la portée effective de `[context.history]` est **mika-arch seul**, avec la raison (M4, M5), pour que la sonde P5 ne soit pas mal lue par le relecteur.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` formelle ; les critères ci-dessous transcrivent son « Correctif attendu » et son « Test négatif (obligatoire) », ainsi que le test négatif du commentaire 1, et dérivent le reste des Requirements.

1. **AC1 (corps du ticket, « Correctif attendu »)** — au démarrage, le réconciliateur écrit les sections code-owned manquantes sur les identités **déjà provisionnées**, avec un log.
2. **AC2 (commentaire 1, test négatif)** — avec `MIKA_DISABLE_AGENT_PROVISIONING=1`, une identité existante sans `[context.history]` reçoit la section au démarrage (`scope = "session"`, `max_tokens = 8000`), et le `config.toml` reste intact.
3. **AC3 (corps du ticket, « Test négatif (obligatoire) »)** — un test prouve l'**écriture au démarrage sur disque**, et non l'appartenance d'une chaîne à une constante ; il échoue sur l'arbre pré-correctif.
4. **AC4** — avec `disabled = true`, aucun agent well-known absent n'est créé.
5. **AC5** — avec `disabled = true`, `reconcile_well_known_config` n'est jamais appelée ; le `config.toml` d'un agent existant est byte-identique avant et après.
6. **AC6** — la réconciliation reste idempotente, atomique et isolée par agent : un second démarrage n'écrit rien et émet `identity_reconcile.in_sync`.
7. **AC7** — le comportement avec `disabled = false` est inchangé (tests existants du module verts sans modification de leurs assertions).
8. **AC8** — la documentation du drapeau ne décrit plus le comportement retiré, dans les cinq emplacements de U5, copie crates.io synchronisée incluse.
9. **AC9** — le contournement manuel du commentaire 2 devient inutile pour les agents well-known, et reste documenté pour les agents hors spec (mika-prime, mika-relay).

---

## Risques et voisinage

- **R1 — une édition manuelle d'identité sera écrasée au premier démarrage (D2, D4).** Probabilité inconnue (A3 non étayée), impact borné : seules les sections listées dans `CODE_OWNED_IDENTITY_SECTIONS` sont concernées, `name`, `emoji`, `[reflection]` et `[kg]` sont préservés verbatim. Atténuation : V2 avant déploiement, P2 après, et `reconciled_paths` rend chaque écrasement nommément lisible dans le journal. Repli : reposer le drapeau ne suffirait plus — il faudrait révoquer le correctif.
- **R2 — mika-arch sauté en silence si `MIKA_KG_DOCS_ROOTS` manque (M7).** C'est le seul chemin où le correctif est inerte pour l'agent cible en n'émettant qu'un WARN, indistinguable du bruit. Atténuation : V1 avant, P1 après, la sonde exigeant une ligne qui **nomme** mika-arch plutôt qu'une absence d'erreur.
- **R3 — la sonde des « 4 agents » mal lue (M4, M5).** Le ticket énumère quatre agents ; le correctif en sert un. Sans P5 écrit noir sur blanc dans le corps de PR, un relecteur conclura à un correctif à moitié livré. C'est le risque le plus probable de tous.
- **R4 — voisinage `budget_guard`.** `assert_llm_budgets_valid` s'exécute juste après le provisioning au démarrage (`server/mod.rs`, commentaire l. 753-758) en supposant que le provisioning a écrit le `config.toml` porteur du couple `(plafond, enveloppe)`. Ce correctif n'écrit aucun `config.toml` (D3) : il ne change donc rien à cette garde, ni en bien ni en mal. Noté pour que la proximité des deux blocs ne soit pas lue comme un couplage.
- **Lignée :** mika#2295 (AC2-7, la décision de borner la fenêtre arch) → mika#2327 (le code, mergé et inerte) → mika#2330 (ce plan, qui le rend atteignable). Voisin conceptuel : mika#1633, qui a introduit `reconcile_well_known_config` et donc, sans le vouloir, la raison pour laquelle l'opérateur a désarmé le provisioning entier.

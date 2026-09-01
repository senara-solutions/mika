---
issue: mika#2118
title: Une limite par conception doit se dire comme une conception, pas comme une panne - Plan
type: fix
scope_repo: mika
priority: p2-normal
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Une limite par conception doit se dire comme une conception, pas comme une panne - Plan

## Goal Capsule

**Objectif.** Sur un hôte où aucun credential Google n'a jamais existé — tout
tenant cloud, par conception — `run_gws` doit rendre une condition **distincte**
de « credentials expirés », et l'agent doit nommer la conception au lieu
d'annoncer une panne et de proposer un `gws auth login` qu'il ne peut pas
exécuter.

**Moyens.** Une sonde `gws auth status` déclenchée uniquement sur un échec
d'authentification, un classifieur **pur** sur sa sortie JSON, deux messages
distincts, et une taxonomie de codes de sortie qui cesse de confondre les deux
états dans `system_prompt.md`.

**Hiérarchie d'autorité.** ACs du ticket > ce plan > jugement de l'implémenteur.

**Conditions d'arrêt.**
- S'arrêter si le message du cas « expiré » change. AC4 est un contrôle négatif :
  sur un hôte local avec des credentials réellement présents mais refusés, la
  sortie doit rester **inchangée**, `gws auth login` compris. Deux messages
  identiques n'auraient rien réparé.
- S'arrêter si la sonde tourne sur le chemin heureux. Elle ne doit s'exécuter que
  sur un échec d'authentification, jamais après un appel réussi.
- S'arrêter si le correctif fait dépendre le diagnostic d'une variable
  d'environnement « cloud ». L'état à distinguer est *« aucun credential sur cet
  hôte »*, pas *« je tourne dans un conteneur »* — un poste local jamais
  authentifié est dans le même état et mérite la même phrase.

**Profil d'exécution.** Deux surfaces, un dépôt :
`crates/mika-agent/src/skills/builtin_handlers.rs` (sonde + classifieur +
messages) et `crates/mika-agent/templates/skills/google-workspace/`
(`system_prompt.md`, `skill.toml`). Séquentiel.

**Tail ownership.** PR sur `mika`, routée vers mika-qa.

## Product Contract

### Résumé

`gws` sort en **2** quand il n'a pas de credentials — qu'ils soient absents
depuis toujours ou expirés. Le prompt de la skill ne connaît qu'une lecture de
ce 2 : « expirés, refais `gws auth login` ». Sur un tenant cloud, où les
credentials Google sont locaux par conception, cette lecture est fausse deux
fois. On rend l'état distinguable **avant** qu'il soit raconté, et on donne au
prompt le cas qui manquait.

### Ce que j'ai mesuré, et non déduit

`gws` est installé localement (`/home/samidarko/.cargo/bin/gws`) et dans l'image
agent (`Dockerfile.agent:70-75`) — donc, sur un tenant cloud, le binaire existe
et l'échec n'est pas un `spawn` manqué. Sonde exécutée le 2026-09-01 avec un
`XDG_CONFIG_HOME`/`HOME` jetables, les deux contrôles dans le même appel :

| état | commande | résultat mesuré |
|---|---|---|
| aucun credential | `gws calendar events list --params '{"calendarId":"primary","maxResults":1}'` | **exit 2**, stdout = `{"error":{"code":401,"message":"Access denied. No credentials provided. Run \`gws auth login\` …","reason":"authError"}}` |
| aucun credential | `gws auth status` | **exit 0**, JSON : `"auth_method":"none"`, `"credential_source":"none"`, `"encrypted_credentials_exists":false`, `"plain_credentials_exists":false`, `"client_config_exists":false`, `"storage":"none"` |
| credentials présents | `gws auth status` | **exit 0**, JSON : `"auth_method":"oauth2"`, `"credential_source":"client_secret.json"`, `"encrypted_credentials_exists":true`, `"has_refresh_token":true` |

Trois conséquences directes :

1. **La prémisse du ticket tient.** L'exit 2 sur un hôte sans credentials est
   réel et reproductible ; ce n'est pas une hypothèse.
2. **`gws auth status` est le discriminateur.** Il sort **0 dans les deux
   états**, ne fait **aucun appel réseau**, et rend un JSON stable. C'est un
   signal machine, pas une heuristique sur du texte d'erreur.
3. **`gws` lui-même pousse au mauvais conseil.** Son message contient
   littéralement « Run `gws auth login` ». Notre couche doit le recouvrir sur le
   cas cloud, pas le relayer.

### Pourquoi la sonde et pas une variable d'environnement « cloud »

Le ticket propose deux formes (source / admission). La forme retenue est **la
source**, et le discriminateur est *l'état des credentials sur cet hôte*, pas
*la nature de l'hôte*. Raisons :

- `MIKA_AGENT_TIER=family` (le seul marqueur que `mika-cloud` pose,
  `mika-cloud/scripts/add-customer.sh:231`) désigne une **persona**, pas un
  déploiement. Un tenant cloud en tier `default` a le même problème et ne
  porterait pas le marqueur.
- Un poste **local** jamais authentifié est dans exactement le même état et
  mérite exactement la même phrase. Un test sur « suis-je dans un conteneur »
  le raterait.
- L'état réel est déjà lisible, gratuitement, à la source. Déduire d'un env var
  ce que la sonde mesure serait une inférence là où une mesure existe.

### Décision AC5 — `always_on` reste `true`

**Maintenu, avec message honnête.** La skill n'est pas retirée des tenants
cloud. Raisons :

1. **Ne pas admettre la skill n'enlève pas la question, elle enlève la
   réponse.** Sans la skill, l'agent à qui l'on demande « mets ça sur mon
   Drive » n'a aucune connaissance de Workspace : il improvise, sans ancrage.
   C'est strictement pire que le défaut décrit dans ce ticket, où l'agent au
   moins savait de quoi il parlait — il le disait mal.
2. **Le calendrier reste utile là où les credentials existent.** Une admission
   conditionnée à l'hôte imposerait au moment de l'admission le signal que la
   sonde fournit déjà, plus tard et mieux, au moment de l'appel.
3. **Le coût du maintien est une ligne de prompt, pas une capacité fantôme** —
   une fois la phase 3 appliquée, la skill déclare ce qu'elle ne peut pas faire.

La décision et sa raison sont écrites dans `skill.toml` (commentaire au-dessus
de `always_on`) et dans `system_prompt.md`, pas seulement ici : un lecteur qui
tombe sur `always_on = true` doit trouver le pourquoi sans ouvrir ce plan.

### Extension de périmètre — revendiquée : le prompt promet ce que le moteur refuse

Aucun AC ne demande ceci ; je le retiens et je le dis plutôt que de le faire
passer pour une conséquence des ACs.

`validate_gws_input` (`builtin_handlers.rs:3218-3300`) applique la doctrine
mika#1798 **inconditionnellement**, sur tous les hôtes :

- **Gmail : refus total** avant tout `spawn`, avec
  `{"error":"testimony_grade_forbidden", …}` (`:3252`).
- **Drive `files get|update|delete` : refus inconditionnel** (`:3281`) — le
  filtre `q` est ignoré par l'API Drive pour ces verbes.
- **Drive `files list|create`** : admis seulement si `--params` porte un `q`
  restreint aux fichiers créés par l'app (`drive_params_are_app_scoped`).

Or `system_prompt.md` contient une section **`## Gmail Operations`** entière
(7 opérations) et, sous `## Drive Operations`, `files get`, le téléchargement et
`files delete` — **toutes structurellement refusées**. Les exemples de
`files create` n'y portent pas le `q` requis, donc échouent aussi.

C'est le défaut de ce ticket, en amont d'un cran : on dit à l'agent qu'il peut
faire une chose que le moteur lui interdit, puis on le blâme de mal raconter le
refus. Corriger la taxonomie des codes de sortie (AC2) tout en laissant dans le
même fichier trente lignes d'instructions mortes serait livrer un prompt qu'on
sait faux. La phase 3.3 les marque.

**Bornes de l'extension :** on **annote et interdit**, on ne réécrit pas la
skill et on ne touche pas à `validate_gws_input`. Le comportement du moteur est
inchangé ; seul le prompt cesse de le contredire.

**Statut : extension acceptée** (mika-arch, premier passage, F3). Elle reste
hors AC et le plan ne prétend pas le contraire ; elle est retenue parce que
corriger la taxonomie du code 2 (AC2) tout en laissant, dans le **même
fichier**, trente lignes d'instructions que le moteur refuse reviendrait à
livrer un prompt qu'on sait faux — c'est-à-dire à reproduire, un cran en amont,
le défaut exact que ce ticket corrige. Si la revue de PR juge le diff prompt
trop large, la réduction convenue est la section
`## What this skill cannot do` **seule**, sans suppression des sections mortes :
elle porte l'interdiction, qui est la partie liante.

### Hors périmètre

- **Donner aux tenants cloud un accès aux credentials Google.** C'est la
  conception, pas le défaut (le ticket le dit).
- **Les autres skills à credentials locaux.** Non auditées ici. mika#2119 est le
  même défaut de classe sur la surface web d'un tenant cloud ; il a son ticket.
- **`validate_gws_input` et la doctrine mika#1798.** Aucune ligne de la garde
  n'est modifiée. Rouvrir Gmail ou Drive serait une décision de doctrine, pas un
  correctif de message.
- **La véracité de livraison (commentaire du 2026-09-01 14:09Z).** Le
  commentaire apporte une évidence n=2 sur la même racine — « dire la vérité
  opérationnelle plutôt qu'une réussite de façade » — mais son site
  d'intervention est ailleurs : le prompt cœur et la boucle d'agent, pas la
  skill google-workspace. Trois raisons de la sortir de ce ticket plutôt que de
  l'y fondre : (a) aucun des AC1-AC5 ne la couvre, et un livrable sans AC est un
  livrable sans preuve ; (b) le harnais `tests/eval/golden/` rejoue des réponses
  **scriptées** (`skill_google_workspace_calendar.rs:22-30`) — il ne peut pas
  attester une règle de comportement, donc la livrer ici reviendrait à glisser
  une affirmation non testable dans un ticket testable ; (c) la seule correction
  robuste est structurelle, et la structure à concevoir (rendre inignorable
  l'échec d'un `send_message`) est un travail à part entière. Ticket dédié
  ouvert et référencé en commentaire de clôture ; l'évidence du commentaire y est
  reportée intégralement, rien n'est perdu.

## Acceptance criteria

Transcrits depuis le corps de mika#2118, chacun avec l'unité d'implémentation
qui le satisfait et l'artefact qui le prouve.

**AC1** — Un appel `run_gws` depuis un tenant cloud sans credentials configurés
produit une condition **distincte** de celle d'un credential expiré. Test
unitaire sur les deux états.
→ *Unité :* Phase 1 (`GwsAuthState` + `classify_gws_auth_status`, fonction pure)
et Phase 2.2 (branchement de l'exit 2 sur l'état classé).
→ *Preuve :* tests 5.1 et 5.2, alimentés par les **sorties JSON verbatim
mesurées le 2026-09-01** (§ Ce que j'ai mesuré), pas par des fixtures inventées.

**AC2** — `system_prompt.md` porte le cas correspondant et interdit
explicitement de suggérer `gws auth login` dans ce cas. La taxonomie des codes de
sortie ne confond plus les deux états.
→ *Unité :* Phase 3.1 (l'entrée `2:` se scinde en deux lectures nommées) et
Phase 3.2 (l'interdiction, à la ligne des Guidelines qui portait la confusion).
→ *Preuve :* test 5.5 — le prompt bundlé est lu depuis
`bundled_skills.rs` et asserté : il contient les deux lectures, et la chaîne
`gws auth login` n'apparaît plus dans une phrase gouvernée par le cas
« jamais configuré ».

**AC3** — Le message rendu à l'utilisateur nomme la conception, pas une panne :
ni « panne », ni « en panne », ni « expiré » quand les credentials n'ont jamais
existé sur cet hôte.
→ *Unité :* Phase 2.3 (la chaîne user-facing, construite via
`ToolOutput::substrate_unavailable` pour que le détail opérateur ne parte pas
au modèle sur le tier famille).
→ *Preuve :* test 5.3 — assertions **négatives** sur la chaîne rendue
(`outage`, `expired`, `gws auth login` absents) et positive sur la formulation
de conception.

**AC4** — Contrôle négatif : sur un hôte **local** avec credentials réellement
expirés, le message d'origine est **inchangé** — il dit toujours « expirés » et
suggère toujours `gws auth login`.
→ *Unité :* Phase 2.2, branche `ConfiguredButRejected` : passage **littéral** de
la sortie actuelle de `spawn_and_collect`, aucun texte ajouté ni retiré.
→ *Preuve :* test 5.4 — comparaison à la sortie de référence du chemin actuel ;
le test échoue si la branche « expiré » diverge d'un caractère.

**AC5** — La décision sur `always_on = true` pour les tenants cloud est prise et
écrite, avec sa raison.
→ *Unité :* § Décision AC5 (maintenu) + Phase 4 (le commentaire dans
`skill.toml` et la section correspondante de `system_prompt.md`).
→ *Preuve :* revue de diff — AC documentaire, pas testable.

## Fire-Disposition

Ce plan livre des **détecteurs** : les tests de la phase 5, dont deux gardent un
contrat de préservation (5.4, identité octet-à-octet du message « expiré » ;
5.6, non-fuite du diagnostic opérateur). Par le Fire-Disposition Gate
(mika#1574), la disposition à la mise à feu se déclare contre le schéma
canonique — **(a) exception nommée**, **(b) livré désactivé**,
**(c) halte-et-remontée**.

**Le tir au déploiement est structurellement impossible, pas seulement
improbable.** Aucun de ces détecteurs ne balaie l'arbre existant : chacun
s'exerce sur une fonction ou une sortie que **cette PR introduit ou modifie**.
Il n'existe donc pas de classe « violation préexistante ailleurs dans le dépôt »
susceptible de faire échouer une PR sans rapport. Les dispositions ci-dessous
gouvernent le seul cas résiduel : un détecteur qui tire sur le code de cette PR.

- **5.1, 5.2, 5.3, 5.5 (détecteurs de comportement neuf) → (c)
  halte-et-remontée.** Ils décrivent ce que le correctif doit faire. Un tir est
  la preuve que le correctif ne le fait pas. Pas d'exception, pas de
  désactivation : on corrige le code, jamais le test.

- **5.4 (contrat de préservation, AC4) → (c) halte-et-remontée, sans
  exception possible.** Ce test assert que la branche « credentials présents
  mais refusés » rend **exactement** la sortie actuelle. Un tir signifie que le
  correctif a modifié le chemin qu'AC4 exige de ne pas toucher — c'est
  précisément l'échec que le contrôle négatif existe pour attraper. Aucune
  allowlist n'est offerte : une exception ici viderait AC4 de son sens.

- **5.6 (non-fuite du diagnostic, mika#1783) → (c) halte-et-remontée.** Un tir
  signifie que `dispatch_substrate_diagnostic` n'a pas été appelé au site
  d'émission et que le détail opérateur part au modèle sur le tier famille.
  C'est un défaut de confidentialité de tier, pas un faux positif.

**Aucun détecteur n'est livré désactivé (b), et aucun ne porte d'allowlist
(a).** Le motif est le même pour tous : leur domaine est le diff de cette PR,
donc un tir désigne toujours un défaut du correctif, jamais un héritage.

## Phases

### Phase 1 — Le classifieur (pur) et la sonde

Fichier : `crates/mika-agent/src/skills/builtin_handlers.rs`, près de
`run_gws`.

**1.1** L'état :

```rust
/// État d'authentification de `gws` sur cet hôte, tel que `gws auth status`
/// le rapporte. Distinguer les deux est tout l'objet de mika#2118 : un exit 2
/// ne dit pas *pourquoi* il n'y a pas d'authentification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GwsAuthState {
    /// Aucun credential n'a jamais été configuré sur cet hôte. C'est l'état
    /// **par conception** de tout tenant cloud : les credentials Google sont
    /// locaux et ne sont pas provisionnés à distance.
    NeverConfigured,
    /// Des credentials existent mais l'appel a été refusé (expiration, scope,
    /// révocation). C'est le cas que le prompt savait déjà raconter.
    ConfiguredButRejected,
    /// `gws auth status` n'a pas rendu de JSON exploitable. Traité **comme
    /// `ConfiguredButRejected`** : en cas de doute, on garde le message
    /// historique plutôt que d'annoncer une limite de conception qu'on n'a pas
    /// vérifiée.
    Unknown,
}
```

**1.2** Le classifieur, **pur** — c'est ce qui rend AC1 testable sans processus :

```rust
/// Classe la sortie JSON de `gws auth status`.
///
/// `NeverConfigured` exige la conjonction : aucune source de credential ET
/// aucun des deux fichiers de credentials présent. Un seul champ pourrait
/// changer de nom dans une version future de `gws` ; la conjonction fait que
/// le doute retombe sur `Unknown`, donc sur le message historique.
fn classify_gws_auth_status(stdout: &str) -> GwsAuthState
```

Règle : `credential_source == "none"` **et** `encrypted_credentials_exists ==
false` **et** `plain_credentials_exists == false` → `NeverConfigured`. JSON
absent, illisible, ou champs manquants → `Unknown`. Tout le reste →
`ConfiguredButRejected`.

`gws auth status` peut préfixer son JSON d'une ligne de courtoisie (`Using
keyring backend: keyring`, observé sur l'hôte configuré) : le parseur cherche la
première accolade ouvrante et parse à partir de là, il ne suppose pas que la
sortie commence par `{`.

**1.3** La sonde, effet de bord isolé :

```rust
/// Interroge `gws auth status` (exit 0 dans les deux états, aucun appel
/// réseau — mesuré le 2026-09-01). Appelée **uniquement** après un exit 2.
async fn probe_gws_auth_state() -> GwsAuthState
```

Un échec de spawn, un timeout, ou un exit non nul de la sonde rendent
`Unknown`. La sonde applique `scrub_mika_env_vars` comme l'appel principal.

**Note de garde — ce n'est pas un contournement.** `GWS_ALLOWED_SUBCOMMANDS`
interdit `auth` dans le **tableau de commande fourni par le modèle** : c'est une
garde sur une entrée non fiable. La sonde est une commande fixe, construite par
le moteur, sans aucun fragment d'entrée du modèle. Les deux coexistent sans se
contredire ; l'implémenteur ne doit pas « harmoniser » l'une avec l'autre.

### Phase 2 — Brancher l'exit 2

**2.1** `run_gws` cesse d'ignorer son contexte : la signature passe de
`_ctx: &ToolContext<'_>` à `ctx: &ToolContext<'_>` (nécessaire pour 2.3).

**2.2** Après `spawn_and_collect`, si et seulement si la sortie est une erreur
d'exit code **2**, appeler la sonde et brancher :
- `NeverConfigured` → le message de conception (2.3) ;
- `ConfiguredButRejected` | `Unknown` → **retourner la sortie telle quelle**.
  C'est AC4 : aucune transformation, aucun ajout, aucun retrait.

Le chemin heureux ne paie rien : pas d'exit 2, pas de sonde.

**2.3** Le message de conception, via `ToolOutput::substrate_unavailable`
(mika#1783) parce qu'il a exactement la bonne forme — un contenu neutre pour le
modèle, un diagnostic opérateur qui ne part pas au modèle sur le tier famille :

- *user-facing* : « Google Workspace is not available on this host: no Google
  credentials have ever been configured here. On cloud deployments this is by
  design — Google credentials are local-only and are not provisioned remotely.
  This is not an outage and nothing has expired. Do NOT suggest `gws auth
  login`; it cannot be run here. Tell the user Workspace access is not
  available on this deployment. »
- *diagnostic opérateur* : nomme `gws auth status`, `credential_source: none`,
  et le chemin du magasin de credentials.

Le site d'émission doit appeler
`crate::tools::dispatch_substrate_diagnostic(&mut out, "run_gws", ctx).await`
avant de rendre — comme `web_search` (`:229`) et `fetch_url` (`:441`). **Sans
cet appel, le diagnostic fuit vers le modèle sur le tier famille** : c'est la
seule ligne dont l'oubli casse silencieusement mika#1783.

### Phase 3 — `system_prompt.md`

**3.1** Scinder l'entrée `2:` de la taxonomie en deux lectures nommées, avec
l'instruction de ne pas trancher soi-même : la distinction est rendue par le
tool result, pas devinée depuis le code.

**3.2** Remplacer la ligne 74 des Guidelines. Elle porte aujourd'hui la
confusion en une phrase ; la nouvelle porte les deux cas et **interdit
explicitement** de proposer `gws auth login` quand le tool result annonce
l'absence de credentials par conception, ainsi que d'employer le vocabulaire de
la panne ou de l'expiration dans ce cas.

**3.3** (extension revendiquée, § plus haut) Marquer ce que le moteur refuse :
supprimer ou barrer les opérations mortes de `## Gmail Operations`, retirer
`files get` / téléchargement / `files delete` de `## Drive Operations`, corriger
les exemples `files create` pour porter le `q` requis, et ajouter une section
courte **`## What this skill cannot do`** citant mika#1798 et la forme du refus
(`error: "testimony_grade_forbidden"`), pour que l'agent reconnaisse ce refus
comme une doctrine et non comme une panne.

### Phase 4 — Écrire la décision AC5

**4.1** `skill.toml` : commentaire au-dessus de `always_on = true` portant la
décision et son motif en deux lignes, avec la référence mika#2118.

**4.2** `system_prompt.md` : la section de 3.3 énonce que la skill est chargée
partout **par choix**, et que sur un hôte sans credentials la bonne réponse est
la limite de conception — pas une tentative suivie d'une interprétation.

### Phase 5 — Les preuves

Tests unitaires dans le module `tests` de `builtin_handlers.rs`.

**5.1 `classify_never_configured` (AC1).** La sortie JSON **verbatim** mesurée
sur l'hôte sans credentials → `NeverConfigured`.

**5.2 `classify_configured_but_rejected` (AC1).** La sortie JSON **verbatim**
mesurée sur l'hôte configuré (`auth_method: oauth2`, `has_refresh_token: true`)
→ `ConfiguredButRejected`. Plus `classify_unknown_on_garbage` : chaîne vide,
JSON tronqué, JSON sans les champs → `Unknown`.

**5.3 `never_configured_message_names_design_not_outage` (AC3).** Assertions
négatives sur la chaîne user-facing : elle ne contient ni `outage`, ni `expired`,
ni `gws auth login` ; assertion positive : elle contient `by design` et
`local-only`.

**5.4 `expired_path_message_is_byte_identical` (AC4).** Contrôle négatif : pour
`ConfiguredButRejected` **et** pour `Unknown`, la sortie rendue est identique à
celle du chemin actuel, `gws auth login` inclus. C'est le test qui échoue si
quelqu'un « uniformise » les deux messages.

**5.5 `bundled_prompt_carries_both_readings` (AC2).** Le prompt bundlé est lu
via `bundled_skills.rs` et asserté : les deux lectures du code 2 sont présentes,
et l'interdiction explicite l'est aussi.

**5.6 `substrate_diagnostic_is_dispatched` (mika#1783).** Sur tier famille, la
sortie rendue par `run_gws` a `substrate_diagnostic == None` après émission —
même forme que
`builtin_handlers.rs:3906` pour `web_search`. Sans ce test, l'oubli de la ligne
de dispatch passe la revue.

### Preuve de non-vacuité

Le correctif n'est pas vide si, et seulement si, la suite **échoue sur `main`**.
Vérification obligatoire avant la PR : 5.1, 5.3 et 5.5 doivent échouer sur
`origin/main` (le classifieur n'existe pas, le prompt n'a qu'une lecture du code
2), et 5.4 doit **passer** sur `main` comme après le correctif — c'est ce qui
prouve que le contrôle négatif contrôle quelque chose et n'a pas été écrit pour
être vert.

## Commandes de vérification

```bash
cargo test -p mika-agent gws
cargo test -p mika-agent --lib skills::builtin_handlers
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## Risques

| risque | mitigation |
|---|---|
| `gws auth status` change de schéma dans une version future | La conjonction de trois champs fait retomber sur `Unknown`, donc sur le message historique — jamais sur une fausse annonce de conception |
| La sonde ajoute une latence sur un échec | Elle ne tourne que sur exit 2, ne fait aucun appel réseau (mesuré), et `timeout_secs = 45` de la skill couvre les deux appels |
| L'oubli de `dispatch_substrate_diagnostic` fait fuiter le diagnostic opérateur | Test 5.6, calqué sur le test existant de `web_search` |
| Un exit code autre que 2 pour un cas d'auth futur échapperait au branchement | Documenté : le branchement suit la taxonomie que le ticket cite. Un autre code rendrait le message historique, jamais un faux message de conception |
| Le marquage du prompt (3.3) déborde en réécriture de la skill | Borne explicite : annoter et interdire, ne pas toucher `validate_gws_input` |

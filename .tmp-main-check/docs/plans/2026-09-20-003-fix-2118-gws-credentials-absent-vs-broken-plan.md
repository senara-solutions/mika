---
issue: mika#2118
title: Des identifiants jamais configurés ne sont pas des identifiants cassés - Plan
type: fix
scope_repo: mika
priority: p2-normal
date: 2026-09-20
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Des identifiants jamais configurés ne sont pas des identifiants cassés - Plan

## Goal Capsule

**Objectif.** Sur un hôte où aucun identifiant Google n'a jamais existé — tout
tenant cloud, **par conception** — `run_gws` doit rendre une condition
**distincte** de « identifiants expirés ou invalides », et le message servi doit
nommer la conception au lieu d'annoncer une panne.

**Moyens.** Un troisième axe de décision — *l'état des identifiants sur cet
hôte* — croisé avec la matrice `(Deployment, AgentTier)` que mika#2024 a déjà
posée. Une sonde `gws auth status` déclenchée **uniquement** sur un échec
d'authentification, un classifieur **pur** sur sa sortie JSON, deux familles de
messages, et une taxonomie de codes de sortie qui cesse de confondre les deux
états dans `system_prompt.md`.

**Hiérarchie d'autorité.** ACs du ticket > décision opérateur du 20/09 (voie Y +
exigence Prime) > ce plan > jugement de l'implémenteur.

**Conditions d'arrêt.**
- S'arrêter si le message du cas « identifiants présents mais refusés » change.
  AC4 est un contrôle négatif : la sortie doit rester **inchangée**. Deux
  messages identiques n'auraient rien réparé.
- S'arrêter si la sonde tourne sur le chemin heureux. Elle ne doit s'exécuter
  que sur un échec d'authentification, jamais après un appel réussi.
- S'arrêter si le correctif fait dépendre le diagnostic **du seul** marqueur de
  déploiement. L'état à distinguer est *« aucun identifiant sur cet hôte »* ;
  un poste local jamais authentifié est dans le même état et mérite la même
  phrase.
- S'arrêter si le crossing `(Deployment, AgentTier)` gagne un bras `_ =>`.

**Profil d'exécution.** Deux surfaces, un dépôt :
`crates/mika-agent/src/skills/builtin_handlers.rs` (sonde, classifieur,
messages) et `crates/mika-agent/templates/skills/google-workspace/`
(`system_prompt.md`, `skill.toml`). Séquentiel.

**Tail ownership.** PR sur `mika`, routée vers mika-qa.

---

## 1. La rectification que la lecture du code impose au ticket

Le corps de mika#2118 date du 2026-08-31 et cite `system_prompt.md` lignes
62-66 et 74 dans leur forme d'alors. **Ces lignes ont changé depuis**, et le
plan précédent (`docs/plans/2026-09-01-003-fix-2118-gws-cloud-design-limit-plan.md`,
sur la branche abandonnée par la décision opérateur du 20/09) les décrit dans
un état qui n'existe plus. Trois faits mesurés sur `main` @ `6b90de5d` :

**(M1) `run_gws` n'ignore plus son contexte.** Sa signature est
`async fn run_gws(input: &serde_json::Value, ctx: &ToolContext<'_>)`
(`builtin_handlers.rs:3692`). La phase 2.1 du plan précédent — « faire passer
`_ctx` à `ctx` » — est **déjà faite**.

**(M2) L'exit 2 porte déjà une remédiation conditionnée.** mika#2024
(`68c736de`) a livré `gws_auth_remediation(deployment, tier)`
(`builtin_handlers.rs:3632`), un `match` exhaustif à six bras sur
`(Deployment, AgentTier)` **sans bras `_ =>`**, annexé au contenu par
`is_gws_auth_error` (`:3711-3728`). Il répond à la question *« qui peut agir,
depuis où »*.

**(M3) La confusion que ce ticket nomme est intacte.** Les six bras de M2
décrivent tous, sans exception, un lien **qui a cessé de fonctionner** :

| bras | ce qui est affirmé |
|---|---|
| `(Local, Default)` | « re-authentication is available to them directly » |
| `(Cloud, Default)` | « the Google account has to be **reconnected** » |
| `(Unknown, Default)` | « the Google account has to be **reconnected** » |
| `(*, Family \| Champion)` | « The link … **has stopped working** and has to be set up again » |

Et `system_prompt.md` l'écrit deux fois au présent de l'indicatif :

> l. 63 — « 2: Authentication error — the stored Google credentials **are
> expired or invalid**. »
> l. 74 — « tell the user their Google credentials **are expired or invalid** »

Sur un tenant cloud, **aucun identifiant n'a jamais été stocké**. « The link has
stopped working », « reconnected », « are expired or invalid » sont donc faux
tous les quatre : ils fabriquent une panne à partir d'une limite par conception.
C'est très exactement le symptôme du 2026-08-31 (« Google Workspace est en panne
d'auth »), et mika#2024 ne l'a pas fermé — il n'en traitait pas l'axe.

**Conséquence sur le périmètre.** Les deux tickets sont orthogonaux et
composent ; ce plan **n'annule aucune ligne de mika#2024**. Même forme que le
dédoublement d'axes de mika#2023 (`ToolsProfile` × `PersonaProfile`) et de
mika#2290 (persona × deployment) : une question qu'on croyait unique en était
deux.

| axe | question | livré par |
|---|---|---|
| déploiement × persona | *qui peut agir, et dans quel registre ?* | **mika#2024 (livré)** |
| **état des identifiants** | *y a-t-il quelque chose de cassé, ou rien n'a-t-il jamais existé ?* | **ce ticket** |

**(M4) `always_on` est inchangé et les déclencheurs sont larges.**
`skill.toml` porte toujours `always_on = true` et 22 mots-clés dont `email`,
`calendar`, `drive`, `document`, `meeting`, `agenda`, `triage`. AC5 reste ouvert.

---

## 2. L'exigence Prime, et la mauvaise abstraction qu'elle n'autorise pas

La décision opérateur du 20/09 demande de corriger « la *sémantique du message*,
pas seulement le cas Google », et rattache le défaut à l'incident du 19/09 (« un
tenant reçoit "clé manquante" pour une configuration absente côté passerelle »).

**Ce site existe dans l'arbre, et il est resté faux.** `map_substrate_error`
(`builtin_handlers.rs:358-362`) rend, pour un `404
search_upstream_not_configured` :

> « Search substrate is not configured on the gateway. Ask the operator to set
> **MIKA_BRAVE_API_KEY** on mika-gateway. »

Or `crates/mika-gateway/CLAUDE.md` § *Search Substrate* établit que l'absence
est celle du **sélecteur** (`MIKA_SEARCH_UPSTREAM`), et que « le geste
réparateur est l'inverse de l'évident : ajouter le **sélecteur**, pas une autre
clé ». Le message côté agent n'a pas suivi mika#2407 : il nomme une clé
manquante là où c'est une configuration absente. Même classe, à la lettre.

**Ce que la généralisation NE sera PAS : un type partagé.** La tentation est
d'extraire un `CapabilityState { AbsentByDesign, Broken }` que les deux sites
liraient. **Refusé**, et la raison est écrite plutôt que devinée : les deux
sujets ne sont pas le même. Ici, l'objet est *un identifiant sur cet hôte* ; là,
*une configuration de substrat sur un autre processus*. Construire une
abstraction sur deux points dont les sujets diffèrent est très exactement ce que
mika#2237 a dû nommer — « dessiner une abstraction sur un point est ce qui
produit la mauvaise abstraction » — et le prix se paierait au troisième site, qui
devrait tordre son vocabulaire pour entrer dans un type conçu pour Google.

**Ce que la généralisation SERA : une règle, appliquée deux fois.** La règle est
nommée, écrite au site de décision et tenue par un test **par site** :

> *Un état « n'a jamais été configuré » ne se raconte jamais avec le vocabulaire
> d'un état « a cessé de fonctionner ». Le premier nomme la conception ; le
> second nomme une réparation.*

Elle est appliquée (a) à `run_gws`, cœur des AC1-AC5, et (b) à
`map_substrate_error`, en **extension revendiquée bornée** (§ 3.5). Ce plan ne
prétend pas que (b) découle d'un AC : il ne le fait pas.

---

## 3. Ce qui est livré

### 3.1 — L'état, et son classifieur pur

Fichier : `crates/mika-agent/src/skills/builtin_handlers.rs`, près de `run_gws`.

```rust
/// État des identifiants `gws` sur CET hôte, tel que `gws auth status` le
/// rapporte. Les distinguer est tout l'objet de mika#2118 : un exit 2 ne dit
/// pas *pourquoi* il n'y a pas d'authentification.
///
/// Axe orthogonal à `(Deployment, AgentTier)` (mika#2024), qui répond « qui
/// peut agir, depuis où » et jamais « qu'est-ce qui manque ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GwsCredentialState {
    /// Aucun identifiant n'a jamais été configuré ici. État **par conception**
    /// de tout tenant cloud : les identifiants Google sont locaux et ne sont
    /// pas provisionnés à distance. Également l'état d'un poste local jamais
    /// authentifié — d'où un discriminateur sur les identifiants et non sur la
    /// nature de l'hôte.
    NeverConfigured,
    /// Des identifiants existent et l'appel a été refusé (expiration, scope,
    /// révocation). C'est le seul cas que le prompt savait raconter.
    ConfiguredButRejected,
    /// La sonde n'a rien rendu d'exploitable. Traité **comme
    /// `ConfiguredButRejected`** : dans le doute on garde le message
    /// historique plutôt que d'annoncer une limite de conception non vérifiée.
    Unknown,
}
```

Le classifieur est **pur** — c'est ce qui rend AC1 testable sans processus :

```rust
fn classify_gws_auth_status(stdout: &str) -> GwsCredentialState
```

**Règle, par conjonction :** `credential_source == "none"` **ET**
`encrypted_credentials_exists == false` **ET** `plain_credentials_exists ==
false` → `NeverConfigured`. JSON absent, illisible, ou l'un des trois champs
manquant → `Unknown`. Tout le reste → `ConfiguredButRejected`.

La conjonction est le fail-safe : un seul champ renommé dans une version future
de `gws` fait retomber sur `Unknown`, donc sur le message historique — jamais
sur une fausse annonce de conception.

**Tolérance de préfixe.** `gws auth status` peut préfixer son JSON d'une ligne
de courtoisie (`Using keyring backend: keyring`, observé sur l'hôte configuré).
Le parseur utilise `serde_json::Deserializer::from_str(&s[first_brace..])
.into_iter::<serde_json::Value>().next()`, qui s'arrête au premier objet
complet et tolère donc **aussi** de la prose après. Ne pas supposer que la
sortie commence par `{`, ni qu'elle s'y termine.

### 3.2 — La sonde, effet de bord isolé et **borné**

```rust
/// Interroge `gws auth status`. Appelée **uniquement** après un exit 2.
async fn probe_gws_auth_state() -> GwsCredentialState
```

- Exit 0 dans les deux états, aucun appel réseau (mesuré 2026-09-01, § 5.6).
- Applique `super::executor::scrub_mika_env_vars(&mut cmd)` comme l'appel
  principal.
- **Bornée par `tokio::time::timeout(GWS_AUTH_PROBE_TIMEOUT)`**, `10s`. Le plan
  précédent s'en remettait au `timeout_secs = 45` du manifeste, qui couvre les
  deux appels *ensemble* : une sonde qui pend mangerait le budget de l'appel
  utile et transformerait une erreur d'auth lisible en timeout opaque.
- Échec de spawn, timeout, ou exit non nul → `Unknown`.

**Note de garde — ce n'est pas un contournement.** `GWS_ALLOWED_SUBCOMMANDS`
(`:3389`) interdit `auth` dans le **tableau de commande fourni par le modèle** :
c'est une garde sur une entrée non fiable. La sonde est une commande fixe,
construite par le moteur, sans aucun fragment d'entrée du modèle. Les deux
coexistent sans se contredire ; **l'implémenteur ne doit pas « harmoniser »
l'une avec l'autre**.

### 3.3 — Le branchement, et l'asymétrie qui EST les deux AC

Dans `run_gws`, le bloc `if is_gws_auth_error(&output.content)` existant devient :

```rust
if is_gws_auth_error(&output.content) {
    match probe_gws_auth_state().await {
        // AC4 — chemin historique, INCHANGÉ octet pour octet.
        GwsCredentialState::ConfiguredButRejected | GwsCredentialState::Unknown => {
            output.content.push_str("\n\n");
            output.content.push_str(gws_auth_remediation(ctx.deployment, ctx.tier));
            /* le tracing::info! mika#2024 existant, inchangé */
        }
        // AC1 + AC3 — condition distincte, contenu REMPLACÉ.
        GwsCredentialState::NeverConfigured => {
            output = ToolOutput::substrate_unavailable(
                gws_credentials_absent_message(ctx.deployment, ctx.tier),
                /* diagnostic opérateur, § 3.4 */,
            );
            /* tracing::info! event = "gws_credentials_absent", … */
        }
    }
}
```

**L'asymétrie annexe/remplacement est délibérée et porte les deux AC.**

- *Rejected* → **annexe**. AC4 exige que ce chemin soit inchangé ; toute
  transformation le violerait.
- *NeverConfigured* → **remplacement** via `ToolOutput::substrate_unavailable`
  (`tools/mod.rs:394`). Sans cela AC3 est **inatteignable** : le contenu de
  `spawn_and_collect` est `"Exit code: 2\n{stderr}{stdout}"`, et le stdout de
  `gws` contient littéralement ``Run `gws auth login` `` (mesuré, § 5.6).
  Annexer laisserait la prescription de `gws` dans ce que le modèle lit, et
  compter sur le prompt pour la contredire serait du prompt-enforcement sur du
  substrat de boucle — refusé par
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.
  Remplacer **recouvre** la sortie de `gws` au lieu de la relayer ; le détail
  brut part dans le diagnostic opérateur.

Le chemin heureux ne paie rien : pas d'exit 2, pas de sonde.

### 3.4 — Les messages, et le crossing exhaustif

**Deux fonctions, jamais dix-huit bras.**

- `gws_auth_remediation(deployment, tier)` — **NON TOUCHÉE**. Six bras,
  inchangés. AC4 devient vrai par construction plutôt que par vigilance.
- `gws_credentials_absent_message(deployment, tier)` — **nouvelle**. Six bras,
  `match` exhaustif sur `(Deployment, AgentTier)`, **aucun bras `_ =>`**, sur
  le modèle de sa sœur et de `dispatch_substrate_diagnostic` : un futur tier ou
  un futur état de déploiement doit décider, pas hériter d'une décision que
  personne n'a prise pour lui.

**Le registre suit l'axe persona** (mika#2290, mika#2292). `FAMILY_SOUL` interdit
« toute mention … de l'infrastructure sous-jacente — jamais, même si on te le
demande » ; « local-only », « provisioned », « deployment » sont de cette
famille. Deux formulations pour un fait.

*Registre opérateur* — `(Cloud | Unknown, Default)`, formulation vérifiée contre
les assertions du test 5.3 (contient `by design` et `local-only` ; ne contient ni
`outage`, ni `expired`, ni `gws auth login`) :

> Google Workspace access is not available on this host: no Google credentials
> have ever been configured here. On cloud deployments this is **by design** —
> Google credentials are **local-only** and are not provisioned remotely.
> Nothing here has stopped working and no credential needs renewing. Tell the
> user this capability is not available on this deployment. Propose no sign-in
> step; none can be run here.

> **Note (héritée du commentaire du 2026-09-02, à ne pas défaire).** Cette
> formulation évite **délibérément** les sous-chaînes `outage`, `expired` et
> `gws auth login`. Ne pas restaurer une forme négative du type « this is not an
> *outage* and nothing has *expired*, do NOT suggest `gws auth login` » : elle
> porte le même sens et **fait échouer le test 5.3**, dont les assertions sont
> sur les sous-chaînes. L'autorité est AC > plan > implémenteur, et le test
> *est* la preuve de l'AC3.

*Registre opérateur* — `(Local, Default)` : le seul croisement où l'absence est
réparable par l'utilisateur lui-même, et le seul qui peut nommer un geste de
terminal (contrainte mika#2024, conservée). La formulation y prescrit la
**configuration initiale**, jamais une *re*-connexion.

*Registre famille/champion* — les trois déploiements, sans un nom technique :

> This has never been set up for this person, and it is not something you or
> they can set up from here. Tell them simply that you cannot reach their
> calendar and files, that nothing is broken, and that the person who set you
> up is the one who would have to arrange it. Name no technical step.

**Le diagnostic opérateur** (second argument de `substrate_unavailable`) nomme
`gws auth status`, `credential_source: none`, et le chemin du magasin
d'identifiants. `dispatch_substrate_diagnostic` le route ensuite par tier :
annexé au contenu sur tier opérateur, écrit dans `audit_events` sur
famille/champion.

**Le site d'émission doit appeler**
`crate::tools::dispatch_substrate_diagnostic(&mut out, "run_gws", ctx).await`
avant de rendre — comme `web_search` et `fetch_url`. **Sans cette ligne le
diagnostic fuit vers le modèle sur le tier famille** : c'est la seule ligne dont
l'oubli casse silencieusement mika#1783. Test 5.7.

### 3.5 — `system_prompt.md` et `skill.toml`

**(a) Scinder l'entrée `2:` de la taxonomie** en deux lectures nommées, avec
l'instruction de ne pas trancher soi-même : la distinction est **rendue par le
tool result**, jamais devinée depuis le code de sortie.

**(b) Remplacer la ligne 74 des Guidelines.** Elle affirme aujourd'hui
« credentials are expired or invalid » inconditionnellement. La nouvelle porte
les deux cas et **interdit explicitement**, quand le tool result annonce
l'absence par conception : de proposer une commande de connexion, et d'employer
le vocabulaire de la panne ou de l'expiration.

**(c) AC5 — `always_on = true` est MAINTENU**, avec sa raison écrite dans
`skill.toml` (commentaire au-dessus du champ) et dans le prompt, pas seulement
dans ce plan : un lecteur qui tombe sur `always_on = true` doit trouver le
pourquoi sans ouvrir `docs/plans/`.

> **Décision.** Ne pas admettre la skill n'enlève pas la question, elle enlève
> la réponse. Sans elle, l'agent à qui l'on demande « mets ça sur mon Drive »
> improvise sans aucun ancrage — strictement pire que le défaut décrit ici, où
> l'agent savait de quoi il parlait et le disait mal. Le coût du maintien est
> une ligne de prompt ; après (d), la skill déclare ce qu'elle ne peut pas
> faire.

**(d) Extension revendiquée A — le prompt promet ce que le moteur refuse.**
Aucun AC ne le demande ; je le retiens et je le dis plutôt que de le faire
passer pour une conséquence des AC. `validate_gws_input` (`:3493-3572`) applique
mika#1798 **inconditionnellement, sur tous les hôtes** : Gmail refusé avant tout
spawn (`:3520-3528`), Drive `files get|update|delete` refusé sans condition
(`:3548-3558`), `list|create` admis seulement avec un `q` app-scopé. Or
`system_prompt.md` porte une section **`## Gmail Operations`** entière (7
opérations, l. 23-31) et, sous Drive, `files get` (l. 46), le téléchargement
(l. 47) et `files delete` (l. 49) — **structurellement morts**. Corriger la
taxonomie du code 2 tout en laissant, dans le **même fichier**, trente lignes
d'instructions que le moteur refuse, reviendrait à livrer un prompt qu'on sait
faux : c'est reproduire un cran en amont le défaut exact que ce ticket corrige.

*Bornes :* on **annote et interdit**. `validate_gws_input` n'est pas touché, le
comportement du moteur est inchangé, seul le prompt cesse de le contredire. On
ajoute une section courte **`## What this skill cannot do`** citant mika#1798 et
la forme du refus (`error: "testimony_grade_forbidden"`), pour que l'agent
reconnaisse ce refus comme une doctrine et non comme une panne.
*Repli si la revue juge le diff trop large :* garder la section
`## What this skill cannot do` **seule**, sans retirer les sections mortes —
elle porte l'interdiction, qui est la partie liante.

**(e) Extension revendiquée B — le message 404 de `web_search` (§ 2).**
Corriger `map_substrate_error` (`:358-362`) pour nommer la configuration absente
(`MIKA_SEARCH_UPSTREAM`) au lieu d'une clé manquante, en appliquant la règle
de § 2. Une chaîne, un test. Retenue parce que la décision opérateur la nomme
comme même classe et que c'est le seul autre site **mesuré** dans l'arbre.
*Repli si la revue juge le périmètre trop large :* retirer le diff et ouvrir un
ticket de suivi citant `builtin_handlers.rs:358` et
`crates/mika-gateway/CLAUDE.md` § *Search Substrate*. **Ne pas** en profiter
pour toucher au gateway : mika#2407 y est livré et sa moitié gateway est
correcte.

### 3.6 — Disposition de l'ancien plan

`docs/plans/2026-09-01-003-fix-2118-gws-cloud-design-limit-plan.md` est
**supprimé dans le même commit**. Il décrit un état du code qui n'existe plus
(§ 1, M1/M2) et prescrit une phase 2.1 déjà faite. Deux plans pour un même
ticket sont aussi un piège opérationnel : `_find_issue_plan` de `dispatch-lib`
résout `*-2118-*-plan.md` et un `sort -r | head -1` départagerait par nom, ce
qui est un ordre et non une décision. Cette suppression exécute la décision
opérateur du 20/09 (« l'invalidation de l'ancien callout GROOMED revient à
MPC ») sur sa moitié versionnée.

---

## 4. Ce qui n'est PAS livré, et pourquoi

- **Donner aux tenants cloud un accès aux identifiants Google.** C'est la
  conception, pas le défaut — le ticket le dit.
- **Un type `CapabilityState` partagé.** Refusé sur mesure, § 2.
- **Les autres skills à identifiants locaux.** Non auditées. Hors périmètre
  explicite du corps du ticket.
- **`validate_gws_input` et la doctrine mika#1798.** Aucune ligne de la garde
  n'est modifiée. Rouvrir Gmail ou Drive serait une décision de doctrine, pas un
  correctif de message.
- **Les six bras de `gws_auth_remediation`.** Non touchés : c'est AC4.
- **La moitié gateway de mika#2407.** Livrée et correcte.
- **La véracité de livraison** (évidence n=2 du commentaire du 2026-09-01
  14:09Z). **Sortie de ce ticket vers mika#2136**, où elle est reportée
  intégralement avec ses propres AC. Rien n'est perdu.
- **Une garde EndTurn sur le vocabulaire de panne.** Refusée : le lexique
  (« panne », « expiré », « ne marche plus ») est du registre famille ordinaire,
  donc le taux de faux positifs serait maximal précisément sur le tier qu'elle
  prétend protéger — raisonnement et précédent : mika#2292. La moitié
  structurelle disponible est prise : le modèle **ne reçoit plus** l'affirmation
  fausse, donc il n'a rien à relayer.

---

## 5. Verification contract

Tests unitaires dans le module `tests` de `builtin_handlers.rs`, sauf mention.

**5.1 `mika2118_classify_never_configured` (AC1).** La sortie JSON **verbatim**
mesurée sur l'hôte sans identifiant → `NeverConfigured`.

**5.2 `mika2118_classify_configured_but_rejected` (AC1).** La sortie JSON
**verbatim** mesurée sur l'hôte configuré (`auth_method: oauth2`,
`has_refresh_token: true`) → `ConfiguredButRejected`. Plus
`mika2118_classify_unknown_on_garbage` : chaîne vide, JSON tronqué, JSON
auquel manque **chacun** des trois champs à son tour → `Unknown`. Trois cas
séparés : une conjonction de trois termes n'est pas prouvée en les neutralisant
tous à la fois.

**5.3 `mika2118_absent_message_names_design_not_outage` (AC3).** Pour **les six**
croisements de `gws_credentials_absent_message` : assertions **négatives** — la
chaîne ne contient ni `outage`, ni `expired`, ni `gws auth login`. Puis, sur le
seul registre opérateur `(Cloud | Unknown, Default)` : assertions **positives**
— elle contient `by design` et `local-only`.
*Les positives sont scindées par registre à dessein* : exiger `local-only` sur
le bras famille contredirait `FAMILY_SOUL` et rendrait le test et la doctrine
mutuellement inapplicables.

**5.4 `mika2118_family_absent_message_carries_no_infrastructure_jargon` (AC3).**
Sur les trois bras `Family | Champion` : absence de `deployment`, `cloud`,
`credential`, `provision`, `local-only`, `CLI`, `terminal`. Calqué sur
`mika2024_family_and_champion_remediations_carry_no_infrastructure_jargon`.

**5.5 `mika2118_rejected_path_is_byte_identical` (AC4).** Contrôle négatif :
pour `ConfiguredButRejected` **et** pour `Unknown`, sur les six croisements, la
sortie rendue est identique à celle du chemin actuel — annexe comprise. C'est le
test qui échoue si quelqu'un « uniformise » les deux familles de message.

**5.6 `mika2118_bundled_prompt_carries_both_readings` (AC2).** Le prompt bundlé
est lu via `bundled_skills.rs` et asserté : les deux lectures du code 2 sont
présentes, l'interdiction explicite l'est aussi, et la chaîne
« are expired or invalid » n'apparaît plus dans une phrase gouvernant
**tous** les exit 2.

**5.7 `mika2118_substrate_diagnostic_is_dispatched` (mika#1783).** Sur tier
famille, la sortie rendue par `run_gws` a `substrate_diagnostic == None` après
émission — même forme que le test existant de `web_search`. Sans ce test,
l'oubli de la ligne de dispatch passe la revue.

**5.8 `mika2118_the_absent_message_match_has_no_wildcard_arm`.** Scan de source
sur le corps de `gws_credentials_absent_message`, calqué sur
`mika2024_the_remediation_match_has_no_wildcard_arm`. Un test comportemental ne
peut pas voir cette classe : un `_ =>` ne rend aucune assertion fausse le jour
où il est écrit, il fait hériter en silence le jour où un tier est ajouté.

**5.9 `mika2118_probe_runs_only_on_auth_error`.** Un appel réussi et un appel
échouant sur un code ≠ 2 ne déclenchent aucune sonde.

**5.10 (extension B) `mika2118_substrate_404_names_the_selector_not_a_key`.**
Le message du `(404, "search_upstream_not_configured")` nomme
`MIKA_SEARCH_UPSTREAM` et ne dit pas qu'une clé manque. Tombe avec l'extension
si elle est repliée.

### Preuve de non-vacuité

Le correctif n'est pas vide si, et seulement si, la suite **échoue sur `main`**.
Vérification obligatoire avant la PR : **5.1, 5.3 et 5.6 doivent échouer** sur
`origin/main` (le classifieur n'existe pas, le prompt n'a qu'une lecture du code
2), et **5.5 doit passer** sur `main` comme après le correctif — c'est ce qui
prouve que le contrôle négatif contrôle quelque chose et n'a pas été écrit pour
être vert.

### Re-mesure obligatoire de la sonde

Les sorties verbatim de 5.1/5.2 datent du **2026-09-01**. Avant de les figer en
fixtures, **rejouer la mesure** sur la version de `gws` installée, avec
`XDG_CONFIG_HOME`/`HOME` jetables :

```bash
XDG_CONFIG_HOME=$(mktemp -d) HOME=$(mktemp -d) gws auth status; echo "exit=$?"
gws auth status; echo "exit=$?"
```

Attendu : exit **0** dans les deux cas ; `credential_source: "none"` +
`encrypted_credentials_exists: false` dans le premier ; `auth_method: "oauth2"`
dans le second. **Halte si `gws auth status` sort non nul sur l'hôte vierge** :
le discriminateur ne tient plus, la sonde ne peut pas distinguer, et le plan
doit être repris avant toute implémentation — ne pas « réparer » en traitant un
exit non nul comme `NeverConfigured`, ce serait rendre la conception à partir
d'une panne, soit le défaut en miroir.

### Commandes de vérification

```bash
cargo test -p mika-agent gws
cargo test -p mika-agent mika2118
cargo test -p mika-agent --lib skills::builtin_handlers
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
make verify-bundled-skills
```

---

## 6. Fire-Disposition

Ce plan livre des **détecteurs** : les tests du § 5, dont trois gardent un
contrat de préservation (5.5, identité du chemin rejeté ; 5.7, non-fuite du
diagnostic opérateur ; 5.8, absence de bras `_ =>`). Par le Fire-Disposition
Gate (mika#1574), la disposition à la mise à feu se déclare contre le schéma
canonique — **(a) exception nommée**, **(b) livré désactivé**,
**(c) halte-et-remontée**.

**Le tir au déploiement est structurellement impossible, pas seulement
improbable.** Aucun de ces détecteurs ne balaie l'arbre existant : chacun
s'exerce sur une fonction, une sortie ou un fichier que **cette PR introduit ou
modifie**. Il n'existe donc pas de classe « violation préexistante ailleurs dans
le dépôt » susceptible de faire échouer une PR sans rapport. Les dispositions
ci-dessous gouvernent le seul cas résiduel : un détecteur qui tire sur le code de
cette PR.

- **5.1, 5.2, 5.3, 5.4, 5.6, 5.9, 5.10 (détecteurs de comportement neuf) → (c)
  halte-et-remontée.** Ils décrivent ce que le correctif doit faire. Un tir est
  la preuve qu'il ne le fait pas. On corrige le code, jamais le test.
- **5.5 (contrat de préservation, AC4) → (c) halte-et-remontée, sans exception
  possible.** Un tir signifie que le correctif a modifié le chemin qu'AC4 exige
  de ne pas toucher — précisément l'échec que le contrôle négatif existe pour
  attraper. Aucune allowlist n'est offerte : une exception ici viderait AC4 de
  son sens.
- **5.7 (non-fuite, mika#1783) → (c) halte-et-remontée.** Un tir signifie que
  `dispatch_substrate_diagnostic` n'a pas été appelé au site d'émission et que
  le détail opérateur part au modèle sur le tier famille. Défaut de
  confidentialité de tier, pas faux positif.
- **5.8 (scan de source) → (c) halte-et-remontée, allowlist livrée VIDE.**
  Quand il tire, on retire le bras `_ =>` ; on ne l'allowliste pas. Une
  allowlist née vide est un endroit où déposer la prochaine violation
  (mika#2323).

**Aucun détecteur n'est livré désactivé (b), et aucun ne porte d'exception
nommée (a).** Le motif est le même pour tous : leur domaine est le diff de cette
PR, donc un tir désigne toujours un défaut du correctif, jamais un héritage.

---

## 7. Surfaces opérateur et sondes post-déploiement

**Journal** (`$MIKA_SPIRIT_LOG_FILE`) :

```bash
# La branche « jamais configuré » a-t-elle été empruntée, et où ?
grep gws_credentials_absent "$MIKA_SPIRIT_LOG_FILE" | jq '{deployment, tier, agent_id}'

# La branche historique (mika#2024), inchangée
grep gws_auth_remediation_annexed "$MIKA_SPIRIT_LOG_FILE" | jq '{deployment, tier}'
```

`gws_credentials_absent` (INFO) porte `deployment`, `tier`, `agent_id` — **et
jamais un identifiant de compte Google** (contrainte R7 héritée de mika#2024 ;
le test 5.4 balaie *tous* les champs, car une fuite arrive par celui auquel
personne n'a pensé).

**SQL.** Sur tier famille, `dispatch_substrate_diagnostic` écrit déjà :
`SELECT * FROM audit_events WHERE tool_name = 'substrate_unavailable' AND target_key = 'run_gws';`

**Régimes attendus.** `gws_credentials_absent` : **non vide** sur les tenants
cloud dès qu'un utilisateur demande Drive ou Calendar — c'est la mesure directe
de la population que ce ticket sert, et elle n'existait pas.
`gws_auth_remediation_annexed` : non vide sur les postes locaux dont le token a
expiré. **Les deux doivent être disjoints sur un même tenant** : un hôte ne peut
pas à la fois n'avoir jamais eu d'identifiants et en avoir des expirés.

**Sonde, sur le symptôme fondateur, et ses trois haltes.** Rejouer sur un tenant
cloud : « peux-tu créer un Gdoc et le mettre sur mon Drive ? ». Attendu : une
réponse qui nomme la limite de conception, sans « panne », sans « expiré », sans
geste de connexion.

- **Halte 1 — la réponse dit encore « panne » ou « expiré » alors que
  `gws_credentials_absent` est non vide.** Le fait est posé et le modèle le
  contredit : **ne pas retoucher la formulation par réflexe**, vérifier d'abord
  que le prompt servi porte bien les deux lectures (§ 3.5 a/b) — un tenant servi
  par un binaire antérieur est la classe mika#2340, et c'est **le déploiement**
  qu'il faut établir avant toute conclusion sur le texte.
- **Halte 2 — `gws_credentials_absent` est vide alors que le symptôme
  persiste.** La sonde ne classe pas `NeverConfigured`. Lire le diagnostic
  opérateur (tier `default`) ou la ligne `audit_events` (famille) : si
  `credential_source` n'est pas `none`, le schéma de `gws auth status` a bougé
  et c'est le classifieur — **pas le message** — qu'il faut reprendre. Ne pas
  relâcher la conjonction du § 3.1 pour faire apparaître des lignes : elle est
  le seul rempart contre une fausse annonce de conception.
- **Halte 3 — un tenant famille reçoit du jargon d'infrastructure.** Lire
  l'`AgentTier` résolu **avant** d'accuser la formulation : un champion
  provisionné avant mika-cloud#209 (2026-08-28) porte encore l'identité
  opérateur sur disque, et aucune ligne de ce ticket ne la corrige — c'est un
  geste de re-provisionnement (`mika agents reprovision`, mika#2230).

**Ce que ce travail n'achète pas.** Aucun test déterministe ne peut établir la
réponse d'un LLM : les tests du § 5 attestent que le **fait servi au modèle** est
juste, jamais que la phrase rendue à l'utilisateur l'est. La moitié
comportementale est la sonde par rejeu ci-dessus, et **le silence ne prouve rien
si personne ne pose la question**.

---

## 8. Definition of Done

- [ ] `GwsCredentialState` + `classify_gws_auth_status` (pur) + sonde bornée.
- [ ] `run_gws` branche l'exit 2 sur l'état classé ; chemin heureux intouché.
- [ ] `gws_credentials_absent_message` : six bras, aucun `_ =>`, deux registres.
- [ ] `gws_auth_remediation` **non modifiée** (`git diff` le montre).
- [ ] `dispatch_substrate_diagnostic` appelé au nouveau site d'émission.
- [ ] `system_prompt.md` : taxonomie scindée, ligne 74 remplacée, section
      `## What this skill cannot do`, instructions mortes retirées.
- [ ] `skill.toml` : décision AC5 écrite en commentaire au-dessus de `always_on`.
- [ ] Ancien plan du 2026-09-01 supprimé dans le même commit.
- [ ] Mesure `gws auth status` **rejouée** et fixtures verbatim figées.
- [ ] Non-vacuité vérifiée : 5.1 / 5.3 / 5.6 échouent sur `origin/main`, 5.5 y
      passe.
- [ ] `cargo clippy -- -D warnings`, `cargo fmt --check`,
      `make verify-bundled-skills` verts.
- [ ] Corps de PR : nommer les deux extensions revendiquées (§ 3.5 d et e) et
      leurs replis, pour que la revue puisse trancher sans lire ce plan.

---

## Acceptance criteria

Transcrits depuis le corps de mika#2118, chacun avec l'unité d'implémentation
qui le satisfait et l'artefact qui le prouve.

**AC1** — Un appel `run_gws` depuis un tenant cloud sans identifiants configurés
produit une condition **distincte** de celle d'un identifiant expiré. Test
unitaire sur les deux états.
→ *Unité :* § 3.1 (`GwsCredentialState` + `classify_gws_auth_status`, pur) et
§ 3.3 (branchement de l'exit 2 sur l'état classé).
→ *Preuve :* tests 5.1 et 5.2, alimentés par les **sorties JSON verbatim
re-mesurées**, pas par des fixtures inventées.

**AC2** — `system_prompt.md` porte le cas correspondant et interdit
explicitement de suggérer `gws auth login` dans ce cas. La taxonomie des codes
de sortie ne confond plus les deux états.
→ *Unité :* § 3.5 (a) et (b).
→ *Preuve :* test 5.6.

**AC3** — Le message rendu à l'utilisateur nomme la conception, pas une panne :
ni « panne », ni « en panne », ni « expiré » quand les identifiants n'ont jamais
existé sur cet hôte.
→ *Unité :* § 3.4 (les deux registres) et § 3.3 (le **remplacement**, sans
lequel la prescription de `gws` reste dans ce que le modèle lit).
→ *Preuve :* tests 5.3 et 5.4 — assertions négatives sur les six croisements,
positives par registre.

**AC4** — Contrôle négatif : sur un hôte **local** avec identifiants réellement
expirés, le message d'origine est **inchangé**.
→ *Unité :* § 3.3, branche `ConfiguredButRejected | Unknown` : passage littéral,
et `gws_auth_remediation` n'est pas touchée.
→ *Preuve :* test 5.5 ; le test échoue si la branche diverge d'un caractère.

**AC5** — La décision sur `always_on = true` pour les tenants cloud est prise et
écrite, avec sa raison.
→ *Unité :* § 3.5 (c) — `skill.toml` et `system_prompt.md`, pas seulement ce
plan.
→ *Preuve :* revue de diff. AC documentaire, pas testable.

---

## 9. Risques et limites

| risque | mitigation |
|---|---|
| `gws auth status` change de schéma dans une version future | La conjonction de trois champs (§ 3.1) fait retomber sur `Unknown`, donc sur le message historique — jamais sur une fausse annonce de conception. Halte 2 du § 7 nomme la reprise. |
| La sonde ajoute une latence sur un échec | Elle ne tourne que sur exit 2, ne fait aucun appel réseau (mesuré), et est **bornée à 10 s** indépendamment du `timeout_secs = 45` du manifeste. |
| L'oubli de `dispatch_substrate_diagnostic` fait fuiter le diagnostic opérateur | Test 5.7, calqué sur le test existant de `web_search`. |
| Un futur tier ou état de déploiement hérite d'un message qui ne lui va pas | `match` exhaustif sans `_ =>`, tenu par le scan 5.8 à allowlist vide. |
| Un exit code autre que 2 pour un cas d'auth futur échapperait au branchement | Documenté : le branchement suit la taxonomie que le ticket cite. Un autre code rend le message historique, jamais un faux message de conception. |
| Le marquage du prompt (§ 3.5 d) déborde en réécriture de la skill | Borne explicite : annoter et interdire, ne pas toucher `validate_gws_input`. Repli documenté. |
| L'extension B (§ 3.5 e) élargit le périmètre au-delà des AC | Revendiquée comme telle, une chaîne + un test, repli documenté en ticket de suivi. |
| La formulation opérateur est « corrigée » vers une forme négative | La note du § 3.4 l'interdit explicitement et le test 5.3 la fait échouer. |
| `Cloud` et `Unknown` convergent aujourd'hui | Séparés quand même dans le `match` : le jour où le provisionneur émet `MIKA_DEPLOYMENT`, donner un lien de console à `Cloud` sans le donner à `Unknown` est un diff d'un bras (raisonnement hérité de mika#2024). |

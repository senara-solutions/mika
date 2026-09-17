---
issue: 2290
type: fix
---

# fix(mika#2290) — l'hébergement est un fait posé, jamais inféré : trois états, un garde, et l'absence de signal produit un silence et non un mensonge

## Symptôme mesuré (2026-09-11, tenant cloud d'Al, canary Vietnam)

En réponse à « Qu'est-ce que la doctrine Mika ? », un tenant **cloud** a affirmé
« tout tourne en local, tes données ne quittent pas ta machine ». Faux pour ce
tenant : il tourne dans le cloud. Affirmation de confidentialité fausse, faite à
un invité de la campagne. p1, chemin démo compagne.

## Ce que le code établit — et qui corrige le corps du ticket

Le corps pose la cause ainsi : « La persona / le prompt revendique local-first
inconditionnellement » et demande de « conditionner la revendication au
déploiement ». Les deux supposent **une revendication, dans la persona, à
conditionner**. La mesure en trouve une — mais pas là, et pas atteignable par le
tenant sinistré.

**M1 — aucune persona ne revendique « local ». Mesuré, exhaustivement.**
Recherche FR+EN (`tourne en local`, `100 % local`, `local-first`, `ne quittent
pas ta machine`, `sur ta machine`, `runs locally`, `never leaves your machine`,
`vie privée`, `confidentialité`, `tes données`) :

| Surface | Résultat |
|---|---|
| `FAMILY_SOUL` (`home.rs:648-700`) | **zéro** occurrence |
| `DEFAULT_SOUL` (`home.rs:552-576`) | **zéro** occurrence |
| `CHAMPION_PERSONA_PLACEHOLDER` (`home.rs:84`) | `= PersonaProfile::Family` → `FAMILY_SOUL`, donc idem |
| `MIKA_DEV/QA/TEST/ARCH_SOUL` (`well_known_agents.rs:1238,1322,1362,1393`) | **zéro** |
| `prompt.rs` — toutes les sections code-managées | **zéro** |
| `skills/bundled/**`, `templates/skills/**` (dont `self-knowledge`) | **zéro** (`local` n'apparaît que comme `~/.local/bin`, `local worktree`) |

**M2 — il existe UNE surface qui l'affirme, elle est embarquée dans le binaire,
et elle n'était pas atteignable par le tenant sinistré.** `docs/architecture.md:13-15` :

> **CLI mode (embedded):** The `mika` binary runs locally. … SQLite stores all
> data on the local filesystem.

Ce texte est compilé dans l'agent (`builtin_handlers.rs:22-23`,
`include_str!(concat!(env!("OUT_DIR"), "/docs/architecture.md"))`) et rendu au
LLM **verbatim** par `get_documentation(topic="architecture")`
(`builtin_handlers.rs:154`). Sur les neuf docs embarqués, c'est le seul à porter
une revendication de localité. Il est conditionnel *dans le document* (« CLI mode
(embedded) ») — une conditionnalité qui ne survit pas à un résumé.

**Mais `get_documentation` est porté par la skill `self-knowledge`**
(`templates/skills/self-knowledge/tools.json`), qui est dans
`DEFAULT_AGENT_SKILL_ALLOWLIST` (`home.rs:535`) et **absente** de
`FAMILY_AGENT_SKILL_ALLOWLIST` (`home.rs:587-594`). Un tenant champion porte
`ToolsProfile::Family` (`home.rs:132`) : il **ne peut pas atteindre ce doc**.

**Donc deux producteurs distincts, et il faut les deux :**

| # | Producteur | Population atteinte | Remède |
|---|---|---|---|
| **P1** | **Fabrication** du modèle sur son prior, non démentie : aucune section du prompt ne dit *où* Mika tourne | **toutes**, dont le tenant mesuré | Décisions 1-3 (fait + garde) |
| **P2** | `docs/architecture.md:13-15` servi verbatim | tiers portant `self-knowledge` (opérateur), y compris **un tenant cloud opérateur** | Décision 6 (corriger le doc) |

**Le déplacement du ticket :** sur la population sinistrée il n'y a rien à
conditionner — il y a un fait à **poser** et une fabrication à **empêcher**.

**M3 — le précédent exact existe et a déjà traité cette classe.** mika#1815 : à
« quel modèle es-tu ? », Mika *inférait* au lieu de lire. Remède encore en place :

- `write_runtime_section` (`prompt.rs:939-951`) écrit `## Runtime` — « You are
  currently running on provider `X` model `Y`. **This is the ground truth** …
  Do NOT infer … quote this line verbatim. »
- `write_self_identity_discipline_section` (`prompt.rs:959-996`) l'ancre en
  quatre règles, dont la **3** : « **Fallback honestly.** If ground truth is
  genuinely unavailable … say "I cannot reliably determine my model" … **Never
  fabricate a confident answer.** »

Même classe — une affirmation de fait sur soi-même, fabriquée faute de
vérité-terrain — et le bloc qui doit la porter existe, à l'emplacement déjà
raisonné (`prompt.rs:1142-1146` : « so the "who am I / **what am I running on**"
block reads coherently »). **Son périmètre est aujourd'hui borné, explicitement,
au modèle et au provider** (`prompt.rs:962-963` : « which model you are, which
provider powers you, your configuration, your capabilities »). La question
« **où** tournes-tu » tombe hors de ce périmètre, donc la règle 3 ne s'y applique
pas, donc rien ne s'oppose à la fabrication. C'est le trou, à l'octet près.

**M4 — deux textes vrais que le modèle peut généraliser à tort.** La doctrine
non-transit de la voix (`mika-gateway/src/voice/mod.rs:10-14`, « the audio MUST
NEVER leave the box the user is on … doctrinal invariant ») est **bornée au lane
testimony** — la ligne 8-9 dit l'inverse pour l'autre voie (« Conversation lane —
cloud STT/TTS is permitted »). Et `## Data-Grade Doctrine` (`prompt.rs:1019`,
mika#1798) porte sur les **grades d'accès**, jamais sur la localisation. Aucun
des deux n'est à corriger ; les nommer sert à savoir ce qui nourrit P1.

**M5 — `mika` ne peut pas connaître son hébergement seul.** Aucun signal
n'existe : `grep -rhoE "MIKA_[A-Z0-9_]+" crates/` ne rend ni `MIKA_DEPLOYMENT`,
ni `MIKA_CLOUD`, ni `MIKA_HOSTING`. Deux quasi-proxys, tous deux refusés
(Décision 7) : `customer_id` (`config.rs:819`, pur transport Telegram) et
`telegram_single_bot_mode` côté gateway. Le canal correct est celui de
`MIKA_AGENT_TIER` : posé par le provisionneur **avant** le premier démarrage, lu
une fois, mis en cache. `mika-cloud` n'est pas présent dans cet espace de travail
(`ls /data/workspace/mika-platform/` → `claude-pilot`, `mika`) : son émission est
un **ticket compagnon**, comme mika#2023 → mika-cloud#242.

**M6 — l'ordre entre dépôts est contraint, dans le même sens qu'en mika#2023 M5 :
`mika` d'abord.** Tant que rien n'émet, le lecteur posé ici lit « absent » et se
tait — état sûr. Si `mika-cloud` émettait `MIKA_DEPLOYMENT=cloud` avant que ce
lecteur existe, la variable serait ignorée et le tenant continuerait d'affirmer
« local » : fail-open exactement pendant la fenêtre où l'on croirait le défaut
fermé.

**M7 — un correctif de prompt seul ne tient pas ; c'est mesuré ici, neuf fois.**
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (mika#2120 :
neuf récidives sous prompt contre zéro quand l'opérateur écrivait la consigne à
la main). Le précédent direct est mika#1814, qui pose la doctrine dans le prompt
**et** le garde EndTurn 5c en disant pourquoi (`prompt.rs:45-49` : « The prompt
content is the *intent* half … the structural half is the guard »). C'est le
garde, non le prompt, qui ferme le p1.

## Le correctif — un fait, trois états, un garde

### Décision 1 — trois états, jamais deux

`Deployment { Local, Cloud, Unknown }` dans `mika-common`, à côté d'`AgentTier`
dont il reprend la forme.

| `MIKA_DEPLOYMENT` | Résolution | Justification |
|---|---|---|
| `local` (casse indifférente, `trim`) | `Local` | Peut affirmer l'hébergement local. |
| `cloud` | `Cloud` | Doit dire la vérité vérifiable (Décision 4). |
| **absente** ou `""` | `Unknown` | **N'affirme rien.** Divergence assumée ci-dessous. |
| non vide, non reconnue | `Unknown` + `warn!` nommant la valeur | Plancher fail-closed. |

**La divergence avec mika#2023 M1 est délibérée et doit être écrite, parce que la
règle y est l'inverse.** Là-bas, l'absence de `MIKA_AGENT_TIER` résout vers
`Default` : *« un unset est la forme légitime du poste opérateur, et lui
appliquer la règle fail-closed casserait la machine de Vincent »*
(`home.rs:1650`, `mika2023_unrecognized_tier_value_fails_closed_but_absence_stays_default`).
Ici, l'absence **est exactement la population sinistrée** : le tenant d'Al ne
portait aucune variable, et c'est ce vide qui a produit l'affirmation fausse.
Faire résoudre l'absence vers `Local` serait réécrire le bug en constante.

Le coût est réel et se paie du bon côté : le poste de Vincent, sans variable,
perd le droit d'**affirmer** « je tourne en local ». Il ne perd pas le droit de
le dire s'il le déclare (`MIKA_DEPLOYMENT=local` dans `~/.mika/.env`, une ligne),
et ce qu'il dit à la place — « je ne peux pas déterminer de façon fiable où je
tourne » — est **vrai**. Un silence honnête sur un poste local coûte une phrase ;
une affirmation fausse sur un tenant cloud coûte la confiance d'un invité de
campagne.

**Pourquoi `Unknown` et non `Cloud` comme plancher.** Le plancher n'est pas
« l'hypothèse la moins flatteuse » mais « l'affirmation la moins risquée ».
Affirmer « tu es dans le cloud » à un utilisateur local est aussi une affirmation
fausse — moins dangereuse, même famille. `Unknown` est le seul état qui
n'affirme rien.

### Décision 2 — le fait vit dans `## Runtime`, et la discipline étend son périmètre

Une ligne ajoutée par `write_runtime_section`, sous celle du modèle :

- `Local` → « You are running **on the user's own machine** (local install). »
- `Cloud` → « You are running **in the cloud**, in an isolated per-tenant
  container — not on the user's machine. »
- `Unknown` → « **Your hosting mode is not declared in this environment.** You do
  not know whether you run locally or in the cloud. »

et une **règle 5** dans `## Self-Identity Discipline`, qui élargit le périmètre
énoncé en `prompt.rs:962-963` : « où tu tournes » et « où vivent les données de
l'utilisateur » relèvent de la même vérité-terrain que « quel modèle es-tu ».
L'élargissement **est** le correctif de la moitié *intent* : la règle 3 (*fallback
honestly*) est déjà écrite mot pour mot pour l'état `Unknown`, elle ne
s'appliquait simplement pas à cette question.

**Pourquoi étendre plutôt qu'ajouter une section.** Le bloc est déjà le « what am
I running on », déjà déclaré vérité-terrain, déjà placé avant
Time/Channel/core-memory, et sa discipline est déjà écrite. Une section
`## Hosting` séparée dupliquerait le raisonnement de placement et laisserait deux
endroits où répondre à « parle-moi de toi ».

**Portée des trois assembleurs, décidée et non subie :**

| Assembleur | Décision |
|---|---|
| `build_system_prompt` (`:1145`) | **rendu** — le tour conversationnel est celui de l'incident |
| `build_silent_prompt` (`:1632`) | **rendu** — un heartbeat qui affirme « local » est aussi faux, et les deux appellent déjà `write_runtime_section` |
| `build_compact_system_prompt` (`:1521`) | **non rendu** — carve-out explicite alignée sur mika#1814/mika#1925 (budget ≤ 5 Ko, MikaModel non utilisé pour des tenants réels). **Le garde, lui, s'y applique** : il lit le texte sortant, pas le prompt. La carve-out prive ce chemin de la moitié *intent*, jamais de la protection. À joindre au suivi mika#1925. |

### Décision 3 — le garde structurel, qui ferme réellement le p1

Famille fabrication, inline, **position 5d**, immédiatement après 5c
`doctrine_public_promo` dont il reprend la forme, le suivi `intent_guard_retries`
et la télémétrie `guard.*` (#953). Il tire quand le texte sortant affirme un
hébergement local **et** que le déploiement résolu n'est pas `Local`.

**Ce garde ferme le p1 sans attendre le ticket compagnon**, et c'est ce qui rend
ce ticket livrable seul : un tenant cloud d'aujourd'hui ne porte aucune variable,
résout donc `Unknown`, donc `≠ Local`, donc l'affirmation mesurée est refusée dès
ce déploiement-ci. Le signal `cloud` améliorera la *réponse* (Décision 4) ; il
n'est pas nécessaire au *refus*.

**Deux couches, et la seconde est le point difficile de tout ce plan.** Le remède
que le corps prescrit contient lui-même le mot « local » : « la MÊME stack
open-source (MIT) est self-hostable en local si tu veux ». Un garde qui tirerait
sur le mot bloquerait la phrase vraie qu'on veut faire dire. La discrimination
n'est donc pas lexicale mais **de portée** :

- **Couche A — sujet :** l'hébergement de *cette* instance ou la localisation des
  données de l'utilisateur (`tourne en local`, `tes données ne quittent pas`,
  `sur ta machine`, `tout est local`, `runs locally`, `your data never leaves`,
  `on your machine`, `100 % local`) — bilingue FR/EN : l'incident est FR,
  l'opérateur travaille en EN, et la formulation EN exacte du bug existe déjà
  publiée (M8). Même raison qu'en 5c.
- **Couche B — assertion à la première personne, au présent, sur l'état de
  fait :** `je tourne`, `tout tourne`, `tes données sont/restent`, `I run`,
  `everything runs`, `your data stays`. Elle **ne matche pas** le
  modal/conditionnel (`peut être hébergé`, `tu peux l'installer`,
  `self-hostable`, `si tu veux`, `can be self-hosted`, `you could run`), ni
  l'interrogatif, ni la négation (`je ne tourne pas en local`).

Les deux doivent tirer. Correction par re-prompt unique, comme tous ses voisins ;
le message nomme l'état résolu et la phrase à ne pas tenir. Non exempté par
`skip_remaining_guards` (#1178) : une revue de PR réussie ne donne licence à
aucune affirmation fausse de confidentialité — même raison littérale qu'en 5c.

### Décision 4 — ce que dit un tenant cloud, et la tension avec la persona famille

Le contenu est prescrit par le corps et repris sans le diluer : isolation par
tenant ; les données sont à l'utilisateur et exportables ; la **même** stack
open-source (MIT) est self-hostable en local s'il le souhaite.

**Tension à trancher, trouvée en chemin et non dans le corps du ticket.**
`FAMILY_SOUL:680-681` interdit à Mika « toute mention … de l'infrastructure
sous-jacente — **jamais, même si on te le demande** ». Or le remède prescrit *est*
de l'infrastructure. Deux conséquences, opposées :

1. L'interdiction **n'a pas empêché le bug** : « tout tourne en local » n'est pas
   du jargon technique, la règle le laisse passer. L'interdiction crée le vide
   sans fournir de fait vrai à sa place.
2. Servir le paragraphe cloud complet à un tenant famille/champion **violerait**
   la persona que Vincent a approuvée.

**Arbitrage : deux registres pour un même fait, et le registre suit l'axe
persona, pas l'axe hébergement.** `PersonaProfile::Operator` reçoit la
formulation complète ; `PersonaProfile::Family` reçoit une formulation sans
jargon — « Je tourne sur un serveur, pas sur ton téléphone. Ce que tu me confies
est à toi, et tu peux le récupérer quand tu veux. » — qui est **vraie, non
technique, et suffisante pour ne pas mentir**. Aucune règle dérivée de la locale
ou du compte n'est introduite : interdiction Prime du 2026-09-09, reprise de
mika#2023.

**Cela ajoute un second axe au bloc `## Runtime`** (persona × hébergement). C'est
un coût réel ; il est payé parce que l'alternative est de choisir entre mentir à
un tenant famille et casser sa persona. Le croisement est un `match` exhaustif,
sans `_ =>`, sur le modèle de `tools/mod.rs:308-345` (`dispatch_substrate_diagnostic`)
dont le commentaire dit pourquoi : le compilateur force chaque nouveau tier à
décider.

### Décision 5 — résolu une fois, mis en cache, threadé ; jamais relu par tour

Trajectoire identique à `AgentTier` en mika#1962 : `Deployment::from_env()` lu une
fois à `server::init_agent` (à côté de `server/mod.rs:534-538`), mis en cache sur
`AgentState`, threadé par `AgentParams` / `SilentAgentParams` / `TeamAgentParams`
jusqu'à `PromptContext` / `SilentPromptContext` et jusqu'au `ToolContext` que lit
le garde.

**`PromptContext` ne porte pas de champ `tier` aujourd'hui** (`prompt.rs:854-892`),
alors que `ToolContext` en porte un (`tools/mod.rs:114`) : la Décision 4 en
impose un, et le déploiement voyage avec lui par le même chemin — celui que
`agent_loop/mod.rs:3220/4230/4997` emprunte déjà pour le tier.

**Ce que ça coûte, nommé :** deux champs de plus sur deux structs de contexte,
donc ~30 sites de construction en tests de `prompt.rs` à compléter. Mécanique, et
le compilateur le porte. **Ce que ça évite** est la raison de le payer : lire
l'environnement dans `write_runtime_section` rendrait le fait non testable (aucun
test ne pourrait faire varier l'état sans muter un global de process) et
réintroduirait la lecture par tour que mika#1962 a explicitement retirée.
Conséquence à dire : comme pour le tier, poser ou retirer la variable sur un
process déjà lancé n'a **aucun effet**.

### Décision 6 — corriger `docs/architecture.md:13-15` (producteur P2)

Le paragraphe est vrai pour le mode CLI et faux dès qu'on le cite hors contexte,
et il est servi **verbatim**. Le corriger est une ligne : nommer le mode dans la
phrase elle-même plutôt que dans le titre de puce, et adjoindre la puce
symétrique pour le mode conteneur, qui existe déjà plus bas dans le document mais
ne dit rien de la localisation des données.

**Ce n'est pas un doublon du garde.** Le garde intercepte ce que Mika *affirme* ;
ce doc est ce qu'on lui *donne à lire*. Laisser une source fausse dans le binaire
en comptant sur le garde reviendrait à demander à un détecteur de rattraper une
erreur qu'on aurait pu ne pas commettre — et le garde ne relit pas le contenu
d'un `tool_result`.

**Attention CI :** `docs/` est source de vérité et `crates/mika-agent/docs/` en
est la copie ; la modification doit passer par `scripts/sync-agent-docs.sh`,
faute de quoi le job `docs-sync` échoue.

### Décision 7 — trois dérivations écartées explicitement

Chacune paraît économique et chacune est fausse :

1. **Dériver l'hébergement du tier** (`Champion` est documenté `home.rs:29` comme
   « an external tester **on a cloud tenant** »). Refusé : le tier est un axe
   *produit* (outils + persona), pas un axe de déploiement. Un champion peut être
   installé localement pour un test ; une famille peut être servie depuis le
   cloud — c'est même l'offre. Faire porter au tier un fait qu'il n'a jamais
   décrit, c'est exactement la coupure à un seul axe que mika#2023 a dû défaire.
2. **Dériver de `customer_id`** (`config.rs:819`). Refusé : champ de transport
   Telegram (`messaging.rs:61-125`), non typé, dont l'absence signifie
   « single-bot ou CLI » et jamais « local ».
3. **Dériver de la présence de Kubernetes / du namespace** (`agents_namespace`,
   défaut `"mika-agents"` identique en local). Refusé : un défaut identique des
   deux côtés ne discrimine rien.

## Implémentation, par fichier

| Fichier | Geste |
|---|---|
| `crates/mika-common/src/home.rs` | `Deployment` + `from_env()` (trois états, casse/trim, `warn!` sur valeur non reconnue), près d'`AgentTier` |
| `crates/mika-agent/src/prompt.rs` | `write_runtime_section` prend `(deployment, persona_profile)` et écrit la ligne ; règle 5 de `## Self-Identity Discipline` ; champs sur `PromptContext` / `SilentPromptContext` ; `build_compact_system_prompt` inchangé (carve-out épinglée) |
| `crates/mika-agent/src/evidence/guards.rs` | `detect_false_local_hosting_claim(text) -> Option<…>`, fonction pure à deux couches, à côté de `detect_doctrine_public_promo` |
| `crates/mika-agent/src/agent_loop/mod.rs` | Garde 5d après 5c : `intent_guard_retries`, `GuardCorrelation`, `warn!(event = "guard.false_local_hosting_claim")`, re-prompt |
| site de `DOCTRINE_PUBLIC_PROMO_LABEL` | `FALSE_LOCAL_HOSTING_LABEL` |
| `crates/mika-agent/src/server/{mod.rs,state.rs}` + params structs | Résolution une fois, cache, threading (déploiement **et** persona) |
| `docs/architecture.md` + `scripts/sync-agent-docs.sh` | Décision 6 |
| `crates/mika-agent/tests/eval/doctrine_regressions/` | Scénario de régression |
| `.env.example`, `CLAUDE.md` racine, `crates/mika-agent/CLAUDE.md` | `MIKA_DEPLOYMENT` : trois états, divergence d'absence vis-à-vis de `MIKA_AGENT_TIER`, carve-out compacte, signal opérateur |

## Contrat de vérification

Tout test d'état porte son **contrôle négatif dans le même appel** — discipline
imposée à cette famille par mika#2023 AC2/AC4.

1. `Deployment::from_env` : `cloud` / `CLOUD ` / `local` résolvent ; `""` et
   absence → `Unknown` ; `prod` → `Unknown` + `warn!` nommant `prod`.
2. `build_system_prompt` rend la ligne des trois états × deux registres persona ;
   `build_silent_prompt` idem ; `build_compact_system_prompt` ne la rend **pas**
   (carve-out épinglée comme décision, pas comme oubli).
3. Garde — positif : « tout tourne en local, tes données ne quittent pas ta
   machine » sous `Unknown` **et** sous `Cloud` → tire.
4. Garde — négatif n°1, **le plus important** : la phrase de remède prescrite par
   le corps (« la même stack open-source est self-hostable en local si tu veux »)
   → ne tire pas. Le garde ne doit pas interdire la vérité qu'il existe pour faire
   dire.
5. Garde — négatif n°2 : la même affirmation positive sous `Local` → ne tire pas.
6. Garde — négatif n°3 : interrogatif / négation (`peux-tu tourner en local ?`,
   `je ne tourne pas en local`) → ne tire pas.
7. Persona famille : la ligne `Cloud` servie sous `PersonaProfile::Family` ne
   contient aucun des termes que `FAMILY_SOUL:680-681` interdit (tickets, GitHub,
   agents, skills, conteneur, tenant).
8. `docs/architecture.md` : la ligne 13 ne porte plus d'affirmation de localité
   détachable de son mode ; `scripts/sync-agent-docs.sh` exécuté, job `docs-sync`
   vert.
9. Scénario eval `doctrine_regressions` rejouant la forme mesurée du 2026-09-11
   (question doctrine, agent non-`Local`, réponse fabriquée, garde, tour corrigé),
   rouge sur le code d'avant.
10. `cargo build` + `cargo clippy --all-targets -- -D warnings` + `cargo test`
    verts ; sorties rouge-avant/vert-après des tests 3 et 4 collées au corps de la
    PR (porte mika#2264).

## Surfaces opérateur

- Journal (`$MIKA_SPIRIT_LOG_FILE`) : `guard.false_local_hosting_claim` (WARN —
  champs `deployment`, `persona`, `matched_subject`, `matched_assertion`,
  `guard_correlation_id`), joint à `guard.correction_accepted` par
  `guard_correlation_id` comme toute la famille #953. **Régime nominal attendu :
  zéro ligne.** Toute occurrence est une affirmation de confidentialité fausse
  interceptée ; répétée sur un même tenant, elle signifie que la moitié *intent*
  n'atteint pas ce chemin — vérifier d'abord la carve-out compacte avant de
  toucher au garde.
- Le fait lui-même est dans le prompt, donc lisible via `MIKA_LOG_LLM_BODIES`
  armé **sur mika-spirit** (mika#2220 : armé sur le process CLI, il est inerte
  pour les tours servis par le démon).
- Contrôle par tenant cloud : lire le bloc `## Runtime` du prompt servi. Tant que
  le ticket compagnon n'est pas livré, il porte `Unknown` — état **attendu**, pas
  une panne.

## Sonde post-déploiement, avec sa halte

Sur 48 h, zéro `guard.false_local_hosting_claim` est le résultat espéré, mais
l'absence ne prouve rien seule — personne ne repose la question tous les jours.
Sonde active : rejouer « Qu'est-ce que la doctrine Mika ? » et « mes données
restent-elles chez moi ? » sur un tenant cloud et sur le poste opérateur, et lire
la réponse.

**Halte :** si l'affirmation reparaît sur un tenant cloud alors que
`guard.false_local_hosting_claim` est vide, **ne pas élargir les motifs du
garde** — c'est que le tour concerné passe par un assembleur qui ne le traverse
pas. Établir lequel d'abord.

## Hors périmètre, délibérément

- **L'émission du signal côté `mika-cloud`.** Dépôt absent de cet espace de
  travail ; **ticket compagnon à ouvrir dans le même geste**, `blockedBy`
  bidirectionnel — exigence Prime reprise de mika#2023 AC6 : un AC qui ne peut
  pas fermer depuis son propre dépôt est un AC mal placé. Ordre contraint : `mika`
  d'abord (M6).
- **Le site marketing**, qui porte la formulation exacte du bug et qui est
  **publiquement faux depuis qu'une offre cloud existe** :
  `site/src/components/OpenSource.tsx:25` (« Your data never leaves your
  machine. »), `Hero.tsx:38` (« runs entirely on your machine »),
  `index.html:7` (même phrase en `meta description`). Non atteignable à
  l'exécution, donc sans effet sur le p1 — mais c'est une affirmation de
  confidentialité fausse sur la surface la plus publique du projet, et elle a pu
  entrer dans le prior du modèle. **Ticket de suivi à ouvrir** ; ce n'est pas une
  décision d'ingénierie.
- **Le rendu sur le chemin compact** — carve-out déclarée (Décision 2), sans
  conséquence sur le p1 puisque le garde couvre ce chemin.
- **mika#2247 (fuites de style tenant)** et **le registre de langue du champion**
  (mika#2023 M3 : `FAMILY_SOUL` impose le français à un champion possiblement
  anglophone). Voisins, déjà nommés ailleurs, causes distinctes.
- **La doctrine non-transit de la voix** (`voice/mod.rs:10-14`) : vraie et bornée
  au lane testimony. Rien à corriger ; nommée en M4 parce qu'elle est
  généralisable à tort.
- **La formulation produit de l'argumentaire confidentialité.** Ce plan pose un
  fait vérifiable. Un discours est un slot produit, comme la persona champion — et
  la même interdiction s'applique : aucune règle dérivée de la locale ou du compte
  comme défaut technique.

## Risques

- **Faux positif du garde sur la phrase de remède.** Risque principal, traité par
  la couche B et par deux contrôles négatifs obligatoires (4 et 5). S'il se
  matérialise, Mika ne peut plus dire à un utilisateur cloud que la stack est
  self-hostable — on aurait supprimé la vérité en voulant supprimer le mensonge.
- **Le second axe persona sur `## Runtime`.** Complexité ajoutée à un bloc simple,
  payée pour ne pas avoir à choisir entre mentir à un tenant famille et casser sa
  persona (Décision 4). Le `match` exhaustif est ce qui empêche la dérive.
- **Silence sur le poste opérateur.** Assumé, réversible d'une ligne, documenté —
  faute de quoi il sera vécu comme une régression plutôt que comme une posture.
- **~30 sites de construction en test à compléter.** Mécanique, porté par le
  compilateur, prix explicite de ne pas relire l'environnement par tour.

## Note zone

Aucun chemin visé n'est sous CODEOWNERS (`.github/CODEOWNERS` couvre
`perimeter/`, `verdict_handler.rs`, `pr_merge_with_gate.rs`, `docs/gate/`) : la PR
peut fermer en autonome, ce qui compte pour un p1 sur le chemin démo.

`crates/mika-common/src/home.rs` est un chemin à large rayonnement (tous les
crates en dépendent) : `cargo test` complet, pas seulement le crate touché.
`docs/architecture.md` est embarqué à la compilation — toute modification impose
`scripts/sync-agent-docs.sh` sous peine d'échec du job `docs-sync`.

## Definition of Done

- `Deployment` existe dans `mika-common`, résout trois états depuis
  `MIKA_DEPLOYMENT`, et fail-close vers `Unknown` sur valeur non reconnue **et**
  sur absence.
- Le bloc `## Runtime` porte la ligne d'hébergement dans `build_system_prompt` et
  `build_silent_prompt`, dans les deux registres persona ; la carve-out du chemin
  compact est épinglée par un test.
- `## Self-Identity Discipline` couvre explicitement « où tournes-tu / où vivent
  mes données ».
- Le garde 5d refuse une affirmation d'hébergement local quand le déploiement
  résolu n'est pas `Local`, et laisse passer la phrase de remède prescrite.
- `docs/architecture.md:13-15` ne porte plus d'affirmation de localité détachable
  de son mode ; les copies sont synchronisées et `docs-sync` est vert.
- Le déploiement est résolu une fois par process et mis en cache ; aucun appel à
  `Deployment::from_env()` hors du site de résolution.
- `MIKA_DEPLOYMENT` est documenté dans `.env.example`, le `CLAUDE.md` racine et
  `crates/mika-agent/CLAUDE.md`.
- Le ticket compagnon `mika-cloud` est ouvert, cité dans le corps de la PR, et
  **non fermé** par elle. Le ticket de suivi « site marketing » est ouvert.
- `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test` verts,
  sorties rouge-avant/vert-après jointes.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de son Fix (« conditionner au déploiement ; dire la vérité
vérifiable sur cloud ; ne jamais affirmer 'local' sur un tenant cloud ») et du
contrat de vérification, **après** la correction de trajectoire de M1/M2 : aucune
persona ne revendique « local », donc « conditionner » se lit « poser le fait,
empêcher structurellement la fabrication, et corriger la seule source atteignable
qui l'affirme ».

- **AC1 — le fait existe et a trois états.** `Deployment::from_env()` résout
  `local`→`Local`, `cloud`→`Cloud` (casse indifférente, `trim` conservé) ; absence
  et `""`→`Unknown` ; toute valeur non vide non reconnue→`Unknown` avec un `warn!`
  nommant la valeur. Test rouge-avant/vert-après portant le positif **et** le
  contrôle négatif de l'absence dans le même appel.

- **AC2 — le prompt porte la vérité-terrain.** `build_system_prompt` et
  `build_silent_prompt` rendent, dans `## Runtime`, une ligne distincte par état :
  `Local` (machine de l'utilisateur), `Cloud` (conteneur isolé par tenant, données
  à l'utilisateur et exportables, même stack MIT self-hostable), `Unknown` (mode
  non déclaré, tu ne sais pas). `## Self-Identity Discipline` énonce que
  l'hébergement relève de cette vérité-terrain. `build_compact_system_prompt` ne
  rend pas la ligne, et un test l'épingle comme décision et non comme oubli.

- **AC3 — « local » n'est plus affirmable hors d'un déploiement local.** Le garde
  5d `false_local_hosting_claim` tire sur une affirmation d'hébergement local à la
  première personne au présent quand le déploiement résolu est `Cloud` **ou**
  `Unknown`, corrige par re-prompt unique, et n'est pas exempté par
  `skip_remaining_guards`. **Ce critère ferme le p1 sans dépendre du ticket
  compagnon**, un tenant cloud actuel résolvant `Unknown`.

- **AC4 — la vérité n'est pas emportée avec le mensonge.** Dans le même test que
  l'AC3 : la phrase de remède prescrite par le corps (stack open-source MIT
  self-hostable en local) ne fait pas tirer le garde ; la même affirmation
  positive sous `Deployment::Local` ne le fait pas tirer ; une formulation
  interrogative ou négative non plus.

- **AC5 — la persona famille n'est pas cassée pour être rendue honnête.** La ligne
  `Cloud` servie sous `PersonaProfile::Family` dit la vérité sans aucun des termes
  d'infrastructure que `FAMILY_SOUL:680-681` interdit. Le croisement
  persona × hébergement est un `match` exhaustif sans `_ =>`, pour que le
  compilateur force tout nouveau profil à décider (modèle : `tools/mod.rs:308-345`).

- **AC6 — la source atteignable qui affirme « local » est corrigée.**
  `docs/architecture.md:13-15` ne porte plus d'affirmation de localité détachable
  de son mode, la puce du mode conteneur dit où vivent les données, les copies
  `crates/mika-agent/docs/` sont synchronisées via `scripts/sync-agent-docs.sh` et
  `docs-sync` est vert. C'est le producteur P2 (M2), distinct de celui que ferme
  l'AC3.

- **AC7 — résolu une fois, jamais par tour.** Le déploiement est lu à
  `server::init_agent`, mis en cache sur `AgentState`, threadé par les params
  structs jusqu'au prompt et au garde.
  `grep -rn 'Deployment::from_env()' crates/mika-agent/src/` ne rend aucun
  résultat de production hors du site de résolution — même garde structurelle que
  `AgentTier::from_env()` (mika#1962).

- **AC8 — aucun AC orphelin.** Le ticket compagnon `mika-cloud` portant l'émission
  de `MIKA_DEPLOYMENT=cloud` au provisionnement est ouvert dans le même geste,
  `blockedBy: mika#2290`, cité par le corps de mika#2290 en retour. La PR de ce
  plan le cite et **ne le ferme pas** : l'ordre est `mika` d'abord (M6). Le ticket
  de suivi sur les trois affirmations du site marketing est ouvert et cité.

- **AC9 — régression rejouable.** Un scénario `doctrine_regressions` rejoue la
  forme mesurée du 2026-09-11 (question sur la doctrine, agent non-`Local`,
  réponse fabriquée, garde, tour corrigé) et échoue sur le code d'avant.

- **AC10 — CI verte.** `cargo build` + `cargo clippy --all-targets -- -D warnings`
  + `cargo test` verts ; sorties rouge-avant/vert-après des tests AC3 et AC4
  collées au corps de la PR (porte mika#2264).

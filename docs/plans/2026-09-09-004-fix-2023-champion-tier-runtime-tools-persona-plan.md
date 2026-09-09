---
issue: 2023
type: fix
---

# fix(mika#2023) — le tier champion existe côté runtime : dissociation outils↔persona, fail-closed sur valeur inconnue, slot persona laissé à Vincent

## Contexte

Ticket fiché le 2026-08-28, re-mesuré le 2026-09-09 — douze jours de dérive à inscrire avant de planifier.

**Le temps 1 est livré.** mika-cloud#209 (mergé 2026-08-28T19:29Z) fait résoudre `champion` vers le tier outils `family` : `mika-cloud/api/src/services/provisioning.rs:389`. Un champion nouvellement provisionné ne reçoit donc plus `shell-exec`/`tmux`/`git-ops`/`github`-écriture. Le commentaire de code se qualifie lui-même de « TEMPS-1 MITIGATION, NOT THE END STATE » et nomme mika#2023 comme propriétaire de la décision restante.

**Côté `mika`, rien n'a bougé.** `AgentTier` (`crates/mika-common/src/home.rs:13-22`) a toujours deux variantes ; zéro occurrence de `champion` dans `crates/` hors un commentaire de `mika-gateway/src/routes.rs:2842`.

**Cap posé par Mika Prime** (2026-09-09, session `00000000-0000-0000-0000-000000000000`) sur la question bloquante du fil (« pas dispatchable tant que Vincent n'a pas tranché ») :
- la **forme** — dissociation deux-axes + invariant fail-safe — est portée par l'orchestrateur et remise à l'architecte ;
- le **contenu** — la persona champion finale — est milestone-scope Vincent ;
- le déblocage : poser un **placeholder famille-réutilisée marqué provisoire**, pour que la forme atterrisse sans attendre la réponse produit ;
- interdiction explicite : ne PAS porter « persona suivant la locale du compte » comme défaut technique — c'est un choix produit déguisé.

## Mesures qui corrigent le corps du ticket

Cinq mesures faites contre le code d'aujourd'hui. Trois répondent aux réserves que Prime demandait de lever avant de promettre quoi que ce soit ; deux corrigent le corps du ticket.

**M1 — le fail-open réel est l'ABSENCE, pas la valeur inconnue.** Aucun chemin ne produit aujourd'hui une valeur inconnue : `add-customer.sh:236` refuse tout hors `default|family`, et `build_tier_env_overrides` n'émet que `family` ou rien. Le trou vivant est « tier connu de la console, non mappé côté provisioning → aucun env émis → `AgentTier::Default` = opérateur » — précisément l'incident champion. Or l'absence est **légitime** sur le poste de Vincent (`MIKA_AGENT_TIER` non posé = opérateur, `home.rs:31`). Donc l'AC2 du corps, lu littéralement (« un tier inconnu reste fail-safe »), ne peut pas s'appliquer à l'absence sans casser le poste opérateur. La coupe correcte : valeur non vide non reconnue → fail-closed vers le tier le plus restreint ; absence → `Default` inchangé. Le trou « console ajoute un tier sans mapping » se ferme côté mika-cloud (ticket compagnon, AC6).

**M2 — `tier_guard.rs:70` est une mine que le compilateur ne voit pas.** `assert_family_tier_env_consistency` sort en `Ok` **uniquement** si `tier == AgentTier::Family`. Un `AgentTier::Champion` provisionné avec la persona et l'allowlist famille sur disque — c'est-à-dire le placeholder — verrait la garde détecter une dérive famille et **refuser le démarrage** (`bail!`, `tier_guard.rs:~131`) pour toute la population champion. C'est une comparaison d'égalité, pas un `match` : ajouter la variante ne déclenche aucune erreur de compilation. Sans AC4, la variante casse le boot de tout champion.

**M3 — le placeholder famille porte bien une hypothèse incompatible, et elle est mesurée.** Réserve de Prime confirmée, dans les deux directions :
- `FAMILY_SOUL` (`home.rs:522+`) impose « Tu réponds en **français** natif » et fige une ouverture premier-tour entièrement française. Un champion **anglophone** reçoit donc l'image miroir exacte du bug fiché : accueil FR à un EN.
- `FAMILY_AGENT_SKILL_ALLOWLIST` contient `google-workspace`, dont le prompt prescrit `gws auth login` (mika#2024, ticket frère, cause distincte, non résolu).

Le placeholder est sûr **sur l'axe outils** — ce qui est l'objet du p0 — et décalé sur l'axe registre. Prix nommé, pas caché.

**M4 — la dissociation est chirurgicale.** Réserve de Prime sur la taille : levée par la mesure. Un seul `match` exhaustif sur `AgentTier` hors `home.rs` (`crates/mika-agent/src/tools/mod.rs:316`), plus une comparaison d'égalité (`tier_guard.rs:70`, cf. M2). Les ~25 autres occurrences sont du portage de champ (`tier: self.tier`, hérité de mika#1962), pas des points de décision.

**M5 — séquencement : `mika` d'abord, jamais l'inverse.** Tant que mika-cloud émet `family` pour un champion (temps 1), la variante `Champion` est **inerte à l'exécution** : aucun champion ne l'atteint. Si mika-cloud basculait d'abord vers l'émission de `champion`, les champions traverseraient le chemin valeur-inconnue — sûr *après* l'AC2 de ce plan, fail-open *avant*. L'ordre est donc contraint : ce ticket, puis le compagnon mika-cloud.

## Re-partition des critères d'acceptation

Le corps enjambe deux dépôts et la moitié a déjà atterri. Prime a confirmé la re-partition et ajouté une exigence.

| AC du corps | Disposition |
|---|---|
| AC1 (tier champion représenté runtime) | **Porté ici** → AC1/AC3 ci-dessous |
| AC2 (`build_tier_env_overrides` ≠ `if tier == "family"`) | **Satisfait** par mika-cloud#209 (`provisioning.rs:389`) ; le volet mika-side devient l'AC2 ci-dessous |
| AC3 (test `non_family_tiers…` réécrit) | **Satisfait** par mika-cloud#209 (`champion_tier_resolves_to_family_tools_tier`, `provisioning.rs:825`) |
| AC4 (champion sans `shell-exec`/`tmux`/`git-ops`/`github`) | **Atténué** par le temps 1 ; durci ici via AC3 (outils famille liés à `Champion` côté runtime) |
| AC5 (francophone accueilli en français) | **Satisfait par accident** du placeholder — et inversé pour un anglophone (M3). Reformulé : relève du slot Vincent + mika#2247 |
| AC6 (`add-customer.sh --tier`) | **Déplacé** → ticket compagnon mika-cloud (AC6 ci-dessous). Un AC qui ne peut pas fermer depuis son propre dépôt est un AC mal placé |

Exigence Prime : le ticket compagnon s'ouvre **dans le même geste** que la re-partition, `blockedBy` bidirectionnel — aucun AC ne doit tomber entre les deux tickets.

## Acceptance criteria

- **AC1** — `AgentTier::Champion` existe dans `crates/mika-common/src/home.rs` et `from_env()` la résout depuis `MIKA_AGENT_TIER=champion` (insensible à la casse, `trim` conservé). Test : `champion` et `CHAMPION ` → `AgentTier::Champion`.

- **AC2** — **Fail-closed sur valeur non reconnue.** Une valeur non vide hors `{default, family, champion}` résout vers le tier d'outils le **plus restreint** (famille) et non plus vers `Default`, avec le `warn!` conservé nommant la valeur. L'absence, `""` et `default` restent `Default` — le poste opérateur est légitime (M1). Test rouge-avant/vert-après portant les **deux contrôles dans le même appel** : positif `MIKA_AGENT_TIER=pro` → tier restreint (aujourd'hui `Default`) ; négatif, variable absente → `Default` (inchangé).

- **AC3** — **Dissociation outils↔persona.** `AgentTier` expose les deux axes séparément : l'axe outils (l'allowlist de skills) et l'axe persona (`identity_toml()`/`soul_md()`) ne dérivent plus d'une correspondance unique. `Champion` = **outils famille** + **persona placeholder famille**, marquée en clair `// PLACEHOLDER (mika#2023) — contenu propriété de Vincent, remplaçable en un site`. **Aucune règle de locale n'est introduite** (interdiction Prime). Test : `Champion` rend l'allowlist famille, et le site de remplacement de la persona est unique.

- **AC4** — **`tier_guard` ne casse pas le boot champion.** `assert_family_tier_env_consistency` accepte un agent provisionné famille-sur-disque quand le tier du process est `Champion` — le placeholder rend cet état *attendu*, pas une dérive. Test rouge-avant/vert-après avec contrôle négatif conservé dans le même test : disque famille + tier `Champion` → `bail!` aujourd'hui / `Ok` après ; disque famille + tier `Default` → `bail!` avant **et** après (M2).

- **AC5** — Le `match ctx.tier` de `crates/mika-agent/src/tools/mod.rs:316` traite `Champion` explicitement, sans `_ =>` fourre-tout : le compilateur devient le gardien à la place du `warn!`. Choix documenté : `Champion` route le diagnostic substrat vers `audit_events` comme `Family` — un testeur externe n'est pas le lecteur d'un diagnostic de substrat.

- **AC6** — **Aucun AC orphelin.** Le ticket compagnon **senara-solutions/mika-cloud#242** est ouvert (2026-09-09, dans le même geste que cette re-partition, exigence Prime), portant (a) le vocabulaire réel de `add-customer.sh --tier` (`scripts/add-customer.sh:236`) et (b) le trou M1 « un tier connu de la console mais non mappé n'émet rien et retombe en opérateur ». Il porte `blockedBy: mika#2023` ; le corps de mika#2023 le cite en retour. La PR de ce plan cite mika-cloud#242 dans son corps et **ne le ferme pas** — l'ordre est mika d'abord (M5).

- **AC7** — `cargo build` + `cargo clippy --all-targets -- -D warnings` + `cargo test` VERTS. Les sorties rouge-avant/vert-après des tests AC2 et AC4 sont collées au corps de la PR (porte mika#2264).

## Hors scope — nommé, pas éludé

- **La persona champion finale.** Slot Vincent, milestone-scope (cap Prime 2026-09-09). Ce plan pose un placeholder et le marque ; il ne tranche pas ce qu'est la voix d'un champion.
- **« Persona suivant la locale du compte ».** Écartée explicitement par Prime : choix produit déguisé en défaut technique.
- **L'accueil d'un champion anglophone** (M3). Contrepartie assumée du placeholder ; relève du slot Vincent et recoupe mika#2247 (fuites style/langue).
- **`gws auth login` héritée via `google-workspace`** dans l'allowlist famille : mika#2024, ticket frère, cause distincte, tient même une fois ce ticket réglé.
- **Les tenants champion déjà provisionnés avant le 2026-08-28.** `write_default_if_missing` (`home.rs:279`) ne réécrit jamais un `identity.toml` existant : un champion provisionné avant mika-cloud#209 **garde la surface opérateur sur disque**, et aucun changement de code ici ne l'en retire. C'est un geste d'exploitation (re-provision ou édition du disque), pas une ligne de Rust. **À vérifier avant tout lancement** — le fil du ticket note qu'Axelle était `champion`/`pending` le 2026-08-28 ; si un tel tenant existe et a bootstrappé avant le fix, le p0 reste ouvert *pour lui* malgré ce plan.

## Note zone

Aucun chemin visé n'est sous CODEOWNERS (`.github/CODEOWNERS` couvre `perimeter/`, `verdict_handler.rs`, `pr_merge_with_gate.rs`, `docs/gate/`). La PR peut donc fermer en autonome — utile pour un bloqueur de lancement.

`crates/mika-common/src/home.rs` est un chemin à large rayonnement (tous les crates en dépendent) : `cargo test` complet, pas seulement le crate touché.

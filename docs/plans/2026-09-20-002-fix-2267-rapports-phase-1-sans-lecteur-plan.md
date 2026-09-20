# Plan — fix(manager) : un rapport Phase 1 écrit dans un puits n'est pas un rapport livré (mika#2267)

**Ticket :** senara-solutions/mika#2267 — p2
**Classe :** substrat d'observabilité + surface de lecture — LECTURE SEULE, aucune autorité d'écriture ajoutée
**Branche :** `bug/2267/manager-les-rapports-phase-1-du-reader`

---

## 1. Le défaut, et la rectification que la lecture du code impose au ticket

Le ticket pose la question ainsi : *« le canal de livraison cm des rapports du
manager semble cassé (sink offline vs endpoint, à déterminer) »*, et propose deux
branches : **réparer la livraison OU documenter le sink de repli**.

La lecture du code déplace le diagnostic d'un cran, et ce déplacement est le
premier livrable du grooming. **Le canal n'est pas « cassé » au sens d'un tuyau
percé : le sink de repli n'a aucun lecteur.** Recherche exhaustive sur l'arbre —
`offline_sink` / `OFFLINE_SINK` n'apparaît qu'à trois endroits : `cadence.rs`
(l'écriture), `spawn.rs` (la config), `crates/mika-agent/CLAUDE.md` (la prose).
**Zéro commande CLI, zéro outil, zéro route HTTP, zéro runbook, zéro consommateur.**

`write_offline_sink` dépose un `.md` horodaté dans un répertoire que rien ne
liste, rien n'expose et aucune procédure ne nomme. Du point de vue du lecteur
humain — celui qui doit rendre le verdict de fidélité — **« le canal est cassé »
et « le canal est un puits » produisent exactement les mêmes octets : aucun.**
C'est la classe que la maison a déjà dû nommer plusieurs fois (mika#2205 : *un
scan silencieusement inactif se lit exactement comme un scan qui n'a rien trouvé
à faire* ; mika#2131 sur les exclusions muettes).

### Ce que la lecture établit, point par point

**(a) Trois chemins d'écriture, zéro chemin de lecture.**
`select_route` (`cadence.rs:556`) rend `Route::Http` quand l'URL applicable est
posée et non vide, `Route::OfflineSink` sinon. À quoi s'ajoute un troisième
chemin, le **repli après échec HTTP** (`cadence.rs:503`). Les trois finissent
écrits ; aucun n'est lu.

**(b) `delivered = true` sur les trois chemins.** HTTP succès (`:482`), repli
après échec (`:504`), sink direct (`:516`) — tous posent `delivered = true` et
sauvent le checkpoint. Donc `CycleOutcome::delivered`, la seule sortie structurée
du cycle, **ne distingue pas « lu par un humain » de « écrit dans un puits »**. Un
opérateur qui demande « est-ce que ça livre ? » obtient « oui » dans les trois cas,
y compris celui où personne ne lira jamais.

**(c) Le repli après échec est le seul chemin muet.** Sur `Err(e)`, le code émet
`manager_cycle_delivery_failed` (WARN), écrit le sink, pose `delivered = true` —
et **n'émet aucun `manager_cycle_delivered`**. Les deux autres chemins en émettent
un, porteur d'un champ `route`. C'est donc le seul chemin où un rapport est écrit
sans qu'aucune ligne ne dise qu'il l'a été, ni où.

**(d) Aucun événement ne dit OÙ.** `manager_cadence_start` (`spawn.rs:249`) porte
`delivery_url_set: bool` et `escalation_url_set: bool` — deux booléens. Ni le
chemin du sink, ni sa provenance, ni ce que la prochaine livraison prendra comme
route. Le chemin est composé **en ligne** dans `manager_config_from_env`
(`spawn.rs:168`) et n'est jamais journalisé. Un opérateur qui cherche ses rapports
ne peut pas savoir où regarder sans lire le source. C'est le motif
`llm_budget_resolved` (mika#2293) non appliqué ici : *un réglage qu'on ne peut pas
observer n'est pas un réglage, c'est un espoir.*

**(e) La documentation pose un piège, et se contredit à six lignes d'écart.**
`crates/mika-agent/CLAUDE.md:1208` nomme **`MIKA_MANAGER_SINK_DIR`** ; le code lit
`MIKA_MANAGER_OFFLINE_SINK_DIR`, que la ligne 1214 de la **même** doc nomme
correctement. Un opérateur qui suit la ligne 1208 pose une variable inerte et ses
rapports atterrissent ailleurs qu'il ne croit — exactement la classe mika#1971
(*« a deployer who followed it put the key in the one place it could not work »*).

**(f) Aucune variable `MIKA_MANAGER_*` n'est déclarée dans le dépôt.** Ni
`.env.example`, ni `docker-compose.yml`, ni `packaging/`. Le canal n'a jamais eu
de déclaration versionnée, donc aucune revue n'a jamais pu constater son absence.

---

## 2. Contrainte de périmètre : la branche « réparer la livraison » n'est pas livrable ici

Le ticket nomme le **canal cm** (control-monitor). `control-monitor` **n'est pas
dans ce workspace** — il ne contient que `claude-pilot/` et `mika/`. Aucune ligne
de ce dépôt ne peut faire exister un endpoint qui vit ailleurs, ni le rendre
vivant.

Ce plan prend donc la **seconde branche que le ticket autorise explicitement** —
« OU documenter le sink de repli » — en la rendant meilleure qu'une
documentation : **un lecteur**. Une prose qui dit « les rapports sont dans
`~/.mika/manager/sink/` » laisse le verdict de fidélité dépendre d'un `ls` qu'il
faut se rappeler ; une sous-commande le rend exécutable et, surtout, **nomme le
chemin qu'elle a consulté**, ce qui est la seule façon de rendre lisible le cas
où la CLI et le daemon ne regardent pas au même endroit.

La branche cm reste ouverte en **ticket de suivi**, et son préalable est une
mesure que ce plan produit : tant qu'on ne sait pas si une URL est seulement
posée, « réparer le canal » serait réparer à l'aveugle.

---

## 3. Ce qui est livré

### C1 — Un résolveur unique du chemin du sink

Le chemin est aujourd'hui composé en ligne dans `manager_config_from_env`. Un
lecteur CLI qui le recomposerait de son côté pourrait diverger de l'écrivain — et
**un lecteur qui regarde ailleurs que l'écrivain est précisément le défaut qu'on
ferme**, reproduit une couche plus haut.

Extraire dans `spawn.rs` :

```rust
/// Where the offline sink lives, and by which door that was decided.
pub enum SinkDirSource { Env, Default }

/// The ONE resolver of the offline-sink directory. The writer (`cadence`) and
/// the reader (`mika milestone reports`) both go through it, so a reader that
/// looks elsewhere than the writer is not expressible.
pub fn resolve_offline_sink_dir() -> (PathBuf, SinkDirSource)
```

`manager_config_from_env` l'appelle au lieu de composer. Même traitement pour
`checkpoint_dir` si le coût est nul ; sinon laissé tel quel — le sink est ce que
le ticket concerne.

**Garde structurelle** (modèle `mika2305_the_scope_has_a_single_decisional_reader`,
`mika1883_..._the_one_helper`) : un scan de source refuse tout autre site lisant
`ENV_OFFLINE_SINK_DIR` ou composant `.join("sink")`. **Allowlist livrée vide** :
quand il tire, on retire le second site, on ne l'allowliste pas.

### C2 — `mika milestone reports` : le lecteur

`crates/mika-cli/src/cli.rs` — nouvelle variante de `MilestoneCommand` :

```rust
/// List Phase 1 reports written to the offline sink, most recent first.
Reports {
    /// Restrict to one milestone: `<owner/repo>#<number>`. Absent → all.
    #[arg(long)] target: Option<String>,
    /// Render the most recent report's Markdown to stdout instead of listing.
    #[arg(long)] latest: bool,
    /// How many entries to list. Ignored with `--latest`.
    #[arg(long, default_value_t = 20)] limit: usize,
    #[arg(long, value_enum, default_value = "text")] format: OutputFormat,
}
```

Implémentation dans `crates/mika-cli/src/commands/milestone.rs` (adaptateur mince
de 84 lignes aujourd'hui) : résout via C1, liste les `.md`, trie par horodatage
décroissant (porté par le nom de fichier, `<slug>-<ts>.md`).

**La propriété porteuse : répertoire absent ≠ répertoire vide.** Deux sorties
distinctes, jamais une liste vide dans les deux cas.

- **Absent** → nomme le chemin consulté, sa provenance (`env` / `default`), et
  dit que la cadence n'a encore rien écrit **ici** — avec la variable à poser si
  le sink vit ailleurs.
- **Présent et vide** → « aucun rapport », en nommant quand même le chemin.

Sans cette distinction, le lecteur reproduirait à sa surface le défaut qu'il
existe pour fermer : un silence qui se lit comme une absence de travail.

La sortie **nomme toujours le chemin consulté**, y compris sur le chemin nominal.
C'est ce qui rend lisible en une ligne le cas où la CLI (lancée par l'opérateur)
et le daemon (lancé par le service, potentiellement sous un autre `HOME`) ne
résolvent pas le même répertoire — enquête autrement longue.

### C3 — `manager_delivery_resolved` : l'événement qui tranche la question du ticket

Émis une fois au démarrage de la cadence, à côté de `manager_cadence_start`, sur
`target: "mika::milestone_manager"` :

| champ | valeur |
|---|---|
| `milestone` | la cible |
| `route_normal` | `"http"` \| `"offline_sink"` — ce qu'une sévérité non-`Blocked` prendra |
| `route_escalation` | `"http"` \| `"offline_sink"` — ce qu'une `Blocked` prendra |
| `delivery_url_set` / `escalation_url_set` | booléens |
| `delivery_token_present` | **booléen** — jamais la valeur, jamais un préfixe, jamais une longueur |
| `offline_sink_dir` | le chemin **résolu** |
| `sink_dir_source` | `"env"` \| `"default"` |

**Événement distinct plutôt qu'enrichissement de `manager_cadence_start`** : c'est
un événement de *configuration*, il répond à « où vont mes rapports ? » et non à
« la cadence a démarré », et il doit se grep seul. Précédent exact et même
raisonnement : `llm_budget_resolved` (mika#2293). Modifier la forme d'un événement
que des sondes existantes lisent serait par ailleurs un changement de format de fil
gratuit.

**Le booléen sur le token est une contrainte dure**, pas une précaution de style —
doctrine `api_key_present` du CLAUDE.md racine, assertée par un test négatif.

### C4 — Le repli après échec cesse d'être muet

Dans la branche `Err(e)` de `cadence.rs`, après `write_offline_sink`, émettre un
`manager_cycle_delivered` portant `route = "offline_sink_fallback"` — **distinct
de `"offline_sink"`**.

Les deux populations disent des choses opposées et doivent rester comptables
séparément : `offline_sink` = *aucune URL n'était posée* (état de bring-up
nominal, rien n'est en panne) ; `offline_sink_fallback` = *une URL était posée et
a échoué* (panne réelle). Les fondre effacerait très exactement la distinction que
le ticket demande de faire — « sink offline vs endpoint, à déterminer ».

`route` devient un **format de fil** : constantes d'un seul lieu + test qui épingle
les trois valeurs (modèle `mika2131_filter_names_are_a_wire_format`). Deux
orthographes d'une même route couperaient une population en deux sans le dire.

### C5 — La doc cesse de mentir, et les variables sont déclarées

- `crates/mika-agent/CLAUDE.md:1208` : `MIKA_MANAGER_SINK_DIR` →
  `MIKA_MANAGER_OFFLINE_SINK_DIR`.
- La même section documente `mika milestone reports` et le fait que **le sink est
  un puits sans lui**.
- `.env.example` : déclarer les dix `MIKA_MANAGER_*` avec leurs défauts.
- **Garde structurelle** : tout `ENV_*` const de `spawn.rs` doit être nommé dans
  `.env.example` (modèle `labels_this_module_writes_are_declared_in_labels_yml`).
  Ferme durablement la classe (e)/(f) plutôt que de corriger l'occurrence.

---

## 4. Ce qui n'est PAS livré, et pourquoi

1. **La livraison HTTP vers cm n'est pas réparée.** `control-monitor` est hors
   workspace (§ 2). **Ticket de suivi**, préalable = la mesure de C3 : réparer un
   canal sans savoir si une URL est seulement posée serait réparer à l'aveugle.

2. **`CycleOutcome::delivered` ne change pas de sémantique.** Il reste vrai sur les
   trois chemins. C'est un champ de sortie lu par les tests et la télémétrie ; le
   durcir changerait un contrat public pour un gain que C3 et C4 donnent déjà. Ce
   qui manquait n'était pas un booléen plus sévère, **c'était de savoir où**. Un
   futur ticket qui voudrait vraiment distinguer « lu » de « écrit » devra d'abord
   définir ce que « lu » veut dire — ce n'est pas une question de booléen.

3. **Le format du sink ne change pas.** `write_offline_sink` continue d'écrire le
   seul `report_markdown`, pas le `DeliveryBody` complet. **Conséquence nommée :
   le lecteur ne peut pas filtrer par sévérité** — elle vit dans le corps du
   rapport, pas dans une en-tête machine. Écrire le JSON complet serait plus riche
   mais changerait le format des fichiers déjà écrits en production, et rien dans
   le ticket ne demande ce filtre. Le tri par date suffit au besoin mesuré : lire
   le dernier rapport. Limite nommée, pas oubli.

4. **L'alarme d'auth sans `escalation_url` n'écrit toujours rien au sink.**
   Trouvé en chemin : `emit_auth_alarm` (`spawn.rs:1046`) retourne sans écrire
   quand l'URL est absente ; seul l'`error!` subsiste. Trou réel mais **distinct**
   — un rapport perdu n'est pas une alarme perdue, et le corriger demande de
   décider ce qu'une alarme dans un puits veut dire. **Ticket de suivi.**

5. **Rien ne devient Phase 2.** Aucune autorité d'écriture n'est ajoutée : le
   lecteur lit, la télémétrie journalise. `no_dispatch_test.rs` reste vert par
   construction, et son jeu de tokens interdits n'est pas touché.

---

## 5. Verification contract

| # | Test | Ce qu'il attrape |
|---|---|---|
| T1 | `reports_lists_sink_entries_most_recent_first` | ordre de tri |
| T2 | `reports_absent_dir_is_distinguishable_from_empty_dir` | **le test porteur** — deux sorties distinctes ; contrôle négatif : si les deux rendent la même chose, il rougit |
| T3 | `sink_dir_resolution_has_a_single_reader` | scan de source, allowlist vide — un second site de résolution |
| T4 | `delivery_route_names_are_a_wire_format` | épingle `"http"` / `"offline_sink"` / `"offline_sink_fallback"` |
| T5 | `http_failure_fallback_emits_its_own_route` | comportemental — un deliverer qui échoue produit la ligne `offline_sink_fallback` ; **avant le fix, zéro ligne** |
| T6 | `delivery_resolved_carries_the_resolved_path_and_never_the_token` | présence du chemin + provenance ; **assertion négative** : la valeur du token n'apparaît dans aucun champ |
| T7 | `every_manager_env_const_is_declared_in_env_example` | scan — ferme la classe (e)/(f) |

**Injection-verified** (doctrine `feedback_verify_pipeline_passes_without_the_fix`,
précédent `todos/mika-manager-cadence-wiring-injection-verification.md`) : pour
**T2, T5 et T6**, documenter dans `todos/mika-2267-injection-verification.md`
l'inversion qui les fait rougir (fondre les deux branches de T2 ; retirer
l'émission de T5 ; passer le token en clair pour T6), puis restaurer et re-vérifier
le vert.

T3, T4 et T7 sont des **scans de source**, délibérément : leur régression ne
rendrait aucune décision fausse, elle la rendrait invisible — et toutes les
assertions comportementales resteraient vertes pendant que l'opérateur reperdrait
la réponse.

**Non-régression :** `cargo test -p mika-agent`, `cargo test -p mika-cli`,
`cargo clippy`, `make verify-bundled-skills`, `no_dispatch_test.rs` vert.

---

## 6. Surfaces opérateur et sondes post-déploiement

### Sonde 1 — trancher la question du ticket (« sink offline vs endpoint »)

```bash
grep manager_delivery_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{route_normal, route_escalation, delivery_url_set, offline_sink_dir, sink_dir_source}'
```

- `route_normal: "offline_sink"` → **aucune URL n'a jamais été posée.** Les
  rapports sont sur disque depuis le début et le canal cm n'a jamais existé. Le
  remède est un geste de configuration plus le ticket de suivi cm — **pas une
  correction de code.**
- `route_normal: "http"` → une URL est posée : passer à la sonde 2.
- **Halte — aucune ligne alors que la cadence tourne** : le binaire déployé est
  antérieur au correctif (classe mika#2340). **Établir le déploiement avant toute
  conclusion sur le canal** — c'est précisément l'erreur que ce plan existe pour
  rendre impossible.

### Sonde 2 — si une URL est posée

```bash
grep manager_cycle_delivered "$MIKA_SPIRIT_LOG_FILE" | jq -r .route | sort | uniq -c
grep -c manager_cycle_delivery_failed "$MIKA_SPIRIT_LOG_FILE"
```

- `http` dominant → la livraison marche et le symptôme est **en aval** : c'est le
  consommateur cm qui ne rend pas les rapports. **Halte : ne pas toucher ce dépôt.**
- `offline_sink_fallback` dominant → l'endpoint refuse ;
  `manager_cycle_delivery_failed` porte l'erreur. Le ticket de suivi cm s'ouvre
  **avec un compte plutôt qu'avec une intuition**.

### Sonde 3 — le verdict de fidélité redevient possible (l'objet du ticket)

```bash
mika milestone reports --latest
```

Doit rendre un rapport Phase 1 non vide. **C'est le critère qui débloque le
verdict de fidélité du Reader.**

- **Halte — sortie vide alors que la sonde 1 dit `offline_sink`** : la CLI et le
  daemon ne résolvent pas le même chemin (`HOME` différent, service sous un autre
  utilisateur). Le lecteur **nomme le chemin consulté** pour que cette halte se
  lise en une ligne au lieu d'une enquête. Comparer avec `offline_sink_dir` de la
  sonde 1.

### Régimes attendus

- `manager_delivery_resolved` : **une ligne par démarrage**, toujours présente.
  Son absence signifie un binaire antérieur, jamais « tout va bien ».
- `offline_sink_fallback` : **zéro en régime sain.** Toute occurrence est un
  endpoint configuré qui refuse.
- `manager_cycle_delivery_failed` : zéro en régime sain, corrélé au précédent.

---

## 7. Definition of Done

- [ ] C1 — résolveur unique + garde structurelle à allowlist vide
- [ ] C2 — `mika milestone reports` avec `--target` / `--latest` / `--limit` / `--format`, absent ≠ vide, chemin toujours nommé
- [ ] C3 — `manager_delivery_resolved` émis au démarrage, token en booléen seul
- [ ] C4 — `offline_sink_fallback` émis, `route` figé comme format de fil
- [ ] C5 — `CLAUDE.md:1208` corrigé, `.env.example` peuplé, garde de déclaration
- [ ] T1–T7 verts ; T2/T5/T6 injection-verified dans `todos/mika-2267-injection-verification.md`
- [ ] `cargo test` / `cargo clippy` / `make verify-bundled-skills` verts ; `no_dispatch_test.rs` vert
- [ ] Deux tickets de suivi ouverts : **canal cm** (§ 4.1) et **alarme d'auth sans sink** (§ 4.4)
- [ ] Corps de PR nommant le déplacement de diagnostic (§ 1) et la contrainte de périmètre (§ 2)

---

## Acceptance criteria

Le corps de mika#2267 ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés des trois questions « À investiguer » du ticket et du symptôme
bloquant (le verdict de fidélité du Reader).

- [ ] **AC1 — La question « où les rapports Phase 1 sont-ils émis, et vers quel
      canal ? » se répond par une commande, sans lire le source.** Après
      déploiement, `grep manager_delivery_resolved "$MIKA_SPIRIT_LOG_FILE"` rend une
      ligne portant `route_normal`, `route_escalation`, `offline_sink_dir` (chemin
      résolu) et `sink_dir_source` (`env` \| `default`).

- [ ] **AC2 — « Le canal est-il configuré/vivant ? » est séparable en deux états
      distincts.** `route = "offline_sink"` (aucune URL posée) et
      `route = "offline_sink_fallback"` (URL posée, livraison échouée) sont deux
      valeurs différentes portées par des lignes `manager_cycle_delivered`
      distinctes. Avant le correctif, le second cas n'émettait aucune ligne.

- [ ] **AC3 — Le sink de repli a un lecteur.** `mika milestone reports` liste les
      rapports du sink du plus récent au plus ancien ; `--latest` rend le Markdown
      du plus récent sur stdout ; `--target <owner/repo>#<n>` restreint à une
      milestone.

- [ ] **AC4 — Un sink absent ne se lit pas comme un sink vide.** Répertoire
      inexistant → message nommant le chemin consulté, sa provenance et la variable
      à poser. Répertoire présent et vide → « aucun rapport », chemin nommé. Les
      deux sorties sont distinctes et un test le vérifie.

- [ ] **AC5 — Le chemin du sink a un résolveur unique.** Écrivain (`cadence`) et
      lecteur (CLI) passent par la même fonction ; un scan de source à allowlist
      vide refuse tout second site lisant `ENV_OFFLINE_SINK_DIR` ou composant
      `.join("sink")`.

- [ ] **AC6 — Aucun matériel de credential ne fuit.** `manager_delivery_resolved`
      porte `delivery_token_present` en **booléen** ; un test assertant
      négativement vérifie que la valeur du token n'apparaît dans aucun champ.

- [ ] **AC7 — Les valeurs de `route` sont un format de fil figé.** `"http"`,
      `"offline_sink"`, `"offline_sink_fallback"` sont des constantes d'un seul
      lieu, épinglées par test ; un renommage est une rupture à dater.

- [ ] **AC8 — La documentation cesse de nommer une variable inerte.**
      `crates/mika-agent/CLAUDE.md:1208` nomme `MIKA_MANAGER_OFFLINE_SINK_DIR` ;
      les dix `MIKA_MANAGER_*` sont déclarées dans `.env.example` ; un test refuse
      un `ENV_*` const non déclaré.

- [ ] **AC9 — Aucune autorité d'écriture n'est ajoutée.** `no_dispatch_test.rs`
      reste vert sans modification de ses `FORBIDDEN_TOKENS` ; le module reste
      LECTURE SEULE.

- [ ] **AC10 — Injection-verified.** `todos/mika-2267-injection-verification.md`
      documente, pour T2, T5 et T6, l'inversion qui fait rougir le test puis le
      retour au vert.

---

## 8. Risques et limites

| Risque | Traitement |
|---|---|
| CLI et daemon résolvent des `HOME` différents → lecteur vide malgré un sink peuplé | Le lecteur **nomme le chemin consulté** ; sonde 3 et sa halte |
| Le correctif fait croire que le canal cm est réparé | § 2 et § 4.1 le disent en toutes lettres ; le corps de PR le répète |
| Le sink croît sans borne (aucune rotation) | Hors périmètre, **nommé** : le lecteur rend le volume visible, ce qui est le préalable à une décision de rotation. Ticket de suivi si la sonde montre un volume problématique |
| Pas de filtre par sévérité dans le lecteur | § 4.3 — limite nommée, tri par date suffit au besoin mesuré |

**Ce que ce travail n'achète pas :** il ne fait pas arriver un rapport chez un
destinataire distant. Il rend **lisible** ce qui est écrit, **attribuable** le
chemin pris, et **mesurable** la question que le ticket posait sans pouvoir y
répondre. Si les rapports doivent atteindre cm, c'est le ticket de suivi — et il
s'ouvrira avec la mesure de la sonde 1 plutôt qu'avec une hypothèse.

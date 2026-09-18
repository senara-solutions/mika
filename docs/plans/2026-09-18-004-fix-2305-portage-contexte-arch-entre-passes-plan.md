# mika#2305 — le portage de contexte entre passes arch : voulu sur un axe, fuite sur l'autre, et rien ne le dit

> Ticket : `senara-solutions/mika#2305`
> Type : fix (substrat — observabilité + garde de non-régression)
> Date : 2026-09-18

---

## Contexte

Le ticket observe, sur la table ronde porte-4-couches du 11/09, des passes arch
successives rendant des **verdicts différents sur un plan inchangé**, et pose une
question binaire : le portage de contexte est-il **voulu** (mémoire d'agent délibérée)
ou est-ce une **fuite de session** ?

La réponse mesurée est : **les deux, sur deux axes que le ticket confond**. Et l'axe où
la fuite était réelle n'est pas celui que le ticket nomme — elle ne vivait pas dans
`--session-id`, qui fait correctement son travail, mais dans l'assembleur de prompt, qui
ignorait la frontière de session que `--session-id` venait d'établir.

Le ticket est explicitement borné par son opérateur : « substrat borné, sans PR ni
pilote ». Ce plan ne change aucun comportement de produit. Il tranche la question par
écrit, rend observable le réglage qui décide, et pose la garde qui manquait.

---

## Ce qui est établi, et comment le vérifier

Quatre faits lus dans le code, chacun vérifiable par une commande. **Les numéros de ligne
sont relevés sur `main` au 2026-09-18** et périment ; ce sont des aides à la relecture, et
c'est la commande de vérification donnée sous chaque fait qui fait foi — pas le nombre.

### E1 — `--session-id` est explicite et délibéré. La prémisse du ticket est fausse sur son axe nommé

`_arch_ask` (`skills/bundled/_shared/dispatch-lib.sh:4640`) prend un `session_id`
**optionnel en `$3`** et le pose en `--session-id` (ligne 4688). `_iterate_groom_loop`
en fait trois usages, tous délibérés et tous commentés :

| Appel | `session_id` passé ? | Intention écrite dans le code |
|---|---|---|
| 1ʳᵉ passe `mika-arch-groom-ticket` (l. 5437) | **non** | session neuve |
| retry UNPARSED (l. 5457) | **oui** | « the retry reuses `$session_id` so the architect sees its own prior turn and can complete it (idempotent) » |
| 2ᵉ passe `mika-arch-second-review` (l. 5500, 5549) | **oui** | « continuing the architect session so findings stay in conversation memory (per `mika-arch-second-review` session-continuity contract) » |

Le session-id est même **inscrit dans le body-callout** publié sur le ticket
(`_write_canonical_callout`, l. 5194-5200 : `session-id: ${session_id}`). Un contrat plus
explicite que celui-là n'existe pas dans ce dépôt.

**Vérification :** `grep -n "session_id" skills/bundled/_shared/dispatch-lib.sh | sed -n '/4640,5600/p'`
— ou directement `sed -n '4680,4692p;5437p;5457p;5500p' skills/bundled/_shared/dispatch-lib.sh`.

**Conséquence : le portage intra-invocation est VOULU.** Il n'y a rien à corriger là, et
la « piste » du ticket (« décider d'un contrat explicite : session neuve par défaut,
`--session-id` pour continuer ») décrit **le contrat déjà en vigueur**.

### E2 — mika-arch n'est pas singleton : un `mika ask` sans `--session-id` mint un UUID frais

Second canal candidat, et il est fermé. `ask.rs:130-144` : quand `--session-id` est
absent, `ask` retombe sur la **session canonique** — mais seulement si l'agent est
singleton. `resolve_canonical_session_id` (`prompt.rs:841`) renvoie `None` quand
`!identity.session.singleton`, et `build_mika_arch_identity` n'émet **aucun** bloc
`[session]` : `grep -n "singleton" crates/mika-agent/src/well_known_agents.rs` ne rend
**rien**. `SessionIdentityConfig::default()` s'applique, et la 1ʳᵉ passe reçoit donc bien
un UUID neuf.

**Vérification :** `grep -c singleton crates/mika-agent/src/well_known_agents.rs` → `0`.

### E3 — la fuite était réelle, et elle vivait dans l'assembleur, pas dans le dispatch

`HistoryScope::Agent` est le **défaut** du champ, et sa doc le dit mot pour mot
(`prompt.rs:407`) : *« Every session of this agent — the pre-mika#2295 behaviour, and the
default. »* Le site de production unique est `agent_loop/mod.rs:4256-4262` :

```rust
let scoped_session_id = match history_config.scope {
    prompt::HistoryScope::Session => Some(session_id),
    prompt::HistoryScope::Agent   => None,      // ← toutes sessions confondues
};
let mut history = db.rebuild_context(scoped_session_id, scope_task_id, 20).await?;
```

Sous `Agent`, `rebuild_context(None, …, 20)` tire les **20 derniers messages de l'agent,
toutes sessions confondues**. Pour un architecte, ces 20 items sont 20 plans et revues
entiers — le commentaire du code le dit déjà : *« twenty of them is twenty times an
unknown quantity »*.

mika-arch **déclare** `scope = "session"` depuis mika#2295/#2327
(`well_known_agents.rs:408-410`). Mais ce ship a mergé **inerte** : `CLAUDE.md`
(§ `MIKA_DISABLE_AGENT_PROVISIONING`) enregistre que `write_default_if_missing` ne
réécrit jamais un `identity.toml` existant, que les identités des quatre agents dataient
du 26/07, et que le seul chemin capable d'écrire la section — le réconciliateur — était
derrière le même `return` que la gel de `config.toml`. Corrigé par **mika#2330**.

Le symptôme mesuré, inscrit tel quel au `CLAUDE.md` :

> history 190–205 KB over 20 messages **spanning 9–10 distinct sessions**,
> `truncated_messages = 0`, arch input 83–89 k against an expected < 40 k

« 20 messages répartis sur 9–10 sessions distinctes » **est** le phénomène du 11/09
décrit par #2305, à la mesure près. Les verdicts des passes 3 et 4 dépendaient bien de
l'historique d'autres passes — mais portées par la fenêtre de contexte, pas par un
`--session-id` fuité.

**Conséquence : le portage inter-invocations était une FUITE, réelle et mesurée, et elle
est DÉJÀ FERMÉE** par mika#2295 (le réglage) + mika#2330 (sa mise en vigueur).

**Vérification :** `sed -n '4256,4262p' crates/mika-agent/src/agent_loop/mod.rs` et
`sed -n '403,410p' crates/mika-agent/src/prompt.rs`.

### E4 — ce qui n'est pas fermé : rien ne prouve la fermeture, rien ne la protège

Trois trous, dans l'ordre où ils coûtent.

**(a) Le réglage qui décide n'est pas observable.** L'instrument existe déjà —
`emit_context_window_assembled` (mika#2295 brique 0, `agent_loop/mod.rs:4270`) — et il
porte **`distinct_sessions`** (`build_context_window_fields`, l. 7165-7169), c'est-à-dire
très exactement le nombre que la question du ticket demande. Mais il **ne porte pas le
`scope` effectif**. Or `distinct_sessions > 1` a deux causes de signes opposés : le scope
est retombé à `agent` (défaut silencieux, le défaut de #2305), **ou** la session itère
légitimement (plan v1 → revue → plan v2, le cas nominal d'un ITERATE). L'instrument ne
peut pas les séparer. C'est la leçon de mika#2293, non appliquée ici : *un réglage qu'on
ne peut pas observer n'est pas un réglage, c'est un espoir* — et c'est précisément ce qui
a laissé mika#2327 échouer en silence pendant des semaines.

**(b) Le garde existant garde la constante, pas le comportement.**
`mika2295_mika_arch_identity_bounds_its_conversation_window` assert sur le TOML
**construit en mémoire**. C'est exactement ce qui était vert pendant toute la période où
la production tournait en `Agent` — le test ne touche pas le site de production, et son
propre voisin le dit (`mika2295_history_block_is_declared_code_owned` : *« This guards the
constant and nothing else: it exercises no code path and proves no write »*). Si demain
`agent_loop/mod.rs:4257` est réécrit, ou si un futur agent-reviewer naît sans déclarer
son scope, le défaut revient et **ressemble à un fonctionnement normal**.

**(c) Canal résiduel que `scope` ne borne pas : la mémoire agent-scoped.**
`update_core_memory`, `store_fact` et `update_fact` sont **délibérément actifs** pour
mika-arch (`MIKA_ARCH_DISABLED_TOOLS` ne les contient pas, et
`test_mika_arch_disabled_tools_excludes_agent_self_state` le pinne comme une décision :
*« agent self-state, not platform side-effect »*). `search_memory` n'est pas désactivé
non plus. Ces canaux traversent **toutes** les sessions par conception, et `scope =
"session"` ne les touche pas. Si mika-arch mémorise un jugement, la passe suivante en est
teintée — ce qui est « voulu » au sens du design, et qui est aussi, littéralement, « le
contexte d'une passe teinte la suivante ».

**Vérification :** `grep -n -A40 "fn build_context_window_fields" crates/mika-agent/src/agent_loop/mod.rs`
et `sed -n '300,341p' crates/mika-agent/src/well_known_agents.rs`.

---

## Décisions

### D1 — la question du ticket est tranchée par écrit, et c'est le livrable principal

Le ticket demande une **décision**, pas un correctif. Elle est :

> **Le portage intra-invocation (1ʳᵉ passe → 2ᵉ passe, et retry UNPARSED) est VOULU,
> explicite et contractuel.** Le portage **inter-invocations** était une **FUITE**,
> réelle et mesurée, portée non par `--session-id` mais par `HistoryScope::Agent`, le
> défaut de la fenêtre de contexte — **déjà fermée** par mika#2295 + mika#2330.

Le contrat que la « piste » du ticket appelle de ses vœux (« session neuve par défaut,
`--session-id` pour continuer ») **est déjà celui en vigueur**, sur les deux moitiés :
neuve par défaut côté `ask` (E2), continuée sur demande explicite côté dispatch (E1).

Ce plan **ne le change pas**. Il le documente, le rend observable, et le protège.

### D2 — le scope effectif rejoint l'événement qui existe, il n'en crée pas un second

Ajouter `history_scope` (`"session"` | `"agent"`) aux champs de
`context_window_assembled`, à côté de `distinct_sessions` qu'il porte déjà. **Un champ
sur un événement existant, pas un nouvel événement** : les deux nombres n'ont de sens
qu'ensemble, et les séparer imposerait à l'opérateur une jointure pour reconstituer une
seule phrase.

Ce qui devient alors lisible en une commande, et ne l'était pas :

| `history_scope` | `distinct_sessions` | Lecture |
|---|---|---|
| `session` | `1` | **nominal** — la passe ne voit qu'elle-même |
| `session` | `> 1` | **halte** — le filtre ne filtre pas ; la fuite est sous `rebuild_context`, pas dans le réglage |
| `agent` | `> 1` | **le défaut de #2305, revenu** — le réglage n'a pas atterri sur le disque (classe mika#2330) |
| `agent` | `1` | scope permissif, fenêtre pauvre par accident — vrai aujourd'hui, faux demain |

**Refusé : émettre la provenance du scope** (à la manière de `llm_budget_resolved`). Le
scope ne vient pas d'une cascade à cinq portes ; il vient d'un `identity.toml` et d'un
seul. La provenance se réduirait à « `identity.toml` ou le défaut », ce que le nom de la
valeur dit déjà.

### D3 — le garde se pose au site de production, pas sur une seconde constante

Un test qui assert de nouveau sur le TOML construit ajouterait une ligne verte à côté de
celle qui était déjà verte pendant la panne. Le garde doit exercer
`agent_loop/mod.rs:4257` : monter une identité `scope = "session"`, écrire des messages
dans **deux** sessions distinctes du même agent, lancer un tour sur la seconde, et
vérifier que la fenêtre assemblée ne contient **aucun** message de la première. Puis le
contrôle négatif : la même chose sous `scope = "agent"` **doit** les contenir — sans quoi
le test passerait aussi sur un `rebuild_context` qui ne rendrait jamais rien.

**Le contrôle négatif est porteur** : c'est lui qui distingue « le filtre filtre » de
« la requête est cassée ».

### D4 — le canal mémoire est nommé, pas fermé

Fermer (c) — retirer `update_core_memory` / `store_fact` / `search_memory` à mika-arch —
serait **amputer la mémoire d'un agent**, c'est-à-dire une décision produit, prise contre
un pin explicite qui la déclare constitutive. Elle n'est pas dans le périmètre d'un
ticket substrat borné, et rien dans l'évidence du 11/09 ne l'implique : E3 explique le
phénomène observé en entier, sans avoir besoin de ce canal.

Il est donc **écrit** — au plan, au `CLAUDE.md`, et dans le ticket de suivi — et laissé
intact. Ce qui ferme la question du ticket est de dire que ce canal existe, pas de
prétendre qu'il n'existe pas.

### D5 — « même verdict 3/3 » n'est pas acheté par ce travail, et le dire fait partie du livrable

Le ticket adosse la reproductibilité du pré-vol (a) à ce correctif. Deux raisons pour
lesquelles elle ne peut pas en découler, et qu'il vaut mieux écrire maintenant que
découvrir sur une sonde rouge :

1. **Un LLM n'est pas déterministe.** Une session neuve garantit un *même état de départ*,
   jamais un *même verdict*. « 3/3 » reste une propriété statistique.
2. **Les deux passes ne posent pas la même question.** `mika-arch-groom-ticket` rend
   READY/ITERATE/ESCALATE ; `mika-arch-second-review` rend GROOMED/ESCALATE. Des verdicts
   différents entre 1ʳᵉ et 2ᵉ passe sur un plan inchangé sont **le design**, pas un
   symptôme — et depuis mika#2363 les deux passes ne voient même plus le même prompt
   (`--only-skill` évince les passes sœurs).

Ce que le correctif achète est plus étroit et réel : **une passe ne lit plus les plans
d'autres tickets.**

### D6 — le retry UNPARSED est correct, et le ticket a tort sur ce point précis

Le ticket craint qu'« un retry qui hérite du contexte d'une passe ratée n'est pas un vrai
re-essai propre ». Mais ce retry ne **re-demande pas** la revue : il demande à
l'architecte de **compléter sa propre réponse** en y ajoutant la ligne `Disposition:`
manquante (le prompt correctif, l. 5445-5456, dit exactement cela). Sans la session, la
demande serait inintelligible — l'architecte n'aurait aucune réponse à compléter. Le
portage est ici la **condition de correction** du mécanisme, pas sa contamination.

**Aucun changement.** Documenté pour que la prochaine lecture ne le « corrige » pas.

---

## Volets d'implémentation

### V1 — `history_scope` sur `context_window_assembled` (D2)

- `ContextWindowFields` : champ `history_scope: &'static str`.
- `build_context_window_fields` : prend le scope en paramètre et le rend
  (`"session"` / `"agent"`), via un `match` **exhaustif sans `_ =>`** sur `HistoryScope`,
  pour qu'une future variante force une décision plutôt que d'hériter d'une étiquette
  fausse.
- Site d'émission `agent_loop/mod.rs` : le scope est déjà en main
  (`history_config.scope`), aucune signature de couche à élargir.
- **Ungated** : `context_window_assembled` l'est déjà ; c'est un événement de
  configuration de fenêtre et il doit rester lisible quand la télémétrie d'appel est
  coupée.
- **Coût connu d'avance, pour qu'il ne soit pas découvert :** `build_context_window_fields`
  est une fonction **pure à cinq paramètres**, appelée par au moins trois tests existants
  (`mika2295_distinct_sessions_is_the_cross_ticket_contamination_detector` et voisins,
  `agent_loop/mod.rs` ~l. 13853, 13865, 13924, 14050). Le sixième paramètre les casse tous
  — mécaniquement, pas sémantiquement. Le passer **au site d'émission** plutôt qu'au
  constructeur éviterait ces retouches, et c'est précisément pourquoi on ne le fait pas :
  le `match` exhaustif de T4 doit vivre dans la fonction pure, seul endroit où il est
  assertable sans souscripteur `tracing`. La retouche des call-sites est le prix de la
  testabilité, payé sciemment.

### V2 — garde comportemental au site de production (D3)

Test d'intégration : deux sessions du même agent, une identité par scope, assertion sur
la fenêtre assemblée + contrôle négatif. Placé auprès des tests de `rebuild_context` /
du tour agent, pas dans `well_known_agents.rs` — il exerce un chemin, il ne lit pas une
constante.

### V3 — garde structurel : un seul lecteur du scope

Scan de source refusant un second `match` sur `HistoryScope` hors du site de production
et de la désérialisation. Motif maison (`mika2205_periodic_scans_do_not_read_the_pat_field_directly`,
`grooming_marker::tests::no_grooming_regex_outside_this_module`) : un deuxième lecteur ne
rendrait aucune décision fausse, il ferait diverger deux réponses à une même question —
la classe que `grooming_marker` a dû engraver une fois.

**Le périmètre du scan est la partie difficile, et l'écrire ici évite le réflexe qui le
viderait.** `HistoryScope::Session` / `HistoryScope::Agent` apparaissent aujourd'hui à sept
endroits légitimes qui ne sont **pas** des lecteurs : trois assertions d'égalité dans
`well_known_agents.rs` (l. 1672, 1765, 1777) et quatre dans `prompt.rs` (l. 4241, 4258,
4263, 4297) — toutes sous `#[cfg(test)]`. Un scan qui refuserait « toute mention » rougirait
donc **immédiatement sur du code sain**, et la réparation naturelle serait de l'élargir
jusqu'à ce qu'il n'attrape plus rien. Le prédicat porte sur la **construction `match`**, pas
sur le nom du type, et exclut les modules de test. Contrôle de bonne foi obligatoire : le
scan doit rougir sur un `match` décisionnel ajouté ailleurs — sans quoi il est vert parce
qu'il ne regarde rien, ce qui est la panne que T2 rend visible sur l'autre axe.

### V4 — documentation

- `CLAUDE.md` : entrée courte sous l'observabilité, donnant la table de lecture de D2, la
  réponse tranchée de D1, et le canal résiduel de D4. **Pas de re-narration de mika#2295 /
  mika#2330** : y renvoyer.
- `crates/mika-agent/CLAUDE.md` : le contrat de session des passes arch (E1), au plus près
  de la description du loop.

### V5 — sonde de production (§ ci-dessous)

Aucun code. C'est ce volet qui **prouve** que la fermeture a pris — la partie que
mika#2327 n'avait pas et qui lui a coûté trois semaines d'inertie.

---

## Verification contract

| # | Test | Ce qu'il rougit si on le casse |
|---|---|---|
| T1 | `scope = session`, deux sessions peuplées → la fenêtre ne contient que la courante | la fuite E3 est revenue au site de production |
| T2 | **contrôle négatif** : `scope = agent`, mêmes données → la fenêtre contient les deux | T1 passait parce que `rebuild_context` ne rendait rien |
| T3 | `context_window_assembled` porte `history_scope`, valeur conforme au scope résolu | l'instrument redevient incapable de séparer les deux causes de `distinct_sessions > 1` |
| T4 | `match` exhaustif sur `HistoryScope` (pas de `_ =>`) au constructeur de champs | une variante future hérite silencieusement d'une étiquette fausse |
| T5 | scan de source : un seul `match` décisionnel sur `HistoryScope` hors désérialisation, **modules de test exclus** | un second lecteur diverge sans rien casser de visible |
| T5b | **contrôle de bonne foi de T5** : un `match` décisionnel ajouté hors du site de production le fait rougir | T5 est vert parce que son prédicat ne regarde rien |
| T6 | mika-arch : `scope = Session` **et** `context.history` ∈ `CODE_OWNED_IDENTITY_SECTIONS` (tests existants, inchangés) | retour de la classe mika#2330 (ship inerte) |
| T7 | `_arch_ask` passe `--session-id` ssl `$3` non vide ; `_iterate_groom_loop` ne le passe pas en 1ʳᵉ passe et le passe en 2ᵉ et au retry (`test-dispatch-lib.sh`) | le contrat E1 dérive sans que personne le remarque |
| T8 | `cargo clippy -D warnings` + suite verte | — |

---

## Fire-Disposition

- **T2 rouge (le contrôle négatif ne voit pas les deux sessions)** → **halte**. T1 ne
  prouve alors rien ; réparer le montage du test avant toute conclusion sur le filtre.
- **T1 rouge mais T6 vert** → le réglage est bien sur le disque et n'est pas honoré : le
  défaut est sous `rebuild_context`, **pas** dans l'identité. Ne pas re-déclarer le scope.
- **T6 rouge** → classe mika#2330, pas mika#2305. Le correctif est un geste de
  réconciliation d'identité ; ce plan est hors sujet pour cette panne.
- **T7 rouge** → quelqu'un a « corrigé » le portage intra-invocation en croyant fermer
  #2305. Restaurer, et lire D6.
- **Sonde de production rouge alors que T1–T3 sont verts** → le tour arch passe par un
  assembleur que ce site ne traverse pas. **Halte** : établir lequel avant de toucher au
  prédicat (même règle que la sonde mika#2290).

---

## Definition of Done

1. La question du ticket est tranchée par écrit, avec sa preuve : portage
   intra-invocation **voulu**, portage inter-invocations **fuite déjà fermée**, et l'axe
   réel (`HistoryScope::Agent`) nommé.
2. `context_window_assembled` porte `history_scope` à côté de `distinct_sessions`, et la
   table de lecture des quatre combinaisons est documentée.
3. Un garde **comportemental** exerce le site de production, avec son contrôle négatif.
4. Un garde **structurel** refuse un second lecteur du scope.
5. Le canal mémoire agent-scoped est nommé comme non borné par `scope`, et laissé intact.
6. `D5` est écrit : ce travail n'achète pas « même verdict 3/3 ».
7. Le contrat de session de `_arch_ask` / `_iterate_groom_loop` est pinné côté shell.
8. `cargo build` + `cargo clippy -D warnings` + suite de tests verts.
9. `CLAUDE.md` et `crates/mika-agent/CLAUDE.md` à jour, sans re-narrer mika#2295/#2330.
10. Aucun comportement produit modifié : aucun scope changé, aucun outil retiré, aucune
    valeur de réglage déplacée.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous sont
dérivés de sa « Question à trancher », de son « Pourquoi ça compte », et du contrat de
vérification ci-dessus.

1. **AC1 — la question est tranchée avec sa preuve.** Le plan et le `CLAUDE.md` répondent
   « voulu » ou « fuite » **par axe**, chaque réponse adossée à un site de code cité
   (`dispatch-lib.sh:4688`, `prompt.rs:841`, `agent_loop/mod.rs:4257`), jamais à une
   impression.
2. **AC2 — la localisation demandée par la « Piste » est produite.** Les trois lieux où
   une session arch est (ré)utilisée sont énumérés avec leur intention, et le lieu de la
   fuite réelle — qui n'est aucun des trois — est nommé.
3. **AC3 — le scope effectif est observable en production.** Une seule commande sur
   `$MIKA_SPIRIT_LOG_FILE` rend, pour un tour arch, le scope en vigueur **et** le nombre
   de sessions distinctes tirées.
4. **AC4 — `distinct_sessions > 1` est désormais interprétable.** Les quatre combinaisons
   `(scope, distinct_sessions)` ont une lecture écrite, dont deux sont des haltes.
5. **AC5 — la non-régression est comportementale.** Un test échoue si le filtrage par
   session cesse d'être appliqué au site de production, et un contrôle négatif échoue si
   ce test devient vacuellement vert.
6. **AC6 — le canal résiduel est déclaré.** La mémoire agent-scoped de mika-arch est
   nommée comme traversant les sessions par conception et non bornée par `scope`, avec
   son ticket de suivi ouvert.
7. **AC7 — la reproductibilité n'est pas sur-promise.** Il est écrit que ce travail ne
   rend pas les verdicts déterministes, et pourquoi des verdicts 1ʳᵉ/2ᵉ passe différents
   sur un plan inchangé sont le design.
8. **AC8 — le retry UNPARSED est préservé.** Il continue de porter la session, avec la
   raison écrite ; aucun changement de comportement.
9. **AC9 — périmètre tenu.** Le diff ne modifie aucune valeur de réglage, ne retire aucun
   outil, ne touche ni `dispatch-lib.sh` (hors tests) ni les identités.

---

## Surfaces opérateur et sonde post-déploiement

**La commande, après V1 :**

```bash
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-arch")
        | {history_scope, distinct_sessions, message_count, history_bytes, truncated_messages}'
```

**Régime attendu pour mika-arch :** `history_scope: "session"` sur **toutes** les lignes,
et `distinct_sessions: 1` sur la quasi-totalité.

**Sonde, sur 48 h et au moins 10 tours arch — avec ses haltes :**

- `history_scope != "session"` ne serait-ce qu'une fois → **le défaut de #2305 est revenu**.
  Classe mika#2330 (réglage non atterri sur le disque) : lire `identity_reconcile` avant
  de toucher au code.
- `history_scope == "session"` avec `distinct_sessions > 1` → **halte**. Le filtre ne
  filtre pas. Ne pas rétrécir `max_tokens` par réflexe : c'est l'autre axe, et il
  masquerait le symptôme sans toucher la cause.
- `history_bytes` retombé dans l'ordre de grandeur pré-incident (à comparer aux **190–205 KB /
  9–10 sessions** enregistrés au `CLAUDE.md`) → la fermeture a pris.
- **Silence de la sonde ne prouve rien si aucun tour arch n'a tourné.** Vérifier
  `message_count` non nul avant de conclure — un instrument muet et un instrument sain
  se ressemblent (mika#2205).

**Sonde symptomatique, facultative :** rejouer deux fois la 1ʳᵉ passe sur un plan
inchangé et comparer les verdicts. **Une divergence n'est pas une panne** (D5) ; ce qui
serait une panne, c'est `distinct_sessions > 1` sur l'une des deux.

---

## Hors périmètre (suivi à ouvrir)

1. **Borner ou retirer la mémoire agent-scoped de mika-arch** (D4). `update_core_memory`,
   `store_fact`, `update_fact`, `search_memory` traversent toutes les sessions par
   conception et sont pinnés comme constitutifs. Les fermer est une décision produit,
   contre un pin explicite. **Ticket de suivi**, avec pour préalable une mesure : ces
   outils sont-ils seulement appelés pendant une passe de grooming ?
2. **Le déterminisme du pré-vol (a)** — « même verdict 3/3 » comme propriété mesurée, avec
   son protocole et son seuil. Question d'évaluation (famille `calibrate-mika-arch`), pas
   de substrat. **Ticket de suivi.**
3. **La taille du prompt système d'arch** — 54 KB → 59,8 KB au 01/09, déjà hors périmètre
   de mika#2189 et attaqué par mika#2363. Ce plan n'y touche pas.
4. **`rewind.rs`**, second appelant de `rebuild_context` : chemin administratif, pas un
   tour ; l'instrument n'y est délibérément pas émis (déjà décidé par mika#2295).

# mika#2136 — un `send_message` échoué ne peut pas se clore en silence

- **Ticket :** senara-solutions/mika#2136
- **Priorité :** p2-normal (ne casse pas la boucle ; casse la confiance du destinataire)
- **Branche :** `fix/2136/agent-l-agent-affirme-une-livraison-qu`
- **Lignage :** mika#2134 (la garde de longueur, qui est en amont et déjà livrée), mika#2118
  (dire la vérité opérationnelle plutôt qu'une réussite de façade — même racine, autre site),
  mika#2126 (markdown Telegram, hors sujet ici), mika#650/#1090 (`NoChannel`, dont le
  traitement est délibérément asymétrique et le reste), mika#1331 (assert-grounded),
  mika#1645 (equivalence-claim), mika#2290 (5d), mika#2276 M2 (`deadline_verdict` — le
  précédent structurel exact), mika#1783 (`substrate_diagnostic` — le canal typé
  outil→moteur qu'on réemploie)

---

## Contexte

Le 2026-09-01, sur le Telegram d'Al, deux dégâts distincts dans la même conversation :

1. Un document de plus de 12 000 caractères est refusé par la garde de l'outil. L'agent
   répond **« Le voici en entier 👆 »**. Rien n'est jamais arrivé. Al doit insister pour
   que l'agent admette la limite — qu'il **connaissait avant de répondre**, chiffrée, dans
   le `tool_result`.
2. Al demande la découpe. L'agent envoie en commençant par **« Partie 2/4 »**. La partie
   1/4 est morte au transport, et l'agent enchaîne sans le dire.

Le ticket pose la question de conception plutôt que le remède : *« quelle structure rend
inignorable l'échec d'un `send_message` »*, et nomme trois pistes — un état de tour qui ne
peut pas se clore sur un envoi échoué non acquitté, une reformulation du `ToolOutput`, ou
autre chose. Il rappelle aussi que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` interdit de traiter
ça par une ligne de prompt.

Le ticket ajoute une contrainte de méthode : le harnais `tests/eval/golden/` rejoue des
réponses scriptées et *« ne peut donc pas attester une règle de comportement »*. C'est
exact, et **c'est un argument pour la structure** : un refus décidé par le moteur se teste
précisément avec des réponses scriptées, puisque ce n'est pas le modèle qui décide. Voir
E7.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le moteur voit l'échec, et aucun des onze guards ne le regarde

`tool_execution/dispatch.rs:320` : `let tool_succeeded = !output.is_error && !non_zero_exit;`
puis ligne 343 `success: tool_succeeded` dans le `ToolCallSummary` poussé dans
`all_tool_summaries`. Un `send_message` refusé ou mort produit donc **déjà** une entrée
`{ name: "send_message", success: false }` que la boucle détient au moment du EndTurn.

Et onze guards lisent `all_tool_summaries` à la clôture (required-tools, completion-claim,
milestone-close, fabricated-action, dev-groom, 6c, 6d, 6e…). Aucun ne regarde la livraison :

```
$ grep -c "send_message" crates/mika-agent/src/evidence/guards.rs
0
```

**Conséquence pour AC1.** Le fait brut « un envoi a échoué » est déjà dans l'état du tour ;
ce qui manque est (a) sa **qualification** (voir E2), et (b) un lecteur à la clôture. Le
plan ne prétend donc pas créer une information qui existe : il la type et la lit.

### E2 — Les deux étages sont typés à la source et perdus dès la sortie de l'outil

`crates/mika-agent/src/tools/send_message.rs` a trois sorties `is_error = true`, qui ne
disent pas la même chose :

| ligne | cause | ce qui est parti |
|---|---|---|
| `:59-66` | garde de longueur (mika#2134) | **rien** — refus avant persistance et avant transport |
| `:93-95` | `SendOutcome::Failed` | le message est parti et est **mort** ; sauvegardé dans `failed_sends` |
| `:117` | erreur infra du sender | rien de garanti |

En aval, ces trois cas sont indistinguables autrement qu'en relisant la prose anglaise du
message d'erreur (`output_summary`, tronqué et scrubbé). **La réparation attendue n'est pas
la même** : pour un refus de longueur, elle est une découpe ; pour un échec transport, elle
est un ré-essai du même texte. Un prédicat qui confondrait les deux produirait du bruit sur
le cas le plus fréquent — un agent qui rédige 5 000 caractères se fait refuser tous les
jours, découpe, et a raison.

### E3 — Deux sorties qui ne délivrent rien rendent pourtant `success`

`SendOutcome::NoChannel` (`:102-113`) et l'absence de sender configuré (`:127-133`)
retournent `ToolOutput::success` **à dessein**, avec le commentaire qui en donne la raison :
`ToolOutput::error` y déclencherait une boucle de ré-essai sur une condition permanente
(#650, garde de régression mika#1090 citée dans le test `test_send_message_no_channel`).
C'est une population réelle de « livraison affirmée sans livraison », et elle est **hors
périmètre** (voir § Hors périmètre) : la traiter ici rouvrirait la boucle que #650 a fermée.

### E4 — En mode conversation, le texte final EST le canal

`server/handlers.rs:1541` : `if let Some(response) = output.text { … sender_arc.send(&response) }`.
Le texte de clôture part sur Telegram comme n'importe quel message. « Le voici en entier 👆 »
est arrivé par là. C'est donc à la fois le lieu du dégât et le seul point où le moteur peut
faire arriver un fait à l'utilisateur sans passer par le modèle.

### E5 — Le précédent structurel existe, il est récent, et il est à la même place

mika#2276 M2 a posé `AgentOutput.deadline_exceeded: Option<DeadlineOverrun>` avec, dans sa
propre doc, le raisonnement qu'on reprend mot pour mot :

> **Why this field exists at all.** `text` cannot answer the question. […] the call site
> could not tell "the turn answered" from "the turn was cut off".

et `post_deadline_verdict_if_cut_off(state, a, &output, …)` appelé en `handlers.rs:1537`,
**avant** l'envoi du texte, avec le commentaire « Placé AVANT l'envoi sur le canal de
réponse, à dessein ». Ici la question est « le tour s'est terminé » ≠ « ce qu'il dit avoir
livré est parti », et le point d'insertion est le même.

### E6 — `ToolOutput` porte déjà un canal typé outil→moteur

`tools/mod.rs:237` : `pub substrate_diagnostic: Option<String>` (mika#1783), **jamais rendu
au LLM**, construit exclusivement par un constructeur dédié « so the discipline is enforced
at the type layer ». C'est exactement la forme dont on a besoin, et son coût est mesuré :
aucune construction littérale `ToolOutput { … }` n'existe hors de `tools/mod.rs`
(`grep -rn "ToolOutput {" crates/mika-agent/src/ | grep -v tools/mod.rs` ne rend que des
types de retour), donc ajouter un champ ne touche aucun site appelant.

### E7 — Le harnais peut attester ce correctif-ci

`tests/eval/harness.rs:382` : `pub fn message_sender(mut self, sender: Arc<dyn MessageSender>) -> Self`.
Un `MessageSender` de test qui refuse le premier fragment et accepte les suivants est
injectable dans `run_agent` de bout en bout. Le harnais ne peut pas attester qu'un **modèle**
dira la vérité — le ticket a raison — mais il atteste parfaitement qu'un **moteur** refuse
de clore. AC6 est donc atteignable sans provider réel.

### E8 — Le `ToolCallSummary` ne suffit pas pour décider « réparé »

`input_summary` est tronqué à `INPUT_SUMMARY_MAX` puis scrubbé (`dispatch.rs:216`). Décider
« ce fragment a été renvoyé avec succès » par égalité sur un préfixe tronqué confondrait deux
fragments partageant leur début — et la direction de l'erreur est mauvaise : on croirait
réparé un fragment mort, donc on se tairait. C'est exactement le silence dont le ticket est
né. D'où D2.

---

## Décisions

### D1 — Structure, sur les deux moitiés ; le prompt garde une part nommée (AC5)

**Décision : le correctif est structurel, en trois pièces, et la part laissée au prompt est
la *formulation* de l'aveu, jamais son *déclenchement*.**

Raison. `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` mesure neuf
récidives sous application par prompt contre zéro quand le fait est posé par le code
(mika#2120). Le ticket est un cas d'école : l'information était **déjà** dans le contexte du
modèle, chiffrée, et il a répondu le contraire. Ajouter une phrase à la Grounding rule
reviendrait à répéter plus fort ce qui n'a pas marché.

Ce qui reste au prompt, et pourquoi ça ne peut pas être structurel : **dire à l'utilisateur
ce qui s'est passé**, dans sa langue et dans le registre de sa persona. mika#2290 a établi
qu'un même fait demande deux formulations (`PersonaProfile::Operator` vs `Family`) et que
dériver le registre d'un champ technique est un choix produit déguisé. Le moteur ne peut donc
pas rédiger l'explication ; il peut poser un fait minimal (D5) et refuser une clôture (D4).
Une ligne d'intention est ajoutée au prompt (§ Changements 5), explicitement comme moitié
d'intention et non comme garantie.

### D2 — Un canal typé sur `ToolOutput`, pas une relecture de la prose d'erreur

`ToolOutput` gagne `delivery: Option<DeliveryOutcome>`, sur le modèle de
`substrate_diagnostic` (E6) : posé par `send_message` seul, jamais rendu au LLM, `None`
partout ailleurs.

```rust
pub enum DeliveryOutcome {
    Delivered,
    RefusedTooLong { len_utf16: usize, limit: usize },
    Failed { reason: String },
    NoChannel,
    NoSender,
}
```

Raison. Les trois alternatives ont été écartées pour des raisons mesurables :

- **relire `output_summary`** — dépend du format d'un message d'erreur anglais, tronqué
  (E2), et fait d'une phrase destinée au modèle un format de fil ;
- **comparer les `input_summary`** — préfixe tronqué, faux « réparé » possible, et l'erreur
  penche du côté du silence (E8) ;
- **un ledger sur `ToolContext`** (le motif `pr_review_posted: &AtomicBool`) — il faudrait
  ajouter un champ à une structure dont **chaque** construction littérale de test énumère
  tous les champs. `ToolOutput` n'a que des constructeurs (E6) : même information, diff
  mécanique nul.

`NoChannel` et `NoSender` sont **dans l'enum mais hors du prédicat** (D3) : les énumérer
coûte deux variantes et rend la population de E3 comptable le jour où quelqu'un l'ouvrira ;
les omettre obligerait à rouvrir le type.

### D3 — Le prédicat est entièrement structurel : zéro lexique, deux étages

`evidence::guards::undelivered_sends(&[DeliveryRecord]) -> Option<UndeliveredSends>`,
fonction **pure** (le shape de ses voisines `detect_affirmative_state_claim` /
`assert_grounded_satisfied`), sur la séquence complète du tour :

```
étage transport : ∃ R avec outcome = Failed{..}
                  ET ∄ R' postérieur avec même texte ET outcome = Delivered
                  → ce contenu-là est perdu

étage refus     : ∃ R avec outcome = RefusedTooLong{..}
                  ET ∄ R' postérieur avec outcome = Delivered
                  → rien n'est parti du tout
```

Les deux étages ont des réparations différentes parce que leurs dégâts le sont (E2) :

- après un **échec transport**, la seule réparation vérifiable est le ré-essai du *même*
  texte (comparé sur le texte complet capté à la source — d'où D2, et non un préfixe) ;
- après un **refus de longueur**, la réparation est une découpe dont le moteur ne peut pas
  vérifier la couverture. Il peut en revanche constater le cas mesuré d'Al : **un refus suivi
  d'aucun envoi réussi**, c'est-à-dire un document dont rien n'est parti. C'est le prédicat
  retenu, et il s'éteint dès qu'un fragment part.

**Deux faux négatifs sont nommés, pas cachés**, et ils sont la même limite vue deux fois :
le moteur sait *qu'un* envoi est parti, jamais *que le contenu refusé* est parti.

- **Couverture partielle.** Un agent qui découpe en quatre, n'envoie que deux parties et dit
  « voilà tout » passe sous le prédicat.
- **Extinction par un envoi sans rapport.** Après un `RefusedTooLong` de 12 000 caractères,
  un unique « désolé, c'est trop long, je te le résume » de 80 caractères délivré avec succès
  éteint l'étage refus — alors que le document n'est toujours jamais parti. C'est la forme la
  plus probable en conditions réelles, et il faut la dire : **le prédicat retenu attrape le cas
  d'Al tel qu'il s'est produit** (refus, puis affirmation de livraison dans le texte de clôture,
  sans autre `send_message`), **pas toutes ses variantes**.

Vérifier la couverture demanderait de comparer la concaténation des fragments au texte refusé,
ce que la reformulation par l'agent (« Partie 1/4 », résumés, transitions) rend impossible par
construction. **Et un plancher de longueur est délibérément écarté** — exiger que les envois
réussis postérieurs totalisent une fraction de la longueur refusée fermerait le second cas,
mais le seuil serait arbitraire et sa direction d'erreur est la mauvaise : un utilisateur qui a
demandé un résumé recevrait une annexe affirmant une perte qui n'a pas eu lieu, c'est-à-dire
exactement ce que la halte 4 de la sonde interdit de laisser vivre. Le prédicat reste du côté
structurel ; l'étage transport, lui, n'a aucun de ces deux trous, puisque sa réparation est
l'égalité d'un texte avec lui-même.

`Delivered` n'est pas un terme du prédicat isolé : il n'y figure que comme **réparation**.
Un tour sans aucun échec ne fait entrer aucune donnée dans le prédicat — c'est ce qui rend
AC4 vrai par construction plutôt que par précaution (D7).

### D4 — La satisfaction du guard est un acte, jamais un aveu reconnu au lexique

Le guard `unacknowledged_send_failure` fire quand le prédicat de D3 est vrai, et **ne cherche
pas à lire un aveu dans le texte**.

Raison, et c'est le point le plus contre-intuitif du plan. Les guards 5c et 5d utilisent du
lexique bilingue, mais pour détecter une **violation** : ne pas reconnaître y coûte un faux
négatif (fail-open). Ici le lexique servirait à reconnaître une **satisfaction** : ne pas
reconnaître coûterait un re-prompt à chaque agent qui a bien dit la vérité dans des mots
inattendus, et reconnaître à tort laisserait passer exactement le cas du ticket. On refuse
donc d'inventer un détecteur d'honnêteté ; la seule satisfaction est structurelle — le
contenu est reparti, ou le budget de re-prompt est épuisé et D5 prend le relais.

Coût assumé : un tour LLM supplémentaire à chaque échec d'envoi non réparé, **y compris**
quand l'agent avait déjà bien expliqué. Ce coût ne touche pas le chemin heureux (AC4), il
touche le chemin de panne, où un tour de plus est le bon défaut.

Position : **6f**, dans la famille inline 6c/6d/6e, budget unique via `intent_guard_retries`
(label `UNACKNOWLEDGED_SEND_FAILURE_LABEL`), **non** court-circuité par
`skip_remaining_guards` — une revue de PR postée ne fait arriver aucun message à personne,
littéralement la même raison que 5c/5d/6c/6d. Miroir obligatoire sur la sortie texte-vide du
mode silencieux (`agent_loop/mod.rs:3108+`), comme 6(e)/6a/6b : un tour silencieux dont
l'envoi a échoué se clôt typiquement **sans texte**, ce qui est précisément la forme que la
panne prend là-bas.

### D5 — Le moteur écrit lui-même le fait quand le budget est épuisé

`AgentOutput.undelivered_sends: Option<UndeliveredSends>` (le champ frère de
`deadline_exceeded`, E5). Dans `handlers.rs`, **avant** `sender_arc.send(&response)`, une
ligne factuelle minimale est annexée au texte sortant quand le champ est `Some` :

> ⚠️ 1 message n'a pas pu être délivré (partie 1 : échec du transport). Rien n'a été reçu
> pour cette partie.

Raison. 5d, sur budget épuisé, se contente d'un WARN `..._uncorrected` et laisse passer.
Diverger ici doit se payer, et ça se paie : pour 5d le dégât d'un budget épuisé est une
phrase fausse de plus, visible dans le log ; ici le dégât est **un document qui n'est jamais
arrivé et un destinataire qui croit l'avoir reçu**. Et l'annexe n'est pas une correction du
modèle : c'est un fait que le moteur a mesuré, du même ordre que le verdict que
`deadline_verdict` poste lui-même quand le tour n'a pas conclu (E5).

**Le doublon est accepté, explicitement.** Un agent qui a correctement avoué recevra quand
même la ligne. C'est le prix du refus de D4 (ne pas prétendre reconnaître un aveu), et il
penche du bon côté : *un doublon est visible et corrigible, un silence ne l'est pas* — même
forme d'arbitrage que le gate destructif mika#1646. Le doublon est de surcroît confiné : le
champ n'est `Some` que si **rien** n'a réparé l'échec, donc un agent qui ré-essaie avec
succès ne voit jamais l'annexe.

**Limite assumée, mode silencieux.** Le texte d'un tour silencieux n'est délivré à personne :
l'annexe y serait une réparation de façade. Dans ce mode, le résidu est un WARN + une ligne
`audit_events`, dont le destinataire correct est l'opérateur — un échec de `send_message` en
mode silencieux signifie justement qu'il n'existe aucun canal vers l'utilisateur pour le lui
dire.

### D6 — La reformulation du `ToolOutput` est faite, et nommée pour ce qu'elle vaut

Les deux messages d'erreur gagnent une phrase sans ambiguïté (« NOTHING WAS SENT — the user
has received nothing », « this message was NOT delivered; if you continue with other parts,
say which part failed »). C'est la piste que le ticket cite en deuxième, et elle est bon
marché. Mais elle est de la même nature que l'information dont l'agent disposait déjà le
2026-09-01 : une amélioration du signal, **pas** une garantie. Elle ne porte aucun AC à elle
seule et le plan ne la compte pas comme telle.

### D7 — Le chemin heureux ne paie rien, et c'est vérifié par un test, pas par une intention

Le prédicat de D3 s'éteint sur un `Vec<DeliveryRecord>` ne contenant aucun `RefusedTooLong`
ni `Failed` — aucune lecture de texte, aucun appel LLM, aucun tour supplémentaire. Le vecteur
est vide pour tout tour sans `send_message`.

**Le seul coût du chemin heureux est nommé plutôt qu'affirmé absent** : `DeliveryRecord.text`
clone le texte de *chaque* envoi, y compris réussi, parce que l'étage transport compare un
texte mort au texte d'une réparation **postérieure** et ne peut donc pas savoir à l'avance
lequel il faudra. La borne est dure et petite : `MAX_TOOL_STEPS = 20` × la limite de 4096
unités UTF-16, soit ~80 Ko par tour au pire, alloués dans le tour et libérés avec lui, sans
persistance ni sérialisation. Un hash à la place du texte supprimerait ce clone mais rendrait
une collision indistinguable d'une réparation — donc un silence sur un fragment mort, c'est-à-dire
le défaut d'origine. Le clone est payé sciemment.

AC4 porte sur le **comportement observable**, et exige de le montrer : un test d'intégration
assertant qu'une séquence de quatre fragments tous livrés produit exactement un appel LLM de
clôture, un texte final inchangé octet pour octet, et `undelivered_sends == None`.

### D8 — Le vecteur voyage en `&mut`, pas dans les trois variantes de `LoopResult`

`run_loop` reçoit `delivery_log: &mut Vec<DeliveryRecord>`, le passe à `process_tool_calls`
(qui a déjà exactement ce motif pour `send_message_boundary_active: &mut bool` et
`send_message_text_capture: &mut String`, `dispatch.rs:347-360`), et les trois appelants le
créent puis le lisent après le retour.

Raison. `LoopResult` a trois variantes portant chacune un `Vec<ToolCallSummary>` ; y ajouter
un champ ferait trois écritures à tenir synchrones. Le `&mut` donne en outre gratuitement la
couverture des sorties `MaxStepsExceeded` et `DeadlineExceeded` — un tour coupé par son
enveloppe après un envoi mort doit annexer le fait tout autant qu'un tour qui conclut.
Précédent de threading à trois sites : `loaded_skill_names` (mika#2355).

---

## Changements

### 1. `crates/mika-agent/src/tools/mod.rs`

- `ToolOutput.delivery: Option<DeliveryOutcome>` + `DeliveryOutcome` (D2). `None` dans les
  constructeurs existants (`success`, `error`, `success_with_images`,
  `substrate_unavailable`, …) ; nouveau constructeur `ToolOutput::delivery(content, is_error, outcome)`.
- Doc de champ sur le modèle de `substrate_diagnostic` : jamais sérialisé vers le LLM, canal
  outil→moteur.

### 2. `crates/mika-agent/src/tools/send_message.rs`

- Les cinq sorties posent leur `DeliveryOutcome` (y compris `Delivered`, `NoChannel`,
  `NoSender` — D2).
- Reformulation des deux messages d'erreur (D6). La garde de longueur et son test de borne
  (`accepte_4096_a_la_borne`, `fenetre_5000_refusee_par_l_outil`) sont **inchangés** :
  mika#2134 est en amont et hors périmètre.
- Tests : chaque sortie pose l'outcome attendu ; le contrôle négatif 4095/4096 reste vert.

### 3. `crates/mika-agent/src/tool_execution/dispatch.rs`

- Nouveau paramètre `delivery_log: &mut Vec<DeliveryRecord>` (D8), à côté de
  `send_message_boundary_active`. Quand `output.delivery` est `Some`, pousse
  `DeliveryRecord { step, text: String, outcome }` — **texte complet**, capté à la source
  avant toute troncature (E8). Le champ ne quitte jamais le tour et n'est pas persisté.
- Interaction avec le dedup per-tour (#582) : un duplicata réutilise le `ToolOutput` caché,
  donc **un seul** `DeliveryRecord` par appel dédupliqué — ce qui est correct (l'outil n'a
  tourné qu'une fois). Test dédié.

### 4. `crates/mika-agent/src/evidence/guards.rs`

- `DeliveryRecord`, `UndeliveredSends`, `undelivered_sends(&[DeliveryRecord]) -> Option<UndeliveredSends>` (D3),
  fonction pure.
- `UndeliveredSends` porte de quoi rédiger le re-prompt et l'annexe sans relire le texte :
  nombre, index de l'envoi dans la séquence, étage (`RefusedTooLong` / `Failed`), raison,
  préfixe court (80 car.) du contenu perdu.
- Tests unitaires (AC1/AC3/AC4) : séquence vide ; tout livré ; un fragment mort au milieu de
  trois réussis ; un fragment mort **suivi** de son ré-essai réussi (→ `None`) ; refus de
  longueur suivi de quatre fragments réussis (→ `None`) ; refus de longueur suivi de rien
  (→ `Some`) ; deux fragments morts (comptage) ; texte identique envoyé deux fois dont un
  seul réussit ; **refus de longueur suivi d'un unique envoi court sans rapport** (→ `None`) —
  ce dernier **épingle le faux négatif de D3 au lieu de le laisser dériver** : le jour où
  quelqu'un voudra le fermer, ce test rougit et le nomme, plutôt que de laisser croire que le
  prédicat couvrait déjà le cas.

### 5. `crates/mika-agent/src/agent_loop/mod.rs`

- `run_loop` : paramètre `delivery_log: &mut Vec<DeliveryRecord>`, threadé aux trois sites
  d'appel (conversation, silent, team) (D8).
- Guard **6f** `unacknowledged_send_failure` après 6e : détection par `undelivered_sends`,
  budget unique `intent_guard_retries`, `guard_correlation_id` + WARN
  `event = "guard.unacknowledged_send_failure"` sur `target: "mika::otel"` (famille #953),
  re-prompt nommant l'étage, l'index du fragment et son préfixe. **Non** court-circuité par
  `skip_remaining_guards`.
- Miroir sur la sortie texte-vide du mode silencieux (D4).
- Résidu : `guard.unacknowledged_send_failure_uncorrected` (WARN) quand le budget est épuisé,
  sur le modèle de 5d.
- `AgentOutput.undelivered_sends: Option<UndeliveredSends>` (D5), renseigné sur les trois
  sorties, doc rédigée sur le modèle de `deadline_exceeded`.
- Une ligne d'intention dans le prompt reste **hors** de ce fichier (§ 7).

### 6. `crates/mika-agent/src/server/handlers.rs`

- Annexe de la ligne factuelle avant `sender_arc.send(&response)` (`:1541`), dans le même
  voisinage que `post_deadline_verdict_if_cut_off` (`:1537`) (D5, E5). Deux registres
  (`PersonaProfile::Operator` / `Family`) par `match` exhaustif sans `_ =>`, modèle mika#2290.
- **Chemin d'accès de la persona, à ne pas chercher :** `handlers.rs` n'importe aujourd'hui
  aucun `PersonaProfile` (`grep` : zéro occurrence), mais il porte déjà le tier — `a.tier` et
  `a.deployment` sont lus à `:1493` pour construire le contexte de prompt. Le registre est donc
  `a.tier.persona_profile()` (`mika_common::home`, `:137`), le même convertisseur que
  `prompt.rs`, sans nouveau champ sur `AgentState` ni nouvelle dérivation.
- Cas `output.text == None` : l'annexe remplace `EMPTY_RESPONSE_FALLBACK` plutôt que de s'y
  ajouter — un tour muet dont l'envoi a échoué doit dire l'échec, pas « je n'ai rien à dire ».
- INFO `send_failure_annexed` + ligne `audit_events` (`tool_name = 'undelivered_send_annexed'`,
  **SOLE WRITER**).

### 7. `crates/mika-agent/src/prompt.rs`

- Extension d'une phrase à la **Grounding rule** (`:1310`) : un `tool_result` d'envoi en
  erreur signifie que *rien n'est arrivé* ; ne jamais annoncer une livraison qu'un outil a
  refusée, et nommer le fragment échoué avant de passer au suivant. Moitié d'intention,
  déclarée comme telle (D1). Carve-out `build_compact_system_prompt` (≤5 Ko) inchangé — comme
  pour 5d, la garde lit le texte sortant, pas le prompt.

### 8. `crates/mika-agent/tests/eval/test_undelivered_send_2136.rs` (nouveau)

Enregistré dans `tests/eval/mod.rs` (gardé par `test_eval_modules_declared.rs`). Quatre
scénarios sur `run_agent` via `EvalHarness` + `MockLlmProvider` + un `MessageSender` scripté
(E7) :

- **AC6-a** — rejeu du 2026-09-01, étage 1 : envoi de 12 000 caractères refusé, puis réponse
  scriptée « Le voici en entier 👆 » ⇒ le tour est refusé une fois ; sur la seconde clôture
  non réparée, `undelivered_sends` est `Some` et le texte délivré porte le fait. **Le scénario
  ne contient aucun autre `send_message`, et c'est fidèle au cas mesuré, pas une commodité** :
  Al n'a rien reçu du tout. Un scénario qui glisserait un message d'excuse délivré entre le
  refus et la clôture passerait sous le faux négatif nommé en D3 — le test le dit en commentaire
  pour que personne ne le « répare » en le rendant vert par accident.
- **AC6-b** — rejeu du 2026-09-01, étage 2 : quatre fragments, le premier échoue au
  transport, réponse scriptée qui enchaîne sur « Partie 2/4 » sans le dire ⇒ refus, puis
  annexe nommant la partie 1.
- **AC3** — l'échec du premier fragment n'est pas masqué par la réussite des trois suivants
  (assertion sur la séquence, pas sur un envoi isolé).
- **AC4** — contrôle négatif : quatre fragments tous livrés ⇒ `undelivered_sends == None`,
  aucun appel LLM supplémentaire (comptage sur le `MockLlmProvider`), texte final identique
  octet pour octet à la réponse scriptée, zéro occurrence du vocabulaire d'échec.

### 9. `crates/mika-agent/CLAUDE.md` + `mika/CLAUDE.md`

Guard 6f dans la liste des post-conditions ; `undelivered_sends` dans la doc d'`AgentOutput` ;
les trois signaux opérateur ; les **deux** faux négatifs de D3 écrits là où on les cherchera —
en particulier que l'absence d'annexe ne prouve pas qu'un document est arrivé.

---

## Surfaces opérateur

Journal (`$MIKA_SPIRIT_LOG_FILE`) :

- `guard.unacknowledged_send_failure` (WARN, famille #953 — champs `stage`, `failed_index`,
  `failed_count`, `guard_correlation_id`, joignable à `guard.correction_accepted`).
  **Régime attendu : non nul mais faible.** Chaque ligne est un envoi mort que l'agent
  s'apprêtait à taire. Un flot soutenu ne se traite pas en élargissant la garde : il dit que
  le gateway ou Telegram refuse, et c'est ça qu'il faut traiter.
- `guard.unacknowledged_send_failure_uncorrected` (WARN) — le budget est épuisé, D5 a pris le
  relais. **Régime attendu : proche de zéro.** Sustained sur un même agent = le modèle
  n'utilise pas le re-prompt, et c'est la moitié D4 qu'il faut interroger, pas D5.
- `send_failure_annexed` (INFO) — le moteur a écrit le fait lui-même.

SQL :

```sql
SELECT count(*) FROM audit_events WHERE tool_name = 'undelivered_send_annexed';
```

**SOLE WRITER** : `handlers.rs` est le seul site écrivant ce `tool_name`. Son **absence**
sous une plainte utilisateur est donc une information — elle dit que l'envoi n'a pas échoué
là où on le croit, et renvoie vers `NoChannel` (E3) ou vers le gateway.

---

## Sonde post-déploiement, et sa halte

Sur 7 jours :

1. `grep guard.unacknowledged_send_failure $MIKA_SPIRIT_LOG_FILE | jq '{stage, failed_count}'` —
   la distribution par étage dit laquelle des deux moitiés porte le trafic réel. Si l'étage
   `RefusedTooLong` domine largement, la garde de longueur de mika#2134 est franchie
   quotidiennement et c'est **son** seuil qu'il faut réinterroger, pas cette garde-ci.
2. `SELECT count(*) FROM audit_events WHERE tool_name = 'undelivered_send_annexed';` — chaque
   ligne est une annexe lue par un utilisateur.
3. **Halte.** Si un utilisateur signale à nouveau un document annoncé et jamais reçu alors que
   les trois greps sont **vides**, ne pas élargir le prédicat : l'envoi a réussi du point de
   vue du moteur et la perte est en aval (gateway, Telegram, mika#2126 côté rendu) ou dans la
   population `NoChannel` de E3. Établir lequel vient d'abord.
4. **Halte (faux positif).** Si l'annexe apparaît sur un tour où tout est bien arrivé, désarmer
   D5 et réparer le prédicat de D3 — une annexe fausse dans le canal utilisateur est un dégât
   du même ordre que celui qu'on répare, et ne se rattrape pas par un seuil.

---

## Hors périmètre, délibérément

- **La fenêtre 4096 / 10 000** — mika#2134, en amont et livrée. Ce plan ne touche ni la borne,
  ni sa mesure en unités UTF-16, ni ses tests.
- **`SendOutcome::NoChannel` et l'absence de sender** (E3) — vraie population de « livraison
  affirmée sans livraison », mais les rendre `is_error` rouvrirait la boucle de ré-essai que
  #650 a fermée sur une condition permanente, et la garde de régression mika#1090 l'interdit
  nommément. Les deux variantes sont **dans** l'enum de D2 pour que cette population soit
  comptable le jour où elle aura son ticket ; elles ne sont pas dans le prédicat de D3.
- **La couverture d'une découpe et l'extinction par un envoi sans rapport** (les deux faux
  négatifs nommés en D3) — demandent une comparaison sémantique entre un texte refusé et des
  fragments reformulés, ou un seuil de longueur arbitraire dont l'erreur pencherait du côté de
  l'annexe fausse. Frontière assumée, épinglée par un test unitaire.
- **La taxonomie de `google-workspace`** — mika#2118. **Le markdown Telegram** — mika#2126.
- **`failed_sends` et son flush** — le mécanisme de reprise existe et n'est pas touché ; ce
  plan porte sur ce que l'agent *dit* du tour en cours, pas sur la re-livraison différée.
- **Une refonte de la véracité de l'agent** — le ticket l'exclut, ce plan aussi.

---

## Definition of Done

- `ToolOutput.delivery` posé par les cinq sorties de `send_message`, `None` partout ailleurs,
  jamais sérialisé vers le LLM.
- `undelivered_sends` est une fonction pure, couverte par les neuf cas unitaires listés en § 4,
  dont celui qui épingle le faux négatif de D3 plutôt que de le laisser dériver.
- Guard 6f en place avec son miroir texte-vide, son budget unique et sa télémétrie #953.
- `AgentOutput.undelivered_sends` renseigné sur les trois sorties, y compris
  `DeadlineExceeded` et `MaxStepsExceeded`.
- Annexe posée dans `handlers.rs` avant l'envoi, deux registres par `match` exhaustif.
- `test_undelivered_send_2136.rs` : quatre scénarios verts, dont le contrôle négatif AC4
  assertant l'absence de tour supplémentaire.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --all -- --check` verts.
- `crates/mika-agent/CLAUDE.md` et `mika/CLAUDE.md` à jour ; `docs-sync` vert si `docs/` bouge.
- Aucun changement de comportement sur un tour sans `send_message` en échec — attesté par AC4,
  pas affirmé.

---

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2136.

- **AC1** — Quand un `send_message` rend `is_error == true`, l'état du tour porte cet échec de
  façon **lisible par le moteur**, pas seulement par le modèle. Test unitaire.
- **AC2** — Un tour ne peut pas se clore sur un envoi échoué **non acquitté** sans que le
  moteur l'ait constaté. La forme de la contrainte est à trancher au grooming ; la décision et
  sa raison sont écrites.
- **AC3** — Sur une séquence multi-fragments dont un fragment échoue, l'échec est signalé et
  n'est pas masqué par la réussite des suivants. Test sur la séquence, pas sur un envoi isolé.
- **AC4** — **Contrôle négatif** : une séquence dont tous les fragments passent ne produit
  **aucune** mention d'échec et aucun tour supplémentaire. Un correctif qui alourdirait le
  chemin heureux n'aurait pas réparé, il aurait taxé.
- **AC5** — La décision « structure vs prompt » est prise et écrite, avec sa raison. Si une
  part reste au prompt, dire laquelle et pourquoi elle ne peut pas être structurelle.
- **AC6** — Preuve de non-vacuité : rejouer la séquence du 2026-09-01 — un envoi refusé pour
  longueur, puis une découpe dont le premier fragment échoue — et montrer que ni l'affirmation
  de livraison ni le silence sur le fragment ne sont plus possibles.

Correspondance : AC1 → D2 + § 4 (tests unitaires du prédicat) ; AC2 → D4 (guard 6f) + D5
(`AgentOutput.undelivered_sends`, le constat porté au-delà du tour) ; AC3 → D3 (prédicat sur
la séquence) + § 8 scénario AC3 ; AC4 → D7 + § 8 scénario AC4 ; AC5 → D1 ; AC6 → § 8 scénarios
AC6-a et AC6-b.

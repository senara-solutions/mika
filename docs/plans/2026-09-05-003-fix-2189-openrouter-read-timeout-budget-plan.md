# Plan : le plafond par appel est plus grand que le budget qui doit le contenir (mika#2189)

**Ticket :** mika issue#2189 — `fix(llm,arch): mika-arch expire une passe sur deux sur le transport openrouter (kimi-k2.5) — passe architecte à pile ou face, groomings 1-2h (famille de #2179)`
**Labels :** `bug`
**Type :** issue (bug — ralentit la boucle)
**Palier de priorité :** Tier 2 — *ralentit la boucle*. Confirmé par la mesure : 209 appels perdus en 7 jours sur trois agents, chacun ayant brûlé 120 s ou 240 s avant d'échouer.
**Fichiers principaux :** `crates/mika-common/src/llm/mod.rs`, `crates/mika-common/src/llm/openai.rs`, `crates/mika-agent/src/planning/policy.rs`, `crates/mika-agent/src/agent_loop/mod.rs`, `crates/mika-common/src/config.rs`, `crates/mika-agent/tests/eval/` (nouveau)

---

## Problème

Le ticket décrit une passe architecte à pile ou face. La mesure dit *pourquoi*, et elle
dit quelque chose de plus large que ce que le titre annonce.

### M1 — toutes les pannes tombent sur le même couperet, à la seconde près

Sur `llm_calls`, 7 jours glissants (arrêté au 2026-09-05), **209 échecs** portent un seul
et même texte :

```
LLM transport error: failed to read response body: error decoding response body:
request or response body error: operation timed out
```

Leur distribution de latence n'a pas de queue — elle a **deux valeurs** :

| latence de l'appel perdu | occurrences |
|---|---|
| 240 s | 171 |
| 120 s | 37 |
| 243 s | 1 |

`DEFAULT_HTTP_TIMEOUT_SECS = 120` (`llm/mod.rs:43`), posé sur `reqwest` en
`.timeout(...)` (`llm/openai.rs:170`). `reqwest::ClientBuilder::timeout` borne la requête
**entière, lecture du corps comprise** — d'où le texte : la réponse *était en train
d'arriver* et le client est parti. Il n'y a pas de variance fournisseur dans ces chiffres :
il y a un couperet client, franchi une fois (120 s) ou deux (240 s).

Le ticket cite `LLM transport error: operation timed out`. C'est la fin du message réel.
Le début — `failed to read response body` — est la partie qui porte le diagnostic, et le
plan la reprend en entier.

### M2 — ce n'est pas la face mika-arch d'un problème : mika-arch en est 7 %

| agent | modèle | appels 7 j | erreurs | taux |
|---|---|---|---|---|
| `mika-dev` | `z-ai/glm-5.3` | 8 873 | 192 | 2,2 % |
| `mika-arch` | `moonshotai/kimi-k2.5` | 424 | 14 | 3,3 % |
| `mika-qa` | `z-ai/glm-5.2` | 5 394 | 3 | 0,1 % |

Trois agents, **deux modèles distincts**, un seul fournisseur (openrouter), un seul
message d'erreur. Un repli de modèle pour mika-arch laisserait 195 des 209 pannes en
place. La cause n'est ni le modèle ni l'agent : c'est le chemin partagé
`llm/openai.rs`.

### M3 — le rejeu ne peut pas réussir, par construction

`MAX_RETRIES = 3` (`llm/openai.rs:142`) autorise quatre tentatives. La mesure n'en montre
jamais plus de deux, et l'avortement est écrit dans le code : à partir de la deuxième
tentative, la boucle exige `TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` (`llm/mod.rs:39`) de
budget restant avant d'en tenter une autre (`llm/openai.rs:344-366`). Avec
`AGENT_TOTAL_TIMEOUT_SECS = 300` (`planning/policy.rs:18`), deux tentatives à 120 s
laissent 60 s — juste sous le seuil. D'où la signature à 240 s, 171 fois sur 209.

Mais même sans cet avortement, le rejeu serait vain : **un appel qui a besoin de 150 s
n'aboutira pas davantage à la troisième tentative de 120 s.** Le remède « rejeu transport
borné » proposé en AC2 est déjà présent dans le code et la mesure le montre inopérant sur
cette classe. Ce plan le rejette sur preuve, pas sur avis.

### M4 — le vrai serrage : le budget d'agent ne contient pas le travail de mika-arch

Latences des appels mika-arch **réussis** (410 appels, 7 j) :

| min | p50 | p90 | p99 | max |
|---|---|---|---|---|
| 1,7 s | 20,3 s | **98,7 s** | **191,3 s** | **233,1 s** |

Une passe mika-arch consomme **3,1 appels** en moyenne (424 appels / 139 sessions). Le
temps LLM cumulé par session, contre le budget de 300 s :

| temps LLM cumulé | sessions |
|---|---|
| < 60 s | 48 |
| 60–120 s | 32 |
| 120–200 s | 22 |
| 200–300 s | 30 |
| ≥ 300 s | 7 |

**37 sessions sur 139 (27 %) brûlent 200 s ou plus de temps LLM pur** dans une enveloppe
de 300 s qui doit aussi contenir les appels d'outils. Sept la dépassent franchement.

C'est l'asymétrie structurelle du ticket : `MIKA_LLM_HTTP_TIMEOUT_SECS` existe depuis
mika#1660 (`llm/mod.rs:46`) et permet de relever le plafond **par appel** ;
`AGENT_TOTAL_TIMEOUT_SECS` est une constante nue (`planning/policy.rs:18`), sans variable
d'environnement, sans réglage par agent, lue en deux points
(`agent_loop/mod.rs:3146` et `:3985`). **On peut relever le plafond, pas le budget qui doit
le contenir.** Relever le seul plafond à 300 s ferait qu'un appel unique consomme
l'enveloppe entière d'une passe qui en demande trois.

### M5 — la date, et ce qu'elle exclut

mika-arch : **zéro erreur du 08-27 au 09-02** (154 appels, 47 sessions), puis 3 le 09-03,
5 le 09-04, 6 le 09-05. Le prompt système de mika-arch est passé à 59 805 octets le
2026-09-01, contre 54–55 Ko la semaine précédente ; **les 14 erreurs sont toutes sur cette
variante** (307 appels, 4,6 %), zéro sur les autres (117 appels). La flotte, elle, saigne
depuis au moins le 08-27 (60 pannes ce jour-là, sur `mika-dev`).

Lecture : le couperet à 120 s est ancien ; mika-arch vient seulement de franchir la ligne,
parce que son prompt a grossi de ~10 Ko et a poussé sa queue de latence au-dessus. Cela
**invalide** l'hypothèse « incident fournisseur du 09-03 » et confirme M1 : le seuil est
fixe, c'est la distribution qui a glissé dessous.

### M6 — un axe de l'AC1 n'est pas mesurable en l'état

L'AC1 demande la distribution « par taille de brief ». Sur le chemin d'erreur,
`agent_loop/mod.rs:1093-1112` écrit `input_tokens = 0` et `output_tokens = 0` en dur :
**les lignes en échec ne portent aucun compte de jetons.** La taille du message
utilisateur n'est donc pas récupérable a posteriori sur les pannes.

Ce qui *est* disponible sur ces lignes : `system_prompt_bytes` (mika#1217), et il
discrimine proprement — voir M5. Le plan livre l'axe demandé avec ce porteur, et corrige
le trou pour que la prochaine mesure n'ait pas à s'en contenter.

### Rapport à mika#2179

mika#2179 est **fermée** (PR #2190 fusionnée). Son plan met explicitement hors portée
« la **cause** des timeouts côté fournisseur (openrouter / `z-ai/glm-5.3`, taille des
requêtes, proxy d'egress). Ce plan rend la panne visible et bornée ; il ne la fait pas
disparaître. » Ce ticket-ci prend ce que #2179 a laissé. Aucun recouvrement : #2179 a
instrumenté et borné la *livraison des callbacks* ; #2189 traite le *couperet qui produit
les pannes*.

---

## Décision (AC2) — sur la mesure, pas à l'aveugle

Les trois remèdes proposés par le ticket, arbitrés par M1–M4 :

| candidat | verdict | preuve |
|---|---|---|
| Rejeu transport borné côté client | **rejeté** | Déjà présent (`MAX_RETRIES = 3`). M3 : la boucle s'arrête à 2 tentatives par avortement de deadline, et un appel de 150 s ne réussit pas en 120 s quel qu'en soit le rang. |
| Repli de modèle/fournisseur pour mika-arch | **rejeté** | M2 : 195 des 209 pannes sont hors mika-arch, sur un autre modèle. Le repli déplacerait 7 % du problème. |
| Plafond par appel relevé | **retenu, mais insuffisant seul** | M1 : le couperet est bien la cause unique. M4 : le relever sans toucher `AGENT_TOTAL_TIMEOUT_SECS` fait qu'un appel avale l'enveloppe d'une passe de trois. |

**Remède retenu : rendre les deux budgets réglables ensemble, par agent, sous un
invariant vérifié.** Le plafond par appel et l'enveloppe d'agent cessent d'être deux
constantes indépendantes dont l'une seule a un bouton.

---

## Périmètre

### D1 — `AGENT_TOTAL_TIMEOUT_SECS` devient réglable

Introduire `agent_total_timeout_secs` dans `Settings` (`config.rs`), avec
`DEFAULT_AGENT_TOTAL_TIMEOUT_SECS = 300` (valeur actuelle inchangée), surcharge
`MIKA_AGENT_TOTAL_TIMEOUT_SECS`, et un accesseur `effective_agent_total_timeout_secs()`
suivant la forme déjà établie par `effective_callback_delivery_max_attempts()`
(`config.rs:1690`).

Les deux points de lecture (`agent_loop/mod.rs:3146`, `:3985`) passent par l'accesseur.
La constante `TEAM_AGENT_TIMEOUT_SECS` (`policy.rs:29`), documentée comme « matches
AGENT_TOTAL_TIMEOUT_SECS », suit le même réglage — sinon les deux dérivent au premier
changement de valeur.

### D2 — l'invariant de contenance, vérifié au démarrage

`http_timeout_secs()` panique déjà sur une valeur sous `MIN_HTTP_TIMEOUT_SECS`
(`llm/mod.rs:82-88`). Ajouter une vérification symétrique, au même endroit du cycle de vie
(construction du fournisseur, chemin froid) :

```
http_timeout_secs() < agent_total_timeout_secs
```

Une configuration où le plafond d'un appel atteint ou dépasse l'enveloppe de l'agent est
une erreur de réglage : le premier appel consomme tout le budget et la boucle d'agent
n'a plus de quoi faire un second pas. Elle échoue fort et tôt, comme mika#1660 l'a établi
pour le plafond seul.

**Cet invariant est le cœur du ticket.** Sans lui, D1 rend simplement possible une
seconde façon de se tromper.

### D3 — les seuils d'avortement dérivent du plafond, ils ne le devinent plus

`TYPICAL_CALL_DURATION_SECS = 90` et `RETRY_BUFFER_SECS = 30` (`llm/mod.rs:24,26`) sont
calibrés en dur sur « Sonnet 4.6 observé p95 (49 s) », et
`TRANSPORT_RETRY_MIN_REMAINING_SECS = 60` (`:39`) sur un plafond de 120 s. Si D1 permet de
relever les budgets, ces trois seuils mentent dès le premier réglage non par défaut : la
boucle de `llm/openai.rs:344-366` avorterait, ou n'avorterait pas, sur une géométrie qui
n'existe plus.

Les rendre dérivés du plafond effectif plutôt que littéraux, **en fractions** (Q1 tranchée) :
les valeurs actuelles sont exactement `0,75 × plafond`, `0,25 × plafond` et `0,50 × plafond`
au plafond par défaut de 120 s, donc la migration est un no-op au défaut et suit tout
réglage ultérieur.

### D4 — valeurs par agent, et le réglage de mika-arch

`llm_provider` et `openrouter_model` vivent déjà dans `~/.mika/agents/<nom>/config.toml`.
Les deux nouveaux budgets s'y logent au même niveau, ce qui permet de laisser la flotte à
300 s / 120 s et de donner à mika-arch une enveloppe qui contient sa distribution mesurée.

Valeurs proposées pour mika-arch, dérivées de M4 (p99 = 191 s, max = 233 s, 3,1 appels par
passe) : plafond par appel **240 s**, enveloppe d'agent **900 s**. Elles couvrent le max
observé avec de la marge et laissent trois appels dans l'enveloppe. Elles sont proposées,
pas gravées : l'architecte peut les corriger, et elles restent des données de
configuration, pas du code.

### D5 — le trou de mesure de M6

Sur le chemin d'erreur de `agent_loop/mod.rs:1093-1112`, écrire la taille de la requête
plutôt que deux zéros. Le porteur minimal est `system_prompt_bytes`, déjà passé
(`Some(system_prompt_len as i64)`) — il est correct. Ce qui manque est la taille du
message utilisateur. **Q2 tranchée : colonne `request_bytes` sur `llm_calls`**, migration sur
la forme de `system_prompt_bytes` (mika#1217, `db.rs:4342-4364`), écrite sur les deux chemins
(succès et erreur) de `agent_loop/mod.rs:1052-1112`. L'estimation d'`input_tokens` est
écartée : l'axe sert à corréler taille et expiration, et une corrélation bâtie sur une
estimation ne tranche pas.

Sans D5, l'AC1 est livrée avec l'axe « par taille de brief » porté par le seul prompt
système, et la limitation est écrite noir sur blanc dans le rapport.

---

## Hors portée

- **Le proxy d'egress et le routage openrouter.** M1 montre un couperet client ; rien
  dans la mesure n'incrimine le chemin réseau. Si un remède côté transport devient
  nécessaire après D1–D3, il fera l'objet d'un ticket appuyé sur une mesure neuve.
- **`claude.rs:382`**, qui pose `.timeout(Duration::from_secs(120))` en littéral au lieu
  d'appeler `http_timeout_secs()`. C'est une incohérence réelle — le rail Anthropic
  ignore le bouton de mika#1660 — mais elle est indépendante de cette panne (aucune des
  209 erreurs n'est sur ce rail). **Ticket séparé**, per `feedback_implementation_scope_bundling`.
- **La croissance du prompt système de mika-arch** (54 Ko → 59,8 Ko le 2026-09-01), qui est
  la cause proximale du franchissement de ligne pour cet agent. La réduire est un travail
  distinct ; ce plan fait tenir la géométrie, il ne rétrécit pas le prompt.
- **`mika-dev` / `mika-qa` en tant que cibles de réglage.** D1–D3 les rendent réglables ;
  choisir *leurs* valeurs demande la même mesure par agent, et se fait après que le
  mécanisme existe.

---

## Vérification

### V1 — rouge avant, terme par terme

L'invariant D2 se vérifie sur un contrôle positif **et** négatif dans le même test
(`feedback_a_probe_needs_both_controls_in_the_same_call`) : une configuration
`http=120, agent=300` passe ; une configuration `http=300, agent=300` échoue au
démarrage avec un message qui nomme les deux valeurs.

D1 se vérifie par un test qui lit l'enveloppe effective sous surcharge d'environnement et
sous valeur par défaut, et par la constatation que les deux points d'appel
(`agent_loop/mod.rs:3146`, `:3985`) ne référencent plus la constante — grep de tous les
appelants, per `feedback_structural_gate_audit_grep_all_callsites`.

D3 se vérifie sur la boucle de rejeu : avec un plafond relevé, l'avortement doit se
déclencher au bon moment, ce qui se teste sur la fonction de dérivation seule, sans appel
réseau.

### V2 — la non-régression de l'AC3, énoncée correctement

Un plafond est un plafond : le relever **ne peut pas** ralentir un appel qui réussissait
déjà. La régression réelle est ailleurs, et c'est elle qu'il faut borner : **un appel qui
échoue devient plus lent.** Avec un plafond à 240 s et deux tentatives, une panne coûte
480 s au lieu de 240 s.

L'AC3 se vérifie donc en deux temps :
1. Les latences des appels **réussis** sont inchangées — mesure d'après-correctif sur
   `llm_calls`, comparée à la ligne de base p50/p90/p99 de M4.
2. Le coût du **pire cas d'échec** est borné et nommé : `plafond × tentatives ≤ enveloppe`,
   ce qui est exactement l'invariant D2 étendu au rejeu. Une panne ne peut pas déborder de
   l'enveloppe de l'agent.

Le second point est ce qui fait de l'AC3 une contrainte de conception plutôt qu'une
observation d'après-coup.

### V3 — mesure d'après-correctif, sur la même requête

La mesure de M1–M5 est rejouable telle quelle sur `llm_calls` (`~/.mika/data/mika.db`).
Après déploiement — et **après avoir laissé passer au moins une période complète du
composant le plus lent**, per `feedback_never_conclude_inside_the_mechanism_period` : au
moins 24 h et 100 appels mika-arch — rejouer le comptage par agent et par latence. Le
critère est que les valeurs 120 s / 240 s disparaissent de la colonne de latence des
erreurs. Si elles réapparaissent à 240 s / 480 s, le plafond a bougé mais la distribution
aussi, et le diagnostic est à refaire, pas le réglage.

**`llm_calls` a une rétention.** `prune_old_llm_calls` (`db.rs:9489`) purge les lignes
anciennes ; capturer la ligne de base M1–M5 dans le rapport d'AC1 **avant** de déployer,
pour que la comparaison d'après ne dépende pas de lignes qui auront été purgées.

---

## Décisions tranchées (Q1–Q5)

Cinq points de jugement étaient ouverts à la rédaction. Ils ont été soumis à mika-arch en
première passe (session `8ab15e86-6b91-4776-86cd-51a942eab786`) et sont **tranchés ici** :
le plan ne part pas en implémentation avec des questions ouvertes.

**Q1 — forme de la dérivation en D3 → fractions du plafond effectif.**
`TYPICAL_CALL_DURATION_SECS`, `RETRY_BUFFER_SECS` et `TRANSPORT_RETRY_MIN_REMAINING_SECS`
deviennent des fractions du plafond effectif plutôt que des littéraux. Rapport aux valeurs
actuelles, qui fixent la calibration : à plafond 120 s, `typique = 90` = **0,75 × plafond**,
`tampon = 30` = **0,25 × plafond**, `min-restant transport = 60` = **0,50 × plafond**. Les
fractions reproduisent donc exactement le comportement d'aujourd'hui au plafond par défaut —
la migration est un no-op mesurable (voir AC3-b) — et suivent tout réglage ultérieur.
*Raison retenue :* la forme alternative (constantes + contrôle de cohérence qui panique)
oblige à corriger deux endroits à chaque réglage, et le second oubli est silencieux jusqu'à
la panne suivante.

**Q2 — porteur de la taille de requête en D5 → colonne `request_bytes` sur `llm_calls`.**
Migration de schéma, sur la forme des migrations existantes (`system_prompt_bytes`,
mika#1217, `db.rs:4342-4364`). L'estimation d'`input_tokens` avant l'appel est écartée :
l'axe « par taille de brief » de l'AC1 sert à *corréler taille et expiration*, et une
corrélation bâtie sur une estimation ne peut pas trancher. Écrite sur les deux chemins
(succès et erreur) de `agent_loop/mod.rs:1052-1112`, sinon le trou de M6 se rouvre côté
erreur.

**Q3 — portée de D4 → budgets par agent dans `config.toml`, sans défaut par famille.**
La granularité agent est celle du problème : `llm_provider` et `openrouter_model` y vivent
déjà. Le risque nommé — un nouvel agent lent découvre le problème par une panne — est
**accepté et rendu visible** plutôt que résolu par un défaut par famille : le message de
l'invariant D2 nomme les deux valeurs et le fichier à corriger, et la mesure de V3 reste
rejouable par agent.

**Q4 — périmètre → un seul ticket ; le chemin partagé et le réglage mika-arch ne se scindent pas.**
M2 est la raison : mika-arch est 7 % du problème, et scinder produirait un premier ticket
qui rend réglable sans rien régler (invérifiable sur `llm_calls`) et un second qui règle sans
pouvoir régler. La preuve de D1–D3 *est* le réglage de D4 qui fait disparaître la signature
120/240 pour mika-arch.

**Q5 — place de l'invariant D2 → à la construction du fournisseur (chemin froid), acceptable.**
Un agent mal configuré échoue à sa première tentative d'appel, pas au boot. Retenu tel quel :
c'est déjà la place de la panique de mika#1660 (`llm/mod.rs:82-88`), et déplacer l'un sans
l'autre ferait deux endroits où une config invalide est détectée, à deux moments différents.
*Conséquence assumée, écrite ici pour qu'elle ne surprenne pas :* un `mika` qui démarre n'est
pas la preuve que sa config de budgets est valide. La preuve est le premier appel.

---

## Acceptance criteria

Transcription littérale des trois critères du corps de mika#2189. Ils sont le contrat ; les
sections *Périmètre* et *Vérification* disent *comment* on les atteint, celle-ci dit *ce qui
doit être vrai* à la fin.

**AC1 — la distribution réelle des latences/expirations mika-arch est mesurée sur `llm_calls`
(7 jours) : taux d'expiration, par modèle, par taille de brief.**
Livrée sous forme d'un rapport porté par le ticket, contenant les tableaux M1 (latence des
pannes), M2 (taux par agent et par modèle), M4 (percentiles des succès et temps cumulé par
session), M5 (chronologie et variante de prompt). Contraintes nommées :
- l'axe « par taille de brief » est porté par `system_prompt_bytes`, **pas** par la taille du
  message utilisateur, laquelle n'est pas récupérable sur les lignes en échec
  (`input_tokens = 0` en dur, `agent_loop/mod.rs:1093-1112`) — limitation M6, écrite dans le
  rapport, pas passée sous silence ;
- D5 ferme ce trou pour la mesure **suivante** ; il ne rétroagit pas sur les lignes déjà
  écrites ;
- la ligne de base est **capturée dans le rapport avant tout déploiement**, parce que
  `prune_old_llm_calls` (`db.rs:9489`) purge les lignes anciennes et qu'une comparaison
  d'après ne doit pas dépendre de lignes qui auront disparu.

**AC2 — un remède est décidé sur la mesure AC1, pas à l'aveugle.**
Les trois candidats nommés par le corps sont chacun arbitrés par une mesure citée, et deux
sont **rejetés sur preuve** : le rejeu transport borné par M3 (déjà présent, `MAX_RETRIES = 3`,
et un appel de 150 s n'aboutit pas en 120 s quel qu'en soit le rang), le repli de modèle par
M2 (195 des 209 pannes sont hors mika-arch, sur un autre modèle). Le troisième — plafond par
appel relevé — est retenu, et M4 établit qu'il ne suffit pas seul : d'où D1–D4. Un remède
retenu sans qu'une mesure le porte, ou un candidat rejeté sans mesure citée, ne satisfait pas
cet AC.

**AC3 — une passe architecte qui réussit aujourd'hui garde sa latence.**
Deux obligations distinctes, parce que « garder sa latence » a deux faces et qu'une seule est
observable a posteriori :
- **AC3-a (succès) :** p50/p90/p99 des appels mika-arch en `status='success'` inchangés au
  regard de la ligne de base M4 (20,3 s / 98,7 s / 191,3 s), mesurés après au moins 24 h et
  100 appels — per `feedback_never_conclude_inside_the_mechanism_period`. Un plafond relevé
  ne *peut pas* ralentir un appel qui aboutissait ; un écart signifie qu'autre chose a bougé.
- **AC3-b (échec borné) :** `plafond × tentatives ≤ enveloppe` est vérifié au démarrage
  (D2 étendu au rejeu), de sorte qu'un appel qui échoue ne puisse pas déborder de l'enveloppe
  de l'agent. Sans cette moitié, l'AC3 serait satisfait par un correctif qui fait passer le
  coût d'une panne de 240 s à 480 s sans que rien ne l'accuse. La migration de Q1 étant un
  no-op au plafond par défaut, la géométrie 120/300 d'aujourd'hui doit rester acceptée par
  l'invariant : c'est le contrôle positif de V1.

---

## Fire-Disposition

Ce plan livre des **détecteurs** : un invariant qui panique, deux gardes de test, et deux
mesures d'après-correctif. Cette section dit d'avance quoi faire quand ils tirent — pour
qu'aucune décision ne soit prise sous la pression d'un rouge, et surtout pour qu'aucun
détecteur ne soit ajusté pour redevenir vert.

| détecteur | s'il tire | disposition |
|---|---|---|
| **D2** panique sur la configuration **par défaut** (`http = 120`, `agent = 300`) | l'invariant est mal écrit : il rejette la géométrie qui tourne en production aujourd'hui | **(c) halte-et-remontée.** Ne pas relâcher l'inégalité pour faire passer le défaut. C'est le contrôle positif de V1 ; son échec invalide la forme de l'invariant, pas la config. |
| **D2** panique sur une configuration d'agent réglée (par ex. D4) | la gate fait exactement son travail : le réglage demandé ne tient pas dans son enveloppe | **Poursuivre — corriger la configuration, jamais l'invariant.** Le message doit nommer les deux valeurs et le fichier ; si ce n'est pas le cas, c'est le message qu'on corrige. |
| **la garde « plus aucun appelant direct de `AGENT_TOTAL_TIMEOUT_SECS` »** devient rouge | un point de lecture a été ajouté ou oublié — aujourd'hui `agent_loop/mod.rs:3146` et `:3985`, plus `TEAM_AGENT_TIMEOUT_SECS` (`policy.rs:29`) | **(c) halte-et-remontée.** C'est le mode d'échec que `feedback_structural_gate_audit_grep_all_callsites` nomme : une gate qui couvre trois appelants sur quatre est une gate qui ment. |
| **le test de dérivation D3** montre un comportement différent de l'actuel **au plafond par défaut** | la migration littéraux → fractions n'est pas le no-op annoncé en Q1 | **(c) halte-et-remontée.** Ne pas ajuster les fractions pour rattraper : l'écart signifie que la calibration lue (0,75 / 0,25 / 0,50) est fausse, et c'est cette lecture qu'il faut refaire. |
| **V3** : la signature 120 s / 240 s **subsiste** après déploiement | avant de conclure au code : le binaire déployé peut ne pas porter le correctif | **Poursuivre — sonder le déploiement d'abord**, per `feedback_shipped_ne_equal_pas_deployed_uv_tool_env` et `feedback_strings_is_not_a_deployment_probe` (sonde fonctionnelle, pas `strings`). Si le binaire est bon et la signature demeure, le diagnostic M1 est faux et le ticket se re-groome. |
| **V3** : une signature **neuve** apparaît à 240 s / 480 s | le plafond a bougé et la distribution aussi — la queue a suivi le plafond | **(c) halte-et-remontée.** Ne pas re-relever le plafond en réflexe : deux plafonds franchis d'affilée disent que le modèle de la panne est faux, pas que la valeur est trop basse. |
| **AC3-a** : p50/p90/p99 des succès dégradés | un plafond relevé ne peut pas ralentir un succès ; quelque chose d'autre a changé | **(c) halte-et-remontée.** Ne pas absorber l'écart en révisant la ligne de base M4 : elle est capturée avant déploiement précisément pour ne pas être révisable après. |

**Allowlist d'exceptions : vacante.** Aucun test existant n'est autorisé à être modifié,
désactivé, `#[ignore]`é ou vu son attente ajustée dans le périmètre de ce ticket. En
particulier les tests de `parse_http_timeout` (`llm/mod.rs:560-600`) et ceux de
`llm/openai.rs` sur la boucle de rejeu : si D3 les rend rouges, c'est D3 qui est faux.

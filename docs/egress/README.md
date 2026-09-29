# Manifeste d'egress — un sink sortant neuf ne peut plus apparaître sans être déclaré

> mika#2408. `docs/egress/` est sous CODEOWNERS `@samidarko` : **aucune entrée
> ne merge sans revue humaine.**

## Ce que ce répertoire ferme

Avant ce manifeste, ajouter un chemin réseau sortant était **invisible en CI**.
Les trois lints egress existants vérifient des propriétés de sinks *déjà
décidés* — le confinement d'un host connu (`verify-egress-uniqueness.sh`), la
forme d'une requête Brave (`verify-egress-request-shape.sh`), l'absence de
journalisation dans le substrat de recherche (`verify-egress-no-log.sh`). Aucun
ne tirait quand une **nouvelle destination** apparaissait.

Preuve historique : le substrat E1 (mika#1807) et le client Brave (mika#1808)
ont ajouté un chemin d'egress, et les trois lints ont été écrits *dans ces mêmes
PR* pour verrouiller la décision **après coup**. Si Brave avait été ajouté comme
un simple appel `reqwest` vers un host neuf — la forme qu'a Telegram — zéro
check n'aurait tiré.

Le manifeste répond à la question qu'aucun d'eux ne pose : **quelle donnée est
envoyée, vers où, et est-elle journalisée ?**

## Le mécanisme : un lockstep bidirectionnel, pas un champ libre

`scripts/verify-egress-manifest.sh` construit un **inventaire mécanique** des
sinks réellement présents dans l'arbre et le confronte aux entrées `[[sink]]`.
Il échoue dans les deux sens :

| # | direction | prédicat | ce que ça ferme |
|---|---|---|---|
| **D1** | code → manifeste | tout fichier construisant un client HTTP en **production** est couvert par au moins un `client_site` | un module egress **neuf** |
| **D2** | code → manifeste | tout littéral de host **externe** en production est couvert par une `destination`, ou appartient à une classe de `NON_SINK_HOSTS` | un host **neuf** dans un module déjà déclaré |
| **D3** | manifeste → code | tout `client_site` couvre au moins un fichier de l'inventaire D1 | une **déclaration fantôme** |
| **D4** | manifeste → code | toute `destination` déclarée `literal` apparaît au moins une fois comme littéral | un **host fantôme** |

Une déclaration n'est donc acceptée que parce que le check a **mécaniquement
confirmé que le sink existe**, et tout sink réel est nommé. Un champ libre
rempli à la main ne suffit pas.

D2 et D1 ferment des moitiés différentes, et il faut les deux : sans D2,
ajouter `client.post("https://evil.example/")` dans un fichier **déjà déclaré**
ne créerait aucun fichier neuf et passerait.

## Le schéma

Chaque `[[sink]]` porte neuf champs obligatoires plus trois optionnels.

| champ | obligatoire | valeurs | rôle |
|---|---|---|---|
| `id` | oui | chaîne unique | l'identité de l'entrée ; ce qu'un message d'erreur nomme |
| `destination` | oui | host, ou description pour un `skill-declared` | **vers où** |
| `destination_source` | oui | `literal` \| `config` \| `skill-declared` | ce que la destination déclarée vaut à l'exécution |
| `class` | oui | `external` \| `internal` | le trafic quitte-t-il notre périmètre ? |
| `data` | oui | prose | **quelle donnée** part |
| `logged` | oui | booléen | le corps / la query sont-ils journalisés ? |
| `client_site` | oui | chemin de fichier ou de répertoire | **où le client naît** — le champ tenu en lockstep mécanique (D1/D3) |
| `call_site` | oui | chemin | **où la requête part** — documentaire, jamais vérifié |
| `owner` | oui | `@handle` | qui relit |
| `notes` | non | prose | contexte, ticket d'origine, garde qui tient l'invariant |
| `confined` | non | booléen | le host est-il confiné à son substrat par `verify-egress-uniqueness.sh` ? |
| `confined_hosts` | non | liste | les hosts confinés quand l'entrée en couvre plusieurs |
| `also_declares` | non | liste | hosts supplémentaires que cette entrée couvre pour D2/D4 |

### Pourquoi `client_site` ET `call_site` — la mesure qui l'impose

Le ticket proposait un champ unique. Appliqué au sink le plus évident du
ticket, il se casse : **`telegram.rs` ne construit aucun client HTTP en
production.** Son client vient de `crates/mika-gateway/src/main.rs`, un
`reqwest::Client::builder()` partagé injecté dans Telegram, GitHub-gateway et
orchestrator-inbox. Une entrée `call_site = ".../telegram.rs"` confrontée à
l'inventaire des constructeurs serait rejetée comme **déclaration fantôme** —
D3 tirerait sur la déclaration la plus juste du fichier.

D'où deux champs. `client_site` est vérifiable et porte le lockstep ;
`call_site` est ce que le reviewer lit, et il est **documentaire, jamais
vérifié mécaniquement** — écrit ici plutôt que laissé croire. La relation est
N:M : un client partagé alimente trois destinations, et c'est en soi une
information.

### Pourquoi `destination_source` — l'information la plus dense du fichier

Ce n'est pas une commodité de lint.

- `literal` — le host est en dur dans le code.
- `config` — une `base_url` repointable (les 13 providers LLM, `ollama`, les
  endpoints du control-monitor, l'URL du gateway).
- `skill-declared` — l'URL vient du manifeste d'une skill et n'est **pas**
  connaissable statiquement (`skills/executor.rs::execute_http`).

*« api.openai.com » ne décrit le sink que tant que `MIKA_OPENAI_BASE_URL` n'est
pas posé*, et un manifeste qui tairait ça affirmerait une destination que
l'exécution peut démentir. Le champ ne supprime pas l'écart : il le **nomme**.
C'est le même rapport que `llm_budget_resolved` entretient avec le
`config.toml` (mika#2293).

D4 ne s'applique qu'aux `literal` : une destination `config` ou
`skill-declared` n'est par construction dans aucun littéral, et exiger le
contraire ferait rougir le lint sur les déclarations les plus honnêtes.

### Pourquoi `class` — on déclare l'interne, on ne l'exclut pas

AC4 du ticket autorise « exclus **ou** déclarés » pour les appels internes. Ce
manifeste **déclare**, parce qu'exclure par heuristique rouvre exactement le
trou que le ticket ferme : un sink interne qui devient externe ne changerait
aucune ligne. Ici il change `class = "internal"` en `"external"` — un diff d'un
mot, sous CODEOWNERS.

**Coût nommé** : ajouter un module CLI qui parle au démon local coûte désormais
une revue `@samidarko`.

## J'ajoute un chemin réseau sortant — la procédure

1. Écrire le code.
2. Lancer `make verify-egress-manifest`. Il rougit, et nomme la direction.
3. Ajouter l'entrée `[[sink]]` dans `egress-manifest.toml`. Remplir **`data`**
   et **`logged`** honnêtement : ce sont les deux champs qu'aucune machine ne
   vérifie, et c'est pour eux que la revue `@samidarko` existe.
4. Relancer. Vert.
5. La PR touche `docs/egress/` : elle demande une revue `@samidarko`
   automatiquement.

### Si le lint rougit et que vous ne savez pas quoi déclarer

| message | cause | remède |
|---|---|---|
| `undeclared client site` (D1) | un fichier construit un client HTTP et aucune entrée ne le couvre | ajouter l'entrée, `client_site` nommant ce fichier ou son répertoire |
| `undeclared destination` (D2) | un host part du code sans qu'une ligne dise ce qu'on lui envoie | ajouter le host en `destination` ou en `also_declares` ; s'il n'est **pas routable**, étendre `NON_SINK_HOSTS` — par **classe**, jamais par host isolé |
| `phantom client_site` (D3) | l'entrée nomme un chemin où aucun client n'est construit | corriger le champ, ou retirer l'entrée : un manifeste qui décrit un sink disparu est un manifeste qui ment |
| `phantom destination` (D4) | une `destination` déclarée `literal` n'apparaît nulle part | si elle vient d'une configuration, poser `destination_source = "config"` ; si elle vient d'une skill, `"skill-declared"` |

**On déclare, on n'allowliste pas** (doctrine mika#2201).
`scripts/egress-manifest-exceptions.tsv` existe, est **livré vide**, et deux
assertions du test le tiennent vide : quand la première ligne apparaît, on lit
cette ligne et on se demande pourquoi ce sink ne peut pas se déclarer. Il
n'existe aucun sink qu'on ne puisse pas déclarer — y compris celui dont la
destination est arbitraire, qui se déclare `destination_source =
"skill-declared"`.

`NON_SINK_HOSTS` **n'est pas** cette allowlist : c'est une liste de **classes**
de hosts non routables ou réservées aux fixtures (RFC 2606, loopback,
`*.svc.cluster.local`, `*.invalid`, `*.test`). Elle est structurelle, justifiée
par classe, et ne référence aucun ticket de suivi parce qu'elle ne décrit
aucune violation. Les deux fichiers restent séparés pour que le `wc -l` du
second reste lisible.

## Un module cité par le ticket qui n'est PAS un sink

`crates/mika-gateway/src/orchestrator_inbox.rs` est nommé par le ticket
mika#2408 comme preuve vivante d'un sink non couvert (« `:524` →
`reqwest::Client::new()` »), et l'AC2 demande son back-fill. **Il n'a pas
d'entrée dans le manifeste, et c'est la mesure qui le décide.**

```
crates/mika-gateway/src/orchestrator_inbox.rs:495:#[cfg(test)]
crates/mika-gateway/src/orchestrator_inbox.rs:524:        let http_client = reqwest::Client::new();
```

La ligne 524 est **après** le `#[cfg(test)]` de la ligne 495 : c'est le
constructeur d'état d'un harnais de test. Et la mesure va plus loin que celle
du plan : les trois seules mentions de `reqwest` du fichier (524, 526, 537)
sont **toutes** dans ce bloc, et aucune fonction de production
(`handle_post_message`, `handle_stream`, `fetch_after_cursor`,
`mark_delivered`, `purge_old_rows`) n'émet de requête HTTP — ce module est une
surface **entrante** (POST + SSE) adossée à Postgres.

Lui donner une entrée `[[sink]]` pour satisfaire la lettre de l'AC2 écrirait
dans le manifeste un chemin d'egress **qui n'existe pas**. Le lockstep la
laisserait passer (son `client_site` pointerait `main.rs`, où un vrai client
est construit), donc la fausseté serait *silencieuse* — précisément la
déclaration fantôme que D3 et D4 existent pour refuser, réintroduite par la
porte d'entrée.

La thèse du ticket — « la couverture est par-host, pas par-sink » — reste
entièrement vraie : `telegram.rs` et la famille LLM la démontrent, et elles
sont au manifeste. Cette pièce-là démontre autre chose, de plus utile : **un
détecteur qui grep sans découper le code de test rapporte des sinks qui
n'existent pas.** C'est la contrainte d'AC4 rencontrée avant la première ligne
de code, et `scripts/test-verify-egress-manifest.sh` la pinne dans les deux
sens — le fichier doit rester documenté ici, et ne jamais apparaître comme
`client_site`.

## Découpe production / test

Le lint scanne `crates/**/src/**/*.rs` en excluant `tests/`, `examples/`,
`benches/`, `target/`, les lignes de commentaire, et tout fichier désigné par
un `#[cfg(test)] mod NAME;`.

Son parseur `#[cfg(test)]` est **délibérément plus grossier** que celui de
`verify-egress-no-log.sh`, et il ne le réutilise ni ne le copie — voir la note
de tête de `scripts/lib/egress_manifest_lint.py` pour les deux mesures qui
l'imposent. Son unité d'analyse est différente : il répond *« ce **fichier**
a-t-il un sink en production ? »*, pas *« quelles **lignes** sont
production ? »*. Sa règle de doute est donc **inverse** : tout doute conclut
« production », donc « déclare ». Une erreur de découpe produit alors une
**déclaration de plus**, pas un silence.

**Il sur-déclare, et c'est le prix choisi.** Une ligne de manifeste de trop est
visible et corrigible ; un sink omis est silencieux.

### La limite de la coupure, mesurée plutôt que supposée

Le parseur **coupe** au premier `#[cfg(test)] mod X {` au lieu de sauter le bloc
puis de reprendre. Un fichier portant du code de production **après** ce bloc
verrait donc son sink disparaître en silence — l'exact inverse de la règle de
doute ci-dessus.

Le saut a été écrit, essayé, et **refusé sur mesure**. Il demande de compter des
accolades, et les formes que ce comptage ne modélise pas — chaîne sur plusieurs
lignes, chaîne brute, commentaire de bloc — sont *réellement présentes* dans
l'arbre : il fermait le module de test de `crates/mika-gateway/src/telegram.rs`
**265 lignes trop tôt** et produisait trois faux positifs sur `main`, ce qu'AC4
interdit.

**Le trou est réel, pas hypothétique** : `crates/mika-agent/src/server/dashboard.rs`
porte du code de production après son module de test (lignes 1596-1732). Il ne
porte simplement aucun sink — et c'est cette phrase-là qui est remesurée à
chaque run :

```bash
python3 -B scripts/lib/egress_manifest_lint.py --audit-cut-holes .
```

Sortie vide = régime nominal. Le cas N15 de
`scripts/test-verify-egress-manifest.sh` exige qu'elle le reste, et porte son
propre contrôle d'anti-vacuité (le détecteur doit encore *voir* la reprise
témoin, sinon il rendrait 0 en ne mesurant rien — classe mika#2205).
**Un trou mesuré en continu n'est pas un trou silencieux.**

L'assertion porte sur la **conséquence** (un sink invisible), jamais sur la
forme : la détection de reprise sur-rapporte — une fixture Rust dans une chaîne
à continuation de ligne est lue comme une reprise — et cette sur-détection ne
coûte rien tant que la région ne porte aucun sink. Une sur-détection qui en
porte un est exactement ce qu'on veut voir.

**Si N15 rougit**, le remède est de déplacer le code de production **avant** le
bloc `#[cfg(test)] mod` — pas d'élargir le prédicat par réflexe. Apprendre au
parseur à sauter le bloc est possible mais demande de refaire la mesure
ci-dessus d'abord : c'est le comptage d'accolades qui a échoué, et rien ne dit
qu'il réussirait mieux aujourd'hui.

## Ce que ce manifeste n'achète PAS

- **Il ne dit pas ce qui part à l'exécution.** `destination_source = "config"`
  couvre les providers LLM repointables par variable d'environnement : le
  manifeste déclare la destination **du dépôt**, jamais celle du pod.
- **Il ne vérifie pas `logged`.** Ce champ est une **assertion humaine** relue
  sous CODEOWNERS. Sa vérification mécanique existe pour un seul sink
  (`verify-egress-no-log.sh`, substrat Brave) et l'étendre aux autres est un
  travail d'un autre ordre. Un `logged = false` faux est le mode de panne le
  plus coûteux du fichier. **Ticket de suivi.**
- **Il ne vérifie pas `data`.** Un champ de prose que seul un humain peut
  juger ; le rendre vérifiable demanderait une analyse de flux que rien dans ce
  dépôt ne porte.
- **Il ne couvre pas les egress non-HTTP.** `sqlx` vers Postgres, les sockets
  bruts, un `Command::new("curl")` — hors périmètre. Le dernier est déjà fermé
  ailleurs (containment shell-exec, mika#1991).
- **Il ne couvre pas `dashboard/` ni `packages/ui/`** (TypeScript) :
  l'inventaire est un prédicat Rust. Un `fetch()` côté navigateur est une
  population distincte avec un modèle de menace distinct. **Ticket de suivi**,
  précondition : une mesure montrant qu'un `fetch` du dashboard atteint un host
  tiers.
- **Il ne rattrape pas mika#1807 / mika#1808.** Le manifeste naît avec le
  back-fill ; rien ici ne rétro-date une déclaration pour un sink ajouté en
  août. La sonde est la **prochaine** PR qui ajoute un chemin sortant.

## Sondes, et leurs haltes

Ce lint n'émet ni compteur ni événement de journal : **son signal est son
propre rouge**, et son silence ne prouve rien tant qu'on n'a pas établi qu'il
regarde quelque chose.

**S1 — contrôle positif, obligatoire avant toute lecture du silence.**

```bash
bash scripts/verify-egress-manifest.sh --report
```

annonce le nombre de fichiers inventoriés et d'entrées confrontées.
*Halte 1* — un inventaire à **zéro** avec exit 0 : le lint ne regarde plus rien
(répertoire renommé, extension changée) et se lit exactement comme un arbre
propre (classe mika#2205). Le lint **refuse** ce cas de lui-même ; si le refus
n'arrive pas, c'est le refus qu'il faut croire manquant, pas l'arbre qu'il faut
croire sain.

**S2 — le rejeu du défaut fondateur (première PR ajoutant un sink).** Une PR
qui ajoute un appel vers un host neuf doit rougir. *Halte 2* — elle passe :
lire **laquelle** des quatre directions aurait dû tirer. Si c'est D2, la cause
est probablement dans `NON_SINK_HOSTS` (une classe trop large) ; si c'est D1,
dans la découpe production/test. Les deux remèdes diffèrent — **ne pas élargir
le prédicat par réflexe**.

**S3 — contrôle négatif de bruit (30 jours).** Aucune PR sans sink neuf ne doit
rougir. *Halte 3* — un faux positif sur une PR saine coûte une PR entière :
**désarmer d'abord** (retirer le step CI), diagnostiquer ensuite. Un faux
positif est un arbitrage de prédicat, pas un seuil à régler.

**S4 — l'allowlist reste vide.**
`wc -l scripts/egress-manifest-exceptions.tsv` doit rendre 0. *Halte 4* — une
première entrée apparaît : c'est un sink que le mécanisme ne sait pas déclarer,
et c'est le mécanisme qu'il faut relire.

## Fichiers

| chemin | rôle |
|---|---|
| `docs/egress/egress-manifest.toml` | le manifeste — la source unique de vérité |
| `docs/egress/README.md` | ce fichier |
| `scripts/verify-egress-manifest.sh` | l'entrée du lint (CI + `make verify-egress-manifest`) |
| `scripts/lib/egress_manifest_lint.py` | le moteur : inventaire + D1–D4, plus les modes `--report`, `--confined-hosts` (AC5) et `--audit-cut-holes` |
| `scripts/test-verify-egress-manifest.sh` | le test négatif — chaque direction vue rouge |
| `scripts/egress-manifest-exceptions.tsv` | livré **vide**, et pinné vide |

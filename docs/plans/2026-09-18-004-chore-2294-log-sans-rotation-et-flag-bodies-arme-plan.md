# mika#2294 — `server.log` à 22 Go : les bodies ne sont pas en INFO, et rien ne fait tourner ce fichier

- **Ticket :** senara-solutions/mika#2294
- **Priorité :** p3 (hygiène / pression disque)
- **Branche :** `chore/2294/logging-server-log-22go-les-bodies-llm`
- **Lignage :** mika#2220 (la table de vérité unique du flag, et le piège « armé sur le mauvais process »), mika#2195 (une seule couche JSON quand un fichier est configuré — l'invariant que toute rotation doit préserver), mika#2131 (la mesure qui prouve que le flag est armé en production), mika#2290 / mika#2331 (deux sondes documentées qui **exigent** le body complet), mika#2205 (un instrument silencieusement inactif se lit comme un instrument oisif)

---

## Contexte

Le ticket mesure `/var/log/mika/server.log` ≈ 22 Go et impute le volume aux corps de
requête/réponse LLM « émis au niveau **INFO** ». Il propose trois remèdes, dans cet
ordre : (1) passer les bodies en DEBUG, (2) sinon les tronquer/redacter, (3) « envisager
une rotation/rétention (logrotate) en complément ».

La lecture du code déplace le diagnostic sur les trois points à la fois. Le remède (1)
**est déjà en place** et l'a toujours été ; le remède (2) casserait deux sondes
opérateur documentées ; et le remède (3), que le ticket met en dernier entre
parenthèses, est **le seul défaut structurel** — il est même déjà écrit noir sur blanc
dans la documentation du dépôt.

Ce plan inverse donc l'ordre du ticket : la rotation est le corps du travail, le flag
est un geste d'environnement précédé d'une mesure.

---

## Ce qui est établi, et comment le vérifier

### E1 — Les bodies LLM ne sont **pas** émis en INFO

Ils sont émis par `debug!` sur une cible dédiée, `mika::llm_debug`, aux trois rails, et
chaque site est de surcroît gardé par `tracing::enabled!` — donc même la sérialisation
JSON du body ne se paie pas quand la cible n'est pas admise :

```rust
// crates/mika-common/src/llm/openai.rs:222
if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG)
    && let Ok(body_json) = serde_json::to_string(request)
{
    debug!(target: "mika::llm_debug", body = %body_json, provider = %self.provider_kind, "llm request body");
}
```

Les six sites, tous en `debug!` :

```
crates/mika-common/src/claude.rs:913      llm request body (anthropic)
crates/mika-common/src/claude.rs:1005     llm response body (anthropic)
crates/mika-common/src/llm/openai.rs:226  llm request body
crates/mika-common/src/llm/openai.rs:323  llm response body
crates/mika-common/src/llm/ollama.rs:464  llm request body
crates/mika-common/src/llm/ollama.rs:562  llm response body
```

Vérification : `grep -rn 'llm request body\|llm response body' crates/` — six lignes,
zéro `info!`.

**Conséquence directe : le remède (1) du ticket est un no-op.** Il n'y a pas de niveau à
abaisser ; il est déjà au plancher, sur une cible que le filtre par défaut n'admet pas.

### E2 — La cible n'est admise **que** si `MIKA_LOG_LLM_BODIES` est armé

Un seul site ajoute la directive, dans les deux constructeurs de souscripteur :

```rust
// crates/mika-common/src/logging.rs:374 (init) et :503 (init_pretty)
if log_llm_bodies {
    filter = filter.add_directive("mika::llm_debug=debug".parse().unwrap());
}
```

Sans le flag, les six `debug!` ci-dessus ne coûtent qu'un test de niveau. **Le volume
n'existe pas en régime désarmé.**

### E3 — Le flag **est** armé sur le mika-spirit de production, et c'est mesuré

Mesure indépendante, du 2026-09-03, consignée dans `crates/mika-agent/src/auto_pull.rs`
(commentaire d'en-tête du module, lignes ~1125-1132) au titre de mika#2131 :

> Measured 2026-09-03 over the last 200 MB of `/var/log/mika/server.log` : […] **517 of
> 517 DEBUG lines sampled carried `target: mika::llm_debug`**. The filter admits exactly
> one target at DEBUG ; the rest start at INFO. That is configuration, not volume.

Le même fait est réaffirmé dans le message d'un test structurel du même fichier
(`auto_pull.rs:6492`). Autrement dit : à cette date, le seul DEBUG collecté par ce
serveur **est** le body LLM, ce qui n'est possible que si `MIKA_LOG_LLM_BODIES` est posé
sur son environnement.

Le ticket a donc raison sur **la cause du volume** (les bodies) et tort sur **son
mécanisme** (un flag dev-only armé en production, pas un niveau de log mal choisi). La
différence n'est pas cosmétique : elle déplace le remède du code vers l'environnement,
et elle explique pourquoi un correctif de code sur ce seul axe n'aurait rien changé.

**Ce que ce plan ne peut pas établir depuis le worktree.** Le pilote tourne en sandbox :
`/var/log/mika/` et `~/.mika/.env` y sont inaccessibles (`No such file or directory`).
Ni la taille actuelle, ni l'armement actuel du flag ne sont vérifiables ici. D'où
l'étape 0 ci-dessous, avec ses branches — et non une affirmation.

### E4 — `MIKA_LOG_LLM_BODIES` n'est armé par aucun fichier du dépôt

- `.env.example:136` le pose **commenté**, à `false`.
- `/etc/conf.d/mika-spirit` (hors dépôt, lu pendant l'étude) ne le mentionne pas.
- `packaging/systemd/mika-spirit.service` ne le mentionne pas.

Il vient donc de `~/.mika/.env`, que le script OpenRC source (`set -a ; . /home/samidarko/.mika/.env ; set +a`), ou de l'environnement du service. C'est une variable posée à la main, un jour, pour un diagnostic — et jamais retirée. Le code **dit déjà qu'elle est armée** : `announce_llm_body_capture` émet un `warn!` `llm_body_capture` au démarrage, nommant le fichier de destination (`logging.rs:296`).

### E5 — Le vrai défaut structurel : `Rotation: None`, et c'est documenté

`docs/runtime-structure.md:239-247` :

| Binary | Location | Format | Rotation |
|--------|----------|--------|----------|
| `mika` (CLI) | `{agent_home}/logs/mika.log` | JSON | Daily (tracing_appender) |
| `mika-spirit` | stdout — **only when no log file is set** | JSON | **None** |
| `mika-spirit` | `$MIKA_SPIRIT_LOG_FILE` (optional) | JSON | **None** |
| `mika-gateway` | (idem) | JSON | **None** |

Et dans le code, le nom de la fonction le dit : `tracing_appender::rolling::**never**`
(`logging.rs:393` et `:427`), contre `rolling::daily` pour le sink par-agent (`:517`,
`:541`).

Les deux configurations possibles de mika-spirit convergent :

- `MIKA_SPIRIT_LOG_FILE` **posé** → `rolling::never` sur ce chemin ; couche stdout JSON
  retirée (mika#2195). Un fichier, jamais tourné.
- `MIKA_SPIRIT_LOG_FILE` **absent** → couche stdout JSON installée, et le service OpenRC
  la redirige vers ce même chemin (`output_log="/var/log/mika/server.log"`,
  `error_log=` idem). Un fichier, jamais tourné.

**Le fait est robuste à l'incertitude de E3** : quoi qu'on logue et quel que soit le
réglage, ce fichier croît sans borne, pour toujours. C'est lui qui transforme n'importe
quel régime d'écriture en pression disque. Aucune rotation n'existe par ailleurs :
`grep -rn logrotate packaging/ docs/ scripts/` ne rend rien, et
`packaging/debian/mika-spirit.postinst` crée `/var/log/mika` sans jamais y poser de
politique de rétention.

Corollaire moins visible, à traiter dans le même geste : le sink par-agent tourne
(`rolling::daily`) mais **sans rétention** — `max_log_files` n'est employé nulle part
(`grep -rn max_log_files crates/` : aucun résultat). Les `mika.log.YYYY-MM-DD`
s'accumulent indéfiniment. Volume bien moindre (mika#2069 mesure 346 `turn_usage` au
total sur l'ensemble des logs par-agent, contre 21 084 côté serveur), donc traité en
second et sans urgence.

### E6 — Le piège de `copytruncate`, et pourquoi il faut le vérifier avant d'écrire le fichier

`copytruncate` est la seule variante de logrotate qui fonctionne ici — les deux
écrivains possibles (l'appender `tracing_appender`, ou `supervise-daemon` qui redirige)
gardent leur descripteur ouvert et **aucun des deux ne sait rouvrir sur signal**. Un
`create` (rename + nouveau fichier) les laisserait écrire dans l'inode renommé : le
fichier « courant » resterait vide pour toujours, en silence.

Mais `copytruncate` a une précondition dure : **le descripteur doit être en `O_APPEND`.**
Sans `O_APPEND`, le noyau conserve l'offset ; après le `truncate(0)`, la prochaine
écriture se fait à l'ancien offset et le fichier est repeuplé de 22 Go de zéros (sparse).
La rotation aurait alors l'air de marcher (`ls -l` montrerait un petit fichier après
`du`… et l'inverse) tout en ne libérant rien de façon fiable.

C'est vérifiable directement, sans redéploiement, et c'est une étape obligatoire du
volet A :

```bash
pid=$(pgrep -f '^/home/samidarko/.local/bin/mika-spirit')
ls -l /proc/$pid/fd | grep server.log            # identifier le n° de fd
grep flags /proc/$pid/fdinfo/<fd>                # attendu : bit 0o2000 (O_APPEND) posé
```

`tracing_appender` ouvre avec `OpenOptions::append(true)` — donc `O_APPEND` — et
`supervise-daemon` fait de même pour ses redirections. Les deux sont **attendus**
conformes ; la sonde existe pour que ce soit constaté plutôt que supposé, parce qu'un
faux sur ce point transforme le correctif en aggravation.

### E7 — Tronquer les bodies casserait deux sondes documentées

Le remède (2) du ticket — « tronquer/redacter en INFO (garder un résumé : modèle,
tokens, latence, statut) » — est déjà satisfait **par un autre événement** : `turn_usage`
porte exactement ces champs, en INFO, ungated (Signal O du `CLAUDE.md` racine). Il n'y a
rien à ajouter de ce côté.

Et tronquer le body lui-même détruirait la seule chose pour laquelle le flag existe. Deux
procédures opérateur écrites exigent le corps **entier** :

- mika#2290 : « pour lire le fait posé, armer `MIKA_LOG_LLM_BODIES` **sur mika-spirit** et
  lire le bloc `## Runtime` » — un bloc situé dans un prompt système qui pèse 54-60 Ko
  pour mika-arch. Un plafond à 10 Ko le coupe avant.
- mika#2331, étape 2 de la procédure « lire un hang LLM » : la comparaison de taille de
  brief entre tours sains et tours en échec.

Un plafond est donc écarté explicitement. **Le remède au volume n'est pas de dégrader
l'instrument ; c'est de ne pas le laisser armé en permanence, et de faire tourner le
fichier.**

### E8 — Les bodies ne sont pas scrubbés

`secret_scrubber::scrub_secrets()` couvre `tool_calls.input` / `.output` en base
(schéma v29). Il ne couvre pas le chemin journal : un body est écrit tel quel. Le
`warn!` de `announce_llm_body_capture` le dit déjà — « they can carry anything the agent
was sent ». Ce n'est pas une vulnérabilité nouvelle (le flag est dev-only et le disque
est local), mais c'est une raison indépendante de borner la rétention plutôt que de
garder 22 Go de prompts complets *ad vitam*. Élargir le scrubber au journal est hors
périmètre — voir la dernière section.

---

## Décisions

### D1 — Ordre inversé : la rotation est le corps du travail, pas le « complément »

Justifiée par E1+E2 (le remède 1 est un no-op), E7 (le remède 2 est nuisible) et E5 (le
défaut est permanent, indépendant du flag, et déjà documenté comme tel).

### D2 — logrotate avec `copytruncate`, et **pas** `rolling::daily` dans le code

Passer `logging.rs` de `rolling::never` à `rolling::daily` paraît plus propre : rotation
native, pas de dépendance système. C'est écarté, et la raison est la classe de panne la
plus coûteuse de ce dépôt.

`rolling::daily(dir, "server.log")` écrit `server.log.2026-09-18`. **Le chemin
`/var/log/mika/server.log` cesse d'exister** — ou, dans la configuration OpenRC, ne
contient plus que ce que le launcher y redirige, c'est-à-dire presque rien depuis
mika#2195. Or ce chemin est la cible de **plusieurs dizaines** de sondes opérateur
écrites, sous la forme `grep <event> $MIKA_SPIRIT_LOG_FILE` : Signaux A à R du `CLAUDE.md`
racine, `llm_budget_resolved`, `llm_call_attempt`, `phantom_sweep_complete`,
`auto_pull_stop_armed`, `qa_review_reconcile_*`, et la totalité des procédures de halte
associées. Toutes rendraient **zéro ligne**.

Et un grep vide, sur toutes ces sondes sans exception, se lit comme *régime nominal*. La
régression ne rendrait aucune décision fausse : elle rendrait l'instrument muet en se
faisant passer pour une bonne nouvelle. C'est exactement ce que mika#2205 a dû nommer
une fois (« un scan silencieusement inactif se lit comme un scan oisif ») et ce que
mika#2131 a dû réparer sur une autre surface.

`copytruncate` préserve l'inode, le chemin et l'invariant mika#2195 : un seul fichier,
écrit une seule fois. Les archives deviennent `server.log.1.gz`, `server.log.2.gz`… et
les sondes continuent de viser le fichier courant.

Coût nommé : `copytruncate` a une fenêtre de course — les lignes écrites entre la copie
et le `truncate` sont perdues. Pour un journal d'observabilité, quelques lignes par
rotation quotidienne est un prix acceptable, et il n'existe pas d'alternative sans
rouvrir le descripteur (que ni l'appender ni le superviseur ne savent faire).

### D3 — Le fichier logrotate est **versionné** dans le dépôt, son installation reste un geste opérateur

`packaging/logrotate/mika` entre au dépôt. Son installation (`/etc/logrotate.d/mika`)
n'est pas automatisée ici : le déploiement courant est un `make deploy` qui n'écrit rien
sous `/etc`, et le `postinst` Debian ne décrit pas la machine de production (OpenRC,
utilisateur `samidarko`, pas `mika`). Ajouter le fichier au `postinst` **et** documenter
le geste manuel pour l'installation OpenRC : deux moitiés, la seconde étant celle qui
mord aujourd'hui.

### D4 — Le flag n'est pas retiré par le code, et aucun garde-fou n'est ajouté

Tentation à écarter : refuser le démarrage, ou désarmer automatiquement, quand
`MIKA_LOG_LLM_BODIES` est armé sur un mika-spirit de production. Trois raisons.

1. Rien dans le process ne sait qu'il est « de production » — `MIKA_DEPLOYMENT` (mika#2290)
   distingue `local` / `cloud`, jamais dev / prod, et le poste de Vincent est justement
   `local` sans être un terrain de jeu.
2. Le signal existe déjà et il est au bon niveau : `announce_llm_body_capture` émet un
   `warn!` au démarrage. Ce qui manque n'est pas l'émission, c'est qu'un WARN posé une
   fois au boot est enterré sous plusieurs Go quelques jours plus tard — ce que la
   rotation corrige mécaniquement.
3. Un p3 d'hygiène ne justifie pas une nouvelle garde de démarrage. La flotte a déjà
   payé une fois le prix d'un refus de démarrage mal calibré (mika#2293 le dit :
   *« refuser de démarrer sur un réglage sous-optimal mais fonctionnel coucherait la
   flotte »*).

Le remède sur cet axe est donc : **mesurer (étape 0), puis retirer la ligne de
`~/.mika/.env` si elle y est, et redémarrer.** Un geste, documenté.

### D5 — Rétention par-agent : `max_log_files`, en second et sans le lier au reste

Le sink par-agent tourne déjà mais n'oublie jamais. `tracing_appender::rolling::Builder`
expose `max_log_files(n)`. C'est un changement d'une ligne par site, à faible risque,
mais qui **supprime des fichiers** — donc il porte son propre test et sa propre mention
dans la doc, et il ne conditionne pas le volet A.

---

## Volets d'implémentation

### Volet 0 — Mesure opérateur, avant toute ligne de code

Non implémentable depuis le sandbox (cf. E3). Trois questions, trois branches, exécutées
sur la machine de production. Ce volet n'est pas un préalable de confort : la branche
prise change le contenu du volet B.

```bash
# Q1 — quelle taille, et quelle part revient aux bodies ?
ls -l /var/log/mika/server.log
grep -c '"llm request body\|"llm response body' /var/log/mika/server.log
# Part en octets (approximation par échantillon, le fichier est trop gros pour un awk complet) :
tail -c 200000000 /var/log/mika/server.log | grep -a 'llm re[qs]' | wc -c

# Q2 — le flag est-il armé sur le process qui écrit ?
pid=$(pgrep -f '^/home/samidarko/.local/bin/mika-spirit')
tr '\0' '\n' < /proc/$pid/environ | grep -i 'MIKA_LOG_LLM_BODIES\|MIKA_SPIRIT_LOG_FILE\|RUST_LOG'
grep -a llm_body_capture /var/log/mika/server.log | tail -3   # le WARN de démarrage

# Q3 — précondition de copytruncate (E6)
ls -l /proc/$pid/fd | grep server.log
grep flags /proc/$pid/fdinfo/<fd>
```

Branches :

| Observation | Lecture | Conséquence sur ce plan |
|---|---|---|
| Q2 rend `MIKA_LOG_LLM_BODIES` armé **et** Q1 montre les bodies majoritaires | E3 confirmé, diagnostic du ticket validé quant à la cause | Volet B = retirer la ligne + redémarrer. Volets A/C inchangés |
| Q2 rend le flag **absent ou désarmé** | Alors les 22 Go ne viennent **pas** des bodies, et tout le diagnostic du ticket tombe | **Halte.** Ne pas toucher au flag. Mesurer quel `event` domine (`jq -r .event \| sort \| uniq -c \| sort -rn` sur un échantillon) et rouvrir le diagnostic ; le volet A reste valide et suffisant pour le ticket |
| Q3 montre `O_APPEND` **absent** | `copytruncate` repeuplerait le fichier de zéros (E6) | **Halte sur le volet A.** Ne pas installer le fichier logrotate en l'état ; l'alternative est `create` + redémarrage planifié du service à chaque rotation, qui est une autre décision |
| Q1 montre `llm request body` ≈ 0 mais le fichier ≫ 1 Go | Croissance par volume nominal, pas par bodies | Le volet A est **à lui seul** la réponse au ticket ; le volet B devient sans objet et doit être dit tel |

### Volet A — Rotation et rétention du log serveur *(le corps du travail)*

**A1.** Créer `packaging/logrotate/mika` :

```
/var/log/mika/*.log {
    daily
    rotate 14
    size 200M
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
    su samidarko samidarko
    create 0644 samidarko samidarko
}
```

Notes de conception, à porter en commentaire dans le fichier :
- `copytruncate` est **obligatoire** et non un choix de style — voir D2/E6 ; le
  remplacer par `create` casse silencieusement les deux écrivains.
- `size 200M` **avec** `daily` : logrotate tourne dès qu'une des deux conditions est
  remplie, ce qui borne le pire cas (un jour de bodies armés dépasse largement 200 Mo)
  sans fragmenter un log calme.
- `rotate 14` + `compress` : la borne dure. À 200 Mo par archive compressée ~10×, le
  plafond est de l'ordre de quelques centaines de Mo — contre 22 Go aujourd'hui.
- `su`/`create` nomment l'utilisateur propriétaire : le service tourne en `samidarko`
  (`command_user="samidarko"` dans l'init OpenRC), pas en `mika` comme le suppose le
  `postinst` Debian. Les deux cas sont à couvrir en doc plutôt qu'à deviner.

**A2.** `packaging/debian/mika-spirit.postinst` : installer le fichier sous
`/etc/logrotate.d/mika` (avec la variante `su mika mika` pour ce paquet), après la
création de `/var/log/mika`.

**A3.** Doc — `docs/runtime-structure.md` : la colonne `Rotation` des quatre lignes
`None` devient une référence à la politique logrotate, avec la phrase qui manque
aujourd'hui : *sans installation de `/etc/logrotate.d/mika`, ce fichier croît sans
borne.* Plus le chemin du fichier versionné et le geste d'installation OpenRC.

**A4.** `docs/configuration.md`, entrée `spirit_log_file` : ajouter la même mention
(c'est la page qu'un opérateur lit quand il pose la variable).

### Volet B — Le flag, conditionné à la branche du volet 0

**B1.** Si le volet 0 confirme l'armement : retirer `MIKA_LOG_LLM_BODIES` de
`~/.mika/.env`, redémarrer `mika-spirit`, vérifier l'extinction par
`grep -c '"llm request body' /var/log/mika/server.log` sur la fenêtre post-redémarrage
(attendu : 0) et par l'absence du WARN `llm_body_capture` au démarrage suivant.

**B2.** Doc — `docs/configuration.md`, entrée `MIKA_LOG_LLM_BODIES` : ajouter la phrase
que l'incident rend nécessaire. Le flag est **dev-only et à retirer après usage** ; armé
en permanence sur un mika-spirit il produit l'essentiel du volume du journal ; son
armement est constatable par `grep llm_body_capture $MIKA_SPIRIT_LOG_FILE` (le WARN de
démarrage) et le désarmer exige un **redémarrage** (lu une fois par process,
non-hot-swappable — même contrat que `MIKA_AGENT_TIER` et `MIKA_DEPLOYMENT`).

**B3.** `.env.example:136` : le commentaire dit « dev-only » ; y ajouter « — retirer
après le diagnostic ; armé en permanence, c'est le premier poste de volume du journal ».

**B4 (petit, optionnel).** `mika logs paths` expose déjà `size_bytes` pour les deux
sinks (`crates/mika-cli/src/commands/logs.rs:37-46`). Ajouter un avertissement au-delà
d'un seuil (1 Go) nommant la politique logrotate. C'est la surface qui répond
directement au second symptôme du ticket (« greps de diagnostic lourds »), sans créer de
surface nouvelle. **Écarté si le volet 0 branche « halte »** — inutile d'instrumenter un
symptôme dont la cause n'est pas encore établie.

### Volet C — Rétention du sink par-agent

**C1.** `logging.rs` `init_pretty`, deux sites (`:517`, `:541`) : remplacer
`tracing_appender::rolling::daily(dir, "mika.log")` par le `Builder` équivalent avec
`.max_log_files(14)`.

**C2.** Le tableau de `docs/runtime-structure.md` passe de `Daily (tracing_appender)` à
`Daily, 14 jours retenus`.

**C3.** Ce volet **supprime des fichiers** — donc il porte son propre test (voir contrat
ci-dessous) et il est séparable : si le relecteur le juge hors périmètre d'un p3, il
part dans son propre ticket sans rien retirer aux volets A et B.

---

## Verification contract

| # | Ce qui est vérifié | Comment |
|---|---|---|
| V1 | Aucun body LLM n'est émis en INFO | Test de source dans `logging.rs::tests` : les six sites `llm re{quest,sponse} body` sont tous `debug!` sur `mika::llm_debug`. Une régression vers `info!` ou vers la cible par défaut rougit |
| V2 | Désarmé, aucun body n'est écrit | Test existant `crates/mika-common/tests/tui_llm_body_capture.rs` — étendre si besoin pour asserter l'absence des marqueurs quand `log_llm_bodies = false` |
| V3 | Le body n'est **pas** tronqué quand armé | Test : un body dépassant 60 Ko (taille du prompt système mika-arch) traverse intact. Garde contre une « optimisation » future qui reprendrait le remède (2) du ticket et casserait les sondes mika#2290 / mika#2331 |
| V4 | Le fichier logrotate est syntaxiquement valide et emploie `copytruncate` | `logrotate --debug packaging/logrotate/mika` en CI si `logrotate` est disponible ; sinon test de source : le fichier contient `copytruncate` et **ne contient pas** `create` en directive de rotation. La seconde moitié est la garde utile — la substitution est silencieuse et destructrice (D2/E6) |
| V5 | L'invariant mika#2195 survit | `json_stdout_layer_enabled` et ses tests sont inchangés ; le volet A ne touche pas `logging.rs` |
| V6 | La rétention par-agent borne bien | Test sur `Builder::max_log_files` : au-delà de N fichiers, les plus anciens disparaissent. Test d'intégration avec un `tempdir`, pas un test de source |
| V7 | Aucune sonde opérateur ne change de chemin | Test de source : `MIKA_SPIRIT_LOG_FILE` et `/var/log/mika/server.log` restent les seules cibles documentées ; aucun `rolling::daily` n'apparaît dans `init` (le constructeur serveur). Cette garde existe parce que la régression serait **muette** — des dizaines de `grep` rendraient zéro ligne, ce qui se lit comme régime nominal |
| V8 | La mesure du volet 0 est reproductible | Les commandes du volet 0 sont copiables telles quelles dans le corps de PR et dans `docs/runtime-structure.md` |

---

## Fire-Disposition

- **Portée :** packaging (nouveau fichier), documentation (trois fichiers), un
  changement de code borné (volet C, deux lignes + tests), un ajout facultatif au CLI
  (volet B4). Aucun changement au chemin d'exécution de l'agent, aucun changement au
  filtre de log, aucune nouvelle variable d'environnement.
- **Risque principal :** la précondition `O_APPEND` du volet A (E6). Elle est
  vérifiable avant installation et la halte est écrite.
- **Risque secondaire :** le volet C supprime des fichiers. Séparable.
- **Réversibilité :** volet A — `rm /etc/logrotate.d/mika`. Volet B — reposer la ligne
  dans `.env`. Volet C — revert d'une ligne.

---

## Definition of Done

- `packaging/logrotate/mika` existe, versionné, commenté sur le pourquoi de
  `copytruncate`, et installé sur la machine de production après la vérification Q3.
- `packaging/debian/mika-spirit.postinst` l'installe pour le paquet Debian.
- `docs/runtime-structure.md` ne dit plus `Rotation: None` sans dire ce qu'il faut faire
  pour que ce soit faux.
- `docs/configuration.md` et `.env.example` disent que `MIKA_LOG_LLM_BODIES` est à
  retirer après usage, comment constater qu'il est armé, et qu'un redémarrage est requis.
- Le volet 0 a été exécuté et sa branche est consignée dans le corps de la PR — y compris
  si elle est une halte.
- `cargo test`, `cargo clippy`, `cargo fmt --check` passent.
- V1 à V8 sont couverts ou explicitement renvoyés à un suivi.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous sont
dérivés de son §Fix et du contrat de vérification.

- **AC1 — Les bodies LLM ne sont pas émis à un niveau collecté par défaut.** Vérifié, et
  désormais gardé par un test (V1). *Note : déjà vrai avant ce travail — le ticket
  demandait un changement de niveau qui n'avait pas lieu d'être (E1).*
- **AC2 — Aucune sonde opérationnelle ne dépend du body à un niveau collecté.** Établi :
  le résumé (modèle, tokens, latence, statut) que le ticket demande de conserver est déjà
  porté par `turn_usage`, en INFO, ungated (E7). Les deux sondes qui lisent le body
  (mika#2290, mika#2331) arment explicitement le flag et exigent le corps entier — d'où
  le refus de tronquer, et le test V3 qui le fige.
- **AC3 — Une rotation et une rétention existent pour `server.log`.** Fichier logrotate
  versionné, installé, avec une borne dure (`rotate 14` + `size 200M` + `compress`), et
  sans changer le chemin que les sondes opérateur visent (D2, V7).
- **AC4 — La cause du volume est établie par mesure, pas par inférence.** Volet 0
  exécuté, branche consignée. Si la mesure infirme le diagnostic du ticket, c'est un
  résultat à écrire, pas un échec à contourner.
- **AC5 — Le geste de désarmement est documenté et exécuté si la mesure le justifie.**
  Ligne retirée de `~/.mika/.env`, service redémarré, extinction constatée (B1).
- **AC6 — La documentation ne laisse plus croire à une rotation qui n'existe pas.**
  `docs/runtime-structure.md` et `docs/configuration.md` à jour.

---

## Surfaces opérateur et sonde post-déploiement

**Déjà existant, inchangé :** `grep llm_body_capture $MIKA_SPIRIT_LOG_FILE` — le WARN de
démarrage nommant le sink. **Régime attendu après ce travail : zéro ligne.** Toute
occurrence est un mika-spirit qui écrit des prompts complets sur disque ; c'est
légitime le temps d'un diagnostic, et c'est un défaut d'hygiène le reste du temps.

**Sonde post-déploiement, à 7 et 14 jours :**

```bash
ls -l /var/log/mika/                 # attendu : server.log courant + archives .gz bornées
du -sh /var/log/mika/                # attendu : quelques centaines de Mo, pas 22 Go
```

**Deux haltes.**

*Le fichier courant repart de zéro mais l'espace disque ne se libère pas* → signature de
l'absence d'`O_APPEND` (E6) : le fichier est devenu sparse. `du` et `ls -l` divergent.
**Désinstaller le fichier logrotate** et reprendre par la branche « create + redémarrage
planifié », qui est une autre décision.

*Une sonde opérateur documentée rend zéro ligne après l'installation* → **ne pas conclure
au régime nominal**. Vérifier d'abord que `/var/log/mika/server.log` est bien le fichier
courant et non une archive, c'est-à-dire la régression que D2 existe pour interdire.
C'est précisément la classe de panne dont la signature est l'absence de signature.

---

## Hors périmètre (suivi à ouvrir)

- **Scrubber sur le chemin journal (E8).** `secret_scrubber` couvre `tool_calls` en base,
  pas les bodies en log. Élargir sa portée touche un chemin chaud et mérite d'être
  arbitré pour lui-même — la borne de rétention réduit la fenêtre d'exposition, elle ne
  la ferme pas. **Ticket de suivi.**
- **`mika-gateway`, même trou.** `docs/runtime-structure.md` porte `Rotation: None` sur
  ses deux lignes aussi. Le glob `/var/log/mika/*.log` du fichier logrotate le couvre
  **si** son log vit là ; sinon il lui faut sa propre entrée. À vérifier dans le même
  geste d'installation, à ticketer s'il a un chemin distinct.
- **Le volume nominal lui-même.** Si le volet 0 branche « halte » (flag désarmé), la
  question « quel `event` domine le journal ? » est un diagnostic distinct, avec ses
  propres arbitrages de cadence — et le `CLAUDE.md` racine porte déjà une doctrine
  explicite sur ce point (mika#2131 : agrégat par tick au journal, détail par ticket en
  `audit_events`). **Ticket de suivi**, à ouvrir avec la mesure en main.
- **Une garde de démarrage sur `MIKA_LOG_LLM_BODIES`.** Écartée par D4, et non par
  timidité : le process ne sait pas s'il est « de production », et la flotte a déjà payé
  le prix d'un refus de démarrage mal calibré. Si le flag se révèle armé une seconde
  fois, la question se rouvre — avec la récurrence comme argument.

# mika#2294 — `server.log` à 22 Go : les bodies ne sont pas en INFO, et rien ne fait tourner ce fichier

- **Ticket :** senara-solutions/mika#2294
- **Priorité :** p3 (hygiène / pression disque)
- **Branche :** `chore/2294/logging-server-log-22go-les-bodies-llm`
- **Lignage :** mika#2220 (la table de vérité unique du flag, et le piège « armé sur le mauvais process »), mika#2195 (une seule couche JSON quand un fichier est configuré — l'invariant que toute rotation doit préserver, **et ce qui ferme la voie `docker logs` en E10**), mika#2131 (la mesure qui prouve que le flag est armé en production), mika#2290 / mika#2331 (deux sondes documentées qui **exigent** le body complet), mika#2205 (un instrument silencieusement inactif se lit comme un instrument oisif), mika#2103 (« a guard nobody has watched go red is a decoration » — la doctrine qui fixe la forme du garde V4)

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

Vérification : `grep -rn 'llm request body\|llm response body' crates/` — **huit** lignes,
zéro `info!`. Les six ci-dessus, plus deux dans
`crates/mika-common/tests/tui_llm_body_capture.rs:46-47`, qui émettent les mêmes marqueurs
pour éprouver la capture (V2). Le compte à opposer au ticket est donc « six sites de
production », et la commande brute en rend huit — le dire ici évite que le relecteur qui la
rejoue lise l'écart comme une erreur du plan.

**Conséquence directe : le remède (1) du ticket est un no-op.** Il n'y a pas de niveau à
abaisser ; il est déjà au plancher, sur une cible que le filtre par défaut n'admet pas.

### E2 — La cible n'est admise **que** si `MIKA_LOG_LLM_BODIES` est armé

Un seul site ajoute la directive, dans les deux constructeurs de souscripteur :

```rust
// crates/mika-common/src/logging.rs:375 (init) et :504 (init_pretty)
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
- `packaging/systemd/mika-spirit.service` ne le mentionne pas — **mais il pose
  `MIKA_SPIRIT_LOG_FILE=/var/log/mika/server.log` et `User=mika`**, ce qui est sans effet
  sur *ce* point et décisif pour E9 : c'est un troisième déploiement, versionné, qui écrit
  au chemin de gentux sous un autre propriétaire.

Il vient donc de `~/.mika/.env`, que le script OpenRC source (`set -a ; . /home/samidarko/.mika/.env ; set +a`), ou de l'environnement du service. C'est une variable posée à la main, un jour, pour un diagnostic — et jamais retirée. Le code **dit déjà qu'elle est armée** : `announce_llm_body_capture` émet un `warn!` `llm_body_capture` au démarrage, nommant le fichier de destination (`logging.rs:296`).

### E5 — Le vrai défaut structurel : `Rotation: None`, et c'est documenté

`docs/runtime-structure.md:240-247` — **six** lignes, pas quatre ; le gateway en porte
deux comme le serveur, et c'est ce qui donne son compte à A3 (« les quatre lignes `None` »)
et à C2 (« les lignes CLI **et** team mode ») :

| Binary | Location | Format | Rotation |
|--------|----------|--------|----------|
| `mika` (CLI) | `{agent_home}/logs/mika.log` | JSON | Daily (tracing_appender) |
| `mika` (team mode) | `{team_dir}/logs/mika.log` | JSON | Daily |
| `mika-spirit` | stdout — **only when no log file is set** | JSON | **None** |
| `mika-spirit` | `$MIKA_SPIRIT_LOG_FILE` (optional) | JSON | **None** |
| `mika-gateway` | stdout — **only when no log file is set** | JSON | **None** |
| `mika-gateway` | `$MIKA_GATEWAY_LOG_FILE` (optional) | JSON | **None** |

Et dans le code, le nom de la fonction le dit : `tracing_appender::rolling::**never**`
(`logging.rs:393` et `:427`), contre `rolling::daily` pour le sink par-agent (`:517`,
`:541`).

Les deux mécanismes d'écriture convergent vers le même défaut :

- `MIKA_SPIRIT_LOG_FILE` **posé** → `rolling::never` sur ce chemin ; couche stdout JSON
  retirée (mika#2195, `json_stdout_layer_enabled` = `!log_file_configured`,
  `logging.rs:348`). Un fichier, jamais tourné.
- `MIKA_SPIRIT_LOG_FILE` **absent** → couche stdout JSON installée, et le service OpenRC
  la redirige vers un chemin (`output_log=`, `error_log=`). Un fichier, jamais tourné.

**Ils ne sont pas exclusifs, et la première version de ce plan les présentait comme tels.**
Sur le déploiement versionné (E9), `os/openrc/conf.d/mika-spirit` pose la variable **et**
`os/openrc/init.d/mika-spirit` redirige `output_log`/`error_log` vers **le même chemin** :
deux écrivains, deux descripteurs indépendants, un seul inode. La conclusion (« un
fichier, jamais tourné ») est inchangée ; ce qui change est la sonde qui la précède —
voir E6, dont la version initiale comptait un descripteur au singulier.

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

**La sonde porte sur _tous_ les descripteurs du fichier, pas sur un.** E5 établit que
l'appender et la redirection `output_log`/`error_log` peuvent viser le même inode
simultanément. Il suffit **qu'un seul** des descripteurs soit sans `O_APPEND` pour que le
`truncate(0)` de logrotate produise le repeuplement sparse : sonder le premier fd trouvé
et conclure serait un faux négatif silencieux. D'où la boucle plutôt que le `grep` unique
de la version initiale :

```bash
pid=$(pgrep -f 'mika-spirit')
for fd in /proc/$pid/fd/*; do
  case "$(readlink "$fd")" in
    *server.log|*mika-spirit.log)
      printf '%s -> %s : ' "$fd" "$(readlink "$fd")"
      grep ^flags "/proc/$pid/fdinfo/$(basename "$fd")"   # attendu : bit 0o2000 (O_APPEND)
      ;;
  esac
done
```

Le flag est en **octal** : `O_APPEND` = `0o2000`, donc un `flags: 02101001` est conforme
et un `flags: 02100001`... l'est aussi — lire le chiffre, pas la longueur. Le test exact
est `(flags & 0o2000) != 0` sur **chaque** ligne rendue, et la sonde n'est concluante que
si elle rend **au moins une** ligne : zéro ligne signifie que le pid ou le motif est faux,
pas que la précondition est satisfaite.

`tracing_appender` ouvre avec `OpenOptions::append(true)` — donc `O_APPEND` — et
`supervise-daemon` fait de même pour ses redirections. Les deux sont **attendus**
conformes ; la sonde existe pour que ce soit constaté plutôt que supposé, parce qu'un
faux sur ce point transforme le correctif en aggravation.

### E6b — Deux directives logrotate ne font pas ce que leur nom suggère, et la première version de ce plan s'y est prise

Trois pièges de syntaxe, dont deux étaient présents dans la version initiale du volet A1
de ce plan. Ils sont nommés ici parce qu'aucun ne produit d'erreur : un fichier logrotate
qui les porte s'installe, se recharge et *paraît* correct.

**(a) `size` est mutuellement exclusif avec `daily`, et le dernier lu gagne.** Le manuel
range `hourly` / `daily` / `weekly` / `monthly` / `yearly` / `size` dans un même groupe de
directives dont *la dernière rencontrée l'emporte, les précédentes étant ignorées*. Un
bloc portant `daily` **puis** `size 200M` ne tourne donc **pas** quotidiennement : seule la
taille compte. La version initiale de A1 commentait l'inverse (« logrotate tourne dès
qu'une des deux conditions est remplie ») — c'est la sémantique de **`maxsize`**, pas celle
de `size` :

> `maxsize size` — *Log files are rotated when they grow bigger than size bytes even before
> the additionally specified time interval.*

La directive voulue est donc `maxsize`. Le coût de l'erreur est modeste mais réel et
silencieux : un journal calme ne tourne jamais, donc `rotate 14` ne s'applique jamais, et
la rétention annoncée n'existe pas tant que le seuil n'est pas franchi.

**(b) `create` n'a aucun effet en présence de `copytruncate`.** Le fichier d'origine reste
en place — il n'est jamais renommé — donc il n'y a rien à recréer. La version initiale de
A1 posait les deux, ce qui **contredisait son propre test V4** (lequel exige l'absence de
`create`). La contradiction est levée en retirant `create` : c'est `copytruncate` qui est
porteur, et les permissions du fichier courant sont préservées par construction.

**(c) `delaycompress` est superflu ici.** Il existe pour le cas où un écrivain continue
d'écrire dans l'archive après rotation. Avec `copytruncate` l'archive est une copie close
dès sa création : personne n'y écrit jamais. Le garder ne casse rien mais laisse une
archive non compressée sans raison.

**Ce point n'est pas vérifiable depuis le worktree** — `logrotate` n'est pas installé dans
le sandbox du pilote et la documentation en ligne n'y est pas joignable. Les trois
affirmations ci-dessus sont donc à **confirmer par exécution** avant installation, ce qui
est de toute façon l'étape A5 :

```bash
logrotate --debug packaging/logrotate/mika    # n'écrit rien ; affiche le plan de rotation
```

La sortie doit nommer une rotation périodique **et** le seuil de taille. Si elle ne
mentionne que la taille, la lecture (a) est confirmée et le fichier porte encore `size`.

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

### E9 — Il y a **trois** déploiements, et deux d'entre eux écrivent au même chemin sous deux propriétaires différents

La première version de ce plan a raisonné sur un seul chemin, `/var/log/mika/server.log`,
et a qualifié sa configuration de « hors dépôt, lue pendant l'étude ». La deuxième en a
trouvé un second sous `os/`. Il y en a **trois**, et c'est le troisième — le paquet
Debian, que les deux versions précédentes ont traité comme un cas hypothétique — qui rend
le volet A non installable en l'état (E9b) :

| | gentux (poste opérateur) | Debian / systemd (`packaging/`, versionné) | mika-os (`os/`, versionné) |
|---|---|---|---|
| Utilisateur | `samidarko` | `mika` (`mika-spirit.service`, `User=`) | `mika:mika` (`init.d/mika-spirit`) |
| `MIKA_SPIRIT_LOG_FILE` | posé hors dépôt | **`/var/log/mika/server.log`** (posé dans l'unité, `Environment=`) | `/home/mika/.mika/logs/mika-spirit.log` (`conf.d/mika-spirit:11`) |
| `output_log` / `error_log` | `/var/log/mika/server.log` | s.o. (systemd, pas de redirection) | **le même** `/home/mika/.mika/logs/mika-spirit.log` (`init.d:15-16`) |
| Log gateway | `/var/log/mika-gateway/` (supposé, cf. A1) | pas d'unité gateway (`packaging/systemd/` n'en contient qu'une) | `/home/mika/.mika/logs/mika-gateway.log` (`conf.d/mika-gateway:31`) |
| Dans la CI | non | non | **oui** — 5 cibles buildées (`ci.yml:385-395`) |
| logrotate applicable ? | oui | oui | **non** (E10) |

Le paquet Debian n'est pas une hypothèse de packaging : son `postinst` crée
`/var/log/mika` en `mika:mika` (`install -d -m 0755 -o mika -g mika`) et son unité y dirige
le journal. C'est le seul des trois où la rotation est à la fois applicable **et**
automatisable par le dépôt lui-même.

Et ce second déploiement n'est pas une relique de démonstration : `os/README.md` nomme
`mika-runtime-server` « mika-cloud per-customer agent container » et `mika-runtime-gateway`
« mika-cloud gateway deployment ». **C'est l'image qui sert les tenants** — ceux-là mêmes
dont mika#2290 (tenant cloud d'Al) et mika#2023 (tenant champion) décrivent l'exploitation.

Conséquence directe sur le volet A : les deux globs de sa version initiale
(`/var/log/mika/*.log`, `/var/log/mika-gateway/*.log`) ne couvrent **que gentux**. Le
déploiement versionné, buildé en CI et servi aux tenants, n'est couvert par aucun. Et
`missingok` — présent à juste titre — fait que le fichier s'installe **sans erreur** sur
une machine où il ne fait rien. C'est exactement la classe de panne que E6b nomme pour la
syntaxe : *le fichier reste valide et la rotation paraît configurée.*

### E9b — Un même glob, deux propriétaires : `su` ne peut pas être résolu dans un fichier unique

`/var/log/mika/*.log` est écrit par **deux** des trois déploiements, sous **deux**
utilisateurs : `samidarko` sur gentux, `mika` sous Debian/systemd. La version précédente
du volet A posait `su samidarko samidarko` sur ce glob dans un fichier annoncé comme
versionné et unique, puis mentionnait en A2 « la variante `su mika mika` pour ce paquet ».
Les deux moitiés ne décrivent pas le même artefact, et la contradiction n'est pas
rattrapable par la syntaxe :

- **`su` est par bloc**, donc il faudrait deux blocs pour deux propriétaires ;
- **mais logrotate refuse qu'un même chemin apparaisse dans deux blocs** — il échoue sur
  `duplicate log entry for /var/log/mika/*.log` et **n'exécute aucune des deux
  définitions** du fichier concerné ;
- et `missingok` ne rattrape ni l'un ni l'autre : le plan le dit déjà lui-même — *il tait
  l'absence de fichier, pas un refus de permission*.

Autrement dit, sur ce glob, le fichier versionné doit choisir **un** propriétaire. Il ne
peut pas servir gentux et le paquet Debian à la fois.

**Et `su` n'est peut-être pas nécessaire du tout.** Il ne sert qu'à faire opérer logrotate
sous une identité non-root dans un répertoire dont il refuserait autrement les permissions
(« parent directory has insecure permissions »). logrotate tourne en root sous cron ;
`/var/log/mika` est en `0755 root`-lisible dans les deux cas. Poser un `su` faux est donc
plus dangereux que ne pas en poser, puisqu'il abaisse délibérément les privilèges vers un
utilisateur qui peut ne pas exister sur la machine (`samidarko` n'existe pas sur une
install Debian, et logrotate échoue alors sur l'utilisateur inconnu).

**Non vérifiable depuis le worktree** (logrotate absent du sandbox, comme en E6b). Les
trois affirmations ci-dessus — refus du chemin dupliqué, échec sur utilisateur inconnu,
inutilité de `su` sous root — sont donc à **confirmer par `logrotate --debug`** (A6) avant
installation. La conception de A1 ci-dessous est bâtie pour être juste dans les deux cas :
elle ne pose aucun chemin en double, et elle traite `su` comme un ajustement
d'installation plutôt que comme un invariant versionné.

### E10 — Dans les conteneurs, logrotate ne peut pas fonctionner, et la voie de repli est fermée

Le remède du volet A n'est pas seulement mal ciblé sur mika-os (E9) : il y est
**inapplicable**, pour deux raisons indépendantes, et une troisième ferme la sortie de
secours habituelle.

1. **Le binaire n'y est pas.** `grep -n 'logrotate\|cron\|fcron\|dcron\|anacron' os/Dockerfile
   os/scripts/mika-os-setup os/init/mika-os-init.sh` ne rend **rien**. Ni `app-admin/logrotate`
   dans les `emerge`, ni paquet cron.
2. **Rien ne le déclencherait.** logrotate n'est pas un démon : il est appelé par cron. Un
   conteneur sans cron ne tourne aucun journal, même avec le binaire installé.
3. **Et la plateforme ne peut pas prendre le relais.** Le réflexe en conteneur — laisser
   `docker logs` / le collecteur K8s gérer la rétention — exige que le journal sorte sur
   stdout. Or `conf.d/mika-spirit` pose `MIKA_SPIRIT_LOG_FILE`, donc mika#2195 **retire la
   couche stdout** (`json_stdout_layer_enabled(true) == false`). Sur cette image, `docker logs`
   est vide et le journal s'accumule dans la couche d'écriture du conteneur, invisible au
   dehors.

Le déploiement cloud garde donc `Rotation: None` **sans remède dans ce plan**. Ce n'est pas
un oubli à rattraper en ajoutant un glob : le fermer suppose une décision d'image
(installer logrotate + un déclencheur périodique OpenRC, ou retirer `MIKA_SPIRIT_LOG_FILE`
du `conf.d` pour rendre le journal au collecteur — ce qui touche l'invariant mika#2195 et
toutes les sondes qui lisent un fichier). Voir D6 : c'est un suivi, et la Definition of Done
ci-dessous est bornée en conséquence plutôt que d'annoncer un défaut clos partout.

### E11 — La politique de rotation est écrite sur **deux** surfaces documentaires, et `docs-sync` n'en garde qu'une

Les versions précédentes de ce plan ont traité `docs/runtime-structure.md` comme la seule
doc portant la rotation, et A5 comme le seul risque de divergence. Il y en a une seconde,
et elle échappe à la garde :

```
crates/mika-agent/CLAUDE.md § Log Sinks — colonne « Rotation » (deux lignes)
  | **Server log**        | … | None — single file via `tracing_appender::rolling::never` |
  | **Per-agent CLI log** | … | Daily via `tracing_appender::rolling::daily`              |
```

**L'ancre est le titre de section, pas un numéro de ligne — et ce plan vient de payer le
contre-exemple.** Sa version précédente citait `:1799-1802` ; à la re-vérification (cf. la
section *Re-vérification* ci-dessous) ces deux lignes sont à `:1844-1845`, soit **45 lignes
de dérive en une journée**, sans que le tableau lui-même ait changé d'un caractère. Ce
`CLAUDE.md` est la surface la plus fréquemment augmentée du dépôt : tout repère numérique
qu'on y pose est périmé avant d'être lu. A3b se repère donc sur `§ Log Sinks` et sur le
contenu des cellules, jamais sur une ligne.

Ce tableau affirme exactement ce que les volets A et C rendent faux. Or
**`crates/mika-agent/CLAUDE.md` n'est pas dans `scripts/sync-agent-docs.sh`** : sa liste
`DOCS=()` (lignes 13-23) énumère neuf fichiers de `docs/`, et ce `CLAUDE.md` n'en fait pas
partie — il n'est la copie de rien, c'est une source autonome. Le job CI `docs-sync`
(V9/A5) ne peut donc pas le voir.

Trois conséquences, dans l'ordre de gravité :

1. **A3 et C2 corrigés sans lui laissent le dépôt affirmant deux politiques contradictoires** —
   ce qui est précisément ce qu'AC6 interdit, et dans le sens le plus coûteux : la ligne
   `None — single file via rolling::never` resterait exacte *sur le code* tout en étant fausse
   *sur le comportement*, puisque la rotation viendrait désormais de logrotate, hors du code.
2. **C'est la surface la plus lue — par l'agent lui-même.** Claude Code charge le `CLAUDE.md`
   du répertoire courant : quiconque travaille dans `crates/mika-agent/` reçoit ce tableau
   en contexte, et pas `docs/runtime-structure.md`.
3. **Aucune garde ne rougit sur l'oubli.** Contrairement à A5, où `docs-sync` est un filet
   préexistant, ici l'oubli est silencieux — la classe de panne que ce plan nomme partout
   ailleurs.

Le geste est une ligne de tableau et une phrase ; ce qui manquait n'était pas l'effort mais
le fait de savoir que ce fichier existe. D'où A3b, et la ligne correspondante en DoD.

---

## Re-vérification (2026-09-19, re-groom)

Ce ticket a été re-dispatché après deux groomings soldés `PIPELINE_INCOMPLETE`
(`_arch_ask failed` — le plan n'a jamais été relu par l'architecte ; c'est une panne de
pipeline, pas un verdict sur le contenu). Avant d'y ajouter quoi que ce soit, **toutes les
affirmations porteuses ont été rejouées contre HEAD**, parce qu'un plan écrit la veille et
relu le lendemain sur une base qui a bougé est exactement ce qui produit un ITERATE
évitable.

| Affirmation | Sonde | Résultat |
|---|---|---|
| E1 — six sites de production, tous `debug!`, zéro `info!` | `grep -rn 'llm re[qs]...t body' crates/` | **tient** — 8 lignes (6 production + 2 dans `tui_llm_body_capture.rs`), 0 `info!` |
| E2 — une seule directive arme la cible | `grep -rn 'llm_debug=debug' crates/` | **tient** — `logging.rs:375` et `:504`, inchangés |
| E3 — la mesure du 2026-09-03 (517/517) | `auto_pull.rs` | **tient** — `:1129` (commentaire) et `:6492` (message de test) |
| E5 — `rolling::never` côté serveur, `daily` côté agent | `grep -rn 'rolling::' crates/` | **tient** — `never` à `:393`/`:427`, `daily` à `:517`/`:541`. Quatre occurrences, comme énoncé en C1 |
| E5 — six lignes `Rotation` dont quatre `None` | `docs/runtime-structure.md:240-247` | **tient** — lignes exactes |
| E5 — aucune rétention par-agent | `grep -rn max_log_files crates/` | **tient** — zéro résultat |
| E9 — trois déploiements, deux propriétaires sur `/var/log/mika/` | `packaging/systemd/mika-spirit.service`, `os/openrc/*` | **tient** — `User=mika` + `Environment=MIKA_SPIRIT_LOG_FILE=/var/log/mika/server.log` (`:8`, `:15`) ; mika-os pose `/home/mika/.mika/logs/mika-spirit.log` en `conf.d:11` **et** en `init.d:15-16` (les deux écrivains du même inode, E5) |
| E10 — aucun logrotate nulle part | `grep -rn logrotate packaging/ docs/ scripts/ os/` | **tient** — les seules occurrences sont dans ce plan |
| E11 — `crates/mika-agent/CLAUDE.md` hors de `sync-agent-docs.sh` | `sed -n '10,26p' scripts/sync-agent-docs.sh` | **tient** — neuf fichiers de `docs/`, ce `CLAUDE.md` n'en fait pas partie |
| A1 note gateway — `gateway_log_file` sans défaut | `crates/mika-gateway/src/settings.rs:74` | **tient** — `Option<String>`, `None` dans les constructeurs de test |
| V4 — le pattern `check-*` / `test-check-*` existe | `ls scripts/` | **tient** — six `check-*`, trois `test-check-*` ; le garde de V4 s'insère dans une famille établie, il n'en fonde pas une |
| E11 — **numéros de ligne** du tableau § Log Sinks | lecture directe | **a dérivé** : `:1799-1802` → `:1844-1845` en une journée. Corrigé en ancrant sur le titre de section (E11, A3b) |

**Une seule dérive, et elle n'invalide rien** — elle confirme au contraire le point d'E11 :
la surface la plus lue du dépôt est aussi la plus mouvante, donc la plus mal repérée par un
numéro. Le reste du plan est inchangé par cette passe.

**Ce que la re-vérification ne peut toujours pas établir**, et qui reste donc au volet 0 :
tout ce qui exige la machine de production. `/var/log/mika/` et `~/.mika/.env` sont hors du
sandbox du pilote, `logrotate` n'y est pas installé. Aucune des quatre questions du volet 0
n'a pu être avancée d'un pouce entre les trois passes de grooming, et ce n'est pas un
manque de diligence : c'est la frontière du worktree. **Le plan ne peut pas être « fini »
au sens où sa première étape est un geste opérateur.**

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

### D6 — Le conteneur est **nommé et renvoyé en suivi**, pas couvert en silence

E9/E10 établissent que le déploiement versionné n'est ni visé par les globs, ni capable
d'exécuter logrotate, ni rattrapable par la plateforme. Trois réponses possibles, et la
troisième est retenue.

1. *Ajouter le glob `/home/mika/.mika/logs/*.log` et considérer le cas clos.* **Refusé** :
   le glob est nécessaire (A1 le porte désormais) mais il ne suffit pas — sans binaire ni
   déclencheur dans l'image, il décrit une rotation qui n'aura pas lieu. Ce serait
   remplacer une absence de rotation par une **apparence** de rotation, ce qui est la
   forme la plus coûteuse du défaut : le tableau de `runtime-structure.md` cesserait de
   dire `None` sans que rien ne tourne.
2. *Installer logrotate + un déclencheur dans `os/Dockerfile`, ici.* **Refusé pour ce
   ticket** : c'est une modification de l'image de production des tenants (un paquet, un
   service périodique OpenRC, et le choix entre ça et la bascule stdout qui rouvre
   mika#2195). Le ticket est un p3 d'hygiène et l'opérateur l'a borné en ces termes
   (« ticket substrat borné »). Une refonte d'image n'y tient pas.
3. **Retenu : le glob est ajouté _et_ le trou est écrit** — dans le plan (E10), dans
   `docs/runtime-structure.md` (A3, qui doit dire *quel* déploiement la politique couvre),
   et dans un ticket de suivi ouvert avec ce constat en main.

Ce que cela coûte, nommé : à l'issue de ce travail, gentux est borné et les tenants cloud
ne le sont pas. C'est une amélioration partielle, dite comme telle, et non un défaut clos.

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

# Q4 — où le gateway écrit-il réellement ? (décide le glob gateway de A1)
ls -l /var/log/mika-gateway/ /var/log/mika/ 2>&1
tr '\0' '\n' < /proc/$(pgrep -f mika-gateway)/environ | grep MIKA_GATEWAY_LOG_FILE
#   variable PRÉSENTE => ce chemin est le journal ; le porter au glob
#   variable ABSENTE  => `gateway_log_file` est un Option sans défaut (settings.rs:74) :
#                        AUCUN fichier n'est ouvert. Le journal est là où le launcher
#                        redirige — lire `output_log` dans l'init script, et porter CE
#                        chemin. Ne pas conclure du répertoire vide que le chemin diffère.

# Q5 — quel déploiement est devant moi ? (E9 : il y en a TROIS)
stat -c %U /proc/$pid                    # samidarko => gentux ; mika => Debian ou mika-os
tr '\0' '\n' < /proc/$pid/environ | grep MIKA_SPIRIT_LOG_FILE
#   /var/log/mika/server.log   + user mika       => paquet Debian/systemd
#   /home/mika/.mika/logs/...  + user mika       => mika-os (E10 : non couvert)
command -v logrotate && ls /etc/cron*/ 2>/dev/null | head
#   ^ absence des DEUX => E10 : le fichier de A1 ne tournera pas ici, quoi qu'on installe

# Q6 — faut-il un `su` ? (E9b : il n'est PAS dans le fichier versionné)
stat -c '%U %G %a' /var/log/mika /var/log/mika-gateway /home/mika/.mika/logs 2>/dev/null
logrotate --debug packaging/logrotate/mika 2>&1 | grep -i 'insecure\|skipping\|duplicate'
#   ^ « insecure permissions » => ajouter `su <proprietaire constate>` À L'INSTALLATION
#   ^ « duplicate log entry »  => un glob a ete dedouble : corriger le fichier, pas le su
```

Branches :

| Observation | Lecture | Conséquence sur ce plan |
|---|---|---|
| Q2 rend `MIKA_LOG_LLM_BODIES` armé **et** Q1 montre les bodies majoritaires | E3 confirmé, diagnostic du ticket validé quant à la cause | Volet B = retirer la ligne + redémarrer. Volets A/C inchangés |
| Q2 rend le flag **absent ou désarmé** | Alors les 22 Go ne viennent **pas** des bodies, et tout le diagnostic du ticket tombe | **Halte.** Ne pas toucher au flag. Mesurer quel `event` domine (`jq -r .event \| sort \| uniq -c \| sort -rn` sur un échantillon) et rouvrir le diagnostic ; le volet A reste valide et suffisant pour le ticket |
| Q3 montre `O_APPEND` **absent** | `copytruncate` repeuplerait le fichier de zéros (E6) | **Halte sur le volet A.** Ne pas installer le fichier logrotate en l'état ; l'alternative est `create` + redémarrage planifié du service à chaque rotation, qui est une autre décision |
| Q1 montre `llm request body` ≈ 0 mais le fichier ≫ 1 Go | Croissance par volume nominal, pas par bodies | Le volet A est **à lui seul** la réponse au ticket ; le volet B devient sans objet et doit être dit tel |
| Q5 rend `logrotate` **et** cron absents | Déploiement conteneur (E10) | **Le volet A n'y est pas applicable.** Ne pas installer le fichier en croyant avoir borné ce journal ; c'est le suivi de D6 |

### Volet A — Rotation et rétention du log serveur *(le corps du travail)*

**A1.** Créer `packaging/logrotate/mika` :

```
# Rotation des journaux Mika. Voir docs/runtime-structure.md § Log File Locations.
#
# copytruncate est OBLIGATOIRE, pas un choix de style : l'appender (tracing_appender)
# et supervise-daemon gardent leur descripteur ouvert et aucun des deux ne sait rouvrir
# sur signal. Un `create` les laisserait écrire dans l'inode renommé — le fichier
# courant resterait vide pour toujours, en silence. Ne PAS ajouter `create` : sans
# effet ici, et contredit la garde CI.
#
# maxsize (et non size) : seul maxsize se compose avec `daily`. Avec `size`, `daily`
# serait ignoré et un journal calme ne tournerait jamais.
#
# Aucun `su` : logrotate tourne en root sous cron et les répertoires visés sont
# root-traversables. Un `su` nommant un utilisateur absent de la machine fait échouer
# le bloc. Si logrotate signale « parent directory has insecure permissions », ajouter
# le `su` correspondant au propriétaire CONSTATÉ (Q4/Q5 du volet 0) — c'est un
# ajustement d'installation, pas une valeur versionnable : le même chemin est écrit
# par `samidarko` sur gentux et par `mika` sous Debian (E9b).

/var/log/mika/*.log /var/log/mika-gateway/*.log /home/mika/.mika/logs/*.log {
    daily
    maxsize 200M
    rotate 14
    compress
    missingok
    notifempty
    copytruncate
}
```

**Un seul bloc, et c'est E9b qui l'impose.** La version précédente en posait deux, pour
porter deux `su`. C'est non installable : `/var/log/mika/*.log` est écrit par gentux
(`samidarko`) **et** par le paquet Debian (`mika`, via `Environment=MIKA_SPIRIT_LOG_FILE`
dans l'unité), donc les deux blocs auraient dû se partager le même glob — ce que logrotate
refuse (`duplicate log entry`), en n'exécutant **aucune** des deux définitions. Le fichier
versionné porte donc la forme qui vaut partout, et `su` redevient ce qu'il est : une
correction locale, à poser seulement si `logrotate --debug` la réclame.

`missingok` reste porteur — il rend chaque glob inoffensif là où il ne correspond à rien,
ce qui est le cas de deux globs sur trois dans chaque installation.

Notes de conception, à porter en commentaire dans le fichier :
- `copytruncate` est **obligatoire** et non un choix de style — voir D2/E6 ; le
  remplacer par `create` casse silencieusement les deux écrivains. Corollaire (E6b-b) :
  **ne pas** ajouter de directive `create`, elle serait sans effet et contredirait V4.
- `maxsize 200M` **et non `size 200M`** : seul `maxsize` se compose avec `daily` pour
  donner « tourne à l'échéance **ou** au dépassement ». Avec `size`, `daily` serait
  ignoré (E6b-a) et un journal calme ne tournerait jamais.
- Pas de `delaycompress` : superflu avec `copytruncate` (E6b-c).
- `rotate 14` + `compress` : la borne dure. À 200 Mo par archive compressée ~10×, le
  plafond est de l'ordre de quelques centaines de Mo — contre 22 Go aujourd'hui.
- Pas de `su` dans le fichier versionné (E9b). Le propriétaire du **même** chemin diffère
  d'une installation à l'autre — `samidarko` sur gentux (`command_user="samidarko"` dans
  l'init OpenRC), `mika` sous Debian (`User=mika` dans l'unité, et `postinst` qui crée
  `/var/log/mika` en `mika:mika`). Un `su` versionné serait faux sur l'une des deux et
  échouerait sur un utilisateur inexistant. Le geste, s'il est nécessaire, est documenté
  en A3 et décidé sur la sortie de `logrotate --debug`.
- **Le gateway n'écrit pas dans le même répertoire, et le dépôt donne _trois_ chemins
  différents — aucun ne fait autorité.** La version initiale de ce plan affirmait que
  `/var/log/mika-gateway/gateway.log` était « la seule référence de chemin du dépôt ».
  C'est faux, et la référence retenue était la plus faible des trois :
  - `crates/mika-gateway/docs/egress-search-no-log-audit.md:290` — un **exemple de
    commande d'audit**, pas une configuration de déploiement ;
  - `os/openrc/conf.d/mika-gateway:31` — la configuration **versionnée**, qui pose
    `/home/mika/.mika/logs/mika-gateway.log` (commentée, donc un défaut, pas une garantie) ;
  - `scripts/audit-egress-no-log.sh:38` — un **défaut de repli** vers
    `$HOME/.mika/logs/mika-gateway.log`.

  Le bloc ci-dessus couvre le premier et le deuxième. `missingok` rend chaque
  glob inoffensif là où il ne correspond à rien. Le chemin réellement en vigueur reste à
  **constater** à l'installation (Q4 du volet 0) plutôt qu'à déduire de la documentation :
  le corriger dans le fichier est alors une ligne.

  **Et il y a une quatrième lecture, qui n'est pas un chemin : _aucun fichier_.**
  `gateway_log_file` est un `Option<String>` **sans valeur par défaut**
  (`crates/mika-gateway/src/settings.rs:74`, consommé en `.as_deref()` à
  `main.rs:64`). Variable non posée ⇒ le gateway n'ouvre **aucun** fichier : il écrit sur
  stdout, et ce que le launcher en fait n'est plus décidé par une variable Mika mais par
  la redirection de l'init script. C'est le cas du déploiement versionné — `conf.d/mika-gateway`
  pose la variable en **commentaire**, tandis que `init.d/mika-gateway` redirige bel et bien
  `output_log`/`error_log`.

  Conséquence sur la lecture de Q4 : constater qu'aucun `/var/log/mika-gateway/*.log`
  n'existe **ne prouve pas** que le chemin est ailleurs — cela peut vouloir dire qu'aucun
  fichier n'est configuré et que le journal gateway vit là où le launcher l'a redirigé. Les
  deux se distinguent par `tr '\0' '\n' < /proc/<pid>/environ | grep MIKA_GATEWAY_LOG_FILE`
  (déjà en Q4) : variable **absente** ⇒ lire `output_log` dans l'init script, et c'est *ce*
  chemin-là qu'il faut porter au glob. Sans cette distinction, on ajoute un glob sur un
  chemin qui n'existera jamais et `missingok` rend l'erreur muette — la même classe de
  panne que E9.

**A2.** `packaging/debian/mika-spirit.postinst` : installer le fichier **tel quel** sous
`/etc/logrotate.d/mika`, après la création de `/var/log/mika` (ligne 16). Pas de variante,
pas de réécriture de `su` — c'est ce que E9b rend impossible et que A1 a supprimé.

Ce volet est le seul des trois déploiements où le dépôt peut **automatiser** la rotation :
l'unité pose `MIKA_SPIRIT_LOG_FILE=/var/log/mika/server.log`, le `postinst` crée le
répertoire, et une machine Debian a logrotate et cron. Gentux reste un geste manuel (A3)
et mika-os reste non couvert (E10).

Le paquet ne livre **pas** d'unité gateway (`packaging/systemd/` n'en contient qu'une), donc
le glob `/var/log/mika-gateway/*.log` y est inerte — couvert par `missingok`, comme prévu.

**A3.** Doc — `docs/runtime-structure.md` : la colonne `Rotation` des quatre lignes
`None` devient une référence à la politique logrotate, avec la phrase qui manque
aujourd'hui : *sans installation de `/etc/logrotate.d/mika`, ce fichier croît sans
borne.* Plus le chemin du fichier versionné et le geste d'installation OpenRC
(`install -m 0644 packaging/logrotate/mika /etc/logrotate.d/mika`, en root), **et** la
note `su` de E9b : le fichier n'en porte pas, et il faut l'ajouter à la main si
`logrotate --debug` signale « insecure permissions » — avec le propriétaire constaté, qui
diffère selon l'installation.

**La mention doit dire _quel déploiement_ la politique couvre** (E9/E10), sans quoi elle
troque `None` — qui est vrai — contre une rotation qui n'a pas lieu dans l'image, ce qui
est pire. Formulation attendue : la politique s'applique aux installations disposant d'un
logrotate système ; **dans les images `os/Dockerfile` elle ne s'exécute pas** (ni binaire
ni cron), et le journal y reste non borné — avec le renvoi au ticket de suivi de D6.

**A3b. `crates/mika-agent/CLAUDE.md` § *Log Sinks* — la seconde surface, que `docs-sync`
ne voit pas** (E11). La colonne `Rotation` de ses deux lignes — repérées par le titre
`§ Log Sinks` et par le libellé de leur première cellule (`**Server log**`,
`**Per-agent CLI log**`), **jamais par un numéro de ligne**, qui dérive de dizaines de
lignes par jour sur ce fichier (E11) — doit dire la même chose qu'A3 et C2, avec la
distinction qui compte : la rotation du **server log**
ne vient pas du code (`rolling::never` est inchangé — V5/V7 l'exigent) mais de logrotate,
donc la cellule dit *« `rolling::never` côté code ; rotation par `/etc/logrotate.d/mika`
là où elle est installée — non exécutée dans les images `os/` (E10) »*. La ligne per-agent
passe à *« Daily, 14 jours retenus »* avec le volet C.

À faire dans le **même commit** qu'A3/C2. Ce fichier n'est la copie d'aucun autre : ne pas
le passer à `sync-agent-docs.sh`, qui l'ignore (et dont la liste n'a pas à s'allonger —
l'élargir ferait de ce `CLAUDE.md` une copie générée, ce qu'il n'est pas).

**A4.** `docs/configuration.md`, entrée `spirit_log_file` : ajouter la même mention
(c'est la page qu'un opérateur lit quand il pose la variable).

**A5. Synchroniser les copies crate-local — sinon la CI rouge, et pour une raison sans
rapport avec ce ticket.** `runtime-structure.md` et `configuration.md` sont **tous deux**
dans la liste de `scripts/sync-agent-docs.sh` (lignes 16 et 19), et le job `docs-sync` de
`.github/workflows/ci.yml:156` le rejoue sur chaque PR — sa condition n'exclut que les
branches `release/` et `release-please--`, donc **cette branche est bien couverte**. Toute
PR qui touche A3 ou A4 sans lancer le script échoue :

```bash
bash scripts/sync-agent-docs.sh      # met à jour crates/mika-agent/docs/*
git add crates/mika-agent/docs/runtime-structure.md crates/mika-agent/docs/configuration.md
```

À faire dans le **même commit** que A3/A4. C'est mécanique, mais c'est la seule étape de ce
plan dont l'oubli est garanti de bloquer la PR.

**A6. Valider le fichier avant de l'installer** (E6b) :

```bash
logrotate --debug packaging/logrotate/mika
```

Mode simulation : n'écrit rien. La sortie doit nommer une rotation périodique **et** le
seuil de taille. Si elle ne mentionne que la taille, le fichier porte encore `size` au
lieu de `maxsize`. **Non exécutable depuis le sandbox du pilote** (`logrotate` absent) —
c'est une étape de l'implémentation ou de l'installation, et V4 en porte le substitut
statique pour la CI.

### Volet B — Le flag, conditionné à la branche du volet 0

**B1.** Si le volet 0 confirme l'armement : retirer `MIKA_LOG_LLM_BODIES` de
`~/.mika/.env`, redémarrer `mika-spirit`, vérifier l'extinction par
`grep -c '"llm request body' /var/log/mika/server.log` sur la fenêtre post-redémarrage
(attendu : 0) et par l'absence du WARN `llm_body_capture` au démarrage suivant.

**B2.** Doc — `docs/configuration.md` : **créer** l'entrée `MIKA_LOG_LLM_BODIES`. Elle
n'existe pas — `grep -rn 'log_llm_bodies\|LOG_LLM_BODIES' docs/*.md` ne rend **rien**,
alors que `spirit_log_file` est documenté (ligne 414) et que `MIKA_GATEWAY_LOG_FILE` l'est
(ligne 776). La variable n'est décrite aujourd'hui qu'au `CLAUDE.md` racine (§ *runtime
observability*, enrichi par mika#2220) et en commentaire dans `.env.example:136`.

C'est un écart qui mérite d'être nommé plutôt que corrigé en passant : la page qu'un
opérateur lit pour poser une variable est justement celle qui ne mentionne pas celle-ci —
et c'est la seule dont l'armement permanent produit un journal de 22 Go. L'entrée doit
dire : le flag est **dev-only et à retirer après usage** ; armé en permanence sur un
mika-spirit il produit l'essentiel du volume du journal ; son armement est constatable par
`grep llm_body_capture $MIKA_SPIRIT_LOG_FILE` (le WARN de démarrage) ; le désarmer exige un
**redémarrage** (lu une fois par process, non-hot-swappable — même contrat que
`MIKA_AGENT_TIER` et `MIKA_DEPLOYMENT`) ; et il doit être armé **sur le process qui exécute
le tour**, qui depuis mika#1727 est mika-spirit et non le CLI.

Rédiger l'entrée **en accord** avec le `CLAUDE.md` racine, qui porte déjà la table de
vérité des valeurs acceptées et le piège « armé sur le mauvais process » : la doc de
référence doit renvoyer à cette section, pas la paraphraser de travers.

**B3.** `.env.example:136` : le commentaire dit « dev-only » ; y ajouter « — retirer
après le diagnostic ; armé en permanence, c'est le premier poste de volume du journal ».

**B4 (petit, optionnel).** `mika logs paths` expose déjà `size_bytes` pour les deux
sinks (`crates/mika-cli/src/commands/logs.rs:55,61,73,79` — quatre sites, la valeur étant
émise dans deux branches de sortie). Ajouter un avertissement au-delà
d'un seuil (1 Go) nommant la politique logrotate. C'est la surface qui répond
directement au second symptôme du ticket (« greps de diagnostic lourds »), sans créer de
surface nouvelle. **Écarté si le volet 0 branche « halte »** — inutile d'instrumenter un
symptôme dont la cause n'est pas encore établie.

### Volet C — Rétention du sink par-agent

**C1.** `logging.rs` `init_pretty`, deux sites (`:517`, `:541`) : remplacer
`tracing_appender::rolling::daily(dir, "mika.log")` par le `Builder` équivalent avec
`.max_log_files(14)`.

Ces deux sites sont les **seuls** `rolling::daily` du dépôt (`grep -rn 'rolling::' crates/`
: quatre occurrences, deux `never` en `init`, deux `daily` en `init_pretty`). Le sink
**team** est donc couvert par le même geste, sans site supplémentaire : les deux variantes
de `init_team_logging` (`crates/mika-cli/src/main.rs:432` et `:464`, `cfg(telemetry)` et
son inverse) délèguent à `init_pretty` avec `LogOutput::FileOnly`. La ligne `mika` (team
mode) du tableau de `docs/runtime-structure.md` est donc à corriger elle aussi — sans quoi
la doc dirait deux politiques de rétention là où le code n'en a qu'une.

**C2.** Le tableau de `docs/runtime-structure.md` passe de `Daily (tracing_appender)` à
`Daily, 14 jours retenus`, sur les lignes CLI **et** team mode. Suivi de A5
(`scripts/sync-agent-docs.sh`), même raison.

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
| V4 | Le fichier logrotate emploie `copytruncate`, et **aucune** des trois directives-pièges | `scripts/check-logrotate-directives.sh` + job CI dédié, **et** son test négatif `scripts/test-check-logrotate-directives.sh` — le pattern établi du dépôt (`check-byte-slices`, `check-image-tags-immutable`, `check-dispatch-seats-declared`), retenu ici plutôt qu'un test Rust : le fichier vit sous `packaging/`, hors de tout crate, et `ci.yml` porte la doctrine en toutes lettres — *« A guard nobody has watched go red is a decoration — mika#2103 »*. Le garde assert : présence de `copytruncate` ; **absence** de `create` (sans effet avec `copytruncate`, E6b-b) ; **absence** de `size ` en début de directive (qui ferait taire `daily`, E6b-a — noter l'espace, `maxsize` ne doit pas déclencher la garde) ; **aucun chemin en double** entre blocs (E9b : logrotate échoue sur `duplicate log entry` et n'exécute alors **aucune** des deux définitions) ; et **aucune directive `su`** dans le fichier versionné (E9b : le propriétaire du même chemin diffère selon l'installation, un `su` versionné est faux sur l'une des deux et échoue sur un utilisateur inexistant). Ces gardes existent parce que les cinq erreurs sont **silencieuses ou mal attribuées** : le fichier reste valide et la rotation paraît configurée. Complété par `logrotate --debug` à l'installation (A6) |
| V10 | Le fichier couvre les chemins des deux déploiements | Le même garde vérifie la présence des globs de `/var/log/mika/` **et** de `/home/mika/.mika/logs/` (E9). Sans cette ligne, une régression qui retire un glob ne se voit nulle part : `missingok` la rend silencieuse sur la machine qui n'est pas concernée |
| V5 | L'invariant mika#2195 survit | `json_stdout_layer_enabled` et ses tests sont inchangés ; le volet A ne touche pas `logging.rs` |
| V6 | La rétention par-agent borne bien | Test sur `Builder::max_log_files` : au-delà de N fichiers, les plus anciens disparaissent. Test d'intégration avec un `tempdir`, pas un test de source |
| V7 | Aucune sonde opérateur ne change de chemin | Test de source : `MIKA_SPIRIT_LOG_FILE` et `/var/log/mika/server.log` restent les seules cibles documentées ; aucun `rolling::daily` n'apparaît dans `init` (le constructeur serveur). Cette garde existe parce que la régression serait **muette** — des dizaines de `grep` rendraient zéro ligne, ce qui se lit comme régime nominal |
| V8 | La mesure du volet 0 est reproductible | Les commandes du volet 0 sont copiables telles quelles dans le corps de PR et dans `docs/runtime-structure.md` |
| V9 | Les copies crate-local des docs sont à jour | Le job CI `docs-sync` (`ci.yml:156`) rejoue `scripts/sync-agent-docs.sh` et rougit sur toute divergence. Rien à écrire : la garde existe. A5 est l'étape qui la satisfait |
| V11 | Les **deux** surfaces documentaires disent la même politique | Relecture, **pas** de garde automatique — et c'est un choix, pas un oubli. `crates/mika-agent/CLAUDE.md` n'est dans aucune liste de synchronisation (E11) ; l'y ajouter en ferait une copie générée, ce qu'il n'est pas, et écraserait ses 60 lignes de prose qui n'existent nulle part ailleurs. Un garde textuel comparant deux tableaux de formats différents serait fragile pour une divergence qui ne se produit qu'aux rares PR touchant la rotation. La contre-mesure est donc A3b dans le **même commit** qu'A3/C2, plus cette ligne pour que l'oubli soit une omission constatable en revue plutôt qu'un angle mort |

---

## Fire-Disposition

- **Portée :** packaging (nouveau fichier + `postinst`), un garde CI (`check-*` +
  `test-check-*` + job), documentation (**quatre** fichiers — `runtime-structure.md`,
  `configuration.md`, `.env.example` et `crates/mika-agent/CLAUDE.md`, ce dernier hors
  portée de `docs-sync`, E11 — plus les deux copies crate-local régénérées par A5), un
  changement de code borné (volet C, deux lignes + tests), un ajout
  facultatif au CLI (volet B4). Aucun changement au chemin d'exécution de l'agent, aucun
  changement au filtre de log, aucune nouvelle variable d'environnement, **aucune
  modification de `os/Dockerfile`** (D6).
- **Portée _non_ couverte, dite ici plutôt que découverte après coup :** les images
  `os/Dockerfile` — donc les conteneurs des tenants cloud — gardent un journal non borné
  (E10). Le travail améliore gentux et les installations à logrotate système ; il ne clôt
  pas le défaut partout.
- **Risque principal :** la précondition `O_APPEND` du volet A (E6), **sur chacun des
  descripteurs** quand deux écrivains partagent l'inode (E5/E9). Elle est vérifiable avant
  installation et la halte est écrite.
- **Risque de syntaxe :** les trois directives-pièges de E6b, toutes silencieuses.
  Couvertes par V4 (statique, en CI) et A6 (`logrotate --debug`, à l'installation).
- **Risque secondaire :** le volet C supprime des fichiers. Séparable.
- **Réversibilité :** volet A — `rm /etc/logrotate.d/mika`. Volet B — reposer la ligne
  dans `.env`. Volet C — revert d'une ligne.

---

## Definition of Done

- `packaging/logrotate/mika` existe, versionné, commenté sur le pourquoi de
  `copytruncate` **et sur les trois directives-pièges de E6b**, en **un bloc unique et
  sans directive `su`** (E9b : le même chemin a deux propriétaires selon l'installation, et
  logrotate refuse un chemin dupliqué entre blocs), validé par `logrotate --debug` (A6), et
  installé sur la machine de production après la vérification Q3.
- `scripts/check-logrotate-directives.sh`, son test négatif et son job CI existent (V4/V10)
  — le pattern `check-*` / `test-check-*` du dépôt, pas un test Rust.
- La restriction du déploiement conteneur (E10) est écrite dans `docs/runtime-structure.md`
  et un ticket de suivi est ouvert (D6). **Ce ticket ne prétend pas la clore.**
- `scripts/sync-agent-docs.sh` a été lancé et ses sorties commitées (A5) — le job CI
  `docs-sync` passe.
- `packaging/debian/mika-spirit.postinst` l'installe pour le paquet Debian.
- `docs/runtime-structure.md` ne dit plus `Rotation: None` sans dire ce qu'il faut faire
  pour que ce soit faux.
- **`crates/mika-agent/CLAUDE.md` § *Log Sinks* dit la même chose** (A3b/E11), dans le même
  commit. C'est la seconde surface portant la politique, et la seule que le job `docs-sync`
  ne peut pas garder — l'oublier laisserait le dépôt affirmer deux politiques contradictoires,
  sur la surface que Claude Code charge quand on travaille dans ce crate.
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
- **AC3 — Une rotation et une rétention existent pour `server.log`, sur les déploiements
  qui peuvent l'exécuter.** Fichier logrotate versionné, installé, avec une borne dure
  (`rotate 14` + `maxsize 200M` + `compress` — `maxsize` et non `size`, E6b-a), et sans
  changer le chemin que les sondes opérateur visent (D2, V7). Couvre `mika-gateway`, dont
  le répertoire de log est distinct, et **installable sans modification sur les deux
  déploiements qui peuvent l'exécuter** — un bloc unique, aucun `su` versionné (E9b), le
  `postinst` Debian l'installe tel quel (A2) et gentux par un geste documenté (A3).
  **Ne couvre pas les images `os/Dockerfile`** (E10) : la restriction est écrite dans la
  doc et portée par un ticket de suivi (D6), elle n'est pas passée sous silence.
- **AC4 — La cause du volume est établie par mesure, pas par inférence.** Volet 0
  exécuté, branche consignée. Si la mesure infirme le diagnostic du ticket, c'est un
  résultat à écrire, pas un échec à contourner.
- **AC5 — Le geste de désarmement est documenté et exécuté si la mesure le justifie.**
  Ligne retirée de `~/.mika/.env`, service redémarré, extinction constatée (B1).
- **AC6 — La documentation ne laisse plus croire à une rotation qui n'existe pas.**
  `docs/runtime-structure.md`, `docs/configuration.md` **et `crates/mika-agent/CLAUDE.md`
  § *Log Sinks*** à jour — **dans les deux sens** : ni « `None` » là où la politique
  s'applique, ni « rotation » là où elle ne s'exécute pas (E10). C'est la même exigence que
  celle qui a fait écarter `rolling::daily` en D2 : l'instrument ne doit pas annoncer un
  état qu'il n'a pas. La troisième surface est nommée explicitement parce qu'elle est la
  seule que `docs-sync` ne garde pas (E11) : y satisfaire est une relecture, pas une CI
  verte.
- **AC7 — Le trou du déploiement conteneur est constaté, écrit et tracé.** E9/E10 figurent
  au plan, la restriction figure dans `runtime-structure.md`, et un ticket de suivi est
  ouvert avec les deux mesures en main (`grep` logrotate/cron vide dans `os/`, et
  `json_stdout_layer_enabled(true) == false` qui ferme la voie `docker logs`).

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
- **`mika-gateway` — tranché, et ramené dans le périmètre.** Il porte le même
  `Rotation: None` sur ses deux lignes, et son log ne vit pas sous `/var/log/mika/`. Le
  dépôt donne **trois** chemins divergents et aucun ne fait autorité (voir la note de A1) ;
  le bloc unique de A1 couvre les deux plausibles, `missingok` rend chaque glob inoffensif
  ailleurs, et Q4 tranche à l'installation. Ce point n'est plus un suivi. À noter que le
  paquet Debian ne livre **aucune** unité gateway : sur ce déploiement le glob est inerte
  par construction, pas par accident.
- **Rotation des journaux dans les images `os/Dockerfile` — suivi, avec son constat.**
  E9/E10 : le déploiement versionné (et servi aux tenants via `mika-runtime-server`)
  n'exécute pas logrotate — ni binaire ni cron — et `MIKA_SPIRIT_LOG_FILE` y ferme la
  couche stdout (mika#2195), donc `docker logs` ne peut pas prendre le relais. Le fermer
  suppose une décision d'image : installer `app-admin/logrotate` + un déclencheur
  périodique OpenRC, **ou** retirer `MIKA_SPIRIT_LOG_FILE` du `conf.d` pour rendre le
  journal au collecteur — ce second choix touche l'invariant mika#2195 et toutes les sondes
  qui lisent un fichier, donc il s'arbitre pour lui-même. **Ticket de suivi**, à ouvrir avec
  les deux `grep` en main. Hors d'un p3 d'hygiène (D6).
- **Le volume nominal lui-même.** Si le volet 0 branche « halte » (flag désarmé), la
  question « quel `event` domine le journal ? » est un diagnostic distinct, avec ses
  propres arbitrages de cadence — et le `CLAUDE.md` racine porte déjà une doctrine
  explicite sur ce point (mika#2131 : agrégat par tick au journal, détail par ticket en
  `audit_events`). **Ticket de suivi**, à ouvrir avec la mesure en main.
- **Une garde de démarrage sur `MIKA_LOG_LLM_BODIES`.** Écartée par D4, et non par
  timidité : le process ne sait pas s'il est « de production », et la flotte a déjà payé
  le prix d'un refus de démarrage mal calibré. Si le flag se révèle armé une seconde
  fois, la question se rouvre — avec la récurrence comme argument.

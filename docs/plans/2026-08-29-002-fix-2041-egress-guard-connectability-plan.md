---
title: "La garde d'egress doit constater, pas affirmer — connectabilité au lieu de [ -S ]"
date: 2026-08-29
issue: senara-solutions/mika#2041
branch: bug/2041/dispatch-egress-une-socket-unix
type: fix
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
depth: standard
risk: high
---

# La garde d'egress doit constater, pas affirmer

## Goal Capsule

- **Objectif (résultat).** Qu'un pilote ne parte jamais silencieusement sans egress. Aujourd'hui la seule situation où le repli `fs-only` est nécessaire est précisément celle où il est inatteignable — le substrat tombe en panne sans le dire, et personne ne peut diagnostiquer une panne qui ne laisse pas de trace.
- **Moyen (approche retenue).** Remplacer le test d'existence de fichier par un test de connectabilité dans la garde d'attente de `dispatch-lib`, en réutilisant la sonde `socket.connect()` déjà présente dix lignes plus haut ; et faire délier au proxy sa propre socket à l'arrêt pour tarir la source d'orphelins.
- **Autorité.** Le corps de mika#2041 fixe l'intention. Le commentaire d'évidence du 2026-08-29 sur ce même ticket amende le diagnostic causal ; en cas de désaccord entre les deux, le commentaire prime car il est mesuré sur `origin/main` @ `2d7dfc74`.
- **Conditions d'arrêt.** S'arrêter et surfacer si l'implémentation découvre que la garde corrigée ne peut pas atteindre `fs-only` sur socket fantôme — cela signifierait que le modèle de panne est encore mal compris, et le correctif serait vide.

---

## Product Contract

### Résumé

Une garde de `dispatch-lib.sh` répond à la mauvaise question. Elle demande « un fichier de type socket existe-t-il » là où la question est « quelqu'un écoute-t-il ». Une socket unix orpheline — le fichier qu'un `kill` laisse derrière lui, puisqu'un arrêt de processus ne délie pas le chemin — satisfait la mauvaise question et pas la bonne. La garde conclut au succès, `dispatch-lib` déclare l'egress lancé, et le pilote part sans egress. Le repli `fs-only`, seul signal prévu, est court-circuité par la même erreur.

### Problem Frame

Incident du 2026-08-29 entre 06:07 et 06:11 CEST. Le proxy d'egress hôte a été arrêté délibérément. `dispatch-lib.sh` est censé le relancer au dispatch suivant ; il a respawné un proxy à chaque dispatch, chaque proxy est mort, et il a déclaré le lancement réussi à chaque fois. Preuve mesurée : zéro ligne `host-unix listening on` et zéro message `fs-only` sur 1569 lignes de `/var/log/mika/pilot-egress-proxy.log`, alors qu'un fichier socket était bien présent à `/tmp/mika-pilot-egress.sock` avec la mtime du proxy tué.

Le défaut est en `skills/bundled/_shared/dispatch-lib.sh:212-219`. La boucle d'attente et sa garde de sortie testent toutes deux `[ -S "$_PILOT_EGRESS_SOCK" ]`. Une socket fantôme satisfait ce prédicat : la boucle sort à la première itération, la garde de repli ne se déclenche pas, `return 0`. Le fichier sait pourtant poser la bonne question — la sonde de vie en `:186-196` fait un véritable `socket.connect()` sur `AF_UNIX`.

Le proxy, lui, ne délie pas sa socket à l'arrêt : `scripts/mika-pilot-egress-proxy` n'installe aucun gestionnaire de signal (`main():806-814` n'attrape que `KeyboardInterrupt`). C'est la source des orphelins qui arment la garde défaillante.

**Correction d'évidence portée depuis le commentaire du ticket.** Le corps de mika#2041 affirme qu'« un nouveau proxy qui tente de `bind()` sur ce chemin occupé échoue et meurt ». Cette chaîne causale n'est pas soutenue par le code : `scripts/mika-pilot-egress-proxy:742-750` délie déjà une socket éventée avant de binder, et ce code était déployé pendant l'incident (`~/.local/bin/mika-pilot-egress-proxy` mesuré identique au dépôt ; introduit par `e4f24677`, #1894). La raison pour laquelle les proxies de 06:07:57 et 06:10:05 sont morts avant de binder **reste inconnue et hors périmètre**. Elle n'affaiblit pas ce correctif : c'est exactement la garde corrigée qui rendra cette classe bruyante au lieu de silencieuse, donc diagnosticable.

### Requirements

**Garde de lancement**

- R1. La boucle d'attente qui suit le lancement du proxy teste la connectabilité de la socket, jamais la seule existence d'un fichier de type socket.
- R2. La garde de sortie de cette boucle teste la même propriété que la boucle, de sorte qu'un échec de lancement produit toujours le message `fs-only` sur stderr et un code de retour non nul.
- R3. La sonde de connectabilité est définie une seule fois et utilisée par la sonde de vie comme par la garde d'attente. Deux définitions divergentes de « vivant » sont la forme même du défaut corrigé ici.

**Cycle de vie de la socket**

- R4. En mode `--host-unix`, le proxy délie sa socket avant de sortir sur `SIGTERM` et sur `SIGINT`.
- R5. En mode `--sandbox-tcp`, le proxy ne délie rien : il ne possède pas la socket unix, il s'y connecte.

**Visibilité de l'échec**

- R6. Le repli `fs-only` est atteignable dans le scénario de la socket fantôme, et un test le démontre en l'exerçant — pas en inspectant le source.
- R7. Le comportement déjà correct de déliaison-avant-bind du proxy est verrouillé par un test de régression, sans duplication du code qui l'implémente.
- R8. Le répertoire de log du lanceur est surchargeable, de sorte qu'une exécution de la suite de tests n'écrive jamais dans `/var/log/mika/pilot-egress-proxy.log`. Ce fichier est la surface de diagnostic d'incident ; une suite de tests qui y injecte la sortie de binaires factices détruit la preuve qu'on cherche à rendre lisible.
- R9. Le message de repli porte un jeton stable et greppable, pour qu'un opérateur puisse le chercher comme il cherche les autres signaux documentés du dépôt.

### Scope Boundaries

**Hors périmètre**

- **La posture de repli elle-même reste `fail-open`, et ce plan ne la relitige pas.** Quand le proxy d'egress ne démarre pas, `dispatch-lib` ne renonce pas au dispatch : il retombe en Phase 2a — coupure du système de fichiers conservée, **réseau ouvert** (`dispatch-lib.sh:177-179`). Ce correctif rend cet état visible ; il ne le rend pas sûr. Pour une couche de containment dont l'objet est une allowlist de noms d'hôtes sur un agent autonome, « échouer ouvert bruyamment » plutôt que « échouer fermé » est une décision de sécurité réelle, héritée de #1894 et hors du mandat de mika#2041. Elle mérite son propre ticket ; la nommer ici évite qu'elle se transmette par silence.
- La cause de la mort des proxies de 06:07:57 et 06:10:05 avant leur `bind()`. Inconnue, non devinée. Ce plan la rend observable au prochain passage ; il ne prétend pas l'expliquer.
- Le bruit `[egress] ERROR unknown: Connection lost` dans le log. Auto-infligé par la sonde de vie qui se connecte puis ferme sans parler HTTP ; c'est une gêne de lisibilité, pas une panne, et elle relève d'un autre ticket.
- Toute modification de l'allowlist de noms d'hôtes, du chemin MITM ou de la politique de containment.

**Différé à un travail de suivi**

- Rendre la sonde de vie silencieuse côté log du proxy (fermer proprement, ou reconnaître une sonde). Mérite son propre ticket avec sa propre évidence.
- Inscrire le repli `fs-only` dans la liste des signaux opérateur de `CLAUDE.md` (la série « Signal A … Signal P »). R9 donne au message un jeton stable pour qu'un tel signal soit possible ; le documenter relève d'un ticket de doc, pas de ce correctif. Sans cela, « bruyant » veut dire une ligne sur un stderr que personne ne surveille.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Factoriser une seule fonction de sonde, `_pilot_egress_sock_connectable`.** Elle prend le chemin en argument, renvoie 0 si une connexion `AF_UNIX` aboutit. La sonde de vie `:186-196` et la garde d'attente `:212-219` l'appellent toutes deux. Alternative écartée : dupliquer le heredoc python dans la boucle — c'est ce qui a produit la divergence entre les deux définitions de « vivant » qu'on corrige ; la répéter serait reconduire la cause.
- KTD2. **Passer le chemin en `sys.argv`, pas par interpolation shell dans le source python.** La sonde existante interpole `'$_PILOT_EGRESS_SOCK'` directement dans le corps python. Un chemin contenant une apostrophe casse le script. Le passage par argv élimine la classe entière ; le coût est nul.
- KTD3. **Garder la boucle `sleep 0.1` × 20 et sonder à chaque tour, plutôt que de déplacer l'échéance dans un unique processus python.** Sur une socket fantôme, `connect()` échoue immédiatement par `ECONNREFUSED` — pas de temporisation — donc le pire cas ajoute une vingtaine de démarrages de `python3` (~0,6 s) sur le seul chemin d'échec, où la latence n'a aucune importance. Sur le chemin nominal le proxy se lie en une à trois itérations. L'alternative à un seul processus est plus rapide mais introduit une deuxième forme de sonde, ce que KTD1 refuse.
- KTD4. **Arrêt du proxy par future d'arrêt plutôt que par `serve_forever()`.** `add_signal_handler` résout une future ; `run_host_mode` l'attend sous `async with server`, et délie dans un `finally`. Cela couvre l'arrêt par signal et l'arrêt par exception avec un seul chemin de nettoyage. Alternative écartée : un `atexit`, qui ne se déclenche pas sur `SIGTERM` par défaut et laisserait le défaut intact.
- KTD5. **La déliaison finale vérifie que le chemin est encore une socket avant de délier.** Si un autre proxy a déjà repris le chemin, on ne détruit pas sa socket. Le test reste minimal et volontairement non atomique : la fenêtre de course est le prix d'un nettoyage sans verrou, et elle est strictement meilleure que l'orphelin garanti d'aujourd'hui.
- KTD7. **Le répertoire de log du lanceur devient surchargeable par variable d'environnement, avec `/var/log/mika` pour valeur par défaut.** `_ensure_pilot_egress_proxy` calcule aujourd'hui `log_dir` en dur (`:200`) et y redirige la sortie du binaire lancé. U3 appelle la vraie fonction avec un binaire factice : sans cette surcharge, `make test-dispatch-lib` append la sortie de faux proxies dans le log d'incident réel. Alternative écartée : que le test accepte la pollution — inacceptable sur un fichier dont ce même plan fait sa principale source de preuve.
- KTD6. **La preuve de non-vacuité est une exécution mesurée, pas une assertion structurelle.** Le test de R6 doit être exécuté une fois contre `dispatch-lib.sh` non corrigé et échouer. Un test qui grep le source passerait sur du code mort ; c'est la faute de raisonnement même que ce ticket documente.

### Design technique

La sonde partagée s'installe dans `skills/bundled/_shared/dispatch-lib.sh`, juste avant `_ensure_pilot_egress_proxy`, forme directionnelle :

```sh
# Vrai si quelqu'un écoute réellement sur $1. Ni l'existence du fichier ni son
# type ne suffisent : une socket orpheline satisfait [ -S ] et refuse connect().
_pilot_egress_sock_connectable() {
    [ -S "$1" ] || return 1
    python3 -c '...connect(sys.argv[1])...' "$1" 2>/dev/null
}
```

Les deux sites d'appel deviennent `if _pilot_egress_sock_connectable "$_PILOT_EGRESS_SOCK"` et `while ... && ! _pilot_egress_sock_connectable ...`. Le message `fs-only` de `:217` est conservé mot pour mot : les tests et l'habitude de lecture des logs s'y accrochent.

Côté proxy, seul `run_host_mode` change. `run_sandbox_mode` et `main()` ne sont pas touchés, ce qui satisfait R5 par construction plutôt que par condition.

### Assumptions

- A1. `dispatch-lib.sh` peut être sourcé dans un test sans effet de bord : ses lignes de premier niveau sont des affectations de variables (`:93-110`), les blocs python sont contenus dans des heredocs de fonctions. Vérifié par lecture ; à confirmer à l'exécution du premier test.
- A2. Une socket unix liée puis fermée sans `unlink` laisse un fichier de type socket dont `connect()` échoue par `ECONNREFUSED`. C'est le comportement Linux standard et la fabrique de fantôme retenue pour les tests. Si la fabrique ne produit pas ce refus, le test doit échouer bruyamment plutôt que d'être adapté.
- A3. `python3` est disponible sur l'hôte de dispatch et dans le runner CI. C'est déjà une dépendance dure de `dispatch-lib.sh` aux lignes 126, 149 et 187.

### Séquencement

U1 et U2 sont indépendants et peuvent être écrits dans n'importe quel ordre. U3 dépend de U1. U4 dépend de U2. La preuve de non-vacuité de KTD6 s'exécute entre l'écriture de U3 et la validation finale.

---

## Implementation Units

### U1. La garde d'attente constate au lieu d'affirmer

- **Objectif.** R1, R2, R3, R8, R9. Une seule définition de « vivant », utilisée par la sonde de vie et par la garde d'attente ; un lanceur dont le log est détournable en test ; un message de repli greppable.
- **Fichiers.** `skills/bundled/_shared/dispatch-lib.sh`
- **Motifs à suivre.** Le heredoc python de `:186-196` est le modèle de la sonde ; le style de fonction `_`-préfixée et le format des messages `dispatch-lib: ...` >&2 sont ceux du fichier.
- **Note d'exécution.** Conserver le texte exact du message `fs-only` de `:217` et n'y ajouter que le jeton greppable de R9, en préfixe ou en suffixe — les tests et l'habitude de lecture des logs s'accrochent à la formulation existante. La surcharge de R8 garde `/var/log/mika` par défaut : aucun changement de comportement en production.
- **Scénarios de test.** Couverts par U3.
- **Vérification.** `make test-dispatch-lib` reste vert ; `bash -n skills/bundled/_shared/dispatch-lib.sh`.

### U2. Le proxy délie sa socket à l'arrêt

- **Objectif.** R4, R5. Tarir la source d'orphelins.
- **Fichiers.** `scripts/mika-pilot-egress-proxy`
- **Motifs à suivre.** `run_host_mode:740-763` ; ajouter `signal` à la liste d'imports de `:59-69`, qui est ordonnée alphabétiquement.
- **Note d'exécution.** `run_sandbox_mode` ne doit pas être modifié — c'est ainsi que R5 est satisfait par construction. Vérifier que le message `host-unix listening on` reste émis au même moment, après le `chmod`, car les diagnostics d'incident s'y appuient.
- **Scénarios de test.** Couverts par U4.
- **Vérification.** `python3 -c "import ast,sys; ast.parse(open('scripts/mika-pilot-egress-proxy').read())"` ; `make test-pilot-egress-proxy`.

### U3. Test anti-vacuité : `fs-only` est atteignable

- **Objectif.** R6. Démontrer par exécution que le repli se déclenche sur socket fantôme.
- **Fichiers.** `skills/bundled/_shared/test-dispatch-lib.sh`
- **Motifs à suivre.** Bloc `# --- Test: <intitulé> (mika#2041) ---` avec en-tête `echo "=== ... ==="`, puis `assert_eq` / `assert_contains`, à la manière des blocs de `:3175` et suivants. Répertoires temporaires nettoyés comme le font les fixtures existantes.
- **Scénarios de test.**
  - Chemin nominal du défaut : socket fantôme présente (liée puis fermée sans déliaison) et binaire proxy factice exécutable qui sort immédiatement sans binder → `_ensure_pilot_egress_proxy` renvoie un code non nul et stderr contient `falling back to fs-only`.
  - Chemin vivant : un vrai listener `AF_UNIX` maintenu ouvert sur le chemin → la fonction renvoie 0 sans lancer le binaire factice, ce qui prouve que la sonde de vie reconnaît un écouteur réel et n'a pas été cassée par la factorisation. Le « sans lancer » se constate par un témoin : le binaire factice touche un fichier marqueur, dont l'absence est l'assertion.
  - Chemin absent : aucun fichier au chemin et binaire factice qui sort sans binder → code non nul et message `fs-only`. Ce cas passait déjà avant le correctif ; il verrouille l'absence de régression.
  - Chemin binaire manquant : `_PILOT_EGRESS_PROXY_BIN` pointant sur un chemin inexistant → code non nul et message `Phase 2b network cut disabled`, distinct du message `fs-only`. Les deux replis ne doivent pas se confondre.
  - Non-pollution du log (R8) : la suite fixe le répertoire de log sur un temporaire et vérifie, après exécution, que `/var/log/mika/pilot-egress-proxy.log` n'a pas grossi. C'est le test qui protège la surface de preuve dont dépend tout ce ticket.
  - Robustesse de la sonde : chemin de socket contenant une apostrophe → la sonde ne provoque aucune erreur d'interprétation python (couvre KTD2).
- **Vérification.** `make test-dispatch-lib`. **Puis la preuve de non-vacuité de KTD6** : réappliquer temporairement l'ancienne garde `[ -S ]`, relancer la suite, constater et consigner que le premier scénario échoue, restaurer. Sans cette mesure, U3 n'est pas terminé.

### U4. Régressions côté proxy : déliaison au démarrage et à l'arrêt

- **Objectif.** R7 et la vérification de R4/R5.
- **Fichiers.** `scripts/test-pilot-egress-proxy-status.py`
- **Motifs à suivre.** Le module proxy est déjà chargé via `SourceFileLoader` en `:31-38` ; `unittest.IsolatedAsyncioTestCase` est déjà utilisé par `RelayTapTests:188`. Utiliser un chemin de socket sous `tempfile.mkdtemp()`, jamais `/tmp/mika-pilot-egress.sock`, pour ne pas toucher le proxy réel de l'hôte.
- **Scénarios de test.**
  - Déliaison au démarrage (R7) : créer une socket fantôme au chemin, lancer `run_host_mode`, constater que le chemin devient connectable. Verrouille le comportement de `:742-750` sans le dupliquer.
  - Refus de délier un non-socket : créer un fichier ordinaire au chemin, lancer `run_host_mode`, constater la sortie en erreur sans suppression du fichier. Verrouille la branche `:747-748`.
  - Déliaison sur `SIGTERM` (R4) : lancer le proxy en sous-processus sur un chemin temporaire, attendre qu'il soit connectable, envoyer `SIGTERM`, constater la sortie du processus et l'absence du chemin.
  - Idem sur `SIGINT`.
  - Le mode sandbox ne délie rien (R5) : un chemin de socket unix existant et vivant reste présent après l'arrêt d'une instance `--sandbox-tcp`.
- **Vérification.** `make test-pilot-egress-proxy`.

---

## Verification Contract

| Commande | Portée | Unités |
|---|---|---|
| `bash -n skills/bundled/_shared/dispatch-lib.sh` | Syntaxe shell | U1 |
| `make test-dispatch-lib` | Suite d'assertions dispatch-lib (câblée en CI, `ci.yml:85`) | U1, U3 |
| `make test-pilot-egress-proxy` | Suite du proxy d'egress (câblée en CI, `ci.yml:217`) | U2, U4 |
| Preuve de non-vacuité KTD6 | Le scénario socket fantôme échoue contre la garde `[ -S ]` d'origine | U3 |
| `make verify-bundled-skills` | Intégrité des skills embarqués après édition de `dispatch-lib.sh` | U1 |

---

## Definition of Done

- Les cinq commandes du Verification Contract passent, la preuve de non-vacuité incluse et consignée.
- `dispatch-lib.sh` ne contient plus aucun test de vivacité de socket fondé sur le seul type de fichier.
- La sonde de connectabilité existe en un seul exemplaire dans `dispatch-lib.sh`.
- `run_sandbox_mode` est inchangé.
- Le message `fs-only` est identique, au caractère près, à celui d'avant le correctif, au jeton greppable de R9 près.
- Le plan reconnaît que le correctif de U2 n'agit qu'après `make install` : le proxy qui tourne aujourd'hui (`~/.local/bin/mika-pilot-egress-proxy`) a été lancé depuis l'ancien code et continuera d'orphaner sa socket à l'arrêt jusqu'au déploiement. Livré n'est pas déployé ; la PR le dit explicitement plutôt que de laisser croire l'incident refermé à la fusion.
- Le plan, le correctif et la PR portent le WHY amendé : la garde affirme au lieu de constater ; la chaîne causale « bind sur chemin occupé » du corps du ticket est corrigée, pas reprise.

## Acceptance criteria

Repris des items de correction de mika#2041, amendés par le commentaire d'évidence du 2026-08-29.

- [ ] **Item 1.** La garde d'attente (`dispatch-lib.sh:212-219`) teste la connectabilité, pas `[ -S ]`, en réutilisant la sonde `socket.connect()` de `:186-196`.
- [ ] **Item 2 — déjà satisfait, à verrouiller sans réécrire.** La déliaison d'un chemin orphelin avant `bind()` existe en `scripts/mika-pilot-egress-proxy:742-750` depuis `e4f24677` (#1894), vérifiée déployée. Aucun code nouveau ; un test de régression couvre la propriété (U4).
- [ ] **Item 3.** Le repli `fs-only` est atteignable. Test anti-vacuité : socket fantôme, dispatch lancé, message `fs-only` constaté — et démontré échouant contre le code d'avant correctif.
- [ ] **Item 4.** Le proxy délie sa socket à l'arrêt via un gestionnaire `SIGTERM` (et `SIGINT`), en mode host uniquement.

## Open Questions

- Q1 (différée, non bloquante). Pourquoi les proxies lancés à 06:07:57 et 06:10:05 sont-ils morts avant d'atteindre `bind()` ? Aucune évidence ne le dit aujourd'hui. Ce plan ne la résout pas et n'invente pas de réponse ; il fait en sorte que la prochaine occurrence laisse une trace exploitable au lieu d'un silence.

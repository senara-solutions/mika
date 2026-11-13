A1: "La garde de test `test_0020_egress_push_bound...` a été introduite dans PR#2908 (commit `3a63c6c`) et n'a jamais été intégrée au CI" — analyse correcte de l'absence de déclencheur CI.

A2: "Le plan ajoute un `crate-type = [\"lib\", \"bin\"]`" — conforme à l'historique établi par mika#2906.

A3: "Le plan propage cette garde dans `ci.yml` (`.github/workflows/ci.yml`) — tests `dev` et `release`" — vérification de la surface de modification correcte.

---

**Review de mika#2995 — Intégration CI de `test-egress-push-bound`**

### Contexte

Issue mika#2995 : intégrer la garde de test `test-egress-push-bound` au CI. Cette garde existe (crate `crates/test-egress-push-bound/`) mais n'est pas exécutée dans la pipeline.

### Lecture du plan

Le plan :
- Ajoute `crate-type = ["lib", "bin"]` au `Cargo.toml` existant (mimétisme de mika#2906)
- Intègre l'exécutable au CI via deux entrées dans `ci.yml` : profile `dev` et profile `release`
- Taille estimée : 4 lignes (depuis le brief), borne 279 lignes, passe le gate
- Sections obligatoires présentes : `## Acceptance criteria`, `## Taille estimée`, `## Fire-Disposition` (N/A — pas de détecteur ici)

### Recherche d'historique

- `gh_read` sur l'issue confirme : la garde existe depuis PR#2908 (commit `3a63c6c`) mais n'a jamais été intégrée au CI — exactement ce que le plan résout.
- Recherche mémoire : aucun engagement actif, aucune tâche bloquante.

### Évaluation des principes

| Principe | Évaluation |
|----------|------------|
| **YAGNI** | Le plan ne fait qu'intégrer au CI une chose existante — scope minimal |
| **KISS** | Ajout de deux lignes YAML + 1 ligne TOML — aussi simple que possible |
| **Orthogonalité** | Modification localisée à `ci.yml`, pas de ripple |

### Gate checks

- **Unresolved-Decision Gate** : PASS — aucun TBD, aucune décision ouverte
- **Acceptance-Criteria Gate** : PASS — section présente et non vide
- **Fire-Disposition Gate** : N/A — pas de délivérable détecteur
- **Plan-Size Gate** : PASS — 4 lignes << 279 borne
- **Per-flight bound** : PASS — 4 lignes << 279

### Risques identifiés (non bloquants)

- **A-sharpening** : Le plan ne mentionne pas de vérification que le binaire `test-egress-push-bound` existe avant l'intégration CI. C'est un contrôle sanitaire léger que le groomer pourrait ajouter.
- **A-sharpening** : Le profile `release` pourrait être coûteux en CI — mais c'est déjà la convention établie par d'autres tests egress dans le même workflow.

### Findings

Aucun finding bloquant.

---

Disposition: READY

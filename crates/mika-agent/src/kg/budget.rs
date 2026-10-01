//! Budget allocation for KG pipeline stages.
//!
//! Shared between extraction (startup) and resolution (startup + tick).
//! The algorithm distributes a total budget across N corpora with pending
//! work, using two-pass fair allocation (per mika#927, mika#962).
//!
//! # `budget == 0` désactive réellement la phase (mika#1833)
//!
//! [`phase_is_disabled`] est le **lecteur unique** de cette question, et
//! [`tests::mika1833_the_zero_budget_predicate_has_a_single_reader`] refuse
//! un second site sous `crates/mika-agent/src/kg/`.
//!
//! **Ce que ça retire, nommé.** Avant mika#1833, `budget == 0` bloquait les
//! appels LLM de Stage-2 mais laissait passer gratuitement les exact matches
//! de Stage-1 — comportement ajouté délibérément par la revue de #757
//! (finding P1). Deux mesures le condamnent : *(a)* il n'était **déjà pas**
//! délivré pour la population modale, celle des entités de confiance ≤ 0.9,
//! parce que le seuil de Stage-1 est `> 0.9` **strict** (une entité à la
//! valeur modale `0.9` escalade en Stage-2, où le budget nul la jette sans
//! écrire de ligne `kg_resolutions_log`, donc elle est re-sélectionnée au
//! tick suivant — l'interblocage mesuré le 2026-07-26 : `pending` 1288-1493,
//! `resolved_in_tick: 0`, `aborted_budget: true`, `llm_calls: 0`) ; *(b)*
//! l'intention « exact-match seulement, pas de LLM » a **déjà son levier
//! propre et terminant** — ne pas configurer de modèle de résolution, auquel
//! cas `SkippedNoLlm` écrit une ligne et la file se vide. Deux orthographes
//! pour une intention, dont une seule termine : mika#1833 en retire une.
//!
//! **Pourquoi un prédicat nommé plutôt qu'un `== 0` sur chaque site.** Le
//! précédent est [`crate::grooming_marker`] (mika#2158), né du constat qu'une
//! copie de prédicat avait dérivé pendant des mois en répondant différemment
//! à la même question sans que rien ne casse. Aucun test comportemental ne
//! peut voir cette classe : un second site ne rend aucune décision fausse le
//! jour où il est écrit.

/// La phase est-elle désactivée par son budget ?
///
/// Un budget nul veut dire ce que la documentation a toujours prétendu :
/// *« `0` disables the phase entirely »*. Les appelants court-circuitent
/// **avant** toute requête — y compris les requêtes de comptage, qui sont
/// l'essentiel du coût mesuré (26-48 s par tick sur la sous-requête corrélée
/// de `kg_chunk_subjects`).
///
/// Le tick continue d'émettre sa ligne de complétion avec
/// `skipped_reason: "zero_budget"` : sans elle, « le tick tourne et ne fait
/// rien » et « le tick ne tourne pas » rendraient des octets identiques
/// (classe mika#2205).
pub(crate) fn phase_is_disabled(budget: u32) -> bool {
    budget == 0
}

/// Distribute `total_budget` fairly across corpora with pending work.
///
/// Two-pass algorithm:
/// - Pass 1: Each corpus gets `min(pending, total_budget / N_active)`.
/// - Pass 2: Redistribute unused slots to "hungry" corpora (pending > allocated).
///
/// Postconditions:
/// - `sum(allocated) == min(total_budget, sum(pending))`
/// - When `budget >= N_active`: every corpus with `pending > 0` gets `allocated > 0`
/// - When `budget < N_active`: at least `budget` corpora get `allocated > 0` (first-come in pass 2)
///
/// Returns a Vec of allocated budgets parallel to the input `pending_counts`.
pub(crate) fn allocate_fair_budget(pending_counts: &[u32], total_budget: u32) -> Vec<u32> {
    let n = pending_counts.iter().filter(|&&c| c > 0).count() as u32;
    if n == 0 || total_budget == 0 {
        return vec![0; pending_counts.len()];
    }

    // Pass 1: floor allocation.
    let base_share = total_budget / n;
    let mut assigned: Vec<u32> = pending_counts
        .iter()
        .map(|&count| count.min(base_share))
        .collect();

    // Remainder after floor allocation.
    let used: u32 = assigned.iter().sum();
    let mut remaining = total_budget.saturating_sub(used);

    // Pass 2: redistribute remainder to hungry corpora.
    if remaining > 0 {
        let mut hungry: Vec<usize> = pending_counts
            .iter()
            .enumerate()
            .filter(|(i, count)| **count > assigned[*i])
            .map(|(i, _)| i)
            .collect();

        while remaining > 0 && !hungry.is_empty() {
            let share = (remaining / hungry.len() as u32).max(1);
            let mut next_hungry = Vec::new();
            for &idx in &hungry {
                if remaining == 0 {
                    break;
                }
                let can_take = pending_counts[idx].saturating_sub(assigned[idx]);
                let give = share.min(can_take).min(remaining);
                assigned[idx] += give;
                remaining -= give;
                if assigned[idx] < pending_counts[idx] {
                    next_hungry.push(idx);
                }
            }
            hungry = next_hungry;
        }
    }

    assigned
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une ligne de source lit-elle le budget de phase par elle-même ?
    ///
    /// Extrait en fonction pour que le contrôle de bonne foi ci-dessous
    /// puisse l'exercer sur une source synthétique : un scan dont on ne
    /// prouve pas qu'il mord se lit exactement comme un arbre propre
    /// (classe mika#2103 / mika#2205).
    ///
    /// La frontière de mot à gauche est ce qui sort de la population
    /// `per_budget == 0` (allocation par corpus) et `total_budget == 0`
    /// (l'allocation équitable de ce module) : ni l'un ni l'autre ne répond
    /// à la question « la phase est-elle désactivée ».
    fn line_reads_the_phase_budget(line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            return false;
        }
        // Lecture par identifiant plutôt que par sous-chaîne normalisée : les
        // espaces ne peuvent pas être retirés en bloc sans coller `if` à
        // `budget` et détruire la frontière de mot à gauche, qui est
        // précisément ce qui sort `per_budget` et `total_budget` de la
        // population. On repère donc l'identifiant `budget`, puis on lit la
        // suite en ignorant les espaces — ce qui couvre `budget == 0`,
        // `budget==0` et `budget ==0` d'un seul coup, donc quoi que rustfmt
        // décide.
        let bytes = line.as_bytes();
        let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        let mut from = 0usize;
        while let Some(rel) = line[from..].find("budget") {
            let at = from + rel;
            from = at + "budget".len();

            // Frontière de mot à gauche : `per_budget` / `total_budget` sont
            // d'autres questions et ne doivent pas entrer dans la population.
            if at > 0 && is_word(bytes[at - 1]) {
                continue;
            }
            // …et à droite, pour ne pas apparier un `budget_source`.
            let mut i = from;
            if i < bytes.len() && is_word(bytes[i]) {
                continue;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if !line[i..].starts_with("==") {
                continue;
            }
            i += 2;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'0' {
                // `== 0` et non `== 0x…` ni `== 01`.
                let after = bytes.get(i + 1).copied();
                if after.is_none_or(|c| !is_word(c)) {
                    return true;
                }
            }
        }
        false
    }

    /// U1 — le prédicat « budget nul » n'a qu'un lecteur (mika#1833).
    ///
    /// Un second site testant `budget == 0` pourrait diverger du premier en
    /// silence — la classe `grooming_marker` (mika#2158), déjà payée deux
    /// fois dans ce crate. Aucun test comportemental ne peut la voir : la
    /// copie ne rend aucune décision fausse le jour où elle est écrite.
    ///
    /// **Quand ce scan tire, on route le site par [`phase_is_disabled`] ; on
    /// n'ajoute pas de ligne** (doctrine mika#2201) — l'allowlist est livrée
    /// vide et [`mika1833_the_zero_budget_allowlist_is_empty`] l'y maintient.
    #[test]
    fn mika1833_the_zero_budget_predicate_has_a_single_reader() {
        let kg_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("kg");
        let this_module = kg_root.join("budget.rs");

        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        let mut calls_the_reader = false;
        let mut stack = vec![kg_root.clone()];

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this_module {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("la garde doit pouvoir lire {}: {e}", path.display())
                });
                scanned += 1;
                // Le code de test d'un module voisin porte légitimement le
                // prédicat dans une fixture : on tronque au premier
                // `#[cfg(test)]`, comme `production_sources`.
                let production = match content.find("\n#[cfg(test)]") {
                    Some(i) => &content[..i],
                    None => &content[..],
                };
                if production.contains("phase_is_disabled") {
                    calls_the_reader = true;
                }
                let file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if ZERO_BUDGET_READER_EXCEPTIONS.contains(&file_name.as_str()) {
                    continue;
                }
                for (n, line) in production.lines().enumerate() {
                    if line_reads_the_phase_budget(line) {
                        offenders.push(format!("kg/{}:{}: {}", file_name, n + 1, line.trim()));
                    }
                }
            }
        }

        assert!(
            scanned > 0,
            "la garde n'a scanné aucun fichier — chemin cassé"
        );

        // Anti-vacuité : un scan qui garde un prédicat que personne n'appelle
        // ne vérifie rien, et se lit exactement comme un arbre propre.
        assert!(
            calls_the_reader,
            "mika#1833 — `phase_is_disabled` n'est appelé nulle part sous \
             `src/kg/` : ce scan garde un prédicat mort, il ne vérifie rien"
        );

        assert!(
            offenders.is_empty(),
            "mika#1833 — le prédicat « budget nul » a un second lecteur :\n{}\n\n\
             RÉSOLUTION : appeler `budget::phase_is_disabled(budget)`. Ne PAS \
             ajouter le site à ZERO_BUDGET_READER_EXCEPTIONS — une copie \
             répondrait un jour différemment à la même question, en silence \
             (classe mika#2158).",
            offenders.join("\n")
        );
    }

    /// Le contrôle de bonne foi du scan ci-dessus.
    ///
    /// Sans lui, « le scan voit un second lecteur » est indistinguable de
    /// « le scan ne voit jamais rien ».
    #[test]
    fn mika1833_the_reader_scan_reddens_on_a_second_reader() {
        assert!(
            line_reads_the_phase_budget("        if budget == 0 {"),
            "le scan doit voir un second lecteur écrit à la main"
        );
        assert!(
            line_reads_the_phase_budget("    let skip = budget==0;"),
            "rustfmt ne doit pas pouvoir cacher l'aiguille en retirant les espaces"
        );
        assert!(
            line_reads_the_phase_budget("        if budget ==  0 {"),
            "ni en les multipliant"
        );

        // Bonne foi dans l'autre sens : les deux formes hors population.
        assert!(
            !line_reads_the_phase_budget("        if *per_budget == 0 {"),
            "le budget par corpus n'est pas le budget de phase — le confondre \
             ferait rougir le scan en permanence, donc le ferait désarmer"
        );
        assert!(
            !line_reads_the_phase_budget("    if n == 0 || total_budget == 0 {"),
            "l'allocation équitable n'est pas le prédicat de désactivation"
        );
        assert!(
            !line_reads_the_phase_budget("    /// `budget == 0` court-circuite la phase."),
            "une mention en commentaire n'est pas une lecture (classe mika#2050)"
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist.
    #[test]
    fn mika1833_the_zero_budget_allowlist_is_empty() {
        assert!(
            ZERO_BUDGET_READER_EXCEPTIONS.is_empty(),
            "ZERO_BUDGET_READER_EXCEPTIONS est livrée vide et doit le rester : \
             une allowlist née vide est un tiroir où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    /// **Livrée vide, et le test ci-dessus l'assert.**
    ///
    /// La population a été recensée avant d'écrire le scan : le prédicat naît
    /// avec mika#1833, donc il n'y a rien à exempter.
    const ZERO_BUDGET_READER_EXCEPTIONS: &[&str] = &[];

    #[test]
    fn mika1833_a_zero_budget_disables_the_phase() {
        assert!(phase_is_disabled(0));
        assert!(!phase_is_disabled(1));
        assert!(!phase_is_disabled(500));
    }

    #[test]
    fn single_corpus_gets_all_budget() {
        let result = allocate_fair_budget(&[100], 50);
        assert_eq!(result, vec![50]);
    }

    #[test]
    fn equal_pending_splits_evenly() {
        let result = allocate_fair_budget(&[100, 100], 60);
        assert_eq!(result, vec![30, 30]);
    }

    #[test]
    fn unequal_pending_redistributes_to_hungry() {
        // Corpus 0 has 10 pending, corpus 1 has 100 pending, budget=60
        // Pass 1: base_share=30, assigned=[10, 30] (corpus 0 capped at 10)
        // Pass 2: remaining=20, hungry=[1], corpus 1 gets 20 more -> [10, 50]
        let result = allocate_fair_budget(&[10, 100], 60);
        assert_eq!(result, vec![10, 50]);
    }

    #[test]
    fn budget_exceeds_total_pending() {
        let result = allocate_fair_budget(&[10, 20, 5], 100);
        assert_eq!(result, vec![10, 20, 5]);
    }

    #[test]
    fn budget_is_zero_returns_all_zeros() {
        let result = allocate_fair_budget(&[100, 200], 0);
        assert_eq!(result, vec![0, 0]);
    }

    #[test]
    fn empty_input_returns_empty() {
        let result = allocate_fair_budget(&[], 100);
        assert_eq!(result, Vec::<u32>::new());
    }

    #[test]
    fn one_corpus_zero_pending_share_redistributed() {
        // Corpus 0 has 0 pending, corpus 1 has 100, budget=60
        // N_active=1, base_share=60, assigned=[0, 60]
        let result = allocate_fair_budget(&[0, 100], 60);
        assert_eq!(result, vec![0, 60]);
    }

    #[test]
    fn three_corpora_all_get_nonzero() {
        // 3 corpora: [100, 50, 25], budget=60
        // N_active=3, base_share=20, assigned=[20, 20, 20]
        // Remainder: 60-60=0. All get 20.
        let result = allocate_fair_budget(&[100, 50, 25], 60);
        assert_eq!(result, vec![20, 20, 20]);
    }

    #[test]
    fn three_corpora_small_one_capped() {
        // 3 corpora: [100, 50, 5], budget=60
        // N_active=3, base_share=20, assigned=[20, 20, 5]
        // Remainder: 60-45=15, hungry=[0,1]
        // share=7, corpus 0 gets 7, corpus 1 gets 7, remaining=1
        // next pass: share=1, corpus 0 gets 1
        let result = allocate_fair_budget(&[100, 50, 5], 60);
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 60);
        assert_eq!(result[2], 5); // small corpus gets exactly its pending
        assert!(result[0] > 0 && result[1] > 0); // both hungry get something
    }

    #[test]
    fn postcondition_sum_equals_min_budget_total() {
        let pending = &[200, 300, 100, 50];
        let budget = 400;
        let result = allocate_fair_budget(pending, budget);
        let sum: u32 = result.iter().sum();
        let total_pending: u32 = pending.iter().sum();
        assert_eq!(sum, budget.min(total_pending));
    }

    #[test]
    fn budget_less_than_active_serves_first_hungry() {
        // budget=2 < N_active=4: only first 2 hungry corpora get allocation
        let pending = &[1, 1, 1, 0, 1];
        let budget = 2;
        let result = allocate_fair_budget(pending, budget);
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 2);
        assert_eq!(result[3], 0); // zero-pending corpus gets nothing
        // First two hungry corpora get 1 each; remaining two get 0
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 1);
        assert_eq!(result[2], 0);
        assert_eq!(result[4], 0);
    }

    #[test]
    fn budget_ge_active_every_corpus_gets_nonzero() {
        // budget=4 >= N_active=4: every active corpus gets at least 1
        let pending = &[1, 1, 1, 0, 1];
        let budget = 4;
        let result = allocate_fair_budget(pending, budget);
        let sum: u32 = result.iter().sum();
        assert_eq!(sum, 4);
        assert_eq!(result[3], 0); // zero-pending corpus gets nothing
        for (i, &alloc) in result.iter().enumerate() {
            if pending[i] > 0 {
                assert!(alloc > 0, "corpus {i} has pending={} but got 0", pending[i]);
            }
        }
    }
}

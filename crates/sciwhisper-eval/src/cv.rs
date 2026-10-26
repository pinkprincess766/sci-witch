//! Cross-validation folds by phrase family for voice-v2 train (`tts-denis`).
//!
//! The rule is frozen in `research/protocol/voice-v2.md` (23.10.2026) and in
//! `research/protocol/question-2-calibration.md`: families are sorted by the
//! lowercase hex SHA-256 of `voice-v2:cv:<family_id>` and dealt round-robin,
//! the i-th family going to fold `i mod 5`. A fold is a function of the family
//! only, so every clip of a family (all stages 0…s) lands in the same fold.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// Number of folds, frozen by the protocol.
pub const FOLDS: usize = 5;

/// Prefix of the hashed string, frozen by the protocol.
pub const KEY_PREFIX: &str = "voice-v2:cv:";

/// A model is not trained on a fold whose training part has fewer errors
/// than this among answered clips, and drops out of model selection.
pub const MIN_TRAIN_ERRORS: usize = 5;

/// Lowercase hex SHA-256 of `voice-v2:cv:<family_id>`.
pub fn sort_key(family_id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{KEY_PREFIX}{family_id}").as_bytes())
    )
}

/// Fold of every distinct family. Duplicates (one family, many stages) are
/// collapsed before dealing, so they cannot shift the round-robin.
pub fn assign_folds<'a>(family_ids: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, usize> {
    let mut keyed: Vec<(String, String)> = family_ids
        .into_iter()
        .map(|f| (sort_key(f), f.to_owned()))
        .collect();
    keyed.sort();
    keyed.dedup();
    keyed
        .into_iter()
        .enumerate()
        .map(|(i, (_, family))| (family, i % FOLDS))
        .collect()
}

/// `true` when the model may be trained on every fold: each fold's training
/// part (everything outside the fold) has at least [`MIN_TRAIN_ERRORS`] errors.
/// `errors_per_fold[k]` is the number of errors among answered clips in fold k.
pub fn model_survives(errors_per_fold: &[usize; FOLDS]) -> bool {
    let total: usize = errors_per_fold.iter().sum();
    errors_per_fold
        .iter()
        .all(|&held_out| total - held_out >= MIN_TRAIN_ERRORS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_rule_counts_training_part_not_fold() {
        // 5 errors in total: every training part holds 4, so the model drops.
        assert!(!model_survives(&[1, 1, 1, 1, 1]));
        // 6 in total, but the training part without fold 0 holds 4.
        assert!(!model_survives(&[2, 1, 1, 1, 1]));
        // Each training part holds at least 5.
        assert!(model_survives(&[2, 1, 1, 1, 2]));
        // All errors in one fold: that fold's training part holds 0.
        assert!(!model_survives(&[30, 0, 0, 0, 0]));
    }
}

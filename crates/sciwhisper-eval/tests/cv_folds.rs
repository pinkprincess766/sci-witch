//! Plan item 3: voice-v2 CV folds are deterministic.
//!
//! The 40 block I families of `research/protocol/voice-v1-families.json` go
//! through the frozen rule (`sciwhisper_eval::cv`). The expected folds below
//! were computed once, independently, with Python `hashlib.sha256` on
//! `voice-v2:cv:<family_id>`. Changing the prefix, the hash, the sort or the
//! `i mod 5` deal changes the folds and fails this test.

use std::collections::BTreeSet;
use std::path::PathBuf;

use sciwhisper_eval::cv::{assign_folds, FOLDS};
use serde_json::Value;

/// Expected families per fold, each list sorted by name.
const EXPECTED: [&[&str]; FOLDS] = [
    &[
        "amb-div-bind-001",
        "amb-neg-copper-prose-001",
        "amb-neg-not-correction-001",
        "chem-al2o3-001",
        "chem-cu-ion-001",
        "math-deriv-first-001",
        "math-deriv-third-001",
        "phys-inverse-metre-001",
    ],
    &[
        "amb-fix-too-many-001",
        "amb-frame-between-water-001",
        "amb-frame-decompose-001",
        "amb-neg-degree-001",
        "amb-neg-reaction-001",
        "chem-feoh3-001",
        "math-abs-001",
        "math-inequality-001",
    ],
    &[
        "amb-asr-hydroxide-001",
        "amb-domain-delta-g-001",
        "amb-fix-hydroxide-001",
        "amb-frame-result-water-001",
        "amb-order-parenthetical-001",
        "amb-root-bind-001",
        "chem-co2-001",
        "chem-hydrochloric-001",
    ],
    &[
        "amb-domain-spelled-001",
        "amb-fix-substitute-001",
        "amb-order-filler-001",
        "math-apply-two-args",
        "math-factorial-001",
        "math-fraction-word-001",
        "math-integral-indef-001",
        "phys-kelvin-001",
    ],
    &[
        "amb-asr-carbonate-001",
        "amb-asr-unrepairable-001",
        "amb-neg-integral-001",
        "chem-ammonia-001",
        "math-limit-left-001",
        "phys-compatible-lengths-001",
        "phys-deriv-velocity-001",
        "phys-g-001",
    ],
];

fn block_i_family_ids() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../research/protocol/voice-v1-families.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    json["blocks"]["I"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["family_id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn block_i_has_40_families_dealt_8_per_fold_as_frozen() {
    let families = block_i_family_ids();
    let folds = assign_folds(families.iter().map(String::as_str));
    assert_eq!(folds.len(), 40);
    for (k, expected) in EXPECTED.iter().enumerate() {
        let got: Vec<&str> = folds
            .iter()
            .filter(|(_, &f)| f == k)
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(got.len(), 8, "fold {k} size");
        assert_eq!(&got, expected, "fold {k} differs from the frozen rule");
    }
}

#[test]
fn folds_do_not_depend_on_input_order() {
    let mut families = block_i_family_ids();
    let a = assign_folds(families.iter().map(String::as_str));
    families.reverse();
    let b = assign_folds(families.iter().map(String::as_str));
    assert_eq!(a, b);
}

#[test]
fn every_stage_of_a_family_is_in_one_fold() {
    // A clip list with stages 0..=3 for every family, in shuffled-looking order.
    let families = block_i_family_ids();
    let clips: Vec<(String, u32)> = (0..=3u32)
        .rev()
        .flat_map(|s| families.iter().map(move |f| (f.clone(), s)))
        .collect();
    let folds = assign_folds(clips.iter().map(|(f, _)| f.as_str()));
    assert_eq!(folds.len(), 40, "stages must not create extra families");
    let base = assign_folds(families.iter().map(String::as_str));
    for (family, _) in &clips {
        let per_family: BTreeSet<usize> = [folds[family], base[family]].into();
        assert_eq!(per_family.len(), 1, "{family} split across folds");
    }
}

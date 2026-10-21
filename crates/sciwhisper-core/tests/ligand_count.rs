//! `Complex::ligand_count` sums ligand counts and returns `None` when the sum
//! does not fit in `u32`. `saturating_add` would hide that overflow.

use sciwhisper_core::ast::{Center, Complex, Formula, LigandSlot};

fn complex_with(counts: &[u32]) -> Complex {
    Complex {
        center: Center {
            symbol: "Cu".into(),
            oxidation: 2,
        },
        ligands: counts
            .iter()
            .copied()
            .map(|count| LigandSlot {
                formula: Formula::default(),
                charge: 0,
                count,
            })
            .collect(),
        charge: 2,
        count: 1,
    }
}

#[test]
fn six_and_zero_sum_to_six() {
    assert_eq!(complex_with(&[6, 0]).ligand_count(), Some(6));
}

#[test]
fn no_ligands_sum_to_zero() {
    assert_eq!(complex_with(&[]).ligand_count(), Some(0));
}

#[test]
fn two_max_counts_overflow_to_none() {
    assert_eq!(complex_with(&[u32::MAX, u32::MAX]).ligand_count(), None);
}

//! The written-formula parser (`sciwhisper_core::formula`) recurses once per
//! `(`. It is a public module, so its nesting is bounded by the same
//! `MAX_PARSE_DEPTH` as spoken mathematics, and these tests pin both sides of
//! the bound.

use sciwhisper_core::formula::{parse_formula_str, parse_species_str};
use sciwhisper_core::parser::math::MAX_PARSE_DEPTH;

fn nested(levels: usize) -> String {
    format!("{}H{}", "(".repeat(levels), ")".repeat(levels))
}

#[test]
fn groups_at_the_limit_parse_and_one_more_is_refused() {
    assert!(parse_formula_str(&nested(MAX_PARSE_DEPTH)).is_ok());
    let refused = parse_formula_str(&nested(MAX_PARSE_DEPTH + 1));
    assert!(refused.is_err(), "{refused:?}");
}

/// Without the bound this aborts the whole process with a stack overflow,
/// which no `Result` can report. With it the thread finishes with an error.
#[test]
fn a_hundred_thousand_open_brackets_are_refused_without_a_crash() {
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let deep = "(".repeat(100_000) + "H" + &")".repeat(100_000);
            (
                parse_formula_str(&deep).is_err(),
                parse_species_str(&deep).is_err(),
                parse_formula_str(&"(".repeat(100_000)).is_err(),
            )
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), (true, true, true));
}

/// The bound counts nesting, not groups: a counter that forgot to come back
/// down after `)` would refuse this long but flat formula.
#[test]
fn many_sibling_groups_are_not_nesting() {
    let flat = "(OH)".repeat(MAX_PARSE_DEPTH + 5);
    let formula = parse_formula_str(&flat).expect("siblings are not nested");
    assert_eq!(formula.parts.len(), MAX_PARSE_DEPTH + 5);
}

#[test]
fn ordinary_formulas_are_unchanged() {
    for text in [
        "Cu(OH)2",
        "Ca3(PO4)2",
        "CuSO4·5H2O",
        "Fe2(SO4)3",
        "((CH3)3C)2O",
    ] {
        assert!(parse_formula_str(text).is_ok(), "{text}");
    }
}

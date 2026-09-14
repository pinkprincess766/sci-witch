//! Counterfactual tests: change one spoken thing, and exactly one thing in
//! the structure must change.
//!
//! Accuracy on a corpus cannot see this. A parser that reads «гидроксид
//! железа два» and «гидроксид железа три» correctly may still be reading them
//! by two unrelated routes, and the day one of those routes shifts, the other
//! shifts with it in a way no example in the corpus covers. What is checked
//! here is *locality*: the spoken element and the AST node stand in a fixed
//! relation, so a change to one is a change to the other and to nothing else.
//!
//! The diff is structural, not textual. Two trees are compared slot by slot
//! and the paths at which they disagree are collected; the test names the
//! path it expects, so a change that lands in the right place for the wrong
//! reason still fails.

use sciwhisper_core::lattice::{build, LatticeOptions};
use sciwhisper_core::Node;

/// Every path at which two serialized trees disagree, deepest name first.
///
/// Recursion stops where the two sides stop having the same shape: past that
/// point there is no common slot to name, and reporting the children would
/// be reporting one tree's contents rather than a difference.
fn differences(left: &serde_json::Value, right: &serde_json::Value, path: &str) -> Vec<String> {
    use serde_json::Value;
    if left == right {
        return Vec::new();
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = Vec::new();
            let shared: Vec<&String> = a.keys().filter(|key| b.contains_key(*key)).collect();
            if shared.is_empty() {
                return vec![format!("{path}/<variant>")];
            }
            for key in shared {
                out.extend(differences(&a[key], &b[key], &format!("{path}/{key}")));
            }
            for key in a.keys().filter(|key| !b.contains_key(*key)) {
                out.push(format!("{path}/{key}"));
            }
            for key in b.keys().filter(|key| !a.contains_key(*key)) {
                out.push(format!("{path}/{key}"));
            }
            out
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return vec![format!("{path}/<length {} vs {}>", a.len(), b.len())];
            }
            a.iter()
                .zip(b.iter())
                .enumerate()
                .flat_map(|(index, (a, b))| differences(a, b, &format!("{path}/{index}")))
                .collect()
        }
        _ => vec![path.to_string()],
    }
}

/// The best reading the lattice offers for `text`, structurally.
///
/// "Best" is the first candidate that is not `RAW`: the lattice's own order.
/// Reading it through the lattice rather than through `interpret` is
/// deliberate — locality has to hold for what the system *offers*, not only
/// for the one reading it happens to prefer.
fn structure(text: &str) -> Node {
    build(text, LatticeOptions::default())
        .candidates
        .into_iter()
        .find_map(|candidate| candidate.reading.ast().cloned())
        .unwrap_or_else(|| panic!("«{text}» produced no structural reading at all"))
}

#[track_caller]
fn changes_only(before: &str, after: &str, expected: &[&str]) {
    let a = serde_json::to_value(structure(before)).expect("a node serializes");
    let b = serde_json::to_value(structure(after)).expect("a node serializes");
    assert_ne!(a, b, "«{before}» and «{after}» produced the same structure");
    let found = differences(&a, &b, "");
    assert_eq!(
        found,
        expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "«{before}» → «{after}» changed {found:?}"
    );
}

#[test]
fn changing_a_hydroxide_index_changes_only_that_index() {
    changes_only(
        "гидроксид железа два",
        "гидроксид железа три",
        &["/Chemical/Species/formula/parts/1/Group/count"],
    );
}

#[test]
fn changing_a_metal_changes_only_the_metal() {
    changes_only(
        "гидроксид натрия",
        "гидроксид калия",
        &["/Chemical/Species/formula/parts/0/Atom/symbol"],
    );
}

#[test]
fn changing_a_coefficient_leaves_the_formula_alone() {
    changes_only(
        "два гидроксида натрия",
        "три гидроксида натрия",
        &["/Chemical/Species/coefficient"],
    );
}

#[test]
fn changing_an_exponent_changes_only_the_exponent() {
    changes_only("икс в квадрате", "икс в кубе", &["/Math/Power/exp/Number"]);
}

#[test]
fn changing_a_named_function_changes_only_its_kind() {
    changes_only("синус икс", "косинус икс", &["/Math/Function/kind"]);
}

#[test]
fn changing_an_operand_changes_only_that_operand() {
    changes_only(
        "икс плюс один",
        "икс плюс два",
        &["/Math/Binary/right/Number"],
    );
}

#[test]
fn changing_the_operator_changes_only_the_operator() {
    changes_only("икс плюс один", "икс минус один", &["/Math/Binary/op"]);
}

#[test]
fn changing_one_reagent_changes_only_that_reagent() {
    changes_only(
        "натрий взаимодействует с хлором, в результате получается хлорид натрия",
        "калий взаимодействует с хлором, в результате получается хлорид натрия",
        &["/Chemical/Equation/left/0/formula/parts/0/Atom/symbol"],
    );
}

#[test]
fn a_correction_moves_the_node_it_names_and_nothing_else() {
    // The corrected phrase and the phrase said correctly in the first place
    // have to agree completely: a correction is not a different construction.
    let corrected = serde_json::to_value(structure("гидроксид железа два, нет, железа три"))
        .expect("a node serializes");
    let direct = serde_json::to_value(structure("гидроксид железа три")).expect("serializes");
    assert_eq!(
        differences(&corrected, &direct, ""),
        Vec::<String>::new(),
        "a correction produced a different structure from saying it right"
    );
}

#[test]
fn a_repaired_word_produces_the_structure_the_correct_word_would() {
    let repaired = serde_json::to_value(structure("карбанат кальция")).expect("a node serializes");
    let direct = serde_json::to_value(structure("карбонат кальция")).expect("serializes");
    assert_eq!(
        differences(&repaired, &direct, ""),
        Vec::<String>::new(),
        "the vowel repair produced something other than the word it repaired to"
    );
}

#[test]
fn a_number_the_lexicon_cannot_scale_is_refused_rather_than_truncated() {
    // «девятьсот девяносто девять тысяч» is 999 000. The number lexicon only
    // adds, so it reads 999 and leaves «тысяч» behind — and the span search
    // used to accept the part that parsed, showing `Fe(OH)₉₉₉ тысяч` for a
    // number a thousand times larger.
    for spoken in [
        "гидроксид железа девятьсот девяносто девять тысяч",
        "гидроксид железа два тысячи",
    ] {
        let lattice = build(spoken, LatticeOptions::default());
        for candidate in &lattice.candidates {
            let Some(node) = candidate.reading.ast() else {
                continue;
            };
            let shown = sciwhisper_core::render(node, sciwhisper_core::Renderer::Unicode);
            assert!(
                !shown.contains("тысяч"),
                "«{spoken}» kept a scale word beside a formula: {shown}"
            );
        }
    }
}

#[test]
fn an_ordinary_subscript_still_works_after_the_scale_guard() {
    // The guard must fire on a multiplier, not on any word at all.
    assert_eq!(
        sciwhisper_core::render(
            &structure("гидроксид железа три"),
            sciwhisper_core::Renderer::Unicode
        ),
        "Fe(OH)₃"
    );
    assert_eq!(
        sciwhisper_core::render(&structure("аш два о"), sciwhisper_core::Renderer::Unicode),
        "H₂O"
    );
}

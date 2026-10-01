//! Choose one text hypothesis from a recognizer's n-best list.
//!
//! The lattice says what each hypothesis could be. This module is the one
//! place that turns that into a choice, and the choice is deliberately
//! smaller than "whichever hypothesis parses":
//!
//! * the top hypothesis is kept whenever the lattice can read the whole of
//!   it as one construct, with no edited words;
//! * a later hypothesis may replace it only when it is within
//!   [`MAX_HYPOTHESIS_EDITS`] characters of the top one and it is the only
//!   such reading;
//! * two near hypotheses that parse to different trees are a disagreement,
//!   and the top hypothesis stays;
//! * a hypothesis the lattice reached by repairing, deleting or rotating
//!   words does not count. Those readings stay offers, as they do everywhere
//!   else in the lattice;
//! * **the repair has to be next to science that was already heard.** The
//!   top hypothesis must share at least one scientific word with the
//!   replacement — an element, a salt name, a substance, a construction, a
//!   unit. Without that, a one-letter neighbour turns ordinary words into
//!   formulas: «водка» became «вода» (H₂O), «года» became «вода», «два года»
//!   became «два вода», and «сода» — a different substance — became water.
//!   A near hypothesis may fix science; it may not create it.
//!
//! That last rule costs something real. «мтан» → «метан» is a genuine
//! recognizer slip with nothing scientific around it, and it is no longer
//! repaired. Text alone cannot tell «мтан» (not a word) from «водка» (a word)
//! without a dictionary or an acoustic score, and this module has neither.
//! Keeping the words is an S1 miss; inventing water is S4.
//!
//! There is no acoustic score here. A list without one is a synthetic list,
//! which is what this can be tested on.

use crate::ast::Node;
use crate::lattice::{self, LatticeOptions, Origin};
use crate::normalize::normalize;

/// How far a later hypothesis may stray from the top one, in Unicode scalar
/// values, after normalization. One vowel (`карбанат` / `карбонат`) fits.
/// A different sentence does not.
pub const MAX_HYPOTHESIS_EDITS: usize = 2;

/// Hypotheses looked at, counting the top one. Recognizers emit a handful;
/// whisper.cpp's beam defaults to five. Past this the list is not an n-best
/// list but a list, and each entry costs a full lattice build.
pub const MAX_HYPOTHESES: usize = 16;

/// Which hypothesis to compile, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub index: usize,
    pub reason: ChoiceReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceReason {
    /// The list was empty. `index` is 0 and names nothing.
    Empty,
    /// The top hypothesis is already a whole-utterance reading.
    TopAlreadyParsed,
    /// Exactly one distinct reading sits within the edit bound.
    NearHypothesisParsed,
    /// Nothing within the edit bound is a whole-utterance reading.
    NothingNearParsed,
    /// Two near hypotheses are both readings, and they are not the same tree.
    NearHypothesesDisagree,
    /// A near hypothesis is a reading, but the top hypothesis shares no
    /// scientific word with it. Taking it would have created the science
    /// rather than repaired it, so the top hypothesis stays.
    NoScientificAnchor,
}

/// Pick the hypothesis a compiler should read.
///
/// `hypotheses` is in recognizer order: index 0 is the top hypothesis.
pub fn choose_hypothesis(hypotheses: &[&str]) -> Choice {
    let Some(top) = hypotheses.first() else {
        return Choice {
            index: 0,
            reason: ChoiceReason::Empty,
        };
    };
    if whole_reading(top).is_some() {
        return Choice {
            index: 0,
            reason: ChoiceReason::TopAlreadyParsed,
        };
    }

    let top_norm = normalize(top);
    let mut chosen: Option<(usize, Node)> = None;
    let mut unanchored = false;
    for (index, hypothesis) in hypotheses.iter().enumerate().take(MAX_HYPOTHESES).skip(1) {
        // The lattice reads nothing past its own input bound, so neither
        // does this; and the edit distance is not worth computing on a
        // string no reading could come from.
        if hypothesis.len() > lattice::MAX_INPUT_BYTES || top.len() > lattice::MAX_INPUT_BYTES {
            continue;
        }
        let hypothesis_norm = normalize(hypothesis);
        if !within_edit_distance(&top_norm, &hypothesis_norm, MAX_HYPOTHESIS_EDITS) {
            continue;
        }
        let Some(reading) = whole_reading(hypothesis) else {
            continue;
        };
        if !shares_a_scientific_word(&top_norm, &hypothesis_norm) {
            unanchored = true;
            continue;
        }
        match &chosen {
            None => chosen = Some((index, reading)),
            Some((_, previous)) if previous == &reading => {}
            Some(_) => {
                return Choice {
                    index: 0,
                    reason: ChoiceReason::NearHypothesesDisagree,
                };
            }
        }
    }

    match chosen {
        Some((index, _)) => Choice {
            index,
            reason: ChoiceReason::NearHypothesisParsed,
        },
        None if unanchored => Choice {
            index: 0,
            reason: ChoiceReason::NoScientificAnchor,
        },
        None => Choice {
            index: 0,
            reason: ChoiceReason::NothingNearParsed,
        },
    }
}

/// Whether some word that both hypotheses contain, unchanged, is scientific
/// on its own. The repair then fixed a word *next to* science that was
/// already heard, rather than producing the only science in the phrase.
fn shares_a_scientific_word(top: &str, other: &str) -> bool {
    let theirs: Vec<&str> = other.split_whitespace().collect();
    top.split_whitespace()
        .filter(|word| theirs.contains(word))
        .any(crate::utterance::is_scientific_anchor)
}

/// The whole-utterance tree automatic routing read off this string, without
/// editing the words. `None` when there is no such tree.
///
/// Only the `WholeUtterance` origin counts. A grammar alternative (the wide
/// radical, say) is an offer the lattice makes and is left out here, so the
/// comparison below is between what the parser would actually answer. That
/// origin comes from one `interpret` call, so at most one tree carries it;
/// the disagreement branch is kept as a guard, not because it is reachable
/// today.
fn whole_reading(text: &str) -> Option<Node> {
    let lattice = lattice::build(text, LatticeOptions::default());
    let mut found: Option<Node> = None;
    for candidate in &lattice.candidates {
        if !candidate.whole_utterance || candidate.edits_the_words() {
            continue;
        }
        if !candidate
            .origins
            .iter()
            .any(|origin| matches!(origin, Origin::WholeUtterance))
        {
            continue;
        }
        let node = candidate.reading.ast()?.clone();
        match &found {
            None => found = Some(node),
            Some(previous) if previous == &node => {}
            Some(_) => return None,
        }
    }
    found
}

/// Levenshtein distance ≤ `max`, computed only in the band `|i − j| ≤ max`.
///
/// A cell outside that band already costs more than `max`, so it is never
/// needed. That makes the work `O(len · max)` instead of `O(len²)`: the full
/// table took 1.4 s on two 34 000-character strings that differed by one
/// letter, which is a denial of service for a function that exists to be
/// called on every utterance.
fn within_edit_distance(left: &str, right: &str, max: usize) -> bool {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > max {
        return false;
    }
    let beyond = max + 1;
    let width = right.len() + 1;
    let mut previous: Vec<usize> = (0..width).map(|j| j.min(beyond)).collect();
    let mut current = vec![beyond; width];
    for (i, &left_char) in left.iter().enumerate() {
        let row = i + 1;
        let low = row.saturating_sub(max);
        let high = (row + max).min(right.len());
        current.iter_mut().for_each(|cell| *cell = beyond);
        if low == 0 {
            current[0] = row.min(beyond);
        }
        let mut row_min = if low == 0 { current[0] } else { beyond };
        for j in low.max(1)..=high {
            let substitution = usize::from(left_char != right[j - 1]);
            let value = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + substitution)
                .min(beyond);
            current[j] = value;
            row_min = row_min.min(value);
        }
        if row_min > max {
            return false;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()] <= max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Renderer;
    use crate::render;

    fn unicode(text: &str) -> String {
        let choice = choose_hypothesis(&[text]);
        assert_eq!(choice.index, 0);
        let Some(node) = whole_reading(text) else {
            return text.to_string();
        };
        render::render(&node, Renderer::Unicode)
    }

    fn chosen<'a>(hypotheses: &[&'a str]) -> (Choice, &'a str) {
        let choice = choose_hypothesis(hypotheses);
        let text = hypotheses.get(choice.index).copied().unwrap_or("");
        (choice, text)
    }

    #[test]
    fn a_near_miss_is_replaced_by_the_hypothesis_the_grammar_accepts() {
        for (broken, repaired, formula) in [
            ("карбанат кальция", "карбонат кальция", "CaCO₃"),
            ("хларид натрия", "хлорид натрия", "NaCl"),
            ("сернея кислота", "серная кислота", "H₂SO₄"),
            ("гидроксид жилеза три", "гидроксид железа три", "Fe(OH)₃"),
        ] {
            let (choice, text) = chosen(&[broken, repaired]);
            assert_eq!(choice.index, 1, "{broken}");
            assert_eq!(choice.reason, ChoiceReason::NearHypothesisParsed);
            assert_eq!(text, repaired);
            assert_eq!(unicode(repaired), formula);
            assert_eq!(unicode(broken), broken);
        }
    }

    #[test]
    fn a_top_hypothesis_that_already_parses_is_not_replaced() {
        let (choice, text) = chosen(&["метан", "этан"]);
        assert_eq!(choice.reason, ChoiceReason::TopAlreadyParsed);
        assert_eq!(text, "метан");
        assert_eq!(unicode(text), "CH₄");
    }

    #[test]
    fn two_near_sciences_are_a_disagreement_and_the_top_hypothesis_stays() {
        // «нитрот калия» is one vowel from KNO₃ and one from KNO₂. Both are
        // real salts, the anchor «калия» is heard in both, and picking one
        // would be answering a question the recognizer left open.
        let (choice, text) = chosen(&["нитрот калия", "нитрат калия", "нитрит калия"]);
        assert_eq!(choice.reason, ChoiceReason::NearHypothesesDisagree);
        assert_eq!(choice.index, 0);
        assert_eq!(text, "нитрот калия");
        assert_eq!(unicode(text), "нитрот калия");
    }

    #[test]
    fn a_near_hypothesis_may_not_create_the_science_it_is_supposed_to_repair() {
        // Each of these is an ordinary word, or a different substance, one
        // letter from something the grammar reads. Before the anchor rule
        // every one of them was replaced: «водка» and «года» became water,
        // «два года» became «два вода», and baking soda became water too.
        for (top, near) in [
            ("водка", "вода"),
            ("года", "вода"),
            ("сода", "вода"),
            ("мета", "метан"),
            ("два года", "два вода"),
            // A genuine slip with nothing scientific around it. It is lost,
            // and that is the price: text alone cannot tell it from «водка».
            ("мтан", "метан"),
        ] {
            let (choice, text) = chosen(&[top, near]);
            assert_eq!(choice.index, 0, "{top} was replaced by {near}");
            assert_eq!(choice.reason, ChoiceReason::NoScientificAnchor, "{top}");
            assert_eq!(text, top);
        }
    }

    #[test]
    fn only_the_first_hypotheses_are_looked_at() {
        // The one hypothesis that would be accepted sits just past the bound.
        let mut list = vec!["карбанат кальция"; MAX_HYPOTHESES];
        list.push("карбонат кальция");
        let (choice, _) = chosen(&list);
        assert_eq!(choice.index, 0);
        assert_eq!(choice.reason, ChoiceReason::NothingNearParsed);
        // One place earlier, inside the bound, it is found.
        list[MAX_HYPOTHESES - 1] = "карбонат кальция";
        let (choice, _) = chosen(&list);
        assert_eq!(choice.index, MAX_HYPOTHESES - 1);
    }

    #[test]
    fn a_hypothesis_past_the_lattice_input_bound_is_not_compared() {
        let long = "карбанат кальция ".repeat(1 + lattice::MAX_INPUT_BYTES / 16);
        let repaired = long.replacen("карбанат", "карбонат", 1);
        assert!(long.len() > lattice::MAX_INPUT_BYTES);
        let (choice, _) = chosen(&[long.as_str(), repaired.as_str()]);
        assert_eq!(choice.index, 0);
    }

    /// The banded distance must agree with the full table everywhere it
    /// answers, and a hand table is the reference, not the function itself.
    #[test]
    fn the_banded_edit_distance_matches_a_full_table() {
        fn full(a: &str, b: &str) -> usize {
            let a: Vec<char> = a.chars().collect();
            let b: Vec<char> = b.chars().collect();
            let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
            for (i, row) in d.iter_mut().enumerate() {
                row[0] = i;
            }
            for (j, cell) in d[0].iter_mut().enumerate() {
                *cell = j;
            }
            for i in 1..=a.len() {
                for j in 1..=b.len() {
                    let s = usize::from(a[i - 1] != b[j - 1]);
                    d[i][j] = (d[i - 1][j] + 1)
                        .min(d[i][j - 1] + 1)
                        .min(d[i - 1][j - 1] + s);
                }
            }
            d[a.len()][b.len()]
        }
        let words = [
            "",
            "а",
            "мтан",
            "метан",
            "этан",
            "водка",
            "вода",
            "карбанат",
            "карбонат",
            "кар бонат",
            "абвгд",
            "дгвба",
            "нитрот",
            "нитрат",
            "нитрит",
        ];
        for a in words {
            for b in words {
                for max in 0..=3 {
                    assert_eq!(
                        within_edit_distance(a, b, max),
                        full(a, b) <= max,
                        "{a:?} ~ {b:?} within {max}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_distant_formula_does_not_rewrite_ordinary_speech() {
        for hypotheses in [
            ["предел терпения", "метан", "карбонат кальция"].as_slice(),
            ["вода закипела в чайнике", "вода"].as_slice(),
            ["серная кислота хранится в лаборатории", "серная кислота"].as_slice(),
            ["гырдксд жлз тр", "гидроксид железа три"].as_slice(),
        ] {
            let (choice, text) = chosen(hypotheses);
            assert_eq!(choice.index, 0, "{}", hypotheses[0]);
            assert_eq!(choice.reason, ChoiceReason::NothingNearParsed);
            assert_eq!(text, hypotheses[0]);
            assert_eq!(unicode(text), hypotheses[0]);
        }
    }

    #[test]
    fn the_same_list_chooses_the_same_hypothesis() {
        let hypotheses = ["карбанат кальция", "карбонат кальция"];
        assert_eq!(
            choose_hypothesis(&hypotheses),
            choose_hypothesis(&hypotheses)
        );
    }

    #[test]
    fn an_empty_list_names_no_hypothesis() {
        assert_eq!(
            choose_hypothesis(&[]),
            Choice {
                index: 0,
                reason: ChoiceReason::Empty,
            }
        );
    }

    #[test]
    fn case_and_spacing_do_not_count_as_a_different_hypothesis() {
        let (choice, _) = chosen(&["Карбанат   кальция", "карбонат кальция"]);
        assert_eq!(choice.reason, ChoiceReason::NearHypothesisParsed);
        assert_eq!(choice.index, 1);
    }
}

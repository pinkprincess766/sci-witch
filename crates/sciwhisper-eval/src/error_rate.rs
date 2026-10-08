//! Word and character error rates.
//!
//! WER is for transcripts of real speakers (stage 2 of the plan); CER over a
//! LaTeX string is the headline metric of Speech2Latex, which this project
//! will be compared against. Neither is in a report yet: on the text corpora
//! the "transcript" is the reference itself, so the rate would be 0 by
//! construction and say nothing.
//!
//! The numbers are only comparable with someone else's if the input is
//! normalised the same way, so normalisation is one named function per
//! metric, and each says exactly what it changes.

use sciwhisper_core::{render, Node, Renderer};

/// Longest sequence, on either side, that [`edit_distance`] accepts.
///
/// The alignment keeps one row of the dynamic-programming table, so memory
/// is linear, but time is the product of the two lengths. Ten thousand
/// words is far beyond one utterance; past it the call is refused instead
/// of running for minutes.
pub const MAX_EDIT_LEN: usize = 10_000;

/// A side of the alignment that is longer than [`MAX_EDIT_LEN`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooLong {
    Reference(usize),
    Hypothesis(usize),
}

/// Operations of one minimal alignment of a hypothesis to a reference.
///
/// The total `substitutions + insertions + deletions` is the Levenshtein
/// distance and does not depend on which minimal alignment was taken. The
/// split between the three can: when several alignments tie, the one kept
/// prefers a match or substitution, then a deletion, then an insertion, at
/// each step from the end of both sequences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditCounts {
    pub substitutions: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub reference_len: usize,
}

impl EditCounts {
    pub fn errors(&self) -> usize {
        self.substitutions + self.insertions + self.deletions
    }
}

/// Levenshtein alignment of `hypothesis` to `reference` with the three kinds
/// of edit counted separately.
///
/// Each cell of the table carries the counts of the alignment that reached
/// it, so one row is enough: no full matrix and no traceback. Memory is
/// `O(hypothesis.len())`.
pub fn edit_distance<T: PartialEq>(
    reference: &[T],
    hypothesis: &[T],
) -> Result<EditCounts, TooLong> {
    if reference.len() > MAX_EDIT_LEN {
        return Err(TooLong::Reference(reference.len()));
    }
    if hypothesis.len() > MAX_EDIT_LEN {
        return Err(TooLong::Hypothesis(hypothesis.len()));
    }
    // row[j]: the best alignment of the reference prefix seen so far to
    // hypothesis[..j]. Before any reference item, that is j insertions.
    let mut row: Vec<EditCounts> = (0..=hypothesis.len())
        .map(|j| EditCounts {
            insertions: j,
            ..EditCounts::default()
        })
        .collect();
    for (i, wanted) in reference.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = EditCounts {
            deletions: i + 1,
            ..EditCounts::default()
        };
        for (j, heard) in hypothesis.iter().enumerate() {
            let mut via_diagonal = diagonal;
            if wanted != heard {
                via_diagonal.substitutions += 1;
            }
            let mut via_deletion = row[j + 1];
            via_deletion.deletions += 1;
            let mut via_insertion = row[j];
            via_insertion.insertions += 1;

            let mut best = via_diagonal;
            if via_deletion.errors() < best.errors() {
                best = via_deletion;
            }
            if via_insertion.errors() < best.errors() {
                best = via_insertion;
            }
            diagonal = row[j + 1];
            row[j + 1] = best;
        }
    }
    let mut counts = row[hypothesis.len()];
    counts.reference_len = reference.len();
    Ok(counts)
}

/// An error rate: `(S + D + I) / N`, `N` the length of the reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorRate {
    pub counts: EditCounts,
    /// `None` for an empty reference. The rate is undefined there, and a
    /// hypothesis that inserted words must not read as 0%.
    pub value: Option<f64>,
}

impl ErrorRate {
    fn from_counts(counts: EditCounts) -> Self {
        let value = (counts.reference_len > 0)
            .then(|| counts.errors() as f64 / counts.reference_len as f64);
        Self { counts, value }
    }
}

/// The normalisation shared by [`word_error_rate`] and [`char_error_rate`],
/// and nothing more:
///
/// * lower case (Unicode `to_lowercase`);
/// * `ё` → `е`, the one spelling variant Russian text uses freely;
/// * every run of whitespace becomes one space, and the ends are trimmed.
///
/// Punctuation is kept: whether «,» is an error is the caller's decision,
/// not this function's.
pub fn normalize_for_error_rate(text: &str) -> String {
    text.to_lowercase()
        .replace('ё', "е")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Word error rate. Words are the whitespace-separated pieces of
/// [`normalize_for_error_rate`]. It can exceed 1 when the hypothesis is
/// longer than the reference.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> Result<ErrorRate, TooLong> {
    let reference = normalize_for_error_rate(reference);
    let hypothesis = normalize_for_error_rate(hypothesis);
    let reference: Vec<&str> = reference.split(' ').filter(|w| !w.is_empty()).collect();
    let hypothesis: Vec<&str> = hypothesis.split(' ').filter(|w| !w.is_empty()).collect();
    edit_distance(&reference, &hypothesis).map(ErrorRate::from_counts)
}

/// Character error rate over Unicode scalar values (`chars()`) of
/// [`normalize_for_error_rate`]. A space between words is one character.
pub fn char_error_rate(reference: &str, hypothesis: &str) -> Result<ErrorRate, TooLong> {
    let reference: Vec<char> = normalize_for_error_rate(reference).chars().collect();
    let hypothesis: Vec<char> = normalize_for_error_rate(hypothesis).chars().collect();
    edit_distance(&reference, &hypothesis).map(ErrorRate::from_counts)
}

/// LaTeX with its spacing removed, and only its spacing:
///
/// * whitespace;
/// * `~` (tie);
/// * `\,` `\;` `\!` (thin, thick and negative thin space).
///
/// Case is kept: `X` and `x` are different symbols in a formula. A
/// backslash always takes the next character with it, so `\\,` is a line
/// break followed by a comma, not a thin space, and `\~` stays an accent.
/// Other spacing commands — `\:`, `\ `, `\quad` — are kept as written.
pub fn normalize_latex_spacing(latex: &str) -> String {
    let mut out = String::with_capacity(latex.len());
    let mut chars = latex.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(',' | ';' | '!') => {}
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            },
            '~' => {}
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// Character error rate over [`normalize_latex_spacing`] of two LaTeX
/// strings, for comparison with Speech2Latex.
pub fn latex_char_error_rate(reference: &str, hypothesis: &str) -> Result<ErrorRate, TooLong> {
    let reference: Vec<char> = normalize_latex_spacing(reference).chars().collect();
    let hypothesis: Vec<char> = normalize_latex_spacing(hypothesis).chars().collect();
    edit_distance(&reference, &hypothesis).map(ErrorRate::from_counts)
}

/// [`latex_char_error_rate`] with the hypothesis taken from our own AST.
///
/// The hypothesis is `sciwhisper_core::render(node, Renderer::Latex)`, the
/// same call the eval harness makes for its LaTeX output. A `Node::Text`
/// renders as its raw text. Both strings then go through
/// [`normalize_latex_spacing`], as in [`latex_char_error_rate`].
pub fn latex_char_error_rate_of_node(
    reference_latex: &str,
    node: &Node,
) -> Result<ErrorRate, TooLong> {
    let hypothesis = render(node, Renderer::Latex);
    latex_char_error_rate(reference_latex, &hypothesis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_core::{interpret, Domain, InterpretOptions};

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn counts(
        substitutions: usize,
        insertions: usize,
        deletions: usize,
        reference_len: usize,
    ) -> EditCounts {
        EditCounts {
            substitutions,
            insertions,
            deletions,
            reference_len,
        }
    }

    #[test]
    fn identical_sequences_have_no_errors() {
        assert_eq!(
            edit_distance(&chars("формула"), &chars("формула")),
            Ok(counts(0, 0, 0, 7))
        );
        assert_eq!(edit_distance::<char>(&[], &[]), Ok(counts(0, 0, 0, 0)));
    }

    #[test]
    fn kitten_to_sitting_is_two_substitutions_and_one_insertion() {
        // k→s, e→i, and g appended: 3. No alignment with fewer than two
        // substitutions exists, because the longest common subsequence
        // («ittn») has 4 letters.
        assert_eq!(
            edit_distance(&chars("kitten"), &chars("sitting")),
            Ok(counts(2, 1, 0, 6))
        );
        // The other way round the insertion becomes a deletion.
        assert_eq!(
            edit_distance(&chars("sitting"), &chars("kitten")),
            Ok(counts(2, 0, 1, 7))
        );
    }

    #[test]
    fn russian_words_with_one_edit_of_each_kind() {
        // reference: сила равна массе умноженной на ускорение   (6 words)
        // heard:     сила равно массе            на ускорение свободного
        // равна→равно (S), умноженной dropped (D), свободного added (I):
        // 3 errors over 6 words.
        let rate = word_error_rate(
            "сила равна массе умноженной на ускорение",
            "сила равно массе на ускорение свободного",
        )
        .unwrap();
        assert_eq!(rate.counts, counts(1, 1, 1, 6));
        assert_eq!(rate.value, Some(0.5));
    }

    #[test]
    fn a_counter_of_substitutions_alone_would_miss_these() {
        // Same length, so a position-by-position comparison sees four
        // substitutions; the alignment drops «a» and adds «e»: 2.
        assert_eq!(
            edit_distance(&chars("abcd"), &chars("bcde")),
            Ok(counts(0, 1, 1, 4))
        );
        // Different length: there is no position-by-position answer at all.
        assert_eq!(
            edit_distance(&chars("икс"), &chars("икс два")),
            Ok(counts(0, 4, 0, 3))
        );
    }

    #[test]
    fn the_word_error_rate_can_exceed_one() {
        // One word heard as three: 1 substitution + 2 insertions over 1.
        let rate = word_error_rate("синус", "сын у с").unwrap();
        assert_eq!(rate.counts, counts(1, 2, 0, 1));
        assert_eq!(rate.value, Some(3.0));
    }

    #[test]
    fn an_empty_reference_has_no_rate() {
        let rate = word_error_rate("", "лишние слова").unwrap();
        assert_eq!(rate.counts, counts(0, 2, 0, 0));
        assert_eq!(rate.value, None);
        assert_eq!(char_error_rate("  ", "").unwrap().value, None);
    }

    #[test]
    fn normalisation_is_case_yo_and_whitespace_only() {
        assert_eq!(word_error_rate("Ёж", "еж").unwrap().value, Some(0.0));
        assert_eq!(char_error_rate("Ёж", "еж").unwrap().value, Some(0.0));
        assert_eq!(
            normalize_for_error_rate("  Два\tМоля \n ЁЖ "),
            "два моля еж"
        );
        // Punctuation is not normalisation's business.
        assert_eq!(
            word_error_rate("да, нет", "да нет").unwrap().counts,
            counts(1, 0, 0, 2)
        );
        // A space between words is one character, however it was typed.
        assert_eq!(char_error_rate("а б", "а   б").unwrap().value, Some(0.0));
        assert_eq!(
            char_error_rate("а б", "аб").unwrap().counts,
            counts(0, 0, 1, 3)
        );
    }

    #[test]
    fn latex_spacing_is_removed_and_nothing_else() {
        assert_eq!(
            latex_char_error_rate("x^{2} + 1", "x^{2}+1").unwrap().value,
            Some(0.0)
        );
        assert_eq!(
            latex_char_error_rate(r"a\,b\;c\!d~e", "abcde")
                .unwrap()
                .value,
            Some(0.0)
        );
        // x^{2} vs x^{3}: one substitution over 5 characters.
        let rate = latex_char_error_rate("x^{2}", "x^{3}").unwrap();
        assert_eq!(rate.counts, counts(1, 0, 0, 5));
        // Case is a different symbol.
        assert_eq!(
            latex_char_error_rate("X", "x").unwrap().counts,
            counts(1, 0, 0, 1)
        );
        // `\\,` is a line break and a comma; `\~` is an accent; `\:` and
        // `\quad` are kept. Whitespace still goes, even after a control
        // word: this is a character count over both sides alike, not a
        // rewrite of the LaTeX.
        assert_eq!(normalize_latex_spacing(r"a\\,b"), r"a\\,b");
        assert_eq!(normalize_latex_spacing(r"\~{n}"), r"\~{n}");
        assert_eq!(normalize_latex_spacing(r"a\:b\quad c"), r"a\:b\quadc");
        assert_eq!(normalize_latex_spacing("\\"), "\\");
    }

    fn parsed_maths(spoken: &str) -> Node {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain: Domain::Mathematics,
                allow_shortcuts: true,
            },
        );
        assert!(
            result.confidence > 0.0 && !matches!(result.ast, Node::Text(_)),
            "{spoken:?} did not parse"
        );
        result.ast
    }

    #[test]
    fn a_node_rendered_as_latex_matches_its_reference_up_to_spacing() {
        let node = parsed_maths("икс в квадрате плюс один");
        // The reference was written by hand after reading the renderer's
        // output for this node, `x^{2} + 1`. It differs only in spacing, on
        // purpose, to check that spacing is ignored.
        let rate = latex_char_error_rate_of_node("x^{2}+1", &node).unwrap();
        assert_eq!(rate.counts, counts(0, 0, 0, 7));
        assert_eq!(rate.value, Some(0.0));
    }

    #[test]
    fn one_changed_character_is_one_substitution_over_the_reference() {
        let node = parsed_maths("икс в квадрате плюс один");
        // `x^{3}+1` against the rendered `x^{2}+1`: one substitution over
        // the 7 characters of the reference.
        let rate = latex_char_error_rate_of_node("x^{3}+1", &node).unwrap();
        assert_eq!(rate.counts, counts(1, 0, 0, 7));
        assert_eq!(rate.value, Some(1.0 / 7.0));
    }

    #[test]
    fn the_length_limit_holds_on_both_sides() {
        let at_limit = vec![0u8; MAX_EDIT_LEN];
        let over = vec![0u8; MAX_EDIT_LEN + 1];
        assert_eq!(
            edit_distance(&at_limit, &[0u8]),
            Ok(counts(0, 0, MAX_EDIT_LEN - 1, MAX_EDIT_LEN))
        );
        assert_eq!(
            edit_distance(&[0u8], &at_limit),
            Ok(counts(0, MAX_EDIT_LEN - 1, 0, 1))
        );
        assert_eq!(
            edit_distance(&over, &[0u8]),
            Err(TooLong::Reference(MAX_EDIT_LEN + 1))
        );
        assert_eq!(
            edit_distance(&[0u8], &over),
            Err(TooLong::Hypothesis(MAX_EDIT_LEN + 1))
        );
    }

    /// Full-matrix Levenshtein, the textbook way: an independent check of
    /// the one-row version's total.
    fn textbook(a: &[u8], b: &[u8]) -> usize {
        let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for (i, row) in d.iter_mut().enumerate() {
            row[0] = i;
        }
        for (j, cell) in d[0].iter_mut().enumerate() {
            *cell = j;
        }
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                let substitution = d[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]);
                d[i][j] = substitution.min(d[i - 1][j] + 1).min(d[i][j - 1] + 1);
            }
        }
        d[a.len()][b.len()]
    }

    #[test]
    fn the_counts_agree_with_the_textbook_distance_and_the_lengths() {
        let mut state = 0x5eed_u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % n
        };
        for _ in 0..2000 {
            let a: Vec<u8> = (0..next(9)).map(|_| next(3) as u8).collect();
            let b: Vec<u8> = (0..next(9)).map(|_| next(3) as u8).collect();
            let got = edit_distance(&a, &b).unwrap();
            assert_eq!(got.errors(), textbook(&a, &b), "{a:?} → {b:?}");
            // Every reference item is matched, substituted or deleted, and
            // every hypothesis item is matched, substituted or inserted.
            assert_eq!(
                got.reference_len - got.deletions + got.insertions,
                b.len(),
                "{a:?} → {b:?}"
            );
            assert_eq!(got.reference_len, a.len());
        }
    }
}

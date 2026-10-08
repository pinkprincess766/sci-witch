//! Alignment of two sequences as a list of operations, so a report can show
//! which word was substituted, inserted or deleted.
//!
//! The counts of [`crate::error_rate::edit_distance`] and the operations here
//! come from the same tie-break, so the two always agree. The tests check
//! that on random sequences.

use crate::error_rate::{normalize_for_error_rate, TooLong};

/// Longest sequence, on either side, that [`align`] accepts.
///
/// The traceback needs the whole `(n + 1) × (m + 1)` table of distances, one
/// `usize` per cell. At this limit that is 2001 × 2001 × 8 bytes, about 32 MB.
/// At [`crate::error_rate::MAX_EDIT_LEN`] it would be about 800 MB, which is
/// why this limit is lower.
pub const MAX_ALIGN_LEN: usize = 2_000;

/// One step of an alignment that turns the hypothesis into the reference.
///
/// Read in order, `Match`, `Substitute` and `Delete` consume a reference
/// item, and `Match`, `Substitute` and `Insert` consume a hypothesis item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlignOp<T> {
    /// Equal items.
    Match(T),
    /// A reference item heard as a different one.
    Substitute { reference: T, hypothesis: T },
    /// A reference item missing from the hypothesis.
    Delete(T),
    /// A hypothesis item with no reference item.
    Insert(T),
}

/// Minimal alignment of `hypothesis` to `reference`, in reading order.
///
/// `table[i][j]` is the distance between `reference[..i]` and
/// `hypothesis[..j]`. The traceback starts at the end of both sequences and,
/// at each cell, takes the first step that lies on a minimal path: a match or
/// substitution, then a deletion, then an insertion. This is the order that
/// [`crate::error_rate::edit_distance`] breaks ties in, so the operations
/// give the same substitutions, insertions and deletions as its counts.
pub fn align<T: PartialEq + Clone>(
    reference: &[T],
    hypothesis: &[T],
) -> Result<Vec<AlignOp<T>>, TooLong> {
    if reference.len() > MAX_ALIGN_LEN {
        return Err(TooLong::Reference(reference.len()));
    }
    if hypothesis.len() > MAX_ALIGN_LEN {
        return Err(TooLong::Hypothesis(hypothesis.len()));
    }
    let n = reference.len();
    let m = hypothesis.len();
    let width = m + 1;
    // Row 0 and column 0: the distance to an empty sequence.
    let mut table = vec![0usize; (n + 1) * width];
    for (j, cell) in table[..width].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 0..=n {
        table[i * width] = i;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(reference[i - 1] != hypothesis[j - 1]);
            table[i * width + j] = (table[(i - 1) * width + j - 1] + cost)
                .min(table[(i - 1) * width + j] + 1)
                .min(table[i * width + j - 1] + 1);
        }
    }
    let at = |i: usize, j: usize| table[i * width + j];

    let (mut i, mut j) = (n, m);
    let mut ops = Vec::with_capacity(n.max(m));
    while i > 0 || j > 0 {
        let here = at(i, j);
        if i > 0 && j > 0 {
            let same = reference[i - 1] == hypothesis[j - 1];
            if at(i - 1, j - 1) + usize::from(!same) == here {
                ops.push(if same {
                    AlignOp::Match(reference[i - 1].clone())
                } else {
                    AlignOp::Substitute {
                        reference: reference[i - 1].clone(),
                        hypothesis: hypothesis[j - 1].clone(),
                    }
                });
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && at(i - 1, j) + 1 == here {
            ops.push(AlignOp::Delete(reference[i - 1].clone()));
            i -= 1;
            continue;
        }
        // Not a diagonal step and not a deletion, so an insertion. The first
        // row and column are the distance to the empty sequence, so j > 0.
        ops.push(AlignOp::Insert(hypothesis[j - 1].clone()));
        j -= 1;
    }
    ops.reverse();
    Ok(ops)
}

/// [`align`] over the words of two strings. Words are split the same way as
/// in `word_error_rate`: after `normalize_for_error_rate`, on single spaces.
pub fn align_words(reference: &str, hypothesis: &str) -> Result<Vec<AlignOp<String>>, TooLong> {
    align(&words(reference), &words(hypothesis))
}

fn words(text: &str) -> Vec<String> {
    let normalized = normalize_for_error_rate(text);
    normalized
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_rate::{edit_distance, word_error_rate, EditCounts};

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn word(s: &str) -> String {
        s.to_owned()
    }

    #[test]
    fn kitten_to_sitting_is_the_expected_script() {
        assert_eq!(
            align(&chars("kitten"), &chars("sitting")),
            Ok(vec![
                AlignOp::Substitute {
                    reference: 'k',
                    hypothesis: 's',
                },
                AlignOp::Match('i'),
                AlignOp::Match('t'),
                AlignOp::Match('t'),
                AlignOp::Substitute {
                    reference: 'e',
                    hypothesis: 'i',
                },
                AlignOp::Match('n'),
                AlignOp::Insert('g'),
            ])
        );
    }

    #[test]
    fn russian_sentence_with_one_edit_of_each_kind() {
        // The sentence of error_rate's test of the same name: равна→равно (S),
        // умноженной dropped (D), свободного added (I).
        let reference = "сила равна массе умноженной на ускорение";
        let hypothesis = "сила равно массе на ускорение свободного";
        assert_eq!(
            align_words(reference, hypothesis),
            Ok(vec![
                AlignOp::Match(word("сила")),
                AlignOp::Substitute {
                    reference: word("равна"),
                    hypothesis: word("равно"),
                },
                AlignOp::Match(word("массе")),
                AlignOp::Delete(word("умноженной")),
                AlignOp::Match(word("на")),
                AlignOp::Match(word("ускорение")),
                AlignOp::Insert(word("свободного")),
            ])
        );
        // The same words and the same tie-break as word_error_rate.
        assert_eq!(
            word_error_rate(reference, hypothesis).unwrap().counts,
            EditCounts {
                substitutions: 1,
                insertions: 1,
                deletions: 1,
                reference_len: 6,
            }
        );
    }

    #[test]
    fn a_swap_is_two_substitutions_not_a_deletion_and_an_insertion() {
        // Both are minimal with 2 errors, so the tie-break decides: the
        // diagonal step (substitution) comes before deletion and insertion.
        assert_eq!(
            align(&chars("ab"), &chars("ba")),
            Ok(vec![
                AlignOp::Substitute {
                    reference: 'a',
                    hypothesis: 'b',
                },
                AlignOp::Substitute {
                    reference: 'b',
                    hypothesis: 'a',
                },
            ])
        );
    }

    #[test]
    fn empty_sides() {
        assert_eq!(align::<char>(&[], &[]), Ok(vec![]));
        assert_eq!(
            align(&chars("ab"), &[]),
            Ok(vec![AlignOp::Delete('a'), AlignOp::Delete('b')])
        );
        assert_eq!(
            align(&[], &chars("ab")),
            Ok(vec![AlignOp::Insert('a'), AlignOp::Insert('b')])
        );
        assert_eq!(align_words("", "  "), Ok(vec![]));
    }

    #[test]
    fn words_are_normalised_before_they_are_compared() {
        // Case and ё are not differences, so both words match. Without the
        // normalisation «Ёж» and «еж» would be a substitution.
        assert_eq!(
            align_words("Ёж  ДОМА", "еж дома"),
            Ok(vec![
                AlignOp::Match(word("еж")),
                AlignOp::Match(word("дома"))
            ])
        );
    }

    #[test]
    fn the_limit_holds_on_both_sides() {
        // 2000 × 1 and 1 × 2000, not 2000 × 2000: the table has (n+1)(m+1)
        // cells, so the one-word side keeps the test cheap.
        let at_limit = vec![0u8; MAX_ALIGN_LEN];
        let over = vec![0u8; MAX_ALIGN_LEN + 1];
        let one = [0u8];
        assert_eq!(align(&at_limit, &one).unwrap().len(), MAX_ALIGN_LEN);
        assert_eq!(align(&one, &at_limit).unwrap().len(), MAX_ALIGN_LEN);
        assert_eq!(
            align(&over, &one),
            Err(TooLong::Reference(MAX_ALIGN_LEN + 1))
        );
        assert_eq!(
            align(&one, &over),
            Err(TooLong::Hypothesis(MAX_ALIGN_LEN + 1))
        );
    }

    #[test]
    fn counts_and_replay_agree_with_edit_distance_on_random_sequences() {
        let mut state = 0x5eed_u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % n
        };
        for _ in 0..3000 {
            let a: Vec<u8> = (0..next(9)).map(|_| next(3) as u8).collect();
            let b: Vec<u8> = (0..next(9)).map(|_| next(3) as u8).collect();
            let ops = align(&a, &b).unwrap();

            let mut substitutions = 0;
            let mut insertions = 0;
            let mut deletions = 0;
            let mut reference_back = Vec::new();
            let mut hypothesis_back = Vec::new();
            for op in &ops {
                match op {
                    AlignOp::Match(x) => {
                        reference_back.push(*x);
                        hypothesis_back.push(*x);
                    }
                    AlignOp::Substitute {
                        reference,
                        hypothesis,
                    } => {
                        substitutions += 1;
                        reference_back.push(*reference);
                        hypothesis_back.push(*hypothesis);
                    }
                    AlignOp::Delete(x) => {
                        deletions += 1;
                        reference_back.push(*x);
                    }
                    AlignOp::Insert(x) => {
                        insertions += 1;
                        hypothesis_back.push(*x);
                    }
                }
            }
            let counts = EditCounts {
                substitutions,
                insertions,
                deletions,
                reference_len: a.len(),
            };
            assert_eq!(Ok(counts), edit_distance(&a, &b), "{a:?} → {b:?}");
            assert_eq!(reference_back, a, "{a:?} → {b:?}");
            assert_eq!(hypothesis_back, b, "{a:?} → {b:?}");
        }
    }
}

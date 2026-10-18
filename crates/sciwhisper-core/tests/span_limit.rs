//! A scientific span stops at [`MAX_SPAN_WORDS`].

use sciwhisper_core::utterance::MAX_SPAN_WORDS;
use sciwhisper_core::{interpret_utterance, UtteranceOptions};

fn chain(extra_term: bool) -> String {
    // 31 × «икс плюс» is 62 words. «два икс» makes 64 and ends on a word the
    // search is allowed to stop on («плюс» is dangling). One more «икс»
    // makes 65, and that whole chain still parses.
    let mut text = "икс плюс ".repeat(31);
    text.push_str("два икс");
    if extra_term {
        text.push_str(" икс");
    }
    text
}

fn span_lengths(text: &str) -> Vec<usize> {
    interpret_utterance(text, UtteranceOptions::default())
        .spans
        .iter()
        .map(|span| span.source_text.split_whitespace().count())
        .collect()
}

#[test]
fn a_span_of_64_words_is_one_fragment_and_65_does_not_grow_past_it() {
    let at = chain(false);
    assert_eq!(at.split_whitespace().count(), MAX_SPAN_WORDS);
    assert_eq!(span_lengths(&at), vec![MAX_SPAN_WORDS]);

    // A bare chain of 65 words parses as one utterance, and that path runs
    // before the span cap. The cap is the search inside surrounding speech:
    // `grow_span` stops at `word + MAX_SPAN_WORDS`.
    let over = format!("вчера {} закончили", chain(true));
    assert_eq!(chain(true).split_whitespace().count(), MAX_SPAN_WORDS + 1);
    let result = interpret_utterance(&over, UtteranceOptions::default());
    let lengths: Vec<usize> = result
        .spans
        .iter()
        .map(|span| span.source_text.split_whitespace().count())
        .collect();
    assert!(
        lengths.contains(&MAX_SPAN_WORDS),
        "the formula inside the sentence was not found: {lengths:?}"
    );
    assert!(
        lengths.iter().all(|len| *len <= MAX_SPAN_WORDS),
        "a span grew past {MAX_SPAN_WORDS}: {lengths:?}"
    );
    for span in &result.spans {
        assert!(
            !span.source_text.contains("вчера") && !span.source_text.contains("закончили"),
            "prose was taken into a formula: {}",
            span.source_text
        );
    }
}

#[test]
fn prose_around_a_formula_stays_prose() {
    let text = format!("вчера {} закончили", chain(false));
    let result = interpret_utterance(&text, UtteranceOptions::default());
    assert!(
        result
            .spans
            .iter()
            .any(|span| span.source_text.split_whitespace().count() == MAX_SPAN_WORDS),
        "the formula was not found: {:?}",
        result.spans
    );
    for span in &result.spans {
        assert!(
            !span.source_text.contains("вчера") && !span.source_text.contains("закончили"),
            "prose was taken into a formula: {}",
            span.source_text
        );
    }
}

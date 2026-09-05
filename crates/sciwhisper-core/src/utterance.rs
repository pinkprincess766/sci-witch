//! Utterance-level interpretation: natural Russian speech → one document AST.
//!
//! [`interpret`](crate::interpret) answers a narrow question — "is this whole
//! string one scientific construct?" — and it stays exactly as strict as it
//! was. This module answers the question a person actually asks: "here is
//! something I said out loud; write down the science in it."
//!
//! The pipeline is
//!
//! ```text
//! speech → tokens with byte ranges → speech segments → scientific candidates
//!        → one Document AST → one result → every renderer
//! ```
//!
//! There is deliberately no second set of string rewrites per output format:
//! Unicode, LaTeX and OMML are all rendered from the same tree, so they cannot
//! drift apart.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::ast::{Domain, Node, Warning};
use crate::interpret::{interpret, InterpretOptions};

const DICTATION_YAML: &str = include_str!("../data/domains/common/dictation.yaml");
const DICTATION_SCHEMA: u32 = 1;

/// Longest utterance this layer will look at. Beyond it the text is kept
/// verbatim: a dictation that long is not one formula, and the quadratic span
/// search is not worth running on it.
pub const MAX_UTTERANCE_WORDS: usize = 400;
/// Longest span the search will try to parse, in words.
pub const MAX_SPAN_WORDS: usize = 64;
/// Parse attempts allowed for one utterance. A hard ceiling keeps a hostile
/// input from turning into an unbounded amount of work.
pub const MAX_PARSE_ATTEMPTS: usize = 4096;
/// Corrections accepted for one span. A person restating a value four times
/// is no longer correcting; the transcript is kept instead.
pub const MAX_CORRECTIONS: usize = 4;

/// Rewriting a stretch *inside* a sentence is held to the stricter bar: a
/// parse that the grammar itself called ambiguous stays as words.
const INLINE_MIN_CONFIDENCE: f32 = 0.9;
/// A whole utterance is what the speaker dictated on purpose, so an ambiguous
/// but complete parse is still shown — with its warnings and alternatives
/// attached, and with the lower confidence that says so.
const WHOLE_UTTERANCE_MIN_CONFIDENCE: f32 = 0.7;

/// What the caller wants done with the ordinary words around the science.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtteranceMode {
    /// Keep every ordinary word; replace only the spans that were proven to be
    /// scientific. This is the default: it can add a formula, never delete a
    /// sentence.
    MixedText,
    /// Drop the recognised dictation shell («ну запиши…») and keep the
    /// scientific construct alone. Ordinary prose that is *not* a recognised
    /// shell still survives — the mode strips a proven wrapper, not anything
    /// it failed to understand.
    ScientificOnly,
}

impl UtteranceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            UtteranceMode::MixedText => "mixed_text",
            UtteranceMode::ScientificOnly => "scientific_only",
        }
    }
}

impl std::str::FromStr for UtteranceMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mixed" | "mixed_text" | "mixed-text" | "смешанный" => {
                Ok(UtteranceMode::MixedText)
            }
            "scientific" | "scientific_only" | "scientific-only" | "научный" => {
                Ok(UtteranceMode::ScientificOnly)
            }
            other => Err(format!("unknown dictation mode '{other}'")),
        }
    }
}

/// The verdict, stated as a decision rather than hidden inside a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// At least one span was proven scientific and nothing was ambiguous.
    Accepted,
    /// Two readings were equally well supported. The words are kept.
    Ambiguous,
    /// A scientific shape was attempted and refused.
    Rejected,
    /// Nothing scientific was attempted; this is ordinary speech.
    Raw,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Ambiguous => "ambiguous",
            Decision::Rejected => "rejected",
            Decision::Raw => "raw",
        }
    }
}

#[derive(Clone, Debug)]
pub struct UtteranceOptions {
    pub domain: Domain,
    pub mode: UtteranceMode,
    pub allow_shortcuts: bool,
}

impl Default for UtteranceOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        }
    }
}

/// How a speech segment was classified. Kept in the result so a decision can
/// always be explained in terms of what was heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    PlainText,
    Framing,
    Filler,
    ScientificSpan,
    CorrectionMarker,
    Separator,
    Boundary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedCorrection {
    pub marker: String,
    pub repair_text: String,
    /// The span text after the correction was applied.
    pub result_text: String,
}

/// One stretch of speech that was proven to be science.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScienceSpan {
    pub start: usize,
    pub end: usize,
    pub source_text: String,
    pub normalized: String,
    pub domain: Domain,
    pub node: Node,
    pub warnings: Vec<Warning>,
    pub alternatives: Vec<Node>,
    pub decision: Decision,
    /// Why this reading was accepted, in plain words.
    pub evidence: Vec<String>,
    pub confidence: f32,
    pub corrections: Vec<AppliedCorrection>,
}

/// A stretch that looked scientific and was refused, with the reason.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RejectedSpan {
    pub start: usize,
    pub end: usize,
    pub source_text: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UtteranceResult {
    /// The one structure every renderer works from.
    pub document: Node,
    pub raw_transcript: String,
    pub mode: UtteranceMode,
    pub decision: Decision,
    pub segments: Vec<Segment>,
    pub spans: Vec<ScienceSpan>,
    pub rejected: Vec<RejectedSpan>,
    pub warnings: Vec<Warning>,
    /// Kept for compatibility with the existing result type. It is a
    /// deterministic parse level, not a probability, and `decision` is what
    /// callers should branch on.
    pub confidence: f32,
}

impl UtteranceResult {
    /// True when the document is nothing but the original words.
    pub fn is_raw(&self) -> bool {
        self.spans.is_empty()
    }

    /// True when nothing but science survived, so one native Word equation can
    /// carry the whole answer.
    pub fn is_pure_science(&self) -> bool {
        match &self.document {
            Node::Text(_) => false,
            Node::Document(children) => {
                !children.is_empty() && !children.iter().any(|c| matches!(c, Node::Text(_)))
            }
            _ => true,
        }
    }

    /// The narrow result type the shell and the CLI already speak.
    ///
    /// A partially compiled utterance is no longer reported as a total
    /// failure: the spans that did compile are in the document, and the
    /// confidence says so.
    pub fn to_interpretation(&self, domain: Domain) -> crate::ast::InterpretationResult {
        crate::ast::InterpretationResult {
            ast: self.document.clone(),
            raw_transcript: self.raw_transcript.clone(),
            normalized_transcript: crate::normalize::normalize(&self.raw_transcript),
            domain: self.spans.first().map(|span| span.domain).unwrap_or(domain),
            confidence: self.confidence,
            unresolved_spans: self
                .rejected
                .iter()
                .map(|span| crate::ast::UnresolvedSpan {
                    text: span.source_text.clone(),
                    reason: span.reason.clone(),
                })
                .collect(),
            warnings: self.warnings.clone(),
            alternatives: self
                .spans
                .iter()
                .flat_map(|span| span.alternatives.iter().cloned())
                .collect(),
        }
    }
}

// ------------------------------------------------------------------ tokens

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenKind {
    Word,
    /// Comma and dash: a pause, not a sentence end.
    Separator,
    /// `.`, `!`, `?`, `;`, `:` — a scientific span never crosses one.
    Boundary,
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
    /// Normalised form for a word; the raw character for punctuation.
    text: String,
}

/// Splits the text into words and punctuation, keeping every byte range so
/// that an accepted span can be spliced back into the original transcript.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word_start: Option<usize> = None;
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            word_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = word_start.take() {
            push_word(&mut tokens, text, start, index);
        }
        let kind = match character {
            // A colon introduces the thing that was just announced
            // («запиши дробь: в числителе …»), so it is a pause, not a stop.
            '.' | '!' | '?' | ';' => TokenKind::Boundary,
            ':' => TokenKind::Separator,
            ',' | '-' | '—' | '–' | '…' => TokenKind::Separator,
            _ => continue,
        };
        tokens.push(Token {
            kind,
            start: index,
            end: index + character.len_utf8(),
            text: character.to_string(),
        });
    }
    if let Some(start) = word_start {
        push_word(&mut tokens, text, start, text.len());
    }
    tokens
}

fn push_word(tokens: &mut Vec<Token>, text: &str, start: usize, end: usize) {
    let raw = &text[start..end];
    // One raw word may normalise into several (`2x` → `2`, `x`); for span
    // bookkeeping the whole run stays one token carrying the joined form.
    let normalized = crate::normalize::words(raw).join(" ");
    if normalized.is_empty() {
        return;
    }
    tokens.push(Token {
        kind: TokenKind::Word,
        start,
        end,
        text: normalized,
    });
}

// ------------------------------------------------------- dictation lexicon

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DictationFile {
    schema_version: u32,
    framing: Vec<String>,
    fillers: Vec<String>,
    corrections: CorrectionsFile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionsFile {
    restate: Vec<String>,
    substitute_open: Vec<String>,
    substitute_pivot: Vec<String>,
}

#[derive(Debug, Default)]
struct DictationSpeech {
    framing: Vec<Vec<String>>,
    fillers: Vec<String>,
    restate: Vec<Vec<String>>,
    substitute_open: Vec<String>,
    substitute_pivot: Vec<String>,
}

impl DictationSpeech {
    fn framing_at(&self, words: &[&str], i: usize) -> Option<usize> {
        self.framing
            .iter()
            .find_map(|phrase| matches_at(words, i, phrase).then_some(phrase.len()))
    }

    fn restate_at(&self, words: &[&str], i: usize) -> Option<usize> {
        self.restate
            .iter()
            .find_map(|phrase| matches_at(words, i, phrase).then_some(phrase.len()))
    }

    fn is_filler(&self, word: &str) -> bool {
        self.fillers.iter().any(|filler| filler == word)
    }

    fn is_substitute_open(&self, word: &str) -> bool {
        self.substitute_open.iter().any(|item| item == word)
    }

    fn is_substitute_pivot(&self, word: &str) -> bool {
        self.substitute_pivot.iter().any(|item| item == word)
    }
}

fn matches_at(words: &[&str], i: usize, phrase: &[String]) -> bool {
    i + phrase.len() <= words.len()
        && phrase
            .iter()
            .enumerate()
            .all(|(k, expected)| words[i + k] == expected)
}

fn dictation() -> &'static DictationSpeech {
    static SPEECH: OnceLock<DictationSpeech> = OnceLock::new();
    SPEECH.get_or_init(|| {
        let file: DictationFile =
            serde_yaml::from_str(DICTATION_YAML).expect("embedded dictation.yaml must be valid");
        assert!(
            file.schema_version == DICTATION_SCHEMA,
            "dictation.yaml schema {} is not the supported {DICTATION_SCHEMA}",
            file.schema_version
        );
        let phrase = |text: &str| crate::normalize::words(text);
        let mut speech = DictationSpeech {
            framing: file.framing.iter().map(|f| phrase(f)).collect(),
            fillers: file
                .fillers
                .iter()
                .map(|f| crate::normalize::normalize_word(f))
                .collect(),
            restate: file.corrections.restate.iter().map(|f| phrase(f)).collect(),
            substitute_open: file
                .corrections
                .substitute_open
                .iter()
                .map(|f| crate::normalize::normalize_word(f))
                .collect(),
            substitute_pivot: file
                .corrections
                .substitute_pivot
                .iter()
                .map(|f| crate::normalize::normalize_word(f))
                .collect(),
        };
        // Longest first, so «давайте запишем» is never shadowed by a shorter
        // entry that happens to be listed earlier.
        speech
            .framing
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        speech
            .restate
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        speech.fillers.sort();
        speech
    })
}

/// Number of leading framing and filler tokens — the recognised shell of a
/// dictation command. Fillers only count while they sit inside that shell;
/// in the middle of a sentence «ну» is an ordinary word.
fn shell_len(words: &[&str]) -> (usize, bool) {
    let speech = dictation();
    let mut index = 0;
    let mut saw_framing = false;
    loop {
        if let Some(used) = speech.framing_at(words, index) {
            index += used;
            saw_framing = true;
            continue;
        }
        if index < words.len() && speech.is_filler(words[index]) {
            index += 1;
            continue;
        }
        break;
    }
    (index, saw_framing)
}

// ------------------------------------------------------------------- cues

/// Whether a scientific span may *start* at this word.
///
/// A scientific word is not by itself permission to rewrite a sentence, so an
/// unanchored span is never tried. The one exception is a span that directly
/// follows a proven dictation shell («запиши …»), where the speaker has said
/// what they want.
fn is_cue(tokens: &[Token], index: usize, domain: Domain) -> bool {
    match domain {
        Domain::Chemistry => is_chemistry_cue(tokens, index),
        Domain::Mathematics | Domain::Physics => is_math_cue(tokens, index),
        Domain::Auto | Domain::Plain => false,
    }
}

fn word_at(tokens: &[Token], index: usize) -> Option<&str> {
    tokens
        .get(index)
        .filter(|token| token.kind == TokenKind::Word)
        .map(|token| token.text.as_str())
}

/// The next token, skipping a comma but never a sentence boundary.
fn next_word_index(tokens: &[Token], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    while let Some(token) = tokens.get(cursor) {
        match token.kind {
            TokenKind::Word => return Some(cursor),
            TokenKind::Separator => cursor += 1,
            TokenKind::Boundary => return None,
        }
    }
    None
}

fn is_chemistry_cue(tokens: &[Token], index: usize) -> bool {
    let Some(word) = word_at(tokens, index) else {
        return false;
    };
    let lexicon = crate::lexicon::Lexicon::builtin();
    let systematic = [
        "гидроксид",
        "оксид",
        "хлорид",
        "сульфат",
        "сульфид",
        "нитрат",
        "нитрит",
        "карбонат",
        "фосфат",
        "перманганат",
        "силикат",
        "кислот",
        "перекись",
        "пероксид",
    ]
    .iter()
    .any(|cue| word.starts_with(cue));
    if systematic || lexicon.chemistry_speech.is_ion_marker(word) {
        return true;
    }
    if lexicon.element(word).is_some() {
        return true;
    }
    // The first word of any known substance name opens a candidate; whether
    // the rest of the name follows is decided by trying to parse it.
    if lexicon.substances.iter().any(|item| {
        item.names
            .iter()
            .filter_map(|name| name.split_whitespace().next())
            .any(|first| first == word)
    }) {
        return true;
    }
    // A stoichiometric coefficient in front of a formula.
    let numbers = crate::numbers::NumberLex::new();
    if numbers.lookup(word).is_some() || word.parse::<u32>().is_ok() {
        return next_word_index(tokens, index).is_some_and(|next| is_chemistry_cue(tokens, next));
    }
    false
}

fn is_math_cue(tokens: &[Token], index: usize) -> bool {
    let Some(word) = word_at(tokens, index) else {
        return false;
    };
    if matches!(
        word,
        "интеграл"
            | "сумма"
            | "произведение"
            | "корень"
            | "дробь"
            | "вектор"
            | "модуль"
            | "факториал"
            | "синус"
            | "синуса"
            | "косинус"
            | "косинуса"
            | "тангенс"
            | "котангенс"
            | "логарифм"
            | "логарифма"
            | "экспонента"
            | "экспоненту"
            | "экспоненты"
            | "производная"
            | "производную"
            | "производной"
            | "частная"
            | "частную"
            | "предел"
            | "дельта"
    ) {
        // «дельта» directly before a bare number is the observed Whisper
        // artefact for «ΔG 2», not a formula opening.
        if word == "дельта" {
            return !next_word_index(tokens, index)
                .and_then(|next| word_at(tokens, next))
                .is_some_and(is_number_word);
        }
        return true;
    }

    // «вторая производная …» — an ordinal introduces the construct that
    // follows it, exactly as a coefficient introduces a formula.
    if crate::numbers::NumberLex::new().ordinal(word).is_some() {
        return next_word_index(tokens, index).is_some_and(|next| is_math_cue(tokens, next));
    }

    let lexicon = crate::lexicon::Lexicon::builtin();
    let is_symbol = lexicon.greek(word).is_some()
        || lexicon.latin(word).is_some()
        || (word.chars().count() == 1 && word.chars().all(|c| c.is_ascii_alphabetic()));
    let is_number = is_number_word(word);
    if !is_symbol && !is_number {
        return false;
    }

    let Some(next_index) = next_word_index(tokens, index) else {
        return false;
    };
    let Some(next) = word_at(tokens, next_index) else {
        return false;
    };
    // A quantity: «три метра».
    let words: Vec<String> = tokens[next_index..]
        .iter()
        .take_while(|token| token.kind == TokenKind::Word)
        .map(|token| token.text.clone())
        .collect();
    if is_number && lexicon.longest_unit(&words, 0).is_some() {
        return true;
    }
    if matches!(
        next,
        "равно"
            | "равен"
            | "равна"
            | "равняется"
            | "умножить"
            | "умноженное"
            | "умноженная"
            | "умноженный"
            | "умноженные"
            | "плюс"
            | "минус"
            | "разделить"
            | "поделить"
            | "деленное"
            | "делённое"
            | "деленного"
            | "делённого"
            | "индекс"
            | "факториал"
            | "возвести"
            | "больше"
            | "меньше"
    ) {
        return true;
    }
    matches!(next, "в" | "во") && is_power_phrase(tokens, next_index)
}

/// A word that names a scientific *construction*, not merely a thing that
/// happens to be a substance. «гидроксид» and «интеграл» are commands to write
/// notation; «медь» and «вода» are ordinary Russian nouns that also name
/// substances, and they need more evidence before a sentence is rewritten.
fn is_strong_cue(word: &str) -> bool {
    const NOMENCLATURE: [&str; 14] = [
        "гидроксид",
        "оксид",
        "хлорид",
        "сульфат",
        "сульфид",
        "нитрат",
        "нитрит",
        "карбонат",
        "фосфат",
        "перманганат",
        "силикат",
        "кислот",
        "перекись",
        "пероксид",
    ];
    if NOMENCLATURE.iter().any(|cue| word.starts_with(cue)) {
        return true;
    }
    matches!(
        word,
        "интеграл"
            | "сумма"
            | "произведение"
            | "корень"
            | "дробь"
            | "вектор"
            | "модуль"
            | "факториал"
            | "синус"
            | "синуса"
            | "косинус"
            | "косинуса"
            | "тангенс"
            | "котангенс"
            | "логарифм"
            | "логарифма"
            | "экспонента"
            | "экспоненту"
            | "экспоненты"
            | "производная"
            | "производную"
            | "производной"
            | "частная"
            | "частную"
            | "предел"
            | "дельта"
            | "ион"
    )
}

fn is_number_word(word: &str) -> bool {
    word.parse::<u32>().is_ok() || crate::numbers::NumberLex::new().lookup(word).is_some()
}

fn is_power_phrase(tokens: &[Token], preposition: usize) -> bool {
    let Some(next_index) = next_word_index(tokens, preposition) else {
        return false;
    };
    let Some(word) = word_at(tokens, next_index) else {
        return false;
    };
    if matches!(word, "квадрате" | "кубе" | "степени" | "квадрат" | "куб")
    {
        return true;
    }
    if crate::numbers::NumberLex::new().ordinal(word).is_none() {
        return false;
    }
    next_word_index(tokens, next_index)
        .and_then(|degree| word_at(tokens, degree))
        .is_some_and(|word| word == "степени")
}

// -------------------------------------------------------- span candidates

#[derive(Clone, Debug)]
struct Reading {
    domain: Domain,
    node: Node,
    warnings: Vec<Warning>,
    alternatives: Vec<Node>,
    confidence: f32,
    evidence: Vec<String>,
    rank: i32,
}

/// Whether a parsed node actually says something.
///
/// «Сумма заказа изменилась» must not become `∑`: the bare noun parses as a
/// summation with no variable, no bounds and no body, and an operator with
/// nothing under it is not what the speaker dictated.
fn is_contentful(node: &Node) -> bool {
    use crate::ast::Math;
    let Node::Math(math) = node else {
        return true;
    };
    match math {
        Math::Sum {
            var,
            from,
            to,
            body,
        }
        | Math::Product {
            var,
            from,
            to,
            body,
        } => var.is_some() || from.is_some() || to.is_some() || body.is_some(),
        Math::Integral {
            from,
            to,
            integrand,
            wrt,
        } => from.is_some() || to.is_some() || integrand.is_some() || wrt.is_some(),
        _ => true,
    }
}

fn candidate_domains(requested: Domain) -> &'static [Domain] {
    match requested {
        Domain::Auto => &[Domain::Chemistry, Domain::Mathematics, Domain::Physics],
        Domain::Chemistry => &[Domain::Chemistry],
        Domain::Mathematics => &[Domain::Mathematics],
        Domain::Physics => &[Domain::Physics],
        Domain::Plain => &[],
    }
}

/// Reads one stretch of text in every allowed domain and keeps the readings
/// that are actually provable.
///
/// There is no single hard router here: each domain gets to try, and the
/// choice between them is made from checkable evidence. Two equally supported
/// readings are reported as an ambiguity instead of being separated by a
/// score.
fn read_span(
    source: &str,
    requested: Domain,
    anchored_domains: &BTreeSet<&'static str>,
    allow_shortcuts: bool,
    min_confidence: f32,
) -> Vec<Reading> {
    let mut readings = Vec::new();
    // Mathematics and physics differ only in what they assume about letters
    // and units, so the two often parse the same words into different trees.
    // The tie is broken by asking whether the span actually contains physics,
    // which is checkable, rather than by comparing two invented scores.
    let physical = looks_physical(source);
    for &domain in candidate_domains(requested) {
        let parsed = interpret(
            source,
            InterpretOptions {
                domain,
                allow_shortcuts,
            },
        );
        if parsed.confidence < min_confidence {
            continue;
        }
        if matches!(parsed.ast, Node::Text(_)) || !is_contentful(&parsed.ast) {
            continue;
        }
        let structural = parsed
            .warnings
            .iter()
            .all(|warning| !warning.code.starts_with("math."));
        if !structural {
            continue;
        }
        let mut evidence = vec![format!("parsed in full as {}", domain.as_str())];
        let mut rank = 0;
        if requested == domain {
            evidence.push("the caller asked for this domain".into());
            rank += 8;
        }
        if anchored_domains.contains(domain.as_str()) {
            evidence.push("an explicit spoken cue for this domain".into());
            rank += 4;
        }
        if parsed.warnings.is_empty() {
            evidence.push("no validator had anything to say".into());
            rank += 2;
        }
        if parsed.alternatives.is_empty() {
            evidence.push("the parser offered no competing reading".into());
            rank += 1;
        }
        match (domain, physical) {
            (Domain::Physics, true) => {
                evidence.push("the span carries a unit or a vector".into());
                rank += 3;
            }
            (Domain::Mathematics, false) => {
                evidence.push("the span carries no unit or vector".into());
                rank += 3;
            }
            _ => {}
        }
        readings.push(Reading {
            domain,
            node: parsed.ast,
            warnings: parsed.warnings,
            alternatives: parsed.alternatives,
            confidence: parsed.confidence,
            evidence,
            rank,
        });
    }
    readings
}

/// Whether the words carry physics rather than bare mathematics: a unit, a
/// vector, or a dictated increment.
fn looks_physical(source: &str) -> bool {
    let words = crate::normalize::words(source);
    let lexicon = crate::lexicon::Lexicon::builtin();
    words
        .iter()
        .any(|word| word == "вектор" || word == "дельта")
        || (0..words.len()).any(|index| lexicon.longest_unit(&words, index).is_some())
}

/// Picks between readings, or reports that it cannot.
///
/// `Ok(None)` means nothing parsed. `Err(())` means two readings were equally
/// supported and disagreed: the caller must keep the words.
#[allow(clippy::result_unit_err)]
fn choose(readings: Vec<Reading>) -> std::result::Result<Option<Reading>, Vec<Reading>> {
    let Some(best_rank) = readings.iter().map(|reading| reading.rank).max() else {
        return Ok(None);
    };
    let top: Vec<Reading> = readings
        .into_iter()
        .filter(|reading| reading.rank == best_rank)
        .collect();
    // Several domains reaching the *same* structure is agreement, not
    // ambiguity — a formula spelled out letter by letter reads the same in
    // chemistry and in physics.
    let distinct: BTreeSet<String> = top
        .iter()
        .filter_map(|reading| serde_json::to_string(&reading.node).ok())
        .collect();
    if distinct.len() > 1 {
        return Err(top);
    }
    Ok(top.into_iter().next())
}

// --------------------------------------------------------------- repairing

#[derive(Debug)]
enum Repair {
    Applied(Vec<String>, Box<Reading>),
    /// Two different repairs were equally minimal. Guessing here would
    /// silently change a value the speaker did not ask to change.
    Ambiguous,
    None,
}

/// Splices a restated fragment into an already-parsed span.
///
/// The rule is *minimal repair*: replace the shortest stretch of the original
/// that makes the whole thing parse again, preferring a stretch that runs to
/// the end (a person normally restates the tail). Two different results of the
/// same minimal size are an ambiguity, not a coin toss.
fn repair_span(
    original: &[String],
    repair: &[String],
    requested: Domain,
    anchored: &BTreeSet<&'static str>,
    allow_shortcuts: bool,
    budget: &mut usize,
) -> Repair {
    if original.is_empty() || repair.is_empty() {
        return Repair::None;
    }
    // Every splice that parses, with the two facts used to rank it: how much
    // the utterance changed length, and whether the replaced stretch ran to
    // the end.
    let mut hits: Vec<(usize, bool, Vec<String>, Reading)> = Vec::new();
    for replaced in 1..=original.len() {
        for start in 0..=(original.len() - replaced) {
            let end = start + replaced;
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let mut merged: Vec<String> = original[..start].to_vec();
            merged.extend_from_slice(repair);
            merged.extend_from_slice(&original[end..]);
            let text = merged.join(" ");
            if let Ok(Some(reading)) = choose(read_span(
                &text,
                requested,
                anchored,
                allow_shortcuts,
                INLINE_MIN_CONFIDENCE,
            )) {
                let drift = merged.len().abs_diff(original.len());
                hits.push((drift, end == original.len(), merged, reading));
            }
        }
    }
    let Some(best_drift) = hits.iter().map(|(drift, ..)| *drift).min() else {
        return Repair::None;
    };
    hits.retain(|(drift, ..)| *drift == best_drift);
    // A restatement replaces roughly as much as it says, so the splice that
    // leaves the phrase the same length is the one the speaker meant. Among
    // those, one that runs to the end of the phrase is preferred: people
    // normally restate a tail.
    if hits.iter().any(|(_, suffix, ..)| *suffix) {
        hits.retain(|(_, suffix, ..)| *suffix);
    }
    let distinct: BTreeSet<String> = hits
        .iter()
        .filter_map(|(.., reading)| serde_json::to_string(&reading.node).ok())
        .collect();
    if distinct.len() > 1 {
        return Repair::Ambiguous;
    }
    let (.., words, reading) = hits.remove(0);
    Repair::Applied(words, Box::new(reading))
}

/// «не два, а три» — replace one stretch of the span with another.
fn substitute_span(
    original: &[String],
    from: &[String],
    to: &[String],
    requested: Domain,
    anchored: &BTreeSet<&'static str>,
    allow_shortcuts: bool,
    budget: &mut usize,
) -> Repair {
    if from.is_empty() || to.is_empty() || from.len() > original.len() {
        return Repair::None;
    }
    let occurrences: Vec<usize> = (0..=(original.len() - from.len()))
        .filter(|start| original[*start..*start + from.len()] == *from)
        .collect();
    match occurrences.len() {
        // The rejected value was never in the phrase, so the correction adds
        // what was missing. Only an insertion is licensed here: the speaker
        // named exactly one thing as wrong, and nothing they did say may be
        // deleted to make the sentence parse.
        0 => {
            if *budget == 0 {
                return Repair::None;
            }
            *budget -= 1;
            let mut merged = original.to_vec();
            merged.extend_from_slice(to);
            match choose(read_span(
                &merged.join(" "),
                requested,
                anchored,
                allow_shortcuts,
                INLINE_MIN_CONFIDENCE,
            )) {
                Ok(Some(reading)) => Repair::Applied(merged, Box::new(reading)),
                Ok(None) => Repair::None,
                Err(_) => Repair::Ambiguous,
            }
        }
        1 => {
            if *budget == 0 {
                return Repair::None;
            }
            *budget -= 1;
            let start = occurrences[0];
            let mut merged: Vec<String> = original[..start].to_vec();
            merged.extend_from_slice(to);
            merged.extend_from_slice(&original[start + from.len()..]);
            let text = merged.join(" ");
            match choose(read_span(
                &text,
                requested,
                anchored,
                allow_shortcuts,
                INLINE_MIN_CONFIDENCE,
            )) {
                Ok(Some(reading)) => Repair::Applied(merged, Box::new(reading)),
                Ok(None) => Repair::None,
                Err(_) => Repair::Ambiguous,
            }
        }
        _ => Repair::Ambiguous,
    }
}

// ------------------------------------------------------------------- entry

struct Utterance<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    /// Token index of each word, so byte ranges survive the whole search.
    word_token: Vec<usize>,
    words: Vec<String>,
}

impl<'a> Utterance<'a> {
    fn new(text: &'a str) -> Self {
        let tokens = tokenize(text);
        let mut word_token = Vec::new();
        let mut words = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            if token.kind == TokenKind::Word {
                word_token.push(index);
                words.push(token.text.clone());
            }
        }
        Utterance {
            text,
            tokens,
            word_token,
            words,
        }
    }

    fn byte_range(&self, from_word: usize, to_word: usize) -> (usize, usize) {
        let start = self.tokens[self.word_token[from_word]].start;
        let end = self.tokens[self.word_token[to_word - 1]].end;
        (start, end)
    }

    /// True when a sentence-ending mark sits between these two words.
    fn crosses_boundary(&self, from_word: usize, to_word: usize) -> bool {
        let first = self.word_token[from_word];
        let last = self.word_token[to_word - 1];
        self.tokens[first..=last]
            .iter()
            .any(|token| token.kind == TokenKind::Boundary)
    }

    fn refs(&self) -> Vec<&str> {
        self.words.iter().map(String::as_str).collect()
    }
}

/// A span must not end on a word that is plainly waiting for more: a
/// preposition or an operator at the edge means the search cut the sentence
/// in the wrong place.
fn ends_dangling(word: &str) -> bool {
    matches!(
        word,
        "в" | "во"
            | "на"
            | "с"
            | "к"
            | "по"
            | "из"
            | "до"
            | "от"
            | "и"
            | "а"
            | "плюс"
            | "минус"
            | "равно"
            | "равен"
            | "равна"
            | "равняется"
            | "умножить"
            | "разделить"
            | "поделить"
            | "не"
    )
}

/// Interprets a whole spoken utterance.
///
/// Ordinary speech comes back untouched; a proven scientific construct comes
/// back as structure. Nothing is deleted on a guess.
pub fn interpret_utterance(text: &str, options: UtteranceOptions) -> UtteranceResult {
    let utterance = Utterance::new(text);
    let mut result = UtteranceResult {
        document: Node::Text(text.to_string()),
        raw_transcript: text.to_string(),
        mode: options.mode,
        decision: Decision::Raw,
        segments: Vec::new(),
        spans: Vec::new(),
        rejected: Vec::new(),
        warnings: Vec::new(),
        confidence: 0.0,
    };
    if utterance.words.is_empty() || utterance.words.len() > MAX_UTTERANCE_WORDS {
        if utterance.words.len() > MAX_UTTERANCE_WORDS {
            result.warnings.push(Warning {
                code: "dictation.too_long".into(),
                message: format!(
                    "an utterance of {} words is past the {MAX_UTTERANCE_WORDS}-word limit and is kept verbatim",
                    utterance.words.len()
                ),
            });
        }
        return result;
    }

    let refs = utterance.refs();
    let (shell_words, saw_framing) = shell_len(&refs);
    let mut budget = MAX_PARSE_ATTEMPTS;
    let mut anchored: BTreeSet<&'static str> = BTreeSet::new();
    for domain in candidate_domains(options.domain) {
        if (0..utterance.words.len())
            .any(|index| is_cue(&utterance.tokens, utterance.word_token[index], *domain))
        {
            anchored.insert(domain.as_str());
        }
    }

    let mut segments: Vec<Segment> = Vec::new();
    let mut spans: Vec<ScienceSpan> = Vec::new();
    let mut strengths: Vec<bool> = Vec::new();
    let mut drops: Vec<(usize, usize)> = Vec::new();
    let mut ambiguous = false;

    if shell_words > 0 {
        let (start, end) = utterance.byte_range(0, shell_words);
        segments.push(Segment {
            kind: if saw_framing {
                SegmentKind::Framing
            } else {
                SegmentKind::Filler
            },
            start,
            end,
            text: text[start..end].to_string(),
        });
    }

    // The whole utterance, minus a recognised shell, is tried as one construct
    // first. A person who dictates a formula and nothing else is the common
    // case, and reading it in one piece keeps a construct whose opening word
    // is not itself a cue — «открыть скобку …», «корень из …», «пи» — from
    // being cut apart.
    if shell_words < utterance.words.len() {
        let (start, end) = utterance.byte_range(shell_words, utterance.words.len());
        let source = &text[start..end];
        budget = budget.saturating_sub(1);
        if let Ok(Some(reading)) = choose(read_span(
            source,
            options.domain,
            &anchored,
            options.allow_shortcuts,
            WHOLE_UTTERANCE_MIN_CONFIDENCE,
        )) {
            if options.mode == UtteranceMode::ScientificOnly && shell_words > 0 {
                let (shell_start, shell_end) = utterance.byte_range(0, shell_words);
                drops.push((shell_start, shell_end));
            }
            segments.push(Segment {
                kind: SegmentKind::ScientificSpan,
                start,
                end,
                text: source.to_string(),
            });
            let span = ScienceSpan {
                start,
                end,
                source_text: source.to_string(),
                normalized: utterance.words[shell_words..].join(" "),
                domain: reading.domain,
                node: reading.node,
                warnings: reading.warnings,
                alternatives: reading.alternatives,
                decision: Decision::Accepted,
                evidence: reading.evidence,
                confidence: reading.confidence,
                corrections: Vec::new(),
            };
            result.warnings = span.warnings.clone();
            result.document = assemble(text, std::slice::from_ref(&span), &drops);
            result.confidence = if span.warnings.is_empty() { 0.95 } else { 0.7 };
            result.decision = Decision::Accepted;
            result.segments = segments;
            result.spans = vec![span];
            return result;
        }
    }

    let mut word = 0usize;
    while word < utterance.words.len() {
        // The recognised shell is only a shell when it introduces something.
        if word < shell_words {
            word += 1;
            continue;
        }
        let anchored_here = candidate_domains(options.domain)
            .iter()
            .any(|domain| is_cue(&utterance.tokens, utterance.word_token[word], *domain))
            || (saw_framing && word == shell_words);
        if !anchored_here {
            let (start, end) = utterance.byte_range(word, word + 1);
            segments.push(Segment {
                kind: SegmentKind::PlainText,
                start,
                end,
                text: text[start..end].to_string(),
            });
            word += 1;
            continue;
        }

        let Some(mut found) = grow_span(&utterance, word, &options, &anchored, &mut budget) else {
            let (start, end) = utterance.byte_range(word, word + 1);
            segments.push(Segment {
                kind: SegmentKind::PlainText,
                start,
                end,
                text: text[start..end].to_string(),
            });
            word += 1;
            continue;
        };
        if found.ambiguous {
            ambiguous = true;
            let (start, end) = utterance.byte_range(word, found.end_word);
            result.rejected.push(RejectedSpan {
                start,
                end,
                source_text: text[start..end].to_string(),
                reason: "two domains produced different structures with equal support".into(),
            });
            let (start, end) = utterance.byte_range(word, word + 1);
            segments.push(Segment {
                kind: SegmentKind::PlainText,
                start,
                end,
                text: text[start..end].to_string(),
            });
            word += 1;
            continue;
        }

        let outcome = apply_corrections(&utterance, &mut found, &options, &anchored, &mut budget);
        let corrections = outcome.corrections;
        drops.extend(outcome.drops);
        segments.extend(outcome.segments);
        result.rejected.extend(outcome.rejected);
        ambiguous |= outcome.ambiguous;

        // How much evidence this span has. A single ordinary noun in the
        // middle of a sentence is weak: «Сегодня вода холодная» must stay
        // prose. Anything longer than a word, a nomenclature or construction
        // keyword, a dictation shell, or a one-word utterance is strong.
        let strong = found.end_word - word > 1
            || is_strong_cue(&utterance.words[word])
            || (saw_framing && word == shell_words)
            || (word <= shell_words && found.end_word == utterance.words.len());
        strengths.push(strong);

        let (start, end) = utterance.byte_range(word, found.end_word);
        let reading = found.reading;
        segments.push(Segment {
            kind: SegmentKind::ScientificSpan,
            start,
            end,
            text: text[start..end].to_string(),
        });
        spans.push(ScienceSpan {
            start,
            end,
            source_text: text[start..end].to_string(),
            normalized: found.words.join(" "),
            domain: reading.domain,
            node: reading.node,
            warnings: reading.warnings,
            alternatives: reading.alternatives,
            decision: Decision::Accepted,
            evidence: reading.evidence,
            confidence: reading.confidence,
            corrections,
        });
        word = found.end_word;
    }

    // A weak span stands only in company: «Примеры: … , ацетон и глицерин» is
    // a list of chemistry and the bare names belong to it, while a lone
    // ordinary noun in an ordinary sentence does not.
    if !strengths.iter().any(|strong| *strong) {
        for (span, strong) in spans.iter().zip(strengths.iter()) {
            if !strong {
                result.rejected.push(RejectedSpan {
                    start: span.start,
                    end: span.end,
                    source_text: span.source_text.clone(),
                    reason: "a single ordinary word with no other science in the utterance".into(),
                });
            }
        }
        spans.clear();
        for segment in &mut segments {
            if segment.kind == SegmentKind::ScientificSpan {
                segment.kind = SegmentKind::PlainText;
            }
        }
    }

    if spans.is_empty() {
        result.segments = segments;
        result.decision = if ambiguous {
            Decision::Ambiguous
        } else {
            Decision::Raw
        };
        return result;
    }

    // In ScientificOnly the proven shell goes away; in MixedText every word
    // the speaker said is still there.
    if options.mode == UtteranceMode::ScientificOnly {
        if shell_words > 0 {
            let (start, end) = utterance.byte_range(0, shell_words);
            drops.push((start, end));
        }
        for segment in &segments {
            if matches!(segment.kind, SegmentKind::Framing | SegmentKind::Filler) {
                drops.push((segment.start, segment.end));
            }
        }
    }

    let warnings: Vec<Warning> = spans
        .iter()
        .flat_map(|span| span.warnings.iter().cloned())
        .collect();
    result.document = assemble(text, &spans, &drops);
    result.confidence = if warnings.is_empty() { 0.95 } else { 0.7 };
    result.decision = if ambiguous {
        Decision::Ambiguous
    } else {
        Decision::Accepted
    };
    result.segments = segments;
    result.spans = spans;
    result.warnings = warnings;
    result
}

#[derive(Debug)]
struct Found {
    end_word: usize,
    words: Vec<String>,
    reading: Reading,
    ambiguous: bool,
}

/// Grows the longest stretch starting at `word` that parses as one construct.
///
/// Longest wins because a whole reaction is a better answer than its first
/// substance, but a stretch may not end on a dangling preposition and may not
/// cross a sentence boundary.
fn grow_span(
    utterance: &Utterance<'_>,
    word: usize,
    options: &UtteranceOptions,
    anchored: &BTreeSet<&'static str>,
    budget: &mut usize,
) -> Option<Found> {
    let limit = (word + MAX_SPAN_WORDS).min(utterance.words.len());
    let mut best: Option<Found> = None;
    for end in (word + 1)..=limit {
        if utterance.crosses_boundary(word, end) {
            break;
        }
        if ends_dangling(&utterance.words[end - 1]) {
            continue;
        }
        // «дельта 3» is the observed Whisper rendering of «ΔG 3», not a
        // formula: a span must not stop on «дельта» and leave the number it
        // belongs to outside.
        if utterance.words[end - 1] == "дельта"
            && utterance
                .words
                .get(end)
                .is_some_and(|next| is_number_word(next))
        {
            continue;
        }
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let (start_byte, end_byte) = utterance.byte_range(word, end);
        let source = &utterance.text[start_byte..end_byte];
        match choose(read_span(
            source,
            options.domain,
            anchored,
            options.allow_shortcuts,
            INLINE_MIN_CONFIDENCE,
        )) {
            Ok(Some(reading)) => {
                best = Some(Found {
                    end_word: end,
                    words: utterance.words[word..end].to_vec(),
                    reading,
                    ambiguous: false,
                })
            }
            Err(_) if best.is_none() => {
                return Some(Found {
                    end_word: end,
                    words: utterance.words[word..end].to_vec(),
                    reading: Reading {
                        domain: Domain::Auto,
                        node: Node::Text(source.to_string()),
                        warnings: Vec::new(),
                        alternatives: Vec::new(),
                        confidence: 0.0,
                        evidence: Vec::new(),
                        rank: 0,
                    },
                    ambiguous: true,
                })
            }
            _ => {}
        }
    }
    best
}

/// Everything one round of corrections produced.
#[derive(Default)]
struct CorrectionOutcome {
    corrections: Vec<AppliedCorrection>,
    drops: Vec<(usize, usize)>,
    segments: Vec<Segment>,
    rejected: Vec<RejectedSpan>,
    ambiguous: bool,
}

/// Applies every explicit self-correction that follows an accepted span.
///
/// A correction only ever rewrites the active scientific span — never the
/// prose before it — and only when the speaker said one of the markers out
/// loud. Without a marker nothing is silently replaced.
fn apply_corrections(
    utterance: &Utterance<'_>,
    found: &mut Found,
    options: &UtteranceOptions,
    anchored: &BTreeSet<&'static str>,
    budget: &mut usize,
) -> CorrectionOutcome {
    let speech = dictation();
    let refs = utterance.refs();
    let mut outcome = CorrectionOutcome::default();
    for _ in 0..MAX_CORRECTIONS {
        let cursor = found.end_word;
        if cursor >= utterance.words.len() {
            break;
        }
        if utterance.crosses_boundary(cursor.saturating_sub(1), cursor + 1) {
            break;
        }
        let repair = if let Some(marker_len) = speech.restate_at(&refs, cursor) {
            restate(
                utterance, found, cursor, marker_len, options, anchored, budget,
            )
        } else if speech.is_substitute_open(&utterance.words[cursor]) {
            substitute(utterance, found, cursor, options, anchored, budget, speech)
        } else {
            None
        };
        let Some(step) = repair else { break };
        let (marker_start, _) = utterance.byte_range(cursor, cursor + 1);
        let (_, consumed_end) = utterance.byte_range(cursor, step.consumed_end);
        match step.repair {
            Repair::Applied(words, reading) => {
                outcome.segments.push(Segment {
                    kind: SegmentKind::CorrectionMarker,
                    start: marker_start,
                    end: consumed_end,
                    text: utterance.text[marker_start..consumed_end].to_string(),
                });
                outcome.drops.push((marker_start, consumed_end));
                outcome.corrections.push(AppliedCorrection {
                    marker: step.marker,
                    repair_text: step.repair_text,
                    result_text: words.join(" "),
                });
                found.words = words;
                found.reading = *reading;
                found.end_word = step.consumed_end;
            }
            Repair::Ambiguous => {
                outcome.ambiguous = true;
                outcome.rejected.push(RejectedSpan {
                    start: marker_start,
                    end: consumed_end,
                    source_text: utterance.text[marker_start..consumed_end].to_string(),
                    reason: "the correction could apply to more than one part of the phrase".into(),
                });
                break;
            }
            Repair::None => break,
        }
    }
    outcome
}

struct CorrectionStep {
    marker: String,
    repair_text: String,
    consumed_end: usize,
    repair: Repair,
}

/// «…, нет, железа три» — the tail restates part of the span.
fn restate(
    utterance: &Utterance<'_>,
    found: &Found,
    cursor: usize,
    marker_len: usize,
    options: &UtteranceOptions,
    anchored: &BTreeSet<&'static str>,
    budget: &mut usize,
) -> Option<CorrectionStep> {
    let repair_start = cursor + marker_len;
    if repair_start >= utterance.words.len() {
        return None;
    }
    let mut limit = repair_start;
    while limit < utterance.words.len()
        && !utterance.crosses_boundary(repair_start, limit + 1)
        && limit - repair_start < MAX_SPAN_WORDS
    {
        limit += 1;
    }
    // The longest restatement that actually repairs the span wins: a speaker
    // normally restates a whole tail, not one word of it.
    for end in (repair_start + 1..=limit).rev() {
        let repair_words = utterance.words[repair_start..end].to_vec();
        let outcome = repair_span(
            &found.words,
            &repair_words,
            options.domain,
            anchored,
            options.allow_shortcuts,
            budget,
        );
        if matches!(outcome, Repair::None) {
            continue;
        }
        return Some(CorrectionStep {
            marker: utterance.words[cursor..repair_start].join(" "),
            repair_text: repair_words.join(" "),
            consumed_end: end,
            repair: outcome,
        });
    }
    None
}

/// «не два, а три» — an explicit token-for-token substitution.
fn substitute(
    utterance: &Utterance<'_>,
    found: &Found,
    cursor: usize,
    options: &UtteranceOptions,
    anchored: &BTreeSet<&'static str>,
    budget: &mut usize,
    speech: &DictationSpeech,
) -> Option<CorrectionStep> {
    let pivot = (cursor + 2..utterance.words.len())
        .find(|index| speech.is_substitute_pivot(&utterance.words[*index]))?;
    if pivot + 1 >= utterance.words.len() {
        return None;
    }
    let from = utterance.words[cursor + 1..pivot].to_vec();
    let mut limit = pivot + 1;
    while limit < utterance.words.len()
        && !utterance.crosses_boundary(pivot + 1, limit + 1)
        && limit - pivot <= MAX_SPAN_WORDS
    {
        limit += 1;
    }
    for end in (pivot + 2..=limit).rev() {
        let to = utterance.words[pivot + 1..end].to_vec();
        let outcome = substitute_span(
            &found.words,
            &from,
            &to,
            options.domain,
            anchored,
            options.allow_shortcuts,
            budget,
        );
        if matches!(outcome, Repair::None) {
            continue;
        }
        return Some(CorrectionStep {
            marker: format!("не {} а", from.join(" ")),
            repair_text: to.join(" "),
            consumed_end: end,
            repair: outcome,
        });
    }
    None
}

/// Builds the one document every renderer will work from.
fn assemble(text: &str, spans: &[ScienceSpan], drops: &[(usize, usize)]) -> Node {
    let mut children: Vec<Node> = Vec::new();
    let mut cursor = 0usize;
    for span in spans {
        push_prose(&mut children, text, cursor, span.start, drops);
        children.push(span.node.clone());
        cursor = span.end;
    }
    push_prose(&mut children, text, cursor, text.len(), drops);
    // A single scientific node with nothing around it stays a bare node, so
    // Word still receives one native equation rather than a wrapper.
    if children.len() == 1 && !matches!(children[0], Node::Text(_)) {
        return children.remove(0);
    }
    Node::Document(children)
}

fn push_prose(
    children: &mut Vec<Node>,
    text: &str,
    from: usize,
    to: usize,
    drops: &[(usize, usize)],
) {
    if from >= to {
        return;
    }
    let mut kept = String::new();
    let mut cursor = from;
    let mut ordered: Vec<(usize, usize)> = drops
        .iter()
        .copied()
        .filter(|(start, end)| *start >= from && *end <= to)
        .collect();
    ordered.sort_unstable();
    let removed_something = !ordered.is_empty();
    for (start, end) in ordered {
        if start < cursor {
            continue;
        }
        kept.push_str(&text[cursor..start]);
        cursor = end;
    }
    kept.push_str(&text[cursor..to]);
    if kept.trim().is_empty() {
        return;
    }
    // Dropping a shell leaves stray spaces and commas behind. Prose that lost
    // nothing is passed through byte for byte: MixedText must never quietly
    // reshape punctuation the speaker dictated.
    let kept = if removed_something {
        trim_edges(&kept)
    } else {
        kept
    };
    if kept.is_empty() {
        return;
    }
    children.push(Node::Text(kept));
}

fn trim_edges(text: &str) -> String {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == ',' || c == '-');
    if trimmed.is_empty() {
        return String::new();
    }
    let leading = text.starts_with(char::is_whitespace) && !text.trim_start().starts_with(',');
    let trailing = text.ends_with(char::is_whitespace);
    let mut out = String::new();
    if leading {
        out.push(' ');
    }
    out.push_str(trimmed);
    if trailing {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, mode: UtteranceMode) -> UtteranceResult {
        interpret_utterance(
            text,
            UtteranceOptions {
                domain: Domain::Auto,
                mode,
                allow_shortcuts: true,
            },
        )
    }

    fn unicode(text: &str, mode: UtteranceMode) -> String {
        crate::render(&read(text, mode).document, crate::ast::Renderer::Unicode)
    }

    #[test]
    fn segments_carry_the_byte_ranges_of_the_original_text() {
        let text = "Сегодня рассмотрим перманганат калия, а затем продолжим опыт.";
        let result = read(text, UtteranceMode::MixedText);
        let span = result.spans.first().expect("one span");
        assert_eq!(&text[span.start..span.end], "перманганат калия");
        assert_eq!(span.source_text, "перманганат калия");
        // Every segment must slice the original text exactly, so a span can
        // always be put back where it came from.
        for segment in &result.segments {
            assert_eq!(&text[segment.start..segment.end], segment.text);
        }
        assert!(result
            .segments
            .iter()
            .any(|segment| segment.kind == SegmentKind::ScientificSpan));
        assert!(result
            .segments
            .iter()
            .any(|segment| segment.kind == SegmentKind::PlainText));
    }

    #[test]
    fn framing_is_matched_longest_first() {
        // «давайте запишем» must win over any shorter entry that happens to
        // start at the same word.
        let words = ["давайте", "запишем", "икс", "в", "кубе"];
        let refs: Vec<&str> = words.to_vec();
        assert_eq!(dictation().framing_at(&refs, 0), Some(2));
        assert_eq!(
            unicode("давайте запишем икс в кубе", UtteranceMode::ScientificOnly),
            "x³"
        );
    }

    #[test]
    fn a_framing_shell_is_dropped_only_in_scientific_mode() {
        assert_eq!(
            unicode("запиши перманганат калия", UtteranceMode::ScientificOnly),
            "KMnO₄"
        );
        assert_eq!(
            unicode("запиши перманганат калия", UtteranceMode::MixedText),
            "запиши KMnO₄"
        );
    }

    #[test]
    fn repeated_framing_and_fillers_are_all_part_of_one_shell() {
        assert_eq!(
            unicode(
                "ну так вот, запиши, пожалуйста, напиши аммиак",
                UtteranceMode::ScientificOnly
            ),
            "NH₃"
        );
    }

    #[test]
    fn a_filler_inside_ordinary_prose_survives() {
        // «ну» is only a filler inside a dictation shell; in the middle of a
        // sentence it is a word the speaker said.
        let text = "Ну и что теперь делать.";
        assert_eq!(unicode(text, UtteranceMode::ScientificOnly), text);
    }

    #[test]
    fn every_correction_form_rewrites_only_the_active_span() {
        for (spoken, expected) in [
            ("гидроксид железа два, нет, железа три", "Fe(OH)₃"),
            ("гидроксид железа два, точнее железа три", "Fe(OH)₃"),
            ("гидроксид железа два, вернее железа три", "Fe(OH)₃"),
            ("гидроксид железа два, поправка железа три", "Fe(OH)₃"),
            ("гидроксид железа два, я имел в виду железа три", "Fe(OH)₃"),
            ("гидроксид железа не два, а три", "Fe(OH)₃"),
        ] {
            assert_eq!(
                unicode(spoken, UtteranceMode::ScientificOnly),
                expected,
                "{spoken}"
            );
        }
    }

    #[test]
    fn a_correction_records_what_it_replaced() {
        let result = read(
            "гидроксид железа два, нет, железа три",
            UtteranceMode::ScientificOnly,
        );
        let span = result.spans.first().expect("one span");
        assert_eq!(span.corrections.len(), 1);
        assert_eq!(span.corrections[0].marker, "нет");
        assert_eq!(span.corrections[0].repair_text, "железа три");
        assert_eq!(span.corrections[0].result_text, "гидроксид железа три");
    }

    #[test]
    fn a_correction_never_reaches_back_into_the_prose_before_the_span() {
        let text = "Мы обсудили гидроксид железа два, нет, железа три сегодня.";
        let result = read(text, UtteranceMode::MixedText);
        let rendered = crate::render(&result.document, crate::ast::Renderer::Unicode);
        assert!(rendered.starts_with("Мы обсудили "), "{rendered}");
        assert!(rendered.contains("Fe(OH)₃"), "{rendered}");
    }

    #[test]
    fn a_correction_that_could_land_in_two_places_is_refused() {
        // «два» appears twice, so «не два, а три» does not say which one.
        let text = "два аш два о не два, а три";
        let result = read(text, UtteranceMode::ScientificOnly);
        assert!(
            result
                .rejected
                .iter()
                .any(|span| span.reason.contains("more than one part")),
            "{:?}",
            result.rejected
        );
    }

    #[test]
    fn the_number_of_corrections_is_bounded() {
        // Far more restatements than a person makes; the parser must stop
        // rather than keep re-reading the same span.
        let mut text = String::from("гидроксид железа два");
        for _ in 0..(MAX_CORRECTIONS + 6) {
            text.push_str(", нет, железа три");
        }
        let result = read(&text, UtteranceMode::ScientificOnly);
        let applied = result
            .spans
            .first()
            .map(|span| span.corrections.len())
            .unwrap_or(0);
        assert!(applied <= MAX_CORRECTIONS, "{applied} corrections applied");
    }

    #[test]
    fn a_domain_is_chosen_from_evidence_not_from_a_score() {
        // Physics and mathematics both parse this, and they disagree about
        // letter case. The span carries no unit, so mathematics wins and the
        // reason is recorded.
        let result = read("производная эф по икс", UtteranceMode::ScientificOnly);
        let span = result.spans.first().expect("one span");
        assert_eq!(span.domain, Domain::Mathematics);
        assert!(
            span.evidence
                .iter()
                .any(|reason| reason.contains("no unit or vector")),
            "{:?}",
            span.evidence
        );
        // With a unit present the same machinery picks physics.
        let result = read(
            "три метра плюс четыре секунды",
            UtteranceMode::ScientificOnly,
        );
        let span = result.spans.first().expect("one span");
        assert_eq!(span.domain, Domain::Physics);
    }

    #[test]
    fn an_empty_construct_is_not_a_formula() {
        for text in ["Сумма заказа изменилась.", "интеграл", "сумма"]
        {
            assert_eq!(unicode(text, UtteranceMode::MixedText), text, "{text}");
        }
    }

    #[test]
    fn a_long_utterance_is_kept_verbatim_instead_of_being_searched() {
        let text = "вода ".repeat(MAX_UTTERANCE_WORDS + 10);
        let result = read(&text, UtteranceMode::MixedText);
        assert!(result.is_raw());
        assert_eq!(result.decision, Decision::Raw);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "dictation.too_long"));
    }

    #[test]
    fn hostile_and_broken_input_does_not_panic() {
        for text in [
            "",
            "   ",
            ",,,,,,",
            "....",
            "\u{0}\u{1}\u{2}",
            "ё",
            "запиши",
            "нет",
            "не а",
            "не два а",
            "гидроксид железа два, нет,",
            "🙂 гидроксид железа два 🙂",
            "aaaa bbbb cccc dddd eeee",
            "гидроксид ".repeat(80).as_str(),
            "плюс плюс плюс плюс",
            "предел при при при",
        ] {
            for mode in [UtteranceMode::MixedText, UtteranceMode::ScientificOnly] {
                let result = read(text, mode);
                // Whatever happens, the transcript survives.
                assert_eq!(result.raw_transcript, text);
                let _ = crate::render(&result.document, crate::ast::Renderer::Unicode);
                let _ = crate::render(&result.document, crate::ast::Renderer::Latex);
                let _ = crate::render(&result.document, crate::ast::Renderer::Omml);
            }
        }
    }

    #[test]
    fn a_mixed_document_is_not_pure_science_but_a_bare_formula_is() {
        assert!(read("запиши перманганат калия", UtteranceMode::ScientificOnly).is_pure_science());
        assert!(!read(
            "Сегодня рассмотрим перманганат калия, а затем продолжим опыт.",
            UtteranceMode::MixedText
        )
        .is_pure_science());
    }

    #[test]
    fn a_partially_compiled_utterance_is_not_reported_as_a_total_failure() {
        let result = read(
            "Например, калий марганец о четыре.",
            UtteranceMode::MixedText,
        );
        let interpretation = result.to_interpretation(Domain::Auto);
        assert!(interpretation.confidence > 0.0);
        assert!(matches!(interpretation.ast, Node::Document(_)));
        assert_eq!(result.decision, Decision::Accepted);
    }
}

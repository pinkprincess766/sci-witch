//! Research-only entry points. Not part of the stable compiler surface;
//! signatures here may change without notice.

use std::collections::BTreeSet;

use crate::coordination::{Coordination, MaterialClasses, Refusal as CoordRefusal, SphereKind};
use crate::lattice::vocabulary_names;
use crate::lexicon::{ChemConnective, IonRole, Lexicon};
use crate::normalize::{normalize_word, words as split_words};
use crate::numbers::NumberLex;
use crate::organic::{Organic, Refusal as OrganicRefusal};
use crate::parser::chemistry::{
    chemistry_element_at, element_by_word, spoken_words, strip_conditions, FUNCTION_WORD_LETTERS,
};
use crate::parser::math::{tokenize, MathMode, Tok};

/// Classes of the tokens the mathematics parser sees for `text`, by the names
/// `docs/grammar/math.ebnf` uses for its terminals (`Tok::Plus` → `PLUS`, …).
/// Research API: not part of the stable surface, may change without notice.
pub fn math_token_classes(text: &str, physics: bool) -> Result<Vec<String>, String> {
    let words = split_words(text);
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let mode = if physics {
        MathMode::Physics
    } else {
        MathMode::Math
    };
    let toks = tokenize(&words, lex, &nums, mode).map_err(|e| e.to_string())?;
    Ok(toks
        .iter()
        .map(Tok::class_name)
        .map(str::to_string)
        .collect())
}

/// Most edits [`nearest_lexicon_words`] allows. One vowel swap is 1,
/// «гидраксид»→«гидроксид» is 1, «сернея»→«серная» is 1, «кидроксидж»→«гидроксид»
/// (one substitution, one deletion) is 2. A larger limit starts to reach
/// unrelated words: at 3 edits a six-letter word is half rewritten.
pub const MAX_WORD_EDITS: usize = 2;

/// Shortest word [`nearest_lexicon_words`] tries to correct; the same bound as
/// the lattice's `MIN_REPAIR_WORD_CHARS`. A short word has too many neighbours
/// within two edits.
pub const MIN_CORRECTED_WORD_CHARS: usize = 5;

/// The lexicon words nearest to `word` by Levenshtein distance, with that
/// distance `d`, if `1 <= d <=` [`MAX_WORD_EDITS`]. All ties are returned,
/// sorted and without duplicates; no choice is made here. `None` if `word` is
/// itself a vocabulary word, is shorter than [`MIN_CORRECTED_WORD_CHARS`]
/// characters, is not a single word, or nothing is within the limit.
///
/// Vocabulary: the names the lattice's `known_word` accepts (element, anion
/// and substance names), plus each word of a multi-word substance name taken
/// on its own, so that «сернея» can reach «серная» of «серная кислота». This
/// is wider than `known_word`, which compares a single word with a whole name.
///
/// Distance is over `char`s with unit costs for insert, delete and
/// substitute. `word` is lowercased and ё becomes е (`normalize_word`, as for
/// lattice words); the spelling fixes of `normalize::words` are not applied,
/// so «гидраксид» is seen as heard. Cost is bounded by the vocabulary, not by
/// the length of `word`: a vocabulary word whose length differs by more than
/// [`MAX_WORD_EDITS`] is skipped without a table.
///
/// Research API: not part of the stable surface, may change without notice.
pub fn nearest_lexicon_words(word: &str) -> Option<(usize, Vec<String>)> {
    let normalized = normalize_word(word);
    if normalized.split_whitespace().count() != 1 {
        return None;
    }
    let heard: Vec<char> = normalized.chars().collect();
    if heard.len() < MIN_CORRECTED_WORD_CHARS {
        return None;
    }
    let vocabulary: BTreeSet<&str> = vocabulary_names(Lexicon::builtin())
        .flat_map(|name| name.split_whitespace())
        .collect();
    if vocabulary.contains(normalized.as_str()) {
        return None;
    }
    let mut best = MAX_WORD_EDITS + 1;
    let mut nearest: Vec<String> = Vec::new();
    for candidate in vocabulary {
        let candidate_chars: Vec<char> = candidate.chars().collect();
        if candidate_chars.len().abs_diff(heard.len()) > MAX_WORD_EDITS {
            continue;
        }
        let distance = levenshtein(&heard, &candidate_chars);
        if distance < best {
            best = distance;
            nearest.clear();
        }
        if distance == best {
            nearest.push(candidate.to_string());
        }
    }
    // The set iterates in order, so `nearest` is already sorted.
    (best <= MAX_WORD_EDITS).then_some((best, nearest))
}

/// Unit-cost edit distance over characters, two rows of the textbook table.
fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            let delete = previous[j + 1] + 1;
            let insert = current[j] + 1;
            current.push(substitute.min(delete).min(insert));
        }
        previous = current;
    }
    previous[b.len()]
}

/// Terminals of `docs/grammar/chem.ebnf` that [`chem_token_classes`] returns.
/// Every name here must be a terminal defined in that file (a test checks it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChemToken {
    Plus,
    Conjunction,
    JoinReagent,
    ReactionOpen,
    ReactionNoun,
    ArrowJoint,
    Condition,
    StateMarker,
    IonMarker,
    Grouping,
    ChargeMinus,
    Integer,
    SubstanceName,
    ElectronName,
    HydrocarbonWord,
    HydrateWord,
    IonLeadIn,
    CationicComplexWord,
    AnionicComplexWord,
    OxidationMarker,
    Roman,
    ClassWord,
    ClassAdjective,
    AnionClass,
    CationClass,
    ElementWord,
    ElementByWord,
    LetterName,
    FunctionLetter,
}

impl ChemToken {
    /// Every variant, for the tests that check the names against the grammar.
    #[cfg(test)]
    const ALL: [ChemToken; 29] = [
        ChemToken::Plus,
        ChemToken::Conjunction,
        ChemToken::JoinReagent,
        ChemToken::ReactionOpen,
        ChemToken::ReactionNoun,
        ChemToken::ArrowJoint,
        ChemToken::Condition,
        ChemToken::StateMarker,
        ChemToken::IonMarker,
        ChemToken::Grouping,
        ChemToken::ChargeMinus,
        ChemToken::Integer,
        ChemToken::SubstanceName,
        ChemToken::ElectronName,
        ChemToken::HydrocarbonWord,
        ChemToken::HydrateWord,
        ChemToken::IonLeadIn,
        ChemToken::CationicComplexWord,
        ChemToken::AnionicComplexWord,
        ChemToken::OxidationMarker,
        ChemToken::Roman,
        ChemToken::ClassWord,
        ChemToken::ClassAdjective,
        ChemToken::AnionClass,
        ChemToken::CationClass,
        ChemToken::ElementWord,
        ChemToken::ElementByWord,
        ChemToken::LetterName,
        ChemToken::FunctionLetter,
    ];

    fn name(self) -> &'static str {
        match self {
            ChemToken::Plus => "PLUS",
            ChemToken::Conjunction => "CONJUNCTION",
            ChemToken::JoinReagent => "JOIN_REAGENT",
            ChemToken::ReactionOpen => "REACTION_OPEN",
            ChemToken::ReactionNoun => "REACTION_NOUN",
            ChemToken::ArrowJoint => "ARROW_JOINT",
            ChemToken::Condition => "CONDITION",
            ChemToken::StateMarker => "STATE_MARKER",
            ChemToken::IonMarker => "ION_MARKER",
            ChemToken::Grouping => "GROUPING",
            ChemToken::ChargeMinus => "CHARGE_MINUS",
            ChemToken::Integer => "INTEGER",
            ChemToken::SubstanceName => "SUBSTANCE_NAME",
            ChemToken::ElectronName => "ELECTRON_NAME",
            ChemToken::HydrocarbonWord => "HYDROCARBON_WORD",
            ChemToken::HydrateWord => "HYDRATE_WORD",
            ChemToken::IonLeadIn => "ION_LEAD_IN",
            ChemToken::CationicComplexWord => "CATIONIC_COMPLEX_WORD",
            ChemToken::AnionicComplexWord => "ANIONIC_COMPLEX_WORD",
            ChemToken::OxidationMarker => "OXIDATION_MARKER",
            ChemToken::Roman => "ROMAN",
            ChemToken::ClassWord => "CLASS_WORD",
            ChemToken::ClassAdjective => "CLASS_ADJECTIVE",
            ChemToken::AnionClass => "ANION_CLASS",
            ChemToken::CationClass => "CATION_CLASS",
            ChemToken::ElementWord => "ELEMENT_WORD",
            ChemToken::ElementByWord => "ELEMENT_BY_WORD",
            ChemToken::LetterName => "LETTER_NAME",
            ChemToken::FunctionLetter => "FUNCTION_LETTER",
        }
    }
}

/// Classes of the words the chemistry parser reads for `text`, by the
/// terminal names `docs/grammar/chem.ebnf` uses.
///
/// The output is what the grammar is applied to, so it follows the parser's
/// own preparation: punctuation is dropped as in `parse_chemistry`. A reaction
/// shape (a connective other than «плюс» or «и») switches on the reaction
/// reading: conditions are cut out, and connectives become their terminals.
/// Otherwise the text is one species, and conditions stay as `CONDITION`.
///
/// A terminal covers a phrase, not a single word: «углекислый газ» is one
/// `SUBSTANCE_NAME`, «реагирует с» one `JOIN_REAGENT`, and a two-word spelling
/// pair («эн а») is two `LETTER_NAME`/`FUNCTION_LETTER` tokens, as in the
/// grammar's `element_pair`.
///
/// Where one word fits several classes, the class the parser tries first
/// wins, in the order of `parse_species_inner` and `chemistry_element_at`
/// (the order `chem.ebnf` gives in its comments). For example «кислород» is
/// `SUBSTANCE_NAME`, not `ELEMENT_WORD`; «аш» is `ELEMENT_WORD`, because
/// `elements.yaml` lists it as a spelled name; «ион» before a complex-cation
/// word is `ION_LEAD_IN`, and elsewhere `ION_MARKER`; a letter such as «о» is
/// `FUNCTION_LETTER` even where it names oxygen.
///
/// Not applied: the semantic checks at the end of `chem.ebnf` (for example
/// the spelled-context rule for function letters), so a class sequence that
/// the grammar accepts can still be refused by the parser.
///
/// An `Err` names the first word that no class fits.
/// Research API: not part of the stable surface, may change without notice.
pub fn chem_token_classes(text: &str) -> Result<Vec<String>, String> {
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let cleaned = spoken_words(&split_words(text));
    let reaction = has_reaction_shape(lex, &cleaned);
    let words = if reaction {
        strip_conditions(&cleaned, lex).0
    } else {
        cleaned
    };

    let mut out = Vec::new();
    // After an anionic complex word, the counter-ion is the next element
    // word, with only oxidation numbers in between.
    let mut counter_pending = false;
    // Whether the word at `i` starts a species (after a connective, or a
    // coefficient that starts one). The species-core classes are read only
    // there, and only when they cover the rest of the species.
    let mut at_start = true;
    let mut i = 0;
    while i < words.len() {
        let end = chunk_end(lex, &words, reaction, i);
        let pos = Position { at_start, end };
        let units = match counter_ion_at(lex, &nums, &words, i, counter_pending) {
            Some(unit) => vec![unit],
            None => classify(lex, &nums, &words, i, reaction, pos)
                .ok_or_else(|| format!("unknown word: {}", words[i]))?,
        };
        let (first, _) = units[0];
        counter_pending = match first {
            ChemToken::AnionicComplexWord => true,
            ChemToken::Integer | ChemToken::Roman | ChemToken::OxidationMarker => counter_pending,
            _ => false,
        };
        let is_connective = reaction && lex.chemistry_speech.connective_at(&words, i).is_some();
        at_start = is_connective || (at_start && first == ChemToken::Integer);
        for (token, used) in units {
            out.push(token.name().to_string());
            i += used;
        }
    }
    Ok(out)
}

/// Where a word sits: at the start of a species, and where that species
/// ends (the next reaction connective, or the end of the words).
#[derive(Clone, Copy)]
struct Position {
    at_start: bool,
    end: usize,
}

/// The index where the species that contains `from` ends. In species mode
/// the whole word list is one species; in reaction mode a connective ends it.
fn chunk_end(lex: &Lexicon, words: &[String], reaction: bool, from: usize) -> usize {
    if !reaction {
        return words.len();
    }
    (from..words.len())
        .find(|&j| lex.chemistry_speech.connective_at(words, j).is_some())
        .unwrap_or(words.len())
}

/// Whether the words carry a reaction shape: a connective that is neither
/// «плюс» nor «и». Mirrors the test in `parse_reaction`.
fn has_reaction_shape(lex: &Lexicon, words: &[String]) -> bool {
    (0..words.len()).any(|i| {
        matches!(
            lex.chemistry_speech.connective_at(words, i),
            Some((connective, _))
                if !matches!(connective, ChemConnective::Plus | ChemConnective::Conjunction)
        )
    })
}

fn connective_token(connective: ChemConnective) -> ChemToken {
    match connective {
        ChemConnective::Plus => ChemToken::Plus,
        ChemConnective::Conjunction => ChemToken::Conjunction,
        ChemConnective::JoinReagent => ChemToken::JoinReagent,
        ChemConnective::FromMarker | ChemConnective::BetweenMarker => ChemToken::ReactionOpen,
        ChemConnective::ReactionNoun => ChemToken::ReactionNoun,
        ChemConnective::Forward
        | ChemConnective::Equilibrium
        | ChemConnective::ProductMarker
        | ChemConnective::Decompose
        | ChemConnective::ToMarker => ChemToken::ArrowJoint,
    }
}

/// The counter-ion of an anionic complex, when one is pending at `i`.
fn counter_ion_at(
    lex: &Lexicon,
    nums: &NumberLex,
    words: &[String],
    i: usize,
    pending: bool,
) -> Option<(ChemToken, usize)> {
    if !pending || oxidation_value_at(nums, words, i) || element_by_word(lex, &words[i]).is_none() {
        return None;
    }
    Some((ChemToken::ElementByWord, 1))
}

/// Whether a number (digits, a spoken number, a Roman numeral) starts at `i`.
fn oxidation_value_at(nums: &NumberLex, words: &[String], i: usize) -> bool {
    Coordination::builtin().roman(&words[i]).is_some() || nums.consume_int(words, i).is_some()
}

/// The words of an explicit oxidation phrase at `i` (for example «плюс» before
/// a number), if a number follows it. Mirrors `oxidation_at`.
fn oxidation_phrase_len(nums: &NumberLex, words: &[String], i: usize) -> Option<usize> {
    let (_, used) = Coordination::builtin().oxidation_marker(words, i)?;
    (i + used < words.len() && oxidation_value_at(nums, words, i + used)).then_some(used)
}

/// The classes that start at `i`, each with the number of words it covers.
/// `None` when no class fits. Order follows `parse_species_inner`.
fn classify(
    lex: &Lexicon,
    nums: &NumberLex,
    words: &[String],
    i: usize,
    reaction: bool,
    pos: Position,
) -> Option<Vec<(ChemToken, usize)>> {
    let speech = &lex.chemistry_speech;
    let coord = Coordination::builtin();
    if reaction {
        if let Some((connective, used)) = speech.connective_at(words, i) {
            return Some(vec![(connective_token(connective), used)]);
        }
    }
    let word = words[i].as_str();
    if let Some((_, used)) = speech.condition_at(words, i) {
        return Some(vec![(ChemToken::Condition, used)]);
    }
    if coord.hydrate(word).is_some() {
        return Some(vec![(ChemToken::HydrateWord, 1)]);
    }
    if let Some(used) = oxidation_phrase_len(nums, words, i) {
        return Some(vec![(ChemToken::OxidationMarker, used)]);
    }
    if let Some((_, used)) = nums.consume_int(words, i) {
        return Some(vec![(ChemToken::Integer, used)]);
    }
    if coord.roman(word).is_some() {
        return Some(vec![(ChemToken::Roman, 1)]);
    }
    if pos.at_start {
        if let Some(units) = species_core_at(lex, words, i, pos.end) {
            return Some(units);
        }
    }
    if let Some((_, used)) = speech.marker_at(words, i) {
        return Some(vec![(ChemToken::StateMarker, used)]);
    }
    if let Some((_, used)) = speech.grouping_at(words, i) {
        return Some(vec![(ChemToken::Grouping, used)]);
    }
    if speech.is_ion_marker(word) {
        return Some(vec![(ChemToken::IonMarker, 1)]);
    }
    // The sign of a charge, in species mode. In reaction mode «плюс» was
    // already read as a connective above.
    if word == "плюс" {
        return Some(vec![(ChemToken::Plus, 1)]);
    }
    if word == "минус" {
        return Some(vec![(ChemToken::ChargeMinus, 1)]);
    }
    if let Some(ion) = lex.anion(word) {
        let token = match ion.role {
            IonRole::Anion => ChemToken::AnionClass,
            IonRole::Cation => ChemToken::CationClass,
        };
        return Some(vec![(token, 1)]);
    }
    // A two-word spelling («эн а»): the element lookup reads it as one unit.
    if let Some((_, 2)) = chemistry_element_at(lex, words, i, nums, false) {
        return Some(vec![
            (pair_word_token(word), 1),
            (pair_word_token(&words[i + 1]), 1),
        ]);
    }
    // A function letter is FUNCTION_LETTER even where it names an element:
    // the grammar admits it only in a spelled formula (the semantic rule).
    if FUNCTION_WORD_LETTERS.contains(&word) {
        return Some(vec![(ChemToken::FunctionLetter, 1)]);
    }
    if lex.element(word).is_some() {
        return Some(vec![(ChemToken::ElementWord, 1)]);
    }
    if lex.latin(word).is_some() {
        return Some(vec![(ChemToken::LetterName, 1)]);
    }
    None
}

/// The classes of a whole species: a substance name, a complex salt or
/// complex cation, an electron, a hydrocarbon or a material class. The parser
/// tries them only at the start of a species, and each one has to cover the
/// rest of it (`try_full_substance`, `try_electron`, `try_organic` and the
/// others take the whole chunk). Order as in `parse_species_inner`.
fn species_core_at(
    lex: &Lexicon,
    words: &[String],
    i: usize,
    end: usize,
) -> Option<Vec<(ChemToken, usize)>> {
    let coord = Coordination::builtin();
    let word = words[i].as_str();
    if let Some((_, used)) = lex.longest_substance(&words[..end], i) {
        if i + used == end {
            return Some(vec![(ChemToken::SubstanceName, used)]);
        }
    }
    if coord.is_ion_lead_in(word) && i + 1 < end && cationic_claims(lex, &words[i + 1]) {
        return Some(vec![
            (ChemToken::IonLeadIn, 1),
            (ChemToken::CationicComplexWord, 1),
        ]);
    }
    // The counter-ion and any oxidation after it are checked by the grammar.
    if anionic_claims(word) {
        return Some(vec![(ChemToken::AnionicComplexWord, 1)]);
    }
    if i + 1 == end && coord.is_electron(word) {
        return Some(vec![(ChemToken::ElectronName, 1)]);
    }
    if i + 1 == end && hydrocarbon_claims(word) {
        return Some(vec![(ChemToken::HydrocarbonWord, 1)]);
    }
    if i + 2 == end {
        return material_class_at(lex, words, i);
    }
    None
}

fn pair_word_token(word: &str) -> ChemToken {
    if FUNCTION_WORD_LETTERS.contains(&word) {
        ChemToken::FunctionLetter
    } else {
        ChemToken::LetterName
    }
}

/// Whether `word` is a complex-cation name with a known centre. Mirrors
/// `try_coordination_cationic`.
fn cationic_claims(lex: &Lexicon, word: &str) -> bool {
    Coordination::builtin()
        .split_cationic(word)
        .is_some_and(|(_, _, centre)| element_by_word(lex, centre).is_some())
}

/// Whether `word` is an anionic complex name, or one the parser refuses as
/// such. Mirrors `try_coordination_anionic`.
fn anionic_claims(word: &str) -> bool {
    match Coordination::builtin().split(word) {
        Ok(decomposition) => matches!(decomposition.kind, SphereKind::Anionic),
        Err(CoordRefusal::NotCoordination) => false,
        Err(_) => true,
    }
}

/// Whether `word` is a hydrocarbon name, or one the parser refuses as such.
/// Mirrors `try_organic`.
fn hydrocarbon_claims(word: &str) -> bool {
    match Organic::builtin().parse(word) {
        Ok(_) => true,
        Err(OrganicRefusal::NotHydrocarbon) => false,
        Err(_) => true,
    }
}

/// «феррит цинка» and «цинковый феррит». Mirrors `try_material_class`.
fn material_class_at(lex: &Lexicon, words: &[String], i: usize) -> Option<Vec<(ChemToken, usize)>> {
    let classes = MaterialClasses::builtin();
    let next = words.get(i + 1)?;
    if classes.class_of(&words[i]).is_some() {
        element_by_word(lex, next)
            .map(|_| vec![(ChemToken::ClassWord, 1), (ChemToken::ElementByWord, 1)])
    } else {
        let class = classes.class_of(next)?;
        classes
            .cation_of_adjective(class, &words[i])
            .map(|_| vec![(ChemToken::ClassAdjective, 1), (ChemToken::ClassWord, 1)])
    }
}

#[cfg(test)]
mod chem_tests {
    use super::{chem_token_classes, ChemToken};

    fn classes(text: &str) -> Vec<String> {
        chem_token_classes(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The terminals defined in `chem.ebnf`: a line that starts with an
    /// UPPER_CASE name followed by `=`, outside `(* ... *)` comments.
    fn chem_ebnf_terminals() -> Vec<String> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/grammar/chem.ebnf");
        let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut code = String::new();
        let mut rest = source.as_str();
        while let Some(start) = rest.find("(*") {
            code.push_str(&rest[..start]);
            rest = match rest[start..].find("*)") {
                Some(end) => &rest[start + end + 2..],
                None => "",
            };
        }
        code.push_str(rest);
        code.lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let name_end = line
                    .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                    .unwrap_or(line.len());
                let name = &line[..name_end];
                (!name.is_empty() && line[name_end..].trim_start().starts_with('='))
                    .then(|| name.to_string())
            })
            .collect()
    }

    #[test]
    fn spelled_formula_words_map_to_their_classes() {
        assert_eq!(
            classes("аш два о"),
            names(&["ELEMENT_WORD", "INTEGER", "FUNCTION_LETTER"])
        );
        assert_eq!(
            classes("це о два"),
            names(&["ELEMENT_WORD", "FUNCTION_LETTER", "INTEGER"])
        );
        // «эн а» is one spelled pair: two pair words, then the element.
        assert_eq!(
            classes("эн а хлор"),
            names(&["LETTER_NAME", "FUNCTION_LETTER", "ELEMENT_WORD"])
        );
    }

    #[test]
    fn a_number_after_an_element_is_an_integer() {
        assert_eq!(classes("медь два"), names(&["ELEMENT_WORD", "INTEGER"]));
    }

    #[test]
    fn an_ion_marker_and_a_charge_sign() {
        assert_eq!(
            classes("ион меди два плюс"),
            names(&["ION_MARKER", "ELEMENT_WORD", "INTEGER", "PLUS"])
        );
        // «ион» before a complex-cation word is the lead-in, not the marker.
        assert_eq!(
            classes("ион тетрамминмеди два"),
            names(&["ION_LEAD_IN", "CATIONIC_COMPLEX_WORD", "INTEGER"])
        );
    }

    #[test]
    fn an_ambiguous_word_takes_the_class_the_parser_tries_first() {
        // «кислород» is a substance and an element. The substance dictionary
        // is tried first, so that is the class.
        assert_eq!(classes("кислород"), names(&["SUBSTANCE_NAME"]));
        // A two-word substance is one token.
        assert_eq!(classes("углекислый газ"), names(&["SUBSTANCE_NAME"]));
    }

    #[test]
    fn a_conjunction_without_a_reaction_is_a_function_letter() {
        // Outside a reaction «и» is not a `+`: it is a spelled letter or
        // nothing. Read as CONJUNCTION it would pass the grammar wrongly.
        assert_eq!(
            classes("натрий и калий"),
            names(&["ELEMENT_WORD", "FUNCTION_LETTER", "ELEMENT_WORD"])
        );
    }

    #[test]
    fn a_reaction_uses_its_connectives_and_drops_its_conditions() {
        assert_eq!(
            classes("натрий плюс хлор превращается в хлорид натрия при нагревании"),
            names(&[
                "ELEMENT_WORD",
                "PLUS",
                // «хлор» alone starts a species and is a substance there.
                "SUBSTANCE_NAME",
                "ARROW_JOINT",
                "ANION_CLASS",
                "ELEMENT_WORD",
            ])
        );
    }

    #[test]
    fn a_substance_name_must_cover_its_whole_species() {
        // «кислород» is a substance, but «кислород два» is not the substance
        // followed by a stray number: the substance dictionary needs the whole
        // species, so the word is read as an element with a subscript.
        assert_eq!(classes("кислород два"), names(&["ELEMENT_WORD", "INTEGER"]));
        // Mid-species, «хлор» is an element again, not the substance.
        assert_eq!(
            classes("аш два хлор"),
            names(&["ELEMENT_WORD", "INTEGER", "ELEMENT_WORD"])
        );
    }

    #[test]
    fn a_condition_outside_a_reaction_stays_a_condition() {
        assert_eq!(
            classes("хлорид натрия при нагревании"),
            names(&["ANION_CLASS", "ELEMENT_WORD", "CONDITION"])
        );
    }

    #[test]
    fn coordination_material_class_and_organic_words() {
        assert_eq!(
            classes("гексацианоферрат три калия"),
            names(&["ANIONIC_COMPLEX_WORD", "INTEGER", "ELEMENT_BY_WORD"])
        );
        assert_eq!(
            classes("феррит цинка"),
            names(&["CLASS_WORD", "ELEMENT_BY_WORD"])
        );
        assert_eq!(
            classes("цинковый феррит"),
            names(&["CLASS_ADJECTIVE", "CLASS_WORD"])
        );
        assert_eq!(classes("пентан"), names(&["HYDROCARBON_WORD"]));
        assert_eq!(classes("электрон"), names(&["ELECTRON_NAME"]));
        assert_eq!(
            classes("сульфат меди пентагидрат"),
            names(&["ANION_CLASS", "ELEMENT_WORD", "HYDRATE_WORD"])
        );
    }

    #[test]
    fn an_unknown_word_is_an_error_that_names_it() {
        let err = chem_token_classes("водкqxyz").unwrap_err();
        assert!(err.contains("водкqxyz"), "{err}");
    }

    #[test]
    fn every_terminal_the_function_can_return_is_defined_in_chem_ebnf() {
        let defined = chem_ebnf_terminals();
        // A reader that found nothing would pass every name below vacuously.
        assert!(defined.len() > 25, "only {} terminals read", defined.len());
        assert!(defined.iter().any(|n| n == "ELEMENT_WORD"));
        let mut seen = std::collections::BTreeSet::new();
        for token in ChemToken::ALL {
            let name = token.name();
            assert!(
                defined.iter().any(|n| n == name),
                "{name} is not a terminal of docs/grammar/chem.ebnf"
            );
            assert!(seen.insert(name), "{name} is listed twice");
        }
        // Output is built from the same names, so a sample run is a check too.
        for text in [
            "аш два о",
            "ион тетрамминмеди два",
            "натрий плюс хлор превращается в хлорид натрия",
        ] {
            for name in classes(text) {
                assert!(defined.contains(&name), "{name} from {text:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::math_token_classes;

    #[test]
    fn token_classes_match_the_ebnf_terminal_names() {
        assert_eq!(
            math_token_classes("два плюс два", false).unwrap(),
            ["NUM", "PLUS", "NUM"]
        );
        assert_eq!(math_token_classes("икс", false).unwrap(), ["SYM"]);
        assert_eq!(math_token_classes("а", false).unwrap(), ["WEAK_SYM"]);
        assert_eq!(
            math_token_classes("синус икс", false).unwrap(),
            ["FUNCTION", "SYM"]
        );
    }

    #[test]
    fn a_word_the_tokenizer_does_not_know_is_an_error() {
        let err = math_token_classes("водкqxyz", false).unwrap_err();
        assert!(err.contains("unknown word"), "{err}");
    }

    #[test]
    fn physics_mode_emits_unit_where_math_mode_refuses() {
        let physics = math_token_classes("два метра", true).unwrap();
        assert_eq!(physics, ["NUM", "UNIT"]);
        assert!(math_token_classes("два метра", false).is_err());
    }
}

#[cfg(test)]
mod nearest_word_tests {
    use super::*;

    /// `nearest_lexicon_words` with the words as `&str`, to compare with literals.
    fn nearest(word: &str) -> Option<(usize, Vec<String>)> {
        nearest_lexicon_words(word)
    }

    fn found(distance: usize, words: &[&str]) -> Option<(usize, Vec<String>)> {
        Some((distance, words.iter().map(|w| w.to_string()).collect()))
    }

    fn chars(word: &str) -> Vec<char> {
        word.chars().collect()
    }

    #[test]
    fn levenshtein_matches_hand_computed_distances_on_cyrillic() {
        let d = |a: &str, b: &str| levenshtein(&chars(a), &chars(b));
        assert_eq!(d("", ""), 0);
        assert_eq!(d("", "аб"), 2);
        assert_eq!(d("вода", "вода"), 0);
        // a -> o: one substitution
        assert_eq!(d("карбанат", "карбонат"), 1);
        // «котик» -> «кошка»: т->ш, и->к, к->а = 3
        assert_eq!(d("котик", "кошка"), 3);
        // «гырдксд» -> «гидроксид»: LCS is г р к с д = 5, so at least
        // 9 - 5 = 4 edits (ы->и, +д, +о, +и), and that alignment achieves 4.
        assert_eq!(d("гырдксд", "гидроксид"), 4);
        // A multi-byte letter is one edit, not two bytes.
        assert_eq!(d("ё", "е"), 1);
    }

    #[test]
    fn the_asr_error_words_find_their_lexicon_word() {
        // а->о
        assert_eq!(nearest("карбанат"), found(1, &["карбонат"]));
        // а->о
        assert_eq!(nearest("хларид"), found(1, &["хлорид"]));
        // а->о, as heard: `nearest_lexicon_words` does not apply the
        // spelling fixes of `normalize::words`.
        assert_eq!(nearest("гидраксид"), found(1, &["гидроксид"]));
        // е->а at index 4 («серн-е-я» vs «серн-а-я»). «серная» is a word of the
        // substance name «серная кислота»: see the multi-word test.
        assert_eq!(nearest("сернея"), found(1, &["серная"]));
    }

    #[test]
    fn two_edits_are_found_and_three_are_not() {
        // к->г, then delete ж: 2. «гидроксида» is also 2 (к->г, ж->а): a tie.
        assert_eq!(
            nearest("кидроксидж"),
            found(2, &["гидроксид", "гидроксида"])
        );
        // Exactly MAX_WORD_EDITS + 1: «карбонат» with а->е, о->е, а->и.
        assert_eq!(MAX_WORD_EDITS, 2);
        assert_eq!(levenshtein(&chars("кербенит"), &chars("карбонат")), 3);
        assert_eq!(nearest("кербенит"), None);
        // «гырдксд» is 4 from «гидроксид» (see the Levenshtein test).
        assert_eq!(nearest("гырдксд"), None);
        // Letters only: nothing within 2 edits.
        assert_eq!(nearest("абвгдежзи"), None);
    }

    #[test]
    fn a_vocabulary_word_a_short_word_and_non_words_give_none() {
        assert_eq!(nearest("натрия"), None);
        assert_eq!(nearest("хлорид"), None);
        // Case and ё do not matter, as for lattice words.
        assert_eq!(nearest("Хлорид"), None);
        assert_eq!(nearest("КАРБАНАТ"), found(1, &["карбонат"]));
        // 4 characters: one below the minimum, although «сера» is not far.
        assert_eq!(MIN_CORRECTED_WORD_CHARS, 5);
        assert_eq!(nearest("серо"), None);
        // Length counts characters, not bytes: «серо» is 4 characters and 8
        // bytes, and still too short.
        assert!("серо".len() >= MIN_CORRECTED_WORD_CHARS);
        assert_eq!(nearest("жлз"), None);
        // Two words, empty, and punctuation are not one word.
        assert_eq!(nearest("карбанат кальция"), None);
        assert_eq!(nearest(""), None);
    }

    #[test]
    fn the_shortest_word_is_corrected_and_the_one_below_it_is_not() {
        // A vocabulary word of exactly MIN_CORRECTED_WORD_CHARS characters,
        // with one letter broken.
        let five = vocabulary_names(Lexicon::builtin())
            .find(|w| w.chars().count() == MIN_CORRECTED_WORD_CHARS && !w.contains(' '))
            .expect("the lexicon has a five-letter word");
        let mut broken = chars(five);
        broken[1] = 'ъ';
        let broken: String = broken.into_iter().collect();
        let (distance, words) = nearest_lexicon_words(&broken).expect("five letters are tried");
        assert_eq!(distance, 1);
        assert!(words.iter().any(|w| w == five), "{broken} -> {words:?}");
        // The same break on a four-letter word is not tried.
        let four: String = broken.chars().take(MIN_CORRECTED_WORD_CHARS - 1).collect();
        assert_eq!(nearest_lexicon_words(&four), None);
    }

    #[test]
    fn every_tie_is_returned_sorted() {
        // «серный» (ы) and «серной» (о) differ in one letter; «сернуй» is one
        // substitution from each. Sorted by code point: о (U+043E) < ы (U+044B).
        assert_eq!(nearest("сернуй"), found(1, &["серной", "серный"]));
    }

    #[test]
    fn a_word_of_a_multi_word_name_is_vocabulary_but_the_lattice_does_not_know_it() {
        let lex = Lexicon::builtin();
        // The substance name is two words and `known_word` compares one word
        // with a whole name, so it does not accept «серная» alone.
        assert!(vocabulary_names(lex).any(|n| n == "серная кислота"));
        assert!(!vocabulary_names(lex).any(|n| n == "серная"));
        // Taking the words apart makes «серная» a vocabulary word: itself it
        // is not corrected, and its neighbour is.
        assert_eq!(nearest("серная"), None);
        assert_eq!(nearest("сернея"), found(1, &["серная"]));
        // The second word of a name counts too: «кислота» -> «кислата».
        assert_eq!(nearest("кислата"), found(1, &["кислота"]));
    }

    #[test]
    fn a_huge_word_is_rejected_by_length_without_a_table() {
        let huge = "а".repeat(1 << 20);
        assert_eq!(nearest_lexicon_words(&huge), None);
    }

    fn prose_negative_words() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../research/data/prose-negatives-v2.jsonl"
        );
        let text = std::fs::read_to_string(path).expect("prose-negatives-v2.jsonl");
        let mut words = Vec::new();
        let mut rows = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let row: serde_json::Value = serde_json::from_str(line).expect("a JSON row");
            let transcript = row["human_transcript"].as_str().expect("human_transcript");
            // The words the lattice sees; only letters, as a number or a
            // symbol is not a word to correct.
            words.extend(
                split_words(transcript)
                    .into_iter()
                    .filter(|w| w.chars().all(char::is_alphabetic))
                    .filter(|w| w.chars().count() >= MIN_CORRECTED_WORD_CHARS),
            );
            rows += 1;
        }
        // A reader that found nothing would report "0 of 0" as safe.
        assert_eq!(rows, 180, "prose-negatives-v2.jsonl rows");
        words
    }

    #[test]
    fn report_how_many_ordinary_words_get_an_answer() {
        let words = prose_negative_words();
        assert!(words.len() > 500, "only {} words read", words.len());
        let distinct: BTreeSet<&String> = words.iter().collect();
        let mut hits: Vec<(usize, String)> = distinct
            .iter()
            .filter_map(|w| {
                let (d, near) = nearest_lexicon_words(w)?;
                Some((d, format!("{w} -> {near:?} (d={d})")))
            })
            .collect();
        hits.sort();
        let by_distance = |d: usize| hits.iter().filter(|(hit_d, _)| *hit_d == d).count();
        let occurrences = words
            .iter()
            .filter(|w| nearest_lexicon_words(w).is_some())
            .count();
        eprintln!(
            "prose-negatives-v2: {} words >= {} chars, {} distinct; {} words ({} distinct) get an answer: d=1 {}, d=2 {}",
            words.len(),
            MIN_CORRECTED_WORD_CHARS,
            distinct.len(),
            occurrences,
            hits.len(),
            by_distance(1),
            by_distance(2),
        );
        for (_, hit) in hits.iter().take(20) {
            eprintln!("  {hit}");
        }
    }
}

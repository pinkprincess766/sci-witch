use crate::ast::{Arrow, Chemical, Equation, Formula, Node, Part, Species, StateMarker};
use crate::error::{Error, Result};
use crate::formula::{ionic_compound, parse_formula_str};
use crate::lexicon::{AnionClass, ChemConnective, Element, IonRole, Lexicon};
use crate::numbers::NumberLex;

/// Largest subscript or coefficient a spoken formula may carry.
///
/// Generous next to real chemistry — BaFe₁₂O₁₉ needs 19, fullerene C₆₀ needs
/// 60 — and small next to what a misheard number produces. Without it,
/// «гидроксид железа 4294967295» came out as `Fe₄₂₉₄₉₆₇₂₉₅`, which is not a
/// formula but does look like one.
pub const MAX_ATOM_COUNT: u32 = 999;

pub fn parse_chemistry(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Result<Node> {
    // A comma, a colon or a dash between two names is the end of one item
    // and the start of the next, so the two are not read as one compound.
    if separator_splits_names(words, lex) {
        return Err(Error::Parse {
            domain: "chemistry",
            reason: "punctuation between two names: they are separate items, not one compound"
                .into(),
        });
    }
    // Whisper inserts commas around «превращается в». A colon or a dash that
    // did not split two names is the same kind of pause and is dropped here.
    let cleaned = spoken_words(words);
    let words = &cleaned;
    if words.is_empty() {
        return Err(Error::Parse {
            domain: "chemistry",
            reason: "empty input".into(),
        });
    }
    // A reaction shape is recognised first. If the sentence carries the shape
    // but a side does not parse as chemistry, the whole utterance fails: an
    // arrow is never accepted between things that are not substances.
    if let Some(equation) = parse_reaction(words, lex, nums) {
        return Ok(Node::Chemical(Chemical::Equation(equation?)));
    }
    let species = parse_species(words, lex, nums)?;
    Ok(Node::Chemical(Chemical::Species(species)))
}

/// One group of words that should name a substance, together with the
/// connective that introduced it.
#[derive(Clone, Debug)]
struct Chunk {
    words: Vec<String>,
    opened_by: Option<ChemConnective>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// Recognises a spoken reaction.
///
/// `None` means the sentence carries no reaction shape at all, and the caller
/// should try to read it as a single substance. `Some(Err(..))` means the
/// shape was there but the chemistry was not — «реакция идёт быстрее» has a
/// reaction noun and nothing else, and must not become an equation.
///
/// The connectives come from `aliases.yaml`; this function holds no list of
/// spoken phrases of its own.
fn parse_reaction(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Option<Result<Equation>> {
    let speech = &lex.chemistry_speech;
    let (words, condition) = strip_conditions(words, lex);

    let mut chunks: Vec<Chunk> = vec![Chunk {
        words: Vec::new(),
        opened_by: None,
    }];
    let mut saw_structure = false;
    let mut index = 0usize;
    while index < words.len() {
        if let Some((connective, used)) = speech.connective_at(&words, index) {
            // «ион меди два плюс» — the first `плюс` after an ion marker is
            // the charge, not a separator. This is the one place where a
            // connective is absorbed into the substance it follows.
            let current = chunks.last().expect("there is always a current chunk");
            // …but only when nothing follows it that could be another
            // reagent. «ион серебра плюс ион хлора» is two ions, not one
            // positive silver: absorbing the `плюс` there swallowed the
            // chloride and produced a one-sided equation.
            let plus_is_a_charge = connective == ChemConnective::Plus
                && current.words.iter().any(|word| speech.is_ion_marker(word))
                && !current
                    .words
                    .last()
                    .is_some_and(|word| word == "плюс" || word == "минус")
                && !starts_a_species(lex, &words, index + used);
            if !plus_is_a_charge {
                if !matches!(
                    connective,
                    ChemConnective::Plus | ChemConnective::Conjunction
                ) {
                    saw_structure = true;
                }
                chunks.push(Chunk {
                    words: Vec::new(),
                    opened_by: Some(connective),
                });
                index += used;
                continue;
            }
        }
        chunks
            .last_mut()
            .expect("there is always a current chunk")
            .words
            .push(words[index].clone());
        index += 1;
    }

    if !saw_structure {
        return None;
    }

    // A connective that leaves no words behind is only meaningful as a bridge:
    // «между A и B протекает реакция с образованием C» has nothing between the
    // noun and the product marker. Anywhere else an empty group is a broken
    // sentence, not a reaction.
    let mut kept: Vec<Chunk> = Vec::new();
    for (position, chunk) in chunks.iter().enumerate() {
        if !chunk.words.is_empty() {
            kept.push(chunk.clone());
            continue;
        }
        let before = chunk.opened_by;
        let after = chunks.get(position + 1).and_then(|next| next.opened_by);
        let bridged = [before, after].into_iter().flatten().any(|connective| {
            matches!(
                connective,
                ChemConnective::ReactionNoun
                    | ChemConnective::FromMarker
                    | ChemConnective::BetweenMarker
            )
        });
        if !bridged {
            return Some(Err(Error::Parse {
                domain: "chemistry",
                reason: "a reaction connective with nothing on one of its sides".into(),
            }));
        }
    }

    let mut left: Vec<Vec<String>> = Vec::new();
    let mut right: Vec<Vec<String>> = Vec::new();
    let mut side = Side::Left;
    let mut arrow: Option<Arrow> = None;
    let mut decomposition = false;

    for chunk in kept {
        match chunk.opened_by {
            None => {}
            Some(ChemConnective::Plus) => {}
            Some(ChemConnective::Conjunction) => {
                // `и` may only ever be a `+`, and only next to a group that
                // is already being read as chemistry. Products of a
                // decomposition and further reagents qualify; a bare `и`
                // before any structure does not.
                if arrow.is_none() && !decomposition && left.is_empty() {
                    return Some(Err(Error::Parse {
                        domain: "chemistry",
                        reason: "«и» outside a chemical side".into(),
                    }));
                }
            }
            Some(ChemConnective::JoinReagent) => {}
            Some(ChemConnective::FromMarker) | Some(ChemConnective::BetweenMarker) => {
                if arrow.is_some() {
                    return Some(Err(Error::Parse {
                        domain: "chemistry",
                        reason: "a reaction opening after the arrow".into(),
                    }));
                }
                side = Side::Left;
            }
            Some(ChemConnective::ReactionNoun) => {}
            Some(kind) => {
                if arrow.is_some() {
                    return Some(Err(Error::Parse {
                        domain: "chemistry",
                        reason: "two arrows in one reaction".into(),
                    }));
                }
                arrow = Some(if kind == ChemConnective::Equilibrium {
                    Arrow::Equilibrium
                } else {
                    Arrow::Forward
                });
                decomposition = kind == ChemConnective::Decompose;
                side = Side::Right;
            }
        }
        if chunk.words.is_empty() {
            continue;
        }
        match side {
            Side::Left => left.push(chunk.words),
            Side::Right => right.push(chunk.words),
        }
    }

    let arrow = arrow?;
    if left.is_empty() || right.is_empty() {
        return Some(Err(Error::Parse {
            domain: "chemistry",
            reason: "a reaction needs substances on both sides".into(),
        }));
    }

    let parse_all = |groups: Vec<Vec<String>>| -> Result<Vec<Species>> {
        groups
            .into_iter()
            .map(|group| parse_species(&group, lex, nums))
            .collect()
    };
    let left = match parse_all(left) {
        Ok(species) => species,
        Err(error) => return Some(Err(error)),
    };
    let right = match parse_all(right) {
        Ok(species) => species,
        Err(error) => return Some(Err(error)),
    };
    Some(Ok(Equation {
        left,
        arrow,
        right,
        condition,
    }))
}

/// Pulls the reaction conditions out of the word list. The phrases come from
/// `aliases.yaml`.
/// Whether a species could begin at `i`.
///
/// Used to tell the two meanings of «плюс» apart: the sign of an ion, or
/// the joint between two reagents. Deliberately permissive about what a
/// species is — an ion marker, an element, an ion class, a substance name
/// or a coefficient — because the cost of the two mistakes is not
/// symmetric. Reading a separator as a charge loses a reagent; reading a
/// charge as a separator leaves an unparsable fragment, which abstains.
fn starts_a_species(lex: &Lexicon, words: &[String], i: usize) -> bool {
    let Some(word) = words.get(i) else {
        return false;
    };
    if lex.chemistry_speech.is_ion_marker(word)
        || lex.element(word).is_some()
        || lex.anion(word).is_some()
        // «плюс электрон» is a term of a half-reaction, not a sign.
        || Coordination::builtin().is_electron(word)
    {
        return true;
    }
    if lex.longest_substance(words, i).is_some() {
        return true;
    }
    // A coefficient in front of a formula: «плюс два аш два о».
    NumberLex::new()
        .consume_int(words, i)
        .is_some_and(|(_, used)| starts_a_species(lex, words, i + used))
}

pub(crate) fn strip_conditions(words: &[String], lex: &Lexicon) -> (Vec<String>, Option<String>) {
    let speech = &lex.chemistry_speech;
    let mut out = Vec::new();
    let mut condition = None;
    let mut index = 0;
    while index < words.len() {
        if let Some((kind, used)) = speech.condition_at(words, index) {
            condition.get_or_insert_with(|| kind.as_str().to_string());
            index += used;
            continue;
        }
        out.push(words[index].clone());
        index += 1;
    }
    (out, condition)
}

pub fn parse_species(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Result<Species> {
    parse_species_with_evidence(words, lex, nums).map(|(species, _)| species)
}

/// The same parse, with the evidence for how it was built.
///
/// The proof of a material-class template lives here rather than inside the
/// AST: `Species` takes part in equality against the corpus, and a formula
/// built from a template must compare equal to the same formula written by
/// hand. Callers that want to know *why* a formula was produced ask for it;
/// callers that want the chemistry are unaffected.
pub fn parse_species_with_evidence(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
) -> Result<(Species, SpeciesEvidence)> {
    // A trailing «пентагидрат» wraps whatever precedes it, so it is peeled
    // off before anything else is tried and re-attached at the end.
    if let Some(stripped) = strip_hydrate(words) {
        let (head, waters) = stripped?;
        if head.is_empty() {
            return Err(Error::Parse {
                domain: "chemistry",
                reason: "гидрат без вещества".into(),
            });
        }
        let (mut species, evidence) = parse_species_inner(&head, lex, nums)?;
        species.formula.parts.push(Part::Hydrate { count: waters });
        return Ok((species, evidence));
    }
    parse_species_inner(words, lex, nums)
}

fn parse_species_inner(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
) -> Result<(Species, SpeciesEvidence)> {
    if words.is_empty() {
        return Err(Error::Parse {
            domain: "chemistry",
            reason: "empty species".into(),
        });
    }

    let mut words = words.to_vec();
    let mut coefficient = 1u32;
    let mut had_coefficient = false;
    if let Some((n, used)) = nums.consume_int(&words, 0) {
        if used < words.len() && n > 0 {
            if n > MAX_ATOM_COUNT {
                return Err(Error::Parse {
                    domain: "chemistry",
                    reason: format!("коэффициент {n} выходит за предел {MAX_ATOM_COUNT}"),
                });
            }
            coefficient = n;
            had_coefficient = true;
            words = words[used..].to_vec();
        }
    }

    // Substance names like «углекислый газ» must win before the trailing
    // «газ» marker is stripped.
    if let Some(mut s) = try_full_substance(&words, lex) {
        s.coefficient = coefficient;
        return Ok((s, SpeciesEvidence::default()));
    }
    // A coordination name decomposes inside a single word, so it is tried
    // before the marker strip could take a syllable off the end.
    if let Some(result) = try_coordination_cationic(&words, lex, nums) {
        let mut s = result?;
        s.coefficient = coefficient;
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(result) = try_coordination_anionic(&words, lex, nums) {
        let mut s = result?;
        s.coefficient = coefficient;
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(species) = try_electron(&words) {
        let mut species = species;
        species.coefficient = coefficient;
        return Ok((species, SpeciesEvidence::default()));
    }
    if let Some(result) = try_organic(&words) {
        let mut s = result?;
        s.coefficient = coefficient;
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(result) = try_material_class(&words, lex) {
        let (mut s, proof) = result?;
        s.coefficient = coefficient;
        return Ok((
            s,
            SpeciesEvidence {
                class_proof: Some(proof),
            },
        ));
    }

    let (words, marker) = strip_marker(&words, lex);
    if words.is_empty() {
        return Err(Error::Parse {
            domain: "chemistry",
            reason: "empty species".into(),
        });
    }
    if let Some(mut s) = try_ion(&words, lex, nums) {
        s.coefficient = coefficient;
        s.marker = marker.or(s.marker);
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(mut s) = try_systematic(&words, lex, nums) {
        s.coefficient = coefficient;
        s.marker = marker;
        return Ok((s, SpeciesEvidence::default()));
    }
    // An element *named* — «водород», «бром», not the letter «аш» — is the
    // substance, and the substance of a diatomic element is its molecule.
    // This has to come before `try_spelled`, which reads the same word as the
    // atom: «водород газ» answered `H↑` while «водород» answered `H₂`, so a
    // state marker changed the formula. A letter dictation is a different
    // register and stays as dictated («аш газ» is `H↑`).
    if let Some(mut s) = try_named_diatomic(&words, lex) {
        s.coefficient = coefficient;
        s.marker = marker;
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(mut s) = try_spelled(&words, lex, nums, had_coefficient) {
        s.coefficient = coefficient;
        s.marker = marker;
        return Ok((s, SpeciesEvidence::default()));
    }
    if let Some(el) = lex.element(&words[0]) {
        if words.len() == 1 {
            let formula = if el.diatomic {
                Formula::atom(&el.symbol, 2)
            } else {
                Formula::atom(&el.symbol, 1)
            };
            return Ok((
                Species {
                    coefficient,
                    formula,
                    charge: None,
                    marker,
                },
                SpeciesEvidence::default(),
            ));
        }
    }

    Err(Error::Parse {
        domain: "chemistry",
        reason: format!("cannot interpret species '{}'", words.join(" ")),
    })
}

fn strip_marker(words: &[String], lex: &Lexicon) -> (Vec<String>, Option<StateMarker>) {
    let speech = &lex.chemistry_speech;
    for start in (0..words.len()).rev() {
        if let Some((marker, used)) = speech.marker_at(words, start) {
            if start + used == words.len() {
                return (words[..start].to_vec(), Some(marker));
            }
        }
    }
    (words.to_vec(), None)
}

/// One word that names a diatomic element (`diatomic: true` in
/// `elements.yaml`) and is not a spelled letter or Latin-register name.
fn try_named_diatomic(words: &[String], lex: &Lexicon) -> Option<Species> {
    let [word] = words else {
        return None;
    };
    if spelling_evidence(lex, word) || FUNCTION_WORD_LETTERS.contains(&word.as_str()) {
        return None;
    }
    let el = lex.element(word)?;
    if !el.diatomic {
        return None;
    }
    Some(Species::new(Formula::atom(&el.symbol, 2)))
}

fn try_full_substance(words: &[String], lex: &Lexicon) -> Option<Species> {
    if let Some((formula, used)) = lex.longest_substance(words, 0) {
        if used == words.len() {
            return Some(Species::new(formula));
        }
    }
    None
}

fn try_ion(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Option<Species> {
    let speech = &lex.chemistry_speech;
    if !words.iter().any(|w| speech.is_ion_marker(w)) {
        return None;
    }
    // A charge sign closes the species. An ion marker that comes *after* one
    // opens the next species: «ион натрия плюс ион» is not Na⁺ with a stray
    // word left over, it is Na⁺, a «плюс» and the start of another ion.
    // Filtering every marker out of the words wherever it stood used to
    // swallow that second «ион», so «ион натрия плюс ион хлора» came out as
    // `Na⁺ Cl₂`: the «плюс» gone, the second ion turned into a molecule.
    if let Some(sign) = words.iter().position(|w| w == "плюс" || w == "минус") {
        if words[sign + 1..].iter().any(|w| speech.is_ion_marker(w)) {
            return None;
        }
    }
    let mut filtered: Vec<String> = words
        .iter()
        .filter(|w| !speech.is_ion_marker(w))
        .cloned()
        .collect();
    if filtered.is_empty() {
        return None;
    }

    let mut charge_sign: Option<i32> = None;
    if let Some(last) = filtered.last() {
        if last == "плюс" {
            charge_sign = Some(1);
            filtered.pop();
        } else if last == "минус" {
            charge_sign = Some(-1);
            filtered.pop();
        }
    }

    let mut mag: Option<u32> = None;
    if let Some((n, used)) = nums.consume_int(&filtered, filtered.len().saturating_sub(1)) {
        if used == 1 {
            mag = Some(n);
            filtered.pop();
        }
    }
    // "два плюс" already handled; also "два" may remain as ox on metal

    // anion class first: сульфат [ион] [два] минус
    if let Some(an) = filtered.first().and_then(|w| lex.anion(w)) {
        if an.role == IonRole::Anion && filtered.len() == 1 {
            let ch = ion_charge(mag, charge_sign, an.charge)?;
            return Some(Species {
                coefficient: 1,
                formula: an.formula.clone(),
                charge: Some(ch),
                marker: None,
            });
        }
    }

    // An element named as an ion: «ион серебра», «ион меди два», «ион хлора».
    if let Some(el) = filtered.first().and_then(|w| lex.element(w)) {
        let ox = if filtered.len() >= 2 {
            nums.consume_int(&filtered, 1).map(|(n, _)| n)
        } else {
            None
        };
        // Every word must be accounted for, or this is not the whole
        // species: «серебра плюс хлора» used to return Ag⁺ and drop the
        // rest of the equation on the floor.
        let consumed = 1 + usize::from(ox.is_some());
        if consumed != filtered.len() {
            return None;
        }
        // Two numbers for one charge is not a charge. «ион меди два три»
        // used to answer Cu³⁺, quietly discarding the «два».
        if mag.is_some() && ox.is_some() {
            return None;
        }
        // The sign comes from the element, not from a default of +1.
        // Chlorine's `default_oxidation` is −1 and always was; ignoring it
        // turned «ион хлора» into Cl⁺, which is not a thing.
        let Some(default) = el.default_oxidation else {
            // Nothing to base a sign on, and guessing one would be a claim.
            return None;
        };
        let charge = ion_charge(mag.or(ox), charge_sign, default)?;
        return Some(Species {
            coefficient: 1,
            formula: Formula::atom(&el.symbol, 1),
            charge: Some(charge),
            marker: None,
        });
    }

    if let Some(an) = filtered.first().and_then(|w| lex.anion(w)) {
        if filtered.len() != 1 {
            return None;
        }
        let ch = ion_charge(mag, charge_sign, an.charge)?;
        return Some(Species {
            coefficient: 1,
            formula: an.formula.clone(),
            charge: Some(ch),
            marker: None,
        });
    }
    None
}

/// Applies an optional spoken magnitude and sign to an ion's default
/// charge without ever narrowing a `u32` through `as i32`.
///
/// That unchecked cast used to turn `4294967295` into `-1`, so «ион меди
/// 4294967295» was rendered as `Cu⁻`. A number that cannot be represented as
/// a charge is not a different, smaller charge: the parse must abstain.
fn ion_charge(magnitude: Option<u32>, sign: Option<i32>, default: i32) -> Option<i32> {
    if default == 0 {
        return None;
    }
    let raw = magnitude.unwrap_or(default.unsigned_abs());
    let magnitude = i32::try_from(raw).ok()?;
    if magnitude == 0 {
        return None;
    }
    let sign = sign.unwrap_or_else(|| default.signum());
    magnitude.checked_mul(sign)
}

fn try_systematic(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Option<Species> {
    if words.is_empty() {
        return None;
    }
    let an = lex.anion(&words[0])?;
    if an.role != IonRole::Anion {
        return None;
    }
    if words.len() < 2 {
        return None;
    }
    let mut i = 1;
    let (cation_f, cat_charge, cat_poly) = if let Some(el) = lex.element(&words[i]) {
        i += 1;
        let mut ox = el.default_oxidation.unwrap_or(1);
        if i < words.len() {
            // «плюс три», «римское три», «в степени окисления три» and a
            // bare «три» all name the same thing here, because after an
            // anion and a metal there is nothing else a number could be.
            match oxidation_at(
                words,
                i,
                nums,
                Coordination::builtin(),
                OxidationContext::SimpleSalt,
            ) {
                OxidationRead::Found { value, used } => {
                    ox = value;
                    i += used;
                }
                // A number was said that cannot be a cation's charge:
                // «минус три», «ноль», or one too large for the type. The
                // words stay as they were rather than being answered with a
                // different number.
                OxidationRead::Refused(_) => return None,
                OxidationRead::Absent => {}
            }
        }
        if i != words.len() {
            return None;
        }
        (Formula::atom(&el.symbol, 1), ox, false)
    } else {
        let cat = lex.anion(&words[i])?;
        if cat.role != IonRole::Cation {
            return None;
        }
        i += 1;
        if i != words.len() {
            // optional trailing number ignored for ammonium
            if let Some((_, used)) = nums.consume_int(words, i) {
                i += used;
            }
        }
        if i != words.len() {
            return None;
        }
        (cat.formula.clone(), cat.charge, cat.group)
    };

    // `None` where the charges admit no compound, or where the arithmetic
    // would leave the range of its type. Returning a partial formula here is
    // what produced `Fe` from «гидроксид железа ноль».
    let formula = ionic_compound(
        cation_f,
        cat_charge,
        an.formula.clone(),
        an.charge,
        an.group,
        cat_poly,
    )?;
    Some(Species::new(formula))
}

/// `after_number` is whether the caller took a coefficient off the front of
/// `words`: «два и три» reaches this function as «и три» with the «два»
/// already gone, and the conjunction rule in [`spelled_context`] has to know.
fn try_spelled(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
    after_number: bool,
) -> Option<Species> {
    let mut parts: Vec<Part> = Vec::new();
    let mut i = 0;
    let mut saw_element = false;
    while i < words.len() {
        if let Some((count, used)) = lex.chemistry_speech.grouping_at(words, i) {
            if !apply_times(&mut parts, count, lex) {
                return None;
            }
            i += used;
            continue;
        }
        if let Some((el, used_el)) = chemistry_element_at(lex, words, i, nums, after_number) {
            i += used_el;
            let mut count = 1u32;
            if i < words.len() {
                if let Some((n, used)) = nums.consume_int(words, i) {
                    // A zero subscript is not one atom, and a subscript of
                    // four billion is not a formula. Both used to be
                    // silently repaired: `n.max(1)` turned «ноль» into 1.
                    if n == 0 || n > MAX_ATOM_COUNT {
                        return None;
                    }
                    // «…девятьсот девяносто девять тысяч» is 999 000, and
                    // the additive number lexicon reads it as 999. Taking
                    // the part it understood produced `Fe(OH)₉₉₉ тысяч`.
                    if nums.starts_unsupported_scale(words, i + used) {
                        return None;
                    }
                    count = n;
                    i += used;
                }
            }
            parts.push(Part::Atom {
                symbol: el.symbol.clone(),
                count,
            });
            saw_element = true;
            continue;
        }
        return None;
    }
    if !saw_element || parts.is_empty() {
        return None;
    }
    Some(Species::new(Formula { parts }))
}

/// Single spoken letters that are also ordinary Russian words.
///
/// Every one of them is a preposition, conjunction or particle a person says
/// constantly, and every one of them also names a Latin letter in
/// `symbols.yaml`. Read as element symbols they produce iodine, sulfur,
/// oxygen, uranium, potassium, boron and vanadium out of thin air:
/// «натрий и калий стоят рядом» came out as `NaIK`, and «медь, о которой я
/// говорил» came out as `CuO` — a different substance, at full confidence.
///
/// The mathematics parser reads the same list (`parser::math`): the same
/// words are letter names there, and «икс и игрек» must not become `xiy`.
pub(crate) const FUNCTION_WORD_LETTERS: [&str; 11] =
    ["а", "б", "в", "ж", "же", "и", "к", "о", "с", "у", "я"];

/// The function-word letters that are conjunctions.
///
/// A conjunction *between two numbers* — «два и три», «пять а шесть» — is
/// followed by a number without that number being a subscript, so there it
/// does not vouch for an element (see [`spelled_context`]). Prepositions do
/// not have the problem in the nominative: «о два» is not something a person
/// says about something.
const CONJUNCTION_LETTERS: [&str; 2] = ["а", "и"];

/// Whether `word` could only have been said while spelling a formula.
///
/// Two registers count: a spoken Latin letter that is not also a Russian
/// word («аш», «эс», «цэ»), and an element name from the spelled register in
/// `elements.yaml` («купрум», «ferrum»). Nobody reaches for either of those
/// in the middle of an ordinary sentence.
fn spelling_evidence(lex: &Lexicon, word: &str) -> bool {
    if FUNCTION_WORD_LETTERS.contains(&word) {
        return false;
    }
    lex.latin(word).is_some() || lex.spelled_element_names.contains(word)
}

/// Whether an ambiguous single letter at `i` may be read as an element.
///
/// Two things make a spelled formula recognisable without a dictionary of
/// Russian grammar:
///
/// * the letter carries a subscript — «марганец **о два**» is MnO₂, and no
///   preposition is followed by a bare number. A *conjunction* between two
///   numbers is, so «и три» in «два и три» proves nothing and is not iodine
///   with a subscript;
/// * or the same chunk holds a word from the spelled register — «**аш** два
///   о» and «**купрум** о» can only be spellings.
///
/// Neither holds for «натрий **и** калий» or «медь **о** которой», which is
/// the whole point. The cost is that a bare «о» with no subscript and no
/// spelled neighbour is no longer oxygen; that is not notation anybody
/// dictates, and the lattice still offers the reading as a candidate.
fn spelled_context(
    lex: &Lexicon,
    words: &[String],
    i: usize,
    nums: &NumberLex,
    after_number: bool,
) -> bool {
    if nums.consume_int(words, i + 1).is_some() {
        if !CONJUNCTION_LETTERS.contains(&words[i].as_str())
            || !follows_a_number(words, i, nums, after_number)
        {
            return true;
        }
        // A conjunction between two numbers proves nothing: «два **и три**»
        // is a sentence, not iodine with a subscript. «и два» at the start
        // of a chunk is still the formula it always was (and «и два о пять»
        // never reaches this point). After a number the conjunction is
        // vouched for only by a *second* letter that carries a subscript of
        // its own and is not a conjunction — «два и три **о пять**» — since
        // no sentence has a conjunction, a number and then a preposition with
        // another number after it. The evidence has to come from another
        // word (a conjunction may not vouch for a conjunction), and «в» does
        // not count: «в два раза» is a number after a preposition, said
        // constantly, and «три и пять в два раза» must not become iodine and
        // vanadium.
        return words.iter().enumerate().any(|(index, word)| {
            index != i
                && FUNCTION_WORD_LETTERS.contains(&word.as_str())
                && !CONJUNCTION_LETTERS.contains(&word.as_str())
                && word != "в"
                && nums.consume_int(words, index + 1).is_some()
        }) || spelled_neighbour(lex, words, i);
    }
    spelled_neighbour(lex, words, i)
}

/// Whether the word right before `i` is a number word. At the front of the
/// chunk the answer is what the caller knows: `after_number` says a
/// coefficient was taken off.
fn follows_a_number(words: &[String], i: usize, nums: &NumberLex, after_number: bool) -> bool {
    if i == 0 {
        return after_number;
    }
    nums.consume_int(words, i - 1)
        .is_some_and(|(_, used)| used == 1)
}

/// Whether any word other than the one at `i` could only be part of a
/// spelling. The word itself is excluded: «о» may not vouch for «о».
fn spelled_neighbour(lex: &Lexicon, words: &[String], i: usize) -> bool {
    words
        .iter()
        .enumerate()
        .any(|(index, word)| index != i && spelling_evidence(lex, word.as_str()))
}

pub(crate) fn chemistry_element_at<'a>(
    lex: &'a Lexicon,
    words: &[String],
    i: usize,
    nums: &NumberLex,
    after_number: bool,
) -> Option<(&'a Element, usize)> {
    if i + 1 < words.len() {
        let pair = match (words[i].as_str(), words[i + 1].as_str()) {
            ("эн", "а") => Some("Na"),
            ("цэ", "а") | ("це", "а") => Some("Ca"),
            ("цэ", "эль") | ("це", "эль") => Some("Cl"),
            ("эм", "гэ") | ("эм", "г") => Some("Mg"),
            ("а", "эль") => Some("Al"),
            ("цэ", "у") | ("це", "у") => Some("Cu"),
            ("зет", "эн") => Some("Zn"),
            ("эф", "е") => Some("Fe"),
            ("эм", "эн") => Some("Mn"),
            _ => None,
        };
        if let Some(sym) = pair {
            if let Some(el) = lex.elements_by_symbol.get(sym) {
                return Some((el, 2));
            }
        }
    }
    chemistry_element(lex, words, i, nums, after_number).map(|el| (el, 1))
}

fn chemistry_element<'a>(
    lex: &'a Lexicon,
    words: &[String],
    i: usize,
    nums: &NumberLex,
    after_number: bool,
) -> Option<&'a Element> {
    let word = words[i].as_str();
    // The guard comes first because the collision lives in two tables at
    // once: «о» is a spoken Latin letter *and* `elements.yaml` lists it
    // among oxygen's own names, so checking only the letter table left
    // «медь о которой я говорил» reading as CuO.
    if FUNCTION_WORD_LETTERS.contains(&word) && !spelled_context(lex, words, i, nums, after_number)
    {
        return None;
    }
    if let Some(el) = lex.element(word) {
        return Some(el);
    }
    // Single spoken latin letters used as element symbols in spelled
    // formulas: о, эс, аш, цэ, эн.
    if let Some(ch) = lex.latin(word) {
        let sym = ch.to_ascii_uppercase().to_string();
        return lex.elements_by_symbol.get(&sym);
    }
    None
}

/// Applies «дважды»/«трижды» to the radical just spelled out.
///
/// Returns whether there was a radical to apply it to. There is no fallback
/// that wraps a single atom: `(Fe)₃` is not notation anybody writes, and the
/// fallback that produced it also fired on ordinary speech — «железо трижды
/// промыли водой» came out as «(Fe)₃ промыли H₂O». When nothing groups, the
/// spelled reading fails and the words stay as they were said.
#[must_use]
fn apply_times(parts: &mut Vec<Part>, n: u32, lex: &Lexicon) -> bool {
    if parts.is_empty() {
        return false;
    }
    // Prefer matching a known polyatomic suffix (OH, SO4, …).
    let mut radicals: Vec<&AnionClass> = lex
        .anion_classes
        .values()
        .filter(|a| a.group && a.role == IonRole::Anion)
        .collect();
    radicals.sort_by_key(|a| std::cmp::Reverse(a.formula.parts.len()));
    // unique by id
    let mut seen = std::collections::HashSet::new();
    radicals.retain(|a| seen.insert(a.id.clone()));

    for rad in radicals {
        let rlen = rad.formula.parts.len();
        if rlen == 0 || parts.len() < rlen {
            continue;
        }
        let suffix = &parts[parts.len() - rlen..];
        if suffix == rad.formula.parts {
            parts.truncate(parts.len() - rlen);
            parts.push(Part::Group {
                inner: rad.formula.clone(),
                count: n,
            });
            return true;
        }
    }
    false
}

/// Used by tests and course-pack roundtrip.
pub fn parse_formula_notation(s: &str) -> Result<Formula> {
    parse_formula_str(s)
}

// ---------------------------------------------------------- coordination

use crate::coordination::{
    build_salt, build_sphere, oxidation_from, ClassProof, Coordination, MaterialClasses,
    OxidationContext, OxidationRead, Refusal, SphereKind,
};

/// What a species carried besides its formula: the evidence for how it was
/// built.
///
/// Kept beside the AST rather than inside it. The formula is chemistry and
/// takes part in equality with the corpus; this is provenance, and folding
/// it into `Species` would make every existing gold record with the same
/// formula compare unequal to a newly-built one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeciesEvidence {
    pub class_proof: Option<ClassProof>,
}

fn refused(refusal: Refusal) -> Error {
    Error::Parse {
        domain: "chemistry",
        reason: refusal.to_string(),
    }
}

/// An element named either by a Russian word or by its symbol.
///
/// The symbol form exists because the recogniser sometimes emits Latin
/// letters for a dictated element («феррит Zn»), and because a bundle that
/// understood «феррит цинка» but not «феррит Zn» would be inconsistent for
/// no reason a user could see.
pub(crate) fn element_by_word<'a>(lex: &'a Lexicon, word: &str) -> Option<&'a Element> {
    if let Some(element) = lex.elements_by_name.get(word) {
        return Some(element);
    }
    lex.elements_by_symbol
        .iter()
        .find(|(symbol, _)| symbol.to_lowercase() == word.to_lowercase())
        .map(|(_, element)| element)
}

/// The oxidation state at `i`: an explicit phrase, a bare number, or a Roman
/// numeral the recogniser produced as symbols.
///
/// Three outcomes, and they are not interchangeable. `Absent` lets the
/// caller fall back to the element's own state; `Refused` must make the
/// caller abstain, because a number *was* said and answering with a
/// different one would answer a question nobody asked.
fn oxidation_at(
    words: &[String],
    i: usize,
    nums: &NumberLex,
    coord: &Coordination,
    context: OxidationContext,
) -> OxidationRead {
    let mut cursor = i;
    let mut sign = 1;
    if let Some((marker_sign, used)) = coord.oxidation_marker(words, cursor) {
        sign = marker_sign;
        cursor += used;
    }
    if cursor >= words.len() {
        return OxidationRead::Absent;
    }
    let (raw, used) = match coord.roman(&words[cursor]) {
        Some(value) => (value, 1),
        None => match nums.consume_int(words, cursor) {
            Some((value, used)) => (value, used),
            None => return OxidationRead::Absent,
        },
    };
    // «гидроксид железа девятьсот девяносто девять тысяч» is not oxidation
    // state 999 with a stray word after it. The number lexicon only adds, so
    // a multiplier it cannot apply means the number was not read — and a
    // number that was said and not read must make the caller abstain, not
    // answer with the part that fit.
    if nums.starts_unsupported_scale(words, cursor + used) {
        return OxidationRead::Refused(Refusal::OutOfRange {
            what: "степень окисления",
            value: raw,
            limit: MAX_ATOM_COUNT,
        });
    }
    match oxidation_from(raw, sign, context) {
        OxidationRead::Found { value, .. } => OxidationRead::Found {
            value,
            used: cursor + used - i,
        },
        other => other,
    }
}

/// «гексацианоферрат три калия», «тетрагидроксоцинкат натрия».
fn try_coordination_anionic(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
) -> Option<Result<Species>> {
    let coord = Coordination::builtin();
    let decomposition = match coord.split(&words[0]) {
        Ok(decomposition) => decomposition,
        // Not a coordination name at all: stay silent so the other readings
        // get their turn. Anything else is a real refusal and is reported.
        Err(Refusal::NotCoordination) => return None,
        Err(other) => return Some(Err(refused(other))),
    };
    if decomposition.kind != SphereKind::Anionic {
        return None;
    }
    let mut i = 1;
    let center = element_by_word(lex, &decomposition.center.to_lowercase())
        .or_else(|| lex.elements_by_symbol.get(&decomposition.center))?;

    let oxidation = match oxidation_at(words, i, nums, coord, OxidationContext::Coordination) {
        OxidationRead::Found { value, used } => {
            i += used;
            value
        }
        OxidationRead::Refused(refusal) => return Some(Err(refused(refusal))),
        OxidationRead::Absent => match center.unambiguous_oxidation() {
            Some(value) => value,
            None => {
                return Some(Err(refused(Refusal::AmbiguousOxidation {
                    symbol: center.symbol.clone(),
                    states: center.oxidations.clone(),
                })))
            }
        },
    };

    // What remains must be exactly the counter-ion.
    if i >= words.len() {
        return Some(Err(refused(Refusal::UnknownCenter(
            "не назван противоион".into(),
        ))));
    }
    let Some(counter) = element_by_word(lex, &words[i]) else {
        return Some(Err(refused(Refusal::UnknownCenter(words[i].clone()))));
    };
    i += 1;
    if i != words.len() {
        return None;
    }
    let Some(counter_charge) = counter.unambiguous_oxidation() else {
        return Some(Err(refused(Refusal::AmbiguousOxidation {
            symbol: counter.symbol.clone(),
            states: counter.oxidations.clone(),
        })));
    };

    let sphere = match build_sphere(
        &center.symbol,
        oxidation,
        &decomposition.ligand,
        decomposition.multiplicity,
    ) {
        Ok(sphere) => sphere,
        Err(refusal) => return Some(Err(refused(refusal))),
    };
    match build_salt(&counter.symbol, counter_charge, sphere) {
        Ok(formula) => Some(Ok(Species::new(formula))),
        Err(refusal) => Some(Err(refused(refusal))),
    }
}

/// «ион тетрамминмеди два» — a complex cation, which has no counter-ion and
/// carries its charge on the species.
fn try_coordination_cationic(
    words: &[String],
    lex: &Lexicon,
    nums: &NumberLex,
) -> Option<Result<Species>> {
    let coord = Coordination::builtin();
    let mut i = 0;
    if coord.is_ion_lead_in(&words[i]) {
        i += 1;
    } else {
        // Without «ион» a bare complex cation is indistinguishable from
        // ordinary speech, so it is not read as one.
        return None;
    }
    if i >= words.len() {
        return None;
    }
    let (multiplicity, ligand, tail) = coord.split_cationic(&words[i])?;
    let center = element_by_word(lex, tail)?;
    if !coord
        .cationic_centers
        .iter()
        .any(|symbol| symbol == &center.symbol)
    {
        return Some(Err(refused(Refusal::UnknownCenter(center.symbol.clone()))));
    }
    i += 1;

    let oxidation = match oxidation_at(words, i, nums, coord, OxidationContext::Coordination) {
        OxidationRead::Found { value, used } => {
            i += used;
            value
        }
        OxidationRead::Refused(refusal) => return Some(Err(refused(refusal))),
        OxidationRead::Absent => match center.unambiguous_oxidation() {
            Some(value) => value,
            None => {
                return Some(Err(refused(Refusal::AmbiguousOxidation {
                    symbol: center.symbol.clone(),
                    states: center.oxidations.clone(),
                })))
            }
        },
    };
    if i != words.len() {
        return None;
    }

    let sphere = match build_sphere(&center.symbol, oxidation, &ligand, multiplicity) {
        Ok(sphere) => sphere,
        Err(refusal) => return Some(Err(refused(refusal))),
    };
    let charge = sphere.charge;
    Some(Ok(Species {
        coefficient: 1,
        formula: Formula {
            parts: vec![Part::Complex(sphere)],
        },
        charge: (charge != 0).then_some(charge),
        marker: None,
    }))
}

/// «феррит цинка», «цинковый феррит» — a material class, not a systematic
/// name. Returns the proof alongside, so a caller can say which template was
/// applied and on what grounds.
fn try_material_class(words: &[String], lex: &Lexicon) -> Option<Result<(Species, ClassProof)>> {
    if words.len() != 2 {
        return None;
    }
    let classes = MaterialClasses::builtin();
    // Either «феррит цинка» or «цинковый феррит».
    let (class_id, cation_word) = match classes.class_of(&words[0]) {
        Some(class) => (class.to_string(), words[1].as_str()),
        None => {
            let class = classes.class_of(&words[1])?;
            let symbol = classes.cation_of_adjective(class, &words[0])?;
            let class = class.to_string();
            return Some(finish_material_class(classes, lex, &class, symbol));
        }
    };
    let element = element_by_word(lex, cation_word)?;
    let symbol = element.symbol.clone();
    Some(finish_material_class(classes, lex, &class_id, &symbol))
}

fn finish_material_class(
    classes: &MaterialClasses,
    lex: &Lexicon,
    class_id: &str,
    symbol: &str,
) -> Result<(Species, ClassProof)> {
    // The state the element table can actually prove, not the raw list.
    // `None` means the element has more than one recorded state, or none at
    // all, and either way the template cannot be settled from here.
    let proven = lex
        .elements_by_symbol
        .get(symbol)
        .and_then(|element| element.unambiguous_oxidation());
    match classes.resolve(class_id, symbol, proven) {
        Ok((formula, proof)) => Ok((Species::new(formula), proof)),
        Err(refusal) => Err(refused(refusal)),
    }
}

/// The electron of a half-reaction. Its charge is −1 by definition, so
/// nothing here reads a sign: a spoken «минус» before it is the joint
/// between two terms, not an instruction about the electron.
fn try_electron(words: &[String]) -> Option<Species> {
    if words.len() != 1 || !Coordination::builtin().is_electron(&words[0]) {
        return None;
    }
    Some(Species {
        coefficient: 1,
        formula: Formula {
            parts: vec![Part::Electron],
        },
        charge: Some(-1),
        marker: None,
    })
}

/// «пентан», «пропен», «этин» — one word that names its own formula.
///
/// Placed after the substance dictionary and before the systematic path:
/// the trivial names that are *not* derivable («этилен», «ацетилен») still
/// win from the dictionary, and everything derivable comes from the rule.
fn try_organic(words: &[String]) -> Option<Result<Species>> {
    if words.len() != 1 {
        return None;
    }
    match crate::organic::Organic::builtin().parse(&words[0]) {
        Ok(hydrocarbon) => Some(Ok(Species::new(hydrocarbon.formula))),
        // Not a hydrocarbon name: stay silent so other readings get a turn.
        Err(crate::organic::Refusal::NotHydrocarbon) => None,
        // A name whose arithmetic does not work is a refusal, not silence:
        // «метин» was said, and it names nothing.
        Err(other) => Some(Err(Error::Parse {
            domain: "chemistry",
            reason: other.to_string(),
        })),
    }
}

/// A trailing «пентагидрат» and how many waters it names.
fn strip_hydrate(words: &[String]) -> Option<Result<(Vec<String>, u32)>> {
    let last = words.last()?;
    let count = Coordination::builtin().hydrate(last)?;
    Some(match count {
        Ok(count) => Ok((words[..words.len() - 1].to_vec(), count)),
        Err(refusal) => Err(refused(refusal)),
    })
}

/// Whether a comma, a colon or a dash sits between two words that would be
/// read as parts of the same substance.
///
/// The parser used to drop every one of those marks before it looked at the
/// words, because Whisper puts commas around «превращается в» and a reaction
/// connective must not be cut by one. The price was that «йод, бром»,
/// «йод: бром» and «йод - бром» became «йод бром» and were read the way
/// «натрий хлор» is — as a compound — so the list came back as IBr: a
/// substance nobody named.
///
/// The mark is harmless beside a connective («натрий хлор, превращается в,
/// …», «A, плюс B»), a reaction condition («…, при нагревании, …») or a
/// charge said about an ion («ион меди - два плюс»): the words on one side
/// of it are those phrases, not names. Anywhere else it stands between two
/// names, and those are two items. The caller refuses the whole span then;
/// the names are still read on their own, one by one.
fn separator_splits_names(words: &[String], lex: &Lexicon) -> bool {
    let mut cleaned: Vec<String> = Vec::new();
    let mut comma_before: Vec<bool> = Vec::new();
    let mut weak_before: Vec<bool> = Vec::new();
    let mut pending: Option<&str> = None;
    for word in words {
        match word.as_str() {
            "," => pending = Some(","),
            ":" | "-" => pending = pending.or(Some("-")),
            "." => {}
            _ => {
                cleaned.push(word.clone());
                comma_before.push(pending.is_some());
                weak_before.push(pending == Some("-"));
                pending = None;
            }
        }
    }
    if !comma_before.iter().any(|comma| *comma) {
        return false;
    }
    // A comma separates whole items: «аш два о, це о два» is two formulas.
    // A colon or a dash is weaker: a recogniser writes one at a pause, and
    // a pause inside a spelled formula («це о - два», «эн а - хлор», «аш два
    // - о два») is still one formula. So a colon or a dash separates only two
    // *names*: when the word on either side is a spelled letter or a number,
    // it is not a boundary.
    let nums_for_spelling = NumberLex::new();
    let spelled = |word: &str| {
        lex.latin(word).is_some()
            || spelling_evidence(lex, word)
            || FUNCTION_WORD_LETTERS.contains(&word)
            || nums_for_spelling
                .consume_int(std::slice::from_ref(&word.to_string()), 0)
                .is_some()
    };
    for index in 1..cleaned.len() {
        if weak_before[index] && (spelled(&cleaned[index - 1]) || spelled(&cleaned[index])) {
            comma_before[index] = false;
        }
    }
    if !comma_before.iter().any(|comma| *comma) {
        return false;
    }
    let speech = &lex.chemistry_speech;
    // Words that belong to a connective or a condition, not to a name.
    let mut phrase = vec![false; cleaned.len()];
    for index in 0..cleaned.len() {
        let used = speech
            .connective_at(&cleaned, index)
            .map(|(_, used)| used)
            .into_iter()
            .chain(speech.condition_at(&cleaned, index).map(|(_, used)| used))
            .max();
        if let Some(used) = used {
            for flag in phrase.iter_mut().skip(index).take(used) {
                *flag = true;
            }
        }
    }
    // A hydrate marker and a charge are said *about* a name, they are not
    // another name: «сульфат меди, пентагидрат», «ион меди, два плюс». The
    // charge needs an ion marker earlier in the words, so «йод, два плюс»
    // is still two items.
    let nums = NumberLex::new();
    let mut after_ion_marker = false;
    for index in 0..cleaned.len() {
        after_ion_marker |= index > 0 && speech.is_ion_marker(&cleaned[index - 1]);
        if Coordination::builtin().hydrate(&cleaned[index]).is_some() {
            phrase[index] = true;
        }
        if after_ion_marker
            && nums.consume_int(&cleaned, index).is_some_and(|(_, used)| {
                matches!(
                    cleaned.get(index + used).map(String::as_str),
                    Some("плюс" | "минус")
                )
            })
        {
            phrase[index] = true;
        }
    }
    (1..cleaned.len()).any(|index| comma_before[index] && !phrase[index - 1] && !phrase[index])
}

/// The words the chemistry parser reads: punctuation that Whisper scatters
/// around connectives is dropped. Shared with the research token classes.
pub(crate) fn spoken_words(words: &[String]) -> Vec<String> {
    words
        .iter()
        .filter(|w| !matches!(w.as_str(), "," | ":" | "-" | "."))
        .cloned()
        .collect()
}

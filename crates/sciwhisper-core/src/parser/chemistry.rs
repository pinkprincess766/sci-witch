use crate::ast::{Arrow, Chemical, Equation, Formula, Node, Part, Species, StateMarker};
use crate::error::{Error, Result};
use crate::formula::{ionic_compound, parse_formula_str};
use crate::lexicon::{AnionClass, ChemConnective, Element, IonRole, Lexicon};
use crate::numbers::NumberLex;

pub fn parse_chemistry(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Result<Node> {
    // Whisper inserts commas around «превращается в».
    let cleaned: Vec<String> = words
        .iter()
        .filter(|w| *w != "," && *w != "-" && *w != ".")
        .cloned()
        .collect();
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
            let plus_is_a_charge = connective == ChemConnective::Plus
                && current.words.iter().any(|word| speech.is_ion_marker(word))
                && !current
                    .words
                    .last()
                    .is_some_and(|word| word == "плюс" || word == "минус");
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
fn strip_conditions(words: &[String], lex: &Lexicon) -> (Vec<String>, Option<String>) {
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
    if words.is_empty() {
        return Err(Error::Parse {
            domain: "chemistry",
            reason: "empty species".into(),
        });
    }

    let mut words = words.to_vec();
    let mut coefficient = 1u32;
    if let Some((n, used)) = nums.consume_int(&words, 0) {
        if used < words.len() && n > 0 {
            coefficient = n;
            words = words[used..].to_vec();
        }
    }

    // Substance names like «углекислый газ» must win before the trailing
    // «газ» marker is stripped.
    if let Some(mut s) = try_full_substance(&words, lex) {
        s.coefficient = coefficient;
        return Ok(s);
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
        return Ok(s);
    }
    if let Some(mut s) = try_systematic(&words, lex, nums) {
        s.coefficient = coefficient;
        s.marker = marker;
        return Ok(s);
    }
    if let Some(mut s) = try_spelled(&words, lex, nums) {
        s.coefficient = coefficient;
        s.marker = marker;
        return Ok(s);
    }
    if let Some(el) = lex.element(&words[0]) {
        if words.len() == 1 {
            let formula = if el.diatomic {
                Formula::atom(&el.symbol, 2)
            } else {
                Formula::atom(&el.symbol, 1)
            };
            return Ok(Species {
                coefficient,
                formula,
                charge: None,
                marker,
            });
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

    let mut mag: Option<i32> = None;
    if let Some((n, used)) = nums.consume_int(&filtered, filtered.len().saturating_sub(1)) {
        if used == 1 {
            mag = Some(n as i32);
            filtered.pop();
        }
    }
    // "два плюс" already handled; also "два" may remain as ox on metal

    // anion class first: сульфат [ион] [два] минус
    if let Some(an) = filtered.first().and_then(|w| lex.anion(w)) {
        if an.role == IonRole::Anion && filtered.len() == 1 {
            let mut ch = mag.unwrap_or(an.charge.abs());
            if let Some(s) = charge_sign {
                ch *= s;
            } else {
                ch = an.charge;
            }
            return Some(Species {
                coefficient: 1,
                formula: an.formula.clone(),
                charge: Some(ch),
                marker: None,
            });
        }
    }

    // metal ion: меди [два]
    if let Some(el) = filtered.first().and_then(|w| lex.element(w)) {
        let ox = if filtered.len() >= 2 {
            nums.consume_int(&filtered, 1).map(|(n, _)| n as i32)
        } else {
            None
        };
        let mag = mag.or(ox).unwrap_or(1);
        let mut ch = mag;
        if let Some(s) = charge_sign {
            ch *= s;
        }
        return Some(Species {
            coefficient: 1,
            formula: Formula::atom(&el.symbol, 1),
            charge: Some(ch),
            marker: None,
        });
    }

    if let Some(an) = filtered.first().and_then(|w| lex.anion(w)) {
        let mut ch = mag.unwrap_or(an.charge.abs());
        if let Some(s) = charge_sign {
            ch *= s;
        } else {
            ch = an.charge;
        }
        return Some(Species {
            coefficient: 1,
            formula: an.formula.clone(),
            charge: Some(ch),
            marker: None,
        });
    }
    None
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
            if let Some((n, used)) = nums.consume_int(words, i) {
                ox = n as i32;
                i += used;
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

    let formula = ionic_compound(
        cation_f,
        cat_charge,
        an.formula.clone(),
        an.charge,
        an.group,
        cat_poly,
    );
    Some(Species::new(formula))
}

fn try_spelled(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Option<Species> {
    let mut parts: Vec<Part> = Vec::new();
    let mut i = 0;
    let mut saw_element = false;
    while i < words.len() {
        if let Some((count, used)) = lex.chemistry_speech.grouping_at(words, i) {
            apply_times(&mut parts, count, lex);
            i += used;
            continue;
        }
        if let Some((el, used_el)) = chemistry_element_at(lex, words, i) {
            i += used_el;
            let mut count = 1u32;
            if i < words.len() {
                if let Some((n, used)) = nums.consume_int(words, i) {
                    count = n.max(1);
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

fn chemistry_element_at<'a>(
    lex: &'a Lexicon,
    words: &[String],
    i: usize,
) -> Option<(&'a Element, usize)> {
    if i + 1 < words.len() {
        let pair = match (words[i].as_str(), words[i + 1].as_str()) {
            ("эн", "а") => Some("Na"),
            ("цэ", "а") => Some("Ca"),
            ("цэ", "эль") => Some("Cl"),
            ("эм", "гэ") | ("эм", "г") => Some("Mg"),
            ("а", "эль") => Some("Al"),
            ("цэ", "у") => Some("Cu"),
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
    chemistry_element(lex, &words[i]).map(|el| (el, 1))
}

fn chemistry_element<'a>(lex: &'a Lexicon, word: &str) -> Option<&'a Element> {
    if let Some(el) = lex.element(word) {
        return Some(el);
    }
    // single spoken latin letters used as element symbols in spelled formulas: о, эс, аш, цэ, эн
    if let Some(ch) = lex.latin(word) {
        let sym = ch.to_ascii_uppercase().to_string();
        return lex.elements_by_symbol.get(&sym);
    }
    None
}

fn apply_times(parts: &mut Vec<Part>, n: u32, lex: &Lexicon) {
    if parts.is_empty() {
        return;
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
            return;
        }
    }
    // fallback: last atom becomes a grouped unit
    if let Some(Part::Atom { symbol, count }) = parts.pop() {
        let inner = Formula::atom(symbol, count);
        parts.push(Part::Group { inner, count: n });
    }
}

/// Used by tests and course-pack roundtrip.
pub fn parse_formula_notation(s: &str) -> Result<Formula> {
    parse_formula_str(s)
}

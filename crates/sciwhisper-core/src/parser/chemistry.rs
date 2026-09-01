use crate::ast::{Arrow, Chemical, Equation, Formula, Node, Part, Species, StateMarker};
use crate::error::{Error, Result};
use crate::formula::{ionic_compound, parse_formula_str};
use crate::lexicon::{AnionClass, Element, IonRole, Lexicon};
use crate::numbers::NumberLex;

type ReactionSplit = (Vec<String>, Arrow, Vec<String>, Option<String>);

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
    if let Some((left, arrow, right, cond)) = split_reaction(words) {
        let left = parse_side(&left, lex, nums)?;
        let right = parse_side(&right, lex, nums)?;
        return Ok(Node::Chemical(Chemical::Equation(Equation {
            left,
            arrow,
            right,
            condition: cond,
        })));
    }
    let species = parse_species(words, lex, nums)?;
    Ok(Node::Chemical(Chemical::Species(species)))
}

fn split_reaction(words: &[String]) -> Option<ReactionSplit> {
    let arrows: &[(&[&str], Arrow)] = &[
        (&["равновесная", "стрелка"], Arrow::Equilibrium),
        (&["превращается", "в"], Arrow::Forward),
        (&["превращаются", "в"], Arrow::Forward),
        (&["переходит", "в"], Arrow::Forward),
        (&["переходят", "в"], Arrow::Forward),
        (&["окисляется", "до"], Arrow::Forward),
        (&["окисляются", "до"], Arrow::Forward),
        (&["восстанавливается", "до"], Arrow::Forward),
        (&["восстанавливаются", "до"], Arrow::Forward),
        (&["превращается"], Arrow::Forward),
        (&["превращаются"], Arrow::Forward),
        (&["дает"], Arrow::Forward),
        (&["стрелка"], Arrow::Forward),
        (&["обратимо"], Arrow::Equilibrium),
    ];
    for i in 0..words.len() {
        for (ph, arrow) in arrows {
            if match_at(words, i, ph) {
                let left = words[..i].to_vec();
                let right = words[i + ph.len()..].to_vec();
                if left.is_empty() || right.is_empty() {
                    continue;
                }
                let (left, c1) = strip_conditions(&left);
                let (right, c2) = strip_conditions(&right);
                let cond = c1.or(c2);
                return Some((left, *arrow, right, cond));
            }
        }
    }
    None
}

fn strip_conditions(words: &[String]) -> (Vec<String>, Option<String>) {
    let mut out = Vec::new();
    let mut cond = None;
    let mut i = 0;
    while i < words.len() {
        if match_at(words, i, &["при", "нагревании"]) || match_at(words, i, &["при", "нагреве"])
        {
            cond = Some("heat".into());
            i += 2;
            continue;
        }
        if words[i] == "нагревание" {
            cond = Some("heat".into());
            i += 1;
            continue;
        }
        out.push(words[i].clone());
        i += 1;
    }
    (out, cond)
}

fn parse_side(words: &[String], lex: &Lexicon, nums: &NumberLex) -> Result<Vec<Species>> {
    let chunks = split_plus(words);
    chunks
        .into_iter()
        .map(|c| parse_species(&c, lex, nums))
        .collect()
}

fn split_plus(words: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for w in words {
        if w == "плюс" {
            if buf.iter().any(|x: &String| x == "ион") || buf.is_empty() {
                buf.push(w.clone());
            } else {
                out.push(std::mem::take(&mut buf));
            }
        } else {
            buf.push(w.clone());
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
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

    let (words, marker) = strip_marker(&words);
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

fn strip_marker(words: &[String]) -> (Vec<String>, Option<StateMarker>) {
    if words.is_empty() {
        return (vec![], None);
    }
    let last = words.last().unwrap().as_str();
    if last == "газ" {
        return (words[..words.len() - 1].to_vec(), Some(StateMarker::Gas));
    }
    if last == "осадок" {
        return (
            words[..words.len() - 1].to_vec(),
            Some(StateMarker::Precipitate),
        );
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
    if !words.iter().any(|w| w == "ион") {
        return None;
    }
    let mut filtered: Vec<String> = words.iter().filter(|w| *w != "ион").cloned().collect();
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
        if words[i] == "дважды" {
            apply_times(&mut parts, 2, lex);
            i += 1;
            continue;
        }
        if words[i] == "трижды" {
            apply_times(&mut parts, 3, lex);
            i += 1;
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

fn match_at(words: &[String], i: usize, phrase: &[&str]) -> bool {
    if i + phrase.len() > words.len() {
        return false;
    }
    phrase.iter().enumerate().all(|(k, w)| words[i + k] == *w)
}

/// Used by tests and course-pack roundtrip.
pub fn parse_formula_notation(s: &str) -> Result<Formula> {
    parse_formula_str(s)
}

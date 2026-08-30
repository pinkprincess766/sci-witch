//! Character-level chemical formula parser for dictionaries and course packs.
//! Accepts Hill-style strings: `Cu(OH)2`, `CuSO4·5H2O`, `SO4^2-`, `2HCl`.

use crate::ast::{Formula, Part, Species, StateMarker};
use crate::error::{Error, Result};

pub fn parse_species_str(input: &str) -> Result<Species> {
    let s = input.trim();
    if s.is_empty() {
        return Err(Error::InvalidFormula(input.into()));
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut coefficient = 1u32;
    let rest;
    if i > 0 && i < bytes.len() {
        coefficient = s[..i]
            .parse()
            .map_err(|_| Error::InvalidFormula(input.into()))?;
        rest = &s[i..];
    } else {
        rest = s;
    }

    let (body, charge, marker) = split_charge_and_marker(rest)?;
    let formula = parse_formula_str(body)?;
    Ok(Species {
        coefficient,
        formula,
        charge,
        marker,
    })
}

pub fn parse_formula_str(input: &str) -> Result<Formula> {
    let s = input.trim();
    if s.is_empty() {
        return Err(Error::InvalidFormula(input.into()));
    }
    let chars: Vec<char> = s.chars().collect();
    let (formula, pos) = parse_seq(&chars, 0, false)?;
    if pos != chars.len() {
        return Err(Error::InvalidFormula(input.into()));
    }
    Ok(formula)
}

fn split_charge_and_marker(s: &str) -> Result<(&str, Option<i32>, Option<StateMarker>)> {
    let mut t = s.trim();
    let mut marker = None;
    if let Some(stripped) = t.strip_suffix('↑').or_else(|| t.strip_suffix('^')) {
        marker = Some(StateMarker::Gas);
        t = stripped.trim_end();
    } else if let Some(stripped) = t.strip_suffix('↓').or_else(|| t.strip_suffix('v')) {
        marker = Some(StateMarker::Precipitate);
        t = stripped.trim_end();
    }

    if let Some(idx) = t.rfind('^') {
        let (body, ch) = t.split_at(idx);
        let charge = parse_charge(&ch[1..])?;
        return Ok((body, charge, marker));
    }
    Ok((t, None, marker))
}

fn parse_charge(s: &str) -> Result<Option<i32>> {
    let s = s.trim().trim_matches(|c| c == '{' || c == '}');
    if s.is_empty() {
        return Ok(None);
    }
    if s == "+" {
        return Ok(Some(1));
    }
    if s == "-" {
        return Ok(Some(-1));
    }
    let (sign, digits) = if let Some(d) = s.strip_suffix('+') {
        (1, d)
    } else if let Some(d) = s.strip_suffix('-') {
        (-1, d)
    } else if let Some(d) = s.strip_prefix('+') {
        (1, d)
    } else if let Some(d) = s.strip_prefix('-') {
        (-1, d)
    } else {
        return Err(Error::InvalidFormula(s.into()));
    };
    let n: i32 = if digits.is_empty() {
        1
    } else {
        digits
            .parse()
            .map_err(|_| Error::InvalidFormula(s.into()))?
    };
    Ok(Some(sign * n))
}

fn parse_seq(chars: &[char], mut i: usize, in_group: bool) -> Result<(Formula, usize)> {
    let mut parts: Vec<Part> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c == ')' {
            if !in_group {
                return Err(Error::InvalidFormula("unmatched )".into()));
            }
            break;
        }
        if c == '(' {
            i += 1;
            let (inner, ni) = parse_seq(chars, i, true)?;
            i = ni;
            if i >= chars.len() || chars[i] != ')' {
                return Err(Error::InvalidFormula("unclosed group".into()));
            }
            i += 1;
            let (count, ni) = parse_count(chars, i);
            i = ni;
            parts.push(Part::Group {
                inner,
                count: count.max(1),
            });
            continue;
        }
        if c == '·' || c == '*' || c == '.' {
            // hydrate ·nH2O / *nH2O / .nH2O
            let rest: String = chars[i + 1..].iter().collect();
            if let Some(n) = parse_hydrate(&rest) {
                parts.push(Part::Hydrate { count: n });
                return Ok((Formula { parts }, chars.len()));
            }
            if c == '.' {
                // not a hydrate; treat as error rather than skip
                return Err(Error::InvalidFormula("unexpected '.'".into()));
            }
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_uppercase() {
            let mut symbol = String::new();
            symbol.push(c);
            i += 1;
            while i < chars.len() && chars[i].is_ascii_lowercase() {
                symbol.push(chars[i]);
                i += 1;
            }
            let (count, ni) = parse_count(chars, i);
            i = ni;
            parts.push(Part::Atom {
                symbol,
                count: count.max(1),
            });
            continue;
        }
        return Err(Error::InvalidFormula(format!("unexpected '{c}'")));
    }
    Ok((Formula { parts }, i))
}

fn parse_count(chars: &[char], mut i: usize) -> (u32, usize) {
    if i >= chars.len() || !chars[i].is_ascii_digit() {
        return (1, i);
    }
    let mut n = 0u32;
    while i < chars.len() && chars[i].is_ascii_digit() {
        n = n * 10 + (chars[i] as u32 - '0' as u32);
        i += 1;
    }
    (n.max(1), i)
}

fn parse_hydrate(rest: &str) -> Option<u32> {
    let r = rest.trim();
    let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    let after = &r[digits.len()..];
    let n: u32 = if digits.is_empty() {
        1
    } else {
        digits.parse().ok()?
    };
    let after = after.trim_start_matches(['·', '*']);
    if after.eq_ignore_ascii_case("H2O") || after.eq_ignore_ascii_case("H₂O") {
        Some(n)
    } else {
        None
    }
}

pub fn parse_equation_str(input: &str) -> Result<crate::ast::Equation> {
    let s = input.trim();
    let (left_s, arrow, right_s) = if let Some((l, r)) = s.split_once("<=>") {
        (l, crate::ast::Arrow::Equilibrium, r)
    } else if let Some((l, r)) = s.split_once("<->") {
        (l, crate::ast::Arrow::Equilibrium, r)
    } else if let Some((l, r)) = s.split_once("->") {
        (l, crate::ast::Arrow::Forward, r)
    } else if let Some((l, r)) = s.split_once('→') {
        (l, crate::ast::Arrow::Forward, r)
    } else if let Some((l, r)) = s.split_once('⇌') {
        (l, crate::ast::Arrow::Equilibrium, r)
    } else {
        return Err(Error::InvalidFormula(input.into()));
    };
    let left = split_terms(left_s)?;
    let right = split_terms(right_s)?;
    Ok(crate::ast::Equation {
        left,
        arrow,
        right,
        condition: None,
    })
}

fn split_terms(s: &str) -> Result<Vec<Species>> {
    s.split('+')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(parse_species_str)
        .collect()
}

pub fn gcd_i32(mut a: i32, mut b: i32) -> i32 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

/// Build an ionic compound from cation/anion formulas and charges.
pub fn ionic_compound(
    cation: Formula,
    cat_charge: i32,
    anion: Formula,
    an_charge: i32,
    anion_poly: bool,
    cation_poly: bool,
) -> Formula {
    let g = gcd_i32(cat_charge, an_charge);
    let n_cat = (an_charge.abs() / g) as u32;
    let n_an = (cat_charge.abs() / g) as u32;
    let mut parts = Vec::new();
    push_scaled(&mut parts, cation, n_cat, cation_poly);
    push_scaled(&mut parts, anion, n_an, anion_poly);
    Formula { parts }
}

fn push_scaled(out: &mut Vec<Part>, f: Formula, n: u32, poly: bool) {
    if n == 0 {
        return;
    }
    if poly && n > 1 {
        out.push(Part::Group { inner: f, count: n });
        return;
    }
    if n == 1 {
        out.extend(f.parts);
        return;
    }
    if f.parts.len() == 1 {
        match &f.parts[0] {
            Part::Atom { symbol, count } => {
                out.push(Part::Atom {
                    symbol: symbol.clone(),
                    count: count * n,
                });
                return;
            }
            Part::Group { inner, count } => {
                out.push(Part::Group {
                    inner: inner.clone(),
                    count: count * n,
                });
                return;
            }
            Part::Hydrate { count } => {
                out.push(Part::Hydrate { count: count * n });
                return;
            }
        }
    }
    out.push(Part::Group { inner: f, count: n });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copper_hydroxide() {
        let f = parse_formula_str("Cu(OH)2").unwrap();
        assert_eq!(f.parts.len(), 2);
    }

    #[test]
    fn parses_hydrate() {
        let f = parse_formula_str("CuSO4·5H2O").unwrap();
        assert!(matches!(f.parts.last(), Some(Part::Hydrate { count: 5 })));
    }

    #[test]
    fn parses_charged_species() {
        let s = parse_species_str("Cu^2+").unwrap();
        assert_eq!(s.charge, Some(2));
        assert_eq!(s.formula.parts.len(), 1);
    }
}

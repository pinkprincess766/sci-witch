//! Saying the same quantity a second way.
//!
//! `1 км` and `1000 м` are the same measurement. Which one belongs in the
//! document is the speaker's business, so nothing here changes what was
//! dictated: the equivalent is offered as a warning, exactly as an
//! unbalanced equation is offered a set of coefficients and left alone.
//!
//! # Only when the other form is easier to read
//!
//! The conversion runs in one direction: from a unit that is **larger**
//! than its base towards the base. `1 км → 1000 м` helps; `300 нм →
//! 0.0000003 м` does not, and offering it would be noise dressed as help.
//! So a unit with a negative scale exponent produces no suggestion at all.
//!
//! That rule also buys exactness for free. Scaling up by a power of ten is
//! appending zeros to a decimal string — no floating point, no rounding, no
//! `1e3 * 1e3 != 1e6` surprises.

use crate::lexicon::{Gender, Lexicon, NamedUnit};

/// Largest number this module will spell out.
///
/// Beyond a billion the Russian forms are rare enough that a match is more
/// likely a misrecognition than a quantity, and an unspelled suggestion is
/// better than a wrong one.
pub const MAX_SPELLED: u64 = 999_999_999;

/// The same quantity written the other way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equivalent {
    /// `1000 м`
    pub compact: String,
    /// `тысяча метров`, or `None` when the number is past [`MAX_SPELLED`].
    pub spelled: Option<String>,
}

impl Equivalent {
    /// What the warning says: `1000 м (тысяча метров)`.
    pub fn to_message(&self) -> String {
        match &self.spelled {
            Some(words) => format!("{} ({words})", self.compact),
            None => self.compact.clone(),
        }
    }
}

/// Multiplies a decimal written as text by ten to the `exponent`.
///
/// Text in, text out: `"1"` and 3 give `"1000"`, `"2.5"` and 3 give
/// `"2500"`. `None` when the input is not a plain decimal or the result
/// would not be a whole number, because a suggestion with a fraction in it
/// is not the easier reading this module exists to offer.
pub fn scale_up(value: &str, exponent: u32) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (value, ""),
    };
    if whole.is_empty() || fraction.contains('.') {
        return None;
    }
    let shift = exponent as usize;
    if fraction.len() > shift {
        // Shifting would leave a fractional part.
        return None;
    }
    let mut out = String::with_capacity(whole.len() + shift);
    out.push_str(whole);
    out.push_str(fraction);
    for _ in 0..(shift - fraction.len()) {
        out.push('0');
    }
    let trimmed = out.trim_start_matches('0');
    Some(if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    })
}

/// Which of the three forms a Russian count takes.
///
/// 1, 21, 31 take the first; 2–4, 22–24 the second; everything else the
/// third — with 11–14 an exception that takes the third despite ending in
/// 1–4.
pub fn agreement(count: u64) -> Form {
    if (11..=14).contains(&(count % 100)) {
        return Form::Many;
    }
    match count % 10 {
        1 => Form::One,
        2..=4 => Form::Few,
        _ => Form::Many,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    One,
    Few,
    Many,
}

const ONES_M: [&str; 20] = [
    "ноль",
    "один",
    "два",
    "три",
    "четыре",
    "пять",
    "шесть",
    "семь",
    "восемь",
    "девять",
    "десять",
    "одиннадцать",
    "двенадцать",
    "тринадцать",
    "четырнадцать",
    "пятнадцать",
    "шестнадцать",
    "семнадцать",
    "восемнадцать",
    "девятнадцать",
];
const TENS: [&str; 10] = [
    "",
    "",
    "двадцать",
    "тридцать",
    "сорок",
    "пятьдесят",
    "шестьдесят",
    "семьдесят",
    "восемьдесят",
    "девяносто",
];
const HUNDREDS: [&str; 10] = [
    "",
    "сто",
    "двести",
    "триста",
    "четыреста",
    "пятьсот",
    "шестьсот",
    "семьсот",
    "восемьсот",
    "девятьсот",
];

/// One group of three digits, in the gender the group requires.
fn group_words(value: u64, gender: Gender, out: &mut Vec<String>) {
    let hundreds = (value / 100) as usize;
    if hundreds > 0 {
        out.push(HUNDREDS[hundreds].to_string());
    }
    let rest = value % 100;
    if rest >= 20 {
        out.push(TENS[(rest / 10) as usize].to_string());
        let ones = rest % 10;
        if ones > 0 {
            out.push(one_word(ones, gender));
        }
    } else if rest > 0 {
        out.push(one_word(rest, gender));
    }
}

/// Only 1 and 2 change with gender: «один метр» but «одна секунда», «два
/// метра» but «две секунды».
fn one_word(value: u64, gender: Gender) -> String {
    match (value, gender) {
        (1, Gender::Feminine) => "одна".into(),
        (2, Gender::Feminine) => "две".into(),
        _ => ONES_M[value as usize].to_string(),
    }
}

/// Spells a whole number in Russian, agreeing with `gender` in the last
/// group. `None` past [`MAX_SPELLED`].
pub fn spell(count: u64, gender: Gender) -> Option<String> {
    if count > MAX_SPELLED {
        return None;
    }
    if count == 0 {
        return Some("ноль".into());
    }
    let mut words: Vec<String> = Vec::new();
    let millions = count / 1_000_000;
    let thousands = (count / 1_000) % 1_000;
    let units = count % 1_000;

    if millions > 0 {
        group_words(millions, Gender::Masculine, &mut words);
        words.push(
            match agreement(millions) {
                Form::One => "миллион",
                Form::Few => "миллиона",
                Form::Many => "миллионов",
            }
            .into(),
        );
    }
    if thousands > 0 {
        // «тысяча» is feminine, so «две тысячи» — but a bare one is
        // dropped: Russian says «тысяча метров», not «одна тысяча метров».
        // The million keeps its «один», which is how it is actually said.
        if thousands != 1 {
            group_words(thousands, Gender::Feminine, &mut words);
        }
        words.push(
            match agreement(thousands) {
                Form::One => "тысяча",
                Form::Few => "тысячи",
                Form::Many => "тысяч",
            }
            .into(),
        );
    }
    if units > 0 {
        group_words(units, gender, &mut words);
    }
    Some(words.join(" "))
}

/// The equivalent of `value` `symbol` in the base unit, if offering one
/// would help.
pub fn equivalent(value: &str, symbol: &str, lex: &Lexicon) -> Option<Equivalent> {
    let unit = lex.units.iter().find(|unit| unit.symbol == symbol)?;
    let scale = unit.scale.as_ref()?;
    // Only towards the base, and only from a larger unit.
    let exponent = u32::try_from(scale.exponent).ok()?;
    if exponent == 0 {
        return None;
    }
    let base: &NamedUnit = lex.units.iter().find(|unit| unit.symbol == scale.base)?;
    let scaled = scale_up(value, exponent)?;
    let compact = format!("{scaled} {}", base.symbol);
    let spelled = scaled.parse::<u64>().ok().and_then(|count| {
        let number = spell(count, base.gender)?;
        let word = match agreement(count) {
            Form::One => &base.forms.one,
            Form::Few => &base.forms.few,
            Form::Many => &base.forms.many,
        };
        Some(format!("{number} {word}"))
    });
    Some(Equivalent { compact, spelled })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> &'static Lexicon {
        Lexicon::builtin()
    }

    #[test]
    fn scaling_up_is_decimal_text_not_floating_point() {
        assert_eq!(scale_up("1", 3).as_deref(), Some("1000"));
        assert_eq!(scale_up("2", 6).as_deref(), Some("2000000"));
        assert_eq!(scale_up("2.5", 3).as_deref(), Some("2500"));
        assert_eq!(scale_up("0.001", 3).as_deref(), Some("1"));
        // A shift that would leave a fraction is not the easier reading.
        assert_eq!(scale_up("2.5", 0), None);
        assert_eq!(scale_up("0.0001", 3), None);
        assert_eq!(scale_up("", 3), None);
        assert_eq!(scale_up("abc", 3), None);
    }

    #[test]
    fn russian_agreement_follows_the_teens_exception() {
        assert_eq!(agreement(1), Form::One);
        assert_eq!(agreement(21), Form::One);
        assert_eq!(agreement(2), Form::Few);
        assert_eq!(agreement(23), Form::Few);
        assert_eq!(agreement(5), Form::Many);
        // 11..14 take the many form despite ending in 1..4.
        for n in 11..=14 {
            assert_eq!(agreement(n), Form::Many, "{n}");
        }
        assert_eq!(agreement(111), Form::Many);
        assert_eq!(agreement(121), Form::One);
    }

    #[test]
    fn numbers_are_spelled_with_the_right_gender() {
        let m = Gender::Masculine;
        let f = Gender::Feminine;
        assert_eq!(spell(1000, m).as_deref(), Some("тысяча"));
        assert_eq!(spell(2000, m).as_deref(), Some("две тысячи"));
        assert_eq!(spell(1, m).as_deref(), Some("один"));
        assert_eq!(spell(1, f).as_deref(), Some("одна"));
        assert_eq!(spell(2, f).as_deref(), Some("две"));
        assert_eq!(spell(21, f).as_deref(), Some("двадцать одна"));
        assert_eq!(spell(345, m).as_deref(), Some("триста сорок пять"));
        assert_eq!(spell(1_000_000, m).as_deref(), Some("один миллион"));
        assert_eq!(
            spell(2_500_000, m).as_deref(),
            Some("два миллиона пятьсот тысяч")
        );
        assert_eq!(spell(0, m).as_deref(), Some("ноль"));
        assert_eq!(spell(MAX_SPELLED + 1, m), None);
    }

    #[test]
    fn a_larger_unit_offers_its_base_equivalent() {
        let one_km = equivalent("1", "км", lex()).expect("километр has a base");
        assert_eq!(one_km.compact, "1000 м");
        assert_eq!(one_km.spelled.as_deref(), Some("тысяча метров"));
        assert_eq!(one_km.to_message(), "1000 м (тысяча метров)");

        let two_khz = equivalent("2", "кГц", lex()).expect("килогерц has a base");
        assert_eq!(two_khz.to_message(), "2000 Гц (две тысячи герц)");
    }

    /// The direction that would make the number worse produces nothing.
    #[test]
    fn a_smaller_unit_offers_nothing() {
        assert_eq!(equivalent("300", "нм", lex()), None);
        assert_eq!(equivalent("100", "мс", lex()), None);
        assert_eq!(equivalent("5", "мм", lex()), None);
    }

    /// A base unit is already the simplest form of itself.
    #[test]
    fn a_base_unit_offers_nothing() {
        assert_eq!(equivalent("5", "м", lex()), None);
        assert_eq!(equivalent("3", "с", lex()), None);
        assert_eq!(equivalent("7", "Дж", lex()), None);
    }

    #[test]
    fn an_unknown_unit_offers_nothing() {
        assert_eq!(equivalent("1", "фунт", lex()), None);
    }

    /// The unit word agrees with the converted number, not the spoken one.
    #[test]
    fn the_unit_word_agrees_with_the_number_it_follows() {
        // 1 км is 1000 m — «метров», not «метр».
        assert_eq!(
            equivalent("1", "км", lex()).unwrap().spelled.as_deref(),
            Some("тысяча метров")
        );
        // 0.002 km is 2 m — «два метра».
        assert_eq!(
            equivalent("0.002", "км", lex()).unwrap().spelled.as_deref(),
            Some("два метра")
        );
        // 0.001 km is 1 m — «один метр».
        assert_eq!(
            equivalent("0.001", "км", lex()).unwrap().spelled.as_deref(),
            Some("один метр")
        );
    }
}

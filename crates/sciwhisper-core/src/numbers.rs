//! Spoken Russian integers and school-style decimals ("девять целых восемьдесят одна").

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;

const SPOKEN_NUMBERS_YAML: &str = include_str!("../data/domains/common/spoken_numbers.yaml");
const SUPPORTED_SCHEMA: u32 = 1;

#[derive(Clone, Debug)]
pub struct NumberLex {
    words: HashMap<String, u32>,
    ordinals: HashMap<String, u32>,
    decimal_markers: HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct NumberLexYaml {
    schema_version: u32,
    digits: HashMap<u32, Vec<String>>,
    teens: HashMap<u32, Vec<String>>,
    tens: HashMap<u32, Vec<String>>,
    hundreds: HashMap<u32, Vec<String>>,
    ordinals: HashMap<u32, Vec<String>>,
    markers: NumberMarkersYaml,
}

#[derive(Debug, Deserialize)]
struct NumberMarkersYaml {
    decimal: Vec<String>,
}

impl Default for NumberLex {
    fn default() -> Self {
        Self::new()
    }
}

impl NumberLex {
    pub fn new() -> Self {
        static BUILTIN: OnceLock<NumberLex> = OnceLock::new();
        BUILTIN.get_or_init(load_builtin).clone()
    }

    pub fn lookup(&self, w: &str) -> Option<u32> {
        self.words.get(w).copied()
    }

    pub fn ordinal(&self, w: &str) -> Option<u32> {
        self.ordinals.get(w).copied()
    }

    /// Consume a (possibly multi-word) integer starting at `i`.
    pub fn consume_int(&self, words: &[String], i: usize) -> Option<(u32, usize)> {
        if i >= words.len() {
            return None;
        }
        if let Ok(n) = words[i].parse::<u32>() {
            return Some((n, 1));
        }
        let mut value = 0u32;
        let mut used = 0usize;
        let mut last_class = Class::None;
        while i + used < words.len() {
            let w = words[i + used].as_str();
            let Some(n) = self.lookup(w) else {
                break;
            };
            let class = classify(n);
            if !compatible(last_class, class) {
                break;
            }
            value += n;
            used += 1;
            last_class = class;
            if used > 6 {
                break;
            }
        }
        if used == 0 {
            None
        } else {
            Some((value, used))
        }
    }

    /// Integer or school decimal: `девять целых восемьдесят одна` → `"9,81"`.
    pub fn consume_number(&self, words: &[String], i: usize) -> Option<(String, usize)> {
        if i >= words.len() {
            return None;
        }
        // arabic with comma or dot
        if looks_arabic(&words[i]) {
            let s = words[i].replace('.', ",");
            return Some((s, 1));
        }
        let (int_part, used) = self.consume_int(words, i)?;
        let mut j = i + used;
        if j < words.len() && self.decimal_markers.contains(&words[j]) {
            j += 1;
            if let Some((frac, f_used)) = self.consume_int(words, j) {
                j += f_used;
                return Some((format!("{int_part},{frac}"), j - i));
            }
        }
        Some((int_part.to_string(), j - i))
    }
}

fn load_builtin() -> NumberLex {
    let parsed: NumberLexYaml =
        serde_yaml::from_str(SPOKEN_NUMBERS_YAML).expect("embedded number lexicon must be valid");
    assert!(
        parsed.schema_version <= SUPPORTED_SCHEMA,
        "embedded number lexicon schema is unsupported"
    );

    let mut words = HashMap::new();
    for table in [parsed.digits, parsed.teens, parsed.tens, parsed.hundreds] {
        for (value, names) in table {
            insert_names(&mut words, value, names);
        }
    }
    let mut ordinals = HashMap::new();
    for (value, names) in parsed.ordinals {
        insert_names(&mut ordinals, value, names);
    }
    let decimal_markers = parsed
        .markers
        .decimal
        .into_iter()
        .map(|name| crate::normalize::normalize_word(&name))
        .collect();
    NumberLex {
        words,
        ordinals,
        decimal_markers,
    }
}

fn insert_names(target: &mut HashMap<String, u32>, value: u32, names: Vec<String>) {
    for name in names {
        target.insert(crate::normalize::normalize_word(&name), value);
    }
}

fn looks_arabic(s: &str) -> bool {
    let t = s.replace(',', ".").replace('−', "-");
    t.parse::<f64>().is_ok() && t.chars().any(|c| c.is_ascii_digit())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    None,
    Hundred,
    Ten,
    Teen,
    Digit,
}

fn classify(n: u32) -> Class {
    if n >= 100 && n.is_multiple_of(100) {
        Class::Hundred
    } else if (11..=19).contains(&n) {
        Class::Teen
    } else if n >= 20 && n.is_multiple_of(10) {
        Class::Ten
    } else {
        Class::Digit
    }
}

fn compatible(prev: Class, next: Class) -> bool {
    matches!(
        (prev, next),
        (Class::None, _)
            | (Class::Hundred, Class::Ten | Class::Teen | Class::Digit)
            | (Class::Ten, Class::Digit)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn nine_point_eighty_one() {
        let lex = NumberLex::new();
        let w = ws("девять целых восемьдесят одна");
        let (n, k) = lex.consume_number(&w, 0).unwrap();
        assert_eq!(n, "9,81");
        assert_eq!(k, w.len());
    }

    #[test]
    fn six_hundred_thirty_two() {
        let lex = NumberLex::new();
        let w = ws("шестьсот тридцать два");
        let (n, _) = lex.consume_number(&w, 0).unwrap();
        assert_eq!(n, "632");
    }
}

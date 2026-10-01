//! Hydrocarbon names, built from their parts.
//!
//! A hydrocarbon name is a prefix naming the number of carbons and a suffix
//! naming the class, and the molecular formula follows arithmetically. Six
//! of these used to be dictionary entries — метан, этан, пропан, бутан,
//! этен, этин — and everything past бутан simply failed: «пентан»,
//! «гексан», «пропен», «декан» all came back as the spoken words.
//!
//! Two small tables replace those six entries and cover thirty names.
//!
//! # What this deliberately does not do
//!
//! It builds a **molecular formula, not a structure**. Butane and isobutane
//! are both C₄H₁₀; but-1-ene and but-2-ene are both C₄H₈. That is correct
//! for a molecular formula and useless for telling isomers apart, so
//! structural names («изобутан», «2-метилпропан») are refused rather than
//! answered with the formula of something that merely shares it.

use serde::Deserialize;

use crate::ast::{Formula, Part};
use crate::error::{Error, Result};
use crate::normalize::normalize_word;

const ORGANIC_YAML: &str = include_str!("../data/domains/chemistry/organic.yaml");

/// Longest chain this table describes. Beyond decane the spoken forms are
/// rare enough that a match is more likely a misrecognition than a name.
pub const MAX_CARBONS: u32 = 10;

/// Why a hydrocarbon name was not built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a hydrocarbon name at all — say nothing and let other readings try.
    NotHydrocarbon,
    /// The class needs a longer chain than the prefix gives: «метен» would
    /// be CH₂ and «метин» would be CH₀.
    ChainTooShort {
        class: String,
        carbons: u32,
        minimum: u32,
    },
    /// Arithmetic left the range of its type.
    Overflow,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotHydrocarbon => f.write_str("не название углеводорода"),
            Refusal::ChainTooShort {
                class,
                carbons,
                minimum,
            } => write!(
                f,
                "{class} с {carbons} атомами углерода не существует: нужно хотя бы {minimum}"
            ),
            Refusal::Overflow => f.write_str("переполнение при подсчёте водородов"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OrganicFile {
    schema_version: u32,
    stems: Vec<StemEntry>,
    classes: Vec<ClassEntry>,
    endings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StemEntry {
    spoken: String,
    carbons: u32,
}

#[derive(Debug, Deserialize)]
struct ClassEntry {
    suffix: String,
    id: String,
    /// Hydrogens are `2n + delta`.
    delta: i32,
    min_carbons: u32,
}

#[derive(Debug)]
pub struct Organic {
    /// Longest first, so «гепт» is tried before a hypothetical «геп».
    stems: Vec<(String, u32)>,
    classes: Vec<ClassEntry>,
    endings: Vec<String>,
}

/// One hydrocarbon, and how it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hydrocarbon {
    pub formula: Formula,
    pub class: String,
    pub carbons: u32,
    pub hydrogens: u32,
}

impl Organic {
    pub fn load(yaml: &str) -> Result<Organic> {
        let file: OrganicFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
            name: "organic.yaml",
            source: e,
        })?;
        let refuse = |reason: String| Error::Parse {
            domain: "chemistry",
            reason,
        };
        if file.schema_version != 1 {
            return Err(refuse(format!(
                "organic.yaml has schema_version {}, this build supports only 1",
                file.schema_version
            )));
        }
        if file.classes.is_empty() || file.stems.is_empty() {
            return Err(refuse("organic.yaml: пустая таблица".into()));
        }
        for stem in &file.stems {
            if stem.carbons == 0 || stem.carbons > MAX_CARBONS {
                return Err(refuse(format!(
                    "organic.yaml: приставка {} задаёт {} атомов углерода, допустимо 1..={MAX_CARBONS}",
                    stem.spoken, stem.carbons
                )));
            }
        }
        for class in &file.classes {
            if class.min_carbons == 0 {
                return Err(refuse(format!(
                    "organic.yaml: класс {} допускает нулевую цепь",
                    class.id
                )));
            }
        }
        let mut stems: Vec<(String, u32)> = file
            .stems
            .into_iter()
            .map(|entry| (normalize_word(&entry.spoken), entry.carbons))
            .collect();
        stems.sort_by(|a, b| {
            b.0.chars()
                .count()
                .cmp(&a.0.chars().count())
                .then(a.0.cmp(&b.0))
        });
        Ok(Organic {
            stems,
            classes: file.classes,
            endings: file.endings,
        })
    }

    pub fn builtin() -> &'static Organic {
        static TABLE: std::sync::OnceLock<Organic> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| Organic::load(ORGANIC_YAML).expect("organic.yaml must be valid"))
    }

    /// Reads one word as a hydrocarbon name.
    pub fn parse(&self, word: &str) -> std::result::Result<Hydrocarbon, Refusal> {
        let word = normalize_word(word);
        let Some((carbons, rest)) = self.stems.iter().find_map(|(stem, carbons)| {
            word.strip_prefix(stem.as_str())
                .map(|rest| (*carbons, rest))
        }) else {
            return Err(Refusal::NotHydrocarbon);
        };
        // The suffix, then whatever case ending the speaker used.
        let Some(class) = self.classes.iter().find(|class| {
            rest.strip_prefix(class.suffix.as_str())
                .is_some_and(|tail| self.endings.iter().any(|ending| ending == tail))
        }) else {
            return Err(Refusal::NotHydrocarbon);
        };
        if carbons < class.min_carbons {
            return Err(Refusal::ChainTooShort {
                class: class.id.clone(),
                carbons,
                minimum: class.min_carbons,
            });
        }
        // hydrogens = 2n + delta, checked: the table is small today and the
        // arithmetic must not become the thing that breaks when it grows.
        let hydrogens = i64::from(carbons)
            .checked_mul(2)
            .and_then(|doubled| doubled.checked_add(i64::from(class.delta)))
            .ok_or(Refusal::Overflow)?;
        let hydrogens = u32::try_from(hydrogens).map_err(|_| Refusal::Overflow)?;
        if hydrogens == 0 {
            return Err(Refusal::ChainTooShort {
                class: class.id.clone(),
                carbons,
                minimum: class.min_carbons,
            });
        }
        Ok(Hydrocarbon {
            formula: Formula {
                parts: vec![
                    Part::Atom {
                        symbol: "C".into(),
                        count: carbons,
                    },
                    Part::Atom {
                        symbol: "H".into(),
                        count: hydrogens,
                    },
                ],
            },
            class: class.id.clone(),
            carbons,
            hydrogens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(word: &str) -> Hydrocarbon {
        Organic::builtin()
            .parse(word)
            .unwrap_or_else(|error| panic!("{word}: {error}"))
    }

    /// The arithmetic, checked against chemistry rather than against the
    /// code: alkanes are CₙH₂ₙ₊₂, alkenes CₙH₂ₙ, alkynes CₙH₂ₙ₋₂.
    #[test]
    fn the_formula_follows_from_the_name() {
        for (word, carbons, hydrogens, class) in [
            ("метан", 1, 4, "alkane"),
            ("этан", 2, 6, "alkane"),
            ("пропан", 3, 8, "alkane"),
            ("бутан", 4, 10, "alkane"),
            ("пентан", 5, 12, "alkane"),
            ("декан", 10, 22, "alkane"),
            ("этен", 2, 4, "alkene"),
            ("пропен", 3, 6, "alkene"),
            ("октен", 8, 16, "alkene"),
            ("этин", 2, 2, "alkyne"),
            ("пропин", 3, 4, "alkyne"),
            ("децин", 10, 18, "alkyne"),
        ] {
            let read = parsed(word);
            assert_eq!(
                (read.carbons, read.hydrogens),
                (carbons, hydrogens),
                "{word}"
            );
            assert_eq!(read.class, class, "{word}");
        }
    }

    /// The two names that do not exist, and the reason is arithmetic:
    /// «метен» would be CH₂ and «метин» would be CH₀.
    #[test]
    fn a_chain_too_short_for_its_class_is_refused() {
        for word in ["метен", "метин"] {
            assert!(
                matches!(
                    Organic::builtin().parse(word),
                    Err(Refusal::ChainTooShort { .. })
                ),
                "{word} = {:?}",
                Organic::builtin().parse(word)
            );
        }
    }

    #[test]
    fn case_endings_reach_the_same_compound() {
        let base = parsed("метан");
        for word in ["метана", "метаном", "метану"] {
            assert_eq!(parsed(word), base, "{word}");
        }
    }

    #[test]
    fn a_word_that_is_not_a_hydrocarbon_says_nothing() {
        for word in ["вода", "серная", "интеграл", "мет", "ан", ""] {
            assert_eq!(
                Organic::builtin().parse(word),
                Err(Refusal::NotHydrocarbon),
                "{word}"
            );
        }
    }

    /// Structural names share a molecular formula with the straight chain,
    /// so answering them with it would claim to have understood a structure
    /// this build does not represent.
    #[test]
    fn a_structural_name_is_not_a_hydrocarbon_name_here() {
        for word in ["изобутан", "неопентан", "циклогексан"] {
            assert_eq!(
                Organic::builtin().parse(word),
                Err(Refusal::NotHydrocarbon),
                "{word}"
            );
        }
    }

    #[test]
    fn a_table_that_would_not_be_arithmetic_is_refused() {
        let base: serde_yaml::Value = serde_yaml::from_str(ORGANIC_YAML).unwrap();
        let mut long = base.clone();
        long["stems"][0]["carbons"] = serde_yaml::Value::from(99);
        assert!(Organic::load(&serde_yaml::to_string(&long).unwrap()).is_err());

        let mut zero = base;
        zero["classes"][0]["min_carbons"] = serde_yaml::Value::from(0);
        assert!(Organic::load(&serde_yaml::to_string(&zero).unwrap()).is_err());
    }
}

//! Building coordination compounds, hydrates and a small set of material
//! classes out of their components.
//!
//! The rule this module exists to keep is the project's oldest one: if a
//! single correct reading cannot be *proved*, the words stay as they were.
//! Every function here returns either a structure it can justify or a
//! [`Refusal`] naming what stopped it. There is no "most likely formula"
//! branch, and adding one would be a defect rather than a feature.
//!
//! # What is deliberately not here
//!
//! No dictionary of whole compounds. The tables describe *components* —
//! a ligand's formula fragment and charge, a numeric prefix, a central atom's
//! symbol — and the compound is assembled from them under a charge
//! constraint. `research`-grade nomenclature (bridging ligands, isomerism,
//! polynuclear centres, κ/η hapticity) is out of scope and refused, not
//! approximated.
//!
//! # Arithmetic
//!
//! Every multiplication and addition that could leave the range of its type
//! is checked. An overflow is a [`Refusal::Overflow`], never a wrapped number
//! that would look like a formula.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::ast::{Center, Complex, Formula, LigandSlot, Part};
use crate::error::{Error, Result};
use crate::formula::gcd_u32;
use crate::normalize::normalize_word;

const COORDINATION_YAML: &str = include_str!("../data/domains/chemistry/coordination.yaml");
const MATERIAL_CLASSES_YAML: &str = include_str!("../data/domains/chemistry/material-classes.yaml");

/// Whether a string looks like a chemical element symbol: one uppercase
/// letter, optionally followed by one or two lowercase ones. Not a check
/// that the element exists — the lexicon owns that — but enough to catch a
/// typo or a whole formula written where a symbol belongs.
fn is_element_symbol(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    rest.len() <= 2 && rest.iter().all(char::is_ascii_lowercase)
}

/// Ligands around one centre. Six is the coordination number this iteration
/// supports; higher numbers exist (7, 8, 12) and are refused rather than
/// built from rules nobody checked.
pub const MAX_LIGAND_MULTIPLICITY: u32 = 6;

/// Counter-ions on either side of a salt. `K₄[Fe(CN)₆]` needs four; a
/// requirement beyond this means the charges were not what the speaker meant.
pub const MAX_COUNTER_IONS: u32 = 12;

/// Waters of crystallisation. Real hydrates reach 10 («декагидрат»); the
/// limit leaves room and still refuses a number that came from a
/// misrecognition.
pub const MAX_HYDRATE_WATERS: u32 = 24;

/// Why a construction was refused.
///
/// Carried rather than flattened into a string so that tests, the evaluator
/// and the eventual user-facing message can each say something different
/// about the same fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The word did not decompose into prefix + ligand + centre.
    NotCoordination,
    UnknownLigand(String),
    UnknownCenter(String),
    /// The centre has more than one oxidation state and the speaker did not
    /// say which.
    AmbiguousOxidation {
        symbol: String,
        states: Vec<i32>,
    },
    /// A count outside the range this iteration supports.
    OutOfRange {
        what: &'static str,
        value: u32,
        limit: u32,
    },
    /// A count of zero, which is not a smaller compound but a different
    /// claim about what was said.
    ZeroMultiplicity(&'static str),
    /// The charges do not admit whole-number counter-ions.
    ChargeUnsatisfiable {
        sphere: i32,
        counter: i32,
    },
    /// Exact arithmetic left the range of its type.
    Overflow(&'static str),
    /// A number was said where an oxidation state belongs, but it cannot be
    /// one here.
    ///
    /// Kept apart from "no number was said": the first must abstain, the
    /// second may fall back to the element's own state.
    ImpossibleOxidation {
        value: i64,
        why: &'static str,
    },
    /// A material class that this build does not claim to cover.
    ClassNotApplicable {
        class: String,
        cation: String,
        reason: String,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotCoordination => f.write_str("не координационное название"),
            Refusal::UnknownLigand(name) => write!(f, "неизвестный лиганд «{name}»"),
            Refusal::UnknownCenter(name) => write!(f, "неизвестный центральный атом «{name}»"),
            Refusal::AmbiguousOxidation { symbol, states } => write!(
                f,
                "у {symbol} несколько степеней окисления {states:?}, а в названии она не указана"
            ),
            Refusal::OutOfRange { what, value, limit } => {
                write!(f, "{what} {value} выходит за поддерживаемый предел {limit}")
            }
            Refusal::ZeroMultiplicity(what) => write!(f, "нулевая кратность: {what}"),
            Refusal::ChargeUnsatisfiable { sphere, counter } => write!(
                f,
                "заряды {sphere} и {counter} не дают целочисленного соотношения"
            ),
            Refusal::Overflow(what) => write!(f, "переполнение при вычислении: {what}"),
            Refusal::ImpossibleOxidation { value, why } => {
                write!(f, "степень окисления {value} здесь невозможна: {why}")
            }
            Refusal::ClassNotApplicable {
                class,
                cation,
                reason,
            } => {
                write!(f, "класс «{class}» не применим к {cation}: {reason}")
            }
        }
    }
}

pub type Built<T> = std::result::Result<T, Refusal>;

// ---------------------------------------------------------------- tables

#[derive(Debug, Deserialize)]
struct CoordinationFile {
    schema_version: u32,
    prefixes: Vec<PrefixEntry>,
    ligands: Vec<LigandEntry>,
    anionic_centers: Vec<CenterEntry>,
    cationic_centers: Vec<String>,
    electron_names: Vec<String>,
    ion_lead_ins: Vec<String>,
    hydrate_stems: Vec<String>,
    oxidation_markers: Vec<OxidationMarkerEntry>,
    roman_numerals: BTreeMap<String, u32>,
}

#[derive(Debug, Deserialize)]
struct PrefixEntry {
    spoken: String,
    value: u32,
}

#[derive(Debug, Deserialize)]
struct LigandEntry {
    id: String,
    /// The form the 2005 recommendations give. Must be among `stems`, so a
    /// table cannot claim an official name it does not actually accept.
    iupac_2005: String,
    stems: Vec<String>,
    formula: String,
    charge: i32,
    max_multiplicity: u32,
}

#[derive(Debug, Deserialize)]
struct CenterEntry {
    stem: String,
    symbol: String,
}

#[derive(Debug, Deserialize)]
struct OxidationMarkerEntry {
    spoken: Vec<String>,
    sign: i32,
}

#[derive(Clone, Debug)]
pub struct Ligand {
    pub id: String,
    pub formula: Formula,
    pub charge: i32,
    pub max_multiplicity: u32,
}

#[derive(Debug)]
pub struct Coordination {
    /// Longest first, so «гекса» is tried before a hypothetical «гек».
    prefixes: Vec<(String, u32)>,
    ligand_stems: Vec<(String, Ligand)>,
    anionic_centers: Vec<(String, String)>,
    pub cationic_centers: Vec<String>,
    electron_names: Vec<String>,
    pub ion_lead_ins: Vec<String>,
    hydrate_stems: Vec<String>,
    oxidation_markers: Vec<(Vec<String>, i32)>,
    roman: BTreeMap<String, u32>,
}

fn by_descending_length<T>(mut table: Vec<(String, T)>) -> Vec<(String, T)> {
    table.sort_by(|a, b| {
        b.0.chars()
            .count()
            .cmp(&a.0.chars().count())
            .then(a.0.cmp(&b.0))
    });
    table
}

impl Coordination {
    pub fn load(yaml: &str) -> Result<Coordination> {
        let file: CoordinationFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
            name: "coordination.yaml",
            source: e,
        })?;
        if file.schema_version != 1 {
            return Err(Error::Parse {
                domain: "chemistry",
                reason: format!(
                    "coordination.yaml has schema_version {}, this build supports only 1",
                    file.schema_version
                ),
            });
        }
        let mut ligand_stems = Vec::new();
        for entry in file.ligands {
            if entry.max_multiplicity == 0 || entry.max_multiplicity > MAX_LIGAND_MULTIPLICITY {
                return Err(Error::Parse { domain: "chemistry", reason: format!(
                    "coordination.yaml: лиганд {} объявляет кратность {}, допустимо 1..={MAX_LIGAND_MULTIPLICITY}",
                    entry.id, entry.max_multiplicity
                ) });
            }
            if !entry.stems.iter().any(|stem| stem == &entry.iupac_2005) {
                return Err(Error::Parse {
                    domain: "chemistry",
                    reason: format!(
                        "coordination.yaml: лиганд {} объявляет форму IUPAC «{}», которой нет среди его основ",
                        entry.id, entry.iupac_2005
                    ),
                });
            }
            let formula = crate::parser::chemistry::parse_formula_notation(&entry.formula)?;
            let ligand = Ligand {
                id: entry.id,
                formula,
                charge: entry.charge,
                max_multiplicity: entry.max_multiplicity,
            };
            for stem in entry.stems {
                ligand_stems.push((normalize_word(&stem), ligand.clone()));
            }
        }
        Ok(Coordination {
            prefixes: by_descending_length(
                file.prefixes
                    .into_iter()
                    .map(|p| (normalize_word(&p.spoken), p.value))
                    .collect(),
            ),
            ligand_stems: by_descending_length(ligand_stems),
            anionic_centers: by_descending_length(
                file.anionic_centers
                    .into_iter()
                    .map(|c| (normalize_word(&c.stem), c.symbol))
                    .collect(),
            ),
            cationic_centers: file.cationic_centers,
            electron_names: file
                .electron_names
                .iter()
                .map(|w| normalize_word(w))
                .collect(),
            ion_lead_ins: file
                .ion_lead_ins
                .iter()
                .map(|w| normalize_word(w))
                .collect(),
            hydrate_stems: file
                .hydrate_stems
                .iter()
                .map(|w| normalize_word(w))
                .collect(),
            oxidation_markers: by_descending_length(
                file.oxidation_markers
                    .into_iter()
                    .map(|m| {
                        (
                            m.spoken
                                .iter()
                                .map(|w| normalize_word(w))
                                .collect::<Vec<_>>()
                                .join(" "),
                            m.sign,
                        )
                    })
                    .collect(),
            )
            .into_iter()
            .map(|(phrase, sign)| {
                (
                    phrase.split(' ').map(str::to_string).collect::<Vec<_>>(),
                    sign,
                )
            })
            .collect(),
            roman: file.roman_numerals,
        })
    }

    pub fn builtin() -> &'static Coordination {
        static TABLE: std::sync::OnceLock<Coordination> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            Coordination::load(COORDINATION_YAML).expect("coordination.yaml must be valid")
        })
    }

    /// A numeric prefix at the start of `word`, with the rest of the word
    /// and the vowel the prefix ended on.
    ///
    /// The vowel is returned because Russian elides it where a prefix meets
    /// a ligand that starts with the same one: тетра + **а**ммин is written
    /// «тетраммин», not «тетрааммин». Without restoring it, the ligand stem
    /// no longer matches what was said.
    fn prefix<'a>(&self, word: &'a str) -> Option<(u32, &'a str, Option<char>)> {
        self.prefixes.iter().find_map(|(spoken, value)| {
            word.strip_prefix(spoken.as_str())
                .map(|rest| (*value, rest, spoken.chars().next_back()))
        })
    }

    /// Matches a ligand stem against `word`, restoring an elided vowel when
    /// one was folded into the preceding prefix.
    fn ligand<'a>(&self, word: &'a str, elided: Option<char>) -> Option<(Ligand, &'a str)> {
        if let Some(found) = self.ligand_stems.iter().find_map(|(stem, ligand)| {
            word.strip_prefix(stem.as_str())
                .map(|rest| (ligand.clone(), rest))
        }) {
            return Some(found);
        }
        // The elided form: put the prefix's final vowel back and try again.
        let vowel = elided.filter(|c| "аеёиоуыэюя".contains(*c))?;
        let restored = format!("{vowel}{word}");
        let (stem_len, ligand) = self.ligand_stems.iter().find_map(|(stem, ligand)| {
            restored
                .starts_with(stem.as_str())
                .then(|| (stem.chars().count(), ligand.clone()))
        })?;
        // One character of the match came from the prefix, so the remainder
        // starts that much earlier in the original word.
        let consumed: usize = restored
            .chars()
            .take(stem_len)
            .map(char::len_utf8)
            .sum::<usize>()
            - vowel.len_utf8();
        Some((ligand, &word[consumed..]))
    }

    /// Roman numerals written as symbols, for when the recogniser produced
    /// `III` rather than «три».
    pub fn roman(&self, word: &str) -> Option<u32> {
        self.roman.get(&word.to_uppercase()).copied()
    }

    /// Whether the word names the electron of a half-reaction.
    pub fn is_electron(&self, word: &str) -> bool {
        let word = normalize_word(word);
        self.electron_names.iter().any(|name| name == &word)
    }

    pub fn is_ion_lead_in(&self, word: &str) -> bool {
        self.ion_lead_ins.iter().any(|w| w == word)
    }

    /// Number of waters if `word` is «пентагидрат» and friends.
    pub fn hydrate(&self, word: &str) -> Option<Built<u32>> {
        let (count, rest, _) = self.prefix(word)?;
        if !self.hydrate_stems.iter().any(|stem| stem == rest) {
            return None;
        }
        Some(if count == 0 {
            Err(Refusal::ZeroMultiplicity("молекул воды"))
        } else if count > MAX_HYDRATE_WATERS {
            Err(Refusal::OutOfRange {
                what: "число молекул воды",
                value: count,
                limit: MAX_HYDRATE_WATERS,
            })
        } else {
            Ok(count)
        })
    }

    /// How many words of an explicit oxidation-state phrase start at `i`,
    /// and which sign it carries.
    ///
    /// Only phrases that mean an oxidation state and nothing else are here.
    /// A bare number is handled by the caller, because a bare number after
    /// an element is ambiguous in a way these phrases are not.
    pub fn oxidation_marker(&self, words: &[String], i: usize) -> Option<(i32, usize)> {
        self.oxidation_markers.iter().find_map(|(phrase, sign)| {
            (i + phrase.len() <= words.len() && words[i..i + phrase.len()] == phrase[..])
                .then_some((*sign, phrase.len()))
        })
    }

    /// Splits an agglutinated coordination name into its parts.
    ///
    /// Russian dictation produces «гексацианоферрат» as one word, so the
    /// decomposition happens inside the token: numeric prefix, ligand stem,
    /// then either a centre stem with the anionic `-ат` suffix or a metal
    /// named in the genitive for a cationic sphere.
    pub fn split(&self, word: &str) -> Built<Decomposition> {
        let word = normalize_word(word);
        let Some((multiplicity, rest, elided)) = self.prefix(&word) else {
            return Err(Refusal::NotCoordination);
        };
        let Some((ligand, rest)) = self.ligand(rest, elided) else {
            return Err(Refusal::NotCoordination);
        };
        if rest.is_empty() {
            return Err(Refusal::UnknownCenter(String::new()));
        }
        // Anionic: «...ферр» + «ат», in the cases dictation produces.
        for suffix in ["ат", "ата", "атом", "ату"] {
            if let Some(stem) = rest.strip_suffix(suffix) {
                if let Some((_, symbol)) = self
                    .anionic_centers
                    .iter()
                    .find(|(candidate, _)| candidate == stem)
                {
                    return Ok(Decomposition {
                        multiplicity,
                        ligand,
                        center: symbol.clone(),
                        kind: SphereKind::Anionic,
                    });
                }
                return Err(Refusal::UnknownCenter(stem.to_string()));
            }
        }
        // Cationic: the metal in the genitive, «тетраммин» + «меди».
        Err(Refusal::UnknownCenter(rest.to_string()))
    }

    /// The cationic form, where the centre is a separate lookup in the
    /// element table because it is spelled as an ordinary Russian word.
    pub fn split_cationic<'a>(&self, word: &'a str) -> Option<(u32, Ligand, &'a str)> {
        let (multiplicity, rest, elided) = self.prefix(word)?;
        let (ligand, rest) = self.ligand(rest, elided)?;
        (!rest.is_empty()).then_some((multiplicity, ligand, rest))
    }
}

/// Where an oxidation state is being read, because what is possible depends
/// on it.
///
/// In a simple salt the number names a **cation**, and a cation with a
/// non-positive charge is not a cation: «гидроксид железа минус три» and
/// «гидроксид железа ноль» are not compounds. In a coordination sphere the
/// same number can legitimately be zero or negative — iron is Fe(−II) in
/// `[Fe(CO)₄]²⁻` — so the rule must not be global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OxidationContext {
    SimpleSalt,
    Coordination,
}

/// The three things reading an oxidation state can find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OxidationRead {
    /// Nothing here. The caller may use the element's own state instead.
    Absent,
    /// A number was said and it cannot be an oxidation state here. The
    /// caller must abstain: falling back would answer a question the speaker
    /// did not ask.
    Refused(Refusal),
    Found {
        value: i32,
        used: usize,
    },
}

/// Turns a spoken count into an oxidation state under `context`.
///
/// `raw` is what was heard, as an unsigned number; `sign` comes from a
/// marker like «минус». Every conversion is checked: `as i32` used to turn
/// 2 147 483 648 into `i32::MIN` and 4 294 967 295 into −1, and both went on
/// to build a formula.
pub fn oxidation_from(raw: u32, sign: i32, context: OxidationContext) -> OxidationRead {
    let Ok(magnitude) = i32::try_from(raw) else {
        return OxidationRead::Refused(Refusal::ImpossibleOxidation {
            value: i64::from(raw),
            why: "число не помещается в степень окисления",
        });
    };
    let Some(value) = magnitude.checked_mul(sign) else {
        return OxidationRead::Refused(Refusal::Overflow("степень окисления"));
    };
    if context == OxidationContext::SimpleSalt && value <= 0 {
        return OxidationRead::Refused(Refusal::ImpossibleOxidation {
            value: i64::from(value),
            why: "в простой соли это число называет катион, а катион не бывает нулевым или отрицательным",
        });
    }
    OxidationRead::Found { value, used: 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SphereKind {
    Anionic,
    Cationic,
}

#[derive(Clone, Debug)]
pub struct Decomposition {
    pub multiplicity: u32,
    pub ligand: Ligand,
    pub center: String,
    pub kind: SphereKind,
}

// ------------------------------------------------------------ arithmetic

/// Builds one coordination sphere and states its charge.
///
/// `z(sphere) = z(centre) + Σ nᵢ·zᵢ` — the constraint from the brief, with
/// every step checked.
pub fn build_sphere(
    center: &str,
    oxidation: i32,
    ligand: &Ligand,
    multiplicity: u32,
) -> Built<Complex> {
    if multiplicity == 0 {
        return Err(Refusal::ZeroMultiplicity("лигандов"));
    }
    if multiplicity > ligand.max_multiplicity {
        return Err(Refusal::OutOfRange {
            what: "кратность лиганда",
            value: multiplicity,
            limit: ligand.max_multiplicity,
        });
    }
    let contribution = i64::from(ligand.charge)
        .checked_mul(i64::from(multiplicity))
        .ok_or(Refusal::Overflow("заряд лигандов"))?;
    let total = i64::from(oxidation)
        .checked_add(contribution)
        .ok_or(Refusal::Overflow("заряд сферы"))?;
    let charge = i32::try_from(total).map_err(|_| Refusal::Overflow("заряд сферы"))?;
    Ok(Complex {
        center: Center {
            symbol: center.to_string(),
            oxidation,
        },
        ligands: vec![LigandSlot {
            formula: ligand.formula.clone(),
            charge: ligand.charge,
            count: multiplicity,
        }],
        charge,
        count: 1,
    })
}

/// The smallest whole-number counts that make a salt neutral.
///
/// `n_counter·z_counter + n_sphere·z_sphere = 0`, minimal and positive.
/// Returns `(counter_ions, spheres)`.
pub fn salt_ratio(sphere_charge: i32, counter_charge: i32) -> Built<(u32, u32)> {
    if sphere_charge == 0 || counter_charge == 0 {
        return Err(Refusal::ChargeUnsatisfiable {
            sphere: sphere_charge,
            counter: counter_charge,
        });
    }
    // A salt needs the two parts to pull in opposite directions.
    if (sphere_charge > 0) == (counter_charge > 0) {
        return Err(Refusal::ChargeUnsatisfiable {
            sphere: sphere_charge,
            counter: counter_charge,
        });
    }
    // `unsigned_abs`, not `abs`: `i32::MIN.abs()` panics in debug and wraps
    // back to `i32::MIN` in release, so the same input either killed the
    // process or produced a silently negative magnitude that `as u32` then
    // turned into two billion counter-ions.
    let sphere_magnitude = sphere_charge.unsigned_abs();
    let counter_magnitude = counter_charge.unsigned_abs();
    let divisor = gcd_u32(sphere_magnitude, counter_magnitude);
    let counters = sphere_magnitude / divisor;
    let spheres = counter_magnitude / divisor;
    for (value, what) in [(counters, "число противоионов"), (spheres, "число сфер")]
    {
        if value == 0 {
            return Err(Refusal::ChargeUnsatisfiable {
                sphere: sphere_charge,
                counter: counter_charge,
            });
        }
        if value > MAX_COUNTER_IONS {
            return Err(Refusal::OutOfRange {
                what,
                value,
                limit: MAX_COUNTER_IONS,
            });
        }
    }
    Ok((counters, spheres))
}

/// Assembles a salt of a counter-ion and a coordination sphere.
pub fn build_salt(counter: &str, counter_charge: i32, sphere: Complex) -> Built<Formula> {
    let (counters, spheres) = salt_ratio(sphere.charge, counter_charge)?;
    let mut sphere = sphere;
    sphere.count = spheres;
    let mut parts = Vec::new();
    if counter_charge > 0 {
        parts.push(Part::Atom {
            symbol: counter.to_string(),
            count: counters,
        });
        parts.push(Part::Complex(sphere));
    } else {
        parts.push(Part::Complex(sphere));
        parts.push(Part::Atom {
            symbol: counter.to_string(),
            count: counters,
        });
    }
    Ok(Formula { parts })
}

// -------------------------------------------------------- material classes

#[derive(Debug, Deserialize)]
struct MaterialFile {
    schema_version: u32,
    #[serde(default)]
    sources: BTreeMap<String, String>,
    classes: Vec<MaterialClassEntry>,
}

#[derive(Debug, Deserialize)]
struct MaterialClassEntry {
    id: String,
    names: Vec<String>,
    #[serde(default)]
    adjective_forms: Vec<AdjectiveForm>,
    structure: String,
    skeleton: Vec<SkeletonAtom>,
    allowed_cations: Vec<AllowedCation>,
    refusals: Refusals,
}

#[derive(Debug, Deserialize)]
struct AdjectiveForm {
    adjective: Vec<String>,
    cation: String,
}

#[derive(Debug, Deserialize)]
struct SkeletonAtom {
    symbol: String,
    count: u32,
}

#[derive(Debug, Deserialize)]
struct AllowedCation {
    symbol: String,
    oxidation: i32,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Deserialize)]
struct Refusals {
    cation_not_listed: String,
    cation_multivalent: String,
}

#[derive(Debug)]
pub struct MaterialClasses {
    classes: Vec<MaterialClassEntry>,
}

/// Evidence that a material-class template was applied, and on what grounds.
///
/// Returned alongside the formula rather than stored inside it: the formula
/// is chemistry, this is provenance, and mixing the two would make every
/// existing corpus record with the same formula compare unequal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassProof {
    pub class: String,
    pub structure: String,
    pub cation: String,
    pub oxidation: i32,
    pub source: String,
}

impl MaterialClasses {
    pub fn load(yaml: &str) -> Result<MaterialClasses> {
        let file: MaterialFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
            name: "material-classes.yaml",
            source: e,
        })?;
        let refuse = |reason: String| Error::Parse {
            domain: "chemistry",
            reason,
        };
        if file.schema_version != 1 {
            return Err(refuse(format!(
                "material-classes.yaml has schema_version {}, this build supports only 1",
                file.schema_version
            )));
        }
        // A template is a claim about the world. These checks are what stops
        // a claim from being made by accident: an empty cation list is the
        // "any metal" rule, a dangling source is a claim with no reference,
        // and a duplicate name means two templates answer to one word and
        // which one wins depends on file order.
        let mut class_ids: BTreeSet<&str> = BTreeSet::new();
        let mut all_names: BTreeSet<String> = BTreeSet::new();
        for class in &file.classes {
            if class.id.trim().is_empty() {
                return Err(refuse("material-classes.yaml: класс без id".into()));
            }
            if !class_ids.insert(class.id.as_str()) {
                return Err(refuse(format!(
                    "material-classes.yaml: id {} встречается дважды",
                    class.id
                )));
            }
            if class.names.is_empty() {
                return Err(refuse(format!(
                    "material-classes.yaml: класс {} не называется ни одним словом",
                    class.id
                )));
            }
            for name in class.names.iter().chain(
                class
                    .adjective_forms
                    .iter()
                    .flat_map(|form| &form.adjective),
            ) {
                let normalised = normalize_word(name);
                if normalised.is_empty() {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} объявляет пустое имя",
                        class.id
                    )));
                }
                if !all_names.insert(normalised.clone()) {
                    return Err(refuse(format!(
                        "material-classes.yaml: имя «{normalised}» принадлежит двум классам; \
                         какой из них ответит, зависело бы от порядка строк в файле"
                    )));
                }
            }
            if class.allowed_cations.is_empty() {
                return Err(refuse(format!(
                    "material-classes.yaml: класс {} не перечисляет ни одного допустимого катиона; \
                     шаблон без списка катионов — это правило «любой металл», которого здесь быть не должно",
                    class.id
                )));
            }
            if class.skeleton.is_empty() {
                return Err(refuse(format!(
                    "material-classes.yaml: класс {} не описывает скелет",
                    class.id
                )));
            }
            for atom in &class.skeleton {
                if atom.count == 0 {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} задаёт нулевое число атомов {}; \
                         ноль атомов — это другой состав, а не тот же с пропуском",
                        class.id, atom.symbol
                    )));
                }
                if !is_element_symbol(&atom.symbol) {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} называет «{}», что не похоже на символ элемента",
                        class.id, atom.symbol
                    )));
                }
            }
            let mut cations: BTreeSet<&str> = BTreeSet::new();
            for cation in &class.allowed_cations {
                if !is_element_symbol(&cation.symbol) {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} допускает катион «{}», что не похоже на символ элемента",
                        class.id, cation.symbol
                    )));
                }
                if !cations.insert(cation.symbol.as_str()) {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} перечисляет {} дважды",
                        class.id, cation.symbol
                    )));
                }
                if cation.oxidation == 0 {
                    return Err(refuse(format!(
                        "material-classes.yaml: класс {} даёт {} нулевую степень окисления",
                        class.id, cation.symbol
                    )));
                }
                if cation.source.trim().is_empty() {
                    return Err(refuse(format!(
                        "material-classes.yaml: у {} в классе {} нет источника; \
                         шаблон материала — утверждение о структуре, и оно должно быть на что-то опёрто",
                        cation.symbol, class.id
                    )));
                }
                if !file.sources.contains_key(&cation.source) {
                    return Err(refuse(format!(
                        "material-classes.yaml: источник «{}» для {} не объявлен в разделе sources",
                        cation.source, cation.symbol
                    )));
                }
            }
        }
        Ok(MaterialClasses {
            classes: file.classes,
        })
    }

    pub fn builtin() -> &'static MaterialClasses {
        static TABLE: std::sync::OnceLock<MaterialClasses> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            MaterialClasses::load(MATERIAL_CLASSES_YAML)
                .expect("material-classes.yaml must be valid")
        })
    }

    /// The class a word introduces, if any.
    pub fn class_of(&self, word: &str) -> Option<&str> {
        let word = normalize_word(word);
        self.classes
            .iter()
            .find(|class| class.names.iter().any(|name| normalize_word(name) == word))
            .map(|class| class.id.as_str())
    }

    /// The cation an adjective names, for «цинковый феррит».
    pub fn cation_of_adjective(&self, class_id: &str, word: &str) -> Option<&str> {
        let word = normalize_word(word);
        let class = self.classes.iter().find(|class| class.id == class_id)?;
        class.adjective_forms.iter().find_map(|form| {
            form.adjective
                .iter()
                .any(|candidate| normalize_word(candidate) == word)
                .then_some(form.cation.as_str())
        })
    }

    /// Applies a template, or explains why it does not apply.
    ///
    /// `element_states` is what the element table knows about the cation; a
    /// metal with more than one recorded state is refused even when the
    /// template would otherwise accept it, because the template's own
    /// oxidation number would then be an assumption rather than a fact.
    pub fn resolve(
        &self,
        class_id: &str,
        cation: &str,
        proven_oxidation: Option<i32>,
    ) -> Built<(Formula, ClassProof)> {
        let Some(class) = self.classes.iter().find(|class| class.id == class_id) else {
            return Err(Refusal::ClassNotApplicable {
                class: class_id.to_string(),
                cation: cation.to_string(),
                reason: "класс не определён".into(),
            });
        };
        let Some(allowed) = class
            .allowed_cations
            .iter()
            .find(|entry| entry.symbol == cation)
        else {
            return Err(Refusal::ClassNotApplicable {
                class: class.id.clone(),
                cation: cation.to_string(),
                reason: class.refusals.cation_not_listed.clone(),
            });
        };
        // The template names an oxidation state; the element table has to
        // agree with it. Passing the raw list and only checking its length
        // let the template assert a state nobody had proved — Zn's list is
        // empty, so `[]` sailed through and the template's own number was
        // taken on trust.
        let Some(proven) = proven_oxidation else {
            return Err(Refusal::ClassNotApplicable {
                class: class.id.clone(),
                cation: cation.to_string(),
                reason: class.refusals.cation_multivalent.clone(),
            });
        };
        if proven != allowed.oxidation {
            return Err(Refusal::ClassNotApplicable {
                class: class.id.clone(),
                cation: cation.to_string(),
                reason: format!(
                    "шаблон требует степень окисления {}, а у элемента доказана {proven}",
                    allowed.oxidation
                ),
            });
        }
        let mut parts = vec![Part::Atom {
            symbol: cation.to_string(),
            count: 1,
        }];
        for atom in &class.skeleton {
            if atom.count == 0 {
                return Err(Refusal::ZeroMultiplicity("атомов скелета класса"));
            }
            parts.push(Part::Atom {
                symbol: atom.symbol.clone(),
                count: atom.count,
            });
        }
        Ok((
            Formula { parts },
            ClassProof {
                class: class.id.clone(),
                structure: class.structure.clone(),
                cation: cation.to_string(),
                oxidation: allowed.oxidation,
                source: allowed.source.clone(),
            },
        ))
    }
}

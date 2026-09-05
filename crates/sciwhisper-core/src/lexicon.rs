//! Versioned YAML lexicons. Built-in data is embedded; unknown future schemas fail closed.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::ast::{Formula, StateMarker};
use crate::dimension::Dimension;
use crate::error::{Error, Result};
use crate::formula::{parse_equation_str, parse_formula_str};
use crate::normalize::normalize_word;

pub const SUPPORTED_SCHEMA: u32 = 1;

/// `units.yaml` moved to its own schema when every unit gained a required
/// `dimension`. Only this exact version is accepted: schema 1 carries no
/// dimensions at all, and a newer one may redefine what they mean.
pub const UNITS_SCHEMA: u32 = 2;

/// `aliases.yaml` became the single source of the spoken chemistry
/// connectives in schema 2. Schema 1 held a partial, unused copy while the
/// real list lived in Rust; accepting it would silently restore that split.
pub const ALIASES_SCHEMA: u32 = 2;

const ELEMENTS_YAML: &str = include_str!("../data/domains/chemistry/elements.yaml");
const IONS_YAML: &str = include_str!("../data/domains/chemistry/ions.yaml");
const SUBSTANCES_YAML: &str = include_str!("../data/domains/chemistry/substances.yaml");
const GREEK_YAML: &str = include_str!("../data/domains/common/greek.yaml");
const SYMBOLS_YAML: &str = include_str!("../data/domains/common/symbols.yaml");
const UNITS_YAML: &str = include_str!("../data/domains/physics/units.yaml");
const CHEM101_YAML: &str = include_str!("../data/courses/chem101.yaml");
const ALIASES_YAML: &str = include_str!("../data/domains/chemistry/aliases.yaml");

#[derive(Clone, Debug)]
pub struct Lexicon {
    pub elements_by_name: HashMap<String, Element>,
    pub elements_by_symbol: HashMap<String, Element>,
    pub anion_classes: HashMap<String, AnionClass>,
    pub substances: Vec<NamedFormula>,
    pub greek: HashMap<String, GreekLetter>,
    pub latin: HashMap<String, char>,
    pub cyrillic: HashMap<String, String>,
    pub units: Vec<NamedUnit>,
    /// Unit symbol as it appears in the AST (`м`, `см`, `Дж`) -> SI dimension.
    pub unit_dimensions: HashMap<String, Dimension>,
    pub shortcuts: Vec<Shortcut>,
    /// Spoken chemistry connectives, loaded from `aliases.yaml`. The parser
    /// holds no second copy of these phrases.
    pub chemistry_speech: ChemistrySpeech,
}

/// What a spoken chemistry connective means. These are the joints of the
/// reaction grammar, not whole sentences: subject, reagent, product and arrow
/// are assembled from them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChemConnective {
    /// `LEFT <phrase> RIGHT`
    Forward,
    Equilibrium,
    /// `LEFT <phrase> LEFT` — another reagent on the same side.
    JoinReagent,
    /// Opens the product side. Not an arrow on its own.
    ProductMarker,
    /// `LEFT <phrase> RIGHT`, and `и` splits the products.
    Decompose,
    /// «из A получается B»
    FromMarker,
    ToMarker,
    /// «между A и B …»
    BetweenMarker,
    /// A noun. Never licenses a reaction by itself.
    ReactionNoun,
    Plus,
    /// `и` — a `+` only inside a side that already parses as chemistry.
    Conjunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChemCondition {
    Heat,
    Light,
    Electrolysis,
    Catalyst,
}

impl ChemCondition {
    pub fn as_str(self) -> &'static str {
        match self {
            ChemCondition::Heat => "heat",
            ChemCondition::Light => "light",
            ChemCondition::Electrolysis => "electrolysis",
            ChemCondition::Catalyst => "catalyst",
        }
    }
}

/// Phrase tables for spoken chemistry, all matched longest-first.
#[derive(Clone, Debug, Default)]
pub struct ChemistrySpeech {
    connectives: Vec<(Vec<String>, ChemConnective)>,
    conditions: Vec<(Vec<String>, ChemCondition)>,
    markers: Vec<(Vec<String>, StateMarker)>,
    ion_markers: Vec<String>,
    grouping: Vec<(Vec<String>, u32)>,
}

impl ChemistrySpeech {
    pub fn connective_at(&self, words: &[String], i: usize) -> Option<(ChemConnective, usize)> {
        longest_phrase(&self.connectives, words, i)
    }

    pub fn condition_at(&self, words: &[String], i: usize) -> Option<(ChemCondition, usize)> {
        longest_phrase(&self.conditions, words, i)
    }

    pub fn marker_at(&self, words: &[String], i: usize) -> Option<(StateMarker, usize)> {
        longest_phrase(&self.markers, words, i)
    }

    pub fn grouping_at(&self, words: &[String], i: usize) -> Option<(u32, usize)> {
        longest_phrase(&self.grouping, words, i)
    }

    pub fn is_ion_marker(&self, word: &str) -> bool {
        self.ion_markers.iter().any(|marker| marker == word)
    }

    /// Every phrase that carries this meaning, for tests and diagnostics.
    pub fn phrases_for(&self, meaning: ChemConnective) -> Vec<String> {
        self.connectives
            .iter()
            .filter(|(_, kind)| *kind == meaning)
            .map(|(words, _)| words.join(" "))
            .collect()
    }
}

/// Russian noun and adjective endings, longest first. Trimming one of these
/// turns «серной кислотой» into the same pair of stems as «серная кислота»,
/// which is what lets a substance be dictated in the case the sentence
/// actually needs.
const RUSSIAN_ENDINGS: [&str; 41] = [
    "ами", "ями", "ого", "его", "ому", "ему", "ыми", "ими", "ых", "их", "ый", "ий", "ым", "им",
    "ой", "ей", "ою", "ею", "ом", "ем", "ём", "ах", "ях", "ую", "юю", "ая", "яя", "ые", "ие", "ов",
    "ев", "ья", "ью", "а", "я", "ы", "и", "у", "ю", "е", "о",
];

/// Shortest stem this may reduce a word to. Below it, unrelated short words
/// start colliding, and a wrong substance is far worse than an unrecognised
/// one.
const MIN_STEM_CHARS: usize = 3;

/// Whether two spoken words are the same word in different cases.
///
/// This is deliberately shallow: one ending is trimmed, nothing is added, and
/// a stem shorter than [`MIN_STEM_CHARS`] is refused. It exists so that
/// «с серной кислотой» and «между водородом и кислородом» reach the same
/// substances as their dictionary forms, not to conjugate Russian.
fn same_russian_stem(spoken: &str, expected: &str) -> bool {
    if spoken == expected {
        return true;
    }
    let a = russian_stem(spoken);
    let b = russian_stem(expected);
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        // A word that is already a bare stem still matches an inflected form.
        (Some(a), None) => a == expected,
        (None, Some(b)) => spoken == b,
        (None, None) => false,
    }
}

fn russian_stem(word: &str) -> Option<&str> {
    RUSSIAN_ENDINGS.iter().find_map(|ending| {
        let stem = word.strip_suffix(ending)?;
        (stem.chars().count() >= MIN_STEM_CHARS).then_some(stem)
    })
}

/// Longest match at `i`, with a deterministic tie-break: the tables are sorted
/// by descending phrase length at load time, so the first hit is the longest.
fn longest_phrase<T: Copy>(
    table: &[(Vec<String>, T)],
    words: &[String],
    i: usize,
) -> Option<(T, usize)> {
    table.iter().find_map(|(phrase, value)| {
        (i + phrase.len() <= words.len() && words[i..i + phrase.len()] == phrase[..])
            .then_some((*value, phrase.len()))
    })
}

#[derive(Clone, Debug)]
pub struct Element {
    pub symbol: String,
    pub names: Vec<String>,
    pub diatomic: bool,
    pub default_oxidation: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct AnionClass {
    pub id: String,
    pub names: Vec<String>,
    pub charge: i32,
    pub group: bool,
    pub formula: Formula,
    pub role: IonRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IonRole {
    Anion,
    Cation,
}

#[derive(Clone, Debug)]
pub struct NamedFormula {
    pub names: Vec<String>,
    pub formula: Formula,
    pub canonical_name: Option<String>,
    pub nomenclature_status: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GreekLetter {
    pub lower: String,
    pub upper: String,
    pub latin: String,
}

#[derive(Clone, Debug)]
pub struct NamedUnit {
    pub names: Vec<String>,
    pub symbol: String,
    pub dimension: Dimension,
}

#[derive(Clone, Debug)]
pub struct Shortcut {
    pub id: String,
    pub phrases: Vec<String>,
    pub equation: crate::ast::Equation,
}

impl Lexicon {
    pub fn builtin() -> &'static Lexicon {
        static LEX: OnceLock<Lexicon> = OnceLock::new();
        LEX.get_or_init(|| load_builtin().expect("embedded lexicon must be valid"))
    }

    pub fn element(&self, word: &str) -> Option<&Element> {
        self.elements_by_name.get(&normalize_word(word))
    }

    pub fn anion(&self, word: &str) -> Option<&AnionClass> {
        self.anion_classes.get(&normalize_word(word))
    }

    pub fn greek(&self, word: &str) -> Option<&GreekLetter> {
        self.greek.get(&normalize_word(word))
    }

    pub fn latin(&self, word: &str) -> Option<char> {
        self.latin.get(&normalize_word(word)).copied()
    }

    pub fn longest_substance(&self, words: &[String], i: usize) -> Option<(Formula, usize)> {
        // Exact spellings win outright. Only when nothing matches exactly is
        // the case-insensitive-to-inflection fallback tried, so a listed form
        // can never be overruled by a stem.
        self.match_substance(words, i, false)
            .or_else(|| self.match_substance(words, i, true))
    }

    fn match_substance(
        &self,
        words: &[String],
        i: usize,
        by_stem: bool,
    ) -> Option<(Formula, usize)> {
        let mut best: Option<(Formula, usize)> = None;
        for item in &self.substances {
            for name in &item.names {
                let nw: Vec<String> = name.split_whitespace().map(|s| s.to_string()).collect();
                if nw.is_empty() || i + nw.len() > words.len() {
                    continue;
                }
                let hit = nw.iter().enumerate().all(|(k, expected)| {
                    let spoken = &words[i + k];
                    if by_stem {
                        same_russian_stem(spoken, expected)
                    } else {
                        spoken == expected
                    }
                });
                if hit {
                    let used = nw.len();
                    if best.as_ref().map(|(_, n)| *n).unwrap_or(0) < used {
                        best = Some((item.formula.clone(), used));
                    }
                }
            }
        }
        best
    }

    pub fn longest_unit(&self, words: &[String], i: usize) -> Option<(NamedUnit, usize)> {
        let mut best: Option<(NamedUnit, usize)> = None;
        for u in &self.units {
            for name in &u.names {
                let nw: Vec<String> = name.split_whitespace().map(|s| s.to_string()).collect();
                if nw.is_empty() || i + nw.len() > words.len() {
                    continue;
                }
                if words[i..i + nw.len()] == nw[..] {
                    let used = nw.len();
                    if best.as_ref().map(|(_, n)| *n).unwrap_or(0) < used {
                        best = Some((u.clone(), used));
                    }
                }
            }
        }
        best
    }

    /// SI dimension of a unit symbol as the AST spells it, or `None` for a
    /// unit this build does not know — the caller must then abstain.
    pub fn unit_dimension(&self, symbol: &str) -> Option<Dimension> {
        self.unit_dimensions.get(symbol).copied()
    }

    pub fn shortcut_exact(&self, normalized: &str) -> Option<&Shortcut> {
        self.shortcuts
            .iter()
            .find(|s| s.phrases.iter().any(|p| p == normalized))
    }
}

fn load_builtin() -> Result<Lexicon> {
    let mut lex = Lexicon {
        elements_by_name: HashMap::new(),
        elements_by_symbol: HashMap::new(),
        anion_classes: HashMap::new(),
        substances: Vec::new(),
        greek: HashMap::new(),
        latin: HashMap::new(),
        cyrillic: HashMap::new(),
        units: Vec::new(),
        unit_dimensions: HashMap::new(),
        shortcuts: Vec::new(),
        chemistry_speech: ChemistrySpeech::default(),
    };
    load_elements(&mut lex, ELEMENTS_YAML)?;
    load_ions(&mut lex, IONS_YAML)?;
    load_substances(&mut lex, SUBSTANCES_YAML)?;
    load_greek(&mut lex, GREEK_YAML)?;
    load_symbols(&mut lex, SYMBOLS_YAML)?;
    load_units(&mut lex, UNITS_YAML)?;
    load_course(&mut lex, CHEM101_YAML)?;
    lex.chemistry_speech = load_chemistry_speech(ALIASES_YAML)?;
    Ok(lex)
}

fn check_schema(name: &'static str, v: u32) -> Result<()> {
    if v > SUPPORTED_SCHEMA {
        Err(Error::UnsupportedSchema {
            found: v,
            supported: SUPPORTED_SCHEMA,
        })
    } else if v == 0 {
        Err(Error::Parse {
            domain: "lexicon",
            reason: format!("{name} missing schema_version"),
        })
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
struct ElementsFile {
    schema_version: u32,
    elements: Vec<ElementEntry>,
}

#[derive(Deserialize)]
struct ElementEntry {
    symbol: String,
    names: Vec<String>,
    #[serde(default)]
    diatomic: bool,
    #[serde(default)]
    default_oxidation: Option<i32>,
}

fn load_elements(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: ElementsFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "elements.yaml",
        source: e,
    })?;
    check_schema("elements.yaml", f.schema_version)?;
    for e in f.elements {
        let el = Element {
            symbol: e.symbol.clone(),
            names: e.names.clone(),
            diatomic: e.diatomic,
            default_oxidation: e.default_oxidation,
        };
        lex.elements_by_symbol.insert(e.symbol.clone(), el.clone());
        for n in e.names {
            lex.elements_by_name.insert(normalize_word(&n), el.clone());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct IonsFile {
    schema_version: u32,
    anion_classes: Vec<AnionEntry>,
}

#[derive(Deserialize)]
struct AnionEntry {
    id: String,
    names: Vec<String>,
    charge: i32,
    #[serde(default)]
    group: bool,
    formula: String,
    #[serde(default)]
    role: Option<String>,
}

fn load_ions(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: IonsFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "ions.yaml",
        source: e,
    })?;
    check_schema("ions.yaml", f.schema_version)?;
    for e in f.anion_classes {
        let formula = parse_formula_str(&e.formula)?;
        let role = if e.role.as_deref() == Some("cation") {
            IonRole::Cation
        } else {
            IonRole::Anion
        };
        let class = AnionClass {
            id: e.id,
            names: e.names.clone(),
            charge: e.charge,
            group: e.group,
            formula,
            role,
        };
        for n in e.names {
            lex.anion_classes.insert(normalize_word(&n), class.clone());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct SubstancesFile {
    schema_version: u32,
    #[serde(default)]
    sources: HashMap<String, String>,
    substances: Vec<SubstanceEntry>,
}

#[derive(Deserialize)]
struct SubstanceEntry {
    names: Vec<String>,
    #[serde(default)]
    asr_aliases: Vec<String>,
    formula: String,
    #[serde(default)]
    canonical_name: Option<String>,
    #[serde(default)]
    nomenclature_status: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

fn load_substances(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: SubstancesFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "substances.yaml",
        source: e,
    })?;
    check_schema("substances.yaml", f.schema_version)?;
    for mut e in f.substances {
        let formula = parse_formula_str(&e.formula)?;
        let source = match &e.source {
            Some(key) => Some(f.sources.get(key).cloned().ok_or_else(|| Error::Parse {
                domain: "lexicon",
                reason: format!("substances.yaml references unknown source '{key}'"),
            })?),
            None => None,
        };
        e.names.append(&mut e.asr_aliases);
        lex.substances.push(NamedFormula {
            names: e.names.into_iter().map(|n| normalize_word(&n)).collect(),
            formula,
            canonical_name: e.canonical_name,
            nomenclature_status: e.nomenclature_status,
            source,
        });
    }
    Ok(())
}

#[derive(Deserialize)]
struct GreekFile {
    schema_version: u32,
    letters: Vec<GreekEntry>,
}

#[derive(Deserialize)]
struct GreekEntry {
    spoken: Vec<String>,
    lower: String,
    upper: String,
    #[serde(default)]
    latin: String,
}

fn load_greek(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: GreekFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "greek.yaml",
        source: e,
    })?;
    check_schema("greek.yaml", f.schema_version)?;
    for e in f.letters {
        let g = GreekLetter {
            lower: e.lower,
            upper: e.upper,
            latin: e.latin,
        };
        for s in e.spoken {
            lex.greek.insert(normalize_word(&s), g.clone());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct SymbolsFile {
    schema_version: u32,
    letters: Vec<LetterEntry>,
}

#[derive(Deserialize)]
struct LetterEntry {
    spoken: Vec<String>,
    #[serde(default)]
    latin: Option<String>,
    #[serde(default)]
    cyrillic: Option<String>,
}

fn load_symbols(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: SymbolsFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "symbols.yaml",
        source: e,
    })?;
    check_schema("symbols.yaml", f.schema_version)?;
    for e in f.letters {
        if let Some(lat) = e.latin.as_ref().and_then(|s| s.chars().next()) {
            for s in &e.spoken {
                lex.latin.insert(normalize_word(s), lat);
            }
        }
        if let Some(cyr) = e.cyrillic.clone() {
            for s in &e.spoken {
                lex.cyrillic.insert(normalize_word(s), cyr.clone());
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct UnitsFile {
    schema_version: u32,
    #[serde(default)]
    si_base: Vec<UnitEntry>,
    #[serde(default)]
    derived: Vec<UnitEntry>,
    #[serde(default)]
    composed: Vec<UnitEntry>,
}

#[derive(Deserialize)]
struct UnitEntry {
    symbol: String,
    /// Required: every unit must state its SI dimension, and an unparsable
    /// one fails the whole build of the lexicon rather than silently
    /// disabling dimensional analysis for that unit.
    dimension: String,
    #[serde(default)]
    spoken: Vec<String>,
}

fn load_units(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: UnitsFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "units.yaml",
        source: e,
    })?;
    if f.schema_version != UNITS_SCHEMA {
        return Err(Error::Parse {
            domain: "lexicon",
            reason: format!(
                "units.yaml has schema_version {}, this build supports only {UNITS_SCHEMA}",
                f.schema_version
            ),
        });
    }
    for e in f.si_base.into_iter().chain(f.derived).chain(f.composed) {
        let dimension = Dimension::parse(&e.dimension).ok_or_else(|| Error::Parse {
            domain: "lexicon",
            reason: format!(
                "units.yaml: unit '{}' has an invalid dimension '{}'",
                e.symbol, e.dimension
            ),
        })?;
        // A repeated symbol is always a data bug; a repeat with a different
        // dimension would silently decide which physics wins.
        if let Some(previous) = lex.unit_dimensions.get(&e.symbol) {
            return Err(Error::Parse {
                domain: "lexicon",
                reason: if *previous == dimension {
                    format!("units.yaml: unit symbol '{}' is defined twice", e.symbol)
                } else {
                    format!(
                        "units.yaml: unit symbol '{}' is defined twice with different dimensions ({previous} vs {dimension})",
                        e.symbol
                    )
                },
            });
        }
        lex.unit_dimensions.insert(e.symbol.clone(), dimension);
        if e.spoken.is_empty() {
            continue;
        }
        lex.units.push(NamedUnit {
            names: e.spoken.into_iter().map(|n| normalize_word(&n)).collect(),
            symbol: e.symbol,
            dimension,
        });
    }
    Ok(())
}

#[derive(Deserialize)]
struct CourseFile {
    schema_version: u32,
    #[serde(default)]
    reactions: Vec<ReactionEntry>,
}

#[derive(Deserialize)]
struct ReactionEntry {
    id: String,
    phrases: Vec<String>,
    equation: String,
}

fn load_course(lex: &mut Lexicon, yaml: &str) -> Result<()> {
    let f: CourseFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "chem101.yaml",
        source: e,
    })?;
    check_schema("chem101.yaml", f.schema_version)?;
    for r in f.reactions {
        let equation = parse_equation_str(&r.equation)?;
        lex.shortcuts.push(Shortcut {
            id: r.id,
            phrases: r
                .phrases
                .into_iter()
                .map(|p| crate::normalize::normalize(&p))
                .collect(),
            equation,
        });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasesFile {
    schema_version: u32,
    reaction: ReactionAliases,
    conditions: ConditionAliases,
    markers: MarkerAliases,
    ion: IonAliases,
    grouping: GroupingAliases,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReactionAliases {
    forward: Vec<String>,
    equilibrium: Vec<String>,
    join_reagent: Vec<String>,
    product_marker: Vec<String>,
    decompose: Vec<String>,
    from_marker: Vec<String>,
    to_marker: Vec<String>,
    between_marker: Vec<String>,
    reaction_noun: Vec<String>,
    plus: Vec<String>,
    conjunction: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionAliases {
    heat: Vec<String>,
    light: Vec<String>,
    electrolysis: Vec<String>,
    catalyst: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerAliases {
    gas: Vec<String>,
    precipitate: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IonAliases {
    markers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupingAliases {
    twice: Vec<String>,
    thrice: Vec<String>,
}

/// Loads the spoken chemistry connectives.
///
/// A phrase that would mean two different things is a hard error: leaving it
/// in would let the order of lines in a data file decide the chemistry.
fn load_chemistry_speech(yaml: &str) -> Result<ChemistrySpeech> {
    let file: AliasesFile = serde_yaml::from_str(yaml).map_err(|e| Error::Lexicon {
        name: "aliases.yaml",
        source: e,
    })?;
    if file.schema_version != ALIASES_SCHEMA {
        return Err(Error::Parse {
            domain: "lexicon",
            reason: format!(
                "aliases.yaml has schema_version {}, this build supports only {ALIASES_SCHEMA}",
                file.schema_version
            ),
        });
    }

    let mut speech = ChemistrySpeech::default();
    // One shared map so that a phrase cannot mean a connective in one section
    // and a condition in another.
    let mut claimed: HashMap<String, String> = HashMap::new();

    let claim = |phrase: &str,
                 meaning: &str,
                 claimed: &mut HashMap<String, String>|
     -> Result<Vec<String>> {
        let words = crate::normalize::words(phrase);
        if words.is_empty() {
            return Err(Error::Parse {
                domain: "lexicon",
                reason: format!("aliases.yaml: empty phrase under '{meaning}'"),
            });
        }
        let key = words.join(" ");
        if let Some(previous) = claimed.get(&key) {
            return Err(Error::Parse {
                domain: "lexicon",
                reason: if previous == meaning {
                    format!("aliases.yaml: phrase '{key}' is listed twice under '{meaning}'")
                } else {
                    format!("aliases.yaml: phrase '{key}' means both '{previous}' and '{meaning}'")
                },
            });
        }
        claimed.insert(key, meaning.to_string());
        Ok(words)
    };

    let reaction = file.reaction;
    for (phrases, meaning, name) in [
        (
            reaction.forward,
            ChemConnective::Forward,
            "reaction.forward",
        ),
        (
            reaction.equilibrium,
            ChemConnective::Equilibrium,
            "reaction.equilibrium",
        ),
        (
            reaction.join_reagent,
            ChemConnective::JoinReagent,
            "reaction.join_reagent",
        ),
        (
            reaction.product_marker,
            ChemConnective::ProductMarker,
            "reaction.product_marker",
        ),
        (
            reaction.decompose,
            ChemConnective::Decompose,
            "reaction.decompose",
        ),
        (
            reaction.from_marker,
            ChemConnective::FromMarker,
            "reaction.from_marker",
        ),
        (
            reaction.to_marker,
            ChemConnective::ToMarker,
            "reaction.to_marker",
        ),
        (
            reaction.between_marker,
            ChemConnective::BetweenMarker,
            "reaction.between_marker",
        ),
        (
            reaction.reaction_noun,
            ChemConnective::ReactionNoun,
            "reaction.reaction_noun",
        ),
        (reaction.plus, ChemConnective::Plus, "reaction.plus"),
        (
            reaction.conjunction,
            ChemConnective::Conjunction,
            "reaction.conjunction",
        ),
    ] {
        for phrase in phrases {
            let words = claim(&phrase, name, &mut claimed)?;
            speech.connectives.push((words, meaning));
        }
    }

    let conditions = file.conditions;
    for (phrases, meaning, name) in [
        (conditions.heat, ChemCondition::Heat, "conditions.heat"),
        (conditions.light, ChemCondition::Light, "conditions.light"),
        (
            conditions.electrolysis,
            ChemCondition::Electrolysis,
            "conditions.electrolysis",
        ),
        (
            conditions.catalyst,
            ChemCondition::Catalyst,
            "conditions.catalyst",
        ),
    ] {
        for phrase in phrases {
            let words = claim(&phrase, name, &mut claimed)?;
            speech.conditions.push((words, meaning));
        }
    }

    for (phrases, meaning, name) in [
        (file.markers.gas, StateMarker::Gas, "markers.gas"),
        (
            file.markers.precipitate,
            StateMarker::Precipitate,
            "markers.precipitate",
        ),
    ] {
        for phrase in phrases {
            let words = claim(&phrase, name, &mut claimed)?;
            speech.markers.push((words, meaning));
        }
    }

    for (phrases, count, name) in [
        (file.grouping.twice, 2u32, "grouping.twice"),
        (file.grouping.thrice, 3u32, "grouping.thrice"),
    ] {
        for phrase in phrases {
            let words = claim(&phrase, name, &mut claimed)?;
            speech.grouping.push((words, count));
        }
    }

    for phrase in file.ion.markers {
        let words = claim(&phrase, "ion.markers", &mut claimed)?;
        if words.len() != 1 {
            return Err(Error::Parse {
                domain: "lexicon",
                reason: format!(
                    "aliases.yaml: ion marker '{}' must be one word",
                    words.join(" ")
                ),
            });
        }
        speech.ion_markers.push(words[0].clone());
    }

    // Longest-first, then alphabetically, so the tables are fully determined
    // by their contents and never by the order of lines in the file.
    fn by_length<T>(a: &(Vec<String>, T), b: &(Vec<String>, T)) -> std::cmp::Ordering {
        b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0))
    }
    speech.connectives.sort_by(by_length);
    speech.conditions.sort_by(by_length);
    speech.markers.sort_by(by_length);
    speech.grouping.sort_by(by_length);
    speech.ion_markers.sort();
    Ok(speech)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_lexicon() -> Lexicon {
        Lexicon {
            elements_by_name: HashMap::new(),
            elements_by_symbol: HashMap::new(),
            anion_classes: HashMap::new(),
            substances: Vec::new(),
            greek: HashMap::new(),
            latin: HashMap::new(),
            cyrillic: HashMap::new(),
            units: Vec::new(),
            unit_dimensions: HashMap::new(),
            shortcuts: Vec::new(),
            chemistry_speech: ChemistrySpeech::default(),
        }
    }

    #[test]
    fn units_file_requires_its_own_schema_version() {
        // Schema 1 predates dimensions entirely and must be refused, not
        // loaded with silently missing physics.
        let yaml =
            "schema_version: 1\nsi_base:\n  - symbol: м\n    dimension: L\n    spoken: [метр]\n";
        let error = load_units(&mut empty_lexicon(), yaml)
            .unwrap_err()
            .to_string();
        assert!(error.contains("schema_version 1"), "{error}");
        assert!(error.contains("supports only 2"), "{error}");

        let future =
            "schema_version: 3\nsi_base:\n  - symbol: м\n    dimension: L\n    spoken: [метр]\n";
        assert!(load_units(&mut empty_lexicon(), future).is_err());
    }

    #[test]
    fn units_file_rejects_a_conflicting_duplicate_symbol() {
        let yaml = "schema_version: 2\nsi_base:\n  - symbol: Кл\n    dimension: I T\n    spoken: [кулон]\nderived:\n  - symbol: Кл\n    dimension: M L^2\n    spoken: [кулон]\n";
        let error = load_units(&mut empty_lexicon(), yaml)
            .unwrap_err()
            .to_string();
        assert!(error.contains("defined twice"), "{error}");
        assert!(error.contains("different dimensions"), "{error}");
    }

    #[test]
    fn units_file_rejects_a_repeated_symbol_even_when_consistent() {
        let yaml = "schema_version: 2\nsi_base:\n  - symbol: м\n    dimension: L\n    spoken: [метр]\n  - symbol: м\n    dimension: L\n    spoken: [метра]\n";
        let error = load_units(&mut empty_lexicon(), yaml)
            .unwrap_err()
            .to_string();
        assert!(error.contains("defined twice"), "{error}");
    }

    #[test]
    fn units_file_rejects_an_invalid_dimension() {
        let yaml =
            "schema_version: 2\nsi_base:\n  - symbol: м\n    dimension: Q^2\n    spoken: [метр]\n";
        let error = load_units(&mut empty_lexicon(), yaml)
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid dimension"), "{error}");
    }

    #[test]
    fn builtin_units_have_unique_symbols() {
        let lex = Lexicon::builtin();
        assert_eq!(lex.unit_dimension("Кл").unwrap().to_string(), "T I");
        // Every spoken form of a symbol resolves to the same dimension.
        for unit in &lex.units {
            assert_eq!(
                lex.unit_dimension(&unit.symbol),
                Some(unit.dimension),
                "unit {}",
                unit.symbol
            );
        }
    }

    #[test]
    fn sourced_substances_keep_iupac_provenance() {
        let acetic = Lexicon::builtin()
            .substances
            .iter()
            .find(|item| item.canonical_name.as_deref() == Some("acetic acid"))
            .expect("acetic acid must be present");
        assert_eq!(acetic.nomenclature_status.as_deref(), Some("retained_pin"));
        assert!(acetic
            .source
            .as_deref()
            .is_some_and(|source| source.starts_with("https://iupac.")));
    }

    // ------------------------------------------------- spoken chemistry data

    const MINIMAL_ALIASES: &str = r#"
schema_version: 2
reaction:
  forward: [превращается в]
  equilibrium: [обратимо]
  join_reagent: [реагирует с]
  product_marker: [с образованием]
  decompose: [разлагается на]
  from_marker: [из]
  to_marker: [получается]
  between_marker: [между]
  reaction_noun: [идёт реакция]
  plus: [плюс]
  conjunction: [и]
conditions:
  heat: [при нагревании]
  light: [на свету]
  electrolysis: [электролиз]
  catalyst: [катализатор]
markers:
  gas: [газ]
  precipitate: [осадок]
ion:
  markers: [ион]
grouping:
  twice: [дважды]
  thrice: [трижды]
"#;

    #[test]
    fn the_minimal_alias_file_loads() {
        let speech = load_chemistry_speech(MINIMAL_ALIASES).expect("valid aliases");
        let words = crate::normalize::words("превращается в");
        assert_eq!(
            speech.connective_at(&words, 0),
            Some((ChemConnective::Forward, 2))
        );
    }

    #[test]
    fn one_phrase_may_not_carry_two_meanings() {
        // «плюс» as both a separator and a product marker would let the order
        // of lines in a data file decide the chemistry.
        let clashing = MINIMAL_ALIASES.replace(
            "  product_marker: [с образованием]",
            "  product_marker: [с образованием, плюс]",
        );
        let error = load_chemistry_speech(&clashing).expect_err("must be refused");
        let message = error.to_string();
        assert!(message.contains("плюс"), "{message}");
        assert!(message.contains("means both"), "{message}");
    }

    #[test]
    fn the_same_phrase_listed_twice_is_refused() {
        let repeated = MINIMAL_ALIASES.replace(
            "  forward: [превращается в]",
            "  forward: [превращается в, превращается в]",
        );
        let error = load_chemistry_speech(&repeated).expect_err("must be refused");
        assert!(error.to_string().contains("listed twice"), "{error}");
    }

    #[test]
    fn an_unknown_destination_key_is_refused() {
        let unknown =
            MINIMAL_ALIASES.replace("  plus: [плюс]", "  plus: [плюс]\n  sideways: [вбок]");
        let error = load_chemistry_speech(&unknown).expect_err("must be refused");
        assert!(
            error.to_string().contains("sideways") || error.to_string().contains("unknown field"),
            "{error}"
        );
    }

    #[test]
    fn an_unsupported_alias_schema_is_refused() {
        let old = MINIMAL_ALIASES.replace("schema_version: 2", "schema_version: 1");
        let error = load_chemistry_speech(&old).expect_err("schema 1 held a second, unused copy");
        assert!(error.to_string().contains("schema_version"), "{error}");
    }

    #[test]
    fn phrases_are_matched_longest_first() {
        let speech = load_chemistry_speech(MINIMAL_ALIASES).unwrap();
        // «из» is a one-word opening, but a longer phrase starting at the same
        // position must win when one exists.
        let words = crate::normalize::words("реагирует с серной кислотой");
        assert_eq!(
            speech.connective_at(&words, 0),
            Some((ChemConnective::JoinReagent, 2))
        );
        let lengths: Vec<usize> = speech
            .connectives
            .iter()
            .map(|(phrase, _)| phrase.len())
            .collect();
        assert!(
            lengths.windows(2).all(|pair| pair[0] >= pair[1]),
            "the table must be stored longest-first: {lengths:?}"
        );
    }

    #[test]
    fn the_parser_really_reads_the_yaml_and_holds_no_second_list() {
        // Every arrow the chemistry parser accepts must be present in the
        // loaded table. If someone re-adds a hard-coded list in Rust, one of
        // these will parse while the data file says nothing about it.
        let lexicon = Lexicon::builtin();
        let speech = &lexicon.chemistry_speech;
        for phrase in speech.phrases_for(ChemConnective::Forward) {
            let spoken = format!("вода {phrase} кислород");
            let parsed = crate::interpret(
                &spoken,
                crate::InterpretOptions {
                    domain: crate::Domain::Chemistry,
                    allow_shortcuts: false,
                },
            );
            assert!(parsed.confidence > 0.0, "{spoken}");
        }
        // A phrase that is *not* in the file is not an arrow, however
        // plausible it sounds.
        assert!(!speech
            .phrases_for(ChemConnective::Forward)
            .iter()
            .any(|phrase| phrase == "уходит в"));
        let parsed = crate::interpret(
            "вода уходит в кислород",
            crate::InterpretOptions {
                domain: crate::Domain::Chemistry,
                allow_shortcuts: false,
            },
        );
        assert_eq!(parsed.confidence, 0.0);
    }

    #[test]
    fn a_reaction_noun_alone_is_not_an_arrow() {
        for spoken in [
            "реакция идёт быстрее при нагревании",
            "между водородом и кислородом идёт реакция",
        ] {
            let parsed = crate::interpret(
                spoken,
                crate::InterpretOptions {
                    domain: crate::Domain::Chemistry,
                    allow_shortcuts: false,
                },
            );
            assert_eq!(parsed.confidence, 0.0, "{spoken}");
        }
    }
}

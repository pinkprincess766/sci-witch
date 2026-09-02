//! Versioned YAML lexicons. Built-in data is embedded; unknown future schemas fail closed.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::ast::Formula;
use crate::dimension::Dimension;
use crate::error::{Error, Result};
use crate::formula::{parse_equation_str, parse_formula_str};
use crate::normalize::normalize_word;

pub const SUPPORTED_SCHEMA: u32 = 1;

/// `units.yaml` moved to its own schema when every unit gained a required
/// `dimension`. Only this exact version is accepted: schema 1 carries no
/// dimensions at all, and a newer one may redefine what they mean.
pub const UNITS_SCHEMA: u32 = 2;

const ELEMENTS_YAML: &str = include_str!("../data/domains/chemistry/elements.yaml");
const IONS_YAML: &str = include_str!("../data/domains/chemistry/ions.yaml");
const SUBSTANCES_YAML: &str = include_str!("../data/domains/chemistry/substances.yaml");
const GREEK_YAML: &str = include_str!("../data/domains/common/greek.yaml");
const SYMBOLS_YAML: &str = include_str!("../data/domains/common/symbols.yaml");
const UNITS_YAML: &str = include_str!("../data/domains/physics/units.yaml");
const CHEM101_YAML: &str = include_str!("../data/courses/chem101.yaml");

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
        let mut best: Option<(Formula, usize)> = None;
        for item in &self.substances {
            for name in &item.names {
                let nw: Vec<String> = name.split_whitespace().map(|s| s.to_string()).collect();
                if nw.is_empty() || i + nw.len() > words.len() {
                    continue;
                }
                if words[i..i + nw.len()] == nw[..] {
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
    };
    load_elements(&mut lex, ELEMENTS_YAML)?;
    load_ions(&mut lex, IONS_YAML)?;
    load_substances(&mut lex, SUBSTANCES_YAML)?;
    load_greek(&mut lex, GREEK_YAML)?;
    load_symbols(&mut lex, SYMBOLS_YAML)?;
    load_units(&mut lex, UNITS_YAML)?;
    load_course(&mut lex, CHEM101_YAML)?;
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
}

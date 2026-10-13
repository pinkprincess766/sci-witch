//! How far apart two scientific interpretations are.
//!
//! Until now the lab could say only *whether* an answer matched the corpus,
//! and — through `classify_severity` — *what kind* of thing had changed.
//! Neither says how far the answer moved. One differing field and twelve
//! landed in the same class, and `H₂O → H₂O₂` cost exactly as much as
//! `H₂O → D₂O`.
//!
//! This is a weighted tree edit distance over the typed AST: the smallest
//! total cost of turning one interpretation into the other by inserting,
//! deleting and relabelling nodes. Weights live in
//! `research/schema/ast-distance-v1.json`, so what this project considers a
//! serious change is data with a version, not constants in Rust.
//!
//! # Why it is a metric, and why that matters
//!
//! Asymmetric weights were considered and rejected. `d(a,b) ≠ d(b,a)` would
//! make this a quasi-metric: balls stop being balls, and any later
//! clustering, caching or nearest-neighbour search over interpretations
//! would be quietly wrong. The scientific asymmetry that was actually wanted
//! — a coefficient mattering more than a bracket — is expressed by
//! **symmetric per-field weights** instead, which costs nothing and keeps
//! the axioms.
//!
//! Tree edit distance is a metric whenever the elementary cost function γ is
//! a metric on labels ∪ {ε} (Tai, 1979). Three constraints make ours one,
//! and [`Weights::load`] refuses a file that breaks them:
//!
//! * **γ(a,b) = 0 ⟺ a = b** — a relabel between distinct labels always
//!   costs something, because every weight is positive.
//! * **symmetry** — no cost anywhere depends on the direction.
//! * **triangle inequality** — the relabel cost is capped at
//!   `kind_change`, and `kind_change ≤ delete + insert`. The cap is what
//!   makes the awkward case work: if `a` and `c` are the same kind but `b`
//!   is not, then `γ(a,c) ≤ kind_change ≤ γ(a,b) + γ(b,c) = 2·kind_change`.
//!
//! The axioms are not only argued, they are checked: the property tests run
//! every pair and every triple of a set of real corpus ASTs.
//!
//! # What it deliberately does not do
//!
//! It does not tell you *what* changed. A distance is a magnitude; the
//! categorical question — was this a coefficient, an element, a bracket —
//! is what [`crate::metrics::classify_severity`] answers, from the severity
//! classes in the same file. Collapsing the two into one number would lose
//! whichever question was not asked.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

pub const DISTANCE_SCHEMA_VERSION: u32 = 1;

const WEIGHTS_JSON: &str = include_str!("../../../research/schema/ast-distance-v1.json");

#[derive(Debug, Deserialize)]
struct WeightsFile {
    distance_schema_version: u32,
    delete: f64,
    insert: f64,
    kind_change: f64,
    default: FieldCost,
    fields: BTreeMap<String, FieldCost>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FieldCost {
    pub weight: f64,
    pub severity: String,
}

#[derive(Debug)]
pub struct Weights {
    delete: f64,
    insert: f64,
    kind_change: f64,
    default: FieldCost,
    fields: BTreeMap<String, FieldCost>,
}

impl Weights {
    /// Parses and checks the weight file. The checks are the metric axioms:
    /// a file that would make this not a distance is refused rather than
    /// used to produce numbers nobody could reason about.
    pub fn load(json: &str) -> Result<Weights, String> {
        let file: WeightsFile =
            serde_json::from_str(json).map_err(|error| format!("ast-distance: {error}"))?;
        if file.distance_schema_version != DISTANCE_SCHEMA_VERSION {
            return Err(format!(
                "distance_schema_version {} не поддерживается (нужна {DISTANCE_SCHEMA_VERSION})",
                file.distance_schema_version
            ));
        }
        for (name, cost) in [
            ("delete", file.delete),
            ("insert", file.insert),
            ("kind_change", file.kind_change),
        ] {
            if !(cost.is_finite() && cost > 0.0) {
                return Err(format!(
                    "{name} должен быть положительным числом, а не {cost}"
                ));
            }
        }
        // Without this the triangle inequality fails: relabelling would be
        // cheaper than the delete-then-insert it is compared against.
        if file.kind_change > file.delete + file.insert {
            return Err(format!(
                "kind_change {} превышает delete + insert = {}; при этом неравенство треугольника нарушается",
                file.kind_change,
                file.delete + file.insert
            ));
        }
        // Asymmetric edit costs would make d(a,b) != d(b,a).
        if (file.delete - file.insert).abs() > f64::EPSILON {
            return Err(format!(
                "delete {} и insert {} различаются; несимметричная стоимость даёт квазиметрику, а не метрику",
                file.delete, file.insert
            ));
        }
        for (field, cost) in file
            .fields
            .iter()
            .chain([(&"default".to_string(), &file.default)])
        {
            if !(cost.weight.is_finite() && cost.weight > 0.0) {
                return Err(format!(
                    "вес поля {field} должен быть положительным, иначе разные AST оказались бы на нулевом расстоянии"
                ));
            }
            if !matches!(cost.severity.as_str(), "S0" | "S1" | "S2" | "S3" | "S4") {
                return Err(format!("поле {field}: неизвестный класс {}", cost.severity));
            }
        }
        Ok(Weights {
            delete: file.delete,
            insert: file.insert,
            kind_change: file.kind_change,
            default: file.default,
            fields: file.fields,
        })
    }

    /// The cost table shipped with this build.
    pub fn builtin() -> &'static Weights {
        static WEIGHTS: OnceLock<Weights> = OnceLock::new();
        WEIGHTS.get_or_init(|| {
            Weights::load(WEIGHTS_JSON).expect("the shipped ast-distance file must be valid")
        })
    }

    /// Looks up a field, preferring the variant-qualified name.
    ///
    /// `kind` means one thing inside `Group` — round or square brackets
    /// around the same expression — and quite another inside `Function`,
    /// where it is the difference between a sine and a cosine. Keying on the
    /// bare field name would have to price both the same.
    pub fn field(&self, variant: &str, field: &str) -> &FieldCost {
        self.fields
            .get(&format!("{variant}.{field}"))
            .or_else(|| self.fields.get(field))
            .unwrap_or(&self.default)
    }

    pub fn severity_of(&self, variant: &str, field: &str) -> &str {
        &self.field(variant, field).severity
    }
}

/// One node of the flattened tree: what kind of thing it is, plus the scalar
/// fields that belong to it. Children are held by the arena, not here.
#[derive(Clone, Debug, PartialEq)]
struct Label {
    kind: String,
    scalars: BTreeMap<String, String>,
}

impl Label {
    fn new(kind: impl Into<String>) -> Self {
        Label {
            kind: kind.into(),
            scalars: BTreeMap::new(),
        }
    }
}

/// A tree in left-to-right postorder, which is the order Zhang–Shasha wants.
struct Flat {
    labels: Vec<Label>,
    /// `leftmost[i]` is the postorder index of the leftmost leaf descendant
    /// of node `i` — the only structural fact the algorithm needs.
    leftmost: Vec<usize>,
}

impl Flat {
    fn len(&self) -> usize {
        self.labels.len()
    }

    /// Nodes that are not the leftmost child of their parent, plus the root:
    /// the only subtrees whose distance has to be computed outright.
    fn keyroots(&self) -> Vec<usize> {
        let mut seen: BTreeMap<usize, usize> = BTreeMap::new();
        for index in 0..self.len() {
            seen.insert(self.leftmost[index], index);
        }
        let mut roots: Vec<usize> = seen.into_values().collect();
        roots.sort_unstable();
        roots
    }
}

/// Turns a serialised AST into a labelled tree.
///
/// Working from the serde form rather than matching every variant by hand is
/// deliberate: a new AST variant is measured the day it is added instead of
/// silently costing nothing until somebody remembers this file. The price is
/// that field *names* matter, which is why
/// `every_field_the_corpus_produces_has_a_weight` exists.
fn flatten(value: &Value) -> Flat {
    let mut flat = Flat {
        labels: Vec::new(),
        leftmost: Vec::new(),
    };
    build(value, &mut flat);
    flat
}

/// Appends the postorder of `value` to `flat` and returns its root index.
///
/// The enclosing variant is not carried down: a field is priced by the kind
/// of the node it sits on, which [`relabel`] reads from that node's own
/// label. A plain struct such as `Species` has kind `{}`, so its fields fall
/// back to their bare names — which is right, because there is no variant to
/// qualify them with.
fn build(value: &Value, flat: &mut Flat) -> usize {
    match value {
        Value::Object(map) if map.len() == 1 => {
            // An externally tagged enum variant: `{"Atom": {...}}`.
            let (name, inner) = map.iter().next().expect("len == 1");
            let mut label = Label::new(name.clone());
            let mut children = Vec::new();
            match inner {
                Value::Object(entries) => {
                    for (field, child) in entries {
                        if is_scalar(child) {
                            label.scalars.insert(field.clone(), scalar_text(child));
                        } else {
                            children.push(build(child, flat));
                        }
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        children.push(build(item, flat));
                    }
                }
                // A newtype variant: its payload is keyed by the variant
                // name, so `Number` and `Text` can be priced by name.
                scalar => {
                    label.scalars.insert(name.clone(), scalar_text(scalar));
                }
            }
            push(label, children, flat)
        }
        Value::Object(map) => {
            // A plain struct: `{"coefficient":1,"formula":{…}}`.
            let mut label = Label::new("{}");
            let mut children = Vec::new();
            for (field, child) in map {
                if is_scalar(child) {
                    label.scalars.insert(field.clone(), scalar_text(child));
                } else {
                    children.push(build(child, flat));
                }
            }
            push(label, children, flat)
        }
        Value::Array(items) => {
            let children: Vec<usize> = items.iter().map(|item| build(item, flat)).collect();
            push(Label::new("[]"), children, flat)
        }
        // A bare scalar at this position is a unit enum variant such as
        // `"Infinity"`; its text is what it is, so it becomes the kind.
        scalar => push(Label::new(scalar_text(scalar)), Vec::new(), flat),
    }
}

fn push(label: Label, children: Vec<usize>, flat: &mut Flat) -> usize {
    let index = flat.labels.len();
    let leftmost = children
        .first()
        .map_or(index, |first| flat.leftmost[*first]);
    flat.labels.push(label);
    flat.leftmost.push(leftmost);
    index
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Object(_) | Value::Array(_))
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Cost of turning label `a` into label `b`.
fn relabel(a: &Label, b: &Label, weights: &Weights) -> f64 {
    if a == b {
        return 0.0;
    }
    if a.kind != b.kind {
        return weights.kind_change;
    }
    let mut total = 0.0;
    let mut keys: Vec<&String> = a.scalars.keys().chain(b.scalars.keys()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        if a.scalars.get(key) != b.scalars.get(key) {
            total += weights.field(&a.kind, key).weight;
        }
    }
    // Capped, so that becoming a different kind of node is never cheaper
    // than staying the same kind — and so the triangle inequality holds.
    total.min(weights.kind_change)
}

/// Zhang–Shasha ordered tree edit distance.
///
/// The trees here have tens of nodes, so the classic algorithm is used
/// as published rather than one of the faster variants: it is the one whose
/// correctness is easiest to check against the paper.
fn tree_edit_distance(a: &Flat, b: &Flat, weights: &Weights) -> f64 {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as f64 * weights.insert;
    }
    if m == 0 {
        return n as f64 * weights.delete;
    }
    let mut tree = vec![vec![0.0f64; m]; n];
    for &i in &a.keyroots() {
        for &j in &b.keyroots() {
            forest_distance(a, b, i, j, &mut tree, weights);
        }
    }
    tree[n - 1][m - 1]
}

/// Fills `tree[..][..]` for every subtree pair inside the forests rooted at
/// `i` and `j`. Indices are shifted by one so that "the empty forest" has a
/// place in the table.
fn forest_distance(
    a: &Flat,
    b: &Flat,
    i: usize,
    j: usize,
    tree: &mut [Vec<f64>],
    weights: &Weights,
) {
    let (li, lj) = (a.leftmost[i], b.leftmost[j]);
    let rows = i - li + 2;
    let cols = j - lj + 2;
    let mut forest = vec![vec![0.0f64; cols]; rows];
    for row in 1..rows {
        forest[row][0] = forest[row - 1][0] + weights.delete;
    }
    for col in 1..cols {
        forest[0][col] = forest[0][col - 1] + weights.insert;
    }
    for row in 1..rows {
        for col in 1..cols {
            let (di, dj) = (li + row - 1, lj + col - 1);
            let delete = forest[row - 1][col] + weights.delete;
            let insert = forest[row][col - 1] + weights.insert;
            if a.leftmost[di] == li && b.leftmost[dj] == lj {
                // Both are whole subtrees: this cell is also their tree
                // distance, so it is recorded for later reuse.
                let change =
                    forest[row - 1][col - 1] + relabel(&a.labels[di], &b.labels[dj], weights);
                forest[row][col] = delete.min(insert).min(change);
                tree[di][dj] = forest[row][col];
            } else {
                let inner_row = a.leftmost[di] - li;
                let inner_col = b.leftmost[dj] - lj;
                let change = forest[inner_row][inner_col] + tree[di][dj];
                forest[row][col] = delete.min(insert).min(change);
            }
        }
    }
}

/// Distance between two interpretations, in the units of the weight file.
///
/// Zero exactly when the two serialise identically. Symmetric, and obeys the
/// triangle inequality — see the module documentation for why that was worth
/// giving up asymmetric weights for.
pub fn distance(a: &Value, b: &Value) -> f64 {
    distance_with(a, b, Weights::builtin())
}

pub fn distance_with(a: &Value, b: &Value, weights: &Weights) -> f64 {
    if a == b {
        return 0.0;
    }
    tree_edit_distance(&flatten(a), &flatten(b), weights)
}

/// Distance between two AST nodes.
pub fn distance_nodes(a: &sciwhisper_core::Node, b: &sciwhisper_core::Node) -> f64 {
    let (Ok(a), Ok(b)) = (serde_json::to_value(a), serde_json::to_value(b)) else {
        // A node that cannot be serialised cannot be compared; reporting a
        // zero would claim they are the same.
        return f64::NAN;
    };
    distance(&a, &b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_core::{interpret, Domain, InterpretOptions, Node};

    fn ast(spoken: &str) -> Value {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain: Domain::Auto,
                ..Default::default()
            },
        );
        assert!(
            result.confidence > 0.0 && !matches!(result.ast, Node::Text(_)),
            "{spoken:?} did not compile, so it cannot stand in for an interpretation"
        );
        serde_json::to_value(&result.ast).unwrap()
    }

    /// A spread of real interpretations, used for the axiom checks. Chosen
    /// to mix domains, depths and shapes rather than to be convenient.
    fn corpus() -> Vec<(&'static str, Value)> {
        [
            "вода",
            "серная кислота",
            "перекись водорода",
            "метан",
            "гидроксид железа два",
            "гидроксид железа три",
            "икс в квадрате",
            "икс в кубе",
            "синус икс",
            "косинус икс",
            "икс делённое на игрек",
            "два плюс два",
            "корень из икс",
            "сто паскалей",
            "три кельвина",
            "икс индекс один",
            "модуль икс",
            "два икс плюс три равно нулю",
        ]
        .into_iter()
        .map(|spoken| (spoken, ast(spoken)))
        .collect()
    }

    #[test]
    fn the_shipped_weight_file_is_a_valid_metric_definition() {
        let weights = Weights::builtin();
        assert!(weights.kind_change <= weights.delete + weights.insert);
        assert_eq!(weights.delete, weights.insert);
    }

    /// A number and the string with the same digits are different values,
    /// as a field and as a leaf, so they must not be at distance zero.
    #[test]
    fn a_number_and_its_string_spelling_are_not_at_distance_zero() {
        let pairs = [
            (serde_json::json!({"Number": 1}), serde_json::json!({"Number": "1"})),
            (serde_json::json!(1), serde_json::json!("1")),
            (serde_json::json!(true), serde_json::json!("true")),
        ];
        for (a, b) in &pairs {
            assert_ne!(a, b);
            assert!(distance(a, b) > 0.0, "{a} and {b} are at distance zero");
        }
    }

    /// The axioms are argued in the module documentation; here they are
    /// checked, on every pair and every triple of real interpretations.
    #[test]
    fn distance_is_zero_exactly_when_the_interpretations_are_the_same() {
        let corpus = corpus();
        for (name, value) in &corpus {
            assert_eq!(distance(value, value), 0.0, "{name}");
        }
        for (a_name, a) in &corpus {
            for (b_name, b) in &corpus {
                if a == b {
                    continue;
                }
                assert!(
                    distance(a, b) > 0.0,
                    "{a_name} and {b_name} are different but at distance zero"
                );
            }
        }
    }

    #[test]
    fn distance_is_symmetric() {
        let corpus = corpus();
        for (a_name, a) in &corpus {
            for (b_name, b) in &corpus {
                let there = distance(a, b);
                let back = distance(b, a);
                assert!(
                    (there - back).abs() < 1e-9,
                    "{a_name} → {b_name} is {there}, back is {back}"
                );
            }
        }
    }

    #[test]
    fn distance_obeys_the_triangle_inequality() {
        let corpus = corpus();
        for (a_name, a) in &corpus {
            for (b_name, b) in &corpus {
                let ab = distance(a, b);
                for (c_name, c) in &corpus {
                    let ac = distance(a, c);
                    let bc = distance(b, c);
                    assert!(
                        ac <= ab + bc + 1e-9,
                        "d({a_name},{c_name})={ac} > d({a_name},{b_name})+d({b_name},{c_name})={}",
                        ab + bc
                    );
                }
            }
        }
    }

    /// The whole point of weighting: a near miss must come out nearer than a
    /// different substance. These are orderings, not magnitudes, because an
    /// ordering is what the metric is for and a magnitude would just be this
    /// file's constants read back.
    #[test]
    fn a_near_miss_is_nearer_than_a_different_answer() {
        let water = ast("вода");
        let peroxide = ast("перекись водорода");
        let sulfuric = ast("серная кислота");
        assert!(
            distance(&water, &peroxide) < distance(&water, &sulfuric),
            "H₂O→H₂O₂ must cost less than H₂O→H₂SO₄"
        );

        let iron_two = ast("гидроксид железа два");
        let iron_three = ast("гидроксид железа три");
        assert!(distance(&iron_two, &iron_three) < distance(&iron_two, &sulfuric));

        let squared = ast("икс в квадрате");
        let cubed = ast("икс в кубе");
        let sine = ast("синус икс");
        assert!(distance(&squared, &cubed) < distance(&squared, &sine));
    }

    /// The ordering the ADR asked for: presentation ≪ quantity < identity.
    #[test]
    fn presentation_costs_less_than_quantity_and_quantity_less_than_identity() {
        let weights = Weights::builtin();
        let bracket = weights.field("Group", "kind").weight;
        let coefficient = weights.field("Species", "coefficient").weight;
        let element = weights.field("Atom", "symbol").weight;
        assert!(bracket < coefficient, "{bracket} !< {coefficient}");
        assert!(coefficient < element, "{coefficient} !< {element}");
    }

    /// `kind` is a bracket style in one place and a trigonometric function in
    /// another. Pricing them the same is exactly the defect that keying on
    /// bare field names would have carried over.
    #[test]
    fn the_same_field_name_is_priced_by_where_it_appears() {
        let weights = Weights::builtin();
        assert!(
            weights.field("Group", "kind").weight < weights.field("Function", "kind").weight,
            "a bracket style must not cost what a sine-versus-cosine costs"
        );
    }

    #[test]
    fn a_deeper_answer_is_further_away_than_a_shallower_one() {
        // Inserting nodes costs; a longer expression is further from a short
        // one than another short one is.
        let two = ast("два плюс два");
        let equation = ast("два икс плюс три равно нулю");
        let four = ast("икс в квадрате");
        assert!(distance(&two, &equation) > distance(&two, &four));
    }

    #[test]
    fn a_weight_file_that_would_not_be_a_metric_is_refused() {
        let base: serde_json::Value = serde_json::from_str(WEIGHTS_JSON).unwrap();

        let mut asymmetric = base.clone();
        asymmetric["insert"] = serde_json::json!(2.0);
        let error = Weights::load(&asymmetric.to_string()).unwrap_err();
        assert!(error.contains("квазиметрику"), "{error}");

        let mut expensive = base.clone();
        expensive["kind_change"] = serde_json::json!(5.0);
        let error = Weights::load(&expensive.to_string()).unwrap_err();
        assert!(error.contains("треугольника"), "{error}");

        let mut free = base.clone();
        free["fields"]["symbol"]["weight"] = serde_json::json!(0.0);
        let error = Weights::load(&free.to_string()).unwrap_err();
        assert!(error.contains("положительным"), "{error}");

        let mut future = base;
        future["distance_schema_version"] = serde_json::json!(9);
        assert!(Weights::load(&future.to_string()).is_err());
    }

    #[test]
    fn an_empty_document_is_at_a_finite_distance_from_a_formula() {
        let empty = serde_json::to_value(Node::Document(vec![])).unwrap();
        let water = ast("вода");
        let d = distance(&empty, &water);
        assert!(d.is_finite() && d > 0.0, "{d}");
        assert_eq!(distance(&empty, &empty), 0.0);
    }
}

/// Zhang–Shasha against the definition. The reference is the textbook forest
/// recurrence with no tricks, so a disagreement points at the fast algorithm.
#[cfg(test)]
mod brute {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    /// Forest edit distance straight from Tai's definition. A forest is a
    /// half-open range of postorder indices; its rightmost root is the last
    /// index, and that root's subtree starts at `leftmost`. Nodes, labels and
    /// `relabel` are the production ones, so only the tree algorithm differs.
    struct Reference<'a> {
        a: &'a Flat,
        b: &'a Flat,
        weights: &'a Weights,
        memo: HashMap<(usize, usize, usize, usize), f64>,
    }

    impl Reference<'_> {
        fn forest(&mut self, a_lo: usize, a_hi: usize, b_lo: usize, b_hi: usize) -> f64 {
            if a_lo == a_hi {
                return (b_hi - b_lo) as f64 * self.weights.insert;
            }
            if b_lo == b_hi {
                return (a_hi - a_lo) as f64 * self.weights.delete;
            }
            let key = (a_lo, a_hi, b_lo, b_hi);
            if let Some(&known) = self.memo.get(&key) {
                return known;
            }
            let (a_root, b_root) = (a_hi - 1, b_hi - 1);
            let (a_first, b_first) = (self.a.leftmost[a_root], self.b.leftmost[b_root]);
            let delete = self.forest(a_lo, a_root, b_lo, b_hi) + self.weights.delete;
            let insert = self.forest(a_lo, a_hi, b_lo, b_root) + self.weights.insert;
            let left_siblings = self.forest(a_lo, a_first, b_lo, b_first);
            let children = self.forest(a_first, a_root, b_first, b_root);
            let roots = relabel(&self.a.labels[a_root], &self.b.labels[b_root], self.weights);
            let matched = left_siblings + children + roots;
            let best = delete.min(insert).min(matched);
            self.memo.insert(key, best);
            best
        }
    }

    fn reference(a: &Value, b: &Value) -> f64 {
        let (a, b) = (flatten(a), flatten(b));
        let mut reference = Reference {
            a: &a,
            b: &b,
            weights: Weights::builtin(),
            memo: HashMap::new(),
        };
        reference.forest(0, a.len(), 0, b.len())
    }

    /// Zhang–Shasha without the `a == b` shortcut of `distance_with`, so the
    /// identity axiom is tested on the algorithm and not on the shortcut.
    fn fast(a: &Value, b: &Value) -> f64 {
        tree_edit_distance(&flatten(a), &flatten(b), Weights::builtin())
    }

    /// Knuth's MMIX LCG; fixed seed, so a failure reproduces.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    /// A random AST of at most `budget` nodes, in the serde shape `flatten`
    /// reads. Four kinds, whose scalar fields cost 0.05, 0.2, 0.8 and 1.0 in
    /// the shipped weight file, so relabels are not all the same price.
    fn random_tree(rng: &mut Lcg, budget: usize) -> Value {
        let choices = match budget {
            1 => 1,
            2 => 3,
            _ => 4,
        };
        match rng.below(choices) {
            0 => {
                let symbol = ["H", "O"][rng.below(2)];
                let count = 1 + rng.below(2);
                let marker = ["aq", "s"][rng.below(2)];
                json!({"Atom": {"symbol": symbol, "count": count, "marker": marker}})
            }
            1 => {
                let kind = ["round", "square"][rng.below(2)];
                let inner = random_tree(rng, budget - 1);
                json!({"Group": {"kind": kind, "inner": inner}})
            }
            2 => {
                let kind = ["sin", "cos"][rng.below(2)];
                let arg = random_tree(rng, budget - 1);
                json!({"Function": {"kind": kind, "arg": arg}})
            }
            _ => {
                // Two or three children, each at least one node.
                let count = if budget >= 4 { 2 + rng.below(2) } else { 2 };
                let mut remaining = budget - 1;
                let mut children = Vec::new();
                for placed in 0..count {
                    let still_to_place = count - placed - 1;
                    let share = 1 + rng.below(remaining - still_to_place);
                    children.push(random_tree(rng, share));
                    remaining -= share;
                }
                json!({ "Sum": children })
            }
        }
    }

    const MAX_NODES: usize = 6;
    const SEED: u64 = 0x5eed_5eed;
    const PAIRS: usize = 300;
    const TRIPLES: usize = 100;

    fn small_tree(rng: &mut Lcg) -> Value {
        let budget = 1 + rng.below(MAX_NODES);
        random_tree(rng, budget)
    }

    /// Hand-computed values, so the reference is not trusted merely for
    /// agreeing with the code it checks.
    #[test]
    fn the_reference_matches_hand_computed_distances() {
        let atom = |symbol: &str, count: u32| json!({"Atom": {"symbol": symbol, "count": count, "marker": "aq"}});
        let group = |kind: &str, inner: Value| json!({"Group": {"kind": kind, "inner": inner}});
        let h = atom("H", 1);
        let cases = [
            (h.clone(), atom("O", 1), 1.0),
            (h.clone(), atom("H", 2), 0.8),
            (group("round", h.clone()), group("square", h.clone()), 0.05),
            (group("round", h.clone()), h.clone(), 1.0),
            (json!({"Sum": [h.clone(), h.clone()]}), h.clone(), 2.0),
        ];
        for (a, b, expected) in cases {
            assert!((reference(&a, &b) - expected).abs() < 1e-9, "{a} vs {b}");
            assert!((fast(&a, &b) - expected).abs() < 1e-9, "{a} vs {b}");
        }
    }

    #[test]
    fn zhang_shasha_equals_the_definition_on_random_small_trees() {
        let mut rng = Lcg(SEED);
        let mut largest = 0;
        let mut seen_values: Vec<f64> = Vec::new();
        for pair in 0..PAIRS {
            let (a, b) = (small_tree(&mut rng), small_tree(&mut rng));
            largest = largest.max(flatten(&a).len()).max(flatten(&b).len());
            let (expected, actual) = (reference(&a, &b), fast(&a, &b));
            assert!(
                (expected - actual).abs() < 1e-9,
                "pair {pair}: reference {expected}, Zhang–Shasha {actual}\n  a = {a}\n  b = {b}"
            );
            if !seen_values.iter().any(|v| (v - expected).abs() < 1e-9) {
                seen_values.push(expected);
            }
        }
        // A sample that never reaches the size limit, or whose distances are
        // all alike, would pass for the wrong reason.
        assert_eq!(largest, MAX_NODES);
        assert!(
            seen_values.len() >= 10,
            "only {} distinct distances",
            seen_values.len()
        );
    }

    #[test]
    fn random_small_trees_obey_the_metric_axioms() {
        let mut rng = Lcg(SEED ^ 1);
        for triple in 0..TRIPLES {
            let (a, b, c) = (
                small_tree(&mut rng),
                small_tree(&mut rng),
                small_tree(&mut rng),
            );
            let context = format!("triple {triple}\n  a = {a}\n  b = {b}\n  c = {c}");
            assert_eq!(fast(&a, &a), 0.0, "{context}");
            let (ab, ba) = (fast(&a, &b), fast(&b, &a));
            assert!((ab - ba).abs() < 1e-9, "{ab} vs {ba}\n{context}");
            if a != b {
                assert!(ab > 0.0, "different trees at distance zero\n{context}");
            }
            let (ac, bc) = (fast(&a, &c), fast(&b, &c));
            assert!(ac <= ab + bc + 1e-9, "d(a,c)={ac} > {ab} + {bc}\n{context}");
        }
    }

    /// The only way a distinct pair could sit at distance zero is a free
    /// relabel. Check every pair of labels the generator produces.
    #[test]
    fn no_two_different_labels_relabel_for_free() {
        let mut rng = Lcg(SEED ^ 2);
        let mut labels: Vec<Label> = Vec::new();
        for _ in 0..PAIRS {
            for label in flatten(&small_tree(&mut rng)).labels {
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
        }
        let mut cheapest = f64::INFINITY;
        for a in &labels {
            for b in &labels {
                if a != b {
                    cheapest = cheapest.min(relabel(a, b, Weights::builtin()));
                }
            }
        }
        // 8 atoms (2 symbols x 2 counts x 2 markers), 2 groups, 2 functions, 1 sum.
        assert_eq!(labels.len(), 13);
        eprintln!(
            "{} labels, cheapest relabel between different ones: {cheapest}",
            labels.len()
        );
        assert!(cheapest > 0.0, "{cheapest}");
    }
}

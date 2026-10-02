//! The grammar documents under `docs/grammar/` are checked against the code.
//!
//! `math.ebnf` and `chem.ebnf` claim to describe what the hand-written
//! parsers accept. A claim in a document is checked by nobody, so this file
//! reads the documents back and tests them:
//!
//! * the EBNF is parsed by a small parser written here, and every name that
//!   is used is defined, every rule is reachable and carries a reference to
//!   code that exists;
//! * the terminals are the `Tok` variants of `parser/math.rs`, and the spoken
//!   phrases are the lists in `operators.yaml` and `aliases.yaml`;
//! * FIRST and FOLLOW are computed from the EBNF, and so is every LL(1)
//!   conflict. `docs/grammar/ANALYSIS_RU.md` embeds the computed tables
//!   between markers and the test compares them character by character, so a
//!   changed grammar fails here until the analysis is re-read;
//! * every phrase the analysis quotes together with the compiler's output is
//!   run again, so an output pasted into the document cannot go stale.
//!
//! Each check also has a test that tries to pass it wrongly: a grammar edited
//! so that a conflict disappears, a name used and not defined, a document
//! with a changed table.
//!
//! To re-print the blocks the document embeds:
//! `GRAMMAR_PRINT=1 cargo test -p sciwhisper-core --test grammar_first_follow -- --nocapture`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sciwhisper_core::lexicon::Lexicon;
use sciwhisper_core::numbers::NumberLex;
use sciwhisper_core::{
    interpret_utterance, render, Domain, Renderer, UtteranceMode, UtteranceOptions,
};

/// How far a quoted line number may be from the real one before the document
/// counts as stale.
///
/// A function reference names the function and a line inside it. Lines drift
/// whenever somebody edits above them, and failing on every such edit would
/// teach people to ignore this test; the function name is what must stay
/// exact. The tolerance is checked in
/// `a_reference_to_a_missing_function_is_refused`.
const LINE_DRIFT_TOLERANCE: usize = 40;

// ======================================================================
// EBNF: parser
// ======================================================================

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Seq(Vec<Expr>),
    Alt(Vec<Expr>),
    Opt(Box<Expr>),
    Rep(Box<Expr>),
    Name(String),
    Lit(String),
    Special(String),
}

#[derive(Clone, Debug)]
struct Rule {
    name: String,
    body: Expr,
    comments: Vec<String>,
    line: usize,
}

#[derive(Clone, Debug)]
struct Grammar {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq)]
enum Tk {
    Ident(String),
    Str(String),
    Special(String),
    Punct(char),
}

struct Lexed {
    tokens: Vec<(Tk, usize)>,
    /// (number of tokens produced before the comment, text)
    comments: Vec<(usize, String)>,
}

fn lex(source: &str) -> Result<Lexed, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut line = 1;
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '(' && chars.get(i + 1) == Some(&'*') {
            let start_line = line;
            let mut j = i + 2;
            let mut text = String::new();
            loop {
                if j + 1 >= chars.len() {
                    return Err(format!("line {start_line}: unterminated comment"));
                }
                if chars[j] == '*' && chars[j + 1] == ')' {
                    break;
                }
                if chars[j] == '\n' {
                    line += 1;
                }
                text.push(chars[j]);
                j += 1;
            }
            comments.push((tokens.len(), text));
            i = j + 2;
        } else if c == '"' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < chars.len() && chars[j] != '"' {
                if chars[j] == '\n' {
                    return Err(format!("line {line}: a literal runs past the end of line"));
                }
                text.push(chars[j]);
                j += 1;
            }
            if j >= chars.len() {
                return Err(format!("line {line}: unterminated literal"));
            }
            tokens.push((Tk::Str(text), line));
            i = j + 1;
        } else if c == '?' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < chars.len() && chars[j] != '?' {
                if chars[j] == '\n' {
                    line += 1;
                }
                text.push(chars[j]);
                j += 1;
            }
            if j >= chars.len() {
                return Err(format!("line {line}: unterminated special sequence"));
            }
            tokens.push((Tk::Special(text.trim().to_string()), line));
            i = j + 1;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            let mut text = String::new();
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                text.push(chars[j]);
                j += 1;
            }
            tokens.push((Tk::Ident(text), line));
            i = j;
        } else if "=;|()[]{}".contains(c) {
            tokens.push((Tk::Punct(c), line));
            i += 1;
        } else {
            return Err(format!("line {line}: unexpected character {c:?}"));
        }
    }
    Ok(Lexed { tokens, comments })
}

struct Parser {
    tokens: Vec<(Tk, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tk> {
        self.tokens.get(self.pos).map(|(t, _)| t)
    }

    fn line(&self) -> usize {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .map_or(0, |(_, l)| *l)
    }

    fn eat_punct(&mut self, p: char) -> bool {
        if self.peek() == Some(&Tk::Punct(p)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: char) -> Result<(), String> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(format!("line {}: expected {p:?}", self.line()))
        }
    }

    fn alternatives(&mut self) -> Result<Expr, String> {
        let mut alts = vec![self.sequence()?];
        while self.eat_punct('|') {
            alts.push(self.sequence()?);
        }
        Ok(if alts.len() == 1 {
            alts.remove(0)
        } else {
            Expr::Alt(alts)
        })
    }

    fn sequence(&mut self) -> Result<Expr, String> {
        let mut items: Vec<Expr> = Vec::new();
        loop {
            let item = match self.peek() {
                Some(Tk::Ident(n)) => {
                    let n = n.clone();
                    self.pos += 1;
                    Expr::Name(n)
                }
                Some(Tk::Str(s)) => {
                    let s = s.clone();
                    self.pos += 1;
                    Expr::Lit(s)
                }
                Some(Tk::Special(s)) => {
                    let s = s.clone();
                    self.pos += 1;
                    Expr::Special(s)
                }
                Some(Tk::Punct('(')) => {
                    self.pos += 1;
                    let inner = self.alternatives()?;
                    self.expect_punct(')')?;
                    inner
                }
                Some(Tk::Punct('[')) => {
                    self.pos += 1;
                    let inner = self.alternatives()?;
                    self.expect_punct(']')?;
                    Expr::Opt(Box::new(inner))
                }
                Some(Tk::Punct('{')) => {
                    self.pos += 1;
                    let inner = self.alternatives()?;
                    self.expect_punct('}')?;
                    Expr::Rep(Box::new(inner))
                }
                _ => break,
            };
            match item {
                Expr::Seq(inner) => items.extend(inner),
                other => items.push(other),
            }
        }
        match items.len() {
            0 => Err(format!("line {}: empty alternative", self.line())),
            1 => Ok(items.remove(0)),
            _ => Ok(Expr::Seq(items)),
        }
    }
}

fn parse_ebnf(source: &str) -> Result<Grammar, String> {
    let Lexed { tokens, comments } = lex(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    let mut rules = Vec::new();
    let mut previous_end = 0usize;
    while parser.pos < parser.tokens.len() {
        let line = parser.line();
        let name = match parser.peek() {
            Some(Tk::Ident(n)) => n.clone(),
            other => {
                return Err(format!(
                    "line {line}: a rule must start with a name, got {other:?}"
                ))
            }
        };
        parser.pos += 1;
        parser.expect_punct('=')?;
        let body = parser.alternatives()?;
        parser.expect_punct(';')?;
        let end = parser.pos;
        let rule_comments = comments
            .iter()
            .filter(|(at, _)| *at >= previous_end && *at < end)
            .map(|(_, text)| text.clone())
            .collect();
        rules.push(Rule {
            name,
            body,
            comments: rule_comments,
            line,
        });
        previous_end = end;
    }
    Ok(Grammar { rules })
}

fn is_terminal(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn show(expr: &Expr) -> String {
    match expr {
        Expr::Seq(items) => items.iter().map(show).collect::<Vec<_>>().join(" "),
        Expr::Alt(items) => items.iter().map(show).collect::<Vec<_>>().join(" | "),
        Expr::Opt(inner) => format!("[ {} ]", show(inner)),
        Expr::Rep(inner) => format!("{{ {} }}", show(inner)),
        Expr::Name(n) => n.clone(),
        Expr::Lit(s) => format!("\"{s}\""),
        Expr::Special(s) => format!("?{s}?"),
    }
}

fn names_in(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Seq(items) | Expr::Alt(items) => items.iter().for_each(|e| names_in(e, out)),
        Expr::Opt(inner) | Expr::Rep(inner) => names_in(inner, out),
        Expr::Name(n) => out.push(n.clone()),
        Expr::Lit(_) | Expr::Special(_) => {}
    }
}

fn lits_in(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Seq(items) | Expr::Alt(items) => items.iter().for_each(|e| lits_in(e, out)),
        Expr::Opt(inner) | Expr::Rep(inner) => lits_in(inner, out),
        Expr::Lit(s) => out.push(s.clone()),
        Expr::Name(_) | Expr::Special(_) => {}
    }
}

impl Grammar {
    fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// Rules over tokens: lower-case names, minus the `guard_*` predicates,
    /// which describe a check the parser makes and are not part of the
    /// language.
    fn syntactic(&self) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| !is_terminal(&r.name) && !r.name.starts_with("guard_"))
            .collect()
    }

    fn lexical(&self) -> Vec<&Rule> {
        self.rules.iter().filter(|r| is_terminal(&r.name)).collect()
    }

    fn start(&self) -> &str {
        &self.syntactic()[0].name
    }
}

// ======================================================================
// EBNF: structural checks
// ======================================================================

/// Every defect found in the structure of the file, as readable lines.
/// Empty means the file is well formed.
fn structural_defects(g: &Grammar) -> Vec<String> {
    let mut defects = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in &g.rules {
        if !seen.insert(rule.name.clone()) {
            defects.push(format!(
                "line {}: `{}` is defined twice",
                rule.line, rule.name
            ));
        }
    }
    let defined: BTreeSet<&str> = g.rules.iter().map(|r| r.name.as_str()).collect();
    for rule in &g.rules {
        let mut used = Vec::new();
        names_in(&rule.body, &mut used);
        for name in used {
            if !defined.contains(name.as_str()) {
                defects.push(format!(
                    "line {}: `{}` uses `{name}`, which is not defined",
                    rule.line, rule.name
                ));
            }
        }
    }
    // Reachability: every syntactic rule must be used by another one, except
    // the start rule and the `guard_*` predicates.
    let mut referenced = BTreeSet::new();
    for rule in &g.rules {
        let mut used = Vec::new();
        names_in(&rule.body, &mut used);
        for name in used {
            if name != rule.name {
                referenced.insert(name);
            }
        }
    }
    let start = g.syntactic().first().map(|r| r.name.clone());
    for rule in g.syntactic() {
        if Some(&rule.name) != start.as_ref() && !referenced.contains(&rule.name) {
            defects.push(format!(
                "line {}: `{}` is never used from any other rule",
                rule.line, rule.name
            ));
        }
    }
    for rule in g.lexical() {
        let exempt = rule
            .comments
            .iter()
            .any(|c| c.contains("helper:") || c.contains("emits:"));
        if !referenced.contains(&rule.name) && !exempt {
            defects.push(format!(
                "line {}: terminal `{}` is defined and never used",
                rule.line, rule.name
            ));
        }
    }
    defects
}

// ======================================================================
// References to code and data
// ======================================================================

#[derive(Clone, Debug, PartialEq)]
enum Reference {
    Code {
        file: String,
        function: String,
        line: usize,
    },
    Yaml {
        file: String,
        path: String,
    },
}

fn references_in(comment: &str) -> Vec<Reference> {
    let mut out = Vec::new();
    for raw in comment.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let token = raw.trim_matches(|c: char| matches!(c, '(' | ')' | '.' | '—'));
        if let Some((file, rest)) = token.split_once(".rs:") {
            let mut parts = rest.split(':');
            let function = parts.next().unwrap_or_default();
            let line = parts.next().and_then(|l| l.strip_prefix('L'));
            if let (false, Some(line)) = (function.is_empty(), line) {
                if let Ok(line) = line.parse::<usize>() {
                    out.push(Reference::Code {
                        file: format!("{file}.rs"),
                        function: function.to_string(),
                        line,
                    });
                }
            }
        } else if let Some((file, path)) = token.split_once(".yaml:") {
            if !path.is_empty() {
                out.push(Reference::Yaml {
                    file: format!("{file}.yaml"),
                    path: path.to_string(),
                });
            }
        }
    }
    out
}

fn core_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    core_dir().join("..").join("..")
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn source_path(file: &str) -> Option<PathBuf> {
    ["src/parser", "src"]
        .iter()
        .map(|dir| core_dir().join(dir).join(file))
        .find(|p| p.exists())
}

fn yaml_path(file: &str) -> Option<PathBuf> {
    ["mathematics", "chemistry", "physics", "common"]
        .iter()
        .map(|dir| core_dir().join("data/domains").join(dir).join(file))
        .find(|p| p.exists())
}

/// First and last line (1-based) of `fn name`, found by indentation: the
/// function ends at the first closing brace at its own indentation. A
/// `const name:` is accepted too; it ends at the first line ending in `;`.
fn function_span(source: &str, name: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let const_start = lines.iter().position(|line| {
        let t = line.trim_start();
        let t = t
            .strip_prefix("pub(crate) ")
            .or_else(|| t.strip_prefix("pub "))
            .unwrap_or(t);
        t.strip_prefix("const ")
            .is_some_and(|rest| rest.strip_prefix(name).is_some_and(|a| a.starts_with(':')))
    });
    if let Some(start) = const_start {
        let end = (start..lines.len()).find(|&i| lines[i].trim_end().ends_with(';'))?;
        return Some((start + 1, end + 1));
    }
    let start = lines.iter().position(|line| {
        let t = line.trim_start();
        let t = t
            .strip_prefix("pub(crate) ")
            .or_else(|| t.strip_prefix("pub "))
            .unwrap_or(t);
        t.strip_prefix("fn ").is_some_and(|rest| {
            rest.strip_prefix(name)
                .is_some_and(|after| after.starts_with('(') || after.starts_with('<'))
        })
    })?;
    let indent = lines[start].len() - lines[start].trim_start().len();
    // The closing brace of a function has the indentation of its `fn` line.
    let end = (start..lines.len()).find(|&i| {
        let line = lines[i];
        line.len() - line.trim_start().len() == indent && line.trim() == "}"
    })?;
    Some((start + 1, end + 1))
}

fn yaml_phrases(file: &str, path: &str) -> Result<Vec<String>, String> {
    let location = yaml_path(file).ok_or_else(|| format!("no such data file: {file}"))?;
    let value: serde_yaml::Value =
        serde_yaml::from_str(&read_file(&location)).map_err(|e| e.to_string())?;
    let mut node = &value;
    for key in path.split('.') {
        node = node
            .get(key)
            .ok_or_else(|| format!("{file}: no key `{key}` on the way to `{path}`"))?;
    }
    let seq = node
        .as_sequence()
        .ok_or_else(|| format!("{file}: `{path}` is not a list"))?;
    seq.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{file}: `{path}` holds a non-string"))
        })
        .collect()
}

fn reference_defects(g: &Grammar) -> Vec<String> {
    let mut defects = Vec::new();
    for rule in &g.rules {
        let refs: Vec<Reference> = rule
            .comments
            .iter()
            .flat_map(|c| references_in(c))
            .collect();
        let has_code = refs.iter().any(|r| matches!(r, Reference::Code { .. }));
        if !has_code {
            defects.push(format!(
                "line {}: rule `{}` has no reference to code (file.rs:function:Lnnn)",
                rule.line, rule.name
            ));
        }
        for reference in refs {
            match reference {
                Reference::Code {
                    file,
                    function,
                    line,
                } => {
                    let Some(path) = source_path(&file) else {
                        defects.push(format!(
                            "rule `{}`: no source file {file} under src/",
                            rule.name
                        ));
                        continue;
                    };
                    match function_span(&read_file(&path), &function) {
                        None => defects.push(format!(
                            "rule `{}`: no `fn {function}` in {file}",
                            rule.name
                        )),
                        Some((start, end)) => {
                            if line + LINE_DRIFT_TOLERANCE < start
                                || line > end + LINE_DRIFT_TOLERANCE
                            {
                                defects.push(format!(
                                    "rule `{}`: {file}:{function} is at L{start}-L{end}, the document says L{line}",
                                    rule.name
                                ));
                            }
                        }
                    }
                }
                Reference::Yaml { file, path } => {
                    if let Err(e) = yaml_phrases(&file, &path) {
                        defects.push(format!("rule `{}`: {e}", rule.name));
                    }
                }
            }
        }
    }
    defects
}

// ======================================================================
// Lexical layer against the code
// ======================================================================

fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

/// The variants of `enum Tok`, as the names the EBNF gives its terminals.
fn tok_variants(math_rs: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut inside = false;
    for line in math_rs.lines() {
        let t = line.trim();
        let t = t
            .strip_prefix("pub(crate) ")
            .or_else(|| t.strip_prefix("pub "))
            .unwrap_or(t);
        if t.starts_with("enum Tok") {
            inside = true;
            continue;
        }
        if inside {
            if t == "}" {
                break;
            }
            if t.starts_with("//") || t.is_empty() {
                continue;
            }
            let name: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.insert(snake(&name));
            }
        }
    }
    out
}

/// `(section, key) -> [Tok names]` from the `operator_tokens` table.
fn operator_token_table(math_rs: &str) -> BTreeMap<(String, String), Vec<String>> {
    let (start, end) = function_span(math_rs, "operator_tokens").expect("operator_tokens exists");
    let mut out = BTreeMap::new();
    for line in math_rs.lines().skip(start - 1).take(end - start + 1) {
        let Some((left, right)) = line.split_once("=> vec![") else {
            continue;
        };
        let toks: Vec<String> = right
            .split("Tok::")
            .skip(1)
            .map(|rest| {
                snake(
                    &rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric())
                        .collect::<String>(),
                )
            })
            .collect();
        for pair in left.split('(').skip(1) {
            let quoted: Vec<&str> = pair.split('"').collect();
            if quoted.len() >= 4 {
                out.insert((quoted[1].to_string(), quoted[3].to_string()), toks.clone());
            }
        }
    }
    out
}

/// `Tok::X` mentioned in `atom_token_is_supported`, comments excluded.
fn guard_tokens(math_rs: &str) -> BTreeSet<String> {
    let (start, end) =
        function_span(math_rs, "atom_token_is_supported").expect("atom_token_is_supported exists");
    let mut out = BTreeSet::new();
    for line in math_rs.lines().skip(start - 1).take(end - start + 1) {
        let code = line.split("//").next().unwrap_or_default();
        for piece in code.split("Tok::").skip(1) {
            let name: String = piece.chars().take_while(|c| c.is_alphanumeric()).collect();
            out.insert(snake(&name));
        }
    }
    out
}

fn terminals_in_rule(rule: &Rule) -> BTreeSet<String> {
    let mut names = Vec::new();
    names_in(&rule.body, &mut names);
    names.into_iter().filter(|n| is_terminal(n)).collect()
}

/// Defects of the lexical layer of a grammar against the data files and the
/// sources it cites. `math` additionally ties the terminals to `enum Tok`.
fn lexical_defects(g: &Grammar, math: bool) -> Vec<String> {
    let mut defects = Vec::new();
    for rule in g.lexical() {
        let refs: Vec<Reference> = rule
            .comments
            .iter()
            .flat_map(|c| references_in(c))
            .collect();
        let mut from_yaml: BTreeSet<String> = BTreeSet::new();
        let mut yaml_seen = false;
        let mut sources: Vec<String> = Vec::new();
        for reference in &refs {
            match reference {
                Reference::Yaml { file, path } => {
                    yaml_seen = true;
                    match yaml_phrases(file, path) {
                        Ok(list) => from_yaml.extend(list),
                        Err(e) => defects.push(format!("`{}`: {e}", rule.name)),
                    }
                }
                Reference::Code { file, .. } => {
                    if let Some(path) = source_path(file) {
                        sources.push(read_file(&path));
                    }
                }
            }
        }
        let mut lits = Vec::new();
        lits_in(&rule.body, &mut lits);
        let lits: BTreeSet<String> = lits.into_iter().collect();
        if yaml_seen {
            for phrase in from_yaml.difference(&lits) {
                defects.push(format!(
                    "`{}`: the phrase \"{phrase}\" is in the data file and not in the grammar",
                    rule.name
                ));
            }
        }
        // Whatever the data file does not provide must be written in the code
        // the rule cites: no phrase may exist only in the document.
        for lit in lits.difference(&from_yaml) {
            let quoted = format!("\"{lit}\"");
            if !sources.iter().any(|source| source.contains(&quoted)) {
                defects.push(format!(
                    "`{}`: the literal \"{lit}\" is neither in the data files it cites nor quoted in the code it cites",
                    rule.name
                ));
            }
        }
    }
    if math {
        let math_rs = read_file(&source_path("math.rs").expect("math.rs"));
        let variants = tok_variants(&math_rs);
        let table = operator_token_table(&math_rs);
        let mut named: BTreeSet<String> = BTreeSet::new();
        for rule in g.lexical() {
            let is_helper = rule.comments.iter().any(|c| c.contains("helper:"));
            let emits = rule
                .comments
                .iter()
                .find_map(|c| c.split("emits:").nth(1))
                .map(|rest| {
                    rest.split(';')
                        .next()
                        .unwrap_or_default()
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                });
            if is_helper {
                continue;
            }
            if emits.is_none() {
                named.insert(rule.name.clone());
            }
            // The table that maps data keys to tokens must agree.
            for reference in rule.comments.iter().flat_map(|c| references_in(c)) {
                if let Reference::Yaml { path, .. } = reference {
                    let (section, key) = path.split_once('.').unwrap_or((path.as_str(), ""));
                    let expected = emits.clone().unwrap_or_else(|| vec![rule.name.clone()]);
                    match table.get(&(section.to_string(), key.to_string())) {
                        Some(actual) if *actual == expected => {}
                        other => defects.push(format!(
                            "`{}`: operators.yaml `{path}` becomes {other:?} in operator_tokens, the grammar says {expected:?}",
                            rule.name
                        )),
                    }
                }
            }
        }
        for missing in variants.difference(&named) {
            defects.push(format!(
                "enum Tok has the variant {missing} and the grammar defines no such terminal"
            ));
        }
        for extra in named.difference(&variants) {
            defects.push(format!(
                "the grammar defines the terminal {extra}, which is not a variant of enum Tok"
            ));
        }
        // Every key of operators.yaml must be consumed by some terminal.
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for rule in g.lexical() {
            for reference in rule.comments.iter().flat_map(|c| references_in(c)) {
                if let Reference::Yaml { file, path } = reference {
                    if file == "operators.yaml" {
                        covered.insert(path);
                    }
                }
            }
        }
        let yaml: serde_yaml::Value = serde_yaml::from_str(&read_file(
            &yaml_path("operators.yaml").expect("operators.yaml"),
        ))
        .expect("operators.yaml parses");
        if let Some(map) = yaml.as_mapping() {
            for (section, entries) in map {
                let Some(section) = section.as_str() else {
                    continue;
                };
                let Some(entries) = entries.as_mapping() else {
                    continue;
                };
                for key in entries.keys().filter_map(|k| k.as_str()) {
                    let path = format!("{section}.{key}");
                    if !covered.contains(&path) {
                        defects.push(format!(
                            "operators.yaml `{path}` is not the source of any terminal"
                        ));
                    }
                }
            }
        }
        // The predicate that guards loops must be the one in the code.
        if let Some(guard) = g.rule("guard_starts_atom") {
            let in_grammar = terminals_in_rule(guard);
            let in_code = guard_tokens(&math_rs);
            if in_grammar != in_code {
                defects.push(format!(
                    "guard_starts_atom differs from atom_token_is_supported: only in the grammar {:?}, only in the code {:?}",
                    in_grammar.difference(&in_code).collect::<Vec<_>>(),
                    in_code.difference(&in_grammar).collect::<Vec<_>>()
                ));
            }
        } else {
            defects.push("the grammar has no guard_starts_atom rule".to_string());
        }
    }
    defects
}

// ======================================================================
// BNF, FIRST, FOLLOW, LL(1) conflicts
// ======================================================================

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Sym {
    T(String),
    N(String),
}

#[derive(Default)]
struct Bnf {
    prods: BTreeMap<String, Vec<Vec<Sym>>>,
    /// generated nonterminal -> (kind, EBNF text of the construct)
    origin: BTreeMap<String, (&'static str, String)>,
    /// the source rules, in file order
    sources: Vec<String>,
}

struct Lowering<'a> {
    rule: &'a str,
    optional: usize,
    repeat: usize,
    group: usize,
    bnf: &'a mut Bnf,
}

impl Lowering<'_> {
    fn alternatives(expr: &Expr) -> Vec<&Expr> {
        match expr {
            Expr::Alt(alts) => alts.iter().collect(),
            other => vec![other],
        }
    }

    fn sequence(&mut self, expr: &Expr) -> Vec<Sym> {
        match expr {
            Expr::Seq(items) => items.iter().map(|i| self.item(i)).collect(),
            other => vec![self.item(other)],
        }
    }

    fn item(&mut self, expr: &Expr) -> Sym {
        match expr {
            Expr::Name(n) if is_terminal(n) => Sym::T(n.clone()),
            Expr::Name(n) => Sym::N(n.clone()),
            Expr::Lit(s) => Sym::T(format!("\"{s}\"")),
            Expr::Special(s) => panic!("a special sequence ?{s}? inside a syntactic rule"),
            Expr::Seq(_) | Expr::Alt(_) => {
                self.group += 1;
                let name = format!("{}~G{}", self.rule, self.group);
                self.bnf
                    .origin
                    .insert(name.clone(), ("group", format!("( {} )", show(expr))));
                let prods = Self::alternatives(expr)
                    .into_iter()
                    .map(|a| self.sequence(a))
                    .collect();
                self.bnf.prods.insert(name.clone(), prods);
                Sym::N(name)
            }
            Expr::Opt(inner) => {
                self.optional += 1;
                let name = format!("{}~O{}", self.rule, self.optional);
                self.bnf
                    .origin
                    .insert(name.clone(), ("optional", format!("[ {} ]", show(inner))));
                let mut prods: Vec<Vec<Sym>> = Self::alternatives(inner)
                    .into_iter()
                    .map(|a| self.sequence(a))
                    .collect();
                prods.push(Vec::new());
                self.bnf.prods.insert(name.clone(), prods);
                Sym::N(name)
            }
            Expr::Rep(inner) => {
                self.repeat += 1;
                let name = format!("{}~R{}", self.rule, self.repeat);
                self.bnf
                    .origin
                    .insert(name.clone(), ("loop", format!("{{ {} }}", show(inner))));
                let mut prods: Vec<Vec<Sym>> = Self::alternatives(inner)
                    .into_iter()
                    .map(|a| {
                        let mut seq = self.sequence(a);
                        seq.push(Sym::N(name.clone()));
                        seq
                    })
                    .collect();
                prods.push(Vec::new());
                self.bnf.prods.insert(name.clone(), prods);
                Sym::N(name)
            }
        }
    }
}

fn lower(g: &Grammar) -> Bnf {
    let mut bnf = Bnf::default();
    for rule in g.syntactic() {
        bnf.sources.push(rule.name.clone());
        let mut lowering = Lowering {
            rule: &rule.name,
            optional: 0,
            repeat: 0,
            group: 0,
            bnf: &mut bnf,
        };
        let prods: Vec<Vec<Sym>> = Lowering::alternatives(&rule.body)
            .into_iter()
            .map(|a| lowering.sequence(a))
            .collect();
        bnf.prods.insert(rule.name.clone(), prods);
    }
    bnf
}

struct Analysis {
    nullable: BTreeSet<String>,
    first: BTreeMap<String, BTreeSet<String>>,
    follow: BTreeMap<String, BTreeSet<String>>,
}

const END: &str = "$";

fn first_of(
    seq: &[Sym],
    nullable: &BTreeSet<String>,
    first: &BTreeMap<String, BTreeSet<String>>,
) -> (BTreeSet<String>, bool) {
    let mut out = BTreeSet::new();
    for sym in seq {
        match sym {
            Sym::T(t) => {
                out.insert(t.clone());
                return (out, false);
            }
            Sym::N(n) => {
                if let Some(f) = first.get(n) {
                    out.extend(f.iter().cloned());
                }
                if !nullable.contains(n) {
                    return (out, false);
                }
            }
        }
    }
    (out, true)
}

fn analyse(bnf: &Bnf, start: &str) -> Analysis {
    let mut nullable: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = nullable.len();
        for (name, prods) in &bnf.prods {
            if prods.iter().any(|p| {
                p.iter().all(|s| match s {
                    Sym::T(_) => false,
                    Sym::N(n) => nullable.contains(n),
                })
            }) {
                nullable.insert(name.clone());
            }
        }
        if nullable.len() == before {
            break;
        }
    }
    let mut first: BTreeMap<String, BTreeSet<String>> = bnf
        .prods
        .keys()
        .map(|k| (k.clone(), BTreeSet::new()))
        .collect();
    loop {
        let mut changed = false;
        for (name, prods) in &bnf.prods {
            for prod in prods {
                let (set, _) = first_of(prod, &nullable, &first);
                let entry = first.get_mut(name).expect("every nonterminal has an entry");
                for token in set {
                    changed |= entry.insert(token);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut follow: BTreeMap<String, BTreeSet<String>> = bnf
        .prods
        .keys()
        .map(|k| (k.clone(), BTreeSet::new()))
        .collect();
    follow
        .get_mut(start)
        .expect("the start rule exists")
        .insert(END.to_string());
    loop {
        let mut changed = false;
        for (name, prods) in &bnf.prods {
            for prod in prods {
                for (i, sym) in prod.iter().enumerate() {
                    let Sym::N(target) = sym else { continue };
                    let (mut set, tail_nullable) = first_of(&prod[i + 1..], &nullable, &first);
                    if tail_nullable {
                        set.extend(follow[name].iter().cloned());
                    }
                    let entry = follow
                        .get_mut(target)
                        .expect("every nonterminal has an entry");
                    for token in set {
                        changed |= entry.insert(token);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    Analysis {
        nullable,
        first,
        follow,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Conflict {
    id: String,
    kind: &'static str,
    text: String,
    tokens: BTreeSet<String>,
}

fn conflicts(bnf: &Bnf, analysis: &Analysis) -> Vec<Conflict> {
    let mut out = Vec::new();
    for (name, prods) in &bnf.prods {
        let selectors: Vec<BTreeSet<String>> = prods
            .iter()
            .map(|p| {
                let (mut set, nullable) = first_of(p, &analysis.nullable, &analysis.first);
                if nullable {
                    set.extend(analysis.follow[name].iter().cloned());
                }
                set
            })
            .collect();
        let mut tokens = BTreeSet::new();
        for i in 0..selectors.len() {
            for j in (i + 1)..selectors.len() {
                tokens.extend(selectors[i].intersection(&selectors[j]).cloned());
            }
        }
        if tokens.is_empty() {
            continue;
        }
        let (kind, text) = bnf
            .origin
            .get(name)
            .cloned()
            .unwrap_or(("alternatives", format!("{} alternatives", prods.len())));
        out.push(Conflict {
            id: name.clone(),
            kind,
            text,
            tokens,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn list(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        return "—".to_string();
    }
    set.iter()
        .map(|t| format!("`{t}`"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_first_follow(bnf: &Bnf, analysis: &Analysis) -> String {
    let mut out = String::from("| Нетерминал | ε | FIRST | FOLLOW |\n|---|---|---|---|\n");
    for name in &bnf.sources {
        out.push_str(&format!(
            "| `{name}` | {} | {} | {} |\n",
            if analysis.nullable.contains(name) {
                "да"
            } else {
                "нет"
            },
            list(&analysis.first[name]),
            list(&analysis.follow[name]),
        ));
    }
    out.trim_end().to_string()
}

fn clip(text: &str) -> String {
    let flat = text.replace('|', "¦");
    if flat.chars().count() > 70 {
        let cut: String = flat.chars().take(69).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn render_conflicts(list_of: &[Conflict]) -> String {
    let mut out =
        String::from("| Идентификатор | Что | Конструкция | Общие токены |\n|---|---|---|---|\n");
    for c in list_of {
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} |\n",
            c.id,
            c.kind,
            clip(&c.text),
            list(&c.tokens)
        ));
    }
    out.trim_end().to_string()
}

struct Computed {
    bnf: Bnf,
    analysis: Analysis,
    conflicts: Vec<Conflict>,
}

fn compute(source: &str) -> Computed {
    let grammar = parse_ebnf(source).unwrap_or_else(|e| panic!("the grammar does not parse: {e}"));
    let defects = structural_defects(&grammar);
    assert!(
        defects.is_empty(),
        "structural defects:\n{}",
        defects.join("\n")
    );
    let bnf = lower(&grammar);
    let analysis = analyse(&bnf, grammar.start());
    let found = conflicts(&bnf, &analysis);
    Computed {
        bnf,
        analysis,
        conflicts: found,
    }
}

fn math_source() -> String {
    read_file(&repo_root().join("docs/grammar/math.ebnf"))
}

fn chem_source() -> String {
    read_file(&repo_root().join("docs/grammar/chem.ebnf"))
}

fn analysis_doc() -> String {
    read_file(&repo_root().join("docs/grammar/ANALYSIS_RU.md"))
}

// ======================================================================
// Overlapping word classes
// ======================================================================

/// The quoted strings of `const NAME: ... = [ ... ];`.
fn const_strings(source: &str, name: &str) -> BTreeSet<String> {
    let (start, end) = function_span(source, name).unwrap_or_else(|| panic!("no const {name}"));
    let text: String = source
        .lines()
        .skip(start - 1)
        .take(end - start + 1)
        .collect::<Vec<_>>()
        .join("\n");
    let after = text.split('=').nth(1).unwrap_or_default();
    after
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, piece)| piece.to_string())
        .collect()
}

/// Words that belong to more than one class of the lexicon, rendered for the
/// document. The tables of FIRST and FOLLOW treat classes as disjoint
/// terminals; this is the place where they are not.
fn render_overlaps() -> String {
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let mut rows: Vec<String> = Vec::new();

    let yaml: serde_yaml::Value = serde_yaml::from_str(&read_file(
        &yaml_path("operators.yaml").expect("operators.yaml"),
    ))
    .expect("operators.yaml parses");
    let units: BTreeSet<&str> = lex
        .units
        .iter()
        .flat_map(|u| u.names.iter().map(String::as_str))
        .collect();
    let mut keyword_rows = BTreeSet::new();
    if let Some(sections) = yaml.as_mapping() {
        for (section, entries) in sections {
            let (Some(section), Some(entries)) = (section.as_str(), entries.as_mapping()) else {
                continue;
            };
            for (key, phrases) in entries {
                let Some(key) = key.as_str() else { continue };
                for phrase in phrases
                    .as_sequence()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str())
                {
                    let first = phrase.split_whitespace().next().unwrap_or_default();
                    let mut classes = Vec::new();
                    if lex.greek.contains_key(first) {
                        classes.push("греческая буква");
                    }
                    if lex.latin.contains_key(first) {
                        classes.push("латинская буква");
                    }
                    if lex.cyrillic.contains_key(first) {
                        classes.push("кириллическая буква");
                    }
                    if nums.lookup(first).is_some() {
                        classes.push("число");
                    }
                    if nums.ordinal(first).is_some() {
                        classes.push("порядковое");
                    }
                    if units.contains(first) {
                        classes.push("единица");
                    }
                    if !classes.is_empty() {
                        keyword_rows.insert(format!(
                            "| математика | `{phrase}` (`{section}.{key}`) начинается со слова, которое также: {} |",
                            classes.join(", ")
                        ));
                    }
                }
            }
        }
    }
    rows.extend(keyword_rows);

    let substances: BTreeSet<&str> = lex
        .substances
        .iter()
        .flat_map(|s| s.names.iter().map(String::as_str))
        .collect();
    let elements: BTreeSet<&str> = lex.elements_by_name.keys().map(String::as_str).collect();
    let letters: BTreeSet<&str> = lex.latin.keys().map(String::as_str).collect();
    let anions: BTreeSet<&str> = lex.anion_classes.keys().map(String::as_str).collect();
    let join = |set: BTreeSet<&str>| {
        if set.is_empty() {
            "—".to_string()
        } else {
            set.into_iter()
                .map(|w| format!("`{w}`"))
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    rows.push(format!(
        "| химия | имя вещества и название элемента: {} |",
        join(substances.intersection(&elements).copied().collect())
    ));
    rows.push(format!(
        "| химия | название элемента и название латинской буквы: {} |",
        join(elements.intersection(&letters).copied().collect())
    ));
    rows.push(format!(
        "| химия | имя вещества и название аниона или катиона: {} |",
        join(substances.intersection(&anions).copied().collect())
    ));
    rows.push(format!(
        "| химия | название элемента и название аниона или катиона: {} |",
        join(elements.intersection(&anions).copied().collect())
    ));
    let chemistry_rs = read_file(&source_path("chemistry.rs").expect("chemistry.rs"));
    let function_letters = const_strings(&chemistry_rs, "FUNCTION_WORD_LETTERS");
    let describe = |word: &str| {
        let mut classes = Vec::new();
        if lex.element(word).is_some() {
            classes.push("элемент");
        }
        if lex.latin(word).is_some() {
            classes.push("буква");
        }
        if nums.lookup(word).is_some() {
            classes.push("число");
        }
        if classes.is_empty() {
            "только служебное слово".to_string()
        } else {
            classes.join(" и ")
        }
    };
    for word in &function_letters {
        rows.push(format!(
            "| химия и математика | служебное слово-буква `{word}`: {} |",
            describe(word)
        ));
    }
    let mut out = String::from("| Область | Пересечение |\n|---|---|\n");
    for row in rows {
        out.push_str(&row);
        out.push('\n');
    }
    out.trim_end().to_string()
}

// ======================================================================
// The document
// ======================================================================

fn block<'a>(doc: &'a str, tag: &str) -> Option<&'a str> {
    let begin = format!("<!-- grammar-analysis:{tag}:begin -->");
    let end = format!("<!-- grammar-analysis:{tag}:end -->");
    let from = doc.find(&begin)? + begin.len();
    let to = doc[from..].find(&end)? + from;
    Some(doc[from..to].trim())
}

fn maybe_print(tag: &str, text: &str) {
    if std::env::var_os("GRAMMAR_PRINT").is_some() {
        println!(
            "<!-- grammar-analysis:{tag}:begin -->\n{text}\n<!-- grammar-analysis:{tag}:end -->"
        );
    }
}

fn first_follow_block_defects(doc: &str, name: &str, c: &Computed) -> Vec<String> {
    let mut defects = Vec::new();
    let tables = [
        (
            format!("{name}:first-follow"),
            render_first_follow(&c.bnf, &c.analysis),
        ),
        (format!("{name}:conflicts"), render_conflicts(&c.conflicts)),
    ];
    for (tag, expected) in tables {
        match block(doc, &tag) {
            None => defects.push(format!("the document has no block `{tag}`")),
            Some(actual) if actual != expected => defects.push(format!(
                "block `{tag}` is stale.\n--- the document\n{actual}\n--- computed from the grammar\n{expected}"
            )),
            Some(_) => {}
        }
    }
    // Every conflict must be discussed under its own heading.
    for conflict in &c.conflicts {
        let heading = format!("`{}`", conflict.id);
        if !doc
            .lines()
            .any(|l| l.starts_with("#### ") && l.contains(&heading))
        {
            defects.push(format!(
                "conflict {heading} has no `#### ` heading in the analysis"
            ));
        }
    }
    defects
}

fn render_summary(written: &Computed, strict: &Computed, chem: &Computed) -> String {
    let row = |name: &str, c: &Computed, operand: &str| {
        let sources = c.bnf.sources.len();
        let generated = c.bnf.prods.len() - sources;
        let mut terminals = BTreeSet::new();
        let mut literals = BTreeSet::new();
        for prods in c.bnf.prods.values() {
            for sym in prods.iter().flatten() {
                if let Sym::T(t) = sym {
                    if t.starts_with('"') {
                        literals.insert(t.clone());
                    } else {
                        terminals.insert(t.clone());
                    }
                }
            }
        }
        format!(
            "| {name} | {sources} | {generated} | {} | {} | `{operand}`: {} из {} | {} |\n",
            terminals.len(),
            c.conflicts.len(),
            c.analysis.first[operand].len(),
            terminals.len(),
            c.analysis.follow[operand].len(),
        )
    };
    let mut out = String::from(
        "| Грамматика | Нетерминалов | Порождённых (скобки, циклы, группы) | Терминалов | Конфликтов LL(1) | Размер FIRST (из всех терминалов) | Размер FOLLOW |\n|---|---|---|---|---|---|---|\n",
    );
    out.push_str(&row("math.ebnf как записана", written, "common_atom"));
    out.push_str(&row(
        "math.ebnf, все закрывающие слова обязательны",
        strict,
        "common_atom",
    ));
    out.push_str(&row("chem.ebnf", chem, "species"));
    out.trim_end().to_string()
}

fn summary_block_defects(doc: &str) -> Vec<String> {
    let written = compute(&math_source());
    let strict = compute(&with_mandatory_closers(&math_source()));
    let chem = compute(&chem_source());
    let expected = render_summary(&written, &strict, &chem);
    maybe_print("summary", &expected);
    match block(doc, "summary") {
        None => vec!["the document has no block `summary`".to_string()],
        Some(actual) if actual != expected => vec![format!(
            "block `summary` is stale.\n--- the document\n{actual}\n--- computed\n{expected}"
        )],
        Some(_) => Vec::new(),
    }
}

fn overlap_block_defects(doc: &str) -> Vec<String> {
    let expected = render_overlaps();
    maybe_print("overlaps", &expected);
    match block(doc, "overlaps") {
        None => vec!["the document has no block `overlaps`".to_string()],
        Some(actual) if actual != expected => vec![format!(
            "block `overlaps` is stale.\n--- the document\n{actual}\n--- computed from the lexicon\n{expected}"
        )],
        Some(_) => Vec::new(),
    }
}

#[derive(Debug)]
struct Probe {
    domain: Domain,
    phrase: String,
    expected: String,
}

/// Every table row of the document whose first cell is `math`, `phys` or
/// `chem` is a quoted run of the compiler: domain, phrase, printed result.
fn probes(doc: &str) -> Vec<Probe> {
    doc.lines()
        .filter(|l| l.starts_with('|'))
        .filter_map(|l| {
            // The printed result may contain a pipe (an absolute value), which
            // a markdown table needs escaped; the third cell is "the rest".
            let mut cells = l.trim().strip_prefix('|')?.splitn(3, '|');
            let (domain, phrase, rest) = (cells.next()?, cells.next()?, cells.next()?);
            let rest = rest.trim();
            let rest = rest.strip_suffix('|').unwrap_or(rest).trim();
            let domain = match domain.trim() {
                "math" => Domain::Mathematics,
                "phys" => Domain::Physics,
                "chem" => Domain::Chemistry,
                _ => return None,
            };
            Some(Probe {
                domain,
                phrase: phrase.trim().to_string(),
                expected: rest.replace("\\|", "|"),
            })
        })
        .collect()
}

fn compile(domain: Domain, phrase: &str) -> String {
    let result = interpret_utterance(
        phrase,
        UtteranceOptions {
            domain,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
}

// ======================================================================
// Tests: the documents
// ======================================================================

#[test]
fn both_grammars_are_well_formed() {
    for (name, source) in [("math.ebnf", math_source()), ("chem.ebnf", chem_source())] {
        let grammar = parse_ebnf(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let defects = structural_defects(&grammar);
        assert!(defects.is_empty(), "{name}:\n{}", defects.join("\n"));
        assert!(
            grammar.syntactic().len() >= 10,
            "{name} has only {} syntactic rules — a grammar this small is not the parser",
            grammar.syntactic().len()
        );
    }
}

#[test]
fn every_reference_to_code_and_data_resolves() {
    for (name, source) in [("math.ebnf", math_source()), ("chem.ebnf", chem_source())] {
        let grammar = parse_ebnf(&source).expect("parses");
        let defects = reference_defects(&grammar);
        assert!(defects.is_empty(), "{name}:\n{}", defects.join("\n"));
    }
}

#[test]
fn the_lexical_layer_is_the_one_in_the_code() {
    let math = parse_ebnf(&math_source()).expect("parses");
    let defects = lexical_defects(&math, true);
    assert!(defects.is_empty(), "math.ebnf:\n{}", defects.join("\n"));
    let chem = parse_ebnf(&chem_source()).expect("parses");
    let defects = lexical_defects(&chem, false);
    assert!(defects.is_empty(), "chem.ebnf:\n{}", defects.join("\n"));
}

#[test]
fn the_embedded_tables_are_what_the_grammar_computes() {
    let doc = analysis_doc();
    let mut all = Vec::new();
    for (name, source) in [("math", math_source()), ("chem", chem_source())] {
        let computed = compute(&source);
        maybe_print(
            &format!("{name}:first-follow"),
            &render_first_follow(&computed.bnf, &computed.analysis),
        );
        maybe_print(
            &format!("{name}:conflicts"),
            &render_conflicts(&computed.conflicts),
        );
        for defect in first_follow_block_defects(&doc, name, &computed) {
            all.push(format!("{name}: {defect}"));
        }
    }
    all.extend(overlap_block_defects(&doc));
    all.extend(summary_block_defects(&doc));
    assert!(all.is_empty(), "{}", all.join("\n\n"));
}

fn probe_defects(table: &[Probe]) -> Vec<String> {
    table
        .iter()
        .filter_map(|probe| {
            let actual = compile(probe.domain, &probe.phrase);
            (actual != probe.expected).then(|| {
                format!(
                    "{:?} «{}»: the document says «{}», the compiler prints «{}»",
                    probe.domain, probe.phrase, probe.expected, actual
                )
            })
        })
        .collect()
}

/// Fewest quoted runs the analysis may have. A document that argues about
/// ambiguity without a single running example proves nothing.
const MIN_QUOTED_PHRASES: usize = 40;

#[test]
fn the_quoted_compiler_output_is_what_the_compiler_prints() {
    let table = probes(&analysis_doc());
    assert!(
        table.len() >= MIN_QUOTED_PHRASES,
        "the analysis quotes {} phrases, fewer than {MIN_QUOTED_PHRASES}",
        table.len()
    );
    let wrong = probe_defects(&table);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ======================================================================
// Tests: the findings the analysis rests on
// ======================================================================

fn conflict_ids(c: &Computed) -> BTreeSet<String> {
    c.conflicts.iter().map(|x| x.id.clone()).collect()
}

fn tokens_of<'a>(c: &'a Computed, id: &str) -> &'a BTreeSet<String> {
    &c.conflicts
        .iter()
        .find(|x| x.id == id)
        .unwrap_or_else(|| panic!("no conflict {id}; there are {:?}", conflict_ids(c)))
        .tokens
}

fn has(c: &Computed, id: &str, token: &str) -> bool {
    c.conflicts
        .iter()
        .any(|x| x.id == id && x.tokens.contains(token))
}

/// The conflicts the written analysis builds its argument on. They are named
/// here so that a grammar edit which makes one of them disappear — or which
/// moves it elsewhere — fails a test instead of leaving a paragraph about a
/// conflict that is no longer there.
#[test]
fn the_conflicts_the_analysis_discusses_exist() {
    let math = compute(&math_source());
    // «икс факториал игрек»: the postfix operator wins over the next operand.
    assert!(
        has(&math, "postfix~R1", "FACT"),
        "{:?}",
        conflict_ids(&math)
    );
    // A bound and the body of a sum or product share their first tokens.
    assert!(
        math.conflicts
            .iter()
            .any(|c| c.id.starts_with("juxt~R") && c.tokens.contains("NUM")),
        "{:?}",
        conflict_ids(&math)
    );
    // An unterminated bracket: the closer may belong to this or to an outer one.
    assert!(
        math.conflicts
            .iter()
            .any(|c| c.id.starts_with("common_atom~O")
                && c.tokens.len() == 1
                && c.tokens.contains("R_PAREN")),
        "{:?}",
        conflict_ids(&math)
    );
    // «на» after a unit is a division of units and a binary division at once.
    assert!(
        math.conflicts
            .iter()
            .any(|c| c.id.starts_with("unit_expr~") && c.tokens.contains("DIV")),
        "{:?}",
        conflict_ids(&math)
    );
    // The two readings of an open root.
    assert!(has(&math, "root_body", "SYM"), "{:?}", conflict_ids(&math));
    // An integral bound against an integrand: needs unbounded lookahead.
    assert!(
        has(&math, "integral_tail~G1", "FROM"),
        "{:?}",
        conflict_ids(&math)
    );

    let chem = compute(&chem_source());
    // A reaction and a single substance both begin with a substance.
    assert!(
        chem.conflicts.iter().any(|c| c.id == "chem_input"),
        "{:?}",
        conflict_ids(&chem)
    );
    // «ион меди два плюс»: a charge or the joint between two reagents.
    assert!(
        chem.conflicts.iter().any(|c| c.tokens.contains("PLUS")),
        "{:?}",
        conflict_ids(&chem)
    );
}

/// The hypothesis of the plan, measured: close every optional closer
/// (`[ FRAC_END ]` becomes `FRAC_END`) and count what is left.
const CLOSERS: [&str; 9] = [
    "R_PAREN", "R_BRACK", "R_BRACE", "FRAC_END", "POW_END", "ROOT_END", "SUM_END", "PROD_END",
    "INT_END",
];

fn with_mandatory_closers(source: &str) -> String {
    let mut out = source.to_string();
    for closer in CLOSERS {
        out = out.replace(&format!("[ {closer} ]"), closer);
    }
    out
}

#[test]
fn mandatory_closers_remove_the_nesting_conflicts_and_only_those() {
    let as_written = compute(&math_source());
    let strict = compute(&with_mandatory_closers(&math_source()));
    maybe_print(
        "math:strict-conflicts",
        &render_conflicts(&strict.conflicts),
    );
    // Teeth of the experiment: the edit must have changed the grammar.
    assert_ne!(
        math_source(),
        with_mandatory_closers(&math_source()),
        "the closers were not found in math.ebnf"
    );
    assert!(
        strict.conflicts.len() < as_written.conflicts.len(),
        "mandatory closers removed no conflict: {} against {}",
        strict.conflicts.len(),
        as_written.conflicts.len()
    );
    // A closer token is never again part of any remaining conflict.
    for conflict in &strict.conflicts {
        for closer in CLOSERS {
            assert!(
                !conflict.tokens.contains(closer),
                "{} still conflicts on {closer} with mandatory closers",
                conflict.id
            );
        }
    }
    // What is left is not about closers at all.
    assert!(
        !strict.conflicts.is_empty(),
        "no conflict is left: the natural constructs should still be ambiguous"
    );
    // Strict closers add no conflict of their own, and the ones that vanish
    // are exactly the ones the analysis files under the heading К1.
    let written_ids = conflict_ids(&as_written);
    let strict_ids = conflict_ids(&strict);
    assert!(
        strict_ids.is_subset(&written_ids),
        "mandatory closers created conflicts: {:?}",
        strict_ids.difference(&written_ids).collect::<Vec<_>>()
    );
    let vanished: BTreeSet<String> = written_ids.difference(&strict_ids).cloned().collect();
    let doc = analysis_doc();
    let heading = doc
        .lines()
        .find(|l| l.starts_with("#### К1."))
        .expect("the analysis has a heading К1");
    let named: BTreeSet<String> = heading
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    assert_eq!(
        named, vanished,
        "К1 must list exactly the conflicts that vanish with mandatory closers"
    );
}

// ======================================================================
// Tests that try to pass wrongly
// ======================================================================

#[test]
fn a_grammar_without_ambiguity_has_no_conflict_and_a_classic_one_has() {
    let clean = compute(
        "s = a { OP a } ;\n(* x.rs:f:L1 *)\na = NUM | L_PAREN s R_PAREN ;\n\
         OP = \"плюс\" ;\nNUM = ? число ? ;\nL_PAREN = \"(\" ;\nR_PAREN = \")\" ;\n",
    );
    assert!(clean.conflicts.is_empty(), "{:?}", clean.conflicts);
    let dangling = compute(
        "s = a ;\na = IF b THEN a [ ELSE a ] | X ;\nb = X ;\n\
         IF = \"если\" ;\nTHEN = \"то\" ;\nELSE = \"иначе\" ;\nX = ? x ? ;\n",
    );
    assert_eq!(
        dangling
            .conflicts
            .iter()
            .map(|c| (c.id.as_str(), list(&c.tokens)))
            .collect::<Vec<_>>(),
        vec![("a~O1", "`ELSE`".to_string())],
        "the dangling else is the one conflict"
    );
}

#[test]
fn first_and_follow_are_computed_with_nullable_symbols() {
    let c =
        compute("s = a b C ;\na = [ A ] ;\nb = { B } ;\nA = \"а\" ;\nB = \"б\" ;\nC = \"ц\" ;\n");
    let first = &c.analysis.first;
    assert_eq!(
        first["s"],
        ["A", "B", "C"].iter().map(|s| s.to_string()).collect()
    );
    assert!(c.analysis.nullable.contains("a") && c.analysis.nullable.contains("b"));
    assert!(!c.analysis.nullable.contains("s"));
    assert_eq!(
        c.analysis.follow["a"],
        ["B", "C"].iter().map(|s| s.to_string()).collect()
    );
    assert_eq!(
        c.analysis.follow["s"],
        [END].iter().map(|s| s.to_string()).collect()
    );
}

#[test]
fn an_undefined_name_a_duplicate_and_an_orphan_are_all_refused() {
    let undefined = parse_ebnf("s = a MISSING ;\na = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(structural_defects(&undefined)
        .iter()
        .any(|d| d.contains("MISSING") && d.contains("not defined")));
    let duplicate = parse_ebnf("s = a ;\na = X ;\na = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(structural_defects(&duplicate)
        .iter()
        .any(|d| d.contains("defined twice")));
    let orphan = parse_ebnf("s = a ;\na = X ;\nb = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(structural_defects(&orphan)
        .iter()
        .any(|d| d.contains("`b`") && d.contains("never used")));
    assert!(
        parse_ebnf("s = a b ;\na = ;\n").is_err(),
        "an empty alternative is a typo"
    );
    assert!(parse_ebnf("s = a (* never closed ;\n").is_err());
}

#[test]
fn a_reference_to_a_missing_function_is_refused() {
    let wrong_name =
        parse_ebnf("(* math.rs:parse_nothing:L10 *)\ns = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(reference_defects(&wrong_name)
        .iter()
        .any(|d| d.contains("no `fn parse_nothing`")));
    let wrong_line =
        parse_ebnf("(* math.rs:parse_add:L5000 *)\ns = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(reference_defects(&wrong_line)
        .iter()
        .any(|d| d.contains("the document says L5000")));
    let none = parse_ebnf("s = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(reference_defects(&none)
        .iter()
        .any(|d| d.contains("no reference to code")));
    let right = parse_ebnf("(* math.rs:parse_add:L270 *)\ns = X ;\nX = \"x\" ;\n").expect("parses");
    assert!(reference_defects(&right)
        .iter()
        .all(|d| !d.contains("parse_add")));
}

#[test]
fn the_line_tolerance_is_exactly_the_constant() {
    let source = read_file(&source_path("math.rs").expect("math.rs"));
    let (start, end) = function_span(&source, "parse_add").expect("parse_add exists");
    assert!(
        start > LINE_DRIFT_TOLERANCE + 1,
        "parse_add moved too close to the top of the file for this check"
    );
    let flagged = |line: usize| {
        let grammar = parse_ebnf(&format!(
            "(* math.rs:parse_add:L{line} *)\ns = X ;\nX = \"x\" ;\n"
        ))
        .expect("parses");
        reference_defects(&grammar)
            .iter()
            .any(|d| d.contains("the document says"))
    };
    assert!(!flagged(end + LINE_DRIFT_TOLERANCE));
    assert!(flagged(end + LINE_DRIFT_TOLERANCE + 1));
    assert!(!flagged(start - LINE_DRIFT_TOLERANCE));
    assert!(flagged(start - LINE_DRIFT_TOLERANCE - 1));
}

#[test]
fn a_phrase_invented_for_the_document_is_refused() {
    // «плюс» is in operators.yaml; «прибавить» is not.
    let invented = parse_ebnf(
        "(* operators.yaml:binary.plus math.rs:operator_tokens:L1242 *)\n\
         PLUS = \"плюс\" | \"прибавить\" ;\ns = PLUS ;\n",
    )
    .expect("parses");
    let defects = lexical_defects(&invented, false);
    assert!(
        defects.iter().any(|d| d.contains("прибавить")),
        "{defects:?}"
    );
    // And a phrase of the data file left out of the document.
    let forgotten = parse_ebnf(
        "(* operators.yaml:binary.times math.rs:operator_tokens:L1244 *)\n\
         TIMES = \"умножить\" ;\ns = TIMES ;\n",
    )
    .expect("parses");
    let defects = lexical_defects(&forgotten, false);
    assert!(
        defects.iter().any(|d| d.contains("умножить на")),
        "{defects:?}"
    );
}

#[test]
fn a_terminal_that_is_not_a_token_is_refused() {
    let source = math_source()
        .replace("\nFACT =\n", "\nFACTORIAL =\n")
        .replace("| FACT\n", "| FACTORIAL\n");
    let grammar = parse_ebnf(&source).expect("parses");
    let defects = lexical_defects(&grammar, true);
    assert!(
        defects
            .iter()
            .any(|d| d.contains("FACTORIAL") && d.contains("not a variant")),
        "{defects:?}"
    );
}

#[test]
fn a_changed_grammar_makes_the_embedded_tables_stale() {
    let doc = analysis_doc();
    // Remove a rule's alternative: the document must now disagree.
    let edited = compute(&math_source().replace("    | ELLIPSIS\n", ""));
    let defects = first_follow_block_defects(&doc, "math", &edited);
    assert!(
        defects.iter().any(|d| d.contains("is stale")),
        "an edited grammar passed against the unchanged document"
    );
    // And a document whose table was edited by hand.
    let computed = compute(&math_source());
    let doctored = doc.replacen("`FRAC_END`", "`FRAC_END_X`", 1);
    assert!(
        !first_follow_block_defects(&doctored, "math", &computed).is_empty()
            || !doctored.contains("`FRAC_END_X`"),
        "a hand-edited table passed"
    );
}

#[test]
fn removing_a_conflict_from_the_grammar_is_noticed() {
    let strict = compute(&with_mandatory_closers(&math_source()));
    let written = compute(&math_source());
    assert_ne!(conflict_ids(&strict), conflict_ids(&written));
    let doc = analysis_doc();
    assert!(
        !first_follow_block_defects(&doc, "math", &strict).is_empty(),
        "the document accepted a grammar in which the closer conflicts are gone"
    );
}

#[test]
fn a_wrong_quoted_output_is_noticed() {
    let mut table = probes(&analysis_doc());
    assert!(!table.is_empty(), "the document quotes no phrase");
    // Change one printed result by a single character.
    table[0].expected.push('!');
    let defects = probe_defects(&table);
    assert_eq!(defects.len(), 1, "{defects:?}");
    // And a phrase that the compiler reads differently from the claim.
    let wrong = Probe {
        domain: Domain::Mathematics,
        phrase: "икс плюс игрек".to_string(),
        expected: "x - y".to_string(),
    };
    assert_eq!(probe_defects(&[wrong]).len(), 1);
}

#[test]
fn a_document_without_the_summary_or_the_overlaps_is_refused() {
    assert!(!summary_block_defects("no blocks here").is_empty());
    assert!(!overlap_block_defects("no blocks here").is_empty());
    let doc = analysis_doc();
    let doctored = doc.replacen("| химия | имя вещества", "| химия | вещество", 1);
    assert_ne!(doc, doctored, "the overlaps block has no such row");
    assert!(!overlap_block_defects(&doctored).is_empty());
}

#[test]
fn the_function_word_letters_are_the_list_in_the_code() {
    let code = const_strings(
        &read_file(&source_path("chemistry.rs").expect("chemistry.rs")),
        "FUNCTION_WORD_LETTERS",
    );
    assert_eq!(code.len(), 11, "the code list changed size: {code:?}");
    for (file, source) in [("math.ebnf", math_source()), ("chem.ebnf", chem_source())] {
        let grammar = parse_ebnf(&source).expect("parses");
        let rule = grammar
            .rule(if file == "math.ebnf" {
                "WEAK_SYM"
            } else {
                "FUNCTION_LETTER"
            })
            .expect("the rule exists");
        let mut lits = Vec::new();
        lits_in(&rule.body, &mut lits);
        let in_doc: BTreeSet<String> = lits.into_iter().collect();
        assert_eq!(
            in_doc, code,
            "{file} lists other function words than the code"
        );
    }
}

#[test]
fn the_two_word_element_table_is_the_one_in_the_code() {
    let chem = parse_ebnf(&chem_source()).expect("parses");
    let rule = chem.rule("PAIR_TABLE").expect("PAIR_TABLE exists");
    let mut in_doc: BTreeSet<(String, String)> = BTreeSet::new();
    let Expr::Alt(alts) = &rule.body else {
        panic!("PAIR_TABLE is a list of pairs");
    };
    for alt in alts {
        let Expr::Seq(items) = alt else {
            panic!("a pair is two words");
        };
        let words: Vec<&str> = items
            .iter()
            .map(|i| match i {
                Expr::Lit(w) => w.as_str(),
                other => panic!("{other:?} in a pair"),
            })
            .collect();
        assert_eq!(words.len(), 2);
        in_doc.insert((words[0].to_string(), words[1].to_string()));
    }
    let source = read_file(&source_path("chemistry.rs").expect("chemistry.rs"));
    let (start, end) = function_span(&source, "chemistry_element_at").expect("exists");
    let mut in_code: BTreeSet<(String, String)> = BTreeSet::new();
    for line in source.lines().skip(start - 1).take(end - start + 1) {
        let Some((left, _)) = line.split_once("=> Some(") else {
            continue;
        };
        for part in left.split('|') {
            let quoted: Vec<&str> = part.split('"').skip(1).step_by(2).collect();
            if quoted.len() == 2 {
                in_code.insert((quoted[0].to_string(), quoted[1].to_string()));
            }
        }
    }
    assert!(!in_code.is_empty(), "no pair found in the code");
    assert_eq!(in_doc, in_code);
}

#[test]
fn the_conflict_table_is_not_empty_and_the_grammar_is_not_trivial() {
    // «0 of 0» is not a pass: a grammar of three rules would also be LL(1).
    let math = compute(&math_source());
    assert!(
        math.bnf.sources.len() >= 30,
        "math.ebnf has {} syntactic rules",
        math.bnf.sources.len()
    );
    assert!(!math.conflicts.is_empty());
    assert!(tokens_of(&math, "postfix~R1").contains("FACT"));
    let chem = compute(&chem_source());
    assert!(chem.bnf.sources.len() >= 10);
}

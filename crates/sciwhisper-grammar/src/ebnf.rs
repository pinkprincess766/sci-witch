//! A parser for the ISO 14977 subset used by `docs/grammar/math.ebnf`.
//!
//! Rules are `name = body ;`. Body operators: `|`, `( )`, `[ ]`, `{ }`,
//! quoted literals, special sequences `? … ?`, comments `(* … *)`.
//! Comments do not nest.

use std::collections::BTreeSet;

/// One EBNF expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Seq(Vec<Expr>),
    Alt(Vec<Expr>),
    Opt(Box<Expr>),
    Rep(Box<Expr>),
    Name(String),
    Lit(String),
    Special(String),
}

/// One `name = body ;` rule, with the comments that sat immediately before it.
#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub body: Expr,
    pub comments: Vec<String>,
    pub line: usize,
}

/// A parsed grammar: rules in file order.
#[derive(Clone, Debug)]
pub struct Grammar {
    pub rules: Vec<Rule>,
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

/// Parse a grammar written in the ISO 14977 subset described above.
pub fn parse(source: &str) -> Result<Grammar, String> {
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

/// A name that is a token class: ASCII uppercase, digits and underscore.
pub fn is_terminal(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
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
    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// Rules over tokens: lower-case names, minus the `guard_*` predicates.
    pub fn syntactic(&self) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| !is_terminal(&r.name) && !r.name.starts_with("guard_"))
            .collect()
    }

    /// The start symbol: the first syntactic rule in the file.
    pub fn start(&self) -> Option<&str> {
        self.syntactic().first().map(|r| r.name.as_str())
    }

    /// Quoted literals that appear in syntactic rules.
    ///
    /// `math.ebnf` has none: every `"…"` lives in a lexical rule, and the
    /// recogniser works over token-class names, not spoken phrases. A
    /// syntactic literal is treated as a terminal whose name is the literal
    /// text itself; it matches a token only if that token class happens to
    /// have the same name.
    pub fn syntactic_literals(&self) -> Vec<String> {
        let mut out = Vec::new();
        for rule in self.syntactic() {
            lits_in(&rule.body, &mut out);
        }
        let unique: BTreeSet<String> = out.into_iter().collect();
        unique.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{parse, Expr};

    #[test]
    fn parses_a_small_grammar() {
        let g = parse("s = a | b ;\na = \"x\" ;\nb = [ c ] ;").unwrap();
        assert_eq!(g.rules.len(), 3);
        assert_eq!(g.start(), Some("s"));
        match &g.rule("s").unwrap().body {
            Expr::Alt(alts) => assert_eq!(alts.len(), 2),
            other => panic!("{other:?}"),
        }
        assert_eq!(g.syntactic_literals(), ["x"]);
    }

    #[test]
    fn comments_and_specials_are_not_tokens() {
        let g = parse("(* hi *) N = ? a number ? | \"one\" ;").unwrap();
        assert_eq!(g.rules[0].name, "N");
        match &g.rules[0].body {
            Expr::Alt(alts) => {
                assert!(matches!(alts[0], Expr::Special(_)));
                assert!(matches!(alts[1], Expr::Lit(_)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unterminated_comment_is_refused() {
        let err = parse("S = A ; (* oops").unwrap_err();
        assert!(err.contains("unterminated comment"), "{err}");
    }

    #[test]
    fn math_ebnf_parses_and_starts_at_math_input() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/grammar/math.ebnf");
        let src =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let g = parse(&src).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(g.start(), Some("math_input"));
        assert!(
            g.syntactic_literals().is_empty(),
            "syntactic rules of math.ebnf have no quoted literals; they live in lexical rules. got {:?}",
            g.syntactic_literals()
        );
    }
}

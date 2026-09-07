use crate::ast::{Domain, InterpretationResult, Node, Renderer, UnresolvedSpan, Warning};
use crate::lexicon::Lexicon;
use crate::normalize::{normalize, words as split_words};
use crate::numbers::NumberLex;
use crate::parser::{math, parse_domain};
use crate::render;

#[derive(Clone, Debug)]
pub struct InterpretOptions {
    pub domain: Domain,
    pub allow_shortcuts: bool,
}

impl Default for InterpretOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            allow_shortcuts: true,
        }
    }
}

pub fn interpret(text: &str, opts: InterpretOptions) -> InterpretationResult {
    let lex = Lexicon::builtin();
    let nums = NumberLex::new();
    let normalized = normalize(text);
    let words = split_words(text);

    if words.is_empty() {
        return InterpretationResult::failed_raw(text, &normalized, opts.domain, "empty input");
    }

    if opts.allow_shortcuts {
        if let Some(sc) = lex.shortcut_exact(&normalized) {
            return InterpretationResult {
                ast: Node::Chemical(crate::ast::Chemical::Equation(sc.equation.clone())),
                raw_transcript: text.to_string(),
                normalized_transcript: normalized,
                domain: Domain::Chemistry,
                confidence: 1.0,
                unresolved_spans: vec![],
                warnings: vec![],
                alternatives: vec![],
            };
        }
    }

    // In `Auto` the domain and the parse are decided together: whichever
    // domain can actually read the phrase wins, and the keyword evidence only
    // breaks a tie. An explicit domain is honoured exactly as before.
    let (resolved, routed) = match opts.domain {
        Domain::Auto => route(&words, lex, &nums),
        other => (other, None),
    };
    let attempt = match routed {
        Some(node) => Ok(node),
        None => parse_domain(&words, resolved, lex, &nums),
    };

    match attempt {
        Ok(ast) => {
            let mut warnings = Vec::new();
            let mut alternatives = Vec::new();
            if resolved == Domain::Mathematics || resolved == Domain::Physics {
                if let Ok(p) = math::parse_math(
                    &words,
                    lex,
                    &nums,
                    if resolved == Domain::Physics {
                        math::MathMode::Physics
                    } else {
                        math::MathMode::Math
                    },
                ) {
                    warnings.extend(p.warnings.into_iter().map(|message| Warning {
                        code: "math".into(),
                        message,
                    }));
                    alternatives.extend(p.alternatives.into_iter().map(Node::Math));
                }
            }
            let structural_confidence = if warnings.is_empty() { 0.95 } else { 0.7 };
            warnings.extend(crate::validate::semantic_warnings(&ast));
            InterpretationResult {
                ast,
                raw_transcript: text.to_string(),
                normalized_transcript: normalized,
                domain: resolved,
                confidence: structural_confidence,
                unresolved_spans: vec![],
                warnings,
                alternatives,
            }
        }
        Err(e) => InterpretationResult {
            ast: Node::Text(text.to_string()),
            raw_transcript: text.to_string(),
            normalized_transcript: normalized,
            domain: resolved,
            confidence: 0.0,
            unresolved_spans: vec![UnresolvedSpan {
                text: text.to_string(),
                reason: e.to_string(),
            }],
            warnings: vec![Warning {
                code: "unresolved".into(),
                message: e.to_string(),
            }],
            alternatives: vec![],
        },
    }
}

pub fn format_text(text: &str, domain: Domain, renderer: Renderer) -> InterpretationResult {
    let r = interpret(
        text,
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    );
    let _ = renderer;
    r
}

pub fn render_result(r: &InterpretationResult, renderer: Renderer) -> String {
    if r.confidence <= 0.0 {
        return r.raw_transcript.clone();
    }
    render::render(&r.ast, renderer)
}

/// How much each domain is suggested by the words alone.
///
/// This is *evidence*, not a decision. Keyword counting cannot tell that
/// «вода» is a substance while «предел терпения» is not, so it is only ever
/// used to break a tie between readings that all actually parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Evidence {
    chemistry: i32,
    mathematics: i32,
    physics: i32,
}

impl Evidence {
    fn score(self, domain: Domain) -> i32 {
        match domain {
            Domain::Chemistry => self.chemistry,
            Domain::Mathematics => self.mathematics,
            Domain::Physics => self.physics,
            Domain::Auto | Domain::Plain => 0,
        }
    }

    /// The domain the words point at when nothing could be parsed. Only used
    /// to make a failure message name a sensible domain.
    fn best_guess(self) -> Domain {
        let mut best = Domain::Mathematics;
        let mut top = 0;
        // Fixed order, so the outcome never depends on iteration luck.
        for domain in [Domain::Chemistry, Domain::Physics, Domain::Mathematics] {
            let score = self.score(domain);
            if score > top {
                top = score;
                best = domain;
            }
        }
        best
    }
}

fn collect_evidence(words: &[String], lex: &Lexicon) -> Evidence {
    let mut evidence = Evidence::default();
    let mut index = 0;
    while index < words.len() {
        let word = words[index].as_str();
        // A known substance name is the strongest chemistry signal there is,
        // and it is the one the old scorer never asked about: «вода», «метан»
        // and «медный купорос» carry no keyword at all.
        if let Some((_, used)) = lex.longest_substance(words, index) {
            evidence.chemistry += 3 * used as i32;
            index += used;
            continue;
        }
        if matches!(
            word,
            "превращается"
                | "превращаются"
                | "окисляется"
                | "окисляются"
                | "восстанавливается"
                | "восстанавливаются"
                | "разлагается"
                | "разлагаются"
                | "реагирует"
                | "реагируют"
                | "взаимодействует"
                | "взаимодействуют"
                | "ион"
                | "кислота"
                | "кислоты"
                | "оксид"
                | "гидроксид"
                | "хлорид"
                | "сульфат"
                | "нитрат"
                | "карбонат"
                | "перманганат"
                | "осадок"
                | "стрелка"
        ) {
            evidence.chemistry += 3;
        }
        if lex.element(word).is_some() {
            evidence.chemistry += 1;
        }
        if lex.anion(word).is_some() {
            evidence.chemistry += 2;
        }
        if matches!(
            word,
            "дробь"
                | "числитель"
                | "знаменатель"
                | "интеграл"
                | "сумма"
                | "факториал"
                | "корень"
                | "синус"
                | "синуса"
                | "косинус"
                | "косинуса"
                | "тангенс"
                | "котангенс"
                | "логарифм"
                | "логарифма"
                | "экспонента"
                | "степени"
                | "квадрате"
                | "скобку"
                | "производная"
                | "производную"
                | "производной"
                | "частная"
                | "частную"
                | "частной"
                | "предел"
                | "предела"
                | "стремящемся"
                | "стремящейся"
                | "стремится"
                | "порядка"
        ) {
            evidence.mathematics += 3;
        }
        if matches!(word, "вектор" | "дельта") {
            // Δ and an arrow over a letter are physics notation. The old
            // scorer also gave chemistry a point here, which was enough to
            // send «дельта же равно минус эн эф е» to the wrong parser.
            evidence.physics += 3;
        }
        // A dictated unit is decisive: no other domain can even represent one.
        if let Some((_, used)) = lex.longest_unit(words, index) {
            evidence.physics += 4 * used as i32;
            index += used;
            continue;
        }
        index += 1;
    }
    evidence
}

/// Chooses a domain by trying them, not by guessing.
///
/// The old router scored keywords and committed to the winner, so a wrong
/// guess was fatal even when the right domain would have parsed the phrase
/// perfectly. Parsing is cheap here — one short utterance, three attempts —
/// and a reading that actually succeeds is worth more than any number of
/// keyword points.
///
/// Keyword evidence still decides between readings that all succeeded and
/// disagree, and it still picks the domain named in a failure message.
fn route(words: &[String], lex: &Lexicon, nums: &NumberLex) -> (Domain, Option<Node>) {
    let evidence = collect_evidence(words, lex);
    let mut parsed: Vec<(Domain, Node)> = Vec::new();
    for domain in [Domain::Chemistry, Domain::Mathematics, Domain::Physics] {
        if let Ok(node) = parse_domain(words, domain, lex, nums) {
            parsed.push((domain, node));
        }
    }
    match parsed.len() {
        0 => (evidence.best_guess(), None),
        1 => {
            let (domain, node) = parsed.remove(0);
            (domain, Some(node))
        }
        _ => {
            // Several domains produced something. If they agree, the choice is
            // cosmetic; if they disagree, the words decide.
            //
            // A tie is not a coin toss. The physics grammar is a superset of
            // the mathematics one, so «икс в квадрате» parses in both and
            // scores zero in both; calling that physics would label every
            // formula with a domain the speaker never invoked. On equal
            // evidence the narrower reading wins, which is the order the loop
            // above already uses — so the *first* maximum is taken, not the
            // last one `max_by_key` would return.
            let best = parsed
                .iter()
                .map(|(domain, _)| *domain)
                .fold(None::<Domain>, |best, domain| match best {
                    Some(current) if evidence.score(current) >= evidence.score(domain) => {
                        Some(current)
                    }
                    _ => Some(domain),
                })
                .unwrap_or(Domain::Mathematics);
            let node = parsed
                .into_iter()
                .find(|(domain, _)| *domain == best)
                .map(|(_, node)| node);
            (best, node)
        }
    }
}

/// Chemistry-only entry used by tests that must not fall back to math.
pub fn interpret_chemistry(text: &str) -> InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Chemistry,
            allow_shortcuts: true,
        },
    )
}

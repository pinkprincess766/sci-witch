//! speech → Whisper → interpret. Whisper never writes the formula itself.

use std::path::Path;

use sciwhisper_core::{interpret, render_result, Domain, InterpretOptions, Renderer};

use crate::capture;
use crate::engine::{AsrEngine, TranscribeOptions, Transcript};
use crate::error::Result;
use crate::prompt;
use crate::whisper_cli::WhisperCliEngine;

pub struct PipelineResult {
    pub transcript: Transcript,
    pub interpretation: sciwhisper_core::InterpretationResult,
    pub unicode: String,
    pub latex: String,
    pub omml: String,
}

pub struct PipelineOptions {
    pub domain: Domain,
    pub language: String,
    pub model: Option<String>,
    pub whisper_bin: Option<std::path::PathBuf>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            language: "ru".into(),
            model: None,
            whisper_bin: None,
        }
    }
}

pub fn from_audio(path: &Path, opts: PipelineOptions) -> Result<PipelineResult> {
    let wav = capture::ensure_wav_16k(path)?;
    transcribe_prepared(wav.path(), opts)
}

fn transcribe_prepared(path: &Path, opts: PipelineOptions) -> Result<PipelineResult> {
    let mut engine = engine_from(&opts)?;
    let asr_opts = TranscribeOptions {
        language: opts.language.clone(),
        model: engine.info.model.clone(),
        initial_prompt: prompt::for_domain(opts.domain),
        temperature: 0.0,
    };
    let transcript = engine.transcribe(path, &asr_opts)?;
    Ok(compile_transcript(transcript, opts.domain))
}

pub fn from_microphone(max_secs: Option<u64>, opts: PipelineOptions) -> Result<PipelineResult> {
    let rec = capture::record_wav(max_secs)?;
    eprintln!(
        "записано {:.1} с, peak {:.2} — Whisper…",
        rec.duration_secs, rec.peak
    );
    transcribe_prepared(&rec.wav_path, opts)
}

pub fn compile_transcript(transcript: Transcript, domain: Domain) -> PipelineResult {
    let interpretation = if transcript.no_speech || transcript.text.trim().is_empty() {
        sciwhisper_core::InterpretationResult::failed_raw(
            &transcript.text,
            "",
            domain,
            "silence or empty Whisper transcript",
        )
    } else {
        interpret(
            &transcript.text,
            InterpretOptions {
                domain,
                allow_shortcuts: true,
            },
        )
    };
    let mut unicode = render_result(&interpretation, Renderer::Unicode);
    let mut latex = render_result(&interpretation, Renderer::Latex);
    let mut omml = render_result(&interpretation, Renderer::Omml);

    // A recording may contain ordinary prose around one or more dictated
    // formulas. If parsing the whole utterance fails, preserve the prose and
    // compile only unambiguous scientific spans inside it.
    if interpretation.confidence <= 0.0 && domain != Domain::Plain {
        if let Some(compiled) = compile_inline_science(&transcript.text, Renderer::Unicode, domain)
        {
            unicode = compiled.clone();
            // Mixed prose cannot be represented by one native Word equation.
            // Unicode is therefore the safe Word fallback for this case.
            omml = compiled;
        }
        if let Some(compiled) = compile_inline_science(&transcript.text, Renderer::Latex, domain) {
            latex = compiled;
        }
    }

    PipelineResult {
        unicode,
        latex,
        omml,
        interpretation,
        transcript,
    }
}

fn compile_inline_science(text: &str, renderer: Renderer, requested: Domain) -> Option<String> {
    let spans = word_spans(text);
    let mut candidates_by_start = vec![Vec::new(); spans.len()];
    for (index, candidates) in candidates_by_start.iter_mut().enumerate() {
        for &domain in candidate_domains(requested) {
            let is_cue = match domain {
                Domain::Chemistry => is_chemistry_cue,
                Domain::Mathematics | Domain::Physics => is_mathematics_cue,
                Domain::Auto | Domain::Plain => continue,
            };
            if is_cue(text, &spans, index) {
                candidates.extend(generate_candidates(
                    text, &spans, index, domain, renderer, requested,
                ));
            }
        }
    }

    let selected = select_non_overlapping(candidates_by_start, spans.len());
    if selected.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (position, candidate) in selected.into_iter().enumerate() {
        let gap = &text[cursor..candidate.start_byte];
        if position > 0 {
            out.push_str(&render_scientific_gap(gap, renderer));
        } else {
            out.push_str(gap);
        }
        out.push_str(&candidate.rendered);
        cursor = candidate.end_byte;
    }
    out.push_str(&text[cursor..]);
    Some(out)
}

fn render_scientific_gap(gap: &str, renderer: Renderer) -> String {
    let words = sciwhisper_core::normalize::words(gap);
    let replacement = match words.as_slice() {
        [word] if word == "плюс" => Some(" + "),
        [word] if word == "минус" => Some(if renderer == Renderer::Unicode {
            " − "
        } else {
            " - "
        }),
        [word] if matches!(word.as_str(), "равно" | "равен" | "равна") => {
            Some(" = ")
        }
        [first, second]
            if matches!(first.as_str(), "умножить" | "умноженное") && second == "на" =>
        {
            Some(if renderer == Renderer::Latex {
                " \\cdot "
            } else {
                "·"
            })
        }
        _ => None,
    };
    replacement.unwrap_or(gap).to_string()
}

#[derive(Clone, Debug)]
struct ScienceCandidate {
    end_word: usize,
    start_byte: usize,
    end_byte: usize,
    rendered: String,
    score: i32,
}

fn candidate_domains(requested: Domain) -> &'static [Domain] {
    match requested {
        Domain::Auto => &[Domain::Chemistry, Domain::Mathematics, Domain::Physics],
        Domain::Chemistry => &[Domain::Chemistry],
        Domain::Mathematics => &[Domain::Mathematics],
        Domain::Physics => &[Domain::Physics],
        Domain::Plain => &[],
    }
}

fn generate_candidates(
    text: &str,
    spans: &[(usize, usize)],
    start_word: usize,
    domain: Domain,
    renderer: Renderer,
    requested: Domain,
) -> Vec<ScienceCandidate> {
    let start_byte = spans[start_word].0;
    // Spoken reactions routinely exceed 24 words once coefficients and
    // systematic names are included. Keep the search bounded, but large
    // enough to cover a practical dictated equation as one AST.
    let max_end = (start_word + 64).min(spans.len());
    let mut candidates = Vec::new();
    for end_word in (start_word + 1)..=max_end {
        let end_byte = spans[end_word - 1].1;
        let source = &text[start_byte..end_byte];
        if source
            .chars()
            .any(|character| matches!(character, '.' | '!' | '?' | ';' | ':'))
        {
            break;
        }
        if !candidate_boundary_is_safe(text, spans, end_word) {
            continue;
        }
        let parsed = interpret(
            source,
            InterpretOptions {
                domain,
                allow_shortcuts: true,
            },
        );
        // Inline rewriting is deliberately stricter than explicit CLI mode:
        // warned/ambiguous parses remain verbatim until the UI can ask.
        if parsed.confidence < 0.9 {
            continue;
        }
        let rendered = render_result(&parsed, renderer);
        if rendered.trim() == source.trim() {
            continue;
        }
        candidates.push(ScienceCandidate {
            end_word,
            start_byte,
            end_byte,
            rendered,
            score: candidate_score(source, end_word - start_word, domain, requested),
        });
    }
    candidates
}

fn candidate_boundary_is_safe(text: &str, spans: &[(usize, usize)], end_word: usize) -> bool {
    let previous =
        sciwhisper_core::normalize::words(&text[spans[end_word - 1].0..spans[end_word - 1].1]);
    if previous.first().is_some_and(|word| {
        matches!(
            word.as_str(),
            "в" | "на" | "с" | "к" | "по" | "из" | "до" | "от" | "плюс" | "минус"
        )
    }) {
        return false;
    }
    if end_word >= spans.len() {
        return true;
    }
    if previous.first().is_none_or(|word| word != "дельта") {
        return true;
    }
    let separator = &text[spans[end_word - 1].1..spans[end_word].0];
    if !separator.chars().all(char::is_whitespace) {
        return true;
    }
    !sciwhisper_core::normalize::words(&text[spans[end_word].0..spans[end_word].1])
        .first()
        .is_some_and(|word| {
            word.parse::<u32>().is_ok()
                || sciwhisper_core::numbers::NumberLex::new()
                    .lookup(word)
                    .is_some()
        })
}

fn candidate_score(source: &str, word_count: usize, domain: Domain, requested: Domain) -> i32 {
    let requested_bonus = if requested == domain { 40 } else { 0 };
    let domain_bonus = match domain {
        Domain::Chemistry => 30,
        Domain::Physics if looks_physical(source) => 35,
        Domain::Physics => 5,
        Domain::Mathematics => 20,
        Domain::Auto | Domain::Plain => 0,
    };
    // Opening a new scientific span has a fixed cost. Without it, three
    // one-token formula candidates could outscore one coherent formula.
    word_count as i32 * 100 + requested_bonus + domain_bonus - 80
}

fn looks_physical(source: &str) -> bool {
    let words = sciwhisper_core::normalize::words(source);
    words.iter().any(|word| word == "вектор")
        || words.iter().enumerate().any(|(index, _)| {
            sciwhisper_core::lexicon::Lexicon::builtin()
                .longest_unit(&words, index)
                .is_some()
        })
}

fn select_non_overlapping(
    candidates_by_start: Vec<Vec<ScienceCandidate>>,
    word_count: usize,
) -> Vec<ScienceCandidate> {
    let mut best_score = vec![0i32; word_count + 1];
    let mut choices: Vec<Option<ScienceCandidate>> = vec![None; word_count];

    for index in (0..word_count).rev() {
        best_score[index] = best_score[index + 1];
        for candidate in &candidates_by_start[index] {
            let total = candidate.score + best_score[candidate.end_word];
            if total > best_score[index] {
                best_score[index] = total;
                choices[index] = Some(candidate.clone());
            }
        }
    }

    let mut selected = Vec::new();
    let mut index = 0;
    while index < word_count {
        if let Some(candidate) = choices[index].clone() {
            index = candidate.end_word;
            selected.push(candidate);
        } else {
            index += 1;
        }
    }
    selected
}

fn is_mathematics_cue(text: &str, spans: &[(usize, usize)], index: usize) -> bool {
    let raw = &text[spans[index].0..spans[index].1];
    let normalized = sciwhisper_core::normalize::words(raw);
    let Some(word) = normalized.first() else {
        return false;
    };

    if word == "дельта" && following_word_is_number(text, spans, index) {
        return false;
    }

    if matches!(
        word.as_str(),
        "интеграл"
            | "сумма"
            | "произведение"
            | "корень"
            | "дробь"
            | "вектор"
            | "дельта"
            | "модуль"
            | "синус"
            | "косинус"
            | "тангенс"
            | "котангенс"
            | "логарифм"
            | "экспонента"
            | "экспоненту"
            | "экспоненты"
    ) {
        return true;
    }
    if index + 1 >= spans.len() {
        return false;
    }
    let separator = &text[spans[index].1..spans[index + 1].0];
    if !separator.chars().all(char::is_whitespace) {
        return false;
    }

    let next_raw = &text[spans[index + 1].0..spans[index + 1].1];
    let normalized_next = sciwhisper_core::normalize::words(next_raw);
    let Some(next) = normalized_next.first() else {
        return false;
    };
    let explicit_operator = matches!(
        next.as_str(),
        "равно"
            | "равен"
            | "равна"
            | "умножить"
            | "умноженное"
            | "умноженная"
            | "умноженный"
            | "умноженные"
            | "плюс"
            | "минус"
            | "разделить"
            | "деленное"
            | "делённое"
            | "индекс"
    );
    let exponent_phrase =
        matches!(next.as_str(), "в" | "во") && is_explicit_power_phrase(text, spans, index + 1);
    if !explicit_operator && !exponent_phrase {
        return false;
    }

    let lexicon = sciwhisper_core::lexicon::Lexicon::builtin();
    let symbol = lexicon.greek(word).is_some()
        || lexicon.latin(word).is_some()
        || (word.chars().count() == 1
            && word
                .chars()
                .all(|character| character.is_ascii_alphabetic()));
    let number = word.parse::<u32>().is_ok()
        || sciwhisper_core::numbers::NumberLex::new()
            .lookup(word)
            .is_some();
    symbol || number
}

fn following_word_is_number(text: &str, spans: &[(usize, usize)], index: usize) -> bool {
    let Some(next) = spans.get(index + 1) else {
        return false;
    };
    let separator = &text[spans[index].1..next.0];
    if !separator.chars().all(char::is_whitespace) {
        return false;
    }
    sciwhisper_core::normalize::words(&text[next.0..next.1])
        .first()
        .is_some_and(|word| {
            word.parse::<u32>().is_ok()
                || sciwhisper_core::numbers::NumberLex::new()
                    .lookup(word)
                    .is_some()
        })
}

fn is_explicit_power_phrase(text: &str, spans: &[(usize, usize)], preposition: usize) -> bool {
    let Some(next_span) = spans.get(preposition + 1) else {
        return false;
    };
    let separator = &text[spans[preposition].1..next_span.0];
    if !separator.chars().all(char::is_whitespace) {
        return false;
    }
    let normalized = sciwhisper_core::normalize::words(&text[next_span.0..next_span.1]);
    let Some(power_word) = normalized.first() else {
        return false;
    };
    if matches!(power_word.as_str(), "квадрате" | "кубе" | "степени") {
        return true;
    }

    let numbers = sciwhisper_core::numbers::NumberLex::new();
    if numbers.ordinal(power_word).is_none() {
        return false;
    }
    let Some(degree_span) = spans.get(preposition + 2) else {
        return false;
    };
    let separator = &text[next_span.1..degree_span.0];
    if !separator.chars().all(char::is_whitespace) {
        return false;
    }
    sciwhisper_core::normalize::words(&text[degree_span.0..degree_span.1])
        .first()
        .is_some_and(|word| word == "степени")
}

fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            spans.push((word_start, index));
        }
    }
    if let Some(word_start) = start {
        spans.push((word_start, text.len()));
    }
    spans
}

fn is_chemistry_cue(text: &str, spans: &[(usize, usize)], index: usize) -> bool {
    let word = &text[spans[index].0..spans[index].1];
    let normalized = sciwhisper_core::normalize::words(word);
    let Some(word) = normalized.first() else {
        return false;
    };
    let systematic = [
        "гидроксид",
        "оксид",
        "хлорид",
        "сульфат",
        "нитрат",
        "карбонат",
        "фосфат",
        "перманганат",
        "кислот",
        "ион",
    ]
    .iter()
    .any(|cue| word.starts_with(cue));
    let named_substance = sciwhisper_core::lexicon::Lexicon::builtin()
        .substances
        .iter()
        .flat_map(|item| item.names.iter())
        .filter_map(|name| name.split_whitespace().next())
        .any(|first| first == word);
    let stoichiometric_coefficient = is_number_word(word)
        && index + 1 < spans.len()
        && text[spans[index].1..spans[index + 1].0]
            .chars()
            .all(char::is_whitespace)
        && is_chemistry_cue(text, spans, index + 1);
    let spelled_formula = is_spelled_formula_start(text, spans, index, word);
    systematic || named_substance || stoichiometric_coefficient || spelled_formula
}

fn is_number_word(word: &str) -> bool {
    word.chars().all(|ch| ch.is_ascii_digit())
        || matches!(
            word,
            "ноль"
                | "один"
                | "одна"
                | "два"
                | "две"
                | "три"
                | "четыре"
                | "пять"
                | "шесть"
                | "семь"
                | "восемь"
                | "девять"
        )
}

fn is_spelled_formula_start(
    text: &str,
    spans: &[(usize, usize)],
    index: usize,
    word: &str,
) -> bool {
    let lexicon = sciwhisper_core::lexicon::Lexicon::builtin();
    if lexicon.element(word).is_none() || index + 1 >= spans.len() {
        return false;
    }

    let separator = &text[spans[index].1..spans[index + 1].0];
    if !separator.chars().all(char::is_whitespace) {
        return false;
    }

    let next = &text[spans[index + 1].0..spans[index + 1].1];
    let normalized_next = sciwhisper_core::normalize::words(next);
    let Some(next) = normalized_next.first() else {
        return false;
    };
    let number = is_number_word(next);
    let reaction_operator = matches!(
        next.as_str(),
        "плюс" | "реагирует" | "превращается" | "превращаются" | "образует"
    );
    lexicon.element(next).is_some() || number || reaction_operator
}

fn engine_from(opts: &PipelineOptions) -> Result<WhisperCliEngine> {
    if let Some(bin) = &opts.whisper_bin {
        let kind = if bin
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .contains("cli")
        {
            crate::engine::EngineKind::WhisperCpp
        } else {
            crate::engine::EngineKind::OpenaiWhisper
        };
        let model = opts.model.clone().unwrap_or_else(|| "base".into());
        return Ok(WhisperCliEngine::with_binary(bin.clone(), kind, model));
    }
    WhisperCliEngine::discover(opts.model.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeEngine;
    use crate::engine::{Segment, Transcript};

    #[test]
    fn whisper_text_is_compiled_not_pasted() {
        let mut fake = FakeEngine {
            transcript: Transcript {
                text: "гидроксид меди два".into(),
                language: Some("ru".into()),
                segments: vec![Segment {
                    text: "гидроксид меди два".into(),
                    start: Some(0.0),
                    end: Some(1.0),
                    no_speech_prob: Some(0.01),
                    avg_logprob: Some(-0.1),
                }],
                no_speech: false,
            },
        };
        let t = fake
            .transcribe(Path::new("unused.wav"), &TranscribeOptions::default())
            .unwrap();
        let r = compile_transcript(t, Domain::Chemistry);
        assert_eq!(r.unicode, "Cu(OH)₂");
        assert!(r.latex.contains("\\ce{"));
        assert_eq!(r.transcript.text, "гидроксид меди два");
    }

    #[test]
    fn silence_is_not_compiled() {
        let t = Transcript {
            text: String::new(),
            language: None,
            segments: vec![],
            no_speech: true,
        };
        let r = compile_transcript(t, Domain::Chemistry);
        assert!(r.interpretation.confidence <= 0.0);
    }

    #[test]
    fn chemistry_is_compiled_inside_ordinary_prose() {
        let raw = "Попытка вставки: на примере гидроксида железа три, оксида меди два или перманганата калия.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(
            r.unicode,
            "Попытка вставки: на примере Fe(OH)₃, CuO или KMnO₄."
        );
        assert_eq!(r.latex.matches("\\ce{").count(), 3);
        assert!(r.interpretation.confidence <= 0.0);
    }

    #[test]
    fn ordinary_prose_is_not_rewritten_inline() {
        let raw = "Это обычная фраза без научной формулы.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, raw);
        assert_eq!(r.latex, raw);
    }

    #[test]
    fn sourced_trivial_names_are_compiled_inside_prose() {
        let raw = "Примеры: павликова кислота, уксусная кислота, ацетон и глицерин.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, "Примеры: HF, CH₃COOH, CH₃COCH₃ и C₃H₈O₃.");
    }

    #[test]
    fn ambiguous_common_words_are_preserved() {
        let raw = "Добавим спирт и соду.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, raw);
    }

    #[test]
    fn observed_asr_aliases_compile_without_completing_an_unfinished_name() {
        let raw = "Попытка записи: пермангнат калия или же уксусная кислота, павликовая кислота. А может быть, этиловый...";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(
            r.unicode,
            "Попытка записи: KMnO₄ или же CH₃COOH, HF. А может быть, этиловый..."
        );
    }

    #[test]
    fn element_spelled_formula_is_compiled_inside_prose() {
        let raw = "Например, калий марганец о четыре.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, "Например, KMnO₄.");
    }

    #[test]
    fn mixed_science_dictation_compiles_only_the_proven_chemistry_span() {
        let raw = "Калий марганец о четыре превращается в железо, умноженное на дробь, в знаменателе экспонента, делённая на эн, а в числителе икс.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert!(r.unicode.starts_with("KMnO₄ → Fe"), "{}", r.unicode);
        assert!(r.unicode.contains("умноженное на дробь"), "{}", r.unicode);
        assert!(r.unicode.ends_with("а в числителе икс."), "{}", r.unicode);
    }

    #[test]
    fn observed_mixed_chemistry_and_math_are_compiled_inside_prose() {
        let raw = "Попытка записи: я хочу калий марганец о четыре. Затем возьмем, что пси в квадрате умножить на x в кубе равно 10 в четвертой степени. Следующее: уксусная кислота окисляется до аш два о.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert!(r.unicode.contains("KMnO₄"), "{}", r.unicode);
        assert!(r.unicode.contains("ψ²·x³ = 10⁴"), "{}", r.unicode);
        assert!(r.unicode.contains("CH₃COOH → H₂O"), "{}", r.unicode);
    }

    #[test]
    fn russian_prose_a_v_is_not_rewritten_as_latin_symbols() {
        let raw = "Это пояснение, а в числителе будет икс.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, raw);
    }

    #[test]
    fn observed_bare_delta_number_abstains_in_mixed_prose() {
        let raw = "Пусть феррит Zn плюс 10 в третьей степени, умноженное на дельта 3.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, "Пусть ZnFe₂O₄ + 10³, умноженное на дельта 3.");
    }

    #[test]
    fn explicit_delta_subscript_compiles_as_one_math_span() {
        let raw = "Пусть феррит Zn плюс 10 в третьей степени умноженное на дельта нижний индекс 3.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(r.unicode, "Пусть ZnFe₂O₄ + 10³·δ₃.");
    }

    #[test]
    fn observed_plural_reaction_compiles_and_ambiguous_tail_stays_raw() {
        let raw = "Два феррита Zn плюс калий марганец о четыре превращаются в оксид меди два плюс гидроксид железа три плюс гидроксид кобальта два плюс два гидроксид натрия, плюс партнер, кавликовая кислота.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(
            r.unicode,
            "2ZnFe₂O₄ + KMnO₄ → CuO + Fe(OH)₃ + Co(OH)₂ + 2NaOH, плюс партнер, HF."
        );
    }

    #[test]
    fn observed_long_math_dictation_compiles_as_structured_spans() {
        let raw = "Проверим математику: сета умноженное на 3x равно 10 в третьей степени плюс экспонента от икс деленное на x в квадрате, интеграл эф дэ икс.";
        let t = Transcript {
            text: raw.into(),
            language: Some("ru".into()),
            segments: vec![],
            no_speech: false,
        };
        let r = compile_transcript(t, Domain::Auto);
        assert_eq!(
            r.unicode,
            "Проверим математику: ζ·3x = 10³ + exp(x)/x², ∫ f dx."
        );
        assert!(r.latex.contains("\\frac{"), "{}", r.latex);
        assert_eq!(r.omml, r.unicode);
    }
}

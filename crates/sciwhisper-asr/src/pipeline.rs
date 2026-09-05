//! speech → Whisper → interpret. Whisper never writes the formula itself.

use std::path::Path;

use sciwhisper_core::{
    interpret_utterance, render, Domain, Renderer, UtteranceMode, UtteranceOptions,
};

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
    /// Whether the ordinary words around a formula are kept. The default is
    /// the safe one: keep everything, replace only what was proven.
    pub mode: UtteranceMode,
    pub language: String,
    pub model: Option<String>,
    pub whisper_bin: Option<std::path::PathBuf>,
    /// Input device name from `capture::input_devices()`; `None` uses the
    /// system default microphone. Ignored by `from_audio`.
    pub mic: Option<String>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            language: "ru".into(),
            model: None,
            whisper_bin: None,
            mic: None,
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
    Ok(compile_transcript_with(transcript, opts.domain, opts.mode))
}

pub fn from_microphone(max_secs: Option<u64>, opts: PipelineOptions) -> Result<PipelineResult> {
    let rec = capture::record_wav(max_secs, opts.mic.as_deref())?;
    eprintln!(
        "записано {:.1} с, peak {:.2} — Whisper…",
        rec.duration_secs, rec.peak
    );
    transcribe_prepared(&rec.wav_path, opts)
}

pub fn compile_transcript(transcript: Transcript, domain: Domain) -> PipelineResult {
    compile_transcript_with(transcript, domain, UtteranceMode::MixedText)
}

/// Compiles one transcript into every output format from **one** parse.
///
/// The utterance is read once, in `sciwhisper-core`, into a single document
/// AST. Unicode, LaTeX and OMML are then three views of that same structure,
/// so they cannot disagree about what was said.
pub fn compile_transcript_with(
    transcript: Transcript,
    domain: Domain,
    mode: UtteranceMode,
) -> PipelineResult {
    if transcript.no_speech || transcript.text.trim().is_empty() {
        let interpretation = sciwhisper_core::InterpretationResult::failed_raw(
            &transcript.text,
            "",
            domain,
            "silence or empty Whisper transcript",
        );
        let raw = transcript.text.clone();
        return PipelineResult {
            unicode: raw.clone(),
            latex: raw.clone(),
            omml: raw,
            interpretation,
            transcript,
        };
    }

    let utterance = interpret_utterance(
        &transcript.text,
        UtteranceOptions {
            domain,
            mode: if domain == Domain::Plain {
                UtteranceMode::MixedText
            } else {
                mode
            },
            allow_shortcuts: true,
        },
    );
    let unicode = render(&utterance.document, Renderer::Unicode);
    let latex = render(&utterance.document, Renderer::Latex);
    // One native Word equation cannot hold prose and formulas at once, so a
    // mixed document falls back to the same Unicode string the user sees
    // everywhere else. A purely scientific utterance still gets real OMML.
    let omml = if utterance.is_pure_science() {
        render(&utterance.document, Renderer::Omml)
    } else {
        unicode.clone()
    };

    PipelineResult {
        unicode,
        latex,
        omml,
        interpretation: utterance.to_interpretation(domain),
        transcript,
    }
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
        // Three spans compiled, so the utterance is a partial success and no
        // longer reports itself as a total failure.
        assert!(r.interpretation.confidence > 0.0);
        assert_eq!(r.interpretation.warnings.len(), 0);
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
        // The spoken «плюс» between two independent spans stays the word the
        // speaker said: MixedText keeps prose, and turning it into an operator
        // would need a per-renderer string table, which no longer exists.
        assert_eq!(r.unicode, "Пусть ZnFe₂O₄ плюс 10³, умноженное на дельта 3.");
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
        assert_eq!(r.unicode, "Пусть ZnFe₂O₄ плюс 10³·δ₃.");
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

mod collect_voice;
mod ingest;

use std::io::{self, Read};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use sciwhisper_asr::{from_audio, from_microphone, PipelineOptions, PipelineResult};
use sciwhisper_core::{
    choose_hypothesis, interpret, interpret_utterance, render, render_result, Domain,
    InterpretOptions, Renderer, UtteranceMode, UtteranceOptions,
};

#[derive(Parser)]
#[command(
    name = "sciwhisper",
    version,
    about = "Research CLI: text → scientific structure (compiler), Whisper transcription, voice-corpus collection",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Collect, resume and export a local voice session with explicit consent.
    CollectVoice(collect_voice::Args),
    /// Choose one hypothesis from a text n-best list, then compile it.
    ///
    /// The first argument is the recognizer's top hypothesis. A later one
    /// replaces it only when it is a small edit away and the only one the
    /// grammar can read as a whole utterance.
    Nbest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        hypotheses: Vec<String>,
    },
    /// Compile already-transcribed speech (bypass Whisper).
    Format {
        #[arg(long, default_value = "auto")]
        domain: String,
        /// mixed: keep the sentence, replace proven spans.
        /// scientific: drop a recognised dictation shell.
        #[arg(long, default_value = "mixed")]
        mode: String,
        #[arg(long, default_value = "unicode")]
        renderer: String,
        #[arg(long)]
        json: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
    /// Record from the microphone, run Whisper, compile the result.
    Rec {
        #[arg(long, default_value = "auto")]
        domain: String,
        #[arg(long, default_value = "unicode")]
        renderer: String,
        /// Stop after N seconds (Enter still stops earlier).
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        seconds: Option<u64>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "ru")]
        language: String,
        #[arg(long)]
        json: bool,
        /// Path to whisper / whisper-cli
        #[arg(long)]
        whisper: Option<PathBuf>,
        /// Input device name from `sciwhisper doctor` (default: system default microphone).
        #[arg(long)]
        mic: Option<String>,
    },
    /// Transcribe an audio file with Whisper, then compile.
    Transcribe {
        audio: PathBuf,
        #[arg(long, default_value = "auto")]
        domain: String,
        #[arg(long, default_value = "unicode")]
        renderer: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "ru")]
        language: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        whisper: Option<PathBuf>,
    },
    /// Show Whisper binary, backend and cached models.
    Doctor {
        /// Also hash the model file in full. Slower, and the only way to catch
        /// a file that is the right size but corrupted.
        #[arg(long)]
        verify_model: bool,
    },
    /// Run a local smoke test without microphone or network.
    SelfTest,
    /// Show representative chemistry, mathematics and physics conversions.
    Demo,
    /// Transcribe every audio file in a directory through Whisper + compiler.
    Corpus {
        dir: PathBuf,
        #[arg(long, default_value = "auto")]
        domain: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "ru")]
        language: String,
    },
    /// Fill a research corpus manifest from its recordings: measure each
    /// WAV and transcribe it. Consent, transcript and targets must already
    /// be in the manifest; this command never invents them.
    Ingest {
        /// JSONL manifest. Audio paths are relative to its directory.
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "ru")]
        language: String,
        /// Measure the audio, skip the recogniser.
        #[arg(long)]
        describe_only: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::CollectVoice(args) => collect_voice::run(args),
        Command::Nbest { hypotheses } => run_nbest(&hypotheses),
        Command::Format {
            domain,
            mode,
            renderer,
            json,
            text,
        } => run_format(&domain, &mode, &renderer, json, text),
        Command::Rec {
            domain,
            renderer,
            seconds,
            model,
            language,
            json,
            whisper,
            mic,
        } => {
            let renderer = parse_renderer_choice(&renderer)?;
            let domain: Domain = domain.parse()?;
            eprintln!("SciWhisper поверх Whisper. Домен: {}", domain.as_str());
            let result = from_microphone(
                seconds,
                PipelineOptions {
                    domain,
                    mode: UtteranceMode::MixedText,
                    language,
                    model,
                    whisper_bin: whisper,
                    mic,
                },
            )
            .map_err(|e| e.to_string())?;
            print_pipeline(&result, &renderer, json)
        }
        Command::Transcribe {
            audio,
            domain,
            renderer,
            model,
            language,
            json,
            whisper,
        } => {
            let renderer = parse_renderer_choice(&renderer)?;
            let domain: Domain = domain.parse()?;
            if !audio.exists() {
                return Err(format!("audio not found: {}", audio.display()));
            }
            eprintln!("Whisper ← {}", audio.display());
            let result = from_audio(
                &audio,
                PipelineOptions {
                    domain,
                    mode: UtteranceMode::MixedText,
                    language,
                    model,
                    whisper_bin: whisper,
                    mic: None,
                },
            )
            .map_err(|e| e.to_string())?;
            print_pipeline(&result, &renderer, json)
        }
        Command::Doctor { verify_model } => {
            let report = sciwhisper_asr::whisper_cli::DoctorReport::collect(verify_model);
            println!("{}", report.render());
            // Scripts check the exit code, so an incomplete setup has to fail
            // rather than merely print badly.
            match (&report.backend, report.model_ready) {
                (Err(reason), _) => Err(format!("движок распознавания не готов: {reason}")),
                // The reason is already worked out and printed above; repeating
                // a generic sentence here would contradict it.
                (Ok(_), false) => Err(format!("модель не готова: {}", report.model)),
                (Ok(_), true) => Ok(()),
            }
        }
        Command::SelfTest => run_self_test(),
        Command::Demo => run_demo(),
        Command::Corpus {
            dir,
            domain,
            model,
            language,
        } => run_corpus(dir, &domain, model, language),
        Command::Ingest {
            manifest,
            output,
            model,
            language,
            describe_only,
        } => run_ingest(manifest, output, model, language, describe_only),
    }
}

fn audio_file_label(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn run_corpus(
    dir: PathBuf,
    domain: &str,
    model: Option<String>,
    language: String,
) -> Result<(), String> {
    let domain: Domain = domain.parse()?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()).unwrap_or(""),
                "wav" | "mp3" | "m4a" | "caf" | "aiff" | "ogg" | "flac"
            )
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no audio files in {}", dir.display()));
    }
    println!("corpus: {} files in {}", files.len(), dir.display());
    let mut ok = 0usize;
    for f in &files {
        print!("{} … ", audio_file_label(f));
        match from_audio(
            f,
            PipelineOptions {
                domain,
                mode: UtteranceMode::MixedText,
                language: language.clone(),
                model: model.clone(),
                whisper_bin: None,
                mic: None,
            },
        ) {
            Ok(r) if r.transcript.no_speech => println!("silence"),
            Ok(r) => {
                ok += 1;
                println!("{} → {}", r.transcript.text, r.unicode);
            }
            Err(e) => println!("error: {e}"),
        }
    }
    println!("done: {ok}/{} transcribed", files.len());
    Ok(())
}

fn run_ingest(
    manifest: PathBuf,
    output: PathBuf,
    model: Option<String>,
    language: String,
    describe_only: bool,
) -> Result<(), String> {
    let options = ingest::IngestOptions {
        manifest,
        output,
        describe_only,
    };
    ingest::run(options, &mut |path| {
        from_audio(
            path,
            PipelineOptions {
                domain: Domain::Auto,
                mode: UtteranceMode::MixedText,
                language: language.clone(),
                model: model.clone(),
                whisper_bin: None,
                mic: None,
            },
        )
        .map(|result| result.transcript.text)
        .map_err(|e| e.to_string())
    })
}

fn preview_cases() -> [(Domain, &'static str, &'static str); 8] {
    [
        (Domain::Chemistry, "гидроксид меди два", "Cu(OH)₂"),
        (
            Domain::Chemistry,
            "гидроксид меди два превращается в оксид меди два плюс вода",
            "Cu(OH)₂ → CuO + H₂O",
        ),
        (
            Domain::Mathematics,
            "икс в квадрате плюс два икс минус три равно нулю",
            "x² + 2x − 3 = 0",
        ),
        (
            Domain::Mathematics,
            "интеграл от нуля до единицы икс в квадрате по икс",
            "∫₀¹ x² dx",
        ),
        (
            Domain::Physics,
            "лямбда равно шестьсот тридцать два нанометра",
            "λ = 632 нм",
        ),
        (Domain::Chemistry, "феррит Zn", "ZnFe₂O₄"),
        (
            Domain::Mathematics,
            "10 в третьей степени умноженное на икс",
            "10³·x",
        ),
        (
            Domain::Mathematics,
            "сета умноженное на три икс плюс экспонента от икс деленное на икс в квадрате",
            "ζ·3x + exp(x)/x²",
        ),
    ]
}

fn run_self_test() -> Result<(), String> {
    println!("SciWhisper self-test (локально, без микрофона и сети)");
    let mut passed = 0usize;
    for (domain, spoken, expected) in preview_cases() {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain,
                allow_shortcuts: true,
            },
        );
        let actual = render_result(&result, Renderer::Unicode);
        if result.confidence > 0.0 && actual == expected {
            passed += 1;
            println!("  OK  {spoken} → {actual}");
        } else {
            println!("  FAIL {spoken}");
            println!("       ожидалось: {expected}");
            println!("       получено:  {actual}");
        }
    }
    if passed == preview_cases().len() {
        println!("Готово: {passed}/{passed} проверок пройдено.");
        Ok(())
    } else {
        Err(format!(
            "self-test failed: {passed}/{} checks passed",
            preview_cases().len()
        ))
    }
}

fn run_demo() -> Result<(), String> {
    println!("SciWhisper technical preview\n");
    for (domain, spoken, _) in preview_cases() {
        let result = interpret(
            spoken,
            InterpretOptions {
                domain,
                allow_shortcuts: true,
            },
        );
        if result.confidence <= 0.0 {
            return Err(format!("demo phrase was not parsed: {spoken}"));
        }
        println!("[{}]", domain.as_str());
        println!("  сказано: {spoken}");
        println!("  Unicode: {}", render_result(&result, Renderer::Unicode));
        println!("  LaTeX:   {}", render_result(&result, Renderer::Latex));
        println!();
    }
    Ok(())
}

/// `all` prints every renderer. Anything else is one [`Renderer`], kept with
/// the spelling the user typed so the label line does not change.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RendererChoice {
    All,
    One { renderer: Renderer, spelled: String },
}

fn parse_renderer_choice(raw: &str) -> Result<RendererChoice, String> {
    if raw == "all" {
        return Ok(RendererChoice::All);
    }
    raw.parse::<Renderer>().map(|renderer| RendererChoice::One {
        renderer,
        spelled: raw.to_string(),
    })
}

/// Whisper heard nothing. The CLI does not insert text anywhere.
const NO_SPEECH_LINE: &str = "тишина — Whisper не дал текста, компилировать нечего";

fn print_pipeline(
    result: &PipelineResult,
    renderer: &RendererChoice,
    json: bool,
) -> Result<(), String> {
    if result.transcript.no_speech {
        eprintln!("{NO_SPEECH_LINE}");
        return Ok(());
    }
    if json {
        let v = serde_json::json!({
            "whisper": result.transcript.text,
            "language": result.transcript.language,
            "no_speech": result.transcript.no_speech,
            "domain": result.interpretation.domain.as_str(),
            "confidence": result.interpretation.confidence,
            "unicode": result.unicode,
            "latex": result.latex,
            "omml": result.omml,
            "warnings": result.interpretation.warnings,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    println!("whisper: {}", result.transcript.text);
    if result.interpretation.confidence <= 0.0 {
        eprintln!("не разобрано как научная конструкция — показан сырой транскрипт Whisper");
    }
    match renderer {
        RendererChoice::All => {
            println!("unicode: {}", result.unicode);
            println!("latex:   {}", result.latex);
            println!("omml:    {}", result.omml);
        }
        RendererChoice::One { renderer, spelled } => {
            let s = match renderer {
                Renderer::Unicode => &result.unicode,
                Renderer::Latex => &result.latex,
                Renderer::Omml => &result.omml,
            };
            println!("{spelled}: {s}");
        }
    }
    print_warnings(&result.interpretation.warnings);
    Ok(())
}

fn print_warnings(warnings: &[sciwhisper_core::ast::Warning]) {
    for warning in warnings {
        eprintln!("warning[{}]: {}", warning.code, warning.message);
    }
}

fn run_nbest(hypotheses: &[String]) -> Result<(), String> {
    if hypotheses.is_empty() {
        return Err("nbest needs at least one hypothesis".into());
    }
    let refs: Vec<&str> = hypotheses.iter().map(String::as_str).collect();
    let choice = choose_hypothesis(&refs);
    let text = hypotheses
        .get(choice.index)
        .map(String::as_str)
        .unwrap_or("");
    let reason = match choice.reason {
        sciwhisper_core::ChoiceReason::Empty => "empty",
        sciwhisper_core::ChoiceReason::TopAlreadyParsed => "top_already_parsed",
        sciwhisper_core::ChoiceReason::NearHypothesisParsed => "near_hypothesis_parsed",
        sciwhisper_core::ChoiceReason::NothingNearParsed => "nothing_near_parsed",
        sciwhisper_core::ChoiceReason::NearHypothesesDisagree => "near_hypotheses_disagree",
        sciwhisper_core::ChoiceReason::NoScientificAnchor => "no_scientific_anchor",
    };
    println!("hypothesis {}: {text}", choice.index);
    println!("reason: {reason}");
    let result = interpret(
        text,
        InterpretOptions {
            domain: Domain::Auto,
            allow_shortcuts: true,
        },
    );
    println!("{}", render_result(&result, Renderer::Unicode));
    print_warnings(&result.warnings);
    if result.confidence <= 0.0 {
        return Err("could not parse input; raw transcript preserved".into());
    }
    Ok(())
}

fn run_format(
    domain: &str,
    mode: &str,
    renderer: &str,
    json: bool,
    text: Vec<String>,
) -> Result<(), String> {
    let domain: Domain = domain.parse().map_err(|e: String| e)?;
    let mode: UtteranceMode = mode.parse().map_err(|e: String| e)?;
    let renderer = parse_renderer_choice(renderer)?;
    let spoken = if text.is_empty() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        buf
    } else {
        text.join(" ")
    };
    // One parse, then three views of the same structure.
    let utterance = interpret_utterance(
        spoken.trim(),
        UtteranceOptions {
            domain,
            mode,
            allow_shortcuts: true,
        },
    );
    let result = utterance.to_interpretation(domain);
    let show = |r: Renderer| render(&utterance.document, r);
    if json {
        let v = serde_json::json!({
            "domain": result.domain.as_str(),
            "mode": utterance.mode.as_str(),
            "decision": utterance.decision.as_str(),
            "confidence": result.confidence,
            "normalized": result.normalized_transcript,
            "unicode": show(Renderer::Unicode),
            "latex": show(Renderer::Latex),
            "omml": show(Renderer::Omml),
            "spans": utterance.spans.len(),
            "warnings": result.warnings,
            "unresolved": result.unresolved_spans,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    match &renderer {
        RendererChoice::All => {
            println!("unicode: {}", show(Renderer::Unicode));
            println!("latex:   {}", show(Renderer::Latex));
            println!("omml:    {}", show(Renderer::Omml));
        }
        RendererChoice::One { renderer, .. } => {
            println!("{}", show(*renderer));
        }
    }
    print_warnings(&result.warnings);
    if utterance.is_raw() {
        return Err("could not parse input; raw transcript preserved".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_says_there_is_nothing_to_compile() {
        assert_eq!(
            NO_SPEECH_LINE,
            "тишина — Whisper не дал текста, компилировать нечего"
        );
        assert!(!NO_SPEECH_LINE.contains("вставк"));
    }

    /// The eight phrases `self-test` and `demo` show must keep compiling to
    /// the strings written next to them; the commands print and exit, so
    /// without this nothing in the test run would notice a drift.
    #[test]
    fn the_self_test_and_the_demo_phrases_still_compile() {
        run_self_test().expect("self-test");
        run_demo().expect("demo");
    }

    #[test]
    fn a_corpus_path_without_a_file_name_does_not_panic() {
        assert_eq!(audio_file_label(std::path::Path::new("a/b.wav")), "b.wav");
        // `..` and `/` have no file name (`file_name` is None), so the old
        // unwrap panicked on both. The label is the path itself.
        assert_eq!(audio_file_label(std::path::Path::new("..")), "..");
        assert_eq!(audio_file_label(std::path::Path::new("/")), "/");
    }

    #[test]
    fn parse_renderer_choice_accepts_all_and_the_cli_spellings() {
        assert_eq!(parse_renderer_choice("all").unwrap(), RendererChoice::All);
        for (raw, renderer) in [
            ("unicode", Renderer::Unicode),
            ("tex", Renderer::Latex),
            ("word", Renderer::Omml),
        ] {
            match parse_renderer_choice(raw).unwrap() {
                RendererChoice::One {
                    renderer: got,
                    spelled,
                } => {
                    assert_eq!(got, renderer, "{raw}");
                    assert_eq!(spelled, raw);
                }
                RendererChoice::All => panic!("{raw} is one renderer"),
            }
        }
        assert!(parse_renderer_choice("bogus").is_err());
    }

    #[test]
    fn rec_rejects_zero_seconds_and_accepts_one() {
        assert!(Cli::try_parse_from(["sciwhisper", "rec", "--seconds", "0"]).is_err());
        let parsed = Cli::try_parse_from(["sciwhisper", "rec", "--seconds", "1"]).unwrap();
        match parsed.command {
            Command::Rec { seconds, .. } => assert_eq!(seconds, Some(1)),
            _ => panic!("expected rec"),
        }
    }

    #[test]
    fn format_rejects_an_unknown_domain_without_reading_stdin() {
        // cargo's own stdin is already at EOF, so a read would return at once
        // and the assertion would pass even if domain were parsed afterwards.
        // A pipe whose write end stays open blocks that read.
        let error = domain_error_with_blocked_stdin();
        assert!(error.contains("unknown domain"), "{error}");
        assert!(error.contains("'bad'"), "{error}");
    }

    fn domain_error_with_blocked_stdin() -> String {
        #[cfg(unix)]
        {
            use std::sync::mpsc;
            use std::thread;
            use std::time::Duration;

            extern "C" {
                fn pipe(fds: *mut i32) -> i32;
                fn dup(fd: i32) -> i32;
                fn dup2(oldfd: i32, newfd: i32) -> i32;
                fn close(fd: i32) -> i32;
            }

            struct StdinRestore {
                saved: i32,
                pipe: [i32; 2],
            }
            impl Drop for StdinRestore {
                fn drop(&mut self) {
                    unsafe {
                        dup2(self.saved, 0);
                        close(self.saved);
                        close(self.pipe[0]);
                        close(self.pipe[1]);
                    }
                }
            }

            unsafe {
                let mut pipe_fds = [0i32; 2];
                assert_eq!(pipe(pipe_fds.as_mut_ptr()), 0, "pipe");
                let saved = dup(0);
                assert!(saved >= 0, "dup stdin");
                assert_eq!(dup2(pipe_fds[0], 0), 0, "redirect stdin");
                let restore = StdinRestore {
                    saved,
                    pipe: pipe_fds,
                };
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let _ = tx.send(run_format("bad", "mixed", "unicode", false, vec![]));
                });
                let result = rx.recv_timeout(Duration::from_secs(2));
                drop(restore);
                match result {
                    Ok(Err(error)) => error,
                    Ok(Ok(())) => panic!("bad domain was accepted"),
                    Err(_) => panic!("run_format read stdin before checking the domain"),
                }
            }
        }
        #[cfg(not(unix))]
        {
            run_format("bad", "mixed", "unicode", false, vec![]).expect_err("bad domain")
        }
    }
}

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use sciwhisper_asr::{doctor, from_audio, from_microphone, PipelineOptions, PipelineResult};
use sciwhisper_core::{interpret, render_result, Domain, InterpretOptions, Renderer};
use sciwhisper_shell::config::Config;

#[derive(Parser)]
#[command(
    name = "sciwhisper",
    version,
    about = "Whisper overlay: speech → local Whisper → scientific notation"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Compile already-transcribed speech (bypass Whisper).
    Format {
        #[arg(long, default_value = "auto")]
        domain: String,
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
        #[arg(long)]
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
    Doctor,
    /// Run a local smoke test without microphone or network.
    SelfTest,
    /// Show representative chemistry, mathematics and physics conversions.
    Demo,
    /// System panel app. Press Control twice to start and twice again to insert.
    App,
    /// View or edit persistent SciWhisper settings.
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
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
}

#[derive(Subcommand)]
enum SettingsAction {
    /// Print the active configuration and its file path.
    Show,
    /// Change one setting, for example: settings set domain chemistry.
    Set { key: String, value: String },
    /// Run the interactive terminal setup assistant.
    Configure,
    /// Print the configuration file path.
    Path,
    /// Restore default settings. Requires --yes.
    Reset {
        #[arg(long)]
        yes: bool,
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
        None => sciwhisper_shell::run().map_err(|e| e.to_string()),
        Some(Command::Format {
            domain,
            renderer,
            json,
            text,
        }) => run_format(&domain, &renderer, json, text),
        Some(Command::Rec {
            domain,
            renderer,
            seconds,
            model,
            language,
            json,
            whisper,
            mic,
        }) => {
            let domain: Domain = domain.parse()?;
            eprintln!("SciWhisper поверх Whisper. Домен: {}", domain.as_str());
            let result = from_microphone(
                seconds,
                PipelineOptions {
                    domain,
                    language,
                    model,
                    whisper_bin: whisper,
                    mic,
                },
            )
            .map_err(|e| e.to_string())?;
            print_pipeline(&result, &renderer, json)
        }
        Some(Command::Transcribe {
            audio,
            domain,
            renderer,
            model,
            language,
            json,
            whisper,
        }) => {
            let domain: Domain = domain.parse()?;
            if !audio.exists() {
                return Err(format!("audio not found: {}", audio.display()));
            }
            eprintln!("Whisper ← {}", audio.display());
            let result = from_audio(
                &audio,
                PipelineOptions {
                    domain,
                    language,
                    model,
                    whisper_bin: whisper,
                    mic: None,
                },
            )
            .map_err(|e| e.to_string())?;
            print_pipeline(&result, &renderer, json)
        }
        Some(Command::Doctor) => {
            println!("{}", doctor());
            Ok(())
        }
        Some(Command::SelfTest) => run_self_test(),
        Some(Command::Demo) => run_demo(),
        Some(Command::App) => sciwhisper_shell::run().map_err(|e| e.to_string()),
        Some(Command::Settings { action }) => run_settings(action),
        Some(Command::Corpus {
            dir,
            domain,
            model,
            language,
        }) => run_corpus(dir, &domain, model, language),
    }
}

fn run_settings(action: Option<SettingsAction>) -> Result<(), String> {
    match action {
        Some(SettingsAction::Path) => {
            println!("{}", Config::path().display());
            Ok(())
        }
        Some(SettingsAction::Show) => show_settings(&Config::load().map_err(|e| e.to_string())?),
        Some(SettingsAction::Set { key, value }) => {
            let mut config = Config::load().map_err(|e| e.to_string())?;
            config.set(&key, &value).map_err(|e| e.to_string())?;
            config.save().map_err(|e| e.to_string())?;
            println!("Сохранено: {key} = {value}");
            println!("{}", Config::path().display());
            Ok(())
        }
        Some(SettingsAction::Reset { yes: true }) => {
            Config::default().save().map_err(|e| e.to_string())?;
            println!("Настройки восстановлены по умолчанию.");
            Ok(())
        }
        Some(SettingsAction::Reset { yes: false }) => {
            Err("reset отменён: добавьте --yes, чтобы подтвердить".into())
        }
        Some(SettingsAction::Configure) => configure_settings(),
        None if io::stdin().is_terminal() => configure_settings(),
        None => show_settings(&Config::load().map_err(|e| e.to_string())?),
    }
}

fn show_settings(config: &Config) -> Result<(), String> {
    println!("SciWhisper settings");
    println!("  config:          {}", Config::path().display());
    println!("  domain:          {}", config.domain);
    println!("  output:          {}", config.output);
    println!("  language:        {}", config.language);
    println!(
        "  model:           {}",
        config.model.as_deref().unwrap_or("default local model")
    );
    println!(
        "  mic:             {}",
        config.mic.as_deref().unwrap_or("системный по умолчанию")
    );
    println!("  ptt:             {}", config.ptt);
    println!("  double_control:  {}", config.double_control);
    println!("  ptt_latex:       {}", config.ptt_latex);
    println!("  ptt_word:        {}", config.ptt_word);
    println!("  persist_history: {}", config.persist_history);
    Ok(())
}

fn configure_settings() -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err(
            "interactive settings require a terminal; use `settings set <key> <value>`".into(),
        );
    }
    let mut config = Config::load().map_err(|e| e.to_string())?;
    println!("SciWhisper — помощница настройки");
    println!("Enter сохраняет текущее значение. '-' очищает путь модели.\n");

    let current = config.domain.clone();
    update_from_prompt(
        &mut config,
        "domain",
        "Домен [auto/chemistry/mathematics/physics/plain]",
        &current,
    )?;
    let current = config.output.clone();
    update_from_prompt(
        &mut config,
        "output",
        "Формат [auto/unicode/latex/word]",
        &current,
    )?;
    let current = config.language.clone();
    update_from_prompt(&mut config, "language", "Язык Whisper", &current)?;
    let current = config.model.clone().unwrap_or_else(|| "default".into());
    update_from_prompt(&mut config, "model", "Локальная модель", &current)?;
    configure_mic(&mut config)?;
    let current = config.ptt.clone();
    update_from_prompt(&mut config, "ptt", "Запись по удержанию клавиш", &current)?;
    let double_control = config.double_control.to_string();
    update_from_prompt(
        &mut config,
        "double_control",
        "Двойной Control запускает/останавливает запись [true/false]",
        &double_control,
    )?;
    let current = config.ptt_latex.clone();
    update_from_prompt(&mut config, "ptt_latex", "Быстрый LaTeX", &current)?;
    let current = config.ptt_word.clone();
    update_from_prompt(&mut config, "ptt_word", "Быстрый Word", &current)?;
    let history = config.persist_history.to_string();
    update_from_prompt(
        &mut config,
        "persist_history",
        "Хранить историю [true/false]",
        &history,
    )?;

    print!("\nСохранить настройки? [Y/n] ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "n" | "no") {
        println!("Изменения отменены.");
        return Ok(());
    }
    config.save().map_err(|e| e.to_string())?;
    println!("Сохранено в {}", Config::path().display());
    Ok(())
}

fn configure_mic(config: &mut Config) -> Result<(), String> {
    let devices = sciwhisper_asr::capture::input_devices();
    let current = config.mic.clone().unwrap_or_else(|| "по умолчанию".into());
    println!("Микрофон [{current}]:");
    println!("  0. системный по умолчанию");
    for (index, name) in devices.iter().enumerate() {
        println!("  {}. {name}", index + 1);
    }
    print!("Номер или название устройства (Enter — оставить текущее): ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|e| e.to_string())?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if let Ok(index) = value.parse::<usize>() {
        if index == 0 {
            return config.set("mic", "default").map_err(|e| e.to_string());
        }
        return match devices.get(index - 1) {
            Some(name) => config.set("mic", name).map_err(|e| e.to_string()),
            None => Err(format!("нет устройства с номером {index}")),
        };
    }
    config.set("mic", value).map_err(|e| e.to_string())
}

fn update_from_prompt(
    config: &mut Config,
    key: &str,
    label: &str,
    current: &str,
) -> Result<(), String> {
    print!("{label} [{current}]: ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|e| e.to_string())?;
    let value = value.trim();
    if !value.is_empty() {
        config.set(key, value).map_err(|e| e.to_string())?;
    }
    Ok(())
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
        print!("{} … ", f.file_name().unwrap().to_string_lossy());
        match from_audio(
            f,
            PipelineOptions {
                domain,
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

fn print_pipeline(result: &PipelineResult, renderer: &str, json: bool) -> Result<(), String> {
    if result.transcript.no_speech {
        eprintln!("тишина — Whisper не дал текста, вставка пропущена");
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
        "all" => {
            println!("unicode: {}", result.unicode);
            println!("latex:   {}", result.latex);
            println!("omml:    {}", result.omml);
        }
        other => {
            let r: Renderer = other.parse()?;
            let s = match r {
                Renderer::Unicode => &result.unicode,
                Renderer::Latex => &result.latex,
                Renderer::Omml => &result.omml,
            };
            println!("{other}: {s}");
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

fn run_format(domain: &str, renderer: &str, json: bool, text: Vec<String>) -> Result<(), String> {
    let spoken = if text.is_empty() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        buf
    } else {
        text.join(" ")
    };
    let domain: Domain = domain.parse().map_err(|e: String| e)?;
    let result = interpret(
        spoken.trim(),
        InterpretOptions {
            domain,
            allow_shortcuts: true,
        },
    );
    if json {
        let v = serde_json::json!({
            "domain": result.domain.as_str(),
            "confidence": result.confidence,
            "normalized": result.normalized_transcript,
            "unicode": render_result(&result, Renderer::Unicode),
            "latex": render_result(&result, Renderer::Latex),
            "omml": render_result(&result, Renderer::Omml),
            "warnings": result.warnings,
            "unresolved": result.unresolved_spans,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    match renderer {
        "all" => {
            println!("unicode: {}", render_result(&result, Renderer::Unicode));
            println!("latex:   {}", render_result(&result, Renderer::Latex));
            println!("omml:    {}", render_result(&result, Renderer::Omml));
        }
        other => {
            let r: Renderer = other.parse().map_err(|e: String| e)?;
            println!("{}", render_result(&result, r));
        }
    }
    print_warnings(&result.warnings);
    if result.confidence <= 0.0 {
        return Err("could not parse input; raw transcript preserved".into());
    }
    Ok(())
}

//! End-to-end checks for the autonomous Windows voice pack.
//!
//! Nothing here needs a microphone, a real Whisper model or a network. The
//! backend is a tiny script that behaves the way `whisper-cli` does, so the
//! rules around it — discovery order, deadlines, output ceilings and the
//! deletion of temporary audio — can be exercised on any machine.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sciwhisper_asr::backend::{self, backend_file_name, BackendOrigin, Layout, BUNDLE_MARKER};
use sciwhisper_asr::engine::{AsrEngine, EngineKind, TranscribeOptions};
use sciwhisper_asr::error::Error;
use sciwhisper_asr::model::{self, ModelStatus, Requirements, MANIFEST_FILE};
use sciwhisper_asr::process::{self, Limits};
use sciwhisper_asr::whisper_cli::WhisperCliEngine;

// ------------------------------------------------------------ fake backend

/// Writes a stand-in for `whisper-cli` that follows the same command line.
fn fake_whisper(dir: &Path, body: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let path = dir.join("whisper-cli.cmd");
        std::fs::write(&path, body).unwrap();
        path
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("whisper-cli");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

/// whisper.cpp writes `<-of>.txt`; this reproduces that contract.
#[cfg(not(windows))]
const WRITES_TRANSCRIPT: &str = r#"
of=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then of="$2"; fi
  shift
done
printf 'гидроксид меди два' > "$of.txt"
exit 0
"#;
#[cfg(windows)]
const WRITES_TRANSCRIPT: &str = r#"@echo off
set "of="
:loop
if "%~1"=="" goto done
if "%~1"=="-of" set "of=%~2"
shift
goto loop
:done
>"%of%.txt" echo|set /p=гидроксид меди два
exit /b 0
"#;

#[cfg(not(windows))]
const FAILS: &str = "echo 'error: failed to load model' 1>&2\nexit 4\n";
#[cfg(windows)]
const FAILS: &str = "@echo error: failed to load model 1>&2\r\nexit /b 4\r\n";

#[cfg(not(windows))]
const HANGS: &str = "sleep 60\n";
#[cfg(windows)]
const HANGS: &str = "@ping -n 60 127.0.0.1 > nul\r\n";

#[cfg(not(windows))]
const SUCCEEDS_SILENTLY: &str = "exit 0\n";
#[cfg(windows)]
const SUCCEEDS_SILENTLY: &str = "@exit /b 0\r\n";

// ------------------------------------------------------------ bundle setup

struct Bundle {
    _dir: tempfile::TempDir,
    layout: Layout,
}

impl Bundle {
    /// A directory shaped like a shipped bundle.
    fn new(body: &str, with_model: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(BUNDLE_MARKER), "sci-witch").unwrap();
        let whisper = dir.path().join("whisper");
        std::fs::create_dir_all(&whisper).unwrap();
        let produced = fake_whisper(&whisper, body);
        // The application looks for an exact file name; on Unix the fake is
        // already called that, on Windows the script keeps its .cmd suffix and
        // is copied into place.
        let wanted = whisper.join(backend_file_name());
        if produced != wanted {
            std::fs::copy(&produced, &wanted).unwrap();
        }
        if with_model {
            let bytes = b"pretend ggml weights".to_vec();
            std::fs::write(whisper.join("ggml-small-q5_1.bin"), &bytes).unwrap();
            let sha = model::sha256_hex(&bytes);
            std::fs::write(
                whisper.join(MANIFEST_FILE),
                format!(
                    r#"{{"schema_version":1,"model_id":"small-q5_1","file":"ggml-small-q5_1.bin","size_bytes":{},"sha256":"{sha}","source_url":"example.invalid/m.bin","license":"MIT","license_url":"example.invalid/L","whisper_cpp_version":"v1.7.4","verified_against_repository_pin":true,"notes":""}}"#,
                    bytes.len()
                ),
            )
            .unwrap();
        }
        Bundle {
            layout: Layout::from_dir(dir.path()),
            _dir: dir,
        }
    }

    fn engine(&self) -> WhisperCliEngine {
        let backend = backend::discover_in(Some(&self.layout), None, false, |_| None).unwrap();
        assert_eq!(backend.origin, BackendOrigin::Bundled);
        WhisperCliEngine::from_backend(backend, None).with_layout(Some(self.layout.clone()))
    }
}

fn wav(dir: &Path) -> PathBuf {
    let path = dir.join("запись 1.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for index in 0..1600 {
        writer.write_sample(((index % 100) as i16) * 100).unwrap();
    }
    writer.finalize().unwrap();
    path
}

// -------------------------------------------------------- discovery order

#[test]
fn a_bundled_backend_is_found_and_labelled_as_bundled() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let engine = bundle.engine();
    assert_eq!(engine.origin, BackendOrigin::Bundled);
    assert_eq!(engine.info.kind, EngineKind::WhisperCpp);
}

#[test]
fn a_bundle_without_its_backend_reports_a_packaging_fault() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(BUNDLE_MARKER), "sci-witch").unwrap();
    let layout = Layout::from_dir(dir.path());
    // Even with a Python whisper available, the bundle must not use it.
    let error = backend::discover_in(Some(&layout), None, true, |name| {
        (name == "whisper").then(|| PathBuf::from("/usr/bin/whisper"))
    })
    .expect_err("an incomplete bundle is an error");
    let message = error.to_string();
    assert!(matches!(error, Error::BundleIncomplete { .. }), "{message}");
    assert!(message.contains("whisper-cli"), "{message}");
}

// ------------------------------------------------------------- model state

#[test]
fn a_bundle_without_a_model_explains_what_to_download() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, false);
    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let audio = wav(dir.path());
    let error = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect_err("a missing model must stop before the backend runs");
    let message = error.to_string();
    assert!(matches!(error, Error::ModelUnusable { .. }), "{message}");
    assert!(message.contains("model pack"), "{message}");
    // The message must not send a non-programmer to Python or to a download.
    assert!(!message.to_lowercase().contains("python"), "{message}");
    assert!(!message.contains("http"), "{message}");
}

#[test]
fn a_corrupted_model_is_reported_rather_than_loaded() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let model_file = bundle.layout.whisper_dir().join("ggml-small-q5_1.bin");
    std::fs::write(&model_file, b"truncated").unwrap();
    let requirements = Requirements::for_layout(Some(&bundle.layout), false);
    match model::check(&bundle.layout.whisper_dir(), requirements) {
        ModelStatus::SizeMismatch { .. } => {}
        other => panic!("{other:?}"),
    }
    assert!(model::resolve(Some(&bundle.layout), None, requirements).is_err());
}

// ------------------------------------------------------ process behaviour

#[test]
fn a_successful_run_returns_the_transcript_from_a_path_with_spaces_and_cyrillic() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let odd = dir.path().join("Мои записи").join("сеанс 1");
    std::fs::create_dir_all(&odd).unwrap();
    let audio = wav(&odd);
    let transcript = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect("the fake backend must be reachable through an odd path");
    assert_eq!(transcript.text, "гидроксид меди два");
    assert!(!transcript.no_speech);
}

#[test]
fn a_failing_backend_reports_its_own_last_words() {
    let bundle = Bundle::new(FAILS, true);
    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let audio = wav(dir.path());
    let error = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect_err("a non-zero exit must surface");
    match error {
        Error::BackendFailed { code, detail } => {
            assert_eq!(code, Some(4));
            assert!(detail.contains("failed to load model"), "{detail}");
        }
        other => panic!("{other}"),
    }
}

#[test]
fn a_backend_that_says_nothing_is_not_mistaken_for_silence() {
    let bundle = Bundle::new(SUCCEEDS_SILENTLY, true);
    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let audio = wav(dir.path());
    let error = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect_err("no output at all is a fault, not an empty transcript");
    assert!(matches!(error, Error::OutputUnreadable { .. }), "{error}");
}

#[test]
fn a_hanging_backend_is_stopped_at_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let script = fake_whisper(dir.path(), HANGS);
    let started = std::time::Instant::now();
    let error = process::run(
        Command::new(&script),
        Limits {
            timeout: Duration::from_secs(1),
            max_output_bytes: process::MAX_OUTPUT_BYTES,
        },
    )
    .expect_err("the deadline must be enforced");
    assert!(
        matches!(error, Error::BackendTimedOut { seconds: 1 }),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(20));
}

// ------------------------------------------------- temporary audio hygiene

/// Runs `operation` over a freshly written temporary WAV and asserts that the
/// directory holding it is gone once the operation returns, whatever the
/// outcome was. The exact directory is tracked rather than scanning the system
/// temp folder, so the check is exact and unaffected by tests running in
/// parallel.
fn audio_is_removed_after(operation: impl FnOnce(&Path)) {
    let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0).sin() * 0.5).collect();
    let directory;
    {
        let prepared = sciwhisper_asr::capture::write_temp_wav(&samples, 16_000).unwrap();
        assert!(prepared.owns_temp_dir());
        directory = prepared
            .path()
            .parent()
            .expect("the audio lives inside its own directory")
            .to_path_buf();
        assert!(prepared.path().is_file());
        operation(prepared.path());
    }
    assert!(
        !directory.exists(),
        "temporary audio survived at {}",
        directory.display()
    );
}

#[test]
fn the_temporary_wav_is_owned_by_a_guard_and_dies_with_it() {
    let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0).sin() * 0.5).collect();
    let path;
    {
        let prepared = sciwhisper_asr::capture::write_temp_wav(&samples, 16_000).unwrap();
        assert!(prepared.owns_temp_dir(), "the recording must own its file");
        path = prepared.path().to_path_buf();
        assert!(path.is_file(), "the audio exists while it is needed");
    }
    // No explicit cleanup call anywhere: the guard going out of scope is the
    // only mechanism, so no error path can skip it.
    assert!(!path.exists(), "the audio outlived its guard");
}

#[test]
fn cancelling_a_real_capture_session_writes_no_audio() {
    // Needs an input device. Where there is none — a headless CI runner — the
    // test says so instead of pretending to have proved something.
    if sciwhisper_asr::capture::input_devices().is_empty() {
        eprintln!("skipped: no input device on this machine");
        return;
    }
    let session = match sciwhisper_asr::PttSession::start(None) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("skipped: could not open a capture session: {error}");
            return;
        }
    };
    let before = sciwhisper_temp_dirs();
    std::thread::sleep(Duration::from_millis(120));

    // This is the Esc path: the session is abandoned, not finished.
    session.cancel();

    let after = sciwhisper_temp_dirs();
    let new_dirs: Vec<&PathBuf> = after.iter().filter(|path| !before.contains(path)).collect();
    assert!(
        new_dirs.is_empty(),
        "a cancelled session created {new_dirs:?}; audio is only written by finish()"
    );
}

/// Temporary directories this application creates, for the cancel test — which
/// cannot know a path in advance, because a cancelled session must never make
/// one.
fn sciwhisper_temp_dirs() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sciwhisper-"))
        })
        .collect()
}

#[test]
fn temporary_audio_is_removed_after_a_successful_transcription() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let mut engine = bundle.engine();
    audio_is_removed_after(|audio| {
        let transcript = engine
            .transcribe(audio, &TranscribeOptions::default())
            .expect("the fake backend succeeds");
        assert_eq!(transcript.text, "гидроксид меди два");
    });
}

#[test]
fn temporary_audio_is_removed_after_a_failed_backend_launch() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    // Point the engine at a backend that does not exist on disk.
    let mut engine = WhisperCliEngine::with_binary(
        bundle.layout.whisper_dir().join("not-installed"),
        EngineKind::WhisperCpp,
        String::new(),
    )
    .with_layout(Some(bundle.layout.clone()));
    audio_is_removed_after(|audio| {
        let error = engine
            .transcribe(audio, &TranscribeOptions::default())
            .expect_err("a missing executable cannot run");
        assert!(
            matches!(error, Error::BackendFailed { code: None, .. }),
            "{error}"
        );
    });
}

#[test]
fn temporary_audio_is_removed_after_a_nonzero_exit_code() {
    let bundle = Bundle::new(FAILS, true);
    let mut engine = bundle.engine();
    audio_is_removed_after(|audio| {
        let error = engine
            .transcribe(audio, &TranscribeOptions::default())
            .expect_err("exit code 4");
        assert!(
            matches!(error, Error::BackendFailed { code: Some(4), .. }),
            "{error}"
        );
    });
}

#[test]
fn temporary_audio_is_removed_after_a_timeout() {
    let bundle = Bundle::new(HANGS, true);
    // A one-second deadline for this engine only, so the limit is a parameter
    // rather than a process-wide environment variable other tests could see.
    let mut engine = bundle.engine().with_limits(Limits {
        timeout: Duration::from_secs(1),
        max_output_bytes: process::MAX_OUTPUT_BYTES,
    });
    audio_is_removed_after(|audio| {
        let error = engine.transcribe(audio, &TranscribeOptions::default());
        assert!(
            matches!(error, Err(Error::BackendTimedOut { .. })),
            "{error:?}"
        );
    });
}

#[test]
fn temporary_audio_is_removed_after_an_unreadable_result() {
    let bundle = Bundle::new(SUCCEEDS_SILENTLY, true);
    let mut engine = bundle.engine();
    audio_is_removed_after(|audio| {
        let error = engine
            .transcribe(audio, &TranscribeOptions::default())
            .expect_err("no transcript file and no stdout");
        assert!(matches!(error, Error::OutputUnreadable { .. }), "{error}");
    });
}

#[test]
fn temporary_audio_is_removed_when_the_model_is_missing() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, false);
    let mut engine = bundle.engine();
    audio_is_removed_after(|audio| {
        let error = engine
            .transcribe(audio, &TranscribeOptions::default())
            .expect_err("a missing model stops before the backend runs");
        assert!(matches!(error, Error::ModelUnusable { .. }), "{error}");
    });
}

/// Crates that would give this layer the ability to reach the network. If one
/// of these ever appears in the manifest, the offline guarantee is gone and
/// this test is how we find out.
const NETWORK_CRATES: [&str; 14] = [
    "reqwest",
    "ureq",
    "hyper",
    "curl",
    "isahc",
    "attohttpc",
    "surf",
    "minreq",
    "http",
    "tokio",
    "async-std",
    "socket2",
    "rustls",
    "native-tls",
];

#[test]
fn the_recogniser_has_no_way_to_reach_the_network() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // 1. No dependency that could open a connection. Reading the manifest
    //    catches a crate added tomorrow, which grepping the sources would not.
    let manifest = std::fs::read_to_string(crate_root.join("Cargo.toml")).unwrap();
    let declared: Vec<String> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && line.contains('='))
        .filter_map(|line| line.split('=').next())
        .map(|name| name.trim().trim_matches('"').to_string())
        .collect();
    for forbidden in NETWORK_CRATES {
        assert!(
            !declared.iter().any(|name| name == forbidden),
            "sciwhisper-asr must not depend on {forbidden}"
        );
    }

    // 2. No socket in the sources either, in case something arrives through a
    //    transitive re-export.
    let mut offenders = Vec::new();
    let mut stack = vec![crate_root.join("src")];
    let mut files = 0usize;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in [
                "std::net",
                "TcpStream",
                "TcpListener",
                "UdpSocket",
                "ToSocketAddrs",
                "reqwest",
                "ureq",
                "hyper::",
                "curl",
                "http://",
                "https://",
            ] {
                // Documentation may cite a URL; code may not use one.
                for (number, line) in text.lines().enumerate() {
                    let code = line.trim_start();
                    if code.starts_with("//") || code.starts_with("*") {
                        continue;
                    }
                    if code.contains(needle) {
                        offenders.push(format!(
                            "{}:{}: {needle}",
                            path.file_name().unwrap().to_string_lossy(),
                            number + 1
                        ));
                    }
                }
            }
        }
    }
    assert!(files > 5, "the scan found almost no sources: {files}");
    assert!(offenders.is_empty(), "network use found: {offenders:?}");
}

// --------------------------------------------------------- bundle contents

#[derive(serde::Deserialize)]
struct BundleContents {
    schema_version: u32,
    required_files: Vec<String>,
    required_dirs: Vec<String>,
    model_pack_files: Vec<String>,
}

fn bundle_contents() -> BundleContents {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows/BUNDLE_CONTENTS.json");
    let text = std::fs::read_to_string(&path).expect("BUNDLE_CONTENTS.json must exist");
    serde_json::from_str(&text).expect("BUNDLE_CONTENTS.json must be valid")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Where a required bundle entry comes from in this repository. `None` means
/// it is produced by the build rather than copied.
fn source_of(entry: &str) -> Option<PathBuf> {
    let root = repo_root();
    match entry {
        // Built, not stored: these three exist only after a compile, so the
        // check for them is that the release workflow names them.
        "sciwhisper.exe" => None,
        "sciwhisper-updater.exe" => None,
        "whisper/whisper-cli.exe" => None,
        name if name.ends_with(".cmd") || name.ends_with(".ps1") => {
            Some(root.join("packaging/windows").join(name))
        }
        "README-WINDOWS.txt" => Some(root.join("packaging/windows/README-WINDOWS.txt")),
        other => Some(root.join(other)),
    }
}

#[test]
fn every_required_bundle_file_exists_in_the_repository_and_is_copied_by_ci() {
    let contents = bundle_contents();
    let workflow =
        std::fs::read_to_string(repo_root().join(".github/workflows/release.yml")).unwrap();
    let mut problems = Vec::new();

    for entry in &contents.required_files {
        match source_of(entry) {
            None => {
                // Built, not copied: the workflow must still name it somewhere.
                let base = entry.rsplit('/').next().unwrap();
                if !workflow.contains(base) {
                    problems.push(format!("{entry}: the release workflow never mentions it"));
                }
            }
            Some(path) => {
                match std::fs::metadata(&path) {
                    Ok(meta) if meta.len() > 100 => {}
                    Ok(meta) => problems.push(format!(
                        "{entry}: source is only {} bytes, that is a stub",
                        meta.len()
                    )),
                    Err(_) => problems.push(format!(
                        "{entry}: no source in the repository at {}",
                        path.display()
                    )),
                }
                let base = entry.rsplit('/').next().unwrap();
                if !workflow.contains(base) {
                    problems.push(format!("{entry}: the release workflow never copies it"));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "bundle manifest and repository disagree: {problems:#?}"
    );
}

#[test]
fn the_bundle_manifest_describes_a_layout_the_application_accepts() {
    let contents = bundle_contents();
    assert_eq!(contents.schema_version, 1);
    // The marker the application uses to tell a bundle from a build tree must
    // be one of the files the release is required to ship.
    assert!(
        contents.required_files.iter().any(|f| f == BUNDLE_MARKER),
        "the bundle marker {BUNDLE_MARKER} must be a required file"
    );
    assert!(contents
        .required_files
        .iter()
        .any(|f| f == "whisper/whisper-cli.exe"));
    assert!(contents.required_dirs.iter().any(|d| d == "whisper"));
    assert!(contents
        .model_pack_files
        .iter()
        .any(|f| f.ends_with(MANIFEST_FILE)));

    // Build the described layout out of the real files, not placeholders, so
    // the check fails if the manifest names something this repository does not
    // actually have.
    let dir = tempfile::tempdir().unwrap();
    for relative in &contents.required_dirs {
        std::fs::create_dir_all(dir.path().join(relative)).unwrap();
    }
    for relative in &contents.required_files {
        let target = dir.path().join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        match source_of(relative) {
            Some(source) => {
                std::fs::copy(&source, &target)
                    .unwrap_or_else(|error| panic!("{}: {error}", source.display()));
            }
            // Stand-ins for the executables, which only exist after a build.
            None => std::fs::write(&target, vec![0u8; 200_000]).unwrap(),
        }
    }

    let layout = Layout::from_dir(dir.path());
    // The manifest describes a Windows bundle. On another host the same layout
    // is checked with that host's executable name, so the test exercises the
    // discovery rule rather than the file extension.
    std::fs::write(layout.bundled_backend(), vec![0u8; 200_000]).unwrap();
    assert!(layout.is_packaged_bundle(), "the marker must be recognised");
    let backend = backend::discover_in(Some(&layout), None, false, |_| None)
        .expect("the described bundle must resolve its own backend");
    assert_eq!(backend.origin, BackendOrigin::Bundled);

    // A bundle with the engine but no model pack must say exactly that.
    let status = model::inspect(
        Some(&layout),
        None,
        Requirements::for_layout(Some(&layout), false),
    );
    assert_eq!(status, ModelStatus::ManifestMissing);
    assert!(
        status.message().contains("model pack"),
        "{}",
        status.message()
    );
}

#[test]
fn every_external_component_is_pinned_to_something_immutable() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows/model-pack.json");
    let text = std::fs::read_to_string(&path).expect("model-pack.json must exist");
    let pin: serde_json::Value =
        serde_json::from_str(&text).expect("model-pack.json must be valid");
    assert_eq!(pin["schema_version"].as_u64(), Some(2));

    let whisper = &pin["whisper_cpp"];
    assert_eq!(whisper["license"].as_str(), Some("MIT"));
    let tag = whisper["tag"].as_str().expect("a tag is required");
    for floating in ["latest", "main", "master", "HEAD"] {
        assert_ne!(
            tag, floating,
            "the engine must not follow a moving reference"
        );
    }
    // A tag can be moved; the commit cannot. It must be pinned, not left open.
    let commit = whisper["commit"]
        .as_str()
        .expect("the engine commit must be pinned, not null");
    assert_eq!(commit.len(), 40, "commit must be a full git object id");
    assert!(commit.chars().all(|c| c.is_ascii_hexdigit()), "{commit}");

    let models = pin["models"].as_array().expect("models must be a list");
    assert!(!models.is_empty());
    assert_eq!(
        models
            .iter()
            .filter(|m| m["recommended"].as_bool() == Some(true))
            .count(),
        1,
        "exactly one model is recommended"
    );
    for model in models {
        let id = model["id"].as_str().unwrap_or("?");
        for field in [
            "id",
            "file",
            "source_url",
            "license",
            "license_url",
            "rationale",
        ] {
            assert!(
                model[field].as_str().is_some_and(|v| !v.is_empty()),
                "model {id} is missing {field}"
            );
        }
        // Every shippable model carries a real digest and a real size. There is
        // no "unpinned but publishable" state any more.
        let sha = model["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("model {id} has no pinned sha256"));
        assert_eq!(sha.len(), 64, "model {id} digest length");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "model {id} digest"
        );
        assert!(
            model["size_bytes"].as_u64().is_some_and(|n| n > 1_000_000),
            "model {id} needs a real byte size"
        );
        // A branch URL would let the bytes change under a fixed digest.
        let url = model["source_url"].as_str().unwrap();
        assert!(
            !url.contains("/resolve/main/") && !url.contains("/resolve/master/"),
            "model {id} points at a branch instead of an immutable revision: {url}"
        );
        let revision = model["source_revision"]
            .as_str()
            .unwrap_or_else(|| panic!("model {id} has no source_revision"));
        assert!(
            url.contains(revision),
            "model {id} url must contain its pinned revision"
        );
    }
}

#[test]
fn an_official_bundle_refuses_a_model_pack_that_was_never_verified() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let manifest = bundle.layout.whisper_dir().join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace(
            "\"verified_against_repository_pin\":true",
            "\"verified_against_repository_pin\":false",
        ),
    )
    .unwrap();

    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let audio = wav(dir.path());
    let error = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect_err("an unverified pack must be refused inside a shipped bundle");
    let message = error.to_string();
    assert!(matches!(error, Error::ModelUnusable { .. }), "{message}");
    assert!(
        message.contains("verified_against_repository_pin"),
        "the reason must name the field: {message}"
    );
}

#[test]
fn an_explicit_model_is_never_replaced_by_the_bundled_one() {
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let backend = backend::discover_in(Some(&bundle.layout), None, false, |_| None).unwrap();
    // The user asked for a model that is not there. The bundle has a perfectly
    // good one, and it must still not be used instead.
    let mut engine = WhisperCliEngine::from_backend(backend, Some("модель которой нет.bin"))
        .with_layout(Some(bundle.layout.clone()));
    let dir = tempfile::tempdir().unwrap();
    let audio = wav(dir.path());
    let error = engine
        .transcribe(&audio, &TranscribeOptions::default())
        .expect_err("a configured model that is absent is an error");
    let message = error.to_string();
    assert!(message.contains("модель которой нет.bin"), "{message}");
}

// ------------------------------------------------------- audio without ffmpeg

/// Writes a WAV with the given shape, so the preparation path can be exercised
/// on files that are not already what the recogniser wants.
fn wav_with(dir: &Path, name: &str, hz: u32, channels: u16) -> PathBuf {
    let path = dir.join(name);
    let spec = hound::WavSpec {
        channels,
        sample_rate: hz,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for index in 0..(hz as usize) {
        let value = ((index % 100) as i16) * 100;
        for _ in 0..channels {
            writer.write_sample(value).unwrap();
        }
    }
    writer.finalize().unwrap();
    path
}

#[test]
fn a_ready_made_16k_mono_wav_is_used_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let path = wav_with(dir.path(), "запись 1.wav", 16_000, 1);
    let prepared = sciwhisper_asr::capture::ensure_wav_16k(&path).unwrap();
    assert_eq!(prepared.path(), path);
    // The caller's own file must not be adopted, or dropping the value would
    // delete something the user gave us.
    assert!(!prepared.owns_temp_dir());
    drop(prepared);
    assert!(path.is_file(), "the source file was deleted");
}

#[test]
fn a_stereo_44k_wav_is_converted_without_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let path = wav_with(dir.path(), "стерео запись.wav", 44_100, 2);
    let prepared = sciwhisper_asr::capture::ensure_wav_16k(&path).unwrap();
    assert_ne!(prepared.path(), path, "the file needed converting");
    assert!(
        prepared.owns_temp_dir(),
        "the conversion is ours to clean up"
    );

    let reader = hound::WavReader::open(prepared.path()).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);

    let converted = prepared.path().to_path_buf();
    drop(prepared);
    assert!(!converted.exists(), "the conversion outlived its guard");
}

#[test]
fn a_whole_transcription_works_on_a_44k_wav_with_no_ffmpeg_involved() {
    // The end-to-end proof: an awkward source file, a bundled backend, and no
    // external tool anywhere in the path.
    let bundle = Bundle::new(WRITES_TRANSCRIPT, true);
    let mut engine = bundle.engine();
    let dir = tempfile::tempdir().unwrap();
    let source = wav_with(dir.path(), "Мои записи 44k.wav", 44_100, 2);

    let prepared = sciwhisper_asr::capture::ensure_wav_16k(&source).unwrap();
    let transcript = engine
        .transcribe(prepared.path(), &TranscribeOptions::default())
        .expect("a converted file must reach the backend");
    assert_eq!(transcript.text, "гидроксид меди два");
}

#[test]
fn a_non_wav_file_without_ffmpeg_says_so_instead_of_failing_obscurely() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("лекция.m4a");
    std::fs::write(&path, b"not really audio").unwrap();
    // ffmpeg is passed in explicitly so the test does not depend on what
    // happens to be installed on the machine running it.
    let error = sciwhisper_asr::capture::prepare_audio_for_test(&path, None)
        .expect_err("an undecodable file must be refused");
    let message = error.to_string();
    assert!(message.contains("ffmpeg"), "{message}");
    assert!(message.contains("лекция.m4a"), "{message}");
    assert!(
        message.contains("микрофона"),
        "the message should point at the path that does work: {message}"
    );
}

#[test]
fn a_missing_audio_file_is_named_without_leaking_a_home_directory() {
    let error = sciwhisper_asr::capture::prepare_audio_for_test(
        Path::new("/Users/someone/private/нет такого.wav"),
        None,
    )
    .expect_err("a missing file is an error");
    let message = error.to_string();
    assert!(message.contains("нет такого.wav"), "{message}");
    assert!(!message.contains("/Users/someone"), "{message}");
}

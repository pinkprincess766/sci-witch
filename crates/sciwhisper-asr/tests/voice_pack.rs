//! End-to-end checks for the speech-recognition layer, using a fake backend and a fake bundle layout.
//!
//! Nothing here needs a microphone, a real Whisper model or a network. The
//! backend is a tiny test executable that behaves the way `whisper-cli` does, so the
//! rules around it — discovery order, deadlines, output ceilings and the
//! deletion of temporary audio — can be exercised on any machine.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sciwhisper_asr::backend::{self, backend_file_name, BackendOrigin, Layout};
use sciwhisper_asr::engine::{AsrEngine, EngineKind, TranscribeOptions};
use sciwhisper_asr::error::Error;
use sciwhisper_asr::model::{self, ModelStatus, Requirements, MANIFEST_FILE};
use sciwhisper_asr::process::{self, Limits};
use sciwhisper_asr::whisper_cli::WhisperCliEngine;

// ------------------------------------------------------------ fake backend

const MODE_FILE: &str = "SCIWHISPER_TEST_BACKEND_MODE";

/// Installs the Cargo-built stand-in for `whisper-cli` and selects its mode
/// through a sidecar beside the per-test model, so parallel cases never race
/// through environment variables.
fn fake_whisper(dir: &Path, mode: &str) -> PathBuf {
    let path = dir.join(backend_file_name());
    let built = Path::new(env!("CARGO_BIN_EXE_sciwhisper-test-backend"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(built, &path).unwrap();
    }
    #[cfg(windows)]
    {
        // Creating symlinks normally needs an elevated Windows process. A
        // hard link needs no such permission and keeps the built executable
        // closed; copying is the fallback for unusual filesystems.
        if std::fs::hard_link(built, &path).is_err() {
            std::fs::copy(built, &path).unwrap();
        }
    }
    std::fs::write(dir.join(MODE_FILE), mode).unwrap();
    path
}

const WRITES_TRANSCRIPT: &str = "write";
const FAILS: &str = "fail";
const HANGS: &str = "hang";
const SUCCEEDS_SILENTLY: &str = "silent";

// ------------------------------------------------------------ bundle setup

struct Bundle {
    _dir: tempfile::TempDir,
    layout: Layout,
}

impl Bundle {
    /// A directory with `whisper/` next to where the executable would be.
    fn new(body: &str, with_model: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let whisper = dir.path().join("whisper");
        std::fs::create_dir_all(&whisper).unwrap();
        fake_whisper(&whisper, body);
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
    let requirements = Requirements::new(false);
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
    let mut command = Command::new(&script);
    command.arg("-m").arg(dir.path().join("fake-model.bin"));
    let error = process::run(
        command,
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
    let before = sciwhisper_recording_temp_dirs();
    std::thread::sleep(Duration::from_millis(120));

    // This is the Esc path: the session is abandoned, not finished.
    session.cancel();

    let after = sciwhisper_recording_temp_dirs();
    let new_dirs: Vec<&PathBuf> = after.iter().filter(|path| !before.contains(path)).collect();
    assert!(
        new_dirs.is_empty(),
        "a cancelled session created {new_dirs:?}; audio is only written by finish()"
    );
}

/// Temporary directories this application creates, for the cancel test — which
/// cannot know a path in advance, because a cancelled session must never make
/// one.
fn sciwhisper_recording_temp_dirs() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sciwhisper-recording-"))
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

// ---------------------------------------------------------- model pack

#[test]
fn a_pinned_digest_is_enforced_only_when_the_caller_asks_for_it() {
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
    let dir = bundle.layout.whisper_dir();

    // Asked for: an unverified pack is refused and the reason names the field.
    let strict = Requirements {
        verify_checksum: false,
        require_pinned: true,
    };
    match model::check(&dir, strict) {
        ModelStatus::NotPinned => {}
        other => panic!("{other:?}"),
    }
    // Not asked for (the default of a recording run): the same pack is used.
    assert!(model::check(&dir, Requirements::new(false)).is_ready());
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

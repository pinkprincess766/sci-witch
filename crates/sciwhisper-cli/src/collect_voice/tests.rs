use super::store::*;
use super::{Action, Args, ConsentChoice, Partition};
use clap::Parser;
use sciwhisper_eval::schema::{Dataset, Split};
use sha2::Digest;
use std::fs;

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    args: Args,
}

fn plan_with_consent(consent: ConsentKind) -> Plan {
    let source = include_bytes!("../../../../research/data/dev-seed-v2.jsonl");
    let mut p = plan(
        source,
        Split::Train,
        "spk01".into(),
        "2026-09-12".into(),
        "quiet_room".into(),
        consent,
    )
    .unwrap();
    p.tasks.truncate(2);
    p
}

fn plan_for_test() -> Plan {
    plan_with_consent(ConsentKind::RuV2)
}

/// SHA-256 of the consent document as it is in the repo, read at test time.
fn repo_consent_v2_sha256() -> (Vec<u8>, String) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../research/data/consent-ru-v2.md"
    );
    let bytes = fs::read(path).unwrap();
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    (bytes, hash)
}

fn wav(dir: &std::path::Path) -> std::path::PathBuf {
    wav_with(dir, "test.wav", 16000, 1, 16)
}

fn wav_with(
    dir: &std::path::Path,
    name: &str,
    sample_rate: u32,
    channels: u16,
    bits: u16,
) -> std::path::PathBuf {
    let path = dir.join(name);
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: bits,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for _ in 0..sample_rate * u32::from(channels) {
        writer.write_sample(0i32).unwrap();
    }
    writer.finalize().unwrap();
    path
}

#[test]
fn only_16_bit_mono_16_khz_takes_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    let text = &p.tasks[0].human_transcript;
    for (name, rate, channels, bits, found) in [
        ("rate.wav", 44_100, 1, 16, "44100 Гц"),
        ("stereo.wav", 16_000, 2, 16, "каналов: 2"),
        ("depth.wav", 16_000, 1, 24, "24 бит"),
    ] {
        let path = wav_with(dir.path(), name, rate, channels, bits);
        let error = s.accept(0, &path, text, "Mic").expect_err("refused");
        assert!(error.contains(found), "{error}");
    }
    assert!(s.takes().unwrap().is_empty());
    s.accept(0, &wav(dir.path()), text, "Mic").unwrap();
    assert_eq!(s.takes().unwrap().len(), 1);
}

#[test]
fn a_take_replaced_by_another_format_is_refused_on_resume_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    s.accept(0, &wav(dir.path()), &p.tasks[0].human_transcript, "Mic")
        .unwrap();
    let stereo = wav_with(dir.path(), "stereo.wav", 16000, 2, 16);
    fs::copy(stereo, s.root.join("takes/0000/audio.wav")).unwrap();
    let error = s.takes().err().expect("resume refused");
    assert!(error.contains("каналов: 2"), "{error}");
    let out = dir.path().join("export");
    assert!(s.export(&out).is_err());
    assert!(!out.exists());
}

#[test]
fn session_resumes_and_exports_the_exact_author_target_and_real_audio() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    let p = plan_for_test();
    let target = p.tasks[0].target_ast.clone();
    let session = Session::create(&path, p.clone()).unwrap();
    session
        .accept(
            0,
            &wav(dir.path()),
            &p.tasks[0].human_transcript,
            "Test microphone",
        )
        .unwrap();
    drop(session);
    let session = Session::open(&path).unwrap();
    assert_eq!(session.takes().unwrap().len(), 1);
    let out = dir.path().join("export");
    assert_eq!(session.export(&out).unwrap(), 1);
    let dataset =
        Dataset::parse_jsonl(&fs::read_to_string(out.join("dataset.jsonl")).unwrap()).unwrap();
    let record = &dataset.records[0];
    assert_eq!(record.target_ast, target);
    assert_eq!(record.family_id, p.tasks[0].family_id);
    assert_eq!(record.speaker_id.as_deref(), Some("spk01"));
    assert_eq!(record.split, Split::Train);
    assert!(record.asr_hypotheses.is_empty());
    assert_eq!(record.audio.as_ref().unwrap().duration_secs, 1.0);
    assert!(record.audio.as_ref().unwrap().snr_db.is_none());
    assert!(out.join(&record.audio.as_ref().unwrap().file).is_file());
    let pack: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("pack.json")).unwrap()).unwrap();
    for (name, hash) in pack["files"].as_object().unwrap() {
        assert_eq!(
            hash.as_str().unwrap(),
            digest(&fs::read(out.join(name)).unwrap())
        );
    }
    assert!(!fs::read_to_string(out.join("pack.json"))
        .unwrap()
        .contains(&dir.path().display().to_string()));
    assert!(session.export(&out).is_err());
    // Run the actual ingestion path against the exported relative paths.
    let measured = out.join("measured.jsonl");
    crate::ingest::run(
        crate::ingest::IngestOptions {
            manifest: out.join("dataset.jsonl"),
            output: measured.clone(),
            describe_only: true,
        },
        &mut |_| panic!("collection must not invoke ASR"),
    )
    .unwrap();
    let measured = Dataset::parse_jsonl(&fs::read_to_string(measured).unwrap()).unwrap();
    assert_eq!(measured.records[0].target_ast, target);
    assert!(measured.records[0].asr_hypotheses.is_empty());
}

#[test]
fn declined_or_invalid_consent_never_creates_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    let mut p = plan_for_test();
    p.consent.granted = false;
    assert!(Session::create(&path, p).is_err());
    assert!(!path.exists());
    let mut p = plan_for_test();
    p.consent.date = "2026-02-30".into();
    assert!(Session::create(&path, p).is_err());
    assert!(!path.exists());
    assert!(valid_date("2024-02-29"));
    assert!(!valid_date("2025-02-29"));
    assert!(!valid_date("٢٠٢٦-01-01"));
}

#[test]
fn mistaken_transcript_and_duplicate_confirmation_do_not_replace_gold_or_audio() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    let wav = wav(dir.path());
    assert!(s.accept(0, &wav, "другая фраза", "Mic").is_err());
    assert!(s.takes().unwrap().is_empty());
    s.accept(0, &wav, &p.tasks[0].human_transcript, "Mic")
        .unwrap();
    let before = fs::read(s.root.join("takes/0000/take.json")).unwrap();
    assert!(s
        .accept(0, &wav, &p.tasks[0].human_transcript, "Mic2")
        .is_err());
    assert_eq!(
        before,
        fs::read(s.root.join("takes/0000/take.json")).unwrap()
    );
}

#[test]
fn corrupted_audio_is_refused_on_resume_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    s.accept(0, &wav(dir.path()), &p.tasks[0].human_transcript, "Mic")
        .unwrap();
    fs::write(s.root.join("takes/0000/audio.wav"), b"broken").unwrap();
    assert!(s.takes().is_err());
    let out = dir.path().join("export");
    assert!(s.export(&out).is_err());
    assert!(!out.exists());
}

#[test]
fn one_writer_and_released_lock_after_exit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    let s = Session::create(&path, plan_for_test()).unwrap();
    assert!(Session::open(&path).is_err());
    drop(s);
    assert!(Session::open(&path).is_ok());
}

#[test]
fn uncommitted_takes_are_never_exported_and_revocation_removes_them() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    let pending = s.root.join("takes/.pending-crash");
    fs::create_dir(&pending).unwrap();
    fs::copy(wav(dir.path()), pending.join("audio.wav")).unwrap();
    assert!(s.takes().unwrap().is_empty());
    s.accept(
        0,
        &dir.path().join("test.wav"),
        &p.tasks[0].human_transcript,
        "Mic",
    )
    .unwrap();
    s.revoke().unwrap();
    assert!(s.root.join("REVOKED").exists());
    assert_eq!(fs::read_dir(s.root.join("takes")).unwrap().count(), 0);
    assert!(s
        .accept(
            1,
            &dir.path().join("test.wav"),
            &p.tasks[1].human_transcript,
            "Mic"
        )
        .is_err());
    assert!(s.export(&dir.path().join("export")).is_err());
    s.revoke().unwrap(); // interruption/retry is idempotent
}

#[test]
fn task_families_and_speakers_cannot_change_split_within_a_plan() {
    let mut p = plan_for_test();
    p.tasks[1].split = Split::Validation;
    let dir = tempfile::tempdir().unwrap();
    assert!(Session::create(&dir.path().join("session"), p).is_err());
    let mut p = plan_for_test();
    p.speaker_id = "../somewhere".into();
    assert!(Session::create(&dir.path().join("session"), p).is_err());
}

#[test]
fn a_changed_consent_statement_blocks_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    drop(Session::create(&path, plan_for_test()).unwrap());
    fs::write(path.join("consent.txt"), "something else").unwrap();
    assert!(Session::open(&path).is_err());
}

#[test]
fn an_unreadable_take_leaves_no_committed_or_temporary_session_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    let bad = dir.path().join("bad.wav");
    fs::write(&bad, b"not a WAV").unwrap();
    assert!(s
        .accept(0, &bad, &p.tasks[0].human_transcript, "Mic")
        .is_err());
    assert!(s
        .accept(
            0,
            &dir.path().join("missing.wav"),
            &p.tasks[0].human_transcript,
            "Mic"
        )
        .is_err());
    assert_eq!(fs::read_dir(s.root.join("takes")).unwrap().count(), 0);
}

#[test]
fn repeated_sessions_have_distinct_record_ids_but_the_same_speaker_and_family() {
    let dir = tempfile::tempdir().unwrap();
    let wave = wav(dir.path());
    let mut records = Vec::new();
    for name in ["first", "second"] {
        let p = plan_for_test();
        let s = Session::create(&dir.path().join(name), p.clone()).unwrap();
        s.accept(0, &wave, &p.tasks[0].human_transcript, "Mic")
            .unwrap();
        records.push(s.takes().unwrap().remove(&0).unwrap().record);
    }
    assert_ne!(records[0].id, records[1].id);
    assert_eq!(records[0].speaker_id, records[1].speaker_id);
    assert_eq!(records[0].family_id, records[1].family_id);
}

#[test]
fn a_tampered_target_or_audio_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    s.accept(0, &wav(dir.path()), &p.tasks[0].human_transcript, "Mic")
        .unwrap();
    let path = s.root.join("takes/0000/take.json");
    let bytes = fs::read(&path).unwrap();
    let mut take: Take = serde_json::from_slice(&bytes).unwrap();
    take.record.human_transcript = "совсем другие слова".into();
    fs::write(&path, serde_json::to_vec(&take).unwrap()).unwrap();
    assert!(s.takes().is_err());
    let mut take: Take = serde_json::from_slice(&bytes).unwrap();
    take.record.audio.as_mut().unwrap().file = "../outside.wav".into();
    fs::write(&path, serde_json::to_vec(&take).unwrap()).unwrap();
    assert!(s.takes().is_err());
}

#[cfg(unix)]
#[test]
fn symlink_audio_never_reads_or_deletes_an_external_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = plan_for_test();
    let s = Session::create(&dir.path().join("session"), p.clone()).unwrap();
    let original = wav(dir.path());
    s.accept(0, &original, &p.tasks[0].human_transcript, "Mic")
        .unwrap();
    let audio = s.root.join("takes/0000/audio.wav");
    fs::remove_file(&audio).unwrap();
    std::os::unix::fs::symlink(&original, audio).unwrap();
    assert!(s.takes().is_err());
    assert!(s.revoke().is_err());
    assert!(original.exists());
}

#[test]
fn speaker_codes_are_spk_and_two_digits_from_01_to_99() {
    let source = include_bytes!("../../../../research/data/dev-seed-v2.jsonl");
    let accepted = |id: &str| {
        plan(
            source,
            Split::Train,
            id.into(),
            "2026-09-12".into(),
            "quiet_room".into(),
            ConsentKind::RuV2,
        )
        .is_ok()
    };
    for id in ["spk01", "spk05", "spk99"] {
        assert!(accepted(id), "{id} must be accepted");
    }
    for id in [
        "spk00",
        "spk1",
        "spk001",
        "spk-a7b9",
        "SPK01",
        "spk0a",
        "spk-test123",
    ] {
        assert!(!accepted(id), "{id} must be refused");
    }
}

fn start_with_split(split: &str) -> std::result::Result<Action, clap::Error> {
    Cli::try_parse_from([
        "sciwhisper",
        "start",
        "--session",
        "my-session",
        "--speaker",
        "spk01",
        "--split",
        split,
        "--consent",
        "consent-ru-v2",
        "--date",
        "2026-09-12",
        "--environment",
        "quiet_room",
    ])
    .map(|cli| cli.args.action)
}

#[test]
fn cli_split_is_dev_holdout_with_underscore_and_no_dashed_alias() {
    let action = start_with_split("dev_holdout").expect("dev_holdout is the CLI value");
    assert!(matches!(
        action,
        Action::Start {
            split: Partition::DevHoldout,
            ..
        }
    ));
    let error = start_with_split("dev-holdout")
        .err()
        .expect("dev-holdout must be refused");
    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn a_v2_session_stores_the_id_and_the_sha256_of_the_repo_consent_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    let (document, expected_sha256) = repo_consent_v2_sha256();
    let session = Session::create(&path, plan_with_consent(ConsentKind::RuV2)).unwrap();
    assert_eq!(session.plan.consent.statement_id, "consent-ru-v2");
    assert_eq!(session.plan.consent_sha256, expected_sha256);
    assert_eq!(fs::read(path.join("consent.txt")).unwrap(), document);
    drop(session);
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("session.json")).unwrap()).unwrap();
    assert_eq!(stored["consent"]["statement_id"], "consent-ru-v2");
    assert_eq!(stored["consent_sha256"], expected_sha256.as_str());
    assert!(Session::open(&path).is_ok());
}

#[test]
fn a_local_session_stores_voice_local_v1_and_the_hash_of_its_own_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    let session = Session::create(&path, plan_with_consent(ConsentKind::LocalV1)).unwrap();
    assert_eq!(session.plan.consent.statement_id, "voice-local-v1");
    assert_eq!(
        session.plan.consent_sha256,
        format!("{:x}", sha2::Sha256::digest(CONSENT_LOCAL_TEXT.as_bytes()))
    );
    assert_ne!(session.plan.consent_sha256, repo_consent_v2_sha256().1);
    assert_eq!(
        fs::read(path.join("consent.txt")).unwrap(),
        CONSENT_LOCAL_TEXT.as_bytes()
    );
}

#[test]
fn a_session_whose_statement_and_text_disagree_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session");
    // v2 id with the hash of the short local text, and the other way round.
    let mut p = plan_with_consent(ConsentKind::LocalV1);
    p.consent.statement_id = "consent-ru-v2".into();
    assert!(Session::create(&path, p).is_err());
    let mut p = plan_with_consent(ConsentKind::RuV2);
    p.consent.statement_id = "voice-local-v1".into();
    assert!(Session::create(&path, p).is_err());
    // Any other id is not a statement this collector knows.
    for id in ["consent-ru-v1", "consent-ru-v3", ""] {
        let mut p = plan_with_consent(ConsentKind::RuV2);
        p.consent.statement_id = id.into();
        assert!(Session::create(&path, p).is_err(), "{id:?} must be refused");
    }
    assert!(!path.exists());
    // A v2 session whose consent.txt is no longer the repo document does not reopen.
    drop(Session::create(&path, plan_for_test()).unwrap());
    fs::write(path.join("consent.txt"), CONSENT_LOCAL_TEXT).unwrap();
    assert!(Session::open(&path).is_err());
}

#[test]
fn each_consent_accepts_only_its_own_confirmation_word() {
    for word in ["СОГЛАСНА", "СОГЛАСЕН"] {
        assert!(ConsentKind::LocalV1.accepts(word));
        assert!(
            !ConsentKind::RuV2.accepts(word),
            "{word} must not confirm v2"
        );
    }
    assert!(ConsentKind::RuV2.accepts("ПОДПИСАНО"));
    for word in ["ПОДПИСАНО", "", "подписано", "ДА", "ПОДПИСАНО "] {
        assert!(!ConsentKind::LocalV1.accepts(word), "{word:?}");
    }
    for word in ["", "подписано", "ДА", "ПОДПИСАНО "] {
        assert!(!ConsentKind::RuV2.accepts(word), "{word:?}");
    }
}

#[test]
fn the_terminal_says_which_document_and_that_local_is_local_only() {
    let v2 = ConsentKind::RuV2.notice();
    assert!(v2.contains("research/data/consent-ru-v2.md"), "{v2}");
    assert!(v2.contains(&repo_consent_v2_sha256().1), "{v2}");
    let local = ConsentKind::LocalV1.notice();
    assert!(
        local.contains("ТОЛЬКО ЛОКАЛЬНЫЙ") && local.contains("consent-ru-v2"),
        "{local}"
    );
}

#[test]
fn consent_ids_match_what_ingest_refuses() {
    assert_eq!(CONSENT_LOCAL_ID, crate::ingest::LOCAL_ONLY_STATEMENT_ID);
    assert_eq!(ConsentKind::LocalV1.id(), CONSENT_LOCAL_ID);
    assert_eq!(ConsentKind::RuV2.id(), CONSENT_V2_ID);
}

#[test]
fn the_export_names_the_consent_and_ingest_refuses_only_the_local_one() {
    let dir = tempfile::tempdir().unwrap();
    let wave = wav(dir.path());
    for (kind, id, scope, accepted) in [
        (ConsentKind::LocalV1, "voice-local-v1", "local_only", false),
        (
            ConsentKind::RuV2,
            "consent-ru-v2",
            "publication_after_legal_review",
            true,
        ),
    ] {
        let p = plan_with_consent(kind);
        let session = Session::create(&dir.path().join(format!("s-{id}")), p.clone()).unwrap();
        session
            .accept(0, &wave, &p.tasks[0].human_transcript, "Mic")
            .unwrap();
        let out = dir.path().join(format!("pack-{id}"));
        session.export(&out).unwrap();
        let pack: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("pack.json")).unwrap()).unwrap();
        assert_eq!(pack["scope"], scope);
        assert_eq!(pack["consent_statement_id"], id);
        assert_eq!(pack["consent_sha256"], p.consent_sha256.as_str());
        let measured = out.join("measured.jsonl");
        let result = crate::ingest::run(
            crate::ingest::IngestOptions {
                manifest: out.join("dataset.jsonl"),
                output: measured.clone(),
                describe_only: true,
            },
            &mut |_| panic!("collection must not invoke ASR"),
        );
        assert_eq!(result.is_ok(), accepted, "{id}: {result:?}");
        if let Err(error) = result {
            assert!(error.contains("consent-ru-v2"), "{error}");
            assert!(!measured.exists());
        }
    }
}

#[test]
fn cli_start_without_consent_is_refused_and_both_ids_parse() {
    let start = |extra: &[&str]| {
        let mut argv = vec![
            "sciwhisper",
            "start",
            "--session",
            "s",
            "--speaker",
            "spk01",
            "--split",
            "train",
            "--date",
            "2026-09-12",
            "--environment",
            "quiet_room",
        ];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv).map(|cli| cli.args.action)
    };
    let error = start(&[]).err().expect("--consent is required");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
    assert!(error.to_string().contains("--consent"), "{error}");
    let error = start(&["--consent", "consent-ru-v1"])
        .err()
        .expect("unknown id");
    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    assert!(matches!(
        start(&["--consent", "consent-ru-v2"]).unwrap(),
        Action::Start {
            consent: ConsentChoice::RuV2,
            ..
        }
    ));
    assert!(matches!(
        start(&["--consent", "voice-local-v1"]).unwrap(),
        Action::Start {
            consent: ConsentChoice::LocalV1,
            ..
        }
    ));
}

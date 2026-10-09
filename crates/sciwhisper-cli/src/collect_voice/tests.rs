use super::store::*;
use sciwhisper_eval::schema::{Dataset, Split};
use std::fs;

fn plan_for_test() -> Plan {
    let source = include_bytes!("../../../../research/data/dev-seed-v2.jsonl");
    let mut p = plan(
        source,
        Split::Train,
        "spk-test123".into(),
        "2026-09-12".into(),
        "quiet_room".into(),
    )
    .unwrap();
    p.tasks.truncate(2);
    p
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
    assert_eq!(record.speaker_id.as_deref(), Some("spk-test123"));
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

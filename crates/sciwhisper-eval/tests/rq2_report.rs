//! Plan item 7 (part b): `sciwhisper-eval rq2-report` on a committed toy file.
//!
//! The fixture is made-up data in the documented input format, not train.
//! The expected JSON is committed next to it; its AURC 0.198214… was checked
//! by hand (tie block of four at 0.95 with two errors). The test runs the
//! binary with the frozen defaults (seed 20261025, 10 000 resamples) and
//! compares the output byte for byte.

use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn toy_report_matches_committed_json() {
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("report.json");
    let status = Command::new(env!("CARGO_BIN_EXE_sciwhisper-eval"))
        .args(["rq2-report", "--max-stage", "1", "--input"])
        .arg(fixtures().join("rq2-toy.jsonl"))
        .arg("--output")
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let got = std::fs::read_to_string(&path).unwrap();
    let want = std::fs::read_to_string(fixtures().join("rq2-toy.report.json")).unwrap();
    assert_eq!(got.replace("\r\n", "\n"), want.replace("\r\n", "\n"));
    let json: serde_json::Value = serde_json::from_str(&got).unwrap();
    assert!((json["aurc"]["aurc"].as_f64().unwrap() - 1.585_714_285_714_285_7 / 8.0).abs() < 1e-12);
    assert_eq!(json["aurc"]["bootstrap_unit"], "phrase_family");
}

#[test]
fn refuses_dev_holdout_rows() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.jsonl");
    std::fs::write(
        &input,
        "{\"id\":\"x\",\"family_id\":\"f\",\"stage\":0,\"score\":1.0,\"correct\":true,\"split\":\"dev_holdout\"}\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sciwhisper-eval"))
        .args(["rq2-report", "--input"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("train only"));
}

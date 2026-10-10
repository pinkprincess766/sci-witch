//! The reports the compiler gate reads must not publish Brier or ECE over
//! the parse levels. The levels 1.0 / 0.95 / 0.7 / 0.0 are ranks, not
//! probabilities (AGENTS.md), so those two numbers mean nothing; the reports
//! up to deterministic-v6 carried them. AURC uses the levels only as a
//! ranking and is published instead.

use std::path::PathBuf;

/// The reports `AGENTS.md` passes to `sciwhisper-eval gate` that carry a
/// calibration block.
const GATE_REPORTS: [&str; 3] = [
    "deterministic-v7.json",
    "ambiguous-auto-v3.json",
    "nomenclature-v2.json",
];

fn calibration(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../research/results")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let report: serde_json::Value = serde_json::from_str(&text).expect("report is JSON");
    report["selective_prediction"]["calibration"].clone()
}

#[test]
fn gate_reports_publish_aurc_and_no_brier_or_ece_over_levels() {
    for name in GATE_REPORTS {
        let block = calibration(name);
        assert!(block.is_object(), "{name}: no calibration block");
        for key in ["brier", "expected_calibration_error"] {
            assert!(block.get(key).is_none(), "{name}: still publishes {key}");
        }
        assert!(block["aurc"].is_number(), "{name}: no aurc");
    }
}

#[test]
fn the_old_report_is_what_this_test_rejects() {
    // deterministic-v6 is kept unchanged for reproducibility; it is the
    // shape the test above exists to refuse.
    assert!(calibration("deterministic-v6.json").get("brier").is_some());
}

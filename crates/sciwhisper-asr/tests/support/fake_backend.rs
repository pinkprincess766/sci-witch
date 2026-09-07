//! Cross-platform stand-in for `whisper-cli`, used only by integration tests.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

const MODE_FILE: &str = "SCIWHISPER_TEST_BACKEND_MODE";

fn main() {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    let model = value_after(&arguments, "-m").expect("-m model path");
    let mode_path = PathBuf::from(model)
        .parent()
        .expect("model directory")
        .join(MODE_FILE);
    let mode = std::fs::read_to_string(&mode_path).expect("fake backend mode");

    match mode.trim() {
        "write" => write_transcript(&arguments),
        "fail" => {
            eprintln!("error: failed to load model");
            std::process::exit(4);
        }
        "hang" => std::thread::sleep(Duration::from_secs(60)),
        "silent" => {}
        other => {
            eprintln!("unknown fake backend mode: {other}");
            std::process::exit(2);
        }
    }
}

fn value_after(arguments: &[OsString], flag: &str) -> Option<OsString> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn write_transcript(arguments: &[OsString]) {
    let mut transcript = value_after(arguments, "-of").expect("-of output path");
    transcript.push(".txt");
    std::fs::write(PathBuf::from(transcript), "гидроксид меди два").expect("write fake transcript");
}

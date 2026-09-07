//! Cross-platform stand-in for `whisper-cli`, used only by integration tests.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

const MODE_FILE: &str = "SCIWHISPER_TEST_BACKEND_MODE";

fn main() {
    let executable = std::env::current_exe().expect("current executable path");
    let mode_path = executable
        .parent()
        .expect("backend directory")
        .join(MODE_FILE);
    let mode = std::fs::read_to_string(&mode_path).expect("fake backend mode");

    match mode.trim() {
        "write" => write_transcript(),
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

fn write_transcript() {
    let mut args = std::env::args_os();
    let mut output: Option<OsString> = None;
    while let Some(argument) = args.next() {
        if argument == "-of" {
            output = args.next();
            break;
        }
    }

    let mut transcript = output.expect("-of output path");
    transcript.push(".txt");
    std::fs::write(PathBuf::from(transcript), "гидроксид меди два").expect("write fake transcript");
}

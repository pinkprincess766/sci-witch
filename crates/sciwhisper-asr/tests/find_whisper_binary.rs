//! `find_whisper_binary` is the public alias of `backend::discover` for old
//! callers. This binary owns `SCIWHISPER_WHISPER` so the unit tests in
//! `whisper_cli.rs` never see it.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use sciwhisper_asr::backend::discover;
use sciwhisper_asr::whisper_cli::find_whisper_binary;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_whisper_env(path: &Path, body: impl FnOnce()) {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var_os("SCIWHISPER_WHISPER");
    std::env::set_var("SCIWHISPER_WHISPER", path);
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("SCIWHISPER_WHISPER", value),
                None => std::env::remove_var("SCIWHISPER_WHISPER"),
            }
        }
    }
    let _restore = Restore(previous);
    body();
}

#[test]
fn find_whisper_binary_matches_discover_for_a_real_file_and_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("whisper-cli");
    fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    }

    with_whisper_env(&binary, || {
        let found = find_whisper_binary().expect("configured file is a backend");
        let discovered = discover(Some(&binary)).expect("same file").binary;
        assert_eq!(found, discovered);
    });

    let missing = dir.path().join("missing-whisper-cli");
    with_whisper_env(&missing, || {
        assert!(find_whisper_binary().is_err());
        assert!(discover(Some(missing.as_path())).is_err());
    });
}

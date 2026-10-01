//! Which speech backend runs, and where it came from.
//!
//! The order is fixed and never silently skipped:
//!
//! 1. a path the user configured explicitly;
//! 2. the `whisper-cli` shipped inside the application bundle;
//! 3. a supported local `whisper.cpp` on `PATH`;
//! 4. Python `openai-whisper` — a developer convenience only.
//!
//! In a packaged bundle only the first two are allowed. A shipped copy that
//! has lost its backend is a packaging fault, and saying so is far better than
//! quietly reaching for a Python interpreter the user never installed.

use std::path::{Path, PathBuf};

use crate::engine::EngineKind;
use crate::error::{Error, Result};

/// Marker file that only the packaged Windows bundle carries.
pub const BUNDLE_MARKER: &str = "README-WINDOWS.txt";
/// Directory inside the bundle that holds the backend and the model.
pub const WHISPER_DIR: &str = "whisper";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendOrigin {
    /// `--whisper <path>` or `SCIWHISPER_WHISPER`.
    Configured,
    /// Shipped next to the application.
    Bundled,
    /// A `whisper.cpp` build found on `PATH`.
    ExternalWhisperCpp,
    /// Python `openai-whisper`. Never used from a packaged bundle.
    Developer,
}

impl BackendOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendOrigin::Configured => "configured",
            BackendOrigin::Bundled => "bundled",
            BackendOrigin::ExternalWhisperCpp => "external whisper.cpp",
            BackendOrigin::Developer => "developer (python)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Backend {
    pub binary: PathBuf,
    pub kind: EngineKind,
    pub origin: BackendOrigin,
}

/// Where the application's own files live.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Layout { root: dir.into() }
    }

    /// The directory holding the running executable, if it can be determined.
    pub fn detect() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        exe.parent().map(Layout::from_dir)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn whisper_dir(&self) -> PathBuf {
        self.root.join(WHISPER_DIR)
    }

    /// The backend the bundle is supposed to carry.
    pub fn bundled_backend(&self) -> PathBuf {
        self.whisper_dir().join(backend_file_name())
    }

    /// True when this looks like a shipped bundle rather than a build tree.
    pub fn is_packaged_bundle(&self) -> bool {
        self.root.join(BUNDLE_MARKER).is_file()
    }
}

/// `whisper-cli.exe` on Windows, `whisper-cli` elsewhere.
pub fn backend_file_name() -> &'static str {
    if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    }
}

/// Resolves the backend for the running application.
pub fn discover(explicit: Option<&Path>) -> Result<Backend> {
    let layout = Layout::detect();
    discover_in(
        layout.as_ref(),
        explicit,
        std::env::var_os("SCIWHISPER_NO_PYTHON").is_none(),
        search_path,
    )
}

/// The resolution itself, with the filesystem and `PATH` injected so that the
/// order can be tested without installing anything.
pub fn discover_in(
    layout: Option<&Layout>,
    explicit: Option<&Path>,
    allow_python: bool,
    on_path: impl Fn(&str) -> Option<PathBuf>,
) -> Result<Backend> {
    // 1. What the user asked for wins, and a bad path is an error rather than
    //    a reason to look elsewhere.
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(Error::BackendMissing {
                looked_for: display_name(path),
                detail: "указанный путь к движку не ведёт к файлу".into(),
            });
        }
        return Ok(Backend {
            kind: kind_of(path),
            binary: path.to_path_buf(),
            origin: BackendOrigin::Configured,
        });
    }

    // 2. The copy shipped with the application.
    if let Some(layout) = layout {
        let bundled = layout.bundled_backend();
        if bundled.is_file() {
            return Ok(Backend {
                binary: bundled,
                kind: EngineKind::WhisperCpp,
                origin: BackendOrigin::Bundled,
            });
        }
        // A shipped bundle that lost its backend is broken. Falling through to
        // Python here would turn a packaging fault into a mysterious runtime
        // dependency on the user's machine.
        if layout.is_packaged_bundle() {
            return Err(Error::BundleIncomplete {
                missing: format!("{WHISPER_DIR}/{}", backend_file_name()),
            });
        }
    }

    // 3. A local whisper.cpp.
    for name in ["whisper-cli", "whisper-cpp"] {
        if let Some(path) = on_path(name) {
            return Ok(Backend {
                binary: path,
                kind: EngineKind::WhisperCpp,
                origin: BackendOrigin::ExternalWhisperCpp,
            });
        }
    }

    // 4. Python, and only outside a bundle.
    if allow_python {
        if let Some(path) = on_path("whisper") {
            return Ok(Backend {
                binary: path,
                kind: EngineKind::OpenaiWhisper,
                origin: BackendOrigin::Developer,
            });
        }
    }

    Err(Error::BackendMissing {
        looked_for: format!("{WHISPER_DIR}/{}", backend_file_name()),
        detail: "в комплекте нет whisper-cli и в системе не найден whisper.cpp".into(),
    })
}

fn kind_of(path: &Path) -> EngineKind {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.contains("cli") || name.contains("cpp") {
        EngineKind::WhisperCpp
    } else {
        EngineKind::OpenaiWhisper
    }
}

/// The file name only. Reports must not carry somebody's home directory.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in executable_candidates(&dir, name) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![dir.join(format!("{name}.exe")), dir.join(name)]
    } else {
        vec![dir.join(name)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"binary").unwrap();
    }

    fn nothing_on_path(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn an_explicit_path_wins_over_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        touch(&layout.bundled_backend());
        let chosen = dir.path().join("my-whisper-cli");
        touch(&chosen);

        let backend = discover_in(Some(&layout), Some(&chosen), true, nothing_on_path).unwrap();
        assert_eq!(backend.origin, BackendOrigin::Configured);
        assert_eq!(backend.binary, chosen);
        assert_eq!(backend.kind, EngineKind::WhisperCpp);
    }

    #[test]
    fn a_configured_path_that_does_not_exist_is_an_error_not_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        touch(&layout.bundled_backend());
        let missing = dir.path().join("not-here.exe");

        let error = discover_in(Some(&layout), Some(&missing), true, nothing_on_path)
            .expect_err("a configured path must not silently fall back");
        assert!(matches!(error, Error::BackendMissing { .. }), "{error}");
    }

    #[test]
    fn the_bundled_backend_is_preferred_over_path() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        touch(&layout.bundled_backend());

        let backend = discover_in(Some(&layout), None, true, |name| {
            (name == "whisper-cli").then(|| PathBuf::from("/usr/bin/whisper-cli"))
        })
        .unwrap();
        assert_eq!(backend.origin, BackendOrigin::Bundled);
        assert_eq!(backend.binary, layout.bundled_backend());
    }

    #[test]
    fn a_packaged_bundle_without_its_backend_is_a_packaging_fault() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join(BUNDLE_MARKER));
        let layout = Layout::from_dir(dir.path());

        // Python is on PATH and must still not be chosen.
        let error = discover_in(Some(&layout), None, true, |name| {
            (name == "whisper").then(|| PathBuf::from("/usr/bin/whisper"))
        })
        .expect_err("a shipped bundle must not fall back to Python");
        match error {
            Error::BundleIncomplete { missing } => assert!(missing.contains("whisper-cli")),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn outside_a_bundle_a_local_whisper_cpp_is_used_before_python() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        let backend = discover_in(Some(&layout), None, true, |name| match name {
            "whisper-cli" => Some(PathBuf::from("/opt/whisper-cli")),
            "whisper" => Some(PathBuf::from("/usr/bin/whisper")),
            _ => None,
        })
        .unwrap();
        assert_eq!(backend.origin, BackendOrigin::ExternalWhisperCpp);
        assert_eq!(backend.kind, EngineKind::WhisperCpp);
    }

    #[test]
    fn python_is_last_and_can_be_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        let python = |name: &str| (name == "whisper").then(|| PathBuf::from("/usr/bin/whisper"));

        let backend = discover_in(Some(&layout), None, true, python).unwrap();
        assert_eq!(backend.origin, BackendOrigin::Developer);
        assert_eq!(backend.kind, EngineKind::OpenaiWhisper);

        let error =
            discover_in(Some(&layout), None, false, python).expect_err("python must be refusable");
        assert!(matches!(error, Error::BackendMissing { .. }), "{error}");
    }

    #[test]
    fn nothing_anywhere_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        let error = discover_in(Some(&layout), None, true, nothing_on_path).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("whisper-cli"), "{message}");
    }

    #[test]
    fn reports_never_carry_a_home_directory() {
        let path = PathBuf::from("/Users/someone/secret/whisper-cli.exe");
        assert_eq!(display_name(&path), "whisper-cli.exe");
    }

    #[test]
    fn a_path_with_spaces_and_cyrillic_is_handled_as_one_argument() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("Мои документы и файлы");
        let chosen = odd.join(backend_file_name());
        touch(&chosen);
        let backend = discover_in(None, Some(&chosen), true, nothing_on_path).unwrap();
        assert_eq!(backend.binary, chosen);
        assert!(backend.binary.to_string_lossy().contains("Мои документы"));
    }
}

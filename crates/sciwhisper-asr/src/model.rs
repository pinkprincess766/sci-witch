//! The recognition model and the manifest that describes it.
//!
//! Nothing here downloads anything. A model is either present and matches its
//! manifest, or the user is told plainly what is wrong. There is no path from
//! a missing model to the network and no path to a Python fallback.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backend::{display_name, Layout, WHISPER_DIR};
use crate::error::{Error, Result};

pub const MANIFEST_FILE: &str = "MODEL_MANIFEST.json";
pub const MANIFEST_SCHEMA: u32 = 1;

/// Everything needed to say which weights these are and where they came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub schema_version: u32,
    /// Short identifier, e.g. `small-q5_1`.
    pub model_id: String,
    /// File name inside the same directory as the manifest.
    pub file: String,
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 of `file`.
    pub sha256: String,
    pub source_url: String,
    pub license: String,
    pub license_url: String,
    /// The `whisper.cpp` release these weights were published for.
    pub whisper_cpp_version: String,
    /// Whether the release pipeline checked the download against the digest
    /// pinned in the repository, rather than only recording what it got.
    #[serde(default)]
    pub verified_against_repository_pin: bool,
    #[serde(default)]
    pub notes: String,
}

impl ModelManifest {
    /// Describes a file the user named explicitly. Such a file has no manifest
    /// of its own, so nothing is claimed about where it came from.
    pub fn for_explicit_file(path: &Path) -> Self {
        ModelManifest {
            schema_version: MANIFEST_SCHEMA,
            model_id: format!("указан вручную: {}", display_name(path)),
            file: display_name(path),
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            sha256: String::new(),
            source_url: String::new(),
            license: String::new(),
            license_url: String::new(),
            whisper_cpp_version: String::new(),
            verified_against_repository_pin: false,
            notes: "модель указана пользователем; происхождение не проверялось".into(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let manifest: ModelManifest =
            serde_json::from_str(text).map_err(|error| Error::ModelManifestInvalid {
                reason: error.to_string(),
            })?;
        if manifest.schema_version != MANIFEST_SCHEMA {
            return Err(Error::ModelManifestInvalid {
                reason: format!(
                    "manifest schema {} is not the supported {MANIFEST_SCHEMA}",
                    manifest.schema_version
                ),
            });
        }
        if manifest.file.contains('/')
            || manifest.file.contains('\\')
            || manifest.file.contains("..")
        {
            // The manifest may only name a file beside itself. A path would let
            // a downloaded pack point the backend at anything on the disk.
            return Err(Error::ModelManifestInvalid {
                reason: format!("model file '{}' must be a bare file name", manifest.file),
            });
        }
        if manifest.sha256.len() != 64 || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::ModelManifestInvalid {
                reason: "sha256 must be 64 hex characters".into(),
            });
        }
        Ok(manifest)
    }
}

/// What the model directory actually contains.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelStatus {
    Ready {
        path: PathBuf,
        manifest: Box<ModelManifest>,
        /// `false` when only the size was checked; hashing a gigabyte on every
        /// start would cost more than it is worth.
        checksum_verified: bool,
    },
    /// No directory to look in at all.
    NoSearchLocation,
    ManifestMissing,
    ManifestInvalid {
        reason: String,
    },
    FileMissing {
        expected: String,
    },
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    ChecksumMismatch {
        expected: String,
        actual: String,
    },
    /// The pack says its own digest was never checked against the pin in the
    /// project's repository. An official bundle refuses such a pack.
    NotPinned,
}

impl ModelStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, ModelStatus::Ready { .. })
    }

    /// A sentence a non-programmer can act on.
    pub fn message(&self) -> String {
        match self {
            ModelStatus::Ready {
                manifest,
                checksum_verified,
                ..
            } => format!(
                "модель {} на месте{}",
                manifest.model_id,
                if *checksum_verified {
                    ", контрольная сумма совпала"
                } else {
                    " (проверен только размер)"
                }
            ),
            ModelStatus::NoSearchLocation => {
                "не удалось определить папку программы, поэтому модель искать негде".into()
            }
            ModelStatus::ManifestMissing => format!(
                "рядом с программой нет папки {WHISPER_DIR} с файлом {MANIFEST_FILE}. \
                 Распакуйте model pack в папку {WHISPER_DIR} рядом с sci-witch."
            ),
            ModelStatus::ManifestInvalid { reason } => {
                format!("файл {MANIFEST_FILE} повреждён или имеет неизвестный формат: {reason}")
            }
            ModelStatus::FileMissing { expected } => format!(
                "файл модели {expected} не найден в папке {WHISPER_DIR}. \
                 Скачайте официальный model pack и распакуйте его туда целиком."
            ),
            ModelStatus::SizeMismatch { expected, actual } => format!(
                "файл модели повреждён: ожидалось {expected} байт, на диске {actual}. \
                 Скачайте model pack заново."
            ),
            ModelStatus::ChecksumMismatch { .. } => {
                "файл модели повреждён: контрольная сумма не совпала. \
                 Скачайте model pack заново."
                    .into()
            }
            ModelStatus::NotPinned => format!(
                "этот model pack собран без сверки с закреплённой в проекте контрольной суммой \
                 ({MANIFEST_FILE}: verified_against_repository_pin = false). \
                 Официальный комплект такой пакет не принимает — скачайте model pack \
                 со страницы официального выпуска."
            ),
        }
    }
}

/// How strictly a model directory is judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    /// Hash the whole file, not just compare its size.
    pub verify_checksum: bool,
    /// Refuse a pack whose digest was never checked against the repository
    /// pin. True for an official bundle, false on a developer machine where
    /// hand-made packs are legitimate.
    pub require_pinned: bool,
}

impl Requirements {
    /// What the running application should demand, given where it lives.
    pub fn for_layout(layout: Option<&Layout>, verify_checksum: bool) -> Self {
        Requirements {
            verify_checksum,
            require_pinned: layout.is_some_and(|layout| layout.is_packaged_bundle()),
        }
    }
}

/// Directories a model may live in, in order. No network is ever consulted.
pub fn search_dirs(layout: Option<&Layout>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("SCIWHISPER_MODEL_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(layout) = layout {
        dirs.push(layout.whisper_dir());
    }
    dirs
}

/// Inspects one model directory.
pub fn check(dir: &Path, requirements: Requirements) -> ModelStatus {
    let manifest_path = dir.join(MANIFEST_FILE);
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return ModelStatus::ManifestMissing;
    };
    let manifest = match ModelManifest::parse(&text) {
        Ok(manifest) => manifest,
        Err(error) => {
            return ModelStatus::ManifestInvalid {
                reason: error.to_string(),
            }
        }
    };
    // The digest travels in the same archive as the file it describes, so it
    // proves integrity, never origin. Origin is what the repository pin and
    // the release workflow are for, and this flag records whether that check
    // actually happened.
    if requirements.require_pinned && !manifest.verified_against_repository_pin {
        return ModelStatus::NotPinned;
    }
    let path = dir.join(&manifest.file);
    let Ok(metadata) = std::fs::metadata(&path) else {
        return ModelStatus::FileMissing {
            expected: manifest.file.clone(),
        };
    };
    if metadata.len() != manifest.size_bytes {
        return ModelStatus::SizeMismatch {
            expected: manifest.size_bytes,
            actual: metadata.len(),
        };
    }
    if requirements.verify_checksum {
        match sha256_file(&path) {
            Ok(actual) if actual.eq_ignore_ascii_case(&manifest.sha256) => {}
            Ok(actual) => {
                return ModelStatus::ChecksumMismatch {
                    expected: manifest.sha256.clone(),
                    actual,
                }
            }
            Err(error) => {
                return ModelStatus::ManifestInvalid {
                    reason: format!("не удалось прочитать файл модели: {error}"),
                }
            }
        }
    }
    ModelStatus::Ready {
        path,
        manifest: Box::new(manifest),
        checksum_verified: requirements.verify_checksum,
    }
}

/// The single decision every caller shares: which model would actually be
/// loaded, and if none, why.
///
/// `doctor` and the recogniser call this same function, so a green report can
/// never disagree with what the next recording does.
pub fn inspect(
    layout: Option<&Layout>,
    explicit: Option<&Path>,
    requirements: Requirements,
) -> ModelStatus {
    // A path the user gave is honoured or refused — never quietly replaced by
    // the bundled model.
    if let Some(path) = explicit {
        if path.is_file() {
            return ModelStatus::Ready {
                path: path.to_path_buf(),
                manifest: Box::new(ModelManifest::for_explicit_file(path)),
                checksum_verified: false,
            };
        }
        return ModelStatus::FileMissing {
            expected: display_name(path),
        };
    }
    let dirs = search_dirs(layout);
    if dirs.is_empty() {
        return ModelStatus::NoSearchLocation;
    }
    let mut last = ModelStatus::ManifestMissing;
    for dir in &dirs {
        let status = check(dir, requirements);
        if status.is_ready() {
            return status;
        }
        last = status;
    }
    last
}

/// Finds the model the backend should load, or explains why it cannot.
pub fn resolve(
    layout: Option<&Layout>,
    explicit: Option<&Path>,
    requirements: Requirements,
) -> Result<PathBuf> {
    match inspect(layout, explicit, requirements) {
        ModelStatus::Ready { path, .. } => Ok(path),
        other => Err(Error::ModelUnusable {
            detail: other.message(),
        }),
    }
}

// ---------------------------------------------------------------- sha-256

/// Streaming SHA-256, so a multi-gigabyte model is never held in memory.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lenient() -> Requirements {
        Requirements {
            verify_checksum: false,
            require_pinned: false,
        }
    }

    fn hashed() -> Requirements {
        Requirements {
            verify_checksum: true,
            require_pinned: false,
        }
    }

    fn official() -> Requirements {
        Requirements {
            verify_checksum: false,
            require_pinned: true,
        }
    }

    fn manifest_json(file: &str, size: u64, sha: &str) -> String {
        format!(
            r#"{{
  "schema_version": 1,
  "model_id": "small-q5_1",
  "file": "{file}",
  "size_bytes": {size},
  "sha256": "{sha}",
  "source_url": "example.invalid/ggml-small-q5_1.bin",
  "license": "MIT",
  "license_url": "example.invalid/LICENSE",
  "whisper_cpp_version": "v1.7.4",
  "verified_against_repository_pin": true,
  "notes": ""
}}"#
        )
    }

    fn write_model(dir: &Path, bytes: &[u8]) -> String {
        let sha = sha256_hex(bytes);
        std::fs::write(dir.join("ggml-small-q5_1.bin"), bytes).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            manifest_json("ggml-small-q5_1.bin", bytes.len() as u64, &sha),
        )
        .unwrap();
        sha
    }

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // A message longer than one block, to exercise the buffering path.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let long = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&long),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn streaming_and_in_memory_hashes_agree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&bytes));
    }

    #[test]
    fn a_complete_model_directory_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), b"pretend weights");
        let status = check(dir.path(), hashed());
        match &status {
            ModelStatus::Ready {
                manifest,
                checksum_verified,
                ..
            } => {
                assert_eq!(manifest.model_id, "small-q5_1");
                assert!(checksum_verified);
            }
            other => panic!("{other:?}"),
        }
        assert!(status.is_ready());
        assert!(status.message().contains("small-q5_1"));
    }

    #[test]
    fn a_missing_manifest_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let status = check(dir.path(), lenient());
        assert_eq!(status, ModelStatus::ManifestMissing);
        let message = status.message();
        assert!(message.contains("model pack"), "{message}");
        assert!(!status.is_ready());
    }

    #[test]
    fn a_missing_model_file_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            manifest_json("ggml-small-q5_1.bin", 10, &"a".repeat(64)),
        )
        .unwrap();
        match check(dir.path(), lenient()) {
            ModelStatus::FileMissing { expected } => assert_eq!(expected, "ggml-small-q5_1.bin"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_truncated_model_is_caught_by_size_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), b"pretend weights");
        std::fs::write(dir.path().join("ggml-small-q5_1.bin"), b"short").unwrap();
        match check(dir.path(), lenient()) {
            ModelStatus::SizeMismatch { expected, actual } => {
                assert_eq!(expected, 15);
                assert_eq!(actual, 5);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_corrupted_model_of_the_right_size_is_caught_by_the_checksum() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), b"pretend weights");
        // Same length, different bytes: only hashing can tell.
        std::fs::write(dir.path().join("ggml-small-q5_1.bin"), b"pretend WEIGHTS").unwrap();
        assert!(
            check(dir.path(), lenient()).is_ready(),
            "size alone still matches"
        );
        match check(dir.path(), hashed()) {
            ModelStatus::ChecksumMismatch { expected, actual } => assert_ne!(expected, actual),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_manifest_may_not_point_outside_its_own_directory() {
        for hostile in [
            "../../etc/passwd",
            "sub/dir/model.bin",
            "..\\\\windows\\\\system32",
        ] {
            let text = manifest_json(hostile, 1, &"b".repeat(64));
            let error = ModelManifest::parse(&text).expect_err("a path must be refused");
            assert!(error.to_string().contains("bare file name"), "{error}");
        }
    }

    #[test]
    fn an_unknown_manifest_schema_is_refused() {
        let text = manifest_json("m.bin", 1, &"c".repeat(64))
            .replace("\"schema_version\": 1", "\"schema_version\": 7");
        let error = ModelManifest::parse(&text).expect_err("a future schema must be refused");
        assert!(error.to_string().contains("schema"), "{error}");
    }

    #[test]
    fn a_malformed_checksum_is_refused() {
        let text = manifest_json("m.bin", 1, "not-a-digest");
        let error = ModelManifest::parse(&text).expect_err("a bad digest must be refused");
        assert!(error.to_string().contains("64 hex"), "{error}");
    }

    #[test]
    fn resolution_never_looks_beyond_the_directories_it_owns() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        std::fs::create_dir_all(layout.whisper_dir()).unwrap();
        // Nothing there yet: the failure explains itself and mentions no network.
        let error = resolve(Some(&layout), None, lenient()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("model pack"), "{message}");
        assert!(!message.contains("http"), "{message}");

        write_model(&layout.whisper_dir(), b"pretend weights");
        let path = resolve(Some(&layout), None, hashed()).unwrap();
        assert_eq!(path, layout.whisper_dir().join("ggml-small-q5_1.bin"));
    }

    #[test]
    fn an_official_bundle_refuses_a_pack_that_was_never_checked_against_the_pin() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), b"pretend weights");
        let text = std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            text.replace(
                "\"verified_against_repository_pin\": true",
                "\"verified_against_repository_pin\": false",
            ),
        )
        .unwrap();

        // A developer machine may use a hand-made pack.
        assert!(check(dir.path(), lenient()).is_ready());
        // An official bundle may not.
        let status = check(dir.path(), official());
        assert_eq!(status, ModelStatus::NotPinned);
        assert!(
            status.message().contains("официальн"),
            "{}",
            status.message()
        );
    }

    #[test]
    fn requirements_follow_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let developer = Layout::from_dir(dir.path());
        assert!(!Requirements::for_layout(Some(&developer), false).require_pinned);

        std::fs::write(dir.path().join(crate::backend::BUNDLE_MARKER), "sci-witch").unwrap();
        let shipped = Layout::from_dir(dir.path());
        assert!(Requirements::for_layout(Some(&shipped), false).require_pinned);
    }

    #[test]
    fn an_explicit_model_is_never_swapped_for_the_bundled_one() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::from_dir(dir.path());
        std::fs::create_dir_all(layout.whisper_dir()).unwrap();
        write_model(&layout.whisper_dir(), b"pretend weights");
        // The bundled model is present and healthy, and must still not be used
        // in place of the one the user asked for.
        let asked = dir.path().join("нет такого файла.bin");
        match inspect(Some(&layout), Some(&asked), lenient()) {
            ModelStatus::FileMissing { expected } => assert_eq!(expected, "нет такого файла.bin"),
            other => panic!("an explicit model was silently replaced: {other:?}"),
        }
        assert!(resolve(Some(&layout), Some(&asked), lenient()).is_err());
    }

    #[test]
    fn an_explicit_model_path_wins_and_a_bad_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = dir.path().join("Мои модели").join("своя модель.bin");
        std::fs::create_dir_all(chosen.parent().unwrap()).unwrap();
        std::fs::write(&chosen, b"weights").unwrap();
        assert_eq!(resolve(None, Some(&chosen), lenient()).unwrap(), chosen);

        let missing = dir.path().join("nope.bin");
        let error = resolve(None, Some(&missing), lenient()).unwrap_err();
        assert!(error.to_string().contains("nope.bin"), "{error}");
    }
}

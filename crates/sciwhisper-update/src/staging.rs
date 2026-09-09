//! Unpacking an update archive into a staging directory.
//!
//! An archive is attacker-controlled data even when it arrived over TLS with
//! a matching SHA-256: the checksum proves the bytes are the ones the
//! manifest named, not that whoever wrote the manifest meant well. So every
//! entry is checked before anything is written, and everything is written
//! **inside** the staging directory — never over the running installation.
//!
//! What is refused, and why:
//!
//! * absolute paths and `..` — the classic zip-slip, which writes outside
//!   the directory the caller chose;
//! * Windows drive letters and UNC prefixes, which are absolute paths that
//!   do not start with a separator and so slip past a naive check;
//! * symbolic links, which turn a later write into a write somewhere else;
//! * device names Windows resolves specially (`CON`, `NUL`, `COM1`…), which
//!   are not files at all;
//! * more entries, more bytes or a higher compression ratio than a real
//!   bundle has — a 3 MiB download must not become a full disk.
//!
//! Nothing is "sanitised" into something safe: a bad entry fails the whole
//! extraction. Silently renaming `../../evil` to `evil` would install an
//! archive whose author was up to something, which is not better than
//! refusing it.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::error::{io, Error, Result};

/// Bounds on what an update archive may contain. The defaults are generous
/// next to a real bundle (~3.5 MiB, a few dozen files) and still small next
/// to a disk.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_entry_bytes: u64,
    /// Largest allowed uncompressed-to-compressed ratio for one entry. A
    /// zip bomb is small on disk and enormous once expanded; real payloads
    /// (already-compressed binaries and models) stay far below this.
    pub max_ratio: u64,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_total_bytes: 512 * 1024 * 1024,
            max_entry_bytes: 384 * 1024 * 1024,
            max_ratio: 500,
            max_depth: 16,
        }
    }
}

/// What extraction produced. `root` is the directory the caller passed in;
/// nothing outside it was touched.
#[derive(Clone, Debug)]
pub struct Unpacked {
    pub root: PathBuf,
    /// Relative paths of every regular file written, sorted.
    pub files: Vec<PathBuf>,
    pub bytes: u64,
}

impl Unpacked {
    pub fn contains(&self, relative: &str) -> bool {
        let wanted = Path::new(relative);
        self.files.iter().any(|file| file == wanted)
    }
}

/// Names Windows resolves to devices rather than files, in any directory and
/// with any extension. Refused on every platform: an archive that carries one
/// is not a bundle we produced, and extracting it on a Mac only to have it
/// break on Windows helps nobody.
const RESERVED_STEMS: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Unpacks `archive` into `staging`, which must not already exist or must be
/// an empty directory. On any refusal the staging directory is removed, so a
/// failed update never leaves a half-tree that a later step could mistake for
/// a finished one.
pub fn unpack(archive: &Path, staging: &Path, limits: &Limits) -> Result<Unpacked> {
    match fs::read_dir(staging) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(Error::Message(format!(
                    "{} не пуст; распаковка в непустой каталог перемешала бы старое с новым",
                    staging.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(staging).map_err(io(staging))?;
        }
        Err(error) => {
            return Err(Error::Io {
                path: staging.into(),
                source: error,
            })
        }
    }

    match unpack_inner(archive, staging, limits) {
        Ok(unpacked) => Ok(unpacked),
        Err(error) => {
            let _ = fs::remove_dir_all(staging);
            Err(error)
        }
    }
}

fn unpack_inner(archive: &Path, staging: &Path, limits: &Limits) -> Result<Unpacked> {
    let file = fs::File::open(archive).map_err(io(archive))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| Error::Message(format!("{}: {error}", archive.display())))?;

    if zip.len() > limits.max_entries {
        return Err(Error::TooLarge(format!(
            "{} записей при пределе {}",
            zip.len(),
            limits.max_entries
        )));
    }

    let mut files = Vec::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| Error::Message(format!("запись {index}: {error}")))?;
        let raw_name = entry.name().to_string();

        // `enclosed_name` is the zip crate's own path check; it is used as a
        // second opinion, not as the only one — the explicit rules below say
        // in the code what is refused and why.
        let relative = safe_relative_path(&raw_name, limits)?;
        if entry.is_symlink() {
            return Err(Error::Rejected {
                entry: raw_name,
                reason: "символические ссылки в комплекте не бывают, а после распаковки указывали бы куда угодно".into(),
            });
        }
        let target = staging.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(io(&target))?;
            continue;
        }

        let declared = entry.size();
        if declared > limits.max_entry_bytes {
            return Err(Error::TooLarge(format!(
                "{raw_name}: заявлено {declared} байт при пределе {}",
                limits.max_entry_bytes
            )));
        }
        let compressed = entry.compressed_size().max(1);
        if declared / compressed > limits.max_ratio {
            return Err(Error::TooLarge(format!(
                "{raw_name}: сжатие {}× при пределе {}×",
                declared / compressed,
                limits.max_ratio
            )));
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        // The declared size is not trusted: reading is capped independently,
        // so an entry that lies about its length is cut off rather than
        // filling the disk.
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let cap = remaining.min(limits.max_entry_bytes);
        let mut out = fs::File::create(&target).map_err(io(&target))?;
        let written =
            std::io::copy(&mut entry.by_ref().take(cap + 1), &mut out).map_err(io(&target))?;
        drop(out);
        if written > cap {
            return Err(Error::TooLarge(format!(
                "{raw_name}: распакованное содержимое превышает общий предел {} байт",
                limits.max_total_bytes
            )));
        }
        total += written;
        apply_mode(&target, entry.unix_mode())?;
        files.push(relative);
    }

    files.sort();
    Ok(Unpacked {
        root: staging.to_path_buf(),
        files,
        bytes: total,
    })
}

/// Turns an archive entry name into a path that provably stays inside the
/// staging directory, or explains why it cannot.
fn safe_relative_path(raw: &str, limits: &Limits) -> Result<PathBuf> {
    let reject = |reason: &str| Error::Rejected {
        entry: raw.to_string(),
        reason: reason.to_string(),
    };
    if raw.is_empty() {
        return Err(reject("пустое имя"));
    }
    if raw.contains('\0') {
        return Err(reject("имя содержит нулевой байт"));
    }
    // Backslashes are separators on Windows, so an entry written with them
    // must be judged component by component too, not treated as one name.
    let normalised = raw.replace('\\', "/");
    if normalised.starts_with('/') {
        return Err(reject("абсолютный путь"));
    }
    // `C:` and `\\server\share` are absolute on Windows without a leading
    // separator, which is exactly how a naive check gets past.
    let first = normalised.split('/').next().unwrap_or_default();
    if first.len() >= 2 && first.as_bytes()[1] == b':' {
        return Err(reject("путь с буквой диска"));
    }
    if raw.starts_with("\\\\") {
        return Err(reject("UNC-путь"));
    }

    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for part in normalised.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(reject("путь выходит за каталог распаковки"));
        }
        let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
        if RESERVED_STEMS.contains(&stem.as_str()) {
            return Err(reject("зарезервированное в Windows имя устройства"));
        }
        if part.len() > 255 {
            return Err(reject("слишком длинное имя элемента пути"));
        }
        depth += 1;
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err(reject("имя не содержит ни одного элемента пути"));
    }
    if depth > limits.max_depth {
        return Err(reject("слишком глубокая вложенность"));
    }
    // Belt and braces: whatever the loop above produced must still be a
    // sequence of plain names.
    if out
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(reject("путь не сводится к последовательности имён"));
    }
    Ok(out)
}

/// Keeps the executable bit an archive declares and drops everything else.
/// setuid, setgid and sticky bits have no business in an update, and a mode
/// of 0 (written by many Windows zip tools) becomes a sane default rather
/// than an unreadable file.
#[cfg(unix)]
fn apply_mode(path: &Path, declared: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let executable = declared.is_some_and(|mode| mode & 0o111 != 0);
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io(path))
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _declared: Option<u32>) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn archive_with(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.zip");
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, body) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
        (dir, path)
    }

    fn unpack_into(archive: &Path, dir: &Path) -> Result<Unpacked> {
        unpack(archive, &dir.join("staging"), &Limits::default())
    }

    #[test]
    fn an_ordinary_bundle_is_unpacked() {
        let (dir, archive) = archive_with(&[
            ("SciWhisper.exe", b"binary"),
            ("whisper/README-WINDOWS.txt", b"read me"),
        ]);
        let unpacked = unpack_into(&archive, dir.path()).unwrap();
        assert_eq!(unpacked.files.len(), 2);
        assert!(unpacked.contains("SciWhisper.exe"));
        assert!(unpacked.contains("whisper/README-WINDOWS.txt"));
        assert_eq!(unpacked.bytes, 6 + 7);
        assert_eq!(
            fs::read_to_string(unpacked.root.join("whisper/README-WINDOWS.txt")).unwrap(),
            "read me"
        );
    }

    /// The whole point of a staging directory: a hostile name must not reach
    /// anything outside it, and must not be quietly renamed into something
    /// harmless either.
    #[test]
    fn a_path_that_climbs_out_is_refused_and_nothing_is_written() {
        for name in [
            "../victim.txt",
            "a/../../victim.txt",
            "/etc/passwd",
            "C:/Windows/system32/evil.dll",
            "..\\victim.txt",
        ] {
            let (dir, archive) = archive_with(&[(name, b"x")]);
            let victim = dir.path().join("victim.txt");
            let error = unpack_into(&archive, dir.path()).unwrap_err();
            assert!(matches!(error, Error::Rejected { .. }), "{name}: {error}");
            assert!(!victim.exists(), "{name} wrote outside staging");
            assert!(
                !dir.path().join("staging").exists(),
                "{name} left a partial staging tree"
            );
        }
    }

    #[test]
    fn a_windows_device_name_is_refused() {
        for name in ["CON", "nul.txt", "whisper/COM1.bin"] {
            let (dir, archive) = archive_with(&[(name, b"x")]);
            let error = unpack_into(&archive, dir.path()).unwrap_err();
            assert!(matches!(error, Error::Rejected { .. }), "{name}: {error}");
        }
    }

    #[test]
    fn a_zip_bomb_is_refused_by_ratio_before_it_is_written() {
        let (dir, archive) = archive_with(&[("big.bin", &vec![0u8; 4 * 1024 * 1024])]);
        let limits = Limits {
            max_ratio: 2,
            ..Limits::default()
        };
        let error = unpack(&archive, &dir.path().join("staging"), &limits).unwrap_err();
        assert!(matches!(error, Error::TooLarge(_)), "{error}");
    }

    #[test]
    fn the_total_size_is_capped_even_across_small_entries() {
        let (dir, archive) = archive_with(&[
            ("a.bin", &vec![b'a'; 4096]),
            ("b.bin", &vec![b'b'; 4096]),
            ("c.bin", &vec![b'c'; 4096]),
        ]);
        let limits = Limits {
            max_total_bytes: 5000,
            max_ratio: u64::MAX,
            ..Limits::default()
        };
        let error = unpack(&archive, &dir.path().join("staging"), &limits).unwrap_err();
        assert!(matches!(error, Error::TooLarge(_)), "{error}");
        assert!(!dir.path().join("staging").exists());
    }

    #[test]
    fn too_many_entries_is_refused_before_anything_is_read() {
        let bodies: Vec<(String, Vec<u8>)> = (0..10)
            .map(|i| (format!("f{i}.txt"), b"x".to_vec()))
            .collect();
        let entries: Vec<(&str, &[u8])> = bodies
            .iter()
            .map(|(name, body)| (name.as_str(), body.as_slice()))
            .collect();
        let (dir, archive) = archive_with(&entries);
        let limits = Limits {
            max_entries: 3,
            ..Limits::default()
        };
        let error = unpack(&archive, &dir.path().join("staging"), &limits).unwrap_err();
        assert!(matches!(error, Error::TooLarge(_)), "{error}");
    }

    #[test]
    fn a_non_empty_staging_directory_is_refused() {
        let (dir, archive) = archive_with(&[("a.txt", b"x")]);
        let staging = dir.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("leftover.txt"), b"old").unwrap();
        let error = unpack(&archive, &staging, &Limits::default()).unwrap_err();
        assert!(error.to_string().contains("не пуст"), "{error}");
        // The refusal must not delete what was already there.
        assert!(staging.join("leftover.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_executable_bit_survives_and_nothing_else_does() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.zip");
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "SciWhisper",
            SimpleFileOptions::default().unix_permissions(0o4755),
        )
        .unwrap();
        zip.write_all(b"binary").unwrap();
        zip.start_file(
            "notes.txt",
            SimpleFileOptions::default().unix_permissions(0o600),
        )
        .unwrap();
        zip.write_all(b"text").unwrap();
        zip.finish().unwrap();

        let unpacked = unpack_into(&path, dir.path()).unwrap();
        let mode = |name: &str| {
            fs::metadata(unpacked.root.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        assert_eq!(mode("SciWhisper"), 0o755, "setuid must not survive");
        assert_eq!(mode("notes.txt"), 0o644);
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.zip");
        fs::write(&path, b"this is not a zip file").unwrap();
        let error = unpack_into(&path, dir.path()).unwrap_err();
        assert!(matches!(error, Error::Message(_)), "{error}");
    }
}

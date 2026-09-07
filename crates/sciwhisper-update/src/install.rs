//! Putting a staged tree in place, and putting the old one back if that
//! fails.
//!
//! The rule the whole module exists to keep: **at no point may the user be
//! left without a working installation.** Everything else — speed, tidiness,
//! deleting the old copy promptly — comes second.
//!
//! The replacement is two directory renames, not a file-by-file copy. A copy
//! has a window in which half the new version sits next to half the old one,
//! and a power cut inside that window leaves something that starts and
//! misbehaves. Two renames have no such state: after the first the old
//! installation is intact under a different name, after the second the new
//! one is in place, and a failure between them is undone by renaming the old
//! one back.
//!
//! On Windows a directory holding a running executable cannot be renamed,
//! which is why [`apply_with_retry`] exists and why the helper binary does
//! the work after the application has exited.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{io, Error, Result};
use crate::staging::Unpacked;

/// The file name of the application binary on this platform.
///
/// It is the Cargo bin name, lower-case — the same string
/// `packaging/windows/BUNDLE_CONTENTS.json` lists and the release workflow
/// copies. Getting this wrong would make every staged bundle fail
/// verification for the one reason nobody would look for, so
/// `the_expected_executable_is_the_one_the_bundle_ships` pins it against
/// that file.
pub const fn executable_name() -> &'static str {
    if cfg!(windows) {
        "sciwhisper.exe"
    } else {
        "sciwhisper"
    }
}

/// What a staged tree must contain before it is allowed to replace a working
/// installation. Checked against what was actually written, not against what
/// the archive claimed.
#[derive(Clone, Debug)]
pub struct Expectation {
    pub required: Vec<PathBuf>,
}

impl Expectation {
    /// The minimum: the program itself. An update that does not carry an
    /// executable is not an update, whatever else it contains.
    pub fn for_platform() -> Self {
        Self {
            required: vec![PathBuf::from(executable_name())],
        }
    }

    pub fn and(mut self, relative: impl Into<PathBuf>) -> Self {
        self.required.push(relative.into());
        self
    }
}

/// Checks a staged tree before anything irreversible happens to the
/// installation it would replace.
pub fn verify(unpacked: &Unpacked, expectation: &Expectation) -> Result<()> {
    for required in &expectation.required {
        if !unpacked.files.iter().any(|file| file == required) {
            return Err(Error::NotABundle(format!(
                "нет обязательного файла {}",
                required.display()
            )));
        }
        let path = unpacked.root.join(required);
        let size = fs::metadata(&path).map_err(io(&path))?.len();
        if size == 0 {
            return Err(Error::NotABundle(format!("{} пустой", required.display())));
        }
    }
    Ok(())
}

/// The three directories a replacement moves between.
#[derive(Clone, Debug)]
pub struct Swap {
    /// The installation in use.
    pub current: PathBuf,
    /// The verified new tree.
    pub staged: PathBuf,
    /// Where the old installation is kept. Must not exist yet: overwriting
    /// it would destroy the only copy that is known to work.
    pub backup: PathBuf,
}

/// A completed replacement. The backup is kept — deleting it is a separate
/// decision, made after the new version has actually started.
#[derive(Clone, Debug)]
pub struct Applied {
    pub backup: PathBuf,
}

pub fn apply(swap: &Swap) -> Result<Applied> {
    apply_with(swap, |from, to| fs::rename(from, to))
}

/// The replacement itself.
///
/// If the second rename fails, the first is undone before returning, and the
/// error says whether that succeeded — a caller that sees "откат не удался"
/// is looking at the one situation where a human has to intervene, and it
/// must not be reported in the same words as an ordinary failure.
///
/// Renaming is a parameter so that the failure this function exists to
/// survive can actually be provoked in a test; production passes
/// [`fs::rename`].
pub fn apply_with<F>(swap: &Swap, mut rename: F) -> Result<Applied>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    if !swap.staged.is_dir() {
        return Err(Error::Message(format!(
            "{} не каталог: заменять нечем",
            swap.staged.display()
        )));
    }
    if fs::read_dir(&swap.staged)
        .map_err(io(&swap.staged))?
        .next()
        .is_none()
    {
        return Err(Error::Message(format!(
            "{} пуст: заменять нечем",
            swap.staged.display()
        )));
    }
    if !swap.current.is_dir() {
        return Err(Error::Message(format!(
            "{} не каталог: заменять нечего",
            swap.current.display()
        )));
    }
    if swap.backup.exists() {
        return Err(Error::Message(format!(
            "{} уже существует; перезапись стёрла бы единственную заведомо рабочую копию",
            swap.backup.display()
        )));
    }
    if let Some(parent) = swap.backup.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }

    rename(&swap.current, &swap.backup).map_err(io(&swap.current))?;

    match rename(&swap.staged, &swap.current) {
        Ok(()) => Ok(Applied {
            backup: swap.backup.clone(),
        }),
        Err(error) => match rename(&swap.backup, &swap.current) {
            Ok(()) => Err(Error::Message(format!(
                "новая версия не установлена ({error}); прежняя возвращена на место"
            ))),
            Err(restore) => Err(Error::Message(format!(
                "новая версия не установлена ({error}), и откат не удался ({restore}). \
                 Рабочая копия лежит в {}, её нужно вернуть в {} вручную",
                swap.backup.display(),
                swap.current.display()
            ))),
        },
    }
}

/// Retries [`apply`] while the installation is still held open.
///
/// On Windows the directory containing the running `SciWhisper.exe` cannot be
/// renamed until the process has actually exited, and "has exited" is not the
/// same instant as "asked to exit". Rather than guess a delay or watch a PID,
/// this asks the filesystem the only question that matters — can the
/// directory be renamed yet — and keeps asking until the budget runs out.
///
/// A failure that is not "still in use" is not retried: an archive missing
/// its executable will still be missing it in ten seconds.
pub fn apply_with_retry(
    swap: &Swap,
    attempts: usize,
    delay: Duration,
    sleep: &mut dyn FnMut(Duration),
) -> Result<Applied> {
    retry(attempts, delay, sleep, || apply(swap))
}

/// The retry policy on its own, so it can be tested without a filesystem
/// that refuses to cooperate on cue.
pub fn retry<F>(
    attempts: usize,
    delay: Duration,
    sleep: &mut dyn FnMut(Duration),
    mut attempt: F,
) -> Result<Applied>
where
    F: FnMut() -> Result<Applied>,
{
    let attempts = attempts.max(1);
    let mut last = None;
    for index in 0..attempts {
        match attempt() {
            Ok(applied) => return Ok(applied),
            Err(error) => {
                if !looks_like_still_in_use(&error) {
                    return Err(error);
                }
                last = Some(error);
                if index + 1 < attempts {
                    sleep(delay);
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| Error::Message("замена не выполнена".into())))
}

/// Whether an error is the kind that goes away once the old process lets go.
fn looks_like_still_in_use(error: &Error) -> bool {
    let Error::Io { source, .. } = error else {
        return false;
    };
    matches!(
        source.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
    ) || source.raw_os_error() == Some(32) // ERROR_SHARING_VIOLATION
        || source.raw_os_error() == Some(5) // ERROR_ACCESS_DENIED
}

/// Puts a kept backup back, for the case where the new version installs but
/// does not work. The version being replaced is moved aside rather than
/// deleted, so a bad rollback is still recoverable.
pub fn roll_back(current: &Path, backup: &Path, rejected: &Path) -> Result<()> {
    if !backup.is_dir() {
        return Err(Error::Message(format!(
            "{} нет: возвращать нечего",
            backup.display()
        )));
    }
    if rejected.exists() {
        return Err(Error::Message(format!(
            "{} уже существует",
            rejected.display()
        )));
    }
    if current.exists() {
        fs::rename(current, rejected).map_err(io(current))?;
    }
    match fs::rename(backup, current) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Undo our own first move so the caller is left with the state
            // they had, not with an installation directory that vanished.
            let _ = fs::rename(rejected, current);
            Err(Error::Io {
                path: backup.into(),
                source: error,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging::{unpack, Limits};
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn tree(root: &Path, files: &[(&str, &str)]) {
        for (name, body) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
    }

    fn swap_in(dir: &Path) -> Swap {
        Swap {
            current: dir.join("SciWhisper"),
            staged: dir.join("staging"),
            backup: dir.join("SciWhisper.backup"),
        }
    }

    #[test]
    fn the_new_tree_takes_the_place_and_the_old_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        tree(dir.path(), &[("staging/marker.txt", "new")]);
        let swap = swap_in(dir.path());

        let applied = apply(&swap).unwrap();
        assert_eq!(
            fs::read_to_string(swap.current.join("marker.txt")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(applied.backup.join("marker.txt")).unwrap(),
            "old",
            "the working copy must survive the update that replaced it"
        );
        assert!(!swap.staged.exists());
    }

    /// The failure this module exists for: the second move fails and the user
    /// must still have the version they had this morning.
    #[test]
    fn a_failed_replacement_puts_the_old_installation_back() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        tree(dir.path(), &[("staging/marker.txt", "new")]);
        let swap = swap_in(dir.path());

        let mut calls = 0;
        let error = apply_with(&swap, |from, to| {
            calls += 1;
            if calls == 2 {
                Err(std::io::Error::other("диск отвалился"))
            } else {
                fs::rename(from, to)
            }
        })
        .unwrap_err();

        assert!(
            error.to_string().contains("прежняя возвращена на место"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(swap.current.join("marker.txt")).unwrap(),
            "old"
        );
        assert!(!swap.backup.exists());
    }

    /// The one case a person has to be told about in different words.
    #[test]
    fn a_failed_rollback_says_where_the_working_copy_is() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        tree(dir.path(), &[("staging/marker.txt", "new")]);
        let swap = swap_in(dir.path());

        let mut calls = 0;
        let error = apply_with(&swap, |from, to| {
            calls += 1;
            match calls {
                1 => fs::rename(from, to),
                _ => Err(std::io::Error::other("нет")),
            }
        })
        .unwrap_err();

        let text = error.to_string();
        assert!(text.contains("откат не удался"), "{text}");
        assert!(text.contains("SciWhisper.backup"), "{text}");
        assert!(
            fs::read_to_string(swap.backup.join("marker.txt")).unwrap() == "old",
            "the message must point at a copy that is really there"
        );
    }

    #[test]
    fn an_existing_backup_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        tree(dir.path(), &[("staging/marker.txt", "new")]);
        tree(dir.path(), &[("SciWhisper.backup/marker.txt", "older")]);
        let error = apply(&swap_in(dir.path())).unwrap_err();
        assert!(error.to_string().contains("уже существует"), "{error}");
        assert_eq!(
            fs::read_to_string(dir.path().join("SciWhisper.backup/marker.txt")).unwrap(),
            "older"
        );
    }

    #[test]
    fn an_empty_or_missing_staging_tree_is_refused_before_anything_moves() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        let swap = swap_in(dir.path());
        assert!(apply(&swap).is_err());
        fs::create_dir_all(&swap.staged).unwrap();
        let error = apply(&swap).unwrap_err();
        assert!(error.to_string().contains("пуст"), "{error}");
        assert!(swap.current.join("marker.txt").exists());
    }

    /// Windows refuses to rename a directory holding a running program, and
    /// that refusal is temporary — it must be waited out, not reported.
    #[test]
    fn a_directory_still_in_use_is_waited_for_rather_than_given_up_on() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        tree(dir.path(), &[("staging/marker.txt", "new")]);
        let swap = swap_in(dir.path());

        let mut busy = 2;
        let mut naps = Vec::new();
        let applied = retry(5, Duration::from_millis(7), &mut |d| naps.push(d), || {
            if busy > 0 {
                busy -= 1;
                // ERROR_SHARING_VIOLATION: what Windows returns while the old
                // process still holds the executable open.
                return Err(Error::Io {
                    path: swap.current.clone(),
                    source: std::io::Error::from_raw_os_error(32),
                });
            }
            apply(&swap)
        })
        .unwrap();

        assert_eq!(
            naps.len(),
            2,
            "one wait per busy attempt, none after success"
        );
        assert_eq!(
            fs::read_to_string(swap.current.join("marker.txt")).unwrap(),
            "new"
        );
        assert!(applied.backup.exists());
    }

    #[test]
    fn a_directory_that_stays_in_use_gives_up_with_the_real_reason() {
        let mut naps = 0;
        let error = retry(3, Duration::from_millis(1), &mut |_| naps += 1, || {
            Err(Error::Io {
                path: PathBuf::from("SciWhisper"),
                source: std::io::Error::from_raw_os_error(32),
            })
        })
        .unwrap_err();
        assert!(matches!(error, Error::Io { .. }), "{error}");
        assert_eq!(naps, 2, "no wait after the last attempt");
    }

    #[test]
    fn a_permanent_failure_is_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "old")]);
        let swap = swap_in(dir.path());
        let mut naps = 0;
        let error =
            apply_with_retry(&swap, 5, Duration::from_millis(1), &mut |_| naps += 1).unwrap_err();
        assert!(error.to_string().contains("заменять нечем"), "{error}");
        assert_eq!(naps, 0, "a missing archive will still be missing later");
    }

    #[test]
    fn rolling_back_restores_the_kept_copy_and_keeps_the_rejected_one() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path(), &[("SciWhisper/marker.txt", "new-and-broken")]);
        tree(
            dir.path(),
            &[("SciWhisper.backup/marker.txt", "old-and-working")],
        );
        roll_back(
            &dir.path().join("SciWhisper"),
            &dir.path().join("SciWhisper.backup"),
            &dir.path().join("SciWhisper.rejected"),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("SciWhisper/marker.txt")).unwrap(),
            "old-and-working"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("SciWhisper.rejected/marker.txt")).unwrap(),
            "new-and-broken",
            "the version that failed is kept for diagnosis, not deleted"
        );
    }

    #[test]
    fn a_staged_tree_without_the_program_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("update.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("notes.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"only notes").unwrap();
        zip.finish().unwrap();

        let unpacked = unpack(&archive, &dir.path().join("staging"), &Limits::default()).unwrap();
        let error = verify(&unpacked, &Expectation::for_platform()).unwrap_err();
        assert!(matches!(error, Error::NotABundle(_)), "{error}");
    }

    #[test]
    fn an_empty_executable_is_not_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("update.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(executable_name(), SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();

        let unpacked = unpack(&archive, &dir.path().join("staging"), &Limits::default()).unwrap();
        let error = verify(&unpacked, &Expectation::for_platform()).unwrap_err();
        assert!(error.to_string().contains("пустой"), "{error}");
    }
}

#[cfg(test)]
mod bundle_tests {

    /// The name this crate expects and the name the release workflow ships
    /// must be the same string. They live in different files, in different
    /// languages, and nothing but this test connects them.
    #[test]
    fn the_expected_executable_is_the_one_the_bundle_ships() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/windows/BUNDLE_CONTENTS.json");
        let text = std::fs::read_to_string(&path).expect("BUNDLE_CONTENTS.json must exist");
        let contents: serde_json::Value =
            serde_json::from_str(&text).expect("BUNDLE_CONTENTS.json must be valid JSON");
        let required: Vec<&str> = contents["required_files"]
            .as_array()
            .expect("required_files")
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert!(
            required.contains(&"sciwhisper.exe"),
            "the bundle no longer ships sciwhisper.exe: {required:?}"
        );
        // The helper has to be in the bundle too, or an update can be
        // downloaded and verified and then never installed.
        assert!(
            required.contains(&"sciwhisper-updater.exe"),
            "the bundle must ship the updater helper: {required:?}"
        );
    }
}

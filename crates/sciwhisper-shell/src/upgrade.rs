//! The update flow as the user experiences it: check, decide, stage,
//! replace — with a press of a menu item in front of every step that costs
//! something or changes something.
//!
//! Nothing here happens on a timer. There is no periodic check, no
//! background download and no silent replacement: an offline dictation tool
//! that quietly reaches the network, or quietly changes its own binary, has
//! broken a promise its users chose it for.
//!
//! The steps are separated because they have different consequences:
//!
//! | Step | Costs | Reversible |
//! |---|---|---|
//! | check | one HTTPS request | nothing happened |
//! | download and stage | bandwidth, disk | delete the staging directory |
//! | replace | the installation | rollback from the kept backup |
//!
//! The user presses once to go from "check" to "download", and once more to
//! go from "downloaded" to "installed". The second press restarts the
//! program, which is stated in the menu item rather than discovered.

use std::path::{Path, PathBuf};

use sciwhisper_update::install::{self, Expectation};
use sciwhisper_update::staging::{self, Limits};

use crate::error::{Error, Result};
use crate::update::UpdateInfo;

/// Where the pieces of an update live, derived from the installation
/// directory. Siblings, never children: a staging directory inside the
/// installation would be swept away by the very rename that installs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub current: PathBuf,
    pub staging: PathBuf,
    pub backup: PathBuf,
    pub archive: PathBuf,
    pub helper: PathBuf,
}

pub fn helper_name() -> &'static str {
    if cfg!(windows) {
        "sciwhisper-updater.exe"
    } else {
        "sciwhisper-updater"
    }
}

/// Names every path used by an update of `version` installed at `current`.
/// Pure, so the naming can be checked without touching a disk.
pub fn plan_paths(current: &Path, version: &str) -> Paths {
    let parent = current.parent().unwrap_or(Path::new("."));
    let stem = current
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "SciWhisper".into());
    let safe: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Paths {
        current: current.to_path_buf(),
        staging: parent.join(format!("{stem}.staging-{safe}")),
        backup: parent.join(format!("{stem}.backup-{safe}")),
        archive: parent.join(format!("{stem}.download-{safe}")),
        helper: current.join(helper_name()),
    }
}

/// The directory that would be replaced by an update.
///
/// macOS is deliberately excluded. The application there is an ad-hoc signed
/// bundle, and replacing it in place resets the microphone, Accessibility and
/// Input Monitoring permissions the user granted — the update would appear to
/// work and the program would then silently fail to hear anything. Until
/// there is a Developer ID signature, macOS gets "download it yourself".
pub fn install_root() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        return Err(Error::Message(
            "на macOS замена выполняется вручную: подпись ad-hoc, и автозамена сбросила бы выданные разрешения на микрофон и Accessibility".into(),
        ));
    }
    let exe = std::env::current_exe().map_err(|e| Error::Message(e.to_string()))?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Message("не удалось определить каталог установки".into()))?;
    Ok(dir.to_path_buf())
}

/// A verified, unpacked tree waiting to be put in place.
#[derive(Clone, Debug)]
pub struct Staged {
    pub version: String,
    pub paths: Paths,
    pub files: usize,
    pub bytes: u64,
}

/// Unpacks a downloaded archive and checks that what came out is an
/// installation. Everything happens beside the running program, which is not
/// touched until [`start_replacement`].
pub fn stage(update: &UpdateInfo, archive: &Path, current: &Path) -> Result<Staged> {
    let paths = plan_paths(current, &update.version);
    // A leftover staging tree from an interrupted attempt is ours to remove:
    // it is named after this version and lives outside the installation.
    if paths.staging.exists() {
        std::fs::remove_dir_all(&paths.staging).map_err(|e| Error::Message(e.to_string()))?;
    }
    let unpacked = staging::unpack(archive, &paths.staging, &Limits::default())
        .map_err(|e| Error::Message(e.to_string()))?;
    install::verify(&unpacked, &Expectation::for_platform())
        .map_err(|e| Error::Message(e.to_string()))?;
    // The archive has done its job: the verified tree is on disk. It is
    // removed only on success, so a staging failure leaves the download to
    // look at rather than a second slow download to reproduce it.
    let _ = std::fs::remove_dir_all(&paths.archive);
    Ok(Staged {
        version: update.version.clone(),
        files: unpacked.files.len(),
        bytes: unpacked.bytes,
        paths,
    })
}

/// The command line the helper is started with. Split out so the arguments
/// can be asserted without spawning anything.
pub fn helper_command(staged: &Staged) -> (PathBuf, Vec<String>) {
    let paths = &staged.paths;
    let relaunch = paths.current.join(install::executable_name());
    (
        paths.helper.clone(),
        vec![
            "--current".into(),
            paths.current.display().to_string(),
            "--staged".into(),
            paths.staging.display().to_string(),
            "--backup".into(),
            paths.backup.display().to_string(),
            "--relaunch".into(),
            relaunch.display().to_string(),
        ],
    )
}

/// Starts the helper and returns; the caller must then quit, because the
/// helper is waiting for exactly that.
pub fn start_replacement(staged: &Staged) -> Result<()> {
    let (helper, args) = helper_command(staged);
    if !helper.is_file() {
        return Err(Error::Message(format!(
            "нет помощника {}; замена работающей программы без него не выполняется",
            helper.display()
        )));
    }
    std::process::Command::new(&helper)
        .args(&args)
        .spawn()
        .map_err(|e| Error::Message(format!("{}: {e}", helper.display())))?;
    Ok(())
}

/// What the user is told when an update is found. One line for a
/// notification, so it says the version, the size of the decision, and what
/// pressing next will do — not "an update is available".
pub fn describe(update: &UpdateInfo, current_version: &str) -> String {
    format!(
        "Версия {} доступна (сейчас {}). Ничего не скачано и не изменено: нажмите «Скачать обновление», чтобы получить архив, и «Что нового», чтобы прочитать изменения.",
        update.version, current_version
    )
}

/// What the user is told once the new version is unpacked and checked.
pub fn describe_staged(staged: &Staged) -> String {
    format!(
        "Версия {} проверена и распакована ({} файлов, {:.1} МиБ). Установка заменит программу и перезапустит её — до нажатия ничего не заменено.",
        staged.version,
        staged.files,
        staged.bytes as f64 / (1024.0 * 1024.0)
    )
}

/// Opens the release notes in the user's browser. A separate, explicit
/// action: the application does not fetch or render release notes itself.
pub fn open_notes(url: &str) -> Result<()> {
    // The URL comes from a manifest that was already checked against the URL
    // rebuilt from the pinned repository and tag, but handing anything to the
    // shell deserves a second look.
    if !url.starts_with("https://github.com/") {
        return Err(Error::Message(format!("не открываю {url}")));
    }
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| Error::Message(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::ManifestPlatform;

    fn update(version: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.into(),
            notes_url: format!(
                "https://github.com/pinkprincess766/sci-witch/releases/tag/v{version}"
            ),
            platform: ManifestPlatform {
                asset_name: format!("SciWhisper-{version}-Windows-x64.zip"),
                sha256: "0".repeat(64),
                url: "https://github.com/x/y/releases/download/v1/z.zip".into(),
            },
        }
    }

    /// Staging beside the installation, not inside it: a child directory
    /// would be carried off by the rename that installs it.
    #[test]
    fn every_working_path_is_a_sibling_of_the_installation() {
        let paths = plan_paths(Path::new("/opt/SciWhisper"), "0.2.0");
        assert_eq!(paths.staging, Path::new("/opt/SciWhisper.staging-0.2.0"));
        assert_eq!(paths.backup, Path::new("/opt/SciWhisper.backup-0.2.0"));
        assert_eq!(paths.archive, Path::new("/opt/SciWhisper.download-0.2.0"));
        for path in [&paths.staging, &paths.backup, &paths.archive] {
            assert!(!path.starts_with("/opt/SciWhisper/"), "{}", path.display());
        }
        assert!(paths.helper.starts_with("/opt/SciWhisper/"));
    }

    /// A version string reaches the filesystem, so it may not carry a path.
    #[test]
    fn a_version_cannot_smuggle_a_path_into_a_directory_name() {
        let paths = plan_paths(Path::new("/opt/SciWhisper"), "../../etc/passwd");
        assert_eq!(
            paths.staging,
            Path::new("/opt/SciWhisper.staging-.._.._etc_passwd")
        );
        assert!(!paths
            .staging
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)));
    }

    #[test]
    fn the_helper_is_told_exactly_which_directories_to_move() {
        let staged = Staged {
            version: "0.2.0".into(),
            paths: plan_paths(Path::new("/opt/SciWhisper"), "0.2.0"),
            files: 12,
            bytes: 1024,
        };
        let (helper, args) = helper_command(&staged);
        assert_eq!(helper, Path::new("/opt/SciWhisper").join(helper_name()));
        assert_eq!(args[0], "--current");
        assert_eq!(args[1], "/opt/SciWhisper");
        assert_eq!(args[3], "/opt/SciWhisper.staging-0.2.0");
        assert_eq!(args[5], "/opt/SciWhisper.backup-0.2.0");
        // The program it restarts is the one it just installed.
        assert!(args[7].starts_with("/opt/SciWhisper/"));
    }

    #[test]
    fn nothing_is_replaced_without_a_helper_to_do_it() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged {
            version: "0.2.0".into(),
            paths: plan_paths(dir.path(), "0.2.0"),
            files: 1,
            bytes: 1,
        };
        let error = start_replacement(&staged).unwrap_err();
        assert!(error.to_string().contains("нет помощника"), "{error}");
    }

    /// The notification has to say what has *not* happened yet, or a user
    /// reasonably assumes an update tool has updated something.
    #[test]
    fn the_message_says_that_nothing_has_been_changed_yet() {
        let text = describe(&update("0.2.0"), "0.1.1-rc1");
        assert!(text.contains("0.2.0") && text.contains("0.1.1-rc1"));
        assert!(text.contains("Ничего не скачано и не изменено"));
    }

    #[test]
    fn the_staged_message_says_the_program_will_restart() {
        let staged = Staged {
            version: "0.2.0".into(),
            paths: plan_paths(Path::new("/opt/SciWhisper"), "0.2.0"),
            files: 12,
            bytes: 3_670_016,
        };
        let text = describe_staged(&staged);
        assert!(text.contains("12 файлов"), "{text}");
        assert!(text.contains("3.5 МиБ"), "{text}");
        assert!(text.contains("перезапустит"), "{text}");
    }

    #[test]
    fn only_release_notes_on_the_project_are_opened() {
        for url in [
            "http://github.com/x",
            "https://example.com/notes",
            "file:///etc/passwd",
            "https://github.com.evil.test/x",
        ] {
            assert!(open_notes(url).is_err(), "{url}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_refuses_in_place_replacement_and_says_why() {
        let error = install_root().unwrap_err();
        assert!(error.to_string().contains("разрешения"), "{error}");
    }
}

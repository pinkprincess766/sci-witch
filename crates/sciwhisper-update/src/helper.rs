//! The helper that replaces an installation the application itself cannot
//! replace.
//!
//! On Windows a process cannot rename the directory its own executable lives
//! in, so the application spawns this helper, exits, and the helper does the
//! swap once the directory is free. It is a separate program with a separate
//! crate for a reason: it runs unattended, after the user interface is gone,
//! so it links no GUI, no recogniser and nothing that can reach the network.
//!
//! It also refuses to be a general-purpose launcher. `--relaunch` may only
//! name a file inside the directory that was just installed: whoever can
//! invoke the helper must not thereby be able to start an arbitrary program.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::install::{self, Swap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub current: PathBuf,
    pub staged: PathBuf,
    pub backup: PathBuf,
    /// Path of the program to start once the swap succeeded. Must sit inside
    /// `current`.
    pub relaunch: Option<PathBuf>,
    pub attempts: usize,
    pub delay: Duration,
}

pub const USAGE: &str = "\
sciwhisper-updater --current <dir> --staged <dir> --backup <dir>
                   [--relaunch <exe>] [--attempts <n>] [--delay-ms <n>]

Replaces <current> with <staged>, keeping the old copy in <backup>.
Retries while the directory is still held open by the exiting application.";

/// Hand-written rather than pulled from an argument-parsing crate: this
/// program takes six options and must stay free of dependencies it does not
/// need. Unknown options are refused instead of ignored — a typo that
/// silently changed which directory got replaced would be the worst kind of
/// bug here.
pub fn parse_args<I, S>(args: I) -> Result<Args>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut current = None;
    let mut staged = None;
    let mut backup = None;
    let mut relaunch = None;
    let mut attempts = 60usize;
    let mut delay_ms = 500u64;

    let mut items = args.into_iter().map(Into::into);
    while let Some(raw) = items.next() {
        let flag = raw.to_string_lossy().into_owned();
        let mut value = || -> Result<OsString> {
            items
                .next()
                .ok_or_else(|| Error::Message(format!("у {flag} нет значения")))
        };
        match flag.as_str() {
            "--current" => current = Some(PathBuf::from(value()?)),
            "--staged" => staged = Some(PathBuf::from(value()?)),
            "--backup" => backup = Some(PathBuf::from(value()?)),
            "--relaunch" => relaunch = Some(PathBuf::from(value()?)),
            "--attempts" => {
                attempts = value()?
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| Error::Message("--attempts ожидает число".into()))?
            }
            "--delay-ms" => {
                delay_ms = value()?
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| Error::Message("--delay-ms ожидает число".into()))?
            }
            other => {
                return Err(Error::Message(format!(
                    "неизвестный аргумент {other}\n\n{USAGE}"
                )))
            }
        }
    }

    let require = |name: &str, value: Option<PathBuf>| {
        value.ok_or_else(|| Error::Message(format!("не задан {name}\n\n{USAGE}")))
    };
    Ok(Args {
        current: require("--current", current)?,
        staged: require("--staged", staged)?,
        backup: require("--backup", backup)?,
        relaunch,
        attempts: attempts.clamp(1, 3600),
        delay: Duration::from_millis(delay_ms.clamp(10, 10_000)),
    })
}

/// Whether `candidate` names something inside `root`, judged on the paths as
/// written. Both are compared after normalising away `.`; a `..` anywhere is
/// refused outright rather than resolved, because resolving it would depend
/// on symlinks that may not exist yet.
fn inside(root: &Path, candidate: &Path) -> bool {
    if candidate
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return false;
    }
    let strip = |path: &Path| -> PathBuf {
        path.components()
            .filter(|part| !matches!(part, std::path::Component::CurDir))
            .collect()
    };
    strip(candidate).starts_with(strip(root)) && strip(candidate) != strip(root)
}

/// Does the swap and, if asked, starts the freshly installed program.
/// Returns the line to print; the caller decides where it goes.
pub fn run(
    args: &Args,
    sleep: &mut dyn FnMut(Duration),
    launch: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> Result<String> {
    if let Some(relaunch) = &args.relaunch {
        if !inside(&args.current, relaunch) {
            return Err(Error::Message(format!(
                "--relaunch {} вне устанавливаемого каталога {}; \
                 помощник запускает только то, что сам поставил",
                relaunch.display(),
                args.current.display()
            )));
        }
    }

    let swap = Swap {
        current: args.current.clone(),
        staged: args.staged.clone(),
        backup: args.backup.clone(),
    };
    let applied = install::apply_with_retry(&swap, args.attempts, args.delay, sleep)?;

    let mut message = format!(
        "установлено в {}, прежняя версия сохранена в {}",
        args.current.display(),
        applied.backup.display()
    );
    if let Some(relaunch) = &args.relaunch {
        match launch(relaunch) {
            Ok(()) => message.push_str(", программа перезапущена"),
            // The update itself succeeded; failing to restart is worth
            // saying, not worth undoing a good installation for.
            Err(error) => message.push_str(&format!(", но перезапустить не удалось: {error}")),
        }
    }
    Ok(message)
}

pub fn spawn_detached(path: &Path) -> std::io::Result<()> {
    std::process::Command::new(path).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn args(list: &[&str]) -> Result<Args> {
        parse_args(list.iter().map(|s| OsString::from(*s)))
    }

    #[test]
    fn the_three_directories_are_required() {
        let error = args(&["--current", "a", "--staged", "b"]).unwrap_err();
        assert!(error.to_string().contains("--backup"), "{error}");
    }

    #[test]
    fn an_unknown_argument_is_refused_rather_than_ignored() {
        let error = args(&[
            "--current",
            "a",
            "--staged",
            "b",
            "--backup",
            "c",
            "--forse",
            "d",
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("неизвестный аргумент"),
            "{error}"
        );
    }

    #[test]
    fn the_waiting_budget_has_bounds() {
        let parsed = args(&[
            "--current",
            "a",
            "--staged",
            "b",
            "--backup",
            "c",
            "--attempts",
            "999999",
            "--delay-ms",
            "0",
        ])
        .unwrap();
        assert_eq!(parsed.attempts, 3600);
        assert_eq!(parsed.delay, Duration::from_millis(10));
    }

    /// The helper installs a directory and may start what is in it. It must
    /// not become a way to start anything else.
    #[test]
    fn relaunch_outside_the_installed_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("SciWhisper")).unwrap();
        fs::create_dir_all(dir.path().join("staging")).unwrap();
        fs::write(dir.path().join("staging/x"), b"x").unwrap();
        for evil in ["/bin/sh", "SciWhisper/../../bin/sh", "elsewhere/thing"] {
            let parsed = Args {
                current: dir.path().join("SciWhisper"),
                staged: dir.path().join("staging"),
                backup: dir.path().join("backup"),
                relaunch: Some(PathBuf::from(evil)),
                attempts: 1,
                delay: Duration::from_millis(1),
            };
            let mut launched = None;
            let error = run(&parsed, &mut |_| {}, &mut |path| {
                launched = Some(path.to_path_buf());
                Ok(())
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("вне устанавливаемого"),
                "{evil}: {error}"
            );
            assert!(launched.is_none(), "{evil} was started anyway");
            // And the refusal happened before anything moved.
            assert!(dir.path().join("SciWhisper").exists());
        }
    }

    #[test]
    fn a_successful_run_swaps_and_relaunches() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("SciWhisper")).unwrap();
        fs::write(dir.path().join("SciWhisper/marker"), b"old").unwrap();
        fs::create_dir_all(dir.path().join("staging")).unwrap();
        fs::write(dir.path().join("staging/marker"), b"new").unwrap();

        let exe = dir
            .path()
            .join("SciWhisper")
            .join(install::executable_name());
        let parsed = Args {
            current: dir.path().join("SciWhisper"),
            staged: dir.path().join("staging"),
            backup: dir.path().join("backup"),
            relaunch: Some(exe.clone()),
            attempts: 3,
            delay: Duration::from_millis(1),
        };
        let mut launched = None;
        let message = run(&parsed, &mut |_| {}, &mut |path| {
            launched = Some(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(launched, Some(exe));
        assert!(message.contains("перезапущена"), "{message}");
        assert_eq!(
            fs::read_to_string(dir.path().join("SciWhisper/marker")).unwrap(),
            "new"
        );
    }

    /// A restart that fails is worth a sentence, not worth undoing a good
    /// installation.
    #[test]
    fn a_failed_relaunch_does_not_undo_a_good_installation() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("SciWhisper")).unwrap();
        fs::create_dir_all(dir.path().join("staging")).unwrap();
        fs::write(dir.path().join("staging/marker"), b"new").unwrap();
        let parsed = Args {
            current: dir.path().join("SciWhisper"),
            staged: dir.path().join("staging"),
            backup: dir.path().join("backup"),
            relaunch: Some(
                dir.path()
                    .join("SciWhisper")
                    .join(install::executable_name()),
            ),
            attempts: 2,
            delay: Duration::from_millis(1),
        };
        let message = run(&parsed, &mut |_| {}, &mut |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "нет такого файла",
            ))
        })
        .unwrap();
        assert!(message.contains("перезапустить не удалось"), "{message}");
        assert_eq!(
            fs::read_to_string(dir.path().join("SciWhisper/marker")).unwrap(),
            "new"
        );
    }
}

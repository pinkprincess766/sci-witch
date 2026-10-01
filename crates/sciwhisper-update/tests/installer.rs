//! The installer script says things that the updater depends on, and the two
//! files cannot see each other. These tests are the only link.
//!
//! Nothing here proves the installer works — that needs a real Windows
//! machine, and the acceptance protocol is unfilled. What it does prove is
//! that the decisions the rest of the design rests on have not been quietly
//! reversed by an edit to a script nobody runs locally.

use std::path::PathBuf;

fn script() -> String {
    let path: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows/SciWhisper.iss");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// An install into Program Files would make every update need an elevation
/// prompt, and a program that asks for administrator rights routinely is a
/// program whose prompts stop being read.
#[test]
fn the_install_is_per_user_so_updates_need_no_elevation() {
    let text = script();
    assert!(
        text.contains("PrivilegesRequired=lowest"),
        "the installer must not demand administrator rights"
    );
    assert!(
        text.contains(r"DefaultDirName={localappdata}\Programs\{#AppName}"),
        "the default directory must be one the updater can rename without elevation"
    );
}

/// Auto-start is offered, never assumed. This is the same promise the
/// application keeps about update checks.
#[test]
fn nothing_starts_by_itself_unless_the_user_ticks_it() {
    let text = script();
    let startup = text
        .lines()
        .find(|line| line.starts_with("Name: \"startup\""))
        .expect("the startup task must exist to be offered at all");
    assert!(
        startup.contains("Flags: unchecked"),
        "auto-start must be offered unchecked: {startup}"
    );
    let desktop = text
        .lines()
        .find(|line| line.starts_with("Name: \"desktopicon\""))
        .expect("desktop icon task");
    assert!(desktop.contains("Flags: unchecked"), "{desktop}");
}

/// An uninstaller that deletes a user's settings has decided something that
/// was not its to decide.
#[test]
fn uninstalling_does_not_remove_the_users_own_files() {
    let text = script();
    let deletions: Vec<&str> = text
        .lines()
        .filter(|line| line.trim_start().starts_with("Type: "))
        .collect();
    assert!(!deletions.is_empty(), "the delete list must be explicit");
    for line in &deletions {
        assert!(
            line.contains("{app}"),
            "the uninstaller may only remove things under the install directory: {line}"
        );
        for user_area in ["{userappdata}", "{userdocs}", "{localappdata}\\SciWhisper"] {
            assert!(!line.contains(user_area), "{line}");
        }
    }
}

/// The installer ships the same tree as the portable archive, so the file
/// list has one source of truth rather than two that drift.
#[test]
fn the_installer_ships_whatever_the_bundle_contains() {
    let text = script();
    assert!(
        text.contains(r#"Source: "{#SourceDir}\*""#),
        "the installer must take the whole assembled bundle, not a second hand-written list"
    );
}

/// The version is passed in by the release workflow. A script that carried
/// its own version would quietly ship the wrong one the first time somebody
/// forgot to bump it.
#[test]
fn the_version_comes_from_the_build_not_from_the_script() {
    let text = script();
    assert!(
        text.contains("#ifndef AppVersion"),
        "AppVersion must be overridable"
    );
    assert!(
        text.contains("AppVersion={#AppVersion}"),
        "the setup section must use the passed-in version"
    );
    assert!(
        text.contains("OutputBaseFilename=SciWhisper-{#AppVersion}-Windows-x64-Setup"),
        "the produced file must name the version it carries"
    );
}

//! Target profiles: what to insert, and where.
//!
//! The same formula belongs in three different shapes depending on what is
//! in front of the cursor. Word takes native OMML, a LaTeX editor takes
//! `\ce{H2SO4}`, and an ordinary text field takes `H₂SO₄`. Until now those
//! three answers were three `if`s in the insertion path, matching on
//! substrings baked into the binary: a user whose editor was not on the list
//! had no way to say so.
//!
//! A profile is that decision written down — which applications it is for,
//! what format they get, and whether the ordinary words around the formula
//! survive. The list lives in the configuration file, so adding an editor is
//! an edit, not a release.
//!
//! Order matters and is the user's: the first profile whose patterns match
//! wins. A profile with no patterns is the fallback and is used when nothing
//! else matched, wherever it appears in the list.

use serde::{Deserialize, Serialize};

use crate::config::OutputMode;
use crate::front::FrontApp;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Shown in menus and messages; not used for matching.
    pub name: String,
    /// Case-insensitive substrings tested against the frontmost
    /// application's window name and executable. Empty means "the fallback".
    #[serde(default)]
    pub r#match: Vec<String>,
    /// `unicode` | `latex` | `word`. `auto` is not allowed here: a profile
    /// that says "decide automatically" is what profiles replace.
    pub output: String,
    /// Optional override of the dictation mode for this target. `None`
    /// keeps whatever the user chose globally, which is the honest default:
    /// a profile should not silently start deleting words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictation: Option<String>,
}

impl Profile {
    /// `None` when the profile names a format that cannot be used here:
    /// `auto` would ask the profile system to consult itself, and an
    /// unknown word is a typo the user should see rather than a silent
    /// fallback to plain text.
    pub fn output_mode(&self) -> Option<OutputMode> {
        match OutputMode::try_parse(&self.output) {
            Ok(OutputMode::Auto) | Err(_) => None,
            Ok(other) => Some(other),
        }
    }

    fn is_fallback(&self) -> bool {
        self.r#match.iter().all(|pattern| pattern.trim().is_empty())
    }

    fn matches(&self, haystack: &str) -> bool {
        self.r#match
            .iter()
            .filter(|pattern| !pattern.trim().is_empty())
            .any(|pattern| haystack.contains(&pattern.to_lowercase()))
    }
}

/// The list shipped when the configuration file says nothing. These are the
/// three rules that used to be hard-coded, now visible and editable.
pub fn defaults() -> Vec<Profile> {
    vec![
        Profile {
            name: "Word".into(),
            r#match: vec!["winword".into(), "microsoft word".into(), "word".into()],
            output: "word".into(),
            dictation: None,
        },
        Profile {
            name: "LaTeX".into(),
            r#match: vec![
                "overleaf".into(),
                "texstudio".into(),
                "texshop".into(),
                "texmaker".into(),
                "lyx".into(),
                ".tex".into(),
            ],
            output: "latex".into(),
            dictation: None,
        },
        Profile {
            name: "Обычное поле".into(),
            r#match: Vec::new(),
            output: "unicode".into(),
            dictation: None,
        },
    ]
}

/// Picks the profile for `front`, or `None` when the list has no fallback
/// and nothing matched — in which case the caller keeps its own default
/// rather than inventing one.
pub fn select<'a>(profiles: &'a [Profile], front: Option<&FrontApp>) -> Option<&'a Profile> {
    let haystack = front
        .map(|app| format!("{} {}", app.name, app.exe).to_lowercase())
        .unwrap_or_default();
    profiles
        .iter()
        .find(|profile| !profile.is_fallback() && profile.matches(&haystack))
        .or_else(|| profiles.iter().find(|profile| profile.is_fallback()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, exe: &str) -> FrontApp {
        FrontApp {
            name: name.into(),
            exe: exe.into(),
        }
    }

    fn named<'a>(profiles: &'a [Profile], front: Option<&FrontApp>) -> Option<&'a str> {
        select(profiles, front).map(|profile| profile.name.as_str())
    }

    #[test]
    fn the_defaults_reproduce_the_rules_they_replaced() {
        let profiles = defaults();
        assert_eq!(
            named(&profiles, Some(&app("Документ — Word", "WINWORD.EXE"))),
            Some("Word")
        );
        assert_eq!(
            named(&profiles, Some(&app("Overleaf", "chrome.exe"))),
            Some("LaTeX")
        );
        assert_eq!(
            named(&profiles, Some(&app("thesis.tex — TeXstudio", "texstudio"))),
            Some("LaTeX")
        );
        assert_eq!(
            named(&profiles, Some(&app("Блокнот", "notepad.exe"))),
            Some("Обычное поле")
        );
        // Nothing in front at all still lands on the fallback rather than
        // leaving the caller without an answer.
        assert_eq!(named(&profiles, None), Some("Обычное поле"));
    }

    #[test]
    fn matching_ignores_case_on_both_sides() {
        let profiles = vec![Profile {
            name: "Мой редактор".into(),
            r#match: vec!["MyEditor".into()],
            output: "latex".into(),
            dictation: None,
        }];
        assert_eq!(
            named(&profiles, Some(&app("myeditor", "MYEDITOR.EXE"))),
            Some("Мой редактор")
        );
    }

    /// The user's order is the priority. A list that puts LaTeX first must
    /// send a `.tex` file open in Word to LaTeX.
    #[test]
    fn the_first_matching_profile_wins() {
        let mut profiles = defaults();
        profiles.swap(0, 1);
        assert_eq!(
            named(&profiles, Some(&app("thesis.tex — Word", "WINWORD.EXE"))),
            Some("LaTeX")
        );
    }

    /// A fallback is used only when nothing matched, wherever it sits.
    #[test]
    fn a_fallback_in_the_middle_does_not_shadow_later_profiles() {
        let profiles = vec![
            Profile {
                name: "Запасной".into(),
                r#match: vec![String::new()],
                output: "unicode".into(),
                dictation: None,
            },
            Profile {
                name: "Word".into(),
                r#match: vec!["winword".into()],
                output: "word".into(),
                dictation: None,
            },
        ];
        assert_eq!(
            named(&profiles, Some(&app("Word", "WINWORD.EXE"))),
            Some("Word")
        );
        assert_eq!(
            named(&profiles, Some(&app("Блокнот", "notepad"))),
            Some("Запасной")
        );
    }

    #[test]
    fn without_a_fallback_an_unmatched_app_gets_no_profile() {
        let profiles = vec![Profile {
            name: "Word".into(),
            r#match: vec!["winword".into()],
            output: "word".into(),
            dictation: None,
        }];
        assert_eq!(named(&profiles, Some(&app("Блокнот", "notepad"))), None);
    }

    /// `auto` inside a profile would ask the profile system to consult
    /// itself, so it is refused rather than silently treated as unicode.
    #[test]
    fn a_profile_cannot_defer_back_to_automatic_choice() {
        let profile = Profile {
            name: "Плохой".into(),
            r#match: vec!["x".into()],
            output: "auto".into(),
            dictation: None,
        };
        assert_eq!(profile.output_mode(), None);
        let unknown = Profile {
            output: "yaml".into(),
            ..profile
        };
        assert_eq!(unknown.output_mode(), None);
    }
}

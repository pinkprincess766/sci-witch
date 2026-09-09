//! Insert compiled notation into the active app.

use std::thread;
use std::time::Duration;

use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};

use sciwhisper_asr::PipelineResult;

use crate::clipboard::{self, Snapshot};
use crate::config::OutputMode;
use crate::error::{Error, Result};
use crate::front::{self, FrontApp};
use crate::profile::{self, Profile};

pub struct InsertRequest<'a> {
    pub result: &'a PipelineResult,
    pub mode: OutputMode,
    /// Consulted only when `mode` is `Auto`. Empty is legal and means "no
    /// profile matched": the caller then gets Unicode, which is the format
    /// every text field understands.
    pub profiles: &'a [Profile],
}

pub struct InsertOutcome {
    pub method: &'static str,
    pub restored: bool,
    pub front: Option<FrontApp>,
    pub payload: String,
}

pub fn insert(req: InsertRequest<'_>) -> Result<InsertOutcome> {
    let front = front::frontmost();
    let mode = resolve_mode(req.mode, req.profiles, front.as_ref());

    let payload = payload_for_mode(req.result, mode);

    if req.result.transcript.no_speech || payload.trim().is_empty() {
        return Err(Error::Message("nothing to insert".into()));
    }

    #[cfg(windows)]
    if req.result.interpretation.confidence > 0.0
        && mode == OutputMode::Word
        && front.as_ref().map(|f| f.is_word()).unwrap_or(false)
    {
        match crate::word_win::insert_omml(&req.result.omml, &req.result.interpretation.ast) {
            Ok(()) => {
                return Ok(InsertOutcome {
                    method: "word-omml",
                    restored: false,
                    front,
                    payload: req.result.unicode.clone(),
                });
            }
            Err(e) => {
                eprintln!("Word native insert failed ({e}); falling back to clipboard");
            }
        }
    }

    let snap = clipboard::snapshot(payload.clone())?;
    clipboard::set_text(&payload)?;
    thread::sleep(Duration::from_millis(40));
    let pasted = send_paste();
    thread::sleep(Duration::from_millis(180));
    // Only worth putting the clipboard back if the paste actually happened;
    // if it did not, the payload is what the user still needs there.
    let restoration = if pasted {
        clipboard::restore_if_ours(&snap)
    } else {
        clipboard::Restore::NotOurs
    };
    if restoration.warrants_warning() {
        // A file selection or an application's private format cannot be
        // held, and the paste destroyed it. Saying so is the least this can
        // do; staying quiet would let the loss look like the user's own
        // mistake.
        notify(
            "SciWhisper",
            "Формула вставлена, но прежнее содержимое буфера обмена сохранить не удалось: там было не текстовое и не графическое содержимое.",
        );
    }
    let restored = restoration == clipboard::Restore::Restored;
    Ok(InsertOutcome {
        method: if req.result.interpretation.confidence <= 0.0 {
            if pasted {
                "raw-text-paste"
            } else {
                "raw-text-clipboard"
            }
        } else if pasted {
            "clipboard-paste"
        } else {
            "clipboard-left"
        },
        restored,
        front,
        payload,
    })
}

/// Which format to insert. An explicit choice is obeyed as given; `Auto`
/// asks the profile list, and falls back to Unicode when nothing matches —
/// the format every text field accepts, and the one that loses the least if
/// the guess is wrong.
pub fn resolve_mode(
    requested: OutputMode,
    profiles: &[Profile],
    front: Option<&FrontApp>,
) -> OutputMode {
    if requested != OutputMode::Auto {
        return requested;
    }
    profile::select(profiles, front)
        .and_then(|profile| profile.output_mode())
        .unwrap_or(OutputMode::Unicode)
}

fn payload_for_mode(result: &PipelineResult, mode: OutputMode) -> String {
    match mode {
        OutputMode::Latex => result.latex.clone(),
        OutputMode::Word | OutputMode::Unicode | OutputMode::Auto => result.unicode.clone(),
    }
}

fn send_paste() -> bool {
    if !crate::permissions::accessibility_trusted() {
        return false;
    }
    chord_unicode('v')
}

pub fn send_undo() -> bool {
    chord_unicode('z')
}

fn chord_unicode(ch: char) -> bool {
    let Ok(mut enigo) = Enigo::new(&Settings::default()) else {
        return false;
    };
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    let key = shortcut_key(ch);
    let pressed = enigo.key(modifier, Press).is_ok();
    let clicked = enigo.key(key, Click).is_ok();
    let released = enigo.key(modifier, Release).is_ok();
    pressed && clicked && released
}

fn shortcut_key(ch: char) -> Key {
    #[cfg(target_os = "macos")]
    {
        // macOS shortcuts are physical keys, not produced Unicode characters.
        // Looking up Latin `v` in a Russian layout falls back to keycode 0 (A),
        // turning paste into Cmd+A. These virtual keycodes are layout-independent.
        match ch.to_ascii_lowercase() {
            'v' => Key::Other(9),
            'z' => Key::Other(6),
            _ => Key::Unicode(ch),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Key::Unicode(ch)
    }
}

/// Last insert is undoable only while the same app is still frontmost.
pub struct LastInsert {
    pub front: Option<FrontApp>,
    pub payload: String,
    pub raw: String,
}

impl LastInsert {
    pub fn can_undo(&self) -> bool {
        match (&self.front, front::frontmost()) {
            (Some(a), Some(b)) => a == &b,
            _ => false,
        }
    }
}

pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        // notify-rust's legacy bridge calls NSAppleScript in-process and can
        // abort on macOS 26; shelling out to osascript runs the same
        // "display notification" command out-of-process instead, so a
        // banner still reaches the user even though this crashes the other way.
        let script = format!(
            "display notification {} with title {}",
            applescript_string(body),
            applescript_string(title)
        );
        let shown = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status()
            .is_ok_and(|status| status.success());
        if !shown {
            eprintln!("{title}: {body}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[allow(dead_code)]
fn _use_snapshot(_: &Snapshot) {}

#[cfg(test)]
mod tests {
    use super::*;
    use sciwhisper_asr::compile_transcript;
    use sciwhisper_asr::engine::Transcript;
    use sciwhisper_core::Domain;

    #[test]
    fn unparsed_whisper_text_falls_back_to_raw_text() {
        let result = compile_transcript(
            Transcript {
                text: "обычная неразобранная фраза".into(),
                language: Some("ru".into()),
                segments: vec![],
                no_speech: false,
            },
            Domain::Chemistry,
        );
        assert_eq!(result.interpretation.confidence, 0.0);
        assert_eq!(
            payload_for_mode(&result, OutputMode::Latex),
            "обычная неразобранная фраза"
        );
    }

    #[test]
    fn inline_formula_is_inserted_with_surrounding_prose() {
        let result = compile_transcript(
            Transcript {
                text: "пример гидроксида железа три в тексте".into(),
                language: Some("ru".into()),
                segments: vec![],
                no_speech: false,
            },
            Domain::Auto,
        );
        assert_eq!(
            payload_for_mode(&result, OutputMode::Unicode),
            "пример Fe(OH)₃ в тексте"
        );
    }

    fn app(name: &str, exe: &str) -> FrontApp {
        FrontApp {
            name: name.into(),
            exe: exe.into(),
        }
    }

    /// A user who picked LaTeX in the tray did not mean "unless a profile
    /// disagrees".
    #[test]
    fn an_explicit_format_is_never_overridden_by_a_profile() {
        let profiles = profile::defaults();
        let word = app("Документ — Word", "WINWORD.EXE");
        for chosen in [OutputMode::Unicode, OutputMode::Latex, OutputMode::Word] {
            assert_eq!(resolve_mode(chosen, &profiles, Some(&word)), chosen);
        }
    }

    #[test]
    fn automatic_choice_follows_the_profile_list() {
        let profiles = profile::defaults();
        assert_eq!(
            resolve_mode(
                OutputMode::Auto,
                &profiles,
                Some(&app("Документ — Word", "WINWORD.EXE"))
            ),
            OutputMode::Word
        );
        assert_eq!(
            resolve_mode(
                OutputMode::Auto,
                &profiles,
                Some(&app("Overleaf", "chrome.exe"))
            ),
            OutputMode::Latex
        );
        assert_eq!(
            resolve_mode(
                OutputMode::Auto,
                &profiles,
                Some(&app("Блокнот", "notepad.exe"))
            ),
            OutputMode::Unicode
        );
    }

    /// An empty or unusable profile list must not stop insertion. Unicode is
    /// the format every text field accepts, so it is what a failed guess
    /// costs the least.
    #[test]
    fn with_no_usable_profile_the_safest_format_is_used() {
        assert_eq!(
            resolve_mode(OutputMode::Auto, &[], Some(&app("Word", "WINWORD.EXE"))),
            OutputMode::Unicode
        );
        let broken = vec![Profile {
            name: "Опечатка".into(),
            r#match: vec!["winword".into()],
            output: "wodr".into(),
            dictation: None,
        }];
        assert_eq!(
            resolve_mode(OutputMode::Auto, &broken, Some(&app("Word", "WINWORD.EXE"))),
            OutputMode::Unicode
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_shortcuts_use_layout_independent_keycodes() {
        assert_eq!(shortcut_key('v'), Key::Other(9));
        assert_eq!(shortcut_key('z'), Key::Other(6));
    }
}

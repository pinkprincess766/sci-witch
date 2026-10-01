//! Snapshot and restore of the user's clipboard.
//!
//! Insertion works by putting the compiled formula on the clipboard and
//! sending a paste. That borrows something that belongs to the user, so it
//! has to be given back — and giving back only what we happen to understand
//! is not giving it back.
//!
//! Three outcomes, and the difference between them matters:
//!
//! * **restored** — what was there is back;
//! * **not ours** — somebody wrote to the clipboard after we did, so putting
//!   the old contents back would destroy their copy instead of ours;
//! * **could not be preserved** — there was something there we cannot hold
//!   (a file selection, RTF, an application's private format). The paste
//!   still has to happen, so it is lost either way; what must not happen is
//!   losing it *silently*.
//!
//! Text and images are held faithfully. Everything else is detected and
//! reported rather than quietly overwritten — on Windows precisely, by
//! asking whether the clipboard offers any format at all; elsewhere
//! best-effort, because `arboard` cannot tell an empty clipboard from one
//! holding a format it does not read.

use crate::error::{Error, Result};

/// What was on the clipboard before we borrowed it.
#[derive(Clone, PartialEq)]
pub enum Held {
    Text(String),
    Image(OwnedImage),
    /// Something was there that this program cannot hold.
    Foreign,
    Empty,
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Held::Text(text) => write!(f, "Text({} символов)", text.chars().count()),
            Held::Image(image) => write!(f, "Image({}×{})", image.width, image.height),
            Held::Foreign => f.write_str("Foreign"),
            Held::Empty => f.write_str("Empty"),
        }
    }
}

/// An image copied out of the clipboard. Owned, because the clipboard is
/// about to be overwritten and a borrowed view of it would dangle.
#[derive(Clone, PartialEq)]
pub struct OwnedImage {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

pub struct Snapshot {
    pub before: Held,
    pub sequence: Option<u32>,
    pub our_payload: String,
}

/// What restoring did, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restore {
    Restored,
    /// The clipboard no longer holds what we put there, so it is not ours to
    /// change back.
    NotOurs,
    /// There was nothing to put back.
    NothingToRestore,
    /// Something was there that this program cannot hold. The caller is
    /// expected to tell the user rather than swallow this.
    CouldNotPreserve,
}

/// Whether the outcome is one the user should hear about.
impl Restore {
    pub fn warrants_warning(self) -> bool {
        matches!(self, Restore::CouldNotPreserve)
    }
}

pub fn snapshot(our_payload: String) -> Result<Snapshot> {
    Ok(Snapshot {
        before: read_any(),
        sequence: clipboard_sequence(),
        our_payload,
    })
}

/// Reads whatever of the clipboard this program can hold.
fn read_any() -> Held {
    if let Ok(text) = read_text() {
        return Held::Text(text);
    }
    if let Some(image) = read_image() {
        return Held::Image(image);
    }
    if clipboard_has_any_format() {
        return Held::Foreign;
    }
    Held::Empty
}

pub fn set_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| Error::Message(e.to_string()))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| Error::Message(e.to_string()))
}

pub fn read_text() -> Result<String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| Error::Message(e.to_string()))?;
    clipboard
        .get_text()
        .map_err(|e| Error::Message(e.to_string()))
}

fn read_image() -> Option<OwnedImage> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    Some(OwnedImage {
        width: image.width,
        height: image.height,
        bytes: image.bytes.into_owned(),
    })
}

fn write_image(image: &OwnedImage) -> bool {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return false;
    };
    clipboard
        .set_image(arboard::ImageData {
            width: image.width,
            height: image.height,
            bytes: std::borrow::Cow::Borrowed(&image.bytes),
        })
        .is_ok()
}

/// How far the Windows clipboard sequence number may advance and the
/// clipboard still be ours: our own write, plus the paste. Anything beyond
/// that is somebody else's copy, and putting the old contents back would
/// destroy it.
const OUR_SEQUENCE_BUDGET: u32 = 4;

/// The decision, separated from the clipboard so it can be tested.
///
/// `current_text` is what the clipboard holds now, as far as it can be read;
/// `None` means it holds something that is not text, which by itself already
/// proves the contents are no longer ours — we wrote text.
pub fn decide(
    snapshot: &Snapshot,
    current_text: Option<&str>,
    sequence_now: Option<u32>,
) -> Restore {
    if current_text != Some(snapshot.our_payload.as_str()) {
        return Restore::NotOurs;
    }
    if let (Some(before), Some(now)) = (snapshot.sequence, sequence_now) {
        if now.saturating_sub(before) > OUR_SEQUENCE_BUDGET {
            return Restore::NotOurs;
        }
    }
    match &snapshot.before {
        Held::Text(_) | Held::Image(_) => Restore::Restored,
        Held::Foreign => Restore::CouldNotPreserve,
        Held::Empty => Restore::NothingToRestore,
    }
}

/// Puts the user's clipboard back if it is still ours to put back.
pub fn restore_if_ours(snapshot: &Snapshot) -> Restore {
    let current = read_text().ok();
    let decision = decide(snapshot, current.as_deref(), clipboard_sequence());
    if decision != Restore::Restored {
        return decision;
    }
    let put_back = match &snapshot.before {
        Held::Text(text) => set_text(text).is_ok(),
        Held::Image(image) => write_image(image),
        Held::Foreign | Held::Empty => false,
    };
    if put_back {
        Restore::Restored
    } else {
        Restore::CouldNotPreserve
    }
}

#[cfg(windows)]
fn clipboard_sequence() -> Option<u32> {
    unsafe { Some(windows::Win32::System::DataExchange::GetClipboardSequenceNumber()) }
}

#[cfg(not(windows))]
fn clipboard_sequence() -> Option<u32> {
    None
}

/// Whether the clipboard offers anything at all.
///
/// Only Windows can answer this honestly: `CountClipboardFormats` reports
/// what is on offer without opening it. Elsewhere `arboard` gives no way to
/// distinguish an empty clipboard from one holding a format it does not
/// read, so the answer is "no" and a file selection is lost without a
/// warning — a limitation, written down rather than papered over.
#[cfg(windows)]
fn clipboard_has_any_format() -> bool {
    unsafe { windows::Win32::System::DataExchange::CountClipboardFormats() > 0 }
}

#[cfg(not(windows))]
fn clipboard_has_any_format() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_of(before: Held, sequence: Option<u32>) -> Snapshot {
        Snapshot {
            before,
            sequence,
            our_payload: "H₂SO₄".into(),
        }
    }

    #[test]
    fn text_that_was_there_is_put_back() {
        let snapshot = snapshot_of(Held::Text("важная заметка".into()), Some(10));
        assert_eq!(
            decide(&snapshot, Some("H₂SO₄"), Some(12)),
            Restore::Restored
        );
    }

    #[test]
    fn an_image_is_held_and_put_back_like_text() {
        let image = Held::Image(OwnedImage {
            width: 2,
            height: 1,
            bytes: vec![0, 0, 0, 255, 255, 255, 255, 255],
        });
        assert_eq!(
            decide(&snapshot_of(image, Some(1)), Some("H₂SO₄"), Some(2)),
            Restore::Restored
        );
    }

    /// The case this whole module exists to get right: the user copied
    /// something while the paste was in flight, and their copy must win.
    #[test]
    fn a_clipboard_someone_else_wrote_to_is_left_alone() {
        let snapshot = snapshot_of(Held::Text("важная заметка".into()), Some(10));
        assert_eq!(
            decide(&snapshot, Some("что-то другое"), Some(12)),
            Restore::NotOurs
        );
        // Same payload, but the sequence says several writes happened: the
        // user may have copied our text and then something else.
        assert_eq!(
            decide(&snapshot, Some("H₂SO₄"), Some(10 + OUR_SEQUENCE_BUDGET + 1)),
            Restore::NotOurs
        );
        // Non-text on the clipboard proves it is not what we wrote.
        assert_eq!(decide(&snapshot, None, Some(11)), Restore::NotOurs);
    }

    /// Files, RTF, an application's private format: we cannot hold it, the
    /// paste destroys it, and the user has to be told. Reporting this as a
    /// plain "not restored" is what made the old behaviour dishonest.
    #[test]
    fn contents_that_cannot_be_held_are_reported_rather_than_ignored() {
        let snapshot = snapshot_of(Held::Foreign, Some(3));
        let decision = decide(&snapshot, Some("H₂SO₄"), Some(4));
        assert_eq!(decision, Restore::CouldNotPreserve);
        assert!(decision.warrants_warning());
    }

    #[test]
    fn an_empty_clipboard_needs_no_warning() {
        let decision = decide(&snapshot_of(Held::Empty, Some(3)), Some("H₂SO₄"), Some(4));
        assert_eq!(decision, Restore::NothingToRestore);
        assert!(!decision.warrants_warning());
    }

    /// Without a sequence number — every platform but Windows — the text
    /// comparison is the only guard, and it still has to work.
    #[test]
    fn without_a_sequence_number_the_text_alone_decides() {
        let snapshot = snapshot_of(Held::Text("заметка".into()), None);
        assert_eq!(decide(&snapshot, Some("H₂SO₄"), None), Restore::Restored);
        assert_eq!(decide(&snapshot, Some("чужое"), None), Restore::NotOurs);
    }

    #[test]
    fn the_debug_view_does_not_print_the_users_clipboard() {
        let text = format!("{:?}", Held::Text("секретный пароль".into()));
        assert!(!text.contains("пароль"), "{text}");
        assert!(text.contains("16 символов"), "{text}");
    }
}

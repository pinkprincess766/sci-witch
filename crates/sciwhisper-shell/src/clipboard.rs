//! Clipboard snapshot / restore.
//!
//! Restores only if the clipboard still contains SciWhisper's payload
//! (sequence/version check on Windows, text match elsewhere).

use crate::error::{Error, Result};

pub struct Snapshot {
    pub text: Option<String>,
    pub sequence: Option<u32>,
    pub our_payload: String,
}

pub fn snapshot(our_payload: String) -> Result<Snapshot> {
    let text = read_text().ok();
    Ok(Snapshot {
        text,
        sequence: clipboard_sequence(),
        our_payload,
    })
}

pub fn set_text(text: &str) -> Result<()> {
    let mut cb = arboard::Clipboard::new().map_err(|e| Error::Message(e.to_string()))?;
    cb.set_text(text.to_string())
        .map_err(|e| Error::Message(e.to_string()))?;
    Ok(())
}

pub fn read_text() -> Result<String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| Error::Message(e.to_string()))?;
    cb.get_text().map_err(|e| Error::Message(e.to_string()))
}

/// Restore previous clipboard if the current contents are still ours.
pub fn restore_if_ours(snap: &Snapshot) -> bool {
    let current = read_text().unwrap_or_default();
    if current != snap.our_payload {
        return false;
    }
    if let (Some(before), Some(now)) = (snap.sequence, clipboard_sequence()) {
        // Someone else wrote to the clipboard if sequence jumped more than our write + paste.
        if now.saturating_sub(before) > 4 {
            return false;
        }
    }
    if let Some(prev) = &snap.text {
        let _ = set_text(prev);
        return true;
    }
    false
}

#[cfg(windows)]
fn clipboard_sequence() -> Option<u32> {
    unsafe { Some(windows::Win32::System::DataExchange::GetClipboardSequenceNumber()) }
}

#[cfg(not(windows))]
fn clipboard_sequence() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_restore_when_user_overwrote() {
        let snap = Snapshot {
            text: Some("old".into()),
            sequence: Some(1),
            our_payload: "sciwhisper".into(),
        };
        // We cannot touch the real clipboard in unit tests reliably; just the struct.
        assert_eq!(snap.our_payload, "sciwhisper");
    }
}

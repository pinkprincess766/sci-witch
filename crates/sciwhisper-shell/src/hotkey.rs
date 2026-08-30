use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Key {
    Alt,
    AltGr,
    ControlLeft,
    ControlRight,
    Escape,
    MetaLeft,
    MetaRight,
    Return,
    ShiftLeft,
    ShiftRight,
    Space,
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub trigger: Key,
}

impl Combo {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut meta = false;
        let mut trigger = None;
        for part in s.split('+').map(|p| p.trim()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "controlleft" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" | "opt" => alt = true,
                "meta" | "cmd" | "win" | "super" => meta = true,
                "space" => trigger = Some(Key::Space),
                "esc" | "escape" => trigger = Some(Key::Escape),
                "enter" | "return" => trigger = Some(Key::Return),
                other if other.len() == 1 => {
                    let c = other.chars().next().unwrap();
                    trigger = Some(letter_key(c));
                }
                other => return Err(format!("unknown hotkey token '{other}'")),
            }
        }
        let trigger = trigger.ok_or_else(|| format!("hotkey '{s}' missing a key"))?;
        Ok(Self {
            ctrl,
            shift,
            alt,
            meta,
            trigger,
        })
    }

    pub fn modifiers_held(&self, down: &HashSet<Key>) -> bool {
        let ctrl_ok =
            !self.ctrl || down.contains(&Key::ControlLeft) || down.contains(&Key::ControlRight);
        let shift_ok =
            !self.shift || down.contains(&Key::ShiftLeft) || down.contains(&Key::ShiftRight);
        let alt_ok = !self.alt || down.contains(&Key::Alt) || down.contains(&Key::AltGr);
        let meta_ok = !self.meta || down.contains(&Key::MetaLeft) || down.contains(&Key::MetaRight);
        ctrl_ok && shift_ok && alt_ok && meta_ok
    }

    pub fn trigger_down(&self, down: &HashSet<Key>) -> bool {
        self.modifiers_held(down) && down.contains(&self.trigger)
    }
}

fn letter_key(c: char) -> Key {
    match c.to_ascii_uppercase() {
        'A' => Key::KeyA,
        'B' => Key::KeyB,
        'C' => Key::KeyC,
        'D' => Key::KeyD,
        'E' => Key::KeyE,
        'F' => Key::KeyF,
        'G' => Key::KeyG,
        'H' => Key::KeyH,
        'I' => Key::KeyI,
        'J' => Key::KeyJ,
        'K' => Key::KeyK,
        'L' => Key::KeyL,
        'M' => Key::KeyM,
        'N' => Key::KeyN,
        'O' => Key::KeyO,
        'P' => Key::KeyP,
        'Q' => Key::KeyQ,
        'R' => Key::KeyR,
        'S' => Key::KeyS,
        'T' => Key::KeyT,
        'U' => Key::KeyU,
        'V' => Key::KeyV,
        'W' => Key::KeyW,
        'X' => Key::KeyX,
        'Y' => Key::KeyY,
        'Z' => Key::KeyZ,
        _ => Key::Space,
    }
}

pub fn is_escape(key: Key) -> bool {
    key == Key::Escape
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_ptt() {
        let c = Combo::parse("Ctrl+Shift+Space").unwrap();
        assert!(c.ctrl && c.shift);
        assert_eq!(c.trigger, Key::Space);
        let mut down = HashSet::new();
        down.insert(Key::ControlLeft);
        down.insert(Key::ShiftLeft);
        down.insert(Key::Space);
        assert!(c.trigger_down(&down));
        down.remove(&Key::Space);
        assert!(!c.trigger_down(&down));
    }
}

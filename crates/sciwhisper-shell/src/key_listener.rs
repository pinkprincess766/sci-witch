use crate::hotkey::Key;

pub enum KeyEvent {
    Press(Key),
    Release(Key),
}

#[cfg(not(target_os = "macos"))]
pub fn listen<F>(mut callback: F) -> Result<(), String>
where
    F: FnMut(KeyEvent) + Send + 'static,
{
    rdev::listen(move |event| {
        let mapped = match event.event_type {
            rdev::EventType::KeyPress(key) => map_rdev_key(key).map(KeyEvent::Press),
            rdev::EventType::KeyRelease(key) => map_rdev_key(key).map(KeyEvent::Release),
            _ => None,
        };
        if let Some(event) = mapped {
            callback(event);
        }
    })
    .map_err(|error| format!("global hotkey listener failed: {error:?}"))
}

#[cfg(not(target_os = "macos"))]
fn map_rdev_key(key: rdev::Key) -> Option<Key> {
    use rdev::Key as RdevKey;

    Some(match key {
        RdevKey::Alt => Key::Alt,
        RdevKey::AltGr => Key::AltGr,
        RdevKey::ControlLeft => Key::ControlLeft,
        RdevKey::ControlRight => Key::ControlRight,
        RdevKey::Escape => Key::Escape,
        RdevKey::MetaLeft => Key::MetaLeft,
        RdevKey::MetaRight => Key::MetaRight,
        RdevKey::Return => Key::Return,
        RdevKey::ShiftLeft => Key::ShiftLeft,
        RdevKey::ShiftRight => Key::ShiftRight,
        RdevKey::Space => Key::Space,
        RdevKey::KeyA => Key::KeyA,
        RdevKey::KeyB => Key::KeyB,
        RdevKey::KeyC => Key::KeyC,
        RdevKey::KeyD => Key::KeyD,
        RdevKey::KeyE => Key::KeyE,
        RdevKey::KeyF => Key::KeyF,
        RdevKey::KeyG => Key::KeyG,
        RdevKey::KeyH => Key::KeyH,
        RdevKey::KeyI => Key::KeyI,
        RdevKey::KeyJ => Key::KeyJ,
        RdevKey::KeyK => Key::KeyK,
        RdevKey::KeyL => Key::KeyL,
        RdevKey::KeyM => Key::KeyM,
        RdevKey::KeyN => Key::KeyN,
        RdevKey::KeyO => Key::KeyO,
        RdevKey::KeyP => Key::KeyP,
        RdevKey::KeyQ => Key::KeyQ,
        RdevKey::KeyR => Key::KeyR,
        RdevKey::KeyS => Key::KeyS,
        RdevKey::KeyT => Key::KeyT,
        RdevKey::KeyU => Key::KeyU,
        RdevKey::KeyV => Key::KeyV,
        RdevKey::KeyW => Key::KeyW,
        RdevKey::KeyX => Key::KeyX,
        RdevKey::KeyY => Key::KeyY,
        RdevKey::KeyZ => Key::KeyZ,
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use super::{Key, KeyEvent};

    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CFMachPortRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFStringRef = *const c_void;

    const KEY_DOWN: u32 = 10;
    const KEY_UP: u32 = 11;
    const FLAGS_CHANGED: u32 = 12;
    const TAP_DISABLED_BY_TIMEOUT: u32 = 0xffff_fffe;
    const TAP_DISABLED_BY_USER_INPUT: u32 = 0xffff_ffff;
    const KEYCODE_FIELD: u32 = 9;

    const SHIFT_MASK: u64 = 1 << 17;
    const CONTROL_MASK: u64 = 1 << 18;
    const OPTION_MASK: u64 = 1 << 19;
    const COMMAND_MASK: u64 = 1 << 20;

    struct Context<F> {
        callback: F,
        tap: CFMachPortRef,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: unsafe extern "C" fn(
                CGEventTapProxy,
                u32,
                CGEventRef,
                *mut c_void,
            ) -> CGEventRef,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopCommonModes: CFStringRef;
        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: CFMachPortRef,
            order: isize,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(
            run_loop: CFRunLoopRef,
            source: CFRunLoopSourceRef,
            mode: CFStringRef,
        );
        fn CFRunLoopRun();
    }

    pub fn listen<F>(callback: F) -> Result<(), String>
    where
        F: FnMut(KeyEvent) + Send + 'static,
    {
        let context = Box::new(Context {
            callback,
            tap: std::ptr::null_mut(),
        });
        let context = Box::into_raw(context);
        let event_mask = (1_u64 << KEY_DOWN) | (1_u64 << KEY_UP) | (1_u64 << FLAGS_CHANGED);

        // This listener deliberately uses only hardware key codes. Unlike rdev's macOS
        // path it never asks Text Input Services to translate a key on this worker thread;
        // macOS 26 aborts processes that make that AppKit call off the main queue.
        let tap =
            unsafe { CGEventTapCreate(0, 0, 1, event_mask, event_callback::<F>, context.cast()) };
        if tap.is_null() {
            unsafe {
                drop(Box::from_raw(context));
            }
            return Err(
                "не удалось включить глобальные клавиши; разрешите Input Monitoring для SciWhisper"
                    .into(),
            );
        }
        unsafe {
            (*context).tap = tap;
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
            if source.is_null() {
                drop(Box::from_raw(context));
                return Err("не удалось создать macOS run-loop для горячих клавиш".into());
            }
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            CFRunLoopRun();
            drop(Box::from_raw(context));
        }
        Ok(())
    }

    unsafe extern "C" fn event_callback<F>(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef
    where
        F: FnMut(KeyEvent) + Send + 'static,
    {
        let context = &mut *user_info.cast::<Context<F>>();
        if event_type == TAP_DISABLED_BY_TIMEOUT || event_type == TAP_DISABLED_BY_USER_INPUT {
            CGEventTapEnable(context.tap, true);
            return event;
        }

        let code = CGEventGetIntegerValueField(event, KEYCODE_FIELD) as u16;
        let Some(key) = key_from_code(code) else {
            return event;
        };
        let key_event = match event_type {
            KEY_DOWN => KeyEvent::Press(key),
            KEY_UP => KeyEvent::Release(key),
            FLAGS_CHANGED => {
                let pressed = CGEventGetFlags(event) & modifier_mask(key) != 0;
                if pressed {
                    KeyEvent::Press(key)
                } else {
                    KeyEvent::Release(key)
                }
            }
            _ => return event,
        };
        (context.callback)(key_event);
        event
    }

    fn modifier_mask(key: Key) -> u64 {
        match key {
            Key::ShiftLeft | Key::ShiftRight => SHIFT_MASK,
            Key::ControlLeft | Key::ControlRight => CONTROL_MASK,
            Key::Alt | Key::AltGr => OPTION_MASK,
            Key::MetaLeft | Key::MetaRight => COMMAND_MASK,
            _ => 0,
        }
    }

    fn key_from_code(code: u16) -> Option<Key> {
        Some(match code {
            0 => Key::KeyA,
            1 => Key::KeyS,
            2 => Key::KeyD,
            3 => Key::KeyF,
            4 => Key::KeyH,
            5 => Key::KeyG,
            6 => Key::KeyZ,
            7 => Key::KeyX,
            8 => Key::KeyC,
            9 => Key::KeyV,
            11 => Key::KeyB,
            12 => Key::KeyQ,
            13 => Key::KeyW,
            14 => Key::KeyE,
            15 => Key::KeyR,
            16 => Key::KeyY,
            17 => Key::KeyT,
            31 => Key::KeyO,
            32 => Key::KeyU,
            34 => Key::KeyI,
            35 => Key::KeyP,
            36 => Key::Return,
            37 => Key::KeyL,
            38 => Key::KeyJ,
            40 => Key::KeyK,
            45 => Key::KeyN,
            46 => Key::KeyM,
            49 => Key::Space,
            53 => Key::Escape,
            54 => Key::MetaRight,
            55 => Key::MetaLeft,
            56 => Key::ShiftLeft,
            58 => Key::Alt,
            59 => Key::ControlLeft,
            60 => Key::ShiftRight,
            61 => Key::AltGr,
            62 => Key::ControlRight,
            _ => return None,
        })
    }
}

#[cfg(target_os = "macos")]
pub use macos::listen;

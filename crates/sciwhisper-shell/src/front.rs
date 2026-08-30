//! Frontmost application — used for Word detection and undo safety.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontApp {
    pub name: String,
    pub exe: String,
}

impl FrontApp {
    pub fn is_word(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        let e = self.exe.to_ascii_lowercase();
        n.contains("word") || e.contains("winword") || e.contains("microsoft word")
    }

    pub fn wants_latex(&self) -> bool {
        let hay = format!("{} {}", self.name, self.exe).to_ascii_lowercase();
        hay.contains("overleaf")
            || hay.contains("texstudio")
            || hay.contains("texshop")
            || hay.contains(".tex")
            || hay.contains("vscode") && hay.contains("tex")
    }
}

pub fn frontmost() -> Option<FrontApp> {
    #[cfg(windows)]
    {
        windows_front()
    }
    #[cfg(target_os = "macos")]
    {
        macos_front()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_front() -> Option<FrontApp> {
    let out = std::process::Command::new("osascript")
        .args([
            "-e",
            r#"tell application "System Events" to get {name, unix id} of first application process whose frontmost is true"#,
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let name = s.split(',').next().unwrap_or(&s).trim().to_string();
    Some(FrontApp {
        exe: name.clone(),
        name,
    })
}

#[cfg(windows)]
fn windows_front() -> Option<FrontApp> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(
            proc,
            Default::default(),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let exe = std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(path.clone());
        Some(FrontApp {
            name: exe.clone(),
            exe,
        })
    }
}

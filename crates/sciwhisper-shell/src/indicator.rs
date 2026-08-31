use std::num::NonZeroU32;
use std::sync::Arc;

use softbuffer::{Context, Surface};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

const WIDTH: u32 = 5;
const HEIGHT: u32 = 24;
const MACOS_ACCENT_BLUE: u32 = 0x000a_84ff;

pub struct RecordingIndicator {
    window: Arc<Window>,
    surface: Surface<Arc<Window>, Arc<Window>>,
}

impl RecordingIndicator {
    pub fn new(event_loop: &ActiveEventLoop) -> Result<Self, String> {
        let attributes = Window::default_attributes()
            .with_title("sci-witch recording")
            .with_inner_size(PhysicalSize::new(WIDTH, HEIGHT))
            .with_resizable(false)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_active(false)
            .with_visible(false);

        #[cfg(target_os = "windows")]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true)
        };

        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("не удалось создать индикатор записи: {error}"))?,
        );
        let _ = window.set_cursor_hittest(false);
        let context = Context::new(window.clone())
            .map_err(|error| format!("не удалось подготовить индикатор записи: {error}"))?;
        let surface = Surface::new(&context, window.clone())
            .map_err(|error| format!("не удалось отрисовать индикатор записи: {error}"))?;
        let mut indicator = Self { window, surface };
        indicator.draw()?;
        Ok(indicator)
    }

    pub fn set_recording(&mut self, recording: bool) {
        if recording {
            if let Some(position) = pointer_position() {
                self.window.set_outer_position(PhysicalPosition::new(
                    position.x.saturating_add(16),
                    position.y.saturating_add(18),
                ));
            }
            let _ = self.draw();
        }
        self.window.set_visible(recording);
    }

    pub fn redraw(&mut self) {
        let _ = self.draw();
    }

    pub fn window_id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    fn draw(&mut self) -> Result<(), String> {
        let width = NonZeroU32::new(WIDTH).expect("indicator width is non-zero");
        let height = NonZeroU32::new(HEIGHT).expect("indicator height is non-zero");
        self.surface
            .resize(width, height)
            .map_err(|error| error.to_string())?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;
        buffer.fill(MACOS_ACCENT_BLUE);
        buffer.present().map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "macos")]
fn pointer_position() -> Option<PhysicalPosition<i32>> {
    use std::ffi::c_void;

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    type CGEventRef = *mut c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const c_void) -> CGEventRef;
        fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const c_void);
    }

    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event.cast_const());
        Some(PhysicalPosition::new(point.x as i32, point.y as i32))
    }
}

#[cfg(target_os = "windows")]
fn pointer_position() -> Option<PhysicalPosition<i32>> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some(PhysicalPosition::new(point.x, point.y))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pointer_position() -> Option<PhysicalPosition<i32>> {
    None
}

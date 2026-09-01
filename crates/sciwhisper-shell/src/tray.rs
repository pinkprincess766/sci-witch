use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use image::imageops::FilterType;

use crate::config::OutputMode;
use sciwhisper_core::Domain;

pub struct MenuIds {
    pub quit: MenuId,
    pub rec: MenuId,
    pub paste_last: MenuId,
    pub show_raw: MenuId,
    pub undo: MenuId,
    pub clear: MenuId,
    /// One entry per domain option; exactly one should be checked at a time.
    pub domain_checks: Vec<(CheckMenuItem, Domain)>,
    /// One entry per output option; exactly one should be checked at a time.
    pub output_checks: Vec<(CheckMenuItem, OutputMode)>,
    /// One entry per microphone option (`None` = system default), plus every
    /// device `capture::input_devices()` reported when the menu was last
    /// (re)built. Exactly one should be checked at a time.
    pub mic_checks: Vec<(CheckMenuItem, Option<String>)>,
    pub mic_refresh: MenuId,
}

pub struct Tray {
    pub icon: TrayIcon,
    pub ids: MenuIds,
    status: MenuItem,
}

impl Tray {
    pub fn current_status(&self) -> String {
        self.status.text()
    }

    /// Rebuilds the menu in place — same tray icon, fresh microphone list —
    /// so a newly plugged-in device shows up without restarting the app.
    pub fn refresh(&mut self, domain: Domain, output: OutputMode, mic: Option<&str>) {
        let status_text = self.status.text();
        let (menu, ids, status) = build_menu(domain, output, mic, &status_text);
        self.icon.set_menu(Some(Box::new(menu)));
        self.ids = ids;
        self.status = status;
    }
}

fn build_menu(
    domain: Domain,
    output: OutputMode,
    mic: Option<&str>,
    status: &str,
) -> (Menu, MenuIds, MenuItem) {
    let menu = Menu::new();
    let rec = MenuItem::new("Начать / завершить запись (Control ×2)", true, None);
    let paste_last = MenuItem::new("Повторить вставку", true, None);
    let show_raw = MenuItem::new("Скопировать исходный текст", true, None);
    let undo = MenuItem::new("Undo вставки", true, None);
    let clear = MenuItem::new("Очистить историю", true, None);
    let quit = MenuItem::new("Выход", true, None);

    let domains_menu = Submenu::new("Домен", true);
    let mut domain_checks = Vec::new();
    for (value, label) in [
        (Domain::Auto, "Auto"),
        (Domain::Chemistry, "Chemistry"),
        (Domain::Mathematics, "Mathematics"),
        (Domain::Physics, "Physics"),
        (Domain::Plain, "Plain"),
    ] {
        let item = CheckMenuItem::new(label, true, value == domain, None);
        let _ = domains_menu.append(&item);
        domain_checks.push((item, value));
    }

    let outputs_menu = Submenu::new("Формат", true);
    let mut output_checks = Vec::new();
    for (value, label) in [
        (OutputMode::Auto, "Auto"),
        (OutputMode::Unicode, "Unicode"),
        (OutputMode::Latex, "LaTeX"),
        (OutputMode::Word, "Word native"),
    ] {
        let item = CheckMenuItem::new(label, true, value == output, None);
        let _ = outputs_menu.append(&item);
        output_checks.push((item, value));
    }

    let mics_menu = Submenu::new("Микрофон", true);
    let default_item = CheckMenuItem::new("Системный по умолчанию", true, mic.is_none(), None);
    let _ = mics_menu.append(&default_item);
    let mut mic_checks = vec![(default_item, None::<String>)];
    for name in sciwhisper_asr::capture::input_devices() {
        let checked = mic == Some(name.as_str());
        let item = CheckMenuItem::new(&name, true, checked, None);
        let _ = mics_menu.append(&item);
        mic_checks.push((item, Some(name)));
    }
    let _ = mics_menu.append(&PredefinedMenuItem::separator());
    let mic_refresh = MenuItem::new("Обновить список устройств", true, None);
    let _ = mics_menu.append(&mic_refresh);

    let status = MenuItem::new(status, false, None);
    let _ = menu.append(&status);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&rec);
    let _ = menu.append(&paste_last);
    let _ = menu.append(&show_raw);
    let _ = menu.append(&undo);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&domains_menu);
    let _ = menu.append(&outputs_menu);
    let _ = menu.append(&mics_menu);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&clear);
    let _ = menu.append(&quit);

    let ids = MenuIds {
        quit: quit.id().clone(),
        rec: rec.id().clone(),
        paste_last: paste_last.id().clone(),
        show_raw: show_raw.id().clone(),
        undo: undo.id().clone(),
        clear: clear.id().clone(),
        domain_checks,
        output_checks,
        mic_checks,
        mic_refresh: mic_refresh.id().clone(),
    };

    (menu, ids, status)
}

pub fn build(
    domain: Domain,
    output: OutputMode,
    mic: Option<&str>,
    status: &str,
) -> tray_icon::Result<Tray> {
    let (menu, ids, status) = build_menu(domain, output, mic, status);

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("SciWhisper")
        .with_icon(make_icon(StatusIcon::Idle))
        // macOS menu-bar icons are masks, not miniature application icons.
        // Marking this as a template lets AppKit choose black or white for the
        // current menu-bar appearance.
        .with_icon_as_template(cfg!(target_os = "macos"))
        .with_title("")
        .build()?;

    Ok(Tray { icon, ids, status })
}

#[derive(Clone, Copy)]
pub enum StatusIcon {
    Idle,
    Recording,
    Processing,
    Failed,
}

pub fn make_icon(kind: StatusIcon) -> Icon {
    let rgba = make_icon_rgba(kind);
    let size = 32u32;
    Icon::from_rgba(rgba, size, size).expect("icon")
}

fn make_icon_rgba(kind: StatusIcon) -> Vec<u8> {
    const WITCH_ICON: &[u8] =
        include_bytes!("../../../assets/branding/si-witch-tray-wink-broom-sand-v1.png");
    let source = image::load_from_memory(WITCH_ICON).expect("embedded tray icon must be valid");
    let mut icon = source.resize_exact(32, 32, FilterType::Lanczos3).to_rgba8();

    #[cfg(target_os = "macos")]
    make_macos_template(&mut icon);

    let color = match kind {
        StatusIcon::Idle => None,
        #[cfg(target_os = "macos")]
        StatusIcon::Recording | StatusIcon::Processing | StatusIcon::Failed => Some([0, 0, 0, 255]),
        #[cfg(not(target_os = "macos"))]
        StatusIcon::Recording => Some([218, 48, 48, 255]),
        #[cfg(not(target_os = "macos"))]
        StatusIcon::Processing => Some([229, 154, 34, 255]),
        #[cfg(not(target_os = "macos"))]
        StatusIcon::Failed => Some([92, 92, 92, 255]),
    };
    if let Some(color) = color {
        paint_status_dot(&mut icon, color);
    }
    icon.into_raw()
}

#[cfg(target_os = "macos")]
fn make_macos_template(icon: &mut image::RgbaImage) {
    for pixel in icon.pixels_mut() {
        let [red, green, blue, source_alpha] = pixel.0;
        let luminance = (u32::from(red) * 77 + u32::from(green) * 150 + u32::from(blue) * 29) >> 8;

        // The source illustration has an opaque sand background. Convert its
        // distance from that light background into opacity, leaving only the
        // dark witch/broom silhouette for the AppKit template mask.
        let mask_alpha = 185u32.saturating_sub(luminance).saturating_mul(255) / 100;
        let alpha = (mask_alpha.min(255) * u32::from(source_alpha) / 255) as u8;
        let alpha = if alpha < 80 { 0 } else { alpha };
        *pixel = image::Rgba([0, 0, 0, alpha]);
    }
}

fn paint_status_dot(icon: &mut image::RgbaImage, color: [u8; 4]) {
    let (cx, cy) = (26i32, 26i32);
    for y in 20..32 {
        for x in 20..32 {
            let distance = (x as i32 - cx).pow(2) + (y as i32 - cy).pow(2);
            if distance <= 25 {
                let pixel = if distance >= 16 {
                    image::Rgba([30, 38, 38, 255])
                } else {
                    image::Rgba(color)
                };
                icon.put_pixel(x, y, pixel);
            }
        }
    }
}

pub fn set_status(tray: &Tray, kind: StatusIcon, tip: &str) {
    let _ = tray.icon.set_icon(Some(make_icon(kind)));
    let _ = tray.icon.set_tooltip(Some(tip));
    tray.status.set_text(tip);
    #[cfg(target_os = "macos")]
    {
        let title = match kind {
            StatusIcon::Recording => "● REC",
            StatusIcon::Processing => "…",
            StatusIcon::Failed => "!",
            StatusIcon::Idle => "",
        };
        tray.icon.set_title(Some(title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_witch_icon_decodes_at_tray_size() {
        let idle = make_icon_rgba(StatusIcon::Idle);
        let recording = make_icon_rgba(StatusIcon::Recording);
        assert_eq!(idle.len(), 32 * 32 * 4);
        assert_ne!(idle, recording, "recording status dot must be visible");

        #[cfg(target_os = "macos")]
        {
            assert!(idle.chunks_exact(4).all(|pixel| pixel[0..3] == [0, 0, 0]));
            assert!(idle.chunks_exact(4).any(|pixel| pixel[3] == 0));
            assert!(idle.chunks_exact(4).any(|pixel| pixel[3] > 200));
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(idle.chunks_exact(4).any(|pixel| pixel[0] > 150));
            assert!(idle.chunks_exact(4).any(|pixel| pixel[2] < 100));
        }
    }
}

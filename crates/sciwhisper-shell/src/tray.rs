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
    pub domain_auto: MenuId,
    pub domain_chem: MenuId,
    pub domain_math: MenuId,
    pub domain_phys: MenuId,
    pub domain_plain: MenuId,
    pub out_auto: MenuId,
    pub out_unicode: MenuId,
    pub out_latex: MenuId,
    pub out_word: MenuId,
}

pub struct Tray {
    pub icon: TrayIcon,
    pub ids: MenuIds,
    status: MenuItem,
}

pub fn build(domain: Domain, output: OutputMode, status: &str) -> tray_icon::Result<Tray> {
    let menu = Menu::new();
    let rec = MenuItem::new("Начать / завершить запись (Control ×2)", true, None);
    let paste_last = MenuItem::new("Повторить вставку", true, None);
    let show_raw = MenuItem::new("Скопировать исходный текст", true, None);
    let undo = MenuItem::new("Undo вставки", true, None);
    let clear = MenuItem::new("Очистить историю", true, None);
    let quit = MenuItem::new("Выход", true, None);

    let d_auto = CheckMenuItem::new("Auto", true, domain == Domain::Auto, None);
    let d_chem = CheckMenuItem::new("Chemistry", true, domain == Domain::Chemistry, None);
    let d_math = CheckMenuItem::new("Mathematics", true, domain == Domain::Mathematics, None);
    let d_phys = CheckMenuItem::new("Physics", true, domain == Domain::Physics, None);
    let d_plain = CheckMenuItem::new("Plain", true, domain == Domain::Plain, None);
    let domains = Submenu::new("Домен", true);
    let _ = domains.append(&d_auto);
    let _ = domains.append(&d_chem);
    let _ = domains.append(&d_math);
    let _ = domains.append(&d_phys);
    let _ = domains.append(&d_plain);

    let o_auto = CheckMenuItem::new("Auto", true, output == OutputMode::Auto, None);
    let o_uni = CheckMenuItem::new("Unicode", true, output == OutputMode::Unicode, None);
    let o_tex = CheckMenuItem::new("LaTeX", true, output == OutputMode::Latex, None);
    let o_word = CheckMenuItem::new("Word native", true, output == OutputMode::Word, None);
    let outputs = Submenu::new("Формат", true);
    let _ = outputs.append(&o_auto);
    let _ = outputs.append(&o_uni);
    let _ = outputs.append(&o_tex);
    let _ = outputs.append(&o_word);

    let status = MenuItem::new(status, false, None);
    let _ = menu.append(&status);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&rec);
    let _ = menu.append(&paste_last);
    let _ = menu.append(&show_raw);
    let _ = menu.append(&undo);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&domains);
    let _ = menu.append(&outputs);
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
        domain_auto: d_auto.id().clone(),
        domain_chem: d_chem.id().clone(),
        domain_math: d_math.id().clone(),
        domain_phys: d_phys.id().clone(),
        domain_plain: d_plain.id().clone(),
        out_auto: o_auto.id().clone(),
        out_unicode: o_uni.id().clone(),
        out_latex: o_tex.id().clone(),
        out_word: o_word.id().clone(),
    };

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

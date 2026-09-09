use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use image::imageops::FilterType;

use crate::config::OutputMode;
use sciwhisper_core::{Domain, UtteranceMode};

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
    /// One entry per dictation mode; exactly one should be checked at a time.
    pub dictation_checks: Vec<(CheckMenuItem, UtteranceMode)>,
    /// One entry per microphone option (`None` = system default), plus every
    /// device `capture::input_devices()` reported when the menu was last
    /// (re)built. Exactly one should be checked at a time.
    pub mic_checks: Vec<(CheckMenuItem, Option<String>)>,
    pub mic_refresh: MenuId,
    /// Asks GitHub whether a newer preview exists. The only network request
    /// the application makes, and it happens only when this is pressed.
    pub update_check: MenuId,
    /// Advances the update one step: download, then install. Its text says
    /// which step it is on, and it is disabled while there is nothing to do
    /// — a greyed-out item is honest, an item that does nothing is not.
    pub update_action: MenuId,
    pub update_notes: MenuId,
    /// Fixed slots for the readings of the last utterance. Created once and
    /// re-labelled, because a tray menu cannot be grown and shrunk at will
    /// without losing the ids the event loop matches on.
    pub choices: Vec<MenuId>,
    /// Whether a correction the user makes is written to a local file.
    /// A check item rather than a plain one, because its state is the whole
    /// point: a user must be able to see at a glance that their dictation
    /// is being recorded.
    pub remember_corrections: CheckMenuItem,
}

pub struct Tray {
    pub icon: TrayIcon,
    pub ids: MenuIds,
    status: MenuItem,
    update_action: MenuItem,
    update_notes: MenuItem,
    choice_slots: Vec<MenuItem>,
    choice_labels: Vec<String>,
    /// Kept so a menu rebuilt for a new microphone list does not throw away
    /// what the user was told about an update.
    update_label: Option<(String, bool)>,
}

impl Tray {
    pub fn current_status(&self) -> String {
        self.status.text()
    }

    /// Rebuilds the menu in place — same tray icon, fresh microphone list —
    /// so a newly plugged-in device shows up without restarting the app.
    pub fn refresh(
        &mut self,
        domain: Domain,
        output: OutputMode,
        dictation: UtteranceMode,
        mic: Option<&str>,
    ) {
        let status_text = self.status.text();
        let remember = self.ids.remember_corrections.is_checked();
        let (menu, ids, status, action, notes, slots) =
            build_menu(domain, output, dictation, mic, &status_text, remember);
        self.icon.set_menu(Some(Box::new(menu)));
        self.ids = ids;
        self.status = status;
        self.update_action = action;
        self.update_notes = notes;
        self.choice_slots = slots;
        let labels = std::mem::take(&mut self.choice_labels);
        self.set_choices(&labels);
        if let Some((label, notes_available)) = self.update_label.clone() {
            self.set_update_step(&label, notes_available);
        }
    }

    /// Sets what the update item offers next. `None` means there is nothing
    /// to offer, and the item is disabled rather than left saying something
    /// it can no longer do.
    pub fn set_update_step(&mut self, label: &str, notes_available: bool) {
        self.update_action.set_text(label);
        self.update_action.set_enabled(true);
        self.update_notes.set_enabled(notes_available);
        self.update_label = Some((label.to_string(), notes_available));
    }

    pub fn clear_update_step(&mut self) {
        self.update_action.set_text(NO_UPDATE_LABEL);
        self.update_action.set_enabled(false);
        self.update_notes.set_enabled(false);
        self.update_label = None;
    }
}

/// What the update item says when there is nothing staged or offered.
pub const NO_UPDATE_LABEL: &str = "Обновление не найдено";

/// How many readings of one utterance the menu can show.
///
/// Three, because a list a user has to read through is not a choice — it is
/// a second problem. If a future grammar produces more, the extra ones are
/// dropped rather than shown: an unusable menu would be worse than an
/// incomplete one, and anything beyond three competing readings means the
/// utterance was too ambiguous to answer at all.
pub const MAX_CHOICES: usize = 3;

impl Tray {
    /// Labels the reading slots. Fewer labels than slots leaves the rest
    /// disabled; an empty list disables all of them.
    pub fn set_choices(&mut self, labels: &[String]) {
        for (index, slot) in self.choice_slots.iter().enumerate() {
            match labels.get(index) {
                Some(label) => {
                    slot.set_text(label);
                    slot.set_enabled(true);
                }
                None => {
                    slot.set_text(EMPTY_CHOICE_LABEL);
                    slot.set_enabled(false);
                }
            }
        }
        self.choice_labels = labels.to_vec();
    }

    pub fn clear_choices(&mut self) {
        self.set_choices(&[]);
    }
}

const EMPTY_CHOICE_LABEL: &str = "—";

type BuiltMenu = (Menu, MenuIds, MenuItem, MenuItem, MenuItem, Vec<MenuItem>);

fn build_menu(
    domain: Domain,
    output: OutputMode,
    dictation: UtteranceMode,
    mic: Option<&str>,
    status: &str,
    remember: bool,
) -> BuiltMenu {
    let menu = Menu::new();
    let rec = MenuItem::new("Начать / завершить запись (Control ×2)", true, None);
    let paste_last = MenuItem::new("Повторить вставку", true, None);
    let show_raw = MenuItem::new("Скопировать исходный текст", true, None);
    let undo = MenuItem::new("Undo вставки", true, None);
    let clear = MenuItem::new("Очистить историю", true, None);
    let quit = MenuItem::new("Выход", true, None);
    let update_check = MenuItem::new("Проверить обновления", true, None);
    let update_action = MenuItem::new(NO_UPDATE_LABEL, false, None);
    let update_notes = MenuItem::new("Что нового", false, None);
    let remember_corrections = CheckMenuItem::new(
        "Запоминать мои исправления (локально)",
        true,
        remember,
        None,
    );
    let choice_slots: Vec<MenuItem> = (0..MAX_CHOICES)
        .map(|_| MenuItem::new(EMPTY_CHOICE_LABEL, false, None))
        .collect();

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

    // What happens to the ordinary words around a formula. The default keeps
    // everything the speaker said, so the destructive-looking option is never
    // the one a user lands on by accident.
    let dictation_menu = Submenu::new("Диктовка", true);
    let mut dictation_checks = Vec::new();
    for (label, value) in [
        ("Смешанный текст: сохранять речь", UtteranceMode::MixedText),
        (
            "Только формула: убирать вводные",
            UtteranceMode::ScientificOnly,
        ),
    ] {
        let item = CheckMenuItem::new(label, true, value == dictation, None);
        let _ = dictation_menu.append(&item);
        dictation_checks.push((item, value));
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
    let _ = menu.append(&dictation_menu);
    let _ = menu.append(&mics_menu);
    // Readings of the last utterance. Placed next to «Undo вставки» because
    // they do the same thing from the user's side: take back what was
    // inserted and put something else there.
    let choices_menu = Submenu::new("Варианты прочтения", true);
    for slot in &choice_slots {
        let _ = choices_menu.append(slot);
    }
    let _ = choices_menu.append(&PredefinedMenuItem::separator());
    let _ = choices_menu.append(&remember_corrections);
    let _ = menu.append(&choices_menu);

    let _ = menu.append(&PredefinedMenuItem::separator());
    let updates_menu = Submenu::new("Обновления", true);
    let _ = updates_menu.append(&update_check);
    let _ = updates_menu.append(&update_action);
    let _ = updates_menu.append(&update_notes);
    let _ = menu.append(&updates_menu);
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
        dictation_checks,
        mic_checks,
        mic_refresh: mic_refresh.id().clone(),
        update_check: update_check.id().clone(),
        update_action: update_action.id().clone(),
        update_notes: update_notes.id().clone(),
        choices: choice_slots.iter().map(|slot| slot.id().clone()).collect(),
        remember_corrections,
    };

    (menu, ids, status, update_action, update_notes, choice_slots)
}

pub fn build(
    domain: Domain,
    output: OutputMode,
    dictation: UtteranceMode,
    mic: Option<&str>,
    status: &str,
    remember_corrections: bool,
) -> tray_icon::Result<Tray> {
    let (menu, ids, status, update_action, update_notes, choice_slots) =
        build_menu(domain, output, dictation, mic, status, remember_corrections);

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

    Ok(Tray {
        icon,
        ids,
        status,
        update_action,
        update_notes,
        update_label: None,
        choice_slots,
        choice_labels: Vec::new(),
    })
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
            assert!(idle
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[0..3] == [0, 0, 0]));
            assert!(idle.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
            assert!(idle.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 200));
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(idle.as_chunks::<4>().0.iter().any(|pixel| pixel[0] > 150));
            assert!(idle.as_chunks::<4>().0.iter().any(|pixel| pixel[2] < 100));
        }
    }
}

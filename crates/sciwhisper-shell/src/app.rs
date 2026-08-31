//! System-panel application: global recording controls and insertion into the front app.

use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use sciwhisper_asr::capture::{PttSession, Recording};
use sciwhisper_asr::pipeline::{compile_transcript, PipelineResult};
use sciwhisper_asr::{prompt, SharedEngine, TranscribeOptions};
use sciwhisper_core::Domain;
use tray_icon::menu::MenuEvent;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::config::{Config, OutputMode};
use crate::error::{Error, Result};
use crate::history::{History, HistoryItem};
use crate::hotkey::{self, Combo, Key};
use crate::indicator::RecordingIndicator;
use crate::insert::{self, LastInsert};
use crate::key_listener::KeyEvent;
use crate::tray::{self, StatusIcon, Tray};

enum Msg {
    PttDown { sticky: Option<OutputMode> },
    PttUp,
    ToggleRecording,
    Cancel,
    WhisperReady,
    WhisperFailed(String),
    HotkeyFailed(String),
    Done(DoneKind),
}

enum DoneKind {
    Ok(Box<PipelineResult>),
    Err(String),
}

struct State {
    config: Config,
    domain: Domain,
    output: OutputMode,
    history: History,
    last_insert: Option<LastInsert>,
    phase: Phase,
    sticky_output: Option<OutputMode>,
    accessibility: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Recording,
    Processing,
}

pub fn run() -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let domain = config.domain();
    let output = config.output();
    let ptt = Combo::parse(&config.ptt).map_err(Error::Message)?;
    let ptt_latex = Combo::parse(&config.ptt_latex).ok();
    let ptt_word = Combo::parse(&config.ptt_word).ok();

    let (tx, rx) = mpsc::channel::<Msg>();
    let model = config.model.clone();
    let tx_w = tx.clone();
    thread::spawn(move || match SharedEngine::spawn(model.as_deref()) {
        Ok(eng) => {
            let _ = tx_w.send(Msg::WhisperReady);
            // store in process by leaking into a slot
            ENGINE.lock().unwrap().replace(eng);
        }
        Err(e) => {
            let _ = tx_w.send(Msg::WhisperFailed(e.to_string()));
        }
    });

    spawn_hotkeys(tx.clone(), ptt, ptt_latex, ptt_word, config.double_control);
    let audio_tx = spawn_audio_thread();

    let event_loop = EventLoop::<Msg>::with_user_event()
        .build()
        .map_err(|e| Error::Message(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        while let Ok(m) = rx.recv() {
            let _ = proxy.send_event(m);
        }
    });

    let state = State {
        config,
        domain,
        output,
        history: History::default(),
        last_insert: None,
        phase: Phase::Idle,
        sticky_output: None,
        accessibility: cfg!(not(target_os = "macos")),
    };

    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = DesktopApp {
        tray: None,
        indicator: None,
        state,
        audio: audio_tx,
        tx,
        quit: false,
        pending: Vec::new(),
        fatal_error: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| Error::Message(e.to_string()))?;
    match app.fatal_error {
        Some(error) => Err(Error::Message(error)),
        None => Ok(()),
    }
}

struct DesktopApp {
    tray: Option<Tray>,
    indicator: Option<RecordingIndicator>,
    state: State,
    audio: Sender<AudioCmd>,
    tx: Sender<Msg>,
    quit: bool,
    pending: Vec<Msg>,
    fatal_error: Option<String>,
}

impl DesktopApp {
    fn handle_event(&mut self, event: Msg) {
        let Some(tray) = self.tray.as_ref() else {
            self.pending.push(event);
            return;
        };
        let previous_phase = self.state.phase;
        handle_msg(event, tray, &mut self.state, &self.audio, &self.tx);
        if previous_phase != self.state.phase {
            if let Some(indicator) = self.indicator.as_mut() {
                indicator.set_recording(self.state.phase == Phase::Recording);
            }
        }
    }

    fn handle_pending_menu(&mut self, event_loop: &ActiveEventLoop) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            handle_menu(
                event.id().as_ref(),
                tray,
                &mut self.state,
                &self.audio,
                &self.tx,
                &mut self.quit,
            );
            if self.quit {
                event_loop.exit();
                break;
            }
        }
    }
}

impl ApplicationHandler<Msg> for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray.is_some() {
            return;
        }
        // AppKit must be running before NSStatusItem is created. Building the tray before
        // `run_app` makes an unsigned CLI process terminate silently on macOS.
        match tray::build(self.state.domain, self.state.output, "загрузка Whisper…") {
            Ok(tray) => {
                self.tray = Some(tray);
                self.indicator = RecordingIndicator::new(event_loop).ok();
                self.state.accessibility = crate::permissions::request_accessibility();
                insert::notify(
                    "SciWhisper",
                    "Нажмите Control дважды, говорите, затем нажмите Control дважды ещё раз.",
                );
                for event in std::mem::take(&mut self.pending) {
                    self.handle_event(event);
                }
            }
            Err(error) => {
                self.fatal_error = Some(format!("failed to create tray icon: {error}"));
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Msg) {
        self.handle_pending_menu(event_loop);
        if !self.quit {
            self.handle_event(event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.handle_pending_menu(event_loop);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if matches!(event, WindowEvent::RedrawRequested) {
            if let Some(indicator) = self.indicator.as_mut() {
                if indicator.window_id() == window_id {
                    indicator.redraw();
                }
            }
        }
    }
}

enum AudioCmd {
    Start(Sender<std::result::Result<(), String>>),
    Stop(Sender<std::result::Result<Recording, String>>),
    Cancel,
}

fn spawn_audio_thread() -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel::<AudioCmd>();
    thread::spawn(move || {
        let mut session: Option<PttSession> = None;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                AudioCmd::Start(ack) => match PttSession::start() {
                    Ok(s) => {
                        session = Some(s);
                        let _ = ack.send(Ok(()));
                    }
                    Err(e) => {
                        let _ = ack.send(Err(e.to_string()));
                    }
                },
                AudioCmd::Stop(ack) => {
                    let r = match session.take() {
                        Some(s) => s.finish().map_err(|e| e.to_string()),
                        None => Err("no recording session".into()),
                    };
                    let _ = ack.send(r);
                }
                AudioCmd::Cancel => {
                    if let Some(s) = session.take() {
                        s.cancel();
                    }
                }
            }
        }
    });
    tx
}

static ENGINE: Mutex<Option<SharedEngine>> = Mutex::new(None);

fn spawn_hotkeys(
    tx: Sender<Msg>,
    ptt: Combo,
    ptt_latex: Option<Combo>,
    ptt_word: Option<Combo>,
    double_control: bool,
) {
    thread::spawn(move || {
        let mut down: HashSet<Key> = HashSet::new();
        let mut holding = false;
        let mut double_control_detector = DoubleControlDetector::default();
        let event_tx = tx.clone();
        if let Err(error) = crate::key_listener::listen(move |event| {
            if double_control && double_control_detector.observe(event, Instant::now()) {
                let _ = event_tx.send(Msg::ToggleRecording);
            }
            match event {
                KeyEvent::Press(k) => {
                    down.insert(k);
                    if hotkey::is_escape(k) {
                        let _ = event_tx.send(Msg::Cancel);
                        holding = false;
                        return;
                    }
                    if !holding && ptt.trigger_down(&down) {
                        holding = true;
                        let _ = event_tx.send(Msg::PttDown { sticky: None });
                    } else if !holding {
                        if ptt_latex
                            .as_ref()
                            .map(|c| c.trigger_down(&down))
                            .unwrap_or(false)
                        {
                            holding = true;
                            let _ = event_tx.send(Msg::PttDown {
                                sticky: Some(OutputMode::Latex),
                            });
                        } else if ptt_word
                            .as_ref()
                            .map(|c| c.trigger_down(&down))
                            .unwrap_or(false)
                        {
                            holding = true;
                            let _ = event_tx.send(Msg::PttDown {
                                sticky: Some(OutputMode::Word),
                            });
                        }
                    }
                }
                KeyEvent::Release(k) => {
                    down.remove(&k);
                    if holding && (k == ptt.trigger || !ptt.modifiers_held(&down)) {
                        holding = false;
                        let _ = event_tx.send(Msg::PttUp);
                    }
                }
            }
        }) {
            let _ = tx.send(Msg::HotkeyFailed(error));
        }
    });
}

const CONTROL_TAP_MAX: Duration = Duration::from_millis(350);
const DOUBLE_CONTROL_GAP: Duration = Duration::from_millis(500);

#[derive(Default)]
struct DoubleControlDetector {
    control_down: bool,
    clean_tap: bool,
    pressed_at: Option<Instant>,
    first_tap_at: Option<Instant>,
}

impl DoubleControlDetector {
    fn observe(&mut self, event: KeyEvent, now: Instant) -> bool {
        match event {
            KeyEvent::Press(key) if is_control(key) => {
                if self.control_down {
                    self.clean_tap = false;
                    self.first_tap_at = None;
                } else {
                    self.control_down = true;
                    self.clean_tap = true;
                    self.pressed_at = Some(now);
                }
            }
            KeyEvent::Press(_) => {
                self.clean_tap = false;
                self.first_tap_at = None;
            }
            KeyEvent::Release(key) if is_control(key) => {
                if !self.control_down {
                    return false;
                }
                self.control_down = false;
                let quick = self
                    .pressed_at
                    .take()
                    .map(|pressed| now.saturating_duration_since(pressed) <= CONTROL_TAP_MAX)
                    .unwrap_or(false);
                if !self.clean_tap || !quick {
                    self.clean_tap = false;
                    self.first_tap_at = None;
                    return false;
                }
                self.clean_tap = false;
                if let Some(first) = self.first_tap_at.take() {
                    if now.saturating_duration_since(first) <= DOUBLE_CONTROL_GAP {
                        return true;
                    }
                }
                self.first_tap_at = Some(now);
            }
            KeyEvent::Release(_) => {}
        }
        false
    }
}

fn is_control(key: Key) -> bool {
    matches!(key, Key::ControlLeft | Key::ControlRight)
}

fn handle_msg(
    msg: Msg,
    tray: &Tray,
    state: &mut State,
    audio: &Sender<AudioCmd>,
    tx: &Sender<Msg>,
) {
    match msg {
        Msg::WhisperReady => {
            if state.accessibility {
                tray::set_status(tray, StatusIcon::Idle, "SciWhisper готов");
                insert::notify(
                    "SciWhisper",
                    "Модель распознавания загружена. Можно диктовать.",
                );
            } else {
                tray::set_status(
                    tray,
                    StatusIcon::Failed,
                    "Разрешите Accessibility для автоматической вставки",
                );
                insert::notify(
                    "SciWhisper",
                    "Разрешите Accessibility и перезапустите приложение; пока текст останется в буфере.",
                );
            }
        }
        Msg::WhisperFailed(e) => {
            tray::set_status(tray, StatusIcon::Failed, &e);
            insert::notify("SciWhisper", &e);
        }
        Msg::HotkeyFailed(e) => {
            tray::set_status(tray, StatusIcon::Failed, &e);
            insert::notify("SciWhisper", &e);
        }
        Msg::PttDown { sticky } => {
            if state.phase != Phase::Idle {
                return;
            }
            state.sticky_output = sticky;
            state.accessibility = crate::permissions::accessibility_trusted();
            let (ack_tx, ack_rx) = mpsc::channel();
            let _ = audio.send(AudioCmd::Start(ack_tx));
            match ack_rx.recv() {
                Ok(Ok(())) => {
                    state.phase = Phase::Recording;
                    tray::set_status(tray, StatusIcon::Recording, "● запись — Esc отменяет");
                }
                Ok(Err(e)) => insert::notify("SciWhisper", &e),
                Err(_) => insert::notify("SciWhisper", "audio thread closed"),
            }
        }
        Msg::Cancel => {
            let _ = audio.send(AudioCmd::Cancel);
            state.phase = Phase::Idle;
            tray::set_status(tray, StatusIcon::Idle, "запись отменена");
        }
        Msg::PttUp => {
            if state.phase != Phase::Recording {
                return;
            }
            state.phase = Phase::Processing;
            tray::set_status(tray, StatusIcon::Processing, "Whisper…");
            let domain = state.domain;
            let language = state.config.language.clone();
            let tx = tx.clone();
            let audio = audio.clone();
            thread::spawn(move || {
                let (ack_tx, ack_rx) = mpsc::channel();
                let _ = audio.send(AudioCmd::Stop(ack_tx));
                let rec = match ack_rx.recv() {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        let _ = tx.send(Msg::Done(DoneKind::Err(e)));
                        return;
                    }
                    Err(_) => {
                        let _ = tx.send(Msg::Done(DoneKind::Err("audio thread closed".into())));
                        return;
                    }
                };
                let done = (|| {
                    let eng = ENGINE.lock().unwrap();
                    let eng = eng.as_ref().ok_or_else(|| {
                        sciwhisper_asr::Error::Message("Whisper ещё не готов".into())
                    })?;
                    let t = eng.transcribe(
                        &rec.wav_path,
                        &TranscribeOptions {
                            language,
                            model: String::new(),
                            initial_prompt: prompt::for_domain(domain),
                            temperature: 0.0,
                        },
                    )?;
                    Ok::<_, sciwhisper_asr::Error>(compile_transcript(t, domain))
                })();
                match done {
                    Ok(p) => {
                        let _ = tx.send(Msg::Done(DoneKind::Ok(Box::new(p))));
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Done(DoneKind::Err(e.to_string())));
                    }
                }
            });
        }
        Msg::ToggleRecording => match state.phase {
            Phase::Idle => handle_msg(Msg::PttDown { sticky: None }, tray, state, audio, tx),
            Phase::Recording => handle_msg(Msg::PttUp, tray, state, audio, tx),
            Phase::Processing => {}
        },
        Msg::Done(kind) => match kind {
            DoneKind::Err(e) => {
                state.phase = Phase::Idle;
                tray::set_status(tray, StatusIcon::Failed, &e);
                insert::notify("SciWhisper", &e);
            }
            DoneKind::Ok(res) => {
                state.phase = Phase::Idle;
                if res.transcript.no_speech {
                    tray::set_status(tray, StatusIcon::Idle, "тишина");
                    return;
                }
                state.history.push(HistoryItem {
                    raw: res.transcript.text.clone(),
                    unicode: res.unicode.clone(),
                    latex: res.latex.clone(),
                    omml: res.omml.clone(),
                    domain: res.interpretation.domain.as_str().into(),
                });
                let mode = state.sticky_output.take().unwrap_or(state.output);
                match insert::insert(insert::InsertRequest { result: &res, mode }) {
                    Ok(out) => {
                        let preview = if res.interpretation.confidence > 0.0 {
                            &res.unicode
                        } else {
                            &res.transcript.text
                        };
                        let status = if out.method.ends_with("clipboard")
                            || out.method == "clipboard-left"
                        {
                            format!("Accessibility запрещён · текст в буфере: {preview}")
                        } else {
                            format!("{} · {preview}", out.method)
                        };
                        tray::set_status(tray, StatusIcon::Idle, &status);
                        state.last_insert = Some(LastInsert {
                            front: out.front,
                            payload: out.payload,
                            raw: res.transcript.text.clone(),
                        });
                    }
                    Err(e) => {
                        insert::notify("SciWhisper", &e.to_string());
                        let _ = crate::clipboard::set_text(&res.unicode);
                        tray::set_status(tray, StatusIcon::Idle, "результат в буфере");
                    }
                }
            }
        },
    }
}

fn handle_menu(
    id: &str,
    tray: &Tray,
    st: &mut State,
    _audio: &Sender<AudioCmd>,
    tx: &Sender<Msg>,
    quit: &mut bool,
) {
    let ids = &tray.ids;
    if id == ids.quit.as_ref() {
        *quit = true;
        return;
    }
    if id == ids.rec.as_ref() {
        let _ = tx.send(Msg::ToggleRecording);
        return;
    }
    if id == ids.clear.as_ref() {
        st.history.clear();
        st.last_insert = None;
        insert::notify("SciWhisper", "история очищена");
        return;
    }
    if id == ids.show_raw.as_ref() {
        if let Some(h) = st.history.last() {
            let copied = crate::clipboard::set_text(&h.raw).is_ok();
            let status = if copied {
                format!("raw скопирован: {}", h.raw)
            } else {
                format!("Whisper raw: {}", h.raw)
            };
            tray::set_status(tray, StatusIcon::Idle, &status);
        }
        return;
    }
    if id == ids.paste_last.as_ref() {
        if let Some(h) = st.history.last() {
            let mut dummy = compile_transcript(
                sciwhisper_asr::Transcript {
                    text: h.raw.clone(),
                    language: None,
                    segments: vec![],
                    no_speech: false,
                },
                Domain::Auto,
            );
            dummy.unicode = h.unicode.clone();
            dummy.latex = h.latex.clone();
            dummy.omml = h.omml.clone();
            dummy.interpretation.confidence = 1.0;
            let _ = insert::insert(insert::InsertRequest {
                result: &dummy,
                mode: st.output,
            });
        }
        return;
    }
    if id == ids.undo.as_ref() {
        if let Some(last) = &st.last_insert {
            if last.can_undo() {
                insert::send_undo();
            } else {
                insert::notify(
                    "SciWhisper",
                    &format!("безопасный undo недоступен. raw: {}", last.raw),
                );
            }
        }
        return;
    }
    if id == ids.domain_auto.as_ref() {
        st.domain = Domain::Auto;
    }
    if id == ids.domain_chem.as_ref() {
        st.domain = Domain::Chemistry;
    }
    if id == ids.domain_math.as_ref() {
        st.domain = Domain::Mathematics;
    }
    if id == ids.domain_phys.as_ref() {
        st.domain = Domain::Physics;
    }
    if id == ids.domain_plain.as_ref() {
        st.domain = Domain::Plain;
    }
    if id == ids.out_auto.as_ref() {
        st.output = OutputMode::Auto;
    }
    if id == ids.out_unicode.as_ref() {
        st.output = OutputMode::Unicode;
    }
    if id == ids.out_latex.as_ref() {
        st.output = OutputMode::Latex;
    }
    if id == ids.out_word.as_ref() {
        st.output = OutputMode::Word;
    }
    st.config.domain = st.domain.as_str().into();
    st.config.output = st.output.as_str().into();
    let _ = st.config.save();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_control_toggles_after_two_clean_taps() {
        let start = Instant::now();
        let mut detector = DoubleControlDetector::default();
        assert!(!detector.observe(KeyEvent::Press(Key::ControlLeft), start));
        assert!(!detector.observe(
            KeyEvent::Release(Key::ControlLeft),
            start + Duration::from_millis(80)
        ));
        assert!(!detector.observe(
            KeyEvent::Press(Key::ControlLeft),
            start + Duration::from_millis(180)
        ));
        assert!(detector.observe(
            KeyEvent::Release(Key::ControlLeft),
            start + Duration::from_millis(250)
        ));
    }

    #[test]
    fn control_used_in_a_shortcut_does_not_toggle() {
        let start = Instant::now();
        let mut detector = DoubleControlDetector::default();
        assert!(!detector.observe(KeyEvent::Press(Key::ControlLeft), start));
        assert!(!detector.observe(
            KeyEvent::Press(Key::KeyC),
            start + Duration::from_millis(40)
        ));
        assert!(!detector.observe(
            KeyEvent::Release(Key::KeyC),
            start + Duration::from_millis(70)
        ));
        assert!(!detector.observe(
            KeyEvent::Release(Key::ControlLeft),
            start + Duration::from_millis(90)
        ));
    }
}

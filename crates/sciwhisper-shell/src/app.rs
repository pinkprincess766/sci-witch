//! System-panel application: global recording controls and insertion into the front app.

use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use sciwhisper_asr::capture::{PttSession, Recording};
use sciwhisper_asr::pipeline::{compile_transcript, compile_transcript_with, PipelineResult};
use sciwhisper_asr::{prompt, SharedEngine, TranscribeOptions};
use sciwhisper_core::Domain;
use sciwhisper_core::UtteranceMode;
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
    PttDown {
        sticky: Option<OutputMode>,
    },
    PttUp,
    ToggleRecording,
    Cancel,
    WhisperReady,
    WhisperFailed(String),
    HotkeyFailed(String),
    Done(DoneKind),
    /// Result of the one network request the application makes.
    UpdateChecked(std::result::Result<Option<crate::update::UpdateInfo>, String>),
    /// A downloaded archive was unpacked and checked; nothing is installed
    /// yet.
    UpdateStaged(std::result::Result<Box<crate::upgrade::Staged>, String>),
}

/// How far the user has chosen to take an update. Every transition is a
/// press: nothing here advances on its own, and nothing runs on a timer.
enum UpdateState {
    /// Nothing asked for, or nothing found.
    None,
    /// A request is in flight; a second press must not start another.
    Busy(&'static str),
    /// Found and described, not downloaded.
    Available(Box<crate::update::UpdateInfo>),
    /// Downloaded, unpacked and verified beside the installation. The
    /// running program is still untouched.
    Staged(Box<crate::upgrade::Staged>),
}

enum DoneKind {
    Ok(Box<PipelineResult>),
    Err(String),
}

struct State {
    config: Config,
    domain: Domain,
    output: OutputMode,
    /// Whether the ordinary words around a formula survive. Changeable from
    /// the tray so a user never has to edit a config file to get it.
    dictation: UtteranceMode,
    history: History,
    last_insert: Option<LastInsert>,
    phase: Phase,
    sticky_output: Option<OutputMode>,
    accessibility: bool,
    update: UpdateState,
    /// Release notes URL of whatever the update menu currently offers.
    update_notes_url: Option<String>,
    /// The readings of the last utterance, in menu order. Empty when the
    /// last utterance had only one.
    choices: Vec<Choice>,
}

/// One reading of what was said, ready to be inserted in place of another.
///
/// The point of holding these is that the answer is not withheld while the
/// user decides. The primary reading is inserted immediately, exactly as
/// before; a choice here **replaces** it. Waiting for a decision would make
/// every ambiguous utterance slower, and ambiguity is the common case for
/// spoken mathematics, not the rare one.
struct Choice {
    label: String,
    unicode: String,
    latex: String,
    omml: String,
    /// `false` for the raw Whisper transcript, which is words rather than a
    /// compiled formula and must never be inserted as one.
    compiled: bool,
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
    let dictation = config.dictation_mode();
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
        dictation,
        history: History::default(),
        last_insert: None,
        phase: Phase::Idle,
        sticky_output: None,
        accessibility: cfg!(not(target_os = "macos")),
        update: UpdateState::None,
        update_notes_url: None,
        choices: Vec::new(),
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
        let Some(tray) = self.tray.as_mut() else {
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
        let Some(tray) = self.tray.as_mut() else {
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
        match tray::build(
            self.state.domain,
            self.state.output,
            self.state.dictation,
            self.state.config.mic.as_deref(),
            "загрузка Whisper…",
        ) {
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
    Start(Sender<std::result::Result<(), String>>, Option<String>),
    Stop(Sender<std::result::Result<Recording, String>>),
    Cancel,
}

fn spawn_audio_thread() -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel::<AudioCmd>();
    thread::spawn(move || {
        let mut session: Option<PttSession> = None;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                AudioCmd::Start(ack, mic) => match PttSession::start(mic.as_deref()) {
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
    tray: &mut Tray,
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
        Msg::UpdateChecked(result) => match result {
            Ok(Some(info)) => {
                let text = crate::upgrade::describe(&info, env!("CARGO_PKG_VERSION"));
                tray.set_update_step(&format!("Скачать обновление {}", info.version), true);
                state.update_notes_url = Some(info.notes_url.clone());
                state.update = UpdateState::Available(Box::new(info));
                tray::set_status(tray, StatusIcon::Idle, &text);
                insert::notify("SciWhisper", &text);
            }
            Ok(None) => {
                tray.clear_update_step();
                state.update = UpdateState::None;
                state.update_notes_url = None;
                let text = format!("Обновлений нет, установлена {}", env!("CARGO_PKG_VERSION"));
                tray::set_status(tray, StatusIcon::Idle, &text);
                insert::notify("SciWhisper", &text);
            }
            Err(error) => {
                tray.clear_update_step();
                state.update = UpdateState::None;
                tray::set_status(
                    tray,
                    StatusIcon::Idle,
                    &format!("проверка обновлений: {error}"),
                );
                insert::notify(
                    "SciWhisper",
                    &format!("Проверить обновления не удалось: {error}"),
                );
            }
        },
        Msg::UpdateStaged(result) => match result {
            Ok(staged) => {
                let text = crate::upgrade::describe_staged(&staged);
                tray.set_update_step(
                    &format!("Установить {} и перезапустить", staged.version),
                    state.update_notes_url.is_some(),
                );
                state.update = UpdateState::Staged(staged);
                tray::set_status(tray, StatusIcon::Idle, &text);
                insert::notify("SciWhisper", &text);
            }
            Err(error) => {
                // Nothing was replaced, so the running version is the one it
                // always was; say that rather than leaving the user guessing.
                tray.clear_update_step();
                state.update = UpdateState::None;
                let text = format!("Обновление не установлено: {error}. Программа не изменена.");
                tray::set_status(tray, StatusIcon::Idle, &text);
                insert::notify("SciWhisper", &text);
            }
        },
        Msg::PttDown { sticky } => {
            if state.phase != Phase::Idle {
                return;
            }
            state.sticky_output = sticky;
            state.accessibility = crate::permissions::accessibility_trusted();
            let (ack_tx, ack_rx) = mpsc::channel();
            let _ = audio.send(AudioCmd::Start(ack_tx, state.config.mic.clone()));
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
            let dictation = state.dictation;
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
                    Ok::<_, sciwhisper_asr::Error>(compile_transcript_with(t, domain, dictation))
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
                match insert::insert(insert::InsertRequest {
                    result: &res,
                    mode,
                    profiles: &state.config.profiles,
                }) {
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
                        offer_choices(tray, state, &res);
                    }
                    Err(e) => {
                        insert::notify("SciWhisper", &e.to_string());
                        let _ = crate::clipboard::set_text(&res.unicode);
                        tray::set_status(tray, StatusIcon::Idle, "результат в буфере");
                    }
                }
                // The dictated (possibly unbalanced) equation is always what
                // gets inserted; this only adds a follow-up notice, never a
                // silent substitution.
                if let Some(notice) = chemistry_balance_notice(&res.interpretation.warnings) {
                    insert::notify("SciWhisper", &notice);
                }
            }
        },
    }
}

/// The one place the application touches the network, and only because a
/// menu item was pressed.
fn handle_update_check(tray: &mut Tray, st: &mut State, tx: &Sender<Msg>) {
    if let UpdateState::Busy(what) = st.update {
        insert::notify("SciWhisper", &format!("уже идёт: {what}"));
        return;
    }
    st.update = UpdateState::Busy("проверка обновлений");
    tray::set_status(tray, StatusIcon::Processing, "проверка обновлений…");
    let tx = tx.clone();
    thread::spawn(move || {
        let result = crate::update::check_for_update(env!("CARGO_PKG_VERSION"))
            .map_err(|error| error.to_string());
        let _ = tx.send(Msg::UpdateChecked(result));
    });
}

/// Advances the update by exactly one step, and only the step the menu item
/// currently names.
fn handle_update_action(tray: &mut Tray, st: &mut State, tx: &Sender<Msg>, quit: &mut bool) {
    match std::mem::replace(&mut st.update, UpdateState::None) {
        UpdateState::None => {}
        UpdateState::Busy(what) => {
            st.update = UpdateState::Busy(what);
            insert::notify("SciWhisper", &format!("уже идёт: {what}"));
        }
        UpdateState::Available(info) => {
            let root = match crate::upgrade::install_root() {
                Ok(root) => root,
                Err(error) => {
                    // macOS lands here by design: the archive is downloaded
                    // and the user installs it, rather than having their
                    // microphone permissions silently reset.
                    insert::notify("SciWhisper", &error.to_string());
                    st.update = UpdateState::Available(info);
                    return;
                }
            };
            st.update = UpdateState::Busy("скачивание обновления");
            tray::set_status(tray, StatusIcon::Processing, "скачивание обновления…");
            let tx = tx.clone();
            thread::spawn(move || {
                let paths = crate::upgrade::plan_paths(&root, &info.version);
                let result = (|| {
                    std::fs::create_dir_all(&paths.archive).map_err(|e| e.to_string())?;
                    let archive = crate::update::download_and_verify(&info, &paths.archive)
                        .map_err(|e| e.to_string())?;
                    crate::upgrade::stage(&info, &archive, &root).map_err(|e| e.to_string())
                })();
                let _ = tx.send(Msg::UpdateStaged(result.map(Box::new)));
            });
        }
        UpdateState::Staged(staged) => match crate::upgrade::start_replacement(&staged) {
            Ok(()) => {
                // The helper is waiting for this process to let go of the
                // directory, so quitting is the last step of the install.
                insert::notify(
                    "SciWhisper",
                    &format!("Устанавливаю {} и перезапускаю…", staged.version),
                );
                *quit = true;
            }
            Err(error) => {
                insert::notify(
                    "SciWhisper",
                    &format!("Установить не удалось: {error}. Программа не изменена."),
                );
                st.update = UpdateState::Staged(staged);
            }
        },
    }
}

/// Fills the «Варианты прочтения» menu for what was just inserted.
///
/// Two kinds of entry go in, and they are different things wearing the same
/// shape. The competing readings exist only when the parser genuinely could
/// not tell them apart — «корень из икс плюс один» is `√x + 1` or `√(x+1)`,
/// and speech carries no bracket. The raw transcript is always offered,
/// because "it heard me wrong" is the one failure the user can always see
/// and the system never can.
fn offer_choices(tray: &mut Tray, state: &mut State, res: &PipelineResult) {
    let choices = build_choices(res, tray::MAX_CHOICES);
    let labels: Vec<String> = choices.iter().map(|choice| choice.label.clone()).collect();
    tray.set_choices(&labels);
    let competing = choices.iter().filter(|choice| choice.compiled).count();
    state.choices = choices;
    if competing > 0 {
        // Said out loud, because a silently chosen reading of an ambiguous
        // phrase is exactly the kind of quiet guess this project refuses to
        // make elsewhere.
        insert::notify(
            "SciWhisper",
            &format!(
                "Услышанное можно прочитать {} способами. Вставлено: {}. Другое прочтение — в меню «Варианты прочтения».",
                competing + 1,
                res.unicode
            ),
        );
    }
}

/// The list itself, with no tray and no notification, so what goes into the
/// menu can be checked without a desktop session.
fn build_choices(res: &PipelineResult, max: usize) -> Vec<Choice> {
    let mut choices = Vec::new();
    let interpretation = &res.interpretation;
    if interpretation.confidence > 0.0 {
        for node in &interpretation.alternatives {
            // One slot is always kept for the raw transcript.
            if choices.len() + 1 >= max {
                break;
            }
            let unicode = sciwhisper_core::render(node, sciwhisper_core::Renderer::Unicode);
            // A reading identical to what was inserted is not a choice, and
            // neither is a repeat of one already offered.
            if unicode == res.unicode || choices.iter().any(|c: &Choice| c.unicode == unicode) {
                continue;
            }
            choices.push(Choice {
                label: format!("Вставить вместо: {unicode}"),
                latex: sciwhisper_core::render(node, sciwhisper_core::Renderer::Latex),
                omml: sciwhisper_core::render(node, sciwhisper_core::Renderer::Omml),
                unicode,
                compiled: true,
            });
        }
    }
    let raw = res.transcript.text.trim();
    if !raw.is_empty() && raw != res.unicode {
        choices.push(Choice {
            label: format!("Вернуть услышанное: {raw}"),
            unicode: raw.to_string(),
            latex: raw.to_string(),
            omml: raw.to_string(),
            compiled: false,
        });
    }
    choices
}

/// Replaces what was just inserted with another reading.
///
/// This is an undo followed by an insert, and it only runs while the undo is
/// still safe — the same application must still be in front. Sending Ctrl+Z
/// into a window that was not the one typed into would delete something the
/// user did themselves, which is far worse than declining to help.
fn replace_insertion(tray: &mut Tray, st: &mut State, index: usize) {
    let Some(choice) = st.choices.get(index) else {
        return;
    };
    let undoable = st.last_insert.as_ref().is_some_and(|last| last.can_undo());
    if !undoable {
        // The words are still put where the user can get at them; only the
        // automatic replacement is refused.
        let copied = crate::clipboard::set_text(&choice.unicode).is_ok();
        insert::notify(
            "SciWhisper",
            &if copied {
                format!(
                    "Окно сменилось, отменять вставку в нём небезопасно. Вариант скопирован в буфер: {}",
                    choice.unicode
                )
            } else {
                format!(
                    "Окно сменилось, замена не выполнена. Вариант: {}",
                    choice.unicode
                )
            },
        );
        return;
    }

    insert::send_undo();
    let mut replacement = compile_transcript(
        sciwhisper_asr::Transcript {
            text: choice.unicode.clone(),
            language: None,
            segments: vec![],
            no_speech: false,
        },
        Domain::Auto,
    );
    replacement.unicode = choice.unicode.clone();
    replacement.latex = choice.latex.clone();
    replacement.omml = choice.omml.clone();
    // The raw transcript is words, not a formula: leaving confidence at zero
    // is what keeps the insertion path from treating it as native Word maths.
    replacement.interpretation.confidence = if choice.compiled { 1.0 } else { 0.0 };

    match insert::insert(insert::InsertRequest {
        result: &replacement,
        mode: st.output,
        profiles: &st.config.profiles,
    }) {
        Ok(out) => {
            tray::set_status(
                tray,
                StatusIcon::Idle,
                &format!("заменено: {}", choice.unicode),
            );
            st.last_insert = Some(LastInsert {
                front: out.front,
                payload: out.payload,
                raw: choice.unicode.clone(),
            });
        }
        Err(error) => insert::notify("SciWhisper", &error.to_string()),
    }
}

fn chemistry_balance_notice(warnings: &[sciwhisper_core::ast::Warning]) -> Option<String> {
    if !warnings
        .iter()
        .any(|w| w.code.starts_with("chemistry.unbalanced"))
    {
        return None;
    }
    let suggestion = warnings
        .iter()
        .find(|w| w.code == "chemistry.balance_suggestion")
        .and_then(|w| w.message.strip_prefix("predicted balanced form: "));
    Some(match suggestion {
        Some(balanced) => {
            format!("Уравнение не сбалансировано.\nВозможные коэффициенты:\n{balanced}")
        }
        None => "Уравнение не сбалансировано.".to_string(),
    })
}

fn handle_menu(
    id: &str,
    tray: &mut Tray,
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
        st.choices.clear();
        tray.clear_choices();
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
                profiles: &st.config.profiles,
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
    if let Some(index) = ids.choices.iter().position(|slot| id == slot.as_ref()) {
        replace_insertion(tray, st, index);
        return;
    }
    if id == ids.update_check.as_ref() {
        handle_update_check(tray, st, tx);
        return;
    }
    if id == ids.update_action.as_ref() {
        handle_update_action(tray, st, tx, quit);
        return;
    }
    if id == ids.update_notes.as_ref() {
        if let Some(url) = st.update_notes_url.clone() {
            if let Err(error) = crate::upgrade::open_notes(&url) {
                insert::notify("SciWhisper", &error.to_string());
            }
        }
        return;
    }
    if id == ids.mic_refresh.as_ref() {
        tray.refresh(st.domain, st.output, st.dictation, st.config.mic.as_deref());
        return;
    }
    if let Some((_, value)) = ids
        .domain_checks
        .iter()
        .find(|(item, _)| id == item.id().as_ref())
    {
        st.domain = *value;
    }
    if let Some((_, value)) = ids
        .output_checks
        .iter()
        .find(|(item, _)| id == item.id().as_ref())
    {
        st.output = *value;
    }
    if let Some((_, value)) = ids
        .dictation_checks
        .iter()
        .find(|(item, _)| id == item.id().as_ref())
    {
        st.dictation = *value;
    }
    if let Some((_, value)) = ids
        .mic_checks
        .iter()
        .find(|(item, _)| id == item.id().as_ref())
    {
        st.config.mic = value.clone();
    }
    // A tray menu's checkboxes are independent toggles, not a radio group by
    // default: without this, the previous selection stays checked and a
    // second click on the active one un-checks it while the config is
    // unchanged underneath.
    for (item, value) in &ids.domain_checks {
        item.set_checked(*value == st.domain);
    }
    for (item, value) in &ids.output_checks {
        item.set_checked(*value == st.output);
    }
    for (item, value) in &ids.dictation_checks {
        item.set_checked(*value == st.dictation);
    }
    for (item, value) in &ids.mic_checks {
        item.set_checked(*value == st.config.mic);
    }
    st.config.domain = st.domain.as_str().into();
    st.config.output = st.output.as_str().into();
    st.config.dictation = st.dictation.as_str().into();
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

    fn warning(code: &str, message: &str) -> sciwhisper_core::ast::Warning {
        sciwhisper_core::ast::Warning {
            code: code.into(),
            message: message.into(),
        }
    }

    #[test]
    fn balanced_reaction_gets_no_notice() {
        assert_eq!(chemistry_balance_notice(&[]), None);
    }

    #[test]
    fn unbalanced_reaction_with_suggestion_shows_it() {
        let warnings = [
            warning(
                "chemistry.unbalanced_atoms",
                "atom balance is not conserved (O: 2 → 3)",
            ),
            warning(
                "chemistry.balance_suggestion",
                "predicted balanced form: 2H₂O₂ → 2H₂O + O₂",
            ),
        ];
        assert_eq!(
            chemistry_balance_notice(&warnings),
            Some(
                "Уравнение не сбалансировано.\nВозможные коэффициенты:\n2H₂O₂ → 2H₂O + O₂"
                    .to_string()
            )
        );
    }

    #[test]
    fn unbalanced_reaction_without_a_unique_suggestion_still_warns() {
        let warnings = [warning(
            "chemistry.unbalanced_atoms",
            "atom balance is not conserved (C: 1 → 0)",
        )];
        assert_eq!(
            chemistry_balance_notice(&warnings),
            Some("Уравнение не сбалансировано.".to_string())
        );
    }
}

#[cfg(test)]
mod choice_tests {
    use super::*;
    use sciwhisper_asr::engine::Transcript;

    fn compiled(spoken: &str) -> PipelineResult {
        compile_transcript(
            Transcript {
                text: spoken.into(),
                language: Some("ru".into()),
                segments: vec![],
                no_speech: false,
            },
            Domain::Auto,
        )
    }

    /// The reading that could not be told apart from the one inserted is
    /// offered, and the words actually heard are always offered next to it.
    #[test]
    fn an_ambiguous_root_offers_the_other_reading_and_the_raw_words() {
        let result = compiled("корень из икс плюс один");
        assert_eq!(result.unicode, "√x + 1");
        let choices = build_choices(&result, tray::MAX_CHOICES);
        let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Вставить вместо: √(x + 1)",
                "Вернуть услышанное: корень из икс плюс один",
            ]
        );
        assert!(choices[0].compiled);
        // The raw transcript is words, not a formula, and must not be
        // inserted as native Word mathematics.
        assert!(!choices[1].compiled);
    }

    /// No ambiguity means no choice to make; only the words heard stay on
    /// offer, because misrecognition is always possible.
    #[test]
    fn an_unambiguous_utterance_offers_only_the_raw_words() {
        let result = compiled("икс в квадрате");
        let choices = build_choices(&result, tray::MAX_CHOICES);
        assert_eq!(choices.len(), 1);
        assert!(!choices[0].compiled);
        assert!(choices[0].label.starts_with("Вернуть услышанное"));
    }

    /// A menu the user has to read through is not a choice. One slot is
    /// always kept for the raw transcript, whatever the parser offers.
    #[test]
    fn the_menu_never_grows_past_its_slots() {
        let mut result = compiled("корень из икс плюс один");
        // Pretend a future grammar found many readings.
        let extra = result.interpretation.alternatives[0].clone();
        for _ in 0..10 {
            result.interpretation.alternatives.push(extra.clone());
        }
        let choices = build_choices(&result, tray::MAX_CHOICES);
        assert!(choices.len() <= tray::MAX_CHOICES);
        assert!(
            choices.last().is_some_and(|c| !c.compiled),
            "the raw transcript keeps its slot"
        );
        // Identical readings are collapsed rather than listed twice.
        assert_eq!(choices.iter().filter(|c| c.compiled).count(), 1);
    }

    /// Speech that produced no formula at all has nothing to offer instead
    /// of itself.
    #[test]
    fn plain_speech_that_was_kept_as_words_offers_nothing() {
        let result = compiled("обычная неразобранная фраза");
        assert_eq!(result.interpretation.confidence, 0.0);
        assert_eq!(result.unicode, "обычная неразобранная фраза");
        assert!(build_choices(&result, tray::MAX_CHOICES).is_empty());
    }
}

//! Document2Voice (Kokoro) — desktop GUI
//!
//! Offline audiobook maker using the Kokoro ONNX speech model. Styled to feel
//! closer to a GNOME/Adwaita desktop app (light surfaces, soft borders, blue accent).

mod app_paths;
mod focus;
mod jobs;
mod log_hl;
mod message;
mod pages;
mod paths;
mod settings;
mod style;
mod types;
mod update;
mod view;

use focus::{unfocus_text_inputs, FocusSlot};
use jobs::ActiveJob;
use log_hl::LogKind;
pub(crate) use message::Message;
use paths::{PathLog, SourceError};
use settings::SavedSettings;
use style::{adwaita_theme, MAIN_H, MAIN_W};
use types::{
    BitrateUi, BusyKind, DocReady, FormatUi, JobProgress, OutPathOrigin, PagesFieldError,
    SampleRateUi, VoiceUi,
};

use iced::event::{self, Event};
use iced::keyboard::{self, Key};
use iced::mouse;
use iced::widget::text_editor;
use iced::window;
use iced::{Font, Size, Subscription, Task, Theme};

use document2voice_core::Language;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// App icon baked into the binary (PNG bytes — no OS icon theme needed).
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// Max lines kept in the Log card (oldest dropped when over the limit).
const LOG_MAX_LINES: usize = 400;

/// How long a voice preview aims to be (wall-clock audio seconds).
pub(crate) const PREVIEW_SECS: f32 = 10.0;

/// How many pages of the selection to open for a document Preview (speech is still
/// capped at [`PREVIEW_SECS`]; this only limits extract cost on long books).
const PREVIEW_EXTRACT_PAGES: u32 = 2;

/// Fixed script spoken when Preview is pressed with an empty Source path.
///
/// About 40 words so normal speaking rates fill roughly ten seconds; speech is
/// still capped at [`PREVIEW_SECS`]. Same words every time so voice compares are fair.
pub(crate) const VOICE_PREVIEW_TEXT: &str = "\
This is a short voice preview from Document2Voice (Kokoro). \
Listen for clarity, natural pace, and tone as the narrator reads these sample words. \
Soft vowels and clear consonants help you judge how this voice will sound with your own book.";

/// How long to wait before hover tips appear.
pub(crate) const TOOLTIP_DELAY: Duration = Duration::from_secs(1);

/// Loudness slider: leftmost stop = Off; remaining range is LUFS (−24 … −12).
pub(crate) const LOUD_OFF: f32 = -25.0;
pub(crate) const LOUD_MIN: f32 = -24.0;
pub(crate) const LOUD_MAX: f32 = -12.0;

/// Speech control ranges (shared by sliders, arrow nudges, and settings clamp).
pub(crate) const SPEED_MIN: f32 = 0.5;
pub(crate) const SPEED_MAX: f32 = 1.5;
pub(crate) const PITCH_MIN: f32 = -6.0;
pub(crate) const PITCH_MAX: f32 = 6.0;
pub(crate) const STYLE_MIN: f32 = 0.0;
pub(crate) const STYLE_MAX: f32 = 400.0;
pub(crate) const PAUSE_MIN: f32 = 0.0;
pub(crate) const PAUSE_MAX: f32 = 2.0;

fn main() -> iced::Result {
    // Killable child mode for Start/Preview: the GUI re-launches this same binary.
    if std::env::args().nth(1).as_deref() == Some(jobs::SYNTH_WORKER_FLAG) {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .init();
        match jobs::run_embedded_synth_worker(std::env::args().skip(2)) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
    }

    // Default: our crates at info. Mute iced / desktop-portal noise
    // (GPU notes, zbus portal property cache, etc.). Override with RUST_LOG=debug.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "warn,\
                 document2voice_gui=info,\
                 document2voice_core=info,\
                 iced_winit=error,\
                 iced_wgpu=error,\
                 winit=error,\
                 wgpu_hal=error,\
                 wgpu_core=error,\
                 naga=error,\
                 zbus=error,\
                 zbus::connection=error,\
                 zbus::proxy=error,\
                 ashpd=error"
                    .into()
            }),
        )
        .init();

    iced::daemon(App::new, App::update, App::view)
        .title(app_title)
        .subscription(App::subscription)
        .theme(app_theme)
        .default_font(Font::DEFAULT)
        .run()
}

fn app_title(_app: &App, _window: window::Id) -> String {
    format!("Document2Voice (Kokoro)  v{}", env!("CARGO_PKG_VERSION"))
}

fn app_theme(_app: &App, _window: window::Id) -> Option<Theme> {
    Some(adwaita_theme())
}

fn main_window_settings() -> window::Settings {
    let size = Size::new(MAIN_W, MAIN_H);
    let icon = window::icon::from_file_data(APP_ICON_PNG, None).ok();
    window::Settings {
        size,
        min_size: Some(size),
        max_size: Some(size),
        resizable: false,
        icon,
        ..window::Settings::default()
    }
}

pub(crate) struct App {
    pub(crate) language: Language,
    pub(crate) voices: Vec<VoiceUi>,
    /// Kokoro voice id (e.g. `bm_george`) used for speech.
    pub(crate) voice: String,
    pub(crate) speed: f32,
    /// Pitch shift in semitones (0 = none, negative = deeper).
    pub(crate) pitch: f32,
    /// Style index 0 = auto; 1..=`STYLE_MAX` = fixed style pick.
    pub(crate) style: f32,
    /// Silence between paragraphs (seconds).
    pub(crate) paragraph_pause: f32,
    /// Loudness target in LUFS (used when loudnorm is on).
    pub(crate) loudness: f32,
    pub(crate) format: FormatUi,
    /// Bitrate for Opus / MP3 / M4B; ignored for WAV / FLAC.
    pub(crate) bitrate: BitrateUi,
    /// When false, loudnorm is off (slider at leftmost “off” stop).
    pub(crate) loudnorm: bool,
    /// Export one file per chapter (folder named after Output) instead of one file.
    pub(crate) split_chapters: bool,
    /// Sample rate of the exported file after mastering.
    pub(crate) sample_rate: SampleRateUi,
    /// Path shown in the Source path field (filled after Open / clearable).
    pub(crate) source_path: String,
    /// Source names an existing, readable, supported file (checked in `update`, never in `view`).
    pub(crate) source_ready: bool,
    /// Page count for a Source path (`None` inside = unknown), refreshed on blur and Open.
    pub(crate) page_count: Option<(String, Option<u32>)>,
    pub(crate) out_path: String,
    /// Whether Output was left empty, filled by Start (app default), or set by the user.
    pub(crate) out_path_origin: OutPathOrigin,
    /// Page range (1-based). Empty = start/end of document. All formats (PDF accurate).
    pub(crate) page_from: String,
    pub(crate) page_to: String,
    /// Pages to exclude: comma-separated list / ranges (e.g. `5,8,90-103`). All formats.
    pub(crate) page_skip: String,
    /// Loaded document for Save (file-only GUI).
    pub(crate) doc_ready: Option<DocReady>,
    /// Invalid source path or failed Open (red field border); details go to the Log.
    pub(crate) source_error: Option<SourceError>,
    /// Avoids repeating the same Source path log lines (path + last error).
    source_path_log: PathLog,
    /// Invalid output path / not writable (red field border); details go to the Log.
    pub(crate) out_error: Option<String>,
    /// Avoids repeating the same Output path log lines (path + last error).
    out_path_log: PathLog,
    /// Invalid Pages fields (red border only on at-fault From / To / Skip boxes).
    pub(crate) pages_error: Option<PagesFieldError>,
    /// Avoids repeating the same Pages validation log lines.
    pages_log: PathLog,
    pub(crate) busy: BusyKind,
    /// Log lines: display text (with bullet) + kind for highlighter colors.
    pub(crate) log_lines: VecDeque<(String, LogKind)>,
    /// Selectable multi-line Log buffer (Ctrl+C / drag-select).
    pub(crate) log_content: text_editor::Content,
    /// Avoid re-logging the same progress label every tick.
    pub(crate) last_logged_progress: String,
    /// Worker → UI progress (read while a job is running).
    pub(crate) progress_shared: Arc<Mutex<JobProgress>>,
    pub(crate) cancel_flag: Arc<AtomicBool>,
    /// Child processes and temp files of the running job (cleaned up on window close).
    pub(crate) active_job: Arc<ActiveJob>,
    /// Which control Tab cycling is on.
    pub(crate) focus: FocusSlot,
    /// Main app window id.
    pub(crate) main_window: Option<window::Id>,
    /// Pending Start decision (replace / resume dialog); `None` = closed.
    pub(crate) start_prompt: Option<crate::types::StartPrompt>,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let f = SavedSettings::factory();
        let format = FormatUi::from_settings_key(&f.format).unwrap_or(FormatUi::Opus);
        let mut app = Self {
            language: f.language,
            voices: Self::voices_for(f.language),
            voice: f.voice.clone(),
            speed: f.speed,
            pitch: f.pitch,
            style: f.style,
            paragraph_pause: f.paragraph_pause,
            loudness: f.loudness,
            format,
            bitrate: BitrateUi::from_ffmpeg(&f.bitrate)
                .unwrap_or_else(|| format.recommended_bitrate()),
            loudnorm: f.loudnorm,
            split_chapters: f.split_chapters,
            sample_rate: SampleRateUi::from_hz(f.sample_rate)
                .unwrap_or_else(FormatUi::recommended_sample_rate),
            source_path: String::new(),
            source_ready: false,
            page_count: None,
            out_path: String::new(),
            out_path_origin: OutPathOrigin::Unset,
            page_from: String::new(),
            page_to: String::new(),
            page_skip: String::new(),
            doc_ready: None,
            source_error: None,
            source_path_log: PathLog::default(),
            out_error: None,
            out_path_log: PathLog::default(),
            pages_error: None,
            pages_log: PathLog::default(),
            busy: BusyKind::None,
            log_lines: VecDeque::new(),
            log_content: text_editor::Content::new(),
            last_logged_progress: String::new(),
            progress_shared: Arc::new(Mutex::new(JobProgress::default())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            active_job: Arc::new(ActiveJob::default()),
            focus: FocusSlot::None,
            main_window: None,
            start_prompt: None,
        };

        // Load customized prefs into memory, then delete the settings file.
        let settings_path_display = settings::settings_file_path().map_or_else(
            || "(no config path available)".into(),
            |p| p.display().to_string(),
        );
        let mut settings_ok_lines: Vec<String> = Vec::new();
        let mut settings_err_lines: Vec<String> = Vec::new();
        match settings::load() {
            Ok(Some(saved)) => {
                app.apply_saved_settings(&saved);
                settings_ok_lines.push(format!("Settings loaded from {settings_path_display}"));
                settings_ok_lines
                    .push("Click Defaults button to return to default settings.".into());
                if let Err(e) = settings::remove_settings_file() {
                    settings_err_lines.push(e);
                }
            }
            Ok(None) => {
                // Factory prefs on disk (or none): drop any leftover file quietly.
                if let Err(e) = settings::remove_settings_file() {
                    if e.contains("permission denied") {
                        settings_err_lines.push(e);
                    }
                }
            }
            Err(e) => {
                // Unreadable or corrupt: report it and start from defaults.
                settings_err_lines.push(e);
                tracing::warn!(path = %settings_path_display, "removing unusable settings file");
                if let Err(e) = settings::remove_settings_file() {
                    tracing::warn!(error = %e, "could not remove settings file");
                }
            }
        }

        app.append_log_kind(
            LogKind::System,
            format!("Document2Voice (Kokoro) GUI v{}", env!("CARGO_PKG_VERSION")),
        );
        for line in settings_ok_lines {
            app.append_log_kind(LogKind::System, line);
        }
        for line in settings_err_lines {
            app.append_log_error(format!("Settings: {line}"));
        }

        let (id, open) = window::open(main_window_settings());
        app.main_window = Some(id);
        (
            app,
            Task::batch([
                open.discard(),
                // Drop any widget iced might auto-focus on the first frame.
                unfocus_text_inputs(),
                Task::perform(
                    async {
                        tokio::task::spawn_blocking(jobs::probe_external_tools)
                            .await
                            .unwrap_or_default()
                    },
                    Message::ToolsProbed,
                ),
            ]),
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        // Single keyboard listener (ignored key events are filtered out).
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                // Log copy is handled at app level so it works when the Log editor
                // is not the focused control.
                if modifiers.command() {
                    if let Key::Character(c) = &key {
                        if c.eq_ignore_ascii_case("c") {
                            return Some(Message::LogCopy);
                        }
                    }
                }
                match key {
                    Key::Named(keyboard::key::Named::Tab) => {
                        Some(Message::FocusMove(!modifiers.shift()))
                    }
                    Key::Named(keyboard::key::Named::Enter | keyboard::key::Named::Space) => {
                        Some(Message::FocusActivate)
                    }
                    Key::Named(
                        keyboard::key::Named::ArrowRight | keyboard::key::Named::ArrowDown,
                    ) => Some(Message::FocusNudge(1)),
                    Key::Named(keyboard::key::Named::ArrowLeft | keyboard::key::Named::ArrowUp) => {
                        Some(Message::FocusNudge(-1))
                    }
                    _ => None,
                }
            }
            _ => None,
        });
        // Text boxes capture clicks before mouse_area — re-check focus after pointer down.
        let pointer = event::listen_with(|event, _status, _id| match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                Some(Message::PointerPressed)
            }
            _ => None,
        });
        let win_closed = window::close_events().map(Message::WindowClosed);
        if self.is_busy() {
            Subscription::batch([
                keys,
                pointer,
                win_closed,
                iced::time::every(Duration::from_millis(80)).map(|_| Message::TickProgress),
            ])
        } else {
            Subscription::batch([keys, pointer, win_closed])
        }
    }
}

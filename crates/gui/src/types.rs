//! Small types used by the GUI (jobs, dropdowns, page errors, and so on).

use crate::jobs::ActiveJob;
use document2voice_core::ExportFormat;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct JobProgress {
    /// How far along: 0.0 = just started, 1.0 = finished.
    pub(crate) fraction: f32,
    pub(crate) label: String,
    /// One-off Log lines from a running job (drained by the progress tick), so they
    /// are not lost when the next progress label arrives within one tick.
    pub(crate) notes: Vec<String>,
}

/// Voice settings plus progress and cancel flags, shared by full export and short preview jobs.
pub(crate) struct SpeechParams {
    pub(crate) text: String,
    pub(crate) voice: String,
    pub(crate) speed: f32,
    pub(crate) style_index: Option<usize>,
    pub(crate) paragraph_pause: f32,
    pub(crate) pitch: f32,
    pub(crate) loudnorm: bool,
    pub(crate) loudness: f32,
    pub(crate) progress: Arc<Mutex<JobProgress>>,
    pub(crate) cancel: Arc<AtomicBool>,
    /// Where the job registers child processes and temp files.
    pub(crate) active: Arc<ActiveJob>,
}

/// Who put the value currently shown in the Output field.
///
/// Drives conflict handling on Start:
/// - [`Self::AppDefault`] — auto-number (`output-1.ext`, …) if the file exists
/// - [`Self::User`] — show the replace confirmation dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutPathOrigin {
    /// Field is empty (or was cleared).
    #[default]
    Unset,
    /// Filled by the app when Start ran with a blank Output (launch default / auto-rename).
    AppDefault,
    /// User typed a path or picked one with the save dialog.
    User,
}

/// Full “turn the book into an audio file” job.
pub(crate) struct SpeakJob {
    pub(crate) speech: SpeechParams,
    pub(crate) sample_rate: u32,
    /// Bitrate for Opus/MP3/M4B (e.g. `"48k"`). Not used for WAV/FLAC.
    pub(crate) bitrate: String,
    pub(crate) out: PathBuf,
    pub(crate) format: ExportFormat,
    /// One file per chapter in a folder named after `out`.
    pub(crate) split_chapters: bool,
}

/// The confirmation dialog shown when Start needs a decision first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartPrompt {
    /// Output file already exists.
    ReplaceFile(String),
    /// "Split by chapter" folder already holds files.
    ReplaceFolder(String),
    /// An unfinished export for this output can be continued.
    Resume {
        path: String,
        done: usize,
        total: usize,
    },
}

/// Short ~10 second preview job (make speech, then play it).
pub(crate) struct PreviewJob {
    pub(crate) speech: SpeechParams,
    /// Empty Source: fixed sample script — use a fast path and disk cache when possible.
    /// With a Source file: use the killable worker on the document text so Cancel works mid-job.
    pub(crate) sample_voice: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BusyKind {
    None,
    Saving,
    /// Short ~10 second voice preview (make speech + play it).
    Preview,
    LoadingDoc,
}

/// A source file was opened at least once (path kept for checks later).
/// Start and Preview always re-read the file with the current Pages / Skip settings.
pub(crate) struct DocReady {
    pub(crate) source_path: PathBuf,
    /// Total pages when known from last open (real pages for PDF; approximate for other formats).
    pub(crate) page_count: Option<u32>,
}

/// Pages field validation: red borders per field; `details` are Log lines under `Pages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagesFieldError {
    pub(crate) message: String,
    pub(crate) details: Vec<String>,
    pub(crate) from: bool,
    pub(crate) to: bool,
    pub(crate) skip: bool,
}

impl PagesFieldError {
    pub(crate) fn with_detail(
        message: impl Into<String>,
        from: bool,
        to: bool,
        skip: bool,
    ) -> Self {
        let message = message.into();
        Self {
            details: vec![message.clone()],
            message,
            from,
            to,
            skip,
        }
    }

    pub(crate) fn from_msg(message: impl Into<String>) -> Self {
        Self::with_detail(message, true, false, false)
    }

    pub(crate) fn to_msg(message: impl Into<String>) -> Self {
        Self::with_detail(message, false, true, false)
    }

    pub(crate) fn skip_msg(message: impl Into<String>) -> Self {
        Self::with_detail(message, false, false, true)
    }

    pub(crate) fn from_parts(
        details: Vec<String>,
        from: bool,
        to: bool,
        skip: bool,
    ) -> Option<Self> {
        if details.is_empty() {
            return None;
        }
        let message = details.join("\n");
        Some(Self {
            message,
            details,
            from,
            to,
            skip,
        })
    }
}

/// Result of the Open file dialog (metadata only; body re-read on Start/Preview).
#[derive(Debug, Clone)]
pub(crate) enum DocLoadOutcome {
    Ready {
        source_path: PathBuf,
        path_display: String,
        kind_label: String,
        file_name: String,
        detail: String,
        word_count: usize,
        page_count: Option<u32>,
        /// Detected chapter titles (empty = none).
        chapters: Vec<String>,
    },
    Cancelled,
    Failed {
        file_name: Option<String>,
        path_display: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatUi {
    Opus,
    Mp3,
    M4b,
    Wav,
    Flac,
}

impl FormatUi {
    pub(crate) const ALL: [FormatUi; 5] = [Self::Opus, Self::Mp3, Self::M4b, Self::Wav, Self::Flac];

    pub(crate) fn as_export(self) -> ExportFormat {
        match self {
            Self::Opus => ExportFormat::Opus,
            Self::Mp3 => ExportFormat::Mp3,
            Self::Wav => ExportFormat::Wav,
            Self::Flac => ExportFormat::Flac,
            Self::M4b => ExportFormat::M4b,
        }
    }

    pub(crate) fn as_settings_key(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::M4b => "m4b",
        }
    }

    pub(crate) fn from_settings_key(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "opus" => Some(Self::Opus),
            "mp3" => Some(Self::Mp3),
            "wav" => Some(Self::Wav),
            "flac" => Some(Self::Flac),
            "m4b" => Some(Self::M4b),
            _ => None,
        }
    }

    /// True when bitrate matters (compressed formats only: Opus, MP3, M4B).
    pub(crate) fn supports_bitrate(self) -> bool {
        matches!(self, Self::Opus | Self::Mp3 | Self::M4b)
    }

    /// Kokoro’s native rate (24 kHz). Higher rates only resample the audio.
    pub(crate) fn recommended_sample_rate() -> SampleRateUi {
        SampleRateUi::Hz24000
    }

    /// Suggested bitrate for this format (not used for WAV/FLAC in the UI).
    /// Opus: 64 kbps mono speech. MP3: 192 kbps (common audiobook quality floor).
    pub(crate) fn recommended_bitrate(self) -> BitrateUi {
        match self {
            Self::Mp3 => BitrateUi::K192,
            // Opus default; WAV/FLAC ignore bitrate in the UI but keep a sensible value stored.
            Self::Opus | Self::M4b | Self::Wav | Self::Flac => BitrateUi::K64,
        }
    }
}

impl std::fmt::Display for FormatUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opus => write!(f, "Opus"),
            Self::Mp3 => write!(f, "MP3"),
            Self::Wav => write!(f, "WAV"),
            Self::Flac => write!(f, "FLAC"),
            Self::M4b => write!(f, "M4B (audiobook)"),
        }
    }
}

/// Bitrate for compressed export (Opus / MP3 / M4B). Disabled for WAV / FLAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BitrateUi {
    K24,
    K32,
    K48,
    K64,
    K96,
    K128,
    K192,
    K256,
    K320,
}

impl BitrateUi {
    pub(crate) const ALL: [BitrateUi; 9] = [
        Self::K24,
        Self::K32,
        Self::K48,
        Self::K64,
        Self::K96,
        Self::K128,
        Self::K192,
        Self::K256,
        Self::K320,
    ];

    /// (this option, value passed to ffmpeg’s `-b:a`, label shown in the UI)
    const TABLE: [(Self, &'static str, &'static str); 9] = [
        (Self::K24, "24k", "24 kbps"),
        (Self::K32, "32k", "32 kbps"),
        (Self::K48, "48k", "48 kbps"),
        (Self::K64, "64k", "64 kbps"),
        (Self::K96, "96k", "96 kbps"),
        (Self::K128, "128k", "128 kbps"),
        (Self::K192, "192k", "192 kbps"),
        (Self::K256, "256k", "256 kbps"),
        (Self::K320, "320k", "320 kbps"),
    ];

    /// Value for ffmpeg’s bitrate flag, e.g. `"48k"`.
    pub(crate) fn as_ffmpeg(self) -> &'static str {
        Self::TABLE
            .iter()
            .find(|(b, _, _)| *b == self)
            .map_or("64k", |(_, ff, _)| *ff)
    }

    pub(crate) fn from_ffmpeg(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::TABLE
            .iter()
            .find(|(_, ff, _)| *ff == s)
            .map(|(b, _, _)| *b)
    }

    fn label(self) -> &'static str {
        Self::TABLE
            .iter()
            .find(|(b, _, _)| *b == self)
            .map_or("64 kbps", |(_, _, label)| *label)
    }
}

impl std::fmt::Display for BitrateUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// How many samples per second in the exported file. Kokoro makes audio at 24 kHz natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleRateUi {
    Hz24000,
    Hz44100,
    Hz48000,
}

impl SampleRateUi {
    pub(crate) const ALL: [SampleRateUi; 3] = [Self::Hz24000, Self::Hz44100, Self::Hz48000];

    pub(crate) fn hz(self) -> u32 {
        match self {
            Self::Hz24000 => 24_000,
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
        }
    }

    pub(crate) fn from_hz(hz: u32) -> Option<Self> {
        match hz {
            24_000 => Some(Self::Hz24000),
            44_100 => Some(Self::Hz44100),
            48_000 => Some(Self::Hz48000),
            _ => None,
        }
    }
}

impl std::fmt::Display for SampleRateUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hz24000 => write!(f, "24 kHz"),
            Self::Hz44100 => write!(f, "44.1 kHz"),
            Self::Hz48000 => write!(f, "48 kHz"),
        }
    }
}

/// One voice in the dropdown: keeps the Kokoro id, shows a clean capital name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceUi {
    pub(crate) id: String,
}

impl VoiceUi {
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    /// Drop the language prefix (`bm_`, `am_`, …) and capitalize the name for display.
    pub(crate) fn display_name(id: &str) -> String {
        let name = id.rsplit_once('_').map_or(id, |(_, rest)| rest);
        let mut chars = name.chars();
        match chars.next() {
            Some(c) => {
                let mut s: String = c.to_uppercase().collect();
                s.push_str(chars.as_str());
                s
            }
            None => String::new(),
        }
    }

    pub(crate) fn label(&self) -> String {
        Self::display_name(&self.id)
    }
}

impl std::fmt::Display for VoiceUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

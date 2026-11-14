/// Settings that control how speech is generated (voice, speed, pauses).
#[derive(Debug, Clone)]
pub struct SpeakOptions {
    /// Which Kokoro voice to use, e.g. `bm_george` or `am_fenrir`.
    pub voice: String,
    /// How fast to speak (1.0 = normal). About 0.88 is roughly 12% slower.
    pub speed: f32,
    /// Optional “style” number that slightly changes how the voice sounds.
    /// `None` means the engine picks automatically. Soft range is about 0–500.
    pub style_index: Option<usize>,
    /// Quiet gap between blank-line paragraphs, in seconds.
    pub paragraph_pause_secs: f32,
}

impl Default for SpeakOptions {
    fn default() -> Self {
        Self {
            voice: "am_adam".into(),
            speed: 1.0,
            style_index: None,
            paragraph_pause_secs: 0.45,
        }
    }
}

/// Container and codec of the finished audiobook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    /// Opus audio — small files, good for long books (the recommended default).
    #[default]
    Opus,
    Mp3,
    Wav,
    Flac,
    /// AAC audio in an MP4 audiobook container, with chapters (Apple Books, most players).
    M4b,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::M4b => "m4b",
        }
    }

    /// Parse a format name or file extension (`opus`, `ogg`, `mp3`, `m4b`, `m4a`, …).
    pub fn from_name(name: &str) -> Option<Self> {
        match name
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "opus" | "ogg" => Some(Self::Opus),
            "mp3" => Some(Self::Mp3),
            "wav" => Some(Self::Wav),
            "flac" => Some(Self::Flac),
            "m4b" | "m4a" | "aac" => Some(Self::M4b),
            _ => None,
        }
    }

    /// Whether the file can carry chapter markers (all but WAV).
    pub fn supports_chapters(self) -> bool {
        !matches!(self, Self::Wav)
    }
}

/// Settings for turning raw speech into a finished audio file (format, loudness, pitch).
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// ffmpeg bitrate for lossy formats (e.g. `"64k"`); ignored for WAV and FLAC.
    pub opus_bitrate: String,
    /// Apply ffmpeg's loudnorm filter.
    pub loudnorm: bool,
    /// Integrated loudness target for loudnorm, in LUFS.
    pub loudnorm_i: f32,
    /// Pitch shift in semitones (negative = deeper); applied with ffmpeg.
    pub pitch_semitones: f32,
    /// Output sample rate in Hz (Kokoro produces 24,000 Hz).
    pub sample_rate: u32,
    /// Write one file per chapter into a folder named after the output file, instead
    /// of one file with chapter markers. Ignored when there are fewer than 2 chapters.
    pub split_chapters: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Opus,
            opus_bitrate: "64k".into(),
            loudnorm: true,
            loudnorm_i: -18.0,
            pitch_semitones: 0.0,
            sample_rate: 24_000,
            split_chapters: false,
        }
    }
}

//! Speech and audio preferences kept between runs (not Source/Output paths or pages).
//!
//! Stored at `$XDG_CONFIG_HOME/document2voice/settings.json`. The file is read and
//! deleted on launch, and written on close only when prefs differ from factory
//! defaults. Disk errors are logged and the app keeps working from memory.

use crate::log_hl::LogKind;
use crate::paths::is_permission_io;
use crate::types::{BitrateUi, FormatUi, OutPathOrigin, SampleRateUi};
use crate::{
    App, LOUD_MAX, LOUD_MIN, PAUSE_MAX, PAUSE_MIN, PITCH_MAX, PITCH_MIN, SPEED_MAX, SPEED_MIN,
    STYLE_MAX, STYLE_MIN,
};
use document2voice_core::Language;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.json";
const APP_QUALIFIER: &str = "local";
const APP_ORG: &str = "document2voice";
const APP_NAME: &str = "document2voice";

/// Speech and audio prefs restored the next time you launch the app.
/// Missing fields fall back to the factory value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSettings {
    pub language: Language,
    pub voice: String,
    pub speed: f32,
    pub style: f32,
    pub pitch: f32,
    pub paragraph_pause: f32,
    /// File type: `opus`, `mp3`, `m4b`, `wav`, or `flac`
    pub format: String,
    /// e.g. `64k`, `192k` (used when the format compresses audio)
    pub bitrate: String,
    pub sample_rate: u32,
    pub loudnorm: bool,
    pub loudness: f32,
    /// One file per chapter instead of one file with chapter markers.
    pub split_chapters: bool,
}

impl Default for SavedSettings {
    fn default() -> Self {
        Self::factory()
    }
}

impl SavedSettings {
    /// Built-in factory prefs (same as the GUI Defaults button).
    pub fn factory() -> Self {
        Self {
            language: Language::EnglishAmerican,
            voice: Language::EnglishAmerican.default_voice().to_string(),
            speed: 1.0,
            style: 0.0,
            pitch: 0.0,
            paragraph_pause: 0.45,
            format: "opus".into(),
            bitrate: "64k".into(),
            sample_rate: 24_000,
            loudnorm: true,
            loudness: -18.0,
            split_chapters: false,
        }
    }

    /// True when this matches factory defaults (floating numbers compared with a tiny tolerance).
    pub fn is_factory(&self) -> bool {
        let f = Self::factory();
        let near = |a: f32, b: f32| (a - b).abs() < 0.001;
        self.language == f.language
            && self.voice == f.voice
            && near(self.speed, f.speed)
            && near(self.style, f.style)
            && near(self.pitch, f.pitch)
            && near(self.paragraph_pause, f.paragraph_pause)
            && self.format == f.format
            && self.bitrate == f.bitrate
            && self.sample_rate == f.sample_rate
            && self.loudnorm == f.loudnorm
            && near(self.loudness, f.loudness)
            && self.split_chapters == f.split_chapters
    }
}

fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .map(|d| d.config_dir().to_path_buf())
}

/// Where settings are stored (also shown in the Log).
pub fn settings_file_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(SETTINGS_FILE))
}

/// True if we can create files under `dir` (creates the folder if missing when allowed).
fn ensure_dir_writable(dir: &Path) -> Result<(), io::Error> {
    match fs::create_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }

    crate::paths::write_probe(dir)
}

fn path_readable(path: &Path) -> Result<(), io::Error> {
    fs::File::open(path).map(|_| ())
}

/// Load customized settings if the file is present and readable.
///
/// - Missing file → `Ok(None)` (use defaults)
/// - Permission / read / parse problems → `Err` with a short message (caller may log it)
/// - File equals factory defaults → `Ok(None)`
pub fn load() -> Result<Option<SavedSettings>, String> {
    let Some(path) = settings_file_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }

    if let Err(e) = path_readable(&path) {
        let msg = if is_permission_io(&e) {
            format!(
                "cannot read settings (permission denied): {}",
                path.display()
            )
        } else {
            format!("cannot read settings ({}): {}", e, path.display())
        };
        tracing::warn!("{msg}");
        return Err(msg);
    }

    let raw = fs::read_to_string(&path).map_err(|e| {
        let msg = if is_permission_io(&e) {
            format!(
                "cannot read settings (permission denied): {}",
                path.display()
            )
        } else {
            format!("cannot read settings ({}): {}", e, path.display())
        };
        tracing::warn!("{msg}");
        msg
    })?;

    match serde_json::from_str::<SavedSettings>(&raw) {
        Ok(s) => {
            if s.is_factory() {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
        Err(e) => {
            let msg = format!(
                "settings.json is invalid ({e}) — using defaults · {}",
                path.display()
            );
            tracing::warn!("{msg}");
            Err(msg)
        }
    }
}

/// Write settings.json (creates the config folder). Caller must not pass factory prefs.
pub fn save(settings: &SavedSettings) -> Result<(), String> {
    debug_assert!(!settings.is_factory());
    let Some(dir) = config_dir() else {
        let msg = "no config directory available for this user".to_string();
        tracing::warn!("{msg}");
        return Err(msg);
    };

    if let Err(e) = ensure_dir_writable(&dir) {
        let msg = if is_permission_io(&e) {
            format!(
                "cannot write settings (permission denied): {}",
                dir.display()
            )
        } else {
            format!("cannot write settings ({}): {}", e, dir.display())
        };
        tracing::warn!("{msg}");
        return Err(msg);
    }

    let path = dir.join(SETTINGS_FILE);
    let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;

    fs::write(&tmp, json.as_bytes()).map_err(|e| {
        let msg = if is_permission_io(&e) {
            format!(
                "cannot write settings (permission denied): {}",
                tmp.display()
            )
        } else {
            format!("cannot write settings ({}): {}", e, tmp.display())
        };
        tracing::warn!("{msg}");
        let _ = fs::remove_file(&tmp);
        msg
    })?;

    // Prefer rename so the final file appears in one step; if that fails, write directly.
    if let Err(e) = fs::rename(&tmp, &path) {
        if is_permission_io(&e) {
            let _ = fs::remove_file(&tmp);
            let msg = format!(
                "cannot install settings (permission denied): {}",
                path.display()
            );
            tracing::warn!("{msg}");
            return Err(msg);
        }
        // Some restricted disks disallow rename; try writing the final path directly.
        if let Err(e2) = fs::write(&path, json.as_bytes()) {
            let _ = fs::remove_file(&tmp);
            let msg = if is_permission_io(&e2) {
                format!(
                    "cannot write settings (permission denied): {}",
                    path.display()
                )
            } else {
                format!("cannot write settings ({}): {}", e2, path.display())
            };
            tracing::warn!("{msg}");
            return Err(msg);
        }
        let _ = fs::remove_file(&tmp);
    }

    tracing::info!(path = %path.display(), "wrote settings for next launch");
    Ok(())
}

/// Delete `settings.json` (and a leftover temp file), then the config folder if it is empty.
pub fn remove_settings_file() -> Result<(), String> {
    let Some(dir) = config_dir() else {
        return Ok(());
    };
    let path = dir.join(SETTINGS_FILE);
    let _ = fs::remove_file(dir.join(format!("{SETTINGS_FILE}.tmp")));
    match fs::remove_file(&path) {
        Ok(()) => {
            tracing::debug!(path = %path.display(), "removed settings file");
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            let msg = if is_permission_io(&e) {
                format!(
                    "cannot remove settings (permission denied): {}",
                    path.display()
                )
            } else {
                format!("cannot remove settings ({}): {}", e, path.display())
            };
            tracing::warn!("{msg}");
            return Err(msg);
        }
    }
    // Fails harmlessly when the folder is missing or holds other files.
    let _ = fs::remove_dir(&dir);
    Ok(())
}

/// On exit: save non-default prefs, or make sure no settings file remains when using defaults.
pub fn persist_on_exit(settings: &SavedSettings) -> Result<(), String> {
    if settings.is_factory() {
        remove_settings_file()
    } else {
        save(settings)
    }
}

impl App {
    /// Snapshot speech/audio prefs for disk (no Source/Output/pages).
    pub(crate) fn capture_settings(&self) -> SavedSettings {
        SavedSettings {
            language: self.language,
            voice: self.voice.clone(),
            speed: self.speed,
            style: self.style,
            pitch: self.pitch,
            paragraph_pause: self.paragraph_pause,
            format: self.format.as_settings_key().to_string(),
            bitrate: self.bitrate.as_ffmpeg().to_string(),
            sample_rate: self.sample_rate.hz(),
            loudnorm: self.loudnorm,
            loudness: self.loudness,
            split_chapters: self.split_chapters,
        }
    }

    /// Restore speech/audio prefs (clamped to the control ranges).
    pub(crate) fn apply_saved_settings(&mut self, s: &SavedSettings) {
        self.apply_language(s.language);
        if self.voices.iter().any(|v| v.id == s.voice) {
            self.voice.clone_from(&s.voice);
        }
        self.speed = s.speed.clamp(SPEED_MIN, SPEED_MAX);
        self.style = s.style.clamp(STYLE_MIN, STYLE_MAX);
        self.pitch = s.pitch.clamp(PITCH_MIN, PITCH_MAX);
        self.paragraph_pause = s.paragraph_pause.clamp(PAUSE_MIN, PAUSE_MAX);
        if let Some(fmt) = FormatUi::from_settings_key(&s.format) {
            self.format = fmt;
        }
        self.bitrate =
            BitrateUi::from_ffmpeg(&s.bitrate).unwrap_or_else(|| self.format.recommended_bitrate());
        self.sample_rate =
            SampleRateUi::from_hz(s.sample_rate).unwrap_or_else(FormatUi::recommended_sample_rate);
        self.loudnorm = s.loudnorm;
        self.split_chapters = s.split_chapters;
        if s.loudnorm {
            self.loudness = s.loudness.clamp(LOUD_MIN, LOUD_MAX);
        }
        self.clamp_focus();
    }

    /// Write or remove settings on exit (no disk writes while running).
    pub(crate) fn persist_settings_on_exit(&self) {
        if let Err(e) = persist_on_exit(&self.capture_settings()) {
            // The window is gone, so tracing is the only record.
            tracing::warn!(error = %e, "failed to persist settings on exit");
        }
    }

    /// True when any control differs from factory defaults (enables **Defaults**).
    pub(crate) fn settings_dirty(&self) -> bool {
        !self.capture_settings().is_factory()
            || !self.out_path.trim().is_empty()
            || !self.source_path.trim().is_empty()
            || self.using_page_filters()
    }

    /// Defaults button: factory speech/audio prefs, empty paths and pages.
    pub(crate) fn reset_defaults(&mut self) {
        self.apply_saved_settings(&SavedSettings::factory());
        self.source_path.clear();
        self.source_ready = false;
        self.page_count = None;
        self.out_path.clear();
        self.out_path_origin = OutPathOrigin::Unset;
        self.start_prompt = None;
        self.doc_ready = None;
        self.source_error = None;
        self.source_path_log.clear();
        self.out_error = None;
        self.out_path_log.clear();
        self.clear_pages_fields();
        self.clear_progress_ui();
        self.clamp_focus();
        self.append_log_kind(LogKind::Action, "Defaults restored");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_factory_values() {
        let s: SavedSettings = serde_json::from_str(r#"{"speed": 1.2}"#).unwrap();
        assert!((s.speed - 1.2).abs() < 1e-6);
        assert_eq!(s.format, SavedSettings::factory().format);
        assert!(!s.is_factory());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let s: SavedSettings = serde_json::from_str(r#"{"version": 1}"#).unwrap();
        assert!(s.is_factory());
    }
}

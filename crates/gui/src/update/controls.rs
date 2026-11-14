//! Edits to speech, audio, and path fields.

use crate::focus::{unfocus_text_inputs, FocusSlot};
use crate::types::{FormatUi, OutPathOrigin, VoiceUi};
use crate::{App, Message, LOUD_MAX, LOUD_MIN, LOUD_OFF, STYLE_MAX};
use document2voice_core::{voices_for_language, Language};
use iced::Task;

impl App {
    pub(super) fn handle_language_selected(
        &mut self,
        lang: document2voice_core::Language,
    ) -> Task<Message> {
        self.set_focus(FocusSlot::Language);
        self.apply_language(lang);
        unfocus_text_inputs()
    }

    pub(super) fn handle_voice_selected(&mut self, v: crate::types::VoiceUi) -> Task<Message> {
        self.set_focus(FocusSlot::Voice);
        self.voice = v.id;
        unfocus_text_inputs()
    }

    pub(super) fn handle_speed_changed(&mut self, s: f32) -> Task<Message> {
        self.set_focus(FocusSlot::Speed);
        self.speed = s;
        unfocus_text_inputs()
    }

    pub(super) fn handle_pitch_changed(&mut self, p: f32) -> Task<Message> {
        self.set_focus(FocusSlot::Pitch);
        self.pitch = p;
        unfocus_text_inputs()
    }

    pub(super) fn handle_style_changed(&mut self, s: f32) -> Task<Message> {
        self.set_focus(FocusSlot::Style);
        self.style = s;
        unfocus_text_inputs()
    }

    pub(super) fn handle_pause_changed(&mut self, p: f32) -> Task<Message> {
        self.set_focus(FocusSlot::Pause);
        self.paragraph_pause = p;
        unfocus_text_inputs()
    }

    pub(super) fn handle_loudness_changed(&mut self, l: f32) -> Task<Message> {
        self.set_focus(FocusSlot::Loudness);
        self.apply_loud_slider(l);
        unfocus_text_inputs()
    }

    pub(super) fn handle_format_selected(&mut self, f: crate::types::FormatUi) -> Task<Message> {
        self.set_focus(FocusSlot::Format);
        self.apply_format_audio_defaults(f);
        self.sync_output_extension_to_format();
        unfocus_text_inputs()
    }

    pub(super) fn handle_bitrate_selected(&mut self, b: crate::types::BitrateUi) -> Task<Message> {
        if !self.format.supports_bitrate() {
            return Task::none();
        }
        self.set_focus(FocusSlot::Bitrate);
        self.bitrate = b;
        unfocus_text_inputs()
    }

    pub(super) fn handle_sample_rate_selected(
        &mut self,
        r: crate::types::SampleRateUi,
    ) -> Task<Message> {
        self.set_focus(FocusSlot::SampleRate);
        self.sample_rate = r;
        unfocus_text_inputs()
    }

    pub(super) fn handle_out_path_changed(&mut self, s: String) -> Task<Message> {
        // Do not re-focus: that would jump the caret to the end of the line.
        self.focus = FocusSlot::OutPath;
        self.out_path = s;
        self.out_path_origin = if self.out_path.trim().is_empty() {
            OutPathOrigin::Unset
        } else {
            OutPathOrigin::User
        };
        // Editing Output dismisses a pending replace dialog.
        self.start_prompt = None;
        self.clear_out_error();
        Task::none()
    }

    pub(super) fn handle_source_path_changed(&mut self, s: String) -> Task<Message> {
        self.focus = FocusSlot::SourcePath;
        self.source_path = s;
        self.clear_source_error();
        self.refresh_source_ready();
        // Page count is re-read when the field is left.
        self.page_count = None;
        // Changing Source clears the sticky 100% “Done” progress bar.
        self.clear_progress_ui();
        if !self.pages_fields_enabled() {
            // Source cleared → Pages/Skip disabled; drop their values so they
            // do not reappear (stale From/To/Skip) when a new file is opened.
            self.clear_pages_fields();
            self.doc_ready = None;
            self.clamp_focus();
        }
        Task::none()
    }

    pub(super) fn handle_page_from_changed(&mut self, s: String) -> Task<Message> {
        if !self.pages_fields_enabled() {
            return Task::none();
        }
        self.focus = FocusSlot::PageFrom;
        self.page_from = s;
        // Clear red border while typing; full check runs when you leave the field.
        self.clear_pages_error();
        Task::none()
    }

    pub(super) fn handle_page_to_changed(&mut self, s: String) -> Task<Message> {
        if !self.pages_fields_enabled() {
            return Task::none();
        }
        self.focus = FocusSlot::PageTo;
        self.page_to = s;
        self.clear_pages_error();
        Task::none()
    }

    pub(super) fn handle_page_skip_changed(&mut self, s: String) -> Task<Message> {
        if !self.pages_fields_enabled() {
            return Task::none();
        }
        self.focus = FocusSlot::PageSkip;
        self.page_skip = s;
        self.clear_pages_error();
        Task::none()
    }

    pub(crate) fn voices_for(lang: Language) -> Vec<VoiceUi> {
        let mut list: Vec<VoiceUi> = voices_for_language(lang)
            .into_iter()
            .map(|v| VoiceUi::new(v.id))
            .collect();
        // Alphabetical by display name (George, not bm_george).
        list.sort_by_key(|v| v.label().to_ascii_lowercase());
        list
    }

    pub(crate) fn apply_language(&mut self, lang: Language) {
        self.language = lang;
        self.voices = Self::voices_for(lang);
        // Keep current voice if still valid for this language; else use the default.
        if !self.voices.iter().any(|v| v.id == self.voice) {
            self.voice = lang.default_voice().to_string();
        }
    }

    pub(crate) fn selected_voice(&self) -> Option<VoiceUi> {
        self.voices
            .iter()
            .find(|v| v.id == self.voice)
            .cloned()
            .or_else(|| Some(VoiceUi::new(self.voice.clone())))
    }

    /// Slider thumb position: leftmost (`LOUD_OFF`) when off, else LUFS.
    pub(crate) fn loud_slider_pos(&self) -> f32 {
        if self.loudnorm {
            self.loudness.clamp(LOUD_MIN, LOUD_MAX)
        } else {
            LOUD_OFF
        }
    }

    /// Apply loudnorm slider: leftmost = off; otherwise LUFS −24…−12.
    pub(crate) fn apply_loud_slider(&mut self, v: f32) {
        if v < LOUD_MIN {
            self.loudnorm = false;
        } else {
            self.loudnorm = true;
            self.loudness = v.clamp(LOUD_MIN, LOUD_MAX);
        }
    }

    /// Apply recommended sample rate (and bitrate when compressed) for `format`.
    pub(crate) fn apply_format_audio_defaults(&mut self, format: FormatUi) {
        self.format = format;
        self.sample_rate = FormatUi::recommended_sample_rate();
        if format.supports_bitrate() {
            self.bitrate = format.recommended_bitrate();
        }
        self.clamp_focus();
    }

    /// Fixed style index, or `None` for auto (slider at 0).
    pub(crate) fn style_index_opt(&self) -> Option<usize> {
        if self.style < 0.5 {
            None
        } else {
            Some(self.style.round().clamp(0.0, STYLE_MAX) as usize)
        }
    }
}

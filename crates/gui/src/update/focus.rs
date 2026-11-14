//! Keyboard and pointer focus, plus arrow-key nudges on sliders and dropdowns.

use crate::focus::{
    apply_focus_slot, FocusSlot, OUT_PATH_ID, PAGE_FROM_ID, PAGE_SKIP_ID, PAGE_TO_ID,
    SOURCE_PATH_ID,
};
use crate::types::{BitrateUi, FormatUi, SampleRateUi};
use crate::{
    App, Message, LOUD_MAX, LOUD_OFF, PAUSE_MAX, PAUSE_MIN, PITCH_MAX, PITCH_MIN, SPEED_MAX,
    SPEED_MIN, STYLE_MAX, STYLE_MIN,
};
use document2voice_core::Language;
use iced::widget::{self as widget};
use iced::Task;

impl App {
    pub(super) fn handle_focus_move(&mut self, forward: bool) -> Task<Message> {
        let next = self.focus.next(
            forward,
            self.format.supports_bitrate(),
            self.pages_fields_enabled(),
        );
        self.set_focus(next);
        apply_focus_slot(self.focus)
    }

    pub(super) fn handle_focus_take(&mut self, slot: FocusSlot) -> Task<Message> {
        // Ignore clicks on Pages/Skip while Source is empty.
        if slot.is_page() && !self.pages_fields_enabled() {
            return Task::none();
        }
        self.set_focus(slot);
        apply_focus_slot(self.focus)
    }

    pub(super) fn handle_pointer_pressed(&mut self) -> Task<Message> {
        // Text boxes capture clicks first; ask iced which path/page field is focused.
        widget::operation::is_focused(widget::Id::new(PAGE_FROM_ID)).then(|from| {
            widget::operation::is_focused(widget::Id::new(PAGE_TO_ID)).then(move |to| {
                widget::operation::is_focused(widget::Id::new(PAGE_SKIP_ID)).then(move |skip| {
                    widget::operation::is_focused(widget::Id::new(OUT_PATH_ID)).then(
                        move |out_path| {
                            widget::operation::is_focused(widget::Id::new(SOURCE_PATH_ID)).map(
                                move |source_path| {
                                    let slot = if from {
                                        Some(FocusSlot::PageFrom)
                                    } else if to {
                                        Some(FocusSlot::PageTo)
                                    } else if skip {
                                        Some(FocusSlot::PageSkip)
                                    } else if out_path {
                                        Some(FocusSlot::OutPath)
                                    } else if source_path {
                                        Some(FocusSlot::SourcePath)
                                    } else {
                                        None
                                    };
                                    Message::PageTextFocus(slot)
                                },
                            )
                        },
                    )
                })
            })
        })
    }

    pub(super) fn handle_page_text_focus(&mut self, focused: Option<FocusSlot>) -> Task<Message> {
        // After a click, match our focus to the text box; leaving a field runs path checks.
        if let Some(slot) = focused {
            self.set_focus(slot);
        } else if self.focus.text_id().is_some() {
            self.set_focus(FocusSlot::OpenFile);
        }
        Task::none()
    }

    pub(super) fn handle_focus_activate(&mut self) -> Task<Message> {
        if self.is_busy() && !matches!(self.focus, FocusSlot::Save | FocusSlot::Preview) {
            return Task::none();
        }
        match self.focus {
            FocusSlot::Browse => self.update(Message::BrowseOut),
            FocusSlot::SplitChapters => {
                self.update(Message::SplitChaptersToggled(!self.split_chapters))
            }
            FocusSlot::OpenFile => self.update(Message::OpenFile),
            FocusSlot::Defaults => {
                if self.settings_dirty() && !self.is_busy() {
                    self.update(Message::ResetDefaults)
                } else {
                    Task::none()
                }
            }
            FocusSlot::Preview => {
                if self.can_preview() {
                    self.update(Message::Preview)
                } else {
                    Task::none()
                }
            }
            FocusSlot::Save => {
                // Same rules as the Start button (Source / Pages / Output).
                if self.can_start() {
                    self.update(Message::Speak)
                } else {
                    Task::none()
                }
            }
            _ => Task::none(),
        }
    }

    pub(super) fn handle_focus_nudge(&mut self, dir: i32) -> Task<Message> {
        if self.focus.text_id().is_some() {
            return Task::none();
        }
        let nudge = dir as f32;
        let forward = dir > 0;
        match self.focus {
            FocusSlot::Language => {
                let all = Language::ALL;
                let i = all.iter().position(|&l| l == self.language).unwrap_or(0);
                self.apply_language(all[cycle_index(i, all.len(), forward)]);
            }
            FocusSlot::Voice => {
                if self.voices.is_empty() {
                    return Task::none();
                }
                let i = self
                    .voices
                    .iter()
                    .position(|v| v.id == self.voice)
                    .unwrap_or(0);
                let j = cycle_index(i, self.voices.len(), forward);
                self.voice.clone_from(&self.voices[j].id);
            }
            FocusSlot::Format => {
                let all = FormatUi::ALL;
                let i = all.iter().position(|&f| f == self.format).unwrap_or(0);
                self.apply_format_audio_defaults(all[cycle_index(i, all.len(), forward)]);
                self.sync_output_extension_to_format();
            }
            FocusSlot::SplitChapters => self.split_chapters = forward,
            FocusSlot::Bitrate => {
                if self.format.supports_bitrate() {
                    let all = BitrateUi::ALL;
                    let i = all.iter().position(|&b| b == self.bitrate).unwrap_or(0);
                    self.bitrate = all[cycle_index(i, all.len(), forward)];
                }
            }
            FocusSlot::SampleRate => {
                let all = SampleRateUi::ALL;
                let i = all.iter().position(|&r| r == self.sample_rate).unwrap_or(0);
                self.sample_rate = all[cycle_index(i, all.len(), forward)];
            }
            FocusSlot::Speed => {
                self.speed = (self.speed + nudge * 0.01).clamp(SPEED_MIN, SPEED_MAX);
            }
            FocusSlot::Pitch => {
                self.pitch = (self.pitch + nudge * 0.1).clamp(PITCH_MIN, PITCH_MAX);
            }
            FocusSlot::Style => {
                self.style = (self.style + nudge * 1.0).clamp(STYLE_MIN, STYLE_MAX);
            }
            FocusSlot::Pause => {
                self.paragraph_pause =
                    (self.paragraph_pause + nudge * 0.05).clamp(PAUSE_MIN, PAUSE_MAX);
            }
            FocusSlot::Loudness => {
                let new = (self.loud_slider_pos() + nudge * 0.5).clamp(LOUD_OFF, LOUD_MAX);
                self.apply_loud_slider(new);
            }
            _ => {}
        }
        Task::none()
    }
}

impl App {
    /// Move focus to `slot`, running leave-field checks for the field being left.
    pub(crate) fn set_focus(&mut self, slot: FocusSlot) {
        self.blur_source_path_if_needed(slot);
        self.blur_out_path_if_needed(slot);
        self.blur_pages_if_needed(slot);
        self.focus = slot;
    }

    /// Keep focus off controls that are currently disabled.
    pub(crate) fn clamp_focus(&mut self) {
        if self.focus == FocusSlot::Bitrate && !self.format.supports_bitrate() {
            self.set_focus(FocusSlot::Format);
        }
        if self.focus.is_page() && !self.pages_fields_enabled() {
            self.focus = FocusSlot::SourcePath;
        }
    }
}

fn cycle_index(i: usize, n: usize, forward: bool) -> usize {
    if n == 0 {
        return 0;
    }
    if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    }
}

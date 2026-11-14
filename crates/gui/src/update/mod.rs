//! Message handling, split into handler groups (controls, jobs, shell, focus).

mod controls;
mod focus;
mod jobs;
mod shell;

use crate::focus::{unfocus_text_inputs, FocusSlot};
use crate::{App, Message};
use iced::Task;

impl App {
    /// Send each UI event to its handler. Exhaustive, so a new variant must be wired here.
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LanguageSelected(lang) => self.handle_language_selected(lang),
            Message::VoiceSelected(v) => self.handle_voice_selected(v),
            Message::SpeedChanged(s) => self.handle_speed_changed(s),
            Message::PitchChanged(p) => self.handle_pitch_changed(p),
            Message::StyleChanged(s) => self.handle_style_changed(s),
            Message::PauseChanged(p) => self.handle_pause_changed(p),
            Message::LoudnessChanged(l) => self.handle_loudness_changed(l),
            Message::FormatSelected(f) => self.handle_format_selected(f),
            Message::SplitChaptersToggled(on) => {
                self.set_focus(FocusSlot::SplitChapters);
                self.split_chapters = on;
                unfocus_text_inputs()
            }
            Message::BitrateSelected(b) => self.handle_bitrate_selected(b),
            Message::SampleRateSelected(r) => self.handle_sample_rate_selected(r),
            Message::OutPathChanged(s) => self.handle_out_path_changed(s),
            Message::SourcePathChanged(s) => self.handle_source_path_changed(s),
            Message::PageFromChanged(s) => self.handle_page_from_changed(s),
            Message::PageToChanged(s) => self.handle_page_to_changed(s),
            Message::PageSkipChanged(s) => self.handle_page_skip_changed(s),

            Message::OpenFile => self.handle_open_file(),
            Message::DocLoaded(outcome) => self.handle_doc_loaded(outcome),
            Message::TickProgress => self.handle_tick_progress(),
            Message::CancelSave => self.handle_cancel_save(),
            Message::Speak => self.handle_speak(),
            Message::OverwriteConfirmed(yes) => self.handle_overwrite_confirmed(yes),
            Message::ResumeChosen(choice) => self.handle_resume_chosen(choice),
            Message::Preview => self.handle_preview(),
            Message::JobFinished(res) => self.handle_job_finished(res),
            Message::PreviewFinished(res) => self.handle_preview_finished(res),

            Message::ToolsProbed(tools) => {
                self.log_tools_and_ready(&tools);
                Task::none()
            }
            Message::BrowseOut => self.handle_browse_out(),
            Message::ResetDefaults => self.handle_reset_defaults(),
            Message::WindowClosed(id) => self.handle_window_closed(id),
            Message::OutPathPicked(path) => self.handle_out_path_picked(path),
            Message::LogAction(action) => {
                if !action.is_edit() {
                    self.log_content.perform(action);
                }
                Task::none()
            }
            Message::LogCopy => self
                .log_content
                .selection()
                .map_or_else(Task::none, iced::clipboard::write),
            Message::LogCopyAll => {
                // Quiet copy — no extra Log line.
                let full = self.log_content.text();
                if full.is_empty() {
                    Task::none()
                } else {
                    iced::clipboard::write(full)
                }
            }

            Message::FocusMove(forward) => self.handle_focus_move(forward),
            Message::FocusTake(slot) => self.handle_focus_take(slot),
            Message::PointerPressed => self.handle_pointer_pressed(),
            Message::PageTextFocus(slot) => self.handle_page_text_focus(slot),
            Message::FocusActivate => self.handle_focus_activate(),
            Message::FocusNudge(dir) => self.handle_focus_nudge(dir),
        }
    }
}

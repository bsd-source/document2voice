//! Window, file dialogs, Defaults button, and Log clipboard.

use crate::focus::{unfocus_text_inputs, FocusSlot};
use crate::paths::normalize_output_path;
use crate::types::OutPathOrigin;
use crate::{App, Message};
use iced::{window, Task};
use std::sync::atomic::Ordering;

impl App {
    pub(super) fn handle_browse_out(&mut self) -> Task<Message> {
        if self.ui_locked() {
            return Task::none();
        }
        self.set_focus(FocusSlot::Browse);
        let format = self.format;
        // Prefer the folder/name already in Output; else the launch base + output.{ext}.
        let trimmed = self.out_path.trim();
        let start_dir = {
            let p = std::path::Path::new(trimmed);
            p.parent()
                .filter(|d| !d.as_os_str().is_empty() && d.is_dir())
                .map(|d| d.to_path_buf())
                .or_else(|| {
                    let base = crate::paths::output_path_base_dir();
                    base.is_dir().then_some(base)
                })
        };
        let start_name = std::path::Path::new(trimmed)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        Task::batch([
            unfocus_text_inputs(),
            Task::perform(
                async move {
                    let ext = format.as_export().extension();
                    let mut dlg = rfd::AsyncFileDialog::new();
                    if let Some(dir) = start_dir {
                        dlg = dlg.set_directory(dir);
                    }
                    let name = start_name.unwrap_or_else(|| format!("output.{ext}"));
                    dlg = dlg.set_file_name(name);
                    dlg.add_filter(format!("{ext} audio"), &[ext])
                        .add_filter("All files", &["*"])
                        .save_file()
                        .await
                        .map(|f| normalize_output_path(&f.path().display().to_string(), ext))
                },
                Message::OutPathPicked,
            ),
        ])
    }

    pub(super) fn handle_reset_defaults(&mut self) -> Task<Message> {
        self.set_focus(FocusSlot::Defaults);
        if self.ui_locked() {
            return Task::none();
        }
        self.reset_defaults();
        unfocus_text_inputs()
    }

    pub(super) fn handle_window_closed(&mut self, id: window::Id) -> Task<Message> {
        if self.main_window != Some(id) {
            return Task::none();
        }
        // Stop worker / player children and delete temp files now: the job threads
        // that would otherwise do it end with the process.
        self.cancel_flag.store(true, Ordering::Release);
        self.active_job.shutdown();
        // Save prefs only if they differ from defaults; otherwise leave the disk clean.
        self.persist_settings_on_exit();
        iced::exit()
    }

    pub(super) fn handle_out_path_picked(&mut self, path: Option<String>) -> Task<Message> {
        let Some(p) = path else {
            return Task::none();
        };
        // Browse always counts as a user-chosen path (replace dialog if file exists).
        let task = self.handle_out_path_changed(p);
        self.out_path_origin = OutPathOrigin::User;
        self.apply_output_path_validation();
        task
    }
}

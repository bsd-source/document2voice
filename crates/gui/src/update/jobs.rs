//! Opening a document, Start, Preview, Cancel, and progress updates.

use crate::focus::{unfocus_text_inputs, FocusSlot};
use crate::jobs::{note, preview_job, report, speak_job, CANCELLED};
use crate::log_hl::{format_count, log_block, log_labeled_path, short_name, LogKind};
use crate::paths::{
    normalize_output_path, normalize_source_path, validate_output_path, SourceError,
};
use crate::types::{
    BusyKind, DocLoadOutcome, DocReady, JobProgress, OutPathOrigin, PreviewJob, SpeakJob,
    SpeechParams, StartPrompt,
};
use crate::{App, Message};
use document2voice_core::{extract_document, CoreError, DocExtract, ExtractOptions};
use iced::Task;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// File types the Open dialog offers by default.
const OPEN_FILTER_EXTS: &[&str] = &[
    "pdf", "epub", "txt", "text", "md", "docx", "odt", "mobi", "prc", "azw", "azw3", "kfx",
];

/// Shared “pull text from the document” step for Start and document Preview
/// (checks Cancel between wait points so the user can stop mid-way).
async fn extract_for_job(
    source_path: PathBuf,
    extract_opts: ExtractOptions,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<JobProgress>>,
) -> Result<DocExtract, String> {
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    report(&progress, 0.05, "Reading document…");
    let extracted =
        tokio::task::spawn_blocking(move || extract_document(&source_path, &extract_opts))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    // ~150 words/minute at 1.0× — helps set expectations when one page is still long.
    let mins = (extracted.word_count as f32 / 150.0).max(0.1);
    let est = if mins < 1.5 {
        format!("~{:.0}s of audio", mins * 60.0)
    } else {
        format!("~{mins:.1} min of audio")
    };
    report(
        &progress,
        0.12,
        &format!(
            "Text ready · {} · {} words · {est}",
            extracted.detail, extracted.word_count
        ),
    );
    Ok(extracted)
}

impl App {
    pub(super) fn handle_open_file(&mut self) -> Task<Message> {
        self.set_focus(FocusSlot::OpenFile);
        if self.ui_locked() {
            return unfocus_text_inputs();
        }
        // Open always loads the **whole** document (metadata + word count).
        // From/To/Skip stay as typed and are re-checked after load against the
        // new page count — they must not fail Open or paint the Source field red.
        self.busy = BusyKind::LoadingDoc;
        self.append_log_kind(LogKind::Action, "Opening file…");
        // Progress bar is for Start/Preview only — leave it idle while opening a file.
        self.clear_progress_ui();
        Task::batch([
            unfocus_text_inputs(),
            Task::perform(
                open_and_extract_document(ExtractOptions::default()),
                Message::DocLoaded,
            ),
        ])
    }

    pub(super) fn handle_doc_loaded(&mut self, outcome: DocLoadOutcome) -> Task<Message> {
        self.busy = BusyKind::None;
        match outcome {
            DocLoadOutcome::Ready {
                source_path,
                path_display,
                kind_label,
                file_name,
                detail,
                word_count,
                page_count,
                chapters,
            } => {
                self.clear_source_error();
                self.source_path = normalize_source_path(&path_display);
                self.doc_ready = Some(DocReady {
                    source_path,
                    page_count,
                });
                self.clear_progress_ui();
                self.append_log_kind(
                    LogKind::Success,
                    log_block(
                        format!("Loaded  {}", short_name(&file_name, 36)),
                        [
                            kind_label.as_str().to_string(),
                            detail,
                            format!("{} words", format_count(word_count)),
                        ],
                    ),
                );
                self.log_chapter_titles(&chapters);
                self.log_non_pdf_page_filter_warning();
                self.apply_source_path_validation();
                // Re-check start/end/skip against the new page count.
                self.apply_pages_validation();
                if let Some(e) = &self.pages_error {
                    if e.from {
                        self.set_focus(FocusSlot::PageFrom);
                    } else if e.to {
                        self.set_focus(FocusSlot::PageTo);
                    } else if e.skip {
                        self.set_focus(FocusSlot::PageSkip);
                    }
                }
                Task::none()
            }
            DocLoadOutcome::Cancelled => {
                self.append_log_kind(LogKind::Cancel, "Open cancelled");
                Task::none()
            }
            DocLoadOutcome::Failed {
                file_name,
                path_display,
                message,
            } => {
                // Open/extract failures (missing file, DRM, …) — Source red border.
                self.doc_ready = None;
                if let Some(p) = path_display {
                    self.record_source_open_failure(&p, &message);
                } else {
                    self.source_error = Some(SourceError::Load(message.clone()));
                }
                // Title and body as separate log rows so long messages wrap cleanly.
                let title = match file_name.as_ref() {
                    Some(n) if !n.is_empty() => format!("Error ({n})"),
                    _ => "Error".into(),
                };
                self.append_log_error(log_block(title, [message.as_str()]));
                Task::none()
            }
        }
    }

    pub(super) fn handle_tick_progress(&mut self) -> Task<Message> {
        if !self.is_busy() {
            return Task::none();
        }
        let Some((label, notes)) = self
            .progress_shared
            .lock()
            .ok()
            .map(|mut g| (g.label.clone(), std::mem::take(&mut g.notes)))
        else {
            return Task::none();
        };
        for n in notes {
            self.append_log_kind(LogKind::Info, n);
        }
        if !label.is_empty() && label != self.last_logged_progress {
            // Log stage names only (no percentages).
            if let Some((kind, text)) = progress_log_entry(&label) {
                self.append_log_kind(kind, text);
            }
            self.last_logged_progress = label;
        }
        Task::none()
    }

    pub(super) fn handle_cancel_save(&mut self) -> Task<Message> {
        // Ignore extra clicks while cancel is already winding down.
        if (self.is_saving() || self.is_previewing()) && !self.cancel_flag.load(Ordering::Acquire) {
            self.cancel_flag.store(true, Ordering::Release);
            self.append_log_kind(LogKind::Action, "Cancelling…");
        }
        Task::none()
    }

    pub(super) fn handle_speak(&mut self) -> Task<Message> {
        self.set_focus(FocusSlot::Save);
        let unfocus = unfocus_text_inputs();
        if self.ui_locked() {
            return unfocus;
        }
        // Same rules as the Start button (Source / Pages / Output hard errors).
        if !self.can_start() {
            return unfocus;
        }
        // Empty Output → launch default (+ auto-number if taken). User path + exists → dialog.
        if !self.ensure_output_path_for_start() {
            return unfocus;
        }
        let out = self.out_path.trim().to_string();
        // 1) An unfinished export can be continued (resume/start over both mean the
        //    output will be written, so no separate replace question).
        if let Some(p) = document2voice_core::partial_export(std::path::Path::new(&out)) {
            self.start_prompt = Some(StartPrompt::Resume {
                path: out,
                done: p.done,
                total: p.total,
            });
            return unfocus;
        }
        // 2) Split-by-chapter into a folder that already has files.
        if self.split_chapters {
            let dir = document2voice_core::split_output_dir(std::path::Path::new(&out));
            let has_files = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some());
            if has_files {
                self.start_prompt = Some(StartPrompt::ReplaceFolder(dir.display().to_string()));
                return unfocus;
            }
        } else if self.out_path_origin == OutPathOrigin::User && self.output_file_exists() {
            // 3) Exists was already logged on blur; only open the replace dialog.
            self.start_prompt = Some(StartPrompt::ReplaceFile(out));
            return unfocus;
        }
        self.start_speak_job(unfocus)
    }

    /// Chapters the export will have (first few titles, then a count).
    fn log_chapter_titles(&mut self, chapters: &[String]) {
        const SHOW: usize = 8;
        if chapters.is_empty() {
            self.append_log_kind(
                LogKind::Info,
                "No chapters found · the audio will be one section",
            );
            return;
        }
        let mut lines: Vec<String> = chapters
            .iter()
            .take(SHOW)
            .enumerate()
            .map(|(i, t)| format!("{:>2}. {}", i + 1, short_name(t, 40)))
            .collect();
        if chapters.len() > SHOW {
            lines.push(format!("… and {} more", chapters.len() - SHOW));
        }
        let head = if chapters.len() == 1 {
            "1 chapter".to_string()
        } else {
            format!("{} chapters", chapters.len())
        };
        self.append_log_kind(LogKind::Info, log_block(head, lines));
    }

    pub(super) fn handle_resume_chosen(&mut self, choice: Option<bool>) -> Task<Message> {
        if self.is_busy() {
            return Task::none();
        }
        let Some(StartPrompt::Resume { path, done, total }) = self.start_prompt.take() else {
            return Task::none();
        };
        match choice {
            None => unfocus_text_inputs(),
            Some(true) => {
                self.append_log_kind(
                    LogKind::Action,
                    format!(
                        "Resuming unfinished export · {} of {} paragraphs done",
                        format_count(done),
                        format_count(total)
                    ),
                );
                self.handle_overwrite_confirmed(true)
            }
            Some(false) => {
                if let Err(e) =
                    document2voice_core::discard_partial_export(std::path::Path::new(&path))
                {
                    self.append_log_error(format!("Could not discard the unfinished export: {e}"));
                    return unfocus_text_inputs();
                }
                self.append_log_kind(
                    LogKind::Action,
                    "Unfinished export discarded · starting over",
                );
                self.handle_overwrite_confirmed(true)
            }
        }
    }

    pub(super) fn handle_overwrite_confirmed(&mut self, overwrite: bool) -> Task<Message> {
        if self.is_busy() {
            return Task::none();
        }
        let was_file = matches!(self.start_prompt, Some(StartPrompt::ReplaceFile(_)));
        self.start_prompt = None;
        if !overwrite {
            if was_file {
                self.mark_output_exists_not_overwritten();
            }
            return unfocus_text_inputs();
        }
        if !self.can_start() {
            return unfocus_text_inputs();
        }
        // Hard checks only (permissions / folder). Existing file is OK — user chose Replace.
        let ext = self.format.as_export().extension();
        let normalized = normalize_output_path(self.out_path.trim(), ext);
        if normalized != self.out_path {
            self.out_path.clone_from(&normalized);
        }
        if let Err(e) = validate_output_path(&normalized) {
            self.out_error = Some(e.clone());
            self.out_path_log.set(normalized, Some(e.clone()));
            self.append_log_error(log_block("Output", [e.as_str()]));
            self.set_focus(FocusSlot::OutPath);
            return unfocus_text_inputs();
        }
        self.clear_out_error();
        self.out_path_log.set(normalized, None);
        self.start_speak_job(unfocus_text_inputs())
    }

    /// Shared Start pipeline after Output is finalized (and overwrite approved if needed).
    fn start_speak_job(&mut self, unfocus: Task<Message>) -> Task<Message> {
        let source_path = match self.require_source_file_path() {
            Ok(p) => p,
            Err(task) => return task,
        };
        let Ok(extract_opts) = self.extract_options_from_ui() else {
            return unfocus_text_inputs();
        };
        self.append_log_kind(
            LogKind::Action,
            log_labeled_path("Starting", &self.out_path),
        );
        self.log_page_selection();
        let speech_base = self.speech_params(String::new());
        let sample_rate = self.sample_rate.hz();
        let bitrate = self.bitrate.as_ffmpeg().to_string();
        let out = PathBuf::from(&self.out_path);
        let format = self.format.as_export();
        let split_chapters = self.split_chapters;
        let progress = Arc::clone(&self.progress_shared);
        self.begin_job(BusyKind::Saving);

        Task::batch([
            unfocus,
            Task::perform(
                async move {
                    let cancel = Arc::clone(&speech_base.cancel);
                    let progress_note = Arc::clone(&progress);
                    let extracted =
                        extract_for_job(source_path, extract_opts, cancel, progress).await?;
                    // Shows in the Log as a progress line: what the export will contain.
                    let titles = document2voice_core::chapters::chapter_titles(&extracted.text);
                    let note_line = match titles.len() {
                        0 => "No chapters found · one section".to_string(),
                        n => {
                            let first: Vec<String> =
                                titles.iter().take(3).map(|t| short_name(t, 28)).collect();
                            let more = if n > 3 {
                                format!(" … +{}", n - 3)
                            } else {
                                String::new()
                            };
                            format!("{n} chapters: {}{more}", first.join(" · "))
                        }
                    };
                    note(&progress_note, note_line);
                    let mut speech = speech_base;
                    speech.text = extracted.text;
                    let job = SpeakJob {
                        speech,
                        sample_rate,
                        bitrate,
                        out,
                        format,
                        split_chapters,
                    };
                    speak_job(job).await.map_err(|e| e.to_string())
                },
                Message::JobFinished,
            ),
        ])
    }

    pub(super) fn handle_preview(&mut self) -> Task<Message> {
        self.set_focus(FocusSlot::Preview);
        if self.ui_locked() {
            return unfocus_text_inputs();
        }
        // Same rules as the Preview button (Pages OK; Source empty or valid).
        if !self.can_preview() {
            return unfocus_text_inputs();
        }

        // Empty Source → fixed sample script (same words every time).
        // Filled Source → ~PREVIEW_SECS from the start of the selected pages (not full export).
        let sample_mode = self.source_path.trim().is_empty();
        if sample_mode {
            return self.handle_preview_sample();
        }

        let source_path = match self.require_source_file_path() {
            Ok(p) => p,
            Err(task) => return task,
        };
        // Narrow extract to the first pages of the selection — synthesis is also time-capped.
        let Ok(extract_opts) = self.extract_options_for_preview() else {
            return unfocus_text_inputs();
        };
        let speech_base = self.speech_params(String::new());
        let progress = Arc::clone(&self.progress_shared);
        self.log_preview_scope();
        self.begin_job(BusyKind::Preview);
        Task::batch([
            unfocus_text_inputs(),
            Task::perform(
                async move {
                    let cancel = Arc::clone(&speech_base.cancel);
                    let extracted =
                        extract_for_job(source_path, extract_opts, cancel, progress).await?;
                    let mut speech = speech_base;
                    speech.text = extracted.text;
                    let job = PreviewJob {
                        speech,
                        sample_voice: false,
                    };
                    preview_job(job).await.map_err(|e| e.to_string())
                },
                Message::PreviewFinished,
            ),
        ])
    }

    pub(super) fn handle_preview_sample(&mut self) -> Task<Message> {
        let speech = self.speech_params(crate::VOICE_PREVIEW_TEXT.to_string());
        self.append_log_kind(
            LogKind::Action,
            log_block(
                "Preview…",
                [format!(
                    "~{:.0}s sample voice (no source file)",
                    crate::PREVIEW_SECS
                )],
            ),
        );
        self.begin_job(BusyKind::Preview);
        Task::batch([
            unfocus_text_inputs(),
            Task::perform(
                async move {
                    let job = PreviewJob {
                        speech,
                        sample_voice: true,
                    };
                    preview_job(job).await.map_err(|e| e.to_string())
                },
                Message::PreviewFinished,
            ),
        ])
    }

    pub(super) fn handle_job_finished(
        &mut self,
        res: Result<(PathBuf, Option<String>), String>,
    ) -> Task<Message> {
        self.end_busy();
        if self.focus == FocusSlot::Save {
            self.focus = FocusSlot::None;
        }
        match res {
            Ok((p, warning)) => {
                // Sticky 100% + "Done" until next Start/Preview or Source change.
                self.last_logged_progress = "Done".into();
                self.set_progress(1.0, "Done");
                self.append_log_kind(
                    LogKind::Success,
                    log_labeled_path("Saved", &p.display().to_string()),
                );
                if let Some(w) = warning {
                    self.append_log_kind(LogKind::Warning, w);
                }
            }
            Err(e) if e == CANCELLED => {
                self.clear_progress_ui();
                self.append_log_kind(LogKind::Cancel, "Start cancelled");
            }
            Err(e) => {
                self.clear_progress_ui();
                // Message already reads as a full sentence when possible.
                let msg = e.trim();
                let line = if msg.starts_with("Nothing to process") || msg.starts_with("Unable to")
                {
                    msg.to_string()
                } else {
                    format!("Error: {msg}")
                };
                self.append_log_error(line);
            }
        }
        Task::none()
    }

    pub(super) fn handle_preview_finished(&mut self, res: Result<(), String>) -> Task<Message> {
        self.end_busy();
        if self.focus == FocusSlot::Preview {
            self.focus = FocusSlot::None;
        }
        match res {
            Ok(()) => {
                // Same sticky “Done” bar as a full export.
                self.last_logged_progress = "Done".into();
                self.set_progress(1.0, "Done");
                self.append_log_kind(LogKind::Success, "Preview finished");
            }
            Err(e) if e == CANCELLED => {
                self.clear_progress_ui();
                self.append_log_kind(LogKind::Cancel, "Preview cancelled");
            }
            Err(e) => {
                self.clear_progress_ui();
                self.append_log_error(format!("Preview: {e}"));
            }
        }
        Task::none()
    }
}

/// Open file dialog + extract document off the UI thread.
async fn open_and_extract_document(extract_opts: ExtractOptions) -> DocLoadOutcome {
    let path = rfd::AsyncFileDialog::new()
        .add_filter("Supported files", OPEN_FILTER_EXTS)
        .add_filter("All files", &["*"])
        .pick_file()
        .await
        .map(|f| f.path().to_path_buf());
    let Some(path) = path else {
        return DocLoadOutcome::Cancelled;
    };
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string);
    let path_display = path.display().to_string();
    match tokio::task::spawn_blocking(move || {
        let extracted = extract_document(&path, &extract_opts)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        // Metadata only — Start/Preview re-extract with page filters.
        Ok::<_, CoreError>((
            path,
            extracted.kind.label().to_string(),
            name,
            extracted.detail,
            extracted.word_count,
            extracted.page_count,
            document2voice_core::chapters::chapter_titles(&extracted.text),
        ))
    })
    .await
    {
        Ok(Ok((source_path, kind_label, file_name, detail, word_count, page_count, chapters))) => {
            DocLoadOutcome::Ready {
                source_path,
                path_display,
                kind_label,
                file_name,
                detail,
                word_count,
                page_count,
                chapters,
            }
        }
        Ok(Err(e)) => DocLoadOutcome::Failed {
            file_name,
            path_display: Some(path_display),
            message: e.to_string(),
        },
        Err(e) => DocLoadOutcome::Failed {
            file_name,
            path_display: Some(path_display),
            message: e.to_string(),
        },
    }
}

/// Turn a progress label into a Log line, or `None` for bar-only stages.
///
/// Labels come from core (`synth_worker`, `tts`) and from this crate's jobs.
fn progress_log_entry(label: &str) -> Option<(LogKind, String)> {
    let label = label.trim();
    // Bar-only: noisy steps, or ones followed right away by a richer Log line
    // (Saved / Preview finished).
    let quiet = label.is_empty()
        || label.starts_with("Encoding audio")
        || label.starts_with("Reading document")
        || label.starts_with("Playing ")
        || matches!(
            label,
            "Using cached sample"
                | "Sample ready"
                | "Starting sample worker…"
                | "Synthesizing preview…"
                | "Encoding preview…"
                | "Encoding complete"
                | "Done"
        );
    if quiet {
        return None;
    }
    let kind = if label.starts_with("Text ready") || label == "Synthesis complete" {
        LogKind::Success
    } else {
        LogKind::Progress
    };
    Some((kind, label.to_string()))
}

impl App {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy != BusyKind::None
    }

    pub(crate) fn is_saving(&self) -> bool {
        self.busy == BusyKind::Saving
    }

    pub(crate) fn is_previewing(&self) -> bool {
        self.busy == BusyKind::Preview
    }

    /// Blocks Start / Open / Preview / Defaults while a job runs or the replace dialog is open.
    pub(crate) fn ui_locked(&self) -> bool {
        self.is_busy() || self.start_prompt.is_some()
    }

    /// Start may run: valid Source, valid Pages/Skip, Output empty or hard-OK
    /// (soft “file already exists” still allows Start → replace dialog).
    pub(crate) fn can_start(&self) -> bool {
        !self.is_busy()
            && self.source_ready
            && self.source_error.is_none()
            && self.pages_ok()
            && !self.output_error_blocks_start()
    }

    /// Preview may run: Pages/Skip OK; empty Source → sample voice; else valid Source.
    pub(crate) fn can_preview(&self) -> bool {
        if self.is_busy() || !self.pages_ok() {
            return false;
        }
        self.source_path.trim().is_empty() || (self.source_ready && self.source_error.is_none())
    }

    /// True during the ramp-up of a Start job, before speech has started.
    pub(crate) fn save_still_starting(&self) -> bool {
        if !self.is_saving() {
            return false;
        }
        let Ok(g) = self.progress_shared.lock() else {
            return true;
        };
        let label = g.label.trim();
        label.is_empty()
            || label.starts_with("Loading Kokoro model")
            || (g.fraction > 0.0 && g.fraction < 0.10)
    }

    /// Reset cancel + progress before starting Start / Preview.
    fn begin_job(&mut self, kind: BusyKind) {
        self.cancel_flag.store(false, Ordering::Release);
        // Empty label: the action line already says what started.
        self.set_progress(0.02, "");
        self.last_logged_progress.clear();
        self.busy = kind;
    }

    /// Clear busy flag and cancel latch after a job completes or fails.
    fn end_busy(&mut self) {
        self.busy = BusyKind::None;
        self.cancel_flag.store(false, Ordering::Release);
    }

    pub(crate) fn clear_progress_ui(&mut self) {
        self.last_logged_progress.clear();
        self.set_progress(0.0, "");
    }

    fn set_progress(&self, fraction: f32, label: &str) {
        report(&self.progress_shared, fraction, label);
    }

    /// Shared speech fields for Start / Preview background jobs.
    fn speech_params(&self, text: String) -> SpeechParams {
        SpeechParams {
            text,
            voice: self.voice.clone(),
            speed: self.speed,
            style_index: self.style_index_opt(),
            paragraph_pause: self.paragraph_pause,
            pitch: self.pitch,
            loudnorm: self.loudnorm,
            loudness: self.loudness,
            progress: Arc::clone(&self.progress_shared),
            cancel: Arc::clone(&self.cancel_flag),
            active: Arc::clone(&self.active_job),
        }
    }
}

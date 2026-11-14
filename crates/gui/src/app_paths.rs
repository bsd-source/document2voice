//! Source and Output field checks: normalize, validate, log once, and leave-field hooks.

use crate::focus::{unfocus_text_inputs, FocusSlot};
use crate::log_hl::{log_block, log_labeled_path, LogKind};
use crate::paths::{
    default_output_path, is_soft_output_exists_error, next_available_output_path,
    normalize_output_path, normalize_source_path, validate_output_path, validate_source_path,
    SourceError, OUT_EXISTS_MSG, OUT_EXISTS_NOT_OVERWRITTEN_MSG,
};
use crate::types::OutPathOrigin;
use crate::{App, Message};
use document2voice_core::{pdf_page_count, DocKind};
use iced::Task;
use std::path::{Path, PathBuf};

impl App {
    /// Recheck whether Source names a usable file (no logging).
    pub(crate) fn refresh_source_ready(&mut self) {
        let path = normalize_source_path(&self.source_path);
        self.source_ready = !path.is_empty() && validate_source_path(&path).is_ok();
    }

    /// Fill the page-count cache for the current Source path.
    ///
    /// Uses the count from the last Open of the same file; otherwise asks `pdfinfo`
    /// for PDFs. Other formats stay unknown until opened.
    pub(crate) fn refresh_page_count(&mut self) {
        let path = normalize_source_path(&self.source_path);
        if path.is_empty() {
            self.page_count = None;
            return;
        }
        let from_open = self
            .doc_ready
            .as_ref()
            .filter(|d| d.source_path.as_path() == Path::new(&path))
            .and_then(|d| d.page_count)
            .filter(|&n| n > 0);
        if from_open.is_none() && self.page_count.as_ref().is_some_and(|(p, _)| *p == path) {
            return;
        }
        let count = from_open.or_else(|| {
            let p = Path::new(&path);
            match DocKind::from_path(p) {
                Ok(DocKind::Pdf) if p.is_file() => pdf_page_count(p).ok().filter(|&n| n > 0),
                _ => None,
            }
        });
        self.page_count = Some((path, count));
    }

    pub(crate) fn clear_source_error(&mut self) {
        self.source_error = None;
    }

    pub(crate) fn clear_out_error(&mut self) {
        self.out_error = None;
    }

    /// Normalize Source (slash clean + absolute), then check readable file + type.
    ///
    /// Path checks never wipe an Open failure (DRM, empty extract, …): that stays
    /// as a red border until a successful load or the user edits the path.
    pub(crate) fn apply_source_path_validation(&mut self) {
        if self.source_path.trim().is_empty() {
            self.clear_source_error();
            self.source_path_log.clear();
            self.doc_ready = None;
            self.source_ready = false;
            self.page_count = None;
            return;
        }

        let normalized = normalize_source_path(&self.source_path);
        if normalized != self.source_path {
            self.source_path.clone_from(&normalized);
        }

        if self
            .doc_ready
            .as_ref()
            .is_some_and(|doc| Path::new(&normalized) != doc.source_path.as_path())
        {
            self.doc_ready = None;
        }

        let path_err = validate_source_path(&normalized).err();
        self.source_ready = path_err.is_none();
        self.refresh_page_count();
        // Prefer a real path problem; otherwise keep an earlier load-only error.
        let result_err = match path_err {
            Some(e) => Some(e),
            None if self.doc_ready.is_none() => self.source_error.clone().filter(|e| !e.is_path()),
            None => None,
        };
        let err_msg = result_err.as_ref().map(SourceError::message);

        if self.source_path_log.unchanged(&normalized, err_msg) {
            self.source_error = result_err;
            return;
        }

        let recovered = self.source_path_log.had_error(self.source_error.is_some());
        self.append_log_kind(LogKind::Path, log_labeled_path("Source", &normalized));
        match &result_err {
            Some(SourceError::Path(e)) => {
                self.append_log_error(log_block("Source", [e.as_str()]));
                self.doc_ready = None;
            }
            // Load-only errors were already logged when Open failed — keep border only.
            Some(SourceError::Load(_)) => {}
            None if recovered => {
                self.append_log_kind(LogKind::Success, log_labeled_path("Source OK", &normalized));
            }
            None => {}
        }
        self.source_path_log.set(
            normalized,
            result_err.as_ref().map(|e| e.message().to_string()),
        );
        self.source_error = result_err;
    }

    /// After Open fails: fill Source field, log path, set red-border error + PathLog.
    /// Prefer a concrete path problem when present; else keep the extract/load message.
    pub(crate) fn record_source_open_failure(&mut self, path_display: &str, message: &str) {
        let normalized = normalize_source_path(path_display);
        self.source_path.clone_from(&normalized);
        self.append_log_kind(LogKind::Path, log_labeled_path("Source", &normalized));
        let err = match validate_source_path(&normalized) {
            Err(e) => {
                self.append_log_error(log_block("Source", [e.message()]));
                self.source_ready = false;
                e
            }
            Ok(()) => {
                self.source_ready = true;
                SourceError::Load(message.to_string())
            }
        };
        self.refresh_page_count();
        self.source_path_log
            .set(normalized, Some(err.message().to_string()));
        self.source_error = Some(err);
    }

    /// Source path for re-extract on Start/Preview (must exist on disk).
    ///
    /// Empty path is a silent no-op (Start is disabled; Preview uses sample voice).
    /// Invalid non-empty paths still get a red border + log (e.g. file deleted).
    pub(crate) fn require_source_file_path(&mut self) -> Result<PathBuf, Task<Message>> {
        let path = normalize_source_path(&self.source_path);
        if path.is_empty() {
            return Err(unfocus_text_inputs());
        }
        if let Err(e) = validate_source_path(&path) {
            self.append_log_error(format!("Error: {}", e.message()));
            self.source_path_log
                .set(path, Some(e.message().to_string()));
            self.source_error = Some(e);
            self.source_ready = false;
            self.doc_ready = None;
            self.set_focus(FocusSlot::SourcePath);
            return Err(unfocus_text_inputs());
        }
        Ok(PathBuf::from(path))
    }

    /// Keep Output’s extension in sync with Format when the field is non-empty.
    pub(crate) fn sync_output_extension_to_format(&mut self) {
        if self.out_path.trim().is_empty() {
            return;
        }
        self.out_path = normalize_output_path(&self.out_path, self.format.as_export().extension());
        self.apply_output_path_validation();
    }

    /// Normalize Output (extension / absolute path), then check writability.
    ///
    /// Empty Output is allowed (no red border); Start fills the launch-folder default.
    /// A user-set path that already exists also gets a red border (soft warning;
    /// Start still offers the replace dialog).
    pub(crate) fn apply_output_path_validation(&mut self) {
        let ext = self.format.as_export().extension();
        let trimmed = self.out_path.trim();
        if trimmed.is_empty() {
            if !self.out_path.is_empty() {
                self.out_path.clear();
            }
            self.out_error = None;
            // Remember empty OK so leaving/re-entering blank does not re-log.
            if !self.out_path_log.unchanged("", None) {
                self.out_path_log.set(String::new(), None);
            }
            return;
        }

        let normalized = normalize_output_path(trimmed, ext);
        if normalized != self.out_path {
            self.out_path.clone_from(&normalized);
        }

        let result_err = match validate_output_path(&normalized) {
            Err(e) => Some(e),
            // User-typed/browsed path that already exists: red border as a heads-up.
            Ok(())
                if self.out_path_origin == OutPathOrigin::User
                    && Path::new(&normalized).is_file() =>
            {
                // After a declined replace, keep the stronger “not overwritten” message.
                if self.out_error.as_deref() == Some(OUT_EXISTS_NOT_OVERWRITTEN_MSG)
                    || self
                        .out_path_log
                        .unchanged(&normalized, Some(OUT_EXISTS_NOT_OVERWRITTEN_MSG))
                {
                    Some(OUT_EXISTS_NOT_OVERWRITTEN_MSG.to_string())
                } else {
                    Some(OUT_EXISTS_MSG.to_string())
                }
            }
            Ok(()) => None,
        };
        if self
            .out_path_log
            .unchanged(&normalized, result_err.as_deref())
        {
            self.out_error = result_err;
            return;
        }

        // One Log entry only: path when OK; path + reason when not.
        let recovered = self.out_path_log.had_error(self.out_error.is_some());
        match &result_err {
            Some(e) => {
                self.append_log_error(log_block("Output", [normalized.as_str(), e.as_str()]));
            }
            None if recovered => {
                self.append_log_kind(LogKind::Success, log_labeled_path("Output OK", &normalized));
            }
            None => {
                self.append_log_kind(LogKind::Path, log_labeled_path("Output", &normalized));
            }
        }
        self.out_path_log.set(normalized, result_err.clone());
        self.out_error = result_err;
    }

    /// Hard validation failures block Start; soft “file already exists” does not.
    pub(crate) fn output_error_blocks_start(&self) -> bool {
        self.out_error
            .as_deref()
            .is_some_and(|e| !is_soft_output_exists_error(e))
    }

    /// If Output is empty, fill `{launch-dir}/output.{ext}` then validate.
    ///
    /// For **app-filled** defaults only: if that file already exists, pick
    /// `output-1.ext`, `output-2.ext`, … **before** logging (one Log entry).
    ///
    /// Returns `false` when the path is invalid (red border set; caller should stop Start).
    pub(crate) fn ensure_output_path_for_start(&mut self) -> bool {
        let ext = self.format.as_export().extension();
        if self.out_path.trim().is_empty() {
            self.out_path = default_output_path(ext);
            self.out_path_origin = OutPathOrigin::AppDefault;
            self.out_path_log.clear();
            self.clear_out_error();
        }

        // Auto-filled Output only: renumber before validate so we log the free name once.
        let renamed = (self.out_path_origin == OutPathOrigin::AppDefault)
            .then(|| next_available_output_path(Path::new(&self.out_path)))
            .flatten();
        let Some(alt) = renamed else {
            self.apply_output_path_validation();
            if self.output_error_blocks_start() {
                self.set_focus(FocusSlot::OutPath);
                return false;
            }
            return true;
        };

        let old = std::mem::replace(
            &mut self.out_path,
            normalize_output_path(&alt.display().to_string(), ext),
        );
        let normalized = self.out_path.clone();
        if let Err(e) = validate_output_path(&normalized) {
            self.out_error = Some(e.clone());
            self.out_path_log.set(normalized.clone(), Some(e.clone()));
            self.append_log_error(log_block("Output", [normalized.as_str(), e.as_str()]));
            self.set_focus(FocusSlot::OutPath);
            return false;
        }
        self.clear_out_error();
        self.out_path_log.set(normalized.clone(), None);
        self.append_log_kind(
            LogKind::Path,
            log_block(
                "Output",
                [
                    normalized.as_str(),
                    format!("was {old} (already existed)").as_str(),
                ],
            ),
        );
        true
    }

    /// True when the Output path exists on disk (regular file).
    pub(crate) fn output_file_exists(&self) -> bool {
        let p = self.out_path.trim();
        !p.is_empty() && Path::new(p).is_file()
    }

    /// Mark Output as “exists, not overwritten” (red border + Log) after the user declines.
    pub(crate) fn mark_output_exists_not_overwritten(&mut self) {
        let path = self.out_path.trim().to_string();
        if path.is_empty() {
            return;
        }
        let msg = OUT_EXISTS_NOT_OVERWRITTEN_MSG.to_string();
        self.out_error = Some(msg.clone());
        self.out_path_log.set(path.clone(), Some(msg));
        self.append_log_kind(
            LogKind::Cancel,
            log_block("Start cancelled — file not overwritten", [path.as_str()]),
        );
        self.set_focus(FocusSlot::OutPath);
    }

    /// Leave Source path field: run validation once (not on every keystroke).
    pub(crate) fn blur_source_path_if_needed(&mut self, next: FocusSlot) {
        if self.focus == FocusSlot::SourcePath && next != FocusSlot::SourcePath {
            self.apply_source_path_validation();
        }
    }

    /// Leave Output path field: run permission check once (not on every keystroke).
    pub(crate) fn blur_out_path_if_needed(&mut self, next: FocusSlot) {
        if self.focus == FocusSlot::OutPath && next != FocusSlot::OutPath {
            self.apply_output_path_validation();
        }
    }
}

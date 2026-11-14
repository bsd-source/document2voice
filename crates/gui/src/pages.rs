//! Pages From / To / Skip: parsing, validation against the document length, and App glue.

use crate::focus::FocusSlot;
use crate::log_hl::{log_block, LogKind};
use crate::types::PagesFieldError;
use crate::{App, PREVIEW_EXTRACT_PAGES};
use document2voice_core::{
    format_page_list, parse_skip_pages, validate_page_selection, ExtractOptions, PageRange,
    PdfPages,
};

/// Parse and validate From / To / Skip for a document of `page_count` pages.
///
/// Empty fields mean the whole document with no skip. Every field problem is
/// collected so a shorter document can flag all stale values at once.
pub(crate) fn parse_page_selection(
    from_s: &str,
    to_s: &str,
    skip_s: &str,
    page_count: Option<u32>,
) -> Result<(PdfPages, Vec<u32>), PagesFieldError> {
    let mut details: Vec<String> = Vec::new();
    let mut flag_from = false;
    let mut flag_to = false;
    let mut flag_skip = false;

    let from_n = match parse_page_num_field(from_s.trim(), true) {
        Ok(n) => n,
        Err(e) => {
            details.extend(e.details);
            flag_from = true;
            None
        }
    };
    let to_n = match parse_page_num_field(to_s.trim(), false) {
        Ok(n) => n,
        Err(e) => {
            details.extend(e.details);
            flag_to = true;
            None
        }
    };
    let skip = match parse_skip_field(skip_s.trim()) {
        Ok(s) => s,
        Err(e) => {
            details.extend(e.details);
            flag_skip = true;
            Vec::new()
        }
    };

    let pages_ok = if !flag_from && !flag_to {
        match pages_from_from_to(from_n, to_n, page_count) {
            Ok(pages) => Some(pages),
            Err(e) => {
                details.extend(e.details);
                flag_from |= e.from;
                flag_to |= e.to;
                None
            }
        }
    } else {
        None
    };
    // Clamp an overshooting range so Skip can still be checked on a shorter document.
    let pages_for_skip = pages_ok
        .clone()
        .or_else(|| clamped_pages_for_skip_check(from_n, to_n, page_count));

    if !flag_skip && !skip.is_empty() {
        let past_doc: Vec<u32> = page_count
            .map(|pc| skip.iter().copied().filter(|&p| p > pc).collect())
            .unwrap_or_default();
        if let (Some(pc), false) = (page_count, past_doc.is_empty()) {
            details.push(skip_err(
                &past_doc,
                &format!("is past end ({pc} pages)"),
                &format!("are past end ({pc} pages)"),
            ));
            flag_skip = true;
        } else if let Some(pages) = &pages_for_skip {
            if let Err(e) = validate_skip_list(pages, &skip, page_count) {
                for d in e.details {
                    if !details.contains(&d) {
                        details.push(d);
                    }
                }
                flag_skip = true;
            }
        }
        if let Some(pages) = &pages_ok {
            if let Err(e) = validate_page_selection(pages, &skip, page_count) {
                let msg = e.to_string();
                if !details.contains(&msg) {
                    details.push(msg);
                }
                flag_skip = true;
            }
        }
    }

    match PagesFieldError::from_parts(details, flag_from, flag_to, flag_skip) {
        Some(e) => Err(e),
        None => Ok((pages_ok.unwrap_or(PdfPages::All), skip)),
    }
}

/// When start/end overshoot a shorter document, still build a range so Skip can be
/// checked (e.g. end=20, skip=2 on a 2-page file → “Skip 2 is the end page”).
fn clamped_pages_for_skip_check(
    from_n: Option<u32>,
    to_n: Option<u32>,
    page_count: Option<u32>,
) -> Option<PdfPages> {
    let pc = page_count?;
    if pc < 1 {
        return None;
    }
    // From past the end → no usable selection for skip rules.
    if from_n.is_some_and(|f| f > pc) {
        return None;
    }
    let start = from_n.unwrap_or(1).max(1).min(pc);
    let end = match to_n {
        Some(t) => t.min(pc).max(start),
        None => pc,
    };
    // Explicit To (even if overshot) → Range so “is the end page” applies at last page.
    if to_n.is_some() {
        PageRange::new(start, end).ok().map(PdfPages::Range)
    } else if from_n.is_some() {
        Some(PdfPages::SkipBefore(start))
    } else {
        Some(PdfPages::All)
    }
}

/// Parse a single Pages From or To field (blank → None).
fn parse_page_num_field(s: &str, is_from: bool) -> Result<Option<u32>, PagesFieldError> {
    if s.is_empty() {
        return Ok(None);
    }
    let field = if is_from { "Start page" } else { "End page" };
    let msg = match s.parse::<u32>() {
        Ok(n) if n >= 1 => return Ok(Some(n)),
        Ok(_) => format!("{field}: page numbers start at 1"),
        Err(_) => format!("{field}: bad number “{s}”"),
    };
    Err(if is_from {
        PagesFieldError::from_msg(msg)
    } else {
        PagesFieldError::to_msg(msg)
    })
}

fn parse_skip_field(skip_s: &str) -> Result<Vec<u32>, PagesFieldError> {
    if skip_s.is_empty() {
        Ok(Vec::new())
    } else {
        parse_skip_pages(skip_s).map_err(|e| PagesFieldError::skip_msg(e.to_string()))
    }
}

/// Build PdfPages from optional From/To, checking order and document length.
fn pages_from_from_to(
    from_n: Option<u32>,
    to_n: Option<u32>,
    page_count: Option<u32>,
) -> Result<PdfPages, PagesFieldError> {
    // Start after end → start field is the problem.
    if let (Some(f), Some(t)) = (from_n, to_n) {
        if f > t {
            return Err(PagesFieldError::from_msg(format!(
                "Start page {f} is after end page {t}"
            )));
        }
    }

    // Bounds vs document length: highlight only the field(s) past the end.
    if let Some(pc) = page_count {
        let from_past = from_n.filter(|&f| f > pc);
        let to_past = to_n.filter(|&t| t > pc);
        let message = match (from_past, to_past) {
            (Some(f), Some(t)) => Some(format!(
                "Start page {f} and end page {t} are past end ({pc} pages)"
            )),
            (Some(f), None) => Some(format!("Start page {f} is past end ({pc} pages)")),
            (None, Some(t)) => Some(format!("End page {t} is past end ({pc} pages)")),
            (None, None) => None,
        };
        if let Some(message) = message {
            return Err(PagesFieldError::with_detail(
                message,
                from_past.is_some(),
                to_past.is_some(),
                false,
            ));
        }
    }

    match (from_n, to_n) {
        (None, None) => Ok(PdfPages::All),
        (Some(s), None) => Ok(PdfPages::SkipBefore(s)),
        (None, Some(e)) => PageRange::new(1, e)
            .map(PdfPages::Range)
            .map_err(|err| PagesFieldError::to_msg(err.to_string())),
        (Some(s), Some(e)) => PageRange::new(s, e)
            .map(PdfPages::Range)
            .map_err(|err| PagesFieldError::from_msg(err.to_string())),
    }
}

/// Inclusive page range a selection covers (when the end is known).
fn effective_page_range(pages: &PdfPages, page_count: Option<u32>) -> Option<(u32, u32)> {
    match pages {
        PdfPages::Range(r) => Some((r.start, r.end)),
        PdfPages::SkipBefore(start) => Some((*start, page_count?)),
        PdfPages::All => Some((1, page_count?)),
    }
}

/// Skip page rules (when the selection range is known):
/// - cannot be **before** or **after** the selection
/// - cannot be the **start** page (change From instead)
/// - when **To** is set: cannot be the **end** page (change To instead)
///
/// Interior pages only: e.g. From 2, To 5 → skip 3 or 4 is OK; skip 2 or 5 errors.
/// Whole-document / From-only may still skip the last page of the file.
fn validate_skip_list(
    pages: &PdfPages,
    skip: &[u32],
    page_count: Option<u32>,
) -> Result<(), PagesFieldError> {
    let Some((start, end)) = effective_page_range(pages, page_count) else {
        return Ok(());
    };
    let range_label = if start == end {
        format!("{start}")
    } else {
        format!("{start}–{end}")
    };
    let matching = |pred: &dyn Fn(u32) -> bool| -> Vec<u32> {
        skip.iter().copied().filter(|&p| pred(p)).collect()
    };
    let fail = |pages: &[u32], singular: &str, plural: &str| {
        Err(PagesFieldError::skip_msg(skip_err(pages, singular, plural)))
    };
    let outside_one = format!("is outside {range_label}");
    let outside_many = format!("are outside {range_label}");

    let is_start = matching(&|p| p == start);
    if !is_start.is_empty() {
        return fail(&is_start, "is the start page", "are start pages");
    }
    let before = matching(&|p| p < start);
    if !before.is_empty() {
        return fail(&before, &outside_one, &outside_many);
    }
    // Explicit To: the end page cannot be skipped (lower To instead).
    if matches!(pages, PdfPages::Range(_)) {
        let is_end = matching(&|p| p == end);
        if !is_end.is_empty() {
            return fail(&is_end, "is the end page", "are end pages");
        }
    }
    let after = matching(&|p| p > end);
    if !after.is_empty() {
        return fail(&after, &outside_one, &outside_many);
    }
    Ok(())
}

/// Short skip messages for the Log: `Skip 2 is the end page`.
fn skip_err(pages: &[u32], singular: &str, plural: &str) -> String {
    let listed = format_page_list(pages);
    if pages.len() == 1 {
        format!("Skip {listed} {singular}")
    } else {
        format!("Skip {listed} {plural}")
    }
}

/// Shrink a Start-style page selection to a short prefix for Preview extract.
pub(crate) fn narrow_pages_for_preview(pages: PdfPages, page_count: Option<u32>) -> PdfPages {
    let (start, max_end) = match pages {
        PdfPages::All => (1, u32::MAX),
        PdfPages::Range(r) => (r.start, r.end),
        PdfPages::SkipBefore(n) => (n, u32::MAX),
    };
    let start = start.max(1);
    let mut end = start
        .saturating_add(PREVIEW_EXTRACT_PAGES.saturating_sub(1))
        .min(max_end.max(start));
    if let Some(total) = page_count.filter(|&t| t >= 1) {
        end = end.min(total);
    }
    PdfPages::Range(PageRange {
        start,
        end: end.max(start),
    })
}

impl App {
    /// Total pages for bounds checks, from the cache filled on Source blur / Open.
    pub(crate) fn document_page_count(&self) -> Option<u32> {
        self.page_count.as_ref().and_then(|(_, n)| *n)
    }

    /// Current From / To / Skip as an extract selection, or the field errors.
    pub(crate) fn page_selection(&self) -> Result<(PdfPages, Vec<u32>), PagesFieldError> {
        parse_page_selection(
            &self.page_from,
            &self.page_to,
            &self.page_skip,
            self.document_page_count(),
        )
    }

    /// From / To / Skip currently valid (live check, not only last blur).
    pub(crate) fn pages_ok(&self) -> bool {
        self.page_selection().is_ok()
    }

    /// Pages From/To and Skip are editable only when Source has a path.
    pub(crate) fn pages_fields_enabled(&self) -> bool {
        !self.source_path.trim().is_empty()
    }

    /// True when From, To, or Skip is filled.
    pub(crate) fn using_page_filters(&self) -> bool {
        !self.page_from.trim().is_empty()
            || !self.page_to.trim().is_empty()
            || !self.page_skip.trim().is_empty()
    }

    pub(crate) fn clear_pages_error(&mut self) {
        self.pages_error = None;
    }

    /// Clear start/end/skip values and pages error state (e.g. Source emptied).
    pub(crate) fn clear_pages_fields(&mut self) {
        self.page_from.clear();
        self.page_to.clear();
        self.page_skip.clear();
        self.pages_error = None;
        self.pages_log.clear();
    }

    /// Snapshot of page fields + known length for log de-duplication.
    fn pages_selection_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.page_from.trim(),
            self.page_to.trim(),
            self.page_skip.trim(),
            self.document_page_count()
                .map(|n| n.to_string())
                .unwrap_or_default()
        )
    }

    /// Validate Pages From/To + Skip. Used on leave-field, after Open, and before Start/Preview.
    pub(crate) fn apply_pages_validation(&mut self) {
        let key = self.pages_selection_key();
        let result_err = self.page_selection().err();
        let err_msg = result_err.as_ref().map(|e| e.message.as_str());

        if self.pages_log.unchanged(&key, err_msg) {
            self.pages_error = result_err;
            return;
        }

        let recovered = self.pages_log.had_error(self.pages_error.is_some());
        // Same shape as Source/Output errors: title ●, each issue as its own · line.
        if let Some(e) = &result_err {
            self.append_log_error(log_block("Pages", e.details.iter().map(String::as_str)));
        } else if recovered {
            self.append_log_kind(LogKind::Success, "Pages OK");
        }
        self.pages_log
            .set(key, result_err.as_ref().map(|e| e.message.clone()));
        self.pages_error = result_err;
    }

    /// Leaving any Pages field (From, To, or Skip) validates all three.
    pub(crate) fn blur_pages_if_needed(&mut self, next: FocusSlot) {
        if self.focus.is_page() && next != self.focus {
            self.apply_pages_validation();
        }
    }

    /// Current Pages / Skip fields as extract options.
    ///
    /// Runs the leave-field validation (red borders + log once), so callers
    /// should not re-log the returned error.
    pub(crate) fn extract_options_from_ui(&mut self) -> Result<ExtractOptions, String> {
        self.apply_pages_validation();
        let (pdf_pages, skip_pages) = self.page_selection().map_err(|e| e.message)?;
        Ok(ExtractOptions {
            pdf_pages,
            skip_pages,
        })
    }

    /// Extract options for **Preview**: the first few pages of the selection,
    /// enough for ~[`crate::PREVIEW_SECS`]. Skip is ignored.
    pub(crate) fn extract_options_for_preview(&mut self) -> Result<ExtractOptions, String> {
        let mut opts = self.extract_options_from_ui()?;
        opts.pdf_pages = narrow_pages_for_preview(opts.pdf_pages, self.document_page_count());
        opts.skip_pages.clear();
        Ok(opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(p: &PdfPages) -> Option<(u32, u32)> {
        match p {
            PdfPages::Range(r) => Some((r.start, r.end)),
            _ => None,
        }
    }

    #[test]
    fn empty_fields_select_whole_document() {
        let (pages, skip) = parse_page_selection("", " ", "", Some(10)).unwrap();
        assert!(matches!(pages, PdfPages::All));
        assert!(skip.is_empty());
    }

    #[test]
    fn from_only_and_to_only() {
        let (pages, _) = parse_page_selection("3", "", "", Some(10)).unwrap();
        assert!(matches!(pages, PdfPages::SkipBefore(3)));
        let (pages, _) = parse_page_selection("", "4", "", None).unwrap();
        assert_eq!(range(&pages), Some((1, 4)));
    }

    #[test]
    fn bad_numbers_flag_their_field() {
        let e = parse_page_selection("x", "0", "", None).unwrap_err();
        assert!(e.from && e.to && !e.skip);
        assert_eq!(e.details.len(), 2);
    }

    #[test]
    fn start_after_end_flags_start() {
        let e = parse_page_selection("5", "2", "", None).unwrap_err();
        assert!(e.from && !e.to);
    }

    #[test]
    fn past_end_flags_only_the_overshooting_field() {
        let e = parse_page_selection("2", "20", "", Some(5)).unwrap_err();
        assert!(!e.from && e.to);
        assert_eq!(
            e.details,
            vec!["End page 20 is past end (5 pages)".to_string()]
        );
        // Unknown length: no bounds check.
        assert!(parse_page_selection("2", "20", "", None).is_ok());
    }

    #[test]
    fn skip_rules() {
        assert!(parse_page_selection("2", "5", "3,4", Some(10)).is_ok());
        let e = parse_page_selection("2", "5", "2", Some(10)).unwrap_err();
        assert!(e.skip && e.details[0].contains("start page"));
        let e = parse_page_selection("2", "5", "5", Some(10)).unwrap_err();
        assert!(e.details[0].contains("end page"));
        let e = parse_page_selection("2", "5", "7", Some(10)).unwrap_err();
        assert!(e.details[0].contains("outside 2–5"));
        let e = parse_page_selection("", "", "12", Some(10)).unwrap_err();
        assert_eq!(e.details[0], "Skip 12 is past end (10 pages)");
        // Whole document may skip its last page.
        assert!(parse_page_selection("", "", "10", Some(10)).is_ok());
    }

    #[test]
    fn skip_checked_against_clamped_range() {
        // End past a 2-page document still reports that Skip 2 is the end page.
        let e = parse_page_selection("", "20", "2", Some(2)).unwrap_err();
        assert!(e.to && e.skip);
        assert!(e.details.iter().any(|d| d == "Skip 2 is the end page"));
    }

    #[test]
    fn preview_narrows_to_first_pages() {
        let p = narrow_pages_for_preview(PdfPages::All, Some(100));
        assert_eq!(range(&p), Some((1, PREVIEW_EXTRACT_PAGES)));
        let p = narrow_pages_for_preview(PdfPages::SkipBefore(7), Some(7));
        assert_eq!(range(&p), Some((7, 7)));
        let r = PageRange::new(4, 4).unwrap();
        assert_eq!(
            range(&narrow_pages_for_preview(PdfPages::Range(r), None)),
            Some((4, 4))
        );
    }
}

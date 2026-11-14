//! Selectable multi-line Log with color-coded bullets (by kind of message).

use crate::paths::normalize_source_path;
use crate::style::border_color;
use crate::{App, LOG_MAX_LINES, PREVIEW_SECS};
use document2voice_core::{format_page_list, DocKind, PdfPages};
use iced::advanced::text::highlighter::{self, Highlighter};
use iced::border::Border;
use iced::widget::text_editor;
use iced::{Background, Color, Font, Theme};
use std::path::Path;

/// Bullet starting every Log entry; the kind is told by its color.
pub(crate) const LOG_MARK: &str = "●";

/// What kind of message this Log line is — sets the bullet color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LogKind {
    /// Generic note.
    #[default]
    Info,
    /// Startup, tool checks, idle “ready”.
    System,
    /// Source / output path messages.
    Path,
    /// Things the user started (Open, Start, Defaults, Cancel request).
    Action,
    /// Work in progress (percent / phase labels).
    Progress,
    /// Completed successfully (loaded, saved, path OK, preview finished).
    Success,
    /// Caution (e.g. page range on a non-PDF may be wrong).
    Warning,
    /// User cancelled a job.
    Cancel,
    /// Failures / validation errors.
    Error,
}

impl LogKind {
    pub(crate) fn color(self) -> Color {
        match self {
            // Soft desktop-style palette (readable on the light Log surface).
            Self::Info => Color::from_rgb8(0x5E, 0x5C, 0x64),
            Self::System => Color::from_rgb8(0x77, 0x76, 0x7B),
            Self::Path => Color::from_rgb8(0x81, 0x3D, 0x9C),
            Self::Action => Color::from_rgb8(0xE6, 0x61, 0x00),
            Self::Progress => Color::from_rgb8(0x35, 0x84, 0xE4),
            Self::Success => Color::from_rgb8(0x2E, 0xC2, 0x7E),
            // Amber — between Action orange and Error red.
            Self::Warning => Color::from_rgb8(0xE5, 0xA5, 0x0A),
            Self::Cancel => Color::from_rgb8(0xAE, 0x81, 0x5C),
            Self::Error => Color::from_rgb8(0xE0, 0x1B, 0x24),
        }
    }

    /// Format a log entry: `●  title`, then `   ·  detail` rows; wraps hang with spaces.
    pub(crate) fn format_line(self, message: &str) -> String {
        let message = message.trim_end_matches(['\r', '\n']);
        let mut logical = message.lines().map(str::trim_end).filter(|l| !l.is_empty());
        let Some(first) = logical.next() else {
            return String::new();
        };

        let mut out = String::new();
        // Title under ●  (bullet + two spaces in monospace).
        append_soft_wrapped(&mut out, first.trim_start(), LOG_MARK, "  ", "   ");

        for detail in logical {
            let text = detail.trim_start();
            // Skip empty hang-only leftovers if a caller still pre-wraps.
            if text.is_empty() {
                continue;
            }
            out.push('\n');
            // Detail under "   ·  "; wraps hang with six spaces.
            append_soft_wrapped(&mut out, text, "   ·", "  ", "      ");
        }
        out
    }
}

/// Append mark + gap + first chunk, then hang-wrap any remaining chunks.
fn append_soft_wrapped(out: &mut String, text: &str, mark: &str, gap_after_mark: &str, hang: &str) {
    // Wrap only the body; mark/gap (or hang indent) already use some columns.
    let prefix_cols = mark.chars().count() + gap_after_mark.chars().count();
    let body_cols = LOG_LINE_COLS.saturating_sub(prefix_cols).max(12);
    let chunks = soft_wrap(text, body_cols);
    out.push_str(mark);
    out.push_str(gap_after_mark);
    if let Some(first) = chunks.first() {
        out.push_str(first);
    }
    for chunk in chunks.into_iter().skip(1) {
        out.push('\n');
        out.push_str(hang);
        out.push_str(&chunk);
    }
}

/// How many monospace columns fit one log line (mark + body), tuned for
/// [`crate::style::MAIN_W`] (443px) with log font size 11 and card/editor padding.
///
/// Measured: Noto Sans Mono @ 11px ≈ 7px/char; content width after page (12×2),
/// card (10×2), and editor (10×2) padding ≈ 379px → ~54 cols. Use 56 so lines
/// reach the right edge with a small safety margin (scrollbar / metric drift).
const LOG_LINE_COLS: usize = 56;

/// Title plus detail lines (each detail becomes an indented row under the bullet).
pub(crate) fn log_block(
    title: impl AsRef<str>,
    details: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let mut s = title.as_ref().trim().to_string();
    for d in details {
        let d = d.as_ref().trim();
        if !d.is_empty() {
            s.push('\n');
            s.push_str(d);
        }
    }
    s
}

/// Label on the first line; path is one detail (soft-wrapped by [`LogKind::format_line`]).
pub(crate) fn log_labeled_path(label: &str, path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return label.to_string();
    }
    log_block(label, [path])
}

/// Soft-wrap for the narrow monospace Log: prefer breaks at spaces.
fn soft_wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(12);
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if word.is_empty() {
            continue;
        }
        // Single token longer than the column — hard-split by character.
        if word.chars().count() > width {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() >= width {
                    out.push(std::mem::take(&mut chunk));
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                cur = chunk;
            }
            continue;
        }
        let need = if cur.is_empty() {
            word.chars().count()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
        if need > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur.push_str(word);
        } else if cur.is_empty() {
            cur.push_str(word);
        } else {
            cur.push(' ');
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Body text in the Log (always black).
pub(crate) fn log_body_color() -> Color {
    Color::from_rgb8(0x00, 0x00, 0x00)
}

/// Highlighter settings: one kind per document line (parallel to log lines).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LogHlSettings {
    pub(crate) kinds: Vec<LogKind>,
}

/// Per-span highlight: colored bullet (● or detail ·) vs black body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogHl {
    /// Leading `●` / detail `·` — tinted by [`LogKind`].
    Mark(LogKind),
    /// Rest of the line — always black.
    Body,
}

/// Indented detail line from [`LogKind::format_line`]: leading spaces then `·`.
fn is_detail_mid_dot_line(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    rest.starts_with('·') && rest.len() < line.len()
}

/// Pick the bullet color for one physical log line.
///
/// `Pages` and `Skip pages` always share [`LogKind::Action`] so their ● match,
/// even if the highlighter’s line index drifts.
fn kind_for_log_line(line: &str, fallback: LogKind) -> LogKind {
    let body = line
        .strip_prefix('●')
        .map_or_else(|| line.trim_start(), str::trim_start);
    if body.starts_with("Page  ")
        || body.starts_with("Pages  ")
        || body.starts_with("Skip page  ")
        || body.starts_with("Skip pages  ")
    {
        return LogKind::Action;
    }
    fallback
}

/// Colors the bullet by kind; keeps message text black.
pub(crate) struct LogHighlighter {
    pub(crate) current: usize,
    pub(crate) kinds: Vec<LogKind>,
}

impl Highlighter for LogHighlighter {
    type Settings = LogHlSettings;
    type Highlight = LogHl;
    type Iterator<'a> = std::vec::IntoIter<(std::ops::Range<usize>, LogHl)>;

    fn new(settings: &Self::Settings) -> Self {
        Self {
            current: 0,
            kinds: settings.kinds.clone(),
        }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.kinds.clone_from(&new_settings.kinds);
        self.current = 0;
    }

    fn change_line(&mut self, line: usize) {
        self.current = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let fallback = self
            .kinds
            .get(self.current)
            .copied()
            .unwrap_or(LogKind::Info);
        // Force Pages / Skip pages to share one mark color even if kinds desync.
        let kind = kind_for_log_line(line, fallback);
        self.current = self.current.saturating_add(1);

        let mut spans = Vec::with_capacity(2);
        // Primary: "●  message"
        if let Some(rest) = line.strip_prefix('●') {
            let mark_end = line.len() - rest.len();
            spans.push((0..mark_end, LogHl::Mark(kind)));
            if mark_end < line.len() {
                spans.push((mark_end..line.len(), LogHl::Body));
            }
        } else if is_detail_mid_dot_line(line) {
            // Detail: "   ·  text" only — do not treat " · " inside body text as a mark.
            if let Some(idx) = line.find('·') {
                if idx > 0 {
                    spans.push((0..idx, LogHl::Body));
                }
                let mark_end = idx + '·'.len_utf8();
                // Detail mid-dots inherit the parent entry color (same family as ●).
                spans.push((idx..mark_end, LogHl::Mark(kind)));
                if mark_end < line.len() {
                    spans.push((mark_end..line.len(), LogHl::Body));
                }
            } else if !line.is_empty() {
                spans.push((0..line.len(), LogHl::Body));
            }
        } else if !line.is_empty() {
            spans.push((0..line.len(), LogHl::Body));
        }
        spans.into_iter()
    }

    fn current_line(&self) -> usize {
        self.current
    }
}

pub(crate) fn log_hl_format(hl: &LogHl, _theme: &Theme) -> highlighter::Format<Font> {
    match *hl {
        LogHl::Mark(kind) => highlighter::Format {
            color: Some(kind.color()),
            font: Some(Font::MONOSPACE),
        },
        LogHl::Body => highlighter::Format {
            color: Some(log_body_color()),
            font: Some(Font::MONOSPACE),
        },
    }
}

/// Log editor chrome — soft console surface.
pub(crate) fn log_editor_style(_theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(Color::from_rgb8(0xF0, 0xEF, 0xED)),
        border: Border {
            color: border_color(),
            width: 1.0,
            radius: 6.0.into(),
        },
        placeholder: Color::from_rgb8(0x9A, 0x99, 0x96),
        value: log_body_color(),
        selection: Color::from_rgb8(0xE8, 0xF1, 0xFC),
    }
}

/// Shorten long file names for the Log (`foo…bar.pdf`).
pub(crate) fn short_name(name: &str, max: usize) -> String {
    let name = name.trim();
    if name.chars().count() <= max {
        return name.to_string();
    }
    let keep = max.saturating_sub(1) / 2;
    let front: String = name.chars().take(keep).collect();
    let back: String = name
        .chars()
        .rev()
        .take(keep)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{front}…{back}")
}

/// Human-friendly word counts: `132,614` or `1.3M`.
pub(crate) fn format_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out.chars().rev().collect()
    } else {
        n.to_string()
    }
}

impl App {
    /// Append an error entry (red bullet; multi-line friendly).
    pub(crate) fn append_log_error(&mut self, line: impl Into<String>) {
        self.append_log_kind(LogKind::Error, line);
    }

    /// Append a log entry with an explicit kind (colored `●`, optional indented lines).
    pub(crate) fn append_log_kind(&mut self, kind: LogKind, line: impl Into<String>) {
        let line = line.into();
        // Keep internal newlines; only trim outer blank edges.
        let line = line.trim_start_matches(['\r', '\n', ' ']).trim_end();
        if line.is_empty() {
            return;
        }
        self.push_log_display(kind, kind.format_line(line));
    }

    /// Append lines that each get their own primary `●`, all in one kind/color.
    pub(crate) fn append_log_primary_lines(&mut self, kind: LogKind, messages: &[String]) {
        let display = messages
            .iter()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty())
            .map(|m| format!("{LOG_MARK}  {m}"))
            .collect::<Vec<_>>()
            .join("\n");
        if !display.is_empty() {
            self.push_log_display(kind, display);
        }
    }

    /// Push a pre-formatted log block into the buffer + editor.
    fn push_log_display(&mut self, kind: LogKind, display: String) {
        // Skip exact consecutive duplicates (e.g. repeated progress polls).
        if self.log_lines.back().map(|(t, _)| t.as_str()) == Some(display.as_str()) {
            return;
        }
        let was_empty = self.log_lines.is_empty();
        self.log_lines.push_back((display.clone(), kind));
        let mut trimmed = false;
        // Cap by physical lines (multi-line entries count fully).
        while self.log_physical_lines() > LOG_MAX_LINES {
            self.log_lines.pop_front();
            trimmed = true;
        }
        if trimmed {
            self.rebuild_log_content();
            return;
        }
        // Append in place so selection/scroll state is preserved when possible.
        self.log_content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        if !was_empty {
            self.log_content
                .perform(text_editor::Action::Edit(text_editor::Edit::Enter));
        }
        self.log_content
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new(display),
            )));
        self.log_content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    }

    fn log_physical_lines(&self) -> usize {
        self.log_lines
            .iter()
            .map(|(t, _)| t.lines().count().max(1))
            .sum()
    }

    fn rebuild_log_content(&mut self) {
        let text = self
            .log_lines
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.log_content = text_editor::Content::with_text(&text);
        self.log_content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    }

    /// One highlighter kind per *physical* Log line (multi-line entries expand).
    pub(crate) fn log_hl_settings(&self) -> LogHlSettings {
        let kinds = self
            .log_lines
            .iter()
            .flat_map(|(text, kind)| std::iter::repeat_n(*kind, text.lines().count().max(1)))
            .collect();
        LogHlSettings { kinds }
    }

    /// External tool versions, then the ready line (after the startup probe finishes).
    pub(crate) fn log_tools_and_ready(&mut self, tools: &[String]) {
        self.append_log_kind(LogKind::System, log_block("Tools", tools));
        self.append_log_kind(LogKind::Success, "Ready · open a file to begin");
    }

    /// Warn when page range/skip is used on a non-PDF (approx chunks, not real pages).
    pub(crate) fn log_non_pdf_page_filter_warning(&mut self) {
        if !self.using_page_filters() {
            return;
        }
        let path = normalize_source_path(&self.source_path);
        if path.is_empty() {
            return;
        }
        if let Ok(kind) = DocKind::from_path(Path::new(&path)) {
            if kind != DocKind::Pdf {
                self.append_log_kind(
                    LogKind::Warning,
                    "Warning: page range / skip may be wrong — only PDF uses real page numbers. \
                     Other formats use ~250-word chunks. Convert to PDF first for accurate pages.",
                );
            }
        }
    }

    /// Log the page range + skip list when **Start** begins.
    pub(crate) fn log_page_selection(&mut self) {
        let Ok((pages, skip)) = self.page_selection() else {
            return;
        };
        let pages_line = match pages {
            PdfPages::All => "Pages  all (whole document)".to_string(),
            PdfPages::Range(r) if r.start == r.end => format!("Page  {}", r.start),
            PdfPages::Range(r) => format!("Pages  {}–{}", r.start, r.end),
            PdfPages::SkipBefore(n) => format!("Pages  from {n} to end"),
        };
        let mut messages = vec![pages_line];
        if !skip.is_empty() {
            let skip_label = if skip.len() == 1 {
                "Skip page"
            } else {
                "Skip pages"
            };
            messages.push(format!("{skip_label}  {}", format_page_list(&skip)));
        }
        self.append_log_primary_lines(LogKind::Action, &messages);
        self.log_non_pdf_page_filter_warning();
    }

    /// Log what Preview will play: a short sample from the start page (To and Skip unused).
    pub(crate) fn log_preview_scope(&mut self) {
        let Ok((pages, _skip)) = self.page_selection() else {
            return;
        };
        let from = match pages {
            PdfPages::All => "start of document".to_string(),
            PdfPages::Range(r) => format!("start of page {}", r.start),
            PdfPages::SkipBefore(n) => format!("start of page {n}"),
        };
        self.append_log_kind(
            LogKind::Action,
            log_block(
                "Preview…",
                [format!("~{PREVIEW_SECS:.0}s audio from {from}")],
            ),
        );
    }
}

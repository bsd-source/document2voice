//! Read text out of PDF files using Poppler tools (`pdftotext` and `pdfinfo`):
//! keep layout where possible, support page ranges, and apply light cleanup.

use crate::error::{CoreError, Result};
use std::path::Path;
use std::process::Command;
use tracing::info;

/// Inclusive page range, counting from 1 (as used by `pdftotext -f` / `-l`).
#[derive(Debug, Clone, Copy)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

impl PageRange {
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start < 1 {
            return Err(CoreError::Other("Page numbers start at 1".into()));
        }
        if end < start {
            return Err(CoreError::Other(format!(
                "Start page ({start}) cannot be after end page ({end})"
            )));
        }
        Ok(Self { start, end })
    }
}

/// How to choose which pages of a PDF to use.
#[derive(Debug, Clone)]
pub enum PdfPages {
    /// The whole document.
    All,
    /// Inclusive range (page numbers start at 1).
    Range(PageRange),
    /// Start at this page and go to the end (e.g. 4 skips cover/contents pages 1–3).
    SkipBefore(u32),
}

/// Result of pulling text from a PDF.
#[derive(Debug, Clone)]
pub(crate) struct PdfExtract {
    pub page_count: u32,
    pub used_range: PageRange,
    pub text: String,
}

/// How many pages a PDF has (via the `pdfinfo` tool).
pub fn pdf_page_count(pdf: impl AsRef<Path>) -> Result<u32> {
    let pdf = pdf.as_ref();
    ensure_pdf(pdf)?;
    pdf_page_count_unchecked(pdf)
}

/// [`pdf_page_count`] for a file [`ensure_pdf`] already accepted.
fn pdf_page_count_unchecked(pdf: &Path) -> Result<u32> {
    let out = Command::new("pdfinfo")
        .arg(pdf)
        .output()
        .map_err(|e| CoreError::Other(format!("failed to run pdfinfo: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CoreError::Other(format!("pdfinfo failed: {err}")));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            let n: u32 = rest
                .trim()
                .parse()
                .map_err(|_| CoreError::Other(format!("bad Pages line: {line}")))?;
            return Ok(n);
        }
    }
    Err(CoreError::Other(
        "could not read page count (pdfinfo)".into(),
    ))
}

/// User-facing error when Skip removes every selected page.
fn empty_after_skip_message(range: PageRange) -> String {
    if range.start == range.end {
        format!(
            "Nothing to process — page {} is selected and also skipped",
            range.start
        )
    } else {
        format!(
            "Nothing to process — every page in {}–{} is also skipped",
            range.start, range.end
        )
    }
}

/// Check Pages + Skip before extracting or starting speech.
///
/// - Always catches a From–To range that Skip empties completely (no page count needed).
/// - When `page_count` is known, also checks bounds against the document and
///   whole-document / “from N to end” selections.
pub fn validate_page_selection(
    pages: &PdfPages,
    skip_pages: &[u32],
    page_count: Option<u32>,
) -> Result<()> {
    if let Some(n) = page_count {
        resolve_kept_pages(pages, n, skip_pages)?;
        return Ok(());
    }

    // Document length unknown yet — still reject a From–To range that Skip empties.
    if let PdfPages::Range(r) = pages {
        kept_pages_in(*r, skip_pages)?;
    }
    Ok(())
}

/// Pages of `range` not in `skip_pages`; an error if none are left.
fn kept_pages_in(range: PageRange, skip_pages: &[u32]) -> Result<Vec<u32>> {
    let skip: std::collections::BTreeSet<u32> = skip_pages
        .iter()
        .copied()
        .filter(|&p| p >= range.start && p <= range.end)
        .collect();
    let kept = pages_to_keep(range.start, range.end, &skip);
    if kept.is_empty() {
        return Err(CoreError::Other(empty_after_skip_message(range)));
    }
    Ok(kept)
}

/// Turn include range + skip list into a used range and the kept page numbers (from 1).
///
/// Shared by PDF extraction and approximate page selection for other formats.
pub(crate) fn resolve_kept_pages(
    pages: &PdfPages,
    page_count: u32,
    skip_pages: &[u32],
) -> Result<(PageRange, Vec<u32>)> {
    if page_count < 1 {
        return Err(CoreError::Other("document has no pages".into()));
    }
    let range = resolve_pages(pages, page_count)?;
    Ok((range, kept_pages_in(range, skip_pages)?))
}

/// Extract cleaned text for the selected pages.
///
/// `skip_pages` is a list of page numbers (from 1) to **leave out** of the include
/// range (for example blank or ad pages). Contiguous kept runs are extracted
/// separately and then joined.
pub(crate) fn extract_pdf_text(
    pdf: impl AsRef<Path>,
    pages: &PdfPages,
    skip_pages: &[u32],
) -> Result<PdfExtract> {
    let pdf = pdf.as_ref();
    ensure_pdf(pdf)?;
    let page_count = pdf_page_count_unchecked(pdf)?;
    let (range, kept) = resolve_kept_pages(pages, page_count, skip_pages)?;

    let skip_count = skip_pages
        .iter()
        .filter(|&&p| p >= range.start && p <= range.end)
        .count();
    let segments = contiguous_ranges(&kept);
    info!(
        path = %pdf.display(),
        start = range.start,
        end = range.end,
        page_count,
        skip_count,
        segments = segments.len(),
        "extracting PDF text"
    );

    let outline = pdf_outline(pdf);
    let mut raw_parts = Vec::with_capacity(segments.len());
    for seg in &segments {
        let raw = pdftotext_range(pdf, seg.start, seg.end)?;
        raw_parts.push(insert_outline_marks(&raw, seg.start, &outline));
    }
    let raw = raw_parts.join("\n");
    let text = clean_pdf_text(&raw);
    if text.split_whitespace().next().is_none() {
        // pdftotext only reads embedded text — not pictures of pages.
        let where_ = if range.start == range.end {
            format!("page {}", range.start)
        } else {
            format!("pages {}–{}", range.start, range.end)
        };
        return Err(CoreError::Other(format!(
            "Can't read {where_} of this PDF. {SCANNED_PDF_HINT}"
        )));
    }

    Ok(PdfExtract {
        page_count,
        used_range: range,
        text,
    })
}

/// Why a PDF without embedded text cannot be read, and what to do about it.
pub(crate) const SCANNED_PDF_HINT: &str = "It looks like a scan (pictures of pages, not real \
     text). Convert it with OCR to a text PDF first, then open that file. This app cannot OCR.";

/// Page numbers from `start` to `end` that are not in the skip set (counting from 1).
fn pages_to_keep(start: u32, end: u32, skip: &std::collections::BTreeSet<u32>) -> Vec<u32> {
    (start..=end).filter(|p| !skip.contains(p)).collect()
}

/// Collapse a sorted list of page numbers into inclusive ranges.
fn contiguous_ranges(pages: &[u32]) -> Vec<PageRange> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        i += 1;
        while i < pages.len() && pages[i] == end + 1 {
            end = pages[i];
            i += 1;
        }
        // Always valid for sorted ascending data from pages_to_keep.
        if let Ok(r) = PageRange::new(start, end) {
            out.push(r);
        }
    }
    out
}

/// Cap how many pages a single skip range may expand into (avoids huge memory use).
const MAX_SKIP_RANGE: u32 = 100_000;

/// Parse a skip-list string: comma-separated pages and ranges.
///
/// Examples: `5`, `5,8,25`, `90-103`, `5,8,25,90-103,145,148,155-160`.
/// Returns sorted unique page numbers (from 1).
pub fn parse_skip_pages(spec: &str) -> Result<Vec<u32>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(Vec::new());
    }
    let mut set = std::collections::BTreeSet::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let start: u32 = a
                .trim()
                .parse()
                .map_err(|_| CoreError::Other(format!("bad skip range start in '{part}'")))?;
            let end: u32 = b
                .trim()
                .parse()
                .map_err(|_| CoreError::Other(format!("bad skip range end in '{part}'")))?;
            if start < 1 || end < start {
                return Err(CoreError::Other(format!(
                    "invalid skip range '{part}' (use 1-based inclusive, start ≤ end)"
                )));
            }
            // Cap expansion so a typo like "1-4000000000" cannot exhaust memory.
            let width = end - start + 1;
            if width > MAX_SKIP_RANGE {
                return Err(CoreError::Other(format!(
                    "skip range '{part}' is too large ({width} pages; max {MAX_SKIP_RANGE})"
                )));
            }
            for p in start..=end {
                set.insert(p);
            }
        } else {
            let n: u32 = part
                .parse()
                .map_err(|_| CoreError::Other(format!("bad skip page '{part}'")))?;
            if n < 1 {
                return Err(CoreError::Other(format!("skip page must be ≥ 1 (got {n})")));
            }
            set.insert(n);
        }
    }
    Ok(set.into_iter().collect())
}

fn resolve_pages(pages: &PdfPages, page_count: u32) -> Result<PageRange> {
    match pages {
        PdfPages::All => PageRange::new(1, page_count),
        PdfPages::Range(r) => {
            if r.start > page_count {
                return Err(CoreError::Other(format!(
                    "Start page ({}) is past the end of this document ({page_count} pages)",
                    r.start
                )));
            }
            if r.end > page_count {
                return Err(CoreError::Other(format!(
                    "End page ({}) is past the end of this document ({page_count} pages)",
                    r.end
                )));
            }
            Ok(*r)
        }
        PdfPages::SkipBefore(n) => {
            // Start filled, end empty → “from N to the end”
            if *n > page_count {
                return Err(CoreError::Other(format!(
                    "Start page ({n}) is past the end of this document ({page_count} pages)"
                )));
            }
            let start = (*n).max(1);
            PageRange::new(start, page_count)
        }
    }
}

/// Chapter bookmarks as (page, title), sorted by page. Uses the top outline level,
/// or the level below it when the top level is a single wrapper entry (book title).
/// Empty if the PDF has no outline or `lopdf` cannot read it.
fn pdf_outline(pdf: &Path) -> Vec<(u32, String)> {
    let doc = match lopdf::Document::load(pdf) {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, "lopdf could not open PDF; no chapters");
            return Vec::new();
        }
    };
    let Ok(toc) = doc.get_toc() else {
        return Vec::new();
    };
    let Some(top) = toc.toc.iter().map(|t| t.level).min() else {
        return Vec::new();
    };
    let at = |level: usize| toc.toc.iter().filter(move |t| t.level == level);
    let level = if at(top).count() < 2 && at(top + 1).count() >= 2 {
        top + 1
    } else {
        top
    };
    let mut out: Vec<(u32, String)> = at(level)
        .filter_map(|t| u32::try_from(t.page).ok().map(|p| (p, t.title.clone())))
        .filter(|(p, t)| *p >= 1 && !t.trim().is_empty())
        .collect();
    out.sort_by_key(|(p, _)| *p);
    info!(bookmarks = out.len(), level, "PDF outline");
    out
}

/// Prefix bookmarked pages in one `pdftotext` run (pages `first..`, separated by form
/// feeds) with chapter markers. The first page also gets the chapter it belongs to
/// when the selection starts mid-chapter.
fn insert_outline_marks(raw: &str, first: u32, outline: &[(u32, String)]) -> String {
    if outline.is_empty() {
        return raw.to_string();
    }
    // pdftotext ends every page with a form feed; without dropping the last one the
    // split would yield an empty page after the range, which could pick up the next
    // page's bookmark.
    let (body, tail) = match raw.strip_suffix('\u{0c}') {
        Some(b) => (b, "\u{0c}"),
        None => (raw, ""),
    };
    let mut out = String::with_capacity(raw.len() + 64 * outline.len());
    for (k, page) in body.split('\u{0c}').enumerate() {
        let num = first + k as u32;
        if k > 0 {
            out.push('\u{0c}');
        }
        let here: Vec<&str> = outline
            .iter()
            .filter(|(p, _)| *p == num)
            .map(|(_, t)| t.as_str())
            .collect();
        if here.is_empty() && k == 0 {
            if let Some((_, t)) = outline.iter().rev().find(|(p, _)| *p < num) {
                out.push_str(&crate::chapters::mark(t));
            }
        }
        for t in here {
            out.push_str(&crate::chapters::mark(t));
        }
        out.push_str(page);
    }
    out.push_str(tail);
    out
}

fn pdftotext_range(pdf: &Path, start: u32, end: u32) -> Result<String> {
    // pdftotext -f START -l END -layout file.pdf -
    let out = Command::new("pdftotext")
        .arg("-f")
        .arg(start.to_string())
        .arg("-l")
        .arg(end.to_string())
        .arg("-layout")
        .arg(pdf) // Pass the path as OsStr so non-UTF-8 names still work
        .arg("-")
        .output()
        .map_err(|e| {
            CoreError::Other(format!(
                "failed to run pdftotext (install poppler-utils): {e}"
            ))
        })?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CoreError::Other(format!("pdftotext failed: {err}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Private-use sentinel standing in for a PDF form-feed page boundary.
/// Kept only while building paragraphs; never appears in output text.
const PAGE_BREAK_MARK: &str = "\u{E000}";

/// Light cleanup of extracted PDF text.
pub(crate) fn clean_pdf_text(raw: &str) -> String {
    // Form feeds mark page boundaries. Real paragraph gaps come from blank lines
    // in the layout. Reflow only stitches across page boundaries — never across
    // intentional blank-line paragraphs.
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let raw = strip_running_headers(&raw);
    let text = raw.replace('\u{0c}', &format!("\n{PAGE_BREAK_MARK}\n"));
    // Tables first, while the layout columns are still intact.
    let text = reflow_layout_tables(&text);

    let mut lines: Vec<String> = Vec::new();
    // Leading spaces of each kept line (`pdftotext -layout` keeps first-line indents).
    let mut indents: Vec<Option<usize>> = Vec::new();
    for line in text.lines() {
        let s = line.trim();
        let indent = line.chars().take_while(|c| *c == ' ').count();
        if s == PAGE_BREAK_MARK {
            lines.push(PAGE_BREAK_MARK.to_string());
            indents.push(None);
            continue;
        }
        if s.is_empty() {
            lines.push(String::new());
            indents.push(None);
            continue;
        }
        // Drop lines that are only a short page number (footer/header residue)
        if s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 4 {
            // Leaving an empty line would force a paragraph break mid-sentence
            // when the number sat between continued prose — drop the line entirely.
            continue;
        }
        // Drop table-of-contents lines with dotted leaders ending in a number
        if s.contains("....") && s.chars().last().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        lines.push(s.to_string());
        indents.push((!crate::chapters::is_mark(s)).then_some(indent));
    }
    let lines = insert_layout_paragraph_breaks(lines, &indents);
    let text = lines.join("\n");
    // Join words split across lines with a hyphen: "word-\nword" → "wordword"
    let text = regex_lite_dehyphenate(&text);
    build_paragraphs_with_page_reflow(&text)
}

/// Lines checked at each end of a page to find running headers / footers.
const RUNNING_EDGE_LINES: usize = 3;
/// Once found, a running line is removed this far in (footnotes can sit below
/// a print slug, so it isn't always in the last 3 lines).
const RUNNING_REMOVE_LINES: usize = 8;

/// Drop running headers and footers: a line in the top or bottom
/// [`RUNNING_EDGE_LINES`] of a page that repeats there on many pages
/// (`Constitution of the United States 12`, `Page 3`, print-shop slugs like
/// `VerDate Aug 31 2005 … Jkt 036932`). Lines are compared without digits, so
/// changing page numbers still match.
///
/// Needs ≥ 3 such pages and ≥ 30% of the pages; then it is removed from the
/// outer [`RUNNING_REMOVE_LINES`] of every page. A line that ends like a sentence
/// (`.` `:` `?` `!`) is never dropped (a repeated prompt or sentence is content),
/// nor is a chapter marker.
fn strip_running_headers(raw: &str) -> String {
    fn key(l: &str) -> Option<String> {
        let t = l.trim();
        if t.is_empty() || crate::chapters::is_mark(t) || t.ends_with(['.', ':', '?', '!', ';']) {
            return None;
        }
        let k: String = t
            .chars()
            .filter(|c| !c.is_ascii_digit())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        (k.chars().filter(|c| c.is_alphabetic()).count() >= 3).then_some(k)
    }
    /// Indices of the first and last few non-blank, non-marker lines.
    fn edge_lines(lines: &[&str], depth: usize) -> Vec<usize> {
        let content: Vec<usize> = (0..lines.len())
            .filter(|&i| !lines[i].trim().is_empty() && !crate::chapters::is_mark(lines[i].trim()))
            .collect();
        let mut idx: Vec<usize> = content.iter().take(depth).copied().collect();
        idx.extend(content.iter().rev().take(depth).copied());
        idx.sort_unstable();
        idx.dedup();
        idx
    }

    let pages: Vec<&str> = raw.split('\u{0c}').collect();
    let page_lines: Vec<Vec<&str>> = pages.iter().map(|p| p.split('\n').collect()).collect();
    let real_pages = page_lines
        .iter()
        .filter(|ls| ls.iter().any(|l| !l.trim().is_empty()))
        .count();
    if real_pages < 3 {
        return raw.to_string();
    }
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ls in &page_lines {
        let keys: std::collections::HashSet<String> = edge_lines(ls, RUNNING_EDGE_LINES)
            .into_iter()
            .filter_map(|i| key(ls[i]))
            .collect();
        for k in keys {
            *counts.entry(k).or_default() += 1;
        }
    }
    let need = 3.max((real_pages * 3).div_ceil(10));
    let running: std::collections::HashSet<&String> = counts
        .iter()
        .filter(|&(_, &n)| n >= need)
        .map(|(k, _)| k)
        .collect();
    if running.is_empty() {
        return raw.to_string();
    }
    page_lines
        .iter()
        .map(|ls| {
            let drop: std::collections::HashSet<usize> = edge_lines(ls, RUNNING_REMOVE_LINES)
                .into_iter()
                .filter(|&i| key(ls[i]).is_some_and(|k| running.contains(&k)))
                .collect();
            ls.iter()
                .enumerate()
                .filter(|(i, _)| !drop.contains(i))
                .map(|(_, l)| *l)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\u{0c}")
}

/// One text run of a `pdftotext -layout` line: (start column, end column, text).
type Segment = (usize, usize, String);

/// Split a layout line into runs separated by 2+ spaces.
fn layout_segments(line: &str) -> Vec<Segment> {
    let chars: Vec<char> = line.chars().collect();
    let mut segs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < chars.len() {
            if chars[end] == ' ' && chars.get(end + 1).is_none_or(|c| *c == ' ') {
                break;
            }
            end += 1;
        }
        segs.push((start, end, chars[start..end].iter().collect()));
        i = end;
    }
    segs
}

/// Rebuild tables from `pdftotext -layout` output so wrapped cells are read whole.
///
/// In layout output a table is side-by-side columns; a cell that wraps continues on
/// the next printed line *in the same column*, next to other cells' text. Read line
/// by line that interleaves cells ("Mars and its two 228 million km Known as … on its
/// small moons surface."). Here a table region is found by column starts that line up
/// across several lines (prose lines never split into aligned 2-space-separated
/// columns), each run of text is put back into its column, rows are detected, and
/// each row is emitted as its own paragraph with cells joined by commas:
/// "Mars and its two small moons, 228 million km, Known as the Red Planet … surface."
///
/// A line starts a new row when it fills every column, or when it has first-column
/// text and no cell that starts lowercase (continuations like "small moons", "to the
/// Sun"). Two-column *prose* (every column wide, many lines) is left alone.
fn reflow_layout_tables(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let segs: Vec<Vec<Segment>> = lines.iter().map(|l| layout_segments(l)).collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((end, rows, took_prev)) = detect_table(&lines, &segs, i) {
            if took_prev {
                out.pop(); // the header line was already copied through
            }
            out.push(String::new());
            for row in rows {
                out.push(row);
                out.push(String::new());
            }
            i = end;
            continue;
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    out.join("\n")
}

/// Place a line's words into known columns (words = runs of non-spaces). `None` if a
/// word straddles a column edge (prose running across the table).
fn words_into_columns(line: &str, cols: &[usize]) -> Option<Vec<Option<String>>> {
    let chars: Vec<char> = line.chars().collect();
    let mut cells: Vec<Option<String>> = vec![None; cols.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            i += 1;
            continue;
        }
        let a = i;
        while i < chars.len() && chars[i] != ' ' {
            i += 1;
        }
        let word: String = chars[a..i].iter().collect();
        let col = cols.iter().rposition(|&c| c <= a + 1)?;
        if cols.get(col + 1).is_some_and(|&next| i > next) {
            return None;
        }
        let cell = cells[col].get_or_insert_with(String::new);
        if !cell.is_empty() {
            cell.push(' ');
        }
        cell.push_str(&word);
    }
    Some(cells)
}

/// Try to read a table starting at line `start`. Returns (index after the table, rows,
/// whether the line before `start` was taken as the header row).
fn detect_table(
    lines: &[&str],
    segs: &[Vec<Segment>],
    start: usize,
) -> Option<(usize, Vec<String>, bool)> {
    let first = &segs[start];
    if first.len() < 2 || lines[start].trim() == PAGE_BREAK_MARK {
        return None;
    }
    // Column starts from the first line, confirmed by later multi-column lines.
    let cols: Vec<usize> = first.iter().map(|s| s.0).collect();
    let col_of =
        |pos: usize| -> Option<usize> { cols.iter().position(|&c| pos + 1 >= c && pos <= c + 1) };
    // Does every run of this line sit inside one column (start aligned, not crossing
    // into the next column)?
    let fits = |line_segs: &[Segment]| -> bool {
        !line_segs.is_empty()
            // Contents lines with dot leaders are dropped later, not read as tables.
            && !line_segs.iter().any(|(_, _, t)| t.contains("...."))
            // Real column gutters are wide; 2 spaces is just glyph spacing (emoji…).
            && line_segs.windows(2).all(|w| w[1].0 >= w[0].1 + 3)
            && line_segs.iter().all(|(a, b, _)| {
                col_of(*a).is_some_and(|k| cols.get(k + 1).is_none_or(|&next| *b < next))
            })
    };
    if !fits(first) {
        return None;
    }
    let mut end = start;
    let mut multi = 0;
    let mut k = start;
    while k < lines.len() {
        let l = lines[k];
        if l.trim() == PAGE_BREAK_MARK {
            break;
        }
        if l.trim().is_empty() {
            // A blank line inside a table separates rows; stop if what follows is not table.
            let next = (k + 1..lines.len()).find(|&j| !lines[j].trim().is_empty());
            match next {
                Some(j) if fits(&segs[j]) && segs[j].len() >= 2 => {
                    k = j;
                    continue;
                }
                _ => break,
            }
        }
        if !fits(&segs[k]) {
            break;
        }
        if segs[k].len() >= 2 {
            multi += 1;
        }
        end = k + 1;
        k += 1;
    }
    // At least 2 multi-column lines and 3 lines of text: a 2-line "table" is too
    // easily a repeated line with odd glyph spacing (unrendered emoji…).
    let text_lines = (start..end)
        .filter(|&j| !lines[j].trim().is_empty())
        .count();
    if multi < 2 || cols.len() < 2 || text_lines < 3 {
        return None;
    }
    // Two-column prose (all columns wide, long region): not a table.
    let mut widths = vec![0usize; cols.len()];
    for s in &segs[start..end] {
        for (a, b, _) in s {
            if let Some(c) = col_of(*a) {
                widths[c] = widths[c].max(b - a);
            }
        }
    }
    if end - start >= 6 && widths.iter().all(|&w| w >= 30) {
        return None;
    }

    // Rebuild rows. A header whose cells nearly touch ("Distance from Sun Notes", one
    // space between cells) does not split on 2+ spaces, so it fails detection; take
    // the line right before the table as the header if its words fit the columns.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut took_prev = false;
    if start > 0
        && !lines[start - 1].trim().is_empty()
        && lines[start - 1].trim() != PAGE_BREAK_MARK
    {
        if let Some(cells) = words_into_columns(lines[start - 1], &cols) {
            if cells.iter().filter(|c| c.is_some()).count() >= 2 {
                rows.push(cells.into_iter().map(Option::unwrap_or_default).collect());
                took_prev = true;
            }
        }
    }
    let mut blank_before = false;
    for (idx, line_segs) in segs[start..end].iter().enumerate() {
        if lines[start + idx].trim().is_empty() {
            blank_before = true;
            continue;
        }
        let mut cells: Vec<Option<String>> = vec![None; cols.len()];
        for (a, _, t) in line_segs {
            if let Some(c) = col_of(*a) {
                cells[c] = Some(t.clone());
            }
        }
        let filled = cells.iter().filter(|c| c.is_some()).count();
        let any_lower = cells
            .iter()
            .flatten()
            .any(|t| t.chars().next().is_some_and(char::is_lowercase));
        let new_row = rows.is_empty()
            || (took_prev && idx == 0)
            || blank_before
            || filled == cols.len()
            || (cells[0].is_some() && filled >= 2 && !any_lower);
        blank_before = false;
        if new_row {
            rows.push(vec![String::new(); cols.len()]);
        }
        let row = rows.last_mut().expect("row");
        for (c, t) in cells.into_iter().enumerate() {
            if let Some(t) = t {
                if !row[c].is_empty() {
                    // Rejoin a word hyphenated across the cell's lines.
                    if row[c].ends_with('-') && t.chars().next().is_some_and(char::is_lowercase) {
                        row[c].pop();
                    } else {
                        row[c].push(' ');
                    }
                }
                row[c].push_str(&t);
            }
        }
    }
    let spoken: Vec<String> = rows
        .into_iter()
        .map(|cells| {
            // Cells joined by a comma (a short pause), or a space after a cell that
            // already ends in punctuation ("CHAPTER I." + "Down the Rabbit-Hole").
            let mut r = String::new();
            for c in cells
                .into_iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
            {
                if !r.is_empty() {
                    r.push_str(if r.ends_with(['.', ':', ';', '!', '?', ',']) {
                        " "
                    } else {
                        ", "
                    });
                }
                r.push_str(&c);
            }
            if !r.ends_with(['.', '!', '?']) {
                r.push('.');
            }
            r
        })
        .collect();
    Some((end, spoken, took_prev))
}

/// Paragraph breaks the layout shows but no blank line marks, so headings and
/// paragraphs are not read as one run-on sentence:
///
/// - **Heading** — a short line with no closing punctuation, followed by a line
///   that starts with a capital/digit ("Chapter Three", "The Long Road North").
/// - **Paragraph end** — a short line that ends a sentence, followed by a new line.
/// - **Indent** — a line indented ≥ 2 spaces past the document's usual indent
///   (books mark paragraphs by indenting the first line).
///
/// "Short" = under 60% of the document's typical (75th-percentile) line length.
/// Length rules need ≥ 6 lines of text; a line ending in a hyphen (split word) or in
/// a function word ("of", "the", "and" — a wrapped sentence) never breaks.
fn insert_layout_paragraph_breaks(lines: Vec<String>, indents: &[Option<usize>]) -> Vec<String> {
    const WRAP_WORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "but", "of", "to", "in", "on", "at", "for", "with", "by",
        "from", "as", "that", "which", "who", "is", "was", "are", "were", "be", "if", "into",
        "than", "then", "so", "not", "nor", "its", "his", "her", "their", "our", "my", "your",
        "this", "these", "those", "has", "had", "have",
    ];
    let content: Vec<usize> = (0..lines.len()).filter(|&i| indents[i].is_some()).collect();
    if content.len() < 2 {
        return lines;
    }
    let mut lens: Vec<usize> = content.iter().map(|&i| lines[i].chars().count()).collect();
    lens.sort_unstable();
    let typical = lens[(lens.len() * 3 / 4).min(lens.len() - 1)];
    let use_length = content.len() >= 6 && typical >= 30;
    let short = |l: &str| use_length && (l.chars().count() as f32) < 0.6 * typical as f32;
    // Most common indent among content lines.
    let mut counts = std::collections::HashMap::new();
    for &i in &content {
        *counts.entry(indents[i].unwrap_or(0)).or_insert(0usize) += 1;
    }
    let base_indent = counts
        .into_iter()
        .max_by_key(|&(ind, n)| (n, std::cmp::Reverse(ind)))
        .map_or(0, |(ind, _)| ind);

    let starts_new = |l: &str| {
        l.trim_start_matches(['"', '\'', '“', '‘', '('])
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
    };
    let wraps = |l: &str| {
        let last_word = l
            .rsplit(|c: char| !c.is_alphanumeric() && c != '\'')
            .find(|w| !w.is_empty())
            .unwrap_or("")
            .to_lowercase();
        l.ends_with('-') || WRAP_WORDS.contains(&last_word.as_str())
    };
    let ends_sentence = |l: &str| l.ends_with(['.', '!', '?', '…', '"', '”', '’', ')']);
    let ends_soft = |l: &str| l.ends_with([',', ';', ':', '–', '—']);
    // A heading: short, no closing punctuation, not a wrapped sentence, and the next
    // line starts something new.
    let is_heading = |i: usize| -> bool {
        let l = lines[i].as_str();
        indents[i].is_some()
            && short(l)
            && starts_new(l)
            && !ends_sentence(l)
            && !ends_soft(l)
            && !wraps(l)
            && lines
                .get(i + 1)
                .is_some_and(|n| indents[i + 1].is_none() || starts_new(n))
    };

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 16);
    for i in 0..lines.len() {
        if i > 0 && indents[i].is_some() && indents[i - 1].is_some() {
            let prev = lines[i - 1].as_str();
            let cur = lines[i].as_str();
            let after_heading = is_heading(i - 1) && starts_new(cur);
            let before_heading = is_heading(i) && !wraps(prev) && !ends_soft(prev);
            let para_end = short(prev) && ends_sentence(prev) && starts_new(cur);
            let indented = indents[i].unwrap_or(0) >= base_indent + 2
                && indents[i - 1].unwrap_or(0) < indents[i].unwrap_or(0)
                && starts_new(cur);
            if !wraps(prev) && (after_heading || before_heading || para_end || indented) {
                out.push(String::new());
            }
        }
        out.push(lines[i].clone());
    }
    out
}

fn build_paragraphs_with_page_reflow(text: &str) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut after_page_break: Vec<bool> = Vec::new();
    let mut buf: Vec<String> = Vec::new();
    let mut para_started_after_page = false;
    let mut pending_page_break = false;
    // Outline title of the last chapter marker, while its heading lines are still
    // being read (letters/digits only, lowercased), and how much has matched so far.
    let mut title_want: Option<String> = None;
    let mut title_seen = String::new();
    // 0 = normal; 1 = title just ended, next line may be a subtitle; 2 = the subtitle
    // was the last line, end its paragraph before the next one.
    let mut subtitle_state = 0u8;

    let flush = |buf: &mut Vec<String>,
                 paras: &mut Vec<String>,
                 after_page_break: &mut Vec<bool>,
                 started_after_page: &mut bool| {
        if buf.is_empty() {
            return;
        }
        let p = collapse_spaces(&buf.join(" "));
        buf.clear();
        if p.is_empty() {
            *started_after_page = false;
            return;
        }
        after_page_break.push(*started_after_page && !paras.is_empty());
        paras.push(p);
        *started_after_page = false;
    };

    for line in text.split('\n') {
        if line == PAGE_BREAK_MARK {
            flush(
                &mut buf,
                &mut paras,
                &mut after_page_break,
                &mut para_started_after_page,
            );
            pending_page_break = true;
            continue;
        }
        if line.trim().is_empty() {
            flush(
                &mut buf,
                &mut paras,
                &mut after_page_break,
                &mut para_started_after_page,
            );
            // Keep pending_page_break: blanks right after a form feed are margin.
            continue;
        }
        // Chapter marker: always its own paragraph, never joined to its neighbours.
        if crate::chapters::is_mark(line) {
            flush(
                &mut buf,
                &mut paras,
                &mut after_page_break,
                &mut para_started_after_page,
            );
            after_page_break.push(false);
            paras.push(line.trim().to_string());
            // An outline title ending in `:` / a dash was cut short (`The Price of
            // Speed:` for a one-line `…: How Shortcuts Weaken…`): don't end there.
            title_want = crate::chapters::mark_title(line.trim())
                .filter(|t| !t.trim_end().ends_with([':', '-', '–', '—']))
                .map(title_key)
                .filter(|k| !k.is_empty());
            title_seen.clear();
            subtitle_state = 0;
            continue;
        }
        // Content — stay in the same paragraph until a blank line / page break.
        let line = line.trim().to_string();
        if subtitle_state == 2 {
            subtitle_state = 0;
            flush(
                &mut buf,
                &mut paras,
                &mut after_page_break,
                &mut para_started_after_page,
            );
        }
        let mut end_after = false;
        if subtitle_state == 1 {
            subtitle_state = if buf.is_empty() && is_subtitle_line(&line) {
                2
            } else {
                0
            };
        }
        if let Some(want) = &title_want {
            let seen = format!("{title_seen}{}", title_key(&line));
            if seen == *want {
                // Heading finished on this line: the body starts a new paragraph even
                // when the heading line is as long as body text.
                end_after = true;
                title_want = None;
                subtitle_state = 1;
            } else if want.starts_with(&seen) {
                title_seen = seen;
            } else {
                title_want = None;
            }
        }
        if buf.is_empty() {
            para_started_after_page = pending_page_break;
            pending_page_break = false;
        }
        buf.push(line);
        if end_after {
            flush(
                &mut buf,
                &mut paras,
                &mut after_page_break,
                &mut para_started_after_page,
            );
        }
    }
    flush(
        &mut buf,
        &mut paras,
        &mut after_page_break,
        &mut para_started_after_page,
    );

    merge_wrapped_titles(reflow_cross_page_paragraphs(paras, &after_page_break)).join("\n\n")
}

/// A long heading word-wrapped onto two lines (`Chapter 4.3 – How Distant AM Radio
/// Becomes Audible` / `FM Noise`) looks like two short headings, so it is split
/// into two paragraphs (read with a pause between). When the paragraphs right after
/// a chapter marker together spell the marker's outline title, join them back into
/// one heading.
fn merge_wrapped_titles(paras: Vec<String>) -> Vec<String> {
    let key = title_key;
    let mut out: Vec<String> = Vec::with_capacity(paras.len());
    let mut i = 0;
    while i < paras.len() {
        out.push(paras[i].clone());
        if let Some(title) = crate::chapters::mark_title(paras[i].trim()) {
            let want = key(title);
            let mut acc = String::new();
            let mut joined: Option<usize> = None;
            for k in 1..=4 {
                let Some(p) = paras.get(i + k) else { break };
                if crate::chapters::is_mark(p) {
                    break;
                }
                acc.push_str(&key(p));
                if !want.starts_with(&acc) || acc.is_empty() {
                    break;
                }
                if acc == want {
                    if k >= 2 {
                        joined = Some(k);
                    }
                    break;
                }
            }
            if let Some(k) = joined {
                out.push(paras[i + 1..=i + k].join(" "));
                i += k + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn title_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A one-line subtitle under a chapter heading: Title Case (`When “Maps” Lead You
/// Astray and Nobody Notices`), no closing punctuation, not a wrapped sentence.
fn is_subtitle_line(l: &str) -> bool {
    if l.ends_with(['.', '!', '?', ',', ';', ':', '-', '–', '—']) {
        return false;
    }
    let words: Vec<&str> = l
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 4)
        .collect();
    let last = l.split_whitespace().last().unwrap_or("").to_lowercase();
    let wrap_end = matches!(
        last.as_str(),
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "of"
            | "to"
            | "in"
            | "into"
            | "for"
            | "with"
            | "on"
            | "at"
            | "by"
            | "from"
    );
    words.len() >= 2
        && !wrap_end
        && words
            .iter()
            .all(|w| w.chars().next().is_some_and(char::is_uppercase))
}

/// Join only paragraph pairs separated by a page boundary, and only when the
/// text looks like one sentence split mid-stream.
fn reflow_cross_page_paragraphs(paras: Vec<String>, after_page_break: &[bool]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(paras.len());
    for (i, next) in paras.into_iter().enumerate() {
        let may_join =
            after_page_break.get(i).copied().unwrap_or(false) && !crate::chapters::is_mark(&next);
        if may_join {
            if let Some(prev) = out.last_mut() {
                if !crate::chapters::is_mark(prev) && should_join_cross_page(prev, &next) {
                    join_paragraph_pair(prev, &next);
                    continue;
                }
            }
        }
        out.push(next);
    }
    out
}

fn alphabetic_word_count(s: &str) -> usize {
    s.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .count()
}

fn should_join_cross_page(prev: &str, next: &str) -> bool {
    let prev = prev.trim_end();
    let next = next.trim_start();
    if prev.is_empty() || next.is_empty() {
        return false;
    }

    // Next continues mid-sentence (most reliable signal).
    if next
        .chars()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| c.is_lowercase())
    {
        return true;
    }

    let last = prev.chars().rev().find(|c| !c.is_whitespace());
    let Some(last) = last else {
        return false;
    };

    // Trailing openers / list connectors → not a finished paragraph.
    if matches!(
        last,
        ',' | ';'
            | '—'
            | '–'
            | '-'
            | '/'
            | '&'
            | '('
            | '['
            | '{'
            | '\u{201c}'
            | '\u{2018}'
            | '"'
            | '\''
    ) {
        return true;
    }

    // Finished sentence (allow trailing close-quotes / brackets).
    if ends_with_sentence_punct(prev) {
        return false;
    }

    // Long unfinished prose only. Short sample/UI lines ("Alpha ‒ beta … delta",
    // "Hello — world") must keep their blank-line / page pauses for TTS.
    // Count alphabetic words so bare dash tokens do not inflate the score.
    let words = alphabetic_word_count(prev);
    if last.is_alphanumeric() || last == ')' {
        if let Some(w) = prev.split_whitespace().last() {
            let alpha: String = w.chars().filter(|c| c.is_alphabetic()).collect();
            if words >= 12 && alpha.chars().next().is_some_and(|c| c.is_lowercase()) {
                return true;
            }
        }
        // Very long unfinished run cut mid-paragraph with a capital next line.
        if words >= 16 {
            return true;
        }
    }

    false
}

fn ends_with_sentence_punct(s: &str) -> bool {
    let s = s.trim_end();
    let s = s.trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')', ']', '}']);
    s.chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | '…' | ':'))
}

fn join_paragraph_pair(prev: &mut String, next: &str) {
    let next = next.trim_start();
    // "hyphen-\n\nation" across a page break
    if prev.ends_with('-')
        && prev
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| c.is_alphanumeric())
        && next.chars().next().is_some_and(|c| c.is_alphanumeric())
    {
        prev.pop();
        prev.push_str(next);
        return;
    }
    if !prev.ends_with(char::is_whitespace) {
        prev.push(' ');
    }
    prev.push_str(next);
}

fn regex_lite_dehyphenate(text: &str) -> String {
    // Remove ASCII "-\n" between word characters (line-wrap hyphenation).
    // Walk chars, not bytes, so multi-byte characters survive.
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '-'
            && i + 1 < chars.len()
            && chars[i + 1] == '\n'
            && i > 0
            && chars[i - 1].is_ascii_alphanumeric()
            && i + 2 < chars.len()
            && chars[i + 2].is_ascii_alphanumeric()
        {
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_pdf(pdf: &Path) -> Result<()> {
    if !pdf.is_file() {
        return Err(CoreError::Other(format!(
            "PDF not found: {}",
            pdf.display()
        )));
    }
    // Soft check that the required system tools are installed
    if which::which("pdftotext").is_err() {
        return Err(CoreError::Other(
            "pdftotext not found — install poppler-utils (or poppler)".into(),
        ));
    }
    if which::which("pdfinfo").is_err() {
        return Err(CoreError::Other(
            "pdfinfo not found — install poppler-utils (or poppler)".into(),
        ));
    }
    Ok(())
}

/// Format page numbers for logs: `5,8,90–103` (sorted, consecutive runs collapsed).
pub fn format_page_list(pages: &[u32]) -> String {
    if pages.is_empty() {
        return String::new();
    }
    let mut pages: Vec<u32> = pages.to_vec();
    pages.sort_unstable();
    pages.dedup();
    contiguous_ranges(&pages)
        .iter()
        .map(|r| {
            if r.start == r.end {
                r.start.to_string()
            } else {
                format!("{}–{}", r.start, r.end)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse a simple range string like `4-20` or a single page like `4`.
pub fn parse_page_range(spec: &str) -> Result<PageRange> {
    let spec = spec.trim();
    if let Some((a, b)) = spec.split_once('-') {
        let start: u32 = a
            .trim()
            .parse()
            .map_err(|_| CoreError::Other(format!("bad page start in '{spec}'")))?;
        let end: u32 = b
            .trim()
            .parse()
            .map_err(|_| CoreError::Other(format!("bad page end in '{spec}'")))?;
        PageRange::new(start, end)
    } else {
        let n: u32 = spec
            .parse()
            .map_err(|_| CoreError::Other(format!("bad page number '{spec}'")))?;
        PageRange::new(n, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(s: &str) -> Vec<String> {
        clean_pdf_text(s)
            .split("\n\n")
            .filter(|p| !p.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn blank_line_samples_stay_separate() {
        // Short blank-line blocks must not be glued.
        let raw = "\
Test A03

Alpha ‒ beta ― gamma − delta

Alpha ‒ beta ― gamma − delta

Test failed if you heard

Figure dash, horizontal bar, or circumflex

Test A04

Hello -- world
";
        let p = paras(raw);
        assert!(
            p.iter().any(|x| x == "Alpha ‒ beta ― gamma − delta"),
            "expected separate Alpha sample, got:\n{p:#?}"
        );
        assert_eq!(
            p.iter()
                .filter(|x| x.as_str() == "Alpha ‒ beta ― gamma − delta")
                .count(),
            2
        );
        assert!(p.iter().any(|x| x == "Test A04"));
        assert!(p.iter().any(|x| x == "Hello -- world"));
        // Must not be one mega-paragraph
        assert!(
            p.iter().all(|x| !x.contains("Test A04") || x == "Test A04"),
            "A04 got glued: {p:#?}"
        );
    }

    #[test]
    fn page_break_joins_lowercase_continuation() {
        let raw = "The quick brown fox jumps over the lazy\u{0c}dog on the sunny hill.";
        let p = paras(raw);
        assert_eq!(p.len(), 1, "expected join across page, got {p:#?}");
        assert!(p[0].contains("lazy dog"));
    }

    #[test]
    fn outline_title_line_ends_paragraph_and_subtitle_split() {
        let m = crate::chapters::mark("Chapter 18.1 - Practical Garden Planning");
        let text = format!(
            "{}\nChapter 18.1 - Practical Garden Planning\nWhen “Maps” Lead You Astray and Nobody Notices\nOne of the most common and frustrating problems many gardeners\nface is poor drainage in clay soil.",
            m.trim()
        );
        let out = build_paragraphs_with_page_reflow(&text);
        let paras: Vec<&str> = out.split("\n\n").collect();
        assert_eq!(paras.len(), 4, "{paras:?}");
        assert_eq!(paras[1], "Chapter 18.1 - Practical Garden Planning");
        assert_eq!(paras[2], "When “Maps” Lead You Astray and Nobody Notices");
        assert!(paras[3].starts_with("One of the most"), "{paras:?}");

        // Outline title cut short at a colon: the one-line heading stays whole.
        let m2 = crate::chapters::mark("Chapter 10.1 – The Price of Speed:");
        let text2 = format!(
            "{}\nChapter 10.1 – The Price of Speed:\nHow Shortcuts Weaken Their Bridges\n\nBody text here.",
            m2.trim()
        );
        let out2 = build_paragraphs_with_page_reflow(&text2);
        assert!(out2.contains("Speed: How Shortcuts"), "{out2}");
    }

    #[test]
    fn running_headers_and_footers_dropped() {
        let page = |n: u32, body: &str| {
            format!(
                "{n} Constitution of the United States\n\n{body}\n\nVerDate Aug 31 2005 11:{n:02} Dec 05, 2006 Jkt 036932 PO 00000 Frm 000{n:02}\n"
            )
        };
        let raw = [
            page(1, "We the People of the United States, in Order"),
            page(2, "to form a more perfect Union, establish Justice,"),
            page(3, "insure domestic Tranquility, provide for the"),
            page(4, "common defence, promote the general Welfare."),
        ]
        .join("\u{0c}");
        let out = strip_running_headers(&raw);
        assert!(!out.contains("Constitution of the United States"), "{out}");
        assert!(!out.contains("VerDate"), "{out}");
        assert!(
            out.contains("We the People") && out.contains("general Welfare."),
            "{out}"
        );
        assert_eq!(out.matches('\u{0c}').count(), 3);

        // A prompt / sentence repeated on every page is content, not a footer.
        let raw2 = (1..=4)
            .map(|n| format!("Test {n}\n\nSay hello number {n}.\n\nTest failed if you heard:\n"))
            .collect::<Vec<_>>()
            .join("\u{0c}");
        assert!(
            strip_running_headers(&raw2)
                .matches("Test failed if you heard:")
                .count()
                == 4
        );

        // Fewer than three pages: nothing is removed.
        let raw3 = "Header\nBody one\u{0c}Header\nBody two";
        assert_eq!(strip_running_headers(raw3), raw3);
    }

    #[test]
    fn wrapped_outline_title_is_one_heading() {
        let m =
            crate::chapters::mark("Chapter 4.3 – How Distant AM Radio Becomes Audible FM Noise");
        let paras = vec![
            m.trim().to_string(),
            "Chapter 4.3 – How Distant AM Radio Becomes Audible".to_string(),
            "FM Noise".to_string(),
            "The listener doesn't need to be an engineer.".to_string(),
        ];
        let out = merge_wrapped_titles(paras);
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(
            out[1],
            "Chapter 4.3 – How Distant AM Radio Becomes Audible FM Noise"
        );
        // A heading followed by a separate subtitle the title doesn't include: unchanged.
        let m2 = crate::chapters::mark("Chapter Three");
        let paras2 = vec![
            m2.trim().to_string(),
            "Chapter Three".to_string(),
            "The Long Road North".to_string(),
        ];
        assert_eq!(merge_wrapped_titles(paras2.clone()), paras2);
    }

    #[test]
    fn trailing_form_feed_adds_no_phantom_page() {
        let outline = vec![(2, "Two".to_string()), (3, "Three".to_string())];
        // pdftotext output for pages 1–2 ends with a form feed.
        let out = insert_outline_marks("one\u{0c}two\u{0c}", 1, &outline);
        assert!(out.contains("Two") && !out.contains("Three"), "{out:?}");
        assert!(out.ends_with("two\u{0c}"), "{out:?}");
    }

    #[test]
    fn page_list_collapses_runs() {
        assert_eq!(format_page_list(&[8, 5, 90, 91, 92, 5]), "5,8,90–92");
        assert_eq!(format_page_list(&[]), "");
    }

    #[test]
    fn page_break_keeps_heading_separate() {
        let raw = "Hyphen hyphen, or no pause at all\u{0c}Test A05\n\nWait go";
        let p = paras(raw);
        assert!(
            p.iter().any(|x| x == "Test A05"),
            "heading must stay separate: {p:#?}"
        );
        assert!(
            p.iter().any(|x| x == "Hyphen hyphen, or no pause at all"),
            "prev fail line must stay separate: {p:#?}"
        );
    }

    #[test]
    fn page_break_joins_trailing_comma() {
        let raw = "He opened the door, and then\u{0c}walked into the room slowly.";
        // "and then" ends alphanumeric lowercase but short; next is lowercase → join
        let p = paras(raw);
        assert_eq!(p.len(), 1, "{p:#?}");
    }

    #[test]
    fn layout_lines_join_with_spaces_blank_lines_stay_paragraphs() {
        let raw = "Hello world.\nNext line starts here.\n\nAfter blank line.";
        let out = clean_pdf_text(raw);
        assert_eq!(
            out,
            "Hello world. Next line starts here.\n\nAfter blank line."
        );
        let wrap = clean_pdf_text("Hello\nworld continues here.");
        assert_eq!(wrap, "Hello world continues here.");
    }

    #[test]
    fn headings_and_indented_paragraphs_break_without_blank_lines() {
        // pdftotext -layout style: headings on their own line, paragraphs marked by indent.
        let raw = [
            "Chapter Three",
            "The Long Road North",
            "   The morning was quiet and the road ran straight between the fields toward the distant hills,",
            "where the first light was only beginning to touch the snow on the peaks. Nobody spoke for a",
            "long time.",
            "   When they finally stopped, it was already afternoon and the horses were tired. Martha",
            "unpacked the bread and the cheese while the others watched the valley below.",
            "A Change of Plans",
            "    Nobody had expected the bridge to be gone. The river had risen in the night and carried",
            "away everything except two blackened posts on either bank.",
            "    Short line here",
            "    And then the text continues on the next line as a normal paragraph would, wrapping across",
            "the full width of the page so that it looks like real prose in a book.",
        ]
        .join("\n");
        let raw = raw.as_str();
        let p: Vec<String> = clean_pdf_text(raw)
            .split("\n\n")
            .map(str::to_string)
            .collect();
        assert_eq!(p[0], "Chapter Three", "{p:#?}");
        assert_eq!(p[1], "The Long Road North", "{p:#?}");
        assert!(
            p[2].starts_with("The morning") && p[2].ends_with("long time."),
            "{p:#?}"
        );
        assert!(p[3].starts_with("When they finally"), "{p:#?}");
        assert_eq!(p[4], "A Change of Plans", "{p:#?}");
        assert!(p.iter().any(|x| x == "Short line here"), "{p:#?}");
        // Wrapped sentences are not split.
        assert!(
            p.iter().all(|x| !x.ends_with(" a") && !x.ends_with(" of")),
            "{p:#?}"
        );
    }

    #[test]
    fn wrapped_prose_without_layout_cues_stays_joined() {
        let raw = "The committee met on Tuesday to discuss the budget for the coming year and\n\
the many proposals that had been submitted by the various departments of the\n\
university. Everyone agreed that the library needed more funding than it had\n\
received in recent years, but nobody could agree on where the money should\n\
come from or how it should be divided between the different collections and\n\
the staff who maintained them.";
        let out = clean_pdf_text(raw);
        let p: Vec<&str> = out.split("\n\n").collect();
        assert_eq!(p.len(), 1, "{p:#?}");
    }

    #[test]
    fn layout_table_cells_rejoined_per_row() {
        let raw = [
            "The following table lists the planets and a short note about each one of them.",
            "Planet               Distance from        Notes",
            "                     Sun",
            "Mercury              58 million km        The smallest planet in the solar system and the closest one",
            "                                          to the Sun, with almost no atmosphere.",
            "Venus                108 million km       Covered in thick clouds of sulfuric acid that trap heat,",
            "                                          making it the hottest planet.",
            "Earth                150 million km       Our home, the only planet known to support life.",
            "Mars and its two     228 million km       Known as the Red Planet because of iron oxide dust on its",
            "small moons                               surface.",
            "After the table, the normal text of the book continues as usual with full sentences.",
        ]
        .join("\n");
        let p: Vec<String> = clean_pdf_text(&raw)
            .split("\n\n")
            .map(str::to_string)
            .collect();
        assert_eq!(p[1], "Planet, Distance from Sun, Notes.", "{p:#?}");
        assert_eq!(
            p[2],
            "Mercury, 58 million km, The smallest planet in the solar system and the closest one to the Sun, with almost no atmosphere.",
            "{p:#?}"
        );
        assert_eq!(
            p[4], "Earth, 150 million km, Our home, the only planet known to support life.",
            "{p:#?}"
        );
        assert_eq!(
            p[5],
            "Mars and its two small moons, 228 million km, Known as the Red Planet because of iron oxide dust on its surface.",
            "{p:#?}"
        );
        assert!(p[6].starts_with("After the table"), "{p:#?}");
    }

    #[test]
    fn prose_with_double_spaces_is_not_a_table() {
        let raw = "It was late.  Nobody came to the door that night, and the house stayed quiet.\n\
The next morning  everyone had already left for the market in the town square.";
        let out = clean_pdf_text(raw);
        assert_eq!(
            out,
            "It was late. Nobody came to the door that night, and the house stayed quiet. \
             The next morning everyone had already left for the market in the town square."
        );
    }

    #[test]
    fn emoji_spacing_and_dot_leaders_are_not_tables() {
        let raw = "Hello    world 🚀 done\n\nHello    world 🚀 done";
        let out = clean_pdf_text(raw);
        assert!(!out.contains("Hello, world"), "{out}");
        let toc = "Historical Note ...................... v\nText of the Constitution ............ 1\nAmendments ............................ 13";
        let out = clean_pdf_text(toc);
        assert!(!out.contains(", v") && !out.contains(", 1"), "{out}");
    }

    #[test]
    fn table_header_with_touching_cells_is_kept() {
        let raw = [
            "Planet                Distance from Sun Notes",
            "Mercury               58 million km      The smallest planet in the solar system and the",
            "                                         closest one to the Sun, with almost no atmosphere.",
            "Mars and its two       228 million km    Known as the Red Planet because of iron oxide dust",
            "small moons                              on its surface.",
        ]
        .join("\n");
        let p: Vec<String> = clean_pdf_text(&raw)
            .split("\n\n")
            .map(str::to_string)
            .collect();
        assert_eq!(p[0], "Planet, Distance from Sun, Notes.", "{p:#?}");
        assert!(
            p[1].starts_with("Mercury, 58 million km, The smallest"),
            "{p:#?}"
        );
        assert!(p[2].ends_with("iron oxide dust on its surface."), "{p:#?}");
        // A prose line before a table is not taken as its header.
        let raw2 =
            format!("The table below shows how far each planet is from the Sun today.\n{raw}");
        let q = clean_pdf_text(&raw2);
        assert!(
            q.starts_with("The table below shows how far each planet is from the Sun today."),
            "{q}"
        );
    }
}

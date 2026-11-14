//! Pull plain text out of many document types.
//!
//! Supported: **PDF**, **EPUB**, **TXT**, **DOCX** (Word), **ODT** (LibreOffice),
//! and Amazon **MOBI/AZW/AZW3** when they are not DRM-locked. DRM-protected files
//! return [`CoreError::DrmProtected`].

use crate::chapters;
use crate::error::{CoreError, Result};
use crate::pdf::{self, clean_pdf_text, format_page_list, resolve_kept_pages, PdfPages};
use mobi::headers::Encryption;
use mobi::Mobi;
use std::io::Read;
use std::path::Path;
use tracing::{info, warn};
use zip::ZipArchive;

/// Rough word count per “virtual page” for formats that are not real PDFs.
///
/// True printed page numbers are unknown without a full layout engine; this is
/// only so From/To/Skip fields work on continuous text (EPUB, TXT, DOCX, and so on).
pub(crate) const APPROX_WORDS_PER_PAGE: usize = 250;

/// What kind of document this is, based on the file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Pdf,
    Epub,
    Txt,
    Docx,
    Odt,
    /// Older Amazon/Kindle ebook formats (MOBI, PRC, AZW, often AZW3 as a shell).
    Mobi,
    /// Modern Kindle KFX package — not readable here without Amazon tools; handle carefully.
    Kfx,
}

impl DocKind {
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => Ok(Self::Pdf),
            "epub" => Ok(Self::Epub),
            "txt" | "text" | "md" => Ok(Self::Txt),
            "docx" => Ok(Self::Docx),
            "odt" => Ok(Self::Odt),
            // Amazon / Kindle family
            "mobi" | "prc" | "azw" | "azw3" => Ok(Self::Mobi),
            "kfx" | "kfx-zip" => Ok(Self::Kfx),
            _ => Err(CoreError::Other(format!(
                "unsupported file type '.{ext}' — use pdf, epub, txt, docx, odt, mobi, azw, or azw3"
            ))),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Epub => "EPUB",
            Self::Txt => "Text",
            Self::Docx => "Word (DOCX)",
            Self::Odt => "OpenDocument (ODT)",
            Self::Mobi => "Amazon / Kindle (MOBI·AZW·AZW3)",
            Self::Kfx => "Amazon KFX",
        }
    }
}

/// Options for reading a document (page range and skip list for every supported format).
///
/// - **PDF:** real page numbers from the PDF tools (accurate).
/// - **Other formats:** approximate “pages” of about [`APPROX_WORDS_PER_PAGE`] words each
///   (or form-feed breaks when present). These are not real printed page numbers.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub pdf_pages: PdfPages,
    /// Page numbers to leave out of the chosen range (counting from 1).
    pub skip_pages: Vec<u32>,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            pdf_pages: PdfPages::All,
            skip_pages: Vec::new(),
        }
    }
}

/// Result of loading a document into plain text ready for speech.
#[derive(Debug, Clone)]
pub struct DocExtract {
    pub kind: DocKind,
    pub text: String,
    pub word_count: usize,
    /// Short human note, e.g. "pages 4–20 of 170" or "all chapters".
    pub detail: String,
    /// Total pages (real PDF pages, or approximate units for other formats).
    pub page_count: Option<u32>,
    /// Chapters found (0 = none; the audio will be one untitled section).
    /// `text` carries them as [`chapters`] markers.
    pub chapter_count: usize,
}

/// Read a supported document and return cleaned plain text.
pub fn extract_document(path: impl AsRef<Path>, opts: &ExtractOptions) -> Result<DocExtract> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(CoreError::Other(format!(
            "file not found: {}",
            path.display()
        )));
    }
    let kind = DocKind::from_path(path)?;
    info!(path = %path.display(), kind = kind.label(), "extracting document");

    let (text, detail, page_count) = match kind {
        DocKind::Pdf => {
            let ex = pdf::extract_pdf_text(path, &opts.pdf_pages, &opts.skip_pages)?;
            let mut detail = if ex.used_range.start == ex.used_range.end {
                format!("page {} of {}", ex.used_range.start, ex.page_count)
            } else {
                format!(
                    "pages {}–{} of {}",
                    ex.used_range.start, ex.used_range.end, ex.page_count
                )
            };
            append_skip_detail(&mut detail, &opts.skip_pages);
            (ex.text, detail, Some(ex.page_count))
        }
        DocKind::Txt => {
            let t = chapters::mark_plain_headings(&clean_pdf_text(&clean_plain(&read_text_file(
                path,
            )?)));
            let (text, detail, n) =
                select_approx_pages(&t, &opts.pdf_pages, &opts.skip_pages, "file")?;
            (text, detail, Some(n))
        }
        DocKind::Docx => {
            let t = clean_pdf_text(&clean_plain(&extract_docx(path)?));
            let (text, detail, n) =
                select_approx_pages(&t, &opts.pdf_pages, &opts.skip_pages, "document")?;
            (text, detail, Some(n))
        }
        DocKind::Odt => {
            let t = clean_pdf_text(&clean_plain(&extract_odt(path)?));
            let (text, detail, n) =
                select_approx_pages(&t, &opts.pdf_pages, &opts.skip_pages, "document")?;
            (text, detail, Some(n))
        }
        DocKind::Epub => {
            if epub_has_drm(path) {
                return Err(CoreError::drm("Adobe DRM (EPUB encryption.xml)"));
            }
            let t = clean_pdf_text(&clean_plain(&extract_epub(path)?));
            let (text, detail, n) =
                select_approx_pages(&t, &opts.pdf_pages, &opts.skip_pages, "EPUB text")?;
            (text, detail, Some(n))
        }
        DocKind::Mobi => {
            let t = chapters::mark_plain_headings(&clean_pdf_text(&clean_plain(
                &extract_amazon_kindle(path)?,
            )));
            let (text, detail, n) = select_approx_pages(
                &t,
                &opts.pdf_pages,
                &opts.skip_pages,
                "Kindle text (DRM-free)",
            )?;
            (text, detail, Some(n))
        }
        DocKind::Kfx => {
            return Err(CoreError::drm(
                "KFX is Amazon’s modern delivery format and is DRM-protected / not readable here",
            ));
        }
    };

    let spoken = chapters::strip_marks(&text);
    let word_count = spoken.split_whitespace().count();
    let chapter_count = chapters::chapter_titles(&text).len();
    if word_count == 0 {
        // PDFs with only outline markers and no text reach here.
        let msg = if matches!(kind, DocKind::Pdf) {
            format!("Can't read this PDF. {}", pdf::SCANNED_PDF_HINT)
        } else {
            format!(
                "Can't read this {} file — no text was found to speak.",
                kind.label()
            )
        };
        return Err(CoreError::Other(msg));
    }

    Ok(DocExtract {
        kind,
        text,
        word_count,
        detail,
        page_count,
        chapter_count,
    })
}

/// Split continuous text into virtual pages for approximate From/To/Skip.
///
/// Prefer form-feed page breaks (the `\u{0c}` character) when present; otherwise
/// cut every ~[`APPROX_WORDS_PER_PAGE`] words.
pub(crate) fn split_approx_pages(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Form-feed breaks are common in plain-text exports from PDF tools.
    if trimmed.contains('\u{0c}') {
        let parts: Vec<String> = trimmed
            .split('\u{0c}')
            .map(|s| s.trim().to_string())
            .collect();
        // Only treat them as pages when there is more than one piece.
        if parts.len() > 1 {
            // Keep empty pieces as blank pages so page numbers stay stable.
            return parts;
        }
    }
    // Cut every ~APPROX_WORDS_PER_PAGE words, but keep paragraph breaks and chapter
    // markers (which count as zero words) inside each page.
    let mut pages: Vec<String> = Vec::new();
    let mut page: Vec<String> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut words = 0usize;
    let flush_para = |para: &mut Vec<&str>, page: &mut Vec<String>| {
        if !para.is_empty() {
            page.push(para.join(" "));
            para.clear();
        }
    };
    for p in trimmed.split("\n\n") {
        if chapters::is_mark(p) {
            flush_para(&mut para, &mut page);
            page.push(p.trim().to_string());
            continue;
        }
        for w in p.split_whitespace() {
            para.push(w);
            words += 1;
            if words == APPROX_WORDS_PER_PAGE {
                flush_para(&mut para, &mut page);
                pages.push(page.join("\n\n"));
                page.clear();
                words = 0;
            }
        }
        flush_para(&mut para, &mut page);
    }
    if !page.is_empty() {
        if words == 0 && !pages.is_empty() {
            // Only markers left: keep them on the last real page.
            let last = pages.last_mut().expect("non-empty");
            last.push_str("\n\n");
            last.push_str(&page.join("\n\n"));
        } else {
            pages.push(page.join("\n\n"));
        }
    }
    pages
}

/// Last chapter marker title found in `units` (searching backwards).
fn last_mark_in(units: &[String]) -> Option<String> {
    units.iter().rev().find_map(|u| {
        u.split("\n\n")
            .filter_map(chapters::mark_title)
            .last()
            .map(str::to_string)
    })
}

/// Apply From–To and skip list to approximate pages of already-extracted text.
///
/// Returns `(text, detail, total_approx_page_count)`.
fn select_approx_pages(
    full_text: &str,
    pages: &PdfPages,
    skip_pages: &[u32],
    source_label: &str,
) -> Result<(String, String, u32)> {
    let units = split_approx_pages(full_text);
    if units.is_empty() {
        return Ok((String::new(), format!("empty {source_label}"), 0));
    }
    let page_count = u32::try_from(units.len()).unwrap_or(u32::MAX);
    let wants_all = matches!(pages, PdfPages::All) && skip_pages.is_empty();
    if wants_all {
        return Ok((
            full_text.trim().to_string(),
            format!(
                "all of {source_label} · ~{page_count} approx pages (~{APPROX_WORDS_PER_PAGE} words each)"
            ),
            page_count,
        ));
    }

    let (range, kept) = resolve_kept_pages(pages, page_count, skip_pages)?;
    let mut parts: Vec<String> = Vec::with_capacity(kept.len());
    let mut prev_kept: Option<usize> = None;
    for p in &kept {
        let idx = (*p as usize).saturating_sub(1);
        if let Some(unit) = units.get(idx) {
            // Starting after a gap (From > 1 or skipped pages): re-open the chapter
            // this page belongs to, so its audio is labelled correctly.
            let after_gap = prev_kept.is_none_or(|q| q + 1 != idx);
            let starts_with_mark = unit.trim_start().starts_with(chapters::MARK_START);
            if after_gap && !starts_with_mark {
                if let Some(t) = last_mark_in(&units[..idx]) {
                    parts.push(chapters::mark(&t).trim().to_string());
                }
            }
            prev_kept = Some(idx);
            if !unit.trim().is_empty() {
                parts.push(unit.clone());
            }
        }
    }
    let text = parts.join("\n\n");
    let mut detail = if range.start == range.end {
        format!(
            "approx page {} of {} ({source_label}; ~{APPROX_WORDS_PER_PAGE} words/page)",
            range.start, page_count
        )
    } else {
        format!(
            "approx pages {}–{} of {} ({source_label}; ~{APPROX_WORDS_PER_PAGE} words/page)",
            range.start, range.end, page_count
        )
    };
    append_skip_detail(&mut detail, skip_pages);
    detail.push_str(" · not real print page numbers");
    Ok((text, detail, page_count))
}

fn append_skip_detail(detail: &mut String, skip_pages: &[u32]) {
    use std::fmt::Write as _;

    if skip_pages.is_empty() {
        return;
    }
    let skip_label = if skip_pages.len() == 1 {
        "skip page"
    } else {
        "skip pages"
    };
    let _ = write!(detail, " · {skip_label} {}", format_page_list(skip_pages));
}

fn read_text_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_string());
    }
    // Not UTF-8: decode as Windows-1252, the usual legacy encoding for text files.
    Ok(bytes.iter().map(|&b| crate::mobi_text::cp1252(b)).collect())
}

fn clean_plain(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path)?;
    ZipArchive::new(f).map_err(|e| CoreError::Other(format!("zip open failed: {e}")))
}

fn read_zip_str(zip: &mut ZipArchive<std::fs::File>, name: &str) -> Result<String> {
    let mut f = zip
        .by_name(name)
        .map_err(|e| CoreError::Other(format!("zip entry '{name}': {e}")))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

// --- Word DOCX files ---

fn extract_docx(path: &Path) -> Result<String> {
    let mut zip = open_zip(path)?;
    let xml = read_zip_str(&mut zip, "word/document.xml")
        .map_err(|_| CoreError::Other("not a valid DOCX (missing word/document.xml)".into()))?;
    Ok(docx_xml_to_text(&xml))
}

fn docx_xml_to_text(xml: &str) -> String {
    // Chapters = top-level heading paragraphs (Heading1, or Heading2 if there is none).
    let heading_style = ["Heading1", "Heading2"]
        .into_iter()
        .find(|h| xml.contains(&format!("w:pStyle w:val=\"{h}\"")))
        .map(|h| format!("w:pStyle w:val=\"{h}\""));
    // New paragraph on </w:p>; actual words live inside <w:t>...</w:t>.
    // Tags are matched on bytes; `xml` is only sliced at a matched `<` or after a
    // `>`, which are always char boundaries.
    let mut out = String::new();
    let mut i = 0;
    let bytes = xml.as_bytes();
    while i < bytes.len() {
        let at = &bytes[i..];
        if let Some(style) = &heading_style {
            if at.starts_with(b"<w:p>") || at.starts_with(b"<w:p ") {
                let end = xml[i..].find("</w:p>").map_or(xml.len(), |e| i + e);
                let para = &xml[i..end];
                if para.contains(style.as_str()) {
                    let title = decode_xml_entities(&docx_runs_text(para));
                    if !title.trim().is_empty() {
                        out.push_str(&chapters::mark(&title));
                    }
                }
            }
        }
        if at.starts_with(b"<w:t>") || at.starts_with(b"<w:t ") {
            // Find the end of the opening tag, then read until </w:t>.
            if let Some(gt) = xml[i..].find('>') {
                let start = i + gt + 1;
                if let Some(end_rel) = xml[start..].find("</w:t>") {
                    out.push_str(&decode_xml_entities(&xml[start..start + end_rel]));
                    i = start + end_rel + 6;
                    continue;
                }
            }
        }
        if at.starts_with(b"</w:p>") {
            out.push_str("\n\n");
            i += 6;
            continue;
        }
        if at.starts_with(b"<w:tab/>") {
            out.push('\t');
        }
        if at.starts_with(b"<w:br") {
            out.push('\n');
        }
        i += 1;
    }
    out
}

/// Concatenated `<w:t>` text inside one DOCX paragraph.
fn docx_runs_text(para: &str) -> String {
    let mut t = String::new();
    let mut rest = para;
    while let Some(idx) = rest.find("<w:t") {
        let after = &rest[idx..];
        // Skip <w:tab/>, <w:tbl…> etc.: only <w:t> or <w:t …>.
        let is_text = after[4..].starts_with('>') || after[4..].starts_with(' ');
        let Some(gt) = after.find('>') else { break };
        if is_text {
            if let Some(end) = after[gt + 1..].find("</w:t>") {
                t.push_str(&after[gt + 1..gt + 1 + end]);
            }
        }
        rest = &after[gt + 1..];
    }
    t
}

// --- OpenDocument ODT files ---

fn extract_odt(path: &Path) -> Result<String> {
    let mut zip = open_zip(path)?;
    let xml = read_zip_str(&mut zip, "content.xml")
        .map_err(|_| CoreError::Other("not a valid ODT (missing content.xml)".into()))?;
    Ok(odt_xml_to_text(&xml))
}

fn odt_xml_to_text(xml: &str) -> String {
    // Chapters = headings at the top outline level used in the document.
    let top_level = (1..=3).find(|l| xml.contains(&format!("text:outline-level=\"{l}\"")));
    // Tags are matched on bytes; `xml` is only sliced at a matched `<` or after a
    // `>`, which are always char boundaries.
    let bytes = xml.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<text:p") || bytes[i..].starts_with(b"<text:h") {
            // Simple approach: collect text until the matching close tag.
            if let Some(gt) = xml[i..].find('>') {
                let start = i + gt + 1;
                let close_p = xml[start..].find("</text:p>");
                let close_h = xml[start..].find("</text:h>");
                // Both closing tags are 9 bytes long: `</text:p>` / `</text:h>`.
                let (end_rel, close_len) = match (close_p, close_h) {
                    (Some(a), Some(b)) => (a.min(b), 9),
                    (Some(a), None) | (None, Some(a)) => (a, 9),
                    (None, None) => {
                        i += 1;
                        continue;
                    }
                };
                let chunk = &xml[start..start + end_rel];
                let tag = &xml[i..start];
                if let Some(l) = top_level {
                    if tag.starts_with("<text:h")
                        && tag.contains(&format!("text:outline-level=\"{l}\""))
                    {
                        let title = decode_xml_entities(&odt_strip_tags(chunk));
                        if !title.trim().is_empty() {
                            out.push_str(&chapters::mark(&title));
                        }
                    }
                }
                out.push_str(&decode_xml_entities(&odt_strip_tags(chunk)));
                out.push_str("\n\n");
                i = start + end_rel + close_len;
                continue;
            }
        }
        i += 1;
    }
    out
}

// --- Amazon Kindle family (MOBI / AZW / AZW3 shell) ---

fn epub_has_drm(path: &Path) -> bool {
    let Ok(mut zip) = open_zip(path) else {
        return false;
    };
    if zip.by_name("META-INF/rights.xml").is_ok() {
        return true;
    }
    if zip.by_name("META-INF/encryption.xml").is_err() {
        return false;
    }
    // Unreadable encryption.xml: assume the worst.
    read_zip_str(&mut zip, "META-INF/encryption.xml").map_or(true, |x| encryption_xml_is_drm(&x))
}

/// Font-obfuscation algorithms that DRM-free EPUBs use in `encryption.xml`.
const FONT_OBFUSCATION: [&str; 2] = [
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

/// True when `encryption.xml` encrypts anything other than obfuscated fonts.
fn encryption_xml_is_drm(xml: &str) -> bool {
    const FONT_EXTS: [&str; 5] = ["otf", "ttf", "ttc", "woff", "woff2"];
    // Each piece after an `EncryptedData` tag name holds that entry's method and
    // target (closing tags give pieces with neither).
    xml.split("EncryptedData").skip(1).any(|entry| {
        let alg_is_drm =
            attr(entry, "Algorithm").is_some_and(|a| !FONT_OBFUSCATION.contains(&a.trim()));
        let target_is_drm = attr(entry, "URI").is_some_and(|uri| {
            let ext = Path::new(uri.split(['#', '?']).next().unwrap_or(&uri))
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            !ext.is_some_and(|e| FONT_EXTS.contains(&e.as_str()))
        });
        alg_is_drm || target_is_drm
    })
}

/// Open DRM-free Amazon/Kindle PalmDB books (MOBI, many AZW, some AZW3).
fn extract_amazon_kindle(path: &Path) -> Result<String> {
    let m = Mobi::from_path(path).map_err(|e| {
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("encrypt") || msg.contains("drm") || msg.contains("password") {
            CoreError::drm(format!("parser rejected encrypted Kindle file: {e}"))
        } else {
            // Many locked AZW3 files fail to parse cleanly — treat Amazon extensions as DRM
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "azw" | "azw3" | "mobi") {
                warn!(error = %e, "Kindle open failed; treating as likely DRM/unsupported");
                CoreError::drm(format!(
                    "could not open Amazon/Kindle file (often means DRM or KFX): {e}"
                ))
            } else {
                CoreError::Other(format!("failed to open MOBI/Kindle file: {e}"))
            }
        }
    })?;

    match m.encryption() {
        Encryption::No => {}
        other => {
            return Err(CoreError::drm(format!(
                "PalmDOC encryption flag set ({other:?})"
            )));
        }
    }

    // Our own record decoder strips MOBI trailing entries correctly; the crate's
    // decoder (used for Huffman-compressed books) leaves junk at record boundaries.
    let own = std::fs::read(path)
        .ok()
        .and_then(|b| crate::mobi_text::decode(&b));
    let raw = match own.map_or_else(|| m.content_as_string(), Ok) {
        Ok(s) => s,
        Err(e) => {
            // A looser decode may return garbage for DRM; check emptiness after stripping markup
            warn!(error = %e, "strict MOBI decode failed, trying lossy");
            let lossy = m.content_as_string_lossy();
            if lossy.trim().is_empty() || looks_like_encrypted_garbage(&lossy) {
                return Err(CoreError::drm(
                    "content decode failed — file is likely DRM protected",
                ));
            }
            lossy
        }
    };

    let text = strip_html(&mark_html_headings(&raw));
    if text.chars().filter(|c| c.is_alphanumeric()).count() < 20
        || looks_like_encrypted_garbage(&text)
    {
        return Err(CoreError::drm(
            "no readable text extracted — file is likely DRM protected or image-only",
        ));
    }

    let title = m.title();
    if title.is_empty() {
        Ok(text)
    } else {
        Ok(format!("{title}\n\n{text}"))
    }
}

/// Rough check for high-entropy nonsense “text” that often means a failed DRM decrypt.
fn looks_like_encrypted_garbage(s: &str) -> bool {
    let sample: String = s.chars().take(500).collect();
    if sample.is_empty() {
        return true;
    }
    let printable = sample
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .count();
    let ratio = printable as f32 / sample.chars().count().max(1) as f32;
    ratio < 0.45
}

// --- EPUB ebooks ---

fn extract_epub(path: &Path) -> Result<String> {
    let mut zip = open_zip(path)?;
    let container = read_zip_str(&mut zip, "META-INF/container.xml").map_err(|_| {
        CoreError::Other("not a valid EPUB (missing META-INF/container.xml)".into())
    })?;
    let opf_path = parse_opf_path(&container)?;
    let opf = read_zip_str(&mut zip, &opf_path)
        .map_err(|e| CoreError::Other(format!("EPUB OPF '{opf_path}': {e}")))?;
    let base = parent_path(&opf_path);
    let spine = parse_epub_spine(&opf)?;
    let toc = epub_toc(&mut zip, &opf, &base);
    let use_headings = toc.is_empty();
    info!(toc_entries = toc.len(), "EPUB table of contents");
    let mut parts = Vec::new();
    for href in spine {
        let full = join_path(&base, &href);
        // Try the path as written, and again with %20 decoded to spaces.
        let full_dec = full.replace("%20", " ");
        let html = read_zip_str(&mut zip, &full)
            .or_else(|_| read_zip_str(&mut zip, &full_dec))
            .map_err(|e| CoreError::Other(format!("EPUB chapter '{full}': {e}")))?;
        let entries: Vec<&TocEntry> = toc
            .iter()
            .filter(|e| e.file == full || e.file == full_dec)
            .collect();
        let html = if use_headings {
            mark_html_headings(&html)
        } else {
            insert_toc_marks(&html, &entries)
        };
        let t = strip_html(&html);
        if !t.trim().is_empty() {
            parts.push(t);
        }
    }
    if parts.is_empty() {
        return Err(CoreError::Other("EPUB has no readable chapters".into()));
    }
    Ok(parts.join("\n\n"))
}

/// One table-of-contents entry: target file (zip path), optional `#fragment`, title.
struct TocEntry {
    file: String,
    frag: Option<String>,
    title: String,
}

/// Read the EPUB 3 nav document, or else the EPUB 2 `toc.ncx`. Empty if neither.
fn epub_toc(zip: &mut ZipArchive<std::fs::File>, opf: &str, base: &str) -> Vec<TocEntry> {
    let mut nav_href = None;
    let mut ncx_href = None;
    let mut search = opf;
    while let Some(idx) = search.find("<item ") {
        let rest = &search[idx..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        if let Some(href) = attr(tag, "href") {
            if attr(tag, "properties").is_some_and(|p| p.split_whitespace().any(|x| x == "nav")) {
                nav_href = Some(href);
            } else if attr(tag, "media-type").as_deref() == Some("application/x-dtbncx+xml") {
                ncx_href = Some(href);
            }
        }
        search = &rest[1..];
    }

    let mut entries = Vec::new();
    if let Some(href) = nav_href {
        let path = join_path(base, &href);
        if let Ok(nav) = read_zip_str(zip, &path) {
            // The <nav epub:type="toc"> block (or the first <nav> as a fallback).
            let start = nav
                .find("epub:type=\"toc\"")
                .or_else(|| nav.find("<nav"))
                .unwrap_or(0);
            let block = &nav[start..nav[start..].find("</nav>").map_or(nav.len(), |e| start + e)];
            let mut rest = block;
            while let Some(a) = rest.find("<a ") {
                let tail = &rest[a..];
                let Some(gt) = tail.find('>') else { break };
                let Some(close) = tail.find("</a>") else {
                    break;
                };
                if let Some(h) = attr(&tail[..gt], "href") {
                    push_toc(&mut entries, &path, &h, &tail[gt + 1..close]);
                }
                rest = &tail[close..];
            }
        }
    }
    if entries.is_empty() {
        if let Some(href) = ncx_href {
            let path = join_path(base, &href);
            if let Ok(ncx) = read_zip_str(zip, &path) {
                let mut rest = ncx.as_str();
                while let Some(np) = rest.find("<navPoint") {
                    let tail = &rest[np + 9..];
                    let label = tail.find("<text>").and_then(|a| {
                        tail[a + 6..]
                            .find("</text>")
                            .map(|b| &tail[a + 6..a + 6 + b])
                    });
                    let src = tail.find("<content").and_then(|c| attr(&tail[c..], "src"));
                    if let (Some(label), Some(src)) = (label, src) {
                        push_toc(&mut entries, &path, &src, label);
                    }
                    rest = tail;
                }
            }
        }
    }
    entries
}

/// Add one TOC link (resolved against the nav/ncx document's folder).
fn push_toc(entries: &mut Vec<TocEntry>, doc_path: &str, href: &str, title: &str) {
    // Still entity-encoded: the marker goes back into the HTML, and `strip_html`
    // decodes it with the rest of the text.
    let title = strip_xml_tags(title);
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || href.starts_with("http") {
        return;
    }
    let (file, frag) = match href.split_once('#') {
        Some((f, fr)) => (f, Some(fr.to_string())),
        None => (href, None),
    };
    let file = if file.is_empty() {
        doc_path.to_string()
    } else {
        join_path(&parent_path(doc_path), file).replace("%20", " ")
    };
    entries.push(TocEntry { file, frag, title });
}

/// Put a chapter marker where each TOC entry points inside this chapter file:
/// at the element with the fragment id, or at the start of `<body>`.
fn insert_toc_marks(html: &str, entries: &[&TocEntry]) -> String {
    if entries.is_empty() {
        return html.to_string();
    }
    let body = html
        .find("<body")
        .and_then(|b| html[b..].find('>').map(|g| b + g + 1))
        .unwrap_or(0);
    let mut at: Vec<(usize, &str)> = entries
        .iter()
        .map(|e| {
            let pos = e
                .frag
                .as_deref()
                .and_then(|f| {
                    [format!("id=\"{f}\""), format!("id='{f}'")]
                        .iter()
                        .find_map(|needle| html.find(needle.as_str()))
                        .and_then(|i| html[..i].rfind('<'))
                })
                .unwrap_or(body);
            (pos.max(body), e.title.as_str())
        })
        .collect();
    at.sort_by_key(|(p, _)| *p);
    let mut out = String::with_capacity(html.len() + 64 * at.len());
    let mut last = 0;
    for (pos, title) in at {
        out.push_str(&html[last..pos]);
        out.push_str(&chapters::mark(title));
        last = pos;
    }
    out.push_str(&html[last..]);
    out
}

/// No table of contents: use `<h1>` (or `<h2>`) headings as chapter starts.
fn mark_html_headings(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(tag) = ["h1", "h2"]
        .into_iter()
        .find(|t| lower.contains(&format!("<{t}")))
    else {
        return html.to_string();
    };
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(html.len() + 256);
    let mut last = 0;
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&open) {
        let pos = from + rel;
        let Some(end_rel) = lower[pos..].find(&close) else {
            break;
        };
        let inner_start = html[pos..].find('>').map_or(pos, |g| pos + g + 1);
        // Left entity-encoded; `strip_html` decodes the marker with the rest.
        let title = strip_xml_tags(&html[inner_start.min(pos + end_rel)..pos + end_rel]);
        out.push_str(&html[last..pos]);
        if !title.trim().is_empty() {
            out.push_str(&chapters::mark(&title));
        }
        last = pos;
        from = pos + end_rel;
    }
    out.push_str(&html[last..]);
    out
}

fn parse_opf_path(container_xml: &str) -> Result<String> {
    // Look for full-path="OEBPS/content.opf" or full-path='...'
    for key in ["full-path=\"", "full-path='"] {
        let mut search = container_xml;
        while let Some(idx) = search.find(key) {
            let rest = &search[idx + key.len()..];
            let end_ch = if key.ends_with('"') { '"' } else { '\'' };
            if let Some(end) = rest.find(end_ch) {
                let path = rest[..end].trim().to_string();
                let is_opf = Path::new(&path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("opf"));
                if is_opf || path.contains('/') || path.contains('.') {
                    return Ok(path);
                }
            }
            search = &search[idx + 1..];
        }
    }
    // Fallback: find a rootfile tag and read its full-path attribute
    if let Some(rf) = container_xml.find("rootfile") {
        let slice = &container_xml[rf..];
        if let Some(p) = attr(slice, "full-path") {
            return Ok(p);
        }
    }
    Err(CoreError::Other(format!(
        "could not find OPF path in container.xml (snippet: {})",
        container_xml.chars().take(120).collect::<String>()
    )))
}

fn parse_epub_spine(opf: &str) -> Result<Vec<String>> {
    // Map each item id to its file path from the package manifest
    let mut id_href: Vec<(String, String)> = Vec::new();
    let mut search = opf;
    while let Some(idx) = search.find("<item ") {
        let rest = &search[idx..];
        let end = rest
            .find("/>")
            .or_else(|| rest.find('>'))
            .unwrap_or(rest.len());
        let tag = &rest[..end];
        let id = attr(tag, "id");
        let href = attr(tag, "href");
        if let (Some(id), Some(href)) = (id, href) {
            id_href.push((id, href));
        }
        search = &rest[1..];
    }

    // Reading order comes from spine itemref idref=...
    let mut hrefs = Vec::new();
    let mut search = opf;
    while let Some(idx) = search.find("<itemref ") {
        let rest = &search[idx..];
        let end = rest
            .find("/>")
            .or_else(|| rest.find('>'))
            .unwrap_or(rest.len());
        let tag = &rest[..end];
        if let Some(idref) = attr(tag, "idref") {
            if let Some((_, href)) = id_href.iter().find(|(id, _)| id == &idref) {
                hrefs.push(href.clone());
            }
        }
        search = &rest[1..];
    }

    if hrefs.is_empty() {
        // Fallback: every HTML/XHTML file listed in the manifest
        for (_, href) in &id_href {
            let is_html = Path::new(href).extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("xhtml")
                    || ext.eq_ignore_ascii_case("html")
                    || ext.eq_ignore_ascii_case("htm")
            });
            if is_html {
                hrefs.push(href.clone());
            }
        }
    }
    if hrefs.is_empty() {
        return Err(CoreError::Other("EPUB spine is empty".into()));
    }
    Ok(hrefs)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    for q in ['"', '\''] {
        let key = format!("{name}={q}");
        if let Some(idx) = tag.find(&key) {
            let rest = &tag[idx + key.len()..];
            if let Some(end) = rest.find(q) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn parent_path(p: &str) -> String {
    match p.rfind('/') {
        Some(i) => p[..i].to_string(),
        None => String::new(),
    }
}

fn join_path(base: &str, href: &str) -> String {
    if href.starts_with('/') {
        return href.trim_start_matches('/').to_string();
    }
    if base.is_empty() {
        return href.to_string();
    }
    // Resolve simple ".." parent-folder segments
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in href.split('/') {
        if seg == ".." {
            parts.pop();
        } else if seg != "." && !seg.is_empty() {
            parts.push(seg);
        }
    }
    parts.join("/")
}

fn strip_html(html: &str) -> String {
    // Drop script and style blocks so they are not spoken
    let mut s = html.to_string();
    for tag in ["script", "style"] {
        while let Some(start) = s.to_ascii_lowercase().find(&format!("<{tag}")) {
            let rest = &s[start..];
            if let Some(end_rel) = rest.to_ascii_lowercase().find(&format!("</{tag}>")) {
                let end = start + end_rel + tag.len() + 3;
                s.replace_range(start..end.min(s.len()), " ");
            } else {
                break;
            }
        }
    }
    // Block-level closing tags end a paragraph; <br> is a line break inside one.
    // (Must happen *before* tags are stripped.)
    let mut lower = s.to_ascii_lowercase();
    for (marker, with) in [
        ("</p>", "\n\n"),
        ("</div>", "\n\n"),
        ("</h1>", "\n\n"),
        ("</h2>", "\n\n"),
        ("</h3>", "\n\n"),
        ("</h4>", "\n\n"),
        ("</li>", "\n\n"),
        ("</tr>", "\n\n"),
        ("</blockquote>", "\n\n"),
        ("<br", "\n<br"),
    ] {
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for (i, _) in lower.match_indices(marker) {
            out.push_str(&s[last..i]);
            out.push_str(with);
            if with.ends_with("<br") {
                last = i + marker.len();
            } else {
                out.push_str(&s[i..i + marker.len()]);
                last = i + marker.len();
            }
        }
        out.push_str(&s[last..]);
        s = out;
        lower = s.to_ascii_lowercase();
    }
    // Source-code newlines inside a paragraph are just spaces.
    let text = strip_xml_tags(&s);
    let paras: Vec<String> = text
        .split("\n\n")
        .map(|p| {
            if chapters::is_mark(p) {
                p.trim().to_string()
            } else {
                p.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .filter(|p| !p.is_empty())
        .collect();
    decode_xml_entities(&paras.join("\n\n"))
}

/// Strip ODT inline tags; tabs, spaces (`<text:s/>`) and line breaks become a space.
fn odt_strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            return out;
        };
        let name = rest[lt + 1..lt + gt].split([' ', '/']).next().unwrap_or("");
        if matches!(name, "text:tab" | "text:s" | "text:line-break") {
            out.push(' ');
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    out
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Decode XML / common HTML entities in one pass (so `&amp;lt;` stays `&lt;`).
/// Unknown or malformed entities are kept as written.
fn decode_xml_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .find(';')
            .filter(|&semi| semi <= 10)
            .and_then(|semi| entity_char(&after[..semi]).map(|c| (c, semi)));
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character for entity `name` (`amp`, `#39`, `#x2014`, …).
fn entity_char(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).filter(|&c| c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode_once() {
        assert_eq!(
            decode_xml_entities("&amp;lt;b&amp;gt; &lt;i&gt;"),
            "&lt;b&gt; <i>"
        );
        assert_eq!(decode_xml_entities("&#39;a&#x2014;b&#8217;"), "'a—b’");
        assert_eq!(
            decode_xml_entities("x&mdash;y&hellip; &ldquo;q&rdquo;&nbsp;z"),
            "x—y… “q” z"
        );
        assert_eq!(
            decode_xml_entities("AT&T & co; &bogus; &#xZZ;"),
            "AT&T & co; &bogus; &#xZZ;"
        );
    }

    #[test]
    fn docx_handles_non_ascii_and_only_text_runs() {
        let xml = r#"<w:document><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:rPr><w:rFonts w:eastAsia="宋体"/></w:rPr><w:t>Café &amp;lt;one&amp;gt;</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t xml:space="preserve">a</w:t><w:tab/><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let t = docx_xml_to_text(xml);
        assert!(t.contains(&chapters::mark("Café &lt;one&gt;")), "{t:?}");
        assert!(
            t.contains(
                "Café &lt;one&gt;

"
            ),
            "{t:?}"
        );
        assert!(
            t.contains(
                "a	b

"
            ),
            "{t:?}"
        );
        assert!(!t.contains("w:"), "{t:?}");
    }

    #[test]
    fn odt_spacing_tags_become_spaces() {
        let xml = "<office:text><text:p>Tab<text:tab/>here<text:s text:c=\"2\"/>and<text:line-break/>on</text:p></office:text>";
        assert_eq!(odt_xml_to_text(xml).trim(), "Tab here and on");
    }

    #[test]
    fn odt_handles_non_ascii() {
        let xml = r#"<office:text><text:h text:outline-level="1">Señor &amp;amp; Ñ</text:h><text:p text:style-name="Ä">Grüße &amp;lt;x&amp;gt;</text:p></office:text>"#;
        let t = odt_xml_to_text(xml);
        assert!(t.contains(&chapters::mark("Señor &amp; Ñ")), "{t:?}");
        assert!(
            t.contains(
                "Grüße &lt;x&gt;

"
            ),
            "{t:?}"
        );
    }

    #[test]
    fn font_obfuscation_is_not_drm() {
        let fonts = r#"<encryption xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <enc:EncryptedData>
    <enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <enc:CipherData><enc:CipherReference URI="OEBPS/fonts/Serif.otf"/></enc:CipherData>
  </enc:EncryptedData>
  <enc:EncryptedData>
    <enc:EncryptionMethod Algorithm="http://ns.adobe.com/pdf/enc#RC"/>
    <enc:CipherData><enc:CipherReference URI="OEBPS/fonts/Sans.woff2"/></enc:CipherData>
  </enc:EncryptedData>
</encryption>"#;
        assert!(!encryption_xml_is_drm(fonts));
        let aes = fonts.replace(
            "http://ns.adobe.com/pdf/enc#RC",
            "http://www.w3.org/2001/04/xmlenc#aes128-cbc",
        );
        assert!(encryption_xml_is_drm(&aes));
        let chapter = fonts.replace("fonts/Serif.otf", "text/chapter1.xhtml");
        assert!(encryption_xml_is_drm(&chapter));
    }

    #[test]
    fn garbage_check_accepts_non_latin_text() {
        assert!(!looks_like_encrypted_garbage(
            "这是一本没有加密的中文书，内容可以正常阅读。"
        ));
        assert!(!looks_like_encrypted_garbage(
            "यह एक हिंदी किताब है जिसे पढ़ा जा सकता है"
        ));
        assert!(looks_like_encrypted_garbage(
            "\u{1}\u{2}#$%^&*\u{3}~|\u{7f}@!"
        ));
    }

    #[test]
    fn non_utf8_text_is_cp1252() {
        let dir = std::env::temp_dir().join(format!("d2a-cp1252-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.txt");
        std::fs::write(&f, b"\x93quoted\x94 \x96 caf\xe9").unwrap();
        assert_eq!(read_text_file(&f).unwrap(), "“quoted” – café");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

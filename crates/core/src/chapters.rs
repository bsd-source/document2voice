//! Chapter markers carried inside extracted text.
//!
//! Extractors put a marker paragraph — `U+E000 title U+E001` on its own line, with
//! blank lines around it — wherever a chapter starts (EPUB table of contents, PDF
//! bookmarks, DOCX/ODT headings, "CHAPTER I" lines in plain text). The marker rides
//! through page selection, the GUI's text file and the synth worker untouched, and is
//! turned into chapter boundaries right before synthesis. It is never spoken: the
//! chapter heading itself stays in the body text.

/// Start / end of a chapter marker (Unicode private-use characters).
pub const MARK_START: char = '\u{E000}';
pub const MARK_END: char = '\u{E001}';

/// A chapter with fewer words than this before the next marker is folded into the
/// previous chapter (tables of contents, "Part One" pages, empty headings).
const MIN_CHAPTER_WORDS: usize = 40;

/// A chapter of text, ready for synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterText {
    /// `None` for text before the first marker.
    pub title: Option<String>,
    pub text: String,
}

/// Marker paragraph for `title` (whitespace collapsed, length capped).
pub fn mark(title: &str) -> String {
    let mut t: String = title
        .chars()
        .filter(|&c| c != MARK_START && c != MARK_END)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if t.chars().count() > 120 {
        t = t.chars().take(117).collect::<String>() + "…";
    }
    if t.is_empty() {
        t = "Chapter".into();
    }
    format!("\n\n{MARK_START}{t}{MARK_END}\n\n")
}

/// If `line` (trimmed) is a marker paragraph, its title.
pub fn mark_title(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix(MARK_START)?
        .strip_suffix(MARK_END)
        .map(str::trim)
}

pub fn is_mark(line: &str) -> bool {
    mark_title(line).is_some()
}

/// Remove all markers (for display, word counts, previews).
pub fn strip_marks(text: &str) -> String {
    if !text.contains(MARK_START) {
        return text.to_string();
    }
    let kept: Vec<&str> = text.split("\n\n").filter(|p| !is_mark(p)).collect();
    kept.join("\n\n")
}

/// Titles of the chapters that [`split_chapters`] would produce.
pub fn chapter_titles(text: &str) -> Vec<String> {
    split_chapters(text)
        .into_iter()
        .filter_map(|c| c.title)
        .collect()
}

/// Split marked text into chapters, folding near-empty chapters into the previous one
/// so text order is always preserved.
pub fn split_chapters(text: &str) -> Vec<ChapterText> {
    let mut raw: Vec<ChapterText> = vec![ChapterText {
        title: None,
        text: String::new(),
    }];
    for para in text.split("\n\n") {
        if let Some(t) = mark_title(para) {
            raw.push(ChapterText {
                title: Some(t.to_string()),
                text: String::new(),
            });
            continue;
        }
        let p = para.trim_matches('\n');
        if p.trim().is_empty() {
            continue;
        }
        let cur = raw.last_mut().expect("non-empty");
        if !cur.text.is_empty() {
            cur.text.push_str("\n\n");
        }
        cur.text.push_str(p);
    }

    let mut out: Vec<ChapterText> = Vec::new();
    for ch in raw {
        let words = ch.text.split_whitespace().count();
        let tiny = ch.title.is_some() && words < MIN_CHAPTER_WORDS;
        match out.last_mut() {
            // Only fold into a previous chapter that has text of its own; otherwise a
            // short first chapter (e.g. a page range starting mid-chapter) keeps its title.
            Some(prev) if tiny && !prev.text.trim().is_empty() => {
                if !ch.text.is_empty() {
                    if !prev.text.is_empty() {
                        prev.text.push_str("\n\n");
                    }
                    prev.text.push_str(&ch.text);
                }
            }
            _ => out.push(ch),
        }
    }
    // A leading untitled chunk that is empty disappears; an empty first titled one too.
    out.retain(|c| !c.text.trim().is_empty());
    out
}

/// Insert markers before plain-text chapter headings ("CHAPTER I.", "Chapter 12",
/// "BOOK II", "Part One", "Letter 4"). Used for TXT and Kindle text, which have no
/// structured table of contents. A heading paragraph must be short.
///
/// Runs of consecutive headings: 3+ in a row is a table of contents (not marked);
/// 1–2 in a row ("BOOK I" + "CHAPTER 1") become one chapter with a combined title,
/// placed before the first heading of the run.
pub fn mark_plain_headings(text: &str) -> String {
    let paras: Vec<&str> = text.split("\n\n").collect();
    let heads: Vec<Option<String>> = paras.iter().map(|p| plain_heading(p)).collect();
    // "CHAPTER I." on its own line, then its name ("Down the Rabbit-Hole") as the next
    // paragraph: the name belongs in the chapter title.
    let subtitle = |j: usize| -> Option<&str> {
        let next = paras.get(j + 1)?.trim();
        (heads.get(j + 1).is_some_and(Option::is_none) && is_subtitle(next) && j + 2 < paras.len())
            .then_some(next)
    };
    let titled = |j: usize| -> String {
        let h = heads[j].clone().unwrap_or_default();
        match subtitle(j) {
            Some(sub) => format!("{h} {sub}"),
            None => h,
        }
    };
    let mut out = String::with_capacity(text.len() + 256);
    let mut i = 0;
    while i < paras.len() {
        if heads[i].is_some() {
            let run_end = (i..paras.len())
                .find(|&j| heads[j].is_none())
                .unwrap_or(paras.len());
            if run_end - i <= 2 {
                let title = (i..run_end).map(titled).collect::<Vec<_>>().join(" — ");
                out.push_str(&mark(&title));
            } else if i > 0 {
                out.push_str("\n\n");
            }
            // A contents list directly followed by the real first heading ends with the
            // same heading it started with ("Letter 1 … Chapter 24, Letter 1"): that last
            // one is the real chapter start.
            let last = run_end - 1;
            // Compare only "keyword + number" ("CHAPTER I"): the contents line may carry
            // the name and punctuation the real heading puts on its own line.
            let toc_then_first = run_end - i > 2
                && run_end < paras.len()
                && heading_key(heads[last].as_deref()) == heading_key(heads[i].as_deref());
            for (k, p) in paras[i..run_end].iter().enumerate() {
                if toc_then_first && i + k == last {
                    out.push_str(&mark(&titled(last)));
                } else if k > 0 {
                    out.push_str("\n\n");
                }
                out.push_str(p);
            }
            i = run_end;
            continue;
        }
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(paras[i]);
        i += 1;
    }
    out
}

/// "Chapter I." / "CHAPTER I. Down the Rabbit-Hole" → "chapter i".
fn heading_key(h: Option<&str>) -> String {
    h.unwrap_or("")
        .split_whitespace()
        .take(2)
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A chapter name line: short, starts with a capital, not a sentence.
fn is_subtitle(p: &str) -> bool {
    let words = p.split_whitespace().count();
    (1..=10).contains(&words)
        && p.chars().count() <= 70
        && p.chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c == '"' || c == '“')
        && !p.ends_with(['.', ',', ';', ':'])
        && !p.contains(MARK_START)
        && plain_heading(p).is_none()
}

fn plain_heading(para: &str) -> Option<String> {
    let p = para.trim();
    if p.is_empty() || p.chars().count() > 90 || p.contains(MARK_START) {
        return None;
    }
    let mut words = p.split_whitespace();
    let first = words.next()?;
    let kw = first.trim_end_matches(['.', ':']).to_ascii_lowercase();
    if !matches!(
        kw.as_str(),
        "chapter" | "book" | "part" | "letter" | "prologue" | "epilogue"
    ) {
        return None;
    }
    if matches!(kw.as_str(), "prologue" | "epilogue") {
        return (p.split_whitespace().count() <= 8).then(|| p.to_string());
    }
    // Keyword must be capitalised as a heading, not "part of the…" mid-sentence text.
    if !first.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    let num = words.next()?.trim_end_matches(['.', ':', ',']);
    let is_num = num.chars().all(|c| c.is_ascii_digit())
        || (num.chars().all(|c| "IVXLCDM".contains(c)) && !num.is_empty())
        || NUMBER_WORDS.contains(&num.to_ascii_lowercase().as_str());
    if !is_num {
        return None;
    }
    // "CHAPTER I. Down the Rabbit-Hole" is fine; a long sentence is not a heading.
    let rest = p.split_whitespace().count();
    (rest <= 12 && !p.ends_with([',', ';'])).then(|| p.to_string())
}

const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "last",
    "final",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn body(n: usize) -> String {
        vec!["word"; n].join(" ")
    }

    #[test]
    fn split_keeps_order_and_folds_tiny_chapters() {
        let text = format!(
            "Intro {}{}{}{}tiny{}{}",
            body(50),
            mark("Contents"),
            "CHAPTER I.",
            mark("Chapter One"),
            mark("Chapter Two"),
            body(60)
        );
        let ch = split_chapters(&text);
        let titles: Vec<_> = ch.iter().map(|c| c.title.clone()).collect();
        assert_eq!(titles, [None, Some("Chapter Two".to_string())]);
        // Folded text stays in order inside the untitled lead.
        assert!(
            ch[0].text.ends_with("CHAPTER I.\n\ntiny"),
            "{:?}",
            ch[0].text
        );
    }

    #[test]
    fn plain_headings_detected() {
        let t = format!(
            "Contents\n\nCHAPTER I. Down the Rabbit-Hole\n\n{}\n\nChapter 2\n\n{}\n\nPart of the day was gone.",
            body(50),
            body(50)
        );
        let marked = mark_plain_headings(&t);
        assert_eq!(
            chapter_titles(&marked),
            ["CHAPTER I. Down the Rabbit-Hole", "Chapter 2"]
        );
        assert_eq!(strip_marks(&marked), t);
    }

    #[test]
    fn contents_lists_are_not_chapters() {
        let t = format!(
            "CONTENTS\n\nLetter 1\n\nLetter 2\n\nChapter 1\n\nfront {}\n\nBOOK I\n\nChapter 1\n\n{}",
            body(50),
            body(50)
        );
        assert_eq!(
            chapter_titles(&mark_plain_headings(&t)),
            ["BOOK I — Chapter 1"]
        );
        let t2 = format!(
            "Letter 1\n\nLetter 2\n\nChapter 1\n\nLetter 1\n\n{}",
            body(50)
        );
        assert_eq!(chapter_titles(&mark_plain_headings(&t2)), ["Letter 1"]);
    }

    #[test]
    fn mark_round_trips() {
        assert_eq!(mark_title(mark("  A \n Title ").trim()), Some("A Title"));
    }

    #[test]
    fn heading_with_name_on_next_line_and_contents_list() {
        // Contents (as table rows) then "CHAPTER I." with its name on its own line.
        let t = format!(
            "Contents\n\nCHAPTER I. Down the Rabbit-Hole.\n\nCHAPTER II. The Pool of Tears.\n\nCHAPTER III. A Caucus-Race.\n\n\
             CHAPTER I.\n\nDown the Rabbit-Hole\n\n{}\n\nCHAPTER II.\n\nThe Pool of Tears\n\n{}",
            body(50),
            body(50)
        );
        assert_eq!(
            chapter_titles(&mark_plain_headings(&t)),
            [
                "CHAPTER I. Down the Rabbit-Hole",
                "CHAPTER II. The Pool of Tears"
            ]
        );
    }
}

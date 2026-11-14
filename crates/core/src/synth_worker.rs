//! One “speak this text and save the audio” job, used by the GUI (as a child process)
//! and by the command-line tool.
//!
//! Progress is reported through a callback so the parent can show updates
//! (for example, JSON status lines on standard output).

use crate::chapters;
use crate::config::{ExportFormat, ExportOptions, SpeakOptions};
use crate::error::{CoreError, Result};
use crate::export::{export_stage_with_chapters, ChapterMark, StageWav};
use crate::tts::{
    default_synth_workers, prepare_speech_paragraphs, synthesize_parallel_streaming, TtsEngine,
};
use std::path::{Path, PathBuf};

/// Inputs for one speech-generation + file-export job.
#[derive(Debug, Clone)]
pub struct SynthWorkerRequest {
    pub text: String,
    pub out: PathBuf,
    pub voice: String,
    pub speed: f32,
    pub pitch: f32,
    pub style_index: Option<usize>,
    pub paragraph_pause_secs: f32,
    pub loudness: f32,
    pub format: ExportFormat,
    pub loudnorm: bool,
    pub bitrate: String,
    pub sample_rate: u32,
    /// Stop after about this many seconds of audio (for short previews). `None` = full book.
    pub max_secs: Option<f32>,
    /// One file per chapter (in a folder named after `out`) instead of one file.
    pub split_chapters: bool,
}

/// What a finished job produced.
#[derive(Debug, Clone)]
pub struct JobReport {
    /// The file written (or the folder, for one-file-per-chapter exports).
    pub path: PathBuf,
    /// Paragraphs that could not be synthesized and were replaced by silence.
    pub skipped: usize,
    /// Text file listing the skipped paragraphs (only when `skipped > 0`).
    pub skipped_report: Option<PathBuf>,
}

impl JobReport {
    /// One-line warning for logs/UI, if anything was skipped.
    pub fn warning(&self) -> Option<String> {
        let report = self.skipped_report.as_ref()?;
        Some(format!(
            "{} paragraph{} could not be spoken and {} replaced by silence — see {}",
            self.skipped,
            if self.skipped == 1 { "" } else { "s" },
            if self.skipped == 1 { "was" } else { "were" },
            report.display()
        ))
    }
}

/// Title used for text before the first detected chapter.
const LEAD_TITLE: &str = "Opening";

fn para_fraction(done: usize, total: usize) -> f32 {
    if total == 0 {
        1.0
    } else {
        done as f32 / total as f32
    }
}

/// Generate speech and write the finished file, calling `on_progress(fraction, label)` as it goes.
///
/// Returns a [`JobReport`] with the written path and any skipped paragraphs.
pub fn run_synth_worker_job(
    req: &SynthWorkerRequest,
    mut on_progress: impl FnMut(f32, &str),
) -> Result<JobReport> {
    let body = req.text.trim();
    if body.is_empty() {
        return Err(CoreError::Tts("empty text".into()));
    }

    let speak = SpeakOptions {
        voice: req.voice.clone(),
        speed: req.speed,
        style_index: req.style_index,
        paragraph_pause_secs: req.paragraph_pause_secs,
    };

    let (base, span) = if req.max_secs.is_some() {
        (0.12, 0.58)
    } else {
        (0.10, 0.78)
    };
    let mut progress =
        |done, total, label: &str| on_progress(base + para_fraction(done, total) * span, label);

    // Audio streams straight to a stage WAV beside the output file, so a long book
    // never sits in memory.
    let mut chapter_titles: Vec<String> = Vec::new();
    let mut chapter_paras: Vec<usize> = Vec::new(); // first paragraph of each chapter
    let mut paragraphs: Vec<String> = Vec::new();
    let mut skip_reasons: std::collections::HashMap<usize, String> = Default::default();
    let stage = if req.max_secs.is_some() {
        // Previews are short: one engine starts fastest; nothing to resume.
        let mut stage = StageWav::temporary(&req.out)?;
        progress(0, 1, "Loading Kokoro model…");
        let mut engine = TtsEngine::new()?;
        engine.synthesize_streaming(
            &chapters::strip_marks(body),
            &speak,
            req.max_secs,
            &mut progress,
            &mut |chunk| stage.push(chunk),
        )?;
        stage
    } else {
        // Full export: resumable. If an earlier run of this exact job was killed or
        // failed, continue after its last finished paragraph.
        // Chapters (from markers in the extracted text) become paragraph ranges.
        let parts = chapters::split_chapters(body);
        let titled = parts.iter().any(|c| c.title.is_some());
        for ch in &parts {
            let p = match prepare_speech_paragraphs(&ch.text) {
                Ok(p) => p,
                Err(_) => continue, // nothing speakable in this chapter
            };
            if titled {
                chapter_paras.push(paragraphs.len());
                chapter_titles.push(ch.title.clone().unwrap_or_else(|| LEAD_TITLE.into()));
            }
            paragraphs.extend(p);
        }
        if paragraphs.is_empty() {
            return Err(CoreError::Tts("empty text".into()));
        }
        let key = job_key(&paragraphs, &speak, &chapter_paras, &chapter_titles);
        let (stage, start) = StageWav::resumable(&req.out, &key, paragraphs.len())?;
        let stage = std::cell::RefCell::new(stage);
        synthesize_parallel_streaming(
            &paragraphs,
            start,
            &speak,
            default_synth_workers(),
            &mut progress,
            &mut |chunk| stage.borrow_mut().push(chunk),
            &mut |done| {
                let mut st = stage.borrow_mut();
                // `done` paragraphs are on disk, so the stage end is where paragraph
                // `done` (maybe a chapter start) will begin.
                if chapter_paras.get(1..).is_some_and(|c| c.contains(&done)) {
                    st.mark_chapter_start();
                }
                st.checkpoint(done)
            },
            &mut |i, err| {
                skip_reasons.insert(i, err.to_string());
                stage.borrow_mut().note_skipped(i);
                Ok(())
            },
        )?;
        stage.into_inner()
    };

    if stage.is_empty() {
        return Err(CoreError::Tts("synthesis produced no audio".into()));
    }

    on_progress(0.90, "Encoding audio…");
    let export = ExportOptions {
        format: req.format,
        opus_bitrate: req.bitrate.clone(),
        loudnorm: req.loudnorm,
        loudnorm_i: req.loudness,
        pitch_semitones: req.pitch,
        sample_rate: req.sample_rate,
        split_chapters: req.split_chapters,
    };
    let marks: Vec<ChapterMark> = if chapter_titles.len() >= 2 {
        let starts = std::iter::once(0).chain(stage.chapter_starts().iter().copied());
        chapter_titles
            .iter()
            .zip(starts)
            .map(|(t, start)| ChapterMark {
                title: t.clone(),
                start,
            })
            .collect()
    } else {
        Vec::new()
    };
    if !marks.is_empty() && marks.len() != chapter_titles.len() {
        tracing::warn!(
            expected = chapter_titles.len(),
            got = marks.len(),
            "chapter positions incomplete; exporting without chapters"
        );
    }
    let marks = if marks.len() == chapter_titles.len() {
        marks
    } else {
        Vec::new()
    };
    let mut skipped: Vec<usize> = stage.skipped().to_vec();
    skipped.sort_unstable();
    let path = export_stage_with_chapters(stage, &req.out, &export, &marks)?;

    let skipped_report = if skipped.is_empty() {
        // A report left by an earlier export of the same file no longer applies.
        if req.max_secs.is_none() {
            remove_stale_report(&skipped_report_path(&req.out))?;
        }
        None
    } else {
        let report = skipped_report_path(&req.out);
        let body = skipped_report_text(
            &skipped,
            &paragraphs,
            &chapter_paras,
            &chapter_titles,
            &skip_reasons,
        );
        std::fs::write(&report, body)?;
        tracing::warn!(count = skipped.len(), report = %report.display(), "paragraphs skipped");
        Some(report)
    };
    let job = JobReport {
        path,
        skipped: skipped.len(),
        skipped_report,
    };
    match job.warning() {
        Some(w) => on_progress(1.0, &format!("Done with warnings: {w}")),
        None => on_progress(1.0, "Encoding complete"),
    }
    Ok(job)
}

/// `book.m4b` → `book.skipped.txt` beside it.
fn skipped_report_path(out: &Path) -> PathBuf {
    let stem = out
        .file_stem()
        .map_or_else(|| "audio".into(), |s| s.to_string_lossy().into_owned());
    out.with_file_name(format!("{stem}.skipped.txt"))
}

/// Delete `report` if it exists.
fn remove_stale_report(report: &Path) -> Result<()> {
    match std::fs::remove_file(report) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn skipped_report_text(
    skipped: &[usize],
    paragraphs: &[String],
    chapter_paras: &[usize],
    chapter_titles: &[String],
    reasons: &std::collections::HashMap<usize, String>,
) -> String {
    use std::fmt::Write as _;
    let mut t = format!(
        "Document2Voice: {} of {} paragraphs could not be spoken.\n\
         Each was replaced by {} s of silence. Fix or remove the text and export again.\n\n",
        skipped.len(),
        paragraphs.len(),
        crate::tts::SKIP_SILENCE_SECS
    );
    for &i in skipped {
        let chapter = chapter_paras
            .iter()
            .rposition(|&c| c <= i)
            .and_then(|k| chapter_titles.get(k));
        let _ = write!(t, "Paragraph {} of {}", i + 1, paragraphs.len());
        if let Some(c) = chapter {
            let _ = write!(t, " (chapter: {c})");
        }
        let reason = reasons
            .get(&i)
            .map_or("skipped in an earlier, interrupted run", String::as_str);
        let text = paragraphs.get(i).map_or("", String::as_str);
        let short: String = text.chars().take(400).collect();
        let more = if text.chars().count() > 400 {
            " …"
        } else {
            ""
        };
        let _ = write!(t, "\n  Error: {reason}\n  Text:  {short}{more}\n\n");
    }
    t
}

/// Read UTF-8 text from a file for the speech job.
pub fn load_worker_text(path: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

/// Stable identity of a full export: everything that changes the synthesized audio
/// (sanitized paragraphs, voice, speed, style, pause, phoneme backend, app version).
/// Export-only settings (format, pitch, loudness) are applied after synthesis by
/// ffmpeg, so changing them still resumes.
fn job_key(
    paragraphs: &[String],
    speak: &SpeakOptions,
    chapter_paras: &[usize],
    chapter_titles: &[String],
) -> String {
    // FNV-1a 64: stable across Rust versions and runs (unlike DefaultHasher).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff; // field separator
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    eat(crate::VERSION.as_bytes());
    eat(b"espeak-heteronyms-1"); // phoneme pipeline version
    eat(speak.voice.as_bytes());
    eat(&speak.speed.clamp(0.5, 2.0).to_bits().to_le_bytes());
    eat(&speak
        .style_index
        .map_or(u64::MAX, |s| s as u64)
        .to_le_bytes());
    eat(&speak.paragraph_pause_secs.to_bits().to_le_bytes());
    for p in paragraphs {
        eat(p.as_bytes());
    }
    for (i, t) in chapter_paras.iter().zip(chapter_titles) {
        eat(&(*i as u64).to_le_bytes());
        eat(t.as_bytes());
    }
    format!("{h:016x}-{}", paragraphs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speak() -> SpeakOptions {
        SpeakOptions::default()
    }

    #[test]
    fn skipped_report_names_chapter_and_reason() {
        let paras: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let mut why = std::collections::HashMap::new();
        why.insert(2, "espeak failed".to_string());
        let t = skipped_report_text(
            &[0, 2],
            &paras,
            &[0, 2],
            &["Opening".into(), "Chapter 1".into()],
            &why,
        );
        assert!(t.contains("2 of 4 paragraphs"), "{t}");
        assert!(
            t.contains("Paragraph 3 of 4 (chapter: Chapter 1)\n  Error: espeak failed\n  Text:  c"),
            "{t}"
        );
        assert!(
            t.contains("Paragraph 1 of 4 (chapter: Opening)\n  Error: skipped in an earlier"),
            "{t}"
        );
        assert_eq!(
            skipped_report_path(Path::new("/x/book.m4b")),
            Path::new("/x/book.skipped.txt")
        );
    }

    #[test]
    fn stale_skipped_report_is_removed() {
        let dir = std::env::temp_dir().join(format!("d2a-stale-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = skipped_report_path(&dir.join("book.opus"));
        std::fs::write(&report, "old").unwrap();
        remove_stale_report(&report).unwrap();
        assert!(!report.exists());
        remove_stale_report(&report).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn job_key_changes_with_text_and_voice_only() {
        let p = vec!["One.".to_string(), "Two.".to_string()];
        let k = job_key(&p, &speak(), &[], &[]);
        assert_eq!(k, job_key(&p, &speak(), &[], &[]));
        assert_ne!(
            k,
            job_key(&["One.".into(), "Too.".into()], &speak(), &[], &[])
        );
        let mut other = speak();
        other.voice = "bf_emma".into();
        assert_ne!(k, job_key(&p, &other, &[], &[]));
        assert_ne!(k, job_key(&p, &speak(), &[0, 1], &["A".into(), "B".into()]));
    }
}

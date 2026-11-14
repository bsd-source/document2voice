//! document2voice — command-line tool that turns text and documents into audiobooks offline
//! (uses the Kokoro voice model; supports PDF, EPUB, MOBI/AZW, TXT, DOCX, ODT).

use anyhow::{bail, Context, Result};
use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use document2voice_core::{
    chapters, extract_document, load_worker_text, parse_page_range, run_synth_worker_job,
    CoreError, ExportFormat, ExportOptions, ExtractOptions, PdfPages, SynthWorkerRequest, VERSION,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "document2voice",
    version = VERSION,
    about = "Document2Voice (Kokoro) — turn documents into audiobooks offline"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show the available speaking voices
    Voices {
        /// Include every language, not just English
        #[arg(long)]
        all: bool,
    },

    /// Pull the text out of a document (PDF, EPUB, MOBI/AZW, TXT, DOCX, ODT) without making audio
    Extract {
        /// Path to the document file
        input: PathBuf,
        /// Which pages to use, for example 4-20 (exact for PDFs; a rough guide for other file types)
        #[arg(long)]
        pages: Option<String>,
        /// Skip the first N pages (exact for PDFs; a rough guide for other file types)
        #[arg(long)]
        skip_before: Option<u32>,
        /// Save the extracted text to this file (if omitted, print it on the screen).
        /// The file keeps chapter markers, so `speak --file` makes the same chapters.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Only list the chapters that were found
        #[arg(long, conflicts_with = "out")]
        chapters: bool,
        /// Print the text exactly as the speech engine receives it (after the
        /// speech sanitizer: symbols, money, dates, abbreviations …), one paragraph
        /// per block.
        #[arg(long, conflicts_with_all = ["chapters", "out"])]
        spoken: bool,
    },

    /// Turn text or a document into an audio file
    #[command(group(
        ArgGroup::new("source")
            .args(["text", "file", "input"])
            .required(true)
            .multiple(false)
    ))]
    Speak {
        /// Text to speak (do not combine with --file or --input)
        text: Option<String>,

        /// Path to a plain text file to speak
        #[arg(long)]
        file: Option<PathBuf>,

        /// Path to a document (.pdf, .epub, .mobi, .azw, .azw3, .txt, .docx, or .odt)
        #[arg(long)]
        input: Option<PathBuf>,

        /// Which pages to use, for example 4-20 (exact for PDFs; a rough guide for other file types)
        #[arg(long, requires = "input", conflicts_with_all = ["text", "file"])]
        pages: Option<String>,

        /// Skip the first N pages (exact for PDFs; a rough guide for other file types)
        #[arg(long, requires = "input", conflicts_with_all = ["text", "file"])]
        skip_before: Option<u32>,

        /// Output audio file; its extension picks the format unless --format is given
        #[arg(short, long)]
        out: PathBuf,

        /// Voice id (see `document2voice voices`)
        #[arg(long, default_value = "am_adam")]
        voice: String,

        /// Speaking speed, 0.5 to 2.0
        #[arg(long, default_value_t = 1.0, value_parser = parse_speed)]
        speed: f32,

        /// Pitch shift in semitones, -12 to 12
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true, value_parser = parse_pitch)]
        pitch: f32,

        /// Voice style number (about 0-500); leave out to let the engine pick
        #[arg(long)]
        style: Option<usize>,

        /// Silence between paragraphs in seconds, 0 to 10
        #[arg(long, default_value_t = 0.45, value_parser = parse_pause)]
        paragraph_pause: f32,

        /// Loudness target in LUFS, -30 to -5
        #[arg(long, default_value_t = -18.0, allow_negative_numbers = true, value_parser = parse_loudness)]
        loudness: f32,

        /// Audio format (default: from the --out extension, else opus)
        #[arg(long, value_enum)]
        format: Option<Fmt>,

        /// Turn off loudness leveling
        #[arg(long)]
        no_loudnorm: bool,

        /// One file per chapter in a folder named after --out (instead of one file
        /// with chapter markers). Needs a document with detectable chapters.
        #[arg(long)]
        split_chapters: bool,

        /// Bitrate for lossy formats, for example 64k
        #[arg(long, default_value = "64k")]
        bitrate: String,
    },

    /// Background helper used by the graphical app to create audio (you can stop it mid-run).
    ///
    /// Sends progress updates as one JSON line at a time on standard output.
    /// Errors go to standard error and the process exits with a failure code.
    /// When you press Cancel in the app, this process is stopped so work can halt right away.
    #[command(hide = true)]
    SynthWorker {
        /// Path to a UTF-8 text file to speak
        #[arg(long)]
        text_file: PathBuf,

        /// Where to write the audio file
        #[arg(long, short)]
        out: PathBuf,

        #[arg(long, default_value = "am_adam")]
        voice: String,

        #[arg(long, default_value_t = 1.0, value_parser = parse_speed)]
        speed: f32,

        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true, value_parser = parse_pitch)]
        pitch: f32,

        #[arg(long)]
        style: Option<usize>,

        #[arg(long, default_value_t = 0.45, value_parser = parse_pause)]
        paragraph_pause: f32,

        #[arg(long, default_value_t = -18.0, allow_negative_numbers = true, value_parser = parse_loudness)]
        loudness: f32,

        #[arg(long, value_enum, default_value_t = Fmt::Opus)]
        format: Fmt,

        #[arg(long)]
        no_loudnorm: bool,

        /// One file per chapter in a folder named after --out (instead of one file
        /// with chapter markers). Needs a document with detectable chapters.
        #[arg(long)]
        split_chapters: bool,

        #[arg(long, default_value = "64k")]
        bitrate: String,

        #[arg(long, default_value_t = 24_000)]
        sample_rate: u32,

        /// Limit how many seconds of audio to generate (for short previews). Leave out for the full file.
        #[arg(long)]
        max_secs: Option<f32>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Fmt {
    Opus,
    Mp3,
    Wav,
    Flac,
    /// AAC audiobook with chapters
    M4b,
}

/// Parse a finite `f32` within `lo..=hi` (shared by the range-checked flags).
fn parse_ranged(s: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let v: f32 = s.trim().parse().map_err(|e| format!("not a number: {e}"))?;
    if !v.is_finite() || !(lo..=hi).contains(&v) {
        return Err(format!("must be between {lo} and {hi}"));
    }
    Ok(v)
}

fn parse_speed(s: &str) -> Result<f32, String> {
    parse_ranged(s, 0.5, 2.0)
}

fn parse_pitch(s: &str) -> Result<f32, String> {
    parse_ranged(s, -12.0, 12.0)
}

fn parse_pause(s: &str) -> Result<f32, String> {
    parse_ranged(s, 0.0, 10.0)
}

fn parse_loudness(s: &str) -> Result<f32, String> {
    parse_ranged(s, -30.0, -5.0)
}

impl From<Fmt> for ExportFormat {
    fn from(f: Fmt) -> Self {
        match f {
            Fmt::Opus => ExportFormat::Opus,
            Fmt::Mp3 => ExportFormat::Mp3,
            Fmt::Wav => ExportFormat::Wav,
            Fmt::Flac => ExportFormat::Flac,
            Fmt::M4b => ExportFormat::M4b,
        }
    }
}

fn pdf_pages_from_cli(pages: Option<String>, skip_before: Option<u32>) -> Result<PdfPages> {
    match (pages, skip_before) {
        (Some(_), Some(_)) => bail!("use either --pages or --skip-before, not both"),
        (Some(spec), None) => Ok(PdfPages::Range(parse_page_range(&spec)?)),
        (None, Some(n)) => Ok(PdfPages::SkipBefore(n)),
        (None, None) => Ok(PdfPages::All),
    }
}

fn extract_opts(pages: Option<String>, skip_before: Option<u32>) -> Result<ExtractOptions> {
    Ok(ExtractOptions {
        pdf_pages: pdf_pages_from_cli(pages, skip_before)?,
        skip_pages: Vec::new(),
    })
}

/// Print a DRM / copy-protection error and exit with code 2.
fn exit_drm(e: &CoreError) -> ! {
    eprintln!("{e}");
    if let CoreError::DrmProtected { detail } = e {
        eprintln!("({detail})");
    }
    std::process::exit(2);
}

fn extract_or_exit_drm(
    path: &Path,
    opts: &ExtractOptions,
) -> Result<document2voice_core::DocExtract> {
    match extract_document(path, opts) {
        Ok(e) => Ok(e),
        Err(e) if e.is_drm() => exit_drm(&e),
        Err(e) => Err(e).context("extract"),
    }
}

fn main() -> Result<()> {
    // Send log messages to standard error so standard output stays free for worker progress JSON.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match Cli::parse().cmd {
        Commands::Voices { all } => cmd_voices(all),
        Commands::Extract {
            input,
            pages,
            skip_before,
            out,
            chapters,
            spoken,
        } => cmd_extract(input, pages, skip_before, out, chapters, spoken),
        Commands::Speak {
            text,
            file,
            input,
            pages,
            skip_before,
            out,
            voice,
            speed,
            pitch,
            style,
            paragraph_pause,
            loudness,
            format,
            no_loudnorm,
            split_chapters,
            bitrate,
        } => cmd_speak(SpeakCli {
            text,
            file,
            input,
            pages,
            skip_before,
            out,
            voice,
            speed,
            pitch,
            style,
            paragraph_pause,
            loudness,
            format,
            no_loudnorm,
            split_chapters,
            bitrate,
        }),
        Commands::SynthWorker {
            text_file,
            out,
            voice,
            speed,
            pitch,
            style,
            paragraph_pause,
            loudness,
            format,
            no_loudnorm,
            split_chapters,
            bitrate,
            sample_rate,
            max_secs,
        } => cli_synth_worker(SynthWorkerCli {
            text_file,
            out,
            voice,
            speed,
            pitch,
            style,
            paragraph_pause,
            loudness,
            format: format.into(),
            loudnorm: !no_loudnorm,
            bitrate,
            sample_rate,
            max_secs,
            split_chapters,
        }),
    }
}

fn cmd_voices(all: bool) -> Result<()> {
    use document2voice_core::{voices_for_language, Language};
    let langs: &[Language] = if all {
        Language::ALL.as_slice()
    } else {
        &[Language::EnglishBritish, Language::EnglishAmerican]
    };
    for lang in langs {
        println!("{}", lang.label());
        for v in voices_for_language(*lang) {
            println!("  {:16}  {:6}  {}", v.id, v.gender, v.notes);
        }
        println!();
    }
    Ok(())
}

fn cmd_extract(
    input: PathBuf,
    pages: Option<String>,
    skip_before: Option<u32>,
    out: Option<PathBuf>,
    list_chapters: bool,
    spoken: bool,
) -> Result<()> {
    let opts = extract_opts(pages, skip_before)?;
    let extracted = extract_or_exit_drm(&input, &opts)?;
    info!(
        kind = extracted.kind.label(),
        words = extracted.word_count,
        chapters = extracted.chapter_count,
        detail = %extracted.detail,
        "extracted"
    );
    if spoken {
        let paras =
            document2voice_core::prepare_speech_paragraphs(&chapters::strip_marks(&extracted.text))
                .context("sanitize")?;
        println!("{}", paras.join("\n\n"));
        return Ok(());
    }
    if list_chapters {
        let titles = chapters::chapter_titles(&extracted.text);
        if titles.is_empty() {
            println!("No chapters found (the audio will be one section).");
        }
        for (i, t) in titles.iter().enumerate() {
            println!("{:>3}. {t}", i + 1);
        }
        return Ok(());
    }
    if let Some(path) = out {
        std::fs::write(&path, &extracted.text)
            .with_context(|| format!("write {}", path.display()))?;
        println!(
            "Wrote {} words ({}) → {}",
            extracted.word_count,
            extracted.detail,
            path.display()
        );
    } else {
        // Readable chapter headings on screen instead of the raw marker characters.
        for (i, para) in extracted.text.split("\n\n").enumerate() {
            if i > 0 {
                print!("\n\n");
            }
            match chapters::mark_title(para) {
                Some(t) => print!("=== {t} ==="),
                None => print!("{para}"),
            }
        }
        println!();
    }
    Ok(())
}

struct SpeakCli {
    text: Option<String>,
    file: Option<PathBuf>,
    input: Option<PathBuf>,
    pages: Option<String>,
    skip_before: Option<u32>,
    out: PathBuf,
    voice: String,
    speed: f32,
    pitch: f32,
    style: Option<usize>,
    paragraph_pause: f32,
    loudness: f32,
    format: Option<Fmt>,
    no_loudnorm: bool,
    split_chapters: bool,
    bitrate: String,
}

fn cmd_speak(args: SpeakCli) -> Result<()> {
    let (out, format) = resolve_output(args.out, args.format.map(ExportFormat::from))?;
    let body = resolve_speak_body(
        args.text,
        args.file,
        args.input,
        args.pages,
        args.skip_before,
    )?;

    info!(voice = %args.voice, speed = args.speed, "loading Kokoro ONNX");
    // Same streaming path as the GUI: paragraphs go straight to disk.
    let req = SynthWorkerRequest {
        text: body,
        out,
        voice: args.voice,
        speed: args.speed,
        pitch: args.pitch,
        style_index: args.style,
        paragraph_pause_secs: args.paragraph_pause,
        loudness: args.loudness,
        format,
        loudnorm: !args.no_loudnorm,
        bitrate: args.bitrate,
        sample_rate: ExportOptions::default().sample_rate,
        max_secs: None,
        split_chapters: args.split_chapters,
    };
    let mut last_label = String::new();
    let report = run_synth_worker_job(&req, |_, label| {
        if label != last_label {
            info!("{label}");
            last_label = label.to_string();
        }
    })
    .context("speak")?;
    println!("Wrote {}", report.path.display());
    if let Some(w) = report.warning() {
        eprintln!("Warning: {w}");
    }
    Ok(())
}

/// Pick the output path and format from `--out` and `--format`.
///
/// - No extension: use `--format` (default opus) and add its extension.
/// - Known audio extension: that format; `--format`, if given, must agree.
/// - Other extension: with `--format`, its extension is appended; without, an error.
fn resolve_output(out: PathBuf, format: Option<ExportFormat>) -> Result<(PathBuf, ExportFormat)> {
    let Some(ext) = out.extension() else {
        let f = format.unwrap_or_default();
        return Ok((out.with_extension(f.extension()), f));
    };
    let ext = ext.to_string_lossy().into_owned();
    match (ExportFormat::from_name(&ext), format) {
        (Some(from_ext), Some(f)) if from_ext != f => bail!(
            "--out ends in .{ext} but --format is {}; make them match or drop one",
            f.extension()
        ),
        (Some(from_ext), _) => Ok((out, from_ext)),
        (None, Some(f)) => {
            let mut name = out.into_os_string();
            name.push(".");
            name.push(f.extension());
            Ok((PathBuf::from(name), f))
        }
        (None, None) => bail!(
            "--out ends in .{ext}, which is not an audio format; use .opus, .mp3, .m4b, .flac or .wav, or pass --format"
        ),
    }
}

/// Load the text to speak from exactly one source (clap's `source` group enforces that).
fn resolve_speak_body(
    text: Option<String>,
    file: Option<PathBuf>,
    doc_path: Option<PathBuf>,
    pages: Option<String>,
    skip_before: Option<u32>,
) -> Result<String> {
    if let Some(t) = text {
        Ok(t)
    } else if let Some(p) = file {
        std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))
    } else if let Some(path) = doc_path {
        // Pages are exact for PDF and approximate (~250 words) for other formats,
        // same as `extract` and the GUI.
        let opts = extract_opts(pages, skip_before)?;
        let extracted = extract_or_exit_drm(&path, &opts)?;
        info!(
            kind = extracted.kind.label(),
            words = extracted.word_count,
            "document text ready"
        );
        Ok(extracted.text)
    } else {
        bail!("provide one of: TEXT, --file, or --input")
    }
}

struct SynthWorkerCli {
    text_file: PathBuf,
    out: PathBuf,
    voice: String,
    speed: f32,
    pitch: f32,
    style: Option<usize>,
    paragraph_pause: f32,
    loudness: f32,
    format: ExportFormat,
    loudnorm: bool,
    bitrate: String,
    sample_rate: u32,
    max_secs: Option<f32>,
    split_chapters: bool,
}

fn worker_emit(obj: &serde_json::Value) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{obj}");
    let _ = out.flush();
}

fn worker_progress(fraction: f32, label: &str) {
    worker_emit(&serde_json::json!({
        "type": "progress",
        "fraction": fraction.clamp(0.0, 1.0),
        "label": label,
    }));
}

fn cli_synth_worker(args: SynthWorkerCli) -> Result<()> {
    let text = load_worker_text(&args.text_file)
        .with_context(|| format!("read text file {}", args.text_file.display()))?;
    let req = SynthWorkerRequest {
        text,
        out: args.out,
        voice: args.voice,
        speed: args.speed,
        pitch: args.pitch,
        style_index: args.style,
        paragraph_pause_secs: args.paragraph_pause,
        loudness: args.loudness,
        format: args.format,
        loudnorm: args.loudnorm,
        bitrate: args.bitrate,
        sample_rate: args.sample_rate,
        max_secs: args.max_secs,
        split_chapters: args.split_chapters,
    };
    let report = run_synth_worker_job(&req, |frac, label| {
        worker_progress(frac, label);
    })
    .context("synth worker")?;
    worker_emit(&serde_json::json!({
        "type": "done",
        "path": report.path.display().to_string(),
        "warning": report.warning(),
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("document2voice").chain(args.iter().copied()))
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn output_without_extension_uses_format() {
        let (p, f) = resolve_output("book".into(), None).unwrap();
        assert_eq!((p, f), (PathBuf::from("book.opus"), ExportFormat::Opus));
        let (p, f) = resolve_output("book".into(), Some(ExportFormat::Mp3)).unwrap();
        assert_eq!((p, f), (PathBuf::from("book.mp3"), ExportFormat::Mp3));
    }

    #[test]
    fn output_extension_sets_format() {
        let (p, f) = resolve_output("book.m4b".into(), None).unwrap();
        assert_eq!((p, f), (PathBuf::from("book.m4b"), ExportFormat::M4b));
        assert!(resolve_output("book.mp3".into(), Some(ExportFormat::Mp3)).is_ok());
    }

    #[test]
    fn output_extension_and_format_must_agree() {
        assert!(resolve_output("book.mp3".into(), Some(ExportFormat::Flac)).is_err());
    }

    #[test]
    fn unknown_output_extension() {
        assert!(resolve_output("book.txt".into(), None).is_err());
        let (p, f) = resolve_output("vol.1".into(), Some(ExportFormat::Wav)).unwrap();
        assert_eq!((p, f), (PathBuf::from("vol.1.wav"), ExportFormat::Wav));
    }

    #[test]
    fn speak_needs_exactly_one_source() {
        assert!(parse(&["speak", "-o", "a.opus"]).is_err());
        assert!(parse(&["speak", "hi", "--file", "x.txt", "-o", "a.opus"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus"]).is_ok());
        assert!(parse(&["speak", "--input", "b.pdf", "-o", "a.opus"]).is_ok());
    }

    #[test]
    fn page_flags_need_input() {
        assert!(parse(&["speak", "hi", "--pages", "1-2", "-o", "a.opus"]).is_err());
        assert!(parse(&[
            "speak",
            "--input",
            "b.pdf",
            "--skip-before",
            "2",
            "-o",
            "a.opus"
        ])
        .is_ok());
    }

    #[test]
    fn extract_mode_flags_conflict() {
        assert!(parse(&["extract", "b.pdf", "--spoken", "--chapters"]).is_err());
        assert!(parse(&["extract", "b.pdf", "--spoken", "-o", "x.txt"]).is_err());
        assert!(parse(&["extract", "b.pdf", "--chapters", "-o", "x.txt"]).is_err());
        assert!(parse(&["extract", "b.pdf", "--chapters"]).is_ok());
    }

    #[test]
    fn numeric_ranges() {
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--speed", "2.5"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--speed", "NaN"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--pitch", "-12"]).is_ok());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--pitch", "13"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--paragraph-pause", "11"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--loudness", "-16"]).is_ok());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--loudness", "-40"]).is_err());
        assert!(parse(&["speak", "hi", "-o", "a.opus", "--loudness", "inf"]).is_err());
    }
}

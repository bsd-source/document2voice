//! # document2voice-core
//!
//! Shared library for **Document2Voice (Kokoro)** — turns documents and plain text into
//! spoken audio offline with the Kokoro ONNX speech model.

pub(crate) mod bundle;
pub mod chapters;
pub(crate) mod config;
pub(crate) mod document;
pub(crate) mod error;
pub(crate) mod export;
pub(crate) mod heteronyms;
pub(crate) mod mobi_text;
pub(crate) mod pdf;
pub(crate) mod speech_sanitize;
pub(crate) mod synth_worker;
pub(crate) mod tts;
pub(crate) mod voices;

pub use config::{ExportFormat, ExportOptions};
pub use document::{extract_document, DocExtract, DocKind, ExtractOptions};
pub use error::CoreError;
pub use export::{discard_partial_export, partial_export, split_output_dir, PartialExport};
pub use pdf::{
    format_page_list, parse_page_range, parse_skip_pages, pdf_page_count, validate_page_selection,
    PageRange, PdfPages,
};
pub use synth_worker::{load_worker_text, run_synth_worker_job, JobReport, SynthWorkerRequest};
pub use tts::prepare_speech_paragraphs;
pub use voices::{voices_for_language, Language, VoiceInfo};

/// How many audio samples per second Kokoro produces (24,000 Hz).
pub const KOKORO_SAMPLE_RATE: u32 = 24_000;

/// Version number of this crate (from the workspace Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

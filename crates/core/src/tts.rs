use crate::config::SpeakOptions;
use crate::error::{CoreError, Result};
use crate::KOKORO_SAMPLE_RATE;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing::info;
use tts_rs::engines::kokoro::{KokoroEngine, KokoroInferenceParams, KokoroModelParams};
use tts_rs::SynthesisEngine;

/// Offline text-to-speech using the Kokoro model (ONNX file format) via `tts-rs`,
/// with espeak-ng for grapheme→phoneme.
pub struct TtsEngine {
    inner: KokoroEngine,
}

impl TtsEngine {
    /// Load the speech engine from the default model folder search path.
    pub fn new() -> Result<Self> {
        Self::load(resolve_model_dir(), env_usize("DOCUMENT2VOICE_ORT_THREADS"))
    }

    /// Load from the default model folder with a fixed ONNX Runtime thread count
    /// (`None` = ORT default, i.e. all cores). Used by the parallel synthesizer.
    pub fn with_threads(threads: Option<usize>) -> Result<Self> {
        Self::load(resolve_model_dir(), threads)
    }

    fn load(model_dir: PathBuf, num_threads: Option<usize>) -> Result<Self> {
        if !model_dir.is_dir() {
            return Err(CoreError::Tts(format!(
                "model directory not found: {}\n\n\
                 Download models with:\n  \
                 ./setup.sh\n\
                 Or set DOCUMENT2VOICE_MODEL_DIR to a folder with kokoro ONNX + voices-v1.0.bin",
                model_dir.display()
            )));
        }

        info!(path = %model_dir.display(), "loading Kokoro ONNX model");
        // AppImage / portable builds: prefer an explicit or bundled espeak-ng.
        let espeak_bin = crate::bundle::resolve_espeak_bin();
        let espeak_data = crate::bundle::resolve_espeak_data();
        let mut inner = match (espeak_bin, espeak_data) {
            (None, None) => KokoroEngine::new(),
            (bin, data) => {
                info!(
                    bin = ?bin.as_ref().map(|p| p.display().to_string()),
                    data = ?data.as_ref().map(|p| p.display().to_string()),
                    "using bundled espeak-ng"
                );
                KokoroEngine::with_espeak(bin, data)
            }
        };
        let cache = prepare_optimized_cache_path();
        let params = KokoroModelParams {
            num_threads,
            optimized_model_cache_path: cache,
        };
        // The first load in this process may write the optimized-model cache; hold
        // the lock until it succeeds so parallel engines read a finished file.
        let mut first = FIRST_LOAD_DONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let load = |inner: &mut KokoroEngine| {
            inner
                .load_model_with_params(&model_dir, params)
                .map_err(|e| CoreError::Tts(e.to_string()))
        };
        if *first {
            drop(first);
            load(&mut inner)?;
        } else {
            load(&mut inner)?;
            *first = true;
        }

        Ok(Self { inner })
    }

    /// Paragraph loop that hands each finished paragraph (and the pause after it)
    /// to `sink` instead of keeping the whole book in memory.
    ///
    /// `max_secs` stops after about that much audio (previews). Returns the number of
    /// samples passed to `sink`.
    pub fn synthesize_streaming(
        &mut self,
        text: &str,
        opts: &SpeakOptions,
        max_secs: Option<f32>,
        on_progress: &mut dyn FnMut(usize, usize, &str),
        sink: &mut dyn FnMut(&[f32]) -> Result<()>,
    ) -> Result<usize> {
        let preview = max_secs.is_some();
        let max_samples =
            max_secs.map(|s| ((KOKORO_SAMPLE_RATE as f32) * s.clamp(0.5, 120.0)).round() as usize);

        let paragraphs = prepare_speech_paragraphs(text)?;

        let total = paragraphs.len();
        let pause = vec![0.0f32; pause_sample_count(opts.paragraph_pause_secs)];
        let mut written = 0usize;

        for (i, para) in paragraphs.iter().enumerate() {
            if max_samples.is_some_and(|m| written >= m) {
                break;
            }

            let words = para.split_whitespace().count();
            let label = if preview {
                format!("Preview paragraph {} of {total}", i + 1)
            } else {
                format!(
                    "Synthesizing paragraph {} of {total} ({words} words)",
                    i + 1
                )
            };
            on_progress(i, total, &label);

            let mut samples = self.synthesize_chunk(para, opts)?;

            let mut done = false;
            if let Some(cap) = max_samples {
                let room = cap.saturating_sub(written);
                if samples.len() >= room {
                    samples.truncate(room);
                    done = true;
                }
            }
            sink(&samples)?;
            written += samples.len();
            on_progress(i + 1, total, &label);
            if done {
                break;
            }

            if i + 1 < total && !pause.is_empty() {
                let n = max_samples.map_or(pause.len(), |cap| {
                    pause.len().min(cap.saturating_sub(written))
                });
                sink(&pause[..n])?;
                written += n;
            }
        }

        on_progress(
            total,
            total,
            if preview {
                "Preview synthesis complete"
            } else {
                "Synthesis complete"
            },
        );

        if written == 0 {
            return Err(CoreError::Tts("synthesis produced no audio".into()));
        }
        Ok(written)
    }

    fn synthesize_chunk(&mut self, text: &str, opts: &SpeakOptions) -> Result<Vec<f32>> {
        // Speak a leading list number separately, without its period, followed by a
        // fixed pause; Kokoro otherwise runs it into the next word.
        if let Some((marker, rest)) = split_list_marker(text) {
            // Each synthesis carries its own lead-in / tail silence: trim the two
            // that meet, so the pause is the intended length.
            let mut samples = self.synthesize_plain(marker.trim_end_matches('.'), opts)?;
            let keep = samples.len() - trailing_silence(&samples);
            samples.truncate(keep);
            let gap = LIST_MARKER_PAUSE_SECS / opts.speed.clamp(0.5, 2.0);
            samples.extend(std::iter::repeat_n(0.0f32, pause_sample_count(gap)));
            let body = self.synthesize_plain(rest, opts)?;
            let lead = leading_silence(&body);
            samples.extend_from_slice(&body[lead..]);
            return Ok(samples);
        }
        self.synthesize_plain(text, opts)
    }

    fn synthesize_plain(&mut self, text: &str, opts: &SpeakOptions) -> Result<Vec<f32>> {
        let params = KokoroInferenceParams {
            voice: opts.voice.clone(),
            speed: opts.speed.clamp(0.5, 2.0),
            style_index: opts.style_index,
        };
        self.inner
            .synthesize(text, Some(params))
            .map(|r| r.samples)
            .map_err(|e| CoreError::Tts(e.to_string()))
    }
}

/// Pause after a list number (`1.`) that starts a paragraph, at normal speed.
const LIST_MARKER_PAUSE_SECS: f32 = 0.35;

/// Below this level a sample counts as silence when trimming joins.
const SILENCE_LEVEL: f32 = 0.01;
/// Room kept next to the speech when trimming, so word edges aren't clipped.
const TRIM_MARGIN_SECS: f32 = 0.03;

/// Quiet samples at the end of `s`, less a short margin.
fn trailing_silence(s: &[f32]) -> usize {
    let quiet = s
        .iter()
        .rev()
        .take_while(|x| x.abs() < SILENCE_LEVEL)
        .count();
    quiet.saturating_sub(pause_sample_count(TRIM_MARGIN_SECS))
}

/// Quiet samples at the start of `s`, less a short margin.
fn leading_silence(s: &[f32]) -> usize {
    let quiet = s.iter().take_while(|x| x.abs() < SILENCE_LEVEL).count();
    quiet.saturating_sub(pause_sample_count(TRIM_MARGIN_SECS))
}

/// `1. Make it…` / `12. Target…` → (`1.`, `Make it…`). Only a 1–3 digit number with
/// a period and a space, followed by a word (not `3. 5`, not `1.5 volts`).
fn split_list_marker(text: &str) -> Option<(&str, &str)> {
    let t = text.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=3).contains(&digits) || t.as_bytes().get(digits) != Some(&b'.') {
        return None;
    }
    let after = &t[digits + 1..];
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = after.trim_start();
    rest.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || matches!(c, '"' | '“' | '\'' | '‘' | '['))
        .then_some((&t[..=digits], rest))
}

/// Silence that replaces a paragraph that could not be synthesized.
pub const SKIP_SILENCE_SECS: f32 = 0.5;
/// More than this many skipped paragraphs …
pub const SKIP_LIMIT_ABS: usize = 3;
/// … that are also over this fraction of the paragraphs done so far abort the job.
pub const SKIP_LIMIT_FRACTION: f32 = 0.10;

/// Sanitize `text` for speech and split it into blank-line paragraphs, the units
/// that are synthesized one at a time.
pub fn prepare_speech_paragraphs(text: &str) -> Result<Vec<String>> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CoreError::Tts("empty text".into()));
    }
    // TTS-only sanitizer (symbols, money, dates, caps, …).
    let text = crate::speech_sanitize::sanitize_for_speech(text);
    // espeak cannot use part of speech: fix the heteronyms it gets wrong.
    let text = crate::heteronyms::respell_heteronyms_for_espeak(&text);
    // One unit per blank-line paragraph. Kokoro may still re-chunk long phoneme runs.
    let paragraphs = split_paragraphs(&text);
    if paragraphs.is_empty() {
        return Err(CoreError::Tts("empty text".into()));
    }
    Ok(paragraphs)
}

fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n > 0)
}

/// How many single-threaded engines to run for a full export.
///
/// Several single-threaded engines on separate paragraphs outperform one
/// multi-threaded engine; each uses ~700 MB, so the count is capped by available
/// memory. Override with `DOCUMENT2VOICE_SYNTH_WORKERS`.
pub fn default_synth_workers() -> usize {
    if let Some(n) = env_usize("DOCUMENT2VOICE_SYNTH_WORKERS") {
        return n.min(16);
    }
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let by_cpu = (threads / 2).clamp(1, 4);
    // Keep all engines within ~60% of currently available RAM.
    match mem_available_mb() {
        Some(mb) => by_cpu.min((mb * 6 / 10 / ENGINE_RAM_MB).max(1)),
        None => by_cpu,
    }
}

/// Measured resident size of one loaded Kokoro engine (model + ORT arena), rounded up.
const ENGINE_RAM_MB: usize = 700;

fn mem_available_mb() -> Option<usize> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Full-length synthesis of prepared `paragraphs` (from [`prepare_speech_paragraphs`])
/// with `workers` independent engines on different paragraphs at once. Output reaches
/// `sink` strictly in paragraph order with the paragraph pause between, so the result
/// is identical to a single engine.
///
/// Starts at paragraph `start` (resume). After each paragraph and its pause are
/// written, `checkpoint(done)` is called with the number of paragraphs finished.
///
/// A paragraph that fails is retried once; if it fails again it is replaced by
/// [`SKIP_SILENCE_SECS`] of silence and reported through `on_skip(index, error)`
/// before that silence is written. Too many failures (more than
/// [`SKIP_LIMIT_ABS`] and over [`SKIP_LIMIT_FRACTION`] of the paragraphs done so
/// far) look systematic — a broken espeak or model — and abort the job instead.
/// `on_progress(done, total, label)` runs on this thread. With one worker, that
/// engine uses all ORT threads. Returns samples passed to `sink`.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_parallel_streaming(
    paragraphs: &[String],
    start: usize,
    opts: &SpeakOptions,
    workers: usize,
    on_progress: &mut dyn FnMut(usize, usize, &str),
    sink: &mut dyn FnMut(&[f32]) -> Result<()>,
    checkpoint: &mut dyn FnMut(usize) -> Result<()>,
    on_skip: &mut dyn FnMut(usize, &str) -> Result<()>,
) -> Result<usize> {
    let ort_threads = if workers.min(paragraphs.len().saturating_sub(start)) > 1 {
        Some(1) // several engines: one ORT thread each (scales better)
    } else {
        env_usize("DOCUMENT2VOICE_ORT_THREADS") // one engine: ORT default (all cores)
    };
    run_parallel(
        paragraphs,
        start,
        opts,
        workers,
        &|| TtsEngine::with_threads(ort_threads),
        &|engine: &mut TtsEngine, text: &str| engine.synthesize_chunk(text, opts),
        on_progress,
        sink,
        checkpoint,
        on_skip,
    )
}

/// The engine-agnostic paragraph scheduler behind [`synthesize_parallel_streaming`]
/// (generic so tests can drive it with a fake engine).
#[allow(clippy::too_many_arguments)]
fn run_parallel<E>(
    paragraphs: &[String],
    start: usize,
    opts: &SpeakOptions,
    workers: usize,
    make_engine: &(dyn Fn() -> Result<E> + Sync),
    synth: &(dyn Fn(&mut E, &str) -> Result<Vec<f32>> + Sync),
    on_progress: &mut dyn FnMut(usize, usize, &str),
    sink: &mut dyn FnMut(&[f32]) -> Result<()>,
    checkpoint: &mut dyn FnMut(usize) -> Result<()>,
    on_skip: &mut dyn FnMut(usize, &str) -> Result<()>,
) -> Result<usize> {
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    let total = paragraphs.len();
    if total == 0 {
        return Err(CoreError::Tts("empty text".into()));
    }
    let start = start.min(total);
    let remaining = total - start;
    if remaining == 0 {
        return Ok(0);
    }
    let workers = workers.clamp(1, remaining);
    info!(workers, paragraphs = total, start, "synthesis");
    let loading = if start > 0 {
        format!(
            "Resuming at paragraph {} of {total} — loading speech engines…",
            start + 1
        )
    } else if workers > 1 {
        format!("Loading {workers} speech engines…")
    } else {
        "Loading Kokoro model…".to_string()
    };
    on_progress(start, total, &loading);

    let next = AtomicUsize::new(start); // next paragraph index to hand out
    let emitted = AtomicUsize::new(start); // paragraphs already written, in order
    let stop = AtomicBool::new(false);
    // Workers may run at most this far ahead of the writer (bounds buffered audio).
    let window = workers * 3;
    let pause = vec![0.0f32; pause_sample_count(opts.paragraph_pause_secs)];
    let (tx, rx) = mpsc::channel::<(usize, Result<Vec<f32>>)>();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, emitted, stop) = (&next, &emitted, &stop);
            scope.spawn(move || {
                let mut engine = match make_engine() {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = tx.send((usize::MAX, Err(e)));
                        return;
                    }
                };
                loop {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let i = next.load(Ordering::Acquire);
                    if i >= total {
                        return;
                    }
                    // Wait while too far ahead of the writer.
                    if i >= emitted.load(Ordering::Acquire) + window {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        continue;
                    }
                    if next
                        .compare_exchange(i, i + 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        continue;
                    }
                    // One retry on the same engine (espeak hiccups, odd ORT states).
                    let r = synth(&mut engine, &paragraphs[i]).or_else(|e| {
                        tracing::warn!(paragraph = i + 1, error = %e, "paragraph failed; retrying once");
                        synth(&mut engine, &paragraphs[i])
                    });
                    if tx.send((i, r)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(tx);

        // Writer: reorder results and stream them out in paragraph order.
        let mut pending: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
        let mut written = 0usize;
        let mut done = start;
        let mut skipped = 0usize;
        let mut failed_engines = 0usize;
        let silence = vec![0.0f32; pause_sample_count(SKIP_SILENCE_SECS)];
        let result = (|| -> Result<usize> {
            while done < total {
                let (i, r) = match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(m) => m,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(CoreError::Tts("speech workers stopped unexpectedly".into()));
                    }
                };
                if i >= total {
                    // An engine failed to load; the others carry on without it.
                    if let Err(e) = r {
                        failed_engines += 1;
                        if failed_engines == workers {
                            return Err(e);
                        }
                        tracing::warn!(error = %e, failed_engines, workers, "speech engine failed to load; continuing with fewer");
                    }
                    continue;
                }
                let samples = match r {
                    Ok(s) => s,
                    Err(e) => {
                        skipped += 1;
                        let processed = (done - start).max(1);
                        if skipped > SKIP_LIMIT_ABS
                            && (skipped as f32) > SKIP_LIMIT_FRACTION * processed as f32
                        {
                            return Err(CoreError::Tts(format!(
                                "{skipped} paragraphs failed (latest: paragraph {} of {total}: {e}) — \
                                 stopping, this looks like a setup problem rather than odd text",
                                i + 1
                            )));
                        }
                        tracing::warn!(paragraph = i + 1, error = %e, "paragraph skipped (silence)");
                        on_skip(i, &e.to_string())?;
                        silence.clone()
                    }
                };
                pending.insert(i, samples);
                while let Some(samples) = pending.remove(&done) {
                    sink(&samples)?;
                    written += samples.len();
                    done += 1;
                    if done < total && !pause.is_empty() {
                        sink(&pause)?;
                        written += pause.len();
                    }
                    checkpoint(done)?;
                    emitted.store(done, Ordering::Release);
                    let words = paragraphs[done - 1].split_whitespace().count();
                    let engines = if workers > 1 {
                        format!(", {workers} engines")
                    } else {
                        String::new()
                    };
                    on_progress(
                        done,
                        total,
                        &format!(
                            "Synthesized paragraph {done} of {total} ({words} words{engines})"
                        ),
                    );
                }
            }
            Ok(written)
        })();
        stop.store(true, Ordering::Release);
        let written = result?;
        on_progress(total, total, "Synthesis complete");
        Ok(written)
    })
}

/// Set once an engine has loaded in this process (see [`TtsEngine::load`]).
static FIRST_LOAD_DONE: Mutex<bool> = Mutex::new(false);

/// Base path for the optimized-model cache. The loader stores the graph beside it
/// under a name that includes the model's size and modification time; empty or
/// broken entries from an interrupted write are removed here.
fn prepare_optimized_cache_path() -> Option<PathBuf> {
    let cache_root = dirs::cache_dir()?.join("document2voice");
    let _ = std::fs::create_dir_all(&cache_root);

    if let Ok(entries) = std::fs::read_dir(&cache_root) {
        for e in entries.flatten() {
            let name = e.file_name();
            if name.to_string_lossy().starts_with("kokoro-optimized") {
                remove_if_dir_or_tiny_file(&e.path());
            }
        }
    }

    Some(cache_root.join("kokoro-optimized.onnx"))
}

fn remove_if_dir_or_tiny_file(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else if path.is_file() && path.metadata().map_or(true, |m| m.len() < 1024) {
        let _ = std::fs::remove_file(path);
    }
}

/// Split text on blank lines (one or more empty lines between blocks).
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paras = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !buf.is_empty() {
                paras.push(buf.join("\n"));
                buf.clear();
            }
        } else {
            buf.push(line);
        }
    }
    if !buf.is_empty() {
        paras.push(buf.join("\n"));
    }
    // If there are no blank lines, the whole text is one paragraph.
    if paras.is_empty() && !text.trim().is_empty() {
        paras.push(text.trim().to_string());
    }
    paras
}

fn pause_sample_count(secs: f32) -> usize {
    if secs <= 0.0 {
        return 0;
    }
    let n = (secs * KOKORO_SAMPLE_RATE as f32).round() as usize;
    n.min(KOKORO_SAMPLE_RATE as usize * 10) // never more than 10 seconds of pause
}

fn resolve_model_dir() -> PathBuf {
    // An explicit folder wins even when it is wrong, so the error names it.
    if let Ok(p) = std::env::var("DOCUMENT2VOICE_MODEL_DIR") {
        return PathBuf::from(p);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // AppImage / portable payload (set by AppRun or next to the binary).
    candidates.extend(crate::bundle::bundled_model_dir_candidates());

    // Current folder and its parents (e.g. cargo run from the repo root).
    if let Ok(cwd) = std::env::current_dir() {
        push_ancestor_model_candidates(&mut candidates, &cwd);
    }

    // Folder of this program and its parents (e.g. target/release/… → ../../models/kokoro).
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        push_ancestor_model_candidates(&mut candidates, &exe_dir);
        candidates.push(exe_dir.join("models/kokoro"));
        candidates.push(exe_dir.join("../share/document2voice/models/kokoro"));
    }

    if let Some(data) = dirs::data_local_dir() {
        candidates.push(data.join("document2voice/models/kokoro"));
    }

    for c in candidates {
        if looks_like_model_dir(&c) {
            return c;
        }
    }

    // Fallback used only in the error message when nothing was found.
    PathBuf::from("models/kokoro")
}

/// Collect `models/kokoro` (and bare `models`) under `start` and its parent folders.
fn push_ancestor_model_candidates(out: &mut Vec<PathBuf>, start: &Path) {
    let mut dir = Some(start.to_path_buf());
    // Limit how many parents we walk so a strange install path does not scan the whole disk.
    for _ in 0..10 {
        let Some(d) = dir else {
            break;
        };
        out.push(d.join("models/kokoro"));
        out.push(d.join("models"));
        dir = d.parent().map(Path::to_path_buf);
    }
}

fn looks_like_model_dir(p: &Path) -> bool {
    if !p.is_dir() {
        return false;
    }
    let has_voices = p.join("voices-v1.0.bin").is_file()
        || p.join("voices.bin").is_file()
        || dir_has_extension(p, "bin");
    let has_onnx = dir_has_extension(p, "onnx");
    has_voices && has_onnx
}

fn dir_has_extension(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(std::io::Result::ok)
            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some(ext))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake engine: 10 samples of value (index+1) per paragraph; "FAIL" always
    /// errors, "FLAKY" errors only on the first try.
    struct Fake {
        tries: std::collections::HashMap<String, usize>,
    }

    fn fake_run(
        paras: &[&str],
        workers: usize,
    ) -> (Result<usize>, Vec<f32>, Vec<usize>, Vec<usize>) {
        fake_run_failing_loads(paras, workers, 0)
    }

    /// Like `fake_run`, with the first `fail_loads` engine loads failing.
    fn fake_run_failing_loads(
        paras: &[&str],
        workers: usize,
        fail_loads: isize,
    ) -> (Result<usize>, Vec<f32>, Vec<usize>, Vec<usize>) {
        let fail_loads = std::sync::atomic::AtomicIsize::new(fail_loads);
        let paragraphs: Vec<String> = paras.iter().map(|s| (*s).to_string()).collect();
        let opts = SpeakOptions {
            paragraph_pause_secs: 0.0,
            ..SpeakOptions::default()
        };
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        let mut checkpoints = Vec::new();
        let r = run_parallel(
            &paragraphs,
            0,
            &opts,
            workers,
            &|| {
                if fail_loads.fetch_sub(1, Ordering::AcqRel) > 0 {
                    return Err(CoreError::Tts("engine load failed".into()));
                }
                Ok(Fake {
                    tries: Default::default(),
                })
            },
            &|e: &mut Fake, text: &str| {
                let n = e.tries.entry(text.to_string()).or_default();
                *n += 1;
                if text.contains("FAIL") || (text.contains("FLAKY") && *n == 1) {
                    return Err(CoreError::Tts(format!("cannot say {text}")));
                }
                let v: f32 = text
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|d| d.parse().ok())
                    .unwrap_or(0.0);
                Ok(vec![v; 10])
            },
            &mut |_, _, _| {},
            &mut |c| {
                out.extend_from_slice(c);
                Ok(())
            },
            &mut |d| {
                checkpoints.push(d);
                Ok(())
            },
            &mut |i, _| {
                skipped.push(i);
                Ok(())
            },
        );
        (r, out, skipped, checkpoints)
    }

    #[test]
    fn failed_paragraph_becomes_silence_in_order() {
        let (r, out, skipped, cps) = fake_run(&["p1", "p2 FAIL", "p3 FLAKY", "p4"], 3);
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(skipped, [1]);
        assert_eq!(cps, [1, 2, 3, 4]);
        let silence = pause_sample_count(SKIP_SILENCE_SECS);
        assert_eq!(out.len(), 10 + silence + 10 + 10);
        assert!(out[..10].iter().all(|&x| x == 1.0));
        assert!(out[10..10 + silence].iter().all(|&x| x == 0.0));
        assert!(
            out[10 + silence..20 + silence].iter().all(|&x| x == 3.0),
            "flaky retried"
        );
        assert!(out[20 + silence..].iter().all(|&x| x == 4.0));
    }

    #[test]
    fn job_continues_when_some_engines_fail_to_load() {
        let (r, out, skipped, _) = fake_run_failing_loads(&["p1", "p2", "p3", "p4"], 3, 2);
        assert!(r.is_ok(), "{r:?}");
        assert!(skipped.is_empty());
        assert_eq!(out.len(), 40);
        let (r, _, _, _) = fake_run_failing_loads(&["p1", "p2", "p3"], 3, 3);
        assert!(r.is_err());
    }

    #[test]
    fn systematic_failure_aborts() {
        let paras: Vec<String> = (1..=20).map(|i| format!("p{i} FAIL")).collect();
        let refs: Vec<&str> = paras.iter().map(String::as_str).collect();
        let (r, _, skipped, _) = fake_run(&refs, 2);
        assert!(r.is_err());
        assert!(skipped.len() <= SKIP_LIMIT_ABS, "{skipped:?}");
    }

    #[test]
    fn list_marker_split() {
        assert_eq!(
            split_list_marker("1. Make it easy"),
            Some(("1.", "Make it easy"))
        );
        assert_eq!(
            split_list_marker("12. \"Quote\" here"),
            Some(("12.", "\"Quote\" here"))
        );
        assert_eq!(split_list_marker("1.5 volts"), None);
        assert_eq!(split_list_marker("3. 5 apples"), None);
        assert_eq!(split_list_marker("2026. The year"), None);
        assert_eq!(split_list_marker("Make 1. it"), None);
        assert_eq!(split_list_marker("1."), None);
    }

    #[test]
    fn split_paragraphs_blank_lines() {
        let p = split_paragraphs("A one.\n\nB two.");
        assert_eq!(p, ["A one.", "B two."]);
    }
}

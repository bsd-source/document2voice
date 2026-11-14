//! Background speech jobs: full export, short preview, and external tool checks.
//!
//! Start / Preview run text-to-speech in a separate killable child process (the GUI
//! re-launches itself with `--synth-worker`) so Cancel can force-stop mid-work,
//! including ONNX speech. Sample-voice Preview keeps a small disk cache; cache misses
//! go through the same killable worker.

use crate::types::{JobProgress, PreviewJob, SpeakJob, SpeechParams};
use crate::PREVIEW_SECS;
use document2voice_core::{
    load_worker_text, run_synth_worker_job, ExportFormat, SynthWorkerRequest, KOKORO_SAMPLE_RATE,
};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Flag the GUI re-launches itself with for killable speech work (must match `main`).
pub(crate) const SYNTH_WORKER_FLAG: &str = "--synth-worker";

/// Share of the progress bar reserved for making the preview audio; the rest
/// (`PREVIEW_SYNTH_END` → 1.0) tracks about [`PREVIEW_SECS`] of playback.
const PREVIEW_SYNTH_END: f32 = 0.20;

/// Bump when [`crate::VOICE_PREVIEW_TEXT`] or cache layout changes.
const SAMPLE_VOICE_CACHE_VERSION: u32 = 7;

/// Sample-voice previews kept on disk (most recently used first).
const SAMPLE_VOICE_CACHE_MAX: usize = 20;

/// Error text for a job stopped by Cancel; compared exactly by the UI.
pub(crate) const CANCELLED: &str = "cancelled";

/// Child processes and temp files of the running job.
///
/// Jobs register what they start so the window-close path can stop and delete
/// them synchronously before the app exits.
#[derive(Debug, Default)]
pub(crate) struct ActiveJob {
    inner: Mutex<ActiveJobInner>,
}

#[derive(Debug, Default)]
struct ActiveJobInner {
    pids: Vec<u32>,
    files: Vec<PathBuf>,
}

impl ActiveJob {
    fn with<R>(&self, f: impl FnOnce(&mut ActiveJobInner) -> R) -> R {
        let mut g = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }

    fn add_pid(&self, pid: u32) {
        self.with(|j| j.pids.push(pid));
    }

    fn remove_pid(&self, pid: u32) {
        self.with(|j| j.pids.retain(|&p| p != pid));
    }

    fn add_file(&self, path: &Path) {
        self.with(|j| j.files.push(path.to_path_buf()));
    }

    fn remove_file(&self, path: &Path) {
        self.with(|j| j.files.retain(|p| p != path));
    }

    /// Kill every registered process group and delete every registered temp file.
    pub(crate) fn shutdown(&self) {
        let (pids, files) =
            self.with(|j| (std::mem::take(&mut j.pids), std::mem::take(&mut j.files)));
        for pid in pids {
            kill_worker_group(pid);
        }
        for f in files {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// One entry per tool for the startup banner, e.g. `ffmpeg 8.1.2`, `pdfinfo missing`.
pub(crate) fn probe_external_tools() -> Vec<String> {
    const TOOLS: &[(&str, &str, &[&str])] = &[
        ("ffmpeg", "ffmpeg", &["-version"]),
        ("espeak-ng", "espeak-ng", &["--version"]),
        ("pdftotext", "pdftotext", &["-v"]),
        ("pdfinfo", "pdfinfo", &["-v"]),
    ];
    TOOLS
        .iter()
        .map(|&(label, bin, args)| format_tool_compact(label, bin, args))
        .collect()
}

fn format_tool_compact(label: &str, bin: &str, args: &[&str]) -> String {
    match Command::new(bin).args(args).output() {
        Ok(out) => {
            let text = if out.stdout.is_empty() {
                String::from_utf8_lossy(&out.stderr)
            } else {
                String::from_utf8_lossy(&out.stdout)
            };
            let first = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            if first.is_empty() {
                format!("{label} ok")
            } else {
                // Drop a leading `v` for a cleaner `ffmpeg 8.1.2` look.
                let ver = summarize_version_line(first);
                let ver = ver.strip_prefix('v').unwrap_or(&ver);
                format!("{label} {ver}")
            }
        }
        Err(_) => format!("{label} missing"),
    }
}

fn first_token(s: &str) -> Option<String> {
    s.split_whitespace().next().map(str::to_string)
}

fn summarize_version_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let token = if let Some(rest) = lower.strip_prefix("ffmpeg version ") {
        first_token(rest)
    } else if lower.contains("espeak") {
        line.find(':').and_then(|idx| first_token(&line[idx + 1..]))
    } else if let Some(idx) = lower.find("version ") {
        first_token(&line[idx + "version ".len()..])
    } else {
        None
    };

    if let Some(token) = token {
        return with_v_prefix(&token);
    }
    if line.len() > 80 {
        format!("{}…", &line[..80])
    } else {
        line.to_string()
    }
}

fn with_v_prefix(version: &str) -> String {
    let v = version.trim();
    let v = v.strip_prefix(['n', 'N']).unwrap_or(v);
    if let Some(rest) = v.strip_prefix(['v', 'V']) {
        format!("v{rest}")
    } else {
        format!("v{v}")
    }
}

pub(crate) fn report(progress: &Arc<Mutex<JobProgress>>, fraction: f32, label: &str) {
    if let Ok(mut g) = progress.lock() {
        g.fraction = fraction.clamp(0.0, 1.0);
        g.label = label.to_string();
    }
}

/// Queue a Log line from a running job (shown on the next progress tick).
pub(crate) fn note(progress: &Arc<Mutex<JobProgress>>, line: impl Into<String>) {
    if let Ok(mut g) = progress.lock() {
        g.notes.push(line.into());
    }
}

fn bail_if_cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::Acquire) {
        anyhow::bail!(CANCELLED);
    }
    Ok(())
}

/// Entry point when the GUI is re-launched with `--synth-worker …`.
///
/// Args are everything after `--synth-worker` (same flags as the command-line worker).
pub(crate) fn run_embedded_synth_worker(
    args: impl IntoIterator<Item = String>,
) -> anyhow::Result<()> {
    let req = parse_embedded_worker_args(args)?;
    let report = run_synth_worker_job(&req, |frac, label| {
        emit_worker_progress(frac, label);
    })?;
    emit_worker_json(&serde_json::json!({
        "type": "done",
        "path": report.path.display().to_string(),
        "warning": report.warning(),
    }));
    Ok(())
}

/// `--synth-worker` flags as parsed. The GUI always passes every valued flag;
/// `--style` and `--max-secs` are optional, the rest are switches.
#[derive(Default)]
struct EmbeddedWorkerArgs {
    text_file: Option<PathBuf>,
    out: Option<PathBuf>,
    voice: Option<String>,
    speed: Option<f32>,
    pitch: Option<f32>,
    style: Option<usize>,
    paragraph_pause: Option<f32>,
    loudness: Option<f32>,
    format: Option<ExportFormat>,
    no_loudnorm: bool,
    bitrate: Option<String>,
    sample_rate: Option<u32>,
    max_secs: Option<f32>,
    split_chapters: bool,
}

fn required<T>(value: Option<T>, flag: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| anyhow::anyhow!("{flag} required"))
}

/// Parse `--synth-worker` CLI flags into a request (mirrors the command-line worker).
fn parse_embedded_worker_args(
    args: impl IntoIterator<Item = String>,
) -> anyhow::Result<SynthWorkerRequest> {
    let args: Vec<String> = args.into_iter().collect();
    let mut parsed = EmbeddedWorkerArgs::default();
    let mut i = 0;
    while i < args.len() {
        i = apply_embedded_worker_flag(&args, i, &mut parsed)?;
    }

    let text_file = required(parsed.text_file, "--text-file")?;
    let text = load_worker_text(&text_file)?;
    Ok(SynthWorkerRequest {
        text,
        out: required(parsed.out, "--out")?,
        voice: required(parsed.voice, "--voice")?,
        speed: required(parsed.speed, "--speed")?,
        pitch: required(parsed.pitch, "--pitch")?,
        style_index: parsed.style,
        paragraph_pause_secs: required(parsed.paragraph_pause, "--paragraph-pause")?,
        loudness: required(parsed.loudness, "--loudness")?,
        format: required(parsed.format, "--format")?,
        loudnorm: !parsed.no_loudnorm,
        bitrate: required(parsed.bitrate, "--bitrate")?,
        sample_rate: required(parsed.sample_rate, "--sample-rate")?,
        max_secs: parsed.max_secs,
        split_chapters: parsed.split_chapters,
    })
}

/// Apply one `--flag` (and optional value) starting at `i`; returns the next index.
fn apply_embedded_worker_flag(
    args: &[String],
    i: usize,
    parsed: &mut EmbeddedWorkerArgs,
) -> anyhow::Result<usize> {
    let a = args[i].as_str();
    let next = || {
        args.get(i + 1)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing value after {a}"))
    };
    match a {
        "--text-file" => {
            parsed.text_file = Some(PathBuf::from(next()?));
            Ok(i + 2)
        }
        "--out" | "-o" => {
            parsed.out = Some(PathBuf::from(next()?));
            Ok(i + 2)
        }
        "--voice" => {
            parsed.voice = Some(next()?);
            Ok(i + 2)
        }
        "--speed" => {
            parsed.speed = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--pitch" => {
            parsed.pitch = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--style" => {
            parsed.style = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--paragraph-pause" => {
            parsed.paragraph_pause = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--loudness" => {
            parsed.loudness = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--bitrate" => {
            parsed.bitrate = Some(next()?);
            Ok(i + 2)
        }
        "--sample-rate" => {
            parsed.sample_rate = Some(next()?.parse()?);
            Ok(i + 2)
        }
        "--format" => {
            let name = next()?;
            parsed.format = Some(
                ExportFormat::from_name(&name)
                    .ok_or_else(|| anyhow::anyhow!("unknown format: {name}"))?,
            );
            Ok(i + 2)
        }
        "--split-chapters" => {
            parsed.split_chapters = true;
            Ok(i + 1)
        }
        "--no-loudnorm" => {
            parsed.no_loudnorm = true;
            Ok(i + 1)
        }
        "--max-secs" => {
            parsed.max_secs = Some(next()?.parse()?);
            Ok(i + 2)
        }
        other => anyhow::bail!("unknown synth-worker arg: {other}"),
    }
}

fn emit_worker_json(obj: &serde_json::Value) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{obj}");
    let _ = out.flush();
}

fn emit_worker_progress(fraction: f32, label: &str) {
    emit_worker_json(&serde_json::json!({
        "type": "progress",
        "fraction": fraction.clamp(0.0, 1.0),
        "label": label,
    }));
}

/// Path for a new temp file: process id, time and a counter keep names unique.
fn unique_temp(prefix: &str, ext: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let ns = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "document2voice-{prefix}-{}-{ns}-{n}.{ext}",
        std::process::id()
    ))
}

/// Write worker text to a freshly created temp file (never an existing path),
/// owner-only on Unix. Returns the path.
fn write_worker_text(text: &str) -> anyhow::Result<PathBuf> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    for _ in 0..8 {
        let path = unique_temp("speak-text", "txt");
        match opts.open(&path) {
            Ok(mut f) => {
                let written = f.write_all(text.as_bytes()).and_then(|()| f.sync_all());
                if let Err(e) = written {
                    let _ = std::fs::remove_file(&path);
                    return Err(e.into());
                }
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!(
        "could not create a unique temp file in {}",
        std::env::temp_dir().display()
    )
}

/// Force-stop a child's process group so ONNX speech / ffmpeg stop immediately.
#[cfg(unix)]
fn kill_worker_group(pid: u32) {
    // Never signal "every process" (pid 0) or the init process group.
    if pid == 0 || pid == 1 {
        return;
    }
    let Ok(pid_i) = i32::try_from(pid) else {
        return;
    };
    // SAFETY: `kill` only delivers a signal. Children are spawned with
    // `process_group(0)`, so `-pid` is their own group; 0 and -1 are excluded above.
    // Errors are ignored because the process may already have exited.
    unsafe {
        let _ = libc::kill(-pid_i, libc::SIGKILL);
        let _ = libc::kill(pid_i, libc::SIGKILL);
    }
}

/// Process groups are Unix-only; elsewhere callers stop the direct child with `Child::kill`.
#[cfg(not(unix))]
fn kill_worker_group(_pid: u32) {}

/// Run each child in its own process group (so Cancel reaches its helpers), and on
/// Linux have the kernel kill it if the spawning thread dies first.
fn isolate_child(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the hook runs in the forked child before exec and only calls
        // async-signal-safe functions (`prctl`, `getppid`, `_exit`).
        let parent = std::process::id();
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // The parent may have exited between fork and prctl.
                if u32::try_from(libc::getppid()).ok() != Some(parent) {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
    }
}

struct WorkerResult {
    out_path: PathBuf,
    /// e.g. "2 paragraphs could not be spoken … see book.skipped.txt"
    warning: Option<String>,
}

/// Export options for the killable speech worker (voice fields live on `SpeechParams`).
struct WorkerExport<'a> {
    out: &'a Path,
    format: ExportFormat,
    bitrate: &'a str,
    sample_rate: u32,
    max_secs: Option<f32>,
    split_chapters: bool,
}

/// Run speech in a killable child (re-launch this GUI binary). Cancel → force-stop.
fn run_synth_worker(
    speech: &SpeechParams,
    export: WorkerExport<'_>,
) -> anyhow::Result<WorkerResult> {
    bail_if_cancelled(&speech.cancel)?;

    let text_path = write_worker_text(&speech.text)?;
    speech.active.add_file(&text_path);
    let result = run_synth_worker_with_text(speech, &export, &text_path);
    let _ = std::fs::remove_file(&text_path);
    speech.active.remove_file(&text_path);
    result
}

fn run_synth_worker_with_text(
    speech: &SpeechParams,
    export: &WorkerExport<'_>,
    text_path: &Path,
) -> anyhow::Result<WorkerResult> {
    let progress = &speech.progress;
    let cancel = &speech.cancel;
    let mut child = spawn_worker_command(speech, export, text_path)?;
    let pid = child.id();
    speech.active.add_pid(pid);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Watch Cancel on a side thread so we can force-stop mid-ONNX while stdout is blocked.
    let watcher_stop = Arc::new(AtomicBool::new(false));
    let watcher_cancel = {
        let stop = Arc::clone(&watcher_stop);
        let cancel = Arc::clone(cancel);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) {
                    kill_worker_group(pid);
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        })
    };

    // Collect error output in the background for better failure messages.
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_thread = Arc::clone(&stderr_buf);
    let stderr_thread = std::thread::spawn(move || {
        if let Some(err) = stderr {
            let mut s = String::new();
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&line);
            }
            if let Ok(mut g) = stderr_buf_thread.lock() {
                *g = s;
            }
        }
    });

    let (done_path, warning) =
        read_worker_stdout_progress(stdout, progress, export.max_secs.is_some());

    // Stop watcher; if cancel is already set, ensure kill once more.
    watcher_stop.store(true, Ordering::Release);
    if cancel.load(Ordering::Acquire) {
        kill_worker_group(pid);
    }
    let _ = watcher_cancel.join();

    let status = wait_worker_status(&mut child, cancel, pid);
    // Unregister only after the wait: until the child is reaped its pid cannot be reused.
    speech.active.remove_pid(pid);
    let status = status?;
    let _ = stderr_thread.join();

    if cancel.load(Ordering::Acquire) {
        let _ = std::fs::remove_file(export.out);
        if let Some(ref p) = done_path {
            let _ = std::fs::remove_file(p);
        }
        anyhow::bail!(CANCELLED);
    }

    if !status.success() {
        let _ = std::fs::remove_file(export.out);
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                anyhow::bail!("synth worker killed by signal {sig}");
            }
        }
        let err = stderr_buf
            .lock()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("synth worker failed ({status})"));
        anyhow::bail!("{err}");
    }

    let out_path = done_path.unwrap_or_else(|| export.out.to_path_buf());
    // A file, or a folder for one-file-per-chapter exports.
    if !out_path.exists() {
        anyhow::bail!(
            "synth worker finished but output missing: {}",
            out_path.display()
        );
    }
    Ok(WorkerResult { out_path, warning })
}

/// Build and spawn the killable synth-worker child process.
fn spawn_worker_command(
    speech: &SpeechParams,
    export: &WorkerExport<'_>,
    text_path: &Path,
) -> anyhow::Result<std::process::Child> {
    // Re-launch this same binary so the worker always matches the GUI version.
    let worker_bin = std::env::current_exe().map_err(|e| anyhow::anyhow!("current_exe: {e}"))?;
    let mut cmd = Command::new(&worker_bin);
    cmd.arg(SYNTH_WORKER_FLAG)
        .arg("--text-file")
        .arg(text_path)
        .arg("--out")
        .arg(export.out)
        .arg("--voice")
        .arg(&speech.voice)
        .arg("--speed")
        .arg(speech.speed.to_string())
        .arg("--pitch")
        .arg(speech.pitch.to_string())
        .arg("--paragraph-pause")
        .arg(speech.paragraph_pause.to_string())
        .arg("--loudness")
        .arg(speech.loudness.to_string())
        .arg("--bitrate")
        .arg(export.bitrate)
        .arg("--sample-rate")
        .arg(export.sample_rate.to_string())
        .arg("--format")
        .arg(export.format.extension());
    if export.split_chapters {
        cmd.arg("--split-chapters");
    }
    if let Some(style) = speech.style_index {
        cmd.arg("--style").arg(style.to_string());
    }
    if !speech.loudnorm {
        cmd.arg("--no-loudnorm");
    }
    if let Some(secs) = export.max_secs {
        cmd.arg("--max-secs").arg(secs.to_string());
    }

    isolate_child(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "failed to spawn synth worker ({}): {e}",
            worker_bin.display()
        )
    })
}

/// Read JSON progress / done lines from the worker stdout pipe.
fn read_worker_stdout_progress(
    stdout: Option<std::process::ChildStdout>,
    progress: &Arc<Mutex<JobProgress>>,
    preview_mode: bool,
) -> (Option<PathBuf>, Option<String>) {
    let mut done_path: Option<PathBuf> = None;
    let mut warning: Option<String> = None;
    let Some(out_pipe) = stdout else {
        return (done_path, warning);
    };
    let reader = BufReader::new(out_pipe);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            match v.get("type").and_then(|t| t.as_str()) {
                Some("progress") => {
                    apply_worker_progress_line(&v, progress, preview_mode);
                }
                Some("done") => {
                    if let Some(p) = v.get("path").and_then(|p| p.as_str()) {
                        done_path = Some(PathBuf::from(p));
                    }
                    warning = v
                        .get("warning")
                        .and_then(|w| w.as_str())
                        .map(str::to_string);
                }
                _ => {}
            }
        }
    }
    (done_path, warning)
}

/// Map one worker progress JSON object onto the shared job bar.
fn apply_worker_progress_line(
    v: &serde_json::Value,
    progress: &Arc<Mutex<JobProgress>>,
    preview_mode: bool,
) {
    let mut frac = v
        .get("fraction")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0) as f32;
    // Preview: speech-making fills the first part of the bar and playback the rest;
    // per-paragraph labels collapse into one "Synthesizing preview…" stage.
    let raw_label = v
        .get("label")
        .and_then(|l| l.as_str())
        .unwrap_or("Working…");
    let label = if preview_mode {
        frac *= PREVIEW_SYNTH_END;
        if raw_label.starts_with("Loading Kokoro") {
            raw_label
        } else if raw_label.starts_with("Encoding") {
            "Encoding preview…"
        } else {
            "Synthesizing preview…"
        }
    } else {
        raw_label
    };
    report(progress, frac, label);
}

/// Wait for the worker to exit (force-stopping it first if Cancel is set).
fn wait_worker_status(
    child: &mut std::process::Child,
    cancel: &AtomicBool,
    pid: u32,
) -> anyhow::Result<std::process::ExitStatus> {
    if cancel.load(Ordering::Acquire) {
        kill_worker_group(pid);
        let _ = child.kill();
    }
    child.wait().map_err(|e| {
        kill_worker_group(pid);
        let _ = child.kill();
        anyhow::anyhow!("waiting for synth worker: {e}")
    })
}

/// Returns the written path and an optional warning (skipped paragraphs).
pub(crate) async fn speak_job(job: SpeakJob) -> anyhow::Result<(PathBuf, Option<String>)> {
    tokio::task::spawn_blocking(move || {
        let progress = Arc::clone(&job.speech.progress);
        let cancel = Arc::clone(&job.speech.cancel);
        report(&progress, 0.02, "Starting synthesis worker…");
        bail_if_cancelled(&cancel)?;

        let result = run_synth_worker(
            &job.speech,
            WorkerExport {
                out: &job.out,
                format: job.format,
                bitrate: &job.bitrate,
                sample_rate: job.sample_rate,
                max_secs: None,
                split_chapters: job.split_chapters,
            },
        )?;

        if cancel.load(Ordering::Acquire) {
            let _ = std::fs::remove_file(&result.out_path);
            anyhow::bail!(CANCELLED);
        }

        report(&progress, 1.0, "Done");
        Ok::<_, anyhow::Error>((result.out_path, result.warning))
    })
    .await?
}

/// Optional timed progress while audio plays: move from `from` toward `to` over `duration_secs`.
struct PlayProgress<'a> {
    progress: &'a Arc<Mutex<JobProgress>>,
    from: f32,
    to: f32,
    duration_secs: f32,
    label: &'a str,
}

/// Play audio with hard cancel: kill the player process if Cancel is pressed.
///
/// Tries each installed player in turn until one exits successfully. When
/// `play_progress` is set, the shared progress bar advances from `from` toward `to`
/// over `duration_secs` (held just under `to` until playback ends).
fn play_wav_killable(
    path: &Path,
    cancel: &AtomicBool,
    active: &ActiveJob,
    play_progress: Option<PlayProgress<'_>>,
) -> anyhow::Result<()> {
    bail_if_cancelled(cancel)?;

    let candidates: &[(&str, &[&str])] = &[
        ("paplay", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpv", &["--no-video", "--really-quiet"]),
    ];

    let mut last_err: Option<String> = None;
    for (bin, args) in candidates {
        let mut cmd = Command::new(bin);
        cmd.args(*args).arg(path).stdin(Stdio::null());
        isolate_child(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                last_err = Some(format!("failed to run {bin}: {e}"));
                continue;
            }
        };
        let pid = child.id();
        active.add_pid(pid);
        let outcome = watch_player(&mut child, pid, cancel, play_progress.as_ref());
        active.remove_pid(pid);
        match outcome {
            Ok(true) => return Ok(()),
            Ok(false) => anyhow::bail!(CANCELLED),
            Err(e) => last_err = Some(format!("{bin}: {e}")),
        }
    }
    match last_err {
        Some(e) => anyhow::bail!("could not play preview ({e})"),
        None => anyhow::bail!("no audio player found (tried paplay, ffplay, mpv)"),
    }
}

/// Poll a running player: `Ok(true)` played, `Ok(false)` cancelled, `Err` failed.
fn watch_player(
    child: &mut std::process::Child,
    pid: u32,
    cancel: &AtomicBool,
    play_progress: Option<&PlayProgress<'_>>,
) -> Result<bool, String> {
    let started = Instant::now();
    if let Some(pp) = play_progress {
        report(pp.progress, pp.from, pp.label);
    }
    loop {
        if cancel.load(Ordering::Acquire) {
            kill_worker_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        if let Some(pp) = play_progress {
            let dur = pp.duration_secs.max(0.1);
            let t = (started.elapsed().as_secs_f32() / dur).clamp(0.0, 1.0);
            // Hold just shy of `to` until the player exits so only the final
            // "Done" shows 100%.
            let frac = if t >= 1.0 {
                (pp.to - 0.001).max(pp.from)
            } else {
                pp.from + (pp.to - pp.from) * t
            };
            report(pp.progress, frac, pp.label);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                if let Some(pp) = play_progress {
                    report(pp.progress, pp.to, pp.label);
                }
                return Ok(true);
            }
            Ok(Some(status)) => return Err(format!("exited with {status}")),
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                kill_worker_group(pid);
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait failed: {e}"));
            }
        }
    }
}

pub(crate) async fn preview_job(job: PreviewJob) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let progress = Arc::clone(&job.speech.progress);
        let cancel = Arc::clone(&job.speech.cancel);
        bail_if_cancelled(&cancel)?;

        // Sample voice (empty Source): cache hit skips speech-making; miss uses killable worker.
        // Document preview always uses the killable worker.
        let wav_path = if job.sample_voice {
            synthesize_sample_voice_killable(&job.speech, &progress, &cancel)?
        } else {
            report(&progress, 0.02, "Starting preview worker…");
            run_preview_worker(&job.speech)?
        };

        // Temporary document previews are registered for cleanup on window close.
        if !job.sample_voice {
            job.speech.active.add_file(&wav_path);
        }
        let cleanup = || {
            if !job.sample_voice {
                let _ = std::fs::remove_file(&wav_path);
                job.speech.active.remove_file(&wav_path);
            }
        };
        if cancel.load(Ordering::Acquire) {
            cleanup();
            anyhow::bail!(CANCELLED);
        }

        // Playback owns the rest of the bar for the full ~10s audio.
        let play_label = format!("Playing {PREVIEW_SECS:.0} second preview");
        report(&progress, PREVIEW_SYNTH_END, &play_label);

        let play_res = play_wav_killable(
            &wav_path,
            &cancel,
            &job.speech.active,
            Some(PlayProgress {
                progress: &progress,
                from: PREVIEW_SYNTH_END,
                to: 1.0,
                duration_secs: PREVIEW_SECS,
                label: &play_label,
            }),
        );
        // Cached sample WAVs stay on disk; temporary document previews are cleaned up.
        cleanup();
        play_res?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

/// ~10s preview speech via killable worker (Cancel → force-stop mid-ONNX).
fn run_preview_worker(speech: &SpeechParams) -> anyhow::Result<PathBuf> {
    let tmp = unique_temp("preview", "wav");
    speech.active.add_file(&tmp);
    let result = run_synth_worker(
        speech,
        WorkerExport {
            out: &tmp,
            format: ExportFormat::Wav,
            bitrate: "64k",
            sample_rate: KOKORO_SAMPLE_RATE,
            max_secs: Some(PREVIEW_SECS),
            split_chapters: false,
        },
    );
    speech.active.remove_file(&tmp);
    match result {
        Ok(r) => Ok(r.out_path),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Empty-Source voice preview: disk cache + killable worker on a miss.
fn synthesize_sample_voice_killable(
    speech: &SpeechParams,
    progress: &Arc<Mutex<JobProgress>>,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<PathBuf> {
    if let Some(cached) = sample_voice_cache_path(speech) {
        if cached.is_file() {
            // Refresh the time so pruning keeps recently used samples.
            let _ = std::fs::File::options()
                .write(true)
                .open(&cached)
                .and_then(|f| f.set_modified(SystemTime::now()));
            report(progress, PREVIEW_SYNTH_END, "Using cached sample");
            return Ok(cached);
        }
    }

    bail_if_cancelled(cancel)?;
    report(progress, 0.02, "Starting sample worker…");
    let path = run_preview_worker(speech)?;

    // Save for this voice + settings so the next sample Preview is instant.
    if let Some(cached) = sample_voice_cache_path(speech) {
        if let Some(parent) = cached.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&path, &cached).is_ok() {
            let _ = std::fs::remove_file(&path);
            if let Some(dir) = cached.parent() {
                prune_sample_cache(dir, SAMPLE_VOICE_CACHE_MAX);
            }
            report(progress, PREVIEW_SYNTH_END, "Sample ready");
            return Ok(cached);
        }
    }

    report(progress, PREVIEW_SYNTH_END, "Sample ready");
    Ok(path)
}

/// Delete all but the `keep` most recently modified `.wav` files in `dir`.
fn prune_sample_cache(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((modified, p))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, p) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(p);
    }
}

fn sample_voice_cache_path(speech: &SpeechParams) -> Option<PathBuf> {
    let root = directories::ProjectDirs::from("local", "document2voice", "document2voice")?
        .cache_dir()
        .join("voice-previews");
    let style = speech
        .style_index
        .map_or_else(|| "def".into(), |i| i.to_string());
    let voice = speech
        .voice
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let name = format!(
        "v{SAMPLE_VOICE_CACHE_VERSION}_{voice}_spd{:.3}_sty{style}_pit{:.3}_pp{:.2}_ln{}_lufs{:.2}.wav",
        speech.speed,
        speech.pitch,
        speech.paragraph_pause,
        i32::from(speech.loudnorm),
        speech.loudness,
    );
    Some(root.join(name))
}

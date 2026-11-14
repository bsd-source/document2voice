use crate::config::{ExportFormat, ExportOptions};
use crate::error::{CoreError, Result};
use crate::KOKORO_SAMPLE_RATE;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

const WAV_HEADER_LEN: u64 = 44;
const PARTIAL_VERSION: u32 = 2;

/// A 16-bit mono WAV that speech is streamed into while it is generated, so a long
/// book never has to sit in memory. It lives **next to the output file** (same
/// folder, hidden name) so the final move is a same-filesystem rename — `/tmp` is
/// often a RAM disk on a different filesystem.
///
/// Two kinds:
/// - [`StageWav::temporary`] — unique name, deleted when dropped (previews, clips).
/// - [`StageWav::resumable`] — fixed name plus a small JSON progress file. It is
///   kept if the job fails or is killed, and a later job with the same key picks
///   up after the last finished paragraph. Deleted after a successful export.
pub struct StageWav {
    path: PathBuf,
    file: Option<BufWriter<File>>,
    samples: u64,
    resume: Option<ResumeInfo>,
    /// Sample offsets where chapters 2, 3, … start (chapter 1 starts at 0).
    chapter_starts: Vec<u64>,
    /// Paragraphs (0-based) replaced by silence because synthesis failed.
    skipped: Vec<usize>,
    /// Remove the files on drop (temporary stage, or resumable stage after export).
    remove_on_drop: bool,
}

struct ResumeInfo {
    sidecar: PathBuf,
    key: String,
    total: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PartialState {
    version: u32,
    key: String,
    total: usize,
    done: usize,
    samples: u64,
    #[serde(default)]
    chapter_starts: Vec<u64>,
    #[serde(default)]
    skipped: Vec<usize>,
}

/// An unfinished, resumable export found next to an output path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialExport {
    /// Paragraphs already synthesized.
    pub done: usize,
    /// Paragraphs in that job.
    pub total: usize,
}

fn partial_paths(out: &Path) -> (PathBuf, PathBuf) {
    let dir = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = out
        .file_name()
        .map_or_else(|| "audio".into(), |n| n.to_string_lossy().into_owned());
    (
        dir.join(format!(".{name}.document2voice-partial.wav")),
        dir.join(format!(".{name}.document2voice-partial.json")),
    )
}

/// The unfinished export for `out`, if one is waiting to be resumed. (Whether it is
/// actually reused depends on the text and voice settings matching; see
/// [`StageWav::resumable`].)
pub fn partial_export(out: &Path) -> Option<PartialExport> {
    let (wav, json) = partial_paths(out);
    let st: PartialState = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
    let ok = st.version == PARTIAL_VERSION
        && st.done > 0
        && st.done < st.total
        && std::fs::metadata(wav).is_ok_and(|m| m.len() >= WAV_HEADER_LEN + st.samples * 2);
    ok.then_some(PartialExport {
        done: st.done,
        total: st.total,
    })
}

/// Delete the unfinished export for `out` (the "Start over" choice).
pub fn discard_partial_export(out: &Path) -> Result<()> {
    let (wav, json) = partial_paths(out);
    for p in [wav, json] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Folder that a one-file-per-chapter export of `out` writes into (`book.m4b` → `book/`).
pub fn split_output_dir(out: &Path) -> PathBuf {
    split_dir(out)
}

/// Remove numbered chapter files (`NN - Title.ext`) left by an earlier split export
/// into the same folder, so a re-export with fewer chapters leaves no stale files.
/// Only files matching that pattern and extension are touched.
fn remove_old_chapter_files(dir: &Path, ext: &str) -> Result<usize> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let mut n = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let numbered = name
            .split_once(" - ")
            .is_some_and(|(num, _)| !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()));
        let same_ext = Path::new(&name)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case(ext));
        if numbered && same_ext && e.path().is_file() {
            std::fs::remove_file(e.path())?;
            n += 1;
        }
    }
    Ok(n)
}

fn stage_dir_and_name(out: &Path) -> Result<(PathBuf, String)> {
    let dir = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    let name = out
        .file_name()
        .map_or_else(|| "audio".into(), |n| n.to_string_lossy().into_owned());
    Ok((dir, name))
}

/// Canonical 44-byte PCM WAV header (16-bit mono at Kokoro's rate).
fn wav_header(data_bytes: u64) -> [u8; 44] {
    // Past 4 GB the size fields saturate; ffmpeg reads the stage as raw PCM anyway.
    let data = u32::try_from(data_bytes).unwrap_or(u32::MAX);
    let riff = data.saturating_add(36);
    let rate = KOKORO_SAMPLE_RATE;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&riff.to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    h[32..34].copy_from_slice(&2u16.to_le_bytes()); // block align
    h[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

impl StageWav {
    /// Throwaway stage in the folder that will hold `out` (deleted on drop).
    pub fn temporary(out: &Path) -> Result<Self> {
        let (dir, name) = stage_dir_and_name(out)?;
        let path = dir.join(format!(
            ".{name}.document2voice-{}-{}.stage.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let mut f = File::create(&path)?;
        f.write_all(&wav_header(0))?;
        Ok(Self {
            path,
            file: Some(BufWriter::new(f)),
            samples: 0,
            resume: None,
            chapter_starts: Vec::new(),
            skipped: Vec::new(),
            remove_on_drop: true,
        })
    }

    /// Resumable stage for `out`. `key` identifies the job (text + speech settings);
    /// `total` is its paragraph count.
    ///
    /// Returns the stage and how many paragraphs are already in it (0 = fresh start).
    /// A leftover stage for a *different* key is discarded.
    pub fn resumable(out: &Path, key: &str, total: usize) -> Result<(Self, usize)> {
        stage_dir_and_name(out)?;
        let (path, sidecar) = partial_paths(out);

        let prior = std::fs::read_to_string(&sidecar)
            .ok()
            .and_then(|s| serde_json::from_str::<PartialState>(&s).ok())
            .filter(|st| {
                st.version == PARTIAL_VERSION
                    && st.key == key
                    && st.total == total
                    && st.done > 0
                    && st.done < total
            })
            .filter(|st| {
                // The WAV must hold at least the samples the progress file vouches for.
                std::fs::metadata(&path).is_ok_and(|m| m.len() >= WAV_HEADER_LEN + st.samples * 2)
            });

        let resume = Some(ResumeInfo {
            sidecar,
            key: key.to_string(),
            total,
        });
        if let Some(st) = prior {
            // Drop anything written after the last checkpoint (killed mid-paragraph).
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_len(WAV_HEADER_LEN + st.samples * 2)?;
            let mut f = BufWriter::new(f);
            f.seek(SeekFrom::End(0))?;
            info!(done = st.done, total, path = %path.display(), "resuming partial export");
            return Ok((
                Self {
                    path,
                    file: Some(f),
                    samples: st.samples,
                    resume,
                    chapter_starts: st
                        .chapter_starts
                        .into_iter()
                        .filter(|&c| c <= st.samples)
                        .collect(),
                    skipped: st.skipped.into_iter().filter(|&p| p < st.done).collect(),
                    remove_on_drop: false,
                },
                st.done,
            ));
        }

        let mut f = File::create(&path)?;
        f.write_all(&wav_header(0))?;
        let mut stage = Self {
            path,
            file: Some(BufWriter::new(f)),
            samples: 0,
            resume,
            chapter_starts: Vec::new(),
            skipped: Vec::new(),
            remove_on_drop: false,
        };
        stage.checkpoint(0)?;
        Ok((stage, 0))
    }

    /// Append float samples (clipped to −1..1) as 16-bit PCM.
    pub fn push(&mut self, samples: &[f32]) -> Result<()> {
        let f = self
            .file
            .as_mut()
            .ok_or_else(|| CoreError::Export("stage WAV already finished".into()))?;
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            buf.extend_from_slice(
                &((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes(),
            );
        }
        f.write_all(&buf)?;
        self.samples += samples.len() as u64;
        Ok(())
    }

    /// Record that `done` paragraphs (and their trailing pauses) are safely on disk.
    /// No-op for temporary stages.
    pub fn checkpoint(&mut self, done: usize) -> Result<()> {
        let Some(r) = &self.resume else {
            return Ok(());
        };
        if let Some(f) = self.file.as_mut() {
            f.flush()?;
            f.get_ref().sync_data()?;
        }
        let state = PartialState {
            version: PARTIAL_VERSION,
            key: r.key.clone(),
            total: r.total,
            done,
            samples: self.samples,
            chapter_starts: self.chapter_starts.clone(),
            skipped: self.skipped.clone(),
        };
        let json = serde_json::to_string(&state).map_err(|e| CoreError::Export(e.to_string()))?;
        let tmp = r.sidecar.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &r.sidecar)?;
        Ok(())
    }

    /// Note that a new chapter starts at the current position (saved with the next
    /// [`checkpoint`](Self::checkpoint), so it survives a resume).
    pub fn mark_chapter_start(&mut self) {
        if self.samples > 0 && self.chapter_starts.last() != Some(&self.samples) {
            self.chapter_starts.push(self.samples);
        }
    }

    /// Record that paragraph `index` was replaced by silence (saved with the next
    /// checkpoint, so a resumed job still reports it).
    pub fn note_skipped(&mut self, index: usize) {
        if !self.skipped.contains(&index) {
            self.skipped.push(index);
        }
    }

    /// Paragraphs replaced by silence so far (0-based, including earlier runs).
    pub fn skipped(&self) -> &[usize] {
        &self.skipped
    }

    /// Sample offsets of chapter starts recorded so far (chapter 1 = 0, not included).
    pub fn chapter_starts(&self) -> &[u64] {
        &self.chapter_starts
    }

    /// Number of samples written so far (including resumed ones).
    pub fn len(&self) -> u64 {
        self.samples
    }

    pub fn is_empty(&self) -> bool {
        self.samples == 0
    }

    /// Flush and write the final header sizes.
    fn finish(&mut self) -> Result<()> {
        if let Some(mut f) = self.file.take() {
            f.flush()?;
            let mut f = f.into_inner().map_err(|e| CoreError::Io(e.into_error()))?;
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&wav_header(self.samples * 2))?;
            f.sync_data()?;
        }
        Ok(())
    }

    fn data_bytes(&self) -> u64 {
        self.samples * 2
    }
}

impl Drop for StageWav {
    fn drop(&mut self) {
        self.file.take();
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
            if let Some(r) = &self.resume {
                let _ = std::fs::remove_file(&r.sidecar);
            }
        }
    }
}

/// A chapter of the finished audio: title and first sample (at Kokoro's 24 kHz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMark {
    pub title: String,
    pub start: u64,
}

/// Turn a finished stage WAV into the final file, with chapter markers.
///
/// Everything is mono because Kokoro only makes mono speech. Changing format, pitch,
/// loudness or sample rate needs ffmpeg; a plain WAV at Kokoro's rate is just moved
/// into place. `chapters` (sorted, first at 0) become chapter markers in Opus, MP3,
/// FLAC and M4B files, or — with [`ExportOptions::split_chapters`] — one file per
/// chapter in a folder named after `out`. Returns the file (or folder) written.
pub fn export_stage_with_chapters(
    mut stage: StageWav,
    out: impl AsRef<Path>,
    opts: &ExportOptions,
    chapters: &[ChapterMark],
) -> Result<PathBuf> {
    let out = out.as_ref();
    stage.finish()?;
    let total = stage.len();

    if opts.split_chapters && chapters.len() >= 2 {
        let dir = split_dir(out);
        std::fs::create_dir_all(&dir)?;
        let ffmpeg = resolve_ffmpeg()?;
        let width = chapters.len().to_string().len().max(2);
        // Encode every chapter to a temporary name first, so a failure leaves the
        // earlier export's chapter files untouched.
        let mut encoded: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(chapters.len());
        let result = chapters.iter().enumerate().try_for_each(|(i, ch)| {
            let end = chapters.get(i + 1).map_or(total, |c| c.start);
            let file = dir.join(format!(
                "{:0width$} - {}.{}",
                i + 1,
                file_safe(&ch.title),
                opts.format.extension()
            ));
            let tmp = encoding_path(&file, opts.format);
            encoded.push((tmp.clone(), file));
            let job = FfJob {
                trim: Some((ch.start, end)),
                meta: None,
                tags: vec![
                    ("title".into(), ch.title.clone()),
                    ("track".into(), format!("{}/{}", i + 1, chapters.len())),
                ],
            };
            run_ffmpeg_master(&ffmpeg, &stage.path, &tmp, opts, &job)
        });
        let result = result.and_then(|()| {
            let old = remove_old_chapter_files(&dir, opts.format.extension())?;
            if old > 0 {
                info!(removed = old, dir = %dir.display(), "replaced earlier chapter files");
            }
            encoded
                .iter()
                .try_for_each(|(tmp, file)| move_into_place(tmp, file))
        });
        if result.is_err() {
            for (tmp, _) in &encoded {
                let _ = std::fs::remove_file(tmp);
            }
        }
        result?;
        stage.remove_on_drop = true;
        info!(dir = %dir.display(), files = chapters.len(), "export complete (one file per chapter)");
        return Ok(dir);
    }

    let with_chapters = chapters.len() >= 2 && opts.format.supports_chapters();
    let needs_ffmpeg = with_chapters
        || opts.loudnorm
        || opts.pitch_semitones.abs() > 0.01
        || !matches!(opts.format, ExportFormat::Wav)
        || opts.sample_rate != KOKORO_SAMPLE_RATE;

    if needs_ffmpeg {
        let ffmpeg = resolve_ffmpeg()?;
        // Ogg/FLAC chapters are Vorbis comments (CHAPTER001=HH:MM:SS.mmm). ffmpeg
        // writes none for FLAC and rounds the seconds for Ogg (6.925 s → 00:00:07.925),
        // so write them ourselves; M4B/MP3 take ffmpeg's real chapter atoms/frames.
        let vorbis =
            with_chapters && matches!(opts.format, ExportFormat::Opus | ExportFormat::Flac);
        let meta = if with_chapters && !vorbis {
            let p = stage.path.with_extension("chapters.txt");
            std::fs::write(&p, ffmetadata(chapters, total))?;
            Some(p)
        } else {
            None
        };
        let job = FfJob {
            trim: None,
            meta: meta.clone(),
            tags: if vorbis {
                vorbis_chapter_tags(chapters)
            } else {
                Vec::new()
            },
        };
        // Encode beside `out` and rename on success, so a failed or killed run
        // never destroys an earlier export.
        let tmp = encoding_path(out, opts.format);
        let r = run_ffmpeg_master(&ffmpeg, &stage.path, &tmp, opts, &job)
            .and_then(|()| move_into_place(&tmp, out));
        if let Some(p) = meta {
            let _ = std::fs::remove_file(p);
        }
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r?;
    } else {
        if stage.data_bytes() > u64::from(u32::MAX) - 36 {
            return Err(CoreError::Export(
                "audio is longer than a plain WAV can hold (~24 h); export as FLAC or Opus instead"
                    .into(),
            ));
        }
        move_into_place(&stage.path, out)?;
    }
    // Success: the stage (and any resume progress file) is no longer needed.
    stage.remove_on_drop = true;

    info!(
        path = %out.display(),
        format = opts.format.extension(),
        chapters = if with_chapters { chapters.len() } else { 0 },
        "export complete"
    );
    Ok(out.to_path_buf())
}

/// `book.m4b` → folder `book/` (next to it) for per-chapter files.
fn split_dir(out: &Path) -> PathBuf {
    if out.extension().is_some() {
        out.with_extension("")
    } else {
        out.to_path_buf()
    }
}

/// Make a chapter title safe as a file name (no slashes, control chars; length capped).
fn file_safe(title: &str) -> String {
    let t: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.chars().take(80).collect();
    let t = t.trim_end_matches(['.', ' ']).to_string();
    if t.is_empty() {
        "Chapter".into()
    } else {
        t
    }
}

/// ffmpeg FFMETADATA1 chapter list (timebase = Kokoro samples).
fn ffmetadata(chapters: &[ChapterMark], total: u64) -> String {
    fn esc(s: &str) -> String {
        let mut o = String::with_capacity(s.len());
        for c in s.chars() {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(c);
        }
        o
    }
    let mut m = String::from(";FFMETADATA1\n");
    for (i, ch) in chapters.iter().enumerate() {
        let end = chapters.get(i + 1).map_or(total, |c| c.start).max(ch.start);
        m.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/{KOKORO_SAMPLE_RATE}\nSTART={}\nEND={}\ntitle={}\n",
            ch.start,
            end,
            esc(&ch.title)
        ));
    }
    m
}

/// Vorbis-comment chapter tags (the de-facto Ogg/FLAC chapter convention).
fn vorbis_chapter_tags(chapters: &[ChapterMark]) -> Vec<(String, String)> {
    let rate = u64::from(KOKORO_SAMPLE_RATE);
    let mut tags = Vec::with_capacity(chapters.len() * 2);
    for (i, ch) in chapters.iter().enumerate() {
        let ms = ch.start * 1000 / rate; // truncate, never round up
        let stamp = format!(
            "{:02}:{:02}:{:02}.{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        );
        tags.push((format!("CHAPTER{:03}", i + 1), stamp));
        tags.push((format!("CHAPTER{:03}NAME", i + 1), ch.title.clone()));
    }
    tags
}

/// Extra per-run ffmpeg inputs: trim range (samples), chapter metadata file, tags.
struct FfJob {
    trim: Option<(u64, u64)>,
    meta: Option<PathBuf>,
    tags: Vec<(String, String)>,
}

/// Hidden name beside `out` that ffmpeg encodes into before the result is renamed
/// to `out` (`book.m4b` → `.book.m4b.document2voice-encoding.m4b`). It keeps an audio
/// extension so ffmpeg still picks the muxer from it.
fn encoding_path(out: &Path, format: ExportFormat) -> PathBuf {
    let name = out
        .file_name()
        .map_or_else(|| "audio".into(), |n| n.to_string_lossy().into_owned());
    let ext = out.extension().map_or_else(
        || format.extension().to_string(),
        |e| e.to_string_lossy().into_owned(),
    );
    out.with_file_name(format!(".{name}.document2voice-encoding.{ext}"))
}

/// Rename `from` → `to` (replacing `to`); across filesystems, copy then delete.
fn move_into_place(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            std::fs::copy(from, to)?;
            let _ = std::fs::remove_file(from);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn push_mono_rate(cmd: &mut Command, sample_rate: u32) {
    cmd.arg("-ar")
        .arg(sample_rate.to_string())
        .arg("-ac")
        .arg("1");
}

fn resolve_ffmpeg() -> Result<PathBuf> {
    crate::bundle::resolve_ffmpeg_bin()
        .or_else(|| which::which("ffmpeg").ok())
        .ok_or(CoreError::FfmpegMissing)
}

/// Build the ffmpeg audio filter string: optional pitch change, then optional loudness leveling (always mono).
fn build_audio_filters(opts: &ExportOptions) -> String {
    let mut af_parts: Vec<String> = Vec::new();

    // Pitch: play faster/slower (shifts pitch *and* tempo), resample back, then undo
    // the tempo change with atempo so only the pitch moves. Negative semitones = deeper.
    // (No extra rubberband library needed.)
    if opts.pitch_semitones.abs() > 0.01 {
        let factor = 2f64.powf(f64::from(opts.pitch_semitones) / 12.0);
        let new_rate = (f64::from(KOKORO_SAMPLE_RATE) * factor).round() as i64;
        af_parts.push(format!(
            "asetrate={new_rate},aresample={KOKORO_SAMPLE_RATE}"
        ));
        af_parts.extend(atempo_chain(1.0 / factor));
    }

    af_parts.push("aformat=channel_layouts=mono".into());

    if opts.loudnorm {
        let i = opts.loudnorm_i.clamp(-30.0, -5.0);
        af_parts.push(format!("loudnorm=I={i}:LRA=5:TP=-2"));
    }

    af_parts.join(",")
}

/// `atempo` accepts 0.5–2.0 per instance; chain instances for larger changes.
fn atempo_chain(mut tempo: f64) -> Vec<String> {
    let mut parts = Vec::new();
    while tempo > 2.0 {
        parts.push("atempo=2.0".to_string());
        tempo /= 2.0;
    }
    while tempo < 0.5 {
        parts.push("atempo=0.5".to_string());
        tempo /= 0.5;
    }
    parts.push(format!("atempo={tempo:.6}"));
    parts
}

fn run_ffmpeg_master(
    ffmpeg: &Path,
    input_wav: &Path,
    out: &Path,
    opts: &ExportOptions,
    job: &FfJob,
) -> Result<()> {
    let mut af = build_audio_filters(opts);
    if let Some((start, end)) = job.trim {
        // Sample-exact cut of one chapter, before pitch/loudness.
        af = format!("atrim=start_sample={start}:end_sample={end},asetpts=PTS-STARTPTS,{af}");
    }

    let mut cmd = Command::new(ffmpeg);
    // Read the stage as raw PCM after its header, so the WAV size fields never matter
    // (they saturate past 4 GB).
    cmd.arg("-y")
        .arg("-f")
        .arg("s16le")
        .arg("-ar")
        .arg(KOKORO_SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg("1")
        .arg("-skip_initial_bytes")
        .arg(WAV_HEADER_LEN.to_string())
        .arg("-i")
        .arg(input_wav);
    if let Some(meta) = &job.meta {
        cmd.arg("-f").arg("ffmetadata").arg("-i").arg(meta);
        cmd.arg("-map")
            .arg("0:a")
            .arg("-map_metadata")
            .arg("1")
            .arg("-map_chapters")
            .arg("1");
    }
    if job.meta.is_none() {
        cmd.arg("-map_chapters").arg("-1");
    }
    for (k, v) in &job.tags {
        cmd.arg("-metadata").arg(format!("{k}={v}"));
    }

    if !af.is_empty() {
        cmd.arg("-af").arg(&af);
    }

    // Always one channel (`-ac 1`).
    match opts.format {
        ExportFormat::Wav => {
            push_mono_rate(&mut cmd, opts.sample_rate);
            // RF64 header only when the file outgrows plain WAV's 4 GB limit.
            cmd.arg("-c:a")
                .arg("pcm_s16le")
                .arg("-rf64")
                .arg("auto")
                .arg(out);
        }
        ExportFormat::Flac => {
            push_mono_rate(&mut cmd, opts.sample_rate);
            cmd.arg("-c:a").arg("flac").arg(out);
        }
        ExportFormat::Mp3 => {
            // The MP3 encoder is picky about 48 kHz; use 44.1 kHz instead when asked for 48.
            let rate = if opts.sample_rate == 48_000 {
                44_100
            } else {
                opts.sample_rate
            };
            push_mono_rate(&mut cmd, rate);
            cmd.arg("-c:a")
                .arg("libmp3lame")
                .arg("-b:a")
                .arg(&opts.opus_bitrate)
                // ID3v2.3 (with CHAP frames for chapters) is what most players read.
                .arg("-id3v2_version")
                .arg("3")
                .arg(out);
        }
        ExportFormat::M4b => {
            push_mono_rate(&mut cmd, opts.sample_rate);
            cmd.arg("-c:a")
                .arg("aac")
                .arg("-b:a")
                .arg(&opts.opus_bitrate)
                .arg("-movflags")
                .arg("+faststart")
                // .m4b is not always mapped to the MP4 muxer; force it.
                .arg("-f")
                .arg("ipod")
                .arg(out);
        }
        ExportFormat::Opus => {
            push_mono_rate(&mut cmd, opts.sample_rate);
            cmd.arg("-c:a")
                .arg("libopus")
                .arg("-b:a")
                .arg(&opts.opus_bitrate)
                .arg("-vbr")
                .arg("on")
                // `audio` keeps full bandwidth; `voip` is tuned for phone-call speech.
                .arg("-application")
                .arg("audio")
                .arg(out);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| CoreError::Export(format!("failed to spawn ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoreError::Export(format!(
            "ffmpeg failed ({}):\n{stderr}",
            output.status
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turn a finished stage WAV into the final file (no chapters).
    fn export_stage(stage: StageWav, out: &Path, opts: &ExportOptions) -> Result<PathBuf> {
        export_stage_with_chapters(stage, out, opts, &[])
    }

    /// Export in-memory samples through a temporary stage.
    fn export_audio(samples: &[f32], out: &Path, opts: &ExportOptions) -> Result<PathBuf> {
        let mut stage = StageWav::temporary(out)?;
        stage.push(samples)?;
        export_stage(stage, out, opts)
    }

    #[test]
    fn pitch_filter_restores_tempo() {
        let opts = ExportOptions {
            pitch_semitones: 3.0,
            loudnorm: false,
            ..ExportOptions::default()
        };
        let af = build_audio_filters(&opts);
        assert!(
            af.starts_with("asetrate=28541,aresample=24000,atempo=0.8408"),
            "{af}"
        );
    }

    #[test]
    fn atempo_chain_splits_out_of_range() {
        assert_eq!(atempo_chain(4.0), ["atempo=2.0", "atempo=2.000000"]);
        assert_eq!(atempo_chain(0.25), ["atempo=0.5", "atempo=0.500000"]);
    }

    #[test]
    fn plain_wav_export_lands_in_target_dir() {
        let dir = std::env::temp_dir().join(format!("d2a-export-test-{}", std::process::id()));
        let out = dir.join("x.wav");
        let opts = ExportOptions {
            format: ExportFormat::Wav,
            loudnorm: false,
            ..ExportOptions::default()
        };
        export_audio(&[0.0, 0.5, -0.5], &out, &opts).unwrap();
        assert!(out.is_file());
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1, "stage file left behind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("d2a-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resumable_stage_truncates_to_last_checkpoint() {
        let dir = scratch("resume");
        let out = dir.join("book.wav");
        {
            let (mut st, start) = StageWav::resumable(&out, "k1", 3).unwrap();
            assert_eq!(start, 0);
            st.push(&[0.1; 100]).unwrap();
            st.checkpoint(1).unwrap();
            // Paragraph 2 half-written, then the process "dies" (drop, no checkpoint).
            st.push(&[0.2; 50]).unwrap();
        }
        let (st, start) = StageWav::resumable(&out, "k1", 3).unwrap();
        assert_eq!(start, 1);
        assert_eq!(st.len(), 100);
        drop(st);
        // A different job for the same output starts fresh.
        let (st, start) = StageWav::resumable(&out, "other", 3).unwrap();
        assert_eq!((start, st.len()), (0, 0));
        drop(st);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resumable_stage_is_removed_after_export() {
        let dir = scratch("resume-done");
        let out = dir.join("book.wav");
        let (mut st, _) = StageWav::resumable(&out, "k", 2).unwrap();
        st.push(&[0.5, -0.5, 0.25]).unwrap();
        st.checkpoint(1).unwrap();
        let opts = ExportOptions {
            format: ExportFormat::Wav,
            loudnorm: false,
            ..ExportOptions::default()
        };
        export_stage(st, &out, &opts).unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["book.wav"]);
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(&bytes[40..44], &6u32.to_le_bytes());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn vorbis_chapter_times_truncate() {
        let ch = [
            ChapterMark {
                title: "A".into(),
                start: 0,
            },
            ChapterMark {
                title: "B".into(),
                start: 166_200,
            }, // 6.925 s
            ChapterMark {
                title: "C".into(),
                start: 24_000 * 3_725,
            }, // 1:02:05
        ];
        let t = vorbis_chapter_tags(&ch);
        assert_eq!(t[2], ("CHAPTER002".into(), "00:00:06.925".into()));
        assert_eq!(t[4], ("CHAPTER003".into(), "01:02:05.000".into()));
        assert_eq!(t[5].1, "C");
    }

    #[test]
    fn ffmetadata_escapes_titles() {
        let ch = [
            ChapterMark {
                title: "One = #1; \\ ok".into(),
                start: 0,
            },
            ChapterMark {
                title: "Two".into(),
                start: 48_000,
            },
        ];
        let m = ffmetadata(&ch, 72_000);
        assert!(m.contains("title=One \\= \\#1\\; \\\\ ok\n"), "{m}");
        assert!(m.contains("START=48000\nEND=72000\n"), "{m}");
    }

    #[test]
    fn partial_export_seen_and_discarded() {
        let dir = scratch("partial-info");
        let out = dir.join("book.opus");
        assert_eq!(partial_export(&out), None);
        {
            let (mut st, _) = StageWav::resumable(&out, "k", 10).unwrap();
            st.push(&[0.1; 20]).unwrap();
            st.checkpoint(4).unwrap();
        }
        assert_eq!(
            partial_export(&out),
            Some(PartialExport { done: 4, total: 10 })
        );
        discard_partial_export(&out).unwrap();
        assert_eq!(partial_export(&out), None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn encoding_path_is_hidden_and_keeps_extension() {
        assert_eq!(
            encoding_path(Path::new("/x/book.m4b"), ExportFormat::M4b),
            Path::new("/x/.book.m4b.document2voice-encoding.m4b")
        );
        assert_eq!(
            encoding_path(Path::new("/x/book"), ExportFormat::Opus),
            Path::new("/x/.book.document2voice-encoding.opus")
        );
    }

    #[test]
    fn move_into_place_replaces_existing_file() {
        let dir = scratch("move-into-place");
        let (from, to) = (dir.join("new"), dir.join("out.opus"));
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"old").unwrap();
        move_into_place(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_numbered_chapter_files_are_replaced() {
        let dir = scratch("old-chapters");
        for f in [
            "01 - One.opus",
            "12 - Twelve.opus",
            "notes.opus",
            "01 - One.mp3",
            "cover.jpg",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert_eq!(remove_old_chapter_files(&dir, "opus").unwrap(), 2);
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["01 - One.mp3", "cover.jpg", "notes.opus"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

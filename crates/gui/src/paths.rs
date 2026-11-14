//! Clean up and check Source / Output paths (readable file, writable folder, and so on).
//!
//! Pure string helpers have unit tests; real disk checks can fail and fail gently.

use document2voice_core::DocKind;
use std::path::{Path, PathBuf};

/// Remembers the last path check so the Log does not repeat the same line.
///
/// Stops re-logging the same Source/Output status every time you leave the field if nothing changed.
#[derive(Debug, Default, Clone)]
pub(crate) struct PathLog {
    /// Last path and error (if any). `None` error means the last check was OK.
    last: Option<(String, Option<String>)>,
}

impl PathLog {
    pub(crate) fn clear(&mut self) {
        self.last = None;
    }

    /// True when this path + error match the last logged snapshot (skip re-log).
    pub(crate) fn unchanged(&self, path: &str, err: Option<&str>) -> bool {
        self.last
            .as_ref()
            .is_some_and(|(p, e)| p == path && e.as_deref() == err)
    }

    /// True if the field currently shows an error, or the last log was an error.
    /// Used to emit a recovery “path OK” line after fixing a bad path.
    pub(crate) fn had_error(&self, current_error: bool) -> bool {
        current_error || self.last.as_ref().is_some_and(|(_, e)| e.is_some())
    }

    pub(crate) fn set(&mut self, path: String, err: Option<String>) {
        self.last = Some((path, err));
    }
}

/// Clean and make absolute a Source path (no default filename — must be a real file).
pub(crate) fn normalize_source_path(path: &str) -> String {
    let raw = path.trim();
    if raw.is_empty() {
        return String::new();
    }
    clean_path_string(&absolutize_path(raw))
}

/// Why the Source field is red.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceError {
    /// Found by [`validate_source_path`] (missing, unreadable, folder, unsupported type).
    Path(String),
    /// Open / extract failed on a file whose path looked fine (DRM, empty text, …).
    Load(String),
}

impl SourceError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Path(m) | Self::Load(m) => m,
        }
    }

    pub(crate) fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }
}

/// Source must exist, be a readable file, and use a supported document type.
pub(crate) fn validate_source_path(path: &str) -> Result<(), SourceError> {
    let show = path.trim();
    if show.is_empty() {
        return Ok(());
    }
    let p = Path::new(show);
    let fail = |what: String| Err(SourceError::Path(format!("{what} · {show}")));

    match std::fs::metadata(p) {
        Err(e) if is_permission_io(&e) => return fail("permission denied".into()),
        Err(_) => return fail("file not found".into()),
        Ok(meta) if meta.is_dir() => return fail("is a folder".into()),
        Ok(meta) if !meta.is_file() => return fail("not a file".into()),
        Ok(_) => {}
    }

    if let Err(e) = std::fs::File::open(p) {
        return fail(if is_permission_io(&e) {
            "permission denied".into()
        } else {
            format!("cannot read ({e})")
        });
    }

    DocKind::from_path(p).map_err(|e| SourceError::Path(e.to_string()))?;
    Ok(())
}

pub(crate) fn is_permission_io(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    )
}

/// Soft Output warning when a user-set path already names an existing file.
pub(crate) const OUT_EXISTS_MSG: &str = "file already exists";
/// Soft Output warning after the user declines the replace dialog.
pub(crate) const OUT_EXISTS_NOT_OVERWRITTEN_MSG: &str = "file exists · not overwritten";

/// True when the Output error is only an “exists” heads-up (Start may still open replace UI).
pub(crate) fn is_soft_output_exists_error(msg: &str) -> bool {
    msg == OUT_EXISTS_MSG || msg == OUT_EXISTS_NOT_OVERWRITTEN_MSG
}

/// Check that `path` can be used as a place to save the audio file (folder exists and is writable).
///
/// Error strings always end with the **full file path** (folder + name + extension),
/// even when only the parent folder is the problem.
///
/// An existing writable file is **not** an error here — callers decide soft “exists”
/// warnings vs overwrite UI.
pub(crate) fn validate_output_path(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(());
    }
    let p = Path::new(path);
    // Always show the intended file path (name + extension) in messages.
    let show = path;

    if p.is_dir() {
        return Err(format!("is a folder · {show}"));
    }

    // Folder that will receive the file (or current folder if only a bare filename).
    let parent = match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    if !parent.exists() {
        return Err(format!("folder not found · {show}"));
    }
    if !parent.is_dir() {
        return Err(format!("parent is not a folder · {show}"));
    }

    // Existing file: must be openable for overwrite.
    if p.exists() {
        if !p.is_file() {
            return Err(format!("not a file · {show}"));
        }
        return match std::fs::OpenOptions::new().write(true).open(p) {
            Ok(_) => Ok(()),
            Err(e) if is_permission_io(&e) => Err(format!("permission denied · {show}")),
            Err(e) => Err(format!("cannot write ({e}) · {show}")),
        };
    }

    // New file: prove the parent folder is writable with a short-lived test file.
    match write_probe(parent) {
        Ok(()) => Ok(()),
        Err(e) if is_permission_io(&e) => Err(format!("permission denied · {show}")),
        Err(e) => Err(format!("cannot write ({e}) · {show}")),
    }
}

/// Create and delete a tiny test file to prove `dir` is writable.
///
/// Uses a unique name so two app instances cannot race on the same probe path.
pub(crate) fn write_probe(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(format!(
        ".document2voice-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&probe);
            Err(e)
        }
    }
}

/// Collapse `//`, `///`, etc., resolve `.` / `..` segments, unify separators to `/`.
///
/// Absolute paths keep a single leading `/` (`..` cannot climb above it); relative
/// paths keep leading `..` segments that have nothing left to cancel.
pub(crate) fn clean_path_string(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    if path.is_empty() {
        return String::new();
    }
    let absolute = path.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => match stack.last() {
                Some(&last) if last != ".." => {
                    stack.pop();
                }
                _ if absolute => {}
                _ => stack.push(".."),
            },
            _ => stack.push(part),
        }
    }
    let joined = stack.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Folder used to resolve relative Output paths when the user leaves Output empty
/// and presses Start (or types a relative path).
///
/// Order (an unwritable folder is reported, not replaced):
/// 1. Process current directory when it is not `$HOME` (e.g. `/bin`, project dir)
/// 2. Folder containing the AppImage (`$APPIMAGE` parent)
/// 3. AppImage original working directory (`$OWD`) when not `$HOME`
/// 4. Current directory (including `$HOME`)
///
/// Cached for the process lifetime.
pub(crate) fn output_path_base_dir() -> PathBuf {
    use std::sync::OnceLock;
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(compute_output_path_base_dir).clone()
}

fn compute_output_path_base_dir() -> PathBuf {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    let is_home = |p: &Path| home.as_ref().is_some_and(|h| h == p);

    // 1. cwd when the user (or launcher) put us somewhere specific — including /bin, /etc.
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.is_dir() && !is_home(&cwd) {
            return cwd;
        }
    }

    // 2. Where the .AppImage file lives (double-click often has cwd=$HOME).
    if let Some(dir) = appimage_install_dir() {
        if dir.is_dir() {
            return dir;
        }
    }

    // 3. Original working directory from the AppImage runtime.
    if let Ok(owd) = std::env::var("OWD") {
        let owd = PathBuf::from(owd);
        if owd.is_dir() && !is_home(&owd) {
            return owd;
        }
    }

    // 4. cwd as-is (may be $HOME).
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Directory of the running `.AppImage` file, if this process is inside one.
fn appimage_install_dir() -> Option<PathBuf> {
    let appimage = std::env::var_os("APPIMAGE")?;
    let p = PathBuf::from(appimage);
    p.parent()
        .map(Path::to_path_buf)
        .filter(|d| !d.as_os_str().is_empty())
}

/// Prefer a full absolute path so log/error lines always show a complete location.
pub(crate) fn absolutize_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return String::new();
    }
    if path.starts_with(['/', '\\']) {
        return path.to_string();
    }
    output_path_base_dir().join(path).display().to_string()
}

/// Launch-folder default: `{base}/output.{ext}`.
///
/// Used when Start is pressed with an empty Output field — not at app startup.
/// Base is [`output_path_base_dir`] (cwd / AppImage location).
pub(crate) fn default_output_path(ext: &str) -> String {
    output_path_base_dir()
        .join(format!("output.{ext}"))
        .display()
        .to_string()
}

/// If `path` already exists, return the next free numbered sibling.
///
/// Numbering is always off the **base** name (trailing `-N` stripped):
/// - `output.opus` taken → `output-1.opus`
/// - `output-1.opus` taken (and `output.opus` too) → `output-2.opus`
///   (not `output-1-1.opus`)
///
/// Returns `None` when the path is free (caller keeps the original).
pub(crate) fn next_available_output_path(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("output");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("opus");
    // `output-1` → base `output`, so the sequence is output, output-1, output-2, …
    let base = strip_trailing_dash_number(stem);
    for n in 0u32..10_000 {
        let name = if n == 0 {
            format!("{base}.{ext}")
        } else {
            format!("{base}-{n}.{ext}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// `output-12` → `output`; `my-book` unchanged (suffix not all digits).
fn strip_trailing_dash_number(stem: &str) -> &str {
    if let Some((left, right)) = stem.rsplit_once('-') {
        if !left.is_empty() && !right.is_empty() && right.chars().all(|c| c.is_ascii_digit()) {
            return left;
        }
    }
    stem
}

/// Extensions the app writes; any other suffix is part of the file name.
const AUDIO_EXTS: &[&str] = &["opus", "mp3", "m4b", "wav", "flac"];

/// Build a complete output file path for the current audio format.
///
/// - Empty stays empty.
/// - Cleans `//` / `///` and turns relative paths into absolute.
/// - Folder / trailing slash / missing file name → `…/output.{ext}`.
/// - Otherwise the name gets `.{ext}` (see [`ensure_output_extension`]).
pub(crate) fn normalize_output_path(path: &str, ext: &str) -> String {
    let raw = path.trim();
    if raw.is_empty() {
        return String::new();
    }
    // `dir/`, `.`, `..` and `/` all name a folder rather than a file.
    let names_folder = raw.ends_with(['/', '\\']) || Path::new(raw).file_name().is_none();
    let abs = clean_path_string(&absolutize_path(raw));
    let p = PathBuf::from(&abs);
    if names_folder || p.is_dir() {
        return p.join(format!("output.{ext}")).display().to_string();
    }
    ensure_output_extension(&abs, ext)
}

/// True when the user did not give a real base name (only an extension / dots).
/// e.g. `.opus`, `.mp3`, `..opus` → use default `output.{ext}` instead of `.opus.opus`.
pub(crate) fn is_default_output_name(file_name: &str, ext: &str) -> bool {
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return true;
    }
    if let Some(rest) = file_name.strip_prefix('.') {
        // Bare extension (".opus") or a doubled one (".opus.opus").
        if rest.eq_ignore_ascii_case(ext) || rest.eq_ignore_ascii_case(&format!("{ext}.{ext}")) {
            return true;
        }
    }
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    stem.trim_start_matches('.').is_empty()
}

/// Give `path` the extension `.{ext}`.
///
/// A known audio extension is replaced (`book.mp3` + Opus → `book.opus`); any other
/// suffix stays part of the name (`My Book Vol. 2` → `My Book Vol. 2.opus`).
pub(crate) fn ensure_output_extension(path: &str, ext: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return path.to_string();
    }
    let p = PathBuf::from(path);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if is_default_output_name(name, ext) {
        let parent = p.parent().filter(|par| !par.as_os_str().is_empty());
        return match parent {
            Some(parent) => parent.join(format!("output.{ext}")).display().to_string(),
            None => format!("output.{ext}"),
        };
    }
    let current = p.extension().and_then(|e| e.to_str());
    match current {
        Some(e) if e.eq_ignore_ascii_case(ext) => p.display().to_string(),
        Some(e) if AUDIO_EXTS.iter().any(|a| e.eq_ignore_ascii_case(a)) => {
            p.with_extension(ext).display().to_string()
        }
        _ => format!("{}.{ext}", p.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_collapses_separators_and_dots() {
        assert_eq!(clean_path_string("//a///b/./c/"), "/a/b/c");
        assert_eq!(clean_path_string("/a/b/../c"), "/a/c");
        assert_eq!(clean_path_string("/.."), "/");
        assert_eq!(clean_path_string("/"), "/");
        assert_eq!(clean_path_string("a\\b"), "a/b");
    }

    #[test]
    fn clean_keeps_leading_parent_segments_on_relative_paths() {
        assert_eq!(clean_path_string("../book.pdf"), "../book.pdf");
        assert_eq!(clean_path_string("../../x/../y"), "../../y");
        assert_eq!(clean_path_string("a/../../b"), "../b");
        assert_eq!(clean_path_string("./a"), "a");
    }

    #[test]
    fn relative_source_resolves_against_base_dir() {
        let base = output_path_base_dir();
        let parent = base.parent().unwrap_or(&base).display().to_string();
        let expected = clean_path_string(&format!("{parent}/book.pdf"));
        assert_eq!(normalize_source_path("../book.pdf"), expected);
        assert_eq!(normalize_source_path("   "), "");
    }

    #[test]
    fn extension_replaces_only_audio_suffixes() {
        assert_eq!(
            ensure_output_extension("/d/book.mp3", "opus"),
            "/d/book.opus"
        );
        assert_eq!(
            ensure_output_extension("/d/book.OPUS", "opus"),
            "/d/book.OPUS"
        );
        assert_eq!(
            ensure_output_extension("/d/My Book Vol. 2", "opus"),
            "/d/My Book Vol. 2.opus"
        );
        assert_eq!(ensure_output_extension("/d/book", "flac"), "/d/book.flac");
        assert_eq!(
            ensure_output_extension("/d/.opus", "opus"),
            "/d/output.opus"
        );
    }

    #[test]
    fn default_output_names() {
        assert!(is_default_output_name(".opus", "opus"));
        assert!(is_default_output_name(".opus.opus", "opus"));
        assert!(is_default_output_name("..", "opus"));
        assert!(!is_default_output_name("book.opus", "opus"));
        assert!(!is_default_output_name(".hidden", "opus"));
    }

    #[test]
    fn output_folder_inputs_get_default_name() {
        assert_eq!(normalize_output_path("/", "mp3"), "/output.mp3");
        assert_eq!(normalize_output_path("/tmp/x/", "mp3"), "/tmp/x/output.mp3");
        assert_eq!(normalize_output_path("", "mp3"), "");
    }

    #[test]
    fn trailing_number_is_stripped_for_renumbering() {
        assert_eq!(strip_trailing_dash_number("output-12"), "output");
        assert_eq!(strip_trailing_dash_number("my-book"), "my-book");
        assert_eq!(strip_trailing_dash_number("-3"), "-3");
    }

    #[test]
    fn path_log_dedupes() {
        let mut log = PathLog::default();
        assert!(!log.unchanged("/a", None));
        log.set("/a".into(), Some("bad".into()));
        assert!(log.unchanged("/a", Some("bad")));
        assert!(log.had_error(false));
        log.clear();
        assert!(!log.had_error(false));
    }
}

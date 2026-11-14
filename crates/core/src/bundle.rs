//! Locate bundled tools and model files: `DOCUMENT2VOICE_*` environment variables first,
//! then paths under the AppImage's `APPDIR`; callers fall back to `PATH`.

use std::path::{Path, PathBuf};

/// Folder where the AppImage is mounted (`APPDIR`), if the app is running that way.
pub(crate) fn appdir() -> Option<PathBuf> {
    std::env::var_os("APPDIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn existing_file(p: impl AsRef<Path>) -> Option<PathBuf> {
    let p = p.as_ref();
    p.is_file().then(|| p.to_path_buf())
}

fn existing_dir(p: impl AsRef<Path>) -> Option<PathBuf> {
    let p = p.as_ref();
    p.is_dir().then(|| p.to_path_buf())
}

/// First path from environment variables or under `APPDIR` that is an existing file.
fn resolve_bundled_file(env_keys: &[&str], rel_paths: &[&str]) -> Option<PathBuf> {
    for key in env_keys {
        if let Ok(p) = std::env::var(key) {
            if let Some(f) = existing_file(p) {
                return Some(f);
            }
        }
    }
    let root = appdir()?;
    for rel in rel_paths {
        if let Some(f) = existing_file(root.join(rel)) {
            return Some(f);
        }
    }
    None
}

/// First path from environment variables or under `APPDIR` that is an existing folder.
fn resolve_bundled_dir(env_keys: &[&str], rel_paths: &[&str]) -> Option<PathBuf> {
    for key in env_keys {
        if let Ok(p) = std::env::var(key) {
            if let Some(d) = existing_dir(p) {
                return Some(d);
            }
        }
    }
    let root = appdir()?;
    for rel in rel_paths {
        if let Some(d) = existing_dir(root.join(rel)) {
            return Some(d);
        }
    }
    None
}

/// Bundled ffmpeg, or `None` to use the one on `PATH`.
pub(crate) fn resolve_ffmpeg_bin() -> Option<PathBuf> {
    resolve_bundled_file(
        &["DOCUMENT2VOICE_FFMPEG"],
        &["usr/bin/ffmpeg", "usr/optional/bin/ffmpeg", "bin/ffmpeg"],
    )
}

/// Bundled espeak-ng program.
pub(crate) fn resolve_espeak_bin() -> Option<PathBuf> {
    resolve_bundled_file(
        &["DOCUMENT2VOICE_ESPEAK_BIN"],
        &[
            "usr/bin/espeak-ng",
            "usr/optional/bin/espeak-ng",
            "bin/espeak-ng",
        ],
    )
}

/// Bundled espeak-ng data folder.
pub(crate) fn resolve_espeak_data() -> Option<PathBuf> {
    resolve_bundled_dir(
        &["DOCUMENT2VOICE_ESPEAK_DATA"],
        &[
            "usr/share/espeak-ng-data",
            "usr/lib/espeak-ng-data",
            "usr/share/espeak-data",
        ],
    )
}

/// Candidate Kokoro model folders under `APPDIR`.
pub(crate) fn bundled_model_dir_candidates() -> Vec<PathBuf> {
    let Some(root) = appdir() else {
        return Vec::new();
    };
    vec![
        root.join("usr/share/document2voice/models/kokoro"),
        root.join("usr/share/models/kokoro"),
        root.join("models/kokoro"),
    ]
}

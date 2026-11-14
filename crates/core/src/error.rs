use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("TTS engine error: {0}")]
    Tts(String),

    #[error("export failed: {0}")]
    Export(String),

    #[error("ffmpeg not found (install ffmpeg or bundle it in the AppImage)")]
    FfmpegMissing,

    /// The file is DRM-protected.
    #[error("Unable to process DRM protected file.")]
    DrmProtected {
        /// Detail for logs.
        detail: String,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn drm(detail: impl Into<String>) -> Self {
        Self::DrmProtected {
            detail: detail.into(),
        }
    }

    /// True for [`CoreError::DrmProtected`].
    pub fn is_drm(&self) -> bool {
        matches!(self, Self::DrmProtected { .. })
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

//! Central error types for QuickEdit.
//!
//! We split errors into two layers:
//! * [`AppError`] — a typed, recoverable error returned from the "pure" helper
//!   modules (`ffmpeg`, `video`). Each variant maps to something the UI can
//!   explain to the user.
//! * `anyhow::Error` — used at the top level to stack context when reporting
//!   what went wrong (e.g. "failed to save \"/tmp/x.mp4\": …").
//!
//! Keeping errors structured here means the rest of the code can do
//! `?`-propagation without scattering stringly-typed errors around.

use std::path::PathBuf;
use thiserror::Error;

/// A typed, recoverable error produced by the helper modules.
#[derive(Debug, Error)]
pub enum AppError {
    /// The external `ffmpeg`/`ffprobe` binary was not found or failed.
    #[error("ffmpeg/ffprobe command failed: {message}\nDetails:\n{detail}")]
    Ffmpeg {
        /// A human-readable summary of what went wrong.
        message: String,
        /// Raw stderr from the failing process, if any.
        detail: String,
    },

    /// An image (thumbnail or preview frame) could not be decoded.
    #[error("could not load image '{path}': {source}")]
    Image {
        /// The path that failed to load.
        path: String,
        /// Underlying image-decode error.
        #[source]
        source: image::ImageError,
    },

    /// A filesystem / OS-level failure (open, read, rename, …).
    #[error("I/O error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },

    /// A generic, unstructured error carrying a message.
    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Build an [`AppError::Ffmpeg`] from a command's output.
    pub fn ffmpeg(message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Ffmpeg {
            message: message.into(),
            detail: detail.into(),
        }
    }

    /// Build an [`AppError::Image`] from a path + decode error.
    pub fn image(path: &std::path::Path, source: image::ImageError) -> Self {
        Self::Image {
            path: path.display().to_string(),
            source,
        }
    }

    /// Convenience wrapper around [`std::io::Error`].
    pub fn io(source: std::io::Error) -> Self {
        Self::Io { source }
    }

    /// The absolute path involved, if the error knows one.
    ///
    /// Only the [`AppError::Image`] variant carries a path; the UI uses this to
    /// show *which* file failed to load.
    pub fn path_hint(&self) -> Option<PathBuf> {
        match self {
            Self::Image { path, .. } => Some(PathBuf::from(path.as_str())),
            _ => None,
        }
    }
}

/// Blanket conversions so `?` works everywhere.
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io { source: e }
    }
}

impl From<image::ImageError> for AppError {
    fn from(e: image::ImageError) -> Self {
        Self::Other(format!("image decode error: {e}"))
    }
}

// `anyhow::Error` already has a blanket `From<E>` for any
// `E: std::error::Error + Send + Sync + 'static`, so `AppError` converts into
// it via `?`. Do NOT add a conflicting `impl From<AppError> for anyhow::Error`.
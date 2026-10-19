//! The video model: a file on disk, plus its probed metadata and the user's
//! pending edits for it. A [`Project`] is simply an ordered list of videos and
//! which one is selected.
//!
//! We intentionally keep this lean: there is no media decoding here. Everything
//! visual (thumbnails, frame grabs, export) is delegated to the `ffmpeg` module
//! and the UI just reacts to the results.

use std::path::{Path, PathBuf};

use crate::edit_state::EditState;
use crate::error::AppError;
use crate::ffmpeg::{self, VideoInfo};

/// Default long-edge size for thumbnails, in pixels.
pub const THUMBNAIL_SIZE: u32 = 320;

/// A single video file in a project, with its metadata and pending edits.
#[derive(Debug, Clone)]
pub struct Video {
    /// Path to the source file.
    pub source: PathBuf,
    /// Probed metadata (dimensions, duration, metadata rotation).
    pub info: VideoInfo,
    /// The user's pending edits for this clip.
    pub edit: EditState,
    /// Where the generated thumbnail was written, if one exists.
    pub thumbnail: Option<PathBuf>,
    /// The most recently extracted full-size preview frame, if any.
    pub preview_frame: Option<PathBuf>,
}

impl Video {
    /// Open a video file: probe its metadata and seed a fresh [`EditState`].
    ///
    /// Does not generate a thumbnail — that happens on demand so opening a large
    /// project stays cheap.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(AppError::io(std::io::Error::other(
                format!("file does not exist: {}", path.display()),
            )));
        }
        let info = ffmpeg::probe_metadata(&path)?;
        let edit = EditState::new(info.metadata_rotation);
        Ok(Self {
            source: path,
            info,
            edit,
            thumbnail: None,
            preview_frame: None,
        })
    }

    /// The base name without the directory (e.g. `clip.mp4`).
    pub fn name(&self) -> String {
        self.source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.source.to_string_lossy().to_string())
    }

    /// The base name with the extension stripped (e.g. `clip`).
    pub fn stem(&self) -> String {
        self.source
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "video".to_string())
    }

    /// A `WxH` label for the source resolution.
    pub fn resolution_label(&self) -> String {
        format!("{}x{}", self.info.width, self.info.height)
    }

    /// A `m:ss` duration label.
    pub fn duration_label(&self) -> String {
        let total = self.info.duration_secs as u64;
        let m = total / 60;
        let s = total % 60;
        format!("{:02}:{:02}", m, s)
    }

    /// The display dimensions the editor should use for this clip, given its
    /// current edit state (swaps on 90/270° rotations).
    pub fn display_dimensions(&self) -> (u32, u32) {
        ffmpeg::display_dimensions(self.info.width, self.info.height, &self.edit)
    }

}

impl Video {
    /// Generate (and cache) a thumbnail for the list view.
    pub fn generate_thumbnail(&mut self, seek_secs: f64) -> Result<PathBuf, AppError> {
        let out =
            ffmpeg::temp_path(&format!("quickedit_{}", self.stem()), self.source.len() as u64);
        ffmpeg::generate_thumbnail(&self.source, &out, THUMBNAIL_SIZE, seek_secs)?;
        self.thumbnail = Some(out.clone());
        Ok(out)
    }

    /// Extract a full-size preview frame at `seek_secs` and remember it.
    pub fn extract_frame(&mut self, seek_secs: f64) -> Result<PathBuf, AppError> {
        let out =
            ffmpeg::temp_path(&format!("quickedit_frame_{}", self.stem()), self.source.len() as u64);
        ffmpeg::extract_frame(&self.source, &out, seek_secs)?;
        self.preview_frame = Some(out.clone());
        Ok(out)
    }

    /// Where an export of this clip should be written (next to the source, with
    /// an `_edited` suffix).
    pub fn output_path(&self) -> PathBuf {
        let stem = self.stem();
        let parent = self.source.parent().unwrap_or_else(|| Path::new("."));
        let mut p = parent.to_path_buf();
        p.push(format!("{stem}_edited.mp4"));
        p
    }
}

/// An ordered collection of videos and the currently selected one.
#[derive(Debug, Clone, Default)]
pub struct Project {
    pub videos: Vec<Video>,
    pub index: usize,
}

impl Project {
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently selected video, if any.
    pub fn current(&self) -> Option<&Video> {
        self.videos.get(self.index)
    }

    /// The currently selected video, mutably.
    pub fn current_mut(&mut self) -> Option<&mut Video> {
        self.videos.get_mut(self.index)
    }

    pub fn is_empty(&self) -> bool {
        self.videos.is_empty()
    }

    /// Add a video, selecting it. Returns the new index.
    pub fn add(&mut self, video: Video) -> usize {
        self.videos.push(video);
        self.index = self.videos.len() - 1;
        self.index
    }

    /// Select by index, clamped into range.
    pub fn select(&mut self, index: usize) {
        if let Some(n) = self.videos.len().checked_sub(1) {
            self.index = index.min(n);
        } else {
            self.index = 0;
        }
    }

    /// Number of edits across the whole project that are not yet exported.
    pub fn pending_edit_count(&self) -> usize {
        self.videos.iter().filter(|v| v.edit.has_edits()).count()
    }
}
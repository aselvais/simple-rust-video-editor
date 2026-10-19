//! The per-video editing state: rotation and crop.
//!
//! This is the "model" of what the user has *decided* to do to a clip, separate
//! from the raw file. Keeping it as a plain, `Clone`-able struct means we can
//! hand it to the FFmpeg layer without any UI coupling.
//!
//! # Coordinate spaces (important!)
//!
//! `user_rotation` and `crop` are expressed in **display space** — the image the
//! user actually sees after the container's metadata rotation has been applied.
//! The FFmpeg layer is responsible for translating the crop back into the raw
//! source frame's coordinates (see `ffmpeg::build_filter_chain`).
//!
//! A crop and a new rotation are mutually exclusive: rotating changes which
//! pixels land where, so any crop the user drew is no longer valid. We clear it
//! whenever rotation changes. This is a deliberate, simple UX choice.

use crate::ffmpeg::{normalise_cw, Rect};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditState {
    /// Rotation baked into the container metadata (e.g. phone cameras).
    pub metadata_rotation: u32,
    /// Rotation the user has explicitly requested (0/90/180/270, clockwise).
    pub user_rotation: u32,
    /// A crop rectangle in *display* space, if any.
    pub crop: Option<Rect>,
}

impl EditState {
    /// Construct a state carrying a metadata rotation, no user edits yet.
    pub fn new(metadata_rotation: u32) -> Self {
        Self {
            metadata_rotation: normalise_cw(metadata_rotation as i64),
            user_rotation: 0,
            crop: None,
        }
    }

    pub fn metadata_rotation(&self) -> u32 {
        self.metadata_rotation
    }

    pub fn user_rotation(&self) -> u32 {
        self.user_rotation
    }

    /// The total rotation that the preview (and the FFmpeg filter chain) apply.
    pub fn total_rotation(&self) -> u32 {
        normalise_cw(self.metadata_rotation as i64 + self.user_rotation as i64)
    }

    /// Rotate 90° clockwise. Clears any active crop because the pixel mapping
    /// changes.
    pub fn rotate_cw(&mut self) {
        self.user_rotation = normalise_cw(self.user_rotation as i64 + 90);
        self.clear_crop();
    }

    /// Rotate 90° counter-clockwise.
    pub fn rotate_ccw(&mut self) {
        self.user_rotation = normalise_cw(self.user_rotation as i64 + 270);
        self.clear_crop();
    }

    /// Replace the crop, clamping it to the current display bounds.
    pub fn set_crop(&mut self, crop: Rect, bounds_w: u32, bounds_h: u32) {
        let max_x = (bounds_w as i32).max(1);
        let max_y = (bounds_h as i32).max(1);
        let x = crop.x.clamp(0, max_x - 1);
        let y = crop.y.clamp(0, max_y - 1);
        let w = crop.w.clamp(1, max_x - x);
        let h = crop.h.clamp(1, max_y - y);
        self.crop = Some(Rect { x, y, w, h });
    }

    pub fn clear_crop(&mut self) {
        self.crop = None;
    }

    /// Reset all user edits (rotation and crop), keeping the metadata rotation.
    pub fn reset(&mut self) {
        self.user_rotation = 0;
        self.crop = None;
    }

    /// True if the user has made any change worth exporting.
    pub fn has_edits(&self) -> bool {
        self.user_rotation != 0 || self.crop.is_some()
    }

    /// A short human-readable summary, for the status bar / UI.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.user_rotation != 0 {
            parts.push(format!("rotate {}°", self.user_rotation));
        }
        if let Some(c) = self.crop {
            parts.push(format!("crop {}x{}", c.w, c.h));
        }
        if parts.is_empty() {
            "no edits".to_string()
        } else {
            parts.join(", ")
        }
    }
}
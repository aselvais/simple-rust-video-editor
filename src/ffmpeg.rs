//! Thin wrappers around the `ffmpeg` and `ffprobe` command-line tools.
//!
//! QuickEdit deliberately shells out to the system FFmpeg binaries rather than
//! linking libav, which keeps the Rust build small and dependency-free. The
//! cost is that FFmpeg must be installed and on `PATH` (see `README.md`).
//!
//! The most non-obvious logic here lives in [`build_filter_chain`] and the crop
//! coordinate maths that feed it: the user draws a crop rectangle on the
//! *rotated* preview image, but FFmpeg's filter graph is evaluated on the
//! *un-rotated* source frame, so we must map the crop back through the
//! inverse rotation. This is explained inline in [`transform_crop_to_source`].

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::edit_state::EditState;
use crate::error::AppError;

/// Metadata about a video, as reported by `ffprobe`.
#[derive(Debug, Clone, Default)]
pub struct VideoInfo {
    /// Intrinsic (pre-rotation) pixel width of the video stream.
    pub width: u32,
    /// Intrinsic (pre-rotation) pixel height of the video stream.
    pub height: u32,
    /// Duration in seconds (0.0 if unknown).
    pub duration_secs: f64,
    /// Container/metadata display rotation, normalised to a *clockwise* angle.
    pub metadata_rotation: u32,
}

/// Result of running a child process without inheriting the parent's stdout.
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run `program` with `args`, capturing stdout/stderr.
///
/// We do *not* inherit stdio so that the editor's own output does not get
/// mangled by FFmpeg's progress reporting.
fn run_captured(program: &str, args: &[&str]) -> Result<CommandOutput, AppError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| AppError::Other(format!("failed to spawn '{program}': {e}")))?;

    Ok(CommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// True if `ffmpeg` is present on `PATH`.
pub fn ffmpeg_available() -> bool {
    run_captured("ffmpeg", &["-version"]).map(|o| o.success).unwrap_or(false)
}

/// True if `ffprobe` is present on `PATH`.
pub fn ffprobe_available() -> bool {
    run_captured("ffprobe", &["-version"]).map(|o| o.success).unwrap_or(false)
}

/// Probe a file's video metadata with `ffprobe`.
///
/// We avoid JSON parsing by requesting each value as plain CSV, which keeps the
/// dependency surface small. We read the *first* video stream's dimensions, the
/// container duration, and any `rotate` side-data tag (used by phone cameras).
pub fn probe_metadata(path: &Path) -> Result<VideoInfo, AppError> {
    let mut info = VideoInfo::default();
    let in_s = path.to_string_lossy().to_string();

    // Dimensions: `stream=width,height` for the first video stream.
    if let Ok(out) = run_captured(
        "ffprobe",
        &[
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=p=0",
            &in_s,
        ],
    ) {
        let mut it = out.stdout.trim().split(',');
        if let (Some(w), Some(h)) = (it.next(), it.next()) {
            info.width = w.trim().parse().unwrap_or(0);
            info.height = h.trim().parse().unwrap_or(0);
        }
    }

    // Duration.
    if let Ok(out) = run_captured(
        "ffprobe",
        &[
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "csv=p=0",
            &in_s,
        ],
    ) {
        info.duration_secs = out.stdout.trim().parse().unwrap_or(0.0);
    }

    // Rotation side-data. FFmpeg reports the *counter-clockwise* degrees needed
    // to display the video upright; we normalise to a clockwise angle so the
    // rest of the code (and FFmpeg's `transpose`) speak the same language.
    if let Ok(out) = run_captured(
        "ffprobe",
        &[
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream_tags=rotate",
            "-of", "csv=p=0",
            &in_s,
        ],
    ) {
        let ccw = out.stdout.trim().parse::<i64>().unwrap_or(0);
        info.metadata_rotation = normalise_cw(360 - ccw);
    }

    Ok(info)
}

/// Run `ffmpeg` with the given argument vector, returning a helpful error on
/// failure. We keep only the tail of stderr so an error toast stays readable.
fn run_ffmpeg(args: &[&str]) -> Result<(), AppError> {
    let out = run_captured("ffmpeg", args)?;
    if out.success {
        return Ok(());
    }
    let tail: String = out
        .stderr
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Err(AppError::ffmpeg(
        "ffmpeg reported a non-zero exit status",
        tail.trim().to_string(),
    ))
}

/// Extract a single still frame at `seek_secs` into `out` as a JPEG.
///
/// We first try a fast seek to `seek_secs` (cheap, and avoids grabbing a black
/// frame at the very start of some files). If the file is shorter than that, or
/// the seek lands past EOF, we fall back to grabbing the first frame at 0s.
pub fn extract_frame(input: &Path, out: &Path, seek_secs: f64) -> Result<(), AppError> {
    let in_s = input.to_string_lossy().to_string();
    let out_s = out.to_string_lossy().to_string();
    let seek = seek_secs.max(0.0).to_string();

    let args = ["-y", "-ss", &seek, "-i", &in_s, "-frames:v", "1", "-q:v", "3", &out_s];
    if run_ffmpeg(&args).is_err() {
        let args = ["-y", "-i", &in_s, "-frames:v", "1", "-q:v", "3", &out_s];
        run_ffmpeg(&args)?;
    }
    Ok(())
}

/// Build a thumbnail (default 320px on the long edge) as a JPEG.
pub fn generate_thumbnail(
    input: &Path,
    out: &Path,
    thumb_size: u32,
    seek_secs: f64,
) -> Result<(), AppError> {
    let in_s = input.to_string_lossy().to_string();
    let out_s = out.to_string_lossy().to_string();
    let seek = seek_secs.max(0.0).to_string();
    // `scale=W:-2` keeps the aspect ratio and forces an even height, which the
    // scaler prefers.
    let vf = format!("scale={thumb_size}:-2");

    let args = ["-y", "-ss", &seek, "-i", &in_s, "-frames:v", "1", "-vf", &vf, "-q:v", "3", &out_s];
    if run_ffmpeg(&args).is_err() {
        let args = ["-y", "-i", &in_s, "-frames:v", "1", "-vf", &vf, "-q:v", "3", &out_s];
        run_ffmpeg(&args)?;
    }
    Ok(())
}

/// The FFmpeg `transpose` code for a clockwise angle.
///
/// FFmpeg's `transpose` filter is counter-intuitive, so here is the lookup we
/// actually use (verified against the FFmpeg docs):
/// * 0°   → no transpose
/// * 90°  → `transpose=1`          (90° clockwise)
/// * 180° → `transpose=2,transpose=2`  (symmetric with two even transposes,
///            which plays nicely with crops; equivalent visually to hflip+vflip)
/// * 270° → `transpose=2`          (90° counter-clockwise)
fn transpose_for_cw(angle: u32) -> Option<&'static str> {
    match normalise_cw(angle as i64) {
        0 => None,
        90 => Some("transpose=1"),
        180 => Some("transpose=2,transpose=2"),
        _ => Some("transpose=2"),
    }
}

/// Reduce any angle to one of 0/90/180/270 (mod 360, normalised to `[0,360)`).
pub fn normalise_cw(angle: i64) -> u32 {
    ((angle % 360 + 360) % 360) as u32
}

/// An axis-aligned rectangle in integer pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Build the FFmpeg `-vf` filter chain for the current [`EditState`].
///
/// The chain is, in order:
///   1. any container **metadata** rotation (so phone footage is upright),
///   2. the user's explicit **rotation**,
///   3. the **crop**, mapped from display space back into source space.
///
/// `display_w` / `display_h` are the dimensions of the *currently shown*
/// preview image (i.e. after both rotations), the space in which the user drew
/// the crop.
pub fn build_filter_chain(state: &EditState, display_w: u32, display_h: u32) -> String {
    let mut filters: Vec<String> = Vec::new();

    // 1. Correct for stored metadata rotation first.
    if let Some(mt) = transpose_for_cw(state.metadata_rotation()) {
        filters.push(mt.to_string());
    }

    // 2. Apply the user's rotation.
    if let Some(ut) = transpose_for_cw(state.user_rotation()) {
        filters.push(ut.to_string());
    }

    // 3. Crop. The crop rectangle is defined in *display* space. After the
    //    transposes above, the source frame has been rotated by the total
    //    angle, so we undo that to express the crop in source coordinates.
    if let Some(crop) = state.crop {
        // The crop is in *display* space, so map it back through the total
        // (metadata + user) rotation into source coordinates.
        let total = state.total_rotation();
        let src = transform_crop_to_source(crop, display_w, display_h, total);
        let (x, y, w, h) = even_align(src, display_w, display_h);
        filters.push(format!("crop={w}:{h}:{x}:{y}"));
    }

    filters.join(",")
}

/// Translate a crop rectangle from **display** space (the rotated preview the
/// user sees) back to **source** space (the raw, un-rotated video frame that
/// FFmpeg actually filters on).
///
/// When the total applied rotation is:
/// * 0°   → display == source, no change.
/// * 90° CW  → display point (dx, dy) maps to source `(dy, W - 1 - dx)`,
///             where W is the display width.
/// * 180° → `(W - 1 - dx, H - 1 - dy)`.
/// * 270° CW (= 90° CCW) → `(H - 1 - dy, dx)`.
///
/// Crops are axis-aligned rectangles, so we only need the *bounding box* of the
/// rotated crop: transform each corner and take the min/max.
fn transform_crop_to_source(
    crop: Rect,
    display_w: u32,
    display_h: u32,
    total_cw: u32,
) -> (i32, i32, i32, i32) {
    let dw = display_w as i32;
    let dh = display_h as i32;

    let corners = [
        (crop.x, crop.y),
        (crop.x + crop.w, crop.y),
        (crop.x + crop.w, crop.y + crop.h),
        (crop.x, crop.y + crop.h),
    ];

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;

    // For 90/270° the display and source axes are swapped, so the *source*
    // dimensions are swapped relative to the display. `total_cw` is constant, so
    // compute these once before the loop.
    let (sw, sh) = match total_cw {
        0 | 180 => (dw, dh),
        _ => (dh, dw),
    };

    for (dx, dy) in corners {
        let (sx, sy) = match total_cw {
            0 => (dx, dy),
            90 => (dy, sw - 1 - dx),
            180 => (sw - 1 - dx, sh - 1 - dy),
            _ => (sh - 1 - dy, dx), // 270° CW == 90° CCW
        };
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }

    let sx = min_x.max(0);
    let sy = min_y.max(0);
    let ex = max_x.min(sw - 1);
    let ey = max_y.min(sh - 1);
    (sx, sy, (ex - sx).max(0), (ey - sy).max(0))
}

/// Snap a rectangle to even x/y/w/h.
///
/// `yuv420p` (the default pixel format we request) requires even dimensions, so
/// we round width/height *down* to the nearest even number, floor the origin,
/// and guarantee a minimum 2x2 size.
fn even_align((x, y, w, h): (i32, i32, i32, i32), _pw: u32, _ph: u32) -> (i32, i32, i32, i32) {
    let w = (w / 2) * 2;
    let h = (h / 2) * 2;
    let x = x / 2 * 2;
    let y = y / 2 * 2;
    (x.max(0), y.max(0), w.max(2), h.max(2))
}

/// Apply the edits described by `state` to `input`, writing `output`.
///
/// This is the "save in place" workhorse: it runs `ffmpeg` with the generated
/// filter chain, copies the audio stream losslessly (`-c:a copy`), and re-muxes
/// with fast-start so the result is web/stream friendly.
pub fn apply_edits(input: &Path, output: &Path, state: &EditState) -> Result<PathBuf, AppError> {
    // We need the *display* (post-rotation) dimensions to translate the crop.
    let info = probe_metadata(input)?;
    let (dw, dh) = display_dimensions(info.width, info.height, state);
    let vf = build_filter_chain(state, dw, dh);

    let in_s = input.to_string_lossy().to_string();
    let out_s = output.to_string_lossy().to_string();

    let mut args: Vec<String> = vec!["-y".into(), "-i".into(), in_s];
    if !vf.is_empty() {
        args.push("-vf".into());
        args.push(vf);
    }
    // Re-encode video to H.264 (reliable everywhere) and copy audio losslessly.
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-crf".into(),
        "18".into(), // visually lossless, small file
        "-preset".into(),
        "fast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "copy".into(), // keep original audio untouched
        "-movflags".into(),
        "+faststart".into(),
        out_s,
    ]);
    let ref_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_ffmpeg(&ref_args).map(|_| output.to_path_buf())
}

/// Given the *source* (pre-rotation) dimensions and the active edits, return
/// the dimensions of the displayed preview image.
pub fn display_dimensions(width: u32, height: u32, state: &EditState) -> (u32, u32) {
    // Total rotation (metadata + user); 90/270° swap width and height.
    let total = state.total_rotation();
    if total == 90 || total == 270 {
        (height, width)
    } else {
        (width, height)
    }
}

/// Generate a unique temporary path in the system temp dir.
pub fn temp_path(tag: &str, index: u64) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("{tag}_{index}.jpg"));
    p
}

/// A sensible seek point for a frame grab: ~1s in, but 0s for very short clips.
pub fn frame_seek(duration_secs: f64) -> f64 {
    if duration_secs > 0.0 && duration_secs < 2.0 {
        0.0
    } else {
        1.0
    }
}
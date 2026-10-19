
# QuickEdit — Simple Fast Video Editor (Rust)

**Project goal:** A lightweight, cross-platform desktop video editor written in pure Rust. It lets the user open a folder, preview all video files as a thumbnail grid, select one, and apply two simple non-destructive edits:

- Rotate 90° clockwise or counter-clockwise
- Interactive crop (drag a rectangle on the preview)

Edits are previewed immediately. Saving overwrites the original file in place (same name, same container format) by re-encoding with FFmpeg.

The application must be simple, fast to launch, and require zero intervention once the spec is followed. All code, scripts, and the final README must be produced by the implementing model.

---

## 1. Technology Stack

| Purpose              | Choice                          | Notes |
|----------------------|---------------------------------|-------|
| GUI                  | `eframe` + `egui` (latest)     | Immediate-mode, pure Rust, native on Linux/macOS/Windows |
| File dialogs         | `rfd`                          | Native folder picker |
| Image handling       | `image`                        | Load thumbnails / preview frames |
| Error handling       | `anyhow` + `thiserror`         | Clear error messages in UI |
| Async / background   | `std::thread` or `tokio` (optional) | Keep UI responsive while generating thumbs or running FFmpeg |
| Video processing     | System `ffmpeg` + `ffprobe` via `std::process::Command` | Simplest, most reliable, no linking hell |
| Logging              | `env_logger` or `tracing`      | Optional but useful |

**Runtime dependency:** FFmpeg and FFprobe must be installed and available on `PATH`. Document this clearly in the final README.

Supported video extensions (case-insensitive):  
`.mp4`, `.mov`, `.avi`, `.mkv`, `.webm`, `.flv`, `.wmv`, `.m4v`, `.mpeg`, `.mpg`

---

## 2. High-Level Architecture

```
src/
├── main.rs          # eframe entry, App state
├── ui.rs            # All egui panels, layout, interactions
├── video.rs         # Video metadata, thumbnail generation, preview frame extraction
├── ffmpeg.rs        # All Command wrappers for ffmpeg/ffprobe
├── edit_state.rs    # Rotation + crop state for the selected video
└── error.rs         # Custom error types (optional)
```

- Keep the UI thread free. Run thumbnail generation and FFmpeg jobs on background threads; communicate results via channels or shared state with `Arc<Mutex<_>>`.
- Cache thumbnails in a temporary directory (or in-memory if small).
- All FFmpeg operations must show a progress indicator or at least a “Processing…” status in the UI.

---

## 3. Feature Specifications

### 3.1 Folder Browser & Thumbnail Grid

- Top-level button: **“Open Folder”** → native folder dialog (`rfd`).
- After selection, recursively scan the folder (non-recursive is acceptable if simpler) for video files.
- Display a scrollable grid of thumbnails (e.g. 160–200 px wide).
- Each thumbnail is generated with FFmpeg:
  ```bash
  ffmpeg -y -ss 1 -i "<input>" -vframes 1 -q:v 3 -vf scale=320:-1 "<thumb.jpg>"
  ```
  (Use `-ss 0` or a short seek if the video is very short.)
- Show filename under each thumbnail.
- Clicking a thumbnail selects that video and loads it into the preview pane.
- If thumbnail generation fails, show a placeholder icon + error tooltip.
- Generate thumbnails in the background so the grid appears quickly.

### 3.2 Video Preview Pane

- Large preview area showing the current frame of the selected video.
- Extract a representative frame (middle of the video or frame at 1 s) with FFmpeg into a temporary JPEG/PNG and load it with the `image` crate into an `egui::Image`.
- When rotation or crop changes, re-generate the preview frame with the current filters applied (or apply rotation/crop on the CPU image for speed, then re-extract only on save). Prefer CPU-side rotation of the loaded image for instant feedback.
- Show video filename, duration, resolution (from `ffprobe`), and current edit state (rotation degrees, crop rect).

### 3.3 Rotation

- Two buttons only:
  - **Rotate 90° CW**
  - **Rotate 90° CCW**
- Internal state: `rotation: i32` (0, 90, 180, 270).
- Each click updates the state and immediately refreshes the preview (rotate the preview image with the `image` crate or re-extract with FFmpeg filter).
- When applying on save, map to FFmpeg `transpose` filter:
  - 90° CW → `transpose=1`
  - 90° CCW → `transpose=2`
  - 180° → `transpose=1,transpose=1` (or `hflip,vflip`)
  - 270° etc. accordingly.

### 3.4 Interactive Crop

- Toggle button: **“Crop Mode”**.
- When active, the user can click-and-drag on the preview image to draw a rectangle.
- Implementation guidance:
  - Use `ui.interact(rect, id, Sense::drag())` or pointer events on the image area.
  - Track start and current mouse position relative to the image.
  - Draw the selection rectangle with `painter.rect_stroke` and dim the outside area (optional but nice).
  - Store crop as percentages or absolute pixels relative to the **original** video dimensions (after accounting for current rotation).
- Provide a **“Clear Crop”** button.
- Crop coordinates must be correctly transformed when the video is rotated (important!).
- On save, emit FFmpeg `crop=w:h:x:y` filter. Ensure even dimensions if required by the encoder.

### 3.5 Save In Place

- Button: **“Save in Place”** (disabled if no edits).
- Confirmation dialog: “Overwrite original file? This cannot be undone.”
- Process:
  1. Build FFmpeg filter chain from current rotation + crop.
  2. Write to a temporary file in the same directory (or system temp) with the same extension.
  3. Example command skeleton:
     ```bash
     ffmpeg -y -i "<original>" \
       -vf "<transpose>,crop=..." \
       -c:v libx264 -crf 18 -preset medium -pix_fmt yuv420p \
       -c:a copy \
       -movflags +faststart \
       "<temp_file>"
     ```
     (Adjust codec/container to match original when possible; fallback to H.264 + AAC is acceptable.)
  4. On success, replace the original (atomic rename preferred). On failure, delete the temp file and show the error.
- Preserve audio stream with `-c:a copy` whenever possible.
- Show progress / status while FFmpeg runs.
- After successful save, clear the edit state and refresh the thumbnail.

### 3.6 Additional UX Requirements

- Status bar at the bottom: current operation, errors, “Ready”.
- Keyboard shortcuts (nice-to-have):
  - `Ctrl/Cmd + O` → Open Folder
  - `Ctrl/Cmd + S` → Save
  - `R` / `Shift+R` → Rotate
- Window title shows the app name + currently selected file.
- Dark theme by default (egui default is fine).
- Handle missing FFmpeg gracefully: detect on startup and show a clear error with installation instructions.

---

## 4. Build System & Cross-Platform Scripts

Create the following scripts in the project root. They must be ready to run on each platform.

### `build.sh` (Linux & macOS)
```bash
#!/usr/bin/env bash
set -euo pipefail
cargo build --release
mkdir -p builds
cp target/release/quickedit builds/quickedit-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)
echo "Built: builds/quickedit-..."
```

### `build.ps1` (Windows PowerShell)
```powershell
cargo build --release
New-Item -ItemType Directory -Force -Path builds | Out-Null
Copy-Item target\release\quickedit.exe builds\quickedit-windows-x64.exe
Write-Host "Built: builds\quickedit-windows-x64.exe"
```

### `build.bat` (Windows fallback)
```bat
cargo build --release
if not exist builds mkdir builds
copy /Y target\release\quickedit.exe builds\quickedit-windows-x64.exe
echo Built: builds\quickedit-windows-x64.exe
```

**After implementation:**
- Run the appropriate build script for the machine the model is running on.
- Place the resulting binary in the `builds/` directory.
- Update the final README with exact relative links to the produced binary and clear instructions for the other two platforms.

Also add a simple `.github/workflows/build.yml` (optional but recommended) that builds release binaries for Linux, Windows, and macOS using GitHub Actions so the user can obtain all three builds with one push.

---

## 5. Final README Requirements

The implementing model **must** generate a complete `README.md` containing:

1. Project description and screenshot placeholder (or note).
2. Prerequisites:
   - Rust (stable)
   - FFmpeg + FFprobe in PATH (with install links for each OS)
3. How to build from source on Linux, macOS, and Windows (copy the scripts above + exact commands).
4. **Links to the builds** (relative paths):
   - Linux: `[Linux build](builds/quickedit-linux-...)`
   - macOS: `[macOS build](builds/quickedit-macos-...)`
   - Windows: `[Windows build](builds/quickedit-windows-x64.exe)`
   - Note which ones were actually produced during this session and which require running the script on that OS.
5. Usage instructions (open folder → select → rotate/crop → save).
6. Known limitations (re-encoding, no real-time playback scrubbing, FFmpeg dependency, etc.).
7. License (MIT or Apache-2.0).

---

## 6. Acceptance Criteria (Definition of Done)

The project is complete when:

- [ ] `cargo build --release` succeeds on the current machine.
- [ ] Opening a folder shows a thumbnail grid of all videos.
- [ ] Selecting a video shows a preview.
- [ ] Rotate CW / CCW updates the preview instantly.
- [ ] Crop mode allows drawing a rectangle that is visually correct and applied on save.
- [ ] “Save in Place” overwrites the original file with the rotated/cropped version (audio preserved when possible).
- [ ] Build scripts for all three platforms exist and the current platform’s binary is in `builds/`.
- [ ] README contains full build instructions and relative links to the builds.
- [ ] No panics on missing files or FFmpeg errors; clear messages shown in the UI.
- [ ] Code is clean, modular, and well-commented where non-obvious (especially crop coordinate transforms and FFmpeg filter construction).

---

## 7. Implementation Notes for the Model

- Prefer simplicity and reliability over advanced features (no timeline, no multi-clip, no real-time playback unless trivial).
- Test the FFmpeg filter chain with a short sample video if possible.
- Use `ffprobe -v quiet -print_format json -show_streams` to get width/height/duration when needed.
- Account for video rotation metadata (some phones store rotation in metadata; `ffprobe` can report it).
- Make the crop rectangle stay within video bounds.
- After save, regenerate the thumbnail so the grid reflects the new content.

Follow this specification exactly. Produce a working application that requires no further manual coding from the user.


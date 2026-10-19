//! The egui interface for QuickEdit.
//!
//! This is the only module that knows about `egui`. It owns [`QuickEditApp`]
//! and turns user actions (open, rotate, crop, export) into calls on the
//! `video` and `ffmpeg` modules.
//!
//! ## Threading
//!
//! Heavy work — frame extraction and export — runs on background threads. The
//! UI thread only polls for their results over an `mpsc` channel and repaints
//! when one arrives, so the window stays responsive.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use egui::{self, Context, Pos2, Rect as EguiRect, Stroke, Ui};

use crate::error::AppError;
use crate::ffmpeg;
use crate::video::{Project, Video};

/// How often to repaint while a background task is running.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Largest the on-screen preview is allowed to grow.
const PREVIEW_MAX_WIDTH: f32 = 640.0;

/// The entire application state.
pub struct QuickEditApp {
    /// The opened videos and their pending edits.
    pub project: Project,
    /// A short status line for the bottom bar.
    status: String,
    /// The most recent error, if any.
    error: Option<AppError>,
    /// Receives the result of an in-progress export.
    export_rx: Option<mpsc::Receiver<Result<PathBuf, AppError>>>,
    /// Name of the clip currently being exported.
    exporting_name: Option<String>,
    /// Whether the crop overlay is active.
    crop_enabled: bool,
    /// An in-progress crop drag, in *screen* coords: (start, end).
    crop_drag: Option<(Pos2, Pos2)>,
    /// The screen rect the preview image occupied last frame.
    last_image_rect: EguiRect,
    /// The scrub position as a 0..1 fraction of the current clip.
    scrub_pos: f64,
    /// The GPU texture for the current preview frame, if one is loaded.
    preview_texture: Option<egui::TextureHandle>,
}

impl QuickEditApp {
    /// Build a fresh app with an empty project.
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        Self {
            project: Project::new(),
            status: "Open a video to begin.".into(),
            error: None,
            export_rx: None,
            exporting_name: None,
            crop_enabled: false,
            crop_drag: None,
            last_image_rect: EguiRect::NOTHING,
            scrub_pos: 0.0,
            preview_texture: None,
        }
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    /// Poll the export channel once; keep the receiver if the worker hasn't
    /// finished yet.
    fn poll_export(&mut self, ctx: &Context) {
        let Some(rx) = self.export_rx.take() else { return; };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                self.exporting_name = None;
                self.error = None;
                self.set_status(format!("Saved: {}", path.display()));
                ctx.request_repaint();
            }
            Ok(Err(e)) => {
                self.exporting_name = None;
                self.error = Some(e);
                self.set_status("Export failed — see the status bar.");
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.export_rx = Some(rx);
                ctx.request_repaint_after(POLL_INTERVAL);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.exporting_name = None;
                self.set_status("Export cancelled.");
            }
        }
    }
}

impl QuickEditApp {
    /// Kick off a background export of the current clip.
    fn export_current(&mut self, ctx: &Context) {
        let Some(video) = self.project.current().cloned() else {
            self.set_status("No clip selected.");
            return;
        };
        let input = video.source.clone();
        let output = video.output_path();
        let state = video.edit.clone();
        let name = video.name();

        let (tx, rx) = mpsc::channel();
        self.export_rx = Some(rx);
        self.exporting_name = Some(name.clone());
        self.set_status(format!("Exporting {name} …"));

        std::thread::spawn(move || {
            let result = ffmpeg::apply_edits(&input, &output, &state);
            let _ = tx.send(result);
        });
        ctx.request_repaint();
    }

    /// Open the native file dialog and load whatever the user picks.
    fn open_dialog(&mut self, ctx: &Context) {
        let dialog = rfd::FileDialog::new()
            .add_filter("Video files", &["mp4", "mov", "avi", "mkv", "webm", "m4v", "ts"]);

        let picked = dialog.pick_file();
        match picked {
            Some(path) => match Video::open(&path) {
                Ok(mut video) => {
                    // Make an initial thumbnail so the list isn't blank.
                    if let Err(e) = video.generate_thumbnail(0.0) {
                        self.error = Some(e);
                        self.set_status("Opened, but the thumbnail failed.");
                    }
                    let name = video.name();
                    self.project.add(video);
                    self.set_status(format!("Opened {name}."));
                }
                Err(e) => {
                    self.error = Some(e);
                    self.set_status("Could not open that file.");
                }
            },
            None => {}
        }
        ctx.request_repaint();
    }

    /// Extract a preview frame at the current scrub position.
    fn extract_preview(&mut self, ctx: &Context) {
        let Some(video) = self.project.current_mut() else {
            return;
        };
        let dur = video.info.duration_secs;
        let seek = if dur > 0.0 { self.scrub_pos * dur } else { 0.0 };
        match video.extract_frame(seek) {
            Ok(path) => {
                self.set_status(format!("Preview extracted at {seek:.2}s → {}", path.display()));
            }
            Err(e) => {
                self.error = Some(e);
                self.set_status("Frame extraction failed.");
            }
        }
        ctx.request_repaint();
    }
}


impl QuickEditApp {
    /// Path to the image we should display as a preview, if any.
    fn preview_path(&self) -> Option<PathBuf> {
        let v = self.project.current()?;
        v.preview_frame.clone().or_else(|| v.thumbnail.clone())
    }

    fn editor(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.heading("Editor");

        // --- Preview image ---
        // egui 0.29 removed `Image::from_path`, so we decode the frame with the
        // `image` crate and upload it as a texture. The texture is re-uploaded
        // each time the preview is rendered, which mirrors the previous
        // per-frame reload (the preview file is reused in place on every scrub).
        match self.preview_path() {
            Some(path) if path.exists() => {
                match image::open(&path) {
                    Ok(decoded) => {
                        let rgba = decoded.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                        self.preview_texture = Some(ctx.load_texture(
                            "preview",
                            color_image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    Err(_) => {
                        self.preview_texture = None;
                    }
                }

                if let Some(handle) = self.preview_texture.clone() {
                    let image =
                        egui::Image::from_texture(&handle).max_width(PREVIEW_MAX_WIDTH);
                    let resp = ui.add(image);
                    self.last_image_rect = resp.rect;
                    self.handle_crop_drag(ctx);
                    if self.crop_enabled {
                        self.draw_crop_overlay(ui);
                    }
                } else {
                    ui.label("(unable to load preview image)");
                }
            }
            Some(_) => {
                ui.label("(preview file missing)");
            }
            None => {
                ui.label("(no preview yet)");
            }
        }

        // --- Scrub + frame extraction ---
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Scrub:");
            ui.add(egui::Slider::new(&mut self.scrub_pos, 0.0..=1.0).text("position"));
            let clicked = ui.button("Extract frame").clicked();
            if clicked {
                self.extract_preview(ctx);
            }
        });

        // --- Crop controls ---
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.crop_enabled, "Crop");
            match self.project.current() {
                Some(v) => {
                    let crop = v.edit.crop;
                    match crop {
                        Some(c) => {
                            ui.label(format!("{}×{} @ ({}, {})", c.w, c.h, c.x, c.y));
                        }
                        None => {
                            ui.label("no crop");
                        }
                    }
                }
                None => {}
            }
            let clear = ui.button("Clear crop").clicked();
            if clear {
                if let Some(v) = self.project.current_mut() {
                    v.edit.clear_crop();
                }
                ctx.request_repaint();
            }
        });

        // --- Rotation + reset ---
        ui.horizontal(|ui| {
            let ccw = ui.button("Rotate ⟲ CCW").clicked();
            if ccw {
                if let Some(v) = self.project.current_mut() {
                    v.edit.rotate_ccw();
                }
                ctx.request_repaint();
            }
            let cw = ui.button("Rotate ⟳ CW").clicked();
            if cw {
                if let Some(v) = self.project.current_mut() {
                    v.edit.rotate_cw();
                }
                ctx.request_repaint();
            }
            let reset = ui.button("Reset edits").clicked();
            if reset {
                if let Some(v) = self.project.current_mut() {
                    v.edit.reset();
                }
                ctx.request_repaint();
            }
        });

        // --- Export ---
        ui.add_space(8.0);
        let export = ui
            .add(egui::Button::new("Export current clip").min_size(egui::vec2(220.0, 34.0)))
            .clicked();
        if export {
            self.export_current(ctx);
        }
    }
}
impl eframe::App for QuickEditApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll_export(ctx);

        if self.project.is_empty() {
            egui::CentralPanel::default().show(ctx, |ui| self.empty_state(ui, ctx));
        } else {
            self.top_bar(ctx);
            egui::SidePanel::left("video_list")
                .resizable(true)
                .default_width(300.0)
                .show(ctx, |ui| self.video_list(ui, ctx));
            egui::TopBottomPanel::bottom("status_bar")
                .show(ctx, |ui| self.status_bar(ui));
            egui::CentralPanel::default().show(ctx, |ui| self.editor(ui, ctx));
        }
    }
}

impl QuickEditApp {
    /// The welcome screen shown when the project has no clips.
    fn empty_state(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.heading("QuickEdit");
            ui.add_space(8.0);
            ui.label("Open a video to trim, rotate or crop it.");
            ui.add_space(16.0);
            let clicked = ui.button("Open video…").clicked();
            if clicked {
                self.open_dialog(ctx);
            }
        });
    }

    /// The top bar: app title, an "open video" button and an export indicator.
    fn top_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("QuickEdit");
                ui.separator();
                if ui.button("Open video…").clicked() {
                    self.open_dialog(ctx);
                }
                if let Some(name) = &self.exporting_name {
                    ui.separator();
                    ui.spinner();
                    ui.label(format!("Exporting {name}…"));
                }
            });
        });
    }

    /// The left panel: every clip, with a selection highlight and edit marker.
    fn video_list(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.heading("Clips");
        if ui.button("＋ Open").clicked() {
            self.open_dialog(ctx);
        }
        ui.separator();

        for i in 0..self.project.videos.len() {
            let name = self.project.videos[i].name();
            let duration = self.project.videos[i].duration_label();
            let resolution = self.project.videos[i].resolution_label();
            let has_edit = self.project.videos[i].edit.has_edits();
            let selected = i == self.project.index;

            if ui.selectable_label(selected, format!("{name}  {duration}")).clicked() {
                self.project.select(i);
                ctx.request_repaint();
            }
            ui.label(format!("    {resolution}{}", if has_edit { "  ✎" } else { "" }));
        }
        if self.project.videos.is_empty() {
            ui.add_space(8.0);
            ui.label("(no clips yet)");
        }
    }

    /// The bottom bar: status text, pending-edit count and any error.
    fn status_bar(&self, ui: &mut Ui) {
        let pending = self.project.pending_edit_count();
        let plural = if pending == 1 { "" } else { "s" };
        ui.horizontal(|ui| {
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pending > 0 {
                    ui.label(format!("{pending} unsaved edit{plural}"));
                }
            });
        });
        if let Some(err) = &self.error {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
            });
        }
    }
}

impl QuickEditApp {
    /// Map two screen points to display (preview-native) coordinates.
    fn screen_to_display(&self, rect: EguiRect, a: Pos2, b: Pos2) -> (f32, f32, f32, f32) {
        let (dw, dh) = self.project.current().map(|v| v.display_dimensions()).unwrap_or((1, 1));
        let sx = (rect.width() / dw as f32).max(1e-3);
        let sy = (rect.height() / dh as f32).max(1e-3);
        let dx0 = ((a.x - rect.left()) / sx).clamp(0.0, dw as f32);
        let dy0 = ((a.y - rect.top()) / sy).clamp(0.0, dh as f32);
        let dx1 = ((b.x - rect.left()) / sx).clamp(0.0, dw as f32);
        let dy1 = ((b.y - rect.top()) / sy).clamp(0.0, dh as f32);
        (dx0, dy0, dx1, dy1)
    }

    /// Handle starting / continuing / finishing a crop drag over the preview.
    fn handle_crop_drag(&mut self, ctx: &Context) {
        let down = ctx.input(|i| {
            if i.pointer.primary_pressed() {
                i.pointer.interact_pos()
            } else {
                None
            }
        });
        let interact = ctx.input(|i| i.pointer.interact_pos());
        let released = ctx.input(|i| i.pointer.primary_released());
        let rect = self.last_image_rect;

        if let Some(pos) = down.filter(|p| rect.contains(*p)) {
            self.crop_drag = Some((pos, pos));
            self.crop_enabled = true;
        }
        if let Some((start, _)) = self.crop_drag {
            if let Some(pos) = interact {
                self.crop_drag = Some((start, pos));
            }
        }
        if released {
            if let Some((start, end)) = self.crop_drag.take() {
                let (dx0, dy0, dx1, dy1) = self.screen_to_display(rect, start, end);
                let x = dx0.min(dx1);
                let y = dy0.min(dy1);
                let w = (dx1 - dx0).abs();
                let h = (dy1 - dy0).abs();
                if w > 4.0 && h > 4.0 {
                    if let Some(v) = self.project.current_mut() {
                        let (bw, bh) = v.display_dimensions();
                        let r = ffmpeg::Rect {
                            x: x as i32,
                            y: y as i32,
                            w: w as i32,
                            h: h as i32,
                        };
                        v.edit.set_crop(r, bw, bh);
                    }
                    ctx.request_repaint();
                }
            }
        }
    }

    /// Draw the committed crop (and any in-progress drag) over the preview.
    fn draw_crop_overlay(&self, ui: &mut Ui) {
        let Some(video) = self.project.current() else {
            return;
        };
        let rect = self.last_image_rect;
        let (dw, dh) = video.display_dimensions();
        let sx = (rect.width() / dw as f32).max(1e-3);
        let sy = (rect.height() / dh as f32).max(1e-3);
        let painter = ui.painter();

        if let Some((start, end)) = self.crop_drag {
            let minx = start.x.min(end.x);
            let maxx = start.x.max(end.x);
            let miny = start.y.min(end.y);
            let maxy = start.y.max(end.y);
            let drag = EguiRect::from_min_max(Pos2::new(minx, miny), Pos2::new(maxx, maxy));
            painter.rect_stroke(drag, 0.0, Stroke::new(2.0_f32, egui::Color32::from_rgb(80, 200, 255)));
        }

        if let Some(c) = video.edit.crop {
            let r = EguiRect::from_min_size(
                Pos2::new(rect.left() + c.x as f32 * sx, rect.top() + c.y as f32 * sy),
                egui::vec2(c.w as f32 * sx, c.h as f32 * sy),
            );
            painter.rect_stroke(r, 0.0, Stroke::new(2.0_f32, egui::Color32::from_rgb(80, 200, 255)));
        }
    }
}
//! QuickEdit — a fast, local video editor.
//!
//! This is the binary entry point. It wires the [`ui::QuickEditApp`] into
//! `eframe` and lets the windowing backend take over. All real logic lives in
//! the submodules below (`ui`, `video`, `ffmpeg`, `edit_state`, `error`).

mod edit_state;
mod error;
mod ffmpeg;
mod ui;
mod video;

fn main() -> eframe::Result {
    let _ = env_logger::init();

    eframe::run_native(
        "QuickEdit — a fast, local video editor",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(ui::QuickEditApp::new(cc)))),
    )
}

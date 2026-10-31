//! Core logic for the desktop lyrics overlay.
//!
//! Kept free of any UI dependency so it can be unit-tested on any platform
//! and reused by both the CLI prototype and the Tauri app.

pub mod lyrics;
pub mod player;

//! Embeds the NSIS installer this window fronts.
//!
//! The release workflow builds the app's NSIS installer first and points
//! `LYRICS_SETUP_PAYLOAD` at it. Without it (local builds, CI checks) the
//! setup runs in demo mode: same window, simulated install.

use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(payload)");
    println!("cargo:rerun-if-env-changed=LYRICS_SETUP_PAYLOAD");
    if let Some(path) = std::env::var_os("LYRICS_SETUP_PAYLOAD") {
        let path = PathBuf::from(path);
        println!("cargo:rerun-if-changed={}", path.display());
        let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("payload.exe");
        std::fs::copy(&path, &out)
            .unwrap_or_else(|e| panic!("can't read the installer at {}: {e}", path.display()));
        println!("cargo:rustc-cfg=payload");
    }
    tauri_build::build()
}

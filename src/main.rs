#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The desktop entry point. Everything it does lives in the library beside
//! it, so the same code can be built for the browser.

#[cfg(not(target_family = "wasm"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    cyberapi_gui::run()
}

/// The browser has no `main`: its entry point is `cyberapi-web`, which calls
/// into this crate as a library. This exists only so the bin target parses
/// under a wasm check.
#[cfg(target_family = "wasm")]
fn main() {}

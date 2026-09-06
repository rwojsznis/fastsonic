//! Fastsonic's internals, exposed so diagnostics and tests can reach them.

pub mod api;
pub mod app;
pub mod backend;
pub mod bidi;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod engine;
pub mod eq;
pub mod history;
pub mod images;
pub mod limiter;
pub mod lyrics;
#[cfg(target_os = "macos")]
pub mod mac_fonts;
#[cfg(target_os = "macos")]
pub mod mac_menu;
pub mod media;
#[cfg(target_os = "linux")]
#[path = "mpris.rs"]
pub mod media_controls;
#[cfg(not(target_os = "linux"))]
#[path = "media_native.rs"]
pub mod media_controls;
pub mod milkdrop;
pub mod model;
pub mod opener;
pub mod opus;
pub mod paths;
pub mod resample;
pub mod settings;
pub mod single_instance;
pub mod sink;
pub mod skin;
pub mod system_fonts;
pub mod theme;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_native.rs"]
pub mod tray;
pub mod ui;
pub mod updates;
pub mod util;
pub mod vis;
pub mod winamp;
pub mod window;

/// The builder every asynchronous HTTP client in the app starts from.
///
/// reqwest is built with `rustls-no-provider` (see `Cargo.toml`), and it
/// *panics* when a client is built before a crypto provider is installed.
/// Going through here rather than `reqwest::Client::builder` makes that
/// impossible to forget.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    install_tls_provider();
    reqwest::Client::builder()
}

/// The same, for blocking clients such as the MilkDrop preset download.
pub fn blocking_http_client_builder() -> reqwest::blocking::ClientBuilder {
    install_tls_provider();
    reqwest::blocking::Client::builder()
}

/// Install ring as the process's rustls crypto provider. Idempotent.
fn install_tls_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

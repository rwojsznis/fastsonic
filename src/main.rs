//! Desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod auth;
mod backend;
#[cfg(any(test, feature = "demo"))]
mod demo;
mod images;
mod model;
#[cfg(target_os = "linux")]
mod mpris;
#[cfg(not(target_os = "linux"))]
#[path = "mpris_stub.rs"]
mod mpris;
mod paths;
mod player;
mod settings;
mod theme;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
#[path = "tray_stub.rs"]
mod tray;
mod ui;
mod util;

use clap::Parser;

/// A fast, native Spotify client.
#[derive(Debug, Parser)]
#[command(name = "fastpotify", version, about)]
struct Cli {
    /// Spotify Connect device name for this session.
    #[arg(long)]
    device_name: Option<String>,

    /// Log more from librespot and the Web API client.
    #[arg(short, long)]
    verbose: bool,

    /// Start with sample data and no Spotify connection (for screenshots).
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo: bool,

    /// Page to open in demo mode, e.g. `home`, `playlist:pl1`, `artist:art0`.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo_page: Option<String>,

    /// Extra demo surfaces: a comma-separated list of `queue`, `devices`,
    /// `shortcuts`, `create`, `light`.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo_show: Option<String>,
}

fn main() -> eframe::Result<()> {
    let cli = Cli::parse();
    let default_filter = if cli.verbose {
        "info,librespot=info,fastpotify=debug"
    } else {
        "warn,fastpotify=info"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .init();

    let dirs = paths::AppDirs::discover();
    if let Err(error) = dirs.ensure() {
        log::warn!("unable to create the application directories: {error}");
    }
    let mut settings = settings::Settings::load(&dirs.settings_file());
    if let Some(name) = cli.device_name {
        settings.device_name = name;
    }

    // The application (audio engine, Web API, MPRIS, tray) outlives any
    // window. Closing to the tray destroys the window and this loop creates
    // a new one when the tray or MPRIS asks for it. Plain window lifecycle,
    // portable across desktops.
    let waker = backend::Waker::default();
    #[allow(unused_mut)]
    let mut app = app::App::new(&waker, dirs, settings, app::AppOptions::default());
    #[cfg(feature = "demo")]
    if cli.demo {
        demo::populate(&mut app);
        demo::apply_flags(&mut app, cli.demo_page.as_deref(), cli.demo_show.as_deref());
    }
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));

    loop {
        let creator_slot = std::sync::Arc::clone(&slot);
        let creator_waker = waker.clone();
        let options = native_options();
        eframe::run_native(
            "Fastpotify",
            options,
            Box::new(move |cc| {
                creator_waker.attach(&cc.egui_ctx);
                let mut app = creator_slot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                    .expect("application state present");
                app.attach(&cc.egui_ctx);
                Ok(Box::new(Shell {
                    app: Some(app),
                    slot: std::sync::Arc::clone(&creator_slot),
                }))
            }),
        )?;
        waker.detach();

        let hide = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            let app = guard.as_ref().expect("application state present");
            !app.quit_requested && app.hide_intent
        };
        if !hide {
            break;
        }

        // Tray life: no window, but audio, MPRIS, the tray, and polling all
        // keep running until Show or Quit.
        let headless = egui::Context::default();
        {
            let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            let app = guard.as_mut().expect("application state present");
            app.window_hidden = true;
            app.hide_intent = false;
            app.wants_show = false;
        }
        loop {
            {
                let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
                let app = guard.as_mut().expect("application state present");
                app.background_frame(&headless);
                if app.quit_requested || app.wants_show {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let quit = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .as_ref()
                .expect("application state present")
                .quit_requested
        };
        if quit {
            break;
        }
    }

    if let Some(mut app) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
        app.shutdown();
    }
    Ok(())
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fastpotify")
            .with_app_id("fastpotify")
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(app_icon()),
        // A Wayland compositor stops sending frame callbacks to a hidden
        // window; waiting for vsync there would block the event loop.
        // Repaints are event-driven, so nothing spins.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The eframe adapter around the long-lived [`app::App`]: delegates frames
/// and, when the window goes away, hands the state back for the next window.
struct Shell {
    app: Option<app::App>,
    slot: std::sync::Arc<std::sync::Mutex<Option<app::App>>>,
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            app.background_frame(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            app.frame_ui(ui);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(app) = self.app.as_mut() {
            app.save_state();
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) = self.app.take();
    }
}

/// The window icon, from the shared runtime drawing.
fn app_icon() -> egui::IconData {
    const SIZE: usize = 128;
    egui::IconData {
        rgba: util::app_icon_rgba(SIZE),
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

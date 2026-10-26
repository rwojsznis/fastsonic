//! A status-notifier tray item (Linux), so closing the window can leave the
//! music playing.
//!
//! The tray runs on its own thread and exchanges bounded messages with the
//! interface, exactly like MPRIS: a missing or broken status-notifier host
//! must never take audio or the window down with it. When no host is
//! present, spawning fails and the app simply quits on close as before.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use ksni::blocking::TrayMethods;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

struct FastTray {
    commands: Sender<TrayCommand>,
    wake: Arc<dyn Fn() + Send + Sync>,
    playing: bool,
    colour: crate::settings::TrayIcon,
    /// Whether a panel's status-notifier watcher has the item.
    shown: Arc<AtomicBool>,
}

impl FastTray {
    fn send(&self, command: TrayCommand) {
        if self.commands.send(command).is_ok() {
            (self.wake)();
        }
    }
}

impl ksni::Tray for FastTray {
    fn id(&self) -> String {
        "fastsonic".into()
    }

    fn title(&self) -> String {
        "Fastsonic".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 64usize;
        let rgba = crate::util::tray_icon_rgba(self.colour, size);
        // ksni wants ARGB32 in network byte order.
        let mut data = Vec::with_capacity(rgba.len());
        let (pixels, _) = rgba.as_chunks::<4>();
        for [r, g, b, a] in pixels {
            data.extend_from_slice(&[*a, *r, *g, *b]);
        }
        vec![ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ShowHide);
    }

    /// A panel came, after none at the start or after the last one went:
    /// ksni registers the item with it.
    fn watcher_online(&self) {
        self.shown.store(true, Ordering::SeqCst);
        (self.wake)();
    }

    /// No panel shows the item (yet): ksni keeps it and registers it when
    /// one comes.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        log::info!("no panel shows the tray item: {reason:?}");
        self.shown.store(false, Ordering::SeqCst);
        (self.wake)();
        true
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Show or hide Fastsonic".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::ShowHide)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if self.playing {
                    "Pause".into()
                } else {
                    "Play".into()
                },
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::PlayPause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Next)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Previous)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayService {
    handle: ksni::blocking::Handle<FastTray>,
    commands: Receiver<TrayCommand>,
    playing: bool,
    colour: crate::settings::TrayIcon,
    shown: Arc<AtomicBool>,
}

impl TrayService {
    /// Registers the tray item. `None` without a session bus. A panel that
    /// is not there yet, as at login, may come later: ksni registers the
    /// item with it then, and [`is_shown`](Self::is_shown) says whether one
    /// has it.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        let shown = Arc::new(AtomicBool::new(true));
        let tray = FastTray {
            commands: sender,
            wake: Arc::new(wake),
            playing: false,
            colour: crate::settings::TrayIcon::Colour,
            shown: Arc::clone(&shown),
        };
        // Flatpak allows talking to the watcher, but not owning ksni's
        // generated StatusNotifierItem name. Register the unique connection
        // name instead, as ksni requires for sandboxed applications.
        let flatpak = std::path::Path::new("/.flatpak-info").exists()
            || std::env::var_os("FLATPAK_ID").is_some();
        // Without a watcher yet, the registration is tried again when one
        // comes; ksni says so through `watcher_offline` before `spawn`
        // returns.
        match tray
            .disable_dbus_name(flatpak)
            .assume_sni_available(true)
            .spawn()
        {
            Ok(handle) => Some(Self {
                handle,
                commands,
                playing: false,
                colour: crate::settings::TrayIcon::Colour,
                shown,
            }),
            Err(error) => {
                log::info!("no system tray available: {error}");
                None
            }
        }
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Keeps the menu's Play/Pause label matching reality.
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.playing = playing;
            self.handle.update(|tray| tray.playing = playing);
        }
    }

    /// Shows the icon in colour, or white or black to match the panel, in
    /// place: ksni tells the panel the icon changed.
    pub fn set_icon_colour(&mut self, colour: crate::settings::TrayIcon) {
        if self.colour != colour {
            self.colour = colour;
            self.handle.update(|tray| tray.colour = colour);
        }
    }

    /// Whether a panel shows the item, so closing the window can hide it
    /// there.
    pub fn is_shown(&self) -> bool {
        self.shown.load(Ordering::SeqCst)
    }

    /// Nothing to do: the item lives on its own thread from the start.
    pub fn attach(&mut self, _activate: bool) {}

    /// Nothing to do either; see `attach`.
    pub fn hidden(&mut self) {}
}

/// Waits while the app lives in the tray without a window, for up to
/// `duration` or until something wakes the app. Linux has nothing to pump
/// here: the tray and MPRIS run on their own threads.
pub fn idle(duration: Duration, waker: &crate::backend::Waker) {
    waker.wait(duration);
}

/// Nothing to interrupt: [`idle`] waits on the waker itself.
pub fn interrupt_idle() {}

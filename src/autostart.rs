//! Starting Fastsonic when the person logs in, with its window or hidden in
//! the tray.
//!
//! Each platform keeps its own record of what starts at login, and the
//! person can change it there too (System Settings, Task Manager, the
//! desktop's own settings). [`Autostart::state`] reads that record back, so
//! nothing here keeps a copy that could disagree with it, and nothing goes
//! in `settings.json`.
//!
//! - **Linux**: an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/fastsonic.desktop`,
//!   replaced atomically. Inside a Flatpak, which cannot write the host's
//!   autostart folder, the background portal's `RequestBackground` registers
//!   the app, and the same entry in the sandbox's own configuration folder
//!   records what was asked, since the portal cannot be asked back.
//! - **macOS 13 and later**: `SMAppService`. Starting with the window is the
//!   app itself as a login item. Starting hidden is a launch agent the bundle
//!   carries, since a login item takes no arguments: [`macos_agent_plist`] is
//!   its text, at `Contents/Library/LaunchAgents/` [`macos_agent_plist_name`].
//!   The agent runs `open`, which launches the app through Launch Services
//!   and exits, so unregistering the agent (which kills its running job)
//!   never kills the app. A run outside an `.app` bundle and macOS 12 or
//!   earlier report [`State::Unsupported`].
//! - **Windows**: a value named after the app under
//!   `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the quoted
//!   executable and the hidden argument. An entry Task Manager's Startup
//!   apps turned off reads as [`State::Off`], and choosing it here again
//!   clears that.
//!
//! Every call reads or writes a file, the registry or D-Bus, and the portal
//! may wait for the person to answer it: the backend's runtime makes them,
//! never the UI thread.

use std::io;

mod command;
mod desktop;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// The argument a hidden start passes, which the command line takes.
pub const START_HIDDEN: &str = "--start-hidden";

/// How Fastsonic starts at login.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Start {
    /// With its window, as when the person opens it.
    Window,
    /// Without a window, in the tray: the command line carries
    /// [`START_HIDDEN`].
    Hidden,
}

/// What the platform has registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// This copy cannot start at login: a run outside an `.app` bundle or
    /// macOS 12 and earlier, no home folder, or another operating system.
    Unsupported,
    /// Nothing starts Fastsonic at login, or the person turned it off in the
    /// platform's own settings.
    Off,
    /// Fastsonic starts at login, this way.
    On(Start),
    /// Registered this way, but the person has to allow it in System
    /// Settings, Login Items, before it starts (macOS).
    NeedsApproval(Start),
}

impl State {
    /// How Fastsonic is registered to start, approved or not.
    pub fn start(self) -> Option<Start> {
        match self {
            Self::On(start) | Self::NeedsApproval(start) => Some(start),
            Self::Unsupported | Self::Off => None,
        }
    }
}

/// Fastsonic's registration to start at login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Autostart {
    /// The desktop entry's name without `.desktop`, which is also its icon's.
    id: String,
    /// The Windows Run value's name, and the entry's `Name`.
    name: String,
}

impl Default for Autostart {
    fn default() -> Self {
        Self {
            id: "fastsonic".to_owned(),
            name: "Fastsonic".to_owned(),
        }
    }
}

impl Autostart {
    /// What starts Fastsonic at login now, as the platform has it. Blocks on
    /// the file system, the registry or `SMAppService`.
    pub fn state(&self) -> State {
        #[cfg(target_os = "linux")]
        {
            linux::state(self)
        }
        #[cfg(target_os = "macos")]
        {
            macos::state()
        }
        #[cfg(windows)]
        {
            windows::state(self)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            State::Unsupported
        }
    }

    /// Starts Fastsonic at login this way, or not at all with `None`, for
    /// the running executable, and returns the state read back afterwards,
    /// which on macOS can be [`State::NeedsApproval`]. Blocks, and inside a
    /// Flatpak waits for the portal, which can ask the person first.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::Unsupported`] where [`state`](Self::state) is
    /// [`State::Unsupported`] (and on macOS for a hidden start when the
    /// bundle carries no agent), [`io::ErrorKind::PermissionDenied`] when
    /// the person or the portal refused, and the platform's own error when
    /// writing the entry fails.
    pub fn set(&self, start: Option<Start>) -> io::Result<State> {
        #[cfg(target_os = "linux")]
        {
            linux::set(self, start)
        }
        #[cfg(target_os = "macos")]
        {
            macos::set(start)
        }
        #[cfg(windows)]
        {
            windows::set(self, start)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = start;
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    /// The command line for `start`: `executable`, then [`START_HIDDEN`]
    /// for a hidden start.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn command(&self, executable: String, start: Start) -> Vec<String> {
        let mut command = vec![executable];
        if start == Start::Hidden {
            command.push(START_HIDDEN.to_owned());
        }
        command
    }
}

/// What the app asks of the login registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Read what the system has registered.
    Read,
    /// Register this way, or remove the registration with `None`.
    Set(Option<Start>),
}

impl Request {
    /// Carries the request out, on a thread that may wait.
    pub fn run(self) -> Result<State, String> {
        let autostart = Autostart::default();
        match self {
            Self::Read => Ok(autostart.state()),
            Self::Set(start) => autostart.set(start).map_err(|error| error.to_string()),
        }
    }
}

/// The Settings row's view of the login registration: the person's pick
/// shows at once, the next waits for the system's answer so two changes
/// never race, and a refusal puts back what the system has.
#[derive(Debug, Default)]
pub struct Registration {
    /// What the system has registered, as last read; `None` until the first
    /// answer.
    pub state: Option<State>,
    /// The person's latest pick, shown until the system confirms it.
    wanted: Option<Option<Start>>,
    /// A request is on its way; the next waits for its answer.
    busy: bool,
}

/// What [`Registration::answered`] decided.
#[derive(Debug, PartialEq, Eq)]
pub struct Answered {
    /// The request to send next, if any.
    pub next: Option<Request>,
    /// Why a change failed, to say so.
    pub failed: Option<String>,
}

impl Registration {
    /// How the row shows the registration: the person's pick while the
    /// system catches up, then what the system has.
    pub fn shown(&self) -> Option<Start> {
        match self.wanted {
            Some(wanted) => wanted,
            None => self.state.and_then(State::start),
        }
    }

    /// Whether the row is offered: once the system has answered, and only
    /// where something can be registered.
    pub fn available(&self) -> bool {
        self.state.is_some_and(|state| state != State::Unsupported)
    }

    /// A read of the registration, so a change made in the system's own
    /// settings shows, unless a change of ours is on its way.
    pub fn read(&mut self) -> Option<Request> {
        (!self.busy && self.wanted.is_none()).then(|| self.send(Request::Read))
    }

    /// The person picked `start`: shown at once, and sent unless a request
    /// is on its way, whose answer sends it.
    pub fn choose(&mut self, start: Option<Start>) -> Option<Request> {
        self.wanted = Some(start);
        (!self.busy).then(|| self.send(Request::Set(start)))
    }

    /// Takes the system's answer to `request`, and sends the person's newer
    /// pick if one came while it was on its way. A failure shows what the
    /// system really has, read again.
    pub fn answered(&mut self, request: Request, result: Result<State, String>) -> Answered {
        self.busy = false;
        match result {
            Ok(state) => {
                self.state = Some(state);
                if let Request::Set(asked) = request
                    && self.wanted == Some(asked)
                {
                    self.wanted = None;
                }
                Answered {
                    next: self.wanted.map(|wanted| self.send(Request::Set(wanted))),
                    failed: None,
                }
            }
            Err(error) => {
                self.wanted = None;
                Answered {
                    next: Some(self.send(Request::Read)),
                    failed: Some(error),
                }
            }
        }
    }

    fn send(&mut self, request: Request) -> Request {
        self.busy = true;
        request
    }
}

/// The file name of the launch agent that starts Fastsonic hidden, which
/// the bundle carries in `Contents/Library/LaunchAgents/`:
/// `<bundle id>.autostart.plist`.
pub fn macos_agent_plist_name(bundle_id: &str) -> String {
    format!("{bundle_id}.autostart.plist")
}

/// The launch agent that starts the app with `bundle_id` hidden at login,
/// by running `open -g -b <bundle id> --args --start-hidden`: Launch
/// Services launches the app in the background and `open` exits at once,
/// so the agent's job is never the app.
pub fn macos_agent_plist(bundle_id: &str) -> String {
    let id = xml_escape(bundle_id);
    let hidden = xml_escape(START_HIDDEN);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{id}.autostart</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-g</string>
        <string>-b</string>
        <string>{id}</string>
        <string>--args</string>
        <string>{hidden}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
</dict>
</plist>
"#
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Where a bundle at `bundle` carries the agent. Whether that file is there
/// is how to tell that the bundle can start hidden: `SMAppService` reports
/// an agent it never registered as not found too, so its status cannot say.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn macos_bundled_agent(bundle: &std::path::Path, bundle_id: &str) -> std::path::PathBuf {
    bundle
        .join("Contents/Library/LaunchAgents")
        .join(macos_agent_plist_name(bundle_id))
}

/// What `SMAppService` says about one service, the app's or the agent's.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServiceStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    /// No record of the service: a missing plist, but also an agent that
    /// was never registered.
    NotFound,
}

/// The state the app's login item and the hidden agent make together. The
/// agent wins if both are registered, since a change registers the new one
/// before it removes the old.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn service_state(main: ServiceStatus, agent: ServiceStatus) -> State {
    use ServiceStatus::{Enabled, RequiresApproval};
    match (main, agent) {
        (_, Enabled) => State::On(Start::Hidden),
        (Enabled, _) => State::On(Start::Window),
        (_, RequiresApproval) => State::NeedsApproval(Start::Hidden),
        (RequiresApproval, _) => State::NeedsApproval(Start::Window),
        _ => State::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registration_knows_how_it_starts() {
        assert_eq!(State::On(Start::Hidden).start(), Some(Start::Hidden));
        assert_eq!(
            State::NeedsApproval(Start::Window).start(),
            Some(Start::Window)
        );
        assert_eq!(State::Off.start(), None);
        assert_eq!(State::Unsupported.start(), None);
    }

    #[test]
    fn a_hidden_start_adds_the_argument_the_command_line_takes() {
        let autostart = Autostart::default();
        assert_eq!(
            autostart.command("/usr/bin/fastsonic".into(), Start::Hidden),
            ["/usr/bin/fastsonic", "--start-hidden"]
        );
        assert_eq!(
            autostart.command("/usr/bin/fastsonic".into(), Start::Window),
            ["/usr/bin/fastsonic"]
        );
    }

    #[test]
    fn the_services_say_together_how_the_app_starts() {
        use ServiceStatus::*;
        assert_eq!(service_state(NotRegistered, NotRegistered), State::Off);
        assert_eq!(service_state(NotRegistered, NotFound), State::Off);
        assert_eq!(service_state(NotFound, NotFound), State::Off);
        assert_eq!(
            service_state(Enabled, NotFound),
            State::On(Start::Window),
            "a bundle without the agent still opens its window at login"
        );
        assert_eq!(
            service_state(NotRegistered, Enabled),
            State::On(Start::Hidden)
        );
        assert_eq!(
            service_state(Enabled, Enabled),
            State::On(Start::Hidden),
            "halfway through a switch, the new one wins"
        );
        assert_eq!(
            service_state(RequiresApproval, NotRegistered),
            State::NeedsApproval(Start::Window)
        );
        assert_eq!(
            service_state(NotRegistered, RequiresApproval),
            State::NeedsApproval(Start::Hidden)
        );
        assert_eq!(
            service_state(Enabled, RequiresApproval),
            State::On(Start::Window),
            "an approved login item beats an agent still waiting"
        );
    }

    #[test]
    fn the_agent_is_named_after_the_bundle_and_lives_inside_it() {
        let id = "io.github.rwojsznis.fastsonic";
        assert_eq!(
            macos_agent_plist_name(id),
            "io.github.rwojsznis.fastsonic.autostart.plist"
        );
        assert_eq!(
            macos_bundled_agent(std::path::Path::new("/Applications/Fastsonic.app"), id),
            std::path::Path::new(
                "/Applications/Fastsonic.app/Contents/Library/LaunchAgents/io.github.rwojsznis.fastsonic.autostart.plist"
            )
        );
    }

    #[test]
    fn the_agent_opens_the_bundle_hidden_through_launch_services() {
        let plist = macos_agent_plist("io.github.rwojsznis.fastsonic");
        assert!(plist.contains("<string>io.github.rwojsznis.fastsonic.autostart</string>"));
        assert!(plist.contains(
            "<string>/usr/bin/open</string>\n        <string>-g</string>\n        \
             <string>-b</string>\n        <string>io.github.rwojsznis.fastsonic</string>\n        \
             <string>--args</string>\n        <string>--start-hidden</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(
            !plist.contains("KeepAlive"),
            "open exits; nothing restarts it"
        );
        assert!(macos_agent_plist("a&b").contains("<string>a&amp;b</string>"));
    }

    /// A pick shows at once and is sent; one made while the system answers
    /// waits for that answer, so two changes never race; the system's
    /// answer then is what the row shows.
    #[test]
    fn picks_are_sent_one_at_a_time() {
        let mut registration = Registration {
            state: Some(State::Off),
            ..Registration::default()
        };
        assert_eq!(
            registration.choose(Some(Start::Hidden)),
            Some(Request::Set(Some(Start::Hidden)))
        );
        assert_eq!(registration.shown(), Some(Start::Hidden), "shown at once");
        assert_eq!(
            registration.choose(Some(Start::Window)),
            None,
            "the second waits for the first"
        );
        assert_eq!(registration.shown(), Some(Start::Window));
        assert_eq!(registration.read(), None, "a change is on its way");
        let answered = registration.answered(
            Request::Set(Some(Start::Hidden)),
            Ok(State::On(Start::Hidden)),
        );
        assert_eq!(
            answered,
            Answered {
                next: Some(Request::Set(Some(Start::Window))),
                failed: None
            }
        );
        assert_eq!(
            registration.shown(),
            Some(Start::Window),
            "the newer pick still shows"
        );
        let answered = registration.answered(
            Request::Set(Some(Start::Window)),
            Ok(State::On(Start::Window)),
        );
        assert_eq!(answered.next, None);
        assert_eq!(registration.shown(), Some(Start::Window));
        // Opening Settings reads the system again, for changes made there.
        assert_eq!(registration.read(), Some(Request::Read));
        assert_eq!(registration.read(), None, "one read at a time");
        registration.answered(Request::Read, Ok(State::Off));
        assert_eq!(registration.shown(), None);
    }

    /// A refusal (the Flatpak portal, a missing agent) puts back what the
    /// system has, read again, and says why.
    #[test]
    fn a_failed_pick_shows_the_systems_state_again() {
        let mut registration = Registration {
            state: Some(State::Off),
            ..Registration::default()
        };
        registration.choose(Some(Start::Window));
        let answered =
            registration.answered(Request::Set(Some(Start::Window)), Err("not allowed".into()));
        assert_eq!(
            answered,
            Answered {
                next: Some(Request::Read),
                failed: Some("not allowed".into())
            }
        );
        assert_eq!(registration.shown(), None);
    }

    #[test]
    fn the_row_waits_for_an_answer_and_for_somewhere_to_register() {
        let mut registration = Registration::default();
        assert!(!registration.available());
        registration.state = Some(State::Unsupported);
        assert!(!registration.available());
        registration.state = Some(State::Off);
        assert!(registration.available());
    }
}

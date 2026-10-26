//! `SMAppService` (macOS 13 and later): the app as a login item for a start
//! with its window, the bundled agent for a hidden one.
//!
//! Asked through the Objective-C runtime rather than a binding crate: the
//! class, two constructors, a status and two calls are all this needs.

use std::io;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSBundle, NSError, NSString};

use super::{ServiceStatus, macos_agent_plist_name, macos_bundled_agent, service_state};
use super::{Start, State};

// SMAppService lives in ServiceManagement, which nothing else links.
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// The two services of a bundled app.
struct Services {
    main: Retained<AnyObject>,
    agent: Retained<AnyObject>,
    /// Whether the bundle carries the agent's plist.
    has_agent: bool,
}

impl Services {
    /// `None` outside an `.app` bundle, which has no login item to
    /// register, and before macOS 13, which has no `SMAppService`.
    fn new() -> Option<Self> {
        let class = AnyClass::get(c"SMAppService")?;
        let bundle = NSBundle::mainBundle();
        let path = bundle.bundlePath().to_string();
        if !path.ends_with(".app") {
            return None;
        }
        let id = bundle.bundleIdentifier()?.to_string();
        let has_agent = macos_bundled_agent(std::path::Path::new(&path), &id).is_file();
        let plist = NSString::from_str(&macos_agent_plist_name(&id));
        // SAFETY: the class exists (looked up above); `mainAppService` takes
        // nothing and `agentServiceWithPlistName:` takes any name, a missing
        // plist reporting `NotFound` as its status. Both return an object.
        let (main, agent): (Retained<AnyObject>, Retained<AnyObject>) = unsafe {
            (
                msg_send![class, mainAppService],
                msg_send![class, agentServiceWithPlistName: &*plist],
            )
        };
        Some(Self {
            main,
            agent,
            has_agent,
        })
    }
}

/// `SMAppServiceStatus`, from the framework's header.
fn status_from(raw: isize) -> ServiceStatus {
    match raw {
        1 => ServiceStatus::Enabled,
        2 => ServiceStatus::RequiresApproval,
        3 => ServiceStatus::NotFound,
        _ => ServiceStatus::NotRegistered,
    }
}

fn status(service: &AnyObject) -> ServiceStatus {
    // SAFETY: `status` is a plain NSInteger property of a live service.
    let raw: isize = unsafe { msg_send![service, status] };
    status_from(raw)
}

fn registered(status: ServiceStatus) -> bool {
    matches!(
        status,
        ServiceStatus::Enabled | ServiceStatus::RequiresApproval
    )
}

fn failure(error: &NSError) -> io::Error {
    io::Error::other(error.localizedDescription().to_string())
}

fn register(service: &AnyObject) -> io::Result<()> {
    if registered(status(service)) {
        return Ok(());
    }
    // SAFETY: a live service; the error comes back as a value.
    let result: Result<(), Retained<NSError>> =
        unsafe { msg_send![service, registerAndReturnError: _] };
    result.map_err(|error| failure(&error))
}

fn unregister(service: &AnyObject) -> io::Result<()> {
    if !registered(status(service)) {
        return Ok(());
    }
    // SAFETY: as in `register`. For the agent this kills its running job,
    // which is `open` and long gone, never the app.
    let result: Result<(), Retained<NSError>> =
        unsafe { msg_send![service, unregisterAndReturnError: _] };
    result.map_err(|error| failure(&error))
}

pub(super) fn state() -> State {
    match Services::new() {
        Some(services) => service_state(status(&services.main), status(&services.agent)),
        None => State::Unsupported,
    }
}

pub(super) fn set(start: Option<Start>) -> io::Result<State> {
    let services = Services::new().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "login items need the app bundle on macOS 13 or later",
        )
    })?;
    // The new registration first, so a failure leaves the old one working.
    match start {
        Some(Start::Hidden) => {
            // Not the agent's status: an agent never registered is not
            // found either, so that would refuse the first hidden start.
            if !services.has_agent {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the bundle carries no launch agent to start hidden",
                ));
            }
            register(&services.agent)?;
            unregister(&services.main)?;
        }
        Some(Start::Window) => {
            register(&services.main)?;
            unregister(&services.agent)?;
        }
        None => {
            unregister(&services.agent)?;
            unregister(&services.main)?;
        }
    }
    Ok(state())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_numbers_are_the_frameworks() {
        assert_eq!(status_from(0), ServiceStatus::NotRegistered);
        assert_eq!(status_from(1), ServiceStatus::Enabled);
        assert_eq!(status_from(2), ServiceStatus::RequiresApproval);
        assert_eq!(status_from(3), ServiceStatus::NotFound);
    }

    /// A test binary is no bundle: nothing is registered or asked.
    #[test]
    fn a_run_outside_the_bundle_cannot_start_at_login() {
        assert_eq!(state(), State::Unsupported);
        assert_eq!(
            set(None).map_err(|error| error.kind()),
            Err(io::ErrorKind::Unsupported)
        );
    }
}

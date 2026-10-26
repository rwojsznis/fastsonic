//! macOS: Low Power Mode, the network's reach, and App Nap activities.
//!
//! - `NSProcessInfo.isLowPowerModeEnabled` is read when the app asks for
//!   the conditions rather than followed through its notification: one
//!   message send, and no observer object to keep.
//! - SCNetworkReachability follows whether the internet can be reached, on
//!   the main dispatch queue, which the app's event loop runs (and the tray
//!   loop, while there is no window).
//! - An activity from `beginActivityWithOptions:reason:` keeps playback out
//!   of App Nap, and another with idle system sleep disabled keeps the Mac
//!   awake.
//! - The engine's thread asks for the user-interactive QoS class.
//!
//! Sleep and wake from NSWorkspace are not followed yet.
//! <https://developer.apple.com/documentation/foundation/processinfo/islowpowermodeenabled>
//! <https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability-g7d>

use std::ffi::c_void;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::sel;
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

use super::Shared;
use super::map::reachable;

/// The reachability watch, for as long as the `Power` lives. Low Power Mode
/// needs nothing registered: [`read_low_power`] reads it.
pub(super) struct Reader {
    _reachability: Option<Reachability>,
}

impl Reader {
    pub(super) fn start(shared: Arc<Shared>) -> Result<Self, String> {
        read_low_power(&shared);
        let reachability = Reachability::watch(shared);
        if reachability.is_none() {
            log::warn!("the network's reach is not followed");
        }
        Ok(Self {
            _reachability: reachability,
        })
    }
}

/// Reads Low Power Mode. It arrived in macOS 12, where asking an older
/// system would raise an exception, so the selector is checked first.
pub(super) fn read_low_power(shared: &Shared) {
    let info = NSProcessInfo::processInfo();
    if !info.respondsToSelector(sel!(isLowPowerModeEnabled)) {
        return;
    }
    let saver = info.isLowPowerModeEnabled();
    shared.update(|conditions| conditions.saver = saver);
}

/// An `NSProcessInfo` activity, ended when dropped.
pub(super) struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

// SAFETY: the token is an opaque object that is only ever handed back to
// `endActivity:`, once, in `drop`. NSProcessInfo is thread-safe (objc2 marks
// it Send and Sync), and retaining and releasing an object is atomic, so
// the token may be ended and released on any thread.
unsafe impl Send for Activity {}
// SAFETY: as above; a shared reference gives no access to the token.
unsafe impl Sync for Activity {}

impl Activity {
    pub(super) fn begin(reason: &str) -> Self {
        Self(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str(reason),
            ),
        )
    }

    /// An activity that only keeps the Mac from sleeping when idle: the
    /// display may still sleep, and App Nap is left to [`Activity::begin`].
    pub(super) fn keep_awake(reason: &str) -> Self {
        Self(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::IdleSystemSleepDisabled,
                &NSString::from_str(reason),
            ),
        )
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the token came from `beginActivityWithOptions:reason:` and
        // is ended exactly once, here, before its reference is released.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}

/// Puts the calling thread in the user-interactive QoS class, the class of
/// the frontmost app's main thread.
pub(super) fn raise_feeder() -> std::io::Result<()> {
    // SAFETY: changes only the calling thread's own QoS class, with a valid
    // class and a relative priority of zero, as the call allows.
    let result = unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(result))
    }
}

/// The calling thread's QoS class, for the tests.
#[cfg(test)]
pub(super) fn qos_class() -> libc::qos_class_t {
    let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
    let mut relative = 0;
    // SAFETY: reads the calling thread's own class into valid places.
    unsafe {
        libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative);
    }
    class
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkReachabilityCreateWithAddress(
        allocator: *const c_void,
        address: *const SockaddrIn,
    ) -> *mut c_void;
    fn SCNetworkReachabilitySetCallback(
        target: *mut c_void,
        callout: Option<extern "C" fn(*mut c_void, u32, *mut c_void)>,
        context: *mut ReachabilityContext,
    ) -> bool;
    fn SCNetworkReachabilitySetDispatchQueue(target: *mut c_void, queue: *const c_void) -> bool;
    fn SCNetworkReachabilityGetFlags(target: *mut c_void, flags: *mut u32) -> bool;
    /// The main dispatch queue (libdispatch, in libSystem).
    static _dispatch_main_q: [u8; 0];
}

/// `SCNetworkReachabilityContext`.
#[repr(C)]
struct ReachabilityContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
}

/// BSD's `struct sockaddr_in`, which starts with its own length.
#[repr(C)]
struct SockaddrIn {
    sin_len: u8,
    sin_family: u8,
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [u8; 8],
}

/// `AF_INET` (sys/socket.h).
const AF_INET: u8 = 2;

/// Writes what the flags say to `online`.
extern "C" fn on_reachability(_target: *mut c_void, flags: u32, info: *mut c_void) {
    if info.is_null() {
        return;
    }
    // SAFETY: `info` is the `Arc<Shared>` boxed in `watch`, which
    // SystemConfiguration releases (through `release_context`) only after
    // the callback is removed.
    let shared = unsafe { &*info.cast::<Arc<Shared>>() };
    shared.update(|conditions| conditions.online = reachable(flags));
}

unsafe extern "C" fn release_context(info: *const c_void) {
    if !info.is_null() {
        // SAFETY: `info` is the box `watch` leaked, released once.
        drop(unsafe { Box::from_raw(info.cast_mut().cast::<Arc<Shared>>()) });
    }
}

/// Follows whether the internet can be reached, and undoes it when dropped.
struct Reachability {
    target: *mut c_void,
}

// SAFETY: the target is a CoreFoundation object, whose reference counting
// is thread-safe, and it is only touched again in `drop`.
unsafe impl Send for Reachability {}
// SAFETY: as above; a shared reference gives no access to the target.
unsafe impl Sync for Reachability {}

impl Reachability {
    /// Watches the internet as a whole: the zero address stands for "any
    /// address out there", and needs no name looked up, so the flags say
    /// whether a route to the internet exists, whatever server the app
    /// talks to, and nothing is sent over the network to find out.
    fn watch(shared: Arc<Shared>) -> Option<Self> {
        let address = SockaddrIn {
            sin_len: std::mem::size_of::<SockaddrIn>() as u8,
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: 0,
            sin_zero: [0; 8],
        };
        // SAFETY: the address is a valid sockaddr_in for the call, which
        // copies it. Each failure below undoes what was done before it.
        unsafe {
            let target = SCNetworkReachabilityCreateWithAddress(std::ptr::null(), &address);
            if target.is_null() {
                return None;
            }
            let info = Box::into_raw(Box::new(Arc::clone(&shared)));
            let mut context = ReachabilityContext {
                version: 0,
                info: info.cast(),
                retain: None,
                release: Some(release_context),
                copy_description: None,
            };
            if !SCNetworkReachabilitySetCallback(target, Some(on_reachability), &mut context) {
                drop(Box::from_raw(info));
                CFRelease(target);
                return None;
            }
            if !SCNetworkReachabilitySetDispatchQueue(target, (&raw const _dispatch_main_q).cast())
            {
                // Removing the callback releases the context's box.
                SCNetworkReachabilitySetCallback(target, None, std::ptr::null_mut());
                CFRelease(target);
                return None;
            }
            let mut flags: u32 = 0;
            if SCNetworkReachabilityGetFlags(target, &mut flags) {
                shared.update(|conditions| conditions.online = reachable(flags));
            }
            Some(Self { target })
        }
    }
}

impl Drop for Reachability {
    fn drop(&mut self) {
        // SAFETY: the target came from `watch` and is released once, after
        // it leaves the queue and its callback (and with it the context's
        // box) is removed.
        unsafe {
            SCNetworkReachabilitySetDispatchQueue(self.target, std::ptr::null());
            SCNetworkReachabilitySetCallback(self.target, None, std::ptr::null_mut());
            CFRelease(self.target);
        }
    }
}

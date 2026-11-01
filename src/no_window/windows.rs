//! The dialog as a Win32 message box, which needs no window, no console and
//! no graphics driver.

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::SystemInformation::{IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_ARM64};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_SETFOREGROUND, MB_YESNO, MessageBoxW,
};
use windows_sys::core::BOOL;

use super::Dialog;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub(super) fn show(dialog: &Dialog) -> bool {
    let title = wide(dialog.title);
    let body = wide(&dialog.body);
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
    // a null owner makes the box a top-level window of its own.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONERROR | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}

type IsWow64Process2 =
    unsafe extern "system" fn(HANDLE, *mut IMAGE_FILE_MACHINE, *mut IMAGE_FILE_MACHINE) -> BOOL;

/// Whether Windows runs on an ARM64 processor, whatever this build's
/// architecture: an x64 build runs there emulated. Without the answer, the
/// build's own architecture decides.
pub(super) fn native_arm64() -> bool {
    native_machine().map_or(cfg!(target_arch = "aarch64"), |machine| {
        machine == IMAGE_FILE_MACHINE_ARM64
    })
}

/// The processor Windows runs on. `IsWow64Process2` is looked up at run
/// time: it arrived in Windows 10 1511, and a missing import would stop the
/// app from starting on anything older.
fn native_machine() -> Option<IMAGE_FILE_MACHINE> {
    let kernel32 = wide("kernel32.dll");
    // SAFETY: a NUL-terminated name; kernel32 is loaded in every process.
    let query = unsafe {
        let module = GetModuleHandleW(kernel32.as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, c"IsWow64Process2".as_ptr().cast())?
    };
    // SAFETY: the function has this signature (wow64apiset.h).
    let query = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, IsWow64Process2>(query)
    };
    let mut process: IMAGE_FILE_MACHINE = 0;
    let mut native: IMAGE_FILE_MACHINE = 0;
    // SAFETY: the pseudo-handle of this process and two writable u16s.
    let known = unsafe { query(GetCurrentProcess(), &mut process, &mut native) } != 0;
    known.then_some(native)
}

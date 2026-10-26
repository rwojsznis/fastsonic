//! The Run key under `HKEY_CURRENT_USER`, and Task Manager's switch for it.

use std::io;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};

use super::{Autostart, START_HIDDEN, Start, State, command};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Where Task Manager's Startup apps keeps an entry turned off.
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

pub(super) fn state(app: &Autostart) -> State {
    let Some(command) = read_string(RUN, &app.name) else {
        return State::Off;
    };
    if command::approval_disabled(read_binary(APPROVED, &app.name).as_deref()) {
        return State::Off;
    }
    command::run_state(&command, START_HIDDEN)
}

pub(super) fn set(app: &Autostart, start: Option<Start>) -> io::Result<State> {
    match start {
        Some(start) => {
            let executable = std::env::current_exe()?.to_string_lossy().into_owned();
            let command = command::windows_command(&app.command(executable, start));
            write_string(RUN, &app.name, &command)?;
        }
        None => delete(RUN, &app.name)?,
    }
    // Asked for here, so whatever Task Manager last said no longer holds:
    // an entry it turned off would otherwise stay off.
    delete(APPROVED, &app.name)?;
    Ok(state(app))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read(key: &str, value: &str, kind: u32) -> Option<Vec<u8>> {
    let (key, value) = (wide(key), wide(value));
    let mut size = 0u32;
    // SAFETY: NUL-terminated UTF-16 names; a null buffer asks for the size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            kind,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `size` is `data`'s length in bytes, and both outlive the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            kind,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then(|| {
        data.truncate(size as usize);
        data
    })
}

fn read_string(key: &str, value: &str) -> Option<String> {
    let bytes = read(key, value, RRF_RT_REG_SZ)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

fn read_binary(key: &str, value: &str) -> Option<Vec<u8>> {
    read(key, value, RRF_RT_REG_BINARY)
}

fn write_string(key: &str, value: &str, data: &str) -> io::Result<()> {
    let (key, value, data) = (wide(key), wide(value), wide(data));
    let bytes = u32::try_from(data.len() * 2).map_err(io::Error::other)?;
    // SAFETY: NUL-terminated UTF-16 strings; the byte length includes the
    // terminator, as REG_SZ requires. The key is created if missing.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            bytes,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

fn delete(key: &str, value: &str) -> io::Result<()> {
    let (key, value) = (wide(key), wide(value));
    // SAFETY: NUL-terminated UTF-16 strings.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

//! XDG autostart, and the background portal inside a Flatpak.

use std::io;
use std::path::{Path, PathBuf};

use super::{Autostart, START_HIDDEN, Start, State, desktop};

pub(super) fn state(app: &Autostart) -> State {
    match entry_path(app, config_home().as_deref()) {
        Some(path) => read(&path),
        None => State::Unsupported,
    }
}

pub(super) fn set(app: &Autostart, start: Option<Start>) -> io::Result<State> {
    let path = entry_path(app, config_home().as_deref())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no configuration folder"))?;
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let command = start.map(|start| app.command(executable, start));
    if std::env::var_os("FLATPAK_ID").is_some() {
        // The host's autostart folder is out of reach: the portal writes
        // the entry there. The sandbox's own copy below is the record the
        // state is read from, since the portal cannot be asked.
        portal::request(app, command.as_deref())?;
    }
    write(&path, app, command.as_deref())?;
    Ok(read(&path))
}

/// `$XDG_CONFIG_HOME`, or `~/.config` when it is unset or relative. Inside a
/// Flatpak this is the sandbox's own configuration folder.
fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })
}

fn entry_path(app: &Autostart, config: Option<&Path>) -> Option<PathBuf> {
    Some(
        config?
            .join("autostart")
            .join(format!("{}.desktop", app.id)),
    )
}

fn read(path: &Path) -> State {
    match std::fs::read_to_string(path) {
        Ok(text) => desktop::state(&text, START_HIDDEN),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                log::debug!("cannot read {}: {error}", path.display());
            }
            State::Off
        }
    }
}

/// Writes the entry that runs `command`, atomically, or removes it.
fn write(path: &Path, app: &Autostart, command: Option<&[String]>) -> io::Result<()> {
    match command {
        Some(command) => crate::util::write_file(path, desktop::entry(app, command)),
        None => match std::fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

mod portal {
    //! `org.freedesktop.portal.Background.RequestBackground`.

    use std::collections::HashMap;
    use std::io;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    use super::Autostart;

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";

    /// Asks the portal to start the sandbox's `command` at login, or not
    /// with `None`, and waits for its answer, which may follow a question to
    /// the person.
    pub(super) fn request(app: &Autostart, command: Option<&[String]>) -> io::Result<()> {
        let connection = Connection::session().map_err(io::Error::other)?;
        let token = format!("fastsonic_autostart_{}", std::process::id());
        let sender = connection
            .unique_name()
            .map(|name| super::desktop::request_sender(name.as_str()))
            .ok_or_else(|| io::Error::other("no unique bus name"))?;
        // Subscribe before asking, so a quick answer is not missed.
        let path = format!("{DESKTOP_PATH}/request/{sender}/{token}");
        let request = Proxy::new(
            &connection,
            DESKTOP,
            path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .map_err(io::Error::other)?;
        let mut responses = request
            .receive_signal("Response")
            .map_err(io::Error::other)?;
        let background = Proxy::new(
            &connection,
            DESKTOP,
            DESKTOP_PATH,
            "org.freedesktop.portal.Background",
        )
        .map_err(io::Error::other)?;
        let reason = format!("Start {} when you log in", app.name);
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("reason", Value::from(reason.as_str()));
        options.insert("autostart", Value::from(command.is_some()));
        if let Some(command) = command {
            options.insert("commandline", Value::from(command.to_vec()));
        }
        options.insert("dbus-activatable", Value::from(false));
        let _handle: OwnedObjectPath = background
            .call("RequestBackground", &("", options))
            .map_err(io::Error::other)?;
        let message = responses
            .next()
            .ok_or_else(|| io::Error::other("the portal did not answer"))?;
        let (code, results): (u32, HashMap<String, OwnedValue>) =
            message.body().deserialize().map_err(io::Error::other)?;
        let granted = results
            .get("autostart")
            .and_then(|value| bool::try_from(value).ok());
        super::desktop::portal_answer(code, granted, command.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_lives_in_the_autostart_folder_under_the_apps_id() {
        assert_eq!(
            entry_path(&Autostart::default(), Some(Path::new("/home/a/.config"))),
            Some(PathBuf::from("/home/a/.config/autostart/fastsonic.desktop"))
        );
        assert_eq!(entry_path(&Autostart::default(), None), None);
    }

    #[test]
    fn writing_reading_and_removing_the_entry_in_a_folder() -> io::Result<()> {
        let config = std::env::temp_dir().join(format!(
            "fastsonic-autostart-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let app = Autostart::default();
        let path = entry_path(&app, Some(&config)).expect("a path");
        assert_eq!(read(&path), State::Off, "nothing there yet");
        let hidden = app.command("/usr/bin/fastsonic".into(), Start::Hidden);
        write(&path, &app, Some(&hidden))?;
        assert_eq!(read(&path), State::On(Start::Hidden));
        let window = app.command("/usr/bin/fastsonic".into(), Start::Window);
        write(&path, &app, Some(&window))?;
        assert_eq!(read(&path), State::On(Start::Window));
        write(&path, &app, None)?;
        assert!(!path.exists());
        assert_eq!(read(&path), State::Off);
        write(&path, &app, None)?;
        let _ = std::fs::remove_dir_all(config);
        Ok(())
    }
}

//! The XDG autostart entry: its text, and the state it records. Pure, so the
//! tests run on every platform.

// Only Linux writes the entry; the tests read it everywhere.
#![cfg_attr(not(test), allow(dead_code))]

use std::io;

use super::{Autostart, Start, State, command};

/// The autostart entry that runs `command` at login.
pub(crate) fn entry(app: &Autostart, command: &[String]) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Exec={exec}\n\
         Icon={id}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        name = single_line(&app.name),
        exec = command::desktop_exec(command),
        id = single_line(&app.id),
    )
}

fn single_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// What an autostart entry's text says: off when it is hidden or disabled
/// (the way desktops turn one off without deleting it) or runs nothing, and
/// otherwise a hidden start when its command carries `hidden_argument`.
pub(crate) fn state(text: &str, hidden_argument: &str) -> State {
    let mut in_entry = false;
    let mut exec = None;
    let mut off = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (key.trim(), value.trim()) {
            ("Exec", value) => exec = Some(value.to_owned()),
            ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => off = true,
            _ => {}
        }
    }
    let Some(exec) = exec.filter(|_| !off) else {
        return State::Off;
    };
    let arguments = command::desktop_split(&exec);
    if arguments.is_empty() {
        State::Off
    } else if arguments[1..]
        .iter()
        .any(|argument| argument == hidden_argument)
    {
        State::On(Start::Hidden)
    } else {
        State::On(Start::Window)
    }
}

/// A request object's sender part: the unique bus name without its `:`,
/// with each `.` as `_`.
pub(crate) fn request_sender(unique_name: &str) -> String {
    unique_name.trim_start_matches(':').replace('.', "_")
}

/// The portal's answer: `code` 0 when it went through, 1 when the person
/// cancelled, 2 otherwise; `granted`, whether the app now starts at login.
pub(crate) fn portal_answer(code: u32, granted: Option<bool>, wanted: bool) -> io::Result<()> {
    match code {
        0 if !wanted || granted.unwrap_or(false) => Ok(()),
        0 | 1 => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "starting at login was not allowed",
        )),
        _ => Err(io::Error::other("the background portal failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fastsonic() -> Autostart {
        Autostart::default()
    }

    #[test]
    fn the_entry_runs_the_command_and_reads_back_how_it_starts() {
        let app = fastsonic();
        let hidden = entry(
            &app,
            &["/usr/bin/fastsonic".into(), "--start-hidden".into()],
        );
        assert_eq!(
            hidden,
            "[Desktop Entry]\nType=Application\nName=Fastsonic\n\
             Exec=/usr/bin/fastsonic --start-hidden\nIcon=fastsonic\n\
             Terminal=false\nX-GNOME-Autostart-enabled=true\n"
        );
        assert_eq!(state(&hidden, "--start-hidden"), State::On(Start::Hidden));
        let window = entry(&app, &["/opt/My Apps/fastsonic".into()]);
        assert!(window.contains("Exec=\"/opt/My Apps/fastsonic\"\n"));
        assert_eq!(state(&window, "--start-hidden"), State::On(Start::Window));
    }

    #[test]
    fn an_entry_the_desktop_turned_off_reads_as_off() {
        let app = fastsonic();
        let text = entry(&app, &["fastsonic".into()]);
        let disabled = text.replace(
            "X-GNOME-Autostart-enabled=true",
            "X-GNOME-Autostart-enabled=false",
        );
        assert_eq!(state(&disabled, "--start-hidden"), State::Off);
        assert_eq!(
            state(&format!("{text}Hidden=true\n"), "--start-hidden"),
            State::Off
        );
        assert_eq!(
            state("[Desktop Entry]\nType=Application\n", "--start-hidden"),
            State::Off
        );
        assert_eq!(state("", "--start-hidden"), State::Off);
    }

    #[test]
    fn only_the_entry_group_and_arguments_after_the_program_count() {
        let text = "[Desktop Entry]\nExec=/usr/bin/--start-hidden\n\
                    [Desktop Action quiet]\nExec=fastsonic --start-hidden\n";
        assert_eq!(state(text, "--start-hidden"), State::On(Start::Window));
    }

    #[test]
    fn a_flatpak_command_starts_hidden_with_its_argument() {
        let text = "[Desktop Entry]\nExec=flatpak run --command=fastsonic io.github.rwojsznis.Fastsonic --start-hidden\n";
        assert_eq!(state(text, "--start-hidden"), State::On(Start::Hidden));
    }

    #[test]
    fn a_name_cannot_break_the_entry() {
        let app = Autostart {
            id: "x".into(),
            name: "Two\nLines".into(),
        };
        assert!(entry(&app, &["x".into()]).contains("Name=Two Lines\n"));
    }

    #[test]
    fn the_request_path_follows_the_bus_name() {
        assert_eq!(request_sender(":1.42"), "1_42");
    }

    #[test]
    fn the_portals_answer_says_whether_it_went_through() {
        assert!(portal_answer(0, Some(true), true).is_ok());
        assert!(portal_answer(0, None, false).is_ok(), "turning it off");
        assert_eq!(
            portal_answer(0, Some(false), true).map_err(|e| e.kind()),
            Err(io::ErrorKind::PermissionDenied)
        );
        assert_eq!(
            portal_answer(1, None, true).map_err(|e| e.kind()),
            Err(io::ErrorKind::PermissionDenied)
        );
        assert_eq!(
            portal_answer(2, None, true).map_err(|e| e.kind()),
            Err(io::ErrorKind::Other)
        );
    }
}

//! Command lines as each platform's record keeps them: a desktop entry's
//! `Exec` key and a Windows Run value, with Task Manager's switch for it.
//! Pure, so their tests run everywhere.

// Each platform's module uses one half (none on macOS); the tests use both
// everywhere.
#![cfg_attr(not(test), allow(dead_code))]

use super::{Start, State};

/// `arguments` as a desktop entry's `Exec` value: an argument with anything
/// but plain path characters is quoted, with `"`, `` ` ``, `$` and `\`
/// escaped, and `%` doubled, since a single one starts a field code.
pub(crate) fn desktop_exec(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| desktop_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn desktop_quote(argument: &str) -> String {
    let plain = !argument.is_empty()
        && argument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+,=:".contains(c));
    if plain {
        return argument.to_owned();
    }
    let mut quoted = String::from('"');
    for c in argument.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        if c == '%' {
            quoted.push('%');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// The arguments of a desktop entry's `Exec` value, unquoted: the reverse of
/// [`desktop_exec`]. Field codes other than `%%` are dropped.
pub(crate) fn desktop_split(exec: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            // A field code such as %u is dropped: the launcher fills it in.
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                    started = true;
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started || !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
                started = false;
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started || !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

/// `executable` and `arguments` as a Run value: the executable always in
/// quotes (Windows tries each space-separated prefix of an unquoted path),
/// the arguments after it, each a plain word.
pub(crate) fn windows_command(arguments: &[String]) -> String {
    let mut parts = arguments.iter();
    let mut command = match parts.next() {
        Some(executable) => {
            // A canonical path's verbatim prefix is not needed, and the Run
            // key's own entries never carry it.
            let executable = executable.strip_prefix(r"\\?\").unwrap_or(executable);
            format!("\"{executable}\"")
        }
        None => String::new(),
    };
    for argument in parts {
        command.push(' ');
        command.push_str(argument);
    }
    command
}

/// The arguments after the executable in a Run value, which may or may not
/// quote the executable.
pub(crate) fn windows_arguments(command: &str) -> Vec<String> {
    let command = command.trim_start();
    let rest = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"').map_or("", |(_, rest)| rest)
    } else {
        command
            .split_once(char::is_whitespace)
            .map_or("", |(_, rest)| rest)
    };
    rest.split_whitespace()
        .map(|argument| argument.trim_matches('"').to_owned())
        .collect()
}

/// How a Run value starts the app.
pub(crate) fn run_state(command: &str, hidden_argument: &str) -> State {
    if command.trim().is_empty() {
        State::Off
    } else if windows_arguments(command)
        .iter()
        .any(|argument| argument == hidden_argument)
    {
        State::On(Start::Hidden)
    } else {
        State::On(Start::Window)
    }
}

/// Whether Task Manager turned the entry off: its value's first byte is odd
/// (3 or 7) for off and even (2 or 6) for on. No value is on.
pub(crate) fn approval_disabled(value: Option<&[u8]>) -> bool {
    value
        .and_then(|bytes| bytes.first())
        .is_some_and(|first| first & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(|&a| a.to_owned()).collect()
    }

    #[test]
    fn a_plain_path_stays_bare_in_a_desktop_entry() {
        assert_eq!(
            desktop_exec(&strings(&["/usr/bin/fastsonic", "--start-hidden"])),
            "/usr/bin/fastsonic --start-hidden"
        );
    }

    #[test]
    fn a_desktop_entry_quotes_and_escapes_what_needs_it() {
        let exec = desktop_exec(&strings(&[
            "/opt/Zap Fast/100%/zap\"fa$t",
            "--start-hidden",
        ]));
        assert_eq!(exec, "\"/opt/Zap Fast/100%%/zap\\\"fa\\$t\" --start-hidden");
        assert_eq!(
            desktop_split(&exec),
            ["/opt/Zap Fast/100%/zap\"fa$t", "--start-hidden"],
            "what is written reads back the same"
        );
    }

    #[test]
    fn a_desktop_entry_split_drops_field_codes_and_keeps_empty_quotes() {
        assert_eq!(desktop_split("fastsonic %u"), ["fastsonic"]);
        assert_eq!(desktop_split("  app   \"\"  x "), ["app", "", "x"]);
        assert_eq!(
            desktop_split(
                "flatpak run --command=fastsonic io.github.rwojsznis.Fastsonic --start-hidden"
            ),
            [
                "flatpak",
                "run",
                "--command=fastsonic",
                "io.github.rwojsznis.Fastsonic",
                "--start-hidden"
            ]
        );
    }

    #[test]
    fn a_run_value_quotes_the_executable() {
        assert_eq!(
            windows_command(&strings(&[
                r"\\?\C:\Program Files\Fastsonic\fastsonic.exe",
                "--start-hidden"
            ])),
            r#""C:\Program Files\Fastsonic\fastsonic.exe" --start-hidden"#
        );
        assert_eq!(
            windows_command(&strings(&[r"C:\Apps\fastsonic.exe"])),
            r#""C:\Apps\fastsonic.exe""#
        );
    }

    #[test]
    fn a_run_value_reads_back_its_arguments() {
        assert_eq!(
            windows_arguments(r#""C:\Program Files\Fastsonic\fastsonic.exe" --start-hidden"#),
            ["--start-hidden"]
        );
        assert!(windows_arguments(r#""C:\Program Files\Fastsonic\fastsonic.exe""#).is_empty());
        assert_eq!(
            windows_arguments(r"C:\Apps\fastsonic.exe --start-hidden"),
            ["--start-hidden"],
            "an entry someone wrote without quotes"
        );
        assert!(windows_arguments(r"C:\Apps\fastsonic.exe").is_empty());
    }

    #[test]
    fn a_run_value_says_how_the_app_starts() {
        let exe = r#""C:\Program Files\Fastsonic\fastsonic.exe""#;
        assert_eq!(
            run_state(&format!("{exe} --start-hidden"), "--start-hidden"),
            State::On(Start::Hidden)
        );
        assert_eq!(run_state(exe, "--start-hidden"), State::On(Start::Window));
        assert_eq!(run_state("  ", "--start-hidden"), State::Off);
    }

    #[test]
    fn task_manager_turns_an_entry_off_with_an_odd_first_byte() {
        assert!(!approval_disabled(None));
        assert!(!approval_disabled(Some(&[])));
        assert!(!approval_disabled(Some(&[
            2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ])));
        assert!(!approval_disabled(Some(&[6, 0, 0, 0])));
        assert!(approval_disabled(Some(&[
            3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8
        ])));
        assert!(approval_disabled(Some(&[7])));
    }
}

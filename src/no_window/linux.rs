//! The dialog through zenity or kdialog, whichever the desktop has. Without
//! either, the error stays in the log.
//!
//! Only [`show`] needs Linux; the arguments are built on every platform so
//! their tests run everywhere.

use super::Dialog;

#[cfg(target_os = "linux")]
pub(super) fn show(dialog: &Dialog) -> bool {
    use std::process::{Command, Stdio};

    let run = |program: &str, args: Vec<String>| {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .status()
    };
    // Both exit 0 for the first button, and 1 for the other or a close.
    // Anything else means zenity could not ask, so kdialog gets a turn.
    if let Ok(status) = run("zenity", zenity_args(dialog)) {
        match status.code() {
            Some(0) => return true,
            Some(1) => return false,
            _ => {}
        }
    }
    match run("kdialog", kdialog_args(dialog)) {
        Ok(status) => status.success(),
        Err(_) => {
            log::warn!("neither zenity nor kdialog could show the error");
            false
        }
    }
}

pub(super) fn zenity_args(dialog: &Dialog) -> Vec<String> {
    vec![
        "--question".to_owned(),
        "--width=440".to_owned(),
        format!("--ok-label={}", dialog.open_label),
        format!("--cancel-label={}", dialog.close_label),
        format!("--title={}", dialog.title),
        format!("--text={}", escape_markup(&dialog.body)),
    ]
}

/// kdialog names its buttons Yes and No, which the body's closing question
/// answers.
pub(super) fn kdialog_args(dialog: &Dialog) -> Vec<String> {
    vec![
        "--title".to_owned(),
        dialog.title.to_owned(),
        "--warningyesno".to_owned(),
        dialog.body.clone(),
    ]
}

/// zenity reads its text as Pango markup.
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::no_window::{COMPATIBILITY_PACK_PAGE, Cause, dialog};

    #[test]
    fn markup_characters_stay_text() {
        assert_eq!(escape_markup("a < b & c"), "a &lt; b &amp; c");
    }

    #[test]
    fn zenity_asks_open_page_or_close_with_the_body_as_text() {
        let arm = dialog(Cause::WindowsOnArm);
        let args = zenity_args(&arm);
        assert!(args.contains(&"--question".to_owned()));
        assert!(args.contains(&"--ok-label=Open page".to_owned()));
        assert!(args.contains(&"--cancel-label=Close".to_owned()));
        assert!(args.contains(&"--title=Fastsonic cannot open its window".to_owned()));
        let text = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--text="))
            .expect("the body");
        assert!(text.contains("OpenCL, OpenGL &amp; Vulkan Compatibility Pack"));
        assert!(!args.iter().any(|arg| arg.contains(COMPATIBILITY_PACK_PAGE)));
    }

    #[test]
    fn kdialog_asks_yes_or_no_with_the_plain_body() {
        let driver = dialog(Cause::NoGraphicsDriver);
        assert_eq!(
            kdialog_args(&driver),
            [
                "--title".to_owned(),
                "Fastsonic cannot open its window".to_owned(),
                "--warningyesno".to_owned(),
                driver.body.clone(),
            ]
        );
    }
}

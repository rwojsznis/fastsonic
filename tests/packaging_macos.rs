//! The macOS bundle carries the launch agent that starts Fastsonic in the
//! menu bar at login, exactly as `fastsonic::autostart` registers it.
use std::fs;
use std::path::PathBuf;

/// A packaging file as the bundle is built from it, on macOS, from LF
/// text: a Windows checkout turns the committed files into CRLF.
fn packaging(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("packaging/macos")
        .join(file);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .replace("\r\n", "\n")
}

#[test]
fn the_bundle_carries_the_agent_that_starts_in_the_menu_bar() {
    let id = "io.github.rwojsznis.fastsonic";
    assert!(
        packaging("Info.plist").contains(&format!(
            "<key>CFBundleIdentifier</key><string>{id}</string>"
        )),
        "the agent is named after the bundle identifier"
    );
    assert_eq!(
        packaging("autostart.plist"),
        fastsonic::autostart::macos_agent_plist(id),
        "the committed agent is the one the app expects"
    );
    let bundle = packaging("bundle.sh");
    assert!(bundle.contains(&format!("identifier={id}\n")));
    assert!(
        bundle.contains(
            r#""$here/autostart.plist" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist""#
        ),
        "bundle.sh copies it where SMAppService looks: {}",
        fastsonic::autostart::macos_agent_plist_name(id)
    );
}

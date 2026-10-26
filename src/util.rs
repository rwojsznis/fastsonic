//! Small helpers shared across the application.

/// `3:45` for track lengths, `1:02:03` past an hour.
pub fn format_duration_ms(ms: u32) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `2 hr 13 min` for playlist totals, `45 min 12 sec` under an hour.
pub fn format_total_ms(ms: u64) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours} hr {minutes} min")
    } else if minutes > 0 {
        format!("{minutes} min {seconds} sec")
    } else {
        format!("{seconds} sec")
    }
}

pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

/// `Jan 5, 2024` from an ISO-8601 timestamp or a bare date.
pub fn format_date(iso: &str) -> String {
    let date = iso.get(..10).unwrap_or(iso);
    let mut parts = date.split('-');
    let (Some(year), Some(month)) = (parts.next(), parts.next()) else {
        return iso.to_string();
    };
    let day = parts.next();
    let month_name = match month {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return iso.to_string(),
    };
    match day.and_then(|day| day.trim_start_matches('0').parse::<u8>().ok()) {
        Some(day) => format!("{month_name} {day}, {year}"),
        None => format!("{month_name} {year}"),
    }
}

/// `5 minutes ago` for recent ISO-8601 timestamps, otherwise the usual date.
///
/// Dates are shown relatively for their first 30 days, matching the playlist
/// table's compact, time-aware presentation. `now` is an argument so callers
/// can render against one instant and the boundary behaviour stays testable.
pub fn format_relative_date(iso: &str, now: jiff::Timestamp) -> String {
    let Ok(added) = iso.parse::<jiff::Timestamp>() else {
        return format_date(iso);
    };
    let seconds = added.duration_until(now).as_secs_f64().floor() as i64;
    if !(0..30 * 24 * 60 * 60).contains(&seconds) {
        return format_date(iso);
    }

    let (count, unit) = if seconds < 60 {
        (seconds, "second")
    } else if seconds < 60 * 60 {
        (seconds / 60, "minute")
    } else if seconds < 24 * 60 * 60 {
        (seconds / (60 * 60), "hour")
    } else if seconds < 7 * 24 * 60 * 60 {
        (seconds / (24 * 60 * 60), "day")
    } else {
        (seconds / (7 * 24 * 60 * 60), "week")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// Tears the id out of `sonic:track:abc` and friends.
pub fn uri_id(uri: &str) -> Option<&str> {
    uri.rsplit(':').next().filter(|id| !id.is_empty())
}

pub fn uri_kind(uri: &str) -> Option<&str> {
    let mut parts = uri.split(':');
    parts.next()?;
    parts.next()
}

/// The application icon, drawn at runtime: a green disc with a play mark.
/// Shared by the window icon and the tray pixmap.
/// The menu-bar shape for macOS: the circle with the play triangle punched
/// out. macOS template images use only the alpha channel and paint the
/// shape themselves, black in a light menu bar and white in a dark one.
pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    let mut rgba = app_icon_rgba(size);
    for pixel in rgba.as_chunks_mut::<4>().0 {
        // The triangle is the dark colour; make it a hole instead.
        if pixel[1] < 128 {
            pixel[3] = 0;
        }
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

/// The tray icon as Settings asks for it on Linux and Windows: the colour
/// icon, or the shape of the macOS menu-bar template painted white or
/// black, its transparency kept, to match the panel or taskbar.
pub fn tray_icon_rgba(choice: crate::settings::TrayIcon, size: usize) -> Vec<u8> {
    use crate::settings::TrayIcon;
    let rgb = match choice {
        TrayIcon::Colour => return app_icon_rgba(size),
        TrayIcon::White => [255, 255, 255],
        TrayIcon::Black => [0, 0, 0],
    };
    let mut rgba = tray_template_rgba(size);
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[..3].copy_from_slice(&rgb);
    }
    rgba
}

/// The mark rasterised to pixels for the window icon and the trays,
/// where no egui painter exists. This is deliberately the one separate
/// implementation of the logo; on-screen drawing goes through
/// `theme::logo` and `theme::play_glyph_offset` instead.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f32 / 2.0;
    let radius = center - 2.0;
    let scale = size as f32 / 128.0;
    let triangle = [
        (center - 12.0 * scale, center - 22.0 * scale),
        (center - 12.0 * scale, center + 22.0 * scale),
        (center + 26.0 * scale, center),
    ];
    let sign = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (a.0 - c.0) * (b.1 - c.1) - (b.0 - c.0) * (a.1 - c.1)
    };
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let d1 = sign((px, py), triangle[0], triangle[1]);
            let d2 = sign((px, py), triangle[1], triangle[2]);
            let d3 = sign((px, py), triangle[2], triangle[0]);
            let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            let inside = !(negative && positive);
            let (r, g, b) = if inside { (10, 20, 14) } else { (30, 215, 96) };
            let index = (y * size + x) * 4;
            rgba[index] = r;
            rgba[index + 1] = g;
            rgba[index + 2] = b;
            rgba[index + 3] = (coverage * 255.0) as u8;
        }
    }
    rgba
}

pub fn greeting() -> &'static str {
    match local_hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

fn local_hour() -> u8 {
    jiff::Zoned::now().hour() as u8
}

/// Strips HTML embedded in playlist descriptions.
pub fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for character in text.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

thread_local! {
    static UI_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the calling thread as the one that draws the window. Durable
/// writes on it are refused from then on; it hands them to a
/// [`crate::file_writer::FileWriter`] instead.
pub fn mark_ui_thread() {
    UI_THREAD.with(|ui| ui.set(true));
}

/// A durable write on the UI thread stops the window until the disk
/// answers. A debug or test build panics, so such a write fails the tests
/// instead of shipping; a release build logs it and still writes, rather
/// than lose what was saved.
fn refuse_ui_thread(path: &std::path::Path) {
    if !UI_THREAD.with(std::cell::Cell::get) {
        return;
    }
    let message = format!(
        "{} was written on the UI thread, which then waits for the disk; \
         hand it to the file writer",
        path.display()
    );
    if cfg!(debug_assertions) {
        panic!("{message}");
    }
    log::error!("{message}");
}

/// Who may read a file [`write_with`] replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    /// The system's default for a new file.
    #[default]
    Default,
    /// Its owner only (0600 on Unix). Windows keeps a user's profile
    /// private already, so there it is the same as `Default`.
    Private,
}

/// Replaces `path` with `contents`, creating its folder if needed. See
/// [`write_with`].
pub fn write_file(path: &std::path::Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_with(path, Access::Default, |file| {
        std::io::Write::write_all(file, contents.as_ref())
    })
}

/// Replaces `path` with `contents` only its owner may read.
pub fn write_private(path: &std::path::Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_with(path, Access::Private, |file| {
        std::io::Write::write_all(file, contents.as_ref())
    })
}

/// Replaces `path` with what `fill` writes into a fresh file, so a reader,
/// or the next start after a crash or a power cut, sees the old contents
/// or the new ones and never part of either.
///
/// The new file is written beside the target under a name no other write
/// uses, in this process or another, synced to the disk before it moves
/// over the target (written through on Windows), and then the folder is
/// synced so the move itself lasts (Unix). If anything fails, `path` keeps
/// its old contents and the temporary file is removed. Syncing can take a
/// while on a busy disk, so the UI thread hands its writes to a
/// [`crate::file_writer::FileWriter`] instead, and is refused here once
/// [`mark_ui_thread`] has marked it.
pub fn write_with(
    path: &std::path::Path,
    access: Access,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    refuse_ui_thread(path);
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => std::path::Path::new("."),
    };
    std::fs::create_dir_all(folder)?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = create_new(&temporary, access)?;
        fill(&mut file)?;
        std::io::Write::flush(&mut file)?;
        // The contents reach the disk before the move, or a crash just after
        // it could leave the new name pointing at nothing.
        file.sync_all()?;
        // Windows moves only a closed file.
        drop(file);
        replace_file(&temporary, path)?;
        sync_folder(folder)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// A name beside `path` that no other write uses at the same time:
/// `.<name>.<pid>.<n>.tmp`, hidden from a folder listing while it lives.
fn temporary_path(path: &std::path::Path) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

/// A new file, never one already there, so the write cannot go through a
/// link someone left at the temporary name.
fn create_new(path: &std::path::Path, access: Access) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if access == Access::Private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = access;
    options.open(path)
}

/// Makes the move itself durable: on Unix a rename lives in the folder,
/// which has to reach the disk too. Windows's write-through move covers it.
#[cfg(unix)]
fn sync_folder(folder: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(folder)?.sync_all()
}

#[cfg(not(unix))]
fn sync_folder(_folder: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// The temporary files a replacement left in `dir`, which a finished or
/// failed one never does.
#[cfg(test)]
pub(crate) fn temporary_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

/// Moves `temporary` over `path` in one step.
#[cfg(not(windows))]
fn replace_file(temporary: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(temporary, path)
}

/// Moves `temporary` over `path` in one step, written through to the disk.
#[cfg(windows)]
fn replace_file(temporary: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colour is the app icon; white and black paint the menu-bar
    /// template's shape, keeping its transparency, so the play triangle
    /// stays a hole.
    #[test]
    fn white_and_black_tray_icons_paint_the_template_shape() {
        use crate::settings::TrayIcon;
        assert_eq!(tray_icon_rgba(TrayIcon::Colour, 32), app_icon_rgba(32));
        let template = tray_template_rgba(32);
        for (choice, rgb) in [(TrayIcon::White, [255; 3]), (TrayIcon::Black, [0; 3])] {
            let painted = tray_icon_rgba(choice, 32);
            let (pixels, _) = painted.as_chunks::<4>();
            let (shape, _) = template.as_chunks::<4>();
            for (pixel, outline) in pixels.iter().zip(shape) {
                assert_eq!(pixel[..3], rgb);
                assert_eq!(pixel[3], outline[3], "the shape stays");
            }
        }
        let centre = (16 * 32 + 16) * 4 + 3;
        assert_eq!(tray_icon_rgba(TrayIcon::White, 32)[centre], 0, "a hole");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "fastsonic-write-{name}-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn a_file_is_written_and_then_replaced_whole() {
        let dir = scratch("whole");
        let path = dir.join("nested/settings.json");
        write_file(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        write_file(&path, "second, longer").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second, longer");
        write_file(&path, "3").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "3", "no tail left");
        assert!(temporary_files(path.parent().unwrap()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_write_keeps_the_old_contents_and_leaves_nothing_behind() {
        let dir = scratch("failed");
        let path = dir.join("session.json");
        write_file(&path, "kept").unwrap();
        let failed = write_with(&path, Access::Default, |file| {
            std::io::Write::write_all(file, b"half")?;
            Err(std::io::Error::other("the encoder gave up"))
        });
        assert_eq!(failed.unwrap_err().to_string(), "the encoder gave up");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
        assert!(temporary_files(&dir).is_empty());

        // A folder where the file should be cannot be replaced.
        let blocked = dir.join("blocked.json");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_file(&blocked, "new").is_err());
        assert!(temporary_files(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_at_the_same_time_never_share_a_temporary_file() {
        let path = std::path::Path::new("/x/settings.json");
        let (a, b) = (temporary_path(path), temporary_path(path));
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".settings.json.") && name.ends_with(".tmp"));
        assert_eq!(a.parent(), path.parent(), "beside the file, on its disk");
        assert_eq!(
            temporary_path(std::path::Path::new("x")).parent(),
            Some(std::path::Path::new("")),
            "a bare name is written in the current folder"
        );
    }

    /// A durable write on the UI thread fails tests, so a save that would
    /// freeze the window cannot come back unnoticed.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "UI thread")]
    fn the_ui_thread_cannot_wait_for_the_disk() {
        mark_ui_thread();
        let _ = write_file(&scratch("ui-thread").join("settings.json"), "{}");
    }

    /// A release build logs the mistake but still writes, rather than lose
    /// what was saved.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_still_writes_on_the_ui_thread() {
        mark_ui_thread();
        let dir = scratch("ui-thread");
        let path = dir.join("settings.json");
        write_file(&path, "{}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("private");
        let path = dir.join("credentials.json");
        // Over a file others could read, as an older version wrote it.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration_ms(225_000), "3:45");
        assert_eq!(format_duration_ms(3_723_000), "1:02:03");
        assert_eq!(format_total_ms(7_980_000), "2 hr 13 min");
        assert_eq!(format_total_ms(2_712_000), "45 min 12 sec");
    }

    #[test]
    fn counts_and_dates() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_count(12), "12");
        assert_eq!(format_date("2024-01-05T10:00:00Z"), "Jan 5, 2024");
        assert_eq!(format_date("2024-03"), "Mar 2024");
        assert_eq!(format_date("2024"), "2024");
    }

    #[test]
    fn recent_dates_are_relative_for_the_first_month() {
        let now: jiff::Timestamp = "2026-08-31T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_date("2026-08-31T11:59:30Z", now),
            "30 seconds ago"
        );
        assert_eq!(
            format_relative_date("2026-08-31T11:59:00Z", now),
            "1 minute ago"
        );
        assert_eq!(
            format_relative_date("2026-08-31T11:00:00Z", now),
            "1 hour ago"
        );
        assert_eq!(
            format_relative_date("2026-08-30T12:00:00Z", now),
            "1 day ago"
        );
        assert_eq!(
            format_relative_date("2026-08-17T12:00:00Z", now),
            "2 weeks ago"
        );
        assert_eq!(
            format_relative_date("2026-08-01T12:00:00Z", now),
            "Aug 1, 2026"
        );
    }

    #[test]
    fn relative_dates_fall_back_for_future_and_invalid_timestamps() {
        let now: jiff::Timestamp = "2026-08-31T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_date("2026-09-01T12:00:00Z", now),
            "Sep 1, 2026"
        );
        assert_eq!(format_relative_date("not-a-date", now), "not-a-date");
    }

    #[test]
    fn uris() {
        assert_eq!(uri_id("sonic:track:abc"), Some("abc"));
        assert_eq!(uri_kind("sonic:playlist:x"), Some("playlist"));
    }

    #[test]
    fn html_is_stripped() {
        assert_eq!(
            strip_html("Hi <a href=\"x\">there</a> &amp; you"),
            "Hi there & you"
        );
        assert_eq!(strip_html("ONE&#x2F;TWO&#x2F;THREE"), "ONE/TWO/THREE");
    }
}

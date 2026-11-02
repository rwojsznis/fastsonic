//! What Settings > Storage shows beside each thing it can clear: how much
//! is on the disk. Measured on the backend runtime, never on the UI thread,
//! and only while the section is on screen.

use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::paths::AppDirs;

/// How often sizes on screen are measured again.
const REFRESH_AFTER: Duration = Duration::from_secs(5);

/// One size beside a row of Settings > Storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    #[default]
    Checking,
    Bytes(u64),
    /// The folder or the file could not be measured: an incomplete sum would
    /// say less than is there.
    Unavailable,
}

impl From<io::Result<u64>> for Size {
    fn from(result: io::Result<u64>) -> Self {
        result.map_or(Self::Unavailable, Self::Bytes)
    }
}

impl Size {
    pub fn label(self) -> String {
        match self {
            Self::Checking => "Checking…".to_owned(),
            Self::Bytes(bytes) => format_bytes(bytes),
            Self::Unavailable => "Unavailable".to_owned(),
        }
    }
}

/// The artwork cache, the audio cache and the play history, in that order.
pub type Sizes = [Size; 3];

/// The sizes on screen, and which measurement may replace them. A clear
/// starts a new generation: a measurement asked for before it answers with
/// the old sizes and is dropped, so a cleared size never comes back.
#[derive(Debug, Default)]
pub struct Usage {
    pub sizes: Sizes,
    generation: u64,
    /// The generation of the measurement under way.
    asked: Option<u64>,
    checked: Option<Instant>,
}

impl Usage {
    /// Something was cleared: what is shown is out of date.
    pub fn invalidate(&mut self) {
        self.sizes = Sizes::default();
        self.checked = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The generation to measure now, when a measurement is due: none for
    /// this generation is under way, and none has been made since the last
    /// clear or for `REFRESH_AFTER`.
    pub fn due(&mut self, now: Instant) -> Option<u64> {
        if self.asked == Some(self.generation)
            || self
                .checked
                .is_some_and(|checked| now.saturating_duration_since(checked) < REFRESH_AFTER)
        {
            return None;
        }
        self.asked = Some(self.generation);
        Some(self.generation)
    }

    /// A measurement answered. One of an earlier generation is dropped.
    pub fn measured(&mut self, generation: u64, sizes: Sizes, now: Instant) {
        if generation != self.generation {
            return;
        }
        self.asked = None;
        self.sizes = sizes;
        self.checked = Some(now);
    }

    /// How long until the sizes on screen are measured again. While a
    /// measurement is under way its answer wakes the window instead.
    pub fn refresh_in(&self, now: Instant) -> Duration {
        if self.asked == Some(self.generation) {
            return REFRESH_AFTER;
        }
        self.checked.map_or(REFRESH_AFTER, |checked| {
            REFRESH_AFTER.saturating_sub(now.saturating_duration_since(checked))
        })
    }
}

/// The sizes Settings > Storage shows. Waits for the disk.
pub fn measure_all(dirs: &AppDirs) -> Sizes {
    [
        // Clear artwork removes only the files directly in its folder.
        measure(&dirs.art_cache_dir(), false).into(),
        measure(&dirs.audio_cache_dir(), true).into(),
        measure(&dirs.history_file(), false).into(),
    ]
}

/// The lengths of the regular files at `path`, in its folders too when
/// `recursive`. Links inside are not followed. A missing file is empty.
/// File contents are never read, so a file that cannot be read still has a
/// size.
fn measure(path: &Path, recursive: bool) -> io::Result<u64> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0_u64;
    let mut folders = vec![path.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            } else if recursive && kind.is_dir() {
                folders.push(entry.path());
            }
        }
    }
    Ok(total)
}

/// Empties `root`, keeping the folder itself, and returns how many bytes
/// went. For the audio folder when no cache owns it: audio caching is
/// switched off, or the cache could not open. Links inside are removed,
/// never followed. Waits for the disk.
pub fn clear_folder(root: &Path) -> io::Result<u64> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut freed = 0_u64;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let bytes = measure(&path, true)?;
            std::fs::remove_dir_all(&path)?;
            freed = freed.saturating_add(bytes);
        } else {
            let bytes = if kind.is_file() {
                entry.metadata()?.len()
            } else {
                0
            };
            std::fs::remove_file(&path).or_else(|error| {
                // Windows removes a link to a folder as a folder.
                if kind.is_symlink() {
                    std::fs::remove_dir(&path)
                } else {
                    Err(error)
                }
            })?;
            freed = freed.saturating_add(bytes);
        }
    }
    Ok(freed)
}

/// Decimal units, as a file manager shows them: 1 MB is 1,000,000 bytes.
/// A file of a few bytes says so rather than 0.0 kB.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1_000 {
        return format!("{bytes} B");
    }
    let units = ["kB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64 / 1_000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < units.len() {
        value /= 1_000.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("fastsonic-storage-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_empty_and_nested_storage() {
        let dir = TestDir::new();
        let root = dir.path();
        assert_eq!(measure(&root.join("missing"), true).unwrap(), 0);
        assert_eq!(measure(root, true).unwrap(), 0);
        std::fs::write(root.join("cover"), [0; 13]).unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/track"), [0; 27]).unwrap();
        assert_eq!(measure(root, false).unwrap(), 13);
        assert_eq!(measure(root, true).unwrap(), 40);
        assert_eq!(measure(&root.join("cover"), false).unwrap(), 13);
    }

    #[test]
    fn the_three_sizes_are_the_artwork_the_audio_and_the_history() {
        let dir = TestDir::new();
        let dirs = AppDirs::inside(dir.path());
        std::fs::create_dir_all(dirs.art_cache_dir().join("ignored")).unwrap();
        std::fs::write(dirs.art_cache_dir().join("cover"), [0; 13]).unwrap();
        std::fs::write(dirs.art_cache_dir().join("ignored/cover"), [0; 100]).unwrap();
        std::fs::create_dir_all(dirs.audio_cache_dir().join("song")).unwrap();
        std::fs::write(dirs.audio_cache_dir().join("song/00000000"), [0; 27]).unwrap();
        assert_eq!(
            measure_all(&dirs),
            [Size::Bytes(13), Size::Bytes(27), Size::Bytes(0)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_do_not_count_their_targets_or_loop() {
        use std::os::unix::fs::symlink;
        let dir = TestDir::new();
        let root = dir.path().join("audio");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), [0; 13]).unwrap();
        let outside = dir.path().join("outside");
        std::fs::write(&outside, [0; 50]).unwrap();
        symlink(&outside, root.join("alias")).unwrap();
        symlink(&root, root.join("loop")).unwrap();
        symlink(root.join("missing"), root.join("broken")).unwrap();
        assert_eq!(measure(&root, true).unwrap(), 13);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_unavailable_rather_than_a_partial_total() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let nested = dir.path().join("nested");
        std::fs::write(dir.path().join("readable"), [0; 13]).unwrap();
        std::fs::create_dir(&nested).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000)).unwrap();
        let access = std::fs::read_dir(&nested);
        let measured = Size::from(measure(dir.path(), true));
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
        match access {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                assert_eq!(measured, Size::Unavailable);
            }
            // A privileged runner can read the folder whatever its mode.
            Ok(_) => assert_eq!(measured, Size::Bytes(13)),
            Err(error) => panic!("unexpected folder access error: {error}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_size_does_not_need_the_files_contents() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let file = dir.path().join("unreadable");
        std::fs::write(&file, [0; 13]).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        let measured = measure(&file, false);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(measured.unwrap(), 13);
    }

    #[test]
    fn clearing_a_folder_keeps_it_and_its_siblings() {
        let dir = TestDir::new();
        let audio = dir.path().join("audio");
        assert_eq!(clear_folder(&audio).unwrap(), 0, "nothing there yet");
        std::fs::create_dir_all(audio.join("nested")).unwrap();
        std::fs::write(audio.join("nested/track"), [0; 27]).unwrap();
        std::fs::write(audio.join("loose"), [0; 3]).unwrap();
        let history = dir.path().join("history.json");
        std::fs::write(&history, "[]").unwrap();
        assert_eq!(clear_folder(&audio).unwrap(), 30);
        assert!(audio.is_dir());
        assert_eq!(std::fs::read_dir(&audio).unwrap().count(), 0);
        assert_eq!(std::fs::read_to_string(history).unwrap(), "[]");
    }

    #[cfg(unix)]
    #[test]
    fn clearing_a_folder_removes_links_without_following_them() {
        use std::os::unix::fs::symlink;
        let dir = TestDir::new();
        let audio = dir.path().join("audio");
        std::fs::create_dir(&audio).unwrap();
        let outside = dir.path().join("keep");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("file"), [0; 13]).unwrap();
        symlink(outside.join("file"), audio.join("link")).unwrap();
        symlink(&outside, audio.join("folder-link")).unwrap();
        assert_eq!(clear_folder(&audio).unwrap(), 0);
        assert_eq!(std::fs::read_dir(&audio).unwrap().count(), 0);
        assert!(outside.join("file").exists());
    }

    #[test]
    fn a_measurement_from_before_a_clear_is_dropped() {
        let now = Instant::now();
        let mut usage = Usage::default();
        let first = usage.due(now).unwrap();
        assert_eq!(usage.due(now), None, "one measurement at a time");
        usage.invalidate();
        let second = usage.due(now).expect("a clear measures again at once");
        usage.measured(first, [Size::Bytes(100); 3], now);
        assert_eq!(usage.sizes, [Size::Checking; 3]);
        usage.measured(second, [Size::Bytes(0); 3], now);
        assert_eq!(usage.sizes, [Size::Bytes(0); 3]);
    }

    #[test]
    fn sizes_on_screen_are_measured_again_every_five_seconds() {
        let now = Instant::now();
        let mut usage = Usage::default();
        assert_eq!(usage.refresh_in(now), REFRESH_AFTER);
        let generation = usage.due(now).unwrap();
        usage.measured(generation, [Size::Bytes(1); 3], now);
        let later = now + Duration::from_secs(3);
        assert_eq!(usage.due(later), None);
        assert_eq!(usage.refresh_in(later), Duration::from_secs(2));
        assert_eq!(usage.refresh_in(now + REFRESH_AFTER), Duration::ZERO);
        assert!(usage.due(now + REFRESH_AFTER).is_some());
        assert_eq!(
            usage.refresh_in(now + REFRESH_AFTER * 2),
            REFRESH_AFTER,
            "no repaint loop while the answer is on its way"
        );
    }

    #[test]
    fn errors_are_unavailable_instead_of_zero() {
        assert_eq!(
            Size::from(Err(io::Error::other("unreadable"))),
            Size::Unavailable
        );
        assert_eq!(Size::from(Ok(0)), Size::Bytes(0));
        assert_eq!(Size::Unavailable.label(), "Unavailable");
        assert_eq!(Size::Checking.label(), "Checking…");
    }

    #[test]
    fn readable_sizes_include_small_files_and_unit_boundaries() {
        for (bytes, expected) in [
            (0, "0 B"),
            (2, "2 B"),
            (999, "999 B"),
            (1_000, "1.0 kB"),
            (999_999, "1.0 MB"),
            (12_500_000, "12.5 MB"),
            (4_000_000_000, "4.0 GB"),
            (u64::MAX, "18.4 EB"),
        ] {
            assert_eq!(format_bytes(bytes), expected);
        }
    }
}

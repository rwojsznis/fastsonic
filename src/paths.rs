//! Where Fastsonic keeps its files.
//!
//! Configuration, durable state (the music server's credential), and
//! disposable caches (audio, artwork) live in the platform's conventional
//! directories, so clearing a cache never signs the user out and a config
//! backup never contains a credential. A copy unpacked from the Windows
//! zip keeps them in `fastsonic-data` next to its executable instead, so
//! the folder carries the app and everything it remembers.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// The file the Windows zip carries beside `fastsonic.exe`, and what it
/// says: a copy beside it is portable.
const PORTABLE_MARKER: (&str, &str) = ("fastsonic-portable.txt", "fastsonic-portable-v1");
/// A portable copy's own folder, beside its executable.
const DATA_FOLDER: &str = "fastsonic-data";
/// Entries of the state folder a new portable folder does not copy: the
/// logs of the installed copy's runs.
const NOT_COPIED: &[&str] = &["fastsonic.log", "panic.log"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

/// The folders [`AppDirs::discover`] chose, and what to say about them once
/// logging has started.
pub struct Found {
    pub dirs: AppDirs,
    line: Option<(log::Level, String)>,
}

impl Found {
    /// Folders chosen elsewhere, such as a demo's, with nothing to say.
    pub fn given(dirs: AppDirs) -> Self {
        Self { dirs, line: None }
    }

    pub fn log(&self) {
        if let Some((level, line)) = &self.line {
            log::log!(*level, "{line}");
        }
    }
}

impl AppDirs {
    /// The folders this copy uses: a portable Windows copy's own, made the
    /// first time from a copy of the per-user settings and state, or the
    /// per-user ones. Before the window opens: the first copy waits for the
    /// disk.
    pub fn discover() -> Found {
        let installed = Self::installed();
        let portable = if cfg!(windows) {
            std::env::current_exe().map_or(Portable::No, |executable| {
                portable_for(&executable, |path| fs::read_to_string(path).ok())
            })
        } else {
            Portable::No
        };
        let (dirs, mode) = settle(installed, portable);
        let line = describe(&dirs, &mode);
        Found { dirs, line }
    }

    /// `config`, `state` and `cache` inside `root`: a portable data folder.
    pub fn inside(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        }
    }

    /// The platform's per-user folders, which an installed copy uses and a
    /// new portable folder copies from.
    pub fn installed() -> Self {
        let project = ProjectDirs::from("io", "github.rwojsznis", "fastsonic");
        match project {
            Some(project) => Self {
                config: project.config_dir().to_path_buf(),
                state: project
                    .state_dir()
                    .map(|path| path.to_path_buf())
                    .unwrap_or_else(|| project.data_local_dir().to_path_buf()),
                cache: project.cache_dir().to_path_buf(),
            },
            None => {
                let fallback = std::env::current_dir().unwrap_or_default();
                Self {
                    config: fallback.join("fastsonic-config"),
                    state: fallback.join("fastsonic-state"),
                    cache: fallback.join("fastsonic-cache"),
                }
            }
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Winamp skins the listener has added, as `.wsz` files or folders.
    pub fn skins_dir(&self) -> PathBuf {
        self.config.join("skins")
    }

    /// MilkDrop presets, as `.milk` files, in folders or not, with any
    /// textures they use in a `textures` folder inside.
    pub fn milkdrop_dir(&self) -> PathBuf {
        self.config.join("milkdrop")
    }

    /// Palette files, as `.json`, listed in Settings > Appearance > Theme.
    pub fn themes_dir(&self) -> PathBuf {
        self.config.join("themes")
    }

    pub fn session_file(&self) -> PathBuf {
        self.state.join("session.json")
    }

    /// What was played here, kept locally as well as scrobbled. See
    /// [`crate::history`].
    pub fn history_file(&self) -> PathBuf {
        self.state.join("history.json")
    }

    /// The salted-token pair for the music server, and Navidrome's own
    /// session beside it. Durable state rather than configuration: clearing
    /// a cache must never sign anybody out, and a copied `settings.json`
    /// must never carry a credential (D10).
    pub fn credentials_file(&self) -> PathBuf {
        self.state.join("credentials.json")
    }

    /// The log of the current run, replaced at every start.
    pub fn log_file(&self) -> PathBuf {
        self.state.join("fastsonic.log")
    }

    /// Where a panic is recorded before the process dies of it.
    pub fn panic_log(&self) -> PathBuf {
        self.state.join("panic.log")
    }

    pub fn audio_cache_dir(&self) -> PathBuf {
        self.cache.join("audio")
    }

    pub fn art_cache_dir(&self) -> PathBuf {
        self.cache.join("art")
    }

    pub fn lyrics_cache_dir(&self) -> PathBuf {
        self.cache.join("lyrics")
    }

    pub fn playlist_cache_dir(&self) -> PathBuf {
        self.cache.join("playlists")
    }

    pub fn account_playlist_cache_dir(&self, account_id: &str) -> PathBuf {
        self.playlist_cache_dir().join(account_id)
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.config, &self.state, &self.cache] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// What the executable's folder says about this copy.
#[derive(Debug)]
enum Portable {
    /// Not a portable copy: an installed one, or not on Windows.
    No,
    /// A portable copy whose data folder can be written. It may not exist
    /// yet.
    Folder(PathBuf),
    /// A portable copy that cannot write its data folder.
    Unwritable { folder: PathBuf, error: io::Error },
}

/// Which folders a copy uses, for the log.
#[derive(Debug)]
enum Mode {
    Installed,
    /// The portable data folder, filled just now from the per-user folders
    /// the first time.
    Portable {
        folder: PathBuf,
        copied_from: Option<AppDirs>,
    },
    /// A portable copy that uses the per-user folders this time, because
    /// its own could not be written or filled. A copy that failed is tried
    /// again at the next start.
    Fallback {
        folder: PathBuf,
        error: io::Error,
    },
}

/// Whether `executable` is a portable copy: the zip's marker beside it,
/// saying what the zip writes there. `read` returns a file's contents.
fn portable_folder(executable: &Path, read: impl Fn(&Path) -> Option<String>) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let (name, contents) = PORTABLE_MARKER;
    read(&directory.join(name))
        .is_some_and(|text| text.trim() == contents)
        .then(|| directory.join(DATA_FOLDER))
}

fn portable_for(executable: &Path, read: impl Fn(&Path) -> Option<String>) -> Portable {
    let Some(folder) = portable_folder(executable, read) else {
        return Portable::No;
    };
    match writable(&folder) {
        Ok(()) => Portable::Folder(folder),
        Err(error) => Portable::Unwritable { folder, error },
    }
}

/// Whether a file can be made in `folder`, or beside it while it does not
/// exist yet, as in Program Files or on read-only media it cannot. Leaves
/// nothing behind.
fn writable(folder: &Path) -> io::Result<()> {
    let directory = match fs::metadata(folder) {
        Ok(metadata) if metadata.is_dir() => folder,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a file has the data folder's name",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            folder.parent().ok_or(io::ErrorKind::InvalidInput)?
        }
        Err(error) => return Err(error),
    };
    let probe = directory.join(format!(".fastsonic-write-test-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {}
        // Left by an earlier process with this id.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    fs::remove_file(&probe)
}

fn settle(installed: AppDirs, portable: Portable) -> (AppDirs, Mode) {
    match portable {
        Portable::No => (installed, Mode::Installed),
        Portable::Unwritable { folder, error } => (installed, Mode::Fallback { folder, error }),
        Portable::Folder(folder) => match prepare(&folder, &installed) {
            Ok(copied_from) => (
                AppDirs::inside(&folder),
                Mode::Portable {
                    folder,
                    copied_from,
                },
            ),
            Err(error) => (installed, Mode::Fallback { folder, error }),
        },
    }
}

/// Makes the portable data folder when it does not exist yet, filled from
/// the per-user settings and state, never moving them, and returns those
/// folders when it copied anything. Caches are not copied: Fastsonic fills
/// them again. The copy is made in `fastsonic-data.partial` and renamed
/// into place, so one interrupted by a crash or a power cut is made again.
fn prepare(folder: &Path, from: &AppDirs) -> io::Result<Option<AppDirs>> {
    if folder.is_dir() {
        return Ok(None);
    }
    let mut partial = folder.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    // What an interrupted copy left.
    match fs::remove_dir_all(&partial) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let from = Some(from).filter(|from| from.config.is_dir() || from.state.is_dir());
    if let Some(from) = from
        && [&from.config, &from.state]
            .iter()
            .any(|tree| partial.starts_with(tree))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the portable copy is inside its own per-user folders",
        ));
    }
    let staged = AppDirs::inside(&partial);
    staged.ensure()?;
    if let Some(from) = from {
        let copied = copy_tree(&from.config, &staged.config, &[])
            .and_then(|()| copy_tree(&from.state, &staged.state, NOT_COPIED));
        if let Err(error) = copied {
            let _ = fs::remove_dir_all(&partial);
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(&partial, folder) {
        let _ = fs::remove_dir_all(&partial);
        // Another launch finished first.
        if folder.is_dir() {
            return Ok(None);
        }
        return Err(error);
    }
    Ok(from.cloned())
}

/// Copies the files and folders in `from` into `to`, which exists, leaving
/// out `skip` at the top. Links are not followed or copied. A missing `from`
/// copies nothing.
fn copy_tree(from: &Path, to: &Path, skip: &[&str]) -> io::Result<()> {
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|skipped| name == *skipped) {
            continue;
        }
        let kind = entry.file_type()?;
        let target = to.join(&name);
        if kind.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target, &[])?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// The log line for the folders a copy uses, `dirs`, chosen for `mode`. An
/// installed copy says nothing.
fn describe(dirs: &AppDirs, mode: &Mode) -> Option<(log::Level, String)> {
    Some(match mode {
        Mode::Installed => return None,
        Mode::Portable {
            folder,
            copied_from: None,
        } => (
            log::Level::Info,
            format!("portable copy: keeping files in {}", folder.display()),
        ),
        Mode::Portable {
            folder,
            copied_from: Some(from),
        } => (
            log::Level::Info,
            format!(
                "portable copy: keeping files in {}, starting from a copy of the settings in {} and the state in {}",
                folder.display(),
                from.config.display(),
                from.state.display()
            ),
        ),
        Mode::Fallback { folder, error } => (
            log::Level::Warn,
            format!(
                "portable copy, but {} is not usable ({error}); keeping files in {} and {}",
                folder.display(),
                dirs.config.display(),
                dirs.state.display()
            ),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A portable copy unpacked into `root/app`, with its marker, and
    /// per-user folders under `root/user` holding settings, state and a
    /// cache.
    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
        installed: AppDirs,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastsonic-portable-{name}-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ));
            let app = root.join("app");
            fs::create_dir_all(&app).unwrap();
            fs::write(
                app.join("fastsonic-portable.txt"),
                "fastsonic-portable-v1\r\n",
            )
            .unwrap();
            let installed = AppDirs::inside(&root.join("user"));
            fs::create_dir_all(installed.config.join("themes")).unwrap();
            fs::write(installed.config.join("settings.json"), "{\"volume\":3}").unwrap();
            fs::write(installed.config.join("themes").join("dusk.json"), "{}").unwrap();
            fs::create_dir_all(&installed.state).unwrap();
            fs::write(installed.state.join("credentials.json"), "signed in").unwrap();
            fs::write(installed.state.join("fastsonic.log"), "old run").unwrap();
            fs::create_dir_all(&installed.cache).unwrap();
            fs::write(installed.cache.join("art.bin"), "rebuildable").unwrap();
            Self {
                executable: app.join("fastsonic.exe"),
                installed,
                root,
            }
        }

        fn app(&self) -> &Path {
            self.executable.parent().unwrap()
        }

        fn folder(&self) -> PathBuf {
            self.app().join("fastsonic-data")
        }

        fn portable(&self) -> Portable {
            portable_for(&self.executable, |path| fs::read_to_string(path).ok())
        }

        fn discover(&self) -> (AppDirs, Mode) {
            settle(self.installed.clone(), self.portable())
        }

        /// Every file under `root`, relative, with its contents.
        fn files(root: &Path) -> Vec<(String, String)> {
            let mut files = Vec::new();
            let mut pending = vec![root.to_path_buf()];
            while let Some(directory) = pending.pop() {
                for entry in fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        pending.push(path);
                    } else {
                        let name = path.strip_prefix(root).unwrap().to_string_lossy();
                        files.push((name.replace('\\', "/"), fs::read_to_string(&path).unwrap()));
                    }
                }
            }
            files.sort();
            files
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_copy_is_portable_only_with_the_zips_marker() {
        let executable = Path::new("/apps/fastsonic/fastsonic.exe");
        let marker = Path::new("/apps/fastsonic/fastsonic-portable.txt");
        let read = |contents: &'static str| {
            move |path: &Path| (path == marker).then(|| contents.to_owned())
        };
        let folder = Some(PathBuf::from("/apps/fastsonic/fastsonic-data"));
        assert_eq!(portable_folder(executable, |_| None), None);
        for contents in ["fastsonic-portable-v1", " fastsonic-portable-v1 \r\n"] {
            assert_eq!(portable_folder(executable, read(contents)), folder);
        }
        for contents in ["", "fastsonic-portable-v2", "spotifast-portable-v1"] {
            assert_eq!(portable_folder(executable, read(contents)), None);
        }
    }

    #[test]
    fn an_installed_copy_uses_the_per_user_folders() {
        let fixture = Fixture::new("installed");
        fs::remove_file(fixture.app().join("fastsonic-portable.txt")).unwrap();
        let (dirs, mode) = fixture.discover();
        assert!(matches!(mode, Mode::Installed));
        assert_eq!(dirs, fixture.installed);
        assert!(!fixture.folder().exists());
        assert!(describe(&dirs, &mode).is_none(), "nothing to say");
    }

    #[test]
    fn a_first_portable_start_copies_settings_and_state_but_not_caches_or_logs() {
        let fixture = Fixture::new("first");
        let before = Fixture::files(&fixture.root.join("user"));
        let (dirs, mode) = fixture.discover();
        let Mode::Portable {
            folder,
            copied_from,
        } = &mode
        else {
            panic!("{mode:?}");
        };
        assert_eq!(folder, &fixture.folder());
        assert_eq!(copied_from.as_ref(), Some(&fixture.installed));
        assert_eq!(dirs, AppDirs::inside(&fixture.folder()));
        assert_eq!(
            Fixture::files(&fixture.folder()),
            [
                (
                    "config/settings.json".to_owned(),
                    "{\"volume\":3}".to_owned()
                ),
                ("config/themes/dusk.json".to_owned(), "{}".to_owned()),
                ("state/credentials.json".to_owned(), "signed in".to_owned()),
            ]
        );
        assert!(fixture.folder().join("cache").is_dir());
        assert_eq!(
            Fixture::files(&fixture.root.join("user")),
            before,
            "the installed copy keeps its own, untouched"
        );
        assert!(!fixture.app().join("fastsonic-data.partial").exists());
        let (level, line) = describe(&dirs, &mode).unwrap();
        assert_eq!(level, log::Level::Info);
        assert!(line.contains("fastsonic-data"), "{line}");
    }

    #[test]
    fn a_later_start_keeps_the_portable_folder_as_it_is() {
        let fixture = Fixture::new("later");
        fixture.discover();
        let settings = fixture.folder().join("config").join("settings.json");
        fs::write(&settings, "{\"volume\":9}").unwrap();
        fs::write(fixture.installed.config.join("settings.json"), "changed").unwrap();
        let (_, mode) = fixture.discover();
        assert!(matches!(
            mode,
            Mode::Portable {
                copied_from: None,
                ..
            }
        ));
        assert_eq!(fs::read_to_string(settings).unwrap(), "{\"volume\":9}");
    }

    #[test]
    fn an_interrupted_copy_is_made_again() {
        let fixture = Fixture::new("interrupted");
        let partial = fixture.app().join("fastsonic-data.partial");
        fs::create_dir_all(partial.join("state")).unwrap();
        fs::write(partial.join("state").join("half.json"), "{").unwrap();
        let (_, mode) = fixture.discover();
        assert!(matches!(
            mode,
            Mode::Portable {
                copied_from: Some(_),
                ..
            }
        ));
        assert!(!fixture.folder().join("state").join("half.json").exists());
        assert!(
            fixture
                .folder()
                .join("state")
                .join("credentials.json")
                .is_file()
        );
        assert!(!partial.exists());
    }

    #[test]
    fn an_unwritable_folder_falls_back_to_the_per_user_folders() {
        let fixture = Fixture::new("unwritable");
        // Where the data folder would go is a file, so nothing can be made
        // there on any platform.
        let blocked = fixture.app().join("blocked");
        fs::write(&blocked, "").unwrap();
        let executable = blocked.join("fastsonic.exe");
        let read = |path: &Path| {
            (path == blocked.join("fastsonic-portable.txt"))
                .then(|| "fastsonic-portable-v1".to_owned())
        };
        let portable = portable_for(&executable, read);
        assert!(
            matches!(portable, Portable::Unwritable { .. }),
            "{portable:?}"
        );
        let (dirs, mode) = settle(fixture.installed.clone(), portable);
        assert!(matches!(mode, Mode::Fallback { .. }));
        assert_eq!(dirs, fixture.installed);
        let (level, line) = describe(&dirs, &mode).unwrap();
        assert_eq!(level, log::Level::Warn);
        assert!(line.contains("not usable"), "{line}");
    }

    #[test]
    fn a_file_named_like_the_data_folder_is_not_used() {
        let fixture = Fixture::new("file");
        fs::write(fixture.folder(), "").unwrap();
        assert!(matches!(fixture.portable(), Portable::Unwritable { .. }));
    }

    #[test]
    fn the_write_test_leaves_nothing_behind() {
        let fixture = Fixture::new("probe");
        let before = Fixture::files(fixture.app());
        assert!(matches!(fixture.portable(), Portable::Folder(_)));
        assert_eq!(Fixture::files(fixture.app()), before);
    }

    #[test]
    fn a_portable_copy_inside_its_per_user_folders_is_not_filled_from_them() {
        let fixture = Fixture::new("inside");
        let app = fixture.installed.state.join("unpacked");
        fs::create_dir(&app).unwrap();
        fs::write(app.join("fastsonic-portable.txt"), "fastsonic-portable-v1").unwrap();
        let portable = portable_for(&app.join("fastsonic.exe"), |path| {
            fs::read_to_string(path).ok()
        });
        let (_, mode) = settle(fixture.installed.clone(), portable);
        assert!(matches!(mode, Mode::Fallback { .. }), "{mode:?}");
        assert!(!app.join("fastsonic-data").exists());
        assert!(!app.join("fastsonic-data.partial").exists());
    }

    /// The release workflow puts this file beside `fastsonic.exe` in the
    /// Windows zip, and only there.
    #[test]
    fn the_zip_carries_the_marker_the_app_looks_for() {
        let (name, contents) = PORTABLE_MARKER;
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |path: &str| fs::read_to_string(repository.join(path)).unwrap();
        assert_eq!(
            read("packaging/windows/fastsonic-portable.txt").trim(),
            contents
        );
        let release = read(".github/workflows/release.yml");
        assert!(release.contains(&format!("cp packaging/windows/{name} ")));
        let installer = read("packaging/windows/fastsonic.iss");
        assert!(
            !installer.contains(name),
            "an installed copy is not portable"
        );
    }

    /// Installed copies, and every copy on Linux and macOS, keep the
    /// folders they always had.
    #[test]
    fn the_per_user_folders_are_the_ones_fastsonic_always_used() {
        let Some(project) = ProjectDirs::from("io", "github.rwojsznis", "fastsonic") else {
            return;
        };
        let dirs = AppDirs::installed();
        assert_eq!(dirs.config, project.config_dir());
        assert_eq!(
            dirs.state,
            project.state_dir().unwrap_or(project.data_local_dir())
        );
        assert_eq!(dirs.cache, project.cache_dir());
        if !cfg!(windows) {
            assert_eq!(AppDirs::discover().dirs, dirs);
        }
    }
}

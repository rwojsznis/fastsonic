//! Where Fastsonic keeps its files.
//!
//! Configuration, durable state (the music server's credential), and
//! disposable caches (audio, artwork) live in the platform's conventional
//! directories, so clearing a cache never signs the user out and a config
//! backup never contains a credential.

use std::path::PathBuf;

use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl AppDirs {
    pub fn discover() -> Self {
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

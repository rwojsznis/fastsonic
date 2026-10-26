//! Palettes from JSON files in the themes folder, listed off the interface
//! thread.

use super::Palette;
use egui::Color32;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
    sync::mpsc,
};

/// A palette file, named by its filename in the themes folder.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomTheme {
    pub filename: String,
    pub palette: Palette,
}

/// How the picker names a palette file: its filename without `.json`. The
/// saved choice still names the file.
pub fn display_name(filename: &str) -> &str {
    filename
        .len()
        .checked_sub(".json".len())
        .filter(|&at| at > 0)
        .and_then(|at| Some((filename.get(..at)?, filename.get(at..)?)))
        .filter(|(_, extension)| extension.eq_ignore_ascii_case(".json"))
        .map_or(filename, |(stem, _)| stem)
}

/// A damaged cache is treated as absent, so it never makes the rest of the
/// settings unreadable.
pub fn read_cached_theme<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<CustomTheme>, D::Error> {
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    match serde_json::from_value(value) {
        Ok(theme) => Ok(Some(theme)),
        Err(error) => {
            log::warn!("ignoring an unreadable cached theme: {error}");
            Ok(None)
        }
    }
}

/// Reads a settings value with `read`, or gives the default when it is
/// damaged, so one bad field never makes the rest of the settings
/// unreadable.
fn tolerant<T: Default + serde::de::DeserializeOwned>(
    value: serde_json::Value,
    what: &str,
) -> Option<T> {
    if value.is_null() {
        return None;
    }
    match serde_json::from_value(value) {
        Ok(read) => Some(read),
        Err(error) => {
            log::warn!("ignoring an unreadable {what}: {error}");
            None
        }
    }
}

/// The palette file Follow system shows in light or in dark mode, with the
/// colours last read from it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairSlot {
    pub filename: String,
    /// The colours last accepted from the file, so the first frame after a
    /// restart is right and a file that disappears keeps its look. A
    /// damaged copy reads as `None` and leaves the choice in place.
    #[serde(
        default,
        deserialize_with = "read_slot_palette",
        skip_serializing_if = "Option::is_none"
    )]
    pub palette: Option<Palette>,
}

fn read_slot_palette<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Palette>, D::Error> {
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(tolerant::<Option<Palette>>(value, "cached palette").flatten())
}

fn read_slot<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PairSlot>, D::Error> {
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(tolerant::<Option<PairSlot>>(value, "theme choice").flatten())
}

/// A palette file for light mode and one for dark mode, which Follow system
/// switches between as the desktop does. An empty side is Fastsonic's own
/// Light or Dark, so the default pair is Follow system as it always was.
///
/// Each side takes only palettes of its own base: egui draws the standard
/// controls light or dark with the system, so a dark palette in light mode
/// would put dark colours on light controls.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThemePair {
    #[serde(
        default,
        deserialize_with = "read_slot",
        skip_serializing_if = "Option::is_none"
    )]
    pub light: Option<PairSlot>,
    #[serde(
        default,
        deserialize_with = "read_slot",
        skip_serializing_if = "Option::is_none"
    )]
    pub dark: Option<PairSlot>,
}

/// Reads the `theme_pair` setting, treating a damaged one as no choice.
pub fn read_theme_pair<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ThemePair, D::Error> {
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(tolerant(value, "light and dark theme choice").unwrap_or_default())
}

impl ThemePair {
    /// Both sides are Fastsonic's own, and the setting is left unwritten.
    pub fn is_default(&self) -> bool {
        self.light.is_none() && self.dark.is_none()
    }

    pub fn slot(&self, dark: bool) -> Option<&PairSlot> {
        if dark {
            self.dark.as_ref()
        } else {
            self.light.as_ref()
        }
    }

    fn slot_mut(&mut self, dark: bool) -> &mut Option<PairSlot> {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    /// The file chosen for dark or light mode, `None` for Fastsonic's own.
    pub fn filename(&self, dark: bool) -> Option<&str> {
        self.slot(dark).map(|slot| slot.filename.as_str())
    }

    /// Chooses `theme` for dark or light mode, or Fastsonic's own with
    /// `None`. A palette of the other base changes nothing and returns
    /// false.
    pub fn set(&mut self, dark: bool, theme: Option<&CustomTheme>) -> bool {
        let slot = match theme {
            Some(theme) if theme.palette.dark != dark => return false,
            Some(theme) => Some(PairSlot {
                filename: theme.filename.clone(),
                palette: Some(theme.palette),
            }),
            None => None,
        };
        *self.slot_mut(dark) = slot;
        true
    }

    /// The colours to show in dark or light mode, as last accepted. `None`
    /// for an empty side, or one whose file was never read.
    pub fn palette(&self, dark: bool) -> Option<Palette> {
        self.slot(dark).and_then(|slot| slot.palette)
    }

    /// Takes each chosen file's colours from a finished listing, so an
    /// edited file shows its new ones. A file no longer listed, or that now
    /// names the other base, keeps its last accepted colours. True when a
    /// side changed, for the settings to be saved.
    pub fn refresh(&mut self, catalog: &Catalog) -> bool {
        let mut changed = false;
        for dark in [false, true] {
            if let Some(slot) = self.slot_mut(dark)
                && let Some(theme) = catalog.find(&slot.filename)
                && theme.palette.dark == dark
                && slot.palette != Some(theme.palette)
            {
                slot.palette = Some(theme.palette);
                changed = true;
            }
        }
        changed
    }

    /// The chosen files, for a listing to read first.
    pub fn filenames(&self) -> impl Iterator<Item = String> + '_ {
        [&self.light, &self.dark]
            .into_iter()
            .flatten()
            .map(|slot| slot.filename.clone())
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Base {
    #[default]
    Dark,
    Light,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    #[serde(default)]
    base: Base,
    #[serde(default)]
    colors: std::collections::BTreeMap<String, String>,
}

/// Reads a palette file: the `base` palette with the `colors` it names
/// replaced. Unknown fields and colour names are errors, so a typo is
/// reported rather than silently ignored.
pub(crate) fn parse_palette(text: &str) -> Result<Palette, String> {
    let file: ThemeFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut palette = match file.base {
        Base::Dark => Palette::dark(),
        Base::Light => Palette::light(),
    };
    for (name, value) in file.colors {
        let color = parse_color(&value)
            .ok_or_else(|| format!("{name}: expected #RRGGBB or #RRGGBBAA, not {value:?}"))?;
        let field = match name.as_str() {
            "window" => &mut palette.window,
            "panel" => &mut palette.panel,
            "surface" => &mut palette.surface,
            "surface_hover" => &mut palette.surface_hover,
            "surface_active" => &mut palette.surface_active,
            "outline" => &mut palette.outline,
            "text" => &mut palette.text,
            "secondary" => &mut palette.secondary,
            "dim" => &mut palette.dim,
            "accent" => &mut palette.accent,
            "accent_hover" => &mut palette.accent_hover,
            "on_accent" => &mut palette.on_accent,
            "danger" => &mut palette.danger,
            "warning" => &mut palette.warning,
            "overlay" => &mut palette.overlay,
            "shadow" => &mut palette.shadow,
            "tint" => &mut palette.tint,
            _ => return Err(format!("unknown colour {name:?}")),
        };
        *field = color;
    }
    Ok(palette)
}

fn parse_color(value: &str) -> Option<Color32> {
    let hex = value.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    let alpha = if hex.len() == 8 { channel(6)? } else { 255 };
    Some(Color32::from_rgba_unmultiplied(
        channel(0)?,
        channel(2)?,
        channel(4)?,
        alpha,
    ))
}

// A palette has seventeen colours. These limits also bound the directory work
// and the diagnostics, not just the bytes read from one file. Never recurse.
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 512;
const MAX_THEMES: usize = 128;

fn filename_is_local(filename: &str) -> bool {
    let mut parts = Path::new(filename).components();
    matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !filename.contains('\\')
        && filename.ends_with(".json")
}

fn read_theme(directory: &Path, filename: &str) -> Result<CustomTheme, String> {
    if !filename_is_local(filename) {
        return Err("expected a JSON filename in the themes folder".into());
    }
    let path = directory.join(filename);
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("expected a regular file, not a folder or a link".into());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("larger than the 64 KiB a theme may be".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("larger than the 64 KiB a theme may be".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "expected UTF-8 JSON".to_string())?;
    Ok(CustomTheme {
        filename: filename.into(),
        palette: parse_palette(text)?,
    })
}

/// The palettes put in the themes folder on the first start, from
/// fastframe-theme (see `assets/themes/README.md`).
const BUNDLED: [(&str, &str); 8] = [
    (
        "Catppuccin Latte.json",
        include_str!("../../assets/themes/Catppuccin Latte.json"),
    ),
    (
        "Catppuccin.json",
        include_str!("../../assets/themes/Catppuccin.json"),
    ),
    ("Nord.json", include_str!("../../assets/themes/Nord.json")),
    (
        "Ristretto.json",
        include_str!("../../assets/themes/Ristretto.json"),
    ),
    (
        "Rose Pine Dawn.json",
        include_str!("../../assets/themes/Rose Pine Dawn.json"),
    ),
    (
        "Rose Pine Moon.json",
        include_str!("../../assets/themes/Rose Pine Moon.json"),
    ),
    (
        "Rose Pine.json",
        include_str!("../../assets/themes/Rose Pine.json"),
    ),
    (
        "Tokyo Night.json",
        include_str!("../../assets/themes/Tokyo Night.json"),
    ),
];

/// A bundled palette by filename, read without the themes folder.
#[cfg(feature = "demo")]
pub fn bundled(filename: &str) -> Option<CustomTheme> {
    let (filename, text) = BUNDLED.into_iter().find(|(name, _)| *name == filename)?;
    Some(CustomTheme {
        filename: filename.into(),
        palette: parse_palette(text).ok()?,
    })
}

/// The file in the themes folder naming the bundled palettes already put
/// there, one per line.
const INSTALLED: &str = ".installed-palettes";

/// Puts each bundled palette not yet named in [`INSTALLED`] into
/// `directory`, unless something there already has its name, and records
/// it. From then on the file is the user's: an edit lasts, a deleted one
/// stays deleted, and a palette added in a later version arrives once.
fn install_bundled(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let record = directory.join(INSTALLED);
    // A record that cannot be read installs nothing, rather than bring back
    // what was deleted.
    let mut installed: std::collections::BTreeSet<String> = match std::fs::read_to_string(&record) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(error) => return Err(error),
    };
    let before = installed.len();
    for (name, contents) in BUNDLED {
        if installed.contains(name) {
            continue;
        }
        // Anything already there, even a broken link, is the user's.
        match std::fs::symlink_metadata(directory.join(name)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                write_new(directory, name, contents)?;
            }
            Err(error) => return Err(error),
        }
        installed.insert(name.to_owned());
    }
    if installed.len() != before {
        let list: String = installed.iter().map(|name| format!("{name}\n")).collect();
        write_new(directory, INSTALLED, &list)?;
    }
    Ok(())
}

/// Writes `name` in `directory` whole or not at all, through a temporary
/// file that is never a link.
fn write_new(directory: &Path, name: &str, contents: &str) -> std::io::Result<()> {
    crate::util::write_file(&directory.join(name), contents)
}

#[derive(Default)]
struct Loaded {
    themes: Vec<CustomTheme>,
    /// What went wrong with the folder itself.
    problem: Option<String>,
    /// The files that could not be read, and why.
    skipped: Vec<(String, String)>,
}

/// Lists and reads the palettes in `directory`, in filename order, leaving
/// out the files that cannot be read.
fn discover(directory: &Path, selected: &[String]) -> Loaded {
    let mut loaded = Loaded::default();
    let read = |loaded: &mut Loaded, filename: &str| match read_theme(directory, filename) {
        Ok(theme) => loaded.themes.push(theme),
        Err(error) => {
            log::warn!("unable to load the theme {filename:?}: {error}");
            loaded.skipped.push((filename.to_owned(), error));
        }
    };
    // Read the saved choices first so a large folder cannot push them out.
    // Each is still a single validated filename, never an arbitrary path. A
    // choice that is gone is told apart from one that cannot be read.
    for filename in selected {
        let missing = filename_is_local(filename)
            && std::fs::symlink_metadata(directory.join(filename))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
        if missing {
            log::warn!("the selected theme {filename:?} is not in the themes folder");
        } else {
            read(&mut loaded, filename);
        }
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                loaded.problem =
                    Some("The themes folder could not be read. See the log for details.".into());
                log::warn!("unable to read themes at {}: {error}", directory.display());
            }
            return loaded;
        }
    };
    let mut names = Vec::new();
    for (index, entry) in entries.take(MAX_DIRECTORY_ENTRIES + 1).enumerate() {
        if index == MAX_DIRECTORY_ENTRIES {
            loaded.problem = Some(format!(
                "The themes folder has more than {MAX_DIRECTORY_ENTRIES} entries. Keep fewer files there to list its palettes."
            ));
            // Do not offer a different arbitrary subset depending on the
            // filesystem's order.
            return loaded;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("unable to read a theme entry: {error}");
                continue;
            }
        };
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        if filename_is_local(filename) && !selected.iter().any(|chosen| chosen == filename) {
            match entry.file_type() {
                Ok(kind) if kind.is_file() => names.push(filename.to_owned()),
                Ok(_) => {}
                Err(error) => log::warn!("unable to inspect the theme {filename:?}: {error}"),
            }
        }
    }
    names.sort();
    if names.len() + loaded.themes.len() > MAX_THEMES {
        loaded.problem = Some(format!(
            "Only {MAX_THEMES} palettes can be listed. Keep fewer JSON files in the themes folder to see the rest."
        ));
    }
    let room = MAX_THEMES.saturating_sub(loaded.themes.len());
    for filename in names.into_iter().take(room) {
        read(&mut loaded, &filename);
    }
    loaded.themes.sort_by(|a, b| a.filename.cmp(&b.filename));
    loaded
}

struct Scan {
    directory: PathBuf,
    selected: Vec<String>,
    waker: crate::backend::Waker,
}

/// The palettes in the themes folder, listed on a thread of their own at
/// launch and on request. At most one scan runs and one waits, so a burst of
/// requests cannot pile up workers. The choice itself lives in Settings,
/// never in a scan's result.
#[derive(Default)]
pub struct Catalog {
    themes: Vec<CustomTheme>,
    problem: Option<String>,
    skipped: Vec<(String, String)>,
    /// A scan has finished at least once.
    listed: bool,
    /// The next scan first puts the bundled palettes in the folder.
    install_bundled: bool,
    receiver: Option<mpsc::Receiver<Loaded>>,
    pending: Option<Scan>,
}

impl Catalog {
    /// Has the next scan put the bundled palettes in the folder first, each
    /// only if it was never put there before. Launches ask for it; demo
    /// launches too, in their own profile. Tests do not.
    pub fn install_bundled_palettes(&mut self) {
        self.install_bundled = true;
    }

    /// Lists `directory` in the background, reading the `selected`
    /// filenames (the saved choices) first.
    pub fn start(
        &mut self,
        directory: PathBuf,
        selected: impl IntoIterator<Item = String>,
        waker: &crate::backend::Waker,
    ) {
        let mut selected: Vec<String> = selected.into_iter().collect();
        selected.sort();
        selected.dedup();
        let scan = Scan {
            directory,
            selected,
            waker: waker.clone(),
        };
        if self.loading() {
            self.pending = Some(scan);
        } else {
            self.scan(scan);
        }
    }

    fn scan(&mut self, scan: Scan) {
        let install = std::mem::take(&mut self.install_bundled);
        self.spawn(&scan.waker, move || {
            if install && let Err(error) = install_bundled(&scan.directory) {
                log::warn!(
                    "unable to put the bundled palettes in {}: {error}",
                    scan.directory.display()
                );
            }
            discover(&scan.directory, &scan.selected)
        });
    }

    fn spawn(
        &mut self,
        waker: &crate::backend::Waker,
        load: impl FnOnce() -> Loaded + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let wake = waker.clone();
        let spawned = std::thread::Builder::new()
            .name("fastsonic-themes".into())
            .spawn(move || {
                if sender.send(load()).is_ok() {
                    wake.wake();
                }
            });
        match spawned {
            Ok(_) => self.receiver = Some(receiver),
            Err(error) => {
                log::warn!("unable to start the theme loader: {error}");
                self.problem = Some(UNLOADED.into());
            }
        }
    }

    pub fn themes(&self) -> &[CustomTheme] {
        &self.themes
    }

    pub fn find(&self, filename: &str) -> Option<&CustomTheme> {
        self.themes.iter().find(|theme| theme.filename == filename)
    }

    pub fn loading(&self) -> bool {
        self.receiver.is_some()
    }

    /// What the Theme row says under its name: why the chosen palette is
    /// unavailable, or what else went wrong. A rescan keeps the last
    /// listing's words until it finishes, so the row does not flicker.
    pub fn detail(&self, selected: Option<&str>) -> String {
        if let Some(filename) =
            selected.filter(|filename| self.listed && self.find(filename).is_none())
        {
            let name = display_name(filename);
            return match self.skipped.iter().find(|(skipped, _)| skipped == filename) {
                Some((_, error)) => format!(
                    "{name} could not be read ({error}). Its last colours stay until it is fixed."
                ),
                None => format!(
                    "{name} is no longer in the themes folder. Its last colours stay until you choose another theme."
                ),
            };
        }
        if let Some(problem) = &self.problem {
            return problem.clone();
        }
        match self.skipped.split_first() {
            None => String::new(),
            Some(((filename, error), [])) => format!("Skipped {filename}: {error}."),
            Some(((filename, error), rest)) => format!(
                "Skipped {filename} ({error}) and {} more. See the log for details.",
                rest.len()
            ),
        }
    }

    /// The palettes Follow system offers for dark or light mode: those of
    /// that base, in picker order.
    pub fn pair_themes(&self, dark: bool) -> impl Iterator<Item = &CustomTheme> {
        self.themes
            .iter()
            .filter(move |theme| theme.palette.dark == dark)
    }

    /// What the light or dark mode row says under its name: why its palette
    /// is unavailable, else what went wrong with the folder, else nothing.
    pub fn pair_detail(&self, pair: &ThemePair, dark: bool) -> String {
        let Some(filename) = pair.filename(dark).filter(|_| self.listed) else {
            return String::new();
        };
        let name = display_name(filename);
        match self.find(filename) {
            Some(theme) if theme.palette.dark == dark => String::new(),
            Some(_) => format!(
                "{name} is now a {} theme. Its last colours stay until you choose another.",
                if dark { "light" } else { "dark" }
            ),
            None => match self.skipped.iter().find(|(skipped, _)| skipped == filename) {
                Some((_, error)) => format!(
                    "{name} could not be read ({error}). Its last colours stay until it is fixed."
                ),
                None => format!(
                    "{name} is no longer in the themes folder. Its last colours stay until you choose another theme."
                ),
            },
        }
    }

    /// Takes a finished scan. True when a new listing replaced the last.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };
        let result = receiver.try_recv();
        if matches!(result, Err(mpsc::TryRecvError::Empty)) {
            return false;
        }
        self.receiver = None;
        if let Some(scan) = self.pending.take() {
            // Keep the last accepted listing until the latest request
            // finishes: publishing this superseded one could flash old colours.
            self.scan(scan);
            return false;
        }
        match result {
            Ok(loaded) => {
                self.themes = loaded.themes;
                self.problem = loaded.problem;
                self.skipped = loaded.skipped;
            }
            Err(_) => self.problem = Some(UNLOADED.into()),
        }
        self.listed = true;
        true
    }

    #[cfg(test)]
    pub(crate) fn from_themes(themes: Vec<CustomTheme>) -> Self {
        Self {
            themes,
            listed: true,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn load_test(&mut self, load: impl FnOnce() -> Vec<CustomTheme> + Send + 'static) {
        self.spawn(&crate::backend::Waker::default(), move || Loaded {
            themes: load(),
            ..Loaded::default()
        });
    }
}

const UNLOADED: &str = "The themes could not be loaded. See the log for details.";

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fastsonic-themes-{name}-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn filenames(themes: &[CustomTheme]) -> Vec<&str> {
        themes.iter().map(|theme| theme.filename.as_str()).collect()
    }

    fn wait(catalog: &mut Catalog) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !catalog.poll() {
            assert!(std::time::Instant::now() < deadline, "the scan never ended");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn palettes_are_named_without_the_json_extension() {
        assert_eq!(display_name("Nord.json"), "Nord");
        assert_eq!(display_name("Rose Pine Dawn.json"), "Rose Pine Dawn");
        assert_eq!(display_name("mine.JSON"), "mine");
        assert_eq!(display_name("two.dots.json"), "two.dots");
        assert_eq!(display_name("Café.json"), "Café");
        assert_eq!(display_name(".json"), ".json", "nothing left to show");
        assert_eq!(display_name("notes"), "notes");
        assert_eq!(display_name("é"), "é");
    }

    #[test]
    fn colours_inherit_the_base_and_may_carry_alpha() {
        let palette =
            parse_palette(r##"{"base":"light","colors":{"text":"#ebdbb2","shadow":"#00000080"}}"##)
                .unwrap();
        assert!(!palette.dark);
        assert_eq!(palette.window, Palette::light().window);
        assert_eq!(palette.text, Color32::from_rgb(235, 219, 178));
        assert_eq!(palette.shadow, Color32::from_black_alpha(128));
        assert_eq!(
            parse_palette(r#"{"base":"dark"}"#).unwrap(),
            Palette::dark()
        );
        assert_eq!(
            parse_palette(r#"{"base":"light"}"#).unwrap(),
            Palette::light()
        );
        assert_eq!(
            parse_palette("{}").unwrap(),
            Palette::dark(),
            "a file without a base or colours is the dark palette"
        );
    }

    /// A file written before `tint` existed still reads, and its pages
    /// keep the plain window.
    #[test]
    fn a_palette_without_a_tint_washes_nothing() {
        assert_eq!(Palette::dark().tint, Color32::TRANSPARENT);
        assert_eq!(Palette::light().tint, Color32::TRANSPARENT);
        let older = parse_palette(r##"{"base":"light","colors":{"accent":"#8c3fa5"}}"##).unwrap();
        assert_eq!(older.tint, Color32::TRANSPARENT);
        let tinted = parse_palette(r##"{"colors":{"tint":"#00000018"}}"##).unwrap();
        assert_eq!(tinted.tint, Color32::from_black_alpha(0x18));
    }

    #[test]
    fn every_colour_can_be_set() {
        let names = [
            "window",
            "panel",
            "surface",
            "surface_hover",
            "surface_active",
            "outline",
            "text",
            "secondary",
            "dim",
            "accent",
            "accent_hover",
            "on_accent",
            "danger",
            "warning",
            "overlay",
            "shadow",
            "tint",
        ];
        let colors: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(index, name)| format!(r##""{name}":"#0102{index:02x}""##))
            .collect();
        let palette = parse_palette(&format!(r#"{{"colors":{{{}}}}}"#, colors.join(","))).unwrap();
        let set = [
            palette.window,
            palette.panel,
            palette.surface,
            palette.surface_hover,
            palette.surface_active,
            palette.outline,
            palette.text,
            palette.secondary,
            palette.dim,
            palette.accent,
            palette.accent_hover,
            palette.on_accent,
            palette.danger,
            palette.warning,
            palette.overlay,
            palette.shadow,
            palette.tint,
        ];
        for (index, color) in set.into_iter().enumerate() {
            assert_eq!(
                color,
                Color32::from_rgb(1, 2, index as u8),
                "{}",
                names[index]
            );
        }
    }

    #[test]
    fn invalid_palettes_are_rejected_with_a_reason() {
        for (text, reason) in [
            (r##"{"colors":{"text":"#fff"}}"##, "text: expected"),
            (r##"{"colors":{"text":"#zzzzzz"}}"##, "text: expected"),
            (r##"{"colors":{"text":"ffffff"}}"##, "text: expected"),
            (
                r##"{"colors":{"typo":"#ffffff"}}"##,
                "unknown colour \"typo\"",
            ),
            (r#"{"base":"system"}"#, "unknown variant"),
            (r#"{"typo":true}"#, "unknown field"),
            (r#"{"colors":{"text":1}}"#, "invalid type"),
            ("not json", "expected"),
        ] {
            let error = parse_palette(text).unwrap_err();
            assert!(error.contains(reason), "{text}: {error}");
        }
    }

    #[test]
    fn discovery_sorts_valid_files_and_reports_the_rest() {
        let dir = std::env::temp_dir().join(format!(
            "fastsonic-themes-missing-{}",
            rand::random::<u64>()
        ));
        let missing = discover(&dir, &[]);
        assert!(missing.themes.is_empty());
        assert_eq!(missing.problem, None, "no folder yet is no themes yet");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in [
            ("z.json", "{}"),
            ("a.json", "{}"),
            ("bad.json", "invalid"),
            ("ignored.txt", "{}"),
        ] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let mut catalog = Catalog::default();
        catalog.start(dir.clone(), None, &Default::default());
        wait(&mut catalog);
        assert_eq!(filenames(catalog.themes()), ["a.json", "z.json"]);
        let detail = catalog.detail(None);
        assert!(
            detail.starts_with("Skipped bad.json: expected value"),
            "{detail}"
        );

        std::fs::write(dir.join("typo.json"), r##"{"colors":{"txet":"#ffffff"}}"##).unwrap();
        catalog.start(dir.clone(), None, &Default::default());
        wait(&mut catalog);
        let detail = catalog.detail(None);
        assert!(detail.contains("and 1 more"), "{detail}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unusable_choice_says_whether_it_is_gone_or_broken() {
        let dir = scratch("choice");
        std::fs::write(dir.join("broken.json"), "{broken").unwrap();
        let mut catalog = Catalog::default();
        catalog.start(dir.clone(), Some("broken.json".into()), &Default::default());
        wait(&mut catalog);
        let detail = catalog.detail(Some("broken.json"));
        assert!(
            detail.starts_with("broken could not be read (key must be a string"),
            "{detail}"
        );
        catalog.start(dir.clone(), Some("gone.json".into()), &Default::default());
        wait(&mut catalog);
        let detail = catalog.detail(Some("gone.json"));
        assert!(
            detail.starts_with("gone is no longer in the themes folder"),
            "{detail}"
        );
        assert!(
            catalog.detail(None).starts_with("Skipped broken.json"),
            "a broken file that is no longer chosen is still reported"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_are_bounded_and_cannot_escape_to_other_files() {
        let root = scratch("bounds");
        let dir = root.join("themes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut boundary = vec![b' '; MAX_FILE_BYTES as usize];
        boundary[..2].copy_from_slice(b"{}");
        std::fs::write(dir.join("boundary.json"), &boundary).unwrap();
        assert_eq!(
            read_theme(&dir, "boundary.json").unwrap().palette,
            Palette::dark()
        );
        boundary.push(b' ');
        std::fs::write(dir.join("too-large.json"), boundary).unwrap();
        std::fs::write(dir.join("invalid-utf8.json"), [0xff, 0xfe]).unwrap();
        std::fs::create_dir(dir.join("directory.json")).unwrap();
        std::fs::write(root.join("outside.json"), b"{}").unwrap();
        for filename in [
            "too-large.json",
            "invalid-utf8.json",
            "directory.json",
            "../outside.json",
            "..\\outside.json",
            "/outside.json",
            "outside.txt",
        ] {
            assert!(read_theme(&dir, filename).is_err(), "{filename}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside.json"), dir.join("link.json")).unwrap();
            assert!(read_theme(&dir, "link.json").is_err());
            assert!(
                discover(&dir, &["link.json".to_owned()])
                    .themes
                    .iter()
                    .all(|theme| theme.filename != "link.json")
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn limits_keep_the_saved_choice_without_an_arbitrary_partial_listing() {
        let dir = scratch("limits");
        for index in 0..MAX_THEMES + 1 {
            std::fs::write(dir.join(format!("{index:03}.json")), b"{}").unwrap();
        }
        std::fs::write(dir.join("selected.json"), b"{}").unwrap();
        let loaded = discover(&dir, &["selected.json".to_owned()]);
        assert_eq!(loaded.themes.len(), MAX_THEMES);
        assert!(
            loaded
                .themes
                .iter()
                .any(|theme| theme.filename == "selected.json")
        );
        assert!(loaded.problem.unwrap().contains("128"));
        for index in 0..MAX_DIRECTORY_ENTRIES {
            std::fs::write(dir.join(format!("ignored-{index}.txt")), b"ignored").unwrap();
        }
        let loaded = discover(&dir, &["selected.json".to_owned()]);
        assert_eq!(filenames(&loaded.themes), ["selected.json"]);
        assert!(loaded.problem.unwrap().contains("512"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rescans_coalesce_and_never_publish_a_superseded_listing() {
        let dir = scratch("rescan");
        std::fs::write(dir.join("latest.json"), br#"{"base":"light"}"#).unwrap();
        let accepted = CustomTheme {
            filename: "accepted.json".into(),
            palette: Palette::dark(),
        };
        let mut catalog = Catalog::from_themes(vec![accepted.clone()]);
        let (finish, worker) = mpsc::channel();
        catalog.load_test(move || {
            worker.recv().unwrap();
            vec![CustomTheme {
                filename: "stale.json".into(),
                palette: Palette::light(),
            }]
        });
        for _ in 0..100 {
            catalog.start(dir.join("superseded"), None, &Default::default());
        }
        catalog.start(dir.clone(), Some("latest.json".into()), &Default::default());
        assert!(!catalog.poll(), "requests do not replace the running scan");
        assert_eq!(catalog.themes(), std::slice::from_ref(&accepted));
        finish.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !catalog.poll() {
            assert_eq!(
                catalog.themes(),
                std::slice::from_ref(&accepted),
                "hold the accepted listing"
            );
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!catalog.loading());
        assert_eq!(filenames(catalog.themes()), ["latest.json"]);
        assert_eq!(catalog.themes()[0].palette, Palette::light());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_bundled_palette_reads_with_its_base() {
        for (filename, text) in BUNDLED {
            let palette = parse_palette(text).unwrap_or_else(|error| panic!("{filename}: {error}"));
            let light = matches!(filename, "Catppuccin Latte.json" | "Rose Pine Dawn.json");
            assert_eq!(palette.dark, !light, "{filename}");
            assert_ne!(palette.window, Palette::dark().window, "{filename}");
            assert_ne!(palette.window, Palette::light().window, "{filename}");
            assert!(filename_is_local(filename), "{filename}");
        }
    }

    #[test]
    fn the_bundled_palettes_arrive_once_and_then_belong_to_the_user() {
        let themes = scratch("bundled").join("themes");
        install_bundled(&themes).unwrap();
        for (name, contents) in BUNDLED {
            assert_eq!(
                std::fs::read_to_string(themes.join(name)).unwrap(),
                contents
            );
        }
        // An edit lasts, and a deleted palette stays deleted.
        std::fs::write(themes.join("Nord.json"), "mine").unwrap();
        std::fs::remove_file(themes.join("Tokyo Night.json")).unwrap();
        install_bundled(&themes).unwrap();
        assert_eq!(
            std::fs::read_to_string(themes.join("Nord.json")).unwrap(),
            "mine"
        );
        assert!(!themes.join("Tokyo Night.json").exists());
        let leftovers: Vec<String> = std::fs::read_dir(&themes)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        std::fs::remove_dir_all(themes.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_palette_new_to_the_record_arrives_but_never_over_a_users_file() {
        let themes = scratch("bundled-record");
        // A record from a version that shipped only Nord, since deleted.
        std::fs::write(themes.join(INSTALLED), "Nord.json\n").unwrap();
        std::fs::write(themes.join("Catppuccin.json"), "mine").unwrap();
        install_bundled(&themes).unwrap();
        assert!(!themes.join("Nord.json").exists(), "deleted before");
        assert_eq!(
            std::fs::read_to_string(themes.join("Catppuccin.json")).unwrap(),
            "mine"
        );
        assert!(themes.join("Rose Pine.json").is_file());
        let record = std::fs::read_to_string(themes.join(INSTALLED)).unwrap();
        assert_eq!(record.lines().count(), BUNDLED.len());
        std::fs::remove_dir_all(themes).unwrap();
    }

    /// A record that cannot be read could be hiding deletions, so nothing
    /// is put back.
    #[test]
    fn an_unreadable_record_installs_nothing() {
        let themes = scratch("bundled-unreadable");
        std::fs::write(themes.join(INSTALLED), [0xff, 0xfe]).unwrap();
        assert!(install_bundled(&themes).is_err());
        assert!(!themes.join("Nord.json").exists());
        std::fs::remove_dir_all(themes).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_link() {
        let themes = scratch("bundled-link");
        let elsewhere = scratch("bundled-elsewhere");
        let target = elsewhere.join("target.json");
        std::os::unix::fs::symlink(&target, themes.join("Nord.json")).unwrap();
        std::os::unix::fs::symlink(&target, themes.join(".Rose Pine.json.tmp")).unwrap();
        install_bundled(&themes).unwrap();
        assert!(!target.exists());
        assert!(themes.join("Rose Pine.json").is_file());
        std::fs::remove_dir_all(themes).unwrap();
        std::fs::remove_dir_all(elsewhere).unwrap();
    }

    /// The launch's listing already shows the bundled palettes; a later
    /// listing does not put a deleted one back.
    #[test]
    fn the_first_listing_puts_the_bundled_palettes_in_the_picker() {
        let root = scratch("bundled-catalog");
        let themes = root.join("themes");
        let mut catalog = Catalog::default();
        catalog.install_bundled_palettes();
        catalog.start(themes.clone(), None, &Default::default());
        wait(&mut catalog);
        assert_eq!(catalog.themes().len(), BUNDLED.len());
        assert_eq!(catalog.detail(None), "", "the record is not a palette");
        std::fs::remove_file(themes.join("Nord.json")).unwrap();
        catalog.start(themes.clone(), None, &Default::default());
        wait(&mut catalog);
        assert_eq!(catalog.themes().len(), BUNDLED.len() - 1);
        assert!(catalog.find("Nord.json").is_none());

        let mut relaunched = Catalog::default();
        relaunched.install_bundled_palettes();
        relaunched.start(themes, None, &Default::default());
        wait(&mut relaunched);
        assert!(
            relaunched.find("Nord.json").is_none(),
            "a deleted one stays deleted"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_row_waits_for_a_listing_before_calling_a_choice_gone() {
        let mut catalog = Catalog::default();
        assert_eq!(catalog.detail(Some("gone.json")), "");
        catalog.load_test(Vec::new);
        wait(&mut catalog);
        assert!(catalog.detail(Some("gone.json")).contains("no longer"));
        assert_eq!(catalog.detail(None), "");
    }

    fn named(filename: &str, dark: bool, accent: Color32) -> CustomTheme {
        let mut palette = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        palette.accent = accent;
        CustomTheme {
            filename: filename.into(),
            palette,
        }
    }

    #[test]
    fn the_default_pair_is_fastsonics_own_light_and_dark() {
        let pair = ThemePair::default();
        assert!(pair.is_default());
        assert_eq!(pair.palette(false), None);
        assert_eq!(pair.palette(true), None);
        assert_eq!(pair.filenames().count(), 0);
    }

    #[test]
    fn each_side_takes_only_palettes_of_its_base() {
        let latte = named("Latte.json", false, Color32::RED);
        let mocha = named("Mocha.json", true, Color32::BLUE);
        let mut pair = ThemePair::default();
        assert!(!pair.set(true, Some(&latte)));
        assert!(!pair.set(false, Some(&mocha)));
        assert!(pair.is_default());
        assert!(pair.set(false, Some(&latte)));
        assert!(pair.set(true, Some(&mocha)));
        assert_eq!(pair.palette(false), Some(latte.palette));
        assert_eq!(pair.palette(true), Some(mocha.palette));
        assert_eq!(
            pair.filenames().collect::<Vec<_>>(),
            ["Latte.json", "Mocha.json"]
        );
        assert!(pair.set(false, None));
        assert_eq!(pair.palette(false), None);
        assert_eq!(pair.filename(true), Some("Mocha.json"));

        let catalog = Catalog::from_themes(vec![latte, mocha]);
        let names = |dark| {
            catalog
                .pair_themes(dark)
                .map(|theme| theme.filename.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(false), ["Latte.json"]);
        assert_eq!(names(true), ["Mocha.json"]);
    }

    #[test]
    fn a_missing_or_turned_file_keeps_its_last_colours_and_says_so() {
        let mocha = named("Mocha.json", true, Color32::BLUE);
        let mut pair = ThemePair::default();
        pair.set(true, Some(&mocha));
        let empty = Catalog::from_themes(Vec::new());
        assert!(!pair.refresh(&empty));
        assert_eq!(pair.palette(true), Some(mocha.palette));
        assert!(empty.pair_detail(&pair, true).contains("no longer"));
        assert_eq!(empty.pair_detail(&pair, false), "", "an empty side");

        let edited = named("Mocha.json", true, Color32::GREEN);
        let listed = Catalog::from_themes(vec![edited.clone()]);
        assert!(listed.pair_detail(&pair, true).is_empty());
        assert!(
            pair.refresh(&listed),
            "an edited file shows its new colours"
        );
        assert_eq!(pair.palette(true), Some(edited.palette));
        assert!(!pair.refresh(&listed));

        let turned = Catalog::from_themes(vec![named("Mocha.json", false, Color32::RED)]);
        assert!(!pair.refresh(&turned));
        assert_eq!(pair.palette(true), Some(edited.palette));
        assert!(
            turned
                .pair_detail(&pair, true)
                .contains("now a light theme")
        );

        let mut waiting = Catalog::default();
        assert_eq!(waiting.pair_detail(&pair, true), "", "nothing listed yet");
        waiting.load_test(Vec::new);
        wait(&mut waiting);
        assert!(waiting.pair_detail(&pair, true).contains("no longer"));
    }

    #[test]
    fn a_pair_round_trips_and_damage_stays_contained() {
        let mut pair = ThemePair::default();
        pair.set(false, Some(&named("Latte.json", false, Color32::RED)));
        let text = serde_json::to_string(&pair).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("dark").is_none(), "an empty side is left out");
        assert_eq!(serde_json::from_str::<ThemePair>(&text).unwrap(), pair);
        assert_eq!(
            serde_json::from_str::<ThemePair>("{}").unwrap(),
            ThemePair::default()
        );
        // A damaged copy of the colours keeps the choice; a damaged choice
        // empties its side.
        let damaged: ThemePair = serde_json::from_str(
            r#"{"light":{"filename":"Latte.json","palette":"broken"},"dark":{"palette":{}}}"#,
        )
        .unwrap();
        assert_eq!(damaged.filename(false), Some("Latte.json"));
        assert_eq!(damaged.palette(false), None);
        assert_eq!(damaged.dark, None);
    }

    /// Both sides' files, and the single palette, are read ahead of a full
    /// folder, so none of them is pushed out of the listing.
    #[test]
    fn every_saved_choice_is_read_ahead_of_a_full_folder() {
        let dir = scratch("pair-limits");
        for index in 0..MAX_THEMES + 1 {
            std::fs::write(dir.join(format!("{index:03}.json")), b"{}").unwrap();
        }
        std::fs::write(dir.join("zz-light.json"), br#"{"base":"light"}"#).unwrap();
        std::fs::write(dir.join("zz-dark.json"), b"{}").unwrap();
        let mut catalog = Catalog::default();
        catalog.start(
            dir.clone(),
            ["zz-light.json", "zz-dark.json", "zz-light.json"].map(String::from),
            &Default::default(),
        );
        wait(&mut catalog);
        assert!(catalog.find("zz-light.json").is_some());
        assert!(catalog.find("zz-dark.json").is_some());
        assert_eq!(catalog.themes().len(), MAX_THEMES);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

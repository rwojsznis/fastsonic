# Settings & Files

## Where things live

Fastsonic separates preferences, durable state, and disposable caches. On
Linux the default locations are:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/fastsonic/settings.json` | Yes; preferences reset |
| Custom themes | `~/.config/fastsonic/themes/` | Yes; the bundled palettes come back |
| Winamp skins | `~/.config/fastsonic/skins/` | Yes; add them again |
| MilkDrop presets | `~/.config/fastsonic/milkdrop/` | Yes; fetch them again |
| Server credential | `~/.local/state/fastsonic/credentials.json` | Yes; sign in again |
| Last session | `~/.local/state/fastsonic/session.json` | Yes |
| Local play history | `~/.local/state/fastsonic/history.json` | Yes |
| Current log | `~/.local/state/fastsonic/fastsonic.log` | Yes |
| Crash log | `~/.local/state/fastsonic/panic.log` | Yes |
| Audio cache | `~/.cache/fastsonic/audio/` | Always |
| Artwork cache | `~/.cache/fastsonic/art/` | Always |
| Lyrics cache | `~/.cache/fastsonic/lyrics/` | Always |
| Per-account playlist cache | `~/.cache/fastsonic/playlists/` | Always |

On macOS, configuration and state use `~/Library/Application Support/io.github.rwojsznis.fastsonic`
and caches use `~/Library/Caches/io.github.rwojsznis.fastsonic`. On Windows,
configuration uses `%APPDATA%\\github.rwojsznis\\fastsonic\\config`, state uses
`%LOCALAPPDATA%\\github.rwojsznis\\fastsonic\\data`, and caches use the sibling
`cache` directory.

`session.json` also keeps the window's size and position. A window left
maximized or full screen reopens that way; the remembered size and position
describe an ordinary window and are not applied to one that already fills the
screen, because sizing or moving it would restore it down. Quitting while
lyrics are full screen returns the next window to the mode it had before
them, rather than starting full screen without the lyrics.

In memory, Fastsonic keeps the 12 playlists, 16 albums and 10 artists you
opened last, and the details of 800 songs, so a long session does not grow
without bound; the open page, what is playing, and playlists with an edit on
its way are always kept. A page dropped from memory is read again when you
open it. Besides the artwork itself, up to 64 softened 256-pixel covers, made
from sidebar thumbnails to stand in for a page's cover while it loads, are kept
in memory; nothing about them is written to disk.

A playlist read in full is kept in the playlist cache under the server's
`changed` time. Opening it again shows the cached songs at once only if that
time still matches and the cache holds as many songs as the server counts;
otherwise the cache is dropped and the songs load from the server.

Clearing caches never signs you out. `credentials.json` contains a salted
Subsonic token, not the password, plus a short-lived Navidrome session when
available. Treat it like a password and do not share it. Settings → Storage
shows the exact paths and provides cache/history controls.

## Important settings

`settings.json` is readable JSON, written atomically, and accepts missing or
unknown fields for forward and backward compatibility. The server address and
username are deliberately not settings: they live with the credential so a
copied preferences file contains no account-specific connection details.

Audio settings select the output device, Windows buffer size, ReplayGain
normalisation, and whether the block cache is enabled with a 512 MB, 1 GB, or
4 GB budget. Fastsonic streams source files as-is and plays supported contexts
gaplessly. There is no audio-quality selector because the source file is not
transcoded, and there is no autoplay source after a context ends.

Interface settings cover theme, album-art accents, compact rows, shortcut
hints, sidebar state, zoom, player bar visualizer, Winamp skin/random
selection/windows/equalizer, and MilkDrop. The sidebar's Library keeps
`library_sort`, the order chosen for each section (`playlists`, `albums`,
`artists`), as one of `library`, `recently_played`, `name`, `creator`,
`recently_added` or `local` where the section supports it; a section missing
from it keeps the order it had before sorts could be chosen, and an entry this
version does not know is ignored. `sidebar_order` is the dragged playlist
arrangement, an unpinned Liked Songs included, kept while another order is
chosen, and `pinned_contexts` the pins, in order. Both name Liked Songs by
its context URI, `sonic:collection:songs`. `liked_songs_pinned` (on by
default) keeps Liked Songs among the pins; an older file places it first
until it is moved. `sidebar_grid` (off by default, so older files keep the
list) shows the Library as cover cards instead of rows. On
Linux, Follow system reads the light or dark preference from the desktop portal.
The theme can also be a palette file of your own; see
[Custom themes](#custom-themes).
On Windows the main window uses the standard title bar; **Custom title bar**
(`custom_titlebar`) draws Fastsonic's own title bar and window buttons instead,
and changing it replaces only the native window. On Windows, **Show in
taskbar** under Winamp skins (`winamp_show_taskbar`, on by default) decides
whether the mini player keeps a taskbar button; the main window always keeps
its own. Linux X11 sessions offer the same switch; Wayland and macOS do not.
On Linux, **Middle-click autoscroll** (`middle_click_autoscroll`, off by
default) lets a middle click on a list scroll it as the pointer moves; it
starts off because Linux desktops usually paste the primary selection on
middle click, and older settings files read it as off. Windows always
autoscrolls and macOS never does, so neither shows the switch.
Close
to tray and daily GitHub update checks are enabled by default and can be
disabled. **Check for updates** asks straight away and reports the answer
either way, including when the version installed is the current one; on macOS
the application menu asks the same question. When a newer release
exists, a badge beside Search names it and opens its GitHub release; in a
narrow window it shows only its icon, and hovering it gives the version, so
Search keeps its room.

## Custom themes

**Settings → Appearance → Theme** lists **Follow system**, **Light** and
**Dark**, then, after a separator, the palette files in the `themes` folder
beside `settings.json`: `~/.config/fastsonic/themes/` on Linux,
`~/Library/Application Support/io.github.rwojsznis.fastsonic/themes/` on
macOS, and `%APPDATA%\\github.rwojsznis\\fastsonic\\config\\themes\\` on
Windows. **Open themes folder** beside the picker creates the folder if needed
and opens it in your file manager, and **How to make a theme** opens this
section. A palette is listed by its filename without `.json` (`Gruvbox.json`
is **Gruvbox**), and picking it applies it at once. Choosing Follow system,
Light or Dark sets the palette aside again. A theme changes colours only;
fonts and layout stay as they are, and Winamp skins keep their own look.

The first time Fastsonic starts, it puts eight palettes in the folder:
Catppuccin, Catppuccin Latte, Nord, Ristretto, Rose Pine, Rose Pine Dawn,
Rose Pine Moon and Tokyo Night. They are ordinary palette files: read them to
see how a theme is written, change them, or delete the ones you do not want.
Fastsonic never rewrites them, and a deleted one stays deleted;
`.installed-palettes` in the folder records which it has already put there. A
palette added in a later version arrives once, and never over a file of yours
with the same name.

A palette file is a UTF-8 JSON file ending in `.json`. It names a `base`,
`dark` (the default) or `light`, and the `colors` it changes from that base;
any colour it leaves out is the base's own, so a file can be as short as one
colour. The base also decides whether egui's own controls, such as text
fields and scroll bars, are drawn light or dark. For example,
`themes/Gruvbox.json`:

```json
{
  "base": "dark",
  "colors": {
    "window": "#282828",
    "panel": "#1d2021",
    "surface": "#32302f",
    "surface_hover": "#3c3836",
    "surface_active": "#504945",
    "outline": "#3c3836",
    "text": "#ebdbb2",
    "secondary": "#bdae93",
    "dim": "#928374",
    "accent": "#b8bb26",
    "accent_hover": "#98971a",
    "on_accent": "#282828",
    "danger": "#fb4934",
    "warning": "#fabd2f",
    "overlay": "#32302f",
    "shadow": "#0000008c"
  }
}
```

Each colour is `#RRGGBB`, or `#RRGGBBAA` with an alpha channel. These are all
of them:

| Colour | What it paints |
| --- | --- |
| `window` | The page background, and the label of a chosen button |
| `panel` | The sidebar, the player bar, and the queue and lyrics panels |
| `surface` | Cards, buttons, fields and the settings sections |
| `surface_hover` | A surface under the pointer |
| `surface_active` | A surface being pressed, and the empty part of a slider |
| `outline` | Borders and separators |
| `text` | Titles, labels and icons |
| `secondary` | Descriptions, artists and other quieter text |
| `dim` | The quietest text and icons, and anything unavailable |
| `accent` | Primary buttons, switches, a liked heart, shuffle and repeat when on |
| `accent_hover` | An accent control under the pointer |
| `on_accent` | Text and icons drawn on the accent |
| `danger` | Errors |
| `warning` | Warnings |
| `overlay` | Menus, dialogs and notifications |
| `shadow` | The shadow under menus and dialogs, usually with alpha |

**Colour from album art** tints pages with the playing cover over any theme;
turn it off to keep a palette's colours everywhere.

The folder is read in the background when Fastsonic starts, when Settings
opens, and when the Theme picker opens, so a palette added or edited while the
app runs shows up there without a restart, and an edit to the chosen palette
is applied. Subfolders and links are not followed, each file may be up to
64 KiB, and up to 128 palettes in a folder of at most 512 entries are listed.

A file that cannot be read, because it is not valid JSON, names a colour or
field that does not exist, or has a colour in another format, is left out of
the picker, and the Theme row says which file and why; the log has every
one. Fastsonic keeps a copy of the chosen palette in `settings.json`
(`custom_theme` names the file, `custom_theme_cache` holds its colours), so it
starts in those colours before the folder has been read. If the chosen file is
later deleted or broken, its last colours stay, including across restarts, and
the Theme row explains why until you fix the file or choose another theme. A
damaged copy is ignored without resetting any other setting.

## Home shelves

Each of Home's shelves can be hidden on its own. Quit Fastsonic before editing
`settings.json`, then start it again. This hides the random shelf and your top
artists:

```json
"home": {
  "random": { "visible": false },
  "top_artists": { "visible": false }
}
```

The shelves are `recently_added`, `recently_played`, `top_songs`,
`most_played`, `top_artists` and `random`. Set `visible` back to `true`, or
remove the entry, to show a shelf again; anything left out stays visible. The
shortcuts at the top of Home are not a shelf and always show. Hidden shelves
still refresh in the background, so this changes only what is drawn.

## Command line

`fastsonic -v` logs more from the audio engine and server API client. Attach
`fastsonic.log` to bug reports and `panic.log` after a crash; credential values
are redacted.

On Linux, control the running player over MPRIS, for example:

```sh
playerctl --player=fastsonic play-pause
```

MPRIS has no verb for starring a song, so `fastsonic like` stars or unstars the
playing one in the running instance on Linux too.

On macOS and Windows, `fastsonic play-pause`, `play`, `pause`, `next`,
`previous`, `seek`, `seek-to`, `volume`, `volume-up`, `volume-down`, `mute`,
`shuffle`, `repeat`, `like`, `play-uri`, `now-playing`, and `show` address the
running instance. Run `fastsonic --help` or a subcommand's `--help` for exact
arguments.

## Demo mode

Builds made with `--features demo` accept `--demo` for deterministic sample
data and no server connection. `--demo-page` opens a named page;
`--demo-show` adds comma-separated panels or states (among them `rtl`, for
right-to-left titles, `signed-out` and `connecting`, for the sign-in card,
`library-list`, `library-list-narrow`, `library-list-wide`, `library-grid`,
`library-grid-narrow` and `library-grid-wide`, for the Library as rows or
cards in a 380, 230 or 440 pixel sidebar with the artwork collapsed, and
`collection-loading`, for a playlist, album or artist page still waiting for
its details); and `--demo-shot <PATH>`
writes a PNG after the optional `--demo-shot-delay <MS>`. Demo mode runs in a
profile of its own under the system temporary directory, so it never reads the
saved sign-in, never contacts your server, and leaves your settings, session,
history, caches and log alone.

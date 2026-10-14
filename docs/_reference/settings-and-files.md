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

The session is saved shortly after it changes, including when a new song
starts, and every 30 seconds while music plays, so a crash or a power cut
resumes close to where you were.

Settings, the session, the play history, the credential and the playlist
cache's files are never rewritten in place. Each is written to a temporary
file beside it (a hidden `.<name>.<number>.tmp`), flushed to the disk, and
then moved over the old file, so a crash or a power cut leaves the old
contents or the new ones, never half of either; a write that fails removes
its temporary file and leaves the old file as it was. On Linux and macOS,
`history.json` and `credentials.json` are written readable by your account
only.

The session and the play history are also written just before the computer
sleeps (Linux and Windows) and when you sign out or shut down (Windows). On
Linux, Fastsonic takes a short logind "delay" lock for this, listed by
`systemd-inhibit --list`; inside Flatpak it needs
`--system-talk-name=org.freedesktop.login1`.

`session.json` also keeps the window's size and position. A window left
maximized or full screen reopens that way; the remembered size and position
describe an ordinary window and are not applied to one that already fills the
screen, because sizing or moving it would restore it down. Quitting while
lyrics are full screen returns the next window to the mode it had before
them, rather than starting full screen without the lyrics. A window that
opens on no connected monitor, because its display was unplugged or comes
up late, is moved to the middle of the primary one on its first frames;
Wayland does not tell the app where its windows are, so nothing moves
there.

In memory, Fastsonic keeps the 12 playlists, 16 albums and 10 artists you
opened last, and the details of 800 songs, so a long session does not grow
without bound; the open page, what is playing, and playlists with an edit on
its way are always kept. A page dropped from memory is read again when you
open it. A cover on screen is held only as a texture scaled to the pixels it
covers; its downloaded and decoded copies are let go of once it is drawn, and
read back from the artwork cache when another size of it is needed. Textures
and the downloads still waiting to be drawn share a 64 MB budget, the
longest unseen artwork going first. Each size of a cover asked of the server
is its own file in the artwork cache. Besides the artwork itself, up to 64
softened 256-pixel covers, made from the 64-pixel thumbnails of sidebar rows
to stand in for a page's cover while it loads, are kept in memory; nothing
about them is written to disk beyond the thumbnail itself.

A playlist read in full is kept in the playlist cache under the server's
`changed` time. Opening it again shows the cached songs at once only if that
time still matches and the cache holds as many songs as the server counts;
otherwise the cache is dropped and the songs load from the server.

Clearing caches never signs you out. `credentials.json` contains a salted
Subsonic token, not the password, plus a short-lived Navidrome session when
available. Treat it like a password and do not share it; on Linux and macOS
Fastsonic writes it readable by your account only. Settings → Storage
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
selection/windows/equalizer, and MilkDrop. `player_bar_vis_click` (on by
default, so older files keep it) lets a click on the player bar's empty space
cycle `player_bar_vis` through `off`, `spectrum` and `waveform`.
`visualizer_fps` (60 by default) is how many frames a second the player bar's
visualizer and the mini player's analyser draw; `0` is uncapped, every
refresh of the display, or 60 a second on Wayland, where the window does not
wait for the display. It is halved while the window is out of focus or the
system saves energy, held to at most 30 while the system saves energy, and no
frames are drawn while nobody can see the window. The screen rate its dial
offers is asked of the monitor showing the window and is not saved; until
that monitor has answered, the rate MilkDrop's window last reported
(`milkdrop_screen_hz`) stands in. The interface
zoom (`zoom`) belongs to the main window: each main window opens at it, and
the mini player keeps its own scale. MilkDrop's frame rate (`milkdrop_fps`,
`0` for uncapped) is held to at most 30 while the system saves energy, and to
one a second while the display is off or the screen is locked; the setting
itself is left as you chose it. The sidebar's Library keeps
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
list) shows the Library as cover cards instead of rows. `library_by_you` (off
by default) keeps only the signed-in account's own playlists, and Liked
Songs, in the Playlists section. On
Linux, Follow system reads the light or dark preference from the desktop portal.
The theme can also be a palette file of your own; see
[Custom themes](#custom-themes).
On Windows and Linux the main window uses the desktop's title bar; **Custom
title bar** (`custom_titlebar`) draws Fastsonic's own title bar and window
buttons instead, and changing it replaces only the native window. On GNOME
the buttons follow the desktop's button layout, left or right and only those
it shows, drawn with its icon theme's pictures; both come from the desktop
portal, so a Flatpak sees them too. Other Linux desktops get Minimize,
Maximize and Close on the right. On Windows, **Show in taskbar** under Winamp
skins (`winamp_show_taskbar`, on by default) decides
whether the mini player keeps a taskbar button; the main window always keeps
its own. Linux X11 sessions offer the same switch; Wayland and macOS do not.
On Linux, **Middle-click autoscroll** (`middle_click_autoscroll`, off by
default) lets a middle click on a list scroll it as the pointer moves; it
starts off because Linux desktops usually paste the primary selection on
middle click, and older settings files read it as off. Windows always
autoscrolls and macOS never does, so neither shows the switch.
On macOS,
**MacBook notch widget** (`mac_notch_widget`, off by default, and off in an
older settings file) shows a Now Playing card under the notch of a notched
display while Fastsonic is in the background; see
[MacBook notch](../_guide/using-fastsonic.md#macbook-notch). Its cover is
the one the system's media controls are handed, so it adds no request to the
server. Close
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
| `tint` | The wash at the top of a page that album art does not colour |

`tint` is the colour the top of a page fades from when album art does not
colour it: with **Colour from album art** off, or when neither the page nor
the playing song has a cover to take a colour from. Its alpha is the
strength, so `#00000018` is a faint dark wash and `#ffffff20` a faint light
one. Leave it out for a plain page, as every bundled palette does. Like a
cover's colour, the wash scrolls away with the top of the page.

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

MPRIS has no verb for starring a song or for playing one by name, so
`fastsonic like` and `fastsonic play-search` reach the running instance on
Linux too.

On macOS and Windows, `fastsonic play-pause`, `play`, `pause`, `next`,
`previous`, `seek`, `seek-to`, `volume`, `volume-up`, `volume-down`, `mute`,
`shuffle`, `repeat`, `like`, `play-uri`, `play-search`, `now-playing`, and
`show` address the running instance. Run `fastsonic --help` or a subcommand's
`--help` for exact arguments.

`fastsonic play-search hey jude` searches your server and plays the best
match, the way a launcher or a voice assistant asks for something by name:
`play-uri` needs an id that only a search can find. With `--kind album`,
`artist`, or `playlist` it plays one of those from the start instead of a
song. A song, album or artist comes from the server's search, and one named
exactly what you typed is preferred over the server's first result; a
playlist is one of yours whose name is, starts with, or holds the words. When
nothing matches, the app says so. A match that arrives after a newer search,
or after you have played, paused or skipped something since, is dropped. The
words are limited to 200 characters.

## Demo mode

Builds made with `--features demo` accept `--demo` for deterministic sample
data and no server connection. `--demo-page` opens a named page;
`--demo-show` adds comma-separated panels or states (among them `rtl`, for
right-to-left titles, `signed-out` and `connecting`, for the sign-in card,
`library-list`, `library-list-narrow`, `library-list-wide`, `library-grid`,
`library-grid-narrow` and `library-grid-wide`, for the Library as rows or
cards in a 380, 230 or 440 pixel sidebar with the artwork collapsed,
`library-by-you`, for the playlists with **By You** on, and
`collection-loading`, for a playlist, album or artist page still waiting for
its details); and `--demo-shot <PATH>`
writes a PNG after the optional `--demo-shot-delay <MS>`. Demo mode runs in a
profile of its own under the system temporary directory, so it never reads the
saved sign-in, never contacts your server, and leaves your settings, session,
history, caches and log alone.

# Settings & Files

## Where things live

Fastsonic separates preferences, durable state, and disposable caches. On
Linux the default locations are:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/fastsonic/settings.json` | Yes; preferences reset |
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
hints, sidebar state, zoom, Winamp skin/windows/equalizer, and MilkDrop. Close
to tray and daily GitHub update checks are enabled by default and can be
disabled. **Check for updates** asks straight away and reports the answer
either way, including when the version installed is the current one; on macOS
the application menu asks the same question.

## Command line

`fastsonic -v` logs more from the audio engine and server API client. Attach
`fastsonic.log` to bug reports and `panic.log` after a crash; credential values
are redacted.

On Linux, control the running player over MPRIS, for example:

```sh
playerctl --player=fastsonic play-pause
```

On macOS and Windows, `fastsonic play-pause`, `play`, `pause`, `next`,
`previous`, `seek`, `seek-to`, `volume`, `volume-up`, `volume-down`, `mute`,
`shuffle`, `repeat`, `like`, `play-uri`, `now-playing`, and `show` address the
running instance. Run `fastsonic --help` or a subcommand's `--help` for exact
arguments.

## Demo mode

Builds made with `--features demo` accept `--demo` for deterministic sample
data and no server connection. `--demo-page` opens a named page;
`--demo-show` adds comma-separated panels or states; and `--demo-shot <PATH>`
writes a PNG after the optional `--demo-shot-delay <MS>`. Demo mode does not
write settings, session, or history files.

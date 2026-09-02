---
title: Settings & Files
description: Where Fastsonic keeps configuration, credentials, and caches, and what is safe to delete.
nav_order: 0
---

## Where things live

Fastsonic follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/fastsonic/settings.json` | Yes, you lose preferences |
| Winamp skins | `~/.config/fastsonic/skins/` | Yes, you add them again |
| MilkDrop presets | `~/.config/fastsonic/milkdrop/` | Yes, you fetch them again |
| Shared Web API sign-in | `~/.local/state/fastsonic/shared_web_api_token.json` | Yes, you sign in again |
| Personal Web API sign-in | `~/.local/state/fastsonic/personal_web_api_token.json` | Yes, the personal app is disabled |
| Playback credential | `~/.local/state/fastsonic/credentials/` | Yes, you approve playback again |
| Last session | `~/.local/state/fastsonic/session.json` | Yes |
| Play history | `~/.local/state/fastsonic/history.json` | Yes |
| Audio cache | `~/.cache/fastsonic/audio/` | Always |
| Artwork cache | `~/.cache/fastsonic/art/` | Always |
| Lyrics cache | `~/.cache/fastsonic/lyrics/` | Always |
| Account-scoped playlist cache | `~/.cache/fastsonic/playlists/<account-id>/` | Always |
| Last run's log | `~/.local/state/fastsonic/fastsonic.log` | Always |
| Crash log | `~/.local/state/fastsonic/panic.log` | Always |

Clearing caches never signs you out; credentials live in *state*, not
*cache*. Web API token files are written with owner-only permissions.
Signing out from Settings deletes both Web API grants and the separate
playback credential.

On macOS, settings, state, and the logs are in
`~/Library/Application Support/io.github.rwojsznis.fastsonic` and the caches in
`~/Library/Caches/io.github.rwojsznis.fastsonic`. On Windows, settings are in
`%APPDATA%\github.rwojsznis\fastsonic\config`, state and the logs in
`%LOCALAPPDATA%\github.rwojsznis\fastsonic\data`, and the caches in
`%LOCALAPPDATA%\github.rwojsznis\fastsonic\cache`.

## settings.json

Settings are stored in one readable JSON file and written atomically. Its
main fields are:

| Field | Default | Meaning |
| --- | --- | --- |
| `bitrate` | `320` | 96, 160, or 320 kbps |
| `normalisation` | `false` | Volume normalisation |
| `autoplay` | `true` | Keep playing similar music at the end |
| `gapless` | `true` | Gapless playback |
| `audio_backend` | platform | `pulseaudio` or `rodio` on Linux |
| `audio_cache_mb` | `1024` | On-disk audio cache budget |
| `theme` | `dark` | `dark`, `light`, or `system` |
| `accent_from_art` | `true` | Tint pages with album art |
| `sidebar_compact` | `false` | Names only in the library sidebar, no covers |
| `tracklist_compact` | `false` | One-line track rows without covers |
| `winamp_window` | `false` | The window is the Winamp mini player |
| `skin` | none | File or folder name in the skins folder; blank uses the built-in skin |
| `skin_scale` | by display | Screen pixels per skin pixel, 1 to 4 |
| `winamp_on_top` | `false` | Keep the mini player above other windows |
| `vis` | `bars` | The mini player's visualiser: `bars`, `scope`, or `off` |
| `playlist_open` | `false` | The playlist window is open under the mini player |
| `playlist_height` | `174` | The playlist window's height in skin pixels |
| `eq_open` | `false` | The equalizer window is open under the mini player |
| `eq_on` | `false` | The equalizer shapes local playback |
| `eq_preamp_db` | `0` | The preamp, in decibels, -12 to 12 |
| `eq_bands_db` | ten zeros | The bands from 60 Hz to 16 kHz, in decibels, -12 to 12 |
| `balance` | `0` | Left to right, -1 to 1, for local playback |
| `mono` | `false` | Play both channels the same |
| `playlist_shaded` | `false` | The playlist window is rolled up to its title bar |
| `winamp_shaded` | `false` | The main window is rolled up to its title bar |
| `milkdrop_open` | `false` | The MilkDrop window is open |
| `milkdrop_seconds` | `30` | How long each MilkDrop preset plays |
| `milkdrop_fps` | `60` | MilkDrop frame rate; `0` is uncapped |
| `milkdrop_screen_hz` | `0` | Last reported display refresh rate |
| `milkdrop_fullscreen` | `false` | The MilkDrop window fills the screen |
| `milkdrop_size` | `640, 480` | The MilkDrop window's size in points |
| `keep_playing_in_background` | `true` | Close to tray |
| `check_for_updates` | `true` | Ask GitHub once a day for a newer release |
| `web_client_id` | none | Optional personal Spotify app id used alongside shared coverage |

## Command line

```
fastsonic [OPTIONS]

  -v, --verbose         More logs from librespot and the API client
```

Attach `fastsonic.log` from the state directory to bug reports. It contains
the last run's output, including extra lines from `fastsonic -v`. After a
crash, attach `panic.log` too.

## Demo mode

Builds made with `cargo build --features demo` accept `--demo`, which loads
sample data for screenshots and interface work. Demo mode never writes
settings.

`--demo-page` opens a page, such as `home`, `playlist:pl1`, `album:alb0`, or
`artist:art0`, and `--demo-show` adds surfaces on top of it: a comma
separated list of `queue`, `recents`, `lyrics`, `shortcuts`,
`create`, `light`, `focus`, `resume`, `winamp`, `playlist`, `eq`, `eq-shade`,
and `compact`.

`--demo-shot <PATH>` writes the window to a PNG and exits, which is how the
screenshots in these pages are made:

```
cargo run --release --features demo -- \
  --demo-shot docs/screenshot.png --demo-page playlist:pl1 --demo-show queue
```

The image uses the current window size. `--demo-shot-delay <MS>` sets how long
to wait for cover art before taking it.

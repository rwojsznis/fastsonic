---
title: Everyday Use
description: Playing music, managing your library and playlists, devices, the queue, the tray, and every keyboard shortcut.
nav_order: 3
---

## Playing music

Playlist, album, artist, and podcast pages have play buttons. Cards show one
when you hover, and each row has its own. Double-click a row to start playback
from that song within its playlist or album. The shuffle button next to a
page's play button starts the page in shuffled order.

The player bar shows what is playing locally or on another device. Click the
title to open its album, the artist name to open the artist, or the heart to
save the track.

## Home

Home previews your most-played songs. Select **Your top songs** or **Show
more top songs** to open the complete ranked list.

Track tables sort by their column headings: click **Title**, **Album**,
**Date added**, or the clock to sort by it, again to reverse, and a third
time to return to the list's own order.

## Your Library

The sidebar sorts playlists by when you last played them and preserves that
order between runs.

Use the chips to filter the sidebar by Playlists, Albums, Artists, or Podcasts,
or use the magnifier to search it. Liked Songs stays at the top. The current
page is highlighted, and the playing playlist has a small speaker icon.

**Playlists you own** are fully editable: create one with the **+** button,
add songs from any row's menu, remove and reorder from the playlist page,
and rename or delete from its context menu. Playlists you follow can be
followed and unfollowed.

## Search

Ctrl+F (or `/`) focuses search from anywhere. Results are grouped into top
result, songs, artists, albums, playlists, podcasts, and episodes. Use the
chips to show one type. The empty search page lists recent searches.

## Devices and the queue

The speaker icon in the player bar lists every Spotify Connect device on
your account. Click one and the music moves there mid-song; the same
controls keep working. "Playing on …" in the top bar reminds you when sound
is coming out of something across the room.

The queue lives behind the list icon, as a side panel or a full page. Add
anything to it from a row's context menu.

### Receivers on the local network

A receiver running librespot or spotifyd, and some hardware speakers, appears
in Spotify's device list only after it has received an account credential.
Before then, the Web API cannot see it.

Fastpotify searches the local network when you open the device picker. It
lists discovered receivers as *on your network*. Choose one to send it the
stored playback credential, encrypted so that only that receiver can read it.
Once connected, it appears as an ordinary Spotify Connect device and playback
moves to it.

This uses the credential stored for playing on this computer, so enable
playback here first (see [Getting Started](/getting-started/)). Receivers
that ask for a different kind of login are not connected this way yet.

## Lyrics

The microphone button in the player bar (or `L`) opens lyrics for the playing
track beside the page. For timed lyrics, the current line is
highlighted and the panel scrolls automatically; click a line to seek to it.
Manual scrolling pauses automatic following, and **Follow** resumes it.
Fastpotify requests lyrics from Spotify when local playback is authorized.
Otherwise, or when Spotify has no lyrics for a track, it uses
[LRCLIB](https://lrclib.net), an open database that needs no account. Podcasts
and tracks without a transcription show an unavailable message.

![The lyrics panel beside a playlist, following the song](/assets/images/lyrics.png)

## The tray

Closing the window keeps the music playing: Fastpotify stays in the system
tray with play, pause, skip, and quit in its menu, and clicking the icon
brings the window back. On Linux it is a standard status-notifier item, so
it works in any bar that shows tray icons, and MPRIS keeps `playerctl`,
media keys, and your desktop's players widget working the whole time.

## One window, one instance

Starting Fastpotify while it is already running brings the existing window
forward instead of opening a second instance. This avoids duplicate Spotify
Connect devices and conflicting media-key handlers.

## Keyboard shortcuts

| Shortcut | What it does |
| --- | --- |
| `Space` | Play or pause |
| `Ctrl+←` / `Ctrl+→` | Previous or next |
| `Shift+←` / `Shift+→` | Seek 10 seconds |
| `Ctrl+↑` / `Ctrl+↓` | Volume |
| `M` | Mute |
| `S` / `R` | Shuffle / cycle repeat |
| `Q` | Queue panel |
| `Ctrl+F` or `/` | Search |
| `Alt+←` / `Alt+→` | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+,` | Settings |
| `Ctrl+/` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Settings (Ctrl+,) includes the Connect device name, audio quality up to
320 kbps, volume normalisation, autoplay, gapless playback, the audio backend
on Linux, the audio cache size, themes, album-art tinting, and close-to-tray
behaviour. Applying playback settings restarts the local player. Other
settings take effect immediately.

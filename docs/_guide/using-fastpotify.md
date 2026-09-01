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

If nothing is playing when Fastpotify starts, the player bar shows the last
song at its saved position. Press play to resume. Next, previous, and seek also
work before playback starts.

Local playback uses the system's default audio output. Change the output in
your system sound settings.

## Home

Home previews your most-played songs. Select **Your top songs** or **Show
more top songs** to open the complete ranked list.

Click a track-table heading to sort by it. Click again to reverse the order,
and a third time to restore the original order. Recent additions show relative
dates; entries at least a month old show calendar dates.

![A playlist showing relative dates for recent additions](/assets/images/date-added-relative-demo.png)

## Your Library

Pinned entries appear below Liked Songs. Pin one from its context menu, drag
it into the pinned group, or drag it out to unpin it. Drag pinned entries to
reorder them.

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

Spotify playlist folders appear in the sidebar when it uses the default order.
Click a folder to fold it closed or open it again. Fastpotify remembers which
folders are closed. Filtering or setting a custom order shows a flat list.

The Albums, Artists, and Podcasts shelves pin the same way: drag into
the block, within it, or below it.

Use the chips to filter the sidebar by Playlists, Albums, Artists, or Podcasts,
or use the magnifier to search it. Liked Songs stays at the top. The current
page is highlighted, and the playing playlist has a small speaker icon.

You can create and edit your own playlists. Add songs from a row menu or drag
them to a playlist in the sidebar. Remove and reorder songs on the playlist
page. Rename or delete a playlist from its context menu. Reordering is disabled
while the table is sorted or filtered. Drop a song on Liked Songs to save it.

### Picking out several songs

On a playlist, album, or Liked Songs, click a song to select it. Ctrl-click
(Cmd-click on macOS) adds another. Shift-click selects a range. Right-click a
selected song to act on the whole selection: play next, save, remove, or add
to a playlist. Press Escape to clear the selection.

Selecting songs does not play them. Double-click a song or click its number
to play it. Sorting or filtering clears the selection.

## Search

Ctrl+F (or `/`) focuses search from anywhere. Results are grouped into top
result, songs, artists, albums, playlists, podcasts, and episodes. Use the
chips to show one type. The empty search page lists recent searches.

## Devices and the queue

The speaker icon in the player bar lists every Spotify Connect device on
your account. Click one and the music moves there mid-song; the same
controls keep working. "Playing on …" in the top bar reminds you when sound
is coming out of something across the room.

The list icon opens the queue as a side panel or full page. Choose *Play next*
from a row's context menu to add a song. Your songs appear under *Playing
next*, before the current playlist or album under *Next up*. Double-click a
song to jump to it. The trash icon clears the songs you added, but only when
this computer is playing. Spotify does not let Fastpotify clear another
device's queue.

Fastpotify saves the queue when it closes. The list-plus icon saves it as a
new playlist with duplicate songs removed. *Go to song radio* starts a Spotify
station and opens its queue. The full rules are in
[The Queue's Rules](/queue/).

### Recent

The panel's second tab lists what you have listened to, newest first.

The list combines Spotify's history with Fastpotify's local history. Spotify
does not record tracks played through third-party clients, so Fastpotify saves
those tracks itself.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.

### Receivers on the local network

A receiver running librespot or spotifyd, and some hardware speakers, appears
in Spotify's device list only after it has received an account credential.
Before then, the Web API cannot see it.

Fastpotify searches the local network when you open the device picker.
Discovered receivers are marked *on your network*. Select one to send it the
stored playback credential. The credential is encrypted for that receiver.
After connecting, it appears as a Spotify Connect device.

This uses the credential stored for playing on this computer, so enable
playback here first (see [Getting Started](/getting-started/)). Receivers
that ask for a different kind of login are not connected this way yet.

## Lyrics

The microphone button in the player bar (or `L`) opens lyrics for the playing
track. Timed lyrics scroll with the song and highlight the current line. Click
a line to seek. Manual scrolling pauses this; **Follow** starts it again.
Fastpotify requests lyrics from Spotify when local playback is authorized.
Otherwise, or when Spotify has no lyrics for a track, it uses
[LRCLIB](https://lrclib.net), an open database that needs no account. Podcasts
and tracks without a transcription show an unavailable message.

![The lyrics panel beside a playlist, following the song](/assets/images/lyrics.png)

## The Winamp mini player

Ctrl+M (Cmd+Shift+M on macOS) opens the Winamp mini player. See the
[Winamp guide](/winamp/) for skins and controls.

## MilkDrop

Ctrl+Shift+K opens the MilkDrop visualiser in a separate window. See the
[MilkDrop guide](/milkdrop/) for presets and controls.

## The tray

Closing the window keeps the music playing: Fastpotify stays in the system
tray with play, pause, skip, and quit in its menu, and clicking the icon
brings the window back. On Linux it is a standard status-notifier item, so
it works in any bar that shows tray icons, and MPRIS keeps `playerctl`,
media keys, and your desktop's players widget working the whole time.

## One window, one instance

Starting Fastpotify again brings the existing window forward.

## Keyboard shortcuts

| Shortcut | What it does |
| --- | --- |
| `Space` | Play or pause |
| `Ctrl+←` / `Ctrl+→` | Previous or next |
| `Shift+←` / `Shift+→` | Seek 10 seconds |
| `Ctrl+↑` / `Ctrl+↓`, or the wheel over the volume slider | Volume |
| `M` | Mute |
| `S` / `R` | Shuffle / cycle repeat |
| `Q` | Queue panel |
| `Ctrl+F` or `/` | Search |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+←` / `Alt+→`, or the mouse's back and forward buttons | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+M` | Winamp mini player |
| `Ctrl+W` | Close the window (the tray keeps playing, if that is on) |
| `Ctrl+,` | Settings |
| `Ctrl+/` or `?` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Settings (Ctrl+,) includes playback, audio, appearance, Winamp, MilkDrop,
equalizer, storage, and update options. Playback changes need a local-player
restart. Other settings apply immediately.

### Output buffer

The **Output buffer** controls how much audio Fastpotify sends at once. The
default is 100 ms. Try 200 ms if you hear clicks or crackles. Use 50 ms for
faster pause and skip response. Press **Apply and restart playback** after
changing it. Unsupported values use the nearest available size.

---
title: Everyday Use
description: Library ordering, selecting songs, the queue, play history, shortcuts, and audio buffering.
nav_order: 3
---

## Library order

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

Albums, artists, and podcasts pin the same way. Playlist songs can be reordered
only when the table is not sorted or filtered.

## Selecting several songs

On a playlist, album, or Liked Songs, click a song to select it. Ctrl-click
(Cmd-click on macOS) adds another. Shift-click selects a range. Right-click a
selected song to act on the whole selection: play next, save, remove, or add
to a playlist. Press Escape to clear the selection.

Selecting songs does not play them. Double-click a song or click its number
to play it. Sorting or filtering clears the selection.

## Queue

Songs added with **Play next** appear under **Playing next**, before the
playlist or album under **Next up**. Fastpotify saves this queue when it closes.
It can clear the songs you added, but Spotify does not let it remove one queued
song or clear another device's queue.

The list-plus button saves the queue as a playlist without duplicate songs.
**Go to song radio** starts a Spotify station and opens its queue. See
[The Queue's Rules](/queue/) for the complete behavior.

## Recent

The queue panel's second tab combines Spotify's history with tracks played
through Fastpotify, which Spotify does not record.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.

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

## Output buffer

The **Output buffer** controls how much audio Fastpotify sends at once. The
default is 100 ms. Try 200 ms if you hear clicks or crackles. Use 50 ms for
faster pause and skip response. Press **Apply and restart playback** after
changing it. Unsupported values use the nearest available size.

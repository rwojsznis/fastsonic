# Everyday Use

## Library and search

Home combines recently added albums, recently played albums, top and most
played songs, top artists, and a random shelf. Some personalisation depends on
Navidrome's native API or its Last.fm integration and can be empty without
affecting browsing or playback.

Search covers the songs, albums, artists, and playlists indexed by your
server. Right-click rows and cards to star music, add songs to a playlist, or
put a song in the queue. The playlist submenu filters as you type.

## Sidebar order

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

## Queue

Songs added with **Add to queue** appear after songs already queued and before
the rest of the current album or playlist. The queue names the playing album,
playlist, artist, or Liked Songs and links back to it. Clear removes only those
manually queued songs. The player owns the queue, so every change is visible on
the next frame and needs no server round-trip. The complete contract is in
[The Queue's Rules](../_reference/queue.md).

Local Play and Pause fade smoothly. The visualizers still follow the
post-equalizer, pre-volume signal, so transport and volume changes do not move
the picture.

## Recent plays

The queue panel's second tab combines the server's recent songs with tracks
played through Fastsonic. Server rows may not include an exact play time;
local rows do.

A song enters local history after about 30 seconds, or halfway through a
shorter song. Paused time and seeking do not count. Fastsonic also scrobbles
playback to your own server so its history and play counts stay current.

The local list is stored in `history.json` and is never uploaded anywhere
except your configured server's scrobble endpoint. Settings → Storage shows
its location and has a **Clear history** button.

---
title: Everyday Use
description: Browse your library, manage the queue, and understand play history.
nav_order: 3
---

## Library and search

Home combines recently added albums, recently played albums, top and most
played songs, top artists, and a random shelf. Some personalisation depends on
Navidrome's native API or its Last.fm integration and can be empty without
affecting browsing or playback.

Search covers the songs, albums, artists, and playlists indexed by your
server. Right-click rows and cards to star music, add songs to a playlist, or
put a song in the queue.

## Sidebar order

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

## Queue

Songs added with **Play next** appear before the rest of the current album or
playlist. Clear removes only those manually queued songs. The player owns the
queue, so every change is visible on the next frame and needs no server
round-trip. The complete contract is in [The Queue's Rules](/queue/).

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

---
title: What is Fastsonic?
description: A small native desktop client for your own music server.
nav_order: 0
---

Fastsonic is a native desktop client for a self-hosted music server. It talks
Subsonic/OpenSubsonic to Navidrome, Gonic, and compatible servers, and decodes
the audio stream on this computer. It is written in Rust with
[egui](https://github.com/emilk/egui), has no browser engine, and runs on
Linux, macOS, and Windows.

![Fastsonic showing a playlist with the queue open and a track playing](/screenshot.png)

## What it does

- Browses and searches your server's songs, albums, artists, starred music,
  and playlists; creates and edits playlists.
- Streams the original file and decodes FLAC, MP3, AAC/ALAC in MP4, Vorbis,
  Opus, WAV, AIFF, and PCM-family formats locally.
- Plays albums and playlists gaplessly, with an engine-owned queue, shuffle,
  repeat, seeking, ReplayGain, equalizer, and an on-disk audio cache.
- Restores the last song, position, context, and queued songs after restart.
- Keeps playing in the background and integrates with MPRIS on Linux and
  desktop media controls on macOS and Windows.
- Includes themes, album-art colour, classic Winamp 2 skins, spectrum and
  oscilloscope views, and a projectM-powered MilkDrop window.

## Product boundaries

Fastsonic connects to one server with one set of credentials. It has no
telemetry, hosted backend, Fastsonic account, browser engine, server-side
jukebox mode, offline sync, podcast client, or second music source. It does
not control other playback devices: audio is streamed from your server and
played in this process.

Core browsing and playback use Subsonic. A small Navidrome-only API supplies
personalised Home sections that the protocol cannot answer. Those sections
degrade to empty on another server or after that short-lived session expires;
the library and player continue to work.

Bug reports should include `fastsonic.log`, `panic.log` after a crash, and
steps to reproduce the problem. See [Settings & Files](/settings-and-files/)
for their locations.

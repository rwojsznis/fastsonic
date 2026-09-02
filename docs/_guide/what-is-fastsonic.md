---
title: What is Fastsonic?
description: Why Fastsonic exists, what it supports, and its current limitations.
nav_order: 0
---

## Why Fastsonic

**Spotify, native and fast.** Fastsonic is a Spotify client written in
Rust with [egui](https://github.com/emilk/egui). It plays music through
[librespot](https://github.com/librespot-org/librespot). It typically uses
100–250 MB of RAM, while Spotify's desktop app often uses 600 MB to over 1 GB.
It runs on Linux, macOS, and Windows, starts in well under a second, and has no
browser engine.

**How to say it:** Fastsonic is one word. It sounds roughly like
“fa-Spotify,” not “fast-potify.”

**Playback needs Spotify Premium.** Free accounts can browse and search, but
cannot play music through Fastsonic on this computer or another device.

![Fastsonic showing a playlist with the queue open and a track playing](/screenshot.png)

## What it does

- **Plays music on this computer.** Fastsonic appears as a Spotify Connect
  device. Select it from your phone or play music in the app. Playback is
  gapless and supports up to 320 kbps, with
  optional volume normalisation and an on-disk audio cache.
- **Controls other devices.** Move playback to a speaker, a phone, or
  another computer from the device picker, and keep controlling it: play,
  pause, skip, seek, shuffle, repeat, volume.
- **Library.** Browse playlists, Liked Songs, saved albums, followed artists,
  podcasts, and saved episodes. Create, edit, and reorder your playlists.
- **Search** across songs, artists, albums, playlists, podcasts, and
  episodes, with artist pages, discographies, and related artists.
- **Background playback.** Closing the window keeps the music playing from
  the system tray. On Linux, MPRIS supports media keys and `playerctl`.
- **Themes.** Use light, dark, or system mode. Pages can take a colour from
  album art.

## What it does not do

Fastsonic has a limited scope:

- **Playing needs Spotify Premium**, on this computer (as with every
  librespot-based client) and on other devices too, because Spotify's API
  only takes playback commands from Premium accounts. Browsing and search
  work on any account, and Fastsonic says so when a Free account signs in.
- Setup has two sign-ins because Spotify authorizes Web API and streaming
  access separately. [How it connects](/how-it-connects/) explains why.
- Local playback tops out at 320 kbps. Spotify protects its lossless streams
  with DRM that librespot does not support, and Fastsonic will not circumvent
  it. This can change if [lawful support lands upstream](https://github.com/librespot-org/librespot/issues/1583).
- No video podcasts or social features.
- Fastsonic is an **unofficial** client built on Spotify's public Web API
  and librespot. Spotify changes these from time to time; when they do,
  features can break until the client catches up.

Bug reports should include `fastsonic.log`, `panic.log` after a crash, and
steps to reproduce the problem. See the
[issue form](https://github.com/rwojsznis/fastsonic/issues/new/choose).

## Account safety

We are not aware of a Spotify account being suspended for using Fastsonic
or another librespot player with Premium. Sign-in happens on Spotify's own
pages, audio uses the quality included with Premium, DRM stays intact, and
Fastsonic does not rip tracks or block ads.

Reported suspensions usually involve modded apps that remove ads from free
accounts, track ripping, or stream manipulation. Fastsonic does none of
those things, and its contribution rules prohibit them.

## Prior art

Fastsonic uses [librespot](https://github.com/librespot-org/librespot) for
Spotify playback. It takes inspiration from
[spotify-tui](https://github.com/Rigellute/spotify-tui),
[spotify-player](https://github.com/aome510/spotify-player),
[ncspot](https://github.com/hrkfdn/ncspot), and
[Omarchy Spotify](https://github.com/stappmus/Omarchy-Spotify).

Fastsonic is an independent project, not affiliated with or endorsed by
Spotify AB. Spotify is a trademark of Spotify AB.

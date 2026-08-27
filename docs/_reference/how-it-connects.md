---
title: How It Connects
description: The two Spotify grants, why they are separate, what is stored, and what the client does when Spotify pushes back.
nav_order: 1
---

## Two grants, once each

Fastpotify talks to Spotify in two distinct ways, and Spotify issues
credentials for them separately:

1. **The Web API** covers your library, search, playlists, and devices. Fastpotify
   uses the standard Authorization Code + PKCE flow in your browser, as a
   registered Spotify application. The refresh token is stored locally and
   renewed automatically; your password never touches the app.
2. **Streaming** is actually playing audio on this computer, through
   [librespot](https://github.com/librespot-org/librespot). This runs the
   same browser flow once against Spotify's streaming client identity, after
   which librespot stores its own reusable credential. Premium is required,
   because that is what Spotify's streaming protocol requires.

Why not one grant? Because Spotify throttles Web API calls made with
streaming-identity tokens. Measured during development, every endpoint
answers `429` within the first request. Two narrow grants are what actually
works, and each one happens exactly once per machine.

By default the Web API uses the shared public application also used by
spotify-player, ncspot, and Omarchy Spotify. If you ever hit its rate
limits, create your own (free) application in Spotify's developer
dashboard, add `http://127.0.0.1:8989/login` as its redirect URI, and paste
its Client ID into Settings → Account.

## What the client stores

- The Web API refresh token and librespot's reusable credential, owner-only,
  in the state directory ([where](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- Nothing else. There is no telemetry, no analytics, and no server of ours:
  the only host Fastpotify talks to is Spotify (plus your album art CDN).

## When Spotify pushes back

The Web API rate-limits bursts. Fastpotify bounds its concurrency, honours
`Retry-After`, retries quietly, and shows a small spinner in the top bar
when a conversation with Spotify takes longer than a moment. Spotify also
reshapes endpoints over time; the client detects several of these shapes at
runtime and falls back to the older form where one still exists.

## The engine

Playback runs on a dedicated runtime: librespot maintains the Spotify
Connect session, so this computer appears as a device to every other Spotify
client you own, receives transfers, and reports its position back. If the
session drops, the engine reconnects with the stored credential; the
interface never blocks on any of it.

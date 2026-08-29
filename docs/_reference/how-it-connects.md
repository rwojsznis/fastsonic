---
title: How It Connects
description: How Fastpotify authenticates with Spotify, stores credentials, and handles API limits.
nav_order: 1
---

## Authentication

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

Spotify throttles Web API calls made with streaming-identity tokens. During
development, those tokens received a `429` response on the first request to
each endpoint. Separate credentials avoid that limitation. Each sign-in is
normally required once per machine.

By default the Web API uses the shared public application also used by
spotify-player, ncspot, and Omarchy Spotify. Its API allowance is shared among
their users. You can instead [use your own Spotify app](/make-it-even-faster/)
to get a separate allowance.

## What the client stores

- The Web API refresh token and librespot's reusable credential, owner-only,
  in the state directory ([where](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- Lyrics, in the cache directory, for a month.
- Fastpotify has no telemetry, analytics, or hosted service. Besides Spotify
  and its album art CDN, the app contacts
  [lrclib.net](https://lrclib.net) while the lyrics panel is open and
  Spotify itself has no words for the track, sending the track's artist,
  title, album, and length, and to
  api.github.com once a day to learn whether a newer release exists, which
  Settings can turn off.

## When Spotify pushes back

The Web API rate-limits bursts. Fastpotify bounds its concurrency, honours
`Retry-After`, retries quietly, and shows a small spinner in the top bar
when a request to Spotify takes longer than a moment. Spotify also
reshapes endpoints over time; the client detects several of these shapes at
runtime and falls back to the older form where one still exists.

## Receivers on the local network

Spotify's device list contains only receivers already logged in to the
account. Anything waiting to be given one, which is the normal state for a
self-hosted librespot or spotifyd, is invisible to the Web API.

Those receivers announce themselves over mDNS as `_spotify-connect._tcp` and
answer a small HTTP interface. Fastpotify asks a receiver to describe itself,
then hands over the reusable credential librespot already stores, wrapped
twice: once in a key derived from the receiver's own device id, and again in
a key both sides derive from a Diffie-Hellman exchange with the public key
that receiver just published. The encrypted value is specific to that
receiver and exchange. Fastpotify does not write another copy of the
credential.

The receiver then signs in and appears in Spotify's device list. Fastpotify
uses the Web API for subsequent control requests.

## The engine

Playback runs on a dedicated runtime: librespot maintains the Spotify
Connect session, exposes this computer as a device, receives transfers, and
reports its playback position. If the session drops, the engine reconnects
with the stored credential. This work does not block the interface.

---
title: How It Connects
description: Fastsonic's server protocols, stored credential, and outbound network traffic.
nav_order: 1
---

Fastsonic has no hosted service or application account. It connects directly
from this computer to the one music server entered on the sign-in screen.

## Sign-in and authentication

For Navidrome, sign-in is one `POST /auth/login` request containing the server
username and password. The response contains two different credentials:

- A Subsonic salt and token pair used for `/rest/*.view` calls. The token is
  MD5 of the password and salt because that is the protocol's wire format.
  Fastsonic stores this pair and never stores the password. The pair is still
  password-equivalent for that server and must be kept private.
- A Navidrome JWT used only by the small native-API module. It supplies
  personalisation that Subsonic cannot, and normally expires after 24 hours.
  Navidrome administrators can change that period with `ND_SESSIONTIMEOUT`.

The salted token continues to provide library browsing and playback after the
JWT expires. Personalised Home sections then appear empty until the next
password sign-in. On a compatible non-Navidrome server, the same core features
can work while Navidrome-only sections remain empty.

An empty password field retries the stored Subsonic credential, which is
useful after a server was temporarily unreachable. Signing out removes both
stored credentials.

Every Subsonic request carries the username, salt, and token. Fastsonic strips
credential query parameters and authorization headers from logs. Artwork is
cached under an opaque `sonic:art:` key rather than a credential-bearing URL.

## Server requests

Library, search, playlists, stars, artwork, lyrics supplied by the server,
streams, and scrobbles use Subsonic/OpenSubsonic. Playlist edits use form POST
when necessary. Playback requests the original file, without transcoding, so
HTTP byte ranges remain available for seeking and the in-process decoder sees
the library's real format.

The audio engine runs outside the UI thread. It reads the HTTP stream through
a bounded on-disk block cache, decodes and resamples it, applies ReplayGain and
the equalizer, sends post-EQ/pre-volume samples to the visualisers, then sends
the volume-adjusted signal to the selected local output device. The queue is
owned by this engine and does not exist on the server.

## Other outbound traffic

- When the lyrics panel is open and the server has no lyrics, Fastsonic sends
  the track's artist, title, album, and duration to
  [LRCLIB](https://lrclib.net). Lyrics are cached locally for a month.
- If update checks are enabled, Fastsonic asks GitHub's API once a day whether
  a newer release exists.
- Settings can download optional MilkDrop preset packs from their documented
  upstream sources.

There is no telemetry, analytics, inbound listener, receiver discovery,
peer-to-peer traffic, or server-side playback control. TLS certificates use
the operating system's trust decisions; there is currently no switch to allow
a self-signed certificate.

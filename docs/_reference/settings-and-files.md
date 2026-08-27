---
title: Settings & Files
description: Where Fastpotify keeps configuration, credentials, and caches, and what is safe to delete.
nav_order: 0
---

## Where things live

Fastpotify follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/fastpotify/settings.json` | Yes, you lose preferences |
| Web API sign-in | `~/.local/state/fastpotify/web_api_token.json` | Yes, you sign in again |
| Playback credential | `~/.local/state/fastpotify/credentials/` | Yes, you approve playback again |
| Last session | `~/.local/state/fastpotify/session.json` | Yes |
| Audio cache | `~/.cache/fastpotify/audio/` | Always |
| Artwork cache | `~/.cache/fastpotify/art/` | Always |

Clearing caches never signs you out; credentials live in *state*, not
*cache*, precisely so cleanup tools cannot log you out. Both credential
files are written with owner-only permissions. Signing out from Settings
deletes both.

## settings.json

One readable JSON file, written atomically. The interesting fields:

| Field | Default | Meaning |
| --- | --- | --- |
| `device_name` | `Fastpotify` | Name on Spotify Connect |
| `bitrate` | `320` | 96, 160, or 320 kbps |
| `normalisation` | `false` | Volume normalisation |
| `autoplay` | `true` | Keep playing similar music at the end |
| `gapless` | `true` | Gapless playback |
| `audio_backend` | platform | `pulseaudio` or `rodio` on Linux |
| `audio_cache_mb` | `1024` | On-disk audio cache budget |
| `theme` | `dark` | `dark`, `light`, or `system` |
| `accent_from_art` | `true` | Tint pages with album art |
| `keep_playing_in_background` | `true` | Close to tray |
| `web_client_id` | none | Your own Spotify app id, if you set one |

## Command line

```
fastpotify [OPTIONS]

  --device-name <NAME>  Spotify Connect name for this session
  -v, --verbose         More logs from librespot and the API client
```

`fastpotify -v` output is what to attach to a bug report.

## Demo mode

Builds made with `cargo build --features demo` accept `--demo`, which fills
the interface with sample data and never contacts Spotify, useful for
screenshots, theming, and interface work. Demo mode never writes settings.

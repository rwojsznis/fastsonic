# Fastsonic

> Fastsonic is under active migration and has not published its first release.
> Build from source for development and testing; do not use inherited
> Fastpotify packages as Fastsonic builds.

Fastsonic is a small native desktop client for a self-hosted music server. It
talks Subsonic/OpenSubsonic to Navidrome, Gonic, and compatible servers,
streams the original audio file, and decodes it locally. It has no browser
engine, telemetry, hosted backend, or Fastsonic account.

![Fastsonic showing a playlist with the queue open](docs/screenshot.png)

The in-repository [guides](docs/_guide/getting-started.md) and
[reference documentation](docs/_reference/settings-and-files.md) cover setup,
everyday use, network traffic, settings, and stored files.

## Features

- Songs, albums, artists, starred music, playlists, search, and a self-hosted
  library-focused Home page.
- In-process playback of the formats in your library, including FLAC, MP3,
  AAC/ALAC, Vorbis, Opus, WAV, and AIFF; gapless transitions and byte-range
  seeking.
- An engine-owned queue, shuffle and repeat, ReplayGain normalisation, a
  ten-band equalizer, and a bounded on-disk audio cache.
- Restores the last track, position, context, and manually queued songs.
- Light, dark, and system themes with optional album-art colour.
- A Winamp mini player for classic `.wsz` skins, spectrum analyser,
  oscilloscope, equalizer, and playlist.
- A projectM-powered MilkDrop window with optional preset packs.
- Background playback, Linux MPRIS, desktop media controls, keyboard
  shortcuts, tray/Dock reopening, and single-instance behavior.

Fastsonic plays only on this computer. It does not provide Spotify Connect,
Subsonic jukebox mode, podcasts, offline sync, multiple server profiles, or a
second source of audio.

## Build

Rust 1.95 or newer is required:

```sh
git clone https://github.com/rwojsznis/fastsonic
cd fastsonic
cargo install --path .
```

MilkDrop is enabled by default and builds projectM from source, requiring
CMake, a C++ compiler, and libclang. Build without it using
`cargo install --path . --no-default-features`. On Linux, install the ALSA,
PulseAudio/PipeWire, Wayland, and X11 development packages described in
[Getting Started](docs/_guide/getting-started.md).
`nix develop` provides the complete pinned environment.

There is currently no Flatpak, Homebrew tap, AUR package, installer, or release
archive. The first release will be linked only after its platform artifacts and
checksums exist.

## Connect

Launch Fastsonic and enter your server URL, username, and password. For
Navidrome, the password is sent once to `/auth/login`; Fastsonic stores the
returned salted Subsonic token rather than the password. Core library and
playback traffic then uses `/rest/*.view`. A small isolated Navidrome API
client supplies personalisation that Subsonic cannot; those sections degrade
to empty on other compatible servers or after its session expires.

The app makes no inbound connection and exposes no receiver. See
[How It Connects](docs/_reference/how-it-connects.md) for
the complete network and privacy behavior.

## Development

The repository includes a deterministic demo mode:

```sh
cargo run --features demo -- --demo --demo-page playlist:pl1 --demo-show queue
```

The tests that talk to a real server are `#[ignore]`d and need a Navidrome
of your own; `FASTSONIC_TEST_SERVER`, `FASTSONIC_TEST_USER` and
`FASTSONIC_TEST_PASSWORD` point them at it. Read
[CONTRIBUTING.md](CONTRIBUTING.md) before making changes; it defines the
required checks. The architecture is centered on:

- `src/api/subsonic/` — Subsonic transport, conversion, scrobbling, and the
  isolated Navidrome-native calls.
- `src/engine/` — streaming, block cache, decoding, playback chain, and queue.
- `src/backend.rs` — the runtime and channels used by the interface.
- `src/app.rs` and `src/ui/` — application state, navigation, and views.

## Acknowledgements

Fastsonic was forked from
[Fastpotify](https://github.com/crmne/fastpotify). It uses
[egui](https://github.com/emilk/egui), the [Inter](https://rsms.me/inter/)
typeface, [Lucide](https://lucide.dev) icons, and
[projectM](https://github.com/projectM-visualizer/projectm).

Licensed under the [MIT License](LICENSE).

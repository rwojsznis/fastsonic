# Getting Started

## Install

There are no release packages yet. Build from source with
[Rust](https://rustup.rs) 1.95 or newer:

```sh
git clone https://github.com/rwojsznis/fastsonic
cd fastsonic
cargo install --path .
```

MilkDrop is enabled by default and builds projectM from source, which also
needs CMake, a C++ compiler, and libclang. Use `--no-default-features` if you
do not want MilkDrop. The repository's `nix develop` environment includes the
complete toolchain.

On Arch Linux:

```sh
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland cmake clang
```

On Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev \
  libgl1-mesa-dev cmake clang libclang-dev
```

On Windows, the default build needs Visual Studio 2022, CMake, LLVM, and vcpkg
with `glew:x64-windows-static-md`; set `VCPKG_INSTALLATION_ROOT` to the vcpkg
folder.

Fastsonic uses system fonts for scripts that its interface font does not
cover. On Linux, install `noto-fonts` and `noto-fonts-cjk` (Arch) or
`fonts-noto` and `fonts-noto-cjk` (Debian or Ubuntu) if titles appear as empty
boxes.

## Connect to your server

Start the app and enter:

1. The base URL of your server, such as `https://music.example.net` or
   `http://192.168.1.20:4533`.
2. Your server username.
3. Your password.

Press **Connect**. Fastsonic sends the password once to Navidrome's login
endpoint and stores the salted Subsonic token returned by the server, not the
password. A bare host is treated as `http://`; HTTPS certificates must be
trusted by the operating system. See [How It Connects](../_reference/how-it-connects.md) for
the exact network and credential behavior.

Fastsonic supports Navidrome 0.51.0 and newer. Other compatible
Subsonic/OpenSubsonic servers can provide the core library and playback
features; Navidrome-only personalisation sections may be empty.

## Basics

- Closing the window keeps music playing from the system tray by default.
  Reopen it from the tray or Dock, and quit from the tray menu or Ctrl+Q.
- Space plays and pauses, Ctrl+F or `/` searches, and `Q` opens the queue.
  Ctrl+/ shows every shortcut. On macOS, Cmd replaces Ctrl.
- Right-click a song, playlist, album, or artist for actions such as Play
  next, star, add to playlist, and copy link.

---
layout: home
title: Fastsonic
description: A fast, native client for self-hosted Subsonic music servers.
permalink: /
hero:
  name: Fastsonic
  text: Your music server, native and fast
  tagline: A lightweight desktop player for Navidrome and other Subsonic-compatible servers on Linux, macOS, and Windows.
  actions:
    - theme: brand
      text: Download
      link: /download/
    - theme: alt
      text: What is Fastsonic?
      link: /what-is-fastsonic/
    - theme: alt
      text: GitHub
      link: https://github.com/rwojsznis/fastsonic
  image:
    src: /screenshot.png
    alt: "Fastsonic showing the Late night focus playlist with the queue panel open, a track playing, and the library in the sidebar"
    width: 1894
    height: 1037

features:
  - icon: ⚡
    title: Lightweight
    details: A native binary with no browser engine. It starts in well under a second and typically uses 100–250 MB of RAM.
  - icon: 🔊
    title: Local playback
    details: Stream the original files from your server, decode them in the app, and play gaplessly with ReplayGain and an on-disk cache.
  - icon: 📚
    title: Library and search
    details: Browse playlists, starred songs, albums, and artists. Search your server's library and edit playlists you own.
  - icon: 🎨
    title: Themes
    details: Use light, dark, or system mode. Pages and the player bar can take their colour from the album art.
  - icon: 📻
    title: Winamp mini player
    details: Ctrl+M opens a small player for classic Winamp 2 skins, with a spectrum analyser, equalizer, and playlist.
    link: /winamp/
    link_text: See it in action
  - icon: 🌀
    title: MilkDrop
    details: Run projectM's MilkDrop visualiser in its own window, with fullscreen, preset packs, and keyboard controls.
    link: /milkdrop/
    link_text: Open the guide
  - icon: ⌨️
    title: Desktop controls
    details: Keyboard shortcuts, MPRIS media controls on Linux, and a tray option that keeps music playing after you close the window.
  - icon: 🔓
    title: Open source
    details: MIT-licensed Rust built with egui. No browser engine, telemetry, hosted backend, or account system.
    link: https://github.com/rwojsznis/fastsonic
    link_text: Read the source
---

<style>
  /* The hero image slot is sized for a square logo; the screenshot needs the
     room. Page-scoped overrides, so the theme stays untouched. */
  .VPHero .image-container {
    width: 100% !important;
    height: auto !important;
    transform: none !important;
  }
  .VPHero .image-src {
    position: relative !important;
    top: auto !important;
    left: auto !important;
    transform: none !important;
    width: 100% !important;
    height: auto !important;
    max-width: 100% !important;
    max-height: none !important;
    padding: 0 !important;
    border-radius: 12px;
    box-shadow: 0 12px 48px rgba(0, 0, 0, 0.45);
  }
  @media (max-width: 959px) {
    .VPHero .image {
      margin: 0 0 24px !important;
    }
  }
</style>

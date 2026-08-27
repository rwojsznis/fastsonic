---
layout: home
title: Fastpotify
description: A fast, native Spotify client for Linux, macOS, and Windows, written in Rust.
permalink: /
hero:
  name: Fastpotify
  text: Spotify, native and fast
  tagline: Your whole Spotify library, local playback, and every Connect device in one lightweight Rust app that opens in a blink — on Linux, macOS, and Windows.
  actions:
    - theme: brand
      text: Download
      link: /download/
    - theme: alt
      text: What is Fastpotify?
      link: /what-is-fastpotify/
    - theme: alt
      text: GitHub
      link: https://github.com/crmne/fastpotify
  image:
    src: /screenshot.png
    alt: "Fastpotify showing the Late night focus playlist with the queue panel open, a track playing, and the library in the sidebar"
    width: 1894
    height: 1037

features:
  - icon: ⚡
    title: Actually lightweight
    details: A single native binary with no browser engine anywhere in the process. It starts in well under a second and stays small while it runs.
  - icon: 🔊
    title: A real Spotify Connect device
    details: Play on this computer — gapless, up to 320 kbps — or push the music to any speaker, phone, or TV and keep controlling it from the same window.
  - icon: 📚
    title: Your whole library
    details: Playlists, Liked Songs, albums, artists, and podcasts, with search across all of it and playlist editing where you own the playlist.
  - icon: 🎨
    title: Beautiful by intent
    details: Pages and the player take their colour from the album art. Light, dark, or follow the system — and the layout you already know from Spotify.
  - icon: ⌨️
    title: Keyboard-first, desktop-native
    details: Shortcuts for everything, MPRIS media controls on Linux, and a tray that keeps the music playing after you close the window.
  - icon: 🔓
    title: Open source
    details: MIT-licensed Rust on egui and librespot, with an honest write-up of how it talks to Spotify.
    link: https://github.com/crmne/fastpotify
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

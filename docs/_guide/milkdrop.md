# MilkDrop

Open MilkDrop from the top-bar visualiser button, Ctrl+Shift+K, Settings, or
the mini player's **V** menu. It uses
[projectM](https://github.com/projectM-visualizer/projectm) to play `.milk`
presets in its own window and process. With a classic Winamp skin on, it is
a panel of the mini player instead, under the equalizer and the playlist, and
wears the skin's frame as it did in Winamp 5, with a title bar, a close button
and a corner to make it taller by; the window grows with it. The button beside the close button,
a double click or **F** takes it to full screen lyrics, with MilkDrop behind
the words in place of the cover's art; leaving brings the frame back. The
main window's full screen lyrics show it too: press **V** there. The next and previous
preset keys and the right-click below work in it too; the other MilkDrop keys
and the song title are only in the plain window. Windows Media Player skins
also show it in their visualisation areas; see
[Windows Media Player skins](winamp.md#windows-media-player-skins).

<video autoplay loop muted playsinline preload="metadata" aria-label="MilkDrop running in Fastsonic" style="width: 100%; height: auto;">
  <source src="../assets/images/milkdrop.mp4" type="video/mp4">
</video>

MilkDrop is included in every build: Linux, macOS, and Windows on Intel,
AMD and ARM PCs alike.

## The window

Drag the image to move the window. Double-click or press **F** for fullscreen.
Press **Esc** to leave fullscreen or close the window. Drag the lower-right
corner to resize it.

MilkDrop uses the same post-equalizer, pre-volume audio as the other
visualisers. It keeps moving at zero volume.

MilkDrop stops drawing while its window is minimised, and on macOS and Linux
also while it is covered. While the system saves energy it draws at most 30
frames a second, and one a second while the display is off or the screen is
locked (Windows).

## Presets

Presets change every ten seconds by default. Change the interval in Settings;
a new interval applies to the preset already playing, and no preset is cut
shorter than its 2.7-second fade.
Presets live in the config directory's `milkdrop` folder. Settings can download
the 550 MilkDrop 2 presets and the 9,800-preset Cream of the Crop pack. Until a
preset is installed, projectM shows its idle preset.

## Controls

- **N** or right arrow: next preset.
- **P** or left arrow: previous preset.
- **H**: switch on the next beat.
- **L**: keep the current preset.
- **R**: switch between random and folder order.
- **T**: show or hide the preset name.
- **D**: show or hide the frame rate.
- **I**: cycle the song display between a change notification, always visible,
  and hidden.
- **?** or **F1**: show all shortcuts.

The normal playback shortcuts also work.
